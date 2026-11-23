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
/// Any one of these permissions (or owner, or administrator).
pub(super) fn can_any(a: &App, permissions: Permissions) -> bool {
    a.guild
        .and_then(|id| a.model.guilds.get(&id))
        .is_some_and(|g| {
            g.my_role == MemberRole::Owner
                || g.my_permissions
                    .intersects(Permissions::ADMINISTRATOR | permissions)
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
            | "guild-delete"
    ) || super::moderation::is_server_page(page)
        || super::expression::is_server_page(page)
        || super::privacy::is_server_page(page)
        || [
            "role:",
            "channel-edit:",
            "channel-delete:",
            "channel-move:",
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
    // Only the pages this person can do something on.
    let pages = [
        ("user-round", "Overview", "server-overview", true),
        (
            "hash",
            "Channels",
            "server-channels",
            can(a, Permissions::MANAGE_CHANNELS),
        ),
        (
            "shield",
            "Roles",
            "roles",
            can(a, Permissions::MANAGE_ROLES),
        ),
        (
            "users",
            "Member Roles",
            "member-role",
            can(a, Permissions::MANAGE_ROLES),
        ),
        (
            "link",
            "Invites",
            "invite",
            can_any(a, Permissions::CREATE_INVITES | Permissions::MANAGE_GUILD),
        ),
        // Expression (expression.rs): emoji and stickers.
        (
            "smile",
            "Emoji",
            "server-emoji",
            can(a, Permissions::MANAGE_EMOJI),
        ),
        (
            "sticker",
            "Stickers",
            "server-stickers",
            can(a, Permissions::MANAGE_EMOJI),
        ),
    ];
    // Moderation, as its own group.
    let moderation = [
        (
            "shield-check",
            "Safety Setup",
            "safety",
            can_any(
                a,
                Permissions::MANAGE_GUILD
                    | Permissions::KICK_MEMBERS
                    | Permissions::BAN_MEMBERS
                    | Permissions::MODERATE_MEMBERS,
            ),
        ),
        (
            "shield",
            "AutoMod",
            "automod",
            can(a, Permissions::MANAGE_GUILD),
        ),
        (
            "file",
            "Audit Log",
            "audit-log",
            can(a, Permissions::VIEW_AUDIT_LOG),
        ),
        (
            "ban",
            "Bans",
            "server-bans",
            can(a, Permissions::BAN_MEMBERS),
        ),
        (
            "user-minus",
            "Prune Members",
            "prune",
            can(a, Permissions::KICK_MEMBERS),
        ),
        (
            "flag",
            "Reports",
            "server-reports",
            can_any(
                a,
                Permissions::MANAGE_MESSAGES
                    | Permissions::KICK_MEMBERS
                    | Permissions::BAN_MEMBERS
                    | Permissions::MODERATE_MEMBERS,
            ),
        ),
    ];
    let kind = a.dialog.split(':').next().unwrap_or_default();
    let title = pages
        .iter()
        .chain(moderation.iter())
        .find(|(_, _, page, _)| *page == a.dialog)
        .map_or(
            match kind {
                "role" => "Edit Role",
                "channel-edit" => "Edit Channel",
                "channel-delete" => "Delete Channel",
                "channel-move" => "Move Channel",
                "ban" => "Ban a Member",
                "unban" => "Revoke Ban",
                "invite-revoke" => "Revoke Invite",
                "guild-delete" => "Delete Server",
                "automod-rule" => "AutoMod Rule",
                "channel-perms" => "Channel Permissions",
                "emoji-edit" => "Edit Emoji",
                "sticker-edit" => "Edit Sticker",
                _ => "Server Settings",
            },
            |(_, label, _, _)| *label,
        );
    // A page opened from a list goes back to that list.
    let back = match kind {
        "role" => ("Back to Roles", "dialog:roles"),
        "channel-edit" | "channel-delete" | "channel-move" | "private" => {
            ("Back to Channels", "dialog:server-channels")
        }
        "ban" | "unban" => ("Back to Bans", "dialog:server-bans"),
        "invite-revoke" => ("Back to Invites", "dialog:invite"),
        "guild-delete" => ("Back to Overview", "dialog:server-overview"),
        "automod-rule" => ("Back to AutoMod", "dialog:automod"),
        "channel-perms" => ("Back to Channels", "dialog:server-channels"),
        "emoji-edit" => ("Back to Emoji", "dialog:server-emoji"),
        "sticker-edit" => ("Back to Stickers", "dialog:server-stickers"),
        _ => ("Back to Server Settings", "dialog:server"),
    };
    settings_page(
        a,
        SettingsPage {
            root: a.dialog == "server",
            label: "Server Settings",
            nav_label: "Server settings",
            mobile_title: "Server Settings",
            back,
            title,
        },
        view!{<h2 class="mx-2 text-xl wrap-anywhere">{name.to_owned()}</h2><h3 class="category mx-2 mt-6 mb-2">"Server Settings"</h3>
            {pages.into_iter().filter(|page|page.3).map(|(glyph,label,page,_)|settings_nav_row(glyph,label,&format!("dialog:{page}"),a.dialog==page)).collect_view()}
            {if moderation.iter().any(|page|page.3){view!{<h3 class="category mx-2 mt-6 mb-2">"Moderation"</h3>}.into_any()}else{().into_any()}}
            {moderation.into_iter().filter(|page|page.3).map(|(glyph,label,page,_)|settings_nav_row(glyph,label,&format!("dialog:{page}"),a.dialog==page)).collect_view()}
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
        <fieldset disabled=!allowed>{form("Overview","server-overview",view!{{field("f-name","Server Name",&g.guild.name,"text")}<p class="muted-copy">{if allowed{"Changes apply to everyone in this server."}else{"You need Manage Server permission to edit this server."}}</p>{saved_note(a,"server-overview")}}.into_any())}
        <div class="settings-divider"></div><h2>"Server Icon"</h2><div class="avatar-edit mb-6 flex items-center gap-4">{if let Some(id)=g.guild.icon_id{view!{<img class="server-profile-icon size-16 rounded-2xl object-cover" src=format!("/api/v1/files/{}/thumbnail",id.0) alt="Server icon"/>}.into_any()}else{view!{<span class="server-profile-icon-placeholder grid size-16 place-items-center rounded-2xl bg-muted text-xl">{g.guild.name.chars().take(2).collect::<String>()}</span>}.into_any()}}<button type="button" class="button secondary" data-pick="live-guild-icon">"Change Icon"</button><input id="live-guild-icon" type="file" accept="image/png,image/jpeg,image/webp" hidden/></div>
        {if a.uploading{view!{<p role="status">{a.upload_progress.clone()}</p>}.into_any()}else{().into_any()}}
        {if g.guild.icon_id.is_some(){button("Remove Icon","submit:guild-icon-remove","danger-text")}else{().into_any()}}
        <div class="settings-divider"></div>{form("Server Profile","server-profile",view!{<label class="field"><span>"Description"</span><textarea id="server-description" rows="3" maxlength="120">{profile.map_or("",|p|p.description.as_str()).to_owned()}</textarea></label><label class="field"><span>"Banner Color"</span><select id="server-banner">{presets.into_iter().enumerate().map(|(i,(label,_,_))|view!{<option value=i.to_string() selected=i==usize::from(banner)>{label}</option>}).collect_view()}</select></label>{saved_note(a,"server-profile")}}.into_any())}</fieldset>
        <div class="settings-divider"></div>{button("Invite People","dialog:invite","secondary")}{if can(a,Permissions::MANAGE_CHANNELS){button("Manage Channels","dialog:server-channels","secondary")}else{().into_any()}}
        <section class="danger-zone mt-8 border-t pt-6"><h3>"Danger Zone"</h3>{if g.my_role==MemberRole::Owner{view!{<p class="muted-copy">"Permanently delete this server and everything in it."</p>{button("Delete Server","dialog:guild-delete","danger-text")}}.into_any()}else{view!{<p class="muted-copy">"Leave this server. You’ll need a new invite to come back."</p>{button("Leave Server","leave-guild","danger-text")}}.into_any()}}</section>
    }.into_any()
}
pub(super) fn channels(a: &App) -> AnyView {
    let allowed = can(a, Permissions::MANAGE_CHANNELS);
    // The same order as the sidebar, with everything nested in a channel under it.
    let channels = channel_tree(a);
    view!{<h2>"Channels"</h2>{if allowed{button("Create Channel","dialog:channel","primary")}else{view!{<p class="muted-copy">"You need Manage Channels permission to edit channels."</p>}.into_any()}}
        {channels.into_iter().map(|(c,depth)|view!{<div class="settings-list-row flex flex-wrap items-center gap-3 border-b py-4" style=indent(depth,24) data-depth=depth.to_string()><span class="settings-row-copy flex min-w-24 flex-1 flex-col gap-1 wrap-anywhere"><strong class="font-semibold">{a.model.channel_name(c)}</strong><small>{match c.kind{ChannelKind::Category=>"Category",ChannelKind::Voice=>"Voice channel",_=>"Text channel"}}</small></span><button class="icon-button" aria-label=format!("Edit {}",a.model.channel_name(c)) disabled=!allowed data-live=format!("dialog:channel-edit:{}",c.id.0)>{icon("settings")}</button></div>}).collect_view()}
    }.into_any()
}
pub(super) fn channel_editor(a: &App, page: &str) -> AnyView {
    let Some(c) = id(page.trim_start_matches("channel-edit:"))
        .and_then(|id| a.model.channels.get(&id))
        .filter(|c| c.guild_id == a.guild)
    else {
        return ().into_any();
    };
    // Slowmode, safety and permissions are separate forms, after this one.
    let safety = super::moderation::channel_safety(a, c);
    let edit = form_with("Edit Channel","channel-save",view!{<fieldset disabled=!can(a,Permissions::MANAGE_CHANNELS)>{field("f-name","Name",c.name.as_deref().unwrap_or(""),"text")}{field("f-value","Topic",a.model.topics.get(&c.id).and_then(|t|t.as_deref()).unwrap_or(""),"text")}<input id="f-extra" type="hidden" value=c.id.0.to_string()/></fieldset>{super::call_extras::bitrate_section(a,c)}
        <div class="settings-divider"></div>{if can(a,Permissions::MANAGE_CHANNELS){view!{{button(if c.kind==ChannelKind::Category{"Move Category"}else{"Move Channel"},&format!("dialog:channel-move:{}",c.id.0),"secondary")}{button(if c.kind==ChannelKind::Category{"Delete Category"}else{"Delete Channel"},&format!("dialog:channel-delete:{}",c.id.0),"danger-text")}}.into_any()}else{().into_any()}}
    }.into_any(),"dialog:server-channels",can(a,Permissions::MANAGE_CHANNELS));
    view! {{edit}{safety}}.into_any()
}
/// Put a channel inside another one, or back at the top level.
pub(super) fn channel_move(a: &App, page: &str) -> AnyView {
    let Some(c) = id(page.trim_start_matches("channel-move:"))
        .and_then(|id| a.model.channels.get(&id))
        .filter(|c| c.guild_id == a.guild)
    else {
        return ().into_any();
    };
    let label = a.model.channel_name(c);
    form_with("Move Channel","channel-move",view!{<p>{format!("Choose where {label} goes. Everything inside it moves with it.")}</p>
        <label class="field">"Inside"<select id="f-value" aria-describedby="channel-move-help">{inside_options(a,c.parent_id,Some(c.id))}</select><small id="channel-move-help">"A channel also follows the permissions of everything it is inside."</small></label><input id="f-extra" type="hidden" value=c.id.0.to_string()/>
    }.into_any(),&format!("dialog:channel-edit:{}",c.id.0),can(a,Permissions::MANAGE_CHANNELS))
}
pub(super) fn channel_delete(a: &App, page: &str) -> AnyView {
    let channel =
        id(page.trim_start_matches("channel-delete:")).and_then(|id| a.model.channels.get(&id));
    let label = channel.map_or("this channel".into(), |c| a.model.channel_name(c));
    // Deleting a category loses nothing; deleting a channel loses its messages.
    // Either way, whatever is nested inside is kept and moves up one level.
    let category = channel.is_some_and(|c| c.kind == ChannelKind::Category);
    let kept =
        if channel.is_some_and(|c| a.model.channels.values().any(|v| v.parent_id == Some(c.id))) {
            " The channels inside it are kept and move up one level."
        } else {
            ""
        };
    form_with(if category{"Delete Category"}else{"Delete Channel"},"channel-delete",view!{<p>{if category{format!("Delete the category {label}?{kept}")}else{format!("Permanently delete {label} and its messages? This cannot be undone.{kept}")}}</p>
        {field("f-name","Type DELETE to confirm","","text")}<input id="f-extra" type="hidden" value=channel.map_or(String::new(),|c|c.id.0.to_string())/>
    }.into_any(),"dialog:server-channels",true)
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
    if !can(a, Permissions::MANAGE_ROLES) {
        return view!{<h2>"Member Roles"</h2><p class="muted-copy">"You need Manage Roles permission to change member roles."</p>}.into_any();
    }
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
        <label class="live-checkbox"><span>"Show website previews for links"</span><input type="checkbox" data-preference="linkPreviews"/></label>
        <h3 class="mt-6">"Profiles"</h3><label class="live-checkbox"><span class="flex flex-col"><span>"Show plain names"</span><small class="text-xs text-subtle">"Other people’s name fonts, colors and effects are turned off. Role colors stay."</small></span><input type="checkbox" data-preference="plainNames"/></label>
        <label class="live-checkbox"><span class="flex flex-col"><span>"Reduce cosmetics"</span><small class="text-xs text-subtle">"Hide other people’s avatar decorations, nameplates, profile effects and frames."</small></span><input type="checkbox" data-preference="reduceCosmetics"/></label>
        <div class="settings-divider"></div><h3>"Preview"</h3><p class="appearance-preview rounded-lg border bg-card p-4">"This is how your messages will look."</p><p class="muted-copy">"StickTalk respects your device’s reduced motion setting."</p>
    }.into_any()
}
pub(super) fn privacy(a: &App) -> AnyView {
    super::privacy::privacy_page(a)
}
pub(super) fn notifications(a: &App) -> AnyView {
    super::privacy::notifications(a)
}
