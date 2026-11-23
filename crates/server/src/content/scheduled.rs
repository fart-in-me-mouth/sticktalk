//! Scheduled ("send later") messages. Stored in the database, so they survive a
//! restart; the content timer delivers each one at its time through the normal
//! send path, which re-checks permissions and slowmode then. A message that can
//! no longer be sent is kept with the reason, and the author is told.
use super::*;
use sqlx::Connection as _;

pub(crate) struct Draft {
    pub channel_id: Id,
    pub content: String,
    pub mention_ids: Vec<Id>,
    pub send_at: u64,
    /// `None` (an update from an older client) keeps the files held now.
    pub attachment_ids: Option<Vec<Id>>,
}

/// `files`: how many files the message will carry.
async fn validate(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    d: &Draft,
    files: usize,
) -> Result<String> {
    text(c, u, ch, Permissions::SEND_MESSAGES).await?;
    if files > 0 {
        text(c, u, ch, Permissions::ATTACH_FILES).await?;
    }
    let content = d.content.trim().to_owned();
    require(
        (!content.is_empty() || files > 0) && content.chars().count() <= MAX_MESSAGE_CHARS,
        ErrorCode::InvalidRequest,
    )?;
    require(
        d.send_at > now() && d.send_at <= now() + MAX_SCHEDULE_AHEAD_MS,
        ErrorCode::InvalidRequest,
    )?;
    unique(&d.mention_ids, MAX_MESSAGE_MENTIONS)?;
    for v in &d.mention_ids {
        text(c, *v, ch, Permissions::VIEW_CHANNEL)
            .await
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    }
    Ok(content)
}

/// The files held for scheduled message `s`, in order.
async fn held(c: &mut SqliteConnection, s: Id) -> Result<Vec<Attachment>> {
    Ok(sqlx::query("SELECT u.* FROM scheduled_attachments a JOIN uploads u ON u.id=a.attachment_id WHERE a.scheduled_id=? ORDER BY a.position")
        .bind(id(s)?)
        .fetch_all(&mut *c)
        .await?
        .iter()
        .map(attachment_row)
        .collect())
}

/// Holds `files` for scheduled message `s`: each must be the author's own
/// unattached, unexpired upload to channel `ch` (as when sending), and not
/// held by another scheduled message. Held files count toward the author's
/// upload quota like any upload. Files held before and left out are released:
/// they become ordinary unattached uploads and are collected as usual.
async fn hold(c: &mut SqliteConnection, u: Id, ch: Id, s: Id, files: &[Id]) -> Result<()> {
    unique(files, MAX_MESSAGE_ATTACHMENTS)?;
    let before: Vec<i64> =
        sqlx::query_scalar("SELECT attachment_id FROM scheduled_attachments WHERE scheduled_id=?")
            .bind(id(s)?)
            .fetch_all(&mut *c)
            .await?;
    for gone in before.iter().filter(|a| !files.contains(&Id(**a as u64))) {
        sqlx::query("DELETE FROM scheduled_attachments WHERE attachment_id=?")
            .bind(gone)
            .execute(&mut *c)
            .await?;
    }
    for (position, a) in files.iter().enumerate() {
        if before.contains(&(a.0 as i64)) {
            sqlx::query("UPDATE scheduled_attachments SET position=? WHERE attachment_id=?")
                .bind(position as i64)
                .bind(id(*a)?)
                .execute(&mut *c)
                .await?;
            continue;
        }
        let claimed = sqlx::query("UPDATE uploads SET expires_at=NULL WHERE id=? AND owner_id=? AND purpose='message' AND channel_id=? AND expires_at>? AND NOT EXISTS(SELECT 1 FROM message_attachments WHERE attachment_id=uploads.id) AND NOT EXISTS(SELECT 1 FROM scheduled_attachments WHERE attachment_id=uploads.id)")
            .bind(id(*a)?)
            .bind(id(u)?)
            .bind(id(ch)?)
            .bind(now() as i64)
            .execute(&mut *c)
            .await?
            .rows_affected();
        require(claimed == 1, ErrorCode::InvalidAttachment)?;
        sqlx::query(
            "INSERT INTO scheduled_attachments(scheduled_id,attachment_id,position) VALUES(?,?,?)",
        )
        .bind(id(s)?)
        .bind(id(*a)?)
        .bind(position as i64)
        .execute(&mut *c)
        .await?;
    }
    Ok(())
}

