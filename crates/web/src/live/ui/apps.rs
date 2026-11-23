//! Views for apps: the Developer section, the install consent screen, Server
//! Settings › Integrations, channel webhooks, embeds and components on app
//! messages, ephemeral answers, forms bots open, and the `/` picker.
//!
//! Only the small hooks at the top are in the first load; they render
//! through `Views`, which `load_views` brings in the first time an app
//! message, a picker or one of these pages is shown.
use super::*;
use crate::live::apps::{self as behavior, Row};
use sticktalk_proto::Component;

/// Permissions an app can ask for, in the editor and the consent screen.
pub(in crate::live) const PERMISSIONS: &[(&str, u64)] = &[
    ("View Channels", Permissions::VIEW_CHANNEL.bits()),
    ("Send Messages", Permissions::SEND_MESSAGES.bits()),
    ("Read Message History", Permissions::READ_HISTORY.bits()),
    ("Embed Links", Permissions::EMBED_LINKS.bits()),
    ("Attach Files", Permissions::ATTACH_FILES.bits()),
    ("Add Reactions", Permissions::ADD_REACTIONS.bits()),
    ("Mention @everyone", Permissions::MENTION_EVERYONE.bits()),
    (
        "Send Messages in Threads",
        Permissions::SEND_MESSAGES_IN_THREADS.bits(),
    ),
    (
        "Create Public Threads",
        Permissions::CREATE_PUBLIC_THREADS.bits(),
    ),
    ("Manage Messages", Permissions::MANAGE_MESSAGES.bits()),
    ("Manage Threads", Permissions::MANAGE_THREADS.bits()),
    ("Manage Channels", Permissions::MANAGE_CHANNELS.bits()),
    ("Manage Roles", Permissions::MANAGE_ROLES.bits()),
    ("Manage Nicknames", Permissions::MANAGE_NICKNAMES.bits()),
    ("Change Nickname", Permissions::CHANGE_NICKNAME.bits()),
    ("Create Invites", Permissions::CREATE_INVITES.bits()),
    ("Manage Webhooks", Permissions::MANAGE_WEBHOOKS.bits()),
    ("Kick Members", Permissions::KICK_MEMBERS.bits()),
    ("Ban Members", Permissions::BAN_MEMBERS.bits()),
    ("Time Out Members", Permissions::MODERATE_MEMBERS.bits()),
    ("View Audit Log", Permissions::VIEW_AUDIT_LOG.bits()),
    ("Manage Server", Permissions::MANAGE_GUILD.bits()),
    ("Administrator", Permissions::ADMINISTRATOR.bits()),
];
pub(in crate::live) const INTENTS: &[(&str, &str, u64)] = &[
    (
        "Messages",
        "Message created, edited and deleted events.",
        Intents::GUILD_MESSAGES.bits() as u64,
    ),
    (
        "Message Content",
        "The text, files and embeds of messages. Each server grants it when adding the app.",
        Intents::MESSAGE_CONTENT.bits() as u64,
    ),
    (
        "Server Members",
        "Member joins, leaves and changes, and the member list.",
        Intents::GUILD_MEMBERS.bits() as u64,
    ),
    (
        "Reactions",
        "Reactions added and removed.",
        Intents::GUILD_MESSAGE_REACTIONS.bits() as u64,
    ),
    (
        "Typing",
        "Who is typing.",
        Intents::GUILD_MESSAGE_TYPING.bits() as u64,
    ),
    (
        "Moderation",
        "Bans and unbans.",
        Intents::GUILD_MODERATION.bits() as u64,
    ),
];

/// The lazily loaded renderers.
#[derive(Clone, Copy)]
pub(in crate::live) struct Views {
    page: fn(&App, &str) -> AnyView,
    extra: fn(&App, &Message) -> AnyView,
    interaction: fn(&App, &Message) -> AnyView,
    ephemeral: fn(&App, Id) -> AnyView,
    picker: fn(&App) -> AnyView,
}
#[lazy]
pub(in crate::live) fn load_views() -> Views {
    Views {
        page,
        extra: extra_view,
        interaction,
        ephemeral,
        picker,
    }
}

// ---------------------------------------------------------- first-load hooks

