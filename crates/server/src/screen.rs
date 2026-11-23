//! Screen control for AI agents (docs/MCP.md, "Control a shared screen").
//!
//! A person in a call can let their AI app (an MCP connection they approved
//! for screen control) ask to control a screen someone else is sharing. The
//! sharer is asked by their own Windows app exactly as for a person, and is
//! told it is an agent. Once they allow it:
//!
//! - the agent's input is queued in the SFU, which performs it on its own clock
//!   over the sharer's control channel, under the checks a person's input gets;
//! - the agent sees through pictures the sharer's app uploads here on request.
//!
//! Nothing here runs unless an agent is asking or acting: no task and no timer.
use crate::{
    db,
    error::{require, Error, Result},
    http,
    state::{now, App, Core},
};
use axum::{
    body::{to_bytes, Body},
    extract::{Path, State},
    http::{Request, StatusCode},
};
use bytes::Bytes;
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use sticktalk_proto::*;
use sticktalk_sfu::{AgentAction, AgentStep};
use tokio::sync::oneshot;

/// How long the sharer has to answer (the SFU's own limit), plus slack.
const ANSWER: Duration = Duration::from_secs(32);
/// How long a picture may take to arrive after it was asked for.
const PICTURE: Duration = Duration::from_secs(8);
/// A grant no agent has used for this long is given back.
const IDLE_MS: u64 = 10 * 60_000;
/// A 1920-pixel JPEG is a few hundred kilobytes; nothing larger is a picture.
const MAX_PICTURE: usize = 2 << 20;
const QUALITY: u8 = 70;

/// Why an agent's call could not be done, in words the agent can act on.
pub(crate) struct Refused {
    pub code: &'static str,
    pub message: String,
}
fn refused(code: &'static str, message: impl Into<String>) -> Refused {
    Refused {
        code,
        message: message.into(),
    }
}

pub(crate) struct Frame {
    pub jpeg: Bytes,
    pub width: u32,
    pub height: u32,
}

struct Control {
    /// The MCP connection (its session hash) that asked. Only it may act.
    mcp: Vec<u8>,
    controller: VoiceSession,
    sharer: VoiceSession,
    /// None while the sharer has not answered.
    grant: Option<u64>,
    answer: Option<oneshot::Sender<bool>>,
    frames: HashMap<u64, oneshot::Sender<Frame>>,
    /// The size of the last picture: the agent's coordinates are its pixels.
    size: Option<(u32, u32)>,
    /// One script at a time.
    acting: Arc<tokio::sync::Mutex<()>>,
    used: u64,
}

/// One entry per person whose agent is asking for or holding control.
#[derive(Default)]
pub(crate) struct Screens(Mutex<HashMap<Id, Control>>);

impl Screens {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Id, Control>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Follows the call's own control events, which are the only authority on
    /// whether a grant exists. Returns a controller whose grant nobody here is
    /// waiting for any more, to be given back.
    pub fn observe(&self, to: VoiceSession, event: &VoiceEvent) -> Option<VoiceSession> {
        let mut all = self.lock();
        match *event {
            VoiceEvent::ControlGranted {
                sharer,
                controller,
                grant,
                agent: true,
            } => {
                let Some(control) = all
                    .get_mut(&controller.user_id)
                    .filter(|c| c.controller == controller && c.sharer == sharer)
                else {
                    return Some(controller);
                };
                control.grant = Some(grant);
                control.used = now();
                if let Some(answer) = control.answer.take() {
                    let _ = answer.send(true);
                }
            }
            VoiceEvent::ControlDenied { sharer } => {
                if all
                    .get(&to.user_id)
                    .is_some_and(|c| c.controller == to && c.sharer == sharer && c.grant.is_none())
                {
                    if let Some(answer) = all.remove(&to.user_id).and_then(|c| c.answer) {
                        let _ = answer.send(false);
                    }
                }
            }
            VoiceEvent::ControlRevoked { sharer, grant } => {
                all.retain(|_, c| !(c.sharer == sharer && c.grant == Some(grant)));
            }
            _ => {}
        }
        None
    }
    /// Either side leaving the call ends the agent's part in it.
    pub fn left(&self, session: VoiceSession) {
        self.lock()
            .retain(|_, c| c.controller != session && c.sharer != session);
    }
    /// Grants no agent has used for a while, forgotten here; the caller gives
    /// them back. Run from the existing housekeeping pass.
    pub fn expire(&self) -> Vec<VoiceSession> {
        let t = now();
        let mut idle = vec![];
        self.lock().retain(|_, c| {
            let keep = c.grant.is_none() || t.saturating_sub(c.used) < IDLE_MS;
            if !keep {
                idle.push(c.controller);
            }
            keep
        });
        idle
    }
    /// The size of the agent's last picture, if this connection holds control.
    pub fn size(&self, mcp: &[u8], user: Id) -> std::result::Result<Option<(u32, u32)>, Refused> {
        self.active(mcp, user).map(|c| c.2)
    }
    /// Whose screen this person's agent controls or is asking for, and whether
    /// it has been allowed yet.
    pub fn current(&self, user: Id) -> Option<(VoiceSession, bool)> {
        self.lock()
            .get(&user)
            .map(|c| (c.sharer, c.grant.is_some()))
    }
    #[allow(clippy::type_complexity)]
    fn active(
        &self,
        mcp: &[u8],
        user: Id,
    ) -> std::result::Result<
        (
            VoiceSession,
            Arc<tokio::sync::Mutex<()>>,
            Option<(u32, u32)>,
        ),
        Refused,
    > {
        let mut all = self.lock();
        let control = all
            .get_mut(&user)
            .filter(|c| c.grant.is_some())
            .ok_or_else(not_controlling)?;
        if control.mcp != mcp {
            return Err(other_connection());
        }
        control.used = now();
        Ok((control.controller, control.acting.clone(), control.size))
    }
}
fn not_controlling() -> Refused {
    refused(
        "not_controlling",
        "You are not controlling a screen: it was never allowed, or control has ended (the person stopped it, stopped sharing, or someone left the call). Use list_screen_shares, then request_screen_control.",
    )
}
fn other_connection() -> Refused {
    refused(
        "other_connection",
        "Another AI connection of this user is controlling a screen. Only one can at a time.",
    )
}

