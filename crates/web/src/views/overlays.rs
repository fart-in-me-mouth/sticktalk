use super::*;

pub fn overlays(vm: &AppVm) -> AnyView {
    view! {
        {if let Some(i)=vm.profile {profile(vm,i.min(7))}else{().into_any()}}
        {if vm.menu.is_empty(){().into_any()}else{menu(vm)}}
        {if vm.modal.is_empty(){().into_any()}else{modal(vm)}}
    }
    .into_any()
}

fn profile(vm: &AppVm, i: usize) -> AnyView {
    let p = &vm.people[i];
    view!{<div class="popover-dismiss" data-action="close"></div><section class="profile-popout" role="dialog" aria-modal="true" aria-label=format!("{}'s profile",p.name) data-floating><div class="profile-banner" style=format!("background:{}",p.color)>{icon_button("more-horizontal","Profile Actions","menu:user")}{icon_button("x","Close Profile","close")}</div><div class="profile-avatar">{avatar(p,"large")}</div><div class="profile-popout-content"><h2 style=format!("color:{}",p.role_color)>{p.name.clone()}</h2><p class="profile-username">{format!("@{} · {}",p.username,if i==1{"she / her"}else{"he / him"})}</p><div class="mutual-links"><button data-action="toast:2 mutual servers">"2 Mutual Servers"{icon("chevron-right")}</button><button data-action="toast:4 mutual friends">"4 Mutual Friends"{icon("chevron-right")}</button></div><p class="custom-status">{p.status.clone()}</p><h3>"Member Since"</h3><p class="member-since">"Apr 14, 2025"</p><h3>"Roles"</h3><div class="role-chips">{role_chip(&vm.roles[if i==0{0}else if i<3{1}else{2}])}{role_chip(&vm.roles[3])}{icon_button("plus","Add Role","menu:role-filter")}</div>{button(if i==0{"Edit Profile"}else{"Message"},if i==0{"screen:user-settings"}else{"screen:dm"},"primary profile-action")}<label class="profile-message"><span class="sr-only">{format!("Message @{}",p.username)}</span><input placeholder=format!("Message @{}",p.username) data-profile-message /></label></div></section>}.into_any()
}

fn menu(vm: &AppVm) -> AnyView {
    view!{<div class="popover-dismiss" data-action="close"></div><div class=format!("dropdown-menu menu-{}",vm.menu) role="menu" aria-label="Actions" data-floating>{match vm.menu.as_str(){
        "server"=>view!{
            {menu_item("Invite to Server","user-plus","modal:invite",false)}{menu_item("Server Settings","settings","screen:server-settings",false)}{menu_item("Create Channel","plus-circle","modal:channel",false)}{menu_item("Create Category","folder","modal:category",false)}<div class="menu-separator"></div>{menu_item("Notification Settings","bell","modal:notifications",false)}{menu_item("Privacy Settings","shield","modal:privacy",false)}<div class="menu-separator"></div>{menu_item("Edit Per-server Profile","pencil","screen:user-settings",false)}<button class="menu-item" role="menuitemcheckbox" aria-checked="false" data-check-menu><span>"Hide Muted Channels"</span><span class="menu-checkbox"></span></button>
        }.into_any(),
        "upload"=>menu_item("Upload a File","file","pick-file",false),
        "devices"=>view!{<h3>"Input / Output Devices"</h3>{menu_item("System Default","check","toast:Using system default device",false)}{menu_item("Voice & Video Settings","settings","screen:user-settings",false)}}.into_any(),
        "quality"=>view!{<h3>"Stream Quality"</h3>{menu_item("1080p · 60 FPS","check","toast:1080p selected",false)}{menu_item("720p · 30 FPS","monitor","toast:720p selected",false)}}.into_any(),
        "volume"=>view!{<label class="range-field">"Stream Volume"<input type="range" min="0" max="100" value="80"/></label>}.into_any(),
        "role-filter"=>view!{{vm.roles.iter().map(|r|menu_item(&r.name,"shield",&format!("toast:{} selected",r.name),false)).collect_view()}}.into_any(),
        "role"=>view!{{menu_item("Edit Role","pencil",&format!("role:{}",vm.selected_role),false)}{menu_item("Copy ID","copy","copy",false)}<div class="menu-separator"></div>{menu_item("Delete Role","trash-2","modal:delete-role",true)}}.into_any(),
        "channel"=>view!{{menu_item("Mark as Read","check-check","toast:Channel marked as read",false)}{menu_item("Notification Settings","bell","modal:notifications",false)}{menu_item("Edit Channel","settings","modal:channel",false)}{menu_item("Copy Link","link","copy",false)}}.into_any(),
        "user"=>view!{{menu_item("Profile","user-round","profile:1",false)}{menu_item("Message","message-circle","screen:dm",false)}{menu_item("Add Friend","user-plus","toast:Friend request sent locally",false)}{menu_item("Copy ID","copy","copy",false)}<div class="menu-separator"></div>{menu_item("Block","ban","modal:block",true)}}.into_any(),
        _=>view!{{menu_item("Add Reaction","smile","modal:emoji",false)}{menu_item("Reply","reply","toast:Reply selected",false)}{menu_item("Pin Message","pin","toast:Message pinned locally",false)}{menu_item("Copy Text","copy","copy",false)}{menu_item("Copy Message Link","link","copy",false)}<div class="menu-separator"></div>{menu_item("Delete Message","trash-2","modal:delete-message",true)}}.into_any()
    }}</div>}.into_any()
}

