//! Server emoji surfaces, using the existing chat/settings visual tokens.
use super::*;
use crate::features3::{self as f, Event as E};
use iced::widget::column;
pub fn glyph<'a>(app: &App, id: Id, size: f32) -> Element<'a, Msg> {
    let picture: Element<'a, Msg> = if let Some(h) = app
        .text_features
        .assets
        .get(&id)
        .and_then(|a| {
            a.frames
                .as_ref()
                .and_then(|f| f.images.get(a.index % f.images.len()))
        })
        .map(|v| v.0.clone())
    {
        image(h).width(size).height(size).into()
    } else {
        container(icon("smile", size, t::TEXT_MUTED))
            .width(size)
            .height(size)
            .into()
    };
    mouse_area(picture)
        .on_enter(f::msg(E::Hover(Some(id))))
        .on_exit(f::msg(E::Hover(None)))
        .into()
}
pub fn settings(app: &App) -> Element<'_, Msg> {
    let allowed = app.permissions().contains(Permissions::MANAGE_EMOJI);
    let mut rows = column![
        title("Emoji"),
        hint("Upload PNG, GIF, or WebP images up to 256 KB. Emoji are resized to 128px."),
        row![
            text_input("Emoji name", &app.text_features.emoji_name)
                .on_input(|s| f::msg(E::Name(s)))
                .padding(t::S12)
                .font(t::regular())
                .size(t::BODY)
                .style(t::input),
            primary(
                if app.text_features.uploading {
                    "Uploading…"
                } else {
                    "Upload Emoji"
                },
                f::msg(E::Pick)
            )
            .on_press_maybe(
                (allowed
                    && !app.text_features.uploading
                    && f::entries(app).len() < MAX_GUILD_EMOJI)
                    .then_some(f::msg(E::Pick))
            )
        ]
        .spacing(t::S12)
        .align_y(Center),
        section(format!(
            "SERVER EMOJI — {} / {}",
            f::entries(app).len(),
            MAX_GUILD_EMOJI
        ))
    ]
    .spacing(t::S16);
    if !allowed {
        rows = rows.push(hint(
            "You need Manage Emoji permission to change server emoji.",
        ));
    }
    if f::entries(app).is_empty() {
        rows = rows.push(
            container(
                column![
                    icon("smile", t::AVATAR, t::TEXT_MUTED),
                    txt("No custom emoji yet")
                ]
                .spacing(t::S12)
                .align_x(Center),
            )
            .center_x(Fill)
            .padding(t::S32),
        );
    }
    for e in f::entries(app) {
        let name = app.text_features.emoji_edits.get(&e.id).unwrap_or(&e.name);
        let id = e.id;
        rows = rows.push(
            row![
                glyph(app, id, t::AVATAR),
                text_input("Emoji name", name)
                    .on_input_maybe(allowed.then_some(move |s| f::msg(E::Rename(id, s))))
                    .font(t::regular())
                    .size(t::BODY)
                    .padding(t::S12)
                    .style(t::input),
                hint(if e.animated { "Animated" } else { "" }).width(t::S24 * 4.),
                action("Save", f::msg(E::Save(id))).on_press_maybe(
                    (allowed && name != &e.name && valid_emoji_name(name))
                        .then_some(f::msg(E::Save(id)))
                ),
                button(icon("trash-2", t::ICON, t::DANGER))
                    .on_press_maybe(allowed.then_some(f::msg(E::Delete(id))))
                    .padding(t::S8)
                    .style(t::icon)
            ]
            .spacing(t::S12)
            .align_y(Center),
        );
    }
    rows.into()
}
pub fn picker(app: &App) -> Option<Element<'_, Msg>> {
    let query = app.features.emoji.query.to_ascii_lowercase();
    let entries: Vec<_> = f::entries(app)
        .iter()
        .filter(|e| query.is_empty() || e.name.to_ascii_lowercase().contains(&query))
        .collect();
    if entries.is_empty() {
        return None;
    }
    let mut grid = column![section(
        app.state
            .guild
            .and_then(|id| app.state.model.guilds.get(&id))
            .map_or("SERVER EMOJI".into(), |g| g.guild.name.to_uppercase())
    )]
    .spacing(t::S4);
    for chunk in entries.chunks(8) {
        let mut r = Row::new().spacing(t::S4);
        for e in chunk {
            r = r.push(
                tooltip(
                    button(
                        container(glyph(app, e.id, t::S24))
                            .center_x(t::AVATAR)
                            .center_y(t::AVATAR),
                    )
                    .on_press(Msg::Emoji(e.token()))
                    .padding(0)
                    .style(t::icon),
                    container(hint(format!(":{}:", e.name)))
                        .padding(t::S8)
                        .style(t::tooltip),
                    tooltip::Position::Top,
                )
                .style(t::tooltip),
            );
        }
        grid = grid.push(r);
    }
    Some(scroller(grid).height(t::S24 * 4.).into())
}
pub fn autocomplete(app: &App) -> Option<Element<'_, Msg>> {
    let entries = f::candidates(app);
    if entries.is_empty() {
        return None;
    }
    let mut rows = column![section("SERVER EMOJI")].spacing(t::S4);
    for (i, e) in entries.iter().enumerate() {
        rows = rows.push(
            button(
                row![
                    glyph(app, e.id, t::S24),
                    txt(format!(":{}:", e.name)),
                    hint("Server emoji")
                ]
                .spacing(t::S12)
                .align_y(Center),
            )
            .on_press(Msg::feature(crate::features::Event::EmojiComplete(
                e.token(),
            )))
            .width(Fill)
            .height(t::AUTOCOMPLETE)
            .padding([t::S4, t::S8])
            .style(t::navigation(i == app.suggestion)),
        );
    }
    Some(container(rows).padding(t::S8).style(t::popover).into())
}
pub fn inline<'a>(
    app: &App,
    m: &Message,
    values: &[crate::markdown::Inline],
    size: f32,
) -> Option<Element<'a, Msg>> {
    use sticktalk_client_core::features3::{parts, Part};
    use unicode_segmentation::UnicodeSegmentation;
    if !values
        .iter()
        .any(|v| !v.style.code && parts(&v.text).iter().any(|p| matches!(p, Part::Emoji(..))))
    {
        return None;
    }
    let known = |id| {
        app.state
            .model
            .launch
            .emoji
            .values()
            .flatten()
            .any(|e| e.id == id)
    };
    let emoji_only = m.attachments.is_empty()
        && parts(&m.content).iter().all(|p| match p {
            Part::Emoji(id, _) => known(*id),
            Part::Text(t) => t
                .graphemes(true)
                .all(|g| g.trim().is_empty() || emoji_picker::has_artwork(g)),
        });
    let mut flow = Row::new().spacing(0).align_y(Center);
    for v in values {
        let hidden = v
            .spoiler
            .is_some_and(|i| !app.revealed.contains(&(m.id, i)));
        if v.style.code || hidden {
            flow = flow.push(super::inline_text(app, m, std::slice::from_ref(v), size));
            continue;
        }
        for p in parts(&v.text) {
            match p {
                Part::Emoji(id, _) if known(id) => {
                    flow = flow.push(glyph(
                        app,
                        id,
                        if emoji_only {
                            t::AVATAR + t::S8
                        } else {
                            size * app.settings.preferences.chat_font_scale
                        },
                    ))
                }
                Part::Emoji(_, s) | Part::Text(s) => {
                    let mut v = v.clone();
                    v.text = s.into();
                    flow = flow.push(super::inline_content(
                        app,
                        m,
                        &[v],
                        if emoji_only { t::AVATAR + t::S8 } else { size },
                    ));
                }
            }
        }
    }
    Some(flow.width(Fill).wrap().into())
}
