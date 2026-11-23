//! Rich message content: polls, forwarding, scheduled messages, link previews,
//! attachment descriptions/spoilers and voice messages.
//! Wire contracts: `sticktalk_proto::content`. Behavior: `docs/features/content.md`.
//!
//! Timing uses one task (`spawn`) that sleeps until the next poll deadline or
//! scheduled message, whichever is first, and is woken when either changes.
//! Nothing is polled; with no open poll and nothing scheduled it only waits.
mod forward;
mod polls;
mod previews;
mod routes;
mod scheduled;
#[cfg(test)]
mod tests;

pub(crate) use routes::routes;

use crate::{
    db::*,
    error::{require, Error, Result},
    state::{now, App, Core},
};
use sqlx::{Row, SqliteConnection};
use sticktalk_proto::*;
pub(crate) type Deliveries = Vec<(Vec<Id>, Event)>;

#[derive(clap::Args, Clone, Debug)]
pub struct ContentOptions {
    /// Fetch website previews for links in messages. This makes outbound HTTP(S)
    /// requests to the linked sites; set it to false to turn previews off entirely.
    #[arg(
        long = "link-previews",
        env = "STICKTALK_LINK_PREVIEWS",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    pub link_previews: bool,
    /// Local browser tests only (requires a loopback bind): previews resolve
    /// this one host name to 127.0.0.1 and resolve nothing else.
    #[arg(long, env = "STICKTALK_MOCK_LINK_PREVIEW_HOST", hide = true)]
    pub mock_link_preview_host: Option<String>,
}
impl Default for ContentOptions {
    fn default() -> Self {
        Self {
            link_previews: true,
            mock_link_preview_host: None,
        }
    }
}

pub(crate) struct Content {
    /// Wakes the timer task after a poll or scheduled message changes.
    wake: tokio::sync::Notify,
    previews: previews::Previews,
}
impl Content {
    pub(crate) fn new(options: &ContentOptions) -> Self {
        Self {
            wake: tokio::sync::Notify::new(),
            previews: previews::Previews::new(options.link_previews),
        }
    }
    /// With the test-only mock host, which needs a loopback-only server.
    pub(crate) fn configured(options: &ContentOptions, bind: std::net::SocketAddr) -> Result<Self> {
        let content = Self::new(options);
        if let Some(host) = &options.mock_link_preview_host {
            require(
                bind.ip().is_loopback() && previews::public_name(host),
                ErrorCode::InvalidRequest,
            )?;
            content.previews.mock(host.to_ascii_lowercase());
        }
        Ok(content)
    }
}

/// Dispatch for `Command::Content`, inside the command transaction.
pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    cmd: ContentCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    use ContentCommand::*;
    let result = match cmd {
        // `commands::execute` marks the stream; there is nothing to store.
        Subscribe => ContentResult::Ack,
        SendMessage { draft, attachments } => {
            set_meta(c, u, &draft, &attachments).await?;
            return send(c, core, key, u, draft, out).await;
        }
        CreatePoll(draft) => return polls::create(c, core, key, u, draft, out).await,
        SendGif {
            channel_id,
            gif_id,
            url,
        } => return send_gif(c, core, key, u, channel_id, gif_id, url, out).await,
        VotePoll {
            channel_id,
            message_id,
            answers,
        } => polls::vote(c, u, channel_id, message_id, answers, out).await?,
        EndPoll {
            channel_id,
            message_id,
        } => polls::end(c, u, channel_id, message_id, out).await?,
        Forward {
            channel_id,
            message_id,
            destinations,
            comment,
        } => {
            core.rate(format!("send:{}", u.0), 60., 10.)?;
            forward::forward(
                c,
                core,
                u,
                (channel_id, message_id),
                destinations,
                comment,
                out,
            )
            .await?
        }
        RemoveEmbeds {
            channel_id,
            message_id,
        } => previews::remove(c, u, channel_id, message_id, out).await?,
        ScheduleMessage {
            channel_id,
            content,
            mention_ids,
            send_at,
            attachment_ids,
        } => {
            let draft = scheduled::Draft {
                channel_id,
                content,
                mention_ids,
                send_at,
                attachment_ids: Some(attachment_ids),
            };
            scheduled::create(c, core, u, draft, out).await?
        }
        UpdateScheduled {
            id,
            content,
            mention_ids,
            send_at,
            attachment_ids,
        } => {
            let draft = scheduled::Draft {
                channel_id: Id(0),
                content,
                mention_ids,
                send_at,
                attachment_ids,
            };
            scheduled::update(c, u, id, draft, out).await?
        }
        CancelScheduled { id } => scheduled::cancel(c, u, id, out).await?,
        SendScheduledNow { id } => return scheduled::send_now(c, core, key, u, id, out).await,
    };
    Ok(CommandResult::Content(result))
}

/// See `forward::snapshot_text`.
pub(crate) async fn forward_text(
    c: &mut SqliteConnection,
    u: Id,
    source_channel: Id,
    source_message: Id,
) -> Option<String> {
    forward::snapshot_text(c, u, source_channel, source_message).await
}