/// Withdraws a question that will not be waited for, or a grant nobody holds.
fn give_back(app: &App, controller: VoiceSession) {
    let _ = app.0.voice.agent_send(controller, AgentAction::Revoke);
}

/// Called for every voice event the call sends to a session.
pub(crate) fn observe(app: &App, to: VoiceSession, event: &VoiceEvent) {
    if let Some(orphan) = app.0.screens.observe(to, event) {
        give_back(app, orphan);
    }
}
pub(crate) fn expire(app: &App) {
    for controller in app.0.screens.expire() {
        give_back(app, controller);
    }
}

/// A screen shared in a call the person is in.
pub(crate) struct Share {
    pub mine: VoiceSession,
    pub sharer: VoiceSession,
    pub channel: Id,
    pub surface: ShareSurface,
}
/// Every screen shared in the calls `user` is in, by other people.
pub(crate) fn shares(core: &Core, user: Id) -> (bool, Vec<Share>) {
    let mine: Vec<_> = core
        .voice
        .iter()
        .filter(|(s, e)| s.user_id == user && e.joined)
        .map(|(s, e)| (*s, e.state.channel_id))
        .collect();
    let mut found = vec![];
    for (session, channel) in &mine {
        for (other, entry) in &core.voice {
            // Listed once, even when the person is in the call on two devices.
            if other.user_id == user
                || !entry.joined
                || entry.state.channel_id != *channel
                || found.iter().any(|f: &Share| f.sharer == *other)
            {
                continue;
            }
            let surface = entry
                .state
                .participant
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Screen)
                .and_then(|t| t.surface);
            if let Some(surface) = surface {
                found.push(Share {
                    mine: *session,
                    sharer: *other,
                    channel: *channel,
                    surface,
                });
            }
        }
    }
    (!mine.is_empty(), found)
}

/// Removes an unanswered question when the agent's call ends early (its
/// connection dropped, or nobody answered in time).
struct Asking<'a> {
    app: &'a App,
    user: Id,
    controller: VoiceSession,
    armed: bool,
}
impl Drop for Asking<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut all = self.app.0.screens.lock();
        if all
            .get(&self.user)
            .is_some_and(|c| c.controller == self.controller && c.grant.is_none())
        {
            all.remove(&self.user);
        }
        drop(all);
        give_back(self.app, self.controller);
    }
}

/// Asks `target` to let this person's agent control the screen they share,
/// and waits for their answer. `Ok` means control is on.
pub(crate) async fn request(
    app: &App,
    mcp: &[u8],
    user: Id,
    target: Id,
) -> std::result::Result<(), Refused> {
    let (in_call, shared) = {
        let core = app.0.core.lock().await;
        shares(&core, user)
    };
    if !in_call {
        return Err(refused(
            "not_in_call",
            "The user is not in a call. They need to join the voice channel in StickTalk first: an agent can only control a screen shared in a call its user is in.",
        ));
    }
    let share = shared
        .into_iter()
        .find(|s| s.sharer.user_id == target)
        .ok_or_else(|| {
            refused(
                "not_sharing",
                "That user is not sharing a screen in the user's call. Check list_screen_shares.",
            )
        })?;
    let (answer, answered) = oneshot::channel();
    {
        let mut all = app.0.screens.lock();
        if let Some(existing) = all.get(&user) {
            if existing.mcp != mcp {
                return Err(other_connection());
            }
            if existing.grant.is_some() && existing.sharer == share.sharer {
                return Ok(());
            }
            return Err(refused(
                "busy",
                "You already control, or are asking to control, a screen. Call release_screen_control first.",
            ));
        }
        all.insert(
            user,
            Control {
                mcp: mcp.to_vec(),
                controller: share.mine,
                sharer: share.sharer,
                grant: None,
                answer: Some(answer),
                frames: HashMap::new(),
                size: None,
                acting: Default::default(),
                used: now(),
            },
        );
    }
    let mut asking = Asking {
        app,
        user,
        controller: share.mine,
        armed: true,
    };
    let asked = app
        .0
        .voice
        .agent(
            share.mine,
            AgentAction::Request {
                sharer: share.sharer,
            },
        )
        .await;
    if asked.is_err() {
        return Err(refused(
            "cannot_control",
            "This screen can't be controlled right now. The person must be sharing a whole screen or a window from the StickTalk Windows app (not a browser), and nobody else may be controlling it or asking to.",
        ));
    }
    match tokio::time::timeout(ANSWER, answered).await {
        Ok(Ok(true)) => {
            asking.armed = false;
            Ok(())
        }
        Ok(Ok(false)) => {
            asking.armed = false;
            Err(refused(
                "declined",
                "They declined, or StickTalk on their PC could not allow it. Don't ask again unless the user tells you to.",
            ))
        }
        Ok(Err(_)) => {
            asking.armed = false;
            Err(refused(
                "ended",
                "The request ended before it was answered: someone left the call or stopped sharing.",
            ))
        }
        Err(_) => Err(refused(
            "no_answer",
            "They did not answer within 30 seconds. Don't ask again unless the user tells you to.",
        )),
    }
}

/// Forgets a picture that is no longer waited for, however its call ends.
struct Waiting<'a> {
    app: &'a App,
    user: Id,
    id: u64,
}
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if let Some(control) = self.app.0.screens.lock().get_mut(&self.user) {
            control.frames.remove(&self.id);
        }
    }
}

