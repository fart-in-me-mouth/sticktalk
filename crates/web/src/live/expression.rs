//! Expression in the web app: custom emoji everywhere (picker, `:name:`
//! completion, rendering, reactions), stickers, Super Reactions and custom
//! typing indicators. Views are in `ui/expression.rs`, styles in
//! `styles/expression.css`, the long-press gesture in `public/expression.js`.
//!
//! Nothing here runs while unused: the library is fetched once per Ready, an
//! unknown emoji is resolved once, and a Super Reaction burst is one CSS
//! animation on one element (no timers, no loops).
use super::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use sticktalk_client_core::http::decode_json;

/// The most recently used emoji and stickers remembered on this device.
const RECENT_EMOJI: usize = 24;
const RECENT_STICKERS: usize = 12;
/// Emoji seen in messages from servers this account is not in.
const RESOLVED_LIMIT: usize = 2_000;
/// How long a Super Reaction burst plays, in milliseconds (matches the CSS).
pub(super) const BURST_MS: u64 = 900;

#[derive(Default)]
pub(super) struct State {
    /// A Ready came and the library is not loaded yet: that waits for the
    /// first conversation (`operations::wake`), not sign-in.
    pub stale: bool,
    /// Stickers by server (None is the built-in pack), from the library and events.
    pub stickers: BTreeMap<Option<Id>, Vec<Sticker>>,
    pub settings: ExpressionSettings,
    pub loaded: bool,
    /// What an emoji from elsewhere is (or that it is gone).
    pub resolved: RefCell<BTreeMap<Id, EmojiInfo>>,
    /// Seen while rendering and not yet asked about.
    pub wanted: RefCell<BTreeSet<Id>>,
    pub requested: RefCell<BTreeSet<Id>>,
    pub resolving: Cell<bool>,
    /// Indicators chosen by others (bounded).
    pub typing: BTreeMap<Id, TypingStyle>,
    /// Super Reactions just added: (message, emoji, when, in Unix ms).
    pub bursts: Vec<(Id, String, u64)>,
    /// The reaction picker adds Super Reactions.
    pub super_mode: bool,
    pub recent: Vec<String>,
    pub recent_stickers: Vec<Id>,
    /// Stickers picked for one message (at most three).
    pub tray: Vec<Id>,
    /// Stickers related to the emoji just typed in the composer.
    pub suggest: String,
    /// Server settings: (server, its emoji and stickers).
    pub admin: Option<(Id, GuildExpressionAdmin)>,
    pub busy: bool,
    /// How much of each bundled section of the emoji picker is drawn, for a
    /// search: (query, [(section, emoji drawn)]). The first rows show at once,
    /// the rest as the list scrolls near them (`ui::expression::PICKER_WINDOW`).
    pub drawn: (String, Vec<(String, usize)>),
}

pub(super) const ACTIONS: &[&str] = &[
    "expr-emoji",
    "expr-react",
    "expr-super-picker",
    "expr-super-mode",
    "super-react",
    "expr-more",
    "expr-sticker",
    "expr-sticker-tray",
    "expr-sticker-send",
    "expr-sticker-fav",
    "expr-suggest",
    "expr-emoji-delete",
    "expr-sticker-delete",
    "expr-typing-reset",
];
pub(super) const FORMS: &[&str] = &[
    "expr-emoji-upload",
    "expr-sticker-upload",
    "expr-emoji-edit",
    "expr-sticker-edit",
    "expr-typing",
];

fn notice(a: &mut App, text: &str) {
    a.notice = text.into();
    a.error = text.into();
}
fn storage_get(key: &str) -> Option<String> {
    let storage = js_sys::Reflect::get(&window(), &"localStorage".into()).ok()?;
    let get = js_sys::Reflect::get(&storage, &"getItem".into()).ok()?;
    let get: js_sys::Function = get.dyn_into().ok()?;
    get.call1(&storage, &key.into()).ok()?.as_string()
}
fn storage_set(key: &str, value: &str) {
    let Ok(storage) = js_sys::Reflect::get(&window(), &"localStorage".into()) else {
        return;
    };
    if let Ok(set) = js_sys::Reflect::get(&storage, &"setItem".into()) {
        if let Ok(set) = set.dyn_into::<js_sys::Function>() {
            let _ = set.call2(&storage, &key.into(), &value.into());
        }
    }
}