/// The ordinary send path (permissions, slowmode, mentions, moderation checks).
pub(crate) async fn send(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    draft: MessageDraft,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    core.rate(format!("send:{}", u.0), 60., 10.)?;
    Box::pin(crate::commands::mutate(
        c,
        core,
        key,
        u,
        Command::SendMessage(draft),
        out,
    ))
    .await
}

/// Descriptions, spoiler flags and voice-message details for the sender's own
/// not-yet-sent uploads in this draft.
async fn set_meta(
    c: &mut SqliteConnection,
    u: Id,
    draft: &MessageDraft,
    patches: &[AttachmentMetaPatch],
) -> Result<()> {
    require(
        patches.len() <= draft.attachment_ids.len(),
        ErrorCode::InvalidRequest,
    )?;
    let voice = patches.iter().filter(|p| p.meta.voice()).count();
    // A voice message is the recording alone, as in Discord.
    require(
        voice == 0 || (draft.attachment_ids.len() == 1 && draft.content.trim().is_empty()),
        ErrorCode::InvalidRequest,
    )?;
    for (i, p) in patches.iter().enumerate() {
        require(
            draft.attachment_ids.contains(&p.attachment_id)
                && !patches[..i]
                    .iter()
                    .any(|q| q.attachment_id == p.attachment_id),
            ErrorCode::InvalidRequest,
        )?;
        let meta = &p.meta;
        let description = match meta.description.as_deref().map(str::trim) {
            Some(d) if !d.is_empty() => {
                require(
                    d.chars().count() <= MAX_ATTACHMENT_DESCRIPTION_CHARS
                        && !d.chars().any(|c| c.is_control() && c != '\n'),
                    ErrorCode::InvalidRequest,
                )?;
                Some(d.to_owned())
            }
            _ => None,
        };
        let row = sqlx::query("SELECT content_type FROM uploads WHERE id=? AND owner_id=? AND purpose='message' AND channel_id IS ? AND expires_at>?")
            .bind(id(p.attachment_id)?)
            .bind(id(u)?)
            .bind(id(draft.channel_id)?)
            .bind(now() as i64)
            .fetch_optional(&mut *c)
            .await?
            .ok_or_else(|| Error::new(ErrorCode::InvalidAttachment))?;
        if let Some(ms) = meta.duration_ms {
            let kind: String = row.get("content_type");
            require(
                (1..=MAX_VOICE_MESSAGE_MS).contains(&ms)
                    && !meta.waveform.is_empty()
                    && meta.waveform.len() <= MAX_VOICE_WAVEFORM_SAMPLES
                    && matches!(kind.as_str(), "audio/ogg" | "video/webm" | "video/mp4"),
                ErrorCode::InvalidRequest,
            )?;
        } else {
            require(meta.waveform.is_empty(), ErrorCode::InvalidRequest)?;
        }
        sqlx::query(
            "UPDATE uploads SET description=?,spoiler=?,duration_ms=?,waveform=? WHERE id=?",
        )
        .bind(description)
        .bind(meta.spoiler)
        .bind(meta.duration_ms.map(i64::from))
        .bind((!meta.waveform.is_empty()).then_some(meta.waveform.as_slice()))
        .bind(id(p.attachment_id)?)
        .execute(&mut *c)
        .await?;
    }
    Ok(())
}

/// Attachment details stored with an upload row (`SELECT *` from uploads).
pub(crate) fn attachment_meta(r: &sqlx::sqlite::SqliteRow) -> AttachmentMeta {
    AttachmentMeta {
        description: r.try_get("description").ok().flatten(),
        spoiler: r.try_get::<i64, _>("spoiler").unwrap_or(0) != 0,
        duration_ms: r
            .try_get::<Option<i64>, _>("duration_ms")
            .ok()
            .flatten()
            .map(|v| v as u32),
        waveform: r
            .try_get::<Option<Vec<u8>>, _>("waveform")
            .ok()
            .flatten()
            .unwrap_or_default(),
    }
}

