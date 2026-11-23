//! Expression: custom emoji in every conversation, stickers, Super Reactions
//! and custom typing indicators. Wire contracts: `sticktalk_proto::expression`.
//! Behavior, routes and limits: `docs/features/expression.md`.
//!
//! Custom emoji and stickers may be used in any conversation by members of the
//! server they belong to (Use External Emoji / Use External Stickers decide it
//! in other servers' channels). Using one in a conversation records a grant for
//! that conversation, so everyone who can read it may load the image, even if
//! they are not in the source server. Nothing here runs while unused: there are
//! no timers, and every lookup is bounded.
mod library;
mod stickers;
#[cfg(test)]
mod tests;

pub(crate) use library::routes;
pub(crate) use stickers::asset_response;

use crate::{
    db::*,
    error::{require, Error, Result},
    permissions,
    state::{now, Core},
};
use sqlx::{Row, SqliteConnection};
use std::collections::BTreeMap;
use sticktalk_proto::*;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) type Deliveries = Vec<(Vec<Id>, Event)>;

/// Dispatch for `Command::Expression`, inside the command transaction.
pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    cmd: ExpressionCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    use ExpressionCommand::*;
    let result = match cmd {
        // `commands::execute` marks the stream; there is nothing to store.
        Subscribe => ExpressionResult::Ack,
        AddSuperReaction {
            channel_id,
            message_id,
            emoji,
        } => super_reaction(c, u, channel_id, message_id, emoji, true, out).await?,
        RemoveSuperReaction {
            channel_id,
            message_id,
            emoji,
        } => super_reaction(c, u, channel_id, message_id, emoji, false, out).await?,
        SendStickers {
            channel_id,
            sticker_ids,
            content,
            reply_to,
            mention_ids,
        } => {
            let send = stickers::Send {
                channel_id,
                sticker_ids,
                content,
                reply_to,
                mention_ids,
            };
            return stickers::send(c, core, key, u, send, out).await;
        }
        UpdateSticker {
            guild_id,
            sticker_id,
            name,
            emoji,
            description,
        } => {
            let edit = stickers::Edit {
                name,
                emoji,
                description,
            };
            stickers::update(c, u, guild_id, sticker_id, edit, out).await?
        }
        DeleteSticker {
            guild_id,
            sticker_id,
        } => stickers::delete(c, u, guild_id, sticker_id, out).await?,
        SetTypingStyle { style } => {
            if let Some(style) = &style {
                validate_style(c, u, style).await?;
            }
            let mut settings = library::settings(c, u).await?;
            settings.typing = style;
            library::store(c, u, &settings, out).await?;
            ExpressionResult::Settings(settings)
        }
        SetStickerFavorites { sticker_ids } => {
            stickers::validate_favorites(c, u, &sticker_ids).await?;
            let mut settings = library::settings(c, u).await?;
            settings.sticker_favorites = sticker_ids;
            library::store(c, u, &settings, out).await?;
            ExpressionResult::Settings(settings)
        }
    };
    Ok(CommandResult::Expression(result))
}

/// The server a custom emoji belongs to, if it still exists.
async fn emoji_guild(c: &mut SqliteConnection, e: Id) -> Result<Option<Id>> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT guild_id FROM guild_emoji WHERE id=?")
            .bind(id(e)?)
            .fetch_optional(&mut *c)
            .await?
            .map(|g| Id(g as u64)),
    )
}

/// Whether `u` may use something from server `source` in this conversation:
/// they are a member of `source`, and in another server's channel they also
/// have `external` (Use External Emoji or Use External Stickers) there.
pub(crate) async fn usable(
    c: &mut SqliteConnection,
    u: Id,
    channel: &ChannelSummary,
    source: Id,
    external: Permissions,
) -> Result<bool> {
    if role(c, u, source).await.is_err() {
        return Ok(false);
    }
    Ok(match channel.guild_id {
        Some(g) if g == source => true,
        Some(g) => permissions::evaluate(c, u, g, Some(channel.id))
            .await?
            .contains(external),
        None => true,
    })
}

