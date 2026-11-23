//! Privacy and notification views: the status menu and custom status editor,
//! message requests, the report dialog, the notification and privacy settings
//! pages, the server's Reports queue, profile additions and the collapsed
//! messages of ignored people. State and actions: `live/privacy.rs`.
use super::*;
use crate::live::privacy as p;

const ROW: &str = "settings-list-row flex flex-wrap items-center gap-3 border-b py-4";
const COPY: &str = "settings-row-copy flex min-w-24 flex-1 flex-col gap-1 wrap-anywhere";

fn status_name(s: PresenceStatus) -> &'static str {
    match s {
        PresenceStatus::Online => "Online",
        PresenceStatus::Idle => "Idle",
        PresenceStatus::Dnd => "Do Not Disturb",
        PresenceStatus::Invisible => "Invisible",
        PresenceStatus::Offline => "Offline",
    }
}
fn dot_class(s: PresenceStatus) -> &'static str {
    match s {
        PresenceStatus::Online => "bg-online",
        PresenceStatus::Idle => "bg-idle",
        PresenceStatus::Dnd => "bg-dnd",
        _ => "bg-offline",
    }
}
/// "until 3:45 PM" / "until Tue 9:00 AM".
fn until(ms: u64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(ms as f64));
    let today = js_sys::Date::new(&JsValue::from_f64(Browser.unix_ms() as f64));
    let same = d.get_date() == today.get_date() && d.get_month() == today.get_month();
    if same {
        format!("until {}", short_time(ms))
    } else {
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        format!(
            "until {} {}",
            DAYS[d.get_day() as usize % 7],
            short_time(ms)
        )
    }
}

/// A person's custom status as one line: emoji and text.
pub(super) fn status_line(u: &UserSummary) -> Option<String> {
    let emoji = u
        .status_emoji
        .as_deref()
        .filter(|e| !e.starts_with('<'))
        .unwrap_or("");
    let text = u.status_text.as_deref().unwrap_or("");
    let line = format!("{emoji} {text}").trim().to_owned();
    (!line.is_empty()).then_some(line)
}

/// The avatar in the user panel: it opens the status menu.
pub(super) fn status_trigger(a: &App, me: &UserSummary) -> AnyView {
    let s = p::shown_status(a);
    view!{<button class="status-trigger relative flex flex-none rounded-full touch:min-h-11 touch:min-w-11 touch:place-items-center" aria-label=format!("Set status, now {}",status_name(s)) title="Set Status" data-live="dialog:privacy-status">{avatar(me,"small")}<span class=format!("presence-dot absolute -right-0.5 -bottom-0.5 size-3 rounded-full border-2 border-panel {}",dot_class(s)) aria-hidden="true"></span></button>}.into_any()
}
/// In user settings (and the phone's You tab): your status, to change it.
pub(super) fn settings_status(a: &App) -> AnyView {
    let s = p::shown_status(a);
    let custom = a.model.me.as_ref().and_then(|m| status_line(&m.user));
    view!{<button class="settings-status flex w-full items-center gap-2 rounded-md px-2 py-2 text-left text-sm hover:bg-accent touch:min-h-12" data-live="dialog:privacy-status"><span class=format!("size-2.5 flex-none rounded-full {}",dot_class(s)) aria-hidden="true"></span><span class="min-w-0 flex-1 truncate">{custom.unwrap_or_else(||status_name(s).to_owned())}</span><small class="text-primary">"Set Status"</small></button>}.into_any()
}

