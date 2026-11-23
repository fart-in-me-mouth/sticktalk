//! Stage channels and scheduled events in the web app. Views are in
//! `ui/stage.rs`. Actions are `stage:<verb>,<args>`; forms are in `FORMS`.
//!
//! Stage state comes in the voice envelope (`StageEvent`): live Stages for
//! the channel list, and the roster of the Stage this connection is on. The
//! audience list itself is drawn by `public/stage.js` (a virtual list), fed
//! from here whenever the roster changes. Your role on the Stage drives the
//! media host: the audience opens no microphone at all.
//!
//! Events: the channel list's counts come with `Event::Schedule` (subscribed
//! after every Ready); lists and details are fetched when their panel opens.
use super::*;
use std::collections::BTreeSet;
use sticktalk_client_core::launch::permissions as channel_rights;

/// Live Stages kept for the channel list.
const MAX_LIVE: usize = 500;
/// Servers whose event summary is kept.
const MAX_SUMMARIES: usize = 256;
/// People on the Stage this connection is on (the server's listener cap and the speakers).
const MAX_ROSTER: usize = 1100;

pub(super) const FORMS: &[&str] = &["stage-start", "stage-topic", "event-form"];

/// The Stage this connection is on.
pub(super) struct Roster {
    pub channel: Id,
    pub instance: Option<StageInstance>,
    pub members: Vec<StageMember>,
}

#[derive(Default)]
pub(super) struct State {
    pub live: BTreeMap<Id, StageInstance>,
    pub roster: Option<Roster>,
    /// The role last given to the media host (None: not on a Stage).
    told: Option<StageRole>,
    /// You moved yourself onto the Stage (started it, or Become a Speaker):
    /// you start unmuted. Invited or accepted speakers start muted.
    self_promoted: bool,
    pub summaries: BTreeMap<Id, GuildEvents>,
    /// The events panel: its server and page.
    pub page: Option<(Id, EventPage)>,
    pub detail: Option<EventDetail>,
    pub interested: Option<(Id, InterestedPage)>,
    /// Events you marked Interested, as far as this client knows.
    pub mine: BTreeSet<Id>,
    /// The event being edited (None in the form: a new event).
    pub editing: Option<ScheduledEvent>,
    /// The Stage a Start dialog is for.
    pub starting: Option<Id>,
    /// The event an invite in the Join dialog leads to.
    pub invite_event: Option<(String, InviteEvent)>,
    /// The last event invite link created here.
    pub invite_link: Option<(Id, String)>,
    pub busy: bool,
}

fn me(a: &App) -> Option<Id> {
    a.model.me.as_ref().map(|m| m.user.id)
}
fn checked(id: &str) -> bool {
    document()
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|e| e.checked())
}
fn notice(a: &mut App, text: &str) {
    a.notice = text.into();
    a.error = text.into();
}
fn fail(app: &Shared, text: String) {
    let mut a = app.borrow_mut();
    a.error = text;
    a.touch();
}

/// A Stage moderator here: Manage Channels, Mute Members and Move Members.
pub(super) fn moderator(a: &App, ch: Id) -> bool {
    channel_rights(&a.model, ch).contains(
        Permissions::MANAGE_CHANNELS | Permissions::MUTE_MEMBERS | Permissions::MOVE_MEMBERS,
    )
}
pub(super) fn is_stage(a: &App, ch: Id) -> bool {
    a.model
        .channels
        .get(&ch)
        .is_some_and(|c| c.kind == ChannelKind::Stage)
}
/// The roster of `ch`, if this connection is on it.
pub(super) fn roster(a: &App, ch: Id) -> Option<&Roster> {
    a.stage.roster.as_ref().filter(|r| r.channel == ch)
}
pub(super) fn my_entry(a: &App) -> Option<&StageMember> {
    let me = me(a)?;
    a.stage
        .roster
        .as_ref()?
        .members
        .iter()
        .find(|m| m.user_id == me)
}
fn guild_rights(a: &App, g: Id) -> Permissions {
    a.model
        .guilds
        .get(&g)
        .map(|g| g.my_permissions)
        .unwrap_or_default()
}
pub(super) fn can_create_events(a: &App, g: Id) -> bool {
    guild_rights(a, g).intersects(Permissions::MANAGE_EVENTS | Permissions::CREATE_EVENTS)
}

// ---- Stage events --------------------------------------------------------------