async fn grant(c: &mut SqliteConnection, table: &str, item: Id, ch: Id) -> Result<()> {
    let sql = match table {
        "emoji" => "INSERT OR IGNORE INTO emoji_grants(emoji_id,channel_id) VALUES(?,?)",
        _ => "INSERT OR IGNORE INTO sticker_grants(sticker_id,channel_id) VALUES(?,?)",
    };
    sqlx::query(sql)
        .bind(id(item)?)
        .bind(id(ch)?)
        .execute(&mut *c)
        .await?;
    Ok(())
}

/// Message text as it is stored: each custom emoji the author may use here is
/// kept (and granted to this conversation); any other becomes `:name:`.
pub(crate) async fn check_content(
    c: &mut SqliteConnection,
    u: Id,
    channel: &Channel,
    content: &str,
) -> Result<String> {
    let tokens = emoji_tokens(content);
    if tokens.is_empty() {
        return Ok(content.to_owned());
    }
    let mut decided: BTreeMap<Id, bool> = BTreeMap::new();
    let mut out = String::with_capacity(content.len());
    let mut last = 0;
    for (range, name, e) in tokens {
        let ok = match decided.get(&e) {
            Some(ok) => *ok,
            None => {
                let ok = match emoji_guild(c, e).await? {
                    Some(g) => {
                        usable(c, u, &channel.summary, g, Permissions::USE_EXTERNAL_EMOJI).await?
                    }
                    None => false,
                };
                if ok {
                    grant(c, "emoji", e, channel.summary.id).await?;
                }
                decided.insert(e, ok);
                ok
            }
        };
        out.push_str(&content[last..range.start]);
        if ok {
            out.push_str(&content[range.clone()]);
        } else {
            out.push(':');
            out.push_str(name);
            out.push(':');
        }
        last = range.end;
    }
    out.push_str(&content[last..]);
    Ok(out)
}

/// A reaction's emoji as stored: one Unicode emoji, or `<:emoji:ID>` for a
/// custom emoji (the name is left out, so renaming keeps reactions together).
/// Adding a custom one needs the same rights as using it in a message.
pub(crate) async fn reaction_emoji(
    c: &mut SqliteConnection,
    u: Id,
    channel: &Channel,
    emoji: &str,
    added: bool,
) -> Result<String> {
    if let Some(e) = custom_emoji_id(emoji) {
        // Removal is allowed after deletion, or after leaving its server.
        if added {
            let g = emoji_guild(c, e)
                .await?
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            require(
                usable(c, u, &channel.summary, g, Permissions::USE_EXTERNAL_EMOJI).await?,
                ErrorCode::InvalidRequest,
            )?;
            grant(c, "emoji", e, channel.summary.id).await?;
        }
        return Ok(format!("<:emoji:{}>", e.0));
    }
    require(unicode_emoji(emoji), ErrorCode::InvalidRequest)?;
    Ok(emoji.to_owned())
}

/// A poll answer's emoji: a custom one is kept only if the author may use it
/// in that conversation (and is then granted to it).
pub(crate) async fn poll_emoji(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    emoji: Option<String>,
) -> Result<Option<String>> {
    let Some(e) = emoji else { return Ok(None) };
    let Some(eid) = custom_emoji_id(&e) else {
        return Ok(Some(e));
    };
    let channel = channel(c, ch).await?;
    let ok = match emoji_guild(c, eid).await? {
        Some(g) => usable(c, u, &channel.summary, g, Permissions::USE_EXTERNAL_EMOJI).await?,
        None => false,
    };
    if ok {
        grant(c, "emoji", eid, ch).await?;
    }
    Ok(ok.then_some(e))
}

