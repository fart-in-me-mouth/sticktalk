//! UI_SPEC.md §1 — the single source of truth for visual tokens and styles.
use iced::{
    widget::{
        button as button_widget, container, rule, scrollable, svg as svg_widget, text, text_editor,
        text_input,
    },
    Border, Color, Font, Shadow, Theme,
};
pub const BG_RAIL: Color = Color::from_rgb8(0x11, 0x11, 0x13);
pub const BG_SIDEBAR: Color = Color::from_rgb8(0x17, 0x17, 0x1a);
pub const BG_CHAT: Color = Color::from_rgb8(0x1c, 0x1c, 0x20);
pub const BG_ELEVATED: Color = Color::from_rgb8(0x26, 0x26, 0x2b);
pub const BG_HOVER: Color = Color::from_rgb8(0x23, 0x23, 0x27);
pub const BG_SELECTED: Color = Color::from_rgb8(0x2e, 0x2e, 0x34);
pub const DIVIDER: Color = Color::from_rgb8(0x2a, 0x2a, 0x2f);
pub const TEXT_PRIMARY: Color = Color::from_rgb8(0xe8, 0xea, 0xed);
pub const TEXT_SECONDARY: Color = Color::from_rgb8(0xa8, 0xa8, 0xb0);
pub const TEXT_MUTED: Color = Color::from_rgb8(0x74, 0x74, 0x7c);
pub const ACCENT: Color = Color::from_rgb8(0x3d, 0xdc, 0x97);
pub const ACCENT_HOVER: Color = Color::from_rgb8(0x5b, 0xe5, 0xaa);
pub const ACCENT_TEXT: Color = Color::from_rgb8(0x08, 0x17, 0x0f);
pub const DANGER: Color = Color::from_rgb8(0xf2, 0x5f, 0x5c);
pub const WARNING: Color = Color::from_rgb8(0xf2, 0xb8, 0x4b);
pub const USER_PANEL: Color = Color::from_rgb8(0x14, 0x14, 0x17);
pub const INPUT_BG: Color = Color::from_rgb8(0x12, 0x12, 0x15);
pub const BACKDROP: Color = Color::from_rgba8(0, 0, 0, 0.7);
pub const TRANSPARENT: Color = Color::TRANSPARENT;
pub const S4: f32 = 4.;
pub const S8: f32 = 8.;
pub const S12: f32 = 12.;
pub const S16: f32 = 16.;
pub const S24: f32 = 24.;
pub const S32: f32 = 32.;
pub const HAIRLINE: f32 = 1.;
pub const FOCUS: f32 = 2.;
pub const CONTINUATION: f32 = 2.;
pub const ROUND: f32 = 999.;
pub const RAIL: f32 = 72.;
pub const SIDEBAR: f32 = 240.;
pub const MEMBERS: f32 = 240.;
pub const HEADER: f32 = 48.;
pub const USER_HEIGHT: f32 = 52.;
pub const GUILD_ICON: f32 = 48.;
pub const GUILD_RADIUS: f32 = 16.;
pub const ROW: f32 = 32.;
pub const MEMBER_ROW: f32 = 48.;
pub const BUTTON: f32 = 38.;
pub const INPUT: f32 = 40.;
pub const COMPOSER_MIN: f32 = 44.;
pub const TYPING: f32 = 24.;
pub const AUTOCOMPLETE: f32 = 36.;
pub const ATTACHMENT_CARD: f32 = 96.;
pub const MIN_W: f32 = 940.;
pub const MIN_H: f32 = 600.;
pub const MEMBER_BREAKPOINT: f32 = 1100.;
pub const LOGIN_W: f32 = 480.;
pub const MODAL_W: f32 = 560.;
pub const SETTINGS_NAV: f32 = 218.;
pub const SETTINGS_CONTENT: f32 = 740.;
pub const MARKDOWN_H2: f32 = 18.;
pub const MARKDOWN_H3: f32 = 16.;
pub const CONTEXT_W: f32 = 240.;
pub const ICON: f32 = 20.;
pub const INLINE_ICON: f32 = 16.;
pub const CHANNEL_ICON: f32 = 18.;
pub const AVATAR: f32 = 40.;
pub const SMALL_AVATAR: f32 = 32.;
pub const TINY_AVATAR: f32 = 16.;
pub const TITLE: f32 = 20.;
pub const BODY: f32 = 15.;
pub const HINT: f32 = 12.;
pub const USER_NAME: f32 = 14.;
pub const LINE: f32 = 1.375;
pub const GLYPH_WIDTH: f32 = 7.5;
pub const MESSAGE_GUTTER: f32 = 72.;
pub const THUMB_W: f32 = 320.;
pub const THUMB_H: f32 = 180.;
pub const IMAGE_MAX_W: f32 = 400.;
pub const IMAGE_MAX_H: f32 = 300.;
pub const VIRTUAL_HEADER: f32 = 48.;
pub const MODAL_INSET: f32 = 64.;
pub const DEFAULT_W: f32 = 1280.;
pub const DEFAULT_H: f32 = 800.;
pub const DAY_MS: u64 = 86_400_000;
pub const GROUP_MS: u64 = 420_000;
pub fn regular() -> Font {
    Font::with_name("StickTalk UI")
}
pub fn mono() -> Font {
    Font::with_name("StickTalk Mono")
}
pub fn medium() -> Font {
    Font {
        weight: iced::font::Weight::Medium,
        ..regular()
    }
}
pub fn semibold() -> Font {
    Font {
        weight: iced::font::Weight::Semibold,
        ..regular()
    }
}
pub fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}
pub fn theme(_: &crate::app::App) -> Theme {
    Theme::custom(
        "StickTalk · neutral",
        iced::theme::Palette {
            background: BG_CHAT,
            text: TEXT_PRIMARY,
            primary: ACCENT,
            success: ACCENT,
            danger: DANGER,
            warning: WARNING,
        },
    )
}
pub fn surface(color: Color) -> container::Style {
    container::Style {
        background: Some(color.into()),
        text_color: Some(TEXT_PRIMARY),
        ..Default::default()
    }
}
pub fn bare(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT_PRIMARY),
        ..Default::default()
    }
}
pub fn rail(_: &Theme) -> container::Style {
    surface(BG_RAIL)
}
pub fn sidebar(_: &Theme) -> container::Style {
    surface(BG_SIDEBAR)
}
pub fn chat(_: &Theme) -> container::Style {
    surface(BG_CHAT)
}
pub fn elevated(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: S8.into(),
            ..Default::default()
        },
        ..surface(BG_ELEVATED)
    }
}
pub fn modal(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: S12.into(),
            ..Default::default()
        },
        shadow: Shadow {
            color: alpha(Color::BLACK, 0.24),
            offset: iced::Vector::new(0., S16),
            blur_radius: S32,
        },
        ..surface(BG_ELEVATED)
    }
}
pub fn tooltip(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: S8.into(),
            ..Default::default()
        },
        ..surface(BG_ELEVATED)
    }
}
pub fn dim(_: &Theme) -> container::Style {
    surface(BACKDROP)
}
pub fn primary_text(_: &Theme) -> text::Style {
    text::Style {
        color: Some(TEXT_PRIMARY),
    }
}
pub fn secondary_text(_: &Theme) -> text::Style {
    text::Style {
        color: Some(TEXT_SECONDARY),
    }
}
pub fn muted_text(_: &Theme) -> text::Style {
    text::Style {
        color: Some(TEXT_MUTED),
    }
}
pub fn accent_text(_: &Theme) -> text::Style {
    text::Style {
        color: Some(ACCENT),
    }
}
pub fn button(_: &Theme, status: button_widget::Status) -> button_widget::Style {
    button_variant(status, BG_ELEVATED, TEXT_PRIMARY)
}
pub fn primary(_: &Theme, status: button_widget::Status) -> button_widget::Style {
    button_variant(status, ACCENT, ACCENT_TEXT)
}
pub fn danger(_: &Theme, status: button_widget::Status) -> button_widget::Style {
    button_variant(status, DANGER, TEXT_PRIMARY)
}
pub fn link(_: &Theme, status: button_widget::Status) -> button_widget::Style {
    let mut s = button_variant(status, TRANSPARENT, ACCENT);
    s.background = Some(TRANSPARENT.into());
    s
}
pub fn danger_link(_: &Theme, status: button_widget::Status) -> button_widget::Style {
    let mut s = icon(&Theme::Dark, status);
    s.text_color = DANGER;
    s
}
fn button_variant(status: button_widget::Status, bg: Color, fg: Color) -> button_widget::Style {
    let disabled = matches!(status, button_widget::Status::Disabled);
    let hover = matches!(status, button_widget::Status::Hovered);
    let pressed = matches!(status, button_widget::Status::Pressed);
    button_widget::Style {
        background: Some(
            if disabled {
                alpha(bg, 0.4)
            } else if hover && bg == ACCENT {
                ACCENT_HOVER
            } else if hover {
                BG_SELECTED
            } else {
                bg
            }
            .into(),
        ),
        text_color: if disabled { alpha(fg, 0.4) } else { fg },
        border: Border {
            color: if pressed { ACCENT } else { TRANSPARENT },
            width: if pressed { FOCUS } else { 0. },
            radius: S8.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}
pub fn icon(_: &Theme, status: button_widget::Status) -> button_widget::Style {
    let mut s = button_variant(status, TRANSPARENT, TEXT_SECONDARY);
    if matches!(
        status,
        button_widget::Status::Hovered | button_widget::Status::Pressed
    ) {
        s.background = Some(BG_HOVER.into());
        s.text_color = TEXT_PRIMARY;
    }
    s
}
pub fn navigation(
    selected: bool,
) -> impl Fn(&Theme, button_widget::Status) -> button_widget::Style {
    move |theme, status| {
        let mut s = icon(theme, status);
        s.border.radius = S4.into();
        if selected {
            s.background = Some(BG_SELECTED.into());
            s.text_color = TEXT_PRIMARY;
        }
        s
    }
}
pub fn reaction(mine: bool) -> impl Fn(&Theme, button_widget::Status) -> button_widget::Style {
    move |theme, status| {
        let mut s = button(theme, status);
        if mine {
            s.background = Some(alpha(ACCENT, 0.15).into());
            s.border.color = ACCENT;
            s.border.width = HAIRLINE;
        }
        s
    }
}
pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: INPUT_BG.into(),
        border: Border {
            radius: S8.into(),
            color: if focused { ACCENT } else { TRANSPARENT },
            width: if focused { FOCUS } else { HAIRLINE },
        },
        icon: TEXT_MUTED,
        placeholder: TEXT_MUTED,
        value: if matches!(status, text_input::Status::Disabled) {
            alpha(TEXT_PRIMARY, 0.4)
        } else {
            TEXT_PRIMARY
        },
        selection: alpha(ACCENT, 0.3),
    }
}
pub fn editor(_: &Theme, status: text_editor::Status) -> text_editor::Style {
    text_editor::Style {
        background: TRANSPARENT.into(),
        border: Border {
            color: if matches!(status, text_editor::Status::Focused { .. }) {
                ACCENT
            } else {
                TRANSPARENT
            },
            width: if matches!(status, text_editor::Status::Focused { .. }) {
                FOCUS
            } else {
                0.
            },
            radius: S8.into(),
        },
        placeholder: TEXT_MUTED,
        value: TEXT_PRIMARY,
        selection: alpha(ACCENT, 0.3),
    }
}
pub fn divider(_: &Theme) -> rule::Style {
    rule::Style {
        color: DIVIDER,
        radius: 0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}
pub fn new_divider(_: &Theme) -> rule::Style {
    rule::Style {
        color: DANGER,
        ..divider(&Theme::Dark)
    }
}
pub fn scroll(_: &Theme, status: scrollable::Status) -> scrollable::Style {
    let visible = !matches!(status, scrollable::Status::Active { .. });
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: if visible { TEXT_MUTED } else { TRANSPARENT }.into(),
            border: Border {
                radius: S4.into(),
                ..Default::default()
            },
        },
    };
    scrollable::Style {
        container: bare(&Theme::Dark),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: BG_ELEVATED.into(),
            border: Border::default(),
            shadow: Shadow::default(),
            icon: TEXT_SECONDARY,
        },
    }
}
pub fn svg(color: Color) -> impl Fn(&Theme, svg_widget::Status) -> svg_widget::Style {
    move |_, _| svg_widget::Style { color: Some(color) }
}
pub fn avatar_color(id: u64) -> Color {
    [
        Color::from_rgb8(83, 103, 95),
        Color::from_rgb8(103, 88, 116),
        Color::from_rgb8(117, 96, 80),
        Color::from_rgb8(80, 98, 113),
    ][id as usize % 4]
}
pub fn message(hovered: bool, mention: bool) -> container::Style {
    if mention {
        surface(alpha(WARNING, 0.08))
    } else if hovered {
        surface(BG_HOVER)
    } else {
        bare(&Theme::Dark)
    }
}

