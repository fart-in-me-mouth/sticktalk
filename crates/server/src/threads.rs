//! Threads, forum posts and announcement channels.
//! Wire contracts: `sticktalk_proto::threads`. Behavior: `docs/features/threads.md`.
//!
//! The model: a thread is a channel row of kind `thread` nested in the channel
//! it belongs to (a text or announcement channel, or a forum for a forum post).
//! It has no permission overwrites of its own, so `permissions::evaluate`
//! gives it its parent's rights; `scope` then applies the thread rules
//! (private membership, Send Messages in Threads, locks). Because a thread is a
//! channel, every message feature works in it unchanged. Threads are never in
//! `db::channels`, so they never reach older clients as channels.
//!
//! Timing: one task (`spawn`) sleeps until the earliest auto-archive deadline
//! and is woken when a thread's deadline moves earlier or a crosspost delivery
//! is queued. With nothing to do it only waits.
mod announce;
mod forum;
#[cfg(test)]
pub(crate) use announce::deliver_queue as deliver_crossposts;
pub(crate) use announce::revalidate as revalidate_follows;
mod routes;
#[cfg(test)]
mod tests;

pub(crate) use routes::routes;

use crate::{
    db::*,
    error::{require, Error, Result},
    state::{now, App, Core},
};
use sqlx::{sqlite::SqliteRow, Row, SqliteConnection};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use sticktalk_proto::*;
pub(crate) type Deliveries = Vec<(Vec<Id>, Event)>;

/// A crosspost delivery was queued since the worker last looked.
static QUEUED: AtomicBool = AtomicBool::new(false);

pub(crate) struct Threads {
    wake: tokio::sync::Notify,
    /// The archive deadline the timer sleeps until (u64::MAX: none).
    armed: AtomicU64,
}
impl Default for Threads {
    fn default() -> Self {
        Self {
            wake: tokio::sync::Notify::new(),
            armed: AtomicU64::new(u64::MAX),
        }
    }
}

const THREAD_RIGHTS: Permissions = Permissions::CREATE_PUBLIC_THREADS
    .union(Permissions::CREATE_PRIVATE_THREADS)
    .union(Permissions::SEND_MESSAGES_IN_THREADS)
    .union(Permissions::MANAGE_THREADS)
    .union(Permissions::PUBLISH_MESSAGES);
/// What a private thread's non-members lose: everything about the channel.
const CHANNEL_RIGHTS: Permissions = Permissions::VIEW_CHANNEL
    .union(Permissions::SEND_MESSAGES)
    .union(Permissions::READ_HISTORY)
    .union(Permissions::ATTACH_FILES)
    .union(Permissions::ADD_REACTIONS)
    .union(Permissions::MANAGE_MESSAGES)
    .union(Permissions::EMBED_LINKS)
    .union(Permissions::MENTION_EVERYONE)
    .union(THREAD_RIGHTS);

/// Thread rules on top of a channel's computed rights (`permissions::evaluate`).
pub(crate) async fn scope(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    p: Permissions,
) -> Result<Permissions> {
    if !p.contains(Permissions::VIEW_CHANNEL) {
        return Ok(p - THREAD_RIGHTS);
    }
    let Some(t) = ThreadScope::load(c, ch).await? else {
        return Ok(p);
    };
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM thread_members WHERE thread_id=? AND user_id=?)",
    )
    .bind(id(ch)?)
    .bind(id(u)?)
    .fetch_one(&mut *c)
    .await?;
    Ok(t.apply(u, member, p))
}
/// What the thread rules read about one thread. The batched evaluator
/// (`capacity::members_with`) loads it once per thread and applies the same
/// rules in memory, so a channel's audience honors private membership and locks.
pub(crate) struct ThreadScope {
    private: bool,
    locked: bool,
    forum_post: bool,
    owner: Id,
    message_count: i64,
}
impl ThreadScope {
    pub(crate) async fn load(c: &mut SqliteConnection, ch: Id) -> Result<Option<Self>> {
        let r = sqlx::query(
            "SELECT private,locked,forum_post,owner_id,message_count FROM threads WHERE channel_id=?",
        )
        .bind(id(ch)?)
        .fetch_optional(&mut *c)
        .await?;
        Ok(r.map(|r| Self {
            private: r.get::<i64, _>("private") != 0,
            locked: r.get::<i64, _>("locked") != 0,
            forum_post: r.get::<i64, _>("forum_post") != 0,
            owner: Id(r.get::<i64, _>("owner_id") as u64),
            message_count: r.get("message_count"),
        }))
    }
    /// The members of a thread (for the batched evaluator).
    pub(crate) async fn members(
        c: &mut SqliteConnection,
        ch: Id,
    ) -> Result<std::collections::HashSet<Id>> {
        Ok(
            sqlx::query_scalar::<_, i64>("SELECT user_id FROM thread_members WHERE thread_id=?")
                .bind(id(ch)?)
                .fetch_all(&mut *c)
                .await?
                .into_iter()
                .map(|v| Id(v as u64))
                .collect(),
        )
    }
    /// The thread rules on rights computed for its parent chain.
    pub(crate) fn apply(&self, u: Id, member: bool, mut p: Permissions) -> Permissions {
        if !p.contains(Permissions::VIEW_CHANNEL) {
            return p - THREAD_RIGHTS;
        }
        let manage = p.contains(Permissions::MANAGE_THREADS);
        if self.private && !member && !manage {
            return p - CHANNEL_RIGHTS;
        }
        // A forum post's first message is "creating the post" (Send Messages in
        // the forum); everything after it is a reply (Send Messages in Threads).
        let first = self.forum_post && self.message_count == 0 && self.owner == u;
        let send = if first {
            p.contains(Permissions::SEND_MESSAGES)
        } else {
            p.contains(Permissions::SEND_MESSAGES_IN_THREADS)
        } && (!self.locked || manage);
        p.set(Permissions::SEND_MESSAGES, send);
        if !send {
            p.remove(
                Permissions::ATTACH_FILES
                    | Permissions::EMBED_LINKS
                    | Permissions::MENTION_EVERYONE,
            );
        }
        if self.locked && !manage {
            p.remove(Permissions::ADD_REACTIONS);
        }
        // No threads inside threads.
        p.remove(
            Permissions::CREATE_PUBLIC_THREADS
                | Permissions::CREATE_PRIVATE_THREADS
                | Permissions::PUBLISH_MESSAGES,
        );
        p
    }
}
/// Outside a thread the rules only drop the thread rights of a channel the
/// person cannot see (as `scope` does).
pub(crate) fn unscoped(p: Permissions) -> Permissions {
    if p.contains(Permissions::VIEW_CHANNEL) {
        p
    } else {
        p - THREAD_RIGHTS
    }
}