/// The APP tag beside a bot's or webhook's name.
pub(super) fn tag(u: &UserSummary) -> AnyView {
    if u.style.bot {
        view!{<span class="app-tag ml-1 inline-flex h-4 flex-none items-center rounded-sm bg-primary px-1 text-[10px] leading-none font-semibold text-primary-foreground" title="This account is an app">"APP"</span>}.into_any()
    } else {
        ().into_any()
    }
}
/// A webhook message's own name for its author.
pub(super) fn author(m: &Message, u: &mut UserSummary) {
    if let Some(name) = m.extra.app.as_ref().and_then(|x| x.username.clone()) {
        u.display_name = Some(name);
    }
}
/// A webhook message's own avatar (a stored copy), if it has one.
pub(super) fn avatar_override(m: &Message) -> Option<AnyView> {
    let url = m.extra.app.as_ref()?.avatar.clone()?;
    Some(view!{<img class="message-avatar size-10 rounded-full object-cover" src=url alt="" width="40" height="40"/>}.into_any())
}
pub(super) fn extra(a: &App, m: &Message) -> AnyView {
    if m.extra.app.is_none() {
        return ().into_any();
    }
    match behavior::views(a) {
        Some(v) => (v.extra)(a, m),
        None => view! {<div class="app-extra min-h-6" aria-busy="true"></div>}.into_any(),
    }
}
pub(super) fn interaction_line(a: &App, m: &Message) -> AnyView {
    if m.extra.app.as_ref().is_none_or(|x| x.interaction.is_none()) {
        return ().into_any();
    }
    match behavior::views(a) {
        Some(v) => (v.interaction)(a, m),
        None => ().into_any(),
    }
}
pub(super) fn ephemeral_list(a: &App, ch: Id) -> AnyView {
    if !a.apps.ephemeral.iter().any(|m| m.channel_id == ch) {
        return ().into_any();
    }
    match behavior::views(a) {
        Some(v) => (v.ephemeral)(a, ch),
        None => ().into_any(),
    }
}
pub(super) fn picker_view(a: &App) -> AnyView {
    if a.apps.picker.is_none() {
        return ().into_any();
    }
    match behavior::views(a) {
        Some(v) => (v.picker)(a),
        None => ().into_any(),
    }
}
pub(super) fn page_view(a: &App, name: &str) -> AnyView {
    match behavior::views(a) {
        Some(v) => (v.page)(a, name),
        None => view! {<p role="status">"Loading…"</p>}.into_any(),
    }
}
pub(super) fn settings_nav(a: &App) -> AnyView {
    settings_nav_row(
        "code",
        "Developer",
        "dialog:developer",
        behavior::is_user_page(&a.dialog),
    )
}
/// In channel settings: the way to this channel's webhooks.
pub(super) fn channel_section(a: &App, c: &ChannelSummary) -> AnyView {
    if !matches!(
        c.kind,
        ChannelKind::Text | ChannelKind::Announcement | ChannelKind::Forum
    ) || !settings::can(a, Permissions::MANAGE_WEBHOOKS)
    {
        return ().into_any();
    }
    view!{<section class="channel-webhooks mt-8 border-t pt-6"><h3 class="category mb-1">"Webhooks"</h3><p class="muted-copy mb-3">"Let other services post messages here, such as CI builds or alerts."</p>{button("Webhooks",&format!("dialog:channel-webhooks:{}",c.id.0),"secondary")}</section>}.into_any()
}
fn has(a: &App, kind: CommandKind) -> bool {
    a.channel.is_some_and(|ch| {
        a.apps.catalogs.iter().any(|c| {
            c.channel_id == ch
                && c.apps
                    .iter()
                    .any(|x| x.commands.iter().any(|c| c.kind == kind))
        })
    })
}
/// "Apps" on a message, when the channel's apps have message commands.
pub(super) fn message_actions(a: &App, m: &Message) -> AnyView {
    if !has(a, CommandKind::Message) {
        return ().into_any();
    }
    ib_plain(
        "layout-grid",
        "Apps",
        &format!("dialog:apps-target:message:{}", m.id.0),
    )
}
/// "Apps" on a member's profile, when the channel's apps have user commands.
pub(super) fn member_actions(a: &App, u: &UserSummary) -> AnyView {
    if !has(a, CommandKind::User) || u.style.bot {
        return ().into_any();
    }
    button(
        "Apps",
        &format!("dialog:apps-target:user:{}", u.id.0),
        "secondary",
    )
}

// ------------------------------------------------------------ lazy views

