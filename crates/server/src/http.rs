use crate::{
    db::*,
    error::{require, Error, Result},
    state::{self, App},
};
use argon2::{
    password_hash::SaltString, Algorithm, Argon2, Params, PasswordHash, PasswordHasher,
    PasswordVerifier, Version,
};
use axum::{
    body::{to_bytes, Body},
    extract::{ConnectInfo, DefaultBodyLimit, Query, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sqlx::{Connection, Row};
use std::net::SocketAddr;
use sticktalk_proto::*;
pub fn router(app: App) -> axum::Router {
    axum::Router::new()
        .merge(crate::browser::routes())
        .merge(crate::custom_emoji::routes())
        .route(
            "/api/v1/users/@me/server-folders",
            get(crate::folders::endpoint),
        )
        .route(
            "/api/v1/channels/{id}/slowmode",
            get(crate::slowmode::endpoint),
        )
        .route(
            "/api/v1/guilds/{id}/messages/search",
            get(crate::search::endpoint),
        )
        .merge(crate::push::routes())
        .route(
            "/api/v1/channels/{id}/history-extent",
            get(crate::history::extent),
        )
        .route("/api/v1/ws", get(crate::ws::upgrade))
        .route("/api/v1/uploads/resumable", post(crate::resumable::create))
        .route(
            "/api/v1/uploads/resumable/{id}",
            get(crate::resumable::status)
                .put(crate::resumable::chunk)
                .delete(crate::resumable::cancel),
        )
        .route(
            "/api/v1/uploads/resumable/{id}/finish",
            post(crate::resumable::finish),
        )
        .route("/api/v1/guilds/{id}/storage", get(crate::resumable::usage))
        .route("/api/update/latest", get(crate::updates::latest))
        .route("/api/v1/update/latest", get(crate::updates::latest))
        .route("/api/update/artifact", get(crate::updates::artifact))
        .route("/api/update/publish", post(crate::updates::publish))
        .route("/api/v1/auth/reset-password", post(crate::admin::reset))
        .route("/api/v1/uploads", post(crate::files::upload))
        .route("/api/v1/files/{id}", get(crate::files::original))
        .route("/api/v1/files/{id}/thumbnail", get(crate::files::thumbnail))
        .route("/api/v1/{*path}", any(api))
        .route("/api/{*path}", any(api))
        .fallback(crate::browser::static_file)
        .layer(axum::middleware::from_fn(json_framework_errors))
        .layer(DefaultBodyLimit::disable())
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            bounded_requests,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            crate::browser::boundary,
        ))
        .with_state(app)
}
async fn bounded_requests(
    State(app): State<App>,
    r: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let path = r.uri().path();
    let transfer = path.contains("/uploads")
        || path.contains("/files/")
        || path.contains("/update/artifact")
        || path.contains("/update/publish")
        || !path.starts_with("/api/");
    let pool = if transfer {
        &app.0.transfers
    } else {
        &app.0.requests
    };
    let Ok(permit) = pool.clone().try_acquire_owned() else {
        return Error::new(ErrorCode::Unavailable).into_response();
    };
    let mut permits = vec![permit];
    if transfer
        && matches!(
            *r.method(),
            axum::http::Method::GET | axum::http::Method::HEAD
        )
    {
        let ip = auth_bucket(&app, &r);
        let mut keys = vec![(format!("download:{ip}"), 8)];
        if let Ok(token) = bearer(r.headers()) {
            match app.0.pool.acquire().await {
                Ok(mut c) => match auth(&mut c, token).await {
                    Ok((_, user)) => keys.push((format!("download-user:{}", user.0), 3)),
                    Err(e) => return e.into_response(),
                },
                Err(_) => return Error::new(ErrorCode::Unavailable).into_response(),
            }
        }
        for (key, limit) in keys {
            match download_permit(&app, key, limit) {
                Ok(p) => permits.push(p),
                Err(e) => return e.into_response(),
            }
        }
    }
    let finishing = path.starts_with("/api/v1/uploads/resumable/") && path.ends_with("/finish");
    let seconds = if finishing {
        900
    } else if transfer {
        120
    } else {
        15
    };
    let response = tokio::select! {
        _ = app.0.shutdown.cancelled() => return Error::new(ErrorCode::Unavailable).into_response(),
        response = tokio::time::timeout(std::time::Duration::from_secs(seconds), next.run(r)) => response,
    };
    let Ok(response) = response else {
        return Error::new(ErrorCode::Unavailable).into_response();
    };
    timed_response(
        response,
        permits,
        app.0.shutdown.clone(),
        std::time::Duration::from_secs(30),
        std::time::Duration::from_secs(if transfer { 900 } else { 30 }),
    )
}
fn download_permit(
    app: &App,
    key: String,
    limit: usize,
) -> Result<tokio::sync::OwnedSemaphorePermit> {
    let mut limits = app
        .0
        .download_limits
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    limits.retain(|_, weak| weak.strong_count() > 0);
    require(
        limits.len() < 4096 || limits.contains_key(&key),
        ErrorCode::Unavailable,
    )?;
    let pool = limits
        .get(&key)
        .and_then(std::sync::Weak::upgrade)
        .unwrap_or_else(|| {
            let pool = std::sync::Arc::new(tokio::sync::Semaphore::new(limit));
            limits.insert(key, std::sync::Arc::downgrade(&pool));
            pool
        });
    pool.try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::LimitExceeded))
}
/// Pump with a single buffered chunk. The timer owns the admission permits, so
/// they expire even if Hyper never polls the body again (a stalled TCP writer).
fn timed_response(
    response: Response,
    permits: Vec<tokio::sync::OwnedSemaphorePermit>,
    stop: tokio_util::sync::CancellationToken,
    idle: std::time::Duration,
    lifetime: std::time::Duration,
) -> Response {
    use futures_util::StreamExt;
    let (parts, body) = response.into_parts();
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        let _permits = permits;
        let pump = async {
            let mut stream = body.into_data_stream();
            while let Ok(Some(chunk)) = tokio::time::timeout(idle, stream.next()).await {
                if !matches!(tokio::time::timeout(idle, tx.send(chunk)).await, Ok(Ok(()))) {
                    break;
                }
            }
        };
        tokio::select! { _ = stop.cancelled() => {}, _ = tokio::time::timeout(lifetime, pump) => {} }
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|chunk| (chunk, rx))
    });
    Response::from_parts(parts, Body::from_stream(stream))
}

