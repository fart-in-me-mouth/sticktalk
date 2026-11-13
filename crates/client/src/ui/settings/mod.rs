//! Full-screen settings. Kept separate from the main-window workstream.
mod channel;
mod fixtures;
mod roles;
mod server;
mod user;
mod widgets;
use super::{
    avatar, ellipsis, hint, icon_button, line, member_action, scroller, section, title, txt,
};
use crate::{
    app::{App, Dialog, GuildTab, Msg},
    focus_button::button,
    icons::icon,
    settings_controller::{
        Action as A, Number as N, RoleTab, TextField as F, Toggle as T, UserPage,
    },
    theme::{self as t, settings as st},
};
pub(crate) use fixtures::{configure, SCREENS};
use iced::{
    alignment::Alignment::Center,
    widget::{
        self, column, container, mouse_area, opaque, row, space, stack, text, text_input, tooltip,
        Column, Row,
    },
    Border, Element, Fill,
};
use sticktalk_client_core::settings::{Density, NoiseSuppressionMode, TriState};
use sticktalk_proto::*;
use widgets::*;

pub(crate) fn view<'a>(app: &'a App, page: &Dialog) -> Element<'a, Msg> {
    let channel = matches!(page, Dialog::ChannelSettings(_));
    let guild = matches!(page, Dialog::GuildSettings(_));
    let user_page = UserPage::from_index(app.user_tab);
    let label = match page {
        Dialog::GuildSettings(tab) => server::label(*tab),
        Dialog::ChannelSettings(_) => {
            if app.settings.channel_permissions {
                "Permissions"
            } else {
                "Overview"
            }
        }
        _ => user_page.label(),
    };
    let mut nav = Column::new().spacing(t::S4);
    if guild {
        nav = nav
            .push(section(
                app.state
                    .guild
                    .and_then(|g| app.state.model.guilds.get(&g))
                    .map_or("SERVER SETTINGS", |g| &g.guild.name),
            ))
            .push(space().height(t::S8));
    } else if channel {
        nav = nav
            .push(section(format!("# {}", app.settings.channel_name)))
            .push(space().height(t::S8));
    } else if let Some(id) = app.me() {
        nav = nav
            .push(
                button(
                    row![
                        avatar(app, id, t::AVATAR, true),
                        column![
                            label_text(ellipsis(&app.state.model.name(id), 20)),
                            row![
                                hint("Edit Profiles").size(t::HINT),
                                icon("pencil", t::HINT, t::TEXT_MUTED)
                            ]
                            .spacing(t::S4)
                            .align_y(Center)
                        ]
                        .spacing(t::S4)
                    ]
                    .spacing(t::S12)
                    .align_y(Center),
                )
                .on_press(Msg::Settings(A::User(UserPage::Profiles)))
                .padding(t::S8)
                .width(Fill)
                .style(t::icon),
            )
            .push(space().height(t::S8));
    }
    nav = nav
        .push(input(
            "Search",
            &app.settings.nav_search,
            F::NavSearch,
            true,
        ))
        .push(space().height(t::S8));
    let q = app.settings.nav_search.to_lowercase();
    if guild {
        if let Dialog::GuildSettings(tab) = page {
            nav = server::navigation(app, *tab, nav, &q);
        }
    } else if channel {
        nav = nav
            .push(nav_item(
                "hash",
                "Overview",
                A::ChannelTab(false),
                !app.settings.channel_permissions,
            ))
            .push(nav_item(
                "shield",
                "Permissions",
                A::ChannelTab(true),
                app.settings.channel_permissions,
            ));
    } else {
        let mut group = None;
        for (i, p) in UserPage::ALL.into_iter().enumerate() {
            let subs = [
                "Account Info",
                "Password & Security",
                "Sign-in Methods",
                "Account Status",
            ];
            let show = p.label().to_lowercase().contains(&q)
                || p == UserPage::Account && subs.iter().any(|s| s.to_lowercase().contains(&q));
            if !show {
                continue;
            }
            let app_group = i >= 5;
            if group != Some(app_group) {
                if app_group {
                    nav = nav.push(space().height(t::S16));
                }
                nav = nav.push(section(if app_group {
                    "APP SETTINGS"
                } else {
                    "USER SETTINGS"
                }));
                group = Some(app_group);
            }
            nav = nav.push(nav_item(p.icon(), p.label(), A::User(p), p == user_page));
            if p == UserPage::Account && user_page == p {
                let mut subnav = Column::new().spacing(0);
                for (i, s) in subs.iter().enumerate() {
                    subnav = subnav.push(
                        button(
                            row![
                                container(
                                    space()
                                        .width(if app.settings.account_section == i as u8 {
                                            t::FOCUS
                                        } else {
                                            t::HAIRLINE
                                        })
                                        .height(st::NAV_ROW)
                                )
                                .style(move |_| t::surface(
                                    if app.settings.account_section == i as u8 {
                                        st::WHITE
                                    } else {
                                        t::DIVIDER
                                    }
                                )),
                                text(*s).font(t::medium()).size(st::SMALL).color(
                                    if app.settings.account_section == i as u8 {
                                        t::TEXT_PRIMARY
                                    } else {
                                        t::TEXT_SECONDARY
                                    }
                                )
                            ]
                            .spacing(t::S16)
                            .align_y(Center),
                        )
                        .on_press(Msg::Settings(A::AccountSection(i as u8)))
                        .height(st::NAV_ROW)
                        .padding([0., t::S32])
                        .width(Fill)
                        .style(t::icon),
                    );
                }
                nav = nav.push(subnav);
            }
        }
    }
    let mut footer = Column::new().spacing(t::S8);
    if !guild && !channel {
        footer = footer
            .push(line())
            .push(
                button(
                    row![
                        label_text("Log Out").color(t::DANGER).width(Fill),
                        icon("log-out", st::NAV_ICON, t::DANGER)
                    ]
                    .align_y(Center),
                )
                .on_press(Msg::Logout)
                .height(st::NAV_ROW)
                .padding([t::S4, t::S8])
                .width(Fill)
                .style(t::danger_link),
            )
            .push(hint(format!("StickTalk {}", env!("CARGO_PKG_VERSION"))).size(t::HINT));
    }
    let left = container(column![scroller(nav).height(Fill), footer].spacing(t::S12))
        .padding([t::S24, t::S12])
        .width(st::NAV)
        .height(Fill)
        .style(t::sidebar);
    let content = if guild {
        server::content(app, page)
    } else if channel {
        channel::content(app)
    } else {
        user::content(app, user_page)
    };
    let mut body = Column::new().spacing(t::S16).push(content);
    if !app.error.is_empty() {
        body = body.push(hint(&app.error).color(t::DANGER));
    }
    if !app.notice.is_empty() {
        body = body.push(hint(&app.notice).color(t::ACCENT));
    }
    let wide = guild && matches!(page, Dialog::GuildSettings(GuildTab::Overview));
    let content_width = if wide {
        t::MODAL_W + st::PREVIEW + t::S32
    } else {
        st::CONTENT
    };
    let top = container(
        row![
            label_text(label).width(Fill),
            tooltip(
                button(icon("x", t::ICON, t::TEXT_SECONDARY))
                    .on_press(Msg::Close)
                    .padding(t::S8)
                    .width(t::INPUT)
                    .height(t::INPUT)
                    .style(t::close_circle),
                small("Close Settings (Esc)"),
                tooltip::Position::Top
            )
            .style(t::tooltip)
        ]
        .align_y(Center),
    )
    .padding([t::S4, t::S24])
    .height(t::HEADER)
    .style(t::chat);
    let dirty = match page {
        Dialog::UserSettings => app.settings.profile_dirty(),
        Dialog::GuildSettings(GuildTab::Overview) => app.settings.server_dirty(),
        Dialog::GuildSettings(GuildTab::Roles) => app.settings.role_dirty(),
        Dialog::ChannelSettings(_) => app.settings.overwrites != app.settings.overwrites_base,
        _ => false,
    };
    let mut right = column![
        top,
        line(),
        container(
            container(scroller(body).id("settings-content").height(Fill))
                .max_width(content_width)
                .width(Fill)
        )
        .padding([t::S24, t::S32])
        .center_x(Fill)
        .height(Fill)
    ]
    .width(Fill)
    .height(Fill);
    if dirty {
        right = right.push(
            container(
                container(
                    row![
                        label_text("Careful — you have unsaved changes!").width(Fill),
                        secondary("Reset", A::Reset, app.settings.saving.is_empty()).style(t::link),
                        primary("Save Changes", A::Save, app.settings.saving.is_empty())
                    ]
                    .spacing(t::S16)
                    .align_y(Center),
                )
                .padding(t::S12)
                .style(|_| {
                    let mut s = t::surface(st::MENU);
                    s.border.radius = t::S8.into();
                    s
                }),
            )
            .max_width(content_width)
            .padding([t::S8, t::S32])
            .center_x(Fill),
        );
    }
    let main: Element<'a, Msg> = container(row![left, right])
        .id("dialog-scope")
        .height(Fill)
        .width(Fill)
        .style(t::chat)
        .into();
    if app.settings.delete_open {
        stack![
            main,
            container(opaque(server::delete_modal(app)))
                .center_x(Fill)
                .center_y(Fill)
                .width(Fill)
                .height(Fill)
                .style(t::dim)
        ]
        .into()
    } else {
        main
    }
}

