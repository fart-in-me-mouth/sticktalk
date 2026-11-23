//! Privacy and notifications in the web app: the status menu and custom
//! status, friend nicknames and notes, ignoring people, message requests,
//! notification settings, reports and the app's alerts (sound and desktop
//! notification). Views are in `ui/privacy.rs`; sounds, desktop notifications
//! and the unread badge are in `public/alerts.js`.
//!
//! Nothing here polls. Settings arrive after every Ready and stay current
//! through `Event::Privacy`; the server decides which messages alert.
use super::*;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::BTreeSet;
use sticktalk_client_core::http::decode_json;

// Sounds, desktop notifications and the unread badge. Nothing is created
// until an alert plays; tests replace it with `globalThis.__sticktalkAlert`.
#[wasm_bindgen(raw_module = "/public/alerts.js")]
extern "C" {
    #[wasm_bindgen(js_name = alert)]
    fn js_alert(kind: &str, title: &str, body: &str, tag: &str, desktop: bool, sound: bool);
    #[wasm_bindgen(js_name = badge)]
    fn js_badge(count: u32);
}

const MAX_NOTES: usize = 200;
const MAX_REVEALED: usize = 500;
/// A resumed stream replays events: older alerts are not played again.
const ALERT_FRESH_MS: u64 = 10_000;

thread_local! {
    static NICKNAMES: RefCell<BTreeMap<Id, String>> = RefCell::default();
    static IGNORED: RefCell<BTreeSet<Id>> = RefCell::default();
}
/// Your private nickname for a friend.
pub(super) fn nickname(u: Id) -> Option<String> {
    NICKNAMES.with(|n| n.borrow().get(&u).cloned())
}
pub(super) fn ignored(u: Id) -> bool {
    IGNORED.with(|i| i.borrow().contains(&u))
}

#[derive(Default)]
pub(super) struct State {
    pub settings: Option<PrivacySettings>,
    pub requests: Option<Result<MessageRequests, String>>,
    /// Your notes about people whose profiles you opened.
    pub notes: BTreeMap<Id, UserNote>,
    pub reports: Option<Result<Vec<MyReport>, String>>,
    /// The server Reports queue being looked at, and whether it shows closed ones.
    pub queue: Option<(Id, Result<ReportPage, String>)>,
    pub queue_closed: bool,
    /// Open reports per server, for moderators (from `GuildReports`).
    pub open_reports: BTreeMap<Id, u32>,
    /// Messages from ignored people that were revealed with a click.
    pub revealed: BTreeSet<Id>,
    /// What the report dialog is about.
    pub report: Option<ReportTarget>,
    pub busy: bool,
    last_badge: std::cell::Cell<Option<u32>>,
}

fn now() -> u64 {
    Browser.unix_ms()
}
fn fail(app: &Shared, e: String) {
    let mut a = app.borrow_mut();
    a.error = e;
    a.touch();
}
fn read(id: &str) -> String {
    document()
        .get_element_by_id(id)
        .and_then(|e| {
            e.dyn_ref::<web_sys::HtmlInputElement>()
                .map(|i| i.value())
                .or_else(|| e.dyn_ref::<web_sys::HtmlSelectElement>().map(|s| s.value()))
                .or_else(|| {
                    e.dyn_ref::<web_sys::HtmlTextAreaElement>()
                        .map(|t| t.value())
                })
        })
        .unwrap_or_default()
}
fn checked(id: &str) -> bool {
    document()
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|i| i.checked())
}
fn radio(name: &str) -> String {
    document()
        .query_selector(&format!("input[name='{name}']:checked"))
        .ok()
        .flatten()
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|e| e.value())
        .unwrap_or_default()
}
fn privacy(a: &mut App, cmd: PrivacyCommand) {
    a.command(Command::Privacy(cmd));
}
async fn call<T: DeserializeOwned>(
    http: &Http<CookieFetch>,
    method: &'static str,
    path: &str,
    body: Option<&impl Serialize>,
) -> Result<T, String> {
    let (body, headers) = match body {
        Some(b) => (
            Body::Bytes(serde_json::to_vec(b).map_err(|_| "Could not encode request")?),
            vec![("Content-Type".into(), "application/json".into())],
        ),
        None => (Body::Empty, vec![]),
    };
    decode_json(
        http.execute(method, &format!("/api/v1{path}"), body, headers, 30_000)
            .await?,
    )
}
/// Refresh one of this feature's open pages, in its page module (an event
/// keeps the state current; the page's own requests are the page's).
fn page(a: &App, what: &'static str, arg: String) {
    if let Some(app) = a.owner.upgrade() {
        features::defer(
            app,
            features::Feature::Privacy,
            features::Call::Other(what, arg),
        );
    }
}
fn later(a: &App, f: impl FnOnce(Shared) + 'static) {
    let owner = a.owner.clone();
    spawn_local(async move {
        if let Some(app) = owner.upgrade() {
            f(app);
        }
    });
}

