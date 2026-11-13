//! Shared ingress accounting. Declared sizes reserve disk/quota, never daily bytes.
use crate::{
    db::id,
    error::{require, Result},
    state::{now, App},
};
use sqlx::SqliteConnection;
use sticktalk_proto::{ErrorCode, Id};
pub const IDLE_MS: u64 = 3_600_000;
pub async fn admit(c: &mut SqliteConnection, user: Id) -> Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM partial_uploads WHERE owner_id=? AND attachment_id IS NULL AND expires_at>?)+(SELECT COUNT(*) FROM upload_reservations WHERE user_id=? AND expires_at>?)")
        .bind(id(user)?).bind(now() as i64).bind(id(user)?).bind(now() as i64).fetch_one(c).await?;
    require(count < 3, ErrorCode::LimitExceeded)
}
pub async fn quota(app: &App, c: &mut SqliteConnection, user: Id, additional: u64) -> Result<()> {
    for (owner, limit) in [
        (Some(id(user)?), app.0.config.operations.user_quota_mb),
        (None, app.0.config.max_total_upload_mb),
    ] {
        if limit == 0 {
            continue;
        }
        let used: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT SUM(byte_size+COALESCE(thumbnail_size,0)) FROM uploads WHERE ? IS NULL OR owner_id=?),0)+COALESCE((SELECT SUM(byte_size+65536) FROM partial_uploads WHERE attachment_id IS NULL AND expires_at>? AND (? IS NULL OR owner_id=?)),0)+COALESCE((SELECT SUM(reserved_bytes+65536) FROM upload_reservations WHERE expires_at>? AND (? IS NULL OR user_id=?)),0)")
            .bind(owner).bind(owner).bind(now() as i64).bind(owner).bind(owner).bind(now() as i64).bind(owner).bind(owner).fetch_one(&mut *c).await?;
        require(
            (used as u64).saturating_add(additional) <= limit.saturating_mul(1024 * 1024),
            ErrorCode::LimitExceeded,
        )?;
    }
    Ok(())
}
/// Commit each arriving chunk before disk I/O, including aborted/retried uploads.
/// No refund on abort/delete: a client cannot recycle its ingress allowance.
pub async fn charge(app: &App, user: Id, bytes: usize) -> Result<()> {
    let charged: i64 = sqlx::query_scalar("INSERT INTO upload_daily_usage(user_id,utc_day,charged_bytes) VALUES(?,?,?) ON CONFLICT(user_id,utc_day) DO UPDATE SET charged_bytes=MIN(9223372036854775807,charged_bytes+excluded.charged_bytes) RETURNING charged_bytes")
        .bind(id(user)?).bind((now()/86_400_000) as i64).bind(bytes as i64).fetch_one(&app.0.pool).await?;
    let budget = app
        .0
        .config
        .operations
        .user_daily_upload_mb
        .saturating_mul(1024 * 1024);
    require(
        budget == 0 || charged as u64 <= budget,
        ErrorCode::LimitExceeded,
    )
}
