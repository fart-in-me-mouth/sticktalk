//! Views for navigation: the server rail (Favorites, folders), pinned and
//! favorite conversations in the sidebar, the header search box and results,
//! the inbox, the Quick Switcher and their dialogs. Behavior that needs no
//! render per keystroke (switcher filtering, filter suggestions, dragging)
//! lives in `public/nav.js`; these views only describe the state.
use super::*;
use crate::nav_logic::{self, Slot};

const ROW: &str = "channel-row group flex min-h-[34px] items-center rounded-md";

/// Unread and mention totals for a server's text channels.
fn guild_counts(a: &App, g: Id) -> (bool, u32) {
    let mut unread = false;
    let mut mentions = 0;
    for c in a.model.channels.values().filter(|c| c.guild_id == Some(g)) {
        if let Some(r) = a.model.reads.get(&c.id) {
            unread |= r.unread_count > 0;
            mentions += r.mention_count;
        }
    }
    (unread, mentions)
}
fn badge(count: u32, class: &'static str) -> AnyView {
    if count == 0 {
        return ().into_any();
    }
    view!{<span class=class aria-label=format!("{count} mentions")>{if count>99{"99+".to_string()}else{count.to_string()}}</span>}.into_any()
}
fn guild_face(g: &GuildSummary, class: &'static str) -> AnyView {
    if let Some(id) = g.guild.icon_id {
        view!{<img class=class src=format!("/api/v1/files/{}/thumbnail",id.0) alt="" loading="lazy"/>}.into_any()
    } else {
        view! {<span class="truncate">{g.guild.name.chars().take(2).collect::<String>()}</span>}
            .into_any()
    }
}
fn server_button(a: &App, g: &GuildSummary, folder: Option<u32>) -> AnyView {
    let id = g.guild.id;
    let selected = a.guild == Some(id) && !a.nav.favorites;
    let (unread, mentions) = guild_counts(a, id);
    let unread = unread && !selected;
    view!{<ui-tooltip class="contents"><ui-tooltip-trigger><button class=format!("server-icon relative grid size-12 min-h-12 flex-none place-items-center font-semibold text-foreground transition-[border-radius,background-color] hover:rounded-2xl hover:bg-secondary {}",if selected{"selected rounded-2xl bg-secondary"}else{"rounded-full bg-muted"}) data-live=format!("guild:{}",id.0) data-rail=format!("g{}",id.0) data-rail-folder=folder.map(|f|f.to_string()) draggable="true" aria-label=g.guild.name.clone() aria-haspopup="dialog" aria-description="Right-click or press Shift+F10 for folder options">
        {if selected{view!{<span class="absolute -left-3 h-10 w-1 rounded-r-sm bg-foreground" aria-hidden="true"></span>}.into_any()}else if unread{view!{<span class="server-unread absolute -left-3 h-2 w-1 rounded-r-sm bg-foreground" aria-label="Unread messages"></span>}.into_any()}else{().into_any()}}
        {guild_face(g,"size-full rounded-[inherit] object-cover")}
        {badge(mentions,"live-unread mention-badge absolute -right-1 -bottom-1 ml-0 border-2 border-rail")}
    </button></ui-tooltip-trigger><ui-tooltip-content class="tooltip" side="right" side-offset="8" hidden=true>{g.guild.name.clone()}</ui-tooltip-content></ui-tooltip>}.into_any()
}
fn folder_slot(a: &App, f: &ServerFolder) -> AnyView {
    let guilds: Vec<&GuildSummary> = f
        .guild_ids
        .iter()
        .filter_map(|g| a.model.guilds.get(g))
        .collect();
    let mut unread = false;
    let mut mentions = 0;
    for g in &guilds {
        let (u, m) = guild_counts(a, g.guild.id);
        unread |= u;
        mentions += m;
    }
    let active = !a.nav.favorites && a.guild.is_some_and(|g| f.guild_ids.contains(&g));
    let color = format!("#{:06x}", f.color);
    let label = format!(
        "{} folder, {} servers{}",
        f.name,
        guilds.len(),
        if f.collapsed { "" } else { ", expanded" }
    );
    let tile = if f.collapsed {
        view!{<span class="grid size-12 grid-cols-2 gap-0.5 rounded-2xl p-1.5" style=format!("background:{color}33")>{guilds.iter().take(4).map(|g|view!{<span class="grid size-[17px] place-items-center overflow-hidden rounded-full bg-muted text-[8px] font-semibold">{guild_face(g,"size-full object-cover")}</span>}).collect_view()}</span>}.into_any()
    } else {
        view!{<span class="grid size-12 place-items-center rounded-2xl" style=format!("background:{color}33;color:{color}")>{icon_with("folder","icon size-6")}</span>}.into_any()
    };
    view!{<div class=format!("rail-folder flex flex-col items-center gap-2 rounded-2xl {}",if f.collapsed{""}else{"pb-2"}) style=if f.collapsed{String::new()}else{format!("background:{color}1f")} data-folder=f.id.to_string()>
        <ui-tooltip class="contents"><ui-tooltip-trigger><button class="server-icon folder-icon relative grid size-12 min-h-12 place-items-center rounded-2xl" data-live=format!("nav-folder-toggle:{}",f.id) data-rail=format!("f{}",f.id) draggable="true" aria-label=label aria-expanded=(!f.collapsed).to_string()>
            {if active&&f.collapsed{view!{<span class="absolute -left-3 h-10 w-1 rounded-r-sm bg-foreground" aria-hidden="true"></span>}.into_any()}else if unread&&f.collapsed{view!{<span class="server-unread absolute -left-3 h-2 w-1 rounded-r-sm bg-foreground" aria-label="Unread messages"></span>}.into_any()}else{().into_any()}}
            {tile}
            {if f.collapsed{badge(mentions,"live-unread mention-badge absolute -right-1 -bottom-1 ml-0 border-2 border-rail")}else{().into_any()}}
        </button></ui-tooltip-trigger><ui-tooltip-content class="tooltip" side="right" side-offset="8" hidden=true>{f.name.clone()}</ui-tooltip-content></ui-tooltip>
        {if f.collapsed{().into_any()}else{guilds.iter().map(|g|server_button(a,g,Some(f.id))).collect_view().into_any()}}
    </div>}.into_any()
}
/// The server rail below Home: Favorites, then servers and folders in the
/// account's order.
pub(super) fn rail(a: &App) -> AnyView {
    let favorites = &a.nav.settings.favorites;
    let fav_mentions: u32 = favorites
        .iter()
        .filter_map(|f| a.model.reads.get(&f.channel_id))
        .map(|r| r.mention_count)
        .sum();
    view!{
        <ui-tooltip class="contents"><ui-tooltip-trigger><button class=format!("server-icon favorites-icon relative grid size-12 min-h-12 flex-none place-items-center transition-[border-radius,background-color] hover:rounded-2xl hover:bg-secondary {}",if a.nav.favorites{"selected rounded-2xl bg-secondary text-warning"}else{"rounded-full bg-muted text-subtle"}) data-live="nav-favorites" aria-label="Favorites" aria-pressed=a.nav.favorites.to_string()>{if a.nav.favorites{view!{<span class="absolute -left-3 h-10 w-1 rounded-r-sm bg-foreground" aria-hidden="true"></span>}.into_any()}else{().into_any()}}{icon("star")}{badge(fav_mentions,"live-unread mention-badge absolute -right-1 -bottom-1 ml-0 border-2 border-rail")}</button></ui-tooltip-trigger><ui-tooltip-content class="tooltip" side="right" side-offset="8" hidden=true>"Favorites"</ui-tooltip-content></ui-tooltip>
        <div class="rail-list flex flex-col items-center gap-2" data-rail-list>{super::super::nav::layout(a).iter().map(|slot|match slot{
            Slot::Guild(g)=>a.model.guilds.get(g).map(|g|server_button(a,g,None)).unwrap_or_else(||().into_any()),
            Slot::Folder(f)=>folder_slot(a,f),
        }).collect_view()}</div>
    }.into_any()
}

