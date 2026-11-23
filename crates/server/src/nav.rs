//! Per-account navigation: pinned conversations, Favorites, the server rail
//! (order and folders), saved messages with reminders, and the inbox (unreads
//! and mentions). Everything is private to the account that owns it, bounded,
//! and checked against what that account may read right now. Reminders use one
//! timer armed for the next one due; nothing here polls.
use crate::{
    db::*,
    error::{require, Error, Result},
    http,
    state::{now, App, Core},
};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Request},
    routing::{get, post, put},
};
use sqlx::{Connection, Row, SqliteConnection};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use sticktalk_proto::*;

/// Streams that asked for navigation events, and the reminder timer's wake-up.
#[derive(Default)]
pub(crate) struct Nav {
    /// Stream IDs (not session keys). A new stream has to opt in again.
    opted: std::sync::Mutex<HashSet<String>>,
    wake: tokio::sync::Notify,
}
const MAX_OPTED: usize = 4096;
const MAX_RAIL: usize = MAX_GUILDS_PER_USER + 10;

pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/users/@me/nav", get(snapshot))
        .route("/api/v1/users/@me/nav/pins", put(put_pins))
        .route("/api/v1/users/@me/nav/favorites", put(put_favorites))
        .route("/api/v1/users/@me/nav/rail", put(put_rail))
        .route("/api/v1/users/@me/saved", get(saved_page))
        .route("/api/v1/users/@me/saved/{id}", put(save).delete(unsave))
        .route("/api/v1/users/@me/mentions", get(mentions))
        .route("/api/v1/users/@me/unreads", get(unreads))
        .route("/api/v1/users/@me/unreads/ack", post(ack))
        .route(
            "/api/v1/users/@me/messages/search",
            get(crate::search::direct_endpoint),
        )
}