const SELECT: &str = "SELECT t.*,ch.name FROM threads t JOIN channels ch ON ch.id=t.channel_id";

fn row(r: &SqliteRow) -> ThreadInfo {
    let opt_id = |name: &str| r.get::<Option<i64>, _>(name).map(|v| Id(v as u64));
    ThreadInfo {
        id: Id(r.get::<i64, _>("channel_id") as u64),
        guild_id: Id(r.get::<i64, _>("guild_id") as u64),
        parent_id: Id(r.get::<i64, _>("parent_id") as u64),
        name: r.get("name"),
        owner_id: Id(r.get::<i64, _>("owner_id") as u64),
        starter_message_id: opt_id("starter_message_id"),
        first_message_id: opt_id("first_message_id"),
        forum_post: r.get::<i64, _>("forum_post") != 0,
        private: r.get::<i64, _>("private") != 0,
        invitable: r.get::<i64, _>("invitable") != 0,
        archived: r.get::<i64, _>("archived") != 0,
        locked: r.get::<i64, _>("locked") != 0,
        pinned: r.get::<i64, _>("pinned") != 0,
        auto_archive_minutes: r.get::<i64, _>("auto_archive_minutes") as u32,
        created_at: r.get::<i64, _>("created_at").max(0) as u64,
        last_activity_at: r.get::<i64, _>("last_activity_at").max(0) as u64,
        archived_at: r
            .get::<Option<i64>, _>("archived_at")
            .map(|v| v.max(0) as u64),
        message_count: r.get::<i64, _>("message_count").max(0) as u32,
        member_count: r.get::<i64, _>("member_count").max(0) as u32,
        last_message_id: opt_id("last_message_id"),
        tag_ids: vec![],
    }
}

/// Rows from `SELECT` (or a query with the same columns), with their tags.
async fn rows(c: &mut SqliteConnection, rows: Vec<SqliteRow>) -> Result<Vec<ThreadInfo>> {
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let mut t = row(&r);
        if t.forum_post {
            t.tag_ids = sqlx::query_scalar::<_, i64>(
                "SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id",
            )
            .bind(id(t.id)?)
            .fetch_all(&mut *c)
            .await?
            .into_iter()
            .map(|v| v as u32)
            .collect();
        }
        out.push(t);
    }
    Ok(out)
}

/// The thread, or None when `ch` is not one. Never checks access.
pub(crate) async fn load(c: &mut SqliteConnection, ch: Id) -> Result<Option<ThreadInfo>> {
    let r = sqlx::query(sqlx::AssertSqlSafe(format!(
        "{SELECT} WHERE t.channel_id=?"
    )))
    .bind(id(ch)?)
    .fetch_optional(&mut *c)
    .await?;
    Ok(rows(c, r.into_iter().collect()).await?.pop())
}

/// The thread if `u` can see it; NotFound for anything else.
async fn visible(c: &mut SqliteConnection, u: Id, t: Id) -> Result<(ThreadInfo, Permissions)> {
    let info = load(c, t)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    access(c, u, t, Permissions::VIEW_CHANNEL).await?;
    let p = crate::permissions::evaluate(c, u, info.guild_id, Some(t)).await?;
    Ok((info, p))
}

async fn is_member(c: &mut SqliteConnection, t: Id, u: Id) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM thread_members WHERE thread_id=? AND user_id=?)",
    )
    .bind(id(t)?)
    .bind(id(u)?)
    .fetch_one(&mut *c)
    .await?)
}

