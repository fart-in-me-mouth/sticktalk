//! Applications, their bot accounts and tokens, and the instance policy.
use super::*;
use subtle::ConstantTimeEq;

/// The instance administrator's policy.
pub(crate) async fn creation(c: &mut SqliteConnection) -> Result<AppCreation> {
    let s: Option<String> =
        sqlx::query_scalar("SELECT creation FROM app_settings WHERE singleton=1")
            .fetch_optional(&mut *c)
            .await?;
    Ok(s.map(en).transpose()?.unwrap_or_default())
}
pub(crate) async fn may_create(c: &mut SqliteConnection, u: Id) -> Result<bool> {
    Ok(match creation(c).await? {
        AppCreation::Everyone => true,
        AppCreation::Disabled => false,
        AppCreation::Allowlist => {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM app_creators WHERE user_id=?)",
            )
            .bind(id(u)?)
            .fetch_one(&mut *c)
            .await?
        }
    })
}

/// The app may act now: not disabled, and the feature is on.
pub(crate) async fn usable(c: &mut SqliteConnection, app: Id) -> Result<()> {
    let disabled: Option<bool> = sqlx::query_scalar("SELECT disabled FROM apps WHERE id=?")
        .bind(id(app)?)
        .fetch_optional(&mut *c)
        .await?;
    match disabled {
        None => Err(Error::new(ErrorCode::Unauthenticated)),
        Some(true) => Err(denied(
            "app_disabled",
            "This app was turned off by the server's administrator.",
        )),
        Some(false) if creation(c).await? == AppCreation::Disabled => Err(denied(
            "apps_disabled",
            "Apps are turned off on this server.",
        )),
        Some(false) => Ok(()),
    }
}

/// A bot token's random part (after `stkb_`): (session key, bot, app).
pub(crate) async fn bot_auth(c: &mut SqliteConnection, raw: &str) -> Result<(Vec<u8>, Id, Id)> {
    use base64::Engine;
    require(
        raw.len() == 43
            && base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(raw)
                .is_ok_and(|v| v.len() == 32),
        ErrorCode::Unauthenticated,
    )?;
    let h = crate::state::hash(raw.as_bytes());
    let r = sqlx::query("SELECT s.token_hash,s.user_id,a.owner_id FROM sessions s JOIN app_sessions a ON a.session_hash=s.token_hash AND a.kind='bot' WHERE s.token_hash=? AND s.expires_at>? AND NOT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=s.user_id)")
        .bind(&h)
        .bind(now() as i64)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::Unauthenticated))?;
    let stored: Vec<u8> = r.get("token_hash");
    require(bool::from(stored.ct_eq(&h)), ErrorCode::Unauthenticated)?;
    let app = Id(r.get::<i64, _>("owner_id") as u64);
    usable(c, app).await?;
    sqlx::query("UPDATE sessions SET last_used_at=? WHERE token_hash=? AND last_used_at<?")
        .bind(now() as i64)
        .bind(&h)
        .bind(now().saturating_sub(60_000) as i64)
        .execute(&mut *c)
        .await?;
    Ok((h, Id(r.get::<i64, _>("user_id") as u64), app))
}

pub(crate) async fn app_of_bot(c: &mut SqliteConnection, bot: Id) -> Result<Id> {
    let app: Option<i64> = sqlx::query_scalar("SELECT id FROM apps WHERE bot_id=?")
        .bind(id(bot)?)
        .fetch_optional(&mut *c)
        .await?;
    app.map(|a| Id(a as u64))
        .ok_or_else(|| Error::new(ErrorCode::Forbidden))
}

pub(crate) async fn info(c: &mut SqliteConnection, app: Id) -> Result<AppInfo> {
    let r = sqlx::query("SELECT a.*,(SELECT COUNT(*) FROM app_installs i JOIN guild_members m ON m.guild_id=i.guild_id AND m.user_id=a.bot_id WHERE i.app_id=a.id) AS n FROM apps a WHERE a.id=?")
        .bind(id(app)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let bot = user(c, Id(r.get::<i64, _>("bot_id") as u64)).await?;
    Ok(AppInfo {
        id: app,
        name: r.get("name"),
        description: r.get("description"),
        icon_id: bot.avatar_id,
        owner_id: Id(r.get::<i64, _>("owner_id") as u64),
        bot,
        public: r.get("public"),
        intents: Intents::from_bits_truncate(r.get::<i64, _>("intents") as u32),
        permissions: Permissions::from_bits_retain(r.get::<i64, _>("permissions") as u64),
        disabled: r.get("disabled"),
        install_count: r.get::<i64, _>("n") as u32,
        created_at: r.get::<i64, _>("created_at") as u64,
    })
}

pub(crate) async fn developer(c: &mut SqliteConnection, u: Id) -> Result<DeveloperState> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM apps WHERE owner_id=? ORDER BY id")
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?;
    let mut apps = vec![];
    for a in ids {
        apps.push(info(c, Id(a as u64)).await?);
    }
    Ok(DeveloperState {
        apps,
        creation: creation(c).await?,
        can_create: may_create(c, u).await?,
    })
}