async fn json_framework_errors(r: Request<Body>, next: axum::middleware::Next) -> Response {
    let response = next.run(r).await;
    if (response.status().is_client_error() || response.status().is_server_error())
        && !response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"))
    {
        return Error::new(match response.status() {
            StatusCode::NOT_FOUND => ErrorCode::NotFound,
            StatusCode::PAYLOAD_TOO_LARGE => ErrorCode::PayloadTooLarge,
            _ => ErrorCode::InvalidRequest,
        })
        .into_response();
    }
    response
}
pub fn bearer(r: &axum::http::HeaderMap) -> Result<&str> {
    r.get("authorization")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| Error::new(ErrorCode::Unauthenticated))
}
// Axum calls IntoResponse after the handler future returns. Keep owned snapshots
// here so the mutation gate and SQLite connection are dropped before encoding.
pub(crate) struct ApiResponse {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    encode: Box<dyn FnOnce() -> Response + Send>,
}
impl ApiResponse {
    pub(crate) fn from_response(response: Response) -> Self {
        Self {
            status: response.status(),
            headers: response.headers().clone(),
            encode: Box::new(|| response),
        }
    }
    fn status_mut(&mut self) -> &mut StatusCode {
        &mut self.status
    }
    pub(crate) fn headers_mut(&mut self) -> &mut axum::http::HeaderMap {
        &mut self.headers
    }
    pub(crate) fn empty() -> Self {
        Self {
            status: StatusCode::NO_CONTENT,
            headers: axum::http::HeaderMap::new(),
            encode: Box::new(|| Response::new(Body::empty())),
        }
    }
}
impl IntoResponse for ApiResponse {
    fn into_response(self) -> Response {
        let mut r = (self.encode)();
        *r.status_mut() = self.status;
        r.headers_mut().extend(self.headers);
        r
    }
}
pub(crate) fn json<T: Serialize + Send + 'static>(v: T) -> ApiResponse {
    ApiResponse {
        status: StatusCode::OK,
        headers: axum::http::HeaderMap::new(),
        encode: Box::new(move || axum::Json(v).into_response()),
    }
}
pub fn password_hash(password: String) -> Result<String> {
    let salt =
        SaltString::encode_b64(&state::random(16)).map_err(|_| Error::new(ErrorCode::Internal))?;
    argon()
        .hash_password(password.as_bytes(), &salt)
        .map(|v| v.to_string())
        .map_err(|_| Error::new(ErrorCode::Internal))
}
fn argon() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(19456, 2, 1, None).expect("fixed Argon2 parameters"),
    )
}
pub(crate) async fn parse<T: DeserializeOwned>(r: Request<Body>) -> Result<T> {
    require(
        r.headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| h.split(';').next() == Some("application/json")),
        ErrorCode::UnsupportedMediaType,
    )?;
    let bytes = to_bytes(r.into_body(), 16384)
        .await
        .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?;
    serde_json::from_slice(&bytes).map_err(|_| Error::new(ErrorCode::InvalidRequest))
}
pub(crate) async fn api(State(app): State<App>, r: Request<Body>) -> Result<ApiResponse> {
    let method = r.method().clone();
    let path = r
        .uri()
        .path()
        .strip_prefix("/api/v1/")
        .or_else(|| r.uri().path().strip_prefix("/api/"))
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_owned();
    if path == "gifs" || path.starts_with("gifs/") {
        return crate::gifs::api(&app, &path, r).await;
    }
    if matches!(path.as_str(), "auth/google/challenge" | "auth/reauth") {
        return crate::reauth::api(&app, &path, r).await;
    }
    if path == "auth/account/delete" {
        return crate::account::api(&app, r).await;
    }
    if crate::google::is_route(&path) {
        return crate::google::api(&app, &path, r).await;
    }
    if !r.uri().path().starts_with("/api/v1/") {
        return Err(Error::new(ErrorCode::NotFound));
    }
    if path == "health" && method == "GET" {
        return Ok(json(HealthResponse {
            version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: PROTOCOL_VERSION,
        }));
    }
    if (path == "auth/register" || path == "auth/login") && method == "POST" {
        let bucket = auth_bucket(&app, &r);
        app.0.core.lock().await.rate(bucket, 10., 5.)?;
        let register = path == "auth/register";
        let (s, password, display) = if register {
            let req: RegisterRequest = parse(r).await?;
            (req.username, req.password, optional(req.display_name, 64)?)
        } else {
            let req: LoginRequest = parse(r).await?;
            (req.username, req.password, None)
        };
        let s = username(&s).map_err(|e| {
            if register {
                e
            } else {
                Error::new(ErrorCode::InvalidCredentials)
            }
        })?;
        require(
            (12..=128).contains(&password.len()),
            if register {
                ErrorCode::InvalidRequest
            } else {
                ErrorCode::InvalidCredentials
            },
        )?;
        app.0
            .core
            .lock()
            .await
            .rate(format!("auth-name:{s}"), 5., 3.)?;
        let permit = app
            .0
            .passwords
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::new(ErrorCode::Unavailable))?;
        let mut login_identity = None;
        let phc = if register {
            tokio::task::spawn_blocking(move || {
                let _p = permit;
                password_hash(password)
            })
            .await
            .map_err(|_| Error::new(ErrorCode::Internal))??
        } else {
            let row = sqlx::query("SELECT id,password_hash FROM users WHERE username=?")
                .bind(&s)
                .fetch_optional(&app.0.pool)
                .await?;
            let exists = row
                .as_ref()
                .is_some_and(|r| !r.get::<String, _>("password_hash").is_empty());
            let phc: String = row
                .as_ref()
                .map(|r| r.get::<String, _>("password_hash"))
                .filter(|h| !h.is_empty())
                .unwrap_or_else(|| app.0.dummy_hash.clone());
            login_identity = row.map(|r| (Id(r.get::<i64, _>("id") as u64), phc.clone()));
            let valid = tokio::task::spawn_blocking(move || {
                let _p = permit;
                PasswordHash::new(&phc)
                    .is_ok_and(|h| argon().verify_password(password.as_bytes(), &h).is_ok())
            })
            .await
            .map_err(|_| Error::new(ErrorCode::Internal))?;
            require(valid && exists, ErrorCode::InvalidCredentials)?;
            String::new()
        };
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let mut tx = c.begin().await?;
        let u = if register {
            let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username=?")
                .bind(&s)
                .fetch_one(&mut *tx)
                .await?;
            require(
                n == 0 && !crate::account::reserved(&mut tx, &s).await?,
                ErrorCode::UsernameTaken,
            )?;
            let u = core.id().await?;
            sqlx::query("INSERT INTO users(id,username,password_hash,display_name,desired_presence,created_at) VALUES(?,?,?,?,'online',?)").bind(id(u)?).bind(&s).bind(phc).bind(display).bind(state::now() as i64).execute(&mut *tx).await?;
            u
        } else {
            // Hashing ran outside the gate. Bind verification to the same account,
            // not a possibly reassigned username.
            let (verified_user, verified_hash) =
                login_identity.ok_or_else(|| Error::new(ErrorCode::InvalidCredentials))?;
            verified_login(&mut tx, &s, verified_user, &verified_hash).await?
        };
        crate::db::enabled(&mut tx, u).await?;
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id=? AND expires_at>?")
                .bind(id(u)?)
                .bind(state::now() as i64)
                .fetch_one(&mut *tx)
                .await?;
        require(n < 5, ErrorCode::LimitExceeded)?;
        let token = state::token();
        let expires_at = state::now() + 30 * 86_400_000;
        sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
            .bind(state::hash(token.as_bytes()))
            .bind(id(u)?)
            .bind(state::now() as i64)
            .bind(expires_at as i64)
            .bind(state::now() as i64)
            .execute(&mut *tx)
            .await?;
        let user = me(&mut tx, u).await?;
        core.persist_id(&mut tx).await?;
        tx.commit().await?;
        let mut response = json(AuthResponse {
            token,
            expires_at,
            user,
        });
        *response.status_mut() = if register {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        };
        response
            .headers_mut()
            .insert("cache-control", "no-store".parse().expect("static"));
        return Ok(response);
    }
    let token = bearer(r.headers())?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, token).await?;
    if path == "auth/logout" && method == "POST" {
        crate::voice::disconnect(&app, &mut core, &mut c, &key, None).await?;
        // Close first: cancellation while SQLite finishes its autocommit must
        // not leave a revoked token's socket receiving live private events.
        if let Some(s) = core.streams.remove(&key) {
            if let Some(c) = s.connection {
                let _ = c.close.send(Some(4001));
            }
        }
        sqlx::query("DELETE FROM sessions WHERE token_hash=?")
            .bind(&key)
            .execute(&mut *c)
            .await?;
        crate::commands::broadcast_presence(&mut c, &mut core, u).await?;
        return Ok(ApiResponse::empty());
    }
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let cursor = core.cursor(&key, u);
    if method == "DELETE"
        && path
            .strip_prefix("uploads/")
            .is_some_and(|p| !p.contains('/'))
    {
        let a = url_id(path.split('/').nth(1).unwrap_or_default())?;
        let row = sqlx::query(
            "SELECT expires_at,storage_key,thumbnail_key FROM uploads WHERE id=? AND owner_id=?",
        )
        .bind(id(a)?)
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
        require(
            row.get::<Option<i64>, _>("expires_at").is_some(),
            ErrorCode::Conflict,
        )?;
        sqlx::query("DELETE FROM uploads WHERE id=?")
            .bind(id(a)?)
            .execute(&mut *c)
            .await?;
        drop(c);
        drop(core);
        let _ = tokio::fs::remove_file(
            app.0
                .config
                .data_dir
                .join("files")
                .join(row.get::<String, _>("storage_key")),
        )
        .await;
        if let Some(k) = row.get::<Option<String>, _>("thumbnail_key") {
            let _ = tokio::fs::remove_file(app.0.config.data_dir.join("files").join(k)).await;
        }
        app.0.tier.wake.notify_one();
        return Ok(ApiResponse::empty());
    }
    require(method == "GET", ErrorCode::InvalidRequest)?;
    let q = Query::<PageQuery>::try_from_uri(r.uri())
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?
        .0;
    validate_page(q)?;
    if q.around.is_some() || q.around_timestamp.is_some() {
        require(
            path.starts_with("channels/") && path.ends_with("/messages"),
            ErrorCode::InvalidRequest,
        )?;
    }
    let parts: Vec<_> = path.split('/').collect();
    if let Some(response) = crate::launch::snapshot(&mut c, &core, u, &key, &parts, &cursor).await?
    {
        return Ok(response);
    }
    match parts.as_slice() {
        ["me"] => Ok(json(Snapshot {
            data: me(&mut c, u).await?,
            cursor,
        })),
        ["users"] => {
            #[derive(Deserialize)]
            struct Users {
                ids: String,
            }
            let query = Query::<Users>::try_from_uri(r.uri())
                .map_err(|_| Error::new(ErrorCode::InvalidRequest))?
                .0;
            let mut ids = query
                .ids
                .split(',')
                .map(url_id)
                .collect::<Result<Vec<_>>>()?;
            unique(&ids, 50)?;
            require(!ids.is_empty(), ErrorCode::InvalidRequest)?;
            ids.sort();
            let mut users = vec![];
            for v in ids {
                profile_access(&mut c, u, v).await?;
                users.push(user(&mut c, v).await?);
            }
            Ok(json(Snapshot {
                data: users,
                cursor,
            }))
        }
        ["relationships"] => Ok(json(page(
            relationships(&mut c, u).await?,
            q,
            cursor,
            |v| v.user.id,
            false,
        ))),
        ["guilds"] => {
            let mut gs = vec![];
            for g in
                sqlx::query_scalar::<_, i64>("SELECT guild_id FROM guild_members WHERE user_id=?")
                    .bind(id(u)?)
                    .fetch_all(&mut *c)
                    .await?
            {
                let mut guild = guild(&mut c, u, Id(g as u64)).await?;
                if !core.streams.get(&key).is_some_and(|s| s.launch_features) {
                    crate::legacy::guild(&mut guild);
                }
                gs.push(guild);
            }
            Ok(json(page(gs, q, cursor, |v| v.guild.id, false)))
        }
        ["guilds", g] => Ok(json(Snapshot {
            data: {
                let mut detail = detail(&mut c, u, url_id(g)?).await?;
                if !core.streams.get(&key).is_some_and(|s| s.launch_features) {
                    crate::legacy::guild(&mut detail.summary);
                }
                detail
            },
            cursor,
        })),
        ["guilds", g, kind] => {
            let g = url_id(g)?;
            role(&mut c, u, g).await?;
            let effective = crate::permissions::evaluate(&mut c, u, g, None).await?;
            match *kind {
                "channels" => Ok(json(page(
                    channels(&mut c, u, Some(g)).await?,
                    q,
                    cursor,
                    |v| v.id,
                    false,
                ))),
                "members" => {
                    let mut members = vec![];
                    let ids=sqlx::query_scalar::<_,i64>("SELECT user_id FROM guild_members WHERE guild_id=? AND (? IS NULL OR user_id<?) AND (? IS NULL OR user_id>?) ORDER BY CASE WHEN ? THEN user_id END DESC,CASE WHEN NOT ? THEN user_id END ASC LIMIT ?")
                        .bind(id(g)?).bind(opt(q.before)?).bind(opt(q.before)?).bind(opt(q.after)?).bind(opt(q.after)?).bind(q.before.is_some()).bind(q.before.is_some()).bind(i64::from(q.limit.unwrap_or(50))+1).fetch_all(&mut *c).await?;
                    for v in ids {
                        let mut member = member(&mut c, g, Id(v as u64)).await?;
                        if !core.streams.get(&key).is_some_and(|s| s.launch_features) {
                            crate::legacy::member(&mut member);
                        }
                        members.push(member);
                    }
                    Ok(json(page(members, q, cursor, |v| v.user.id, false)))
                }
                "invites" => {
                    let mut items = vec![];
                    for iid in sqlx::query_scalar::<_, i64>(
                        "SELECT id FROM invites WHERE guild_id=? AND (creator_id=? OR ?) AND expires_at>? AND uses<max_uses AND (? IS NULL OR id<?) AND (? IS NULL OR id>?) ORDER BY CASE WHEN ? THEN id END DESC,CASE WHEN NOT ? THEN id END ASC LIMIT ?",
                    )
                    .bind(id(g)?)
                    .bind(id(u)?)
                    .bind(effective.contains(Permissions::MANAGE_GUILD))
                    .bind(state::now() as i64)
                    .bind(opt(q.before)?).bind(opt(q.before)?).bind(opt(q.after)?).bind(opt(q.after)?)
                    .bind(q.before.is_some()).bind(q.before.is_some()).bind(i64::from(q.limit.unwrap_or(50))+1)
                    .fetch_all(&mut *c)
                    .await?
                    {
                        items.push(invite(&mut c, Id(iid as u64)).await?);
                    }
                    Ok(json(page(items, q, cursor, |v| v.id, false)))
                }
                "bans" => {
                    guild_perm(&mut c, u, g, Permissions::BAN_MEMBERS).await?;
                    let mut items = vec![];
                    for v in sqlx::query_scalar::<_, i64>(
                        "SELECT user_id FROM guild_bans WHERE guild_id=? AND (? IS NULL OR user_id<?) AND (? IS NULL OR user_id>?) ORDER BY CASE WHEN ? THEN user_id END DESC,CASE WHEN NOT ? THEN user_id END ASC LIMIT ?",
                    )
                    .bind(id(g)?)
                    .bind(opt(q.before)?).bind(opt(q.before)?).bind(opt(q.after)?).bind(opt(q.after)?)
                    .bind(q.before.is_some()).bind(q.before.is_some()).bind(i64::from(q.limit.unwrap_or(50))+1)
                    .fetch_all(&mut *c)
                    .await?
                    {
                        items.push(ban(&mut c, g, Id(v as u64)).await?);
                    }
                    Ok(json(page(items, q, cursor, |v| v.user.id, false)))
                }
                _ => Err(Error::new(ErrorCode::NotFound)),
            }
        }
        ["dms"] => Ok(json(page(
            channels(&mut c, u, None)
                .await?
                .into_iter()
                .filter(|v| v.guild_id.is_none())
                .collect(),
            q,
            cursor,
            |v| v.id,
            false,
        ))),
        ["read-states"] => Ok(json(page(
            read_states(&mut c, u).await?,
            q,
            cursor,
            |v| v.channel_id,
            false,
        ))),
        ["channels", ch] => Ok(json(Snapshot {
            data: access(&mut c, u, url_id(ch)?, Permissions::VIEW_CHANNEL).await?,
            cursor,
        })),
        ["channels", ch, "messages"] => {
            let ch = url_id(ch)?;
            text(&mut c, u, ch, Permissions::READ_HISTORY).await?;
            // Separate static statements keep SQLite on the (channel_id,id) index;
            // ORDER BY CASE would force a history-wide sort before applying LIMIT.
            let limit = i64::from(q.limit.unwrap_or(50)) + 1;
            let around = q
                .around
                .map(|v| v.0)
                .or_else(|| q.around_timestamp.map(|t| (t - SNOWFLAKE_EPOCH_MS) << 22));
            let mut older_more = false;
            let mut newer_more = false;
            let ids = if let Some(pivot) = around {
                let count = i64::from(q.limit.unwrap_or(50));
                let mut left = sqlx::query_scalar::<_, i64>("SELECT id FROM messages WHERE channel_id=? AND id<=? AND deleted_at IS NULL ORDER BY id DESC LIMIT ?")
                    .bind(id(ch)?).bind(pivot as i64).bind(count + 1).fetch_all(&mut *c).await?;
                let mut right = sqlx::query_scalar::<_, i64>("SELECT id FROM messages WHERE channel_id=? AND id>? AND deleted_at IS NULL ORDER BY id ASC LIMIT ?")
                    .bind(id(ch)?).bind(pivot as i64).bind(count + 1).fetch_all(&mut *c).await?;
                let left_count = (((count + 1) / 2) as usize)
                    .min(left.len())
                    .max((count as usize).saturating_sub(right.len()))
                    .min(left.len());
                let right_count = (count as usize - left_count).min(right.len());
                older_more = left.len() > left_count;
                newer_more = right.len() > right_count;
                left.truncate(left_count);
                right.truncate(right_count);
                left.extend(right);
                left
            } else if let Some(after) = q.after {
                sqlx::query_scalar::<_,i64>("SELECT id FROM messages WHERE channel_id=? AND id>? AND deleted_at IS NULL ORDER BY id ASC LIMIT ?").bind(id(ch)?).bind(id(after)?).bind(limit).fetch_all(&mut *c).await?
            } else if let Some(before) = q.before {
                sqlx::query_scalar::<_,i64>("SELECT id FROM messages WHERE channel_id=? AND id<? AND deleted_at IS NULL ORDER BY id DESC LIMIT ?").bind(id(ch)?).bind(id(before)?).bind(limit).fetch_all(&mut *c).await?
            } else {
                sqlx::query_scalar::<_,i64>("SELECT id FROM messages WHERE channel_id=? AND deleted_at IS NULL ORDER BY id DESC LIMIT ?").bind(id(ch)?).bind(limit).fetch_all(&mut *c).await?
            };
            let mut items = vec![];
            for m in ids {
                items.push(message(&mut c, u, ch, Id(m as u64), false).await?);
            }
            let mut page = page(items, q, cursor, |v| v.id, true);
            if around.is_some() {
                page.next_before = page.items.last().map(|m| m.id).filter(|_| older_more);
                page.next_after = page.items.first().map(|m| m.id).filter(|_| newer_more);
                page.has_more = older_more || newer_more;
            }
            let mut authors: Vec<_> = page.items.iter().map(|m| m.author_id).collect();
            authors.sort();
            authors.dedup();
            for a in authors {
                page.users.push(user(&mut c, a).await?);
            }
            Ok(json(page))
        }
        ["channels", ch, "messages", m] => {
            let ch = url_id(ch)?;
            text(&mut c, u, ch, Permissions::READ_HISTORY).await?;
            Ok(json(Snapshot {
                data: message(&mut c, u, ch, url_id(m)?, false).await?,
                cursor,
            }))
        }
        _ => Err(Error::new(ErrorCode::NotFound)),
    }
}
pub fn url_id(s: &str) -> Result<Id> {
    let i = Id(s
        .parse()
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?);
    id(i)?;
    Ok(i)
}
fn validate_page(q: PageQuery) -> Result<()> {
    require(
        [
            q.before.is_some(),
            q.after.is_some(),
            q.around.is_some(),
            q.around_timestamp.is_some(),
        ]
        .into_iter()
        .filter(|v| *v)
        .count()
            <= 1
            && q.limit.is_none_or(|l| (1..=50).contains(&l))
            && q.around_timestamp.is_none_or(|t| {
                t >= SNOWFLAKE_EPOCH_MS && t - SNOWFLAKE_EPOCH_MS <= (i64::MAX as u64 >> 22)
            }),
        ErrorCode::InvalidRequest,
    )?;
    opt(q.around)?;
    opt(q.before)?;
    opt(q.after)?;
    Ok(())
}
fn page<T, F: Fn(&T) -> Id>(
    mut items: Vec<T>,
    q: PageQuery,
    cursor: ResumeCursor,
    key: F,
    messages: bool,
) -> Page<T> {
    let desc = q.before.is_some() || (messages && q.after.is_none());
    items.retain(|v| q.before.is_none_or(|b| key(v) < b) && q.after.is_none_or(|a| key(v) > a));
    items.sort_by_key(|v| key(v));
    if desc {
        items.reverse();
    }
    let limit = usize::from(q.limit.unwrap_or(50));
    let has_more = items.len() > limit;
    items.truncate(limit);
    let last = items.last().map(key).filter(|_| has_more);
    Page {
        items,
        users: vec![],
        next_before: if desc { last } else { None },
        next_after: if desc { None } else { last },
        has_more,
        cursor,
    }
}

