use super::*;
mod settings;
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
    view! {<span class=format!("avatar {size}")>{if let Some(id)=u.avatar_id{view!{<img class="avatar-face" src=format!("/api/v1/files/{}/thumbnail",id.0) alt="" loading="lazy"/>}.into_any()}else{view!{<span class="avatar-face">{initials}</span>}.into_any()}}</span>}
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
    if !a.ready && a.model.cursor.is_none() && (a.model.me.is_some() || a.status == "Connecting…")
    {
        return view!{<div class="app-shell skeleton-shell" role="status" aria-label="Loading conversations"><div class="left-panel"><nav class="server-rail"><img src="/public/icons/icon-192.png" width="48" height="48" alt="StickTalk"/></nav><aside class="channel-sidebar"><div class="sidebar-header">"StickTalk"</div><div class="skeleton-lines"><span></span><span></span><span></span><span></span></div></aside></div><main class="main-pane"><header class="channel-header"><strong>"StickTalk"</strong></header><div class="connection-banner">"Reconnecting…"</div><div class="skeleton-messages"><div></div><div></div><div></div></div><div class="skeleton-composer"></div></main></div>}.into_any();
    }
    view!{<div id="sticktalk" data-live-app data-screen=a.screen.clone() data-self-id=a.model.me.as_ref().map(|me|me.user.id.0.to_string())>
        <div hidden inner_html=include_str!("../../public/icons/sprite.svg")></div>
        {if a.model.me.is_none(){auth(a)}else{shell(a)}}
        {if a.error.is_empty(){().into_any()}else{view!{<div class="live-error" role="alert">{a.error.clone()}{ib("x","Dismiss","dismiss-error")}</div>}.into_any()}}
        {if a.dialog.is_empty(){().into_any()}else{a.dialog_render.map_or_else(loading_dialog, |render|render(a))}}

    </div>}.into_any()
}
fn auth(a: &App) -> AnyView {
    if a.status == "Reconnecting…" {
        return view!{<main class="auth-page"><section class="auth-card"><h1>"Reconnecting…"</h1><p class="muted-copy">"Your conversations will appear when the server is available."</p>{button("Try Again","recover","primary")}</section></main>}.into_any();
    }

    let signup = a.signup;
    let register = a.register;
    view!{<main class="auth-page"><form class="auth-card" data-live-form="auth"><div class="auth-brand"><img class="auth-logo" src="/public/icons/icon-192.png" alt="" width="48" height="48"/><span>"StickTalk"</span></div><h1>{if signup{"Choose a username"}else if register{"Create an account"}else{"Welcome back!"}}</h1><p class="auth-subtitle">{if signup{"Choose how people will find you on StickTalk."}else if register{"Create your StickTalk account."}else{"Log in to catch up with your friends."}}</p>
        <div class="field"><label for="auth-username">"Username"</label><input id="auth-username" name="username" autocomplete="username" autocapitalize="none" autocorrect="off" spellcheck="false" required minlength="3" maxlength="32" autofocus /></div>
        {if signup{().into_any()}else{view!{<div class="field"><label for="auth-password">"Password"</label><input id="auth-password" name="password" type="password" autocomplete=if register{"new-password"}else{"current-password"} required minlength="12" maxlength="128"/></div>}.into_any()}}
        <button class="button primary" type="submit" disabled=!a.status.is_empty()>{if signup{"Continue"}else if register{"Create Account"}else{"Log In"}}</button>
        <p class="auth-status" role="status">{a.status.clone()}</p>
        {if a.google&&!signup {view!{<a class="button secondary google-button" href="/api/v1/browser/google/start"><img src="/public/google-g.svg" alt="" width="20" height="20"/>"Continue with Google"</a>}.into_any()}else{().into_any()}}
        {if signup{().into_any()}else{view!{<p class="auth-switch">{if register{"Already have an account? "}else{"Need an account? "}}<button type="button" data-live="register">{if register{"Log in"}else{"Register"}}</button></p>}.into_any()}}
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
            {a.model.guilds.values().map(|g|view!{<button class=format!("server-icon {}",if a.guild==Some(g.guild.id){"selected"}else{""}) data-live=format!("guild:{}",g.guild.id.0) title=g.guild.name.clone() aria-label=g.guild.name.clone()>{if let Some(id)=g.guild.icon_id{view!{<img src=format!("/api/v1/files/{}/thumbnail",id.0) alt="" loading="lazy"/>}.into_any()}else{view!{<span>{g.guild.name.chars().take(2).collect::<String>()}</span>}.into_any()}}</button>}).collect_view()}
            {ib("plus","Add a Server","dialog:create")}
        </nav><aside class="channel-sidebar"><div class="sidebar-header"><strong>{guild_name}</strong>{if a.guild.is_some(){view!{{ib("plus","Create Channel","dialog:channel")}{ib("settings","Server Settings","dialog:server")}}.into_any()}else{ib("users","New Group Message","dialog:group")}}</div>
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
        <main class="main-pane"><header class="channel-header"><div class="mobile-menu">{ib("menu","Open Channels","panel:left")}</div>{icon(if channel.is_some_and(|c|c.kind==ChannelKind::Voice){"volume-2"}else if channel.is_some(){"hash"}else{"users"})}<h1>{title.clone()}</h1><div class="header-actions">{if a.screen=="voice"{ib("message-circle","Open Chat",&a.model.channels.values().find(|c|c.guild_id==a.guild&&c.kind==ChannelKind::Text).map_or("home".into(),|c|format!("channel:{}",c.id.0)))}else{view!{{ib("bell","Notification Settings","dialog:channel-notifications")}{ib("pin","Pinned Messages","dialog:pins")}{ib("users","Member List","panel:right")}}.into_any()}}</div></header>
            {if !a.status.is_empty(){view!{<div class="connection-banner" role="status">{a.status.clone()}</div>}.into_any()}else{().into_any()}}
            {if a.screen=="voice"{a.voice_render.map_or_else(loading_call, |render|render(a))}else if channel.is_some(){chat(a,&title)}else{home(a)}}
        </main>
        <aside class=format!("member-list {}",if a.panel=="right"{"is-open"}else{""}) aria-label="Members">{member_groups(a)}{button("Load More","members-more","link-button")}</aside>
        {if channel.is_none(){view!{<nav class="bottom-tabs" aria-label="Main navigation">{button("Home","home","active")}{button("Notifications","dialog:notifications","")}{button("You","dialog:settings","")}</nav>}.into_any()}else{().into_any()}}
    </div>}.into_any()
}
fn home(a: &App) -> AnyView {
    view!{<div class="live-home"><div class="friends-toolbar"><h2>"Friends"</h2>{button("Add Friend","dialog:friend","primary")}{ib("users","New Group Message","dialog:group")}{button("Join a Server","dialog:join","secondary")}</div>
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
    view!{<div class="chat-body"><div id="live-messages" class="message-list" role="log" aria-label="Messages" tabindex="0" data-channel=c.0.to_string() data-history-generation=a.history_generation.to_string() data-window-start=a.window_rank.to_string() data-window-end=(a.window_rank+shown.len()as u64).to_string() data-total=count.to_string()>
        <div style=format!("height:{}px",a.window_rank*88) aria-hidden="true"></div>
        {if shown.is_empty(){view!{<div class="conversation-intro"><span class="intro-icon">{icon("hash")}</span><h2>{format!("Welcome to #{title}!")}</h2><p>{if a.history_busy{"Loading messages…"}else{"This is the beginning of this conversation."}}</p></div>}.into_any()}else{().into_any()}}
        {shown.iter().enumerate().map(|(i,m)|message(a,m,crate::wire::continues_group(m.author_id.0,m.created_at,i.checked_sub(1).map(|p|(shown[p].author_id.0,shown[p].created_at))))).collect_view()}
        <div style=format!("height:{}px",bottom*88) aria-hidden="true"></div>
        </div><div class="composer-wrap">
        {if let Some(reply)=a.reply{view!{<div class="reply-preview">{format!("Replying to {}",messages.and_then(|m|m.get(&reply)).map_or("message".into(),|m|a.model.name(m.author_id)))}{ib("x","Cancel Reply","cancel-reply")}</div>}.into_any()}else{().into_any()}}
        {a.attachments.iter().enumerate().map(|(i,f)|view!{<div class="live-upload"><span>{f.filename.clone()}</span>{ib("x","Remove Attachment",&format!("remove-attachment:{i}"))}</div>}).collect_view()}
        {if a.uploading{view!{<div class="live-upload" role="status">{a.upload_progress.clone()}{button("Pause","upload-cancel","secondary")}</div>}.into_any()}else{().into_any()}}
        {if a.completions.is_empty(){().into_any()}else{view!{<div id="live-completions" class="autocomplete" role="listbox" aria-label="Suggestions">{a.completions.iter().enumerate().map(|(i,(label,insert,kind))|view!{<button id=format!("completion-{i}") role="option" aria-selected=if i==0{"true"}else{"false"} data-completion=insert.clone()><span class="completion-avatar">{match kind.as_str() {"Role"=>icon("shield"),"Channel"=>icon("hash"),"Emoji"=>view!{<span>{insert.clone()}</span>}.into_any(),_=>view!{<span>{label.chars().take(1).collect::<String>()}</span>}.into_any()}}</span><span>{label.clone()}</span><small>{if kind=="User"{insert.clone()}else{kind.clone()}}</small></button>}).collect_view()}</div>}.into_any()}}
        <div class="composer"><button type="button" class="icon-button" title="Add Attachment" aria-label="Add Attachment" data-live="pick-file">{icon("plus-circle")}</button><input type="file" id="live-file" hidden/><label class="composer-input"><span class="sr-only">"Message"</span><textarea id="live-input" rows="1" placeholder=format!("Message #{title}") aria-label="Message" role="combobox" aria-autocomplete="list" aria-controls="live-completions" aria-expanded=(!a.completions.is_empty()).to_string() maxlength="4000"></textarea></label>{button("GIF","dialog:gif","link-button composer-gif")}{ib("smile","Emoji Picker","dialog:emoji")}<button class="icon-button" aria-label="Send Message" title="Send Message" data-live="send-input" disabled={a.uploading||a.preparing_send||!a.ready}>{icon("send")}</button></div>
        <div class="typing-indicator">{crate::wire::typing_label(&a.model.typing.iter().filter(|((channel,_),expiry)|*channel==c&&**expiry>Browser.unix_ms()).map(|((_,u),_)|a.model.name(*u)).take(3).collect::<Vec<_>>())}</div>
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
    view!{<article class=format!("message {} {}",if continuation{"continuation"}else{"group-head"},if mention{"mentioned"}else{""}) data-live-message=m.id.0.to_string() tabindex="0" aria-label=format!("Message from {}",name(&author))>
        <div class="avatar-gutter">{if continuation{view!{<time class="gutter-time">{short_time(m.created_at)}</time>}.into_any()}else{view!{<button data-live=format!("profile:{}",m.author_id.0) aria-label=format!("{}'s profile",name(&author))>{avatar(&author,"message-avatar")}</button>}.into_any()}}</div>
        <div class="message-content">{if continuation{().into_any()}else{view!{<div class="message-meta"><button class="author" style=format!("color:{}",color(a,m.author_id)) data-live=format!("profile:{}",m.author_id.0)>{name(&author)}</button><time>{timestamp(m.created_at)}</time></div>}.into_any()}}
        {if let Some(reply)=m.reply_to{view!{<button class="reply-preview" data-live=format!("seek-message:{}",reply.0)>{icon("reply")}{a.model.messages.get(&m.channel_id).and_then(|ms|ms.get(&reply)).map_or("Original message".into(),|m|format!("{}: {}",a.model.name(m.author_id),m.content.chars().take(100).collect::<String>()))}</button>}.into_any()}else{().into_any()}}
        <div class="markdown">{document.blocks.iter().map(|b|match b{Block::Code{text,..}=>view!{<pre><code>{text.clone()}</code></pre>}.into_any(),Block::Paragraph(v)|Block::Header(_,v)|Block::Quote(v)|Block::List(v)=>view!{<p>{v.iter().map(|v|inline(a,v,m)).collect_view()}</p>}.into_any()}).collect_view()}</div>
        {if m.edited_at.is_some(){view!{<small class="edited">"(edited)"</small>}.into_any()}else{().into_any()}}{m.attachments.iter().map(attachment).collect_view()}
        <div class="reactions">{m.reactions.iter().map(|r|view!{<button class=if r.me{"reaction mine"}else{"reaction"} data-live=format!("react:{},{}",m.id.0,r.emoji) aria-label=format!("React with {}",r.emoji) aria-pressed=r.me.to_string()>{r.emoji.clone()}" "{r.count}</button>}).collect_view()}</div></div>
        <div class="message-actions">{ib("smile","Add Reaction",&format!("react:{},👍",m.id.0))}{ib("reply","Reply",&format!("reply:{}",m.id.0))}{ib("pin","Pin Message",&format!("pin:{}",m.id.0))}{if own{view!{{ib("pencil","Edit Message",&format!("edit:{}",m.id.0))}{ib("trash-2","Delete Message",&format!("delete:{}",m.id.0))}}.into_any()}else{().into_any()}}</div>
    </article>}.into_any()
}
fn attachment(f: &Attachment) -> AnyView {
    let original = format!("/api/v1/files/{}", f.id.0);
    let thumb = format!("{original}/thumbnail");
    if f.content_type.starts_with("image/") {
        if let Some(t) = &f.thumbnail {
            view!{<button class="attachment-image" data-view-image=original data-filename=f.filename.clone() data-bytes=f.byte_size.to_string() aria-label=format!("Open {}",f.filename)><img src=thumb.clone() srcset=format!("{thumb} 1x") loading="lazy" width=t.width height=t.height alt=f.filename.clone()/></button>}.into_any()
        } else {
            view!{<button class="button secondary" data-view-image=original data-filename=f.filename.clone()>{format!("View {}",f.filename)}</button>}.into_any()
        }
    } else if crate::wire::video_hint(&f.content_type, &f.filename) {
        view! {<div class="attachment-video"><video controls playsinline preload="none" src=original.clone() aria-label=f.filename.clone()></video><a href=original download=f.filename.clone()>{format!("Download {}",f.filename)}</a></div>}
            .into_any()
    } else {
        view!{<a class="file-card" href=original download=f.filename.clone()>{icon("file")}<span>{f.filename.clone()}<small>{format!("{} bytes",f.byte_size)}</small></span>{icon("download")}</a>}.into_any()
    }
}
fn voice(a: &App) -> AnyView {
    view!{<div class="voice-view" data-call-channel=a.voice_channel.map_or(String::new(),|id|id.0.to_string())><button class="call-minimize" data-live="media:minimize" aria-label="Minimize call">{icon("chevron-down")}<span>"Voice Connected"</span></button><div id="live-media-stage" class="participant-grid" aria-label="Call participants"></div><div class="call-controls">{ib("mic","Mute","media:mute")}{ib("headphones","Deafen","media:deafen")}{ib("video","Camera","media:camera")}<span class="desktop-share">{ib("monitor","Share Screen","media:share")}</span>{ib("volume-2","Audio Output","media:output")}{ib("phone-off","Leave Call","media:leave")}</div><p id="live-call-status" class="call-connection" role="status">"Connecting…"</p></div>}.into_any()
}
fn form(title: &str, kind: &str, fields: AnyView) -> AnyView {
    let (label, class) = match kind {
        "delete-account" => ("Delete Account", "danger"),
        "channel-delete" => ("Delete Channel", "danger"),
        "invite-revoke" => ("Revoke Invite", "danger"),
        "unban" => ("Revoke Ban", "primary"),
        "ban" => ("Ban Member", "danger"),
        "role-create" => ("Create Role", "primary"),
        "guild-icon-remove" => ("Remove Icon", "danger"),
        "create" => ("Create Server", "primary"),
        "channel" => ("Create Channel", "primary"),
        "join" => ("Join Server", "primary"),
        "invite" => ("Create Invite", "primary"),
        _ => ("Save", "primary"),
    };
    view!{<form data-live-form=kind.to_owned()><h2>{title.to_owned()}</h2>{fields}<button type="submit" class=format!("button {class}")>{label}</button></form>}.into_any()
}
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
        "group"=>form("New Group Message","group",view!{{field("f-name","Group Name","","text")}<fieldset class="group-recipients"><legend>"Friends"</legend>{a.model.relationships.values().filter(|r|r.kind==RelationshipKind::Friend).map(|r|view!{<label class="live-checkbox">{avatar(&r.user,"small")}<span>{name(&r.user)}</span><input type="checkbox" data-group-user=r.user.id.0.to_string()/></label>}).collect_view()}</fieldset><input id="f-value" type="hidden"/><p class="muted-copy">"Choose 2 to 15 friends."</p>}.into_any()),
        "friend"=>form("Add Friend","friend",field("f-name","Username","","text")),
        "create"=>view!{{form("Create a Server","create",field("f-name","Server Name","","text"))}{button("Join a Server","dialog:join","link-button")}}.into_any(),
        "join"=>form("Join a Server","join",field("f-name","Invite Code","","text")),
        "channel"=>form("Create Channel","channel",view!{{field("f-name","Name","","text")}<label class="field">"Type"<select id="f-extra"><option value="text">"Text"</option><option value="voice">"Voice"</option><option value="category">"Category"</option></select></label><label class="field">"Category"<select id="f-value"><option value="">"None"</option>{a.model.channels.values().filter(|c|c.guild_id==a.guild&&c.kind==ChannelKind::Category).map(|c|view!{<option value=c.id.0.to_string()>{c.name.clone().unwrap_or_default()}</option>}).collect_view()}</select></label>}.into_any()),
        "invite"=>settings::invites(a),
        "profile-edit"=>profile_editor(a),
        "about"=>form("About Me","about",view!{{field("f-name","Pronouns",a.profile.as_ref().map_or("",|p|p.profile.pronouns.as_str()),"text")}<p class="muted-copy">"Optional. Let people know how to refer to you."</p><label class="field"><span>"About Me"</span><textarea id="f-value" rows="4" maxlength="190" aria-describedby="bio-help">{a.profile.as_ref().map_or("",|p|p.profile.about_me.as_str()).to_owned()}</textarea><small id="bio-help">"A little about you, up to 190 characters."</small></label>{field("f-extra","Banner Color",&format!("#{:06x}",a.profile.as_ref().map_or(0x496c63,|p|p.profile.banner_color)),"color")}<p class="muted-copy">"Choose the color at the top of your profile."</p>}.into_any()),
        "password"=>form("Password & Security","password",view!{{field("f-name",if a.google{"Current Password (or verify with Google below)"}else{"Current Password"},"","password")}{field("f-value","New Password","","password")}{if a.google{button("Verify with Google","reauth-google:password","secondary")}else{().into_any()}}{if a.google{view!{<p class="muted-copy">"Google proof is valid for one password change within 5 minutes. Re-enter the new password after returning."</p>}.into_any()}else{().into_any()}}}.into_any()),
        "delete-account"=>form("Delete Account Permanently","delete-account",view!{<p>"This erases your messages and attachments and signs out all devices. Encrypted backups expire after 14 days. Your username is held for 30 days."</p>{field("f-name",if a.google{"Current Password (or verify with Google)"}else{"Current Password"},"","password")}{field("f-value","Type DELETE to confirm","","text")}{if a.google{button("Verify with Google","reauth-google:delete_account","secondary")}else{().into_any()}}}.into_any()),
        "devices"=>view!{<h2>"Devices"</h2>{a.devices.iter().map(|d|view!{<div class="device-row"><span><strong>{d.device_name.clone()}</strong><small>{format!("{} {}",d.platform,if d.current{"· Current Device"}else{""})}</small></span>{button("Log Out",&format!("device-revoke:{}",d.id),"secondary")}</div>}).collect_view()}{button("Log Out All Other Devices","logout-others","danger-text")}}.into_any(),
        "media-settings"=>view!{<h2>"Voice & Video"</h2><div id="live-device-settings"></div>}.into_any(),
        "notifications"|"channel-notifications"=>settings::notifications(a),
        value if value.starts_with("target-notifications:")=>settings::notifications(a),
        "roles"=>settings::roles(a),
        "member-role"=>settings::member_roles(a),
        "private"=>form("Private Channel","private",view!{<p>"Restrict this channel to one role. The owner retains access."</p><label class="field">"Allowed Role"<select id="f-name">{a.roles.values().filter(|r|Some(r.guild_id)==a.guild&&r.id!=r.guild_id).map(|r|view!{<option value=r.id.0.to_string()>{r.name.clone()}</option>}).collect_view()}</select></label>}.into_any()),
        "pins"=>view!{<h2>"Pinned Messages"</h2>{a.pins.iter().map(|m|message(a,m,false)).collect_view()}}.into_any(),
        "edit"=>{let text=a.channel.and_then(|c|a.model.messages.get(&c)).and_then(|ms|a.editing.and_then(|m|ms.get(&m))).map_or("",|m|m.content.as_str());form("Edit Message","edit",field("f-name","Message",text,"text"))},
        "emoji"=>{let emojis=if a.query.is_empty(){sticktalk_emoji::all().take(80).collect()}else{sticktalk_emoji::search(&a.query,80)};view!{<h2>"Emoji"</h2><label class="search"><span class="sr-only">"Search Emoji"</span><input id="emoji-query" type="search" value=a.query.clone() placeholder="Search emoji" data-emoji-query/></label><div class="emoji-grid">{emojis.into_iter().map(|e|view!{<button title=e.name() aria-label=e.name() data-live=format!("emoji:{}",e.emoji())>{e.emoji()}</button>}).collect_view()}</div>}.into_any()},
        "gif" if matches!(a.gif_capability,GifCapability::Disabled)=>view!{<h2>"GIFs"</h2><p class="muted-copy">"GIF search is not enabled on this server. You can still upload an image or use emoji."</p>}.into_any(),
        "gif"=>view!{<h2>"GIFs"</h2><form data-gif-search><label class="field"><span>"Search GIFs"</span><input id="gif-query" type="search"/></label><button class="button secondary" type="submit">"Search"</button></form><div class="live-gif-results">{a.gifs.iter().map(|g|view!{<button data-live=format!("gif-send:{}",g.id)><video src=g.preview.url.clone() preload="none" muted loop playsinline controls width=g.preview.width height=g.preview.height aria-label=g.title.clone()></video><span>{g.title.clone()}</span></button>}).collect_view()}</div>}.into_any(),
        "profile"=>if let Some(p)=&a.profile{let u=&p.profile.user;view!{<div class="profile-banner"></div>{avatar(u,"large")}<h2>{name(u)}</h2><p>{format!("@{}",u.username)}</p><p>{p.profile.pronouns.clone()}</p><p>{p.profile.about_me.clone()}</p><p>{format!("{} mutual servers · {} mutual friends",p.mutual_servers.len(),p.mutual_friends.len())}</p>{button("Message",&format!("friend:dm,{}",u.id.0),"primary")}{button("Block",&format!("friend:block,{}",u.id.0),"danger-text")}}.into_any()}else{().into_any()},
        value if value.starts_with("role:")=>{let r=id(value.trim_start_matches("role:")).and_then(|id|a.roles.get(&id));if let Some(r)=r{form("Edit Role","role-save",view!{<fieldset disabled=!settings::can(a,Permissions::MANAGE_ROLES)>{field("f-name","Role Name",&r.name,"text")}{field("f-value","Color",&format!("#{:06x}",r.color),"color")}<input id="f-permissions" type="hidden" value=r.permissions.bits().to_string()/><div class="live-permissions">{role_permissions(r)}</div><label class="live-checkbox"><input id="f-hoist" type="checkbox" checked=r.hoist/>"Display role members separately"</label><label class="live-checkbox"><input id="f-mentionable" type="checkbox" checked=r.mentionable/>"Allow anyone to mention this role"</label><input id="f-extra" type="hidden" value=r.id.0.to_string()/>{if r.id!=r.guild_id{button("Delete Role",&format!("role-delete:{}",r.id.0),"danger-text")}else{().into_any()}}</fieldset>}.into_any())}else{().into_any()}},
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
    view!{<div class="modal-backdrop" data-live-close><section class=format!("modal live-modal {}",if a.dialog=="profile"{"profile-popout"}else{""}) data-dialog=a.dialog.clone() role="dialog" aria-modal="true" aria-label="Settings" tabindex="-1"><div class="live-modal-close">{ib("x","Close","close")}</div>{body}</section></div>}.into_any()
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
    view!{{groups.into_iter().map(|(title,permissions)|view!{<section class="permission-group"><h3>{title}</h3>{permissions.into_iter().map(|(label,bit)|view!{<label class="live-checkbox"><span>{label}</span><input type="checkbox" checked=role.permissions.contains(bit) data-role-bit=bit.bits().to_string()/></label>}).collect_view()}</section>}).collect_view()}}.into_any()
}

#[lazy]
pub(super) fn load_dialog() -> fn(&App) -> AnyView {
    dialog
}
#[lazy]
pub(super) fn load_call() -> fn(&App) -> AnyView {
    voice
}
fn loading_dialog() -> AnyView {
    view!{<div class="modal-backdrop"><section class="modal live-modal" role="dialog" aria-modal="true" aria-label="Loading"><p role="status">"Loading…"</p></section></div>}.into_any()
}
fn loading_call() -> AnyView {
    view! {<div class="voice-view" role="status">"Connecting…"</div>}.into_any()
}

fn member_groups(a: &App) -> AnyView {
    if a.guild.is_none() {
        if let Some(c) = a.channel.and_then(|id| a.model.channels.get(&id)) {
            return view!{<h2 class="category">{format!("Members — {}",c.recipient_ids.len())}</h2>{c.recipient_ids.iter().filter_map(|id|a.model.users.get(id)).map(|u|view!{<button class="member-row" data-live=format!("profile:{}",u.id.0)>{avatar(u,"small")}<span>{name(u)}</span></button>}).collect_view()}{if c.kind==ChannelKind::GroupDm {button("Manage Group","dialog:group-members","nav-row")}else{().into_any()}}}.into_any();
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
        view!{<section class=if label=="Offline"{"member-group offline"}else{"member-group"}>
            <h2 class="category">{format!("{} — {}",label,members.len())}</h2>
            {members.into_iter().map(|m|view!{<button class="member-row" data-live=format!("profile:{}",m.user.id.0)>{avatar(&m.user,"small")}<span data-member-name=m.user.id.0.to_string() style=format!("color:{}",color(a,m.user.id))>{m.nickname.clone().unwrap_or_else(||name(&m.user))}{if m.role==MemberRole::Owner {icon("crown")} else {().into_any()}}</span></button>}).collect_view()}
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
    view!{<div class="settings-identity">{avatar(u,"settings-avatar")}<span><strong>{name(u)}</strong><small>{format!("@{}",u.username)}</small><button class="muted-copy" data-live="dialog:profile-edit">"Edit Profile"</button></span></div>
        <label class="settings-search"><span class="sr-only">"Search settings"</span><input type="search" placeholder="Search settings" data-settings-search/></label>
        <h2 class="category">"User Settings"</h2>
        {[( "user-round","My Account","account"),("user-round","Profiles","profile-edit"),("shield","Privacy & Safety","privacy"),("shield","Password & Security","password"),("monitor","Sessions","devices"),("bell","Notifications","notifications")].into_iter().map(|(glyph,label,page)|view!{<button class=format!("settings-nav-row {}",if a.dialog==page{"selected"}else{""}) data-live=format!("dialog:{page}")>{icon(glyph)}<span>{label}</span>{icon("chevron-right")}</button>}).collect_view()}
        <h2 class="category">"App Settings"</h2>
        <button class=format!("settings-nav-row {}",if a.dialog=="media-settings"{"selected"}else{""}) data-live="dialog:media-settings">{icon("headphones")}<span>"Voice & Video"</span>{icon("chevron-right")}</button>
        <button class=format!("settings-nav-row {}",if a.dialog=="appearance"{"selected"}else{""}) data-live="dialog:appearance">{icon("monitor")}<span>"Appearance"</span>{icon("chevron-right")}</button>
        <div class="settings-divider"></div>{button("Log Out","logout","danger-text settings-nav-row")}<small class="muted-copy">"StickTalk 0.1.0"</small>
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
    view!{<section class=format!("live-modal settings-page {}",if a.dialog=="settings"{"settings-root"}else{"settings-subpage"}) data-dialog=a.dialog.clone() role="dialog" aria-modal="true" aria-label="User Settings" tabindex="-1">
        <nav class="settings-nav" aria-label="User settings">{settings_nav(a)}</nav>
        <div class="settings-main"><header class="settings-topbar"><div class="settings-back">{ib("arrow-left","Back to Settings","dialog:settings")}</div><strong>{title}</strong>{ib("x","Close","close")}</header><div class="settings-content">{body}</div></div>
        <header class="settings-mobile-header"><h1>"Settings"</h1>{ib("x","Close","close")}</header>
    </section>}.into_any()
}
fn account(a: &App) -> AnyView {
    let me = &a.model.me.as_ref().unwrap().user;
    view!{<h2>"My Account"</h2><section class="account-card"><div class="profile-banner"></div><div class="account-identity">{avatar(me,"large")}<div><strong>{name(me)}</strong><p>{format!("@{}",me.username)}</p></div>{button("Edit Profile","dialog:profile-edit","secondary")}</div><div class="settings-field-row"><span><strong>"Display Name"</strong><p>{name(me)}</p></span>{button("Edit","dialog:profile-edit","secondary")}</div><div class="settings-field-row"><span><strong>"Username"</strong><p>{me.username.clone()}</p></span></div></section><div class="settings-divider"></div><h2>"Password & Security"</h2>{button("Change Password","dialog:password","secondary")}{if a.google{view!{<h2>"Google Sign-in"</h2>{field("google-current-password","Current Password (or verify with Google first)","","password")}{button("Verify with Google to Link","reauth-google:link","secondary")}{button("Link Google","google-link","secondary")}{button("Verify with Google to Unlink","reauth-google:unlink","secondary")}{button("Unlink Google","google-unlink","secondary")}}.into_any()}else{().into_any()}}<div class="settings-divider"></div><h2>"Devices"</h2><p class="muted-copy">"Manage where you’re signed in."</p>{button("Manage Devices","dialog:devices","secondary")}<section class="danger-zone"><h3>"Danger Zone"</h3><p class="muted-copy">"Permanently delete your account and messages. This cannot be undone."</p>{button("Delete Account","dialog:delete-account","danger-text")}</section>}.into_any()
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
    view!{<h2>"Profiles"</h2><div class="profile-editor">
        <aside class="profile-preview" aria-label="Profile preview"><div class="profile-banner" style=format!("background:#{banner:06x}")></div>{avatar(me,"large")}<div class="profile-preview-copy"><h2 data-preview-name>{name(me)}</h2><p class="profile-username">{format!("@{}",me.username)}</p><p data-preview-status>{me.status_text.clone().unwrap_or_default()}</p><div class="settings-divider"></div><h3>"About Me"</h3><p>{profile.map_or("",|p|p.about_me.as_str()).to_owned()}</p></div></aside>
        <div><div class="avatar-edit">{avatar(me,"large")}<div><label class="button secondary" for="live-avatar">"Change Avatar"</label><input type="file" id="live-avatar" accept="image/png,image/jpeg,image/webp" hidden/>{if me.avatar_id.is_some(){button("Remove Avatar","avatar-remove","danger-text")}else{().into_any()}}</div></div>
        {if a.uploading{view!{<p role="status">{a.upload_progress.clone()}</p>}.into_any()}else{().into_any()}}
        <form data-live-form="profile" data-profile-editor data-fallback-name=me.username.clone()><div class="field"><label for="f-name">"Display Name"</label><input id="f-name" type="text" maxlength="64" value=me.display_name.clone().unwrap_or_default() aria-describedby="display-name-help"/><small id="display-name-help">"How you appear in conversations. Leave blank to use your username."</small></div><div class="field"><label for="f-value">"Status"</label><input id="f-value" type="text" maxlength="128" value=me.status_text.clone().unwrap_or_default() aria-describedby="status-help"/><small id="status-help">"Let friends know what you’re up to."</small></div>
        <div class="profile-save-bar" hidden><span role="status">"You have unsaved changes."</span><button type="button" class="button link-button" data-profile-reset>"Reset"</button><button type="submit" class="button primary">"Save changes"</button></div></form>
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
    view!{<h2>"Group Members"</h2>{c.recipient_ids.iter().filter_map(|id|a.model.users.get(id)).map(|u|view!{<div class="device-row">{avatar(u,"small")}<span>{name(u)}</span>{if owner&&Some(u.id)!=c.owner_id {view!{{button("Remove",&format!("group-remove:{}",u.id.0),"danger-text")}{button("Make Owner",&format!("group-owner:{}",u.id.0),"secondary")}}.into_any()}else{().into_any()}}</div>}).collect_view()}
    {form("Add Friend","group-add",view!{<label class="field">"Friend"<select id="f-name">{a.model.relationships.values().filter(|r|r.kind==RelationshipKind::Friend&&!c.recipient_ids.contains(&r.user.id)).map(|r|view!{<option value=r.user.id.0.to_string()>{name(&r.user)}</option>}).collect_view()}</select></label>}.into_any())}
    {button("Leave Group","group-leave","danger-text")}}.into_any()
}
