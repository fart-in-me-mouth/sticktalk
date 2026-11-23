//! Invite cards: a message with `extra.activity` that shows a running session
//! (who is in it, Join) and its summary once it ended. The server rewrites the
//! card when its participants change and when it ends; each session posts at
//! most `MAX_ACTIVITY_CARDS`. Also the audit entries and the kept results.
use super::*;
use sqlx::Connection;

fn card(s: &Session, live: bool, summary: Option<ActivitySummary>) -> ActivityCard {
    let mut people: Vec<String> = vec![];
    let mut users: Vec<Id> = vec![];
    for p in &s.participants {
        if !users.contains(&p.session.user_id) {
            users.push(p.session.user_id);
            if people.len() < 8 {
                people.push(p.name.clone());
            }
        }
    }
    ActivityCard {
        session_id: s.id,
        call_channel_id: s.channel,
        guild_id: s.guild,
        activity_id: s.info.id.clone(),
        name: s.info.name.clone(),
        icon: s.info.icon.clone(),
        people: if live { people } else { vec![] },
        count: if live { users.len() as u16 } else { 0 },
        live,
        started_at: s.started_at,
        summary,
    }
}

/// The text older apps show for a card.
fn fallback(inviter: &str, c: &ActivityCard) -> String {
    format!(
        "{inviter} invited you to {} in a call. Join the call to take part.",
        c.name
    )
}

/// "Invite": a card for the session in `call` posted in `target`.
pub(super) async fn post(
    core: &mut Core,
    c: &mut SqliteConnection,
    call: Id,
    user: Id,
    target: Id,
) -> Result<()> {
    let s = &core.activities.sessions[&call];
    require(s.cards.len() < MAX_ACTIVITY_CARDS, ErrorCode::LimitExceeded)?;
    let guild = s.guild;
    let channel = db::text(
        c,
        user,
        target,
        Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
    )
    .await?;
    match guild {
        // A direct or group call can only be joined from its own conversation.
        None => require(target == call, ErrorCode::InvalidRequest)?,
        // A server call: that server's channels, or a conversation of yours.
        Some(g) => require(
            channel.summary.guild_id.is_none_or(|cg| cg == g),
            ErrorCode::InvalidRequest,
        )?,
    }
    require(
        !matches!(channel.summary.kind, ChannelKind::Stage),
        ErrorCode::InvalidRequest,
    )?;
    crate::moderation::coverage::timed_out(c, user, target).await?;
    let s = &core.activities.sessions[&call];
    let card = card(s, true, None);
    let inviter = s
        .participants
        .iter()
        .find(|p| p.session.user_id == user)
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let m = core.id().await?;
    let payload = rmp_serde::to_vec_named(&card).map_err(|_| Error::new(ErrorCode::Internal))?;
    {
        let mut tx = c.begin().await?;
        sqlx::query(
            "INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(?,?,?,?,?)",
        )
        .bind(db::id(m)?)
        .bind(db::id(target)?)
        .bind(db::id(user)?)
        .bind(fallback(&inviter, &card))
        .bind(now() as i64)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO activity_cards(message_id,session_id,payload) VALUES(?,?,?)")
            .bind(db::id(m)?)
            .bind(db::id(card.session_id)?)
            .bind(payload)
            .execute(&mut *tx)
            .await?;
        core.persist_id(&mut tx).await?;
        db::recount(&mut tx, target).await?;
        tx.commit().await?;
    }
    if let Some(s) = core.activities.sessions.get_mut(&call) {
        s.cards.push((m, target, user));
    }
    let message = db::message(c, user, target, m, false).await?;
    let recipients = db::audience(c, target).await?;
    let author = db::user(c, user).await?;
    core.emit(&recipients, Event::MessageCreated { message, author })
}

async fn rewrite(
    core: &mut Core,
    c: &mut SqliteConnection,
    cards: &[(Id, Id, Id)],
    card: &ActivityCard,
) -> Result<()> {
    let payload = rmp_serde::to_vec_named(card).map_err(|_| Error::new(ErrorCode::Internal))?;
    for (m, ch, author) in cards {
        let changed = sqlx::query("UPDATE activity_cards SET payload=? WHERE message_id=?")
            .bind(&payload)
            .bind(db::id(*m)?)
            .execute(&mut *c)
            .await?
            .rows_affected();
        // A deleted card stays deleted.
        if changed == 0 {
            continue;
        }
        let Ok(message) = db::message(c, *author, *ch, *m, false).await else {
            continue;
        };
        let recipients = db::audience(c, *ch).await?;
        core.emit(&recipients, Event::MessageUpdated(message))?;
    }
    Ok(())
}