async fn verified_login(
    c: &mut sqlx::SqliteConnection,
    username: &str,
    user: Id,
    verified_hash: &str,
) -> Result<Id> {
    let current: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE id=? AND username=? AND password_hash=?")
            .bind(id(user)?)
            .bind(username)
            .bind(verified_hash)
            .fetch_optional(c)
            .await?;
    require(current.is_some(), ErrorCode::InvalidCredentials)?;
    Ok(user)
}
pub(crate) fn auth_bucket(app: &App, r: &Request<Body>) -> String {
    let peer = r
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|v| v.0.ip());
    let ip = if peer == Some(app.0.config.trusted_proxy) {
        r.headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse::<std::net::IpAddr>().ok())
            .or(peer)
    } else {
        peer
    }
    .map(|v| v.to_string())
    .unwrap_or_else(|| "local".into());
    format!("auth-ip:{ip}")
}

pub(crate) fn verify_password(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc).is_ok_and(|h| argon().verify_password(password.as_bytes(), &h).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn s3_unpolled_slow_body_releases_admission_and_separate_pool_preserves_rest() {
        use std::{sync::Arc, time::Duration};
        let pool = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = pool.clone().try_acquire_owned().unwrap();
        let body = Body::from_stream(futures_util::stream::iter(
            (0..10).map(|_| Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"chunk"))),
        ));
        let response = timed_response(
            Response::new(body),
            vec![permit],
            tokio_util::sync::CancellationToken::new(),
            Duration::from_millis(20),
            Duration::from_millis(100),
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(pool.available_permits(), 1);
        drop(response);
        let (_dir, app) = crate::audit_tests::fixture().await;
        let transfers = app.0.transfers.clone().try_acquire_many_owned(64).unwrap();
        use tower::ServiceExt;
        let r = router(app.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        drop(transfers);
        let permits: Vec<_> = (0..3)
            .map(|_| download_permit(&app, "download-user:1".into(), 3).unwrap())
            .collect();
        assert!(download_permit(&app, "download-user:1".into(), 3).is_err());
        assert!(download_permit(&app, "download-user:2".into(), 3).is_ok());
        drop(permits);
        let permits: Vec<_> = (0..8)
            .map(|_| download_permit(&app, "download-ip:127.0.0.1".into(), 8).unwrap())
            .collect();
        assert!(download_permit(&app, "download-ip:127.0.0.1".into(), 8).is_err());
        drop(permits);
    }
    #[tokio::test]
    async fn login_cannot_follow_reassigned_username() {
        let mut c = sqlx::SqliteConnection::connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE users(id INTEGER PRIMARY KEY,username TEXT UNIQUE,password_hash TEXT)",
        )
        .execute(&mut c)
        .await
        .unwrap();
        sqlx::query("INSERT INTO users VALUES(1,'alice','verified')")
            .execute(&mut c)
            .await
            .unwrap();
        assert_eq!(
            verified_login(&mut c, "alice", Id(1), "verified")
                .await
                .unwrap(),
            Id(1)
        );
        // Models a rename and a new registration while Argon2 verification ran.
        sqlx::query("UPDATE users SET username='renamed' WHERE id=1")
            .execute(&mut c)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users VALUES(2,'alice','different')")
            .execute(&mut c)
            .await
            .unwrap();
        assert_eq!(
            verified_login(&mut c, "alice", Id(1), "verified")
                .await
                .unwrap_err()
                .0
                .code,
            ErrorCode::InvalidCredentials
        );
        assert_eq!(
            verified_login(&mut c, "alice", Id(2), "verified")
                .await
                .unwrap_err()
                .0
                .code,
            ErrorCode::InvalidCredentials
        );
    }
}
