//! Admission: rules screening and membership applications (with interviews).
use super::*;
use crate::moderation::audit;

/// Decided applications are kept this long for the applicant and reviewers.
const DECIDED_KEEP_MS: u64 = 30 * 86_400_000;
/// A rejected applicant may apply again after this long.
const REAPPLY_MS: u64 = 86_400_000;

fn questions(list: Vec<ApplicationQuestion>) -> Result<Vec<ApplicationQuestion>> {
    require(
        list.len() <= MAX_APPLICATION_QUESTIONS,
        ErrorCode::LimitExceeded,
    )?;
    list.into_iter()
        .map(|q| {
            let prompt = label(&q.prompt, MAX_QUESTION_CHARS)?;
            let choices = if q.kind == QuestionKind::Choice {
                let choices = q
                    .choices
                    .iter()
                    .map(|v| label(v, MAX_CHOICE_CHARS))
                    .collect::<Result<Vec<_>>>()?;
                let unique: HashSet<&String> = choices.iter().collect();
                require(
                    (2..=MAX_CHOICES).contains(&choices.len()) && unique.len() == choices.len(),
                    ErrorCode::InvalidRequest,
                )?;
                choices
            } else {
                vec![]
            };
            Ok(ApplicationQuestion {
                kind: q.kind,
                prompt,
                required: q.required,
                choices,
            })
        })
        .collect()
}
/// Validate an admission document (also used for templates). Keeps the
/// version of `old`, raising it when the rules changed.
pub(super) fn validate(mut a: Admission, old: &Admission) -> Result<Admission> {
    a.description = text(&a.description, MAX_SCREENING_DESCRIPTION_CHARS, true)?;
    require(a.rules.len() <= MAX_RULES, ErrorCode::LimitExceeded)?;
    a.rules = a
        .rules
        .iter()
        .map(|r| text(r, MAX_RULE_CHARS, true))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|r| !r.is_empty())
        .collect();
    if a.screening && a.rules.is_empty() {
        return Err(invalid(
            "rules",
            "Add at least one rule to turn on rules screening.",
        ));
    }
    a.questions = questions(a.questions)?;
    let changed = a.rules != old.rules || a.description != old.description;
    a.rules_version = if changed || old.rules_version == 0 {
        old.rules_version + 1
    } else {
        old.rules_version
    };
    Ok(a)
}

pub(super) async fn set(
    c: &mut SqliteConnection,
    u: Id,
    a: Admission,
    reaccept: bool,
    out: &mut Deliveries,
) -> Result<CommunityResult> {
    let g = a.guild_id;
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    let old = admission_settings(c, g).await?;
    let a = validate(a, &old)?;
    save_admission(c, &a).await?;
    // Members whose rights change: everyone asked to accept again, or
    // everyone released when screening is turned off.
    let mut affected: Vec<Id> = vec![];
    if a.screening && reaccept && a.rules_version != old.rules_version {
        let exempt = with_right(c, g, Permissions::MANAGE_GUILD).await?;
        for v in guild_users(c, g).await? {
            if exempt.contains(&v) {
                continue;
            }
            let mut m = member_state(c, g, v).await?;
            if !m.rules_pending {
                m.rules_pending = true;
                store(c, v, &m).await?;
                affected.push(v);
            }
        }
    } else if !a.screening {
        affected = sqlx::query_scalar::<_, i64>(
            "SELECT user_id FROM community_members WHERE guild_id=? AND rules_pending=1",
        )
        .bind(id(g)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|v| Id(v as u64))
        .collect();
        sqlx::query("UPDATE community_members SET rules_pending=0 WHERE guild_id=?")
            .bind(id(g)?)
            .execute(&mut *c)
            .await?;
    }
    for v in affected {
        out.push((vec![v], Event::GuildUpsert(guild(c, v, g).await?)));
        let m = member_state(c, g, v).await?;
        me_event(out, v, &m);
    }
    let on = |b: bool| Some(if b { "On" } else { "Off" }.to_owned());
    let entry = audit::Entry::new(g, AuditAction::AdmissionUpdate, AuditTarget::Guild, Some(g))
        .named(guild_name(c, g).await?)
        .change("rules screening", on(old.screening), on(a.screening))
        .change(
            "rules",
            Some(old.rules.len().to_string()),
            Some(a.rules.len().to_string()),
        )
        .change("applications", on(old.applications), on(a.applications))
        .change(
            "questions",
            Some(old.questions.len().to_string()),
            Some(a.questions.len().to_string()),
        )
        .reason(
            (reaccept && a.rules_version != old.rules_version)
                .then(|| "Members must accept the rules again.".to_owned()),
        );
    audit::write(c, u, entry).await?;
    emit(
        out,
        guild_users(c, g).await?,
        CommunityEvent::Updated { guild_id: g },
    );
    Ok(CommunityResult::Admission(Box::new(a)))
}

