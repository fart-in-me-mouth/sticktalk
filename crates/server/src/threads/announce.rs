//! Announcement channels: publishing, following and crosspost delivery.
//!
//! Publishing queues one delivery per following channel. The worker (woken by
//! `observe` after the command commits) writes each copy in its own short
//! transaction, a few at a time, so a channel with many followers never holds
//! the command gate for long. A copy carries only the published message: its
//! text (mentions written out as plain names) and files, the source server
//! and channel names, and a link back that only the source's readers get.
use super::*;

/// The most deliveries waiting at once (all channels).
const MAX_QUEUE: i64 = 10_000;
/// Copies written per worker turn before the gate is released.
const BATCH: i64 = 16;

async fn announcement(c: &mut SqliteConnection, u: Id, ch: Id, p: Permissions) -> Result<Channel> {
    let channel = access(c, u, ch, Permissions::VIEW_CHANNEL | p).await?;
    require(
        channel.summary.kind == ChannelKind::Announcement,
        ErrorCode::InvalidRequest,
    )?;
    Ok(channel)
}

pub(super) async fn publish(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    m: Id,
    out: &mut Deliveries,
) -> Result<()> {
    let channel = announcement(c, u, ch, Permissions::READ_HISTORY).await?;
    let g = channel
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    let msg = message(c, u, ch, m, false).await?;
    let p = crate::permissions::evaluate(c, u, g, Some(ch)).await?;
    require(
        p.contains(Permissions::MANAGE_MESSAGES)
            || (msg.author_id == u && p.contains(Permissions::PUBLISH_MESSAGES)),
        ErrorCode::Forbidden,
    )?;
    require(
        msg.call.is_none() && msg.extra.crosspost().is_none() && msg.extra.poll.is_none(),
        ErrorCode::InvalidRequest,
    )?;
    require(!msg.extra.published(), ErrorCode::Conflict)?;
    let recent: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM published_messages WHERE channel_id=? AND published_at>?",
    )
    .bind(id(ch)?)
    .bind(now().saturating_sub(3_600_000) as i64)
    .fetch_one(&mut *c)
    .await?;
    if recent as usize >= MAX_PUBLISHES_PER_HOUR {
        let oldest: i64 = sqlx::query_scalar(
            "SELECT MIN(published_at) FROM published_messages WHERE channel_id=? AND published_at>?",
        )
        .bind(id(ch)?)
        .bind(now().saturating_sub(3_600_000) as i64)
        .fetch_one(&mut *c)
        .await?;
        let wait = (oldest.max(0) as u64 + 3_600_000).saturating_sub(now());
        let mut e = Error::rate(wait.clamp(1, u64::from(u32::MAX)) as u32);
        e.0.message = "This channel has published 10 messages in the last hour.".into();
        return Err(e);
    }
    let followers: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM channel_follows WHERE source_channel_id=?")
            .bind(id(ch)?)
            .fetch_one(&mut *c)
            .await?;
    let waiting: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM crosspost_queue")
        .fetch_one(&mut *c)
        .await?;
    require(waiting + followers <= MAX_QUEUE, ErrorCode::Unavailable)?;
    sqlx::query("INSERT INTO published_messages(message_id,channel_id,published_by,published_at) VALUES(?,?,?,?)")
        .bind(id(m)?)
        .bind(id(ch)?)
        .bind(id(u)?)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    let n = sqlx::query("INSERT OR IGNORE INTO crosspost_queue(source_message_id,follow_id,action,queued_at) SELECT ?,id,'create',? FROM channel_follows WHERE source_channel_id=?")
        .bind(id(m)?)
        .bind(now() as i64)
        .bind(id(ch)?)
        .execute(&mut *c)
        .await?
        .rows_affected();
    if n > 0 {
        queued();
    }
    out.push((
        audience(c, ch).await?,
        Event::Threads(ThreadEvent::Published {
            channel_id: ch,
            message_id: m,
        }),
    ));
    Ok(())
}

