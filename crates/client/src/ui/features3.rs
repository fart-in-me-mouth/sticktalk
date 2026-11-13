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
pub fn search_panel(app: &App) -> Element<'_, Msg> {
    let s = &app.text_features.search;
    let mut content = column![
        row![
            title("Search Results"),
            space().width(Fill),
            icon_button("x", "Close Search", f::msg(E::SearchClose))
        ]
        .align_y(Center),
        hint(&s.query),
        line()
    ]
    .spacing(t::S16);
    if s.loading {
        content = content.push(hint("Searching messages…"));
    }
    if !s.error.is_empty() {
        content = content
            .push(txt(&s.error).color(t::DANGER))
            .push(action("Try again", f::msg(E::SearchSubmit)));
    }
    let mut results = column![].spacing(t::S16);
    if let Some(page) = &s.page {
        content = content.push(section(format!(
            "{}{} RESULTS",
            page.items.len(),
            if page.has_more { "+" } else { "" }
        )));
        if page.items.is_empty() && !s.loading {
            results = results.push(
                container(
                    column![
                        icon("search", t::AVATAR, t::TEXT_MUTED),
                        txt("No messages found"),
                        hint("Try another term or remove a filter.")
                    ]
                    .spacing(t::S12)
                    .align_x(Center),
                )
                .padding(t::S24)
                .center_x(Fill),
            );
        }
        for m in &page.items {
            if !app.state.model.channels.contains_key(&m.channel_id) {
                continue;
            }
            let channel = app
                .state
                .model
                .channels
                .get(&m.channel_id)
                .map_or("Deleted channel".into(), |c| {
                    format!("#{}", app.state.model.channel_name(c))
                });
            let name = page.users.iter().find(|u| u.id == m.author_id).map_or_else(
                || app.state.model.name(m.author_id),
                |u| u.display_name.as_deref().unwrap_or(&u.username).to_owned(),
            );
            let spans: Vec<_> =
                sticktalk_client_core::features3::highlights(&m.content, &page.terms)
                    .into_iter()
                    .map(|(text, matched)| {
                        widget::text::Span::<()>::new(text.to_owned()).color(if matched {
                            t::WARNING
                        } else {
                            t::TEXT_PRIMARY
                        })
                    })
                    .collect();
            let snippet = widget::rich_text(spans)
                .font(t::regular())
                .size(t::BODY)
                .wrapping(widget::text::Wrapping::WordOrGlyph);
            results = results.push(
                button(
                    column![
                        section(channel),
                        row![
                            txt(name).font(t::semibold()),
                            space().width(Fill),
                            hint(timestamp(m.created_at, true))
                        ]
                        .spacing(t::S8)
                        .align_y(Center),
                        snippet,
                        hint("Jump to message")
                    ]
                    .spacing(t::S8),
                )
                .on_press(f::msg(E::SearchJump(m.channel_id, m.id)))
                .padding(t::S12)
                .width(Fill)
                .style(t::secondary),
            );
        }
    } else if !s.loading && s.error.is_empty() {
        results = results.push(hint(
            "Use from:, in:, has:link/file/image, before:, after:, or mentions: to narrow results.",
        ));
    }
    content = content.push(scroller(results).height(Fill));
    content = content.push(
        row![
            action("Newer", f::msg(E::SearchPage(true))).on_press_maybe(
                (s.cursors.len() > 1 && !s.loading).then_some(f::msg(E::SearchPage(true)))
            ),
            space().width(Fill),
            action("Older", f::msg(E::SearchPage(false))).on_press_maybe(
                (s.page.as_ref().is_some_and(|p| p.has_more) && !s.loading)
                    .then_some(f::msg(E::SearchPage(false)))
            )
        ]
        .spacing(t::S8)
        .align_y(Center),
    );
    container(content)
        .padding(t::S16)
        .width(t::SEARCH_RESULTS)
        .height(Fill)
        .style(t::sidebar)
        .into()
}
fn slowmode_label(seconds: u32) -> &'static str {
    match seconds {
        0 => "Off",
        5 => "5 seconds",
        10 => "10 seconds",
        30 => "30 seconds",
        60 => "1 minute",
        300 => "5 minutes",
        900 => "15 minutes",
        3600 => "1 hour",
        21600 => "6 hours",
        _ => "Off",
    }
}
pub fn slowmode_settings(app: &App, id: Id, allowed: bool) -> Element<'_, Msg> {
    let Some(status) = app.state.model.launch.slowmode.get(&id) else {
        return hint("Loading slowmode…").into();
    };
    let seconds = status.seconds;
    let control: Element<'_, Msg> = if allowed {
        let labels: Vec<_> = SLOWMODE_SECONDS
            .iter()
            .map(|s| slowmode_label(*s))
            .collect();
        widget::pick_list(labels, Some(slowmode_label(seconds)), |label| {
            f::msg(E::Slowmode(
                SLOWMODE_SECONDS
                    .iter()
                    .copied()
                    .find(|s| slowmode_label(*s) == label)
                    .unwrap_or(0),
            ))
        })
        .padding(t::S12)
        .font(t::regular())
        .text_size(t::BODY)
        .style(t::settings::pick_list)
        .menu_style(t::settings::menu)
        .into()
    } else {
        container(txt(slowmode_label(seconds)).color(t::TEXT_MUTED))
            .padding(t::S12)
            .style(t::elevated)
            .into()
    };
    column![control,hint(if allowed{"Limit how often members can send a message. Members with Manage Messages or Manage Channels are exempt. Changes apply immediately."}else{"You need Manage Channels permission to change slowmode."})].spacing(t::S12).into()
}

pub fn slowmode_composer(app: &App, id: Id) -> Option<Element<'_, Msg>> {
    let c = app.text_features.cooldowns.get(&id)?;
    if c.seconds == 0 {
        return None;
    }
    let remaining = f::remaining(app, id).div_ceil(1000);
    let label = if c.bypassed {
        "Slowmode is enabled. You are exempt.".into()
    } else if remaining > 0 {
        format!(
            "Slowmode — send again in {}:{:02}",
            remaining / 60,
            remaining % 60
        )
    } else {
        format!(
            "Slowmode is enabled — one message every {}.",
            slowmode_label(c.seconds)
        )
    };
    Some(
        row![
            icon(
                "clock",
                t::INLINE_ICON,
                if remaining > 0 {
                    t::WARNING
                } else {
                    t::TEXT_MUTED
                }
            ),
            hint(label).color(if remaining > 0 {
                t::WARNING
            } else {
                t::TEXT_MUTED
            })
        ]
        .spacing(t::S8)
        .align_y(Center)
        .into(),
    )
}
