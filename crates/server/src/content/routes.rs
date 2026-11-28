//! REST reads for rich content. Mutations are WebSocket commands.
use super::*;
use crate::http;
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
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
    http::budgeted(app, http::bearer(h)?, &[("rest", 120., 30.)]).await
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
    let (mut c, u) = http::budgeted(&app, http::bearer(&h)?, &[("file", 600., 120.)]).await?;
    let (kind, bytes) = previews::image(&mut c, u, Id(ch), Id(m), index).await?;
    // A message's preview images change only when its text does (an edit
    // fetches them again); the hash of the bytes names this version.
    let version = {
        use sha2::Digest;
        let d = sha2::Sha256::digest(&bytes);
        d[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let tag = crate::caching::etag("embed", m, format!("{index}-{version}"));
    if crate::caching::fresh(&h, &tag) {
        return Ok(crate::caching::not_modified(
            crate::caching::Policy::Checked,
            &tag,
        ));
    }
    Ok(crate::caching::image(
        crate::caching::Policy::Checked,
        &tag,
        &kind,
        bytes,
    ))
}
