use super::*;
use crate::main_window::{mm, muted, Entity, Message as Main, Popover};
use iced::widget::column;
pub fn view(app: &App) -> Element<'_, Msg> {
    let name = app
        .state
        .guild
        .and_then(|g| app.state.model.guilds.get(&g))
        .map_or("Direct messages".into(), |g| g.guild.name.clone());
    let mut header = row![button(
        row![
            elided::label(name)
                .font(t::semibold())
                .size(t::CHANNEL_NAME)
                .width(Fill),
            icon(
                if app.main.popover == Some(Popover::Server) {
                    "chevron-up"
                } else {
                    "chevron-down"
                },
                t::INLINE_ICON,
                t::TEXT_SECONDARY
            )
        ]
        .align_y(Center)
    )
    .on_press(if app.state.guild.is_some() {
        mm(Main::Open(Popover::Server))
    } else {
        mm(Main::Open(Popover::Quick))
    })
    .width(Fill)
    .height(t::HEADER)
    .padding([t::S8, t::S12])
    .style(t::icon)]
    .align_y(Center);
    if app.state.guild.is_some() {
        header = header.push(controls::icon_button(
            "user-plus",
            "Invite People",
            Msg::Open(Dialog::Invite),
            false,
        ));
    }
    let mut list = column![].spacing(0);
    if app.state.guild.is_none() {
        list = list
            .push(nav(
                "users",
                "Friends",
                Msg::Friends(FriendTab::Online),
                matches!(app.state.screen, Screen::Friends(_)),
            ))
            .push(space().height(t::S16))
            .push(
                row![
                    section("Direct Messages").width(Fill),
                    controls::icon_button("plus", "New Group", Msg::Open(Dialog::NewGroup), false)
                ]
                .align_y(Center),
            );
    }
    let hide = app
        .state
        .model
        .launch
        .settings(app.state.guild, None)
        .hide_muted_channels;
    for c in app.sorted_channels() {
        if c.kind == ChannelKind::Category {
            let hover = app.main.hovered_category == Some(c.id);
            let collapsed = app.main.collapsed.contains(&c.id);
            let mut header = row![button(
                row![
                    icon(
                        if collapsed {
                            "chevron-right"
                        } else {
                            "chevron-down"
                        },
                        t::S12,
                        if hover {
                            t::TEXT_PRIMARY
                        } else {
                            t::TEXT_MUTED
                        }
                    ),
                    elided::label(app.state.model.channel_name(c))
                        .size(t::HINT)
                        .font(t::semibold())
                        .color(if hover {
                            t::TEXT_PRIMARY
                        } else {
                            t::TEXT_MUTED
                        })
                        .width(Fill)
                ]
                .spacing(t::S4)
                .align_y(Center)
            )
            .on_press(mm(Main::Category(c.id)))
            .padding(0)
            .height(t::S32)
            .width(Fill)
            .style(t::icon)]
            .align_y(Center);
            if hover && app.permissions().contains(Permissions::MANAGE_CHANNELS) {
                header = header.push(controls::icon_button(
                    "plus",
                    "Create Channel",
                    Msg::Open(Dialog::NewChannel(ChannelKind::Text, Some(c.id))),
                    false,
                ));
            }
            list = list.push(space().height(t::S12)).push(
                mouse_area(header)
                    .on_enter(mm(Main::CategoryHover(Some(c.id))))
                    .on_exit(mm(Main::CategoryLeave(c.id)))
                    .on_right_press(mm(Main::Open(Popover::Context(Entity::Channel(c.id))))),
            );
            continue;
        }
        let selected = app.channel_selected(c.id);
        let unread = app
            .state
            .model
            .reads
            .get(&c.id)
            .is_some_and(|r| r.unread_count > 0);
        if hide && muted(app, c.id) && !selected {
            continue;
        }
        if c.parent_id.is_some_and(|p| app.main.collapsed.contains(&p))
            && !selected
            && !unread
            && !app.own_voice().is_some_and(|v| v.channel_id == c.id)
        {
            continue;
        }
        list = list.push(channel_row(app, c));
        if c.kind == ChannelKind::Voice {
            for v in app
                .state
                .model
                .launch
                .voice
                .values()
                .filter(|v| v.channel_id == c.id)
            {
                list = list.push(participant(app, v));
            }
        }
    }
    container(column![
        header,
        line(),
        container(
            mouse_area(scroller(list).id("channels").height(Fill))
                .on_exit(mm(Main::ChannelHover(None)))
        )
        .padding([t::S8, t::S8])
        .height(Fill),
        voice_connected(app),
        crate::features::update_pill(app),
        user_panel(app)
    ])
    .width(t::SIDEBAR)
    .height(Fill)
    .style(t::sidebar)
    .into()
}
fn channel_row<'a>(app: &'a App, c: &'a ChannelSummary) -> Element<'a, Msg> {
    let selected = app.channel_selected(c.id);
    let hovered = app.main.hovered_channel == Some(c.id);
    let muted = muted(app, c.id);
    let opacity = if muted { 0.4 } else { 1. };
    let read = app.state.model.reads.get(&c.id);
    let unread = read.is_some_and(|r| r.unread_count > 0);
    let active = selected || hovered;
    let color = t::alpha(
        if active || unread {
            t::TEXT_PRIMARY
        } else {
            t::TEXT_SECONDARY
        },
        opacity,
    );
    let glyph = if app.state.model.launch.is_private(c) {
        "lock-keyhole"
    } else {
        match c.kind {
            ChannelKind::Voice => "volume-2",
            ChannelKind::GroupDm => "users",
            ChannelKind::Direct => "at-sign",
            _ => "hash",
        }
    };
    let leading: Element<'_, Msg> = if c.kind == ChannelKind::Direct {
        c.recipient_ids
            .iter()
            .copied()
            .find(|id| Some(*id) != app.me())
            .map(|id| {
                controls::avatar_fade(
                    app,
                    id,
                    t::S24,
                    if selected {
                        t::BG_SELECTED
                    } else if hovered {
                        t::BG_HOVER
                    } else {
                        t::BG_SIDEBAR
                    },
                    false,
                    opacity,
                )
            })
            .unwrap_or_else(|| {
                icon(
                    glyph,
                    t::ICON,
                    t::alpha(
                        if active {
                            t::TEXT_SECONDARY
                        } else {
                            t::TEXT_MUTED
                        },
                        opacity,
                    ),
                )
                .into()
            })
    } else {
        icon(
            glyph,
            t::ICON,
            t::alpha(
                if active {
                    t::TEXT_SECONDARY
                } else {
                    t::TEXT_MUTED
                },
                opacity,
            ),
        )
        .into()
    };
    // Actions are siblings, not nested buttons: a gear/invite never selects the row.
    let label = button(
        row![
            leading,
            elided::label(app.state.model.channel_name(c))
                .size(t::CHANNEL_NAME)
                .font(if selected || unread {
                    t::semibold()
                } else {
                    t::medium()
                })
                .color(color)
                .width(Fill)
        ]
        .spacing(t::S8)
        .align_y(Center),
    )
    .on_press(Msg::Select(c.id))
    .height(t::CHANNEL_ROW)
    .width(Fill)
    .padding([0., t::S8])
    .style(t::navigation(selected));
    let mut actions = row![label].spacing(0).align_y(Center);
    if active && c.guild_id.is_some() {
        if app
            .channel_permissions(c.id)
            .contains(Permissions::CREATE_INVITES)
        {
            actions = actions.push(controls::icon_button(
                "user-plus",
                "Create Invite",
                Msg::Open(Dialog::Invite),
                false,
            ));
        }
        if app
            .channel_permissions(c.id)
            .contains(Permissions::MANAGE_CHANNELS)
        {
            actions = actions.push(controls::icon_button(
                "settings",
                "Edit Channel",
                Msg::Open(Dialog::ChannelSettings(c.id)),
                false,
            ));
        }
    } else if let Some(r) = read.filter(|r| r.mention_count > 0) {
        actions = actions.push(
            container(
                hint(super::rail::mention_label(r.mention_count))
                    .font(t::semibold())
                    .color(t::BADGE_TEXT),
            )
            .padding([0., t::S8])
            .style(|_| container::Style {
                border: Border {
                    radius: t::ROUND.into(),
                    ..Default::default()
                },
                ..t::surface(t::DANGER_BUTTON)
            }),
        );
    }
    let row = container(actions)
        .id(format!("channel-row-{}", c.id.0))
        .height(t::CHANNEL_ROW)
        .style(move |_| container::Style {
            border: Border {
                radius: t::TOOLTIP_RADIUS.into(),
                ..Default::default()
            },
            ..t::surface(if selected {
                t::BG_SELECTED
            } else if hovered {
                t::BG_HOVER
            } else {
                t::TRANSPARENT
            })
        });
    let marker = container(
        container(space())
            .width(t::S4)
            .height(t::S8)
            .style(move |_| container::Style {
                border: Border {
                    radius: t::ROUND.into(),
                    ..Default::default()
                },
                ..t::surface(if unread && !selected {
                    t::TEXT_PRIMARY
                } else {
                    t::TRANSPARENT
                })
            }),
    )
    .center_y(t::CHANNEL_ROW);
    observe::anchor(
        mouse_area(stack![row, marker])
            .on_enter(mm(Main::ChannelHover(Some(c.id))))
            .on_move(move |_| mm(Main::ChannelHover(Some(c.id))))
            .on_exit(mm(Main::ChannelLeave(c.id)))
            .on_right_press(mm(Main::Open(Popover::Context(Entity::Channel(c.id))))),
    )
}
fn participant<'a>(app: &'a App, v: &'a GuildVoiceState) -> Element<'a, Msg> {
    let p = &v.participant;
    let user = p.session.user_id;
    let speaking = app.state.model.launch.speaking.contains(&p.session);
    let mut body = row![
        controls::voice_avatar(app, user, speaking),
        txt(ellipsis(&app.state.model.name(user), 12))
            .size(t::USER_NAME)
            .color(controls::name_color(app, user))
            .width(Fill)
    ]
    .spacing(t::S8)
    .align_y(Center);
    if p.muted {
        body = body.push(icon("mic-off", t::INLINE_ICON, t::TEXT_MUTED));
    }
    if p.deafened {
        body = body.push(icon("headphone-off", t::INLINE_ICON, t::TEXT_MUTED));
    }
    if p.tracks.iter().any(|t| t.kind == TrackKind::Screen) {
        body = body.push(
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
    if p.tracks.iter().any(|t| t.kind == TrackKind::Camera) {
        body = body.push(icon("video", t::INLINE_ICON, t::TEXT_MUTED));
    }
    container(observe::anchor(
        mouse_area(
            button(body)
                .on_press(controls::profile_message(user))
                .width(Fill)
                .height(t::VOICE_USER_ROW)
                .padding(0)
                .style(t::icon),
        )
        .on_right_press(mm(Main::Open(Popover::Context(Entity::User(user))))),
    ))
    .padding(Padding {
        left: t::S32 + t::S4,
        ..Padding::ZERO
    })
    .into()
}
pub fn voice_connected(app: &App) -> Element<'_, Msg> {
    if app.calls.call.desired_channel.is_some() {
        return call::connected(app);
    }
    let Some(v) = app.own_voice() else {
        return space().height(0).into();
    };
    let channel = app
        .state
        .model
        .channels
        .get(&v.channel_id)
        .map_or("Voice".into(), |c| app.state.model.channel_name(c));
    let server = app
        .state
        .model
        .guilds
        .get(&v.guild_id)
        .map_or("".into(), |g| g.guild.name.clone());
    let label = row![
        controls::tip(
            icon("signal", t::ICON, t::ACCENT),
            app.state
                .model
                .launch
                .rtt_ms
                .map_or("Ping unavailable".into(), |p| format!("{p} ms")),
            tooltip::Position::Top
        ),
        column![
            txt("Voice Connected")
                .font(t::semibold())
                .size(t::USER_NAME)
                .color(t::ACCENT),
            button(
                hint(ellipsis(&format!("{channel} / {server}"), 22))
                    .wrapping(iced::widget::text::Wrapping::None)
            )
            .on_press(mm(Main::FocusVoice(v.channel_id)))
            .padding(0)
            .style(t::icon)
        ]
        .width(Fill),
        controls::icon_button("phone-off", "Disconnect", mm(Main::Disconnect), false)
    ]
    .spacing(t::S4)
    .align_y(Center);
    use crate::main_window::MediaAction;
    let tile = |glyph: &str, label: &str, action| {
        controls::tip(
            button(icon(glyph, t::ICON, t::TEXT_SECONDARY))
                .on_press(mm(Main::Media(action)))
                .width(Fill)
                .height(t::S32)
                .padding(t::TOOL_ICON_PADDING)
                .style(|theme, status| {
                    let mut style = t::icon(theme, status);
                    style.background = Some(t::BG_HOVER.into());
                    style.border.radius = t::S4.into();
                    style
                }),
            label,
            tooltip::Position::Top,
        )
    };
    let tiles = row![
        tile("video", "Camera", MediaAction::Camera),
        tile("monitor-up", "Share Screen", MediaAction::Screen)
    ]
    .spacing(t::S8);
    let mut panel = column![label, tiles].spacing(t::S8);
    if v.participant
        .tracks
        .iter()
        .any(|p| p.kind == TrackKind::Screen)
    {
        panel = panel.push(
            row![
                icon("monitor-up", t::INLINE_ICON, t::ACCENT),
                hint("Screen 1").width(Fill),
                controls::icon_button(
                    "x",
                    "Stop Sharing",
                    mm(Main::Media(MediaAction::StopScreen)),
                    false
                )
            ]
            .spacing(t::S8)
            .align_y(Center),
        );
    }
    container(panel)
        .padding(t::S8)
        .width(Fill)
        .style(|_| t::surface(t::USER_PANEL))
        .into()
}
fn split<'a>(app: &App, mic: bool) -> Element<'a, Msg> {
    let on = if mic {
        app.main.muted
    } else {
        app.main.deafened
    };
    let glyph = if mic {
        if on {
            "mic-off"
        } else {
            "mic"
        }
    } else if on {
        "headphone-off"
    } else {
        "headphones"
    };
    let mut body = row![button(icon(
        glyph,
        t::ICON,
        if on { t::DANGER } else { t::TEXT_SECONDARY }
    ))
    .on_press(mm(if mic { Main::Mute } else { Main::Deafen }))
    .width(t::S32)
    .height(t::S32)
    .padding(t::TOOL_ICON_PADDING)
    .style(move |theme, status| {
        let mut s = t::icon(theme, status);
        if on {
            s.background = Some(t::alpha(t::DANGER, 0.12).into());
        }
        s
    })]
    .spacing(0);
    let hovered = app.main.split_hover == Some(mic);
    if hovered {
        body = body.push(
            button(icon("chevron-down", t::S12, t::TEXT_SECONDARY))
                .on_press(mm(Main::Open(Popover::Devices(mic))))
                .width(t::S16)
                .height(t::S32)
                .padding(0)
                .style(t::icon),
        );
    }
    controls::tip(
        mouse_area(container(body).style(move |_| container::Style {
            border: Border {
                radius: t::TOOLTIP_RADIUS.into(),
                ..Default::default()
            },
            ..t::surface(if hovered { t::BG_HOVER } else { t::TRANSPARENT })
        }))
        .on_enter(mm(Main::SplitHover(Some(mic))))
        .on_exit(mm(Main::SplitHover(None))),
        if mic { "Mute" } else { "Deafen" },
        tooltip::Position::Top,
    )
}
fn user_panel(app: &App) -> Element<'_, Msg> {
    let Some(own) = app.state.model.me.as_ref() else {
        return space().into();
    };
    let me = own.user.id;
    let sub = if app.main.user_hover {
        format!("@{}", own.user.username)
    } else {
        own.user
            .status_text
            .clone()
            .unwrap_or_else(|| status(own.desired_presence).0.into())
    };
    let identity = observe::anchor(
        mouse_area(
            button(
                row![
                    controls::avatar(app, me, t::SMALL_AVATAR, t::USER_PANEL, false),
                    column![
                        txt(ellipsis(
                            &app.state.model.name(me),
                            if app.main.split_hover.is_some() {
                                7
                            } else {
                                10
                            }
                        ))
                        .wrapping(widget::text::Wrapping::None)
                        .font(t::semibold())
                        .size(t::USER_NAME),
                        hint(ellipsis(
                            &sub,
                            if app.main.split_hover.is_some() {
                                9
                            } else {
                                12
                            }
                        ))
                        .wrapping(widget::text::Wrapping::None)
                    ]
                    .spacing(0)
                    .width(Fill)
                ]
                .spacing(t::S4)
                .align_y(Center),
            )
            .on_press(controls::profile_message(me))
            .width(Fill)
            .height(t::USER_HEIGHT)
            .padding(0)
            .style(t::icon),
        )
        .on_enter(mm(Main::UserHover(true)))
        .on_exit(mm(Main::UserHover(false)))
        .on_right_press(mm(Main::Open(Popover::Context(Entity::User(me))))),
    );
    container(
        container(
            row![
                identity,
                split(app, true),
                split(app, false),
                controls::icon_button(
                    "settings",
                    "User Settings",
                    Msg::Open(Dialog::UserSettings),
                    false
                )
            ]
            .spacing(0)
            .align_y(Center),
        )
        .padding([0., t::S4])
        .height(t::USER_HEIGHT)
        .style(|_| container::Style {
            border: Border {
                radius: t::S8.into(),
                ..Default::default()
            },
            ..t::surface(t::USER_PANEL)
        }),
    )
    .padding(t::S8)
    .into()
}
