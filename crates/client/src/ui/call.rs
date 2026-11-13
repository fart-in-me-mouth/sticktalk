use super::*;
use crate::voice_ui::Action;
use iced::widget::{column, row};
use sticktalk_client_core::voice::Phase;
fn msg(action: Action) -> Msg {
    Msg::Call(action)
}
pub(super) fn participant<'a>(
    app: &'a App,
    p: &VoiceParticipant,
    compact: bool,
) -> Element<'a, Msg> {
    let tile_height = if app.calls.call.participants.len() > 6 {
        ((app.height - t::HEADER - t::S32 - 2. * t::S24 - t::GUILD_ICON - 2. * t::S12) / 3.)
            .clamp(t::SOURCE_PREVIEW_H, t::CALL_TILE_HEIGHT)
    } else {
        t::CALL_TILE_HEIGHT
    };
    let has_stream = p.tracks.iter().any(|t| t.kind == TrackKind::Screen);
    let watch = || {
        action(
            if Some(p.session) == app.calls.call.own {
                "Stream quality"
            } else {
                "Watch stream"
            },
            msg(if Some(p.session) == app.calls.call.own {
                Action::Picker
            } else {
                Action::Focus(Some(p.session))
            }),
        )
    };
    let speaking = app.calls.call.speaking.contains(&p.session);
    let face = container(avatar(
        app,
        p.session.user_id,
        if compact { t::S24 } else { t::CALL_AVATAR },
        false,
    ))
    .padding(2)
    .style(move |_| container::Style {
        border: Border {
            color: if speaking { t::ACCENT } else { t::TRANSPARENT },
            width: 2.,
            radius: t::ROUND.into(),
        },
        ..t::surface(t::BG_HOVER)
    });
    let mut flags = row![].spacing(t::S4).align_y(Center);
    if p.muted {
        flags = flags.push(icon("mic-off", t::INLINE_ICON, t::TEXT_MUTED));
    }
    if p.deafened {
        flags = flags.push(icon("headphone-off", t::INLINE_ICON, t::DANGER));
    }
    if p.tracks.iter().any(|t| t.kind == TrackKind::Screen) {
        flags = flags.push(
            container(hint("LIVE").color(t::TEXT_PRIMARY).font(t::semibold()))
                .padding([0., t::S4])
                .style(|_| container::Style {
                    border: Border {
                        radius: t::S4.into(),
                        ..Default::default()
                    },
                    ..t::surface(t::DANGER)
                }),
        );
    }
    if p.tracks.iter().any(|t| t.kind == TrackKind::Camera) {
        flags = flags.push(icon("video", t::INLINE_ICON, t::TEXT_MUTED));
    }
    let body: Element<'a, Msg> = if compact {
        row![
            face,
            txt(ellipsis(&app.state.model.name(p.session.user_id), 16))
                .size(14)
                .width(Fill),
            flags
        ]
        .spacing(t::S4)
        .align_y(Center)
        .height(28)
        .into()
    } else {
        let mut body = column![
            face,
            txt(ellipsis(&app.state.model.name(p.session.user_id), 24)).size(14),
            flags
        ]
        .spacing(t::S8)
        .align_x(Center);
        if has_stream {
            body = body.push(watch());
        }
        if p.tracks.iter().any(|t| t.kind == TrackKind::Camera) {
            let mut footer = row![
                txt(ellipsis(&app.state.model.name(p.session.user_id), 16)).width(Fill),
                flags_for_camera(p)
            ]
            .spacing(t::S4)
            .align_y(Center)
            .height(t::BUTTON);
            if has_stream {
                footer = footer.push(watch());
            }
            column![
                super::stream_view::video(
                    app,
                    p.session,
                    TrackKind::Camera,
                    tile_height - t::BUTTON - 2. * t::S4
                ),
                container(footer).padding([t::S4, t::S8])
            ]
            .width(Fill)
            .into()
        } else {
            container(body).center_x(Fill).center_y(tile_height).into()
        }
    };
    mouse_area(container(body).width(Fill).style(move |theme| {
        let mut style = if compact {
            t::bare(theme)
        } else {
            t::elevated(theme)
        };
        if speaking && !compact {
            style.border.color = t::ACCENT;
            style.border.width = t::FOCUS;
        }
        style
    }))
    .on_right_press(Msg::Open(Dialog::MemberMenu(p.session.user_id)))
    .into()
}
/// Call-only controls: native focus behavior, round hit targets and explicit
/// active/danger states, with no animation or redraw subscription.
fn control_button(
    name: &'static str,
    label: &'static str,
    action: Action,
    active: bool,
    danger: bool,
) -> Element<'static, Msg> {
    let warning = matches!(name, "mic-off" | "headphone-off");
    let active_color = if warning { t::DANGER } else { t::ACCENT };
    let color = if danger {
        t::TEXT_PRIMARY
    } else if active {
        active_color
    } else {
        t::TEXT_PRIMARY
    };
    tooltip(
        button(
            container(icon(name, t::ICON, color))
                .center_x(Fill)
                .center_y(Fill),
        )
        .on_press(msg(action))
        .centered()
        .width(t::GUILD_ICON)
        .height(t::GUILD_ICON)
        .padding(t::S12)
        .style(move |theme, status| {
            let mut style = if danger {
                t::danger(theme, status)
            } else {
                t::secondary(theme, status)
            };
            style.border.radius = t::ROUND.into();
            if active && !danger {
                style.background = Some(
                    t::alpha(
                        active_color,
                        if matches!(
                            status,
                            widget::button::Status::Hovered | widget::button::Status::Pressed
                        ) {
                            0.24
                        } else {
                            0.12
                        },
                    )
                    .into(),
                );
            }
            style
        }),
        hint(label),
        tooltip::Position::Top,
    )
    .style(t::tooltip)
    .into()
}
pub(super) fn controls(app: &App) -> Element<'_, Msg> {
    row![
        control_button(
            if app.calls.call.muted {
                "mic-off"
            } else {
                "mic"
            },
            if app.calls.call.muted {
                "Unmute (Ctrl+Shift+M)"
            } else {
                "Mute (Ctrl+Shift+M)"
            },
            Action::Mute,
            app.calls.call.muted,
            false
        ),
        control_button(
            if app.calls.call.deafened {
                "headphone-off"
            } else {
                "headphones"
            },
            if app.calls.call.deafened {
                "Undeafen (Ctrl+Shift+D)"
            } else {
                "Deafen (Ctrl+Shift+D)"
            },
            Action::Deafen,
            app.calls.call.deafened,
            false
        ),
        control_button(
            "video",
            if app.calls.camera {
                "Turn off camera"
            } else {
                "Turn on camera"
            },
            Action::ToggleCamera,
            app.calls.camera,
            false
        ),
        control_button(
            "monitor-up",
            if app.calls.sharing {
                "Stream quality"
            } else {
                "Share screen"
            },
            Action::Picker,
            app.calls.sharing,
            false
        ),
        control_button("phone-off", "Disconnect", Action::Leave, false, true),
    ]
    .spacing(t::S12)
    .align_y(Center)
    .into()
}
pub(super) fn connected(app: &App) -> Element<'_, Msg> {
    let Some(channel) = app.calls.call.desired_channel else {
        return space().height(0).into();
    };
    let name = app
        .state
        .model
        .channels
        .get(&channel)
        .map(|c| app.state.model.channel_name(c))
        .unwrap_or_default();
    let server = app
        .state
        .model
        .channels
        .get(&channel)
        .and_then(|c| c.guild_id)
        .and_then(|g| app.state.model.guilds.get(&g))
        .map(|g| g.guild.name.clone())
        .unwrap_or_default();
    let label = match app.calls.call.phase {
        Phase::Connected => "Voice Connected",
        Phase::Reconnecting => "Reconnecting…",
        _ => "Connecting…",
    };
    let mut body = column![
        row![
            icon("signal", t::INLINE_ICON, t::ACCENT),
            button(txt(label).size(14).font(t::semibold()).color(t::ACCENT))
                .on_press(msg(Action::ConnectionStats))
                .style(t::link)
                .width(Fill),
            icon_button("phone-off", "Disconnect", msg(Action::Leave))
        ]
        .spacing(t::S4)
        .align_y(Center),
        button(hint(format!("{name} / {server}")))
            .on_press(msg(Action::Join(channel)))
            .style(t::link),
        row![
            icon_button("video", "Camera", msg(Action::ToggleCamera)),
            icon_button("monitor-up", "Share Screen", msg(Action::Picker))
        ]
        .spacing(t::S8)
    ]
    .spacing(t::S4);
    if app.calls.connection_stats {
        let stats = app.calls.call.stats.as_ref();
        body = body.push(
            container(
                column![
                    section("CONNECTION QUALITY"),
                    hint(
                        stats
                            .and_then(|s| s.rtt_ms)
                            .map_or("Measuring ping…".into(), |ms| format!("{ms} ms ping"))
                    ),
                    hint(stats.and_then(|s| s.loss_per_million).map_or(
                        "Measuring packet loss…".into(),
                        |loss| format!("{:.2}% packet loss", f64::from(loss) / 10_000.)
                    )),
                ]
                .spacing(t::S4),
            )
            .padding(t::S8)
            .style(t::elevated),
        );
    }
    if app.calls.sharing {
        body = body.push(
            row![
                hint("Sharing your screen").width(Fill),
                icon_button("x", "Stop streaming", msg(Action::StopShare))
            ]
            .align_y(Center),
        );
    }
    container(body)
        .padding(t::S8)
        .width(Fill)
        .style(t::sidebar)
        .into()
}
pub(super) fn view(app: &App, id: Id) -> Element<'_, Msg> {
    let name = app
        .state
        .model
        .channels
        .get(&id)
        .map(|c| app.state.model.channel_name(c))
        .unwrap_or_default();
    let in_call = app.calls.call.desired_channel == Some(id);
    let mut grid = column![].spacing(t::S12);
    let participants: Vec<_> = if app.calls.call.channel == Some(id) {
        app.calls.call.participants.values().collect()
    } else {
        app.calls
            .call
            .guild_states
            .values()
            .filter(|s| s.channel_id == id)
            .map(|s| &s.participant)
            .collect()
    };
    if participants.is_empty() {
        grid = grid.push(
            container(
                column![
                    icon("headphones", t::CALL_AVATAR, t::TEXT_MUTED),
                    title("No one is in voice"),
                    hint("Join the call to get started."),
                ]
                .spacing(t::S16)
                .align_x(Center),
            )
            .center_x(Fill)
            .padding(t::S32),
        );
    } else if participants.len() == 1 {
        grid = grid.push(
            container(hint(
                if in_call && Some(participants[0].session) == app.calls.call.own {
                    "You're the only one here. Invite someone to join."
                } else {
                    "1 person in voice"
                },
            ))
            .center_x(Fill)
            .padding(t::S8),
        );
    }
    for chunk in participants.chunks(3) {
        let mut tiles = row![].spacing(t::S12);
        for p in chunk {
            tiles = tiles.push(container(participant(app, p, false)).width(Fill));
        }
        grid = grid.push(tiles);
    }
    let mut content = column![
        container(
            row![
                icon("volume-2", t::ICON, t::TEXT_SECONDARY),
                txt(name).font(t::semibold()).width(Fill),
                icon_button(
                    "signal",
                    "Connection statistics",
                    msg(Action::ConnectionStats)
                )
            ]
            .spacing(t::S12)
            .align_y(Center)
        )
        .padding([t::S8, t::S16])
        .height(t::HEADER),
        line(),
        container(if let Some(stream) = app.calls.focus.filter(|_| in_call) {
            super::stream_view::view(app, stream)
        } else {
            let max_width = participants.len().clamp(1, 3) as f32 * t::CALL_TILE_HEIGHT * 16. / 9.
                + 2. * t::S12;
            let grid = container(grid).max_width(max_width);
            if participants.len() <= 3 {
                container(grid).center_x(Fill).center_y(Fill).into()
            } else {
                container(container(scroller(grid)).max_width(max_width))
                    .center_x(Fill)
                    .into()
            }
        })
        .padding(t::S24)
        .height(Fill)
        .width(Fill),
    ];
    if app.calls.call.control.grant.is_some() {
        content = content.push(controlling(app));
    }
    if in_call {
        content = content.push(
            container(controls(app))
                .padding(t::S16)
                .center_x(Fill)
                .style(t::sidebar),
        );
    } else {
        content = content.push(
            container(primary("Join Voice", msg(Action::Join(id))))
                .padding(t::S24)
                .center_x(Fill),
        );
    }
    if app.calls.stats && app.calls.focus.is_none() {
        content = content.push(super::stream_view::stats(app));
    }
    container(content)
        .width(Fill)
        .height(Fill)
        .style(t::chat)
        .into()
}
pub(super) fn consent(app: &App) -> Element<'_, Msg> {
    let Some((requester, _)) = app.calls.call.control.pending else {
        return space().into();
    };
    container(
        column![
            title(format!(
                "{} wants to control your screen",
                app.state.model.name(requester.user_id)
            )),
            hint("Only allow people you trust. Control ends when you move your mouse or stop sharing."),
            hint("This request expires after 30 seconds."),
            row![action("Deny", msg(Action::DecideControl(false))), primary("Allow", msg(Action::DecideControl(true)))].spacing(t::S8)
        ]
        .spacing(t::S16),
    )
    .padding(t::S24)
    .max_width(480)
    .style(t::modal)
    .into()
}
pub(super) fn user_audio(app: &App, id: Id) -> Element<'_, Msg> {
    let audio = app.calls.call.settings.user(id);
    column![
        hint(format!("User Volume — {}%", audio.volume)),
        widget::slider(0..=200, audio.volume, move |v| msg(Action::UserVolume(
            id, v
        )))
        .style(t::slider),
        action(
            if audio.muted {
                "Unmute locally"
            } else {
                "Mute locally"
            },
            msg(Action::UserMute(id, !audio.muted))
        ),
    ]
    .spacing(t::S8)
    .into()
}

