use super::*;
use crate::main_window::{mm, Entity, Message as Main, Popover};
use controls::{item, separator};
use iced::widget::column;
pub fn server(app: &App) -> Element<'_, Msg> {
    let Some(g) = app.state.guild.and_then(|g| app.state.model.guilds.get(&g)) else {
        return space().into();
    };
    let p = app.permissions();
    let mut list = column![
        item(
            "Invite to Server",
            "user-plus",
            Msg::Open(Dialog::Invite),
            false
        ),
        item(
            "Server Settings",
            "settings",
            Msg::Open(Dialog::GuildSettings(GuildTab::Overview)),
            false
        )
    ]
    .spacing(0);
    if p.contains(Permissions::MANAGE_CHANNELS) {
        list = list
            .push(item(
                "Create Channel",
                "plus-circle",
                Msg::Open(Dialog::NewChannel(ChannelKind::Text, None)),
                false,
            ))
            .push(item(
                "Create Category",
                "folder",
                Msg::Open(Dialog::NewChannel(ChannelKind::Category, None)),
                false,
            ));
    }
    list = list
        .push(separator())
        .push(item(
            "Notification Settings",
            "bell",
            mm(Main::Open(Popover::Notifications(Some(g.guild.id), None))),
            false,
        ))
        .push(item(
            "Privacy Settings",
            "shield",
            Msg::Open(Dialog::UserSettings),
            false,
        ))
        .push(separator())
        .push(item(
            "Edit Per-server Profile",
            "user-round",
            mm(Main::Open(Popover::Nickname(app.me().unwrap()))),
            false,
        ));
    let hide = app
        .state
        .model
        .launch
        .settings(app.state.guild, None)
        .hide_muted_channels;
    list = list.push(item(
        "Hide Muted Channels",
        if hide { "square-check" } else { "square" },
        mm(Main::HideMuted),
        false,
    ));
    if Some(g.guild.owner_id) != app.me() {
        list = list.push(separator()).push(item(
            "Leave Server",
            "log-out",
            Msg::Open(Dialog::Confirm(
                "Leave this server?".into(),
                Command::LeaveGuild {
                    guild_id: g.guild.id,
                },
            )),
            true,
        ));
    }
    card(list)
}
pub fn card<'a>(items: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    container(scroller(items).height(Length::Shrink))
        .padding(t::S8)
        .width(t::CONTEXT_W)
        .style(|theme| container::Style {
            border: Border {
                radius: t::S4.into(),
                ..controls::surface(theme).border
            },
            ..controls::surface(theme)
        })
        .into()
}
pub fn entity<'a>(app: &'a App, e: &'a Entity) -> Element<'a, Msg> {
    let mut list = column![].spacing(0);
    let id = match e {
        Entity::User(id) => {
            list = list.push(item(
                "Profile",
                "user-round",
                controls::profile_message(*id),
                false,
            ));
            if Some(*id) != app.me() {
                list = list
                    .push(item(
                        "Message",
                        "message-circle",
                        Msg::Command(Command::OpenDm { user_id: *id }),
                        false,
                    ))
                    .push(item(
                        "Add Friend",
                        "user-plus",
                        Msg::Command(Command::SendFriendRequest {
                            username: app
                                .state
                                .model
                                .users
                                .get(id)
                                .map_or(String::new(), |u| u.username.clone()),
                        }),
                        false,
                    ));
            }
            if Some(*id) != app.me() && app.calls.call.channel.is_some() {
                list = list
                    .push(separator())
                    .push(container(super::call::user_audio(app, *id)).padding([t::S8, t::S8]));
            }
            if app.state.guild.is_some()
                && (Some(*id) == app.me()
                    && app.permissions().contains(Permissions::CHANGE_NICKNAME)
                    || app.permissions().contains(Permissions::MANAGE_NICKNAMES))
            {
                list = list.push(item(
                    "Change Nickname",
                    "pencil",
                    mm(Main::Open(Popover::Nickname(*id))),
                    false,
                ));
            }
            if let Some(g) = app.state.guild {
                let owner = app.state.model.guilds.get(&g).map(|g| g.guild.owner_id);
                if Some(*id) != owner && Some(*id) != app.me() {
                    if app.permissions().contains(Permissions::KICK_MEMBERS) {
                        list = list.push(separator()).push(item(
                            "Kick Member",
                            "user-minus",
                            Msg::Open(Dialog::Confirm(
                                "Kick this member?".into(),
                                Command::KickMember {
                                    guild_id: g,
                                    user_id: *id,
                                },
                            )),
                            true,
                        ));
                    }
                    if app.permissions().contains(Permissions::BAN_MEMBERS) {
                        list = list.push(item(
                            "Ban Member",
                            "ban",
                            Msg::Open(Dialog::Confirm(
                                "Ban this member?".into(),
                                Command::BanMember {
                                    guild_id: g,
                                    user_id: *id,
                                    reason: None,
                                },
                            )),
                            true,
                        ));
                    }
                }
            }
            Some(*id)
        }
        Entity::Channel(id) => {
            if let Some(c) = app.state.model.channels.get(id) {
                if c.kind != ChannelKind::Category {
                    list = list
                        .push(item("Open Channel", "hash", Msg::Select(*id), false))
                        .push(item(
                            "Notification Settings",
                            "bell",
                            mm(Main::Open(Popover::Notifications(c.guild_id, Some(*id)))),
                            false,
                        ));
                }
                if app
                    .channel_permissions(*id)
                    .contains(Permissions::CREATE_INVITES)
                {
                    list = list.push(item(
                        "Create Invite",
                        "user-plus",
                        Msg::Open(Dialog::Invite),
                        false,
                    ));
                }
                if app
                    .channel_permissions(*id)
                    .contains(Permissions::MANAGE_CHANNELS)
                {
                    list = list.push(item(
                        "Edit Channel",
                        "settings",
                        Msg::Open(Dialog::ChannelSettings(*id)),
                        false,
                    ));
                }
            }
            Some(*id)
        }
        Entity::Guild(id) => {
            list = list.push(item(
                "Add to Folder",
                "folder",
                crate::features3::msg(crate::features3::Event::FolderAdd(*id)),
                false,
            ));
            if app
                .state
                .model
                .launch
                .server_folders
                .iter()
                .any(|f| f.guild_ids.contains(id))
            {
                list = list.push(item(
                    "Remove from Folder",
                    "folder",
                    crate::features3::msg(crate::features3::Event::FolderRemove(*id)),
                    false,
                ));
            }
            list = list
                .push(item(
                    "Open Server",
                    "message-circle",
                    Msg::Guild(Some(*id)),
                    false,
                ))
                .push(item(
                    "Notification Settings",
                    "bell",
                    mm(Main::Open(Popover::Notifications(Some(*id), None))),
                    false,
                ));
            Some(*id)
        }
        Entity::Rail => {
            list = list
                .push(item(
                    "Add a Server",
                    "plus",
                    Msg::Open(Dialog::AddGuild),
                    false,
                ))
                .push(controls::toggle(
                    "Developer Mode",
                    app.settings.preferences.developer_mode,
                    mm(Main::Developer(!app.settings.preferences.developer_mode)),
                ));
            None
        }
        Entity::Link(url) => {
            list = list
                .push(item(
                    "Open Link",
                    "external-link",
                    Msg::Link(url.clone()),
                    false,
                ))
                .push(item("Copy Link", "copy", Msg::Copy(url.clone()), false));
            None
        }
    };
    if app.settings.preferences.developer_mode {
        if let Some(id) = id {
            list = list.push(separator()).push(item(
                "Copy ID",
                "copy",
                Msg::Copy(id.0.to_string()),
                false,
            ));
        }
    }
    card(list)
}
pub fn notifications(app: &App, guild: Option<Id>, channel: Option<Id>) -> Element<'_, Msg> {
    let mut n = app.state.model.launch.settings(guild, channel);
    n.guild_id = guild;
    n.channel_id = channel;
    let mut list = column![
        txt("Notification Settings")
            .font(t::semibold())
            .size(t::CHANNEL_NAME),
        hint(if channel.is_some() {
            "Channel override"
        } else {
            "Server defaults"
        }),
        separator()
    ];
    for (label, level) in [
        ("All Messages", NotificationLevel::All),
        ("Only @mentions", NotificationLevel::Mentions),
        ("Nothing", NotificationLevel::Nothing),
    ] {
        let mut next = n.clone();
        next.level = level;
        list = list.push(item(
            label,
            if n.level == level { "check" } else { "circle" },
            mm(Main::Notify(next)),
            false,
        ));
    }
    let mut next = n.clone();
    next.muted = !n.muted;
    next.mute_until = None;
    list = list
        .push(separator())
        .push(controls::toggle("Mute", n.muted, mm(Main::Notify(next))));
    let mut next = n.clone();
    next.suppress_everyone = !n.suppress_everyone;
    list = list.push(controls::toggle(
        "Suppress @everyone",
        n.suppress_everyone,
        mm(Main::Notify(next)),
    ));
    card(list)
}
pub fn devices(mic: bool) -> Element<'static, Msg> {
    card(
        column![
            txt(if mic { "Input Device" } else { "Output Device" })
                .font(t::semibold())
                .size(t::CHANNEL_NAME),
            separator(),
            item("System Default", "check", mm(Main::Close), false),
            hint("Device selection is configured in Voice & Video."),
            item(
                "Voice & Video",
                "settings",
                Msg::Open(Dialog::UserSettings),
                false
            )
        ]
        .spacing(t::S4),
    )
}
