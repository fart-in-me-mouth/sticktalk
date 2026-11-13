use super::*;
use iced::widget::column;
pub(super) const LOCKED: &str = "You don't have permission to change this setting.";
pub(super) fn label_text<'a>(s: impl Into<String>) -> widget::Text<'a> {
    text(s.into())
        .size(st::LABEL)
        .font(t::semibold())
        .color(t::TEXT_PRIMARY)
}
pub(super) fn small<'a>(s: impl Into<String>) -> widget::Text<'a> {
    text(s.into())
        .size(st::SMALL)
        .font(t::regular())
        .color(t::TEXT_SECONDARY)
}
pub(super) fn primary<'a>(s: &str, a: A, enabled: bool) -> crate::focus_button::Button<'a> {
    button(
        text(s.to_owned())
            .size(st::SMALL)
            .font(t::medium())
            .style(t::inherit_text),
    )
    .on_press_maybe(enabled.then_some(Msg::Settings(a)))
    .height(t::BUTTON)
    .padding([t::S8, t::S16])
    .style(t::primary)
}
pub(super) fn secondary<'a>(s: &str, a: A, enabled: bool) -> crate::focus_button::Button<'a> {
    button(
        column![
            text(s.to_owned())
                .size(st::SMALL)
                .font(t::medium())
                .style(t::inherit_text),
            space().width(t::ATTACHMENT_CARD - t::S32).height(0.)
        ]
        .spacing(0),
    )
    .on_press_maybe(enabled.then_some(Msg::Settings(a)))
    .height(t::BUTTON)
    .padding([t::S8, t::S16])
    .style(st::secondary)
}

pub(super) fn disabled_tip<'a>(
    e: impl Into<Element<'a, Msg>>,
    enabled: bool,
    reason: &str,
) -> Element<'a, Msg> {
    if enabled {
        e.into()
    } else {
        tooltip(e, settings_tooltip(reason), tooltip::Position::Top)
            .delay(std::time::Duration::from_millis(400))
            .style(t::bare)
            .into()
    }
}
pub(super) fn input<'a>(
    placeholder: &str,
    value: &'a str,
    f: F,
    enabled: bool,
) -> Element<'a, Msg> {
    let e = text_input(placeholder, value)
        .on_input_maybe(enabled.then_some(move |s| Msg::Settings(A::Text(f, s))))
        .size(st::LABEL)
        .font(t::regular())
        .padding([t::S8, t::S12])
        .style(t::input);
    disabled_tip(e, enabled, LOCKED)
}
pub(super) fn field<'a>(
    label: &str,
    placeholder: &str,
    value: &'a str,
    f: F,
    enabled: bool,
) -> Element<'a, Msg> {
    column![section(label), input(placeholder, value, f, enabled)]
        .spacing(t::S8)
        .into()
}
pub(super) fn nav_item<'a>(i: &str, s: &str, a: A, selected: bool) -> Element<'a, Msg> {
    button(
        row![
            icon(
                i,
                st::NAV_ICON,
                if selected {
                    t::TEXT_PRIMARY
                } else {
                    t::TEXT_SECONDARY
                }
            ),
            text(s.to_owned())
                .size(st::SMALL)
                .font(t::medium())
                .style(t::inherit_text)
        ]
        .spacing(t::S12)
        .align_y(Center),
    )
    .on_press(Msg::Settings(a))
    .height(st::NAV_ROW)
    .width(Fill)
    .padding([t::S4, t::S8])
    .style(t::navigation(selected))
    .into()
}
pub(super) fn toggle<'a>(
    label: &str,
    description: &str,
    on: bool,
    action: A,
    enabled: bool,
) -> Element<'a, Msg> {
    let symbol: Element<'a, Msg> = if on {
        icon("check", t::HINT, t::ACCENT).into()
    } else {
        space().into()
    };
    let knob: Element<'a, Msg> = container(symbol)
        .center_x(st::SWITCH_H - t::S4)
        .center_y(st::SWITCH_H - t::S4)
        .style(move |_| {
            let mut s = t::surface(t::alpha(st::WHITE, if enabled { 1. } else { 0.4 }));
            s.border.radius = t::ROUND.into();
            s
        })
        .into();
    let knob_row = if on {
        row![space().width(Fill), knob]
    } else {
        row![knob, space().width(Fill)]
    };
    let switch = container(knob_row)
        .padding(t::FOCUS)
        .width(st::SWITCH_W)
        .height(st::SWITCH_H)
        .style(move |_| {
            let mut s = t::surface(t::alpha(
                if on { t::ACCENT } else { st::SWITCH_OFF },
                (if on { t::ACCENT } else { st::SWITCH_OFF }).a * if enabled { 1. } else { 0.4 },
            ));
            s.border.radius = t::ROUND.into();
            s
        });
    let e = button(
        row![
            column![
                label_text(label).color(if enabled {
                    t::TEXT_PRIMARY
                } else {
                    t::alpha(t::TEXT_PRIMARY, 0.4)
                }),
                small(description).color(if enabled {
                    t::TEXT_MUTED
                } else {
                    t::alpha(t::TEXT_MUTED, 0.4)
                })
            ]
            .spacing(t::S8)
            .width(Fill),
            switch
        ]
        .spacing(t::S24)
        .align_y(Center),
    )
    .on_press_maybe(enabled.then_some(Msg::Settings(action)))
    .width(Fill)
    .padding([t::S12, 0.])
    .style(st::toggle_row);
    disabled_tip(e, enabled, LOCKED)
}
pub(super) fn choice<'a>(label: &str, selected: bool, a: A) -> Element<'a, Msg> {
    button(
        row![
            icon(
                if selected { "check" } else { "circle" },
                t::ICON,
                if selected { t::ACCENT } else { t::TEXT_MUTED }
            ),
            small(label).color(t::TEXT_PRIMARY)
        ]
        .spacing(t::S12)
        .align_y(Center),
    )
    .on_press(Msg::Settings(a))
    .padding(t::S12)
    .width(Fill)
    .style(t::navigation(selected))
    .into()
}
pub(super) fn number<'a>(
    label: &str,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    n: N,
    suffix: &str,
) -> Element<'a, Msg> {
    column![
        row![
            label_text(label).width(Fill),
            small(format!("{value:.0}{suffix}"))
        ],
        widget::slider(range, value, move |v| Msg::Settings(A::Number(n, v)))
            .step(1_f32)
            .style(st::slider)
    ]
    .spacing(t::S16)
    .into()
}
pub(super) fn swatches<'a>(color: u32, field: bool, enabled: bool) -> Element<'a, Msg> {
    let mut r = Row::new().spacing(t::S8);
    for c in st::COLORS {
        r = r.push(
            button(space().width(t::S24).height(t::S24))
                .on_press_maybe(enabled.then_some(Msg::Settings(if field {
                    A::ProfileColor(c)
                } else {
                    A::Text(F::RoleHex, format!("#{c:06X}"))
                })))
                .padding(0)
                .style(st::swatch(st::rgb(c), c == color)),
        );
    }
    r.into()
}
pub(super) fn role_chip<'a>(r: &'a Role) -> Element<'a, Msg> {
    container(
        row![
            container(space().width(t::S8).height(t::S8)).style(move |_| {
                let mut s = t::surface(st::rgb(r.color));
                s.border.radius = t::ROUND.into();
                s
            }),
            small(ellipsis(&r.name, 20))
        ]
        .spacing(t::S8)
        .align_y(Center),
    )
    .padding([t::S4, t::S8])
    .style(|_| {
        let mut s = t::surface(st::ACTION_BG);
        s.border.radius = t::S4.into();
        s
    })
    .into()
}
pub(super) fn row_action<'a>(i: &str, label: &str, a: A, enabled: bool) -> Element<'a, Msg> {
    tooltip(
        button(icon(
            i,
            t::ICON,
            if enabled {
                t::TEXT_PRIMARY
            } else {
                t::alpha(t::TEXT_PRIMARY, 0.4)
            },
        ))
        .on_press_maybe(enabled.then_some(Msg::Settings(a)))
        .width(t::INPUT)
        .height(t::INPUT)
        .padding(t::S8)
        .style(st::row_action),
        settings_tooltip(if enabled { label } else { LOCKED }),
        tooltip::Position::Top,
    )
    .delay(std::time::Duration::from_millis(400))
    .style(t::bare)
    .into()
}

