//! Expression views: custom emoji in text and reactions, the emoji and sticker
//! pickers, sticker messages, Super Reaction chips, typing indicators, and the
//! server and user settings pages for them. State and actions are in
//! `live/expression.rs`; states and motion are in `styles/expression.css`.
use super::*;
use crate::live::expression::{self as ex, Look};

const GRID: &str = "emoji-grid mt-2 grid grid-cols-8 gap-1 phone:grid-cols-6 touch:grid-cols-[repeat(auto-fill,minmax(44px,1fr))]";
/// Emoji drawn when the picker opens: sixteen rows of eight (the picker shows
/// about fourteen at 1440x900, and fewer, of six, on a phone), then
/// `PICKER_STEP` more each time the end comes within two rows of view.
pub(super) const PICKER_WINDOW: usize = 128;
const PICKER_STEP: usize = 64;
const CELL: &str = "grid min-h-10 place-items-center rounded-md text-center text-[28px] hover:bg-accent focus-visible:bg-accent touch:min-h-11";

/// A custom emoji image. Animated ones carry a still first frame too: with
/// reduced motion only the still shows, until hovered or focused.
pub(super) fn emoji_image(e: Id, name: &str, animated: bool, class: &str) -> AnyView {
    let alt = format!(":{name}:");
    if animated {
        view!{<span class=format!("custom-emoji animated {class}") role="img" aria-label=alt.clone() title=alt><img class="emoji-still" src=format!("/api/v1/emoji/{}/preview",e.0) alt="" loading="lazy" decoding="async" draggable="false"/><img class="emoji-motion" src=format!("/api/v1/emoji/{}",e.0) alt="" loading="lazy" decoding="async" draggable="false"/></span>}.into_any()
    } else {
        view!{<img class=format!("custom-emoji {class}") src=format!("/api/v1/emoji/{}",e.0) alt=alt.clone() title=alt loading="lazy" decoding="async" draggable="false"/>}.into_any()
    }
}

/// A `<:name:id>` token anywhere: the image, or `:name:` once it is gone.
fn token_view(a: &App, e: Id, name: &str, class: &str) -> AnyView {
    match ex::look(a, e) {
        Look::Known(n, animated, _) => emoji_image(e, &n, animated, class),
        Look::Gone(n) => view! {<span class="emoji-gone">{format!(":{}:", if n.is_empty() { name } else { &n })}</span>}.into_any(),
        Look::Unknown => {
            ex::want(a, e);
            emoji_image(e, name, false, class)
        }
    }
}

/// A run of message text: custom emoji tokens become images that say which
/// server they are from when clicked.
pub(super) fn text(a: &App, t: &str) -> AnyView {
    let tokens = emoji_tokens(t);
    if tokens.is_empty() {
        return text_links(a, t);
    }
    let mut parts = vec![];
    let mut last = 0;
    for (range, name, e) in tokens {
        if range.start > last {
            parts.push(text_links(a, &t[last..range.start]));
        }
        let gone = matches!(ex::look(a, e), Look::Gone(_));
        parts.push(if gone {
            token_view(a, e, name, "")
        } else {
            view!{<button type="button" class="emoji-button" data-live=format!("dialog:emoji-info:{}",e.0) aria-label=format!("Emoji :{name}:. Show details")>{token_view(a,e,name,"")}</button>}.into_any()
        });
        last = range.end;
    }
    if last < t.len() {
        parts.push(text_links(a, &t[last..]));
    }
    parts.collect_view().into_any()
}