/// The session's people changed: its cards follow.
pub(super) async fn refresh(core: &mut Core, c: &mut SqliteConnection, call: Id) -> Result<()> {
    let Some(s) = core.activities.sessions.get(&call) else {
        return Ok(());
    };
    if s.cards.is_empty() {
        return Ok(());
    }
    let (cards, card) = (s.cards.clone(), card(s, true, None));
    rewrite(core, c, &cards, &card).await
}

/// The session ended: its cards show the summary.
pub(super) async fn ended(
    core: &mut Core,
    c: &mut SqliteConnection,
    cards: &[(Id, Id, Id)],
    info: &ActivityInfo,
    summary: &ActivitySummary,
) -> Result<()> {
    if cards.is_empty() {
        return Ok(());
    }
    let payload: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT payload FROM activity_cards WHERE message_id=?")
            .bind(db::id(cards[0].0)?)
            .fetch_optional(&mut *c)
            .await?;
    let mut card: ActivityCard = match payload.and_then(|p| rmp_serde::from_slice(&p).ok()) {
        Some(card) => card,
        None => return Ok(()),
    };
    card.live = false;
    card.people.clear();
    card.count = 0;
    card.name = info.name.clone();
    card.summary = Some(summary.clone());
    rewrite(core, c, cards, &card).await
}

/// `extra.activity` on a message.
pub(crate) async fn decorate(c: &mut SqliteConnection, m: &mut Message) -> Result<()> {
    let payload: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT payload FROM activity_cards WHERE message_id=?")
            .bind(db::id(m.id)?)
            .fetch_optional(&mut *c)
            .await?;
    if let Some(card) = payload.and_then(|p| rmp_serde::from_slice::<ActivityCard>(&p).ok()) {
        m.extra.activity = Some(Box::new(card));
    }
    Ok(())
}

/// A finished session's result, kept with its conversation (the newest 50).
pub(super) async fn keep_result(
    c: &mut SqliteConnection,
    channel: Id,
    name: &str,
    text: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO activity_results(channel_id,name,text,ended_at) VALUES(?,?,?,?)")
        .bind(db::id(channel)?)
        .bind(name)
        .bind(text)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    sqlx::query("DELETE FROM activity_results WHERE channel_id=? AND id NOT IN (SELECT id FROM activity_results WHERE channel_id=? ORDER BY id DESC LIMIT ?)")
        .bind(db::id(channel)?)
        .bind(db::id(channel)?)
        .bind(MAX_ACTIVITY_RESULTS as i64)
        .execute(&mut *c)
        .await?;
    Ok(())
}

/// A file shared into Watch Together in a guild call is in the audit log:
/// everyone in the session can watch it, whatever they may read.
pub(super) async fn audit_share(
    c: &mut SqliteConnection,
    call: Id,
    actor: Id,
    item: &ActivityMedia,
) -> Result<()> {
    let guild: Option<i64> = sqlx::query_scalar("SELECT guild_id FROM channels WHERE id=?")
        .bind(db::id(call)?)
        .fetch_optional(&mut *c)
        .await?
        .flatten();
    let Some(guild) = guild else {
        return Ok(());
    };
    use crate::moderation::audit::Entry;
    let entry = Entry::new(
        Id(guild as u64),
        AuditAction::ActivityMediaShare,
        AuditTarget::Channel,
        Some(call),
    )
    .change("file", None, Some(item.filename.clone()))
    .change("from_channel", None, Some(item.channel_id.0.to_string()));
    let mut tx = c.begin().await?;
    crate::moderation::audit::write(&mut tx, actor, entry).await?;
    tx.commit().await?;
    Ok(())
}

/// Starting and ending a session in a guild voice channel is in the audit log.
pub(super) async fn audit(
    c: &mut SqliteConnection,
    guild: Id,
    actor: Id,
    action: AuditAction,
    ch: Id,
    name: &str,
    reason: Option<String>,
) -> Result<()> {
    use crate::moderation::audit::Entry;
    let entry = Entry::new(guild, action, AuditTarget::Channel, Some(ch))
        .change("activity", None, Some(name.to_owned()))
        .reason(reason);
    let mut tx = c.begin().await?;
    crate::moderation::audit::write(&mut tx, actor, entry).await?;
    tx.commit().await?;
    Ok(())
}
