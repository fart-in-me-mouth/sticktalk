//! What can be launched: the built-in activities, the ones the instance
//! administrator registered (`sticktalk-server admin activities ...`), and each
//! server's choice of which of them its members may use.
use super::*;
use crate::http;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, put},
};
use clap::Subcommand;
use serde::Deserialize;
use sqlx::Row;

/// The built-in activities, bundled with the server and loaded only when launched.
pub(super) fn builtins() -> Vec<ActivityInfo> {
    let one =
        |id: &str, name: &str, description: &str, icon: &str, min: u8, max: u8| ActivityInfo {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            icon: icon.into(),
            min_players: min,
            max_players: max,
            url: format!("/activities/{id}/"),
            builtin: true,
            origin: None,
        };
    vec![
        one(
            "watch-together",
            "Watch Together",
            "Watch a video or listen to audio from this server together, in sync.",
            "play",
            1,
            16,
        ),
        one(
            "whiteboard",
            "Whiteboard",
            "Draw, write and pin sticky notes on one shared canvas.",
            "pencil",
            1,
            16,
        ),
        one(
            "four-in-a-row",
            "Four in a Row",
            "Drop discs in turn and line up four before your opponent.",
            "circle",
            2,
            2,
        ),
        one(
            "checkers",
            "Checkers",
            "The classic jumping game on an 8 × 8 board.",
            "layout-grid",
            2,
            2,
        ),
        one(
            "sketch-guess",
            "Sketch & Guess",
            "One person draws a word, everyone else races to guess it.",
            "palette",
            2,
            16,
        ),
    ]
}

fn custom_row(r: &sqlx::sqlite::SqliteRow) -> ActivityInfo {
    let id: i64 = r.get("id");
    ActivityInfo {
        id: format!("custom-{id}"),
        name: r.get("name"),
        description: r.get("description"),
        icon: r.get("icon"),
        min_players: r.get::<i64, _>("min_players") as u8,
        max_players: r.get::<i64, _>("max_players") as u8,
        url: format!("/activities/frame/custom-{id}"),
        builtin: false,
        origin: Some(r.get("origin")),
    }
}

/// A registered activity's frame URL and origin, for the shell (`frames.rs`).
pub(super) async fn custom(c: &mut SqliteConnection, id: &str) -> Result<(String, String)> {
    let n: i64 = id
        .strip_prefix("custom-")
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    sqlx::query_as("SELECT url,origin FROM custom_activities WHERE id=?")
        .bind(n)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))
}

/// Every activity with whether a server has it on. Built-in ones are on
/// unless the server turned them off; registered ones are off until turned on.
async fn for_guild(
    c: &mut SqliteConnection,
    guild: Option<Id>,
) -> Result<Vec<(ActivityInfo, bool)>> {
    let mut out: Vec<(ActivityInfo, bool)> = builtins().into_iter().map(|a| (a, true)).collect();
    let rows = sqlx::query("SELECT * FROM custom_activities ORDER BY id")
        .fetch_all(&mut *c)
        .await?;
    for r in &rows {
        let dms = r.get::<i64, _>("in_dms") != 0;
        out.push((custom_row(r), guild.is_none() && dms));
    }
    if let Some(g) = guild {
        let toggles: Vec<(String, bool)> =
            sqlx::query_as("SELECT activity_id,enabled FROM guild_activities WHERE guild_id=?")
                .bind(db::id(g)?)
                .fetch_all(&mut *c)
                .await?;
        for (id, enabled) in toggles {
            if let Some(a) = out.iter_mut().find(|(a, _)| a.id == id) {
                a.1 = enabled;
            }
        }
    }
    Ok(out)
}

/// The activity `id`, if it may be launched in this server (or in a direct call).
pub(super) async fn available(
    c: &mut SqliteConnection,
    guild: Option<Id>,
    id: &str,
) -> Result<ActivityInfo> {
    for_guild(c, guild)
        .await?
        .into_iter()
        .find(|(a, on)| a.id == id && *on)
        .map(|(a, _)| a)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))
}

