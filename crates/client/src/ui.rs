pub(crate) mod audit;
mod avatar_mask;
mod controls;
mod elided;
pub mod emoji_picker;
pub mod gif_picker;
mod history;
mod link_context;
mod main_window;
pub mod media_settings;
mod members;
mod menus;
mod observe;
mod popout;
mod rail;
pub(crate) mod settings;
mod sidebar;
mod tooltip_delay;
pub mod uploads;
mod video;
pub mod viewer;
use crate::{
    app::{editor_binding, App, Dialog, FriendTab, GuildTab, Msg, Screen},
    focus_button::{button, Button},
    icons::icon,
    model::ConnectionState,
    theme as t,
};
use iced::{
    alignment::Alignment::Center,
    widget::{
        self, column, container, image, mouse_area, opaque, row, rule, scrollable, space, stack,
        text, text_editor, text_input, tooltip, Column, Row,
    },
    Border, Color, Element, Fill, Length, Padding,
};
use sticktalk_proto::*;
mod call;
mod share_picker;
#[cfg(feature = "wgpu")]
mod stream_input;
mod stream_view;
pub(crate) fn purge_main_assets() {
    avatar_mask::clear();
}
fn txt<'a>(value: impl Into<String>) -> widget::Text<'a> {
    text(value.into())
        .font(t::regular())
        .size(t::BODY)
        .style(t::primary_text)
}
pub(crate) fn hint<'a>(value: impl Into<String>) -> widget::Text<'a> {
    text(value.into())
        .font(t::regular())
        .size(t::HINT)
        .style(t::muted_text)
}
fn section<'a>(value: impl Into<String>) -> widget::Text<'a> {
    hint(value).font(t::semibold())
}
pub(crate) fn title<'a>(value: impl Into<String>) -> widget::Text<'a> {
    txt(value).size(t::TITLE).font(t::semibold())
}
pub(crate) fn action<'a>(label: impl Into<String>, message: Msg) -> Button<'a> {
    button(
        text(label.into())
            .font(t::regular())
            .size(t::BODY)
            .style(t::inherit_text),
    )
    .on_press(message)
    .height(t::BUTTON)
    .padding([t::S8, t::S16])
    .style(t::secondary)
}
pub(crate) fn primary<'a>(label: impl Into<String>, message: Msg) -> Button<'a> {
    button(
        text(label.into())
            .font(t::semibold())
            .size(t::BODY)
            .align_x(Center)
            .style(t::inherit_text),
    )
    .on_press(message)
    .centered()
    .height(t::BUTTON)
    .padding([t::S8, t::S16])
    .style(t::primary)
}
fn footer<'a>(label: impl Into<String>, message: Msg) -> Element<'a, Msg> {
    row![
        space().width(Fill),
        action("Cancel", Msg::Close),
        primary(label, message)
    ]
    .spacing(t::S8)
    .align_y(Center)
    .into()
}
fn google_button(app: &App) -> Element<'_, Msg> {
    button(
        row![
            container(
                widget::svg(widget::svg::Handle::from_memory(
                    include_bytes!("../assets/google-g.svg").as_slice()
                ))
                .width(t::ICON)
                .height(t::ICON)
            )
            .padding(t::S4)
            .style(t::google_mark),
            text("Continue with Google")
                .font(t::medium())
                .size(t::BODY)
                .style(t::inherit_text)
        ]
        .spacing(t::S12)
        .align_y(Center),
    )
    .on_press_maybe((!app.busy).then_some(Msg::GoogleStart(false)))
    .centered()
    .width(Fill)
    .height(t::HEADER)
    .padding([t::S8, t::S12])
    .style(t::google_button)
    .into()
}

