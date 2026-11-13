use super::*;
use crate::voice_ui::Action;
use iced::widget::{column, row};
pub(super) fn view(app: &App, session: VoiceSession) -> Element<'_, Msg> {
    container(
        column![
            row![
                txt(format!(
                    "{}'s stream",
                    app.state.model.name(session.user_id)
                ))
                .width(Fill),
                icon_button("x", "Close stream", Msg::Call(Action::Focus(None)))
            ]
            .align_y(Center),
            hint("Video display is unavailable on this build."),
            action(
                "Request control",
                Msg::Call(Action::RequestControl(session))
            ),
            super::call::user_audio(app, session.user_id),
        ]
        .spacing(t::S8),
    )
    .padding(t::S16)
    .width(Fill)
    .style(t::elevated)
    .into()
}
pub(super) fn stats(app: &App) -> Element<'_, Msg> {
    let label = app
        .calls
        .call
        .stats
        .as_ref()
        .map_or("Waiting for statistics…".into(), |s| {
            format!(
                "Transport {:.1} fps · {:.2} Mbps · loss {} · RTT {} · {:?}",
                s.rx_fps_milli as f64 / 1000.,
                s.rx_bps as f64 / 1_000_000.,
                s.loss_per_million
                    .map_or("unknown".into(), |p| format!("{:.2}%", p as f64 / 10_000.)),
                s.rtt_ms.map_or("unknown".into(), |r| format!("{r} ms")),
                s.path
            )
        });
    container(hint(label))
        .padding(t::S12)
        .style(t::elevated)
        .into()
}