/// A file `u` may share into a session: audio or video attached to a message
/// they can read.
pub(super) async fn shareable(c: &mut SqliteConnection, u: Id, a: Id) -> Result<ActivityMedia> {
    let row = sqlx::query("SELECT up.id,up.filename,up.content_type,up.byte_size,m.channel_id,m.created_at FROM message_attachments ma JOIN messages m ON m.id=ma.message_id JOIN uploads up ON up.id=ma.attachment_id WHERE ma.attachment_id=? AND m.deleted_at IS NULL LIMIT 1")
        .bind(db::id(a)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let item = media_row(&row);
    db::text(
        c,
        u,
        item.channel_id,
        Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
    )
    .await?;
    require(
        item.content_type.starts_with("video/") || item.content_type.starts_with("audio/"),
        ErrorCode::UnsupportedMediaType,
    )?;
    Ok(item)
}

fn media_row(r: &sqlx::sqlite::SqliteRow) -> ActivityMedia {
    ActivityMedia {
        attachment_id: Id(r.get::<i64, _>("id") as u64),
        filename: r.get("filename"),
        content_type: r.get("content_type"),
        byte_size: r.get::<i64, _>("byte_size") as u64,
        channel_id: Id(r.get::<i64, _>("channel_id") as u64),
        created_at: r.get::<i64, _>("created_at") as u64,
    }
}

// ---- REST ---------------------------------------------------------------------

pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/activities", get(catalog))
        .route("/api/v1/activities/media", get(media))
        .route("/api/v1/guilds/{id}/activities", get(guild_list))
        .route(
            "/api/v1/guilds/{id}/activities/{activity}",
            put(guild_toggle),
        )
        .merge(super::frames::routes())
}

#[derive(Deserialize)]
struct ChannelQuery {
    channel_id: Option<String>,
}

fn channel_param(q: &ChannelQuery) -> Result<Option<Id>> {
    match q.channel_id.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => s
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0 && *n <= i64::MAX as u64)
            .map(|n| Some(Id(n)))
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest)),
    }
}

async fn catalog(
    State(app): State<App>,
    Query(q): Query<ChannelQuery>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let ch = channel_param(&q)?.ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    let channel = db::access(&mut c, u, ch, Permissions::VIEW_CHANNEL).await?;
    let (activities, can_launch) = match (channel.summary.kind, channel.summary.guild_id) {
        (ChannelKind::Direct | ChannelKind::GroupDm, None) => {
            (for_guild(&mut c, None).await?, true)
        }
        (ChannelKind::Voice, Some(g)) => {
            let p = crate::permissions::evaluate(&mut c, u, g, Some(ch)).await?;
            (
                for_guild(&mut c, Some(g)).await?,
                p.contains(Permissions::USE_ACTIVITIES),
            )
        }
        // Stages and text channels have no activities.
        _ => (vec![], false),
    };
    let results = if channel_readable(&mut c, u, ch).await {
        sqlx::query_as::<_, (String, String, i64)>(
            "SELECT name,text,ended_at FROM activity_results WHERE channel_id=? ORDER BY id DESC LIMIT 5",
        )
        .bind(db::id(ch)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|(name, text, ended_at)| ActivityResult {
            name,
            text,
            ended_at: ended_at as u64,
        })
        .collect()
    } else {
        vec![]
    };
    Ok(crate::callextras::js(&ActivityCatalog {
        activities: activities
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(a, _)| a)
            .collect(),
        can_launch,
        results,
    }))
}

async fn channel_readable(c: &mut SqliteConnection, u: Id, ch: Id) -> bool {
    db::access(
        c,
        u,
        ch,
        Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
    )
    .await
    .is_ok()
}

