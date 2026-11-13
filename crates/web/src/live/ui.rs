use super::*;
fn icon(name: &str) -> AnyView {
    view! { <svg class="icon" aria-hidden="true" viewBox="0 0 24 24"><use href=format!("#i-{name}")></use></svg> }.into_any()
}
use sticktalk_client_core::markdown::{Block, Inline, Target};
fn button(label: &str, action: &str, class: &str) -> AnyView {
    view!{<button class=format!("button {class}") data-live=action.to_owned()>{label.to_owned()}</button>}.into_any()
}
fn ib(glyph: &str, label: &str, action: &str) -> AnyView {
    view!{<button class="icon-button" aria-label=label.to_owned() title=label.to_owned() data-live=action.to_owned()>{icon(glyph)}</button>}.into_any()
}
fn field(id: &str, label: &str, text: &str, kind: &str) -> AnyView {
    view!{<div class="field"><label for=id.to_owned()>{label.to_owned()}</label><input id=id.to_owned() type=kind.to_owned() value=text.to_owned()/></div>}.into_any()
}
fn name(u: &UserSummary) -> String {
    u.display_name.clone().unwrap_or_else(|| u.username.clone())
}
fn avatar(u: &UserSummary, size: &str) -> AnyView {
    let initials = name(u)
        .split_whitespace()
        .filter_map(|s| s.chars().next())
        .take(2)
        .collect::<String>();
    view! {<span class=format!("avatar {size}")><span class="avatar-face">{initials}</span></span>}
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
pub(super) fn app(a: &App) -> AnyView {
    view!{<div id="sticktalk" data-live-app data-screen=a.screen.clone() data-self-id=a.model.me.as_ref().map(|me|me.user.id.0.to_string())>
        <div hidden inner_html=include_str!("../../public/icons/sprite.svg")></div>
        {if a.model.me.is_none(){auth(a)}else{shell(a)}}
        {if a.error.is_empty(){().into_any()}else{view!{<div class="live-error" role="alert">{a.error.clone()}{ib("x","Dismiss","dismiss-error")}</div>}.into_any()}}
        {if a.dialog.is_empty(){().into_any()}else{dialog(a)}}
        <div id="live-viewer" class="live-viewer" hidden role="dialog" aria-modal="true" aria-label="Image viewer"></div>
    </div>}.into_any()
}
fn auth(a: &App) -> AnyView {
    if a.status == "Reconnecting…" {
        return view!{<main class="auth-page"><section class="auth-card"><h1>"Reconnecting…"</h1><p class="muted-copy">"Your conversations will appear when the server is available."</p>{button("Try Again","recover","primary")}</section></main>}.into_any();
    }

    let signup = a.signup;
    let register = a.register;
    view!{<main class="auth-page"><form class="auth-card" data-live-form="auth"><img class="auth-logo" src="/public/icons/icon-192.png" alt="StickTalk" width="48" height="48"/><h1>{if signup{"Choose a username"}else if register{"Create an account"}else{"Log in"}}</h1>
        <div class="field"><label for="auth-username">"Username"</label><input id="auth-username" name="username" autocomplete="username" required minlength="3" maxlength="32" autofocus /></div>
        {if signup{().into_any()}else{view!{<div class="field"><label for="auth-password">"Password"</label><input id="auth-password" name="password" type="password" autocomplete=if register{"new-password"}else{"current-password"} required minlength="12" maxlength="128"/></div>}.into_any()}}
        <button class="button primary" type="submit" disabled=!a.status.is_empty()>{if signup{"Continue"}else if register{"Create Account"}else{"Log In"}}</button>
        <p role="status">{a.status.clone()}</p>
        {if a.google&&!signup {view!{<a class="button secondary google-button" href="/api/v1/browser/google/start"><img src="/public/google-g.svg" alt="" width="20" height="20"/>"Continue with Google"</a>}.into_any()}else{().into_any()}}
        {if signup{().into_any()}else{button(if register{"Already have an account? Log in"}else{"Create an account"},"register","link-button")}}
    </form></main>}.into_any()
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
    view!{<div class=format!("app-shell live-shell panel-{} {}",a.panel,if channel.is_some(){"in-channel"}else{""})>
        <div class="left-panel"><nav class="server-rail" aria-label="Servers"><button class="server-icon brand" data-live="home" aria-label="Home"><img src="/public/icons/icon-192.png" alt="" width="48" height="48"/></button><div class="rail-divider"></div>
            {a.model.guilds.values().map(|g|view!{<button class=format!("server-icon {}",if a.guild==Some(g.guild.id){"selected"}else{""}) data-live=format!("guild:{}",g.guild.id.0) title=g.guild.name.clone() aria-label=g.guild.name.clone()>{g.guild.name.chars().take(2).collect::<String>()}</button>}).collect_view()}
            {ib("plus","Add a Server","dialog:create")}
        </nav><aside class="channel-sidebar"><div class="sidebar-header"><strong>{guild_name}</strong>{ib("plus","Create Channel","dialog:channel")}{ib("settings","Server Settings","dialog:server")}</div>
            <nav class="channel-scroll" aria-label="Channels">
            {button("Friends","home","nav-row")}
            {a.model.channels.values().filter(|c|c.guild_id==a.guild).map(|c|{
                let unread=a.model.reads.get(&c.id);let label=a.model.channel_name(c);
                if c.kind==ChannelKind::Category{return view!{<h2 class="category">{label}</h2>}.into_any()}
                view!{<div class=format!("channel-row {}",if a.channel==Some(c.id){"selected"}else{""})><button class="channel-main" data-live=format!("{}:{}",if c.kind==ChannelKind::Voice{"voice"}else{"channel"},c.id.0)>{icon(if c.kind==ChannelKind::Voice{"volume-2"}else if c.guild_id.is_none(){"message-circle"}else{"hash"})}<span>{label}</span>{if let Some(read)=unread.filter(|r|r.unread_count>0){view!{<span class="live-unread">{read.unread_count.to_string()}</span>}.into_any()}else{().into_any()}}</button></div>}.into_any()
            }).collect_view()}
            </nav>
            <section id="live-connected" class="voice-connected" hidden><div class="voice-connection-line"><strong>"Voice Connected"</strong></div><div class="voice-tiles">{ib("video","Camera","media:camera")}{ib("monitor","Share Screen","media:share")}{ib("phone-off","Disconnect","media:leave")}</div></section>
            <section class="user-panel">{avatar(me,"small")}<button class="self-label" data-live=format!("profile:{}",me.id.0)><strong>{name(me)}</strong><small>{format!("@{}",me.username)}</small></button>{ib("mic","Mute","media:mute")}{ib("headphones","Deafen","media:deafen")}{ib("settings","User Settings","dialog:settings")}</section>
        </aside></div><div class="panel-scrim" data-live="panel:" aria-hidden="true"></div>
        <main class="main-pane"><header class="channel-header"><div class="mobile-menu">{ib("menu","Open Channels","panel:left")}</div>{icon(if channel.is_some_and(|c|c.kind==ChannelKind::Voice){"volume-2"}else if channel.is_some(){"hash"}else{"users"})}<h1>{title.clone()}</h1><div class="header-actions">{ib("bell","Notification Settings","dialog:notifications")}{ib("pin","Pinned Messages","dialog:pins")}{ib("users","Member List","panel:right")}</div></header>
            {if !a.status.is_empty(){view!{<div class="connection-banner" role="status">{a.status.clone()}</div>}.into_any()}else{().into_any()}}
            {if a.screen=="voice"{voice(a)}else if channel.is_some(){chat(a,&title)}else{home(a)}}
        </main>
        <aside class=format!("member-list {}",if a.panel=="right"{"is-open"}else{""}) aria-label="Members"><h2 class="category">"Members"</h2>{a.model.members.values().filter(|m|Some(m.guild_id)==a.guild).map(|m|view!{<button class="member-row" data-live=format!("profile:{}",m.user.id.0)>{avatar(&m.user,"small")}<span data-member-name=m.user.id.0.to_string() style=format!("color:{}",color(a,m.user.id))>{m.nickname.clone().unwrap_or_else(||name(&m.user))}</span></button>}).collect_view()}{button("Load More","members-more","link-button")}</aside>
        {if channel.is_none(){view!{<nav class="bottom-tabs" aria-label="Main navigation">{button("Home","home","active")}{button("Notifications","dialog:notifications","")}{button("You","dialog:settings","")}</nav>}.into_any()}else{().into_any()}}
    </div>}.into_any()
}
fn home(a: &App) -> AnyView {
    view!{<div class="live-home"><div class="friends-toolbar"><h2>"Friends"</h2>{button("Add Friend","dialog:friend","primary")}{button("Join a Server","dialog:join","secondary")}</div>
        {if a.model.relationships.is_empty(){view!{<p class="muted-copy">"No friends yet. Add a friend to start a conversation."</p>}.into_any()}else{().into_any()}}
        {a.model.relationships.values().map(|r|view!{<div class="friend-row">{avatar(&r.user,"small")}<button class="friend-info" data-live=format!("profile:{}",r.user.id.0)><strong>{name(&r.user)}</strong><small>{format!("@{} · {}",r.user.username,match r.kind{RelationshipKind::Friend=>"Friend",RelationshipKind::IncomingRequest=>"Incoming request",RelationshipKind::OutgoingRequest=>"Outgoing request",RelationshipKind::Blocked=>"Blocked"})}</small></button>
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
    view!{<div class="chat-body"><div id="live-messages" class="message-list" role="log" aria-label="Messages" tabindex="0" data-channel=c.0.to_string() data-window-start=a.window_rank.to_string() data-window-end=(a.window_rank+shown.len()as u64).to_string() data-total=count.to_string()>
        <div style=format!("height:{}px",a.window_rank*88) aria-hidden="true"></div>
        {if shown.is_empty(){view!{<div class="conversation-intro"><span class="intro-icon">{icon("hash")}</span><h2>{format!("Welcome to #{title}!")}</h2><p>{if a.history_busy{"Loading messages…"}else{"This is the beginning of this conversation."}}</p></div>}.into_any()}else{().into_any()}}
        {shown.iter().map(|m|message(a,m)).collect_view()}
        <div style=format!("height:{}px",bottom*88) aria-hidden="true"></div>
        </div><div class="composer-wrap">
        {if let Some(reply)=a.reply{view!{<div class="reply-preview">{format!("Replying to {}",messages.and_then(|m|m.get(&reply)).map_or("message".into(),|m|a.model.name(m.author_id)))}{ib("x","Cancel Reply","cancel-reply")}</div>}.into_any()}else{().into_any()}}
        {a.attachments.iter().enumerate().map(|(i,f)|view!{<div class="live-upload"><span>{f.filename.clone()}</span>{ib("x","Remove Attachment",&format!("remove-attachment:{i}"))}</div>}).collect_view()}
        {if a.uploading{view!{<div class="live-upload" role="status">{a.upload_progress.clone()}{button("Pause","upload-cancel","secondary")}</div>}.into_any()}else{().into_any()}}
        <div class="composer"><button type="button" class="icon-button" title="Add Attachment" aria-label="Add Attachment" data-live="pick-file">{icon("plus-circle")}</button><input type="file" id="live-file" hidden/><label class="composer-input"><span class="sr-only">"Message"</span><textarea id="live-input" rows="1" placeholder=format!("Message #{title}") aria-label="Message" maxlength="4000"></textarea></label>{button("GIF","dialog:gif","link-button")}{ib("smile","Emoji Picker","dialog:emoji")}{ib("send","Send Message","send-input")}</div>
        <div class="typing-indicator">{a.model.typing.iter().filter(|((channel,_),expiry)|*channel==c&&**expiry>Browser.unix_ms()).map(|((_,u),_)|a.model.name(*u)).take(3).collect::<Vec<_>>().join(", ")}{if a.model.typing.iter().any(|((ch,_),expiry)|*ch==c&&*expiry>Browser.unix_ms()){" is typing…"}else{""}}</div>
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
fn message(a: &App, m: &Message) -> AnyView {
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
    view!{<article class=format!("message {}",if mention{"mentioned"}else{""}) data-live-message=m.id.0.to_string()>
        <div class="avatar-gutter"><button data-live=format!("profile:{}",m.author_id.0) aria-label=format!("{}'s profile",name(&author))>{avatar(&author,"message-avatar")}</button></div>
        <div class="message-content"><div class="message-meta"><button class="author" style=format!("color:{}",color(a,m.author_id)) data-live=format!("profile:{}",m.author_id.0)>{name(&author)}</button><time>{timestamp(m.created_at)}</time>{if m.edited_at.is_some(){view!{<small class="edited">"(edited)"</small>}.into_any()}else{().into_any()}}</div>
        {if let Some(reply)=m.reply_to{view!{<button class="reply-preview" data-live=format!("seek-message:{}",reply.0)>{icon("reply")}{a.model.messages.get(&m.channel_id).and_then(|ms|ms.get(&reply)).map_or("Original message".into(),|m|format!("{}: {}",a.model.name(m.author_id),m.content.chars().take(100).collect::<String>()))}</button>}.into_any()}else{().into_any()}}
        <div class="markdown">{document.blocks.iter().map(|b|match b{Block::Code{text,..}=>view!{<pre><code>{text.clone()}</code></pre>}.into_any(),Block::Paragraph(v)|Block::Header(_,v)|Block::Quote(v)|Block::List(v)=>view!{<p>{v.iter().map(|v|inline(a,v,m)).collect_view()}</p>}.into_any()}).collect_view()}</div>
        {m.attachments.iter().map(attachment).collect_view()}
        <div class="reactions">{m.reactions.iter().map(|r|view!{<button class=if r.me{"reaction mine"}else{"reaction"} data-live=format!("react:{},{}",m.id.0,r.emoji) aria-label=format!("React with {}",r.emoji) aria-pressed=r.me.to_string()>{r.emoji.clone()}" "{r.count}</button>}).collect_view()}</div></div>
        <div class="message-actions">{ib("smile","Add Reaction",&format!("react:{},👍",m.id.0))}{ib("reply","Reply",&format!("reply:{}",m.id.0))}{ib("pin","Pin Message",&format!("pin:{}",m.id.0))}{if own{view!{{ib("pencil","Edit Message",&format!("edit:{}",m.id.0))}{ib("trash-2","Delete Message",&format!("delete:{}",m.id.0))}}.into_any()}else{().into_any()}}</div>
    </article>}.into_any()
}
fn attachment(f: &Attachment) -> AnyView {
    let original = format!("/api/v1/files/{}", f.id.0);
    let thumb = format!("{original}/thumbnail");
    if f.content_type.starts_with("image/") {
        if let Some(t) = &f.thumbnail {
            view!{<button class="attachment-image" data-view-image=original data-filename=f.filename.clone() aria-label=format!("Open {}",f.filename)><img src=thumb.clone() srcset=format!("{thumb} 1x") loading="lazy" width=t.width height=t.height alt=f.filename.clone()/></button>}.into_any()
        } else {
            view!{<button class="button secondary" data-view-image=original data-filename=f.filename.clone()>{format!("View {}",f.filename)}</button>}.into_any()
        }
    } else if f.content_type.starts_with("video/") {
        view! {<video controls preload="none" src=original aria-label=f.filename.clone()></video>}
            .into_any()
    } else {
        view!{<a class="file-card" href=original download=f.filename.clone()>{icon("file")}<span>{f.filename.clone()}<small>{format!("{} bytes",f.byte_size)}</small></span>{icon("download")}</a>}.into_any()
    }
}
fn voice(a: &App) -> AnyView {
    view!{<div class="voice-view"><div class="call-heading"><h2>"Voice Call"</h2>{button("Open Chat",&a.model.channels.values().find(|c|c.guild_id==a.guild&&c.kind==ChannelKind::Text).map_or("home".into(),|c|format!("channel:{}",c.id.0)),"secondary")}</div><div id="live-media-stage" class="participant-grid" aria-label="Call participants"></div><div class="call-controls">{ib("mic","Mute","media:mute")}{ib("headphones","Deafen","media:deafen")}{ib("video","Camera","media:camera")}<span class="desktop-share">{ib("monitor","Share Screen","media:share")}</span>{ib("volume-2","Audio Output","media:output")}{ib("phone-off","Leave Call","media:leave")}</div><p id="live-call-status" class="call-connection" role="status">"Connecting…"</p></div>}.into_any()
}
fn form(title: &str, kind: &str, fields: AnyView) -> AnyView {
    view!{<form data-live-form=kind.to_owned()><h2>{title.to_owned()}</h2>{fields}<button type="submit" class="button primary">"Save"</button></form>}.into_any()
}
fn dialog(a: &App) -> AnyView {
    let body=match a.dialog.as_str(){
        "settings"=>view!{<h2>"User Settings"</h2>{[ ("Profile","profile-edit"),("About Me","about"),("Password & Security","password"),("Devices","devices"),("Voice & Video","media-settings"),("Notifications","notifications")].into_iter().map(|(label,page)|button(label,&format!("dialog:{page}"),"nav-row")).collect_view()}{button("Log Out","logout","danger-text")}}.into_any(),
        "server"=>view!{<h2>"Server Settings"</h2>{[("Create Server","create"),("Join Server","join"),("Create Channel","channel"),("Invite People","invite"),("Roles","roles"),("Member Roles","member-role"),("Private Channel","private")].into_iter().map(|(label,page)|button(label,&format!("dialog:{page}"),"nav-row")).collect_view()}}.into_any(),
        "friend"=>form("Add Friend","friend",field("f-name","Username","","text")),
        "create"=>view!{{form("Create a Server","create",field("f-name","Server Name","","text"))}{button("Join a Server","dialog:join","link-button")}}.into_any(),
        "join"=>form("Join a Server","join",field("f-name","Invite Code","","text")),
        "channel"=>form("Create Channel","channel",view!{{field("f-name","Name","","text")}<label class="field">"Type"<select id="f-extra"><option value="text">"Text"</option><option value="voice">"Voice"</option><option value="category">"Category"</option></select></label><label class="field">"Category"<select id="f-value"><option value="">"None"</option>{a.model.channels.values().filter(|c|c.guild_id==a.guild&&c.kind==ChannelKind::Category).map(|c|view!{<option value=c.id.0.to_string()>{c.name.clone().unwrap_or_default()}</option>}).collect_view()}</select></label>}.into_any()),
        "invite"=>form("Invite People","invite",view!{<p>"Create an invite valid for 7 days."</p>{a.model.invites.values().map(|i|view!{<p><code>{i.code.clone()}</code><button class="link-button" data-copy=i.code.clone()>"Copy"</button></p>}).collect_view()}}.into_any()),
        "profile-edit"=>form("Profile","profile",view!{{field("f-name","Display Name",a.model.me.as_ref().and_then(|me|me.user.display_name.as_deref()).unwrap_or(""),"text")}{field("f-value","Status",a.model.me.as_ref().and_then(|me|me.user.status_text.as_deref()).unwrap_or(""),"text")}}.into_any()),
        "about"=>form("About Me","about",view!{{field("f-name","Pronouns","","text")}{field("f-value","About Me","","text")}}.into_any()),
        "password"=>form("Password & Security","password",view!{{field("f-name","Current Password (blank for Google-only)","","password")}{field("f-value","New Password","","password")}}.into_any()),
        "devices"=>view!{<h2>"Devices"</h2>{a.devices.iter().map(|d|view!{<div class="device-row"><span><strong>{d.device_name.clone()}</strong><small>{format!("{} {}",d.platform,if d.current{"· Current Device"}else{""})}</small></span>{button("Log Out",&format!("device-revoke:{}",d.id),"secondary")}</div>}).collect_view()}{button("Log Out All Other Devices","logout-others","danger-text")}}.into_any(),
        "media-settings"=>view!{<h2>"Voice & Video"</h2><div id="live-device-settings"></div>}.into_any(),
        "notifications"=>view!{{form("Notifications","notifications",view!{{field("f-name","Notify (all, mentions, nothing)","mentions","text")}{field("f-value","Muted (true or false)","false","text")}{field("f-extra","Suppress @everyone (true or false)","false","text")}}.into_any())}<section class="push-explainer"><h3>"Notifications when you're away"</h3><p>"Allow StickTalk to notify you about DMs and mentions. No message text appears on your lock screen. You can turn this off in your browser settings."</p><button class="button secondary" data-push-enable disabled=!a.push>"Enable Browser Notifications"</button></section>}.into_any(),
        "roles"=>view!{<h2>"Roles"</h2>{a.roles.values().filter(|r|Some(r.guild_id)==a.guild).map(|r|button(&r.name,&format!("role:{}",r.id.0),"nav-row")).collect_view()}{form("Create Role","role-create",field("f-name","Role Name","","text"))}}.into_any(),
        "member-role"=>form("Assign Member Roles","member-role",view!{<label class="field">"Member"<select id="f-name">{a.model.members.values().map(|m|view!{<option value=m.user.id.0.to_string()>{name(&m.user)}</option>}).collect_view()}</select></label><input id="f-value" type="hidden" value=""/>{a.roles.values().filter(|r|Some(r.guild_id)==a.guild&&r.id!=r.guild_id).map(|r|view!{<label class="live-checkbox"><input type="checkbox" data-member-role=r.id.0.to_string()/><span>{r.name.clone()}</span></label>}).collect_view()}<p class="muted-copy">"Replaces all roles for this member."</p>}.into_any()),
        "private"=>form("Private Channel","private",view!{<p>"Restrict this channel to one role. The owner retains access."</p><label class="field">"Allowed Role"<select id="f-name">{a.roles.values().filter(|r|Some(r.guild_id)==a.guild&&r.id!=r.guild_id).map(|r|view!{<option value=r.id.0.to_string()>{r.name.clone()}</option>}).collect_view()}</select></label>}.into_any()),
        "pins"=>view!{<h2>"Pinned Messages"</h2>{a.pins.iter().map(|m|message(a,m)).collect_view()}}.into_any(),
        "edit"=>{let text=a.channel.and_then(|c|a.model.messages.get(&c)).and_then(|ms|a.editing.and_then(|m|ms.get(&m))).map_or("",|m|m.content.as_str());form("Edit Message","edit",field("f-name","Message",text,"text"))},
        "emoji"=>{let emojis=if a.query.is_empty(){sticktalk_emoji::all().take(80).collect()}else{sticktalk_emoji::search(&a.query,80)};view!{<h2>"Emoji"</h2><label class="search"><span class="sr-only">"Search Emoji"</span><input id="emoji-query" type="search" value=a.query.clone() placeholder="Search emoji" data-emoji-query/></label><div class="emoji-grid">{emojis.into_iter().map(|e|view!{<button title=e.name() aria-label=e.name() data-live=format!("emoji:{}",e.emoji())>{e.emoji()}</button>}).collect_view()}</div>}.into_any()},
        "gif"=>view!{<h2>"GIFs"</h2><form data-gif-search><label class="field"><span>"Search GIFs"</span><input id="gif-query" type="search"/></label><button class="button secondary" type="submit">"Search"</button></form><div class="live-gif-results">{a.gifs.iter().map(|g|view!{<button data-live=format!("gif-send:{}",g.id)><video src=g.preview.url.clone() preload="none" muted loop playsinline controls width=g.preview.width height=g.preview.height aria-label=g.title.clone()></video><span>{g.title.clone()}</span></button>}).collect_view()}</div>}.into_any(),
        "profile"=>if let Some(p)=&a.profile{let u=&p.profile.user;view!{<div class="profile-banner"></div>{avatar(u,"large")}<h2>{name(u)}</h2><p>{format!("@{}",u.username)}</p><p>{p.profile.pronouns.clone()}</p><p>{p.profile.about_me.clone()}</p><p>{format!("{} mutual servers · {} mutual friends",p.mutual_servers.len(),p.mutual_friends.len())}</p>{button("Message",&format!("friend:dm,{}",u.id.0),"primary")}{button("Block",&format!("friend:block,{}",u.id.0),"danger-text")}}.into_any()}else{().into_any()},
        value if value.starts_with("role:")=>{let r=id(value.trim_start_matches("role:")).and_then(|id|a.roles.get(&id));if let Some(r)=r{form("Edit Role","role-save",view!{{field("f-name","Role Name",&r.name,"text")}{field("f-value","Color",&format!("#{:06x}",r.color),"color")}<input id="f-permissions" type="hidden" value=r.permissions.bits().to_string()/><div class="live-permissions">{role_permissions(r)}</div><label class="live-checkbox"><input id="f-hoist" type="checkbox" checked=r.hoist/>"Display role members separately"</label><label class="live-checkbox"><input id="f-mentionable" type="checkbox" checked=r.mentionable/>"Allow anyone to mention this role"</label><input id="f-extra" type="hidden" value=r.id.0.to_string()/>{button("Delete Role",&format!("role-delete:{}",r.id.0),"danger-text")}}.into_any())}else{().into_any()}},
        _=>view!{<h2>"Settings"</h2>}.into_any(),
    };
    view!{<div class="modal-backdrop" data-live-close><section class="modal live-modal" data-dialog=a.dialog.clone() role="dialog" aria-modal="true" aria-label="Settings" tabindex="-1"><div class="live-modal-close">{ib("x","Close","close")}</div>{body}</section></div>}.into_any()
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
    let permissions = [
        ("View Channels", Permissions::VIEW_CHANNEL),
        ("Send Messages", Permissions::SEND_MESSAGES),
        ("Read History", Permissions::READ_HISTORY),
        ("Attach Files", Permissions::ATTACH_FILES),
        ("Add Reactions", Permissions::ADD_REACTIONS),
        ("Manage Messages", Permissions::MANAGE_MESSAGES),
        ("Manage Channels", Permissions::MANAGE_CHANNELS),
        ("Manage Server", Permissions::MANAGE_GUILD),
        ("Create Invites", Permissions::CREATE_INVITES),
        ("Manage Roles", Permissions::MANAGE_ROLES),
        ("Kick Members", Permissions::KICK_MEMBERS),
        ("Ban Members", Permissions::BAN_MEMBERS),
        ("Connect", Permissions::CONNECT),
        ("Speak", Permissions::SPEAK),
        ("Share Screen", Permissions::STREAM),
        ("Video", Permissions::VIDEO),
        ("Voice Activity", Permissions::USE_VOICE_ACTIVITY),
        ("Administrator", Permissions::ADMINISTRATOR),
    ];
    view!{{permissions.into_iter().map(|(label,bit)|view!{<label class="live-checkbox"><span>{label}</span><input type="checkbox" checked=role.permissions.contains(bit) data-role-bit=bit.bits().to_string()/></label>}).collect_view()}}.into_any()
}
