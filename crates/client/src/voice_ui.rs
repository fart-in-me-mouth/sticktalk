//! Small main-window integration surface; widgets live in ui/call and ui/share_picker.
use crate::{
    app::{App, Msg, Screen},
    backend::{self, Data, Request},
    model::ConnectionState,
};
use iced::Task;
use sticktalk::voice::{self, Command as MediaCommand, Event as MediaEvent};
use sticktalk_client_core::voice::{Call, Effect, Phase, VoiceSettings};
use sticktalk_proto::*;

#[derive(Debug, Clone)]
pub struct SourceChoice {
    pub source: sticktalk_media::Source,
    pub thumbnail: Option<iced::widget::image::Handle>,
    pub gpu_thumbnail: Option<std::sync::Arc<sticktalk_media::DisplayFrame>>,
    pub preview_elapsed: std::time::Duration,
    pub process_id: Option<u32>,
}
impl std::ops::Deref for SourceChoice {
    type Target = sticktalk_media::Source;
    fn deref(&self) -> &Self::Target {
        &self.source
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    Join(Id),
    Leave,
    Mute,
    Deafen,
    Media(u64, MediaEvent),
    Picker,
    Sources(u64, Result<Vec<SourceChoice>, String>),
    ChooseSource(usize),
    Fps(u32),
    Height(u32),
    IncludeAudio(bool),
    AudioProcess(u32),
    AudioMenu,
    StartShare,
    StopShare,
    ClosePicker,
    ToggleCamera,
    Cameras(Result<Vec<sticktalk_media::CameraDevice>, String>),
    Settings(VoiceSettings),
    UserVolume(Id, u16),
    UserMute(Id, bool),
    RequestControl(VoiceSession),
    DecideControl(bool),
    ConsentTimeout(u64),
    Revoke,
    ViewerInput(ControlInput),
    ViewerEscapeTap,
    Focus(Option<VoiceSession>),
    Stats,
    Visibility(Vec<(VoiceSession, TrackKind)>),
    PickerTab(bool),
    QualityMenu,
    RefreshSources,
    ChooseCamera(usize),
    CloseCameras,
    Fullscreen,
    Popout,
    WindowClosed(iced::window::Id),
    WindowFocus(iced::window::Id, bool),
    WindowHidden(iced::window::Id, Option<bool>),
    WindowResized(iced::window::Id, iced::Size),
    Save(u64),
    Saved(Result<(), String>),
}
#[derive(Debug, Clone)]
pub struct VideoState {
    pub frame: std::sync::Arc<sticktalk_media::DisplayFrame>,
    pub decode_ms: f32,
    pub bps: u64,
    pub dropped: u64,
    pub fps: f32,
    pub sampled_at: std::time::Instant,
    pub received: u32,
}
#[derive(Default)]
pub struct State {
    pub sound_hook: Option<fn(voice::Sound)>,
    pub call: Call,
    pub config: Option<voice::Config>,
    pub handle: Option<voice::Handle>,
    pub picker: bool,
    pub picker_revision: u64,
    pub source_generation: u64,
    pub picker_loading: bool,
    pub picker_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub sources: Vec<SourceChoice>,
    pub source: Option<usize>,
    pub active_source: Option<sticktalk_media::Source>,
    pub audio_source: Option<u32>,
    pub audio_menu: bool,
    pub sharing: bool,
    pub camera: bool,
    pub focus: Option<VoiceSession>,
    pub stats: bool,
    pub input_sequence: u64,
    pub mock_video: Option<iced::widget::image::Handle>,
    pub mock_playback: bool,
    pub preview_only: bool,
    pub picker_windows: bool,
    pub quality_menu: bool,
    pub camera_picker: bool,
    pub cameras: Vec<sticktalk_media::CameraDevice>,
    pub fullscreen: bool,
    pub main_window: Option<iced::window::Id>,
    pub hidden_windows: std::collections::BTreeSet<iced::window::Id>,
    pub popout: Option<iced::window::Id>,
    pub popout_size: Option<iced::Size>,
    pub videos: std::collections::BTreeMap<(VoiceSession, u8), VideoState>,
    pub decoded: std::collections::BTreeMap<(Id, u8), u64>,
    pub settings_path: Option<std::path::PathBuf>,
    pub save_revision: u64,
    pub save_dirty: bool,
}
impl App {
    pub(crate) fn mock_call_screen(&mut self, screen: &str) {
        if screen == "source-preview" {
            self.calls.mock_playback = true;
            self.calls.preview_only = true;
            return;
        }

        if !matches!(
            screen,
            "voice-call"
                | "share-picker"
                | "voice-consent"
                | "voice-settings"
                | "focused-stream"
                | "stream-stats"
                | "share-picker-windows"
                | "share-quality"
                | "controlling-banner"
                | "request-control"
                | "camera-tiles"
                | "camera-chooser"
        ) {
            return;
        }
        let Some(mut channel) = self
            .state
            .model
            .channels
            .values()
            .find(|c| c.guild_id.is_some())
            .cloned()
        else {
            return;
        };
        channel.id = Id(880001);
        channel.kind = ChannelKind::Voice;
        channel.name = Some("Lounge".into());
        let id = channel.id;
        let guild = channel.guild_id.unwrap();
        self.state.model.channels.insert(id, channel);
        self.state.guild = Some(guild);
        self.state.screen = Screen::Channel(id);
        self.calls.call.phase = Phase::Connected;
        self.calls.call.channel = Some(id);
        self.calls.call.desired_channel = Some(id);
        for user in [Id(1), Id(2), Id(3)] {
            let session = VoiceSession {
                user_id: user,
                connection_id: Id(900 + user.0),
            };
            let mut p = VoiceParticipant {
                session,
                muted: user == Id(3),
                deafened: false,
                tracks: vec![],
            };
            if user == Id(2) {
                p.tracks.push(PublishedTrack {
                    mid: "screen".into(),
                    kind: TrackKind::Screen,
                    codec: Some(VideoCodec::H264),
                    layers: vec![],
                    surface: Some(ShareSurface::Monitor),
                });
            }
            if user == self.me().unwrap_or(Id(1)) {
                self.calls.call.own = Some(session);
                self.calls.call.speaking.insert(session);
            }
            self.calls.call.participants.insert(session, p.clone());
            self.calls.call.guild_states.insert(
                session,
                GuildVoiceState {
                    guild_id: guild,
                    channel_id: id,
                    participant: p,
                },
            );
        }
        if matches!(
            screen,
            "share-picker" | "share-picker-windows" | "share-quality"
        ) {
            self.calls.picker = true;
            self.calls.sources = vec![SourceChoice {
                source: sticktalk_media::Source {
                    kind: sticktalk_media::SourceKind::Monitor,
                    id: 0,
                    name: "Screen 1".into(),
                    size: sticktalk_media::Size {
                        width: 2560,
                        height: 1440,
                    },
                    primary: true,
                },
                thumbnail: Some(iced::widget::image::Handle::from_path(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/assets/call-preview.png"
                ))),
                gpu_thumbnail: None,
                preview_elapsed: std::time::Duration::ZERO,
                process_id: None,
            }];
            self.calls.sources.push(SourceChoice {
                source: sticktalk_media::Source {
                    kind: sticktalk_media::SourceKind::Window,
                    id: 0,
                    name: "Sprint planning — Design".into(),
                    size: sticktalk_media::Size {
                        width: 1920,
                        height: 1080,
                    },
                    primary: false,
                },
                thumbnail: Some(iced::widget::image::Handle::from_path(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/assets/call-preview.png"
                ))),
                gpu_thumbnail: None,
                preview_elapsed: std::time::Duration::ZERO,
                process_id: None,
            });
            self.calls.source = Some(0);
            self.calls.picker_windows = screen == "share-picker-windows";
            if self.calls.picker_windows {
                self.calls.source = Some(1);
            }
            self.calls.quality_menu = screen == "share-quality";
        }
        self.calls.mock_playback = true;
        if matches!(
            screen,
            "focused-stream" | "stream-stats" | "request-control" | "controlling-banner"
        ) {
            self.calls.focus = Some(VoiceSession {
                user_id: Id(2),
                connection_id: Id(902),
            });
        }
        if screen == "stream-stats" {
            self.calls.stats = true;
            self.calls.call.stats = Some(VoiceStats {
                tx_bps: 0,
                rx_bps: 8_240_000,
                tx_fps_milli: 0,
                rx_fps_milli: 59_940,
                loss_per_million: Some(0),
                rtt_ms: Some(12),
                path: MediaPath::Direct,
                cap_dropped_packets: 0,
                queue_dropped_packets: 0,
                mtu_dropped_packets: 0,
            });
        }
        if screen == "camera-tiles" {
            for p in self.calls.call.participants.values_mut() {
                p.tracks.push(PublishedTrack {
                    mid: "camera".into(),
                    kind: TrackKind::Camera,
                    codec: Some(VideoCodec::H264),
                    layers: vec![],
                    surface: None,
                });
            }
        }
        if screen == "camera-chooser" {
            self.calls.camera_picker = true;
            self.calls.cameras = vec![
                sticktalk_media::CameraDevice {
                    id: "mock-1".into(),
                    name: "Integrated Camera".into(),
                },
                sticktalk_media::CameraDevice {
                    id: "mock-2".into(),
                    name: "USB HD Camera".into(),
                },
            ];
        }
        if screen == "controlling-banner" {
            let own = self.calls.call.own.unwrap();
            let track = self
                .calls
                .call
                .participants
                .values()
                .flat_map(|p| &p.tracks)
                .find(|t| t.kind == TrackKind::Screen)
                .cloned();
            for p in self.calls.call.participants.values_mut() {
                p.tracks.retain(|t| t.kind != TrackKind::Screen);
                if p.session == own {
                    p.tracks.extend(track.clone());
                }
            }
            self.calls.sharing = true;
            self.calls.focus = Some(own);
            self.calls.call.control.granted(
                self.calls.call.own.unwrap(),
                VoiceSession {
                    user_id: Id(2),
                    connection_id: Id(902),
                },
                42,
            );
        }
        if screen == "voice-consent" {
            self.calls.call.control.request(
                VoiceSession {
                    user_id: Id(2),
                    connection_id: Id(902),
                },
                crate::mock::now(),
                30_000,
            );
        }
        if screen == "voice-settings" {
            self.dialog = crate::app::Dialog::UserSettings;
            self.user_tab = 3;
        }
    }
    pub fn call_effects(&mut self, effects: Vec<Effect>, nonce: Option<u64>) {
        for effect in effects {
            match effect {
                Effect::Signal(signal) => {
                    let nonce = nonce.unwrap_or_else(backend::nonce);
                    match sticktalk_client_core::voice::encode(nonce, signal) {
                        Ok(v) => {
                            self.request(Request::Voice(v));
                        }
                        Err(e) => self.error = e,
                    }
                }
                Effect::StartTransport { max_rtp_bytes } => {
                    if !self.state.session.as_ref().is_some_and(|s| s.config.mock) {
                        self.calls.config = Some(voice::Config {
                            bind_ip: std::env::var("STICKTALK_CLIENT_VOICE_BIND_IP")
                                .unwrap_or_else(|_| "127.0.0.1".into()),
                            generation: backend::nonce(),
                            mtu: max_rtp_bytes,
                            settings_json: serde_json::to_string(&self.calls.call.settings)
                                .unwrap_or_default(),
                        });
                    }
                }
                Effect::StopTransport => {
                    if let Some(handle) = self.calls.handle.take() {
                        let _ = handle.send(MediaCommand::Stop);
                    }
                    self.calls.config = None;
                    self.calls.sharing = false;
                    self.calls.camera = false;
                    self.calls.decoded.clear();
                    self.calls.videos.clear();
                    self.calls.picker = false;
                    self.calls.picker_loading = false;
                    if let Some(cancel) = self.calls.picker_cancel.take() {
                        cancel.store(true, std::sync::atomic::Ordering::Release);
                    }
                    self.calls.sources.clear();
                }
                Effect::Gate { muted, deafened } => {
                    self.call_command(MediaCommand::Gate(muted, deafened));
                }
                Effect::ReleaseInput => self.call_command(MediaCommand::ReleaseInput),
            }
        }
    }
    pub fn call_command(&mut self, c: MediaCommand) {
        if let Some(h) = &self.calls.handle {
            if let Err(e) = h.send(c) {
                self.error = e;
            }
        }
    }
    pub fn call_signal(&mut self, s: VoiceSignal) {
        self.call_effects(vec![Effect::Signal(s)], None);
    }
    pub fn call_network(&mut self, data: &Data) {
        match data {
            Data::State(s) if !matches!(s, ConnectionState::Ready) => {
                let effects = self.calls.call.disconnected();
                self.call_effects(effects, None);
            }
            Data::Frame(ServerMsg::Ready(ready)) => {
                self.calls.call.guild_states = ready
                    .voice_states
                    .iter()
                    .map(|s| (s.participant.session, s.clone()))
                    .collect();
                let nonce = backend::nonce();
                let effects = self.calls.call.ready(nonce);
                self.call_effects(effects, Some(nonce));
                let path =
                    crate::storage::root().join(format!("voice-{}.json", ready.me.user.id.0));
                // Mock/screenshot sessions never read or change real preferences.
                if !self.state.session.as_ref().is_some_and(|s| s.config.mock)
                    && self.calls.settings_path.as_ref() != Some(&path)
                {
                    self.calls.call.settings = voice::load_settings(&path);
                    self.calls.settings_path = Some(path);
                }
            }
            Data::Frame(ServerMsg::Resumed(_)) => {
                let nonce = backend::nonce();
                let effects = self.calls.call.ready(nonce);
                self.call_effects(effects, Some(nonce));
            }
            Data::Frame(ServerMsg::Voice(payload)) => {
                match sticktalk_client_core::voice::decode(payload) {
                    Ok(event) => {
                        let me = self.me().unwrap_or(Id(0));
                        let effects =
                            self.calls
                                .call
                                .event(payload.nonce, &event, me, crate::mock::now());
                        self.call_effects(effects, None);
                        self.call_command(MediaCommand::Event(event.clone()));
                        self.call_command(MediaCommand::Participants(
                            self.calls.call.own,
                            self.calls.call.participants.values().cloned().collect(),
                        ));
                        let name = self
                            .calls
                            .call
                            .control
                            .grant
                            .map(|g| self.state.model.name(g.controller.user_id))
                            .unwrap_or_default();
                        self.call_command(MediaCommand::ControlState(
                            self.calls.call.control.clone(),
                            self.calls.call.own,
                            name,
                        ));
                        if let VoiceEvent::Error { message, .. } = event {
                            self.error = message;
                        }
                    }
                    Err(e) => self.error = e,
                }
            }
            _ => {}
        }
    }
    pub fn call_update(&mut self, action: Action) -> Task<Msg> {
        match action {
            Action::Join(channel) => {
                if self.calls.call.desired_channel == Some(channel) {
                    self.state.screen = Screen::Channel(channel);
                    return Task::none();
                }
                self.state.screen = Screen::Channel(channel);
                self.state.guild = self
                    .state
                    .model
                    .channels
                    .get(&channel)
                    .and_then(|c| c.guild_id);
                let nonce = backend::nonce();
                let effects = self.calls.call.join(channel, nonce);
                self.call_effects(effects, Some(nonce));
            }
            Action::Leave => {
                if self.calls.call.desired_channel.is_some() {
                    self.call_cue(voice::Sound::VoiceLeave);
                }
                let effects = self.calls.call.leave();
                self.call_effects(effects, None);
                self.calls.videos.clear();
                return self.call_update(Action::Focus(None));
            }
            Action::Mute => {
                let effects = self
                    .calls
                    .call
                    .set_state(!self.calls.call.muted, self.calls.call.deafened);
                self.call_effects(effects, None);
                self.call_cue(if self.calls.call.muted {
                    voice::Sound::Mute
                } else {
                    voice::Sound::Unmute
                });
            }
            Action::Deafen => {
                let effects = self
                    .calls
                    .call
                    .set_state(self.calls.call.muted, !self.calls.call.deafened);
                self.call_effects(effects, None);
            }
            Action::Media(generation, _)
                if self.calls.config.as_ref().map(|c| c.generation) != Some(generation) => {}
            Action::Media(_, event) => match event {
                MediaEvent::Ready(handle) => {
                    self.calls.handle = Some(handle);
                    self.call_command(MediaCommand::Visible(self.call_visible()));
                    let gate = self.calls.call.gate();
                    self.call_effects(vec![gate], None);
                    self.call_command(MediaCommand::Participants(
                        self.calls.call.own,
                        self.calls.call.participants.values().cloned().collect(),
                    ));
                }
                MediaEvent::Signal(signal) => self.call_signal(signal),
                MediaEvent::Connected => {
                    self.calls.call.phase = Phase::Connected;
                    self.call_cue(voice::Sound::VoiceJoin);
                }
                MediaEvent::Error(e) => {
                    self.error = e;
                    let effects = self.calls.call.leave();
                    self.call_effects(effects, None);
                }
                MediaEvent::Toast(s) => self.notice = s,
                MediaEvent::Sharing(active) => self.calls.sharing = active,
                MediaEvent::Camera(active) => self.calls.camera = active,
                MediaEvent::Speaking(None, speaking) => {
                    if let Some(own) = self.calls.call.own {
                        if speaking {
                            self.calls.call.speaking.insert(own);
                        } else {
                            self.calls.call.speaking.remove(&own);
                        }
                    }
                }
                MediaEvent::Speaking(Some(_), _) => {} // SFU level events identify remote sessions.
                MediaEvent::Hotkey(key) => match key {
                    voice::hotkeys::Hotkey::Mute => return self.call_update(Action::Mute),
                    voice::hotkeys::Hotkey::Deafen => return self.call_update(Action::Deafen),
                    voice::hotkeys::Hotkey::Revoke => {
                        if self.calls.call.control.grant.is_some() {
                            return self.call_update(Action::Revoke);
                        }
                    }
                    voice::hotkeys::Hotkey::PushToTalk(_) => {}
                },
                MediaEvent::Video {
                    session,
                    kind,
                    frame,
                    decode_ms,
                    dropped,
                    bps,
                } => {
                    if self.call_visible().contains(&(session, kind)) {
                        let v = self
                            .calls
                            .videos
                            .entry((session, kind as u8))
                            .or_insert_with(|| VideoState {
                                frame: frame.clone(),
                                decode_ms,
                                dropped,
                                bps,
                                fps: 0.,
                                sampled_at: std::time::Instant::now(),
                                received: 0,
                            });
                        v.frame = frame;
                        v.decode_ms = decode_ms;
                        v.bps = bps;
                        v.dropped = dropped;
                        v.received += 1;
                        if v.sampled_at.elapsed().as_secs_f32() >= 1. {
                            v.fps = v.received as f32 / v.sampled_at.elapsed().as_secs_f32();
                            v.received = 0;
                            v.sampled_at = std::time::Instant::now();
                        }
                    }
                }
                MediaEvent::Decoded { user, kind, frames } => {
                    self.calls.decoded.insert((user, kind as u8), frames);
                }
            },
            Action::RefreshSources if !self.calls.picker => {}
            Action::RefreshSources | Action::Picker => {
                self.calls.picker = true;
                if self.calls.picker_loading {
                    return Task::none();
                }
                self.calls.picker_loading = true;
                self.calls.source_generation = self.calls.source_generation.wrapping_add(1);
                let generation = self.calls.source_generation;
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                self.calls.picker_cancel = Some(cancel.clone());

                if !self.state.session.as_ref().is_some_and(|s| s.config.mock) {
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                let sources = sticktalk_media::list_sources()?;
                                #[cfg(feature = "wgpu")]
                                let thumbnailer = sticktalk_media::GpuThumbnailer::new().ok();
                                Ok::<_, sticktalk_media::Error>(
                                    sources
                                        .into_iter()
                                        .take(32)
                                        .take_while(|_| {
                                            !cancel.load(std::sync::atomic::Ordering::Acquire)
                                        })
                                        .map(|source| {
                                            let process_id =
                                                sticktalk_media::source_process_id(&source);
                                            let preview_started = std::time::Instant::now();
                                            #[cfg(feature = "wgpu")]
                                            let gpu_thumbnail = thumbnailer
                                                .as_ref()
                                                .and_then(|t| t.thumbnail(&source).ok());
                                            #[cfg(not(feature = "wgpu"))]
                                            let gpu_thumbnail = None;
                                            let thumbnail = if gpu_thumbnail.is_some() {
                                                None
                                            } else {
                                                sticktalk_media::source_thumbnail(&source).ok().map(
                                                    |t| {
                                                        iced::widget::image::Handle::from_rgba(
                                                            t.width, t.height, t.rgba,
                                                        )
                                                    },
                                                )
                                            };
                                            SourceChoice {
                                                source,
                                                process_id,
                                                thumbnail,
                                                gpu_thumbnail,
                                                preview_elapsed: preview_started.elapsed(),
                                            }
                                        })
                                        .collect(),
                                )
                            })
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|v| v.map_err(|e| e.to_string()))
                        },
                        move |s| Msg::Call(Action::Sources(generation, s)),
                    );
                }
            }
            Action::Sources(_, _) if !self.calls.picker => {}
            Action::Sources(generation, _) if generation != self.calls.source_generation => {}
            Action::Sources(_, s) => {
                self.calls.picker_loading = false;
                match s {
                    Ok(s) => {
                        let elapsed = s
                            .iter()
                            .map(|s| s.preview_elapsed.as_secs_f64())
                            .sum::<f64>();
                        let fallback = s.iter().filter(|s| s.thumbnail.is_some()).count();
                        eprintln!("picker snapshot: {} sources, {:.1} ms total, {} measured CPU fallback previews",s.len(),elapsed*1000.,fallback);
                        let selected = self
                            .calls
                            .source
                            .and_then(|i| self.calls.sources.get(i))
                            .map(|s| (s.kind, s.id, s.process_id));
                        self.calls.source = selected.and_then(|(kind, id, pid)| {
                            s.iter()
                                .position(|s| s.kind == kind && s.id == id && s.process_id == pid)
                        });
                        self.calls.sources = s;
                        self.calls.picker_revision = self.calls.picker_revision.wrapping_add(1);
                    }
                    Err(e) => self.error = e,
                }
            }
            Action::ChooseSource(i) => {
                self.calls.source = (i < self.calls.sources.len()).then_some(i)
            }
            Action::ClosePicker => {
                if let Some(cancel) = self.calls.picker_cancel.take() {
                    cancel.store(true, std::sync::atomic::Ordering::Release);
                }
                self.calls.source_generation = self.calls.source_generation.wrapping_add(1);
                self.calls.picker_loading = false;
                self.calls.picker = false;
                self.calls.sources.clear();
                self.calls.source = None;
            }
            Action::PickerTab(windows) => self.calls.picker_windows = windows,
            Action::QualityMenu => self.calls.quality_menu = !self.calls.quality_menu,
            Action::CloseCameras => {
                self.calls.camera_picker = false;
                self.calls.cameras.clear();
            }
            Action::ChooseCamera(i) => {
                if let Some(camera) = self.calls.cameras.get(i).cloned() {
                    self.call_command(MediaCommand::Camera(Some(camera)));
                    self.calls.camera_picker = false;
                }
            }
            Action::Fullscreen => {
                self.calls.fullscreen = !self.calls.fullscreen;
                let mode = if self.calls.fullscreen {
                    iced::window::Mode::Fullscreen
                } else {
                    iced::window::Mode::Windowed
                };
                if let Some(id) = self.calls.popout.or(self.calls.main_window) {
                    return iced::window::set_mode(id, mode);
                }
                return Task::none();
            }
            Action::WindowFocus(id, focused) => {
                if focused {
                    self.calls.hidden_windows.remove(&id);
                }
                let task = self.update(Msg::Focus(focused));
                return Task::batch([
                    task,
                    iced::window::is_minimized(id)
                        .map(move |hidden| Msg::Call(Action::WindowHidden(id, hidden))),
                ]);
            }
            Action::WindowHidden(id, hidden) => {
                if hidden == Some(true) {
                    self.calls.hidden_windows.insert(id);
                } else {
                    self.calls.hidden_windows.remove(&id);
                }
            }
            Action::WindowClosed(id) => {
                if self.calls.popout == Some(id) {
                    self.calls.popout = None;
                    self.calls.fullscreen = false;
                    return iced::window::close(id);
                }
                self.call_command(MediaCommand::ReleaseInput);
                return iced::exit();
            }
            Action::WindowResized(id, size) => {
                if self.calls.popout == Some(id) {
                    self.calls.popout_size = Some(size);
                } else {
                    let resize = self.update(Msg::Resize(size));
                    return Task::batch([
                        resize,
                        iced::window::is_minimized(id)
                            .map(move |hidden| Msg::Call(Action::WindowHidden(id, hidden))),
                    ]);
                }
            }
            Action::Popout => {
                if let Some(id) = self.calls.popout.take() {
                    self.calls.fullscreen = false;
                    return iced::window::close(id);
                }
                let restore = if std::mem::take(&mut self.calls.fullscreen) {
                    self.calls.main_window.map_or_else(Task::none, |id| {
                        iced::window::set_mode(id, iced::window::Mode::Windowed)
                    })
                } else {
                    Task::none()
                };
                let (id, open) = iced::window::open(iced::window::Settings {
                    size: iced::Size::new(960., 600.),
                    min_size: Some(iced::Size::new(640., 400.)),
                    exit_on_close_request: false,
                    visible: !crate::agent_window::enabled(),
                    ..Default::default()
                });
                self.calls.popout = Some(id);
                self.calls.popout_size = Some(iced::Size::new(960., 600.));
                return Task::batch([restore, open.discard()]);
            }
            Action::Visibility(visible) => {
                let release = self.calls.call.control.grant.is_some_and(|g| {
                    Some(g.controller) == self.calls.call.own
                        && !visible.contains(&(g.sharer, TrackKind::Screen))
                });
                self.calls.videos.retain(|(s, k), _| {
                    visible.iter().any(|(v, kind)| v == s && *kind as u8 == *k)
                });
                self.call_command(MediaCommand::Visible(visible));
                if release {
                    return self.call_update(Action::Revoke);
                }
            }
            Action::Fps(fps) => {
                self.calls.call.settings.share_fps = fps;
                self.call_share_quality();
                self.call_save();
            }
            Action::Height(height) => {
                self.calls.call.settings.share_height = height;
                self.call_share_quality();
                self.call_save();
            }
            Action::AudioProcess(pid) => {
                self.calls.audio_source = Some(pid);
                self.calls.audio_menu = false;
            }
            Action::AudioMenu => self.calls.audio_menu = !self.calls.audio_menu,
            Action::IncludeAudio(audio) => {
                self.calls.call.settings.share_include_audio = audio;
                self.call_save();
            }
            Action::StartShare => {
                if let Some(s) = self
                    .calls
                    .source
                    .and_then(|i| self.calls.sources.get(i))
                    .cloned()
                {
                    if self.calls.call.settings.share_include_audio
                        && s.process_id.or(self.calls.audio_source).is_none()
                    {
                        self.error =
                            "Choose an application process for audio before starting the share."
                                .into();
                    } else {
                        let pid = self
                            .calls
                            .call
                            .settings
                            .share_include_audio
                            .then_some(s.process_id.or(self.calls.audio_source))
                            .flatten();
                        let q = match voice::quality(&self.calls.call.settings) {
                            Ok(q) => q,
                            Err(e) => {
                                self.error = e;
                                return Task::none();
                            }
                        };
                        self.calls.active_source = Some(s.source.clone());
                        self.call_command(MediaCommand::Share(s.source, pid, q));
                        return self.call_update(Action::ClosePicker);
                    }
                }
            }
            Action::StopShare => self.call_command(MediaCommand::StopShare),
            Action::ToggleCamera => {
                if self.calls.camera {
                    self.call_command(MediaCommand::Camera(None));
                } else {
                    return Task::perform(
                        async {
                            tokio::task::spawn_blocking(sticktalk_media::camera_devices)
                                .await
                                .map_err(|e| e.to_string())
                                .and_then(|v| v.map_err(|e| e.to_string()))
                        },
                        |r| Msg::Call(Action::Cameras(r)),
                    );
                }
            }
            Action::Cameras(Ok(cameras)) => {
                self.calls.cameras = cameras;
                self.calls.camera_picker = true;
            }
            Action::Cameras(Err(e)) => self.error = e,
            Action::Settings(s) => {
                if let Err(e) = s.validate() {
                    self.error = e;
                } else {
                    self.calls.call.settings = s.clone();
                    self.call_command(MediaCommand::Configure(s));
                    self.call_save();
                }
            }
            Action::UserVolume(id, volume) => {
                let mut u = self.calls.call.settings.user(id);
                u.volume = volume;
                if let Err(e) = self.calls.call.settings.set_user(id, u) {
                    self.error = e;
                } else {
                    self.call_apply_settings();
                }
            }
            Action::UserMute(id, muted) => {
                let mut u = self.calls.call.settings.user(id);
                u.muted = muted;
                if let Err(e) = self.calls.call.settings.set_user(id, u) {
                    self.error = e;
                } else {
                    self.call_apply_settings();
                }
            }
            Action::RequestControl(sharer) => {
                self.call_signal(VoiceSignal::RequestControl { sharer });
                // Request-control must not leave Enter/Space focused on its button.
                return iced_runtime::task::effect(iced_runtime::Action::widget(
                    iced::advanced::widget::operation::focusable::unfocus::<()>(),
                ));
            }
            Action::DecideControl(allow) => {
                if let Some((requester, allow)) =
                    self.calls.call.control.decide(crate::mock::now(), allow)
                {
                    self.call_signal(VoiceSignal::DecideControl { requester, allow });
                }
            }
            Action::ConsentTimeout(deadline) => {
                if self
                    .calls
                    .call
                    .control
                    .pending
                    .is_some_and(|(_, d)| d == deadline)
                {
                    return self.call_update(Action::DecideControl(false));
                }
            }
            Action::ViewerEscapeTap => {
                let _ = self.call_update(Action::ViewerInput(ControlInput::Key {
                    usage: 41,
                    down: true,
                }));
                return self.call_update(Action::ViewerInput(ControlInput::Key {
                    usage: 41,
                    down: false,
                }));
            }
            Action::ViewerInput(input) => {
                if let Some(g) = self
                    .calls
                    .call
                    .control
                    .grant
                    .filter(|g| Some(g.controller) == self.calls.call.own)
                {
                    self.calls.input_sequence = self.calls.input_sequence.wrapping_add(1);
                    self.call_command(MediaCommand::ControlInput(ControlData::Input {
                        sharer: g.sharer,
                        grant: g.token,
                        sequence: self.calls.input_sequence,
                        input,
                    }));
                }
            }
            Action::Revoke => {
                self.calls.call.control.clear();
                self.call_command(MediaCommand::ControlState(
                    self.calls.call.control.clone(),
                    self.calls.call.own,
                    String::new(),
                ));
                self.call_signal(VoiceSignal::RevokeControl);
            }
            Action::Focus(focus) => {
                self.calls.focus = focus;
                if focus.is_none() {
                    if let Some(id) = self.calls.popout.take() {
                        self.calls.fullscreen = false;
                        return iced::window::close(id);
                    }
                    if std::mem::take(&mut self.calls.fullscreen) {
                        return self.calls.main_window.map_or_else(Task::none, |id| {
                            iced::window::set_mode(id, iced::window::Mode::Windowed)
                        });
                    }
                }
            }
            Action::Stats => self.calls.stats = !self.calls.stats,
            Action::Save(revision) if revision == self.calls.save_revision => {
                self.calls.save_dirty = false;
                if let Some(path) = self.calls.settings_path.clone() {
                    let settings = self.calls.call.settings.clone();
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                voice::save_settings(&path, &settings)
                            })
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|v| v)
                        },
                        |r| Msg::Call(Action::Saved(r)),
                    );
                }
            }
            Action::Save(_) => {}
            Action::Saved(Err(e)) => self.error = e,
            Action::Saved(Ok(())) => {}
        }
        Task::none()
    }
    pub fn call_visible(&self) -> Vec<(VoiceSession, TrackKind)> {
        let showing = matches!(self.state.screen, Screen::Channel(c) if self.calls.call.desired_channel == Some(c))
            && !matches!(
                self.dialog,
                crate::app::Dialog::UserSettings | crate::app::Dialog::GuildSettings(_)
            )
            && self
                .calls
                .main_window
                .is_none_or(|id| !self.calls.hidden_windows.contains(&id));
        if !showing {
            return self
                .calls
                .popout
                .filter(|id| !self.calls.hidden_windows.contains(id))
                .and(self.calls.focus)
                .map(|s| vec![(s, TrackKind::Screen)])
                .unwrap_or_default();
        }
        self.calls
            .call
            .participants
            .values()
            .flat_map(|p| {
                p.tracks.iter().filter_map(move |t| match t.kind {
                    TrackKind::Camera if self.calls.focus.is_none() => Some((p.session, t.kind)),
                    TrackKind::Screen if self.calls.focus == Some(p.session) => {
                        Some((p.session, t.kind))
                    }
                    _ => None,
                })
            })
            .take(8)
            .collect()
    }
    fn call_apply_settings(&mut self) {
        self.call_command(MediaCommand::Configure(self.calls.call.settings.clone()));
        self.call_save();
    }
    fn call_cue(&self, cue: voice::Sound) {
        if let Some(hook) = self.calls.sound_hook {
            hook(cue);
        }
    }
    fn call_save(&mut self) {
        if let Err(e) = self.calls.call.settings.validate() {
            self.error = e;
            return;
        }
        self.calls.save_revision = self.calls.save_revision.wrapping_add(1);
        self.calls.save_dirty = self.calls.settings_path.is_some();
    }
    fn call_share_quality(&mut self) {
        if self.calls.sharing {
            if let Ok(q) = voice::quality(&self.calls.call.settings) {
                self.call_command(MediaCommand::Quality(q));
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Visible(pub Vec<(VoiceSession, TrackKind)>);
impl std::hash::Hash for Visible {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.len().hash(state);
        for (session, kind) in &self.0 {
            session.hash(state);
            (*kind as u8).hash(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn popout_and_close_clear_fullscreen_ownership_without_polling_os_tasks() {
        let mut app = App::headless("focused-stream", iced::Size::new(1280., 800.));
        app.calls.main_window = Some(iced::window::Id::unique());
        app.calls.fullscreen = true;
        // These Tasks are deliberately never polled: no OS window is opened.
        let _unpolled = app.call_update(Action::Popout);
        assert!(!app.calls.fullscreen);
        assert!(app.calls.popout.is_some());
        app.calls.fullscreen = true;
        let _unpolled = app.call_update(Action::Focus(None));
        assert!(app.calls.popout.is_none());
        assert!(!app.calls.fullscreen);
    }
    #[test]
    fn hidden_screen_unsubscribes_and_popout_keeps_only_its_stream() {
        let mut app = App::headless("voice-call", iced::Size::new(1280., 800.));
        assert!(app.call_visible().is_empty());
        let session = VoiceSession {
            user_id: Id(2),
            connection_id: Id(902),
        };
        app.calls.focus = Some(session);
        assert_eq!(app.call_visible(), vec![(session, TrackKind::Screen)]);
        app.dialog = crate::app::Dialog::UserSettings;
        assert!(app.call_visible().is_empty());
        app.calls.popout = Some(iced::window::Id::unique());
        assert_eq!(app.call_visible(), vec![(session, TrackKind::Screen)]);
        app.calls.popout = None;
        app.calls.focus = None;
        assert!(app.call_visible().is_empty());
    }
}