/// Audio and video to pick from: the newest in one conversation you can read,
/// or (without `channel_id`) the newest you sent anywhere you can still read.
async fn media(
    State(app): State<App>,
    Query(q): Query<ChannelQuery>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let items = match channel_param(&q)? {
        Some(ch) => {
            db::text(
                &mut c,
                u,
                ch,
                Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
            )
            .await?;
            sqlx::query("SELECT up.id,up.filename,up.content_type,up.byte_size,m.channel_id,m.created_at FROM messages m JOIN message_attachments ma ON ma.message_id=m.id JOIN uploads up ON up.id=ma.attachment_id WHERE m.channel_id=? AND m.deleted_at IS NULL AND (up.content_type LIKE 'video/%' OR up.content_type LIKE 'audio/%') ORDER BY m.id DESC LIMIT 30")
                .bind(db::id(ch)?)
                .fetch_all(&mut *c)
                .await?
                .iter()
                .map(media_row)
                .collect::<Vec<_>>()
        }
        None => {
            let rows = sqlx::query("SELECT up.id,up.filename,up.content_type,up.byte_size,m.channel_id,m.created_at FROM uploads up JOIN message_attachments ma ON ma.attachment_id=up.id JOIN messages m ON m.id=ma.message_id WHERE up.owner_id=? AND m.deleted_at IS NULL AND (up.content_type LIKE 'video/%' OR up.content_type LIKE 'audio/%') ORDER BY m.id DESC LIMIT 30")
                .bind(db::id(u)?)
                .fetch_all(&mut *c)
                .await?;
            let mut out = vec![];
            for r in &rows {
                let item = media_row(r);
                if channel_readable(&mut c, u, item.channel_id).await {
                    out.push(item);
                }
            }
            out
        }
    };
    Ok(crate::callextras::js(&ActivityMediaList { items }))
}

async fn manager(app: &App, h: &HeaderMap, g: &str) -> Result<(Id, Id)> {
    let g = Id(g
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && *n <= i64::MAX as u64)
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?);
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, http::bearer(h)?).await?;
    let p = crate::permissions::evaluate(&mut c, u, g, None)
        .await
        .map_err(|_| Error::new(ErrorCode::NotFound))?;
    require(
        p.intersects(Permissions::MANAGE_GUILD | Permissions::ADMINISTRATOR),
        ErrorCode::Forbidden,
    )?;
    Ok((g, u))
}

async fn guild_list(
    State(app): State<App>,
    Path(g): Path<String>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (g, u) = manager(&app, &h, &g).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("rest:{}", u.0), 120., 30.)?;
    let mut c = app.0.pool.acquire().await?;
    let activities = for_guild(&mut c, Some(g))
        .await?
        .into_iter()
        .map(|(activity, enabled)| GuildActivity { activity, enabled })
        .collect();
    Ok(crate::callextras::js(&GuildActivities { activities }))
}

async fn guild_toggle(
    State(app): State<App>,
    Path((g, activity)): Path<(String, String)>,
    r: Request<Body>,
) -> Result<Response> {
    let (g, u) = manager(&app, r.headers(), &g).await?;
    let toggle: ActivityToggle = http::parse(r).await?;
    let mut core = app.0.core.lock().await;
    core.rate(format!("activity-settings:{}", u.0), 30., 10.)?;
    let mut c = app.0.pool.acquire().await?;
    let (info, was) = for_guild(&mut c, Some(g))
        .await?
        .into_iter()
        .find(|(a, _)| a.id == activity)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let mut tx = sqlx::Connection::begin(&mut *c).await?;
    sqlx::query("INSERT INTO guild_activities(guild_id,activity_id,enabled) VALUES(?,?,?) ON CONFLICT(guild_id,activity_id) DO UPDATE SET enabled=excluded.enabled")
        .bind(db::id(g)?)
        .bind(&activity)
        .bind(toggle.enabled)
        .execute(&mut *tx)
        .await?;
    // Which activities members can start is a server setting: audited.
    if was != toggle.enabled {
        let state = |on: bool| Some(if on { "on" } else { "off" }.to_owned());
        crate::moderation::audit::write(
            &mut tx,
            u,
            crate::moderation::audit::Entry::new(
                g,
                AuditAction::GuildUpdate,
                AuditTarget::Guild,
                Some(g),
            )
            .change(
                &format!("activity {}", info.name),
                state(was),
                state(toggle.enabled),
            ),
        )
        .await?;
    }
    tx.commit().await?;
    if !toggle.enabled {
        super::disabled(&mut core, &mut c, g, &activity, u).await?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---- the instance administrator's CLI ----------------------------------------

/// `sticktalk-server admin activities ...`: register activities by URL.
#[derive(Subcommand, Clone, Debug)]
pub enum ActivityAdmin {
    /// Registered activities.
    List,
    /// Register an activity. Its frame loads `url` (HTTPS) in a sandbox; only
    /// that origin is allowed in the activity's frame, nowhere else.
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        url: String,
        /// An emoji, or an icon name of the app's set.
        #[arg(long, default_value = "🎲")]
        icon: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long, default_value_t = 1)]
        min_players: u8,
        #[arg(long, default_value_t = 16)]
        max_players: u8,
        /// Also offer it in direct and group calls (servers choose for themselves).
        #[arg(long)]
        in_dms: bool,
    },
    /// Remove a registered activity (`custom-<n>` or `<n>`).
    Remove { id: String },
}

