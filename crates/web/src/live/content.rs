//! Rich content in the web app: polls, forwarding, scheduled messages, voice
//! messages, attachment descriptions and spoilers, link previews and GIF
//! favorites. Views are in `ui/content.rs`; recording and playback are in
//! `public/content.js`, which is loaded only when one of them is used.
use super::*;
use sticktalk_client_core::http::Body;

/// Everything this module keeps between renders. Every list is bounded.
#[derive(Default)]
pub(super) struct State {
    /// Descriptions and spoiler flags for files waiting in the composer.
    pub meta: BTreeMap<Id, AttachmentMeta>,
    /// A voice recording on its way up: (file name, its details). It is sent
    /// as soon as its upload finishes.
    pub voice: Option<(String, AttachmentMeta)>,
    /// Your scheduled messages (all conversations, at most 50).
    pub scheduled: Vec<ScheduledMessage>,
    pub forward_query: String,
    pub forward_selected: Vec<Id>,
    /// Answer rows shown in the poll form (2 to 10).
    pub poll_answers: usize,
    /// (message, answer, voters, more to load).
    pub voters: Option<(Id, u8, Vec<UserSummary>, bool)>,
    /// GIF picker: "" (browse), "favorites" or "search".
    pub gif_tab: String,
    pub gif_categories: Vec<GifCategory>,
    pub gif_favorites: Vec<String>,
    pub favorite_items: Vec<GifItem>,
    pub gif_busy: bool,
    /// Spoilers opened and descriptions shown, by attachment.
    pub revealed: std::collections::BTreeSet<Id>,
    pub alt_shown: std::collections::BTreeSet<Id>,
    /// Polls whose results you asked to see before voting.
    pub results_shown: std::collections::BTreeSet<Id>,
}

pub(super) const ACTIONS: &[&str] = &[
    "poll-vote",
    "poll-end",
    "poll-voters",
    "poll-voters-more",
    "poll-answer-add",
    "poll-answer-remove",
    "forward",
    "forward-pick",
    "forward-query",
    "embeds-remove",
    "schedule-at",
    "scheduled-send",
    "scheduled-cancel",
    "attachment-spoiler",
    "spoiler-reveal",
    "alt-toggle",
    "poll-results",
    "voice-upload",
    "jump-message",
    "voice-error",
    "gif-tab",
    "gif-category",
    "gif-star",
];
pub(super) const FORMS: &[&str] = &[
    "poll",
    "forward",
    "schedule",
    "scheduled-edit",
    "attachment-meta",
];

fn checked(id: &str) -> bool {
    document()
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|e| e.checked())
}
fn notice(a: &mut App, text: &str) {
    a.notice = text.into();
    a.error = text.into();
}

/// The draft a send command carries, for both send paths.
pub(super) fn draft_of(cmd: Option<&Command>) -> Option<&MessageDraft> {
    match cmd? {
        Command::SendMessage(d) => Some(d),
        Command::Content(ContentCommand::SendMessage { draft, .. }) => Some(draft),
        _ => None,
    }
}

/// Sending from the composer: files with a description or spoiler flag go
/// through the content send, which stores those details with them.
pub(super) fn send_command(a: &mut App, draft: MessageDraft) -> Command {
    let attachments: Vec<AttachmentMetaPatch> = draft
        .attachment_ids
        .iter()
        .filter_map(|id| {
            a.content
                .meta
                .get(id)
                .filter(|m| !m.is_empty())
                .map(|meta| AttachmentMetaPatch {
                    attachment_id: *id,
                    meta: meta.clone(),
                })
        })
        .collect();
    for id in &draft.attachment_ids {
        a.content.meta.remove(id);
    }
    if attachments.is_empty() {
        Command::SendMessage(draft)
    } else {
        Command::Content(ContentCommand::SendMessage { draft, attachments })
    }
}

/// A finished upload. A voice recording is sent right away, alone.
pub(super) fn uploaded(a: &mut App, file: &Attachment) -> bool {
    let Some((name, _)) = &a.content.voice else {
        return false;
    };
    if *name != file.filename {
        return false;
    }
    let (_, meta) = a.content.voice.take().unwrap();
    let Some(channel_id) = a.channel else {
        return true;
    };
    let reply_to = a.reply.take();
    a.command(Command::Content(ContentCommand::SendMessage {
        draft: MessageDraft {
            channel_id,
            content: String::new(),
            reply_to,
            attachment_ids: vec![file.id],
            mention_ids: vec![],
        },
        attachments: vec![AttachmentMetaPatch {
            attachment_id: file.id,
            meta,
        }],
    }));
    true
}

