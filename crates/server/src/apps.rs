//! Apps, bot users, application commands, interactions, message components
//! and incoming webhooks. See `docs/features/apps.md` (contract) and
//! `docs/BOTS.md` (developer guide).
//!
//! Security model:
//! - A bot is an ordinary account (`users`) with no password and a username no
//!   person can register or sign in with. Its token is a `sessions` row listed
//!   in `app_sessions`; `db::auth` refuses those rows, so a bot token never
//!   works where a person's session does. Bots use the WebSocket (this
//!   module's `authenticate`) and `/api/v1/bot/*`, and every command a bot
//!   sends passes `gate`: an allowlist, then the ordinary permission checks of
//!   its member in each server.
//! - A webhook posts as its own account, which is never a server member: in
//!   `permissions::evaluate` it holds a fixed set of rights in its channel
//!   (and the threads in it) and nothing anywhere else. Its secret URL is
//!   stored hashed; its posting credential (`app_sessions`) is used only here.
//! - Everything a bot or webhook posts goes through `commands::execute`, so
//!   AutoMod, timeouts, slowmode, mentions, audit and fan-out apply as for a
//!   person (`moderation::precheck` screens embeds via `outgoing`).
use crate::{
    db::*,
    error::{require, Error, Result},
    state::{now, App, Core, Stream},
};
use sqlx::{Row, SqliteConnection};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use sticktalk_proto::*;

pub(crate) mod admin;
mod install;
pub(crate) use install::settle;
mod interactions;
mod messages;
mod registry;
mod rest;
mod store;
#[cfg(test)]
mod tests;
mod webhooks;

type Deliveries = Vec<(Vec<Id>, Event)>;

/// Session expiry for bot and webhook credentials: they last until revoked.
pub(crate) const FOREVER: i64 = 253_402_300_799_000;
/// Rights a webhook has in its own channel and the threads in it.
const WEBHOOK_RIGHTS: Permissions = Permissions::from_bits_retain(
    Permissions::VIEW_CHANNEL.bits()
        | Permissions::SEND_MESSAGES.bits()
        | Permissions::SEND_MESSAGES_IN_THREADS.bits()
        | Permissions::EMBED_LINKS.bits()
        | Permissions::ATTACH_FILES.bits(),
);

/// In-memory state, under the command gate (`Core::apps`).
#[derive(Default)]
pub(crate) struct Runtime {
    /// Bot connections (by stream ID): what each may receive.
    pub(crate) bots: HashMap<String, BotStream>,
    /// People's streams that sent `AppCommand::Subscribe`.
    pub(crate) subscribed: HashSet<String>,
    pub(crate) interactions: interactions::Interactions,
    pub(crate) images: messages::ImageQueue,
    /// Files people passed to bots in interactions (`interactions::FileGrants`).
    pub(crate) files: interactions::FileGrants,
    /// Wakes the timer task (interaction deadlines, image fetches).
    pub(crate) wake: Arc<tokio::sync::Notify>,
    /// `STICKTALK_PUBLIC_URL`, for webhook URLs.
    pub(crate) public: String,
}

