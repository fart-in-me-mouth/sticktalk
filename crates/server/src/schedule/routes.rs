//! REST for scheduled events: lists and details (fetched when a panel opens),
//! the calendar file, the cover image and an invite's event. Changes are
//! WebSocket commands (`ScheduleCommand`), except the cover image's bytes.
use super::*;
use crate::http;
use axum::{
    body::{to_bytes, Body},
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};

pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/guilds/{guild}/events", get(list))
        .route("/api/v1/events/{event}", get(detail))
        .route("/api/v1/events/{event}/interested", get(interested))
        .route("/api/v1/events/{event}/calendar.ics", get(calendar))
        .route(
            "/api/v1/events/{event}/cover",
            get(cover).put(set_cover).delete(remove_cover),
        )
        .route("/api/v1/invites/{code}/event", get(invite_event))
}

async fn who(app: &App, h: &HeaderMap) -> Result<(sqlx::pool::PoolConnection<sqlx::Sqlite>, Id)> {
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, http::bearer(h)?).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("rest:{}", u.0), 120., 30.)?;
    Ok((c, u))
}

async fn list(
    State(app): State<App>,
    Path(g): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    let g = Id(g);
    let p = guild_rights(&mut c, u, g)
        .await
        .map_err(|_| Error::new(ErrorCode::NotFound))?;
    let now = now();
    let mut events = vec![];
    for s in active(&mut c, g).await? {
        if visible(&mut c, u, &s).await.is_err() {
            continue;
        }
        if let Some(o) = s.current(&mut c, now).await? {
            events.push(s.wire(&o));
        }
    }
    events.sort_by_key(|e| (e.status != EventStatus::Live, e.start_at, e.id));
    let interested = sqlx::query_scalar::<_, i64>(
        "SELECT i.event_id FROM event_interest i JOIN scheduled_events e ON e.id=i.event_id WHERE i.user_id=? AND e.guild_id=?",
    )
    .bind(id(u)?)
    .bind(id(g)?)
    .fetch_all(&mut *c)
    .await?
    .into_iter()
    .map(|v| Id(v as u64))
    .collect();
    Ok(http::json(EventPage {
        events,
        interested,
        can_create: p.intersects(Permissions::MANAGE_EVENTS | Permissions::CREATE_EVENTS),
        can_manage: p.contains(Permissions::MANAGE_EVENTS),
    }))
}

#[derive(serde::Deserialize)]
struct Occ {
    occurrence: Option<u64>,
}

async fn detail(
    State(app): State<App>,
    Path(e): Path<u64>,
    Query(q): Query<Occ>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    let s = load(&mut c, Id(e)).await?;
    visible(&mut c, u, &s).await?;
    let event = match q.occurrence {
        Some(k) => {
            require(s.rule.contains(k), ErrorCode::NotFound)?;
            let o = s.occurrence(&mut c, k).await?;
            s.wire(&o)
        }
        None => s.view(&mut c, now()).await?,
    };
    let me_interested: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM event_interest WHERE event_id=? AND user_id=?)",
    )
    .bind(id(s.id)?)
    .bind(id(u)?)
    .fetch_one(&mut *c)
    .await?;
    let location_name = match event.location.channel() {
        Some(ch) => db::access(&mut c, u, ch, Permissions::VIEW_CHANNEL)
            .await
            .ok()
            .and_then(|ch| ch.summary.name),
        None => None,
    };
    let upcoming = if s.status == EventStatus::Ended || s.status == EventStatus::Cancelled {
        vec![]
    } else {
        s.rule.after(event.occurrence, 5)
    };
    Ok(http::json(EventDetail {
        can_manage: can_manage(&mut c, u, &s).await?,
        event,
        me_interested,
        location_name,
        upcoming,
    }))
}

#[derive(serde::Deserialize)]
struct After {
    after: Option<u64>,
}

const INTERESTED_PAGE: usize = 50;

async fn interested(
    State(app): State<App>,
    Path(e): Path<u64>,
    Query(q): Query<After>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    let s = load(&mut c, Id(e)).await?;
    visible(&mut c, u, &s).await?;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT user_id FROM event_interest WHERE event_id=? AND user_id>? ORDER BY user_id LIMIT ?",
    )
    .bind(id(s.id)?)
    .bind(q.after.unwrap_or(0).min(i64::MAX as u64) as i64)
    .bind(INTERESTED_PAGE as i64 + 1)
    .fetch_all(&mut *c)
    .await?;
    let more = ids.len() > INTERESTED_PAGE;
    let mut users = vec![];
    for v in ids.into_iter().take(INTERESTED_PAGE) {
        users.push(db::user(&mut c, Id(v as u64)).await?);
    }
    let next = more.then(|| users.last().map(|u| u.id)).flatten();
    Ok(http::json(InterestedPage { users, next }))
}