/// After a Ready, with the first conversation (`operations::wake`): load
/// the library. The core subscribed at Ready.
pub(super) fn ready(app: Shared) {
    app.borrow_mut().expression.stale = false;
    let (http, epoch) = (app.borrow().http(), app.borrow().epoch);
    {
        let mut a = app.borrow_mut();
        if a.expression.recent.is_empty() {
            a.expression.recent = storage_get("sticktalk-recent-emoji")
                .filter(|v| v.len() <= 4096)
                .and_then(|v| serde_json::from_slice::<Vec<String>>(v.as_bytes()).ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.len() <= 64)
                .take(RECENT_EMOJI)
                .collect();
            a.expression.recent_stickers = storage_get("sticktalk-recent-stickers")
                .filter(|v| v.len() <= 1024)
                .and_then(|v| serde_json::from_slice::<Vec<u64>>(v.as_bytes()).ok())
                .unwrap_or_default()
                .into_iter()
                .map(Id)
                .take(RECENT_STICKERS)
                .collect();
        }
    }
    spawn_local(async move {
        let Ok(response) = http
            .execute(
                "GET",
                "/api/v1/users/@me/expression",
                Body::Empty,
                vec![],
                30_000,
            )
            .await
        else {
            return;
        };
        let Ok(library) = decode_json::<ExpressionLibrary>(response) else {
            return;
        };
        let mut a = app.borrow_mut();
        if a.epoch != epoch {
            return;
        }
        for set in library.emoji {
            a.model.launch.emoji.insert(set.guild_id, set.emoji);
        }
        a.model.launch.emoji_revision = a.model.launch.emoji_revision.wrapping_add(1);
        a.expression.stickers = library
            .stickers
            .into_iter()
            .map(|s| (s.guild_id, s.stickers))
            .collect();
        a.expression.settings = library.settings;
        a.expression.loaded = true;
        a.touch();
    });
}

/// An expression event (only sent because this session subscribed).
pub(super) fn event(a: &mut App, e: ExpressionEvent) {
    match e {
        ExpressionEvent::StickersUpdated { guild_id, stickers } => {
            a.expression.stickers.insert(Some(guild_id), stickers);
            refresh_admin(a, guild_id);
        }
        ExpressionEvent::SuperReactionUpdated(update) => {
            let Some(m) = a
                .model
                .messages
                .get_mut(&update.channel_id)
                .and_then(|ms| ms.get_mut(&update.message_id))
            else {
                return;
            };
            let mut list = m.extra.super_reactions().to_vec();
            // Each recipient gets its own copy, so `me` is about this account.
            match list.iter_mut().find(|r| r.emoji == update.reaction.emoji) {
                Some(r) => {
                    r.count = update.reaction.count;
                    r.me = update.reaction.me;
                }
                None => list.push(update.reaction.clone()),
            }
            list.retain(|r| r.count > 0);
            list.sort_by(|x, y| x.emoji.cmp(&y.emoji));
            m.extra.set_super_reactions(list);
            if update.added && a.channel == Some(update.channel_id) {
                let now = Browser.unix_ms();
                let bursts = &mut a.expression.bursts;
                bursts.retain(|(_, _, at)| now.saturating_sub(*at) < BURST_MS);
                bursts
                    .retain(|(m, e, _)| !(*m == update.message_id && *e == update.reaction.emoji));
                if bursts.len() >= 32 {
                    bursts.remove(0);
                }
                bursts.push((update.message_id, update.reaction.emoji, now));
            }
        }
        ExpressionEvent::TypingStyle { user_id, style } => {
            if a.expression.typing.len() >= 256 && !a.expression.typing.contains_key(&user_id) {
                a.expression.typing.pop_first();
            }
            a.expression.typing.insert(user_id, style);
        }
        ExpressionEvent::SettingsUpdated(settings) => a.expression.settings = settings,
    }
}

fn refresh_admin(a: &App, guild: Id) {
    if a.expression
        .admin
        .as_ref()
        .is_some_and(|(g, _)| *g == guild)
    {
        if let Some(app) = a.owner.upgrade() {
            features::defer(
                app,
                features::Feature::Expression,
                features::Call::Other("admin", guild.0.to_string()),
            );
        }
    }
}

/// A dialog opened: load what it shows.
pub(super) fn opened(app: Shared, name: &str) {
    let mut a = app.borrow_mut();
    if name == "emoji" || name == "stickers" || name.starts_with("emoji-react:") {
        a.expression.drawn = Default::default();
        // The sparkles button turns Super Reaction on after opening.
        a.expression.super_mode = false;
        a.expression.tray.clear();
    }
    if admin_page(name) {
        if let Some(g) = a.guild {
            if a.expression.admin.as_ref().is_none_or(|(v, _)| *v != g) {
                a.expression.admin = None;
            }
            a.expression.busy = true;
            drop(a);
            spawn_local(async move { load_admin(app, g).await });
        }
    }
}

/// The server settings pages for emoji and stickers.
pub(super) fn admin_page(name: &str) -> bool {
    matches!(name, "server-emoji" | "server-stickers")
        || name.starts_with("emoji-edit:")
        || name.starts_with("sticker-edit:")
}

