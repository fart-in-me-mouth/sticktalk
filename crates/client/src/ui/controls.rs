//! Shared main-window controls; styles are intentionally explicit.
use super::*;
use crate::main_window::{mm, Message as Main, Popover};
use iced::widget::column;
pub fn rgb(value: u32) -> Color {
    Color::from_rgb8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}
pub fn name_color(app: &App, id: Id) -> Color {
    app.state
        .model
        .launch
        .color(app.state.guild, id)
        .map_or(t::TEXT_PRIMARY, rgb)
}
pub fn mention_color(app: &App, id: Id) -> Color {
    app.state
        .model
        .launch
        .color(app.state.guild, id)
        .map_or(t::ACCENT, rgb)
}
pub fn surface(_: &iced::Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: t::S8.into(),
            width: t::HAIRLINE,
            color: t::DIVIDER,
        },
        shadow: iced::Shadow {
            color: t::alpha(Color::BLACK, 0.35),
            offset: iced::Vector::new(0., t::S8),
            blur_radius: t::S16,
        },
        ..t::surface(t::MENU_BG)
    }
}
pub fn menu_style(
    danger: bool,
) -> impl Fn(&iced::Theme, widget::button::Status) -> widget::button::Style {
    move |theme, status| {
        let mut s = t::icon(theme, status);
        s.border.radius = t::S4.into();
        s.text_color = if danger { t::DANGER } else { t::TEXT_PRIMARY };
        if matches!(
            status,
            widget::button::Status::Hovered | widget::button::Status::Pressed
        ) {
            s.background = Some(if danger { t::DANGER } else { t::ACCENT }.into());
            s.text_color = if danger {
                t::TEXT_PRIMARY
            } else {
                t::ACCENT_TEXT
            };
        }
        s
    }
}
pub fn item<'a>(
    label: impl Into<String>,
    glyph: &str,
    message: Msg,
    danger: bool,
) -> Element<'a, Msg> {
    // Both label and SVG inherit the menu row foreground.
    button(
        row![
            text(label.into())
                .font(t::medium())
                .size(t::USER_NAME)
                .style(t::inherit_text)
                .width(Fill),
            crate::icons::inherited(glyph, t::MENU_ICON)
        ]
        .spacing(t::S8)
        .align_y(Center),
    )
    .on_press(message)
    .height(t::ROW)
    .width(Fill)
    .padding([0., t::S8])
    .style(menu_style(danger))
    .into()
}
pub fn separator<'a>() -> Element<'a, Msg> {
    container(super::line()).padding([t::S4, 0.]).into()
}
pub fn tip<'a>(
    content: impl Into<Element<'a, Msg>>,
    label: impl Into<String>,
    position: tooltip::Position,
) -> Element<'a, Msg> {
    let body = container(
        text(label.into())
            .font(t::medium())
            .size(t::USER_NAME)
            .color(t::TEXT_PRIMARY),
    )
    .padding([t::S8, t::S12])
    .style(|_| container::Style {
        border: Border {
            radius: t::TOOLTIP_RADIUS.into(),
            ..Default::default()
        },
        ..t::surface(t::MENU_BG)
    });
    let arrow=widget::svg(widget::svg::Handle::from_memory(br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><path d="M0 0h8L4 5z" fill="#111214"/></svg>"##.as_slice())).width(t::S8).height(t::S8);
    let balloon: Element<'a, Msg> = if position == tooltip::Position::Right {
        let arrow=widget::svg(widget::svg::Handle::from_memory(br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><path d="M8 0v8L3 4z" fill="#111214"/></svg>"##.as_slice())).width(t::S8).height(t::S8);
        row![arrow, body].align_y(Center).into()
    } else {
        column![body, arrow].align_x(Center).spacing(0).into()
    };
    super::tooltip_delay::warm(
        tooltip(content, balloon, position)
            .delay(std::time::Duration::from_millis(400))
            .padding(0)
            .gap(t::S4)
            .style(t::bare),
    )
}
pub fn icon_button<'a>(glyph: &str, label: &str, message: Msg, active: bool) -> Element<'a, Msg> {
    let danger = glyph == "phone-off";
    let member = glyph == "users";
    tip(
        button(crate::icons::inherited(glyph, t::ICON))
            .on_press(message)
            .width(t::S32)
            .height(t::S32)
            .padding(t::TOOL_ICON_PADDING)
            .style(move |theme, status| {
                let mut s = t::icon(theme, status);
                s.border.radius = t::TOOLTIP_RADIUS.into();
                if danger
                    && matches!(
                        status,
                        widget::button::Status::Hovered | widget::button::Status::Pressed
                    )
                {
                    s.text_color = t::DANGER;
                    s.background = Some(t::alpha(t::DANGER, 0.12).into());
                }
                if active {
                    s.background = Some(t::BG_SELECTED.into());
                    s.text_color = if member { t::ACCENT } else { t::TEXT_PRIMARY };
                }
                s
            }),
        label,
        tooltip::Position::Top,
    )
}
pub fn toggle<'a>(label: impl Into<String>, on: bool, message: Msg) -> Element<'a, Msg> {
    let glyph: Element<'a, Msg> = if on {
        icon("check", t::S12, t::ACCENT).into()
    } else {
        space().into()
    };
    let knob = container(glyph)
        .width(t::TOGGLE_KNOB)
        .height(t::TOGGLE_KNOB)
        .center_x(t::TOGGLE_KNOB)
        .center_y(t::TOGGLE_KNOB)
        .style(|_| container::Style {
            border: Border {
                radius: t::ROUND.into(),
                ..Default::default()
            },
            ..t::surface(t::TEXT_PRIMARY)
        });
    let track = container(knob)
        .width(t::TOGGLE_W)
        .height(t::S24)
        .padding(t::FOCUS)
        .align_x(if on {
            iced::alignment::Horizontal::Right
        } else {
            iced::alignment::Horizontal::Left
        })
        .style(move |_| container::Style {
            border: Border {
                radius: t::S12.into(),
                ..Default::default()
            },
            ..t::surface(if on { t::ACCENT } else { t::TOGGLE_OFF })
        });
    button(
        row![txt(label).size(t::USER_NAME).width(Fill), track]
            .spacing(t::S12)
            .align_y(Center),
    )
    .on_press(message)
    .width(Fill)
    .padding([t::S8, t::S8])
    .style(t::icon)
    .into()
}
pub fn profile_message(id: Id) -> Msg {
    mm(Main::Open(Popover::Profile(id)))
}
pub fn status_badge<'a>(presence: PresenceStatus, size: f32, parent: Color) -> Element<'a, Msg> {
    status_badge_alpha(presence, size, parent, 1.)
}
fn status_badge_alpha<'a>(
    presence: PresenceStatus,
    size: f32,
    parent: Color,
    opacity: f32,
) -> Element<'a, Msg> {
    let c = status(presence).1;
    let c = Color::from_rgb(
        parent.r + (c.r - parent.r) * opacity,
        parent.g + (c.g - parent.g) * opacity,
        parent.b + (c.b - parent.b) * opacity,
    );
    let hex = |c: Color| {
        format!(
            "#{:02x}{:02x}{:02x}",
            (c.r * 255.) as u8,
            (c.g * 255.) as u8,
            (c.b * 255.) as u8
        )
    };
    let ring = if size >= 80. { 5. } else { 3. };
    let dot = if size >= 80. { 16. } else { 10. };
    let total = dot + 2. * ring;
    let r = dot / 2.;
    let center = total / 2.;
    let extra = match presence {
        PresenceStatus::Idle => format!(
            r#"<circle cx="{}" cy="{}" r="{}" fill="{}"/>"#,
            center + r * 0.35,
            center - r * 0.35,
            r * 0.85,
            hex(parent)
        ),
        PresenceStatus::Dnd => format!(
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="1" fill="{}"/>"#,
            center - r * 0.65,
            center - r * 0.18,
            r * 1.3,
            r * 0.36,
            hex(parent)
        ),
        PresenceStatus::Offline | PresenceStatus::Invisible => format!(
            r#"<circle cx="{center}" cy="{center}" r="{}" fill="{}"/>"#,
            r * 0.55,
            hex(parent)
        ),
        _ => String::new(),
    };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{total}" height="{total}"><circle cx="{center}" cy="{center}" r="{center}" fill="{}"/><circle cx="{center}" cy="{center}" r="{r}" fill="{}"/>{extra}</svg>"#,
        hex(parent),
        hex(c)
    );
    widget::svg(widget::svg::Handle::from_memory(svg.into_bytes()))
        .width(total)
        .height(total)
        .into()
}
pub fn voice_avatar<'a>(app: &App, id: Id, speaking: bool) -> Element<'a, Msg> {
    let inset = if speaking { t::FOCUS } else { 0. };
    container(super::avatar(app, id, t::S24 - inset * 2., false))
        .width(t::S24)
        .height(t::S24)
        .padding(inset)
        .style(move |_| container::Style {
            border: Border {
                radius: t::ROUND.into(),
                width: inset,
                color: t::ACCENT,
            },
            ..t::surface(t::BG_SIDEBAR)
        })
        .into()
}
pub fn avatar<'a>(app: &App, id: Id, size: f32, parent: Color, speaking: bool) -> Element<'a, Msg> {
    avatar_fade(app, id, size, parent, speaking, 1.)
}
pub fn avatar_fade<'a>(
    app: &App,
    id: Id,
    size: f32,
    parent: Color,
    speaking: bool,
    opacity: f32,
) -> Element<'a, Msg> {
    let mix = move |fg: Color| {
        Color::from_rgb(
            parent.r + (fg.r - parent.r) * opacity,
            parent.g + (fg.g - parent.g) * opacity,
            parent.b + (fg.b - parent.b) * opacity,
        )
    };
    let inset = if speaking { t::FOCUS } else { 0. };
    let inner = size - inset * 2.;
    let face: Element<'a, Msg> = if let Some(h) = app
        .state
        .model
        .users
        .get(&id)
        .and_then(|u| u.avatar_id)
        .and_then(|id| app.images.get(id))
    {
        image(super::avatar_mask::masked(h, size))
            .width(inner)
            .height(inner)
            .border_radius(t::ROUND)
            .opacity(opacity)
            .into()
    } else {
        let initials = app
            .state
            .model
            .name(id)
            .split_whitespace()
            .take(if size <= t::S24 { 1 } else { 2 })
            .filter_map(|s| s.chars().next())
            .collect::<String>();
        container(
            txt(initials)
                .size(inner * 0.4)
                .font(t::semibold())
                .color(t::alpha(t::TEXT_PRIMARY, opacity)),
        )
        .center_x(inner)
        .center_y(inner)
        .style(move |_| container::Style {
            border: Border {
                radius: t::ROUND.into(),
                ..Default::default()
            },
            ..t::surface(mix(t::avatar_color(id.0)))
        })
        .into()
    };
    let face: Element<'a, Msg> = container(face)
        .width(size)
        .height(size)
        .padding(inset)
        .style(move |_| container::Style {
            border: Border {
                radius: t::ROUND.into(),
                width: inset,
                color: t::ACCENT,
            },
            ..t::surface(parent)
        })
        .into();
    let presence = app
        .state
        .model
        .presences
        .get(&id)
        .copied()
        .unwrap_or(PresenceStatus::Offline);
    stack![
        face,
        container(status_badge_alpha(presence, size, parent, opacity))
            .align_right(size)
            .align_bottom(size)
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uncolored_mentions_keep_the_default_mint_tint() {
        let app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        assert_eq!(mention_color(&app, Id(4)), t::ACCENT);
        assert_eq!(name_color(&app, Id(4)), t::TEXT_PRIMARY);
        assert_eq!(mention_color(&app, Id(2)), name_color(&app, Id(2)));
    }
}
