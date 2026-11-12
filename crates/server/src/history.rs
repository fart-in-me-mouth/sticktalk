//! O(1) count plus at most 67 covering-index seeks. No COUNT/OFFSET on requests.
use crate::{db, error::Result, http, state::App};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
};
use sqlx::Row;
use sticktalk_proto::*;

pub(crate) async fn extent(
    State(app): State<App>,
    Path(ch): Path<u64>,
    headers: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = db::auth(&mut c, http::bearer(&headers)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
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
