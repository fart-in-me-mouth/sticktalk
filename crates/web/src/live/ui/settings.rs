//! Lazy settings surfaces. Every mutation uses an existing typed server command.
use super::*;

pub(super) fn can(a: &App, permission: Permissions) -> bool {
    a.guild
        .and_then(|id| a.model.guilds.get(&id))
        .is_some_and(|g| {
            g.my_role == MemberRole::Owner
                || g.my_permissions.contains(Permissions::ADMINISTRATOR)
                || g.my_permissions.contains(permission)
        })
}
pub(super) fn is_server_page(page: &str) -> bool {
    matches!(
        page,
        "server"
            | "server-overview"
            | "server-channels"
            | "server-bans"
            | "ban"
            | "server-emoji"
            | "roles"
            | "member-role"
            | "private"
            | "invite"
    ) || [
        "role:",
        "channel-edit:",
        "channel-delete:",
        "invite-revoke:",
        "unban:",
    ]
    .iter()
    .any(|prefix| page.starts_with(prefix))
}
pub(super) fn server_layout(a: &App, body: AnyView) -> AnyView {
    let name = a
        .guild
        .and_then(|g| a.model.guilds.get(&g))
        .map_or("Server Settings", |g| g.guild.name.as_str());
    let pages = [
        ("user-round", "Overview", "server-overview"),
        ("hash", "Channels", "server-channels"),
        ("shield", "Roles", "roles"),
        ("users", "Member Roles", "member-role"),
        ("link", "Invites", "invite"),
        ("shield", "Bans", "server-bans"),
        ("smile", "Emoji", "server-emoji"),
    ];
    let title = pages.iter().find(|(_, _, page)| *page == a.dialog).map_or(
        if a.dialog.starts_with("role:") {
            "Edit Role"
        } else {
            "Server Settings"
        },
        |(_, label, _)| *label,
    );
    settings_page(
        a,
        SettingsPage {
            root: a.dialog == "server",
            label: "Server Settings",
            nav_label: "Server settings",
            mobile_title: "Server Settings",
            back: ("Back to Server Settings", "dialog:server"),
            title,
        },
        view!{<h2 class="mx-2 text-xl wrap-anywhere">{name.to_owned()}</h2><h3 class="category mx-2 mt-6 mb-2">"Server Settings"</h3>
            {pages.into_iter().map(|(glyph,label,page)|settings_nav_row(glyph,label,&format!("dialog:{page}"),a.dialog==page)).collect_view()}
        }.into_any(),
        body,
    )
}
pub(super) fn server_overview(a: &App) -> AnyView {
    let Some(g) = a.guild.and_then(|id| a.model.guilds.get(&id)) else {
        return view! {<h2>"Server Settings"</h2><p>"Select a server first."</p>}.into_any();
    };
    let allowed = can(a, Permissions::MANAGE_GUILD);
    let profile = a
        .model
        .settings
        .guild_profiles
        .get(&g.guild.id)
        .map(|g| &g.profile);
    let banner = profile.map_or(0, |p| p.banner_color).min(9);
    // Same ten preset gradients as the native Server Profile page.
    let presets = [
        ("Green", 0x376f61, 0x1c3436),
        ("Blue", 0x487ea0, 0x29364e),
        ("Violet", 0x78609f, 0x37304b),
        ("Rose", 0xa05e79, 0x4b3041),
        ("Amber", 0xa67c50, 0x49382d),
        ("Teal", 0x408f8d, 0x253e4d),
        ("Indigo", 0x646cb0, 0x323350),
        ("Magenta", 0x965b9e, 0x49304d),
        ("Gold", 0xa19350, 0x4d4229),
        ("Slate", 0x687480, 0x323941),
    ];
    let (_, start, end) = presets[usize::from(banner)];
    view!{<div class="profile-banner" style=format!("background:linear-gradient(135deg,#{start:06x},#{end:06x})")></div>
        <fieldset disabled=!allowed>{form("Overview","server-overview",view!{{field("f-name","Server Name",&g.guild.name,"text")}<p class="muted-copy">{if allowed{"Changes apply to everyone in this server."}else{"You need Manage Server permission to edit this server."}}</p>}.into_any())}
        <div class="settings-divider"></div><h2>"Server Icon"</h2><div class="avatar-edit mb-6 flex items-center gap-4">{if let Some(id)=g.guild.icon_id{view!{<img class="server-profile-icon size-16 rounded-2xl object-cover" src=format!("/api/v1/files/{}/thumbnail",id.0) alt="Server icon"/>}.into_any()}else{view!{<span class="server-profile-icon-placeholder grid size-16 place-items-center rounded-2xl bg-muted text-xl">{g.guild.name.chars().take(2).collect::<String>()}</span>}.into_any()}}<label class="button secondary" for="live-guild-icon">"Change Icon"</label><input id="live-guild-icon" type="file" accept="image/png,image/jpeg,image/webp" hidden/></div>
        {if a.uploading{view!{<p role="status">{a.upload_progress.clone()}</p>}.into_any()}else{().into_any()}}
        {if g.guild.icon_id.is_some(){form("Remove Icon","guild-icon-remove",().into_any())}else{().into_any()}}
        <div class="settings-divider"></div>{form("Server Profile","server-profile",view!{<label class="field"><span>"Description"</span><textarea id="server-description" rows="3" maxlength="120">{profile.map_or("",|p|p.description.as_str()).to_owned()}</textarea></label><label class="field"><span>"Banner Color"</span><select id="server-banner">{presets.into_iter().enumerate().map(|(i,(label,_,_))|view!{<option value=i.to_string() selected=i==usize::from(banner)>{label}</option>}).collect_view()}</select></label>}.into_any())}</fieldset>
        <div class="settings-divider"></div>{button("Invite People","dialog:invite","secondary")}{button("Manage Channels","dialog:server-channels","secondary")}
    }.into_any()
}
pub(super) fn channels(a: &App) -> AnyView {
    let allowed = can(a, Permissions::MANAGE_CHANNELS);
    let mut channels = a
        .model
        .channels
        .values()
        .filter(|c| c.guild_id == a.guild)
        .collect::<Vec<_>>();
    channels.sort_by_key(|c| (c.position, c.id));
    view!{<h2>"Channels"</h2>{if allowed{button("Create Channel","dialog:channel","primary")}else{view!{<p class="muted-copy">"You need Manage Channels permission to edit channels."</p>}.into_any()}}
        {channels.into_iter().map(|c|view!{<div class="settings-list-row flex flex-wrap items-center gap-3 border-b py-4"><span class="settings-row-copy flex min-w-24 flex-1 flex-col gap-1 wrap-anywhere"><strong class="font-semibold">{a.model.channel_name(c)}</strong><small>{match c.kind{ChannelKind::Category=>"Category",ChannelKind::Voice=>"Voice channel",_=>"Text channel"}}</small></span><button class="icon-button" aria-label=format!("Edit {}",a.model.channel_name(c)) disabled=!allowed data-live=format!("dialog:channel-edit:{}",c.id.0)>{icon("settings")}</button></div>}).collect_view()}
    }.into_any()
}
pub(super) fn channel_editor(a: &App, page: &str) -> AnyView {
    let Some(c) = id(page.trim_start_matches("channel-edit:"))
        .and_then(|id| a.model.channels.get(&id))
        .filter(|c| c.guild_id == a.guild)
    else {
        return ().into_any();
    };
    form("Edit Channel","channel-save",view!{<fieldset disabled=!can(a,Permissions::MANAGE_CHANNELS)>{field("f-name","Name",c.name.as_deref().unwrap_or(""),"text")}{field("f-value","Topic",a.model.topics.get(&c.id).and_then(|t|t.as_deref()).unwrap_or(""),"text")}<input id="f-extra" type="hidden" value=c.id.0.to_string()/></fieldset>
        <div class="settings-divider"></div>{button("Delete Channel",&format!("dialog:channel-delete:{}",c.id.0),"danger-text")}
    }.into_any())
}
pub(super) fn channel_delete(a: &App, page: &str) -> AnyView {
    let channel =
        id(page.trim_start_matches("channel-delete:")).and_then(|id| a.model.channels.get(&id));
    form("Delete Channel","channel-delete",view!{<p>{format!("Permanently delete {} and its messages? This cannot be undone.",channel.map_or("this channel".into(),|c|a.model.channel_name(c)))}</p>
        {field("f-name","Type DELETE to confirm","","text")}<input id="f-extra" type="hidden" value=channel.map_or(String::new(),|c|c.id.0.to_string())/>
    }.into_any())
}
fn more(a: &App) -> AnyView {
    if a.admin_busy {
        view! {<p class="muted-copy" role="status">"Loading…"</p>}.into_any()
    } else if a.admin_more {
        button("Load More", "admin-more", "secondary")
    } else {
        ().into_any()
    }
}
pub(super) fn invites(a: &App) -> AnyView {
    let allowed = can(a, Permissions::CREATE_INVITES);
    let all = can(a, Permissions::MANAGE_GUILD);
    let me = a.model.me.as_ref().map(|me| me.user.id);
    let invites = a
        .model
        .invites
        .values()
        .filter(|i| Some(i.guild_id) == a.guild && (all || (allowed && Some(i.creator_id) == me)))
        .collect::<Vec<_>>();
    view!{{form("Invites","invite",view!{<p class="muted-copy">"Create a link valid for 7 days, for up to 100 people."</p><fieldset disabled=!allowed><input id="f-name" type="hidden"/></fieldset>}.into_any())}
        {if !a.admin_busy&&invites.is_empty(){view!{<p class="muted-copy">"No active invites."</p>}.into_any()}else{().into_any()}}
        {invites.into_iter().map(|i|view!{<div class="settings-list-row flex flex-wrap items-center gap-3 border-b py-4"><span class="settings-row-copy flex min-w-24 flex-1 flex-col gap-1 wrap-anywhere"><code>{i.code.clone()}</code><small>{format!("{} / {} uses",i.uses,i.max_uses)}</small></span><button class="button secondary" data-copy=format!("{}/?invite={}",a.origin,i.code)>"Copy Link"</button>{button("Revoke",&format!("dialog:invite-revoke:{}",i.code),"danger-text")}</div>}).collect_view()}{more(a)}
    }.into_any()
}
pub(super) fn bans(a: &App) -> AnyView {
    let bans = a
        .model
        .bans
        .values()
        .filter(|b| Some(b.guild_id) == a.guild && can(a, Permissions::BAN_MEMBERS))
        .collect::<Vec<_>>();
    view!{<h2>"Bans"</h2>{if !can(a,Permissions::BAN_MEMBERS){view!{<p class="muted-copy">"You need Ban Members permission to manage bans."</p>}.into_any()}else if bans.is_empty()&&!a.admin_busy{view!{<p class="muted-copy">"No banned members."</p>}.into_any()}else{().into_any()}}
        {bans.into_iter().map(|b|view!{<div class="settings-list-row flex flex-wrap items-center gap-3 border-b py-4">{avatar(&b.user,"small")}<span class="settings-row-copy flex min-w-24 flex-1 flex-col gap-1 wrap-anywhere"><strong class="font-semibold">{name(&b.user)}</strong><small>{b.reason.clone().unwrap_or("No reason provided".into())}</small></span>{button("Revoke",&format!("dialog:unban:{}",b.user.id.0),"danger-text")}</div>}).collect_view()}{more(a)}
        {if can(a,Permissions::BAN_MEMBERS){view!{<div class="settings-divider"></div>{button("Ban a Member","dialog:ban","danger-text")}}.into_any()}else{().into_any()}}
    }.into_any()
}
pub(super) fn roles(a: &App) -> AnyView {
    let allowed = can(a, Permissions::MANAGE_ROLES);
    let mut roles = a
        .roles
        .values()
        .filter(|r| Some(r.guild_id) == a.guild)
        .collect::<Vec<_>>();
    roles.sort_by_key(|r| (std::cmp::Reverse(r.position), r.id));
    let loaded = a
        .model
        .members
        .values()
        .filter(|m| Some(m.guild_id) == a.guild)
        .count();
    let total = a
        .guild
        .and_then(|g| a.model.guilds.get(&g))
        .map_or(loaded, |g| g.guild.member_count as usize);
    view!{<div class="section-heading flex items-center justify-between gap-4"><h2>"Roles"</h2><button class="button secondary" data-role-reorder disabled=!allowed aria-pressed="false">"Reorder"</button></div><p class="muted-copy roles-help mt-3 mb-6">"Tap a role to edit its permissions. Higher roles take priority."</p><div class="role-list mb-8">
        {roles.into_iter().map(|r|{let count=a.model.members.values().filter(|m|Some(m.guild_id)==a.guild&&(r.id==r.guild_id||m.role_ids.contains(&r.id))).count();view!{<div class="role-list-row flex items-center gap-2 border-b" data-role-id=r.id.0.to_string()><button class="role-editor-link flex min-h-[72px] min-w-0 flex-1 items-center gap-3 py-3 [&>.icon]:size-4 [&>.icon]:text-subtle" data-live=format!("role:{}",r.id.0)><span class="role-color size-4 flex-none rounded-full" style=format!("background:#{:06x}",if r.color==0{0xa8a8b0}else{r.color})></span><span class="settings-row-copy flex min-w-24 flex-1 flex-col gap-1 wrap-anywhere"><strong class="font-semibold">{r.name.clone()}</strong><small>{format!("{}{} member{}",if r.id==r.guild_id{total}else{count},if r.id!=r.guild_id&&loaded<total{"+"}else{""},if count==1&&loaded>=total{""}else{"s"})}</small></span>{icon("chevron-right")}</button>{if r.id!=r.guild_id{view!{<select class="role-position w-[132px] flex-none" aria-label=format!("Position for {}",r.name) data-role-position=r.id.0.to_string() disabled=!allowed hidden></select>}.into_any()}else{().into_any()}}</div>}}).collect_view()}</div>
        <fieldset disabled=!allowed>{form("Create Role","role-create",field("f-name","Role Name","","text"))}</fieldset>
    }.into_any()
}