fn status_menu(a: &App) -> AnyView {
    let current = p::status(a);
    let settings = a.privacy.settings.as_ref();
    let ends = settings.and_then(|s| s.status.until);
    let custom = a.model.me.as_ref().and_then(|m| status_line(&m.user));
    let clear_at = settings.and_then(|s| s.custom.clear_at);
    let options = [
        (PresenceStatus::Online, "Online", "You’re available."),
        (PresenceStatus::Idle, "Idle", "You’re away for a while."),
        (
            PresenceStatus::Dnd,
            "Do Not Disturb",
            "No sounds, desktop notifications, push or ringing.",
        ),
        (
            PresenceStatus::Invisible,
            "Invisible",
            "You appear offline, but can use StickTalk as usual.",
        ),
    ];
    view!{<h2>"Set Status"</h2>
        <div class="status-custom mb-4 flex items-center gap-2 rounded-lg border p-3"><span class="min-w-0 flex-1 truncate">{custom.clone().unwrap_or_else(||"No custom status".into())}{clear_at.map(|t|view!{<small class="block text-xs text-subtle">{format!("Clears {}",until(t))}</small>})}</span>{button(if custom.is_some(){"Edit"}else{"Set a Custom Status"},"dialog:privacy-custom","secondary")}{if custom.is_some(){ib_plain("x","Clear Custom Status","privacy:custom-clear")}else{().into_any()}}</div>
        <label class="field"><span>"For how long"</span><select id="p-status-for" aria-describedby="p-status-for-help">{p::DURATIONS.iter().map(|(m,label)|view!{<option value=m.to_string() selected=*m==0>{*label}</option>}).collect_view()}</select><small id="p-status-for-help">"Idle, Do Not Disturb and Invisible go back to Online after this."</small></label>
        <div class="status-options grid gap-1" role="group" aria-label="Status">
        {options.into_iter().map(|(s,label,help)|view!{<button class=format!("status-option flex items-center gap-3 rounded-md p-3 text-left hover:bg-accent touch:min-h-12 {}",if s==current{"bg-secondary"}else{""}) aria-pressed=(s==current).to_string() data-live=format!("privacy:status,{}",match s{PresenceStatus::Online=>"online",PresenceStatus::Idle=>"idle",PresenceStatus::Dnd=>"dnd",_=>"invisible"})><span class=format!("size-3 flex-none rounded-full {}",dot_class(s)) aria-hidden="true"></span><span class="flex min-w-0 flex-col"><strong class="font-semibold">{label}</strong><small class="text-subtle">{if s==current&&ends.is_some(){format!("{help} Back to Online {}.",until(ends.unwrap_or_default()))}else{help.to_owned()}}</small></span></button>}).collect_view()}
        </div>
    }.into_any()
}
const QUICK_EMOJI: [&str; 16] = [
    "💬", "🍜", "☕", "🎮", "📚", "💼", "🏖️", "✈️", "🤒", "😴", "🎧", "🏠", "🎉", "🔕", "🚗", "❤️",
];
fn custom_editor(a: &App) -> AnyView {
    let me = a
        .model
        .me
        .as_ref()
        .map(|m| m.user.clone())
        .unwrap_or_default();
    let emoji = me.status_emoji.clone().unwrap_or_default();
    view!{{form("Set a Custom Status","privacy-custom",view!{
        <label class="field"><span>"Emoji"</span><input id="p-status-emoji" type="text" maxlength="64" value=emoji.clone() aria-describedby="p-emoji-help" autocomplete="off"/><small id="p-emoji-help">"One emoji, or pick one below."</small></label>
        <div class="quick-emoji mb-4 grid grid-cols-8 gap-1" role="group" aria-label="Pick an emoji">{QUICK_EMOJI.iter().map(|e|view!{<button type="button" class="min-h-10 rounded-md text-xl hover:bg-accent touch:min-h-11" aria-label=format!("Use {e}") data-live=format!("privacy:pick-emoji,{e}")>{*e}</button>}).collect_view()}</div>
        <label class="field"><span>"What’s happening?"</span><input id="p-status-text" type="text" maxlength="128" value=me.status_text.clone().unwrap_or_default()/></label>
        <label class="field"><span>"Clear after"</span><select id="p-status-clear"><option value="30">"30 minutes"</option><option value="60">"1 hour"</option><option value="240" selected>"4 hours"</option><option value="today">"Today"</option><option value="week">"This week"</option><option value="never">"Don’t clear"</option><option value="custom">"Choose a time…"</option></select></label>
        <label class="field"><span>"Clear at (for “Choose a time…”)"</span><input id="p-status-at" type="datetime-local"/></label>
    }.into_any())}}.into_any()
}