/// A Stage event from the voice envelope.
pub(super) fn voice_event(app: &Shared, e: StageEvent) {
    let mut a = app.borrow_mut();
    let mut roster_changed = false;
    match e {
        StageEvent::Snapshot { stages } => {
            a.stage.live = stages
                .into_iter()
                .take(MAX_LIVE)
                .map(|s| (s.channel_id, s))
                .collect();
        }
        StageEvent::Live(i) => {
            if let Some(r) = a
                .stage
                .roster
                .as_mut()
                .filter(|r| r.channel == i.channel_id)
            {
                r.instance = Some(i.clone());
            }
            if a.stage.live.len() < MAX_LIVE || a.stage.live.contains_key(&i.channel_id) {
                a.stage.live.insert(i.channel_id, i);
            }
        }
        StageEvent::Ended { channel_id, .. } => {
            a.stage.live.remove(&channel_id);
            if let Some(r) = a.stage.roster.as_mut().filter(|r| r.channel == channel_id) {
                r.instance = None;
            }
            // The Stage is over for everyone on it: leave its media and screen.
            if a.voice_channel == Some(channel_id) {
                a.voice_channel = None;
                a.stage.roster = None;
                a.stage.told = None;
                media_action("transport-lost");
                a.touch();
                drop(a);
                operations::action(app.clone(), "call-ended");
                return;
            }
        }
        StageEvent::Roster(r) => {
            if a.voice_channel == Some(r.channel_id) {
                let mut members = r.members;
                members.truncate(MAX_ROSTER);
                if let Some(i) = &r.instance {
                    a.stage.live.insert(r.channel_id, i.clone());
                }
                a.stage.roster = Some(Roster {
                    channel: r.channel_id,
                    instance: r.instance,
                    members,
                });
                roster_changed = true;
            }
        }
        StageEvent::Members {
            channel_id,
            upserts,
            removed,
        } => {
            if let Some(r) = a.stage.roster.as_mut().filter(|r| r.channel == channel_id) {
                r.members.retain(|m| !removed.contains(&m.user_id));
                for m in upserts {
                    if let Some(x) = r.members.iter_mut().find(|x| x.user_id == m.user_id) {
                        *x = m;
                    } else if r.members.len() < MAX_ROSTER {
                        r.members.push(m);
                    }
                }
                // Speakers first, each group in the order it was given.
                r.members.sort_by_key(|m| m.role != StageRole::Speaker);
                roster_changed = true;
            }
        }
    }
    if roster_changed {
        sync(&mut a);
    }
    a.touch();
}

/// Tells the media host your role and the audience to draw.
fn sync(a: &mut App) {
    let role = my_entry(a).map(|m| m.role);
    if role.is_some() && role != a.stage.told {
        let first = a.stage.told.is_none();
        a.stage.told = role;
        match role {
            Some(StageRole::Speaker) => media_action(if a.stage.self_promoted || first {
                "stage-speaker"
            } else {
                "stage-speaker-muted"
            }),
            _ => {
                // Others were told this device left the speakers; so is this one.
                let me = me(a);
                let ch = a.stage.roster.as_ref().map(|r| r.channel);
                a.model
                    .launch
                    .voice
                    .retain(|s, v| Some(s.user_id) != me || Some(v.channel_id) != ch);
                media_action("stage-listener")
            }
        }
        a.stage.self_promoted = false;
    }
    let Some(r) = a.stage.roster.as_ref() else {
        return;
    };
    let me = me(a);
    let moderator = r
        .members
        .iter()
        .find(|m| Some(m.user_id) == me)
        .is_some_and(|m| m.moderator);
    let audience: Vec<serde_json::Value> = r
        .members
        .iter()
        .filter(|m| m.role == StageRole::Audience)
        .map(|m| {
            serde_json::json!({
                "id": m.user_id.0.to_string(),
                "name": m.name,
                "avatar": m.avatar_id.map(|f| format!("/api/v1/files/{}/thumbnail", f.0)),
                "hand": m.hand_raised_at.is_some(),
                "invited": m.invited,
                "moderator": m.moderator,
            })
        })
        .collect();
    let data = serde_json::json!({
        "audience": audience,
        "me": me.map(|m| m.0.to_string()),
        "moderator": moderator,
    });
    media_action(&format!("stage-roster:{data}"));
}

// ---- events (scheduled) ----------------------------------------------------------

