//! Minimal additive page until the parallel settings shell is merged.
use super::*;
use crate::voice_ui::Action;
use iced::widget::{column, row};
use sticktalk_client_core::voice::{settings::NoiseSuppression, VoiceSettings};
fn change(s: &VoiceSettings, f: impl FnOnce(&mut VoiceSettings)) -> Msg {
    let mut s = s.clone();
    f(&mut s);
    Msg::Call(Action::Settings(s))
}
pub(super) fn view(app: &App) -> Element<'_, Msg> {
    let s = &app.calls.call.settings;
    let toggle = |label: &str, value: bool, message: Msg| {
        action(
            format!("{label}: {}", if value { "On" } else { "Off" }),
            message,
        )
    };
    let mut modes = row![].spacing(t::S8);
    for (label, mode) in [
        ("Off", NoiseSuppression::Off),
        ("Standard", NoiseSuppression::Standard),
        ("High", NoiseSuppression::High),
    ] {
        modes = modes.push(
            button(txt(label))
                .on_press(change(s, |s| s.noise_suppression = mode))
                .padding(t::S8)
                .style(t::navigation(s.noise_suppression == mode)),
        );
    }
    column![title("Voice & Video"),
        section("Input Volume"),widget::slider(0..=200,s.input_volume,move|v|change(s,|s|s.input_volume=v)).style(t::slider),
        section("Output Volume"),widget::slider(0..=200,s.output_volume,move|v|change(s,|s|s.output_volume=v)).style(t::slider),
        line(),section("Noise Suppression"),modes,
        toggle("Echo cancellation",s.echo_cancellation,change(s,|s|s.echo_cancellation= !s.echo_cancellation)),
        toggle("Automatic gain control",s.agc,change(s,|s|s.agc= !s.agc)),
        toggle("Automatic sensitivity",s.sensitivity_auto,change(s,|s|s.sensitivity_auto= !s.sensitivity_auto)),
        hint(format!("Input sensitivity: {:.0} dBFS",s.sensitivity_dbfs)),
        widget::slider(-96.0..=0.0,s.sensitivity_dbfs,move|v|change(s,|s|s.sensitivity_dbfs=v)).style(t::slider),
        line(),toggle("Push to talk",s.push_to_talk,change(s,|s|s.push_to_talk= !s.push_to_talk)),
        hint(format!("Release delay: {} ms",s.push_to_talk_release_delay_ms)),
        widget::slider(0..=1000,s.push_to_talk_release_delay_ms,move|v|change(s,|s|s.push_to_talk_release_delay_ms=v)).style(t::slider),
        toggle("Studio / Music",s.studio_mode,change(s,|s|s.studio_mode= !s.studio_mode)),
        hint("Studio mode changes take effect when you rejoin the call. Keep a separate microphone selected to avoid Bluetooth hands-free audio."),
    ].spacing(t::S16).into()
}
