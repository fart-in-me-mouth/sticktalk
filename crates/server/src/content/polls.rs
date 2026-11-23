//! Polls. A poll is a message whose text is the question; answers and votes
//! live in their own tables. Votes are checked here, never trusted from clients.
use super::*;

fn valid_emoji(e: &str) -> bool {
    if custom_emoji_id(e).is_some() {
        return true;
    }
    // One emoji: a short run of non-ASCII, non-control characters.
    !e.is_empty()
        && e.len() <= 32
        && e.chars().count() <= 8
        && !e
            .chars()
            .any(|c| c.is_ascii() || c.is_control() || c.is_whitespace())
}

pub(super) async fn create(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    d: PollDraft,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    let question = clean(&d.question, MAX_POLL_QUESTION_CHARS)?;
    require(
        (MIN_POLL_ANSWERS..=MAX_POLL_ANSWERS).contains(&d.answers.len())
            && POLL_DURATION_HOURS.contains(&d.duration_hours),
        ErrorCode::InvalidRequest,
    )?;
    let mut answers = vec![];
    for a in &d.answers {
        let text = clean(&a.text, MAX_POLL_ANSWER_CHARS)?;
        let emoji = match a.emoji.as_deref().map(str::trim) {
            Some(e) if !e.is_empty() => {
                require(valid_emoji(e), ErrorCode::InvalidRequest)?;
                Some(e.to_owned())
            }
            _ => None,
        };
        answers.push((text, emoji));
    }
    // The question is the message text, so search, notifications, replies and
    // older apps all show it. The ordinary send path checks permissions.
    let draft = MessageDraft {
        channel_id: d.channel_id,
        content: question.clone(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    };
    let CommandResult::Message(mut message) = send(c, core, key, u, draft, out).await? else {
        return Err(Error::new(ErrorCode::Internal));
    };
    let expires = now() + u64::from(d.duration_hours) * 3_600_000;
    sqlx::query("INSERT INTO polls(message_id,question,multiple,expires_at) VALUES(?,?,?,?)")
        .bind(id(message.id)?)
        .bind(&question)
        .bind(d.multiple)
        .bind(expires as i64)
        .execute(&mut *c)
        .await?;
    for (i, (text, emoji)) in answers.iter().enumerate() {
        sqlx::query("INSERT INTO poll_answers(message_id,idx,text,emoji) VALUES(?,?,?,?)")
            .bind(id(message.id)?)
            .bind(i as i64)
            .bind(text)
            .bind(emoji)
            .execute(&mut *c)
            .await?;
    }
    message.extra.poll = load(c, message.id, u).await?;
    for (_, e) in out.iter_mut() {
        if let Event::MessageCreated { message: m, .. } = e {
            if m.id == message.id {
                m.extra.poll = message.extra.poll.clone();
            }
        }
    }
    Ok(CommandResult::Message(message))
}

/// The poll on a message, as `viewer` sees it (with their own votes).
pub(super) async fn load(c: &mut SqliteConnection, m: Id, viewer: Id) -> Result<Option<Poll>> {
    let Some(r) =
        sqlx::query("SELECT question,multiple,expires_at,closed_at FROM polls WHERE message_id=?")
            .bind(id(m)?)
            .fetch_optional(&mut *c)
            .await?
    else {
        return Ok(None);
    };
    let expires_at = r.get::<i64, _>("expires_at").max(0) as u64;
    let answers = sqlx::query("SELECT a.text,a.emoji,(SELECT COUNT(*) FROM poll_votes v WHERE v.message_id=a.message_id AND v.idx=a.idx) AS votes FROM poll_answers a WHERE a.message_id=? ORDER BY a.idx")
        .bind(id(m)?)
        .fetch_all(&mut *c)
        .await?
        .iter()
        .map(|a| PollAnswer {
            text: a.get("text"),
            emoji: a.get("emoji"),
            votes: a.get::<i64, _>("votes") as u32,
        })
        .collect();
    let voters: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT user_id) FROM poll_votes WHERE message_id=?")
            .bind(id(m)?)
            .fetch_one(&mut *c)
            .await?;
    let my_votes = sqlx::query_scalar::<_, i64>(
        "SELECT idx FROM poll_votes WHERE message_id=? AND user_id=? ORDER BY idx",
    )
    .bind(id(m)?)
    .bind(id(viewer)?)
    .fetch_all(&mut *c)
    .await?
    .into_iter()
    .map(|v| v as u8)
    .collect();
    Ok(Some(Poll {
        question: r.get("question"),
        answers,
        multiple: r.get::<i64, _>("multiple") != 0,
        expires_at,
        closed: r.get::<Option<i64>, _>("closed_at").is_some() || expires_at <= now(),
        voters: voters as u32,
        my_votes,
    }))
}

