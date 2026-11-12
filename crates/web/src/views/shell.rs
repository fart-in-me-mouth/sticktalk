use super::*;

pub fn shell(vm: &AppVm) -> AnyView {
    let home = matches!(vm.screen.as_str(), "home" | "dm" | "notifications");
    view! {
        <div class=format!("app-shell panel-{}",vm.panel)>
            <div class="left-panel" aria-label="Servers and channels">{rail(vm,home)}{sidebar(vm,home)}</div>
            <div class="panel-scrim" data-action="close" aria-hidden="true"></div>
            <main class="main-pane" aria-label="Conversation">
                {header(vm,home)}
                {if vm.offline{view!{<div class="connection-banner" role="status">{icon("wifi-off")} "Reconnecting…" <span>"Your messages will appear when you're back online."</span></div>}.into_any()}else{().into_any()}}
                {match vm.screen.as_str(){"home"=>screens::home(vm),"notifications"=>screens::notifications(vm),"voice"=>screens::voice(vm),"empty"=>empty("hash","This is the beginning of #general","Send a Message","screen:chat"),"loading"=>view!{<div class="skeleton-messages" role="status" aria-label="Loading messages">{(0..6).map(|_|view!{<div class="skeleton-row"><span></span><div><span></span><span></span></div></div>}).collect_view()}</div>}.into_any(),_=>chat(vm)}}
            </main>
            {if matches!(vm.screen.as_str(),"chat"|"dm"){members(vm)}else{().into_any()}}
            {bottom_tabs(vm)}
        </div>
    }.into_any()
}

fn rail(vm: &AppVm, home: bool) -> AnyView {
    view! { <nav class="server-rail" aria-label="Servers">
        <button class=if home{"server-icon selected brand"}else{"server-icon brand"} data-action="screen:home" aria-label="Home" data-tooltip="Home"><img src="public/icons/icon-192.png" alt="" width="48" height="48" /></button>
        <div class="rail-divider"></div>
        {[("The Den","D","den",true),("Side Project","SP","project",false),("Weekend Crew","WC","weekend",false)].into_iter().map(|(name,initials,class,unread)|view!{
            <div class="server-wrap"><button class=format!("server-icon {class} {} {}",if vm.server==name&&!home{"selected"}else{""},if unread{"unread"}else{""}) data-action=format!("server:{name}") aria-label=format!("{initials}: {name}") data-tooltip=name><span aria-hidden="true">{initials}</span></button>{if name=="Side Project" {badge("3","server-count")}else{().into_any()}}</div>
        }).collect_view()}
        <button class="server-icon add-server" data-action="modal:create" aria-label="Add a Server" data-tooltip="Add a Server">{icon("plus")}</button>
    </nav> }.into_any()
}