/// Hands the held files back to the send path, which attaches only
/// unexpired uploads: they get a short lease, then become the message's.
/// Runs in the delivery's transaction, so a refused delivery keeps them held.
async fn release_for_send(c: &mut SqliteConnection, s: Id) -> Result<Vec<Id>> {
    let files: Vec<i64> = sqlx::query_scalar(
        "SELECT attachment_id FROM scheduled_attachments WHERE scheduled_id=? ORDER BY position",
    )
    .bind(id(s)?)
    .fetch_all(&mut *c)
    .await?;
    sqlx::query("UPDATE uploads SET expires_at=? WHERE id IN (SELECT attachment_id FROM scheduled_attachments WHERE scheduled_id=?)")
        .bind((now() + 600_000) as i64)
        .bind(id(s)?)
        .execute(&mut *c)
        .await?;
    Ok(files.into_iter().map(|v| Id(v as u64)).collect())
}

fn row(r: &sqlx::sqlite::SqliteRow) -> ScheduledMessage {
    ScheduledMessage {
        id: Id(r.get::<i64, _>("id") as u64),
        channel_id: Id(r.get::<i64, _>("channel_id") as u64),
        content: r.get("content"),
        mention_ids: serde_json::from_str::<Vec<u64>>(&r.get::<String, _>("mention_ids"))
            .unwrap_or_default()
            .into_iter()
            .map(Id)
            .collect(),
        send_at: r.get::<i64, _>("send_at").max(0) as u64,
        created_at: r.get::<i64, _>("created_at").max(0) as u64,
        failure: r.get("failure"),
        attachments: vec![],
    }
}

async fn own(c: &mut SqliteConnection, u: Id, s: Id) -> Result<ScheduledMessage> {
    let mut m = sqlx::query("SELECT * FROM scheduled_messages WHERE id=? AND author_id=?")
        .bind(id(s)?)
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?
        .map(|r| row(&r))
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    m.attachments = held(c, s).await?;
    Ok(m)
}

fn mentions_json(ids: &[Id]) -> String {
    serde_json::to_string(&ids.iter().map(|v| v.0).collect::<Vec<_>>()).unwrap_or_default()
}

pub(super) async fn create(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    d: Draft,
    out: &mut Deliveries,
) -> Result<ContentResult> {
    let files = d.attachment_ids.clone().unwrap_or_default();
    let content = validate(c, u, d.channel_id, &d, files.len()).await?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scheduled_messages WHERE author_id=?")
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
    require(n < MAX_SCHEDULED_PER_USER as i64, ErrorCode::LimitExceeded)?;
    let s = core.id().await?;
    sqlx::query("INSERT INTO scheduled_messages(id,author_id,channel_id,content,mention_ids,send_at,created_at) VALUES(?,?,?,?,?,?,?)")
        .bind(id(s)?)
        .bind(id(u)?)
        .bind(id(d.channel_id)?)
        .bind(&content)
        .bind(mentions_json(&d.mention_ids))
        .bind(d.send_at as i64)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    hold(c, u, d.channel_id, s, &files).await?;
    let scheduled = own(c, u, s).await?;
    out.push((
        vec![u],
        Event::Content(ContentEvent::ScheduledUpserted(scheduled.clone())),
    ));
    Ok(ContentResult::Scheduled(scheduled))
}

/// Edit or reschedule; this also retries one that failed.
pub(super) async fn update(
    c: &mut SqliteConnection,
    u: Id,
    s: Id,
    d: Draft,
    out: &mut Deliveries,
) -> Result<ContentResult> {
    let old = own(c, u, s).await?;
    let files = d
        .attachment_ids
        .clone()
        .unwrap_or_else(|| old.attachments.iter().map(|a| a.id).collect());
    let content = validate(c, u, old.channel_id, &d, files.len()).await?;
    hold(c, u, old.channel_id, s, &files).await?;
    sqlx::query(
        "UPDATE scheduled_messages SET content=?,mention_ids=?,send_at=?,failure=NULL WHERE id=?",
    )
    .bind(&content)
    .bind(mentions_json(&d.mention_ids))
    .bind(d.send_at as i64)
    .bind(id(s)?)
    .execute(&mut *c)
    .await?;
    let scheduled = own(c, u, s).await?;
    out.push((
        vec![u],
        Event::Content(ContentEvent::ScheduledUpserted(scheduled.clone())),
    ));
    Ok(ContentResult::Scheduled(scheduled))
}

pub(super) async fn cancel(
    c: &mut SqliteConnection,
    u: Id,
    s: Id,
    out: &mut Deliveries,
) -> Result<ContentResult> {
    let old = own(c, u, s).await?;
    sqlx::query("DELETE FROM scheduled_messages WHERE id=?")
        .bind(id(s)?)
        .execute(&mut *c)
        .await?;
    out.push((
        vec![u],
        Event::Content(ContentEvent::ScheduledRemoved {
            id: s,
            channel_id: old.channel_id,
            sent: None,
        }),
    ));
    Ok(ContentResult::Ack)
}