/// Performs a plan on the controlled screen and, if `picture` is set, returns
/// a picture taken `settle` after the last step, at most `max` pixels wide or
/// tall. Returns once everything has been performed.
pub(crate) async fn perform(
    app: &App,
    mcp: &[u8],
    user: Id,
    plan: Plan,
    keep_held: Duration,
    picture: Option<(u16, Duration)>,
) -> std::result::Result<Option<Frame>, Refused> {
    let (controller, acting, _) = app.0.screens.active(mcp, user)?;
    // Never queue behind another call: that would hold a request slot open.
    let Ok(_turn) = acting.try_lock() else {
        return Err(refused(
            "busy",
            "The previous actions are still being performed. Wait for that call to return.",
        ));
    };
    let mut steps = plan.steps;
    let mut end = plan.end;
    let mut waiting = None;
    if let Some((max, settle)) = picture {
        end += settle;
        // Small enough to be an exact JavaScript number in the sharer's page.
        let id = rand::random::<u64>() >> 11;
        let (arrived, frame) = oneshot::channel();
        app.0
            .screens
            .lock()
            .get_mut(&user)
            .filter(|c| c.controller == controller && c.grant.is_some())
            .ok_or_else(not_controlling)?
            .frames
            .insert(id, arrived);
        steps.push((
            end,
            AgentStep::Capture {
                id,
                max,
                quality: QUALITY,
            },
        ));
        waiting = Some((Waiting { app, user, id }, frame));
    }
    let queued = app
        .0
        .voice
        .agent(controller, AgentAction::Script { steps, keep_held })
        .await;
    if let Err(error) = queued {
        return Err(match error {
            sticktalk_sfu::Error::Limit => refused(
                "busy",
                "The previous actions are still being performed. Wait a moment and try again.",
            ),
            sticktalk_sfu::Error::Backpressure => {
                refused("unavailable", "StickTalk is busy. Try again shortly.")
            }
            _ => not_controlling(),
        });
    }
    let Some((_waiting, frame)) = waiting else {
        tokio::time::sleep(end).await;
        return Ok(None);
    };
    match tokio::time::timeout(end + PICTURE, frame).await {
        Ok(Ok(frame)) => {
            if let Some(control) = app.0.screens.lock().get_mut(&user) {
                control.size = Some((frame.width, frame.height));
                control.used = now();
            }
            Ok(Some(frame))
        }
        Ok(Err(_)) => Err(not_controlling()),
        Err(_) => Err(refused(
            "no_picture",
            "The actions were performed, but no picture of the screen arrived. Their StickTalk app may be busy or have lost its connection. Try screen_screenshot.",
        )),
    }
}

/// The agent gives control back. False when it held none.
pub(crate) async fn release(app: &App, mcp: &[u8], user: Id) -> std::result::Result<bool, Refused> {
    let controller = {
        let mut all = app.0.screens.lock();
        match all.get(&user) {
            None => return Ok(false),
            Some(c) if c.mcp != mcp => return Err(other_connection()),
            Some(_) => all.remove(&user).map(|c| c.controller),
        }
    };
    if let Some(controller) = controller {
        let _ = app.0.voice.agent(controller, AgentAction::Revoke).await;
    }
    Ok(true)
}

pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new().route(
        "/api/v1/voice/control/frames/{id}",
        axum::routing::post(upload),
    )
}
/// The sharer's app delivers a picture an agent's grant asked for. Only a
/// picture that is being waited for, from the person whose screen it is, is
/// taken; it is handed to the waiting call and never stored.
async fn upload(
    State(app): State<App>,
    Path(id): Path<u64>,
    r: Request<Body>,
) -> Result<StatusCode> {
    let token = http::bearer(r.headers())?.to_owned();
    let user = {
        let mut c = app.0.pool.acquire().await?;
        let (_, user) = db::auth(&mut c, &token).await?;
        db::enabled(&mut c, user).await?;
        user
    };
    app.0
        .core
        .lock()
        .await
        .rate(format!("control-frame:{}", user.0), 1200., 120.)?;
    let waited = |all: &HashMap<Id, Control>| {
        all.iter()
            .find(|(_, c)| c.sharer.user_id == user && c.frames.contains_key(&id))
            .map(|(k, _)| *k)
    };
    require(waited(&app.0.screens.lock()).is_some(), ErrorCode::NotFound)?;
    let jpeg = to_bytes(r.into_body(), MAX_PICTURE)
        .await
        .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?;
    let (width, height) = jpeg_size(&jpeg).ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    let mut all = app.0.screens.lock();
    let arrived = waited(&all)
        .and_then(|k| all.get_mut(&k))
        .and_then(|c| c.frames.remove(&id))
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let _ = arrived.send(Frame {
        jpeg,
        width,
        height,
    });
    Ok(StatusCode::NO_CONTENT)
}

/// Width and height from a JPEG's frame header; `None` if it is not a JPEG.
pub(crate) fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 4 || b[0] != 0xff || b[1] != 0xd8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xff {
            return None;
        }
        let marker = b[i + 1];
        if marker == 0xff {
            i += 1;
            continue;
        }
        if marker == 0x01 || (0xd0..=0xd9).contains(&marker) {
            i += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
        if length < 2 {
            return None;
        }
        // Start-of-frame markers, which are not the table markers among them.
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let size = b.get(i + 5..i + 9)?;
            let height = u32::from(u16::from_be_bytes([size[0], size[1]]));
            let width = u32::from(u16::from_be_bytes([size[2], size[3]]));
            return (width > 0 && height > 0).then_some((width, height));
        }
        i += 2 + length;
    }
    None
}

// ---- From an agent's actions to timed input ---------------------------------