fn sidebar(vm: &AppVm, home: bool) -> AnyView {
    view! { <aside class="channel-sidebar">
        {if home {view!{<div class="sidebar-header">{button("Find or start a conversation","modal:quick","search-trigger")}</div><nav class="dm-nav"><button class=if vm.screen=="home"{"nav-row selected"}else{"nav-row"} data-action="screen:home">{icon("users")} "Friends"</button><button class="nav-row" data-action="screen:notifications">{icon("bell")} "Notifications" {badge("2","count")}</button></nav><div class="category"><span>"Direct Messages"</span>{icon_button("plus","New Message","modal:new-dm")}</div><nav class="channel-scroll dm-list">{vm.people.iter().enumerate().skip(1).take(4).map(|(i,p)|view!{<button class=if vm.screen=="dm"&&i==1{"dm-row selected"}else{"dm-row"} data-action="screen:dm">{avatar(p,"small")}<span>{p.name.clone()}<small>{p.status.clone()}</small></span>{if i==1{badge("2","count")}else{().into_any()}}</button>}).collect_view()}</nav>}.into_any()}
        else{view!{<div class="sidebar-header"><button class="server-heading" data-action="menu:server">{vm.server.clone()}{icon("chevron-down")}</button>{icon_button("user-plus","Invite People","modal:invite")}</div><div class="channel-scroll"><div class="category"><button data-collapse="text-channels">{icon("chevron-down")} "Text Channels"</button>{icon_button("plus","Create Channel","modal:channel")}</div><nav id="text-channels" aria-label="Text channels">{[("general",0,true),("introductions",0,false),("announcements",3,true),("projects",0,false),("screenshots",0,false)].into_iter().map(|(name,count,unread)|view!{<div class=format!("channel-row {} {}",if vm.channel==name{"selected"}else{""},if unread{"is-unread"}else{""}) data-context="channel"><button class="channel-main" data-action=format!("channel:{name}")>{icon(if name=="announcements"{"lock"}else{"hash"})}<span>{name}</span></button>{if count>0{badge(&count.to_string(),"count")}else{view!{<div class="channel-actions">{icon_button("user-plus","Create Invite","modal:invite")}{icon_button("settings","Edit Channel","modal:channel")}</div>}.into_any()}}</div>}).collect_view()}</nav><div class="category voice-category"><button data-collapse="voice-channels">{icon("chevron-down")} "Voice Channels"</button>{icon_button("plus","Create Channel","modal:channel")}</div><nav id="voice-channels" aria-label="Voice channels"><div class="channel-row"><button class="channel-main" data-action="channel:Lounge">{icon("volume-2")}<span>"Lounge"</span></button></div><div class="voice-users">{[0,1,3].into_iter().map(|i|view!{<button class="voice-user" data-action=format!("profile:{i}")><span class=if i==1{"speaking"}else{""}>{avatar(&vm.people[i],"tiny")}</span><span>{vm.people[i].name.clone()}</span>{if i==1 {badge("LIVE","live")}else{icon("mic-off")}}</button>}).collect_view()}</div><div class="channel-row"><button class="channel-main" data-action="channel:Game Room">{icon("volume-2")}<span>"Game Room"</span></button></div><div class="channel-row muted-channel"><button class="channel-main" data-action="channel:Quiet Room">{icon("lock")}<span>"Quiet Room"</span></button></div></nav></div>}.into_any()}}
        {if vm.connected{view!{<section class="voice-connected"><div class="voice-connection-line"><button class="voice-status" data-action="screen:voice" data-tooltip="Connection: 24 ms">{icon("signal")}<span><strong>"Voice Connected"</strong><small>{format!("Lounge / {}",vm.server)}</small></span></button>{icon_button("phone-off","Disconnect","disconnect")}</div><div class="voice-tiles">{icon_button("video","Turn On Camera","toast:Camera is off in this preview")}{icon_button("monitor","Share Screen","modal:share")}</div></section>}.into_any()}else{().into_any()}}
        <section class="user-panel">{person_button(&vm.people[0],0,"small")}<button class="self-label" data-action="profile:0"><strong>"Alex Chen"</strong><small class="status-copy">"Online"</small><small class="username-copy">"@alex"</small></button><div class=format!("split-control {}",if vm.muted{"is-muted"}else{""})>{icon_button(if vm.muted{"mic-off"}else{"mic"},"Mute","mute")}<button class="split-chevron" aria-label="Input Devices" data-tooltip="Input Devices" data-action="menu:devices">{icon("chevron-down")}</button></div><div class="split-control">{icon_button(if vm.deafened{"headphone-off"}else{"headphones"},"Deafen","deafen")}<button class="split-chevron" aria-label="Output Devices" data-tooltip="Output Devices" data-action="menu:devices">{icon("chevron-down")}</button></div>{icon_button("settings","User Settings","screen:user-settings")}</section>
    </aside> }.into_any()
}

fn header(vm: &AppVm, home: bool) -> AnyView {
    let dm = vm.screen == "dm";
    view! { <header class="channel-header">
        <div class="mobile-menu">{icon_button("menu","Open Channels","panel:left")}</div>
        {if dm{avatar(&vm.people[1],"tiny")}else{icon(if home{"users"}else if vm.screen=="voice"{"volume-2"}else{"hash"})}}
        <h1>{if dm{"Maya".into()}else if vm.screen=="home"{"Friends".into()}else if vm.screen=="notifications"{"Notifications".into()}else if vm.screen=="voice"{"Lounge".into()}else{vm.channel.clone()}}</h1>
        <span class="header-divider"></span><button class="channel-topic" data-action="modal:topic">{if home{""}else if dm{"Working on something"}else{"A place for the everyday stuff."}}</button>
        <div class="header-actions">{icon_button("bell","Notification Settings","modal:notifications")}{icon_button("pin","Pinned Messages","modal:pins")}<span class=if vm.panel=="right"{"active-member"}else{""}>{icon_button("users","Member List","panel:right")}</span><div class="header-search">{search(&format!("Search {}",vm.server),"messages")}</div></div>
    </header> }.into_any()
}