/// The Message Requests entry above the direct messages: a quiet count.
pub(super) fn requests_row(a: &App) -> AnyView {
    let c = a
        .privacy
        .settings
        .as_ref()
        .map(|s| s.requests)
        .unwrap_or_default();
    if a.guild.is_some() || c.pending + c.spam == 0 {
        return ().into_any();
    }
    view!{<button class="nav-row message-requests flex w-full items-center justify-start gap-2 rounded-md p-2 text-sm font-medium text-sidebar-foreground hover:bg-sidebar-accent touch:min-h-11 phone:text-base" data-live="privacy:requests">{icon_with("mail","icon text-muted-foreground")}<span class="flex-1 text-left">"Message Requests"</span>{if c.pending>0{view!{<span class="live-unread quiet-count rounded-full bg-muted px-1.5 text-xs text-subtle" aria-label=format!("{} requests",c.pending)>{c.pending.to_string()}</span>}.into_any()}else{().into_any()}}</button>}.into_any()
}
fn request_card(a: &App, r: &MessageRequest) -> AnyView {
    let ch = r.channel_id.0;
    let u = r.user.id.0;
    view!{<article class="request-card border-b py-4" data-request=ch.to_string()><div class="flex items-center gap-3">{avatar(&r.user,"small")}<span class=COPY><strong class="font-semibold">{name(&r.user)}</strong><small>{format!("@{}{}",r.user.username,if r.mutual_servers.is_empty(){String::new()}else{format!(" · In {}",r.mutual_servers.iter().map(|g|g.name.clone()).collect::<Vec<_>>().join(", "))})}</small></span></div>
        <div class="request-messages my-3 grid gap-1 rounded-md bg-muted p-3 text-sm">{if r.messages.is_empty(){view!{<p class="muted-copy">"No messages yet."</p>}.into_any()}else{r.messages.iter().rev().map(|m|view!{<p class="wrap-anywhere">{crate::wire::snippet(&a.model.content(m))}</p>}).collect_view().into_any()}}</div>
        <div class="flex flex-wrap gap-2">{button("Accept",&format!("privacy:open-request,{ch}"),"primary")}{button("Ignore",&format!("privacy:decline,{ch}"),"secondary")}{button("Block and Report",&format!("privacy:block-report,{ch},{u}"),"danger-text")}</div></article>}.into_any()
}
fn requests_page(a: &App) -> AnyView {
    let body = match &a.privacy.requests {
        None => view!{<p class="muted-copy" role="status">"Loading…"</p>}.into_any(),
        Some(Err(e)) => view!{<p class="muted-copy" role="alert">{e.clone()}</p>}.into_any(),
        Some(Ok(r)) => view!{
            <h3 class="category mt-4 mb-2">{format!("Requests — {}",r.requests.len())}</h3>
            {if r.requests.is_empty(){view!{<p class="muted-copy">"No message requests."</p>}.into_any()}else{r.requests.iter().map(|x|request_card(a,x)).collect_view().into_any()}}
            <h3 class="category mt-6 mb-2">{format!("Spam — {}",r.spam.len())}</h3>
            {if r.spam.is_empty(){view!{<p class="muted-copy">"Nothing here."</p>}.into_any()}else{r.spam.iter().map(|x|request_card(a,x)).collect_view().into_any()}}
        }.into_any(),
    };
    view!{<h2>"Message Requests"</h2><p class="muted-copy">"People you don’t know who share a server with you. They aren’t told whether you accept, ignore or report them."</p>{body}<div class="settings-divider"></div>{button("Who Can Message Me","dialog:privacy","link-button")}}.into_any()
}

fn report_target_text(a: &App, t: &ReportTarget) -> String {
    match t {
        ReportTarget::Message {
            channel_id,
            message_id,
        } => {
            let author = a
                .model
                .messages
                .get(channel_id)
                .and_then(|m| m.get(message_id))
                .map(|m| a.model.name(m.author_id));
            format!(
                "A message from {}",
                author.unwrap_or_else(|| "someone".into())
            )
        }
        ReportTarget::User { user_id } => format!("{}’s profile", a.model.name(*user_id)),
        ReportTarget::Guild { guild_id } => format!(
            "The server {}",
            a.model
                .guilds
                .get(guild_id)
                .map_or("".into(), |g| g.guild.name.clone())
        ),
        ReportTarget::Activity { user_id, .. } => {
            format!("What {} did in an activity", a.model.name(*user_id))
        }
    }
}
fn report_dialog(a: &App) -> AnyView {
    let Some(t) = a.privacy.report else {
        return view! {<h2>"Report"</h2><p class="muted-copy">"Nothing to report."</p>}.into_any();
    };
    let who = match t {
        ReportTarget::Message { channel_id, .. } | ReportTarget::Activity { channel_id, .. }
            if a.model
                .channels
                .get(&channel_id)
                .is_some_and(|c| c.guild_id.is_some()) =>
        {
            "This server’s moderators"
        }
        _ => "This StickTalk server’s administrator",
    };
    form("Report","privacy-report",view!{<p>{report_target_text(a,&t)}</p><p class="muted-copy">{format!("{who} will see what you report as it is now, even if it is edited or deleted later. They won’t see who reported it.")}</p>
        <fieldset class="report-categories my-4 grid gap-1"><legend class="mb-2 text-[13px] font-medium text-subtle">"What’s wrong?"</legend>{ReportCategory::ALL.iter().enumerate().map(|(i,c)|view!{<label class="live-checkbox"><span>{c.label()}</span><input type="radio" name="p-category" value=p::category_name(*c) checked=i==0/></label>}).collect_view()}</fieldset>
        <label class="field"><span>"Anything else? (optional)"</span><textarea id="p-report-note" rows="3" maxlength="1000"></textarea></label>
    }.into_any())
}

/// Report a message (not your own) from its actions.
pub(super) fn message_actions(a: &App, m: &Message) -> AnyView {
    if a.model
        .me
        .as_ref()
        .is_some_and(|me| me.user.id == m.author_id)
    {
        return ().into_any();
    }
    ib_plain(
        "flag",
        "Report Message",
        &format!("privacy:report,message,{},{}", m.channel_id.0, m.id.0),
    )
}
/// An ignored person's message, collapsed until revealed.
pub(super) fn ignored_message(a: &App, m: &Message) -> Option<AnyView> {
    if !p::ignored(m.author_id) || a.privacy.revealed.contains(&m.id) {
        return None;
    }
    Some(view!{<article class="message ignored-message flex items-center gap-2 px-4 py-1 text-sm text-subtle" data-live-message=m.id.0.to_string() tabindex="0" aria-label="Ignored message">{icon_with("eye-off","icon size-4")}<span>"Ignored message"</span><button class="link-button text-primary hover:underline touch:min-h-11" data-live=format!("privacy:reveal,{}",m.id.0)>"Show"</button></article>}.into_any())
}