pub(crate) struct BotStream {
    pub(crate) app: Id,
    pub(crate) user: Id,
    pub(crate) intents: Intents,
    /// Servers that granted Message Content.
    pub(crate) content_guilds: HashSet<Id>,
    /// Their channels and threads (filled as they appear).
    pub(crate) content_channels: HashSet<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Credential {
    Bot { app: Id },
    Webhook { webhook: Id },
}

tokio::task_local! {
    /// Set only around this module's own validated sends: the message may
    /// have no text, because it carries embeds or components.
    static RICH: bool;
}
/// For `commands::validate_message`.
pub(crate) fn rich() -> bool {
    RICH.try_with(|v| *v).unwrap_or(false)
}
pub(crate) async fn rich_scope<F: std::future::Future>(f: F) -> F::Output {
    RICH.scope(true, f).await
}

/// The kind of credential behind a session key, if it is not a person's.
pub(crate) async fn credential(c: &mut SqliteConnection, key: &[u8]) -> Result<Option<Credential>> {
    let r = sqlx::query("SELECT kind,owner_id FROM app_sessions WHERE session_hash=?")
        .bind(key)
        .fetch_optional(&mut *c)
        .await?;
    Ok(r.map(|r| {
        let owner = Id(r.get::<i64, _>("owner_id") as u64);
        if r.get::<String, _>("kind") == "bot" {
            Credential::Bot { app: owner }
        } else {
            Credential::Webhook { webhook: owner }
        }
    }))
}

/// WebSocket Identify: a bot token (`stkb_…`) or a person's session token.
pub(crate) async fn authenticate(
    core: &mut Core,
    c: &mut SqliteConnection,
    token: &str,
) -> Result<(Vec<u8>, Id)> {
    let Some(raw) = token.strip_prefix(BOT_TOKEN_PREFIX) else {
        return auth(c, token).await;
    };
    let (key, bot, app) = store::bot_auth(c, raw).await?;
    core.rate(format!("bot-identify:{}", app.0), 6., 3.)?;
    core.cursor(&key, bot);
    register(core, c, &key, app, bot).await?;
    Ok((key, bot))
}

/// Makes sure the bot's stream is filtered by its intents before any event
/// can be queued to it.
async fn register(
    core: &mut Core,
    c: &mut SqliteConnection,
    key: &[u8],
    app: Id,
    bot: Id,
) -> Result<()> {
    let Some(stream) = core.streams.get(key).map(|s| s.id.clone()) else {
        return Ok(());
    };
    if core
        .apps
        .bots
        .get(&stream)
        .is_some_and(|b| b.app == app && b.user == bot)
    {
        return Ok(());
    }
    let live: HashSet<String> = core.streams.values().map(|s| s.id.clone()).collect();
    core.apps.bots.retain(|id, _| live.contains(id));
    let intents = Intents::from_bits_truncate(
        sqlx::query_scalar::<_, i64>("SELECT intents FROM apps WHERE id=?")
            .bind(id(app)?)
            .fetch_one(&mut *c)
            .await? as u32,
    );
    let mut content_guilds = HashSet::new();
    let mut content_channels = HashSet::new();
    if intents.contains(Intents::MESSAGE_CONTENT) {
        content_guilds = sqlx::query_scalar::<_, i64>("SELECT i.guild_id FROM app_installs i JOIN guild_members m ON m.guild_id=i.guild_id AND m.user_id=? WHERE i.app_id=? AND i.message_content=1")
            .bind(id(bot)?)
            .bind(id(app)?)
            .fetch_all(&mut *c)
            .await?
            .into_iter()
            .map(|g| Id(g as u64))
            .collect();
        for g in &content_guilds {
            content_channels.extend(
                sqlx::query_scalar::<_, i64>(
                    "SELECT id FROM channels WHERE guild_id=? ORDER BY id DESC LIMIT 20000",
                )
                .bind(id(*g)?)
                .fetch_all(&mut *c)
                .await?
                .into_iter()
                .map(|v| Id(v as u64)),
            );
        }
    }
    core.apps.bots.insert(
        stream,
        BotStream {
            app,
            user: bot,
            intents,
            content_guilds,
            content_channels,
        },
    );
    Ok(())
}

/// Bots may only send these commands; every one still runs the ordinary
/// permission checks of the bot's member.
fn bot_allowed(cmd: &Command) -> bool {
    use Command::*;
    match cmd {
        SendMessage(_)
        | EditMessage(_)
        | DeleteMessage { .. }
        | AddReaction { .. }
        | RemoveReaction { .. }
        | Typing { .. }
        | MarkRead { .. }
        | KickMember { .. }
        | BanMember { .. }
        | UnbanMember { .. }
        | CreateInvite { .. }
        | RevokeInvite { .. }
        | CreateChannel { .. }
        | UpdateChannel { .. }
        | ReorderChannels { .. }
        | DeleteChannel { .. }
        | UpdateGuild { .. }
        | SubscribeGuild { .. } => true,
        Launch(l) => matches!(
            l,
            LaunchCommand::SetSlowmode { .. }
                | LaunchCommand::CreateRole { .. }
                | LaunchCommand::UpdateRole { .. }
                | LaunchCommand::DeleteRole { .. }
                | LaunchCommand::ReorderRoles { .. }
                | LaunchCommand::SetMemberRoles { .. }
                | LaunchCommand::SetOverwrites { .. }
                | LaunchCommand::SetNickname { .. }
                | LaunchCommand::PinMessage { .. }
                | LaunchCommand::Moderation(
                    ModerationCommand::Kick { .. } | ModerationCommand::Timeout { .. }
                )
        ),
        Threads(t) => !matches!(
            t,
            ThreadCommand::Subscribe
                | ThreadCommand::Follow { .. }
                | ThreadCommand::Unfollow { .. }
        ),
        Content(c) => matches!(
            c,
            ContentCommand::SendMessage { .. }
                | ContentCommand::CreatePoll(_)
                | ContentCommand::EndPoll { .. }
                | ContentCommand::RemoveEmbeds { .. }
        ),
        Apps(a) => a.bot_only(),
        _ => false,
    }
}

/// Commands that keep no receipt (interaction traffic is idempotent by its
/// own state, and webhooks have their own limits).
pub(crate) fn transient(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::Apps(
            AppCommand::Subscribe
                | AppCommand::Invoke(_)
                | AppCommand::Autocomplete { .. }
                | AppCommand::Component { .. }
                | AppCommand::SubmitModal { .. }
                | AppCommand::Respond { .. }
                | AppCommand::EditOriginal { .. }
                | AppCommand::DeleteOriginal { .. }
                | AppCommand::FollowUp { .. }
                | AppCommand::ExecuteWebhook(_)
                | AppCommand::EditWebhookMessage { .. }
                | AppCommand::DeleteWebhookMessage { .. }
        )
    )
}

