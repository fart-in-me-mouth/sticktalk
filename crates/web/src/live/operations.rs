use super::*;
use sha2::{Digest, Sha256};
use sticktalk_client_core::{http::decode_json, upload::UploadSource, web::BlobSource};

pub(super) fn action(app: Shared, raw: &str) {
    let (kind, arg) = raw.split_once(':').unwrap_or((raw, ""));
    match kind {
        "recover" => {
            if app.borrow().model.me.is_none() {
                let http = app.borrow().http();
                spawn_local(async move {
                    match http.json::<BrowserSession>("/browser/session").await {
                        Ok(session) => start_session(app, session),
                        Err(e) => {
                            let mut a = app.borrow_mut();
                            a.status = if !window().navigator().on_line() {
                                "Reconnecting…".into()
                            } else {
                                String::new()
                            };
                            a.error = if a.status.is_empty() {
                                e
                            } else {
                                String::new()
                            };
                            a.touch();
                        }
                    }
                });
            }
        }
        "dismiss-error" => {
            let mut a = app.borrow_mut();
            a.error.clear();
            a.touch();
        }
        "auth" => {
            let http = app.borrow().http();
            let username = value("auth-username");
            let password = value("auth-password");
            let register = app.borrow().register;
            let signup = app.borrow().signup;
            {
                let mut a = app.borrow_mut();
                a.error.clear();
                a.status = "Signing in…".into();
                a.touch();
            }
            spawn_local(async move {
                let result = if signup {
                    http.post::<BrowserSession>(
                        "/browser/session/google/complete",
                        &GoogleCompleteRequest {
                            ticket: String::new(),
                            username,
                        },
                    )
                    .await
                } else if register {
                    http.post(
                        "/browser/session/register",
                        &RegisterRequest {
                            username,
                            password,
                            display_name: None,
                        },
                    )
                    .await
                } else {
                    http.post(
                        "/browser/session/login",
                        &LoginRequest { username, password },
                    )
                    .await
                };
                match result {
                    Ok(session) => start_session(app, session),
                    Err(error) => {
                        let mut a = app.borrow_mut();
                        a.error = error;
                        a.status.clear();
                        a.touch();
                    }
                }
            });
        }
        "register" => {
            let mut a = app.borrow_mut();
            a.register = !a.register;
            a.error.clear();
            a.touch();
        }
        "logout" => {
            let http = app.borrow().http();
            spawn_local(async move {
                let result = http
                    .execute("POST", "/api/v1/auth/logout", Body::Empty, vec![], 30_000)
                    .await;
                match result {
                    Ok(r) if (200..300).contains(&r.status) => {
                        media_action("leave");
                        media_action("purge");
                        let mut a = app.borrow_mut();
                        a.clear_account();
                        a.error.clear();
                        a.touch();
                    }
                    _ => fail(&app, "Could not log out. Try again.".into()),
                }
            });
        }
        "focus" => {
            let mut a = app.borrow_mut();
            if a.ready {
                a.send(ClientMsg::Focus {
                    focused: arg == "true",
                });
            }
            drop(a);
            if arg == "true" {
                mark_read(&app);
            }
        }
        "home" => {
            let mut a = app.borrow_mut();
            a.screen = "home".into();
            a.channel = None;
            a.guild = None;
            a.panel.clear();
            a.touch();
        }
        "panel" => {
            let mut a = app.borrow_mut();
            a.panel = if a.panel == arg {
                "".into()
            } else {
                arg.into()
            };
            a.touch();
        }
        "guild" => {
            if let Some(g) = id(arg) {
                {
                    let mut a = app.borrow_mut();
                    a.guild = Some(g);
                    a.panel.clear();
                    a.command(Command::SubscribeGuild {
                        guild_id: Some(g),
                        presence_user_ids: vec![],
                    });
                    a.touch();
                }
                let c = app
                    .borrow()
                    .model
                    .channels
                    .values()
                    .find(|c| c.guild_id == Some(g) && c.kind == ChannelKind::Text)
                    .map(|c| c.id);
                if let Some(c) = c {
                    load_channel(app.clone(), c, None);
                }
                load_members(app, g, None);
            }
        }
        "channel" => {
            if let Some(c) = id(arg) {
                load_channel(app, c, None)
            }
        }
        "seek" => {
            if let Ok(rank) = arg.parse() {
                let channel = app.borrow().channel;
                if let Some(c) = channel {
                    load_channel(app, c, Some(rank));
                }
            }
        }
        "typing" => {
            let mut a = app.borrow_mut();
            if Browser.monotonic_ms().saturating_sub(a.typing_sent) > 8000 {
                if let Some(c) = a.channel {
                    a.typing_sent = Browser.monotonic_ms();
                    a.command(Command::Typing { channel_id: c });
                }
            }
        }
        "send" => send_message(app, arg.to_owned()),
        "dialog" => open_dialog(app, arg),
        "close" => {
            let mut a = app.borrow_mut();
            a.dialog.clear();
            a.error.clear();
            a.profile = None;
            a.panel.clear();
            a.touch();
        }
        "submit" => submit(app, arg),
        "friend" => {
            if let Some((op, user)) = arg.split_once(',') {
                if let Some(user_id) = id(user) {
                    let command = match op {
                        "accept" => Command::AcceptFriendRequest { user_id },
                        "decline" => Command::DeclineFriendRequest { user_id },
                        "cancel" => Command::CancelFriendRequest { user_id },
                        "block" => Command::BlockUser { user_id },
                        "unblock" => Command::UnblockUser { user_id },
                        "remove" => Command::RemoveFriend { user_id },
                        _ => Command::OpenDm { user_id },
                    };
                    app.borrow_mut().command(command);
                }
            }
        }
        "reply" => {
            let mut a = app.borrow_mut();
            a.reply = id(arg);
            a.editing = None;
            a.touch();
        }
        "edit" => {
            let mut a = app.borrow_mut();
            a.editing = id(arg);
            a.reply = None;
            a.dialog = "edit".into();
            a.touch();
        }
        "cancel-reply" => {
            let mut a = app.borrow_mut();
            a.reply = None;
            a.editing = None;
            a.touch();
        }
        "delete" => {
            let channel = app.borrow().channel;
            if let (Some(c), Some(m)) = (channel, id(arg)) {
                app.borrow_mut().command(Command::DeleteMessage {
                    channel_id: c,
                    message_id: m,
                });
            }
        }
        "pin" => {
            let channel = app.borrow().channel;
            if let (Some(c), Some(m)) = (channel, id(arg)) {
                app.borrow_mut()
                    .command(Command::Launch(LaunchCommand::PinMessage {
                        channel_id: c,
                        message_id: m,
                        pinned: true,
                    }));
            }
        }
        "react" => {
            if let Some((mid, emoji)) = arg.split_once(',') {
                let mut a = app.borrow_mut();
                if let (Some(c), Some(m)) = (a.channel, id(mid)) {
                    let mine = a
                        .model
                        .messages
                        .get(&c)
                        .and_then(|v| v.get(&m))
                        .is_some_and(|m| m.reactions.iter().any(|r| r.emoji == emoji && r.me));
                    a.command(if mine {
                        Command::RemoveReaction {
                            channel_id: c,
                            message_id: m,
                            emoji: emoji.into(),
                        }
                    } else {
                        Command::AddReaction {
                            channel_id: c,
                            message_id: m,
                            emoji: emoji.into(),
                        }
                    });
                }
            }
        }
        "emoji-query" => {
            let mut a = app.borrow_mut();
            a.query = arg.chars().take(200).collect();
            a.touch();
        }
        "emoji" => {
            media_action(&format!("insert:{arg}"));
            let mut a = app.borrow_mut();
            a.dialog.clear();
            a.touch();
        }
        "profile" => {
            if let Some(u) = id(arg) {
                let http = app.borrow().http();
                let privacy = app.borrow().privacy_generation;
                spawn_local(async move {
                    match http
                        .json::<Snapshot<ProfileDetail>>(&format!("/users/{u}/profile", u = u.0))
                        .await
                    {
                        Ok(p) => {
                            let mut a = app.borrow_mut();
                            if a.privacy_generation != privacy || !a.model.snapshot_valid(&p.cursor)
                            {
                                return;
                            }
                            a.profile = Some(p.data);
                            a.dialog = "profile".into();
                            a.touch();
                        }
                        Err(e) => fail(&app, e),
                    }
                });
            }
        }
        "device-revoke" => {
            app.borrow_mut()
                .command(Command::Launch(LaunchCommand::LogoutSession {
                    session_id: arg.into(),
                }))
        }
        "logout-others" => app
            .borrow_mut()
            .command(Command::Launch(LaunchCommand::LogoutAllOthers)),
        "role" => {
            let mut a = app.borrow_mut();
            a.dialog = format!("role:{arg}");
            a.touch();
        }
        "role-delete" => {
            if let Some(r) = id(arg) {
                let mut a = app.borrow_mut();
                if let Some(g) = a.guild {
                    a.command(Command::Launch(LaunchCommand::DeleteRole {
                        guild_id: g,
                        role_id: r,
                    }));
                }
            }
        }
        "members-more" => {
            let guild = app.borrow().guild;
            if let Some(g) = guild {
                let after = app.borrow().model.members.keys().next_back().copied();
                load_members(app, g, after);
            }
        }
        "signal" => match decode_signal(arg) {
            Ok(signal) => {
                let mut a = app.borrow_mut();
                if matches!(signal, VoiceSignal::Leave) {
                    a.voice_channel = None;
                }
                a.voice(signal);
            }
            Err(e) => fail(&app, e),
        },
        "voice" => {
            if let Some(c) = id(arg) {
                let mut a = app.borrow_mut();
                if a.voice_channel != Some(c) {
                    if a.voice_channel.is_some() {
                        a.voice(VoiceSignal::Leave);
                    }
                    a.voice_channel = Some(c);
                    a.voice(VoiceSignal::Join {
                        channel_id: c,
                        capabilities: VoiceCapabilities {
                            video: vec![VideoCodec::H264],
                            hardware_decode: vec![],
                        },
                    });
                }
                a.screen = "voice".into();
                a.channel = Some(c);
                a.panel.clear();
                a.touch();
            }
        }
        "media" => media_action(arg),
        "media-error" => fail(&app, arg.into()),
        "upload" => upload(app),
        "upload-cancel" => {
            app.borrow_mut().upload_cancel = true;
        }
        "remove-attachment" => {
            let mut a = app.borrow_mut();
            if let Ok(index) = arg.parse::<usize>() {
                if index < a.attachments.len() {
                    a.attachments.remove(index);
                }
            }
            a.touch();
        }
        "gif-search" => gif_search(app, arg),
        "gif-send" => {
            let url = app
                .borrow()
                .gifs
                .iter()
                .find(|g| g.id == arg)
                .map(|g| g.send.url.clone());
            if let Some(url) = url {
                send_message(app.clone(), url);
                app.borrow_mut().dialog.clear();
            }
        }
        "read" => mark_read(&app),
        _ => {}
    }
}
fn fail(app: &Shared, error: String) {
    let mut a = app.borrow_mut();
    a.error = error;
    a.touch();
}
fn mentions(model: &Model, content: &str) -> (String, Vec<Id>) {
    let mut content = sticktalk_emoji::replace_shortcodes(content).into_owned();
    let mut ids = vec![];
    for u in model.users.values() {
        let token = format!("@{}", u.username);
        if content.split_whitespace().any(|word| word == token) {
            content = content
                .split_inclusive(char::is_whitespace)
                .map(|word| {
                    if word.trim_end() == token {
                        format!("<@{}>{}", u.id.0, &word[word.trim_end().len()..])
                    } else {
                        word.to_owned()
                    }
                })
                .collect();
            ids.push(u.id);
        }
    }
    (content, ids)
}
fn send_message(app: Shared, content: String) {
    let mut a = app.borrow_mut();
    let Some(channel_id) = a.channel else { return };
    if content.trim().is_empty() && a.attachments.is_empty() {
        return;
    }
    if !a.ready {
        a.error = "Reconnecting… Your draft is kept.".into();
        a.touch();
        return;
    }
    if a.pending
        .values()
        .any(|(cmd, _)| matches!(cmd,Command::SendMessage(d) if d.channel_id==channel_id))
    {
        a.error = "Your previous message is still sending. Your draft is kept.".into();
        a.touch();
        return;
    }
    let (content, mention_ids) = mentions(&a.model, &content);
    let reply_to = a.reply;
    let attachment_ids = a.attachments.iter().map(|a| a.id).collect();
    a.command(Command::SendMessage(MessageDraft {
        channel_id,
        content,
        reply_to,
        attachment_ids,
        mention_ids,
    }));
    a.reply = None;
    media_action(&format!("sent:{}", channel_id.0));
}
fn mark_read(app: &Shared) {
    if document().hidden() || !document().has_focus().unwrap_or(false) {
        return;
    }
    if document()
        .get_element_by_id("live-messages")
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        .is_none_or(|e| e.scroll_height() - e.scroll_top() - e.client_height() > 96)
    {
        return;
    }
    let mut a = app.borrow_mut();
    if let Some(c) = a.channel {
        if let Some(last) = a
            .model
            .messages
            .get(&c)
            .and_then(|m| m.keys().next_back())
            .copied()
        {
            if let Some(cmd) = a.model.mark_read_command(c, last) {
                a.command(cmd);
            }
        }
    }
}

