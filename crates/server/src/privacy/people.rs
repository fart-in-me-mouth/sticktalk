//! Private to the person who sets them: friend nicknames, notes about anyone,
//! and the ignore list. None of it is ever shown to the other person.
use super::Deliveries;
use crate::{
    db::*,
    error::{require, Result},
    http,
    state::now,
    App,
};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
};
use sqlx::{Row, SqliteConnection};
use sticktalk_proto::*;

/// Nicknames of current friends only (a former friend's nickname is not shown).
pub(crate) async fn nicknames(c: &mut SqliteConnection, u: Id) -> Result<Vec<FriendNickname>> {
    Ok(sqlx::query("SELECT n.other_id,n.nickname FROM friend_nicknames n JOIN relationships r ON r.user_id=n.user_id AND r.other_id=n.other_id AND r.kind='friend' WHERE n.user_id=? ORDER BY n.other_id LIMIT ?")
        .bind(id(u)?)
        .bind(MAX_FRIEND_NICKNAMES as i64)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| FriendNickname {
            user_id: Id(r.get::<i64, _>(0) as u64),
            nickname: r.get(1),
        })
        .collect())
}
/// Trimmed text, `None` when empty. `lines` allows line breaks (notes).
fn clean(text: Option<String>, max: usize, lines: bool) -> Result<Option<String>> {
    let Some(t) = text else { return Ok(None) };
    let t = t.trim();
    if t.is_empty() {
        return Ok(None);
    }
    require(
        t.chars().count() <= max && !t.chars().any(|c| c.is_control() && !(lines && c == '\n')),
        ErrorCode::InvalidRequest,
    )?;
    Ok(Some(t.to_owned()))
}
pub(crate) async fn set_nickname(
    c: &mut SqliteConnection,
    u: Id,
    v: Id,
    nickname: Option<String>,
) -> Result<()> {
    require(v != u, ErrorCode::InvalidRequest)?;
    let nickname = clean(nickname, MAX_FRIEND_NICKNAME_CHARS, false)?;
    let Some(nickname) = nickname else {
        sqlx::query("DELETE FROM friend_nicknames WHERE user_id=? AND other_id=?")
            .bind(id(u)?)
            .bind(id(v)?)
            .execute(&mut *c)
            .await?;
        return Ok(());
    };
    friends(c, u, v).await?;
    // Former friends' rows do not count, and are replaced as space is needed.
    sqlx::query("DELETE FROM friend_nicknames WHERE user_id=? AND other_id NOT IN (SELECT other_id FROM relationships WHERE user_id=? AND kind='friend')")
        .bind(id(u)?)
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM friend_nicknames WHERE user_id=? AND other_id<>?")
            .bind(id(u)?)
            .bind(id(v)?)
            .fetch_one(&mut *c)
            .await?;
    require(n < MAX_FRIEND_NICKNAMES as i64, ErrorCode::LimitExceeded)?;
    sqlx::query("INSERT INTO friend_nicknames(user_id,other_id,nickname) VALUES(?,?,?) ON CONFLICT(user_id,other_id) DO UPDATE SET nickname=excluded.nickname")
        .bind(id(u)?)
        .bind(id(v)?)
        .bind(nickname)
        .execute(&mut *c)
        .await?;
    Ok(())
}

