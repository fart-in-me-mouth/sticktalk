//! O(1) count plus at most 67 covering-index seeks. No COUNT/OFFSET on requests.
use crate::{db, error::Result, http, state::App};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
};
use sqlx::Row;
use sticktalk_proto::*;

/// The per-user budget for history reads (a page of messages and the history
/// extent; a first visit to a conversation needs one of each). 240 a minute
/// with a burst of 60 lets someone open about 30 conversations in a row
/// without waiting, then one every half second. Abuse stays bounded: each
/// read is index-only and at most 100 messages or 67 seeks, the budget is per
/// account on top of the shared 120-a-minute budget for other reads, and
/// sockets, commands and uploads keep their own limits.
pub(crate) fn rate(core: &mut crate::state::Core, u: Id) -> Result<()> {
    core.rate(format!("history:{}", u.0), 240., 60.)
}
/// `channels/{id}/messages`, the REST path of a history page.
pub(crate) fn is_history_page(path: &str) -> bool {
    matches!(
        path.split('/').collect::<Vec<_>>().as_slice(),
        ["channels", _, "messages"]
    )
}

pub(crate) async fn extent(
    State(app): State<App>,
    Path(ch): Path<u64>,
    headers: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = db::auth(&mut c, http::bearer(&headers)?).await?;
    rate(&mut core, u)?;
    let ch = Id(ch);
    db::text(
        &mut c,
        u,
        ch,
        Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
    )
    .await?;
    let count: Option<i64> =
        sqlx::query_scalar("SELECT message_count FROM history_extents WHERE channel_id=?")
            .bind(db::id(ch)?)
            .fetch_optional(&mut *c)
            .await?;
    let count = count.unwrap_or(0).max(0) as u64;
    let mut anchors = Vec::new();
    if count > 0 {
        let first = sqlx::query("SELECT ordinal,message_id FROM history_entries WHERE channel_id=? ORDER BY ordinal LIMIT 1").bind(db::id(ch)?).fetch_one(&mut *c).await?;
        let last = sqlx::query("SELECT ordinal,message_id FROM history_entries WHERE channel_id=? ORDER BY ordinal DESC LIMIT 1").bind(db::id(ch)?).fetch_one(&mut *c).await?;
        let start = first.get::<i64, _>("ordinal") as u64;
        let end = last.get::<i64, _>("ordinal") as u64;
        for i in 0..=64_u64 {
            let target = start + ((u128::from(end - start) * u128::from(i)) / 64) as u64;
            let r = sqlx::query("SELECT ordinal,message_id FROM history_entries WHERE channel_id=? AND ordinal>=? ORDER BY ordinal LIMIT 1").bind(db::id(ch)?).bind(target as i64).fetch_one(&mut *c).await?;
            let message_id = Id(r.get::<i64, _>("message_id") as u64);
            if anchors
                .last()
                .is_some_and(|a: &HistoryAnchor| a.message_id == message_id)
            {
                continue;
            }
            let rank = if end == start {
                0
            } else {
                ((u128::from((r.get::<i64, _>("ordinal") as u64) - start) * u128::from(count - 1))
                    / u128::from(end - start)) as u64
            };
            anchors.push(HistoryAnchor { rank, message_id });
        }
    }
    Ok(http::json(Snapshot {
        data: HistoryExtent {
            channel_id: ch,
            message_count: count,
            anchors,
        },
        cursor: core.cursor(&key, u),
    }))
}