async fn membership(c: &mut SqliteConnection, t: Id, u: Id) -> Result<Option<ThreadMembership>> {
    Ok(
        sqlx::query("SELECT joined_at,notify FROM thread_members WHERE thread_id=? AND user_id=?")
            .bind(id(t)?)
            .bind(id(u)?)
            .fetch_optional(&mut *c)
            .await?
            .map(|r| ThreadMembership {
                thread_id: t,
                joined_at: r.get::<i64, _>("joined_at").max(0) as u64,
                notify: en(r.get("notify")).unwrap_or_default(),
            }),
    )
}

fn thread_name(s: &str) -> Result<String> {
    let s = s.trim();
    require(
        !s.is_empty()
            && s.chars().count() <= MAX_THREAD_NAME_CHARS
            && !s.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )?;
    Ok(s.to_owned())
}
fn archive_minutes(m: u32) -> Result<u32> {
    require(
        THREAD_ARCHIVE_MINUTES.contains(&m),
        ErrorCode::InvalidRequest,
    )?;
    Ok(m)
}

/// At most `MAX_ACTIVE_THREADS_PER_GUILD` threads may be active at once.
async fn active_room(c: &mut SqliteConnection, g: Id) -> Result<()> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM threads WHERE guild_id=? AND archived=0")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    require(
        (n as usize) < MAX_ACTIVE_THREADS_PER_GUILD,
        ErrorCode::LimitExceeded,
    )
}

/// Tell everyone who can see the thread how it is now.
async fn announce(c: &mut SqliteConnection, out: &mut Deliveries, t: Id) -> Result<ThreadInfo> {
    let info = load(c, t)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let users = audience(c, t).await?;
    out.push((
        users,
        Event::Threads(ThreadEvent::ThreadUpsert(info.clone())),
    ));
    Ok(info)
}

/// Adds `v` as a member (caught up to now). Returns false if already one.
async fn join(c: &mut SqliteConnection, out: &mut Deliveries, t: Id, v: Id) -> Result<bool> {
    let added = sqlx::query(
        "INSERT OR IGNORE INTO thread_members(thread_id,user_id,joined_at) VALUES(?,?,?)",
    )
    .bind(id(t)?)
    .bind(id(v)?)
    .bind(now() as i64)
    .execute(&mut *c)
    .await?
    .rows_affected()
        > 0;
    if added {
        init_read(c, v, t, true).await?;
        out.push((
            vec![v],
            Event::Threads(ThreadEvent::Membership {
                thread_id: t,
                membership: membership(c, t, v).await?,
            }),
        ));
        out.push((vec![v], Event::ReadStateUpdated(read(c, v, t).await?)));
    }
    Ok(added)
}

async fn leave(c: &mut SqliteConnection, out: &mut Deliveries, t: Id, v: Id) -> Result<bool> {
    let removed = sqlx::query("DELETE FROM thread_members WHERE thread_id=? AND user_id=?")
        .bind(id(t)?)
        .bind(id(v)?)
        .execute(&mut *c)
        .await?
        .rows_affected()
        > 0;
    if removed {
        sqlx::query("DELETE FROM read_states WHERE user_id=? AND channel_id=?")
            .bind(id(v)?)
            .bind(id(t)?)
            .execute(&mut *c)
            .await?;
        out.push((
            vec![v],
            Event::Threads(ThreadEvent::Membership {
                thread_id: t,
                membership: None,
            }),
        ));
    }
    Ok(removed)
}

