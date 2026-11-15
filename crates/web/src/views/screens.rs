use super::*;

pub fn auth(vm: &AppVm) -> AnyView {
    let register = vm.screen == "register";
    view! { <main class="auth-page"><form class="auth-card" data-auth>
        <div class="auth-brand"><img src="public/icons/icon-192.png" alt="" width="40" height="40" /><strong>"StickTalk"</strong></div>
        <h1>{if register{"Create an account"}else{"Log in"}}</h1>
        {input("Server address","sticktalk.ts.net","text","The address shared by your server owner.")}
        {input("Username",if register{""}else{"alex"},"text","")}
        {if register{input("Display name","","text","How your name appears to friends.")}else{().into_any()}}
        {input("Password","","password","")}
        <p class="field-error" id="auth-error" role="alert" hidden>"Enter your username and password."</p>
        <button type="submit" class="button primary">{if register{"Create Account"}else{"Log In"}}</button>
        <div class="or-divider"><span>"or"</span></div>
        <button type="button" class="button google-button" data-action="toast:Google sign-in is not connected yet"><img src="public/google-g.svg" width="20" height="20" alt=""/>"Continue with Google"</button>
        <p class="auth-switch">{if register{"Already have an account? "}else{"Need an account? "}}<button type="button" class="link-button" data-action=if register{"screen:login"}else{"screen:register"}>{if register{"Log in"}else{"Register"}}</button></p>
    </form></main> }.into_any()
}

pub fn home(vm: &AppVm) -> AnyView {
    let people: Vec<usize> = match vm.tab.as_str() {
        "All" => (1..8).collect(),
        "Pending" => vec![6],
        "Blocked" => vec![],
        _ => vec![1, 2, 3, 4],
    };
    view! {<div class="friends-page"><div class="friends-toolbar">{tabs(&["Online","All","Pending","Blocked"],&vm.tab,"tab")}{button("Add Friend","modal:add-friend","primary")}</div><div class="friends-content">{search("Search friends","friends")}<h2 class="small-heading">{format!("{} — {}",vm.tab,people.len())}</h2>{if people.is_empty(){empty("users","No blocked users","Add Friend","modal:add-friend")}else{view!{<div id="friends">{people.into_iter().map(|i|{let p=&vm.people[i];view!{<div class="friend-row" data-filter-row data-context="user"><button class="friend-info" data-action=format!("profile:{i}")>{avatar(p,"message-avatar")}<span><strong>{p.name.clone()}</strong><small>{if vm.tab=="Pending"{"Incoming Friend Request".to_owned()}else{p.presence.clone()}}</small></span></button><div class="friend-actions">{icon_button(if vm.tab=="Pending"{"check"}else{"message-circle"},if vm.tab=="Pending"{"Accept Friend Request"}else{"Message"},if vm.tab=="Pending"{"toast:Friend request accepted locally"}else{"screen:dm"})}{icon_button("more-horizontal","More Friend Actions","menu:user")}</div></div>}}).collect_view()}</div>}.into_any()}}</div><aside class="friends-aside"><h2>"Active Now"</h2><div class="active-now"><div class="active-now-heading">{avatar(&vm.people[1],"small")}<span><strong>"Maya"</strong><small>"In a voice channel"</small></span></div><button class="active-call" data-action="screen:voice">{icon("volume-2")}<span><strong>"Lounge"</strong><small>"The Den · 3 people"</small></span>{icon("chevron-right")}</button></div></aside></div>}.into_any()
}

pub fn notifications(vm: &AppVm) -> AnyView {
    view!{<div class="notifications-page"><h2>"Notifications"</h2>{tabs(&["Mentions","Unread"],"Mentions","tab")}<div class="notification-card">{avatar(&vm.people[3],"message-avatar")}<div><strong>"Sam Rivera"</strong><small>"The Den / #general · Today at 4:20 PM"</small><p><span class="mention">"@Alex Chen"</span> " I put the notes here for later."</p>{button("Jump to Message","screen:chat","secondary")}</div></div><div class="notification-card">{avatar(&vm.people[1],"message-avatar")}<div><strong>"Maya"</strong><small>"Direct Message · Today at 3:48 PM"</small><p>"Are you joining us tonight?"</p>{button("Message","screen:dm","secondary")}</div></div></div>}.into_any()
}