/// Before every command (`commands::execute`): who may send it.
pub(crate) async fn gate(
    core: &mut Core,
    c: &mut SqliteConnection,
    key: &[u8],
    u: Id,
    cmd: &Command,
) -> Result<()> {
    match credential(c, key).await? {
        None => {
            if let Command::Apps(a) = cmd {
                require(!a.bot_only() && !a.webhook_only(), ErrorCode::Forbidden)?;
                if let Some(s) = core.streams.get(key) {
                    if core.apps.subscribed.len() < 65_536 {
                        core.apps.subscribed.insert(s.id.clone());
                    }
                }
            }
        }
        Some(Credential::Bot { app }) => {
            if !bot_allowed(cmd) {
                return Err(denied(
                    "bot",
                    "Bots can't use this. See docs/BOTS.md for what bots may do.",
                ));
            }
            store::usable(c, app).await?;
            core.rate(format!("bot:{}", app.0), 600., 60.)?;
            register(core, c, key, app, u).await?;
        }
        Some(Credential::Webhook { .. }) => {
            require(
                matches!(cmd, Command::Apps(a) if a.webhook_only()),
                ErrorCode::Forbidden,
            )?;
        }
    }
    install::guard(c, cmd).await?;
    warm(core, c, cmd).await
}

/// A new thread's messages reach bots that may read content there.
async fn warm(core: &mut Core, c: &mut SqliteConnection, cmd: &Command) -> Result<()> {
    if core.apps.bots.is_empty() {
        return Ok(());
    }
    let ch = match cmd {
        Command::SendMessage(d) => d.channel_id,
        Command::Content(ContentCommand::SendMessage { draft, .. }) => draft.channel_id,
        Command::Apps(AppCommand::Send { channel_id, .. }) => *channel_id,
        _ => return Ok(()),
    };
    let g: Option<Option<i64>> = sqlx::query_scalar("SELECT guild_id FROM channels WHERE id=?")
        .bind(id(ch)?)
        .fetch_optional(&mut *c)
        .await?;
    if let Some(Some(g)) = g {
        for b in core.apps.bots.values_mut() {
            if b.content_guilds.contains(&Id(g as u64)) && b.content_channels.len() < 40_000 {
                b.content_channels.insert(ch);
            }
        }
    }
    Ok(())
}

/// How one event reaches one stream (`Core::emit_filtered`).
pub(crate) enum View {
    /// Not a bot: the ordinary rules apply.
    Pass,
    Skip,
    /// A bot's member event: delivered regardless of its selected server.
    Member,
    /// Delivered as this projection (message content withheld).
    Project(Box<Event>),
}

