//! Application commands: registration, the `/` picker's catalog, per-server
//! overrides, and checking what a person typed against a command.
use super::*;
use crate::moderation::audit;

fn chat_name(s: &str) -> bool {
    (1..=MAX_COMMAND_NAME_CHARS).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}
fn text_ok(s: &str, min: usize, max: usize) -> bool {
    let n = s.chars().count();
    (min..=max).contains(&n) && !s.chars().any(char::is_control)
}

fn check_value(kind: OptionKind, v: &OptionValue) -> bool {
    match v {
        OptionValue::String(s) => {
            kind == OptionKind::String && text_ok(s, 1, MAX_COMMAND_CHOICE_CHARS)
        }
        OptionValue::Integer(_) => kind == OptionKind::Integer,
        OptionValue::Number(n) => kind == OptionKind::Number && n.0.is_finite(),
        _ => false,
    }
}

fn check_options(options: &[CommandOption], depth: u8) -> Result<()> {
    let bad = |m: &str| Err(invalid("commands", m.to_owned()));
    if options.len() > MAX_COMMAND_OPTIONS {
        return bad("A command can have up to 25 options.");
    }
    let subs = options
        .iter()
        .filter(|o| matches!(o.kind, OptionKind::SubCommand | OptionKind::SubCommandGroup))
        .count();
    if subs != 0 && subs != options.len() {
        return bad("Subcommands can't be mixed with other options.");
    }
    let mut names = HashSet::new();
    let mut optional = false;
    for o in options {
        if !chat_name(&o.name) || !names.insert(o.name.as_str()) {
            return bad("Option names are 1-32 lowercase letters, digits, - or _, and unique.");
        }
        if !text_ok(&o.description, 1, MAX_COMMAND_DESCRIPTION_CHARS) {
            return bad("Every option needs a description (up to 100 characters).");
        }
        match o.kind {
            OptionKind::SubCommandGroup => {
                if depth != 0
                    || o.options.is_empty()
                    || o.options.iter().any(|s| s.kind != OptionKind::SubCommand)
                {
                    return bad("A group holds subcommands, at the top level only.");
                }
                check_options(&o.options, depth + 1)?;
            }
            OptionKind::SubCommand => {
                if depth > 1
                    || o.options.iter().any(|s| {
                        matches!(s.kind, OptionKind::SubCommand | OptionKind::SubCommandGroup)
                    })
                {
                    return bad("Subcommands can't contain subcommands.");
                }
                check_options(&o.options, depth + 1)?;
            }
            _ => {
                if !o.options.is_empty() {
                    return bad("Only subcommands have options.");
                }
                if o.required && optional {
                    return bad("Required options come before optional ones.");
                }
                optional |= !o.required;
                if o.choices.len() > MAX_OPTION_CHOICES
                    || (!o.choices.is_empty() && o.autocomplete)
                    || (!o.choices.is_empty()
                        && !matches!(
                            o.kind,
                            OptionKind::String | OptionKind::Integer | OptionKind::Number
                        ))
                    || o.choices.iter().any(|c| {
                        !text_ok(&c.name, 1, MAX_COMMAND_CHOICE_CHARS)
                            || !check_value(o.kind, &c.value)
                    })
                {
                    return bad("Choices: up to 25 named values of the option's type, and not with autocomplete.");
                }
                if o.autocomplete
                    && !matches!(
                        o.kind,
                        OptionKind::String | OptionKind::Integer | OptionKind::Number
                    )
                {
                    return bad("Only text and number options can autocomplete.");
                }
                let numeric = matches!(o.kind, OptionKind::Integer | OptionKind::Number);
                if (o.min_value.is_some() || o.max_value.is_some()) && !numeric
                    || [o.min_value, o.max_value]
                        .iter()
                        .flatten()
                        .any(|n| !n.0.is_finite())
                    || matches!((o.min_value, o.max_value), (Some(a), Some(b)) if a.0 > b.0)
                {
                    return bad("Minimum and maximum values are for numbers, with min ≤ max.");
                }
                if (o.min_length.is_some() || o.max_length.is_some())
                    && o.kind != OptionKind::String
                    || o.max_length.is_some_and(|m| m == 0 || m > 6000)
                    || o.min_length.is_some_and(|m| m > 6000)
                    || matches!((o.min_length, o.max_length), (Some(a), Some(b)) if a > b)
                {
                    return bad("Lengths are for text options: 0-6000, with min ≤ max.");
                }
            }
        }
    }
    Ok(())
}