/// On someone's profile: the real name behind your nickname, your nickname
/// for them (friends), your note, Ignore and Report.
pub(super) fn profile_extras(a: &App, u: &UserSummary) -> AnyView {
    if a.model.me.as_ref().is_some_and(|me| me.user.id == u.id) {
        return ().into_any();
    }
    let friend = a
        .model
        .relationships
        .get(&u.id)
        .is_some_and(|r| r.kind == RelationshipKind::Friend);
    let nick = p::nickname(u.id);
    let real = u.display_name.clone().unwrap_or_else(|| u.username.clone());
    let note = a
        .privacy
        .notes
        .get(&u.id)
        .map(|n| n.note.clone())
        .unwrap_or_default();
    let ignored = p::ignored(u.id);
    let id = u.id.0;
    view!{<section class="profile-private mt-4 border-t pt-4">
        {nick.as_ref().map(|_|view!{<p class="text-sm text-subtle">{format!("Their name: {real}")}</p>})}
        {status_line(u).filter(|_|!ignored).map(|s|view!{<p class="profile-status text-sm">{s}</p>})}
        <form data-live-form="privacy-note" class="mt-3"><input id="p-note-user" type="hidden" value=id.to_string()/><label class="field mb-2"><span>"Note (only you can see it)"</span><textarea id="p-note" rows="2" maxlength="256" placeholder="Click to add a note">{note}</textarea></label><button type="submit" class="button secondary">"Save Note"</button></form>
        {if friend{view!{<form data-live-form="privacy-nickname" class="mt-3"><input id="p-nick-user" type="hidden" value=id.to_string()/><label class="field mb-2"><span>"Friend Nickname (only you see it)"</span><input id="p-nickname" type="text" maxlength="32" value=nick.clone().unwrap_or_default()/></label><button type="submit" class="button secondary">"Save Nickname"</button></form>}.into_any()}else{().into_any()}}
        <div class="mt-3 flex flex-wrap gap-2">{if ignored{button("Stop Ignoring",&format!("privacy:unignore,{id}"),"secondary")}else{button("Ignore",&format!("privacy:ignore,{id}"),"secondary")}}{button("Report Profile",&format!("privacy:report,user,{id}"),"danger-text")}</div>
    </section>}.into_any()
}

fn checkbox(label: &str, help: &str, id: &str, checked: bool) -> AnyView {
    view!{<label class="live-checkbox"><span class="flex flex-col"><span>{label.to_owned()}</span>{(!help.is_empty()).then(||view!{<small class="text-subtle">{help.to_owned()}</small>})}</span><input id=id.to_owned() type="checkbox" checked=checked/></label>}.into_any()
}
/// A switch that saves at once: `action` is sent with ",on" or ",off" (the new state).
fn toggle(label: &str, help: &str, action: &str, on: bool) -> AnyView {
    view!{<label class="live-checkbox"><span class="flex flex-col"><span>{label.to_owned()}</span>{(!help.is_empty()).then(||view!{<small class="text-subtle">{help.to_owned()}</small>})}</span><input type="checkbox" checked=on aria-label=label.to_owned() data-live=format!("{action},{}",if on{"off"}else{"on"})/></label>}.into_any()
}

