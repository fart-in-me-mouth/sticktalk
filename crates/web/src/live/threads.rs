//! Threads, forum channels and announcement channels in the web app.
//! Views are in `ui/threads.rs`. Every action here is named `thread-*`,
//! `threads-*`, `forum-*` or is one of the announcement actions below.
//!
//! A thread is a channel the client keeps in `model.channels` (kind `Thread`,
//! nested in its parent) while it knows about it, so the ordinary chat,
//! history, composer, reactions and pins work in it unchanged. The sidebar,
//! channel pickers and the tree leave threads out; `ui/threads.rs` lists the
//! threads you joined under their channels.
use super::*;
use sticktalk_client_core::launch::permissions as channel_rights;

/// Threads kept at once (joined ones, opened ones and listed ones).
const MAX_KNOWN: usize = 2000;

#[derive(Default)]
pub(super) struct Browser {
    pub parent: Option<Id>,
    pub archived: bool,
    pub threads: Vec<ThreadInfo>,
    pub has_more: bool,
    pub next: Option<u64>,
    pub busy: bool,
}

#[derive(Default)]
pub(super) struct Forum {
    pub channel: Option<Id>,
    pub settings: Option<ForumSettings>,
    pub posts: Vec<ForumPost>,
    pub users: BTreeMap<Id, UserSummary>,
    pub has_more: bool,
    pub next: Option<u64>,
    pub busy: bool,
    /// A newer search, filter or order replaces a page still loading.
    pub generation: u64,
    /// None: the forum's default order.
    pub sort: Option<ForumSort>,
    pub tags: Vec<u32>,
    pub query: String,
    /// None: the forum's default layout.
    pub layout: Option<ForumLayout>,
}

/// Everything this module keeps between renders. Every list is bounded.
#[derive(Default)]
pub(super) struct State {
    pub threads: BTreeMap<Id, ThreadInfo>,
    pub joined: BTreeMap<Id, ThreadMembership>,
    pub browser: Browser,
    pub forum: Forum,
    /// Thread members dialog: (thread, members, more to load).
    pub members: Option<(Id, Vec<ThreadMember>, bool)>,
    /// Follows dialog: (channel, followings).
    pub follows: Option<(Id, Vec<ChannelFollow>)>,
    /// The thread being started: (parent, from this message).
    pub creating: Option<(Id, Option<Id>)>,
}

