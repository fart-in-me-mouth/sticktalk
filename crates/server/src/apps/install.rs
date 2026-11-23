//! Adding apps to servers (the consent screen's install), removing them, the
//! managed role, and the Integrations page.
use super::*;
use crate::moderation::audit;

/// Permission names for the audit log and the consent screen.
fn permission_list(p: Permissions) -> String {
    let mut names: Vec<&str> = p.iter_names().map(|(n, _)| n).collect();
    names.sort_unstable();
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ").to_ascii_lowercase()
    }
}

pub(crate) async fn preview(c: &mut SqliteConnection, u: Id, app: Id) -> Result<InstallPreview> {
    let info = store::info(c, app).await?;
    require(
        (info.public || info.owner_id == u) && !info.disabled,
        ErrorCode::NotFound,
    )?;
    store::usable(c, app).await?;
    let owner = user(c, info.owner_id).await?;
    let mut guilds = vec![];
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE user_id=? ORDER BY guild_id")
            .bind(id(u)?)
            .fetch_all(&mut *c)
            .await?;
    for g in ids.into_iter().map(|g| Id(g as u64)) {
        let p = crate::permissions::evaluate(c, u, g, None).await?;
        if !p.contains(Permissions::MANAGE_GUILD) {
            continue;
        }
        let r = sqlx::query("SELECT name,icon_id,owner_id FROM guilds WHERE id=?")
            .bind(id(g)?)
            .fetch_one(&mut *c)
            .await?;
        guilds.push(InstallGuild {
            id: g,
            name: r.get("name"),
            icon_id: r.get::<Option<i64>, _>("icon_id").map(|v| Id(v as u64)),
            my_permissions: p,
            owner: r.get::<i64, _>("owner_id") == id(u)?,
            installed: role(c, info.bot.id, g).await.is_ok(),
        });
    }
    Ok(InstallPreview {
        app: info,
        owner,
        guilds,
    })
}

/// The managed role of an app in a server, if it has one.
pub(crate) async fn managed_role(c: &mut SqliteConnection, g: Id, role: Id) -> Result<Option<Id>> {
    let app: Option<i64> =
        sqlx::query_scalar("SELECT app_id FROM app_installs WHERE guild_id=? AND role_id=?")
            .bind(id(g)?)
            .bind(id(role)?)
            .fetch_optional(&mut *c)
            .await?;
    Ok(app.map(|a| Id(a as u64)))
}

async fn is_bot(c: &mut SqliteConnection, u: Id) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM apps WHERE bot_id=?)")
            .bind(id(u)?)
            .fetch_one(&mut *c)
            .await?,
    )
}