/// After every Ready: opt in to privacy events; the answer carries the
/// settings (`answered`), so no separate request is made.
pub(super) fn connected(app: Shared) {
    let mut a = app.borrow_mut();
    privacy(&mut a, PrivacyCommand::Subscribe);
}
fn apply(a: &mut App, s: PrivacySettings) {
    let names: BTreeMap<Id, String> = s
        .nicknames
        .iter()
        .map(|n| (n.user_id, n.nickname.clone()))
        .collect();
    NICKNAMES.with(|n| *n.borrow_mut() = names.clone());
    a.model.nicknames = names;
    IGNORED.with(|i| *i.borrow_mut() = s.ignored.iter().map(|u| u.id).collect());
    a.privacy.settings = Some(s);
    badge(a);
}
/// Signed out: forget everything private.
pub(super) fn reset() {
    NICKNAMES.with(|n| n.borrow_mut().clear());
    IGNORED.with(|i| i.borrow_mut().clear());
}

pub(super) fn event(a: &mut App, e: &PrivacyEvent) {
    match e {
        PrivacyEvent::Settings(s) => apply(a, (**s).clone()),
        PrivacyEvent::Alert(alert) => play(a, alert),
        PrivacyEvent::Requests(c) => {
            if let Some(s) = a.privacy.settings.as_mut() {
                s.requests = *c;
            }
            if a.dialog == "privacy-requests" {
                page(a, "requests", String::new());
            }
        }
        PrivacyEvent::GuildReports { guild_id, open } => {
            a.privacy.open_reports.insert(*guild_id, *open);
            if a.dialog == "server-reports" && a.guild == Some(*guild_id) {
                let g = *guild_id;
                page(a, "queue", g.0.to_string());
            }
        }
        PrivacyEvent::ReportUpdated(r) => {
            if let Some(Ok(list)) = a.privacy.reports.as_mut() {
                if let Some(old) = list.iter_mut().find(|x| x.id == r.id) {
                    *old = r.clone();
                }
            }
        }
        PrivacyEvent::Note(n) => {
            remember_note(a, n.clone());
        }
    }
}
fn remember_note(a: &mut App, n: UserNote) {
    if a.privacy.notes.len() >= MAX_NOTES && !a.privacy.notes.contains_key(&n.user_id) {
        a.privacy.notes.clear();
    }
    a.privacy.notes.insert(n.user_id, n);
}

/// The status you chose (Online when unknown).
pub(super) fn status(a: &App) -> PresenceStatus {
    a.privacy
        .settings
        .as_ref()
        .map(|s| s.status.status)
        .or_else(|| a.model.me.as_ref().map(|m| m.desired_presence))
        .unwrap_or(PresenceStatus::Online)
}
/// What your own presence dot shows (the automatic idle included).
pub(super) fn shown_status(a: &App) -> PresenceStatus {
    match (
        status(a),
        a.privacy.settings.as_ref().is_some_and(|s| s.auto_idle),
    ) {
        (PresenceStatus::Online, true) => PresenceStatus::Idle,
        (s, _) => s,
    }
}
pub(super) fn global(a: &App) -> GlobalNotifications {
    a.privacy
        .settings
        .as_ref()
        .map(|s| s.global)
        .unwrap_or_default()
}
/// The added notification controls for one scope.
pub(super) fn extra(a: &App, guild: Option<Id>, channel: Option<Id>) -> Option<&NotificationExtra> {
    a.privacy.settings.as_ref().and_then(|s| {
        s.notification_extras
            .iter()
            .find(|x| x.guild_id == guild && x.channel_id == channel)
    })
}
fn active(n: &NotificationSettings) -> bool {
    n.muted && n.mute_until.is_none_or(|t| t > now())
}
/// The channels a conversation is nested in, outermost first, then itself.
fn lineage(a: &App, c: &ChannelSummary) -> Vec<Id> {
    let mut chain = vec![c.id];
    let mut parent = c.parent_id;
    while let Some(p) = parent.filter(|p| !chain.contains(p)) {
        chain.push(p);
        parent = a.model.channels.get(&p).and_then(|x| x.parent_id);
        if chain.len() > 64 {
            break;
        }
    }
    chain.reverse();
    chain
}
/// Muted here or anywhere above (the server, a channel it is inside).
pub(super) fn muted(a: &App, c: &ChannelSummary) -> bool {
    let n = &a.model.settings.notifications;
    if c.guild_id.is_some() && n.get(&(c.guild_id, None)).is_some_and(active) {
        return true;
    }
    lineage(a, c)
        .iter()
        .any(|ch| n.get(&(c.guild_id, Some(*ch))).is_some_and(active))
}
/// A muted channel the server's "Hide muted channels" leaves out of the list.
pub(super) fn hidden(a: &App, c: &ChannelSummary) -> bool {
    c.guild_id.is_some()
        && a.channel != Some(c.id)
        && a.model
            .settings
            .notifications
            .get(&(c.guild_id, None))
            .is_some_and(|g| g.hide_muted_channels)
        && muted(a, c)
}

