//! Incoming webhooks: managing them (Manage Webhooks), and posting through
//! their secret URL. A webhook posts as its own account, which holds fixed
//! rights in its channel only (`super::principal`).
//!
//! `POST /api/v1/webhooks/{id}/{token}` takes Discord's JSON shape (or
//! multipart with `payload_json` and files); `/slack` and `/github` accept a
//! documented subset of those services' payloads and turn them into embeds.
use super::*;
use crate::moderation::audit;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use subtle::ConstantTimeEq;

const KINDS: &[ChannelKind] = &[
    ChannelKind::Text,
    ChannelKind::Announcement,
    ChannelKind::Forum,
    ChannelKind::Thread,
];

fn webhook_row(r: &sqlx::sqlite::SqliteRow, avatar: Option<Id>) -> Webhook {
    Webhook {
        id: Id(r.get::<i64, _>("id") as u64),
        guild_id: Id(r.get::<i64, _>("guild_id") as u64),
        channel_id: Id(r.get::<i64, _>("channel_id") as u64),
        name: r.get("name"),
        avatar_id: avatar,
        user_id: Id(r.get::<i64, _>("user_id") as u64),
        creator_id: Id(r.get::<i64, _>("creator_id") as u64),
        created_at: r.get::<i64, _>("created_at") as u64,
        last_used_at: r.get::<Option<i64>, _>("last_used_at").map(|v| v as u64),
    }
}
pub(crate) async fn load(c: &mut SqliteConnection, w: Id) -> Result<Webhook> {
    let r = sqlx::query(
        "SELECT w.*,u.avatar_id FROM webhooks w JOIN users u ON u.id=w.user_id WHERE w.id=?",
    )
    .bind(id(w)?)
    .fetch_optional(&mut *c)
    .await?
    .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let avatar = r.get::<Option<i64>, _>("avatar_id").map(|v| Id(v as u64));
    Ok(webhook_row(&r, avatar))
}

/// Manage Webhooks in the webhook's channel (NotFound when it can't be seen).
async fn managed(c: &mut SqliteConnection, u: Id, w: Id) -> Result<Webhook> {
    let hook = load(c, w).await?;
    access(c, u, hook.channel_id, Permissions::MANAGE_WEBHOOKS).await?;
    Ok(hook)
}

pub(crate) async fn manageable(c: &mut SqliteConnection, u: Id, g: Id) -> Result<Vec<Webhook>> {
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM webhooks WHERE guild_id=? ORDER BY channel_id,id")
            .bind(id(g)?)
            .fetch_all(&mut *c)
            .await?;
    let mut out = vec![];
    for w in ids {
        if let Ok(hook) = managed(c, u, Id(w as u64)).await {
            out.push(hook);
        }
    }
    Ok(out)
}

fn webhook_name(s: &str) -> Result<String> {
    let s = s.trim();
    if !(1..=MAX_WEBHOOK_NAME_CHARS).contains(&s.chars().count()) || s.chars().any(char::is_control)
    {
        return Err(invalid("name", "Webhook names are 1-80 characters."));
    }
    Ok(s.to_owned())
}

fn url(c: &Core, w: Id, secret: &str) -> String {
    format!(
        "{}/api/v1/webhooks/{}/{}",
        c.apps.public.trim_end_matches('/'),
        w.0,
        secret
    )
}

async fn changed(c: &mut SqliteConnection, out: &mut Deliveries, g: Id) -> Result<()> {
    integrations_changed(c, out, g).await
}