/// Before any command: managed roles change only with their app, and bots
/// are not DM partners or server owners.
pub(crate) async fn guard(c: &mut SqliteConnection, cmd: &Command) -> Result<()> {
    match cmd {
        Command::Launch(LaunchCommand::DeleteRole { guild_id, role_id }) => {
            if managed_role(c, *guild_id, *role_id).await?.is_some() {
                return Err(denied(
                    "managed_role",
                    "This role belongs to an app. Remove the app in Server Settings › Integrations to delete it.",
                ));
            }
        }
        Command::Launch(LaunchCommand::SetMemberRoles {
            guild_id,
            user_id,
            role_ids,
        }) => {
            let old = crate::permissions::role_ids(c, *guild_id, *user_id).await?;
            for r in old.iter().chain(role_ids.iter()) {
                if old.contains(r) == role_ids.contains(r) {
                    continue;
                }
                if managed_role(c, *guild_id, *r).await?.is_some() {
                    return Err(denied(
                        "managed_role",
                        "An app's role stays with its app: it can't be given to or taken from anyone.",
                    ));
                }
            }
        }
        Command::TransferGuildOwnership { user_id, .. } if is_bot(c, *user_id).await? => {
            return Err(denied("bot", "Apps can't own a server."));
        }
        Command::OpenDm { user_id } | Command::AddGroupRecipient { user_id, .. }
            if is_bot(c, *user_id).await? =>
        {
            return Err(denied("bot", "Apps can't receive direct messages."));
        }
        Command::CreateGroupDm { user_ids, .. } => {
            for v in user_ids {
                if is_bot(c, *v).await? {
                    return Err(denied("bot", "Apps can't receive direct messages."));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

async fn roles_changed(c: &mut SqliteConnection, out: &mut Deliveries, g: Id) -> Result<()> {
    let users = guild_users(c, g).await?;
    out.push((
        users.clone(),
        Event::RolesUpdated {
            guild_id: g,
            roles: crate::permissions::roles(c, g).await?,
        },
    ));
    for v in users {
        out.push((vec![v], Event::GuildUpsert(guild(c, v, g).await?)));
    }
    Ok(())
}

pub(crate) async fn install(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    r: InstallRequest,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let g = r.guild_id;
    let mine = crate::permissions::evaluate(c, u, g, None).await?;
    require(
        mine.contains(Permissions::MANAGE_GUILD),
        ErrorCode::Forbidden,
    )?;
    let info = store::info(c, r.app_id).await?;
    require(info.public || info.owner_id == u, ErrorCode::NotFound)?;
    store::usable(c, r.app_id).await?;
    core.rate(format!("app-install:{}", u.0), 10., 5.)?;
    let bot = info.bot.id;
    require(role(c, bot, g).await.is_err(), ErrorCode::AlreadyExists)?;
    let banned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM guild_bans WHERE guild_id=? AND user_id=?)",
    )
    .bind(id(g)?)
    .bind(id(bot)?)
    .fetch_one(&mut *c)
    .await?;
    require(!banned, ErrorCode::Banned)?;
    let wanted = Permissions::from_bits_truncate(r.permissions.bits());
    if !info.permissions.contains(wanted) || wanted.contains(Permissions::OWNER) {
        return Err(invalid(
            "permissions",
            "Grant only permissions the app asks for.",
        ));
    }
    let owner = mine.contains(Permissions::OWNER);
    if !owner && !mine.contains(wanted) {
        return Err(denied(
            "permissions",
            "You can't give an app permissions you don't have.",
        ));
    }
    if wanted.contains(Permissions::ADMINISTRATOR) && !(owner && r.confirm_administrator) {
        return Err(denied(
            "administrator",
            "Only the server's owner can give an app Administrator, and must confirm it.",
        ));
    }
    if r.message_content && !info.intents.contains(Intents::MESSAGE_CONTENT) {
        return Err(invalid(
            "message_content",
            "This app doesn't ask to read messages.",
        ));
    }
    guild_quota(c, bot).await?;
    require(
        guild_users(c, g).await?.len() < MAX_GUILD_MEMBERS,
        ErrorCode::LimitExceeded,
    )?;
    sqlx::query("INSERT INTO guild_members VALUES(?,?,'member',?)")
        .bind(id(g)?)
        .bind(id(bot)?)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    let mut role_id = None;
    if !wanted.is_empty() {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles WHERE guild_id=?")
            .bind(id(g)?)
            .fetch_one(&mut *c)
            .await?;
        require(count < MAX_ROLES_PER_GUILD as i64, ErrorCode::LimitExceeded)?;
        let rid = core.id().await?;
        sqlx::query("UPDATE roles SET position=position+1 WHERE guild_id=? AND position>=1")
            .bind(id(g)?)
            .execute(&mut *c)
            .await?;
        let role_name: String = info.name.chars().take(100).collect();
        sqlx::query("INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(?,?,?,1,?)")
            .bind(id(g)?)
            .bind(id(rid)?)
            .bind(role_name)
            .bind(wanted.bits() as i64)
            .execute(&mut *c)
            .await?;
        sqlx::query("INSERT INTO member_roles VALUES(?,?,?)")
            .bind(id(g)?)
            .bind(id(bot)?)
            .bind(id(rid)?)
            .execute(&mut *c)
            .await?;
        role_id = Some(rid);
    }
    sqlx::query("INSERT INTO app_installs(app_id,guild_id,role_id,permissions,message_content,installed_by,installed_at) VALUES(?,?,?,?,?,?,?) ON CONFLICT(app_id,guild_id) DO UPDATE SET role_id=excluded.role_id,permissions=excluded.permissions,message_content=excluded.message_content,installed_by=excluded.installed_by,installed_at=excluded.installed_at")
        .bind(id(r.app_id)?)
        .bind(id(g)?)
        .bind(opt(role_id)?)
        .bind(wanted.bits() as i64)
        .bind(r.message_content)
        .bind(id(u)?)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    if role_id.is_some() {
        roles_changed(c, out, g).await?;
    }
    crate::commands::joined(c, out, g, bot).await?;
    audit::write(
        c,
        u,
        audit::Entry::new(g, AuditAction::AppInstall, AuditTarget::App, Some(r.app_id))
            .named(info.name.clone())
            .change("permissions", None, Some(permission_list(wanted)))
            .change(
                "message_content",
                None,
                Some(
                    if r.message_content {
                        "granted"
                    } else {
                        "not granted"
                    }
                    .into(),
                ),
            ),
    )
    .await?;
    integrations_changed(c, out, g).await?;
    commands_changed(c, out, g).await?;
    // The bot's open connections pick up the server's content grant.
    let streams: Vec<String> = core
        .streams
        .values()
        .filter(|s| s.user == bot)
        .map(|s| s.id.clone())
        .collect();
    for s in streams {
        core.apps.bots.remove(&s);
    }
    Ok(AppResult::Installation(Box::new(
        installation(c, g, r.app_id).await?,
    )))
}

pub(crate) async fn uninstall(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    g: Id,
    app: Id,
    out: &mut Deliveries,
) -> Result<AppResult> {
    require(
        crate::permissions::evaluate(c, u, g, None)
            .await?
            .contains(Permissions::MANAGE_GUILD),
        ErrorCode::Forbidden,
    )?;
    remove(c, core, &[], u, g, app, false, out).await?;
    Ok(AppResult::Ack)
}

/// Removes the app's member, managed role, commands and overrides from `g`,
/// and nothing else (its messages and the server's webhooks stay).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn remove(
    c: &mut SqliteConnection,
    core: &mut Core,
    _key: &[u8],
    u: Id,
    g: Id,
    app: Id,
    deleted: bool,
    out: &mut Deliveries,
) -> Result<()> {
    let r = sqlx::query("SELECT i.role_id,a.bot_id,a.name FROM app_installs i JOIN apps a ON a.id=i.app_id WHERE i.app_id=? AND i.guild_id=?")
        .bind(id(app)?)
        .bind(id(g)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let bot = Id(r.get::<i64, _>("bot_id") as u64);
    let app_name: String = r.get("name");
    let role_id = r.get::<Option<i64>, _>("role_id").map(|v| Id(v as u64));
    sqlx::query("DELETE FROM app_installs WHERE app_id=? AND guild_id=?")
        .bind(id(app)?)
        .bind(id(g)?)
        .execute(&mut *c)
        .await?;
    sqlx::query("DELETE FROM app_commands WHERE app_id=? AND guild_id=?")
        .bind(id(app)?)
        .bind(id(g)?)
        .execute(&mut *c)
        .await?;
    sqlx::query("DELETE FROM app_command_permissions WHERE app_id=? AND guild_id=?")
        .bind(id(app)?)
        .bind(id(g)?)
        .execute(&mut *c)
        .await?;
    if let Some(rid) = role_id {
        let position: Option<i64> =
            sqlx::query_scalar("SELECT position FROM roles WHERE guild_id=? AND id=?")
                .bind(id(g)?)
                .bind(id(rid)?)
                .fetch_optional(&mut *c)
                .await?;
        if let Some(position) = position {
            sqlx::query("DELETE FROM permission_overwrites WHERE kind='role' AND target_id=? AND channel_id IN(SELECT id FROM channels WHERE guild_id=?)")
                .bind(id(rid)?)
                .bind(id(g)?)
                .execute(&mut *c)
                .await?;
            sqlx::query("DELETE FROM roles WHERE guild_id=? AND id=?")
                .bind(id(g)?)
                .bind(id(rid)?)
                .execute(&mut *c)
                .await?;
            sqlx::query("UPDATE roles SET position=position-1 WHERE guild_id=? AND position>?")
                .bind(id(g)?)
                .bind(position)
                .execute(&mut *c)
                .await?;
        }
    }
    if role(c, bot, g).await.is_ok() {
        crate::commands::remove_member(c, out, g, bot, RemovalReason::Kicked).await?;
    }
    if role_id.is_some() {
        roles_changed(c, out, g).await?;
    }
    audit::write(
        c,
        u,
        audit::Entry::new(g, AuditAction::AppRemove, AuditTarget::App, Some(app))
            .named(app_name)
            .reason(deleted.then(|| "The app was deleted by its owner.".to_owned())),
    )
    .await?;
    core.apps.interactions.forget_guild(app, g);
    integrations_changed(c, out, g).await?;
    commands_changed(c, out, g).await
}

pub(crate) async fn installation(c: &mut SqliteConnection, g: Id, app: Id) -> Result<Installation> {
    let r = sqlx::query("SELECT * FROM app_installs WHERE app_id=? AND guild_id=?")
        .bind(id(app)?)
        .bind(id(g)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let info = store::info(c, app).await?;
    let commands = registry::commands(c, app, Some(g)).await?;
    let overrides = registry::overrides(c, g, app).await?;
    Ok(Installation {
        app_id: app,
        guild_id: g,
        name: info.name,
        description: info.description,
        icon_id: info.icon_id,
        bot: info.bot,
        role_id: r.get::<Option<i64>, _>("role_id").map(|v| Id(v as u64)),
        permissions: Permissions::from_bits_retain(r.get::<i64, _>("permissions") as u64),
        message_content: r.get("message_content"),
        installed_by: Id(r.get::<i64, _>("installed_by") as u64),
        installed_at: r.get::<i64, _>("installed_at") as u64,
        commands,
        overrides,
        disabled: info.disabled,
    })
}

/// Server Settings › Integrations: apps for Manage Server, and the webhooks
/// of the channels the viewer may manage webhooks in.
pub(crate) async fn integrations(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
) -> Result<GuildIntegrations> {
    let p = crate::permissions::evaluate(c, u, g, None).await?;
    let mut result = GuildIntegrations::default();
    if p.contains(Permissions::MANAGE_GUILD) {
        let apps: Vec<i64> = sqlx::query_scalar("SELECT i.app_id FROM app_installs i JOIN apps a ON a.id=i.app_id JOIN guild_members m ON m.guild_id=i.guild_id AND m.user_id=a.bot_id WHERE i.guild_id=? ORDER BY i.installed_at")
            .bind(id(g)?)
            .fetch_all(&mut *c)
            .await?;
        for a in apps {
            result.apps.push(installation(c, g, Id(a as u64)).await?);
        }
    }
    result.webhooks = webhooks::manageable(c, u, g).await?;
    require(
        p.contains(Permissions::MANAGE_GUILD)
            || !result.webhooks.is_empty()
            || p.contains(Permissions::MANAGE_WEBHOOKS),
        ErrorCode::Forbidden,
    )?;
    Ok(result)
}