fn conv_icon(c: &ChannelSummary) -> &'static str {
    match c.kind {
        ChannelKind::Voice => "volume-2",
        ChannelKind::Stage => "audio-lines",
        ChannelKind::GroupDm => "users",
        ChannelKind::Direct => "message-circle",
        kind => super::threads::kind_icon(kind).unwrap_or("hash"),
    }
}
/// Where a message is, for result cards and the inbox: "#general", a voice
/// channel's chat, "#general › Launch party" for a thread or forum post, or
/// the people of a DM.
fn place(a: &App, c: &ChannelSummary) -> String {
    let name = a.model.channel_name(c);
    if c.guild_id.is_none() {
        return name;
    }
    match c
        .parent_id
        .filter(|_| c.kind == ChannelKind::Thread)
        .and_then(|p| a.model.channels.get(&p))
    {
        Some(p) => format!("#{} › {name}", a.model.channel_name(p)),
        None => format!("#{name}"),
    }
}
/// A conversation row outside the channel tree (Pinned, Favorites).
fn conv_row(a: &App, c: &ChannelSummary, action: String, sub: Option<String>) -> AnyView {
    let label = a.model.channel_name(c);
    let selected = a.channel == Some(c.id);
    let read = a.model.reads.get(&c.id);
    let unread = read.is_some_and(|r| r.unread_count > 0);
    let count = read.map_or(0, |r| {
        if c.guild_id.is_none() {
            r.unread_count
        } else {
            r.mention_count
        }
    });
    view!{<div class=format!("{ROW} nav-conv-row {}",if selected{"selected bg-secondary"}else{"hover:bg-sidebar-accent"}) data-conv=c.id.0.to_string()><button class=format!("channel-main flex min-w-0 flex-1 items-center gap-2 p-2 text-sm font-medium touch:min-h-11 phone:text-base {}",if selected||unread{"text-foreground"}else{"text-sidebar-foreground"}) title=label.clone() data-live=action>{icon_with(conv_icon(c),"icon text-muted-foreground")}<span class="flex min-w-0 flex-col items-start"><span class="max-w-full truncate">{label.clone()}</span>{sub.map(|s|view!{<small class="max-w-full truncate text-xs text-muted-foreground">{s}</small>})}</span>{if count>0{view!{<span class="live-unread">{count.to_string()}</span>}.into_any()}else if unread{view!{<span class="live-unread quiet" aria-label="Unread messages"></span>}.into_any()}else{().into_any()}}</button>{row_options(c)}</div>}.into_any()
}
/// The "more" button on a conversation row: pin, favorite, mark as read.
pub(super) fn row_options(c: &ChannelSummary) -> AnyView {
    if c.kind == ChannelKind::Category {
        return ().into_any();
    }
    view!{<button class="row-options mr-1 hidden size-6 flex-none place-items-center rounded-sm text-muted-foreground group-focus-within:grid group-hover:grid hover:text-foreground touch:grid touch:size-11" aria-label="Conversation options" title="Conversation options" data-live=format!("nav-menu:conv:{}",c.id.0)>{icon_with("more-horizontal","icon size-4")}</button>}.into_any()
}
/// The conversations a Favorites category holds, in order.
fn favorites_list(a: &App) -> AnyView {
    let s = &a.nav.settings;
    if s.favorites.is_empty() {
        return view!{<div class="nav-empty px-2 py-6 text-sm text-subtle"><p class="mb-3">"Add channels and conversations you visit most. Open a conversation’s options and choose Add to Favorites."</p>{button("Manage Favorites","nav-menu:favorites-manage","secondary")}</div>}.into_any();
    }
    let sub = |c: &ChannelSummary| {
        c.guild_id
            .and_then(|g| a.model.guilds.get(&g))
            .map(|g| g.guild.name.clone())
    };
    let rows = |category: Option<u32>| {
        s.favorites
            .iter()
            .filter(|f| f.category_id == category)
            .filter_map(|f| a.model.channels.get(&f.channel_id))
            .map(|c| conv_row(a, c, format!("nav-go:channel:{}", c.id.0), sub(c)))
            .collect_view()
    };
    view!{
        {rows(None)}
        {s.favorite_categories.iter().map(|k|view!{<div class="category-row mt-4 flex min-h-7 items-center gap-1 pr-1 pl-2"><button class="category flex min-w-0 flex-1 items-center gap-1 text-left" data-live=format!("nav-fav-cat-toggle:{}",k.id) aria-expanded=(!k.collapsed).to_string()>{icon_with(if k.collapsed{"chevron-right"}else{"chevron-down"},"icon size-3")}<span class="truncate">{k.name.clone()}</span></button></div>{if k.collapsed{().into_any()}else{rows(Some(k.id)).into_any()}}}).collect_view()}
        <div class="mt-4 px-2">{button("Manage Favorites","nav-menu:favorites-manage","link-button")}</div>
    }.into_any()
}
/// Above the channel list: the switcher entry (the phone's way in), then
/// Favorites (when chosen on the rail) or this list's pinned conversations.
pub(super) fn sidebar_top(a: &App) -> AnyView {
    let finder = view! {<button class="nav-find mb-2 flex h-8 w-full items-center gap-2 rounded-md bg-rail px-2 text-left text-[13px] text-muted-foreground hover:text-foreground touch:min-h-11" data-live="nav-switcher" aria-keyshortcuts="Control+K Meta+K">{icon_with("search","icon size-4")}<span class="min-w-0 flex-1 truncate">"Find or start a conversation"</span><kbd class="text-[11px] phone:hidden">"Ctrl K"</kbd></button>};
    if a.nav.favorites {
        return view! {{finder}{favorites_list(a)}}.into_any();
    }
    let pinned: Vec<&ChannelSummary> = a
        .nav
        .settings
        .pinned
        .iter()
        .filter_map(|c| a.model.channels.get(c))
        .filter(|c| c.guild_id == a.guild)
        .collect();
    if pinned.is_empty() {
        return finder.into_any();
    }
    view!{{finder}<h2 class="category nav-pinned mx-2 mt-2 mb-1 flex items-center gap-1">{icon_with("pin","icon size-3")}"Pinned"</h2>{pinned.into_iter().map(|c|conv_row(a,c,format!("{}:{}",if c.kind==ChannelKind::Voice{"voice"}else{"channel"},c.id.0),None)).collect_view()}<div class="rail-divider mx-2 my-2 h-px bg-border"></div>}.into_any()
}
/// Direct messages without the pinned ones, which are listed above them.
pub(super) fn unpinned(a: &App, c: &ChannelSummary) -> bool {
    c.guild_id.is_some() || !a.nav.is_pinned(c.id)
}