/// Milliseconds between two inputs: at most 500 a second, half the SFU's limit.
const GAP: u64 = 2;
/// Mouse movement is sent this often, about a mouse's own 125 reports a second.
const MOUSE_STEP: u64 = 8;
pub(crate) const MAX_PLAN_MS: u64 = 30_000;
const MAX_STEPS: usize = 3000;
pub(crate) const MAX_ACTIONS: usize = 100;

#[derive(Debug)]
pub(crate) struct Plan {
    pub steps: Vec<(Duration, AgentStep)>,
    /// When the last step has been performed.
    pub end: Duration,
    /// Keys or buttons are still down at the end.
    pub holding: bool,
}

/// A USB HID keyboard usage for a key's name.
pub(crate) fn key(name: &str) -> std::result::Result<u16, String> {
    let name = name.trim().to_ascii_lowercase();
    let bytes = name.as_bytes();
    if let [c] = bytes {
        match c {
            b'a'..=b'z' => return Ok(4 + u16::from(c - b'a')),
            b'1'..=b'9' => return Ok(30 + u16::from(c - b'1')),
            b'0' => return Ok(39),
            _ => {}
        }
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
        if (1..=12).contains(&n) {
            return Ok(57 + n);
        }
    }
    Ok(match name.as_str() {
        "enter" | "return" => 40,
        "esc" | "escape" => 41,
        "backspace" => 42,
        "tab" => 43,
        "space" | "spacebar" | " " => 44,
        "-" | "minus" => 45,
        "=" | "equal" | "equals" => 46,
        "[" => 47,
        "]" => 48,
        "\\" | "backslash" => 49,
        ";" | "semicolon" => 51,
        "'" | "quote" => 52,
        "`" | "backquote" | "grave" => 53,
        "," | "comma" => 54,
        "." | "period" => 55,
        "/" | "slash" => 56,
        "capslock" => 57,
        "insert" => 73,
        "home" => 74,
        "pageup" | "page_up" => 75,
        "delete" | "del" => 76,
        "end" => 77,
        "pagedown" | "page_down" => 78,
        "right" | "arrowright" => 79,
        "left" | "arrowleft" => 80,
        "down" | "arrowdown" => 81,
        "up" | "arrowup" => 82,
        "ctrl" | "control" | "lctrl" => 0xe0,
        "shift" | "lshift" => 0xe1,
        "alt" | "lalt" => 0xe2,
        "rctrl" => 0xe4,
        "rshift" => 0xe5,
        "ralt" | "altgr" => 0xe6,
        "win" | "windows" | "meta" | "super" | "cmd" | "command" => {
            return Err("The Windows key can't be pressed remotely.".into())
        }
        _ => {
            return Err(format!(
                "Unknown key \"{name}\". Use a letter, a digit, f1-f12, enter, esc, backspace, tab, space, up, down, left, right, home, end, pageup, pagedown, insert, delete, capslock, ctrl, shift, alt, rctrl, rshift, ralt, or one of - = [ ] \\ ; ' ` , . /"
            ))
        }
    })
}
/// The keys of a combination such as `ctrl+shift+t`, in the order to press.
fn combo(text: &str) -> std::result::Result<Vec<u16>, String> {
    let keys: Vec<u16> = if text.trim().len() <= 1 {
        vec![key(text)?]
    } else {
        text.split('+')
            .map(key)
            .collect::<std::result::Result<_, _>>()?
    };
    if keys.is_empty() || keys.len() > 6 {
        return Err("A key combination has one to six keys.".into());
    }
    Ok(keys)
}
/// The key that types a character on a US keyboard, and whether with Shift.
fn character(c: char) -> Option<(u16, bool)> {
    const SHIFTED: &str = "!@#$%^&*()";
    Some(match c {
        'a'..='z' => (4 + (c as u16 - 'a' as u16), false),
        'A'..='Z' => (4 + (c as u16 - 'A' as u16), true),
        '1'..='9' => (30 + (c as u16 - '1' as u16), false),
        '0' => (39, false),
        '\n' => (40, false),
        '\t' => (43, false),
        ' ' => (44, false),
        '-' => (45, false),
        '_' => (45, true),
        '=' => (46, false),
        '+' => (46, true),
        '[' => (47, false),
        '{' => (47, true),
        ']' => (48, false),
        '}' => (48, true),
        '\\' => (49, false),
        '|' => (49, true),
        ';' => (51, false),
        ':' => (51, true),
        '\'' => (52, false),
        '"' => (52, true),
        '`' => (53, false),
        '~' => (53, true),
        ',' => (54, false),
        '<' => (54, true),
        '.' => (55, false),
        '>' => (55, true),
        '/' => (56, false),
        '?' => (56, true),
        _ => {
            let digit = SHIFTED.find(c)? as u16;
            (30 + digit, true)
        }
    })
}
fn button(a: &Map<String, Value>) -> std::result::Result<u8, String> {
    Ok(match a.get("button").and_then(Value::as_str) {
        None | Some("left") => 0,
        Some("right") => 1,
        Some("middle") => 2,
        Some("back") => 3,
        Some("forward") => 4,
        Some(other) => {
            return Err(format!(
                "Unknown button \"{other}\". Use left, right, middle, back or forward."
            ))
        }
    })
}
fn whole(
    a: &Map<String, Value>,
    name: &str,
    default: Option<i64>,
    least: i64,
    most: i64,
) -> std::result::Result<i64, String> {
    match a.get(name) {
        None | Some(Value::Null) => {
            default.ok_or_else(|| format!("\"{name}\" is required for this action."))
        }
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite())
            .map(|n| (n.round() as i64).clamp(least, most))
            .ok_or_else(|| format!("\"{name}\" must be a number.")),
    }
}

