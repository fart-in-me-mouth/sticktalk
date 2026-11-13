//! Desktop call lifetime. Importing this module never opens a device or socket.
pub mod control;
mod control_banner;
pub mod hotkeys;
#[cfg(windows)]
mod native;
pub mod peer;
pub mod rtp;
#[cfg(windows)]
mod socket_wait;
#[cfg(feature = "wgpu")]
pub mod video;
use std::sync::{
    mpsc::{SyncSender, TrySendError},
    Arc, Mutex,
};
use sticktalk_client_core::voice::{settings::NoiseSuppression, VoiceSettings};
pub use sticktalk_desktop_platform::sound::Sound;
use sticktalk_media::{
    voice::{InputMode, NoiseSuppressionMode, VoiceOptions},
    FrameRate, Quality, Resolution,
};
use sticktalk_proto::*;

/// Host policy hook. Never call with true without explicit audible consent and
/// known quiet-state policy; headless/mock hosts leave the callback unset.
pub fn play_cue(sound: Sound, quiet_state_confirmed: bool) -> Result<bool, String> {
    sticktalk_desktop_platform::windows::play_sound(sound, quiet_state_confirmed)
        .map_err(|e| e.to_string())
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Config {
    pub bind_ip: String,
    pub generation: u64,
    pub mtu: u16,
    pub settings_json: String,
}
pub fn bind_address(value: &str) -> Result<std::net::IpAddr, String> {
    let ip: std::net::IpAddr = value
        .parse()
        .map_err(|_| "Voice bind address must be a literal IP")?;
    if ip.is_unspecified()
        || ip.is_multicast()
        || matches!(ip,std::net::IpAddr::V4(v4) if v4.is_broadcast())
    {
        return Err("Voice bind address must be a specific unicast IP".into());
    }
    Ok(ip)
}
/// Only audible, currently subscribed speakers may duck other applications.
/// This pure policy is also exercised by the no-device tests.
pub fn attenuation_active(
    settings: &VoiceSettings,
    deafened: bool,
    mut speakers: impl Iterator<Item = Id>,
) -> bool {
    settings.attenuation > 0
        && !deafened
        && settings.output_volume > 0
        && speakers.any(|id| {
            let user = settings.user(id);
            !user.muted && user.volume > 0
        })
}

#[cfg(test)]
mod wake_tests {
    use super::*;
    #[test]
    fn attenuation_tracks_only_audible_present_speakers() {
        let mut settings = VoiceSettings {
            attenuation: 30,
            ..Default::default()
        };
        let speaking = || std::iter::once(Id(2));
        assert!(attenuation_active(&settings, false, speaking()));
        assert!(!attenuation_active(&settings, true, speaking()));
        assert!(!attenuation_active(&settings, false, std::iter::empty()));
        settings
            .set_user(
                Id(2),
                sticktalk_client_core::voice::settings::UserAudio {
                    volume: 100,
                    muted: true,
                },
            )
            .unwrap();
        assert!(!attenuation_active(&settings, false, speaking()));
        settings
            .set_user(
                Id(2),
                sticktalk_client_core::voice::settings::UserAudio {
                    volume: 0,
                    muted: false,
                },
            )
            .unwrap();
        assert!(!attenuation_active(&settings, false, speaking()));
        settings.output_volume = 0;
        assert!(!attenuation_active(&settings, false, speaking()));
    }
    #[test]
    fn queued_command_wakes_blocked_transport_without_a_poll_timer() {
        let socket = std::net::UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let h = Handle(
            tx,
            Default::default(),
            std::sync::Arc::new(std::sync::Mutex::new(Some(socket.try_clone().unwrap()))),
        );
        h.send(Command::Stop).unwrap();
        let mut bytes = [0u8; 1];
        assert_eq!(socket.recv_from(&mut bytes).unwrap().0, 0);
        assert!(matches!(rx.try_recv(), Ok(Command::Stop)));
    }
}
#[derive(Debug, Clone)]
pub enum Command {
    Event(VoiceEvent),
    Participants(Option<VoiceSession>, Vec<VoiceParticipant>),
    Gate(bool, bool),
    PushToTalk(bool),
    Configure(VoiceSettings),
    Share(
        sticktalk_media::Source,
        Option<u32>,
        sticktalk_media::Quality,
    ),
    StopShare,
    Quality(Quality),
    Camera(Option<sticktalk_media::CameraDevice>),
    Visible(Vec<(VoiceSession, TrackKind)>),
    ControlState(
        sticktalk_client_core::voice::control::Consent,
        Option<VoiceSession>,
        String,
    ),
    ControlInput(ControlData),
    ReleaseInput,
    Stop,
}
type Emergency = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;
#[derive(Clone)]
pub struct Handle(
    SyncSender<Command>,
    Emergency,
    Arc<Mutex<Option<std::net::UdpSocket>>>,
);
impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Handle").finish()
    }
}
impl Handle {
    pub fn send(&self, command: Command) -> Result<(), String> {
        let stop = matches!(
            &command,
            Command::Stop | Command::ReleaseInput | Command::StopShare
        ) || matches!(&command,Command::ControlState(consent,own,_) if !own.zip(consent.grant).is_some_and(|(o,g)|consent.may_inject(o,g.token)));
        // Emergency notification must precede the bounded queue: it works even
        // during a blocked codec/device operation or an overloaded command queue.
        if stop {
            if let Ok(callback) = self.1.lock() {
                if let Some(callback) = &*callback {
                    callback();
                }
            }
        }
        self.0.try_send(command).map_err(|e| -> String {
            match e {
                TrySendError::Full(_) => "Media command queue full; retry".into(),
                _ => "Call stopped".into(),
            }
        })?;
        if let Some(socket) = self.2.lock().unwrap().as_ref() {
            if let Ok(address) = socket.local_addr() {
                let _ = socket.send_to(&[], address);
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub enum Event {
    Ready(Handle),
    Signal(VoiceSignal),
    Connected,
    Speaking(Option<u64>, bool),
    Toast(String),
    Error(String),
    Sharing(bool),
    Camera(bool),
    Hotkey(hotkeys::Hotkey),
    Video {
        session: VoiceSession,
        kind: TrackKind,
        frame: std::sync::Arc<sticktalk_media::DisplayFrame>,
        decode_ms: f32,
        dropped: u64,
        bps: u64,
    },
    /// Decoded hardware surfaces remain on the media worker; never CPU-read back.
    Decoded {
        user: Id,
        kind: TrackKind,
        frames: u64,
    },
}
pub fn options(s: &VoiceSettings) -> Result<VoiceOptions, String> {
    s.validate()?;
    let mut o = if s.studio_mode {
        VoiceOptions::studio()
    } else {
        VoiceOptions::default()
    };
    o.packet_duration_ms = 10;
    if !s.studio_mode {
        o.noise_mode = match s.noise_suppression {
            NoiseSuppression::Off => NoiseSuppressionMode::Off,
            NoiseSuppression::Standard => NoiseSuppressionMode::Standard,
            NoiseSuppression::High => NoiseSuppressionMode::HighQuality,
            NoiseSuppression::Auto => match s.auto_high_quality {
                Some(true) => NoiseSuppressionMode::HighQuality,
                Some(false) => NoiseSuppressionMode::Standard,
                None => NoiseSuppressionMode::Auto,
            },
        };
        o.echo_cancellation = s.echo_cancellation;
        o.automatic_gain = s.agc;
        o.automatic_sensitivity = s.sensitivity_auto;
        o.sensitivity_dbfs = s.sensitivity_dbfs;
        // Manual sensitivity is an explicit level threshold, not a second
        // hidden neural confidence threshold the user cannot adjust.
        o.vad_probability = if s.sensitivity_auto { 0.5 } else { 0.0 };
    }
    o.mode = if s.push_to_talk {
        InputMode::PushToTalk
    } else {
        InputMode::VoiceActivity
    };
    o.ptt_release_delay_ms = s.push_to_talk_release_delay_ms;
    Ok(o)
}
pub fn quality(s: &VoiceSettings) -> Result<Quality, String> {
    s.validate()?;
    Ok(Quality {
        fps: FrameRate::try_from(s.share_fps).map_err(|e| e.to_string())?,
        resolution: match s.share_height {
            0 => Resolution::Source,
            480 => Resolution::P480,
            720 => Resolution::P720,
            1440 => Resolution::P1440,
            _ => Resolution::P1080,
        },
    })
}
/// Bounded, cancellable bridge. The blocking actor exists ONLY in a joined call.
#[cfg(windows)]
pub fn subscription(config: &Config) -> impl iced::futures::Stream<Item = Event> {
    use iced::futures::SinkExt;
    let config = config.clone();
    iced::stream::channel(
        128,
        move |mut output: iced::futures::channel::mpsc::Sender<Event>| async move {
            let (sender, commands) = std::sync::mpsc::sync_channel(128);
            let (events, mut received) = tokio::sync::mpsc::channel(128);
            let wake = std::sync::Arc::new(std::sync::Mutex::new(None));
            let emergency: Emergency = Default::default();
            let handle = Handle(sender, emergency.clone(), wake.clone());

            if output.send(Event::Ready(handle.clone())).await.is_err() {
                return;
            }
            let worker = tokio::task::spawn_blocking(move || {
                if let Err(e) = native::run(config, commands, &events, wake, emergency) {
                    let _ = events.blocking_send(Event::Error(e));
                }
            });
            // Subscription cancellation drops this guard and asks the actor to stop.
            struct Stop(Handle);
            impl Drop for Stop {
                fn drop(&mut self) {
                    let _ = self.0.send(Command::Stop);
                }
            }
            let _stop = Stop(handle);
            while let Some(event) = received.recv().await {
                if output.send(event).await.is_err() {
                    break;
                }
            }
            let _ = worker.await;
        },
    )
}

pub fn load_settings(path: &std::path::Path) -> VoiceSettings {
    std::fs::File::open(path)
        .ok()
        .and_then(|f| {
            use std::io::Read;
            let mut bytes = vec![];
            f.take(256 * 1024 + 1).read_to_end(&mut bytes).ok()?;
            if bytes.len() > 256 * 1024 {
                return None;
            }
            let s: VoiceSettings = serde_json::from_slice(&bytes).ok()?;
            s.validate().ok()?;
            Some(s)
        })
        .unwrap_or_default()
}
pub fn save_settings(path: &std::path::Path, s: &VoiceSettings) -> Result<(), String> {
    s.validate()?;
    let bytes = serde_json::to_vec(s).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_bind_policy_never_uses_wildcards_or_network_enumeration() {
        assert!(bind_address("127.0.0.1").unwrap().is_loopback());
        // Injected addresses only: this test never binds these interfaces.
        assert!(bind_address("100.64.1.2").is_ok());
        for bad in ["0.0.0.0", "::", "224.0.0.1", "255.255.255.255", "localhost"] {
            assert!(bind_address(bad).is_err());
        }
    }
    #[test]
    fn settings_roundtrip_and_studio_bypass() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("voice.json");
        let mut s = VoiceSettings {
            studio_mode: true,
            ..Default::default()
        };
        s.set_user(
            Id(42),
            sticktalk_client_core::voice::settings::UserAudio {
                volume: 200,
                muted: true,
            },
        )
        .unwrap();
        save_settings(&path, &s).unwrap();
        let loaded = load_settings(&path);
        assert_eq!(loaded.user(Id(42)).volume, 200);
        assert!(loaded.user(Id(42)).muted);
        let o = options(&loaded).unwrap();
        assert!(!o.echo_cancellation);
        assert!(!o.automatic_gain);
        assert!(!o.noise_suppression);
        assert_eq!(quality(&loaded).unwrap(), Quality::default());
    }
    #[test]
    fn emergency_release_does_not_depend_on_command_queue_capacity() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
        sender.try_send(Command::Gate(true, true)).unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let callback: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            flag.store(true, Ordering::Release);
        });
        let handle = Handle(
            sender,
            Arc::new(Mutex::new(Some(callback))),
            Default::default(),
        );
        assert!(handle.send(Command::Stop).is_err());
        assert!(stopped.load(Ordering::Acquire));
    }
}
