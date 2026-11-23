//! Interactions: someone uses a command, a component or a form; the bot gets
//! it over its WebSocket and answers within `INTERACTION_RESPONSE_MS`. Nothing
//! here calls out to a bot-supplied URL. State is in memory and bounded.
use super::*;
use std::collections::{BTreeSet, VecDeque};
use subtle::ConstantTimeEq;

/// Interactions kept (answered ones stay for edits and follow-ups until the
/// token expires). Every live entry counts against its user's and its app's
/// share, so no account or app can fill the table for everyone; at a cap the
/// oldest answered entry is forgotten (it can no longer be edited or followed
/// up) instead of refusing the new one.
const MAX_PENDING: usize = 4096;
const MAX_PER_APP: usize = 1024;
const MAX_PER_USER: usize = 64;
/// Waiting for the bot's answer, per user.
const MAX_PENDING_PER_USER: usize = 16;
const MAX_EPHEMERAL: usize = 4096;
const MAX_FOLLOWUPS: u8 = 10;

#[derive(Clone)]
enum Kind {
    Command { path: String, kind: CommandKind },
    Autocomplete { request: u32 },
    Component { message: Id, ephemeral: bool },
    Modal { path: String },
}

#[derive(Clone, Copy)]
enum Original {
    Message(Id),
    Ephemeral(Id),
}

#[derive(Clone)]
struct Pending {
    app: Id,
    bot: Id,
    channel: Id,
    user: Id,
    /// The invoking connection: ephemeral answers, forms and suggestions go there.
    stream: String,
    token: String,
    kind: Kind,
    created: u64,
    answered: bool,
    /// Deferred, and whether the answer will be ephemeral.
    defer: Option<bool>,
    original: Option<Original>,
    modal: Option<Modal>,
    followups: u8,
}

/// Files people passed to a bot as an attachment option: the bot may download
/// each one (`GET /api/v1/bot/attachments/{id}`), and only those, while the
/// interaction's token lasts, if it can still read the channel the command
/// was used in. Bounded; expired grants are dropped when the table is full
/// or when one is read. No timer.
#[derive(Default)]
pub(crate) struct FileGrants(HashMap<(Id, Id), (Id, u64)>);
const MAX_FILE_GRANTS: usize = 4096;
impl FileGrants {
    fn grant(&mut self, bot: Id, file: Id, channel: Id, until: u64) {
        if self.0.len() >= MAX_FILE_GRANTS {
            let t = now();
            self.0.retain(|_, (_, until)| *until > t);
        }
        if self.0.len() < MAX_FILE_GRANTS || self.0.contains_key(&(bot, file)) {
            self.0.insert((bot, file), (channel, until));
        }
    }
    /// The channel the file was passed in, while `bot` may download it.
    pub(crate) fn channel(&mut self, bot: Id, file: Id) -> Option<Id> {
        match self.0.get(&(bot, file)).copied() {
            Some((channel, until)) if until > now() => Some(channel),
            Some(_) => {
                self.0.remove(&(bot, file));
                None
            }
            None => None,
        }
    }
}

struct Ephemeral {
    app: Id,
    user: Id,
    message: EphemeralMessage,
}

#[derive(Default)]
pub(crate) struct Interactions {
    pending: HashMap<Id, Pending>,
    deadlines: BTreeSet<(u64, Id)>,
    ephemeral: HashMap<Id, Ephemeral>,
    ephemeral_order: VecDeque<(u64, Id)>,
}

