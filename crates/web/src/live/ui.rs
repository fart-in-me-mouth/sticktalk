//! Live client views. Styling is Tailwind CSS utilities plus the shadcn/ui
//! component classes in `styles/app.css`; dialogs, tooltips, menus and avatars
//! are UI SDK custom elements (`public/ui.js`). Semantic class names such as
//! `message` or `live-modal` carry no styles of their own here: they are stable
//! hooks for `public/*.js` and the browser tests.
use super::*;
mod activities;
pub(in crate::live) mod apps;
mod authsec;
mod call_extras;
mod call_views;
mod community;
mod content;
mod expression;
mod identity;
pub(super) use identity::load_invite_view;
mod moderation;
mod navui;
pub(super) mod picker;
mod privacy;
mod profiles;
mod settings;
mod stage;
mod themes;
mod threads;
use sticktalk_client_core::markdown::{Block, Inline, Target};

fn icon(name: &str) -> AnyView {
    icon_with(name, "icon")
}
fn icon_with(name: &str, class: &'static str) -> AnyView {
    view! { <svg class=class aria-hidden="true" viewBox="0 0 24 24"><use href=format!("#i-{name}")></use></svg> }.into_any()
}
/// shadcn Button. `class` picks the variant: primary, secondary, danger,
/// link-button or danger-text.
/// Confirmation for a form that stays open after a successful save.
fn saved_note(a: &App, form: &'static str) -> AnyView {
    if a.saved == Some((a.dialog_generation, form)) {
        view! {<p class="form-saved" role="status">"Saved."</p>}.into_any()
    } else {
        ().into_any()
    }
}
fn button(label: &str, action: &str, class: &str) -> AnyView {
    view!{<button class=format!("button {class}") data-live=action.to_owned()>{label.to_owned()}</button>}.into_any()
}
/// Ghost icon button with a UI SDK tooltip naming it.
fn ib(glyph: &str, label: &str, action: &str) -> AnyView {
    view!{<ui-tooltip class="contents"><ui-tooltip-trigger><button class="icon-button" aria-label=label.to_owned() data-live=action.to_owned()>{icon(glyph)}</button></ui-tooltip-trigger><ui-tooltip-content class="tooltip" hidden=true>{label.to_owned()}</ui-tooltip-content></ui-tooltip>}.into_any()
}
/// Icon button for dense repeated rows (message actions): no tooltip element each.
fn ib_plain(glyph: &str, label: &str, action: &str) -> AnyView {
    view!{<button class="icon-button" aria-label=label.to_owned() title=label.to_owned() data-live=action.to_owned()>{icon(glyph)}</button>}.into_any()
}
fn field(id: &str, label: &str, text: &str, kind: &str) -> AnyView {
    view!{<div class="field"><label for=id.to_owned()>{label.to_owned()}</label><input id=id.to_owned() type=kind.to_owned() value=text.to_owned()/></div>}.into_any()
}
fn name(u: &UserSummary) -> String {
    super::privacy::nickname(u.id)
        .unwrap_or_else(|| u.display_name.clone().unwrap_or_else(|| u.username.clone()))
}
/// UI SDK Avatar, with its decoration and animation (`ui/profiles.rs`).
fn avatar(u: &UserSummary, size: &str) -> AnyView {
    profiles::avatar(u, size)
}
fn color(a: &App, user: Id) -> String {
    let role = a.model.members.get(&user).and_then(|m| {
        m.role_ids
            .iter()
            .filter_map(|id| a.roles.get(id))
            .filter(|r| r.color != 0)
            .max_by_key(|r| r.position)
    });
    role.map_or("var(--text-primary)".into(), |r| {
        format!("#{:06x}", crate::wire::legible_role_color(r.color))
    })
}
/// The app-wide toast: an error, or a short confirmation (`App::notice`). An
/// open dialog lives in the browser's top layer, so while one is open the toast
/// is rendered inside it (see `modal`). `public/live.js` dismisses it after a while.
fn error_toast(a: &App) -> AnyView {
    if a.error.is_empty() {
        return ().into_any();
    }
    let notice = a.notice == a.error;
    view!{<div class=format!("{} fixed top-[calc(var(--header-height)+var(--safe-top)+8px)] left-1/2 z-[200] flex max-w-[calc(100vw-32px)] -translate-x-1/2 items-center gap-4 rounded-lg border bg-popover px-4 py-3 text-sm shadow-lg phone:w-[calc(100vw-32px)]",if notice{"live-notice border-primary"}else{"live-error border-warning"}) data-toast=if notice{"notice"}else{"error"} role=if notice{"status"}else{"alert"}>{a.error.clone()}{ib_plain("x","Dismiss","dismiss-error")}</div>}.into_any()
}
/// UI SDK Dialog, controlled by `App::dialog`: it traps focus, makes the page
/// inert and reports dismissals, which `public/ui.js` turns into the `close` action.
fn modal(a: &App, class: &str, label: &str, body: AnyView) -> AnyView {
    view!{<ui-dialog class="contents" open=true>
        <ui-dialog-backdrop class="modal-backdrop" hidden=true></ui-dialog-backdrop>
        <ui-dialog-content class=format!("modal live-modal {class}") data-dialog=a.dialog.clone() aria-label=label.to_owned() hidden=true>
            {body}
            <div class="live-modal-close">{ib_plain("x","Close","close")}</div>
            {error_toast(a)}
        </ui-dialog-content>
    </ui-dialog>}.into_any()
}
/// What a full-window settings sheet is called and where its Back button goes.
struct SettingsPage<'a> {
    root: bool,
    label: &'a str,
    nav_label: &'a str,
    mobile_title: &'a str,
    back: (&'a str, &'a str),
    title: &'a str,
}
/// Full-window settings: the same dialog, edge to edge, with its own navigation.
fn settings_page(a: &App, page: SettingsPage, nav: AnyView, body: AnyView) -> AnyView {
    let SettingsPage {
        root,
        label,
        nav_label,
        mobile_title,
        back,
        title,
    } = page;
    view!{<ui-dialog class="contents" open=true>
        <ui-dialog-content class=format!("live-modal settings-page {}",if root{"settings-root"}else{"settings-subpage"}) data-dialog=a.dialog.clone() aria-label=label.to_owned() initial-focus="content" hidden=true>
            <nav class=format!("settings-nav w-[280px] flex-none overflow-y-auto overscroll-contain bg-sidebar px-4 pt-8 pb-[calc(32px+var(--safe-bottom))] [scroll-padding-block:80px_calc(32px+var(--safe-bottom))] narrow:w-60 {}",if root{"phone:h-full phone:w-full phone:pt-[calc(72px+var(--safe-top))] phone:pb-[calc(48px+var(--safe-bottom))]"}else{"phone:hidden"}) aria-label=nav_label.to_owned()>{nav}</nav>
            <div class=format!("settings-main min-h-0 min-w-0 flex-1 overflow-y-auto overscroll-contain {}",if root{"phone:hidden"}else{""})>
                <header class="settings-topbar sticky top-0 z-[3] flex min-h-12 items-center gap-3 border-b bg-background px-6 py-2 phone:min-h-[calc(56px+var(--safe-top))] phone:px-2 phone:pt-[calc(8px+var(--safe-top))]"><div class=if root{"settings-back hidden"}else{"settings-back hidden phone:block"}>{ib_plain("arrow-left",back.0,back.1)}</div><strong class="flex-1 text-base font-semibold">{title.to_owned()}</strong>{ib_plain("x","Close","close")}</header>
                <div class="settings-content">{body}</div>
            </div>
            <header class=if root{"settings-mobile-header hidden phone:absolute phone:top-0 phone:z-[3] phone:flex phone:min-h-[calc(56px+var(--safe-top))] phone:w-full phone:items-center phone:justify-between phone:bg-sidebar phone:px-4 phone:pt-[calc(8px+var(--safe-top))] phone:pb-2"}else{"settings-mobile-header hidden"}><h1>{mobile_title.to_owned()}</h1>{ib_plain("x","Close","close")}</header>
            {error_toast(a)}
        </ui-dialog-content>
    </ui-dialog>}.into_any()
}
/// One row of a settings navigation (shadcn sidebar menu button).
fn settings_nav_row(glyph: &str, label: &str, action: &str, selected: bool) -> AnyView {
    view!{<button class=format!("settings-nav-row flex min-h-9 w-full items-center gap-3 rounded-md px-3 py-2 text-left text-sm font-medium hover:bg-accent hover:text-foreground touch:min-h-12 touch:text-[15px] phone:mt-0.5 phone:min-h-[52px] phone:bg-accent phone:p-3 {}",if selected{"selected bg-secondary text-foreground"}else{"text-subtle"}) data-live=action.to_owned()>{icon(glyph)}<span class="flex-1">{label.to_owned()}</span>{icon_with("chevron-right","icon hidden size-4 phone:block")}</button>}.into_any()
}

