//! Navigation features in the live client: message search, the Quick
//! Switcher, the inbox (unreads, mentions, saved messages and reminders),
//! pinned conversations, Favorites and server folders. State lives in
//! `App::nav`; views are in `ui/navui.rs`; keyboard, drag and suggestion
//! behavior is in `public/nav.js`. Every action here is named `nav-*`.
use super::*;
use crate::nav_logic::{self, Item, Place, Slot};
use serde::{de::DeserializeOwned, Serialize};
use sticktalk_client_core::http::decode_json;

#[derive(Default)]
pub(super) struct NavState {
    pub settings: NavSettings,
    /// The Favorites entry on the rail is the one selected.
    pub favorites: bool,
    pub search_open: bool,
    pub search_query: String,
    /// The server searched, or None for direct messages.
    pub search_scope: Option<Id>,
    /// The direct message or group searched, when one was open.
    pub search_channel: Option<Id>,
    pub search: Option<Result<MessageSearchPage, String>>,
    /// `before` cursors of the pages already passed, for Previous.
    pub search_pages: Vec<Option<Id>>,
    pub search_before: Option<Id>,
    pub generation: u64,
    pub inbox_tab: String,
    pub unreads: Option<Result<InboxUnreads, String>>,
    pub mentions: Option<Result<MessageSearchPage, String>>,
    pub mentions_everyone: bool,
    pub mentions_roles: bool,
    pub saved: Option<Result<SavedPage, String>>,
    pub busy: bool,
    /// A reminder that has just come due, shown until dismissed or opened.
    pub reminder: Option<SavedMessage>,
}
impl NavState {
    pub fn is_pinned(&self, c: Id) -> bool {
        self.settings.pinned.contains(&c)
    }
    pub fn is_favorite(&self, c: Id) -> bool {
        self.settings.favorites.iter().any(|f| f.channel_id == c)
    }
}

async fn call<T: DeserializeOwned>(
    http: &Http<CookieFetch>,
    method: &'static str,
    path: &str,
    body: Option<&impl Serialize>,
) -> Result<T, String> {
    let (body, headers) = match body {
        Some(b) => (
            Body::Bytes(serde_json::to_vec(b).map_err(|_| "Could not encode request")?),
            vec![("Content-Type".into(), "application/json".into())],
        ),
        None => (Body::Empty, vec![]),
    };
    decode_json(
        http.execute(method, &format!("/api/v1{path}"), body, headers, 30_000)
            .await?,
    )
}
const NO_BODY: Option<&()> = None;
fn encode(s: &str) -> String {
    js_sys::encode_uri_component(s).into()
}
/// Runs `f` with the app after the current borrow has ended.
fn later(a: &App, f: impl FnOnce(Shared) + 'static) {
    let owner = a.owner.clone();
    spawn_local(async move {
        if let Some(app) = owner.upgrade() {
            f(app);
        }
    });
}