pub fn settings(vm: &AppVm) -> AnyView {
    let server = vm.screen != "user-settings";
    let title = if vm.screen == "roles" {
        "Roles"
    } else if vm.screen == "role-editor" {
        "Edit Role"
    } else {
        &vm.setting
    };
    view!{<div class="settings-shell"><aside class="settings-nav"><div class="settings-nav-inner">{if server{view!{<h2 class="settings-server-name">{vm.server.clone()}</h2>}.into_any()}else{view!{<button class="settings-profile" data-action="setting:Profiles">{avatar(&vm.people[0],"message-avatar")}<span><strong>"Alex Chen"</strong><small>"Edit Profiles" {icon("pencil")}</small></span></button>}.into_any()}}{search("Search settings","settings-items")}<nav id="settings-items">{if server{server_nav(vm)}else{user_nav(vm)}}<div class="nav-divider"></div><button class="settings-nav-row danger-text" data-action=if server{"modal:delete-server"}else{"modal:logout"}>{icon(if server{"trash-2"}else{"log-out"})}{if server{"Delete Server"}else{"Log Out"}}</button><small class="version">"StickTalk 0.1.0"</small><div class="about-credit">"Inter & JetBrains Mono · OFL"<br/>"Lucide · ISC · Twemoji · CC-BY 4.0"</div></nav></div></aside><main class="settings-main"><header class="settings-topbar"><div class="settings-nav-toggle">{icon_button("menu","Settings Navigation","toggle-settings-nav")}</div><h1>{title.to_owned()}</h1>{icon_button("x","Close Settings","screen:chat")}</header><div class=format!("settings-content {}",if vm.screen=="server-settings"&&vm.setting=="Server Profile"{"wide-content"}else{""})>{match vm.screen.as_str(){"roles"=>roles(vm),"role-editor"=>role_editor(vm),_=>if server{server_content(vm)}else{user_content(vm)}}}</div>{if vm.dirty{view!{<div class="unsaved-bar" role="status"><strong>"Careful — you have unsaved changes!"</strong><div>{button("Reset","reset","link-button")}{button("Save Changes","save","primary")}</div></div>}.into_any()}else{().into_any()}}</main></div>}.into_any()
}

fn nav_row(label: &str, glyph: &str, selected: bool, action: &str) -> AnyView {
    view!{<button class=if selected{"settings-nav-row selected"}else{"settings-nav-row"} data-action=action.to_owned() data-filter-row>{icon(glyph)}{label.to_owned()}</button>}.into_any()
}
fn user_nav(vm: &AppVm) -> AnyView {
    view!{<h2 class="nav-group-title">"User Settings"</h2>{nav_row("My Account","user-round",matches!(vm.setting.as_str(),"Account Info"|"Password & Security"|"Sign-in Methods"|"Account Status"),"setting:Account Info")}<div class="settings-subnav">{["Account Info","Password & Security","Sign-in Methods","Account Status"].into_iter().map(|s|view!{<button class=if vm.setting==s{"selected"}else{""} data-action=format!("setting:{s}")>{s}</button>}).collect_view()}</div>{[("Profiles","pencil"),("Privacy & Safety","shield"),("Devices","smartphone"),("Notifications","bell")].into_iter().map(|(s,i)|nav_row(s,i,vm.setting==s,&format!("setting:{s}"))).collect_view()}<h2 class="nav-group-title">"App Settings"</h2>{[("Voice & Video","mic"),("Appearance","palette"),("Accessibility","eye"),("Keybinds","key-round"),("Advanced","code")].into_iter().map(|(s,i)|nav_row(s,i,vm.setting==s,&format!("setting:{s}"))).collect_view()}}.into_any()
}
fn server_nav(vm: &AppVm) -> AnyView {
    view!{{[("Server Profile","user-round"),("Engagement","message-circle")].into_iter().map(|(s,i)|nav_row(s,i,vm.setting==s&&vm.screen=="server-settings",&format!("setting:{s}"))).collect_view()}<h2 class="nav-group-title">"Expression"</h2>{nav_row("Emoji","smile",vm.setting=="Emoji","setting:Emoji")}<h2 class="nav-group-title">"People"</h2>{nav_row("Members","users",vm.setting=="Members","setting:Members")}{nav_row("Roles","shield",vm.screen=="roles"||vm.screen=="role-editor","screen:roles")}{[("Invites","link"),("Access","lock")].into_iter().map(|(s,i)|nav_row(s,i,vm.setting==s,&format!("setting:{s}"))).collect_view()}<h2 class="nav-group-title">"Moderation"</h2>{nav_row("Bans","ban",vm.setting=="Bans","setting:Bans")}}.into_any()
}