/// The window title and app icon count, as the badge setting asks.
pub(super) fn badge(a: &App) {
    let mode = global(a).badge;
    let mut count = 0u32;
    if mode != BadgeMode::Off {
        for c in a.model.channels.values() {
            let Some(r) = a.model.reads.get(&c.id) else {
                continue;
            };
            let direct = c.guild_id.is_none();
            let quiet = muted(a, c);
            count = count.saturating_add(match mode {
                BadgeMode::Mentions if direct && !quiet => r.unread_count.min(1),
                BadgeMode::Mentions => r.mention_count.min(1),
                BadgeMode::All if !quiet => u32::from(r.unread_count > 0),
                _ => 0,
            });
        }
    }
    if a.privacy.last_badge.replace(Some(count)) != Some(count) {
        js_badge(count);
    }
}

/// A message or reaction the server says should alert. Played only when it is
/// fresh, not in Do Not Disturb, and not while you are looking at it.
fn play(a: &App, alert: &Alert) {
    if now().saturating_sub(alert.at) > ALERT_FRESH_MS || status(a) == PresenceStatus::Dnd {
        return;
    }
    let focused = !document().hidden() && document().has_focus().unwrap_or(false);
    if focused && a.channel == Some(alert.channel_id) && alert.kind != AlertKind::Reaction {
        return;
    }
    let g = global(a);
    let sound = g.sounds
        && match alert.kind {
            AlertKind::Message => g.sound_message,
            AlertKind::Mention => g.sound_mention,
            AlertKind::Direct => g.sound_direct,
            AlertKind::Reaction => g.sound_reaction,
        };
    let desktop = g.desktop && !focused;
    if !sound && !desktop {
        return;
    }
    let who = a.model.name(alert.user_id);
    let place = a
        .model
        .channels
        .get(&alert.channel_id)
        .map(|c| match c.guild_id {
            Some(_) => format!(" (#{})", a.model.channel_name(c)),
            None if c.kind == ChannelKind::GroupDm => format!(" ({})", a.model.channel_name(c)),
            None => String::new(),
        })
        .unwrap_or_default();
    let (title, body) = match alert.kind {
        AlertKind::Reaction => (
            format!("{who}{place}"),
            format!(
                "Reacted {} to your message",
                alert.emoji.clone().unwrap_or_default()
            ),
        ),
        _ => {
            let text = a
                .model
                .messages
                .get(&alert.channel_id)
                .and_then(|m| m.get(&alert.message_id))
                .map(|m| crate::wire::snippet(&a.model.content(m)))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "New message".into());
            (format!("{who}{place}"), text)
        }
    };
    let kind = match alert.kind {
        AlertKind::Message => "message",
        AlertKind::Mention => "mention",
        AlertKind::Direct => "direct",
        AlertKind::Reaction => "reaction",
    };
    js_alert(
        kind,
        &title,
        &body,
        &alert.channel_id.0.to_string(),
        desktop,
        sound,
    );
}

/// The duration choices, in minutes (0: until changed).
pub(super) const DURATIONS: [(u64, &str); 6] = [
    (30, "For 30 minutes"),
    (60, "For 1 hour"),
    (180, "For 3 hours"),
    (480, "For 8 hours"),
    (1440, "For 24 hours"),
    (0, "Until I change it"),
];
pub(super) const MUTES: [(&str, &str); 6] = [
    ("15", "For 15 minutes"),
    ("60", "For 1 hour"),
    ("180", "For 3 hours"),
    ("480", "For 8 hours"),
    ("1440", "For 24 hours"),
    ("forever", "Until I turn it back on"),
];
fn status_of(s: &str) -> Option<PresenceStatus> {
    Some(match s {
        "online" => PresenceStatus::Online,
        "idle" => PresenceStatus::Idle,
        "dnd" => PresenceStatus::Dnd,
        "invisible" => PresenceStatus::Invisible,
        _ => return None,
    })
}
/// The end of today, or of this week (Sunday night), in local time.
fn end_of(days_ahead: u32) -> u64 {
    let d = js_sys::Date::new(&JsValue::from_f64(now() as f64));
    d.set_hours(23);
    d.set_minutes(59);
    d.set_seconds(59);
    d.set_date(d.get_date() + days_ahead);
    d.get_time() as u64
}