pub(super) async fn accept(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    version: u32,
    out: &mut Deliveries,
) -> Result<CommunityResult> {
    role(c, u, g).await?;
    let a = admission_settings(c, g).await?;
    let mut m = member_state(c, g, u).await?;
    if !m.rules_pending {
        return Ok(CommunityResult::Me(Box::new(m)));
    }
    if version != a.rules_version {
        let mut e = Error::new(ErrorCode::Conflict);
        e.0.field = Some("rules".into());
        e.0.message = "The rules just changed. Read them again before accepting.".into();
        return Err(e);
    }
    m.rules_pending = false;
    m.rules_version = a.rules_version;
    store(c, u, &m).await?;
    onboarding::reach(c, g, u, "rules").await?;
    if !m.questions_pending {
        rights_changed(c, out, g, u).await?;
    }
    me_event(out, u, &m);
    Ok(CommunityResult::Me(Box::new(m)))
}

/// The applicant's view of their application, if they have one.
pub(super) async fn mine(c: &mut SqliteConnection, u: Id, g: Id) -> Result<Option<MyApplication>> {
    let r = sqlx::query("SELECT a.*,g.name FROM community_applications a JOIN guilds g ON g.id=a.guild_id WHERE a.guild_id=? AND a.user_id=?")
        .bind(id(g)?)
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(r.map(|r| my_row(&r)))
}
fn my_row(r: &sqlx::sqlite::SqliteRow) -> MyApplication {
    MyApplication {
        guild_id: Id(r.get::<i64, _>("guild_id") as u64),
        guild_name: r.get("name"),
        status: en(r.get("status")).unwrap_or(ApplicationStatus::Pending),
        reason: r.get("reason"),
        interview_channel_id: r
            .get::<Option<i64>, _>("interview_channel_id")
            .map(|v| Id(v as u64)),
        created_at: r.get::<i64, _>("created_at") as u64,
        decided_at: r.get::<Option<i64>, _>("decided_at").map(|v| v as u64),
    }
}
/// Everything the applicant's own applications page lists (newest first).
pub(super) async fn my_applications(c: &mut SqliteConnection, u: Id) -> Result<Vec<MyApplication>> {
    Ok(sqlx::query("SELECT a.*,g.name FROM community_applications a JOIN guilds g ON g.id=a.guild_id WHERE a.user_id=? ORDER BY a.created_at DESC LIMIT 50")
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?
        .iter()
        .map(my_row)
        .collect())
}
/// What joining `g` asks of `u`.
pub(super) async fn join_gate(c: &mut SqliteConnection, u: Id, g: Id) -> Result<JoinGate> {
    let a = admission_settings(c, g).await?;
    let r = sqlx::query("SELECT name,(SELECT COUNT(*) FROM guild_members WHERE guild_id=guilds.id) AS n,COALESCE((SELECT description FROM guild_profiles WHERE guild_id=guilds.id),'') AS d FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let screening = a.screening && !a.rules.is_empty();
    Ok(JoinGate {
        guild_id: g,
        name: r.get("name"),
        description: r.get("d"),
        member_count: r.get::<i64, _>("n") as u32,
        joined: role(c, u, g).await.is_ok(),
        screening,
        screening_description: if screening {
            a.description.clone()
        } else {
            String::new()
        },
        rules: if screening { a.rules } else { vec![] },
        applications: a.applications,
        questions: if a.applications { a.questions } else { vec![] },
        application: mine(c, u, g).await?,
    })
}
/// The server an invite code leads to, if the invite can still be used.
pub(super) async fn invite_guild(c: &mut SqliteConnection, code: &str) -> Result<Id> {
    // Invite codes and vanity codes (identity.rs).
    let found = crate::identity::find_invite(c, code).await?;
    let i = invite(
        c,
        Id(found.ok_or_else(|| Error::new(ErrorCode::InvalidInvite))? as u64),
    )
    .await?;
    require(i.expires_at > now(), ErrorCode::InviteExpired)?;
    require(i.uses < i.max_uses, ErrorCode::InviteExhausted)?;
    Ok(i.guild_id)
}
/// Who may join or apply at all: not banned, invites not paused, and old
/// enough for the server's verification level.
pub(super) async fn eligible(c: &mut SqliteConnection, g: Id, u: Id) -> Result<()> {
    let banned: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_bans WHERE guild_id=? AND user_id=?")
            .bind(id(g)?)
            .bind(id(u)?)
            .fetch_one(&mut *c)
            .await?;
    require(banned == 0, ErrorCode::Banned)?;
    let safety = crate::moderation::safety(c, g).await?;
    if safety.invites_paused_until.is_some() {
        return Err(denied(
            "invites_paused",
            "This server has paused new members joining. Try again later.",
        ));
    }
    if safety.verification_level != VerificationLevel::None {
        let created: i64 = sqlx::query_scalar("SELECT created_at FROM users WHERE id=?")
            .bind(id(u)?)
            .fetch_one(&mut *c)
            .await?;
        let ready = (created.max(0) as u64).saturating_add(ACCOUNT_AGE_MS);
        if ready > now() {
            let minutes = ready.saturating_sub(now()).div_ceil(60_000);
            return Err(denied(
                "verification",
                format!("This server asks for accounts at least 5 minutes old. You can apply in {minutes} minute{}.", if minutes == 1 { "" } else { "s" }),
            ));
        }
    }
    Ok(())
}
fn answers(qs: &[ApplicationQuestion], list: Vec<String>) -> Result<Vec<String>> {
    require(list.len() == qs.len(), ErrorCode::InvalidRequest)?;
    qs.iter()
        .zip(list)
        .map(|(q, a)| {
            let a = match q.kind {
                QuestionKind::Short => text(&a, MAX_SHORT_ANSWER_CHARS, false)?,
                QuestionKind::Paragraph => text(&a, MAX_PARAGRAPH_ANSWER_CHARS, true)?,
                QuestionKind::Choice => {
                    let a = a.trim().to_owned();
                    require(
                        a.is_empty() || q.choices.contains(&a),
                        ErrorCode::InvalidRequest,
                    )?;
                    a
                }
            };
            if q.required && a.is_empty() {
                return Err(invalid(
                    "answers",
                    format!("Answer “{}” to apply.", q.prompt),
                ));
            }
            Ok(a)
        })
        .collect()
}
async fn queue_event(c: &mut SqliteConnection, out: &mut Deliveries, g: Id) -> Result<()> {
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM community_applications WHERE guild_id=? AND status='pending'",
    )
    .bind(id(g)?)
    .fetch_one(&mut *c)
    .await?;
    emit(
        out,
        with_right(c, g, Permissions::KICK_MEMBERS).await?,
        CommunityEvent::Queue {
            guild_id: g,
            pending: pending as u32,
        },
    );
    Ok(())
}
async fn applicant_event(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    g: Id,
    v: Id,
) -> Result<MyApplication> {
    let my = mine(c, v, g)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    emit(
        out,
        vec![v],
        CommunityEvent::Application(Box::new(my.clone())),
    );
    Ok(my)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn apply(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    via: JoinVia,
    list: Vec<String>,
    accept_rules: bool,
    out: &mut Deliveries,
) -> Result<CommunityResult> {
    let (g, source, code) = match via {
        JoinVia::Invite { code } => (
            invite_guild(c, &code).await?,
            JoinSource::Invite,
            Some(code),
        ),
        JoinVia::Discovery { guild_id } => {
            directory::joinable(c, guild_id).await?;
            (guild_id, JoinSource::Discovery, None)
        }
    };
    require(role(c, u, g).await.is_err(), ErrorCode::AlreadyExists)?;
    eligible(c, g, u).await?;
    let a = admission_settings(c, g).await?;
    if !a.applications {
        return Err(invalid(
            "application",
            "This server does not take applications. Join it instead.",
        ));
    }
    core.rate(format!("apply:{}", u.0), 2., 5.)?;
    let existing = sqlx::query(
        "SELECT status,decided_at FROM community_applications WHERE guild_id=? AND user_id=?",
    )
    .bind(id(g)?)
    .bind(id(u)?)
    .fetch_optional(&mut *c)
    .await?;
    if let Some(r) = existing {
        let status: ApplicationStatus = en(r.get("status"))?;
        require(
            status != ApplicationStatus::Pending,
            ErrorCode::AlreadyExists,
        )?;
        let decided = r.get::<Option<i64>, _>("decided_at").unwrap_or(0).max(0) as u64;
        if status == ApplicationStatus::Rejected && decided + REAPPLY_MS > now() {
            return Err(denied(
                "application",
                "Your application was declined. You can apply again tomorrow.",
            ));
        }
    }
    let list = answers(&a.questions, list)?;
    let screening = a.screening && !a.rules.is_empty();
    if screening && !accept_rules {
        return Err(invalid("rules", "Accept the server rules to apply."));
    }
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM community_applications WHERE guild_id=? AND status='pending'",
    )
    .bind(id(g)?)
    .fetch_one(&mut *c)
    .await?;
    if pending as usize >= MAX_PENDING_APPLICATIONS {
        let mut e = Error::new(ErrorCode::LimitExceeded);
        e.0.message = "This server has too many applications waiting. Try again later.".into();
        return Err(e);
    }
    sqlx::query("DELETE FROM community_applications WHERE guild_id=? AND status<>'pending' AND decided_at<?")
        .bind(id(g)?)
        .bind(now().saturating_sub(DECIDED_KEEP_MS) as i64)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT OR REPLACE INTO community_applications(guild_id,user_id,status,questions,answers,rules_accepted,source,invite_code,created_at) VALUES(?,?,'pending',?,?,?,?,?,?)")
        .bind(id(g)?)
        .bind(id(u)?)
        .bind(json(&a.questions)?)
        .bind(json(&list)?)
        .bind(screening && accept_rules)
        .bind(enum_str(source))
        .bind(code)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    queue_event(c, out, g).await?;
    let my = applicant_event(c, out, g, u).await?;
    Ok(CommunityResult::Application(Box::new(my)))
}