/// After every Ready: fetch the account's arrangement (the rail's folders and
/// order, pinned conversations, favorites). Called by the core: the first
/// screen draws it, and the state module would bring 50 KB of shared chunks
/// to sign-in with it (task `headroom`).
pub(super) fn ready(app: Shared) {
    let (http, epoch) = {
        let a = app.borrow();
        (a.http(), a.epoch)
    };
    spawn_local(async move {
        let result = http.json::<Snapshot<ReadyNav>>("/users/@me/nav").await;
        let mut a = app.borrow_mut();
        if a.epoch != epoch {
            return;
        }
        if let Ok(s) = result {
            a.nav.settings = s.data.into();
            a.touch();
        }
    });
}
/// `NavSettings` as the core reads it after Ready: the same fields, the
/// rail's adjacently tagged items as plain `{type, id}` pairs. serde's derived
/// decoder for an adjacently tagged enum carries a second, buffered path
/// (docs/features/perf.md), which the core would otherwise keep for this.
#[derive(serde::Deserialize)]
struct ReadyNav {
    #[serde(default)]
    pinned: Vec<ChannelId>,
    #[serde(default)]
    favorite_categories: Vec<FavoriteCategory>,
    #[serde(default)]
    favorites: Vec<FavoriteEntry>,
    #[serde(default)]
    folders: Vec<ServerFolder>,
    #[serde(default)]
    rail: Vec<RailWire>,
    #[serde(default)]
    saved: SavedSummary,
}
#[derive(serde::Deserialize)]
struct RailWire {
    #[serde(rename = "type")]
    kind: String,
    id: u64,
}
impl From<ReadyNav> for NavSettings {
    fn from(n: ReadyNav) -> Self {
        Self {
            pinned: n.pinned,
            favorite_categories: n.favorite_categories,
            favorites: n.favorites,
            folders: n.folders,
            rail: n
                .rail
                .into_iter()
                .filter_map(|r| match r.kind.as_str() {
                    "guild" => Some(RailItem::Guild(Id(r.id))),
                    "folder" => u32::try_from(r.id).ok().map(RailItem::Folder),
                    _ => None,
                })
                .collect(),
            saved: n.saved,
        }
    }
}
/// Navigation events. Called with the app borrowed; refreshes run after.
pub(super) fn event(a: &mut App, event: &Event) {
    match event {
        Event::NavUpdated(s) => a.nav.settings = s.clone(),
        Event::SavedUpdated(s) => {
            a.nav.settings.saved = *s;
            if a.dialog == "nav-inbox" && a.nav.inbox_tab == "saved" {
                later(a, |app| fetch_saved(app, false));
            }
        }
        Event::ReminderDue(item) => {
            a.nav.reminder = Some(item.clone());
        }
        _ => {}
    }
}