/// `privacy:<what>,<args>` actions from the views.
pub(super) fn action(app: Shared, arg: &str) {
    let mut parts = arg.split(',');
    let what = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    let first = rest.first().copied().unwrap_or_default();
    let second = rest.get(1).copied().unwrap_or_default();
    match what {
        "status" => {
            let Some(status) = status_of(first) else {
                return;
            };
            let minutes = read("p-status-for").parse::<u64>().unwrap_or(0);
            let until =
                (status != PresenceStatus::Online && minutes > 0).then(|| now() + minutes * 60_000);
            let mut a = app.borrow_mut();
            privacy(&mut a, PrivacyCommand::SetStatus { status, until });
            a.dialog.clear();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
            a.touch();
        }
        "pick-emoji" => {
            if let Some(input) = document()
                .get_element_by_id("p-status-emoji")
                .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
            {
                input.set_value(first);
            }
        }
        "custom-clear" => {
            let mut a = app.borrow_mut();
            privacy(
                &mut a,
                PrivacyCommand::SetCustomStatus(CustomStatus::default()),
            );
        }
        "ignore" | "unignore" => {
            if let Some(user_id) = id(first) {
                let mut a = app.borrow_mut();
                privacy(
                    &mut a,
                    if what == "ignore" {
                        PrivacyCommand::Ignore { user_id }
                    } else {
                        PrivacyCommand::Unignore { user_id }
                    },
                );
                a.touch();
            }
        }
        "reveal" => {
            if let Some(m) = id(first) {
                let mut a = app.borrow_mut();
                if a.privacy.revealed.len() >= MAX_REVEALED {
                    a.privacy.revealed.clear();
                }
                a.privacy.revealed.insert(m);
                a.touch();
            }
        }
        "accept" | "decline" => {
            if let Some(channel_id) = id(first) {
                let mut a = app.borrow_mut();
                privacy(
                    &mut a,
                    if what == "accept" {
                        PrivacyCommand::AcceptRequest { channel_id }
                    } else {
                        PrivacyCommand::DeclineRequest { channel_id }
                    },
                );
                if let Some(Ok(r)) = a.privacy.requests.as_mut() {
                    r.requests.retain(|x| x.channel_id != channel_id);
                    r.spam.retain(|x| x.channel_id != channel_id);
                }
                a.touch();
            }
        }
        // Block and report: the request goes, the person is blocked, and reported.
        "block-report" => {
            if let (Some(channel_id), Some(user_id)) = (id(first), id(second)) {
                let mut a = app.borrow_mut();
                privacy(&mut a, PrivacyCommand::DeclineRequest { channel_id });
                a.command(Command::BlockUser { user_id });
                privacy(
                    &mut a,
                    PrivacyCommand::Report(ReportDraft {
                        target: ReportTarget::User { user_id },
                        category: ReportCategory::Spam,
                        note: Some("Reported from a message request.".into()),
                    }),
                );
                if let Some(Ok(r)) = a.privacy.requests.as_mut() {
                    r.requests.retain(|x| x.channel_id != channel_id);
                    r.spam.retain(|x| x.channel_id != channel_id);
                }
                a.notice = "Blocked and reported.".into();
                a.error = a.notice.clone();
                a.touch();
            }
        }
        // Accept, then open the conversation once it has arrived (`answered`).
        "open-request" => {
            if let Some(c) = id(first) {
                let mut a = app.borrow_mut();
                privacy(&mut a, PrivacyCommand::AcceptRequest { channel_id: c });
            }
        }
        "report" => {
            let target = match first {
                "message" => id(second).zip(rest.get(2).and_then(|v| id(v))).map(
                    |(channel_id, message_id)| ReportTarget::Message {
                        channel_id,
                        message_id,
                    },
                ),
                "user" => id(second).map(|user_id| ReportTarget::User { user_id }),
                "guild" => id(second).map(|guild_id| ReportTarget::Guild { guild_id }),
                _ => None,
            };
            if let Some(t) = target {
                app.borrow_mut().privacy.report = Some(t);
                super::operations::action(app, "dialog:privacy-report");
            }
        }
        "resolve" => {
            let (Some(report), Some(g)) = (id(first), app.borrow().guild) else {
                return;
            };
            let action = match second {
                "dismiss" => ReportAction::Dismiss,
                "delete" => ReportAction::DeleteMessage,
                "kick" => ReportAction::Kick,
                "ban" => ReportAction::Ban,
                "timeout" => ReportAction::Timeout {
                    seconds: read(&format!("p-timeout-{}", report.0))
                        .parse()
                        .unwrap_or(3600),
                },
                _ => return,
            };
            let reason = Some(read(&format!("p-reason-{}", report.0)))
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty());
            let http = app.borrow().http();
            app.borrow_mut().privacy.busy = true;
            spawn_local(async move {
                let body = ReportResolution { action, reason };
                let result: Result<Report, String> = call(
                    &http,
                    "POST",
                    &format!("/guilds/{}/reports/{}/resolve", g.0, report.0),
                    Some(&body),
                )
                .await;
                app.borrow_mut().privacy.busy = false;
                match result {
                    Ok(_) => {
                        {
                            let mut a = app.borrow_mut();
                            a.notice = "Report handled.".into();
                            a.error = a.notice.clone();
                        }
                        fetch_queue(app, g);
                    }
                    Err(e) => fail(&app, e),
                }
            });
        }
        "queue" => {
            let g = app.borrow().guild;
            if let Some(g) = g {
                app.borrow_mut().privacy.queue_closed = first == "closed";
                fetch_queue(app, g);
            }
        }
        "dm-allow" => {
            let mut a = app.borrow_mut();
            let mut dm = a
                .privacy
                .settings
                .as_ref()
                .map(|s| s.dm.clone())
                .unwrap_or_default();
            dm.allow = if first == "server_members" {
                DmAllow::ServerMembers
            } else {
                DmAllow::Friends
            };
            privacy(&mut a, PrivacyCommand::SetDmPrivacy(dm));
        }
        "dm-server" => {
            let Some(g) = id(first) else { return };
            let mut a = app.borrow_mut();
            let mut dm = a
                .privacy
                .settings
                .as_ref()
                .map(|s| s.dm.clone())
                .unwrap_or_default();
            dm.servers.retain(|o| o.guild_id != g);
            match second {
                "allow" => dm.servers.push(DmServerOverride {
                    guild_id: g,
                    allow: true,
                }),
                "block" => dm.servers.push(DmServerOverride {
                    guild_id: g,
                    allow: false,
                }),
                _ => {}
            }
            privacy(&mut a, PrivacyCommand::SetDmPrivacy(dm));
        }
        "global" => {
            let mut a = app.borrow_mut();
            let mut g = global(&a);
            let on = second == "on";
            match first {
                "desktop" => g.desktop = on,
                "sounds" => g.sounds = on,
                "sound_message" => g.sound_message = on,
                "sound_mention" => g.sound_mention = on,
                "sound_direct" => g.sound_direct = on,
                "sound_reaction" => g.sound_reaction = on,
                "badge" => {
                    g.badge = match second {
                        "all" => BadgeMode::All,
                        "off" => BadgeMode::Off,
                        _ => BadgeMode::Mentions,
                    }
                }
                "reactions" => {
                    g.reactions = match second {
                        "all" => ReactionAlerts::All,
                        "never" => ReactionAlerts::Never,
                        _ => ReactionAlerts::Direct,
                    }
                }
                _ => return,
            }
            privacy(&mut a, PrivacyCommand::SetGlobalNotifications(g));
        }
        "unmute" => {
            let target = parse_target(first, second);
            if let Some((guild, channel)) = target {
                let mut a = app.borrow_mut();
                let mut o = options(&a, guild, channel);
                o.settings.muted = false;
                o.settings.mute_until = None;
                privacy(&mut a, PrivacyCommand::SetNotifications(o));
            }
        }
        "requests" => super::operations::action(app, "dialog:privacy-requests"),
        _ => {}
    }
}
/// The wire name of a report category (the radio button's value).
pub(super) fn category_name(c: ReportCategory) -> String {
    serde_json::to_value(c)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}
