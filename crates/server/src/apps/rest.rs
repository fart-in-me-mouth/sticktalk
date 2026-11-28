//! REST: reads for people (Developer section, consent screen, Integrations,
//! the `/` picker's catalog, stored embed images) and the bot API
//! (`Authorization: Bot stkb_…`, under `/api/v1/bot/`). Bot calls that change
//! anything run as commands (`commands::execute`), so they get the same
//! checks as the WebSocket.
use super::*;
use crate::http::{self, ApiResponse};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use tokio::io::AsyncWriteExt;

pub(super) fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/applications/@me", get(developer))
        .route("/api/v1/applications/{id}/install", get(install_preview))
        .route("/api/v1/guilds/{id}/integrations", get(integrations))
        .route("/api/v1/channels/{id}/commands", get(catalog))
        .route(
            "/api/v1/channels/{c}/messages/{m}/app-images/{slot}",
            get(image),
        )
        .route("/api/v1/bot/@me", get(bot_me))
        .route("/api/v1/bot/guilds/{id}/channels", get(bot_channels))
        .route("/api/v1/bot/guilds/{id}/members", get(bot_members))
        .route(
            "/api/v1/bot/channels/{id}/messages",
            get(bot_history).post(bot_send),
        )
        .route(
            "/api/v1/bot/channels/{c}/messages/{m}",
            axum::routing::patch(bot_edit).delete(bot_delete),
        )
        .route(
            "/api/v1/bot/commands",
            get(bot_commands).put(bot_set_commands),
        )
        .route(
            "/api/v1/bot/guilds/{id}/commands",
            get(bot_guild_commands).put(bot_set_guild_commands),
        )
        .route(
            "/api/v1/bot/interactions/{id}/{token}/callback",
            post(bot_callback),
        )
        .route(
            "/api/v1/bot/interactions/{id}/{token}/original",
            axum::routing::patch(bot_edit_original).delete(bot_delete_original),
        )
        .route(
            "/api/v1/bot/interactions/{id}/{token}/followup",
            post(bot_follow_up),
        )
        .route("/api/v1/bot/uploads/{id}", put(bot_upload))
        .route("/api/v1/bot/attachments/{id}", get(bot_attachment))
}

type Person<'a> = (sqlx::pool::PoolConnection<sqlx::Sqlite>, Id);
async fn person<'a>(app: &'a App, h: &HeaderMap) -> Result<Person<'a>> {
    http::budgeted(app, http::bearer(h)?, &[("rest", 120., 30.)]).await
}

async fn developer(State(app): State<App>, h: HeaderMap) -> Result<ApiResponse> {
    let (mut c, u) = person(&app, &h).await?;
    Ok(http::json(store::developer(&mut c, u).await?))
}
async fn install_preview(
    State(app): State<App>,
    Path(a): Path<u64>,
    h: HeaderMap,
) -> Result<ApiResponse> {
    let (mut c, u) = person(&app, &h).await?;
    Ok(http::json(install::preview(&mut c, u, Id(a)).await?))
}
async fn integrations(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<ApiResponse> {
    let (mut c, u) = person(&app, &h).await?;
    Ok(http::json(install::integrations(&mut c, u, Id(g)).await?))
}
async fn catalog(State(app): State<App>, Path(ch): Path<u64>, h: HeaderMap) -> Result<ApiResponse> {
    let (mut c, u) = person(&app, &h).await?;
    Ok(http::json(registry::catalog(&mut c, u, Id(ch)).await?))
}
async fn image(
    State(app): State<App>,
    Path((ch, m, slot)): Path<(u64, u64, i64)>,
    h: HeaderMap,
) -> Result<Response> {
    let (mut c, u) = http::budgeted(&app, http::bearer(&h)?, &[("file", 600., 120.)]).await?;
    let (kind, bytes) = messages::image(&mut c, u, Id(ch), Id(m), slot).await?;
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

// ------------------------------------------------------------------ bots

struct Bot {
    key: Vec<u8>,
    user: Id,
    app: Id,
}

/// `Authorization: Bot stkb_…` (or `Bearer stkb_…`).
async fn bot(app: &App, h: &HeaderMap) -> Result<Bot> {
    let value = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| Error::new(ErrorCode::Unauthenticated))?;
    let token = value
        .strip_prefix("Bot ")
        .or_else(|| value.strip_prefix("Bearer "))
        .and_then(|t| t.strip_prefix(BOT_TOKEN_PREFIX))
        .ok_or_else(|| Error::new(ErrorCode::Unauthenticated))?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, user, a) = store::bot_auth(&mut c, token).await?;
    core.rate(format!("bot-rest:{}", a.0), 300., 50.)?;
    core.cursor(&key, user);
    register(&mut core, &mut c, &key, a, user).await?;
    Ok(Bot { key, user, app: a })
}

