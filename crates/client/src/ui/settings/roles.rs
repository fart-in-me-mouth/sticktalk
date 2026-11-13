use super::*;
use iced::widget::column;
pub(super) const PERMISSIONS: &[(&str, Permissions, &str, &str)] = &[
    (
        "General Server Permissions",
        Permissions::MANAGE_EMOJI,
        "Manage Emoji",
        "Upload, rename, and delete custom server emoji.",
    ),
    (
        "General Server Permissions",
        Permissions::VIEW_CHANNEL,
        "View Channels",
        "Allows members to view channels.",
    ),
    (
        "General Server Permissions",
        Permissions::MANAGE_CHANNELS,
        "Manage Channels",
        "Create, edit, and delete channels.",
    ),
    (
        "General Server Permissions",
        Permissions::MANAGE_GUILD,
        "Manage Server",
        "Edit the server name, icon, and profile.",
    ),
    (
        "General Server Permissions",
        Permissions::MANAGE_ROLES,
        "Manage Roles",
        "Create and edit roles below your highest role.",
    ),
    (
        "Membership",
        Permissions::MANAGE_MEMBERS,
        "Manage Members",
        "Manage memberships in this server.",
    ),
    (
        "Membership",
        Permissions::CREATE_INVITES,
        "Create Invite",
        "Invite new people to this server.",
    ),
    (
        "Membership",
        Permissions::CHANGE_NICKNAME,
        "Change Nickname",
        "Change your own server nickname.",
    ),
    (
        "Membership",
        Permissions::MANAGE_NICKNAMES,
        "Manage Nicknames",
        "Change nicknames of members below your highest role.",
    ),
    (
        "Membership",
        Permissions::KICK_MEMBERS,
        "Kick Members",
        "Remove members from this server.",
    ),
    (
        "Membership",
        Permissions::BAN_MEMBERS,
        "Ban Members",
        "Permanently ban members from this server.",
    ),
    (
        "Text Channel Permissions",
        Permissions::SEND_MESSAGES,
        "Send Messages",
        "Send messages in text channels.",
    ),
    (
        "Text Channel Permissions",
        Permissions::READ_HISTORY,
        "Read Message History",
        "Read messages sent before joining a channel.",
    ),
    (
        "Text Channel Permissions",
        Permissions::ATTACH_FILES,
        "Attach Files",
        "Upload files and images to channels.",
    ),
    (
        "Text Channel Permissions",
        Permissions::ADD_REACTIONS,
        "Add Reactions",
        "Add new reactions to messages.",
    ),
    (
        "Text Channel Permissions",
        Permissions::EMBED_LINKS,
        "Embed Links",
        "Include link previews in messages.",
    ),
    (
        "Text Channel Permissions",
        Permissions::MENTION_EVERYONE,
        "Mention @everyone, @here, and All Roles",
        "Notify everyone or mention roles that are not mentionable.",
    ),
    (
        "Text Channel Permissions",
        Permissions::MANAGE_MESSAGES,
        "Manage Messages",
        "Delete messages by other members and manage pins.",
    ),
    (
        "Voice Channel Permissions",
        Permissions::CONNECT,
        "Connect",
        "Join voice channels.",
    ),
    (
        "Voice Channel Permissions",
        Permissions::SPEAK,
        "Speak",
        "Speak in voice channels.",
    ),
    (
        "Voice Channel Permissions",
        Permissions::STREAM,
        "Stream",
        "Share your screen and system audio.",
    ),
    (
        "Voice Channel Permissions",
        Permissions::VIDEO,
        "Video",
        "Share camera video.",
    ),
    (
        "Voice Channel Permissions",
        Permissions::MUTE_MEMBERS,
        "Mute Members",
        "Mute other members in voice channels.",
    ),
    (
        "Voice Channel Permissions",
        Permissions::DEAFEN_MEMBERS,
        "Deafen Members",
        "Deafen other members in voice channels.",
    ),
    (
        "Voice Channel Permissions",
        Permissions::MOVE_MEMBERS,
        "Move Members",
        "Move and disconnect members in voice channels.",
    ),
    (
        "Voice Channel Permissions",
        Permissions::USE_VOICE_ACTIVITY,
        "Use Voice Activity",
        "Transmit using voice activity instead of push to talk.",
    ),
    (
        "Voice Channel Permissions",
        Permissions::PRIORITY_SPEAKER,
        "Priority Speaker",
        "Prioritize this member's voice while speaking.",
    ),
    (
        "Advanced",
        Permissions::ADMINISTRATOR,
        "Administrator",
        "Grants all permissions and bypasses channel permissions. Use with care.",
    ),
];
pub(super) fn editable(app: &App, role: &Role) -> bool {
    app.state
        .model
        .guilds
        .get(&role.guild_id)
        .zip(app.me())
        .is_some_and(|(g, me)| {
            app.state.model.settings.can_edit_role(
                &g.guild,
                me,
                app.state.model.settings.roles.get(&role.id).unwrap_or(role),
            )
        })
        && app.settings.saving.is_empty()
}
fn can_grant(app: &App, p: Permissions) -> bool {
    app.permissions().contains(p)
}
pub(super) fn content(app: &App) -> Element<'_, Msg> {
    if let Some(role) = &app.settings.role {
        return editor(app, role);
    }
    list(app)
}
fn list(app: &App) -> Element<'_, Msg> {
    let gid = app.state.guild.unwrap();
    let q = app.settings.search.to_lowercase();
    let mut roles: Vec<_> = app
        .state
        .model
        .settings
        .roles
        .values()
        .filter(|r| r.guild_id == gid && r.id != gid && r.name.to_lowercase().contains(&q))
        .collect();
    roles.sort_by_key(|r| std::cmp::Reverse(r.position));
    let can_create = app.permissions().contains(Permissions::MANAGE_ROLES);
    let c=column![title("Roles"),small("Use roles to group your server members and assign permissions."),button(row![icon("shield",t::S32,t::TEXT_SECONDARY),column![label_text("Default Permissions"),small("@everyone • applies to all server members")].spacing(t::S4).width(Fill),icon("chevron-right",t::ICON,t::TEXT_SECONDARY)].spacing(t::S16).align_y(Center)).on_press(Msg::Settings(A::EditRole(gid))).padding(t::S16).width(Fill).style(t::choice),row![input("Search Roles",&app.settings.search,F::Search,true),disabled_tip(primary("Create Role",A::CreateRole,can_create),can_create,LOCKED)].spacing(t::S16).align_y(Center),small("Members use the color of the highest role they have on this list. Drag roles to reorder them."),row![section(format!("Roles — {}",roles.len())).width(Fill),section("MEMBERS").width(t::ATTACHMENT_CARD),space().width(2.*t::INPUT+t::S12)].spacing(t::S12).width(Fill),line()].spacing(t::S16);
    let mut table = Column::new().spacing(0);
    for r in roles {
        let allowed = editable(app, r);
        let count = app
            .state
            .model
            .members
            .values()
            .filter(|m| m.guild_id == gid && m.role_ids.contains(&r.id))
            .count();
        let row = container(
            row![
                disabled_tip(
                    mouse_area(icon("menu", t::INLINE_ICON, t::TEXT_MUTED))
                        .on_press(Msg::Settings(A::DragStart(r.id))),
                    allowed,
                    LOCKED
                ),
                icon(
                    "shield",
                    t::ICON,
                    if r.color == 0 {
                        t::TEXT_SECONDARY
                    } else {
                        st::rgb(r.color)
                    }
                ),
                label_text(&r.name).width(Fill),
                row![
                    small(count.to_string()),
                    icon("user-round", t::INLINE_ICON, t::TEXT_MUTED)
                ]
                .spacing(t::S8)
                .width(t::ATTACHMENT_CARD)
                .align_y(Center),
                row_action("pencil", "Edit Role", A::EditRole(r.id), true),
                row_action(
                    "more-horizontal",
                    "More Role Actions",
                    A::RoleMenu(r.id),
                    allowed
                )
            ]
            .spacing(t::S12)
            .align_y(Center),
        )
        .padding([t::S8, 0.])
        .height(st::TABLE_ROW)
        .style(move |_| {
            t::surface(if app.settings.dragging == Some(r.id) {
                t::BG_SELECTED
            } else {
                t::BG_CHAT
            })
        });
        table = table.push(
            mouse_area(row)
                .on_enter(Msg::Settings(A::DragOver(r.id)))
                .on_move(move |_| Msg::Settings(A::DragOver(r.id)))
                .on_release(Msg::Settings(A::DragEnd))
                .on_exit(Msg::Settings(A::DragLeave(r.id))),
        );
        if app.settings.dragging.is_some() && app.settings.drag_over == Some(r.id) {
            table = table.push(
                container(space().height(t::FOCUS).width(Fill)).style(|_| t::surface(t::ACCENT)),
            );
        }
        if app.settings.role_menu == Some(r.id) {
            table = table.push(row![
                space().width(Fill),
                secondary("Delete Role", A::DeleteRole(r.id), allowed).style(t::danger)
            ]);
        }
    }
    c.push(table).into()
}
fn editor<'a>(app: &'a App, role: &'a Role) -> Element<'a, Msg> {
    let mut roles: Vec<_> = app
        .state
        .model
        .settings
        .roles
        .values()
        .filter(|r| r.guild_id == role.guild_id)
        .collect();
    roles.sort_by_key(|r| std::cmp::Reverse(r.position));
    let mut inner = column![
        row![
            button(
                row![
                    icon("arrow-left", t::INLINE_ICON, t::ACCENT),
                    small("Back").color(t::ACCENT)
                ]
                .spacing(t::S8)
                .align_y(Center)
            )
            .on_press(Msg::Settings(A::BackRoles))
            .height(t::BUTTON)
            .padding([t::S8, t::S12])
            .style(t::link),
            row_action(
                "plus",
                "Create Role",
                A::CreateRole,
                app.permissions().contains(Permissions::MANAGE_ROLES)
            )
        ]
        .align_y(Center),
        line()
    ]
    .spacing(t::S8);
    for r in roles {
        inner = inner.push(
            button(
                row![
                    super::super::dot(st::role_color(r.color), t::S8),
                    small(ellipsis(&r.name, 16))
                ]
                .spacing(t::S8)
                .align_y(Center),
            )
            .on_press(Msg::Settings(A::EditRole(r.id)))
            .padding([t::S8, t::S12])
            .height(st::NAV_ROW)
            .width(Fill)
            .style(t::navigation(r.id == role.id)),
        );
    }
    let mut panel = column![row![
        title(format!("Edit Role — {}", role.name)).width(Fill),
        row_action(
            "more-horizontal",
            "Role Actions",
            A::RoleMenu(role.id),
            editable(app, role)
        )
    ]
    .align_y(Center)]
    .spacing(t::S24);
    let mut tabs = Row::new().spacing(t::S24);
    let tab = app.settings.role_tab.unwrap_or(RoleTab::Display);
    for (label, v) in [
        ("Display", RoleTab::Display),
        ("Permissions", RoleTab::Permissions),
        ("Manage Members", RoleTab::Members),
    ] {
        tabs = tabs.push(
            button(
                column![
                    small(label).color(if tab == v {
                        t::TEXT_PRIMARY
                    } else {
                        t::TEXT_SECONDARY
                    }),
                    container(space().width(Fill).height(t::FOCUS)).style(move |_| t::surface(
                        if tab == v { t::ACCENT } else { t::TRANSPARENT }
                    ))
                ]
                .spacing(t::S12),
            )
            .on_press(Msg::Settings(A::RoleTab(v)))
            .padding(0)
            .style(t::icon),
        );
    }
    panel = panel.push(tabs).push(match tab {
        RoleTab::Display => display(app, role),
        RoleTab::Permissions => permissions(app, role),
        RoleTab::Members => members(app, role),
    });
    let panel: Element<'a, Msg> = if app.settings.role_menu == Some(role.id) {
        let can_delete = editable(app, role) && role.id != role.guild_id;
        let menu = container(disabled_tip(
            button(small("Delete Role").style(t::inherit_text))
                .on_press_maybe(can_delete.then_some(Msg::Settings(A::DeleteRole(role.id))))
                .height(t::ROW)
                .width(Fill)
                .padding([t::S4, t::S8])
                .style(st::menu_danger),
            can_delete,
            if role.id == role.guild_id {
                "The @everyone role cannot be deleted."
            } else {
                LOCKED
            },
        ))
        .width(st::MENU_WIDTH)
        .padding(st::MENU_PADDING)
        .style(st::menu_surface);
        stack![
            panel,
            container(
                widget::float(opaque(menu))
                    .translate(|_, _| iced::Vector::new(0., t::HEADER + t::S8))
            )
            .align_right(Fill)
            .style(t::bare)
        ]
        .into()
    } else {
        panel.into()
    };
    row![
        container(scroller(inner)).width(t::SIDEBAR - t::S32),
        container(panel).width(Fill)
    ]
    .spacing(t::S24)
    .into()
}
fn display<'a>(app: &'a App, r: &Role) -> Element<'a, Msg> {
    let allowed = editable(app, r);
    column![
        field(
            "ROLE NAME",
            "Role name",
            &app.settings.role.as_ref().unwrap().name,
            F::RoleName,
            allowed && r.id != r.guild_id
        ),
        section("ROLE COLOR"),
        disabled_tip(swatches(r.color, false, allowed), allowed, LOCKED),
        field(
            "CUSTOM COLOR",
            "#000000",
            &app.settings.role_hex,
            F::RoleHex,
            allowed
        ),
        line(),
        toggle(
            "Display role members separately",
            "Show members with this role in their own group.",
            r.hoist,
            A::Toggle(T::Hoist, !r.hoist),
            allowed && r.id != r.guild_id
        ),
        line(),
        toggle(
            "Allow anyone to @mention this role",
            "Members can notify everyone assigned this role.",
            r.mentionable,
            A::Toggle(T::Mentionable, !r.mentionable),
            allowed && r.id != r.guild_id
        ),
    ]
    .spacing(t::S24)
    .into()
}
fn permissions<'a>(app: &'a App, r: &Role) -> Element<'a, Msg> {
    let q = app.settings.permission_search.to_lowercase();
    let allowed = editable(app, r);
    let mut c = column![input(
        "Search permissions",
        &app.settings.permission_search,
        F::PermissionSearch,
        true
    )]
    .spacing(t::S24);
    for section_name in [
        "General Server Permissions",
        "Membership",
        "Text Channel Permissions",
        "Voice Channel Permissions",
        "Advanced",
    ] {
        let entries: Vec<_> = PERMISSIONS
            .iter()
            .filter(|(s, _, n, d)| {
                *s == section_name
                    && (n.to_lowercase().contains(&q) || d.to_lowercase().contains(&q))
            })
            .collect();
        if entries.is_empty() {
            continue;
        }
        let all = entries
            .iter()
            .fold(Permissions::empty(), |p, (_, bit, _, _)| p | *bit);
        let clear = all & app.permissions();
        c = c.push(
            row![
                title(section_name).width(Fill),
                disabled_tip(
                    secondary("Clear permissions", A::ClearPermissions(clear), allowed)
                        .style(t::link),
                    allowed,
                    LOCKED
                )
            ]
            .align_y(Center),
        );
        let mut rows = Column::new().spacing(0);
        for (_, bit, name, desc) in entries {
            rows = rows
                .push(toggle(
                    name,
                    desc,
                    r.permissions.contains(*bit),
                    A::Permission(*bit, !r.permissions.contains(*bit)),
                    allowed && can_grant(app, *bit),
                ))
                .push(line());
        }
        c = c.push(rows);
    }
    c.into()
}
fn members<'a>(app: &'a App, r: &Role) -> Element<'a, Msg> {
    let allowed = editable(app, r) && r.id != r.guild_id;
    let q = app.settings.search.to_lowercase();
    let mut c = column![row![
        input("Search Members", &app.settings.search, F::Search, true),
        disabled_tip(
            primary(
                if app.settings.adding_members {
                    "Done"
                } else {
                    "Add Members"
                },
                A::AddMembers(!app.settings.adding_members),
                allowed
            ),
            allowed,
            LOCKED
        )
    ]
    .spacing(t::S16)]
    .spacing(t::S16);
    let members: Vec<_> = app
        .state
        .model
        .members
        .values()
        .filter(|m| {
            m.guild_id == r.guild_id
                && (app.settings.adding_members || m.role_ids.contains(&r.id))
                && app.state.model.name(m.user.id).to_lowercase().contains(&q)
        })
        .collect();
    if members.is_empty() {
        c = c.push(small("No members found."));
    }
    for m in members {
        let assigned = m.role_ids.contains(&r.id);
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
                    disabled_tip(
                        secondary(
                            if assigned { "Remove" } else { "Add" },
                            A::Assign(m.user.id, !assigned),
                            allowed
                        ),
                        allowed,
                        LOCKED
                    )
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