pub(super) fn ready(app: Shared) {
    let mut a = app.borrow_mut();
    a.command(Command::Schedule(ScheduleCommand::Subscribe));
}

fn summary(a: &mut App, s: GuildEvents) {
    if s.count == 0 {
        a.stage.summaries.remove(&s.guild_id);
    } else if a.stage.summaries.len() < MAX_SUMMARIES || a.stage.summaries.contains_key(&s.guild_id)
    {
        a.stage.summaries.insert(s.guild_id, s);
    }
}

/// An `Event::Schedule`.
pub(super) fn schedule_event(a: &mut App, e: ScheduleUpdate) {
    match e {
        ScheduleUpdate::Summary(s) => summary(a, s),
        ScheduleUpdate::Upsert(e) => {
            if let Some((g, page)) = a.stage.page.as_mut() {
                if *g == e.guild_id {
                    page.events.retain(|x| x.id != e.id);
                    if matches!(e.status, EventStatus::Scheduled | EventStatus::Live) {
                        page.events.push(e.clone());
                        page.events
                            .sort_by_key(|e| (e.status != EventStatus::Live, e.start_at, e.id));
                    }
                }
            }
            if let Some(d) = a.stage.detail.as_mut().filter(|d| d.event.id == e.id) {
                d.event = e;
            }
        }
        ScheduleUpdate::Removed { event_id, .. } => {
            if let Some((_, page)) = a.stage.page.as_mut() {
                page.events.retain(|x| x.id != event_id);
            }
            if a.stage
                .detail
                .as_ref()
                .is_some_and(|d| d.event.id == event_id)
            {
                a.stage.detail = None;
                if a.dialog.starts_with("event:") {
                    a.dialog = "events".into();
                }
            }
            a.stage.mine.remove(&event_id);
        }
        ScheduleUpdate::Interest {
            event_id,
            interested,
        } => {
            if interested {
                a.stage.mine.insert(event_id);
            } else {
                a.stage.mine.remove(&event_id);
            }
            if let Some(d) = a.stage.detail.as_mut().filter(|d| d.event.id == event_id) {
                if d.me_interested != interested {
                    d.me_interested = interested;
                    d.event.interested = if interested {
                        d.event.interested.saturating_add(1)
                    } else {
                        d.event.interested.saturating_sub(1)
                    };
                }
            }
        }
        ScheduleUpdate::Starting(e) => {
            // In-app only, and never in Do Not Disturb.
            let dnd = a
                .model
                .me
                .as_ref()
                .is_some_and(|m| m.desired_presence == PresenceStatus::Dnd);
            if !dnd {
                notice(a, &format!("Starting now: {}", e.name));
            }
        }
    }
}

/// The answer to a `ScheduleCommand`.
pub(super) fn answered(app: Shared, cmd: Option<&Command>, result: ScheduleResult) {
    let mut a = app.borrow_mut();
    a.stage.busy = false;
    match result {
        ScheduleResult::Summary { guilds } => {
            a.stage.summaries.clear();
            for g in guilds {
                summary(&mut a, g);
            }
        }
        ScheduleResult::Event(e) => {
            let from_form = matches!(
                cmd,
                Some(Command::Schedule(
                    ScheduleCommand::Create(_) | ScheduleCommand::Update { .. }
                ))
            );
            schedule_event(&mut a, ScheduleUpdate::Upsert(e.clone()));
            if from_form && a.dialog == "event-form" {
                a.stage.editing = None;
                drop(a);
                open(app, e.id, Some(e.occurrence));
                return;
            }
        }
        ScheduleResult::Message(_) => notice(&mut a, "Event shared."),
        ScheduleResult::Invite(invite) => {
            if let Some(Command::Schedule(ScheduleCommand::CreateInvite { event_id, .. })) = cmd {
                let link = format!("{}/invite/{}", a.origin, invite.code);
                a.stage.invite_link = Some((*event_id, link));
            }
        }
        ScheduleResult::Ack => {}
    }
    a.touch();
}

