//! Identity REST routes (cookie or bearer). Reads only, except the role icon
//! upload; every mutation is an `IdentityCommand`.
use super::*;
use crate::{http, state::App};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/guilds/{g}/identity", get(admin))
        .route("/api/v1/users/@me/tags", get(my_tags))
        .route("/api/v1/tags/{g}", get(tag_card))
        .route("/api/v1/invites/{code}/card", get(invite_card))
        .route("/api/v1/invites/{code}/art/{slot}", get(vanity::art))
        .route("/api/v1/directory/{g}/art", get(directory_art))
        .route(
            "/api/v1/guilds/{g}/roles/{r}/icon",
            get(looks::icon_endpoint).post(looks::upload_icon),
        )
        .route("/invite/{code}", get(vanity::redirect))
}
async fn rest<'a>(
    app: &'a App,
    h: &HeaderMap,
) -> Result<(
    tokio::sync::MutexGuard<'a, Core>,
    sqlx::pool::PoolConnection<sqlx::Sqlite>,
    Id,
)> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, http::bearer(h)?).await?;
    core.rate(format!("identity-rest:{}", u.0), 120., 30.)?;
    Ok((core, c, u))
}

async fn admin(
    State(app): State<App>,
    Path(g): Path<String>,
    h: HeaderMap,
) -> Result<Json<IdentityAdmin>> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    let g = http::url_id(&g)?;
    guild_perm(&mut c, u, g, Permissions::MANAGE_GUILD).await?;
    let wearers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE tag_guild=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    let vanity = vanity::info(&mut c, g).await?;
    let invites: Vec<String> = sqlx::query_scalar("SELECT code FROM invites WHERE guild_id=? AND expires_at>? AND uses<max_uses AND id NOT IN(SELECT invite_id FROM vanity_codes WHERE invite_id IS NOT NULL) ORDER BY id DESC LIMIT 50")
        .bind(id(g)?)
        .bind(now() as i64)
        .fetch_all(&mut *c)
        .await?;
    let wait = vanity::wait_ms(&mut c, g).await?;
    Ok(Json(IdentityAdmin {
        guild_id: g,
        tag: tags::load(&mut c, g).await?,
        wearers: wearers as u32,
        look: guild_look(&mut c, g).await?,
        invite_background: looks::invite_background(&mut c, g).await?,
        vanity,
        vanity_wait_ms: wait,
        invites,
    }))
}
async fn my_tags(State(app): State<App>, h: HeaderMap) -> Result<Json<MyTags>> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    Ok(Json(tags::mine(&mut c, u).await?))
}
async fn tag_card(
    State(app): State<App>,
    Path(g): Path<String>,
    h: HeaderMap,
) -> Result<Json<TagCard>> {
    let (_core, mut c, u) = rest(&app, &h).await?;
    Ok(Json(tags::card(&mut c, u, http::url_id(&g)?).await?))
}
async fn invite_card(
    State(app): State<App>,
    Path(code): Path<String>,
    h: HeaderMap,
) -> Result<Json<InviteCard>> {
    let (core, mut c, u) = rest(&app, &h).await?;
    Ok(Json(vanity::card(&mut c, &core, u, &code).await?))
}
async fn directory_art(
    State(app): State<App>,
    Path(g): Path<String>,
    h: HeaderMap,
) -> Result<Json<DirectoryArt>> {
    let (_core, mut c, _u) = rest(&app, &h).await?;
    let g = http::url_id(&g)?;
    require(
        crate::community::listed_icon(&mut c, g).await?,
        ErrorCode::NotFound,
    )?;
    Ok(Json(DirectoryArt {
        banner: guild_look(&mut c, g).await?.banner,
        tag: tags::load(&mut c, g).await?.map(|t| t.worn(g)),
    }))
}