fn draft(s: &ScheduledMessage, attachment_ids: Vec<Id>) -> MessageDraft {
    MessageDraft {
        channel_id: s.channel_id,
        content: s.content.clone(),
        reply_to: None,
        attachment_ids,
        mention_ids: s.mention_ids.clone(),
    }
}

pub(super) async fn send_now(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    s: Id,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    let old = own(c, u, s).await?;
    let files = release_for_send(c, s).await?;
    sqlx::query("DELETE FROM scheduled_messages WHERE id=?")
        .bind(id(s)?)
        .execute(&mut *c)
        .await?;
    let result = send(c, core, key, u, draft(&old, files), out).await?;
    let sent = match &result {
        CommandResult::Message(m) => Some(m.id),
        _ => None,
    };
    out.push((
        vec![u],
        Event::Content(ContentEvent::ScheduledRemoved {
            id: s,
            channel_id: old.channel_id,
            sent,
        }),
    ));
    Ok(result)
}

/// What the author is told when a scheduled message cannot be delivered.
fn failure(e: &Error) -> String {
    match e.0.code {
        ErrorCode::RateLimited if e.0.field.as_deref() == Some("slowmode") => {
            "Slowmode is on in that channel. Reschedule it or send it now.".into()
        }
        ErrorCode::Blocked => "You can't message this person.".into(),
        ErrorCode::NotFound | ErrorCode::Forbidden => {
            "You can no longer send messages there.".into()
        }
        _ => "It could not be sent.".into(),
    }
}

/// Sends every scheduled message that is due, oldest first.
pub(crate) async fn deliver_due(app: &App) -> Result<()> {
    loop {
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let due = sqlx::query("SELECT * FROM scheduled_messages WHERE failure IS NULL AND send_at<=? ORDER BY send_at,id LIMIT 16")
            .bind(now() as i64)
            .fetch_all(&mut *c)
            .await?;
        if due.is_empty() {
            return Ok(());
        }
        for r in &due {
            let s = row(r);
            let author = Id(r.get::<i64, _>("author_id") as u64);
            let mut out = vec![];
            // Timeout, verification level and AutoMod, exactly as for a live send.
            let files: Vec<Id> = sqlx::query_scalar::<_, i64>(
                "SELECT attachment_id FROM scheduled_attachments WHERE scheduled_id=? ORDER BY position",
            )
            .bind(id(s.id)?)
            .fetch_all(&mut *c)
            .await?
            .into_iter()
            .map(|v| Id(v as u64))
            .collect();
            let command = Command::SendMessage(draft(&s, files));
            let checked = crate::moderation::precheck(app, &mut core, &mut c, author, &command)
                .await
                .map(drop);
            let mut tx = c.begin().await?;
            release_for_send(&mut tx, s.id).await?;
            sqlx::query("DELETE FROM scheduled_messages WHERE id=?")
                .bind(id(s.id)?)
                .execute(&mut *tx)
                .await?;
            let sent = match checked {
                Ok(()) => {
                    crate::commands::mutate(&mut tx, &mut core, &[], author, command, &mut out)
                        .await
                }
                Err(e) => Err(e),
            };
            match sent {
                Ok(result) => {
                    let sent = match result {
                        CommandResult::Message(m) => Some(m.id),
                        _ => None,
                    };
                    out.push((
                        vec![author],
                        Event::Content(ContentEvent::ScheduledRemoved {
                            id: s.id,
                            channel_id: s.channel_id,
                            sent,
                        }),
                    ));
                    core.persist_id(&mut tx).await?;
                    tx.commit().await?;
                }
                Err(e) => {
                    drop(tx);
                    out.clear();
                    let reason = failure(&e);
                    sqlx::query("UPDATE scheduled_messages SET failure=? WHERE id=?")
                        .bind(&reason)
                        .bind(id(s.id)?)
                        .execute(&mut *c)
                        .await?;
                    out.push((
                        vec![author],
                        Event::Content(ContentEvent::ScheduledUpserted(ScheduledMessage {
                            failure: Some(reason),
                            ..s
                        })),
                    ));
                }
            }
            deliver(app, &mut core, &mut c, out).await?;
        }
    }
}

pub(super) async fn list(
    c: &mut SqliteConnection,
    u: Id,
    ch: Option<Id>,
) -> Result<ScheduledMessagesPage> {
    let rows = sqlx::query("SELECT * FROM scheduled_messages WHERE author_id=? AND (? IS NULL OR channel_id=?) ORDER BY send_at,id LIMIT 50")
        .bind(id(u)?)
        .bind(opt(ch)?)
        .bind(opt(ch)?)
        .fetch_all(&mut *c)
        .await?;
    Ok(ScheduledMessagesPage {
        items: rows.iter().map(row).collect(),
    })
}
