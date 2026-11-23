use crate::{
    error::{require, Error, Result},
    state::{hash, now, Core},
};
use serde::{de::DeserializeOwned, Serialize};
use sqlx::{sqlite::SqliteRow, Row, SqliteConnection};
use std::collections::{HashMap, HashSet};
use sticktalk_proto::*;
use subtle::ConstantTimeEq;
/// Run schema upgrades on a dedicated connection before accepting requests.
/// SQLite table rebuilds must not cascade-delete referencing rows. Always restore
/// FK enforcement, even on a migration error, before returning to a pool.
pub(crate) async fn migrate(c: &mut SqliteConnection) -> Result<()> {
    sqlx::query("PRAGMA foreign_keys=OFF")
        .execute(&mut *c)
        .await?;
    let result = sqlx::migrate!().run(&mut *c).await;
    sqlx::query("PRAGMA foreign_keys=ON")
        .execute(&mut *c)
        .await?;
    result.map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let violations = sqlx::query("PRAGMA foreign_key_check").fetch_all(c).await?;
    require(violations.is_empty(), ErrorCode::Unavailable)
}
pub fn id(v: Id) -> Result<i64> {
    require(v.0 > 0 && v.0 <= i64::MAX as u64, ErrorCode::InvalidRequest)?;
    Ok(v.0 as i64)
}
pub fn opt(v: Option<Id>) -> Result<Option<i64>> {
    v.map(id).transpose()
}
pub fn enum_str<T: Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}
pub fn en<T: DeserializeOwned>(s: String) -> Result<T> {
    serde_json::from_value(s.into()).map_err(|_| Error::new(ErrorCode::Internal))
}
pub fn username(s: &str) -> Result<String> {
    let s = s.trim().to_ascii_lowercase();
    require(
        (3..=32).contains(&s.len())
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        ErrorCode::InvalidRequest,
    )?;
    Ok(s)
}
pub fn name(s: &str, max: usize) -> Result<String> {
    let s = s.trim();
    require(
        !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )?;
    Ok(s.to_owned())
}
pub fn optional(s: Option<String>, max: usize) -> Result<Option<String>> {
    s.map(|s| name(&s, max)).transpose()
}
pub fn unique(ids: &[Id], max: usize) -> Result<()> {
    require(
        ids.len() <= max
            && ids.iter().all(|x| id(*x).is_ok())
            && ids.iter().copied().collect::<HashSet<_>>().len() == ids.len(),
        ErrorCode::InvalidRequest,
    )
}
pub async fn auth(c: &mut SqliteConnection, token: &str) -> Result<(Vec<u8>, Id)> {
    use base64::Engine;
    require(
        token.len() == 43
            && base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(token)
                .is_ok_and(|v| v.len() == 32),
        ErrorCode::Unauthenticated,
    )?;
    let h = hash(token.as_bytes());
    let r = sqlx::query("SELECT token_hash,user_id,expires_at FROM sessions WHERE token_hash=? AND NOT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=sessions.user_id)")
        .bind(&h)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::Unauthenticated))?;
    let stored: Vec<u8> = r.get("token_hash");
    require(bool::from(stored.ct_eq(&h)), ErrorCode::Unauthenticated)?;
    require(
        r.get::<i64, _>("expires_at") > now() as i64,
        ErrorCode::SessionExpired,
    )?;
    sqlx::query("UPDATE sessions SET last_used_at=? WHERE token_hash=? AND last_used_at<?")
        .bind(now() as i64)
        .bind(&h)
        .bind(now().saturating_sub(60_000) as i64)
        .execute(&mut *c)
        .await?;
    Ok((h, Id(r.get::<i64, _>("user_id") as u64)))
}
pub fn user_row(r: &SqliteRow) -> UserSummary {
    UserSummary {
        id: Id(r.get::<i64, _>("id") as u64),
        username: r.get("username"),
        display_name: r.get("display_name"),
        avatar_id: r.get::<Option<i64>, _>("avatar_id").map(|v| Id(v as u64)),
        status_text: r.get("status_text"),
        style: Box::new(crate::profiles::style_column(r)),
    }
}
pub async fn user(c: &mut SqliteConnection, u: Id) -> Result<UserSummary> {
    let r = sqlx::query("SELECT * FROM users WHERE id=?")
        .bind(id(u)?)
        .fetch_optional(c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok(user_row(&r))
}
pub async fn me(c: &mut SqliteConnection, u: Id) -> Result<SelfUser> {
    let r = sqlx::query("SELECT * FROM users WHERE id=?")
        .bind(id(u)?)
        .fetch_one(c)
        .await?;
    Ok(SelfUser {
        user: user_row(&r),
        desired_presence: en(r.get("desired_presence"))?,
    })
}
pub async fn profile_access(c: &mut SqliteConnection, viewer: Id, target: Id) -> Result<()> {
    let n:i64=sqlx::query_scalar("SELECT (?=?) OR EXISTS(SELECT 1 FROM relationships WHERE user_id=? AND other_id=?) OR EXISTS(SELECT 1 FROM guild_members a JOIN guild_members b ON a.guild_id=b.guild_id WHERE a.user_id=? AND b.user_id=?) OR EXISTS(SELECT 1 FROM dm_members a JOIN dm_members b ON a.channel_id=b.channel_id WHERE a.user_id=? AND b.user_id=?)")
    .bind(id(viewer)?).bind(id(target)?).bind(id(viewer)?).bind(id(target)?).bind(id(viewer)?).bind(id(target)?).bind(id(viewer)?).bind(id(target)?).fetch_one(&mut *c).await?;
    if n != 0 {
        return Ok(());
    }
    let candidates: Vec<i64> = sqlx::query_scalar("SELECT DISTINCT m.channel_id FROM messages m WHERE m.author_id=? AND m.deleted_at IS NULL AND (m.channel_id IN(SELECT ch.id FROM channels ch JOIN guild_members gm ON gm.guild_id=ch.guild_id WHERE gm.user_id=?) OR m.channel_id IN(SELECT channel_id FROM dm_members WHERE user_id=?))")
        .bind(id(target)?).bind(id(viewer)?).bind(id(viewer)?).fetch_all(&mut *c).await?;
    for ch in candidates {
        if access(c, viewer, Id(ch as u64), Permissions::READ_HISTORY)
            .await
            .is_ok()
        {
            return Ok(());
        }
    }
    Err(Error::new(ErrorCode::NotFound))
}
pub async fn role(c: &mut SqliteConnection, u: Id, g: Id) -> Result<MemberRole> {
    let r: Option<String> =
        sqlx::query_scalar("SELECT role FROM guild_members WHERE guild_id=? AND user_id=?")
            .bind(id(g)?)
            .bind(id(u)?)
            .fetch_optional(c)
            .await?;
    en(r.ok_or_else(|| Error::new(ErrorCode::NotFound))?)
}
pub async fn guild_perm(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    p: Permissions,
) -> Result<MemberRole> {
    let r = role(c, u, g).await?;
    require(
        crate::permissions::evaluate(c, u, g, None)
            .await?
            .contains(p),
        if p == Permissions::OWNER {
            ErrorCode::OwnerRequired
        } else {
            ErrorCode::Forbidden
        },
    )?;
    Ok(r)
}
pub async fn member(c: &mut SqliteConnection, g: Id, u: Id) -> Result<Member> {
    let r = sqlx::query("SELECT role,joined_at FROM guild_members WHERE guild_id=? AND user_id=?")
        .bind(id(g)?)
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let role = en(r.get("role"))?;
    Ok(Member {
        guild_id: g,
        user: user(c, u).await?,
        role,
        permissions: crate::permissions::evaluate(c, u, g, None).await?,
        role_ids: crate::permissions::role_ids(c, g, u).await?,
        timeout_until: crate::moderation::timeout_until(c, g, u).await?,
        server_style: Box::new(crate::profiles::member_style(c, g, u).await?),
        nickname: sqlx::query_scalar(
            "SELECT nickname FROM nicknames WHERE guild_id=? AND user_id=?",
        )
        .bind(id(g)?)
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?,
        joined_at: r.get::<i64, _>("joined_at") as u64,
    })
}
pub async fn guild_users(c: &mut SqliteConnection, g: Id) -> Result<Vec<Id>> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT user_id FROM guild_members WHERE guild_id=? ORDER BY user_id",
    )
    .bind(id(g)?)
    .fetch_all(c)
    .await?
    .into_iter()
    .map(|v| Id(v as u64))
    .collect())
}
pub async fn guild(c: &mut SqliteConnection, u: Id, g: Id) -> Result<GuildSummary> {
    let role = role(c, u, g).await?;
    let r=sqlx::query("SELECT *, (SELECT COUNT(*) FROM guild_members WHERE guild_id=guilds.id) AS n FROM guilds WHERE id=?").bind(id(g)?).fetch_one(&mut *c).await?;
    Ok(GuildSummary {
        guild: Guild {
            id: g,
            owner_id: Id(r.get::<i64, _>("owner_id") as u64),
            name: r.get("name"),
            icon_id: r.get::<Option<i64>, _>("icon_id").map(|v| Id(v as u64)),
            member_count: r.get::<i64, _>("n") as u32,
        },
        my_role: role,
        my_permissions: crate::permissions::evaluate(c, u, g, None).await?,
    })
}
pub async fn channel(c: &mut SqliteConnection, ch: Id) -> Result<Channel> {
    let r = sqlx::query("SELECT * FROM channels WHERE id=?")
        .bind(id(ch)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let recipient_ids = sqlx::query_scalar::<_, i64>(
        "SELECT user_id FROM dm_members WHERE channel_id=? ORDER BY user_id",
    )
    .bind(id(ch)?)
    .fetch_all(c)
    .await?
    .into_iter()
    .map(|v| Id(v as u64))
    .collect();
    Ok(Channel {
        summary: ChannelSummary {
            id: ch,
            kind: en(r.get("kind"))?,
            guild_id: r.get::<Option<i64>, _>("guild_id").map(|v| Id(v as u64)),
            parent_id: r.get::<Option<i64>, _>("parent_id").map(|v| Id(v as u64)),
            name: r.get("name"),
            position: r.get::<i64, _>("position") as u16,
            recipient_ids,
            owner_id: r.get::<Option<i64>, _>("owner_id").map(|v| Id(v as u64)),
        },
        topic: r.get("topic"),
    })
}
pub async fn access(c: &mut SqliteConnection, u: Id, ch: Id, p: Permissions) -> Result<Channel> {
    let mut channel = channel(c, ch).await?;
    if let Some(g) = channel.summary.guild_id {
        let effective = crate::permissions::evaluate(c, u, g, Some(ch)).await?;
        require(
            effective.contains(Permissions::VIEW_CHANNEL),
            ErrorCode::NotFound,
        )?;
        require(effective.contains(p), ErrorCode::Forbidden)?;
        channel.summary.parent_id = visible_parent(c, u, g, channel.summary.parent_id).await?;
    } else {
        require(
            channel.summary.recipient_ids.contains(&u),
            ErrorCode::NotFound,
        )?;
    }
    Ok(channel)
}
/// The nearest channel above a nested one that `u` can see, starting at `parent`.
/// A channel inside one its viewer cannot see is shown one level further out (at
/// the top when nothing above it is visible), so a hidden channel's ID never leaks.
pub async fn visible_parent(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    mut parent: Option<Id>,
) -> Result<Option<Id>> {
    for _ in 0..crate::permissions::MAX_DEPTH {
        let Some(p) = parent else { break };
        if crate::permissions::evaluate(c, u, g, Some(p))
            .await?
            .contains(Permissions::VIEW_CHANNEL)
        {
            return Ok(Some(p));
        }
        parent = sqlx::query_scalar::<_, Option<i64>>("SELECT parent_id FROM channels WHERE id=?")
            .bind(id(p)?)
            .fetch_optional(&mut *c)
            .await?
            .flatten()
            .map(|v| Id(v as u64));
    }
    Ok(None)
}
pub async fn text(c: &mut SqliteConnection, u: Id, ch: Id, p: Permissions) -> Result<Channel> {
    let channel = access(c, u, ch, p).await?;
    // A voice channel keeps its own text chat, under the same permission bits.
    require(
        matches!(
            channel.summary.kind,
            ChannelKind::Text | ChannelKind::Direct | ChannelKind::GroupDm | ChannelKind::Voice
        ),
        ErrorCode::InvalidRequest,
    )?;
    Ok(channel)
}
pub async fn audience(c: &mut SqliteConnection, ch: Id) -> Result<Vec<Id>> {
    let ch = channel(c, ch).await?;
    if let Some(g) = ch.summary.guild_id {
        crate::capacity::members_with(c, g, Some(ch.summary.id), Permissions::VIEW_CHANNEL).await
    } else {
        Ok(ch.summary.recipient_ids)
    }
}
pub async fn channels(
    c: &mut SqliteConnection,
    u: Id,
    g: Option<Id>,
) -> Result<Vec<ChannelSummary>> {
    // Server channels are loaded and evaluated a server at a time (see
    // `capacity`); only direct messages are read one by one.
    let (guilds, dms): (Vec<i64>, Vec<i64>) = if let Some(g) = g {
        role(c, u, g).await?;
        (vec![id(g)?], vec![])
    } else {
        (
            sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE user_id=? ORDER BY guild_id")
                .bind(id(u)?)
                .fetch_all(&mut *c)
                .await?,
            sqlx::query_scalar("SELECT channel_id FROM dm_members WHERE user_id=? AND visible=1 ORDER BY channel_id")
                .bind(id(u)?)
                .fetch_all(&mut *c)
                .await?,
        )
    };
    let mut list = vec![];
    let mut parents: HashMap<Id, Option<Id>> = HashMap::new();
    for g in guilds {
        for (ch, visible) in crate::capacity::guild_channels(c, u, Id(g as u64)).await? {
            parents.insert(ch.id, ch.parent_id);
            if visible {
                list.push(ch);
            }
        }
    }
    for i in dms {
        list.push(channel(c, Id(i as u64)).await?.summary);
    }
    let visible: HashSet<Id> = list.iter().map(|c| c.id).collect();
    let (nested, mut dms): (Vec<_>, Vec<_>) = list.into_iter().partition(|c| c.guild_id.is_some());
    // A channel inside one the user cannot see is listed under the nearest visible
    // channel above it (see `visible_parent`), after that channel's own children.
    let mut children: HashMap<Option<Id>, Vec<(bool, ChannelSummary)>> = HashMap::new();
    for mut ch in nested {
        let inside = ch.parent_id;
        for _ in 0..=parents.len() {
            match ch.parent_id {
                Some(p) if !visible.contains(&p) => {
                    ch.parent_id = parents.get(&p).copied().flatten()
                }
                _ => break,
            }
        }
        if ch.parent_id.is_some_and(|p| !visible.contains(&p)) {
            ch.parent_id = None;
        }
        children
            .entry(ch.parent_id)
            .or_default()
            .push((ch.parent_id != inside, ch));
    }
    let mut sorted = |parent: Option<Id>| {
        let mut v = children.remove(&parent).unwrap_or_default();
        // Reversed, because the walk below takes channels from the end.
        v.sort_by_key(|(moved, c)| std::cmp::Reverse((c.guild_id, *moved, c.position, c.id)));
        v.into_iter().map(|(_, c)| c)
    };
    // Tree ordering: each channel is followed by everything nested in it; DMs last by ID.
    let mut out = vec![];
    let mut next: Vec<ChannelSummary> = sorted(None).collect();
    while let Some(ch) = next.pop() {
        next.extend(sorted(Some(ch.id)));
        out.push(ch);
    }
    dms.sort_by_key(|c| c.id);
    out.extend(dms);
    Ok(out)
}
pub async fn detail(c: &mut SqliteConnection, u: Id, g: Id) -> Result<GuildDetail> {
    Ok(GuildDetail {
        summary: guild(c, u, g).await?,
        channels: channels(c, u, Some(g)).await?,
    })
}
pub async fn relationships(c: &mut SqliteConnection, u: Id) -> Result<Vec<Relationship>> {
    let rows = sqlx::query(
        "SELECT other_id,kind,since FROM relationships WHERE user_id=? ORDER BY other_id",
    )
    .bind(id(u)?)
    .fetch_all(&mut *c)
    .await?;
    let mut out = vec![];
    for r in rows {
        out.push(Relationship {
            user: user(c, Id(r.get::<i64, _>("other_id") as u64)).await?,
            kind: en(r.get("kind"))?,
            since: r.get::<i64, _>("since") as u64,
        });
    }
    Ok(out)
}
pub async fn relationship(c: &mut SqliteConnection, u: Id, v: Id) -> Result<Option<Relationship>> {
    let r = sqlx::query("SELECT kind,since FROM relationships WHERE user_id=? AND other_id=?")
        .bind(id(u)?)
        .bind(id(v)?)
        .fetch_optional(&mut *c)
        .await?;
    match r {
        Some(r) => Ok(Some(Relationship {
            user: user(c, v).await?,
            kind: en(r.get("kind"))?,
            since: r.get::<i64, _>("since") as u64,
        })),
        None => Ok(None),
    }
}
pub async fn blocked(c: &mut SqliteConnection, a: Id, b: Id) -> Result<bool> {
    let n:i64=sqlx::query_scalar("SELECT COUNT(*) FROM relationships WHERE kind='blocked' AND ((user_id=? AND other_id=?) OR (user_id=? AND other_id=?))").bind(id(a)?).bind(id(b)?).bind(id(b)?).bind(id(a)?).fetch_one(c).await?;
    Ok(n > 0)
}
pub async fn friends(c: &mut SqliteConnection, a: Id, b: Id) -> Result<()> {
    require(
        relationship(c, a, b)
            .await?
            .is_some_and(|r| r.kind == RelationshipKind::Friend),
        ErrorCode::Forbidden,
    )
}
pub async fn dm_quota(c: &mut SqliteConnection, u: Id, ch: Option<Id>) -> Result<()> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM dm_members WHERE user_id=? AND visible=1 AND channel_id<>?",
    )
    .bind(id(u)?)
    .bind(opt(ch)?.unwrap_or(0))
    .fetch_one(c)
    .await?;
    require(n < 64, ErrorCode::LimitExceeded)
}
pub async fn guild_quota(c: &mut SqliteConnection, u: Id) -> Result<()> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE user_id=?")
        .bind(id(u)?)
        .fetch_one(c)
        .await?;
    require(n < MAX_GUILDS_PER_USER as i64, ErrorCode::LimitExceeded)
}
pub fn attachment_row(r: &SqliteRow) -> Attachment {
    Attachment {
        id: Id(r.get::<i64, _>("id") as u64),
        filename: r.get("filename"),
        content_type: r.get("content_type"),
        byte_size: r.get::<i64, _>("byte_size") as u64,
        width: r.get::<Option<i64>, _>("width").map(|v| v as u32),
        height: r.get::<Option<i64>, _>("height").map(|v| v as u32),
        thumbnail: r
            .get::<Option<String>, _>("thumbnail_key")
            .map(|_| Thumbnail {
                width: r.get::<i64, _>("thumbnail_width") as u16,
                height: r.get::<i64, _>("thumbnail_height") as u16,
                byte_size: r.get::<i64, _>("thumbnail_size") as u32,
                content_type: r.get("thumbnail_type"),
            }),
        meta: crate::content::attachment_meta(r),
    }
}
pub async fn message(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    m: Id,
    deleted: bool,
) -> Result<Message> {
    let r = sqlx::query("SELECT * FROM messages WHERE id=? AND channel_id=?")
        .bind(id(m)?)
        .bind(id(ch)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    require(
        deleted || r.get::<Option<i64>, _>("deleted_at").is_none(),
        ErrorCode::NotFound,
    )?;
    let attachments=sqlx::query("SELECT u.* FROM uploads u JOIN message_attachments a ON a.attachment_id=u.id WHERE a.message_id=? ORDER BY a.position").bind(id(m)?).fetch_all(&mut *c).await?.iter().map(attachment_row).collect();
    let mention_ids = sqlx::query_scalar::<_, i64>(
        "SELECT user_id FROM message_mentions WHERE message_id=? ORDER BY user_id",
    )
    .bind(id(m)?)
    .fetch_all(&mut *c)
    .await?
    .into_iter()
    .map(|v| Id(v as u64))
    .collect();
    let call = crate::calls::summary(c, m).await?;
    let reactions=sqlx::query("SELECT emoji,COUNT(*) AS n,MAX(user_id=?) AS me FROM reactions WHERE message_id=? GROUP BY emoji ORDER BY emoji").bind(id(u)?).bind(id(m)?).fetch_all(&mut *c).await?.into_iter().map(|r|ReactionSummary {emoji:r.get("emoji"),count:r.get::<i64,_>("n") as u32,me:r.get::<i64,_>("me")!=0}).collect();
    let mut message = Message {
        id: m,
        channel_id: ch,
        author_id: Id(r.get::<i64, _>("author_id") as u64),
        content: r.get("content"),
        created_at: r.get::<i64, _>("created_at") as u64,
        edited_at: r.get::<Option<i64>, _>("edited_at").map(|v| v as u64),
        reply_to: r.get::<Option<i64>, _>("reply_to").map(|v| Id(v as u64)),
        attachments,
        mention_ids,
        reactions,
        call,
        extra: Default::default(),
    };
    crate::content::decorate(c, u, &mut message).await?;
    Ok(message)
}
pub async fn read(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<ReadState> {
    let r=sqlx::query("SELECT last_read_id,unread_count,mention_count,(SELECT MAX(id) FROM messages WHERE channel_id=? AND deleted_at IS NULL) AS latest FROM read_states WHERE user_id=? AND channel_id=?").bind(id(ch)?).bind(id(u)?).bind(id(ch)?).fetch_one(c).await?;
    Ok(ReadState {
        channel_id: ch,
        last_read_id: r
            .get::<Option<i64>, _>("last_read_id")
            .map(|v| Id(v as u64)),
        latest_message_id: r.get::<Option<i64>, _>("latest").map(|v| Id(v as u64)),
        unread_count: u32::try_from(r.get::<i64, _>("unread_count"))
            .map_err(|_| Error::new(ErrorCode::LimitExceeded))?,
        mention_count: u32::try_from(r.get::<i64, _>("mention_count"))
            .map_err(|_| Error::new(ErrorCode::LimitExceeded))?,
    })
}
pub async fn init_read(c: &mut SqliteConnection, u: Id, ch: Id, caught_up: bool) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO read_states(user_id,channel_id,last_read_id) VALUES(?,?,CASE WHEN ? THEN (SELECT MAX(id) FROM messages WHERE channel_id=?) ELSE NULL END)").bind(id(u)?).bind(id(ch)?).bind(caught_up).bind(id(ch)?).execute(c).await?;
    Ok(())
}
pub async fn recount(c: &mut SqliteConnection, ch: Id) -> Result<()> {
    sqlx::query("UPDATE read_states SET unread_count=(SELECT COUNT(*) FROM messages m WHERE m.channel_id=read_states.channel_id AND m.deleted_at IS NULL AND m.author_id<>read_states.user_id AND m.id>COALESCE(read_states.last_read_id,0)),mention_count=(SELECT COUNT(*) FROM messages m JOIN message_mentions mm ON mm.message_id=m.id WHERE m.channel_id=read_states.channel_id AND m.deleted_at IS NULL AND m.author_id<>read_states.user_id AND m.id>COALESCE(read_states.last_read_id,0) AND mm.user_id=read_states.user_id) WHERE channel_id=?").bind(id(ch)?).execute(c).await?;
    Ok(())
}
pub async fn read_states(c: &mut SqliteConnection, u: Id) -> Result<Vec<ReadState>> {
    let mut out = vec![];
    for ch in channels(c, u, None).await? {
        if matches!(
            ch.kind,
            ChannelKind::Text | ChannelKind::Direct | ChannelKind::GroupDm
        ) {
            out.push(read(c, u, ch.id).await?);
        } else if ch.kind == ChannelKind::Voice {
            init_read(c, u, ch.id, true).await?;
            out.push(read(c, u, ch.id).await?);
        }
    }
    out.sort_by_key(|r| r.channel_id);
    Ok(out)
}
pub async fn presence(
    c: &mut SqliteConnection,
    core: &Core,
    viewer: Id,
    u: Id,
) -> Result<Presence> {
    let online = core.streams.values().any(|s| {
        s.user == u
            && (s.connection.is_some()
                || (s.ever_connected && now().saturating_sub(s.touched) < 5000))
    });
    let mut status = if online {
        me(c, u).await?.desired_presence
    } else {
        PresenceStatus::Offline
    };
    if viewer != u && (status == PresenceStatus::Invisible || blocked(c, viewer, u).await?) {
        status = PresenceStatus::Offline;
    }
    Ok(Presence { user_id: u, status })
}
pub async fn base_presence(c: &mut SqliteConnection, u: Id) -> Result<Vec<Id>> {
    Ok(sqlx::query_scalar::<_,i64>("SELECT peer FROM (SELECT other_id AS peer FROM relationships WHERE user_id=? AND kind='friend' UNION SELECT b.user_id AS peer FROM dm_members a JOIN dm_members b ON a.channel_id=b.channel_id WHERE a.user_id=? AND a.visible=1) WHERE peer<>? AND NOT EXISTS(SELECT 1 FROM relationships r WHERE r.kind='blocked' AND ((r.user_id=? AND r.other_id=peer) OR (r.user_id=peer AND r.other_id=?))) ORDER BY peer").bind(id(u)?).bind(id(u)?).bind(id(u)?).bind(id(u)?).bind(id(u)?).fetch_all(c).await?.into_iter().map(|v|Id(v as u64)).collect())
}
pub async fn ready(c: &mut SqliteConnection, core: &mut Core, key: &[u8], u: Id) -> Result<Ready> {
    let mut guilds = vec![];
    for g in sqlx::query_scalar::<_, i64>(
        "SELECT guild_id FROM guild_members WHERE user_id=? ORDER BY guild_id",
    )
    .bind(id(u)?)
    .fetch_all(&mut *c)
    .await?
    {
        guilds.push(guild(c, u, Id(g as u64)).await?);
    }
    let channels = channels(c, u, None).await?;
    let relationships = relationships(c, u).await?;
    let mut ids: Vec<_> = channels
        .iter()
        .filter(|ch| ch.kind == ChannelKind::Direct)
        .flat_map(|ch| ch.recipient_ids.iter())
        .copied()
        .filter(|v| *v != u && !relationships.iter().any(|r| r.user.id == *v))
        .collect();
    ids.sort();
    ids.dedup();
    let mut users = vec![];
    for v in ids {
        users.push(user(c, v).await?);
    }
    let mut ids = base_presence(c, u).await?;
    ids.push(u);
    ids.sort();
    ids.dedup();
    let mut presences = vec![];
    for v in ids {
        presences.push(presence(c, core, u, v).await?);
    }
    let visible_channels: Vec<_> = channels.iter().map(|ch| ch.id).collect();
    let mut roles = vec![];
    let mut memberships = vec![];
    for g in &guilds {
        roles.extend(crate::permissions::roles(c, g.guild.id).await?);
        memberships.push(member(c, g.guild.id, u).await?);
    }
    Ok(Ready {
        roles,
        memberships,
        notification_settings: crate::launch::notifications(c, u).await?,
        cursor: core.cursor(key, u),
        me: me(c, u).await?,
        guilds,
        channels,
        relationships,
        users,
        read_states: read_states(c, u).await?,
        presences,
        voice_states: core
            .voice
            .values()
            .filter(|v| visible_channels.contains(&v.state.channel_id))
            .filter(|v| v.joined && !v.dm())
            .map(|v| v.state.clone())
            .collect(),
    })
}
pub async fn invite(c: &mut SqliteConnection, i: Id) -> Result<Invite> {
    let r = sqlx::query("SELECT * FROM invites WHERE id=?")
        .bind(id(i)?)
        .fetch_one(c)
        .await?;
    Ok(Invite {
        id: i,
        code: r.get("code"),
        guild_id: Id(r.get::<i64, _>("guild_id") as u64),
        creator_id: Id(r.get::<i64, _>("creator_id") as u64),
        expires_at: r.get::<i64, _>("expires_at") as u64,
        max_uses: r.get::<i64, _>("max_uses") as u32,
        uses: r.get::<i64, _>("uses") as u32,
    })
}
pub async fn ban(c: &mut SqliteConnection, g: Id, u: Id) -> Result<Ban> {
    let r = sqlx::query("SELECT * FROM guild_bans WHERE guild_id=? AND user_id=?")
        .bind(id(g)?)
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
    Ok(Ban {
        guild_id: g,
        user: user(c, u).await?,
        actor_id: Id(r.get::<i64, _>("actor_id") as u64),
        reason: r.get("reason"),
        created_at: r.get::<i64, _>("created_at") as u64,
    })
}
pub async fn gc_uploads(c: &mut SqliteConnection) -> Result<Vec<String>> {
    let rows=sqlx::query("SELECT id,storage_key,thumbnail_key FROM uploads u WHERE expires_at IS NULL AND NOT EXISTS(SELECT 1 FROM message_attachments WHERE attachment_id=u.id) AND NOT EXISTS(SELECT 1 FROM forwarded_attachments WHERE attachment_id=u.id) AND NOT EXISTS(SELECT 1 FROM users WHERE avatar_id=u.id) AND NOT EXISTS(SELECT 1 FROM guilds WHERE icon_id=u.id) AND NOT EXISTS(SELECT 1 FROM profile_uploads WHERE upload_id=u.id) LIMIT 100").fetch_all(&mut *c).await?;
    let mut keys = vec![];
    for r in rows {
        keys.push(r.get("storage_key"));
        if let Some(k) = r.get::<Option<String>, _>("thumbnail_key") {
            keys.push(k);
        }
        sqlx::query("DELETE FROM uploads WHERE id=?")
            .bind(r.get::<i64, _>("id"))
            .execute(&mut *c)
            .await?;
    }
    Ok(keys)
}

pub async fn enabled(c: &mut SqliteConnection, user: Id) -> Result<()> {
    let disabled: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=?)")
            .bind(id(user)?)
            .fetch_one(c)
            .await?;
    require(!disabled, ErrorCode::InvalidCredentials)
}