pub(crate) fn view(rt: &mut Runtime, s: &Stream, event: &Event) -> View {
    let Some(bot) = rt.bots.get_mut(&s.id) else {
        if matches!(event, Event::Apps(_)) && !rt.subscribed.contains(&s.id) {
            return View::Skip;
        }
        return View::Pass;
    };
    let has = |i: Intents| bot.intents.contains(i);
    // Deny by default: every kind of event is classified here (the match is
    // exhaustive, so a new `Event` variant does not compile until it is), and
    // the feature events pass only the variants listed. Anything that carries
    // message content (`privacy::gate::carries_content`) is treated like a
    // message: the Messages intent, and Message Content for that channel.
    let content = |bot: &BotStream, ch: Id| {
        has(Intents::GUILD_MESSAGES)
            && bot.intents.contains(Intents::MESSAGE_CONTENT)
            && bot.content_channels.contains(&ch)
    };
    match event {
        Event::MessageCreated { message, author } => {
            if !has(Intents::GUILD_MESSAGES) && message.author_id != bot.user {
                return View::Skip;
            }
            match project(bot, message) {
                Some(m) => View::Project(Box::new(Event::MessageCreated {
                    message: m,
                    author: author.clone(),
                })),
                None => View::Pass,
            }
        }
        Event::MessageUpdated(message) => {
            if !has(Intents::GUILD_MESSAGES) {
                return View::Skip;
            }
            match project(bot, message) {
                Some(m) => View::Project(Box::new(Event::MessageUpdated(m))),
                None => View::Pass,
            }
        }
        Event::MessageDeleted { .. } | Event::PinsUpdated { .. } => {
            if has(Intents::GUILD_MESSAGES) {
                View::Pass
            } else {
                View::Skip
            }
        }
        Event::ReactionUpdated(_) => {
            if has(Intents::GUILD_MESSAGE_REACTIONS) {
                View::Pass
            } else {
                View::Skip
            }
        }
        Event::Typing { .. } => {
            if has(Intents::GUILD_MESSAGE_TYPING) {
                View::Pass
            } else {
                View::Skip
            }
        }
        // A poll's text and a link's preview are message content.
        Event::Content(ContentEvent::PollUpdated { channel_id, .. })
        | Event::Content(ContentEvent::EmbedsUpdated { channel_id, .. }) => {
            if content(bot, *channel_id) {
                View::Pass
            } else {
                View::Skip
            }
        }
        Event::Content(
            ContentEvent::ScheduledUpserted(_) | ContentEvent::ScheduledRemoved { .. },
        ) => View::Skip,
        Event::MemberUpsert(m) if m.user.id == bot.user => View::Member,
        Event::MemberUpsert(_) | Event::MemberRemoved { .. } => {
            if has(Intents::GUILD_MEMBERS) {
                View::Member
            } else {
                View::Skip
            }
        }
        // Names, avatars and tags of people the bot shares a server with.
        Event::UserUpdated(_) => {
            if has(Intents::GUILD_MEMBERS) {
                View::Pass
            } else {
                View::Skip
            }
        }
        Event::BanUpsert(_) | Event::BanRemoved { .. } => {
            if has(Intents::GUILD_MODERATION) {
                View::Pass
            } else {
                View::Skip
            }
        }
        Event::ChannelUpsert(ch) => {
            if ch
                .summary
                .guild_id
                .is_some_and(|g| bot.content_guilds.contains(&g))
                && bot.content_channels.len() < 40_000
            {
                bot.content_channels.insert(ch.summary.id);
            }
            View::Pass
        }
        // The server's structure, as any member sees it.
        Event::GuildUpsert(_)
        | Event::GuildJoined { .. }
        | Event::GuildRemoved { .. }
        | Event::ChannelRemoved { .. }
        | Event::ChannelsReordered { .. }
        | Event::RolesUpdated { .. }
        | Event::SlowmodeUpdated { .. }
        | Event::GuildEmojiUpdated { .. }
        | Event::GuildProfileUpdated(_)
        | Event::InviteUpsert(_)
        | Event::InviteRemoved { .. }
        | Event::SelfUpdated(_)
        | Event::Apps(_) => View::Pass,
        // Threads: their names and settings, like channels'.
        Event::Threads(t) => match t {
            ThreadEvent::ThreadUpsert(_)
            | ThreadEvent::ThreadRemoved { .. }
            | ThreadEvent::Membership { .. }
            | ThreadEvent::ForumUpdated(_)
            | ThreadEvent::Published { .. }
            | ThreadEvent::FollowsChanged { .. } => View::Pass,
        },
        // A person's own data, profile text, presence, and the other features'
        // events: no intent covers them.
        Event::PresenceUpdated(_)
        | Event::ProfileUpdated(_)
        | Event::ReadStateUpdated(_)
        | Event::NotificationSettingsUpdated(_)
        | Event::NavUpdated(_)
        | Event::SavedUpdated(_)
        | Event::ReminderDue(_)
        | Event::ServerFoldersUpdated(_)
        | Event::DataExportUpdated(_)
        | Event::SessionsUpdated
        | Event::RelationshipUpsert(_)
        | Event::RelationshipRemoved { .. }
        | Event::Moderation(_)
        | Event::Expression(_)
        | Event::Privacy(_)
        | Event::Community(_)
        | Event::Schedule(_)
        | Event::Appearance(_) => View::Skip,
    }
}

