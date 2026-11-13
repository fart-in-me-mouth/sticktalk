//! Settings-only native lifetimes. No stream or periodic meter outside Let's Check.
use iced::futures::SinkExt;
use sticktalk_client_core::voice::VoiceSettings;
use sticktalk_media as media;
#[derive(Clone, Debug)]
pub enum Event {
    Ready(Handle),
    Devices(Vec<media::AudioDevice>, Vec<media::CameraDevice>),
    Level(media::MicLevel),
    Probe(bool),
    Error(String),
}
#[derive(Clone, Debug)]
pub struct Handle(tokio::sync::watch::Sender<VoiceSettings>);
impl Handle {
    pub fn configure(&self, settings: VoiceSettings) {
        self.0.send_replace(settings);
    }
}
#[derive(Clone, Debug)]
pub struct Config {
    pub settings: String,
    pub testing: bool,
}
// Changing a fader/threshold must not reopen WASAPI streams or restart the
// meter. Only entering/leaving the explicit test changes the subscription.
impl PartialEq for Config {
    fn eq(&self, other: &Self) -> bool {
        self.testing == other.testing
    }
}
impl Eq for Config {}
impl std::hash::Hash for Config {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.testing, state);
    }
}
/// IMM callbacks wake an async queue. No timer, frame subscription or device
/// capture is installed just by opening Voice & Video settings.
pub fn subscription(config: &Config) -> impl iced::futures::Stream<Item = Event> {
    let config = config.clone();
    iced::stream::channel(
        8,
        move |mut output: iced::futures::channel::mpsc::Sender<Event>| async move {
            let (notify, mut changes) = tokio::sync::mpsc::channel(1);
            let watcher = tokio::task::spawn_blocking(move || {
                media::AudioDeviceWatcher::start_with_callback(Some(std::sync::Arc::new(
                    move || {
                        let _ = notify.try_send(());
                    },
                )))
            })
            .await;
            let _watcher = match watcher {
                Ok(Ok(w)) => w,
                e => {
                    let _ = output
                        .send(Event::Error(format!(
                            "Device notifications unavailable: {}",
                            match e {
                                Ok(Err(e)) => e.to_string(),
                                Err(e) => e.to_string(),
                                _ => String::new(),
                            }
                        )))
                        .await;
                    return;
                }
            };
            let Ok(settings) = serde_json::from_str::<VoiceSettings>(&config.settings) else {
                return;
            };
            let (controls, mut settings_rx) = tokio::sync::watch::channel(settings.clone());
            if output.send(Event::Ready(Handle(controls))).await.is_err() {
                return;
            }
            if settings.auto_high_quality.is_none() {
                if let Ok(probe) =
                    tokio::task::spawn_blocking(media::voice::benchmark_noise_quality).await
                {
                    let _ = output
                        .send(Event::Probe(
                            probe.recommended == media::voice::NoiseSuppressionMode::HighQuality,
                        ))
                        .await;
                }
            }
            let (levels, mut meter) = tokio::sync::mpsc::channel(1);
            let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            struct Stop(std::sync::Arc<std::sync::atomic::AtomicBool>);
            impl Drop for Stop {
                fn drop(&mut self) {
                    self.0.store(false, std::sync::atomic::Ordering::Release);
                }
            }
            let _stop = Stop(alive.clone());
            if config.testing {
                tokio::task::spawn_blocking(move || {
                    let mut run = || -> Result<(), String> {
                        let mut settings = settings_rx.borrow_and_update().clone();
                        let make_test =
                            |settings: &VoiceSettings| -> Result<media::MicTestSession, String> {
                                let test = media::MicTestSession::start(
                                    media::VoiceDevices {
                                        input: settings.input_device_id.clone(),
                                        output: settings.output_device_id.clone(),
                                    },
                                    sticktalk::voice::options(settings)?,
                                )
                                .map_err(|e| e.to_string())?;
                                test.set_gains(settings.input_volume, settings.output_volume)
                                    .map_err(|e| e.to_string())?;
                                test.set_loopback_gain(0.5).map_err(|e| e.to_string())?;
                                Ok(test)
                            };
                        let mut test = make_test(&settings)?;
                        while alive.load(std::sync::atomic::Ordering::Acquire) {
                            if settings_rx.has_changed().unwrap_or(false) {
                                let next = settings_rx.borrow_and_update().clone();
                                if next.studio_mode != settings.studio_mode {
                                    test.stop().map_err(|e| e.to_string())?;
                                    test = make_test(&next)?;
                                } else {
                                    test.configure(sticktalk::voice::options(&next)?)
                                        .map_err(|e| e.to_string())?;
                                    if next.input_device_id != settings.input_device_id
                                        || next.output_device_id != settings.output_device_id
                                    {
                                        test.set_devices(media::VoiceDevices {
                                            input: next.input_device_id.clone(),
                                            output: next.output_device_id.clone(),
                                        })
                                        .map_err(|e| e.to_string())?;
                                    }
                                    test.set_gains(next.input_volume, next.output_volume)
                                        .map_err(|e| e.to_string())?;
                                }
                                settings = next;
                            }
                            match test.recv_level_timeout(std::time::Duration::from_millis(100)) {
                                Ok(level) => {
                                    if levels.blocking_send(Ok(level)).is_err() {
                                        break;
                                    }
                                }
                                Err(media::Error::Timeout(_)) => {}
                                Err(e) => return Err(e.to_string()),
                            }
                        }
                        Ok(())
                    };
                    if let Err(e) = run() {
                        let _ = levels.blocking_send(Err(e));
                    }
                });
            }
            loop {
                match tokio::task::spawn_blocking(|| -> Result<_, String> {
                    Ok((
                        media::audio_devices().map_err(|e| e.to_string())?,
                        media::camera_devices().map_err(|e| e.to_string())?,
                    ))
                })
                .await
                {
                    Ok(Ok((devices, cameras))) => {
                        if output.send(Event::Devices(devices, cameras)).await.is_err() {
                            break;
                        }
                    }
                    e => {
                        let _ = output
                            .send(Event::Error(format!("Device list unavailable: {e:?}")))
                            .await;
                    }
                }
                loop {
                    tokio::select! {
                        change = changes.recv() => { if change.is_none() { return; } break; },
                        level = meter.recv(), if config.testing => match level {
                            Some(Ok(level)) => if output.send(Event::Level(level)).await.is_err() { return; },
                            Some(Err(e)) => { let _ = output.send(Event::Error(e)).await; return; },
                            None => return,
                        },
                    }
                }
            }
        },
    )
}
/// A fake endpoint source runs through exactly the same bounded enumeration and
/// selection code as Windows. It never constructs an AudioClient.
pub struct FakeDevices(pub Vec<media::AudioDevice>);
impl media::AudioDeviceBackend for FakeDevices {
    fn enumerate(&mut self) -> media::Result<Vec<media::AudioDevice>> {
        Ok(self.0.clone())
    }
}
pub fn fixture_devices() -> Vec<media::AudioDevice> {
    let d = |id: &str, name: &str, direction, default| media::AudioDevice {
        id: id.into(),
        name: name.into(),
        direction,
        default_console: default,
        default_communications: id == "headset-mic" || id == "headset-hfp",
    };
    media::enumerate_audio_devices(&mut FakeDevices(vec![
        d(
            "usb-mic",
            "Microphone (USB Audio)",
            media::AudioDirection::Input,
            true,
        ),
        d(
            "headset-mic",
            "Headset microphone (Bluetooth Hands-Free)",
            media::AudioDirection::Input,
            false,
        ),
        d(
            "headphones",
            "Headphones (Bluetooth Stereo)",
            media::AudioDirection::Output,
            true,
        ),
        d(
            "speakers",
            "Speakers (Realtek Audio)",
            media::AudioDirection::Output,
            false,
        ),
        d(
            "headset-hfp",
            "Headset (Bluetooth Hands-Free)",
            media::AudioDirection::Output,
            false,
        ),
    ]))
    .unwrap()
}

