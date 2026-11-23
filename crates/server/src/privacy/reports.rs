//! In-app reports about a message, a person or a server.
//!
//! A report keeps what was reported as it was when it was filed (bounded
//! evidence), so a later edit or deletion does not erase it. Reports about
//! content in a server go to that server's moderators (Reports queue); reports
//! about direct messages, people and servers go to the instance administrator
//! (`sticktalk-server admin reports ...`). The reporter is never shown to the
//! reported person or to moderators; reporters can see the status of their own.
use super::Deliveries;
use crate::{
    db::*,
    error::{require, Error, Result},
    http,
    moderation::audit,
    permissions as acl,
    state::{now, App, Core},
};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
};
use sqlx::{Row, SqliteConnection};
use sticktalk_proto::*;

/// Who handles a server's reports: anyone with a moderation permission.
pub(crate) const HANDLERS: Permissions = Permissions::from_bits_retain(
    Permissions::MANAGE_MESSAGES.bits()
        | Permissions::KICK_MEMBERS.bits()
        | Permissions::BAN_MEMBERS.bits()
        | Permissions::MODERATE_MEMBERS.bits()
        | Permissions::ADMINISTRATOR.bits(),
);
pub(crate) async fn handler(c: &mut SqliteConnection, u: Id, g: Id) -> Result<bool> {
    Ok(match acl::evaluate(c, u, g, None).await {
        Ok(p) => p.intersects(HANDLERS),
        Err(e) if e.0.code == ErrorCode::NotFound => false,
        Err(e) => return Err(e),
    })
}
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut v: String = s.chars().take(max.saturating_sub(1)).collect();
        v.push('…');
        v
    }
}
fn clean_note(note: Option<String>, max: usize) -> Result<Option<String>> {
    let Some(n) = note else { return Ok(None) };
    let n = n.trim();
    if n.is_empty() {
        return Ok(None);
    }
    require(
        n.chars().count() <= max && !n.chars().any(|c| c.is_control() && c != '\n'),
        ErrorCode::InvalidRequest,
    )?;
    Ok(Some(n.to_owned()))
}
fn key(t: &ReportTarget) -> String {
    match t {
        ReportTarget::Message { message_id, .. } => format!("m:{}", message_id.0),
        ReportTarget::User { user_id } => format!("u:{}", user_id.0),
        ReportTarget::Guild { guild_id } => format!("g:{}", guild_id.0),
    }
}
fn summary(target: &ReportTarget, e: &ReportEvidence) -> String {
    let who = e
        .user
        .as_ref()
        .map_or("someone".to_owned(), |u| format!("@{}", u.username));
    match target {
        ReportTarget::Message { .. } => match (&e.channel_name, &e.guild_name) {
            (Some(ch), Some(_)) => format!("A message from {who} in #{ch}"),
            _ => format!("A direct message from {who}"),
        },
        ReportTarget::User { .. } => format!("{who}'s profile"),
        ReportTarget::Guild { .. } => format!(
            "The server {}",
            e.guild_name.clone().unwrap_or_else(|| "(deleted)".into())
        ),
    }
}
fn status_of(s: &str) -> ReportStatus {
    match s {
        "dismissed" => ReportStatus::Dismissed,
        "actioned" => ReportStatus::Actioned,
        _ => ReportStatus::Open,
    }
}
fn row(r: &sqlx::sqlite::SqliteRow) -> Result<Report> {
    let target: ReportTarget = serde_json::from_str(&r.get::<String, _>("target"))
        .map_err(|_| Error::new(ErrorCode::Internal))?;
    Ok(Report {
        id: Id(r.get::<i64, _>("id") as u64),
        target,
        queue: if r.get::<String, _>("queue") == "guild" {
            ReportQueue::Guild
        } else {
            ReportQueue::Admin
        },
        guild_id: r.get::<Option<i64>, _>("guild_id").map(|g| Id(g as u64)),
        category: en(r.get("category"))?,
        note: r.get("note"),
        evidence: serde_json::from_str(&r.get::<String, _>("evidence")).unwrap_or_default(),
        status: status_of(&r.get::<String, _>("status")),
        action: r.get("action"),
        created_at: r.get::<i64, _>("created_at") as u64,
        resolved_at: r.get::<Option<i64>, _>("resolved_at").map(|v| v as u64),
    })
}
fn mine(r: &Report) -> MyReport {
    MyReport {
        id: r.id,
        target: r.target,
        category: r.category,
        status: r.status,
        created_at: r.created_at,
        resolved_at: r.resolved_at,
        summary: summary(&r.target, &r.evidence),
    }
}
async fn load(c: &mut SqliteConnection, report: Id) -> Result<(Report, Id)> {
    let r = sqlx::query("SELECT * FROM reports WHERE id=?")
        .bind(id(report)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok((row(&r)?, Id(r.get::<i64, _>("reporter_id") as u64)))
}
async fn open_count(c: &mut SqliteConnection, g: Id) -> Result<u32> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reports WHERE queue='guild' AND guild_id=? AND status='open'",
    )
    .bind(id(g)?)
    .fetch_one(c)
    .await?;
    Ok(n as u32)
}
/// Tell the server's moderators who are online how many reports are open.
async fn queue_event(
    c: &mut SqliteConnection,
    core: &Core,
    out: &mut Deliveries,
    g: Id,
) -> Result<()> {
    let open = open_count(c, g).await?;
    let mut online: Vec<Id> = core.streams.values().map(|s| s.user).collect();
    online.sort();
    online.dedup();
    let mut to = vec![];
    for u in online {
        if handler(c, u, g).await? {
            to.push(u);
        }
    }
    if !to.is_empty() {
        out.push((
            to,
            Event::Privacy(PrivacyEvent::GuildReports { guild_id: g, open }),
        ));
    }
    Ok(())
}