/// Dispatch for `Command::Threads`, inside the command transaction.
pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    cmd: ThreadCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    use ThreadCommand::*;
    let result = match cmd {
        // `commands::execute` marks the stream.
        Subscribe => ThreadResult::Snapshot(snapshot(c, u).await?),
        Create(new) => {
            core.rate(format!("thread:{}", u.0), 10., 5.)?;
            ThreadResult::Thread(Box::new(create(c, core, u, new, out).await?))
        }
        CreatePost(post) => {
            core.rate(format!("thread:{}", u.0), 10., 5.)?;
            return Box::pin(forum::create_post(c, core, key, u, post, out)).await;
        }
        Update { thread_id, patch } => {
            ThreadResult::Thread(Box::new(update(c, u, thread_id, patch, out).await?))
        }
        Delete { thread_id } => {
            let (info, p) = visible(c, u, thread_id).await?;
            require(
                p.contains(Permissions::MANAGE_THREADS),
                ErrorCode::Forbidden,
            )?;
            remove_thread(c, out, &info).await?;
            ThreadResult::Ack
        }
        Join { thread_id } => {
            let (info, _) = visible(c, u, thread_id).await?;
            if join(c, out, thread_id, u).await? {
                announce(c, out, info.id).await?;
            }
            ThreadResult::Ack
        }
        Leave { thread_id } => {
            let (info, p) = visible(c, u, thread_id).await?;
            if leave(c, out, thread_id, u).await? {
                if info.private && !p.contains(Permissions::MANAGE_THREADS) {
                    out.push(removed(&info, vec![u]));
                }
                announce(c, out, info.id).await?;
            }
            ThreadResult::Ack
        }
        AddMember { thread_id, user_id } => {
            add_member(c, u, thread_id, user_id, out).await?;
            ThreadResult::Ack
        }
        RemoveMember { thread_id, user_id } => {
            let (info, p) = visible(c, u, thread_id).await?;
            require(
                p.contains(Permissions::MANAGE_THREADS) || (info.private && info.owner_id == u),
                ErrorCode::Forbidden,
            )?;
            if leave(c, out, thread_id, user_id).await? {
                let sees = crate::permissions::evaluate(c, user_id, info.guild_id, Some(thread_id))
                    .await
                    .is_ok_and(|p| p.contains(Permissions::VIEW_CHANNEL));
                if !sees {
                    out.push(removed(&info, vec![user_id]));
                }
                announce(c, out, info.id).await?;
            }
            ThreadResult::Ack
        }
        SetNotify { thread_id, notify } => {
            visible(c, u, thread_id).await?;
            let n =
                sqlx::query("UPDATE thread_members SET notify=? WHERE thread_id=? AND user_id=?")
                    .bind(enum_str(notify))
                    .bind(id(thread_id)?)
                    .bind(id(u)?)
                    .execute(&mut *c)
                    .await?
                    .rows_affected();
            require(n > 0, ErrorCode::InvalidRequest)?;
            out.push((
                vec![u],
                Event::Threads(ThreadEvent::Membership {
                    thread_id,
                    membership: membership(c, thread_id, u).await?,
                }),
            ));
            ThreadResult::Ack
        }
        UpdateForum { forum_id, patch } => {
            ThreadResult::Forum(forum::update(c, u, forum_id, patch, out).await?)
        }
        Publish {
            channel_id,
            message_id,
        } => {
            announce::publish(c, u, channel_id, message_id, out).await?;
            ThreadResult::Ack
        }
        Follow {
            source_channel_id,
            target_channel_id,
        } => ThreadResult::Follow(
            announce::follow(c, core, u, source_channel_id, target_channel_id, out).await?,
        ),
        Unfollow { follow_id } => {
            announce::unfollow(c, u, follow_id, out).await?;
            ThreadResult::Ack
        }
    };
    Ok(CommandResult::Threads(result))
}

fn removed(info: &ThreadInfo, users: Vec<Id>) -> (Vec<Id>, Event) {
    (
        users,
        Event::Threads(ThreadEvent::ThreadRemoved {
            thread_id: info.id,
            parent_id: info.parent_id,
        }),
    )
}

/// Your joined active threads, newest activity first, with read states.
async fn snapshot(c: &mut SqliteConnection, u: Id) -> Result<ThreadSnapshot> {
    let mut out = ThreadSnapshot::default();
    for t in joined_active(c, u).await? {
        if let Some(m) = membership(c, t.id, u).await? {
            out.memberships.push(m);
        }
        out.read_states.push(read(c, u, t.id).await?);
        out.threads.push(t);
    }
    Ok(out)
}

async fn joined_active(c: &mut SqliteConnection, u: Id) -> Result<Vec<ThreadInfo>> {
    let r = sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} JOIN thread_members tm ON tm.thread_id=t.channel_id AND tm.user_id=? WHERE t.archived=0 ORDER BY t.last_activity_at DESC LIMIT ?")))
        .bind(id(u)?)
        .bind(MAX_JOINED_THREADS_SNAPSHOT as i64)
        .fetch_all(&mut *c)
        .await?;
    let mut list = vec![];
    for t in rows(c, r).await? {
        if access(c, u, t.id, Permissions::VIEW_CHANNEL).await.is_ok() {
            list.push(t);
        }
    }
    Ok(list)
}

/// Read states of the active threads you joined (added to `db::read_states`).
pub(crate) async fn member_reads(c: &mut SqliteConnection, u: Id) -> Result<Vec<ReadState>> {
    let mut out = vec![];
    for t in joined_active(c, u).await? {
        out.push(read(c, u, t.id).await?);
    }
    Ok(out)
}

/// The threads whose messages count for your Inbox mentions: joined, visible.
pub(crate) async fn readable(c: &mut SqliteConnection, u: Id) -> Result<Vec<Id>> {
    Ok(joined_active(c, u)
        .await?
        .into_iter()
        .map(|t| t.id)
        .collect())
}