fn field_row(label: &str, value: &str, reveal: bool) -> AnyView {
    view!{<div class="account-field"><strong>{label.to_owned()}</strong><span>{value.to_owned()}{if reveal{view!{<button class="link-button" data-reveal>"Reveal"</button>}.into_any()}else{().into_any()}}</span>{button("Edit","modal:edit-field","secondary")}</div>}.into_any()
}
fn user_content(vm: &AppVm) -> AnyView {
    match vm.setting.as_str(){
        "Account Info"=>view!{<section><div class="account-card"><div class="account-banner"></div><div class="account-card-body">{avatar(&vm.people[0],"large")}<div><h2>"Alex Chen"</h2><p>"@alex"</p></div>{button("Edit Profile","setting:Profiles","secondary")}</div></div><h2>"Account Info"</h2>{field_row("Display Name","Alex Chen",false)}{field_row("Username","alex",false)}{field_row("Email","********@gmail.com",true)}<div class="section-divider"></div><h2>"Password & Security"</h2><p class="muted-copy">"Keep your account secure with a strong password."</p>{button("Change Password","setting:Password & Security","primary")}<div class="section-divider"></div><h2>"Sign-in Methods"</h2><div class="method-row"><img src="public/google-g.svg" alt="" width="24" height="24"/><span><strong>"Google"</strong><small>"Connected"</small></span>{button("Manage","setting:Sign-in Methods","secondary")}</div></section>}.into_any(),
        "Profiles"=>view!{<section><h2>"Profiles"</h2><div class="profile-form">{avatar(&vm.people[0],"large")}{button("Change Avatar","toast:Avatar upload preview","primary")}</div>{input("Display name","Alex Chen","text","")}{input("Pronouns","he / him","text","")}<div class="field"><label for="about-me">"About me"</label><textarea id="about-me" rows="4" data-dirty>"Usually up for a game. Ask me about my current project."</textarea><small>"190 characters remaining"</small></div>{button("Save Changes","save","primary")}</section>}.into_any(),
        "Password & Security"=>view!{<section><h2>"Password & Security"</h2>{input("Current password","","password","")}{input("New password","","password","")}{button("Update Password","toast:Password changes require a connected server","primary")}<div class="section-divider"></div><h2>"Two-factor Authentication"</h2><p class="muted-copy">"Protect your account with an authenticator app."</p><button class="button secondary" disabled>"Enable Authenticator"</button></section>}.into_any(),
        "Sign-in Methods"=>view!{<section><h2>"Sign-in Methods"</h2><div class="method-row"><img src="public/google-g.svg" width="24" height="24" alt=""/><span><strong>"Google"</strong><small>"alex.chen@gmail.com"</small></span>{button("Disconnect","modal:unlink-google","secondary")}</div><div class="method-row">{icon("key-round")}<span><strong>"Password"</strong><small>"Last changed November 1, 2026"</small></span>{button("Change","setting:Password & Security","secondary")}</div></section>}.into_any(),
        "Devices"=>view!{<section><h2>"Devices"</h2><p class="muted-copy">"Manage where you're logged in."</p>{[("Chrome on Windows","Current device · Active now"),("Safari on iPhone","Last active today at 9:14 AM")].into_iter().map(|(name,info)|view!{<div class="device-row">{icon("smartphone")}<span><strong>{name}</strong><small>{info}</small></span>{button("Log Out","modal:logout","secondary")}</div>}).collect_view()}</section>}.into_any(),
        "Appearance"=>view!{<section><h2>"Appearance"</h2><h3>"Theme"</h3>{tabs(&["Dark","Light","System"],"Dark","tab")}<p class="muted-copy">"Dark theme is currently available."</p><div class="section-divider"></div><h3>"Message Density"</h3>{tabs(&["Cozy","Compact"],&vm.tab,"tab")}<div class="section-divider"></div><label class="range-field">"Chat Font Scale"<input type="range" min="80" max="150" value="100"/><small>"100%"</small></label></section>}.into_any(),
        "Voice & Video"=>view!{<section><h2>"Voice & Video"</h2>{input("Input device","Default microphone","text","")}{input("Output device","Default speakers","text","")}<label class="range-field">"Input Volume"<input type="range" value="80"/></label><div class="section-divider"></div>{toggle("Noise suppression","Reduce steady background noise.",true,"toast:Noise suppression preference changed locally")}{toggle("Echo cancellation","Prevent audio feedback.",true,"toast:Echo cancellation preference changed locally")}{button("Test Microphone","toast:Microphone access is disabled in this preview","secondary")}</section>}.into_any(),
        "Keybinds"=>view!{<section><h2>"Keybinds"</h2>{[("Quick Switcher","Ctrl + K"),("Toggle Mute","Ctrl + Shift + M"),("Toggle Deafen","Ctrl + Shift + D"),("Next Channel","Alt + Down"),("Previous Channel","Alt + Up"),("Close Overlay","Esc")].into_iter().map(|(label,key)|view!{<div class="keybind-row"><strong>{label}</strong><kbd>{key}</kbd></div>}).collect_view()}</section>}.into_any(),
        "Account Status"=>view!{<section><h2>"Account Status"</h2><div class="status-card">{icon("shield-check")}<div><strong>"Your account is in good standing"</strong><p>"There are no restrictions on your account."</p></div></div></section>}.into_any(),
        _=>view!{<section><h2>{vm.setting.clone()}</h2>{toggle(if vm.setting=="Advanced"{"Developer mode"}else if vm.setting=="Accessibility"{"Always underline links"}else{"Direct messages from server members"},if vm.setting=="Advanced"{"Show Copy ID in context menus."}else if vm.setting=="Accessibility"{"Make links easier to identify."}else{"Allow people in your servers to message you."},true,"toggle-local")}{toggle(if vm.setting=="Notifications"{"Mention notifications"}else{"Show online status"},"You can change this at any time.",false,"toggle-local")}</section>}.into_any()
    }
}