pub(super) fn app(a: &App) -> AnyView {
    profiles::begin(a);
    super::themes::sync(a);
    if !a.ready && a.model.cursor.is_none() && (a.model.me.is_some() || a.status == "Connecting…")
    {
        return view!{<div class="app-shell skeleton-shell pointer-events-none relative top-(--viewport-offset) flex h-(--viewport-height) bg-background" role="status" aria-label="Loading conversations"><div class="left-panel flex w-[312px] flex-none bg-sidebar phone:hidden"><nav class="server-rail flex w-[72px] flex-col items-center gap-2 bg-rail p-3"><img class="rounded-2xl" src=themes::logo() data-app-logo="" width="48" height="48" alt="StickTalk"/></nav><aside class="channel-sidebar flex w-60 flex-col"><div class="sidebar-header flex h-12 items-center border-b px-4 font-semibold">"StickTalk"</div><div class="skeleton-lines grid gap-4 px-4 py-6 [&>span]:h-4 [&>span]:w-4/5 [&>span]:rounded-sm [&>span]:bg-muted"><span></span><span></span><span></span><span></span></div></aside></div><main class="main-pane flex min-w-0 flex-1 flex-col"><header class="channel-header flex h-12 flex-none items-center border-b px-4"><strong>"StickTalk"</strong></header>{if a.status=="Reconnecting…"{view!{<div class="connection-banner bg-warning px-4 py-2 text-sm text-rail" role="status">"Reconnecting…"</div>}.into_any()}else{().into_any()}}<div class="skeleton-messages mt-auto grid gap-6 p-4 [&>div]:h-10 [&>div]:w-[min(80%,480px)] [&>div]:rounded-lg [&>div]:bg-muted"><div></div><div></div><div></div></div><div class="skeleton-composer m-4 h-12 rounded-lg bg-muted"></div></main></div>}.into_any();
    }
    view!{<div id="sticktalk" data-live-app data-screen=a.screen.clone() data-self-id=a.model.me.as_ref().map(|me|me.user.id.0.to_string())>
        {if a.model.me.is_none(){auth(a)}else{shell(a)}}
        {if a.model.me.is_some(){call_views::incoming(a)}else{().into_any()}}
        {if a.dialog.is_empty(){error_toast(a)}else if let Some(picker)=picker::view(a){picker}else{a.dialog_render.map_or_else(||loading_dialog(a), |render|render(a))}}

    </div>}.into_any()
}
const AUTH_PAGE: &str = "auth-page grid h-(--viewport-height) place-items-center overflow-y-auto p-6 [scroll-padding-block:calc(16px+var(--safe-top))_calc(24px+var(--safe-bottom))] phone:relative phone:top-(--viewport-offset) phone:flex phone:flex-col phone:justify-[safe_center] phone:bg-background phone:px-6 phone:pt-[calc(32px+var(--safe-top))] phone:pb-[calc(32px+var(--safe-bottom))]";
const AUTH_CARD: &str = "auth-card w-[min(440px,100%)] rounded-xl border bg-background p-8 shadow-sm phone:my-auto phone:w-full phone:max-w-[480px] phone:rounded-none phone:border-0 phone:bg-transparent phone:p-0 phone:shadow-none";
/// The sign-in form the app draws itself: only after a logout or a session
/// that ended (the first sign-in is the static form in index.html). Its view
/// is a split module (task `headroom`); until it arrives, an empty card.
fn auth(a: &App) -> AnyView {
    const PAGE: &str = AUTH_PAGE;
    const CARD: &str = AUTH_CARD;
    if a.status == "Reconnecting…" {
        return view!{<main class=PAGE><section class=CARD><h1>"Reconnecting…"</h1><p class="muted-copy my-4">"Your conversations will appear when the server is available."</p>{button("Try Again","recover","primary")}</section></main>}.into_any();
    }
    lazy_view(a, |a| &a.views.auth, || Box::pin(load_auth())).map_or_else(
        || view!{<main class=PAGE><section class=CARD aria-busy="true"><p class="auth-status my-3 text-[13px] text-muted-foreground empty:hidden" role="status">{a.status.clone()}</p></section></main>}.into_any(),
        |form| form(a),
    )
}
#[lazy]
fn load_auth() -> fn(&App) -> AnyView {
    auth_form
}
fn auth_form(a: &App) -> AnyView {
    const PAGE: &str = AUTH_PAGE;
    const CARD: &str = AUTH_CARD;
    let signup = a.signup;
    let register = a.register;
    view!{<main class=PAGE><form class=CARD data-live-form="auth"><div class="auth-brand flex items-center gap-3 text-xl font-semibold phone:justify-center"><img class="auth-logo flex-none rounded-xl" src=themes::logo() data-app-logo="" alt="" width="48" height="48"/><span>"StickTalk"</span></div><h1 class="mt-8 mb-2 text-[28px] leading-tight phone:text-center">{if signup{"Choose a username"}else if register{"Create an account"}else{"Welcome back!"}}</h1><p class="auth-subtitle mb-8 text-[15px] text-subtle phone:text-center">{if signup{"Choose how people will find you on StickTalk."}else if register{"Create your StickTalk account."}else{"Log in to catch up with your friends."}}</p>
        <div class="field mb-6"><label for="auth-username">"Username"</label><input class="h-12 text-base touch:min-h-[52px]" id="auth-username" name="username" autocomplete="username" autocapitalize="none" autocorrect="off" spellcheck="false" required minlength="3" maxlength="32" autofocus />{if register||signup{view!{<small>"3 to 32 letters, numbers or underscores."</small>}.into_any()}else{().into_any()}}</div>
        {if signup{().into_any()}else{view!{<div class="field mb-6"><label for="auth-password">"Password"</label><input class="h-12 text-base touch:min-h-[52px]" id="auth-password" name="password" type="password" autocomplete=if register{"new-password"}else{"current-password"} required maxlength="128"/>{if register{().into_any()}else{view!{<a class="self-start text-[13px]" href="/?reset">"Forgot your password?"</a>}.into_any()}}</div>}.into_any()}}
        <button class="button primary h-12 w-full text-[15px]" type="submit" disabled=a.status=="Signing in…">{if signup{"Continue"}else if register{"Create Account"}else{"Log In"}}</button>
        <p class="auth-status my-3 text-[13px] text-muted-foreground empty:hidden" role="status">{a.status.clone()}</p>
        {if a.google&&!signup {view!{<a class="button secondary google-button mt-4 flex h-12 w-full border bg-muted text-[15px]" href="/api/v1/browser/google/start"><img src="/public/google-g.svg" alt="" width="20" height="20"/>"Continue with Google"</a>}.into_any()}else{().into_any()}}
        {if signup{().into_any()}else{view!{<p class="auth-switch mt-6 text-[15px] text-subtle phone:text-center">{if register{"Already have an account? "}else{"Need an account? "}}<button class="inline-flex min-h-11 items-center px-1 font-semibold text-primary hover:underline" type="button" data-live="register">{if register{"Log in"}else{"Register"}}</button></p>}.into_any()}}
        <p class="auth-legal mt-4 text-[13px] text-muted-foreground phone:text-center">"By continuing, you agree to the "<a href="/public/terms.html">"Terms of Service"</a>"."</p>
    </form></main>}.into_any()
}
/// Channels in display order, each with how deeply it is nested. In a server, at
/// every level the channels come before the categories (so none reads as part of
/// the category above it) and each one is followed by everything nested inside
/// it. Direct messages: most recently active first.
fn channel_tree(a: &App) -> Vec<(&ChannelSummary, usize)> {
    if a.guild.is_none() {
        let mut root = a
            .model
            .channels
            .values()
            .filter(|c| c.guild_id.is_none() && navui::unpinned(a, c))
            .collect::<Vec<_>>();
        root.sort_by_key(|c| {
            std::cmp::Reverse(
                a.model
                    .reads
                    .get(&c.id)
                    .and_then(|r| r.latest_message_id)
                    .unwrap_or(c.id),
            )
        });
        return root.into_iter().map(|c| (c, 0)).collect();
    }
    let mut inside: BTreeMap<Option<Id>, Vec<&ChannelSummary>> = BTreeMap::new();
    // Threads are listed under their channel by `threads::rows`, not as channels.
    for c in a
        .model
        .channels
        .values()
        .filter(|c| c.guild_id == a.guild && c.kind != ChannelKind::Thread)
    {
        // A channel whose parent has not arrived (or just left) is listed at the top.
        let parent = c.parent_id.filter(|p| a.model.channels.contains_key(p));
        inside.entry(parent).or_default().push(c);
    }
    let mut level = |parent: Option<Id>, depth: usize| {
        let mut list = inside.remove(&parent).unwrap_or_default();
        // Reversed, because the walk below takes channels from the end.
        list.sort_by_key(|c| {
            std::cmp::Reverse((c.kind == ChannelKind::Category, c.position, c.id))
        });
        list.into_iter().map(move |c| (c, depth))
    };
    let mut out = Vec::with_capacity(a.model.channels.len());
    let mut next = level(None, 0).collect::<Vec<_>>();
    while let Some((c, depth)) = next.pop() {
        next.extend(level(Some(c.id), depth + 1));
        out.push((c, depth));
    }
    out
}
/// How far a nested row is moved in, as a style attribute: `step` px per level.
/// Past eight levels rows stop moving in, so a deep tree still fits the sidebar.
fn indent(depth: usize, step: usize) -> Option<String> {
    (depth > 0).then(|| format!("margin-left:{}px", depth.min(8) * step))
}
/// "Create channel in ..." beside a row, for people who manage channels.
fn create_inside(c: &ChannelSummary, label: &str, class: &'static str) -> AnyView {
    view!{<button class=class aria-label=format!("Create channel in {label}") title=format!("Create channel in {label}") data-live=format!("dialog:channel-in:{}",c.id.0)>{icon_with("plus","icon size-4")}</button>}.into_any()
}
/// The places a channel can go, as options: every channel of this server in
/// sidebar order, moved in by depth. `moving` and everything nested in it are
/// left out, because a channel cannot go inside itself.
fn inside_options(a: &App, selected: Option<Id>, moving: Option<Id>) -> AnyView {
    let mut skip = None;
    let mut options = vec![];
    for (c, depth) in channel_tree(a) {
        if skip.is_some_and(|d| depth > d) {
            continue;
        }
        skip = (Some(c.id) == moving).then_some(depth);
        // A forum holds posts, not channels.
        if skip.is_none() && c.kind != ChannelKind::Forum {
            options.push(view!{<option value=c.id.0.to_string() selected=selected==Some(c.id)>{format!("{}{}","\u{2003}".repeat(depth),a.model.channel_name(c))}</option>});
        }
    }
    view!{<option value="" selected=selected.is_none()>"None (top level)"</option>{options.into_iter().collect_view()}}.into_any()
}
/// You are online to yourself; everyone else is as the server last reported,
/// except someone you ignore, whose presence you do not see (privacy.md).
fn online(a: &App, user: Id) -> bool {
    if super::privacy::ignored(user) {
        return false;
    }
    match a.model.presences.get(&user) {
        Some(PresenceStatus::Offline | PresenceStatus::Invisible) => false,
        Some(_) => true,
        None => a.model.me.as_ref().is_some_and(|me| me.user.id == user),
    }
}
/// An avatar with its presence dot (UI_SPEC colors): online, idle, do not disturb.
fn avatar_status(a: &App, u: &UserSummary) -> AnyView {
    let dot = match a.model.presences.get(&u.id) {
        _ if super::privacy::ignored(u.id) => "",
        Some(PresenceStatus::Idle) => "bg-idle",
        Some(PresenceStatus::Dnd) => "bg-dnd",
        _ if online(a, u.id) => "bg-online",
        _ => "",
    };
    view!{<span class="relative flex flex-none">{avatar(u,"small")}{if dot.is_empty(){().into_any()}else{view!{<span class=format!("presence-dot absolute -right-0.5 -bottom-0.5 size-3 rounded-full border-2 border-sidebar {dot}") aria-hidden="true"></span>}.into_any()}}</span>}.into_any()
}
/// What stands in front of a direct or group conversation in the list: the
/// other person's avatar with their presence, or the group mark. `None` for
/// server channels (and a person not loaded yet), which keep their glyph.
fn dm_face(a: &App, c: &ChannelSummary) -> Option<AnyView> {
    match c.kind {
        ChannelKind::Direct => {
            let me = a.model.me.as_ref().map(|m| m.user.id);
            c.recipient_ids
                .iter()
                .find(|id| Some(**id) != me)
                .and_then(|id| a.model.users.get(id))
                .map(|u| avatar_status(a, u))
        }
        ChannelKind::GroupDm => Some(view!{<span class="grid size-8 flex-none place-items-center rounded-full bg-muted">{icon_with("users","icon text-muted-foreground")}</span>}.into_any()),
        _ => None,
    }
}
/// Who is in a voice channel, listed under it for everyone in the server.
fn voice_occupants(a: &App, channel: Id, depth: usize) -> AnyView {
    let mut people = BTreeMap::new();
    for v in a
        .model
        .launch
        .voice
        .values()
        .filter(|v| v.channel_id == channel)
    {
        people.insert(v.participant.session.user_id, &v.participant);
    }
    if people.is_empty() {
        return ().into_any();
    }
    view!{<ul class="voice-occupants m-0 mb-1 flex list-none flex-col gap-0.5 p-0 pl-9" style=indent(depth,12)>{people.into_iter().map(|(user,p)|view!{<li class="flex min-h-7 items-center gap-2 pr-2 text-[13px] text-sidebar-foreground"><span class="min-w-0 flex-1 truncate">{a.model.name(user)}</span>{if p.deafened{icon_with("headphone-off","icon size-3.5 text-muted-foreground")}else if p.muted{icon_with("mic-off","icon size-3.5 text-muted-foreground")}else{().into_any()}}</li>}).collect_view()}</ul>}.into_any()
}
fn shell(a: &App) -> AnyView {
    let channel = a.channel.and_then(|id| a.model.channels.get(&id));
    let title = channel.map_or("Friends".into(), |c| a.model.channel_name(c));
    let guild = a.guild.and_then(|g| a.model.guilds.get(&g));
    let guild_name = if a.nav.favorites {
        "Favorites".into()
    } else {
        guild.map_or("Direct Messages".into(), |g| g.guild.name.clone())
    };
    let me = &a.model.me.as_ref().unwrap().user;
    let manage_channels = settings::can(a, Permissions::MANAGE_CHANNELS);
    // Server Settings is for people who can change something in it.
    let admin = settings::can_any(
        a,
        Permissions::MANAGE_GUILD
            | Permissions::MANAGE_CHANNELS
            | Permissions::MANAGE_ROLES
            | Permissions::BAN_MEMBERS,
    );
    let owner = guild.is_some_and(|g| g.my_role == MemberRole::Owner);
    let requests = a
        .model
        .relationships
        .values()
        .filter(|r| r.kind == RelationshipKind::IncomingRequest)
        .count();
    let request_badge = |class: &'static str| {
        if requests == 0 {
            return ().into_any();
        }
        view!{<span class=class aria-label=format!("{requests} friend requests")>{requests.to_string()}</span>}.into_any()
    };
    let in_call = a.screen == "voice";
    let direct = channel.is_some_and(|c| c.guild_id.is_none());
    let topic = channel
        .and_then(|c| a.model.topics.get(&c.id))
        .and_then(|t| t.clone())
        .unwrap_or_default();
    // "Open Chat" returns to the conversation you were in before the call.
    let open_chat = a
        .last_text
        .filter(|c| a.model.channels.contains_key(c))
        .or_else(|| {
            a.model
                .channels
                .values()
                .find(|c| a.guild.is_some() && c.guild_id == a.guild && c.kind == ChannelKind::Text)
                .map(|c| c.id)
        })
        .map_or("home".into(), |c| format!("channel:{}", c.0));
    let call_name = a
        .voice_channel
        .and_then(|c| a.model.channels.get(&c))
        .map(|c| {
            // "<channel> / <server>", as UI_PARITY describes the panel.
            match c.guild_id.and_then(|g| a.model.guilds.get(&g)) {
                Some(g) => format!("{} / {}", a.model.channel_name(c), g.guild.name),
                None => a.model.channel_name(c),
            }
        })
        .unwrap_or_default();
    view!{<div class=format!("app-shell live-shell relative top-(--viewport-offset) flex h-(--viewport-height) bg-background phone:pt-(--safe-top) panel-{} {}",a.panel,if channel.is_some(){"in-channel"}else{""})>
        <div class=format!("left-panel flex w-[312px] flex-none touch-pan-y bg-sidebar xl:w-[344px] 2xl:w-[384px] phone:absolute phone:top-(--safe-top) phone:bottom-0 phone:left-0 phone:z-30 phone:w-[min(340px,90vw)] {}",if a.panel=="left"{"phone:translate-x-0"}else{"phone:-translate-x-full"})><nav class="server-rail flex w-[72px] flex-none touch-pan-y flex-col items-center gap-2 overflow-y-auto overscroll-contain bg-rail p-3" aria-label="Servers"><button class="server-icon brand relative grid size-12 min-h-12 place-items-center rounded-2xl" data-live="home" aria-label="Home"><img class="rounded-2xl" src=themes::logo() data-app-logo="" alt="" width="48" height="48"/>{request_badge("live-unread absolute -right-1 -bottom-1 ml-0 border-2 border-rail")}</button><div class="rail-divider min-h-0.5 w-8 bg-border"></div>
            {navui::rail(a)}
            {ib("plus","Add a Server","dialog:create")}
            {community::discover_button(a)}
        </nav><aside class=format!("channel-sidebar flex min-h-0 min-w-0 flex-1 flex-col{}",identity::sidebar_class(a))><div class="sidebar-header flex h-12 min-h-12 items-center gap-1 border-b px-3 py-2">{if a.guild.is_some()&&!a.nav.favorites{view!{
                <ui-menu class="contents"><ui-menu-trigger><button class="server-heading flex h-8 min-w-0 flex-1 items-center gap-1 rounded-md px-1 hover:bg-sidebar-accent" aria-label=format!("{guild_name} menu") title=guild_name.clone()><strong class="min-w-0 flex-1 truncate text-base font-semibold">{guild_name.clone()}</strong>{icon_with("chevron-down","icon size-4 text-muted-foreground")}</button></ui-menu-trigger>
                <ui-menu-content class="menu" side="bottom" align="start" side-offset="6" hidden=true>
                    <ui-menu-item class="menu-item" value="dialog:invite">{icon("user-plus")}"Invite People"</ui-menu-item>
                    <ui-menu-item class="menu-item" value=a.guild.map_or(String::new(),|g|format!("dialog:target-notifications:guild:{}",g.0))>{icon("bell")}"Notification Settings"</ui-menu-item>
                    <ui-menu-item class="menu-item" value=a.guild.map_or(String::new(),|g|format!("profiles:open-server,{}",g.0))>{icon("user-round")}"Edit Server Profile"</ui-menu-item>
                    {community::menu_items(a)}
                    {if manage_channels{view!{<ui-menu-separator class="menu-separator"></ui-menu-separator><ui-menu-item class="menu-item" value="dialog:channel">{icon("hash")}"Create Channel"</ui-menu-item><ui-menu-item class="menu-item" value="dialog:category">{icon("folder")}"Create Category"</ui-menu-item>}.into_any()}else{().into_any()}}
                    {if admin{view!{<ui-menu-separator class="menu-separator"></ui-menu-separator><ui-menu-item class="menu-item" value="dialog:server">{icon("settings")}"Server Settings"</ui-menu-item>}.into_any()}else{().into_any()}}
                    {if owner{().into_any()}else{view!{<ui-menu-separator class="menu-separator"></ui-menu-separator><ui-menu-item class="menu-item danger-text" value="leave-guild">{icon("log-out")}"Leave Server"</ui-menu-item>}.into_any()}}
                </ui-menu-content></ui-menu>
                {if manage_channels{ib("plus","Create Channel","dialog:channel")}else{().into_any()}}{if admin{ib("settings","Server Settings","dialog:server")}else{().into_any()}}}.into_any()}else{view!{<strong class="min-w-0 flex-1 truncate px-1 text-base font-semibold">{guild_name}</strong>{if a.nav.favorites{ib("settings","Manage Favorites","nav-menu:favorites-manage")}else{ib("users","New Group Message","dialog:group")}}}.into_any()}}</div>
            <nav class="channel-scroll min-h-0 flex-1 touch-pan-y overflow-y-auto overscroll-contain p-2" aria-label="Channels">
            {identity::sidebar_banner(a)}
            {navui::sidebar_top(a)}
            {community::sidebar_rows(a)}
            {stage::events_entry(a)}
            {if a.nav.favorites{().into_any()}else{view!{<button class="nav-row flex w-full items-center justify-start gap-2 rounded-md p-2 text-sm font-medium text-sidebar-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground touch:min-h-11 phone:text-base" data-live="home">{icon_with("users","icon text-muted-foreground")}"Friends"{request_badge("live-unread")}</button>{privacy::requests_row(a)}}.into_any()}}
            {community::shown(a,channel_tree(a)).into_iter().filter(|(c,_)|!a.nav.favorites&&!super::privacy::hidden(a,c)).map(|(c,depth)|{
                let label=a.model.channel_name(c);
                if c.kind==ChannelKind::Category{return view!{<div class=format!("category-row flex min-h-7 items-center gap-1 pr-1 pl-2 {}",if depth>0{"nested mt-2"}else{"mt-4"}) style=indent(depth,12) data-depth=depth.to_string()><h2 class="category min-w-0 flex-1 truncate" title=label.clone()>{label.clone()}</h2>{if manage_channels{create_inside(c,&label,"grid size-6 flex-none place-items-center rounded-sm text-muted-foreground hover:bg-sidebar-accent hover:text-foreground touch:size-11")}else{().into_any()}}</div>}.into_any()}
                let selected=a.channel==Some(c.id);
                let read=a.model.reads.get(&c.id);
                // Muted: no unread dot (mentions still count).
                let muted=super::privacy::muted(a,c);
                let unread=read.is_some_and(|r|r.unread_count>0)&&!muted;
                // A direct message is always for you; in a server only mentions are counted.
                let count=read.map_or(0,|r|if c.guild_id.is_none()&&!muted{r.unread_count}else{r.mention_count});
                let voice=c.kind==ChannelKind::Voice;
                // A conversation is shown by who it is with: their picture and presence,
                // or the group mark at the same size so the names line up.
                let face=dm_face(a,c);
                let pad=if face.is_some(){"px-2 py-1.5"}else{"p-2"};
                let open=if c.kind==ChannelKind::Stage{format!("stage:open,{}",c.id.0)}else{format!("{}:{}",if voice{"voice"}else{"channel"},c.id.0)};
                view!{<div class=format!("channel-row group flex min-h-[34px] items-center rounded-md {} {}",if selected{"selected bg-secondary"}else{"hover:bg-sidebar-accent"},if depth>0{"nested"}else{""}) style=indent(depth,12) data-depth=depth.to_string()>{profiles::dm_nameplate(a,c)}<button class=format!("channel-main flex min-w-0 flex-1 items-center gap-2 text-sm font-medium touch:min-h-11 phone:text-base {pad} {}",if selected||unread{"text-foreground"}else{"text-sidebar-foreground"}) title=label.clone() data-live=open>{match face{Some(face)=>face,None=>icon_with(if voice{"volume-2"}else if let Some(glyph)=stage::kind_icon(c.kind){glyph}else if let Some(glyph)=threads::kind_icon(c.kind){glyph}else if c.guild_id.is_none(){"message-circle"}else{"hash"},"icon text-muted-foreground")}}<span class="truncate">{label.clone()}</span>{identity::dm_tag(a,c)}{if count>0{view!{<span class="live-unread">{count.to_string()}</span>}.into_any()}else if unread{view!{<span class="live-unread quiet" aria-label="Unread messages"></span>}.into_any()}else{().into_any()}}</button>{call_views::row_chat(a,c)}{stage::row_chat(a,c)}{if manage_channels&&c.guild_id.is_some(){create_inside(c,&label,if selected{"channel-add mr-1 grid size-6 flex-none touch:size-11 place-items-center rounded-sm text-muted-foreground hover:text-foreground"}else{"channel-add mr-1 hidden size-6 flex-none touch:size-11 place-items-center rounded-sm text-muted-foreground hover:text-foreground group-focus-within:grid group-hover:grid"})}else{().into_any()}}{navui::row_options(c)}</div>{stage::row_extra(a,c,depth)}{if voice||c.kind==ChannelKind::Stage{voice_occupants(a,c.id,depth)}else{().into_any()}}{threads::rows(a,c,depth)}}.into_any()
            }).collect_view()}
            </nav>
            <section id="live-connected" class="voice-connected border-t p-3 text-sm" hidden><div class="voice-connection-line flex min-w-0 flex-col"><strong class="text-primary">"Voice Connected"</strong><small class="truncate" title=call_name.clone()>{call_name.clone()}</small></div><div class="voice-tiles mt-2 flex gap-2 [&_.icon-button]:flex-1 [&_.icon-button]:bg-accent">{ib("video","Camera","media:camera")}{ib("monitor","Share Screen","media:share")}{ib("phone-off","Disconnect","media:leave")}</div></section>
            <section class="user-panel m-2 flex min-h-[52px] items-center gap-1 rounded-lg bg-panel p-2 touch:min-h-[60px] phone:h-auto phone:pb-[max(8px,var(--safe-bottom))]">{privacy::status_trigger(a,me)}<button class="self-label ml-1 min-w-0 flex-1 touch:min-h-11" title=name(me) data-live=format!("profile:{}",me.id.0)><strong class="block truncate text-sm leading-tight font-semibold">{name(me)}</strong><small class="block truncate text-xs">{format!("@{}",me.username)}</small></button>{ib("mic","Mute","media:mute")}{ib("headphones","Deafen","media:deafen")}{ib("settings","User Settings","dialog:settings")}</section>
        </aside></div><div class=if a.panel.is_empty(){"panel-scrim hidden"}else{"panel-scrim hidden touch-pan-y phone:absolute phone:inset-0 phone:z-[25] phone:block phone:bg-black/60"} data-live="panel:" aria-hidden="true"></div>
        <main class=format!("main-pane flex min-w-0 flex-1 touch-pan-y flex-col {}",if channel.is_none(){"phone:pb-[calc(56px+var(--safe-bottom))]"}else{""})><header class="channel-header flex h-12 min-h-12 flex-none items-center gap-2 border-b px-4 py-2 phone:h-(--header-height) phone:min-h-(--header-height) phone:px-2"><div class="mobile-menu hidden phone:block">{ib("menu","Open Channels","panel:left")}</div>{icon_with(if in_call{"volume-2"}else if direct{"at-sign"}else if let Some(glyph)=channel.and_then(|c|threads::kind_icon(c.kind)){glyph}else if channel.is_some(){"hash"}else{"users"},"icon text-muted-foreground")}<div class="flex min-w-0 flex-1 items-baseline gap-3">{if in_call{None}else{threads::title(a)}.unwrap_or_else(||view!{<h1 class="min-w-0 truncate text-base font-semibold" title=title.clone()>{title.clone()}</h1>}.into_any())}{if topic.is_empty()||in_call{().into_any()}else{view!{<p class="channel-topic m-0 min-w-0 flex-1 truncate text-[13px] text-subtle phone:hidden" title=topic.clone()>{topic.clone()}</p>}.into_any()}}</div><div class="header-actions flex gap-2 phone:gap-0">{if in_call{ib("message-circle","Open Chat",&open_chat)}else if channel.is_some(){view!{{a.channel.map_or_else(||().into_any(),|c|call_views::header(a,c))}{a.channel.map_or_else(||().into_any(),|c|threads::header(a,c))}<div class="contents phone:hidden">{if a.channel.is_some_and(|c|a.threads.threads.contains_key(&c)){().into_any()}else{ib("bell","Notification Settings","dialog:channel-notifications")}}{ib("pin","Pinned Messages","dialog:pins")}{ib("users","Member List","panel:right")}</div>}.into_any()}else{().into_any()}}{if in_call{().into_any()}else{navui::header(a)}}</div></header>
            {if !a.status.is_empty(){view!{<div class="connection-banner bg-warning px-4 py-2 text-sm text-rail" role="status">{a.status.clone()}</div>}.into_any()}else{().into_any()}}
            {if in_call{call_views::beside(a,a.voice_render.map_or_else(loading_call, |render|render(a)))}else if channel.is_some_and(|c|c.kind==ChannelKind::Forum){lazy_view(a,|a|&a.views.forum,||Box::pin(load_forum())).map_or_else(loading_call,|forum|forum(a,&title))}else if channel.is_some(){chat(a,&title)}else{home(a)}}
        </main>
        {if a.nav.search_open{lazy_view(a,|a|&a.views.search,||Box::pin(load_search())).map_or_else(||().into_any(),|search|search(a))}else{().into_any()}}
        {if channel.is_some()&&!a.nav.search_open{view!{<aside class=format!("member-list w-60 flex-none touch-pan-y overflow-auto overscroll-contain bg-sidebar {} {}",if a.panel=="right"{"is-open narrow:absolute narrow:top-12 narrow:right-0 narrow:bottom-0 narrow:z-20 narrow:shadow-[-8px_0_24px_#0006] phone:top-[calc(var(--header-height)+var(--safe-top))] phone:z-30 phone:w-[min(300px,90vw)] phone:pb-(--safe-bottom)"}else{"narrow:hidden"},if a.members_hidden{"min-[1101px]:hidden"}else{""}) aria-label="Members">{member_groups(a)}{if a.guild.is_some()&&a.members_more{button("Load More","members-more","link-button")}else{().into_any()}}</aside>}.into_any()}else{().into_any()}}
        {if channel.is_none(){view!{<nav class="bottom-tabs hidden phone:absolute phone:inset-x-0 phone:bottom-0 phone:flex phone:h-[calc(56px+var(--safe-bottom))] phone:justify-around phone:border-t phone:bg-sidebar phone:pb-(--safe-bottom)" aria-label="Main navigation">{button("Home","home","active text-primary")}{button("Notifications","nav-inbox","text-subtle")}{button("You","dialog:settings","text-subtle")}</nav>}.into_any()}else{().into_any()}}
        {navui::overlay(a)}
    </div>}.into_any()
}
/// Friends, in the order they need you: requests waiting for an answer, then
/// friends (online first), then requests you sent, then people you blocked.
fn home(a: &App) -> AnyView {
    let order = |kind: RelationshipKind| match kind {
        RelationshipKind::IncomingRequest => 0,
        RelationshipKind::Friend => 1,
        RelationshipKind::OutgoingRequest => 2,
        RelationshipKind::Blocked => 3,
    };
    let mut people = a.model.relationships.values().collect::<Vec<_>>();
    people.sort_by_key(|r| {
        (
            order(r.kind),
            !online(a, r.user.id),
            name(&r.user).to_lowercase(),
        )
    });
    people.retain(|r| match a.friends_tab.as_str() {
        "online" => r.kind == RelationshipKind::Friend && online(a, r.user.id),
        "pending" => matches!(
            r.kind,
            RelationshipKind::IncomingRequest | RelationshipKind::OutgoingRequest
        ),
        "blocked" => r.kind == RelationshipKind::Blocked,
        _ => true,
    });
    let nobody = people.is_empty() && !a.model.relationships.is_empty();
    let count = |kind: RelationshipKind| people.iter().filter(|r| r.kind == kind).count();
    let mut section = None;
    let rows = people.iter().map(|r|{
        let heading = (section != Some(r.kind)).then(|| {
            let label = match r.kind {
                RelationshipKind::IncomingRequest => "Friend requests",
                RelationshipKind::Friend => "Friends",
                RelationshipKind::OutgoingRequest => "Requests you sent",
                RelationshipKind::Blocked => "Blocked",
            };
            view!{<h3 class="category mt-6 mb-2">{format!("{label} — {}",count(r.kind))}</h3>}
        });
        section = Some(r.kind);
        let state = match r.kind {
            RelationshipKind::Friend => "Friend",
            RelationshipKind::IncomingRequest => "Incoming request",
            RelationshipKind::OutgoingRequest => "Outgoing request",
            RelationshipKind::Blocked => "Blocked",
        };
        let status = privacy::status_line(&r.user)
            .filter(|_| r.kind == RelationshipKind::Friend && !super::privacy::ignored(r.user.id));
        view!{{heading}<div class="friend-row flex items-center gap-3 border-t py-4 phone:flex-wrap">{avatar_status(a,&r.user)}<button class="friend-info flex min-w-0 flex-1 flex-col gap-1 touch:min-h-11 phone:min-w-[140px] phone:text-base" data-live=format!("profile:{}",r.user.id.0)><span class="flex max-w-full min-w-0 items-center gap-1"><strong class="truncate font-semibold">{name(&r.user)}</strong>{identity::tag_static(a,&r.user)}</span><small class="max-w-full truncate">{format!("@{} · {}",r.user.username,status.as_deref().unwrap_or(state))}</small></button>
        {match r.kind{RelationshipKind::IncomingRequest=>view!{{button("Accept",&format!("friend:accept,{}",r.user.id.0),"primary")}{button("Decline",&format!("friend:decline,{}",r.user.id.0),"secondary")}}.into_any(),RelationshipKind::OutgoingRequest=>button("Cancel",&format!("friend:cancel,{}",r.user.id.0),"secondary"),RelationshipKind::Blocked=>button("Unblock",&format!("friend:unblock,{}",r.user.id.0),"secondary"),_=>ib("message-circle","Message",&format!("friend:dm,{}",r.user.id.0))}}
        </div>}
    }).collect_view();
    view!{<div class="live-home touch-pan-y overflow-auto overscroll-contain p-6 phone:p-4"><div class="friends-toolbar mb-2 flex flex-wrap items-center gap-3 phone:gap-2"><h2 class="mr-auto">"Friends"</h2>{button("Add Friend","dialog:friend","primary")}{ib("users","New Group Message","dialog:group")}{button("Join a Server","dialog:join","secondary")}</div>
        <div class="friends-tabs mb-2 flex flex-wrap gap-2" role="group" aria-label="Show">{[("all","All"),("online","Online"),("pending","Pending"),("blocked","Blocked")].into_iter().map(|(tab,label)|view!{<button class=format!("button {}",if a.friends_tab==tab{"secondary"}else{"link-button"}) aria-pressed=(a.friends_tab==tab).to_string() data-live=format!("friends:{tab}")>{label}</button>}).collect_view()}</div>
        {if nobody{view!{<p class="muted-copy mt-4">"Nobody to show here."</p>}.into_any()}else{().into_any()}}
        {if a.model.relationships.is_empty(){view!{<p class="muted-copy mt-4">"No friends yet. Add a friend to start a conversation."</p>}.into_any()}else{().into_any()}}
        {rows}
    </div>}.into_any()
}
/// The local calendar day of a timestamp.
fn day(ms: u64) -> (u32, u32, u32) {
    let d = js_sys::Date::new(&JsValue::from_f64(ms as f64));
    (d.get_full_year(), d.get_month(), d.get_date())
}
fn day_label(ms: u64) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let (year, month, date) = day(ms);
    format!("{} {date}, {year}", MONTHS[month as usize % 12])
}
/// One line of a message, for a reply preview: names for mentions, spoilers hidden.
fn preview(a: &App, m: &Message) -> String {
    let text = crate::wire::snippet(&expression::plain(&a.model.content(m)));
    if !text.is_empty() {
        text
    } else if let Some(s) = m.extra.stickers().first() {
        format!("Sticker: {}", s.name)
    } else if m.attachments.iter().any(|f| f.meta.voice()) {
        "Voice message".into()
    } else if !m.attachments.is_empty() {
        "Attachment".into()
    } else if let Some(f) = &m.extra.forwarded {
        let inner = crate::wire::snippet(&f.content);
        if inner.is_empty() {
            "Forwarded message".into()
        } else {
            format!("Forwarded: {inner}")
        }
    } else {
        text
    }
}
fn chat(a: &App, title: &str) -> AnyView {
    let c = a.channel.unwrap();
    if let Some(gate) = moderation::age_gate(a, c) {
        return view! {<div class="chat-body flex min-h-0 flex-1 flex-col">{moderation::raid_banner(a)}{gate}</div>}.into_any();
    }
    let kind = a
        .model
        .channels
        .get(&c)
        .map_or(ChannelKind::Text, |c| c.kind);
    let messages = a.model.messages.get(&c);
    let mut shown: Vec<_> = a
        .window_ids
        .iter()
        .filter_map(|id| messages.and_then(|m| m.get(id)))
        .collect();
    let count = a
        .extent
        .as_ref()
        .map_or(shown.len() as u64, |e| e.message_count);
    if shown.len() > 50 {
        shown.drain(..shown.len() - 50);
    }
    let bottom = count.saturating_sub(a.window_rank + shown.len() as u64);
    let me = a.model.me.as_ref().map(|me| me.user.id);
    // A direct message is a person, a group is a group; only channels get a "#".
    let (intro_icon, intro_title, placeholder) = match kind {
        ChannelKind::Direct => ("at-sign", title.to_owned(), format!("Message @{title}")),
        ChannelKind::GroupDm => ("users", title.to_owned(), format!("Message {title}")),
        ChannelKind::Thread => (
            "message-square-text",
            title.to_owned(),
            format!("Message {title}"),
        ),
        _ => (
            "hash",
            format!("Welcome to #{title}!"),
            format!("Message #{title}"),
        ),
    };
    let invite = a.guild.is_some() && settings::can(a, Permissions::CREATE_INVITES);
    view!{<div class="chat-body flex min-h-0 flex-1 flex-col">{call_views::banner(a,c)}{moderation::raid_banner(a)}<div id="live-messages" class="message-list min-h-0 flex-1 touch-pan-y overflow-auto overscroll-contain pb-4 [overflow-anchor:none]" role="log" aria-label="Messages" tabindex="0" data-channel=c.0.to_string() data-history-generation=a.history_generation.to_string() data-window-start=a.window_rank.to_string() data-window-end=(a.window_rank+shown.len()as u64).to_string() data-total=count.to_string()>
        <div style=format!("height:{}px",a.window_rank*88) aria-hidden="true"></div>
        {if shown.is_empty(){view!{<div class="conversation-intro px-4 py-8"><span class="intro-icon inline-flex rounded-full bg-muted p-4">{icon(intro_icon)}</span><h2 class="mt-4 mb-2">{intro_title}</h2><p class="text-subtle">{if a.history_busy{"Loading messages…"}else{"This is the beginning of this conversation."}}</p>{if invite&&!a.history_busy{view!{<div class="mt-4">{button("Invite People","dialog:invite","primary")}</div>}.into_any()}else{().into_any()}}</div>}.into_any()}else{().into_any()}}
        {shown.iter().enumerate().map(|(i,m)|{
            let previous=i.checked_sub(1).map(|p|shown[p]);
            let new_day=previous.is_none_or(|p|day(p.created_at)!=day(m.created_at));
            let unread=Some(m.author_id)!=me&&a.new_after.is_some_and(|read|m.id>read&&previous.is_none_or(|p|p.id<=read));
            view!{{if new_day{view!{<div class="day-divider" role="separator"><span>{day_label(m.created_at)}</span></div>}.into_any()}else{().into_any()}}{if unread{view!{<div class="new-divider" role="separator" aria-label="New messages"><span>"New"</span></div>}.into_any()}else{().into_any()}}{message(a,m,!new_day&&!unread&&crate::wire::continues_group(m.author_id.0,m.created_at,previous.map(|p|(p.author_id.0,p.created_at))))}}
        }).collect_view()}
        <div style=format!("height:{}px",bottom*88) aria-hidden="true"></div>
        {if bottom==0{apps::ephemeral_list(a,c)}else{().into_any()}}
        </div><div class="composer-wrap @container/composer relative px-4 pb-(--safe-bottom) phone:px-2">
        {if let Some(reply)=a.reply{let target=messages.and_then(|m|m.get(&reply));view!{<div class="reply-preview flex items-center gap-2 py-1 text-[13px] text-subtle touch:min-h-11"><span class="min-w-0 flex-1 truncate">{format!("Replying to {}",target.map_or("message".into(),|m|a.model.name(m.author_id)))}{target.map(|m|view!{<span class="text-muted-foreground">{format!(" · {}",preview(a,m))}</span>})}</span>{ib_plain("x","Cancel Reply","cancel-reply")}</div>}.into_any()}else{().into_any()}}
        {content::scheduled_bar(a,c)}
        {threads::composer_note(a,c)}
        {a.attachments.iter().enumerate().map(|(i,f)|content::attachment_chip(a,i,f)).collect_view()}
        {expression::suggestions(a)}
        {if a.uploading{view!{<div class="live-upload mb-2 flex items-center justify-between gap-4 rounded-lg bg-muted p-2 text-xs" role="status">{a.upload_progress.clone()}{button("Pause","upload-cancel","secondary")}</div>}.into_any()}else{().into_any()}}
        {apps::picker_view(a)}
        {if a.completions.is_empty(){().into_any()}else{view!{<div id="live-completions" class="autocomplete absolute inset-x-4 bottom-full z-40 max-h-[min(320px,50vh)] overflow-y-auto rounded-lg border bg-popover p-2 shadow-lg" role="listbox" aria-label="Suggestions">{a.completions.iter().enumerate().map(|(i,(label,insert,kind))|view!{<button class="flex h-9 w-full items-center gap-2 rounded-sm px-2 text-sm aria-selected:bg-secondary touch:min-h-11" id=format!("completion-{i}") role="option" aria-selected=if i==0{"true"}else{"false"} data-completion=insert.clone()><span class="completion-avatar grid size-6 flex-none place-items-center overflow-hidden rounded-full bg-sidebar text-xs [&_.icon]:size-4">{match kind.as_str() {"Role"=>icon("shield"),"Channel"=>icon("hash"),"Emoji"=>view!{<span>{insert.clone()}</span>}.into_any(),"Custom"=>expression::completion_icon(a,insert),_=>view!{<span>{label.chars().take(1).collect::<String>()}</span>}.into_any()}}</span><span>{label.clone()}</span><small class="ml-auto">{if kind=="User"{insert.clone()}else{kind.clone()}}</small></button>}).collect_view()}</div>}.into_any()}}
        {moderation::composer_notice(a,c)}
        {community::composer_notice(a,c)}
        <div class="composer flex items-end gap-2 rounded-lg bg-muted px-3 py-2 phone:gap-1 phone:px-2 phone:py-1 @max-[24rem]/composer:gap-1 @max-[24rem]/composer:px-2"><button type="button" class="icon-button" title="Add Attachment" aria-label="Add Attachment" data-live="pick-file">{icon("plus-circle")}</button><input type="file" id="live-file" multiple hidden/><label class="composer-input flex min-w-0 flex-1"><span class="sr-only">"Message"</span><textarea class="max-h-[40vh] min-h-8 flex-1 resize-none rounded-none border-0 bg-transparent px-0 py-1 text-[15px] leading-6 shadow-none focus-visible:ring-0 touch:min-h-11 touch:py-2.5 phone:text-base" id="live-input" rows="1" placeholder=placeholder aria-label="Message" role="combobox" aria-autocomplete="list" aria-controls="live-completions" aria-expanded=(!a.completions.is_empty()).to_string()></textarea></label><span id="live-count" class="composer-count" hidden></span><button class="composer-gif inline-flex h-8 flex-none items-center justify-center rounded-md px-2 text-sm font-semibold text-primary hover:bg-accent touch:min-h-11 touch:min-w-11 @max-[32rem]/composer:hidden" data-live="dialog:gif">"GIF"</button>{ib_plain("smile","Emoji Picker","dialog:emoji")}<span class="contents @max-[32rem]/composer:hidden">{expression::composer_button()}</span><button class="composer-kaomoji inline-flex h-8 flex-none items-center justify-center rounded-md px-2 text-sm font-semibold text-primary hover:bg-accent @max-[32rem]/composer:hidden" aria-label="Kaomoji" title="Kaomoji" data-live="dialog:kaomoji">"(^▽^)"</button>{content::composer_tools(a)}<button class="icon-button composer-send" aria-label="Send Message" title="Send Message" data-live="send-input" disabled={a.uploading||a.preparing_send||!a.ready}>{icon("send")}</button></div>
        <div class="typing-indicator h-6 text-xs leading-6 text-subtle">{expression::typing(a,c)}</div>
        </div></div>}.into_any()
}
/// Plain message text, with each "#channel" of this server made a link to it.
fn text_links(a: &App, text: &str) -> AnyView {
    if a.guild.is_none() || !text.contains('#') {
        return view! {<span>{text.to_owned()}</span>}.into_any();
    }
    let mut parts = vec![];
    let mut plain = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('#') {
        let tail = &rest[at + 1..];
        let len = tail
            .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(tail.len());
        plain.push_str(&rest[..at]);
        let starts_word = plain
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let channel = a.model.channels.values().find(|c| {
            starts_word
                && c.guild_id == a.guild
                && c.kind == ChannelKind::Text
                && c.name.as_deref() == Some(&tail[..len])
        });
        if let Some(c) = channel.filter(|_| len > 0) {
            parts.push(view! {<span>{std::mem::take(&mut plain)}</span>}.into_any());
            parts.push(view!{<button class="mention" data-live=format!("channel:{}",c.id.0)>{format!("#{}",&tail[..len])}</button>}.into_any());
        } else {
            plain.push('#');
            plain.push_str(&tail[..len]);
        }
        rest = &tail[len..];
    }
    plain.push_str(rest);
    parts.push(view! {<span>{plain}</span>}.into_any());
    parts.collect_view().into_any()
}
/// An invite link (`?invite=CODE` or `/invite/CODE`, on any server).
fn invite_url(url: &str) -> bool {
    url.contains("invite=") || url.contains("/invite/")
}
/// An invite code or link as text. Every invite the app shows goes through
/// this or [`message_link`], so Streamer Mode (`.streamer-invite`,
/// themes.css) hides all of them, whatever their form.
pub(super) fn invite_text(text: &str, class: &'static str) -> AnyView {
    view! {<code class=format!("streamer-invite {class}") data-invite-code="">{text.to_owned()}</code>}
        .into_any()
}
/// A link in a message; an invite link is marked for Streamer Mode.
/// A link in a message. `masked`: its text is not its address. The host and
/// the mark are what the link checks (public/links.js or its successor) read.
fn message_link(url: &str, text: &str, masked: bool) -> AnyView {
    let host = sticktalk_client_core::urls::parse(url)
        .map(|u| u.host)
        .unwrap_or_default();
    view!{<a class=if invite_url(url){"streamer-invite"}else{""} href=url.to_owned() target="_blank" rel="noopener noreferrer" title=host.clone() data-link-host=host data-masked=masked.then_some("")>{text.to_owned()}</a>}.into_any()
}
fn inline(a: &App, v: &Inline, m: &Message) -> AnyView {
    let style = format!(
        "{}{}{}{}",
        if v.style.bold { "font-weight:600;" } else { "" },
        if v.style.italic {
            "font-style:italic;"
        } else {
            ""
        },
        if v.style.strike {
            "text-decoration:line-through;"
        } else if v.style.underline {
            "text-decoration:underline;"
        } else {
            ""
        },
        if v.style.code {
            "font-family:JetBrains Mono,monospace;background:var(--bg-rail);"
        } else {
            ""
        }
    );
    let body=match &v.target{Some(Target::Mention(u))if m.mention_ids.contains(u)=>view!{<button class="mention" data-live=format!("profile:{}",u.0)>{format!("@{}",a.model.name(*u))}</button>}.into_any(),Some(target@(Target::Auto(url)|Target::Masked(url)))if url.starts_with("https://")||url.starts_with("http://")=>message_link(url,&v.text,matches!(target,Target::Masked(_))),_ if v.style.code=>view!{<span>{v.text.clone()}</span>}.into_any(),_=>expression::text(a,&v.text)};
    if v.spoiler.is_some() {
        view!{<button class="live-spoiler" data-spoiler aria-label="Reveal spoiler">{body}</button>}.into_any()
    } else {
        view! {<span style=style>{body}</span>}.into_any()
    }
}
/// Message text as markdown: escaped DOM text, never HTML.
fn markdown(a: &App, m: &Message, text: &str) -> AnyView {
    let document = sticktalk_client_core::markdown::parse(text);
    let line = |v: &Vec<Inline>| v.iter().map(|v| inline(a, v, m)).collect_view();
    view!{<div class=if crate::live::expression::jumbo(text){"markdown jumbo-emoji"}else{"markdown"}>{document.blocks.iter().map(|b|match b{
            Block::Code{text,..}=>view!{<pre><code>{text.clone()}</code></pre>}.into_any(),
            Block::Header(level,v)=>view!{<p class=match *level{1=>"md-h1",2=>"md-h2",_=>"md-h3"}>{line(v)}</p>}.into_any(),
            Block::Quote(v)=>view!{<blockquote>{line(v)}</blockquote>}.into_any(),
            Block::List(v)=>view!{<p class="md-item">{line(v)}</p>}.into_any(),
            Block::Paragraph(v) if v.is_empty()=>view!{<p class="md-blank"></p>}.into_any(),
            Block::Paragraph(v)=>view!{<p>{line(v)}</p>}.into_any()}).collect_view()}</div>}.into_any()
}
fn message(a: &App, m: &Message, continuation: bool) -> AnyView {
    if let Some(record) = call_views::record(a, m) {
        return record;
    }
    if let Some(hidden) = privacy::ignored_message(a, m) {
        return hidden;
    }
    let mut author = profiles::effective(a, m.author_id);
    // A crossposted announcement is shown as from its source server.
    if let Some(x) = m.extra.crosspost() {
        author.display_name = Some(x.source_guild_name.clone());
    }
    apps::author(m, &mut author);
    let own = a
        .model
        .me
        .as_ref()
        .is_some_and(|me| me.user.id == m.author_id);
    let mention = a
        .model
        .me
        .as_ref()
        .is_some_and(|me| m.mention_ids.contains(&me.user.id));
    // In a server, pinning and deleting other people's messages need Manage Messages.
    let moderator = a.guild.is_some() && settings::can(a, Permissions::MANAGE_MESSAGES);
    let rich = m.extra.poll.is_some()
        || m.extra.forwarded.is_some()
        || m.extra.gif.is_some()
        || m.extra.activity.is_some()
        || m.attachments.iter().any(|f| f.meta.voice());
    let body = if m.extra.activity.is_some() {
        activities::card(a, m)
    } else if m.extra.poll.is_some() {
        content::poll_card(a, m)
    } else if let Some(x) = m.extra.crosspost() {
        threads::crosspost(a, m, x)
    } else if let Some(f) = &m.extra.forwarded {
        view! {{if m.content.trim().is_empty(){().into_any()}else{markdown(a,m,&m.content)}}{content::forwarded(a,m,f)}}.into_any()
    } else {
        content::gif(a, m).unwrap_or_else(|| markdown(a, m, &m.content))
    };
    view!{<article class=format!("message group/message relative flex gap-4 px-4 hover:bg-accent {} {}",if continuation{"continuation mt-0.5"}else{"group-head mt-4 in-data-[density=compact]:mt-2"},if mention{"mentioned border-l-2 border-warning bg-warning/8"}else{""}) data-live-message=m.id.0.to_string() tabindex="0" aria-label=format!("Message from {}",name(&author))>
        <div class="avatar-gutter flex w-10 flex-none items-start justify-center self-start">{if continuation{view!{<time class="gutter-time invisible text-xs leading-[21px] whitespace-nowrap group-focus-within/message:visible group-hover/message:visible">{short_time(m.created_at)}</time>}.into_any()}else{view!{<button class="touch:grid touch:min-h-11 touch:min-w-11 touch:place-items-center touch:p-0.5" data-live=format!("profile:{}",m.author_id.0) aria-label=format!("{}'s profile",name(&author))>{apps::avatar_override(m).unwrap_or_else(||avatar(&author,"message-avatar"))}</button>}.into_any()}}</div>
        <div class="message-content min-w-0 flex-1 wrap-anywhere">{if continuation{().into_any()}else{view!{<div class="message-meta flex min-h-[21px] items-baseline gap-2 phone:flex-wrap phone:gap-x-2 phone:gap-y-1"><button class="author role-color max-w-[min(60%,360px)] truncate p-0 font-semibold" style=themes::role_style(&color(a,m.author_id)) title=name(&author) data-live=format!("profile:{}",m.author_id.0)>{identity::role_name(a,m.author_id,&name(&author),profiles::name_for(a,m.author_id,&name(&author)))}</button>{identity::after_name(a,&author)}<time class="text-xs">{timestamp(m.created_at)}</time></div>}.into_any()}}
        {if let Some(reply)=m.reply_to{view!{<button class="reply-preview flex max-w-full items-center gap-2 py-1 text-[13px] text-subtle touch:min-h-11" data-live=format!("seek-message:{}",reply.0)>{icon_with("reply","icon size-4")}<span class="min-w-0 truncate">{a.model.messages.get(&m.channel_id).and_then(|ms|ms.get(&reply)).map_or("Original message".into(),|m|format!("{}: {}",a.model.name(m.author_id),preview(a,m)))}</span></button>}.into_any()}else{().into_any()}}
        {apps::interaction_line(a,m)}{body}
        {if m.edited_at.is_some(){view!{<small class="edited">"(edited)"</small>}.into_any()}else{().into_any()}}{expression::stickers(m)}{content::attachments(a,&m.attachments)}{content::embeds(a,m)}{feature_extras(a,m)}{threads::extras(a,m)}{stage::message_extra(a,m)}
        {expression::reactions(a,m)}</div>
        <div class="message-actions absolute -top-3 right-4 hidden rounded-lg border bg-popover shadow-sm group-focus-within/message:flex group-hover/message:flex touch:hidden!">{ib_plain("smile","Add Reaction",&format!("dialog:emoji-react:{}",m.id.0))}{expression::message_actions(m)}{ib_plain("reply","Reply",&format!("reply:{}",m.id.0))}{ib_plain("forward","Forward",&format!("forward:{}",m.id.0))}{if a.guild.is_none()||moderator{ib_plain("pin","Pin Message",&format!("pin:{}",m.id.0))}else{().into_any()}}{if own&&!rich{ib_plain("pencil","Edit Message",&format!("edit:{}",m.id.0))}else{().into_any()}}{navui::message_actions(m)}{apps::message_actions(a,m)}{threads::message_actions(a,m)}{privacy::message_actions(a,m)}{themes::message_actions(m)}{if own||moderator{ib_plain("trash-2","Delete Message",&format!("delete:{}",m.id.0))}else{().into_any()}}</div>
    </article>}.into_any()
}
/// Under a message's text: an app's embeds and components, invite cards.
fn feature_extras(a: &App, m: &Message) -> AnyView {
    view! {{apps::extra(a,m)}{identity::invite_cards(a,m)}}.into_any()
}
/// A message as its author typed it: mentions are "@username" again.
fn editable(a: &App, m: &Message) -> String {
    let mut text = m.content.clone();
    for id in &m.mention_ids {
        if let Some(u) = a.model.users.get(id) {
            text = text.replace(&format!("<@{}>", id.0), &format!("@{}", u.username));
        }
    }
    text
}
fn attachment(f: &Attachment) -> AnyView {
    let original = format!("/api/v1/files/{}", f.id.0);
    if f.content_type.starts_with("image/") {
        if let Some(b) = crate::wire::image_box(f) {
            let src = if b.original {
                original.clone()
            } else {
                format!("{original}/thumbnail")
            };
            view!{<button class="attachment-image mt-2 block max-w-full overflow-hidden rounded-lg p-0" data-view-image=original data-filename=f.filename.clone() data-bytes=f.byte_size.to_string() aria-label=format!("Open {}",f.filename)><img class="block h-auto max-w-full" src=src.clone() srcset=format!("{src} 1x") loading="lazy" width=b.width height=b.height alt=f.meta.description.clone().unwrap_or_else(||f.filename.clone())/></button>}.into_any()
        } else {
            view!{<button class="button secondary" data-view-image=original data-filename=f.filename.clone()>{format!("View {}",f.filename)}</button>}.into_any()
        }
    } else if crate::wire::video_hint(&f.content_type, &f.filename) {
        view! {<div class="attachment-video mt-2 max-w-full"><video class="block max-h-[400px] max-w-full rounded-lg" controls playsinline preload="none" src=original.clone() aria-label=f.meta.description.clone().unwrap_or_else(||f.filename.clone())></video><a class="inline-flex min-h-11 max-w-full items-center text-sm wrap-anywhere" href=original download=f.filename.clone()>{format!("Download {}",f.filename)}</a></div>}
            .into_any()
    } else {
        view!{<a class="file-card mt-2 flex max-w-[min(100%,420px)] items-center gap-4 rounded-lg bg-muted p-3 text-foreground no-underline" href=original download=f.filename.clone() title=f.filename.clone()>{icon("file")}<span class="min-w-0 flex-1"><span class="block truncate">{f.filename.clone()}</span><small>{crate::wire::file_size(f.byte_size)}</small></span>{icon("download")}</a>}.into_any()
    }
}
fn voice(a: &App) -> AnyView {
    if a.voice_channel
        .is_some_and(|c| crate::live::stage::is_stage(a, c))
    {
        return stage::view(a);
    }
    // Who is in this call: the media host builds its tiles from these names and pictures.
    let mut people = a.voice_channel.map_or(vec![], |c| {
        a.model
            .launch
            .voice
            .values()
            .filter(|v| v.channel_id == c)
            .map(|v| v.participant.session.user_id)
            .collect::<Vec<_>>()
    });
    people.extend(a.model.me.as_ref().map(|me| me.user.id));
    people.extend(call_views::people(a));
    people.sort();
    people.dedup();
    view!{<div class="voice-view flex min-h-0 min-w-0 flex-1 flex-col bg-rail p-4 phone:p-2 phone:pb-[max(8px,var(--safe-bottom))]" data-call-channel=a.voice_channel.map_or(String::new(),|id|id.0.to_string())><ul id="live-call-people" hidden>{people.into_iter().map(|u|view!{<li data-user=u.0.to_string() data-name=a.model.name(u) data-tag=identity::tag_text(a,u) data-avatar=a.model.users.get(&u).and_then(|u|u.avatar_id).map_or(String::new(),|id|format!("/api/v1/files/{}/thumbnail",id.0))></li>}).collect_view()}</ul><button class="call-minimize flex min-h-11 flex-none items-center gap-2 px-2 text-[13px] text-subtle" data-live="media:minimize" aria-label="Minimize call">{icon("chevron-down")}<span>"Minimize call"</span></button>{call_views::ring_list(a)}{activities::banner(a)}{activities::slot(a)}<div id="live-media-stage" class="participant-grid" aria-label="Call participants"></div><div class="call-controls flex flex-none flex-wrap justify-center gap-3 p-3 phone:gap-2 phone:px-0 [&_.icon-button]:size-12 [&_.icon-button]:rounded-full [&_.icon-button]:bg-muted [&_[data-live='media:leave']]:w-20 [&_[data-live='media:leave']]:rounded-[32px] [&_[data-live='media:leave']]:bg-destructive [&_[data-live='media:leave']]:text-primary-foreground">{ib("mic","Mute","media:mute")}{ib("headphones","Deafen","media:deafen")}{ib("video","Camera","media:camera")}<span class="desktop-share contents">{ib("monitor","Share Screen","media:share")}</span><span class="contents phone:hidden">{ib("volume-2","Audio Devices","media:output")}</span>{call_views::controls(a)}{activities::controls(a)}{call_extras::controls(a)}{ib("phone-off","Leave Call","media:leave")}</div><p id="live-call-status" class="call-connection m-0 min-h-4 text-center text-[13px] text-subtle" role="status">"Connecting…"</p></div>}.into_any()
}
fn form(title: &str, kind: &str, fields: AnyView) -> AnyView {
    form_with(title, kind, fields, "", true)
}
/// `back` adds a Cancel button that returns to that page. `enabled` is false
/// for someone who may look at the form but not save it.
fn form_with(title: &str, kind: &str, fields: AnyView, back: &str, enabled: bool) -> AnyView {
    let (label, class) = match kind {
        "delete-account" => ("Delete Account", "danger"),
        "channel-delete" => ("Delete", "danger"),
        "channel-move" => ("Move Channel", "primary"),
        "guild-delete" => ("Delete Server", "danger"),
        "invite-revoke" => ("Revoke Invite", "danger"),
        "unban" => ("Revoke Ban", "primary"),
        "ban" => ("Ban Member", "danger"),
        "role-create" => ("Create Role", "primary"),
        "friend" => ("Send Friend Request", "primary"),
        "group" => ("Create Group", "primary"),
        "group-add" => ("Add to Group", "primary"),
        "password" => ("Change Password", "primary"),
        "about" | "role-save" | "edit" | "channel-save" | "member-role" => {
            ("Save Changes", "primary")
        }
        "create" => ("Create Server", "primary"),
        "channel" => ("Create Channel", "primary"),
        "join" => ("Join Server", "primary"),
        "invite" => ("Create Invite", "primary"),
        _ => ("Save", "primary"),
    };
    view!{<form data-live-form=kind.to_owned()><h2>{title.to_owned()}</h2>{fields}<button type="submit" class=format!("button {class}") disabled=!enabled>{label}</button>{if back.is_empty(){().into_any()}else{view!{<button type="button" class="button secondary mt-2 ml-2 phone:ml-0 phone:w-full" data-live=back.to_owned()>"Cancel"</button>}.into_any()}}</form>}.into_any()
}
/// A password field, with the hint password managers fill it by.
fn secret(id: &str, label: &str, autocomplete: &'static str) -> AnyView {
    view!{<div class="field"><label for=id.to_owned()>{label.to_owned()}</label><input id=id.to_owned() type="password" autocomplete=autocomplete maxlength="128"/></div>}.into_any()
}
/// What you can do from someone's profile depends on who they are to you.
fn profile_actions(a: &App, u: &UserSummary) -> AnyView {
    if a.model.me.as_ref().is_some_and(|me| me.user.id == u.id) {
        return view!{<div class="profile-actions mt-4 flex flex-wrap gap-2">{button("Edit Profile","dialog:profile-edit","secondary")}{authsec::switch_accounts()}</div>}.into_any();
    }
    let id = u.id.0;
    let relation = a.model.relationships.get(&u.id).map(|r| r.kind);
    let blocked = relation == Some(RelationshipKind::Blocked);
    let friendship = match relation {
        Some(RelationshipKind::Friend) => {
            button("Remove Friend", &format!("friend:remove,{id}"), "secondary")
        }
        Some(RelationshipKind::IncomingRequest) => button(
            "Accept Friend Request",
            &format!("friend:accept,{id}"),
            "secondary",
        ),
        Some(RelationshipKind::OutgoingRequest) => button(
            "Cancel Friend Request",
            &format!("friend:cancel,{id}"),
            "secondary",
        ),
        Some(RelationshipKind::Blocked) => ().into_any(),
        None => button(
            "Add Friend",
            &format!("friend-add:{}", u.username),
            "secondary",
        ),
    };
    view!{<div class="profile-actions mt-4 flex flex-wrap gap-2">{if blocked{().into_any()}else{button("Message",&format!("friend:dm,{id}"),"primary")}}{friendship}{if blocked{button("Unblock",&format!("friend:unblock,{id}"),"secondary")}else{button("Block",&format!("friend:block,{id}"),"danger-text")}}</div>{moderation::member_actions(a,u)}{apps::member_actions(a,u)}{privacy::profile_extras(a,u)}}.into_any()
}
/// What a dialog is called for assistive technology.
fn dialog_label(dialog: &str) -> &'static str {
    match dialog.split(':').next().unwrap_or_default() {
        "profile" => "Profile",
        "mod-timeout" => "Time Out Member",
        "mod-kick" => "Kick Member",
        "mod-ban" => "Ban Member",
        "pins" => "Pinned Messages",
        "edit" => "Edit Message",
        "emoji-info" => "Emoji",
        "friend" => "Add Friend",
        "group" => "New Group Message",
        "group-members" => "Group Members",
        "create" => "Create a Server",
        "join" => "Join a Server",
        "channel" | "channel-in" | "category" => "Create Channel",
        value if value.starts_with("nav-") => navui::dialog_label(value),
        value if privacy::label(value).is_some() => privacy::label(value).unwrap_or("Dialog"),
        value if themes::title(value).is_some() => themes::title(value).unwrap_or("Dialog"),
        _ => "Dialog",
    }
}
/// Create Channel. `kind` and `parent` preselect the type and the channel the new
/// one goes inside (the "+" beside a row, or Create Category).
fn channel_form(a: &App, kind: &str, parent: Option<Id>) -> AnyView {
    let title = if kind == "category" {
        "Create Category"
    } else {
        "Create Channel"
    };
    form(title,"channel",view!{{field("f-name","Name","","text")}<label class="field">"Type"<select id="f-extra"><option value="text" selected=kind=="text">"Text"</option><option value="voice" selected=kind=="voice">"Voice"</option><option value="category" selected=kind=="category">"Category"</option><option value="announcement" selected=kind=="announcement">"Announcement"</option><option value="forum" selected=kind=="forum">"Forum"</option><option value="stage" selected=kind=="stage">"Stage"</option></select></label><label class="field">"Inside"<select id="f-value" aria-describedby="channel-inside-help">{inside_options(a,parent,None)}</select><small id="channel-inside-help">"Channels and categories can go inside any other channel or category, as deep as you like."</small></label>}.into_any())
}
const DEVICE_ROW: &str =
    "device-row my-4 flex items-center justify-between gap-4 phone:items-start";