pub(super) const ACTIONS: &[&str] = &[
    "thread-open",
    "thread-new",
    "thread-join",
    "thread-leave",
    "thread-archive",
    "thread-unarchive",
    "thread-lock",
    "thread-unlock",
    "thread-delete",
    "thread-notify",
    "thread-members",
    "thread-members-more",
    "thread-remove-member",
    "thread-parent",
    "threads-browser",
    "threads-tab",
    "threads-more",
    "forum-sort",
    "forum-tag",
    "forum-layout",
    "forum-more",
    "forum-react",
    "forum-pin",
    "forum-new",
    "publish",
    "follow",
    "follows",
    "unfollow",
];
pub(super) const FORMS: &[&str] = &[
    "thread-new",
    "thread-rename",
    "thread-add-member",
    "forum-post",
    "forum-search",
    "forum-settings",
    "follow",
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

/// The thread, if the client knows it.
pub(super) fn thread(a: &App, t: Id) -> Option<&ThreadInfo> {
    a.threads.threads.get(&t)
}
pub(super) fn joined(a: &App, t: Id) -> bool {
    a.threads.joined.contains_key(&t)
}

/// Your rights in a channel or thread, for showing the right buttons. The
/// server decides; this only mirrors its rules.
pub(super) fn rights(a: &App, ch: Id) -> Permissions {
    let mut p = channel_rights(&a.model, ch);
    if let Some(t) = thread(a, ch) {
        let manage = p.contains(Permissions::MANAGE_THREADS);
        let me = a.model.me.as_ref().map(|m| m.user.id);
        let first = t.forum_post && t.message_count == 0 && Some(t.owner_id) == me;
        let send = if first {
            p.contains(Permissions::SEND_MESSAGES)
        } else {
            p.contains(Permissions::SEND_MESSAGES_IN_THREADS)
        } && (!t.locked || manage);
        p.set(Permissions::SEND_MESSAGES, send);
    }
    p
}
pub(super) fn manage(a: &App, ch: Id) -> bool {
    rights(a, ch).contains(Permissions::MANAGE_THREADS)
}

/// Keeps a thread, and the channel row that lets ordinary chat code work in it.
fn remember(a: &mut App, t: ThreadInfo) {
    a.model.channels.insert(t.id, t.summary());
    a.threads.threads.insert(t.id, t);
    while a.threads.threads.len() > MAX_KNOWN {
        // Forget the least recently active thread that is neither joined nor open.
        let old = a
            .threads
            .threads
            .values()
            .filter(|t| !a.threads.joined.contains_key(&t.id) && a.channel != Some(t.id))
            .min_by_key(|t| t.last_activity_at)
            .map(|t| t.id);
        let Some(old) = old else { break };
        forget(a, old);
    }
}
fn forget(a: &mut App, t: Id) {
    a.threads.threads.remove(&t);
    a.threads.joined.remove(&t);
    a.model.purge_channel(t);
}

/// The starter message (in its parent) shows the thread's latest numbers.
fn preview_on_starter(a: &mut App, t: &ThreadInfo, gone: bool) {
    let Some(m) = t.starter_message_id else {
        return;
    };
    if let Some(msg) = a
        .model
        .messages
        .get_mut(&t.parent_id)
        .and_then(|ms| ms.get_mut(&m))
    {
        let preview = (!gone).then(|| ThreadPreview {
            thread_id: t.id,
            name: t.name.clone(),
            message_count: t.message_count,
            last_activity_at: t.last_activity_at,
            archived: t.archived,
            locked: t.locked,
            private: t.private,
        });
        msg.extra.set_threads(|x| x.thread = preview);
        a.model.layout_revision = a.model.layout_revision.wrapping_add(1);
    }
}

fn later(a: &App, f: impl FnOnce(Shared) + 'static) {
    let owner = a.owner.clone();
    spawn_local(async move {
        if let Some(app) = owner.upgrade() {
            f(app);
        }
    });
}

pub(super) fn event(a: &mut App, e: ThreadEvent) {
    match e {
        ThreadEvent::ThreadUpsert(t) => {
            preview_on_starter(a, &t, false);
            if a.threads.forum.channel == Some(t.parent_id) {
                if let Some(p) = a
                    .threads
                    .forum
                    .posts
                    .iter_mut()
                    .find(|p| p.thread.id == t.id)
                {
                    p.thread = t.clone();
                } else if t.forum_post && a.threads.forum.posts.len() < 200 {
                    let owner = t.owner_id;
                    if let Some(u) = a.model.users.get(&owner).cloned() {
                        a.threads.forum.users.insert(owner, u);
                    }
                    a.threads.forum.posts.insert(
                        a.threads
                            .forum
                            .posts
                            .iter()
                            .take_while(|p| p.thread.pinned)
                            .count(),
                        ForumPost {
                            thread: t.clone(),
                            first: None,
                            reaction_count: 0,
                        },
                    );
                }
            }
            if let Some(row) = a.threads.browser.threads.iter_mut().find(|x| x.id == t.id) {
                *row = t.clone();
            }
            remember(a, t);
        }
        ThreadEvent::ThreadRemoved {
            thread_id,
            parent_id,
        } => {
            if let Some(t) = a.threads.threads.get(&thread_id).cloned() {
                preview_on_starter(a, &t, true);
            }
            a.threads.forum.posts.retain(|p| p.thread.id != thread_id);
            a.threads.browser.threads.retain(|t| t.id != thread_id);
            if a.channel == Some(thread_id) {
                a.error = "That thread is gone or no longer available to you.".into();
                later(a, move |app| operations::load_channel(app, parent_id, None));
            }
            forget(a, thread_id);
        }
        ThreadEvent::Membership {
            thread_id,
            membership,
        } => match membership {
            Some(m) => {
                a.threads.joined.insert(thread_id, m);
            }
            None => {
                a.threads.joined.remove(&thread_id);
                a.model.reads.remove(&thread_id);
            }
        },
        ThreadEvent::ForumUpdated(s) => {
            if a.threads.forum.channel == Some(s.channel_id) {
                a.threads.forum.settings = Some(s);
            }
        }
        ThreadEvent::Published {
            channel_id,
            message_id,
        } => {
            if let Some(m) = a
                .model
                .messages
                .get_mut(&channel_id)
                .and_then(|ms| ms.get_mut(&message_id))
            {
                m.extra.set_threads(|x| x.published = true);
                a.model.layout_revision = a.model.layout_revision.wrapping_add(1);
            }
        }
        ThreadEvent::FollowsChanged { channel_id } => {
            if a.threads
                .follows
                .as_ref()
                .is_some_and(|(c, _)| *c == channel_id)
            {
                later(a, move |app| {
                    spawn_local(async move { load_follows(app, channel_id).await });
                });
            }
        }
    }
}

/// After each fresh snapshot: keep the threads we know in the model (Ready
/// replaced its channels) and subscribe, which answers with your threads.
pub(super) fn ready(app: Shared) {
    let mut a = app.borrow_mut();
    let known: Vec<ChannelSummary> = a
        .threads
        .threads
        .values()
        .map(ThreadInfo::summary)
        .collect();
    for s in known {
        a.model.channels.insert(s.id, s);
    }
    a.command(Command::Threads(ThreadCommand::Subscribe));
}

pub(super) fn refused(cmd: &ThreadCommand, code: ErrorCode) -> Option<&'static str> {
    Some(match (cmd, code) {
        (_, ErrorCode::LimitExceeded) => match cmd {
            ThreadCommand::Follow { .. } => {
                "That channel has as many followers, or this channel follows as many channels, as it can."
            }
            _ => "This server has as many active threads as it can. Archive some first.",
        },
        (ThreadCommand::Follow { .. }, ErrorCode::Conflict) => "This channel already follows it.",
        (ThreadCommand::Follow { .. }, ErrorCode::Forbidden | ErrorCode::NotFound) => {
            "You need Manage Server in that server to follow into it."
        }
        (ThreadCommand::Publish { .. }, ErrorCode::Conflict) => "That message is already published.",
        (ThreadCommand::Publish { .. }, ErrorCode::Forbidden) => {
            "You can publish your own messages with Publish Messages, or anyone’s with Manage Messages."
        }
        (ThreadCommand::Create(_), ErrorCode::Conflict) => "That message already has a thread.",
        (ThreadCommand::Create(_) | ThreadCommand::CreatePost(_), ErrorCode::Forbidden) => {
            "You don’t have permission to start one here."
        }
        (ThreadCommand::CreatePost(_), ErrorCode::InvalidRequest) => {
            "Add a title and a message. This forum may require a tag."
        }
        (_, ErrorCode::Forbidden) => "Only the thread’s owner or a moderator can do that.",
        _ => return None,
    })
}