pub(crate) async fn file(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    draft: ReportDraft,
    out: &mut Deliveries,
) -> Result<MyReport> {
    core.rate(
        format!("report:{}", u.0),
        f64::from(REPORTS_PER_HOUR) / 60.,
        3.,
    )?;
    let note = clean_note(draft.note, MAX_REPORT_NOTE_CHARS)?;
    let open: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE reporter_id=? AND status='open'")
            .bind(id(u)?)
            .fetch_one(&mut *c)
            .await?;
    require(
        open < MAX_OPEN_REPORTS_PER_USER as i64,
        ErrorCode::LimitExceeded,
    )?;
    let mut evidence = ReportEvidence::default();
    let (queue, guild_id, target_user) = match draft.target {
        ReportTarget::Message {
            channel_id,
            message_id,
        } => {
            let channel = text(
                c,
                u,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
            )
            .await?;
            let m = message(c, u, channel_id, message_id, false).await?;
            require(m.author_id != u, ErrorCode::InvalidRequest)?;
            evidence.user = Some(user(c, m.author_id).await?);
            evidence.content = clip(&m.content, MAX_REPORT_EVIDENCE_CHARS);
            evidence.attachments = m
                .attachments
                .iter()
                .take(MAX_MESSAGE_ATTACHMENTS)
                .map(|a| clip(&a.filename, 200))
                .collect();
            evidence.sent_at = Some(m.created_at);
            evidence.edited_at = m.edited_at;
            match channel.summary.guild_id {
                Some(g) => {
                    evidence.channel_name = channel.summary.name.clone();
                    evidence.guild_name = Some(guild(c, u, g).await?.guild.name);
                    ("guild", Some(g), Some(m.author_id))
                }
                None => ("admin", None, Some(m.author_id)),
            }
        }
        ReportTarget::User { user_id } => {
            require(user_id != u, ErrorCode::InvalidRequest)?;
            profile_access(c, u, user_id).await?;
            let p = crate::launch::user_profile(c, user_id).await?;
            evidence.about = Some(clip(&p.about_me, 1_000)).filter(|a| !a.is_empty());
            evidence.content = clip(
                &[p.user.display_name.clone(), p.user.status_text.clone()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("\n"),
                MAX_REPORT_EVIDENCE_CHARS,
            );
            evidence.user = Some(p.user);
            ("admin", None, Some(user_id))
        }
        ReportTarget::Guild { guild_id } => {
            let g = guild(c, u, guild_id).await?.guild;
            let profile = crate::launch::guild_profile(c, guild_id).await?;
            evidence.guild_name = Some(g.name);
            evidence.content = clip(&profile.description, MAX_REPORT_EVIDENCE_CHARS);
            evidence.user = Some(user(c, g.owner_id).await?);
            ("admin", None, None)
        }
    };
    let report = core.id().await?;
    let t = now();
    let inserted = sqlx::query("INSERT INTO reports(id,reporter_id,queue,guild_id,target,target_key,target_user_id,category,note,evidence,status,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,'open',?) ON CONFLICT DO NOTHING")
        .bind(id(report)?)
        .bind(id(u)?)
        .bind(queue)
        .bind(opt(guild_id)?)
        .bind(serde_json::to_string(&draft.target).map_err(|_| Error::new(ErrorCode::Internal))?)
        .bind(key(&draft.target))
        .bind(opt(target_user)?)
        .bind(enum_str(draft.category))
        .bind(&note)
        .bind(serde_json::to_string(&evidence).map_err(|_| Error::new(ErrorCode::Internal))?)
        .bind(t as i64)
        .execute(&mut *c)
        .await?
        .rows_affected();
    // One open report per person and target.
    require(inserted == 1, ErrorCode::AlreadyExists)?;
    if let Some(g) = guild_id {
        queue_event(c, core, out, g).await?;
    }
    let (r, _) = load(c, report).await?;
    Ok(mine(&r))
}

fn action_name(a: ReportAction) -> &'static str {
    match a {
        ReportAction::Dismiss => "dismiss",
        ReportAction::DeleteMessage => "delete_message",
        ReportAction::Timeout { .. } => "timeout",
        ReportAction::Kick => "kick",
        ReportAction::Ban => "ban",
    }
}
/// The existing command that carries out a moderator's action (None: nothing
/// to run). It runs with its own permission checks and audit entry.
async fn action_command(
    c: &mut SqliteConnection,
    g: Id,
    r: &Report,
    target_user: Option<Id>,
    action: ReportAction,
    reason: Option<String>,
) -> Result<Option<Command>> {
    let person = || target_user.ok_or_else(|| Error::new(ErrorCode::InvalidRequest));
    Ok(match action {
        ReportAction::Dismiss => None,
        ReportAction::DeleteMessage => {
            let ReportTarget::Message {
                channel_id,
                message_id,
            } = r.target
            else {
                return Err(Error::new(ErrorCode::InvalidRequest));
            };
            let alive: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id=? AND channel_id=? AND deleted_at IS NULL)",
            )
            .bind(id(message_id)?)
            .bind(id(channel_id)?)
            .fetch_one(&mut *c)
            .await?;
            alive.then_some(Command::DeleteMessage {
                channel_id,
                message_id,
            })
        }
        ReportAction::Timeout { seconds } => {
            require(seconds > 0, ErrorCode::InvalidRequest)?;
            Some(Command::Launch(LaunchCommand::Moderation(
                ModerationCommand::Timeout {
                    guild_id: g,
                    user_id: person()?,
                    seconds,
                    reason,
                },
            )))
        }
        ReportAction::Kick => Some(Command::Launch(LaunchCommand::Moderation(
            ModerationCommand::Kick {
                guild_id: g,
                user_id: person()?,
                reason,
            },
        ))),
        ReportAction::Ban => Some(Command::BanMember {
            guild_id: g,
            user_id: person()?,
            reason,
        }),
    })
}
/// `POST /api/v1/guilds/{g}/reports/{id}/resolve` with `ReportResolution`.
/// The action runs as an ordinary command of the moderator (its permission
/// check and audit entry included), then the report is closed and logged.
pub(crate) async fn resolve_endpoint(
    State(app): State<App>,
    Path((g, report)): Path<(String, String)>,
    r: axum::http::Request<axum::body::Body>,
) -> Result<http::ApiResponse> {
    let g = http::url_id(&g)?;
    let report = http::url_id(&report)?;
    let token = http::bearer(r.headers())?.to_owned();
    let body: ReportResolution = http::parse(r).await?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, &token).await?;
    core.rate(format!("report-resolve:{}", u.0), 60., 10.)?;
    let (found, reporter) = load(&mut c, report).await?;
    require(
        found.guild_id == Some(g) && found.queue == ReportQueue::Guild,
        ErrorCode::NotFound,
    )?;
    require(handler(&mut c, u, g).await?, ErrorCode::NotFound)?;
    require(found.status == ReportStatus::Open, ErrorCode::Conflict)?;
    let reason = clean_note(body.reason, MAX_REASON_CHARS)?;
    let target_user: Option<i64> =
        sqlx::query_scalar("SELECT target_user_id FROM reports WHERE id=?")
            .bind(id(report)?)
            .fetch_one(&mut *c)
            .await?;
    let target_user = target_user.map(|v| Id(v as u64));
    if let Some(command) =
        action_command(&mut c, g, &found, target_user, body.action, reason.clone()).await?
    {
        let bytes: [u8; 8] = crate::state::random(8)
            .try_into()
            .map_err(|_| Error::new(ErrorCode::Internal))?;
        let nonce = u64::from_le_bytes(bytes) % (i64::MAX as u64) + 1;
        drop(c);
        let response =
            crate::commands::execute(&app, &mut core, &key, u, CommandRequest { nonce, command })
                .await?;
        c = app.0.pool.acquire().await?;
        if let CommandOutcome::Err(e) = response.outcome {
            return Err(Error(e));
        }
    }
    let name = action_name(body.action);
    let mut out: Deliveries = vec![];
    {
        use sqlx::Connection as _;
        let mut tx = c.begin().await?;
        let n = sqlx::query("UPDATE reports SET status=?,action=?,resolved_at=?,resolved_by=? WHERE id=? AND status='open'")
            .bind(if body.action == ReportAction::Dismiss {
                "dismissed"
            } else {
                "actioned"
            })
            .bind(name)
            .bind(now() as i64)
            .bind(id(u)?)
            .bind(id(report)?)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        require(n == 1, ErrorCode::Conflict)?;
        let (kind, target) = match found.target {
            ReportTarget::Message { message_id, .. } => (AuditTarget::Message, Some(message_id)),
            _ => (AuditTarget::User, target_user),
        };
        audit::write(
            &mut tx,
            u,
            audit::Entry::new(g, AuditAction::ReportResolve, kind, target)
                .named(format!("Report: {}", found.category.label()))
                .change("action", None, Some(name.replace('_', " ")))
                .reason(reason),
        )
        .await?;
        tx.commit().await?;
    }
    queue_event(&mut c, &core, &mut out, g).await?;
    let (updated, _) = load(&mut c, report).await?;
    out.push((
        vec![reporter],
        Event::Privacy(PrivacyEvent::ReportUpdated(mine(&updated))),
    ));
    super::deliver(&mut core, &mut c, out).await?;
    Ok(http::json(updated))
}