fn icon_button<'a>(name: &str, label: &str, message: Msg) -> Element<'a, Msg> {
    controls::icon_button(name, label, message, false)
}
fn member_action(app: &App, user: Id) -> Element<'_, Msg> {
    let size = t::ICON + 2. * t::S8;
    let action = if Some(user) != app.me() {
        icon_button(
            "more-horizontal",
            "Manage member",
            Msg::Open(Dialog::MemberMenu(user)),
        )
    } else {
        space().into()
    };
    container(action).width(size).height(size).into()
}
fn message_tool<'a>(name: &str, label: &str, message: Msg) -> Element<'a, Msg> {
    controls::icon_button(name, label, message, false)
}
fn nav<'a>(name: &str, label: impl Into<String>, message: Msg, selected: bool) -> Button<'a> {
    button(
        row![
            icon(
                name,
                t::INLINE_ICON,
                if selected {
                    t::TEXT_PRIMARY
                } else {
                    t::TEXT_SECONDARY
                }
            ),
            txt(label).font(t::medium()).width(Fill)
        ]
        .spacing(t::S12)
        .align_y(Center),
    )
    .on_press(message)
    .padding([t::S8, t::S12])
    .height(t::INPUT)
    .width(Fill)
    .style(t::navigation(selected))
}
fn line<'a>() -> Element<'a, Msg> {
    rule::horizontal(t::HAIRLINE).style(t::divider).into()
}
fn scroller<'a>(body: impl Into<Element<'a, Msg>>) -> crate::scroll::SmoothScrollable<'a> {
    crate::scroll::SmoothScrollable::new(body)
        .direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::new()
                .width(t::S4)
                .scroller_width(t::S4),
        ))
        .style(t::scroll)
}
fn field<'a>(
    label: impl Into<String>,
    placeholder: &'a str,
    value: &'a str,
    n: u8,
) -> Element<'a, Msg> {
    column![
        section(label),
        text_input(placeholder, value)
            .id(format!("dialog-field-{n}"))
            .on_input(move |v| Msg::Field(n, v))
            .size(t::BODY)
            .padding([t::S12, t::S16])
            .line_height(iced::Pixels(t::S16))
            .style(t::input)
    ]
    .spacing(t::S8)
    .into()
}
fn chip<'a>(label: impl Into<String>, color: Color) -> Element<'a, Msg> {
    container(hint(label).color(color).font(t::semibold()))
        .padding([t::S4, t::S8])
        .style(move |_| container::Style {
            border: Border {
                radius: t::S4.into(),
                ..Default::default()
            },
            ..t::surface(t::alpha(color, 0.15))
        })
        .into()
}
fn ellipsis(value: &str, chars: usize) -> String {
    if value.chars().count() > chars {
        format!(
            "{}…",
            value
                .chars()
                .take(chars.saturating_sub(1))
                .collect::<String>()
        )
    } else {
        value.into()
    }
}
fn status(s: PresenceStatus) -> (&'static str, Color) {
    match s {
        PresenceStatus::Online => ("Online", t::ACCENT),
        PresenceStatus::Idle => ("Idle", t::WARNING),
        PresenceStatus::Dnd => ("Do not disturb", t::DANGER),
        PresenceStatus::Invisible => ("Invisible", t::TEXT_MUTED),
        PresenceStatus::Offline => ("Offline", t::TEXT_MUTED),
    }
}
fn dot<'a>(color: Color, size: f32) -> Element<'a, Msg> {
    container(space())
        .width(size)
        .height(size)
        .style(move |_| container::Style {
            border: Border {
                radius: t::ROUND.into(),
                ..Default::default()
            },
            ..t::surface(color)
        })
        .into()
}
fn avatar<'a>(app: &App, id: Id, size: f32, presence: bool) -> Element<'a, Msg> {
    if presence {
        return controls::avatar(app, id, size, t::BG_SIDEBAR, false);
    }
    let face: Element<'_, Msg> = if let Some(handle) = app
        .state
        .model
        .users
        .get(&id)
        .and_then(|u| u.avatar_id)
        .and_then(|a| app.images.get(a))
    {
        image(handle.clone())
            .width(size)
            .height(size)
            .border_radius(t::ROUND)
            .content_fit(iced::ContentFit::Cover)
            .into()
    } else {
        let name = app.state.model.name(id);
        let initials = name
            .split_whitespace()
            .take(if size <= t::S24 { 1 } else { 2 })
            .filter_map(|v| v.chars().next())
            .collect::<String>();
        container(
            txt(initials)
                .size(size * 0.4)
                .line_height(1.)
                .wrapping(widget::text::Wrapping::None)
                .font(t::semibold()),
        )
        .width(size)
        .height(size)
        .center_x(size)
        .center_y(size)
        .style(move |_| container::Style {
            border: Border {
                radius: t::ROUND.into(),
                ..Default::default()
            },
            ..t::surface(t::avatar_color(id.0))
        })
        .into()
    };
    face
}
fn avatar_on<'a>(
    app: &'a App,
    id: Id,
    size: f32,
    presence: bool,
    background: Color,
) -> Element<'a, Msg> {
    if presence {
        controls::avatar(app, id, size, background, false)
    } else {
        avatar(app, id, size, false)
    }
}
fn emoji<'a>(value: &str, size: f32) -> Element<'a, Msg> {
    emoji_picker::glyph(value, size)
}
fn empty<'a>(name: &str, label: impl Into<String>, button: Button<'a>) -> Element<'a, Msg> {
    container(
        column![icon(name, t::AVATAR, t::TEXT_MUTED), txt(label), button]
            .spacing(t::S16)
            .align_x(Center),
    )
    .padding(t::S32)
    .width(Fill)
    .center_x(Fill)
    .center_y(Fill)
    .style(t::bare)
    .into()
}
pub fn view(app: &App) -> Element<'_, Msg> {
    let content = content_view(app);
    let content: Element<'_, Msg> = if (app.state.session.is_none() || app.state.model.me.is_none())
        && app.features.confirm_update
    {
        if let Some(overlay) = crate::features::overlay(app) {
            stack![content, overlay].into()
        } else {
            content
        }
    } else {
        content
    };
    if !app.features.crash_notice {
        return content;
    }
    stack![
        content,
        opaque(
            container(
                container(
                    column![
                        title("StickTalk closed unexpectedly"),
                        hint("A local diagnostic log is available. Nothing was uploaded."),
                        row![
                            action(
                                "Dismiss",
                                Msg::feature(crate::features::Event::Desktop(
                                    crate::desktop_host::Event::DismissCrash
                                ))
                            ),
                            primary(
                                "Open logs folder",
                                Msg::feature(crate::features::Event::Desktop(
                                    crate::desktop_host::Event::OpenLogs
                                ))
                            )
                        ]
                        .spacing(t::S8)
                    ]
                    .spacing(t::S16)
                )
                .padding(t::S24)
                .width(t::LOGIN_W)
                .style(t::modal)
            )
            .center_x(Fill)
            .center_y(Fill)
            .style(t::dim)
        )
    ]
    .into()
}
fn content_view(app: &App) -> Element<'_, Msg> {
    if app.main.avatar_gallery {
        return controls::avatar_gallery(app);
    }
    if app.calls.preview_only {
        return stream_view::video(
            app,
            VoiceSession {
                user_id: Id(2),
                connection_id: Id(902),
            },
            TrackKind::Screen,
            app.height,
        );
    }

    if app.calls.fullscreen && app.calls.popout.is_none() {
        if let Some(session) = app.calls.focus {
            return stream_view::view_at(app, session, app.height);
        }
    }
    if app.state.session.is_none() {
        return login(app);
    }
    if app.state.model.me.is_none() {
        return container(
            container(
                column![
                    icon("message-circle", t::AVATAR, t::ACCENT),
                    title(
                        if matches!(app.state.connection, ConnectionState::Connecting) {
                            "Connecting…"
                        } else {
                            "Reconnecting…"
                        }
                    ),
                    connection_banner(app),
                    skeleton(),
                    action("Sign in instead", Msg::Logout)
                ]
                .spacing(t::S24),
            )
            .width(t::LOGIN_W)
            .padding(t::S32)
            .style(t::modal),
        )
        .center_x(Fill)
        .center_y(Fill)
        .style(t::rail)
        .into();
    }
    let settings_page = if matches!(
        app.dialog,
        Dialog::UserSettings | Dialog::GuildSettings(_) | Dialog::ChannelSettings(_)
    ) {
        Some(&app.dialog)
    } else {
        app.settings_return.as_ref()
    };
    let main: Element<'_, Msg> = if let Some(page) = settings_page {
        settings(app, page)
    } else {
        row![
            rail(app),
            sidebar(app),
            match app.state.screen {
                Screen::Friends(tab) => friends(app, tab),
                Screen::Channel(id)
                    if app
                        .state
                        .model
                        .channels
                        .get(&id)
                        .is_some_and(|c| c.kind == ChannelKind::Voice) =>
                    call::view(app, id),
                Screen::Channel(id) => chat(app, id),
            }
        ]
        .height(Fill)
        .into()
    };
    let mut layers = vec![main];
    if let Some(overlay) = video::overlay(app) {
        layers.push(opaque(overlay));
    }
    if app.calls.camera_picker {
        layers.push(
            container(opaque(share_picker::cameras(app)))
                .center_x(Fill)
                .center_y(Fill)
                .style(t::dim)
                .into(),
        );
    }
    if app.calls.picker {
        layers.push(
            container(opaque(share_picker::view(app)))
                .center_x(Fill)
                .center_y(Fill)
                .style(t::dim)
                .into(),
        );
    }
    if app.calls.call.control.pending.is_some() {
        layers.push(
            container(opaque(call::consent(app)))
                .center_x(Fill)
                .center_y(Fill)
                .style(t::dim)
                .into(),
        );
    }
    if let Some(overlay) = crate::features::overlay(app) {
        layers.push(overlay);
    }
    if !matches!(
        app.dialog,
        Dialog::None | Dialog::UserSettings | Dialog::GuildSettings(_) | Dialog::ChannelSettings(_)
    ) {
        let context = matches!(
            app.dialog,
            Dialog::MessageActions(_) | Dialog::GuildMenu | Dialog::MemberMenu(_)
        );
        let overlay = if context {
            let body = widget::sensor(
                container(context_menu(app))
                    .id("main-popover")
                    .max_height(app.height - t::S16),
            )
            .key(app.main.measurement_revision)
            .on_show(|s| crate::main_window::mm(crate::main_window::Message::PopoverSize(s)))
            .on_resize(|s| crate::main_window::mm(crate::main_window::Message::PopoverSize(s)));
            container(observe::popup(
                opaque(body),
                app.main.position,
                app.main.anchor,
            ))
            .id("dialog-scope")
            .width(Fill)
            .height(Fill)
            .style(t::bare)
        } else {
            container(opaque(modal(app)))
                .id("dialog-scope")
                .center_x(Fill)
                .center_y(Fill)
                .style(t::dim)
        };
        layers.push(
            stack![
                mouse_area(container(space()).width(Fill).height(Fill).style(t::bare))
                    .on_press(Msg::Close)
                    .on_right_press(Msg::Close),
                overlay
            ]
            .into(),
        );
    }
    if let Some(overlay) = main_window::overlay(app) {
        layers.push(overlay);
    }
    if app.main.dropping {
        layers.push(main_window::drop_overlay());
    }
    let is_error = !app.error.is_empty() && matches!(app.dialog, Dialog::None);
    if !app.notice.is_empty() || is_error {
        layers.push(
            container(
                container(
                    row![
                        icon(
                            if is_error { "circle-help" } else { "check" },
                            t::INLINE_ICON,
                            if is_error { t::DANGER } else { t::ACCENT }
                        ),
                        txt(if is_error { &app.error } else { &app.notice })
                    ]
                    .spacing(t::S12)
                    .align_y(Center),
                )
                .padding([t::S12, t::S16])
                .max_width((app.width - t::S24 * 2.).min(t::QUICK_W))
                .style(t::elevated),
            )
            .center_x(Fill)
            .align_bottom(Fill)
            .padding(t::S24)
            .style(t::bare)
            .into(),
        );
    }
    stack(layers).into()
}
fn login(app: &App) -> Element<'_, Msg> {
    let signup = app.signup.is_some();
    let mut form = column![
        row![
            icon("message-circle", t::S32, t::ACCENT),
            title("StickTalk")
        ]
        .spacing(t::S12)
        .align_y(Center),
        title(if signup {
            "Pick a username"
        } else if app.register {
            "Create an account"
        } else {
            "Sign in"
        })
    ]
    .spacing(if app.height < t::DEFAULT_H {
        t::S12
    } else {
        t::S16
    });
    if !signup && app.change_server {
        form = form.push(
            column![
                section("SERVER ADDRESS"),
                row![
                    text_input("https://chat.example.com", &app.server)
                        .id("login-server")
                        .on_input_maybe((!app.busy).then_some(Msg::Server))
                        .on_submit(Msg::RefreshMeta)
                        .padding([t::S12, t::S16])
                        .size(t::BODY)
                        .line_height(iced::Pixels(t::S16))
                        .width(Fill)
                        .style(t::input),
                    action(
                        if app.meta_loading {
                            "Connecting…"
                        } else {
                            "Connect"
                        },
                        Msg::RefreshMeta
                    )
                    .on_press_maybe((!app.meta_loading && !app.busy).then_some(Msg::RefreshMeta))
                ]
                .spacing(t::S8)
                .align_y(Center),
                hint("Use the address from your server administrator.").color(t::TEXT_SECONDARY)
            ]
            .spacing(t::S8),
        );
        if !app.meta_error.is_empty() {
            form = form.push(hint(&app.meta_error).color(t::DANGER));
        }
    }
    if !signup && app.google_available() {
        form = form.push(google_button(app)).push(
            row![line(), hint("or"), line()]
                .spacing(t::S16)
                .align_y(Center),
        );
    }
    form = form.push(
        column![
            section("USERNAME"),
            text_input("your_username", &app.username)
                .id("login-username")
                .on_input_maybe((!app.busy).then_some(Msg::Username))
                .on_submit(if signup {
                    Msg::GoogleComplete
                } else {
                    Msg::Tab(false)
                })
                .padding([t::S12, t::S16])
                .size(t::BODY)
                .line_height(iced::Pixels(t::S16))
                .style(t::input)
        ]
        .spacing(t::S8),
    );
    if app.register && !signup {
        form = form.push(
            column![
                section("DISPLAY NAME"),
                text_input("Optional", &app.display)
                    .on_input_maybe((!app.busy).then_some(Msg::Display))
                    .on_submit(Msg::Tab(false))
                    .padding([t::S12, t::S16])
                    .size(t::BODY)
                    .line_height(iced::Pixels(t::S16))
                    .style(t::input)
            ]
            .spacing(t::S8),
        );
    }
    if !signup {
        let mut password = column![
            section("PASSWORD"),
            text_input("", &app.password)
                .id("login-password")
                .secure(true)
                .on_input_maybe((!app.busy).then_some(Msg::Password))
                .on_submit(Msg::Login)
                .padding([t::S12, t::S16])
                .size(t::BODY)
                .line_height(iced::Pixels(t::S16))
                .style(t::input)
        ]
        .spacing(t::S8);
        if app.register {
            password = password.push(hint("At least 12 characters.").color(t::TEXT_SECONDARY));
        }
        form = form.push(password);
    }
    if !app.error.is_empty() {
        form = form.push(hint(&app.error).color(t::DANGER));
    }
    form = form.push(
        container(
            primary(
                if app.busy {
                    if app.register || signup {
                        "Creating account…"
                    } else {
                        "Signing in…"
                    }
                } else if signup {
                    "Create account"
                } else if app.register {
                    "Register"
                } else {
                    "Sign in"
                },
                if signup {
                    Msg::GoogleComplete
                } else {
                    Msg::Login
                },
            )
            .on_press_maybe((!app.busy).then_some(if signup {
                Msg::GoogleComplete
            } else {
                Msg::Login
            }))
            .width(Fill),
        )
        .id("login-submit")
        .width(Fill),
    );
    if app.oauth_pending() || signup {
        form = form.push(
            action("Cancel", Msg::GoogleCancel)
                .padding(0)
                .style(t::link),
        );
    } else {
        form = form.push(
            row![
                hint(if app.register {
                    "Already have an account?"
                } else {
                    "Need an account?"
                }),
                button(
                    text(if app.register { "Sign in" } else { "Register" })
                        .font(t::semibold())
                        .size(t::HINT)
                        .style(t::inherit_text)
                )
                .padding(0)
                .on_press(Msg::ToggleRegister)
                .style(t::link)
            ]
            .spacing(t::S4)
            .align_y(Center),
        );
    }
    if app.features.update_ready.is_some() {
        form = form.push(crate::features::update_pill(app));
    }
    let mut body = column![container(scroller(form).height(Length::Shrink))
        .id("login-card")
        .padding(if app.height < t::DEFAULT_H {
            t::S24
        } else {
            t::S32
        })
        .width(t::LOGIN_W)
        .max_height(app.height - t::S32 * 2.)
        .style(t::modal)]
    .spacing(t::S12)
    .align_x(Center);
    if !signup {
        body = body.push(
            button(hint(if app.change_server {
                "Done"
            } else {
                "Change server"
            }))
            .on_press_maybe(
                (!app.busy && !app.server.trim().is_empty()).then_some(Msg::ChangeServer),
            )
            .padding(t::S4)
            .style(t::link),
        );
    }
    container(body)
        .center_x(Fill)
        .center_y(Fill)
        .style(t::rail)
        .into()
}
fn rail(app: &App) -> Element<'_, Msg> {
    rail::view(app)
}

fn sidebar(app: &App) -> Element<'_, Msg> {
    sidebar::view(app)
}
fn connection_banner(app: &App) -> Element<'_, Msg> {
    if matches!(app.state.connection, ConnectionState::Ready) {
        return space().height(0).into();
    }
    let mut body = row![
        icon("wifi-off", t::INLINE_ICON, t::WARNING),
        hint(match &app.state.connection {
            ConnectionState::Stopped(v) => v.clone(),
            ConnectionState::Connecting => "Connecting to the server…".into(),
            _ => "Reconnecting… Your conversation will resume automatically.".into(),
        })
        .color(t::WARNING),
        space().width(Fill)
    ]
    .spacing(t::S8)
    .align_y(Center);
    if matches!(app.state.connection, ConnectionState::Stopped(_)) {
        body = body.push(
            action("Reconnect", Msg::Reconnect)
                .height(t::ROW)
                .style(t::link),
        );
    }
    container(body)
        .padding([t::S8, t::S16])
        .width(Fill)
        .style(t::elevated)
        .into()
}

fn friend_action<'a>(name: &str, label: &str, message: Msg) -> Element<'a, Msg> {
    controls::tip(
        button(icon(name, t::ICON, t::TEXT_SECONDARY))
            .on_press(message)
            .width(t::ICON + 2. * t::S8)
            .height(t::ICON + 2. * t::S8)
            .padding(t::S8)
            .style(t::friend_action),
        label,
        tooltip::Position::Top,
    )
}