pub(crate) async fn note(c: &mut SqliteConnection, u: Id, v: Id) -> Result<UserNote> {
    let r = sqlx::query("SELECT note,updated_at FROM user_notes WHERE user_id=? AND other_id=?")
        .bind(id(u)?)
        .bind(id(v)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(UserNote {
        user_id: v,
        note: r.as_ref().map(|r| r.get("note")).unwrap_or_default(),
        updated_at: r.map_or(0, |r| r.get::<i64, _>("updated_at") as u64),
    })
}
pub(crate) async fn set_note(
    c: &mut SqliteConnection,
    u: Id,
    v: Id,
    note: Option<String>,
) -> Result<UserNote> {
    require(v != u, ErrorCode::InvalidRequest)?;
    let note = clean(note, MAX_NOTE_CHARS, true)?;
    let Some(text) = note else {
        sqlx::query("DELETE FROM user_notes WHERE user_id=? AND other_id=?")
            .bind(id(u)?)
            .bind(id(v)?)
            .execute(&mut *c)
            .await?;
        return self::note(c, u, v).await;
    };
    profile_access(c, u, v).await?;
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM user_notes WHERE user_id=? AND other_id<>?")
            .bind(id(u)?)
            .bind(id(v)?)
            .fetch_one(&mut *c)
            .await?;
    require(n < MAX_NOTES as i64, ErrorCode::LimitExceeded)?;
    sqlx::query("INSERT INTO user_notes(user_id,other_id,note,updated_at) VALUES(?,?,?,?) ON CONFLICT(user_id,other_id) DO UPDATE SET note=excluded.note,updated_at=excluded.updated_at")
        .bind(id(u)?)
        .bind(id(v)?)
        .bind(text)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    self::note(c, u, v).await
}
/// `GET /api/v1/users/{id}/note`: your note about them (empty when none).
pub(crate) async fn note_endpoint(
    State(app): State<App>,
    Path(v): Path<String>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let v = http::url_id(&v)?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    profile_access(&mut c, u, v).await?;
    let data = note(&mut c, u, v).await?;
    Ok(http::json(Snapshot {
        data,
        cursor: core.cursor(&key, u),
    }))
}

pub(crate) async fn ignored(c: &mut SqliteConnection, u: Id) -> Result<Vec<UserSummary>> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT ignored_id FROM user_ignores WHERE user_id=? ORDER BY created_at LIMIT ?",
    )
    .bind(id(u)?)
    .bind(MAX_IGNORED_USERS as i64)
    .fetch_all(&mut *c)
    .await?;
    let mut out = vec![];
    for v in ids {
        out.push(user(c, Id(v as u64)).await?);
    }
    Ok(out)
}
pub(crate) async fn ignores(c: &mut SqliteConnection, u: Id, v: Id) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM user_ignores WHERE user_id=? AND ignored_id=?)",
    )
    .bind(id(u)?)
    .bind(id(v)?)
    .fetch_one(c)
    .await?)
}
pub(crate) async fn ignore(
    c: &mut SqliteConnection,
    u: Id,
    v: Id,
    on: bool,
    out: &mut Deliveries,
) -> Result<()> {
    require(v != u, ErrorCode::InvalidRequest)?;
    if on {
        user(c, v).await?;
        profile_access(c, u, v).await?;
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM user_ignores WHERE user_id=? AND ignored_id<>?",
        )
        .bind(id(u)?)
        .bind(id(v)?)
        .fetch_one(&mut *c)
        .await?;
        require(n < MAX_IGNORED_USERS as i64, ErrorCode::LimitExceeded)?;
        sqlx::query(
            "INSERT OR IGNORE INTO user_ignores(user_id,ignored_id,created_at) VALUES(?,?,?)",
        )
        .bind(id(u)?)
        .bind(id(v)?)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    } else {
        sqlx::query("DELETE FROM user_ignores WHERE user_id=? AND ignored_id=?")
            .bind(id(u)?)
            .bind(id(v)?)
            .execute(&mut *c)
            .await?;
    }
    recount_user(c, u, out).await
}

/// Unread and mention counts exclude ignored authors and suppressed mentions
/// (`db::recount` uses the same conditions for a channel).
pub(crate) async fn recount_user(
    c: &mut SqliteConnection,
    u: Id,
    out: &mut Deliveries,
) -> Result<()> {
    let before: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT channel_id,unread_count,mention_count FROM read_states WHERE user_id=?",
    )
    .bind(id(u)?)
    .fetch_all(&mut *c)
    .await?;
    sqlx::query("UPDATE read_states SET unread_count=(SELECT COUNT(*) FROM messages m WHERE m.channel_id=read_states.channel_id AND m.deleted_at IS NULL AND m.author_id<>read_states.user_id AND m.id>COALESCE(read_states.last_read_id,0) AND NOT EXISTS(SELECT 1 FROM user_ignores i WHERE i.user_id=read_states.user_id AND i.ignored_id=m.author_id)),mention_count=(SELECT COUNT(*) FROM messages m JOIN message_mentions mm ON mm.message_id=m.id WHERE m.channel_id=read_states.channel_id AND m.deleted_at IS NULL AND m.author_id<>read_states.user_id AND m.id>COALESCE(read_states.last_read_id,0) AND mm.user_id=read_states.user_id AND NOT EXISTS(SELECT 1 FROM user_ignores i WHERE i.user_id=read_states.user_id AND i.ignored_id=m.author_id) AND NOT EXISTS(SELECT 1 FROM suppressed_mentions s WHERE s.message_id=m.id AND s.user_id=read_states.user_id)) WHERE user_id=?")
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    let after: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT channel_id,unread_count,mention_count FROM read_states WHERE user_id=?",
    )
    .bind(id(u)?)
    .fetch_all(&mut *c)
    .await?;
    for row in after.iter().filter(|r| !before.contains(r)) {
        let ch = Id(row.0 as u64);
        if access(c, u, ch, Permissions::VIEW_CHANNEL).await.is_ok() {
            out.push((vec![u], Event::ReadStateUpdated(read(c, u, ch).await?)));
        }
    }
    Ok(())
}