pub(super) fn answered(app: Shared, cmd: Option<&Command>, result: ThreadResult) {
    let mut a = app.borrow_mut();
    match result {
        ThreadResult::Snapshot(s) => {
            a.threads.joined = s
                .memberships
                .into_iter()
                .map(|m| (m.thread_id, m))
                .collect();
            for r in s.read_states {
                a.model.reads.insert(r.channel_id, r);
            }
            for t in s.threads {
                remember(&mut a, t);
            }
        }
        ThreadResult::Thread(t) => {
            let created = matches!(cmd, Some(Command::Threads(ThreadCommand::Create(_))));
            let id = t.id;
            remember(&mut a, *t);
            if created {
                a.dialog.clear();
                a.threads.creating = None;
                drop(a);
                operations::load_channel(app, id, None);
                return;
            }
            if matches!(a.dialog.as_str(), "thread-rename") {
                a.dialog.clear();
            }
        }
        ThreadResult::Post { thread, message } => {
            let id = thread.id;
            a.attachments
                .retain(|f| !message.attachments.iter().any(|m| m.id == f.id));
            remember(&mut a, *thread);
            a.dialog.clear();
            drop(a);
            operations::load_channel(app, id, None);
            return;
        }
        ThreadResult::Forum(s) => {
            if a.threads.forum.channel == Some(s.channel_id) {
                a.threads.forum.settings = Some(s);
            }
            a.dialog.clear();
            notice(&mut a, "Forum settings saved.");
        }
        ThreadResult::Follow(f) => {
            a.dialog.clear();
            let text = format!(
                "#{} now follows #{} from {}.",
                f.target_channel_name, f.source_channel_name, f.source_guild_name
            );
            notice(&mut a, &text);
        }
        ThreadResult::Ack => match cmd {
            Some(Command::Threads(ThreadCommand::Publish { .. })) => {
                notice(&mut a, "Published to every following channel.")
            }
            Some(Command::Threads(ThreadCommand::Delete { .. })) => a.dialog.clear(),
            Some(Command::Threads(ThreadCommand::AddMember { .. }))
                if a.dialog.starts_with("thread-members") =>
            {
                let t = a.threads.members.as_ref().map(|(t, ..)| *t);
                drop(a);
                if let Some(t) = t {
                    spawn_local(async move { load_members(app, t, false).await });
                }
                return;
            }
            _ => {}
        },
    }
    a.touch();
}