/// A published message changed: queue the change for every follower (a
/// waiting delivery absorbs it). Nothing happens for other messages.
pub(super) async fn queue_change(c: &mut SqliteConnection, m: Id, action: &str) -> Result<()> {
    let n = sqlx::query("INSERT INTO crosspost_queue(source_message_id,follow_id,action,queued_at) SELECT p.message_id,f.id,?,? FROM published_messages p JOIN channel_follows f ON f.source_channel_id=p.channel_id WHERE p.message_id=? ON CONFLICT(source_message_id,follow_id) DO UPDATE SET action=CASE WHEN crosspost_queue.action='create' THEN 'create' ELSE excluded.action END")
        .bind(action)
        .bind(now() as i64)
        .bind(id(m)?)
        .execute(&mut *c)
        .await?
        .rows_affected();
    if n > 0 {
        queued();
    }
    Ok(())
}

async fn follow_row(c: &mut SqliteConnection, f: Id) -> Result<ChannelFollow> {
    let r = sqlx::query("SELECT f.*,s.guild_id AS sg,s.name AS sn,t.guild_id AS tg,t.name AS tn,(SELECT name FROM guilds WHERE id=s.guild_id) AS sgn,(SELECT name FROM guilds WHERE id=t.guild_id) AS tgn FROM channel_follows f JOIN channels s ON s.id=f.source_channel_id JOIN channels t ON t.id=f.target_channel_id WHERE f.id=?")
        .bind(id(f)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok(ChannelFollow {
        id: f,
        source_channel_id: Id(r.get::<i64, _>("source_channel_id") as u64),
        source_guild_id: Id(r.get::<i64, _>("sg") as u64),
        source_guild_name: r.get("sgn"),
        source_channel_name: r.get("sn"),
        target_channel_id: Id(r.get::<i64, _>("target_channel_id") as u64),
        target_guild_id: Id(r.get::<i64, _>("tg") as u64),
        target_guild_name: r.get("tgn"),
        target_channel_name: r.get("tn"),
        created_by: Id(r.get::<i64, _>("created_by") as u64),
        created_at: r.get::<i64, _>("created_at").max(0) as u64,
    })
}

/// Who may manage a following into this text channel: Manage Server there
/// (StickTalk has no webhooks; this is the closest right).
async fn target_manager(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<Channel> {
    let target = access(c, u, ch, Permissions::VIEW_CHANNEL).await?;
    require(
        target.summary.kind == ChannelKind::Text,
        ErrorCode::InvalidRequest,
    )?;
    let g = target
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    Ok(target)
}
/// Who may see and remove the followers of an announcement channel.
async fn source_manager(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<bool> {
    Ok(announcement(c, u, ch, Permissions::MANAGE_CHANNELS)
        .await
        .is_ok())
}

pub(super) async fn follow(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    source: Id,
    target: Id,
    out: &mut Deliveries,
) -> Result<ChannelFollow> {
    core.rate(format!("follow:{}", u.0), 10., 5.)?;
    announcement(c, u, source, Permissions::READ_HISTORY).await?;
    target_manager(c, u, target).await?;
    let (followers, follows): (i64, i64) = (
        sqlx::query_scalar("SELECT COUNT(*) FROM channel_follows WHERE source_channel_id=?")
            .bind(id(source)?)
            .fetch_one(&mut *c)
            .await?,
        sqlx::query_scalar("SELECT COUNT(*) FROM channel_follows WHERE target_channel_id=?")
            .bind(id(target)?)
            .fetch_one(&mut *c)
            .await?,
    );
    require(
        (followers as usize) < MAX_FOLLOWERS_PER_CHANNEL
            && (follows as usize) < MAX_FOLLOWS_PER_TARGET,
        ErrorCode::LimitExceeded,
    )?;
    let f = core.id().await?;
    let inserted = sqlx::query("INSERT OR IGNORE INTO channel_follows(id,source_channel_id,target_channel_id,created_by,created_at) VALUES(?,?,?,?,?)")
        .bind(id(f)?)
        .bind(id(source)?)
        .bind(id(target)?)
        .bind(id(u)?)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?
        .rows_affected();
    require(inserted == 1, ErrorCode::Conflict)?;
    for ch in [source, target] {
        out.push((
            vec![u],
            Event::Threads(ThreadEvent::FollowsChanged { channel_id: ch }),
        ));
    }
    follow_row(c, f).await
}

pub(super) async fn unfollow(
    c: &mut SqliteConnection,
    u: Id,
    f: Id,
    out: &mut Deliveries,
) -> Result<()> {
    let row = follow_row(c, f).await?;
    let allowed = target_manager(c, u, row.target_channel_id).await.is_ok()
        || source_manager(c, u, row.source_channel_id).await?;
    require(allowed, ErrorCode::NotFound)?;
    sqlx::query("DELETE FROM channel_follows WHERE id=?")
        .bind(id(f)?)
        .execute(&mut *c)
        .await?;
    for ch in [row.source_channel_id, row.target_channel_id] {
        out.push((
            vec![u],
            Event::Threads(ThreadEvent::FollowsChanged { channel_id: ch }),
        ));
    }
    Ok(())
}

/// `GET /api/v1/channels/{c}/follows`: the followers of an announcement
/// channel (for its managers) or what a text channel follows (for the
/// target server's managers). Anyone else gets NotFound.
pub(super) async fn follows(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<ChannelFollows> {
    let column = if source_manager(c, u, ch).await? {
        "source_channel_id"
    } else if target_manager(c, u, ch).await.is_ok() {
        "target_channel_id"
    } else {
        return Err(Error::new(ErrorCode::NotFound));
    };
    let ids: Vec<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT id FROM channel_follows WHERE {column}=? ORDER BY created_at LIMIT 100"
    )))
    .bind(id(ch)?)
    .fetch_all(&mut *c)
    .await?;
    let mut out = ChannelFollows::default();
    for f in ids {
        out.follows.push(follow_row(c, Id(f as u64)).await?);
    }
    Ok(out)
}

/// Writes queued copies until the queue is empty.
pub(super) async fn deliver_queue(app: &App) -> Result<()> {
    loop {
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let jobs = sqlx::query("SELECT source_message_id,follow_id,action FROM crosspost_queue ORDER BY queued_at LIMIT ?")
            .bind(BATCH)
            .fetch_all(&mut *c)
            .await?;
        if jobs.is_empty() {
            return Ok(());
        }
        for job in jobs {
            let source = Id(job.get::<i64, _>("source_message_id") as u64);
            let follow = Id(job.get::<i64, _>("follow_id") as u64);
            let action: String = job.get("action");
            let mut out = vec![];
            let mut tx = sqlx::Connection::begin(&mut *c).await?;
            let done = deliver(&mut tx, &mut core, source, follow, &action, &mut out).await;
            if done.is_err() {
                // Undo the partial copy. A delivery that cannot be made is
                // dropped below, never retried forever.
                drop(tx);
                out.clear();
                tx = sqlx::Connection::begin(&mut *c).await?;
            }
            sqlx::query("DELETE FROM crosspost_queue WHERE source_message_id=? AND follow_id=?")
                .bind(id(source)?)
                .bind(id(follow)?)
                .execute(&mut *tx)
                .await?;
            core.persist_id(&mut tx).await?;
            tx.commit().await?;
            if let Err(e) = done {
                tracing::warn!(code = ?e.0.code, "crosspost delivery dropped");
            }
            crate::content::deliver(app, &mut core, &mut c, out).await?;
        }
    }
}

async fn deliver(
    c: &mut SqliteConnection,
    core: &mut Core,
    source: Id,
    follow: Id,
    action: &str,
    out: &mut Deliveries,
) -> Result<()> {
    let f = follow_row(c, follow).await?;
    let copies: Vec<i64> = sqlx::query_scalar(
        "SELECT message_id FROM crossposts WHERE source_message_id=? AND follow_id=?",
    )
    .bind(id(source)?)
    .bind(id(follow)?)
    .fetch_all(&mut *c)
    .await?;
    let original = sqlx::query("SELECT content,author_id,created_at FROM messages WHERE id=? AND channel_id=? AND deleted_at IS NULL")
        .bind(id(source)?)
        .bind(id(f.source_channel_id)?)
        .fetch_optional(&mut *c)
        .await?;
    let target = f.target_channel_id;
    // The copy is the target server's: its owner acts for the server.
    let owner = Id(
        sqlx::query_scalar::<_, i64>("SELECT owner_id FROM guilds WHERE id=?")
            .bind(id(f.target_guild_id)?)
            .fetch_one(&mut *c)
            .await? as u64,
    );
    match (action, original) {
        ("delete", _) | (_, None) => {
            for m in copies {
                Box::pin(crate::commands::mutate(
                    c,
                    core,
                    &[],
                    owner,
                    Command::DeleteMessage {
                        channel_id: target,
                        message_id: Id(m as u64),
                    },
                    out,
                ))
                .await?;
            }
        }
        ("update", Some(r)) => {
            let text = crate::content::plain_text(c, &r.get::<String, _>("content")).await?;
            for m in copies {
                sqlx::query("UPDATE messages SET content=?,edited_at=? WHERE id=?")
                    .bind(&text)
                    .bind(now() as i64)
                    .bind(m)
                    .execute(&mut *c)
                    .await?;
                sqlx::query("UPDATE forwarded_messages SET content=? WHERE message_id=?")
                    .bind(&text)
                    .bind(m)
                    .execute(&mut *c)
                    .await?;
                for (users, copy) in
                    copies_for(c, owner, target, Id(m as u64), f.source_channel_id).await?
                {
                    out.push((users, Event::MessageUpdated(copy)));
                }
            }
        }
        (_, Some(r)) => {
            if !copies.is_empty() {
                return Ok(());
            }
            let text = crate::content::plain_text(c, &r.get::<String, _>("content")).await?;
            let m = core.id().await?;
            sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(?,?,?,?,?)")
                .bind(id(m)?)
                .bind(id(target)?)
                .bind(id(f.created_by)?)
                .bind(&text)
                .bind(now() as i64)
                .execute(&mut *c)
                .await?;
            sqlx::query("INSERT INTO forwarded_messages(message_id,source_channel_id,source_message_id,content,created_at) VALUES(?,?,?,?,?)")
                .bind(id(m)?)
                .bind(id(f.source_channel_id)?)
                .bind(id(source)?)
                .bind(&text)
                .bind(r.get::<i64, _>("created_at"))
                .execute(&mut *c)
                .await?;
            sqlx::query("INSERT INTO forwarded_attachments(message_id,attachment_id,position) SELECT ?,attachment_id,position FROM message_attachments WHERE message_id=?")
                .bind(id(m)?)
                .bind(id(source)?)
                .execute(&mut *c)
                .await?;
            sqlx::query("INSERT INTO crossposts(message_id,source_message_id,source_channel_id,source_guild_id,source_guild_name,source_channel_name,follow_id) VALUES(?,?,?,?,?,?,?)")
                .bind(id(m)?)
                .bind(id(source)?)
                .bind(id(f.source_channel_id)?)
                .bind(id(f.source_guild_id)?)
                .bind(&f.source_guild_name)
                .bind(&f.source_channel_name)
                .bind(id(follow)?)
                .execute(&mut *c)
                .await?;
            let author = user(c, f.created_by).await?;
            for (users, copy) in copies_for(c, owner, target, m, f.source_channel_id).await? {
                out.push((
                    users,
                    Event::MessageCreated {
                        message: copy,
                        author: author.clone(),
                    },
                ));
            }
            crate::commands::reads_event(c, out, target).await?;
        }
    }
    Ok(())
}

/// The copy as the target channel's readers see it, in two groups: with the
/// link back to the source for people who can read the source, without it
/// for everyone else (as forwarding does).
async fn copies_for(
    c: &mut SqliteConnection,
    owner: Id,
    target: Id,
    m: Id,
    source_channel: Id,
) -> Result<Vec<(Vec<Id>, Message)>> {
    let base = message(c, owner, target, m, false).await?;
    let link =
        sqlx::query("SELECT source_message_id,source_guild_id FROM crossposts WHERE message_id=?")
            .bind(id(m)?)
            .fetch_one(&mut *c)
            .await?;
    let mut readers = vec![];
    let mut others = vec![];
    for v in audience(c, target).await? {
        if text(c, v, source_channel, Permissions::READ_HISTORY)
            .await
            .is_ok()
        {
            readers.push(v);
        } else {
            others.push(v);
        }
    }
    let mut out = vec![];
    for (users, linked) in [(readers, true), (others, false)] {
        if users.is_empty() {
            continue;
        }
        let mut copy = base.clone();
        if let Some(f) = copy.extra.forwarded.as_mut() {
            f.source = linked.then(|| ForwardSource {
                channel_id: source_channel,
                message_id: Id(link.get::<i64, _>("source_message_id") as u64),
                guild_id: Some(Id(link.get::<i64, _>("source_guild_id") as u64)),
            });
        }
        out.push((users, copy));
    }
    Ok(out)
}
