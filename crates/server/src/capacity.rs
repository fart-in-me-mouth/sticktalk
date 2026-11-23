//! Batched channel visibility for the raised capacity limits (500 channels per
//! server, 1,000 members, 250 roles). `permissions::evaluate` costs several
//! queries per (member, channel) pair; listing a server's channels or a
//! channel's audience that way grows with channels x members. These functions
//! load a server's roles, overwrites and timeouts once (a fixed number of
//! queries) and evaluate in memory with the same rules as `evaluate`.
use crate::{
    db::{en, id},
    error::Result,
    permissions as acl,
    state::now,
};
use sqlx::{Row, SqliteConnection};
use std::collections::{HashMap, HashSet};
use sticktalk_proto::*;

/// Everything about one server that permission evaluation reads.
struct Guild {
    id: Id,
    owner: Id,
    masks: HashMap<Id, Permissions>,
    overwrites: HashMap<Id, Vec<PermissionOverwrite>>,
    parents: HashMap<Id, Option<Id>>,
    timed_out: HashSet<Id>,
    /// The thread rules (`threads::scope`) for the one thread being evaluated.
    thread: Option<(Id, crate::threads::ThreadScope, HashSet<Id>)>,
}
impl Guild {
    async fn load(c: &mut SqliteConnection, g: Id, timeouts_for: Option<Id>) -> Result<Self> {
        let owner: i64 = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id=?")
            .bind(id(g)?)
            .fetch_one(&mut *c)
            .await?;
        let masks = sqlx::query("SELECT id,permissions FROM roles WHERE guild_id=?")
            .bind(id(g)?)
            .fetch_all(&mut *c)
            .await?
            .into_iter()
            .map(|r| {
                (
                    Id(r.get::<i64, _>(0) as u64),
                    Permissions::from_bits_retain(r.get::<i64, _>(1) as u64),
                )
            })
            .collect();
        let mut overwrites: HashMap<Id, Vec<PermissionOverwrite>> = HashMap::new();
        for r in sqlx::query("SELECT o.channel_id,o.kind,o.target_id,o.allow,o.deny FROM permission_overwrites o JOIN channels ch ON ch.id=o.channel_id WHERE ch.guild_id=? ORDER BY o.channel_id,o.kind,o.target_id")
            .bind(id(g)?)
            .fetch_all(&mut *c)
            .await?
        {
            overwrites
                .entry(Id(r.get::<i64, _>(0) as u64))
                .or_default()
                .push(PermissionOverwrite {
                    kind: en(r.get(1))?,
                    target_id: Id(r.get::<i64, _>(2) as u64),
                    allow: Permissions::from_bits_retain(r.get::<i64, _>(3) as u64),
                    deny: Permissions::from_bits_retain(r.get::<i64, _>(4) as u64),
                });
        }
        let parents = sqlx::query("SELECT id,parent_id FROM channels WHERE guild_id=?")
            .bind(id(g)?)
            .fetch_all(&mut *c)
            .await?
            .into_iter()
            .map(|r| {
                (
                    Id(r.get::<i64, _>(0) as u64),
                    r.get::<Option<i64>, _>(1).map(|v| Id(v as u64)),
                )
            })
            .collect();
        let timed_out = sqlx::query_scalar::<_, i64>(
            "SELECT user_id FROM member_timeouts WHERE guild_id=? AND until>? AND (? IS NULL OR user_id=?)",
        )
        .bind(id(g)?)
        .bind(now() as i64)
        .bind(timeouts_for.map(|v| v.0 as i64))
        .bind(timeouts_for.map(|v| v.0 as i64))
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|v| Id(v as u64))
        .collect();
        Ok(Self {
            id: g,
            owner: Id(owner as u64),
            masks,
            overwrites,
            parents,
            timed_out,
            thread: None,
        })
    }
    /// `ch` and every channel it is nested in, outermost first (as `acl::lineage`).
    fn lineage(&self, ch: Id) -> Vec<Id> {
        let mut chain = vec![];
        let mut at = Some(ch);
        while let Some(v) = at {
            if !self.parents.contains_key(&v) || chain.len() as i64 > acl::MAX_DEPTH {
                break;
            }
            chain.push(v);
            at = self.parents.get(&v).copied().flatten();
        }
        chain.reverse();
        chain
    }
    /// The same result as `acl::evaluate(user, guild, Some(ch))` for a member
    /// whose role IDs (including @everyone) are `roles`.
    fn evaluate(&self, user: Id, roles: &[Id], ch: Option<Id>) -> Permissions {
        if user == self.owner {
            return Permissions::all();
        }
        let mut p = roles
            .iter()
            .filter_map(|r| self.masks.get(r))
            .fold(Permissions::empty(), |p, m| p | *m)
            - Permissions::OWNER;
        if p.contains(Permissions::ADMINISTRATOR) {
            return Permissions::all() - Permissions::OWNER;
        }
        if let Some(ch) = ch {
            for scope in self.lineage(ch) {
                if let Some(list) = self.overwrites.get(&scope) {
                    p = acl::apply(p, self.id, user, roles, list);
                }
            }
            p = acl::implicit(p);
            p = match &self.thread {
                Some((t, scope, members)) if *t == ch => {
                    scope.apply(user, members.contains(&user), p)
                }
                _ => crate::threads::unscoped(p),
            };
        }
        if self.timed_out.contains(&user) {
            p &= crate::moderation::TIMED_OUT_KEEPS;
        }
        p
    }
}
async fn member_roles(
    c: &mut SqliteConnection,
    g: Id,
    u: Option<Id>,
) -> Result<HashMap<Id, Vec<Id>>> {
    let mut out: HashMap<Id, Vec<Id>> = HashMap::new();
    for r in sqlx::query("SELECT m.user_id,r.role_id FROM guild_members m LEFT JOIN member_roles r ON r.guild_id=m.guild_id AND r.user_id=m.user_id AND r.role_id<>m.guild_id WHERE m.guild_id=? AND (? IS NULL OR m.user_id=?) ORDER BY m.user_id,r.role_id")
        .bind(id(g)?)
        .bind(u.map(|v| v.0 as i64))
        .bind(u.map(|v| v.0 as i64))
        .fetch_all(&mut *c)
        .await?
    {
        let roles = out.entry(Id(r.get::<i64, _>(0) as u64)).or_insert_with(|| vec![g]);
        if let Some(role) = r.get::<Option<i64>, _>(1) {
            roles.push(Id(role as u64));
        }
    }
    Ok(out)
}
/// Every channel of server `g`, and whether `u` (a member) can see it, in four
/// queries. Visibility matches `acl::evaluate(..).contains(VIEW_CHANNEL)`.
pub(crate) async fn guild_channels(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
) -> Result<Vec<(ChannelSummary, bool)>> {
    let guild = Guild::load(c, g, Some(u)).await?;
    let roles = member_roles(c, g, Some(u))
        .await?
        .remove(&u)
        .unwrap_or_else(|| vec![g]);
    let rows = sqlx::query("SELECT id,kind,parent_id,name,position,owner_id FROM channels WHERE guild_id=? AND kind<>'thread' ORDER BY position,id")
        .bind(id(g)?)
        .fetch_all(&mut *c)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let ch = Id(r.get::<i64, _>(0) as u64);
        let visible = guild
            .evaluate(u, &roles, Some(ch))
            .contains(Permissions::VIEW_CHANNEL);
        out.push((
            ChannelSummary {
                id: ch,
                kind: en(r.get(1))?,
                guild_id: Some(g),
                parent_id: r.get::<Option<i64>, _>(2).map(|v| Id(v as u64)),
                name: r.get(3),
                position: r.get::<i64, _>(4) as u16,
                recipient_ids: vec![],
                owner_id: r.get::<Option<i64>, _>(5).map(|v| Id(v as u64)),
            },
            visible,
        ));
    }
    Ok(out)
}
/// Members of server `g` whose permissions in channel `ch` (or in the server,
/// for `None`) contain `p`, ordered by user ID, in five queries.
pub(crate) async fn members_with(
    c: &mut SqliteConnection,
    g: Id,
    ch: Option<Id>,
    p: Permissions,
) -> Result<Vec<Id>> {
    let mut guild = Guild::load(c, g, None).await?;
    if let Some(ch) = ch {
        if let Some(scope) = crate::threads::ThreadScope::load(c, ch).await? {
            let members = crate::threads::ThreadScope::members(c, ch).await?;
            guild.thread = Some((ch, scope, members));
        }
    }
    let mut users: Vec<_> = member_roles(c, g, None)
        .await?
        .into_iter()
        .filter(|(u, roles)| guild.evaluate(*u, roles, ch).contains(p))
        .map(|(u, _)| u)
        .collect();
    users.sort();
    Ok(users)
}
/// Whether a member holding only `roles` (plus @everyone) could see `ch`,
/// ignoring member-specific overwrites. For the channel access summary.
pub(crate) async fn role_can_view(
    c: &mut SqliteConnection,
    g: Id,
    ch: Id,
    role_sets: &[Vec<Id>],
) -> Result<Vec<bool>> {
    let guild = Guild::load(c, g, Some(Id(0))).await?;
    Ok(role_sets
        .iter()
        .map(|roles| {
            let mut all = vec![g];
            all.extend(roles.iter().copied().filter(|r| *r != g));
            guild
                .evaluate(Id(0), &all, Some(ch))
                .contains(Permissions::VIEW_CHANNEL)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit_tests::fixture;
    /// The batched evaluator must agree with `evaluate` for every member and
    /// channel, through nested overwrites, roles, administrators, timeouts and
    /// the thread rules (private membership, locks).
    #[tokio::test]
    async fn batched_visibility_matches_evaluate() {
        let (_dir, app) = fixture().await;
        let mut c = app.0.pool.acquire().await.unwrap();
        for q in [
            "INSERT INTO guild_members VALUES(10,3,'member',0),(10,4,'member',0)",
            "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(20,'category',10,'cat',1,0)",
            "INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(21,'text',10,20,'inner',0,0)",
            "INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(22,'voice',10,21,'deep',0,0)",
            // View Channel and Manage Threads.
            "INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,30,'mods',2,34359738369)",
            "INSERT INTO member_roles VALUES(10,3,30)",
            "INSERT INTO member_roles VALUES(10,4,11)",
            "INSERT INTO permission_overwrites VALUES(20,'role',10,0,1)",
            "INSERT INTO permission_overwrites VALUES(20,'role',30,1,0)",
            "INSERT INTO permission_overwrites VALUES(21,'member',3,0,2)",
            "INSERT INTO permission_overwrites VALUES(22,'member',2,1,0)",
            // A private thread (member: 4) and a locked public one.
            "INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(23,'thread',10,21,'secret',0,0)",
            "INSERT INTO threads(channel_id,parent_id,guild_id,owner_id,private,auto_archive_minutes,created_at,last_activity_at,archive_at) VALUES(23,21,10,1,1,60,0,0,0)",
            "INSERT INTO thread_members VALUES(23,4,0,'default')",
            "INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(24,'thread',10,11,'closed',0,0)",
            "INSERT INTO threads(channel_id,parent_id,guild_id,owner_id,locked,auto_archive_minutes,created_at,last_activity_at,archive_at) VALUES(24,11,10,1,1,60,0,0,0)",
        ] {
            sqlx::query(q).execute(&mut *c).await.unwrap();
        }
        sqlx::query(
            "INSERT INTO member_timeouts(guild_id,user_id,until,actor_id) VALUES(10,2,?,1)",
        )
        .bind((now() + 60_000) as i64)
        .execute(&mut *c)
        .await
        .unwrap();
        for ch in [11, 20, 21, 22, 23, 24] {
            let ch = Id(ch);
            for p in [
                Permissions::VIEW_CHANNEL,
                Permissions::SEND_MESSAGES,
                Permissions::ADD_REACTIONS,
                Permissions::MANAGE_THREADS,
                Permissions::CREATE_PUBLIC_THREADS,
            ] {
                let batch = members_with(&mut c, Id(10), Some(ch), p).await.unwrap();
                let mut slow = vec![];
                for u in crate::db::guild_users(&mut c, Id(10)).await.unwrap() {
                    if acl::evaluate(&mut c, u, Id(10), Some(ch))
                        .await
                        .unwrap()
                        .contains(p)
                    {
                        slow.push(u);
                    }
                }
                assert_eq!(batch, slow, "channel {ch:?} {p:?}");
            }
        }
        for u in 1..=4 {
            let listed = guild_channels(&mut c, Id(u), Id(10)).await.unwrap();
            for (ch, visible) in listed {
                let slow = acl::evaluate(&mut c, Id(u), Id(10), Some(ch.id))
                    .await
                    .unwrap()
                    .contains(Permissions::VIEW_CHANNEL);
                assert_eq!(visible, slow, "user {u} channel {:?}", ch.id);
            }
        }
    }
}
