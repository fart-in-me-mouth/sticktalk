use super::*;
use crate::main_window::{mm, Entity, Message as Main, Popover};
use iced::widget::column;
pub fn view(app: &App) -> Element<'_, Msg> {
    let home = column![
        cell(app, None, "Home"),
        container(line()).width(t::S32).padding([t::S8, 0.])
    ]
    .spacing(t::S8)
    .align_x(Center);
    let mut list = Column::new().spacing(t::S8).align_x(Center);
    for f in &app.state.model.launch.server_folders {
        list = list.push(folder(app, f));
    }
    for g in app.state.model.guilds.values() {
        if !app
            .state
            .model
            .launch
            .server_folders
            .iter()
            .any(|f| f.guild_ids.contains(&g.guild.id))
        {
            list = list.push(cell(app, Some(g.guild.id), &g.guild.name));
        }
    }
    let add = controls::tip(
        button(crate::icons::inherited("plus", t::ICON))
            .on_press(Msg::Open(Dialog::AddGuild))
            .width(t::GUILD_ICON)
            .height(t::GUILD_ICON)
            .padding(t::S12)
            .style(|theme, status| {
                let mut style = t::icon(theme, status);
                let active = matches!(
                    status,
                    widget::button::Status::Hovered | widget::button::Status::Pressed
                );
                style.border.radius = if active { t::GUILD_RADIUS } else { t::ROUND }.into();
                style.background = Some(if active { t::ACCENT } else { t::BG_ELEVATED }.into());
                style.text_color = if active { t::ACCENT_TEXT } else { t::ACCENT };
                style
            }),
        "Add a Server",
        tooltip::Position::Right,
    );
    mouse_area(
        container(
            column![home, scroller(list).id("server-rail").height(Fill), add]
                .spacing(t::S8)
                .align_x(Center),
        )
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
        let focused = matches!(
            status,
            widget::button::Status::Hovered | widget::button::Status::Pressed
        );
        s.background = Some(
            if selected || focused {
                t::BG_SELECTED
            } else {
                t::BG_ELEVATED
            }
            .into(),
        );
        s.border.radius = if selected || hovered || focused {
            t::GUILD_RADIUS
        } else {
            t::ROUND
        }
        .into();
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
        let label = mention_label(mentions);
        let width = t::BADGE_H.max(label.len() as f32 * t::HINT * 0.65 + t::S8);
        layers.push(
            container(
                container(
                    container(
                        hint(label)
                            .font(t::bold())
                            .color(t::BADGE_TEXT)
                            .line_height(1.)
                            .wrapping(widget::text::Wrapping::None),
                    )
                    .center_x(width)
                    .center_y(t::BADGE_H)
                    .style(|_| container::Style {
                        border: Border {
                            radius: t::ROUND.into(),
                            ..Default::default()
                        },
                        ..t::surface(t::DANGER_BUTTON)
                    }),
                )
                .padding(t::BADGE_RING)
                .style(|_| container::Style {
                    border: Border {
                        radius: t::ROUND.into(),
                        ..Default::default()
                    },
                    ..t::surface(t::BG_RAIL)
                }),
            )
            .align_right(t::RAIL - t::S8)
            .align_bottom(t::GUILD_ICON)
            .into(),
        );
    }
    controls::tip(
        observe::anchor(
            mouse_area(stack(layers))
                .on_enter(Msg::GuildHover(Some(id)))
                .on_exit(mm(Main::RailLeave(id)))
                .on_right_press(mm(Main::Open(Popover::Context(
                    id.map_or(Entity::Rail, Entity::Guild),
                )))),
        ),
        name,
        tooltip::Position::Right,
    )
}