pub(super) async fn withdraw(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    out: &mut Deliveries,
) -> Result<CommunityResult> {
    let r = sqlx::query("UPDATE community_applications SET status='withdrawn',decided_at=? WHERE guild_id=? AND user_id=? AND status='pending'")
        .bind(now() as i64)
        .bind(id(g)?)
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    require(r.rows_affected() == 1, ErrorCode::NotFound)?;
    queue_event(c, out, g).await?;
    let my = applicant_event(c, out, g, u).await?;
    Ok(CommunityResult::Application(Box::new(my)))
}

pub(super) async fn review(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    v: Id,
    approve: bool,
    reason: Option<String>,
    out: &mut Deliveries,
) -> Result<CommunityResult> {
    guild_perm(c, u, g, Permissions::KICK_MEMBERS).await?;
    let r = sqlx::query(
        "SELECT * FROM community_applications WHERE guild_id=? AND user_id=? AND status='pending'",
    )
    .bind(id(g)?)
    .bind(id(v)?)
    .fetch_optional(&mut *c)
    .await?
    .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let reason = reason
        .map(|r| text(&r, MAX_REASON_CHARS, true))
        .transpose()?
        .filter(|r| !r.is_empty());
    let applicant = user(c, v).await?;
    if approve && role(c, v, g).await.is_err() {
        let banned: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM guild_bans WHERE guild_id=? AND user_id=?")
                .bind(id(g)?)
                .bind(id(v)?)
                .fetch_one(&mut *c)
                .await?;
        require(banned == 0, ErrorCode::Banned)?;
        guild_quota(c, v).await?;
        require(
            guild_users(c, g).await?.len() < MAX_GUILD_MEMBERS,
            ErrorCode::LimitExceeded,
        )?;
        sqlx::query("INSERT INTO guild_members VALUES(?,?,'member',?)")
            .bind(id(g)?)
            .bind(id(v)?)
            .bind(now() as i64)
            .execute(&mut *c)
            .await?;
        let source = en(r.get("source")).unwrap_or(JoinSource::Other);
        let code: Option<String> = r.get("invite_code");
        let accepted = r.get::<i64, _>("rules_accepted") != 0;
        admit(c, g, v, source, code.as_deref(), accepted).await?;
        crate::commands::joined(c, out, g, v).await?;
    }
    sqlx::query("UPDATE community_applications SET status=?,reason=?,reviewer_id=?,decided_at=? WHERE guild_id=? AND user_id=?")
        .bind(if approve { "approved" } else { "rejected" })
        .bind(&reason)
        .bind(id(u)?)
        .bind(now() as i64)
        .bind(id(g)?)
        .bind(id(v)?)
        .execute(&mut *c)
        .await?;
    let entry = audit::Entry::new(
        g,
        if approve {
            AuditAction::ApplicationApprove
        } else {
            AuditAction::ApplicationReject
        },
        AuditTarget::User,
        Some(v),
    )
    .named(applicant.username)
    .reason(reason);
    audit::write(c, u, entry).await?;
    queue_event(c, out, g).await?;
    applicant_event(c, out, g, v).await?;
    Ok(CommunityResult::Ack)
}