pub(super) fn member_roles(a: &App) -> AnyView {
    let members = a
        .model
        .members
        .values()
        .filter(|m| Some(m.guild_id) == a.guild)
        .collect::<Vec<_>>();
    let assigned = members.first().map_or(&[][..], |m| m.role_ids.as_slice());
    let selected = assigned
        .iter()
        .filter(|id| Some(**id) != a.guild)
        .map(|id| id.0.to_string())
        .collect::<Vec<_>>()
        .join(",");
    form("Assign Member Roles","member-role",view!{<label class="field"><span>"Member"</span><select id="f-name">{members.iter().map(|m|view!{<option value=m.user.id.0.to_string() data-role-ids={m.role_ids.iter().map(|id|id.0.to_string()).collect::<Vec<_>>().join(",")}>{name(&m.user)}</option>}).collect_view()}</select></label>
        <input id="f-value" type="hidden" value=selected/>
        {a.roles.values().filter(|r|Some(r.guild_id)==a.guild&&r.id!=r.guild_id).map(|r|view!{<label class="live-checkbox"><span>{r.name.clone()}</span><input type="checkbox" checked=assigned.contains(&r.id) data-member-role=r.id.0.to_string()/></label>}).collect_view()}
        <p class="muted-copy">"Replaces all roles for this member."</p>
    }.into_any())
}
pub(super) fn appearance() -> AnyView {
    view!{<h2>"Appearance"</h2><p class="muted-copy">"Saved on this device."</p><h3>"Theme"</h3><p>"Dark"</p>
        <label class="field"><span>"Message Display"</span><select data-preference="density"><option value="cozy">"Cozy"</option><option value="compact">"Compact"</option></select></label>
        <label class="field"><span>"Chat Font Scaling"</span><select data-preference="fontScale"><option value="0.8">"80%"</option><option value="0.9">"90%"</option><option value="1">"100%"</option><option value="1.1">"110%"</option><option value="1.25">"125%"</option><option value="1.5">"150%"</option></select></label>
        <label class="live-checkbox"><span>"Always underline links"</span><input type="checkbox" data-preference="underlineLinks"/></label>
        <div class="settings-divider"></div><h3>"Preview"</h3><p class="appearance-preview rounded-lg border bg-card p-4">"This is how your messages will look."</p><p class="muted-copy">"StickTalk respects your device’s reduced motion setting."</p>
    }.into_any()
}
pub(super) fn privacy(a: &App) -> AnyView {
    let blocked = a
        .model
        .relationships
        .values()
        .filter(|r| r.kind == RelationshipKind::Blocked)
        .collect::<Vec<_>>();
    view!{<h2>"Privacy & Safety"</h2><h2>"Blocked Users"</h2>{if blocked.is_empty(){view!{<p class="muted-copy">"You haven’t blocked anyone."</p>}.into_any()}else{().into_any()}}
        {blocked.into_iter().map(|r|view!{<div class="settings-list-row flex flex-wrap items-center gap-3 border-b py-4">{avatar(&r.user,"small")}<span class="settings-row-copy flex min-w-24 flex-1 flex-col gap-1 wrap-anywhere">{name(&r.user)}</span>{button("Unblock",&format!("friend:unblock,{}",r.user.id.0),"secondary")}</div>}).collect_view()}
        <div class="settings-divider"></div><h2>"Your Data"</h2><p class="muted-copy">"Messages are not stored in the offline app cache. Browser notifications never show message text on your lock screen."</p>{button("Manage Sessions","dialog:devices","secondary")}
    }.into_any()
}
pub(super) fn notifications(a: &App) -> AnyView {
    let explicit = a
        .dialog
        .strip_prefix("target-notifications:")
        .and_then(|v| v.split_once(':'))
        .and_then(|(kind, value)| id(value).map(|id| (kind, id)));
    let (guild, channel) = if let Some((kind, target)) = explicit {
        if kind == "guild" {
            (Some(target), None)
        } else {
            (
                a.model.channels.get(&target).and_then(|c| c.guild_id),
                Some(target),
            )
        }
    } else if let Some(c) = a
        .channel
        .and_then(|id| a.model.channels.get(&id))
        .filter(|c| {
            matches!(
                c.kind,
                ChannelKind::Text | ChannelKind::Direct | ChannelKind::GroupDm
            )
        })
    {
        (c.guild_id, Some(c.id))
    } else {
        (
            a.guild.or_else(|| a.model.guilds.keys().next().copied()),
            None,
        )
    };
    let target = channel.map_or_else(
        || guild.map_or(String::new(), |g| format!("guild:{}", g.0)),
        |c| format!("channel:{}", c.0),
    );
    let n = a.model.settings.notifications.get(&(guild, channel));
    let level = n.map_or(NotificationLevel::Mentions, |n| n.level);
    view!{<fieldset hidden=target.is_empty()>{form("Notifications","notifications",view!{
        <label class="field"><span>"Server or Conversation"</span><select id="f-target" data-notification-target><option value="" disabled selected=target.is_empty()>"Choose a conversation"</option>
        {a.model.guilds.values().map(|g|view!{<option value=format!("guild:{}",g.guild.id.0) selected=target==format!("guild:{}",g.guild.id.0)>{g.guild.name.clone()}</option>}).collect_view()}
        {a.model.channels.values().filter(|c|matches!(c.kind,ChannelKind::Text|ChannelKind::Direct|ChannelKind::GroupDm)).map(|c|view!{<option value=format!("channel:{}",c.id.0) selected=target==format!("channel:{}",c.id.0)>{format!("#{}",a.model.channel_name(c))}</option>}).collect_view()}</select></label>
        <label class="field"><span>"Notify Me About"</span><select id="f-name"><option value="all" selected=level==NotificationLevel::All>"All messages"</option><option value="mentions" selected=level==NotificationLevel::Mentions>"Only @mentions"</option><option value="nothing" selected=level==NotificationLevel::Nothing>"Nothing"</option></select></label>
        <label class="live-checkbox"><span>"Mute notifications"</span><input id="f-muted" type="checkbox" checked=n.is_some_and(|n|n.muted)/></label><label class="live-checkbox"><span>"Suppress @everyone and @here"</span><input id="f-suppress" type="checkbox" checked=n.is_some_and(|n|n.suppress_everyone)/></label>
    }.into_any())}</fieldset>{if target.is_empty(){view!{<p class="muted-copy">"Choose a server or start a conversation to customize notifications."</p>{button("Add Friend","dialog:friend","secondary")}{button("Create a Server","dialog:create","secondary")}}.into_any()}else{().into_any()}}{if a.push{view!{<section class="push-explainer mt-6 border-t pt-6"><h3>"Notifications when you’re away"</h3><p>"Allow StickTalk to notify you about DMs and mentions. No message text appears on your lock screen. You can turn this off in your browser settings."</p><button class="button secondary" data-push-enable>"Enable Browser Notifications"</button></section>}.into_any()}else{().into_any()}}}.into_any()
}