fn color_of(c: Option<u32>) -> String {
    c.map_or_else(String::new, |c| format!("border-left-color:#{c:06x}"))
}
fn link(url: &str, body: AnyView, class: &'static str) -> AnyView {
    view!{<a class=class href=url.to_owned() target="_blank" rel="noopener noreferrer nofollow">{body}</a>}.into_any()
}
fn embed(a: &App, m: &Message, e: &RichEmbed) -> AnyView {
    let inline: Vec<&EmbedField> = e.fields.iter().filter(|f| f.inline).collect();
    let block: Vec<&EmbedField> = e.fields.iter().filter(|f| !f.inline).collect();
    let field = |f: &EmbedField| view! {<div class="min-w-0"><p class="text-[13px] font-semibold text-foreground wrap-anywhere">{f.name.clone()}</p><div class="text-sm wrap-anywhere">{markdown(a,m,&f.value)}</div></div>};
    view!{<article class="app-embed relative grid max-w-[min(100%,520px)] gap-1 rounded-md border-l-4 border-primary bg-card p-3" style=color_of(e.color)>
        {e.thumbnail.clone().map(|t|view!{<img class="app-embed-thumbnail float-right ml-3 size-20 rounded-md object-cover" src=t.url width=t.width height=t.height alt=""/>})}
        {e.author.clone().map(|au|match au.url{Some(u)=>link(&u,view!{<span>{au.name.clone()}</span>}.into_any(),"text-xs font-semibold text-foreground hover:underline"),None=>view!{<p class="text-xs font-semibold text-foreground">{au.name}</p>}.into_any()})}
        {e.title.clone().map(|t|match e.url.clone(){Some(u)=>link(&u,view!{<span>{t.clone()}</span>}.into_any(),"font-semibold text-primary wrap-anywhere hover:underline"),None=>view!{<p class="font-semibold text-foreground wrap-anywhere">{t}</p>}.into_any()})}
        {e.description.clone().map(|d|view!{<div class="text-sm wrap-anywhere">{markdown(a,m,&d)}</div>})}
        {if inline.is_empty(){().into_any()}else{view!{<div class="mt-1 grid grid-cols-[repeat(auto-fill,minmax(140px,1fr))] gap-2">{inline.into_iter().map(field).collect_view()}</div>}.into_any()}}
        {block.into_iter().map(|f|view!{<div class="mt-1">{field(f)}</div>}).collect_view()}
        {e.image.clone().map(|i|view!{<img class="app-embed-image mt-2 block h-auto max-h-[300px] max-w-full rounded-md object-contain" src=i.url loading="lazy" width=i.width height=i.height alt=""/>})}
        {if e.footer.is_some()||e.timestamp.is_some(){view!{<p class="mt-1 text-xs text-subtle">{e.footer.clone().map(|f|f.text).unwrap_or_default()}{if e.footer.is_some()&&e.timestamp.is_some(){" · "}else{""}}{e.timestamp.map(timestamp).unwrap_or_default()}</p>}.into_any()}else{().into_any()}}
    </article>}.into_any()
}
fn button_class(s: ButtonStyle) -> &'static str {
    match s {
        ButtonStyle::Primary => "button primary",
        ButtonStyle::Secondary | ButtonStyle::Link => "button secondary",
        ButtonStyle::Success => {
            "button app-success bg-online text-primary-foreground hover:opacity-90"
        }
        ButtonStyle::Danger => "button danger",
    }
}
/// Buttons and selects; `ch` and `m` say where a press goes.
fn components(a: &App, ch: Id, m: Id, rows: &[ActionRow]) -> AnyView {
    view!{<div class="app-components mt-2 flex flex-col gap-2">{rows.iter().enumerate().map(|(i,row)|view!{<div class="flex flex-wrap items-center gap-2">{row.components.iter().map(|c|match c{
        Component::Button(b)=>{
            let label=format!("{}{}",b.emoji.clone().map(|e|format!("{e} ")).unwrap_or_default(),b.label.clone().unwrap_or_default());
            if b.style==ButtonStyle::Link{
                let url=b.url.clone().unwrap_or_default();
                view!{<a class="button secondary app-link inline-flex items-center gap-1" href=url target="_blank" rel="noopener noreferrer nofollow" aria-disabled=b.disabled.to_string()>{label}{icon_with("external-link","icon size-4")}</a>}.into_any()
            }else{
                view!{<button class=format!("{} app-button",button_class(b.style)) disabled=b.disabled data-live=format!("apps:press,{},{},{}",ch.0,m.0,b.custom_id.clone().unwrap_or_default())>{label}</button>}.into_any()
            }
        }
        Component::Select(s)=>{
            let field=format!("apps-select-{}-{i}",m.0);
            let options:Vec<(String,String,bool)>=match s.kind{
                SelectKind::String=>s.options.iter().map(|o|(o.value.clone(),o.label.clone(),o.default)).collect(),
                SelectKind::User=>a.model.members.values().filter(|x|Some(x.guild_id)==a.guild).take(100).map(|x|(x.user.id.0.to_string(),name(&x.user),false)).collect(),
                SelectKind::Role=>a.roles.values().filter(|r|Some(r.guild_id)==a.guild&&r.id!=r.guild_id).map(|r|(r.id.0.to_string(),r.name.clone(),false)).collect(),
                SelectKind::Channel=>a.model.channels.values().filter(|c|c.guild_id==a.guild&&c.kind!=ChannelKind::Category).map(|c|(c.id.0.to_string(),format!("#{}",c.name.clone().unwrap_or_default()),false)).collect(),
            };
            view!{<form class="app-select flex w-full max-w-[min(100%,420px)] items-center gap-2" data-live-form=format!("apps-select:{}:{}:{i}",ch.0,m.0)>
                <input type="hidden" id=format!("apps-select-id-{}-{i}",m.0) value=s.custom_id.clone()/>
                <label class="sr-only" for=field.clone()>{s.placeholder.clone().unwrap_or_else(||"Choose".into())}</label>
                <select class="field min-w-0 flex-1" id=field multiple=s.max_values>1 disabled=s.disabled size=if s.max_values>1{"3"}else{"1"}>
                    {if s.max_values<=1{view!{<option value="" disabled=true selected=!options.iter().any(|o|o.2)>{s.placeholder.clone().unwrap_or_else(||"Make a selection".into())}</option>}.into_any()}else{().into_any()}}
                    {options.into_iter().map(|(v,l,d)|view!{<option value=v selected=d>{l}</option>}).collect_view()}
                </select>
                <button type="submit" class="button secondary" disabled=s.disabled>"Choose"</button>
            </form>}.into_any()
        }
    }).collect_view()}</div>}).collect_view()}</div>}.into_any()
}
fn extra_view(a: &App, m: &Message) -> AnyView {
    let Some(x) = m.extra.app.as_ref() else {
        return ().into_any();
    };
    view!{<div class="app-extra flex flex-col gap-2">{x.embeds.iter().map(|e|embed(a,m,e)).collect_view()}{if x.components.is_empty(){().into_any()}else{components(a,m.channel_id,m.id,&x.components)}}</div>}.into_any()
}
fn interaction(a: &App, m: &Message) -> AnyView {
    let Some(i) = m.extra.app.as_ref().and_then(|x| x.interaction.clone()) else {
        return ().into_any();
    };
    let who = a
        .model
        .users
        .get(&i.user_id)
        .map_or_else(|| "Someone".into(), name);
    view!{<p class="app-interaction flex min-w-0 items-center gap-1 text-[13px] text-subtle">{icon_with("reply","icon size-4 -scale-x-100")}<strong class="truncate font-semibold text-foreground">{who}</strong>{if i.kind==CommandKind::ChatInput{" used "}else{" used the app "}}<span class="font-medium text-primary">{if i.kind==CommandKind::ChatInput{format!("/{}",i.name)}else{i.name.clone()}}</span></p>}.into_any()
}
fn as_message(e: &EphemeralMessage) -> Message {
    Message {
        id: e.id,
        channel_id: e.channel_id,
        author_id: e.author.id,
        content: e.content.clone(),
        created_at: e.created_at,
        edited_at: None,
        reply_to: None,
        attachments: vec![],
        mention_ids: vec![],
        reactions: vec![],
        call: None,
        extra: MessageExtra {
            app: Some(Box::new(e.app.clone())),
            ..Default::default()
        },
    }
}
fn ephemeral(a: &App, ch: Id) -> AnyView {
    view!{<div class="app-ephemeral-list flex flex-col">{a.apps.ephemeral.iter().filter(|m|m.channel_id==ch).map(|e|{
        let m=as_message(e);
        let who=name(&e.author);
        view!{<article class="message app-ephemeral relative mt-4 flex gap-4 border-l-2 border-primary bg-primary/5 px-4 py-1" data-ephemeral=e.id.0.to_string() aria-label=format!("Only you can see this message from {who}")>
            <div class="avatar-gutter flex w-10 flex-none items-start justify-center">{avatar(&e.author,"message-avatar")}</div>
            <div class="message-content min-w-0 flex-1 wrap-anywhere">
                {interaction(a,&m)}
                <div class="message-meta flex min-h-[21px] items-baseline gap-2"><span class="author font-semibold">{who.clone()}</span>{tag(&e.author)}<time class="text-xs">{timestamp(e.created_at)}</time></div>
                {if e.thinking{view!{<p class="app-thinking text-sm text-subtle" role="status">{format!("{who} is thinking…")}</p>}.into_any()}else if e.content.trim().is_empty(){().into_any()}else{markdown(a,&m,&e.content)}}
                {extra_view(a,&m)}
                <p class="mt-1 flex flex-wrap items-center gap-1 text-xs text-subtle">{icon_with("eye","icon size-3.5")}"Only you can see this ·"<button class="text-primary hover:underline touch:min-h-11" data-live=format!("apps:dismiss,{}",e.id.0)>"Dismiss message"</button></p>
            </div>
        </article>}
    }).collect_view()}</div>}.into_any()
}
fn picker(a: &App) -> AnyView {
    let Some(p) = a.apps.picker.as_ref() else {
        return ().into_any();
    };
    if p.rows.is_empty() {
        if p.hint.is_empty() {
            return ().into_any();
        }
        return view!{<div class="command-hint absolute inset-x-4 bottom-full z-40 mb-1 rounded-lg border bg-popover px-3 py-2 text-sm text-subtle shadow-lg wrap-anywhere" role="status">{p.hint.clone()}</div>}.into_any();
    }
    let mut last = String::new();
    let mut out = vec![];
    for (i, r) in p.rows.iter().enumerate() {
        if r.group != last {
            last = r.group.clone();
            out.push(view!{<p class="category px-2 pt-2 pb-1" role="presentation">{r.group.clone()}</p>}.into_any());
        }
        out.push(row(i, r));
    }
    view!{<div id="live-completions" class="autocomplete command-picker absolute inset-x-4 bottom-full z-40 max-h-[min(360px,55vh)] overflow-y-auto rounded-lg border bg-popover p-2 shadow-lg" role="listbox" aria-label="Commands">
        {out}
        {if p.hint.is_empty(){().into_any()}else{view!{<p class="border-t px-2 pt-2 mt-1 text-xs text-subtle wrap-anywhere" role="presentation">{p.hint.clone()}</p>}.into_any()}}
    </div>}.into_any()
}
fn row(i: usize, r: &Row) -> AnyView {
    view!{<button class="flex min-h-9 w-full items-center gap-3 rounded-sm px-2 py-1 text-left text-sm aria-selected:bg-secondary touch:min-h-11" id=format!("completion-{i}") role="option" aria-selected=if i==0{"true"}else{"false"} data-completion=r.insert.clone()><span class="font-semibold text-foreground">{r.label.clone()}</span><small class="ml-auto truncate text-subtle">{r.detail.clone()}</small></button>}.into_any()
}