/// A message as a bot without Message Content there sees it, or None when it
/// may see all of it (its own messages and messages that mention it).
fn project(bot: &BotStream, m: &Message) -> Option<Message> {
    if m.author_id == bot.user
        || m.mention_ids.contains(&bot.user)
        || (bot.intents.contains(Intents::MESSAGE_CONTENT)
            && bot.content_channels.contains(&m.channel_id))
    {
        return None;
    }
    Some(withheld(m))
}
pub(crate) fn withheld(m: &Message) -> Message {
    Message {
        content: String::new(),
        attachments: vec![],
        extra: Default::default(),
        ..m.clone()
    }
}

/// `moderation::precheck`: the text each app command would post, so AutoMod,
/// timeouts and verification apply to bots and webhooks as to anyone.
pub(crate) struct Outgoing {
    pub channel_id: Id,
    pub text: String,
    pub send: bool,
}
pub(crate) fn outgoing(core: &Core, cmd: &AppCommand) -> Vec<Outgoing> {
    let draft_text =
        |d: &AppMessageDraft| messages::screen_text(&d.content, &d.embeds, &d.components);
    let at = |answer: &InteractionAnswer| core.apps.interactions.channel(answer.interaction_id);
    let mut out = vec![];
    match cmd {
        AppCommand::Send { channel_id, draft } => out.push(Outgoing {
            channel_id: *channel_id,
            text: draft_text(draft),
            send: true,
        }),
        AppCommand::Edit {
            channel_id, draft, ..
        } => out.push(Outgoing {
            channel_id: *channel_id,
            text: draft_text(draft),
            send: false,
        }),
        AppCommand::Respond { answer, response } => {
            let draft = match response {
                InteractionResponse::Message(d) => Some((d, true)),
                InteractionResponse::UpdateMessage(d) => Some((d, false)),
                _ => None,
            };
            if let (Some((d, send)), Some(ch)) = (draft, at(answer)) {
                out.push(Outgoing {
                    channel_id: ch,
                    text: draft_text(d),
                    send,
                });
            }
        }
        AppCommand::EditOriginal { answer, draft } | AppCommand::FollowUp { answer, draft } => {
            if let Some(ch) = at(answer) {
                out.push(Outgoing {
                    channel_id: ch,
                    text: draft_text(draft),
                    send: matches!(cmd, AppCommand::FollowUp { .. }),
                });
            }
        }
        AppCommand::ExecuteWebhook(p) => out.push(Outgoing {
            channel_id: p.channel_id,
            text: webhooks::screen_text(p),
            send: true,
        }),
        AppCommand::EditWebhookMessage {
            channel_id, post, ..
        } => out.push(Outgoing {
            channel_id: *channel_id,
            text: webhooks::screen_text(post),
            send: false,
        }),
        _ => {}
    }
    out
}