/// Opening a forum shows its posts instead of a message history. True when
/// `c` was handled here.
pub(super) fn open_special(app: &Shared, c: Id) -> bool {
    let kind = app.borrow().model.channels.get(&c).map(|c| c.kind);
    if kind != Some(ChannelKind::Forum) {
        return false;
    }
    {
        let mut a = app.borrow_mut();
        if a.channel != Some(c) {
            a.threads.forum = Forum {
                channel: Some(c),
                ..Default::default()
            };
        }
        a.channel = Some(c);
        a.guild = a
            .model
            .channels
            .get(&c)
            .and_then(|c| c.guild_id)
            .or(a.guild);
        a.panel.clear();
        a.reply = None;
        a.editing = None;
        a.history_busy = false;
        a.window_ids.clear();
        a.touch();
    }
    let app = app.clone();
    spawn_local(async move { load_forum(app, c, false).await });
    true
}

async fn load_forum(app: Shared, forum: Id, more: bool) {
    let (http, epoch, generation, path, known) = {
        let mut a = app.borrow_mut();
        let f = &mut a.threads.forum;
        if f.channel != Some(forum) || (more && (f.busy || !f.has_more)) {
            return;
        }
        f.busy = true;
        f.generation = f.generation.wrapping_add(1);
        let mut path = format!("/channels/{}/posts?limit=25", forum.0);
        if let Some(s) = f.sort {
            path.push_str(match s {
                ForumSort::Activity => "&sort=activity",
                ForumSort::Created => "&sort=created",
            });
        }
        if !f.tags.is_empty() {
            let tags: Vec<String> = f.tags.iter().map(u32::to_string).collect();
            path.push_str(&format!("&tags={}", tags.join(",")));
        }
        if !f.query.trim().is_empty() {
            path.push_str(&format!(
                "&q={}",
                js_sys::encode_uri_component(f.query.trim())
            ));
        }
        if let (true, Some(n)) = (more, f.next) {
            path.push_str(&format!("&before={n}"));
        }
        let known = a.threads.forum.settings.is_some();
        (a.http(), a.epoch, a.threads.forum.generation, path, known)
    };
    // Settings are fetched once per visit; `ForumUpdated` keeps them current.
    let settings = if more || known {
        None
    } else {
        http.json::<ForumSettings>(&format!("/channels/{}/forum", forum.0))
            .await
            .ok()
    };
    let page = http.json::<ForumPage>(&path).await;
    let mut a = app.borrow_mut();
    if a.epoch != epoch
        || a.threads.forum.channel != Some(forum)
        || a.threads.forum.generation != generation
    {
        return;
    }
    a.threads.forum.busy = false;
    match page {
        Ok(page) => {
            if let Some(s) = settings {
                a.threads.forum.settings = Some(s);
            }
            for u in page.users {
                a.model.users.entry(u.id).or_insert_with(|| u.clone());
                a.threads.forum.users.insert(u.id, u);
            }
            if !more {
                a.threads.forum.posts.clear();
            }
            for p in &page.posts {
                remember(&mut a, p.thread.clone());
            }
            a.threads.forum.posts.extend(page.posts);
            a.threads.forum.posts.truncate(200);
            a.threads.forum.has_more = page.has_more;
            a.threads.forum.next = page.next;
        }
        Err(e) => a.error = e,
    }
    a.touch();
}