/// The frame URL and its origin: HTTPS, no credentials or fragment. Plain
/// HTTP is accepted only for loopback test servers.
pub(crate) fn frame_url(raw: &str) -> Result<(String, String)> {
    let u = reqwest::Url::parse(raw.trim()).map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let loopback = u
        .host_str()
        .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "[::1]");
    require(
        (u.scheme() == "https" || (u.scheme() == "http" && loopback))
            && u.host_str().is_some_and(|h| !h.contains('*'))
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
            && raw.len() <= 512,
        ErrorCode::InvalidRequest,
    )?;
    Ok((u.to_string(), u.origin().ascii_serialization()))
}

fn clean(s: &str, max: usize, empty: bool) -> Result<String> {
    let s = s.trim();
    require(
        (empty || !s.is_empty()) && s.chars().count() <= max && !s.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )?;
    Ok(s.to_owned())
}

pub async fn run_admin(c: &mut SqliteConnection, command: &ActivityAdmin) -> Result<()> {
    match command {
        ActivityAdmin::List => {
            let rows = sqlx::query("SELECT * FROM custom_activities ORDER BY id")
                .fetch_all(&mut *c)
                .await?;
            if rows.is_empty() {
                println!("No registered activities.");
            }
            for r in &rows {
                let a = custom_row(r);
                println!(
                    "{}\t{}\t{}\t{}-{} players{}",
                    a.id,
                    a.name,
                    r.get::<String, _>("url"),
                    a.min_players,
                    a.max_players,
                    if r.get::<i64, _>("in_dms") != 0 {
                        "\tdirect calls"
                    } else {
                        ""
                    }
                );
            }
        }
        ActivityAdmin::Add {
            name,
            url,
            icon,
            description,
            min_players,
            max_players,
            in_dms,
        } => {
            let name = clean(name, 40, false)?;
            let description = clean(description, 200, true)?;
            require(valid_activity_icon(icon.trim()), ErrorCode::InvalidRequest)?;
            require(
                (1..=MAX_ACTIVITY_PARTICIPANTS as u8).contains(min_players)
                    && (*min_players..=MAX_ACTIVITY_PARTICIPANTS as u8).contains(max_players),
                ErrorCode::InvalidRequest,
            )?;
            let (url, origin) = frame_url(url)?;
            let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM custom_activities")
                .fetch_one(&mut *c)
                .await?;
            require(n < MAX_CUSTOM_ACTIVITIES as i64, ErrorCode::LimitExceeded)?;
            let id: i64 = sqlx::query_scalar("INSERT INTO custom_activities(name,description,icon,url,origin,min_players,max_players,in_dms,created_at) VALUES(?,?,?,?,?,?,?,?,?) RETURNING id")
                .bind(&name)
                .bind(&description)
                .bind(icon.trim())
                .bind(&url)
                .bind(&origin)
                .bind(i64::from(*min_players))
                .bind(i64::from(*max_players))
                .bind(*in_dms)
                .bind(now() as i64)
                .fetch_one(&mut *c)
                .await?;
            println!("Registered custom-{id}: {name} ({origin}). Servers turn it on in Server Settings › Activities.");
        }
        ActivityAdmin::Remove { id } => {
            let n: i64 = id
                .trim_start_matches("custom-")
                .parse()
                .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
            let gone = sqlx::query("DELETE FROM custom_activities WHERE id=?")
                .bind(n)
                .execute(&mut *c)
                .await?
                .rows_affected();
            sqlx::query("DELETE FROM guild_activities WHERE activity_id=?")
                .bind(format!("custom-{n}"))
                .execute(&mut *c)
                .await?;
            require(gone == 1, ErrorCode::NotFound)?;
            println!("Removed custom-{n}. Sessions already running end with their calls.");
        }
    }
    Ok(())
}
