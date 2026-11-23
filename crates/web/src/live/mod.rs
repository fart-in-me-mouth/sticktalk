//! Browser host. Protocol DTOs and bounded projections belong to client-core.
mod authsec;
mod calls;
mod community;
mod content;
mod expression;
mod features;
mod history_cache;
mod moderation;
mod nav;
mod operations;
mod privacy;
mod profiles;
mod stage;
mod themes;
mod threads;
use crate::wire::{decode_signal, stringify_ids};
mod ui;
use crate::inbound::{self, Inbound};
use futures_channel::mpsc;
use futures_util::{FutureExt, SinkExt, StreamExt};
use leptos::prelude::*;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
use sticktalk_client_core::{
    http::{Body, Http, HttpRequest, HttpResponse, HttpTransport},
    io::{nonce, Clock, Task},
    model::Model,
    protocol::{Frame, Transport},
    web::{Browser, Fetch},
};
use sticktalk_proto::*;
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::spawn_local;

// Root-relative host module is shipped by Trunk; it need not be embedded as a WASM snippet.
#[wasm_bindgen(raw_module = "/public/live.js")]
extern "C" {
    #[wasm_bindgen(js_name = beforeRender)]
    fn before_render();
    #[wasm_bindgen(js_name = mediaEvent)]
    fn media_event(value: &str);
    #[wasm_bindgen(js_name = mediaAction)]
    fn media_action(action: &str);
    #[wasm_bindgen(js_name = installPlatform)]
    fn install_platform();
    #[wasm_bindgen(js_name = recoverSession)]
    fn recover_session() -> js_sys::Promise;
    #[wasm_bindgen(js_name = chatReady)]
    fn chat_ready();
    #[wasm_bindgen(js_name = sessionRevoked)]
    fn session_revoked() -> js_sys::Promise;
}

