use super::*;
use iced::widget::column;
use iced::Padding;
pub(super) fn label(tab: GuildTab) -> &'static str {
    match tab {
        GuildTab::Overview => "Server Profile",
        GuildTab::Engagement => "Engagement",
        GuildTab::Channels => "Channels",
        GuildTab::Members => "Members",
        GuildTab::Roles => "Roles",
        GuildTab::Invites => "Invites",
        GuildTab::Bans => "Bans",
        GuildTab::Emoji => "Emoji",
        GuildTab::Access => "Access",
        GuildTab::AuditLog => "Audit Log",
    }
}
pub(super) fn navigation<'a>(
    app: &'a App,
    tab: GuildTab,
    mut nav: Column<'a, Msg>,
    q: &str,
) -> Column<'a, Msg> {
    for (group, entries) in [
        (
            "",
            vec![
                ("user-round", GuildTab::Overview),
                ("bell", GuildTab::Engagement),
            ],
        ),
        ("EXPRESSION", vec![("smile", GuildTab::Emoji)]),
        (
            "PEOPLE",
            vec![
                ("users", GuildTab::Members),
                ("shield", GuildTab::Roles),
                ("link", GuildTab::Invites),
                ("shield", GuildTab::Access),
            ],
        ),
        (
            "MODERATION",
            vec![("file", GuildTab::AuditLog), ("ban", GuildTab::Bans)],
        ),
    ] {
        if !group.is_empty() {
            nav = nav.push(space().height(t::S16)).push(section(group));
        }
        for (i, p) in entries {
            if label(p).to_lowercase().contains(q) {
                nav = nav.push(nav_item(i, label(p), A::Server(p), tab == p));
            }
        }
    }
    if app
        .state
        .guild
        .and_then(|g| app.state.model.guilds.get(&g))
        .is_some_and(|g| Some(g.guild.owner_id) == app.me())
    {
        nav = nav.push(space().height(t::S16)).push(line()).push(
            button(
                row![
                    label_text("Delete Server")
                        .size(st::SMALL)
                        .color(t::DANGER)
                        .width(Fill),
                    icon("trash-2", st::NAV_ICON, t::DANGER)
                ]
                .align_y(Center),
            )
            .on_press(Msg::Settings(A::DeleteServer))
            .height(st::NAV_ROW)
            .width(Fill)
            .padding(t::S8)
            .style(t::danger_link),
        );
    }
    nav
}
pub(super) fn content<'a>(app: &'a App, page: &Dialog) -> Element<'a, Msg> {
    let Dialog::GuildSettings(tab) = page else {
        return space().into();
    };
    let Some(gid) = app.state.guild else {
        return hint("Select a server.").into();
    };
    match tab {
        GuildTab::Overview=>profile(app),GuildTab::Roles=>roles::content(app),GuildTab::Members=>members(app),GuildTab::Invites=>invites(app,gid),GuildTab::Bans=>bans(app,gid),
        GuildTab::Engagement=>column![title("Engagement"),section("SYSTEM MESSAGES CHANNEL"),disabled_tip(widget::pick_list(vec!["No system messages".to_owned()],Some("No system messages".to_owned()),|_|Msg::Settings(A::Server(GuildTab::Engagement))).padding(t::S12).style(st::pick_list).menu_style(st::menu),false,"System messages channels are not available on this server yet."),line(),title("Default Notifications"),small("Default notification preferences for new members are not available on this server yet."),secondary("Your Notification Settings",A::Notifications(gid),true)].spacing(t::S24).into(),
        GuildTab::Emoji=>super::super::features3::settings(app),
        GuildTab::Channels=>{let mut c=column![title("Channels")].spacing(t::S16);for ch in app.sorted_channels(){c=c.push(row![label_text(app.state.model.channel_name(ch)).width(Fill),icon_button("settings","Edit Channel",Msg::Open(Dialog::ChannelSettings(ch.id)))].align_y(Center));}c.into()}
        _=>column![title(label(*tab)),small(match tab{GuildTab::Emoji=>"Custom server emoji management is not available yet.",GuildTab::Access=>"Manage who can join with server invites and member moderation.",_=>"Audit logs are not available yet."})].spacing(t::S24).into(),
    }
}
fn profile(app: &App) -> Element<'_, Msg> {
    let p = &app.settings.server;
    let enabled =
        app.permissions().contains(Permissions::MANAGE_GUILD) && app.settings.saving.is_empty();
    let mut grid = Column::new().spacing(t::S8);
    for chunk in (0..10u8).collect::<Vec<_>>().chunks(5) {
        let mut r = Row::new().spacing(t::S8);
        for i in chunk {
            let i = *i;
            r = r.push(disabled_tip(
                button(space().width(Fill).height(st::SWATCH_H))
                    .width(Fill)
                    .on_press_maybe(enabled.then_some(Msg::Settings(A::Banner(i))))
                    .padding(0)
                    .style(move |_, _| widget::button::Style {
                        background: Some(st::gradient(i)),
                        border: Border {
                            color: if p.banner == i {
                                st::WHITE
                            } else {
                                t::TRANSPARENT
                            },
                            width: t::FOCUS,
                            radius: t::S8.into(),
                        },
                        ..Default::default()
                    }),
                enabled,
                LOCKED,
            ));
        }
        grid = grid.push(r);
    }
    let form = column![
        title("Server Profile"),
        field("NAME", "Server name", &p.name, F::ServerName, enabled),
        line(),
        section("ICON"),
        small("We recommend an image of at least 512x512"),
        row![
            super::super::action(
                "Change Server Icon",
                Msg::PickUpload(UploadPurpose::GuildIcon)
            )
            .on_press_maybe(enabled.then_some(Msg::PickUpload(UploadPurpose::GuildIcon)))
            .style(t::primary),
            secondary("Remove Icon", A::RemoveIcon, enabled && p.icon.is_some())
                .style(st::danger_secondary)
        ]
        .spacing(t::S12),
        line(),
        section("BANNER COLOR"),
        grid,
        line(),
        section("DESCRIPTION"),
        textarea(&app.settings.description_editor, false, enabled),
        row![
            space().width(Fill),
            hint(format!("{}/120", p.description.chars().count()))
        ]
    ]
    .spacing(t::S16);
    let form = container(
        scroller(form.padding(iced::Padding {
            bottom: if app.settings.server_dirty() {
                t::INPUT + t::S32 * 2.
            } else {
                0.
            },
            ..iced::Padding::ZERO
        }))
        .id("settings-server-form")
        .height((app.height - st::TOP * 2.).max(t::ATTACHMENT_CARD)),
    )
    .width(Fill);
    if st::columns(app.width).1 < st::CONTENT {
        column![form, preview(app)].spacing(t::S24).into()
    } else {
        row![form, preview(app)].spacing(t::S24).into()
    }
}