/// What to say when the server refuses one of these commands.
pub(super) fn refused(cmd: &ContentCommand, code: ErrorCode) -> Option<&'static str> {
    Some(match (cmd, code) {
        (ContentCommand::Subscribe, _) => return None,
        (ContentCommand::VotePoll { .. } | ContentCommand::EndPoll { .. }, ErrorCode::Conflict) => {
            "This poll has ended."
        }
        (ContentCommand::EndPoll { .. }, ErrorCode::Forbidden) => {
            "Only the poll’s author or a moderator can end it."
        }
        (ContentCommand::Forward { .. }, ErrorCode::NotFound | ErrorCode::Forbidden) => {
            "You can’t forward that message to one of those conversations."
        }
        (ContentCommand::Forward { .. }, ErrorCode::Blocked) => {
            "You can’t message one of those people."
        }
        (ContentCommand::ScheduleMessage { .. }, ErrorCode::LimitExceeded) => {
            "You can have up to 50 scheduled messages. Send or cancel one first."
        }
        (
            ContentCommand::ScheduleMessage { .. } | ContentCommand::UpdateScheduled { .. },
            ErrorCode::InvalidRequest,
        ) => "Pick a time in the next 30 days, and write a message.",
        (ContentCommand::CreatePoll(_), ErrorCode::InvalidRequest) => {
            "A poll needs a question and 2 to 10 answers."
        }
        _ => return None,
    })
}

/// A successful content command.
pub(super) fn answered(a: &mut App, cmd: Option<&Command>, result: ContentResult) {
    match result {
        ContentResult::Poll {
            channel_id,
            message_id,
            poll,
        } => set_poll(a, channel_id, message_id, poll, true),
        ContentResult::Forwarded(sent) => {
            a.dialog.clear();
            a.content.forward_selected.clear();
            a.content.forward_query.clear();
            notice(
                a,
                if sent.len() == 1 {
                    "Message forwarded."
                } else {
                    "Message forwarded to each conversation."
                },
            );
        }
        ContentResult::Scheduled(s) => {
            upsert_scheduled(a, s.clone());
            if matches!(
                cmd,
                Some(Command::Content(ContentCommand::ScheduleMessage { .. }))
            ) {
                media_action(&format!("sent:{}", s.channel_id.0));
                notice(a, &format!("Scheduled for {}.", when(s.send_at)));
            }
            a.dialog = if a.dialog.starts_with("scheduled-edit:") {
                "scheduled".into()
            } else {
                String::new()
            };
        }
        ContentResult::Ack => {
            if matches!(
                cmd,
                Some(Command::Content(ContentCommand::RemoveEmbeds { .. }))
            ) {
                notice(a, "Link previews removed.");
            }
        }
    }
}

fn upsert_scheduled(a: &mut App, s: ScheduledMessage) {
    let list = &mut a.content.scheduled;
    list.retain(|v| v.id != s.id);
    list.push(s);
    list.sort_by_key(|v| (v.send_at, v.id));
    list.truncate(MAX_SCHEDULED_PER_USER);
}

fn message_mut(a: &mut App, ch: Id, m: Id) -> Option<&mut Message> {
    a.model.messages.get_mut(&ch).and_then(|ms| ms.get_mut(&m))
}
fn set_poll(a: &mut App, ch: Id, m: Id, mut poll: Poll, mine: bool) {
    if let Some(message) = message_mut(a, ch, m) {
        if !mine {
            if let Some(old) = &message.extra.poll {
                poll.my_votes = old.my_votes.clone();
            }
        }
        message.extra.poll = Some(Box::new(poll));
    }
}