async fn load_admin(app: Shared, g: Id) {
    let (http, epoch) = (app.borrow().http(), app.borrow().epoch);
    let result = async {
        decode_json::<GuildExpressionAdmin>(
            http.execute(
                "GET",
                &format!("/api/v1/guilds/{}/expression", g.0),
                Body::Empty,
                vec![],
                30_000,
            )
            .await?,
        )
    }
    .await;
    let mut a = app.borrow_mut();
    if a.epoch != epoch {
        return;
    }
    a.expression.busy = false;
    match result {
        Ok(admin) => a.expression.admin = Some((g, admin)),
        Err(e) => a.error = e,
    }
    a.touch();
}

/// Called while rendering: remember an emoji we know nothing about, and ask
/// the server about everything remembered once this render has finished.
pub(super) fn want(a: &App, e: Id) {
    let s = &a.expression;
    if s.requested.borrow().contains(&e) || s.resolved.borrow().contains_key(&e) {
        return;
    }
    s.wanted.borrow_mut().insert(e);
    resolve_soon(a);
}
fn resolve_soon(a: &App) {
    if a.expression.resolving.replace(true) {
        return;
    }
    // Called while rendering: the lookup runs in this feature's module, later.
    if let Some(app) = a.owner.upgrade() {
        features::defer(
            app,
            features::Feature::Expression,
            features::Call::State("resolve", String::new()),
        );
    }
}
fn resolve(app: Shared) {
    spawn_local(async move {
        let (http, epoch, ids) = {
            let a = app.borrow();
            let mut wanted = a.expression.wanted.borrow_mut();
            let ids: Vec<Id> = wanted.iter().take(MAX_RESOLVE_EMOJI).copied().collect();
            for e in &ids {
                wanted.remove(e);
            }
            let mut requested = a.expression.requested.borrow_mut();
            if requested.len() > RESOLVED_LIMIT {
                requested.clear();
            }
            requested.extend(ids.iter().copied());
            (a.http(), a.epoch, ids)
        };
        if ids.is_empty() {
            app.borrow().expression.resolving.set(false);
            return;
        }
        let list = ids
            .iter()
            .map(|e| e.0.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let result = http
            .execute(
                "GET",
                &format!("/api/v1/emoji/resolve?ids={list}"),
                Body::Empty,
                vec![],
                30_000,
            )
            .await
            .and_then(decode_json::<Vec<EmojiInfo>>);
        let a = app.borrow();
        a.expression.resolving.set(false);
        if a.epoch != epoch {
            return;
        }
        {
            let mut resolved = a.expression.resolved.borrow_mut();
            let infos = result.unwrap_or_default();
            for e in &ids {
                let info = infos
                    .iter()
                    .find(|i| i.id == *e)
                    .cloned()
                    .unwrap_or(EmojiInfo {
                        id: *e,
                        name: String::new(),
                        animated: false,
                        guild_id: None,
                        guild_name: None,
                        guild_icon_id: None,
                        available: false,
                    });
                if resolved.len() >= RESOLVED_LIMIT {
                    resolved.pop_first();
                }
                resolved.insert(*e, info);
            }
        }
        a.touch();
        if !a.expression.wanted.borrow().is_empty() {
            resolve_soon(&a);
        }
    });
}

/// What we know about a custom emoji.
pub(super) enum Look {
    /// In one of this account's servers, or resolved: (name, animated, server).
    Known(String, bool, Option<Id>),
    /// Deleted, or not visible to this account.
    Gone(String),
    Unknown,
}
pub(super) fn look(a: &App, e: Id) -> Look {
    for (g, list) in &a.model.launch.emoji {
        if let Some(v) = list.iter().find(|v| v.id == e) {
            return Look::Known(v.name.clone(), v.animated, Some(*g));
        }
    }
    match a.expression.resolved.borrow().get(&e) {
        Some(i) if i.available => Look::Known(i.name.clone(), i.animated, i.guild_id),
        Some(i) => Look::Gone(i.name.clone()),
        None => Look::Unknown,
    }
}

/// Every custom emoji this account may use, current server first.
pub(super) fn usable(a: &App) -> Vec<(Id, &GuildEmoji)> {
    let mut sets: Vec<(&Id, &Vec<GuildEmoji>)> = a
        .model
        .launch
        .emoji
        .iter()
        .filter(|(g, _)| a.model.guilds.contains_key(g))
        .collect();
    sets.sort_by_key(|(g, _)| (Some(**g) != a.guild, **g));
    sets.into_iter()
        .flat_map(|(g, list)| list.iter().map(move |e| (*g, e)))
        .collect()
}

/// Which emoji `:name:` means here: this server's first, then the others'.
fn by_name<'a>(a: &'a App, name: &str) -> Option<&'a GuildEmoji> {
    usable(a)
        .into_iter()
        .map(|(_, e)| e)
        .find(|e| e.name.eq_ignore_ascii_case(name))
}