fn parse_target(kind: &str, value: &str) -> Option<(Option<Id>, Option<Id>)> {
    let target = id(value)?;
    Some(if kind == "guild" {
        (Some(target), None)
    } else {
        (None, Some(target))
    })
}
/// The full current preference for a scope (defaults when there is none).
pub(super) fn options(a: &App, guild: Option<Id>, channel: Option<Id>) -> NotificationOptions {
    let guild = guild.or_else(|| {
        channel
            .and_then(|c| a.model.channels.get(&c))
            .and_then(|c| c.guild_id)
    });
    let settings = a
        .model
        .settings
        .notifications
        .get(&(guild, channel))
        .cloned()
        .unwrap_or(NotificationSettings {
            guild_id: guild,
            channel_id: channel,
            level: if guild.is_some() && channel.is_none() {
                NotificationLevel::Mentions
            } else if guild.is_none() {
                NotificationLevel::All
            } else {
                // A channel without its own setting follows the one above it.
                inherited(a, guild, channel)
            },
            muted: false,
            mute_until: None,
            suppress_everyone: false,
            hide_muted_channels: false,
        });
    let x = extra(a, guild, channel);
    NotificationOptions {
        settings,
        suppress_roles: x.is_some_and(|x| x.suppress_roles),
        mobile_push: x.is_none_or(|x| x.mobile_push),
    }
}
/// The level a channel gets from what it is nested in, or from its server.
pub(super) fn inherited(a: &App, guild: Option<Id>, channel: Option<Id>) -> NotificationLevel {
    let n = &a.model.settings.notifications;
    if let Some(c) = channel.and_then(|c| a.model.channels.get(&c)) {
        let chain = lineage(a, c);
        for p in chain.iter().rev().skip(1) {
            if let Some(s) = n.get(&(guild, Some(*p))) {
                return s.level;
            }
        }
    }
    n.get(&(guild, None))
        .map_or(NotificationLevel::Mentions, |s| s.level)
}