/// A content event (only sent because this session subscribed).
pub(super) fn event(a: &mut App, e: ContentEvent) {
    let me = a.model.me.as_ref().map(|me| me.user.id);
    match e {
        ContentEvent::PollUpdated {
            channel_id,
            message_id,
            mut poll,
            voter,
        } => {
            let mine = voter.as_ref().is_some_and(|v| Some(v.user_id) == me);
            if let Some(v) = voter.filter(|_| mine) {
                poll.my_votes = v.answers;
            }
            set_poll(a, channel_id, message_id, poll, mine);
            if let Some((m, answer, _, _)) = a.content.voters.clone() {
                if m == message_id && a.dialog.starts_with("poll-voters:") {
                    if let Some(app) = a.owner.upgrade() {
                        spawn_local(async move { voters(app, m, answer, false).await });
                    }
                }
            }
        }
        ContentEvent::EmbedsUpdated {
            channel_id,
            message_id,
            embeds,
        } => {
            if let Some(message) = message_mut(a, channel_id, message_id) {
                message.extra.embeds = embeds;
            }
        }
        ContentEvent::ScheduledUpserted(s) => {
            if let Some(failure) = &s.failure {
                let text = format!("A scheduled message wasn’t sent. {failure}");
                a.error = text;
            }
            upsert_scheduled(a, s);
        }
        ContentEvent::ScheduledRemoved { id, .. } => a.content.scheduled.retain(|s| s.id != id),
    }
}

/// After each fresh snapshot: subscribe to content events and load the
/// scheduled list.
pub(super) fn ready(app: Shared) {
    app.borrow_mut()
        .command(Command::Content(ContentCommand::Subscribe));
    let (http, epoch) = (app.borrow().http(), app.borrow().epoch);
    spawn_local(async move {
        if let Ok(page) = http
            .json::<ScheduledMessagesPage>("/users/@me/scheduled-messages")
            .await
        {
            let mut a = app.borrow_mut();
            if a.epoch == epoch {
                a.content.scheduled = page.items;
                a.content.scheduled.truncate(MAX_SCHEDULED_PER_USER);
                a.touch();
            }
        }
    });
}

/// A dialog of ours opened: reset its state and load what it shows.
pub(super) fn opened(app: Shared, name: &str) {
    let mut a = app.borrow_mut();
    if name == "poll" {
        a.content.poll_answers = 2;
    } else if name.starts_with("forward:") {
        a.content.forward_selected.clear();
        a.content.forward_query.clear();
    } else if name == "gif" {
        a.content.gif_tab.clear();
        a.gifs.clear();
        drop(a);
        let epoch = app.borrow().epoch;
        spawn_local(async move { gif_landing(app, epoch).await });
    }
}

async fn gif_landing(app: Shared, epoch: u64) {
    let (http, capability) = {
        let a = app.borrow();
        (a.http(), a.gif_capability.clone())
    };
    if let Ok(f) = http.json::<GifFavoritesResponse>("/gifs/favorites").await {
        let mut a = app.borrow_mut();
        if a.epoch != epoch {
            return;
        }
        a.content.gif_favorites = f.ids;
        a.content.gif_favorites.truncate(MAX_GIF_FAVORITES);
        a.touch();
    }
    match capability {
        GifCapability::Enabled(GifConfig::Proxy { .. }) => {
            if let Ok(c) = http.json::<GifCategoriesResponse>("/gifs/categories").await {
                let mut a = app.borrow_mut();
                if a.epoch == epoch {
                    a.content.gif_categories = c.items.into_iter().take(48).collect();
                    a.touch();
                }
            }
        }
        // Without the proxy there are no categories: trending is the landing.
        GifCapability::Enabled(GifConfig::Direct { .. }) => {
            super::operations::gif_search(app, String::new(), epoch).await
        }
        GifCapability::Disabled => {}
    }
}

/// Your favorites, resolved to playable GIFs (the newest 48).
async fn gif_favorites(app: Shared, epoch: u64) {
    let (http, ids, capability) = {
        let mut a = app.borrow_mut();
        a.content.gif_busy = true;
        a.touch();
        (
            a.http(),
            a.content.gif_favorites.clone(),
            a.gif_capability.clone(),
        )
    };
    let mut items = vec![];
    for chunk in ids.chunks(GIF_PAGE_SIZE).take(2) {
        let page = match &capability {
            GifCapability::Enabled(GifConfig::Proxy { .. }) => http
                .json::<GifPage>(&format!("/gifs/items?ids={}", chunk.join(",")))
                .await
                .ok(),
            GifCapability::Enabled(GifConfig::Direct {
                public_client_key, ..
            }) => wasm_bindgen_futures::JsFuture::from(direct_items(
                public_client_key,
                &chunk.join(","),
            ))
            .await
            .ok()
            .and_then(|v| v.as_string())
            .and_then(|s| serde_json::from_str::<GifPage>(&s).ok()),
            GifCapability::Disabled => None,
        };
        items.extend(page.map(|p| p.items).unwrap_or_default());
    }
    let mut a = app.borrow_mut();
    if a.epoch != epoch {
        return;
    }
    // Newest first, as starred.
    items.sort_by_key(|g| ids.iter().position(|id| *id == g.id));
    a.content.favorite_items = items;
    a.content.gif_busy = false;
    a.touch();
}

