//! MCP tools. One table drives `tools/list` schemas, argument validation and
//! write gating. Reads reuse the REST projections; writes go through
//! `commands::execute`, so permissions, slowmode, rate limits, receipts and live
//! events behave exactly as for the user's own clients.
use super::transport::Grant;
use crate::{
    db,
    error::Error,
    state::{App, Core},
};
use serde_json::{json, Map, Value};
use sqlx::SqliteConnection;
use std::collections::HashMap;
use sticktalk_proto::*;

type RpcResult = std::result::Result<Value, (i64, String)>;

#[derive(Clone, Copy)]
enum Kind {
    Id,
    Text(usize),
    Limit { default: i64, max: i64 },
    Bool,
}
struct Arg {
    name: &'static str,
    kind: Kind,
    required: bool,
    description: &'static str,
}
struct Tool {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    write: bool,
    destructive: bool,
    idempotent: bool,
    args: &'static [Arg],
}
const fn arg(name: &'static str, kind: Kind, required: bool, description: &'static str) -> Arg {
    Arg {
        name,
        kind,
        required,
        description,
    }
}
const SERVER: Arg = arg("server_id", Kind::Id, true, "Server ID from list_servers.");
const CHANNEL: Arg = arg(
    "channel_id",
    Kind::Id,
    true,
    "Channel or DM ID from list_channels or list_direct_messages.",
);
const MESSAGE: Arg = arg(
    "message_id",
    Kind::Id,
    true,
    "Message ID from read_messages.",
);
const TOOLS: &[Tool] = &[
    Tool {
        name: "get_me",
        title: "Who am I",
        description: "The signed-in StickTalk user and whether this connection may send messages.",
        write: false,
        destructive: false,
        idempotent: true,
        args: &[],
    },
    Tool {
        name: "list_servers",
        title: "List servers",
        description: "Servers (communities) the user belongs to, with unread and mention counts.",
        write: false,
        destructive: false,
        idempotent: true,
        args: &[],
    },
    Tool {
        name: "list_channels",
        title: "List channels",
        description: "Channels in a server that the user can see, in sidebar order, with category, the channels each is nested inside (outermost first), topic, unread counts and whether the user can send there.",
        write: false,
        destructive: false,
        idempotent: true,
        args: &[SERVER],
    },
    Tool {
        name: "list_direct_messages",
        title: "List DMs",
        description: "Open direct messages and group DMs, most recent first, with recipients and unread counts.",
        write: false,
        destructive: false,
        idempotent: true,
        args: &[],
    },
    Tool {
        name: "list_unread",
        title: "List unread",
        description: "Channels and DMs with unread messages, mentions first. Use read_messages with after = last_read_message_id to read just the new ones.",
        write: false,
        destructive: false,
        idempotent: true,
        args: &[],
    },
    Tool {
        name: "read_messages",
        title: "Read messages",
        description: "Messages in a channel or DM, oldest first. Without a cursor returns the newest. Page back with before = older_cursor, forward with after = newer_cursor, or center on a message with around. Content is raw StickTalk markdown; <@ID> is a user mention.",
        write: false,
        destructive: false,
        idempotent: true,
        args: &[
            CHANNEL,
            arg("limit", Kind::Limit { default: 25, max: 50 }, false, "Messages to return, 1-50 (default 25)."),
            arg("before", Kind::Id, false, "Return messages older than this message ID."),
            arg("after", Kind::Id, false, "Return messages newer than this message ID."),
            arg("around", Kind::Id, false, "Return messages around this message ID, including it."),
        ],
    },
    Tool {
        name: "search_messages",
        title: "Search messages",
        description: "Full-text search of a server's message history the user can read, newest first. Query syntax: words or \"exact phrases\", plus filters from:USERNAME, in:CHANNEL, mentions:USERNAME, before:YYYY-MM-DD, after:YYYY-MM-DD, has:link|file|image. DMs are not searchable.",
        write: false,
        destructive: false,
        idempotent: true,
        args: &[
            SERVER,
            arg("query", Kind::Text(512), true, "Search text and filters."),
            arg("limit", Kind::Limit { default: 25, max: 50 }, false, "Results, 1-50 (default 25)."),
            arg("before", Kind::Id, false, "Pass next_before from the previous page."),
        ],
    },
    Tool {
        name: "list_members",
        title: "List members",
        description: "Members of a server with usernames, display names and roles. Filter by name with query. Use a member's id to mention them as <@ID> or open a DM.",
        write: false,
        destructive: false,
        idempotent: true,
        args: &[
            SERVER,
            arg("query", Kind::Text(64), false, "Only members whose username or display name contains this."),
            arg("limit", Kind::Limit { default: 50, max: 100 }, false, "Members, 1-100 (default 50)."),
            arg("after", Kind::Id, false, "Pass next_cursor from the previous page."),
        ],
    },
    Tool {
        name: "send_message",
        title: "Send message",
        description: "Post a message as the user in a channel or DM. It is visible to everyone there under the user's name, so confirm the text with the user unless they asked you to post it. Up to 4000 characters of StickTalk markdown; mention with <@USER_ID>.",
        write: true,
        destructive: false,
        idempotent: false,
        args: &[
            CHANNEL,
            arg("content", Kind::Text(4000), true, "Message text."),
            arg("reply_to_message_id", Kind::Id, false, "Reply to this message in the same channel."),
        ],
    },
    Tool {
        name: "edit_message",
        title: "Edit message",
        description: "Replace the text of one of the user's own messages. The previous text is not kept.",
        write: true,
        destructive: true,
        idempotent: true,
        args: &[
            CHANNEL,
            MESSAGE,
            arg("content", Kind::Text(4000), true, "New message text."),
        ],
    },
    Tool {
        name: "delete_message",
        title: "Delete message",
        description: "Permanently delete a message (the user's own, or anyone's where they may manage messages). Cannot be undone: confirm with the user first, then pass confirm: true.",
        write: true,
        destructive: true,
        idempotent: true,
        args: &[
            CHANNEL,
            MESSAGE,
            arg("confirm", Kind::Bool, true, "Must be true. Deletion is permanent."),
        ],
    },
    Tool {
        name: "add_reaction",
        title: "Add reaction",
        description: "React to a message as the user with a Unicode emoji (for example 👍) or a server emoji written <:name:ID>.",
        write: true,
        destructive: false,
        idempotent: true,
        args: &[CHANNEL, MESSAGE, arg("emoji", Kind::Text(128), true, "The emoji.")],
    },
    Tool {
        name: "remove_reaction",
        title: "Remove reaction",
        description: "Remove the user's own reaction from a message.",
        write: true,
        destructive: false,
        idempotent: true,
        args: &[CHANNEL, MESSAGE, arg("emoji", Kind::Text(128), true, "The emoji.")],
    },
    Tool {
        name: "mark_read",
        title: "Mark read",
        description: "Mark a channel or DM read up to a message (default: the newest), clearing its unread badge on the user's devices.",
        write: true,
        destructive: false,
        idempotent: true,
        args: &[
            CHANNEL,
            arg("message_id", Kind::Id, false, "Read up to and including this message."),
        ],
    },
    Tool {
        name: "open_dm",
        title: "Open DM",
        description: "Get (or open) the direct-message channel with a user, by user_id or username, so you can read or send there.",
        write: true,
        destructive: false,
        idempotent: true,
        args: &[
            arg("user_id", Kind::Id, false, "User ID, e.g. from list_members."),
            arg("username", Kind::Text(64), false, "Username, without @."),
        ],
    },
];

