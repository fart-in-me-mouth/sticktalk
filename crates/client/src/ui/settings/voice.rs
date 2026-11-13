//! Voice settings use the call's persisted model and explicit mic-test lifetime.
use super::*;
use crate::ui::{action, primary};
use crate::voice_ui::Action;
use iced::widget::{column, row};
use sticktalk_client_core::voice::{settings::NoiseSuppression, VoiceSettings};
use sticktalk_media::AudioDirection;
fn change(s: &VoiceSettings, f: impl FnOnce(&mut VoiceSettings)) -> Msg {
    let mut s = s.clone();
    f(&mut s);
    Msg::Call(Action::Settings(s))
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Choice {
    id: Option<String>,
    name: String,
}
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}
fn device(app: &App, direction: AudioDirection) -> Element<'_, Msg> {
    let s = &app.calls.call.settings;
    let mut choices = vec![Choice {
        id: None,
        name: "Default (follows Windows)".into(),
    }];
    choices.extend(
        app.calls
            .audio_devices
            .iter()
            .filter(|d| d.direction == direction)
            .map(|d| Choice {
                id: Some(d.id.clone()),
                name: d.name.clone(),
            }),
    );
    let id = if direction == AudioDirection::Input {
        &s.input_device_id
    } else {
        &s.output_device_id
    };
    let selected = choices
        .iter()
        .find(|c| &c.id == id)
        .cloned()
        .unwrap_or(Choice {
            id: id.clone(),
            name: "Unavailable — using Default".into(),
        });
    widget::pick_list(choices, Some(selected), move |choice| {
        change(s, |s| {
            if direction == AudioDirection::Input {
                s.input_device_id = choice.id
            } else {
                s.output_device_id = choice.id
            }
        })
    })
    .width(Fill)
    .padding(t::S12)
    .font(t::regular())
    .text_size(t::USER_NAME)
    .style(st::pick_list)
    .menu_style(st::menu)
    .into()
}
fn toggle<'a>(label: &str, help: &str, on: bool, message: Msg) -> Element<'a, Msg> {
    super::widgets::toggle_message(label, help, on, message, true)
}
fn volume<'a>(s: &'a VoiceSettings, input: bool) -> Element<'a, Msg> {
    let value = if input {
        s.input_volume
    } else {
        s.output_volume
    };
    column![
        row![
            section(if input {
                "INPUT VOLUME"
            } else {
                "OUTPUT VOLUME"
            })
            .width(Fill),
            hint(format!("{value}%"))
        ],
        widget::slider(0..=200, value, move |v| change(s, |s| if input {
            s.input_volume = v
        } else {
            s.output_volume = v
        }))
        .style(st::slider)
    ]
    .spacing(t::S8)
    .into()
}
fn meter(app: &App) -> Element<'_, Msg> {
    let s = &app.calls.call.settings;
    let level = app.calls.mic_level.map_or(-96., |l| l.rms_dbfs);
    let mut bars = row![].spacing(t::S4);
    for i in 0..40 {
        let active = level > -96. + i as f32 * 96. / 40.;
        bars = bars.push(
            container(space())
                .width(Fill)
                .height(t::S16)
                .style(move |_| t::surface(if active { t::ACCENT } else { t::BG_SELECTED })),
        );
    }
    let meter = container(bars).center_y(t::S32);
    if !s.sensitivity_auto {
        stack![
            meter,
            widget::slider(-96.0..=0.0, s.sensitivity_dbfs, move |v| change(s, |s| s
                .sensitivity_dbfs =
                v))
            .height(t::S32)
            .style(t::threshold_slider)
        ]
        .into()
    } else {
        meter.into()
    }
}
pub(super) fn view(app: &App) -> Element<'_, Msg> {
    let s = &app.calls.call.settings;
    let mut modes = row![].spacing(t::S4);
    for (label, mode) in [
        ("Auto", NoiseSuppression::Auto),
        ("Off", NoiseSuppression::Off),
        ("Standard", NoiseSuppression::Standard),
        ("High quality", NoiseSuppression::High),
    ] {
        modes = modes.push(
            button(txt(label).size(t::USER_NAME))
                .on_press(change(s, |s| s.noise_suppression = mode))
                .padding([t::S8, t::S12])
                .style(t::navigation(s.noise_suppression == mode)),
        );
    }
    let mut input = column![
        title("Voice & Video"),
        row![
            column![section("INPUT DEVICE"), device(app, AudioDirection::Input)]
                .spacing(t::S8)
                .width(Fill),
            column![
                section("OUTPUT DEVICE"),
                device(app, AudioDirection::Output)
            ]
            .spacing(t::S8)
            .width(Fill)
        ]
        .spacing(t::S16),
        row![volume(s, true), volume(s, false)].spacing(t::S16),
        row![
            column![
                txt("Mic Test").font(t::semibold()),
                hint(if app.calls.mic_test {
                    "Speak to hear yourself. Keep your headphones on."
                } else {
                    "Check your microphone and hear how you sound."
                })
            ]
            .spacing(t::S4)
            .width(Fill),
            primary(
                if app.calls.mic_test {
                    "Stop Checking"
                } else {
                    "Let's Check"
                },
                Msg::Call(Action::MicTest)
            )
        ]
        .spacing(t::S16)
        .align_y(Center),
    ]
    .spacing(t::S16);
    if let Some(error) = &app.calls.audio_error {
        input = input.push(hint(error).color(t::DANGER));
    }
    if app.calls.mic_test && (s.sensitivity_auto || s.push_to_talk) {
        input = input.push(meter(app));
    }
    input=input.push(hint("Bluetooth: A2DP gives high-quality stereo playback. Using a headset microphone switches to HFP, reducing playback quality. Select a separate microphone to keep stereo audio."));
    input = input.push(line()).push(title("Input Mode"));
    let mut mode = row![].spacing(t::S8);
    for (label, ptt) in [("Voice Activity", false), ("Push to Talk", true)] {
        mode = mode.push(
            button(
                row![
                    icon(
                        if s.push_to_talk == ptt {
                            "circle-dot"
                        } else {
                            "circle"
                        },
                        t::INLINE_ICON,
                        if s.push_to_talk == ptt {
                            t::ACCENT
                        } else {
                            t::TEXT_MUTED
                        }
                    ),
                    txt(label)
                ]
                .spacing(t::S8),
            )
            .on_press(change(s, |s| s.push_to_talk = ptt))
            .padding(t::S12)
            .style(t::navigation(s.push_to_talk == ptt)),
        );
    }
    input = input.push(mode);
    if s.push_to_talk {
        input = input
            .push(
                row![
                    column![
                        section("KEYBIND"),
                        hint(if app.calls.recording_key {
                            "Press a key. Esc cancels."
                        } else {
                            "Works while a fullscreen game is focused."
                        })
                    ]
                    .spacing(t::S4)
                    .width(Fill),
                    action(
                        if app.calls.recording_key {
                            "Recording…".into()
                        } else {
                            crate::audio_settings::key_name(s.push_to_talk_key)
                        },
                        Msg::Call(Action::RecordKey)
                    )
                    .style(if app.calls.recording_key {
                        t::primary
                    } else {
                        st::secondary
                    }),
                    action("Clear", change(s, |s| s.push_to_talk_key = None))
                        .on_press_maybe(
                            (s.push_to_talk_key.is_some() && !app.calls.recording_key)
                                .then(|| change(s, |s| s.push_to_talk_key = None))
                        )
                        .style(t::link)
                ]
                .spacing(t::S16)
                .align_y(Center),
            )
            .push(row![
                section("RELEASE DELAY").width(Fill),
                hint(format!("{} ms", s.push_to_talk_release_delay_ms))
            ])
            .push(
                widget::slider(0..=1000, s.push_to_talk_release_delay_ms, move |v| {
                    change(s, |s| s.push_to_talk_release_delay_ms = v)
                })
                .style(st::slider),
            );
    } else {
        input = input.push(toggle(
            "Automatically determine input sensitivity",
            "Turn off to set the microphone threshold yourself.",
            s.sensitivity_auto,
            change(s, |s| s.sensitivity_auto = !s.sensitivity_auto),
        ));
        if !s.sensitivity_auto {
            input = input
                .push(row![
                    section("INPUT SENSITIVITY").width(Fill),
                    hint(format!("{:.0} dB", s.sensitivity_dbfs))
                ])
                .push(meter(app));
            if !app.calls.mic_test {
                input = input.push(hint("Start Let's Check to see live input levels."));
            }
        }
    }
    input = input
        .push(line())
        .push(title("Voice Processing"))
        .push(section("NOISE SUPPRESSION"))
        .push(modes)
        .push(hint(match s.noise_suppression {
            NoiseSuppression::Auto => match s.auto_high_quality {
                Some(true) => "Auto selected High quality for this computer.",
                Some(false) => "Auto selected Standard to keep calls responsive.",
                None => "Auto checks this computer once to choose the best quality.",
            },
            NoiseSuppression::High => "Reduce background noise with higher CPU use.",
            NoiseSuppression::Standard => "Lightweight suppression for everyday background noise.",
            NoiseSuppression::Off => "No noise suppression.",
        }))
        .push(toggle(
            "Echo Cancellation",
            "Reduce echo from your speakers.",
            s.echo_cancellation,
            change(s, |s| s.echo_cancellation = !s.echo_cancellation),
        ))
        .push(toggle(
            "Automatic Gain Control",
            "Keep your voice at a consistent level.",
            s.agc,
            change(s, |s| s.agc = !s.agc),
        ))
        .push(toggle(
            "Studio / Music",
            "Stereo audio with voice processing off. Rejoin to apply.",
            s.studio_mode,
            change(s, |s| s.studio_mode = !s.studio_mode),
        ))
        .push(line())
        .push(title("Attenuation"))
        .push(hint(
            "Lower other apps while someone speaks. Off by default.",
        ))
        .push(
            row![
                widget::slider(0..=100, s.attenuation, move |v| change(s, |s| s
                    .attenuation =
                    v))
                .style(st::slider),
                hint(if s.attenuation == 0 {
                    "Off".into()
                } else {
                    format!("{}%", s.attenuation)
                })
            ]
            .spacing(t::S16),
        )
        .push(line())
        .push(title("Camera"));
    let mut cameras = vec![Choice {
        id: None,
        name: "Default camera".into(),
    }];
    cameras.extend(app.calls.audio_cameras.iter().map(|d| Choice {
        id: Some(d.id.clone()),
        name: d.name.clone(),
    }));
    let selected = cameras
        .iter()
        .find(|c| c.id == s.camera_device_id)
        .cloned()
        .unwrap_or(cameras[0].clone());
    input = input
        .push(
            widget::pick_list(cameras, Some(selected), move |c| {
                change(s, |s| s.camera_device_id = c.id)
            })
            .padding(t::S12)
            .width(Fill)
            .font(t::regular())
            .text_size(t::USER_NAME)
            .style(st::pick_list)
            .menu_style(st::menu),
        )
        .push(hint("Choose the camera to use in calls."));
    input.into()
}