fn check(drafts: &[CommandDraft]) -> Result<()> {
    if drafts.len() > MAX_COMMANDS_PER_SCOPE {
        return Err(invalid(
            "commands",
            "Register up to 100 commands per scope.",
        ));
    }
    let mut seen = HashSet::new();
    for d in drafts {
        let ok = match d.kind {
            CommandKind::ChatInput => {
                chat_name(&d.name) && text_ok(&d.description, 1, MAX_COMMAND_DESCRIPTION_CHARS)
            }
            _ => {
                text_ok(&d.name, 1, MAX_COMMAND_NAME_CHARS)
                    && d.description.is_empty()
                    && d.options.is_empty()
            }
        };
        if !ok {
            return Err(invalid(
                "commands",
                format!("Command \"{}\": slash commands need a lowercase name and a description; menu commands a name only.", d.name.chars().take(40).collect::<String>()),
            ));
        }
        if !seen.insert((d.kind, d.name.clone())) {
            return Err(invalid("commands", "Command names are unique per kind."));
        }
        check_options(&d.options, 0)?;
    }
    let bytes = serde_json::to_vec(drafts).map_err(|_| Error::new(ErrorCode::Internal))?;
    require(
        bytes.len() <= MAX_COMMANDS_BYTES,
        ErrorCode::PayloadTooLarge,
    )
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    #[serde(default)]
    description: String,
    #[serde(default)]
    options: Vec<CommandOption>,
    #[serde(default)]
    default_member_permissions: Option<Permissions>,
}

fn row(r: &sqlx::sqlite::SqliteRow) -> Result<ApplicationCommand> {
    let stored: Stored = serde_json::from_str(&r.get::<String, _>("data"))
        .map_err(|_| Error::new(ErrorCode::Internal))?;
    Ok(ApplicationCommand {
        id: Id(r.get::<i64, _>("id") as u64),
        app_id: Id(r.get::<i64, _>("app_id") as u64),
        guild_id: r.get::<Option<i64>, _>("guild_id").map(|v| Id(v as u64)),
        kind: en(r.get("kind"))?,
        name: r.get("name"),
        description: stored.description,
        options: stored.options,
        default_member_permissions: stored.default_member_permissions,
    })
}