/// The conversation the notifications page is about, from the page name or
/// the open conversation.
fn notify_target(a: &App) -> Option<(Option<Id>, Option<Id>)> {
    if let Some((kind, value)) = a
        .dialog
        .strip_prefix("target-notifications:")
        .and_then(|v| v.split_once(':'))
    {
        let target = id(value)?;
        return Some(if kind == "guild" {
            (Some(target), None)
        } else {
            (
                a.model.channels.get(&target).and_then(|c| c.guild_id),
                Some(target),
            )
        });
    }
    // The open conversation, else the open server, else the first server.
    if let Some(c) = a
        .channel
        .and_then(|c| a.model.channels.get(&c))
        .filter(|c| {
            matches!(
                c.kind,
                ChannelKind::Text | ChannelKind::Voice | ChannelKind::Direct | ChannelKind::GroupDm
            )
        })
    {
        return Some((c.guild_id, Some(c.id)));
    }
    a.guild
        .or_else(|| a.model.guilds.keys().next().copied())
        .map(|g| (Some(g), None))
}
fn scope_label(a: &App, guild: Option<Id>, channel: Option<Id>) -> String {
    match (guild, channel.and_then(|c| a.model.channels.get(&c))) {
        (Some(g), Some(c)) => format!(
            "#{} — {}",
            a.model.channel_name(c),
            a.model
                .guilds
                .get(&g)
                .map_or(String::new(), |g| g.guild.name.clone())
        ),
        (None, Some(c)) => a.model.channel_name(c),
        (Some(g), None) => a
            .model
            .guilds
            .get(&g)
            .map_or(String::new(), |g| g.guild.name.clone()),
        _ => String::new(),
    }
}
fn scope_form(a: &App, guild: Option<Id>, channel: Option<Id>) -> AnyView {
    let o = p::options(a, guild, channel);
    let s = &o.settings;
    let own = a
        .model
        .settings
        .notifications
        .contains_key(&(guild, channel));
    let server = channel.is_none();
    let muted_now = s.muted && s.mute_until.is_none_or(|t| t > Browser.unix_ms());
    let mute_state = if !muted_now {
        "Not muted".to_owned()
    } else {
        match s.mute_until {
            Some(t) => format!("Muted {}", until(t)),
            None => "Muted until you turn it back on".into(),
        }
    };
    let inherited = channel.is_some() && guild.is_some() && !own;
    let (scope, target) = match channel {
        Some(c) => ("channel", c.0),
        None => ("guild", guild.map_or(0, |g| g.0)),
    };
    let level = |l: NotificationLevel, value: &str, label: &str, help: &str| view! {<label class="live-checkbox"><span class="flex flex-col"><span>{label.to_owned()}</span><small class="text-subtle">{help.to_owned()}</small></span><input type="radio" name="p-level" value=value.to_owned() checked=s.level==l/></label>};
    form(&format!("Notifications: {}",scope_label(a,guild,channel)),"privacy-notify",view!{
        <input id="p-scope" type="hidden" value=scope/><input id="p-target" type="hidden" value=target.to_string()/>
        {inherited.then(||view!{<p class="muted-copy">{format!("This channel follows {} until you change it here.",if a.model.channels.get(&channel.unwrap_or_default()).is_some_and(|c|c.parent_id.is_some()){"the channel it is inside"}else{"the server"})}</p>})}
        <fieldset class="my-4 grid gap-1"><legend class="mb-2 text-[13px] font-medium text-subtle">"Notify me about"</legend>
            {level(NotificationLevel::All,"all","All messages","Every new message.")}
            {level(NotificationLevel::Mentions,"mentions","Only @mentions","Messages that mention you, and DMs.")}
            {level(NotificationLevel::Nothing,"nothing","Nothing","No notifications. Mentions still show a badge.")}
        </fieldset>
        {if channel.is_none_or(|c|a.model.channels.get(&c).is_some_and(|c|c.guild_id.is_some())){view!{
            {checkbox("Suppress @everyone and @here","They won’t notify, badge or reach your Inbox.","p-suppress-everyone",s.suppress_everyone)}
            {checkbox("Suppress all role @mentions","Mentions of a role you have won’t notify, badge or reach your Inbox.","p-suppress-roles",o.suppress_roles)}
        }.into_any()}else{().into_any()}}
        <label class="field mt-4"><span>{format!("Mute — {mute_state}")}</span><select id="p-mute"><option value="" selected>"Keep as it is"</option>{if muted_now{view!{<option value="off">"Unmute"</option>}.into_any()}else{().into_any()}}{p::MUTES.iter().map(|(v,label)|view!{<option value=*v>{*label}</option>}).collect_view()}</select></label>
        {if server&&guild.is_some(){view!{
            {checkbox("Hide muted channels","Muted channels leave the channel list until unmuted.","p-hide-muted",s.hide_muted_channels)}
            {checkbox("Mobile push notifications","Push to your phone and devices where StickTalk isn’t open.","p-mobile-push",o.mobile_push)}
        }.into_any()}else{().into_any()}}
        {saved_note(a,"privacy-notify")}
    }.into_any())
}
/// Channels of a server with their own settings, to edit or reset.
fn overrides(a: &App, g: Id) -> AnyView {
    let rows: Vec<_> = a
        .model
        .settings
        .notifications
        .values()
        .filter(|n| n.guild_id == Some(g) && n.channel_id.is_some())
        .filter_map(|n| a.model.channels.get(&n.channel_id?).map(|c| (c, n)))
        .collect();
    view!{<h3 class="category mt-8 mb-2">"Channel overrides"</h3>
        {if rows.is_empty(){view!{<p class="muted-copy">"No channel has its own settings. Open a channel’s bell to give it one; channels inside it follow it."</p>}.into_any()}else{rows.into_iter().map(|(c,n)|{let muted=n.muted&&n.mute_until.is_none_or(|t|t>Browser.unix_ms());view!{<div class=ROW><span class=COPY><strong class="font-semibold">{format!("#{}",a.model.channel_name(c))}</strong><small>{format!("{}{}",match n.level{NotificationLevel::All=>"All messages",NotificationLevel::Mentions=>"Only @mentions",NotificationLevel::Nothing=>"Nothing"},if muted{" · Muted"}else{""})}</small></span>{button("Edit",&format!("dialog:target-notifications:channel:{}",c.id.0),"secondary")}{if muted{button("Unmute",&format!("privacy:unmute,channel,{}",c.id.0),"link-button")}else{().into_any()}}</div>}}).collect_view().into_any()}}
    }.into_any()
}
fn global_section(a: &App) -> AnyView {
    let g = p::global(a);
    let radio = |name: &str, value: &str, label: &str, on: bool, action: String| view! {<label class="live-checkbox"><span>{label.to_owned()}</span><input type="radio" name=name.to_owned() value=value.to_owned() checked=on data-live=action/></label>};
    view!{<section class="global-notifications mt-8 border-t pt-6"><h2>"All Notifications"</h2>
        {toggle("Desktop notifications","Show a notification while StickTalk isn’t the window you’re using.","privacy:global,desktop",g.desktop)}
        <h3 class="category mt-6 mb-2">"Sounds"</h3>
        {toggle("Notification sounds","Turns every sound below on or off.","privacy:global,sounds",g.sounds)}
        <fieldset class="grid gap-1 pl-4" disabled=!g.sounds>
            {toggle("New messages","","privacy:global,sound_message",g.sound_message)}
            {toggle("Mentions","","privacy:global,sound_mention",g.sound_mention)}
            {toggle("Direct messages","","privacy:global,sound_direct",g.sound_direct)}
            {toggle("Reactions","","privacy:global,sound_reaction",g.sound_reaction)}
        </fieldset>
        <h3 class="category mt-6 mb-2">"Unread badge"</h3>
        <fieldset class="grid gap-1">{radio("p-badge","mentions","Mentions and direct messages",g.badge==BadgeMode::Mentions,"privacy:global,badge,mentions".into())}{radio("p-badge","all","Every unread conversation",g.badge==BadgeMode::All,"privacy:global,badge,all".into())}{radio("p-badge","off","No badge",g.badge==BadgeMode::Off,"privacy:global,badge,off".into())}</fieldset>
        <h3 class="category mt-6 mb-2">"Notify me about reactions"</h3>
        <fieldset class="grid gap-1">{radio("p-reactions","direct","In direct and group messages",g.reactions==ReactionAlerts::Direct,"privacy:global,reactions,direct".into())}{radio("p-reactions","all","On all my messages",g.reactions==ReactionAlerts::All,"privacy:global,reactions,all".into())}{radio("p-reactions","never","Never",g.reactions==ReactionAlerts::Never,"privacy:global,reactions,never".into())}</fieldset>
        <p class="muted-copy mt-4">"Do Not Disturb silences all of these, push and call ringing, wherever you’re signed in."</p>
    </section>}.into_any()
}
/// User Settings › Notifications, and a server's or conversation's settings.
pub(super) fn notifications(a: &App) -> AnyView {
    let target = notify_target(a);
    let picker = view! {<label class="field"><span>"Server or Conversation"</span><select data-notification-target aria-label="Server or conversation"><option value="" selected=target.is_none() disabled>"Choose a conversation"</option>
    {a.model.guilds.values().map(|g|{let v=format!("guild:{}",g.guild.id.0);view!{<option value=v.clone() selected=target==Some((Some(g.guild.id),None))>{g.guild.name.clone()}</option>}}).collect_view()}
    {a.model.channels.values().filter(|c|matches!(c.kind,ChannelKind::Text|ChannelKind::Voice|ChannelKind::Direct|ChannelKind::GroupDm)).map(|c|{let v=format!("channel:{}",c.id.0);view!{<option value=v selected=target.is_some_and(|t|t.1==Some(c.id))>{scope_label(a,c.guild_id,Some(c.id))}</option>}}).collect_view()}</select></label>};
    view!{
        {picker}
        {match target{Some((g,c))=>view!{{scope_form(a,g,c)}{match (g,c){(Some(g),None)=>overrides(a,g),_=>().into_any()}}}.into_any(),None=>view!{<p class="muted-copy">"Choose a server or start a conversation to customize its notifications."</p>}.into_any()}}
        {global_section(a)}
        {if a.push{view!{<section class="push-explainer mt-6 border-t pt-6"><h3>"Notifications when you’re away"</h3><p>"Allow StickTalk to notify you about DMs and mentions. No message text appears on your lock screen. You can turn this off in your browser settings."</p><button class="button secondary" data-push-enable>"Enable Browser Notifications"</button></section>}.into_any()}else{().into_any()}}
    }.into_any()
}