/// Win32 key identity, not locale-dependent display text. Escape cancels.
pub fn virtual_key(key: &iced::keyboard::Key) -> Option<u32> {
    use iced::keyboard::{key::Named, Key};
    match key {
        Key::Character(c) => {
            let mut units = c.encode_utf16();
            let unit = units.next()?;
            if units.next().is_some() {
                return None;
            }
            let vk = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW(unit) };
            (vk != -1).then_some(u32::from(vk as u16 & 0xff))
        }
        Key::Named(n) => Some(match n {
            Named::Escape => 0x1b,
            Named::Space => 0x20,
            Named::Control => 0xa2,
            Named::Shift => 0xa0,
            Named::Alt => 0xa4,
            Named::Tab => 9,
            Named::Enter => 0x0d,
            Named::CapsLock => 0x14,
            Named::Backspace => 8,
            Named::ArrowLeft => 0x25,
            Named::ArrowUp => 0x26,
            Named::ArrowRight => 0x27,
            Named::ArrowDown => 0x28,
            Named::Home => 0x24,
            Named::End => 0x23,
            Named::Insert => 0x2d,
            Named::Delete => 0x2e,
            Named::PageUp => 0x21,
            Named::PageDown => 0x22,
            Named::F1 => 0x70,
            Named::F2 => 0x71,
            Named::F3 => 0x72,
            Named::F4 => 0x73,
            Named::F5 => 0x74,
            Named::F6 => 0x75,
            Named::F7 => 0x76,
            Named::F8 => 0x77,
            Named::F9 => 0x78,
            Named::F10 => 0x79,
            Named::F11 => 0x7a,
            Named::F12 => 0x7b,
            Named::F13 => 0x7c,
            Named::F14 => 0x7d,
            Named::F15 => 0x7e,
            Named::F16 => 0x7f,
            Named::F17 => 0x80,
            Named::F18 => 0x81,
            Named::F19 => 0x82,
            Named::F20 => 0x83,
            Named::F21 => 0x84,
            Named::F22 => 0x85,
            Named::F23 => 0x86,
            Named::F24 => 0x87,
            _ => return None,
        }),
        _ => None,
    }
}
pub fn record_virtual_key(
    key: &iced::keyboard::Key,
    physical: &iced::keyboard::key::Physical,
) -> Option<u32> {
    use iced::keyboard::key::{Code, Physical};
    Some(match physical {
        Physical::Code(code) => match code {
            Code::ControlRight => 0xa3,
            Code::ShiftRight => 0xa1,
            Code::AltRight => 0xa5,
            Code::Numpad0 => 0x60,
            Code::Numpad1 => 0x61,
            Code::Numpad2 => 0x62,
            Code::Numpad3 => 0x63,
            Code::Numpad4 => 0x64,
            Code::Numpad5 => 0x65,
            Code::Numpad6 => 0x66,
            Code::Numpad7 => 0x67,
            Code::Numpad8 => 0x68,
            Code::Numpad9 => 0x69,
            Code::NumpadMultiply => 0x6a,
            Code::NumpadAdd => 0x6b,
            Code::NumpadSubtract => 0x6d,
            Code::NumpadDecimal => 0x6e,
            Code::NumpadDivide => 0x6f,
            _ => return virtual_key(key),
        },
        _ => return virtual_key(key),
    })
}
pub fn key_name(key: Option<u32>) -> String {
    match key {
        None => "Record a keybind".into(),
        Some(0x20) => "Space".into(),
        Some(0xa2) => "Left Ctrl".into(),
        Some(0xa0) => "Left Shift".into(),
        Some(0xa4) => "Left Alt".into(),
        Some(0xa3) => "Right Ctrl".into(),
        Some(0xa1) => "Right Shift".into(),
        Some(0xa5) => "Right Alt".into(),
        Some(0x14) => "Caps Lock".into(),
        Some(0x0d) => "Enter".into(),
        Some(k @ 0x70..=0x87) => format!("F{}", k - 0x6f),
        Some(k @ 0x60..=0x69) => format!("Numpad {}", k - 0x60),
        Some(k @ 0x30..=0x5a) => char::from_u32(k).unwrap().to_string(),
        Some(k) => format!("Key {k}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fake_hotplug_uses_native_enumeration_and_selection_seam() {
        let mut fake = FakeDevices(fixture_devices());
        let first = media::enumerate_audio_devices(&mut fake).unwrap();
        let selected = media::choose_audio_device(
            &first,
            media::AudioDirection::Input,
            Some("usb-mic"),
            false,
        )
        .unwrap();
        assert_eq!(selected.name, "Microphone (USB Audio)");
        fake.0.retain(|d| d.id != "usb-mic");
        fake.0
            .iter_mut()
            .filter(|d| d.direction == media::AudioDirection::Input)
            .for_each(|d| d.default_console = true);
        let changed = media::enumerate_audio_devices(&mut fake).unwrap();
        assert_eq!(
            media::choose_audio_device(
                &changed,
                media::AudioDirection::Input,
                Some("usb-mic"),
                false
            )
            .unwrap()
            .id,
            "headset-mic"
        );
        assert_eq!(
            media::choose_audio_device(&changed, media::AudioDirection::Output, None, false)
                .unwrap()
                .id,
            "headphones"
        );
        fake.0.clear();
        assert!(media::choose_audio_device(
            &media::enumerate_audio_devices(&mut fake).unwrap(),
            media::AudioDirection::Output,
            None,
            false
        )
        .is_none());
    }
    #[test]
    fn recorder_and_settings_close_never_start_devices() {
        use crate::{
            app::{App, Msg},
            voice_ui::Action,
        };
        let mut app = App::headless("audio-ptt", iced::Size::new(1280., 800.));
        let _ = app.update(Msg::Call(Action::RecordedKey(0x56)));
        assert_eq!(app.calls.call.settings.push_to_talk_key, Some(0x56));
        assert!(!app.calls.recording_key);
        let _ = app.update(Msg::Call(Action::MicTest));
        assert!(app.calls.mic_test);
        let _ = app.update(Msg::Close);
        assert!(!app.calls.mic_test);
        assert!(app.calls.mic_level.is_none());
        assert!(app.calls.handle.is_none());
        let _ = app.update(Msg::Call(Action::Audio(Event::Error(
            "fake permission denial".into(),
        ))));
        assert!(app.calls.audio_error.is_some());
        assert!(!app.calls.mic_test);
    }
    #[test]
    fn old_preferences_default_new_fields_and_validate_bounds() {
        let old: VoiceSettings = serde_json::from_str("{\"noise_suppression\":\"High\"}").unwrap();
        assert_eq!(old.attenuation, 0);
        assert!(old.auto_high_quality.is_none());
        old.validate().unwrap();
        let mut s = old;
        s.attenuation = 101;
        assert!(s.validate().is_err());
        assert_eq!(
            virtual_key(&iced::keyboard::Key::Character("v".into())),
            Some(0x56)
        );
        assert_eq!(key_name(Some(0x56)), "V");
    }
    #[test]
    fn live_controls_are_coalesced_without_restarting_the_test() {
        let a = Config {
            settings: "initial".into(),
            testing: true,
        };
        let b = Config {
            settings: "new threshold".into(),
            testing: true,
        };
        assert_eq!(a, b);
        let (sender, mut receiver) = tokio::sync::watch::channel(VoiceSettings::default());
        let handle = Handle(sender);
        for value in 0..=200 {
            handle.configure(VoiceSettings {
                input_volume: value,
                ..Default::default()
            });
        }
        assert!(receiver.has_changed().unwrap());
        assert_eq!(receiver.borrow_and_update().input_volume, 200);
        assert!(!receiver.has_changed().unwrap());
    }
    #[test]
    fn fake_mic_meter_and_processed_loopback_need_no_devices() {
        use media::voice::*;
        let mut backend = FakeDevices(fixture_devices());
        assert!(!media::enumerate_audio_devices(&mut backend)
            .unwrap()
            .is_empty());
        let mut p = VoiceProcessor::new(VoiceOptions {
            noise_mode: NoiseSuppressionMode::Off,
            mode: InputMode::PushToTalk,
            echo_cancellation: false,
            automatic_gain: false,
            ..Default::default()
        })
        .unwrap();
        p.set_push_to_talk(true);
        let pcm = std::array::from_fn(|i| ((i / 2) as f32 * 0.05).sin() * 0.25);
        for _ in 0..6 {
            p.capture_stereo(&pcm).unwrap();
        }
        let level = media::measure_mic_level(&pcm, p.sensitivity_dbfs());
        assert!(level.rms_dbfs > -20. && !level.clipping);
        assert!(p.last_processed_stereo().iter().any(|s| s.abs() > 0.01));
        let clipping = media::measure_mic_level(&[1.; TICK], -50.);
        assert!(clipping.clipping);
        assert_eq!(media::measure_mic_level(&[0.; TICK], -50.).rms_dbfs, -120.);
    }
}