pub(super) fn action(app: Shared, kind: &str, arg: &str) {
    match kind {
        "nav-favorites" => {
            let mut a = app.borrow_mut();
            a.nav.favorites = true;
            a.touch();
        }
        "nav-go" => {
            close_dialog(&app);
            go(app, arg);
        }
        "nav-jump" => {
            if let Some((c, m)) = arg
                .split_once(',')
                .and_then(|(c, m)| Some((id(c)?, id(m)?)))
            {
                jump(app, c, m);
            }
        }
        "nav-switcher" => open(&app, "nav-switcher"),
        "nav-menu" => open(&app, &format!("nav-{arg}")),
        "nav-search-open" => {
            let mut a = app.borrow_mut();
            a.nav.search_open = true;
            a.panel.clear();
            a.touch();
        }
        "nav-search-close" => {
            let mut a = app.borrow_mut();
            a.nav.search_open = false;
            a.nav.search = None;
            a.nav.search_query.clear();
            a.nav.generation = a.nav.generation.wrapping_add(1);
            a.touch();
        }
        "nav-search" => {
            let query: String = arg.trim().chars().take(512).collect();
            {
                let mut a = app.borrow_mut();
                a.nav.search_open = true;
                a.nav.search_query = query.clone();
                a.nav.search_scope = a.guild;
                a.nav.search_channel = a
                    .channel
                    .filter(|c| {
                        a.model
                            .channels
                            .get(c)
                            .is_some_and(|c| c.guild_id.is_none())
                    })
                    .filter(|_| a.nav.search_scope.is_none());
                a.nav.search_pages.clear();
                a.nav.search_before = None;
                if query.is_empty() {
                    a.nav.search = None;
                    a.touch();
                    return;
                }
            }
            search(app);
        }
        "nav-search-page" => {
            {
                let mut a = app.borrow_mut();
                let next = match &a.nav.search {
                    Some(Ok(p)) => p.next_before,
                    _ => None,
                };
                if arg == "next" {
                    let Some(next) = next else { return };
                    let current = a.nav.search_before;
                    a.nav.search_pages.push(current);
                    a.nav.search_before = Some(next);
                } else {
                    let Some(previous) = a.nav.search_pages.pop() else {
                        return;
                    };
                    a.nav.search_before = previous;
                }
            }
            search(app);
        }
        "nav-inbox" => {
            {
                let mut a = app.borrow_mut();
                let tab = if arg.is_empty() { "unreads" } else { arg };
                a.nav.inbox_tab = tab.into();
                a.nav.unreads = None;
                a.nav.mentions = None;
                a.nav.saved = None;
            }
            open(&app, "nav-inbox");
            inbox_tab(app);
        }
        "nav-inbox-tab" => {
            app.borrow_mut().nav.inbox_tab = arg.into();
            app.borrow().touch();
            inbox_tab(app);
        }
        "nav-mentions-filter" => {
            {
                let mut a = app.borrow_mut();
                match arg {
                    "everyone" => a.nav.mentions_everyone = !a.nav.mentions_everyone,
                    _ => a.nav.mentions_roles = !a.nav.mentions_roles,
                }
                a.nav.mentions = None;
                a.touch();
            }
            fetch_mentions(app, false);
        }
        "nav-mentions-more" => fetch_mentions(app, true),
        "nav-saved-more" => fetch_saved(app, true),
        "nav-ack" | "nav-ack-all" => {
            let ids: Vec<Id> = if kind == "nav-ack" {
                id(arg).into_iter().collect()
            } else {
                match &app.borrow().nav.unreads {
                    Some(Ok(u)) => u.channels.iter().map(|c| c.read.channel_id).collect(),
                    _ => vec![],
                }
            };
            ack(app, ids);
        }
        "nav-ack-guild" => {
            let ids: Vec<Id> = {
                let a = app.borrow();
                a.model
                    .channels
                    .values()
                    .filter(|c| c.guild_id == id(arg) && c.kind == ChannelKind::Text)
                    .filter(|c| a.model.reads.get(&c.id).is_some_and(|r| r.unread_count > 0))
                    .map(|c| c.id)
                    .take(MAX_ACK_CHANNELS)
                    .collect()
            };
            close_dialog(&app);
            ack(app, ids);
        }
        "nav-save" => {
            let channel = app.borrow().channel;
            if let (Some(c), Some(m)) = (channel, id(arg)) {
                save(app, c, m, None, "Saved. Find it in Inbox, Saved.");
            }
        }
        "nav-remind" => {
            let channel = app.borrow().channel;
            if let (Some(c), Some(m)) = (channel, id(arg)) {
                open(&app, &format!("nav-remind:{},{}", c.0, m.0));
            }
        }
        "nav-remind-at" => {
            let mut parts = arg.split(',');
            let (Some(c), Some(m), Some(preset)) = (
                parts.next().and_then(id),
                parts.next().and_then(id),
                parts.next(),
            ) else {
                return;
            };
            let now = Browser.unix_ms();
            let at = if preset == "custom" {
                let text = value("nav-remind-custom");
                let ms = js_sys::Date::new(&JsValue::from_str(&text)).get_time();
                (ms.is_finite() && ms > 0.).then_some(ms as u64)
            } else {
                let d = js_sys::Date::new_0();
                let minutes = u64::from(d.get_hours()) * 60 + u64::from(d.get_minutes());
                nav_logic::reminder_at(now, preset, minutes)
            };
            let Some(at) = at.filter(|t| *t > now) else {
                fail(&app, "Choose a time in the future.");
                return;
            };
            close_dialog(&app);
            let when = js_sys::Date::new(&JsValue::from_f64(at as f64))
                .to_locale_string("default", &JsValue::UNDEFINED)
                .as_string()
                .unwrap_or_default();
            save(app, c, m, Some(at), &format!("Reminder set for {when}."));
        }
        "nav-unsave" | "nav-reminder-clear" => {
            if let Some(m) = id(arg) {
                unsave(app, m, kind == "nav-reminder-clear");
            }
        }
        "nav-reminder-dismiss" => {
            let mut a = app.borrow_mut();
            a.nav.reminder = None;
            a.touch();
        }
        "nav-pin" => {
            if let Some(c) = id(arg) {
                let mut list = app.borrow().nav.settings.pinned.clone();
                if list.contains(&c) {
                    list.retain(|v| *v != c);
                } else if list.len() >= MAX_PINNED_CONVERSATIONS {
                    fail(&app, "You can pin up to 50 conversations.");
                    return;
                } else {
                    list.push(c);
                }
                close_dialog(&app);
                put(app, "/users/@me/nav/pins", SetPinned { channel_ids: list });
            }
        }
        "nav-fav" => {
            if let Some(c) = id(arg) {
                let s = app.borrow().nav.settings.clone();
                let mut favorites = s.favorites.clone();
                if favorites.iter().any(|f| f.channel_id == c) {
                    favorites.retain(|f| f.channel_id != c);
                } else if favorites.len() >= MAX_FAVORITES {
                    fail(&app, "Favorites can hold up to 200 conversations.");
                    return;
                } else {
                    favorites.push(FavoriteEntry {
                        channel_id: c,
                        category_id: None,
                    });
                }
                close_dialog(&app);
                put_favorites(app, s.favorite_categories, favorites);
            }
        }
        "nav-fav-step" => {
            if let Some((c, d)) = arg.split_once(',') {
                let s = app.borrow().nav.settings.clone();
                if let (Some(c), Ok(d)) = (id(c), d.parse::<i32>()) {
                    if let Some(list) = nav_logic::favorite_step(&s.favorites, c, d) {
                        put_favorites(app, s.favorite_categories, list);
                    }
                }
            }
        }
        "nav-fav-move" => {
            if let Some((c, k)) = arg.split_once(',') {
                let s = app.borrow().nav.settings.clone();
                let category = k.parse::<u32>().ok().filter(|k| *k > 0);
                if let Some(c) = id(c) {
                    let mut list = s.favorites.clone();
                    if let Some(i) = list.iter().position(|f| f.channel_id == c) {
                        let mut entry = list.remove(i);
                        entry.category_id = category;
                        list.push(entry);
                        put_favorites(app, s.favorite_categories, list);
                    }
                }
            }
        }
        "nav-fav-cat-toggle" | "nav-fav-cat-delete" => {
            if let Ok(k) = arg.parse::<u32>() {
                let s = app.borrow().nav.settings.clone();
                let mut categories = s.favorite_categories.clone();
                let mut favorites = s.favorites.clone();
                if kind == "nav-fav-cat-toggle" {
                    for c in categories.iter_mut().filter(|c| c.id == k) {
                        c.collapsed = !c.collapsed;
                    }
                } else {
                    categories.retain(|c| c.id != k);
                    for f in favorites.iter_mut().filter(|f| f.category_id == Some(k)) {
                        f.category_id = None;
                    }
                }
                put_favorites(app, categories, favorites);
            }
        }
        "nav-form" => form(app, arg),
        "nav-rail-drop" => {
            let mut parts = arg.split(',');
            if let (Some(src), Some(dst), Some(place)) = (
                parts.next().and_then(Item::parse),
                parts.next().and_then(Item::parse),
                parts.next().and_then(Place::parse),
            ) {
                rail(app, |l| nav_logic::drop(l, src, dst, place));
            }
        }
        "nav-rail-step" => {
            if let Some((item, d)) = arg.split_once(',') {
                if let (Some(item), Ok(d)) = (Item::parse(item), d.parse::<i32>()) {
                    rail(app, |l| nav_logic::step(l, item, d));
                }
            }
        }
        "nav-folder-new" => {
            if let Some(g) = id(arg) {
                close_dialog(&app);
                rail(app, |l| nav_logic::into_folder(l, g, None));
            }
        }
        "nav-folder-add" => {
            if let Some((g, f)) = arg.split_once(',') {
                if let (Some(g), Ok(f)) = (id(g), f.parse::<u32>()) {
                    close_dialog(&app);
                    rail(app, |l| nav_logic::into_folder(l, g, Some(f)));
                }
            }
        }
        "nav-folder-out" => {
            if let Some(g) = id(arg) {
                close_dialog(&app);
                rail(app, |l| nav_logic::out_of_folder(l, g));
            }
        }
        "nav-folder-ungroup" => {
            if let Ok(f) = arg.parse::<u32>() {
                close_dialog(&app);
                rail(app, |l| nav_logic::ungroup(l, f));
            }
        }
        "nav-folder-toggle" => {
            if let Ok(f) = arg.parse::<u32>() {
                rail(app, |l| {
                    nav_logic::edit_folder(l, f, |f| f.collapsed = !f.collapsed)
                });
            }
        }
        _ => {}
    }
}

