//! Forum channels: settings, tags, posts and the post list.
use super::*;

pub(super) async fn settings(c: &mut SqliteConnection, forum: Id) -> Result<ForumSettings> {
    let r = sqlx::query("SELECT * FROM forum_settings WHERE channel_id=?")
        .bind(id(forum)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let tags = sqlx::query(
        "SELECT id,name,emoji,moderated FROM forum_tags WHERE forum_id=? ORDER BY position,id",
    )
    .bind(id(forum)?)
    .fetch_all(&mut *c)
    .await?
    .into_iter()
    .map(|t| ForumTag {
        id: t.get::<i64, _>("id") as u32,
        name: t.get("name"),
        emoji: t.get("emoji"),
        moderated: t.get::<i64, _>("moderated") != 0,
    })
    .collect();
    Ok(ForumSettings {
        channel_id: forum,
        guidelines: r.get("guidelines"),
        tags,
        require_tags: r.get::<i64, _>("require_tags") != 0,
        default_reaction: r.get("default_reaction"),
        sort: en(r.get("sort"))?,
        layout: en(r.get("layout"))?,
        post_slowmode: r.get::<i64, _>("post_slowmode") as u32,
        reply_slowmode: r.get::<i64, _>("reply_slowmode") as u32,
        default_auto_archive_minutes: r.get::<i64, _>("default_auto_archive_minutes") as u32,
    })
}

/// The forum, if `u` can see it.
pub(super) async fn forum(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    p: Permissions,
) -> Result<Channel> {
    let channel = access(c, u, ch, Permissions::VIEW_CHANNEL | p).await?;
    require(
        channel.summary.kind == ChannelKind::Forum,
        ErrorCode::InvalidRequest,
    )?;
    Ok(channel)
}

fn emoji(s: &str) -> Result<String> {
    let s = s.trim();
    let custom = s
        .strip_prefix("<:")
        .and_then(|v| v.strip_suffix('>'))
        .and_then(|v| v.rsplit_once(':'))
        .is_some_and(|(name, n)| {
            !name.is_empty()
                && name.len() <= 32
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && n.parse::<u64>()
                    .is_ok_and(|n| n > 0 && n <= i64::MAX as u64)
        });
    use unicode_segmentation::UnicodeSegmentation;
    require(
        custom || (s.len() <= 32 && s.graphemes(true).count() == 1 && emojis::get(s).is_some()),
        ErrorCode::InvalidRequest,
    )?;
    Ok(s.to_owned())
}

pub(super) async fn update(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    patch: ForumSettingsPatch,
    out: &mut Deliveries,
) -> Result<ForumSettings> {
    forum(c, u, ch, Permissions::MANAGE_CHANNELS).await?;
    let mut s = settings(c, ch).await?;
    if let Some(g) = patch.guidelines {
        let g = g.trim().to_owned();
        require(
            g.chars().count() <= MAX_FORUM_GUIDELINES_CHARS
                && !g.chars().any(|c| c.is_control() && c != '\n'),
            ErrorCode::InvalidRequest,
        )?;
        s.guidelines = g;
    }
    if let Some(r) = patch.require_tags {
        s.require_tags = r;
    }
    if let Some(e) = patch.default_reaction {
        s.default_reaction = if e.trim().is_empty() {
            None
        } else {
            Some(emoji(&e)?)
        };
    }
    if let Some(v) = patch.sort {
        s.sort = v;
    }
    if let Some(v) = patch.layout {
        s.layout = v;
    }
    for (value, slot) in [
        (patch.post_slowmode, &mut s.post_slowmode),
        (patch.reply_slowmode, &mut s.reply_slowmode),
    ] {
        if let Some(v) = value {
            require(v <= MAX_FORUM_SLOWMODE_SECONDS, ErrorCode::InvalidRequest)?;
            *slot = v;
        }
    }
    if let Some(m) = patch.default_auto_archive_minutes {
        s.default_auto_archive_minutes = archive_minutes(m)?;
    }
    sqlx::query("UPDATE forum_settings SET guidelines=?,require_tags=?,default_reaction=?,sort=?,layout=?,post_slowmode=?,reply_slowmode=?,default_auto_archive_minutes=? WHERE channel_id=?")
        .bind(&s.guidelines)
        .bind(s.require_tags)
        .bind(&s.default_reaction)
        .bind(enum_str(s.sort))
        .bind(enum_str(s.layout))
        .bind(i64::from(s.post_slowmode))
        .bind(i64::from(s.reply_slowmode))
        .bind(i64::from(s.default_auto_archive_minutes))
        .bind(id(ch)?)
        .execute(&mut *c)
        .await?;
    if let Some(tags) = patch.tags {
        require(tags.len() <= MAX_FORUM_TAGS, ErrorCode::LimitExceeded)?;
        let mut names = std::collections::HashSet::new();
        for t in &tags {
            let n = t.name.trim();
            require(
                !n.is_empty()
                    && n.chars().count() <= MAX_FORUM_TAG_CHARS
                    && !n.chars().any(char::is_control)
                    && names.insert(n.to_lowercase())
                    && (t.id == 0 || s.tags.iter().any(|o| o.id == t.id)),
                ErrorCode::InvalidRequest,
            )?;
            if let Some(e) = &t.emoji {
                if !e.trim().is_empty() {
                    emoji(e)?;
                }
            }
        }
        let kept: Vec<u32> = tags.iter().map(|t| t.id).filter(|i| *i != 0).collect();
        require(
            kept.len() == kept.iter().collect::<std::collections::HashSet<_>>().len(),
            ErrorCode::InvalidRequest,
        )?;
        for old in &s.tags {
            if !kept.contains(&old.id) {
                sqlx::query("DELETE FROM forum_tags WHERE forum_id=? AND id=?")
                    .bind(id(ch)?)
                    .bind(i64::from(old.id))
                    .execute(&mut *c)
                    .await?;
                sqlx::query("DELETE FROM thread_tags WHERE tag_id=? AND thread_id IN (SELECT channel_id FROM threads WHERE parent_id=?)")
                    .bind(i64::from(old.id))
                    .bind(id(ch)?)
                    .execute(&mut *c)
                    .await?;
            }
        }
        for (position, t) in tags.iter().enumerate() {
            let emoji = t.emoji.as_deref().map(str::trim).filter(|e| !e.is_empty());
            let tag_id = if t.id == 0 {
                let next: i64 = sqlx::query_scalar(
                    "UPDATE forum_settings SET next_tag_id=next_tag_id+1 WHERE channel_id=? RETURNING next_tag_id-1",
                )
                .bind(id(ch)?)
                .fetch_one(&mut *c)
                .await?;
                next
            } else {
                i64::from(t.id)
            };
            sqlx::query("INSERT INTO forum_tags(forum_id,id,name,emoji,moderated,position) VALUES(?,?,?,?,?,?) ON CONFLICT(forum_id,id) DO UPDATE SET name=excluded.name,emoji=excluded.emoji,moderated=excluded.moderated,position=excluded.position")
                .bind(id(ch)?)
                .bind(tag_id)
                .bind(t.name.trim())
                .bind(emoji)
                .bind(t.moderated)
                .bind(position as i64)
                .execute(&mut *c)
                .await?;
        }
    }
    let s = settings(c, ch).await?;
    out.push((
        audience(c, ch).await?,
        Event::Threads(ThreadEvent::ForumUpdated(s.clone())),
    ));
    Ok(s)
}

/// A post's tags: at most five, all from its forum; moderated tags are added or
/// removed only with Manage Threads; a forum that requires tags gets one.
pub(super) async fn check_tags(
    c: &mut SqliteConnection,
    forum: Id,
    old: &[u32],
    new: &[u32],
    manage: bool,
) -> Result<()> {
    let s = settings(c, forum).await?;
    require(new.len() <= MAX_TAGS_PER_POST, ErrorCode::InvalidRequest)?;
    let mut seen = std::collections::HashSet::new();
    for t in new {
        require(
            seen.insert(*t) && s.tags.iter().any(|f| f.id == *t),
            ErrorCode::InvalidRequest,
        )?;
    }
    for tag in s.tags.iter().filter(|t| t.moderated) {
        if old.contains(&tag.id) != new.contains(&tag.id) {
            require(manage, ErrorCode::Forbidden)?;
        }
    }
    require(
        !s.require_tags || !new.is_empty() || manage,
        ErrorCode::InvalidRequest,
    )
}

pub(super) async fn set_tags(c: &mut SqliteConnection, t: Id, tags: &[u32]) -> Result<()> {
    sqlx::query("DELETE FROM thread_tags WHERE thread_id=?")
        .bind(id(t)?)
        .execute(&mut *c)
        .await?;
    for tag in tags {
        sqlx::query("INSERT INTO thread_tags(thread_id,tag_id) VALUES(?,?)")
            .bind(id(t)?)
            .bind(i64::from(*tag))
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}

fn slowed(seconds: u32, last: Option<i64>) -> Result<()> {
    let Some(last) = last else { return Ok(()) };
    let wait = (last.max(0) as u64 + u64::from(seconds) * 1000).saturating_sub(now());
    if seconds > 0 && wait > 0 {
        let mut e = Error::rate(wait.min(u64::from(u32::MAX)) as u32);
        e.0.field = Some("slowmode".into());
        e.0.message = "Slowmode is enabled in this forum.".into();
        return Err(e);
    }
    Ok(())
}

pub(super) async fn reply_slowmode(
    c: &mut SqliteConnection,
    u: Id,
    post: &ThreadInfo,
) -> Result<()> {
    let seconds = settings(c, post.parent_id).await?.reply_slowmode;
    if seconds == 0 {
        return Ok(());
    }
    let last = sqlx::query_scalar("SELECT created_at FROM messages WHERE author_id=? AND channel_id=? ORDER BY id DESC LIMIT 1")
        .bind(id(u)?)
        .bind(id(post.id)?)
        .fetch_optional(&mut *c)
        .await?;
    slowed(seconds, last)
}

pub(super) async fn create_post(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    post: NewPost,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    let ch = forum(c, u, post.forum_id, Permissions::SEND_MESSAGES).await?;
    let g = ch
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    let p = crate::permissions::evaluate(c, u, g, Some(post.forum_id)).await?;
    let manage = p.intersects(
        Permissions::MANAGE_THREADS | Permissions::MANAGE_CHANNELS | Permissions::MANAGE_MESSAGES,
    );
    let s = settings(c, post.forum_id).await?;
    let title = thread_name(&post.title)?;
    check_tags(
        c,
        post.forum_id,
        &[],
        &post.tag_ids,
        p.contains(Permissions::MANAGE_THREADS),
    )
    .await?;
    require(
        !post.content.trim().is_empty() || !post.attachment_ids.is_empty(),
        ErrorCode::InvalidRequest,
    )?;
    let minutes = archive_minutes(
        post.auto_archive_minutes
            .unwrap_or(s.default_auto_archive_minutes),
    )?;
    if !manage && s.post_slowmode > 0 {
        let last = sqlx::query_scalar(
            "SELECT MAX(created_at) FROM threads WHERE parent_id=? AND owner_id=?",
        )
        .bind(id(post.forum_id)?)
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
        slowed(s.post_slowmode, last)?;
    }
    active_room(c, g).await?;
    unique(&post.attachment_ids, MAX_MESSAGE_ATTACHMENTS)?;
    let t = core.id().await?;
    let at = now();
    sqlx::query("INSERT INTO channels(id,kind,guild_id,parent_id,name,position,created_at) VALUES(?,'thread',?,?,?,0,?)")
        .bind(id(t)?)
        .bind(id(g)?)
        .bind(id(post.forum_id)?)
        .bind(&title)
        .bind(at as i64)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT INTO threads(channel_id,parent_id,guild_id,owner_id,forum_post,auto_archive_minutes,created_at,last_activity_at,archive_at) VALUES(?,?,?,?,1,?,?,?,?)")
        .bind(id(t)?)
        .bind(id(post.forum_id)?)
        .bind(id(g)?)
        .bind(id(u)?)
        .bind(i64::from(minutes))
        .bind(at as i64)
        .bind(at as i64)
        .bind((at + u64::from(minutes) * 60_000) as i64)
        .execute(&mut *c)
        .await?;
    set_tags(c, t, &post.tag_ids).await?;
    // Files uploaded to the forum for this post move into it.
    for a in &post.attachment_ids {
        let moved = sqlx::query("UPDATE uploads SET channel_id=? WHERE id=? AND owner_id=? AND purpose='message' AND channel_id=? AND expires_at>?")
            .bind(id(t)?)
            .bind(id(*a)?)
            .bind(id(u)?)
            .bind(id(post.forum_id)?)
            .bind(now() as i64)
            .execute(&mut *c)
            .await?
            .rows_affected();
        require(moved == 1, ErrorCode::InvalidAttachment)?;
    }
    join(c, out, t, u).await?;
    let draft = MessageDraft {
        channel_id: t,
        content: post.content,
        reply_to: None,
        attachment_ids: post.attachment_ids,
        mention_ids: post.mention_ids,
    };
    // The ordinary send path: permissions, mentions, moderation checks.
    let sent = if post.attachments.is_empty() {
        // Boxed: the send path is deep; inline, its future made every frame
        // above it larger (docs/features/perf.md, stack depth).
        Box::pin(crate::content::send(c, core, key, u, draft, out)).await?
    } else {
        Box::pin(crate::content::mutate(
            c,
            core,
            key,
            u,
            ContentCommand::SendMessage {
                draft,
                attachments: post.attachments,
            },
            out,
        ))
        .await?
    };
    let CommandResult::Message(message) = sent else {
        return Err(Error::new(ErrorCode::Internal));
    };
    let thread = load(c, t)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::Internal))?;
    Ok(CommandResult::Threads(ThreadResult::Post {
        thread: Box::new(thread),
        message: Box::new(message),
    }))
}

