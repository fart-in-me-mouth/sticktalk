use super::*;
use iced::widget::column;
pub(super) fn content(app: &App) -> Element<'_, Msg> {
    let Some(id) = app.settings.channel else {
        return hint("Loading channel…").into();
    };
    let effective = app
        .state
        .model
        .channels
        .get(&id)
        .and_then(|ch| {
            app.state.model.guilds.get(&ch.guild_id?).map(|g| {
                app.state.model.settings.channel_permissions(
                    &g.guild,
                    app.me().unwrap_or(Id(0)),
                    ch.parent_id,
                    id,
                )
            })
        })
        .unwrap_or_default();
    let allowed = effective.contains(if app.settings.channel_permissions {
        Permissions::MANAGE_ROLES
    } else {
        Permissions::MANAGE_CHANNELS
    }) && app.settings.saving.is_empty();
    if !app.settings.channel_permissions {
        return column![
            title("Overview"),
            field(
                "CHANNEL NAME",
                "Channel name",
                &app.settings.channel_name,
                F::ChannelName,
                allowed
            ),
            field(
                "CHANNEL TOPIC",
                "Let everyone know how to use this channel",
                &app.settings.channel_topic,
                F::Topic,
                allowed
            ),
            line(),
            section("SLOWMODE"),
            small("Slowmode is not available yet."),
            primary("Save Changes", A::Save, allowed)
        ]
        .spacing(t::S24)
        .into();
    }
    let gid = app
        .state
        .model
        .channels
        .get(&id)
        .and_then(|c| c.guild_id)
        .unwrap_or(Id(0));
    let private = app.settings.overwrites.iter().any(|o| {
        o.kind == OverwriteKind::Role
            && o.target_id == gid
            && o.deny.contains(Permissions::VIEW_CHANNEL)
    });
    let mut c = column![
        title("Permissions"),
        toggle(
            "Private Channel",
            "Only selected members and roles will be able to view this channel.",
            private,
            A::Toggle(T::Private, !private),
            allowed
        ),
        line(),
        label_text("Roles / Members")
    ]
    .spacing(t::S24);
    let mut targets = Column::new().spacing(t::S8);
    for o in &app.settings.overwrites {
        let name = if o.kind == OverwriteKind::Role {
            app.state
                .model
                .settings
                .roles
                .get(&o.target_id)
                .map_or("Deleted Role".into(), |r| r.name.clone())
        } else {
            app.state.model.name(o.target_id)
        };
        targets = targets.push(
            secondary(&name, A::OverwriteTarget(o.kind, o.target_id), true)
                .width(Fill)
                .style(t::navigation(
                    app.settings.overwrite_target == Some((o.kind, o.target_id)),
                )),
        );
    }
    let role_options: Vec<_> = app
        .state
        .model
        .settings
        .roles
        .values()
        .filter(|r| {
            r.guild_id == gid
                && !app
                    .settings
                    .overwrites
                    .iter()
                    .any(|o| o.kind == OverwriteKind::Role && o.target_id == r.id)
        })
        .map(|r| (format!("{} · {}", r.name, r.id.0), r.id))
        .collect();
    let member_options: Vec<_> = app
        .state
        .model
        .members
        .values()
        .filter(|m| {
            m.guild_id == gid
                && !app
                    .settings
                    .overwrites
                    .iter()
                    .any(|o| o.kind == OverwriteKind::Member && o.target_id == m.user.id)
        })
        .map(|m| {
            (
                format!("{} · {}", app.state.model.name(m.user.id), m.user.id.0),
                m.user.id,
            )
        })
        .collect();
    if allowed {
        targets = targets.push(
            widget::pick_list(
                role_options
                    .iter()
                    .map(|(s, _)| s.clone())
                    .collect::<Vec<_>>(),
                None::<String>,
                move |s| {
                    Msg::Settings(A::AddOverwrite(
                        OverwriteKind::Role,
                        role_options.iter().find(|(n, _)| *n == s).unwrap().1,
                    ))
                },
            )
            .placeholder("Add a role")
            .width(Fill)
            .padding(t::S12)
            .style(st::pick_list)
            .menu_style(st::menu),
        );
        targets = targets.push(
            widget::pick_list(
                member_options
                    .iter()
                    .map(|(s, _)| s.clone())
                    .collect::<Vec<_>>(),
                None::<String>,
                move |s| {
                    Msg::Settings(A::AddOverwrite(
                        OverwriteKind::Member,
                        member_options.iter().find(|(n, _)| *n == s).unwrap().1,
                    ))
                },
            )
            .placeholder("Add a member")
            .width(Fill)
            .padding(t::S12)
            .style(st::pick_list)
            .menu_style(st::menu),
        );
    }
    let mut perms = column![
        small("Deny overrides this scope, neutral inherits, and allow grants the permission."),
        row![space().width(Fill), hint("DENY   INHERIT   ALLOW")],
        line()
    ]
    .spacing(t::S16);
    if let Some((kind, target)) = app.settings.overwrite_target {
        if let Some(o) = app
            .settings
            .overwrites
            .iter()
            .find(|o| o.kind == kind && o.target_id == target)
        {
            for (_, bit, name, desc) in roles::PERMISSIONS
                .iter()
                .filter(|(_, bit, _, _)| *bit != Permissions::ADMINISTRATOR)
            {
                let state = if o.allow.contains(*bit) {
                    TriState::Allow
                } else if o.deny.contains(*bit) {
                    TriState::Deny
                } else {
                    TriState::Neutral
                };
                let enabled = allowed && effective.contains(*bit);
                let mut values = Row::new().spacing(t::S4);
                for (label, v, col) in [
                    ("x", TriState::Deny, t::DANGER),
                    ("minus", TriState::Neutral, t::TEXT_SECONDARY),
                    ("check", TriState::Allow, t::ACCENT),
                ] {
                    let symbol: Element<'_, Msg> = icon(
                        label,
                        t::INLINE_ICON,
                        if enabled { col } else { t::alpha(col, 0.4) },
                    )
                    .into();
                    values = values.push(disabled_tip(
                        button(symbol)
                            .on_press_maybe(
                                enabled.then_some(Msg::Settings(A::OverwritePermission(*bit, v))),
                            )
                            .width(t::S32)
                            .height(t::S32)
                            .padding(t::S8)
                            .style(t::navigation(state == v)),
                        enabled,
                        LOCKED,
                    ));
                }
                perms = perms
                    .push(
                        row![
                            column![label_text(*name), small(*desc)]
                                .spacing(t::S8)
                                .width(Fill),
                            values
                        ]
                        .spacing(t::S16)
                        .align_y(Center),
                    )
                    .push(line());
            }
            perms = perms.push(
                secondary("Remove Overwrite", A::RemoveOverwrite, allowed).style(t::danger_link),
            );
        }
    } else {
        perms = perms.push(small("Choose a role or member to edit their permissions."));
    }
    c = c.push(
        row![
            container(targets).width(t::SIDEBAR - t::S32),
            container(perms).width(Fill)
        ]
        .spacing(t::S24),
    );
    c.into()
}