#[wasm_bindgen(raw_module = "/public/live.js")]
extern "C" {
    #[wasm_bindgen(js_name = directItems)]
    fn direct_items(key: &str, ids: &str) -> js_sys::Promise;
}

pub(super) fn send_gif(app: &Shared, gif_id: &str, url: String) {
    let mut a = app.borrow_mut();
    let Some(channel_id) = a.channel else { return };
    a.dialog.clear();
    a.command(Command::Content(ContentCommand::SendGif {
        channel_id,
        gif_id: gif_id.into(),
        url,
    }));
}

async fn voters(app: Shared, m: Id, answer: u8, more: bool) {
    let (http, channel, after, epoch) = {
        let a = app.borrow();
        let after = a
            .content
            .voters
            .as_ref()
            .filter(|_| more)
            .and_then(|(_, _, users, _)| users.last().map(|u| u.id));
        (a.http(), a.channel, after, a.epoch)
    };
    let Some(ch) = channel else { return };
    let route = format!(
        "/channels/{}/messages/{}/poll/answers/{answer}/voters{}",
        ch.0,
        m.0,
        after.map_or(String::new(), |id| format!("?after={}", id.0))
    );
    let page = http.json::<PollVotersPage>(&route).await;
    let mut a = app.borrow_mut();
    if a.epoch != epoch {
        return;
    }
    match page {
        Ok(page) => {
            let mut users = if more {
                a.content
                    .voters
                    .take()
                    .map(|(_, _, users, _)| users)
                    .unwrap_or_default()
            } else {
                vec![]
            };
            users.extend(page.users);
            users.truncate(500);
            a.content.voters = Some((m, answer, users, page.has_more));
        }
        Err(e) => a.error = e,
    }
    a.touch();
}

/// Local time for a timestamp, as "Tue, Nov 24 at 9:00 AM".
pub(super) fn when(ms: u64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(ms as f64));
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let h = d.get_hours();
    format!(
        "{}, {} {} at {}:{:02} {}",
        DAYS[d.get_day() as usize % 7],
        MONTHS[d.get_month() as usize % 12],
        d.get_date(),
        if h.is_multiple_of(12) { 12 } else { h % 12 },
        d.get_minutes(),
        if h < 12 { "AM" } else { "PM" }
    )
}
/// `datetime-local` value for a timestamp (local time, minutes).
pub(super) fn local_input(ms: u64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(ms as f64));
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date(),
        d.get_hours(),
        d.get_minutes()
    )
}
fn parse_local(value: &str) -> Option<u64> {
    if value.len() < 16 {
        return None;
    }
    let t = js_sys::Date::new(&JsValue::from_str(value)).get_time();
    (t.is_finite() && t > 0.).then_some(t as u64)
}
/// Quick choices for "send later": in an hour, tomorrow at 9, next Monday at 9.
pub(super) fn presets() -> Vec<(&'static str, u64)> {
    let now = Browser.unix_ms();
    let at_nine = |days: u32| {
        let d = js_sys::Date::new(&JsValue::from_f64(now as f64));
        d.set_date(d.get_date() + days);
        d.set_hours(9);
        d.set_minutes(0);
        d.set_seconds(0);
        d.set_milliseconds(0);
        d.get_time() as u64
    };
    let today = js_sys::Date::new(&JsValue::from_f64(now as f64)).get_day();
    let monday = (8 - today) % 7;
    vec![
        ("In 1 hour", now + 3_600_000),
        ("Tomorrow at 9:00 AM", at_nine(1)),
        (
            "Monday at 9:00 AM",
            at_nine(if monday == 0 { 7 } else { monday }),
        ),
    ]
}

/// The composer's text, with mentions written as the server expects.
fn composer_text(a: &App) -> (String, Vec<Id>) {
    let (text, mut ids) = super::operations::mentions(&a.model, &value("live-input"));
    ids.truncate(MAX_MESSAGE_MENTIONS);
    (text, ids)
}

