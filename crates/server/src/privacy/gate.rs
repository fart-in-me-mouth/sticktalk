//! The last check before an event reaches anyone (`launch::filter_event`):
//!
//! - a message request's recipient gets nothing from that conversation until
//!   they accept it;
//! - in an age-restricted channel, someone who has not confirmed their age
//!   gets no message content (new messages, edits, reactions, typing, poll
//!   tallies or link previews); deletions, read states and counts still come;
//! - typing from someone you ignore is not shown to you.
//!
//! History, pins, saved messages, search, the Inbox and push check `age` too.
use crate::{db::id, error::Result};
use sqlx::SqliteConnection;
use std::collections::HashSet;
use sticktalk_proto::*;

fn channel_of(e: &Event) -> Option<Id> {
    Some(match e {
        Event::MessageCreated { message, .. } | Event::MessageUpdated(message) => {
            message.channel_id
        }
        Event::MessageDeleted { channel_id, .. }
        | Event::Typing { channel_id, .. }
        | Event::PinsUpdated { channel_id, .. } => *channel_id,
        Event::ReadStateUpdated(r) => r.channel_id,
        Event::ReactionUpdated(r) => r.channel_id,
        Event::ChannelUpsert(ch) => ch.summary.id,
        Event::Content(ContentEvent::PollUpdated { channel_id, .. })
        | Event::Content(ContentEvent::EmbedsUpdated { channel_id, .. }) => *channel_id,
        Event::Appearance(AppearanceEvent::Tts { channel_id, .. }) => *channel_id,
        _ => return None,
    })
}
/// Events that carry what was said.
fn carries_content(e: &Event) -> bool {
    matches!(
        e,
        Event::MessageCreated { .. }
            | Event::MessageUpdated(_)
            | Event::ReactionUpdated(_)
            | Event::Typing { .. }
            | Event::Content(ContentEvent::PollUpdated { .. })
            | Event::Content(ContentEvent::EmbedsUpdated { .. })
            | Event::Appearance(AppearanceEvent::Tts { .. })
    )
}
pub(crate) async fn filter(c: &mut SqliteConnection, users: &mut Vec<Id>, e: &Event) -> Result<()> {
    if users.is_empty() {
        return Ok(());
    }
    let Some(ch) = channel_of(e) else {
        return Ok(());
    };
    let guild: Option<Option<i64>> = sqlx::query_scalar("SELECT guild_id FROM channels WHERE id=?")
        .bind(id(ch)?)
        .fetch_optional(&mut *c)
        .await?;
    match guild.flatten() {
        None => {
            let hidden = super::requests::hidden(c, ch).await?;
            if !hidden.is_empty() {
                // The sender's own echo still goes to the sender.
                users.retain(|u| !hidden.contains(u));
            }
        }
        Some(_) if carries_content(e) => {
            // Restricted itself or by a channel it is nested in (a thread).
            crate::moderation::age::retain_confirmed(c, ch, users).await?;
        }
        Some(_) => {}
    }
    if let Event::Typing { user_id, .. } = e {
        let ignorers: HashSet<i64> =
            sqlx::query_scalar("SELECT user_id FROM user_ignores WHERE ignored_id=?")
                .bind(id(*user_id)?)
                .fetch_all(&mut *c)
                .await?
                .into_iter()
                .collect();
        users.retain(|u| !ignorers.contains(&(u.0 as i64)));
    }
    Ok(())
}

/// True when `ch` (or a channel it is nested in) is age-restricted and `u`
/// has not confirmed their age there.
pub(crate) async fn age_blocked(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<bool> {
    crate::moderation::age::blocked(c, u, ch).await
}
/// Refuse reading `ch` (with the moderation task's `age_restricted` refusal).
pub(crate) async fn age(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<()> {
    crate::moderation::age_gate(c, u, ch).await
}