const SETTINGS_EMPTY: &str = "settings-empty flex flex-col items-start gap-4 pt-6 pb-2 [&>.icon]:size-8 [&>.icon]:text-subtle";
fn dialog(a: &App) -> AnyView {
    if let Some((body, label, class)) = content::dialog(a) {
        return modal(a, class, label, body);
    }
    if let Some((body, label, class)) = threads::dialog(a) {
        return modal(a, class, label, body);
    }
    if let Some(view) = feature_dialog(a) {
        return view;
    }
    if let Some(view) = expression::dialog(a) {
        return view;
    }
    if let Some(view) = identity::dialog(a) {
        return view;
    }
    let body=match a.dialog.as_str(){
        "settings"|"account"=>account(a),
        "server"|"server-overview"=>settings::server_overview(a),
        "server-activities"=>activities::settings_page(a),
        value if value.starts_with("nav-")=>navui::dialog(a,value),
        "server-channels"=>settings::channels(a),
        "server-bans"=>settings::bans(a),
        "ban" if !a.model.members.values().any(|m|Some(m.guild_id)==a.guild&&m.role!=MemberRole::Owner&&a.model.me.as_ref().is_some_and(|me|me.user.id!=m.user.id))=>view!{<h2>"Ban a Member"</h2><div class=SETTINGS_EMPTY>{icon("users")}<p>"There is no one else in this server to ban."</p>{button("Back to Bans","dialog:server-bans","secondary")}</div>}.into_any(),
        "ban"=>form_with("Ban a Member","ban",view!{<p>"This removes the member and prevents them from rejoining until you revoke the ban."</p><label class="field"><span>"Member"</span><select id="f-name">{a.model.members.values().filter(|m|m.guild_id==a.guild.unwrap_or_default()&&m.role!=MemberRole::Owner&&a.model.me.as_ref().is_some_and(|me|me.user.id!=m.user.id)).map(|m|view!{<option value=m.user.id.0.to_string()>{name(&m.user)}</option>}).collect_view()}</select></label>{field("f-value","Reason (optional)","","text")}}.into_any(),"dialog:server-bans",true),
        "server-emoji"=>view!{<h2>"Emoji"</h2><p class="muted-copy">"Custom server emoji are not available yet. Everyone can use the emoji picker in chat."</p>}.into_any(),
        "appearance"=>themes_page(a),
        value if themes::title(value).is_some()=>themes_page(a),
        "privacy"=>settings::privacy(a),
        "group-members"=>group_members(a),
        "group" if a.model.relationships.values().filter(|r|r.kind==RelationshipKind::Friend).count()<2=>view!{<h2>"New Group Message"</h2><div class=SETTINGS_EMPTY>{icon("users")}<p>"Add at least two friends to start a group message."</p>{button("Add Friends","dialog:friend","primary")}</div>}.into_any(),
        "group"=>form("New Group Message","group",view!{{field("f-name","Group Name (optional)","","text")}<fieldset class="group-recipients my-4"><legend class="text-[13px] font-medium text-subtle">"Friends"</legend>{a.model.relationships.values().filter(|r|r.kind==RelationshipKind::Friend).map(|r|view!{<label class="live-checkbox">{avatar(&r.user,"small")}<span class="mr-auto">{name(&r.user)}</span><input type="checkbox" data-group-user=r.user.id.0.to_string()/></label>}).collect_view()}</fieldset><input id="f-value" type="hidden"/><p class="muted-copy">"Choose 2 to 15 friends."</p>}.into_any()),
        "friend"=>form("Add Friend","friend",view!{{field("f-name","Username","","text")}<p class="muted-copy">"Enter their username exactly, for example alex_m."</p>}.into_any()),
        "create"=>view!{{form("Create a Server","create",field("f-name","Server Name","","text"))}<div class="-mx-4 mt-2">{button("Join a Server","dialog:join","link-button")}</div>{community::create_extra(a)}}.into_any(),
        "join"=>form("Join a Server","join",view!{{field("f-name","Invite Link or Code","","text")}{match &a.invite_preview{Some((_,Ok(p)))=>view!{<p class="invite-preview" role="status"><strong>{p.name.clone()}</strong>{format!(" · {} member{}{}",p.member_count,if p.member_count==1{""}else{"s"},if p.joined{" · You’re already a member"}else{""})}</p>}.into_any(),Some((_,Err(e)))=>view!{<p class="invite-preview muted-copy" role="status">{e.clone()}</p>}.into_any(),None=>().into_any()}}{identity::join_card(a)}{community::join_note(a)}{stage::invite_event(a)}}.into_any()),
        "channel"=>channel_form(a,"text",None),
        "category"=>channel_form(a,"category",None),
        value if value.starts_with("channel-in:")=>channel_form(a,"text",id(value.trim_start_matches("channel-in:"))),
        "invite"=>settings::invites(a),
        "profile-edit"=>profiles::editor(a),
        "about"=>form("About Me","about",view!{{field("f-name","Pronouns",a.profile.as_ref().map_or("",|p|p.profile.pronouns.as_str()),"text")}<p class="muted-copy">"Optional. Let people know how to refer to you."</p><label class="field"><span>"About Me"</span><textarea id="f-value" rows="4" maxlength="190" aria-describedby="bio-help">{a.profile.as_ref().map_or("",|p|p.profile.about_me.as_str()).to_owned()}</textarea><small id="bio-help">"A little about you, up to 190 characters."</small></label>{field("f-extra","Banner Color",&format!("#{:06x}",a.profile.as_ref().map_or(0x496c63,|p|if p.profile.banner_color==0{0x496c63}else{p.profile.banner_color})),"color")}<p class="muted-copy">"Choose the color at the top of your profile."</p>{saved_note(a,"about")}}.into_any()),
        "password"=>view!{{form("Password & Security","password",view!{{secret("f-name",if a.google{"Current Password (or verify with Google below)"}else{"Current Password"},"current-password")}{secret("f-value","New Password","new-password")}{secret("f-extra","Confirm New Password","new-password")}{if a.google{button("Verify with Google","reauth-google:password","secondary")}else{().into_any()}}{if a.google{view!{<p class="muted-copy">"Google proof is valid for one password change within 5 minutes. Re-enter the new password after returning."</p>}.into_any()}else{().into_any()}}}.into_any())}{authsec::security_root()}}.into_any(),
        "data-export"=>authsec::data_page(),
        "delete-account"=>form_with("Delete Account Permanently","delete-account",view!{<p>"This erases your messages and attachments and signs out all devices. Encrypted backups expire after 14 days. Your username is held for 30 days."</p>{field("f-name",if a.google{"Current Password (or verify with Google)"}else{"Current Password"},"","password")}{field("f-value","Type DELETE to confirm","","text")}{if a.google{button("Verify with Google","reauth-google:delete_account","secondary")}else{().into_any()}}}.into_any(),"dialog:account",true),
        "devices"=>view!{<h2>"Devices"</h2><p class="muted-copy">"Where your account is signed in. Log out any device you don’t recognize."</p>{a.devices.iter().map(|d|{let known=d.platform!="Unknown";view!{<div class=DEVICE_ROW><span class="min-w-0 wrap-anywhere"><strong class="font-semibold">{if known{d.device_name.clone()}else{"Unnamed device".to_owned()}}</strong><small>{format!("{}{}{}",if known{d.platform.as_str()}else{""},if known{" · "}else{""},if d.current{"Current Device".to_owned()}else if d.last_active==0{"Not used yet".to_owned()}else{format!("Last active {}",timestamp(d.last_active))})}</small></span>{if d.current{().into_any()}else{button("Log Out",&format!("device-revoke:{}",d.id),"secondary")}}</div>}}).collect_view()}{button("Log Out All Other Devices","logout-others","danger-text")}}.into_any(),
        "media-settings"=>view!{<h2>"Voice & Video"</h2><div id="live-device-settings"></div>}.into_any(),
        "notifications"|"channel-notifications"=>settings::notifications(a),
        value if value.starts_with("target-notifications:")=>settings::notifications(a),
        "roles"=>settings::roles(a),
        "member-role"=>settings::member_roles(a),
        "private"=>form("Private Channel","private",view!{<p>"Restrict this channel to one role. The owner retains access."</p><label class="field">"Allowed Role"<select id="f-name">{a.roles.values().filter(|r|Some(r.guild_id)==a.guild&&r.id!=r.guild_id).map(|r|view!{<option value=r.id.0.to_string()>{r.name.clone()}</option>}).collect_view()}</select></label>}.into_any()),
        "pins"=>view!{<h2>"Pinned Messages"</h2>{if a.pins.is_empty(){view!{<div class=SETTINGS_EMPTY>{icon("pin")}<p>"No pinned messages yet."</p><small>"Open a message’s actions and choose Pin Message to keep it here."</small></div>}.into_any()}else{a.pins.iter().map(|m|view!{<div class="pinned-message border-b pb-3">{message(a,m,false)}<div class="mt-2 flex gap-2 pl-14">{button("Jump",&format!("jump:{}",m.id.0),"secondary")}{if a.guild.is_none()||settings::can(a,Permissions::MANAGE_MESSAGES){button("Unpin",&format!("unpin:{}",m.id.0),"danger-text")}else{().into_any()}}</div></div>}).collect_view().into_any()}}}.into_any(),
        "edit"=>{let text=a.channel.and_then(|c|a.model.messages.get(&c)).and_then(|ms|a.editing.and_then(|m|ms.get(&m))).map_or(String::new(),|m|editable(a,m));form("Edit Message","edit",view!{<label class="field"><span>"Message"</span><textarea id="f-name" rows="5" aria-describedby="edit-help">{text}</textarea><small id="edit-help">"Enter saves. Shift+Enter adds a line."</small></label>}.into_any())},
        "profile"=>profiles::card(a),
        value if value.starts_with("role:")=>{let r=id(value.trim_start_matches("role:")).and_then(|id|a.roles.get(&id));if let Some(r)=r{view!{{form_with("Edit Role","role-save",view!{<fieldset disabled=!settings::can(a,Permissions::MANAGE_ROLES)>{field("f-name","Role Name",&r.name,"text")}{field("f-value","Color",&format!("#{:06x}",r.color),"color")}<input id="f-permissions" type="hidden" value=r.permissions.bits().to_string()/><div class="live-permissions my-4">{role_permissions(r)}</div><label class="live-checkbox"><input id="f-hoist" type="checkbox" checked=r.hoist/>"Display role members separately"</label><label class="live-checkbox"><input id="f-mentionable" type="checkbox" checked=r.mentionable/>"Allow anyone to mention this role"</label><input id="f-extra" type="hidden" value=r.id.0.to_string()/>{if r.id!=r.guild_id{button("Delete Role",&format!("role-delete:{}",r.id.0),"danger-text")}else{().into_any()}}</fieldset>}.into_any(),"dialog:roles",settings::can(a,Permissions::MANAGE_ROLES))}{identity::role_editor(a,r)}}.into_any()}else{().into_any()}},
        value if value.starts_with("channel-edit:")=>settings::channel_editor(a,value),
        value if value.starts_with("channel-delete:")=>settings::channel_delete(a,value),
        value if value.starts_with("channel-move:")=>settings::channel_move(a,value),
        value if moderation::owns(value)=>moderation::page(a,value),
        value if super::apps::owns_page(value)=>apps::page_view(a,value),
        value if privacy::owns(value)=>privacy::page(a,value),
        value if value.starts_with("invite-revoke:")=>form_with("Revoke Invite","invite-revoke",view!{<p>"This invite link will stop working immediately."</p><input id="f-name" type="hidden" value=value.trim_start_matches("invite-revoke:").to_owned()/>}.into_any(),"dialog:invite",true),
        value if value.starts_with("unban:")=>form_with("Revoke Ban","unban",view!{<p>"This person will be able to join again with an invite."</p><input id="f-name" type="hidden" value=value.trim_start_matches("unban:").to_owned()/>}.into_any(),"dialog:server-bans",true),
        "guild-delete"=>form_with("Delete Server","guild-delete",view!{<p>"This permanently deletes the server, its channels and every message in them, for everyone. It cannot be undone."</p>{field("f-name","Type the server’s name to confirm","","text")}}.into_any(),"dialog:server-overview",true),
        _=>view!{<h2>"Settings"</h2>}.into_any(),
    };
    if user_settings(&a.dialog) {
        return settings_layout(a, body);
    }
    if settings::is_server_page(&a.dialog) {
        return settings::server_layout(a, body);
    }
    modal(
        a,
        if a.dialog == "profile" {
            "profile-popout"
        } else {
            ""
        },
        dialog_label(&a.dialog),
        body,
    )
}

