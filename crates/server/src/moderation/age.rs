//! Age-restricted channels and everything nested inside them.
//!
//! A conversation is age-restricted when it, or any channel it is nested in,
//! is marked age-restricted: threads and forum posts inside an age-restricted
//! channel, and channels nested inside one, are restricted with it (task
//! `qa2`; before, every check looked at the conversation's own row only, so a
//! thread of an age-restricted channel was readable without confirming).
//! Confirming at a restricted channel covers everything nested inside it;
//! confirming from inside (a thread) confirms each restricted channel above.
//!
//! Every per-user path (`age_gate`: history, pins, saved messages and
//! reminders, link preview images, forward sources; `privacy::gate`: live
//! events, push, alerts, the Inbox, mentions; search) asks [`blocked`].
use crate::{db::id, error::Result, permissions::MAX_DEPTH};
use sqlx::SqliteConnection;
use std::collections::HashSet;
use sticktalk_proto::Id;

/// `ch` (?1) and the channels it is nested in, at most ?2 levels.
macro_rules! up {
    ($tail:literal) => {
        concat!(
            "WITH RECURSIVE up(id,parent_id,depth) AS (",
            "SELECT id,parent_id,0 FROM channels WHERE id=?1 ",
            "UNION ALL SELECT c.id,c.parent_id,up.depth+1 FROM channels c ",
            "JOIN up ON c.id=up.parent_id WHERE up.depth<?2) ",
            $tail
        )
    };
}

/// The age-restricted channels among `ch` and the channels it is nested in.
pub(crate) async fn roots(c: &mut SqliteConnection, ch: Id) -> Result<Vec<Id>> {
    let rows: Vec<i64> = sqlx::query_scalar(up!(
        "SELECT up.id FROM up JOIN channel_age_restricted r ON r.channel_id=up.id"
    ))
    .bind(id(ch)?)
    .bind(MAX_DEPTH)
    .fetch_all(&mut *c)
    .await?;
    Ok(rows.into_iter().map(|v| Id(v as u64)).collect())
}

/// `ch` is age-restricted (itself or above) and `u` has not confirmed every
/// restricted channel on the way.
pub(crate) async fn blocked(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<bool> {
    Ok(sqlx::query_scalar(up!(
        "SELECT EXISTS(SELECT 1 FROM up JOIN channel_age_restricted r ON r.channel_id=up.id WHERE NOT EXISTS(SELECT 1 FROM age_confirmations a WHERE a.channel_id=up.id AND a.user_id=?3))"
    ))
    .bind(id(ch)?)
    .bind(MAX_DEPTH)
    .bind(id(u)?)
    .fetch_one(&mut *c)
    .await?)
}

/// Keep only the people who may see content of `ch` (event fan-out).
pub(crate) async fn retain_confirmed(
    c: &mut SqliteConnection,
    ch: Id,
    users: &mut Vec<Id>,
) -> Result<()> {
    for root in roots(c, ch).await? {
        let confirmed: HashSet<i64> =
            sqlx::query_scalar("SELECT user_id FROM age_confirmations WHERE channel_id=?")
                .bind(id(root)?)
                .fetch_all(&mut *c)
                .await?
                .into_iter()
                .collect();
        users.retain(|u| confirmed.contains(&(u.0 as i64)));
    }
    Ok(())
}

/// `ConfirmAge` from `ch`: it and every restricted channel above it.
pub(crate) async fn confirm(c: &mut SqliteConnection, u: Id, ch: Id, at: u64) -> Result<()> {
    let mut targets = roots(c, ch).await?;
    if !targets.contains(&ch) {
        targets.push(ch);
    }
    for t in targets {
        sqlx::query("INSERT OR IGNORE INTO age_confirmations VALUES(?,?,?)")
            .bind(id(u)?)
            .bind(id(t)?)
            .bind(at as i64)
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}
