//! "Request my data": a ZIP of everything the account owns, built by one
//! background worker for the whole server (so at most one archive is written at
//! a time), streamed to disk page by page, downloadable only by its owner for
//! seven days. The worker sleeps until a job is queued or the next archive is
//! due to expire; it never scans on a period.
use super::*;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sqlx::{sqlite::SqliteRow, Column, Connection, Row, TypeInfo, ValueRef};
use std::{collections::HashSet, io::Write, path::PathBuf, sync::Mutex, time::Duration};

pub(super) const TTL: u64 = 7 * 86_400_000;
pub(super) const COOLDOWN: u64 = 86_400_000;
/// Waiting jobs across the server; more are refused until the queue drains.
const MAX_QUEUED: i64 = 64;
/// Attachment bytes copied into one archive. Past this they are listed, not copied.
pub(super) const MAX_ATTACHMENT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const PAGE: i64 = 500;
const MAX_OPTED: usize = 4096;
/// Per-account tables exported as they are (rows whose `user_id` is the account).
const SETTINGS_TABLES: &[&str] = &[
    "notification_settings",
    "nicknames",
    "member_roles",
    "server_folders",
    "gif_favorites",
    "nav_pins",
    "nav_favorite_categories",
    "nav_favorites",
    "nav_rail",
    "saved_messages",
    "read_states",
    "reactions",
    "session_devices",
    // Added by the later features (`docs/features/integration.md`).
    "poll_votes",
    "thread_members",
    "soundboard_favorites",
    "soundboard_entrance",
    "soundboard_entrance_enabled",
];

#[derive(Default)]
pub(super) struct Exports {
    wake: tokio::sync::Notify,
    /// Stream IDs that asked for export events (only new web clients do).
    opted: Mutex<HashSet<String>>,
}

pub(super) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/users/@me/data-export", get(status).post(request))
        .route(
            "/api/v1/users/@me/data-export/{id}",
            axum::routing::delete(remove),
        )
        .route("/api/v1/users/@me/data-export/{id}/download", get(download))
}

pub(crate) fn wake(app: &App) {
    app.0.authsec.export.wake.notify_one();
}

fn dir(app: &App) -> PathBuf {
    app.0.config.data_dir.join("exports")
}

fn export_row(r: &SqliteRow) -> Result<DataExport> {
    Ok(DataExport {
        id: r.get::<i64, _>("id").to_string(),
        state: db::en(r.get("state"))?,
        requested_at: r.get::<i64, _>("requested_at") as u64,
        ready_at: r.get::<Option<i64>, _>("ready_at").map(|v| v as u64),
        expires_at: r.get::<Option<i64>, _>("expires_at").map(|v| v as u64),
        byte_size: r.get::<Option<i64>, _>("byte_size").map(|v| v as u64),
    })
}

async fn latest(c: &mut SqliteConnection, user: Id) -> Result<Option<DataExport>> {
    sqlx::query(
        "SELECT * FROM data_exports WHERE user_id=? ORDER BY requested_at DESC,id DESC LIMIT 1",
    )
    .bind(id(user)?)
    .fetch_optional(c)
    .await?
    .as_ref()
    .map(export_row)
    .transpose()
}

fn next_request_at(export: &Option<DataExport>) -> Option<u64> {
    export
        .as_ref()
        .filter(|e| e.state != DataExportState::Failed)
        .map(|e| e.requested_at + COOLDOWN)
        .filter(|t| *t > state::now())
}

async fn emit(app: &App, user: Id, export: DataExport) {
    let mut core = app.0.core.lock().await;
    let opted = app
        .0
        .authsec
        .export
        .opted
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = core.emit_filtered(&[user], Event::DataExportUpdated(export), |s| {
        opted.contains(&s.id)
    });
}