fn fail(app: &Shared, text: &str) {
    let mut a = app.borrow_mut();
    a.error = text.into();
    a.notice.clear();
    a.touch();
}
fn notice(app: &Shared, text: &str) {
    let mut a = app.borrow_mut();
    a.error = text.into();
    a.notice = text.into();
    a.touch();
}
fn open(app: &Shared, dialog: &str) {
    let mut a = app.borrow_mut();
    a.dialog = dialog.into();
    a.dialog_generation = a.dialog_generation.wrapping_add(1);
    a.error.clear();
    a.touch();
}
fn close_dialog(app: &Shared) {
    let mut a = app.borrow_mut();
    if !a.dialog.is_empty() {
        a.dialog.clear();
        a.dialog_generation = a.dialog_generation.wrapping_add(1);
        a.touch();
    }
}
/// Open what a Quick Switcher row or a Favorites row names.
fn go(app: Shared, target: &str) {
    let channel = target.strip_prefix("channel:").and_then(id);
    {
        let mut a = app.borrow_mut();
        a.nav.favorites = match channel {
            Some(c) => a.nav.favorites && a.nav.is_favorite(c),
            None => target == "nav-favorites",
        };
    }
    if target != "nav-favorites" {
        operations::action(app, target);
    }
}
/// Show a message in its conversation, wherever that is.
fn jump(app: Shared, c: Id, m: Id) {
    let same = {
        let mut a = app.borrow_mut();
        if !a.model.channels.contains_key(&c) {
            drop(a);
            // A message in a thread the client has not loaded yet.
            if super::threads::jump(app.clone(), c, m) {
                return;
            }
            fail(&app, "That conversation is no longer available.");
            return;
        }
        a.dialog.clear();
        a.dialog_generation = a.dialog_generation.wrapping_add(1);
        a.nav.favorites = a.nav.favorites && a.nav.is_favorite(c);
        a.nav.reminder = a.nav.reminder.take().filter(|r| r.message_id != m);
        // On a phone the results cover the conversation: close them to show it.
        if window()
            .inner_width()
            .ok()
            .and_then(|w| w.as_f64())
            .is_some_and(|w| w < 768.)
        {
            a.nav.search_open = false;
        }
        a.touch();
        a.channel == Some(c)
    };
    if same {
        operations::action(app, &format!("seek-message:{}", m.0));
    } else {
        operations::load_history(app, c, None, None, Some(m));
    }
}