pub(crate) fn list(write: bool) -> Vec<Value> {
    TOOLS
        .iter()
        .filter(|t| write || !t.write)
        .map(|t| {
            let mut properties = Map::new();
            for a in t.args {
                let schema = match a.kind {
                    Kind::Id => json!({"type": "string", "pattern": "^[0-9]{1,19}$"}),
                    Kind::Text(max) => json!({"type": "string", "minLength": 1, "maxLength": max}),
                    Kind::Limit { default, max } => {
                        json!({"type": "integer", "minimum": 1, "maximum": max, "default": default})
                    }
                    Kind::Bool => json!({"type": "boolean"}),
                };
                let mut schema = schema;
                schema["description"] = a.description.into();
                properties.insert(a.name.into(), schema);
            }
            let required: Vec<_> = t
                .args
                .iter()
                .filter(|a| a.required)
                .map(|a| a.name)
                .collect();
            json!({
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "inputSchema": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false,
                },
                "annotations": {
                    "title": t.title,
                    "readOnlyHint": !t.write,
                    "destructiveHint": t.destructive,
                    "idempotentHint": t.idempotent,
                    "openWorldHint": false,
                },
            })
        })
        .collect()
}

struct Fail {
    code: String,
    message: String,
}
type Out = std::result::Result<Value, Fail>;
fn fail(code: &str, message: impl Into<String>) -> Fail {
    Fail {
        code: code.into(),
        message: message.into(),
    }
}
impl From<sqlx::Error> for Fail {
    fn from(e: sqlx::Error) -> Self {
        Error::from(e).into()
    }
}
impl From<Error> for Fail {
    fn from(e: Error) -> Self {
        let e = e.0;
        let custom = e.message != format!("{:?}", e.code);
        let message = match e.code {
            ErrorCode::NotFound => "Not found, or the user can't see it. Check the IDs with list_servers, list_channels or read_messages.".to_owned(),
            ErrorCode::Forbidden => "The user doesn't have permission to do that here.".to_owned(),
            ErrorCode::Blocked => "This user can't be messaged because one of you blocked the other.".to_owned(),
            ErrorCode::RateLimited => format!(
                "Rate limited. Wait {} seconds and try again.",
                e.retry_after_ms.unwrap_or(1000).div_ceil(1000)
            ),
            ErrorCode::InvalidRequest if custom => e.message,
            ErrorCode::InvalidRequest => "StickTalk rejected the request. Check the IDs, that content is 1-4000 characters, that mentioned users can see the channel, and that a reply target is in the same channel.".to_owned(),
            ErrorCode::LimitExceeded => "A StickTalk limit was reached (for example reactions per message or open DMs).".to_owned(),
            ErrorCode::Unavailable => "StickTalk is busy. Try again shortly.".to_owned(),
            _ if custom => e.message,
            code => format!("StickTalk returned {}.", db::enum_str(code)),
        };
        Fail {
            code: db::enum_str(e.code),
            message,
        }
    }
}
fn success(v: Value) -> Value {
    json!({
        "content": [{"type": "text", "text": v.to_string()}],
        "structuredContent": v,
        "isError": false,
    })
}
/// Errors go in both places: some clients show only text, others only structure.
fn failure(f: Fail) -> Value {
    json!({
        "content": [{"type": "text", "text": format!("Error ({}): {}", f.code, f.message)}],
        "structuredContent": {"error": {"code": f.code, "message": f.message}},
        "isError": true,
    })
}