async fn status(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<axum::Json<DataExportStatus>> {
    let (key, user) = authed(&app, &headers).await?;
    {
        let mut core = app.0.core.lock().await;
        core.rate(format!("rest:{}", user.0), 120., 30.)?;
        // This client understands export events: send it the next one.
        if let Some(stream) = core.streams.get(&key) {
            let mut opted = app
                .0
                .authsec
                .export
                .opted
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if opted.len() >= MAX_OPTED {
                let live: HashSet<&String> = core.streams.values().map(|s| &s.id).collect();
                opted.retain(|id| live.contains(id));
            }
            if opted.len() < MAX_OPTED {
                opted.insert(stream.id.clone());
            }
        }
    }
    let export = latest(&mut *app.0.pool.acquire().await?, user).await?;
    Ok(axum::Json(DataExportStatus {
        next_request_at: next_request_at(&export),
        export,
    }))
}

async fn request(State(app): State<App>, headers: HeaderMap) -> Result<http::ApiResponse> {
    let (key, user) = authed(&app, &headers).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("export-user:{}", user.0), 5., 3.)?;
    let export = {
        let _gate = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let mut tx = c.begin().await?;
        let previous = latest(&mut tx, user).await?;
        // One job per account at a time, then one request per day.
        require(
            !previous.as_ref().is_some_and(|e| {
                matches!(e.state, DataExportState::Queued | DataExportState::Running)
            }),
            ErrorCode::Conflict,
        )?;
        if let Some(at) = next_request_at(&previous) {
            return Err(Error::rate((at - state::now()).min(u32::MAX as u64) as u32));
        }
        let queued: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM data_exports WHERE state IN('queued','running')",
        )
        .fetch_one(&mut *tx)
        .await?;
        require(queued < MAX_QUEUED, ErrorCode::Unavailable)?;
        // Re-authentication last: a refused request must not burn the proof.
        crate::reauth::consume(&mut tx, &headers, &key, AccountOperation::DataExport).await?;
        let row = (rand::random::<u64>() >> 1).max(1) as i64;
        sqlx::query(
            "INSERT INTO data_exports(id,user_id,state,requested_at) VALUES(?,?,'queued',?)",
        )
        .bind(row)
        .bind(id(user)?)
        .bind(state::now() as i64)
        .execute(&mut *tx)
        .await?;
        // Keep a short history: the newest three requests.
        sqlx::query("DELETE FROM data_exports WHERE user_id=? AND state IN('failed','expired') AND id NOT IN (SELECT id FROM data_exports WHERE user_id=? ORDER BY requested_at DESC LIMIT 3)")
            .bind(id(user)?)
            .bind(id(user)?)
            .execute(&mut *tx)
            .await?;
        let export = latest(&mut tx, user)
            .await?
            .ok_or_else(|| Error::new(ErrorCode::Internal))?;
        tx.commit().await?;
        export
    };
    wake(&app);
    emit(&app, user, export.clone()).await;
    let mut response = http::json(DataExportStatus {
        next_request_at: next_request_at(&Some(export.clone())),
        export: Some(export),
    });
    clear_reauth_cookie(&mut response);
    Ok(response)
}

fn row_id(value: &str) -> Result<i64> {
    value
        .parse::<i64>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))
}

async fn remove(
    State(app): State<App>,
    Path(export): Path<String>,
    headers: HeaderMap,
) -> Result<http::ApiResponse> {
    let (_, user) = authed(&app, &headers).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("rest:{}", user.0), 120., 30.)?;
    let export = row_id(&export)?;
    let key: Option<Option<String>> = {
        let _gate = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let mut tx = c.begin().await?;
        let key: Option<Option<String>> = sqlx::query_scalar(
            "SELECT file_key FROM data_exports WHERE id=? AND user_id=? AND state='ready'",
        )
        .bind(export)
        .bind(id(user)?)
        .fetch_optional(&mut *tx)
        .await?;
        sqlx::query("UPDATE data_exports SET state='expired',file_key=NULL,expires_at=NULL WHERE id=? AND user_id=? AND state='ready'")
            .bind(export)
            .bind(id(user)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        key
    };
    let key = key.ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    if let Some(key) = key {
        let _ = tokio::fs::remove_file(dir(&app).join(format!("{key}.zip"))).await;
    }
    if let Some(e) = latest(&mut *app.0.pool.acquire().await?, user).await? {
        emit(&app, user, e).await;
    }
    Ok(http::ApiResponse::empty())
}

async fn download(
    State(app): State<App>,
    Path(export): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let (_, user) = authed(&app, &headers).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("rest:{}", user.0), 120., 30.)?;
    let export = row_id(&export)?;
    // Someone else's archive is indistinguishable from a missing one.
    let row = sqlx::query("SELECT e.file_key,e.byte_size,u.username FROM data_exports e JOIN users u ON u.id=e.user_id WHERE e.id=? AND e.user_id=? AND e.state='ready' AND e.expires_at>?")
        .bind(export)
        .bind(id(user)?)
        .bind(state::now() as i64)
        .fetch_optional(&app.0.pool)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let key: String = row
        .get::<Option<String>, _>("file_key")
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let file = tokio::fs::File::open(dir(&app).join(format!("{key}.zip")))
        .await
        .map_err(|_| Error::new(ErrorCode::NotFound))?;
    let length = file.metadata().await?.len();
    let name: String = row.get("username");
    let mut r = Body::from_stream(tokio_util::io::ReaderStream::new(file)).into_response();
    let h = r.headers_mut();
    h.insert("content-type", "application/zip".parse().expect("static"));
    h.insert("content-length", length.into());
    h.insert(
        "content-disposition",
        format!("attachment; filename=\"sticktalk-data-{name}.zip\"")
            .parse()
            .map_err(|_| Error::new(ErrorCode::Internal))?,
    );
    h.insert("cache-control", "no-store".parse().expect("static"));
    Ok(r)
}

// ----- Worker ------------------------------------------------------------------