/// The poll's channel row, checked: the message exists in `ch`, is not deleted
/// and `u` may read it.
async fn readable(c: &mut SqliteConnection, u: Id, ch: Id, m: Id) -> Result<(Channel, Id)> {
    let channel = text(c, u, ch, Permissions::READ_HISTORY).await?;
    let author: i64 = sqlx::query_scalar("SELECT m.author_id FROM polls p JOIN messages m ON m.id=p.message_id WHERE p.message_id=? AND m.channel_id=? AND m.deleted_at IS NULL")
        .bind(id(m)?)
        .bind(id(ch)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok((channel, Id(author as u64)))
}

fn broadcast(mut poll: Poll) -> Poll {
    poll.my_votes.clear();
    poll
}

pub(super) async fn vote(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    m: Id,
    mut answers: Vec<u8>,
    out: &mut Deliveries,
) -> Result<ContentResult> {
    readable(c, u, ch, m).await?;
    let poll = load(c, m, u)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    require(!poll.closed, ErrorCode::Conflict)?;
    answers.sort_unstable();
    answers.dedup();
    require(
        answers.iter().all(|a| usize::from(*a) < poll.answers.len())
            && (poll.multiple || answers.len() <= 1),
        ErrorCode::InvalidRequest,
    )?;
    sqlx::query("DELETE FROM poll_votes WHERE message_id=? AND user_id=?")
        .bind(id(m)?)
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    for a in &answers {
        sqlx::query("INSERT INTO poll_votes(message_id,user_id,idx,created_at) VALUES(?,?,?,?)")
            .bind(id(m)?)
            .bind(id(u)?)
            .bind(i64::from(*a))
            .bind(now() as i64)
            .execute(&mut *c)
            .await?;
    }
    let poll = load(c, m, u)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::Internal))?;
    out.push((
        audience(c, ch).await?,
        Event::Content(ContentEvent::PollUpdated {
            channel_id: ch,
            message_id: m,
            poll: broadcast(poll.clone()),
            voter: Some(PollVoter {
                user_id: u,
                answers,
            }),
        }),
    ));
    Ok(ContentResult::Poll {
        channel_id: ch,
        message_id: m,
        poll,
    })
}

pub(super) async fn end(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    m: Id,
    out: &mut Deliveries,
) -> Result<ContentResult> {
    let (channel, author) = readable(c, u, ch, m).await?;
    require(
        author == u || moderator(c, u, &channel).await?,
        ErrorCode::Forbidden,
    )?;
    let closed = sqlx::query(
        "UPDATE polls SET closed_at=? WHERE message_id=? AND closed_at IS NULL AND expires_at>?",
    )
    .bind(now() as i64)
    .bind(id(m)?)
    .bind(now() as i64)
    .execute(&mut *c)
    .await?
    .rows_affected();
    require(closed == 1, ErrorCode::Conflict)?;
    let poll = load(c, m, u)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::Internal))?;
    out.push((
        audience(c, ch).await?,
        Event::Content(ContentEvent::PollUpdated {
            channel_id: ch,
            message_id: m,
            poll: broadcast(poll.clone()),
            voter: None,
        }),
    ));
    Ok(ContentResult::Poll {
        channel_id: ch,
        message_id: m,
        poll,
    })
}

/// Closes every poll whose deadline has passed and tells everyone who can see it.
pub(super) async fn close_due(app: &App) -> Result<()> {
    loop {
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let due = sqlx::query("SELECT p.message_id,m.channel_id,m.deleted_at FROM polls p JOIN messages m ON m.id=p.message_id WHERE p.closed_at IS NULL AND p.expires_at<=? ORDER BY p.expires_at LIMIT 32")
            .bind(now() as i64)
            .fetch_all(&mut *c)
            .await?;
        if due.is_empty() {
            return Ok(());
        }
        let mut out = vec![];
        for r in &due {
            let m = Id(r.get::<i64, _>("message_id") as u64);
            let ch = Id(r.get::<i64, _>("channel_id") as u64);
            sqlx::query("UPDATE polls SET closed_at=expires_at WHERE message_id=?")
                .bind(id(m)?)
                .execute(&mut *c)
                .await?;
            if r.get::<Option<i64>, _>("deleted_at").is_some() {
                continue;
            }
            let Some(poll) = load(&mut c, m, Id(1)).await? else {
                continue;
            };
            out.push((
                audience(&mut c, ch).await?,
                Event::Content(ContentEvent::PollUpdated {
                    channel_id: ch,
                    message_id: m,
                    poll: broadcast(poll),
                    voter: None,
                }),
            ));
        }
        deliver(app, &mut core, &mut c, out).await?;
    }
}

/// Who chose one answer, a page at a time (`after` is a user ID).
pub(super) async fn voters(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    m: Id,
    answer: u8,
    after: Option<Id>,
) -> Result<PollVotersPage> {
    readable(c, u, ch, m).await?;
    let ids = sqlx::query_scalar::<_, i64>("SELECT user_id FROM poll_votes WHERE message_id=? AND idx=? AND user_id>? ORDER BY user_id LIMIT 51")
        .bind(id(m)?)
        .bind(i64::from(answer))
        .bind(opt(after)?.unwrap_or(0))
        .fetch_all(&mut *c)
        .await?;
    let has_more = ids.len() > 50;
    let mut users = vec![];
    for v in ids.into_iter().take(50) {
        users.push(user(c, Id(v as u64)).await?);
    }
    Ok(PollVotersPage { users, has_more })
}
