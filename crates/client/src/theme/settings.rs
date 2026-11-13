//! UI_PARITY §0, §2 and §3. Additive settings tokens.
use super::*;
pub const CONTENT: f32 = super::SETTINGS_CONTENT - super::AVATAR;
pub const NAV: f32 = 256.;
pub const NAV_ROW: f32 = 34.;
pub const NAV_ICON: f32 = 18.;
pub const LABEL: f32 = 16.;
pub const SMALL: f32 = 14.;
pub const TABLE_ROW: f32 = 56.;
pub const PREVIEW: f32 = 320.;
pub const PROFILE_PREVIEW: f32 = 340.;
pub const BANNER: f32 = 105.;
pub const LARGE_AVATAR: f32 = 80.;
pub const SERVER_ICON: f32 = 64.;
pub const TEXTAREA: f32 = 128.;
pub const SERVER_DESCRIPTION_H: f32 = 80.;
pub const SWATCH_W: f32 = 104.;
pub const SWATCH_H: f32 = 64.;
pub const SWITCH_W: f32 = 40.;
pub const SWITCH_H: f32 = 24.;
pub const MENU_PADDING: f32 = 6.;
pub const MENU_WIDTH: f32 = 220.;
pub const SECONDARY: Color = Color::from_rgb8(0x3a, 0x3a, 0x41);
pub const SECONDARY_HOVER: Color = Color::from_rgb8(0x45, 0x45, 0x4d);
pub const ACTION_BG: Color = Color::from_rgb8(0x2b, 0x2b, 0x30);
pub const SWITCH_OFF: Color = Color::from_rgba8(0x72, 0x76, 0x7d, 0.6);
pub const MENU: Color = Color::from_rgb8(0x11, 0x12, 0x14);
pub const WHITE: Color = Color::WHITE;
pub const COLORS: [u32; 10] = [
    0x3ddc97, 0x57a7d9, 0x9878d5, 0xd77d9b, 0xe5a663, 0x54bcb5, 0x7283c4, 0xbc89c8, 0xd9c570,
    0x74747c,
];
pub const GRADIENTS: [(u32, u32); 10] = [
    (0x376f61, 0x1c3436),
    (0x487ea0, 0x29364e),
    (0x78609f, 0x37304b),
    (0xa05e79, 0x4b3041),
    (0xa67c50, 0x49382d),
    (0x408f8d, 0x253e4d),
    (0x646cb0, 0x323350),
    (0x965b9e, 0x49304d),
    (0xa19350, 0x4d4229),
    (0x687480, 0x323941),
];
pub fn rgb(v: u32) -> Color {
    Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}
pub fn secondary(theme: &Theme, status: button_widget::Status) -> button_widget::Style {
    let mut s = button(theme, status);
    s.background = Some(
        if matches!(
            status,
            button_widget::Status::Hovered | button_widget::Status::Pressed
        ) {
            SECONDARY_HOVER
        } else {
            SECONDARY
        }
        .into(),
    );
    if status == button_widget::Status::Disabled {
        s.background = Some(alpha(SECONDARY, 0.4).into());
    }
    s
}
pub fn row_action(theme: &Theme, status: button_widget::Status) -> button_widget::Style {
    let mut s = secondary(theme, status);
    if status == button_widget::Status::Active {
        s.background = Some(ACTION_BG.into());
    }
    s
}
pub fn slider(_: &Theme, _: iced::widget::slider::Status) -> iced::widget::slider::Style {
    use iced::widget::slider::*;
    Style {
        rail: Rail {
            backgrounds: (ACCENT.into(), SECONDARY.into()),
            width: S8,
            border: Border {
                radius: S4.into(),
                ..Default::default()
            },
        },
        handle: Handle {
            shape: HandleShape::Rectangle {
                width: S8 as u16,
                border_radius: S4.into(),
            },
            background: WHITE.into(),
            border_width: 0.,
            border_color: TRANSPARENT,
        },
    }
}
pub fn pick_list(
    _: &Theme,
    status: iced::widget::pick_list::Status,
) -> iced::widget::pick_list::Style {
    iced::widget::pick_list::Style {
        text_color: TEXT_PRIMARY,
        placeholder_color: TEXT_MUTED,
        handle_color: TEXT_SECONDARY,
        background: INPUT_BG.into(),
        border: Border {
            color: if matches!(status, iced::widget::pick_list::Status::Opened { .. }) {
                ACCENT
            } else {
                TRANSPARENT
            },
            width: HAIRLINE,
            radius: S8.into(),
        },
    }
}
pub fn menu(_: &Theme) -> iced::widget::overlay::menu::Style {
    iced::widget::overlay::menu::Style {
        background: MENU.into(),
        border: Border {
            radius: S8.into(),
            color: DIVIDER,
            width: HAIRLINE,
        },
        text_color: TEXT_PRIMARY,
        selected_text_color: ACCENT_TEXT,
        selected_background: ACCENT.into(),
        shadow: Shadow::default(),
    }
}
pub fn swatch(
    color: Color,
    selected: bool,
) -> impl Fn(&Theme, button_widget::Status) -> button_widget::Style {
    move |_, status| button_widget::Style {
        background: Some(
            if status == button_widget::Status::Disabled {
                alpha(color, 0.4)
            } else {
                color
            }
            .into(),
        ),
        text_color: TEXT_PRIMARY,
        border: Border {
            color: if selected {
                WHITE
            } else if status == button_widget::Status::Hovered {
                TEXT_SECONDARY
            } else {
                TRANSPARENT
            },
            width: FOCUS,
            radius: S8.into(),
        },
        ..Default::default()
    }
}
pub fn textarea(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let mut s = editor(theme, status);
    s.background = INPUT_BG.into();
    s
}
pub fn gradient(index: u8) -> iced::Background {
    let (a, b) = GRADIENTS[index.min(9) as usize];
    iced::gradient::Linear::new(iced::Radians(std::f32::consts::FRAC_PI_4))
        .add_stop(0., rgb(a))
        .add_stop(1., rgb(b))
        .into()
}

pub fn danger_secondary(theme: &Theme, status: button_widget::Status) -> button_widget::Style {
    let mut s = secondary(theme, status);
    s.text_color = if status == button_widget::Status::Disabled {
        alpha(DANGER, 0.4)
    } else {
        DANGER
    };
    s
}

pub fn role_color(v: u32) -> Color {
    if v == 0 {
        TEXT_MUTED
    } else {
        rgb(v)
    }
}
pub fn toggle_row(theme: &Theme, status: button_widget::Status) -> button_widget::Style {
    let mut style = icon(theme, status);
    if status == button_widget::Status::Disabled {
        style.background = None;
    }
    style
}
pub fn menu_surface(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: DIVIDER,
            width: HAIRLINE,
            radius: S8.into(),
        },
        shadow: Shadow {
            color: alpha(Color::BLACK, 0.24),
            offset: iced::Vector::new(0., S8),
            blur_radius: S24,
        },
        ..surface(MENU)
    }
}
pub fn menu_danger(theme: &Theme, status: button_widget::Status) -> button_widget::Style {
    let mut s = icon(theme, status);
    s.border.radius = S4.into();
    if matches!(
        status,
        button_widget::Status::Hovered | button_widget::Status::Pressed
    ) {
        s.background = Some(DANGER.into());
        s.text_color = WHITE;
    } else {
        s.background = None;
        s.text_color = if status == button_widget::Status::Disabled {
            alpha(DANGER, 0.4)
        } else {
            DANGER
        };
    }
    s
}
