//! Shared activities in the browser (`docs/features/activities.md`).
//!
//! The Rust side keeps only what the views need: the session of the call this
//! client is in, the tile's layout, and a pending Join from an invite card.
//! Everything else (the launcher, the frame and its SDK bridge, the shared
//! player) is `public/activities.js`, loaded the first time an activity is
//! seen or the launcher is opened. Nobody pays for any of it otherwise.
//!
//! Lazily loaded (docs/features/perf.md): the voice envelope's activity
//! events go, undecoded, to `state`; the actions (launcher, join, signals,
//! sharing a file) to `run`. `activities.js` (the host: launcher, frame, SDK
//! bridge, player) is imported only when this client launches or joins one,
//! opens the launcher, or takes part: until then a session someone else
//! started is only shown by the Rust views (the banner, the dot).
use super::features::{self, Call};
use super::*;

#[wasm_bindgen(raw_module = "/public/live.js")]
extern "C" {
    #[wasm_bindgen(js_name = activityEvent)]
    fn activity_event(value: &str);
    #[wasm_bindgen(js_name = activityAction)]
    fn activity_action(action: &str);
}

/// Signals `activities.js` sent that it waits on (answers go back to it).
const MAX_PENDING: usize = 64;

#[derive(Default)]
pub(super) struct State {
    /// The activity of the call this client is in.
    pub session: Option<ActivitySession>,
    /// How the tile is laid out: "split" (beside the people), "focus" (the
    /// whole call view) or "mini" (a small floating window).
    pub layout: &'static str,
    /// From an invite card: join this session once the call connects.
    pending: Option<(Id, Id)>,
    nonces: std::collections::VecDeque<u64>,
    /// "X started Y" banners the person closed.
    pub dismissed: Option<Id>,
    /// `activities.js` has been asked for (it then gets every event).
    host: bool,
}
/// The tag an activity event from the voice envelope is handed over with.
pub(super) const VOICE_TAG: &str = "voice_activities";

/// The activity of the call this client is in now (none after leaving it).
pub(super) fn current(a: &App) -> Option<&ActivitySession> {
    let c = a.voice_channel?;
    a.activities.session.as_ref().filter(|s| s.channel_id == c)
}

fn forward(event: &ActivityEvent) {
    let mut v = serde_json::to_value(event).unwrap_or_default();
    crate::wire::stringify_ids(&mut v);
    activity_event(&v.to_string());
}

/// The host script is needed now: give it the session it has not seen.
fn wake_host(app: &Shared) {
    let session = {
        let mut a = app.borrow_mut();
        if std::mem::replace(&mut a.activities.host, true) {
            return;
        }
        current(&a).cloned()
    };
    if let Some(s) = session {
        forward(&ActivityEvent::Session(Box::new(s)));
    }
}

/// `VoiceEvent::Activities`, from the voice envelope.
fn voice_event(app: &Shared, event: ActivityEvent) {
    let mut a = app.borrow_mut();
    let mut join = None;
    let touch = match &event {
        ActivityEvent::Session(s) => {
            if a.voice_channel != Some(s.channel_id) {
                return;
            }
            let new = a.activities.session.as_ref().map(|o| o.session_id) != Some(s.session_id);
            if new && a.activities.layout.is_empty() {
                a.activities.layout = "split";
            }
            if a.activities
                .pending
                .is_some_and(|(c, id)| c == s.channel_id && id == s.session_id)
                && s.you.is_none()
            {
                a.activities.pending = None;
                join = Some(s.session_id);
            }
            a.activities.session = Some((**s).clone());
            true
        }
        ActivityEvent::Ended { session_id, .. } => {
            let mine = a
                .activities
                .session
                .as_ref()
                .is_some_and(|s| s.session_id == *session_id);
            if mine {
                a.activities.session = None;
            }
            mine
        }
        _ => false,
    };
    if touch {
        a.touch();
    }
    // Someone else's session: the call view shows it; the host script loads
    // when this client takes part (or opens the launcher).
    let taking_part = matches!(&event, ActivityEvent::Session(s) if s.you.is_some())
        || matches!(
            &event,
            ActivityEvent::State { .. } | ActivityEvent::Message { .. }
        );
    let host = a.activities.host || taking_part || join.is_some();
    a.activities.host = host;
    drop(a);
    if host {
        forward(&event);
    }
    if let Some(id) = join {
        activity_action(&format!("join:{}", id.0));
    }
}

/// An answer (Ack or Error) to a signal `activities.js` sent: it goes back
/// there. Written by hand: serializing a `VoiceEvent` would bring every
/// variant's encoder into the voice module.
pub(super) fn answer(app: &Shared, nonce: u64, event: &VoiceEvent) -> bool {
    let mut a = app.borrow_mut();
    let Some(i) = a.activities.nonces.iter().position(|n| *n == nonce) else {
        return false;
    };
    a.activities.nonces.remove(i);
    drop(a);
    let v = match event {
        VoiceEvent::Ack => serde_json::json!({ "type": "ack" }),
        VoiceEvent::Error { code, message } => serde_json::json!({
            "type": "error",
            "data": { "code": code, "message": message },
        }),
        _ => return false,
    };
    let mut v = v;
    v["request_nonce"] = nonce.to_string().into();
    activity_event(&v.to_string());
    true
}