pub(super) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(error) = recover(&app).await {
            tracing::warn!(?error, "data export recovery failed");
        }
        loop {
            let next = match work(&app).await {
                Ok(next) => next,
                Err(error) => {
                    tracing::warn!(?error, "data export worker failed");
                    Some(state::now() + 60_000)
                }
            };
            let wait = next.map(|t| t.saturating_sub(state::now()).min(30 * 86_400_000));
            tokio::select! {
                _ = app.0.shutdown.cancelled() => break,
                _ = app.0.authsec.export.wake.notified() => {}
                _ = async {
                    match wait {
                        Some(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                        None => std::future::pending::<()>().await,
                    }
                } => {}
            }
        }
    });
}

/// After a restart: interrupted jobs start again; stray files are removed.
async fn recover(app: &App) -> Result<()> {
    tokio::fs::create_dir_all(dir(app)).await?;
    let _gate = app.0.core.lock().await;
    sqlx::query("UPDATE data_exports SET state='queued' WHERE state='running'")
        .execute(&app.0.pool)
        .await?;
    Ok(())
}

/// Run every queued job, expire what is due, delete orphans; return the next deadline.
pub(super) async fn work(app: &App) -> Result<Option<u64>> {
    loop {
        let job = sqlx::query("SELECT id,user_id FROM data_exports WHERE state='queued' ORDER BY requested_at,id LIMIT 1")
            .fetch_optional(&app.0.pool)
            .await?;
        let Some(job) = job else { break };
        run(app, job.get("id"), Id(job.get::<i64, _>("user_id") as u64)).await?;
    }
    let expired: Vec<(i64, i64, Option<String>)> = {
        let _gate = app.0.core.lock().await;
        sqlx::query("UPDATE data_exports SET state='expired',expires_at=NULL WHERE state='ready' AND expires_at<=? RETURNING id,user_id,file_key")
            .bind(state::now() as i64)
            .fetch_all(&app.0.pool)
            .await?
            .into_iter()
            .map(|r| (r.get("id"), r.get("user_id"), r.get("file_key")))
            .collect()
    };
    for (_, user, _) in &expired {
        let user = Id(*user as u64);
        if let Some(e) = latest(&mut *app.0.pool.acquire().await?, user).await? {
            emit(app, user, e).await;
        }
    }
    // Every archive on disk must belong to a live, ready row.
    let live: HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT file_key FROM data_exports WHERE state='ready' AND file_key IS NOT NULL",
    )
    .fetch_all(&app.0.pool)
    .await?
    .into_iter()
    .collect();
    let mut entries = tokio::fs::read_dir(dir(app)).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        let key = name.strip_suffix(".zip").unwrap_or("");
        if !live.contains(key) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
    sqlx::query(
        "UPDATE data_exports SET file_key=NULL WHERE state='expired' AND file_key IS NOT NULL",
    )
    .execute(&app.0.pool)
    .await?;
    Ok(sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MIN(expires_at) FROM data_exports WHERE state='ready'",
    )
    .fetch_one(&app.0.pool)
    .await?
    .map(|v| v.max(0) as u64))
}

async fn execute(
    app: &App,
    query: sqlx::query::Query<'_, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>,
) -> Result<bool> {
    let _gate = app.0.core.lock().await;
    Ok(query.execute(&app.0.pool).await?.rows_affected() == 1)
}

async fn run(app: &App, job: i64, user: Id) -> Result<()> {
    execute(
        app,
        sqlx::query("UPDATE data_exports SET state='running' WHERE id=? AND state='queued'")
            .bind(job),
    )
    .await?;
    if let Some(e) = latest(&mut *app.0.pool.acquire().await?, user).await? {
        emit(app, user, e).await;
    }
    let key = state::token();
    let part = dir(app).join(format!("{key}.part"));
    let done = dir(app).join(format!("{key}.zip"));
    let (worker, path) = (app.clone(), part.clone());
    let runtime = tokio::runtime::Handle::current();
    let built = tokio::task::spawn_blocking(move || build(&runtime, &worker, user, &path))
        .await
        .map_err(|_| Error::new(ErrorCode::Internal))
        .and_then(|r| r);
    let size = match built {
        Ok(size) if tokio::fs::rename(&part, &done).await.is_ok() => Some(size),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(?error, "data export failed");
            None
        }
    };
    let now = state::now() as i64;
    let ready = match size {
        Some(size) => {
            execute(app, sqlx::query("UPDATE data_exports SET state='ready',ready_at=?,expires_at=?,byte_size=?,file_key=? WHERE id=? AND state='running'")
                .bind(now)
                .bind(now + TTL as i64)
                .bind(size as i64)
                .bind(&key)
                .bind(job))
            .await?
        }
        None => {
            execute(app, sqlx::query("UPDATE data_exports SET state='failed' WHERE id=? AND state='running'").bind(job)).await?;
            false
        }
    };
    if !ready {
        // Failed, or the account was erased meanwhile: no file may remain.
        let _ = tokio::fs::remove_file(&part).await;
        let _ = tokio::fs::remove_file(&done).await;
    }
    if let Some(e) = latest(&mut *app.0.pool.acquire().await?, user).await? {
        emit(app, user, e).await;
    }
    Ok(())
}
// ----- Archive -------------------------------------------------------------------