fn server_content(vm: &AppVm) -> AnyView {
    match vm.setting.as_str(){
        "Server Profile"=>view!{<div class="server-profile-layout"><section class="server-profile-form"><h2>"Server Profile"</h2><div class="field"><label for="server-name">"Name"</label><input id="server-name" value=vm.server.clone() data-change-action="server-name" maxlength="100"/></div><div class="field"><label>"Icon"</label><p class="muted-copy">"We recommend an image of at least 512x512"</p><div class="inline-actions">{button("Change Server Icon","toast:Server icon upload preview","primary")}{button("Remove Icon","dirty","secondary danger-text")}</div></div><div class="field"><label>"Banner Color"</label><div class="banner-swatches">{(0..10).map(|i|view!{<button class=format!("banner-swatch swatch-{i} {}",if i==vm.banner{"selected"}else{""}) aria-label=format!("Banner color {}",i+1) data-action=format!("banner:{i}")></button>}).collect_view()}</div></div><div class="field"><label for="server-description">"Description"</label><textarea id="server-description" rows="4" maxlength="120" data-change-action="server-description">{vm.server_description.clone()}</textarea><small class="character-count">{format!("{}/120",vm.server_description.chars().count())}</small></div></section><aside class="server-preview"><div class=format!("server-preview-banner swatch-{}",vm.banner)></div><div class="server-preview-icon">"D"</div><div class="server-preview-body"><h2>{vm.server.clone()}</h2><p><span class="online-dot"></span>" 5 Online "<span class="offline-dot"></span>" 21 Members"</p><small>"Est. Apr 2025"</small><p>{vm.server_description.clone()}</p></div></aside></div>}.into_any(),
        "Members"=>view!{<section><h2>"Members — 21"</h2>{search("Search Members","server-members")}<div class="role-filter">{button("All Roles","menu:role-filter","secondary")}</div><div id="server-members">{vm.people.iter().enumerate().map(|(i,p)|view!{<div class="server-member-row" data-filter-row>{avatar(p,"small")}<strong>{p.name.clone()}</strong>{role_chip(&vm.roles[if i==0{0}else if i<3{1}else{2}])}{icon_button("more-horizontal","Member Actions","menu:user")}</div>}).collect_view()}</div></section>}.into_any(),
        "Invites"=>view!{<section><h2>"Invites"</h2><p class="muted-copy">"Invite people to The Den."</p>{button("Create Invite","modal:invite","primary")}<div class="invite-row"><strong>"den-friends"</strong><span>"3 uses · Expires in 7 days"</span>{icon_button("copy","Copy Invite","copy")}{icon_button("x","Revoke Invite","toast:Invite revoked locally")}</div></section>}.into_any(),
        "Bans"=>empty("shield-check","No banned members","View Members","setting:Members"),
        "Emoji"=>view!{<section><h2>"Server Emoji"</h2><p class="muted-copy">"Upload an image to use as a custom emoji."</p>{button("Upload Emoji","toast:Emoji upload preview","primary")}<div class="emoji-grid">{["1f331","1f525","1f680","1f44d"].into_iter().map(|e|view!{<img src=format!("public/emoji/{e}.png") alt="Server emoji" width="32" height="32"/>}).collect_view()}</div></section>}.into_any(),
        _=>view!{<section><h2>{vm.setting.clone()}</h2>{toggle("Allow members to create invites","Members can invite friends to this server.",true,"toggle-local")}{toggle("Notify everyone about new messages","Members can choose their own notification settings.",false,"toggle-local")}</section>}.into_any()
    }
}

