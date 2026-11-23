//! Shared authorization entry point for chat, REST, fan-out and the voice host.
//! Call under the application's serialization gate, using the current transaction.
use crate::{
    db::id,
    error::{require, Error, Result},
};
use sqlx::{Row, SqliteConnection};
use sticktalk_proto::*;

/// Owner bypass is based on guild ownership, never on assignable bits.
/// `channel=None` computes guild permissions; Some also applies the ACLs of every
/// channel it is nested in, outermost first, then its own.
/// Nonmembers and cross-guild channels are NotFound. Administrator bypasses overwrites,
/// but does not grant the legacy OWNER sentinel (ownership-only operations).
pub async fn evaluate(
    c: &mut SqliteConnection,
    user: Id,
    guild: Id,
    channel: Option<Id>,
) -> Result<Permissions> {
    let owner: Option<i64> = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id=? AND EXISTS(SELECT 1 FROM guild_members WHERE guild_id=? AND user_id=?)")
        .bind(id(guild)?).bind(id(guild)?).bind(id(user)?).fetch_optional(&mut *c).await?;
    let owner = owner.ok_or_else(|| Error::new(ErrorCode::NotFound))? == id(user)?;
    let scopes = match channel {
        Some(ch) => lineage(c, guild, ch).await?,
        None => vec![],
    };
    if owner {
        return Ok(Permissions::all());
    }
    let roles = role_ids(c, guild, user).await?;
    let masks: Vec<i64> = sqlx::query_scalar("SELECT permissions FROM roles WHERE guild_id=? AND (id=? OR id IN(SELECT role_id FROM member_roles WHERE guild_id=? AND user_id=?))")
        .bind(id(guild)?).bind(id(guild)?).bind(id(guild)?).bind(id(user)?).fetch_all(&mut *c).await?;
    let mut p = masks.into_iter().fold(Permissions::empty(), |p, b| {
        p | Permissions::from_bits_retain(b as u64)
    }) - Permissions::OWNER;
    if p.contains(Permissions::ADMINISTRATOR) {
        return Ok(Permissions::all() - Permissions::OWNER);
    }
    for scope in scopes {
        p = apply(p, guild, user, &roles, &overwrites(c, scope).await?);
    }
    if channel.is_some() {
        p = implicit(p);
    }
    Ok(p)
}
/// The longest chain of nested channels that is followed. A guild holds fewer
/// channels than this, so the bound only stops a corrupt, cyclic chain.
pub(crate) const MAX_DEPTH: i64 = 64;
/// `ch` and every channel it is nested in, outermost first. NotFound when `ch`
/// is not a channel of `guild`.
pub(crate) async fn lineage(c: &mut SqliteConnection, guild: Id, ch: Id) -> Result<Vec<Id>> {
    let chain: Vec<i64> = sqlx::query_scalar(
        "WITH RECURSIVE up(id,parent_id,depth) AS (         SELECT id,parent_id,0 FROM channels WHERE id=?1 AND guild_id=?2          UNION ALL          SELECT c.id,c.parent_id,up.depth+1 FROM channels c JOIN up ON c.id=up.parent_id          WHERE c.guild_id=?2 AND up.depth<?3)          SELECT id FROM up ORDER BY depth DESC",
    )
    .bind(id(ch)?)
    .bind(id(guild)?)
    .bind(MAX_DEPTH)
    .fetch_all(c)
    .await?;
    require(!chain.is_empty(), ErrorCode::NotFound)?;
    Ok(chain.into_iter().map(|v| Id(v as u64)).collect())
}
fn apply(
    mut p: Permissions,
    everyone: Id,
    user: Id,
    roles: &[Id],
    overwrites: &[PermissionOverwrite],
) -> Permissions {
    for o in overwrites
        .iter()
        .filter(|o| o.kind == OverwriteKind::Role && o.target_id == everyone)
    {
        p = (p - o.deny) | o.allow;
    }
    let mut allow = Permissions::empty();
    let mut deny = Permissions::empty();
    for o in overwrites.iter().filter(|o| {
        o.kind == OverwriteKind::Role && o.target_id != everyone && roles.contains(&o.target_id)
    }) {
        allow |= o.allow;
        deny |= o.deny;
    }
    p = (p - deny) | allow;
    for o in overwrites
        .iter()
        .filter(|o| o.kind == OverwriteKind::Member && o.target_id == user)
    {
        p = (p - o.deny) | o.allow;
    }
    p - Permissions::OWNER - Permissions::ADMINISTRATOR
}
fn implicit(mut p: Permissions) -> Permissions {
    if !p.contains(Permissions::VIEW_CHANNEL) {
        p.remove(
            Permissions::VIEW_CHANNEL
                | Permissions::SEND_MESSAGES
                | Permissions::READ_HISTORY
                | Permissions::ATTACH_FILES
                | Permissions::ADD_REACTIONS
                | Permissions::MANAGE_MESSAGES
                | Permissions::EMBED_LINKS
                | Permissions::MENTION_EVERYONE
                | Permissions::CONNECT
                | Permissions::SPEAK
                | Permissions::STREAM
                | Permissions::VIDEO
                | Permissions::USE_VOICE_ACTIVITY
                | Permissions::PRIORITY_SPEAKER
                | Permissions::MUTE_MEMBERS
                | Permissions::DEAFEN_MEMBERS
                | Permissions::MOVE_MEMBERS,
        );
    }
    if !p.contains(Permissions::SEND_MESSAGES) {
        p.remove(
            Permissions::ATTACH_FILES | Permissions::EMBED_LINKS | Permissions::MENTION_EVERYONE,
        );
    }
    if !p.contains(Permissions::CONNECT) {
        p.remove(
            Permissions::SPEAK
                | Permissions::STREAM
                | Permissions::VIDEO
                | Permissions::USE_VOICE_ACTIVITY
                | Permissions::PRIORITY_SPEAKER,
        );
    }
    p
}
pub(crate) async fn role_ids(c: &mut SqliteConnection, g: Id, u: Id) -> Result<Vec<Id>> {
    let mut ids = vec![g];
    ids.extend(sqlx::query_scalar::<_, i64>("SELECT role_id FROM member_roles WHERE guild_id=? AND user_id=? AND role_id<>? ORDER BY role_id")
        .bind(id(g)?).bind(id(u)?).bind(id(g)?).fetch_all(c).await?.into_iter().map(|v| Id(v as u64)));
    Ok(ids)
}
pub(crate) async fn roles(c: &mut SqliteConnection, g: Id) -> Result<Vec<Role>> {
    sqlx::query("SELECT * FROM roles WHERE guild_id=? ORDER BY position,id")
        .bind(id(g)?)
        .fetch_all(c)
        .await?
        .into_iter()
        .map(|r| {
            Ok(Role {
                id: Id(r.get::<i64, _>("id") as u64),
                guild_id: g,
                name: r.get("name"),
                color: r.get::<i64, _>("color") as u32,
                position: r.get::<i64, _>("position") as u16,
                hoist: r.get("hoist"),
                mentionable: r.get("mentionable"),
                permissions: Permissions::from_bits_retain(r.get::<i64, _>("permissions") as u64),
            })
        })
        .collect()
}
pub(crate) async fn overwrites(
    c: &mut SqliteConnection,
    ch: Id,
) -> Result<Vec<PermissionOverwrite>> {
    sqlx::query("SELECT * FROM permission_overwrites WHERE channel_id=? ORDER BY kind,target_id")
        .bind(id(ch)?)
        .fetch_all(c)
        .await?
        .into_iter()
        .map(|r| {
            Ok(PermissionOverwrite {
                kind: crate::db::en(r.get("kind"))?,
                target_id: Id(r.get::<i64, _>("target_id") as u64),
                allow: Permissions::from_bits_retain(r.get::<i64, _>("allow") as u64),
                deny: Permissions::from_bits_retain(r.get::<i64, _>("deny") as u64),
            })
        })
        .collect()
}
pub(crate) async fn highest(c: &mut SqliteConnection, g: Id, u: Id) -> Result<u16> {
    Ok(sqlx::query_scalar::<_,i64>("SELECT COALESCE(MAX(position),0) FROM roles WHERE guild_id=? AND id IN(SELECT role_id FROM member_roles WHERE guild_id=? AND user_id=?)")
        .bind(id(g)?).bind(id(g)?).bind(id(u)?).fetch_one(c).await? as u16)
}
pub(crate) async fn above_member(
    c: &mut SqliteConnection,
    g: Id,
    actor: Id,
    target: Id,
) -> Result<()> {
    let owner: i64 = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    require(
        id(target)? != owner
            && (id(actor)? == owner || highest(c, g, actor).await? > highest(c, g, target).await?),
        ErrorCode::HierarchyViolation,
    )
}
pub(crate) async fn manage_role(c: &mut SqliteConnection, g: Id, u: Id, r: &Role) -> Result<()> {
    let p = evaluate(c, u, g, None).await?;
    require(p.contains(Permissions::MANAGE_ROLES), ErrorCode::Forbidden)?;
    require(
        p.contains(Permissions::OWNER) || highest(c, g, u).await? > r.position,
        ErrorCode::HierarchyViolation,
    )
}