/// Message text without tokens, for one-line previews: `:name:`.
pub(super) fn plain(text: &str) -> String {
    let tokens = emoji_tokens(text);
    if tokens.is_empty() {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut last = 0;
    for (range, name, _) in tokens {
        out.push_str(&text[last..range.start]);
        out.push_str(&format!(":{name}:"));
        last = range.end;
    }
    out.push_str(&text[last..]);
    out
}

/// A reaction's emoji: Unicode as text, a custom one as its image.
fn glyph(a: &App, emoji: &str) -> AnyView {
    match custom_emoji_id(emoji) {
        Some(e) => token_view(a, e, "emoji", "reaction-emoji"),
        None => view! {<span class="reaction-emoji">{emoji.to_owned()}</span>}.into_any(),
    }
}
/// One emoji (a reaction, a poll answer's emoji), not interactive.
pub(super) fn emoji_text(a: &App, emoji: &str) -> AnyView {
    glyph(a, emoji)
}
fn spoken(a: &App, emoji: &str) -> String {
    match custom_emoji_id(emoji) {
        Some(e) => match ex::look(a, e) {
            Look::Known(n, _, _) | Look::Gone(n) if !n.is_empty() => format!(":{n}:"),
            _ => "a custom emoji".into(),
        },
        None => emoji.to_owned(),
    }
}

fn chip(a: &App, m: &Message, r: &ReactionSummary, burst: bool) -> AnyView {
    let normal = format!("react:{},{}", m.id.0, r.emoji);
    let sup = format!("super-react:{},{}", m.id.0, r.emoji);
    let (action, alt) = if burst { (sup, normal) } else { (normal, sup) };
    let now = Browser.unix_ms();
    let playing = burst
        .then(|| {
            a.expression
                .bursts
                .iter()
                .find(|(id, e, at)| {
                    *id == m.id && *e == r.emoji && now.saturating_sub(*at) < ex::BURST_MS
                })
                .map(|(_, _, at)| now.saturating_sub(*at))
        })
        .flatten();
    let class = format!(
        "reaction{}{}{} relative inline-flex items-center gap-1 rounded-lg border px-2 py-1 text-sm text-foreground touch:min-h-11 touch:min-w-11 {}",
        if r.me { " mine" } else { "" },
        if burst { " super" } else { "" },
        if playing.is_some() { " super-burst" } else { "" },
        if r.me { "border-primary bg-primary/15" } else { "border-transparent bg-muted" }
    );
    let label = format!(
        "{} with {}, {} {}. {}",
        if burst { "Super React" } else { "React" },
        spoken(a, &r.emoji),
        r.count,
        if r.count == 1 {
            "reaction"
        } else {
            "reactions"
        },
        if burst {
            "Shift+click for a normal reaction."
        } else {
            "Shift+click or press and hold for a Super Reaction."
        }
    );
    view!{<button class=class data-live=action data-live-alt=alt aria-label=label aria-pressed=r.me.to_string() style=playing.map(|t|format!("--burst-delay:-{t}ms"))>{if burst{icon_with("sparkles","icon super-mark size-3.5")}else{().into_any()}}{glyph(a,&r.emoji)}<span class="reaction-count">{r.count}</span></button>}.into_any()
}

/// Normal reactions, each followed by its Super Reaction chip (own count).
pub(super) fn reactions(a: &App, m: &Message) -> AnyView {
    let supers = m.extra.super_reactions();
    let mut chips = vec![];
    for r in &m.reactions {
        chips.push(chip(a, m, r, false));
        if let Some(s) = supers.iter().find(|s| s.emoji == r.emoji) {
            chips.push(chip(a, m, s, true));
        }
    }
    for s in supers
        .iter()
        .filter(|s| !m.reactions.iter().any(|r| r.emoji == s.emoji))
    {
        chips.push(chip(a, m, s, true));
    }
    view! {<div class="reactions mt-1 flex flex-wrap gap-1 empty:hidden">{chips}</div>}.into_any()
}

/// The stickers a message carries, at 160 px.
pub(super) fn stickers(m: &Message) -> AnyView {
    if m.extra.stickers().is_empty() {
        return ().into_any();
    }
    view!{<div class="message-stickers mt-1 flex flex-wrap gap-2">{m.extra.stickers().iter().map(|s|{
        let label=if s.description.is_empty(){format!("Sticker: {}",s.name)}else{format!("Sticker: {}. {}",s.name,s.description)};
        if s.deleted{
            view!{<div class="sticker-gone grid size-40 place-items-center rounded-lg border border-dashed p-3 text-center text-sm text-subtle" role="img" aria-label=label>{format!("{} (sticker deleted)",s.name)}</div>}.into_any()
        }else if s.animated{
            view!{<span class="sticker animated block size-40" role="img" aria-label=label title=s.name.clone()><img class="emoji-still size-40 object-contain" src=s.still_url() alt="" loading="lazy" width="160" height="160" draggable="false"/><img class="emoji-motion size-40 object-contain" src=s.url() alt="" loading="lazy" width="160" height="160" draggable="false"/></span>}.into_any()
        }else{
            view!{<img class="sticker block size-40 object-contain" src=s.url() alt=label title=s.name.clone() loading="lazy" width="160" height="160" draggable="false"/>}.into_any()
        }
    }).collect_view()}</div>}.into_any()
}

/// Extra hover action: add a Super Reaction.
pub(super) fn message_actions(m: &Message) -> AnyView {
    ib_plain(
        "sparkles",
        "Add Super Reaction",
        &format!("expr-super-picker:{}", m.id.0),
    )
}

/// The composer's sticker button, beside the emoji button.
pub(super) fn composer_button() -> AnyView {
    ib_plain("sticker", "Sticker Picker", "dialog:stickers")
}

/// Stickers whose related emoji was just typed, above the composer.
pub(super) fn suggestions(a: &App) -> AnyView {
    let wanted = a.expression.suggest.trim_end_matches('\u{fe0f}');
    if wanted.is_empty() {
        return ().into_any();
    }
    let found: Vec<&Sticker> = usable_stickers(a)
        .into_iter()
        .flat_map(|(_, list)| list)
        .filter(|s| s.emoji.trim_end_matches('\u{fe0f}') == wanted)
        .take(6)
        .collect();
    if found.is_empty() {
        return ().into_any();
    }
    view!{<div class="sticker-suggestions mx-4 mb-1 flex items-center gap-2 overflow-x-auto rounded-lg border bg-popover p-2 shadow-sm" role="group" aria-label="Suggested stickers"><small class="flex-none px-1 text-subtle">"Stickers"</small>{found.into_iter().map(|s|view!{<button class="sticker-suggestion size-16 flex-none rounded-md p-1 hover:bg-accent touch:size-[72px]" data-live=format!("expr-sticker-send:{}",s.id.0) aria-label=format!("Send sticker {}",s.name) title=s.name.clone()>{sticker_thumb(s)}</button>}).collect_view()}{ib_plain("x","Hide sticker suggestions","expr-suggest:")}</div>}.into_any()
}

fn sticker_thumb(s: &Sticker) -> AnyView {
    let item = s.item();
    if s.animated {
        view!{<span class="sticker animated block size-full"><img class="emoji-still size-full object-contain" src=item.still_url() alt="" loading="lazy" draggable="false"/><img class="emoji-motion size-full object-contain" src=item.url() alt="" loading="lazy" draggable="false"/></span>}.into_any()
    } else {
        view!{<img class="size-full object-contain" src=item.url() alt="" loading="lazy" draggable="false"/>}.into_any()
    }
}

/// The custom indicator when exactly one person is typing and chose one.
pub(super) fn typing(a: &App, c: Id) -> AnyView {
    let now = Browser.unix_ms();
    let typists: Vec<Id> = a
        .model
        .typing
        .iter()
        .filter(|((channel, _), expiry)| *channel == c && **expiry > now)
        .map(|((_, u), _)| *u)
        .collect();
    if let [one] = typists.as_slice() {
        if let Some(style) = a.expression.typing.get(one) {
            return typing_view(a, &a.model.name(*one), style);
        }
    }
    let names: Vec<String> = typists.iter().take(3).map(|u| a.model.name(*u)).collect();
    view! {{crate::wire::typing_label(&names)}}.into_any()
}
fn typing_view(a: &App, name: &str, style: &TypingStyle) -> AnyView {
    view!{<span class=format!("typing-custom motion-{} inline-flex items-center gap-1.5",style.motion.name())><span class="typing-emoji inline-flex gap-0.5" aria-hidden="true">{style.emoji.iter().enumerate().map(|(i,e)|view!{<span class="typing-glyph inline-block" style=format!("--i:{i}")>{match custom_emoji_id(e){Some(id)=>token_view(a,id,"emoji","typing-custom-emoji"),None=>view!{<span>{e.clone()}</span>}.into_any()}}</span>}).collect_view()}</span><span>{format!("{name} {}…",style.verb.phrase())}</span></span>}.into_any()
}

/// The icon in a `:` completion row for a custom emoji.
pub(super) fn completion_icon(a: &App, insert: &str) -> AnyView {
    let name = insert.trim_matches(':');
    let found = ex::usable(a)
        .into_iter()
        .find(|(_, e)| e.name == name || custom_emoji_id(insert) == Some(e.id));
    match found {
        Some((_, e)) => emoji_image(e.id, &e.name, e.animated, "size-6"),
        None => ().into_any(),
    }
}

fn guild_name(a: &App, g: Id) -> String {
    a.model
        .guilds
        .get(&g)
        .map_or("Server".into(), |g| g.guild.name.clone())
}
fn guild_icon(a: &App, g: Option<Id>) -> AnyView {
    let Some(g) = g else {
        return view! {<img class="size-5 rounded-md" src="/public/icons/icon-192.png" alt=""/>}
            .into_any();
    };
    match a.model.guilds.get(&g).and_then(|v| v.guild.icon_id) {
        Some(icon) => view!{<img class="size-5 rounded-md object-cover" src=format!("/api/v1/files/{}/thumbnail",icon.0) alt="" loading="lazy"/>}.into_any(),
        None => view!{<span class="grid size-5 place-items-center rounded-md bg-muted text-[10px] font-semibold">{guild_name(a,g).chars().take(2).collect::<String>()}</span>}.into_any(),
    }
}
fn section(a: &App, g: Option<Id>, title: &str, body: AnyView) -> AnyView {
    view!{<section class="picker-section mt-4" aria-label=title.to_owned()><h3 class="category mb-1 flex items-center gap-2 text-xs font-semibold uppercase text-subtle">{guild_icon(a,g)}<span class="truncate">{title.to_owned()}</span></h3>{body}</section>}.into_any()
}

fn tabs(current: &str) -> AnyView {
    let tab = |label: &str, target: &str, live: &str| {
        let on = current == target;
        view!{<button class=if on{"button secondary"}else{"link-button"} role="tab" aria-selected=on.to_string() data-live=live.to_owned()>{label.to_owned()}</button>}.into_any()
    };
    view!{<div class="picker-tabs mb-3 flex flex-wrap gap-2" role="tablist" aria-label="Pickers">{tab("Emoji","emoji","dialog:emoji")}{tab("Stickers","stickers","dialog:stickers")}{tab("GIFs","gif","dialog:gif")}<button class="link-button phone:hidden" data-live="dialog:kaomoji">"Kaomoji"</button></div>}.into_any()
}

/// A custom emoji in a picker: insert it, or react with it.
fn custom_cell(e: &GuildEmoji, react: &Option<String>) -> AnyView {
    let action = match react {
        Some(m) => format!("expr-react:{m},{}", e.token()),
        None => format!("expr-emoji:{}", e.token()),
    };
    view!{<button class=CELL title=format!(":{}:",e.name) aria-label=format!(":{}:",e.name) data-live=action>{emoji_image(e.id,&e.name,e.animated,"size-8")}</button>}.into_any()
}
fn unicode_cell(glyph: &str, name: &str, react: &Option<String>) -> AnyView {
    let action = match react {
        Some(m) => format!("expr-react:{m},{glyph}"),
        None => format!("emoji:{glyph}"),
    };
    view!{<button class=CELL title=name.to_owned() aria-label=name.to_owned() data-live=action>{glyph.to_owned()}</button>}.into_any()
}

fn emoji_picker(a: &App, react: Option<String>) -> AnyView {
    let query = a.query.to_lowercase();
    let usable = ex::usable(a);
    let mut by_guild: Vec<(Id, Vec<&GuildEmoji>)> = vec![];
    for (g, e) in &usable {
        if !query.is_empty() && !e.name.to_lowercase().contains(&query) {
            continue;
        }
        match by_guild.iter_mut().find(|(v, _)| v == g) {
            Some((_, list)) => list.push(e),
            None => by_guild.push((*g, vec![*e])),
        }
    }
    let categories = sticktalk_emoji::categories();
    let category = if categories.contains(&a.expression.category.as_str()) {
        a.expression.category.as_str()
    } else {
        categories[0]
    };
    let bundled: Vec<sticktalk_emoji::Emoji> = if query.is_empty() {
        sticktalk_emoji::by_category(category).take(400).collect()
    } else {
        sticktalk_emoji::search(&query, 240)
    };
    let recent: Vec<AnyView> = if query.is_empty() {
        a.expression
            .recent
            .iter()
            .filter_map(|r| match custom_emoji_id(r) {
                Some(id) => usable
                    .iter()
                    .find(|(_, e)| e.id == id)
                    .map(|(_, e)| custom_cell(e, &react)),
                None => Some(unicode_cell(
                    r,
                    sticktalk_emoji::emoji_to_shortcode(r).unwrap_or(r),
                    &react,
                )),
            })
            .take(16)
            .collect()
    } else {
        vec![]
    };
    let nothing = by_guild.is_empty() && bundled.is_empty();
    let nothing_bundled = bundled.is_empty();
    // Only the first rows are drawn (a category has up to 400 emoji); more
    // follow as the grid scrolls (`data-emoji-more`, public/live.js).
    let w = &a.expression.window;
    let limit = if w.0 == a.expression.category && w.1 == a.query {
        w.2.max(PICKER_WINDOW)
    } else {
        PICKER_WINDOW
    };
    let more = (bundled.len() > limit).then(|| format!("expr-more:{}", limit + PICKER_STEP));
    let bundled = bundled.into_iter().take(limit);
    view!{
        <h2>{if react.is_some(){"Add Reaction"}else{"Emoji"}}</h2>
        {match &react{
            None=>tabs("emoji"),
            Some(_)=>view!{<div class="super-toggle mb-3 flex items-center gap-3 rounded-lg border p-2"><button class=if a.expression.super_mode{"button primary"}else{"button secondary"} aria-pressed=a.expression.super_mode.to_string() data-live="expr-super-mode">{icon_with("sparkles","icon size-4")}"Super Reaction"</button><small class="muted-copy">{if a.expression.super_mode{"Your reaction bursts for everyone viewing it."}else{"Turn on to add a Super Reaction."}}</small></div>}.into_any(),
        }}
        <label class="search flex"><span class="sr-only">"Search Emoji"</span><input class="w-full" id="emoji-query" type="search" value=a.query.clone() placeholder="Search all emoji" data-emoji-query autocomplete="off"/></label>
        {if nothing{view!{<p class="muted-copy mt-4">"No emoji match that search."</p>}.into_any()}else{().into_any()}}
        {by_guild.into_iter().map(|(g,list)|section(a,Some(g),&guild_name(a,g),view!{<div class=GRID>{list.into_iter().map(|e|custom_cell(e,&react)).collect_view()}</div>}.into_any())).collect_view()}
        {if recent.is_empty(){().into_any()}else{section(a,None,"Recently Used",view!{<div class=GRID>{recent}</div>}.into_any())}}
        {if query.is_empty(){view!{<nav class="emoji-categories mt-4 flex gap-1 overflow-x-auto" aria-label="Emoji categories">{categories.iter().map(|c|view!{<button class=if *c==category{"button secondary flex-none text-xs"}else{"link-button flex-none text-xs"} aria-pressed=(*c==category).to_string() data-live=format!("expr-category:{c}")>{*c}</button>}).collect_view()}</nav>}.into_any()}else{().into_any()}}
        {if nothing_bundled{().into_any()}else{view!{<section class="picker-section mt-2" aria-label=if query.is_empty(){category.to_owned()}else{"Search results".to_owned()}><div class=GRID>{bundled.map(|e|unicode_cell(e.emoji(),e.name(),&react)).collect_view()}</div>{more.map(|m|view!{<div class="h-px" data-emoji-more=m aria-hidden="true"></div>})}</section>}.into_any()}}
    }.into_any()
}

/// Built-in pack first, then each server the account is in.
fn usable_stickers(a: &App) -> Vec<(Option<Id>, &Vec<Sticker>)> {
    let mut sets: Vec<(Option<Id>, &Vec<Sticker>)> = a
        .expression
        .stickers
        .iter()
        .filter(|(g, _)| g.is_none_or(|g| a.model.guilds.contains_key(&g)))
        .map(|(g, l)| (*g, l))
        .collect();
    sets.sort_by_key(|(g, _)| (g.is_some(), *g != a.guild, *g));
    sets
}

fn sticker_cell(a: &App, s: &Sticker) -> AnyView {
    let picked = a.expression.tray.contains(&s.id);
    let favorite = a.expression.settings.sticker_favorites.contains(&s.id);
    view!{<div class=format!("sticker-cell group/sticker relative rounded-lg {}",if picked{"ring-2 ring-primary"}else{""})>
        <button class="sticker-pick block size-24 rounded-lg p-1 hover:bg-accent focus-visible:bg-accent phone:size-[88px]" data-live=format!("expr-sticker:{}",s.id.0) aria-label=format!("Send sticker {}{}",s.name,if s.description.is_empty(){String::new()}else{format!(". {}",s.description)}) title=s.name.clone()>{sticker_thumb(s)}</button>
        <div class="sticker-tools absolute top-0.5 right-0.5 flex gap-0.5">
            <button class=format!("icon-button size-7 rounded-full bg-popover/90 {}",if favorite{"gif-star is-starred"}else{""}) aria-label=format!("{} {}",if favorite{"Remove from favorites:"}else{"Add to favorites:"},s.name) aria-pressed=favorite.to_string() data-live=format!("expr-sticker-fav:{}",s.id.0)>{icon_with("star","icon size-3.5")}</button>
            <button class="icon-button size-7 rounded-full bg-popover/90" aria-label=format!("{} {}",if picked{"Take out of this message:"}else{"Add to this message:"},s.name) aria-pressed=picked.to_string() data-live=format!("expr-sticker-tray:{}",s.id.0)>{icon_with(if picked{"check"}else{"plus"},"icon size-3.5")}</button>
        </div>
    </div>}.into_any()
}

fn sticker_picker(a: &App) -> AnyView {
    let query = a.query.to_lowercase();
    let sets = usable_stickers(a);
    let matches = |s: &Sticker| {
        query.is_empty()
            || s.name.to_lowercase().contains(&query)
            || s.emoji.contains(query.as_str())
            || s.description.to_lowercase().contains(&query)
    };
    let all: Vec<&Sticker> = sets.iter().flat_map(|(_, l)| l.iter()).collect();
    let favorites: Vec<&Sticker> = a
        .expression
        .settings
        .sticker_favorites
        .iter()
        .filter_map(|id| all.iter().find(|s| s.id == *id).copied())
        .filter(|s| matches(s))
        .collect();
    let recent: Vec<&Sticker> = if query.is_empty() {
        a.expression
            .recent_stickers
            .iter()
            .filter_map(|id| all.iter().find(|s| s.id == *id).copied())
            .collect()
    } else {
        vec![]
    };
    let grid = "sticker-grid mt-2 grid grid-cols-[repeat(auto-fill,minmax(96px,1fr))] gap-2 phone:grid-cols-[repeat(auto-fill,minmax(88px,1fr))]";
    let found = sets.iter().any(|(_, l)| l.iter().any(&matches));
    let tray = &a.expression.tray;
    view!{
        <h2>"Stickers"</h2>{tabs("stickers")}
        <label class="search flex"><span class="sr-only">"Search Stickers"</span><input class="w-full" id="emoji-query" type="search" value=a.query.clone() placeholder="Search stickers" data-emoji-query autocomplete="off"/></label>
        {if tray.is_empty(){view!{<p class="muted-copy mt-2 text-xs">"Choose a sticker to send it. Use + to put up to 3 on one message with what you’ve typed."</p>}.into_any()}else{view!{<div class="sticker-tray sticky top-0 z-[2] mt-3 flex items-center gap-2 rounded-lg border bg-popover p-2" role="status"><span class="flex-1 text-sm">{format!("{} sticker{} on this message",tray.len(),if tray.len()==1{""}else{"s"})}</span><button class="button primary" data-live="expr-sticker-tray:send">{if tray.len()==1{"Send Sticker".to_owned()}else{format!("Send {} Stickers",tray.len())}}</button></div>}.into_any()}}
        {if !a.expression.loaded{view!{<p class="muted-copy mt-4" role="status">"Loading stickers…"</p>}.into_any()}else if !found{view!{<p class="muted-copy mt-4">"No stickers match that search."</p>}.into_any()}else{().into_any()}}
        {if favorites.is_empty(){().into_any()}else{section(a,None,"Favorites",view!{<div class=grid>{favorites.into_iter().map(|s|sticker_cell(a,s)).collect_view()}</div>}.into_any())}}
        {if recent.is_empty(){().into_any()}else{section(a,None,"Recently Used",view!{<div class=grid>{recent.into_iter().map(|s|sticker_cell(a,s)).collect_view()}</div>}.into_any())}}
        {sets.into_iter().filter(|(_,l)|l.iter().any(&matches)).map(|(g,l)|section(a,g,&g.map_or("StickTalk".into(),|g|guild_name(a,g)),view!{<div class=grid>{l.iter().filter(|s|matches(s)).map(|s|sticker_cell(a,s)).collect_view()}</div>}.into_any())).collect_view()}
    }.into_any()
}

/// Which server a custom emoji in a message is from.
fn emoji_info(a: &App, e: Id) -> AnyView {
    let (name, animated, guild) = match ex::look(a, e) {
        Look::Known(n, animated, g) => (n, animated, g),
        Look::Gone(n) => (n, false, None),
        Look::Unknown => {
            ex::want(a, e);
            ("emoji".into(), false, None)
        }
    };
    let resolved = a.expression.resolved.borrow().get(&e).cloned();
    let from = match (guild, resolved.as_ref().and_then(|r| r.guild_name.clone())) {
        (Some(g), _) => format!("From {}", guild_name(a, g)),
        (None, Some(n)) => format!("From {n}"),
        (None, None) if matches!(ex::look(a, e), Look::Gone(_)) => "This emoji was deleted.".into(),
        _ => "From a server you’re not in. You can see it because it was used here.".into(),
    };
    view!{<div class="emoji-info flex items-center gap-4"><div class="grid size-16 flex-none place-items-center rounded-lg bg-muted">{emoji_image(e,&name,animated,"size-12")}</div><div class="min-w-0"><h2 class="m-0 wrap-anywhere">{format!(":{name}:")}</h2><p class="muted-copy m-0">{from}</p>{if animated{view!{<p class="muted-copy m-0 text-xs">"Animated emoji"</p>}.into_any()}else{().into_any()}}</div></div>}.into_any()
}

fn slots(used: usize, max: u32, label: &str) -> AnyView {
    view! {<p class="slot-count muted-copy m-0">{format!("{used} of {max} {label} slots used")}</p>}
        .into_any()
}
fn creator(admin: &GuildExpressionAdmin, u: Option<Id>) -> String {
    u.and_then(|u| admin.users.iter().find(|v| v.id == u))
        .map_or("Unknown".into(), |u| format!("Added by {}", name(u)))
}
fn upload_form(kind: &str, busy: bool) -> AnyView {
    let sticker = kind == "expr-sticker-upload";
    view!{<form data-live-form=kind.to_owned() class="upload-form mt-4 grid gap-1 rounded-lg border p-4">
        <h3 class="m-0">{if sticker{"Upload a Sticker"}else{"Upload an Emoji"}}</h3>
        <p class="muted-copy m-0">{if sticker{"PNG, APNG, GIF or WebP, up to 512 KB. Shown at 160×160. Animated stickers can be at most 320×320."}else{"PNG, JPEG, GIF or WebP (animated GIF, WebP and APNG work too), up to 256 KB. Resized to 128×128."}}</p>
        <div class="field"><label for="expr-file">"Image"</label><input id="expr-file" type="file" required accept=if sticker{"image/png,image/apng,image/gif,image/webp"}else{"image/png,image/apng,image/jpeg,image/gif,image/webp"}/></div>
        <div class="field"><label for="expr-name">"Name"</label><input id="expr-name" type="text" required maxlength=if sticker{"30"}else{"32"} autocomplete="off" aria-describedby="expr-name-help"/><small id="expr-name-help">{if sticker{"2 to 30 letters, numbers, spaces, hyphens or underscores."}else{"2 to 32 letters, numbers or underscores. People type it as :name:."}}</small></div>
        {if sticker{view!{<div class="field"><label for="expr-emoji">"Related Emoji"</label><input id="expr-emoji" type="text" required maxlength="16" placeholder="👋" autocomplete="off" aria-describedby="expr-emoji-help"/><small id="expr-emoji-help">"Typing this emoji suggests the sticker."</small></div><div class="field"><label for="expr-description">"Description (for screen readers)"</label><input id="expr-description" type="text" maxlength="100" autocomplete="off"/></div>}.into_any()}else{().into_any()}}
        <button type="submit" class="button primary justify-self-start" disabled=busy>{if busy{"Uploading…"}else{"Upload"}}</button>
    </form>}.into_any()
}

fn server_emoji(a: &App) -> AnyView {
    let allowed = settings::can(a, Permissions::MANAGE_EMOJI);
    if !allowed {
        return view!{<h2>"Emoji"</h2><p class="muted-copy">"You need Manage Expressions permission to manage this server’s emoji. Everyone can use them."</p>}.into_any();
    }
    let Some((_, admin)) = a
        .expression
        .admin
        .as_ref()
        .filter(|(g, _)| Some(*g) == a.guild)
    else {
        return view! {<h2>"Emoji"</h2><p class="muted-copy" role="status">"Loading…"</p>}
            .into_any();
    };
    let (moving, still): (Vec<&EmojiDetail>, Vec<&EmojiDetail>) =
        admin.emoji.iter().partition(|e| e.emoji.animated);
    let list = |title: &str, items: Vec<&EmojiDetail>| {
        view!{<h3 class="category mt-6 mb-2">{title.to_owned()}</h3>{if items.is_empty(){view!{<p class="muted-copy">"None yet."</p>}.into_any()}else{items.into_iter().map(|e|view!{<div class="settings-list-row flex flex-wrap items-center gap-3 border-b py-3" data-emoji-row=e.emoji.name.clone()>{emoji_image(e.emoji.id,&e.emoji.name,e.emoji.animated,"size-8")}<span class="settings-row-copy flex min-w-24 flex-1 flex-col gap-1 wrap-anywhere"><strong class="font-semibold">{format!(":{}:",e.emoji.name)}</strong><small>{creator(admin,e.creator_id)}</small></span><button class="icon-button" aria-label=format!("Edit :{}:",e.emoji.name) data-live=format!("dialog:emoji-edit:{}",e.emoji.id.0)>{icon("pencil")}</button></div>}).collect_view().into_any()}}}.into_any()
    };
    view!{<h2>"Emoji"</h2><p class="muted-copy">"Add up to 250 still and 250 animated emoji for everyone in this server. Members can also use them in other servers and in direct messages."</p>
        {slots(still.len(),admin.max_static_emoji,"still")}{slots(moving.len(),admin.max_animated_emoji,"animated")}
        {upload_form("expr-emoji-upload",a.expression.busy)}
        {list("Still Emoji",still)}{list("Animated Emoji",moving)}
    }.into_any()
}

fn server_stickers(a: &App) -> AnyView {
    if !settings::can(a, Permissions::MANAGE_EMOJI) {
        return view!{<h2>"Stickers"</h2><p class="muted-copy">"You need Manage Expressions permission to manage this server’s stickers. Everyone can use them."</p>}.into_any();
    }
    let Some((_, admin)) = a
        .expression
        .admin
        .as_ref()
        .filter(|(g, _)| Some(*g) == a.guild)
    else {
        return view! {<h2>"Stickers"</h2><p class="muted-copy" role="status">"Loading…"</p>}
            .into_any();
    };
    view!{<h2>"Stickers"</h2><p class="muted-copy">"Add up to 60 stickers for everyone in this server. Members can also send them in other servers and in direct messages."</p>
        {slots(admin.stickers.len(),admin.max_stickers,"sticker")}
        {upload_form("expr-sticker-upload",a.expression.busy)}
        <div class="mt-6 grid grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-3">{admin.stickers.iter().map(|s|view!{<div class="sticker-admin flex flex-col gap-2 rounded-lg border p-3" data-sticker-row=s.name.clone()><div class="mx-auto size-24">{sticker_thumb(s)}</div><strong class="wrap-anywhere">{s.name.clone()}</strong><small class="muted-copy">{format!("{} · {}",s.emoji,creator(admin,s.creator_id))}</small><button class="button secondary" data-live=format!("dialog:sticker-edit:{}",s.id.0) aria-label=format!("Edit sticker {}",s.name)>"Edit"</button></div>}).collect_view()}</div>
    }.into_any()
}

fn emoji_edit(a: &App, page: &str) -> AnyView {
    let e = id(page.trim_start_matches("emoji-edit:")).and_then(|id| {
        a.expression
            .admin
            .as_ref()
            .and_then(|(_, adm)| adm.emoji.iter().find(|e| e.emoji.id == id))
    });
    let Some(e) = e else {
        return view! {<p class="muted-copy" role="status">"Loading…"</p>}.into_any();
    };
    view!{<div class="mb-4 flex items-center gap-3">{emoji_image(e.emoji.id,&e.emoji.name,e.emoji.animated,"size-12")}<strong>{format!(":{}:",e.emoji.name)}</strong></div>
        {form_with("Rename Emoji","expr-emoji-edit",view!{{field("expr-edit-name","Name",&e.emoji.name,"text")}<input id="expr-edit-id" type="hidden" value=e.emoji.id.0.to_string()/><p class="muted-copy">"Renaming keeps it in every message and reaction it is already in."</p>}.into_any(),"dialog:server-emoji",true)}
        <div class="settings-divider"></div>{button("Delete Emoji",&format!("expr-emoji-delete:{}",e.emoji.id.0),"danger-text")}<p class="muted-copy">"Messages that used it will show :name: instead."</p>
    }.into_any()
}

fn sticker_edit(a: &App, page: &str) -> AnyView {
    let s = id(page.trim_start_matches("sticker-edit:")).and_then(|id| {
        a.expression
            .admin
            .as_ref()
            .and_then(|(_, adm)| adm.stickers.iter().find(|s| s.id == id))
    });
    let Some(s) = s else {
        return view! {<p class="muted-copy" role="status">"Loading…"</p>}.into_any();
    };
    view!{<div class="mb-4 size-40">{sticker_thumb(s)}</div>
        {form_with("Edit Sticker","expr-sticker-edit",view!{{field("expr-edit-name","Name",&s.name,"text")}{field("expr-edit-emoji","Related Emoji",&s.emoji,"text")}{field("expr-edit-description","Description (for screen readers)",&s.description,"text")}<input id="expr-edit-id" type="hidden" value=s.id.0.to_string()/>}.into_any(),"dialog:server-stickers",true)}
        <div class="settings-divider"></div>{button("Delete Sticker",&format!("expr-sticker-delete:{}",s.id.0),"danger-text")}<p class="muted-copy">"Messages that carried it will say it was deleted."</p>
    }.into_any()
}

/// User settings › Expression: typing indicator and Super Reaction motion.
fn expression_settings(a: &App) -> AnyView {
    let style = a.expression.settings.typing.clone();
    let current = style.clone().unwrap_or_default();
    let emoji_text = |i: usize| {
        current
            .emoji
            .get(i)
            .map_or(String::new(), |e| match custom_emoji_id(e) {
                Some(id) => ex::usable(a)
                    .into_iter()
                    .find(|(_, v)| v.id == id)
                    .map_or(e.clone(), |(_, v)| format!(":{}:", v.name)),
                None => e.clone(),
            })
    };
    let (verb, custom) = match &current.verb {
        TypingVerb::Typing => ("typing", String::new()),
        TypingVerb::Writing => ("writing", String::new()),
        TypingVerb::Custom(v) => ("custom", v.clone()),
    };
    let me = a.model.me.as_ref().map_or("You".into(), |m| name(&m.user));
    view!{<h2>"Typing Indicator"</h2><p class="muted-copy">"When you’re the only one typing, others see your emoji and word instead of the standard indicator."</p>
        <div class="typing-preview my-4 rounded-lg border bg-card p-4 text-sm" aria-label="Preview">{match &style{Some(s)=>typing_view(a,&me,s),None=>view!{<span>{format!("{me} is typing…")}</span>}.into_any()}}</div>
        <form data-live-form="expr-typing">
            <fieldset class="flex flex-wrap gap-3"><legend class="mb-2 text-[13px] font-medium text-subtle">"Up to three emoji"</legend>{(0..MAX_TYPING_EMOJI).map(|i|view!{<label class="field w-24"><span class="sr-only">{format!("Emoji {}",i+1)}</span><input id=format!("expr-typing-emoji-{i}") type="text" maxlength="40" value=emoji_text(i) placeholder=["✍️","💬",":name:"][i] autocomplete="off" aria-label=format!("Emoji {}",i+1)/></label>}).collect_view()}</fieldset>
            <small class="muted-copy">"Type or paste an emoji, a shortcode like :pencil:, or a custom emoji’s :name:."</small>
            <label class="field mt-4"><span>"Motion"</span><select id="expr-typing-motion">{TypingMotion::ALL.iter().map(|m|view!{<option value=m.name() selected=*m==current.motion>{m.label()}</option>}).collect_view()}</select></label>
            <label class="field"><span>"Word"</span><select id="expr-typing-verb"><option value="typing" selected=verb=="typing">"is typing"</option><option value="writing" selected=verb=="writing">"is writing"</option><option value="custom" selected=verb=="custom">"My own word"</option></select></label>
            <div class="field"><label for="expr-typing-custom">"Your own word (after “is”)"</label><input id="expr-typing-custom" type="text" maxlength="20" value=custom placeholder="cooking up something" autocomplete="off" aria-describedby="expr-typing-custom-help"/><small id="expr-typing-custom-help">"Up to 20 letters. Spaces, hyphens and apostrophes are fine between letters."</small></div>
            <button type="submit" class="button primary">"Save Typing Indicator"</button>{if style.is_some(){view!{<button type="button" class="button secondary mt-2 ml-2 phone:ml-0 phone:w-full" data-live="expr-typing-reset">"Use the Standard Indicator"</button>}.into_any()}else{().into_any()}}
        </form>
        <div class="settings-divider"></div><h2>"Super Reactions"</h2>
        <p class="muted-copy">"Super Reaction animations, with motion and the other animation settings, are in Accessibility."</p>{button("Open Accessibility","dialog:accessibility","link-button")}
    }.into_any()
}

/// Server settings pages of ours, for `settings::is_server_page`.
pub(super) fn is_server_page(page: &str) -> bool {
    ex::admin_page(page)
}
pub(super) fn is_user_page(page: &str) -> bool {
    page == "expression-settings"
}
/// The settings navigation row for the Expression page.
pub(super) fn settings_nav(a: &App) -> AnyView {
    settings_nav_row(
        "sparkles",
        "Expression",
        "dialog:expression-settings",
        a.dialog == "expression-settings",
    )
}

/// The dialogs and settings pages of ours, fully wrapped.
pub(super) fn dialog(a: &App) -> Option<AnyView> {
    let d = a.dialog.as_str();
    let react = d.strip_prefix("emoji-react:").map(str::to_owned);
    Some(match d {
        "emoji" => modal(a, "picker-modal", "Emoji", emoji_picker(a, None)),
        "stickers" => modal(a, "picker-modal", "Stickers", sticker_picker(a)),
        _ if react.is_some() => modal(a, "picker-modal", "Add Reaction", emoji_picker(a, react)),
        _ if d.starts_with("emoji-info:") => {
            let e = id(d.trim_start_matches("emoji-info:"))?;
            modal(a, "", "Emoji", emoji_info(a, e))
        }
        "server-emoji" => settings::server_layout(a, server_emoji(a)),
        "server-stickers" => settings::server_layout(a, server_stickers(a)),
        _ if d.starts_with("emoji-edit:") => settings::server_layout(a, emoji_edit(a, d)),
        _ if d.starts_with("sticker-edit:") => settings::server_layout(a, sticker_edit(a, d)),
        "expression-settings" => settings_layout(a, expression_settings(a)),
        _ => return None,
    })
}