impl Interactions {
    pub(crate) fn channel(&self, interaction: Id) -> Option<Id> {
        self.pending.get(&interaction).map(|p| p.channel)
    }
    pub(crate) fn next_deadline(&self) -> Option<u64> {
        let a = self.deadlines.first().map(|d| d.0);
        let b = self
            .ephemeral_order
            .front()
            .map(|(at, _)| at + INTERACTION_TOKEN_MS);
        match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
    pub(crate) fn forget_app(&mut self, app: Id) {
        self.pending.retain(|_, p| p.app != app);
        self.ephemeral.retain(|_, e| e.app != app);
    }
    pub(crate) fn forget_guild(&mut self, app: Id, _guild: Id) {
        // Pending answers in that server fail on their own; nothing is kept.
        self.pending.retain(|_, p| p.app != app || p.answered);
    }
    fn take(&mut self, answer: &InteractionAnswer, app: Id) -> Result<Pending> {
        let p = self
            .pending
            .get(&answer.interaction_id)
            .filter(|p| p.app == app && bool::from(p.token.as_bytes().ct_eq(answer.token.as_bytes())))
            .filter(|p| now() < p.created + INTERACTION_TOKEN_MS)
            .ok_or_else(|| {
                invalid(
                    "interaction",
                    "Unknown or expired interaction. Answer within 3 seconds; edit and follow up within 15 minutes.",
                )
            })?;
        Ok(p.clone())
    }
    fn put(&mut self, id: Id, p: Pending) {
        self.pending.insert(id, p);
    }
    /// Makes room for one more interaction of `user` with `app`, or says why
    /// there is none.
    fn admit(&mut self, user: Id, app: Id) -> Result<()> {
        let unanswered = self
            .pending
            .values()
            .filter(|p| p.user == user && !p.answered)
            .count();
        require(unanswered < MAX_PENDING_PER_USER, ErrorCode::RateLimited)?;
        type Share<'a> = (&'a dyn Fn(&Pending) -> bool, usize, ErrorCode);
        let shares: [Share; 3] = [
            (
                &|p: &Pending| p.user == user,
                MAX_PER_USER,
                ErrorCode::RateLimited,
            ),
            (
                &|p: &Pending| p.app == app,
                MAX_PER_APP,
                ErrorCode::Unavailable,
            ),
            (&|_: &Pending| true, MAX_PENDING, ErrorCode::Unavailable),
        ];
        for (mine, cap, code) in shares {
            if self.pending.values().filter(|p| mine(p)).count() < cap {
                continue;
            }
            let oldest = self
                .pending
                .iter()
                .filter(|(_, p)| p.answered && mine(p))
                .min_by_key(|(id, p)| (p.created, **id))
                .map(|(id, _)| *id);
            match oldest {
                Some(id) => {
                    self.pending.remove(&id);
                }
                None => return Err(Error::new(code)),
            }
        }
        Ok(())
    }
    /// Tests: pretend the interaction happened `ms` earlier.
    #[cfg(test)]
    pub(crate) fn backdate(&mut self, id: Id, ms: u64) {
        if let Some(p) = self.pending.get_mut(&id) {
            p.created = p.created.saturating_sub(ms);
        }
        self.deadlines = self
            .deadlines
            .iter()
            .map(|(at, i)| {
                if *i == id {
                    (at.saturating_sub(ms), *i)
                } else {
                    (*at, *i)
                }
            })
            .collect();
    }
}

/// Fails interactions nobody answered and forgets expired ones.
pub(crate) fn expire(core: &mut Core) {
    let t = now();
    let mut failed = vec![];
    while let Some(&(at, id)) = core.apps.interactions.deadlines.first() {
        if at > t {
            break;
        }
        core.apps.interactions.deadlines.pop_first();
        let Some(p) = core.apps.interactions.pending.get(&id) else {
            continue;
        };
        if !p.answered && t >= p.created + answer_window(&p.kind) {
            let p = core.apps.interactions.pending.remove(&id).expect("present");
            if !matches!(p.kind, Kind::Autocomplete { .. }) {
                failed.push((id, p));
            }
        } else if t >= p.created + INTERACTION_TOKEN_MS {
            core.apps.interactions.pending.remove(&id);
        }
    }
    while let Some(&(at, id)) = core.apps.interactions.ephemeral_order.front() {
        if at + INTERACTION_TOKEN_MS > t {
            break;
        }
        core.apps.interactions.ephemeral_order.pop_front();
        core.apps.interactions.ephemeral.remove(&id);
    }
    for (id, p) in failed {
        let event = Event::Apps(AppEvent::Failed {
            interaction_id: id,
            channel_id: p.channel,
            message: "The app didn't respond in time.".into(),
        });
        let _ = core.emit_filtered(&[p.user], event, |s| s.id == p.stream);
    }
}

fn answer_window(kind: &Kind) -> u64 {
    match kind {
        Kind::Autocomplete { .. } => AUTOCOMPLETE_TIMEOUT_MS,
        _ => INTERACTION_RESPONSE_MS,
    }
}