fn flags_for_camera(p: &VoiceParticipant) -> Element<'static, Msg> {
    let mut flags = row![].spacing(t::S4).align_y(Center);
    if p.muted {
        flags = flags.push(icon("mic-off", t::INLINE_ICON, t::TEXT_MUTED));
    }
    if p.deafened {
        flags = flags.push(icon("headphone-off", t::INLINE_ICON, t::DANGER));
    }
    if p.tracks.iter().any(|t| t.kind == TrackKind::Screen) {
        flags = flags.push(
            container(hint("LIVE").font(t::semibold()).color(t::TEXT_PRIMARY))
                .padding([0., t::S4])
                .style(|_| container::Style {
                    border: Border {
                        radius: t::S4.into(),
                        ..Default::default()
                    },
                    ..t::surface(t::DANGER)
                }),
        );
    }
    flags.into()
}

pub(super) fn controlling(app: &App) -> Element<'_, Msg> {
    let Some(grant) = app.calls.call.control.grant else {
        return space().into();
    };
    let name = app.state.model.name(grant.controller.user_id);
    let own = Some(grant.sharer) == app.calls.call.own;
    container(
        row![
            icon("monitor-up", t::INLINE_ICON, t::ACCENT),
            txt(if own {
                format!("{name} is controlling your screen — Stop (Ctrl+Alt+Shift+S)")
            } else {
                format!("{name} is controlling")
            })
            .width(Fill),
            action("Stop", msg(Action::Revoke))
        ]
        .spacing(t::S12)
        .align_y(Center),
    )
    .padding(t::S8)
    .style(t::elevated)
    .into()
}