async fn run(app: &App, b: &Bot, command: Command) -> Result<CommandResult> {
    let mut core = app.0.core.lock().await;
    let nonce = u64::from_be_bytes(crate::state::random(8).try_into().unwrap_or([1; 8]))
        & (i64::MAX as u64);
    let response = crate::commands::execute(
        app,
        &mut core,
        &b.key,
        b.user,
        CommandRequest {
            nonce: nonce.max(1),
            command,
        },
    )
    .await?;
    match response.outcome {
        CommandOutcome::Ok(r) => Ok(r),
        CommandOutcome::Err(e) => Err(Error(e)),
    }
}
fn reply(r: CommandResult) -> Response {
    match r {
        CommandResult::Apps(AppResult::Message(m)) => http::json(*m).into_response(),
        CommandResult::Apps(AppResult::Ephemeral(m)) => http::json(*m).into_response(),
        CommandResult::Apps(AppResult::Commands(v)) => http::json(v).into_response(),
        _ => StatusCode::NO_CONTENT.into_response(),
    }
}
fn body<T: serde::de::DeserializeOwned>(
    j: std::result::Result<Json<T>, axum::extract::rejection::JsonRejection>,
) -> Result<T> {
    j.map(|Json(v)| v).map_err(|e| {
        let mut err = Error::new(ErrorCode::InvalidRequest);
        err.0.message = e.body_text();
        err
    })
}

#[derive(serde::Serialize)]
struct BotMe {
    user: UserSummary,
    app: AppInfo,
    guilds: Vec<GuildSummary>,
}
async fn bot_me(State(app): State<App>, h: HeaderMap) -> Result<ApiResponse> {
    let b = bot(&app, &h).await?;
    let mut c = app.0.pool.acquire().await?;
    let mut guilds = vec![];
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE user_id=? ORDER BY guild_id")
            .bind(id(b.user)?)
            .fetch_all(&mut *c)
            .await?;
    for g in ids {
        guilds.push(guild(&mut c, b.user, Id(g as u64)).await?);
    }
    let mut info = store::info(&mut c, b.app).await?;
    info.owner_id = Id(0);
    Ok(http::json(BotMe {
        user: user(&mut c, b.user).await?,
        app: info,
        guilds,
    }))
}