/// The app's commands for one scope (`None`: global) or, with a server, both.
pub(crate) async fn commands(
    c: &mut SqliteConnection,
    app: Id,
    guild: Option<Id>,
) -> Result<Vec<ApplicationCommand>> {
    sqlx::query("SELECT * FROM app_commands WHERE app_id=? AND (guild_id IS NULL OR guild_id IS ?) ORDER BY guild_id IS NOT NULL,position,id")
        .bind(id(app)?)
        .bind(opt(guild)?)
        .fetch_all(&mut *c)
        .await?
        .iter()
        .map(row)
        .collect()
}
pub(crate) async fn command(c: &mut SqliteConnection, cmd: Id) -> Result<ApplicationCommand> {
    let r = sqlx::query("SELECT * FROM app_commands WHERE id=?")
        .bind(id(cmd)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    row(&r)
}

/// `SetCommands`: replaces one scope's commands; same-named commands keep
/// their IDs (and so their per-server overrides).
pub(crate) async fn set_commands(
    c: &mut SqliteConnection,
    core: &mut Core,
    app: Id,
    guild: Option<Id>,
    drafts: Vec<CommandDraft>,
    out: &mut Deliveries,
) -> Result<AppResult> {
    core.rate(format!("app-commands:{}", app.0), 10., 5.)?;
    check(&drafts)?;
    if let Some(g) = guild {
        let installed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM app_installs WHERE app_id=? AND guild_id=?)",
        )
        .bind(id(app)?)
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
        require(installed, ErrorCode::NotFound)?;
    }
    let existing: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id,kind,name FROM app_commands WHERE app_id=? AND guild_id IS ?")
            .bind(id(app)?)
            .bind(opt(guild)?)
            .fetch_all(&mut *c)
            .await?;
    let mut kept = HashSet::new();
    for (position, d) in drafts.iter().enumerate() {
        let kind = enum_str(d.kind);
        let data = serde_json::to_string(&Stored {
            description: d.description.clone(),
            options: d.options.clone(),
            default_member_permissions: d
                .default_member_permissions
                .map(|p| Permissions::from_bits_truncate(p.bits()) - Permissions::OWNER),
        })
        .map_err(|_| Error::new(ErrorCode::Internal))?;
        if let Some((cid, _, _)) = existing.iter().find(|(_, k, n)| *k == kind && *n == d.name) {
            sqlx::query("UPDATE app_commands SET data=?,position=? WHERE id=?")
                .bind(data)
                .bind(position as i64)
                .bind(cid)
                .execute(&mut *c)
                .await?;
            kept.insert(*cid);
        } else {
            let cid = core.id().await?;
            sqlx::query("INSERT INTO app_commands(id,app_id,guild_id,kind,name,data,position) VALUES(?,?,?,?,?,?,?)")
                .bind(id(cid)?)
                .bind(id(app)?)
                .bind(opt(guild)?)
                .bind(kind)
                .bind(&d.name)
                .bind(data)
                .bind(position as i64)
                .execute(&mut *c)
                .await?;
        }
    }
    for (cid, _, _) in existing.iter().filter(|(cid, _, _)| !kept.contains(cid)) {
        sqlx::query("DELETE FROM app_commands WHERE id=?")
            .bind(cid)
            .execute(&mut *c)
            .await?;
        sqlx::query("DELETE FROM app_command_permissions WHERE target_id=?")
            .bind(cid)
            .execute(&mut *c)
            .await?;
    }
    let guilds: Vec<i64> = match guild {
        Some(g) => vec![id(g)?],
        None => {
            sqlx::query_scalar("SELECT guild_id FROM app_installs WHERE app_id=?")
                .bind(id(app)?)
                .fetch_all(&mut *c)
                .await?
        }
    };
    for g in guilds {
        commands_changed(c, out, Id(g as u64)).await?;
    }
    let list = sqlx::query(
        "SELECT * FROM app_commands WHERE app_id=? AND guild_id IS ? ORDER BY position,id",
    )
    .bind(id(app)?)
    .bind(opt(guild)?)
    .fetch_all(&mut *c)
    .await?
    .iter()
    .map(row)
    .collect::<Result<Vec<_>>>()?;
    Ok(AppResult::Commands(list))
}

pub(crate) async fn overrides(
    c: &mut SqliteConnection,
    g: Id,
    app: Id,
) -> Result<Vec<CommandOverrides>> {
    let rows = sqlx::query("SELECT target_id,kind,id,allow FROM app_command_permissions WHERE guild_id=? AND app_id=? ORDER BY target_id,kind,id")
        .bind(id(g)?)
        .bind(id(app)?)
        .fetch_all(&mut *c)
        .await?;
    let mut out: Vec<CommandOverrides> = vec![];
    for r in rows {
        let target = Id(r.get::<i64, _>("target_id") as u64);
        let t = CommandTarget {
            kind: en(r.get("kind"))?,
            id: Id(r.get::<i64, _>("id") as u64),
            allow: r.get("allow"),
        };
        match out.last_mut() {
            Some(o) if o.id == target => o.targets.push(t),
            _ => out.push(CommandOverrides {
                id: target,
                targets: vec![t],
            }),
        }
    }
    Ok(out)
}

