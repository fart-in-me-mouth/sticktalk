use super::*;
use crate::voice_ui::Action;
use iced::widget::{column, row};

pub(super) fn video(
    app: &App,
    session: VoiceSession,
    kind: TrackKind,
    height: f32,
) -> Element<'_, Msg> {
    #[cfg(feature = "wgpu")]
    if let Some(video) = app.calls.videos.get(&(session, kind as u8)) {
        let frame = video.frame.clone();
        let controlling = app
            .calls
            .call
            .control
            .grant
            .filter(|g| {
                kind == TrackKind::Screen
                    && Some(g.controller) == app.calls.call.own
                    && g.sharer == session
            })
            .map(|g| g.token);
        return container(widget::responsive(move |size| {
            let scale =
                (size.width / frame.size.width as f32).min(size.height / frame.size.height as f32);
            container(
                widget::shader(super::stream_input::Playback {
                    frame: frame.clone(),
                    controlling,
                })
                .width(frame.size.width as f32 * scale)
                .height(frame.size.height as f32 * scale),
            )
            .center_x(Fill)
            .center_y(Fill)
            .into()
        }))
        .width(Fill)
        .height(height)
        .style(t::rail)
        .into();
    }
    if app.calls.mock_playback {
        if kind == TrackKind::Camera {
            return container(
                column![
                    avatar(app, session.user_id, t::CALL_AVATAR, false),
                    hint(app.state.model.name(session.user_id))
                ]
                .spacing(t::S12)
                .align_x(Center),
            )
            .center_x(Fill)
            .center_y(height)
            .style(t::rail)
            .into();
        }
        let mut board = row![].spacing(t::S12);
        for (label, items) in [
            ("TO DO", vec!["Review call layouts", "Keyboard shortcuts"]),
            (
                "IN PROGRESS",
                vec!["Stream quality controls", "Camera chooser"],
            ),
            ("DONE", vec!["Call participants", "Screen sharing"]),
        ] {
            let mut col = column![section(label)].spacing(t::S8);
            for item in items {
                col = col.push(
                    container(column![txt(item), hint("Design review · Today")].spacing(t::S8))
                        .padding(t::S12)
                        .width(Fill)
                        .style(t::elevated),
                );
            }
            board = board.push(col.width(Fill));
        }
        return container(
            column![
                row![
                    icon("layout-grid", t::ICON, t::ACCENT),
                    title("Sprint planning")
                ]
                .spacing(t::S12),
                hint("Design / November sprint"),
                board
            ]
            .spacing(t::S16),
        )
        .padding(t::S24)
        .height(height)
        .width(Fill)
        .style(t::rail)
        .into();
    }
    if let Some(handle) = app.calls.mock_video.as_ref() {
        return image(handle.clone())
            .width(Fill)
            .height(height)
            .content_fit(iced::ContentFit::Contain)
            .into();
    }
    container(
        column![
            icon(
                if kind == TrackKind::Camera {
                    "video"
                } else {
                    "monitor-up"
                },
                t::CALL_AVATAR,
                t::TEXT_MUTED
            ),
            hint("Waiting for video…")
        ]
        .spacing(t::S16)
        .align_x(Center),
    )
    .height(height)
    .center_x(Fill)
    .center_y(height)
    .style(t::rail)
    .into()
}
pub(super) fn view(app: &App, session: VoiceSession) -> Element<'_, Msg> {
    widget::responsive(move |size| view_at(app, session, size.height)).into()
}
pub(super) fn view_at(app: &App, session: VoiceSession, available: f32) -> Element<'_, Msg> {
    let height = (available - t::BUTTON - t::CALL_STRIP_HEIGHT - t::S32 - 2. * t::S12)
        .max(t::SOURCE_PREVIEW_H);
    let mut strip = row![].spacing(t::S8);
    for p in app.calls.call.participants.values().take(8) {
        strip = strip.push(
            container(super::call::participant(app, p, true))
                .padding(t::S8)
                .width(Fill)
                .style(t::elevated),
        );
    }
    let mut screen = vec![mouse_area(video(app, session, TrackKind::Screen, height))
        .on_double_click(Msg::Call(Action::Fullscreen))
        .into()];
    if app.calls.stats {
        screen.push(
            container(stats(app))
                .padding(t::S12)
                .align_right(Fill)
                .into(),
        );
    }
    let control: Element<'_, Msg> = if Some(session) == app.calls.call.own {
        row![
            action("Stream quality", Msg::Call(Action::Picker)),
            action("Stop streaming", Msg::Call(Action::StopShare)).style(t::danger)
        ]
        .spacing(t::S8)
        .into()
    } else if let Some(grant) = app.calls.call.control.grant.filter(|g| g.sharer == session) {
        if Some(grant.controller) == app.calls.call.own {
            tooltip(
                action("Stop control", Msg::Call(Action::Revoke)),
                txt("Click the video to control. Hold Esc for 1 second to release."),
                tooltip::Position::Bottom,
            )
            .into()
        } else {
            hint(format!(
                "{} is controlling",
                app.state.model.name(grant.controller.user_id)
            ))
            .into()
        }
    } else if app
        .calls
        .control_request
        .is_some_and(|(sharer, _)| sharer == session)
    {
        hint("Request sent · awaiting consent")
            .color(t::WARNING)
            .into()
    } else {
        action(
            "Request control",
            Msg::Call(Action::RequestControl(session)),
        )
        .into()
    };
    container(
        column![
            row![
                icon("monitor-up", t::ICON, t::TEXT_SECONDARY),
                txt(if Some(session) == app.calls.call.own {
                    "You're streaming".into()
                } else {
                    format!("{}'s screen", app.state.model.name(session.user_id))
                })
                .font(t::semibold())
                .width(Fill),
                control,
                icon_button("signal", "Stream statistics", Msg::Call(Action::Stats)),
                icon_button(
                    "maximize",
                    "Fullscreen (double-click video)",
                    Msg::Call(Action::Fullscreen)
                ),
                icon_button("external-link", "Pop out", Msg::Call(Action::Popout)),
                icon_button("x", "Close stream", Msg::Call(Action::Focus(None)))
            ]
            .spacing(t::S8)
            .align_y(Center),
            stack(screen),
            strip,
        ]
        .spacing(t::S12),
    )
    .padding(t::S16)
    .width(Fill)
    .height(Fill)
    .style(t::chat)
    .into()
}
pub(super) fn stats(app: &App) -> Element<'_, Msg> {
    let mut lines = column![section("STREAM STATISTICS")].spacing(t::S4);
    if let Some(session) = app.calls.focus {
        if let Some(video) = app.calls.videos.get(&(session, TrackKind::Screen as u8)) {
            lines = lines.push(hint(format!(
                "{:.1} fps · {} × {} · {:.2} ms decode · {} dropped",
                video.fps,
                video.frame.size.width,
                video.frame.size.height,
                video.decode_ms,
                video.dropped
            )));
        } else if app.calls.mock_playback {
            lines = lines.push(hint("59.9 fps · 1920 × 1080 · 1.42 ms decode · 0 dropped"));
        }
    }
    let label =
        app.calls
            .call
            .stats
            .as_ref()
            .map_or("Waiting for transport statistics…".into(), |s| {
                format!(
                    "{:.2} Mbps · {} RTT",
                    app.calls
                        .focus
                        .and_then(|session| app
                            .calls
                            .videos
                            .get(&(session, TrackKind::Screen as u8)))
                        .map_or(s.rx_bps, |v| v.bps) as f64
                        / 1_000_000.,
                    s.rtt_ms.map_or("unknown".into(), |r| format!("{r} ms"))
                )
            });
    container(lines.push(hint(label)))
        .padding(t::S12)
        .style(t::elevated)
        .into()
}