/// `activity-signal:<nonce>:<json>` from `activities.js`: an activity signal
/// to send. Decoding JSON into a `VoiceSignal` is large code: it is shared
/// with the media host's signals (`voice_action`), out of the core.
fn signal(app: Shared, arg: String) {
    let Some((n, raw)) = arg.split_once(':') else {
        return;
    };
    let Ok(nonce) = n.parse::<u64>() else { return };
    if !(1..=9_007_199_254_740_991).contains(&nonce) {
        return;
    }
    let Ok(signal @ VoiceSignal::Activities(_)) = crate::wire::decode_signal(raw) else {
        return;
    };
    let mut a = app.borrow_mut();
    if a.activities.nonces.len() >= MAX_PENDING {
        a.activities.nonces.pop_front();
    }
    a.activities.nonces.push_back(nonce);
    a.voice_request(signal, nonce);
}

/// `activity-send-file:<channel>:<upload>`: a file uploaded for Watch Together
/// is sent to the call's chat, so it is a readable attachment there.
fn send_file(app: Shared, arg: &str) {
    let Some((c, u)) = arg.split_once(':') else {
        return;
    };
    let (Some(channel_id), Some(upload)) = (id(c), id(u)) else {
        return;
    };
    let mut a = app.borrow_mut();
    if a.voice_channel != Some(channel_id) {
        return;
    }
    a.command(Command::SendMessage(MessageDraft {
        channel_id,
        content: String::new(),
        reply_to: None,
        attachment_ids: vec![upload],
        mention_ids: vec![],
    }));
}

/// `activity:<verb>[,<args>]` from the views and from `activities.js`.
fn action(app: Shared, arg: &str) {
    let mut parts = arg.split(',');
    let verb = parts.next().unwrap_or_default();
    match verb {
        // Open the launcher (the button in the call controls).
        "launcher" => {
            wake_host(&app);
            activity_action("launcher")
        }
        "layout" => {
            let mut a = app.borrow_mut();
            a.activities.layout = match parts.next() {
                Some("focus") => "focus",
                Some("mini") => "mini",
                _ => "split",
            };
            a.touch();
        }
        "dismiss" => {
            let mut a = app.borrow_mut();
            a.activities.dismissed = a.activities.session.as_ref().map(|s| s.session_id);
            a.touch();
        }
        // Join from the call view's banner, or from an invite card (which
        // joins the call first when needed).
        "join" => {
            let (Some(call), Some(session)) =
                (parts.next().and_then(id), parts.next().and_then(id))
            else {
                return;
            };
            let (here, dm) = {
                let a = app.borrow();
                (
                    a.voice_channel == Some(call),
                    a.model.channels.get(&call).is_some_and(|c| {
                        matches!(c.kind, ChannelKind::Direct | ChannelKind::GroupDm)
                    }),
                )
            };
            if here {
                wake_host(&app);
                activity_action(&format!("join:{}", session.0));
                operations::action(app, &format!("voice:{}", call.0));
                return;
            }
            app.borrow_mut().activities.pending = Some((call, session));
            if dm {
                spawn_local(calls::action(app, format!("answer,{}", call.0)));
            } else {
                operations::action(app, &format!("voice:{}", call.0));
            }
        }
        "notice" => {
            let mut a = app.borrow_mut();
            a.notice = arg
                .strip_prefix("notice,")
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            a.touch();
        }
        _ => {}
    }
}

/// What a person does with activities (`features.rs`): the launcher, joining,
/// the host script's signals, sharing a file into the call's chat.
#[leptos::wasm_split::wasm_split(
    activities,
    wasm_split_path = ::leptos::wasm_split,
    fallible,
    preload(prefetch)
)]
pub(super) fn run(app: Shared, call: features::Call) -> features::Outcome {
    if let Call::Action(kind, arg) = call {
        match kind.as_str() {
            "activity-signal" => signal(app, arg),
            "activity-send-file" => send_file(app, &arg),
            _ => action(app, &arg),
        }
    }
    Ok(true)
}

/// The call's activity events (from the voice envelope, `mod.rs`).
#[leptos::wasm_split::wasm_split(
    activities_state,
    wasm_split_path = ::leptos::wasm_split,
    fallible
)]
pub(super) fn state(app: Shared, call: features::Call) -> features::Outcome {
    if let Call::Event(raw) = call {
        let Ok(e) = crate::inbound::feature::activity_event(&raw.data) else {
            return Ok(false);
        };
        voice_event(&app, e);
    }
    Ok(true)
}
