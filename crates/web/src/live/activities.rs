//! Shared activities in the browser (`docs/features/activities.md`).
//!
//! The Rust side keeps only what the views need: the session of the call this
//! client is in, the tile's layout, and a pending Join from an invite card.
//! Everything else (the launcher, the frame and its SDK bridge, the shared
//! player) is `public/activities.js`, loaded the first time an activity is
//! seen or the launcher is opened. Nobody pays for any of it otherwise.
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
}

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

/// `VoiceEvent::Activities`, from the voice envelope.
pub(super) fn voice_event(app: &Shared, event: ActivityEvent) {
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
    drop(a);
    forward(&event);
    if let Some(id) = join {
        activity_action(&format!("join:{}", id.0));
    }
}

/// An answer (Ack or Error) to a signal `activities.js` sent: it goes back there.
pub(super) fn answer(app: &Shared, nonce: u64, event: &VoiceEvent) -> bool {
    let mut a = app.borrow_mut();
    let Some(i) = a.activities.nonces.iter().position(|n| *n == nonce) else {
        return false;
    };
    a.activities.nonces.remove(i);
    drop(a);
    let mut v = serde_json::to_value(event).unwrap_or_default();
    crate::wire::stringify_ids(&mut v);
    v["request_nonce"] = nonce.to_string().into();
    activity_event(&v.to_string());
    true
}

/// `activity-signal:<nonce>:<json>` from `activities.js`: an activity signal
/// to send. Decoding JSON into a `VoiceSignal` is large code: it stays in a
/// lazily loaded chunk, as the media host's signals do (`voice_action`).
pub(super) fn signal(app: Shared, arg: &str) {
    let epoch = app.borrow().epoch;
    let arg = arg.to_owned();
    spawn_local(async move { signal_lazy(app, arg, epoch).await });
}

#[lazy]
fn signal_lazy(app: Shared, arg: String, epoch: u64) {
    if app.borrow().epoch != epoch {
        return;
    }
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
pub(super) fn send_file(app: Shared, arg: &str) {
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
pub(super) fn action(app: Shared, arg: &str) {
    let mut parts = arg.split(',');
    let verb = parts.next().unwrap_or_default();
    match verb {
        // Open the launcher (the button in the call controls).
        "launcher" => activity_action("launcher"),
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