struct Builder {
    steps: Vec<(Duration, AgentStep)>,
    at: u64,
    size: Option<(u32, u32)>,
    keys: std::collections::BTreeSet<u16>,
    buttons: u8,
}
impl Builder {
    fn input(&mut self, input: ControlInput) {
        match input {
            ControlInput::Key { usage, down: true } => {
                self.keys.insert(usage);
            }
            ControlInput::Key { usage, down: false } => {
                self.keys.remove(&usage);
            }
            ControlInput::Button { button, down: true } => self.buttons |= 1 << button,
            ControlInput::Button {
                button,
                down: false,
            } => self.buttons &= !(1 << button),
            _ => {}
        }
        self.steps
            .push((Duration::from_millis(self.at), AgentStep::Input(input)));
        self.at += GAP;
    }
    /// A pixel of the agent's last picture, as a point on the shared surface.
    fn point(&self, x: i64, y: i64) -> std::result::Result<ControlInput, String> {
        let (width, height) = self
            .size
            .ok_or("Take a screenshot first: x and y are pixels of the most recent screenshot.")?;
        if x < 0 || y < 0 || x >= i64::from(width) || y >= i64::from(height) {
            return Err(format!(
                "({x}, {y}) is outside the {width}x{height} screenshot."
            ));
        }
        let scale = |v: i64, extent: u32| {
            if extent <= 1 {
                0
            } else {
                ((v * 65535 + i64::from(extent - 1) / 2) / i64::from(extent - 1)) as u16
            }
        };
        Ok(ControlInput::Move {
            x: scale(x, width),
            y: scale(y, height),
        })
    }
    /// Moves to `x`,`y` when the action names a place; both or neither.
    fn aim(&mut self, a: &Map<String, Value>, required: bool) -> std::result::Result<(), String> {
        if !required && !a.contains_key("x") && !a.contains_key("y") {
            return Ok(());
        }
        let x = whole(a, "x", None, i64::MIN, i64::MAX)?;
        let y = whole(a, "y", None, i64::MIN, i64::MAX)?;
        let point = self.point(x, y)?;
        self.input(point);
        Ok(())
    }
    fn action(&mut self, a: &Map<String, Value>) -> std::result::Result<(), String> {
        let kind = a
            .get("type")
            .and_then(Value::as_str)
            .ok_or("Every action needs a \"type\".")?;
        let fields: &[&str] = match kind {
            "move" => &["x", "y"],
            "look" => &["dx", "dy", "ms"],
            "click" => &["x", "y", "button", "count", "hold_ms"],
            "mouse_down" | "mouse_up" => &["x", "y", "button"],
            "scroll" => &["x", "y", "direction", "amount"],
            "key" => &["key", "hold_ms"],
            "key_down" | "key_up" => &["key"],
            "type" => &["text"],
            "drag" => &["x", "y", "to_x", "to_y", "button", "ms"],
            "wait" => &["ms"],
            other => {
                return Err(format!(
                    "Unknown action type \"{other}\". Use move, look, click, mouse_down, mouse_up, scroll, key, key_down, key_up, type, drag or wait."
                ))
            }
        };
        if let Some(extra) = a
            .keys()
            .find(|k| *k != "type" && !fields.contains(&k.as_str()))
        {
            return Err(format!(
                "Unknown field \"{extra}\" in a \"{kind}\" action. Its fields: {}.",
                fields.join(", ")
            ));
        }
        let text = |name: &str| {
            a.get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("\"{name}\" is required for a \"{kind}\" action."))
        };
        match kind {
            "move" => self.aim(a, true)?,
            "look" => {
                let dx = whole(a, "dx", Some(0), -30_000, 30_000)?;
                let dy = whole(a, "dy", Some(0), -30_000, 30_000)?;
                let ms = whole(a, "ms", Some(0), 0, 10_000)? as u64;
                let chunks = dx
                    .unsigned_abs()
                    .max(dy.unsigned_abs())
                    .div_ceil(2048)
                    .max(1);
                let count = chunks.max(ms / MOUSE_STEP) as i64;
                let start = self.at;
                let (mut sent_x, mut sent_y) = (0, 0);
                for i in 1..=count {
                    let (to_x, to_y) = (dx * i / count, dy * i / count);
                    self.at = self.at.max(start + ms * (i as u64 - 1) / count as u64);
                    if to_x != sent_x || to_y != sent_y {
                        self.input(ControlInput::MoveBy {
                            dx: (to_x - sent_x) as i16,
                            dy: (to_y - sent_y) as i16,
                        });
                    }
                    (sent_x, sent_y) = (to_x, to_y);
                }
                self.at = self.at.max(start + ms);
            }
            "click" => {
                let button = button(a)?;
                let count = whole(a, "count", Some(1), 1, 3)?;
                let hold = whole(a, "hold_ms", Some(40), 1, 5000)? as u64;
                self.aim(a, false)?;
                for i in 0..count {
                    self.input(ControlInput::Button { button, down: true });
                    self.at += hold;
                    self.input(ControlInput::Button {
                        button,
                        down: false,
                    });
                    if i + 1 < count {
                        self.at += 60;
                    }
                }
            }
            "mouse_down" | "mouse_up" => {
                let button = button(a)?;
                self.aim(a, false)?;
                self.input(ControlInput::Button {
                    button,
                    down: kind == "mouse_down",
                });
            }
            "scroll" => {
                // Positive is away from the person (up) and to the right.
                let (x, y) = match text("direction")? {
                    "up" => (0, 120),
                    "down" => (0, -120),
                    "left" => (-120, 0),
                    "right" => (120, 0),
                    other => {
                        return Err(format!(
                            "Unknown direction \"{other}\". Use up, down, left or right."
                        ))
                    }
                };
                let amount = whole(a, "amount", Some(3), 1, 50)?;
                self.aim(a, false)?;
                for _ in 0..amount {
                    self.input(ControlInput::Wheel { x, y });
                    self.at += 10;
                }
            }
            "key" => {
                let keys = combo(text("key")?)?;
                let hold = whole(a, "hold_ms", Some(40), 1, 10_000)? as u64;
                for usage in &keys {
                    self.input(ControlInput::Key {
                        usage: *usage,
                        down: true,
                    });
                }
                self.at += hold;
                for usage in keys.iter().rev() {
                    self.input(ControlInput::Key {
                        usage: *usage,
                        down: false,
                    });
                }
            }
            "key_down" | "key_up" => {
                for usage in combo(text("key")?)? {
                    self.input(ControlInput::Key {
                        usage,
                        down: kind == "key_down",
                    });
                }
            }
            "type" => {
                let typed = text("text")?;
                if typed.chars().count() > 500 {
                    return Err("\"text\" is at most 500 characters per action.".into());
                }
                for c in typed.chars() {
                    let (usage, shift) = character(c).ok_or_else(|| {
                        format!("\"{c}\" can't be typed: only the characters of a US keyboard can.")
                    })?;
                    // Shift a typed capital needs is not one the agent holds.
                    let shift = shift && !self.keys.contains(&0xe1);
                    if shift {
                        self.input(ControlInput::Key {
                            usage: 0xe1,
                            down: true,
                        });
                    }
                    self.input(ControlInput::Key { usage, down: true });
                    self.at += 12;
                    self.input(ControlInput::Key { usage, down: false });
                    if shift {
                        self.input(ControlInput::Key {
                            usage: 0xe1,
                            down: false,
                        });
                    }
                    self.at += 12;
                }
            }
            "drag" => {
                let button = button(a)?;
                let ms = whole(a, "ms", Some(400), 50, 10_000)? as u64;
                let from = (
                    whole(a, "x", None, i64::MIN, i64::MAX)?,
                    whole(a, "y", None, i64::MIN, i64::MAX)?,
                );
                let to = (
                    whole(a, "to_x", None, i64::MIN, i64::MAX)?,
                    whole(a, "to_y", None, i64::MIN, i64::MAX)?,
                );
                let start = self.point(from.0, from.1)?;
                self.point(to.0, to.1)?;
                self.input(start);
                self.input(ControlInput::Button { button, down: true });
                self.at += 40;
                let count = (ms / MOUSE_STEP).max(2) as i64;
                let begun = self.at;
                for i in 1..=count {
                    let along = |a: i64, b: i64| a + (b - a) * i / count;
                    let point = self.point(along(from.0, to.0), along(from.1, to.1))?;
                    self.at = self.at.max(begun + ms * i as u64 / count as u64);
                    self.input(point);
                }
                self.at += 40;
                self.input(ControlInput::Button {
                    button,
                    down: false,
                });
            }
            "wait" => self.at += whole(a, "ms", None, 1, 10_000)? as u64,
            _ => unreachable!(),
        }
        Ok(())
    }
}

