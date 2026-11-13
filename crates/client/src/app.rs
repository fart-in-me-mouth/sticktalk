use crate::{
    backend::{self, Config, Data, Delivery, Request, Session},
    media::{self, Decoded, Images},
    mock::now,
    model::{ConnectionState, Model},
    storage,
};
use iced::{
    keyboard::{self, key::Named},
    widget::{operation, scrollable, text_editor},
    window, Event as UiEvent, Subscription, Task,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};
use sticktalk_proto::*;

use sticktalk_client_core::state::ClientState;
pub use sticktalk_client_core::state::{Dialog, Form, FriendTab, GuildTab, Screen};
#[derive(Debug, Clone)]
pub enum Msg {
    Call(crate::voice_ui::Action),
    Main(crate::main_window::Message),
    Feature(Box<crate::features::Event>),
    TextFeature(Box<crate::features3::Event>),
    Settings(crate::settings_controller::Action),
    SettingsStored(Result<(), String>),
    SettingsPersist(u64),
    SettingsKey(keyboard::Key, keyboard::Modifiers),
    Keyboard(keyboard::Key, keyboard::Modifiers, iced::event::Status),
    Server(String),
    Username(String),
    Password(String),
    Display(String),
    ToggleRegister,
    RefreshMeta,
    Meta(String, Result<MetaResponse, String>),
    GoogleStart(bool),
    GoogleCancel,
    GoogleToken(u64, bool, Result<String, String>),
    GoogleCallback(u64),
    GoogleResult(u64, Result<backend::GoogleOutcome, String>),
    GoogleComplete,
    GoogleSession(u64, Result<Session, String>),
    Account(backend::AccountAction),
    Methods(u64, bool, Result<backend::AccountResult, String>),
    SavePassword,
    DeleteAccount,
    AccountDeleted(Result<(), String>),
    MemberSearch(String),
    MenuMove(bool),
    BrowserOpened(bool),
    AgentShown(bool),
    ReactPicker(Id),
    MarkUnread(Id),
    Reveal(Id, usize),
    HoverLeave(Id),
    HoverClear(u64),
    Login,
    Auth(Result<Session, String>),
    Restore(Option<Session>),
    Net(Delivery),
    Guild(Option<Id>),
    Select(Id),
    Friends(FriendTab),
    Open(Dialog),
    Close,
    Field(u8, String),
    ToggleUser(Id),
    Submit,
    Command(Command),
    Confirm(Command),
    Editor(text_editor::Action),
    Send,
    Edit(Id),
    EditLast,
    Reply(Id),
    CancelCompose,
    Reaction(Id, String, bool),
    Mention(Id),
    Suggest(i32),
    CompleteMention,
    DismissSuggestions,
    Scroll(scrollable::Viewport),
    HistoryViewport(iced::Size),
    OpenGifPicker,
    ScrollInternal,
    ScrollMonitorQuery,
    ScrollMonitorRate(u32),
    ScrollSettled,
    Measured {
        message: Id,
        generation: u64,
        width: u32,
        seq: u64,
        editing: bool,
        height: f32,
    },
    Older,
    Present,
    ReadDue(u64),
    TypingExpire,
    IdleDue,
    Activity,
    Focus(bool),
    Dismiss,
    Link(String),
    OpenLink(String),
    PreviewReply(Id),
    JumpTo(Id),
    PickUpload(UploadPurpose),
    Picked(UploadPurpose, Option<PathBuf>, u64),
    RemoveUpload(Id),
    Drop(PathBuf),
    Image(Id, u64, Result<Decoded, String>),
    Download(Attachment),
    SaveOriginal(Id, Option<PathBuf>),
    Reorder(Id, bool),
    MoreMembers,
    MoreInvites,
    MoreBans,
    FocusComposer,
    Tab(bool),
    Reconnect,
    GuildHover(Option<Option<Id>>),
    Presence(PresenceStatus),
    Logout,
    LoggedOut,
    Resize(iced::Size),
    Escape,
    Hover(Option<Id>),
    ToggleMembers,
    Copy(String),
    Emoji(String),
    Capture,
    Captured(window::Screenshot),
    CaptureSaved(Result<(), String>),
}

// Event-rate independent: high-Hz pointer deltas can each be small while the
// scrollbar is traversing many viewports/second. Do not rely only on jump size.
fn fast_scroll_delta(distance: f32, viewport: f32, elapsed: Duration) -> bool {
    distance > viewport * 0.5
        || (elapsed < Duration::from_millis(80)
            && distance / elapsed.as_secs_f32().max(0.001) > viewport * 6.)
}

type RowKey = (
    u64,
    u32,
    u64,
    Option<Id>,
    usize,
    Option<Id>,
    Option<Id>,
    usize,
    u64,
);
struct HistorySlot {
    height: f32,
    grouped: bool,
    author: Id,
    created_at: u64,
}
struct RowCache {
    key: RowKey,
    rows: Vec<(Id, f32, bool)>,
    offsets: Vec<f32>,
}