/// Words people and channels can be suggested from as filters are typed.
fn suggestions(a: &App) -> String {
    let mut users: BTreeMap<String, String> = BTreeMap::new();
    let me = a.model.me.as_ref().map(|m| m.user.id);
    let mut add = |u: &UserSummary| {
        if users.len() < 300 {
            users.insert(u.username.clone(), name(u));
        }
    };
    for m in a
        .model
        .members
        .values()
        .filter(|m| Some(m.guild_id) == a.guild)
    {
        add(&m.user);
    }
    for r in a
        .model
        .relationships
        .values()
        .filter(|r| r.kind == RelationshipKind::Friend)
    {
        add(&r.user);
    }
    for c in a.model.channels.values().filter(|c| c.guild_id.is_none()) {
        for u in c
            .recipient_ids
            .iter()
            .filter(|u| Some(**u) != me)
            .filter_map(|u| a.model.users.get(u))
        {
            add(u);
        }
    }
    let channels: Vec<String> = a
        .model
        .channels
        .values()
        .filter(|c| a.guild.is_some() && c.guild_id == a.guild && c.kind == ChannelKind::Text)
        .filter_map(|c| c.name.clone())
        .take(64)
        .collect();
    serde_json::json!({"users":users.into_iter().collect::<Vec<_>>(),"channels":channels})
        .to_string()
}
fn scope_name(a: &App) -> String {
    match a.guild.and_then(|g| a.model.guilds.get(&g)) {
        Some(g) => g.guild.name.clone(),
        None => match a.channel.and_then(|c| a.model.channels.get(&c)) {
            Some(c) => a.model.channel_name(c),
            None => "Direct Messages".into(),
        },
    }
}
fn search_input(a: &App, id: &'static str, class: &'static str) -> AnyView {
    let scope = scope_name(a);
    view!{<form class=format!("nav-search relative {class}") role="search" data-nav-search-form><label class="flex h-7 items-center gap-1 rounded-md bg-rail px-2 text-[13px]"><span class="sr-only">{format!("Search {scope}")}</span><input id=id class="h-6 min-w-0 flex-1 border-0 bg-transparent p-0 text-[13px] shadow-none focus-visible:ring-0" type="search" autocomplete="off" spellcheck="false" maxlength="512" placeholder=format!("Search {scope}") value=a.nav.search_query.clone() data-nav-search data-suggest=suggestions(a) role="combobox" aria-autocomplete="list" aria-controls="nav-suggest" aria-expanded="false"/>{icon_with("search","icon size-4 text-muted-foreground")}</label><div id="nav-suggest" class="nav-suggest absolute top-full right-0 z-50 mt-1 hidden max-h-[min(360px,60vh)] w-[min(320px,calc(100vw-16px))] overflow-y-auto rounded-lg border bg-popover p-2 shadow-lg" role="listbox" aria-label="Search filters"></div></form>}.into_any()
}
/// The chat header's search box and inbox button.
pub(super) fn header(a: &App) -> AnyView {
    let mentions: u32 = a.model.reads.values().map(|r| r.mention_count).sum();
    let due = a.nav.settings.saved.due;
    view!{<div class="nav-header flex items-center gap-2 phone:gap-0">
        {search_input(a,"nav-search-input","w-[clamp(140px,18vw,240px)] phone:hidden")}
        <div class="hidden phone:block">{ib("search","Search","nav-search-open")}</div>
        <span class="relative inline-flex">{ib("inbox","Inbox","nav-inbox")}{if mentions+due>0{view!{<span class="live-unread mention-badge pointer-events-none absolute top-0.5 right-0 ml-0" aria-label=format!("{} new in inbox",mentions+due)>{(mentions+due).min(99).to_string()}</span>}.into_any()}else{().into_any()}}</span>
    </div>}.into_any()
}
fn time_label(ms: u64) -> String {
    timestamp(ms)
}
/// One message in a list away from its conversation: who, where, when, text.
fn card(a: &App, m: &Message, terms: &[String], extra: AnyView) -> AnyView {
    let author = a.model.users.get(&m.author_id).cloned().unwrap_or_default();
    let place = a
        .model
        .channels
        .get(&m.channel_id)
        .map_or("Conversation".into(), |c| place(a, c));
    let text = nav_logic::excerpt(&a.model.content(m), 300);
    let parts = nav_logic::highlight(&text, terms);
    view!{<article class="nav-card relative rounded-lg border bg-card p-3 hover:bg-accent" data-nav-message=m.id.0.to_string()>
        <button class="nav-card-main flex w-full gap-3 text-left" data-live=format!("nav-jump:{},{}",m.channel_id.0,m.id.0) aria-label=format!("Jump to message from {} in {}",name(&author),place)>
            <span class="flex-none">{avatar(&author,"small")}</span>
            <span class="min-w-0 flex-1"><span class="flex flex-wrap items-baseline gap-x-2 text-[13px]"><strong class="font-semibold" style=format!("color:{}",color(a,m.author_id))>{name(&author)}</strong><span class="text-muted-foreground">{place.clone()}</span><time class="text-xs text-muted-foreground">{time_label(m.created_at)}</time></span>
            <span class="mt-1 block text-sm wrap-anywhere whitespace-pre-wrap">{parts.into_iter().map(|(t,hit)|if hit{view!{<mark class="nav-hit rounded-sm bg-warning/30 px-0.5 text-foreground">{t}</mark>}.into_any()}else{view!{<span>{t}</span>}.into_any()}).collect_view()}</span>
            {if m.attachments.is_empty(){().into_any()}else{view!{<span class="mt-1 flex flex-wrap gap-2 text-xs text-subtle">{m.attachments.iter().map(|f|view!{<span class="inline-flex items-center gap-1">{icon_with(if f.content_type.starts_with("image/"){"image"}else if f.content_type.starts_with("video/"){"video"}else{"paperclip"},"icon size-3.5")}{f.filename.clone()}</span>}).collect_view()}</span>}.into_any()}}</span>
        </button>
        {extra}
    </article>}.into_any()
}
/// Search results, beside the conversation (over it on a phone).
pub(super) fn search_panel(a: &App) -> AnyView {
    if !a.nav.search_open {
        return ().into_any();
    }
    let terms = match &a.nav.search {
        Some(Ok(p)) => p.terms.clone(),
        _ => vec![],
    };
    let page = a.nav.search_pages.len() + 1;
    let body = match &a.nav.search {
        _ if a.nav.search_query.is_empty() => view!{<div class="nav-empty p-4 text-sm text-subtle"><p>"Search messages by words, or narrow it down:"</p><ul class="mt-2 grid gap-1 [&_code]:rounded-sm [&_code]:bg-rail [&_code]:px-1">
            <li><code>"from:"</code>" a person"</li><li><code>"mentions:"</code>" a person"</li><li><code>"in:"</code>" a channel"</li><li><code>"has:"</code>" link, file, image or video"</li><li><code>"before:"</code>", "<code>"after:"</code>", "<code>"during:"</code>" a date (YYYY-MM-DD)"</li></ul></div>}.into_any(),
        None => view!{<p class="p-4 text-sm text-subtle" role="status">"Searching…"</p>}.into_any(),
        Some(Err(e)) => view!{<p class="p-4 text-sm text-warning" role="alert">{e.clone()}</p>}.into_any(),
        Some(Ok(p)) if p.items.is_empty() => view!{<div class="nav-empty p-4 text-sm text-subtle" role="status"><p>"No results. Check the spelling, or try fewer filters."</p></div>}.into_any(),
        Some(Ok(p)) => view!{<p class="sr-only" role="status">{format!("{} results on page {page}",p.items.len())}</p><div class="grid gap-2 p-3">{p.items.iter().map(|m|card(a,m,&terms,().into_any())).collect_view()}</div>
            <div class="nav-pager flex items-center justify-between gap-2 px-3 pb-4">{if a.nav.search_pages.is_empty(){view!{<span></span>}.into_any()}else{button("Previous","nav-search-page:prev","secondary")}}<span class="text-xs text-subtle">{format!("Page {page}")}</span>{if p.has_more{button("Next","nav-search-page:next","secondary")}else{view!{<span></span>}.into_any()}}</div>}.into_any(),
    };
    view!{<aside class="nav-results flex w-[min(420px,40vw)] flex-none touch-pan-y flex-col border-l bg-sidebar narrow:absolute narrow:top-12 narrow:right-0 narrow:bottom-0 narrow:z-20 narrow:w-[min(420px,100vw)] narrow:shadow-[-8px_0_24px_#0006] phone:top-[calc(var(--header-height)+var(--safe-top))] phone:z-30 phone:w-full phone:border-l-0 phone:pb-(--safe-bottom)" aria-label="Search results">
        <header class="flex min-h-12 items-center gap-2 border-b px-3 py-2"><strong class="min-w-0 flex-1 truncate text-sm font-semibold">{match &a.nav.search{Some(Ok(p)) if !a.nav.search_query.is_empty()=>format!("{}{} results",p.items.len(),if p.has_more{"+"}else{""}),_=>"Search".into()}}</strong>{ib_plain("x","Close Search","nav-search-close")}</header>
        <div class="hidden border-b p-2 phone:block">{search_input(a,"nav-search-mobile","w-full")}</div>
        <div class="min-h-0 flex-1 overflow-y-auto overscroll-contain">{body}</div>
    </aside>}.into_any()
}
/// Save and Remind on a message (also copied into the phone's action sheet).
pub(super) fn message_actions(m: &Message) -> AnyView {
    view!{{ib_plain("bookmark","Save Message",&format!("nav-save:{}",m.id.0))}{ib_plain("clock","Remind Me",&format!("nav-remind:{}",m.id.0))}}.into_any()
}
/// A reminder that has come due, until it is opened or dismissed.
pub(super) fn overlay(a: &App) -> AnyView {
    let Some(r) = &a.nav.reminder else {
        return ().into_any();
    };
    let what = r
        .message
        .as_ref()
        .map_or("This message is no longer available.".into(), |m| {
            format!(
                "{}: {}",
                a.model.name(m.author_id),
                nav_logic::excerpt(&a.model.content(m), 120)
            )
        });
    view!{<div class="nav-reminder fixed right-4 bottom-[calc(16px+var(--safe-bottom))] z-[150] flex w-[min(360px,calc(100vw-32px))] flex-col gap-2 rounded-lg border border-primary bg-popover p-4 text-sm shadow-lg" role="alert"><strong class="flex items-center gap-2">{icon_with("clock","icon size-4 text-primary")}"Reminder"</strong><p class="wrap-anywhere text-subtle">{what}</p><div class="flex gap-2">{if r.message.is_some(){button("Jump to Message",&format!("nav-jump:{},{}",r.channel_id.0,r.message_id.0),"primary")}else{().into_any()}}{button("Dismiss","nav-reminder-dismiss","secondary")}</div></div>}.into_any()
}