pub fn inherit_text(_: &Theme) -> text::Style {
    text::Style { color: None }
}

pub fn choice(theme: &Theme, status: button_widget::Status) -> button_widget::Style {
    let mut s = button(theme, status);
    s.background = Some(
        if matches!(
            status,
            button_widget::Status::Hovered | button_widget::Status::Pressed
        ) {
            BG_HOVER
        } else {
            BG_CHAT
        }
        .into(),
    );
    s
}
pub fn input_container(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: S8.into(),
            ..Default::default()
        },
        ..surface(INPUT_BG)
    }
}

pub fn close_circle(theme: &Theme, status: button_widget::Status) -> button_widget::Style {
    let mut s = icon(theme, status);
    s.border = Border {
        color: TEXT_MUTED,
        width: HAIRLINE,
        radius: ROUND.into(),
    };
    s
}

pub fn google_mark(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: S4.into(),
            ..Default::default()
        },
        ..surface(Color::WHITE)
    }
}
pub fn google_button(theme: &Theme, status: button_widget::Status) -> button_widget::Style {
    let mut s = button(theme, status);
    s.border = Border {
        color: TEXT_MUTED,
        width: HAIRLINE,
        radius: S8.into(),
    };
    s
}

// Media surfaces (B1 and GIFS.md).
pub const PICKER_W: f32 = 420.;
pub const VIEWER_MARGIN: f32 = 48.;
pub const VIEWER_BACKDROP: Color = Color::from_rgba8(0, 0, 0, 0.85);
pub fn popover(theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: DIVIDER,
            width: HAIRLINE,
            radius: S8.into(),
        },
        ..modal(theme)
    }
}