pub(super) fn load_channel(app: Shared, c: Id, rank: Option<u64>) {
    if rank.is_some() && app.borrow().history_busy && app.borrow().channel == Some(c) {
        return;
    }
    if app
        .borrow()
        .model
        .channels
        .get(&c)
        .is_some_and(|c| c.kind == ChannelKind::Voice)
    {
        action(app, &format!("voice:{c}", c = c.0));
        return;
    }
    let (abort, registration) = futures_util::future::AbortHandle::new_pair();
    let (http, generation, anchor) = {
        let mut a = app.borrow_mut();
        if let Some(previous) = a.history_abort.replace(abort) {
            previous.abort();
        }
        a.history_generation += 1;
        a.history_busy = true;
        if a.channel != Some(c) {
            a.reply = None;
            a.editing = None;
            let old = std::mem::take(&mut a.attachments);
            if let Some(previous) = a.channel {
                if !old.is_empty() {
                    a.channel_attachments.insert(previous, old);
                }
            }
            a.attachments = a.channel_attachments.remove(&c).unwrap_or_default();
            a.extent = None;
            a.window_ids.clear();
        }
        a.channel = Some(c);
        a.guild = a.model.channels.get(&c).and_then(|c| c.guild_id);
        a.screen = "chat".into();
        a.panel.clear();
        a.model.touch(c);
        let anchor = rank.and_then(|r| {
            a.extent
                .as_ref()
                .and_then(|e| e.anchors.iter().min_by_key(|v| v.rank.abs_diff(r)).cloned())
        });
        a.touch();
        (a.http(), a.history_generation, anchor)
    };
    let request = async move {
        let path = format!(
            "/channels/{c}/messages?limit=50{}",
            anchor
                .as_ref()
                .map_or(String::new(), |v| format!("&around={}", v.message_id.0)),
            c = c.0
        );
        let result = http.json::<Page<Message>>(&path).await;
        let extent = http
            .json::<Snapshot<HistoryExtent>>(&format!("/channels/{c}/history-extent", c = c.0))
            .await;
        let mut a = app.borrow_mut();
        if a.history_generation != generation || a.channel != Some(c) {
            return;
        }
        a.history_busy = false;
        match result {
            Ok(mut page) => {
                page.items.sort_by_key(|m| m.id);
                a.window_ids = page.items.iter().map(|m| m.id).collect();
                if let Ok(extent) = extent {
                    if a.model.snapshot_valid(&extent.cursor) {
                        a.extent = Some(extent.data);
                    }
                }
                let count = a
                    .extent
                    .as_ref()
                    .map_or(page.items.len() as u64, |v| v.message_count);
                a.window_rank = if let Some(anchor) = anchor {
                    let index = page
                        .items
                        .iter()
                        .position(|m| m.id == anchor.message_id)
                        .unwrap_or(page.items.len() / 2) as u64;
                    crate::wire::page_rank(
                        count,
                        page.items.len(),
                        Some((anchor.rank, index as usize)),
                    )
                } else {
                    crate::wire::page_rank(count, page.items.len(), None)
                };
                a.model.merge_page(c, page);
                if rank.is_none() {
                    media_action("present");
                }
                a.touch();
                drop(a);
                mark_read(&app);
            }
            Err(e) => {
                a.error = e;
                a.touch();
            }
        }
    };
    spawn_local(async move {
        let _ = futures_util::future::Abortable::new(request, registration).await;
    });
}
fn load_members(app: Shared, g: Id, after: Option<Id>) {
    let http = app.borrow().http();
    spawn_local(async move {
        match http
            .json::<Page<Member>>(&format!(
                "/guilds/{g}/members?limit=50{}",
                after.map_or(String::new(), |u| format!("&after={u}", u = u.0)),
                g = g.0
            ))
            .await
        {
            Ok(page) => {
                let mut a = app.borrow_mut();
                if a.guild != Some(g) || !a.model.snapshot_valid(&page.cursor) {
                    return;
                }
                if after.is_none() {
                    a.model.members.clear();
                }
                for u in page.items {
                    a.model.users.insert(u.user.id, u.user.clone());
                    a.model.members.insert(u.user.id, u);
                }
                a.touch();
            }
            Err(e) => fail(&app, e),
        }
    });
}
fn open_dialog(app: Shared, name: &str) {
    {
        let mut a = app.borrow_mut();
        a.dialog = name.into();
        if name == "pins" {
            a.pins.clear();
        }
        a.query.clear();
        a.error.clear();
        a.touch();
    }
    if name == "media-settings" {
        media_action("devices");
    }
    let http = app.borrow().http();
    let privacy = app.borrow().privacy_generation;
    let name = name.to_owned();
    let guild = app.borrow().guild;
    let channel = app.borrow().channel;
    spawn_local(async move {
        let result: Result<(), String> = async {
            match name.as_str() {
                "devices" => {
                    let p: Snapshot<Vec<DeviceSession>> = http.json("/me/sessions").await?;
                    if app.borrow().privacy_generation != privacy
                        || !app.borrow().model.snapshot_valid(&p.cursor)
                    {
                        return Ok(());
                    }
                    app.borrow_mut().devices = p.data;
                }
                "roles" => {
                    if let Some(g) = guild {
                        let p: Snapshot<Vec<Role>> =
                            http.json(&format!("/guilds/{g}/roles", g = g.0)).await?;
                        let mut a = app.borrow_mut();
                        if a.privacy_generation != privacy || !a.model.snapshot_valid(&p.cursor) {
                            return Ok(());
                        }
                        a.roles.retain(|_, r| r.guild_id != g);
                        for r in p.data {
                            a.roles.insert(r.id, r);
                        }
                    }
                }
                "pins" => {
                    if let Some(c) = channel {
                        let p: Snapshot<Vec<Message>> =
                            http.json(&format!("/channels/{c}/pins", c = c.0)).await?;
                        if app.borrow().privacy_generation != privacy
                            || !app.borrow().model.snapshot_valid(&p.cursor)
                        {
                            return Ok(());
                        }
                        app.borrow_mut().pins = p.data;
                    }
                }
                "notifications" => {
                    let p: Snapshot<Vec<NotificationSettings>> =
                        http.json("/me/notifications").await?;
                    if app.borrow().privacy_generation != privacy
                        || !app.borrow().model.snapshot_valid(&p.cursor)
                    {
                        return Ok(());
                    }
                    app.borrow_mut().notifications = p.data;
                }
                _ => {}
            }
            Ok(())
        }
        .await;
        if let Err(e) = result {
            fail(&app, e)
        } else {
            app.borrow().touch();
        }
    });
}
fn submit(app: Shared, form: &str) {
    let name = value("f-name");
    let text = value("f-value");
    let extra = value("f-extra");
    let mut a = app.borrow_mut();
    let channel = a.channel;
    let guild = a.guild;
    let cmd = match form {
        "friend" => Some(Command::SendFriendRequest { username: name }),
        "create" => Some(Command::CreateGuild {
            name,
            icon_id: None,
        }),
        "join" => Some(Command::JoinGuild { invite_code: name }),
        "channel" => guild.map(|guild_id| Command::CreateChannel {
            guild_id,
            kind: if extra == "voice" {
                ChannelKind::Voice
            } else if extra == "category" {
                ChannelKind::Category
            } else {
                ChannelKind::Text
            },
            name,
            parent_id: id(&text),
            topic: None,
        }),
        "invite" => guild.map(|guild_id| Command::CreateInvite {
            guild_id,
            ttl_seconds: 604800,
            max_uses: 100,
        }),
        "role-create" => {
            guild.map(|guild_id| Command::Launch(LaunchCommand::CreateRole { guild_id, name }))
        }
        "role-save" => id(&extra)
            .and_then(|r| a.roles.get(&r).cloned())
            .map(|mut role| {
                role.name = name;
                role.color =
                    u32::from_str_radix(text.trim_start_matches('#'), 16).unwrap_or(role.color);
                if let Some(bits) = id(&value("f-permissions")) {
                    role.permissions = Permissions::from_bits_retain(bits.0);
                } else if value("f-permissions") == "0" {
                    role.permissions = Permissions::empty();
                }
                role.hoist = checked("f-hoist");
                role.mentionable = checked("f-mentionable");
                Command::Launch(LaunchCommand::UpdateRole { role })
            }),
        "member-role" => {
            if let (Some(guild_id), Some(user_id)) = (guild, id(&name)) {
                Some(Command::Launch(LaunchCommand::SetMemberRoles {
                    guild_id,
                    user_id,
                    role_ids: text.split(',').filter_map(id).collect(),
                }))
            } else {
                None
            }
        }
        "private" => channel.map(|channel_id| {
            let mut overwrites = vec![PermissionOverwrite {
                kind: OverwriteKind::Role,
                target_id: guild.unwrap_or(channel_id),
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
            }];
            if let Some(role) = id(&name) {
                overwrites.push(PermissionOverwrite {
                    kind: OverwriteKind::Role,
                    target_id: role,
                    allow: Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
                    deny: Permissions::empty(),
                });
            }
            Command::Launch(LaunchCommand::SetOverwrites {
                channel_id,
                overwrites,
            })
        }),
        "edit" => {
            if let (Some(channel_id), Some(message_id)) = (channel, a.editing) {
                let (content, mention_ids) = mentions(&a.model, &name);
                Some(Command::EditMessage(MessageEdit {
                    channel_id,
                    message_id,
                    content,
                    mention_ids,
                }))
            } else {
                None
            }
        }
        "profile" => Some(Command::UpdateProfile(ProfilePatch {
            display_name: if name.is_empty() {
                Patch::Clear
            } else {
                Patch::Set(name)
            },
            status_text: if text.is_empty() {
                Patch::Clear
            } else {
                Patch::Set(text)
            },
            ..Default::default()
        })),
        "about" => Some(Command::Launch(LaunchCommand::UpdateUserProfile {
            banner_color: 0,
            pronouns: name,
            about_me: text,
        })),
        "notifications" => {
            let level = match name.as_str() {
                "all" => NotificationLevel::All,
                "nothing" => NotificationLevel::Nothing,
                _ => NotificationLevel::Mentions,
            };
            Some(Command::Launch(LaunchCommand::SetNotifications(
                NotificationSettings {
                    guild_id: if channel.is_none() { guild } else { None },
                    channel_id: channel,
                    level,
                    muted: text == "true",
                    mute_until: None,
                    suppress_everyone: extra == "true",
                    hide_muted_channels: false,
                },
            )))
        }
        "password" => {
            let http = a.http();
            drop(a);
            spawn_local(async move {
                match http
                    .post::<serde_json::Value>(
                        "/auth/password",
                        &SetPasswordRequest {
                            current_password: if name.is_empty() { None } else { Some(name) },
                            new_password: text,
                        },
                    )
                    .await
                {
                    Ok(_) => {
                        let mut a = app.borrow_mut();
                        a.dialog.clear();
                        a.touch();
                    }
                    Err(e) => fail(&app, e),
                }
            });
            return;
        }
        _ => None,
    };
    if let Some(cmd) = cmd {
        a.command(cmd);
    } else {
        a.error = "Select a server or channel first.".into();
        a.touch();
    }
}
pub(super) fn response(app: Shared, response: CommandResponse) {
    let mut a = app.borrow_mut();
    let pending = a.pending.remove(&response.nonce);
    match response.outcome {
        CommandOutcome::Err(error) => {
            if let Some((Command::SendMessage(draft), _)) = &pending {
                media_action(&format!(
                    "restore-draft:{}:{}",
                    draft.channel_id.0, draft.content
                ));
                if a.channel == Some(draft.channel_id) && a.reply.is_none() {
                    a.reply = draft.reply_to;
                }
            }
            a.error = error.message;
            a.touch();
        }
        CommandOutcome::Ok(result) => {
            let operation = pending.map(|(c, _)| c);
            a.error.clear();
            match result {
                CommandResult::Message(message) => {
                    // Responses identify the committed operation; only events may
                    // replace live entities (receipt snapshots can be historical).
                    if let Some(Command::SendMessage(draft)) = &operation {
                        if a.channel == Some(message.channel_id) {
                            a.attachments
                                .retain(|f| !draft.attachment_ids.contains(&f.id));
                        } else if let Some(files) =
                            a.channel_attachments.get_mut(&message.channel_id)
                        {
                            files.retain(|f| !draft.attachment_ids.contains(&f.id));
                        }
                    }
                    if a.channel == Some(message.channel_id) {
                        a.editing = None;
                        a.dialog.clear();
                    }
                }
                CommandResult::Channel(c) => {
                    let id = c.summary.id;
                    if !a.model.channels.contains_key(&id) {
                        a.touch();
                        return;
                    }
                    a.dialog.clear();
                    a.touch();
                    drop(a);
                    load_channel(app, id, None);
                    return;
                }
                CommandResult::Guild(g) => {
                    let id = g.summary.guild.id;
                    if !a.model.guilds.contains_key(&id) {
                        a.touch();
                        return;
                    }
                    a.guild = Some(id);
                    a.dialog.clear();
                    a.touch();
                    drop(a);
                    action(app, &format!("guild:{id}", id = id.0));
                    return;
                }
                CommandResult::Invite(_) => {
                    // The authorized InviteUpsert event owns the current code.
                }
                CommandResult::Role(r) => {
                    let _ = r; // RolesUpdated owns the projection.
                    a.dialog = "roles".into();
                }
                _ => {
                    if !matches!(
                        operation,
                        Some(
                            Command::Typing { .. }
                                | Command::MarkRead { .. }
                                | Command::SubscribeGuild { .. }
                        )
                    ) {
                        a.dialog.clear();
                    }
                }
            }
            a.touch();
        }
    }
}
fn gif_search(app: Shared, query: &str) {
    let http = app.borrow().http();
    let q = js_sys::encode_uri_component(query)
        .as_string()
        .unwrap_or_default();
    let capability = app.borrow().gif_capability.clone();
    spawn_local(async move {
        match capability {
            GifCapability::Enabled(GifConfig::Proxy { .. }) => {
                let route = if q.is_empty() {
                    "/gifs/trending".into()
                } else {
                    format!("/gifs/search?q={q}")
                };
                match http.json::<GifPage>(&route).await {
                    Ok(p) => {
                        let mut a = app.borrow_mut();
                        a.gifs = p.items;
                        a.touch();
                    }
                    Err(e) => fail(&app, e),
                }
            }
            GifCapability::Enabled(GifConfig::Direct { .. }) => fail(
                &app,
                "Direct GIF discovery is unavailable under this server's same-origin policy."
                    .into(),
            ),
            _ => fail(&app, "GIF search is not enabled on this server.".into()),
        }
    });
}
fn upload(app: Shared) {
    let input = document()
        .get_element_by_id("live-file")
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok());
    let Some(file) = input.and_then(|e| e.files()).and_then(|f| f.get(0)) else {
        return;
    };
    let (http, channel, epoch) = {
        let mut a = app.borrow_mut();
        if a.uploading {
            return;
        }
        let Some(c) = a.channel else { return };
        a.uploading = true;
        a.upload_cancel = false;
        a.upload_progress = "Checking file…".into();
        a.touch();
        (a.http(), c, a.epoch)
    };
    spawn_local(async move {
        let result: Result<Attachment, String> = async {
            let source = BlobSource {
                blob: file.clone().unchecked_into(),
                filename: file.name(),
            };
            let len = source.byte_size().await?;
            let mut hash = Sha256::new();
            let mut chunks = source.chunks(0, len).await?;
            while let Some(bytes) = chunks.next().await {
                if app.borrow().upload_cancel || app.borrow().epoch != epoch {
                    return Err("Upload cancelled".into());
                }
                hash.update(bytes?);
            }
            let digest = format!("{:x}", hash.finalize());
            // Only nonsecret resume metadata is persisted. Re-selecting the same file
            // validates its complete digest before resuming at the server's offset.
            let key = format!("sticktalk-upload:{channel}:{digest}", channel = channel.0);
            let saved = storage_get(&key);
            let mut upload = if let Some(upload_id) = saved {
                http.json::<ResumableUpload>(&format!("/uploads/resumable/{upload_id}"))
                    .await
                    .ok()
            } else {
                None
            };
            if upload.is_none() {
                let created = http
                    .post::<ResumableUpload>(
                        "/uploads/resumable",
                        &CreateUploadRequest {
                            purpose: UploadPurpose::Message,
                            channel_id: Some(channel),
                            filename: file.name(),
                            content_type: if file.type_().is_empty() {
                                "application/octet-stream".into()
                            } else {
                                file.type_()
                            },
                            byte_size: len,
                            sha256: digest,
                        },
                    )
                    .await?;
                storage_set(&key, Some(&created.upload_id));
                upload = Some(created);
            }
            let mut upload = upload.unwrap();
            let route = format!("/api/v1/uploads/resumable/{}", upload.upload_id);
            let started = Browser.monotonic_ms();
            let initial = upload.offset;
            if let Some(done) = upload.completed {
                return Ok(done.attachment);
            }
            while upload.offset < len {
                if app.borrow().upload_cancel || app.borrow().epoch != epoch {
                    return Err("Upload paused. Select the same file to resume.".into());
                }
                let count = (len - upload.offset).min(u64::from(upload.chunk_size));
                let chunks = source.chunks(upload.offset, count).await?;
                upload = decode_json(
                    http.execute(
                        "PUT",
                        &route,
                        Body::Stream {
                            chunks,
                            length: count,
                        },
                        vec![("Upload-Offset".into(), upload.offset.to_string())],
                        120_000,
                    )
                    .await?,
                )?;
                let elapsed = (Browser.monotonic_ms() - started).max(1);
                let speed = (upload.offset - initial) * 1000 / elapsed;
                let eta = (len - upload.offset) / speed.max(1);
                let mut a = app.borrow_mut();
                a.upload_progress = format!(
                    "{}% · {} KiB/s · {} s remaining",
                    upload.offset * 100 / len.max(1),
                    speed / 1024,
                    eta
                );
                a.touch();
            }
            let done: UploadResponse = decode_json(
                http.execute(
                    "POST",
                    &format!("{route}/finish"),
                    Body::Empty,
                    vec![],
                    3_600_000,
                )
                .await?,
            )?;
            storage_set(&key, None);
            Ok(done.attachment)
        }
        .await;
        let mut a = app.borrow_mut();
        if a.epoch != epoch {
            return;
        }
        a.uploading = false;
        match result {
            Ok(file) => {
                if a.channel == Some(channel) {
                    a.attachments.push(file)
                } else {
                    a.channel_attachments.entry(channel).or_default().push(file);
                }
            }
            Err(e) => a.error = e,
        }
        a.touch();
    });
}
fn storage_get(key: &str) -> Option<String> {
    let store = js_sys::Reflect::get(window().as_ref(), &"sessionStorage".into()).ok()?;
    let get = js_sys::Reflect::get(&store, &"getItem".into())
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()?;
    get.call1(&store, &key.into()).ok()?.as_string()
}
fn storage_set(key: &str, value: Option<&str>) {
    if let Ok(store) = js_sys::Reflect::get(window().as_ref(), &"sessionStorage".into()) {
        let method = if value.is_some() {
            "setItem"
        } else {
            "removeItem"
        };
        if let Ok(f) = js_sys::Reflect::get(&store, &method.into())
            .and_then(|v| v.dyn_into::<js_sys::Function>())
        {
            let _ = if let Some(v) = value {
                f.call2(&store, &key.into(), &v.into())
            } else {
                f.call1(&store, &key.into())
            };
        }
    }
}

fn checked(id: &str) -> bool {
    document()
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|e| e.checked())
}