/// `submit:privacy-*` forms.
pub(super) fn submit(app: Shared, form: &str) {
    let mut a = app.borrow_mut();
    match form {
        "privacy-custom" => {
            let text = read("p-status-text");
            let emoji = read("p-status-emoji");
            let clear_at = match read("p-status-clear").as_str() {
                "today" => Some(end_of(0)),
                "week" => {
                    let d = js_sys::Date::new(&JsValue::from_f64(now() as f64));
                    Some(end_of((7 - d.get_day()) % 7))
                }
                "never" | "" => None,
                "custom" => {
                    let v = js_sys::Date::new(&JsValue::from_str(&read("p-status-at")));
                    let t = v.get_time();
                    if t.is_nan() || (t as u64) <= now() {
                        a.error = "Choose a time in the future.".into();
                        a.touch();
                        return;
                    }
                    Some(t as u64)
                }
                minutes => minutes.parse::<u64>().ok().map(|m| now() + m * 60_000),
            };
            privacy(
                &mut a,
                PrivacyCommand::SetCustomStatus(CustomStatus {
                    text: Some(text.trim().to_owned()).filter(|t| !t.is_empty()),
                    emoji: Some(emoji.trim().to_owned()).filter(|t| !t.is_empty()),
                    clear_at,
                }),
            );
            a.dialog.clear();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
            a.touch();
        }
        "privacy-notify" => {
            let Some((guild, channel)) = parse_target(&read("p-scope"), &read("p-target")) else {
                return;
            };
            let channel_guild = channel
                .and_then(|c| a.model.channels.get(&c))
                .and_then(|c| c.guild_id);
            let guild = guild.or(channel_guild);
            let mut o = options(&a, guild, channel);
            o.settings.level = match radio("p-level").as_str() {
                "all" => NotificationLevel::All,
                "nothing" => NotificationLevel::Nothing,
                _ => NotificationLevel::Mentions,
            };
            o.settings.suppress_everyone = checked("p-suppress-everyone");
            o.suppress_roles = checked("p-suppress-roles");
            if channel.is_none() {
                o.settings.hide_muted_channels = checked("p-hide-muted");
                o.mobile_push = checked("p-mobile-push");
            }
            match read("p-mute").as_str() {
                "" => {
                    // Unchanged: keep the mute and its deadline as they are.
                }
                "off" => {
                    o.settings.muted = false;
                    o.settings.mute_until = None;
                }
                "forever" => {
                    o.settings.muted = true;
                    o.settings.mute_until = None;
                }
                minutes => {
                    if let Ok(m) = minutes.parse::<u64>() {
                        o.settings.muted = true;
                        o.settings.mute_until = Some(now() + m * 60_000);
                    }
                }
            }
            privacy(&mut a, PrivacyCommand::SetNotifications(o));
        }
        "privacy-nickname" => {
            if let Some(user_id) = id(&read("p-nick-user")) {
                let nickname = Some(read("p-nickname").trim().to_owned()).filter(|n| !n.is_empty());
                privacy(
                    &mut a,
                    PrivacyCommand::SetFriendNickname { user_id, nickname },
                );
            }
        }
        "privacy-note" => {
            if let Some(user_id) = id(&read("p-note-user")) {
                let note = Some(read("p-note").trim().to_owned()).filter(|n| !n.is_empty());
                privacy(&mut a, PrivacyCommand::SetNote { user_id, note });
            }
        }
        "privacy-report" => {
            let Some(target) = a.privacy.report else {
                return;
            };
            let category = ReportCategory::ALL
                .iter()
                .copied()
                .find(|c| category_name(*c) == radio("p-category"))
                .unwrap_or(ReportCategory::Other);
            let note = Some(read("p-report-note").trim().to_owned()).filter(|n| !n.is_empty());
            privacy(
                &mut a,
                PrivacyCommand::Report(ReportDraft {
                    target,
                    category,
                    note,
                }),
            );
        }
        _ => {}
    }
}
/// A command of ours finished. Some forms close or confirm.
pub(super) fn answered(a: &mut App, op: &PrivacyCommand, result: &CommandResult) {
    match (op, result) {
        (PrivacyCommand::Report(_), CommandResult::Privacy(PrivacyResult::Filed(r))) => {
            a.privacy.report = None;
            if a.dialog == "privacy-report" {
                a.dialog.clear();
                a.dialog_generation = a.dialog_generation.wrapping_add(1);
            }
            if let Some(Ok(list)) = a.privacy.reports.as_mut() {
                list.insert(0, r.clone());
            }
            a.notice =
                "Thanks. Your report was sent; you can follow it in Privacy & Safety.".into();
            a.error = a.notice.clone();
        }
        (
            PrivacyCommand::AcceptRequest { channel_id },
            CommandResult::Privacy(PrivacyResult::Settings(s)),
        ) => {
            apply(a, (**s).clone());
            if a.dialog == "privacy-requests" {
                a.dialog.clear();
                a.dialog_generation = a.dialog_generation.wrapping_add(1);
                let c = *channel_id;
                later(a, move |app| {
                    super::operations::action(app, &format!("channel:{}", c.0))
                });
            }
        }
        (
            PrivacyCommand::Ignore { .. } | PrivacyCommand::Unignore { .. },
            CommandResult::Privacy(PrivacyResult::Settings(s)),
        ) => {
            apply(a, (**s).clone());
            a.notice = if matches!(op, PrivacyCommand::Ignore { .. }) {
                "Ignored. They won’t know; you can stop ignoring them in Privacy & Safety."
            } else {
                "No longer ignored."
            }
            .into();
            a.error = a.notice.clone();
        }
        (
            PrivacyCommand::SetNotifications(_),
            CommandResult::Privacy(PrivacyResult::Settings(s)),
        ) => {
            apply(a, (**s).clone());
            // Saved: a second save keeps the mute's deadline unless changed again.
            if let Some(select) = document()
                .get_element_by_id("p-mute")
                .and_then(|e| e.dyn_into::<web_sys::HtmlSelectElement>().ok())
            {
                select.set_value("");
            }
        }
        (_, CommandResult::Privacy(PrivacyResult::Settings(s))) => apply(a, (**s).clone()),
        (_, CommandResult::Privacy(PrivacyResult::Note(n))) => remember_note(a, n.clone()),
        _ => {}
    }
}
/// Error text for our commands.
pub(super) fn refused(op: &PrivacyCommand, code: ErrorCode) -> Option<&'static str> {
    Some(match (op, code) {
        (PrivacyCommand::Report(_), ErrorCode::AlreadyExists) => {
            "You already reported this. We’ll look at it."
        }
        (PrivacyCommand::Report(_), ErrorCode::RateLimited) => {
            "You’ve sent several reports recently. Try again later."
        }
        (PrivacyCommand::Report(_), ErrorCode::LimitExceeded) => {
            "You have many open reports. Wait for some to be handled."
        }
        (PrivacyCommand::SetFriendNickname { .. }, ErrorCode::Forbidden) => {
            "Nicknames are for friends."
        }
        (PrivacyCommand::SetCustomStatus(_), ErrorCode::InvalidRequest) => {
            "Use up to 128 characters and a single emoji."
        }
        (PrivacyCommand::Ignore { .. }, ErrorCode::LimitExceeded) => {
            "You can ignore up to 100 people."
        }
        (PrivacyCommand::SetNote { .. }, ErrorCode::LimitExceeded) => {
            "You have the most notes you can keep. Remove one first."
        }
        _ => return None,
    })
}

