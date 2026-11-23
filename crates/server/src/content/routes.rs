//! REST reads for rich content. Mutations are WebSocket commands.
use super::*;
use crate::http;
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::get,
};

pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route(
            "/api/v1/channels/{channel}/messages/{message}/poll/answers/{answer}/voters",
            get(voters),
        )
        .route(
            "/api/v1/channels/{channel}/messages/{message}/embeds/{index}/image",
            get(embed_image),
        )
        .route("/api/v1/users/@me/scheduled-messages", get(scheduled))
}

#[derive(serde::Deserialize)]
struct After {
    after: Option<u64>,
}
#[derive(serde::Deserialize)]
struct ChannelFilter {
    channel_id: Option<u64>,
}

async fn who(app: &App, h: &HeaderMap) -> Result<(sqlx::pool::PoolConnection<sqlx::Sqlite>, Id)> {
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, http::bearer(h)?).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("rest:{}", u.0), 120., 30.)?;
    Ok((c, u))
}

async fn voters(
    State(app): State<App>,
    Path((ch, m, answer)): Path<(u64, u64, u8)>,
    Query(q): Query<After>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    require(
        usize::from(answer) < MAX_POLL_ANSWERS,
        ErrorCode::InvalidRequest,
    )?;
    let page = polls::voters(&mut c, u, Id(ch), Id(m), answer, q.after.map(Id)).await?;
    Ok(http::json(page))
}

async fn scheduled(
    State(app): State<App>,
    Query(q): Query<ChannelFilter>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    let page = scheduled::list(&mut c, u, q.channel_id.map(Id)).await?;
    Ok(http::json(page))
}

async fn embed_image(
    State(app): State<App>,
    Path((ch, m, index)): Path<(u64, u64, u8)>,
    h: HeaderMap,
) -> Result<Response> {
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, http::bearer(&h)?).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("file:{}", u.0), 600., 120.)?;
    let (kind, bytes) = previews::image(&mut c, u, Id(ch), Id(m), index).await?;
    Ok((
        [
            ("content-type", kind),
            ("cache-control", "private, max-age=86400".into()),
            ("x-content-type-options", "nosniff".into()),
            (
                "content-security-policy",
                "default-src 'none'; sandbox".into(),
            ),
        ],
        bytes,
    )
        .into_response())
}