fn friends(app: &App, tab: FriendTab) -> Element<'_, Msg> {
    let mut tabs = row![
        icon("users", t::ICON, t::TEXT_SECONDARY),
        txt("Friends").font(t::semibold()),
        space().width(t::S16)
    ]
    .spacing(t::S8)
    .align_y(Center);
    for (label, value) in [
        ("Online", FriendTab::Online),
        ("All", FriendTab::All),
        ("Pending", FriendTab::Pending),
        ("Blocked", FriendTab::Blocked),
    ] {
        tabs = tabs.push(
            action(label, Msg::Friends(value))
                .height(t::ROW)
                .style(t::navigation(tab == value)),
        );
    }
    tabs = tabs
        .push(space().width(Fill))
        .push(primary("Add Friend", Msg::Open(Dialog::AddFriend)).height(t::ROW));
    let header = container(tabs)
        .padding([t::S8, t::S16])
        .height(t::HEADER)
        .style(t::chat);
    let rels: Vec<_> = app
        .state
        .model
        .relationships
        .values()
        .filter(|r| match tab {
            FriendTab::Online => {
                r.kind == RelationshipKind::Friend
                    && app
                        .state
                        .model
                        .presences
                        .get(&r.user.id)
                        .is_some_and(|s| *s != PresenceStatus::Offline)
            }
            FriendTab::All => r.kind == RelationshipKind::Friend,
            FriendTab::Pending => matches!(
                r.kind,
                RelationshipKind::IncomingRequest | RelationshipKind::OutgoingRequest
            ),
            FriendTab::Blocked => r.kind == RelationshipKind::Blocked,
        })
        .collect();
    let mut list = column![
        section(format!(
            "{} — {}",
            match tab {
                FriendTab::Online => "ONLINE",
                FriendTab::All => "ALL FRIENDS",
                FriendTab::Pending => "PENDING REQUESTS",
                FriendTab::Blocked => "BLOCKED",
            },
            rels.len()
        )),
        space().height(t::S8)
    ]
    .spacing(t::S8);
    for r in &rels {
        let id = r.user.id;
        let (label, color) = match r.kind {
            RelationshipKind::IncomingRequest => ("Incoming friend request", t::TEXT_SECONDARY),
            RelationshipKind::OutgoingRequest => ("Outgoing friend request", t::TEXT_MUTED),
            RelationshipKind::Blocked => ("Blocked", t::TEXT_MUTED),
            _ => status(
                app.state
                    .model
                    .presences
                    .get(&id)
                    .copied()
                    .unwrap_or(PresenceStatus::Offline),
            ),
        };
        let mut actions = Row::new().spacing(t::S8);
        match r.kind {
            RelationshipKind::Friend => {
                actions = actions
                    .push(friend_action(
                        "message-circle",
                        "Send message",
                        Msg::Command(Command::OpenDm { user_id: id }),
                    ))
                    .push(friend_action(
                        "more-horizontal",
                        "More",
                        crate::main_window::mm(crate::main_window::Message::Open(
                            crate::main_window::Popover::Context(crate::main_window::Entity::User(
                                id,
                            )),
                        )),
                    ));
            }
            RelationshipKind::IncomingRequest => {
                actions = actions
                    .push(friend_action(
                        "check",
                        "Accept",
                        Msg::Command(Command::AcceptFriendRequest { user_id: id }),
                    ))
                    .push(friend_action(
                        "x",
                        "Decline",
                        Msg::Command(Command::DeclineFriendRequest { user_id: id }),
                    ));
            }
            RelationshipKind::OutgoingRequest => {
                actions = actions.push(friend_action(
                    "x",
                    "Cancel request",
                    Msg::Command(Command::CancelFriendRequest { user_id: id }),
                ))
            }
            RelationshipKind::Blocked => {
                actions = actions.push(
                    action(
                        "Unblock",
                        Msg::Command(Command::UnblockUser { user_id: id }),
                    )
                    .style(t::link),
                )
            }
        }
        list = list.push(line()).push(
            mouse_area(
                container(
                    row![
                        avatar(app, id, t::AVATAR, true),
                        column![
                            txt(app.state.model.name(id)).font(t::semibold()),
                            hint(label).color(color)
                        ]
                        .spacing(t::S4),
                        space().width(Fill),
                        actions
                    ]
                    .spacing(t::S16)
                    .align_y(Center),
                )
                .padding([t::S12, t::S8])
                .style(move |_| {
                    t::surface(if app.main.hovered_member == Some(id) {
                        t::BG_HOVER
                    } else {
                        t::TRANSPARENT
                    })
                }),
            )
            .on_enter(crate::main_window::mm(
                crate::main_window::Message::MemberHover(Some(id)),
            ))
            .on_exit(crate::main_window::mm(
                crate::main_window::Message::MemberHover(None),
            )),
        );
    }
    let center: Element<'_, Msg> = if rels.is_empty() {
        empty(
            "users",
            match tab {
                FriendTab::Pending => "No pending requests.",
                FriendTab::Blocked => "No blocked accounts.",
                FriendTab::Online => "No friends online.",
                FriendTab::All => "No friends yet.",
            },
            primary("Add a friend", Msg::Open(Dialog::AddFriend)),
        )
    } else {
        container(scroller(list).height(Fill))
            .padding(t::S24)
            .height(Fill)
            .style(t::bare)
            .into()
    };
    let right = container(
        column![
            section("Active Now"),
            space().height(t::S24),
            icon("message-circle", t::S32, t::TEXT_MUTED),
            txt("It's quiet for now.").font(t::semibold())
        ]
        .spacing(t::S12),
    )
    .padding(t::S24)
    .width(t::MEMBERS)
    .height(Fill)
    .style(t::sidebar);
    let mut body = row![center].height(Fill);
    if app.width >= t::MEMBER_BREAKPOINT {
        body = body.push(right);
    }
    container(column![header, line(), connection_banner(app), body])
        .width(Fill)
        .height(Fill)
        .style(t::chat)
        .into()
}
fn skeleton<'a>() -> Element<'a, Msg> {
    let block = |width| {
        container(space())
            .height(t::S12)
            .width(width)
            .style(t::elevated)
    };
    column![
        row![
            container(space())
                .width(t::AVATAR)
                .height(t::AVATAR)
                .style(t::elevated),
            column![block(t::SIDEBAR), block(t::THUMB_W)].spacing(t::S12)
        ]
        .spacing(t::S16),
        space().height(t::S16),
        row![
            container(space())
                .width(t::AVATAR)
                .height(t::AVATAR)
                .style(t::elevated),
            column![block(t::THUMB_W), block(t::SIDEBAR)].spacing(t::S12)
        ]
        .spacing(t::S16)
    ]
    .padding(t::S16)
    .into()
}
fn chat(app: &App, id: Id) -> Element<'_, Msg> {
    let Some(channel) = app.state.model.channels.get(&id) else {
        return empty(
            "message-circle",
            "This conversation is no longer available.",
            primary("Go to friends", Msg::Friends(FriendTab::Online)),
        );
    };
    let name = app.state.model.channel_name(channel);
    let header = main_window::header(app, channel);
    let rows = app.visible_items();
    let (_, _, top, tail) = app.visible_range();
    let history: Element<'_, Msg> = if !app.main.search.is_empty() {
        main_window::local_results(app, id)
    } else if app.extent.active() {
        history::view(app)
    } else if app.state.loading && rows.is_empty() {
        container(skeleton()).height(Fill).style(t::bare).into()
    } else if rows.is_empty() {
        empty(
            "hash",
            format!("This is the beginning of #{}", name),
            primary("Write the first message", Msg::FocusComposer),
        )
    } else {
        let top_label: Element<'_, Msg> = if app.state.history_more {
            action("Load earlier messages", Msg::Older)
                .height(t::ROW)
                .style(t::link)
                .into()
        } else {
            txt(format!("This is the beginning of #{}", name))
                .font(t::semibold())
                .into()
        };
        let mut list = column![
            container(top_label)
                .height(t::VIRTUAL_HEADER)
                .center_x(Fill)
                .center_y(t::VIRTUAL_HEADER)
                .style(t::bare),
            space().height(top)
        ]
        .spacing(0);
        let visible = rows.into_iter().map(|(id, m, height, grouped)| {
            (
                id,
                match m {
                    Some(m) => message_row(app, m, height, grouped),
                    _ => message_skeleton(height),
                },
            )
        });
        list = list.push(widget::keyed_column(visible).width(Fill));
        list = list.push(space().height(tail + t::HISTORY_BOTTOM));
        scroller(list)
            .id("messages")
            .on_scroll(Msg::Scroll)
            .height(Fill)
            .into()
    };
    let mut history_layers = vec![history];
    if !app.state.at_present
        && app.total_h > app.viewport_h
        && app.total_h - app.scroll_y - app.viewport_h > app.viewport_h
    {
        history_layers.push(
            container(action("Jump to present", Msg::Present).style(t::button))
                .center_x(Fill)
                .align_bottom(Fill)
                .padding(t::S16)
                .style(t::bare)
                .into(),
        );
    }
    let history = widget::sensor(stack(history_layers).height(Fill).width(Fill))
        .on_show(Msg::HistoryViewport)
        .on_resize(Msg::HistoryViewport);
    let center = column![connection_banner(app), history, composer(app, channel)]
        .height(Fill)
        .width(Fill);
    let mut body = row![center].height(Fill);
    if app
        .members_open
        .unwrap_or(app.width >= t::MEMBER_BREAKPOINT)
    {
        body = body.push(member_panel(app, channel));
    }
    container(column![header, line(), body].height(Fill).width(Fill))
        .width(Fill)
        .height(Fill)
        .style(t::chat)
        .into()
}
fn composer<'a>(app: &'a App, channel: &'a ChannelSummary) -> Element<'a, Msg> {
    let permissions = app.channel_permissions(channel.id);
    if !permissions.contains(Permissions::SEND_MESSAGES) {
        return container(
            container(
                row![
                    icon("lock-keyhole", t::ICON, t::TEXT_MUTED),
                    hint("You don't have permission to send messages in this channel.")
                ]
                .spacing(t::S12)
                .align_y(Center),
            )
            .width(Fill)
            .padding([t::S12, t::S16])
            .style(t::elevated),
        )
        .id("composer")
        .padding([t::S8, t::S16])
        .style(t::chat)
        .into();
    }
    let mut body = column![].spacing(t::S8);
    if let Some(cards) = uploads::cards(app) {
        body = body.push(cards);
    }
    if let Some(completion) = emoji_picker::autocomplete(app) {
        body = body.push(completion);
    }
    let raw = app.editor.text();
    // Nonce-keyed optimistic previews disappear only when the correlated
    // response arrives; real message IDs are never fabricated or duplicated.
    let pending: Vec<_> = app
        .pending_messages()
        .map(|(nonce, draft)| {
            (
                nonce,
                container(
                    row![
                        icon("send", t::INLINE_ICON, t::TEXT_MUTED),
                        column![
                            hint("Sending…"),
                            txt(if draft.content.is_empty() {
                                "Attachment message".into()
                            } else {
                                ellipsis(&draft.content, 120)
                            })
                            .style(t::secondary_text)
                        ]
                        .spacing(t::S4)
                    ]
                    .spacing(t::S12)
                    .align_y(Center),
                )
                .padding([t::S8, t::S12])
                .style(t::elevated)
                .into(),
            )
        })
        .collect();
    if !pending.is_empty() {
        body = body.push(
            container(
                scroller(widget::keyed_column(pending).spacing(t::S4)).height(Length::Shrink),
            )
            .max_height(t::ATTACHMENT_CARD)
            .style(t::bare),
        );
    }
    if let Some(id) = app.state.reply {
        body = body.push(
            container(
                row![
                    icon("reply", t::INLINE_ICON, t::TEXT_SECONDARY),
                    hint(format!(
                        "Replying to {}",
                        app.state
                            .model
                            .messages
                            .get(&channel.id)
                            .and_then(|ms| ms.get(&id))
                            .map_or("a deleted message".into(), |m| app
                                .state
                                .model
                                .name(m.author_id))
                    )),
                    space().width(Fill),
                    icon_button("x", "Cancel reply", Msg::CancelCompose)
                ]
                .spacing(t::S8)
                .align_y(Center),
            )
            .padding([t::S4, t::S12])
            .style(t::elevated),
        );
    }
    let candidates = app.mention_candidates();
    if !candidates.is_empty() {
        let mut suggestions = column![section("MENTION SOMEONE")].spacing(t::S4);
        for (i, id) in candidates.iter().enumerate() {
            if let Some(user) = app.state.model.users.get(id) {
                suggestions = suggestions.push(
                    button(
                        row![
                            avatar(app, *id, t::S24, false),
                            elided::label(app.state.model.name(*id)).width(Fill),
                            elided::label(format!("@{}", user.username))
                                .size(t::HINT)
                                .color(t::TEXT_SECONDARY)
                                .width(Fill)
                        ]
                        .spacing(t::S12)
                        .align_y(Center),
                    )
                    .on_press(Msg::Mention(*id))
                    .padding([t::S4, t::S8])
                    .height(t::AUTOCOMPLETE)
                    .width(Fill)
                    .style(t::navigation(i == app.suggestion)),
                );
            }
        }
        body = body.push(
            container(
                scroller(suggestions)
                    .id("composer-suggestions")
                    .height(Length::Shrink),
            )
            .max_height((app.height * 0.4).min(t::S32 * 10.))
            .padding(t::S8)
            .style(t::elevated),
        );
    }
    if !app.state.attachments.is_empty() {
        let mut attachments = Row::new().spacing(t::S8);
        for a in &app.state.attachments {
            attachments = attachments.push(
                container(
                    column![
                        row![
                            icon("file", t::ICON, t::TEXT_SECONDARY),
                            space().width(Fill),
                            icon_button("x", "Remove attachment", Msg::RemoveUpload(a.id))
                        ]
                        .align_y(Center),
                        hint(ellipsis(&a.filename, 12))
                    ]
                    .spacing(t::S8),
                )
                .width(t::ATTACHMENT_CARD)
                .height(t::ATTACHMENT_CARD)
                .padding(t::S8)
                .style(t::elevated),
            );
        }
        body = body.push(attachments.wrap());
    }
    if app.state.editing.is_some() {
        body = body.push(
            container(
                row![
                    hint("Editing a message above"),
                    space().width(Fill),
                    action("Cancel", Msg::CancelCompose).style(t::link)
                ]
                .align_y(Center),
            )
            .padding(t::S8)
            .style(t::elevated),
        );
    } else {
        let placeholder = format!(
            "Message {}{}",
            if channel.kind == ChannelKind::Text {
                "#"
            } else {
                "@"
            },
            app.state.model.channel_name(channel)
        );
        let wrap =
            ((app.viewport_w - t::MESSAGE_GUTTER - t::S32) / t::GLYPH_WIDTH).max(1.) as usize;
        let lines = raw
            .lines()
            .map(|line| line.chars().count().max(1).div_ceil(wrap))
            .sum::<usize>()
            .max(1);
        // History height shrinks as the composer grows. Basing this limit on
        // that feedback signal eventually inverted clamp's bounds at 940×600.
        let max_height = ((app.height - t::HEADER - t::TYPING - t::S32) * 0.5).max(t::COMPOSER_MIN);
        let height = (lines as f32 * t::BODY * t::LINE + t::S24).clamp(t::COMPOSER_MIN, max_height);
        let editor = compose_editor(app, placeholder, height, !candidates.is_empty());
        body = body.push(
            container(
                row![
                    if permissions.contains(Permissions::ATTACH_FILES) {
                        icon_button(
                            "plus-circle",
                            "Upload a file",
                            Msg::PickUpload(UploadPurpose::Message),
                        )
                    } else {
                        space().width(0).into()
                    },
                    editor,
                    if !matches!(app.features.gifs.capability, GifCapability::Disabled) {
                        controls::gif_button()
                    } else {
                        space().width(0).into()
                    },
                    icon_button("smile", "Choose an emoji", Msg::Open(Dialog::EmojiPicker))
                ]
                .spacing(t::S12)
                .align_y(Center),
            )
            .padding([t::S12, t::S16])
            .style(t::elevated),
        );
    }
    let names: Vec<_> = app
        .state
        .model
        .typing
        .keys()
        .filter(|(c, _)| *c == channel.id)
        .map(|(_, u)| app.state.model.name(*u))
        .collect();
    let typing = match names.as_slice() {
        [] => {
            if app.uploading > 0 {
                "Uploading attachment…".into()
            } else {
                String::new()
            }
        }
        [a] => format!("{a} is typing…"),
        [a, b] => format!("{a} and {b} are typing…"),
        _ => "Several people are typing…".into(),
    };
    let dots: Element<'_, Msg> = if typing.is_empty() {
        space().width(0).into()
    } else {
        row![
            dot(t::TEXT_MUTED, t::S4),
            dot(t::TEXT_MUTED, t::S4),
            dot(t::TEXT_MUTED, t::S4)
        ]
        .spacing(t::S4)
        .into()
    };
    body = body.push(
        container(
            row![
                dots,
                elided::label(typing)
                    .size(t::HINT)
                    .color(t::TEXT_MUTED)
                    .width(Fill)
            ]
            .spacing(t::S8)
            .align_y(Center),
        )
        .height(t::TYPING)
        .padding([0., t::S8])
        .style(t::bare),
    );
    container(body)
        .id("composer")
        .padding(Padding {
            top: t::S8,
            right: t::S16,
            bottom: 0.,
            left: t::S16,
        })
        .style(t::chat)
        .into()
}
fn member_panel<'a>(app: &'a App, channel: &'a ChannelSummary) -> Element<'a, Msg> {
    members::view(app, channel)
}
fn message_skeleton<'a>(height: f32) -> Element<'a, Msg> {
    let bar = |width| {
        container(space())
            .width(width)
            .height(t::S8)
            .style(|_| container::Style {
                border: Border {
                    radius: t::S4.into(),
                    ..Default::default()
                },
                ..t::surface(t::BG_ELEVATED)
            })
    };
    container(
        row![
            container(space())
                .width(t::AVATAR)
                .height(t::AVATAR)
                .style(|_| container::Style {
                    border: Border {
                        radius: (t::AVATAR / 2.).into(),
                        ..Default::default()
                    },
                    ..t::surface(t::BG_ELEVATED)
                }),
            column![bar(180.), bar(280.)].spacing(t::S8)
        ]
        .spacing(t::S16),
    )
    .padding([t::S8, t::S16])
    .height(height)
    .width(Fill)
    .clip(true)
    .style(t::bare)
    .into()
}
fn message_row<'a>(app: &'a App, m: &'a Message, _height: f32, grouped: bool) -> Element<'a, Msg> {
    let compact =
        app.settings.preferences.density == sticktalk_client_core::settings::Density::Compact;
    let hovered = app.hovered == Some(m.id);
    let mentioned = app.me().is_some_and(|id| m.mention_ids.contains(&id));
    let mut content = column![].spacing(t::S4).width(Fill);
    if let Some(reply) = m.reply_to {
        let target = app
            .state
            .model
            .messages
            .get(&m.channel_id)
            .and_then(|ms| ms.get(&reply));
        let mut line = row![icon("reply", t::INLINE_ICON, t::TEXT_MUTED)]
            .spacing(t::S8)
            .align_y(Center);
        if let Some(target) = target {
            line = line
                .push(link_context::wrap(
                    button(avatar(app, target.author_id, t::TINY_AVATAR, false))
                        .on_press(controls::profile_message(target.author_id))
                        .padding(0)
                        .style(t::icon),
                ))
                .push(link_context::wrap(
                    button(
                        hint(ellipsis(&app.state.model.name(target.author_id), 20))
                            .color(controls::name_color(app, target.author_id)),
                    )
                    .on_press(controls::profile_message(target.author_id))
                    .padding(0)
                    .style(t::icon),
                ))
                .push(
                    button(
                        elided::label(app.state.model.content(target))
                            .size(t::HINT)
                            .color(t::TEXT_MUTED)
                            .width(Fill),
                    )
                    .on_press(Msg::PreviewReply(reply))
                    .padding(0)
                    .width(Fill)
                    .style(t::icon),
                );
        } else {
            line = line.push(
                button(hint("Original message deleted or unavailable"))
                    .on_press(Msg::PreviewReply(reply))
                    .padding(0)
                    .style(t::icon),
            );
        }
        content = content.push(line);
    }
    if !grouped {
        content = content.push(
            row![
                link_context::wrap(
                    button(
                        container(
                            elided::label(app.state.model.name(m.author_id))
                                .font(t::medium())
                                .size(t::CHANNEL_NAME)
                                .color(controls::name_color(app, m.author_id))
                        )
                        .max_width(
                            (app.viewport_w - t::MESSAGE_GUTTER - t::SEARCH_REST - t::S16)
                                .max(t::S32)
                        )
                    )
                    .on_press(controls::profile_message(m.author_id))
                    .padding(0)
                    .width(Length::Shrink)
                    .style(t::icon)
                ),
                hint(timestamp(m.created_at, false))
            ]
            .spacing(t::S8)
            .align_y(Center),
        );
    }
    if app.state.editing == Some(m.id) {
        content = content
            .push(
                container(compose_editor(
                    app,
                    String::new(),
                    t::COMPOSER_MIN + t::S24,
                    false,
                ))
                .padding(t::S12)
                .style(t::elevated),
            )
            .push(hint("escape to cancel • enter to save"));
    } else if !m.content.is_empty() {
        content =
            content.push(gif_picker::inline(app, m).unwrap_or_else(|| linked_content(app, m)));
    }
    let mut attachments = column![].spacing(t::S8);
    for a in &m.attachments {
        let preview: Element<'_, Msg> = if crate::video::is_video(a) {
            video::view(app, &crate::video::attachment_key(a), false)
        } else if a.thumbnail.is_some() {
            let (w, h) = thumb_size(a);
            let image: Element<'_, Msg> = if let Some(handle) = app.images.get(a.id) {
                image(handle.clone())
                    .width(w)
                    .height(h)
                    .border_radius(t::S8)
                    .content_fit(iced::ContentFit::Contain)
                    .into()
            } else {
                container(icon("image", t::ICON, t::TEXT_MUTED))
                    .width(w)
                    .height(h)
                    .center_x(w)
                    .center_y(h)
                    .style(t::elevated)
                    .into()
            };
            button(image)
                .on_press(Msg::feature(crate::features::Event::ViewerOpen(a.clone())))
                .padding(0)
                .style(t::icon)
                .into()
        } else {
            button(
                row![
                    icon("file", t::S32, t::TEXT_SECONDARY),
                    column![
                        txt(ellipsis(&a.filename, 36)),
                        hint(format!("{:.1} KB", a.byte_size as f64 / 1024.))
                    ]
                    .spacing(t::S4),
                    icon("download", t::ICON, t::TEXT_SECONDARY)
                ]
                .spacing(t::S12)
                .align_y(Center),
            )
            .on_press(Msg::Download(a.clone()))
            .padding(t::S12)
            .style(t::button)
            .into()
        };
        attachments = attachments.push(preview);
    }
    if !m.attachments.is_empty() {
        content = content.push(
            container(attachments)
                .padding(Padding {
                    top: t::S4,
                    ..Padding::ZERO
                })
                .style(t::bare),
        );
    }
    if !m.reactions.is_empty() {
        let mut reactions = Row::new().spacing(t::S4);
        for r in m
            .reactions
            .iter()
            .filter(|r| emoji_picker::has_artwork(&r.emoji))
        {
            reactions = reactions.push(
                button(
                    row![
                        emoji(&r.emoji, t::INLINE_ICON),
                        hint(r.count.to_string()).color(if r.me {
                            t::ACCENT
                        } else {
                            t::TEXT_SECONDARY
                        })
                    ]
                    .spacing(t::S8)
                    .align_y(Center),
                )
                .on_press_maybe(
                    app.channel_permissions(m.channel_id)
                        .contains(Permissions::ADD_REACTIONS)
                        .then(|| Msg::Reaction(m.id, r.emoji.clone(), r.me)),
                )
                .padding([t::S4, t::S8])
                .style(t::reaction(r.me)),
            );
        }
        if hovered
            && app
                .channel_permissions(m.channel_id)
                .contains(Permissions::ADD_REACTIONS)
        {
            reactions = reactions.push(
                button(icon("plus", t::INLINE_ICON, t::TEXT_SECONDARY))
                    .on_press(Msg::ReactPicker(m.id))
                    .padding(t::S4)
                    .style(t::icon),
            );
        }
        content = content.push(reactions.wrap());
    }
    let gutter: Element<'_, Msg> = if grouped || compact {
        container(hint(if hovered || compact {
            timestamp(m.created_at, true)
        } else {
            String::new()
        }))
        .width(t::AVATAR)
        .style(t::bare)
        .into()
    } else {
        link_context::wrap(
            button(avatar(app, m.author_id, t::AVATAR, false))
                .on_press(Msg::Open(Dialog::Profile(m.author_id)))
                .padding(0)
                .style(t::icon),
        )
    };
    let (date, new) = app.row_decorations(m);
    let top = if compact {
        t::S4
    } else if grouped {
        t::CONTINUATION
    } else {
        t::S16
    };
    let inner = container(row![gutter, content].spacing(t::S16))
        .padding(Padding {
            top,
            right: t::S16,
            bottom: t::CONTINUATION,
            left: t::S16,
        })
        .width(Fill)
        .style(move |_| t::message(hovered, mentioned));
    let mut layers = vec![inner.into()];
    if mentioned {
        layers.push(
            container(
                container(space())
                    .width(t::FOCUS)
                    .height(Fill)
                    .style(|_| t::surface(t::WARNING)),
            )
            .height(Fill)
            .style(t::bare)
            .into(),
        );
    }
    if hovered {
        let permissions = app.channel_permissions(m.channel_id);
        let mut tools = Row::new().spacing(0);
        if permissions.contains(Permissions::ADD_REACTIONS) {
            tools = tools.push(message_tool("smile", "React", Msg::ReactPicker(m.id)));
        }
        if permissions.contains(Permissions::SEND_MESSAGES) {
            tools = tools.push(message_tool("reply", "Reply", Msg::Reply(m.id)));
        }
        if Some(m.author_id) == app.me() && permissions.contains(Permissions::SEND_MESSAGES) {
            tools = tools.push(message_tool("pencil", "Edit", Msg::Edit(m.id)));
        }
        if Some(m.author_id) == app.me()
            || app
                .channel_permissions(m.channel_id)
                .contains(Permissions::MANAGE_MESSAGES)
        {
            tools = tools.push(message_tool(
                "trash-2",
                "Delete",
                Msg::Open(Dialog::Confirm(
                    "Delete this message? This cannot be undone.".into(),
                    Command::DeleteMessage {
                        channel_id: m.channel_id,
                        message_id: m.id,
                    },
                )),
            ));
        }
        tools = tools.push(message_tool(
            "more-horizontal",
            "More",
            Msg::Open(Dialog::MessageActions(m.id)),
        ));
        layers.push(
            container(
                widget::float(
                    mouse_area(container(tools).style(|_| container::Style {
                        border: Border {
                            color: t::DIVIDER,
                            width: t::HAIRLINE,
                            radius: t::S8.into(),
                        },
                        ..t::surface(t::BG_ELEVATED)
                    }))
                    .on_enter(Msg::Hover(Some(m.id)))
                    .on_exit(Msg::HoverLeave(m.id)),
                )
                .translate(move |_, _| {
                    iced::Vector::new(0., if grouped { -t::S32 } else { -t::S16 })
                })
                .style(|_| widget::float::Style::default()),
            )
            .align_right(Fill)
            .padding([0., t::S16])
            .style(t::bare)
            .into(),
        );
    }
    let message: Element<'_, Msg> = observe::anchor(
        mouse_area(stack(layers))
            .on_enter(Msg::Hover(Some(m.id)))
            .on_exit(Msg::HoverLeave(m.id))
            .on_right_press(Msg::Open(Dialog::MessageActions(m.id))),
    );
    let mut full = column![].spacing(0);
    if date {
        full = full.push(
            container(
                row![
                    line(),
                    hint(
                        crate::datetime::label(m.created_at, false)
                            .split(" at ")
                            .next()
                            .unwrap_or("")
                            .to_owned()
                    ),
                    line()
                ]
                .spacing(t::S16)
                .align_y(Center),
            )
            .height(t::S32)
            .padding([0., t::S16])
            .style(t::bare),
        );
    }
    if new {
        full = full.push(
            container(
                row![
                    rule::horizontal(t::HAIRLINE).style(t::new_divider),
                    hint("NEW").color(t::DANGER)
                ]
                .spacing(t::S8)
                .align_y(Center),
            )
            .height(t::S24)
            .padding([0., t::S16])
            .style(t::bare),
        );
    }
    let id = m.id;
    let generation = app.state.generation;
    let width = app.viewport_w.to_bits();
    let seq = app.row_revision(m);
    let editing = app.state.editing == Some(id);
    widget::sensor(
        container(full.push(message))
            .id(format!("message-{}", id.0))
            .width(Fill),
    )
    .key((id, generation, width, seq, editing))
    .on_show(move |size| Msg::Measured {
        message: id,
        generation,
        width,
        seq,
        editing,
        height: size.height,
    })
    .on_resize(move |size| Msg::Measured {
        message: id,
        generation,
        width,
        seq,
        editing,
        height: size.height,
    })
    .into()
}
fn thumb_size(a: &Attachment) -> (f32, f32) {
    if let Some(thumb) = &a.thumbnail {
        let scale = (t::THUMB_W.min(t::IMAGE_MAX_W) / thumb.width.max(1) as f32)
            .min(t::THUMB_H.min(t::IMAGE_MAX_H) / thumb.height.max(1) as f32)
            .min(1.);
        (thumb.width as f32 * scale, thumb.height as f32 * scale)
    } else {
        (t::THUMB_W, t::THUMB_H)
    }
}
fn timestamp(ms: u64, short: bool) -> String {
    if short {
        let parts = crate::datetime::local(ms);
        return format!("{:02}:{:02}", parts.hour, parts.minute);
    }
    crate::datetime::label(ms, false)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RichLink {
    Url(String, bool),
    Spoiler(Id, usize),
    Profile(Id),
}
fn inline_content<'a>(
    app: &App,
    m: &Message,
    values: &[crate::markdown::Inline],
    size: f32,
) -> Element<'a, Msg> {
    inline_content_edited(app, m, values, size, false)
}
fn inline_content_edited<'a>(
    app: &App,
    m: &Message,
    values: &[crate::markdown::Inline],
    size: f32,
    edited: bool,
) -> Element<'a, Msg> {
    use unicode_segmentation::UnicodeSegmentation;
    let is_emoji = |g: &str| {
        g.chars().next().is_some_and(|c| c as u32 >= 0x2300)
            && (emoji_picker::has_artwork(g) || sticktalk_emoji::emoji_to_shortcode(g).is_some())
    };
    if !values
        .iter()
        .any(|v| !v.style.code && v.text.graphemes(true).any(is_emoji))
    {
        return inline_text_edited(app, m, values, size, edited);
    }
    // Keep styled words and artwork in the same wrapping flow. Text-only
    // messages retain the original single cached rich paragraph.
    let mut flow = Row::new().spacing(0).align_y(Center);
    for value in values {
        let mut buffer = String::new();
        let hidden = value
            .spoiler
            .is_some_and(|i| !app.revealed.contains(&(m.id, i)));
        for grapheme in value.text.graphemes(true) {
            let picture = !value.style.code && !hidden && is_emoji(grapheme);
            if picture || grapheme.chars().all(char::is_whitespace) {
                if !picture {
                    buffer.push_str(grapheme);
                }
                if !buffer.is_empty() {
                    let mut piece = value.clone();
                    piece.text = std::mem::take(&mut buffer);
                    flow = flow.push(inline_text(app, m, &[piece], size));
                }
                if picture && emoji_picker::has_artwork(grapheme) {
                    flow = flow.push(emoji_picker::glyph(
                        grapheme,
                        size * app.settings.preferences.chat_font_scale,
                    ));
                }
            } else {
                buffer.push_str(grapheme);
            }
        }
        if !buffer.is_empty() {
            let mut piece = value.clone();
            piece.text = buffer;
            flow = flow.push(inline_text(app, m, &[piece], size));
        }
    }
    if edited {
        flow = flow.push(hint(" (edited)"));
    }
    flow.width(Fill).wrap().into()
}
fn inline_text<'a>(
    app: &App,
    m: &Message,
    values: &[crate::markdown::Inline],
    size: f32,
) -> Element<'a, Msg> {
    inline_text_edited(app, m, values, size, false)
}
fn inline_text_edited<'a>(
    app: &App,
    m: &Message,
    values: &[crate::markdown::Inline],
    size: f32,
    edited: bool,
) -> Element<'a, Msg> {
    use crate::markdown::Target;
    let mut spans = Vec::new();
    for value in values {
        let mut label = value.text.clone();
        let mut font = if value.style.code {
            t::mono()
        } else if value.style.bold {
            t::semibold()
        } else {
            t::regular()
        };
        if value.style.italic && !value.style.code {
            font.style = iced::font::Style::Italic;
        }
        let mut span = widget::span(label.clone())
            .font(font)
            .underline(value.style.underline)
            .strikethrough(value.style.strike);
        if value.style.code {
            span = span
                .background(t::BG_ELEVATED)
                .border(Border {
                    radius: t::S4.into(),
                    ..Default::default()
                })
                .padding([0., t::S4]);
        }
        if let Some(target) = &value.target {
            match target {
                Target::Auto(url) => {
                    span = span
                        .color(t::ACCENT)
                        .underline(app.settings.preferences.underline_links)
                        .link(RichLink::Url(url.clone(), false))
                }
                Target::Masked(url) => {
                    span = span
                        .color(t::ACCENT)
                        .underline(app.settings.preferences.underline_links)
                        .link(RichLink::Url(url.clone(), true))
                }
                Target::Mention(id) if m.mention_ids.contains(id) => {
                    label = format!("@{}", app.state.model.name(*id));
                    span = widget::span(label)
                        .font(font)
                        .color(controls::mention_color(app, *id))
                        .link(RichLink::Profile(*id))
                        .background(t::alpha(controls::mention_color(app, *id), 0.20))
                        .border(Border {
                            radius: t::S4.into(),
                            ..Default::default()
                        })
                        .padding([0., t::FOCUS]);
                }
                _ => {}
            }
        }
        if let Some(index) = value.spoiler {
            if !app.revealed.contains(&(m.id, index)) {
                span = span
                    .color(t::BG_SELECTED)
                    .background(t::BG_SELECTED)
                    .link(RichLink::Spoiler(m.id, index))
                    .underline(false)
                    .strikethrough(false);
            }
        }
        spans.push(span);
    }
    if edited {
        spans.push(
            widget::span(" (edited)")
                .size(t::HINT)
                .font(t::regular())
                .color(t::TEXT_MUTED),
        );
    }
    link_context::wrap(
        widget::rich_text(spans)
            .font(t::regular())
            .size(size * app.settings.preferences.chat_font_scale)
            .line_height(t::LINE)
            .style(t::primary_text)
            .on_link_click(|link| match link {
                RichLink::Url(url, true) => Msg::Link(url),
                RichLink::Url(url, false) => Msg::OpenLink(url),
                RichLink::Spoiler(id, i) => Msg::Reveal(id, i),
                RichLink::Profile(id) => controls::profile_message(id),
            }),
    )
}
fn linked_content<'a>(app: &'a App, m: &Message) -> Element<'a, Msg> {
    use crate::markdown::Block;
    let doc = app.markdown.get(m.id, &m.content);
    let mut body = column![].spacing(t::S4).width(Fill);
    for (index, block) in doc.blocks.iter().enumerate() {
        body = body.push(match block {
            Block::Paragraph(v) => inline_content_edited(
                app,
                m,
                v,
                t::BODY,
                m.edited_at.is_some() && index + 1 == doc.blocks.len(),
            ),
            Block::Header(n, v) => inline_content(
                app,
                m,
                v,
                match n {
                    1 => t::TITLE,
                    2 => t::MARKDOWN_H2,
                    _ => t::MARKDOWN_H3,
                },
            ),
            Block::List(v) => row![txt("•"), inline_content(app, m, v, t::BODY)]
                .spacing(t::S8)
                .padding([0., t::S8])
                .into(),
            Block::Quote(v) => row![
                container(space())
                    .width(t::S4)
                    .height(t::BODY * t::LINE + t::S8)
                    .style(|_| t::surface(t::DIVIDER)),
                container(inline_content(app, m, v, t::BODY))
                    .padding([t::S4, t::S8])
                    .style(t::bare)
            ]
            .into(),
            Block::Code {
                language,
                text: code,
            } => {
                let mut code_body = column![].spacing(t::S4);
                if !language.is_empty() {
                    code_body = code_body.push(hint(language));
                }
                code_body = code_body.push(
                    crate::scroll::SmoothScrollable::new(
                        txt(code)
                            .size(t::BODY * app.settings.preferences.chat_font_scale)
                            .font(t::mono())
                            .wrapping(widget::text::Wrapping::None),
                    )
                    .direction(scrollable::Direction::Horizontal(
                        scrollable::Scrollbar::new()
                            .width(t::S4)
                            .scroller_width(t::S4),
                    ))
                    .style(t::scroll)
                    .horizontal(),
                );
                container(code_body)
                    .padding(t::S8)
                    .width(Fill)
                    .style(|_| container::Style {
                        border: Border {
                            color: t::DIVIDER,
                            width: t::HAIRLINE,
                            radius: t::S8.into(),
                        },
                        ..t::surface(t::INPUT_BG)
                    })
                    .into()
            }
        });
    }
    if m.edited_at.is_some() && !matches!(doc.blocks.last(), Some(Block::Paragraph(_))) {
        body = body.push(hint("(edited)"));
    }
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    m.content.hash(&mut hash);
    m.edited_at.hash(&mut hash);
    app.settings
        .preferences
        .chat_font_scale
        .to_bits()
        .hash(&mut hash);
    app.settings.preferences.underline_links.hash(&mut hash);
    for id in &m.mention_ids {
        app.state.model.name(*id).hash(&mut hash);
    }
    for value in app.revealed.iter().filter(|(id, _)| *id == m.id) {
        value.hash(&mut hash);
    }
    app.message_layouts.wrap(m.id, hash.finish(), body.into())
}
fn menu_item<'a>(name: &str, label: &str, message: Msg, danger: bool) -> Element<'a, Msg> {
    controls::item(label, name, message, danger)
}
fn context_menu(app: &App) -> Element<'_, Msg> {
    if matches!(app.dialog, Dialog::GuildMenu) {
        return menus::server(app);
    }
    let mut items = Column::new().spacing(t::S4);
    match &app.dialog {
        Dialog::GuildMenu => {
            if let Some(g) = app.state.guild {
                let p = app.permissions();
                if p.contains(Permissions::CREATE_INVITES) {
                    items = items.push(menu_item(
                        "user-plus",
                        "Invite people",
                        Msg::Open(Dialog::Invite),
                        false,
                    ));
                }
                items = items.push(menu_item(
                    "settings",
                    "Server settings",
                    Msg::Open(Dialog::GuildSettings(GuildTab::Overview)),
                    false,
                ));
                if p.contains(Permissions::MANAGE_CHANNELS) {
                    items = items.push(menu_item(
                        "plus",
                        "Create channel",
                        Msg::Open(Dialog::NewChannel(ChannelKind::Text, None)),
                        false,
                    ));
                }
                items = items.push(line()).push(menu_item(
                    "bell",
                    "Notification Settings",
                    Msg::Settings(crate::settings_controller::Action::Notifications(g)),
                    false,
                ));
                if Some(app.state.model.guilds[&g].guild.owner_id) != app.me() {
                    items = items.push(line()).push(menu_item(
                        "log-out",
                        "Leave server",
                        Msg::Open(Dialog::Confirm(
                            "Leave this server? Owners must transfer ownership first.".into(),
                            Command::LeaveGuild { guild_id: g },
                        )),
                        true,
                    ));
                }
            }
        }
        Dialog::MessageActions(id) => {
            if let Screen::Channel(channel_id) = app.state.screen {
                if let Some(m) = app
                    .state
                    .model
                    .messages
                    .get(&channel_id)
                    .and_then(|ms| ms.get(id))
                {
                    let permissions = app.channel_permissions(channel_id);
                    if permissions.contains(Permissions::ADD_REACTIONS) {
                        let mut reactions = Row::new().spacing(t::S4);
                        for value in ["🌱", "❤️", "👍", "✨", "🎉", "👀"] {
                            let reacted = m.reactions.iter().any(|r| r.emoji == value && r.me);
                            reactions = reactions.push(
                                button(emoji(value, t::ICON))
                                    .on_press(Msg::Reaction(*id, value.into(), reacted))
                                    .padding(t::S4)
                                    .style(t::reaction(reacted)),
                            );
                        }
                        reactions = reactions.push(
                            button(icon("plus", t::INLINE_ICON, t::TEXT_SECONDARY))
                                .on_press(Msg::ReactPicker(*id))
                                .padding(t::S4)
                                .style(t::icon),
                        );
                        items = items.push(reactions).push(line());
                    }
                    if permissions.contains(Permissions::SEND_MESSAGES) {
                        items = items.push(menu_item("reply", "Reply", Msg::Reply(*id), false));
                    }
                    if Some(m.author_id) == app.me()
                        && permissions.contains(Permissions::SEND_MESSAGES)
                    {
                        items = items.push(menu_item("pencil", "Edit", Msg::Edit(*id), false));
                    }
                    if app
                        .channel_permissions(channel_id)
                        .contains(Permissions::MANAGE_MESSAGES)
                    {
                        let pinned = app
                            .state
                            .model
                            .launch
                            .pins
                            .get(&channel_id)
                            .is_some_and(|pins| pins.iter().any(|m| m.id == *id));
                        items = items.push(controls::item(
                            if pinned {
                                "Unpin Message"
                            } else {
                                "Pin Message"
                            },
                            "pin",
                            crate::main_window::mm(crate::main_window::Message::Pin(
                                channel_id, *id, !pinned,
                            )),
                            false,
                        ));
                    }
                    if app.settings.preferences.developer_mode {
                        items = items.push(controls::item(
                            "Copy ID",
                            "copy",
                            Msg::Copy(id.0.to_string()),
                            false,
                        ));
                    }

                    items = items.push(menu_item(
                        "copy",
                        "Copy Text",
                        Msg::Copy(m.content.clone()),
                        false,
                    ));
                    items = items
                        .push(menu_item(
                            "link",
                            "Copy Message Link",
                            Msg::Copy(format!(
                                "{}/channels/{}/{}",
                                app.server.trim_end_matches('/'),
                                channel_id.0,
                                id.0
                            )),
                            false,
                        ))
                        .push(menu_item(
                            "message-circle",
                            "Mark Unread",
                            Msg::MarkUnread(*id),
                            false,
                        ));
                    if Some(m.author_id) == app.me()
                        || app
                            .channel_permissions(channel_id)
                            .contains(Permissions::MANAGE_MESSAGES)
                    {
                        items = items.push(line()).push(menu_item(
                            "trash-2",
                            "Delete",
                            Msg::Open(Dialog::Confirm(
                                "Delete this message? This cannot be undone.".into(),
                                Command::DeleteMessage {
                                    channel_id,
                                    message_id: *id,
                                },
                            )),
                            true,
                        ));
                    }
                }
            }
        }
        Dialog::MemberMenu(id) => {
            if app.calls.call.channel.is_some() {
                items = items.push(call::user_audio(app, *id));
            }
            if let Some(guild_id) = app.state.guild {
                if app.permissions().contains(Permissions::KICK_MEMBERS) {
                    items = items.push(menu_item(
                        "log-out",
                        "Kick",
                        Msg::Open(Dialog::Confirm(
                            "Kick this member?".into(),
                            Command::KickMember {
                                guild_id,
                                user_id: *id,
                            },
                        )),
                        true,
                    ));
                }
                if app.permissions().contains(Permissions::BAN_MEMBERS) {
                    items = items.push(menu_item(
                        "ban",
                        "Ban",
                        Msg::Open(Dialog::Confirm(
                            "Ban this member?".into(),
                            Command::BanMember {
                                guild_id,
                                user_id: *id,
                                reason: None,
                            },
                        )),
                        true,
                    ));
                }
            }
        }
        _ => {}
    }

    if app.settings.preferences.developer_mode {
        let id = match app.dialog {
            Dialog::MessageActions(id) | Dialog::MemberMenu(id) | Dialog::Profile(id) => Some(id),
            Dialog::GuildMenu => app.state.guild,
            _ => None,
        };
        if let Some(id) = id {
            items = items.push(line()).push(menu_item(
                "copy",
                "Copy ID",
                Msg::Copy(id.0.to_string()),
                false,
            ));
        }
    }
    menus::card(items)
}
fn modal(app: &App) -> Element<'_, Msg> {
    if matches!(app.dialog, Dialog::GuildSettings(_) | Dialog::UserSettings) {
        return settings(app, &app.dialog);
    }
    let (heading, sub) = match &app.dialog {
        Dialog::AddGuild => ("Add a server", ""),
        Dialog::CreateGuild => ("Create a server", ""),
        Dialog::JoinGuild => ("Join a server", "Enter an invite code."),
        Dialog::Invite => ("Invite people", ""),
        Dialog::AddFriend => (
            "Add a friend",
            "You can add a friend with their exact username.",
        ),
        Dialog::NewChannel(ChannelKind::Category, _) => ("Create a category", ""),
        Dialog::NewChannel(_, _) => ("Create a channel", ""),
        Dialog::Notifications(_) => return settings::notifications(app),
        Dialog::ChannelSettings(_) => ("Channel settings", ""),
        Dialog::NewGroup => (
            "New group message",
            "Choose two or more friends to get together.",
        ),
        Dialog::GroupSettings(_) => ("Group settings", ""),
        Dialog::Profile(_) => ("Profile", ""),
        Dialog::Confirm(_, _) => ("Confirm action", ""),
        Dialog::ExternalLink(_) => (
            "Open an external link?",
            "This will open in your default browser.",
        ),
        Dialog::ReplyPreview(_) => ("Original message", ""),
        Dialog::HistoryJump => (
            "Jump to message",
            "Enter a message ID or a date. Dates use UTC.",
        ),
        Dialog::Password => (
            if app.methods.as_ref().is_some_and(|m| m.has_password) {
                "Change password"
            } else {
                "Set password"
            },
            "",
        ),
        Dialog::EmojiPicker => ("Choose an emoji", ""),
        _ => ("", ""),
    };
    let mut body = column![
        row![
            title(heading),
            space().width(Fill),
            icon_button("x", "Close", Msg::Close)
        ]
        .align_y(Center),
        space().height(t::S8)
    ]
    .spacing(t::S16);
    if !sub.is_empty() {
        body = body.push(txt(sub).style(t::secondary_text));
    }
    match &app.dialog {
        Dialog::AddGuild => {
            for (name, heading, _sub, dialog) in [
                ("plus-circle", "Create a server", "", Dialog::CreateGuild),
                ("users", "Join a server", "", Dialog::JoinGuild),
            ] {
                body = body.push(
                    button(
                        row![
                            icon(name, t::S32, t::ACCENT),
                            column![txt(heading).font(t::semibold())]
                                .spacing(t::S8)
                                .width(Fill),
                            icon("chevron-right", t::ICON, t::TEXT_MUTED)
                        ]
                        .spacing(t::S16)
                        .align_y(Center),
                    )
                    .on_press(Msg::Open(dialog))
                    .padding(t::S24)
                    .width(Fill)
                    .style(t::choice),
                );
            }
        }
        Dialog::CreateGuild => {
            body = body.push(
                button(stack![
                    widget::svg(widget::svg::Handle::from_memory(
                        include_bytes!("../assets/upload-circle.svg").as_slice()
                    ))
                    .width(t::ATTACHMENT_CARD)
                    .height(t::ATTACHMENT_CARD),
                    container(hint("Upload"))
                        .center_x(t::ATTACHMENT_CARD)
                        .align_bottom(t::ATTACHMENT_CARD)
                        .padding(Padding {
                            bottom: t::S16,
                            ..Padding::ZERO
                        })
                        .style(t::bare)
                ])
                .on_press(Msg::PickUpload(UploadPurpose::GuildIcon))
                .padding(0)
                .style(t::icon),
            );
            if app.form.icon.is_some() {
                body = body.push(hint("Icon uploaded."));
            }
            body = body
                .push(field(
                    "SERVER NAME",
                    "e.g. The Common Room",
                    &app.form.name,
                    0,
                ))
                .push(footer("Create server", Msg::Submit));
        }
        Dialog::JoinGuild => {
            body = body
                .push(field(
                    "INVITE CODE",
                    "Paste your invite code",
                    &app.form.code,
                    2,
                ))
                .push(footer("Join server", Msg::Submit));
        }
        Dialog::Invite => {
            let ready = !app.form.code.is_empty();
            let retry = !app.error.is_empty();
            let code = if app.form.code.is_empty() {
                if retry {
                    "Invite unavailable"
                } else {
                    "Creating your invite…"
                }
                .into()
            } else {
                app.form.code.clone()
            };
            body = body
                .push(
                    container(
                        row![
                            icon("link", t::ICON, t::TEXT_MUTED),
                            txt(ellipsis(&code, 36)).width(Fill),
                            controls::icon_button_maybe(
                                "copy",
                                "Copy invite code",
                                ready.then(|| Msg::Copy(app.form.code.clone())),
                                false
                            )
                        ]
                        .spacing(t::S12)
                        .align_y(Center),
                    )
                    .padding(t::S16)
                    .style(t::input_container),
                )
                .push(hint("Valid for 7 days · Up to 100 uses"));
            if let Some(guild_id) = app.state.guild {
                body = body.push(
                    action(
                        if retry {
                            "Try again"
                        } else {
                            "Create a new invite"
                        },
                        Msg::Command(Command::CreateInvite {
                            guild_id,
                            ttl_seconds: 604800,
                            max_uses: 100,
                        }),
                    )
                    .on_press_maybe((ready || retry).then_some(Msg::Command(
                        Command::CreateInvite {
                            guild_id,
                            ttl_seconds: 604800,
                            max_uses: 100,
                        },
                    )))
                    .style(t::link),
                );
            }
        }
        Dialog::AddFriend => {
            body = body
                .push(field("USERNAME", "their_username", &app.form.name, 0))
                .push(footer("Send friend request", Msg::Submit));
        }
        Dialog::NewChannel(kind, parent) => {
            body = body
                .push(chip(
                    if *kind == ChannelKind::Category {
                        "CATEGORY"
                    } else {
                        "TEXT CHANNEL"
                    },
                    t::ACCENT,
                ))
                .push(field(
                    "NAME",
                    if *kind == ChannelKind::Category {
                        "e.g. Projects"
                    } else {
                        "e.g. little-wins"
                    },
                    &app.form.name,
                    0,
                ));
            if *kind == ChannelKind::Text {
                body = body.push(field(
                    "TOPIC (OPTIONAL)",
                    "What is this room for?",
                    &app.form.topic,
                    1,
                ));
            }
            if let Some(parent) = parent {
                body = body.push(hint(format!(
                    "Inside {}",
                    app.state
                        .model
                        .channels
                        .get(parent)
                        .map_or(String::new(), |v| app.state.model.channel_name(v))
                )));
            }
            body = body.push(footer(
                if *kind == ChannelKind::Category {
                    "Create category"
                } else {
                    "Create channel"
                },
                Msg::Submit,
            ));
        }
        Dialog::ChannelSettings(id) => {
            if let Some(c) = app.state.model.channels.get(id) {
                body = body.push(field("NAME", "Channel name", &app.form.name, 0));
                if c.kind == ChannelKind::Text {
                    body = body.push(field("TOPIC", "A short description", &app.form.topic, 1));
                }
                if app.permissions().contains(Permissions::MANAGE_CHANNELS) {
                    body=body.push(action("Delete channel",Msg::Open(Dialog::Confirm("Delete this channel and all its messages? This cannot be undone.".into(),Command::DeleteChannel{channel_id:*id}))).style(t::danger_link)).push(footer("Save changes",Msg::Submit));
                } else {
                    body = body.push(hint("Only server moderators can change this channel."));
                }
            }
        }
        Dialog::NewGroup => {
            body = body.push(field(
                "GROUP NAME",
                "e.g. Weekend makers",
                &app.form.name,
                0,
            ));
            for r in app
                .state
                .model
                .relationships
                .values()
                .filter(|r| r.kind == RelationshipKind::Friend)
            {
                let selected = app.form.selected.contains(&r.user.id);
                body = body.push(
                    button(
                        row![
                            avatar(app, r.user.id, t::SMALL_AVATAR, true),
                            txt(app.state.model.name(r.user.id)).width(Fill),
                            icon(
                                if selected { "check" } else { "plus" },
                                t::INLINE_ICON,
                                if selected { t::ACCENT } else { t::TEXT_MUTED }
                            )
                        ]
                        .spacing(t::S12)
                        .align_y(Center),
                    )
                    .on_press(Msg::ToggleUser(r.user.id))
                    .padding(t::S8)
                    .width(Fill)
                    .style(t::navigation(selected)),
                );
            }
            body = body.push(
                row![
                    space().width(Fill),
                    action("Cancel", Msg::Close),
                    primary(
                        format!("Create group · {} selected", app.form.selected.len()),
                        Msg::Submit,
                    )
                    .on_press_maybe(if app.form.selected.len() >= 2 {
                        Some(Msg::Submit)
                    } else {
                        None
                    })
                ]
                .spacing(t::S8),
            );
        }
        Dialog::GroupSettings(id) => {
            if let Some(c) = app.state.model.channels.get(id) {
                body = body.push(field("GROUP NAME", "Group name", &app.form.name, 0));
                let owner = c.owner_id == app.me();
                if owner {
                    body = body.push(footer("Save name", Msg::Submit));
                }
                body = body.push(section("MEMBERS"));
                for user_id in &c.recipient_ids {
                    let mut member = row![
                        avatar(app, *user_id, t::SMALL_AVATAR, true),
                        txt(app.state.model.name(*user_id)).width(Fill)
                    ]
                    .spacing(t::S12)
                    .align_y(Center);
                    if c.owner_id == Some(*user_id) {
                        member = member.push(chip("OWNER", t::ACCENT));
                    } else if owner {
                        member = member
                            .push(icon_button(
                                "shield",
                                "Make owner",
                                Msg::Open(Dialog::Confirm(
                                    "Transfer group ownership?".into(),
                                    Command::TransferGroupOwnership {
                                        channel_id: *id,
                                        user_id: *user_id,
                                    },
                                )),
                            ))
                            .push(icon_button(
                                "user-minus",
                                "Remove member",
                                Msg::Command(Command::RemoveGroupRecipient {
                                    channel_id: *id,
                                    user_id: *user_id,
                                }),
                            ));
                    }
                    body = body.push(member);
                }
                if owner {
                    body = body.push(section("ADD FRIENDS"));
                    for r in app.state.model.relationships.values().filter(|r| {
                        r.kind == RelationshipKind::Friend && !c.recipient_ids.contains(&r.user.id)
                    }) {
                        body = body.push(
                            action(
                                app.state.model.name(r.user.id),
                                Msg::Command(Command::AddGroupRecipient {
                                    channel_id: *id,
                                    user_id: r.user.id,
                                }),
                            )
                            .style(t::link),
                        );
                    }
                }
                body = body.push(line()).push(
                    action(
                        "Leave group",
                        Msg::Open(Dialog::Confirm(
                            "Leave this group? Owners must transfer ownership first.".into(),
                            Command::LeaveGroupDm { channel_id: *id },
                        )),
                    )
                    .style(t::danger_link),
                );
            }
        }
        Dialog::Profile(id) => {
            body = body.push(
                row![
                    avatar(app, *id, t::MODAL_INSET, true),
                    column![
                        title(app.state.model.name(*id)),
                        hint(
                            app.state
                                .model
                                .users
                                .get(id)
                                .map_or(String::new(), |u| format!("@{}", u.username))
                        ),
                        hint(
                            app.state
                                .model
                                .users
                                .get(id)
                                .and_then(|u| u.status_text.clone())
                                .unwrap_or_default()
                        )
                    ]
                    .spacing(t::S8)
                ]
                .spacing(t::S16)
                .align_y(Center),
            );
            if Some(*id) != app.me() {
                body = body.push(
                    row![
                        primary("Message", Msg::Command(Command::OpenDm { user_id: *id })),
                        action(
                            "Remove friend",
                            Msg::Open(Dialog::Confirm(
                                "Remove this person from your friends?".into(),
                                Command::RemoveFriend { user_id: *id }
                            ))
                        )
                        .style(t::link),
                        icon_button(
                            "ban",
                            "Block",
                            Msg::Open(Dialog::Confirm(
                                "Block this person? Direct messaging and friendship will end."
                                    .into(),
                                Command::BlockUser { user_id: *id }
                            ))
                        )
                    ]
                    .spacing(t::S8),
                );
            }
            if let (Some(guild_id), Some(m)) = (app.state.guild, app.state.model.members.get(id)) {
                let p = app.permissions();
                let my_role = app.state.model.guilds.get(&guild_id).map(|g| g.my_role);
                let lower = match my_role {
                    Some(MemberRole::Owner) => m.role != MemberRole::Owner,
                    Some(MemberRole::Admin) => m.role == MemberRole::Member,
                    _ => false,
                };
                if lower && Some(*id) != app.me() {
                    body = body.push(line()).push(section("SERVER MODERATION"));
                    if p.contains(Permissions::KICK_MEMBERS) {
                        body = body.push(
                            action(
                                "Kick member",
                                Msg::Open(Dialog::Confirm(
                                    "Remove this member from the server?".into(),
                                    Command::KickMember {
                                        guild_id,
                                        user_id: *id,
                                    },
                                )),
                            )
                            .style(t::danger_link),
                        );
                    }
                    if p.contains(Permissions::BAN_MEMBERS) {
                        body = body
                            .push(field(
                                "BAN REASON (OPTIONAL)",
                                "A short moderation note",
                                &app.form.reason,
                                3,
                            ))
                            .push(
                                action(
                                    "Ban member",
                                    Msg::Open(Dialog::Confirm(
                                        "Ban this member from the server?".into(),
                                        Command::BanMember {
                                            guild_id,
                                            user_id: *id,
                                            reason: if app.form.reason.is_empty() {
                                                None
                                            } else {
                                                Some(app.form.reason.clone())
                                            },
                                        },
                                    )),
                                )
                                .style(t::danger_link),
                            );
                    }
                    if p.contains(Permissions::OWNER) {
                        body=body.push(row![action("Make admin",Msg::Command(Command::SetMemberRole{guild_id,user_id:*id,role:MemberRole::Admin})),action("Make member",Msg::Command(Command::SetMemberRole{guild_id,user_id:*id,role:MemberRole::Member}))].spacing(t::S8)).push(action("Transfer ownership",Msg::Open(Dialog::Confirm("Make this person the server owner? You will become an admin.".into(),Command::TransferGuildOwnership{guild_id,user_id:*id}))).style(t::link));
                    }
                }
            }
            if let Screen::Channel(channel_id) = app.state.screen {
                if app
                    .state
                    .model
                    .channels
                    .get(&channel_id)
                    .is_some_and(|c| c.kind == ChannelKind::Direct)
                {
                    body = body.push(
                        action(
                            "Close direct message",
                            Msg::Confirm(Command::CloseDm { channel_id }),
                        )
                        .style(t::danger_link),
                    );
                }
            }
        }
        Dialog::Confirm(label, command) => {
            body = body.push(txt(label)).push(
                row![
                    space().width(Fill),
                    action("Cancel", Msg::Close),
                    action("Confirm", Msg::Confirm(command.clone())).style(t::danger)
                ]
                .spacing(t::S12),
            );
        }
        Dialog::ExternalLink(url) => {
            body = body.push(txt(url).style(t::accent_text)).push(
                row![
                    space().width(Fill),
                    action("Cancel", Msg::Close),
                    primary("Open link", Msg::OpenLink(url.clone()))
                ]
                .spacing(t::S12),
            );
        }
        Dialog::HistoryJump => {
            body = body
                .push(
                    text_input("YYYY-MM-DD or message ID", &app.form.name)
                        .on_input(|value| Msg::Field(0, value))
                        .on_submit(Msg::Submit)
                        .padding(t::S12)
                        .size(t::BODY)
                        .style(t::input),
                )
                .push(footer("Jump", Msg::Submit));
        }
        Dialog::ReplyPreview(id) => {
            if let Some(m) = &app.preview {
                body = body
                    .push(txt(app.state.model.name(m.author_id)).font(t::semibold()))
                    .push(linked_content(app, m))
                    .push(action("Jump to message", Msg::JumpTo(*id)).style(t::link));
            } else {
                body = body.push(hint(format!(
                    "Loading message {}. It may have been deleted.",
                    id.0
                )));
            }
        }
        Dialog::Password => {
            if app.methods.as_ref().is_some_and(|m| !m.has_password) {
                body=body.push(hint("Saving requires a fresh Google sign-in. Choose your existing linked Google account."));
            }
            if app.methods.as_ref().is_some_and(|m| m.has_password) {
                body = body.push(
                    column![
                        section("CURRENT PASSWORD"),
                        text_input("", &app.form.name)
                            .secure(true)
                            .on_input(|v| Msg::Field(0, v))
                            .padding(t::S12)
                            .size(t::BODY)
                            .style(t::input)
                    ]
                    .spacing(t::S8),
                );
            }
            body = body
                .push(
                    column![
                        section("NEW PASSWORD"),
                        text_input("", &app.form.topic)
                            .secure(true)
                            .on_input(|v| Msg::Field(1, v))
                            .padding(t::S12)
                            .size(t::BODY)
                            .style(t::input),
                        hint("At least 12 characters.")
                    ]
                    .spacing(t::S8),
                )
                .push(
                    row![
                        space().width(Fill),
                        action("Cancel", Msg::Close),
                        primary("Save password", Msg::SavePassword)
                            .on_press_maybe((!app.busy).then_some(Msg::SavePassword))
                    ]
                    .spacing(t::S8),
                );
        }
        Dialog::EmojiPicker => {
            let mut choices = Row::new().spacing(t::S8);
            for value in ["🌱", "❤️", "👍", "✨", "🎉", "👀", "🔥", "🙌", "👋"] {
                choices = choices.push(
                    button(emoji(value, t::S24))
                        .on_press(Msg::Emoji(value.into()))
                        .padding(t::S8)
                        .style(t::icon),
                );
            }
            body = body
                .push(choices)
                .push(hint("Emoji artwork: Twemoji (CC BY 4.0)."));
        }
        _ => {}
    }
    if !app.error.is_empty() {
        body = body.push(hint(&app.error).color(t::DANGER));
    }
    if app.uploading > 0 {
        body = body.push(hint("Uploading…"));
    }
    container(scroller(body.padding(t::S24)).height(Length::Shrink))
        .width(t::MODAL_W)
        .max_height((app.height - t::MODAL_INSET).max(t::SIDEBAR))
        .style(t::modal)
        .into()
}
fn settings<'a>(app: &'a App, page: &Dialog) -> Element<'a, Msg> {
    settings::view(app, page)
}