/// A JSON value for one SQLite cell. Integers beyond 2^53 (IDs) become strings
/// so that JavaScript readers keep every digit.
fn cell(row: &SqliteRow, i: usize) -> Value {
    let Ok(raw) = row.try_get_raw(i) else {
        return Value::Null;
    };
    if raw.is_null() {
        return Value::Null;
    }
    match raw.type_info().name() {
        "INTEGER" | "INT8" | "BIGINT" | "BOOLEAN" => {
            let v: i64 = row.try_get(i).unwrap_or_default();
            if v.unsigned_abs() > (1 << 53) {
                Value::String(v.to_string())
            } else {
                v.into()
            }
        }
        "REAL" => row.try_get::<f64, _>(i).map_or(Value::Null, Value::from),
        "BLOB" => row
            .try_get::<Vec<u8>, _>(i)
            .map_or(Value::Null, |b| Value::String(STANDARD.encode(b))),
        _ => row
            .try_get::<String, _>(i)
            .map_or(Value::Null, Value::String),
    }
}
fn row_json(row: &SqliteRow) -> Value {
    let mut map = serde_json::Map::new();
    for column in row.columns() {
        let name = column.name();
        // Never export a credential, even a hashed one.
        if name.ends_with("_hash") || name == "secret" || name == "token" {
            continue;
        }
        map.insert(name.to_owned(), cell(row, column.ordinal()));
    }
    Value::Object(map)
}
fn big(v: i64) -> Value {
    Value::String(v.to_string())
}
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
/// A safe archive file name: letters, digits, dot, dash, underscore; bounded.
fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    let cleaned = cleaned.trim_start_matches('.').to_owned();
    if cleaned.is_empty() {
        "file".into()
    } else {
        cleaned
    }
}

struct Archive<'a> {
    zip: zip::ZipWriter<std::io::BufWriter<std::fs::File>>,
    runtime: &'a tokio::runtime::Handle,
    app: &'a App,
}
impl Archive<'_> {
    fn options(compress: bool) -> zip::write::SimpleFileOptions {
        zip::write::SimpleFileOptions::default()
            .compression_method(if compress {
                zip::CompressionMethod::Deflated
            } else {
                zip::CompressionMethod::Stored
            })
            .large_file(true)
    }
    fn json(&mut self, name: &str, value: &Value) -> Result<()> {
        self.zip
            .start_file(name, Self::options(true))
            .map_err(zip_error)?;
        serde_json::to_writer_pretty(&mut self.zip, value)
            .map_err(|_| Error::new(ErrorCode::Internal))?;
        Ok(())
    }
    fn text(&mut self, name: &str, value: &str) -> Result<()> {
        self.zip
            .start_file(name, Self::options(true))
            .map_err(zip_error)?;
        self.zip.write_all(value.as_bytes())?;
        Ok(())
    }
    fn rows(
        &self,
        query: sqlx::query::Query<'_, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>,
    ) -> Result<Vec<SqliteRow>> {
        Ok(self.runtime.block_on(query.fetch_all(&self.app.0.pool))?)
    }
    /// Copy one stored file into the archive, chunk by chunk.
    fn copy(&mut self, name: &str, key: &str, size: u64) -> Result<()> {
        use futures_util::StreamExt;
        let body = self
            .runtime
            .block_on(self.app.0.tier.get(self.app, key, 0, size))?;
        let mut stream = body.into_data_stream();
        self.zip
            .start_file(name, Self::options(false))
            .map_err(zip_error)?;
        while let Some(chunk) = self.runtime.block_on(stream.next()) {
            let chunk = chunk.map_err(|_| Error::new(ErrorCode::Unavailable))?;
            self.zip.write_all(&chunk)?;
        }
        Ok(())
    }
}
fn zip_error(_: zip::result::ZipError) -> Error {
    Error::new(ErrorCode::Internal)
}