pub(crate) async fn set_permissions(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    target: Id,
    targets: Vec<CommandTarget>,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let p = crate::permissions::evaluate(c, u, g, None).await?;
    require(p.contains(Permissions::MANAGE_GUILD), ErrorCode::Forbidden)?;
    require(targets.len() <= 100, ErrorCode::LimitExceeded)?;
    // The target is an installed app, or one of its commands visible here.
    let app: Option<i64> =
        sqlx::query_scalar("SELECT app_id FROM app_installs WHERE guild_id=? AND app_id=?")
            .bind(id(g)?)
            .bind(id(target)?)
            .fetch_optional(&mut *c)
            .await?;
    let (app, name) = match app {
        Some(a) => (Id(a as u64), "all commands".to_owned()),
        None => {
            let cmd = command(c, target).await?;
            require(cmd.guild_id.is_none_or(|cg| cg == g), ErrorCode::NotFound)?;
            let installed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM app_installs WHERE guild_id=? AND app_id=?)",
            )
            .bind(id(g)?)
            .bind(id(cmd.app_id)?)
            .fetch_one(&mut *c)
            .await?;
            require(installed, ErrorCode::NotFound)?;
            (cmd.app_id, format!("/{}", cmd.name))
        }
    };
    let mut seen = HashSet::new();
    for t in &targets {
        require(seen.insert((t.kind, t.id)), ErrorCode::InvalidRequest)?;
        match t.kind {
            TargetKind::Role => {
                role_in(c, g, t.id).await?;
            }
            TargetKind::Channel => {
                let ch = channel(c, t.id)
                    .await
                    .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
                require(ch.summary.guild_id == Some(g), ErrorCode::InvalidRequest)?;
            }
        }
    }
    let before = overrides(c, g, app).await?;
    let describe = |list: &[CommandTarget]| {
        list.iter()
            .map(|t| {
                format!(
                    "{} {} {}",
                    if t.allow { "allow" } else { "deny" },
                    enum_str(t.kind),
                    t.id.0
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let old = before
        .iter()
        .find(|o| o.id == target)
        .map(|o| describe(&o.targets))
        .unwrap_or_default();
    sqlx::query("DELETE FROM app_command_permissions WHERE guild_id=? AND target_id=?")
        .bind(id(g)?)
        .bind(id(target)?)
        .execute(&mut *c)
        .await?;
    for t in &targets {
        sqlx::query("INSERT INTO app_command_permissions(guild_id,app_id,target_id,kind,id,allow) VALUES(?,?,?,?,?,?)")
            .bind(id(g)?)
            .bind(id(app)?)
            .bind(id(target)?)
            .bind(enum_str(t.kind))
            .bind(id(t.id)?)
            .bind(t.allow)
            .execute(&mut *c)
            .await?;
    }
    let app_name: String = sqlx::query_scalar("SELECT name FROM apps WHERE id=?")
        .bind(id(app)?)
        .fetch_one(&mut *c)
        .await?;
    audit::write(
        c,
        u,
        audit::Entry::new(
            g,
            AuditAction::CommandPermissionsUpdate,
            AuditTarget::App,
            Some(app),
        )
        .named(app_name)
        .change("command", None, Some(name))
        .change(
            "overrides",
            Some(old).filter(|s| !s.is_empty()),
            Some(describe(&targets)).filter(|s| !s.is_empty()),
        ),
    )
    .await?;
    integrations_changed(c, out, g).await?;
    commands_changed(c, out, g).await?;
    Ok(AppResult::Ack)
}

/// Who may use commands in one channel.
pub(crate) struct Ctx {
    pub guild: Id,
    pub channel: Id,
    pub lineage: Vec<Id>,
    pub roles: Vec<Id>,
    pub rights: Permissions,
}
impl Ctx {
    pub(crate) async fn new(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<Self> {
        let channel = access(c, u, ch, Permissions::VIEW_CHANNEL).await?;
        let guild = channel
            .summary
            .guild_id
            .ok_or_else(|| invalid("channel", "Apps work in servers."))?;
        Ok(Self {
            guild,
            channel: ch,
            lineage: crate::permissions::lineage(c, guild, ch).await?,
            roles: crate::permissions::role_ids(c, guild, u).await?,
            rights: crate::permissions::evaluate(c, u, guild, Some(ch)).await?,
        })
    }
    /// The overrides decide when they say anything; then the command's
    /// default permissions. Manage Server can always use every command.
    pub(crate) fn allows(&self, cmd: &ApplicationCommand, rules: &[CommandOverrides]) -> bool {
        if !self.rights.contains(Permissions::USE_APPLICATION_COMMANDS) {
            return false;
        }
        if self.rights.contains(Permissions::MANAGE_GUILD) {
            return true;
        }
        let own = rules.iter().find(|o| o.id == cmd.id);
        let app = rules.iter().find(|o| o.id == cmd.app_id);
        let pick = |kind: TargetKind| -> Vec<CommandTarget> {
            let of = |o: Option<&CommandOverrides>| -> Vec<CommandTarget> {
                o.map(|o| {
                    o.targets
                        .iter()
                        .filter(|t| t.kind == kind)
                        .copied()
                        .collect()
                })
                .unwrap_or_default()
            };
            let v = of(own);
            if v.is_empty() {
                of(app)
            } else {
                v
            }
        };
        // Channels: the nearest channel in the lineage with a rule wins.
        let channels = pick(TargetKind::Channel);
        if let Some(rule) = self
            .lineage
            .iter()
            .rev()
            .find_map(|ch| channels.iter().find(|t| t.id == *ch))
        {
            if !rule.allow {
                return false;
            }
        }
        // Roles: any allowed role grants; otherwise any denied role refuses.
        let roles = pick(TargetKind::Role);
        let mine: Vec<&CommandTarget> = roles
            .iter()
            .filter(|t| self.roles.contains(&t.id))
            .collect();
        if mine.iter().any(|t| t.allow) {
            return true;
        }
        if mine.iter().any(|t| !t.allow) {
            return false;
        }
        cmd.default_member_permissions
            .is_none_or(|p| self.rights.contains(p))
    }
}

/// Apps installed in the server whose bot is still a member, not disabled.
pub(crate) async fn installed(c: &mut SqliteConnection, g: Id) -> Result<Vec<(Id, String, Id)>> {
    if store::creation(c).await? == AppCreation::Disabled {
        return Ok(vec![]);
    }
    Ok(sqlx::query("SELECT a.id,a.name,a.bot_id FROM app_installs i JOIN apps a ON a.id=i.app_id JOIN guild_members m ON m.guild_id=i.guild_id AND m.user_id=a.bot_id WHERE i.guild_id=? AND a.disabled=0 ORDER BY a.name,a.id")
        .bind(id(g)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| {
            (
                Id(r.get::<i64, _>("id") as u64),
                r.get("name"),
                Id(r.get::<i64, _>("bot_id") as u64),
            )
        })
        .collect())
}

/// `GET /api/v1/channels/{id}/commands`.
pub(crate) async fn catalog(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<CommandCatalog> {
    let ctx = Ctx::new(c, u, ch).await?;
    let mut result = CommandCatalog {
        channel_id: ch,
        apps: vec![],
    };
    if !ctx.rights.contains(Permissions::USE_APPLICATION_COMMANDS) {
        return Ok(result);
    }
    for (app, name, bot) in installed(c, ctx.guild).await? {
        let rules = overrides(c, ctx.guild, app).await?;
        let commands: Vec<ApplicationCommand> = commands(c, app, Some(ctx.guild))
            .await?
            .into_iter()
            .filter(|cmd| ctx.allows(cmd, &rules))
            .collect();
        if commands.is_empty() {
            continue;
        }
        let icon = user(c, bot).await?.avatar_id;
        result.apps.push(CatalogApp {
            app_id: app,
            name,
            icon_id: icon,
            bot_id: bot,
            commands,
        });
    }
    Ok(result)
}

/// What a person typed, checked against the command's definition. Resolves
/// users, roles, channels and files, so the bot never has to trust IDs.
pub(crate) struct Parsed {
    pub options: Vec<InvokedOption>,
    pub resolved: Resolved,
    /// The command as used, e.g. `weather forecast`.
    pub path: String,
}

pub(crate) async fn parse(
    c: &mut SqliteConnection,
    u: Id,
    ctx: &Ctx,
    cmd: &ApplicationCommand,
    given: Vec<InvokedOption>,
    autocomplete: bool,
) -> Result<Parsed> {
    let mut resolved = Resolved::default();
    let mut path = cmd.name.clone();
    let options = level(
        c,
        u,
        ctx,
        &cmd.options,
        given,
        autocomplete,
        &mut resolved,
        &mut path,
        0,
    )
    .await?;
    if autocomplete {
        let focused = count_focused(&options);
        require(focused == 1, ErrorCode::InvalidRequest)?;
    }
    Ok(Parsed {
        options,
        resolved,
        path,
    })
}
fn count_focused(o: &[InvokedOption]) -> usize {
    o.iter()
        .map(|o| usize::from(o.focused) + count_focused(&o.options))
        .sum()
}

#[allow(clippy::too_many_arguments)]
fn level<'a>(
    c: &'a mut SqliteConnection,
    u: Id,
    ctx: &'a Ctx,
    defs: &'a [CommandOption],
    given: Vec<InvokedOption>,
    autocomplete: bool,
    resolved: &'a mut Resolved,
    path: &'a mut String,
    depth: u8,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<InvokedOption>>> + Send + 'a>> {
    Box::pin(async move {
        let bad = |m: String| invalid("options", m);
        require(
            depth < 3 && given.len() <= MAX_COMMAND_OPTIONS,
            ErrorCode::InvalidRequest,
        )?;
        let subs = defs
            .iter()
            .any(|d| matches!(d.kind, OptionKind::SubCommand | OptionKind::SubCommandGroup));
        if subs {
            let [one]: [InvokedOption; 1] = given
                .try_into()
                .map_err(|_| bad("Choose one subcommand.".into()))?;
            let def = defs
                .iter()
                .find(|d| d.name == one.name)
                .ok_or_else(|| bad(format!("Unknown subcommand {}.", one.name)))?;
            require(one.value.is_none(), ErrorCode::InvalidRequest)?;
            path.push(' ');
            path.push_str(&def.name);
            let inner = level(
                c,
                u,
                ctx,
                &def.options,
                one.options,
                autocomplete,
                resolved,
                path,
                depth + 1,
            )
            .await?;
            return Ok(vec![InvokedOption {
                name: def.name.clone(),
                value: None,
                options: inner,
                focused: false,
            }]);
        }
        let mut out = vec![];
        let mut seen = HashSet::new();
        for g in given {
            let def = defs
                .iter()
                .find(|d| d.name == g.name)
                .ok_or_else(|| bad(format!("Unknown option {}.", g.name)))?;
            require(
                seen.insert(def.name.clone()) && g.options.is_empty(),
                ErrorCode::InvalidRequest,
            )?;
            let value = g
                .value
                .ok_or_else(|| bad(format!("{} needs a value.", def.name)))?;
            if g.focused {
                // The value being typed: only its type matters.
                require(autocomplete && def.autocomplete, ErrorCode::InvalidRequest)?;
                let typed = match (&value, def.kind) {
                    (OptionValue::String(s), OptionKind::String) => s.chars().count() <= 6000,
                    (OptionValue::String(s), OptionKind::Integer | OptionKind::Number) => {
                        s.len() <= 32
                    }
                    (OptionValue::Integer(_), OptionKind::Integer) => true,
                    (OptionValue::Number(n), OptionKind::Number) => n.0.is_finite(),
                    _ => false,
                };
                require(typed, ErrorCode::InvalidRequest)?;
                out.push(InvokedOption {
                    name: def.name.clone(),
                    value: Some(value),
                    options: vec![],
                    focused: true,
                });
                continue;
            }
            let value = check_given(c, u, ctx, def, value, resolved).await?;
            out.push(InvokedOption {
                name: def.name.clone(),
                value: Some(value),
                options: vec![],
                focused: false,
            });
        }
        if !autocomplete {
            if let Some(missing) = defs.iter().find(|d| d.required && !seen.contains(&d.name)) {
                return Err(bad(format!("{} is required.", missing.name)));
            }
        }
        Ok(out)
    })
}

async fn check_given(
    c: &mut SqliteConnection,
    u: Id,
    ctx: &Ctx,
    def: &CommandOption,
    value: OptionValue,
    resolved: &mut Resolved,
) -> Result<OptionValue> {
    let bad = |m: String| invalid("options", m);
    let name = &def.name;
    let in_range =
        |n: f64| def.min_value.is_none_or(|m| n >= m.0) && def.max_value.is_none_or(|m| n <= m.0);
    let chosen =
        |v: &OptionValue| def.choices.is_empty() || def.choices.iter().any(|ch| ch.value == *v);
    let v = match (def.kind, value) {
        (OptionKind::String, OptionValue::String(s)) => {
            let n = s.chars().count();
            if s.chars().any(|c| c.is_control() && c != '\n')
                || n < usize::from(def.min_length.unwrap_or(0))
                || n > usize::from(def.max_length.unwrap_or(6000))
                || n == 0
            {
                return Err(bad(format!("{name} has the wrong length.")));
            }
            OptionValue::String(s)
        }
        (OptionKind::Integer, OptionValue::Integer(i)) if in_range(i as f64) => {
            OptionValue::Integer(i)
        }
        (OptionKind::Number, OptionValue::Number(n)) if n.0.is_finite() && in_range(n.0) => {
            OptionValue::Number(n)
        }
        (OptionKind::Number, OptionValue::Integer(i)) if in_range(i as f64) => {
            OptionValue::Number(Number(i as f64))
        }
        (OptionKind::Boolean, OptionValue::Boolean(b)) => OptionValue::Boolean(b),
        (OptionKind::User, OptionValue::User(v))
        | (OptionKind::Mentionable, OptionValue::User(v)) => {
            role(c, v, ctx.guild)
                .await
                .map_err(|_| bad(format!("{name}: choose someone in this server.")))?;
            if !resolved.users.iter().any(|x| x.id == v) {
                resolved.users.push(user(c, v).await?);
            }
            if def.kind == OptionKind::Mentionable {
                OptionValue::Mentionable(v)
            } else {
                OptionValue::User(v)
            }
        }
        (OptionKind::Mentionable, OptionValue::Mentionable(v)) => {
            if role(c, v, ctx.guild).await.is_ok() {
                if !resolved.users.iter().any(|x| x.id == v) {
                    resolved.users.push(user(c, v).await?);
                }
            } else {
                let r = role_in(c, ctx.guild, v)
                    .await
                    .map_err(|_| bad(format!("{name}: choose a member or role.")))?;
                if !resolved.roles.iter().any(|x| x.id == v) {
                    resolved.roles.push(r);
                }
            }
            OptionValue::Mentionable(v)
        }
        (OptionKind::Role, OptionValue::Role(v))
        | (OptionKind::Mentionable, OptionValue::Role(v)) => {
            let r = role_in(c, ctx.guild, v)
                .await
                .map_err(|_| bad(format!("{name}: choose a role in this server.")))?;
            if !resolved.roles.iter().any(|x| x.id == v) {
                resolved.roles.push(r);
            }
            if def.kind == OptionKind::Mentionable {
                OptionValue::Mentionable(v)
            } else {
                OptionValue::Role(v)
            }
        }
        (OptionKind::Channel, OptionValue::Channel(v)) => {
            let ch = access(c, u, v, Permissions::VIEW_CHANNEL)
                .await
                .map_err(|_| bad(format!("{name}: choose a channel you can see.")))?;
            require(
                ch.summary.guild_id == Some(ctx.guild),
                ErrorCode::InvalidRequest,
            )?;
            if !resolved.channels.iter().any(|x| x.id == v) {
                resolved.channels.push(ch.summary);
            }
            OptionValue::Channel(v)
        }
        (OptionKind::Attachment, OptionValue::Attachment(a)) => {
            let r = sqlx::query("SELECT * FROM uploads WHERE id=? AND owner_id=? AND purpose='message' AND channel_id=? AND expires_at>?")
                .bind(id(a)?)
                .bind(id(u)?)
                .bind(id(ctx.channel)?)
                .bind(now() as i64)
                .fetch_optional(&mut *c)
                .await?
                .ok_or_else(|| bad(format!("{name}: upload the file to this channel first.")))?;
            resolved.attachments.push(attachment_row(&r));
            OptionValue::Attachment(a)
        }
        _ => return Err(bad(format!("{name} has the wrong type."))),
    };
    if !chosen(&v) {
        return Err(bad(format!("{name}: pick one of the choices.")));
    }
    Ok(v)
}

pub(crate) async fn role_in(c: &mut SqliteConnection, g: Id, r: Id) -> Result<Role> {
    crate::permissions::roles(c, g)
        .await?
        .into_iter()
        .find(|x| x.id == r)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))
}