fn compose_editor<'a>(
    app: &'a App,
    placeholder: String,
    height: f32,
    autocomplete: bool,
) -> Element<'a, Msg> {
    let empty = app.editor.text().trim().is_empty();
    let autocomplete = autocomplete || !app.emoji_candidates().is_empty();
    let viewing_image = app.features.viewer.current().is_some();
    let quick_binding = app
        .settings
        .preferences
        .keybinds
        .get("Quick Switcher")
        .map_or("Ctrl+K", String::as_str);
    let editor = text_editor(&app.editor)
        // URLs, hashes, and pasted tokens must remain editable inside the same
        // viewport that the composer's wrapped-height estimate reserves.
        .wrapping(widget::text::Wrapping::WordOrGlyph)
        .id("composer-editor")
        .placeholder(placeholder)
        .key_binding(move |e| {
            if viewing_image {
                Some(text_editor::Binding::Custom(Msg::feature(
                    crate::features::Event::Key(e.key),
                )))
            } else {
                editor_binding(quick_binding, e, empty, autocomplete)
            }
        })
        .font(t::regular())
        .size(t::BODY)
        .line_height(t::LINE)
        .height(height)
        .padding(t::S4)
        .style(t::editor);
    if matches!(app.dialog, Dialog::None) {
        editor.on_action(Msg::Editor).into()
    } else {
        editor.into()
    }
}

