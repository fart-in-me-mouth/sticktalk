//! REST reads for the community tools. Every mutation is a WebSocket command.
use super::*;
use crate::{http, state::App};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::get,
    Router,
};

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/guilds/{id}/community", get(community))
        .route("/api/v1/guilds/{id}/applications", get(applications))
        .route("/api/v1/guilds/{id}/listing", get(listing))
        .route("/api/v1/guilds/{id}/templates", get(guild_templates))
        .route("/api/v1/guilds/{id}/insights", get(insights_page))
        .route("/api/v1/users/@me/applications", get(my_applications))
        .route("/api/v1/invites/{code}/admission", get(invite_gate))
        .route("/api/v1/directory", get(directory_page))
        .route("/api/v1/directory/{id}", get(directory_preview))
        .route("/api/v1/templates", get(builtin_templates))
        .route("/api/v1/templates/{code}", get(template))
}

type Rest<'a> = (
    tokio::sync::MutexGuard<'a, Core>,
    sqlx::pool::PoolConnection<sqlx::Sqlite>,
    Id,
);
async fn rest<'a>(app: &'a App, h: &HeaderMap) -> Result<Rest<'a>> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, http::bearer(h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    Ok((core, c, u))
}
type Params = Query<HashMap<String, String>>;

async fn community(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    Ok(http::json(state(&mut c, u, Id(g)).await?))
}
async fn applications(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    let g = Id(g);
    guild_perm(&mut c, u, g, Permissions::KICK_MEMBERS).await?;
    Ok(http::json(admission::queue(&mut c, g).await?))
}
async fn listing(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    Ok(http::json(directory::admin(&mut c, u, Id(g)).await?))
}
async fn guild_templates(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    Ok(http::json(templates::list(&mut c, u, Id(g)).await?))
}
/// The owner, and members with View Audit Log or Manage Server.
async fn insights_page(
    State(app): State<App>,
    Path(g): Path<u64>,
    Query(q): Params,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    let g = Id(g);
    role(&mut c, u, g).await?;
    require(
        acl::evaluate(&mut c, u, g, None)
            .await?
            .intersects(Permissions::VIEW_AUDIT_LOG | Permissions::MANAGE_GUILD),
        ErrorCode::Forbidden,
    )?;
    let days = q.get("days").and_then(|d| d.parse().ok()).unwrap_or(30);
    Ok(http::json(insights::report(&mut c, g, days).await?))
}
async fn my_applications(State(app): State<App>, h: HeaderMap) -> Result<http::ApiResponse> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    Ok(http::json(admission::my_applications(&mut c, u).await?))
}
/// What an invite's server asks of new members. The code is the secret.
async fn invite_gate(
    State(app): State<App>,
    Path(code): Path<String>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    let g = admission::invite_guild(&mut c, &code).await?;
    Ok(http::json(admission::join_gate(&mut c, u, g).await?))
}
async fn directory_page(
    State(app): State<App>,
    Query(q): Params,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut core, mut c, u) = rest(&app, &h).await?;
    let text = q.get("q").map(String::as_str).unwrap_or_default();
    require(text.chars().count() <= 100, ErrorCode::InvalidRequest)?;
    let category = q.get("category").and_then(|v| DirectoryCategory::parse(v));
    Ok(http::json(
        directory::page(&mut core, &mut c, u, text, category).await?,
    ))
}
async fn directory_preview(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut core, mut c, u) = rest(&app, &h).await?;
    Ok(http::json(
        directory::preview(&mut core, &mut c, u, Id(g)).await?,
    ))
}
async fn builtin_templates(State(app): State<App>, h: HeaderMap) -> Result<http::ApiResponse> {
    let (_core, _c, _u) = rest(&app, &h).await?;
    Ok(http::json(
        templates::BUILTIN
            .iter()
            .filter_map(|(k, _, _)| templates::builtin(k))
            .collect::<Vec<_>>(),
    ))
}
/// A template's preview. Holding the link is what lets you see it.
async fn template(
    State(app): State<App>,
    Path(code): Path<String>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (_core, mut c, _u) = rest(&app, &h).await?;
    let mut t = templates::by_code(&mut c, &code).await?;
    // Who made it and from where is not part of the shared preview.
    t.source_guild_id = None;
    t.creator_id = None;
    Ok(http::json(t))
}
