//! Direct and group calls in the browser: the ringing and "call in progress"
//! state the server sends, answering, declining and ringing again, the chat
//! panel beside a call, and the ring sounds. A call's media is the ordinary
//! call path (`voice:<channel>`, the media host in `public/media.js`).
use super::*;

// The ring sounds. The module is tiny and makes no sound, timer or audio graph
// until a ring starts; tests replace it with a silent recorder.
#[wasm_bindgen(raw_module = "/public/calls.js")]
extern "C" {
    #[wasm_bindgen(js_name = ring)]
    fn play(kind: &str);
}

/// More than any one person's conversations can hold at once.
const MAX_CALLS: usize = 256;

#[derive(Default)]
pub(super) struct Calls {
    /// The calls in this user's conversations, by conversation.
    pub live: BTreeMap<Id, DmCall>,
    /// The conversation or voice channel whose chat is open beside the call
    /// (or, for a voice channel, on its own).
    pub chat: Option<Id>,
    /// Leaves this client sent. Each is answered by one "call over for you";
    /// any other one means the server took this connection out of the call.
    stale: BTreeMap<Id, u32>,
    /// The ring sound playing now: "", "incoming" or "outgoing".
    sound: std::cell::Cell<&'static str>,
}

impl Calls {
    /// Signed out: forget every call. The sound is left for `sync` to stop.
    pub(super) fn reset(&mut self) {
        self.live.clear();
        self.chat = None;
        self.stale.clear();
    }
}

fn me(a: &App) -> Option<Id> {
    a.model.me.as_ref().map(|me| me.user.id)
}

/// The call ringing for this user now, if any (the newest first).
pub(super) fn incoming(a: &App) -> Option<&DmCall> {
    let me = me(a)?;
    a.calls
        .live
        .values()
        .filter(|c| {
            c.ringing.contains(&me)
                && a.voice_channel != Some(c.channel_id)
                && a.model.channels.contains_key(&c.channel_id)
        })
        .max_by_key(|c| c.started_at)
}

/// This user is in a call that is ringing others and nobody else has joined.
fn outgoing(a: &App) -> bool {
    let Some(me) = me(a) else { return false };
    a.voice_channel
        .and_then(|c| a.calls.live.get(&c))
        .is_some_and(|c| {
            !c.ringing.is_empty() && c.participants.iter().all(|p| p.session.user_id == me)
        })
}

/// Called with every render: starts or stops the ring sound to match the state.
pub(super) fn sync(a: &App) {
    let dnd = a
        .model
        .me
        .as_ref()
        .is_some_and(|me| me.desired_presence == PresenceStatus::Dnd);
    let next = if a.model.me.is_none() {
        ""
    } else if incoming(a).is_some() && !dnd {
        "incoming"
    } else if outgoing(a) {
        "outgoing"
    } else {
        ""
    };
    if a.calls.sound.replace(next) != next {
        play(next);
    }
}

/// A call event from the server.
pub(super) fn event(app: &Shared, event: VoiceEvent) {
    let mut a = app.borrow_mut();
    let mut removed = false;
    match event {
        VoiceEvent::CallSnapshot { calls } => {
            a.calls.live = calls
                .into_iter()
                .take(MAX_CALLS)
                .map(|c| (c.channel_id, c))
                .collect();
            a.calls.stale.clear();
        }
        VoiceEvent::CallState(call) => {
            if a.calls.live.len() < MAX_CALLS || a.calls.live.contains_key(&call.channel_id) {
                a.calls.live.insert(call.channel_id, call);
            }
        }
        VoiceEvent::CallEnded { channel_id } => {
            a.calls.live.remove(&channel_id);
            match a.calls.stale.get_mut(&channel_id) {
                Some(n) if *n > 1 => *n -= 1,
                Some(_) => {
                    a.calls.stale.remove(&channel_id);
                }
                None => removed = a.voice_channel == Some(channel_id),
            }
        }
        _ => {}
    }
    if removed {
        // The server ended this connection's place in the call (blocked,
        // removed from the group, or its media failed): stop the media here.
        a.voice_channel = None;
        media_action("transport-lost");
    }
    a.touch();
    drop(a);
    if removed {
        operations::action(app.clone(), "call-ended");
    }
}

/// This client is about to leave the call it is in.
pub(super) fn leaving(a: &mut App) {
    if let Some(c) = a.voice_channel.filter(|c| a.calls.live.contains_key(c)) {
        *a.calls.stale.entry(c).or_default() += 1;
    }
}

/// `call:<verb>,<channel>[,<arg>]` actions from call surfaces.
pub(super) fn action(app: Shared, arg: &str) {
    let mut parts = arg.split(',');
    let verb = parts.next().unwrap_or_default();
    let channel = parts.next().and_then(id);
    let extra = parts.next().unwrap_or_default();
    match (verb, channel) {
        // Start, answer and join are one thing on the server: joining the call.
        ("start" | "answer", Some(c)) => {
            let known = app
                .borrow()
                .model
                .channels
                .get(&c)
                .is_some_and(|c| matches!(c.kind, ChannelKind::Direct | ChannelKind::GroupDm));
            if !known {
                return;
            }
            if extra == "video" {
                media_action("camera-on-join");
            }
            app.borrow_mut().calls.chat = None;
            operations::action(app, &format!("voice:{}", c.0));
        }
        ("decline", Some(c)) => {
            let mut a = app.borrow_mut();
            a.voice(VoiceSignal::DeclineCall { channel_id: c });
            // Quiet now; the server's answer ends the ring on every device.
            let me = a.model.me.as_ref().map(|me| me.user.id);
            if let (Some(call), Some(me)) = (a.calls.live.get_mut(&c), me) {
                call.ringing.retain(|u| *u != me);
            }
            a.touch();
        }
        ("ring", Some(c)) => {
            if let Some(user) = id(extra) {
                let mut a = app.borrow_mut();
                a.voice(VoiceSignal::Ring {
                    channel_id: c,
                    user_ids: vec![user],
                });
            }
        }
        // The chat of a voice channel, or of the conversation a call is in.
        ("chat", Some(c)) => {
            {
                let mut a = app.borrow_mut();
                if a.calls.chat == Some(c) && a.screen == "voice" {
                    a.calls.chat = None;
                    a.touch();
                    return;
                }
                a.calls.chat = Some(c);
            }
            operations::load_channel(app, c, None);
        }
        ("chat-close", _) => {
            let mut a = app.borrow_mut();
            a.calls.chat = None;
            a.touch();
        }
        _ => {}
    }
}