pub(super) fn dialog_label(dialog: &str) -> &'static str {
    match dialog.split(':').next().unwrap_or_default() {
        "nav-switcher" => "Quick Switcher",
        "nav-inbox" => "Inbox",
        "nav-remind" => "Remind Me",
        "nav-server" => "Server Options",
        "nav-folder" => "Folder Options",
        "nav-conv" => "Conversation Options",
        "nav-favorites-manage" => "Manage Favorites",
        _ => "Dialog",
    }
}
pub(super) fn dialog(a: &App, name: &str) -> AnyView {
    let (kind, arg) = name.split_once(':').unwrap_or((name, ""));
    match kind {
        "nav-switcher" => switcher(a),
        "nav-inbox" => inbox(a),
        "nav-remind" => remind(arg),
        "nav-server" => server_menu(a, arg),
        "nav-folder" => folder_menu(a, arg),
        "nav-conv" => conv_menu(a, arg),
        "nav-favorites-manage" => favorites_manage(a),
        _ => ().into_any(),
    }
}

/// Everything the switcher can find. The page filters these as you type.
fn switcher(a: &App) -> AnyView {
    struct Row {
        rank: u8,
        kind: &'static str,
        glyph: &'static str,
        label: String,
        sub: String,
        action: String,
        recent: u64,
        count: u32,
    }
    let mut rows = vec![];
    let me = a.model.me.as_ref().map(|m| m.user.id);
    let mut dm_people = std::collections::BTreeSet::new();
    for c in a.model.channels.values() {
        if c.kind == ChannelKind::Category {
            continue;
        }
        let read = a.model.reads.get(&c.id);
        let unread = read.is_some_and(|r| r.unread_count > 0);
        let count = read.map_or(0, |r| {
            if c.guild_id.is_none() {
                r.unread_count
            } else {
                r.mention_count
            }
        });
        let (sub, kind) = match c.guild_id.and_then(|g| a.model.guilds.get(&g)) {
            Some(g) => (g.guild.name.clone(), "channel"),
            None => {
                if c.kind == ChannelKind::Direct {
                    dm_people.extend(c.recipient_ids.iter().copied().filter(|u| Some(*u) != me));
                }
                (
                    if c.kind == ChannelKind::GroupDm {
                        "Group".into()
                    } else {
                        "Direct Message".into()
                    },
                    "dm",
                )
            }
        };
        rows.push(Row {
            rank: if count > 0
                && (c.guild_id.is_none() || read.is_some_and(|r| r.mention_count > 0))
            {
                2
            } else if unread {
                1
            } else {
                0
            },
            kind,
            glyph: conv_icon(c),
            label: a.model.channel_name(c),
            sub: match c.kind {
                ChannelKind::Voice => format!("{sub} · Voice"),
                ChannelKind::Thread => c
                    .parent_id
                    .and_then(|p| a.model.channels.get(&p))
                    .map_or(sub.clone(), |p| {
                        format!("{sub} · #{}", a.model.channel_name(p))
                    }),
                _ => sub,
            },
            action: format!("channel:{}", c.id.0),
            recent: read.and_then(|r| r.latest_message_id).map_or(0, |m| m.0),
            count,
        });
    }
    for g in a.model.guilds.values() {
        let (unread, mentions) = guild_counts(a, g.guild.id);
        rows.push(Row {
            rank: if mentions > 0 {
                2
            } else if unread {
                1
            } else {
                0
            },
            kind: "server",
            glyph: "layout-grid",
            label: g.guild.name.clone(),
            sub: "Server".into(),
            action: format!("guild:{}", g.guild.id.0),
            recent: 0,
            count: mentions,
        });
    }
    for r in a
        .model
        .relationships
        .values()
        .filter(|r| r.kind == RelationshipKind::Friend && !dm_people.contains(&r.user.id))
    {
        rows.push(Row {
            rank: 0,
            kind: "person",
            glyph: "user-round",
            label: name(&r.user),
            sub: format!("@{}", r.user.username),
            action: format!("friend:dm,{}", r.user.id.0),
            recent: 0,
            count: 0,
        });
    }
    rows.sort_by(|x, y| {
        y.rank
            .cmp(&x.rank)
            .then(y.recent.cmp(&x.recent))
            .then(x.label.to_lowercase().cmp(&y.label.to_lowercase()))
    });
    rows.truncate(1000);
    view!{<div class="nav-switcher" data-nav-switcher><h2 class="sr-only">"Quick Switcher"</h2><label class="field"><span>"Where would you like to go?"</span><input id="nav-switcher-input" type="search" autocomplete="off" spellcheck="false" placeholder="Search people, conversations, channels and servers" role="combobox" aria-autocomplete="list" aria-controls="nav-switcher-list" aria-expanded="true" data-nav-switcher-input/></label>
        <p class="mt-1 text-xs text-subtle">"Start with # for channels, @ for people, * for servers. Arrow keys choose, Enter goes."</p>
        <div id="nav-switcher-list" class="mt-3 flex max-h-[min(420px,55vh)] flex-col gap-0.5 overflow-y-auto" role="listbox" aria-label="Results">{rows.into_iter().enumerate().map(|(i,r)|view!{<button class="nav-switch-row flex min-h-10 w-full items-center gap-3 rounded-md px-2 text-left aria-selected:bg-secondary hover:bg-accent touch:min-h-12" role="option" id=format!("nav-switch-{i}") aria-selected="false" hidden={i>=40} data-live=format!("nav-go:{}",r.action) data-kind=r.kind data-rank=r.rank.to_string() data-search=format!("{} {}",r.label.to_lowercase(),r.sub.to_lowercase())>{icon_with(r.glyph,"icon size-4 flex-none text-muted-foreground")}<span class="min-w-0 flex-1 truncate text-sm font-medium">{r.label.clone()}</span><small class="max-w-[40%] truncate text-xs text-muted-foreground">{r.sub.clone()}</small>{if r.count>0{view!{<span class="live-unread">{r.count.min(99).to_string()}</span>}.into_any()}else if r.rank>0{view!{<span class="live-unread quiet" aria-label="Unread"></span>}.into_any()}else{().into_any()}}</button>}).collect_view()}</div>
        <p class="nav-switch-empty mt-3 text-sm text-subtle" role="status" hidden>"Nothing matches. Try another name."</p>
    </div>}.into_any()
}