/// Write the archive at `path` and return its size in bytes.
pub(super) fn build(
    runtime: &tokio::runtime::Handle,
    app: &App,
    user: Id,
    path: &std::path::Path,
) -> Result<u64> {
    let u = id(user)?;
    let file = std::fs::File::create(path)?;
    let mut a = Archive {
        zip: zip::ZipWriter::new(std::io::BufWriter::new(file)),
        runtime,
        app,
    };
    let now = state::now();
    let me = a
        .rows(sqlx::query("SELECT id,username,display_name,status_text,desired_presence,created_at,avatar_id,password_hash<>'' AS has_password FROM users WHERE id=?").bind(u))?
        .pop()
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let username: String = me.get("username");
    let google = !a
        .rows(sqlx::query("SELECT 1 FROM oauth_identities WHERE user_id=?").bind(u))?
        .is_empty();
    let passkeys: Vec<Value> = a
        .rows(sqlx::query("SELECT name,created_at,last_used_at,backed_up FROM passkeys WHERE user_id=? ORDER BY created_at").bind(u))?
        .iter()
        .map(row_json)
        .collect();
    let email = a
        .rows(
            sqlx::query("SELECT email,verified_at,pending_email FROM user_emails WHERE user_id=?")
                .bind(u),
        )?
        .first()
        .map(row_json);
    let sessions: Vec<Value> = a
        .rows(sqlx::query("SELECT created_at,last_used_at,expires_at FROM sessions WHERE user_id=? ORDER BY created_at").bind(u))?
        .iter()
        .map(row_json)
        .collect();
    let profile = a
        .rows(sqlx::query("SELECT * FROM user_profiles WHERE user_id=?").bind(u))?
        .first()
        .map(row_json);
    a.json(
        "account.json",
        &json!({
            "format": "sticktalk-data-export",
            "format_version": 1,
            "generated_at": now,
            "server": app.0.config.public_url,
            "user": {
                "id": big(u),
                "username": username,
                "display_name": me.get::<Option<String>, _>("display_name"),
                "status_text": me.get::<Option<String>, _>("status_text"),
                "presence": me.get::<String, _>("desired_presence"),
                "created_at": me.get::<i64, _>("created_at"),
            },
            "profile": profile,
            "sign_in": {
                "has_password": me.get::<bool, _>("has_password"),
                "google_linked": google,
                "passkeys": passkeys,
                "email": email,
            },
            "sessions": sessions,
        }),
    )?;

    // Settings: every per-account row of the known settings tables.
    let mut settings = serde_json::Map::new();
    for table in SETTINGS_TABLES {
        let has_user = !a
            .rows(
                sqlx::query("SELECT 1 FROM pragma_table_info(?) WHERE name='user_id'").bind(*table),
            )?
            .is_empty();
        if !has_user {
            continue;
        }
        // `table` comes from the fixed list above, never from input.
        let rows = a.rows(
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT * FROM {table} WHERE user_id=? LIMIT 100000"
            )))
            .bind(u),
        )?;
        settings.insert(
            (*table).to_owned(),
            Value::Array(rows.iter().map(row_json).collect()),
        );
    }
    a.json("settings.json", &Value::Object(settings))?;

    let servers = a.rows(sqlx::query("SELECT g.id,g.name,g.owner_id=gm.user_id AS owner,gm.role,gm.joined_at FROM guild_members gm JOIN guilds g ON g.id=gm.guild_id WHERE gm.user_id=? ORDER BY gm.joined_at").bind(u))?;
    let servers_json: Vec<Value> = servers
        .iter()
        .map(|r| json!({"id": big(r.get("id")), "name": r.get::<String,_>("name"), "owner": r.get::<bool,_>("owner"), "role": r.get::<String,_>("role"), "joined_at": r.get::<i64,_>("joined_at")}))
        .collect();
    a.json("servers.json", &Value::Array(servers_json))?;

    let relationships: Vec<Value> = a
        .rows(sqlx::query("SELECT r.kind,r.since,u.id,u.username,u.display_name FROM relationships r JOIN users u ON u.id=r.other_id WHERE r.user_id=? ORDER BY r.since").bind(u))?
        .iter()
        .map(|r| json!({"kind": r.get::<String,_>("kind"), "since": r.get::<i64,_>("since"), "user": {"id": big(r.get("id")), "username": r.get::<String,_>("username"), "display_name": r.get::<Option<String>,_>("display_name")}}))
        .collect();
    a.json("relationships.json", &Value::Array(relationships))?;

    // Messages: one file per conversation, streamed a page at a time.
    let channels = a.rows(sqlx::query("SELECT c.id,c.kind,c.name,c.guild_id,g.name AS guild_name,(SELECT COUNT(*) FROM messages m WHERE m.channel_id=c.id AND m.author_id=? AND m.deleted_at IS NULL) AS count FROM channels c LEFT JOIN guilds g ON g.id=c.guild_id WHERE c.id IN (SELECT DISTINCT channel_id FROM messages WHERE author_id=? AND deleted_at IS NULL) ORDER BY c.id").bind(u).bind(u))?;
    let mut channel_index = vec![];
    let mut total = 0i64;
    for ch in &channels {
        let channel: i64 = ch.get("id");
        let count: i64 = ch.get("count");
        total += count;
        let file = format!("messages/{channel}.json");
        let kind: String = ch.get("kind");
        let recipients: Vec<String> = if ch.get::<Option<i64>, _>("guild_id").is_none() {
            a.rows(sqlx::query("SELECT u.username FROM dm_members d JOIN users u ON u.id=d.user_id WHERE d.channel_id=? AND d.user_id<>? ORDER BY u.username LIMIT 16").bind(channel).bind(u))?
                .iter()
                .map(|r| r.get("username"))
                .collect()
        } else {
            vec![]
        };
        let header = json!({
            "id": big(channel),
            "kind": kind,
            "name": ch.get::<Option<String>,_>("name"),
            "server": ch.get::<Option<i64>,_>("guild_id").map(big),
            "server_name": ch.get::<Option<String>,_>("guild_name"),
            "recipients": recipients,
        });
        channel_index.push(json!({"channel": header, "messages": count, "file": file}));
        a.zip
            .start_file(&file, Archive::options(true))
            .map_err(zip_error)?;
        let mut head =
            serde_json::to_string(&header).map_err(|_| Error::new(ErrorCode::Internal))?;
        head.pop(); // Reopen the object to append the message array.
        write!(a.zip, "{head},\"messages\":[")?;
        let mut after = 0i64;
        let mut first = true;
        loop {
            let page = a.rows(sqlx::query("SELECT m.id,m.created_at,m.edited_at,m.reply_to,m.content,(SELECT json_group_array(json_object('id',CAST(up.id AS TEXT),'filename',up.filename,'content_type',up.content_type,'byte_size',up.byte_size)) FROM message_attachments ma JOIN uploads up ON up.id=ma.attachment_id WHERE ma.message_id=m.id) AS attachments FROM messages m WHERE m.channel_id=? AND m.author_id=? AND m.deleted_at IS NULL AND m.id>? ORDER BY m.id LIMIT ?").bind(channel).bind(u).bind(after).bind(PAGE))?;
            let Some(last) = page.last() else { break };
            after = last.get("id");
            for m in &page {
                let attachments: Value = serde_json::from_str(&m.get::<String, _>("attachments"))
                    .unwrap_or(Value::Array(vec![]));
                let item = json!({
                    "id": big(m.get("id")),
                    "timestamp": m.get::<i64,_>("created_at"),
                    "edited_at": m.get::<Option<i64>,_>("edited_at"),
                    "reply_to": m.get::<Option<i64>,_>("reply_to").map(big),
                    "content": m.get::<String,_>("content"),
                    "attachments": attachments,
                });
                if !first {
                    a.zip.write_all(b",\n")?;
                }
                first = false;
                serde_json::to_writer(&mut a.zip, &item)
                    .map_err(|_| Error::new(ErrorCode::Internal))?;
            }
            if (page.len() as i64) < PAGE {
                break;
            }
        }
        a.zip.write_all(b"]}\n")?;
    }
    a.json("messages/index.json", &Value::Array(channel_index.clone()))?;

    // Attachments: copied while within the size budget, otherwise listed.
    let mut included = 0u64;
    let mut listing = vec![];
    let mut after = 0i64;
    loop {
        let page = a.rows(sqlx::query("SELECT DISTINCT up.id,up.filename,up.content_type,up.byte_size,up.storage_key,ma.message_id FROM uploads up JOIN message_attachments ma ON ma.attachment_id=up.id JOIN messages m ON m.id=ma.message_id WHERE m.author_id=? AND m.deleted_at IS NULL AND up.id>? ORDER BY up.id LIMIT 200").bind(u).bind(after))?;
        let Some(last) = page.last() else { break };
        after = last.get("id");
        for f in &page {
            let attachment: i64 = f.get("id");
            let size = f.get::<i64, _>("byte_size").max(0) as u64;
            let filename: String = f.get("filename");
            let mut entry = json!({
                "id": big(attachment),
                "message": big(f.get("message_id")),
                "filename": filename,
                "content_type": f.get::<String,_>("content_type"),
                "byte_size": size,
                "url": format!("{}/api/v1/files/{attachment}", app.0.config.public_url.trim_end_matches('/')),
            });
            if included + size <= MAX_ATTACHMENT_BYTES {
                let name = format!("attachments/{attachment}-{}", safe_name(&filename));
                match a.copy(&name, &f.get::<String, _>("storage_key"), size) {
                    Ok(()) => {
                        included += size;
                        entry["file"] = Value::String(name);
                    }
                    Err(_) => entry["file"] = Value::Null,
                }
            } else {
                entry["file"] = Value::Null;
            }
            listing.push(entry);
        }
        if page.len() < 200 {
            break;
        }
    }
    let attachments = listing.len();
    let copied = listing.iter().filter(|e| e["file"].is_string()).count();
    a.json("attachments/index.json", &Value::Array(listing))?;

    let extra = features(&mut a, u)?;

    // Human-readable index. Every user-controlled string is HTML-escaped.
    let readme = format!(
        "StickTalk data export for @{username}\nGenerated {now} (Unix milliseconds) by {server}.\n\n\
account.json        Your account, profile, sign-in methods and active sessions.\n\
settings.json       Your settings, saved messages, pins, favorites and folders.\n\
servers.json        Servers you are in, and your role in each.\n\
relationships.json  Friends, friend requests and blocked people.\n\
messages/           One JSON file per conversation with every message you wrote ({total} in {conversations} conversations). messages/index.json lists them.\n\
attachments/        Files you attached ({copied} of {attachments} copied here; attachments/index.json lists all of them with their links).\n\
threads.json        Threads and forum posts you started (their messages are in messages/).\n\
scheduled.json      Messages you scheduled and have not sent yet ({scheduled}), with their held files.\n\
soundboard/         Soundboard sounds you uploaded ({sounds}); soundboard/index.json lists them.\n\
camera-backgrounds/ Your custom camera backgrounds ({backgrounds}).\n\n\
All times are Unix milliseconds (UTC). IDs are strings. Open index.html in a browser for an overview.\n",
        server = app.0.config.public_url,
        conversations = channels.len(),
        scheduled = extra.scheduled,
        sounds = extra.sounds,
        backgrounds = extra.backgrounds,
    );
    a.text("README.txt", &readme)?;
    let mut html = format!(
        "<!doctype html><meta charset=utf-8><title>StickTalk data export</title><style>body{{font:15px system-ui,sans-serif;max-width:760px;margin:32px auto;padding:0 16px;color:#111}}td,th{{padding:4px 12px 4px 0;text-align:left}}</style><h1>StickTalk data for @{}</h1><p>{} messages in {} conversations, {} servers, {} of {} attachments included.</p><h2>Files</h2><ul><li><a href=\"account.json\">account.json</a><li><a href=\"settings.json\">settings.json</a><li><a href=\"servers.json\">servers.json</a><li><a href=\"relationships.json\">relationships.json</a><li><a href=\"messages/index.json\">messages/index.json</a><li><a href=\"attachments/index.json\">attachments/index.json</a><li><a href=\"threads.json\">threads.json</a><li><a href=\"scheduled.json\">scheduled.json</a><li><a href=\"soundboard/index.json\">soundboard/index.json</a><li><a href=\"camera-backgrounds/index.json\">camera-backgrounds/index.json</a></ul><h2>Conversations</h2><table><tr><th>Conversation<th>Server<th>Messages",
        html_escape(&username),
        total,
        channels.len(),
        servers.len(),
        copied,
        attachments
    );
    for entry in &channel_index {
        let c = &entry["channel"];
        let name = c["name"]
            .as_str()
            .map(|n| format!("#{n}"))
            .unwrap_or_else(|| {
                let people: Vec<&str> = c["recipients"]
                    .as_array()
                    .map(|v| v.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                format!("Direct messages with {}", people.join(", "))
            });
        html.push_str(&format!(
            "<tr><td><a href=\"{}\">{}</a><td>{}<td>{}",
            html_escape(entry["file"].as_str().unwrap_or_default()),
            html_escape(&name),
            html_escape(c["server_name"].as_str().unwrap_or("")),
            entry["messages"]
        ));
    }
    html.push_str("</table>\n");
    a.text("index.html", &html)?;

    let mut out = a.zip.finish().map_err(zip_error)?;
    out.flush()?;
    let file = out
        .into_inner()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    file.sync_all()?;
    Ok(file.metadata()?.len())
}