/// What to put in the composer for a custom emoji: `:name:` when that name
/// means this emoji, otherwise its full token.
pub(super) fn insert_text(a: &App, e: &GuildEmoji) -> String {
    if by_name(a, &e.name).is_some_and(|v| v.id == e.id) {
        format!(":{}:", e.name)
    } else {
        e.token()
    }
}

/// Before sending: `:name:` becomes the custom emoji's token, outside code.
pub(super) fn prepare_send(a: &App, content: &str) -> String {
    if !content.contains(':') || a.model.launch.emoji.is_empty() {
        return content.to_owned();
    }
    let mut out = String::with_capacity(content.len());
    let (mut fence, mut tick) = (false, false);
    let mut rest = content;
    while let Some(c) = rest.chars().next() {
        if let Some(tail) = rest.strip_prefix("```") {
            fence = !fence;
            out.push_str("```");
            rest = tail;
            continue;
        }
        if c == '`' && !fence {
            tick = !tick;
        } else if c == '\n' {
            tick = false;
        } else if c == ':' && !fence && !tick {
            let tail = &rest[1..];
            let len = tail
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            if (2..=32).contains(&len) && tail[len..].starts_with(':') {
                // Not already part of a `<:name:id>` token.
                if !out.ends_with('<') {
                    if let Some(e) = by_name(a, &tail[..len]) {
                        out.push_str(&e.token());
                        rest = &tail[len + 1..];
                        continue;
                    }
                }
            }
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// `:` completions for custom emoji: (label, inserted text, kind).
pub(super) fn completions(a: &App, query: &str, limit: usize) -> Vec<(String, String, String)> {
    if query.is_empty() {
        return vec![];
    }
    usable(a)
        .into_iter()
        .filter(|(_, e)| e.name.to_lowercase().contains(query))
        .take(limit)
        .map(|(_, e)| (format!(":{}:", e.name), insert_text(a, e), "Custom".into()))
        .collect()
}

/// An emoji was picked (also from the bundled grid): it is recently used.
pub(super) fn used(a: &mut App, emoji: &str) {
    if plausible_unicode_emoji(emoji) || custom_emoji_id(emoji).is_some() {
        remember(a, emoji);
    }
}
fn remember(a: &mut App, emoji: &str) {
    let s = &mut a.expression;
    s.recent.retain(|e| e != emoji);
    s.recent.insert(0, emoji.to_owned());
    s.recent.truncate(RECENT_EMOJI);
    if let Ok(v) = serde_json::to_string(&s.recent) {
        storage_set("sticktalk-recent-emoji", &v);
    }
}
fn remember_sticker(a: &mut App, sticker: Id) {
    let s = &mut a.expression;
    s.recent_stickers.retain(|e| *e != sticker);
    s.recent_stickers.insert(0, sticker);
    s.recent_stickers.truncate(RECENT_STICKERS);
    let ids: Vec<u64> = s.recent_stickers.iter().map(|i| i.0).collect();
    if let Ok(v) = serde_json::to_string(&ids) {
        storage_set("sticktalk-recent-stickers", &v);
    }
}

/// Normal or Super Reaction from the chips and the picker.
fn toggle(a: &mut App, mid: Id, emoji: &str, burst: bool) {
    let Some(c) = a.channel else { return };
    let Some(m) = a.model.messages.get(&c).and_then(|v| v.get(&mid)) else {
        return;
    };
    // The server keeps a custom emoji as <:emoji:ID>.
    let emoji = match custom_emoji_id(emoji) {
        Some(e) => format!("<:emoji:{}>", e.0),
        None => emoji.to_owned(),
    };
    if burst {
        let mine = m
            .extra
            .super_reactions()
            .iter()
            .any(|r| r.emoji == emoji && r.me);
        a.command(Command::Expression(if mine {
            ExpressionCommand::RemoveSuperReaction {
                channel_id: c,
                message_id: mid,
                emoji,
            }
        } else {
            ExpressionCommand::AddSuperReaction {
                channel_id: c,
                message_id: mid,
                emoji,
            }
        }));
    } else {
        let mine = m.reactions.iter().any(|r| r.emoji == emoji && r.me);
        a.command(if mine {
            Command::RemoveReaction {
                channel_id: c,
                message_id: mid,
                emoji,
            }
        } else {
            Command::AddReaction {
                channel_id: c,
                message_id: mid,
                emoji,
            }
        });
    }
}

fn send_stickers(a: &mut App, ids: Vec<Id>, with_text: bool) {
    let Some(channel_id) = a.channel else { return };
    if ids.is_empty() || !a.ready {
        return;
    }
    let (content, mention_ids) = if with_text {
        let text = prepare_send(a, &value("live-input"));
        super::operations::mentions(&a.model, &text)
    } else {
        (String::new(), vec![])
    };
    for s in &ids {
        remember_sticker(a, *s);
    }
    let reply_to = a.reply.take();
    a.command(Command::Expression(ExpressionCommand::SendStickers {
        channel_id,
        sticker_ids: ids,
        content,
        reply_to,
        mention_ids,
    }));
    a.expression.tray.clear();
}

pub(super) fn action(app: Shared, kind: &str, arg: &str) {
    let mut a = app.borrow_mut();
    match kind {
        // Insert into the composer, from the picker.
        "expr-emoji" => {
            remember(&mut a, arg);
            let text = match custom_emoji_id(arg) {
                Some(e) => usable(&a)
                    .into_iter()
                    .find(|(_, v)| v.id == e)
                    .map_or_else(|| arg.to_owned(), |(_, v)| insert_text(&a, v)),
                None => arg.to_owned(),
            };
            media_action(&format!("insert:{text}"));
            a.dialog.clear();
            a.touch();
        }
        "expr-react" => {
            if let Some((mid, emoji)) = arg.split_once(',') {
                remember(&mut a, emoji);
                let burst = a.expression.super_mode;
                a.dialog.clear();
                a.dialog_generation = a.dialog_generation.wrapping_add(1);
                if let Some(mid) = id(mid) {
                    toggle(&mut a, mid, emoji, burst);
                }
                a.touch();
            }
        }
        "expr-super-picker" => {
            drop(a);
            action_dialog(&app, &format!("emoji-react:{arg}"));
            app.borrow_mut().expression.super_mode = true;
            app.borrow().touch();
        }
        "expr-super-mode" => {
            a.expression.super_mode = !a.expression.super_mode;
            a.touch();
        }
        "super-react" => {
            if let Some((mid, emoji)) = arg.split_once(',') {
                if let Some(mid) = id(mid) {
                    toggle(&mut a, mid, emoji, true);
                }
            }
        }
        // A section's undrawn part scrolled near: draw more of it ("key,count").
        "expr-more" => {
            if let Some((key, n)) = arg.split_once(',') {
                let n = n.parse::<usize>().unwrap_or(0);
                if a.expression.drawn.0 != a.query {
                    a.expression.drawn = (a.query.clone(), vec![]);
                }
                let sections = &mut a.expression.drawn.1;
                if let Some((_, shown)) = sections.iter_mut().find(|(k, _)| k == key) {
                    *shown = (*shown).max(n);
                } else if sections.len() < 16 {
                    // Bounded: the bundled categories and the search results.
                    sections.push((key.into(), n));
                }
                a.touch();
            }
        }
        // Send one sticker now.
        "expr-sticker" | "expr-sticker-send" => {
            if let Some(s) = id(arg) {
                send_stickers(&mut a, vec![s], false);
                a.expression.suggest.clear();
                a.touch();
            }
        }
        // Add to (or take out of) the stickers for one message.
        "expr-sticker-tray" => {
            if arg == "send" {
                let ids = a.expression.tray.clone();
                send_stickers(&mut a, ids, true);
            } else if let Some(s) = id(arg) {
                let tray = &mut a.expression.tray;
                if let Some(i) = tray.iter().position(|v| *v == s) {
                    tray.remove(i);
                } else if tray.len() < MAX_MESSAGE_STICKERS {
                    tray.push(s);
                } else {
                    notice(&mut a, "A message can carry up to 3 stickers.");
                }
            }
            a.touch();
        }
        "expr-sticker-fav" => {
            if let Some(s) = id(arg) {
                let mut favorites = a.expression.settings.sticker_favorites.clone();
                if let Some(i) = favorites.iter().position(|v| *v == s) {
                    favorites.remove(i);
                } else {
                    favorites.insert(0, s);
                    favorites.truncate(MAX_STICKER_FAVORITES);
                }
                a.expression.settings.sticker_favorites = favorites.clone();
                a.command(Command::Expression(
                    ExpressionCommand::SetStickerFavorites {
                        sticker_ids: favorites,
                    },
                ));
                a.touch();
            }
        }
        // The composer ends with this emoji (or with none): suggest stickers.
        "expr-suggest" => {
            let arg: String = arg.chars().take(8).collect();
            if a.expression.suggest != arg {
                a.expression.suggest = arg;
                a.touch();
            }
        }
        "expr-emoji-delete" => {
            if let (Some(g), Some(e)) = (a.guild, id(arg)) {
                a.command(Command::Launch(LaunchCommand::DeleteEmoji {
                    guild_id: g,
                    emoji_id: e,
                }));
            }
        }
        "expr-sticker-delete" => {
            if let (Some(g), Some(s)) = (a.guild, id(arg)) {
                a.command(Command::Expression(ExpressionCommand::DeleteSticker {
                    guild_id: g,
                    sticker_id: s,
                }));
            }
        }
        "expr-typing-reset" => {
            a.command(Command::Expression(ExpressionCommand::SetTypingStyle {
                style: None,
            }));
        }
        _ => {}
    }
}
fn action_dialog(app: &Shared, name: &str) {
    super::operations::action(app.clone(), &format!("dialog:{name}"));
}

/// One emoji typed in a settings field: a Unicode emoji, `:shortcode:`, or a
/// custom emoji's `:name:`.
fn typed_emoji(a: &App, raw: &str) -> Result<Option<String>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    if let Some(name) = raw.strip_prefix(':').and_then(|r| r.strip_suffix(':')) {
        if let Some(e) = by_name(a, name) {
            return Ok(Some(e.token()));
        }
        if let Some(e) = sticktalk_emoji::shortcode_to_emoji(name) {
            return Ok(Some(e.into()));
        }
        return Err(format!("There’s no emoji called :{name}:."));
    }
    if custom_emoji_id(raw).is_some() || plausible_unicode_emoji(raw) {
        return Ok(Some(raw.into()));
    }
    Err("Enter one emoji in each box, like ✍️ or :pencil:.".into())
}

/// Clears a form after a successful upload (its fields and "unsaved" state).
fn reset_form(kind: &str) {
    let Some(form) = document()
        .query_selector(&format!("form[data-live-form=\"{kind}\"]"))
        .ok()
        .flatten()
    else {
        return;
    };
    if let Ok(reset) = js_sys::Reflect::get(&form, &"reset".into()) {
        if let Ok(reset) = reset.dyn_into::<js_sys::Function>() {
            let _ = reset.call0(&form);
        }
    }
}

pub(super) fn submit(app: Shared, form: &str) {
    // The page tracks unsaved forms; tell it this one is on its way.
    media_action(&format!("form-sent:{form}"));
    match form {
        "expr-typing" => {
            let mut a = app.borrow_mut();
            let mut emoji = vec![];
            for i in 0..MAX_TYPING_EMOJI {
                match typed_emoji(&a, &value(&format!("expr-typing-emoji-{i}"))) {
                    Ok(Some(e)) => emoji.push(e),
                    Ok(None) => {}
                    Err(problem) => {
                        a.error = problem;
                        media_action(&format!("form-failed:{form}"));
                        a.touch();
                        return;
                    }
                }
            }
            let motion = TypingMotion::parse(&value("expr-typing-motion")).unwrap_or_default();
            let verb = match value("expr-typing-verb").as_str() {
                "writing" => TypingVerb::Writing,
                "custom" => {
                    let custom = value("expr-typing-custom").trim().to_owned();
                    if !valid_typing_verb(&custom) {
                        a.error = "Use 1 to 20 letters for your own word, like “cooking”. Spaces, hyphens and apostrophes are fine between letters.".into();
                        media_action(&format!("form-failed:{form}"));
                        a.touch();
                        return;
                    }
                    TypingVerb::Custom(custom)
                }
                _ => TypingVerb::Typing,
            };
            a.command(Command::Expression(ExpressionCommand::SetTypingStyle {
                style: Some(TypingStyle {
                    emoji,
                    motion,
                    verb,
                }),
            }));
        }
        "expr-emoji-edit" => {
            let mut a = app.borrow_mut();
            let name = value("expr-edit-name").trim().to_owned();
            if !valid_emoji_name(&name) {
                a.error = "Emoji names are 2 to 32 letters, numbers or underscores.".into();
                media_action(&format!("form-failed:{form}"));
                a.touch();
                return;
            }
            if let (Some(g), Some(e)) = (a.guild, id(&value("expr-edit-id"))) {
                a.command(Command::Launch(LaunchCommand::RenameEmoji {
                    guild_id: g,
                    emoji_id: e,
                    name,
                }));
            }
        }
        "expr-sticker-edit" => {
            let mut a = app.borrow_mut();
            if let (Some(g), Some(s)) = (a.guild, id(&value("expr-edit-id"))) {
                a.command(Command::Expression(ExpressionCommand::UpdateSticker {
                    guild_id: g,
                    sticker_id: s,
                    name: value("expr-edit-name").trim().into(),
                    emoji: value("expr-edit-emoji").trim().into(),
                    description: value("expr-edit-description").trim().into(),
                }));
            }
        }
        "expr-emoji-upload" | "expr-sticker-upload" => {
            let sticker = form == "expr-sticker-upload";
            spawn_local(async move { upload(app, sticker).await });
        }
        _ => {}
    }
}

/// Reads the chosen file and posts it to the upload route.
async fn upload(app: Shared, sticker: bool) {
    let (http, epoch, guild) = {
        let a = app.borrow();
        (a.http(), a.epoch, a.guild)
    };
    let Some(g) = guild else { return };
    let kind = if sticker {
        "expr-sticker-upload"
    } else {
        "expr-emoji-upload"
    };
    let fail = |app: &Shared, text: String| {
        let mut a = app.borrow_mut();
        a.expression.busy = false;
        a.error = text;
        media_action(&format!("form-failed:{kind}"));
        a.touch();
    };
    let file = document()
        .get_element_by_id("expr-file")
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .and_then(|e| e.files())
        .and_then(|f| f.get(0));
    let Some(file) = file else {
        return fail(&app, "Choose an image first.".into());
    };
    let limit = if sticker {
        MAX_STICKER_BYTES
    } else {
        MAX_EMOJI_BYTES
    };
    if file.size() > limit as f64 {
        return fail(
            &app,
            format!(
                "That image is too large. {} can be up to {} KB.",
                if sticker { "Stickers" } else { "Emoji" },
                limit / 1024
            ),
        );
    }
    let name = value("expr-name").trim().to_owned();
    let encode = |v: &str| String::from(js_sys::encode_uri_component(v));
    let mut query = format!("name={}", encode(&name));
    if sticker {
        let emoji = value("expr-emoji").trim().to_owned();
        let description = value("expr-description").trim().to_owned();
        if !valid_sticker_name(&name) {
            return fail(
                &app,
                "Sticker names are 2 to 30 letters, numbers, spaces, hyphens or underscores."
                    .into(),
            );
        }
        if !plausible_unicode_emoji(&emoji) {
            return fail(&app, "Choose one related emoji, like 👋.".into());
        }
        query.push_str(&format!(
            "&emoji={}&description={}",
            encode(&emoji),
            encode(&description)
        ));
    } else if !valid_emoji_name(&name) {
        return fail(
            &app,
            "Emoji names are 2 to 32 letters, numbers or underscores.".into(),
        );
    }
    app.borrow_mut().expression.busy = true;
    app.borrow().touch();
    let bytes = match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
        Ok(buffer) => js_sys::Uint8Array::new(&buffer).to_vec(),
        Err(_) => return fail(&app, "Couldn’t read that file.".into()),
    };
    let path = format!(
        "/api/v1/guilds/{}/{}?{}",
        g.0,
        if sticker { "stickers" } else { "emoji" },
        query
    );
    let content_type = if file.type_().is_empty() {
        "application/octet-stream".to_owned()
    } else {
        file.type_()
    };
    let result = http
        .execute(
            "POST",
            &path,
            Body::Bytes(bytes),
            vec![("Content-Type".into(), content_type)],
            60_000,
        )
        .await
        .and_then(|r| decode_json::<serde_json::Value>(r).map(|_| ()));
    if app.borrow().epoch != epoch {
        return;
    }
    match result {
        Ok(()) => {
            {
                let mut a = app.borrow_mut();
                notice(
                    &mut a,
                    if sticker {
                        "Sticker added."
                    } else {
                        "Emoji added."
                    },
                );
                media_action(&format!("form-saved:{kind}"));
                reset_form(kind);
            }
            load_admin(app, g).await;
        }
        Err(e) => fail(&app, e),
    }
}

/// A command of ours was answered (moderation-style ownership).
pub(super) fn owns(op: &Command) -> bool {
    matches!(
        op,
        Command::Expression(_)
            | Command::Launch(
                LaunchCommand::RenameEmoji { .. } | LaunchCommand::DeleteEmoji { .. }
            )
    )
}
pub(super) fn answered(app: Shared, op: Command, result: CommandResult) {
    let mut a = app.borrow_mut();
    a.error.clear();
    a.notice.clear();
    match (&op, result) {
        (
            Command::Expression(ExpressionCommand::SendStickers {
                content,
                channel_id,
                ..
            }),
            CommandResult::Message(_),
        ) => {
            if !content.is_empty() {
                media_action(&format!("sent:{}", channel_id.0));
            }
            if a.dialog == "stickers" || a.dialog == "emoji" {
                a.dialog.clear();
            }
        }
        (_, CommandResult::Expression(ExpressionResult::Settings(s))) => {
            let typing = matches!(
                op,
                Command::Expression(ExpressionCommand::SetTypingStyle { .. })
            );
            a.expression.settings = *s;
            if typing {
                notice(&mut a, "Typing indicator saved.");
                media_action("form-saved:expr-typing");
            }
        }
        (Command::Launch(LaunchCommand::RenameEmoji { guild_id, .. }), _) => {
            media_action("form-saved:expr-emoji-edit");
            notice(&mut a, "Emoji renamed.");
            if a.dialog.starts_with("emoji-edit:") {
                a.dialog = "server-emoji".into();
            }
            refresh_admin(&a, *guild_id);
        }
        (Command::Launch(LaunchCommand::DeleteEmoji { guild_id, .. }), _) => {
            notice(&mut a, "Emoji deleted.");
            if a.dialog.starts_with("emoji-edit:") {
                a.dialog = "server-emoji".into();
            }
            refresh_admin(&a, *guild_id);
        }
        (Command::Expression(ExpressionCommand::UpdateSticker { guild_id, .. }), _) => {
            media_action("form-saved:expr-sticker-edit");
            notice(&mut a, "Sticker saved.");
            if a.dialog.starts_with("sticker-edit:") {
                a.dialog = "server-stickers".into();
            }
            refresh_admin(&a, *guild_id);
        }
        (Command::Expression(ExpressionCommand::DeleteSticker { guild_id, .. }), _) => {
            notice(&mut a, "Sticker deleted.");
            if a.dialog.starts_with("sticker-edit:") {
                a.dialog = "server-stickers".into();
            }
            refresh_admin(&a, *guild_id);
        }
        (_, CommandResult::Expression(ExpressionResult::SuperReaction(update))) => {
            // The event updates everyone else; apply our own answer at once.
            event(
                &mut a,
                ExpressionEvent::SuperReactionUpdated(ReactionUpdate {
                    added: false,
                    ..*update
                }),
            );
        }
        _ => {}
    }
    a.touch();
}

/// Is a message made only of emoji (and at most 27 of them)? Then they are big.
pub(super) fn jumbo(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || text.len() > 600 {
        return false;
    }
    let tokens = emoji_tokens(text);
    let mut count = tokens.len();
    let mut last = 0;
    let mut rest = String::new();
    for (range, _, _) in &tokens {
        rest.push_str(&text[last..range.start]);
        rest.push(' ');
        last = range.end;
    }
    rest.push_str(&text[last..]);
    let (mut joined, mut regional, mut keycap) = (false, 0, false);
    for c in rest.chars() {
        let v = c as u32;
        match v {
            // Joiners, variation selectors, skin tones and tags extend the emoji before them.
            0x200D => joined = true,
            0xFE0E | 0xFE0F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F => {}
            0x20E3 if keycap => {}
            0x1F1E6..=0x1F1FF => {
                regional += 1;
                if regional % 2 == 1 {
                    count += 1;
                }
            }
            _ if c.is_whitespace() => joined = false,
            _ if pictographic(v) => {
                if !joined {
                    count += 1;
                }
                joined = false;
            }
            _ if c == '#' || c == '*' || c.is_ascii_digit() => {
                keycap = true;
                count += 1;
                continue;
            }
            _ => return false,
        }
        if v != 0xFE0F {
            keycap = false;
        }
    }
    (1..=27).contains(&count)
}
fn pictographic(v: u32) -> bool {
    matches!(
        v,
        0x1F000..=0x1FAFF
            | 0x2600..=0x27BF
            | 0x2300..=0x23FF
            | 0x2B00..=0x2BFF
            | 0x2190..=0x21FF
            | 0x25A0..=0x25FF
            | 0x2934
            | 0x2935
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
            | 0x00A9
            | 0x00AE
            | 0x203C
            | 0x2049
            | 0x2122
            | 0x2139
            | 0x24C2
    )
}

/// The expression feature's one entry point (`features.rs`).
#[leptos::wasm_split::wasm_split(
    expression,
    wasm_split_path = ::leptos::wasm_split,
    fallible,
    preload(prefetch)
)]
pub(super) fn run(app: Shared, call: features::Call) -> features::Outcome {
    use features::Call;
    if app.borrow().expression.stale {
        ready(app.clone());
    }
    match call {
        Call::Action(kind, arg) => action(app, &kind, &arg),
        Call::Other("admin", g) => {
            if let Some(g) = id(&g) {
                spawn_local(async move { load_admin(app, g).await });
            }
        }
        Call::Submit(form) => submit(app, &form),
        Call::Opened(name) => opened(app, &name),
        _ => {}
    }
    Ok(true)
}