/// Navigation events go only to streams that opted in, so older clients never
/// receive an event variant they cannot decode.
fn emit(app: &App, core: &mut Core, u: Id, event: Event) -> Result<()> {
    let opted = app.0.nav.opted.lock().unwrap_or_else(|e| e.into_inner());
    core.emit_filtered(&[u], event, |s| opted.contains(&s.id))
}
fn opt_in(app: &App, core: &Core, key: &[u8]) {
    let Some(stream) = core.streams.get(key) else {
        return;
    };
    let mut opted = app.0.nav.opted.lock().unwrap_or_else(|e| e.into_inner());
    if opted.len() >= MAX_OPTED {
        let live: HashSet<&String> = core.streams.values().map(|s| &s.id).collect();
        opted.retain(|id| live.contains(id));
    }
    if opted.len() < MAX_OPTED {
        opted.insert(stream.id.clone());
    }
}
fn readable(kind: ChannelKind) -> bool {
    matches!(
        kind,
        ChannelKind::Text | ChannelKind::Direct | ChannelKind::GroupDm | ChannelKind::Announcement
    )
}
/// The conversation, if `u` can see it now. Never an error for a hidden one.
async fn visible(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<Option<Channel>> {
    match access(c, u, ch, Permissions::VIEW_CHANNEL).await {
        Ok(ch) => Ok(Some(ch)),
        Err(e) if matches!(e.0.code, ErrorCode::NotFound | ErrorCode::Forbidden) => Ok(None),
        Err(e) => Err(e),
    }
}
/// The message, if `u` may read it now (not deleted, history readable).
async fn readable_message(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    m: Id,
) -> Result<Option<Message>> {
    match text(
        c,
        u,
        ch,
        Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
    )
    .await
    {
        Ok(_) => {}
        Err(e) if e.0.code == ErrorCode::Internal || e.0.code == ErrorCode::Unavailable => {
            return Err(e)
        }
        Err(_) => return Ok(None),
    }
    match message(c, u, ch, m, false).await {
        Ok(m) => Ok(Some(m)),
        Err(e) if e.0.code == ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub(crate) async fn settings(c: &mut SqliteConnection, u: Id) -> Result<NavSettings> {
    let mut out = NavSettings::default();
    let pins: Vec<i64> =
        sqlx::query_scalar("SELECT channel_id FROM nav_pins WHERE user_id=? ORDER BY position")
            .bind(id(u)?)
            .fetch_all(&mut *c)
            .await?;
    for ch in pins {
        if visible(c, u, Id(ch as u64)).await?.is_some() {
            out.pinned.push(Id(ch as u64));
        }
    }
    out.favorite_categories = sqlx::query(
        "SELECT id,name,collapsed FROM nav_favorite_categories WHERE user_id=? ORDER BY position",
    )
    .bind(id(u)?)
    .fetch_all(&mut *c)
    .await?
    .into_iter()
    .map(|r| FavoriteCategory {
        id: r.get::<i64, _>("id") as u32,
        name: r.get("name"),
        collapsed: r.get("collapsed"),
    })
    .collect();
    let favorites = sqlx::query(
        "SELECT channel_id,category_id FROM nav_favorites WHERE user_id=? ORDER BY position",
    )
    .bind(id(u)?)
    .fetch_all(&mut *c)
    .await?;
    for r in favorites {
        let ch = Id(r.get::<i64, _>("channel_id") as u64);
        if visible(c, u, ch).await?.is_some() {
            out.favorites.push(FavoriteEntry {
                channel_id: ch,
                category_id: r.get::<Option<i64>, _>("category_id").map(|v| v as u32),
            });
        }
    }
    out.folders = crate::folders::list(c, u).await?;
    let foldered: BTreeSet<Id> = out
        .folders
        .iter()
        .flat_map(|f| f.guild_ids.iter().copied())
        .collect();
    let folder_ids: BTreeSet<u32> = out.folders.iter().map(|f| f.id).collect();
    let members: BTreeSet<i64> =
        sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE user_id=?")
            .bind(id(u)?)
            .fetch_all(&mut *c)
            .await?
            .into_iter()
            .collect();
    for r in
        sqlx::query("SELECT guild_id,folder_id FROM nav_rail WHERE user_id=? ORDER BY position")
            .bind(id(u)?)
            .fetch_all(&mut *c)
            .await?
    {
        match (
            r.get::<Option<i64>, _>("guild_id"),
            r.get::<Option<i64>, _>("folder_id"),
        ) {
            (Some(g), None) if members.contains(&g) && !foldered.contains(&Id(g as u64)) => {
                out.rail.push(RailItem::Guild(Id(g as u64)))
            }
            (None, Some(f)) if folder_ids.contains(&(f as u32)) => {
                out.rail.push(RailItem::Folder(f as u32))
            }
            _ => {}
        }
    }
    out.saved = summary(c, u).await?;
    Ok(out)
}
pub(crate) async fn summary(c: &mut SqliteConnection, u: Id) -> Result<SavedSummary> {
    let r = sqlx::query("SELECT COUNT(*) AS n, COALESCE(SUM(remind_at IS NOT NULL AND fired_at IS NULL),0) AS pending, COALESCE(SUM(remind_at IS NOT NULL AND fired_at IS NOT NULL),0) AS due FROM saved_messages WHERE user_id=?")
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
    Ok(SavedSummary {
        count: r.get::<i64, _>("n") as u32,
        pending: r.get::<i64, _>("pending") as u32,
        due: r.get::<i64, _>("due") as u32,
    })
}

/// GET /api/v1/users/@me/nav. Also opts this stream in to navigation events.
async fn snapshot(State(app): State<App>, h: HeaderMap) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let data = settings(&mut c, u).await?;
    let cursor = core.cursor(&key, u);
    opt_in(&app, &core, &key);
    Ok(http::json(Snapshot { data, cursor }))
}
/// Authenticate, rate-limit and parse one navigation write.
async fn begin_write<T: serde::de::DeserializeOwned>(
    app: &App,
    r: Request<Body>,
) -> Result<(
    tokio::sync::MutexGuard<'_, Core>,
    sqlx::pool::PoolConnection<sqlx::Sqlite>,
    Id,
    T,
)> {
    let token = http::bearer(r.headers())?.to_owned();
    let body: T = http::parse(r).await?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, &token).await?;
    core.rate(format!("nav-write:{}", u.0), 60., 20.)?;
    Ok((core, c, u, body))
}
async fn finish(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
) -> Result<http::ApiResponse> {
    let data = settings(c, u).await?;
    emit(app, core, u, Event::NavUpdated(data.clone()))?;
    Ok(http::json(data))
}
async fn put_pins(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    let (mut core, mut c, u, body) = begin_write::<SetPinned>(&app, r).await?;
    unique(&body.channel_ids, MAX_PINNED_CONVERSATIONS)?;
    for ch in &body.channel_ids {
        let channel = visible(&mut c, u, *ch)
            .await?
            .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
        require(
            channel.summary.kind != ChannelKind::Category,
            ErrorCode::InvalidRequest,
        )?;
    }
    let mut tx = c.begin().await?;
    sqlx::query("DELETE FROM nav_pins WHERE user_id=?")
        .bind(id(u)?)
        .execute(&mut *tx)
        .await?;
    for (position, ch) in body.channel_ids.iter().enumerate() {
        sqlx::query("INSERT INTO nav_pins(user_id,channel_id,position) VALUES(?,?,?)")
            .bind(id(u)?)
            .bind(id(*ch)?)
            .bind(position as i64)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    finish(&app, &mut core, &mut c, u).await
}
async fn put_favorites(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    let (mut core, mut c, u, body) = begin_write::<SetFavorites>(&app, r).await?;
    require(
        body.categories.len() <= MAX_FAVORITE_CATEGORIES && body.favorites.len() <= MAX_FAVORITES,
        ErrorCode::LimitExceeded,
    )?;
    let mut categories = BTreeSet::new();
    for k in &body.categories {
        require(
            k.id > 0 && categories.insert(k.id) && valid_category_name(&k.name),
            ErrorCode::InvalidRequest,
        )?;
    }
    let mut seen = BTreeSet::new();
    for f in &body.favorites {
        require(
            seen.insert(f.channel_id) && f.category_id.is_none_or(|k| categories.contains(&k)),
            ErrorCode::InvalidRequest,
        )?;
        let channel = visible(&mut c, u, f.channel_id)
            .await?
            .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
        require(
            channel.summary.kind != ChannelKind::Category,
            ErrorCode::InvalidRequest,
        )?;
    }
    let mut tx = c.begin().await?;
    for delete in [
        "DELETE FROM nav_favorites WHERE user_id=?",
        "DELETE FROM nav_favorite_categories WHERE user_id=?",
    ] {
        sqlx::query(delete).bind(id(u)?).execute(&mut *tx).await?;
    }
    for (position, k) in body.categories.iter().enumerate() {
        sqlx::query("INSERT INTO nav_favorite_categories(user_id,id,name,collapsed,position) VALUES(?,?,?,?,?)")
            .bind(id(u)?)
            .bind(i64::from(k.id))
            .bind(k.name.trim())
            .bind(k.collapsed)
            .bind(position as i64)
            .execute(&mut *tx)
            .await?;
    }
    for (position, f) in body.favorites.iter().enumerate() {
        sqlx::query(
            "INSERT INTO nav_favorites(user_id,channel_id,category_id,position) VALUES(?,?,?,?)",
        )
        .bind(id(u)?)
        .bind(id(f.channel_id)?)
        .bind(f.category_id.map(i64::from))
        .bind(position as i64)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    finish(&app, &mut core, &mut c, u).await
}
async fn put_rail(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    let (mut core, mut c, u, body) = begin_write::<SetRail>(&app, r).await?;
    require(body.order.len() <= MAX_RAIL, ErrorCode::LimitExceeded)?;
    let folder_ids: BTreeSet<u32> = body.folders.iter().map(|f| f.id).collect();
    let foldered: BTreeSet<Id> = body
        .folders
        .iter()
        .flat_map(|f| f.guild_ids.iter().copied())
        .collect();
    let mut seen = BTreeSet::new();
    for item in &body.order {
        require(seen.insert(*item), ErrorCode::InvalidRequest)?;
        match item {
            RailItem::Guild(g) => {
                require(!foldered.contains(g), ErrorCode::InvalidRequest)?;
                role(&mut c, u, *g).await?;
            }
            RailItem::Folder(f) => require(folder_ids.contains(f), ErrorCode::InvalidRequest)?,
        }
    }
    let mut tx = c.begin().await?;
    // Validates folders and membership of every server in them.
    let mut deliveries = vec![];
    crate::folders::set(&mut tx, u, body.folders, &mut deliveries).await?;
    sqlx::query("DELETE FROM nav_rail WHERE user_id=?")
        .bind(id(u)?)
        .execute(&mut *tx)
        .await?;
    for (position, item) in body.order.iter().enumerate() {
        let (g, f) = match item {
            RailItem::Guild(g) => (Some(id(*g)?), None),
            RailItem::Folder(f) => (None, Some(i64::from(*f))),
        };
        sqlx::query("INSERT INTO nav_rail(user_id,position,guild_id,folder_id) VALUES(?,?,?,?)")
            .bind(id(u)?)
            .bind(position as i64)
            .bind(g)
            .bind(f)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    for (users, event) in deliveries {
        core.emit(&users, event)?;
    }
    finish(&app, &mut core, &mut c, u).await
}

#[derive(serde::Deserialize)]
struct PageQuery {
    before: Option<u64>,
    limit: Option<u16>,
}
fn page_limit(limit: Option<u16>) -> Result<u16> {
    let limit = limit.unwrap_or(25);
    require((1..=50).contains(&limit), ErrorCode::InvalidRequest)?;
    Ok(limit)
}
/// One saved row as its owner may see it now.
async fn saved_row(
    c: &mut SqliteConnection,
    u: Id,
    r: &sqlx::sqlite::SqliteRow,
) -> Result<SavedMessage> {
    let ch = Id(r.get::<i64, _>("channel_id") as u64);
    let m = Id(r.get::<i64, _>("message_id") as u64);
    Ok(SavedMessage {
        id: r.get::<i64, _>("seq") as u64,
        channel_id: ch,
        message_id: m,
        saved_at: r.get::<i64, _>("saved_at") as u64,
        remind_at: r.get::<Option<i64>, _>("remind_at").map(|v| v as u64),
        due: r.get::<Option<i64>, _>("fired_at").is_some()
            && r.get::<Option<i64>, _>("remind_at").is_some(),
        message: readable_message(c, u, ch, m).await?,
    })
}
async fn saved_page(
    State(app): State<App>,
    Query(q): Query<PageQuery>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let limit = page_limit(q.limit)?;
    let before = q.before.unwrap_or(i64::MAX as u64).min(i64::MAX as u64) as i64;
    // Reminders that are due come first, then the rest, newest first.
    let rows = sqlx::query("SELECT seq,channel_id,message_id,saved_at,remind_at,fired_at FROM saved_messages WHERE user_id=? AND seq<? ORDER BY seq DESC LIMIT ?")
        .bind(id(u)?)
        .bind(before)
        .bind(i64::from(limit) + 1)
        .fetch_all(&mut *c)
        .await?;
    let has_more = rows.len() > limit as usize;
    let mut items = vec![];
    let mut users = BTreeMap::new();
    for r in rows.iter().take(limit as usize) {
        let item = saved_row(&mut c, u, r).await?;
        if let Some(m) = &item.message {
            if let std::collections::btree_map::Entry::Vacant(v) = users.entry(m.author_id) {
                v.insert(user(&mut c, m.author_id).await?);
            }
        }
        items.push(item);
    }
    let next_before = has_more.then(|| items.last().map(|i| i.id)).flatten();
    Ok(http::json(Snapshot {
        data: SavedPage {
            items,
            users: users.into_values().collect(),
            next_before,
            has_more,
            summary: summary(&mut c, u).await?,
        },
        cursor: core.cursor(&key, u),
    }))
}
async fn save(
    State(app): State<App>,
    Path(m): Path<String>,
    r: Request<Body>,
) -> Result<http::ApiResponse> {
    let m = http::url_id(&m)?;
    let (mut core, mut c, u, body) = begin_write::<SaveMessage>(&app, r).await?;
    // Only a message this account may read now can be saved.
    readable_message(&mut c, u, body.channel_id, m)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let t = now();
    if let Some(at) = body.remind_at {
        require(
            !body.clear_reminder && at + 5_000 > t && at <= t + MAX_REMINDER_AHEAD_MS,
            ErrorCode::InvalidRequest,
        )?;
    }
    let mut tx = c.begin().await?;
    let existing: Option<i64> =
        sqlx::query_scalar("SELECT seq FROM saved_messages WHERE user_id=? AND message_id=?")
            .bind(id(u)?)
            .bind(id(m)?)
            .fetch_optional(&mut *tx)
            .await?;
    if existing.is_none() {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM saved_messages WHERE user_id=?")
            .bind(id(u)?)
            .fetch_one(&mut *tx)
            .await?;
        require(n < MAX_SAVED_MESSAGES as i64, ErrorCode::LimitExceeded)?;
        sqlx::query("INSERT INTO saved_messages(user_id,message_id,channel_id,saved_at,remind_at) VALUES(?,?,?,?,?)")
            .bind(id(u)?)
            .bind(id(m)?)
            .bind(id(body.channel_id)?)
            .bind(t as i64)
            .bind(body.remind_at.map(|v| v as i64))
            .execute(&mut *tx)
            .await?;
    } else if body.remind_at.is_some() || body.clear_reminder {
        sqlx::query(
            "UPDATE saved_messages SET remind_at=?,fired_at=NULL WHERE user_id=? AND message_id=?",
        )
        .bind(body.remind_at.map(|v| v as i64))
        .bind(id(u)?)
        .bind(id(m)?)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    app.0.nav.wake.notify_one();
    let row = sqlx::query("SELECT seq,channel_id,message_id,saved_at,remind_at,fired_at FROM saved_messages WHERE user_id=? AND message_id=?")
        .bind(id(u)?)
        .bind(id(m)?)
        .fetch_one(&mut *c)
        .await?;
    let item = saved_row(&mut c, u, &row).await?;
    let s = summary(&mut c, u).await?;
    emit(&app, &mut core, u, Event::SavedUpdated(s))?;
    Ok(http::json(item))
}
#[derive(serde::Deserialize)]
struct UnsaveQuery {
    #[serde(default)]
    reminder_only: bool,
}
async fn unsave(
    State(app): State<App>,
    Path(m): Path<String>,
    Query(q): Query<UnsaveQuery>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let m = http::url_id(&m)?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("nav-write:{}", u.0), 60., 20.)?;
    let changed = if q.reminder_only {
        sqlx::query("UPDATE saved_messages SET remind_at=NULL,fired_at=NULL WHERE user_id=? AND message_id=?")
    } else {
        sqlx::query("DELETE FROM saved_messages WHERE user_id=? AND message_id=?")
    }
    .bind(id(u)?)
    .bind(id(m)?)
    .execute(&mut *c)
    .await?
    .rows_affected();
    require(changed == 1, ErrorCode::NotFound)?;
    app.0.nav.wake.notify_one();
    let s = summary(&mut c, u).await?;
    emit(&app, &mut core, u, Event::SavedUpdated(s))?;
    Ok(http::json(s))
}

/// Starts the reminder timer. It sleeps until the next reminder is due, or
/// until a reminder is set or cleared, and does nothing else.
pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            let next = match next_due(&app).await {
                Ok(next) => next,
                Err(_) => Some(now() + 60_000),
            };
            let wait = next.map(|t| t.saturating_sub(now()).min(30 * 86_400_000));
            tokio::select! {
                _ = app.0.shutdown.cancelled() => break,
                _ = app.0.nav.wake.notified() => continue,
                _ = async {
                    match wait {
                        Some(ms) => tokio::time::sleep(std::time::Duration::from_millis(ms)).await,
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    if fire_due(&app).await.is_err() {
                        // Do not spin on a failing database: try again shortly.
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        }
    });
}
async fn next_due(app: &App) -> Result<Option<u64>> {
    let mut c = app.0.pool.acquire().await?;
    Ok(sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MIN(remind_at) FROM saved_messages WHERE remind_at IS NOT NULL AND fired_at IS NULL",
    )
    .fetch_one(&mut *c)
    .await?
    .map(|v| v.max(0) as u64))
}
/// Marks every reminder that has come due, tells its owner in the app, and
/// sends a push notification when they are not looking at the app.
pub(crate) async fn fire_due(app: &App) -> Result<usize> {
    let mut pushes = vec![];
    let fired = {
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let t = now();
        let rows = sqlx::query("SELECT seq,user_id,channel_id,message_id,saved_at,remind_at,fired_at FROM saved_messages WHERE remind_at IS NOT NULL AND fired_at IS NULL AND remind_at<=? ORDER BY remind_at LIMIT 100")
            .bind(t as i64)
            .fetch_all(&mut *c)
            .await?;
        for r in &rows {
            sqlx::query("UPDATE saved_messages SET fired_at=? WHERE seq=?")
                .bind(t as i64)
                .bind(r.get::<i64, _>("seq"))
                .execute(&mut *c)
                .await?;
        }
        let mut owners = BTreeSet::new();
        for r in &rows {
            let u = Id(r.get::<i64, _>("user_id") as u64);
            let mut item = saved_row(&mut c, u, r).await?;
            item.due = true;
            if item.message.is_some() {
                pushes.push((u, item.channel_id, item.message_id));
            }
            emit(app, &mut core, u, Event::ReminderDue(item))?;
            owners.insert(u);
        }
        for u in owners {
            let s = summary(&mut c, u).await?;
            emit(app, &mut core, u, Event::SavedUpdated(s))?;
        }
        rows.len()
    };
    for (u, ch, m) in pushes {
        let _ = crate::push::reminder(app, u, ch, m).await;
    }
    Ok(fired)
}

/// Every text conversation `u` may read the history of now.
async fn readable_channels(c: &mut SqliteConnection, u: Id) -> Result<Vec<Id>> {
    let mut out = vec![];
    for ch in channels(c, u, None).await? {
        if readable(ch.kind)
            && text(
                c,
                u,
                ch.id,
                Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
            )
            .await
            .is_ok()
        {
            out.push(ch.id);
        }
    }
    // Threads of an age-restricted channel follow it (`moderation/age.rs`).
    let any_restricted: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channel_age_restricted)")
            .fetch_one(&mut *c)
            .await?;
    for t in crate::threads::readable(c, u).await? {
        if !any_restricted || !crate::moderation::age::blocked(c, u, t).await? {
            out.push(t);
        }
    }
    Ok(out)
}
#[derive(serde::Deserialize)]
struct MentionQuery {
    before: Option<Id>,
    limit: Option<u16>,
    /// Include @everyone and @here.
    #[serde(default = "yes")]
    everyone: bool,
    /// Include mentions of a role you have.
    #[serde(default = "yes")]
    roles: bool,
}
fn yes() -> bool {
    true
}
pub(crate) async fn mention_feed(
    c: &mut SqliteConnection,
    u: Id,
    before: Option<Id>,
    limit: u16,
    everyone: bool,
    roles: bool,
) -> Result<MessageSearchPage> {
    if let Some(b) = before {
        id(b)?;
    }
    let channels = readable_channels(c, u).await?;
    let empty = MessageSearchPage {
        items: vec![],
        users: vec![],
        next_before: None,
        has_more: false,
        terms: vec![],
    };
    if channels.is_empty() {
        return Ok(empty);
    }
    let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT m.id,m.channel_id FROM message_mentions mm JOIN messages m ON m.id=mm.message_id WHERE mm.user_id=",
    );
    q.push_bind(id(u)?)
        .push(" AND m.deleted_at IS NULL AND m.author_id<>")
        .push_bind(id(u)?)
        .push(" AND m.author_id NOT IN (SELECT other_id FROM relationships WHERE user_id=")
        .push_bind(id(u)?)
        .push(" AND kind='blocked') AND m.author_id NOT IN (SELECT ignored_id FROM user_ignores WHERE user_id=")
        .push_bind(id(u)?)
        .push(") AND NOT EXISTS(SELECT 1 FROM suppressed_mentions s WHERE s.message_id=m.id AND s.user_id=mm.user_id) AND m.channel_id IN (");
    {
        let mut seq = q.separated(",");
        for ch in channels {
            seq.push_bind(id(ch)?);
        }
        seq.push_unseparated(")");
    }
    if let Some(b) = before {
        q.push(" AND m.id<").push_bind(id(b)?);
    }
    // Explicit mentions always count; broadcast ones only when asked for.
    q.push(" AND (NOT EXISTS(SELECT 1 FROM message_broadcast_mentions b WHERE b.message_id=m.id AND b.user_id=mm.user_id)");
    if everyone {
        q.push(" OR EXISTS(SELECT 1 FROM message_broadcast_mentions b WHERE b.message_id=m.id AND b.user_id=mm.user_id AND b.kind IN('everyone','here'))");
    }
    if roles {
        q.push(" OR EXISTS(SELECT 1 FROM message_broadcast_mentions b WHERE b.message_id=m.id AND b.user_id=mm.user_id AND b.kind='role')");
    }
    q.push(") ORDER BY m.id DESC LIMIT ")
        .push_bind(i64::from(limit) + 1);
    let rows = q.build().fetch_all(&mut *c).await?;
    let has_more = rows.len() > limit as usize;
    let mut items = vec![];
    let mut users = BTreeMap::new();
    for r in rows.iter().take(limit as usize) {
        let m = message(
            c,
            u,
            Id(r.get::<i64, _>("channel_id") as u64),
            Id(r.get::<i64, _>("id") as u64),
            false,
        )
        .await?;
        if let std::collections::btree_map::Entry::Vacant(v) = users.entry(m.author_id) {
            v.insert(user(c, m.author_id).await?);
        }
        items.push(m);
    }
    Ok(MessageSearchPage {
        next_before: has_more.then(|| items.last().map(|m| m.id)).flatten(),
        has_more,
        items,
        users: users.into_values().collect(),
        terms: vec![],
    })
}
async fn mentions(
    State(app): State<App>,
    Query(q): Query<MentionQuery>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("inbox:{}", u.0), 60., 10.)?;
    let limit = page_limit(q.limit)?;
    let data = mention_feed(&mut c, u, q.before, limit, q.everyone, q.roles).await?;
    Ok(http::json(Snapshot {
        data,
        cursor: core.cursor(&key, u),
    }))
}
pub(crate) async fn unread_feed(c: &mut SqliteConnection, u: Id) -> Result<InboxUnreads> {
    let mut states: Vec<ReadState> = read_states(c, u)
        .await?
        .into_iter()
        .filter(|r| r.unread_count > 0)
        .collect();
    states.sort_by_key(|r| std::cmp::Reverse(r.latest_message_id));
    let has_more = states.len() > MAX_INBOX_CHANNELS;
    states.truncate(MAX_INBOX_CHANNELS);
    let mut out = vec![];
    let mut users = BTreeMap::new();
    for read in states {
        let ch = read.channel_id;
        // Muted and unconfirmed age-restricted conversations are left out.
        if crate::privacy::notify::inbox_hidden(c, u, ch).await? {
            continue;
        }
        let mut messages = vec![];
        if text(
            c,
            u,
            ch,
            Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
        )
        .await
        .is_ok()
        {
            let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM messages WHERE channel_id=? AND deleted_at IS NULL AND author_id<>? AND id>? ORDER BY id DESC LIMIT ?")
                .bind(id(ch)?)
                .bind(id(u)?)
                .bind(opt(read.last_read_id)?.unwrap_or(0))
                .bind(MAX_INBOX_MESSAGES_PER_CHANNEL as i64)
                .fetch_all(&mut *c)
                .await?;
            for m in ids {
                let m = message(c, u, ch, Id(m as u64), false).await?;
                if let std::collections::btree_map::Entry::Vacant(v) = users.entry(m.author_id) {
                    v.insert(user(c, m.author_id).await?);
                }
                messages.push(m);
            }
        }
        out.push(UnreadChannel { read, messages });
    }
    Ok(InboxUnreads {
        channels: out,
        users: users.into_values().collect(),
        has_more,
    })
}
async fn unreads(State(app): State<App>, h: HeaderMap) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("inbox:{}", u.0), 60., 10.)?;
    let data = unread_feed(&mut c, u).await?;
    Ok(http::json(Snapshot {
        data,
        cursor: core.cursor(&key, u),
    }))
}
/// Marks each channel read up to its newest message, like reading it.
async fn ack(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    let (mut core, mut c, u, body) = begin_write::<AckChannels>(&app, r).await?;
    unique(&body.channel_ids, MAX_ACK_CHANNELS)?;
    for ch in &body.channel_ids {
        text(&mut c, u, *ch, Permissions::VIEW_CHANNEL).await?;
    }
    let mut changed = vec![];
    for ch in body.channel_ids {
        let old = read(&mut c, u, ch).await?;
        let Some(latest) = old.latest_message_id else {
            continue;
        };
        if old.last_read_id.is_none_or(|v| v < latest) {
            sqlx::query("UPDATE read_states SET last_read_id=? WHERE user_id=? AND channel_id=?")
                .bind(id(latest)?)
                .bind(id(u)?)
                .bind(id(ch)?)
                .execute(&mut *c)
                .await?;
            recount(&mut c, ch).await?;
            let state = read(&mut c, u, ch).await?;
            core.emit(&[u], Event::ReadStateUpdated(state.clone()))?;
            changed.push(state);
        }
    }
    Ok(http::json(AckResult {
        read_states: changed,
    }))
}

#[cfg(test)]
mod tests;