async fn load_browser(app: Shared, more: bool) {
    let (http, epoch, parent, path) = {
        let mut a = app.borrow_mut();
        let b = &mut a.threads.browser;
        let Some(parent) = b.parent else { return };
        if b.busy || (more && !b.has_more) {
            return;
        }
        b.busy = true;
        let mut path = format!(
            "/channels/{}/threads?archived={}&limit=50",
            parent.0, b.archived
        );
        if let (true, Some(n)) = (more, b.next) {
            path.push_str(&format!("&before={n}"));
        }
        (a.http(), a.epoch, parent, path)
    };
    let page = http.json::<ThreadPage>(&path).await;
    let mut a = app.borrow_mut();
    if a.epoch != epoch || a.threads.browser.parent != Some(parent) {
        return;
    }
    a.threads.browser.busy = false;
    match page {
        Ok(page) => {
            if !more {
                a.threads.browser.threads.clear();
            }
            for t in &page.threads {
                remember(&mut a, t.clone());
            }
            a.threads.browser.threads.extend(page.threads);
            a.threads.browser.threads.truncate(500);
            a.threads.browser.has_more = page.has_more;
            a.threads.browser.next = page.next;
        }
        Err(e) => a.error = e,
    }
    a.touch();
}

async fn load_members(app: Shared, t: Id, more: bool) {
    let (http, epoch, after) = {
        let a = app.borrow();
        let after = match (&a.threads.members, more) {
            (Some((x, list, _)), true) if *x == t => list.last().map(|m| m.user.id.0),
            _ => None,
        };
        (a.http(), a.epoch, after)
    };
    let mut path = format!("/threads/{}/members", t.0);
    if let Some(after) = after {
        path.push_str(&format!("?after={after}"));
    }
    let page = http.json::<ThreadMembersPage>(&path).await;
    let mut a = app.borrow_mut();
    if a.epoch != epoch {
        return;
    }
    match page {
        Ok(page) => {
            let mut list = match a.threads.members.take() {
                Some((x, list, _)) if x == t && more => list,
                _ => vec![],
            };
            for m in &page.members {
                a.model
                    .users
                    .entry(m.user.id)
                    .or_insert_with(|| m.user.clone());
            }
            list.extend(page.members);
            list.truncate(1000);
            a.threads.members = Some((t, list, page.has_more));
        }
        Err(e) => a.error = e,
    }
    a.touch();
}

async fn load_follows(app: Shared, ch: Id) {
    let (http, epoch) = {
        let a = app.borrow();
        (a.http(), a.epoch)
    };
    let list = http
        .json::<ChannelFollows>(&format!("/channels/{}/follows", ch.0))
        .await;
    let mut a = app.borrow_mut();
    if a.epoch != epoch {
        return;
    }
    match list {
        Ok(list) => a.threads.follows = Some((ch, list.follows)),
        Err(_) => a.threads.follows = Some((ch, vec![])),
    }
    a.touch();
}

/// Opens a thread, fetching it first when the client does not know it yet.
pub(super) fn open(app: Shared, t: Id, message: Option<Id>) {
    if app.borrow().threads.threads.contains_key(&t) {
        app.borrow_mut().dialog.clear();
        operations::load_history(app, t, None, None, message);
        return;
    }
    let (http, epoch) = {
        let a = app.borrow();
        (a.http(), a.epoch)
    };
    spawn_local(async move {
        let detail = http
            .json::<ThreadDetail>(&format!("/threads/{}", t.0))
            .await;
        let mut a = app.borrow_mut();
        if a.epoch != epoch {
            return;
        }
        match detail {
            Ok(d) => {
                if let Some(m) = d.me {
                    a.threads.joined.insert(t, m);
                }
                if let Some(r) = d.read_state {
                    a.model.reads.insert(t, r);
                }
                remember(&mut a, d.thread);
                a.dialog.clear();
                drop(a);
                operations::load_history(app, t, None, None, message);
            }
            Err(e) => {
                a.error = e;
                a.touch();
            }
        }
    });
}

/// A search result or link to a message in a thread the client has not seen.
pub(super) fn jump(app: Shared, ch: Id, m: Id) -> bool {
    if app.borrow().model.channels.contains_key(&ch) {
        return false;
    }
    open(app, ch, Some(m));
    true
}

fn patch(a: &mut App, t: Id, patch: ThreadPatch) {
    a.command(Command::Threads(ThreadCommand::Update {
        thread_id: t,
        patch,
    }));
}

