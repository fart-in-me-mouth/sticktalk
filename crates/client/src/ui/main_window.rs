use super::*;
use crate::main_window::{mm, Message as Main, Popover};
use iced::widget::column;
pub fn header<'a>(app: &'a App, c: &'a ChannelSummary) -> Element<'a, Msg> {
    let topic = app
        .state
        .model
        .topics
        .get(&c.id)
        .and_then(|s| s.as_deref())
        .unwrap_or("");
    let members = app
        .members_open
        .unwrap_or(app.width >= t::MEMBER_BREAKPOINT);
    let narrow = app.width < t::DEFAULT_W;
    let search_width = if narrow && !app.main.search_focused {
        t::SEARCH_REST
    } else {
        t::SEARCH_W
    };
    let search_hint = c
        .guild_id
        .and_then(|id| app.state.model.guilds.get(&id))
        .map_or_else(
            || "Search messages".into(),
            |g| format!("Search {}", g.guild.name),
        );
    let search = text_input(&search_hint, &app.main.search)
        .id("header-search")
        .on_input(|s| mm(Main::Search(s)))
        .padding([t::S4, t::S8])
        .font(t::regular())
        .size(t::USER_NAME)
        .style(t::input)
        .width(Fill);
    let muted = crate::main_window::muted(app, c.id);
    container(
        row![
            icon(
                if c.kind == ChannelKind::Text {
                    "hash"
                } else if c.kind == ChannelKind::Voice {
                    "volume-2"
                } else {
                    "at-sign"
                },
                t::ICON,
                t::TEXT_MUTED
            ),
            container(
                elided::label(app.state.model.channel_name(c))
                    .size(t::CHANNEL_NAME)
                    .font(t::semibold())
            )
            .max_width(
                ((app.width - t::RAIL - t::SIDEBAR - search_width - t::S32 * 6.) / 2.)
                    .clamp(t::S32, t::SEARCH_W)
            ),
            container(rule::vertical(t::HAIRLINE).style(t::divider)).height(t::S24),
            button(
                elided::label(topic)
                    .width(Fill)
                    .size(t::USER_NAME)
                    .color(t::TEXT_MUTED)
            )
            .on_press_maybe((!topic.is_empty()).then_some(mm(Main::Open(Popover::Topic(c.id)))))
            .padding(0)
            .width(Fill)
            .style(t::icon),
            controls::icon_button(
                if muted { "bell-off" } else { "bell" },
                "Notification Settings",
                mm(Main::Open(Popover::Notifications(c.guild_id, Some(c.id)))),
                false
            ),
            controls::icon_button(
                "pin",
                "Pinned Messages",
                mm(Main::Open(Popover::Pins(c.id))),
                app.main.popover == Some(Popover::Pins(c.id))
            ),
            controls::icon_button("users", "Member List", Msg::ToggleMembers, members),
            container(
                row![
                    observe::focus(search),
                    icon("search", t::INLINE_ICON, t::TEXT_MUTED)
                ]
                .spacing(t::S4)
                .align_y(Center)
            )
            .width(search_width)
            .height(t::VOICE_USER_ROW)
            .padding(Padding {
                right: t::S8,
                ..Padding::ZERO
            })
            .style(|_| container::Style {
                border: Border {
                    radius: t::TOOLTIP_RADIUS.into(),
                    ..Default::default()
                },
                ..t::surface(t::INPUT_BG)
            })
        ]
        .spacing(t::S8)
        .align_y(Center),
    )
    .padding([t::S8, t::S16])
    .height(t::HEADER)
    .style(t::chat)
    .into()
}
pub fn overlay(app: &App) -> Option<Element<'_, Msg>> {
    let p = app.main.popover.as_ref()?;
    let body = match p {
        Popover::Server => menus::server(app),
        Popover::Profile(id) => popout::profile(app, *id),
        Popover::Pins(id) => popout::pins(app, *id),
        Popover::Notifications(g, c) => menus::notifications(app, *g, *c),
        Popover::Topic(id) => container(
            column![
                txt("Channel Topic").font(t::semibold()),
                txt(app
                    .state
                    .model
                    .topics
                    .get(id)
                    .and_then(|s| s.as_deref())
                    .unwrap_or("No topic set."))
            ]
            .spacing(t::S12),
        )
        .width(t::POPOUT_W)
        .padding(t::S16)
        .style(controls::surface)
        .into(),
        Popover::Quick => popout::quick(app),
        Popover::Devices(mic) => menus::devices(*mic),
        Popover::Context(e) => menus::entity(app, e),
        Popover::Mutual(id, s) => popout::mutual(app, *id, *s),
        Popover::Roles(id) => popout::roles(app, *id),
        Popover::Nickname(id) => popout::nickname(app, *id),
    };
    let size = if app.main.popover_size.width > 0. {
        app.main.popover_size
    } else {
        iced::Size::new(
            if matches!(p, Popover::Quick) {
                t::QUICK_W
            } else {
                t::POPOUT_W
            },
            t::S32 * 10.,
        )
    };
    let desired = match p {
        Popover::Server => iced::Point::new(t::RAIL + t::S8, t::HEADER + t::S4),
        Popover::Notifications(_, Some(_)) | Popover::Topic(_) => {
            iced::Point::new(app.main.position.x, t::HEADER + t::S8)
        }
        Popover::Pins(_) => iced::Point::new(
            app.width - t::MEMBERS - t::POPOUT_W - t::S32,
            t::HEADER + t::S8,
        ),
        Popover::Quick => iced::Point::new((app.width - t::QUICK_W) / 2., t::HEADER + t::S32),
        Popover::Devices(_) => {
            iced::Point::new(t::RAIL + t::S8, app.height - t::USER_HEIGHT - size.height)
        }
        Popover::Profile(_) | Popover::Mutual(_, _) => {
            let anchor = app
                .main
                .anchor
                .unwrap_or(iced::Rectangle::new(app.main.position, iced::Size::ZERO));
            let x = if anchor.x >= app.width - t::MEMBERS {
                anchor.x.min(app.width - t::MEMBERS) - size.width - t::S8
            } else {
                anchor.x + anchor.width + t::S8
            };
            iced::Point::new(x, anchor.y)
        }
        _ => app.main.position,
    };
    let avoid = if matches!(p, Popover::Context(_)) {
        app.main.anchor
    } else {
        None
    };
    let body = widget::sensor(
        container(scroller(body).height(Length::Shrink))
            .id("main-popover")
            .max_height(app.height - t::S16),
    )
    .key(app.main.measurement_revision)
    .on_show(|size| mm(Main::PopoverSize(size)))
    .on_resize(|size| mm(Main::PopoverSize(size)));
    // Measure against the whole window before translating. Padding/pin would
    // constrain the card to the remaining height and hide its true extent.
    let overlay = container(observe::popup(opaque(body), desired, avoid))
        .id("dialog-scope")
        .width(Fill)
        .height(Fill);
    Some(
        stack![
            mouse_area(container(space()).width(Fill).height(Fill).style(
                if matches!(p, Popover::Quick) {
                    t::dim
                } else {
                    t::bare
                }
            ))
            .on_press(mm(Main::Close))
            .on_right_press(mm(Main::Close)),
            overlay
        ]
        .into(),
    )
}
pub fn drop_overlay<'a>() -> Element<'a, Msg> {
    container(
        container(
            column![
                icon("file", t::S32, t::ACCENT),
                txt("Drop files to upload")
                    .font(t::semibold())
                    .size(t::TITLE),
                hint("Up to 4 files · 25 MB each")
            ]
            .spacing(t::S16)
            .align_x(Center),
        )
        .padding(t::S32)
        .style(t::elevated),
    )
    .center_x(Fill)
    .center_y(Fill)
    .style(t::dim)
    .into()
}
pub fn local_results(app: &App, id: Id) -> Element<'_, Msg> {
    let query = app.main.search.to_lowercase();
    let results: Vec<_> = app
        .state
        .model
        .messages
        .get(&id)
        .into_iter()
        .flat_map(|ms| ms.values())
        .filter(|m| app.state.model.content(m).to_lowercase().contains(&query))
        .take(100)
        .collect();
    let mut list = column![
        section(format!("Search results — {}", results.len())),
        hint("Messages loaded in this conversation.").color(t::TEXT_SECONDARY)
    ]
    .spacing(t::S12);
    if results.is_empty() {
        list = list.push(
            container(
                column![
                    icon("search", t::AVATAR, t::TEXT_SECONDARY),
                    txt("No messages found"),
                    hint("Try a different word.").color(t::TEXT_SECONDARY),
                    action("Clear search", mm(Main::Search(String::new())))
                ]
                .spacing(t::S16)
                .align_x(Center),
            )
            .center_x(Fill)
            .padding(t::S32),
        );
    }
    for m in results {
        list = list.push(
            button(
                column![
                    txt(app.state.model.name(m.author_id))
                        .color(controls::name_color(app, m.author_id))
                        .font(t::medium()),
                    txt(ellipsis(&app.state.model.content(m), 200)),
                    hint(timestamp(m.created_at, false))
                ]
                .spacing(t::S4),
            )
            .on_press(Msg::JumpTo(m.id))
            .padding(t::S12)
            .width(Fill)
            .style(t::icon),
        );
    }
    container(scroller(list).height(Fill))
        .padding(t::S16)
        .height(Fill)
        .into()
}