pub(super) fn action(app: Shared, kind: &str, arg: &str) {
    let pair = |arg: &str| {
        arg.split_once(',')
            .and_then(|(m, i)| Some((id(m)?, i.parse::<u8>().ok()?)))
    };
    match kind {
        "poll-vote" => {
            let mut a = app.borrow_mut();
            let (Some(ch), Some((m, answer))) = (a.channel, pair(arg)) else {
                return;
            };
            let Some(poll) = a
                .model
                .messages
                .get(&ch)
                .and_then(|ms| ms.get(&m))
                .and_then(|m| m.extra.poll.clone())
            else {
                return;
            };
            let mut answers = poll.my_votes.clone();
            if answers.contains(&answer) {
                answers.retain(|v| *v != answer);
            } else if poll.multiple {
                answers.push(answer);
            } else {
                answers = vec![answer];
            }
            a.command(Command::Content(ContentCommand::VotePoll {
                channel_id: ch,
                message_id: m,
                answers,
            }));
        }
        "poll-end" => {
            let mut a = app.borrow_mut();
            if let (Some(ch), Some(m)) = (a.channel, id(arg)) {
                a.command(Command::Content(ContentCommand::EndPoll {
                    channel_id: ch,
                    message_id: m,
                }));
            }
        }
        "poll-voters" | "poll-voters-more" => {
            let Some((m, answer)) = pair(arg) else { return };
            let more = kind == "poll-voters-more";
            if !more {
                app.borrow_mut().content.voters = None;
                super::operations::action(app.clone(), &format!("dialog:poll-voters:{arg}"));
            }
            spawn_local(async move { voters(app, m, answer, more).await });
        }
        "poll-answer-add" | "poll-answer-remove" => {
            let mut a = app.borrow_mut();
            let n = a.content.poll_answers.max(MIN_POLL_ANSWERS);
            a.content.poll_answers = if kind == "poll-answer-add" {
                (n + 1).min(MAX_POLL_ANSWERS)
            } else {
                // Move the later answers up into the removed row's place.
                if let Ok(i) = arg.parse::<usize>() {
                    for j in i..n.saturating_sub(1) {
                        for field in ["poll-answer", "poll-emoji"] {
                            let next = value(&format!("{field}-{}", j + 1));
                            set_value(&format!("{field}-{j}"), &next);
                        }
                    }
                    for field in ["poll-answer", "poll-emoji"] {
                        set_value(&format!("{field}-{}", n - 1), "");
                    }
                }
                n.saturating_sub(1).max(MIN_POLL_ANSWERS)
            };
            a.touch();
        }
        "forward" => {
            if id(arg).is_some() {
                super::operations::action(app, &format!("dialog:forward:{arg}"));
            }
        }
        "forward-pick" => {
            let mut a = app.borrow_mut();
            if let Some(ch) = id(arg) {
                let picked = &mut a.content.forward_selected;
                if picked.contains(&ch) {
                    picked.retain(|v| *v != ch);
                } else if picked.len() < MAX_FORWARD_DESTINATIONS {
                    picked.push(ch);
                } else {
                    a.error = "You can forward to up to 5 conversations at once.".into();
                }
            }
            a.touch();
        }
        "forward-query" => {
            let mut a = app.borrow_mut();
            a.content.forward_query = arg.chars().take(100).collect();
            a.touch();
        }
        "embeds-remove" => {
            let mut a = app.borrow_mut();
            if let (Some(ch), Some(m)) = (a.channel, id(arg)) {
                a.command(Command::Content(ContentCommand::RemoveEmbeds {
                    channel_id: ch,
                    message_id: m,
                }));
            }
        }
        "schedule-at" => {
            if let Ok(at) = arg.parse::<u64>() {
                schedule(&app, at);
            }
        }
        "scheduled-send" | "scheduled-cancel" => {
            let mut a = app.borrow_mut();
            if let Some(s) = id(arg) {
                a.command(Command::Content(if kind == "scheduled-send" {
                    ContentCommand::SendScheduledNow { id: s }
                } else {
                    ContentCommand::CancelScheduled { id: s }
                }));
            }
        }
        "attachment-spoiler" => {
            let mut a = app.borrow_mut();
            if let Some(f) = id(arg) {
                let meta = a.content.meta.entry(f).or_default();
                meta.spoiler = !meta.spoiler;
            }
            a.touch();
        }
        "spoiler-reveal" | "alt-toggle" | "poll-results" => {
            let mut a = app.borrow_mut();
            if let Some(f) = id(arg) {
                let set = match kind {
                    "spoiler-reveal" => &mut a.content.revealed,
                    "alt-toggle" => &mut a.content.alt_shown,
                    _ => &mut a.content.results_shown,
                };
                if !set.remove(&f) || kind == "spoiler-reveal" {
                    set.insert(f);
                }
                while set.len() > 256 {
                    set.pop_first();
                }
            }
            a.touch();
        }
        "voice-upload" => voice_upload(app),
        "jump-message" => {
            let target = arg
                .split_once(',')
                .and_then(|(c, m)| Some((id(c)?, id(m)?)));
            if let Some((c, m)) =
                target.filter(|(c, _)| app.borrow().model.channels.contains_key(c))
            {
                app.borrow_mut().dialog.clear();
                super::operations::load_history(app, c, None, None, Some(m));
            } else if let Some((c, m)) = target {
                super::threads::jump(app, c, m);
            }
        }
        "voice-error" => {
            let mut a = app.borrow_mut();
            a.error = arg.chars().take(200).collect();
            a.touch();
        }
        "gif-tab" => {
            let epoch = app.borrow().epoch;
            {
                let mut a = app.borrow_mut();
                a.content.gif_tab = if arg == "favorites" {
                    arg.into()
                } else {
                    String::new()
                };
                a.gifs.clear();
                a.touch();
            }
            if arg == "favorites" {
                spawn_local(async move { gif_favorites(app, epoch).await });
            } else {
                spawn_local(async move { gif_landing(app, epoch).await });
            }
        }
        "gif-category" => {
            app.borrow_mut().content.gif_tab = "search".into();
            set_value("gif-query", arg);
            let (query, epoch) = (arg.to_owned(), app.borrow().epoch);
            spawn_local(async move { super::operations::gif_search(app, query, epoch).await });
        }
        "gif-star" => {
            let gif = arg.to_owned();
            let (http, starred) = {
                let a = app.borrow();
                (a.http(), a.content.gif_favorites.contains(&gif))
            };
            if !starred && app.borrow().content.gif_favorites.len() >= MAX_GIF_FAVORITES {
                let mut a = app.borrow_mut();
                a.error = "You can keep up to 200 favorite GIFs. Remove one first.".into();
                a.touch();
                return;
            }
            spawn_local(async move {
                let route = format!("/api/v1/gifs/favorites/{gif}");
                let result = http
                    .execute(
                        if starred { "DELETE" } else { "PUT" },
                        &route,
                        Body::Empty,
                        vec![],
                        15_000,
                    )
                    .await;
                let mut a = app.borrow_mut();
                match result {
                    Ok(r) if (200..300).contains(&r.status) => {
                        let list = &mut a.content.gif_favorites;
                        list.retain(|v| *v != gif);
                        if starred {
                            a.content.favorite_items.retain(|g| g.id != gif);
                            notice(&mut a, "Removed from favorites.");
                        } else {
                            list.insert(0, gif);
                            notice(&mut a, "Added to favorites.");
                        }
                    }
                    _ => a.error = "Could not update your favorites. Try again.".into(),
                }
                a.touch();
            });
        }
        _ => {}
    }
}