/// RFC 5545 text: escape, then fold lines at 75 octets.
fn ics_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\r', "")
        .replace('\n', "\\n")
}
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut width = 0;
    for ch in line.chars() {
        let n = ch.len_utf8();
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += n;
    }
    out.push_str("\r\n");
    out
}
fn utc_stamp(ms: u64) -> String {
    let ts = jiff::Timestamp::from_millisecond(ms.min(i64::MAX as u64) as i64)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    ts.strftime("%Y%m%dT%H%M%SZ").to_string()
}

/// The calendar file for an event: the whole series with its rule, in the
/// creator's time zone (by IANA name), cancelled occurrences excluded.
pub(crate) async fn ics(c: &mut SqliteConnection, s: &Series, origin: &str) -> Result<String> {
    let first = s.occurrence(c, s.rule.first_at).await?;
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        "PRODID:-//StickTalk//Events//EN".to_owned(),
        "CALSCALE:GREGORIAN".to_owned(),
        "METHOD:PUBLISH".to_owned(),
        "BEGIN:VEVENT".to_owned(),
        format!("UID:event-{}@sticktalk", s.id.0),
        format!("DTSTAMP:{}", utc_stamp(now())),
    ];
    let start = s.rule.start_local();
    let stamp = |dt: jiff::civil::DateTime| dt.strftime("%Y%m%dT%H%M%S").to_string();
    lines.push(format!("DTSTART;TZID={}:{}", s.time_zone, stamp(start)));
    if let Some(d) = s.duration {
        lines.push(format!("DURATION:PT{}S", d / 1000));
    }
    let rrule = match s.rule.recurrence() {
        EventRecurrence::None => None,
        EventRecurrence::Daily => Some("FREQ=DAILY".to_owned()),
        EventRecurrence::Weekly => Some("FREQ=WEEKLY".to_owned()),
        EventRecurrence::EveryTwoWeeks => Some("FREQ=WEEKLY;INTERVAL=2".to_owned()),
        EventRecurrence::Monthly => Some("FREQ=MONTHLY".to_owned()),
        EventRecurrence::Yearly => Some("FREQ=YEARLY".to_owned()),
        EventRecurrence::Weekdays => Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_owned()),
    };
    if let Some(mut r) = rrule {
        if let Some(until) = s.rule.until {
            r.push_str(&format!(";UNTIL={}", utc_stamp(until.saturating_sub(1000))));
        }
        lines.push(format!("RRULE:{r}"));
        let cancelled: Vec<i64> = sqlx::query_scalar(
            "SELECT occurrence FROM event_exceptions WHERE event_id=? AND cancelled=1 ORDER BY occurrence LIMIT 100",
        )
        .bind(id(s.id)?)
        .fetch_all(&mut *c)
        .await?;
        for k in cancelled {
            lines.push(format!("EXDATE:{}", utc_stamp(k as u64)));
        }
    }
    lines.push(format!("SUMMARY:{}", ics_text(&first.name)));
    if !first.description.is_empty() {
        lines.push(format!("DESCRIPTION:{}", ics_text(&first.description)));
    }
    let place = match &first.location {
        EventLocation::External { place } => place.clone(),
        EventLocation::Stage { channel_id } | EventLocation::Voice { channel_id } => {
            let name = db::channel(c, *channel_id)
                .await
                .ok()
                .and_then(|ch| ch.summary.name)
                .unwrap_or_default();
            format!("StickTalk: {name}")
        }
    };
    lines.push(format!("LOCATION:{}", ics_text(&place)));
    if !origin.is_empty() {
        lines.push(format!("URL:{origin}/?event={}", s.id.0));
    }
    if s.status == EventStatus::Cancelled {
        lines.push("STATUS:CANCELLED".to_owned());
    }
    lines.push("END:VEVENT".to_owned());
    lines.push("END:VCALENDAR".to_owned());
    Ok(lines.iter().map(|l| fold(l)).collect())
}

async fn calendar(State(app): State<App>, Path(e): Path<u64>, h: HeaderMap) -> Result<Response> {
    let (mut c, u) = who(&app, &h).await?;
    let s = load(&mut c, Id(e)).await?;
    visible(&mut c, u, &s).await?;
    // The link inside is this server's own address, as the browser reached it.
    let origin = h
        .get("host")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() <= 255 && v.bytes().all(|b| b.is_ascii_graphic()))
        .map(|host| format!("https://{host}"))
        .unwrap_or_default();
    let body = ics(&mut c, &s, &origin).await?;
    Ok((
        StatusCode::OK,
        [
            ("content-type", "text/calendar; charset=utf-8"),
            ("cache-control", "private, no-store"),
            ("x-content-type-options", "nosniff"),
            ("content-disposition", "attachment; filename=\"event.ics\""),
        ],
        body,
    )
        .into_response())
}

/// PNG, JPEG, WebP or GIF, from the bytes themselves.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else {
        None
    }
}
fn check_image(bytes: &[u8]) -> Result<()> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| Error::new(ErrorCode::UnsupportedMediaType))?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|_| Error::new(ErrorCode::UnsupportedMediaType))?;
    require(
        (16..=8192).contains(&w) && (16..=8192).contains(&h),
        ErrorCode::InvalidRequest,
    )
}