#[derive(serde::Deserialize)]
pub(super) struct PostQuery {
    pub(super) sort: Option<ForumSort>,
    /// Comma-separated tag IDs; a post with any of them matches.
    pub(super) tags: Option<String>,
    pub(super) q: Option<String>,
    pub(super) before: Option<u64>,
    pub(super) limit: Option<u16>,
}

/// The forum view: pinned post first (on the first page), then by the chosen
/// order. Search matches the title or the first message.
pub(super) async fn posts(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    q: PostQuery,
) -> Result<ForumPage> {
    forum(c, u, ch, Permissions::READ_HISTORY).await?;
    let s = settings(c, ch).await?;
    let sort = q.sort.unwrap_or(s.sort);
    let limit = q.limit.unwrap_or(25);
    require(
        (1..=MAX_THREADS_PAGE).contains(&limit),
        ErrorCode::InvalidRequest,
    )?;
    let key = match sort {
        ForumSort::Activity => "t.last_activity_at",
        ForumSort::Created => "t.created_at",
    };
    let tags: Vec<i64> = match &q.tags {
        Some(list) if !list.is_empty() => list
            .split(',')
            .map(|v| v.parse::<u32>().map(i64::from))
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?,
        _ => vec![],
    };
    require(tags.len() <= MAX_FORUM_TAGS, ErrorCode::InvalidRequest)?;
    let words: Vec<String> =
        q.q.as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .collect();
    require(
        words.len() <= 8 && words.iter().all(|w| w.chars().count() <= 64),
        ErrorCode::InvalidRequest,
    )?;
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(SELECT);
    query.push(" WHERE t.parent_id=").push_bind(id(ch)?);
    if !tags.is_empty() {
        query.push(" AND EXISTS(SELECT 1 FROM thread_tags tt WHERE tt.thread_id=t.channel_id AND tt.tag_id IN (");
        let mut list = query.separated(",");
        for t in &tags {
            list.push_bind(*t);
        }
        query.push("))");
    }
    for w in &words {
        query
            .push(" AND (instr(lower(ch.name),")
            .push_bind(w.clone())
            .push(")>0 OR t.first_message_id IN (SELECT rowid FROM message_fts WHERE message_fts MATCH ")
            .push_bind(format!("\"{}\"*", w.replace('"', "\"\"")))
            .push("))");
    }
    let first_page = q.before.is_none();
    if let Some(b) = q.before {
        query
            .push(format!(" AND t.pinned=0 AND {key}<"))
            .push_bind(b.min(i64::MAX as u64) as i64);
    }
    query
        .push(format!(
            " ORDER BY t.pinned DESC, {key} DESC, t.channel_id DESC LIMIT "
        ))
        .push_bind(i64::from(limit) + 1);
    let r = query.build().fetch_all(&mut *c).await?;
    let mut list = rows(c, r).await?;
    let has_more = list.len() > usize::from(limit);
    list.truncate(usize::from(limit));
    if !first_page {
        list.retain(|t| !t.pinned);
    }
    let next = if has_more {
        list.last().map(|t| match sort {
            ForumSort::Activity => t.last_activity_at,
            ForumSort::Created => t.created_at,
        })
    } else {
        None
    };
    let mut page = ForumPage {
        has_more,
        next,
        ..Default::default()
    };
    let mut users = std::collections::BTreeMap::new();
    for t in list {
        let first = match t.first_message_id {
            Some(m) => message(c, u, t.id, m, false).await.ok(),
            None => None,
        };
        let reaction_count = match &first {
            Some(m) => m.reactions.iter().map(|r| r.count).sum(),
            None => 0,
        };
        if let std::collections::btree_map::Entry::Vacant(v) = users.entry(t.owner_id) {
            v.insert(user(c, t.owner_id).await?);
        }
        page.posts.push(ForumPost {
            thread: t,
            first,
            reaction_count,
        });
    }
    page.users = users.into_values().collect();
    Ok(page)
}