fn search(app: Shared) {
    let (http, path, generation, epoch) = {
        let mut a = app.borrow_mut();
        a.nav.generation = a.nav.generation.wrapping_add(1);
        a.nav.search = None;
        a.touch();
        let q = encode(&a.nav.search_query);
        let before = a
            .nav
            .search_before
            .map_or(String::new(), |b| format!("&before={}", b.0));
        let path = match a.nav.search_scope {
            Some(g) => format!("/guilds/{}/messages/search?limit=25&q={q}{before}", g.0),
            None => format!(
                "/users/@me/messages/search?limit=25&q={q}{before}{}",
                a.nav
                    .search_channel
                    .map_or(String::new(), |c| format!("&channel_id={}", c.0))
            ),
        };
        (a.http(), path, a.nav.generation, a.epoch)
    };
    spawn_local(async move {
        let result = http.json::<Snapshot<MessageSearchPage>>(&path).await;
        let mut a = app.borrow_mut();
        if a.epoch != epoch || a.nav.generation != generation {
            return;
        }
        let mut places = vec![];
        a.nav.search = Some(result.map(|s| {
            for u in &s.data.users {
                a.model.users.entry(u.id).or_insert_with(|| u.clone());
            }
            places = s.data.items.iter().map(|m| m.channel_id).collect();
            s.data
        }));
        a.touch();
        drop(a);
        if !places.is_empty() {
            super::threads::learn(&app, places);
        }
    });
}