/// Records the interaction and hands it to the bot.
#[allow(clippy::too_many_arguments)]
async fn deliver(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    app: Id,
    bot: Id,
    ctx: &registry::Ctx,
    kind: Kind,
    data: InteractionData,
    message: Option<Message>,
    out: &mut Deliveries,
) -> Result<Id> {
    core.apps.interactions.admit(u, app)?;
    if !online(core, bot) {
        let name: String = sqlx::query_scalar("SELECT name FROM apps WHERE id=?")
            .bind(id(app)?)
            .fetch_one(&mut *c)
            .await?;
        let mut e = Error::new(ErrorCode::Unavailable);
        e.0.field = Some("app_offline".into());
        e.0.message = format!("{name} isn't connected right now. Try again later.");
        return Err(e);
    }
    let interaction = core.id().await?;
    let token = crate::state::token();
    let created = now();
    let stream = core
        .streams
        .get(key)
        .map(|s| s.id.clone())
        .unwrap_or_default();
    let app_permissions = crate::permissions::evaluate(c, bot, ctx.guild, Some(ctx.channel))
        .await
        .unwrap_or_default();
    let window = answer_window(&kind);
    core.apps.interactions.put(
        interaction,
        Pending {
            app,
            bot,
            channel: ctx.channel,
            user: u,
            stream,
            token: token.clone(),
            kind,
            created,
            answered: false,
            defer: None,
            original: None,
            modal: None,
            followups: 0,
        },
    );
    let d = &mut core.apps.interactions.deadlines;
    d.insert((created + window, interaction));
    d.insert((created + INTERACTION_TOKEN_MS, interaction));
    core.apps.wake.notify_one();
    if let InteractionData::Command(command) = &data {
        for f in &command.resolved.attachments {
            core.apps
                .files
                .grant(bot, f.id, ctx.channel, created + INTERACTION_TOKEN_MS);
        }
    }
    out.push((
        vec![bot],
        Event::Apps(AppEvent::Interaction(Box::new(Interaction {
            id: interaction,
            token,
            app_id: app,
            guild_id: ctx.guild,
            channel_id: ctx.channel,
            user: user(c, u).await?,
            member_permissions: ctx.rights,
            app_permissions,
            data,
            message: message.map(Box::new),
            created_at: created,
        }))),
    ));
    Ok(interaction)
}

async fn app_in(c: &mut SqliteConnection, g: Id, app: Id) -> Result<Id> {
    registry::installed(c, g)
        .await?
        .into_iter()
        .find(|a| a.0 == app)
        .map(|a| a.2)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))
}

pub(crate) async fn invoke(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    inv: Invocation,
    out: &mut Deliveries,
) -> Result<AppResult> {
    core.rate(format!("interact:{}", u.0), 60., 10.)?;
    let cmd = registry::command(c, inv.command_id).await?;
    let ctx = registry::Ctx::new(c, u, inv.channel_id).await?;
    require(
        cmd.guild_id.is_none_or(|g| g == ctx.guild),
        ErrorCode::NotFound,
    )?;
    let bot = app_in(c, ctx.guild, cmd.app_id).await?;
    let rules = registry::overrides(c, ctx.guild, cmd.app_id).await?;
    if !ctx.allows(&cmd, &rules) {
        return Err(denied("command", "You can't use this command here."));
    }
    let mut parsed = registry::parse(c, u, &ctx, &cmd, inv.options, false).await?;
    match cmd.kind {
        CommandKind::ChatInput => require(inv.target_id.is_none(), ErrorCode::InvalidRequest)?,
        CommandKind::User => {
            let t = inv
                .target_id
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            role(c, t, ctx.guild).await?;
            parsed.resolved.users.push(user(c, t).await?);
        }
        CommandKind::Message => {
            let t = inv
                .target_id
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            text(c, u, ctx.channel, Permissions::READ_HISTORY).await?;
            parsed
                .resolved
                .messages
                .push(message(c, u, ctx.channel, t, false).await?);
        }
    }
    let data = InteractionData::Command(CommandData {
        command_id: cmd.id,
        name: cmd.name.clone(),
        kind: cmd.kind,
        options: parsed.options,
        target_id: inv.target_id,
        resolved: parsed.resolved,
    });
    let path = if cmd.kind == CommandKind::ChatInput {
        parsed.path
    } else {
        cmd.name.clone()
    };
    let interaction = deliver(
        c,
        core,
        key,
        u,
        cmd.app_id,
        bot,
        &ctx,
        Kind::Command {
            path,
            kind: cmd.kind,
        },
        data,
        None,
        out,
    )
    .await?;
    Ok(AppResult::Interaction {
        interaction_id: interaction,
    })
}