pub(super) fn action(app: Shared, kind: &str, arg: &str) {
    let target = id(arg.split(',').next().unwrap_or_default());
    match kind {
        "thread-open" => {
            if let Some(t) = target {
                open(app, t, None);
            }
        }
        "thread-parent" => {
            let parent =
                target.and_then(|t| app.borrow().threads.threads.get(&t).map(|t| t.parent_id));
            if let Some(p) = parent {
                operations::load_channel(app, p, None);
            }
        }
        "thread-new" => {
            let Some(parent) = target else { return };
            let message = arg.split(',').nth(1).and_then(id);
            app.borrow_mut().threads.creating = Some((parent, message));
            operations::action(app, "dialog:thread-new");
        }
        "thread-join" | "thread-leave" => {
            let Some(t) = target else { return };
            let cmd = if kind == "thread-join" {
                ThreadCommand::Join { thread_id: t }
            } else {
                ThreadCommand::Leave { thread_id: t }
            };
            app.borrow_mut().command(Command::Threads(cmd));
        }
        "thread-archive" | "thread-unarchive" => {
            let Some(t) = target else { return };
            patch(
                &mut app.borrow_mut(),
                t,
                ThreadPatch {
                    archived: Some(kind == "thread-archive"),
                    ..Default::default()
                },
            );
        }
        "thread-lock" | "thread-unlock" => {
            let Some(t) = target else { return };
            patch(
                &mut app.borrow_mut(),
                t,
                ThreadPatch {
                    locked: Some(kind == "thread-lock"),
                    ..Default::default()
                },
            );
        }
        "forum-pin" => {
            let Some(t) = target else { return };
            let pinned = app
                .borrow()
                .threads
                .threads
                .get(&t)
                .is_some_and(|t| t.pinned);
            patch(
                &mut app.borrow_mut(),
                t,
                ThreadPatch {
                    pinned: Some(!pinned),
                    ..Default::default()
                },
            );
        }
        "thread-delete" => {
            if let Some(t) = target {
                app.borrow_mut()
                    .command(Command::Threads(ThreadCommand::Delete { thread_id: t }));
            }
        }
        "thread-notify" => {
            let Some(t) = target else { return };
            let notify = match arg.split(',').nth(1) {
                Some("all") => ThreadNotify::All,
                Some("mentions") => ThreadNotify::Mentions,
                Some("nothing") => ThreadNotify::Nothing,
                _ => ThreadNotify::Default,
            };
            app.borrow_mut()
                .command(Command::Threads(ThreadCommand::SetNotify {
                    thread_id: t,
                    notify,
                }));
        }
        "thread-members" | "thread-members-more" => {
            let Some(t) = target else { return };
            let more = kind == "thread-members-more";
            if !more {
                app.borrow_mut().threads.members = None;
                operations::action(app.clone(), &format!("dialog:thread-members:{}", t.0));
            }
            spawn_local(async move { load_members(app, t, more).await });
        }
        "thread-remove-member" => {
            let mut parts = arg.split(',');
            let (Some(t), Some(u)) = (parts.next().and_then(id), parts.next().and_then(id)) else {
                return;
            };
            app.borrow_mut()
                .command(Command::Threads(ThreadCommand::RemoveMember {
                    thread_id: t,
                    user_id: u,
                }));
            if let Some((x, list, _)) = app.borrow_mut().threads.members.as_mut() {
                if *x == t {
                    list.retain(|m| m.user.id != u);
                }
            }
            app.borrow().touch();
        }
        "threads-browser" => {
            let Some(parent) = target else { return };
            {
                let mut a = app.borrow_mut();
                a.threads.browser = Browser {
                    parent: Some(parent),
                    ..Default::default()
                };
            }
            operations::action(app.clone(), "dialog:threads");
            spawn_local(async move { load_browser(app, false).await });
        }
        "threads-tab" => {
            {
                let mut a = app.borrow_mut();
                a.threads.browser.archived = arg == "archived";
                a.threads.browser.threads.clear();
                a.threads.browser.next = None;
                a.threads.browser.busy = false;
                a.touch();
            }
            spawn_local(async move { load_browser(app, false).await });
        }
        "threads-more" => spawn_local(async move { load_browser(app, true).await }),
        "forum-sort" | "forum-tag" | "forum-layout" => {
            let reload = {
                let mut a = app.borrow_mut();
                let f = &mut a.threads.forum;
                match kind {
                    "forum-sort" => {
                        f.sort = Some(if arg == "created" {
                            ForumSort::Created
                        } else {
                            ForumSort::Activity
                        });
                        true
                    }
                    "forum-tag" => {
                        if let Ok(tag) = arg.parse::<u32>() {
                            if f.tags.contains(&tag) {
                                f.tags.retain(|t| *t != tag);
                            } else if f.tags.len() < MAX_FORUM_TAGS {
                                f.tags.push(tag);
                            }
                        } else {
                            f.tags.clear();
                        }
                        true
                    }
                    _ => {
                        f.layout = Some(if arg == "gallery" {
                            ForumLayout::Gallery
                        } else {
                            ForumLayout::List
                        });
                        false
                    }
                }
            };
            let forum = app.borrow().threads.forum.channel;
            app.borrow().touch();
            if let (true, Some(forum)) = (reload, forum) {
                spawn_local(async move { load_forum(app, forum, false).await });
            }
        }
        "forum-more" => {
            let forum = app.borrow().threads.forum.channel;
            if let Some(forum) = forum {
                spawn_local(async move { load_forum(app, forum, true).await });
            }
        }
        "forum-new" => operations::action(app, "dialog:forum-post"),
        "forum-react" => {
            let mut parts = arg.split(',');
            let (Some(t), Some(m)) = (parts.next().and_then(id), parts.next().and_then(id)) else {
                return;
            };
            let emoji = {
                let a = app.borrow();
                a.threads
                    .forum
                    .settings
                    .as_ref()
                    .and_then(|s| s.default_reaction.clone())
            };
            if let Some(emoji) = emoji {
                app.borrow_mut().command(Command::AddReaction {
                    channel_id: t,
                    message_id: m,
                    emoji,
                });
                if let Some(p) = app
                    .borrow_mut()
                    .threads
                    .forum
                    .posts
                    .iter_mut()
                    .find(|p| p.thread.id == t)
                {
                    p.reaction_count += 1;
                }
                app.borrow().touch();
            }
        }
        "publish" => {
            let (ch, m) = {
                let a = app.borrow();
                (a.channel, target)
            };
            if let (Some(ch), Some(m)) = (ch, m) {
                app.borrow_mut()
                    .command(Command::Threads(ThreadCommand::Publish {
                        channel_id: ch,
                        message_id: m,
                    }));
            }
        }
        "follow" => {
            if let Some(ch) = target {
                operations::action(app, &format!("dialog:follow:{}", ch.0));
            }
        }
        "follows" => {
            let Some(ch) = target else { return };
            app.borrow_mut().threads.follows = None;
            operations::action(app.clone(), &format!("dialog:follows:{}", ch.0));
            spawn_local(async move { load_follows(app, ch).await });
        }
        "unfollow" => {
            if let Some(f) = target {
                app.borrow_mut()
                    .command(Command::Threads(ThreadCommand::Unfollow { follow_id: f }));
            }
        }
        _ => {}
    }
}