/// Opens an event's detail (and fetches it).
fn open(app: Shared, e: Id, occurrence: Option<u64>) {
    {
        let mut a = app.borrow_mut();
        a.dialog = format!("event:{}", e.0);
        a.dialog_generation = a.dialog_generation.wrapping_add(1);
        a.stage.detail = None;
        a.stage.interested = None;
        a.error.clear();
        a.touch();
    }
    let (http, generation) = {
        let a = app.borrow();
        (a.http(), a.dialog_generation)
    };
    let path = match occurrence {
        Some(o) => format!("/events/{}?occurrence={o}", e.0),
        None => format!("/events/{}", e.0),
    };
    spawn_local(async move {
        let detail = http.json::<EventDetail>(&path).await;
        let people = http
            .json::<InterestedPage>(&format!("/events/{}/interested", e.0))
            .await;
        let mut a = app.borrow_mut();
        if a.dialog_generation != generation {
            return;
        }
        match detail {
            Ok(d) => {
                if d.me_interested {
                    a.stage.mine.insert(d.event.id);
                }
                a.stage.detail = Some(d);
                if let Ok(p) = people {
                    a.stage.interested = Some((e, p));
                }
            }
            Err(error) => a.error = error,
        }
        a.touch();
    });
}

/// Opens a server's events list (and fetches it).
fn list(app: Shared, g: Id) {
    {
        let mut a = app.borrow_mut();
        a.dialog = "events".into();
        a.dialog_generation = a.dialog_generation.wrapping_add(1);
        if a.stage.page.as_ref().is_none_or(|p| p.0 != g) {
            a.stage.page = None;
        }
        a.error.clear();
        a.touch();
    }
    let (http, generation) = {
        let a = app.borrow();
        (a.http(), a.dialog_generation)
    };
    spawn_local(async move {
        let page = http
            .json::<EventPage>(&format!("/guilds/{}/events", g.0))
            .await;
        let mut a = app.borrow_mut();
        if a.dialog_generation != generation {
            return;
        }
        match page {
            Ok(p) => {
                a.stage.mine.extend(p.interested.iter().copied());
                a.stage.page = Some((g, p));
            }
            Err(error) => a.error = error,
        }
        a.touch();
    });
}

fn schedule(a: &mut App, c: ScheduleCommand) {
    a.stage.busy = true;
    a.command(Command::Schedule(c));
}
fn stage(a: &mut App, s: StageSignal) {
    a.voice(VoiceSignal::Stage(s));
}

/// Joins a Stage: as audience (the server says otherwise for speakers).
fn join(app: Shared, ch: Id) {
    {
        let a = app.borrow();
        if a.voice_channel == Some(ch) {
            drop(a);
            operations::action(app, &format!("voice:{}", ch.0));
            return;
        }
    }
    app.borrow_mut().stage.told = None;
    media_action("stage-listener");
    operations::action(app, &format!("voice:{}", ch.0));
}