fn clean(d: &mut AppDraft) -> Result<()> {
    d.name = name(&d.name, 4 * MAX_APP_NAME_CHARS)
        .map_err(|_| invalid("name", "Give the app a name (up to 32 characters)."))?;
    require(
        d.name.chars().count() <= MAX_APP_NAME_CHARS,
        ErrorCode::InvalidRequest,
    )
    .map_err(|_| invalid("name", "Give the app a name (up to 32 characters)."))?;
    d.description = d.description.trim().to_owned();
    if d.description.chars().count() > MAX_APP_DESCRIPTION_CHARS
        || d.description.chars().any(|c| c.is_control() && c != '\n')
    {
        return Err(invalid(
            "description",
            "The description can be up to 400 characters.",
        ));
    }
    d.permissions = Permissions::from_bits_truncate(d.permissions.bits()) - Permissions::OWNER;
    d.intents = Intents::from_bits_truncate(d.intents.bits());
    Ok(())
}

/// Claims one of `u`'s uploaded avatars for an app or webhook account.
pub(crate) async fn claim_avatar(c: &mut SqliteConnection, u: Id, a: Id) -> Result<()> {
    let n = sqlx::query("UPDATE uploads SET expires_at=NULL WHERE id=? AND owner_id=? AND purpose='avatar' AND expires_at>?")
        .bind(id(a)?)
        .bind(id(u)?)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?
        .rows_affected();
    require(n == 1, ErrorCode::InvalidAttachment)
}

/// A username no person can register or sign in with (it has a hyphen).
async fn account_name(c: &mut SqliteConnection, base: &str) -> Result<String> {
    let mut stem: String = base
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    stem = stem.trim_matches('_').chars().take(20).collect();
    if stem.is_empty() {
        stem = "app".into();
    }
    for _ in 0..8 {
        let tag: String = crate::state::random(2)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let candidate = format!("{stem}-{tag}");
        let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE username=?)")
            .bind(&candidate)
            .fetch_one(&mut *c)
            .await?;
        if !taken {
            return Ok(candidate);
        }
    }
    Err(Error::new(ErrorCode::Conflict))
}

/// Inserts a bot or webhook account: no password, the APP style.
pub(crate) async fn create_account(
    c: &mut SqliteConnection,
    core: &mut Core,
    base: &str,
    display: &str,
    avatar: Option<Id>,
) -> Result<Id> {
    let user = core.id().await?;
    let username = account_name(c, base).await?;
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,avatar_id,desired_presence,created_at,style) VALUES(?,?,'',?,?,'online',?,?)")
        .bind(id(user)?)
        .bind(username)
        .bind(display)
        .bind(opt(avatar)?)
        .bind(now() as i64)
        .bind(r#"{"bot":true}"#)
        .execute(&mut *c)
        .await?;
    Ok(user)
}

/// A new credential for `user` (`kind` is `bot` or `webhook`): returns the
/// secret (never stored) and its session key.
pub(crate) async fn credential_for(
    c: &mut SqliteConnection,
    user: Id,
    kind: &str,
    owner: Id,
) -> Result<(String, Vec<u8>)> {
    let secret = crate::state::token();
    let key = crate::state::hash(secret.as_bytes());
    sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
        .bind(&key)
        .bind(id(user)?)
        .bind(now() as i64)
        .bind(FOREVER)
        .bind(0_i64)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT INTO app_sessions(session_hash,kind,owner_id) VALUES(?,?,?)")
        .bind(&key)
        .bind(kind)
        .bind(id(owner)?)
        .execute(&mut *c)
        .await?;
    Ok((secret, key))
}

