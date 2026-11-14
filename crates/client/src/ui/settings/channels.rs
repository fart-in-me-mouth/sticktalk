//! Server channel hierarchy. Actions are sibling buttons, never nested click targets.
use super::*;
use iced::{
    widget::{column, mouse_area},
    Padding,
};

pub(super) fn content(app: &App) -> Element<'_, Msg> {
    let manage = app.permissions().contains(Permissions::MANAGE_CHANNELS);
    let channels = app.sorted_channels();
    let mut body = column![row![
        title("Channels").width(Fill),
        super::super::action(
            "Create Channel",
            Msg::Open(Dialog::NewChannel(ChannelKind::Text, None))
        )
        .on_press_maybe(manage.then_some(Msg::Open(Dialog::NewChannel(ChannelKind::Text, None))))
        .style(st::secondary),
        super::super::action(
            "Create Category",
            Msg::Open(Dialog::NewChannel(ChannelKind::Category, None))
        )
        .on_press_maybe(
            manage.then_some(Msg::Open(Dialog::NewChannel(ChannelKind::Category, None)))
        )
        .style(st::secondary)
    ]
    .spacing(t::S16)
    .align_y(Center),]
    .spacing(t::S8);
    let roots: Vec<_> = channels
        .iter()
        .copied()
        .filter(|c| c.parent_id.is_none() && c.kind != ChannelKind::Category)
        .collect();
    if !roots.is_empty() {
        body = body.push(section("UNCATEGORIZED"));
        for ch in roots {
            body = body.push(channel_row(app, ch, &channels));
        }
    }
    for category in channels
        .iter()
        .copied()
        .filter(|c| c.kind == ChannelKind::Category)
    {
        let collapsed = app.settings.collapsed_categories.contains(&category.id);
        let heading = button(
            row![
                icon(
                    if collapsed {
                        "chevron-right"
                    } else {
                        "chevron-down"
                    },
                    t::INLINE_ICON,
                    t::TEXT_MUTED
                ),
                super::super::elided::label(app.state.model.channel_name(category).to_uppercase())
                    .size(t::HINT)
                    .font(t::semibold())
                    .color(t::TEXT_MUTED)
                    .width(Fill)
            ]
            .spacing(t::S8)
            .align_y(Center),
        )
        .on_press(Msg::Settings(A::CategoryCollapse(category.id)))
        .width(Fill)
        .height(t::CHANNEL_ROW)
        .padding(t::S8)
        .style(t::icon);
        body = body.push(space().height(t::S16)).push(hover_row(
            app,
            category,
            heading.into(),
            &channels,
        ));
        if !collapsed {
            for ch in channels
                .iter()
                .copied()
                .filter(|c| c.parent_id == Some(category.id))
            {
                body = body.push(channel_row(app, ch, &channels));
            }
            body = body.push(create_channel(Some(category.id), manage));
        }
    }
    body.into()
}

fn create_channel<'a>(parent: Option<Id>, enabled: bool) -> Element<'a, Msg> {
    container(
        button(
            row![
                icon("plus", t::INLINE_ICON, t::ACCENT),
                text("Create Channel")
                    .size(t::BODY)
                    .font(t::medium())
                    .style(t::inherit_text)
            ]
            .spacing(t::S8)
            .align_y(Center),
        )
        .height(t::BUTTON)
        .padding([t::S8, t::S16])
        .on_press_maybe(enabled.then_some(Msg::Open(Dialog::NewChannel(ChannelKind::Text, parent))))
        .style(t::link),
    )
    .padding(Padding {
        left: t::S32,
        ..Padding::ZERO
    })
    .into()
}

fn channel_row<'a>(
    app: &'a App,
    ch: &'a ChannelSummary,
    channels: &[&ChannelSummary],
) -> Element<'a, Msg> {
    let glyph = if app.state.model.launch.is_private(ch) {
        "lock-keyhole"
    } else if ch.kind == ChannelKind::Voice {
        "volume-2"
    } else {
        "hash"
    };
    let label = button(
        row![
            icon(glyph, t::INLINE_ICON, t::TEXT_MUTED),
            super::super::elided::label(app.state.model.channel_name(ch))
                .size(t::CHANNEL_NAME)
                .font(t::regular())
                .color(t::TEXT_SECONDARY)
                .width(Fill)
        ]
        .spacing(t::S8)
        .align_y(Center),
    )
    .on_press(Msg::Open(Dialog::ChannelSettings(ch.id)))
    .height(t::CHANNEL_ROW)
    .width(Fill)
    .padding([0., t::S8])
    .style(t::icon);
    container(hover_row(app, ch, label.into(), channels))
        .padding(Padding {
            left: t::S24,
            ..Padding::ZERO
        })
        .into()
}

fn hover_row<'a>(
    app: &'a App,
    ch: &'a ChannelSummary,
    label: Element<'a, Msg>,
    channels: &[&ChannelSummary],
) -> Element<'a, Msg> {
    let hovered = app.settings.hovered_channel == Some(ch.id);
    let mut body = row![label].align_y(Center).spacing(t::S4);
    if hovered
        && app
            .channel_permissions(ch.id)
            .contains(Permissions::MANAGE_CHANNELS)
    {
        let siblings: Vec<_> = channels
            .iter()
            .filter(|c| {
                c.parent_id == ch.parent_id
                    && (c.kind == ChannelKind::Category) == (ch.kind == ChannelKind::Category)
            })
            .collect();
        if let Some(index) = siblings.iter().position(|c| c.id == ch.id) {
            for (down, enabled, glyph, label) in [
                (false, index > 0, "arrow-up", "Move up"),
                (true, index + 1 < siblings.len(), "arrow-down", "Move down"),
            ] {
                if enabled {
                    body = body.push(icon_button(glyph, label, Msg::Reorder(ch.id, down)));
                }
            }
        }
        body = body
            .push(icon_button(
                "settings",
                "Edit channel",
                Msg::Open(Dialog::ChannelSettings(ch.id)),
            ))
            .push(icon_button(
                "trash-2",
                "Delete channel",
                Msg::Open(Dialog::Confirm(
                    if ch.kind == ChannelKind::Category {
                        "Delete this category? Its channels will become uncategorized.".into()
                    } else {
                        "Delete this channel and all its messages? This cannot be undone.".into()
                    },
                    Command::DeleteChannel { channel_id: ch.id },
                )),
            ));
    }
    mouse_area(container(body).width(Fill).style(move |_| {
        let mut style = t::surface(if hovered { t::BG_HOVER } else { t::TRANSPARENT });
        style.border.radius = t::S4.into();
        style
    }))
    .on_enter(Msg::Settings(A::ChannelHover(Some(ch.id))))
    .on_exit(Msg::Settings(A::ChannelLeave(ch.id)))
    .into()
}