// ---------------------------------------------------------------- pages

fn page(a: &App, name: &str) -> AnyView {
    if name == "developer" {
        return developer(a);
    }
    if let Some(app) = name.strip_prefix("developer-app:").and_then(id) {
        return developer_app(a, app);
    }
    if let Some(app) = name.strip_prefix("developer-delete:").and_then(id) {
        let label = a
            .apps
            .developer
            .as_ref()
            .and_then(|d| d.apps.iter().find(|x| x.id == app))
            .map_or("this app".into(), |x| x.name.clone());
        return form_with("Delete App","apps-delete",view!{<p>{format!("Delete {label}? It leaves every server it is in, its bot signs out and its token stops working. Its messages stay.")}</p><input type="hidden" id="apps-id" value=app.0.to_string()/>}.into_any(),&format!("dialog:developer-app:{}",app.0),true);
    }
    match name {
        "app-install" => return install(a),
        "server-integrations" => return integrations(a),
        "apps-modal" => return modal_form(a),
        _ => {}
    }
    if let Some(app) = name.strip_prefix("integration-app:").and_then(id) {
        return integration_app(a, app);
    }
    if let Some(app) = name.strip_prefix("integration-remove:").and_then(id) {
        let label = installed(a, app).map_or("this app".into(), |i| i.name.clone());
        return form_with("Remove App","apps-remove",view!{<p>{format!("Remove {label} from this server? Its bot leaves, and its role and commands are removed. Its messages stay.")}</p><input type="hidden" id="apps-id" value=app.0.to_string()/>}.into_any(),&format!("dialog:integration-app:{}",app.0),true);
    }
    if let Some(ch) = name.strip_prefix("channel-webhooks:").and_then(id) {
        return webhooks(a, ch);
    }
    if let Some(w) = name.strip_prefix("webhook-delete:").and_then(id) {
        let back = a
            .apps
            .webhooks
            .as_ref()
            .map_or("dialog:server-integrations".into(), |(c, _)| {
                format!("dialog:channel-webhooks:{}", c.0)
            });
        return form_with("Delete Webhook","apps-webhook-delete",view!{<p>"Its URL stops working at once. Messages it posted stay."</p><input type="hidden" id="apps-webhook-id" value=w.0.to_string()/>}.into_any(),&back,true);
    }
    if let Some(rest) = name.strip_prefix("apps-target:") {
        return target(a, rest);
    }
    ().into_any()
}

fn permission_names(p: Permissions) -> String {
    let names: Vec<&str> = PERMISSIONS
        .iter()
        .filter(|(_, b)| p.contains(Permissions::from_bits_retain(*b)))
        .map(|(n, _)| *n)
        .collect();
    if names.is_empty() {
        "No permissions".into()
    } else {
        names.join(", ")
    }
}

