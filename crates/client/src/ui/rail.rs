use super::*;
use crate::main_window::{mm, Entity, Message as Main, Popover};
use iced::widget::column;
pub fn view(app: &App) -> Element<'_, Msg> {
    let mut list = column![
        cell(app, None, "Home"),
        container(line()).width(t::S32).padding([t::S8, 0.])
    ]
    .spacing(t::S8)
    .align_x(Center);
    for g in app.state.model.guilds.values() {
        list = list.push(cell(app, Some(g.guild.id), &g.guild.name));
    }
    list = list.push(controls::tip(
        button(icon("plus", t::ICON, t::ACCENT))
            .on_press(Msg::Open(Dialog::AddGuild))
            .width(t::GUILD_ICON)
            .height(t::GUILD_ICON)
            .padding(t::S12)
            .style(t::icon),
        "Add a Server",
        tooltip::Position::Right,
    ));
    mouse_area(
        container(scroller(list).height(Fill))
            .padding([t::S12, 0.])
            .width(t::RAIL)
            .height(Fill)
            .style(t::rail),
    )
    .on_right_press(mm(Main::Open(Popover::Context(Entity::Rail))))
    .into()
}
fn cell<'a>(app: &App, id: Option<Id>, name: &str) -> Element<'a, Msg> {
    let selected = app.state.guild == id;
    let hovered = app.hovered_guild == Some(id);
    let radius = if selected || hovered {
        t::GUILD_RADIUS
    } else {
        t::ROUND
    };
    let glyph: Element<'a, Msg> = if id.is_none() {
        icon("message-circle", t::ICON, t::ACCENT).into()
    } else if let Some(h) = id
        .and_then(|g| app.state.model.guilds.get(&g))
        .and_then(|g| g.guild.icon_id)
        .and_then(|i| app.images.get(i))
    {
        image(h.clone())
            .width(t::GUILD_ICON)
            .height(t::GUILD_ICON)
            .border_radius(radius)
            .into()
    } else {
        txt(name
            .split_whitespace()
            .take(2)
            .filter_map(|s| s.chars().next())
            .collect::<String>())
        .font(t::semibold())
        .into()
    };
    let btn = button(
        container(glyph)
            .center_x(t::GUILD_ICON)
            .center_y(t::GUILD_ICON),
    )
    .on_press(Msg::Guild(id))
    .padding(0)
    .style(move |theme, status| {
        let mut s = t::icon(theme, status);
        s.background = Some(
            if selected {
                t::BG_SELECTED
            } else {
                t::BG_ELEVATED
            }
            .into(),
        );
        s.border.radius = radius.into();
        s
    });
    let mut unread = false;
    let mut mentions = 0u32;
    if let Some(g) = id {
        for c in app
            .state
            .model
            .channels
            .values()
            .filter(|c| c.guild_id == Some(g))
        {
            if let Some(r) = app.state.model.reads.get(&c.id) {
                unread |= r.unread_count > 0;
                mentions = mentions.saturating_add(r.mention_count);
            }
        }
    }
    let marker = container(space())
        .width(t::S4)
        .height(if selected {
            t::INPUT
        } else if hovered {
            t::ICON
        } else {
            t::S8
        })
        .style(move |_| container::Style {
            border: Border {
                radius: t::S4.into(),
                ..Default::default()
            },
            ..t::surface(if selected || hovered || unread {
                t::TEXT_PRIMARY
            } else {
                t::TRANSPARENT
            })
        });
    let mut layers = vec![
        container(btn).center_x(t::RAIL).into(),
        container(marker).center_y(t::GUILD_ICON).into(),
    ];
    if mentions > 0 {
        layers.push(
            container(
                container(
                    hint(if mentions > 99 {
                        "99+".into()
                    } else {
                        mentions.to_string()
                    })
                    .font(t::semibold())
                    .color(t::TEXT_PRIMARY),
                )
                .padding([0., t::S4])
                .style(|_| container::Style {
                    border: Border {
                        radius: t::ROUND.into(),
                        width: 3.,
                        color: t::BG_RAIL,
                    },
                    ..t::surface(t::DANGER)
                }),
            )
            .align_right(t::RAIL - t::S8)
            .align_bottom(t::GUILD_ICON)
            .into(),
        );
    }
    controls::tip(
        mouse_area(stack(layers))
            .on_enter(Msg::GuildHover(Some(id)))
            .on_exit(Msg::GuildHover(None))
            .on_right_press(mm(Main::Open(Popover::Context(
                id.map_or(Entity::Rail, Entity::Guild),
            )))),
        name,
        tooltip::Position::Right,
    )
}