/// What the later features keep for an account (threads, scheduled messages,
/// soundboard uploads, camera backgrounds). Poll votes, thread memberships,
/// soundboard favorites and entrance sounds are in `settings.json`; thread
/// and forum post messages are in `messages/` like any message.
struct FeatureCounts {
    scheduled: usize,
    sounds: usize,
    backgrounds: usize,
}
fn features(a: &mut Archive, u: i64) -> Result<FeatureCounts> {
    let threads: Vec<Value> = a
        .rows(sqlx::query("SELECT t.channel_id,ch.name,t.parent_id,p.name AS parent_name,t.guild_id,t.forum_post,t.private,t.archived,t.locked,t.created_at,t.message_count,(SELECT json_group_array(ft.name) FROM thread_tags tt JOIN forum_tags ft ON ft.id=tt.tag_id AND ft.forum_id=t.parent_id WHERE tt.thread_id=t.channel_id) AS tags FROM threads t JOIN channels ch ON ch.id=t.channel_id LEFT JOIN channels p ON p.id=t.parent_id WHERE t.owner_id=? ORDER BY t.created_at LIMIT 100000").bind(u))?
        .iter()
        .map(|r| {
            json!({
                "id": big(r.get("channel_id")),
                "name": r.get::<String,_>("name"),
                "kind": if r.get::<i64,_>("forum_post") != 0 { "forum_post" } else { "thread" },
                "parent": big(r.get("parent_id")),
                "parent_name": r.get::<Option<String>,_>("parent_name"),
                "server": big(r.get("guild_id")),
                "private": r.get::<i64,_>("private") != 0,
                "archived": r.get::<i64,_>("archived") != 0,
                "locked": r.get::<i64,_>("locked") != 0,
                "created_at": r.get::<i64,_>("created_at"),
                "messages": r.get::<i64,_>("message_count"),
                "tags": serde_json::from_str::<Value>(&r.get::<Option<String>,_>("tags").unwrap_or_default()).unwrap_or(Value::Array(vec![])),
            })
        })
        .collect();
    a.json("threads.json", &Value::Array(threads))?;

    let scheduled: Vec<Value> = a
        .rows(sqlx::query("SELECT s.id,s.channel_id,s.content,s.mention_ids,s.send_at,s.created_at,s.failure,(SELECT json_group_array(json_object('id',CAST(up.id AS TEXT),'filename',up.filename,'content_type',up.content_type,'byte_size',up.byte_size)) FROM scheduled_attachments sa JOIN uploads up ON up.id=sa.attachment_id WHERE sa.scheduled_id=s.id) AS attachments FROM scheduled_messages s WHERE s.author_id=? ORDER BY s.send_at").bind(u))?
        .iter()
        .map(|r| {
            json!({
                "id": big(r.get("id")),
                "channel": big(r.get("channel_id")),
                "content": r.get::<String,_>("content"),
                "mention_ids": serde_json::from_str::<Value>(&r.get::<String,_>("mention_ids")).unwrap_or(Value::Array(vec![])),
                "send_at": r.get::<i64,_>("send_at"),
                "created_at": r.get::<i64,_>("created_at"),
                "failure": r.get::<Option<String>,_>("failure"),
                "attachments": serde_json::from_str::<Value>(&r.get::<String,_>("attachments")).unwrap_or(Value::Array(vec![])),
            })
        })
        .collect();
    let scheduled_count = scheduled.len();
    a.json("scheduled.json", &Value::Array(scheduled))?;

    // Soundboard sounds this account uploaded (each at most 512 KB).
    let mut sounds = vec![];
    let mut after = 0i64;
    loop {
        let page = a.rows(sqlx::query("SELECT s.id,s.guild_id,g.name AS guild_name,s.name,s.emoji,s.volume,s.duration_ms,s.format,s.data,s.created_at FROM soundboard_sounds s LEFT JOIN guilds g ON g.id=s.guild_id WHERE s.created_by=? AND s.id>? ORDER BY s.id LIMIT 50").bind(u).bind(after))?;
        let Some(last) = page.last() else { break };
        after = last.get("id");
        for r in &page {
            let sid: i64 = r.get("id");
            let name: String = r.get("name");
            let format: String = r.get("format");
            let file = format!("soundboard/{sid}-{}.{format}", safe_name(&name));
            a.zip
                .start_file(&file, Archive::options(false))
                .map_err(zip_error)?;
            a.zip.write_all(&r.get::<Vec<u8>, _>("data"))?;
            sounds.push(json!({
                "id": big(sid),
                "server": big(r.get("guild_id")),
                "server_name": r.get::<Option<String>,_>("guild_name"),
                "name": name,
                "emoji": r.get::<Option<String>,_>("emoji"),
                "volume": r.get::<i64,_>("volume"),
                "duration_ms": r.get::<i64,_>("duration_ms"),
                "created_at": r.get::<i64,_>("created_at"),
                "file": file,
            }));
        }
        if page.len() < 50 {
            break;
        }
    }
    let sound_count = sounds.len();
    a.json("soundboard/index.json", &Value::Array(sounds))?;

    // Custom camera backgrounds (at most ten, each at most 10 MB).
    let mut backgrounds = vec![];
    for r in a.rows(sqlx::query("SELECT id,kind,mime,data,created_at FROM camera_backgrounds WHERE user_id=? ORDER BY created_at LIMIT 10").bind(u))? {
        let bid: i64 = r.get("id");
        let mime: String = r.get("mime");
        let ext = match mime.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            "image/gif" => "gif",
            "video/mp4" => "mp4",
            "video/webm" => "webm",
            _ => "bin",
        };
        let file = format!("camera-backgrounds/{bid}.{ext}");
        a.zip
            .start_file(&file, Archive::options(false))
            .map_err(zip_error)?;
        a.zip.write_all(&r.get::<Vec<u8>, _>("data"))?;
        backgrounds.push(json!({
            "id": big(bid),
            "kind": r.get::<String,_>("kind"),
            "content_type": mime,
            "created_at": r.get::<i64,_>("created_at"),
            "file": file,
        }));
    }
    let background_count = backgrounds.len();
    a.json("camera-backgrounds/index.json", &Value::Array(backgrounds))?;
    Ok(FeatureCounts {
        scheduled: scheduled_count,
        sounds: sound_count,
        backgrounds: background_count,
    })
}