/// A private group conversation between the applicant and reviewers: made
/// once, and every later reviewer who opens it is added to it.
pub(super) async fn interview(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    g: Id,
    v: Id,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    guild_perm(c, u, g, Permissions::KICK_MEMBERS).await?;
    require(u != v, ErrorCode::InvalidRequest)?;
    let existing: Option<Option<i64>> = sqlx::query_scalar("SELECT interview_channel_id FROM community_applications WHERE guild_id=? AND user_id=? AND status='pending'")
        .bind(id(g)?)
        .bind(id(v)?)
        .fetch_optional(&mut *c)
        .await?;
    let existing = existing.ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    require(
        !blocked(c, u, v).await? && !blocked(c, v, u).await?,
        ErrorCode::Blocked,
    )?;
    let ch = match existing.map(|v| Id(v as u64)) {
        Some(ch) => {
            let members: Vec<i64> =
                sqlx::query_scalar("SELECT user_id FROM dm_members WHERE channel_id=?")
                    .bind(id(ch)?)
                    .fetch_all(&mut *c)
                    .await?;
            if members.contains(&id(u)?) {
                sqlx::query("UPDATE dm_members SET visible=1 WHERE channel_id=? AND user_id=?")
                    .bind(id(ch)?)
                    .bind(id(u)?)
                    .execute(&mut *c)
                    .await?;
            } else {
                require(
                    members.len() < MAX_GROUP_PARTICIPANTS,
                    ErrorCode::LimitExceeded,
                )?;
                dm_quota(c, u, None).await?;
                sqlx::query("INSERT INTO dm_members VALUES(?,?,?,1)")
                    .bind(id(ch)?)
                    .bind(id(u)?)
                    .bind(now() as i64)
                    .execute(&mut *c)
                    .await?;
                init_read(c, u, ch, true).await?;
            }
            ch
        }
        None => {
            dm_quota(c, u, None).await?;
            dm_quota(c, v, None).await?;
            let mut title = format!("Interview · {}", guild_name(c, g).await?);
            while title.len() > 64 {
                title.pop();
            }
            let ch = core.id().await?;
            sqlx::query("INSERT INTO channels(id,kind,name,position,owner_id,created_at) VALUES(?,'group_dm',?,0,?,?)")
                .bind(id(ch)?)
                .bind(title.trim())
                .bind(id(u)?)
                .bind(now() as i64)
                .execute(&mut *c)
                .await?;
            for w in [u, v] {
                sqlx::query("INSERT INTO dm_members VALUES(?,?,?,1)")
                    .bind(id(ch)?)
                    .bind(id(w)?)
                    .bind(now() as i64)
                    .execute(&mut *c)
                    .await?;
                init_read(c, w, ch, true).await?;
            }
            sqlx::query("UPDATE community_applications SET interview_channel_id=? WHERE guild_id=? AND user_id=?")
                .bind(id(ch)?)
                .bind(id(g)?)
                .bind(id(v)?)
                .execute(&mut *c)
                .await?;
            ch
        }
    };
    let channel = channel(c, ch).await?;
    out.push((
        channel.summary.recipient_ids.clone(),
        Event::ChannelUpsert(channel.clone()),
    ));
    for w in &channel.summary.recipient_ids {
        out.push((vec![*w], Event::ReadStateUpdated(read(c, *w, ch).await?)));
    }
    applicant_event(c, out, g, v).await?;
    Ok(CommandResult::Channel(channel))
}

