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
    Sources(Result<Vec<SourceChoice>, String>),
    ChooseSource(usize),
    Fps(u32),
    Height(u32),
    IncludeAudio(bool),
    StartShare,
    StopShare,
    ClosePicker,
    ToggleCamera,
    Cameras(Result<Vec<sticktalk_media::CameraDevice>, String>),
    Settings(VoiceSettings),
    Audio(crate::audio_settings::Event),
    MicTest,
    RecordKey,
    RecordedKey(u32),
    UserVolume(Id, u16),
    UserMute(Id, bool),
    RequestControl(VoiceSession),
    DecideControl(bool),
    ConsentTimeout(u64),
    Revoke,
    Focus(Option<VoiceSession>),
    Stats,
    Save(u64),
    Saved(Result<(), String>),
}
#[derive(Default)]
pub struct State {
    pub sound_hook: Option<fn(voice::Sound)>,
    pub call: Call,
    pub config: Option<voice::Config>,
    pub handle: Option<voice::Handle>,
    pub picker: bool,
    pub sources: Vec<SourceChoice>,
    pub source: Option<usize>,
    pub sharing: bool,
    pub camera: bool,
    pub focus: Option<VoiceSession>,
    pub stats: bool,
    pub decoded: std::collections::BTreeMap<(Id, u8), u64>,
    pub settings_path: Option<std::path::PathBuf>,
    pub save_revision: u64,
    pub save_dirty: bool,
    pub audio_devices: Vec<sticktalk_media::AudioDevice>,
    pub audio_handle: Option<crate::audio_settings::Handle>,
    pub audio_error: Option<String>,
    pub audio_cameras: Vec<sticktalk_media::CameraDevice>,
    pub mic_test: bool,
    pub mic_level: Option<sticktalk_media::MicLevel>,
    pub recording_key: bool,
}
impl App {
    pub(crate) fn mock_call_screen(&mut self, screen: &str) {
        if !matches!(
            screen,
            "voice-call"
                | "share-picker"
                | "voice-consent"
                | "voice-settings"
                | "audio-default"
                | "audio-mic-test"
                | "audio-manual"
                | "audio-ptt"
                | "audio-devices"
                | "audio-bluetooth"
                | "audio-processing"
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
        if screen == "share-picker" {
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
                thumbnail: None,
                process_id: None,
            }];
            self.calls.source = Some(0);
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
        if screen == "voice-settings" || screen.starts_with("audio-") {
            self.calls.audio_devices = crate::audio_settings::fixture_devices();
            self.calls.call.settings.auto_high_quality = Some(true);
            if screen == "audio-mic-test" || screen == "audio-manual" {
                self.calls.mic_test = true;
                self.calls.mic_level = Some(sticktalk_media::MicLevel {
                    rms_dbfs: -28.0,
                    peak_dbfs: -16.0,
                    threshold_dbfs: -50.0,
                    clipping: false,
                });
            }
            if screen == "audio-manual" {
                self.calls.call.settings.sensitivity_auto = false;
            }
            if screen == "audio-ptt" {
                self.calls.call.settings.push_to_talk = true;
                self.calls.recording_key = true;
            }
            if screen == "audio-bluetooth" {
                self.calls.call.settings.output_device_id = Some("headphones".into());
                self.calls.call.settings.input_device_id = Some("usb-mic".into());
            }

            self.dialog = crate::app::Dialog::UserSettings;
            self.user_tab = crate::settings_controller::UserPage::Voice.index();
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
                }
                Effect::Gate { muted, deafened } => {
                    self.call_command(MediaCommand::Gate(muted, deafened));
                }
                Effect::ReleaseInput => {} // Native driver deliberately has no live injector.
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
                MediaEvent::Decoded { user, kind, frames } => {
                    self.calls.decoded.insert((user, kind as u8), frames);
                }
            },
            Action::Picker => {
                self.calls.picker = true;
                self.calls.sources.clear();
                self.calls.source = None;
                if !self.state.session.as_ref().is_some_and(|s| s.config.mock) {
                    return Task::perform(
                        async {
                            tokio::task::spawn_blocking(|| {
                                let sources = sticktalk_media::list_sources()?;
                                Ok::<_, sticktalk_media::Error>(
                                    sources
                                        .into_iter()
                                        .take(32)
                                        .map(|source| {
                                            let process_id =
                                                sticktalk_media::source_process_id(&source);
                                            let thumbnail =
                                                sticktalk_media::source_thumbnail(&source)
                                                    .ok()
                                                    .map(|t| {
                                                        iced::widget::image::Handle::from_rgba(
                                                            t.width, t.height, t.rgba,
                                                        )
                                                    });
                                            SourceChoice {
                                                source,
                                                process_id,
                                                thumbnail,
                                            }
                                        })
                                        .collect(),
                                )
                            })
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|v| v.map_err(|e| e.to_string()))
                        },
                        |s| Msg::Call(Action::Sources(s)),
                    );
                }
            }
            Action::Sources(s) => match s {
                Ok(s) => self.calls.sources = s,
                Err(e) => self.error = e,
            },
            Action::ChooseSource(i) => {
                self.calls.source = (i < self.calls.sources.len()).then_some(i)
            }
            Action::ClosePicker => self.calls.picker = false,
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
                    if self.calls.call.settings.share_include_audio && s.process_id.is_none() {
                        self.error =
                            "Choose an application process for audio before starting the share."
                                .into();
                    } else {
                        let pid = self
                            .calls
                            .call
                            .settings
                            .share_include_audio
                            .then_some(s.process_id)
                            .flatten();
                        self.call_command(MediaCommand::Share(s.source, pid));
                        self.calls.picker = false;
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
                if let Some(camera) = cameras
                    .iter()
                    .find(|c| Some(&c.id) == self.calls.call.settings.camera_device_id.as_ref())
                    .or_else(|| cameras.first())
                {
                    self.call_command(MediaCommand::Camera(Some(camera.clone())));
                } else {
                    self.error = "No camera found.".into();
                }
            }
            Action::Cameras(Err(e)) => self.error = e,
            Action::Audio(event) => match event {
                crate::audio_settings::Event::Ready(handle) => {
                    handle.configure(self.calls.call.settings.clone());
                    self.calls.audio_handle = Some(handle);
                }
                crate::audio_settings::Event::Devices(devices, cameras) => {
                    self.calls.audio_devices = devices;
                    self.calls.audio_cameras = cameras;
                }
                crate::audio_settings::Event::Level(level) => {
                    if self.calls.mic_test {
                        self.calls.mic_level = Some(level);
                    }
                }
                crate::audio_settings::Event::Probe(high) => {
                    self.calls.call.settings.auto_high_quality = Some(high);
                    if let Some(handle) = &self.calls.audio_handle {
                        handle.configure(self.calls.call.settings.clone());
                    }
                    self.call_save();
                }
                crate::audio_settings::Event::Error(e) => {
                    self.calls.mic_test = false;
                    eprintln!("Audio settings: {e}");
                    self.calls.audio_error = Some("Audio is unavailable. Check your device and Windows microphone permissions, then try again.".into());
                }
            },
            Action::MicTest => {
                self.calls.audio_error = None;
                self.calls.mic_test = !self.calls.mic_test;
                self.calls.mic_level = None;
            }
            Action::RecordKey => {
                self.calls.recording_key = !self.calls.recording_key;
                self.call_command(MediaCommand::PushToTalk(false));
                let mut live = self.calls.call.settings.clone();
                if self.calls.recording_key {
                    live.push_to_talk_key = None;
                }
                self.call_command(MediaCommand::Configure(live));
            }
            Action::RecordedKey(key) => {
                if self.calls.recording_key {
                    self.calls.recording_key = false;
                    if key != 0x1b {
                        self.calls.call.settings.push_to_talk_key = Some(key);
                        self.call_apply_settings();
                    } else {
                        self.call_command(MediaCommand::Configure(
                            self.calls.call.settings.clone(),
                        ));
                    }
                }
            }
            Action::Settings(s) => {
                if let Err(e) = s.validate() {
                    self.error = e;
                } else {
                    // Clicking another setting ends key capture and restores
                    // the saved hook, rather than leaving a hidden recorder.
                    self.calls.recording_key = false;
                    self.calls.call.settings = s.clone();
                    self.calls.audio_error = None;
                    if let Some(handle) = &self.calls.audio_handle {
                        handle.configure(s.clone());
                    }
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
                self.call_signal(VoiceSignal::RequestControl { sharer })
            }
            Action::DecideControl(allow) => {
                // Never grant OS input without the required native safety banner.
                if allow {
                    self.error =
                        "Remote control is unavailable until the safety banner is installed."
                            .into();
                }
                if let Some((requester, _)) =
                    self.calls.call.control.decide(crate::mock::now(), false)
                {
                    self.call_signal(VoiceSignal::DecideControl {
                        requester,
                        allow: false,
                    });
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
            Action::Revoke => {
                self.calls.call.control.clear();
                self.call_signal(VoiceSignal::RevokeControl);
            }
            Action::Focus(focus) => self.calls.focus = focus,
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
        self.main.muted = self.calls.call.muted;
        self.main.deafened = self.calls.call.deafened;
        Task::none()
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
