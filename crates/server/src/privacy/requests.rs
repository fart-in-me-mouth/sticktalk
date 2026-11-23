//! Who can start a DM with you, and message requests.
//!
//! Friends only is the default, as before. When someone allows members of the
//! servers they share, a first-contact DM is created as a request: the
//! recipient's side stays hidden (`dm_members.visible=0`), nothing they get is
//! more than a quiet count (`PrivacyEvent::Requests`), and `gate::filter`
//! keeps the conversation's events from them until they accept. The sender is
//! never told whether a request was accepted, ignored or marked as spam.
//!
//! Spam heuristic (bounded and simple): a burst of first contacts from one
//! account (5 within 10 minutes), a new account (under 3 days) starting 2
//! within 10 minutes, or a message that is only links while the request is
//! pending. First contacts are rate limited per sender (10 an hour, 30 a day).
use super::Deliveries;
use crate::{
    db::*,
    error::{require, Error, Result},
    http,
    state::now,
    App,
};
use axum::{extract::State, http::HeaderMap};
use sqlx::{Row, SqliteConnection};
use sticktalk_proto::*;
const BURST_WINDOW_MS: u64 = 10 * 60_000;
const BURST: i64 = 5;
const NEW_ACCOUNT_MS: u64 = 3 * 86_400_000;
const NEW_ACCOUNT_BURST: i64 = 2;