async fn create(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    new: NewThread,
    out: &mut Deliveries,
) -> Result<ThreadInfo> {
    let parent = access(
        c,
        u,
        new.parent_id,
        Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
    )
    .await?;
    require(
        matches!(
            parent.summary.kind,
            ChannelKind::Text | ChannelKind::Announcement
        ),
        ErrorCode::InvalidRequest,
    )?;
    let g = parent
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    let right = if new.private {
        Permissions::CREATE_PRIVATE_THREADS
    } else {
        Permissions::CREATE_PUBLIC_THREADS
    };
    access(c, u, new.parent_id, right).await?;
    let name = thread_name(&new.name)?;
    let minutes = archive_minutes(new.auto_archive_minutes)?;
    let mut starter_author = None;
    if let Some(m) = new.message_id {
        // Private threads are never started from a message, as in Discord.
        require(!new.private, ErrorCode::InvalidRequest)?;
        let msg = message(c, u, new.parent_id, m, false)
            .await
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        require(msg.call.is_none(), ErrorCode::InvalidRequest)?;
        let taken: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM threads WHERE starter_message_id=?)")
                .bind(id(m)?)
                .fetch_one(&mut *c)
                .await?;
        require(!taken, ErrorCode::Conflict)?;
        starter_author = Some(msg.author_id);
    }
    active_room(c, g).await?;
    let t = core.id().await?;
    let at = now();
    sqlx::query("INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(?,'thread',?,?,?,0,?)")
        .bind(id(t)?)
        .bind(id(g)?)
        .bind(id(new.parent_id)?)
        .bind(&name)
        .bind(at as i64)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT INTO threads(channel_id,parent_id,guild_id,owner_id,starter_message_id,private,invitable,auto_archive_minutes,created_at,last_activity_at,archive_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
        .bind(id(t)?)
        .bind(id(new.parent_id)?)
        .bind(id(g)?)
        .bind(id(u)?)
        .bind(opt(new.message_id)?)
        .bind(new.private)
        .bind(!new.private || new.invitable)
        .bind(i64::from(minutes))
        .bind(at as i64)
        .bind(at as i64)
        .bind((at + u64::from(minutes) * 60_000) as i64)
        .execute(&mut *c)
        .await?;
    join(c, out, t, u).await?;
    if let Some(a) = starter_author.filter(|a| *a != u) {
        if access(c, a, t, Permissions::VIEW_CHANNEL).await.is_ok() {
            join(c, out, t, a).await?;
        }
    }
    announce(c, out, t).await
}

async fn update(
    c: &mut SqliteConnection,
    u: Id,
    t: Id,
    patch: ThreadPatch,
    out: &mut Deliveries,
) -> Result<ThreadInfo> {
    let (old, p) = visible(c, u, t).await?;
    let manage = p.contains(Permissions::MANAGE_THREADS);
    let owner = old.owner_id == u;
    // A locked thread is the moderators'. An archived one must be reopened
    // before anything else about it changes.
    require(!old.locked || manage, ErrorCode::Forbidden)?;
    require(
        !old.archived || manage || patch.archived == Some(false),
        ErrorCode::Forbidden,
    )?;
    let mut name = old.name.clone();
    if let Some(n) = &patch.name {
        require(owner || manage, ErrorCode::Forbidden)?;
        name = thread_name(n)?;
    }
    let mut archived = old.archived;
    if let Some(a) = patch.archived {
        require(owner || manage, ErrorCode::Forbidden)?;
        if old.archived && !a {
            active_room(c, old.guild_id).await?;
        }
        archived = a;
    }
    let mut locked = old.locked;
    if let Some(l) = patch.locked {
        require(manage, ErrorCode::Forbidden)?;
        locked = l;
    }
    let mut minutes = old.auto_archive_minutes;
    if let Some(m) = patch.auto_archive_minutes {
        require(owner || manage, ErrorCode::Forbidden)?;
        minutes = archive_minutes(m)?;
    }
    let mut invitable = old.invitable;
    if let Some(i) = patch.invitable {
        require(old.private && (owner || manage), ErrorCode::InvalidRequest)?;
        invitable = i;
    }
    let mut pinned = old.pinned;
    if let Some(pin) = patch.pinned {
        require(old.forum_post, ErrorCode::InvalidRequest)?;
        require(manage, ErrorCode::Forbidden)?;
        pinned = pin;
    }
    if let Some(tags) = &patch.tag_ids {
        require(old.forum_post, ErrorCode::InvalidRequest)?;
        require(owner || manage, ErrorCode::Forbidden)?;
        forum::check_tags(c, old.parent_id, &old.tag_ids, tags, manage).await?;
    }
    let at = now();
    let archived_at = if archived {
        old.archived_at.or(Some(at))
    } else {
        None
    };
    // Reopening counts as activity, so it does not archive again at once.
    let last_activity = if old.archived && !archived {
        at
    } else {
        old.last_activity_at
    };
    let archive_at = (!archived).then(|| last_activity + u64::from(minutes) * 60_000);
    if pinned && !old.pinned {
        let others: Vec<i64> = sqlx::query_scalar(
            "SELECT channel_id FROM threads WHERE parent_id=? AND pinned=1 AND channel_id<>?",
        )
        .bind(id(old.parent_id)?)
        .bind(id(t)?)
        .fetch_all(&mut *c)
        .await?;
        for other in others {
            sqlx::query("UPDATE threads SET pinned=0 WHERE channel_id=?")
                .bind(other)
                .execute(&mut *c)
                .await?;
            announce(c, out, Id(other as u64)).await?;
        }
    }
    sqlx::query("UPDATE channels SET name=? WHERE id=?")
        .bind(&name)
        .bind(id(t)?)
        .execute(&mut *c)
        .await?;
    sqlx::query("UPDATE threads SET archived=?,archived_at=?,archive_at=?,locked=?,auto_archive_minutes=?,invitable=?,pinned=?,last_activity_at=? WHERE channel_id=?")
        .bind(archived)
        .bind(archived_at.map(|v| v as i64))
        .bind(archive_at.map(|v| v as i64))
        .bind(locked)
        .bind(i64::from(minutes))
        .bind(invitable)
        .bind(pinned)
        .bind(last_activity as i64)
        .bind(id(t)?)
        .execute(&mut *c)
        .await?;
    if let Some(tags) = &patch.tag_ids {
        forum::set_tags(c, t, tags).await?;
    }
    announce(c, out, t).await
}