fn report_status(s: ReportStatus) -> &'static str {
    match s {
        ReportStatus::Open => "Open",
        ReportStatus::Dismissed => "Closed, no action",
        ReportStatus::Actioned => "Action taken",
    }
}
/// User Settings › Privacy & Safety.
pub(super) fn privacy_page(a: &App) -> AnyView {
    let settings = a.privacy.settings.clone().unwrap_or_default();
    let blocked = a
        .model
        .relationships
        .values()
        .filter(|r| r.kind == RelationshipKind::Blocked)
        .collect::<Vec<_>>();
    let members = settings.dm.allow == DmAllow::ServerMembers;
    let server_choice = |g: &GuildSummary| {
        let o = settings
            .dm
            .servers
            .iter()
            .find(|o| o.guild_id == g.guild.id)
            .map(|o| o.allow);
        let v = |x: &str| format!("privacy:dm-server,{},{x}", g.guild.id.0);
        view! {<div class=ROW><span class=COPY>{g.guild.name.clone()}</span><label class="field mb-0 w-48"><span class="sr-only">{format!("Direct messages from {}",g.guild.name)}</span><select data-dm-server=g.guild.id.0.to_string()><option value=v("default") selected=o.is_none()>"Use my default"</option><option value=v("allow") selected=o==Some(true)>"Allow requests"</option><option value=v("block") selected=o==Some(false)>"Don’t allow"</option></select></label></div>}
    };
    view!{<h2>"Privacy & Safety"</h2>
        <section class="dm-privacy"><h3 class="category mt-4 mb-2">"Who can message me"</h3>
            <fieldset class="grid gap-1"><label class="live-checkbox"><span class="flex flex-col"><span>"Friends only"</span><small class="text-subtle">"Only friends can start a direct message with you."</small></span><input type="radio" name="p-dm" checked=!members data-live="privacy:dm-allow,friends"/></label><label class="live-checkbox"><span class="flex flex-col"><span>"Friends and people in my servers"</span><small class="text-subtle">"Others arrive as message requests you can accept, ignore or report. They aren’t notified."</small></span><input type="radio" name="p-dm" checked=members data-live="privacy:dm-allow,server_members"/></label></fieldset>
            <h3 class="category mt-6 mb-2">"Per server"</h3>
            {if a.model.guilds.is_empty(){view!{<p class="muted-copy">"You aren’t in any servers."</p>}.into_any()}else{a.model.guilds.values().map(server_choice).collect_view().into_any()}}
            <p class="muted-copy mt-2">"A server can still pause direct messages between members who aren’t friends."</p>
            {button(&format!("Message Requests ({})",settings.requests.pending),"dialog:privacy-requests","secondary")}
        </section>
        <div class="settings-divider"></div><h3 class="category mb-2">"Ignored"</h3><p class="muted-copy">"Their messages are collapsed and they can’t notify you. They aren’t told, and can still see you."</p>
        {if settings.ignored.is_empty(){view!{<p class="muted-copy">"You aren’t ignoring anyone."</p>}.into_any()}else{settings.ignored.iter().map(|u|view!{<div class=ROW>{avatar(u,"small")}<span class=COPY>{name(u)}</span>{button("Stop Ignoring",&format!("privacy:unignore,{}",u.id.0),"secondary")}</div>}).collect_view().into_any()}}
        <div class="settings-divider"></div><h3 class="category mb-2">"Blocked Users"</h3>{if blocked.is_empty(){view!{<p class="muted-copy">"You haven’t blocked anyone."</p>}.into_any()}else{().into_any()}}
        {blocked.into_iter().map(|r|view!{<div class=ROW>{avatar(&r.user,"small")}<span class=COPY>{name(&r.user)}</span>{button("Unblock",&format!("friend:unblock,{}",r.user.id.0),"secondary")}</div>}).collect_view()}
        <div class="settings-divider"></div><h3 class="category mb-2">"My Reports"</h3>
        {match &a.privacy.reports{None=>view!{<p class="muted-copy" role="status">"Loading…"</p>}.into_any(),Some(Err(e))=>view!{<p class="muted-copy">{e.clone()}</p>}.into_any(),Some(Ok(list)) if list.is_empty()=>view!{<p class="muted-copy">"You haven’t reported anything."</p>}.into_any(),Some(Ok(list))=>list.iter().map(|r|view!{<div class=ROW data-report=r.id.0.to_string()><span class=COPY><strong class="font-semibold">{r.summary.clone()}</strong><small>{format!("{} · {}",r.category.label(),timestamp(r.created_at))}</small></span><span class="report-status text-sm">{report_status(r.status)}</span></div>}).collect_view().into_any()}}
        <div class="settings-divider"></div><h2>"Your Data"</h2><p class="muted-copy">"Messages are not stored in the offline app cache. Browser notifications never show message text on your lock screen."</p>{button("Manage Devices","dialog:devices","secondary")}
    }.into_any()
}