fn developer(a: &App) -> AnyView {
    let Some(d) = a.apps.developer.as_ref() else {
        return view! {<h2>"Developer"</h2><p role="status">"Loading…"</p>}.into_any();
    };
    view!{<h2>"Developer"</h2><p class="muted-copy">"Apps add bots, slash commands and buttons to servers. Their bot acts only with its token, and only with the permissions a server grants it. See docs/BOTS.md for the API."</p>
        {if d.can_create{form("New App","apps-new",view!{{field("apps-name","Name","","text")}<p class="muted-copy">"Up to 32 characters. You can change it later."</p>}.into_any())}else{view!{<div class=SETTINGS_EMPTY>{icon("lock")}<p>{match d.creation{AppCreation::Disabled=>"Apps are turned off on this StickTalk server.",_=>"The administrator of this StickTalk server hasn't let your account create apps."}}</p></div>}.into_any()}}
        <h3 class="category mt-8 mb-2">{format!("Your Apps — {}",d.apps.len())}</h3>
        {if d.apps.is_empty(){view!{<p class="muted-copy">"You haven't made an app yet."</p>}.into_any()}else{view!{<div class="grid gap-2">{d.apps.iter().map(|x|view!{<button class="app-card flex w-full items-center gap-3 rounded-lg border bg-card p-3 text-left hover:bg-accent touch:min-h-12" data-live=format!("dialog:developer-app:{}",x.id.0)>{avatar(&x.bot,"small")}<span class="flex min-w-0 flex-1 flex-col"><strong class="truncate font-semibold">{x.name.clone()}{tag(&x.bot)}</strong><small class="truncate text-subtle">{format!("{} · in {} server{}{}",if x.public{"Public"}else{"Private"},x.install_count,if x.install_count==1{""}else{"s"},if x.disabled{" · Turned off by the administrator"}else{""})}</small></span>{icon_with("chevron-right","icon size-4")}</button>}).collect_view()}</div>}.into_any()}}
    }.into_any()
}

fn developer_app(a: &App, app: Id) -> AnyView {
    let Some(x) = a
        .apps
        .developer
        .as_ref()
        .and_then(|d| d.apps.iter().find(|x| x.id == app))
        .cloned()
    else {
        return view! {<p role="status">"Loading…"</p>}.into_any();
    };
    let link = format!("{}/?app={}", a.origin.trim_end_matches('/'), x.id.0);
    let token = a
        .apps
        .token
        .as_ref()
        .filter(|(t, _)| *t == app)
        .map(|(_, t)| t.clone());
    view!{
        {token.map(|t|view!{<section class="app-token mb-6 rounded-lg border border-warning bg-warning/10 p-4" role="alert"><h3 class="font-semibold">"Your bot token"</h3><p class="my-2 text-sm">"Copy it now: it won't be shown again. Anyone with it can act as your bot. Reset it if it leaks."</p><code class="block rounded-md bg-field p-2 text-sm break-all" data-token="">{t.clone()}</code><div class="mt-3 flex flex-wrap gap-2"><button class="button primary" data-copy=t>"Copy Token"</button>{button("Done","apps:token-hide","secondary")}</div></section>})}
        <div class="mb-4 flex items-center gap-3">{avatar(&x.bot,"large")}<div class="min-w-0"><h2 class="mb-0 flex items-center">{x.name.clone()}{tag(&x.bot)}</h2><small class="text-subtle">{format!("@{} · in {} server{}",x.bot.username,x.install_count,if x.install_count==1{""}else{"s"})}</small></div></div>
        {if x.disabled{view!{<p class="mb-4 rounded-md bg-destructive/10 p-3 text-sm text-destructive" role="status">"The administrator turned this app off. Its bot can't connect and its commands are hidden."</p>}.into_any()}else{().into_any()}}
        <form data-live-form="apps-edit"><input type="hidden" id="apps-id" value=x.id.0.to_string()/>
            {field("apps-name","Name",&x.name,"text")}
            <div class="field"><label for="apps-description">"Description"</label><textarea id="apps-description" rows="3" maxlength="400">{x.description.clone()}</textarea></div>
            <div class="field"><span class="text-[13px] font-medium text-subtle">"Icon"</span><div class="flex flex-wrap items-center gap-2"><label class="button secondary" for=format!("apps-icon-{}",x.id.0)>"Upload Icon"</label><input class="sr-only" type="file" accept="image/png,image/jpeg,image/gif,image/webp" id=format!("apps-icon-{}",x.id.0) data-live-file=format!("apps:icon,app,{}",x.id.0)/>{if x.icon_id.is_some(){button("Remove Icon",&format!("apps:icon-clear,{}",x.id.0),"danger-text")}else{().into_any()}}</div></div>
            <label class="live-checkbox"><span>"Public: anyone with Manage Server may add it"</span><input type="checkbox" id="apps-public" checked=x.public/></label>
            <h3 class="category mt-6 mb-1">"Permissions the install link asks for"</h3>
            <div class="app-permissions grid gap-x-6 sm:grid-cols-2">{PERMISSIONS.iter().map(|(label,bit)|view!{<label class="live-checkbox"><span>{*label}</span><input type="checkbox" id=format!("apps-perm-{bit}") checked=x.permissions.contains(Permissions::from_bits_retain(*bit))/></label>}).collect_view()}</div>
            <h3 class="category mt-6 mb-1">"Events the bot receives (intents)"</h3>
            {INTENTS.iter().map(|(label,about,bit)|view!{<label class="live-checkbox items-start"><span class="flex flex-col"><span>{*label}</span><small class="text-subtle">{*about}</small></span><input type="checkbox" id=format!("apps-intent-{bit}") checked=x.intents.contains(Intents::from_bits_truncate(*bit as u32))/></label>}).collect_view()}
            <button type="submit" class="button primary mt-4">"Save Changes"</button>
        </form>
        <h3 class="category mt-8 mb-1">"Install Link"</h3><p class="muted-copy mb-2">"Share it with people who manage a server. They see exactly what the app asks for before adding it."</p>
        <div class="flex flex-wrap items-center gap-2"><code class="min-w-0 flex-1 rounded-md bg-field p-2 text-sm break-all">{link.clone()}</code><button class="button secondary" data-copy=link>"Copy Link"</button></div>
        <h3 class="category mt-8 mb-1">"Bot Token"</h3><p class="muted-copy mb-2">"Resetting makes a new token and disconnects the bot; the old one stops working at once."</p>
        <div class="flex flex-wrap gap-2">{button("Reset Token",&format!("apps:reset-token,{}",x.id.0),"secondary")}{button("Delete App",&format!("dialog:developer-delete:{}",x.id.0),"danger-text")}</div>
    }.into_any()
}