pub(crate) async fn create(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    mut d: AppDraft,
    out: &mut Deliveries,
) -> Result<AppResult> {
    if !may_create(c, u).await? {
        return Err(denied(
            "apps_creation",
            "The administrator of this StickTalk server hasn't let your account create apps.",
        ));
    }
    clean(&mut d)?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM apps WHERE owner_id=?")
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
    require(n < MAX_APPS_PER_USER as i64, ErrorCode::LimitExceeded)?;
    core.rate(format!("app-create:{}", u.0), 10., 3.)?;
    let icon = match d.icon {
        Patch::Set(a) => {
            claim_avatar(c, u, a).await?;
            Some(a)
        }
        _ => None,
    };
    let bot = create_account(c, core, &d.name, &d.name, icon).await?;
    let app = core.id().await?;
    sqlx::query("INSERT INTO apps(id,owner_id,bot_id,name,description,public,intents,permissions,created_at) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(id(app)?)
        .bind(id(u)?)
        .bind(id(bot)?)
        .bind(&d.name)
        .bind(&d.description)
        .bind(d.public)
        .bind(i64::from(d.intents.bits()))
        .bind(d.permissions.bits() as i64)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    let (secret, _) = credential_for(c, bot, "bot", app).await?;
    out.push((vec![u], Event::Apps(AppEvent::AppsUpdated)));
    Ok(AppResult::Token {
        app: Box::new(info(c, app).await?),
        token: format!("{BOT_TOKEN_PREFIX}{secret}"),
    })
}

async fn owned(c: &mut SqliteConnection, u: Id, app: Id) -> Result<Id> {
    let bot: Option<i64> = sqlx::query_scalar("SELECT bot_id FROM apps WHERE id=? AND owner_id=?")
        .bind(id(app)?)
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?;
    bot.map(|b| Id(b as u64))
        .ok_or_else(|| Error::new(ErrorCode::NotFound))
}

/// Everyone who shares a server with the bot (for its profile changes).
async fn bot_viewers(c: &mut SqliteConnection, bot: Id) -> Result<Vec<Id>> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT DISTINCT b.user_id FROM guild_members a JOIN guild_members b ON a.guild_id=b.guild_id WHERE a.user_id=? LIMIT 20000")
        .bind(id(bot)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|v| Id(v as u64))
        .collect())
}

pub(crate) async fn update(
    c: &mut SqliteConnection,
    u: Id,
    app: Id,
    mut d: AppDraft,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let bot = owned(c, u, app).await?;
    clean(&mut d)?;
    match d.icon {
        Patch::Keep => {}
        Patch::Clear => {
            sqlx::query("UPDATE users SET avatar_id=NULL WHERE id=?")
                .bind(id(bot)?)
                .execute(&mut *c)
                .await?;
        }
        Patch::Set(a) => {
            claim_avatar(c, u, a).await?;
            sqlx::query("UPDATE users SET avatar_id=? WHERE id=?")
                .bind(id(a)?)
                .bind(id(bot)?)
                .execute(&mut *c)
                .await?;
        }
    }
    sqlx::query("UPDATE apps SET name=?,description=?,public=?,intents=?,permissions=? WHERE id=?")
        .bind(&d.name)
        .bind(&d.description)
        .bind(d.public)
        .bind(i64::from(d.intents.bits()))
        .bind(d.permissions.bits() as i64)
        .bind(id(app)?)
        .execute(&mut *c)
        .await?;
    sqlx::query("UPDATE users SET display_name=? WHERE id=?")
        .bind(&d.name)
        .bind(id(bot)?)
        .execute(&mut *c)
        .await?;
    out.push((vec![u], Event::Apps(AppEvent::AppsUpdated)));
    let viewers = bot_viewers(c, bot).await?;
    if !viewers.is_empty() {
        out.push((viewers, Event::UserUpdated(user(c, bot).await?)));
    }
    Ok(AppResult::App(Box::new(info(c, app).await?)))
}

pub(crate) async fn delete(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    app: Id,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let bot = owned(c, u, app).await?;
    let guilds: Vec<i64> = sqlx::query_scalar("SELECT guild_id FROM app_installs WHERE app_id=?")
        .bind(id(app)?)
        .fetch_all(&mut *c)
        .await?;
    for g in guilds {
        install::remove(c, core, key, u, Id(g as u64), app, true, out).await?;
    }
    sqlx::query("DELETE FROM sessions WHERE user_id=?")
        .bind(id(bot)?)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO disabled_users VALUES(?)")
        .bind(id(bot)?)
        .execute(&mut *c)
        .await?;
    sqlx::query("DELETE FROM apps WHERE id=?")
        .bind(id(app)?)
        .execute(&mut *c)
        .await?;
    super::disconnect(core, bot);
    core.apps.interactions.forget_app(app);
    out.push((vec![u], Event::Apps(AppEvent::AppsUpdated)));
    Ok(AppResult::Ack)
}

pub(crate) async fn reset_token(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    app: Id,
) -> Result<AppResult> {
    let bot = owned(c, u, app).await?;
    core.rate(format!("app-token:{}", app.0), 6., 3.)?;
    sqlx::query("DELETE FROM sessions WHERE user_id=?")
        .bind(id(bot)?)
        .execute(&mut *c)
        .await?;
    let (secret, _) = credential_for(c, bot, "bot", app).await?;
    super::disconnect(core, bot);
    Ok(AppResult::Token {
        app: Box::new(info(c, app).await?),
        token: format!("{BOT_TOKEN_PREFIX}{secret}"),
    })
}
