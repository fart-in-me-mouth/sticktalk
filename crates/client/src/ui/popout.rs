use super::*;
use crate::main_window::{mm, Entity, Message as Main, Popover};
use iced::widget::column;
pub fn profile(app: &App, id: Id) -> Element<'_, Msg> {
    let detail = app.state.model.launch.profiles.get(&id);
    let profile = detail.map(|d| &d.profile);
    let name = app.state.model.name(id);
    let user = app.state.model.users.get(&id);
    let color = profile.map_or(t::BG_ELEVATED, |p| controls::rgb(p.banner_color));
    let banner = container(row![
        space().width(Fill),
        controls::icon_button(
            "more-horizontal",
            "More",
            mm(Main::Open(Popover::Context(Entity::User(id)))),
            false
        )
    ])
    .height(t::BANNER_H)
    .padding(t::S8)
    .style(move |_| container::Style {
        border: Border {
            radius: iced::border::Radius {
                top_left: t::S8,
                top_right: t::S8,
                ..Default::default()
            },
            ..Default::default()
        },
        ..t::surface(color)
    });
    let face = container(controls::avatar(
        app,
        id,
        t::PROFILE_AVATAR,
        t::BG_HOVER,
        false,
    ))
    .padding(t::MENU_PADDING)
    .style(|_| container::Style {
        border: Border {
            radius: t::ROUND.into(),
            ..Default::default()
        },
        ..t::surface(t::BG_HOVER)
    });
    let top = stack![
        column![banner, space().height(t::PROFILE_AVATAR / 2.)],
        container(face).padding(Padding {
            top: t::BANNER_H - t::PROFILE_AVATAR / 2.,
            left: t::S16,
            ..Padding::ZERO
        })
    ];
    let mut content = column![
        txt(name)
            .font(t::semibold())
            .size(t::TITLE)
            .color(controls::name_color(app, id)),
        txt(format!(
            "@{}{}",
            user.map_or("unknown", |u| u.username.as_str()),
            profile
                .filter(|p| !p.pronouns.is_empty())
                .map_or(String::new(), |p| format!(" · {}", p.pronouns))
        ))
        .size(t::USER_NAME)
    ]
    .spacing(t::S8);
    if let Some(d) = detail {
        content = content.push(
            row![
                button(
                    hint(format!("{} Mutual Servers", d.mutual_servers.len()))
                        .color(t::TEXT_SECONDARY)
                )
                .on_press(mm(Main::Open(Popover::Mutual(id, true))))
                .padding(0)
                .style(t::icon),
                hint("·"),
                button(
                    hint(format!("{} Mutual Friends", d.mutual_friends.len()))
                        .color(t::TEXT_SECONDARY)
                )
                .on_press(mm(Main::Open(Popover::Mutual(id, false))))
                .padding(0)
                .style(t::icon)
            ]
            .spacing(t::S4)
            .align_y(Center),
        );
        if let Some(s) = user.and_then(|u| u.status_text.as_deref()) {
            content = content.push(txt(s).size(t::USER_NAME));
        }
        if !d.profile.about_me.is_empty() {
            content = content.push(txt(&d.profile.about_me).size(t::USER_NAME));
        }
        let joined = app
            .state
            .guild
            .and_then(|g| d.mutual_servers.iter().find(|s| s.guild.id == g))
            .map_or(d.profile.created_at, |s| s.member_since);
        content = content.push(section("Member since")).push(hint({
            let d = crate::datetime::local(joined);
            crate::datetime::member_since(d)
        }));
    } else {
        content = content.push(hint("Loading profile…"));
    }
    if let Some(g) = app.state.guild {
        let mut header = row![section("Roles").width(Fill)].align_y(Center);
        if app.permissions().contains(Permissions::MANAGE_ROLES) {
            header = header.push(controls::icon_button(
                "plus",
                "Add Role",
                mm(Main::Open(Popover::Roles(id))),
                false,
            ));
        }
        content = content.push(header);
        let roles = app.state.model.launch.member_roles(g, id);
        let mut chips = widget::Row::new().spacing(t::S4);
        for r in roles.into_iter().filter(|r| r.id != g) {
            chips = chips.push(
                container(
                    row![
                        dot(controls::rgb(r.color), t::S8),
                        hint(&r.name).color(t::TEXT_PRIMARY)
                    ]
                    .spacing(t::S4)
                    .align_y(Center),
                )
                .padding([t::S4, t::S8])
                .style(|_| container::Style {
                    border: Border {
                        radius: t::S4.into(),
                        ..Default::default()
                    },
                    ..t::surface(t::ROLE_CHIP)
                }),
            );
        }
        content = content.push(chips.wrap());
    }
    if Some(id) == app.me() {
        content = content.push(
            primary("Edit Profile", mm(Main::EditProfile))
                .height(t::PROFILE_ACTION)
                .width(Fill),
        );
    } else {
        let relationship = app.state.model.relationships.get(&id).map(|r| r.kind);
        let (label, message, enabled) = match relationship {
            Some(RelationshipKind::Friend) => (
                "Message",
                Msg::Command(Command::OpenDm { user_id: id }),
                true,
            ),
            Some(RelationshipKind::IncomingRequest) => (
                "Accept Friend Request",
                Msg::Command(Command::AcceptFriendRequest { user_id: id }),
                true,
            ),
            Some(RelationshipKind::OutgoingRequest) => {
                ("Friend Request Sent", Msg::Activity, false)
            }
            Some(RelationshipKind::Blocked) => (
                "Unblock",
                Msg::Command(Command::UnblockUser { user_id: id }),
                true,
            ),
            None => (
                "Add Friend",
                Msg::Command(Command::SendFriendRequest {
                    username: user.map_or(String::new(), |u| u.username.clone()),
                }),
                user.is_some(),
            ),
        };
        content = content.push(
            primary(label, message.clone())
                .on_press_maybe(enabled.then_some(message))
                .height(t::PROFILE_ACTION)
                .width(Fill),
        );
        content = content.push(
            text_input(
                &format!("Message @{}", user.map_or("user", |u| u.username.as_str())),
                &app.main.message,
            )
            .id("popout-message")
            .on_input_maybe(
                (relationship != Some(RelationshipKind::Blocked)).then_some(|v| mm(Main::Draft(v))),
            )
            .on_submit(mm(Main::SendDm(id)))
            .padding(t::S8)
            .size(t::USER_NAME)
            .font(t::regular())
            .style(t::input),
        );
    }
    container(column![top, container(content).padding([t::S12, t::S16])])
        .width(t::POPOUT_W)
        .style(|_| container::Style {
            background: Some(t::BG_HOVER.into()),
            ..controls::surface(&iced::Theme::Dark)
        })
        .into()
}
pub fn pins(app: &App, id: Id) -> Element<'_, Msg> {
    let pins = app.state.model.launch.pins.get(&id);
    let mut list = column![
        row![
            icon("pin", t::ICON, t::TEXT_SECONDARY),
            txt("Pinned Messages").font(t::semibold()).width(Fill),
            controls::icon_button("x", "Close", mm(Main::Close), false)
        ]
        .spacing(t::S8)
        .align_y(Center),
        line()
    ]
    .spacing(t::S12);
    if let Some(pins) = pins {
        if pins.is_empty() {
            list = list.push(hint("No pinned messages in this channel."));
        }
        for m in pins {
            let mut actions = row![button(txt("Jump").size(t::USER_NAME).color(t::ACCENT))
                .on_press(Msg::JumpTo(m.id))
                .padding(t::S4)
                .style(t::icon)]
            .align_y(Center);
            if app
                .channel_permissions(id)
                .contains(Permissions::MANAGE_MESSAGES)
            {
                actions = actions.push(controls::icon_button(
                    "pin-off",
                    "Unpin Message",
                    mm(Main::Pin(id, m.id, false)),
                    false,
                ));
            }
            list = list.push(
                container(
                    column![
                        button(
                            txt(app.state.model.name(m.author_id))
                                .font(t::medium())
                                .color(controls::name_color(app, m.author_id))
                        )
                        .on_press(controls::profile_message(m.author_id))
                        .padding(0)
                        .style(t::icon),
                        hint(timestamp(m.created_at, false)),
                        txt(ellipsis(&app.state.model.content(m), 200)),
                        actions
                    ]
                    .spacing(t::S4),
                )
                .padding(t::S12)
                .style(t::elevated),
            );
        }
    } else {
        list = list.push(hint("Loading pinned messages…"));
    }
    container(scroller(list).height(Length::Shrink))
        .padding(t::S16)
        .width(t::POPOUT_W + t::S32)
        .max_height(app.height - t::HEADER - t::S32)
        .style(controls::surface)
        .into()
}
pub fn quick(app: &App) -> Element<'_, Msg> {
    let header = column![
        text_input("Where would you like to go?", &app.main.quick)
            .id("quick-switcher")
            .on_input(|s| mm(Main::Quick(s)))
            .on_submit(mm(Main::QuickChoose))
            .padding(t::S12)
            .font(t::regular())
            .size(t::CHANNEL_NAME)
            .style(t::input),
        hint("↑↓ to choose · Enter to jump · Esc to close").color(t::TEXT_SECONDARY)
    ]
    .spacing(t::S8);
    let mut list = Column::new().spacing(t::S4);
    for (index, (name, e, server)) in app.quick_items().into_iter().enumerate() {
        let msg = match e {
            Entity::Channel(id) => Msg::Select(id),
            Entity::Guild(id) => Msg::Guild(Some(id)),
            _ => mm(Main::Close),
        };
        list = list.push(
            button(
                row![
                    icon(
                        match e {
                            Entity::Guild(_) => "message-circle",
                            Entity::Channel(id) =>
                                match app.state.model.channels.get(&id).map(|c| c.kind) {
                                    Some(ChannelKind::Direct) => "at-sign",
                                    Some(ChannelKind::GroupDm) => "users",
                                    Some(ChannelKind::Voice) => "volume-2",
                                    _ => "hash",
                                },
                            _ => "hash",
                        },
                        t::ICON,
                        t::TEXT_SECONDARY
                    ),
                    column![
                        elided::label(name).font(t::medium()).width(Fill),
                        elided::label(server)
                            .size(t::HINT)
                            .color(t::TEXT_SECONDARY)
                            .width(Fill)
                    ]
                    .width(Fill)
                ]
                .spacing(t::S12)
                .align_y(Center),
            )
            .on_press(msg)
            .width(Fill)
            .height(t::QUICK_ROW)
            .padding(t::S8)
            .style(t::navigation(app.main.quick_index == index)),
        );
    }
    if app.quick_items().is_empty() {
        list = list.push(
            container(
                column![
                    icon("search", t::S32, t::TEXT_SECONDARY),
                    txt("No matching conversations"),
                    hint("Try a different name.").color(t::TEXT_SECONDARY)
                ]
                .spacing(t::S12)
                .align_x(Center),
            )
            .padding(t::S24)
            .center_x(Fill),
        );
    }
    container(
        column![
            header,
            container(scroller(list).id("quick-results").height(Length::Shrink))
                .max_height(crate::main_window::quick_results_height(app.height))
        ]
        .spacing(t::S16),
    )
    .padding(t::S16)
    .width(t::QUICK_W)
    .style(controls::surface)
    .into()
}
pub fn mutual(app: &App, id: Id, servers: bool) -> Element<'_, Msg> {
    let mut list = column![
        txt(if servers {
            "Mutual Servers"
        } else {
            "Mutual Friends"
        })
        .font(t::semibold()),
        line()
    ]
    .spacing(t::S8);
    if let Some(d) = app.state.model.launch.profiles.get(&id) {
        if servers {
            for s in &d.mutual_servers {
                list = list.push(
                    action(s.guild.name.clone(), Msg::Guild(Some(s.guild.id))).style(t::icon),
                );
            }
        } else {
            for u in &d.mutual_friends {
                list = list.push(
                    action(app.state.model.name(u.id), controls::profile_message(u.id))
                        .style(t::icon),
                );
            }
        }
    }
    container(list)
        .padding(t::S16)
        .width(t::POPOUT_W)
        .style(controls::surface)
        .into()
}
pub fn roles(app: &App, id: Id) -> Element<'_, Msg> {
    let mut list = column![txt("Roles").font(t::semibold()), line()].spacing(t::S4);
    if let Some(g) = app.state.guild {
        let mine = app
            .me()
            .map(|u| {
                app.state
                    .model
                    .launch
                    .member_roles(g, u)
                    .first()
                    .map_or(0, |r| r.position)
            })
            .unwrap_or(0);
        let owner = app
            .state
            .model
            .guilds
            .get(&g)
            .is_some_and(|g| Some(g.guild.owner_id) == app.me());
        let selected = app.state.model.launch.member_roles(g, id);
        for r in app
            .state
            .model
            .launch
            .roles
            .get(&g)
            .into_iter()
            .flatten()
            .filter(|r| r.id != g && (owner || r.position < mine))
        {
            list = list.push(controls::item(
                &r.name,
                if selected.iter().any(|s| s.id == r.id) {
                    "check"
                } else {
                    "circle"
                },
                mm(Main::Role(id, r.id)),
                false,
            ));
        }
    }
    menus::card(list)
}
pub fn nickname(app: &App, id: Id) -> Element<'_, Msg> {
    container(
        column![
            txt("Change Nickname").font(t::semibold()),
            hint("Use a different name in this server. Leave empty to reset."),
            text_input("Nickname", &app.main.message)
                .id("nickname")
                .on_input(|s| mm(Main::Nickname(s)))
                .on_submit(mm(Main::SaveNickname(id)))
                .padding(t::S8)
                .font(t::regular())
                .style(t::input),
            primary("Save", mm(Main::SaveNickname(id)))
        ]
        .spacing(t::S12),
    )
    .padding(t::S16)
    .width(t::POPOUT_W)
    .style(controls::surface)
    .into()
}
