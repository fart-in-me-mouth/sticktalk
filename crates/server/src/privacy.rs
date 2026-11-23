//! Privacy and notifications: the chosen status (with a duration) and the
//! automatic idle, custom status with emoji and expiry, friend nicknames and
//! notes, ignoring people, who can DM you (message requests and spam),
//! notification controls and alerts, in-app reports, and the server side of
//! age-restricted channels. See `docs/features/privacy.md`.
//!
//! Hooks into shared code are one-line calls: `commands::mutate` dispatches
//! `Command::Privacy` here; `launch::filter_event` calls `gate::filter`;
//! `mentions` asks `notify::decide`; `db::presence` asks `status::shown`.
//! Everything timed (status and custom-status expiry, notification mutes, the
//! automatic idle) is handled by one task asleep until the next deadline.
pub(crate) mod gate;
pub(crate) mod notify;
pub(crate) mod people;
pub(crate) mod reports;
pub(crate) mod requests;
pub(crate) mod status;
#[cfg(test)]
mod tests;
use crate::{
    db::*,
    error::{Error, Result},
    http,
    state::{now, App, Core},
};
use axum::{
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use sqlx::SqliteConnection;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use sticktalk_proto::*;
pub(crate) type Deliveries = Vec<(Vec<Id>, Event)>;
const MAX_SUBSCRIBED: usize = 4096;

/// In-memory privacy state, owned by `Core`. Every collection is bounded by
/// the live streams or users.
#[derive(Default)]
pub(crate) struct Runtime {
    /// Event streams that opted in to `Event::Privacy`.
    pub(crate) subscribed: HashSet<String>,
    /// Wakes the deadline task when a nearer deadline is set.
    pub(crate) wake: Arc<tokio::sync::Notify>,
    /// Users online with no focused window, and since when.
    pub(crate) unfocused: HashMap<Id, u64>,
    /// Users shown as Idle because no window has been focused for a while.
    pub(crate) idle: HashSet<Id>,
}

pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/users/@me/privacy", get(snapshot))
        .route(
            "/api/v1/users/@me/message-requests",
            get(requests::endpoint),
        )
        .route("/api/v1/users/@me/reports", get(reports::mine_endpoint))
        .route("/api/v1/users/{id}/note", get(people::note_endpoint))
        .route("/api/v1/guilds/{id}/reports", get(reports::queue_endpoint))
        .route(
            "/api/v1/guilds/{id}/reports/{report}/resolve",
            post(reports::resolve_endpoint),
        )
}

/// Everything here that belongs to `u`, as of now.
pub(crate) async fn settings(
    c: &mut SqliteConnection,
    core: &Core,
    u: Id,
) -> Result<PrivacySettings> {
    Ok(PrivacySettings {
        status: status::choice(c, u).await?,
        auto_idle: core.privacy.idle.contains(&u),
        custom: status::custom(c, u).await?,
        nicknames: people::nicknames(c, u).await?,
        ignored: people::ignored(c, u).await?,
        dm: requests::dm_privacy(c, u).await?,
        global: notify::global(c, u).await?,
        notification_extras: notify::extras(c, u).await?,
        requests: requests::counts(c, u).await?,
    })
}
/// Queue the account's settings for its other devices.
pub(crate) async fn settings_event(
    c: &mut SqliteConnection,
    core: &Core,
    out: &mut Deliveries,
    u: Id,
) -> Result<PrivacySettings> {
    let s = settings(c, core, u).await?;
    out.push((
        vec![u],
        Event::Privacy(PrivacyEvent::Settings(Box::new(s.clone()))),
    ));
    Ok(s)
}

async fn snapshot(State(app): State<App>, h: HeaderMap) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let data = settings(&mut c, &core, u).await?;
    Ok(http::json(Snapshot {
        data,
        cursor: core.cursor(&key, u),
    }))
}