pub(crate) async fn autocomplete(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    inv: Invocation,
    request: u32,
    out: &mut Deliveries,
) -> Result<AppResult> {
    core.rate(format!("autocomplete:{}", u.0), 120., 20.)?;
    let cmd = registry::command(c, inv.command_id).await?;
    let ctx = registry::Ctx::new(c, u, inv.channel_id).await?;
    require(
        cmd.guild_id.is_none_or(|g| g == ctx.guild) && cmd.kind == CommandKind::ChatInput,
        ErrorCode::NotFound,
    )?;
    let bot = app_in(c, ctx.guild, cmd.app_id).await?;
    let rules = registry::overrides(c, ctx.guild, cmd.app_id).await?;
    require(ctx.allows(&cmd, &rules), ErrorCode::Forbidden)?;
    let parsed = registry::parse(c, u, &ctx, &cmd, inv.options, true).await?;
    if !online(core, bot) {
        return Ok(AppResult::Ack);
    }
    let data = InteractionData::Autocomplete(CommandData {
        command_id: cmd.id,
        name: cmd.name.clone(),
        kind: cmd.kind,
        options: parsed.options,
        target_id: None,
        resolved: parsed.resolved,
    });
    deliver(
        c,
        core,
        key,
        u,
        cmd.app_id,
        bot,
        &ctx,
        Kind::Autocomplete { request },
        data,
        None,
        out,
    )
    .await?;
    Ok(AppResult::Ack)
}