fn chat(vm: &AppVm) -> AnyView {
    let dm = vm.screen == "dm";
    view! { <div class="chat-body">
        <MessageList people=vm.people.clone() messages=vm.messages.clone() dm=dm channel=vm.channel.clone() />
        <div class="composer-wrap"><div class="pending-attachment" hidden><span>{icon("file")} "notes.txt"</span>{icon_button("x","Remove Attachment","remove-attachment")}</div><div class="composer"><div>{icon_button("plus-circle","Add Attachment","menu:upload")}</div><label class="composer-input"><span class="sr-only">"Message"</span><textarea id="message-input" rows="1" placeholder=if dm{"Message @Maya".into()}else{format!("Message #{}",vm.channel)} aria-label=if dm{"Message @Maya".into()}else{format!("Message #{}",vm.channel)}></textarea></label><button class="icon-button gif-button" aria-label="GIF Picker" data-tooltip="GIF Picker" data-action="modal:gif"><span>"GIF"</span></button>{icon_button("smile","Emoji Picker","modal:emoji")}<button class="icon-button send-button" aria-label="Send Message" data-tooltip="Send Message" data-send>{icon("send")}</button></div><div class="typing-indicator"><span class="typing-dots">"•••"</span>{match vm.typing.as_slice(){[]=>String::new(),[name]=>format!("{name} is typing…"),[first,second]=>format!("{first} and {second} are typing…"),_=>"Several people are typing…".into()}}</div></div>
    </div> }.into_any()
}

#[component]
fn MessageList(
    people: Vec<PersonVm>,
    messages: Vec<MessageVm>,
    dm: bool,
    channel: String,
) -> impl IntoView {
    let history_people_data = people.clone();
    let range = RwSignal::new((118usize, 128usize));
    let history_people = if dm { 2 } else { 5 };
    view! { <div class="message-list" id="messages" data-scroll="messages" role="log" aria-label="Messages" tabindex="0" on:scroll=move |ev| {
        let element=event_target::<web_sys::HtmlElement>(&ev);
        let next=history_window(element.scroll_top() as f64,element.client_height() as f64,128);
        if range.get_untracked()!=next{range.set(next);}
    }>
        <div class="virtual-history" aria-label="Earlier messages">
            <div style=move || format!("height:{}px",range.get().0*88) aria-hidden="true"></div>
            {move || {let (start,end)=range.get();(start..end).map(|i|view!{<article class="history-message" data-history-row><div>{person_button(&history_people_data[i%history_people],i%history_people,"message-avatar")}</div><div><div class="message-meta"><button class="author" data-action=format!("profile:{}",i%history_people) style=format!("color:{}",history_people_data[i%history_people].role_color)>{history_people_data[i%history_people].name.clone()}</button><time>"Yesterday at 7:12 PM"</time></div><p>{["I'll be around later tonight.","The new update is ready. Let me know what you think.","Thanks for the games yesterday!", "Saved the notes in #projects."][i%4]}</p></div></article>}).collect_view()}}
            <div style=move || format!("height:{}px",(128-range.get().1)*88) aria-hidden="true"></div>
        </div>
        <div class="conversation-intro"><span class="intro-icon">{if dm{avatar(&people[1],"large")}else{icon("hash")}}</span><h2>{if dm{"Maya".into()}else{format!("Welcome to #{}!",channel)}}</h2><p>{if dm{"This is the beginning of your direct messages with Maya.".into()}else{format!("This is the beginning of #{}.",channel)}}</p></div>
        <div class="date-separator"><span>"November 12, 2026"</span></div>
        {messages.iter().map(|m|message(m,&people)).collect_view()}
        <div class="message-end"></div>
    </div><button class="jump-present" data-jump>"Jump to present" {icon("arrow-down")}</button> }
}