pub(crate) fn notifications(app: &App) -> Element<'_, Msg> {
    let Some(s) = &app.settings.notification else {
        return hint("Loading notification settings…").into();
    };
    let mut c = column![
        row![
            title("Notification Settings").width(Fill),
            icon_button("x", "Close", Msg::Close)
        ]
        .align_y(Center),
        hint(
            app.state
                .model
                .guilds
                .get(&s.guild_id.unwrap())
                .map_or("Server", |g| &g.guild.name)
        ),
        line(),
        label_text("Server Notification Settings")
    ]
    .spacing(t::S16);
    for (label, level) in [
        ("All Messages", NotificationLevel::All),
        ("Only @mentions", NotificationLevel::Mentions),
        ("Nothing", NotificationLevel::Nothing),
    ] {
        c = c.push(choice(label, s.level == level, A::Level(level)));
    }
    c = c
        .push(line())
        .push(toggle(
            "Suppress @everyone and @here",
            "",
            s.suppress_everyone,
            A::Toggle(T::SuppressEveryone, !s.suppress_everyone),
            true,
        ))
        .push(toggle(
            "Mute Server",
            "Unread indicators and notifications are muted.",
            s.muted,
            A::Toggle(T::Muted, !s.muted),
            true,
        ));
    if s.muted {
        let durations = [
            ("For 15 minutes", 900_000),
            ("For 1 hour", 3_600_000),
            ("For 8 hours", 28_800_000),
            ("For 24 hours", 86_400_000),
            ("Until I turn it back on", 0),
        ];
        let selected = durations
            .iter()
            .find(|(_, v)| *v == app.settings.mute_duration_ms)
            .map(|(s, _)| s.to_string());
        c = c.push(
            widget::pick_list(
                durations
                    .iter()
                    .map(|(s, _)| s.to_string())
                    .collect::<Vec<_>>(),
                selected,
                move |s| {
                    Msg::Settings(A::MuteDuration(
                        durations.iter().find(|(l, _)| *l == s).unwrap().1,
                    ))
                },
            )
            .padding(t::S12)
            .style(st::pick_list)
            .menu_style(st::menu),
        );
    }
    container(c.push(row![
        space().width(Fill),
        primary("Done", A::SaveNotifications, true)
    ]))
    .width(t::MODAL_W)
    .padding(t::S24)
    .style(t::modal)
    .into()
}