pub(crate) async fn create(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    ch: Id,
    name: String,
    avatar: Option<Id>,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let channel = access(c, u, ch, Permissions::MANAGE_WEBHOOKS).await?;
    let g = channel
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    if !KINDS.contains(&channel.summary.kind) {
        return Err(invalid(
            "channel",
            "Webhooks post in text, announcement and forum channels and threads.",
        ));
    }
    let name = webhook_name(&name)?;
    core.rate(format!("webhook-create:{}", u.0), 10., 5.)?;
    // A webhook's name is the author name of everything it posts.
    crate::moderation::coverage::name_check(core, c, u, g, "webhook_name", &name).await?;
    let in_channel: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webhooks WHERE channel_id=?")
        .bind(id(ch)?)
        .fetch_one(&mut *c)
        .await?;
    let in_guild: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webhooks WHERE guild_id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    require(
        in_channel < MAX_WEBHOOKS_PER_CHANNEL as i64 && in_guild < MAX_WEBHOOKS_PER_GUILD as i64,
        ErrorCode::LimitExceeded,
    )?;
    if let Some(a) = avatar {
        store::claim_avatar(c, u, a).await?;
    }
    let account = store::create_account(c, core, "webhook", &name, avatar).await?;
    let w = core.id().await?;
    let (_, session) = store::credential_for(c, account, "webhook", w).await?;
    let secret = crate::state::token();
    sqlx::query("INSERT INTO webhooks(id,guild_id,channel_id,user_id,name,token_hash,session_hash,creator_id,created_at) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(id(w)?)
        .bind(id(g)?)
        .bind(id(ch)?)
        .bind(id(account)?)
        .bind(&name)
        .bind(crate::state::hash(secret.as_bytes()))
        .bind(&session)
        .bind(id(u)?)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    audit::write(
        c,
        u,
        audit::Entry::new(g, AuditAction::WebhookCreate, AuditTarget::Webhook, Some(w))
            .named(name.clone())
            .change(
                "channel",
                None,
                Some(format!(
                    "#{}",
                    channel.summary.name.clone().unwrap_or_default()
                )),
            ),
    )
    .await?;
    changed(c, out, g).await?;
    let webhook = load(c, w).await?;
    Ok(AppResult::WebhookSecret(Box::new(WebhookSecret {
        url: url(core, w, &secret),
        webhook,
    })))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn update(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    w: Id,
    name: Option<String>,
    avatar: Patch<Id>,
    move_to: Option<Id>,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let old = managed(c, u, w).await?;
    let mut entry = audit::Entry::new(
        old.guild_id,
        AuditAction::WebhookUpdate,
        AuditTarget::Webhook,
        Some(w),
    );
    if let Some(n) = name {
        let n = webhook_name(&n)?;
        crate::moderation::coverage::name_check(core, c, u, old.guild_id, "webhook_name", &n)
            .await?;
        sqlx::query("UPDATE webhooks SET name=? WHERE id=?")
            .bind(&n)
            .bind(id(w)?)
            .execute(&mut *c)
            .await?;
        sqlx::query("UPDATE users SET display_name=? WHERE id=?")
            .bind(&n)
            .bind(id(old.user_id)?)
            .execute(&mut *c)
            .await?;
        entry = entry.change("name", Some(old.name.clone()), Some(n));
    }
    match avatar {
        Patch::Keep => {}
        Patch::Clear => {
            sqlx::query("UPDATE users SET avatar_id=NULL WHERE id=?")
                .bind(id(old.user_id)?)
                .execute(&mut *c)
                .await?;
            entry = entry.change("avatar", Some("set".into()), None);
        }
        Patch::Set(a) => {
            store::claim_avatar(c, u, a).await?;
            sqlx::query("UPDATE users SET avatar_id=? WHERE id=?")
                .bind(id(a)?)
                .bind(id(old.user_id)?)
                .execute(&mut *c)
                .await?;
            entry = entry.change("avatar", None, Some("changed".into()));
        }
    }
    if let Some(ch) = move_to.filter(|ch| *ch != old.channel_id) {
        let target = access(c, u, ch, Permissions::MANAGE_WEBHOOKS).await?;
        require(
            target.summary.guild_id == Some(old.guild_id) && KINDS.contains(&target.summary.kind),
            ErrorCode::InvalidRequest,
        )?;
        sqlx::query("UPDATE webhooks SET channel_id=? WHERE id=?")
            .bind(id(ch)?)
            .bind(id(w)?)
            .execute(&mut *c)
            .await?;
        entry = entry.change(
            "channel",
            Some(old.channel_id.0.to_string()),
            Some(ch.0.to_string()),
        );
    }
    audit::write(c, u, entry.named(old.name.clone())).await?;
    changed(c, out, old.guild_id).await?;
    Ok(AppResult::Webhook(Box::new(load(c, w).await?)))
}

pub(crate) async fn rotate(
    c: &mut SqliteConnection,
    core: &Core,
    u: Id,
    w: Id,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let hook = managed(c, u, w).await?;
    let secret = crate::state::token();
    sqlx::query("UPDATE webhooks SET token_hash=? WHERE id=?")
        .bind(crate::state::hash(secret.as_bytes()))
        .bind(id(w)?)
        .execute(&mut *c)
        .await?;
    audit::write(
        c,
        u,
        audit::Entry::new(
            hook.guild_id,
            AuditAction::WebhookUpdate,
            AuditTarget::Webhook,
            Some(w),
        )
        .named(hook.name.clone())
        .change("secret", None, Some("rotated".into())),
    )
    .await?;
    changed(c, out, hook.guild_id).await?;
    Ok(AppResult::WebhookSecret(Box::new(WebhookSecret {
        url: url(core, w, &secret),
        webhook: hook,
    })))
}

pub(crate) async fn delete(
    c: &mut SqliteConnection,
    u: Id,
    w: Id,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let hook = managed(c, u, w).await?;
    sqlx::query("DELETE FROM webhooks WHERE id=?")
        .bind(id(w)?)
        .execute(&mut *c)
        .await?;
    audit::write(
        c,
        u,
        audit::Entry::new(
            hook.guild_id,
            AuditAction::WebhookDelete,
            AuditTarget::Webhook,
            Some(w),
        )
        .named(hook.name.clone()),
    )
    .await?;
    changed(c, out, hook.guild_id).await?;
    Ok(AppResult::Ack)
}

// ------------------------------------------------------------- posting

pub(crate) fn screen_text(p: &WebhookPost) -> String {
    let mut t = messages::screen_text(&p.content, &p.embeds, &[]);
    for s in [&p.username, &p.thread_name].into_iter().flatten() {
        t.push('\n');
        t.push_str(s);
    }
    t
}

struct Hook {
    id: Id,
    channel: Id,
}
async fn of_account(c: &mut SqliteConnection, u: Id) -> Result<Hook> {
    let r = sqlx::query("SELECT id,channel_id FROM webhooks WHERE user_id=?")
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok(Hook {
        id: Id(r.get::<i64, _>("id") as u64),
        channel: Id(r.get::<i64, _>("channel_id") as u64),
    })
}

/// The webhook's channel, or a thread (or forum post) directly in it.
async fn target(c: &mut SqliteConnection, hook: &Hook, ch: Id) -> Result<Channel> {
    let channel = channel(c, ch).await?;
    require(
        ch == hook.channel
            || (channel.summary.kind == ChannelKind::Thread
                && channel.summary.parent_id == Some(hook.channel)),
        ErrorCode::NotFound,
    )?;
    Ok(channel)
}

fn display_name(s: &Option<String>) -> Result<Option<String>> {
    match s.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(s) => webhook_name(s)
            .map(Some)
            .map_err(|_| invalid("username", "username is 1-80 characters.")),
    }
}

pub(crate) async fn execute(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    p: WebhookPost,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let hook = of_account(c, u).await?;
    core.rate(format!("webhook:{}", hook.id.0), 30., 5.)?;
    let channel = target(c, &hook, p.channel_id).await?;
    core.rate(
        format!("webhook-channel:{}", channel.summary.id.0),
        30.,
        10.,
    )?;
    let username = display_name(&p.username)?;
    let (embeds, mut images) = messages::check_embeds(&p.embeds)?;
    if let Some(a) = p.avatar_url.as_deref().filter(|a| !a.is_empty()) {
        require(messages::web_url(a), ErrorCode::InvalidRequest)
            .map_err(|_| invalid("avatar_url", "avatar_url is an http(s) URL."))?;
        images.insert(0, (messages::Slot::Avatar, a.to_owned()));
    }
    let (content, mention_ids) =
        messages::mentions(c, u, &channel, &p.content, Some(&p.allowed_mentions)).await?;
    let app = AppMessage {
        webhook_id: Some(hook.id),
        username,
        embeds,
        ..Default::default()
    };
    sqlx::query("UPDATE webhooks SET last_used_at=? WHERE id=?")
        .bind(now() as i64)
        .bind(id(hook.id)?)
        .execute(&mut *c)
        .await?;
    if channel.summary.kind == ChannelKind::Forum {
        let title = p
            .thread_name
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                invalid(
                    "thread_name",
                    "Posting in a forum starts a post: give it a thread_name.",
                )
            })?;
        let result = rich_scope(Box::pin(crate::threads::mutate(
            c,
            core,
            key,
            u,
            ThreadCommand::CreatePost(NewPost {
                forum_id: channel.summary.id,
                title: title.to_owned(),
                content,
                attachment_ids: p.attachment_ids,
                attachments: vec![],
                mention_ids,
                tag_ids: vec![],
                auto_archive_minutes: None,
            }),
            out,
        )))
        .await?;
        let CommandResult::Threads(ThreadResult::Post { message, thread }) = result else {
            return Err(Error::new(ErrorCode::Internal));
        };
        let mut m = *message;
        messages::attach(c, core, &mut m, app, images, thread.id, out).await?;
        return Ok(AppResult::Message(Box::new(m)));
    }
    if p.thread_name.is_some() {
        return Err(invalid("thread_name", "thread_name is for forum webhooks."));
    }
    let m = messages::post(
        c,
        core,
        key,
        u,
        MessageDraft {
            channel_id: channel.summary.id,
            content,
            reply_to: None,
            attachment_ids: p.attachment_ids,
            mention_ids,
        },
        app,
        images,
        out,
    )
    .await?;
    Ok(AppResult::Message(Box::new(m)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn edit_message(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    ch: Id,
    m: Id,
    p: WebhookPost,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let hook = of_account(c, u).await?;
    core.rate(format!("webhook:{}", hook.id.0), 30., 5.)?;
    let channel = target(c, &hook, ch).await?;
    let old = message(c, u, ch, m, false).await?;
    require(old.author_id == u, ErrorCode::NotFound)?;
    let (embeds, images) = messages::check_embeds(&p.embeds)?;
    let (content, mention_ids) =
        messages::mentions(c, u, &channel, &p.content, Some(&p.allowed_mentions)).await?;
    let before = old.extra.app.map(|a| *a).unwrap_or_default();
    let app = AppMessage {
        webhook_id: Some(hook.id),
        embeds,
        ..before
    };
    let m = messages::replace(
        c,
        core,
        key,
        u,
        ch,
        m,
        content,
        mention_ids,
        app,
        images,
        out,
    )
    .await?;
    Ok(AppResult::Message(Box::new(m)))
}

pub(crate) async fn delete_message(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    ch: Id,
    m: Id,
    out: &mut Deliveries,
) -> Result<AppResult> {
    let hook = of_account(c, u).await?;
    target(c, &hook, ch).await?;
    let old = message(c, u, ch, m, false).await?;
    require(old.author_id == u, ErrorCode::NotFound)?;
    Box::pin(crate::commands::mutate(
        c,
        core,
        key,
        u,
        Command::DeleteMessage {
            channel_id: ch,
            message_id: m,
        },
        out,
    ))
    .await?;
    Ok(AppResult::Ack)
}

// ---------------------------------------------------------------- REST

pub(super) fn routes() -> Router<App> {
    Router::new()
        .route(
            "/api/v1/webhooks/{id}/{token}",
            get(info).post(execute_json),
        )
        .route("/api/v1/webhooks/{id}/{token}/slack", post(execute_slack))
        .route("/api/v1/webhooks/{id}/{token}/github", post(execute_github))
        .route(
            "/api/v1/webhooks/{id}/{token}/messages/{message}",
            axum::routing::patch(edit_json).delete(delete_json),
        )
        .route("/api/v1/channels/{id}/webhooks", get(list))
}

async fn list(
    State(app): State<App>,
    Path(ch): Path<u64>,
    h: HeaderMap,
) -> Result<crate::http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, crate::http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    drop(core);
    let channel = access(&mut c, u, Id(ch), Permissions::MANAGE_WEBHOOKS).await?;
    let g = channel
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let list: Vec<Webhook> = manageable(&mut c, u, g)
        .await?
        .into_iter()
        .filter(|w| w.channel_id == Id(ch))
        .collect();
    Ok(crate::http::json(list))
}

/// The secret URL's webhook: (webhook, its account, its session key).
async fn authorize(app: &App, w: u64, token: &str) -> Result<(Webhook, Vec<u8>)> {
    let gone = || Error::new(ErrorCode::NotFound);
    let w = Id(w);
    require(id(w).is_ok() && token.len() == 43, ErrorCode::NotFound)?;
    let mut c = app.0.pool.acquire().await?;
    let r = sqlx::query("SELECT token_hash,session_hash FROM webhooks WHERE id=?")
        .bind(id(w)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(gone)?;
    let expected: Vec<u8> = r.get("token_hash");
    let given = crate::state::hash(token.as_bytes());
    require(bool::from(expected.ct_eq(&given)), ErrorCode::NotFound)?;
    Ok((load(&mut c, w).await?, r.get("session_hash")))
}

#[derive(serde::Deserialize, Default)]
struct ExecuteQuery {
    #[serde(default)]
    wait: Option<bool>,
    #[serde(default)]
    thread_id: Option<String>,
}

fn flexible_ids<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<Id>, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Flex {
        N(u64),
        S(String),
    }
    let v: Vec<Flex> = serde::Deserialize::deserialize(d)?;
    v.into_iter()
        .take(100)
        .map(|f| match f {
            Flex::N(n) => Ok(Id(n)),
            Flex::S(s) => s.parse::<u64>().map(Id).map_err(serde::de::Error::custom),
        })
        .collect()
}

/// Discord's `allowed_mentions`, with IDs as numbers or strings.
#[derive(serde::Deserialize, Default)]
struct Allowed {
    #[serde(default)]
    parse: Vec<MentionParse>,
    #[serde(default, deserialize_with = "flexible_ids")]
    users: Vec<Id>,
    #[serde(default, deserialize_with = "flexible_ids")]
    roles: Vec<Id>,
}

/// The JSON body (Discord's shape; unknown fields are ignored).
#[derive(serde::Deserialize, Default)]
struct Payload {
    #[serde(default)]
    content: String,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
    #[serde(default)]
    embeds: Vec<EmbedDraft>,
    #[serde(default)]
    allowed_mentions: Option<Allowed>,
    #[serde(default)]
    thread_name: Option<String>,
}
impl Payload {
    fn into_post(self, channel_id: Id, attachment_ids: Vec<Id>) -> WebhookPost {
        let a = self.allowed_mentions.unwrap_or(Allowed {
            parse: vec![MentionParse::Users],
            ..Default::default()
        });
        WebhookPost {
            channel_id,
            content: self.content,
            username: self.username,
            avatar_url: self.avatar_url,
            embeds: self.embeds,
            allowed_mentions: AllowedMentions {
                parse: a.parse,
                users: a.users,
                roles: a.roles,
            },
            attachment_ids,
            thread_name: self.thread_name,
        }
    }
}

fn thread(q: &ExecuteQuery, hook: &Webhook) -> Result<Id> {
    match &q.thread_id {
        None => Ok(hook.channel_id),
        Some(s) => s
            .parse::<u64>()
            .ok()
            .filter(|v| *v > 0 && *v <= i64::MAX as u64)
            .map(Id)
            .ok_or_else(|| invalid("thread_id", "thread_id is a thread's ID.")),
    }
}

/// Runs a webhook command as the webhook's own account.
async fn run(app: &App, hook: &Webhook, key: &[u8], cmd: AppCommand) -> Result<CommandResult> {
    let mut core = app.0.core.lock().await;
    let nonce = u64::from_be_bytes(crate::state::random(8).try_into().unwrap_or([1; 8]))
        & (i64::MAX as u64);
    let response = crate::commands::execute(
        app,
        &mut core,
        key,
        hook.user_id,
        CommandRequest {
            nonce: nonce.max(1),
            command: Command::Apps(cmd),
        },
    )
    .await?;
    match response.outcome {
        CommandOutcome::Ok(r) => Ok(r),
        CommandOutcome::Err(e) => Err(Error(e)),
    }
}

async fn read_json<T: serde::de::DeserializeOwned>(r: Request<Body>, cap: usize) -> Result<T> {
    let bytes = axum::body::to_bytes(r.into_body(), cap)
        .await
        .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?;
    serde_json::from_slice(&bytes).map_err(|e| {
        let mut err = Error::new(ErrorCode::InvalidRequest);
        err.0.message = format!("The body is not valid JSON for this webhook: {e}");
        err
    })
}

fn content_type(r: &Request<Body>) -> String {
    r.headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn answer(result: CommandResult, wait: bool) -> Result<Response> {
    let CommandResult::Apps(AppResult::Message(m)) = result else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    if wait {
        Ok(crate::http::json(*m).into_response())
    } else {
        Ok(StatusCode::NO_CONTENT.into_response())
    }
}

/// Requests to webhook URLs from one address are bounded.
async fn throttle(app: &App, ip: String) -> Result<()> {
    app.0
        .core
        .lock()
        .await
        .rate(format!("webhook-ip:{ip}"), 120., 30.)
}

async fn execute_json(
    State(app): State<App>,
    Path((w, token)): Path<(u64, String)>,
    Query(q): Query<ExecuteQuery>,
    r: Request<Body>,
) -> Result<Response> {
    throttle(&app, crate::http::auth_bucket(&app, &r)).await?;
    let (hook, key) = authorize(&app, w, &token).await?;
    let target = thread(&q, &hook)?;
    let post = match content_type(&r).as_str() {
        "application/json" => read_json::<Payload>(r, 64 * 1024)
            .await?
            .into_post(target, vec![]),
        "multipart/form-data" => multipart(&app, &hook, target, r).await?,
        _ => return Err(Error::new(ErrorCode::UnsupportedMediaType)),
    };
    let result = run(
        &app,
        &hook,
        &key,
        AppCommand::ExecuteWebhook(Box::new(post)),
    )
    .await?;
    answer(result, q.wait.unwrap_or(false))
}

async fn edit_json(
    State(app): State<App>,
    Path((w, token, m)): Path<(u64, String, u64)>,
    Query(q): Query<ExecuteQuery>,
    r: Request<Body>,
) -> Result<Response> {
    throttle(&app, crate::http::auth_bucket(&app, &r)).await?;
    let (hook, key) = authorize(&app, w, &token).await?;
    let target = thread(&q, &hook)?;
    require(
        content_type(&r) == "application/json",
        ErrorCode::UnsupportedMediaType,
    )?;
    let post = read_json::<Payload>(r, 64 * 1024)
        .await?
        .into_post(target, vec![]);
    let result = run(
        &app,
        &hook,
        &key,
        AppCommand::EditWebhookMessage {
            channel_id: target,
            message_id: Id(m),
            post: Box::new(post),
        },
    )
    .await?;
    answer(result, true)
}

async fn delete_json(
    State(app): State<App>,
    Path((w, token, m)): Path<(u64, String, u64)>,
    Query(q): Query<ExecuteQuery>,
    r: Request<Body>,
) -> Result<Response> {
    throttle(&app, crate::http::auth_bucket(&app, &r)).await?;
    let (hook, key) = authorize(&app, w, &token).await?;
    let target = thread(&q, &hook)?;
    run(
        &app,
        &hook,
        &key,
        AppCommand::DeleteWebhookMessage {
            channel_id: target,
            message_id: Id(m),
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET` on the secret URL: the webhook, without its secret.
async fn info(
    State(app): State<App>,
    Path((w, token)): Path<(u64, String)>,
    r: Request<Body>,
) -> Result<crate::http::ApiResponse> {
    throttle(&app, crate::http::auth_bucket(&app, &r)).await?;
    let (hook, _) = authorize(&app, w, &token).await?;
    Ok(crate::http::json(hook))
}

/// Multipart: `payload_json` and up to ten files (`files[n]` or `file`).
async fn multipart(app: &App, hook: &Webhook, target: Id, r: Request<Body>) -> Result<WebhookPost> {
    use axum::extract::FromRequest;
    let mut form = axum::extract::Multipart::from_request(r, app)
        .await
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let mut payload = Payload::default();
    let mut files = vec![];
    // Files go where the message goes: a forum's are uploaded to the forum.
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?
    {
        let name = field.name().unwrap_or_default().to_owned();
        if name == "payload_json" {
            let bytes = field
                .bytes()
                .await
                .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?;
            require(bytes.len() <= 64 * 1024, ErrorCode::PayloadTooLarge)?;
            payload = serde_json::from_slice(&bytes)
                .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        } else if name == "file" || name.starts_with("files[") || name.starts_with("file[") {
            require(
                files.len() < MAX_MESSAGE_ATTACHMENTS,
                ErrorCode::LimitExceeded,
            )?;
            files.push(super::rest::store_upload(app, hook.user_id, target, field).await?);
        } else {
            return Err(invalid(
                "multipart",
                "Send payload_json and files[n] parts.",
            ));
        }
    }
    Ok(payload.into_post(target, files))
}

// --------------------------------------------------------- Slack shape

#[derive(serde::Deserialize, Default)]
struct SlackField {
    #[serde(default)]
    title: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    short: bool,
}
#[derive(serde::Deserialize, Default)]
struct SlackAttachment {
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    pretext: Option<String>,
    #[serde(default)]
    author_name: Option<String>,
    #[serde(default)]
    author_link: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    title_link: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    fields: Vec<SlackField>,
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    thumb_url: Option<String>,
    #[serde(default)]
    footer: Option<String>,
    #[serde(default)]
    ts: Option<serde_json::Value>,
}
#[derive(serde::Deserialize, Default)]
struct SlackPayload {
    #[serde(default)]
    text: String,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    attachments: Vec<SlackAttachment>,
}

/// Slack's `<url|label>` links, `<url>` and entities, as Markdown.
pub(crate) fn slack_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        let Some(close) = tail.find('>') else {
            out.push('<');
            rest = tail;
            continue;
        };
        let inner = &tail[..close];
        let (target, label) = inner.split_once('|').unwrap_or((inner, ""));
        if messages::web_url(target) {
            if label.is_empty() {
                out.push_str(target);
            } else {
                out.push_str(&format!("[{label}]({target})"));
            }
        } else if let Some(name) = target.strip_prefix('!') {
            // `<!here>`, `<!channel>`: written out, never a ping.
            out.push_str(&format!("@\u{200B}{name}"));
        } else {
            out.push_str(if label.is_empty() { target } else { label });
        }
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn hex_color(s: &Option<String>) -> Option<u32> {
    let s = s.as_deref()?.trim();
    match s {
        "good" => Some(0x2EB886),
        "warning" => Some(0xDAA038),
        "danger" => Some(0xA30200),
        _ => u32::from_str_radix(s.trim_start_matches('#'), 16)
            .ok()
            .filter(|c| *c <= 0xFF_FFFF && s.trim_start_matches('#').len() == 6),
    }
}
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}
fn some(s: Option<String>, max: usize) -> Option<String> {
    s.map(|s| clip(s.trim(), max)).filter(|s| !s.is_empty())
}
fn link(s: Option<String>) -> Option<String> {
    s.filter(|u| messages::web_url(u))
}

fn from_slack(p: SlackPayload) -> Payload {
    let embeds = p
        .attachments
        .into_iter()
        .take(MAX_EMBEDS)
        .map(|a| {
            let mut description = String::new();
            for part in [a.pretext, a.text].into_iter().flatten() {
                if !description.is_empty() {
                    description.push('\n');
                }
                description.push_str(&slack_text(&part));
            }
            let timestamp =
                a.ts.as_ref()
                    .and_then(|v| {
                        v.as_f64()
                            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                    })
                    .and_then(|secs| jiff::Timestamp::from_second(secs as i64).ok())
                    .map(|t| t.to_string());
            EmbedDraft {
                title: some(a.title, MAX_EMBED_TITLE_CHARS),
                description: some(Some(description), MAX_EMBED_DESCRIPTION_CHARS),
                url: link(a.title_link),
                color: hex_color(&a.color),
                fields: a
                    .fields
                    .into_iter()
                    .take(MAX_EMBED_FIELDS)
                    .filter(|f| !f.title.trim().is_empty() && !f.value.trim().is_empty())
                    .map(|f| EmbedField {
                        name: clip(&f.title, MAX_EMBED_FIELD_NAME_CHARS),
                        value: clip(&slack_text(&f.value), MAX_EMBED_FIELD_VALUE_CHARS),
                        inline: f.short,
                    })
                    .collect(),
                author: some(a.author_name, MAX_EMBED_AUTHOR_CHARS).map(|name| EmbedAuthor {
                    name,
                    url: link(a.author_link),
                }),
                footer: some(a.footer, MAX_EMBED_FOOTER_CHARS).map(|text| EmbedFooter { text }),
                image: link(a.image_url).map(|url| EmbedUrl { url }),
                thumbnail: link(a.thumb_url).map(|url| EmbedUrl { url }),
                timestamp,
            }
        })
        .filter(|e| *e != EmbedDraft::default())
        .collect();
    Payload {
        content: clip(&slack_text(&p.text), MAX_MESSAGE_CHARS),
        username: p.username,
        avatar_url: link(p.icon_url),
        embeds,
        allowed_mentions: None,
        thread_name: None,
    }
}

async fn execute_slack(
    State(app): State<App>,
    Path((w, token)): Path<(u64, String)>,
    Query(q): Query<ExecuteQuery>,
    r: Request<Body>,
) -> Result<Response> {
    throttle(&app, crate::http::auth_bucket(&app, &r)).await?;
    let (hook, key) = authorize(&app, w, &token).await?;
    let target = thread(&q, &hook)?;
    let payload: SlackPayload = match content_type(&r).as_str() {
        "application/json" => read_json(r, 64 * 1024).await?,
        // Slack's form encoding: `payload=<json>`.
        "application/x-www-form-urlencoded" => {
            let bytes = axum::body::to_bytes(r.into_body(), 64 * 1024)
                .await
                .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?;
            let form: HashMap<String, String> = serde_urlencoded::from_bytes(&bytes)
                .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
            let json = form
                .get("payload")
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            serde_json::from_str(json).map_err(|_| Error::new(ErrorCode::InvalidRequest))?
        }
        _ => return Err(Error::new(ErrorCode::UnsupportedMediaType)),
    };
    let post = from_slack(payload).into_post(target, vec![]);
    run(
        &app,
        &hook,
        &key,
        AppCommand::ExecuteWebhook(Box::new(post)),
    )
    .await?;
    // Slack clients expect a plain "ok".
    Ok((StatusCode::OK, [("content-type", "text/plain")], "ok").into_response())
}

// -------------------------------------------------------- GitHub shape

fn s(v: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut v = v;
    for p in path {
        v = v.get(p)?;
    }
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A GitHub event as an embed, or None for events that are not shown.
fn from_github(event: &str, v: &serde_json::Value) -> Option<Payload> {
    let repo = s(v, &["repository", "full_name"]).unwrap_or_else(|| "GitHub".into());
    let sender = s(v, &["sender", "login"]);
    let action = s(v, &["action"]).unwrap_or_default();
    let (title, url, description, color): (String, Option<String>, Option<String>, u32) =
        match event {
            "ping" => (
                format!("[{repo}] Webhook connected"),
                s(v, &["repository", "html_url"]),
                s(v, &["zen"]),
                0x5865F2,
            ),
            "push" => {
                let branch = s(v, &["ref"])?.rsplit('/').next()?.to_owned();
                let commits = v.get("commits")?.as_array()?;
                if commits.is_empty() {
                    return None;
                }
                let lines: Vec<String> = commits
                    .iter()
                    .take(5)
                    .map(|c| {
                        let sha: String =
                            s(c, &["id"]).unwrap_or_default().chars().take(7).collect();
                        let msg = s(c, &["message"]).unwrap_or_default();
                        let first = msg.lines().next().unwrap_or_default();
                        let author = s(c, &["author", "username"])
                            .or_else(|| s(c, &["author", "name"]))
                            .unwrap_or_default();
                        format!("`{sha}` {} - {author}", clip(first, 80))
                    })
                    .collect();
                let n = commits.len();
                (
                    format!(
                        "[{repo}:{branch}] {n} new commit{}",
                        if n == 1 { "" } else { "s" }
                    ),
                    s(v, &["compare"]),
                    Some(lines.join("\n")),
                    0x7289DA,
                )
            }
            "pull_request" => {
                let merged =
                    v.pointer("/pull_request/merged").and_then(|m| m.as_bool()) == Some(true);
                let what = match action.as_str() {
                    "opened" | "reopened" | "ready_for_review" => action.replace('_', " "),
                    "closed" if merged => "merged".into(),
                    "closed" => "closed".into(),
                    _ => return None,
                };
                (
                    format!(
                        "[{repo}] Pull request {what}: #{} {}",
                        s(v, &["pull_request", "number"]).unwrap_or_default(),
                        s(v, &["pull_request", "title"]).unwrap_or_default()
                    ),
                    s(v, &["pull_request", "html_url"]),
                    (action == "opened")
                        .then(|| s(v, &["pull_request", "body"]))
                        .flatten(),
                    if merged {
                        0x6F42C1
                    } else if action == "closed" {
                        0xCB2431
                    } else {
                        0x2CBE4E
                    },
                )
            }
            "issues" => {
                if !matches!(action.as_str(), "opened" | "closed" | "reopened") {
                    return None;
                }
                (
                    format!(
                        "[{repo}] Issue {action}: #{} {}",
                        s(v, &["issue", "number"]).unwrap_or_default(),
                        s(v, &["issue", "title"]).unwrap_or_default()
                    ),
                    s(v, &["issue", "html_url"]),
                    (action == "opened")
                        .then(|| s(v, &["issue", "body"]))
                        .flatten(),
                    if action == "closed" {
                        0xCB2431
                    } else {
                        0xEB6420
                    },
                )
            }
            "issue_comment" => {
                if action != "created" {
                    return None;
                }
                (
                    format!(
                        "[{repo}] New comment on #{}: {}",
                        s(v, &["issue", "number"]).unwrap_or_default(),
                        s(v, &["issue", "title"]).unwrap_or_default()
                    ),
                    s(v, &["comment", "html_url"]),
                    s(v, &["comment", "body"]),
                    0xE68D60,
                )
            }
            "release" => {
                if action != "published" {
                    return None;
                }
                (
                    format!(
                        "[{repo}] New release published: {}",
                        s(v, &["release", "tag_name"]).unwrap_or_default()
                    ),
                    s(v, &["release", "html_url"]),
                    s(v, &["release", "name"]),
                    0x2CBE4E,
                )
            }
            "star" => {
                if action != "created" {
                    return None;
                }
                (
                    format!("[{repo}] New star added"),
                    s(v, &["repository", "html_url"]),
                    None,
                    0xFFD33D,
                )
            }
            "create" | "delete" => (
                format!(
                    "[{repo}] {} {} {}",
                    if event == "create" { "New" } else { "Deleted" },
                    s(v, &["ref_type"]).unwrap_or_default(),
                    if event == "create" { "created:" } else { "" }
                )
                .replace("  ", " ")
                    + &s(v, &["ref"]).unwrap_or_default(),
                s(v, &["repository", "html_url"]),
                None,
                if event == "create" {
                    0x2CBE4E
                } else {
                    0xCB2431
                },
            ),
            _ => return None,
        };
    Some(Payload {
        content: String::new(),
        username: Some("GitHub".into()),
        avatar_url: None,
        embeds: vec![EmbedDraft {
            title: Some(clip(&title, MAX_EMBED_TITLE_CHARS)),
            description: description
                .map(|d| clip(d.trim(), 500))
                .filter(|d| !d.is_empty()),
            url: link(url),
            color: Some(color),
            author: sender.map(|name| EmbedAuthor {
                name: clip(&name, MAX_EMBED_AUTHOR_CHARS),
                url: None,
            }),
            ..Default::default()
        }],
        allowed_mentions: Some(Allowed::default()),
        thread_name: None,
    })
}

async fn execute_github(
    State(app): State<App>,
    Path((w, token)): Path<(u64, String)>,
    Query(q): Query<ExecuteQuery>,
    headers: HeaderMap,
    r: Request<Body>,
) -> Result<Response> {
    throttle(&app, crate::http::auth_bucket(&app, &r)).await?;
    let (hook, key) = authorize(&app, w, &token).await?;
    let target = thread(&q, &hook)?;
    require(
        content_type(&r) == "application/json",
        ErrorCode::UnsupportedMediaType,
    )?;
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body: serde_json::Value = read_json(r, 1024 * 1024).await?;
    let Some(payload) = from_github(&event, &body) else {
        // Accepted, not shown.
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let post = payload.into_post(target, vec![]);
    run(
        &app,
        &hook,
        &key,
        AppCommand::ExecuteWebhook(Box::new(post)),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod unit {
    use super::*;
    #[test]
    fn slack_markup_becomes_markdown_and_never_pings() {
        assert_eq!(
            slack_text("Deploy <https://ci.example.com/1|#1> done &amp; <!channel>"),
            "Deploy [#1](https://ci.example.com/1) done & @\u{200B}channel"
        );
        assert_eq!(slack_text("<javascript:alert(1)|x>"), "x");
        let p = from_slack(serde_json::from_str(r#"{"text":"hi","attachments":[{"color":"good","title":"Build","title_link":"https://ci.example.com","fields":[{"title":"Branch","value":"main","short":true}]}]}"#).unwrap());
        assert_eq!(p.embeds.len(), 1);
        assert_eq!(p.embeds[0].color, Some(0x2EB886));
        assert_eq!(p.embeds[0].fields[0].name, "Branch");
    }
    #[test]
    fn github_events_become_embeds() {
        let push = serde_json::json!({
            "ref":"refs/heads/main","compare":"https://github.com/o/r/compare/a...b",
            "repository":{"full_name":"o/r"},"sender":{"login":"alex"},
            "commits":[{"id":"0123456789","message":"Fix the thing\n\nmore","author":{"username":"alex"}}]
        });
        let p = from_github("push", &push).unwrap();
        assert_eq!(
            p.embeds[0].title.as_deref(),
            Some("[o/r:main] 1 new commit")
        );
        assert!(p.embeds[0]
            .description
            .as_deref()
            .unwrap()
            .contains("`0123456` Fix the thing"));
        assert!(from_github("watch", &push).is_none());
        let pr = serde_json::json!({"action":"closed","repository":{"full_name":"o/r"},"pull_request":{"number":7,"title":"T","merged":true,"html_url":"https://github.com/o/r/pull/7"}});
        assert!(from_github("pull_request", &pr).unwrap().embeds[0]
            .title
            .as_deref()
            .unwrap()
            .contains("merged"));
    }
}