async fn add_member(
    c: &mut SqliteConnection,
    u: Id,
    t: Id,
    v: Id,
    out: &mut Deliveries,
) -> Result<()> {
    let (info, p) = visible(c, u, t).await?;
    let manage = p.contains(Permissions::MANAGE_THREADS);
    require(!info.locked || manage, ErrorCode::Forbidden)?;
    if info.private {
        require(
            manage || info.owner_id == u || (info.invitable && is_member(c, t, u).await?),
            ErrorCode::Forbidden,
        )?;
    } else {
        require(p.contains(Permissions::SEND_MESSAGES), ErrorCode::Forbidden)?;
    }
    // They must be able to see the channel the thread is in.
    let parent = crate::permissions::evaluate(c, v, info.guild_id, Some(info.parent_id))
        .await
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    require(
        parent.contains(Permissions::VIEW_CHANNEL),
        ErrorCode::InvalidRequest,
    )?;
    if join(c, out, t, v).await? {
        let info = announce(c, out, t).await?;
        // Make sure the new member hears about a private thread at all.
        out.push((vec![v], Event::Threads(ThreadEvent::ThreadUpsert(info))));
    }
    Ok(())
}

/// Deletes a thread and everything in it.
async fn remove_thread(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    info: &ThreadInfo,
) -> Result<()> {
    let before = audience(c, info.id).await?;
    sqlx::query("DELETE FROM channels WHERE id=?")
        .bind(id(info.id)?)
        .execute(&mut *c)
        .await?;
    out.push(removed(info, before));
    Ok(())
}

/// Before `DeleteChannel` moves a channel's children up: its threads go with it.
pub(crate) async fn parent_deleted(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    ch: Id,
) -> Result<()> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT channel_id FROM threads WHERE parent_id=?")
        .bind(id(ch)?)
        .fetch_all(&mut *c)
        .await?;
    for t in ids {
        if let Some(info) = load(c, Id(t as u64)).await? {
            remove_thread(c, out, &info).await?;
        }
    }
    Ok(())
}

/// `UpdateChannel` and `DeleteChannel` do not apply to threads (they have
/// their own commands, and Manage Threads rather than Manage Channels).
pub(crate) fn not_thread(ch: &Channel) -> Result<()> {
    require(
        ch.summary.kind != ChannelKind::Thread,
        ErrorCode::InvalidRequest,
    )
}