fn roles(vm: &AppVm) -> AnyView {
    view!{<section><h2>"Roles"</h2><p class="muted-copy">"Use roles to group your server members and assign permissions."</p><button class="default-permissions" data-action="role:4">{icon("users")}<span><strong>"Default Permissions"</strong><small>"@everyone · applies to all server members"</small></span>{icon("chevron-right")}</button><div class="roles-search">{search("Search Roles","roles-table")}{button("Create Role","create-role","primary")}</div><p class="role-help">"Members use the color of the highest role they have on this list. Drag roles to reorder them."</p><div class="role-table-heading"><span>{format!("Roles — {}",vm.roles.len())}</span><span>"Members"</span></div><div id="roles-table">{vm.roles.iter().enumerate().map(|(i,r)|view!{<div class="role-row" draggable="true" data-role-index=i.to_string() data-filter-row><span class="role-shield" style=format!("color:{}",r.color)>{icon("shield")}</span><button class="role-name" data-action=format!("role:{i}")>{r.name.clone()}</button><span class="role-member-count">{r.members}{icon("user-round")}</span><div class="row-actions">{icon_button("pencil",&format!("Edit {}",r.name),&format!("role:{i}"))}{icon_button("more-horizontal",&format!("More {} Actions",r.name),"menu:role")}</div></div>}).collect_view()}</div></section>}.into_any()
}

fn role_editor(vm: &AppVm) -> AnyView {
    let role = &vm.roles[vm.selected_role];
    view!{<div class="role-editor-layout"><aside class="role-editor-nav"><div class="role-back">{button("Back","screen:roles","link-button")}{icon_button("plus","Create Role","create-role")}</div>{vm.roles.iter().enumerate().map(|(i,r)|view!{<button class=if i==vm.selected_role{"role-nav-row selected"}else{"role-nav-row"} data-action=format!("role:{i}")><span class="role-dot" style=format!("background:{}",r.color)></span><span>{r.name.clone()}</span></button>}).collect_view()}</aside><section class="role-editor-main"><div class="role-editor-heading"><h2>{format!("Edit Role — {}",role.name)}</h2>{icon_button("more-horizontal","Role Actions","menu:role")}</div>{tabs(&["Display","Permissions","Manage Members"],&vm.role_tab,"role-tab")}{match vm.role_tab.as_str(){"Permissions"=>permissions(vm),"Manage Members"=>view!{<div class="manage-members">{search("Search Members","role-members")}{button("Add Members","modal:add-members","primary")}<div id="role-members">{vm.people.iter().enumerate().take(role.members.min(8)).map(|(i,p)|view!{<div class="server-member-row" data-filter-row>{avatar(p,"small")}<strong>{p.name.clone()}</strong>{icon_button("x","Remove Member",&format!("toast:{} removed locally",vm.people[i].name))}</div>}).collect_view()}</div></div>}.into_any(),_=>view!{<div class="role-display"><div class="field"><label for="role-name">"Role Name"</label><input id="role-name" value=role.name.clone() data-change-action="role-name" maxlength="100"/></div><div class="field"><label>"Role Color"</label><p class="muted-copy">"Members use the color of their highest role."</p><div class="role-colors">{["#3DDC97","#C8A2E0","#6CB5ED","#F2B84B","#F25F5C","#E8EAED","#7BAE7F","#E29CC0","#A8A8B0","#DA9B6A"].into_iter().map(|c|view!{<button class=if role.color==c{"color-swatch selected"}else{"color-swatch"} style=format!("background:{c}") aria-label=format!("Role color {c}") data-action=format!("role-color:{c}")>{if role.color==c{icon("check")}else{().into_any()}}</button>}).collect_view()}</div><label class="hex-input">"Custom color"<input value=role.color.clone() data-change-action="role-color" pattern="#[0-9A-Fa-f]{6}" maxlength="7"/></label></div><div class="section-divider"></div>{toggle("Display role members separately","Show members with this role in their own group in the member list.",role.hoisted,"hoist")}{toggle("Allow anyone to @mention this role","Members can notify everyone assigned to this role.",role.mentionable,"mentionable")}</div>}.into_any()}}</section></div>}.into_any()
}