struct Args(Map<String, Value>);
impl Args {
    fn parse(tool: &Tool, mut map: Map<String, Value>) -> std::result::Result<Self, Fail> {
        map.retain(|_, v| !v.is_null());
        let accepted = || {
            if tool.args.is_empty() {
                "none".to_owned()
            } else {
                tool.args
                    .iter()
                    .map(|a| a.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        if let Some(k) = map.keys().find(|k| !tool.args.iter().any(|a| a.name == *k)) {
            return Err(fail(
                "invalid_arguments",
                format!(
                    "Unknown argument \"{k}\". Accepted arguments: {}.",
                    accepted()
                ),
            ));
        }
        for a in tool.args {
            let Some(v) = map.get_mut(a.name) else {
                if a.required {
                    return Err(fail(
                        "invalid_arguments",
                        format!(
                            "Missing required argument \"{}\": {}",
                            a.name, a.description
                        ),
                    ));
                }
                continue;
            };
            let bad = |what: &str| {
                fail(
                    "invalid_arguments",
                    format!("Argument \"{}\" must be {what}.", a.name),
                )
            };
            match a.kind {
                Kind::Id => {
                    let n = match &*v {
                        Value::String(s)
                            if (1..=19).contains(&s.len())
                                && s.bytes().all(|b| b.is_ascii_digit()) =>
                        {
                            s.parse::<u64>().ok()
                        }
                        Value::Number(n) => n.as_u64(),
                        _ => None,
                    }
                    .filter(|n| *n > 0 && *n <= i64::MAX as u64)
                    .ok_or_else(|| bad("an ID string of digits"))?;
                    *v = Value::from(n);
                }
                Kind::Text(max) => {
                    let s = v.as_str().ok_or_else(|| bad("a string"))?;
                    if s.trim().is_empty() || s.chars().count() > max {
                        return Err(bad(&format!(
                            "a non-empty string of at most {max} characters"
                        )));
                    }
                }
                Kind::Limit { max, .. } => {
                    // Clamp rather than reject: some clients hide min/max from the model.
                    let n = match &*v {
                        Value::Number(n) => n.as_f64(),
                        Value::String(s) => s.trim().parse::<f64>().ok(),
                        _ => None,
                    }
                    .ok_or_else(|| bad("an integer"))?;
                    *v = Value::from((n as i64).clamp(1, max));
                }
                Kind::Bool => {
                    if !v.is_boolean() {
                        return Err(bad("true or false"));
                    }
                }
            }
        }
        Ok(Self(map))
    }
    fn id(&self, k: &str) -> Option<Id> {
        self.0.get(k).and_then(Value::as_u64).map(Id)
    }
    fn req_id(&self, k: &str) -> Id {
        self.id(k).unwrap_or_default()
    }
    fn text(&self, k: &str) -> Option<String> {
        self.0.get(k).and_then(Value::as_str).map(str::to_owned)
    }
    fn limit(&self, tool: &Tool, k: &str) -> i64 {
        self.0.get(k).and_then(Value::as_i64).unwrap_or_else(|| {
            tool.args
                .iter()
                .find_map(|a| match a.kind {
                    Kind::Limit { default, .. } if a.name == k => Some(default),
                    _ => None,
                })
                .unwrap_or(25)
        })
    }
    fn flag(&self, k: &str) -> bool {
        self.0.get(k).and_then(Value::as_bool).unwrap_or(false)
    }
}

pub(crate) async fn call(
    app: &App,
    grant: &Grant,
    name: &str,
    args: Map<String, Value>,
) -> RpcResult {
    let Some(tool) = TOOLS.iter().find(|t| t.name == name) else {
        return Err((-32602, format!("Unknown tool: {name}")));
    };
    if tool.write && !grant.write {
        return Ok(failure(fail(
            "read_only_connection",
            "This connection was approved as read-only. To allow sending, disconnect StickTalk in your AI app (or log it out under Settings › Devices), connect again and choose \"Read and send\".",
        )));
    }
    let args = match Args::parse(tool, args) {
        Ok(a) => a,
        Err(f) => return Ok(failure(f)),
    };
    if let Err(e) = app
        .0
        .core
        .lock()
        .await
        .rate(format!("mcp:{}", grant.user.0), 120., 30.)
    {
        return Ok(failure(e.into()));
    }
    let ctx = Ctx { app, grant };
    let out = match tool.name {
        "get_me" => ctx.get_me().await,
        "list_servers" => ctx.list_servers().await,
        "list_channels" => ctx.list_channels(args.req_id("server_id")).await,
        "list_direct_messages" => ctx.list_dms().await,
        "list_unread" => ctx.list_unread().await,
        "read_messages" => ctx.read_messages(tool, &args).await,
        "search_messages" => ctx.search(tool, &args).await,
        "list_members" => ctx.list_members(tool, &args).await,
        "send_message" => ctx.send(&args).await,
        "edit_message" => ctx.edit(&args).await,
        "delete_message" => ctx.delete(&args).await,
        "add_reaction" | "remove_reaction" => ctx.react(&args, tool.name == "add_reaction").await,
        "mark_read" => ctx.mark_read(&args).await,
        "open_dm" => ctx.open_dm(&args).await,
        _ => Err(fail("internal", "Unhandled tool.")),
    };
    Ok(match out {
        Ok(v) => success(v),
        Err(f) => failure(f),
    })
}

// ---- Projections -----------------------------------------------------------

fn sid(i: Id) -> String {
    i.0.to_string()
}
/// RFC 3339 UTC from Unix milliseconds.
pub(crate) fn iso(ms: u64) -> String {
    let secs = ms / 1000;
    let rem = secs % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}
fn kind(k: ChannelKind) -> &'static str {
    match k {
        ChannelKind::Category => "category",
        ChannelKind::Text => "text",
        ChannelKind::Direct => "dm",
        ChannelKind::GroupDm => "group_dm",
        ChannelKind::Voice => "voice",
        ChannelKind::Announcement => "announcement",
        ChannelKind::Forum => "forum",
        ChannelKind::Thread => "thread",
    }
}
fn user_json(u: &UserSummary) -> Value {
    let mut v = json!({"id": sid(u.id), "username": u.username});
    if let Some(d) = &u.display_name {
        v["display_name"] = d.as_str().into();
    }
    v
}
/// Explicit `<@ID>` user mentions, as the desktop composer would send them.
pub(crate) fn mention_ids(content: &str) -> Vec<Id> {
    let mut out = vec![];
    let mut rest = content;
    while let Some(i) = rest.find("<@") {
        rest = &rest[i + 2..];
        let Some(end) = rest.find('>') else { break };
        let inner = rest[..end].strip_prefix('!').unwrap_or(&rest[..end]);
        if (1..=19).contains(&inner.len()) && inner.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(n) = inner.parse::<u64>() {
                if n > 0 && n <= i64::MAX as u64 && !out.contains(&Id(n)) && out.len() < 100 {
                    out.push(Id(n));
                }
            }
        }
    }
    out
}
#[derive(Default)]
struct Users(HashMap<Id, Option<UserSummary>>);
impl Users {
    async fn get(
        &mut self,
        c: &mut SqliteConnection,
        viewer: Id,
        u: Id,
    ) -> crate::Result<Option<&UserSummary>> {
        if let std::collections::hash_map::Entry::Vacant(slot) = self.0.entry(u) {
            let visible = db::profile_access(c, viewer, u).await.is_ok();
            slot.insert(if visible {
                Some(db::user(c, u).await?)
            } else {
                None
            });
        }
        Ok(self.0.get(&u).and_then(Option::as_ref))
    }
    async fn json(&mut self, c: &mut SqliteConnection, viewer: Id, u: Id) -> crate::Result<Value> {
        Ok(self
            .get(c, viewer, u)
            .await?
            .map(user_json)
            .unwrap_or_else(|| json!({"id": sid(u)})))
    }
}
async fn message_json(
    c: &mut SqliteConnection,
    users: &mut Users,
    viewer: Id,
    m: &Message,
) -> crate::Result<Value> {
    let mut v = json!({
        "id": sid(m.id),
        "author": users.json(c, viewer, m.author_id).await?,
        "content": m.content,
        "created_at": iso(m.created_at),
    });
    if let Some(t) = m.edited_at {
        v["edited_at"] = iso(t).into();
    }
    if let Some(r) = m.reply_to {
        v["reply_to_message_id"] = sid(r).into();
    }
    if !m.attachments.is_empty() {
        v["attachments"] = m
            .attachments
            .iter()
            .map(|a| json!({"filename": a.filename, "content_type": a.content_type, "size_bytes": a.byte_size}))
            .collect();
    }
    if !m.reactions.is_empty() {
        v["reactions"] = m
            .reactions
            .iter()
            .map(|r| json!({"emoji": r.emoji, "count": r.count, "me": r.me}))
            .collect();
    }
    if !m.mention_ids.is_empty() {
        let mut mentions = vec![];
        for u in &m.mention_ids {
            mentions.push(users.json(c, viewer, *u).await?);
        }
        v["mentions"] = mentions.into();
    }
    Ok(v)
}
async fn unread(c: &mut SqliteConnection, u: Id, ch: Id) -> crate::Result<(u32, u32, Option<Id>)> {
    let row: Option<(i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT unread_count,mention_count,last_read_id FROM read_states WHERE user_id=? AND channel_id=?",
    )
    .bind(db::id(u)?)
    .bind(db::id(ch)?)
    .fetch_optional(c)
    .await?;
    Ok(row.map_or((0, 0, None), |(n, m, l)| {
        (n as u32, m as u32, l.map(|v| Id(v as u64)))
    }))
}
async fn latest(c: &mut SqliteConnection, ch: Id) -> crate::Result<Option<Id>> {
    let v: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(id) FROM messages WHERE channel_id=? AND deleted_at IS NULL",
    )
    .bind(db::id(ch)?)
    .fetch_one(c)
    .await?;
    Ok(v.map(|v| Id(v as u64)))
}
/// A human name for any channel: `#name`, or the other DM participants.
async fn channel_name(
    c: &mut SqliteConnection,
    users: &mut Users,
    me: Id,
    ch: &ChannelSummary,
) -> crate::Result<String> {
    if ch.guild_id.is_some() || ch.name.is_some() {
        return Ok(ch.name.clone().unwrap_or_default());
    }
    let mut names = vec![];
    for u in ch.recipient_ids.iter().filter(|u| **u != me) {
        if let Some(s) = users.get(c, me, *u).await? {
            names.push(s.display_name.clone().unwrap_or_else(|| s.username.clone()));
        }
    }
    Ok(names.join(", "))
}