pub(crate) fn unicode_emoji(e: &str) -> bool {
    !e.is_empty() && e.len() <= 32 && e.graphemes(true).count() == 1 && emojis::get(e).is_some()
}

async fn super_reaction(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    m: Id,
    emoji: String,
    added: bool,
    out: &mut Deliveries,
) -> Result<ExpressionResult> {
    let channel = text(c, u, ch, Permissions::ADD_REACTIONS).await?;
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM messages WHERE id=? AND channel_id=? AND deleted_at IS NULL",
    )
    .bind(id(m)?)
    .bind(id(ch)?)
    .fetch_optional(&mut *c)
    .await?;
    require(found.is_some(), ErrorCode::NotFound)?;
    let emoji = reaction_emoji(c, u, &channel, &emoji, added).await?;
    let changed = if added {
        // Normal and Super Reactions share the per-message limit of distinct emoji.
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (SELECT emoji FROM reactions WHERE message_id=?1 UNION SELECT emoji FROM super_reactions WHERE message_id=?1) WHERE emoji<>?2",
        )
        .bind(id(m)?)
        .bind(&emoji)
        .fetch_one(&mut *c)
        .await?;
        require(
            (n as usize) < MAX_REACTIONS_PER_MESSAGE,
            ErrorCode::LimitExceeded,
        )?;
        sqlx::query("INSERT OR IGNORE INTO super_reactions VALUES(?,?,?,?)")
            .bind(id(m)?)
            .bind(id(u)?)
            .bind(&emoji)
            .bind(now() as i64)
            .execute(&mut *c)
            .await?
            .rows_affected()
            > 0
    } else {
        sqlx::query("DELETE FROM super_reactions WHERE message_id=? AND user_id=? AND emoji=?")
            .bind(id(m)?)
            .bind(id(u)?)
            .bind(&emoji)
            .execute(&mut *c)
            .await?
            .rows_affected()
            > 0
    };
    let mut result = None;
    for v in audience(c, ch).await? {
        let r = sqlx::query("SELECT COUNT(*) AS n,COALESCE(MAX(user_id=?),0) AS me FROM super_reactions WHERE message_id=? AND emoji=?")
            .bind(id(v)?)
            .bind(id(m)?)
            .bind(&emoji)
            .fetch_one(&mut *c)
            .await?;
        let update = ReactionUpdate {
            channel_id: ch,
            message_id: m,
            user_id: u,
            added,
            reaction: ReactionSummary {
                emoji: emoji.clone(),
                count: r.get::<i64, _>("n") as u32,
                me: r.get::<i64, _>("me") != 0,
            },
        };
        if v == u {
            result = Some(update.clone());
        }
        if changed {
            out.push((
                vec![v],
                Event::Expression(ExpressionEvent::SuperReactionUpdated(update)),
            ));
        }
    }
    Ok(ExpressionResult::SuperReaction(
        result.ok_or_else(|| Error::new(ErrorCode::Internal))?,
    ))
}

/// Up to three emoji (Unicode, or custom ones the person may use), a motion
/// and a verb from the allowed set.
async fn validate_style(c: &mut SqliteConnection, u: Id, style: &TypingStyle) -> Result<()> {
    require(
        style.emoji.len() <= MAX_TYPING_EMOJI,
        ErrorCode::InvalidRequest,
    )?;
    for e in &style.emoji {
        if let Some(eid) = custom_emoji_id(e) {
            let g = emoji_guild(c, eid)
                .await?
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            require(role(c, u, g).await.is_ok(), ErrorCode::InvalidRequest)?;
        } else {
            require(unicode_emoji(e), ErrorCode::InvalidRequest)?;
        }
    }
    if let TypingVerb::Custom(verb) = &style.verb {
        require(valid_typing_verb(verb), ErrorCode::InvalidRequest)?;
    }
    Ok(())
}

