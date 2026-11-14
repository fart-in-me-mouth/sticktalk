use super::*;
use sha2::{Digest, Sha256};
use sticktalk_client_core::{http::decode_json, upload::UploadSource, web::BlobSource};

// Read the submitted surface synchronously, before loading the lazy command
// builder. Navigation may replace these shared field IDs while it downloads.
struct Submission {
    fields: BTreeMap<&'static str, String>,
    epoch: u64,
    guild: Option<Id>,
    channel: Option<Id>,
    editing: Option<Id>,
}
impl Submission {
    fn capture(a: &App) -> Self {
        let mut fields = [
            "f-name",
            "f-value",
            "f-extra",
            "f-permissions",
            "f-target",
            "server-description",
            "server-banner",
        ]
        .into_iter()
        .map(|key| (key, value(key)))
        .collect::<BTreeMap<_, _>>();
        for key in ["f-hoist", "f-mentionable", "f-muted", "f-suppress"] {
            fields.insert(key, checked(key).to_string());
        }
        Self {
            fields,
            epoch: a.epoch,
            guild: a.guild,
            channel: a.channel,
            editing: a.editing,
        }
    }
}

pub(super) fn action(app: Shared, raw: &str) {
    let (kind, arg) = raw.split_once(':').unwrap_or((raw, ""));
    match kind {
        "dismiss-notice" => {
            let mut a = app.borrow_mut();
            if a.status.starts_with("Your ") {
                a.status.clear();
            }
            a.touch();
        }
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
        "reauth-google" => {
            if matches!(arg, "password" | "link" | "unlink" | "delete_account") {
                let _ = window()
                    .location()
                    .set_href(&format!("/api/v1/browser/google/start?operation={arg}"));
            }
        }
        "google-link" | "google-unlink" => {
            let http = app.borrow().http();
            let password = value("google-current-password");
            let linking = kind == "google-link";
            spawn_local(async move {
                let proof = if password.is_empty() {
                    None
                } else {
                    match http
                        .reauthenticate(&ReauthRequest {
                            operation: if linking {
                                AccountOperation::Link
                            } else {
                                AccountOperation::Unlink
                            },
                            current_password: Some(password),
                            id_token: None,
                        })
                        .await
                    {
                        Ok(proof) => Some(proof.proof),
                        Err(e) => {
                            fail(&app, e);
                            return;
                        }
                    }
                };
                if linking {
                    let _ = window()
                        .location()
                        .set_href("/api/v1/browser/google/start?operation=link_identity");
                } else {
                    match http
                        .account_authorized(
                            sticktalk_client_core::backend::AccountAction::Unlink,
                            proof.as_deref(),
                        )
                        .await
                    {
                        Ok(_) => {
                            app.borrow_mut().error = "Google unlinked.".into();
                            app.borrow().touch();
                        }
                        Err(e) => fail(&app, e),
                    }
                }
            });
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
                        // The authenticated logout already revoked server media.
                        // Do not send a late Leave after clearing this session.
                        end_session(&app, "");
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
        "history-older" | "history-newer" => {
            let channel = app.borrow().channel;
            if let Some(c) = channel {
                load_history(app, c, None, Some(kind == "history-older"), None);
            }
        }
        "seek-message" => {
            let channel = app.borrow().channel;
            if let (Some(c), Some(message)) = (channel, id(arg)) {
                load_history(app, c, None, None, Some(message));
            }
        }
        "present" => {
            let channel = app.borrow().channel;
            if let Some(c) = channel {
                load_channel(app, c, None);
            }
        }
        "typing" => {
            let mut a = app.borrow_mut();
            if a.typing_sent == 0 || Browser.monotonic_ms().saturating_sub(a.typing_sent) > 8000 {
                if let Some(c) = a.channel {
                    a.typing_sent = Browser.monotonic_ms();
                    a.command(Command::Typing { channel_id: c });
                }
            }
        }
        "complete" => {
            let epoch = {
                let mut a = app.borrow_mut();
                a.completion_query = arg.to_owned();
                if a.completion_busy {
                    return;
                }
                a.completion_busy = true;
                a.epoch
            };
            spawn_local(async move {
                let (request, rows) = completion_rows(app.clone()).await;
                let mut a = app.borrow_mut();
                if a.epoch != epoch {
                    return;
                }
                a.completion_busy = false;
                if a.completion_query == request {
                    a.completions = rows;
                    a.touch();
                }
            });
        }
        "complete-close" => {
            let mut a = app.borrow_mut();
            a.completion_query.clear();
            a.completions.clear();
            a.touch();
        }
        "avatar-remove" => app
            .borrow_mut()
            .command(Command::UpdateProfile(ProfilePatch {
                avatar_id: Patch::Clear,
                ..Default::default()
            })),
        "send" => send_message(app, arg.to_owned()),
        "dialog" => open_dialog(app, arg),
        "close" => {
            let mut a = app.borrow_mut();
            a.dialog.clear();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
            a.error.clear();
            a.profile = None;
            a.panel.clear();
            a.touch();
        }
        "submit" => {
            let submission = Submission::capture(&app.borrow());
            let form = arg.to_owned();
            spawn_local(async move {
                submit(app, form, submission).await;
            });
        }
        "admin-more" => {
            let (name, epoch, generation) = {
                let mut a = app.borrow_mut();
                if a.admin_busy || !a.admin_more {
                    return;
                }
                a.admin_busy = true;
                a.touch();
                (a.dialog.clone(), a.epoch, a.dialog_generation)
            };
            spawn_local(async move {
                dialog_data(app, name, epoch, generation).await;
            });
        }
        "group-remove" | "group-owner" | "group-leave" => {
            let mut a = app.borrow_mut();
            if let Some(channel_id) = a.channel {
                let cmd = if kind == "group-leave" {
                    Some(Command::LeaveGroupDm { channel_id })
                } else {
                    id(arg).map(|user_id| {
                        if kind == "group-owner" {
                            Command::TransferGroupOwnership {
                                channel_id,
                                user_id,
                            }
                        } else {
                            Command::RemoveGroupRecipient {
                                channel_id,
                                user_id,
                            }
                        }
                    })
                };
                if let Some(cmd) = cmd {
                    a.command(cmd);
                }
            }
        }
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
            let epoch = app.borrow().epoch;
            let arg = arg.to_owned();
            spawn_local(async move {
                profile_request(app, arg, epoch).await;
            });
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
        "role-move" => {
            let epoch = app.borrow().epoch;
            let arg = arg.to_owned();
            spawn_local(async move {
                move_role(app, arg, epoch).await;
            });
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
        "signal" | "signal-request" | "voice" => {
            let epoch = app.borrow().epoch;
            let kind = kind.to_owned();
            let arg = arg.to_owned();
            spawn_local(async move {
                voice_action(app, kind, arg, epoch).await;
            });
        }
        "media" => media_action(arg),
        "media-error" => fail(&app, arg.into()),
        "upload" | "avatar-upload" | "guild-icon-upload" => {
            let (control, purpose) = match kind {
                "avatar-upload" => ("live-avatar", UploadPurpose::Avatar),
                "guild-icon-upload" => ("live-guild-icon", UploadPurpose::GuildIcon),
                _ => ("live-file", UploadPurpose::Message),
            };
            let file = document()
                .get_element_by_id(control)
                .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
                .and_then(|e| e.files())
                .and_then(|f| f.get(0));
            if let Some(file) = file {
                let epoch = app.borrow().epoch;
                let channel = app.borrow().channel;
                let guild = app.borrow().guild;
                spawn_local(async move {
                    upload(app, purpose, file, channel, guild, epoch).await;
                });
            }
        }
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
        "gif-search" => {
            let epoch = app.borrow().epoch;
            let query = arg.to_owned();
            spawn_local(async move {
                gif_search(app, query, epoch).await;
            });
        }
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
    let mut content = content.to_owned();
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
#[lazy]
fn expand_shortcodes(content: String) -> String {
    sticktalk_emoji::replace_shortcodes(&content).into_owned()
}
fn send_message(app: Shared, content: String) {
    let candidate = content.split(':').any(|part| {
        !part.is_empty()
            && part.len() <= 64
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'+' || b == b'-')
    }) && content.matches(':').count() >= 2;
    if !candidate {
        send_prepared(app, content);
        return;
    }
    let (epoch, channel) = {
        let mut a = app.borrow_mut();
        if a.preparing_send {
            return;
        }
        a.preparing_send = true;
        (a.epoch, a.channel)
    };
    spawn_local(async move {
        let content = expand_shortcodes(content).await;
        let mut a = app.borrow_mut();
        if a.epoch != epoch {
            return;
        }
        a.preparing_send = false;
        if a.channel != channel {
            a.error = "The message is still in your draft.".into();
            a.touch();
            return;
        }
        drop(a);
        send_prepared(app, content);
    });
}
fn send_prepared(app: Shared, content: String) {
    let mut a = app.borrow_mut();
    let Some(channel_id) = a.channel else { return };
    if a.uploading {
        a.error = "Wait for the upload to finish, or pause it before sending.".into();
        a.touch();
        return;
    }
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
    a.completions.clear();
    a.completion_query.clear();
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
    load_history(app, c, rank, None, None);
}
fn load_history(app: Shared, c: Id, rank: Option<u64>, edge: Option<bool>, target: Option<Id>) {
    if target.is_none()
        && (rank.is_some() || edge.is_some())
        && app.borrow().history_busy
        && app.borrow().channel == Some(c)
    {
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
    let member_guild = {
        let a = app.borrow();
        a.model
            .channels
            .get(&c)
            .and_then(|c| c.guild_id)
            .filter(|g| Some(*g) != a.guild)
    };
    let (abort, registration) = futures_util::future::AbortHandle::new_pair();
    let (http, generation, anchor) = {
        let mut a = app.borrow_mut();
        if let Some(previous) = a.history_abort.replace(abort) {
            previous.abort();
        }
        a.history_generation += 1;
        a.history_busy = true;
        if a.channel != Some(c) {
            a.typing_sent = 0;
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
        a.completions.clear();
        a.completion_query.clear();
        a.channel = Some(c);
        a.guild = a.model.channels.get(&c).and_then(|c| c.guild_id);
        a.screen = "chat".into();
        a.panel.clear();
        a.model.touch(c);
        let anchor = target
            .map(|message_id| HistoryAnchor {
                message_id,
                rank: a
                    .window_ids
                    .iter()
                    .position(|id| *id == message_id)
                    .map(|i| a.window_rank + i as u64)
                    .or_else(|| {
                        a.extent.as_ref().and_then(|e| {
                            e.anchors
                                .iter()
                                .min_by_key(|v| v.message_id.0.abs_diff(message_id.0))
                                .map(|v| v.rank)
                        })
                    })
                    .unwrap_or(0),
            })
            .or_else(|| {
                edge.and_then(|older| {
                    crate::wire::adjacent_pivot(a.window_ids.len(), older).map(|index| {
                        HistoryAnchor {
                            rank: a.window_rank + index as u64,
                            message_id: a.window_ids[index],
                        }
                    })
                })
                .or_else(|| {
                    rank.and_then(|r| {
                        a.extent.as_ref().and_then(|e| {
                            e.anchors.iter().min_by_key(|v| v.rank.abs_diff(r)).cloned()
                        })
                    })
                })
            });
        a.touch();
        (a.http(), a.history_generation, anchor)
    };
    if let Some(g) = member_guild {
        load_members(app.clone(), g, None);
    }
    let request = async move {
        let path = format!(
            "/channels/{c}/messages?limit=50{}",
            anchor
                .as_ref()
                .map_or(String::new(), |v| format!("&around={}", v.message_id.0)),
            c = c.0
        );
        let extent_path = format!("/channels/{c}/history-extent", c = c.0);
        let (result, extent) = futures_util::join!(
            http.json::<Page<Message>>(&path),
            http.json::<Snapshot<HistoryExtent>>(&extent_path)
        );
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
                if let Some(message) = target {
                    media_action(&format!("history-jump:{}:{}", c.0, message.0));
                } else if let Some(rank) = rank {
                    if let Some(message) = a.window_ids.get(
                        rank.saturating_sub(a.window_rank)
                            .min(a.window_ids.len().saturating_sub(1) as u64)
                            as usize,
                    ) {
                        media_action(&format!("history-jump:{}:{}", c.0, message.0));
                    }
                }
                if rank.is_none() && edge.is_none() && target.is_none() {
                    media_action("present");
                }
                a.touch();
                drop(a);
                chat_ready();
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
        a.dialog_generation = a.dialog_generation.wrapping_add(1);
        a.admin_after = None;
        a.admin_more = false;
        a.admin_busy = matches!(name, "invite" | "server-bans");
        if name == "pins" {
            a.pins.clear();
        }
        a.query.clear();
        a.error.clear();
        a.touch();
    }
    let epoch = app.borrow().epoch;
    let generation = app.borrow().dialog_generation;
    let name = name.to_owned();
    spawn_local(async move {
        dialog_data(app, name, epoch, generation).await;
    });
}
#[lazy]
fn dialog_data(app: Shared, name: String, epoch: u64, generation: u64) {
    if app.borrow().epoch != epoch {
        return;
    }
    let name = name.as_str();
    if name == "media-settings" {
        media_action("devices");
    }
    let http = app.borrow().http();
    let privacy = app.borrow().privacy_generation;
    let name = name.to_owned();
    let guild = app.borrow().guild;
    let channel = app.borrow().channel;
    let after = app.borrow().admin_after;
    spawn_local(async move {
        let result: Result<(), String> = async {
            match name.as_str() {
                "server" | "server-overview" => {
                    if let Some(g) = guild {
                        let p: Snapshot<GuildPreview> =
                            http.json(&format!("/guilds/{}/preview", g.0)).await?;
                        let mut a = app.borrow_mut();
                        if a.epoch == epoch
                            && a.dialog_generation == generation
                            && a.privacy_generation == privacy
                        {
                            a.model.merge_settings(Snapshot {
                                cursor: p.cursor,
                                data: sticktalk_client_core::settings::SnapshotData::GuildProfile(
                                    p.data,
                                ),
                            });
                        }
                    }
                }
                "profile-edit" | "about" => {
                    let me = app.borrow().model.me.as_ref().map(|me| me.user.id);
                    if let Some(me) = me {
                        let p: Snapshot<ProfileDetail> =
                            http.json(&format!("/users/{}/profile", me.0)).await?;
                        let mut a = app.borrow_mut();
                        if a.epoch == epoch
                            && a.dialog_generation == generation
                            && a.privacy_generation == privacy
                            && a.model.snapshot_valid(&p.cursor)
                        {
                            // A save can commit while this snapshot is in flight.
                            // Replay the model's journal rather than resurrecting
                            // the previous bio/banner in the live preview.
                            a.model.merge_settings(Snapshot {
                                cursor: p.cursor,
                                data: sticktalk_client_core::settings::SnapshotData::Profile(
                                    p.data.clone(),
                                ),
                            });
                            let mut detail = p.data;
                            if let Some(profile) = a.model.settings.profiles.get(&me) {
                                detail.profile = profile.clone();
                            }
                            a.profile = Some(detail);
                        }
                    }
                }
                "devices" => {
                    let p: Snapshot<Vec<DeviceSession>> = http.json("/me/sessions").await?;
                    if app.borrow().privacy_generation != privacy
                        || !app.borrow().model.snapshot_valid(&p.cursor)
                    {
                        return Ok(());
                    }
                    app.borrow_mut().devices = p.data;
                }
                "roles" | "member-role" | "private" => {
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
                        a.model.reconcile_metadata(&p.cursor);
                    }
                }
                "invite" | "server-bans" => {
                    if let Some(g) = guild {
                        let allowed = app.borrow().model.guilds.get(&g).is_some_and(|g| {
                            g.my_role == MemberRole::Owner
                                || g.my_permissions.intersects(
                                    Permissions::ADMINISTRATOR
                                        | if name == "invite" {
                                            Permissions::CREATE_INVITES | Permissions::MANAGE_GUILD
                                        } else {
                                            Permissions::BAN_MEMBERS
                                        },
                                )
                        });
                        if !allowed {
                            return Ok(());
                        }
                        let suffix = after.map_or(String::new(), |id| format!("&after={}", id.0));
                        if name == "invite" {
                            let p: Page<Invite> = http
                                .json(&format!("/guilds/{}/invites?limit=50{suffix}", g.0))
                                .await?;
                            let mut a = app.borrow_mut();
                            if a.epoch != epoch
                                || a.dialog_generation != generation
                                || a.privacy_generation != privacy
                                || !a.model.snapshot_valid(&p.cursor)
                            {
                                return Ok(());
                            }
                            if after.is_none() {
                                a.model.invites.retain(|_, i| i.guild_id != g);
                            }
                            a.admin_more = p.has_more;
                            a.admin_after = p.next_after;
                            for i in p.items {
                                a.model.invites.insert(i.id, i);
                            }
                            a.model.reconcile_metadata(&p.cursor);
                        } else {
                            let p: Page<Ban> = http
                                .json(&format!("/guilds/{}/bans?limit=50{suffix}", g.0))
                                .await?;
                            let mut a = app.borrow_mut();
                            if a.epoch != epoch
                                || a.dialog_generation != generation
                                || a.privacy_generation != privacy
                                || !a.model.snapshot_valid(&p.cursor)
                            {
                                return Ok(());
                            }
                            if after.is_none() {
                                a.model.bans.retain(|_, b| b.guild_id != g);
                            }
                            a.admin_more = p.has_more;
                            a.admin_after = p.next_after;
                            for b in p.items {
                                a.model.bans.insert(b.user.id, b);
                            }
                            a.model.reconcile_metadata(&p.cursor);
                        }
                    }
                }
                value if value.starts_with("channel-edit:") => {
                    if let Some(c) = id(value.trim_start_matches("channel-edit:")) {
                        let p: Snapshot<Channel> = http.json(&format!("/channels/{}", c.0)).await?;
                        let mut a = app.borrow_mut();
                        if a.epoch == epoch
                            && a.dialog_generation == generation
                            && a.privacy_generation == privacy
                            && a.model.snapshot_valid(&p.cursor)
                        {
                            a.model.topics.insert(c, p.data.topic);
                            a.model.reconcile_metadata(&p.cursor);
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
                value
                    if matches!(value, "notifications" | "channel-notifications")
                        || value.starts_with("target-notifications:") =>
                {
                    let p: Snapshot<Vec<NotificationSettings>> =
                        http.json("/me/notifications").await?;
                    if app.borrow().privacy_generation != privacy
                        || !app.borrow().model.snapshot_valid(&p.cursor)
                    {
                        return Ok(());
                    }
                    let mut a = app.borrow_mut();
                    a.model.merge_settings(Snapshot {
                        cursor: p.cursor,
                        data: sticktalk_client_core::settings::SnapshotData::Notifications(p.data),
                    });
                }
                _ => {}
            }
            Ok(())
        }
        .await;
        if app.borrow().epoch != epoch || app.borrow().dialog_generation != generation {
            return;
        }
        app.borrow_mut().admin_busy = false;
        if let Err(e) = result {
            fail(&app, e)
        } else {
            app.borrow().touch();
        }
    });
}

#[lazy]
fn submit(app: Shared, form: String, submission: Submission) {
    let Submission {
        mut fields,
        epoch,
        guild,
        channel,
        editing,
    } = submission;
    let name = fields.remove("f-name").unwrap_or_default();
    let text = fields.remove("f-value").unwrap_or_default();
    let extra = fields.remove("f-extra").unwrap_or_default();
    let value = |key: &str| fields.get(key).cloned().unwrap_or_default();
    let checked = |key: &str| fields.get(key).is_some_and(|value| value == "true");
    let mut a = app.borrow_mut();
    if a.epoch != epoch {
        return;
    }
    let cmd = match form.as_str() {
        "server-profile" => guild.map(|guild_id| {
            Command::Launch(LaunchCommand::UpdateGuildProfile {
                guild_id,
                description: value("server-description"),
                banner_color: value("server-banner").parse().unwrap_or(0),
            })
        }),
        "guild-icon-remove" => guild.map(|guild_id| Command::UpdateGuild {
            guild_id,
            patch: GuildPatch {
                icon_id: Patch::Clear,
                ..Default::default()
            },
        }),
        "server-overview" => guild.map(|guild_id| Command::UpdateGuild {
            guild_id,
            patch: GuildPatch {
                name: Patch::Set(name),
                ..Default::default()
            },
        }),
        "channel-save" => id(&extra).map(|channel_id| Command::UpdateChannel {
            channel_id,
            patch: ChannelPatch {
                name: Patch::Set(name),
                topic: if text.is_empty() {
                    Patch::Clear
                } else {
                    Patch::Set(text)
                },
            },
        }),
        "channel-delete" => {
            if name != "DELETE" {
                a.error = "Type DELETE to confirm channel deletion.".into();
                a.touch();
                return;
            }
            id(&extra).map(|channel_id| Command::DeleteChannel { channel_id })
        }
        "invite-revoke" => guild.map(|guild_id| Command::RevokeInvite {
            guild_id,
            code: name,
        }),
        "unban" => guild.and_then(|guild_id| {
            id(&name).map(|user_id| Command::UnbanMember { guild_id, user_id })
        }),
        "ban" => guild.and_then(|guild_id| {
            id(&name).map(|user_id| Command::BanMember {
                guild_id,
                user_id,
                reason: if text.is_empty() { None } else { Some(text) },
            })
        }),
        "group-add" => channel.and_then(|channel_id| {
            id(&name).map(|user_id| Command::AddGroupRecipient {
                channel_id,
                user_id,
            })
        }),
        "group" => {
            let user_ids: Vec<_> = text.split(',').filter_map(id).take(15).collect();
            if user_ids.len() < 2 {
                a.error = "Choose at least 2 friends.".into();
                a.touch();
                return;
            }
            Some(Command::CreateGroupDm { name, user_ids })
        }
        "friend" => Some(Command::SendFriendRequest { username: name }),
        "create" => Some(Command::CreateGuild {
            name,
            icon_id: None,
        }),
        "join" => Some(Command::JoinGuild {
            invite_code: crate::wire::invite_code(&name),
        }),
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
            if let (Some(channel_id), Some(message_id)) = (channel, editing) {
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
            banner_color: u32::from_str_radix(extra.trim_start_matches('#'), 16)
                .unwrap_or(0x496c63),
            pronouns: name,
            about_me: text,
        })),
        "notifications" => {
            let level = match name.as_str() {
                "all" => NotificationLevel::All,
                "nothing" => NotificationLevel::Nothing,
                _ => NotificationLevel::Mentions,
            };
            let target = value("f-target");
            let Some((kind, id)) = target
                .split_once(':')
                .and_then(|(kind, value)| id(value).map(|id| (kind, id)))
            else {
                a.error = "Choose a server or conversation first.".into();
                a.touch();
                return;
            };
            let (guild_id, channel_id) = if kind == "guild" {
                (Some(id), None)
            } else {
                (a.model.channels.get(&id).and_then(|c| c.guild_id), Some(id))
            };
            Some(Command::Launch(LaunchCommand::SetNotifications(
                NotificationSettings {
                    guild_id,
                    channel_id,
                    level,
                    muted: checked("f-muted"),
                    mute_until: None,
                    suppress_everyone: checked("f-suppress"),
                    hide_muted_channels: false,
                },
            )))
        }
        "password" => {
            let http = a.http();
            drop(a);
            // The current socket is revoked before the rotated-cookie HTTP
            // response arrives. Do not let an old-cookie cleanup race it.
            media_action("password-changing");
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
                        let _ = window().location().reload();
                    }
                    Err(e) => {
                        media_action("password-finished");
                        fail(&app, e);
                    }
                }
            });
            return;
        }
        "delete-account" => {
            if text != "DELETE" {
                a.error = "Type DELETE to confirm account erasure.".into();
                a.touch();
                return;
            }
            let http = a.http();
            drop(a);
            spawn_local(async move {
                let proof = if name.is_empty() {
                    None
                } else {
                    match http
                        .reauthenticate(&ReauthRequest {
                            operation: AccountOperation::DeleteAccount,
                            current_password: Some(name),
                            id_token: None,
                        })
                        .await
                    {
                        Ok(p) => Some(p.proof),
                        Err(e) => {
                            fail(&app, e);
                            return;
                        }
                    }
                };
                // Empty proof leaves the HttpOnly Google proof cookie authoritative.
                let result = if let Some(proof) = proof {
                    http.delete_account(&proof).await
                } else {
                    match http
                        .execute(
                            "POST",
                            "/api/v1/auth/account/delete",
                            sticktalk_client_core::http::Body::Bytes(b"{}".to_vec()),
                            vec![("Content-Type".into(), "application/json".into())],
                            30_000,
                        )
                        .await
                    {
                        Ok(r) if r.status == 204 => Ok(()),
                        Ok(r) => decode_json::<serde_json::Value>(r).map(|_| ()),
                        Err(e) => Err(e),
                    }
                };
                match result {
                    Ok(()) => {
                        end_session(&app, "Your account was deleted.");
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
            if matches!(operation, Some(Command::UpdateProfile(_))) {
                media_action("profile-saved");
            }
            if matches!(
                operation,
                Some(Command::Launch(
                    LaunchCommand::LogoutSession { .. } | LaunchCommand::LogoutAllOthers
                ))
            ) && a.dialog == "devices"
            {
                a.touch();
                drop(a);
                action(app, "dialog:devices");
                return;
            }
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
                    if matches!(operation, Some(Command::UpdateGuild { .. })) {
                        a.touch();
                        return;
                    }
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
                                | Command::UpdateProfile(_)
                                | Command::Launch(LaunchCommand::UpdateUserProfile { .. })
                                | Command::Launch(LaunchCommand::UpdateGuildProfile { .. })
                                | Command::Launch(LaunchCommand::ReorderRoles { .. })
                                | Command::Launch(LaunchCommand::LogoutSession { .. })
                                | Command::Launch(LaunchCommand::LogoutAllOthers)
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
#[wasm_bindgen(raw_module = "/public/live.js")]
extern "C" {
    #[wasm_bindgen(js_name = directSearch)]
    fn direct_search(key: &str, query: &str) -> js_sys::Promise;
}
#[lazy]
fn move_role(app: Shared, arg: String, epoch: u64) {
    let mut a = app.borrow_mut();
    if a.epoch != epoch {
        return;
    }
    let Some(guild_id) = a.guild else { return };
    let Some((role, direction)) = arg.split_once(',') else {
        return;
    };
    let Some(role) = id(role) else { return };
    let mut roles = a
        .roles
        .values()
        .filter(|r| r.guild_id == guild_id)
        .collect::<Vec<_>>();
    roles.sort_by_key(|r| (r.position, r.id));
    let Some(index) = roles.iter().position(|r| r.id == role) else {
        return;
    };
    let next = match direction {
        "up" if index + 1 < roles.len() => index + 1,
        "down" if index > 1 => index - 1,
        position => match position.parse::<usize>() {
            Ok(position) if position > 0 && position < roles.len() => roles.len() - position,
            _ => return,
        },
    };
    if index == 0 {
        return;
    }
    let moved = roles.remove(index);
    roles.insert(next, moved);
    let role_ids = roles.into_iter().map(|r| r.id).collect();
    a.command(Command::Launch(LaunchCommand::ReorderRoles {
        guild_id,
        role_ids,
    }));
}
#[lazy]
fn gif_search(app: Shared, query: String, epoch: u64) {
    if app.borrow().epoch != epoch {
        return;
    }
    let http = app.borrow().http();
    let q = js_sys::encode_uri_component(&query)
        .as_string()
        .unwrap_or_default();
    let capability = app.borrow().gif_capability.clone();
    let raw_query = query.to_owned();
    let privacy = app.borrow().privacy_generation;
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
            GifCapability::Enabled(GifConfig::Direct {
                public_client_key, ..
            }) => {
                let result = wasm_bindgen_futures::JsFuture::from(direct_search(
                    &public_client_key,
                    &raw_query,
                ))
                .await
                .map_err(|_| "GIF provider unavailable. Try again later.".to_string())
                .and_then(|v| {
                    serde_json::from_str::<GifPage>(&v.as_string().unwrap_or_default())
                        .map_err(|_| "Invalid GIF response".into())
                });
                if app.borrow().privacy_generation != privacy {
                    return;
                }
                match result {
                    Ok(page) => {
                        let mut a = app.borrow_mut();
                        a.gifs = page.items;
                        a.touch();
                    }
                    Err(e) => fail(&app, e),
                }
            }
            _ => fail(&app, "GIF search is not enabled on this server.".into()),
        }
    });
}
#[lazy]
fn upload(
    app: Shared,
    purpose: UploadPurpose,
    file: web_sys::File,
    channel: Option<Id>,
    guild: Option<Id>,
    epoch: u64,
) {
    if app.borrow().epoch != epoch {
        return;
    }
    let (http, channel, epoch) = {
        let mut a = app.borrow_mut();
        if a.uploading {
            return;
        }
        if purpose == UploadPurpose::Message && a.attachments.len() >= MAX_MESSAGE_ATTACHMENTS {
            a.error = "You can attach up to 4 files to a message.".into();
            a.touch();
            return;
        }
        if purpose == UploadPurpose::GuildIcon && guild.is_none() {
            return;
        }
        let c = if purpose != UploadPurpose::Message {
            None
        } else {
            let Some(c) = channel else { return };
            Some(c)
        };
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
            let key = format!(
                "sticktalk-upload:{channel}:{digest}",
                channel = channel.map_or_else(
                    || if purpose == UploadPurpose::Avatar {
                        "avatar".into()
                    } else {
                        format!("guild-icon:{}", guild.unwrap().0)
                    },
                    |c| c.0.to_string()
                )
            );
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
                            purpose,
                            channel_id: channel,
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
                if purpose == UploadPurpose::Avatar {
                    a.command(Command::UpdateProfile(ProfilePatch {
                        avatar_id: Patch::Set(file.id),
                        ..Default::default()
                    }));
                } else if purpose == UploadPurpose::GuildIcon {
                    if let Some(guild_id) = guild.filter(|id| a.model.guilds.contains_key(id)) {
                        a.command(Command::UpdateGuild {
                            guild_id,
                            patch: GuildPatch {
                                icon_id: Patch::Set(file.id),
                                ..Default::default()
                            },
                        });
                    }
                } else if let Some(channel) = channel {
                    if a.channel == Some(channel) {
                        a.attachments.push(file)
                    } else {
                        a.channel_attachments.entry(channel).or_default().push(file);
                    }
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

#[lazy]
fn completion_rows(app: Shared) -> (String, Vec<(String, String, String)>) {
    let a = app.borrow();
    let request = a.completion_query.clone();
    let Some((kind, query)) = request.split_once('|') else {
        return (request, vec![]);
    };
    let query = query.to_lowercase();
    let mut rows = vec![];
    if kind == "@" {
        for u in a
            .model
            .users
            .values()
            .filter(|u| a.guild.is_none() || a.model.members.contains_key(&u.id))
        {
            let label = u.display_name.as_deref().unwrap_or(&u.username);
            if label.to_lowercase().contains(&query) || u.username.to_lowercase().contains(&query) {
                rows.push((label.into(), format!("@{}", u.username), "User".into()));
            }
            if rows.len() >= 8 {
                break;
            }
        }
        for r in a
            .roles
            .values()
            .filter(|r| {
                Some(r.guild_id) == a.guild
                    && r.mentionable
                    && r.name.to_lowercase().contains(&query)
            })
            .take(8 - rows.len().min(8))
        {
            rows.push((r.name.clone(), format!("<@&{}>", r.id.0), "Role".into()));
        }
    } else if kind == "#" {
        for c in a
            .model
            .channels
            .values()
            .filter(|c| {
                c.guild_id == a.guild
                    && c.kind == ChannelKind::Text
                    && c.name
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query)
            })
            .take(8)
        {
            let label = c.name.clone().unwrap_or_default();
            rows.push((label.clone(), format!("#{label}"), "Channel".into()));
        }
    } else if kind == ":" {
        for e in sticktalk_emoji::search(&query, 8) {
            rows.push((e.name().into(), e.emoji().into(), "Emoji".into()));
        }
    }
    (request, rows)
}

#[lazy]
fn voice_action(app: Shared, kind: String, arg: String, epoch: u64) {
    if app.borrow().epoch != epoch {
        return;
    }
    let arg = arg.as_str();
    match kind.as_str() {
        "signal" | "signal-request" => {
            let (request_nonce, raw) = if kind == "signal-request" {
                let Some((id, raw)) = arg.split_once(':') else {
                    return;
                };
                let Ok(id) = id.parse::<u64>() else { return };
                if !(1..=9_007_199_254_740_991).contains(&id) {
                    return;
                }
                (Some(id), raw)
            } else {
                (None, arg)
            };
            match decode_signal(raw) {
                Ok(signal) => {
                    let mut a = app.borrow_mut();
                    if matches!(signal, VoiceSignal::Leave) {
                        a.voice_channel = None;
                    }
                    if let Some(nonce) = request_nonce {
                        a.voice_request(signal, nonce);
                    } else {
                        a.voice(signal);
                    }
                }
                Err(e) => fail(&app, e),
            }
        }
        "voice" => {
            if let Some(c) = id(arg) {
                let mut a = app.borrow_mut();
                if a.voice_channel != Some(c) {
                    if a.voice_channel.is_some() {
                        a.voice(VoiceSignal::Leave);
                    }
                    media_action("joining");
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
        _ => {}
    }
}

#[lazy]
fn profile_request(app: Shared, arg: String, epoch: u64) {
    if app.borrow().epoch != epoch {
        return;
    }
    let arg = arg.as_str();

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
                    if a.privacy_generation != privacy || !a.model.snapshot_valid(&p.cursor) {
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