/// `stage:<verb>,<args>`.
pub(super) fn action(app: Shared, arg: &str) {
    let mut parts = arg.split(',');
    let verb = parts.next().unwrap_or_default();
    let first = parts.next().and_then(id);
    let second = parts.next().unwrap_or_default();
    let channel = app
        .borrow()
        .voice_channel
        .filter(|c| is_stage(&app.borrow(), *c));
    match verb {
        // A Stage row in the channel list.
        "open" => {
            let Some(ch) = first else { return };
            let (live, moderator) = {
                let a = app.borrow();
                (a.stage.live.contains_key(&ch), moderator(&a, ch))
            };
            if live || app.borrow().voice_channel == Some(ch) {
                join(app, ch);
            } else if moderator {
                let mut a = app.borrow_mut();
                a.stage.starting = Some(ch);
                a.dialog = "stage-start".into();
                a.dialog_generation = a.dialog_generation.wrapping_add(1);
                a.error.clear();
                a.touch();
            } else {
                let mut a = app.borrow_mut();
                notice(&mut a, "This Stage hasn’t started yet.");
                a.touch();
            }
        }
        "join" => {
            if let Some(ch) = first {
                join(app, ch);
            }
        }
        "raise" | "lower" => {
            if let Some(ch) = channel {
                stage(
                    &mut app.borrow_mut(),
                    StageSignal::RaiseHand {
                        channel_id: ch,
                        raised: verb == "raise",
                    },
                );
            }
        }
        "accept" | "dismiss" | "invite" | "audience" => {
            let (Some(ch), Some(user_id)) = (channel, first) else {
                return;
            };
            let s = match verb {
                "accept" => StageSignal::Accept {
                    channel_id: ch,
                    user_id,
                },
                "dismiss" => StageSignal::Dismiss {
                    channel_id: ch,
                    user_id,
                },
                "invite" => StageSignal::Invite {
                    channel_id: ch,
                    user_id,
                },
                _ => StageSignal::ToAudience {
                    channel_id: ch,
                    user_id,
                },
            };
            stage(&mut app.borrow_mut(), s);
        }
        "answer" => {
            if let Some(ch) = channel {
                let accept = arg.ends_with(",yes");
                stage(
                    &mut app.borrow_mut(),
                    StageSignal::AnswerInvite {
                        channel_id: ch,
                        accept,
                    },
                );
            }
        }
        "speak" => {
            if let Some(ch) = channel {
                let mut a = app.borrow_mut();
                a.stage.self_promoted = true;
                stage(&mut a, StageSignal::BecomeSpeaker { channel_id: ch });
            }
        }
        "end" => {
            if let Some(ch) = channel {
                stage(&mut app.borrow_mut(), StageSignal::End { channel_id: ch });
            }
        }
        "topic" => {
            let mut a = app.borrow_mut();
            a.dialog = "stage-topic".into();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
            a.touch();
        }
        // Events.
        "events" => {
            let g = first.or(app.borrow().guild);
            if let Some(g) = g {
                list(app, g);
            }
        }
        "event" => {
            if let Some(e) = first {
                open(app, e, second.parse().ok());
            }
        }
        "event-new" => {
            let mut a = app.borrow_mut();
            a.stage.editing = None;
            a.dialog = "event-form".into();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
            a.error.clear();
            a.touch();
        }
        "event-edit" => {
            let mut a = app.borrow_mut();
            a.stage.editing = a.stage.detail.as_ref().map(|d| d.event.clone());
            a.dialog = "event-form".into();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
            a.error.clear();
            a.touch();
        }
        "interest" => {
            if let Some(e) = first {
                let mut a = app.borrow_mut();
                let interested = second != "0";
                a.command(Command::Schedule(ScheduleCommand::SetInterested {
                    event_id: e,
                    interested,
                }));
            }
        }
        "event-start" | "event-end" | "event-delete" => {
            if let Some(e) = first {
                let mut a = app.borrow_mut();
                schedule(
                    &mut a,
                    match verb {
                        "event-start" => ScheduleCommand::Start { event_id: e },
                        "event-end" => ScheduleCommand::End { event_id: e },
                        _ => ScheduleCommand::Delete { event_id: e },
                    },
                );
                if verb == "event-delete" {
                    a.dialog = "events".into();
                    a.touch();
                }
            }
        }
        "event-cancel" => {
            let (Some(e), Ok(occurrence)) = (first, second.parse::<u64>()) else {
                return;
            };
            let scope = if arg.ends_with(",following") {
                EditScope::Following
            } else {
                EditScope::This
            };
            schedule(
                &mut app.borrow_mut(),
                ScheduleCommand::Cancel {
                    event_id: e,
                    occurrence,
                    scope,
                },
            );
        }
        "event-share" => {
            let Some(e) = first else { return };
            let channel = id(&value("event-share-channel"));
            let mut a = app.borrow_mut();
            match channel {
                Some(channel_id) => schedule(
                    &mut a,
                    ScheduleCommand::Share {
                        event_id: e,
                        channel_id,
                    },
                ),
                None => {
                    a.error = "Choose a channel to share it in.".into();
                    a.touch();
                }
            }
        }
        "event-invite" => {
            if let Some(e) = first {
                schedule(
                    &mut app.borrow_mut(),
                    ScheduleCommand::CreateInvite {
                        event_id: e,
                        ttl_seconds: 7 * 24 * 3600,
                        max_uses: 1000,
                    },
                );
            }
        }
        // Where the event happens: its Stage or voice channel.
        "event-join" => {
            let location = app
                .borrow()
                .stage
                .detail
                .as_ref()
                .map(|d| d.event.location.clone());
            let ch = first.or_else(|| location.and_then(|l| l.channel()));
            if let Some(ch) = ch {
                {
                    let mut a = app.borrow_mut();
                    a.dialog.clear();
                    a.touch();
                }
                if is_stage(&app.borrow(), ch) {
                    action(app, &format!("open,{}", ch.0));
                } else {
                    operations::action(app, &format!("voice:{}", ch.0));
                }
            }
        }
        "event-cover" => {
            if let Some(e) = first {
                upload_cover(app, e);
            }
        }
        "event-cover-remove" => {
            if let Some(e) = first {
                let http = app.borrow().http();
                spawn_local(async move {
                    let r = http
                        .execute(
                            "DELETE",
                            &format!("/api/v1/events/{}/cover", e.0),
                            Body::Empty,
                            vec![],
                            30_000,
                        )
                        .await;
                    if let Err(error) = r.and_then(|r| {
                        if r.status < 300 {
                            Ok(())
                        } else {
                            Err("The cover image could not be removed.".into())
                        }
                    }) {
                        fail(&app, error);
                    }
                });
            }
        }
        _ => {}
    }
}