fn find<'a>(rows: &'a [ActionRow], custom_id: &str) -> Option<&'a Component> {
    rows.iter().flat_map(|r| &r.components).find(|c| match c {
        Component::Button(b) => b.custom_id.as_deref() == Some(custom_id),
        Component::Select(s) => s.custom_id == custom_id,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn component(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    ch: Id,
    m: Id,
    custom_id: String,
    values: Vec<String>,
    out: &mut Deliveries,
) -> Result<AppResult> {
    core.rate(format!("interact:{}", u.0), 60., 10.)?;
    let ctx = registry::Ctx::new(c, u, ch).await?;
    require(
        ctx.rights.contains(Permissions::USE_APPLICATION_COMMANDS),
        ErrorCode::Forbidden,
    )?;
    let (app, rows, message, ephemeral) = match core.apps.interactions.ephemeral.get(&m) {
        Some(e) => {
            require(
                e.user == u && e.message.channel_id == ch,
                ErrorCode::NotFound,
            )?;
            (e.app, e.message.app.components.clone(), None, true)
        }
        None => {
            text(c, u, ch, Permissions::READ_HISTORY).await?;
            let msg = message(c, u, ch, m, false).await?;
            let extra = msg
                .extra
                .app
                .clone()
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            let app = extra
                .app_id
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            let bot: i64 = sqlx::query_scalar("SELECT bot_id FROM apps WHERE id=?")
                .bind(id(app)?)
                .fetch_optional(&mut *c)
                .await?
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            require(msg.author_id == Id(bot as u64), ErrorCode::NotFound)?;
            (app, extra.components, Some(msg), false)
        }
    };
    let bot = app_in(c, ctx.guild, app).await?;
    let bad = |m: &str| invalid("component", m.to_owned());
    let found =
        find(&rows, &custom_id).ok_or_else(|| bad("That control isn't on this message."))?;
    let kind = match found {
        Component::Button(b) => {
            if b.disabled || b.style == ButtonStyle::Link || !values.is_empty() {
                return Err(bad("That button can't be pressed."));
            }
            ComponentKind::Button
        }
        Component::Select(s) => {
            if s.disabled
                || values.len() < usize::from(s.min_values)
                || values.len() > usize::from(s.max_values)
                || values.iter().collect::<HashSet<_>>().len() != values.len()
            {
                return Err(bad("Choose between the select's minimum and maximum."));
            }
            for v in &values {
                let ok = match s.kind {
                    SelectKind::String => s.options.iter().any(|o| &o.value == v),
                    SelectKind::User => match parse_id(v) {
                        Some(x) => role(c, x, ctx.guild).await.is_ok(),
                        None => false,
                    },
                    SelectKind::Role => match parse_id(v) {
                        Some(x) => registry::role_in(c, ctx.guild, x).await.is_ok(),
                        None => false,
                    },
                    SelectKind::Channel => match parse_id(v) {
                        Some(x) => access(c, u, x, Permissions::VIEW_CHANNEL)
                            .await
                            .is_ok_and(|ch| ch.summary.guild_id == Some(ctx.guild)),
                        None => false,
                    },
                };
                if !ok {
                    return Err(bad("That choice isn't available."));
                }
            }
            match s.kind {
                SelectKind::String => ComponentKind::StringSelect,
                SelectKind::User => ComponentKind::UserSelect,
                SelectKind::Role => ComponentKind::RoleSelect,
                SelectKind::Channel => ComponentKind::ChannelSelect,
            }
        }
    };
    // The bot sees its own message whole.
    let shown = match &message {
        Some(_) => Some(message_for(c, bot, ch, m).await?),
        None => None,
    };
    let interaction = deliver(
        c,
        core,
        key,
        u,
        app,
        bot,
        &ctx,
        Kind::Component {
            message: m,
            ephemeral,
        },
        InteractionData::Component(ComponentData {
            custom_id,
            kind,
            values,
        }),
        shown,
        out,
    )
    .await?;
    Ok(AppResult::Interaction {
        interaction_id: interaction,
    })
}

fn parse_id(s: &str) -> Option<Id> {
    s.parse::<u64>()
        .ok()
        .filter(|v| *v > 0 && *v <= i64::MAX as u64)
        .map(Id)
}

async fn message_for(c: &mut SqliteConnection, viewer: Id, ch: Id, m: Id) -> Result<Message> {
    message(c, viewer, ch, m, false).await
}

pub(crate) async fn submit_modal(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    interaction: Id,
    values: Vec<ModalValue>,
    out: &mut Deliveries,
) -> Result<AppResult> {
    core.rate(format!("interact:{}", u.0), 60., 10.)?;
    let p = core
        .apps
        .interactions
        .pending
        .get(&interaction)
        .filter(|p| p.user == u && p.modal.is_some() && now() < p.created + INTERACTION_TOKEN_MS)
        .cloned()
        .ok_or_else(|| invalid("modal", "This form expired. Use the command again."))?;
    let modal = p.modal.clone().expect("checked");
    let bad = |m: String| invalid("modal", m);
    let mut seen = HashSet::new();
    for v in &values {
        let input = modal
            .inputs
            .iter()
            .find(|i| i.custom_id == v.custom_id)
            .ok_or_else(|| bad("That field isn't on this form.".into()))?;
        let n = v.value.chars().count();
        let max = usize::from(input.max_length.unwrap_or(MAX_TEXT_INPUT_CHARS));
        if !seen.insert(v.custom_id.clone())
            || (n > 0 && n < usize::from(input.min_length))
            || n > max
            || v.value.chars().any(|c| c.is_control() && c != '\n')
            || (input.style == TextInputStyle::Short && v.value.contains('\n'))
        {
            return Err(bad(format!("{}: check the length.", input.label)));
        }
        if input.required && v.value.trim().is_empty() {
            return Err(bad(format!("{} is required.", input.label)));
        }
    }
    if let Some(missing) = modal
        .inputs
        .iter()
        .find(|i| i.required && !seen.contains(&i.custom_id))
    {
        return Err(bad(format!("{} is required.", missing.label)));
    }
    // One submission per form.
    if let Some(old) = core.apps.interactions.pending.get_mut(&interaction) {
        old.modal = None;
    }
    let ctx = registry::Ctx::new(c, u, p.channel).await?;
    let bot = app_in(c, ctx.guild, p.app).await?;
    let path = match &p.kind {
        Kind::Command { path, .. } | Kind::Modal { path } => path.clone(),
        Kind::Component { .. } | Kind::Autocomplete { .. } => String::new(),
    };
    let next = deliver(
        c,
        core,
        key,
        u,
        p.app,
        bot,
        &ctx,
        Kind::Modal { path },
        InteractionData::ModalSubmit(ModalData {
            custom_id: modal.custom_id,
            values,
        }),
        None,
        out,
    )
    .await?;
    Ok(AppResult::Interaction {
        interaction_id: next,
    })
}

fn check_modal(m: &Modal) -> Result<()> {
    let bad = |s: &str| Err(invalid("modal", s.to_owned()));
    if !(1..=MAX_CUSTOM_ID_CHARS).contains(&m.custom_id.chars().count())
        || !(1..=45).contains(&m.title.chars().count())
        || !(1..=MAX_MODAL_INPUTS).contains(&m.inputs.len())
    {
        return bad("A form has a custom_id, a title of up to 45 characters and 1-5 fields.");
    }
    let mut ids = HashSet::new();
    for i in &m.inputs {
        let max = i.max_length.unwrap_or(MAX_TEXT_INPUT_CHARS);
        if !ids.insert(i.custom_id.clone())
            || !(1..=MAX_CUSTOM_ID_CHARS).contains(&i.custom_id.chars().count())
            || !(1..=45).contains(&i.label.chars().count())
            || max == 0
            || max > MAX_TEXT_INPUT_CHARS
            || i.min_length > max
            || i.placeholder
                .as_ref()
                .is_some_and(|p| p.chars().count() > 100)
            || i.value
                .as_ref()
                .is_some_and(|v| v.chars().count() > usize::from(max))
        {
            return bad(
                "Form fields: unique custom_ids, labels up to 45 characters, lengths up to 4000.",
            );
        }
    }
    Ok(())
}

/// An answer only the person who used the command sees (not stored).
async fn ephemeral(
    c: &mut SqliteConnection,
    core: &mut Core,
    p: &Pending,
    interaction: Id,
    replace: Option<Id>,
    draft: &AppMessageDraft,
    thinking: bool,
) -> Result<EphemeralMessage> {
    let channel = channel(c, p.channel).await?;
    messages::check_components(&draft.components)?;
    let (mut embeds, _) = messages::check_embeds(&draft.embeds)?;
    // Not fetched: an ephemeral answer is not stored.
    for e in &mut embeds {
        e.image = None;
        e.thumbnail = None;
    }
    let none = AllowedMentions::default();
    let (content, _) = messages::mentions(c, p.bot, &channel, &draft.content, Some(&none)).await?;
    require(
        thinking
            || (content.chars().count() <= MAX_MESSAGE_CHARS
                && (!content.trim().is_empty()
                    || !embeds.is_empty()
                    || !draft.components.is_empty())),
        ErrorCode::InvalidRequest,
    )?;
    let id = match replace {
        Some(id) => id,
        None => core.id().await?,
    };
    let interaction_meta = meta(p);
    let message = EphemeralMessage {
        id,
        channel_id: p.channel,
        interaction_id: interaction,
        author: user(c, p.bot).await?,
        content,
        app: AppMessage {
            app_id: Some(p.app),
            embeds,
            components: draft.components.clone(),
            interaction: interaction_meta,
            ..Default::default()
        },
        thinking,
        created_at: now(),
    };
    let t = &mut core.apps.interactions;
    if !t.ephemeral.contains_key(&id) {
        while t.ephemeral.len() >= MAX_EPHEMERAL {
            match t.ephemeral_order.pop_front() {
                Some((_, old)) => {
                    t.ephemeral.remove(&old);
                }
                None => break,
            }
        }
        t.ephemeral_order.push_back((now(), id));
    }
    t.ephemeral.insert(
        id,
        Ephemeral {
            app: p.app,
            user: p.user,
            message: message.clone(),
        },
    );
    core.apps.wake.notify_one();
    let stream = p.stream.clone();
    core.emit_filtered(
        &[p.user],
        Event::Apps(AppEvent::Ephemeral(Box::new(message.clone()))),
        |s| s.id == stream,
    )?;
    Ok(message)
}

fn meta(p: &Pending) -> Option<InteractionMeta> {
    match &p.kind {
        Kind::Command { path, kind } => Some(InteractionMeta {
            name: path.clone(),
            user_id: p.user,
            kind: *kind,
        }),
        Kind::Modal { path } if !path.is_empty() => Some(InteractionMeta {
            name: path.clone(),
            user_id: p.user,
            kind: CommandKind::ChatInput,
        }),
        _ => None,
    }
}

fn remove_ephemeral(core: &mut Core, p: &Pending, id: Id) -> Result<()> {
    core.apps.interactions.ephemeral.remove(&id);
    let stream = p.stream.clone();
    core.emit_filtered(
        &[p.user],
        Event::Apps(AppEvent::EphemeralRemoved {
            channel_id: p.channel,
            id,
        }),
        |s| s.id == stream,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn respond(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    bot: Id,
    answer: InteractionAnswer,
    response: InteractionResponse,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let app = store::app_of_bot(c, bot).await?;
    let mut p = core.apps.interactions.take(&answer, app)?;
    require(!p.answered, ErrorCode::Conflict)?;
    if now() >= p.created + answer_window(&p.kind) {
        return Err(invalid(
            "interaction",
            "Too late: answer within 3 seconds, or defer first.",
        ));
    }
    let interaction = answer.interaction_id;
    let component = match &p.kind {
        Kind::Component { message, ephemeral } => Some((*message, *ephemeral)),
        _ => None,
    };
    let autocomplete = matches!(p.kind, Kind::Autocomplete { .. });
    let bad = |m: &str| invalid("response", m.to_owned());
    if autocomplete != matches!(response, InteractionResponse::Autocomplete { .. }) {
        return Err(bad(
            "Autocomplete is answered with choices, and only autocomplete is.",
        ));
    }
    let result = match response {
        InteractionResponse::Message(draft) => {
            if draft.ephemeral {
                let m = ephemeral(c, core, &p, interaction, None, &draft, false).await?;
                p.original = Some(Original::Ephemeral(m.id));
                AppResult::Ephemeral(Box::new(m))
            } else {
                let m =
                    messages::send_draft(c, core, key, bot, app, p.channel, draft, meta(&p), out)
                        .await?;
                p.original = Some(Original::Message(m.id));
                AppResult::Message(Box::new(m))
            }
        }
        InteractionResponse::Defer { ephemeral: private } => {
            let m = ephemeral(
                c,
                core,
                &p,
                interaction,
                None,
                &AppMessageDraft::default(),
                true,
            )
            .await?;
            p.defer = Some(private);
            p.original = Some(Original::Ephemeral(m.id));
            AppResult::Ack
        }
        InteractionResponse::DeferUpdate => {
            let (m, eph) = component.ok_or_else(|| bad("Only components can defer an update."))?;
            p.original = Some(if eph {
                Original::Ephemeral(m)
            } else {
                Original::Message(m)
            });
            AppResult::Ack
        }
        InteractionResponse::UpdateMessage(draft) => {
            let (m, eph) =
                component.ok_or_else(|| bad("Only components can update their message."))?;
            p.original = Some(if eph {
                Original::Ephemeral(m)
            } else {
                Original::Message(m)
            });
            update_original(c, core, key, bot, app, &p, interaction, draft, out).await?
        }
        InteractionResponse::Modal(modal) => {
            if matches!(p.kind, Kind::Modal { .. }) {
                return Err(bad("A form can't open another form."));
            }
            check_modal(&modal)?;
            p.modal = Some(modal.clone());
            let stream = p.stream.clone();
            core.emit_filtered(
                &[p.user],
                Event::Apps(AppEvent::Modal {
                    interaction_id: interaction,
                    app_id: app,
                    channel_id: p.channel,
                    modal,
                }),
                |s| s.id == stream,
            )?;
            AppResult::Ack
        }
        InteractionResponse::Autocomplete { choices } => {
            let Kind::Autocomplete { request } = p.kind else {
                return Err(bad("Not an autocomplete interaction."));
            };
            if choices.len() > MAX_AUTOCOMPLETE_CHOICES
                || choices.iter().any(|c| {
                    !(1..=MAX_COMMAND_CHOICE_CHARS).contains(&c.name.chars().count())
                        || c.name.chars().any(char::is_control)
                        || !matches!(&c.value, OptionValue::String(s) if s.chars().count() <= 100)
                            && !matches!(c.value, OptionValue::Integer(_))
                            && !matches!(c.value, OptionValue::Number(n) if n.0.is_finite())
                })
            {
                return Err(bad("Up to 25 choices with names of up to 100 characters."));
            }
            let stream = p.stream.clone();
            core.emit_filtered(
                &[p.user],
                Event::Apps(AppEvent::Autocomplete { request, choices }),
                |s| s.id == stream,
            )?;
            core.apps.interactions.pending.remove(&interaction);
            return Ok(AppResult::Ack);
        }
    };
    p.answered = true;
    core.apps.interactions.put(interaction, p);
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
async fn update_original(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    bot: Id,
    app: Id,
    p: &Pending,
    interaction: Id,
    draft: AppMessageDraft,
    out: &mut Deliveries,
) -> Result<AppResult> {
    match p.original {
        Some(Original::Ephemeral(id)) => {
            let m = ephemeral(c, core, p, interaction, Some(id), &draft, false).await?;
            Ok(AppResult::Ephemeral(Box::new(m)))
        }
        Some(Original::Message(m)) => {
            let m = messages::edit_draft(c, core, key, bot, app, p.channel, m, draft, out).await?;
            Ok(AppResult::Message(Box::new(m)))
        }
        None => Err(invalid("response", "Answer the interaction first.")),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn edit_original(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    bot: Id,
    answer: InteractionAnswer,
    draft: AppMessageDraft,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let app = store::app_of_bot(c, bot).await?;
    let mut p = core.apps.interactions.take(&answer, app)?;
    require(p.answered, ErrorCode::InvalidRequest)?;
    let interaction = answer.interaction_id;
    // A public answer that was deferred: the placeholder becomes the message.
    if let (Some(false), Some(Original::Ephemeral(placeholder))) = (p.defer, p.original) {
        let m =
            messages::send_draft(c, core, key, bot, app, p.channel, draft, meta(&p), out).await?;
        remove_ephemeral(core, &p, placeholder)?;
        p.defer = None;
        p.original = Some(Original::Message(m.id));
        core.apps.interactions.put(interaction, p);
        return Ok(AppResult::Message(Box::new(m)));
    }
    p.defer = None;
    let r = update_original(c, core, key, bot, app, &p, interaction, draft, out).await?;
    core.apps.interactions.put(interaction, p);
    Ok(r)
}

pub(crate) async fn delete_original(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    bot: Id,
    answer: InteractionAnswer,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let app = store::app_of_bot(c, bot).await?;
    let mut p = core.apps.interactions.take(&answer, app)?;
    match p.original.take() {
        Some(Original::Ephemeral(id)) => remove_ephemeral(core, &p, id)?,
        Some(Original::Message(m)) => {
            Box::pin(crate::commands::mutate(
                c,
                core,
                key,
                bot,
                Command::DeleteMessage {
                    channel_id: p.channel,
                    message_id: m,
                },
                out,
            ))
            .await?;
        }
        None => return Err(invalid("response", "There is no answer to delete.")),
    }
    core.apps.interactions.put(answer.interaction_id, p);
    Ok(AppResult::Ack)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn follow_up(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    bot: Id,
    answer: InteractionAnswer,
    draft: AppMessageDraft,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let app = store::app_of_bot(c, bot).await?;
    let mut p = core.apps.interactions.take(&answer, app)?;
    require(
        p.answered && p.followups < MAX_FOLLOWUPS,
        ErrorCode::LimitExceeded,
    )?;
    p.followups += 1;
    let interaction = answer.interaction_id;
    let result = if draft.ephemeral {
        let m = ephemeral(c, core, &p, interaction, None, &draft, false).await?;
        AppResult::Ephemeral(Box::new(m))
    } else {
        let m =
            messages::send_draft(c, core, key, bot, app, p.channel, draft, meta(&p), out).await?;
        AppResult::Message(Box::new(m))
    };
    core.apps.interactions.put(interaction, p);
    Ok(result)
}

#[cfg(test)]
mod caps {
    use super::*;

    fn answered(user: u64, app: u64, created: u64) -> Pending {
        Pending {
            app: Id(app),
            bot: Id(app + 1000),
            channel: Id(11),
            user: Id(user),
            stream: String::new(),
            token: String::new(),
            kind: Kind::Autocomplete { request: 0 },
            created,
            answered: true,
            defer: None,
            original: None,
            modal: None,
            followups: 0,
        }
    }

    /// A few accounts whose bot answers at once cannot fill the table: each
    /// account and each app holds a bounded share, and at the cap its own
    /// oldest answered entry makes room. Someone else's command still works.
    #[test]
    fn answered_interactions_count_and_one_user_or_app_cannot_fill_the_table() {
        let mut t = Interactions::default();
        let mut next = 1u64;
        // Five accounts, one bot, 2,000 instantly answered interactions each.
        for round in 0..2000u64 {
            for user in 1..=5u64 {
                t.admit(Id(user), Id(7)).unwrap();
                t.put(Id(next), answered(user, 7, round));
                next += 1;
            }
        }
        assert_eq!(t.pending.len(), 5 * MAX_PER_USER);
        // Thousands of accounts with that one bot: capped at its share.
        for user in 100..3000u64 {
            t.admit(Id(user), Id(7)).unwrap();
            t.put(Id(next), answered(user, 7, 5000 + user));
            next += 1;
        }
        let of =
            |t: &Interactions, app: u64| t.pending.values().filter(|p| p.app == Id(app)).count();
        assert_eq!(of(&t, 7), MAX_PER_APP);
        // Everyone else's app is unaffected.
        assert!(t.admit(Id(9), Id(8)).is_ok());
        // The newest of the user's entries survive; the oldest went.
        let kept: Vec<u64> = t
            .pending
            .values()
            .filter(|p| p.user == Id(1))
            .map(|p| p.created)
            .collect();
        assert!(
            kept.iter().all(|c| *c >= 2000 - MAX_PER_USER as u64),
            "{kept:?}"
        );
        // Still: at most 16 waiting for an answer per person.
        let mut w = Interactions::default();
        for i in 0..MAX_PENDING_PER_USER as u64 {
            w.admit(Id(1), Id(7)).unwrap();
            let mut p = answered(1, 7, i);
            p.answered = false;
            w.put(Id(i + 1), p);
        }
        assert_eq!(
            w.admit(Id(1), Id(7)).unwrap_err().0.code,
            ErrorCode::RateLimited
        );
    }
}