/// Server Settings › Reports, for the server's moderators.
fn server_reports(a: &App) -> AnyView {
    let can = |p: Permissions| settings::can(a, p);
    if !settings::can_any(
        a,
        Permissions::MANAGE_MESSAGES
            | Permissions::KICK_MEMBERS
            | Permissions::BAN_MEMBERS
            | Permissions::MODERATE_MEMBERS,
    ) {
        return view!{<h2>"Reports"</h2><p class="muted-copy">"You need a moderation permission to see reports."</p>}.into_any();
    }
    let closed = a.privacy.queue_closed;
    let tabs = view! {<div class="mb-4 flex gap-2" role="group" aria-label="Show">{[("open","Open",!closed),("closed","Closed",closed)].into_iter().map(|(v,label,on)|view!{<button class=format!("button {}",if on{"secondary"}else{"link-button"}) aria-pressed=on.to_string() data-live=format!("privacy:queue,{v}")>{label}</button>}).collect_view()}</div>};
    let list = match a.privacy.queue.as_ref().filter(|(g, _)| Some(*g) == a.guild) {
        None => view!{<p class="muted-copy" role="status">"Loading…"</p>}.into_any(),
        Some((_, Err(e))) => view!{<p class="muted-copy" role="alert">{e.clone()}</p>}.into_any(),
        Some((_, Ok(page))) if page.reports.is_empty() => view!{<div class=SETTINGS_EMPTY>{icon("flag")}<p>{if closed{"No closed reports."}else{"No open reports. Members can report a message from its actions."}}</p></div>}.into_any(),
        Some((_, Ok(page))) => page.reports.iter().map(|r|{
            let e = &r.evidence;
            let id = r.id.0;
            let is_message = matches!(r.target, ReportTarget::Message{..});
            view!{<article class="report-card border-b py-4" data-report=id.to_string()>
                <div class="flex flex-wrap items-baseline gap-2"><strong class="font-semibold">{r.category.label()}</strong><small class="text-subtle">{format!("Reported {}",timestamp(r.created_at))}</small>{r.action.clone().map(|x|view!{<span class="ml-auto text-sm">{format!("{} · {}",report_status(r.status),x.replace('_'," "))}</span>})}</div>
                <div class="report-evidence my-2 rounded-md bg-muted p-3 text-sm">{e.user.as_ref().map(|u|view!{<p class="font-semibold">{format!("{} (@{}){}",u.display_name.clone().unwrap_or_else(||u.username.clone()),u.username,e.channel_name.as_ref().map_or(String::new(),|c|format!(" in #{c}")))}</p>})}<p class="wrap-anywhere whitespace-pre-wrap">{e.content.clone()}</p>{(!e.attachments.is_empty()).then(||view!{<p class="text-subtle">{format!("Attachments: {}",e.attachments.join(", "))}</p>})}{e.sent_at.map(|t|view!{<small class="text-subtle">{format!("Sent {}{}",timestamp(t),if e.edited_at.is_some(){" (edited before the report)"}else{""})}</small>})}</div>
                {r.note.clone().map(|n|view!{<p class="text-sm">{format!("Reporter’s note: {n}")}</p>})}
                {if r.status==ReportStatus::Open{view!{<div class="mt-3 grid gap-2"><label class="field mb-0"><span>"Reason (audit log)"</span><input id=format!("p-reason-{id}") type="text" maxlength="512"/></label>
                    <div class="flex flex-wrap items-end gap-2">{button("Dismiss",&format!("privacy:resolve,{id},dismiss"),"secondary")}
                    {if is_message&&can(Permissions::MANAGE_MESSAGES){button("Delete Message",&format!("privacy:resolve,{id},delete"),"secondary")}else{().into_any()}}
                    {if can(Permissions::MODERATE_MEMBERS){view!{<label class="field mb-0 w-36"><span>"Time out for"</span><select id=format!("p-timeout-{id}")><option value="60">"1 minute"</option><option value="600">"10 minutes"</option><option value="3600" selected>"1 hour"</option><option value="86400">"1 day"</option><option value="604800">"1 week"</option></select></label>{button("Time Out",&format!("privacy:resolve,{id},timeout"),"secondary")}}.into_any()}else{().into_any()}}
                    {if can(Permissions::KICK_MEMBERS){button("Kick",&format!("privacy:resolve,{id},kick"),"danger-text")}else{().into_any()}}
                    {if can(Permissions::BAN_MEMBERS){button("Ban",&format!("privacy:resolve,{id},ban"),"danger-text")}else{().into_any()}}</div></div>}.into_any()}else{().into_any()}}
            </article>}
        }).collect_view().into_any(),
    };
    view!{<h2>"Reports"</h2><p class="muted-copy">"Reports from members about messages in this server. Reporters are never shown. Every action is recorded in the audit log."</p>{tabs}{list}}.into_any()
}

pub(super) fn owns(page: &str) -> bool {
    matches!(
        page,
        "privacy-status"
            | "privacy-custom"
            | "privacy-requests"
            | "privacy-report"
            | "server-reports"
    )
}
pub(super) fn is_server_page(page: &str) -> bool {
    page == "server-reports"
}
pub(super) fn label(page: &str) -> Option<&'static str> {
    Some(match page {
        "privacy-status" => "Set Status",
        "privacy-custom" => "Set a Custom Status",
        "privacy-requests" => "Message Requests",
        "privacy-report" => "Report",
        "server-reports" => "Reports",
        _ => return None,
    })
}
pub(super) fn page(a: &App, page: &str) -> AnyView {
    match page {
        "privacy-status" => status_menu(a),
        "privacy-custom" => custom_editor(a),
        "privacy-requests" => requests_page(a),
        "privacy-report" => report_dialog(a),
        "server-reports" => server_reports(a),
        _ => ().into_any(),
    }
}