pub(super) fn mention_label(count: u32) -> String {
    if count > 99 {
        "99+".into()
    } else {
        count.to_string()
    }
}
fn folder<'a>(app: &App, f: &ServerFolder) -> Element<'a, Msg> {
    use crate::features3::{msg, Event};
    let id = f.id;
    let color = controls::rgb(f.color);
    let mut mentions = 0u32;
    let mut unread = false;
    for c in app
        .state
        .model
        .channels
        .values()
        .filter(|c| c.guild_id.is_some_and(|g| f.guild_ids.contains(&g)))
    {
        if let Some(r) = app.state.model.reads.get(&c.id) {
            mentions = mentions.saturating_add(r.mention_count);
            unread |= r.unread_count > 0;
        }
    }
    let selected = f.collapsed && app.state.guild.is_some_and(|g| f.guild_ids.contains(&g));
    let glyph: Element<'a, Msg> = if f.collapsed {
        let mut grid = column![].spacing(t::S4);
        for chunk in f
            .guild_ids
            .iter()
            .take(4)
            .copied()
            .collect::<Vec<_>>()
            .chunks(2)
        {
            let mut row = Row::new().spacing(t::S4);
            for guild in chunk {
                let face: Element<'a, Msg> = if let Some(handle) = app
                    .state
                    .model
                    .guilds
                    .get(guild)
                    .and_then(|g| g.guild.icon_id)
                    .and_then(|i| app.images.get(i))
                {
                    image(handle.clone())
                        .width(t::INLINE_ICON)
                        .height(t::INLINE_ICON)
                        .border_radius(t::S4)
                        .into()
                } else {
                    let initials = app
                        .state
                        .model
                        .guilds
                        .get(guild)
                        .map_or(String::new(), |g| {
                            g.guild
                                .name
                                .split_whitespace()
                                .filter_map(|s| s.chars().next())
                                .take(1)
                                .collect::<String>()
                        });
                    let color = t::avatar_color(guild.0);
                    container(hint(initials).font(t::semibold()).color(t::TEXT_PRIMARY))
                        .center_x(t::INLINE_ICON)
                        .center_y(t::INLINE_ICON)
                        .style(move |_| container::Style {
                            border: Border {
                                radius: t::S4.into(),
                                ..Default::default()
                            },
                            ..t::surface(color)
                        })
                        .into()
                };
                row = row.push(face);
            }
            grid = grid.push(row);
        }
        container(grid)
            .center_x(t::GUILD_ICON)
            .center_y(t::GUILD_ICON)
            .into()
    } else {
        container(icon("folder-open", t::S24, color))
            .center_x(t::GUILD_ICON)
            .center_y(t::GUILD_ICON)
            .into()
    };
    let mut layers = vec![container(
        button(glyph)
            .on_press(msg(Event::FolderToggle(id)))
            .width(t::GUILD_ICON)
            .height(t::GUILD_ICON)
            .padding(0)
            .style(move |theme, status| {
                let mut s = t::icon(theme, status);
                s.background = Some(t::alpha(color, 0.16).into());
                s.border.radius = t::GUILD_RADIUS.into();
                s
            }),
    )
    .center_x(t::RAIL)
    .into()];
    if f.collapsed && (selected || unread) {
        layers.push(
            container(
                container(space())
                    .width(t::S4)
                    .height(if selected { t::INPUT } else { t::S8 })
                    .style(|_| container::Style {
                        border: Border {
                            radius: t::S4.into(),
                            ..Default::default()
                        },
                        ..t::surface(t::TEXT_PRIMARY)
                    }),
            )
            .center_y(t::GUILD_ICON)
            .into(),
        );
    }
    if f.collapsed && mentions > 0 {
        layers.push(
            container(
                container(
                    hint(mention_label(mentions))
                        .font(t::bold())
                        .color(t::BADGE_TEXT),
                )
                .padding([0., t::S4])
                .style(|_| container::Style {
                    border: Border {
                        radius: t::ROUND.into(),
                        ..Default::default()
                    },
                    ..t::surface(t::DANGER)
                }),
            )
            .align_right(t::RAIL - t::S8)
            .align_bottom(t::GUILD_ICON)
            .into(),
        );
    }
    let header = controls::tip(
        mouse_area(stack(layers)).on_right_press(msg(Event::FolderEdit(id))),
        format!("{} — {} servers", f.name, f.guild_ids.len()),
        tooltip::Position::Right,
    );
    let mut list = column![header].spacing(t::S8);
    if !f.collapsed {
        for id in &f.guild_ids {
            if let Some(g) = app.state.model.guilds.get(id) {
                list = list.push(cell(app, Some(*id), &g.guild.name));
            }
        }
    }
    container(list)
        .width(t::RAIL)
        .style(move |_| container::Style {
            border: Border {
                radius: t::GUILD_RADIUS.into(),
                ..Default::default()
            },
            ..t::surface(t::alpha(color, 0.05))
        })
        .into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn badge_counts_are_bounded() {
        for (n, label) in [
            (2, "2"),
            (10, "10"),
            (99, "99"),
            (100, "99+"),
            (u32::MAX, "99+"),
        ] {
            assert_eq!(super::mention_label(n), label);
        }
    }
}