fn permissions(vm: &AppVm) -> AnyView {
    let sections = [
        (
            "General Server Permissions",
            vec![
                (
                    "View Channels",
                    "Allows members to view channels by default.",
                ),
                (
                    "Manage Channels",
                    "Allows members to create, edit, and delete channels.",
                ),
                (
                    "Manage Roles",
                    "Allows members to create and modify lower roles.",
                ),
            ],
        ),
        (
            "Membership",
            vec![
                ("Kick Members", "Remove members from this server."),
                ("Create Invite", "Invite new people to this server."),
            ],
        ),
        (
            "Text Channel Permissions",
            vec![
                ("Send Messages", "Send messages in text channels."),
                ("Manage Messages", "Delete messages from other members."),
                ("Add Reactions", "Add new reactions to messages."),
            ],
        ),
        (
            "Voice Channel Permissions",
            vec![
                ("Connect", "Join voice channels."),
                ("Speak", "Speak in voice channels."),
                ("Share Screen", "Stream to other members."),
            ],
        ),
        (
            "Advanced",
            vec![("Administrator", "Grants all permissions. Use with caution.")],
        ),
    ];
    let mut offset = 0;
    view!{<div class="permission-editor">{search("Search permissions","permission-list")}<div id="permission-list">{sections.into_iter().map(|(name,rows)|{let base=offset;offset+=rows.len();view!{<section class="permission-section"><div class="permission-section-heading"><h3>{name}</h3><button class="link-button" data-clear-permissions>"Clear permissions"</button></div>{rows.into_iter().enumerate().map(|(i,(label,desc))|view!{<div data-filter-row>{toggle(label,desc,vm.permissions[base+i],&format!("permission:{}",base+i))}</div>}).collect_view()}</section>}}).collect_view()}</div></div>}.into_any()
}

pub fn voice(vm: &AppVm) -> AnyView {
    view!{<div class="voice-view"><div class="call-heading"><div><h2>"Lounge"</h2><p>"The Den · 3 people connected"</p></div>{button("Open Chat","screen:chat","secondary")}</div><div class="participant-grid">{[0,1,3].into_iter().map(|i|view!{<div class=format!("participant-tile {}",if i==1{"is-speaking"}else{""})>{avatar(&vm.people[i],"large")}<div class="participant-label"><span>{vm.people[i].name.clone()}</span>{icon(if i==3{"mic-off"}else{"mic"})}</div>{if i==1{badge("Speaking","speaking-badge")}else{().into_any()}}</div>}).collect_view()}<button class="stream-tile" data-action="screen:stream"><img src="public/mock-workspace.jpg" alt="Maya's shared screen" width="400" height="225"/><span>{badge("LIVE","live")}<strong>"Maya's screen"</strong><small>"Watch Stream"</small></span></button></div><div class="call-controls">{icon_button(if vm.muted{"mic-off"}else{"mic"},"Mute","mute")}{icon_button(if vm.deafened{"headphone-off"}else{"headphones"},"Deafen","deafen")}{icon_button("video","Turn On Camera","toast:Camera access is disabled in this preview")}{icon_button("monitor","Share Screen","modal:share")}<span class="disconnect-control">{icon_button("phone-off","Leave Call","disconnect")}</span></div><p class="call-connection">{icon("signal")} "Voice Connected · 24 ms"</p></div>}.into_any()
}

pub fn stream(vm: &AppVm) -> AnyView {
    view!{<main class="stream-view"><header><div>{badge("LIVE","live")}<h1>"Maya's screen"</h1><span>"Lounge / The Den"</span></div>{icon_button("x","Stop Watching","screen:voice")}</header><div class="stream-stage"><img src="public/mock-workspace.jpg" alt="Maya's shared workspace screen"/><div class="stream-quality">"1080p · 60 FPS"</div></div><footer><div>{avatar(&vm.people[1],"small")}<strong>"Maya"</strong></div><div>{icon_button("volume-2","Stream Volume","menu:volume")}{icon_button("settings","Stream Quality","menu:quality")}{icon_button("maximize","Full Screen","fullscreen")}{button("Stop Watching","screen:voice","secondary")}</div></footer></main>}.into_any()
}
