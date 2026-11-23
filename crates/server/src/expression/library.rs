//! REST reads for expression, the sticker upload, and per-account settings.
//! Mutations other than uploads are WebSocket commands.
use super::*;
use crate::{http, state::App};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::get,
};

pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/users/@me/expression", get(library))
        .route("/api/v1/guilds/{id}/expression", get(admin))
        .route(
            "/api/v1/guilds/{id}/stickers",
            get(guild_stickers).post(stickers::upload),
        )
        .route("/api/v1/stickers/{id}", get(stickers::asset))
        .route("/api/v1/stickers/{id}/still", get(stickers::still))
        .route("/api/v1/emoji/resolve", get(resolve))
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

pub(super) async fn settings(c: &mut SqliteConnection, u: Id) -> Result<ExpressionSettings> {
    let row = sqlx::query("SELECT typing,sticker_favorites FROM expression_settings WHERE user_id=?")
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(row.map_or_else(ExpressionSettings::default, |r| ExpressionSettings {
        typing: r
            .get::<Option<String>, _>("typing")
            .and_then(|t| serde_json::from_str(&t).ok()),
        sticker_favorites: serde_json::from_str(&r.get::<String, _>("sticker_favorites"))
            .unwrap_or_default(),
    }))
}
pub(super) async fn store(
    c: &mut SqliteConnection,
    u: Id,
    s: &ExpressionSettings,
    out: &mut Deliveries,
) -> Result<()> {
    let typing = s
        .typing
        .as_ref()
        .map(|t| serde_json::to_string(t).map_err(|_| Error::new(ErrorCode::Internal)))
        .transpose()?;
    let favorites =
        serde_json::to_string(&s.sticker_favorites).map_err(|_| Error::new(ErrorCode::Internal))?;
    sqlx::query("INSERT INTO expression_settings(user_id,typing,sticker_favorites) VALUES(?,?,?) ON CONFLICT(user_id) DO UPDATE SET typing=excluded.typing,sticker_favorites=excluded.sticker_favorites")
        .bind(id(u)?)
        .bind(typing)
        .bind(favorites)
        .execute(&mut *c)
        .await?;
    out.push((
        vec![u],
        Event::Expression(ExpressionEvent::SettingsUpdated(s.clone())),
    ));
    Ok(())
}

/// Everything the pickers need: each server's emoji and stickers, the
/// built-in pack, and this account's settings.
async fn library(State(app): State<App>, h: HeaderMap) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    let guilds: Vec<i64> =
        sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE user_id=? ORDER BY guild_id")
            .bind(id(u)?)
            .fetch_all(&mut *c)
            .await?;
    let mut out = ExpressionLibrary {
        settings: settings(&mut c, u).await?,
        ..Default::default()
    };
    out.stickers.push(GuildStickerSet {
        guild_id: None,
        stickers: builtin_stickers(),
    });
    for g in guilds.into_iter().map(|g| Id(g as u64)) {
        let emoji = crate::custom_emoji::emojis(&mut c, g).await?;
        if !emoji.is_empty() {
            out.emoji.push(GuildEmojiSet { guild_id: g, emoji });
        }
        let stickers = stickers::list(&mut c, g).await?;
        if !stickers.is_empty() {
            out.stickers.push(GuildStickerSet {
                guild_id: Some(g),
                stickers,
            });
        }
    }
    Ok(http::json(out))
}

/// The settings page: who added what, and the slots used (Manage Expressions).
async fn admin(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    let g = Id(g);
    manage(&mut c, u, g).await?;
    let emoji = sqlx::query("SELECT id,name,animated,creator_id,created_at FROM guild_emoji WHERE guild_id=? ORDER BY name,id")
        .bind(id(g)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| EmojiDetail {
            emoji: GuildEmoji {
                id: Id(r.get::<i64, _>("id") as u64),
                guild_id: g,
                name: r.get("name"),
                animated: r.get("animated"),
            },
            creator_id: r.get::<Option<i64>, _>("creator_id").map(|v| Id(v as u64)),
            created_at: r.get::<i64, _>("created_at") as u64,
        })
        .collect::<Vec<_>>();
    let stickers = stickers::list(&mut c, g).await?;
    let mut creators: Vec<Id> = emoji
        .iter()
        .filter_map(|e| e.creator_id)
        .chain(stickers.iter().filter_map(|s| s.creator_id))
        .collect();
    creators.sort();
    creators.dedup();
    let mut users = vec![];
    for v in creators {
        if let Ok(user) = user(&mut c, v).await {
            users.push(user);
        }
    }
    Ok(http::json(GuildExpressionAdmin {
        emoji,
        stickers,
        users,
        max_static_emoji: MAX_GUILD_STATIC_EMOJI as u32,
        max_animated_emoji: MAX_GUILD_ANIMATED_EMOJI as u32,
        max_stickers: MAX_GUILD_STICKERS as u32,
    }))
}

async fn guild_stickers(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    role(&mut c, u, Id(g)).await?;
    Ok(http::json(stickers::list(&mut c, Id(g)).await?))
}

#[derive(serde::Deserialize)]
struct Ids {
    ids: String,
}

/// What each emoji is, for the ones the asker may see (members of its server,
/// or readers of a conversation it was used in). Deleted ones come back with
/// `available: false` and their last name, so they read as `:name:`.
async fn resolve(
    State(app): State<App>,
    Query(q): Query<Ids>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    let mut ids = vec![];
    for part in q.ids.split(',').filter(|s| !s.is_empty()) {
        let v: u64 = part
            .parse()
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        id(Id(v))?;
        if !ids.contains(&Id(v)) {
            ids.push(Id(v));
        }
    }
    require(
        !ids.is_empty() && ids.len() <= MAX_RESOLVE_EMOJI,
        ErrorCode::InvalidRequest,
    )?;
    let mut out = vec![];
    for e in ids {
        let row = sqlx::query("SELECT e.guild_id,e.name,e.animated,g.name AS guild_name,g.icon_id FROM guild_emoji e JOIN guilds g ON g.id=e.guild_id WHERE e.id=?")
            .bind(id(e)?)
            .fetch_optional(&mut *c)
            .await?;
        if let Some(r) = row {
            let g = Id(r.get::<i64, _>("guild_id") as u64);
            if !may_see(&mut c, u, "emoji", e, g).await? {
                continue;
            }
            let member = role(&mut c, u, g).await.is_ok();
            out.push(EmojiInfo {
                id: e,
                name: r.get("name"),
                animated: r.get("animated"),
                guild_id: member.then_some(g),
                guild_name: member.then(|| r.get("guild_name")),
                guild_icon_id: if member {
                    r.get::<Option<i64>, _>("icon_id").map(|v| Id(v as u64))
                } else {
                    None
                },
                available: true,
            });
        } else if let Some(name) =
            sqlx::query_scalar::<_, String>("SELECT name FROM deleted_emoji WHERE id=?")
                .bind(id(e)?)
                .fetch_optional(&mut *c)
                .await?
        {
            out.push(EmojiInfo {
                id: e,
                name,
                animated: false,
                guild_id: None,
                guild_name: None,
                guild_icon_id: None,
                available: false,
            });
        }
    }
    Ok(http::json(out))
}