fn set_value(id: &str, text: &str) {
    if let Some(e) = document().get_element_by_id(id) {
        if let Some(e) = e.dyn_ref::<web_sys::HtmlInputElement>() {
            e.set_value(text);
        } else if let Some(e) = e.dyn_ref::<web_sys::HtmlTextAreaElement>() {
            e.set_value(text);
        }
    }
}

fn schedule(app: &Shared, at: u64) {
    let mut a = app.borrow_mut();
    let Some(channel_id) = a.channel else { return };
    let (content, mention_ids) = composer_text(&a);
    if content.trim().is_empty() {
        a.error = "Write a message to schedule first.".into();
        a.touch();
        return;
    }
    a.command(Command::Content(ContentCommand::ScheduleMessage {
        channel_id,
        content,
        mention_ids,
        send_at: at,
    }));
}

/// The recording is in `#live-voice-file`, with its length and waveform.
fn voice_upload(app: Shared) {
    let Some(input) = document()
        .get_element_by_id("live-voice-file")
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    else {
        return;
    };
    let Some(file) = input.files().and_then(|f| f.get(0)) else {
        return;
    };
    let data = input.dataset();
    let duration_ms = data
        .get("duration")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| (1..=MAX_VOICE_MESSAGE_MS).contains(v));
    let waveform: Vec<u8> = data
        .get("waveform")
        .unwrap_or_default()
        .split(',')
        .filter_map(|v| v.parse().ok())
        .take(MAX_VOICE_WAVEFORM_SAMPLES)
        .collect();
    let (Some(duration_ms), false) = (duration_ms, waveform.is_empty()) else {
        return;
    };
    let (channel, guild, epoch) = {
        let mut a = app.borrow_mut();
        if a.uploading {
            a.error = "Wait for the upload to finish, then record again.".into();
            a.touch();
            return;
        }
        a.content.voice = Some((
            file.name(),
            AttachmentMeta {
                duration_ms: Some(duration_ms),
                waveform,
                ..Default::default()
            },
        ));
        (a.channel, a.guild, a.epoch)
    };
    spawn_local(async move {
        super::operations::upload(app, UploadPurpose::Message, file, channel, guild, epoch).await;
    });
}