/// A dialog of ours opened: fetch what it shows.
pub(super) fn opened(app: Shared, name: &str) {
    match name {
        "privacy-requests" => fetch_requests(app),
        "privacy" => fetch_reports(app),
        "server-reports" => {
            let g = app.borrow().guild;
            if let Some(g) = g {
                app.borrow_mut().privacy.queue_closed = false;
                fetch_queue(app, g);
            }
        }
        _ => {}
    }
}
/// A profile opened: fetch your note about them.
pub(super) fn profile_opened(app: Shared, u: Id) {
    let (http, epoch, me) = {
        let a = app.borrow();
        (a.http(), a.epoch, a.model.me.as_ref().map(|m| m.user.id))
    };
    if me == Some(u) {
        return;
    }
    spawn_local(async move {
        let result = http
            .json::<Snapshot<UserNote>>(&format!("/users/{}/note", u.0))
            .await;
        let mut a = app.borrow_mut();
        if a.epoch == epoch {
            if let Ok(n) = result {
                remember_note(&mut a, n.data);
                a.touch();
            }
        }
    });
}
fn fetch_requests(app: Shared) {
    let (http, epoch) = {
        let a = app.borrow();
        (a.http(), a.epoch)
    };
    spawn_local(async move {
        let result = http
            .json::<Snapshot<MessageRequests>>("/users/@me/message-requests")
            .await
            .map(|s| s.data);
        let mut a = app.borrow_mut();
        if a.epoch == epoch {
            a.privacy.requests = Some(result);
            a.touch();
        }
    });
}
fn fetch_reports(app: Shared) {
    let (http, epoch) = {
        let a = app.borrow();
        (a.http(), a.epoch)
    };
    spawn_local(async move {
        let result = http
            .json::<Snapshot<Vec<MyReport>>>("/users/@me/reports")
            .await
            .map(|s| s.data);
        let mut a = app.borrow_mut();
        if a.epoch == epoch {
            a.privacy.reports = Some(result);
            a.touch();
        }
    });
}
fn fetch_queue(app: Shared, g: Id) {
    let (http, epoch, closed) = {
        let a = app.borrow();
        (a.http(), a.epoch, a.privacy.queue_closed)
    };
    spawn_local(async move {
        let result = http
            .json::<Snapshot<ReportPage>>(&format!(
                "/guilds/{}/reports?status={}",
                g.0,
                if closed { "closed" } else { "open" }
            ))
            .await
            .map(|s| s.data);
        let mut a = app.borrow_mut();
        if a.epoch == epoch && a.guild == Some(g) {
            if let Ok(page) = &result {
                if !closed {
                    a.privacy.open_reports.insert(g, page.reports.len() as u32);
                }
            }
            a.privacy.queue = Some((g, result));
            a.touch();
        }
    });
}