/// `permissions::evaluate` for an account that is not a member of `guild`:
/// a webhook's fixed rights in its channel (and threads in it), else NotFound.
pub(crate) async fn principal(
    c: &mut SqliteConnection,
    user: Id,
    guild: Id,
    channel: Option<Id>,
) -> Result<Permissions> {
    let r =
        sqlx::query("SELECT channel_id,creator_id FROM webhooks WHERE user_id=? AND guild_id=?")
            .bind(id(user)?)
            .bind(id(guild)?)
            .fetch_optional(&mut *c)
            .await?
            .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let Some(ch) = channel else {
        return Ok(Permissions::empty());
    };
    let home = Id(r.get::<i64, _>("channel_id") as u64);
    let lineage = crate::permissions::lineage(c, guild, ch).await?;
    if !lineage.contains(&home) {
        return Ok(Permissions::empty());
    }
    let mut p = WEBHOOK_RIGHTS;
    // @everyone and @here only while the webhook's creator may use them there.
    let creator = Id(r.get::<i64, _>("creator_id") as u64);
    if role(c, creator, guild).await.is_ok()
        && Box::pin(crate::permissions::evaluate(c, creator, guild, Some(ch)))
            .await
            .is_ok_and(|p| p.contains(Permissions::MENTION_EVERYONE))
    {
        p |= Permissions::MENTION_EVERYONE;
    }
    crate::threads::scope(c, user, ch, p).await
}

/// `db::message`: what an app or webhook added.
pub(crate) async fn decorate(c: &mut SqliteConnection, m: &mut Message) -> Result<()> {
    let data: Option<String> =
        sqlx::query_scalar("SELECT data FROM app_messages WHERE message_id=?")
            .bind(id(m.id)?)
            .fetch_optional(&mut *c)
            .await?;
    if let Some(data) = data {
        if let Ok(app) = serde_json::from_str::<AppMessage>(&data) {
            m.extra.app = Some(Box::new(app));
        }
    }
    Ok(())
}

/// `Command::Apps`, inside the command transaction.
pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    cmd: AppCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    use AppCommand::*;
    let result = match cmd {
        Subscribe => AppResult::Ack,
        CreateApp(d) => store::create(c, core, u, d, out).await?,
        UpdateApp { app_id, draft } => store::update(c, u, app_id, draft, out).await?,
        DeleteApp { app_id } => store::delete(c, core, key, u, app_id, out).await?,
        ResetToken { app_id } => store::reset_token(c, core, u, app_id).await?,
        Install(r) => install::install(c, core, u, r, out).await?,
        Uninstall { guild_id, app_id } => {
            install::uninstall(c, core, u, guild_id, app_id, out).await?
        }
        SetCommandPermissions {
            guild_id,
            id,
            targets,
        } => registry::set_permissions(c, u, guild_id, id, targets, out).await?,
        CreateWebhook {
            channel_id,
            name,
            avatar_id,
        } => webhooks::create(c, core, u, channel_id, name, avatar_id, out).await?,
        UpdateWebhook {
            webhook_id,
            name,
            avatar,
            channel_id,
        } => webhooks::update(c, core, u, webhook_id, name, avatar, channel_id, out).await?,
        RotateWebhook { webhook_id } => webhooks::rotate(c, core, u, webhook_id, out).await?,
        DeleteWebhook { webhook_id } => webhooks::delete(c, u, webhook_id, out).await?,
        Invoke(inv) => interactions::invoke(c, core, key, u, inv, out).await?,
        Autocomplete {
            invocation,
            request,
        } => interactions::autocomplete(c, core, key, u, invocation, request, out).await?,
        Component {
            channel_id,
            message_id,
            custom_id,
            values,
        } => {
            interactions::component(
                c, core, key, u, channel_id, message_id, custom_id, values, out,
            )
            .await?
        }
        SubmitModal {
            interaction_id,
            values,
        } => interactions::submit_modal(c, core, key, u, interaction_id, values, out).await?,
        SetCommands { guild_id, commands } => {
            let app = store::app_of_bot(c, u).await?;
            registry::set_commands(c, core, app, guild_id, commands, out).await?
        }
        Respond { answer, response } => {
            interactions::respond(c, core, key, u, answer, response, out).await?
        }
        EditOriginal { answer, draft } => {
            interactions::edit_original(c, core, key, u, answer, draft, out).await?
        }
        DeleteOriginal { answer } => {
            interactions::delete_original(c, core, key, u, answer, out).await?
        }
        FollowUp { answer, draft } => {
            interactions::follow_up(c, core, key, u, answer, draft, out).await?
        }
        Send { channel_id, draft } => {
            let app = store::app_of_bot(c, u).await?;
            core.rate(format!("bot-send:{}", app.0), 60., 10.)?;
            let m =
                messages::send_draft(c, core, key, u, app, channel_id, draft, None, out).await?;
            AppResult::Message(Box::new(m))
        }
        Edit {
            channel_id,
            message_id,
            draft,
        } => {
            let app = store::app_of_bot(c, u).await?;
            let m = messages::edit_draft(c, core, key, u, app, channel_id, message_id, draft, out)
                .await?;
            AppResult::Message(Box::new(m))
        }
        ExecuteWebhook(post) => webhooks::execute(c, core, key, u, *post, out).await?,
        EditWebhookMessage {
            channel_id,
            message_id,
            post,
        } => webhooks::edit_message(c, core, key, u, channel_id, message_id, *post, out).await?,
        DeleteWebhookMessage {
            channel_id,
            message_id,
        } => webhooks::delete_message(c, core, key, u, channel_id, message_id, out).await?,
    };
    Ok(CommandResult::Apps(result))
}