/// The shared fetch adapter enforces response/chunk bounds and cancellation. The
/// server's browser header name is deliberately translated at this host boundary.
#[derive(Clone)]
struct CookieFetch(String);
impl HttpTransport for CookieFetch {
    fn execute(&self, mut request: HttpRequest) -> Task<'_, Result<HttpResponse, String>> {
        if request.method != "GET" {
            // A remotely revoked HttpOnly session cookie can outlive its app
            // state. Login still needs its readable double-submit CSRF cookie.
            let csrf = if self.0.is_empty() {
                csrf_cookie()
            } else {
                self.0.clone()
            };
            if !csrf.is_empty() {
                request.headers.push(("X-StickTalk-CSRF".into(), csrf));
            }
        }
        Box::pin(async move { Fetch::default().execute(request).await })
    }
}
type Shared = Rc<RefCell<App>>;
struct App {
    model: Model,
    csrf: String,
    origin: String,
    screen: String,
    panel: String,
    dialog: String,
    error: String,
    status: String,
    ready: bool,
    register: bool,
    signup: bool,
    google: bool,
    push: bool,
    guild: Option<Id>,
    channel: Option<Id>,
    voice_channel: Option<Id>,
    /// Direct and group calls, and the chat beside a call.
    calls: calls::Calls,
    connection: Option<mpsc::Sender<Outgoing>>,
    pending: BTreeMap<sticktalk_proto::Nonce, (Command, u64)>,
    epoch: u64,
    privacy_generation: u64,
    history_generation: u64,
    history_busy: bool,
    history_abort: Option<futures_util::future::AbortHandle>,
    extent: Option<HistoryExtent>,
    /// Recently visited conversations that reopen without a request.
    history_cache: history_cache::HistoryCache,
    window_rank: u64,
    window_ids: Vec<Id>,
    roles: BTreeMap<Id, Role>,
    devices: Vec<DeviceSession>,
    profile: Option<ProfileDetail>,
    pins: Vec<Message>,
    notifications: Vec<NotificationSettings>,
    admin_after: Option<Id>,
    admin_more: bool,
    admin_busy: bool,
    dialog_generation: u64,
    /// The form a save last succeeded in, and the dialog it was in; that form
    /// shows "Saved." until it is edited again or the dialog changes.
    saved: Option<(u64, &'static str)>,
    /// A confirmation shown in the toast; it is one while it equals `error`.
    notice: String,
    /// The server has more members of this server than are loaded.
    members_more: bool,
    /// The member list was closed on a wide window, where it is otherwise always shown.
    members_hidden: bool,
    /// The text conversation that "Open Chat" and leaving a call return to.
    last_text: Option<Id>,
    /// The last message read when the conversation was opened; "New" is drawn after it.
    new_after: Option<Id>,
    /// Which friends the Friends page lists: all, online, pending or blocked.
    friends_tab: String,
    /// What the invite typed into Join a Server leads to (or why it does not).
    invite_preview: Option<(String, Result<InvitePreview, String>)>,
    /// The invite code now in the Join field; an answer about any other is stale.
    invite_wanted: String,
    /// This session has told the server what device it is.
    device_named: bool,
    reply: Option<Id>,
    editing: Option<Id>,
    attachments: Vec<Attachment>,
    channel_attachments: BTreeMap<Id, Vec<Attachment>>,
    typing_sent: u64,
    typing_expiry_active: bool,
    query: String,
    completions: Vec<(String, String, String)>,
    completion_query: String,
    completion_busy: bool,
    gifs: Vec<GifItem>,
    gif_capability: GifCapability,
    preparing_send: bool,
    uploading: bool,
    upload_progress: String,
    upload_cancel: bool,
    revision: RwSignal<u64>,
    owner: std::rc::Weak<RefCell<App>>,
    dialog_render: Option<fn(&App) -> AnyView>,
    voice_render: Option<fn(&App) -> AnyView>,
    loading_dialog: std::cell::Cell<bool>,
    loading_voice: std::cell::Cell<bool>,
    /// Views in their own modules (the forum, search results).
    views: ui::LazyViews,
    nav: nav::NavState,
    /// Polls, forwarding, scheduled messages, voice messages, GIF favorites.
    content: content::State,
    moderation: moderation::State,
    /// Threads, forums and announcement following.
    threads: threads::State,
    /// Custom emoji everywhere, stickers, Super Reactions, typing indicators.
    expression: expression::State,
    /// Profile cards, the profile editor and profile image uploads.
    profiles: profiles::State,
    /// Status, nicknames, notes, ignore, requests, notifications, reports.
    privacy: privacy::State,
    /// Rules screening, applications, onboarding, Discover, templates, insights.
    community: community::State,
    /// Stage channels and scheduled events.
    stage: stage::State,
    /// Custom and server themes, the theme editor, shared themes.
    themes: themes::State,
}
impl App {
    fn clear_account(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.privacy_generation = self.privacy_generation.wrapping_add(1);
        if let Some(abort) = self.history_abort.take() {
            abort.abort();
        }
        self.connection = None;
        self.model = Model::default();
        self.csrf.clear();
        self.error.clear();
        self.ready = false;
        self.register = false;
        self.signup = false;
        self.status.clear();
        self.guild = None;
        self.channel = None;
        self.voice_channel = None;
        self.calls.reset();
        self.pending.clear();
        self.window_ids.clear();
        self.extent = None;
        self.history_cache.clear();
        self.history_busy = false;
        self.history_generation = self.history_generation.wrapping_add(1);
        self.typing_expiry_active = false;
        self.roles.clear();
        self.devices.clear();
        self.profile = None;
        self.profiles = Default::default();
        self.pins.clear();
        self.notifications.clear();
        self.admin_after = None;
        self.admin_more = false;
        self.admin_busy = false;
        self.dialog_generation = self.dialog_generation.wrapping_add(1);
        self.saved = None;
        self.notice.clear();
        self.members_more = false;
        self.last_text = None;
        self.new_after = None;
        self.friends_tab = "all".into();
        self.invite_preview = None;
        self.invite_wanted.clear();
        self.device_named = false;
        self.reply = None;
        self.editing = None;
        self.attachments.clear();
        self.channel_attachments.clear();
        self.gifs.clear();
        self.query.clear();
        self.completions.clear();
        self.completion_query.clear();
        self.completion_busy = false;
        self.upload_cancel = true;
        self.preparing_send = false;
        self.uploading = false;
        self.upload_progress.clear();
        self.content = Default::default();
        self.threads = Default::default();
        self.dialog.clear();
        self.panel.clear();
        self.screen = "home".into();
        self.nav = Default::default();
        self.moderation = Default::default();
        self.expression = Default::default();
        self.privacy = Default::default();
        privacy::reset();
        self.community = Default::default();
        self.stage = Default::default();
        self.themes = Default::default();
    }
    fn http(&self) -> Http<CookieFetch> {
        Http::new(&self.origin, "", CookieFetch(self.csrf.clone())).expect("same origin HTTP")
    }
    fn touch(&self) {
        if !self.dialog.is_empty()
            && self.dialog_render.is_none()
            && !self.loading_dialog.replace(true)
        {
            let owner = self.owner.clone();
            spawn_local(async move {
                let render = ui::load_dialog().await;
                if let Some(app) = owner.upgrade() {
                    app.borrow_mut().dialog_render = Some(render);
                    app.borrow().touch();
                }
            });
        }
        if self.screen == "voice"
            && self.voice_render.is_none()
            && !self.loading_voice.replace(true)
        {
            let owner = self.owner.clone();
            spawn_local(async move {
                let render = ui::load_call().await;
                if let Some(app) = owner.upgrade() {
                    app.borrow_mut().voice_render = Some(render);
                    app.borrow().touch();
                }
            });
        }
        calls::sync(self);
        before_render();
        self.revision.update(|v| *v = v.wrapping_add(1));
    }
    fn send(&mut self, message: ClientMsg) {
        if self
            .connection
            .as_mut()
            .is_none_or(|tx| tx.try_send(Outgoing::Message(message)).is_err())
        {
            self.error = "Connection unavailable. Try again when reconnected.".into();
            self.touch();
        }
    }
    fn command(&mut self, command: Command) {
        if !self.ready {
            self.error = "Reconnecting… Your draft is kept.".into();
            self.touch();
            return;
        }
        if self.pending.len() >= 128 {
            self.error = "Too many pending actions. Wait for the server.".into();
            self.touch();
            return;
        }
        let n = nonce(&Browser);
        // Kept once, for the answer and for a resend after a reconnect; the
        // socket encodes it from here.
        self.pending.insert(n, (command, Browser.monotonic_ms()));
        if self
            .connection
            .as_mut()
            .is_none_or(|tx| tx.try_send(Outgoing::Command(n)).is_err())
        {
            self.error = "Connection unavailable. Try again when reconnected.".into();
            self.touch();
        }
    }
    fn voice(&mut self, signal: VoiceSignal) {
        self.voice_request(signal, nonce(&Browser));
    }
    fn voice_request(&mut self, signal: VoiceSignal, request_nonce: u64) {
        if let Ok(payload) = rmp_serde::to_vec_named(&signal) {
            self.send(ClientMsg::Voice(VoicePayload {
                nonce: request_nonce,
                version: VOICE_VERSION,
                kind: "signal".into(),
                payload,
            }));
        }
    }
}

pub fn mount() {
    install_platform();
    leptos::mount::mount_to_body(|| {
        let revision = RwSignal::new(0);
        let app = Rc::new(RefCell::new(App {
            model: Model::default(),
            csrf: String::new(),
            origin: window().location().origin().unwrap_or_default(),
            screen: "home".into(),
            panel: String::new(),
            dialog: String::new(),
            error: String::new(),
            status: "Connecting…".into(),
            ready: false,
            register: false,
            signup: window()
                .location()
                .search()
                .unwrap_or_default()
                .contains("google=signup"),
            google: false,
            push: false,
            guild: None,
            channel: window().location().search().ok().and_then(|s| {
                s.trim_start_matches('?')
                    .split('&')
                    .find_map(|part| part.strip_prefix("channel=").and_then(id))
            }),
            connection: None,
            voice_channel: None,
            calls: Default::default(),
            pending: BTreeMap::new(),
            epoch: 0,
            privacy_generation: 0,
            history_generation: 0,
            history_busy: false,
            history_abort: None,
            extent: None,
            history_cache: Default::default(),
            window_rank: 0,
            window_ids: vec![],
            roles: BTreeMap::new(),
            devices: vec![],
            profile: None,
            pins: vec![],
            notifications: vec![],
            admin_after: None,
            admin_more: false,
            admin_busy: false,
            dialog_generation: 0,
            saved: None,
            notice: String::new(),
            members_more: false,
            members_hidden: false,
            last_text: None,
            new_after: None,
            friends_tab: "all".into(),
            invite_preview: None,
            invite_wanted: String::new(),
            device_named: false,
            reply: None,
            editing: None,
            attachments: vec![],
            channel_attachments: BTreeMap::new(),
            typing_sent: 0,
            typing_expiry_active: false,
            query: String::new(),
            completions: vec![],
            completion_query: String::new(),
            completion_busy: false,
            gifs: vec![],
            gif_capability: GifCapability::Disabled,
            preparing_send: false,
            uploading: false,
            upload_progress: String::new(),
            upload_cancel: false,
            revision,
            owner: std::rc::Weak::new(),
            dialog_render: None,
            voice_render: None,
            loading_dialog: std::cell::Cell::new(false),
            loading_voice: std::cell::Cell::new(false),
            views: Default::default(),
            nav: Default::default(),
            content: Default::default(),
            moderation: Default::default(),
            threads: Default::default(),
            expression: Default::default(),
            profiles: Default::default(),
            privacy: Default::default(),
            community: Default::default(),
            stage: Default::default(),
            themes: Default::default(),
        }));
        app.borrow_mut().owner = Rc::downgrade(&app);
        let actions = app.clone();
        let listener = Closure::<dyn Fn(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(event) = event.dyn_ref::<web_sys::CustomEvent>() else {
                return;
            };
            let Some(action) = event.detail().as_string() else {
                return;
            };
            operations::action(actions.clone(), &action);
        });
        window()
            .add_event_listener_with_callback("sticktalk-live", listener.as_ref().unchecked_ref())
            .expect("action listener");
        listener.forget();
        let boot = app.clone();
        spawn_local(async move {
            let http = boot.borrow().http();
            let metadata = boot.clone();
            let metadata_http = boot.borrow().http();
            spawn_local(async move {
                let http = metadata_http;
                if let Ok(c) = http
                    .json::<BrowserCapabilities>("/browser/capabilities")
                    .await
                {
                    let mut a = metadata.borrow_mut();
                    a.google = c.google_redirect;
                    a.push = c.web_push;
                    a.touch();
                }
                if let Ok(meta) = http.json::<MetaResponse>("/meta").await {
                    metadata.borrow_mut().gif_capability = meta.gifs;
                }
            });
            let recovered = wasm_bindgen_futures::JsFuture::from(recover_session())
                .await
                .ok()
                .and_then(|v| v.as_string())
                .and_then(|s| serde_json::from_slice::<BrowserSession>(s.as_bytes()).ok());
            let session = if let Some(session) = recovered {
                Ok(session)
            } else {
                http.json::<BrowserSession>("/browser/session").await
            };
            if let Ok(session) = session {
                start_session(boot.clone(), session);
            } else {
                // Only a real 401 means signed out. A server that cannot be
                // reached shows "Reconnecting…", not a login form that cannot work.
                let revoked = wasm_bindgen_futures::JsFuture::from(session_revoked())
                    .await
                    .ok()
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                {
                    let mut a = boot.borrow_mut();
                    a.status = if revoked {
                        String::new()
                    } else {
                        "Reconnecting…".into()
                    };
                    a.csrf = csrf_cookie();
                    a.touch();
                }
                if !revoked {
                    operations::retry_recover(boot.clone());
                }
            }
        });
        let render = StoredValue::new_local(app);
        view! { {move || {revision.get(); render.with_value(|a|ui::app(&a.borrow()))}} }
    });
}
#[inline(never)]
fn csrf_cookie() -> String {
    // Read only the non-HttpOnly CSRF cookie. Never read/store session credentials.
    js_sys::Reflect::get(document().as_ref(), &"cookie".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
        .split(';')
        .find_map(|v| {
            v.trim()
                .strip_prefix("__Host-sticktalk_csrf=")
                .map(str::to_owned)
        })
        .unwrap_or_default()
}
fn start_session(app: Shared, session: BrowserSession) {
    {
        let mut a = app.borrow_mut();
        a.epoch += 1;
        a.csrf = session.csrf_token;
        a.model.me = Some(session.user);
        a.error.clear();
        a.signup = false;
        a.touch();
    }
    let epoch = app.borrow().epoch;
    spawn_local(async move {
        socket(app, epoch).await;
    });
}

#[inline(never)]
fn end_session(app: &Shared, notice: &str) {
    // Dropping the sender wakes the socket task, which closes its transport.
    // Invalidate in-flight projections before rendering the login screen.
    let mut a = app.borrow_mut();
    a.clear_account();
    a.status = notice.into();
    media_action("transport-lost");
    media_action("purge");
    a.touch();
}

/// What the app hands the socket: a message, or the nonce of a pending command.
enum Outgoing {
    Message(ClientMsg),
    Command(sticktalk_proto::Nonce),
}
#[derive(Clone, Copy)]
struct Encoders {
    message: fn(&ClientMsg) -> Result<Vec<u8>, String>,
    command: fn(sticktalk_proto::Nonce, &Command) -> Result<Vec<u8>, String>,
}
// Keep optional command serializers out of the login/idle core. The split
// loader caches this function; the named MessagePack wire format is unchanged.
#[lazy]
fn load_message_encoder() -> Encoders {
    Encoders {
        message: encode_message,
        command: encode_command,
    }
}
use crate::wire::{encode_command, encode_message};
async fn send(
    socket: &mut sticktalk_client_core::protocol::Outgoing,
    msg: &ClientMsg,
) -> Result<(), String> {
    // Identify, heartbeats and focus need no encoder module.
    if let Some(bytes) = crate::wire::encode_small(0, crate::wire::Small::Message(msg)) {
        return send_bytes(socket, bytes).await;
    }
    let encode = load_message_encoder().await;
    send_bytes(socket, (encode.message)(msg)?).await
}
async fn send_bytes(
    socket: &mut sticktalk_client_core::protocol::Outgoing,
    bytes: Vec<u8>,
) -> Result<(), String> {
    if bytes.len() > MAX_WS_FRAME_BYTES as usize {
        return Err("Command is too large".into());
    }
    socket.send(Frame::Binary(bytes)).await
}
/// A pending command, encoded from where it is kept. None when it was answered.
async fn encode_pending(
    app: &Shared,
    nonce: sticktalk_proto::Nonce,
) -> Option<Result<Vec<u8>, String>> {
    // The subscriptions and housekeeping commands a session sends by itself.
    {
        let a = app.borrow();
        let (command, _) = a.pending.get(&nonce)?;
        if let Some(bytes) = crate::wire::encode_small(nonce, crate::wire::Small::Command(command))
        {
            return Some(Ok(bytes));
        }
    }
    let encode = load_message_encoder().await;
    let a = app.borrow();
    let (command, _) = a.pending.get(&nonce)?;
    Some((encode.command)(nonce, command))
}
async fn socket(app: Shared, epoch: u64) {
    let mut backoff = 500;
    let mut first = true;
    loop {
        if app.borrow().epoch != epoch {
            return;
        }
        let origin = app.borrow().origin.clone();
        {
            let mut a = app.borrow_mut();
            a.ready = false;
            // The first attempt is not a reconnection; nothing has gone wrong yet.
            a.status = if first {
                "Connecting…"
            } else {
                "Reconnecting…"
            }
            .into();
            a.touch();
        }
        first = false;
        let url = format!("{}/api/v1/ws", origin.replacen("http", "ws", 1));
        let connect = Browser.connect(url).fuse();
        let timeout = Browser.sleep(15_000).fuse();
        futures_util::pin_mut!(connect, timeout);
        let connection = futures_util::select! {v=connect=>v.ok(), _=timeout=>None};
        if let Some((mut outgoing, mut incoming)) = connection {
            let (tx, mut rx) = mpsc::channel(128);
            app.borrow_mut().connection = Some(tx);
            let mut heartbeat = 30_000;
            let mut deadline = Browser.monotonic_ms() + 10_000;
            let mut ack = Browser.monotonic_ms();
            let mut timeout_ms = 75_000;
            let mut identified = false;
            loop {
                if app.borrow().epoch != epoch {
                    let _ = outgoing.send(Frame::Close(None)).await;
                    return;
                }
                enum Wake {
                    Frame(Option<Result<Frame, String>>),
                    Send(Option<Outgoing>),
                    Timer,
                }
                let wake = {
                    let f = incoming.next().fuse();
                    let r = rx.next().fuse();
                    let t = Browser
                        .sleep(deadline.saturating_sub(Browser.monotonic_ms()))
                        .fuse();
                    futures_util::pin_mut!(f, r, t);
                    futures_util::select! {v=f=>Wake::Frame(v),v=r=>Wake::Send(v),_=t=>Wake::Timer}
                };
                match wake {
                    Wake::Send(Some(Outgoing::Message(msg))) => {
                        if send(&mut outgoing, &msg).await.is_err() {
                            break;
                        }
                    }
                    Wake::Send(Some(Outgoing::Command(n))) => {
                        let sent = match encode_pending(&app, n).await {
                            Some(Ok(bytes)) => send_bytes(&mut outgoing, bytes).await,
                            Some(Err(e)) => Err(e),
                            None => Ok(()),
                        };
                        if sent.is_err() {
                            break;
                        }
                    }
                    Wake::Send(None) => return,
                    Wake::Timer => {
                        if !app.borrow().ready
                            || Browser.monotonic_ms().saturating_sub(ack) > timeout_ms
                        {
                            break;
                        }
                        let seq = app.borrow().model.cursor.as_ref().map_or(0, |c| c.seq);
                        if send(
                            &mut outgoing,
                            &ClientMsg::Heartbeat(Heartbeat {
                                nonce: nonce(&Browser),
                                last_seq: seq,
                            }),
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                        deadline = Browser.monotonic_ms() + heartbeat;
                    }
                    Wake::Frame(Some(Ok(Frame::Close(Some(4001))))) => {
                        // Session revocation closes the socket without an error
                        // payload. Never leave private history visible while
                        // retrying an already invalid cookie indefinitely.
                        end_session(&app, "Your session ended. Please log in again.");
                        return;
                    }
                    Wake::Frame(Some(Ok(Frame::Binary(bytes)))) => {
                        let Ok(frame) = inbound::frame(&bytes) else {
                            app.borrow_mut().error = "Invalid server protocol frame".into();
                            break;
                        };
                        match frame {
                            Inbound::Hello(h) => {
                                if identified || h.version != PROTOCOL_VERSION {
                                    break;
                                }
                                identified = true;
                                heartbeat = u64::from(h.heartbeat_interval_ms.max(1000));
                                timeout_ms = u64::from(h.heartbeat_timeout_ms.max(1000));
                                let resume = app.borrow().model.cursor.clone();
                                if send(
                                    &mut outgoing,
                                    &ClientMsg::Identify(Identify {
                                        // Custom emoji list updates (expression.md).
                                        text_features: true,
                                        version: PROTOCOL_VERSION,
                                        token: String::new(),
                                        resume,
                                        launch_features: true,
                                    }),
                                )
                                .await
                                .is_err()
                                {
                                    break;
                                }
                            }
                            Inbound::Ready(ready) => {
                                {
                                    let mut a = app.borrow_mut();
                                    a.roles =
                                        ready.roles.iter().cloned().map(|r| (r.id, r)).collect();
                                    a.notifications = ready.notification_settings.clone();
                                    a.model.ready(*ready);
                                    a.ready = true;
                                    a.status.clear();
                                    a.history_busy = false;
                                    a.touch();
                                }
                                deadline = Browser.monotonic_ms() + heartbeat;
                                ack = Browser.monotonic_ms();
                                backoff = 500;
                                if retry_pending(&app, &mut outgoing).await.is_err() {
                                    break;
                                }
                                operations::ready(app.clone());
                                features::spawn(
                                    app.clone(),
                                    features::Feature::Nav,
                                    features::Call::Ready,
                                );
                                stage::ready(app.clone());
                                moderation::connected(app.clone());
                                privacy::connected(app.clone());
                                themes::connected(app.clone());
                                let channel = app.borrow().channel;
                                if let Some(c) = channel {
                                    operations::load_channel(app.clone(), c, None);
                                }
                                media_action("reconnected");
                                let focused =
                                    !document().hidden() && document().has_focus().unwrap_or(false);
                                let _ = send(&mut outgoing, &ClientMsg::Focus { focused }).await;
                            }
                            Inbound::Resumed => {
                                {
                                    let mut a = app.borrow_mut();
                                    a.ready = true;
                                    a.status.clear();
                                    a.touch();
                                }
                                deadline = Browser.monotonic_ms() + heartbeat;
                                ack = Browser.monotonic_ms();
                                backoff = 500;
                                if retry_pending(&app, &mut outgoing).await.is_err() {
                                    break;
                                }
                                let focused =
                                    !document().hidden() && document().has_focus().unwrap_or(false);
                                let _ = send(&mut outgoing, &ClientMsg::Focus { focused }).await;
                            }
                            Inbound::HeartbeatAck => ack = Browser.monotonic_ms(),
                            Inbound::ResyncRequired => {
                                app.borrow_mut().model.cursor = None;
                                break;
                            }
                            // A feature's event goes, undecoded, to that feature's module
                            // (routes in `inbound.rs`, modules in `features.rs`). The next
                            // frame waits for it, so the server's order is kept.
                            Inbound::Event(seq, inbound::Routed::Feature(f, raw), _) => {
                                let fresh = app
                                    .borrow()
                                    .model
                                    .cursor
                                    .as_ref()
                                    .is_some_and(|c| seq > c.seq);
                                if fresh {
                                    match features::run(app.clone(), f, features::Call::Event(raw))
                                        .await
                                    {
                                        Ok(true) => {}
                                        Ok(false) => {
                                            app.borrow_mut().error =
                                                "Invalid server protocol frame".into();
                                            break;
                                        }
                                        // Not downloaded: reconnect. The cursor has not moved,
                                        // so the resumed stream sends the event again.
                                        Err(_) => break,
                                    }
                                    if app.borrow().epoch != epoch {
                                        return;
                                    }
                                    let mut a = app.borrow_mut();
                                    a.model.advance(seq);
                                    leave_missing_channel(&mut a);
                                    a.touch();
                                }
                            }
                            Inbound::Event(seq, inbound::Routed::Core(event), size) => {
                                let event = Box::new(EventEnvelope { seq, event });
                                let mut a = app.borrow_mut();
                                let admin_access = match &event.event {
                                    Event::GuildUpsert(g) => {
                                        Some((g.guild.id, g.my_role, g.my_permissions))
                                    }
                                    _ => None,
                                };
                                let reduced_access =
                                    admin_access.is_some_and(|(id, role, permissions)| {
                                        a.model.guilds.get(&id).is_some_and(|old| {
                                            !(old.my_permissions & !permissions).is_empty()
                                                || (old.my_role == MemberRole::Owner
                                                    && role != MemberRole::Owner)
                                        })
                                    });
                                if a.model.cursor.as_ref().is_some_and(|c| event.seq > c.seq) {
                                    match &event.event {
                                        Event::MessageCreated { message, .. }
                                            if a.channel == Some(message.channel_id) =>
                                        {
                                            let tail = a.extent.as_ref().is_none_or(|e| {
                                                a.window_rank + a.window_ids.len() as u64
                                                    >= e.message_count
                                            });
                                            if let Some(extent) = a.extent.as_mut() {
                                                extent.message_count =
                                                    extent.message_count.saturating_add(1);
                                            }
                                            if tail && !a.window_ids.contains(&message.id) {
                                                a.window_ids.push(message.id);
                                                if a.window_ids.len() > 50 {
                                                    a.window_ids.remove(0);
                                                    a.window_rank += 1;
                                                }
                                            }
                                        }
                                        Event::MessageCreated { message, .. } => {
                                            a.history_cache.counted(message.channel_id, 1)
                                        }
                                        Event::MessageDeleted {
                                            channel_id,
                                            message_id,
                                        } if a.channel == Some(*channel_id) => {
                                            a.window_ids.retain(|id| id != message_id);
                                            if a.reply == Some(*message_id) {
                                                a.reply = None;
                                                a.error =
                                                    "The message you were replying to was deleted."
                                                        .into();
                                            }
                                            if let Some(extent) = a.extent.as_mut() {
                                                extent.message_count =
                                                    extent.message_count.saturating_sub(1);
                                            }
                                        }
                                        Event::MessageDeleted { channel_id, .. } => {
                                            a.history_cache.counted(*channel_id, -1)
                                        }
                                        Event::RolesUpdated { guild_id, roles } => {
                                            a.roles.retain(|_, r| r.guild_id != *guild_id);
                                            for r in roles {
                                                a.roles.insert(r.id, r.clone());
                                            }
                                        }
                                        Event::NotificationSettingsUpdated(n) => {
                                            a.notifications.retain(|v| {
                                                v.guild_id != n.guild_id
                                                    || v.channel_id != n.channel_id
                                            });
                                            a.notifications.push(n.clone());
                                        }
                                        _ => {}
                                    }
                                    // Read aloud (`/tts`, TTS notifications): themes.rs.
                                    if let Event::MessageCreated { message, .. } = &event.event {
                                        themes::message_created(&a, message);
                                    }
                                }
                                let typing = matches!(event.event, Event::Typing { .. });
                                let created = match &event.event {
                                    Event::MessageCreated { message, .. } => {
                                        Some((message.channel_id, message.author_id))
                                    }
                                    _ => None,
                                };
                                let pins_changed = matches!(&event.event, Event::PinsUpdated { channel_id, .. } if a.channel == Some(*channel_id) && a.dialog == "pins");
                                // Someone new in this server: ask whether they are online too.
                                let joined = matches!(&event.event, Event::MemberUpsert(m) if Some(m.guild_id) == a.guild && !a.model.members.contains_key(&m.user.id));
                                let revoked = matches!(
                                    &event.event,
                                    Event::ChannelRemoved { .. } | Event::GuildRemoved { .. }
                                );
                                let removed = match &event.event {
                                    Event::ChannelRemoved { channel_id, .. } => vec![*channel_id],
                                    Event::GuildRemoved { guild_id, .. } => a
                                        .model
                                        .channels
                                        .values()
                                        .filter(|c| c.guild_id == Some(*guild_id))
                                        .map(|c| c.id)
                                        .collect(),
                                    _ => vec![],
                                };
                                let profile_changed =
                                    matches!(&event.event, Event::ProfileUpdated(_));
                                let badge = matches!(
                                    &event.event,
                                    Event::ReadStateUpdated(_)
                                        | Event::MessageCreated { .. }
                                        | Event::NotificationSettingsUpdated(_)
                                        | Event::ChannelRemoved { .. }
                                        | Event::GuildRemoved { .. }
                                );
                                let applied = a.model.apply_sized(*event, size);
                                if applied && badge {
                                    privacy::badge(&a);
                                }
                                // Someone who has just sent their message is no longer typing it.
                                if let Some(sent) = created {
                                    a.model.typing.remove(&sent);
                                }
                                let arrived =
                                    applied && created.is_some_and(|(c, _)| a.channel == Some(c));
                                if let Some(g) = a.guild.filter(|_| applied && joined) {
                                    operations::subscribe(&mut a, g);
                                }
                                if applied && profile_changed {
                                    let updated = a
                                        .profile
                                        .as_ref()
                                        .and_then(|detail| {
                                            a.model.settings.profiles.get(&detail.profile.user.id)
                                        })
                                        .cloned();
                                    if let (Some(detail), Some(updated)) = (&mut a.profile, updated)
                                    {
                                        detail.profile = updated;
                                    }
                                }
                                if let Some((guild, role, permissions)) =
                                    admin_access.filter(|_| applied)
                                {
                                    if reduced_access {
                                        a.privacy_generation = a.privacy_generation.wrapping_add(1);
                                    }
                                    let admin = role == MemberRole::Owner
                                        || permissions.contains(Permissions::ADMINISTRATOR);
                                    let all_invites =
                                        admin || permissions.contains(Permissions::MANAGE_GUILD);
                                    let own_invites = all_invites
                                        || permissions.contains(Permissions::CREATE_INVITES);
                                    let me = a.model.me.as_ref().map(|me| me.user.id);
                                    a.model.invites.retain(|_, i| {
                                        i.guild_id != guild
                                            || (own_invites
                                                && (all_invites || Some(i.creator_id) == me))
                                    });
                                    if !admin && !permissions.contains(Permissions::BAN_MEMBERS) {
                                        a.model.bans.retain(|_, b| b.guild_id != guild);
                                    }
                                }
                                if applied && revoked {
                                    a.privacy_generation = a.privacy_generation.wrapping_add(1);
                                    a.completions.clear();
                                    a.completion_query.clear();
                                    a.profile = None;
                                    a.pins.clear();
                                    a.dialog.clear();
                                    for c in removed {
                                        a.channel_attachments.remove(&c);
                                        media_action(&format!("purge-channel:{}", c.0));
                                    }
                                    if a.guild.is_some_and(|g| !a.model.guilds.contains_key(&g)) {
                                        a.guild = None;
                                    }
                                    if a.voice_channel
                                        .is_some_and(|c| !a.model.channels.contains_key(&c))
                                    {
                                        a.voice_channel = None;
                                        media_action("transport-lost");
                                    }
                                }
                                leave_missing_channel(&mut a);
                                a.touch();
                                drop(a);
                                if arrived {
                                    // After this render has scrolled the list: a message
                                    // that lands in front of you is read, not unread.
                                    let app = app.clone();
                                    spawn_local(async move {
                                        Browser.sleep(50).await;
                                        operations::mark_read(&app);
                                    });
                                }
                                if applied && pins_changed {
                                    operations::action(app.clone(), "pins-refresh");
                                }
                                if typing && !app.borrow().typing_expiry_active {
                                    app.borrow_mut().typing_expiry_active = true;
                                    let app = app.clone();
                                    let epoch = app.borrow().epoch;
                                    spawn_local(async move {
                                        loop {
                                            if app.borrow().epoch != epoch {
                                                break;
                                            }
                                            let next =
                                                app.borrow().model.typing.values().min().copied();
                                            let Some(next) = next else {
                                                app.borrow_mut().typing_expiry_active = false;
                                                break;
                                            };
                                            Browser
                                                .sleep(next.saturating_sub(Browser.unix_ms()) + 1)
                                                .await;
                                            let mut a = app.borrow_mut();
                                            if a.epoch != epoch {
                                                break;
                                            }
                                            a.model
                                                .typing
                                                .retain(|_, expiry| *expiry > Browser.unix_ms());
                                            a.touch();
                                        }
                                    });
                                }
                            }
                            Inbound::Response(response) => {
                                operations::response(app.clone(), response).await
                            }
                            Inbound::Voice(payload) => {
                                // Voice-state broadcasts also reach people who are not in
                                // the call: they show who is in each voice channel. Only
                                // someone in a call has the events passed on to the media host.
                                // The empty call list sent after every Ready changes nothing
                                // while no call is known: do not load the voice module for it.
                                let idle = payload.payload == crate::wire::EMPTY_CALL_SNAPSHOT
                                    && app.borrow().calls.live.is_empty();
                                // Stage state (the channel list's live Stages, the roster)
                                // is the stage feature's, not the media host's: it goes,
                                // undecoded, to that feature's state module. The empty list
                                // sent after every Ready changes nothing while none is known.
                                let stage = (payload.version == VOICE_VERSION
                                    && payload.kind == "event")
                                    .then(|| inbound::tagged(&payload.payload).ok())
                                    .flatten()
                                    .filter(|(tag, _)| *tag == "stage")
                                    .and_then(|(_, data)| data);
                                if let Some(data) = stage {
                                    let idle = payload.payload == crate::wire::EMPTY_STAGE_SNAPSHOT
                                        && app.borrow().stage.live.is_empty();
                                    if !idle {
                                        let raw = inbound::Raw {
                                            tag: stage::VOICE_TAG,
                                            data: data.to_vec(),
                                        };
                                        let call = features::Call::Event(raw);
                                        match features::run(
                                            app.clone(),
                                            features::Feature::Stage,
                                            call,
                                        )
                                        .await
                                        {
                                            Ok(true) => {}
                                            Ok(false) => {
                                                app.borrow_mut().error =
                                                    "Invalid server protocol frame".into();
                                                break;
                                            }
                                            Err(_) => break,
                                        }
                                    }
                                } else if payload.version == VOICE_VERSION
                                    && payload.kind == "event"
                                    && !idle
                                {
                                    forward_voice(app.clone(), epoch, payload).await;
                                }
                            }
                            Inbound::Error(e) => {
                                if matches!(
                                    e.code,
                                    ErrorCode::Unauthenticated | ErrorCode::SessionExpired
                                ) {
                                    end_session(&app, "Your session ended. Please log in again.");
                                    return;
                                }
                                app.borrow_mut().error =
                                    sticktalk_client_core::http::error_text(&e);
                                app.borrow().touch();
                            }
                        }
                    }
                    _ => break,
                }
            }
            if app.borrow().epoch != epoch {
                return;
            }
            media_action("transport-lost");
            {
                let mut a = app.borrow_mut();
                a.connection = None;
                a.voice_channel = None;
                // The call ended with the connection: do not leave its empty screen up.
                if a.screen == "voice" {
                    a.channel = a.last_text.filter(|c| a.model.channels.contains_key(c));
                    a.screen = if a.channel.is_some() { "chat" } else { "home" }.into();
                    a.window_ids.clear();
                    a.extent = None;
                }
                // Events may be missed until the stream resumes or resyncs.
                a.history_cache.clear();
                a.ready = false;
                a.pending
                    .retain(|_, (_, at)| Browser.monotonic_ms().saturating_sub(*at) < 590_000);
            }
        }
        // Browser WebSocket failures hide the HTTP upgrade status. The host
        // probes only on disconnect, and only an authoritative 401 is terminal.
        let revoked = wasm_bindgen_futures::JsFuture::from(session_revoked())
            .await
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if app.borrow().epoch != epoch {
            return;
        }
        if revoked {
            end_session(&app, "Your session ended. Please log in again.");
            return;
        }
        Browser.sleep(backoff + nonce(&Browser) % 500).await;
        backoff = (backoff * 2).min(15_000);
    }
}
/// The open conversation is gone (removed, or no longer visible): go home.
fn leave_missing_channel(a: &mut App) {
    if a.channel
        .is_some_and(|c| !a.model.channels.contains_key(&c))
    {
        a.channel = None;
        a.screen = "home".into();
        a.extent = None;
        a.window_ids.clear();
        a.attachments.clear();
        a.reply = None;
        a.editing = None;
    }
}
async fn retry_pending(
    app: &Shared,
    socket: &mut sticktalk_client_core::protocol::Outgoing,
) -> Result<(), String> {
    let nonces = {
        let mut a = app.borrow_mut();
        let before = a.pending.len();
        a.pending
            .retain(|_, (_, at)| Browser.monotonic_ms().saturating_sub(*at) < 590_000);
        if a.pending.len() != before {
            a.error = "Some actions could not be confirmed. Check history before retrying.".into();
            a.touch();
        }
        a.pending.keys().copied().collect::<Vec<_>>()
    };
    for n in nonces {
        if let Some(bytes) = encode_pending(app, n).await {
            send_bytes(socket, bytes?).await?;
        }
    }
    Ok(())
}
fn value(id: &str) -> String {
    let Some(e) = document().get_element_by_id(id) else {
        return String::new();
    };
    if let Some(e) = e.dyn_ref::<web_sys::HtmlInputElement>() {
        e.value()
    } else if let Some(e) = e.dyn_ref::<web_sys::HtmlSelectElement>() {
        e.value()
    } else if let Some(e) = e.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        e.value()
    } else {
        String::new()
    }
}
fn id(value: &str) -> Option<Id> {
    value
        .parse::<u64>()
        .ok()
        .filter(|v| *v > 0 && *v <= i64::MAX as u64)
        .map(Id)
}

// Large voice DTO decoding/JSON projection is never on cold-chat's path.
#[lazy]
fn forward_voice(app: Shared, epoch: u64, payload: VoicePayload) {
    if app.borrow().epoch != epoch {
        return;
    }
    if let Ok(event) = inbound::de::<VoiceEvent>(&payload.payload) {
        // Stage state goes to the stage feature before this (`socket`).
        if matches!(event, VoiceEvent::Stage(_)) {
            return;
        }
        if matches!(
            event,
            VoiceEvent::CallState(_)
                | VoiceEvent::CallEnded { .. }
                | VoiceEvent::CallSnapshot { .. }
        ) {
            calls::event(&app, event);
            return;
        }
        if matches!(
            event,
            VoiceEvent::GuildState(_)
                | VoiceEvent::GuildLeft { .. }
                | VoiceEvent::GuildSnapshot { .. }
        ) {
            let mut a = app.borrow_mut();
            a.model.launch.voice_event(event.clone());
            a.touch();
        }
        if app.borrow().voice_channel.is_none() {
            return;
        }
        let mut v = serde_json::to_value(event).unwrap_or_default();
        stringify_ids(&mut v);
        v["request_nonce"] = payload.nonce.to_string().into();
        media_event(&v.to_string());
    }
}