fn short_time(ms: u64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(ms as f64));
    let h = d.get_hours();
    format!(
        "{}:{:02} {}",
        if h.is_multiple_of(12) { 12 } else { h % 12 },
        d.get_minutes(),
        if h < 12 { "AM" } else { "PM" }
    )
}
fn timestamp(ms: u64) -> String {
    let parts = |ms: u64| {
        let d = js_sys::Date::new(&JsValue::from_f64(ms as f64));
        sticktalk_client_core::datetime::Parts {
            year: d.get_full_year() as u16,
            month: (d.get_month() + 1) as u16,
            day: d.get_date() as u16,
            hour: d.get_hours() as u16,
            minute: d.get_minutes() as u16,
        }
    };
    let now = Browser.unix_ms();
    sticktalk_client_core::datetime::label(
        parts(ms),
        parts(now),
        parts(now.saturating_sub(86_400_000)),
        false,
    )
}

use crate::permission_lists::ROLE_PERMISSIONS;
fn role_permissions(role: &Role) -> AnyView {
    view!{{ROLE_PERMISSIONS.iter().map(|&(title,permissions)|view!{<section class="permission-group [&+&]:mt-6 [&+&]:border-t [&+&]:pt-6 [&_.live-checkbox]:border-b"><h3 class="category mb-1">{title}</h3>{permissions.iter().map(|&(label,bit)|view!{<label class="live-checkbox"><span>{label}</span><input type="checkbox" checked=role.permissions.contains(bit) data-role-bit=bit.bits().to_string()/></label>}).collect_view()}</section>}).collect_view()}}.into_any()
}

