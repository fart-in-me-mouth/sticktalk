//! Forwarding: a destination message carries a frozen snapshot of the source's
//! text and attachments. Attachments are the same stored files, referenced
//! again (never re-uploaded). The link back to the source is only shown to
//! people who can read the source.
use super::*;

/// Message text with `<@id>` mentions written out as "@name", so a snapshot
/// reads the same in a conversation where those people are unknown.
async fn plain_mentions(c: &mut SqliteConnection, content: &str) -> Result<String> {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(at) = rest.find("<@") {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 2..];
        let end = tail.find('>').filter(|e| *e > 0 && *e <= 20);
        match end.and_then(|e| tail[..e].parse::<u64>().ok().map(|v| (e, v))) {
            Some((e, v)) if v > 0 && v <= i64::MAX as u64 => {
                let name = user(c, Id(v))
                    .await
                    .map(|u| u.display_name.unwrap_or(u.username))
                    .unwrap_or_else(|_| "unknown".into());
                out.push('@');
                out.push_str(&name);
                rest = &tail[e + 1..];
            }
            _ => {
                out.push_str("<@");
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

async fn attachments(c: &mut SqliteConnection, m: Id) -> Result<Vec<Attachment>> {
    Ok(sqlx::query("SELECT u.* FROM uploads u JOIN forwarded_attachments f ON f.attachment_id=u.id WHERE f.message_id=? ORDER BY f.position")
        .bind(id(m)?)
        .fetch_all(&mut *c)
        .await?
        .iter()
        .map(attachment_row)
        .collect())
}

/// The snapshot on a forwarded message, as `viewer` sees it.
pub(super) async fn load(
    c: &mut SqliteConnection,
    viewer: Id,
    m: Id,
) -> Result<Option<ForwardedMessage>> {
    let Some(r) = sqlx::query("SELECT source_channel_id,source_message_id,content,created_at FROM forwarded_messages WHERE message_id=?")
        .bind(id(m)?)
        .fetch_optional(&mut *c)
        .await?
    else {
        return Ok(None);
    };
    let source_channel = Id(r.get::<i64, _>("source_channel_id") as u64);
    let source_message = Id(r.get::<i64, _>("source_message_id") as u64);
    Ok(Some(ForwardedMessage {
        content: r.get("content"),
        attachments: attachments(c, m).await?,
        created_at: r.get::<i64, _>("created_at").max(0) as u64,
        source: source(c, viewer, source_channel, source_message).await?,
    }))
}

/// Where a snapshot came from, if `viewer` may read it there.
async fn source(
    c: &mut SqliteConnection,
    viewer: Id,
    ch: Id,
    m: Id,
) -> Result<Option<ForwardSource>> {
    let Ok(channel) = text(c, viewer, ch, Permissions::READ_HISTORY).await else {
        return Ok(None);
    };
    let live: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE id=? AND channel_id=? AND deleted_at IS NULL)",
    )
    .bind(id(m)?)
    .bind(id(ch)?)
    .fetch_one(&mut *c)
    .await?;
    Ok(live.then_some(ForwardSource {
        channel_id: ch,
        message_id: m,
        guild_id: channel.summary.guild_id,
    }))
}

pub(super) async fn file_access(c: &mut SqliteConnection, u: Id, a: Id) -> Result<bool> {
    let channels = sqlx::query_scalar::<_, i64>("SELECT DISTINCT m.channel_id FROM forwarded_attachments f JOIN messages m ON m.id=f.message_id WHERE f.attachment_id=? AND m.deleted_at IS NULL LIMIT 64")
        .bind(id(a)?)
        .fetch_all(&mut *c)
        .await?;
    for ch in channels {
        if text(c, u, Id(ch as u64), Permissions::READ_HISTORY)
            .await
            .is_ok()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The text a forward of this message would carry, or None if `u` cannot
/// forward it. Moderation checks this before the command runs.
pub(super) async fn snapshot_text(
    c: &mut SqliteConnection,
    u: Id,
    source_channel: Id,
    source_message: Id,
) -> Option<String> {
    text(c, u, source_channel, Permissions::READ_HISTORY)
        .await
        .ok()?;
    let original = message(c, u, source_channel, source_message, false)
        .await
        .ok()?;
    match &original.extra.forwarded {
        Some(f) => Some(f.content.clone()),
        None => plain_mentions(c, &original.content).await.ok(),
    }
}

/// Same checks as sending a message: may send (and attach), not blocked in a
/// DM, slowmode. A hidden DM is shown again for its members.
async fn destination(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    files: bool,
    out: &mut Deliveries,
) -> Result<(Channel, Vec<Id>)> {
    let channel = text(c, u, ch, Permissions::SEND_MESSAGES).await?;
    if files {
        access(c, u, ch, Permissions::ATTACH_FILES).await?;
    }
    let recipients = audience(c, ch).await?;
    if channel.summary.kind == ChannelKind::Direct {
        let peer = *channel
            .summary
            .recipient_ids
            .iter()
            .find(|v| **v != u)
            .ok_or_else(|| Error::new(ErrorCode::Internal))?;
        require(!blocked(c, u, peer).await?, ErrorCode::Blocked)?;
    }
    crate::slowmode::check(c, u, ch).await?;
    if channel.summary.kind == ChannelKind::Direct {
        let mut changed = vec![];
        for v in &recipients {
            dm_quota(c, *v, Some(ch)).await?;
            let visible: i64 = sqlx::query_scalar(
                "SELECT visible FROM dm_members WHERE channel_id=? AND user_id=?",
            )
            .bind(id(ch)?)
            .bind(id(*v)?)
            .fetch_one(&mut *c)
            .await?;
            if visible == 0 {
                changed.push(*v);
            }
        }
        sqlx::query("UPDATE dm_members SET visible=1 WHERE channel_id=?")
            .bind(id(ch)?)
            .execute(&mut *c)
            .await?;
        if !changed.is_empty() {
            for v in &recipients {
                out.push((changed.clone(), Event::UserUpdated(user(c, *v).await?)));
            }
            out.push((changed, Event::ChannelUpsert(channel.clone())));
        }
    }
    Ok((channel, recipients))
}

pub(super) async fn forward(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    (source_channel, source_message): (Id, Id),
    destinations: Vec<Id>,
    comment: String,
    out: &mut Deliveries,
) -> Result<ContentResult> {
    require(!destinations.is_empty(), ErrorCode::InvalidRequest)?;
    unique(&destinations, MAX_FORWARD_DESTINATIONS)?;
    let comment = comment.trim().to_owned();
    require(
        comment.chars().count() <= MAX_MESSAGE_CHARS
            && !comment
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t'),
        ErrorCode::InvalidRequest,
    )?;
    text(c, u, source_channel, Permissions::READ_HISTORY).await?;
    let original = message(c, u, source_channel, source_message, false).await?;
    // Forwarding a forward passes its snapshot on.
    let (content, files, created_at) = match &original.extra.forwarded {
        Some(f) => (f.content.clone(), f.attachments.clone(), f.created_at),
        None => (
            plain_mentions(c, &original.content).await?,
            original.attachments.clone(),
            original.created_at,
        ),
    };
    require(
        !content.trim().is_empty() || !files.is_empty(),
        ErrorCode::InvalidRequest,
    )?;
    let mut sent = vec![];
    for ch in destinations {
        let (_, recipients) = destination(c, u, ch, !files.is_empty(), out).await?;
        let m = core.id().await?;
        sqlx::query(
            "INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(?,?,?,?,?)",
        )
        .bind(id(m)?)
        .bind(id(ch)?)
        .bind(id(u)?)
        .bind(&comment)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
        sqlx::query("INSERT INTO forwarded_messages(message_id,source_channel_id,source_message_id,content,created_at) VALUES(?,?,?,?,?)")
            .bind(id(m)?)
            .bind(id(source_channel)?)
            .bind(id(source_message)?)
            .bind(&content)
            .bind(created_at as i64)
            .execute(&mut *c)
            .await?;
        for (i, a) in files.iter().enumerate() {
            sqlx::query("INSERT INTO forwarded_attachments(message_id,attachment_id,position) VALUES(?,?,?)")
                .bind(id(m)?)
                .bind(id(a.id)?)
                .bind(i as i64)
                .execute(&mut *c)
                .await?;
        }
        let message = message(c, u, ch, m, false).await?;
        // Two copies of the event: with the link back for people who can read
        // the source, without it for everyone else.
        let mut readers = vec![];
        let mut others = vec![];
        for v in recipients {
            if text(c, v, source_channel, Permissions::READ_HISTORY)
                .await
                .is_ok()
            {
                readers.push(v);
            } else {
                others.push(v);
            }
        }
        let author = user(c, u).await?;
        for (users, linked) in [(readers, true), (others, false)] {
            if users.is_empty() {
                continue;
            }
            let mut copy = message.clone();
            if let (Some(f), false) = (copy.extra.forwarded.as_mut(), linked) {
                f.source = None;
            }
            out.push((
                users,
                Event::MessageCreated {
                    message: copy,
                    author: author.clone(),
                },
            ));
        }
        crate::commands::reads_event(c, out, ch).await?;
        sent.push(message);
    }
    Ok(ContentResult::Forwarded(sent))
}