pub struct App {
    pub calls: crate::voice_ui::State,
    pub main: crate::main_window::State,
    pub extent: crate::history_extent::Window,
    pub features: crate::features::State,
    pub text_features: crate::features3::State,
    pub settings: crate::settings_controller::State,
    pub state: ClientState,
    trace: Option<PathBuf>,
    trace_counts: BTreeMap<&'static str, u64>,
    trace_total: u64,
    pub message_layouts: crate::message_layout::Cache,
    history_slots: std::cell::RefCell<BTreeMap<Id, HistorySlot>>,
    pub fast_scrolling: bool,
    scroll_settle_armed: bool,
    last_scroll: Instant,
    pub images: Images,
    pub tx: Option<backend::Handle>,
    pub server: String,
    pub username: String,
    pub password: String,
    pub display: String,
    pub register: bool,
    pub meta_loading: bool,
    pub meta_error: String,
    pub google_id: Option<String>,
    pub signup: Option<GoogleSignupRequired>,
    pub methods: Option<AuthMethodsResponse>,
    pub settings_return: Option<Dialog>,
    pub member_search: String,
    pub reaction_target: Option<Id>,
    pub revealed: BTreeSet<(Id, usize)>,
    pub markdown: crate::markdown::Cache,
    auth_serial: u64,
    auth_task: Option<iced::task::Handle>,
    mock_account: backend::MockAccount,
    hover_serial: u64,
    pub busy: bool,
    pub error: String,
    pub notice: String,
    pub dialog: Dialog,
    pub form: Form,
    pub(crate) form_revision: u64,
    pub editor: text_editor::Content,
    pub hovered_guild: Option<Option<Id>>,
    pub scroll_y: f32,
    pub viewport_h: f32,
    pub viewport_w: f32,
    pub total_h: f32,
    pub width: f32,
    pub height: f32,
    pub focused: bool,
    pub hovered: Option<Id>,
    pub members_open: Option<bool>,
    pub user_tab: u8,
    pub suggestion: usize,
    pub suggestions_dismissed: bool,
    last_typing: Option<Instant>,
    last_activity: Instant,
    idle_armed: bool,
    pub uploading: usize,
    capture: Option<PathBuf>,
    initial_screen: String,
    stats: Option<PathBuf>,
    heights: BTreeMap<Id, (u64, u32, u64, bool, f32)>,
    layout_cache: std::cell::RefCell<Option<RowCache>>,
    pub(crate) layout_revision: u64,
    pub(crate) mock_mode: bool,
    typing_armed: bool,
    media_tasks: BTreeMap<Id, iced::task::Handle>,
    resolving: BTreeSet<Id>,
    pub preview: Option<Message>,
}
impl App {
    pub fn new() -> (Self, Task<Msg>) {
        let args: Vec<_> = std::env::args().collect();
        let mock = args.iter().any(|v| v == "--mock");
        let flag = |name: &str| {
            args.iter()
                .position(|v| v == name)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        let capture = flag("--capture").map(PathBuf::from);
        let initial_screen = flag("--screen").unwrap_or_default();
        let qa_session = crate::agent_window::qa_session();
        let server = qa_session
            .as_ref()
            .map_or_else(storage::server, |s| s.config.server.clone());
        let session = if qa_session.is_some() {
            qa_session
        } else if mock
            && !matches!(
                initial_screen.as_str(),
                "login" | "register" | "google-login" | "pick-username"
            )
        {
            Some(Session {
                config: Config {
                    server: "http://127.0.0.1:8080".into(),
                    token: String::new(),
                    mock: true,
                    generation: backend::nonce(),
                    scenario: initial_screen.clone(),
                },
            })
        } else {
            None
        };
        let state = Self {
            text_features: crate::features3::State::default(),
            calls: crate::voice_ui::State::default(),
            main: crate::main_window::State::default(),
            extent: crate::history_extent::Window::default(),
            features: crate::features::State {
                crash_notice: !mock && sticktalk_desktop_platform::diagnostics::previous_crash(),
                start_minimized: !mock && storage::start_minimized(),
                ..Default::default()
            },
            settings: crate::settings_controller::State {
                preferences: crate::settings_controller::State::load_preferences(mock),
                ..Default::default()
            },
            state: ClientState {
                session,
                ..ClientState::default()
            },
            trace: flag("--trace-events").map(PathBuf::from),
            trace_counts: BTreeMap::new(),
            trace_total: 0,
            message_layouts: Default::default(),
            history_slots: Default::default(),
            fast_scrolling: false,
            scroll_settle_armed: false,
            last_scroll: Instant::now(),
            images: Images::default(),
            tx: None,
            server: server.clone(),
            username: String::new(),
            password: String::new(),
            display: String::new(),
            register: initial_screen == "register",
            meta_loading: !mock,
            meta_error: String::new(),
            google_id: mock.then(|| "mock-desktop".into()),
            signup: (initial_screen == "pick-username").then(|| GoogleSignupRequired {
                ticket: "mock-signup-ticket".into(),
                expires_at: now() + 600_000,
            }),
            methods: None,
            settings_return: None,
            member_search: String::new(),
            reaction_target: None,
            revealed: BTreeSet::new(),
            markdown: crate::markdown::Cache::default(),
            auth_serial: 0,
            auth_task: None,
            mock_account: std::sync::Arc::new(std::sync::Mutex::new(
                crate::mock::MockAuth::default(),
            )),
            hover_serial: 0,
            busy: false,
            error: String::new(),
            notice: String::new(),
            dialog: Dialog::None,
            form: Form::default(),
            form_revision: 0,
            editor: text_editor::Content::new(),
            hovered_guild: None,
            scroll_y: 0.0,
            viewport_h: crate::window_size().height
                - crate::theme::HEADER
                - crate::theme::COMPOSER_MIN
                - crate::theme::S24
                - crate::theme::TYPING
                - crate::theme::S8,
            viewport_w: crate::window_size().width
                - crate::theme::RAIL
                - crate::theme::SIDEBAR
                - if crate::window_size().width >= crate::theme::MEMBER_BREAKPOINT {
                    crate::theme::MEMBERS
                } else {
                    0.
                },
            total_h: 0.0,
            width: crate::window_size().width,
            height: crate::window_size().height,
            focused: true,
            hovered: None,
            members_open: None,
            user_tab: 0,
            suggestion: 0,
            suggestions_dismissed: false,
            last_typing: None,
            last_activity: Instant::now(),
            idle_armed: false,
            uploading: 0,
            capture,
            stats: flag("--stats").map(PathBuf::from),
            heights: BTreeMap::new(),
            layout_cache: std::cell::RefCell::new(None),
            layout_revision: 0,
            initial_screen,
            mock_mode: mock,
            typing_armed: false,
            media_tasks: BTreeMap::new(),
            resolving: BTreeSet::new(),
            preview: None,
        };
        let task = if !mock && state.state.session.is_none() {
            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        storage::token(&server).map(|token| Session {
                            config: Config {
                                server,
                                token,
                                mock: false,
                                generation: backend::nonce(),
                                scenario: String::new(),
                            },
                        })
                    })
                    .await
                    .unwrap_or(None)
                },
                Msg::Restore,
            )
        } else if state.capture.is_some() && state.state.session.is_none() {
            Task::perform(
                async {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                },
                |_| Msg::Capture,
            )
        } else {
            Task::none()
        };
        if mock && state.signup.is_some() {
            let _ = state.mock_account.lock().unwrap().google("mock-new");
        }
        let meta_task = if mock {
            Task::none()
        } else {
            let server = state.server.clone();
            Task::perform(backend::meta(server.clone(), false), move |r| {
                Msg::Meta(server.clone(), r)
            })
        };
        let agent_task = if crate::agent_window::enabled() {
            window::oldest()
                .and_then(|id| window::run(id, crate::agent_window::show_secondary))
                .map(Msg::AgentShown)
        } else {
            Task::none()
        };
        let scroll_task = window::oldest()
            .and_then(|id| window::run(id, crate::scroll::pacing::monitor_rate))
            .map(Msg::ScrollMonitorRate);
        let activation = crate::platform_glue::initial_activation().map_or_else(Task::none, |a| {
            Task::done(Msg::feature(crate::features::Event::Activation(a)))
        });
        (
            state,
            Task::batch([
                task,
                meta_task,
                agent_task,
                scroll_task,
                activation,
                crate::desktop_host::initialize(mock || crate::agent_window::enabled()),
            ]),
        )
    }
    pub fn subscription(&self) -> Subscription<Msg> {
        let mut subs = vec![
            crate::features::subscription(self),
            crate::features3::subscription(self),
            iced::event::listen_with(|event, _status, _id| match event {
                UiEvent::Window(window::Event::CloseRequested) => {
                    Some(Msg::Call(crate::voice_ui::Action::WindowClosed(_id)))
                }
                UiEvent::Window(window::Event::Resized(size)) => {
                    Some(Msg::Call(crate::voice_ui::Action::WindowResized(_id, size)))
                }

                UiEvent::Window(window::Event::Moved(_)) => Some(Msg::ScrollMonitorQuery),
                UiEvent::Window(window::Event::Focused) => {
                    Some(Msg::Call(crate::voice_ui::Action::WindowFocus(_id, true)))
                }
                UiEvent::Window(window::Event::Unfocused) => {
                    Some(Msg::Call(crate::voice_ui::Action::WindowFocus(_id, false)))
                }
                UiEvent::Window(window::Event::FileDropped(path)) => Some(Msg::Drop(path)),
                UiEvent::Window(window::Event::FileHovered(_)) => {
                    Some(Msg::Main(crate::main_window::Message::DropHover(true)))
                }
                UiEvent::Window(window::Event::FilesHoveredLeft) => {
                    Some(Msg::Main(crate::main_window::Message::DropHover(false)))
                }
                UiEvent::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                    Some(Msg::Main(crate::main_window::Message::Pointer(position)))
                }
                UiEvent::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                    Some(Msg::Keyboard(key, modifiers, _status))
                }
                UiEvent::Mouse(iced::mouse::Event::ButtonPressed(_)) => Some(Msg::Activity),
                UiEvent::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                    Some(Msg::Settings(crate::settings_controller::Action::DragEnd))
                }
                _ => None,
            }),
        ];
        if self.voice_settings_open() {
            if self.calls.recording_key {
                subs.push(iced::event::listen_with(|event, _, _| {
                    if let UiEvent::Keyboard(keyboard::Event::KeyPressed {
                        key,
                        physical_key,
                        ..
                    }) = event
                    {
                        crate::audio_settings::record_virtual_key(&key, &physical_key)
                            .filter(|key| *key != 0x1b)
                            .map(|key| Msg::Call(crate::voice_ui::Action::RecordedKey(key)))
                    } else {
                        None
                    }
                }));
            }
            if !self.state.session.as_ref().is_some_and(|s| s.config.mock) {
                subs.push(
                    Subscription::run_with(
                        crate::audio_settings::Config {
                            settings: serde_json::to_string(&self.calls.call.settings)
                                .unwrap_or_default(),
                            testing: self.calls.mic_test,
                        },
                        crate::audio_settings::subscription,
                    )
                    .map(|e| Msg::Call(crate::voice_ui::Action::Audio(e))),
                );
            }
        }
        let toast = if !self.error.is_empty()
            && matches!(self.dialog, Dialog::None)
            && self.state.session.is_some()
        {
            self.error.clone()
        } else {
            self.notice.clone()
        };
        if !toast.is_empty() {
            subs.push(Subscription::run_with(toast, |_| {
                iced::futures::stream::once(async {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    Msg::Dismiss
                })
            }));
        }
        if let Some(expiry) = self
            .state
            .model
            .launch
            .notifications
            .values()
            .filter(|n| n.muted)
            .filter_map(|n| n.mute_until)
            .filter(|t| *t > now())
            .min()
        {
            subs.push(Subscription::run_with(expiry, |expiry| {
                let expiry = *expiry;
                iced::futures::stream::once(async move {
                    tokio::time::sleep(Duration::from_millis(expiry.saturating_sub(now()))).await;
                    Msg::Main(crate::main_window::Message::NotificationExpiry)
                })
            }));
        }
        if let Some(s) = &self.state.session {
            subs.push(
                Subscription::run_with(s.config.clone(), backend::subscription).map(Msg::Net),
            );
        }
        if let Some(config) = &self.calls.config {
            if self.calls.call.settings.auto_high_quality.is_none() && !self.voice_settings_open() {
                subs.push(Subscription::run_with("audio-first-run-probe", |_| {
                    iced::futures::stream::once(async {
                        let probe = tokio::task::spawn_blocking(
                            sticktalk_media::voice::benchmark_noise_quality,
                        )
                        .await;
                        Msg::Call(crate::voice_ui::Action::Audio(match probe {
                            Ok(probe) => crate::audio_settings::Event::Probe(
                                probe.recommended
                                    == sticktalk_media::voice::NoiseSuppressionMode::HighQuality,
                            ),
                            Err(e) => crate::audio_settings::Event::Error(e.to_string()),
                        }))
                    })
                }));
            }
            subs.push(
                Subscription::run_with(config.clone(), sticktalk::voice::subscription)
                    .with(config.generation)
                    .map(|(generation, e)| {
                        Msg::Call(crate::voice_ui::Action::Media(generation, e))
                    }),
            );
        }
        let visible = crate::voice_ui::Visible(self.call_visible());
        subs.push(Subscription::run_with(visible, |v| {
            let v = v.0.clone();
            iced::futures::stream::once(
                async move { Msg::Call(crate::voice_ui::Action::Visibility(v)) },
            )
        }));
        if self.calls.picker && !self.mock_mode {
            subs.push(Subscription::run_with(self.calls.picker_revision, |_| {
                iced::futures::stream::once(async {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    Msg::Call(crate::voice_ui::Action::RefreshSources)
                })
            }));
        }
        if self.calls.save_dirty {
            subs.push(Subscription::run_with(
                self.calls.save_revision,
                |revision| {
                    let revision = *revision;
                    iced::futures::stream::once(async move {
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        Msg::Call(crate::voice_ui::Action::Save(revision))
                    })
                },
            ));
        }
        if let Some((_, deadline)) = self.calls.call.control.pending {
            subs.push(Subscription::run_with(deadline, |deadline| {
                let deadline = *deadline;
                iced::futures::stream::once(async move {
                    tokio::time::sleep(Duration::from_millis(
                        deadline.saturating_sub(crate::mock::now()),
                    ))
                    .await;
                    Msg::Call(crate::voice_ui::Action::ConsentTimeout(deadline))
                })
            }));
        }
        Subscription::batch(subs)
    }
    pub(crate) fn voice_settings_open(&self) -> bool {
        matches!(self.dialog, Dialog::UserSettings)
            && self.user_tab == crate::settings_controller::UserPage::Voice.index()
    }
    pub fn update(&mut self, message: Msg) -> Task<Msg> {
        if matches!(
            message,
            Msg::Activity | Msg::Editor(_) | Msg::Scroll(_) | Msg::Focus(_)
        ) {
            self.features.trim_epoch = self.features.trim_epoch.wrapping_add(1);
            self.features.trim_done = false;
        }
        // Row-height reports and hover cosmetics cannot change media eligibility.
        // Do not rescan/rebuild the history index once for every measured row.
        let reconcile = !matches!(
            message,
            Msg::Measured { .. } | Msg::Hover(_) | Msg::HoverLeave(_) | Msg::HoverClear(_)
        );
        let was_voice_settings = self.voice_settings_open();
        let task = self.update_main(message);
        if was_voice_settings && !self.voice_settings_open() {
            self.calls.audio_error = None;
            self.calls.audio_handle = None;
            if self.calls.recording_key {
                self.call_command(sticktalk::voice::Command::Configure(
                    self.calls.call.settings.clone(),
                ));
            }
            self.calls.mic_test = false;
            self.calls.mic_level = None;
            self.calls.recording_key = false;
        }
        if reconcile {
            Task::batch([
                task,
                crate::features::reconcile(self),
                crate::features3::reconcile(self),
            ])
        } else {
            task
        }
    }
    fn update_main(&mut self, message: Msg) -> Task<Msg> {
        if matches!(message, Msg::Escape) && self.calls.recording_key {
            self.calls.recording_key = false;
            self.call_command(sticktalk::voice::Command::Configure(
                self.calls.call.settings.clone(),
            ));
            return Task::none();
        }

        if let Some(path) = &self.trace {
            let name = match &message {
                Msg::Measured { .. } => "Measured",
                Msg::Scroll(_) => "Scroll",
                Msg::Image(..) => "Image",
                Msg::ReadDue(_) => "ReadDue",
                Msg::Hover(_) | Msg::HoverLeave(_) | Msg::HoverClear(_) => "Hover",
                Msg::Net(v) => match v.data.as_ref() {
                    Data::History { .. } => "History",
                    Data::Frame(ServerMsg::Event(_)) => "Event",
                    _ => "NetOther",
                },
                Msg::Activity => "Activity",
                Msg::TypingExpire => "TypingExpire",
                _ => "Other",
            };
            *self.trace_counts.entry(name).or_default() += 1;
            self.trace_total += 1;
            if self.trace_total.is_power_of_two() {
                let data = serde_json::json!({"counts":self.trace_counts,"total":self.trace_total,"scroll_y":self.scroll_y,"viewport_h":self.viewport_h,"viewport_w":self.viewport_w,"total_h":self.total_h,"height":self.history_height(),"retained":self.state.model.retained_count(),"heights":self.heights.len(),"cursor":self.state.model.cursor.as_ref().map(|c|c.seq)});
                let _ = std::fs::write(path, serde_json::to_vec_pretty(&data).unwrap());
            }
        }
        match message {
            Msg::Call(action) => return self.call_update(action),
            Msg::Main(m) => return self.main_update(m),
            Msg::TextFeature(event) => return crate::features3::update(self, *event),
            Msg::Feature(event) => return crate::features::update(self, *event),
            Msg::Settings(action) => return self.settings_update(action),
            Msg::Keyboard(key, modifiers, status) => {
                // Record first: existing shortcuts must not consume the chord
                // while the Keybinds page is waiting for a new binding.
                if self.settings.binding.is_some() {
                    if key == keyboard::Key::Named(Named::Escape) {
                        self.settings.binding = None;
                        return Task::none();
                    }
                    return self.update(Msg::SettingsKey(key, modifiers));
                }
                let binding = self
                    .settings
                    .preferences
                    .keybinds
                    .get("Quick Switcher")
                    .map_or("Ctrl+K", String::as_str);
                if let Some(action) = crate::main_window::key(&key, modifiers, binding) {
                    return self.main_update(action);
                }
                let message = match key {
                    keyboard::Key::Named(Named::Escape) => Msg::Escape,
                    keyboard::Key::Named(Named::Tab) if status == iced::event::Status::Ignored => {
                        Msg::Tab(modifiers.shift())
                    }
                    keyboard::Key::Named(Named::ArrowDown)
                        if status == iced::event::Status::Ignored =>
                    {
                        Msg::MenuMove(false)
                    }
                    keyboard::Key::Named(Named::ArrowUp)
                        if status == iced::event::Status::Ignored =>
                    {
                        Msg::MenuMove(true)
                    }
                    _ => Msg::SettingsKey(key, modifiers),
                };
                return self.update(message);
            }
            Msg::SettingsKey(key, modifiers) => {
                if let Some(index) = self.settings.binding {
                    let Some(chord) = crate::main_window::chord(&key, modifiers) else {
                        return Task::none();
                    };
                    self.settings.binding = None;
                    return self.settings_update(crate::settings_controller::Action::Text(
                        crate::settings_controller::TextField::Keybind(index),
                        chord,
                    ));
                }
                let binding = self
                    .settings
                    .preferences
                    .keybinds
                    .get("Quick Switcher")
                    .map_or("Ctrl+K", String::as_str);
                if let Some(action) = crate::main_window::key(&key, modifiers, binding) {
                    return self.main_update(action);
                }
                return crate::features::update(self, crate::features::Event::Key(key));
            }
            Msg::SettingsPersist(revision) => return self.settings_persist(revision),
            Msg::SettingsStored(result) => {
                if let Err(e) = result {
                    self.error = format!("Could not save settings: {e}");
                }
            }
            Msg::Hover(id) => {
                self.hover_serial += 1;
                self.hovered = id;
            }
            Msg::HoverLeave(id) => {
                if self.hovered == Some(id) {
                    self.hover_serial += 1;
                    let serial = self.hover_serial;
                    return Task::perform(
                        async {
                            tokio::time::sleep(Duration::from_millis(80)).await;
                        },
                        move |_| Msg::HoverClear(serial),
                    );
                }
            }
            Msg::HoverClear(serial) => {
                if self.hover_serial == serial {
                    self.hovered = None;
                }
            }
            Msg::RefreshMeta => {
                self.meta_loading = true;
                self.meta_error.clear();
                self.google_id = None;
                self.features.gifs.capability = GifCapability::Disabled;
                let server = self.server.trim().trim_end_matches('/').to_owned();
                return Task::perform(backend::meta(server.clone(), self.mock_mode), move |r| {
                    Msg::Meta(server.clone(), r)
                });
            }
            Msg::Meta(server, result) => {
                if server.trim_end_matches('/') == self.server.trim().trim_end_matches('/') {
                    self.meta_loading = false;
                    match result {
                        Ok(meta) => {
                            self.meta_error.clear();
                            self.google_id = meta.google_client_id;
                            self.features.gifs.capability = meta.gifs;
                            if !self.mock_mode {
                                storage::save_server(server.trim().trim_end_matches('/'));
                            }
                        }
                        Err(error) => {
                            self.meta_error = format!(
                                "Could not connect. {error}. Check the address and try again."
                            )
                        }
                    }
                }
            }
            Msg::GoogleStart(link) => {
                if crate::agent_window::enabled() && !self.mock_mode {
                    self.error = "Use mock sign-in in agent mode.".into();
                    return Task::none();
                }
                if self.busy || !self.google_available() {
                    return Task::none();
                }
                self.busy = true;
                self.error.clear();
                self.auth_serial += 1;
                let serial = self.auth_serial;
                let id = self.google_id.clone().unwrap_or_default();
                let mock = self.mock_mode;
                let server = self.server.clone();
                let session_token = if link {
                    self.state
                        .session
                        .as_ref()
                        .map(|s| s.config.token.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let stream = iced::stream::channel(2, move |mut output| async move {
                    let result = if mock {
                        Ok("mock-new".into())
                    } else {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let flow = async {
                            let challenge = backend::Http::new(&server, &session_token)?
                                .core()
                                .google_challenge(if link {
                                    AccountOperation::Link
                                } else {
                                    AccountOperation::SignIn
                                })
                                .await?;
                            crate::oauth::sign_in(id, challenge.nonce, tx).await
                        };
                        tokio::pin!(flow);
                        tokio::select! {
                            accepted=rx=>{if accepted.is_ok(){let _=futures_util::SinkExt::send(&mut output,Msg::GoogleCallback(serial)).await;}flow.await},
                            result=&mut flow=>result,
                        }
                    };
                    let _ = futures_util::SinkExt::send(
                        &mut output,
                        Msg::GoogleToken(serial, link, result),
                    )
                    .await;
                });
                let (task, handle) = Task::run(stream, |m| m).abortable();
                self.auth_task = Some(handle.abort_on_drop());
                return task;
            }
            Msg::GoogleCancel => {
                self.auth_task.take();
                self.auth_serial += 1;
                self.busy = false;
                self.signup = None;
                self.error = "Google sign-in cancelled.".into();
            }
            Msg::GoogleCallback(serial) => {
                if serial == self.auth_serial && !self.mock_mode && !crate::agent_window::enabled()
                {
                    return window::oldest()
                        .then(|id| id.map_or_else(Task::none, window::gain_focus));
                }
            }
            Msg::GoogleToken(serial, link, result) => {
                if serial != self.auth_serial {
                    return Task::none();
                }
                self.auth_task.take();
                match result {
                    Ok(token) => {
                        let task = if link {
                            let Some(session) = &self.state.session else {
                                self.busy = false;
                                return Task::none();
                            };
                            let generation = session.config.generation;
                            Task::perform(
                                backend::account(
                                    session.config.clone(),
                                    backend::AccountAction::Link(token),
                                    self.mock_account.clone(),
                                    (!self.form.name.is_empty())
                                        .then(|| std::mem::take(&mut self.form.name)),
                                ),
                                move |r| Msg::Methods(generation, false, r),
                            )
                        } else {
                            Task::perform(
                                backend::google(
                                    self.server.clone(),
                                    token,
                                    self.mock_mode,
                                    self.mock_account.clone(),
                                ),
                                move |r| Msg::GoogleResult(serial, r),
                            )
                        };
                        return task;
                    }
                    Err(e) => {
                        self.busy = false;
                        self.error = e;
                    }
                }
            }
            Msg::GoogleResult(serial, result) => {
                if serial != self.auth_serial {
                    return Task::none();
                }
                self.busy = false;
                match result {
                    Ok(backend::GoogleOutcome::Session(session)) => {
                        return self.update(Msg::Auth(Ok(session)))
                    }
                    Ok(backend::GoogleOutcome::Signup(ticket)) => {
                        self.signup = Some(ticket);
                        self.username.clear();
                        self.password.clear();
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::GoogleComplete => {
                if self.busy {
                    return Task::none();
                }
                if let Some(signup) = &self.signup {
                    if signup.expires_at <= now() {
                        self.signup = None;
                        self.error = "Signup expired. Sign in with Google again.".into();
                        return Task::none();
                    }
                    self.busy = true;
                    self.error.clear();
                    let serial = self.auth_serial;
                    return Task::perform(
                        backend::google_complete(
                            self.server.clone(),
                            signup.ticket.clone(),
                            self.username.trim().to_ascii_lowercase(),
                            self.mock_mode,
                            self.mock_account.clone(),
                        ),
                        move |r| Msg::GoogleSession(serial, r),
                    );
                }
            }
            Msg::GoogleSession(serial, result) => {
                if serial == self.auth_serial {
                    if result.is_ok() {
                        self.signup = None;
                    }
                    return self.update(Msg::Auth(result));
                }
            }
            Msg::Account(action) => {
                if self.busy {
                    return Task::none();
                }
                if let Some(session) = &self.state.session {
                    self.busy = true;
                    self.error.clear();
                    let generation = session.config.generation;
                    let password = matches!(action, backend::AccountAction::Password { .. });
                    return Task::perform(
                        backend::account(
                            session.config.clone(),
                            action,
                            self.mock_account.clone(),
                            (!self.form.name.is_empty())
                                .then(|| std::mem::take(&mut self.form.name)),
                        ),
                        move |r| Msg::Methods(generation, password, r),
                    );
                }
            }
            Msg::Methods(generation, password, result) => {
                if self
                    .state
                    .session
                    .as_ref()
                    .is_none_or(|s| s.config.generation != generation)
                {
                    return Task::none();
                }
                self.busy = false;
                match result {
                    Ok(result) => {
                        self.methods = Some(result.methods);
                        if let Some(session) = result.session {
                            return self.update(Msg::Auth(Ok(session)));
                        }
                        if password {
                            self.form.name.clear();
                            self.form.topic.clear();
                            let page = self.settings_return.take().unwrap_or(Dialog::UserSettings);
                            self.open(page);
                            self.notice = "Password saved.".into();
                        }
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::DeleteAccount => {
                if self.busy {
                    return Task::none();
                }
                let Some(session) = self.state.session.as_ref() else {
                    return Task::none();
                };
                if self
                    .state
                    .model
                    .me
                    .as_ref()
                    .is_none_or(|me| me.user.username != self.form.topic.trim())
                {
                    self.error = "Type your username to confirm permanent account deletion.".into();
                    return Task::none();
                }
                self.busy = true;
                self.error.clear();
                let password = std::mem::take(&mut self.form.name);
                return Task::perform(
                    backend::delete_account(
                        session.config.clone(),
                        (!password.is_empty()).then_some(password),
                    ),
                    Msg::AccountDeleted,
                );
            }
            Msg::AccountDeleted(result) => {
                self.busy = false;
                match result {
                    Ok(()) => return self.update(Msg::Logout),
                    Err(e) => self.error = e,
                }
            }
            Msg::SavePassword => {
                let has_password = self.methods.as_ref().is_some_and(|m| m.has_password);
                let new = std::mem::take(&mut self.form.topic);
                let current = std::mem::take(&mut self.form.name);
                if !(12..=128).contains(&new.len()) {
                    self.error = "Use a password of 12–128 bytes.".into();
                    return Task::none();
                }
                return self.update(Msg::Account(backend::AccountAction::Password {
                    current: has_password.then_some(current),
                    new,
                }));
            }
            Msg::MemberSearch(value) => self.member_search = value,
            Msg::ReactPicker(id) => {
                self.reaction_target = Some(id);
                return crate::features::update(
                    self,
                    crate::features::Event::Picker(crate::features::Picker::Emoji),
                );
            }
            Msg::Reveal(id, index) => {
                self.revealed.insert((id, index));
            }
            Msg::MarkUnread(id) => {
                if matches!(self.state.screen, Screen::Channel(_)) {
                    self.state.mark_unread(id);
                    self.dialog = Dialog::None;
                    self.notice = "Marked unread on this device.".into();
                }
            }
            Msg::ToggleMembers => {
                let current = self
                    .members_open
                    .unwrap_or(self.width >= crate::theme::MEMBER_BREAKPOINT);
                self.members_open = Some(!current);
            }
            Msg::AgentShown(ok) => {
                if !ok {
                    eprintln!(
                        "No non-primary monitor available; leaving window hidden and exiting."
                    );
                    return iced::exit();
                }
            }
            Msg::MenuMove(previous) => {
                if self.main.popover == Some(crate::main_window::Popover::Quick) {
                    return self.main_update(crate::main_window::Message::QuickMove(previous));
                }
                if self.features.picker != crate::features::Picker::None {
                    return crate::features::update(
                        self,
                        crate::features::Event::Key(keyboard::Key::Named(if previous {
                            Named::ArrowUp
                        } else {
                            Named::ArrowDown
                        })),
                    );
                }
                if matches!(
                    self.dialog,
                    Dialog::MessageActions(_) | Dialog::GuildMenu | Dialog::MemberMenu(_)
                ) {
                    return self.update(Msg::Tab(previous));
                }
            }
            Msg::BrowserOpened(ok) => {
                if !ok {
                    self.error =
                        "Could not open your browser. Check your default browser settings.".into();
                }
            }

            Msg::Copy(code) => {
                self.main.popover = None;
                if self.mock_mode || crate::agent_window::enabled() {
                    self.notice = "Copied!".into();
                    return Task::none();
                }
                self.notice = "Copied!".into();
                return iced::clipboard::write(code);
            }
            Msg::Emoji(value) => {
                self.features.picker = crate::features::Picker::None;
                self.features.emoji.record(&value);
                let saved = crate::features::persist(self);
                if let Some(id) = self.reaction_target.take() {
                    return Task::batch([saved, self.update(Msg::Reaction(id, value, false))]);
                }
                self.editor
                    .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                        std::sync::Arc::new(value),
                    )));
                self.dialog = Dialog::None;
                return Task::batch([saved, operation::focus("composer-editor")]);
            }
            Msg::Server(v) => {
                if self.busy {
                    return Task::none();
                }
                self.meta_loading = false;
                self.meta_error.clear();
                self.google_id = None;
                self.features.gifs.capability = GifCapability::Disabled;
                self.server = v;
            }
            Msg::Username(v) => self.username = v,
            Msg::Password(v) => self.password = v,
            Msg::Display(v) => self.display = v,
            Msg::ToggleRegister => {
                self.register = !self.register;
                self.error.clear();
            }
            Msg::Link(url) => {
                if reqwest::Url::parse(&url).is_ok_and(|v| matches!(v.scheme(), "https" | "http")) {
                    self.dialog = Dialog::ExternalLink(url);
                }
            }
            Msg::OpenLink(url) => {
                if self.mock_mode || crate::agent_window::enabled() {
                    self.notice = "Browser action mocked.".into();
                    self.dialog = Dialog::None;
                    return Task::none();
                }
                if !reqwest::Url::parse(&url).is_ok_and(|u| matches!(u.scheme(), "https" | "http"))
                {
                    return Task::none();
                }
                self.dialog = self.settings_return.take().unwrap_or(Dialog::None);
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || open::that(url))
                            .await
                            .is_ok_and(|v| v.is_ok())
                    },
                    Msg::BrowserOpened,
                );
            }
            Msg::JumpTo(message) => {
                self.main.popover = None;
                self.main.search.clear();
                self.jump_history(backend::HistoryAnchor::Message(message));
                if self.extent.active() {
                    return operation::scroll_to(
                        "messages",
                        scrollable::AbsoluteOffset {
                            x: 0.,
                            y: self.scroll_y,
                        },
                    );
                }
            }
            Msg::PreviewReply(message) => {
                if let Screen::Channel(channel) = self.state.screen {
                    self.preview = self
                        .state
                        .model
                        .messages
                        .get(&channel)
                        .and_then(|m| m.get(&message))
                        .cloned();
                    self.dialog = Dialog::ReplyPreview(message);
                    if self.preview.is_none() {
                        self.request(Request::Message {
                            channel,
                            message,
                            generation: self.state.generation,
                        });
                    }
                }
            }
            Msg::Dismiss => {
                self.error.clear();
                self.notice.clear();
            }
            Msg::Focus(focused) => {
                self.focused = focused;
                if focused {
                    self.activity();
                }
            }
            Msg::Login => {
                if self.mock_mode {
                    if self.username.trim().len() < 3 || self.password.len() < 12 {
                        self.error="Use a username of at least 3 characters and a password of at least 12 bytes.".into();
                        return Task::none();
                    }
                    self.password.clear();
                    self.error.clear();
                    self.state.session = Some(Session {
                        config: Config {
                            server: self.server.clone(),
                            token: self.username.clone(),
                            mock: true,
                            generation: backend::nonce(),
                            scenario: String::new(),
                        },
                    });
                    return Task::none();
                }
                if self.busy {
                    return Task::none();
                }
                self.busy = true;
                self.error.clear();
                let password = std::mem::take(&mut self.password);
                return Task::perform(
                    backend::authenticate(
                        self.server.trim().trim_end_matches('/').into(),
                        self.username.trim().to_ascii_lowercase(),
                        password,
                        self.register,
                        self.display.trim().into(),
                    ),
                    Msg::Auth,
                );
            }
            Msg::Auth(v) => {
                self.busy = false;
                match v {
                    Ok(s) => {
                        self.server = s.config.server.clone();
                        self.state.session = Some(s);
                        self.password.clear();
                        self.signup = None;
                        self.state.connection = ConnectionState::Connecting;
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::Restore(s) => {
                self.state.session = s;
                if self.state.session.is_some() {
                    self.state.connection = ConnectionState::Connecting;
                }
            }
            Msg::Net(delivery) => return self.network((*delivery.data).clone()),
            Msg::Guild(g) => {
                self.main.popover = None;
                self.state.model.active_guild = g;
                self.state.guild = g;
                self.state.model.members.clear();
                self.state.member_after = None;
                self.images.pending.clear();
                if g.is_some() {
                    let channel = self
                        .sorted_channels()
                        .into_iter()
                        .find(|c| c.kind == ChannelKind::Text)
                        .map(|c| c.id);
                    if let Some(id) = channel {
                        return self.navigate(id);
                    }
                } else {
                    self.state.screen = Screen::Friends(FriendTab::Online);
                    self.send(Command::SubscribeGuild {
                        guild_id: None,
                        presence_user_ids: vec![],
                    });
                }
            }
            Msg::Select(id) => {
                if self
                    .state
                    .model
                    .channels
                    .get(&id)
                    .is_some_and(|c| c.kind == ChannelKind::Voice)
                {
                    return self.main_update(crate::main_window::Message::Voice(id));
                }
                return self.navigate(id);
            }
            Msg::Friends(tab) => {
                self.main.popover = None;
                self.state.model.active_guild = None;
                self.state.guild = None;
                self.state.screen = Screen::Friends(tab);
                self.dialog = Dialog::None;
                self.send(Command::SubscribeGuild {
                    guild_id: None,
                    presence_user_ids: vec![],
                });
            }
            Msg::Open(dialog) => {
                if matches!(dialog, Dialog::EmojiPicker) {
                    self.reaction_target = None;
                    return crate::features::update(
                        self,
                        crate::features::Event::Picker(crate::features::Picker::Emoji),
                    );
                }
                self.features.picker = crate::features::Picker::None;
                if self.settings_dirty() {
                    return self.settings_attention();
                }
                let field = if matches!(dialog, Dialog::JoinGuild) {
                    "dialog-field-2"
                } else {
                    "dialog-field-0"
                };
                let account = matches!(dialog, Dialog::UserSettings);
                let menu = matches!(
                    dialog,
                    Dialog::MessageActions(_) | Dialog::MemberMenu(_) | Dialog::GuildMenu
                );
                self.open(dialog);
                if account {
                    return Task::batch([
                        operation::focus(field),
                        self.update(Msg::Account(backend::AccountAction::Read)),
                    ]);
                }
                if menu {
                    return self.update(Msg::Tab(false));
                }
                return operation::focus(field);
            }
            Msg::Close | Msg::Escape => {
                if self.calls.fullscreen {
                    return self.call_update(crate::voice_ui::Action::Fullscreen);
                }
                if self.calls.picker {
                    return self.call_update(crate::voice_ui::Action::ClosePicker);
                }
                if self.calls.camera_picker {
                    return self.call_update(crate::voice_ui::Action::CloseCameras);
                }
                if self.features.video.fullscreen.is_some() {
                    return crate::video::update(self, crate::video::Event::CloseFullscreen);
                }
                if self.settings.binding.take().is_some() {
                    return Task::none();
                }
                if self.settings.delete_open {
                    self.settings.delete_open = false;
                    return Task::none();
                }
                if self.settings.role_menu.take().is_some() {
                    return Task::none();
                }
                if self.settings.dragging.take().is_some() {
                    self.settings.drag_over = None;
                    return Task::none();
                }
                if self.settings_dirty() {
                    return self.settings_attention();
                }
                if matches!(self.dialog, Dialog::None) && self.features.viewer.current().is_some() {
                    self.features.viewer.close();
                    return Task::none();
                }
                if self.features.picker != crate::features::Picker::None {
                    return crate::features::update(
                        self,
                        crate::features::Event::Picker(crate::features::Picker::None),
                    );
                }
                if self.main.popover.take().is_some() {
                    return Task::none();
                }
                if matches!(message, Msg::Escape) && matches!(self.dialog, Dialog::None) {
                    self.mark_channel_read();
                }
                self.form_revision = self.form_revision.wrapping_add(1);
                if matches!(self.dialog, Dialog::CreateGuild) {
                    if let Some(id) = self.form.icon.take() {
                        self.request(Request::DiscardUpload(id));
                    }
                }
                if !matches!(self.dialog, Dialog::None) {
                    self.dialog = self.settings_return.take().unwrap_or(Dialog::None);
                    self.reaction_target = None;
                    if matches!(self.dialog, Dialog::UserSettings) {
                        self.open(Dialog::UserSettings);
                    }
                } else if self.signup.is_some() || self.busy && self.state.session.is_none() {
                    return self.update(Msg::GoogleCancel);
                } else {
                    self.cancel_compose();
                }
            }
            Msg::Field(n, v) => match n {
                0 => self.form.name = v,
                1 => self.form.topic = v,
                2 => self.form.code = v,
                _ => self.form.reason = v,
            },
            Msg::ToggleUser(id) => {
                if !self.form.selected.remove(&id) {
                    self.form.selected.insert(id);
                }
            }
            Msg::Submit => return self.submit(),
            Msg::Command(command) => {
                if matches!(
                    self.main.popover,
                    Some(crate::main_window::Popover::Context(_))
                ) {
                    self.main.popover = None;
                }
                self.send(command);
            }
            Msg::Confirm(command) => {
                self.dialog = self.settings_return.take().unwrap_or(Dialog::None);
                self.send(command);
            }
            Msg::Editor(action) => {
                let edit = action.is_edit();
                if edit {
                    self.suggestions_dismissed = false;
                    self.suggestion = 0;
                }
                if !edit
                    || self.editor.text().chars().count() < MAX_MESSAGE_CHARS
                    || matches!(
                        action,
                        text_editor::Action::Edit(
                            text_editor::Edit::Backspace | text_editor::Edit::Delete
                        )
                    )
                {
                    self.editor.perform(action);
                }
                if edit {
                    self.activity();
                    if let Screen::Channel(channel) = self.state.screen {
                        if self
                            .last_typing
                            .is_none_or(|v| v.elapsed() > Duration::from_secs(4))
                        {
                            self.send(Command::Typing {
                                channel_id: channel,
                            });
                            self.last_typing = Some(Instant::now());
                        }
                    }
                }
            }
            Msg::Send => {
                if let Screen::Channel(channel_id) = self.state.screen {
                    let content = sticktalk_client_core::features3::prepare_send(
                        &self.editor.text(),
                        crate::features3::entries(self),
                    );
                    if content.chars().count() > MAX_MESSAGE_CHARS {
                        self.error = "Messages can have up to 4,000 characters.".into();
                        return Task::none();
                    }
                    if content.trim().is_empty() && self.state.attachments.is_empty() {
                        return Task::none();
                    }
                    let mention_ids = self
                        .state
                        .selected_mentions
                        .iter()
                        .filter(|id| content.contains(&format!("<@{}>", id.0)))
                        .copied()
                        .collect();
                    let command = if let Some(message_id) = self.state.editing {
                        Command::EditMessage(MessageEdit {
                            channel_id,
                            message_id,
                            content,
                            mention_ids,
                        })
                    } else {
                        Command::SendMessage(MessageDraft {
                            channel_id,
                            content,
                            reply_to: self.state.reply,
                            attachment_ids: self.state.attachments.iter().map(|v| v.id).collect(),
                            mention_ids,
                        })
                    };
                    if self.send(command) {
                        replace_editor(&mut self.editor, "");
                        self.state.editing = None;
                        self.state.reply = None;
                        self.state.selected_mentions.clear();
                        self.state.attachments.clear();
                        self.state.drafts.remove(&channel_id);
                        self.state.at_present = true;
                        return self.jump();
                    }
                }
            }
            Msg::Edit(id) => {
                if let Screen::Channel(c) = self.state.screen {
                    if let Some(m) = self.state.model.messages.get(&c).and_then(|ms| ms.get(&id)) {
                        replace_editor(&mut self.editor, &m.content);
                        self.state.selected_mentions = m.mention_ids.iter().copied().collect();
                        self.state.editing = Some(id);
                        self.state.reply = None;
                        self.dialog = Dialog::None;
                        let mut y = crate::theme::VIRTUAL_HEADER;
                        for (m, h, _) in self.rows() {
                            if m.id == id {
                                break;
                            }
                            y += h;
                        }
                        self.scroll_y = y;
                        return Task::batch([
                            operation::scroll_to(
                                "messages",
                                scrollable::AbsoluteOffset { x: 0., y },
                            ),
                            operation::focus("composer-editor"),
                        ]);
                    }
                }
            }
            Msg::EditLast => {
                if self.editor.text().trim().is_empty() {
                    if let Screen::Channel(c) = self.state.screen {
                        if let Some(id) = self
                            .state
                            .model
                            .messages
                            .get(&c)
                            .and_then(|ms| {
                                ms.values().rev().find(|m| Some(m.author_id) == self.me())
                            })
                            .map(|m| m.id)
                        {
                            return self.update(Msg::Edit(id));
                        }
                    }
                }
            }
            Msg::Reply(id) => {
                self.state.reply = Some(id);
                self.state.editing = None;
                self.dialog = Dialog::None;
            }
            Msg::CancelCompose => self.cancel_compose(),
            Msg::Reaction(message_id, emoji, remove) => {
                self.dialog = Dialog::None;
                if let Screen::Channel(channel_id) = self.state.screen {
                    self.send(if remove {
                        Command::RemoveReaction {
                            channel_id,
                            message_id,
                            emoji,
                        }
                    } else {
                        Command::AddReaction {
                            channel_id,
                            message_id,
                            emoji,
                        }
                    });
                }
            }
            Msg::Suggest(delta) => {
                let count = self
                    .mention_candidates()
                    .len()
                    .max(self.emoji_candidates().len())
                    .max(crate::features3::candidates(self).len());
                if count > 0 {
                    self.suggestion =
                        (self.suggestion as i32 + delta).rem_euclid(count as i32) as usize;
                }
            }
            Msg::CompleteMention => {
                if let Some(e) = crate::features3::candidates(self).get(self.suggestion) {
                    let token = e.token();
                    return crate::features::update(
                        self,
                        crate::features::Event::EmojiComplete(token),
                    );
                }
                if let Some(e) = self.emoji_candidates().get(self.suggestion) {
                    return crate::features::update(
                        self,
                        crate::features::Event::EmojiComplete(e.emoji().into()),
                    );
                }
                if let Some(id) = self.mention_candidates().get(self.suggestion).copied() {
                    return self.update(Msg::Mention(id));
                }
            }
            Msg::DismissSuggestions => self.suggestions_dismissed = true,
            Msg::Mention(id) => {
                if self.state.selected_mentions.len() >= MAX_MESSAGE_MENTIONS
                    && !self.state.selected_mentions.contains(&id)
                {
                    self.error = "You can mention up to 100 people in one message.".into();
                    return Task::none();
                }
                self.state.selected_mentions.insert(id);
                self.suggestions_dismissed = true;

                let current = self.editor.text();
                let head = current
                    .rfind('@')
                    .map_or(current.as_str(), |i| &current[..i]);
                replace_editor(&mut self.editor, &format!("{}<@{}> ", head, id.0));
            }
            Msg::Measured {
                message,
                generation,
                width,
                seq,
                editing,
                height,
            } => {
                if generation != self.state.generation
                    || width != self.viewport_w.to_bits()
                    || self
                        .state
                        .model
                        .messages
                        .values()
                        .find_map(|rows| rows.get(&message))
                        .is_none_or(|m| seq != self.row_revision(m))
                    || !height.is_finite()
                    || height <= 0.
                    || height > 100_000.
                    || editing != (self.state.editing == Some(message))
                {
                    return Task::none();
                }
                // Absolute f32 layout coordinates lose subpixel precision in tall virtual lists.
                // Do not feed that rounding noise back into spacer geometry and perpetual redraws.
                if self.heights.get(&message).is_some_and(|(g, w, s, e, h)| {
                    *g == generation
                        && *w == width
                        && *s == seq
                        && *e == editing
                        && (*h - height).abs() < 0.5
                }) {
                    return Task::none();
                }
                let mut offset = crate::theme::VIRTUAL_HEADER;
                let mut old = height;
                self.ensure_rows();
                if let Some(cache) = self.layout_cache.borrow().as_ref() {
                    if let Some(index) = cache.rows.iter().position(|row| row.0 == message) {
                        offset += cache.offsets[index];
                        old = cache.rows[index].1;
                    }
                }
                self.heights
                    .insert(message, (generation, width, seq, editing, height));
                if let Some(slot) = self.history_slots.borrow_mut().get_mut(&message) {
                    slot.height = height;
                }
                self.layout_revision = self.layout_revision.wrapping_add(1);
                if let Some(cache) = self.layout_cache.borrow_mut().as_mut() {
                    if let Some(row) = cache.rows.iter_mut().find(|v| v.0 == message) {
                        row.1 = height;
                    }
                    let mut y = 0.;
                    for (index, row) in cache.rows.iter().enumerate() {
                        cache.offsets[index] = y;
                        y += row.1;
                    }
                    *cache.offsets.last_mut().unwrap() = y;
                    cache.key.8 = self.layout_revision;
                }

                while self.heights.len() > 20_000 {
                    self.heights.pop_first();
                }
                if self.extent.active() {
                    if self.extent.landed_anchor.is_some_and(|id| message < id) {
                        if let Some(top) = self.extent.window_top.as_mut() {
                            *top = (*top + old - height).max(0.);
                        }
                    }
                    return Task::none();
                }
                if (old - height).abs() > 0.5 {
                    if self.state.at_present {
                        self.scroll_y = (self.history_height() - self.viewport_h).max(0.);
                        return operation::snap_to_end("messages");
                    }
                    if offset + old < self.scroll_y {
                        self.scroll_y = (self.scroll_y + height - old).max(0.);
                        return operation::scroll_to(
                            "messages",
                            scrollable::AbsoluteOffset {
                                x: 0.,
                                y: self.scroll_y,
                            },
                        );
                    }
                }
            }
            Msg::OpenGifPicker => {
                return crate::features::update(
                    self,
                    crate::features::Event::Picker(crate::features::Picker::Gif),
                );
            }
            Msg::HistoryViewport(size) => {
                if (self.viewport_h - size.height).abs() > 0.5
                    || (self.viewport_w - size.width).abs() > 0.5
                {
                    self.viewport_h = size.height;
                    self.viewport_w = size.width;
                    if self.state.at_present {
                        return self.jump();
                    }
                }
            }
            Msg::ScrollInternal => {}
            Msg::ScrollMonitorRate(rate) => crate::scroll::pacing::set_refresh_millihertz(rate),
            Msg::ScrollMonitorQuery => {
                return window::oldest()
                    .and_then(|id| window::run(id, crate::scroll::pacing::monitor_rate))
                    .map(Msg::ScrollMonitorRate)
            }
            Msg::Scroll(v) => {
                if (self.scroll_y - v.absolute_offset().y).abs() < 0.5
                    && (self.viewport_h - v.bounds().height).abs() < 0.5
                    && self.viewport_w.to_bits() == v.bounds().width.to_bits()
                    && (self.total_h - v.content_bounds().height).abs() < 0.5
                {
                    return Task::none();
                }
                let keep_present =
                    self.state.at_present && (self.viewport_h - v.bounds().height).abs() > 0.5;
                let distance = (self.scroll_y - v.absolute_offset().y).abs();
                let rapid =
                    fast_scroll_delta(distance, self.viewport_h, self.last_scroll.elapsed());
                self.fast_scrolling |= rapid;
                if rapid && self.state.seek_loading {
                    self.state.history_seek = self.state.history_seek.wrapping_add(1);
                    self.state.seek_loading = false;
                }
                self.last_scroll = Instant::now();
                self.scroll_y = v.absolute_offset().y;
                self.viewport_h = v.bounds().height;
                self.viewport_w = v.bounds().width;
                self.total_h = v.content_bounds().height;
                if keep_present {
                    return self.jump();
                }
                if self.extent.active() {
                    self.state.at_present =
                        self.scroll_y + self.viewport_h >= self.extent.total() - 24.;
                    self.extent_landing();
                    return Task::none();
                }
                self.state.at_present = self.state.after.is_none()
                    && self.scroll_y + self.viewport_h >= self.total_h - 24.0;
                let mut tasks = vec![];
                if !self.fast_scrolling {
                    tasks.push(self.load_images());
                }
                if !self.scroll_settle_armed {
                    self.scroll_settle_armed = true;
                    tasks.push(Task::perform(
                        async {
                            tokio::time::sleep(Duration::from_millis(60)).await;
                        },
                        |_| Msg::ScrollSettled,
                    ));
                }
                tasks.push(self.defer_read());
                if !self.fast_scrolling
                    && self.scroll_y < 100.0
                    && self.state.history_more
                    && !self.state.loading
                {
                    tasks.push(self.older());
                }
                if !self.fast_scrolling
                    && self.scroll_y + self.viewport_h >= self.total_h - 100.
                    && !self.state.loading
                {
                    if let (Screen::Channel(channel), Some(after)) =
                        (self.state.screen, self.state.after)
                    {
                        self.state.loading = self.request(Request::NewerHistory {
                            seek: self.state.history_seek,
                            channel,
                            after,
                            generation: self.state.generation,
                        });
                    }
                }
                return Task::batch(tasks);
            }
            Msg::ScrollSettled => {
                let remaining =
                    Duration::from_millis(60).saturating_sub(self.last_scroll.elapsed());
                if !remaining.is_zero() {
                    return Task::perform(
                        async move {
                            tokio::time::sleep(remaining).await;
                        },
                        |_| Msg::ScrollSettled,
                    );
                }
                self.scroll_settle_armed = false;
                self.fast_scrolling = false;
                self.fetch_landing();
                return Task::batch([self.load_images(), self.defer_read()]);
            }
            Msg::Older => return self.older(),
            Msg::Present => {
                self.extent = crate::history_extent::Window::default();
                if let Screen::Channel(channel) = self.state.screen {
                    self.state.history_seek = self.state.history_seek.wrapping_add(1);
                    self.state.jump_seek = None;
                    self.state.seek_loading = false;
                    self.state.after = None;
                    self.request(Request::View);
                    self.state.model.messages.remove(&channel);
                    self.history_slots.borrow_mut().clear();
                    self.layout_cache.replace(None);
                    self.state.loading = true;
                    self.request(Request::History {
                        seek: Some(self.state.history_seek),
                        channel,
                        before: None,
                        generation: self.state.generation,
                    });
                }
                self.state.at_present = true;
                return self.jump();
            }
            Msg::ReadDue(serial) => {
                if serial == self.state.read_serial && self.focused && !self.fast_scrolling {
                    self.flush_read();
                }
            }
            Msg::TypingExpire => {
                self.typing_armed = false;
                self.state
                    .model
                    .typing
                    .retain(|_, expires| *expires > now());
                return self.typing_expiry();
            }
            Msg::IdleDue => {
                self.idle_armed = false;
                if self.state.session.is_some() {
                    if self.last_activity.elapsed() >= Duration::from_secs(300) {
                        if self
                            .state
                            .model
                            .me
                            .as_ref()
                            .is_some_and(|u| u.desired_presence == PresenceStatus::Online)
                        {
                            self.send(Command::SetPresence {
                                status: PresenceStatus::Idle,
                            });
                        }
                    } else {
                        return self.arm_idle();
                    }
                }
            }
            Msg::Activity => {
                self.activity();
                if !self.idle_armed && self.state.session.is_some() {
                    return self.arm_idle();
                }
            }
            Msg::PickUpload(purpose) => {
                if self.mock_mode || crate::agent_window::enabled() {
                    self.notice = "File selection mocked.".into();
                    return Task::none();
                }
                let form_revision = self.form_revision;
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Choose a file")
                            .pick_file()
                            .await
                            .map(|v| v.path().to_path_buf())
                    },
                    move |v| Msg::Picked(purpose, v, form_revision),
                );
            }
            Msg::Picked(purpose, Some(path), form_revision) => {
                if purpose == UploadPurpose::Message && !self.mock_mode {
                    if let Screen::Channel(channel) = self.state.screen {
                        return crate::features::update(
                            self,
                            crate::features::Event::UploadStart(channel, path, false),
                        );
                    }
                }
                if purpose == UploadPurpose::GuildIcon && form_revision != self.form_revision {
                    return Task::none();
                }
                let channel = if purpose == UploadPurpose::Message {
                    match self.state.screen {
                        Screen::Channel(c) => Some(c),
                        _ => None,
                    }
                } else {
                    None
                };
                if purpose == UploadPurpose::Message
                    && self.state.attachments.len() + self.uploading >= 4
                {
                    self.error = "You can attach up to four files.".into();
                } else {
                    self.uploading += 1;
                    self.request(Request::Upload {
                        form_revision,
                        path,
                        purpose,
                        channel,
                        generation: self.state.generation,
                    });
                }
            }
            Msg::Picked(_, None, _) => {}
            Msg::Drop(path) => {
                self.main.dropping = false;
                return self.update(Msg::Picked(
                    UploadPurpose::Message,
                    Some(path),
                    self.form_revision,
                ));
            }
            Msg::RemoveUpload(id) => {
                if let Some(job) = self
                    .features
                    .uploads
                    .iter()
                    .find(|(_, t)| t.job.attachment.as_ref().is_some_and(|a| a.id == id))
                    .map(|(id, _)| *id)
                {
                    return crate::features::update(
                        self,
                        crate::features::Event::UploadCancel(job),
                    );
                }
                self.state.attachments.retain(|v| v.id != id);
                self.request(Request::DiscardUpload(id));
            }
            Msg::Image(id, generation, result) => {
                self.media_tasks.remove(&id);
                if generation == self.state.generation {
                    match result {
                        Ok(v) => self.images.insert(v),
                        Err(_) => {
                            self.images.pending.remove(&id);
                            if self.images.failed.len() >= 64 {
                                self.images.failed.pop_first();
                            }
                            self.images.failed.insert(id);
                        }
                    }
                    return self.load_images();
                }
            }
            Msg::Download(a) => {
                if self.mock_mode || crate::agent_window::enabled() {
                    self.notice = "Save dialog mocked.".into();
                    return Task::none();
                }
                return Task::perform(
                    async move {
                        let path = rfd::AsyncFileDialog::new()
                            .set_file_name(&a.filename)
                            .save_file()
                            .await
                            .map(|v| v.path().to_path_buf());
                        (a.id, path)
                    },
                    |(id, path)| Msg::SaveOriginal(id, path),
                );
            }
            Msg::SaveOriginal(attachment, Some(path)) => {
                self.request(Request::SaveOriginal { attachment, path });
            }
            Msg::SaveOriginal(_, None) => {}
            Msg::Reorder(id, down) => self.reorder(id, down),
            Msg::Reconnect => {
                if let Some(s) = &mut self.state.session {
                    s.config.generation = backend::nonce();
                    self.state.connection = ConnectionState::Connecting;
                    self.state.pending.clear();
                    self.state.pending_files.clear();
                    self.tx = None;
                }
            }
            Msg::Tab(previous) => {
                if !matches!(self.dialog, Dialog::None) {
                    use iced::advanced::widget::{operate, operation as op};
                    return if previous {
                        operate(op::scope(
                            "dialog-scope".into(),
                            op::focusable::focus_previous(),
                        ))
                        .map(|_: ()| Msg::Activity)
                    } else {
                        operate(op::scope(
                            "dialog-scope".into(),
                            op::focusable::focus_next(),
                        ))
                        .map(|_: ()| Msg::Activity)
                    };
                }
                return if previous {
                    operation::focus_previous()
                } else {
                    operation::focus_next()
                };
            }
            Msg::FocusComposer => return operation::focus("composer-editor"),
            Msg::GuildHover(id) => self.hovered_guild = id,
            Msg::MoreInvites => {
                if let (Some(guild), Some(after)) = (self.state.guild, self.state.invite_after) {
                    self.request(Request::Invites {
                        guild,
                        after: Some(after),
                        generation: self.state.generation,
                    });
                }
            }
            Msg::MoreBans => {
                if let (Some(guild), Some(after)) = (self.state.guild, self.state.ban_after) {
                    self.request(Request::Bans {
                        guild,
                        after: Some(after),
                        generation: self.state.generation,
                    });
                }
            }
            Msg::MoreMembers => {
                if let (Some(guild), Some(after)) = (self.state.guild, self.state.member_after) {
                    self.request(Request::Members {
                        guild,
                        after: Some(after),
                        generation: self.state.generation,
                    });
                }
            }
            Msg::Presence(status) => {
                self.send(Command::SetPresence { status });
            }
            Msg::Logout => {
                let effects = self.calls.call.leave();
                self.call_effects(effects, None);
                self.main = crate::main_window::State::default();
                self.extent = crate::history_extent::Window::default();
                crate::ui::purge_main_assets();
                self.features = Default::default();
                if let Some(s) = self.state.session.take() {
                    let config = s.config.clone();
                    if !config.mock {
                        tokio::task::spawn_blocking(move || crate::video::purge(&config));
                    }
                    let account = self.me();
                    self.tx = None;
                    self.state.pending.clear();
                    self.state.pending_files.clear();
                    self.media_tasks.clear();
                    if let (Some(s), Some(id)) = (&self.state.session, self.me()) {
                        let c = s.config.clone();
                        tokio::task::spawn_blocking(move || media::purge(&c, id));
                    }
                    self.heights.clear();
                    self.history_slots.borrow_mut().clear();
                    self.layout_cache.borrow_mut().take();
                    self.state.model = Model::default();
                    self.settings = crate::settings_controller::State {
                        preferences: self.settings.preferences.clone(),
                        preference_task: self.settings.preference_task.take(),
                        preference_revision: self.settings.preference_revision,
                        ..Default::default()
                    };
                    self.settings_return = None;
                    self.state.manual_unread.clear();
                    self.methods = None;
                    self.busy = false;
                    self.auth_task.take();
                    self.auth_serial += 1;
                    self.markdown.clear();
                    self.message_layouts.clear();
                    self.revealed.clear();
                    self.images = Images::default();
                    crate::ui::purge_main_assets();
                    self.state.drafts.clear();
                    self.state.draft_mentions.clear();
                    self.state.selected_mentions.clear();
                    self.preview = None;
                    self.state.reply = None;
                    self.state.editing = None;
                    self.form = Form::default();
                    self.resolving.clear();
                    self.state.attachments.clear();
                    replace_editor(&mut self.editor, "");
                    self.dialog = Dialog::None;
                    self.state.connection = ConnectionState::Offline;
                    return Task::perform(
                        async move {
                            if !s.config.mock {
                                if let Ok(http) =
                                    backend::Http::new(&s.config.server, &s.config.token)
                                {
                                    http.logout().await;
                                }
                            }
                            let _ = tokio::task::spawn_blocking(move || {
                                storage::forget(&s.config.server);
                                if let Some(id) = account {
                                    media::purge(&s.config, id);
                                }
                            })
                            .await;
                        },
                        |_| Msg::LoggedOut,
                    );
                }
            }
            Msg::LoggedOut => self.notice.clear(),
            Msg::Resize(size) => {
                self.width = size.width;
                self.height = size.height;
                self.viewport_w = size.width
                    - crate::theme::RAIL
                    - crate::theme::SIDEBAR
                    - if self
                        .members_open
                        .unwrap_or(size.width >= crate::theme::MEMBER_BREAKPOINT)
                    {
                        crate::theme::MEMBERS
                    } else {
                        0.
                    };
                self.viewport_h = size.height
                    - crate::theme::HEADER
                    - crate::theme::COMPOSER_MIN
                    - crate::theme::S24
                    - crate::theme::TYPING
                    - crate::theme::S8;
                return self.load_images();
            }
            Msg::Capture => {
                return window::oldest()
                    .and_then(window::screenshot)
                    .map(Msg::Captured)
            }
            Msg::Captured(screenshot) => {
                if let Some(path) = self.capture.take() {
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                if let Some(parent) = path.parent() {
                                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                                }
                                image::save_buffer(
                                    path,
                                    &screenshot.rgba,
                                    screenshot.size.width,
                                    screenshot.size.height,
                                    image::ColorType::Rgba8,
                                )
                                .map_err(|e| e.to_string())
                            })
                            .await
                            .map_err(|e| e.to_string())?
                        },
                        Msg::CaptureSaved,
                    );
                }
            }
            Msg::CaptureSaved(result) => {
                if let Err(e) = result {
                    eprintln!("Screenshot: {e}");
                }
                return iced::exit();
            }
        }
        Task::none()
    }
    fn network(&mut self, data: Data) -> Task<Msg> {
        self.call_network(&data);
        match data {
            Data::Launch { data, generation } => {
                if generation == self.state.generation
                    && self.state.model.snapshot_valid(data.cursor())
                {
                    if let sticktalk_client_core::launch::SnapshotData::Extent(v) = &data {
                        self.extent_snapshot(&v.data);
                    }
                    let cursor = data.cursor().clone();
                    let acl = matches!(
                        data,
                        sticktalk_client_core::launch::SnapshotData::Overwrites(..)
                    );
                    if let sticktalk_client_core::launch::SnapshotData::Profile(v) = &data {
                        self.state
                            .model
                            .users
                            .insert(v.data.profile.user.id, v.data.profile.user.clone());
                    }
                    self.state.model.launch.snapshot(data);
                    if acl {
                        self.main_next_acl();
                    }
                    self.state.model.reconcile_metadata(&cursor);
                    if self.extent.active() && self.state.at_present {
                        return self.jump();
                    }
                }
            }
            Data::Settings {
                snapshot,
                generation,
            } => {
                if generation == self.form_revision
                    && self.state.model.snapshot_valid(&snapshot.cursor)
                {
                    let data = snapshot.data.clone();
                    if self.state.model.merge_settings(snapshot) {
                        self.settings_snapshot(&data);
                        self.settings_sync_profile();
                        self.settings_sync_server();
                    } else if matches!(
                        data,
                        sticktalk_client_core::settings::SnapshotData::Sessions(_)
                    ) {
                        self.settings_query(sticktalk_client_core::settings::Query::Sessions);
                    }
                }
            }
            Data::Connected(tx) => self.tx = Some(tx),
            Data::Rejected { nonce, reason } => {
                self.settings_response(nonce, false);
                let files = self.state.pending_files.remove(&nonce).unwrap_or_default();
                if let Some(Command::SendMessage(draft)) = self.state.pending.remove(&nonce) {
                    self.restore_send(draft, files);
                }
                self.error = reason;
            }
            Data::Uncertain(nonces) => {
                for n in nonces {
                    self.settings_response(n, false);
                    self.state.pending.remove(&n);
                    self.state.pending_files.remove(&n);
                }
                self.error="Some actions could not be confirmed within the safe retry window. Inspect the current conversation before sending again.".into();
            }

            Data::State(state) => {
                if matches!(&state,ConnectionState::Stopped(v)if v.starts_with("Session expired")) {
                    let task = self.update(Msg::Logout);
                    self.error = "Your session expired. Sign in again.".into();
                    return task;
                }
                self.state.connection = state;
            }
            Data::Frame(frame) => match frame {
                ServerMsg::Ready(ready) => {
                    self.features.desktop.settings = ready.notification_settings.clone();
                    let old = self.state.screen;
                    self.main.popover = None;
                    self.main.pending_dm = None;
                    self.state.model.ready(*ready);
                    self.main_sync_voice();
                    self.markdown.clear();
                    self.message_layouts.clear();
                    self.revealed.clear();
                    if let (Some(session), Some(account)) = (&self.state.session, self.me()) {
                        let config = session.config.clone();
                        tokio::task::spawn_blocking(move || media::maintain(&config, account));
                    }
                    self.images = Images::default();
                    crate::ui::purge_main_assets();
                    self.state.connection = ConnectionState::Ready;
                    self.state.loading = false;
                    self.state.generation += 1;
                    self.ack();
                    let mut tasks = vec![self.arm_idle()];
                    if self.initial_screen == "friends" {
                        self.state.guild = None;
                        self.state.screen = Screen::Friends(FriendTab::All);
                    } else {
                        let c = match old {
                            Screen::Channel(id) if self.state.model.channels.contains_key(&id) => {
                                Some(id)
                            }
                            _ => self
                                .state
                                .model
                                .channels
                                .values()
                                .find(|c| c.kind == ChannelKind::Text)
                                .map(|c| c.id),
                        };
                        if let Some(c) = c {
                            self.state.guild = self.state.model.channels[&c].guild_id;
                            tasks.push(self.navigate(c));
                        }
                    }
                    match self.initial_screen.as_str() {
                        "home-online" | "empty-home" => {
                            self.state.guild = None;
                            self.state.screen = Screen::Friends(FriendTab::Online);
                        }
                        "home-all" => {
                            self.state.guild = None;
                            self.state.screen = Screen::Friends(FriendTab::All);
                        }
                        "home-pending" => {
                            self.state.guild = None;
                            self.state.screen = Screen::Friends(FriendTab::Pending);
                        }
                        "home-blocked" => {
                            self.state.guild = None;
                            self.state.screen = Screen::Friends(FriendTab::Blocked);
                        }
                        "dm-chat" => tasks.push(self.navigate(Id(20))),
                        "group-chat" => tasks.push(self.navigate(Id(21))),
                        "guild-settings-overview" => {
                            self.open(Dialog::GuildSettings(GuildTab::Overview))
                        }
                        "guild-settings-channels" => {
                            self.open(Dialog::GuildSettings(GuildTab::Channels))
                        }
                        "guild-settings-members" => {
                            self.open(Dialog::GuildSettings(GuildTab::Members))
                        }
                        "guild-settings-invites" => {
                            self.open(Dialog::GuildSettings(GuildTab::Invites))
                        }
                        "guild-settings-bans" => self.open(Dialog::GuildSettings(GuildTab::Bans)),
                        "sign-in-methods" | "user-settings-account" => {
                            self.open(Dialog::UserSettings);
                            self.user_tab = 0;
                            self.methods = Some(AuthMethodsResponse {
                                has_password: true,
                                google_linked: true,
                            });
                        }
                        "user-settings-profile" => {
                            self.open(Dialog::UserSettings);
                            self.user_tab = 1;
                        }
                        "user-settings-appearance" => {
                            self.open(Dialog::UserSettings);
                            self.user_tab = 2;
                        }
                        "community-picker" => self.open(Dialog::AddGuild),
                        "create-guild-form" => self.open(Dialog::CreateGuild),
                        "join-guild" => self.open(Dialog::JoinGuild),
                        "add-friend" => self.open(Dialog::AddFriend),
                        "mention-autocomplete" => {
                            replace_editor(&mut self.editor, "@mi");
                            tasks.push(operation::focus("composer-editor"));
                        }
                        "edit-message" => {
                            self.state.editing = Some(Id(2276));
                            replace_editor(
                                &mut self.editor,
                                "I tried the first version today. It feels thoughtful.",
                            );
                        }
                        "message-hover" => self.hovered = Some(Id(2279)),

                        "dm" => tasks.push(self.navigate(Id(20))),
                        "group" => {
                            tasks.push(self.navigate(Id(21)));
                            self.open(Dialog::GroupSettings(Id(21)));
                        }
                        "channels" => self.open(Dialog::GuildSettings(GuildTab::Channels)),
                        "invites" => self.open(Dialog::GuildSettings(GuildTab::Invites)),
                        "members" => self.open(Dialog::GuildSettings(GuildTab::Members)),
                        "add-guild" => self.open(Dialog::AddGuild),
                        "message" => {
                            if let Some(m) = self.state.model.channels.get(&Id(12)) {
                                let _ = m;
                                self.open(Dialog::MessageActions(Id(2279)));
                            }
                        }
                        "invite" => self.open(Dialog::Invite),
                        "create-guild" => self.open(Dialog::AddGuild),
                        "context" => self.open(Dialog::MessageActions(Id(2279))),
                        "context-own-message" => self.open(Dialog::MessageActions(Id(2276))),
                        "masked-link-confirm" => self.open(Dialog::ExternalLink(
                            "https://example.com/release-notes".into(),
                        )),
                        "toast" => self.notice = "Invite copied.".into(),
                        "register" => {}
                        "settings" => self.open(Dialog::UserSettings),
                        "guild" => self.open(Dialog::GuildSettings(GuildTab::Overview)),
                        "pending" => {
                            self.state.guild = None;
                            self.state.screen = Screen::Friends(FriendTab::Pending)
                        }
                        _ => {}
                    }
                    self.initial_screen.clear();
                    if crate::agent_window::qa_session().is_some() {
                        if let Ok(value) = std::env::var("STICKTALK_QA_PREFERENCES") {
                            if let Ok(mut preferences) = serde_json::from_str::<
                                sticktalk_client_core::settings::Preferences,
                            >(&value)
                            {
                                preferences.validate();
                                self.settings.preferences = preferences;
                                self.settings.preference_revision =
                                    self.settings.preference_revision.wrapping_add(1);
                                tasks
                                    .push(self.settings_persist(self.settings.preference_revision));
                            }
                        }
                        if let Ok(path) = std::env::var("STICKTALK_QA_PREFERENCES_REPORT") {
                            let bytes = serde_json::to_vec(&self.settings.preferences)
                                .expect("validated QA preferences");
                            tasks.push(Task::perform(
                                async move {
                                    tokio::fs::write(path, bytes)
                                        .await
                                        .map_err(|error| error.to_string())
                                },
                                Msg::SettingsStored,
                            ));
                        }
                        if let Ok(channel) = std::env::var("STICKTALK_QA_CHANNEL") {
                            if let Ok(channel) = channel.parse::<u64>() {
                                if self.state.model.channels.contains_key(&Id(channel)) {
                                    tasks.push(self.navigate(Id(channel)));
                                }
                            }
                        }
                        if let Ok(content) = std::env::var("STICKTALK_QA_SEND") {
                            replace_editor(&mut self.editor, &content);
                            tasks.push(Task::done(Msg::Send));
                        }
                    }
                    if self.capture.is_some() {
                        tasks.push(Task::perform(
                            async {
                                tokio::time::sleep(Duration::from_secs(2)).await;
                            },
                            |_| Msg::Capture,
                        ));
                    }
                    return Task::batch(tasks);
                }
                ServerMsg::Event(envelope) => {
                    if self
                        .state
                        .model
                        .cursor
                        .as_ref()
                        .is_some_and(|c| envelope.seq > c.seq)
                    {
                        self.features.desktop.observe(&envelope.event);
                        if let Event::MessageCreated { message, .. } = &envelope.event {
                            crate::features::committed(self, message);
                            self.features.desktop.default_level =
                                Some(self.settings.preferences.notification_level);
                            if let Some(toast) = self.features.desktop.notification(
                                &self.state.model,
                                message,
                                self.focused,
                                self.state.screen == Screen::Channel(message.channel_id),
                            ) {
                                self.features.flash_pending =
                                    message.mention_ids.contains(&self.me().unwrap_or(Id(0)));
                                crate::platform_glue::dispatch(
                                    toast,
                                    self.features.desktop.dry_run,
                                    crate::platform_glue::sound_requested(
                                        self.settings.preferences.notification_sounds,
                                        self.focused,
                                    ),
                                );
                            }
                        }
                    }
                    let removed = matches!(
                        envelope.event,
                        Event::GuildRemoved { .. } | Event::ChannelRemoved { .. }
                    );
                    let typing = matches!(envelope.event, Event::Typing { .. });
                    let media_changed = matches!(
                        envelope.event,
                        Event::SelfUpdated(_) | Event::UserUpdated(_) | Event::GuildUpsert(_)
                    );
                    let messages = matches!(
                        envelope.event,
                        Event::MessageCreated { .. }
                            | Event::MessageUpdated(_)
                            | Event::MessageDeleted { .. }
                    );
                    if let Event::MessageDeleted {
                        channel_id,
                        message_id,
                        ..
                    } = &envelope.event
                    {
                        if self.state.screen == Screen::Channel(*channel_id)
                            && self
                                .state
                                .model
                                .cursor
                                .as_ref()
                                .is_some_and(|c| envelope.seq > c.seq)
                        {
                            self.history_slots.borrow_mut().remove(message_id);
                        }
                    }
                    self.main_live_event(&envelope.event);
                    let sessions_changed = matches!(envelope.event, Event::SessionsUpdated);
                    self.state.model.apply(*envelope);
                    self.settings_sync_profile();
                    self.settings_sync_server();
                    if sessions_changed {
                        self.settings_query(sticktalk_client_core::settings::Query::Sessions);
                    }
                    self.ack();
                    if removed {
                        self.features.video = Default::default();
                        if let Some(config) = self
                            .state
                            .session
                            .as_ref()
                            .map(|s| s.config.clone())
                            .filter(|c| !c.mock)
                        {
                            tokio::task::spawn_blocking(move || crate::video::purge(&config));
                        }
                        self.state.generation += 1;
                        self.message_layouts.clear();
                        self.markdown.clear();
                        self.media_tasks.clear();
                        self.preview = None;
                        self.state
                            .drafts
                            .retain(|id, _| self.state.model.channels.contains_key(id));
                        self.state.pending.retain(|_, c| match c {
                            Command::SendMessage(v) => {
                                self.state.model.channels.contains_key(&v.channel_id)
                            }
                            Command::EditMessage(v) => {
                                self.state.model.channels.contains_key(&v.channel_id)
                            }
                            _ => true,
                        });
                        self.state
                            .pending_files
                            .retain(|nonce, _| self.state.pending.contains_key(nonce));
                        self.images = Images::default();
                        crate::ui::purge_main_assets();
                        if let (Some(s), Some(id)) = (&self.state.session, self.me()) {
                            let c = s.config.clone();
                            tokio::task::spawn_blocking(move || media::purge(&c, id));
                        }
                        if let Screen::Channel(c) = self.state.screen {
                            if !self.state.model.channels.contains_key(&c) {
                                self.history_slots.borrow_mut().clear();
                                self.layout_cache.replace(None);
                                self.state.screen = Screen::Friends(FriendTab::Online);
                                self.state.guild = None;
                            }
                        }
                    }
                    if media_changed {
                        return self.load_images();
                    }
                    if typing {
                        return self.typing_expiry();
                    }
                    if messages {
                        let t = self.load_images();
                        if self.state.at_present {
                            return Task::batch([t, self.jump()]);
                        }
                        return t;
                    }
                }
                ServerMsg::Response(response) => {
                    self.settings_response(
                        response.nonce,
                        matches!(response.outcome, CommandOutcome::Ok(_)),
                    );
                    let command = self.state.pending.remove(&response.nonce);
                    let files = self
                        .state
                        .pending_files
                        .remove(&response.nonce)
                        .unwrap_or_default();
                    match response.outcome {
                        CommandOutcome::Err(e) => {
                            self.error = format!(
                                "{}{}",
                                e.message,
                                e.retry_after_ms.map_or(String::new(), |ms| format!(
                                    " Try again in {}s.",
                                    ms.div_ceil(1000)
                                ))
                            );
                            if let Some(Command::SendMessage(d)) = command {
                                self.restore_send(d, files);
                            }
                        }
                        CommandOutcome::Ok(result) => {
                            if !response.replayed {
                                match result {
                                    CommandResult::Channel(c) => {
                                        let id = c.summary.id;
                                        if matches!(
                                            command,
                                            Some(
                                                Command::OpenDm { .. }
                                                    | Command::CreateGroupDm { .. }
                                            )
                                        ) {
                                            if let Some((user, content)) =
                                                self.main.pending_dm.take()
                                            {
                                                if c.summary.recipient_ids.contains(&user) {
                                                    self.send(Command::SendMessage(MessageDraft {
                                                        channel_id: id,
                                                        content,
                                                        reply_to: None,
                                                        attachment_ids: vec![],
                                                        mention_ids: vec![],
                                                    }));
                                                }
                                            }
                                            self.main.popover = None;
                                            self.state.guild = None;
                                            return self.navigate(id);
                                        }
                                    }
                                    CommandResult::Guild(d) => {
                                        if matches!(
                                            command,
                                            Some(
                                                Command::CreateGuild { .. }
                                                    | Command::JoinGuild { .. }
                                            )
                                        ) {
                                            return self
                                                .update(Msg::Guild(Some(d.summary.guild.id)));
                                        }
                                    }
                                    CommandResult::Role(role) => {
                                        if matches!(
                                            command,
                                            Some(Command::Launch(LaunchCommand::CreateRole { .. }))
                                        ) {
                                            return self.settings_update(
                                                crate::settings_controller::Action::EditRole(
                                                    role.id,
                                                ),
                                            );
                                        }
                                    }
                                    CommandResult::Invite(v) => {
                                        self.form.code = v.code;
                                        self.notice="Invite created. Share the code with someone you trust.".into();
                                    }
                                    CommandResult::Subscription { presences } => {
                                        for p in presences {
                                            self.state.model.presences.insert(p.user_id, p.status);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                ServerMsg::ResyncRequired(_) => {
                    self.media_tasks.clear();
                    if let (Some(s), Some(id)) = (&self.state.session, self.me()) {
                        let c = s.config.clone();
                        tokio::task::spawn_blocking(move || media::purge(&c, id));
                    }
                    self.heights.clear();
                    self.history_slots.borrow_mut().clear();
                    self.layout_cache.borrow_mut().take();
                    self.state.model = Model::default();
                    self.images = Images::default();
                    crate::ui::purge_main_assets();
                    self.state.connection = ConnectionState::Connecting;
                    self.state.loading = false;
                }
                ServerMsg::Resumed(v) => {
                    if let Some(cursor) = self.state.model.cursor.as_mut() {
                        if cursor.stream_id == v.cursor.stream_id {
                            cursor.seq = cursor.seq.max(v.cursor.seq);
                        }
                    }
                    self.state.connection = ConnectionState::Ready;
                    self.ack();
                    self.subscribe();
                }
                ServerMsg::Voice(v) => self.main_voice(v),
                ServerMsg::Error(e) => self.error = e.message,
                _ => {}
            },
            Data::NewerHistory {
                seek,
                channel,
                page,
                generation,
            } => {
                if generation != self.state.generation
                    || seek != self.state.history_seek
                    || self.state.screen != Screen::Channel(channel)
                {
                    return Task::none();
                }
                if self.state.newer_history(channel, page) {
                    self.layout_revision = self.layout_revision.wrapping_add(1);
                    return self.load_images();
                }
            }
            Data::AroundHistory {
                channel,
                anchor,
                seek,
                page,
                generation,
            } => {
                if generation != self.state.generation
                    || seek != self.state.history_seek
                    || self.state.screen != Screen::Channel(channel)
                {
                    return Task::none();
                }
                if let Some(task) = self.extent_page(channel, seek, page.clone()) {
                    return task;
                }
                self.state.seek_loading = false;
                if !self.state.model.snapshot_valid(&page.cursor) {
                    self.error = "History changed while loading. Please try again.".into();
                    return Task::none();
                }
                if self.state.jump_seek != Some(seek) {
                    // Reload evicted bodies in-place. Retain compact slots/heights so
                    // a landing fetch cannot shrink history or move the scrollbar.
                    let keep: BTreeSet<_> = self.visible_items().iter().map(|r| r.0).collect();
                    if !page.items.iter().any(|m| keep.contains(&m.id)) {
                        self.error = "This history region is no longer available.".into();
                        return Task::none();
                    }
                    if let Some(messages) = self.state.model.messages.get_mut(&channel) {
                        messages.retain(|id, _| keep.contains(id));
                    }
                    self.state.model.merge_page(channel, page);
                    self.layout_revision = self.layout_revision.wrapping_add(1);
                    self.fetch_landing();
                    return Task::batch([self.load_images(), self.defer_read()]);
                }
                let target = self.state.replace_history(channel, anchor, page);
                self.history_slots.borrow_mut().clear();
                self.layout_cache.replace(None);
                self.layout_revision = self.layout_revision.wrapping_add(1);
                self.scroll_y = crate::theme::VIRTUAL_HEADER
                    + self
                        .rows()
                        .iter()
                        .take_while(|(m, _, _)| m.id < target)
                        .map(|(_, h, _)| h)
                        .sum::<f32>();
                self.scroll_y = (self.scroll_y - self.viewport_h / 2.).max(0.);
                return Task::batch([
                    self.load_images(),
                    operation::scroll_to(
                        "messages",
                        scrollable::AbsoluteOffset {
                            x: 0.,
                            y: self.scroll_y,
                        },
                    ),
                ]);
            }
            Data::History {
                seek,
                channel,
                page,
                generation,
            } => {
                if generation != self.state.generation
                    || seek.is_some_and(|serial| serial != self.state.history_seek)
                {
                    return Task::none();
                }
                let old_height = self.history_height();
                let initial = self
                    .state
                    .model
                    .messages
                    .get(&channel)
                    .is_none_or(BTreeMap::is_empty);
                if !self.state.older_history(channel, page) {
                    self.error =
                        "History changed while loading. Reopen this channel to refresh.".into();
                    return Task::none();
                }
                self.layout_revision = self.layout_revision.wrapping_add(1);
                if self.extent.active() && self.state.at_present {
                    self.extent.start_rank =
                        self.extent.count.saturating_sub(self.rows().len() as u64);
                    self.extent.loaded = true;
                    self.extent.ends_at_present = true;
                }
                let images = self.load_images();
                if self.state.session.as_ref().is_some_and(|s| {
                    s.config.mock
                        && matches!(s.config.scenario.as_str(), "stress-scroll" | "scroll-5000")
                }) {
                    self.state.at_present = false;
                    self.scroll_y = 0.;
                    if self.state.history_more {
                        self.state.loading = true;
                        self.request(Request::History {
                            seek: Some(self.state.history_seek),
                            channel,
                            before: self.state.before,
                            generation: self.state.generation,
                        });
                    } else if let Some(path) = self.stats.take() {
                        let (start, end, _, _) = self.visible_range();
                        let stats = serde_json::json!({"messages_seen":self.state.history_seen,"retained_messages":self.state.model.retained_count(),"retained_bytes":self.state.model.retained_bytes(),"rendered_message_widgets":end-start});
                        tokio::task::spawn_blocking(move || {
                            std::fs::write(path, serde_json::to_vec_pretty(&stats).unwrap())
                        });
                    }
                    return images;
                }
                if initial || self.state.at_present {
                    return Task::batch([images, self.jump()]);
                }
                let added = (self.history_height() - old_height).max(0.0);
                self.scroll_y += added;
                return Task::batch([
                    images,
                    operation::scroll_to(
                        "messages",
                        scrollable::AbsoluteOffset {
                            x: 0.0,
                            y: self.scroll_y,
                        },
                    ),
                ]);
            }
            Data::Channel {
                snapshot,
                generation,
            } => {
                if generation == self.state.generation
                    && self.state.model.snapshot_valid(&snapshot.cursor)
                    && self
                        .state
                        .model
                        .channels
                        .contains_key(&snapshot.data.summary.id)
                {
                    let channel = snapshot.data.summary.id;
                    let old = self
                        .state
                        .model
                        .topics
                        .get(&channel)
                        .cloned()
                        .flatten()
                        .unwrap_or_default();
                    if self.settings.channel == Some(channel) && self.settings.channel_topic == old
                    {
                        self.settings.channel_topic =
                            snapshot.data.topic.clone().unwrap_or_default();
                    }
                    self.state.model.topics.insert(channel, snapshot.data.topic);
                    self.state.model.reconcile_metadata(&snapshot.cursor);
                }
            }
            Data::Members {
                guild,
                after,
                page,
                generation,
            } => {
                if self.state.member_page(guild, after, page, generation) {
                    if matches!(self.dialog, Dialog::GuildSettings(GuildTab::Roles)) {
                        if let Some(next) = self.state.member_after {
                            self.request(Request::Members {
                                guild,
                                after: Some(next),
                                generation,
                            });
                        }
                    }
                    self.subscribe();
                    return self.load_images();
                }
            }
            Data::Invites {
                guild,
                after,
                page,
                generation,
            } => {
                self.state.invite_page(guild, after, page, generation);
            }
            Data::Bans {
                guild,
                after,
                page,
                generation,
            } => {
                self.state.ban_page(guild, after, page, generation);
            }
            Data::Uploaded {
                form_revision,
                response,
                purpose,
                channel,
                generation,
            } => {
                self.uploading = self.uploading.saturating_sub(1);
                if generation != self.state.generation
                    || purpose == UploadPurpose::GuildIcon && form_revision != self.form_revision
                {
                    self.request(Request::DiscardUpload(response.attachment.id));
                    return Task::none();
                }
                match purpose {
                    UploadPurpose::Message => {
                        if matches!(self.state.screen,Screen::Channel(c)if Some(c)==channel) {
                            self.state.attachments.push(response.attachment);
                        }
                    }
                    UploadPurpose::Avatar => {
                        self.send(Command::UpdateProfile(ProfilePatch {
                            avatar_id: Patch::Set(response.attachment.id),
                            ..ProfilePatch::default()
                        }));
                    }
                    UploadPurpose::GuildIcon => {
                        if matches!(self.dialog, Dialog::CreateGuild) {
                            self.form.icon = Some(response.attachment.id);
                        } else if let (Dialog::GuildSettings(GuildTab::Overview), Some(guild_id)) =
                            (&self.dialog, self.state.guild)
                        {
                            let _ = guild_id;
                            self.settings.server.icon = Some(response.attachment.id);
                            return self.load_images();
                        } else {
                            self.request(Request::DiscardUpload(response.attachment.id));
                        }
                    }
                }
            }
            Data::Users {
                snapshot,
                generation,
            } => {
                if generation == self.state.generation
                    && self.state.model.snapshot_valid(&snapshot.cursor)
                {
                    for user in snapshot.data {
                        self.resolving.remove(&user.id);
                        self.state.model.users.insert(user.id, user);
                    }
                    self.state.model.reconcile_metadata(&snapshot.cursor);
                    self.state.model.compact_users();
                }
            }
            Data::Message {
                snapshot,
                generation,
            } => {
                if generation == self.state.generation
                    && self.state.model.snapshot_valid(&snapshot.cursor)
                {
                    self.preview = Some(snapshot.data);
                }
            }
            Data::Saved => self.notice = "File saved.".into(),
            Data::Error(e) => {
                self.state.loading = false;
                self.state.seek_loading = false;
                self.uploading = 0;
                if !e.is_empty() {
                    self.error = e;
                }
            }
        }
        Task::none()
    }
    pub fn me(&self) -> Option<Id> {
        self.state.model.me.as_ref().map(|u| u.user.id)
    }
    pub fn permissions(&self) -> Permissions {
        self.state.model.permissions(self.state.guild)
    }
    fn restore_send(&mut self, draft: MessageDraft, files: Vec<Attachment>) {
        if !self.state.model.channels.contains_key(&draft.channel_id) {
            return;
        }
        if self.state.screen == Screen::Channel(draft.channel_id)
            && self.editor.text().trim().is_empty()
        {
            replace_editor(&mut self.editor, &draft.content);
            self.state.selected_mentions = draft.mention_ids.iter().copied().collect();
            self.state.reply = draft.reply_to;
            if self.state.attachments.is_empty() {
                self.state.attachments = files;
            }
        } else if !draft.content.is_empty() {
            if self.state.drafts.len() >= 4 {
                self.state.drafts.pop_first();
            }
            self.state
                .draft_mentions
                .entry(draft.channel_id)
                .or_insert_with(|| draft.mention_ids.iter().copied().collect());
            self.state
                .drafts
                .entry(draft.channel_id)
                .or_insert(draft.content);
            self.state
                .draft_mentions
                .retain(|id, _| self.state.drafts.contains_key(id));
        }
    }
    pub fn request(&mut self, r: Request) -> bool {
        if let Some(tx) = &self.tx {
            match tx.try_send(r) {
                Ok(()) => return true,
                Err(error) => {
                    self.error = error;
                    return false;
                }
            }
        }
        self.error = "Connection busy or unavailable. Please retry.".into();
        false
    }
    pub fn send(&mut self, command: Command) -> bool {
        if self.state.connection != ConnectionState::Ready {
            self.error = "You are offline. This action was not queued.".into();
            return false;
        }
        let required = match &command {
            Command::SendMessage(m) => Some((
                m.channel_id,
                Permissions::SEND_MESSAGES
                    | if m.attachment_ids.is_empty() {
                        Permissions::empty()
                    } else {
                        Permissions::ATTACH_FILES
                    },
            )),
            Command::EditMessage(m) => Some((m.channel_id, Permissions::SEND_MESSAGES)),
            Command::Typing { channel_id } => Some((*channel_id, Permissions::SEND_MESSAGES)),
            Command::AddReaction { channel_id, .. }
            | Command::RemoveReaction { channel_id, .. } => {
                Some((*channel_id, Permissions::ADD_REACTIONS))
            }
            Command::Launch(LaunchCommand::PinMessage { channel_id, .. }) => {
                Some((*channel_id, Permissions::MANAGE_MESSAGES))
            }
            _ => None,
        };
        if required.is_some_and(|(channel, permission)| {
            !self.channel_permissions(channel).contains(permission)
        }) {
            self.error = "You don't have permission to do that in this channel.".into();
            return false;
        }
        if self.state.pending.len() >= 32 {
            self.error = "Please wait for pending actions to finish.".into();
            return false;
        }
        let nonce = backend::nonce();
        if self.request(Request::Command(CommandRequest {
            nonce,
            command: command.clone(),
        })) {
            if let Command::SendMessage(draft) = &command {
                let files: Vec<_> = self
                    .state
                    .attachments
                    .iter()
                    .filter(|a| draft.attachment_ids.contains(&a.id))
                    .cloned()
                    .collect();
                if !files.is_empty() {
                    self.state.pending_files.insert(nonce, files);
                }
            }
            self.state.pending.insert(nonce, command);
            true
        } else {
            false
        }
    }
    fn ack(&mut self) {
        if let Some(cursor) = self.state.model.cursor.clone() {
            self.request(Request::Applied(cursor));
        }
    }
    fn navigate(&mut self, id: Id) -> Task<Msg> {
        if self
            .state
            .model
            .channels
            .get(&id)
            .is_some_and(|c| c.kind == ChannelKind::Voice)
        {
            return self.call_update(crate::voice_ui::Action::Join(id));
        }
        if !self.state.model.channels.contains_key(&id) {
            return Task::none();
        }
        self.flush_read();
        if let Screen::Channel(old) = self.state.screen {
            let draft = self.editor.text();
            if !draft.is_empty() {
                if self.state.drafts.len() >= 4 {
                    self.state.drafts.pop_first();
                }
                self.state.drafts.insert(old, draft);
                self.state
                    .draft_mentions
                    .insert(old, self.state.selected_mentions.clone());
                self.state
                    .draft_mentions
                    .retain(|id, _| self.state.drafts.contains_key(id));
            }
        }
        for a in std::mem::take(&mut self.state.attachments) {
            if self
                .features
                .uploads
                .values()
                .any(|t| t.job.attachment.as_ref().is_some_and(|v| v.id == a.id))
            {
                continue;
            }
            self.request(Request::DiscardUpload(a.id));
        }
        self.extent = crate::history_extent::Window::default();
        let old_guild = self.state.guild;
        self.state.guild = self.state.model.channels[&id].guild_id;
        self.state.manual_unread.remove(&id);
        self.state.screen = Screen::Channel(id);
        self.dialog = Dialog::None;
        self.features.picker = crate::features::Picker::None;
        self.features.viewer.close();
        self.state.generation += 1;
        self.media_tasks.clear();
        self.resolving.clear();
        self.uploading = 0;
        self.request(Request::View);
        self.state.loading = true;
        self.state.history_more = false;
        self.state.before = None;
        self.state.after = None;
        self.scroll_y = 0.0;
        self.state.at_present = true;
        self.state.read_id = None;
        self.state.reply = None;
        self.state.editing = None;
        self.history_slots.borrow_mut().clear();
        self.layout_cache.replace(None);
        self.fast_scrolling = false;
        self.state.seek_loading = false;
        self.state.jump_seek = None;
        self.state.history_seek = self.state.history_seek.wrapping_add(1);
        self.images.pending.clear();
        replace_editor(
            &mut self.editor,
            self.state.drafts.get(&id).map_or("", String::as_str),
        );
        self.state.selected_mentions = self
            .state
            .draft_mentions
            .get(&id)
            .cloned()
            .unwrap_or_default();
        self.state.model.touch(id);
        self.state.model.enforce_bounds();
        self.request(Request::History {
            seek: Some(self.state.history_seek),
            channel: id,
            before: None,
            generation: self.state.generation,
        });
        self.request(Request::Channel {
            channel: id,
            generation: self.state.generation,
        });
        if self.state.guild != old_guild || self.state.model.members.is_empty() {
            self.state.model.members.clear();
            if let Some(guild) = self.state.guild {
                self.request(Request::Members {
                    guild,
                    after: None,
                    generation: self.state.generation,
                });
            }
        }
        self.main_channel(id);
        self.subscribe();
        self.jump()
    }
    fn subscribe(&mut self) {
        if self.state.connection != ConnectionState::Ready {
            return;
        }
        let ids = if self.state.guild.is_some() {
            self.state.model.members.keys().take(100).copied().collect()
        } else {
            vec![]
        };
        self.send(Command::SubscribeGuild {
            guild_id: self.state.guild,
            presence_user_ids: ids,
        });
    }
    fn jump_history(&mut self, anchor: backend::HistoryAnchor) {
        if let Screen::Channel(channel) = self.state.screen {
            self.dialog = Dialog::None;
            self.state.history_seek = self.state.history_seek.wrapping_add(1);
            self.state.jump_seek = Some(self.state.history_seek);
            self.state.at_present = false;
            if self.extent.active() {
                match anchor {
                    backend::HistoryAnchor::Message(id) => {
                        let rank = self
                            .extent
                            .anchors
                            .iter()
                            .rev()
                            .find(|a| a.message_id <= id)
                            .map_or(0, |a| a.rank);
                        self.scroll_y = (rank as f32 * crate::theme::HISTORY_ROW
                            - self.viewport_h / 2.)
                            .clamp(0., (self.extent.total() - self.viewport_h).max(0.));
                        self.extent.loaded = false;
                        self.extent.landing_y = None;
                        self.extent.ends_at_present = false;
                        self.extent.pending = Some((
                            self.state.history_seek,
                            sticktalk_proto::HistoryAnchor {
                                rank,
                                message_id: id,
                            },
                        ));
                    }
                    backend::HistoryAnchor::Timestamp(_) => {
                        self.extent = crate::history_extent::Window::default()
                    }
                }
            }
            self.request(Request::View);
            self.state.seek_loading = self.request(Request::AroundHistory {
                channel,
                anchor,
                seek: self.state.history_seek,
                generation: self.state.generation,
            });
        }
    }
    fn fetch_landing(&mut self) {
        if self.state.seek_loading {
            return;
        }
        let Screen::Channel(channel) = self.state.screen else {
            return;
        };
        let visible = self.visible_items();
        let Some(first_missing) = visible.iter().position(|(_, m, _, _)| m.is_none()) else {
            return;
        };
        let center = (first_missing + 24).min(visible.len() - 1);
        let anchor = backend::HistoryAnchor::Message(visible[center].0);
        self.state.history_seek = self.state.history_seek.wrapping_add(1);
        self.state.seek_loading = self.request(Request::AroundHistory {
            seek: self.state.history_seek,
            channel,
            anchor,
            generation: self.state.generation,
        });
    }
    fn older(&mut self) -> Task<Msg> {
        if !self.state.loading && self.state.history_more {
            if let Screen::Channel(channel) = self.state.screen {
                self.state.loading = self.request(Request::History {
                    seek: Some(self.state.history_seek),
                    channel,
                    before: self.state.before,
                    generation: self.state.generation,
                });
            }
        }
        Task::none()
    }
    pub fn pending_messages(&self) -> impl Iterator<Item = (Nonce, &MessageDraft)> {
        self.state.pending.iter().filter_map(|(nonce, command)| {
            match (self.state.screen, command) {
                (Screen::Channel(channel), Command::SendMessage(draft))
                    if draft.channel_id == channel =>
                {
                    Some((*nonce, draft))
                }
                _ => None,
            }
        })
    }
    pub fn sorted_channels(&self) -> Vec<&ChannelSummary> {
        let mut root: Vec<_> = self
            .state
            .model
            .channels
            .values()
            .filter(|c| c.guild_id == self.state.guild && c.parent_id.is_none())
            .collect();
        root.sort_by_key(|c| (c.position, c.id));
        let mut out = vec![];
        for c in root {
            out.push(c);
            if c.kind == ChannelKind::Category {
                let mut children: Vec<_> = self
                    .state
                    .model
                    .channels
                    .values()
                    .filter(|v| v.parent_id == Some(c.id))
                    .collect();
                children.sort_by_key(|v| (v.position, v.id));
                out.extend(children);
            }
        }
        out
    }
    pub fn mention_candidates(&self) -> Vec<Id> {
        if self.suggestions_dismissed {
            return vec![];
        }
        self.state.model.mention_candidates(&self.editor.text())
    }
    fn previous_row(&self, m: &Message) -> Option<(Id, Id, u64)> {
        let loaded = self.state.model.messages.get(&m.channel_id).and_then(|ms| {
            ms.range(..m.id)
                .next_back()
                .map(|(id, m)| (*id, m.author_id, m.created_at))
        });
        let indexed = self
            .history_slots
            .borrow()
            .range(..m.id)
            .next_back()
            .map(|(id, slot)| (*id, slot.author, slot.created_at));
        loaded.into_iter().chain(indexed).max_by_key(|v| v.0)
    }
    pub fn row_decorations(&self, m: &Message) -> (bool, bool) {
        let previous = self.previous_row(m);
        let date = previous.is_none_or(|(_, _, time)| {
            crate::datetime::local(time).date() != crate::datetime::local(m.created_at).date()
        });
        let unread = self.state.model.reads.get(&m.channel_id).is_some_and(|r| {
            r.unread_count > 0
                && r.last_read_id.is_some_and(|id| {
                    m.id > id && previous.is_none_or(|(previous, _, _)| previous <= id)
                })
        });
        (date, unread)
    }
    fn grouped_row(&self, m: &Message) -> bool {
        self.previous_row(m).is_some_and(|(_, author, time)| {
            author == m.author_id
                && m.created_at.saturating_sub(time) < crate::theme::GROUP_MS
                && crate::datetime::local(m.created_at).date()
                    == crate::datetime::local(time).date()
        })
    }
    pub fn row_revision(&self, m: &Message) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        m.content.hash(&mut hash);
        m.edited_at.hash(&mut hash);
        m.attachments.len().hash(&mut hash);
        m.reactions.len().hash(&mut hash);
        self.row_decorations(m).hash(&mut hash);
        self.grouped_row(m).hash(&mut hash);
        self.settings
            .preferences
            .chat_font_scale
            .to_bits()
            .hash(&mut hash);
        matches!(
            self.settings.preferences.density,
            sticktalk_client_core::settings::Density::Compact
        )
        .hash(&mut hash);
        self.settings.preferences.underline_links.hash(&mut hash);
        hash.finish()
    }
    fn ensure_rows(&self) {
        use crate::theme as t;
        let Screen::Channel(channel) = self.state.screen else {
            return;
        };
        let Some(messages) = self.state.model.messages.get(&channel) else {
            return;
        };
        let key = (
            self.state.generation,
            self.viewport_w.to_bits(),
            self.state.model.layout_revision,
            self.state.editing,
            if self.state.editing.is_some() {
                self.editor.text().len()
            } else {
                0
            },
            messages.first_key_value().map(|(id, _)| *id),
            messages.last_key_value().map(|(id, _)| *id),
            messages.len(),
            self.layout_revision,
        );
        if self
            .layout_cache
            .borrow()
            .as_ref()
            .is_some_and(|cache| cache.key == key)
        {
            return;
        }
        let result: Vec<_> = self
            .state
            .model
            .messages
            .get(&channel)
            .into_iter()
            .flat_map(|v| v.values())
            .map(|m| {
                let grouped = self.grouped_row(m);
                let width = (self.viewport_w - t::MESSAGE_GUTTER - t::S16).max(t::ATTACHMENT_CARD);
                let chars = (width / t::GLYPH_WIDTH).max(1.) as usize;
                let resolved = self.state.model.content(m);
                let lines = resolved
                    .lines()
                    .map(|l| l.chars().count().max(1).div_ceil(chars))
                    .sum::<usize>()
                    .max(1);
                let body = if self.state.editing == Some(m.id) {
                    t::COMPOSER_MIN + t::S24 + t::S24 + t::HINT * t::LINE + t::S4
                } else if crate::gifs::inline_url(&m.content) {
                    t::THUMB_H
                } else {
                    lines as f32 * t::BODY * self.settings.preferences.chat_font_scale * t::LINE
                };
                let (date, unread) = self.row_decorations(m);
                let h = if grouped {
                    t::CONTINUATION
                } else {
                    t::S16 + t::BODY * t::LINE + t::S4
                } + body
                    + t::CONTINUATION
                    + if m.reply_to.is_some() { t::S24 } else { 0. }
                    + if !m.reactions.is_empty() {
                        t::S24 + t::S4
                    } else {
                        0.
                    }
                    + m.attachments
                        .iter()
                        .map(|a| {
                            if let Some(thumb) = &a.thumbnail {
                                let scale = (t::THUMB_W / thumb.width.max(1) as f32)
                                    .min(t::THUMB_H / thumb.height.max(1) as f32)
                                    .min(1.);
                                thumb.height as f32 * scale + t::S4
                            } else {
                                t::S32 + t::S24 + t::S4
                            }
                        })
                        .sum::<f32>()
                    + if date { t::S32 } else { 0. }
                    + if unread { t::S24 } else { 0. };
                let estimated = h.max(if grouped { t::S24 } else { t::AVATAR + t::S16 });
                let measured = self
                    .heights
                    .get(&m.id)
                    .filter(|(g, w, seq, editing, _)| {
                        *g == self.state.generation
                            && *w == self.viewport_w.to_bits()
                            && *seq == self.row_revision(m)
                            && *editing == (self.state.editing == Some(m.id))
                    })
                    .map(|v| v.4)
                    .unwrap_or(estimated);
                (m, measured, grouped)
            })
            .collect();
        let mut slots = self.history_slots.borrow_mut();
        for (m, height, grouped) in &result {
            slots.insert(
                m.id,
                HistorySlot {
                    height: *height,
                    grouped: *grouped,
                    author: m.author_id,
                    created_at: m.created_at,
                },
            );
        }
        // Compact IDs/heights survive body eviction. Cap metadata separately from the 1,000-body cap.
        while slots.len() > 20_000 {
            if self.state.at_present {
                slots.pop_first();
            } else {
                slots.pop_last();
            }
        }
        let rows: Vec<_> = slots
            .iter()
            .map(|(id, slot)| (*id, slot.height, slot.grouped))
            .collect();
        let mut offsets = Vec::with_capacity(rows.len() + 1);
        offsets.push(0.);
        for (_, h, _) in &rows {
            offsets.push(offsets.last().copied().unwrap() + h);
        }
        self.layout_cache
            .replace(Some(RowCache { key, rows, offsets }));
    }
    pub fn rows(&self) -> Vec<(&Message, f32, bool)> {
        self.ensure_rows();
        let Screen::Channel(channel) = self.state.screen else {
            return vec![];
        };
        let Some(messages) = self.state.model.messages.get(&channel) else {
            return vec![];
        };
        self.layout_cache
            .borrow()
            .as_ref()
            .map(|cache| {
                cache
                    .rows
                    .iter()
                    .filter_map(|(id, h, g)| messages.get(id).map(|m| (m, *h, *g)))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn visible_items(&self) -> Vec<(Id, Option<&Message>, f32, bool)> {
        let (start, end, _, _) = self.visible_range();
        let Screen::Channel(channel) = self.state.screen else {
            return vec![];
        };
        let messages = self.state.model.messages.get(&channel);
        self.layout_cache
            .borrow()
            .as_ref()
            .map(|cache| {
                cache.rows[start..end]
                    .iter()
                    .map(|(id, h, g)| (*id, messages.and_then(|m| m.get(id)), *h, *g))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn visible_rows(&self) -> Vec<(&Message, f32, bool)> {
        self.visible_items()
            .into_iter()
            .filter_map(|(_, m, h, g)| m.map(|m| (m, h, g)))
            .collect()
    }
    pub(crate) fn reset_history_geometry(&mut self) {
        self.history_slots.borrow_mut().clear();
        self.layout_cache.replace(None);
        self.layout_revision = self.layout_revision.wrapping_add(1);
    }
    pub fn history_height(&self) -> f32 {
        if self.extent.active() {
            return self.extent.total();
        }
        self.ensure_rows();
        self.layout_cache
            .borrow()
            .as_ref()
            .and_then(|c| c.offsets.last().copied())
            .unwrap_or(0.)
            + crate::theme::VIRTUAL_HEADER
            + crate::theme::HISTORY_BOTTOM
    }
    pub fn visible_range(&self) -> (usize, usize, f32, f32) {
        self.ensure_rows();
        let cache = self.layout_cache.borrow();
        let Some(cache) = cache.as_ref() else {
            return (0, 0, 0., 0.);
        };
        let y = (self.scroll_y - crate::theme::VIRTUAL_HEADER).max(0.);
        let first = cache
            .offsets
            .partition_point(|offset| *offset < y)
            .saturating_sub(1);
        let last = cache
            .offsets
            .partition_point(|offset| *offset < y + self.viewport_h);
        let start = first.saturating_sub(4).min(cache.rows.len());
        let end = (last + 4).min(cache.rows.len());
        (
            start,
            end,
            cache.offsets[start],
            cache.offsets.last().unwrap() - cache.offsets[end],
        )
    }
    fn visible_read(&self) -> Option<Id> {
        self.ensure_rows();
        let Screen::Channel(channel) = self.state.screen else {
            return None;
        };
        let messages = self.state.model.messages.get(&channel)?;
        let cache = self.layout_cache.borrow();
        let cache = cache.as_ref()?;
        let first = cache
            .offsets
            .partition_point(|y| *y + crate::theme::VIRTUAL_HEADER < self.scroll_y)
            .saturating_sub(1);
        cache
            .rows
            .iter()
            .enumerate()
            .skip(first)
            .take_while(|(i, _)| {
                cache.offsets[*i + 1] + crate::theme::VIRTUAL_HEADER
                    <= self.scroll_y + self.viewport_h
            })
            .filter(|(_, row)| messages.contains_key(&row.0))
            .map(|(_, row)| row.0)
            .last()
    }
    fn defer_read(&mut self) -> Task<Msg> {
        if !self.focused || self.fast_scrolling {
            return Task::none();
        }
        let read = self.visible_read();
        if read == self.state.read_id {
            return Task::none();
        }
        self.state.read_id = read;
        self.state.read_serial = self.state.read_serial.wrapping_add(1);
        let serial = self.state.read_serial;
        Task::perform(
            async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                serial
            },
            Msg::ReadDue,
        )
    }
    fn flush_read(&mut self) {
        if let Some(command) = self.state.read_command() {
            self.send(command);
        }
    }
    fn jump(&mut self) -> Task<Msg> {
        self.scroll_y = (self.history_height() - self.viewport_h).max(0.0);
        self.state.read_id = self.visible_read();
        self.state.read_serial += 1;
        let serial = self.state.read_serial;
        Task::batch([
            operation::snap_to("messages", scrollable::RelativeOffset::END),
            Task::perform(
                async move {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    serial
                },
                Msg::ReadDue,
            ),
        ])
    }
    fn cancel_compose(&mut self) {
        self.state.reply = None;
        self.state.editing = None;
        replace_editor(&mut self.editor, "");
        self.state.selected_mentions.clear();
    }
    fn activity(&mut self) {
        self.last_activity = Instant::now();
        if self
            .state
            .model
            .me
            .as_ref()
            .is_some_and(|u| u.desired_presence == PresenceStatus::Idle)
        {
            self.send(Command::SetPresence {
                status: PresenceStatus::Online,
            });
        }
    }
    fn arm_idle(&mut self) -> Task<Msg> {
        self.idle_armed = true;
        let wait = Duration::from_secs(300).saturating_sub(self.last_activity.elapsed());
        Task::perform(
            async move {
                tokio::time::sleep(wait).await;
            },
            |_| Msg::IdleDue,
        )
    }
    fn typing_expiry(&mut self) -> Task<Msg> {
        if self.typing_armed {
            return Task::none();
        }
        if let Some(t) = self.state.model.typing.values().min() {
            self.typing_armed = true;
            let wait = Duration::from_millis(t.saturating_sub(now()).max(50));
            Task::perform(
                async move {
                    tokio::time::sleep(wait).await;
                },
                |_| Msg::TypingExpire,
            )
        } else {
            Task::none()
        }
    }
    fn load_images(&mut self) -> Task<Msg> {
        self.markdown.retain(&self.state.model);
        self.revealed.retain(|(id, _)| {
            self.state
                .model
                .messages
                .values()
                .any(|m| m.contains_key(id))
        });
        let Some(s) = &self.state.session else {
            return Task::none();
        };
        let Some(account) = self.me() else {
            return Task::none();
        };

        let config = s.config.clone();
        let unknown: Vec<_> = self
            .visible_rows()
            .into_iter()
            .flat_map(|(m, _, _)| {
                m.mention_ids
                    .iter()
                    .copied()
                    .chain(std::iter::once(m.author_id))
            })
            .filter(|id| !self.state.model.users.contains_key(id) && !self.resolving.contains(id))
            .take(50)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if !unknown.is_empty() {
            self.resolving.extend(unknown.iter().copied());
            self.request(Request::Users {
                ids: unknown,
                generation: self.state.generation,
            });
        }

        let mut ids: Vec<_> = self
            .visible_rows()
            .into_iter()
            .flat_map(|(m, _, _)| {
                m.attachments
                    .iter()
                    .filter(|a| a.thumbnail.is_some())
                    .map(|a| a.id)
                    .chain(
                        self.state
                            .model
                            .users
                            .get(&m.author_id)
                            .and_then(|u| u.avatar_id),
                    )
            })
            .collect();
        ids.extend(
            self.state
                .model
                .guilds
                .values()
                .filter_map(|g| g.guild.icon_id),
        );
        ids.extend(
            self.state
                .model
                .members
                .values()
                .take(30)
                .filter_map(|m| m.user.avatar_id),
        );
        if let Some(me) = &self.state.model.me {
            ids.extend(me.user.avatar_id);
        }
        ids.extend(self.settings.server.icon);
        ids.sort();
        ids.dedup();
        self.media_tasks.retain(|id, _| ids.contains(id));
        self.images.pending.retain(|id| ids.contains(id));
        self.images.failed.retain(|id| ids.contains(id));
        let mut tasks = vec![];
        for id in ids {
            self.images.touch(id);
            if !self.images.contains(id) && self.images.pending.len() < 2 {
                self.images.pending.insert(id);
                let c = config.clone();
                let generation = self.state.generation;
                let (task, handle) = Task::perform(media::load(c, account, id), move |v| {
                    Msg::Image(id, generation, v)
                })
                .abortable();
                self.media_tasks.insert(id, handle.abort_on_drop());
                tasks.push(task);
            }
        }
        Task::batch(tasks)
    }
    pub fn oauth_pending(&self) -> bool {
        self.auth_task.is_some()
    }
    pub fn google_available(&self) -> bool {
        self.google_id.is_some()
            && (self.mock_mode || crate::oauth::configured(self.google_id.as_deref()))
    }
    fn open(&mut self, dialog: Dialog) {
        if let Dialog::Profile(id) = dialog {
            let _ = self.main_open(crate::main_window::Popover::Profile(id));
            return;
        }
        self.main.popover = None;
        self.main.position = self.main.pointer;
        self.main.anchor = self.main.anchor.filter(|r| r.contains(self.main.pointer));
        self.main.popover_size = iced::Size::ZERO;
        self.main.measurement_revision = self.main.measurement_revision.wrapping_add(1);
        if matches!(dialog, Dialog::MessageActions(_)) {
            if let Screen::Channel(id) = self.state.screen {
                self.request(Request::Launch {
                    read: sticktalk_client_core::launch::Read::Pins(id),
                    generation: self.state.generation,
                });
            }
        }
        self.form_revision = self.form_revision.wrapping_add(1);
        if matches!(self.dialog, Dialog::CreateGuild) {
            if let Some(id) = self.form.icon.take() {
                self.request(Request::DiscardUpload(id));
            }
        }
        let is_settings = matches!(
            dialog,
            Dialog::UserSettings | Dialog::GuildSettings(_) | Dialog::ChannelSettings(_)
        );
        if matches!(
            self.dialog,
            Dialog::UserSettings | Dialog::GuildSettings(_) | Dialog::ChannelSettings(_)
        ) && !is_settings
        {
            self.settings_return = Some(self.dialog.clone());
        }
        if is_settings {
            self.settings_return = None;
        }
        if !matches!(dialog, Dialog::MessageActions(_) | Dialog::MemberMenu(_)) {
            self.form = Form::default();
        }
        self.error.clear();
        match &dialog {
            Dialog::CreateGuild => {
                self.form.name = format!(
                    "{}'s server",
                    self.state
                        .model
                        .me
                        .as_ref()
                        .map_or("My", |m| m.user.username.as_str())
                );
            }
            Dialog::Invite => {
                if let Some(guild_id) = self.state.guild {
                    if let Some(invite) = self
                        .state
                        .model
                        .invites
                        .values()
                        .find(|v| v.guild_id == guild_id)
                    {
                        self.form.code = invite.code.clone();
                    } else {
                        self.send(Command::CreateInvite {
                            guild_id,
                            ttl_seconds: 604800,
                            max_uses: 100,
                        });
                    }
                }
            }
            Dialog::UserSettings => {
                if let Some(me) = &self.state.model.me {
                    self.form.code = me.user.username.clone();
                    self.form.name = me.user.display_name.clone().unwrap_or_default();
                    self.form.topic = me.user.status_text.clone().unwrap_or_default();
                }
            }
            Dialog::GuildSettings(tab) => {
                if let Some(g) = self.state.guild {
                    self.form.name = self
                        .state
                        .model
                        .guilds
                        .get(&g)
                        .map_or(String::new(), |v| v.guild.name.clone());
                    if *tab == GuildTab::Invites {
                        self.request(Request::Invites {
                            after: None,
                            guild: g,
                            generation: self.state.generation,
                        });
                    }
                    if *tab == GuildTab::Members || *tab == GuildTab::Roles {
                        self.request(Request::Members {
                            after: None,
                            guild: g,
                            generation: self.state.generation,
                        });
                    }
                    if *tab == GuildTab::Bans {
                        self.request(Request::Bans {
                            after: None,
                            guild: g,
                            generation: self.state.generation,
                        });
                    }
                }
            }
            Dialog::ChannelSettings(c) | Dialog::GroupSettings(c) => {
                if let Some(ch) = self.state.model.channels.get(c) {
                    self.form.name = ch.name.clone().unwrap_or_default();
                    self.form.topic = self
                        .state
                        .model
                        .topics
                        .get(c)
                        .cloned()
                        .flatten()
                        .unwrap_or_default();
                }
            }
            _ => {}
        }
        self.settings_open(&dialog);
        self.dialog = dialog;
    }
    fn submit(&mut self) -> Task<Msg> {
        let name = self.form.name.trim().to_owned();
        let topic = self.form.topic.trim().to_owned();
        let mut close = !matches!(
            self.dialog,
            Dialog::UserSettings | Dialog::GuildSettings(_) | Dialog::ChannelSettings(_)
        );
        let command = match self.dialog.clone() {
            Dialog::HistoryJump => {
                match backend::HistoryAnchor::parse(&name) {
                    Ok(anchor) => self.jump_history(anchor),
                    Err(error) => self.error = error,
                }
                return Task::none();
            }
            Dialog::AddGuild | Dialog::CreateGuild | Dialog::JoinGuild => {
                if !self.form.code.trim().is_empty() {
                    Some(Command::JoinGuild {
                        invite_code: self.form.code.trim().into(),
                    })
                } else if !name.is_empty() {
                    Some(Command::CreateGuild {
                        name,
                        icon_id: self.form.icon,
                    })
                } else {
                    self.error = "Choose a name or paste an invite code.".into();
                    None
                }
            }
            Dialog::UserSettings => Some(Command::UpdateProfile(ProfilePatch {
                display_name: if name.is_empty() {
                    Patch::Clear
                } else {
                    Patch::Set(name)
                },
                status_text: if topic.is_empty() {
                    Patch::Clear
                } else {
                    Patch::Set(topic)
                },
                ..ProfilePatch::default()
            })),
            Dialog::GuildSettings(GuildTab::Overview) => {
                self.state.guild.map(|guild_id| Command::UpdateGuild {
                    guild_id,
                    patch: GuildPatch {
                        name: Patch::Set(name),
                        icon_id: Patch::Keep,
                    },
                })
            }
            Dialog::NewChannel(kind, parent_id) => {
                self.state.guild.map(|guild_id| Command::CreateChannel {
                    guild_id,
                    kind,
                    name,
                    parent_id,
                    topic: if topic.is_empty() { None } else { Some(topic) },
                })
            }
            Dialog::ChannelSettings(channel_id) => Some(Command::UpdateChannel {
                channel_id,
                patch: ChannelPatch {
                    name: Patch::Set(name),
                    topic: if topic.is_empty() {
                        Patch::Clear
                    } else {
                        Patch::Set(topic)
                    },
                },
            }),
            Dialog::GroupSettings(channel_id) => Some(Command::UpdateChannel {
                channel_id,
                patch: ChannelPatch {
                    name: Patch::Set(name),
                    topic: Patch::Keep,
                },
            }),
            Dialog::AddFriend => Some(Command::SendFriendRequest {
                username: self.form.name.trim().to_ascii_lowercase(),
            }),
            Dialog::NewGroup => Some(Command::CreateGroupDm {
                name,
                user_ids: self.form.selected.iter().copied().collect(),
            }),
            Dialog::Profile(user_id) => {
                if let Some(guild_id) = self.state.guild {
                    close = false;
                    Some(Command::BanMember {
                        guild_id,
                        user_id,
                        reason: if self.form.reason.is_empty() {
                            None
                        } else {
                            Some(self.form.reason.clone())
                        },
                    })
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(c) = command {
            if self.send(c) && close {
                self.form_revision = self.form_revision.wrapping_add(1);
                self.form.icon = None;
                self.dialog = self.settings_return.take().unwrap_or(Dialog::None);
            }
        }
        Task::none()
    }
    fn reorder(&mut self, id: Id, down: bool) {
        let Some(guild_id) = self.state.guild else {
            return;
        };
        let Some(channel) = self.state.model.channels.get(&id) else {
            return;
        };
        let parent = channel.parent_id;
        let mut siblings: Vec<_> = self
            .state
            .model
            .channels
            .values()
            .filter(|c| c.guild_id == Some(guild_id) && c.parent_id == parent)
            .map(|c| (c.position, c.id))
            .collect();
        siblings.sort();
        let Some(i) = siblings.iter().position(|(_, v)| *v == id) else {
            return;
        };
        let other = if down { i + 1 } else { i.saturating_sub(1) };
        if other >= siblings.len() {
            return;
        }
        siblings.swap(i, other);
        let replacements: BTreeMap<_, _> = siblings
            .iter()
            .enumerate()
            .map(|(i, (_, id))| (*id, i as u16))
            .collect();
        let placements = self
            .state
            .model
            .channels
            .values()
            .filter(|c| c.guild_id == Some(guild_id))
            .map(|c| ChannelPlacement {
                channel_id: c.id,
                parent_id: c.parent_id,
                position: replacements.get(&c.id).copied().unwrap_or(c.position),
            })
            .collect();
        self.send(Command::ReorderChannels {
            guild_id,
            placements,
        });
    }
}
pub fn editor_binding(
    event: text_editor::KeyPress,
    empty: bool,
    autocomplete: bool,
) -> Option<text_editor::Binding<Msg>> {
    if matches!(event.status, text_editor::Status::Focused { .. }) {
        if autocomplete {
            let custom = match event.key {
                keyboard::Key::Named(Named::ArrowDown) => Some(Msg::Suggest(1)),
                keyboard::Key::Named(Named::ArrowUp) => Some(Msg::Suggest(-1)),
                keyboard::Key::Named(Named::Tab | Named::Enter) => Some(Msg::CompleteMention),
                keyboard::Key::Named(Named::Escape) => Some(Msg::DismissSuggestions),
                _ => None,
            };
            if let Some(msg) = custom {
                return Some(text_editor::Binding::Custom(msg));
            }
        }
        match event.key {
            keyboard::Key::Named(Named::Tab) => {
                return Some(text_editor::Binding::Custom(Msg::Tab(
                    event.modifiers.shift(),
                )))
            }
            keyboard::Key::Named(Named::Enter) if !event.modifiers.shift() => {
                return Some(text_editor::Binding::Custom(Msg::Send))
            }
            keyboard::Key::Named(Named::ArrowUp) if empty => {
                return Some(text_editor::Binding::Custom(Msg::EditLast))
            }
            keyboard::Key::Named(Named::Escape) => {
                return Some(text_editor::Binding::Custom(Msg::CancelCompose))
            }
            _ => {}
        }
    }
    text_editor::Binding::from_key_press(event)
}

// Keep the native editor/buffer identity and its shaped-font state when
// replacing drafts, rather than swapping Content underneath an existing widget.
fn replace_editor(content: &mut text_editor::Content, value: &str) {
    content.perform(text_editor::Action::SelectAll);
    content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(
        std::sync::Arc::new(value.to_owned()),
    )));
}
#[cfg(test)]
fn editor_content(value: &str) -> text_editor::Content {
    let mut content = text_editor::Content::new();
    if !value.is_empty() {
        content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(
            std::sync::Arc::new(value.to_owned()),
        )));
    }
    content
}

impl App {
    /// Pure fixture constructor: no subscriptions, credential reads, browser, network, dialogs or window.
    pub(crate) fn headless(screen: &str, size: iced::Size) -> Self {
        let (mut app, _) = Self::new();
        app.capture = None;
        app.initial_screen.clear();
        app.stats = None;
        app.mock_mode = true;
        app.width = size.width;
        app.height = size.height;
        app.viewport_w = size.width
            - crate::theme::RAIL
            - crate::theme::SIDEBAR
            - if size.width >= crate::theme::MEMBER_BREAKPOINT {
                crate::theme::MEMBERS
            } else {
                0.
            };
        app.viewport_h = size.height
            - crate::theme::HEADER
            - crate::theme::COMPOSER_MIN
            - crate::theme::S24
            - crate::theme::TYPING
            - crate::theme::S8;
        let mut mock = crate::mock::Mock::new();
        mock.configure(
            if matches!(screen, "main-history-extent" | "main-history-skeleton") {
                "scroll-5000"
            } else {
                screen
            },
        );
        let channel = match screen {
            "dm-chat" => Id(20),
            "group-chat" => Id(21),
            _ => Id(12),
        };
        let page = mock.history(channel, None);
        let extent = if matches!(screen, "main-history-extent" | "main-history-skeleton") {
            mock.request::<PathBuf, backend::Handle>(Request::Launch {
                read: sticktalk_client_core::launch::Read::Extent(channel),
                generation: 0,
            })
        } else {
            None
        };
        let benchmark_messages = (matches!(screen, "scroll-5000" | "scroll-skeleton"))
            .then(|| mock.model.messages[&channel].clone());
        app.state.model = std::mem::take(&mut mock.model);
        app.state.model.messages.retain(|id, _| *id == channel);
        app.state
            .model
            .messages
            .insert(channel, page.items.into_iter().map(|m| (m.id, m)).collect());
        if let Some(messages) = benchmark_messages {
            app.state.model.messages.insert(channel, messages);
        }
        app.state.history_more = page.has_more;
        app.state.before = page.next_before;
        app.state.loading = false;
        app.state.connection = ConnectionState::Ready;
        app.state.screen = Screen::Channel(channel);
        app.state.guild = app.state.model.channels[&channel].guild_id;
        app.state.model.active_guild = app.state.guild;
        app.google_id = Some("mock-desktop".into());
        app.signup = None;
        app.state.session = Some(Session {
            config: Config {
                server: storage::DEFAULT_SERVER.into(),
                token: "river".into(),
                mock: true,
                generation: 1,
                scenario: screen.into(),
            },
        });
        app.server = "https://chat.sticktalk.example".into();
        app.dialog = Dialog::None;
        app.typing_armed = false;
        for id in [Id(2), Id(3)] {
            app.state.model.typing.insert((channel, id), now() + 8000);
        }
        let image = image::load_from_memory(sticktalk_client_core::mock::IMAGE)
            .unwrap()
            .into_rgba8();
        app.images.insert(Decoded {
            id: Id(900001),
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        });
        match screen {
            "login"
            | "register"
            | "google-login"
            | "pick-username"
            | "login-error"
            | "login-connecting"
            | "crash-recovery"
            | "login-update"
            | "login-update-confirm" => {
                app.state.session = None;
                app.register = screen == "register";
                if matches!(
                    screen,
                    "login" | "login-error" | "login-connecting" | "crash-recovery"
                ) {
                    app.google_id = None;
                }
                app.meta_loading = screen == "login-connecting";
                app.features.crash_notice = screen == "crash-recovery";
                if matches!(screen, "login-update" | "login-update-confirm") {
                    app.features.update_ready = Some("0.2.0".into());
                }
                app.features.confirm_update = screen == "login-update-confirm";
                if screen == "login-error" {
                    app.meta_error =
                        "Could not connect. Check the server address and try again.".into();
                }
                if screen == "pick-username" {
                    app.signup = match app.mock_account.lock().unwrap().google("mock-new").unwrap()
                    {
                        GoogleAuthResponse::SignupRequired(s) => Some(s),
                        _ => None,
                    };
                }
            }
            "connecting" => {
                app.state.model.me = None;
                app.state.connection = ConnectionState::Connecting;
            }
            "home-online" | "empty-home" => {
                app.state.guild = None;
                app.state.screen = Screen::Friends(FriendTab::Online);
            }
            "home-all" => {
                app.state.guild = None;
                app.state.screen = Screen::Friends(FriendTab::All);
            }
            "home-pending" => {
                app.state.guild = None;
                app.state.screen = Screen::Friends(FriendTab::Pending);
            }
            "home-blocked" => {
                app.state.guild = None;
                app.state.screen = Screen::Friends(FriendTab::Blocked);
            }
            "community-picker" => app.open(Dialog::AddGuild),
            "create-guild-form" => app.open(Dialog::CreateGuild),
            "join-guild" => app.open(Dialog::JoinGuild),
            "invite" => {
                app.open(Dialog::Invite);
                app.form.code = "STICK-7mQx-29ab".into();
            }
            "history-jump" => {
                app.open(Dialog::HistoryJump);
                app.form.name = "2026-11-01".into();
            }
            "reply-preview" => {
                app.preview = app
                    .state
                    .model
                    .messages
                    .get(&Id(12))
                    .and_then(|m| m.values().next())
                    .cloned();
                app.open(Dialog::ReplyPreview(app.preview.as_ref().unwrap().id));
            }
            "add-friend" => app.open(Dialog::AddFriend),
            "guild-settings-overview" => app.open(Dialog::GuildSettings(GuildTab::Overview)),
            "guild-settings-channels" => app.open(Dialog::GuildSettings(GuildTab::Channels)),
            "guild-settings-members" => app.open(Dialog::GuildSettings(GuildTab::Members)),
            "guild-settings-invites" => app.open(Dialog::GuildSettings(GuildTab::Invites)),
            "guild-settings-bans" => app.open(Dialog::GuildSettings(GuildTab::Bans)),
            "user-settings-account" | "sign-in-methods" => {
                app.open(Dialog::UserSettings);
                app.user_tab = 0;
                app.methods = Some(AuthMethodsResponse {
                    has_password: true,
                    google_linked: true,
                });
            }
            "user-settings-profile" => {
                app.open(Dialog::UserSettings);
                app.user_tab = 1;
            }
            "user-settings-appearance" => {
                app.open(Dialog::UserSettings);
                app.user_tab = 2;
            }
            "context" => app.open(Dialog::MessageActions(Id(2279))),
            "context-own-message" => app.open(Dialog::MessageActions(Id(2276))),
            "masked-link-confirm" => app.open(Dialog::ExternalLink(
                "https://example.com/release-notes".into(),
            )),
            "toast" => app.notice = "Invite copied.".into(),
            "loading" => {
                app.state.model.messages.clear();
                app.state.loading = true;
            }
            "disconnected" => app.state.connection = ConnectionState::Backoff(4),
            "mention-autocomplete" => replace_editor(&mut app.editor, "@mi"),
            "edit-message" => {
                app.state.editing = Some(Id(2276));
                replace_editor(
                    &mut app.editor,
                    "I tried the first version today. It feels thoughtful.",
                );
            }
            "message-hover" => app.hovered = Some(Id(2279)),
            "message-hover-own" => app.hovered = Some(Id(2276)),
            "message-hover-grouped" => app.hovered = Some(Id(2273)),
            "change-server" => {
                app.state.session = None;
            }
            "channel-top" => {
                app.state.at_present = false;
                app.scroll_y = 0.;
            }
            "markdown-revealed" => {
                app.revealed.insert((Id(2278), 0));
            }
            "member-menu" => {
                app.open(Dialog::GuildSettings(GuildTab::Members));
                app.open(Dialog::MemberMenu(Id(2)));
            }
            "password-settings" => {
                app.open(Dialog::UserSettings);
                app.methods = Some(AuthMethodsResponse {
                    has_password: true,
                    google_linked: true,
                });
                app.open(Dialog::Password);
            }
            "sign-in-methods-unlinked" | "sign-in-methods-google-only" => {
                app.open(Dialog::UserSettings);
                app.user_tab = 0;
                app.methods = Some(AuthMethodsResponse {
                    has_password: screen == "sign-in-methods-unlinked",
                    google_linked: screen == "sign-in-methods-google-only",
                });
            }
            _ => {}
        }
        use crate::main_window::{Entity, Popover};
        if screen.starts_with("main-") {
            app.settings.preferences.developer_mode = true;
            app.main.pointer = iced::Point::new(size.width - crate::theme::MEMBERS + 40., 160.);
            app.main.position = app.main.pointer;
            app.main.popover = match screen {
                "main-server-menu" | "main-server-menu-member" => Some(Popover::Server),
                "main-profile" => Some(Popover::Profile(Id(2))),
                "main-profile-idle" => Some(Popover::Profile(Id(3))),
                "main-profile-dnd" => {
                    app.state.model.presences.insert(Id(4), PresenceStatus::Dnd);
                    Some(Popover::Profile(Id(4)))
                }
                "main-profile-offline" => Some(Popover::Profile(Id(5))),
                "main-own-profile" => Some(Popover::Profile(Id(1))),
                "main-pins" => Some(Popover::Pins(Id(12))),
                "main-notifications" => Some(Popover::Notifications(Some(Id(10)), Some(Id(12)))),
                "main-topic" => Some(Popover::Topic(Id(12))),
                "main-quick-switcher" => {
                    app.main.quick = "gnrl".into();
                    Some(Popover::Quick)
                }
                "main-user-context" => Some(Popover::Context(Entity::User(Id(2)))),
                "main-channel-context" => Some(Popover::Context(Entity::Channel(Id(22)))),
                "main-server-context" => Some(Popover::Context(Entity::Guild(Id(10)))),
                "main-rail-context" => Some(Popover::Context(Entity::Rail)),
                "main-devices" => Some(Popover::Devices(true)),
                "main-mutual-servers" => Some(Popover::Mutual(Id(2), true)),
                "main-role-picker" => Some(Popover::Roles(Id(2))),
                "main-nickname" => Some(Popover::Nickname(Id(2))),
                _ => None,
            };
            match screen {
                "main-notifications" => {
                    app.main.position = iced::Point::new(
                        size.width - crate::theme::SEARCH_W - crate::theme::S32 * 3.,
                        crate::theme::HEADER,
                    )
                }
                "main-topic" => {
                    app.main.position = iced::Point::new(
                        crate::theme::RAIL + crate::theme::SIDEBAR + crate::theme::S32 * 5.,
                        crate::theme::HEADER,
                    )
                }
                "main-channel-context" => {
                    app.main.position = iced::Point::new(
                        crate::theme::RAIL + crate::theme::S32 * 4.,
                        crate::theme::HEADER + crate::theme::S32 * 4.,
                    )
                }
                "main-server-context" | "main-rail-context" => {
                    app.main.position =
                        iced::Point::new(crate::theme::RAIL / 2., crate::theme::GUILD_ICON * 3.)
                }
                "main-own-profile" => {
                    app.main.position = iced::Point::new(
                        crate::theme::RAIL + crate::theme::S24,
                        size.height - crate::theme::S32,
                    )
                }
                "main-server-menu-member" => {
                    app.state
                        .model
                        .guilds
                        .get_mut(&Id(10))
                        .unwrap()
                        .guild
                        .owner_id = Id(2);
                }
                "main-no-voice" => {
                    app.state.model.launch.voice.clear();
                    app.state.model.launch.speaking.clear();
                }
                "main-members-hidden" => app.members_open = Some(false),
                "main-rail-unread" => {
                    if let Some(r) = app.state.model.reads.get_mut(&Id(41)) {
                        r.unread_count = 4;
                    }
                }
                "main-rail-hover" => app.hovered_guild = Some(Some(Id(30))),
                "main-channel-hover" => app.main.hovered_channel = Some(Id(22)),
                "main-category-hover" => app.main.hovered_category = Some(Id(11)),
                "main-category-collapsed" => {
                    app.main.collapsed.insert(Id(11));
                }
                "main-streaming" => {
                    if let Some(v) = app
                        .state
                        .model
                        .launch
                        .voice
                        .values_mut()
                        .find(|v| v.participant.session.user_id == Id(1))
                    {
                        v.participant.tracks.push(PublishedTrack {
                            mid: "screen".into(),
                            kind: TrackKind::Screen,
                            codec: Some(VideoCodec::H264),
                            layers: vec![],
                            surface: Some(ShareSurface::Monitor),
                        });
                    }
                }
                "main-muted-controls" => {
                    app.main.muted = true;
                    app.main.deafened = true;
                }
                "main-user-hover" => app.main.user_hover = true,
                "main-offline-hover" => app.main.hovered_member = Some(Id(5)),
                "main-drop-overlay" => app.main.dropping = true,
                "main-local-search" => app.main.search = "sketches".into(),
                "main-message-menu" => {
                    app.open(Dialog::MessageActions(Id(2279)));
                }
                _ => {}
            }
        }
        if let Some(Popover::Profile(id)) = app.main.popover {
            let y = match id.0 {
                2 => 105.,
                1 => 147.,
                4 => 231.,
                3 => 273.,
                5 => 359.,
                _ => 401.,
            };
            app.main.anchor = Some(iced::Rectangle::new(
                iced::Point::new(size.width - crate::theme::MEMBERS + crate::theme::S16, y),
                iced::Size::new(
                    crate::theme::MEMBERS - crate::theme::S32,
                    crate::theme::PARITY_MEMBER_ROW,
                ),
            ));
            if screen == "main-own-profile" {
                app.main.anchor = Some(iced::Rectangle::new(
                    iced::Point::new(
                        crate::theme::RAIL + crate::theme::S8,
                        size.height - crate::theme::USER_HEIGHT - crate::theme::S16,
                    ),
                    iced::Size::new(
                        crate::theme::SIDEBAR - crate::theme::S16,
                        crate::theme::USER_HEIGHT,
                    ),
                ));
            }
        }
        match screen {
            "main-last-long" | "main-last-long-no-typing" => {
                let messages = app.state.model.messages.get_mut(&channel).unwrap();
                messages.last_entry().unwrap().get_mut().content = "Thanks, everyone. Let's pick this up tomorrow. The notes include the decisions, follow-up questions, and next steps so nobody needs to search through the conversation again. ".repeat(5);
                if screen.ends_with("no-typing") {
                    app.state.model.typing.clear();
                }
            }
            "main-last-image" => {
                let messages = app.state.model.messages.get_mut(&channel).unwrap();
                let attachment = messages
                    .values()
                    .find(|m| !m.attachments.is_empty())
                    .unwrap()
                    .attachments
                    .clone();
                let last = messages.last_entry().unwrap().into_mut();
                last.content = "Here's the final photo for tomorrow.".into();
                last.attachments = attachment;
            }
            "main-search-focused" => app.main.search_focused = true,
            "main-avatar-sizes" => {
                app.main.avatar_gallery = true;
                app.state.model.presences.insert(Id(4), PresenceStatus::Dnd);
            }
            "main-rail-counts" => {
                for (id, count) in [(Id(14), 10), (Id(31), 100), (Id(41), 99)] {
                    if let Some(read) = app.state.model.reads.get_mut(&id) {
                        read.mention_count = count;
                    }
                }
            }
            _ => {}
        }
        if let Some(methods) = &app.methods {
            app.mock_account.lock().unwrap().methods = methods.clone();
        }
        if screen == "scroll-skeleton" {
            app.state.at_present = false;
            app.scroll_y = app.history_height() * 0.5;
            app.fast_scrolling = true;
        }
        if let Some(Data::Launch {
            data: sticktalk_client_core::launch::SnapshotData::Extent(v),
            ..
        }) = extent
        {
            app.extent_snapshot(&v.data);
            app.state.at_present = false;
            app.scroll_y = app.extent.total() * 0.5;
            app.extent.loaded = false;
            app.extent.ends_at_present = false;
            if screen == "main-history-extent" {
                let anchor = app
                    .extent
                    .nearest(app.scroll_y + app.viewport_h / 2.)
                    .unwrap();
                let mut peer = crate::mock::Mock::new();
                peer.configure("scroll-5000");
                if let Some(Data::AroundHistory { page, .. }) = peer
                    .request::<PathBuf, backend::Handle>(Request::AroundHistory {
                        channel,
                        anchor: backend::HistoryAnchor::Message(anchor.message_id),
                        seek: 0,
                        generation: 0,
                    })
                {
                    let before = page
                        .items
                        .iter()
                        .filter(|m| m.id < anchor.message_id)
                        .count() as u64;
                    app.extent.start_rank = anchor.rank.saturating_sub(before);
                    app.state.model.messages.remove(&channel);
                    app.state.model.merge_page(channel, page);
                    app.reset_history_geometry();
                    app.extent.loaded = true;
                }
            }
        }
        if screen.starts_with("main-history-") {
            if let Some(c) = app.state.model.channels.get_mut(&channel) {
                c.name = Some("general".into());
            }
            if let Some(messages) = app.state.model.messages.get_mut(&channel) {
                for (i, m) in messages.values_mut().enumerate() {
                    m.content = match i % 3 {
                        0 => "The notes from this afternoon are ready. I added the decisions and the open questions so we can pick up where we left off tomorrow.".into(),
                        1 => "Thanks for the thoughtful feedback. I'll bring the updated sketches to our next conversation, including the simpler version we discussed.".into(),
                        _ => "I've shared the photos and the reading list in design-lab. Let me know if anything is missing, and feel free to add your own suggestions.".into(),
                    };
                }
            }
        }
        crate::ui::settings::configure(&mut app, screen);
        app.error.clear();
        app.mock_call_screen(screen);
        crate::features::fixture(&mut app, screen);
        crate::features3::fixture(&mut app, screen);
        app
    }
    pub(crate) fn headless_scroll(&mut self) {
        if self.state.at_present {
            self.scroll_y = (self.history_height() - self.viewport_h).max(0.);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(key: Named, modifiers: keyboard::Modifiers) -> text_editor::KeyPress {
        text_editor::KeyPress {
            key: keyboard::Key::Named(key),
            modified_key: keyboard::Key::Named(key),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Enter),
            modifiers,
            text: None,
            status: text_editor::Status::Focused { is_hovered: false },
        }
    }
    #[test]
    fn high_refresh_drag_uses_velocity_not_only_single_event_distance() {
        assert!(fast_scroll_delta(40., 1200., Duration::from_millis(4)));
        assert!(fast_scroll_delta(2000., 1200., Duration::from_secs(1)));
        assert!(!fast_scroll_delta(
            14.25,
            1200.,
            Duration::from_micros(3571)
        ));
        assert!(!fast_scroll_delta(0.25, 1200., Duration::from_millis(1)));
    }
    #[test]
    fn virtual_index_keeps_evicted_slots_and_only_builds_visible_rows() {
        let mut app = App::headless("scroll-5000", iced::Size::new(2560., 1440.));
        app.state.at_present = false;
        let total = app.history_height();
        app.scroll_y = total * 0.5;
        let rows = app.visible_items();
        assert!(rows.len() < 64);
        let target = rows[rows.len() / 2].0;
        app.state
            .model
            .messages
            .get_mut(&Id(12))
            .unwrap()
            .retain(|id, _| *id < Id(1_000_100));
        app.state.model.layout_revision += 1;
        assert_eq!(app.history_height(), total);
        assert!(app
            .visible_items()
            .iter()
            .any(|(id, m, _, _)| *id == target && m.is_none()));
        let y = app.scroll_y;
        let mut backend = crate::mock::Mock::new();
        backend.configure("scroll-5000");
        let before = Id(app.visible_items().last().unwrap().0 .0 + 1);
        let page = backend.history(Id(12), Some(before));
        app.state.history_seek = 7;
        let generation = app.state.generation;
        let stale = page.clone();
        let _ = app.network(Data::AroundHistory {
            seek: 6,
            anchor: backend::HistoryAnchor::Message(target),
            channel: Id(12),
            page: stale,
            generation,
        });
        assert!(!app.state.model.messages[&Id(12)].contains_key(&target));
        let _ = app.network(Data::AroundHistory {
            seek: 7,
            anchor: backend::HistoryAnchor::Message(target),
            channel: Id(12),
            page,
            generation,
        });
        assert!(app.state.model.messages[&Id(12)].contains_key(&target));
        assert_eq!(app.scroll_y, y);
        assert_eq!(app.history_height(), total);
    }
    #[test]
    fn unrelated_stream_events_do_not_invalidate_message_geometry() {
        let mut app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        let id = Id(2276);
        let before = app.row_revision(&app.state.model.messages[&Id(12)][&id]);
        let total = app.history_height();
        app.state.model.cursor.as_mut().unwrap().seq += 1;
        assert_eq!(
            app.row_revision(&app.state.model.messages[&Id(12)][&id]),
            before
        );
        assert_eq!(app.history_height(), total);
        app.state
            .model
            .messages
            .get_mut(&Id(12))
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .content
            .push_str(" edited");
        assert_ne!(
            app.row_revision(&app.state.model.messages[&Id(12)][&id]),
            before
        );
    }
    #[test]
    fn subpixel_row_measurements_do_not_invalidate_layout_forever() {
        let mut app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        let id = Id(2276);
        let width = app.viewport_w.to_bits();
        let seq = app.row_revision(&app.state.model.messages[&Id(12)][&id]);
        app.heights
            .insert(id, (app.state.generation, width, seq, false, 80.25));
        let revision = app.layout_revision;
        for height in [80.25 - 1.0 / 128.0, 80.25 + 1.0 / 128.0, 80.25] {
            let _ = app.update(Msg::Measured {
                message: id,
                generation: app.state.generation,
                width,
                seq,
                editing: false,
                height,
            });
        }
        assert_eq!(app.layout_revision, revision);
        assert_eq!(app.heights[&id].4, 80.25);
    }
    #[test]
    fn cancelled_server_icon_upload_cannot_apply_to_another_form() {
        let mut app = App::headless("create-guild-form", iced::Size::new(1280., 800.));
        let revision = app.form_revision;
        let attachment = app.state.model.messages[&Id(12)][&Id(2274)].attachments[0].clone();
        let delivery = |revision| Data::Uploaded {
            form_revision: revision,
            response: UploadResponse {
                attachment: attachment.clone(),
                expires_at: now() + 60000,
            },
            purpose: UploadPurpose::GuildIcon,
            channel: None,
            generation: 0,
        };
        let _ = app.network(delivery(revision));
        assert_eq!(app.form.icon, Some(attachment.id));
        app.open(Dialog::GuildSettings(GuildTab::Overview));
        let _ = app.network(delivery(revision));
        assert!(app.form.icon.is_none());
        app.open(Dialog::CreateGuild);
        let _ = app.network(delivery(revision));
        assert!(app.form.icon.is_none());
        let _ = app.network(delivery(app.form_revision));
        assert_eq!(app.form.icon, Some(attachment.id));
    }
    #[test]
    fn settings_escape_modal_return_and_google_cancel_are_local() {
        let mut app = App::headless("member-menu", iced::Size::new(1280., 800.));
        let _ = app.update(Msg::Escape);
        assert!(matches!(
            app.dialog,
            Dialog::GuildSettings(GuildTab::Members)
        ));
        let _ = app.update(Msg::Escape);
        assert!(matches!(app.dialog, Dialog::None));
        app.open(Dialog::UserSettings);
        let command = Command::CreateInvite {
            guild_id: Id(10),
            ttl_seconds: 3600,
            max_uses: 10,
        };
        app.open(Dialog::Confirm("Confirm".into(), command.clone()));
        let _ = app.update(Msg::Confirm(command));
        assert!(matches!(app.dialog, Dialog::UserSettings));
        let mut app = App::headless("pick-username", iced::Size::new(1280., 800.));
        let _ = app.update(Msg::GoogleCancel);
        assert!(app.signup.is_none());
        assert!(!app.busy);
        assert!(app.error.contains("cancelled"));
    }
    #[test]
    fn composer_keyboard_contract() {
        assert!(matches!(
            editor_binding(
                key(Named::Enter, keyboard::Modifiers::empty()),
                false,
                false
            ),
            Some(text_editor::Binding::Custom(Msg::Send))
        ));
        assert!(matches!(
            editor_binding(key(Named::Enter, keyboard::Modifiers::SHIFT), false, false),
            Some(text_editor::Binding::Enter)
        ));
        assert!(matches!(
            editor_binding(
                key(Named::ArrowUp, keyboard::Modifiers::empty()),
                true,
                false
            ),
            Some(text_editor::Binding::Custom(Msg::EditLast))
        ));
        assert!(matches!(
            editor_binding(
                key(Named::ArrowUp, keyboard::Modifiers::empty()),
                false,
                false
            ),
            Some(text_editor::Binding::Move(_))
        ));
        assert!(matches!(
            editor_binding(key(Named::Enter, keyboard::Modifiers::empty()), false, true),
            Some(text_editor::Binding::Custom(Msg::CompleteMention))
        ));
        assert!(matches!(
            editor_binding(key(Named::Tab, keyboard::Modifiers::SHIFT), false, false),
            Some(text_editor::Binding::Custom(Msg::Tab(true)))
        ));
    }
    #[test]
    fn mention_selection_and_editor_replacement() {
        let (mut app, _) = App::new();
        app.editor = editor_content("@mi");
        let _ = app.update(Msg::Mention(Id(2)));
        assert_eq!(app.editor.text(), "<@2> ");
        assert_eq!(
            app.state
                .selected_mentions
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![Id(2)]
        );
        let _ = app.update(Msg::CancelCompose);
        assert!(app.state.selected_mentions.is_empty());
        assert_eq!(
            editor_content("line one\nline two").text(),
            "line one\nline two"
        );
    }
    #[test]
    fn virtualized_rows_are_bounded_and_measurements_override_estimates() {
        let (mut app, _) = App::new();
        let mock = crate::mock::Mock::new();
        app.state.model.ready(mock.ready());
        app.state.screen = Screen::Channel(Id(12));
        let page = mock.history(Id(12), None);
        assert!(app.state.model.merge_page(Id(12), page));
        let (start, end, _, _) = app.visible_range();
        assert!(end - start < 50);
        let id = app.rows()[0].0.id;
        let seq = app.row_revision(&app.state.model.messages[&Id(12)][&id]);
        app.heights.insert(
            id,
            (
                app.state.generation,
                app.viewport_w.to_bits(),
                seq,
                false,
                777.,
            ),
        );
        app.layout_revision += 1;
        assert_eq!(app.rows()[0].1, 777.);
        app.viewport_w += 1.;
        assert_ne!(app.rows()[0].1, 777.);
    }
    fn test_page<T>(app: &App, items: Vec<T>) -> Page<T> {
        Page {
            items,
            users: vec![],
            next_before: None,
            next_after: None,
            has_more: false,
            cursor: app.state.model.cursor.clone().unwrap(),
        }
    }
    #[test]
    fn metadata_continuation_pages_append_instead_of_replacing() {
        let mut app = App::headless("guild-settings-members", iced::Size::new(1280., 800.));
        let guild = app.state.guild.unwrap();
        let generation = app.state.generation;
        let members: Vec<_> = app
            .state
            .model
            .members
            .values()
            .filter(|m| m.guild_id == guild)
            .take(2)
            .cloned()
            .collect();
        assert_eq!(members.len(), 2);
        for (n, member) in members.iter().enumerate() {
            let page = test_page(&app, vec![member.clone()]);
            let _ = app.network(Data::Members {
                guild,
                after: (n > 0).then_some(members[0].user.id),
                page,
                generation,
            });
        }
        assert_eq!(app.state.model.members.len(), 2);
        for n in 0..2 {
            let invite = Invite {
                id: Id(800 + n),
                code: format!("code{n}"),
                guild_id: guild,
                creator_id: members[0].user.id,
                expires_at: 0,
                max_uses: 10,
                uses: 0,
            };
            let page = test_page(&app, vec![invite]);
            let _ = app.network(Data::Invites {
                guild,
                after: (n > 0).then_some(Id(800)),
                page,
                generation,
            });
            let ban = Ban {
                guild_id: guild,
                user: members[n as usize].user.clone(),
                actor_id: app.me().unwrap(),
                reason: None,
                created_at: 0,
            };
            let page = test_page(&app, vec![ban]);
            let _ = app.network(Data::Bans {
                guild,
                after: (n > 0).then_some(members[0].user.id),
                page,
                generation,
            });
        }
        assert_eq!(app.state.model.invites.len(), 2);
        assert_eq!(app.state.model.bans.len(), 2);
    }
    #[test]
    fn jump_to_unloaded_reply_replaces_only_the_region_and_ignores_stale_results() {
        let mut app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        let channel = Id(12);
        let generation = app.state.generation;
        let mut source = crate::mock::Mock::new();
        source.configure("scroll-5000");
        let page = source.history(channel, Some(Id(1_000_100)));
        let target = page.items[20].id;
        app.state.history_seek = 42;
        app.state.jump_seek = Some(42);
        let old_count = app.state.model.retained_count();
        let _ = app.network(Data::History {
            channel,
            seek: Some(41),
            page: page.clone(),
            generation,
        });
        let _ = app.network(Data::NewerHistory {
            channel,
            seek: 41,
            page: page.clone(),
            generation,
        });
        assert_eq!(app.state.model.retained_count(), old_count);

        let _ = app.network(Data::AroundHistory {
            channel,
            anchor: backend::HistoryAnchor::Message(target),
            seek: 41,
            page: page.clone(),
            generation,
        });
        assert_eq!(app.state.model.retained_count(), old_count);
        let _ = app.network(Data::AroundHistory {
            channel,
            anchor: backend::HistoryAnchor::Message(target),
            seek: 42,
            page,
            generation,
        });
        assert!(app.state.model.messages[&channel].contains_key(&target));
        assert!(app.state.model.messages[&channel].len() <= 50);
        assert!(app.history_slots.borrow().len() <= 50);
        assert!(!app.state.at_present);
    }
    #[test]
    fn failed_history_request_does_not_leave_the_landing_loader_stuck() {
        let mut app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        app.state.seek_loading = true;
        let _ = app.network(Data::Error("Server request failed".into()));
        assert!(!app.state.seek_loading);
    }
    #[test]
    fn server_connection_errors_are_visible_and_stale_meta_is_ignored() {
        let mut app = App::headless("login", iced::Size::new(1280., 800.));
        let _ = app.update(Msg::Server("https://new.example".into()));
        let _ = app.update(Msg::RefreshMeta);
        assert!(app.meta_loading);
        let _ = app.update(Msg::Meta(
            "https://old.example".into(),
            Err("unreachable".into()),
        ));
        assert!(app.meta_error.is_empty());
        assert!(app.meta_loading);
        let _ = app.update(Msg::Meta(
            "https://new.example".into(),
            Err("unreachable".into()),
        ));
        assert!(!app.meta_loading);
        assert!(app.meta_error.contains("Check the address"));
        let _ = app.update(Msg::Server("https://fixed.example".into()));
        assert!(app.meta_error.is_empty());
        assert!(app.google_id.is_none());
    }
    #[test]
    fn invite_before_login_is_retained_without_losing_login_screen() {
        let mut app = App::headless("login", iced::Size::new(1280., 800.));
        let activation =
            sticktalk_desktop_platform::activation::Activation::Invite("valid-code".into());
        let _ = app.update(Msg::feature(crate::features::Event::Activation(
            activation.clone(),
        )));
        assert_eq!(app.features.desktop.pending_activation, Some(activation));
        assert!(app.state.session.is_none());
    }
}

#[cfg(test)]
mod qa_channel_controls {
    use super::*;
    #[test]
    fn read_only_keyboard_send_and_reaction_shortcuts_are_not_queued() {
        let mut app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        app.state.connection = ConnectionState::Ready;
        app.state.screen = Screen::Channel(Id(12));
        app.state.model.launch.effective_permissions.insert(
            Id(12),
            Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
        );
        replace_editor(&mut app.editor, "Keep this draft when sending is denied");
        let _ = app.update(Msg::Send);
        assert!(app.error.contains("permission"));
        assert_eq!(app.editor.text(), "Keep this draft when sending is denied");
        assert!(app.state.pending.is_empty());
        app.error.clear();
        assert!(!app.send(Command::AddReaction {
            channel_id: Id(12),
            message_id: Id(2279),
            emoji: "👍".into()
        }));
        assert!(app.error.contains("permission"));
        assert!(app.state.pending.is_empty());
    }
}

#[cfg(test)]
mod qa_notification_defaults {
    use super::*;
    #[tokio::test]
    async fn real_event_path_honors_device_notification_defaults_without_os_effects() {
        let mut app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        app.focused = false;
        app.state.screen = Screen::Friends(FriendTab::All);
        app.features.desktop.settings.clear();
        app.features.desktop.last_toast = None;
        let mut message = app.state.model.messages[&Id(12)]
            .values()
            .next()
            .unwrap()
            .clone();
        message.author_id = Id(2);
        message.content = "An ordinary non-mention message".into();
        message.mention_ids.clear();
        app.settings.preferences.notification_level = NotificationLevel::Nothing;
        let seq = app.state.model.cursor.as_ref().unwrap().seq + 1;
        let author = app.state.model.users[&Id(2)].clone();
        let _ = app.network(Data::Frame(ServerMsg::Event(Box::new(EventEnvelope {
            seq,
            event: Event::MessageCreated {
                message: message.clone(),
                author: author.clone(),
            },
        }))));
        assert!(app.features.desktop.last_toast.is_none());
        app.settings.preferences.notification_level = NotificationLevel::All;
        message.id = Id(message.id.0 + 1);
        let _ = app.network(Data::Frame(ServerMsg::Event(Box::new(EventEnvelope {
            seq: seq + 1,
            event: Event::MessageCreated { message, author },
        }))));
        assert!(app.features.desktop.last_toast.is_some());
    }
}