fn message(m: &MessageVm, people: &[PersonVm]) -> AnyView {
    let p = &people[m.author];
    view! { <article class=format!("message {} {}",if m.continuation{"continuation"}else{""},if m.mention{"mentioned"}else{""}) data-message=m.id.to_string() data-context="message">
        {if m.id==6 {view!{<div class="new-divider"><span>"NEW"</span></div>}.into_any()}else{().into_any()}}
        <div class="avatar-gutter">{if m.continuation{view!{<time class="short-time">"4:18 PM"</time>}.into_any()}else{person_button(p,m.author,"message-avatar")}}</div>
        <div class="message-content">
            {if m.reply{view!{<button class="reply-preview" data-action="toast:Reply highlighted">{icon("reply")}{avatar(&people[1],"mini")}<strong>"Maya"</strong><span>"Also, finally cleaned up my desk…"</span></button>}.into_any()}else{().into_any()}}
            {if m.continuation{().into_any()}else{view!{<div class="message-meta"><button class="author" data-action=format!("profile:{}",m.author) style=format!("color:{}",p.role_color)>{p.name.clone()}</button><time>{m.time.clone()}</time></div>}.into_any()}}
            <div class="markdown" inner_html=markdown(&m.body)></div>{if m.edited{view!{<span class="edited">"(edited)"</span>}.into_any()}else{().into_any()}}
            {match &m.attachment {
                AttachmentVm::Image {src,srcset,alt,width,height,name}=>view!{<button class="attachment-image" data-action="modal:image" aria-label=format!("Open {name}")><img src=src.clone() srcset=srcset.clone() loading="lazy" width=*width height=*height style=format!("aspect-ratio:{width}/{height}") alt=alt.clone() /></button>}.into_any(),
                AttachmentVm::File {name,size}=>view!{<div class="file-card">{icon("file")}<span><strong>{name.clone()}</strong><small>{size.clone()}</small></span>{icon_button("download",&format!("Download {name}"),"toast:File download unavailable")}</div>}.into_any(),
                AttachmentVm::None=>().into_any()
            }}
            <div class="reactions">{m.reactions.iter().map(|(emoji,count,mine)|view!{<button class=if *mine{"reaction mine"}else{"reaction"} data-action=format!("react:{},{}",m.id,emoji) aria-label=format!("React with {}, {} reactions",if *emoji=="1f525"{"fire"}else{"thumbs up"},count) aria-pressed=mine.to_string()><img src=format!("public/emoji/{emoji}.png") alt=if *emoji=="1f525"{"fire"}else{"thumbs up"} width="16" height="16" />{count.to_string()}</button>}).collect_view()}</div>
        </div><div class="message-actions">{icon_button("smile","Add Reaction",&format!("message-emoji:{}",m.id))}{icon_button("reply","Reply","toast:Reply selected")}{if m.author==0{icon_button("pencil","Edit Message","modal:edit-message")}else{().into_any()}}{icon_button("more-horizontal","More Message Actions","menu:message")}</div>
    </article> }.into_any()
}

fn members(vm: &AppVm) -> AnyView {
    if vm.screen == "dm" {
        return view!{<aside class=format!("member-list dm-details {}",if vm.panel=="right"{"is-open"}else{""}) aria-label="Maya's profile"><div class="mobile-member-header"><h2>"Profile"</h2>{icon_button("x","Close Profile","close")}</div><div class="dm-details-content">{avatar(&vm.people[1],"large")}<h2>"Maya"</h2><p>"@maya · she / her"</p><div class="section-divider"></div><h3>"About Me"</h3><p>"Working on something"</p><h3>"Member Since"</h3><p>"Apr 14, 2025"</p><div class="section-divider"></div><button data-action="profile:1">"2 Mutual Servers" {icon("chevron-right")}</button><button data-action="profile:1">"4 Mutual Friends" {icon("chevron-right")}</button>{button("View Profile","profile:1","secondary")}</div></aside>}.into_any();
    }

    view! { <aside class=format!("member-list {}",if vm.panel=="right"{"is-open"}else{""}) aria-label="Members"><div class="mobile-member-header"><h2>"Members"</h2>{icon_button("x","Close Members","close")}</div>{[("Owner — 1",vec![0]),("Moderators — 2",vec![1,2]),("Online — 2",vec![3,4]),("Offline — 3",vec![5,6,7])].into_iter().map(|(heading,people)|view!{<section class="member-group"><h2>{heading}</h2>{people.into_iter().map(|i|{let p=&vm.people[i];view!{<button class=format!("member-row {}",if p.presence=="offline"{"offline-member"}else{""}) data-action=format!("profile:{i}") data-context="user">{avatar(p,"small")}<span><strong style=format!("color:{}",p.role_color)>{p.name.clone()}{if p.owner{icon("crown")}else{().into_any()}}</strong><small>{p.status.clone()}</small></span></button>}}).collect_view()}</section>}).collect_view()}</aside> }.into_any()
}

fn bottom_tabs(vm: &AppVm) -> AnyView {
    view! { <nav class="bottom-tabs" aria-label="Main navigation">{[("home","Home","home","message-circle"),("notifications","Notifications","notifications","bell"),("user-settings","You","you","user-round")].into_iter().map(|(screen,label,class,glyph)|view!{<button class=format!("{class} {}",if vm.screen==screen{"active"}else{""}) data-action=format!("screen:{screen}")>{icon(glyph)}<span>{label}</span>{if screen=="notifications"{view!{<span class="tab-dot"></span>}.into_any()}else{().into_any()}}</button>}).collect_view()}</nav> }.into_any()
}