async fn bot_channels(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<ApiResponse> {
    let b = bot(&app, &h).await?;
    let mut c = app.0.pool.acquire().await?;
    Ok(http::json(channels(&mut c, b.user, Some(Id(g))).await?))
}

#[derive(serde::Deserialize)]
struct Page {
    #[serde(default)]
    after: Option<u64>,
    #[serde(default)]
    before: Option<u64>,
    #[serde(default)]
    limit: Option<u16>,
}

async fn bot_members(
    State(app): State<App>,
    Path(g): Path<u64>,
    Query(q): Query<Page>,
    h: HeaderMap,
) -> Result<ApiResponse> {
    let b = bot(&app, &h).await?;
    let mut c = app.0.pool.acquire().await?;
    let g = Id(g);
    role(&mut c, b.user, g).await?;
    let intents: i64 = sqlx::query_scalar("SELECT intents FROM apps WHERE id=?")
        .bind(id(b.app)?)
        .fetch_one(&mut *c)
        .await?;
    if !Intents::from_bits_truncate(intents as u32).contains(Intents::GUILD_MEMBERS) {
        return Err(denied(
            "intents",
            "Listing members needs the Server Members intent.",
        ));
    }
    let limit = i64::from(q.limit.unwrap_or(100).clamp(1, 1000));
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT user_id FROM guild_members WHERE guild_id=? AND user_id>? ORDER BY user_id LIMIT ?",
    )
    .bind(id(g)?)
    .bind(q.after.unwrap_or(0) as i64)
    .bind(limit)
    .fetch_all(&mut *c)
    .await?;
    let mut members = vec![];
    for u in ids {
        members.push(member(&mut c, g, Id(u as u64)).await?);
    }
    Ok(http::json(members))
}

/// History, newest first, with message content withheld where the bot was
/// not granted it (the same rule as its WebSocket events).
async fn bot_history(
    State(app): State<App>,
    Path(ch): Path<u64>,
    Query(q): Query<Page>,
    h: HeaderMap,
) -> Result<ApiResponse> {
    let b = bot(&app, &h).await?;
    let ch = Id(ch);
    let mut c = app.0.pool.acquire().await?;
    let channel = text(&mut c, b.user, ch, Permissions::READ_HISTORY).await?;
    let limit = i64::from(q.limit.unwrap_or(50).clamp(1, 100));
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM messages WHERE channel_id=? AND deleted_at IS NULL AND id<? AND id>? ORDER BY id DESC LIMIT ?")
        .bind(id(ch)?)
        .bind(q.before.map_or(i64::MAX, |v| v as i64))
        .bind(q.after.unwrap_or(0) as i64)
        .bind(limit)
        .fetch_all(&mut *c)
        .await?;
    let granted = match channel.summary.guild_id {
        Some(g) => {
            let r = sqlx::query("SELECT i.message_content,a.intents FROM app_installs i JOIN apps a ON a.id=i.app_id WHERE i.app_id=? AND i.guild_id=?")
                .bind(id(b.app)?)
                .bind(id(g)?)
                .fetch_optional(&mut *c)
                .await?;
            r.is_some_and(|r| {
                r.get::<bool, _>("message_content")
                    && Intents::from_bits_truncate(r.get::<i64, _>("intents") as u32)
                        .contains(Intents::MESSAGE_CONTENT)
            })
        }
        None => false,
    };
    let mut out = vec![];
    for m in ids {
        let m = message(&mut c, b.user, ch, Id(m as u64), false).await?;
        out.push(
            if granted || m.author_id == b.user || m.mention_ids.contains(&b.user) {
                m
            } else {
                withheld(&m)
            },
        );
    }
    Ok(http::json(out))
}

async fn bot_send(
    State(app): State<App>,
    Path(ch): Path<u64>,
    h: HeaderMap,
    j: std::result::Result<Json<AppMessageDraft>, axum::extract::rejection::JsonRejection>,
) -> Result<Response> {
    let b = bot(&app, &h).await?;
    let draft = body(j)?;
    let r = run(
        &app,
        &b,
        Command::Apps(AppCommand::Send {
            channel_id: Id(ch),
            draft,
        }),
    )
    .await?;
    Ok(reply(r))
}
async fn bot_edit(
    State(app): State<App>,
    Path((ch, m)): Path<(u64, u64)>,
    h: HeaderMap,
    j: std::result::Result<Json<AppMessageDraft>, axum::extract::rejection::JsonRejection>,
) -> Result<Response> {
    let b = bot(&app, &h).await?;
    let draft = body(j)?;
    let r = run(
        &app,
        &b,
        Command::Apps(AppCommand::Edit {
            channel_id: Id(ch),
            message_id: Id(m),
            draft,
        }),
    )
    .await?;
    Ok(reply(r))
}
async fn bot_delete(
    State(app): State<App>,
    Path((ch, m)): Path<(u64, u64)>,
    h: HeaderMap,
) -> Result<Response> {
    let b = bot(&app, &h).await?;
    let r = run(
        &app,
        &b,
        Command::DeleteMessage {
            channel_id: Id(ch),
            message_id: Id(m),
        },
    )
    .await?;
    Ok(reply(r))
}