/// The pop-out shares the same bounded call/media lifetime; no second connection.
pub fn view_window(app: &App, id: iced::window::Id) -> Element<'_, Msg> {
    if app.calls.popout == Some(id) {
        if let Some(session) = app.calls.focus {
            return stream_view::view_at(
                app,
                session,
                app.calls.popout_size.map_or(600., |s| s.height),
            );
        }
    }
    view(app)
}

#[cfg(test)]
mod integration_layout_tests {
    use super::*;
    #[test]
    fn owner_and_member_reserve_the_same_action_column() {
        let app = App::headless("guild-settings-members", iced::Size::new(1280., 800.));
        let owner = member_action(&app, app.me().unwrap());
        let member = member_action(&app, Id(2));
        assert_eq!(owner.as_widget().size(), member.as_widget().size());
        assert_eq!(owner.as_widget().size().width, Length::Fixed(36.));
    }
}

#[cfg(test)]
mod qa_long_composer {
    use super::*;
    #[test]
    fn long_unbroken_drafts_paint_wrapped_rows_not_one_clipped_line() {
        crate::headless::load_fonts();
        let mut app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        app.editor = text_editor::Content::with_text(&"x".repeat(512));
        #[cfg(not(feature = "wgpu"))]
        let mut renderer = iced_tiny_skia::Renderer::new(t::regular(), iced::Pixels(t::BODY));
        #[cfg(feature = "wgpu")]
        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            t::regular(),
            iced::Pixels(t::BODY),
        ));
        let mut ui = iced_runtime::user_interface::UserInterface::build(
            compose_editor(&app, String::new(), 160., false),
            iced::Size::new(240., 160.),
            iced_runtime::user_interface::Cache::new(),
            &mut renderer,
        );
        ui.draw(
            &mut renderer,
            &t::theme(&app),
            &iced::advanced::renderer::Style {
                text_color: t::TEXT_PRIMARY,
            },
            iced::mouse::Cursor::Unavailable,
        );
        drop(ui);
        #[cfg(feature = "wgpu")]
        let iced::Renderer::Secondary(mut renderer) = renderer
        else {
            unreachable!()
        };
        let pixels = iced_tiny_skia::window::compositor::screenshot(
            &mut renderer,
            &iced_tiny_skia::graphics::Viewport::with_physical_size(iced::Size::new(240, 160), 1.),
            t::BG_CHAT,
        );
        // Ignore the first line and rounded borders. At least two subsequent
        // glyph rows must be painted, not hidden past the horizontal viewport.
        let painted = (32..150)
            .filter(|&y| {
                (8..232)
                    .filter(|&x| {
                        let i = (y * 240 + x) * 4;
                        pixels[i] > 150 && pixels[i + 1] > 150 && pixels[i + 2] > 150
                    })
                    .count()
                    > 5
            })
            .count();
        assert!(
            painted >= 20,
            "long draft rendered only {painted} later glyph scanlines"
        );
    }
}