/// Turns an agent's list of actions into timed input. `size` is the size of
/// the agent's last picture, whose pixels its coordinates are.
pub(crate) fn plan(
    actions: &[Value],
    size: Option<(u32, u32)>,
) -> std::result::Result<Plan, String> {
    if actions.len() > MAX_ACTIONS {
        return Err(format!("At most {MAX_ACTIONS} actions per call."));
    }
    let mut b = Builder {
        steps: vec![],
        at: 0,
        size,
        keys: Default::default(),
        buttons: 0,
    };
    for (i, action) in actions.iter().enumerate() {
        let action = action
            .as_object()
            .ok_or_else(|| format!("Action {} must be an object.", i + 1))?;
        b.action(action)
            .map_err(|e| format!("Action {}: {e}", i + 1))?;
        if b.at > MAX_PLAN_MS || b.steps.len() > MAX_STEPS {
            return Err(format!(
                "These actions take longer than {} seconds (or are too many). Split them across calls.",
                MAX_PLAN_MS / 1000
            ));
        }
    }
    Ok(Plan {
        end: Duration::from_millis(b.at),
        holding: !b.keys.is_empty() || b.buttons != 0,
        steps: b.steps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn inputs(p: &Plan) -> Vec<(u64, ControlInput)> {
        p.steps
            .iter()
            .map(|(at, step)| match step {
                AgentStep::Input(input) => (at.as_millis() as u64, input.clone()),
                AgentStep::Capture { .. } => panic!("plans hold input only"),
            })
            .collect()
    }
    fn key_step(usage: u16, down: bool) -> ControlInput {
        ControlInput::Key { usage, down }
    }
    const SIZE: Option<(u32, u32)> = Some((1280, 720));

    #[test]
    fn keys_are_named_as_an_agent_would_name_them() {
        assert_eq!(key("w"), Ok(26));
        assert_eq!(key("W"), Ok(26));
        assert_eq!(key("1"), Ok(30));
        assert_eq!(key("0"), Ok(39));
        assert_eq!(key("F1"), Ok(58));
        assert_eq!(key("f12"), Ok(69));
        assert_eq!(key("Escape"), Ok(41));
        assert_eq!(key("space"), Ok(44));
        assert_eq!(key("up"), Ok(82));
        assert_eq!(key("ctrl"), Ok(0xe0));
        assert_eq!(key("rshift"), Ok(0xe5));
        assert!(key("win").is_err());
        assert!(key("f13").is_err());
        assert!(key("launch").is_err());
        assert_eq!(combo("ctrl+shift+t"), Ok(vec![0xe0, 0xe1, 23]));
        assert_eq!(combo("+"), Err(key("+").unwrap_err()));
        assert_eq!(combo("="), Ok(vec![46]));
        assert_eq!(character('A'), Some((4, true)));
        assert_eq!(character('!'), Some((30, true)));
        assert_eq!(character(')'), Some((39, true)));
        assert_eq!(character('?'), Some((56, true)));
        assert_eq!(character('é'), None);
    }

    #[test]
    fn a_key_press_holds_then_lets_go_in_reverse() {
        let p = plan(&[json!({"type":"key","key":"ctrl+c","hold_ms":100})], None).unwrap();
        assert_eq!(
            inputs(&p),
            vec![
                (0, key_step(0xe0, true)),
                (2, key_step(6, true)),
                (104, key_step(6, false)),
                (106, key_step(0xe0, false)),
            ]
        );
        assert!(!p.holding);
        assert_eq!(p.end, Duration::from_millis(108));
    }

    #[test]
    fn holding_a_key_while_looking_around_is_one_plan() {
        let p = plan(
            &[
                json!({"type":"key_down","key":"w"}),
                json!({"type":"look","dx":800,"dy":-40,"ms":400}),
                json!({"type":"click","button":"left"}),
            ],
            None,
        )
        .unwrap();
        let steps = inputs(&p);
        assert_eq!(steps[0], (0, key_step(26, true)));
        let (mut dx, mut dy, mut moves) = (0i64, 0i64, 0);
        for (at, input) in &steps {
            if let ControlInput::MoveBy { dx: x, dy: y } = input {
                assert!(x.unsigned_abs() <= 2048 && y.unsigned_abs() <= 2048);
                assert!((2..=402).contains(at));
                dx += i64::from(*x);
                dy += i64::from(*y);
                moves += 1;
            }
        }
        // Exactly the asked-for movement, spread over the time like a hand's.
        assert_eq!((dx, dy, moves), (800, -40, 50));
        assert!(steps.windows(2).all(|w| w[1].0 >= w[0].0 + GAP));
        // W is still down when the plan ends: the caller keeps it held.
        assert!(p.holding);
        assert!(matches!(
            steps.last(),
            Some((
                _,
                ControlInput::Button {
                    button: 0,
                    down: false
                }
            ))
        ));
    }

    #[test]
    fn a_large_instant_look_is_split_into_reports_a_mouse_could_send() {
        let p = plan(&[json!({"type":"look","dx":-5000})], None).unwrap();
        let moved: Vec<i16> = inputs(&p)
            .iter()
            .map(|(_, i)| match i {
                ControlInput::MoveBy { dx, dy: 0 } => *dx,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(moved.len(), 3);
        assert_eq!(moved.iter().map(|d| i64::from(*d)).sum::<i64>(), -5000);
    }

    #[test]
    fn places_are_pixels_of_the_last_picture() {
        let p = plan(&[json!({"type":"click","x":1279,"y":0,"count":2})], SIZE).unwrap();
        let steps = inputs(&p);
        assert_eq!(steps[0].1, ControlInput::Move { x: 65535, y: 0 });
        assert_eq!(steps.len(), 5);
        let middle = plan(&[json!({"type":"move","x":640,"y":360})], SIZE).unwrap();
        let ControlInput::Move { x, y } = inputs(&middle)[0].1 else {
            panic!()
        };
        assert!((32700..32900).contains(&x) && (32700..32900).contains(&y));
        assert!(plan(&[json!({"type":"move","x":1280,"y":0})], SIZE)
            .unwrap_err()
            .contains("outside"));
        assert!(plan(&[json!({"type":"click","x":5,"y":5})], None)
            .unwrap_err()
            .contains("screenshot first"));
        assert!(plan(&[json!({"type":"click","x":5})], SIZE).is_err());
    }

    #[test]
    fn typing_uses_shift_only_where_the_character_needs_it() {
        let p = plan(&[json!({"type":"type","text":"aB!"})], None).unwrap();
        let steps: Vec<_> = inputs(&p).into_iter().map(|(_, i)| i).collect();
        assert_eq!(
            steps,
            vec![
                key_step(4, true),
                key_step(4, false),
                key_step(0xe1, true),
                key_step(5, true),
                key_step(5, false),
                key_step(0xe1, false),
                key_step(0xe1, true),
                key_step(30, true),
                key_step(30, false),
                key_step(0xe1, false),
            ]
        );
        assert!(plan(&[json!({"type":"type","text":"naïve"})], None).is_err());
    }

    #[test]
    fn scrolling_dragging_and_waiting() {
        let p = plan(
            &[
                json!({"type":"scroll","direction":"down","amount":2}),
                json!({"type":"wait","ms":500}),
                json!({"type":"drag","x":0,"y":0,"to_x":100,"to_y":50,"ms":80}),
            ],
            SIZE,
        )
        .unwrap();
        let steps = inputs(&p);
        assert_eq!(steps[0].1, ControlInput::Wheel { x: 0, y: -120 });
        assert_eq!(steps[1].1, ControlInput::Wheel { x: 0, y: -120 });
        assert!(steps[2].0 >= 500);
        assert_eq!(steps[2].1, ControlInput::Move { x: 0, y: 0 });
        assert_eq!(
            steps[3].1,
            ControlInput::Button {
                button: 0,
                down: true
            }
        );
        assert_eq!(
            steps.last().unwrap().1,
            ControlInput::Button {
                button: 0,
                down: false
            }
        );
        // The pointer ends on the drop point before the button is let go.
        let last_move = steps
            .iter()
            .rev()
            .find_map(|(_, i)| match i {
                ControlInput::Move { x, y } => Some((*x, *y)),
                _ => None,
            })
            .unwrap();
        let ControlInput::Move { x, y } = Builder {
            steps: vec![],
            at: 0,
            size: SIZE,
            keys: Default::default(),
            buttons: 0,
        }
        .point(100, 50)
        .unwrap() else {
            panic!()
        };
        assert_eq!(last_move, (x, y));
        assert!(!p.holding);
    }

    #[test]
    fn mistakes_are_explained_and_long_plans_refused() {
        let wrong = |v: Value| plan(&[v], SIZE).unwrap_err();
        assert!(wrong(json!({"type":"jump"})).contains("Unknown action type"));
        assert!(wrong(json!({"type":"look","dx":5,"duration":9}))
            .contains("Unknown field \"duration\""));
        assert!(wrong(json!({"type":"key","key":"win+r"})).contains("Windows key"));
        assert!(wrong(json!({"type":"click","button":"side"})).contains("Unknown button"));
        assert!(wrong(json!({"type":"scroll","direction":"north"})).contains("Unknown direction"));
        assert!(wrong(json!({"type":"key"})).contains("required"));
        assert!(wrong(json!("click")).contains("must be an object"));
        let long: Vec<_> = (0..4).map(|_| json!({"type":"wait","ms":10000})).collect();
        assert!(plan(&long, SIZE)
            .unwrap_err()
            .contains("longer than 30 seconds"));
        let many: Vec<_> = (0..101).map(|_| json!({"type":"wait","ms":1})).collect();
        assert!(plan(&many, SIZE).is_err());
        assert!(plan(&[], SIZE).unwrap().steps.is_empty());
    }

    #[test]
    fn a_picture_is_measured_from_its_own_header() {
        // SOI, an APP0 segment to skip, then a baseline frame header: 720 x 1280.
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend([0xff, 0xc0, 0x00, 0x11, 0x08, 0x02, 0xd0, 0x05, 0x00, 0x03]);
        assert_eq!(jpeg_size(&jpeg), Some((1280, 720)));
        assert_eq!(jpeg_size(&jpeg[..12]), None);
        assert_eq!(jpeg_size(b"\x89PNG\r\n\x1a\n0000"), None);
        assert_eq!(jpeg_size(&[]), None);
        // A huffman table is not a frame header.
        let tables = [0xff, 0xd8, 0xff, 0xc4, 0x00, 0x02, 0xff, 0xd9];
        assert_eq!(jpeg_size(&tables), None);
    }

    fn session(user: u64, connection: u64) -> VoiceSession {
        VoiceSession {
            user_id: Id(user),
            connection_id: Id(connection),
        }
    }
    fn asking(screens: &Screens, mcp: &[u8]) -> oneshot::Receiver<bool> {
        let (answer, answered) = oneshot::channel();
        screens.lock().insert(
            Id(2),
            Control {
                mcp: mcp.to_vec(),
                controller: session(2, 20),
                sharer: session(1, 10),
                grant: None,
                answer: Some(answer),
                frames: HashMap::new(),
                size: None,
                acting: Default::default(),
                used: now(),
            },
        );
        answered
    }

    #[test]
    fn the_calls_events_decide_what_an_agent_holds() {
        let screens = Screens::default();
        let (sharer, controller) = (session(1, 10), session(2, 20));
        let granted = |grant| VoiceEvent::ControlGranted {
            sharer,
            controller,
            grant,
            agent: true,
        };
        // A grant nobody asked for here is handed back.
        assert_eq!(screens.observe(controller, &granted(5)), Some(controller));
        let mut answered = asking(&screens, b"one");
        assert!(screens.size(b"one", Id(2)).is_err());
        // A person's grant of the same screen is not the agent's.
        let person = VoiceEvent::ControlGranted {
            sharer,
            controller,
            grant: 5,
            agent: false,
        };
        assert_eq!(screens.observe(controller, &person), None);
        assert!(answered.try_recv().is_err());
        // The event reaches every participant; it counts once.
        assert_eq!(screens.observe(sharer, &granted(7)), None);
        assert_eq!(screens.observe(controller, &granted(7)), None);
        assert_eq!(answered.try_recv(), Ok(true));
        assert_eq!(screens.current(Id(2)), Some((sharer, true)));
        assert!(screens.size(b"one", Id(2)).is_ok());
        // Another of the person's AI connections cannot use it.
        assert_eq!(
            screens.size(b"two", Id(2)).err().unwrap().code,
            "other_connection"
        );
        // Someone else's revoke leaves it; its own ends it.
        screens.observe(sharer, &VoiceEvent::ControlRevoked { sharer, grant: 8 });
        assert!(screens.current(Id(2)).is_some());
        screens.observe(sharer, &VoiceEvent::ControlRevoked { sharer, grant: 7 });
        assert!(screens.current(Id(2)).is_none());

        let mut answered = asking(&screens, b"one");
        screens.observe(session(3, 30), &VoiceEvent::ControlDenied { sharer });
        assert!(answered.try_recv().is_err());
        screens.observe(controller, &VoiceEvent::ControlDenied { sharer });
        assert_eq!(answered.try_recv(), Ok(false));
        assert!(screens.current(Id(2)).is_none());

        let mut answered = asking(&screens, b"one");
        screens.left(sharer);
        assert!(matches!(
            answered.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
    }

    #[test]
    fn only_an_unused_grant_expires() {
        let screens = Screens::default();
        let _answered = asking(&screens, b"one");
        // A question still open is the SFU's to time out, not ours.
        screens.lock().get_mut(&Id(2)).unwrap().used = 0;
        assert!(screens.expire().is_empty());
        screens.lock().get_mut(&Id(2)).unwrap().grant = Some(1);
        screens.lock().get_mut(&Id(2)).unwrap().used = now();
        assert!(screens.expire().is_empty());
        screens.lock().get_mut(&Id(2)).unwrap().used = now() - IDLE_MS - 1;
        assert_eq!(screens.expire(), vec![session(2, 20)]);
        assert!(screens.current(Id(2)).is_none());
    }
}