/// `GET /api/v1/users/@me/reports`: your reports, newest first.
pub(crate) async fn mine_endpoint(
    State(app): State<App>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let rows = sqlx::query("SELECT * FROM reports WHERE reporter_id=? ORDER BY id DESC LIMIT ?")
        .bind(id(u)?)
        .bind(MAX_REPORTS_PAGE as i64)
        .fetch_all(&mut *c)
        .await?;
    let mut data = vec![];
    for r in &rows {
        data.push(mine(&row(r)?));
    }
    Ok(http::json(Snapshot {
        data,
        cursor: core.cursor(&key, u),
    }))
}
/// `GET /api/v1/guilds/{id}/reports?status=open|closed&before=`: the
/// server's Reports queue, for its moderators. Never names the reporter.
pub(crate) async fn queue_endpoint(
    State(app): State<App>,
    Path(g): Path<String>,
    Query(q): Query<super::PageQuery>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let g = http::url_id(&g)?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    role(&mut c, u, g).await?;
    require(handler(&mut c, u, g).await?, ErrorCode::Forbidden)?;
    let open = match q.status.as_deref() {
        None | Some("open") => true,
        Some("closed") => false,
        _ => return Err(Error::new(ErrorCode::InvalidRequest)),
    };
    let before = opt(q.before)?.unwrap_or(i64::MAX);
    let rows = sqlx::query(if open {
        "SELECT * FROM reports WHERE queue='guild' AND guild_id=? AND status='open' AND id<? ORDER BY id DESC LIMIT ?"
    } else {
        "SELECT * FROM reports WHERE queue='guild' AND guild_id=? AND status<>'open' AND id<? ORDER BY id DESC LIMIT ?"
    })
    .bind(id(g)?)
    .bind(before)
    .bind(MAX_REPORTS_PAGE as i64 + 1)
    .fetch_all(&mut *c)
    .await?;
    let mut reports = vec![];
    for r in rows.iter().take(MAX_REPORTS_PAGE) {
        reports.push(row(r)?);
    }
    let next_before = (rows.len() > MAX_REPORTS_PAGE)
        .then(|| reports.last().map(|r| r.id))
        .flatten();
    Ok(http::json(Snapshot {
        data: ReportPage {
            reports,
            next_before,
        },
        cursor: core.cursor(&key, u),
    }))
}