fn inbox_tab(app: Shared) {
    let tab = app.borrow().nav.inbox_tab.clone();
    match tab.as_str() {
        "mentions" => {
            if app.borrow().nav.mentions.is_none() {
                {
                    let mut a = app.borrow_mut();
                    if !a.nav.mentions_everyone && !a.nav.mentions_roles {
                        // First visit: include everything, as Discord does.
                        a.nav.mentions_everyone = true;
                        a.nav.mentions_roles = true;
                    }
                }
                fetch_mentions(app, false)
            }
        }
        "saved" => {
            if app.borrow().nav.saved.is_none() {
                fetch_saved(app, false)
            }
        }
        _ => {
            if app.borrow().nav.unreads.is_none() {
                fetch_unreads(app)
            }
        }
    }
}
fn fetch_unreads(app: Shared) {
    let (http, epoch) = (app.borrow().http(), app.borrow().epoch);
    spawn_local(async move {
        let result = http
            .json::<Snapshot<InboxUnreads>>("/users/@me/unreads")
            .await;
        let mut a = app.borrow_mut();
        if a.epoch != epoch || a.dialog != "nav-inbox" {
            return;
        }
        a.nav.unreads = Some(result.map(|s| {
            for u in &s.data.users {
                a.model.users.entry(u.id).or_insert_with(|| u.clone());
            }
            s.data
        }));
        a.touch();
    });
}
fn fetch_mentions(app: Shared, more: bool) {
    let (http, epoch, path) = {
        let mut a = app.borrow_mut();
        if a.nav.busy && more {
            return;
        }
        let before = match (&a.nav.mentions, more) {
            (Some(Ok(p)), true) => match p.next_before {
                Some(b) => format!("&before={}", b.0),
                None => return,
            },
            _ => String::new(),
        };
        a.nav.busy = true;
        (
            a.http(),
            a.epoch,
            format!(
                "/users/@me/mentions?limit=25&everyone={}&roles={}{before}",
                a.nav.mentions_everyone, a.nav.mentions_roles
            ),
        )
    };
    spawn_local(async move {
        let result = http.json::<Snapshot<MessageSearchPage>>(&path).await;
        let mut a = app.borrow_mut();
        a.nav.busy = false;
        if a.epoch != epoch || a.dialog != "nav-inbox" {
            return;
        }
        match (result, a.nav.mentions.take(), more) {
            (Ok(s), Some(Ok(mut old)), true) => {
                for u in &s.data.users {
                    a.model.users.entry(u.id).or_insert_with(|| u.clone());
                }
                old.items.extend(s.data.items);
                old.items.truncate(500);
                old.next_before = s.data.next_before;
                old.has_more = s.data.has_more && old.items.len() < 500;
                a.nav.mentions = Some(Ok(old));
            }
            (Ok(s), _, _) => {
                for u in &s.data.users {
                    a.model.users.entry(u.id).or_insert_with(|| u.clone());
                }
                a.nav.mentions = Some(Ok(s.data));
            }
            (Err(e), _, _) => a.nav.mentions = Some(Err(e)),
        }
        let places: Vec<Id> = match &a.nav.mentions {
            Some(Ok(p)) => p.items.iter().map(|m| m.channel_id).collect(),
            _ => vec![],
        };
        a.touch();
        drop(a);
        super::threads::learn(&app, places);
    });
}
fn fetch_saved(app: Shared, more: bool) {
    let (http, epoch, path) = {
        let mut a = app.borrow_mut();
        if a.nav.busy && more {
            return;
        }
        let before = match (&a.nav.saved, more) {
            (Some(Ok(p)), true) => match p.next_before {
                Some(b) => format!("&before={b}"),
                None => return,
            },
            _ => String::new(),
        };
        a.nav.busy = true;
        (
            a.http(),
            a.epoch,
            format!("/users/@me/saved?limit=50{before}"),
        )
    };
    spawn_local(async move {
        let result = http.json::<Snapshot<SavedPage>>(&path).await;
        let mut a = app.borrow_mut();
        a.nav.busy = false;
        if a.epoch != epoch || a.dialog != "nav-inbox" {
            return;
        }
        match (result, a.nav.saved.take(), more) {
            (Ok(s), Some(Ok(mut old)), true) => {
                for u in &s.data.users {
                    a.model.users.entry(u.id).or_insert_with(|| u.clone());
                }
                old.items.extend(s.data.items);
                old.next_before = s.data.next_before;
                old.has_more = s.data.has_more;
                old.summary = s.data.summary;
                a.nav.settings.saved = s.data.summary;
                a.nav.saved = Some(Ok(old));
            }
            (Ok(s), _, _) => {
                for u in &s.data.users {
                    a.model.users.entry(u.id).or_insert_with(|| u.clone());
                }
                a.nav.settings.saved = s.data.summary;
                a.nav.saved = Some(Ok(s.data));
            }
            (Err(e), _, _) => a.nav.saved = Some(Err(e)),
        }
        let places: Vec<Id> = match &a.nav.saved {
            Some(Ok(p)) => p.items.iter().map(|i| i.channel_id).collect(),
            _ => vec![],
        };
        a.touch();
        drop(a);
        super::threads::learn(&app, places);
    });
}
fn ack(app: Shared, channel_ids: Vec<Id>) {
    if channel_ids.is_empty() {
        return;
    }
    let http = app.borrow().http();
    spawn_local(async move {
        let body = AckChannels {
            channel_ids: channel_ids.clone(),
        };
        match call::<AckResult>(&http, "POST", "/users/@me/unreads/ack", Some(&body)).await {
            Ok(r) => {
                let mut a = app.borrow_mut();
                for s in r.read_states {
                    a.model.reads.insert(s.channel_id, s);
                }
                if let Some(Ok(u)) = &mut a.nav.unreads {
                    u.channels
                        .retain(|c| !channel_ids.contains(&c.read.channel_id));
                }
                a.touch();
            }
            Err(e) => fail(&app, &e),
        }
    });
}
fn save(app: Shared, c: Id, m: Id, remind_at: Option<u64>, done: &str) {
    let http = app.borrow().http();
    let done = done.to_owned();
    spawn_local(async move {
        let body = SaveMessage {
            channel_id: c,
            remind_at,
            clear_reminder: false,
        };
        match call::<SavedMessage>(
            &http,
            "PUT",
            &format!("/users/@me/saved/{}", m.0),
            Some(&body),
        )
        .await
        {
            Ok(_) => notice(&app, &done),
            Err(e) if e == "A limit has been reached." => fail(
                &app,
                "You can save up to 200 messages. Remove some in Inbox, Saved.",
            ),
            Err(e) => fail(&app, &e),
        }
    });
}
fn unsave(app: Shared, m: Id, reminder_only: bool) {
    let http = app.borrow().http();
    spawn_local(async move {
        let path = format!(
            "/users/@me/saved/{}{}",
            m.0,
            if reminder_only {
                "?reminder_only=true"
            } else {
                ""
            }
        );
        match call::<SavedSummary>(&http, "DELETE", &path, NO_BODY).await {
            Ok(s) => {
                let mut a = app.borrow_mut();
                a.nav.settings.saved = s;
                if let Some(Ok(page)) = &mut a.nav.saved {
                    if reminder_only {
                        for item in page.items.iter_mut().filter(|i| i.message_id == m) {
                            item.remind_at = None;
                            item.due = false;
                        }
                    } else {
                        page.items.retain(|i| i.message_id != m);
                    }
                    page.summary = s;
                }
                a.touch();
            }
            Err(e) => fail(&app, &e),
        }
    });
}
/// Send a complete navigation write, showing it at once and keeping the
/// server's answer (or going back to the server's state if it is refused).
fn put(app: Shared, path: &'static str, body: impl Serialize + 'static) {
    let http = app.borrow().http();
    spawn_local(async move {
        let result = call::<NavSettings>(&http, "PUT", path, Some(&body)).await;
        match result {
            Ok(s) => {
                let mut a = app.borrow_mut();
                a.nav.settings = s;
                a.touch();
            }
            Err(e) => {
                fail(&app, &e);
                ready(app);
            }
        }
    });
}
fn put_favorites(app: Shared, categories: Vec<FavoriteCategory>, favorites: Vec<FavoriteEntry>) {
    {
        let mut a = app.borrow_mut();
        a.nav.settings.favorite_categories = categories.clone();
        a.nav.settings.favorites = favorites.clone();
        a.touch();
    }
    put(
        app,
        "/users/@me/nav/favorites",
        SetFavorites {
            categories,
            favorites,
        },
    );
}
/// The rail as shown now.
pub(super) fn layout(a: &App) -> Vec<Slot> {
    let guilds: Vec<Id> = a.model.guilds.keys().copied().collect();
    nav_logic::layout(&a.nav.settings, &guilds)
}
fn rail(app: Shared, change: impl FnOnce(&[Slot]) -> Option<Vec<Slot>>) {
    let next = {
        let a = app.borrow();
        change(&layout(&a))
    };
    let Some(next) = next else { return };
    let body = nav_logic::to_rail(&next);
    {
        let mut a = app.borrow_mut();
        a.nav.settings.folders = body.folders.clone();
        a.nav.settings.rail = body.order.clone();
        a.touch();
    }
    put(app, "/users/@me/nav/rail", body);
}
/// Forms in navigation dialogs. Fields are read by ID, as elsewhere.
fn form(app: Shared, kind: &str) {
    let (kind, arg) = kind.split_once(':').unwrap_or((kind, ""));
    match kind {
        "remind" => action(app, "nav-remind-at", &format!("{arg},custom")),
        "folder" => {
            let Ok(f) = arg.parse::<u32>() else { return };
            let name = value("nav-folder-name").trim().to_owned();
            if !valid_folder_name(&name) {
                fail(&app, "Folder names are 1 to 32 characters.");
                return;
            }
            let color = u32::from_str_radix(value("nav-folder-color").trim_start_matches('#'), 16)
                .ok()
                .filter(|c| *c <= 0xffffff);
            close_dialog(&app);
            rail(app, |l| {
                nav_logic::edit_folder(l, f, |folder| {
                    folder.name = name;
                    if let Some(c) = color {
                        folder.color = c;
                    }
                })
            });
        }
        "fav-category" | "fav-rename" => {
            let field = if kind == "fav-category" {
                "nav-fav-cat-name".to_owned()
            } else {
                format!("nav-fav-cat-name-{arg}")
            };
            let name = value(&field).trim().to_owned();
            if !valid_category_name(&name) {
                fail(&app, "Category names are 1 to 32 characters.");
                return;
            }
            let s = app.borrow().nav.settings.clone();
            let mut categories = s.favorite_categories.clone();
            if kind == "fav-category" {
                if categories.len() >= MAX_FAVORITE_CATEGORIES {
                    fail(&app, "Favorites can have up to 20 categories.");
                    return;
                }
                let id = categories.iter().map(|c| c.id).max().unwrap_or(0) + 1;
                categories.push(FavoriteCategory {
                    id,
                    name,
                    collapsed: false,
                });
            } else if let Ok(k) = arg.parse::<u32>() {
                for c in categories.iter_mut().filter(|c| c.id == k) {
                    c.name = name.clone();
                }
            }
            put_favorites(app, categories, s.favorites);
        }
        _ => {}
    }
}