/// Puts the picture chosen in the event form as the event's cover.
fn upload_cover(app: Shared, e: Id) {
    let file = document()
        .get_element_by_id("event-cover-file")
        .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
        .and_then(|el| el.files())
        .and_then(|f| f.get(0));
    let Some(file) = file else { return };
    if file.size() > MAX_EVENT_COVER_BYTES as f64 {
        fail(&app, "That picture is larger than 4 MB.".into());
        return;
    }
    let http = app.borrow().http();
    spawn_local(async move {
        let bytes = match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
            Ok(buffer) => js_sys::Uint8Array::new(&buffer).to_vec(),
            Err(_) => return fail(&app, "That picture could not be read.".into()),
        };
        let r = http
            .execute(
                "PUT",
                &format!("/api/v1/events/{}/cover", e.0),
                Body::Bytes(bytes),
                vec![("Content-Type".into(), "application/octet-stream".into())],
                60_000,
            )
            .await;
        match r {
            Ok(r) if r.status < 300 => {
                let mut a = app.borrow_mut();
                notice(&mut a, "Cover image saved.");
                a.touch();
            }
            _ => fail(
                &app,
                "Use a PNG, JPEG, WebP or GIF picture of at most 4 MB.".into(),
            ),
        }
    });
}

/// Local midnight-free "YYYY-MM-DDTHH:MM" for a datetime-local input.
pub(super) fn local_input(ms: u64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(ms as f64));
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date(),
        d.get_hours(),
        d.get_minutes()
    )
}
/// A datetime-local value, read in this browser's time zone.
fn parse_local(value: &str) -> Option<u64> {
    if value.len() < 16 {
        return None;
    }
    let ms = js_sys::Date::new(&JsValue::from_str(value)).get_time();
    (ms.is_finite() && ms > 0.0).then_some(ms as u64)
}
/// This browser's IANA time zone.
pub(super) fn zone() -> String {
    let options = js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &js_sys::Object::new())
        .resolved_options();
    js_sys::Reflect::get(&options, &JsValue::from_str("timeZone"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "UTC".into())
}
/// "Tue, Dec 1, 7:00 PM CET": in the viewer's time zone, with the zone named.
pub(super) fn when(ms: u64) -> String {
    let options = js_sys::Object::new();
    for (k, v) in [
        ("weekday", "short"),
        ("month", "short"),
        ("day", "numeric"),
        ("hour", "numeric"),
        ("minute", "2-digit"),
        ("timeZoneName", "short"),
    ] {
        let _ = js_sys::Reflect::set(&options, &JsValue::from_str(k), &JsValue::from_str(v));
    }
    js_sys::Date::new(&JsValue::from_f64(ms as f64))
        .to_locale_string("default", &options)
        .into()
}