fn modal(vm: &AppVm) -> AnyView {
    let selected = vm
        .messages
        .iter()
        .find(|m| m.id == vm.selected_message)
        .unwrap_or(&vm.messages[0]);
    let selected_author = &vm.people[selected.author];
    let title = match vm.modal.as_str() {
        "create" => "Add a Server",
        "create-server" => "Create a Server",
        "join-server" => "Join a Server",
        "invite" => "Invite people to The Den",
        "sheet" => "Message Actions",
        "quick" => "Quick Switcher",
        "emoji" => "Add a Reaction",
        "gif" => "GIFs",
        "pins" => "Pinned Messages",
        "channel" => "Edit Channel",
        "category" => "Create Category",
        "notifications" => "Notification Settings",
        "privacy" => "Privacy Settings",
        "topic" => "Channel Topic",
        "share" => "Share Your Screen",
        "image" => "workspace.jpg",
        "add-friend" => "Add Friend",
        "new-dm" => "New Message",
        "add-members" => "Add Members",
        "edit-message" => "Edit Message",
        "delete-server" => "Delete Server",
        "logout" => "Log Out",
        "delete-role" => "Delete Role",
        "delete-message" => "Delete Message",
        "block" => "Block Maya",
        "unlink-google" => "Disconnect Google",
        _ => "Edit Account",
    };
    view!{<div class=format!("modal-backdrop modal-{}",vm.modal) data-backdrop><section class=if vm.modal=="sheet"{"modal action-sheet"}else{"modal"} role="dialog" aria-modal="true" aria-labelledby="modal-title" tabindex="-1"><div class="sheet-handle"></div><header class="modal-header"><h2 id="modal-title">{title}</h2>{icon_button("x","Close","close")}</header><div class="modal-content">{match vm.modal.as_str(){
        "create"=>view!{<div class="choice-cards"><button data-action="modal:create-server">{icon("plus-circle")}<span><strong>"Create a server"</strong><small>"Start a server for your friends."</small></span>{icon("chevron-right")}</button><button data-action="modal:join-server">{icon("users")}<span><strong>"Join a server"</strong><small>"Use an invite from a friend."</small></span>{icon("chevron-right")}</button></div>}.into_any(),
        "create-server"=>view!{{input("Server name","","text","")}{button("Create Server","toast:Server created locally","primary")}}.into_any(),
        "join-server"=>view!{{input("Invite code","","text","Paste the invite code shared by your friend.")}{button("Join Server","toast:Server joined locally","primary")}}.into_any(),
        "invite"=>view!{<p class="muted-copy">"Share this link to invite friends to #general."</p><div class="invite-code"><code>"sticktalk.ts.net/invite/den-friends"</code>{button("Copy","copy","primary")}</div><small class="muted-copy">"Your invite link expires in 7 days."</small><div class="modal-footer">{button("Edit Invite Link","modal:invite-options","link-button")}</div>}.into_any(),
        "sheet"=>view!{<div class="sheet-message">{avatar(selected_author,"small")}<span><strong>{selected_author.name.clone()}</strong><small>{selected.body.clone()}</small></span></div><div class="sheet-reactions">{["1f44d","2764","1f602","1f525","1f389"].into_iter().map(|e|view!{<button data-action=format!("react:{},{}",vm.selected_message,e) aria-label="React"><img src=format!("public/emoji/{e}.png") alt="Reaction" width="28" height="28"/></button>}).collect_view()}</div>{menu_item("Reply","reply","toast:Reply selected",false)}{menu_item("Add Reaction","smile","modal:emoji",false)}{menu_item("Copy Text","copy","copy",false)}{menu_item("Pin Message","pin","toast:Message pinned locally",false)}{menu_item("Mark Unread","mail","close",false)}{menu_item("Delete Message","trash-2","modal:delete-message",true)}}.into_any(),
        "quick"=>view!{{search("Where would you like to go?","quick-results")}<div class="quick-results" id="quick-results">{[("general","hash","channel:general"),("projects","hash","channel:projects"),("Maya","user-round","screen:dm"),("Lounge","volume-2","screen:voice"),("The Den","message-circle","server:The Den")].into_iter().map(|(label,i,action)|view!{<button data-filter-row data-action=action>{icon(i)}<span>{label}</span><small>"Enter to jump"</small></button>}).collect_view()}</div><small class="muted-copy">"Use arrow keys to select, Enter to jump, Esc to close."</small>}.into_any(),
        "emoji"=>view!{{search("Search emoji","emoji-picker")}<div class="emoji-grid" id="emoji-picker">{[("1f44b","Wave"),("1f44d","Thumbs up"),("2764","Heart"),("1f602","Laugh"),("1f525","Fire"),("1f389","Celebrate"),("1f680","Rocket"),("2728","Sparkles"),("2705","Check"),("1f331","Seedling"),("1f440","Eyes"),("1f604","Smile")].into_iter().map(|(e,label)|view!{<button data-filter-row data-action=format!("react:{},{}",vm.selected_message,e) aria-label=label><img src=format!("public/emoji/{e}.png") alt=label width="32" height="32"/></button>}).collect_view()}</div>}.into_any(),
        "gif"=>empty("image","Search GIFs when connected","Close","close"),
        "pins"=>view!{<div class="pinned-message"><div class="message-meta"><strong>"Alex Chen"</strong><time>"Nov 10, 2026"</time></div><p>"Game night on Thursday at 7:30. Join us in Lounge."</p>{button("Jump to Message","screen:chat","secondary")}</div>}.into_any(),
        "notifications"=>view!{<p class="muted-copy">"Choose which messages notify you."</p>{["All Messages","Only @mentions","Nothing"].into_iter().enumerate().map(|(i,l)|view!{<label class="radio-row"><input type="radio" name="notifications" checked=i==1/>{l}</label>}).collect_view()}<div class="section-divider"></div>{toggle("Mute this channel","Suppress all notifications for this channel.",false,"toggle-local")}{button("Done","close","primary")}}.into_any(),
        "privacy"=>view!{{toggle("Direct Messages","Allow messages from members of this server.",true,"toggle-local")}{button("Done","close","primary")}}.into_any(),
        "topic"=>view!{<p>"A place for the everyday stuff. Share what you're working on, make plans, or just say hi."</p>}.into_any(),
        "channel"|"category"=>view!{{input(if vm.modal=="category"{"Category name"}else{"Channel name"},&vm.channel,"text","")}{input("Topic","A place for the everyday stuff.","text","")}{toggle("Private Channel","Only selected roles and members can view this channel.",false,"toggle-local")}{button("Save Changes","save","primary")}}.into_any(),
        "share"=>view!{<p class="muted-copy desktop-share-hint">"Choose a screen or window to share."</p><p class="muted-copy mobile-share-hint">"Screen sharing is available in desktop browsers. You can watch streams on your phone."</p><button class="share-source" data-action="toast:Screen selection requires a connected call">{icon("monitor")}<span>"Screen 1"</span></button><div class="share-quality"><label>"Resolution"<select><option>"480p"</option><option>"720p"</option><option selected>"1080p"</option><option>"1440p"</option><option>"Source"</option></select></label><label>"Frame Rate"<select><option>"15 FPS"</option><option>"30 FPS"</option><option selected>"60 FPS"</option></select></label></div>{toggle("Share Audio","Include sound from your screen.",false,"toggle-local")}{button("Go Live","toast:Screen sharing requires a connected server","primary")}}.into_any(),
        "image"=>view!{<img class="viewer-image" src="public/mock-workspace.jpg" alt="Clean desk workspace"/><small class="muted-copy">"workspace.jpg · 6.8 KB"</small>}.into_any(),
        "add-friend"=>view!{{input("Username","","text","Enter your friend's username.")}{button("Send Friend Request","toast:Friend request sent locally","primary")}}.into_any(),
        "new-dm"|"add-members"=>view!{{search("Search people","people-picker")}<div id="people-picker">{vm.people.iter().enumerate().skip(1).map(|(i,p)|view!{<button class="picker-person" data-filter-row data-action=if vm.modal=="new-dm"{"screen:dm".to_owned()}else{format!("toast:{} added locally",p.name)}>{avatar(p,"small")}<span>{p.name.clone()}</span><small>{format!("@{}",vm.people[i].username)}</small></button>}).collect_view()}</div>}.into_any(),
        "edit-message"=>view!{<label class="field">"Message"<textarea rows="3">"Got it, thanks. 7:30 tonight?"</textarea><small>"escape to cancel · enter to save"</small></label>{button("Save","toast:Message edited locally","primary")}}.into_any(),
        "edit-field"=>view!{{input("New value","","text","")}{button("Save","save","primary")}}.into_any(),
        _=>view!{<p>{if vm.modal=="logout"{"Are you sure you want to log out?"}else if vm.modal=="delete-server"{"This cannot be undone. All channels and messages will be deleted."}else{"Are you sure? This action affects your account or server."}}</p>{if vm.modal=="delete-server"{view!{<div class="field"><label for="confirm-server">"Enter the server name to confirm"</label><input id="confirm-server" data-confirm-server=vm.server.clone()/></div>}.into_any()}else{().into_any()}}<div class="modal-footer">{button("Cancel","close","secondary")}<button class="button danger" data-action=if vm.modal=="logout"{"screen:login"}else{"toast:Confirmed locally"} data-confirm disabled=vm.modal=="delete-server">{if vm.modal=="logout"{"Log Out"}else{"Confirm"}}</button></div>}.into_any()
    }}</div></section></div>}.into_any()
}