pub(crate) async fn dm_privacy(c: &mut SqliteConnection, u: Id) -> Result<DmPrivacy> {
    let allow: Option<String> = sqlx::query_scalar("SELECT allow FROM dm_privacy WHERE user_id=?")
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?;
    let servers = sqlx::query("SELECT guild_id,allow FROM dm_privacy_servers WHERE user_id=? AND guild_id IN (SELECT guild_id FROM guild_members WHERE user_id=?) ORDER BY guild_id")
        .bind(id(u)?)
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| DmServerOverride {
            guild_id: Id(r.get::<i64, _>(0) as u64),
            allow: r.get::<i64, _>(1) != 0,
        })
        .collect();
    Ok(DmPrivacy {
        allow: allow.map(en).transpose()?.unwrap_or_default(),
        servers,
    })
}
pub(crate) async fn set_dm_privacy(c: &mut SqliteConnection, u: Id, p: DmPrivacy) -> Result<()> {
    let ids: Vec<Id> = p.servers.iter().map(|s| s.guild_id).collect();
    unique(&ids, MAX_GUILDS_PER_USER)?;
    for g in &ids {
        role(c, u, *g).await?;
    }
    sqlx::query("INSERT INTO dm_privacy(user_id,allow) VALUES(?,?) ON CONFLICT(user_id) DO UPDATE SET allow=excluded.allow")
        .bind(id(u)?)
        .bind(enum_str(p.allow))
        .execute(&mut *c)
        .await?;
    sqlx::query("DELETE FROM dm_privacy_servers WHERE user_id=?")
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    for s in p.servers {
        sqlx::query("INSERT INTO dm_privacy_servers(user_id,guild_id,allow) VALUES(?,?,?)")
            .bind(id(u)?)
            .bind(id(s.guild_id)?)
            .bind(i64::from(s.allow))
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}
pub(crate) async fn counts(c: &mut SqliteConnection, u: Id) -> Result<RequestCounts> {
    let r = sqlx::query("SELECT COALESCE(SUM(state='pending'),0),COALESCE(SUM(state='spam'),0) FROM message_requests WHERE recipient_id=?")
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
    Ok(RequestCounts {
        pending: r.get::<i64, _>(0) as u32,
        spam: r.get::<i64, _>(1) as u32,
    })
}
async fn counts_event(c: &mut SqliteConnection, out: &mut Deliveries, u: Id) -> Result<()> {
    out.push((
        vec![u],
        Event::Privacy(PrivacyEvent::Requests(counts(c, u).await?)),
    ));
    Ok(())
}

/// `OpenDm` with no conversation yet. `Ok(false)`: friends, a normal DM.
/// `Ok(true)`: allowed as a message request. Refused otherwise.
pub(crate) async fn opening(c: &mut SqliteConnection, sender: Id, recipient: Id) -> Result<bool> {
    if friends(c, sender, recipient).await.is_ok() {
        return Ok(false);
    }
    let shared: Vec<i64> = sqlx::query_scalar("SELECT a.guild_id FROM guild_members a JOIN guild_members b ON a.guild_id=b.guild_id WHERE a.user_id=? AND b.user_id=? ORDER BY a.guild_id")
        .bind(id(sender)?)
        .bind(id(recipient)?)
        .fetch_all(&mut *c)
        .await?;
    let p = dm_privacy(c, recipient).await?;
    let allowed = shared.iter().any(|g| {
        p.servers
            .iter()
            .find(|o| o.guild_id.0 as i64 == *g)
            .map_or(p.allow == DmAllow::ServerMembers, |o| o.allow)
    });
    require(allowed, ErrorCode::Forbidden)?;
    // A server's paused DMs and verification level apply, as for any DM between non-friends.
    crate::moderation::dm_pair(c, sender, recipient).await?;
    let t = now();
    for (window, limit) in [
        (3_600_000, FIRST_CONTACTS_PER_HOUR),
        (86_400_000, FIRST_CONTACTS_PER_DAY),
    ] {
        let r = sqlx::query(
            "SELECT COUNT(*),MIN(created_at) FROM message_requests WHERE sender_id=? AND created_at>?",
        )
        .bind(id(sender)?)
        .bind(t.saturating_sub(window) as i64)
        .fetch_one(&mut *c)
        .await?;
        if r.get::<i64, _>(0) >= i64::from(limit) {
            let oldest = r.get::<Option<i64>, _>(1).unwrap_or(t as i64).max(0) as u64;
            let wait = (oldest + window)
                .saturating_sub(t)
                .clamp(1_000, u64::from(u32::MAX));
            return Err(Error::rate(wait as u32));
        }
    }
    Ok(true)
}
/// Record the new request (`opening` said yes) and give the recipient the count.
pub(crate) async fn record(
    c: &mut SqliteConnection,
    ch: Id,
    sender: Id,
    recipient: Id,
    out: &mut Deliveries,
) -> Result<()> {
    let t = now();
    let recent: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM message_requests WHERE sender_id=? AND created_at>?",
    )
    .bind(id(sender)?)
    .bind(t.saturating_sub(BURST_WINDOW_MS) as i64)
    .fetch_one(&mut *c)
    .await?;
    let created: i64 = sqlx::query_scalar("SELECT created_at FROM users WHERE id=?")
        .bind(id(sender)?)
        .fetch_one(&mut *c)
        .await?;
    let new_account = t.saturating_sub(created.max(0) as u64) < NEW_ACCOUNT_MS;
    // `recent` does not count this one yet.
    let spam = recent + 1 >= BURST || (new_account && recent + 1 >= NEW_ACCOUNT_BURST);
    sqlx::query("INSERT INTO message_requests(channel_id,recipient_id,sender_id,state,created_at) VALUES(?,?,?,?,?) ON CONFLICT(channel_id,recipient_id) DO NOTHING")
        .bind(id(ch)?)
        .bind(id(recipient)?)
        .bind(id(sender)?)
        .bind(if spam { "spam" } else { "pending" })
        .bind(t as i64)
        .execute(&mut *c)
        .await?;
    counts_event(c, out, recipient).await
}
/// Recipients whose side of DM `ch` is a request that is still hidden.
pub(crate) async fn hidden(c: &mut SqliteConnection, ch: Id) -> Result<Vec<Id>> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT recipient_id FROM message_requests WHERE channel_id=? AND state IN('pending','spam','ignored')")
    .bind(id(ch)?)
    .fetch_all(&mut *c)
    .await?
    .into_iter()
    .map(|v| Id(v as u64))
    .collect())
}
fn links_only(content: &str) -> bool {
    let mut words = content.split_whitespace().peekable();
    words.peek().is_some()
        && words.all(|w| {
            let w = w.trim_matches(|c| c == '<' || c == '>');
            w.starts_with("https://") || w.starts_with("http://")
        })
}
/// A message is being sent in DM `ch`. A recipient replying accepts their
/// request; a links-only message moves a pending request to spam. Returns the
/// recipients that must stay hidden.
pub(crate) async fn sending(
    c: &mut SqliteConnection,
    ch: Id,
    author: Id,
    content: &str,
    out: &mut Deliveries,
) -> Result<Vec<Id>> {
    let rows = sqlx::query("SELECT recipient_id,sender_id,state FROM message_requests WHERE channel_id=? AND state IN('pending','spam','ignored')")
    .bind(id(ch)?)
    .fetch_all(&mut *c)
    .await?;
    let mut hidden = vec![];
    for r in rows {
        let recipient = Id(r.get::<i64, _>(0) as u64);
        let sender = Id(r.get::<i64, _>(1) as u64);
        let state: String = r.get(2);
        if recipient == author {
            sqlx::query("UPDATE message_requests SET state='accepted' WHERE channel_id=? AND recipient_id=?")
                .bind(id(ch)?)
                .bind(id(recipient)?)
                .execute(&mut *c)
                .await?;
            counts_event(c, out, recipient).await?;
            continue;
        }
        if author == sender && state == "pending" && links_only(content) {
            sqlx::query(
                "UPDATE message_requests SET state='spam' WHERE channel_id=? AND recipient_id=?",
            )
            .bind(id(ch)?)
            .bind(id(recipient)?)
            .execute(&mut *c)
            .await?;
            counts_event(c, out, recipient).await?;
        }
        hidden.push(recipient);
    }
    Ok(hidden)
}
/// The recipient opened the conversation themselves (`OpenDm`): that accepts it.
pub(crate) async fn opened_by(
    c: &mut SqliteConnection,
    ch: Id,
    u: Id,
    out: &mut Deliveries,
) -> Result<()> {
    let n = sqlx::query("UPDATE message_requests SET state='accepted' WHERE channel_id=? AND recipient_id=? AND state IN('pending','spam','ignored')")
    .bind(id(ch)?)
    .bind(id(u)?)
    .execute(&mut *c)
    .await?
    .rows_affected();
    if n > 0 {
        counts_event(c, out, u).await?;
    }
    Ok(())
}
async fn request(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<Id> {
    let sender: Option<i64> = sqlx::query_scalar("SELECT sender_id FROM message_requests WHERE channel_id=? AND recipient_id=? AND state IN('pending','spam','ignored')")
    .bind(id(ch)?)
    .bind(id(u)?)
    .fetch_optional(&mut *c)
    .await?;
    sender
        .map(|v| Id(v as u64))
        .ok_or_else(|| Error::new(ErrorCode::NotFound))
}
pub(crate) async fn accept(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    out: &mut Deliveries,
) -> Result<()> {
    let sender = request(c, u, ch).await?;
    dm_quota(c, u, Some(ch)).await?;
    sqlx::query(
        "UPDATE message_requests SET state='accepted' WHERE channel_id=? AND recipient_id=?",
    )
    .bind(id(ch)?)
    .bind(id(u)?)
    .execute(&mut *c)
    .await?;
    sqlx::query("UPDATE dm_members SET visible=1 WHERE channel_id=? AND user_id=?")
        .bind(id(ch)?)
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    init_read(c, u, ch, false).await?;
    out.push((vec![u], Event::UserUpdated(user(c, sender).await?)));
    out.push((vec![u], Event::ChannelUpsert(channel(c, ch).await?)));
    out.push((vec![u], Event::ReadStateUpdated(read(c, u, ch).await?)));
    counts_event(c, out, u).await
}
pub(crate) async fn decline(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    out: &mut Deliveries,
) -> Result<()> {
    request(c, u, ch).await?;
    sqlx::query(
        "UPDATE message_requests SET state='ignored' WHERE channel_id=? AND recipient_id=?",
    )
    .bind(id(ch)?)
    .bind(id(u)?)
    .execute(&mut *c)
    .await?;
    counts_event(c, out, u).await
}

async fn list(c: &mut SqliteConnection, u: Id, state: &str) -> Result<Vec<MessageRequest>> {
    let rows = sqlx::query("SELECT channel_id,sender_id,created_at FROM message_requests WHERE recipient_id=? AND state=? ORDER BY created_at DESC LIMIT ?")
        .bind(id(u)?)
        .bind(state)
        .bind(MAX_LISTED_REQUESTS as i64)
        .fetch_all(&mut *c)
        .await?;
    let mut out = vec![];
    for r in rows {
        let ch = Id(r.get::<i64, _>(0) as u64);
        let sender = Id(r.get::<i64, _>(1) as u64);
        let mut mutual_servers = vec![];
        for g in sqlx::query_scalar::<_, i64>("SELECT a.guild_id FROM guild_members a JOIN guild_members b ON a.guild_id=b.guild_id WHERE a.user_id=? AND b.user_id=? ORDER BY a.guild_id LIMIT 10")
            .bind(id(u)?)
            .bind(id(sender)?)
            .fetch_all(&mut *c)
            .await?
        {
            mutual_servers.push(guild(c, u, Id(g as u64)).await?.guild);
        }
        let mut messages = vec![];
        for m in sqlx::query_scalar::<_, i64>("SELECT id FROM messages WHERE channel_id=? AND deleted_at IS NULL ORDER BY id DESC LIMIT 3")
            .bind(id(ch)?)
            .fetch_all(&mut *c)
            .await?
        {
            messages.push(message(c, u, ch, Id(m as u64), false).await?);
        }
        out.push(MessageRequest {
            channel_id: ch,
            user: user(c, sender).await?,
            created_at: r.get::<i64, _>(2) as u64,
            spam: state == "spam",
            mutual_servers,
            messages,
        });
    }
    Ok(out)
}
/// `GET /api/v1/users/@me/message-requests`
pub(crate) async fn endpoint(State(app): State<App>, h: HeaderMap) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let data = MessageRequests {
        requests: list(&mut c, u, "pending").await?,
        spam: list(&mut c, u, "spam").await?,
    };
    Ok(http::json(Snapshot {
        data,
        cursor: core.cursor(&key, u),
    }))
}

#[cfg(test)]
mod unit {
    #[test]
    fn links_only_messages() {
        assert!(super::links_only("https://a.example/x"));
        assert!(super::links_only(" <https://a.example> http://b.example "));
        assert!(!super::links_only("hi https://a.example"));
        assert!(!super::links_only("   "));
    }
}