/// `Command::Privacy`, inside the command's transaction.
pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    cmd: PrivacyCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    use PrivacyCommand::*;
    let settings_result =
        |s: PrivacySettings| Ok(CommandResult::Privacy(PrivacyResult::Settings(Box::new(s))));
    match cmd {
        Subscribe => {
            let stream = core
                .streams
                .get(key)
                .map(|s| s.id.clone())
                .ok_or_else(|| Error::new(ErrorCode::Internal))?;
            let rt = &mut core.privacy;
            if rt.subscribed.len() >= MAX_SUBSCRIBED {
                let live: HashSet<&String> = core.streams.values().map(|s| &s.id).collect();
                rt.subscribed.retain(|s| live.contains(s));
            }
            if core.privacy.subscribed.len() < MAX_SUBSCRIBED {
                core.privacy.subscribed.insert(stream);
            }
            Ok(CommandResult::Ack)
        }
        SetStatus { status, until } => {
            status::set(c, core, u, status, until, out).await?;
            settings_result(settings_event(c, core, out, u).await?)
        }
        SetCustomStatus(custom) => {
            status::set_custom(c, core, u, custom, out).await?;
            settings_result(settings_event(c, core, out, u).await?)
        }
        SetFriendNickname { user_id, nickname } => {
            people::set_nickname(c, u, user_id, nickname).await?;
            settings_result(settings_event(c, core, out, u).await?)
        }
        SetNote { user_id, note } => {
            let note = people::set_note(c, u, user_id, note).await?;
            out.push((vec![u], Event::Privacy(PrivacyEvent::Note(note.clone()))));
            Ok(CommandResult::Privacy(PrivacyResult::Note(note)))
        }
        Ignore { user_id } => {
            people::ignore(c, u, user_id, true, out).await?;
            settings_result(settings_event(c, core, out, u).await?)
        }
        Unignore { user_id } => {
            people::ignore(c, u, user_id, false, out).await?;
            settings_result(settings_event(c, core, out, u).await?)
        }
        SetDmPrivacy(p) => {
            requests::set_dm_privacy(c, u, p).await?;
            settings_result(settings_event(c, core, out, u).await?)
        }
        AcceptRequest { channel_id } => {
            requests::accept(c, u, channel_id, out).await?;
            settings_result(settings_event(c, core, out, u).await?)
        }
        DeclineRequest { channel_id } => {
            requests::decline(c, u, channel_id, out).await?;
            settings_result(settings_event(c, core, out, u).await?)
        }
        SetNotifications(o) => {
            notify::set(c, core, u, o, out).await?;
            settings_result(settings_event(c, core, out, u).await?)
        }
        SetGlobalNotifications(g) => {
            notify::set_global(c, u, g).await?;
            settings_result(settings_event(c, core, out, u).await?)
        }
        Report(draft) => {
            let filed = reports::file(c, core, u, draft, out).await?;
            Ok(CommandResult::Privacy(PrivacyResult::Filed(filed)))
        }
    }
}

/// After a command committed and its events went out: alerts for new
/// messages and reactions (sound and desktop notification decisions).
pub(crate) async fn observe(app: &App, core: &mut Core, c: &mut SqliteConnection, e: &Event) {
    if core.privacy.subscribed.is_empty() {
        return;
    }
    if let Err(error) = notify::observe(app, core, c, e).await {
        tracing::debug!(?error, "privacy alert skipped");
    }
}

/// Send deliveries made outside a command (timers), through the same final
/// gate as command events.
pub(crate) async fn deliver(
    core: &mut Core,
    c: &mut SqliteConnection,
    out: Deliveries,
) -> Result<()> {
    for (users, e) in out {
        for (users, e) in crate::launch::filter_event(c, users, e).await? {
            if let Event::UserUpdated(ref profile) = e {
                let mut streams = Vec::new();
                for s in core.streams.values().filter(|s| users.contains(&s.user)) {
                    if crate::commands::profile_projection(c, s.user, profile.id, s.guild).await? {
                        streams.push(s.id.clone());
                    }
                }
                core.emit_filtered(&users, e, |s| streams.contains(&s.id))?;
            } else {
                core.emit(&users, e)?;
            }
        }
    }
    Ok(())
}

/// Expire what is due and return the next deadline.
async fn tick(app: &App) -> Result<Option<u64>> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut out: Deliveries = vec![];
    let mut presence = vec![];
    let more_status = status::expire(&mut c, &mut core, &mut out, &mut presence).await?;
    let more_mutes = notify::expire_mutes(&mut c, &mut out).await?;
    let mut idle = vec![];
    status::expire_idle(&mut core, &mut idle);
    for u in &idle {
        settings_event(&mut c, &core, &mut out, *u).await?;
    }
    presence.extend(idle);
    deliver(&mut core, &mut c, out).await?;
    presence.sort();
    presence.dedup();
    for u in presence {
        crate::commands::broadcast_presence(&mut c, &mut core, u).await?;
    }
    if more_status || more_mutes {
        return Ok(Some(now()));
    }
    let mut next = status::next(&mut c, &core).await?;
    if let Some(m) = notify::next_mute(&mut c).await? {
        next = Some(next.map_or(m, |n| n.min(m)));
    }
    Ok(next)
}
/// One task per server process, asleep until the next deadline or a nearer one.
pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let wake = app.0.core.lock().await.privacy.wake.clone();
        loop {
            let next = match tick(&app).await {
                Ok(next) => next,
                Err(error) => {
                    tracing::warn!(?error, "privacy deadlines failed");
                    Some(now() + 5_000)
                }
            };
            let wait = next.map(|t| t.saturating_sub(now()).min(30 * 86_400_000));
            tokio::select! {
                _ = app.0.shutdown.cancelled() => break,
                _ = wake.notified() => {}
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

#[derive(serde::Deserialize)]
pub(crate) struct PageQuery {
    pub before: Option<Id>,
    pub status: Option<String>,
}