/// Before a `Typing` event from `u` in `ch`: their indicator, if they chose one.
pub(crate) async fn typing(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    out: &mut Deliveries,
) -> Result<()> {
    let Some(mut style) = library::settings(c, u).await?.typing else {
        return Ok(());
    };
    // Custom emoji in it must stay usable; one that is gone is dropped.
    let mut kept = vec![];
    for e in std::mem::take(&mut style.emoji) {
        match custom_emoji_id(&e) {
            Some(eid) => {
                if let Some(g) = emoji_guild(c, eid).await? {
                    if role(c, u, g).await.is_ok() {
                        grant(c, "emoji", eid, ch).await?;
                        kept.push(e);
                    }
                }
            }
            None => kept.push(e),
        }
    }
    style.emoji = kept;
    let viewers = audience(c, ch)
        .await?
        .into_iter()
        .filter(|v| *v != u)
        .collect();
    out.push((
        viewers,
        Event::Expression(ExpressionEvent::TypingStyle { user_id: u, style }),
    ));
    Ok(())
}

/// Fills in `Message::extra.stickers` and `super_reactions` for one viewer.
pub(crate) async fn decorate(c: &mut SqliteConnection, viewer: Id, m: &mut Message) -> Result<()> {
    m.extra.stickers = stickers::load(c, m.id).await?;
    m.extra.super_reactions = sqlx::query("SELECT emoji,COUNT(*) AS n,MAX(user_id=?) AS me FROM super_reactions WHERE message_id=? GROUP BY emoji ORDER BY emoji")
        .bind(id(viewer)?)
        .bind(id(m.id)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| ReactionSummary {
            emoji: r.get("emoji"),
            count: r.get::<i64, _>("n") as u32,
            me: r.get::<i64, _>("me") != 0,
        })
        .collect();
    Ok(())
}

/// Whether `u` may load a custom emoji or sticker image: a member of its
/// server, or able to read a conversation it was used in.
pub(crate) async fn may_see(
    c: &mut SqliteConnection,
    u: Id,
    table: &str,
    item: Id,
    guild: Id,
) -> Result<bool> {
    if role(c, u, guild).await.is_ok() {
        return Ok(true);
    }
    let sql = match table {
        "emoji" => "SELECT g.channel_id FROM emoji_grants g JOIN channels ch ON ch.id=g.channel_id WHERE g.emoji_id=?1 AND (EXISTS(SELECT 1 FROM dm_members d WHERE d.channel_id=ch.id AND d.user_id=?2) OR EXISTS(SELECT 1 FROM guild_members gm WHERE gm.guild_id=ch.guild_id AND gm.user_id=?2)) LIMIT 64",
        _ => "SELECT g.channel_id FROM sticker_grants g JOIN channels ch ON ch.id=g.channel_id WHERE g.sticker_id=?1 AND (EXISTS(SELECT 1 FROM dm_members d WHERE d.channel_id=ch.id AND d.user_id=?2) OR EXISTS(SELECT 1 FROM guild_members gm WHERE gm.guild_id=ch.guild_id AND gm.user_id=?2)) LIMIT 64",
    };
    let channels: Vec<i64> = sqlx::query_scalar(sql)
        .bind(id(item)?)
        .bind(id(u)?)
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

/// The channel and text a sticker send creates, for AutoMod and timeouts.
pub(crate) fn outgoing(cmd: &ExpressionCommand) -> Option<(Id, &str, &[Id])> {
    match cmd {
        ExpressionCommand::SendStickers {
            channel_id,
            content,
            mention_ids,
            ..
        } => Some((*channel_id, content.as_str(), mention_ids.as_slice())),
        _ => None,
    }
}

/// Manage Expressions (the Manage Emoji bit) in that server.
pub(crate) async fn manage(c: &mut SqliteConnection, u: Id, g: Id) -> Result<()> {
    require(
        permissions::evaluate(c, u, g, None)
            .await?
            .contains(Permissions::MANAGE_EMOJI),
        ErrorCode::Forbidden,
    )
}