/// After `CreateChannel` inserts a channel.
pub(crate) async fn channel_created(
    c: &mut SqliteConnection,
    ch: Id,
    kind: ChannelKind,
) -> Result<()> {
    if kind == ChannelKind::Forum {
        sqlx::query("INSERT INTO forum_settings(channel_id) VALUES(?)")
            .bind(id(ch)?)
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}

/// Uploads: a forum takes files for a post that is not created yet (they move
/// into the post when it is). Everything else is the message gate.
pub(crate) async fn upload_target(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    p: Permissions,
) -> Result<Channel> {
    let channel = access(c, u, ch, Permissions::VIEW_CHANNEL).await?;
    if channel.summary.kind == ChannelKind::Forum {
        return access(c, u, ch, p | Permissions::SEND_MESSAGES).await;
    }
    text(c, u, ch, p).await
}

/// `SendMessage`, before its checks: people mentioned in a private thread
/// join it (when the sender may add people), forum reply slowmode, and the
/// active-thread limit for a message that would reopen an archived thread.
pub(crate) async fn before_send(
    c: &mut SqliteConnection,
    u: Id,
    d: &MessageDraft,
    out: &mut Deliveries,
) -> Result<()> {
    let Some(info) = load(c, d.channel_id).await? else {
        return Ok(());
    };
    let p = crate::permissions::evaluate(c, u, info.guild_id, Some(info.id)).await?;
    require(p.contains(Permissions::VIEW_CHANNEL), ErrorCode::NotFound)?;
    let manage = p.contains(Permissions::MANAGE_THREADS);
    if info.archived {
        active_room(c, info.guild_id).await?;
    }
    if info.private && p.contains(Permissions::SEND_MESSAGES) {
        let inviter =
            manage || info.owner_id == u || (info.invitable && is_member(c, info.id, u).await?);
        for v in &d.mention_ids {
            if !inviter || *v == u || id(*v).is_err() {
                continue;
            }
            let sees = crate::permissions::evaluate(c, *v, info.guild_id, Some(info.parent_id))
                .await
                .is_ok_and(|p| p.contains(Permissions::VIEW_CHANNEL));
            if sees {
                join(c, out, info.id, *v).await?;
            }
        }
    }
    if info.forum_post && info.message_count > 0 && !manage {
        forum::reply_slowmode(c, u, &info).await?;
    }
    Ok(())
}

/// `SendMessage`, after the message is stored: activity, reopening, the
/// author and the people they named join, and everyone sees the new count.
pub(crate) async fn after_send(
    c: &mut SqliteConnection,
    u: Id,
    m: &Message,
    explicit: &[Id],
    out: &mut Deliveries,
) -> Result<()> {
    let Some(info) = load(c, m.channel_id).await? else {
        return Ok(());
    };
    let at = now();
    sqlx::query("UPDATE threads SET archived=0,archived_at=NULL,last_activity_at=?,archive_at=?,first_message_id=COALESCE(first_message_id,CASE WHEN forum_post=1 THEN ? END) WHERE channel_id=?")
        .bind(at as i64)
        .bind((at + u64::from(info.auto_archive_minutes) * 60_000) as i64)
        .bind(id(m.id)?)
        .bind(id(info.id)?)
        .execute(&mut *c)
        .await?;
    join(c, out, info.id, u).await?;
    for v in explicit {
        if *v != u
            && access(c, *v, info.id, Permissions::VIEW_CHANNEL)
                .await
                .is_ok()
        {
            join(c, out, info.id, *v).await?;
        }
    }
    announce(c, out, info.id).await?;
    Ok(())
}

/// A forum's settings, for the audit log (`moderation::feature_audit`).
pub(crate) async fn forum_settings(c: &mut SqliteConnection, forum: Id) -> Result<ForumSettings> {
    forum::settings(c, forum).await
}
/// A following, for the audit log.
pub(crate) async fn follow_info(c: &mut SqliteConnection, f: Id) -> Result<ChannelFollow> {
    announce::follow_row(c, f).await
}

/// `ReadStateUpdated` in a thread goes only to its members.
pub(crate) async fn readers(c: &mut SqliteConnection, ch: Id, users: Vec<Id>) -> Result<Vec<Id>> {
    let thread: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM threads WHERE channel_id=?)")
            .bind(id(ch)?)
            .fetch_one(&mut *c)
            .await?;
    if !thread {
        return Ok(users);
    }
    let members: Vec<i64> =
        sqlx::query_scalar("SELECT user_id FROM thread_members WHERE thread_id=?")
            .bind(id(ch)?)
            .fetch_all(&mut *c)
            .await?;
    Ok(users
        .into_iter()
        .filter(|u| members.contains(&(u.0 as i64)))
        .collect())
}

/// `MarkRead` in a thread you have not joined: nothing is tracked for you.
pub(crate) async fn untracked(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
) -> Result<Option<ReadState>> {
    let tracked: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM threads WHERE channel_id=?1) OR EXISTS(SELECT 1 FROM read_states WHERE user_id=?2 AND channel_id=?1)",
    )
    .bind(id(ch)?)
    .bind(id(u)?)
    .fetch_one(&mut *c)
    .await?;
    if tracked {
        return Ok(None);
    }
    Ok(Some(ReadState {
        channel_id: ch,
        last_read_id: None,
        latest_message_id: None,
        unread_count: 0,
        mention_count: 0,
    }))
}

/// Notification level in a thread: members use their thread setting (Default
/// follows the parent channel's); everyone else hears only about mentions.
pub(crate) async fn notify_level(
    c: &mut SqliteConnection,
    user: Id,
    channel: &Channel,
    settings: &[NotificationSettings],
    level: &mut NotificationLevel,
    muted: &mut bool,
) -> Result<()> {
    if channel.summary.kind != ChannelKind::Thread {
        return Ok(());
    }
    let Some(parent) =
        sqlx::query_scalar::<_, i64>("SELECT parent_id FROM threads WHERE channel_id=?")
            .bind(id(channel.summary.id)?)
            .fetch_optional(&mut *c)
            .await?
    else {
        return Ok(());
    };
    match membership(c, channel.summary.id, user)
        .await?
        .map(|m| m.notify)
    {
        None => *level = NotificationLevel::Mentions,
        Some(ThreadNotify::Default) => {
            if let Some(s) = settings
                .iter()
                .find(|s| s.channel_id == Some(Id(parent as u64)))
            {
                *level = s.level;
                *muted = s.muted && s.mute_until.is_none_or(|t| t > now());
            }
        }
        Some(ThreadNotify::All) => *level = NotificationLevel::All,
        Some(ThreadNotify::Mentions) => *level = NotificationLevel::Mentions,
        Some(ThreadNotify::Nothing) => *level = NotificationLevel::Nothing,
    }
    Ok(())
}