fn archive_choice() -> u32 {
    value("thread-archive")
        .parse()
        .ok()
        .filter(|m| THREAD_ARCHIVE_MINUTES.contains(m))
        .unwrap_or(1440)
}

pub(super) fn submit(app: Shared, form: &str) {
    let mut a = app.borrow_mut();
    match form {
        "thread-new" => {
            let Some((parent, message)) = a.threads.creating else {
                return;
            };
            let name = value("thread-name").trim().to_owned();
            if name.is_empty() {
                a.error = "Give the thread a name.".into();
                a.touch();
                return;
            }
            let private = message.is_none() && checked("thread-private");
            a.command(Command::Threads(ThreadCommand::Create(NewThread {
                parent_id: parent,
                name,
                message_id: message,
                private,
                invitable: !private || checked("thread-invitable"),
                auto_archive_minutes: archive_choice(),
            })));
        }
        "thread-rename" => {
            let Some(t) = a.channel.filter(|c| a.threads.threads.contains_key(c)) else {
                return;
            };
            let name = value("thread-name").trim().to_owned();
            let mut p = ThreadPatch {
                auto_archive_minutes: Some(archive_choice()),
                ..Default::default()
            };
            if !name.is_empty() {
                p.name = Some(name);
            }
            let info = a.threads.threads.get(&t).cloned();
            if let Some(info) = info.as_ref().filter(|t| t.forum_post) {
                let tags: Vec<u32> = a
                    .threads
                    .forum
                    .settings
                    .iter()
                    .flat_map(|s| s.tags.iter())
                    .filter(|tag| checked(&format!("post-tag-{}", tag.id)))
                    .map(|tag| tag.id)
                    .collect();
                if tags != info.tag_ids {
                    p.tag_ids = Some(tags);
                }
            }
            if info.as_ref().is_some_and(|t| t.private) {
                p.invitable = Some(checked("thread-invitable"));
            }
            patch(&mut a, t, p);
        }
        "thread-add-member" => {
            let Some((t, ..)) = a.threads.members.as_ref().map(|(t, ..)| (*t,)) else {
                return;
            };
            let Some(user) = id(&value("thread-member")) else {
                a.error = "Choose someone to add.".into();
                a.touch();
                return;
            };
            a.command(Command::Threads(ThreadCommand::AddMember {
                thread_id: t,
                user_id: user,
            }));
        }
        "forum-post" => {
            let Some(forum) = a.threads.forum.channel else {
                return;
            };
            let title = value("post-title").trim().to_owned();
            let content = value("post-content");
            if title.is_empty() || (content.trim().is_empty() && a.attachments.is_empty()) {
                a.error = "Add a title and a message.".into();
                a.touch();
                return;
            }
            let tag_ids: Vec<u32> = a
                .threads
                .forum
                .settings
                .iter()
                .flat_map(|s| s.tags.iter())
                .filter(|tag| checked(&format!("post-tag-{}", tag.id)))
                .map(|tag| tag.id)
                .collect();
            let attachment_ids: Vec<Id> = a.attachments.iter().map(|f| f.id).collect();
            let attachments: Vec<AttachmentMetaPatch> = attachment_ids
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
            a.command(Command::Threads(ThreadCommand::CreatePost(NewPost {
                forum_id: forum,
                title,
                content,
                attachment_ids,
                attachments,
                mention_ids: vec![],
                tag_ids,
                auto_archive_minutes: None,
            })));
        }
        "forum-search" => {
            a.threads.forum.query = value("forum-query").chars().take(200).collect();
            let forum = a.threads.forum.channel;
            drop(a);
            if let Some(forum) = forum {
                spawn_local(async move { load_forum(app, forum, false).await });
            }
        }
        "forum-settings" => {
            let Some(forum) = a.threads.forum.channel else {
                return;
            };
            let old = a.threads.forum.settings.clone().unwrap_or_default();
            let mut tags = vec![];
            for (i, t) in old.tags.iter().enumerate() {
                if checked(&format!("tag-remove-{i}")) {
                    continue;
                }
                tags.push(ForumTag {
                    id: t.id,
                    name: value(&format!("tag-name-{i}")).trim().to_owned(),
                    emoji: Some(value(&format!("tag-emoji-{i}")).trim().to_owned())
                        .filter(|e| !e.is_empty()),
                    moderated: checked(&format!("tag-moderated-{i}")),
                });
            }
            let new = value("tag-new").trim().to_owned();
            if !new.is_empty() {
                tags.push(ForumTag {
                    id: 0,
                    name: new,
                    emoji: Some(value("tag-new-emoji").trim().to_owned()).filter(|e| !e.is_empty()),
                    moderated: checked("tag-new-moderated"),
                });
            }
            let seconds = |field: &str| value(field).trim().parse::<u32>().unwrap_or(0);
            a.command(Command::Threads(ThreadCommand::UpdateForum {
                forum_id: forum,
                patch: ForumSettingsPatch {
                    guidelines: Some(value("forum-guidelines")),
                    tags: Some(tags),
                    require_tags: Some(checked("forum-require-tags")),
                    default_reaction: Some(value("forum-reaction").trim().to_owned()),
                    sort: Some(if value("forum-sort") == "created" {
                        ForumSort::Created
                    } else {
                        ForumSort::Activity
                    }),
                    layout: Some(if value("forum-layout") == "gallery" {
                        ForumLayout::Gallery
                    } else {
                        ForumLayout::List
                    }),
                    post_slowmode: Some(seconds("forum-post-slowmode")),
                    reply_slowmode: Some(seconds("forum-reply-slowmode")),
                    default_auto_archive_minutes: Some(archive_choice()),
                },
            }));
        }
        "follow" => {
            let Some(source) = a.dialog.strip_prefix("follow:").and_then(id) else {
                return;
            };
            let Some(target) = id(&value("follow-target")) else {
                a.error = "Choose a channel to follow into.".into();
                a.touch();
                return;
            };
            a.command(Command::Threads(ThreadCommand::Follow {
                source_channel_id: source,
                target_channel_id: target,
            }));
        }
        _ => {}
    }
}