fn install(a: &App) -> AnyView {
    let preview = match &a.apps.install {
        None => return view! {<h2>"Add App"</h2><p role="status">"Loading…"</p>}.into_any(),
        Some(Err(e)) => return view! {<h2>"Add App"</h2><div class=SETTINGS_EMPTY>{icon("lock")}<p>{e.clone()}</p></div>}.into_any(),
        Some(Ok(p)) => p,
    };
    let x = &preview.app;
    let guilds: Vec<&InstallGuild> = preview.guilds.iter().filter(|g| !g.installed).collect();
    let admin = x.permissions.contains(Permissions::ADMINISTRATOR);
    let owner_somewhere = guilds.iter().any(|g| g.owner);
    let all_mine = guilds
        .iter()
        .map(|g| g.my_permissions)
        .fold(Permissions::empty(), |p, q| p | q);
    view!{<form class="app-install" data-live-form="apps-install"><input type="hidden" id="apps-id" value=x.id.0.to_string()/>
        <div class="flex items-center gap-3">{avatar(&x.bot,"large")}<div class="min-w-0"><h2 class="mb-0 flex items-center">{x.name.clone()}{tag(&x.bot)}</h2><small class="text-subtle">{format!("Made by {} · in {} server{}",name(&preview.owner),x.install_count,if x.install_count==1{""}else{"s"})}</small></div></div>
        {if x.description.is_empty(){().into_any()}else{view!{<p class="mt-3 text-sm">{x.description.clone()}</p>}.into_any()}}
        {if guilds.is_empty(){view!{<div class=SETTINGS_EMPTY>{icon("shield")}<p>"You need Manage Server in a server to add apps, and it may already be there."</p></div>}.into_any()}else{view!{
            <div class="field mt-4"><label for="apps-guild">"Add to Server"</label><select id="apps-guild">{guilds.iter().map(|g|view!{<option value=g.id.0.to_string()>{g.name.clone()}</option>}).collect_view()}</select></div>
            <h3 class="category mt-4 mb-1">"It will be able to"</h3>
            <p class="muted-copy mb-1">"Untick anything you don't want to grant. You can only grant permissions you have."</p>
            {PERMISSIONS.iter().filter(|(_,b)|x.permissions.contains(Permissions::from_bits_retain(*b))).map(|(label,bit)|{let mine=all_mine.contains(Permissions::from_bits_retain(*bit))||guilds.iter().any(|g|g.owner);view!{<label class="live-checkbox"><span>{*label}{if mine{"".to_owned()}else{" (you don't have this)".to_owned()}}</span><input type="checkbox" id=format!("apps-perm-{bit}") checked=mine&&*bit!=Permissions::ADMINISTRATOR.bits() disabled=!mine/></label>}}).collect_view()}
            {if x.permissions.is_empty(){view!{<p class="muted-copy">"It asks for no permissions: it joins with no role."</p>}.into_any()}else{().into_any()}}
            {if x.intents.contains(Intents::MESSAGE_CONTENT){view!{<label class="live-checkbox items-start mt-2"><span class="flex flex-col"><span>"Read message content"</span><small class="text-subtle">"Without this, it sees only messages that mention it."</small></span><input type="checkbox" id="apps-content"/></label>}.into_any()}else{().into_any()}}
            {if admin&&owner_somewhere{view!{<label class="live-checkbox items-start mt-2"><span class="flex flex-col"><span class="text-destructive">"I understand Administrator gives it every permission"</span><small class="text-subtle">"Only the server's owner can grant Administrator."</small></span><input type="checkbox" id="apps-admin"/></label>}.into_any()}else{().into_any()}}
            <p class="muted-copy mt-3">"You can remove it any time in Server Settings › Integrations."</p>
            <button type="submit" class="button primary mt-3">"Add App"</button>
        }.into_any()}}
    </form>}.into_any()
}

fn installed(a: &App, app: Id) -> Option<&Installation> {
    a.apps
        .integrations
        .as_ref()
        .filter(|(g, _)| Some(*g) == a.guild)
        .and_then(|(_, v)| v.apps.iter().find(|i| i.app_id == app))
}

fn integrations(a: &App) -> AnyView {
    let Some((_, list)) = a
        .apps
        .integrations
        .as_ref()
        .filter(|(g, _)| Some(*g) == a.guild)
    else {
        return view! {<h2>"Integrations"</h2><p role="status">"Loading…"</p>}.into_any();
    };
    let manage = settings::can(a, Permissions::MANAGE_GUILD);
    view!{<h2>"Integrations"</h2><p class="muted-copy">"Apps and webhooks in this server. Apps join with a role holding only what was granted when they were added."</p>
        {if manage{view!{<h3 class="category mt-6 mb-2">{format!("Apps — {}",list.apps.len())}</h3>
            {if list.apps.is_empty(){view!{<p class="muted-copy">"No apps yet. Open an app's install link to add one."</p>}.into_any()}else{view!{<div class="grid gap-2">{list.apps.iter().map(|i|view!{<button class="integration-app flex w-full items-center gap-3 rounded-lg border bg-card p-3 text-left hover:bg-accent touch:min-h-12" data-live=format!("dialog:integration-app:{}",i.app_id.0)>{avatar(&i.bot,"small")}<span class="flex min-w-0 flex-1 flex-col"><strong class="flex items-center truncate font-semibold">{i.name.clone()}{tag(&i.bot)}</strong><small class="truncate text-subtle">{format!("{} command{} · {}",i.commands.len(),if i.commands.len()==1{""}else{"s"},permission_names(i.permissions))}</small></span>{icon_with("chevron-right","icon size-4")}</button>}).collect_view()}</div>}.into_any()}}}.into_any()}else{().into_any()}}
        <h3 class="category mt-8 mb-2">{format!("Webhooks — {}",list.webhooks.len())}</h3>
        {if list.webhooks.is_empty(){view!{<p class="muted-copy">"No webhooks. Create one in a channel's settings."</p>}.into_any()}else{view!{<div class="grid gap-2">{list.webhooks.iter().map(|w|{let ch=a.model.channels.get(&w.channel_id).and_then(|c|c.name.clone()).unwrap_or_default();view!{<button class="integration-webhook flex w-full items-center gap-3 rounded-lg border bg-card p-3 text-left hover:bg-accent touch:min-h-12" data-live=format!("dialog:channel-webhooks:{}",w.channel_id.0)>{icon("link")}<span class="flex min-w-0 flex-1 flex-col"><strong class="truncate font-semibold">{w.name.clone()}</strong><small class="truncate text-subtle">{format!("#{ch}")}</small></span>{icon_with("chevron-right","icon size-4")}</button>}}).collect_view()}</div>}.into_any()}}
    }.into_any()
}