#[lazy]
pub(super) fn load_dialog() -> fn(&App) -> AnyView {
    dialog
}

/// A view in its own WASM module. The first use downloads it; until then
/// the caller shows a placeholder.
pub(super) struct LazyView<F: Copy> {
    view: std::cell::Cell<Option<F>>,
    loading: std::cell::Cell<bool>,
}
impl<F: Copy> Default for LazyView<F> {
    fn default() -> Self {
        Self {
            view: std::cell::Cell::new(None),
            loading: std::cell::Cell::new(false),
        }
    }
}
type Loading<F> = std::pin::Pin<Box<dyn std::future::Future<Output = F>>>;
/// Views that are not on the first screen (docs/features/perf.md).
#[derive(Default)]
pub(super) struct LazyViews {
    pub forum: LazyView<fn(&App, &str) -> AnyView>,
    pub search: LazyView<fn(&App) -> AnyView>,
    /// Feature pages and dialogs, each in its feature's view module: opening
    /// User Settings does not download them.
    pub community: LazyView<fn(&App) -> Option<AnyView>>,
    pub stage: LazyView<StageDialog>,
    pub themes: LazyView<fn(&App, &str) -> Option<AnyView>>,
    pub identity: LazyView<identity::Views>,
    pub activity_card: LazyView<fn(&App, &Message) -> AnyView>,
    pub stage_card: LazyView<fn(&App, &Message) -> AnyView>,
    pub auth: LazyView<fn(&App) -> AnyView>,
    pub poll: LazyView<fn(&App, &Message) -> AnyView>,
    pub apps_picker: LazyView<fn(&App) -> AnyView>,
    /// The contents of the expression picker's tabs (`ui/picker.rs`).
    pub picker: LazyView<fn(&App) -> AnyView>,
}
type StageDialog = fn(&App) -> Option<(AnyView, &'static str, &'static str)>;
#[lazy]
pub(super) fn load_community_dialog() -> fn(&App) -> Option<AnyView> {
    community::dialog
}
#[lazy]
pub(super) fn load_stage_dialog() -> StageDialog {
    stage::dialog
}
#[lazy]
pub(super) fn load_themes_page() -> fn(&App, &str) -> Option<AnyView> {
    themes::page
}
/// Server identity's settings pages, tag card and role editor controls.
#[lazy]
pub(super) fn load_identity_dialog() -> identity::Views {
    identity::table()
}
/// The dialogs whose views are in a feature's view module (`LazyViews`),
/// with a placeholder while that module downloads.
fn feature_dialog(a: &App) -> Option<AnyView> {
    let d = a.dialog.as_str();
    if super::stage::owns_dialog(d) {
        let view = lazy_view(a, |a| &a.views.stage, || Box::pin(load_stage_dialog()));
        return Some(match view.and_then(|v| v(a)) {
            Some((body, label, class)) => modal(a, class, label, body),
            None => loading_dialog(a),
        });
    }
    if community::owns(d) {
        let view = lazy_view(
            a,
            |a| &a.views.community,
            || Box::pin(load_community_dialog()),
        );
        return Some(view.and_then(|v| v(a)).unwrap_or_else(|| loading_dialog(a)));
    }
    None
}
/// A themes page's body (it goes in the settings frame like any page).
fn themes_page(a: &App) -> AnyView {
    let view = lazy_view(a, |a| &a.views.themes, || Box::pin(load_themes_page()));
    view.and_then(|v| v(a, &a.dialog)).unwrap_or_else(|| {
        let title = themes::title(&a.dialog).unwrap_or("Appearance");
        view! {<h2>{title}</h2><p class="muted-copy" role="status">"Loading…"</p>}.into_any()
    })
}
/// The view in `slot`, or None while its module is still downloading.
fn lazy_view<F: Copy + 'static>(
    a: &App,
    slot: fn(&App) -> &LazyView<F>,
    load: fn() -> Loading<F>,
) -> Option<F> {
    let s = slot(a);
    if let Some(view) = s.view.get() {
        return Some(view);
    }
    if !s.loading.replace(true) {
        let owner = a.owner.clone();
        spawn_local(async move {
            let view = load().await;
            if let Some(app) = owner.upgrade() {
                let a = app.borrow();
                slot(&a).view.set(Some(view));
                a.touch();
            }
        });
    }
    None
}
#[lazy]
fn load_forum() -> fn(&App, &str) -> AnyView {
    threads::forum
}
#[lazy]
fn load_search() -> fn(&App) -> AnyView {
    navui::search_panel
}
#[lazy]
pub(super) fn load_call() -> fn(&App) -> AnyView {
    voice
}
fn loading_dialog(a: &App) -> AnyView {
    modal(
        a,
        "",
        "Loading",
        view! {<p role="status">"Loading…"</p>}.into_any(),
    )
}
fn loading_call() -> AnyView {
    view! {<div class="voice-view flex min-h-0 flex-1 flex-col bg-rail p-4" role="status">"Connecting…"</div>}.into_any()
}