async fn bot_commands(State(app): State<App>, h: HeaderMap) -> Result<ApiResponse> {
    let b = bot(&app, &h).await?;
    let mut c = app.0.pool.acquire().await?;
    Ok(http::json(
        registry::commands(&mut c, b.app, None)
            .await?
            .into_iter()
            .filter(|c| c.guild_id.is_none())
            .collect::<Vec<_>>(),
    ))
}
async fn bot_guild_commands(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<ApiResponse> {
    let b = bot(&app, &h).await?;
    let mut c = app.0.pool.acquire().await?;
    Ok(http::json(
        registry::commands(&mut c, b.app, Some(Id(g)))
            .await?
            .into_iter()
            .filter(|c| c.guild_id == Some(Id(g)))
            .collect::<Vec<_>>(),
    ))
}
async fn bot_set_commands(
    State(app): State<App>,
    h: HeaderMap,
    j: std::result::Result<Json<Vec<CommandDraft>>, axum::extract::rejection::JsonRejection>,
) -> Result<Response> {
    let b = bot(&app, &h).await?;
    let commands = body(j)?;
    let r = run(
        &app,
        &b,
        Command::Apps(AppCommand::SetCommands {
            guild_id: None,
            commands,
        }),
    )
    .await?;
    Ok(reply(r))
}
async fn bot_set_guild_commands(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
    j: std::result::Result<Json<Vec<CommandDraft>>, axum::extract::rejection::JsonRejection>,
) -> Result<Response> {
    let b = bot(&app, &h).await?;
    let commands = body(j)?;
    let r = run(
        &app,
        &b,
        Command::Apps(AppCommand::SetCommands {
            guild_id: Some(Id(g)),
            commands,
        }),
    )
    .await?;
    Ok(reply(r))
}

fn answer(i: u64, token: String) -> InteractionAnswer {
    InteractionAnswer {
        interaction_id: Id(i),
        token,
    }
}
async fn bot_callback(
    State(app): State<App>,
    Path((i, token)): Path<(u64, String)>,
    h: HeaderMap,
    j: std::result::Result<Json<InteractionResponse>, axum::extract::rejection::JsonRejection>,
) -> Result<Response> {
    let b = bot(&app, &h).await?;
    let response = body(j)?;
    let r = run(
        &app,
        &b,
        Command::Apps(AppCommand::Respond {
            answer: answer(i, token),
            response,
        }),
    )
    .await?;
    Ok(reply(r))
}
async fn bot_edit_original(
    State(app): State<App>,
    Path((i, token)): Path<(u64, String)>,
    h: HeaderMap,
    j: std::result::Result<Json<AppMessageDraft>, axum::extract::rejection::JsonRejection>,
) -> Result<Response> {
    let b = bot(&app, &h).await?;
    let draft = body(j)?;
    let r = run(
        &app,
        &b,
        Command::Apps(AppCommand::EditOriginal {
            answer: answer(i, token),
            draft,
        }),
    )
    .await?;
    Ok(reply(r))
}
async fn bot_delete_original(
    State(app): State<App>,
    Path((i, token)): Path<(u64, String)>,
    h: HeaderMap,
) -> Result<Response> {
    let b = bot(&app, &h).await?;
    let r = run(
        &app,
        &b,
        Command::Apps(AppCommand::DeleteOriginal {
            answer: answer(i, token),
        }),
    )
    .await?;
    Ok(reply(r))
}
async fn bot_follow_up(
    State(app): State<App>,
    Path((i, token)): Path<(u64, String)>,
    h: HeaderMap,
    j: std::result::Result<Json<AppMessageDraft>, axum::extract::rejection::JsonRejection>,
) -> Result<Response> {
    let b = bot(&app, &h).await?;
    let draft = body(j)?;
    let r = run(
        &app,
        &b,
        Command::Apps(AppCommand::FollowUp {
            answer: answer(i, token),
            draft,
        }),
    )
    .await?;
    Ok(reply(r))
}

/// `PUT /api/v1/bot/uploads/{channel}` (multipart, one `file`): a file the
/// bot can attach to its next message there, like a person's upload.
async fn bot_upload(
    State(app): State<App>,
    Path(ch): Path<u64>,
    h: HeaderMap,
    r: Request<Body>,
) -> Result<Response> {
    use axum::extract::FromRequest;
    let b = bot(&app, &h).await?;
    let mut form = axum::extract::Multipart::from_request(r, &app)
        .await
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let field = form
        .next_field()
        .await
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    require(field.name() == Some("file"), ErrorCode::InvalidRequest)?;
    let a = store_upload(&app, b.user, Id(ch), field).await?;
    let mut c = app.0.pool.acquire().await?;
    let row = sqlx::query("SELECT * FROM uploads WHERE id=?")
        .bind(id(a)?)
        .fetch_one(&mut *c)
        .await?;
    Ok((StatusCode::CREATED, http::json(attachment_row(&row))).into_response())
}

/// A file a person passed to this bot in an interaction (an attachment
/// option), and nothing else: the grant is made when the interaction reaches
/// the bot, and the bot must be able to read that channel's messages now, as
/// a member reading the message would. Anything else is "not found".
async fn bot_attachment(
    State(app): State<App>,
    Path(a): Path<u64>,
    h: HeaderMap,
) -> Result<Response> {
    let b = bot(&app, &h).await?;
    let a = Id(a);
    let row = {
        let mut core = app.0.core.lock().await;
        core.rate(format!("file:{}", b.user.0), 600., 120.)?;
        let ch = core
            .apps
            .files
            .channel(b.user, a)
            .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
        let mut c = app.0.pool.acquire().await?;
        text(&mut c, b.user, ch, Permissions::READ_HISTORY)
            .await
            .map_err(|_| Error::new(ErrorCode::NotFound))?;
        sqlx::query("SELECT * FROM uploads WHERE id=? AND (expires_at IS NULL OR expires_at>?)")
            .bind(id(a)?)
            .bind(now() as i64)
            .fetch_optional(&mut *c)
            .await?
            .ok_or_else(|| Error::new(ErrorCode::NotFound))?
    };
    crate::files::serve(&app, a, &row, &h, false).await
}

/// Stores one multipart file for `owner` in `ch`, with the same checks,
/// quotas and accounting as a person's upload (`files::upload`).
pub(crate) async fn store_upload(
    app: &App,
    owner: Id,
    ch: Id,
    mut field: axum::extract::multipart::Field<'_>,
) -> Result<Id> {
    let _slot = app
        .0
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    {
        let mut c = app.0.pool.acquire().await?;
        crate::threads::upload_target(&mut c, owner, ch, Permissions::ATTACH_FILES).await?;
    }
    let _disk = app.0.upload_io.lock().await;
    let max = app.0.config.max_upload_mb.saturating_mul(1024 * 1024);
    crate::resumable::disk_guard(app, 65536).await?;
    let day = (now() / 86_400_000) as i64;
    let reservation = crate::state::stream_id();
    {
        let mut core = app.0.core.lock().await;
        core.rate(
            format!("upload:{}", owner.0),
            20.,
            2. * MAX_MESSAGE_ATTACHMENTS as f64,
        )?;
        let mut c = app.0.pool.acquire().await?;
        crate::upload_accounting::admit(&mut c, owner).await?;
        crate::upload_accounting::quota(app, &mut c, owner, 65536).await?;
        sqlx::query("INSERT INTO upload_reservations(key,user_id,utc_day,reserved_bytes,expires_at,channel_id) VALUES(?,?,?,0,?,?)")
            .bind(&reservation)
            .bind(id(owner)?)
            .bind(day)
            .bind((now() + crate::upload_accounting::IDLE_MS) as i64)
            .bind(id(ch)?)
            .execute(&mut *c)
            .await?;
    }
    let dir = app.0.config.data_dir.join("files");
    let staging = dir.join(format!("{reservation}.tmp"));
    let final_key = format!("{reservation}.bin");
    let thumb_key = format!("{reservation}.jpg");
    let result = async {
        let name = crate::files::filename(field.file_name().unwrap_or("file"));
        let claimed = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_owned();
        let mut file = tokio::fs::File::create(&staging).await?;
        let mut size = 0u64;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?
        {
            size += chunk.len() as u64;
            crate::upload_accounting::charge(app, owner, chunk.len()).await?;
            require(max == 0 || size <= max, ErrorCode::PayloadTooLarge)?;
            crate::resumable::disk_guard(app, chunk.len() as u64).await?;
            {
                let _gate = app.0.core.lock().await;
                let mut c = app.0.pool.acquire().await?;
                crate::upload_accounting::quota(app, &mut c, owner, chunk.len() as u64).await?;
                sqlx::query("UPDATE upload_reservations SET reserved_bytes=?,expires_at=? WHERE key=?")
                    .bind(size as i64)
                    .bind((now() + crate::upload_accounting::IDLE_MS) as i64)
                    .bind(&reservation)
                    .execute(&mut *c)
                    .await?;
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        drop(file);
        let permit = app
            .0
            .images
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        let path = staging.clone();
        let media = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            crate::profiles::media::prepare(&path, &claimed, UploadPurpose::Message)
        })
        .await
        .map_err(|_| Error::new(ErrorCode::Internal))??;
        if !media.thumbnail_bytes.is_empty() {
            tokio::fs::write(dir.join(&thumb_key), &media.thumbnail_bytes).await?;
        }
        tokio::fs::rename(&staging, dir.join(&final_key)).await?;
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        crate::threads::upload_target(&mut c, owner, ch, Permissions::ATTACH_FILES).await?;
        let a = core.id().await?;
        sqlx::query("INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,width,height,thumbnail_key,thumbnail_width,thumbnail_height,thumbnail_size,thumbnail_type,created_at,expires_at) VALUES(?,?,'message',?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(id(a)?).bind(id(owner)?).bind(id(ch)?).bind(name).bind(media.mime).bind(size as i64).bind(&final_key)
            .bind(media.width.map(i64::from)).bind(media.height.map(i64::from)).bind(media.thumb.map(|_| &thumb_key))
            .bind(media.thumb.map(|t| i64::from(t.0))).bind(media.thumb.map(|t| i64::from(t.1))).bind(media.thumb.map(|t| i64::from(t.2)))
            .bind(media.thumb.map(|_| "image/jpeg")).bind(now() as i64).bind((now() + 3_600_000) as i64)
            .execute(&mut *c)
            .await?;
        sqlx::query("DELETE FROM upload_reservations WHERE key=?")
            .bind(&reservation)
            .execute(&mut *c)
            .await?;
        core.persist_id(&mut c).await?;
        drop(c);
        drop(core);
        crate::tier::published(app, &final_key, media.thumb.map(|_| thumb_key.as_str())).await;
        Ok::<Id, Error>(a)
    }
    .await;
    if result.is_err() {
        let _gate = app.0.core.lock().await;
        if let Ok(mut c) = app.0.pool.acquire().await {
            let _ = sqlx::query("DELETE FROM upload_reservations WHERE key=?")
                .bind(&reservation)
                .execute(&mut *c)
                .await;
        }
        drop(_gate);
        for p in [staging, dir.join(&final_key), dir.join(&thumb_key)] {
            let _ = tokio::fs::remove_file(p).await;
        }
    }
    result
}
