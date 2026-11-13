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
    let search = text_input(
        &format!(
            "Search {}",
            app.state
                .guild
                .and_then(|g| app.state.model.guilds.get(&g))
                .map_or("messages", |g| g.guild.name.as_str())
        ),
        &app.main.search,
    )
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
            txt(ellipsis(&app.state.model.channel_name(c), 18))
                .size(t::CHANNEL_NAME)
                .font(t::semibold()),
            container(rule::vertical(t::HAIRLINE).style(t::divider)).height(t::S24),
            button(
                txt(ellipsis(topic, if narrow { 16 } else { 60 }))
                    .size(t::USER_NAME)
                    .color(t::TEXT_MUTED)
            )
            .on_press(mm(Main::Open(Popover::Topic(c.id))))
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
                    search,
                    hint("Local"),
                    icon("search", t::INLINE_ICON, t::TEXT_MUTED)
                ]
                .spacing(t::S4)
                .align_y(Center)
            )
            .width(if narrow {
                t::SEARCH_NARROW
            } else {
                t::SEARCH_W
            })
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
    let (x, y) = match p {
        Popover::Server => (t::RAIL + t::S8, t::HEADER + t::S4),
        Popover::Notifications(_, Some(_)) | Popover::Topic(_) => {
            (app.main.position.x, t::HEADER + t::S8)
        }
        Popover::Pins(_) => (
            app.width - t::MEMBERS - t::POPOUT_W - t::S32,
            t::HEADER + t::S8,
        ),
        Popover::Quick => ((app.width - t::QUICK_W) / 2., t::HEADER + t::S32),
        Popover::Devices(_) => (t::RAIL + t::S8, app.height - t::USER_HEIGHT - t::S24 * 7.),
        Popover::Profile(_) | Popover::Mutual(_, _) => {
            let x = if app.main.position.x >= app.width - t::MEMBERS {
                app.width - t::MEMBERS - t::POPOUT_W - t::S8
            } else {
                app.main.position.x + t::S16
            };
            (
                x,
                app.main
                    .position
                    .y
                    .min((app.height - t::BANNER_H - t::S32 * 16.).max(t::S8)),
            )
        }
        _ => (app.main.position.x, app.main.position.y),
    };
    let width = if matches!(p, Popover::Quick) {
        t::QUICK_W
    } else if matches!(
        p,
        Popover::Server
            | Popover::Context(_)
            | Popover::Devices(_)
            | Popover::Roles(_)
            | Popover::Notifications(..)
    ) {
        t::CONTEXT_W
    } else {
        t::POPOUT_W + t::S32
    };
    let height = if matches!(p, Popover::Server) {
        t::ROW * 9. + t::S32 * 2.
    } else {
        t::S32 * 10.
    };
    let overlay = container(opaque(body))
        .padding(Padding {
            left: x.clamp(t::S8, (app.width - width - t::S8).max(t::S8)),
            top: y.clamp(t::S8, (app.height - height - t::S8).max(t::S8)),
            ..Padding::ZERO
        })
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
    let mut list = column![hint("Local search · loaded messages only")].spacing(t::S12);
    for m in app
        .state
        .model
        .messages
        .get(&id)
        .into_iter()
        .flat_map(|ms| ms.values())
        .filter(|m| app.state.model.content(m).to_lowercase().contains(&query))
        .take(100)
    {
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