fn member_groups(a: &App) -> AnyView {
    const ROW: &str = "member-row flex min-h-[42px] w-full items-center gap-3 px-4 py-1 hover:bg-sidebar-accent touch:min-h-11";
    const NAME: &str = "role-color min-w-0 truncate text-[15px] font-medium [&_.icon]:ml-1 [&_.icon]:inline [&_.icon]:size-3.5 [&_.icon]:text-warning";
    if a.guild.is_none() {
        if let Some(c) = a.channel.and_then(|id| a.model.channels.get(&id)) {
            return view!{<h2 class="category mx-4 mt-6 mb-2">{format!("Members — {}",c.recipient_ids.len())}</h2>{c.recipient_ids.iter().filter_map(|id|a.model.users.get(id)).map(|u|view!{<button class=ROW title=name(u) data-live=format!("profile:{}",u.id.0)>{profiles::nameplate(u)}{avatar_status(a,u)}<span class=NAME data-member-name=u.id.0.to_string()>{profiles::name_for(a,u.id,&name(u))}</span>{identity::tag_static(a,u)}</button>}).collect_view()}{if c.kind==ChannelKind::GroupDm {view!{<button class="nav-row mx-2 mt-2 flex w-[calc(100%-16px)] items-center rounded-md p-2 text-sm font-medium text-sidebar-foreground hover:bg-sidebar-accent touch:min-h-11" data-live="dialog:group-members">"Manage Group"</button>}.into_any()}else{().into_any()}}}.into_any();
        }
    }
    let mut groups: BTreeMap<(i64, String), Vec<&Member>> = BTreeMap::new();
    for m in a
        .model
        .members
        .values()
        .filter(|m| Some(m.guild_id) == a.guild)
    {
        let hoisted = m
            .role_ids
            .iter()
            .filter_map(|id| a.roles.get(id))
            .filter(|r| r.hoist)
            .max_by_key(|r| r.position);
        let group = if !online(a, m.user.id) {
            (i64::MAX, "Offline".into())
        } else if let Some(r) = hoisted {
            (-(r.position as i64), r.name.clone())
        } else {
            (i64::MAX - 1, "Online".into())
        };
        groups.entry(group).or_default().push(m);
    }
    view!{{groups.into_iter().map(|((_,label),mut members)|{
        members.sort_by_key(|m|name(&m.user).to_lowercase());
        let offline=label=="Offline";
        view!{<section class=if offline{"member-group offline"}else{"member-group"}>
            <h2 class="category mx-4 mt-6 mb-2">{format!("{} — {}",label,members.len())}</h2>
            {members.into_iter().map(|m|{let shown=super::privacy::nickname(m.user.id).or_else(||m.nickname.clone()).unwrap_or_else(||name(&m.user));let look=profiles::member_user(m);view!{<button class=if offline{"member-row flex min-h-[42px] w-full items-center gap-3 px-4 py-1 opacity-60 hover:bg-sidebar-accent hover:opacity-100 touch:min-h-11"}else{ROW} title=shown.clone() data-live=format!("profile:{}",m.user.id.0)>{profiles::nameplate(&look)}{avatar_status(a,&look)}<span class=NAME data-member-name=m.user.id.0.to_string() style=themes::role_style(&color(a,m.user.id))>{identity::role_name(a,m.user.id,&shown,profiles::styled_name(&shown,look.style.name.as_ref()))}{if m.role==MemberRole::Owner {icon("crown")} else {().into_any()}}{apps::tag(&m.user)}</span>{identity::row_after(a,&m.user)}</button>}}).collect_view()}
        </section>}
    }).collect_view()}}.into_any()
}
fn user_settings(page: &str) -> bool {
    matches!(
        page,
        "settings"
            | "account"
            | "profile-edit"
            | "about"
            | "password"
            | "devices"
            | "media-settings"
            | "notifications"
            | "channel-notifications"
            | "appearance"
            | "privacy"
            | "delete-account"
            | "data-export"
    ) || page.starts_with("target-notifications:")
        || expression::is_user_page(page)
        || themes::is_user_page(page)
        || super::apps::is_user_page(page)
}
fn settings_nav(a: &App) -> AnyView {
    let u = &a.model.me.as_ref().unwrap().user;
    view!{<div class="settings-identity flex items-center gap-3 px-2 pt-2 pb-6">{avatar(u,"settings-avatar")}<span class="flex min-w-0 flex-col gap-1"><strong class="text-lg leading-tight font-semibold wrap-anywhere">{name(u)}</strong><small>{format!("@{}",u.username)}</small><button class="text-left text-[13px] text-primary hover:underline touch:min-h-11" data-live="dialog:profile-edit">"Edit Profile"</button></span></div>{privacy::settings_status(a)}
        <label class="settings-search"><span class="sr-only">"Search settings"</span><input class="mb-4 w-full" type="search" placeholder="Search settings" data-settings-search/></label>
        <h2 class="category mx-2 mt-6 mb-2">"User Settings"</h2>
        {[( "user-round","My Account","account"),("user-round","Profiles","profile-edit"),("shield","Privacy & Safety","privacy"),("shield","Password & Security","password"),("monitor","Devices","devices"),("download","Your Data","data-export"),("bell","Notifications","notifications")].into_iter().map(|(glyph,label,page)|settings_nav_row(glyph,label,&format!("dialog:{page}"),a.dialog==page)).collect_view()}
        <h2 class="category mx-2 mt-6 mb-2">"App Settings"</h2>
        {settings_nav_row("headphones","Voice & Video","dialog:media-settings",a.dialog=="media-settings")}
        {settings_nav_row("monitor","Appearance","dialog:appearance",a.dialog=="appearance")}
        {themes::settings_nav(a)}
        {expression::settings_nav(a)}
        {apps::settings_nav(a)}
        <div class="settings-divider my-6 phone:mb-4"></div><button class="settings-nav-row danger-text flex min-h-9 w-full items-center gap-3 rounded-md px-3 py-2 text-left text-sm font-medium text-destructive hover:bg-destructive/10 touch:min-h-12 touch:text-[15px] phone:mt-0.5 phone:min-h-[52px] phone:bg-accent phone:p-3" data-live="logout">{icon("log-out")}<span class="flex-1">"Log Out"</span></button><small class="muted-copy px-3 pt-4">{concat!("StickTalk ",env!("CARGO_PKG_VERSION"))}</small>
    }.into_any()
}
fn settings_layout(a: &App, body: AnyView) -> AnyView {
    let title = match a.dialog.as_str() {
        "account" => "My Account",
        "profile-edit" => "Profiles",
        "about" => "About Me",
        "password" => "Password & Security",
        "devices" => "Devices",
        "appearance" => "Appearance",
        "privacy" => "Privacy & Safety",
        "delete-account" => "Delete Account",
        "data-export" => "Your Data",
        "media-settings" => "Voice & Video",
        "expression-settings" => "Expression",
        value if themes::title(value).is_some() => themes::title(value).unwrap_or("Settings"),
        value if super::apps::title(value).is_some() => {
            super::apps::title(value).unwrap_or("Settings")
        }
        "notifications" | "channel-notifications" => "Notifications",
        value if value.starts_with("target-notifications:") => "Notifications",
        _ => "Settings",
    };
    settings_page(
        a,
        SettingsPage {
            root: a.dialog == "settings",
            label: "User Settings",
            nav_label: "User settings",
            mobile_title: "Settings",
            // A page opened from another page goes back to that page.
            back: match a.dialog.as_str() {
                "about" => ("Back to Profiles", "dialog:profile-edit"),
                "delete-account" => ("Back to My Account", "dialog:account"),
                "theme-editor" => ("Back to Appearance", "dialog:appearance"),
                _ => ("Back to Settings", "dialog:settings"),
            },
            title,
        },
        settings_nav(a),
        body,
    )
}
fn account(a: &App) -> AnyView {
    let me = &a.model.me.as_ref().unwrap().user;
    const FIELD_ROW: &str =
        "settings-field-row flex items-center justify-between gap-4 border-t p-4 text-[15px]";
    view!{<h2>"My Account"</h2><section class="account-card overflow-hidden rounded-xl border bg-card"><div class="profile-banner"></div><div class="account-identity flex flex-wrap items-center gap-4 px-4 pb-4 phone:gap-3">{avatar(me,"large -mt-6 border-[6px] border-card")}<div><strong class="text-xl font-semibold">{name(me)}</strong><p class="mt-1 text-[13px] text-subtle">{format!("@{}",me.username)}</p></div><span class="ml-auto phone:ml-0">{button("Edit Profile","dialog:profile-edit","secondary")}</span></div><div class=FIELD_ROW><span class="min-w-0 wrap-anywhere"><strong class="font-semibold">"Display Name"</strong><p class="mt-1 text-subtle">{name(me)}</p></span>{button("Edit","dialog:profile-edit","secondary")}</div><div class=FIELD_ROW><span class="min-w-0 wrap-anywhere"><strong class="font-semibold">"Username"</strong><p class="streamer-account mt-1 text-subtle">{me.username.clone()}</p></span></div></section><div class="settings-divider"></div><h2>"Password & Security"</h2>{button("Change Password","dialog:password","secondary")}{if a.google{view!{<h2>"Google Sign-in"</h2>{field("google-current-password","Current Password (or verify with Google first)","","password")}{button("Verify with Google to Link","reauth-google:link","secondary")}{button("Link Google","google-link","secondary")}{button("Verify with Google to Unlink","reauth-google:unlink","secondary")}{button("Unlink Google","google-unlink","secondary")}}.into_any()}else{().into_any()}}<div class="settings-divider"></div><h2>"Devices"</h2><p class="muted-copy">"Manage where you’re signed in."</p>{button("Manage Devices","dialog:devices","secondary")}<section class="danger-zone mt-8 border-t pt-6"><h3>"Danger Zone"</h3><p class="muted-copy">"Permanently delete your account and messages. This cannot be undone."</p>{button("Delete Account","dialog:delete-account","danger-text")}</section>}.into_any()
}
fn group_members(a: &App) -> AnyView {
    let Some(c) = a
        .channel
        .and_then(|id| a.model.channels.get(&id))
        .filter(|c| c.kind == ChannelKind::GroupDm)
    else {
        return ().into_any();
    };
    let owner = a
        .model
        .me
        .as_ref()
        .is_some_and(|me| Some(me.user.id) == c.owner_id);
    view!{<h2>"Group Members"</h2>{c.recipient_ids.iter().filter_map(|id|a.model.users.get(id)).map(|u|view!{<div class=DEVICE_ROW>{avatar(u,"small")}<span class="mr-auto min-w-0 wrap-anywhere">{name(u)}</span>{if owner&&Some(u.id)!=c.owner_id {view!{{button("Remove",&format!("group-remove:{}",u.id.0),"danger-text")}{button("Make Owner",&format!("group-owner:{}",u.id.0),"secondary")}}.into_any()}else{().into_any()}}</div>}).collect_view()}
    {form("Add Friend","group-add",view!{<label class="field">"Friend"<select id="f-name">{a.model.relationships.values().filter(|r|r.kind==RelationshipKind::Friend&&!c.recipient_ids.contains(&r.user.id)).map(|r|view!{<option value=r.user.id.0.to_string()>{name(&r.user)}</option>}).collect_view()}</select></label>}.into_any())}
    {button("Leave Group","group-leave","danger-text")}}.into_any()
}