/// The navigation feature's one entry point (`features.rs`).
#[leptos::wasm_split::wasm_split(
    nav,
    wasm_split_path = ::leptos::wasm_split,
    fallible,
    preload(prefetch)
)]
pub(super) fn run(app: Shared, call: features::Call) -> features::Outcome {
    if let features::Call::Action(kind, arg) = call {
        action(app, &kind, &arg);
    }
    Ok(true)
}

/// The nav feature's state: its events, the answers to its commands and the fresh snapshot (Ready).
/// A separate, small split entry: keeping a feature's state current does not
/// load its pages and forms (`features.rs`).
#[leptos::wasm_split::wasm_split(
    nav_state,
    wasm_split_path = ::leptos::wasm_split,
    fallible
)]
pub(super) fn state(app: Shared, call: features::Call) -> features::Outcome {
    use features::Call;
    match call {
        Call::Event(raw) => {
            let e = match raw.tag {
                "nav_updated" => inbound::de(&raw.data).map(Event::NavUpdated),
                "saved_updated" => inbound::de(&raw.data).map(Event::SavedUpdated),
                "reminder_due" => inbound::de(&raw.data).map(Event::ReminderDue),
                _ => Err(()),
            };
            let Ok(e) = e else { return Ok(false) };
            event(&mut app.borrow_mut(), &e);
        }

        _ => {}
    }
    Ok(true)
}