/// The expression feature's state: its events, the answers to its commands and the fresh snapshot (Ready).
/// A separate, small split entry: keeping a feature's state current does not
/// load its pages and forms (`features.rs`).
#[leptos::wasm_split::wasm_split(
    expression_state,
    wasm_split_path = ::leptos::wasm_split,
    fallible
)]
pub(super) fn state(app: Shared, call: features::Call) -> features::Outcome {
    use features::{Call, Reply};
    match call {
        Call::Ready => ready(app),
        Call::Event(raw) => {
            let Ok(e) = crate::inbound::feature::expression_event(&raw.data) else {
                return Ok(false);
            };
            event(&mut app.borrow_mut(), e);
        }
        Call::Answer(Some(op), reply) => {
            let result = match reply {
                Reply::Core(result) => result,
                Reply::Raw(raw) => match crate::inbound::feature::expression_result(&raw.data) {
                    Ok(r) => CommandResult::Expression(r),
                    Err(()) => return Ok(false),
                },
            };
            answered(app, op, result);
        }
        Call::State("resolve", _) => resolve(app),

        _ => {}
    }
    Ok(true)
}

/// Dialogs whose opening this feature handles (`opened`).
pub(super) fn owns_dialog(name: &str) -> bool {
    name == "emoji" || name == "stickers" || name.starts_with("emoji-react:") || admin_page(name)
}