/// Submits one of `FORMS`.
pub(super) fn submit(app: Shared, form: &str) {
    let mut a = app.borrow_mut();
    match form {
        "stage-start" => {
            let Some(ch) = a.stage.starting else { return };
            let topic = value("stage-topic").trim().to_owned();
            if topic.is_empty() {
                a.error = "Give the Stage a topic.".into();
                a.touch();
                return;
            }
            let note = value("stage-note").trim().to_owned();
            let event_id = id(&value("stage-event"));
            a.dialog.clear();
            a.stage.self_promoted = true;
            a.touch();
            let start = StageSignal::Start {
                channel_id: ch,
                topic,
                privacy_note: (!note.is_empty()).then_some(note),
                event_id,
            };
            drop(a);
            // Join first (a moderator may wait on a Stage), then open it.
            join(app.clone(), ch);
            stage(&mut app.borrow_mut(), start);
        }
        "stage-topic" => {
            let Some(ch) = a.voice_channel else { return };
            let topic = value("stage-topic").trim().to_owned();
            let note = value("stage-note").trim().to_owned();
            if topic.is_empty() {
                a.error = "Give the Stage a topic.".into();
                a.touch();
                return;
            }
            stage(
                &mut a,
                StageSignal::Update {
                    channel_id: ch,
                    topic,
                    privacy_note: (!note.is_empty()).then_some(note),
                },
            );
            a.dialog.clear();
            a.touch();
        }
        "event-form" => {
            let name = value("event-name").trim().to_owned();
            let description = value("event-description");
            let Some(start) = parse_local(&value("event-start")) else {
                a.error = "Choose when the event starts.".into();
                a.touch();
                return;
            };
            let end = parse_local(&value("event-end"));
            let place = value("event-place");
            let location = match value("event-location-kind").as_str() {
                "external" => EventLocation::External { place },
                kind => match id(&value("event-channel")) {
                    Some(channel_id) if kind == "stage" => EventLocation::Stage { channel_id },
                    Some(channel_id) => EventLocation::Voice { channel_id },
                    None => {
                        a.error = "Choose the channel it happens in.".into();
                        a.touch();
                        return;
                    }
                },
            };
            let recurrence = match value("event-repeat").as_str() {
                "daily" => EventRecurrence::Daily,
                "weekly" => EventRecurrence::Weekly,
                "every_two_weeks" => EventRecurrence::EveryTwoWeeks,
                "monthly" => EventRecurrence::Monthly,
                "yearly" => EventRecurrence::Yearly,
                "weekdays" => EventRecurrence::Weekdays,
                _ => EventRecurrence::None,
            };
            let following = checked("event-scope-following");
            match a.stage.editing.clone() {
                None => {
                    let Some(g) = a.guild else { return };
                    schedule(
                        &mut a,
                        ScheduleCommand::Create(NewEvent {
                            guild_id: g,
                            name,
                            description,
                            start_at: start,
                            end_at: end,
                            location,
                            recurrence,
                            time_zone: zone(),
                        }),
                    );
                }
                Some(e) => {
                    let scope = if following {
                        EditScope::Following
                    } else {
                        EditScope::This
                    };
                    let patch = EventPatch {
                        name: (name != e.name).then_some(name),
                        description: (description != e.description).then_some(description),
                        start_at: (start / 60_000 != e.start_at / 60_000).then_some(start),
                        end_at: match end {
                            Some(v) if Some(v / 60_000) != e.end_at.map(|x| x / 60_000) => {
                                Patch::Set(v)
                            }
                            None if e.end_at.is_some() => Patch::Clear,
                            _ => Patch::Keep,
                        },
                        location: (location != e.location).then_some(location),
                        recurrence: (recurrence != e.recurrence
                            && (scope == EditScope::Following
                                || e.recurrence == EventRecurrence::None))
                            .then_some(recurrence),
                        time_zone: None,
                    };
                    schedule(
                        &mut a,
                        ScheduleCommand::Update {
                            event_id: e.id,
                            occurrence: e.occurrence,
                            scope,
                            patch,
                        },
                    );
                }
            }
            a.touch();
        }
        _ => {}
    }
}

/// The Join dialog's invite leads to an event: fetch its card too.
pub(super) fn invite_preview(app: Shared, code: String) {
    let http = app.borrow().http();
    spawn_local(async move {
        let found = http
            .json::<InviteEvent>(&format!("/invites/{code}/event"))
            .await;
        let mut a = app.borrow_mut();
        if a.dialog == "join" && a.invite_wanted == code {
            a.stage.invite_event = found.ok().map(|e| (code, e));
            a.touch();
        }
    });
}

/// Friendly text for refused Stage and event actions.
pub(super) fn refused(cmd: &ScheduleCommand, code: ErrorCode) -> Option<&'static str> {
    Some(match (cmd, code) {
        (_, ErrorCode::Forbidden) => "You need Create Events or Manage Events to do that.",
        (ScheduleCommand::Create(_), ErrorCode::LimitExceeded) => {
            "This server has as many events as it can. Cancel or delete some first."
        }
        (ScheduleCommand::Create(_) | ScheduleCommand::Update { .. }, ErrorCode::InvalidRequest) => {
            "Check the name, the times (in the future, the end after the start) and the place. Events somewhere else need an end time."
        }
        _ => return None,
    })
}
