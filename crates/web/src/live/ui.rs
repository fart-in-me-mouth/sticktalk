//! Live client views. Styling is Tailwind CSS utilities plus the shadcn/ui
//! component classes in `styles/app.css`; dialogs, tooltips, menus and avatars
//! are UI SDK custom elements (`public/ui.js`). Semantic class names such as
//! `message` or `live-modal` carry no styles of their own here: they are stable
//! hooks for `public/*.js` and the browser tests.
use super::*;
mod settings;
use sticktalk_client_core::markdown::{Block, Inline, Target};

fn icon(name: &str) -> AnyView {
    icon_with(name, "icon")
}
fn icon_with(name: &str, class: &'static str) -> AnyView {
    view! { <svg class=class aria-hidden="true" viewBox="0 0 24 24"><use href=format!("#i-{name}")></use></svg> }.into_any()
}
/// shadcn Button. `class` picks the variant: primary, secondary, danger,
/// link-button or danger-text.
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
    u.display_name.clone().unwrap_or_else(|| u.username.clone())
}
/// UI SDK Avatar: the initials stay until the image has loaded, and return if it fails.
fn avatar(u: &UserSummary, size: &str) -> AnyView {
    let initials = name(u)
        .split_whitespace()
        .filter_map(|s| s.chars().next())
        .take(2)
        .collect::<String>();
    view! {<ui-avatar class=format!("avatar {size}")>{u.avatar_id.map(|id|view!{<img class="avatar-face" src=format!("/api/v1/files/{}/thumbnail",id.0) alt="" loading="lazy"/>})}<ui-avatar-fallback class="avatar-face">{initials}</ui-avatar-fallback></ui-avatar>}
        .into_any()
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
/// The app-wide error toast. An open dialog lives in the browser's top layer,
/// so while one is open the toast is rendered inside it (see `modal`).
fn error_toast(a: &App) -> AnyView {
    if a.error.is_empty() {
        return ().into_any();
    }
    view!{<div class="live-error fixed bottom-8 left-1/2 z-[200] flex max-w-[calc(100vw-32px)] -translate-x-1/2 items-center gap-4 rounded-lg border border-warning bg-popover px-4 py-3 text-sm shadow-lg phone:bottom-[72px] phone:w-[calc(100vw-32px)]" role="alert">{a.error.clone()}{ib_plain("x","Dismiss","dismiss-error")}</div>}.into_any()
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
    if !a.ready && a.model.cursor.is_none() && (a.model.me.is_some() || a.status == "Connecting…")
    {
        return view!{<div class="app-shell skeleton-shell pointer-events-none relative top-(--viewport-offset) flex h-(--viewport-height) bg-background" role="status" aria-label="Loading conversations"><div class="left-panel flex w-[312px] flex-none bg-sidebar phone:hidden"><nav class="server-rail flex w-[72px] flex-col items-center gap-2 bg-rail p-3"><img class="rounded-2xl" src="/public/icons/icon-192.png" width="48" height="48" alt="StickTalk"/></nav><aside class="channel-sidebar flex w-60 flex-col"><div class="sidebar-header flex h-12 items-center border-b px-4 font-semibold">"StickTalk"</div><div class="skeleton-lines grid gap-4 px-4 py-6 [&>span]:h-4 [&>span]:w-4/5 [&>span]:rounded-sm [&>span]:bg-muted"><span></span><span></span><span></span><span></span></div></aside></div><main class="main-pane flex min-w-0 flex-1 flex-col"><header class="channel-header flex h-12 flex-none items-center border-b px-4"><strong>"StickTalk"</strong></header><div class="connection-banner bg-warning px-4 py-2 text-sm text-rail">"Reconnecting…"</div><div class="skeleton-messages mt-auto grid gap-6 p-4 [&>div]:h-10 [&>div]:w-[min(80%,480px)] [&>div]:rounded-lg [&>div]:bg-muted"><div></div><div></div><div></div></div><div class="skeleton-composer m-4 h-12 rounded-lg bg-muted"></div></main></div>}.into_any();
    }
    view!{<div id="sticktalk" data-live-app data-screen=a.screen.clone() data-self-id=a.model.me.as_ref().map(|me|me.user.id.0.to_string())>
        <div hidden inner_html=include_str!("../../public/icons/sprite.svg")></div>
        {if a.model.me.is_none(){auth(a)}else{shell(a)}}
        {if a.dialog.is_empty(){error_toast(a)}else{a.dialog_render.map_or_else(||loading_dialog(a), |render|render(a))}}

    </div>}.into_any()
}
fn auth(a: &App) -> AnyView {
    const PAGE: &str = "auth-page grid h-(--viewport-height) place-items-center overflow-y-auto p-6 [scroll-padding-block:calc(16px+var(--safe-top))_calc(24px+var(--safe-bottom))] phone:relative phone:top-(--viewport-offset) phone:flex phone:flex-col phone:justify-[safe_center] phone:bg-background phone:px-6 phone:pt-[calc(32px+var(--safe-top))] phone:pb-[calc(32px+var(--safe-bottom))]";
    const CARD: &str = "auth-card w-[min(440px,100%)] rounded-xl border bg-background p-8 shadow-sm phone:my-auto phone:w-full phone:max-w-[480px] phone:rounded-none phone:border-0 phone:bg-transparent phone:p-0 phone:shadow-none";
    if a.status == "Reconnecting…" {
        return view!{<main class=PAGE><section class=CARD><h1>"Reconnecting…"</h1><p class="muted-copy my-4">"Your conversations will appear when the server is available."</p>{button("Try Again","recover","primary")}</section></main>}.into_any();
    }

    let signup = a.signup;
    let register = a.register;
    view!{<main class=PAGE><form class=CARD data-live-form="auth"><div class="auth-brand flex items-center gap-3 text-xl font-semibold phone:justify-center"><img class="auth-logo flex-none rounded-xl" src="/public/icons/icon-192.png" alt="" width="48" height="48"/><span>"StickTalk"</span></div><h1 class="mt-8 mb-2 text-[28px] leading-tight phone:text-center">{if signup{"Choose a username"}else if register{"Create an account"}else{"Welcome back!"}}</h1><p class="auth-subtitle mb-8 text-[15px] text-subtle phone:text-center">{if signup{"Choose how people will find you on StickTalk."}else if register{"Create your StickTalk account."}else{"Log in to catch up with your friends."}}</p>
        <div class="field mb-6"><label for="auth-username">"Username"</label><input class="h-12 text-base touch:min-h-[52px]" id="auth-username" name="username" autocomplete="username" autocapitalize="none" autocorrect="off" spellcheck="false" required minlength="3" maxlength="32" autofocus /></div>
        {if signup{().into_any()}else{view!{<div class="field mb-6"><label for="auth-password">"Password"</label><input class="h-12 text-base touch:min-h-[52px]" id="auth-password" name="password" type="password" autocomplete=if register{"new-password"}else{"current-password"} required maxlength="128"/></div>}.into_any()}}
        <button class="button primary h-12 w-full text-[15px]" type="submit" disabled=a.status=="Signing in…">{if signup{"Continue"}else if register{"Create Account"}else{"Log In"}}</button>
        <p class="auth-status my-3 text-[13px] text-muted-foreground empty:hidden" role="status">{a.status.clone()}</p>
        {if a.google&&!signup {view!{<a class="button secondary google-button mt-4 flex h-12 w-full border bg-muted text-[15px]" href="/api/v1/browser/google/start"><img src="/public/google-g.svg" alt="" width="20" height="20"/>"Continue with Google"</a>}.into_any()}else{().into_any()}}
        {if signup{().into_any()}else{view!{<p class="auth-switch mt-6 text-[15px] text-subtle phone:text-center">{if register{"Already have an account? "}else{"Need an account? "}}<button class="inline-flex min-h-11 items-center px-1 font-semibold text-primary hover:underline" type="button" data-live="register">{if register{"Log in"}else{"Register"}}</button></p>}.into_any()}}
    </form></main>}.into_any()
}
/// Server channels in display order: root channels by position, and each
/// category followed by the channels nested inside it.
fn channel_tree(a: &App) -> Vec<&ChannelSummary> {
    let mut root = a
        .model
        .channels
        .values()
        .filter(|c| c.guild_id == a.guild && c.parent_id.is_none())
        .collect::<Vec<_>>();
    if a.guild.is_none() {
        return root;
    }
    root.sort_by_key(|c| (c.position, c.id));
    let mut out = Vec::with_capacity(root.len());
    for c in root {
        out.push(c);
        if c.kind == ChannelKind::Category {
            let mut children = a
                .model
                .channels
                .values()
                .filter(|v| v.parent_id == Some(c.id))
                .collect::<Vec<_>>();
            children.sort_by_key(|v| (v.position, v.id));
            out.extend(children);
        }
    }
    out
}
fn shell(a: &App) -> AnyView {
    let channel = a.channel.and_then(|id| a.model.channels.get(&id));
    let title = channel.map_or("Friends".into(), |c| a.model.channel_name(c));
    let guild_name = a
        .guild
        .and_then(|g| a.model.guilds.get(&g))
        .map(|g| g.guild.name.clone())
        .unwrap_or("Direct Messages".into());
    let me = &a.model.me.as_ref().unwrap().user;
    let manage_channels = settings::can(a, Permissions::MANAGE_CHANNELS);
    let nested = |c: &ChannelSummary| a.guild.is_some() && c.parent_id.is_some();
    view!{<div class=format!("app-shell live-shell relative top-(--viewport-offset) flex h-(--viewport-height) bg-background phone:pt-(--safe-top) panel-{} {}",a.panel,if channel.is_some(){"in-channel"}else{""})>
        <div class=format!("left-panel flex w-[312px] flex-none touch-pan-y bg-sidebar phone:absolute phone:top-(--safe-top) phone:bottom-0 phone:left-0 phone:z-30 phone:w-[min(340px,90vw)] {}",if a.panel=="left"{"phone:translate-x-0"}else{"phone:-translate-x-full"})><nav class="server-rail flex w-[72px] flex-none touch-pan-y flex-col items-center gap-2 overflow-y-auto overscroll-contain bg-rail p-3" aria-label="Servers"><button class="server-icon brand grid size-12 min-h-12 place-items-center overflow-hidden rounded-2xl" data-live="home" aria-label="Home"><img src="/public/icons/icon-192.png" alt="" width="48" height="48"/></button><div class="rail-divider min-h-0.5 w-8 bg-border"></div>
            {a.model.guilds.values().map(|g|{let selected=a.guild==Some(g.guild.id);view!{<ui-tooltip class="contents"><ui-tooltip-trigger><button class=format!("server-icon relative grid size-12 min-h-12 place-items-center font-semibold text-foreground transition-[border-radius,background-color] hover:rounded-2xl hover:bg-secondary {}",if selected{"selected rounded-2xl bg-secondary"}else{"rounded-full bg-muted"}) data-live=format!("guild:{}",g.guild.id.0) aria-label=g.guild.name.clone()>{if selected{view!{<span class="absolute -left-3 h-10 w-1 rounded-r-sm bg-foreground" aria-hidden="true"></span>}.into_any()}else{().into_any()}}{if let Some(id)=g.guild.icon_id{view!{<img class="size-full rounded-[inherit] object-cover" src=format!("/api/v1/files/{}/thumbnail",id.0) alt="" loading="lazy"/>}.into_any()}else{view!{<span>{g.guild.name.chars().take(2).collect::<String>()}</span>}.into_any()}}</button></ui-tooltip-trigger><ui-tooltip-content class="tooltip" side="right" side-offset="8" hidden=true>{g.guild.name.clone()}</ui-tooltip-content></ui-tooltip>}}).collect_view()}
            {ib("plus","Add a Server","dialog:create")}
        </nav><aside class="channel-sidebar flex min-h-0 w-60 flex-col phone:w-[calc(100%-72px)]"><div class="sidebar-header flex h-12 min-h-12 items-center gap-1 border-b px-3 py-2">{if a.guild.is_some(){view!{
                <ui-menu class="contents"><ui-menu-trigger><button class="server-heading flex h-8 min-w-0 flex-1 items-center gap-1 rounded-md px-1 hover:bg-sidebar-accent" aria-label=format!("{guild_name} menu")><strong class="min-w-0 flex-1 truncate text-base font-semibold">{guild_name.clone()}</strong>{icon_with("chevron-down","icon size-4 text-muted-foreground")}</button></ui-menu-trigger>
                <ui-menu-content class="menu" side="bottom" align="start" side-offset="6" hidden=true>
                    <ui-menu-item class="menu-item" value="dialog:invite">{icon("user-plus")}"Invite People"</ui-menu-item>
                    <ui-menu-item class="menu-item" value=a.guild.map_or(String::new(),|g|format!("dialog:target-notifications:guild:{}",g.0))>{icon("bell")}"Notification Settings"</ui-menu-item>
                    {if manage_channels{view!{<ui-menu-separator class="menu-separator"></ui-menu-separator><ui-menu-item class="menu-item" value="dialog:channel">{icon("hash")}"Create Channel"</ui-menu-item><ui-menu-item class="menu-item" value="dialog:category">{icon("folder")}"Create Category"</ui-menu-item>}.into_any()}else{().into_any()}}
                    <ui-menu-separator class="menu-separator"></ui-menu-separator>
                    <ui-menu-item class="menu-item" value="dialog:server">{icon("settings")}"Server Settings"</ui-menu-item>
                </ui-menu-content></ui-menu>
                {ib("plus","Create Channel","dialog:channel")}{ib("settings","Server Settings","dialog:server")}}.into_any()}else{view!{<strong class="min-w-0 flex-1 truncate px-1 text-base font-semibold">{guild_name}</strong>{ib("users","New Group Message","dialog:group")}}.into_any()}}</div>
            <nav class="channel-scroll min-h-0 flex-1 touch-pan-y overflow-y-auto overscroll-contain p-2" aria-label="Channels">
            <button class="nav-row flex w-full items-center justify-start gap-2 rounded-md p-2 text-sm font-medium text-sidebar-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground touch:min-h-11 phone:text-base" data-live="home">{icon_with("users","icon text-muted-foreground")}"Friends"</button>
            {channel_tree(a).into_iter().map(|c|{
                let unread=a.model.reads.get(&c.id);let label=a.model.channel_name(c);
                if c.kind==ChannelKind::Category{return view!{<div class="category-row mt-4 flex min-h-7 items-center gap-1 pr-1 pl-2"><h2 class="category min-w-0 flex-1 truncate">{label.clone()}</h2>{if manage_channels{view!{<button class="grid size-6 flex-none place-items-center rounded-sm text-muted-foreground hover:bg-sidebar-accent hover:text-foreground touch:size-11" aria-label=format!("Create channel in {label}") title=format!("Create channel in {label}") data-live=format!("dialog:channel-in:{}",c.id.0)>{icon_with("plus","icon size-4")}</button>}.into_any()}else{().into_any()}}</div>}.into_any()}
                let selected=a.channel==Some(c.id);
                view!{<div class=format!("channel-row min-h-[34px] rounded-md {} {}",if selected{"selected bg-secondary"}else{"hover:bg-sidebar-accent"},if nested(c){"nested ml-2"}else{""})><button class=format!("channel-main flex w-full items-center gap-2 p-2 text-sm font-medium touch:min-h-11 phone:text-base {}",if selected{"text-foreground"}else{"text-sidebar-foreground"}) data-live=format!("{}:{}",if c.kind==ChannelKind::Voice{"voice"}else{"channel"},c.id.0)>{icon_with(if c.kind==ChannelKind::Voice{"volume-2"}else if c.guild_id.is_none(){"message-circle"}else{"hash"},"icon text-muted-foreground")}<span class="truncate">{label}</span>{if let Some(read)=unread.filter(|r|r.unread_count>0){view!{<span class="live-unread">{read.unread_count.to_string()}</span>}.into_any()}else{().into_any()}}</button></div>}.into_any()
            }).collect_view()}
            </nav>
            <section id="live-connected" class="voice-connected border-t p-3 text-sm text-primary" hidden><div class="voice-connection-line"><strong>"Voice Connected"</strong></div><div class="voice-tiles mt-2 flex gap-2 [&_.icon-button]:flex-1 [&_.icon-button]:bg-accent">{ib("video","Camera","media:camera")}{ib("monitor","Share Screen","media:share")}{ib("phone-off","Disconnect","media:leave")}</div></section>
            <section class="user-panel m-2 flex min-h-[52px] items-center gap-1 rounded-lg bg-panel p-2 touch:min-h-[60px] phone:h-auto phone:pb-[max(8px,var(--safe-bottom))]">{avatar(me,"small")}<button class="self-label ml-1 min-w-0 flex-1 touch:min-h-11" data-live=format!("profile:{}",me.id.0)><strong class="block truncate text-sm leading-tight font-semibold">{name(me)}</strong><small class="block truncate text-xs">{format!("@{}",me.username)}</small></button>{ib("mic","Mute","media:mute")}{ib("headphones","Deafen","media:deafen")}{ib("settings","User Settings","dialog:settings")}</section>
        </aside></div><div class=if a.panel.is_empty(){"panel-scrim hidden"}else{"panel-scrim hidden touch-pan-y phone:absolute phone:inset-0 phone:z-[25] phone:block phone:bg-black/60"} data-live="panel:" aria-hidden="true"></div>
        <main class=format!("main-pane flex min-w-0 flex-1 touch-pan-y flex-col {}",if channel.is_none(){"phone:pb-[calc(56px+var(--safe-bottom))]"}else{""})><header class="channel-header flex h-12 min-h-12 flex-none items-center gap-2 border-b px-4 py-2 phone:h-(--header-height) phone:min-h-(--header-height) phone:px-2"><div class="mobile-menu hidden phone:block">{ib("menu","Open Channels","panel:left")}</div>{icon_with(if channel.is_some_and(|c|c.kind==ChannelKind::Voice){"volume-2"}else if channel.is_some(){"hash"}else{"users"},"icon text-muted-foreground")}<h1 class="min-w-0 flex-1 truncate text-base font-semibold">{title.clone()}</h1><div class="header-actions flex gap-2 phone:gap-0">{if a.screen=="voice"{ib("message-circle","Open Chat",&a.model.channels.values().find(|c|c.guild_id==a.guild&&c.kind==ChannelKind::Text).map_or("home".into(),|c|format!("channel:{}",c.id.0)))}else{view!{{ib("bell","Notification Settings","dialog:channel-notifications")}{ib("pin","Pinned Messages","dialog:pins")}{ib("users","Member List","panel:right")}}.into_any()}}</div></header>
            {if !a.status.is_empty(){view!{<div class="connection-banner bg-warning px-4 py-2 text-sm text-rail" role="status">{a.status.clone()}</div>}.into_any()}else{().into_any()}}
            {if a.screen=="voice"{a.voice_render.map_or_else(loading_call, |render|render(a))}else if channel.is_some(){chat(a,&title)}else{home(a)}}
        </main>
        <aside class=format!("member-list w-60 flex-none touch-pan-y overflow-auto overscroll-contain bg-sidebar {}",if a.panel=="right"{"is-open narrow:absolute narrow:top-12 narrow:right-0 narrow:bottom-0 narrow:z-20 narrow:shadow-[-8px_0_24px_#0006] phone:top-[calc(var(--header-height)+var(--safe-top))] phone:z-30 phone:w-[min(300px,90vw)] phone:pb-(--safe-bottom)"}else{"narrow:hidden"}) aria-label="Members">{member_groups(a)}{if a.guild.is_some(){button("Load More","members-more","link-button")}else{().into_any()}}</aside>
        {if channel.is_none(){view!{<nav class="bottom-tabs hidden phone:absolute phone:inset-x-0 phone:bottom-0 phone:flex phone:h-[calc(56px+var(--safe-bottom))] phone:justify-around phone:border-t phone:bg-sidebar phone:pb-(--safe-bottom)" aria-label="Main navigation">{button("Home","home","active text-primary")}{button("Notifications","dialog:notifications","text-subtle")}{button("You","dialog:settings","text-subtle")}</nav>}.into_any()}else{().into_any()}}
    </div>}.into_any()
}
fn home(a: &App) -> AnyView {
    view!{<div class="live-home touch-pan-y overflow-auto overscroll-contain p-6 phone:p-4"><div class="friends-toolbar mb-6 flex flex-wrap items-center gap-3 phone:gap-2"><h2 class="mr-auto">"Friends"</h2>{button("Add Friend","dialog:friend","primary")}{ib("users","New Group Message","dialog:group")}{button("Join a Server","dialog:join","secondary")}</div>
        {if a.model.relationships.is_empty(){view!{<p class="muted-copy">"No friends yet. Add a friend to start a conversation."</p>}.into_any()}else{().into_any()}}
        {a.model.relationships.values().map(|r|view!{<div class="friend-row flex items-center gap-3 border-t py-4 phone:flex-wrap">{avatar(&r.user,"small")}<button class="friend-info flex min-w-0 flex-1 flex-col gap-1 touch:min-h-11 phone:min-w-[140px] phone:text-base" data-live=format!("profile:{}",r.user.id.0)><strong class="font-semibold">{name(&r.user)}</strong><small>{format!("@{} · {}",r.user.username,match r.kind{RelationshipKind::Friend=>"Friend",RelationshipKind::IncomingRequest=>"Incoming request",RelationshipKind::OutgoingRequest=>"Outgoing request",RelationshipKind::Blocked=>"Blocked"})}</small></button>
        {match r.kind{RelationshipKind::IncomingRequest=>view!{{button("Accept",&format!("friend:accept,{}",r.user.id.0),"primary")}{button("Decline",&format!("friend:decline,{}",r.user.id.0),"secondary")}}.into_any(),RelationshipKind::OutgoingRequest=>button("Cancel",&format!("friend:cancel,{}",r.user.id.0),"secondary"),RelationshipKind::Blocked=>button("Unblock",&format!("friend:unblock,{}",r.user.id.0),"secondary"),_=>ib("message-circle","Message",&format!("friend:dm,{}",r.user.id.0))}}
        </div>}).collect_view()}
    </div>}.into_any()
}
fn chat(a: &App, title: &str) -> AnyView {
    let c = a.channel.unwrap();
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
    view!{<div class="chat-body flex min-h-0 flex-1 flex-col"><div id="live-messages" class="message-list min-h-0 flex-1 touch-pan-y overflow-auto overscroll-contain pb-4 [overflow-anchor:none]" role="log" aria-label="Messages" tabindex="0" data-channel=c.0.to_string() data-history-generation=a.history_generation.to_string() data-window-start=a.window_rank.to_string() data-window-end=(a.window_rank+shown.len()as u64).to_string() data-total=count.to_string()>
        <div style=format!("height:{}px",a.window_rank*88) aria-hidden="true"></div>
        {if shown.is_empty(){view!{<div class="conversation-intro px-4 py-8"><span class="intro-icon inline-flex rounded-full bg-muted p-4">{icon("hash")}</span><h2 class="mt-4 mb-2">{format!("Welcome to #{title}!")}</h2><p class="text-subtle">{if a.history_busy{"Loading messages…"}else{"This is the beginning of this conversation."}}</p></div>}.into_any()}else{().into_any()}}
        {shown.iter().enumerate().map(|(i,m)|message(a,m,crate::wire::continues_group(m.author_id.0,m.created_at,i.checked_sub(1).map(|p|(shown[p].author_id.0,shown[p].created_at))))).collect_view()}
        <div style=format!("height:{}px",bottom*88) aria-hidden="true"></div>
        </div><div class="composer-wrap relative px-4 pb-(--safe-bottom) phone:px-2">
        {if let Some(reply)=a.reply{view!{<div class="reply-preview flex items-center gap-2 py-1 text-[13px] text-subtle touch:min-h-11 phone:min-w-0 phone:wrap-anywhere">{format!("Replying to {}",messages.and_then(|m|m.get(&reply)).map_or("message".into(),|m|a.model.name(m.author_id)))}{ib_plain("x","Cancel Reply","cancel-reply")}</div>}.into_any()}else{().into_any()}}
        {a.attachments.iter().enumerate().map(|(i,f)|view!{<div class="live-upload mb-2 flex items-center justify-between gap-4 rounded-lg bg-muted p-2 text-xs"><span>{f.filename.clone()}</span>{ib_plain("x","Remove Attachment",&format!("remove-attachment:{i}"))}</div>}).collect_view()}
        {if a.uploading{view!{<div class="live-upload mb-2 flex items-center justify-between gap-4 rounded-lg bg-muted p-2 text-xs" role="status">{a.upload_progress.clone()}{button("Pause","upload-cancel","secondary")}</div>}.into_any()}else{().into_any()}}
        {if a.completions.is_empty(){().into_any()}else{view!{<div id="live-completions" class="autocomplete absolute inset-x-4 bottom-full z-40 max-h-[min(320px,50vh)] overflow-y-auto rounded-lg border bg-popover p-2 shadow-lg" role="listbox" aria-label="Suggestions">{a.completions.iter().enumerate().map(|(i,(label,insert,kind))|view!{<button class="flex h-9 w-full items-center gap-2 rounded-sm px-2 text-sm aria-selected:bg-secondary touch:min-h-11" id=format!("completion-{i}") role="option" aria-selected=if i==0{"true"}else{"false"} data-completion=insert.clone()><span class="completion-avatar grid size-6 flex-none place-items-center overflow-hidden rounded-full bg-sidebar text-xs [&_.icon]:size-4">{match kind.as_str() {"Role"=>icon("shield"),"Channel"=>icon("hash"),"Emoji"=>view!{<span>{insert.clone()}</span>}.into_any(),_=>view!{<span>{label.chars().take(1).collect::<String>()}</span>}.into_any()}}</span><span>{label.clone()}</span><small class="ml-auto">{if kind=="User"{insert.clone()}else{kind.clone()}}</small></button>}).collect_view()}</div>}.into_any()}}
        <div class="composer flex items-end gap-2 rounded-lg bg-muted px-3 py-2 phone:gap-1 phone:px-2 phone:py-1"><button type="button" class="icon-button" title="Add Attachment" aria-label="Add Attachment" data-live="pick-file">{icon("plus-circle")}</button><input type="file" id="live-file" hidden/><label class="composer-input flex min-w-0 flex-1"><span class="sr-only">"Message"</span><textarea class="max-h-[40vh] min-h-8 flex-1 resize-none rounded-none border-0 bg-transparent px-0 py-1 text-[15px] leading-6 shadow-none focus-visible:ring-0 touch:min-h-11 touch:py-2.5 phone:text-base" id="live-input" rows="1" placeholder=format!("Message #{title}") aria-label="Message" role="combobox" aria-autocomplete="list" aria-controls="live-completions" aria-expanded=(!a.completions.is_empty()).to_string() maxlength="4000"></textarea></label><button class="composer-gif inline-flex h-8 flex-none items-center justify-center rounded-md px-2 text-sm font-semibold text-primary hover:bg-accent touch:min-h-11 touch:min-w-11" data-live="dialog:gif">"GIF"</button>{ib_plain("smile","Emoji Picker","dialog:emoji")}<button class="icon-button" aria-label="Send Message" title="Send Message" data-live="send-input" disabled={a.uploading||a.preparing_send||!a.ready}>{icon("send")}</button></div>
        <div class="typing-indicator h-6 text-xs leading-6 text-subtle">{crate::wire::typing_label(&a.model.typing.iter().filter(|((channel,_),expiry)|*channel==c&&**expiry>Browser.unix_ms()).map(|((_,u),_)|a.model.name(*u)).take(3).collect::<Vec<_>>())}</div>
        </div></div>}.into_any()
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
    let body=match &v.target{Some(Target::Mention(u))if m.mention_ids.contains(u)=>view!{<button class="mention" data-live=format!("profile:{}",u.0)>{format!("@{}",a.model.name(*u))}</button>}.into_any(),Some(Target::Auto(url)|Target::Masked(url))if url.starts_with("https://")||url.starts_with("http://")=>view!{<a href=url.clone() target="_blank" rel="noopener noreferrer">{v.text.clone()}</a>}.into_any(),_=>view!{<span>{v.text.clone()}</span>}.into_any()};
    if v.spoiler.is_some() {
        view!{<button class="live-spoiler" data-spoiler aria-label="Reveal spoiler">{body}</button>}.into_any()
    } else {
        view! {<span style=style>{body}</span>}.into_any()
    }
}
fn message(a: &App, m: &Message, continuation: bool) -> AnyView {
    let author = a.model.users.get(&m.author_id).cloned().unwrap_or_default();
    let document = sticktalk_client_core::markdown::parse(&m.content);
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
    view!{<article class=format!("message group/message relative flex gap-4 px-4 hover:bg-accent {} {}",if continuation{"continuation mt-0.5"}else{"group-head mt-4 in-data-[density=compact]:mt-2"},if mention{"mentioned border-l-2 border-warning bg-warning/8"}else{""}) data-live-message=m.id.0.to_string() tabindex="0" aria-label=format!("Message from {}",name(&author))>
        <div class="avatar-gutter flex w-10 flex-none items-start justify-center self-start">{if continuation{view!{<time class="gutter-time invisible text-xs leading-[21px] whitespace-nowrap group-focus-within/message:visible group-hover/message:visible">{short_time(m.created_at)}</time>}.into_any()}else{view!{<button class="touch:grid touch:min-h-11 touch:min-w-11 touch:place-items-center touch:p-0.5" data-live=format!("profile:{}",m.author_id.0) aria-label=format!("{}'s profile",name(&author))>{avatar(&author,"message-avatar")}</button>}.into_any()}}</div>
        <div class="message-content min-w-0 flex-1 wrap-anywhere">{if continuation{().into_any()}else{view!{<div class="message-meta flex min-h-[21px] items-baseline gap-2 phone:flex-wrap phone:gap-x-2 phone:gap-y-1"><button class="author max-w-[min(45%,240px)] truncate p-0 font-semibold" style=format!("color:{}",color(a,m.author_id)) data-live=format!("profile:{}",m.author_id.0)>{name(&author)}</button><time class="text-xs">{timestamp(m.created_at)}</time></div>}.into_any()}}
        {if let Some(reply)=m.reply_to{view!{<button class="reply-preview flex items-center gap-2 py-1 text-[13px] text-subtle touch:min-h-11 phone:min-w-0 phone:wrap-anywhere" data-live=format!("seek-message:{}",reply.0)>{icon_with("reply","icon size-4")}{a.model.messages.get(&m.channel_id).and_then(|ms|ms.get(&reply)).map_or("Original message".into(),|m|format!("{}: {}",a.model.name(m.author_id),m.content.chars().take(100).collect::<String>()))}</button>}.into_any()}else{().into_any()}}
        <div class="markdown">{document.blocks.iter().map(|b|match b{Block::Code{text,..}=>view!{<pre><code>{text.clone()}</code></pre>}.into_any(),Block::Paragraph(v)|Block::Header(_,v)|Block::Quote(v)|Block::List(v)=>view!{<p>{v.iter().map(|v|inline(a,v,m)).collect_view()}</p>}.into_any()}).collect_view()}</div>
        {if m.edited_at.is_some(){view!{<small class="edited">"(edited)"</small>}.into_any()}else{().into_any()}}{m.attachments.iter().map(attachment).collect_view()}
        <div class="reactions mt-1 flex gap-1 empty:hidden">{m.reactions.iter().map(|r|view!{<button class=if r.me{"reaction mine rounded-lg border border-primary bg-primary/15 px-2 py-1 text-sm text-foreground touch:min-h-11 touch:min-w-11"}else{"reaction rounded-lg border border-transparent bg-muted px-2 py-1 text-sm text-foreground touch:min-h-11 touch:min-w-11"} data-live=format!("react:{},{}",m.id.0,r.emoji) aria-label=format!("React with {}",r.emoji) aria-pressed=r.me.to_string()>{r.emoji.clone()}" "{r.count}</button>}).collect_view()}</div></div>
        <div class="message-actions absolute -top-3 right-4 hidden rounded-lg border bg-popover shadow-sm group-focus-within/message:flex group-hover/message:flex touch:hidden!">{ib_plain("smile","Add Reaction",&format!("react:{},👍",m.id.0))}{ib_plain("reply","Reply",&format!("reply:{}",m.id.0))}{ib_plain("pin","Pin Message",&format!("pin:{}",m.id.0))}{if own{view!{{ib_plain("pencil","Edit Message",&format!("edit:{}",m.id.0))}{ib_plain("trash-2","Delete Message",&format!("delete:{}",m.id.0))}}.into_any()}else{().into_any()}}</div>
    </article>}.into_any()
}
fn attachment(f: &Attachment) -> AnyView {
    let original = format!("/api/v1/files/{}", f.id.0);
    let thumb = format!("{original}/thumbnail");
    if f.content_type.starts_with("image/") {
        if let Some(t) = &f.thumbnail {
            view!{<button class="attachment-image mt-2 block max-w-full overflow-hidden rounded-lg p-0" data-view-image=original data-filename=f.filename.clone() data-bytes=f.byte_size.to_string() aria-label=format!("Open {}",f.filename)><img class="block h-auto max-h-[350px] max-w-full object-contain" src=thumb.clone() srcset=format!("{thumb} 1x") loading="lazy" width=t.width height=t.height alt=f.filename.clone()/></button>}.into_any()
        } else {
            view!{<button class="button secondary" data-view-image=original data-filename=f.filename.clone()>{format!("View {}",f.filename)}</button>}.into_any()
        }
    } else if crate::wire::video_hint(&f.content_type, &f.filename) {
        view! {<div class="attachment-video mt-2 max-w-full"><video class="block max-h-[400px] max-w-full rounded-lg" controls playsinline preload="none" src=original.clone() aria-label=f.filename.clone()></video><a class="inline-flex min-h-11 max-w-full items-center text-sm wrap-anywhere" href=original download=f.filename.clone()>{format!("Download {}",f.filename)}</a></div>}
            .into_any()
    } else {
        view!{<a class="file-card mt-2 flex items-center gap-4 rounded-lg bg-muted p-3 text-foreground no-underline" href=original download=f.filename.clone()>{icon("file")}<span>{f.filename.clone()}<small>{format!("{} bytes",f.byte_size)}</small></span>{icon("download")}</a>}.into_any()
    }
}
fn voice(a: &App) -> AnyView {
    view!{<div class="voice-view flex min-h-0 flex-1 flex-col bg-rail p-4 phone:p-2 phone:pb-[max(8px,var(--safe-bottom))]" data-call-channel=a.voice_channel.map_or(String::new(),|id|id.0.to_string())><button class="call-minimize flex min-h-11 flex-none items-center gap-2 px-2 text-[13px] text-subtle" data-live="media:minimize" aria-label="Minimize call">{icon("chevron-down")}<span>"Minimize call"</span></button><div id="live-media-stage" class="participant-grid" aria-label="Call participants"></div><div class="call-controls flex flex-none justify-center gap-3 p-3 phone:gap-2 phone:px-0 [&_.icon-button]:size-12 [&_.icon-button]:rounded-full [&_.icon-button]:bg-muted [&_[data-live='media:leave']]:w-20 [&_[data-live='media:leave']]:rounded-[32px] [&_[data-live='media:leave']]:bg-destructive [&_[data-live='media:leave']]:text-primary-foreground">{ib("mic","Mute","media:mute")}{ib("headphones","Deafen","media:deafen")}{ib("video","Camera","media:camera")}<span class="desktop-share contents">{ib("monitor","Share Screen","media:share")}</span>{ib("volume-2","Audio Output","media:output")}{ib("phone-off","Leave Call","media:leave")}</div><p id="live-call-status" class="call-connection m-0 min-h-4 text-center text-[13px] text-primary" role="status">"Connecting…"</p></div>}.into_any()
}
fn form(title: &str, kind: &str, fields: AnyView) -> AnyView {
    let (label, class) = match kind {
        "delete-account" => ("Delete Account", "danger"),
        "channel-delete" => ("Delete Channel", "danger"),
        "invite-revoke" => ("Revoke Invite", "danger"),
        "unban" => ("Revoke Ban", "primary"),
        "ban" => ("Ban Member", "danger"),
        "role-create" => ("Create Role", "primary"),
        "friend" => ("Send Friend Request", "primary"),
        "group" => ("Create Group", "primary"),
        "about" | "role-save" | "edit" => ("Save Changes", "primary"),
        "guild-icon-remove" => ("Remove Icon", "danger"),
        "create" => ("Create Server", "primary"),
        "channel" => ("Create Channel", "primary"),
        "join" => ("Join Server", "primary"),
        "invite" => ("Create Invite", "primary"),
        _ => ("Save", "primary"),
    };
    view!{<form data-live-form=kind.to_owned()><h2>{title.to_owned()}</h2>{fields}<button type="submit" class=format!("button {class}")>{label}</button></form>}.into_any()
}
/// Create Channel. `kind` and `parent` preselect the type and the category the
/// new channel is nested in (the "+" beside a category, or Create Category).
fn channel_form(a: &App, kind: &str, parent: Option<Id>) -> AnyView {
    let title = if kind == "category" {
        "Create Category"
    } else {
        "Create Channel"
    };
    form(title,"channel",view!{{field("f-name","Name","","text")}<label class="field">"Type"<select id="f-extra"><option value="text" selected=kind=="text">"Text"</option><option value="voice" selected=kind=="voice">"Voice"</option><option value="category" selected=kind=="category">"Category"</option></select></label><label class="field">"Category"<select id="f-value" aria-describedby="channel-category-help"><option value="">"None"</option>{a.model.channels.values().filter(|c|c.guild_id==a.guild&&c.kind==ChannelKind::Category).map(|c|view!{<option value=c.id.0.to_string() selected=parent==Some(c.id)>{c.name.clone().unwrap_or_default()}</option>}).collect_view()}</select><small id="channel-category-help">"Text and voice channels can be nested inside a category."</small></label>}.into_any())
}
const DEVICE_ROW: &str =
    "device-row my-4 flex items-center justify-between gap-4 phone:items-start";
const SETTINGS_EMPTY: &str = "settings-empty flex flex-col items-start gap-4 pt-6 pb-2 [&>.icon]:size-8 [&>.icon]:text-subtle";
fn dialog(a: &App) -> AnyView {
    let body=match a.dialog.as_str(){
        "settings"|"account"=>account(a),
        "server"|"server-overview"=>settings::server_overview(a),
        "server-channels"=>settings::channels(a),
        "server-bans"=>settings::bans(a),
        "ban"=>form("Ban a Member","ban",view!{<p>"This removes the member and prevents them from rejoining until you revoke the ban."</p><label class="field"><span>"Member"</span><select id="f-name">{a.model.members.values().filter(|m|m.guild_id==a.guild.unwrap_or_default()&&a.model.me.as_ref().is_some_and(|me|me.user.id!=m.user.id)).map(|m|view!{<option value=m.user.id.0.to_string()>{name(&m.user)}</option>}).collect_view()}</select></label>{field("f-value","Reason (optional)","","text")}}.into_any()),
        "server-emoji"=>view!{<h2>"Emoji"</h2><p class="muted-copy">"Custom server emoji are not available yet. Everyone can use the emoji picker in chat."</p>}.into_any(),
        "appearance"=>settings::appearance(),
        "privacy"=>settings::privacy(a),
        "group-members"=>group_members(a),
        "group" if a.model.relationships.values().filter(|r|r.kind==RelationshipKind::Friend).count()<2=>view!{<h2>"New Group Message"</h2><div class=SETTINGS_EMPTY>{icon("users")}<p>"Add at least two friends to start a group message."</p>{button("Add Friends","dialog:friend","primary")}</div>}.into_any(),
        "group"=>form("New Group Message","group",view!{{field("f-name","Group Name","","text")}<fieldset class="group-recipients my-4"><legend class="text-[13px] font-medium text-subtle">"Friends"</legend>{a.model.relationships.values().filter(|r|r.kind==RelationshipKind::Friend).map(|r|view!{<label class="live-checkbox">{avatar(&r.user,"small")}<span class="mr-auto">{name(&r.user)}</span><input type="checkbox" data-group-user=r.user.id.0.to_string()/></label>}).collect_view()}</fieldset><input id="f-value" type="hidden"/><p class="muted-copy">"Choose 2 to 15 friends."</p>}.into_any()),
        "friend"=>form("Add Friend","friend",field("f-name","Username","","text")),
        "create"=>view!{{form("Create a Server","create",field("f-name","Server Name","","text"))}<div class="-mx-4 mt-2">{button("Join a Server","dialog:join","link-button")}</div>}.into_any(),
        "join"=>form("Join a Server","join",field("f-name","Invite Code","","text")),
        "channel"=>channel_form(a,"text",None),
        "category"=>channel_form(a,"category",None),
        value if value.starts_with("channel-in:")=>channel_form(a,"text",id(value.trim_start_matches("channel-in:"))),
        "invite"=>settings::invites(a),
        "profile-edit"=>profile_editor(a),
        "about"=>form("About Me","about",view!{{field("f-name","Pronouns",a.profile.as_ref().map_or("",|p|p.profile.pronouns.as_str()),"text")}<p class="muted-copy">"Optional. Let people know how to refer to you."</p><label class="field"><span>"About Me"</span><textarea id="f-value" rows="4" maxlength="190" aria-describedby="bio-help">{a.profile.as_ref().map_or("",|p|p.profile.about_me.as_str()).to_owned()}</textarea><small id="bio-help">"A little about you, up to 190 characters."</small></label>{field("f-extra","Banner Color",&format!("#{:06x}",a.profile.as_ref().map_or(0x496c63,|p|if p.profile.banner_color==0{0x496c63}else{p.profile.banner_color})),"color")}<p class="muted-copy">"Choose the color at the top of your profile."</p>}.into_any()),
        "password"=>form("Password & Security","password",view!{{field("f-name",if a.google{"Current Password (or verify with Google below)"}else{"Current Password"},"","password")}{field("f-value","New Password","","password")}{if a.google{button("Verify with Google","reauth-google:password","secondary")}else{().into_any()}}{if a.google{view!{<p class="muted-copy">"Google proof is valid for one password change within 5 minutes. Re-enter the new password after returning."</p>}.into_any()}else{().into_any()}}}.into_any()),
        "delete-account"=>form("Delete Account Permanently","delete-account",view!{<p>"This erases your messages and attachments and signs out all devices. Encrypted backups expire after 14 days. Your username is held for 30 days."</p>{field("f-name",if a.google{"Current Password (or verify with Google)"}else{"Current Password"},"","password")}{field("f-value","Type DELETE to confirm","","text")}{if a.google{button("Verify with Google","reauth-google:delete_account","secondary")}else{().into_any()}}}.into_any()),
        "devices"=>view!{<h2>"Devices"</h2>{a.devices.iter().map(|d|view!{<div class=DEVICE_ROW><span class="min-w-0 wrap-anywhere"><strong class="font-semibold">{d.device_name.clone()}</strong><small>{format!("{} {}",d.platform,if d.current{"· Current Device"}else{""})}</small></span>{button("Log Out",&format!("device-revoke:{}",d.id),"secondary")}</div>}).collect_view()}{button("Log Out All Other Devices","logout-others","danger-text")}}.into_any(),
        "media-settings"=>view!{<h2>"Voice & Video"</h2><div id="live-device-settings"></div>}.into_any(),
        "notifications"|"channel-notifications"=>settings::notifications(a),
        value if value.starts_with("target-notifications:")=>settings::notifications(a),
        "roles"=>settings::roles(a),
        "member-role"=>settings::member_roles(a),
        "private"=>form("Private Channel","private",view!{<p>"Restrict this channel to one role. The owner retains access."</p><label class="field">"Allowed Role"<select id="f-name">{a.roles.values().filter(|r|Some(r.guild_id)==a.guild&&r.id!=r.guild_id).map(|r|view!{<option value=r.id.0.to_string()>{r.name.clone()}</option>}).collect_view()}</select></label>}.into_any()),
        "pins"=>view!{<h2>"Pinned Messages"</h2>{if a.pins.is_empty(){view!{<div class=SETTINGS_EMPTY>{icon("pin")}<p>"No pinned messages yet."</p><small>"Open a message’s actions and choose Pin Message to keep it here."</small></div>}.into_any()}else{a.pins.iter().map(|m|message(a,m,false)).collect_view().into_any()}}}.into_any(),
        "edit"=>{let text=a.channel.and_then(|c|a.model.messages.get(&c)).and_then(|ms|a.editing.and_then(|m|ms.get(&m))).map_or("",|m|m.content.as_str());form("Edit Message","edit",field("f-name","Message",text,"text"))},
        "emoji"=>{let emojis=if a.query.is_empty(){sticktalk_emoji::all().take(80).collect()}else{sticktalk_emoji::search(&a.query,80)};view!{<h2>"Emoji"</h2><label class="search flex"><span class="sr-only">"Search Emoji"</span><input class="w-full" id="emoji-query" type="search" value=a.query.clone() placeholder="Search emoji" data-emoji-query/></label><div class="emoji-grid mt-4 grid grid-cols-8 gap-2 phone:grid-cols-6 touch:grid-cols-[repeat(auto-fit,minmax(44px,1fr))]">{emojis.into_iter().map(|e|view!{<button class="min-h-10 rounded-md text-center text-[28px] hover:bg-accent touch:min-h-11" title=e.name() aria-label=e.name() data-live=format!("emoji:{}",e.emoji())>{e.emoji()}</button>}).collect_view()}</div>}.into_any()},
        "gif" if matches!(a.gif_capability,GifCapability::Disabled)=>view!{<h2>"GIFs"</h2><p class="muted-copy">"GIF search is not enabled on this server. You can still upload an image or use emoji."</p><div class="empty-actions mt-6 flex flex-wrap gap-3 phone:[&>.button]:flex-1">{button("Upload an Image","pick-image","primary")}{button("Choose Emoji","dialog:emoji","secondary")}</div>}.into_any(),
        "gif"=>view!{<h2>"GIFs"</h2><form data-gif-search><label class="field"><span>"Search GIFs"</span><input id="gif-query" type="search"/></label><button class="button secondary" type="submit">"Search"</button></form><div class="live-gif-results mt-4 grid grid-cols-2 gap-3 [&_video]:max-w-full">{a.gifs.iter().map(|g|view!{<button data-live=format!("gif-send:{}",g.id)><video src=g.preview.url.clone() preload="none" muted loop playsinline controls width=g.preview.width height=g.preview.height aria-label=g.title.clone()></video><span>{g.title.clone()}</span></button>}).collect_view()}</div>}.into_any(),
        "profile"=>if let Some(p)=&a.profile{let u=&p.profile.user;view!{<div class="profile-banner -mx-4" style=format!("background:#{:06x}",if p.profile.banner_color==0{0x496c63}else{p.profile.banner_color})></div>{avatar(u,"large -mt-8 border-[6px] border-popover")}<h2>{name(u)}</h2><p>{format!("@{}",u.username)}</p><p>{p.profile.pronouns.clone()}</p><p>{p.profile.about_me.clone()}</p><p>{format!("{} mutual servers · {} mutual friends",p.mutual_servers.len(),p.mutual_friends.len())}</p>{button("Message",&format!("friend:dm,{}",u.id.0),"primary")}{button("Block",&format!("friend:block,{}",u.id.0),"danger-text")}}.into_any()}else{().into_any()},
        value if value.starts_with("role:")=>{let r=id(value.trim_start_matches("role:")).and_then(|id|a.roles.get(&id));if let Some(r)=r{form("Edit Role","role-save",view!{<fieldset disabled=!settings::can(a,Permissions::MANAGE_ROLES)>{field("f-name","Role Name",&r.name,"text")}{field("f-value","Color",&format!("#{:06x}",r.color),"color")}<input id="f-permissions" type="hidden" value=r.permissions.bits().to_string()/><div class="live-permissions my-4">{role_permissions(r)}</div><label class="live-checkbox"><input id="f-hoist" type="checkbox" checked=r.hoist/>"Display role members separately"</label><label class="live-checkbox"><input id="f-mentionable" type="checkbox" checked=r.mentionable/>"Allow anyone to mention this role"</label><input id="f-extra" type="hidden" value=r.id.0.to_string()/>{if r.id!=r.guild_id{button("Delete Role",&format!("role-delete:{}",r.id.0),"danger-text")}else{().into_any()}}</fieldset>}.into_any())}else{().into_any()}},
        value if value.starts_with("channel-edit:")=>settings::channel_editor(a,value),
        value if value.starts_with("channel-delete:")=>settings::channel_delete(a,value),
        value if value.starts_with("invite-revoke:")=>form("Revoke Invite","invite-revoke",view!{<p>"This invite link will stop working immediately."</p><input id="f-name" type="hidden" value=value.trim_start_matches("invite-revoke:").to_owned()/>}.into_any()),
        value if value.starts_with("unban:")=>form("Revoke Ban","unban",view!{<p>"This person will be able to join again with an invite."</p><input id="f-name" type="hidden" value=value.trim_start_matches("unban:").to_owned()/>}.into_any()),
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
        "Settings",
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

fn role_permissions(role: &Role) -> AnyView {
    let groups = [
        (
            "General",
            vec![
                ("View Channels", Permissions::VIEW_CHANNEL),
                ("Read History", Permissions::READ_HISTORY),
                ("Manage Server", Permissions::MANAGE_GUILD),
                ("Manage Channels", Permissions::MANAGE_CHANNELS),
                ("Manage Roles", Permissions::MANAGE_ROLES),
                ("Create Invites", Permissions::CREATE_INVITES),
            ],
        ),
        (
            "Text",
            vec![
                ("Send Messages", Permissions::SEND_MESSAGES),
                ("Attach Files", Permissions::ATTACH_FILES),
                ("Add Reactions", Permissions::ADD_REACTIONS),
                ("Manage Messages", Permissions::MANAGE_MESSAGES),
            ],
        ),
        (
            "Voice & Video",
            vec![
                ("Connect", Permissions::CONNECT),
                ("Speak", Permissions::SPEAK),
                ("Share Screen", Permissions::STREAM),
                ("Video", Permissions::VIDEO),
                ("Voice Activity", Permissions::USE_VOICE_ACTIVITY),
            ],
        ),
        (
            "Moderation",
            vec![
                ("Kick Members", Permissions::KICK_MEMBERS),
                ("Ban Members", Permissions::BAN_MEMBERS),
                ("Administrator", Permissions::ADMINISTRATOR),
            ],
        ),
    ];
    view!{{groups.into_iter().map(|(title,permissions)|view!{<section class="permission-group [&+&]:mt-6 [&+&]:border-t [&+&]:pt-6 [&_.live-checkbox]:border-b"><h3 class="category mb-1">{title}</h3>{permissions.into_iter().map(|(label,bit)|view!{<label class="live-checkbox"><span>{label}</span><input type="checkbox" checked=role.permissions.contains(bit) data-role-bit=bit.bits().to_string()/></label>}).collect_view()}</section>}).collect_view()}}.into_any()
}

#[lazy]
pub(super) fn load_dialog() -> fn(&App) -> AnyView {
    dialog
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
    const NAME: &str = "min-w-0 truncate text-[15px] font-medium [&_.icon]:ml-1 [&_.icon]:inline [&_.icon]:size-3.5 [&_.icon]:text-warning";
    if a.guild.is_none() {
        if let Some(c) = a.channel.and_then(|id| a.model.channels.get(&id)) {
            return view!{<h2 class="category mx-4 mt-6 mb-2">{format!("Members — {}",c.recipient_ids.len())}</h2>{c.recipient_ids.iter().filter_map(|id|a.model.users.get(id)).map(|u|view!{<button class=ROW data-live=format!("profile:{}",u.id.0)>{avatar(u,"small")}<span class=NAME>{name(u)}</span></button>}).collect_view()}{if c.kind==ChannelKind::GroupDm {view!{<button class="nav-row mx-2 mt-2 flex w-[calc(100%-16px)] items-center rounded-md p-2 text-sm font-medium text-sidebar-foreground hover:bg-sidebar-accent touch:min-h-11" data-live="dialog:group-members">"Manage Group"</button>}.into_any()}else{().into_any()}}}.into_any();
        }
    }
    let mut groups: BTreeMap<(i64, String), Vec<&Member>> = BTreeMap::new();
    for m in a
        .model
        .members
        .values()
        .filter(|m| Some(m.guild_id) == a.guild)
    {
        let online = !matches!(
            a.model.presences.get(&m.user.id),
            None | Some(PresenceStatus::Offline | PresenceStatus::Invisible)
        );
        let hoisted = m
            .role_ids
            .iter()
            .filter_map(|id| a.roles.get(id))
            .filter(|r| r.hoist)
            .max_by_key(|r| r.position);
        let group = if !online {
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
            {members.into_iter().map(|m|view!{<button class=if offline{"member-row flex min-h-[42px] w-full items-center gap-3 px-4 py-1 opacity-30 hover:bg-sidebar-accent hover:opacity-100 touch:min-h-11 phone:opacity-70"}else{ROW} data-live=format!("profile:{}",m.user.id.0)>{avatar(&m.user,"small")}<span class=NAME data-member-name=m.user.id.0.to_string() style=format!("color:{}",color(a,m.user.id))>{m.nickname.clone().unwrap_or_else(||name(&m.user))}{if m.role==MemberRole::Owner {icon("crown")} else {().into_any()}}</span></button>}).collect_view()}
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
    ) || page.starts_with("target-notifications:")
}
fn settings_nav(a: &App) -> AnyView {
    let u = &a.model.me.as_ref().unwrap().user;
    view!{<div class="settings-identity flex items-center gap-3 px-2 pt-2 pb-6">{avatar(u,"settings-avatar")}<span class="flex min-w-0 flex-col gap-1"><strong class="text-lg leading-tight font-semibold wrap-anywhere">{name(u)}</strong><small>{format!("@{}",u.username)}</small><button class="text-left text-[13px] text-primary hover:underline touch:min-h-11" data-live="dialog:profile-edit">"Edit Profile"</button></span></div>
        <label class="settings-search"><span class="sr-only">"Search settings"</span><input class="mb-4 w-full" type="search" placeholder="Search settings" data-settings-search/></label>
        <h2 class="category mx-2 mt-6 mb-2">"User Settings"</h2>
        {[( "user-round","My Account","account"),("user-round","Profiles","profile-edit"),("shield","Privacy & Safety","privacy"),("shield","Password & Security","password"),("monitor","Sessions","devices"),("bell","Notifications","notifications")].into_iter().map(|(glyph,label,page)|settings_nav_row(glyph,label,&format!("dialog:{page}"),a.dialog==page)).collect_view()}
        <h2 class="category mx-2 mt-6 mb-2">"App Settings"</h2>
        {settings_nav_row("headphones","Voice & Video","dialog:media-settings",a.dialog=="media-settings")}
        {settings_nav_row("monitor","Appearance","dialog:appearance",a.dialog=="appearance")}
        <div class="settings-divider my-6 phone:mb-4"></div><button class="settings-nav-row danger-text flex min-h-9 w-full items-center gap-3 rounded-md px-3 py-2 text-left text-sm font-medium text-destructive hover:bg-destructive/10 touch:min-h-12 touch:text-[15px] phone:mt-0.5 phone:min-h-[52px] phone:bg-accent phone:p-3" data-live="logout">{icon("log-out")}<span class="flex-1">"Log Out"</span></button><small class="muted-copy px-3 pt-4">{concat!("StickTalk ",env!("CARGO_PKG_VERSION"))}</small>
    }.into_any()
}
fn settings_layout(a: &App, body: AnyView) -> AnyView {
    let title = match a.dialog.as_str() {
        "account" => "My Account",
        "profile-edit" => "Profiles",
        "about" => "About Me",
        "password" => "Password & Security",
        "devices" => "Sessions",
        "appearance" => "Appearance",
        "privacy" => "Privacy & Safety",
        "delete-account" => "Delete Account",
        "media-settings" => "Voice & Video",
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
            back: ("Back to Settings", "dialog:settings"),
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
    view!{<h2>"My Account"</h2><section class="account-card overflow-hidden rounded-xl border bg-card"><div class="profile-banner"></div><div class="account-identity flex flex-wrap items-center gap-4 px-4 pb-4 phone:gap-3">{avatar(me,"large -mt-6 border-[6px] border-card")}<div><strong class="text-xl font-semibold">{name(me)}</strong><p class="mt-1 text-[13px] text-subtle">{format!("@{}",me.username)}</p></div><span class="ml-auto phone:ml-0">{button("Edit Profile","dialog:profile-edit","secondary")}</span></div><div class=FIELD_ROW><span class="min-w-0 wrap-anywhere"><strong class="font-semibold">"Display Name"</strong><p class="mt-1 text-subtle">{name(me)}</p></span>{button("Edit","dialog:profile-edit","secondary")}</div><div class=FIELD_ROW><span class="min-w-0 wrap-anywhere"><strong class="font-semibold">"Username"</strong><p class="mt-1 text-subtle">{me.username.clone()}</p></span></div></section><div class="settings-divider"></div><h2>"Password & Security"</h2>{button("Change Password","dialog:password","secondary")}{if a.google{view!{<h2>"Google Sign-in"</h2>{field("google-current-password","Current Password (or verify with Google first)","","password")}{button("Verify with Google to Link","reauth-google:link","secondary")}{button("Link Google","google-link","secondary")}{button("Verify with Google to Unlink","reauth-google:unlink","secondary")}{button("Unlink Google","google-unlink","secondary")}}.into_any()}else{().into_any()}}<div class="settings-divider"></div><h2>"Devices"</h2><p class="muted-copy">"Manage where you’re signed in."</p>{button("Manage Devices","dialog:devices","secondary")}<section class="danger-zone mt-8 border-t pt-6"><h3>"Danger Zone"</h3><p class="muted-copy">"Permanently delete your account and messages. This cannot be undone."</p>{button("Delete Account","dialog:delete-account","danger-text")}</section>}.into_any()
}
fn profile_editor(a: &App) -> AnyView {
    let me = &a.model.me.as_ref().unwrap().user;
    let profile = a
        .profile
        .as_ref()
        .filter(|p| p.profile.user.id == me.id)
        .map(|p| &p.profile);
    let banner = profile.map_or(0x496c63, |p| p.banner_color);
    let banner = if banner == 0 { 0x496c63 } else { banner };
    view!{<h2>"Profiles"</h2><div class="profile-editor grid grid-cols-[minmax(0,1fr)_minmax(240px,280px)] items-start gap-6 narrow:grid-cols-[minmax(0,1fr)]">
        <aside class="profile-preview relative col-start-2 row-start-1 w-full self-start overflow-hidden rounded-xl border bg-card narrow:col-start-1 narrow:max-w-none" aria-label="Profile preview"><div class="profile-banner h-24 flex-none" style=format!("background:#{banner:06x}")></div>{avatar(me,"large relative -mt-8 ml-4 flex border-4 border-card")}<div class="profile-preview-copy px-4 pt-3 pb-6 wrap-anywhere phone:pb-4"><h2 class="leading-tight wrap-anywhere" data-preview-name>{name(me)}</h2><p class="profile-username mt-1 mb-2 text-[13px] text-subtle">{format!("@{}",me.username)}</p><p class="whitespace-pre-wrap wrap-anywhere empty:hidden" data-preview-status>{me.status_text.clone().unwrap_or_default()}</p><div class="settings-divider my-4"></div><h3>"About Me"</h3><p class=if profile.is_none_or(|p|p.about_me.is_empty()){"muted-copy whitespace-pre-wrap wrap-anywhere"}else{"whitespace-pre-wrap wrap-anywhere"}>{profile.map_or("No bio yet.",|p|if p.about_me.is_empty(){"No bio yet."}else{p.about_me.as_str()}).to_owned()}</p></div></aside>
        <div class="col-start-1 row-start-1 min-w-0 narrow:row-start-2"><div class="avatar-edit mb-6 flex items-center gap-4 phone:[&>.avatar]:hidden">{avatar(me,"large")}<div class="flex flex-col items-start gap-2"><label class="button secondary" for="live-avatar">"Change Avatar"</label><input type="file" id="live-avatar" accept="image/png,image/jpeg,image/webp" hidden/>{if me.avatar_id.is_some(){button("Remove Avatar","avatar-remove","danger-text")}else{().into_any()}}</div></div>
        {if a.uploading{view!{<p role="status">{a.upload_progress.clone()}</p>}.into_any()}else{().into_any()}}
        <form data-live-form="profile" data-profile-editor data-fallback-name=me.username.clone()><div class="field"><label for="f-name">"Display Name"</label><input id="f-name" type="text" maxlength="64" value=me.display_name.clone().unwrap_or_default() aria-describedby="display-name-help"/><small id="display-name-help">"How you appear in conversations. Leave blank to use your username."</small></div><div class="field"><label for="f-value">"Status"</label><input id="f-value" type="text" maxlength="128" value=me.status_text.clone().unwrap_or_default() aria-describedby="status-help"/><small id="status-help">"Let friends know what you’re up to."</small></div>
        <div class="profile-save-bar fixed right-[max(16px,calc((100vw-280px-700px)/2+16px))] bottom-[calc(16px+var(--safe-bottom))] left-[max(296px,calc((100vw-280px-700px)/2+296px))] z-[5] flex items-center gap-2 rounded-lg border bg-rail px-4 py-3 shadow-lg phone:right-2 phone:bottom-[calc(8px+var(--safe-bottom))] phone:left-2 phone:flex-wrap phone:p-3" hidden><span class="flex-1 text-[13px] phone:basis-full" role="status">"You have unsaved changes."</span><button type="button" class="button link-button phone:flex-1" data-profile-reset>"Reset"</button><button type="submit" class="button primary phone:flex-1">"Save changes"</button></div></form>
        <div class="settings-divider"></div><h3>"About Me & Banner"</h3><p class="muted-copy">"Add a bio, pronouns and a splash of color to your profile."</p>{button("Edit About Me","dialog:about","secondary")}</div></div>
    }.into_any()
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