async fn cover(State(app): State<App>, Path(e): Path<u64>, h: HeaderMap) -> Result<Response> {
    let (mut c, u) = who(&app, &h).await?;
    let s = load(&mut c, Id(e)).await?;
    visible(&mut c, u, &s).await?;
    let (mime, bytes): (String, Vec<u8>) =
        sqlx::query_as("SELECT content_type,bytes FROM event_covers WHERE event_id=?")
            .bind(id(s.id)?)
            .fetch_optional(&mut *c)
            .await?
            .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok((
        StatusCode::OK,
        [
            ("content-type", mime),
            ("cache-control", "private, max-age=86400".to_owned()),
            ("x-content-type-options", "nosniff".to_owned()),
            (
                "content-security-policy",
                "default-src 'none'; sandbox".to_owned(),
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn set_cover(
    State(app): State<App>,
    Path(e): Path<u64>,
    h: HeaderMap,
    body: Body,
) -> Result<http::ApiResponse> {
    let slot = app
        .0
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let (s, u) = {
        let (mut c, u) = who(&app, &h).await?;
        app.0
            .core
            .lock()
            .await
            .rate(format!("event-cover:{}", u.0), 10., 3.)?;
        let s = load(&mut c, Id(e)).await?;
        manage(&mut c, u, &s).await?;
        (s, u)
    };
    let bytes = to_bytes(body, MAX_EVENT_COVER_BYTES)
        .await
        .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?;
    let mime = sniff(&bytes).ok_or_else(|| {
        let mut e = Error::new(ErrorCode::UnsupportedMediaType);
        e.0.message = "Use a PNG, JPEG, WebP or GIF picture.".into();
        e
    })?;
    let check = bytes.clone();
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        check_image(&check)
    })
    .await
    .map_err(|_| Error::new(ErrorCode::Unavailable))??;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    // Still theirs to change after the upload.
    let s = load(&mut c, s.id).await?;
    manage(&mut c, u, &s).await?;
    sqlx::query("INSERT INTO event_covers(event_id,content_type,bytes) VALUES(?,?,?) ON CONFLICT(event_id) DO UPDATE SET content_type=excluded.content_type,bytes=excluded.bytes")
        .bind(id(s.id)?)
        .bind(mime)
        .bind(bytes.to_vec())
        .execute(&mut *c)
        .await?;
    sqlx::query("UPDATE scheduled_events SET cover_version=cover_version+1 WHERE id=?")
        .bind(id(s.id)?)
        .execute(&mut *c)
        .await?;
    let s = load(&mut c, s.id).await?;
    let mut out = vec![];
    changed(&mut c, &s, &mut out).await?;
    for (users, event) in out {
        core.emit(&users, event)?;
    }
    Ok(http::json(s.view(&mut c, now()).await?))
}

async fn remove_cover(
    State(app): State<App>,
    Path(e): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    let mut core = app.0.core.lock().await;
    let s = load(&mut c, Id(e)).await?;
    manage(&mut c, u, &s).await?;
    sqlx::query("DELETE FROM event_covers WHERE event_id=?")
        .bind(id(s.id)?)
        .execute(&mut *c)
        .await?;
    sqlx::query("UPDATE scheduled_events SET cover_version=0 WHERE id=?")
        .bind(id(s.id)?)
        .execute(&mut *c)
        .await?;
    let s = load(&mut c, s.id).await?;
    let mut out = vec![];
    changed(&mut c, &s, &mut out).await?;
    for (users, event) in out {
        core.emit(&users, event)?;
    }
    Ok(http::json(s.view(&mut c, now()).await?))
}

/// What an invite to an event shows before anyone joins: the event's card.
/// Anyone signed in with the invite code may see it (like the invite preview).
async fn invite_event(
    State(app): State<App>,
    Path(code): Path<String>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, _) = who(&app, &h).await?;
    require(code.len() == 22, ErrorCode::NotFound)?;
    let e: i64 = sqlx::query_scalar("SELECT e.event_id FROM invite_events e JOIN invites i ON i.id=e.invite_id WHERE i.code_hash=? AND i.expires_at>? AND i.uses<i.max_uses")
        .bind(crate::state::hash(code.as_bytes()))
        .bind(now() as i64)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let s = load(&mut c, Id(e as u64)).await?;
    let view = s.view(&mut c, now()).await?;
    let location_name = match view.location.channel() {
        Some(ch) => db::channel(&mut c, ch)
            .await
            .ok()
            .and_then(|ch| ch.summary.name),
        None => None,
    };
    Ok(http::json(InviteEvent {
        status: view.status,
        event: EventCard {
            event_id: s.id,
            guild_id: s.guild,
            name: view.name,
            start_at: view.start_at,
            end_at: view.end_at,
            location: view.location,
            location_name,
            cover: view.cover,
            recurrence: view.recurrence,
        },
    }))
}