struct Ctx<'a> {
    app: &'a App,
    grant: &'a Grant,
}
impl Ctx<'_> {
    /// Reads hold the writer gate like REST snapshots do, so they never see a
    /// half-applied mutation's projections.
    async fn open(
        &self,
    ) -> crate::Result<(
        tokio::sync::MutexGuard<'_, Core>,
        sqlx::pool::PoolConnection<sqlx::Sqlite>,
    )> {
        let core = self.app.0.core.lock().await;
        let c = self.app.0.pool.acquire().await?;
        Ok((core, c))
    }
    fn me(&self) -> Id {
        self.grant.user
    }
    async fn get_me(&self) -> Out {
        let (_gate, mut c) = self.open().await?;
        let me = db::me(&mut c, self.me()).await?;
        Ok(json!({
            "user": user_json(&me.user),
            "access": if self.grant.write { "read_and_send" } else { "read_only" },
            "server_url": super::issuer(self.app),
        }))
    }
    async fn list_servers(&self) -> Out {
        let (_gate, mut c) = self.open().await?;
        let u = self.me();
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT guild_id FROM guild_members WHERE user_id=? ORDER BY guild_id",
        )
        .bind(db::id(u)?)
        .fetch_all(&mut *c)
        .await?;
        let mut servers = vec![];
        for g in ids {
            let g = Id(g as u64);
            let s = db::guild(&mut c, u, g).await?;
            let (mut unread_count, mut mention_count) = (0, 0);
            for ch in db::channels(&mut c, u, Some(g)).await? {
                if ch.kind == ChannelKind::Text {
                    let (n, m, _) = unread(&mut c, u, ch.id).await?;
                    unread_count += n;
                    mention_count += m;
                }
            }
            servers.push(json!({
                "id": sid(g),
                "name": s.guild.name,
                "member_count": s.guild.member_count,
                "my_role": db::enum_str(s.my_role),
                "unread_count": unread_count,
                "mention_count": mention_count,
            }));
        }
        Ok(json!({"servers": servers}))
    }
    async fn list_channels(&self, g: Id) -> Out {
        let (_gate, mut c) = self.open().await?;
        let u = self.me();
        let list = db::channels(&mut c, u, Some(g)).await?;
        let guild = db::guild(&mut c, u, g).await?;
        let mut out = vec![];
        for ch in &list {
            if ch.kind == ChannelKind::Category {
                continue;
            }
            let mut v = json!({"id": sid(ch.id), "name": ch.name, "type": kind(ch.kind)});
            // Every channel this one is nested in, outermost first.
            let mut inside = vec![];
            let mut at = ch.parent_id;
            while let Some(p) = at.and_then(|p| list.iter().find(|x| x.id == p)) {
                if inside.len() == list.len() {
                    break;
                }
                inside.push(p);
                at = p.parent_id;
            }
            if let Some(cat) = inside.first().filter(|p| p.kind == ChannelKind::Category) {
                v["category"] = cat.name.clone().into();
            }
            if !inside.is_empty() {
                v["inside"] = inside
                    .iter()
                    .rev()
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .into();
            }
            if let Some(topic) = db::channel(&mut c, ch.id).await?.topic {
                v["topic"] = topic.into();
            }
            if ch.kind == ChannelKind::Text {
                let (n, m, _) = unread(&mut c, u, ch.id).await?;
                v["unread_count"] = n.into();
                v["mention_count"] = m.into();
                v["can_send"] = crate::permissions::evaluate(&mut c, u, g, Some(ch.id))
                    .await?
                    .contains(Permissions::SEND_MESSAGES)
                    .into();
            }
            out.push(v);
        }
        Ok(json!({"server": {"id": sid(g), "name": guild.guild.name}, "channels": out}))
    }
    async fn list_dms(&self) -> Out {
        let (_gate, mut c) = self.open().await?;
        let u = self.me();
        let mut users = Users::default();
        let mut rows = vec![];
        for ch in db::channels(&mut c, u, None).await? {
            if ch.guild_id.is_some() {
                continue;
            }
            let mut recipients = vec![];
            for r in ch.recipient_ids.iter().filter(|r| **r != u) {
                recipients.push(users.json(&mut c, u, *r).await?);
            }
            let (n, m, _) = unread(&mut c, u, ch.id).await?;
            let last = latest(&mut c, ch.id).await?;
            let name = channel_name(&mut c, &mut users, u, &ch).await?;
            rows.push((
                last,
                json!({
                    "id": sid(ch.id),
                    "type": kind(ch.kind),
                    "name": name,
                    "recipients": recipients,
                    "unread_count": n,
                    "mention_count": m,
                    "last_message_id": last.map(sid),
                }),
            ));
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        Ok(json!({"direct_messages": rows.into_iter().map(|r| r.1).collect::<Vec<_>>()}))
    }
    async fn list_unread(&self) -> Out {
        let (_gate, mut c) = self.open().await?;
        let u = self.me();
        let mut users = Users::default();
        let mut servers: HashMap<Id, String> = HashMap::new();
        let mut rows = vec![];
        for ch in db::channels(&mut c, u, None).await? {
            if !matches!(
                ch.kind,
                ChannelKind::Text | ChannelKind::Direct | ChannelKind::GroupDm
            ) {
                continue;
            }
            let (n, m, last_read) = unread(&mut c, u, ch.id).await?;
            if n == 0 && m == 0 {
                continue;
            }
            let mut v = json!({
                "channel_id": sid(ch.id),
                "name": channel_name(&mut c, &mut users, u, &ch).await?,
                "type": kind(ch.kind),
                "unread_count": n,
                "mention_count": m,
            });
            if let Some(l) = last_read {
                v["last_read_message_id"] = sid(l).into();
            }
            if let Some(g) = ch.guild_id {
                if let std::collections::hash_map::Entry::Vacant(slot) = servers.entry(g) {
                    slot.insert(db::guild(&mut c, u, g).await?.guild.name);
                }
                v["server"] = json!({"id": sid(g), "name": servers[&g]});
            }
            rows.push(((m, n), v));
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        let total = rows.len();
        rows.truncate(50);
        Ok(
            json!({"unread": rows.into_iter().map(|r| r.1).collect::<Vec<_>>(), "total_channels": total}),
        )
    }
    async fn read_messages(&self, tool: &Tool, a: &Args) -> Out {
        let cursors = [a.id("before"), a.id("after"), a.id("around")];
        if cursors.iter().flatten().count() > 1 {
            return Err(fail(
                "invalid_arguments",
                "Use at most one of before, after and around.",
            ));
        }
        let [before, after, around] = cursors;
        let limit = a.limit(tool, "limit");
        let ch = a.req_id("channel_id");
        let (_gate, mut c) = self.open().await?;
        let u = self.me();
        let channel = db::text(&mut c, u, ch, Permissions::READ_HISTORY).await?;
        let cid = db::id(ch)?;
        let q_desc = "SELECT id FROM messages WHERE channel_id=? AND id<? AND deleted_at IS NULL ORDER BY id DESC LIMIT ?";
        let q_asc = "SELECT id FROM messages WHERE channel_id=? AND id>? AND deleted_at IS NULL ORDER BY id ASC LIMIT ?";
        let (ids, older, newer): (Vec<i64>, bool, bool) = if let Some(p) = around {
            let p = db::id(p)?;
            let mut left: Vec<i64> = sqlx::query_scalar("SELECT id FROM messages WHERE channel_id=? AND id<=? AND deleted_at IS NULL ORDER BY id DESC LIMIT ?")
                .bind(cid).bind(p).bind(limit + 1).fetch_all(&mut *c).await?;
            let mut right: Vec<i64> = sqlx::query_scalar(q_asc)
                .bind(cid)
                .bind(p)
                .bind(limit + 1)
                .fetch_all(&mut *c)
                .await?;
            let want_left = (((limit + 1) / 2) as usize)
                .max((limit as usize).saturating_sub(right.len()))
                .min(left.len());
            let want_right = (limit as usize - want_left).min(right.len());
            let (older, newer) = (left.len() > want_left, right.len() > want_right);
            left.truncate(want_left);
            left.reverse();
            right.truncate(want_right);
            left.extend(right);
            (left, older, newer)
        } else if let Some(x) = after {
            let x = db::id(x)?;
            let mut v: Vec<i64> = sqlx::query_scalar(q_asc)
                .bind(cid)
                .bind(x)
                .bind(limit + 1)
                .fetch_all(&mut *c)
                .await?;
            let newer = v.len() > limit as usize;
            v.truncate(limit as usize);
            let older: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages WHERE channel_id=? AND id<=? AND deleted_at IS NULL)").bind(cid).bind(x).fetch_one(&mut *c).await?;
            (v, older, newer)
        } else {
            let x = before.map(db::id).transpose()?.unwrap_or(i64::MAX);
            let mut v: Vec<i64> = sqlx::query_scalar(q_desc)
                .bind(cid)
                .bind(x)
                .bind(limit + 1)
                .fetch_all(&mut *c)
                .await?;
            let older = v.len() > limit as usize;
            v.truncate(limit as usize);
            v.reverse();
            let newer = before.is_some() && sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM messages WHERE channel_id=? AND id>=? AND deleted_at IS NULL)").bind(cid).bind(x).fetch_one(&mut *c).await?;
            (v, older, newer)
        };
        let mut users = Users::default();
        let mut messages = vec![];
        for id in &ids {
            let m = db::message(&mut c, u, ch, Id(*id as u64), false).await?;
            messages.push(message_json(&mut c, &mut users, u, &m).await?);
        }
        let mut info = json!({
            "id": sid(ch),
            "type": kind(channel.summary.kind),
            "name": channel_name(&mut c, &mut users, u, &channel.summary).await?,
        });
        if let Some(g) = channel.summary.guild_id {
            info["server"] =
                json!({"id": sid(g), "name": db::guild(&mut c, u, g).await?.guild.name});
        }
        let first = ids.first().map(|v| Id(*v as u64));
        let last = ids.last().map(|v| Id(*v as u64));
        Ok(json!({
            "channel": info,
            "messages": messages,
            "older_cursor": first.filter(|_| older).map(sid),
            "newer_cursor": last.filter(|_| newer).map(sid),
        }))
    }
    async fn search(&self, tool: &Tool, a: &Args) -> Out {
        let g = a.req_id("server_id");
        let u = self.me();
        let (mut gate, mut c) = self.open().await?;
        gate.rate(format!("search:{}", u.0), 30., 5.)?;
        let q = MessageSearchQuery {
            q: a.text("query").unwrap_or_default(),
            before: a.id("before"),
            limit: Some(a.limit(tool, "limit") as u16),
        };
        let (items, found_users, next_before, has_more, _) =
            crate::search::search(&mut c, u, g, &q).await?;
        let mut users = Users::default();
        for s in found_users {
            users.0.insert(s.id, Some(s));
        }
        let names: HashMap<Id, Option<String>> = db::channels(&mut c, u, Some(g))
            .await?
            .into_iter()
            .map(|ch| (ch.id, ch.name))
            .collect();
        let mut results = vec![];
        for m in &items {
            results.push(json!({
                "channel": {"id": sid(m.channel_id), "name": names.get(&m.channel_id).cloned().flatten()},
                "message": message_json(&mut c, &mut users, u, m).await?,
            }));
        }
        Ok(json!({
            "results": results,
            "has_more": has_more,
            "next_before": next_before.filter(|_| has_more).map(sid),
        }))
    }
    async fn list_members(&self, tool: &Tool, a: &Args) -> Out {
        let g = a.req_id("server_id");
        let limit = a.limit(tool, "limit");
        let (_gate, mut c) = self.open().await?;
        db::role(&mut c, self.me(), g).await?;
        let pattern = a.text("query").map(|q| {
            let escaped: String = q
                .trim()
                .chars()
                .flat_map(|ch| match ch {
                    '%' | '_' | '\\' => vec!['\\', ch],
                    ch => vec![ch],
                })
                .collect();
            format!("%{escaped}%")
        });
        let ids: Vec<i64> = sqlx::query_scalar("SELECT m.user_id FROM guild_members m JOIN users u ON u.id=m.user_id WHERE m.guild_id=?1 AND m.user_id>?2 AND (?4 IS NULL OR u.username LIKE ?4 ESCAPE '\\' OR u.display_name LIKE ?4 ESCAPE '\\') ORDER BY m.user_id LIMIT ?3")
            .bind(db::id(g)?)
            .bind(a.id("after").map(db::id).transpose()?.unwrap_or(0))
            .bind(limit + 1)
            .bind(&pattern)
            .fetch_all(&mut *c)
            .await?;
        let more = ids.len() > limit as usize;
        let mut members = vec![];
        for id in ids.iter().take(limit as usize) {
            let m = db::member(&mut c, g, Id(*id as u64)).await?;
            let mut v = user_json(&m.user);
            v["role"] = db::enum_str(m.role).into();
            if let Some(n) = m.nickname {
                v["nickname"] = n.into();
            }
            members.push(v);
        }
        Ok(json!({
            "members": members,
            "next_cursor": ids.get(limit as usize - 1).filter(|_| more).map(|v| v.to_string()),
        }))
    }

    /// Writes run as the connection's own session, exactly like a client command.
    async fn run(&self, command: Command) -> std::result::Result<CommandResult, Fail> {
        let mut core = self.app.0.core.lock().await;
        let nonce = loop {
            let mut b = [0u8; 8];
            b.copy_from_slice(&crate::state::random(8));
            let n = u64::from_le_bytes(b) & i64::MAX as u64;
            if n > 0 {
                break n;
            }
        };
        let response = crate::commands::execute(
            self.app,
            &mut core,
            &self.grant.session,
            self.grant.user,
            CommandRequest { nonce, command },
        )
        .await;
        // The backing session never opens a socket; don't retain a replay stream.
        if core
            .streams
            .get(&self.grant.session)
            .is_some_and(|s| s.connection.is_none())
        {
            core.streams.remove(&self.grant.session);
        }
        match response?.outcome {
            CommandOutcome::Ok(r) => Ok(r),
            CommandOutcome::Err(e) => Err(Error(e).into()),
        }
    }
    async fn message_out(&self, key: &str, r: CommandResult) -> Out {
        let CommandResult::Message(m) = r else {
            return Err(fail("internal", "Unexpected result."));
        };
        let (_gate, mut c) = self.open().await?;
        let mut users = Users::default();
        Ok(
            json!({key: true, "channel_id": sid(m.channel_id), "message": message_json(&mut c, &mut users, self.me(), &m).await?}),
        )
    }
    async fn send(&self, a: &Args) -> Out {
        let content = a.text("content").unwrap_or_default();
        let r = self
            .run(Command::SendMessage(MessageDraft {
                channel_id: a.req_id("channel_id"),
                mention_ids: mention_ids(&content),
                content,
                reply_to: a.id("reply_to_message_id"),
                attachment_ids: vec![],
            }))
            .await?;
        self.message_out("sent", r).await
    }
    async fn edit(&self, a: &Args) -> Out {
        let content = a.text("content").unwrap_or_default();
        let r = self
            .run(Command::EditMessage(MessageEdit {
                channel_id: a.req_id("channel_id"),
                message_id: a.req_id("message_id"),
                mention_ids: mention_ids(&content),
                content,
            }))
            .await?;
        self.message_out("edited", r).await
    }
    async fn delete(&self, a: &Args) -> Out {
        if !a.flag("confirm") {
            return Err(fail(
                "confirmation_required",
                "Deleting is permanent. Show the user the message, and once they agree call delete_message again with confirm: true.",
            ));
        }
        let (channel_id, message_id) = (a.req_id("channel_id"), a.req_id("message_id"));
        self.run(Command::DeleteMessage {
            channel_id,
            message_id,
        })
        .await?;
        Ok(json!({"deleted": true, "channel_id": sid(channel_id), "message_id": sid(message_id)}))
    }
    async fn react(&self, a: &Args, add: bool) -> Out {
        let (channel_id, message_id) = (a.req_id("channel_id"), a.req_id("message_id"));
        let emoji = a.text("emoji").unwrap_or_default().trim().to_owned();
        let r = self
            .run(if add {
                Command::AddReaction {
                    channel_id,
                    message_id,
                    emoji: emoji.clone(),
                }
            } else {
                Command::RemoveReaction {
                    channel_id,
                    message_id,
                    emoji: emoji.clone(),
                }
            })
            .await?;
        let count = match r {
            CommandResult::Reaction(r) => r.reaction.count,
            _ => 0,
        };
        Ok(json!({"message_id": sid(message_id), "emoji": emoji, "reacted": add, "count": count}))
    }
    async fn mark_read(&self, a: &Args) -> Out {
        let ch = a.req_id("channel_id");
        let target = match a.id("message_id") {
            Some(m) => Some(m),
            None => {
                let (_gate, mut c) = self.open().await?;
                db::text(&mut c, self.me(), ch, Permissions::VIEW_CHANNEL).await?;
                latest(&mut c, ch).await?
            }
        };
        let Some(message_id) = target else {
            return Ok(
                json!({"channel_id": sid(ch), "marked": false, "reason": "The channel has no messages."}),
            );
        };
        let r = self
            .run(Command::MarkRead {
                channel_id: ch,
                message_id,
            })
            .await?;
        let CommandResult::ReadState(s) = r else {
            return Err(fail("internal", "Unexpected result."));
        };
        Ok(json!({
            "channel_id": sid(ch),
            "marked": true,
            "last_read_message_id": s.last_read_id.map(sid),
            "unread_count": s.unread_count,
            "mention_count": s.mention_count,
        }))
    }
    async fn open_dm(&self, a: &Args) -> Out {
        let user_id = match (a.id("user_id"), a.text("username")) {
            (Some(id), None) => id,
            (None, Some(name)) => {
                let name = db::username(name.trim().trim_start_matches('@'))
                    .map_err(|_| fail("not_found", "No user has that username."))?;
                let (_gate, mut c) = self.open().await?;
                let found: Option<i64> =
                    sqlx::query_scalar("SELECT id FROM users WHERE username=?")
                        .bind(name)
                        .fetch_optional(&mut *c)
                        .await?;
                Id(found.ok_or_else(|| fail("not_found", "No user has that username."))? as u64)
            }
            _ => {
                return Err(fail(
                    "invalid_arguments",
                    "Pass exactly one of user_id or username.",
                ))
            }
        };
        let r = self.run(Command::OpenDm { user_id }).await.map_err(|f| {
            if f.code == "forbidden" {
                fail("forbidden", "StickTalk only opens new DMs between friends. Use list_direct_messages for existing DMs, or ask the user to add this person as a friend.")
            } else {
                f
            }
        })?;
        let CommandResult::Channel(ch) = r else {
            return Err(fail("internal", "Unexpected result."));
        };
        let (_gate, mut c) = self.open().await?;
        let mut users = Users::default();
        let mut recipients = vec![];
        for r in ch.summary.recipient_ids.iter().filter(|r| **r != self.me()) {
            recipients.push(users.json(&mut c, self.me(), *r).await?);
        }
        Ok(
            json!({"channel_id": sid(ch.summary.id), "type": kind(ch.summary.kind), "recipients": recipients}),
        )
    }
}

#[cfg(test)]
mod unit {
    use super::*;
    #[test]
    fn iso_dates() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(SNOWFLAKE_EPOCH_MS), "2025-01-01T00:00:00Z");
        assert_eq!(iso(1_709_208_000_000), "2024-02-29T12:00:00Z");
    }
    #[test]
    fn mentions_are_explicit_user_ids() {
        assert_eq!(
            mention_ids("hi <@12> and <@!12> <@&5> <@x> <@3>"),
            vec![Id(12), Id(3)]
        );
        assert!(mention_ids("<@99999999999999999999>").is_empty());
    }
    #[test]
    fn every_tool_dispatches_and_writes_are_flagged() {
        for t in TOOLS {
            assert_eq!(
                t.write,
                !matches!(
                    t.name,
                    "get_me"
                        | "list_servers"
                        | "list_channels"
                        | "list_direct_messages"
                        | "list_unread"
                        | "read_messages"
                        | "search_messages"
                        | "list_members"
                ),
                "{}",
                t.name
            );
            assert!(!t.destructive || t.write);
        }
        assert_eq!(list(false).len(), 8);
        assert_eq!(list(true).len(), TOOLS.len());
    }
}