/// One task for every deadline (interaction answers, expiries) and image
/// fetches; it sleeps with no deadline while nothing is pending.
pub(crate) fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let wake = {
            let mut core = app.0.core.lock().await;
            core.apps.public = app.0.config.public_url.clone();
            core.apps.wake.clone()
        };
        loop {
            let (next, jobs) = {
                let mut core = app.0.core.lock().await;
                interactions::expire(&mut core);
                (
                    core.apps.interactions.next_deadline(),
                    core.apps.images.take(),
                )
            };
            if !jobs.is_empty() {
                messages::fetch_images(&app, jobs).await;
                continue;
            }
            let sleep = async {
                match next {
                    Some(at) => {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            at.saturating_sub(now()).max(1),
                        ))
                        .await
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                _ = app.0.shutdown.cancelled() => break,
                _ = wake.notified() => {}
                _ = sleep => {}
            }
        }
    });
}

pub(crate) fn routes() -> axum::Router<App> {
    rest::routes().merge(webhooks::routes())
}

/// A refusal with a reason to show (`field` names it).
pub(crate) fn denied(field: &str, message: impl Into<String>) -> Error {
    let mut e = Error::new(ErrorCode::Forbidden);
    e.0.field = Some(field.into());
    e.0.message = message.into();
    e
}
pub(crate) fn invalid(field: &str, message: impl Into<String>) -> Error {
    let mut e = Error::new(ErrorCode::InvalidRequest);
    e.0.field = Some(field.into());
    e.0.message = message.into();
    e
}

/// Members of `g` who may manage its integrations (for `IntegrationsUpdated`).
async fn managers(c: &mut SqliteConnection, g: Id) -> Result<Vec<Id>> {
    let mut v = vec![];
    for u in guild_users(c, g).await? {
        if crate::permissions::evaluate(c, u, g, None)
            .await
            .is_ok_and(|p| p.intersects(Permissions::MANAGE_GUILD | Permissions::MANAGE_WEBHOOKS))
        {
            v.push(u);
        }
    }
    Ok(v)
}
pub(crate) async fn integrations_changed(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    g: Id,
) -> Result<()> {
    out.push((
        managers(c, g).await?,
        Event::Apps(AppEvent::IntegrationsUpdated { guild_id: g }),
    ));
    Ok(())
}
pub(crate) async fn commands_changed(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    g: Id,
) -> Result<()> {
    out.push((
        guild_users(c, g).await?,
        Event::Apps(AppEvent::CommandsUpdated { guild_id: g }),
    ));
    Ok(())
}
/// Closes every connection of `user` (token reset, app deleted).
pub(crate) fn disconnect(core: &mut Core, user: Id) {
    let keys: Vec<Vec<u8>> = core
        .streams
        .iter()
        .filter(|(_, s)| s.user == user)
        .map(|(k, _)| k.clone())
        .collect();
    for k in keys {
        if let Some(s) = core.streams.remove(&k) {
            core.apps.bots.remove(&s.id);
            if let Some(conn) = s.connection {
                let _ = conn.close.send(Some(4001));
            }
        }
    }
}
/// A bot has a live connection.
pub(crate) fn online(core: &Core, user: Id) -> bool {
    core.streams
        .values()
        .any(|s| s.user == user && s.connection.is_some())
}