fn preview(app: &App) -> Element<'_, Msg> {
    let p = &app.settings.server;
    let data = app
        .state
        .guild
        .and_then(|g| app.state.model.settings.guild_profiles.get(&g));
    let count = app
        .state
        .guild
        .and_then(|g| app.state.model.guilds.get(&g))
        .map_or(0, |g| g.guild.member_count);
    let icon_widget: Element<'_, Msg> =
        if let Some(handle) = p.icon.and_then(|id| app.images.get(id)) {
            widget::image(handle)
                .width(st::SERVER_ICON)
                .height(st::SERVER_ICON)
                .into()
        } else {
            container(title(p.name.chars().take(2).collect::<String>()))
                .center_x(st::SERVER_ICON)
                .center_y(st::SERVER_ICON)
                .style(|_| {
                    let mut s = t::surface(t::BG_SELECTED);
                    s.border.radius = t::GUILD_RADIUS.into();
                    s
                })
                .into()
        };
    let banner = container(space().width(Fill).height(st::BANNER)).style(move |_| {
        let mut s = st::banner(t::BG_ELEVATED);
        s.background = Some(st::gradient(p.banner));
        s
    });
    let meta = row![
        super::super::dot(t::ACCENT, t::S8),
        hint(format!("{} Online", data.map_or(0, |p| p.online_count))),
        super::super::dot(t::TEXT_MUTED, t::S8),
        hint(format!("{count} Members"))
    ]
    .spacing(t::S8)
    .align_y(Center);
    let details = column![
        label_text(&p.name),
        small(&p.description),
        meta,
        hint(format!(
            "Est. {}",
            data.map_or("—".into(), |p| month_year(p.profile.created_at))
        ))
    ]
    .spacing(t::S16);
    let base = column![
        banner,
        container(details).padding(Padding {
            top: st::SERVER_ICON / 2. + t::S16,
            left: t::S16,
            right: t::S16,
            bottom: t::S16
        })
    ];
    container(stack![
        base,
        container(icon_widget).padding(Padding {
            top: st::BANNER - st::SERVER_ICON / 2.,
            left: t::S16,
            ..Padding::ZERO
        })
    ])
    .width(st::PREVIEW)
    .style(|_| {
        let mut s = t::surface(t::BG_SIDEBAR);
        s.border.radius = t::S8.into();
        s
    })
    .into()
}
fn members(app: &App) -> Element<'_, Msg> {
    let gid = app.state.guild.unwrap();
    let q = app.settings.search.to_lowercase();
    let mut filters = row![secondary("All Members", A::MemberFilter(None), true)
        .style(t::navigation(app.settings.member_filter.is_none()))]
    .spacing(t::S8);
    for role in app
        .state
        .model
        .settings
        .roles
        .values()
        .filter(|r| r.guild_id == gid && r.id != gid)
    {
        filters = filters.push(
            secondary(&role.name, A::MemberFilter(Some(role.id)), true)
                .style(t::navigation(app.settings.member_filter == Some(role.id))),
        );
    }
    let mut c = column![
        title("Members"),
        input("Search members", &app.settings.search, F::Search, true),
        scroller(filters)
            .direction(widget::scrollable::Direction::Horizontal(
                widget::scrollable::Scrollbar::new().width(t::S4)
            ))
            .style(t::scroll),
        row![
            section("MEMBER").width(Fill),
            section("ROLES").width(t::SIDEBAR),
            section("JOINED").width(t::ATTACHMENT_CARD),
            space().width(t::ICON + 2. * t::S8)
        ]
        .spacing(t::S12)
        .width(Fill),
        line()
    ]
    .spacing(t::S16);
    for m in app.state.model.members.values().filter(|m| {
        m.guild_id == gid
            && (app.state.model.name(m.user.id).to_lowercase().contains(&q)
                || m.user.username.to_lowercase().contains(&q))
            && app
                .settings
                .member_filter
                .is_none_or(|r| m.role_ids.contains(&r))
    }) {
        let chips = m
            .role_ids
            .iter()
            .filter_map(|id| app.state.model.settings.roles.get(id))
            .filter(|r| r.id != gid)
            .fold(Column::new().spacing(t::S4), |c, r| c.push(role_chip(r)));
        c = c
            .push(
                row![
                    avatar(app, m.user.id, t::SMALL_AVATAR, true),
                    column![
                        label_text(ellipsis(&app.state.model.name(m.user.id), 24)),
                        hint(format!("@{}", m.user.username))
                    ]
                    .spacing(t::S4)
                    .width(Fill),
                    container(chips).width(t::SIDEBAR),
                    hint(
                        crate::datetime::date(m.joined_at)
                            .split(" at ")
                            .next()
                            .unwrap_or("")
                            .to_owned()
                    )
                    .width(t::ATTACHMENT_CARD),
                    member_action(app, m.user.id)
                ]
                .spacing(t::S12)
                .align_y(Center),
            )
            .push(line());
    }
    if app.state.member_after.is_some() {
        c = c
            .push(super::super::action("Load more members", Msg::MoreMembers).style(st::secondary));
    }
    c.into()
}
fn invites(app: &App, gid: Id) -> Element<'_, Msg> {
    let mut c = column![
        title("Invites"),
        small("Manage active invite links for this server."),
        super::super::action("Invite People", Msg::Open(Dialog::Invite))
            .on_press_maybe(
                app.permissions()
                    .contains(Permissions::CREATE_INVITES)
                    .then_some(Msg::Open(Dialog::Invite))
            )
            .style(t::primary),
        row![
            section("INVITE CODE").width(Fill),
            section("USES").width(t::ATTACHMENT_CARD),
            space().width(t::INPUT + t::S12 + t::ATTACHMENT_CARD)
        ]
        .spacing(t::S12)
        .width(Fill),
        line()
    ]
    .spacing(t::S24);
    let invites: Vec<_> = app
        .state
        .model
        .invites
        .values()
        .filter(|v| v.guild_id == gid)
        .collect();
    if invites.is_empty() {
        c = c.push(small("No active invites."));
    }
    for v in invites {
        c = c
            .push(
                row![
                    label_text(&v.code).width(Fill),
                    small(format!("{} / {}", v.uses, v.max_uses)).width(t::ATTACHMENT_CARD),
                    container(icon_button(
                        "copy",
                        "Copy Invite",
                        Msg::Copy(v.code.clone())
                    ))
                    .width(t::INPUT),
                    disabled_tip(
                        super::super::action(
                            "Revoke",
                            Msg::Command(Command::RevokeInvite {
                                guild_id: gid,
                                code: v.code.clone()
                            })
                        )
                        .on_press_maybe(
                            app.permissions()
                                .contains(Permissions::MANAGE_GUILD)
                                .then_some(Msg::Command(Command::RevokeInvite {
                                    guild_id: gid,
                                    code: v.code.clone()
                                }))
                        )
                        .style(t::danger_link)
                        .width(t::ATTACHMENT_CARD),
                        app.permissions().contains(Permissions::MANAGE_GUILD),
                        LOCKED
                    )
                ]
                .spacing(t::S12)
                .align_y(Center),
            )
            .push(line());
    }
    if app.state.invite_after.is_some() {
        c = c
            .push(super::super::action("Load more invites", Msg::MoreInvites).style(st::secondary));
    }
    c.into()
}
fn bans(app: &App, gid: Id) -> Element<'_, Msg> {
    let mut c = column![
        title("Bans"),
        small("Banned members cannot rejoin this server."),
        line()
    ]
    .spacing(t::S24);
    let bans: Vec<_> = app
        .state
        .model
        .bans
        .values()
        .filter(|v| v.guild_id == gid)
        .collect();
    if bans.is_empty() {
        c = c
            .push(icon("shield", t::AVATAR, t::TEXT_MUTED))
            .push(label_text("No banned members."));
    }
    for b in bans {
        c = c
            .push(
                row![
                    avatar(app, b.user.id, t::SMALL_AVATAR, false),
                    column![
                        label_text(app.state.model.name(b.user.id)),
                        small(b.reason.clone().unwrap_or_default())
                    ]
                    .spacing(t::S8)
                    .width(Fill),
                    super::super::action(
                        "Unban",
                        Msg::Command(Command::UnbanMember {
                            guild_id: gid,
                            user_id: b.user.id
                        })
                    )
                    .on_press_maybe(
                        app.permissions()
                            .contains(Permissions::BAN_MEMBERS)
                            .then_some(Msg::Command(Command::UnbanMember {
                                guild_id: gid,
                                user_id: b.user.id
                            }))
                    )
                    .style(st::secondary)
                ]
                .spacing(t::S16)
                .align_y(Center),
            )
            .push(line());
    }
    if app.state.ban_after.is_some() {
        c = c.push(super::super::action("Load more bans", Msg::MoreBans).style(st::secondary));
    }
    c.into()
}
pub(super) fn delete_modal(app: &App) -> Element<'_, Msg> {
    let name = app
        .state
        .guild
        .and_then(|g| app.state.model.guilds.get(&g))
        .map_or("", |g| g.guild.name.as_str());
    container(
        column![
            title(format!("Delete ‘{name}’")),
            small("This cannot be undone. All channels and messages will be permanently deleted."),
            section("ENTER THE SERVER NAME TO CONFIRM"),
            input(
                "Server name",
                &app.settings.delete_name,
                F::DeleteName,
                true
            ),
            row![
                space().width(Fill),
                secondary("Cancel", A::CancelDelete, true),
                primary(
                    "Delete Server",
                    A::ConfirmDelete,
                    app.settings.delete_name == name
                )
                .style(t::danger)
            ]
            .spacing(t::S12)
        ]
        .spacing(t::S24),
    )
    .width(t::MODAL_W)
    .padding(t::S24)
    .style(t::modal)
    .into()
}