/// Fills in the thread preview, crosspost source and published flag.
pub(crate) async fn decorate(c: &mut SqliteConnection, viewer: Id, m: &mut Message) -> Result<()> {
    let r = sqlx::query("SELECT (SELECT channel_id FROM threads WHERE starter_message_id=?1) AS thread,(SELECT source_guild_name FROM crossposts WHERE message_id=?1) AS guild,(SELECT source_channel_name FROM crossposts WHERE message_id=?1) AS channel,EXISTS(SELECT 1 FROM published_messages WHERE message_id=?1) AS published")
        .bind(id(m.id)?)
        .fetch_one(&mut *c)
        .await?;
    let mut extra = MessageThreads {
        published: r.get::<i64, _>("published") != 0,
        ..Default::default()
    };
    if let (Some(guild), Some(channel)) = (
        r.get::<Option<String>, _>("guild"),
        r.get::<Option<String>, _>("channel"),
    ) {
        extra.crosspost = Some(Crosspost {
            source_guild_name: guild,
            source_channel_name: channel,
        });
    }
    if let Some(t) = r.get::<Option<i64>, _>("thread") {
        let t = Id(t as u64);
        if access(c, viewer, t, Permissions::VIEW_CHANNEL)
            .await
            .is_ok()
        {
            if let Some(info) = load(c, t).await? {
                extra.thread = Some(preview(&info));
            }
        }
    }
    m.extra.set_threads(|t| *t = extra);
    Ok(())
}

fn preview(t: &ThreadInfo) -> ThreadPreview {
    ThreadPreview {
        thread_id: t.id,
        name: t.name.clone(),
        message_count: t.message_count,
        last_activity_at: t.last_activity_at,
        archived: t.archived,
        locked: t.locked,
        private: t.private,
    }
}

/// `EditMessage` changed a message: published ones update their copies.
pub(crate) async fn source_edited(c: &mut SqliteConnection, m: Id) -> Result<()> {
    announce::queue_change(c, m, "update").await
}
/// `DeleteMessage` removed a message: published ones remove their copies.
pub(crate) async fn source_deleted(c: &mut SqliteConnection, m: Id) -> Result<()> {
    announce::queue_change(c, m, "delete").await
}

/// Search scope for one server: threads under the readable channels `parents`
/// that `u` may read (private ones only as a member or with Manage Threads).
/// Returns SQL selecting their channel IDs, and its bound values in order.
pub(crate) async fn search_scope(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    parents: &[Id],
) -> Result<(Vec<i64>, Vec<i64>)> {
    let mut manage = vec![];
    let mut all = vec![];
    for p in parents {
        all.push(id(*p)?);
        if crate::permissions::evaluate(c, u, g, Some(*p))
            .await?
            .contains(Permissions::MANAGE_THREADS)
        {
            manage.push(id(*p)?);
        }
    }
    Ok((all, manage))
}

/// Called after a command commits, for each delivery: re-arms the archive
/// timer when a deadline moved earlier and wakes the crosspost worker.
pub(crate) fn observe(app: &App, e: &Event) {
    let threads = &app.0.threads;
    if let Event::Threads(ThreadEvent::ThreadUpsert(t)) = e {
        if !t.archived {
            let due = t.last_activity_at + u64::from(t.auto_archive_minutes) * 60_000;
            if due < threads.armed.load(Ordering::Relaxed) {
                threads.wake.notify_one();
            }
        }
    }
    if QUEUED.swap(false, Ordering::AcqRel) {
        threads.wake.notify_one();
    }
}
fn queued() {
    QUEUED.store(true, Ordering::Release);
}

/// Starts the thread timer: auto-archive deadlines and crosspost deliveries.
pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            let _ = announce::deliver_queue(&app).await;
            let next = match next_deadline(&app).await {
                Ok(next) => next,
                Err(_) => Some(now() + 30_000),
            };
            app.0
                .threads
                .armed
                .store(next.unwrap_or(u64::MAX), Ordering::Relaxed);
            let sleep = async {
                match next {
                    Some(at) => {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            at.saturating_sub(now()),
                        ))
                        .await
                    }
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = app.0.shutdown.cancelled() => return,
                _ = app.0.threads.wake.notified() => continue,
                _ = sleep => {}
            }
            let _ = archive_due(&app).await;
        }
    });
}

async fn next_deadline(app: &App) -> Result<Option<u64>> {
    let mut c = app.0.pool.acquire().await?;
    let at: Option<i64> =
        sqlx::query_scalar("SELECT MIN(archive_at) FROM threads WHERE archive_at IS NOT NULL")
            .fetch_one(&mut *c)
            .await?;
    Ok(at.map(|v| v.max(0) as u64))
}

/// Archives every thread whose time has come, in batches.
pub(crate) async fn archive_due(app: &App) -> Result<usize> {
    let mut total = 0;
    loop {
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let due: Vec<i64> = sqlx::query_scalar(
            "SELECT channel_id FROM threads WHERE archive_at<=? ORDER BY archive_at LIMIT 100",
        )
        .bind(now() as i64)
        .fetch_all(&mut *c)
        .await?;
        if due.is_empty() {
            return Ok(total);
        }
        let mut out = vec![];
        let mut tx = sqlx::Connection::begin(&mut *c).await?;
        for t in &due {
            sqlx::query("UPDATE threads SET archived=1,archived_at=archive_at,archive_at=NULL WHERE channel_id=?")
                .bind(t)
                .execute(&mut *tx)
                .await?;
            announce(&mut tx, &mut out, Id(*t as u64)).await?;
        }
        tx.commit().await?;
        total += due.len();
        crate::content::deliver(app, &mut core, &mut c, out).await?;
    }
}