/// Fills in `Message::extra` for one viewer (their own votes; the forward's
/// source only if they may read it).
pub(crate) async fn decorate(c: &mut SqliteConnection, viewer: Id, m: &mut Message) -> Result<()> {
    m.extra.poll = polls::load(c, m.id, viewer).await?.map(Box::new);
    m.extra.forwarded = forward::load(c, viewer, m.id).await?.map(Box::new);
    m.extra.embeds = previews::load(c, m.channel_id, m.id).await?;
    m.extra.gif = sqlx::query_scalar("SELECT gif_id FROM message_gifs WHERE message_id=?")
        .bind(id(m.id)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(())
}

/// The provider's media hosts; GIF URLs must be on one of them.
pub(crate) fn gif_media(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    url.len() <= 2048
        && rest.len() > host.len() + 1
        && matches!(
            host,
            "static.klipy.com" | "static1.klipy.com" | "static2.klipy.com"
        )
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

#[allow(clippy::too_many_arguments)]
async fn send_gif(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    ch: Id,
    gif_id: String,
    url: String,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    let provider_id = gif_id.strip_prefix("klipy:").filter(|v| {
        (1..=128).contains(&v.len())
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    });
    require(
        provider_id.is_some() && gif_media(&url),
        ErrorCode::InvalidRequest,
    )?;
    let draft = MessageDraft {
        channel_id: ch,
        content: url,
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    };
    let CommandResult::Message(mut message) = send(c, core, key, u, draft, out).await? else {
        return Err(Error::new(ErrorCode::Internal));
    };
    sqlx::query("INSERT INTO message_gifs(message_id,gif_id) VALUES(?,?)")
        .bind(id(message.id)?)
        .bind(&gif_id)
        .execute(&mut *c)
        .await?;
    message.extra.gif = Some(gif_id);
    for (_, e) in out.iter_mut() {
        if let Event::MessageCreated { message: m, .. } = e {
            if m.id == message.id {
                m.extra.gif = message.extra.gif.clone();
            }
        }
    }
    Ok(CommandResult::Message(message))
}

/// Message text with `<@id>` mentions written out as "@name" (crossposts).
pub(crate) async fn plain_text(c: &mut SqliteConnection, content: &str) -> Result<String> {
    forward::plain_mentions(c, content).await
}

/// Whether someone may download a stored file because a forwarded copy of it
/// is in a conversation they can read.
pub(crate) async fn forwarded_file_access(
    c: &mut SqliteConnection,
    u: Id,
    attachment: Id,
) -> Result<bool> {
    forward::file_access(c, u, attachment).await
}

/// Recognizes Ogg Opus recordings (voice messages), from the upload prefix.
pub(crate) fn audio_mime(head: &[u8]) -> Option<&'static str> {
    (head.len() >= 36 && head.starts_with(b"OggS") && &head[28..36] == b"OpusHead")
        .then_some("audio/ogg")
}

/// Called after a command commits, for each delivery: starts link previews
/// for new or edited messages and re-arms the timer when deadlines change.
pub(crate) fn observe(app: &App, e: &Event) {
    match e {
        Event::MessageCreated { message, .. } => {
            if message.extra.poll.is_some() {
                app.0.content.wake.notify_one();
            }
            previews::queue(app, message);
        }
        Event::MessageUpdated(message) => previews::queue(app, message),
        Event::Content(ContentEvent::ScheduledUpserted(_)) => app.0.content.wake.notify_one(),
        _ => {}
    }
}

/// Starts the content timer: poll deadlines and scheduled deliveries.
pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            let next = match next_deadline(&app).await {
                Ok(next) => next,
                Err(_) => Some(now() + 30_000),
            };
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
                _ = app.0.content.wake.notified() => continue,
                _ = sleep => {}
            }
            let _ = polls::close_due(&app).await;
            let _ = scheduled::deliver_due(&app).await;
        }
    });
}

async fn next_deadline(app: &App) -> Result<Option<u64>> {
    let mut c = app.0.pool.acquire().await?;
    let poll: Option<i64> =
        sqlx::query_scalar("SELECT MIN(expires_at) FROM polls WHERE closed_at IS NULL")
            .fetch_one(&mut *c)
            .await?;
    let scheduled: Option<i64> =
        sqlx::query_scalar("SELECT MIN(send_at) FROM scheduled_messages WHERE failure IS NULL")
            .fetch_one(&mut *c)
            .await?;
    Ok([poll, scheduled]
        .into_iter()
        .flatten()
        .map(|v| v.max(0) as u64)
        .min())
}

/// Emits committed deliveries the way `commands::execute` does, for work the
/// server starts itself (deadlines, scheduled sends, finished previews).
pub(crate) async fn deliver(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    out: Deliveries,
) -> Result<()> {
    for (users, e) in out {
        if let Event::MessageCreated { ref message, .. } = e {
            crate::push::message(app, c, &users, message).await;
            crate::mentions::notify(app, core, c, message).await?;
        }
        observe(app, &e);
        let alert = matches!(e, Event::MessageCreated { .. }).then(|| e.clone());
        for (users, e) in crate::launch::filter_event(c, users, e).await? {
            core.emit(&users, e)?;
        }
        if let Some(e) = alert {
            crate::privacy::observe(app, core, c, &e).await;
        }
    }
    Ok(())
}

/// Plain text with no control characters, trimmed, at most `max` characters.
fn clean(s: &str, max: usize) -> Result<String> {
    let s = s.trim();
    require(
        !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )?;
    Ok(s.to_owned())
}

/// Someone may moderate messages here: Manage Messages in a server channel.
async fn moderator(c: &mut SqliteConnection, u: Id, ch: &Channel) -> Result<bool> {
    Ok(match ch.summary.guild_id {
        Some(g) => crate::permissions::evaluate(c, u, g, Some(ch.summary.id))
            .await?
            .contains(Permissions::MANAGE_MESSAGES),
        None => false,
    })
}