pub(super) fn submit(app: Shared, form: &str) {
    let mut a = app.borrow_mut();
    match form {
        "poll" => {
            let Some(channel_id) = a.channel else { return };
            let n = a
                .content
                .poll_answers
                .clamp(MIN_POLL_ANSWERS, MAX_POLL_ANSWERS);
            let answers: Vec<PollAnswerDraft> = (0..n)
                .map(|i| PollAnswerDraft {
                    text: value(&format!("poll-answer-{i}")).trim().to_owned(),
                    emoji: Some(value(&format!("poll-emoji-{i}")).trim().to_owned())
                        .filter(|e| !e.is_empty()),
                })
                .filter(|v| !v.text.is_empty())
                .collect();
            let question = value("poll-question").trim().to_owned();
            if question.is_empty() || answers.len() < MIN_POLL_ANSWERS {
                a.error = "Add a question and at least two answers.".into();
                a.touch();
                return;
            }
            a.command(Command::Content(ContentCommand::CreatePoll(PollDraft {
                channel_id,
                question,
                answers,
                multiple: checked("poll-multiple"),
                duration_hours: value("poll-duration").parse().unwrap_or(24),
            })));
        }
        "forward" => {
            let Some((ch, m)) = a
                .dialog
                .strip_prefix("forward:")
                .and_then(id)
                .and_then(|m| a.channel.map(|ch| (ch, m)))
            else {
                return;
            };
            if a.content.forward_selected.is_empty() {
                a.error = "Choose where to forward it.".into();
                a.touch();
                return;
            }
            let destinations = a.content.forward_selected.clone();
            a.command(Command::Content(ContentCommand::Forward {
                channel_id: ch,
                message_id: m,
                destinations,
                comment: value("forward-comment"),
            }));
        }
        "schedule" => {
            let Some(at) = parse_local(&value("schedule-at")) else {
                a.error = "Choose a date and time.".into();
                a.touch();
                return;
            };
            drop(a);
            schedule(&app, at);
        }
        "scheduled-edit" => {
            let Some(s) = a.dialog.strip_prefix("scheduled-edit:").and_then(id) else {
                return;
            };
            let Some(at) = parse_local(&value("scheduled-at")) else {
                a.error = "Choose a date and time.".into();
                a.touch();
                return;
            };
            let content = value("scheduled-text");
            let mention_ids = a
                .content
                .scheduled
                .iter()
                .find(|v| v.id == s)
                .map(|v| v.mention_ids.clone())
                .unwrap_or_default();
            a.command(Command::Content(ContentCommand::UpdateScheduled {
                id: s,
                content,
                mention_ids,
                send_at: at,
            }));
        }
        "attachment-meta" => {
            let Some(f) = a.dialog.strip_prefix("attachment:").and_then(id) else {
                return;
            };
            let description = value("attachment-description").trim().to_owned();
            if description.chars().count() > MAX_ATTACHMENT_DESCRIPTION_CHARS {
                a.error = "Descriptions can be up to 1,024 characters.".into();
                a.touch();
                return;
            }
            let meta = a.content.meta.entry(f).or_default();
            meta.description = Some(description).filter(|d| !d.is_empty());
            meta.spoiler = checked("attachment-spoiler");
            a.dialog.clear();
            a.touch();
        }
        _ => {}
    }
}