fn rule_chips(a: &App, target: Id, rules: &[CommandTarget]) -> AnyView {
    if rules.is_empty() {
        return view! {<p class="text-xs text-subtle">"No overrides: the defaults apply."</p>}
            .into_any();
    }
    view!{<div class="flex flex-wrap gap-2">{rules.iter().map(|t|{
        let label=match t.kind{TargetKind::Role=>a.roles.get(&t.id).map_or("a role".into(),|r|if r.id==r.guild_id{"@everyone".into()}else{format!("@{}",r.name)}),TargetKind::Channel=>a.model.channels.get(&t.id).map_or("a channel".into(),|c|format!("#{}",c.name.clone().unwrap_or_default()))};
        view!{<span class=format!("app-rule inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs {}",if t.allow{"text-foreground"}else{"text-destructive"})>{format!("{} {label}",if t.allow{"Allow"}else{"Deny"})}<button class="grid size-5 place-items-center rounded-full hover:bg-accent touch:size-8" aria-label=format!("Remove rule for {label}") data-live=format!("apps:rule-remove,{},{},{}",target.0,if t.kind==TargetKind::Role{"role"}else{"channel"},t.id.0)>{icon_with("x","icon size-3")}</button></span>}
    }).collect_view()}</div>}.into_any()
}

fn integration_app(a: &App, app: Id) -> AnyView {
    let Some(i) = installed(a, app) else {
        return view! {<p role="status">"Loading…"</p>}.into_any();
    };
    let rules = |id: Id| {
        i.overrides
            .iter()
            .find(|o| o.id == id)
            .map(|o| o.targets.clone())
            .unwrap_or_default()
    };
    let added_by = a
        .model
        .users
        .get(&i.installed_by)
        .map_or_else(|| "someone".into(), name);
    view!{<div class="mb-4 flex items-center gap-3">{avatar(&i.bot,"large")}<div class="min-w-0"><h2 class="mb-0 flex items-center">{i.name.clone()}{tag(&i.bot)}</h2><small class="text-subtle">{format!("Added by {added_by} {}",timestamp(i.installed_at))}</small></div></div>
        {if i.description.is_empty(){().into_any()}else{view!{<p class="mb-4 text-sm">{i.description.clone()}</p>}.into_any()}}
        <h3 class="category mb-1">"Permissions"</h3><p class="text-sm">{permission_names(i.permissions)}</p>
        <p class="muted-copy mt-1">{if i.message_content{"It may read message content in this server."}else{"It sees only messages that mention it."}}</p>
        <h3 class="category mt-6 mb-1">"All Commands"</h3><p class="muted-copy mb-2">"Rules for every command of this app. A command's own rules replace these."</p>
        {rule_chips(a,app,&rules(app))}
        <h3 class="category mt-6 mb-2">{format!("Commands — {}",i.commands.len())}</h3>
        {if i.commands.is_empty(){view!{<p class="muted-copy">"This app hasn't registered commands yet."</p>}.into_any()}else{view!{<div class="grid gap-3">{i.commands.iter().map(|c|view!{<div class="app-command rounded-lg border bg-card p-3"><p class="font-semibold">{match c.kind{CommandKind::ChatInput=>format!("/{}",c.name),CommandKind::User=>format!("{} (member menu)",c.name),CommandKind::Message=>format!("{} (message menu)",c.name)}}</p>{if c.description.is_empty(){().into_any()}else{view!{<p class="text-sm text-subtle">{c.description.clone()}</p>}.into_any()}}<div class="mt-2">{rule_chips(a,c.id,&rules(c.id))}</div></div>}).collect_view()}</div>}.into_any()}}
        <form class="mt-6" data-live-form="apps-rule"><h3 class="category mb-2">"Add a Rule"</h3>
            <div class="field"><label for="apps-target">"For"</label><select id="apps-target"><option value=app.0.to_string()>"All commands"</option>{i.commands.iter().map(|c|view!{<option value=c.id.0.to_string()>{if c.kind==CommandKind::ChatInput{format!("/{}",c.name)}else{c.name.clone()}}</option>}).collect_view()}</select></div>
            <div class="field"><label for="apps-rule-on">"Role or Channel"</label><select id="apps-rule-on"><optgroup label="Roles">{a.roles.values().filter(|r|Some(r.guild_id)==a.guild).map(|r|view!{<option value=format!("role:{}",r.id.0)>{if r.id==r.guild_id{"@everyone".into()}else{format!("@{}",r.name)}}</option>}).collect_view()}</optgroup><optgroup label="Channels">{a.model.channels.values().filter(|c|c.guild_id==a.guild&&c.kind!=ChannelKind::Voice).map(|c|view!{<option value=format!("channel:{}",c.id.0)>{format!("#{}",c.name.clone().unwrap_or_default())}</option>}).collect_view()}</optgroup></select></div>
            <div class="field"><label for="apps-rule-allow">"Rule"</label><select id="apps-rule-allow"><option value="deny">"Deny"</option><option value="allow">"Allow"</option></select></div>
            <button type="submit" class="button primary">"Add Rule"</button>
        </form>
        <div class="settings-divider my-6"></div>{button("Remove App",&format!("dialog:integration-remove:{}",app.0),"danger-text")}
    }.into_any()
}