fn inbox(a: &App) -> AnyView {
    let tab = a.nav.inbox_tab.as_str();
    let tabs = [
        ("unreads", "Unreads"),
        ("mentions", "Mentions"),
        ("saved", "Saved"),
    ];
    let body = match tab {
        "mentions" => mentions_tab(a),
        "saved" => saved_tab(a),
        _ => unreads_tab(a),
    };
    view!{<h2>"Inbox"</h2><div class="nav-inbox-tabs mb-3 flex flex-wrap gap-2" role="tablist" aria-label="Inbox">{tabs.into_iter().map(|(t,label)|view!{<button class=format!("button {}",if tab==t{"secondary"}else{"link-button"}) role="tab" aria-selected=(tab==t).to_string() data-live=format!("nav-inbox-tab:{t}")>{label}{if t=="saved"&&a.nav.settings.saved.due>0{view!{<span class="live-unread ml-2">{a.nav.settings.saved.due.to_string()}</span>}.into_any()}else{().into_any()}}</button>}).collect_view()}</div><div class="nav-inbox-body" role="tabpanel">{body}</div>}.into_any()
}
fn loading_or<T>(v: &Option<Result<T, String>>, f: impl FnOnce(&T) -> AnyView) -> AnyView {
    match v {
        None => view! {<p class="text-sm text-subtle" role="status">"Loading…"</p>}.into_any(),
        Some(Err(e)) => {
            view! {<p class="text-sm text-warning" role="alert">{e.clone()}</p>}.into_any()
        }
        Some(Ok(v)) => f(v),
    }
}
fn unreads_tab(a: &App) -> AnyView {
    loading_or(&a.nav.unreads, |u| {
        if u.channels.is_empty() {
            return view!{<div class=SETTINGS_EMPTY>{icon("check-check")}<p>"You’re all caught up."</p></div>}.into_any();
        }
        let mut groups: Vec<(Option<Id>, Vec<&UnreadChannel>)> = vec![];
        for c in &u.channels {
            let g = a
                .model
                .channels
                .get(&c.read.channel_id)
                .and_then(|c| c.guild_id);
            match groups.iter_mut().find(|(k, _)| *k == g) {
                Some((_, v)) => v.push(c),
                None => groups.push((g, vec![c])),
            }
        }
        view!{<div class="mb-3 flex justify-end">{button("Mark All as Read","nav-ack-all","secondary")}</div>
        {groups.into_iter().map(|(g,channels)|view!{<section class="nav-unread-group mb-4"><h3 class="category mb-2">{g.and_then(|g|a.model.guilds.get(&g)).map_or("Direct Messages".into(),|g|g.guild.name.clone())}</h3>
            {channels.into_iter().map(|c|{let ch=a.model.channels.get(&c.read.channel_id);view!{<div class="nav-unread mb-3 rounded-lg border p-2"><div class="mb-2 flex items-center gap-2"><strong class="min-w-0 flex-1 truncate text-sm">{ch.map_or("Conversation".into(),|ch|place(a,ch))}</strong>{if c.read.mention_count>0{view!{<span class="live-unread">{c.read.mention_count.to_string()}</span>}.into_any()}else{().into_any()}}<button class="button link-button" data-live=format!("nav-ack:{}",c.read.channel_id.0)>"Mark as Read"</button></div>
                <div class="grid gap-2">{c.messages.iter().rev().map(|m|card(a,m,&[],().into_any())).collect_view()}</div>
                {if c.read.unread_count as usize>c.messages.len(){view!{<button class="button link-button mt-1" data-live=format!("nav-go:channel:{}",c.read.channel_id.0)>{format!("{} more unread",c.read.unread_count as usize-c.messages.len())}</button>}.into_any()}else{().into_any()}}</div>}}).collect_view()}
        </section>}).collect_view()}
        {if u.has_more{view!{<p class="text-xs text-subtle">"Showing the 50 most recently active conversations."</p>}.into_any()}else{().into_any()}}}.into_any()
    })
}
fn mentions_tab(a: &App) -> AnyView {
    let filters = view! {<div class="nav-mention-filters mb-3 flex flex-wrap gap-2" role="group" aria-label="Include">
    <button class=format!("button {}",if a.nav.mentions_everyone{"secondary"}else{"link-button"}) aria-pressed=a.nav.mentions_everyone.to_string() data-live="nav-mentions-filter:everyone">"@everyone and @here"</button>
    <button class=format!("button {}",if a.nav.mentions_roles{"secondary"}else{"link-button"}) aria-pressed=a.nav.mentions_roles.to_string() data-live="nav-mentions-filter:roles">"Role mentions"</button></div>};
    view!{{filters}{loading_or(&a.nav.mentions,|p|{
        if p.items.is_empty(){return view!{<div class=SETTINGS_EMPTY>{icon("at-sign")}<p>"No mentions yet."</p></div>}.into_any();}
        view!{<div class="grid gap-2">{p.items.iter().map(|m|card(a,m,&[],().into_any())).collect_view()}</div>{if p.has_more{view!{<div class="mt-3">{button(if a.nav.busy{"Loading…"}else{"Load More"},"nav-mentions-more","secondary")}</div>}.into_any()}else{().into_any()}}}.into_any()
    })}}.into_any()
}
fn saved_tab(a: &App) -> AnyView {
    loading_or(&a.nav.saved, |p| {
        if p.items.is_empty() {
            return view!{<div class=SETTINGS_EMPTY>{icon("bookmark")}<p>"No saved messages."</p><small>"Open a message’s actions and choose Save Message or Remind Me."</small></div>}.into_any();
        }
        let mut items: Vec<&SavedMessage> = p.items.iter().collect();
        // Reminders that have come due first, then upcoming, then saved.
        items.sort_by_key(|i| {
            (
                !i.due,
                i.remind_at.is_none(),
                i.remind_at.unwrap_or(0),
                std::cmp::Reverse(i.id),
            )
        });
        view!{<p class="mb-3 text-xs text-subtle">{format!("{} of {} saved",p.summary.count,MAX_SAVED_MESSAGES)}</p><div class="grid gap-2">{items.into_iter().map(|i|{
            let tag=match (i.due,i.remind_at){(true,_)=>view!{<span class="nav-due mr-auto inline-flex items-center gap-1 text-xs font-semibold text-warning">{icon_with("clock","icon size-3.5")}"Reminder due"</span>}.into_any(),(false,Some(t))=>view!{<span class="mr-auto inline-flex items-center gap-1 text-xs text-subtle">{icon_with("clock","icon size-3.5")}{format!("Reminds {}",time_label(t))}</span>}.into_any(),_=>view!{<span class="mr-auto"></span>}.into_any()};
            let controls=view!{<div class="mt-2 flex flex-wrap items-center gap-2">{tag}{if i.message.is_some(){button("Remind Me",&format!("nav-menu:remind:{},{}",i.channel_id.0,i.message_id.0),"link-button")}else{().into_any()}}{if i.remind_at.is_some(){button("Clear Reminder",&format!("nav-reminder-clear:{}",i.message_id.0),"link-button")}else{().into_any()}}{button("Remove",&format!("nav-unsave:{}",i.message_id.0),"danger-text")}</div>}.into_any();
            match &i.message{Some(m)=>card(a,m,&[],controls),None=>view!{<article class="nav-card rounded-lg border bg-card p-3"><p class="text-sm text-subtle">"This message is no longer available."</p>{controls}</article>}.into_any()}
        }).collect_view()}</div>{if p.has_more{view!{<div class="mt-3">{button("Load More","nav-saved-more","secondary")}</div>}.into_any()}else{().into_any()}}}.into_any()
    })
}
fn remind(arg: &str) -> AnyView {
    let presets = [
        ("20m", "In 20 minutes"),
        ("1h", "In 1 hour"),
        ("3h", "In 3 hours"),
        ("tomorrow", "Tomorrow at 9 AM"),
        ("week", "Next week"),
    ];
    view!{<h2>"Remind Me"</h2><p class="muted-copy">"You’ll get a notification about this message, and it stays in Inbox, Saved."</p><div class="nav-presets my-4 grid gap-2">{presets.into_iter().map(|(p,label)|button(label,&format!("nav-remind-at:{arg},{p}"),"secondary")).collect_view()}</div>
        <form class="flex flex-wrap items-end gap-2" data-nav-form=format!("remind:{arg}")><div class="field mb-0 min-w-0 flex-1"><label for="nav-remind-custom">"Custom date and time"</label><input id="nav-remind-custom" type="datetime-local" required/></div><button class="button primary" type="submit">"Set Reminder"</button></form>}.into_any()
}
fn server_menu(a: &App, arg: &str) -> AnyView {
    let Some(g) = id(arg).and_then(|g| a.model.guilds.get(&g)) else {
        return view! {<p>"This server is no longer available."</p>}.into_any();
    };
    let gid = g.guild.id;
    let layout = super::super::nav::layout(a);
    let folder = layout.iter().find_map(|s| match s {
        Slot::Folder(f) if f.guild_ids.contains(&gid) => Some(f.clone()),
        _ => None,
    });
    let others: Vec<ServerFolder> = layout
        .iter()
        .filter_map(|s| match s {
            Slot::Folder(f) if !f.guild_ids.contains(&gid) => Some(f.clone()),
            _ => None,
        })
        .collect();
    view!{<h2>{g.guild.name.clone()}</h2><div class="nav-menu grid gap-2">
        {button("Move Up",&format!("nav-rail-step:g{},-1",gid.0),"secondary")}{button("Move Down",&format!("nav-rail-step:g{},1",gid.0),"secondary")}
        {match &folder{Some(f)=>view!{{button(&format!("Remove from {}",f.name),&format!("nav-folder-out:{}",gid.0),"secondary")}{button("Folder Settings",&format!("nav-menu:folder:{}",f.id),"secondary")}}.into_any(),None=>button("Create Folder",&format!("nav-folder-new:{}",gid.0),"secondary")}}
        {others.into_iter().map(|f|button(&format!("Add to {}",f.name),&format!("nav-folder-add:{},{}",gid.0,f.id),"secondary")).collect_view()}
        {button("Mark as Read",&format!("nav-ack-guild:{}",gid.0),"link-button")}
    </div><p class="mt-4 text-xs text-subtle">"Tip: drag a server onto another to make a folder."</p>}.into_any()
}
fn folder_menu(a: &App, arg: &str) -> AnyView {
    let Some(f) = arg.parse::<u32>().ok().and_then(|f| {
        super::super::nav::layout(a)
            .into_iter()
            .find_map(|s| match s {
                Slot::Folder(v) if v.id == f => Some(v),
                _ => None,
            })
    }) else {
        return view! {<p>"This folder is no longer available."</p>}.into_any();
    };
    view!{<form data-nav-form=format!("folder:{}",f.id)><h2>"Folder Settings"</h2><div class="field"><label for="nav-folder-name">"Folder Name"</label><input id="nav-folder-name" type="text" maxlength="32" required value=f.name.clone()/></div><div class="field"><label for="nav-folder-color">"Folder Color"</label><input id="nav-folder-color" type="color" value=format!("#{:06x}",f.color)/></div><div class="nav-swatches mb-4 flex gap-2" role="group" aria-label="Suggested colors">{nav_logic::FOLDER_COLORS.iter().map(|c|view!{<button type="button" class="size-8 rounded-full border-2 border-transparent hover:border-foreground" style=format!("background:#{c:06x}") aria-label=format!("Color #{c:06x}") data-nav-color=format!("#{c:06x}")></button>}).collect_view()}</div><button class="button primary" type="submit">"Save Changes"</button></form>
        <div class="nav-menu mt-4 grid gap-2">{button(if f.collapsed{"Expand Folder"}else{"Collapse Folder"},&format!("nav-folder-toggle:{}",f.id),"secondary")}{button("Move Up",&format!("nav-rail-step:f{},-1",f.id),"secondary")}{button("Move Down",&format!("nav-rail-step:f{},1",f.id),"secondary")}{button("Ungroup Folder",&format!("nav-folder-ungroup:{}",f.id),"danger-text")}</div>}.into_any()
}
fn conv_menu(a: &App, arg: &str) -> AnyView {
    let Some(c) = id(arg).and_then(|c| a.model.channels.get(&c)) else {
        return view! {<p>"This conversation is no longer available."</p>}.into_any();
    };
    let pinned = a.nav.is_pinned(c.id);
    let favorite = a.nav.is_favorite(c.id);
    let unread = a.model.reads.get(&c.id).is_some_and(|r| r.unread_count > 0);
    view!{<h2>{a.model.channel_name(c)}</h2><div class="nav-menu grid gap-2">
        {button(if pinned{"Unpin"}else{"Pin to Top"},&format!("nav-pin:{}",c.id.0),"secondary")}
        {button(if favorite{"Remove from Favorites"}else{"Add to Favorites"},&format!("nav-fav:{}",c.id.0),"secondary")}
        {if unread{button("Mark as Read",&format!("nav-ack:{}",c.id.0),"secondary")}else{().into_any()}}
        {if c.kind!=ChannelKind::Voice{button("Notification Settings",&match c.guild_id{Some(_)=>format!("dialog:target-notifications:channel:{}",c.id.0),None=>format!("dialog:target-notifications:channel:{}",c.id.0)},"link-button")}else{().into_any()}}
    </div>}.into_any()
}
fn favorites_manage(a: &App) -> AnyView {
    let s = &a.nav.settings;
    let options = |selected: Option<u32>| {
        view!{<option value="0" selected=selected.is_none()>"No category"</option>{s.favorite_categories.iter().map(|k|view!{<option value=k.id.to_string() selected=selected==Some(k.id)>{k.name.clone()}</option>}).collect_view()}}.into_any()
    };
    view!{<h2>"Manage Favorites"</h2><p class="muted-copy">{format!("{} of {} conversations. Arrange them into categories; only you see this.",s.favorites.len(),MAX_FAVORITES)}</p>
        <form class="my-4 flex flex-wrap items-end gap-2" data-nav-form="fav-category"><div class="field mb-0 min-w-0 flex-1"><label for="nav-fav-cat-name">"New Category"</label><input id="nav-fav-cat-name" type="text" maxlength="32" required/></div><button class="button primary" type="submit">"Create"</button></form>
        {s.favorite_categories.iter().map(|k|view!{<form class="mb-2 flex flex-wrap items-end gap-2" data-nav-form=format!("fav-rename:{}",k.id)><div class="field mb-0 min-w-0 flex-1"><label for=format!("nav-fav-cat-name-{}",k.id)>"Category"</label><input id=format!("nav-fav-cat-name-{}",k.id) type="text" maxlength="32" required value=k.name.clone()/></div><button class="button secondary" type="submit">"Rename"</button><button class="button danger-text" type="button" data-live=format!("nav-fav-cat-delete:{}",k.id)>"Delete"</button></form>}).collect_view()}
        <h3 class="category mt-6 mb-2">"Conversations"</h3>
        {if s.favorites.is_empty(){view!{<p class="text-sm text-subtle">"Nothing here yet. Open a conversation’s options and choose Add to Favorites."</p>}.into_any()}else{().into_any()}}
        {s.favorites.iter().filter_map(|f|a.model.channels.get(&f.channel_id).map(|c|(f,c))).map(|(f,c)|view!{<div class="nav-fav-row flex flex-wrap items-center gap-2 border-b py-2">{icon_with(conv_icon(c),"icon size-4 text-muted-foreground")}<span class="min-w-0 flex-1 truncate text-sm">{a.model.channel_name(c)}</span><label class="sr-only" for=format!("nav-fav-cat-{}",c.id.0)>"Category"</label><select id=format!("nav-fav-cat-{}",c.id.0) data-nav-fav-category=c.id.0.to_string()>{options(f.category_id)}</select>{ib_plain("arrow-up","Move Up",&format!("nav-fav-step:{},-1",c.id.0))}{ib_plain("arrow-down","Move Down",&format!("nav-fav-step:{},1",c.id.0))}{ib_plain("x","Remove from Favorites",&format!("nav-fav:{}",c.id.0))}</div>}).collect_view()}
    }.into_any()
}
