//! Full-screen settings. Kept separate from the main-window workstream.
mod channel;
mod channels;
mod fixtures;
mod roles;
mod server;
mod user;
mod voice;
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
use sticktalk_client_core::settings::{Density, TriState};
use sticktalk_proto::*;
use widgets::*;

pub(crate) fn view<'a>(app: &'a App, page: &Dialog) -> Element<'a, Msg> {
    let channel = matches!(page, Dialog::ChannelSettings(_));
    let guild = matches!(page, Dialog::GuildSettings(_));
    let user_page = UserPage::from_index(app.user_tab);
    let mut nav = Column::new().spacing(t::S4);
    if guild {
        nav = nav
            .push(
                super::elided::label(
                    app.state
                        .guild
                        .and_then(|g| app.state.model.guilds.get(&g))
                        .map_or("SERVER SETTINGS", |g| &g.guild.name),
                )
                .size(t::HINT)
                .font(t::semibold())
                .color(t::TEXT_MUTED)
                .width(Fill),
            )
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
                            super::elided::label(app.state.model.name(id))
                                .size(st::LABEL)
                                .font(t::semibold())
                                .width(Fill),
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
            let show = p.label().to_lowercase().contains(&q)
                || p == UserPage::Account
                    && "account info password & security sign-in methods account status"
                        .contains(&q);
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
    // The sidebar fills the left region; only its inner navigation has a fixed width.
    let (left_width, content_width) = st::columns(app.width);
    let left = container(
        container(column![scroller(nav).id("settings-nav").height(Fill), footer].spacing(t::S12))
            .padding(iced::Padding {
                top: st::TOP,
                right: t::S8,
                bottom: t::S24,
                left: t::S8,
            })
            .width(st::NAV)
            .height(Fill),
    )
    .align_right(left_width)
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
    let dirty = app.settings_dirty();
    // Close is outside the scrolling tree, beside the content rather than the window edge.
    let close = container(
        column![
            tooltip(
                button(icon("x", t::ICON, t::TEXT_SECONDARY))
                    .on_press(Msg::Close)
                    .padding(t::S8)
                    .width(t::INPUT)
                    .height(t::INPUT)
                    .style(t::close_circle),
                settings_tooltip("Close Settings"),
                tooltip::Position::Top
            )
            .delay(std::time::Duration::from_millis(400))
            .style(t::bare),
            hint("ESC")
        ]
        .spacing(t::S8)
        .align_x(Center),
    )
    .padding(iced::Padding {
        top: st::TOP,
        left: t::S24,
        ..iced::Padding::ZERO
    })
    .width(st::CLOSE_GUTTER);
    let scroll = scroller(body.padding(iced::Padding {
        top: st::TOP,
        bottom: if dirty {
            t::INPUT + t::S32 * 2.
        } else {
            st::TOP
        },
        ..iced::Padding::ZERO
    }))
    .id("settings-content")
    .height(Fill);
    let content: Element<'a, Msg> = if dirty {
        let shake = match app.settings.shake_step {
            1 | 3 | 5 => t::S8,
            2 | 4 => -t::S8,
            _ => 0.,
        };
        let bar = container(
            row![
                small("Careful — you have unsaved changes!")
                    .color(t::TEXT_PRIMARY)
                    .width(Fill),
                secondary("Reset", A::Reset, app.settings.saving.is_empty()).style(t::link),
                primary(
                    "Save Changes",
                    if matches!(app.dialog, Dialog::UserSettings)
                        && user_page == UserPage::Account
                        && app.settings.edit_account
                    {
                        A::SaveAccount
                    } else {
                        A::Save
                    },
                    app.settings.saving.is_empty()
                )
            ]
            .spacing(t::S8)
            .align_y(Center),
        )
        .padding(t::S12)
        .width(Fill)
        .style(st::menu_surface);
        stack![
            scroll,
            container(widget::float(bar).translate(move |_, _| iced::Vector::new(shake, 0.)))
                .padding(iced::Padding {
                    bottom: t::S24,
                    ..iced::Padding::ZERO
                })
                .align_bottom(Fill)
                .width(Fill)
        ]
        .into()
    } else {
        scroll.into()
    };
    let right = row![
        container(content).width(content_width).height(Fill),
        close,
        space().width(Fill)
    ]
    .height(Fill)
    .width(Fill);
    let main: Element<'a, Msg> = container(row![left, space().width(st::CONTENT_GAP), right])
        .id(
            if !app.settings.delete_open
                && matches!(
                    app.dialog,
                    Dialog::UserSettings | Dialog::GuildSettings(_) | Dialog::ChannelSettings(_)
                )
            {
                "dialog-scope"
            } else {
                "settings-underlay"
            },
        )
        .height(Fill)
        .width(Fill)
        .style(t::chat)
        .into();
    if app.settings.delete_open {
        stack![
            main,
            opaque(
                container(server::delete_modal(app))
                    .id("dialog-scope")
                    .center_x(Fill)
                    .center_y(Fill)
                    .width(Fill)
                    .height(Fill)
                    .style(t::dim)
            )
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
    let header = column![
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
        line()
    ]
    .spacing(t::S16);
    let mut c =
        column![label_text("Server Notification Settings")].spacing(if app.height < t::DEFAULT_H {
            // Keep the complete mute-duration row visible at the minimum size.
            // The fixed footer must not bisect a native dropdown's text.
            t::S4
        } else {
            t::S16
        });
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
    let footer = container(row![
        space().width(Fill),
        primary("Done", A::SaveNotifications, true)
    ])
    .id("notification-footer");
    container(
        column![
            header,
            container(scroller(c).height(iced::Length::Shrink)).max_height(
                (app.height - t::S32 * 2. - t::HEADER - t::S24 * 4. - t::BUTTON)
                    .max(t::ATTACHMENT_CARD)
            ),
            footer
        ]
        .spacing(t::S16),
    )
    .width(t::MODAL_W)
    .padding(t::S24)
    .style(t::modal)
    .into()
}