fn webhooks(a: &App, ch: Id) -> AnyView {
    let channel = a
        .model
        .channels
        .get(&ch)
        .and_then(|c| c.name.clone())
        .unwrap_or_default();
    let list = a
        .apps
        .webhooks
        .as_ref()
        .filter(|(c, _)| *c == ch)
        .map(|(_, l)| l.clone());
    let secret = a.apps.secret.clone();
    view!{<h2>{format!("Webhooks in #{channel}")}</h2><p class="muted-copy">"A webhook has a secret URL: anything that can send it a POST can post here as the webhook, and nothing else. Keep the URL private; rotate it if it leaks."</p>
        {secret.map(|(w,url)|view!{<section class="webhook-secret mt-4 rounded-lg border border-warning bg-warning/10 p-4" role="alert" data-webhook=w.0.to_string()><h3 class="font-semibold">"Webhook URL"</h3><p class="my-2 text-sm">"Copy it now: it won't be shown again."</p><code class="block rounded-md bg-field p-2 text-sm break-all" data-webhook-url="">{url.clone()}</code><div class="mt-3 flex flex-wrap gap-2"><button class="button primary" data-copy=url>"Copy URL"</button>{button("Done","apps:secret-hide","secondary")}</div><p class="muted-copy mt-3">"Post JSON like {\"content\":\"Hello\"}. Slack- and GitHub-style payloads go to the URL with /slack or /github added."</p></section>})}
        <form class="mt-4" data-live-form="apps-webhook-new"><input type="hidden" id="apps-channel" value=ch.0.to_string()/><h3 class="category mb-2">"New Webhook"</h3>{field("apps-webhook-name","Name","","text")}<button type="submit" class="button primary">"Create Webhook"</button></form>
        <h3 class="category mt-8 mb-2">"Webhooks"</h3>
        {match list{None=>view!{<p role="status">"Loading…"</p>}.into_any(),Some(l) if l.is_empty()=>view!{<p class="muted-copy">"No webhooks in this channel yet."</p>}.into_any(),Some(l)=>view!{<div class="grid gap-3">{l.into_iter().map(|w|view!{<div class="webhook-row rounded-lg border bg-card p-3">
            <form class="flex flex-wrap items-end gap-2" data-live-form=format!("apps-webhook-save:{}",w.id.0)><div class="field mb-0 min-w-0 flex-1"><label for=format!("apps-webhook-name-{}",w.id.0)>"Name"</label><input id=format!("apps-webhook-name-{}",w.id.0) type="text" value=w.name.clone() maxlength="80"/></div><button type="submit" class="button secondary">"Save"</button></form>
            <p class="mt-2 text-xs text-subtle">{format!("Created {}{}",timestamp(w.created_at),w.last_used_at.map(|t|format!(" · last used {}",timestamp(t))).unwrap_or_default())}</p>
            <div class="mt-2 flex flex-wrap gap-2"><label class="button secondary" for=format!("apps-icon-{}",w.id.0)>"Avatar"</label><input class="sr-only" type="file" accept="image/png,image/jpeg,image/gif,image/webp" id=format!("apps-icon-{}",w.id.0) data-live-file=format!("apps:icon,webhook,{}",w.id.0)/>{button("Rotate URL",&format!("apps:rotate,{}",w.id.0),"secondary")}{button("Delete",&format!("dialog:webhook-delete:{}",w.id.0),"danger-text")}</div>
        </div>}).collect_view()}</div>}.into_any()}}
    }.into_any()
}

fn modal_form(a: &App) -> AnyView {
    let Some((_, app, _, m)) = a.apps.modal.as_ref() else {
        return view! {<p class="muted-copy">"This form is closed."</p>}.into_any();
    };
    let from = a
        .apps
        .catalogs
        .iter()
        .flat_map(|c| c.apps.iter())
        .find(|x| x.app_id == *app)
        .map(|x| x.name.clone());
    view!{<form class="app-modal" data-live-form="apps-modal"><h2>{m.title.clone()}</h2>{from.map(|f|view!{<p class="muted-copy mb-3">{format!("From {f}")}</p>})}
        {m.inputs.iter().enumerate().map(|(i,input)|{let fid=format!("apps-input-{i}");let max=input.max_length.unwrap_or(MAX_TEXT_INPUT_CHARS).to_string();view!{<div class="field"><label for=fid.clone()>{input.label.clone()}{if input.required{" *"}else{""}}</label>{if input.style==TextInputStyle::Paragraph{view!{<textarea id=fid rows="4" maxlength=max minlength=input.min_length.to_string() required=input.required placeholder=input.placeholder.clone().unwrap_or_default()>{input.value.clone().unwrap_or_default()}</textarea>}.into_any()}else{view!{<input id=fid type="text" maxlength=max minlength=input.min_length.to_string() required=input.required placeholder=input.placeholder.clone().unwrap_or_default() value=input.value.clone().unwrap_or_default()/>}.into_any()}}</div>}}).collect_view()}
        <div class="mt-4 flex flex-wrap gap-2"><button type="submit" class="button primary">"Submit"</button><button type="button" class="button secondary" data-live="close">"Cancel"</button></div>
    </form>}.into_any()
}

/// A user or message command chosen from a menu.
fn target(a: &App, rest: &str) -> AnyView {
    let (kind, target) = rest.split_once(':').unwrap_or_default();
    let kind = if kind == "user" {
        CommandKind::User
    } else {
        CommandKind::Message
    };
    let Some(target) = id(target) else {
        return ().into_any();
    };
    let commands: Vec<(String, &ApplicationCommand)> = a
        .apps
        .catalogs
        .iter()
        .filter(|c| Some(c.channel_id) == a.channel)
        .flat_map(|c| c.apps.iter())
        .flat_map(|x| {
            x.commands
                .iter()
                .filter(|c| c.kind == kind)
                .map(|c| (x.name.clone(), c))
        })
        .collect();
    view!{<h2>"Apps"</h2>
        {if commands.is_empty(){view!{<p class="muted-copy">"No apps here have commands for this."</p>}.into_any()}else{view!{<div class="grid gap-2">{commands.into_iter().map(|(app,c)|view!{<button class="flex w-full items-center gap-3 rounded-lg border bg-card p-3 text-left hover:bg-accent touch:min-h-12" data-live=format!("apps:target,{},{}",c.id.0,target.0)><span class="flex min-w-0 flex-1 flex-col"><strong class="truncate font-semibold">{c.name.clone()}</strong><small class="truncate text-subtle">{app}</small></span></button>}).collect_view()}</div>}.into_any()}}
    }.into_any()
}