/// The privacy feature's one entry point (`features.rs`).
#[leptos::wasm_split::wasm_split(
    privacy,
    wasm_split_path = ::leptos::wasm_split,
    fallible,
    preload(prefetch)
)]
pub(super) fn run(app: Shared, call: features::Call) -> features::Outcome {
    use features::Call;
    match call {
        Call::Action(_, arg) => action(app, &arg),
        Call::Submit(form) => submit(app, &form),
        Call::Opened(name) => opened(app, &name),
        Call::Other("requests", _) => fetch_requests(app),
        Call::Other("queue", g) => {
            if let Some(g) = id(&g) {
                fetch_queue(app, g);
            }
        }
        Call::Other("profile", user) => {
            if let Some(u) = id(&user) {
                profile_opened(app, u);
            }
        }

        _ => {}
    }
    Ok(true)
}

/// The privacy feature's state: its events, the answers to its commands.
/// A separate, small split entry: keeping a feature's state current does not
/// load its pages and forms (`features.rs`).
#[leptos::wasm_split::wasm_split(
    privacy_state,
    wasm_split_path = ::leptos::wasm_split,
    fallible
)]
pub(super) fn state(app: Shared, call: features::Call) -> features::Outcome {
    use features::{Call, Reply};
    match call {
        Call::Event(raw) => {
            let Ok(e) = crate::inbound::feature::privacy_event(&raw.data) else {
                return Ok(false);
            };
            event(&mut app.borrow_mut(), &e);
        }
        Call::Answer(Some(Command::Privacy(op)), reply) => {
            let result = match reply {
                Reply::Core(result) => result,
                Reply::Raw(raw) => match crate::inbound::feature::privacy_result(&raw.data) {
                    Ok(r) => CommandResult::Privacy(r),
                    Err(()) => return Ok(false),
                },
            };
            let mut a = app.borrow_mut();
            answered(&mut a, &op, &result);
            a.touch();
        }
        _ => {}
    }
    Ok(true)
}

/// Dialogs whose opening this feature handles (`opened`).
pub(super) fn owns_dialog(name: &str) -> bool {
    matches!(name, "privacy-requests" | "privacy" | "server-reports")
}