/// The review queue: every waiting application (oldest first) and the latest
/// decided ones.
pub(super) async fn queue(c: &mut SqliteConnection, g: Id) -> Result<ApplicationQueue> {
    let mut q = ApplicationQueue::default();
    for (status, list) in [("pending", &mut q.pending), ("decided", &mut q.decided)] {
        let sql = if status == "pending" {
            "SELECT * FROM community_applications WHERE guild_id=? AND status='pending' ORDER BY created_at LIMIT 500"
        } else {
            "SELECT * FROM community_applications WHERE guild_id=? AND status<>'pending' ORDER BY decided_at DESC LIMIT 50"
        };
        for r in sqlx::query(sql).bind(id(g)?).fetch_all(&mut *c).await? {
            let v = Id(r.get::<i64, _>("user_id") as u64);
            let created: i64 = sqlx::query_scalar("SELECT created_at FROM users WHERE id=?")
                .bind(id(v)?)
                .fetch_one(&mut *c)
                .await?;
            list.push(Application {
                guild_id: g,
                user: user(c, v).await?,
                account_created_at: created.max(0) as u64,
                answers: parse(r.get("answers")),
                questions: parse(r.get("questions")),
                status: en(r.get("status"))?,
                reason: r.get("reason"),
                reviewer_id: r.get::<Option<i64>, _>("reviewer_id").map(|v| Id(v as u64)),
                interview_channel_id: r
                    .get::<Option<i64>, _>("interview_channel_id")
                    .map(|v| Id(v as u64)),
                source: en(r.get("source")).unwrap_or(JoinSource::Other),
                created_at: r.get::<i64, _>("created_at") as u64,
                decided_at: r.get::<Option<i64>, _>("decided_at").map(|v| v as u64),
            });
        }
    }
    Ok(q)
}