pub(super) fn textarea<'a>(
    content: &'a widget::text_editor::Content,
    about: bool,
    enabled: bool,
) -> Element<'a, Msg> {
    let mut e = widget::text_editor(content)
        .font(t::regular())
        .size(st::LABEL)
        .height(if about {
            st::TEXTAREA
        } else {
            st::SERVER_DESCRIPTION_H
        })
        .padding(t::S12)
        .style(st::textarea);
    if enabled {
        e = e.on_action(move |a| {
            Msg::Settings(if about {
                A::EditAbout(a)
            } else {
                A::EditDescription(a)
            })
        });
    }
    disabled_tip(e, enabled, LOCKED)
}

pub(super) fn month_year(time: UnixMillis) -> String {
    let d = crate::datetime::local(time);
    format!(
        "{} {}",
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
            [(d.month as usize).saturating_sub(1).min(11)],
        d.year
    )
}

// The arrow is part of the tooltip, not an animated or window-backed overlay.
pub(super) fn settings_tooltip<'a>(label: &str) -> Element<'a, Msg> {
    let arrow = widget::svg(widget::svg::Handle::from_memory(
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 8"><path d="M0 0H16L8 8Z" fill="currentColor"/></svg>"#
    )).width(st::TOOLTIP_ARROW * 2.).height(st::TOOLTIP_ARROW)
        .style(|_, _| widget::svg::Style { color: Some(st::MENU) });
    column![
        container(small(label).font(t::medium()).color(t::TEXT_PRIMARY))
            .padding([t::S8, t::S12])
            .max_width(st::PROFILE_PREVIEW)
            .style(|_| {
                let mut s = t::surface(st::MENU);
                s.border.radius = st::TOOLTIP_RADIUS.into();
                s
            }),
        arrow
    ]
    .align_x(Center)
    .into()
}