use crate::admin::ReportAdmin;
/// The administrator's queue: `queue='admin'`, and reports whose server is gone.
pub(crate) async fn admin(c: &mut SqliteConnection, cmd: &ReportAdmin) -> Result<()> {
    match cmd {
        ReportAdmin::List { all } => {
            let rows = sqlx::query(if *all {
                "SELECT * FROM reports WHERE (queue='admin' OR guild_id IS NULL) ORDER BY id DESC LIMIT 200"
            } else {
                "SELECT * FROM reports WHERE (queue='admin' OR guild_id IS NULL) AND status='open' ORDER BY id DESC LIMIT 200"
            })
            .fetch_all(&mut *c)
            .await?;
            for r in &rows {
                let r = row(r)?;
                println!(
                    "{}\t{}\t{}\t{:?}\t{}",
                    r.id.0,
                    r.created_at,
                    enum_str(r.category),
                    r.status,
                    summary(&r.target, &r.evidence)
                );
            }
            Ok(())
        }
        ReportAdmin::Show { id: report } => {
            let (r, _) = admin_report(c, *report).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&r).map_err(|_| Error::new(ErrorCode::Internal))?
            );
            Ok(())
        }
        ReportAdmin::Dismiss { id: report } | ReportAdmin::DisableUser { id: report } => {
            let (r, target_user) = admin_report(c, *report).await?;
            require(r.status == ReportStatus::Open, ErrorCode::Conflict)?;
            let disable = matches!(cmd, ReportAdmin::DisableUser { .. });
            if disable {
                let v = target_user.ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
                sqlx::query("INSERT OR IGNORE INTO disabled_users VALUES(?)")
                    .bind(v)
                    .execute(&mut *c)
                    .await?;
                sqlx::query("DELETE FROM sessions WHERE user_id=?")
                    .bind(v)
                    .execute(&mut *c)
                    .await?;
            }
            sqlx::query("UPDATE reports SET status=?,action=?,resolved_at=? WHERE id=?")
                .bind(if disable { "actioned" } else { "dismissed" })
                .bind(if disable { "disable_user" } else { "dismiss" })
                .bind(now() as i64)
                .bind(*report as i64)
                .execute(&mut *c)
                .await?;
            println!("Done.");
            Ok(())
        }
    }
}
async fn admin_report(c: &mut SqliteConnection, report: u64) -> Result<(Report, Option<i64>)> {
    require(
        report > 0 && report <= i64::MAX as u64,
        ErrorCode::InvalidRequest,
    )?;
    let r = sqlx::query("SELECT * FROM reports WHERE id=? AND (queue='admin' OR guild_id IS NULL)")
        .bind(report as i64)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok((row(&r)?, r.get("target_user_id")))
}
