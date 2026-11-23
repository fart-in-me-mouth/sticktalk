//! Community: every command, route, permission check and limit.
use super::*;
use crate::{audit_tests::fixture, commands, http, state::App};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use base64::Engine as _;
use tower::ServiceExt;

static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
async fn run(app: &App, u: u64, command: Command) -> CommandOutcome {
    let mut core = app.0.core.lock().await;
    core.limits.clear();
    let nonce = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    commands::execute(
        app,
        &mut core,
        &[u as u8; 32],
        Id(u),
        CommandRequest { nonce, command },
    )
    .await
    .unwrap()
    .outcome
}
async fn cm(app: &App, u: u64, cmd: CommunityCommand) -> CommandOutcome {
    run(app, u, Command::Community(cmd)).await
}
#[track_caller]
fn ok(o: CommandOutcome) -> CommandResult {
    match o {
        CommandOutcome::Ok(r) => r,
        CommandOutcome::Err(e) => panic!("expected success, got {e:?}"),
    }
}
#[track_caller]
fn err(o: CommandOutcome) -> ApiError {
    match o {
        CommandOutcome::Err(e) => e,
        o => panic!("expected an error, got {o:?}"),
    }
}
fn send(ch: u64, content: &str) -> Command {
    Command::SendMessage(MessageDraft {
        channel_id: Id(ch),
        content: content.into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    })
}
async fn sql(app: &App, q: &str) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(q.to_owned()))
        .execute(&app.0.pool)
        .await
        .unwrap();
}
async fn scalar(app: &App, q: &'static str) -> i64 {
    sqlx::query_scalar(q).fetch_one(&app.0.pool).await.unwrap()
}
async fn token(app: &App, u: u8) -> String {
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([u + 100; 32]);
    sqlx::query("INSERT OR IGNORE INTO sessions VALUES(?,?,?,?,?)")
        .bind(crate::state::hash(token.as_bytes()))
        .bind(i64::from(u))
        .bind(now() as i64)
        .bind((now() + 600_000) as i64)
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    token
}
async fn get<T: serde::de::DeserializeOwned>(
    app: &App,
    u: u8,
    path: &str,
) -> std::result::Result<T, StatusCode> {
    let token = token(app, u).await;
    app.0.core.lock().await.limits.clear();
    let r = http::router(app.clone())
        .oneshot(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    if r.status() != StatusCode::OK {
        return Err(r.status());
    }
    Ok(serde_json::from_slice(&to_bytes(r.into_body(), 8 << 20).await.unwrap()).unwrap())
}
async fn invite(app: &App) -> String {
    match ok(run(
        app,
        1,
        Command::CreateInvite {
            guild_id: Id(10),
            ttl_seconds: 3600,
            max_uses: 100,
        },
    )
    .await)
    {
        CommandResult::Invite(i) => i.code,
        r => panic!("{r:?}"),
    }
}
async fn join(app: &App, u: u64, code: &str) -> CommandOutcome {
    run(
        app,
        u,
        Command::JoinGuild {
            invite_code: code.into(),
        },
    )
    .await
}
fn admission(
    screening: bool,
    rules: &[&str],
    applications: bool,
    questions: Vec<ApplicationQuestion>,
) -> CommunityCommand {
    CommunityCommand::SetAdmission {
        guild_id: Id(10),
        screening,
        description: "Read these".into(),
        rules: rules.iter().map(|r| (*r).into()).collect(),
        reaccept: false,
        applications,
        questions,
    }
}
async fn rights(app: &App, u: u64, ch: Option<u64>) -> Permissions {
    let mut c = app.0.pool.acquire().await.unwrap();
    acl::evaluate(&mut c, Id(u), Id(10), ch.map(Id))
        .await
        .unwrap()
}
async fn me(app: &App, u: u64) -> MemberCommunity {
    let mut c = app.0.pool.acquire().await.unwrap();
    member_state(&mut c, Id(10), Id(u)).await.unwrap()
}
/// Members 2..: role `rid` with `p` at `position`.
async fn give(app: &App, u: u64, rid: u64, p: Permissions, position: u16) {
    sql(
        app,
        &format!(
            "INSERT OR REPLACE INTO roles(guild_id,id,name,position,permissions) VALUES(10,{rid},'r{rid}',{position},{});
             INSERT OR IGNORE INTO member_roles VALUES(10,{u},{rid});",
            p.bits() as i64
        ),
    )
    .await;
}

#[tokio::test]
async fn screening_restricts_new_members_until_they_accept() {
    let (_d, app) = fixture().await;
    // Only managers configure admission.
    let e = err(cm(&app, 2, admission(true, &["Be kind"], false, vec![])).await);
    assert_eq!(e.code, ErrorCode::Forbidden);
    let e = err(cm(&app, 1, admission(true, &[], false, vec![])).await);
    assert_eq!(e.field.as_deref(), Some("rules"));
    let e = err(cm(&app, 1, admission(true, &["x"; 17], false, vec![])).await);
    assert_eq!(e.code, ErrorCode::LimitExceeded);
    ok(cm(
        &app,
        1,
        admission(true, &["Be kind", "No spam"], false, vec![]),
    )
    .await);
    let message = match ok(run(&app, 1, send(11, "hello")).await) {
        CommandResult::Message(m) => m.id,
        r => panic!("{r:?}"),
    };
    let code = invite(&app).await;
    ok(join(&app, 3, &code).await);
    assert!(me(&app, 3).await.rules_pending);
    // Read, but not talk, react or join voice: as a timeout.
    assert_eq!(
        rights(&app, 3, Some(11)).await,
        crate::moderation::TIMED_OUT_KEEPS
    );
    let e = err(run(&app, 3, send(11, "hi")).await);
    assert_eq!(e.field.as_deref(), Some("screening"));
    let e = err(run(
        &app,
        3,
        Command::AddReaction {
            channel_id: Id(11),
            message_id: message,
            emoji: "👍".into(),
        },
    )
    .await);
    assert_eq!(e.code, ErrorCode::Forbidden);
    // The batched evaluator agrees: the channel is listed, but 3 cannot send.
    {
        let mut c = app.0.pool.acquire().await.unwrap();
        assert!(channels(&mut c, Id(3), Some(Id(10)))
            .await
            .unwrap()
            .iter()
            .any(|ch| ch.id == Id(11)));
        let senders =
            crate::capacity::members_with(&mut c, Id(10), Some(Id(11)), Permissions::SEND_MESSAGES)
                .await
                .unwrap();
        assert!(!senders.contains(&Id(3)) && senders.contains(&Id(2)));
    }
    let s: CommunityState = get(&app, 3, "/api/v1/guilds/10/community").await.unwrap();
    assert!(s.me.rules_pending && s.admission.rules.len() == 2);
    let e = err(cm(
        &app,
        3,
        CommunityCommand::AcceptRules {
            guild_id: Id(10),
            rules_version: 99,
        },
    )
    .await);
    assert_eq!(e.code, ErrorCode::Conflict);
    ok(cm(
        &app,
        3,
        CommunityCommand::AcceptRules {
            guild_id: Id(10),
            rules_version: s.admission.rules_version,
        },
    )
    .await);
    ok(run(&app, 3, send(11, "hi")).await);
    // Changed rules, accepted again by everyone but managers.
    let mut again = admission(true, &["Be kind", "No spam", "Have fun"], false, vec![]);
    if let CommunityCommand::SetAdmission { reaccept, .. } = &mut again {
        *reaccept = true;
    }
    ok(cm(&app, 1, again).await);
    assert!(me(&app, 3).await.rules_pending && me(&app, 2).await.rules_pending);
    assert!(!me(&app, 1).await.rules_pending);
    ok(run(&app, 1, send(11, "owner still talks")).await);
    // Turning screening off releases everyone.
    ok(cm(&app, 1, admission(false, &["Be kind"], false, vec![])).await);
    ok(run(&app, 2, send(11, "free")).await);
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM audit_log WHERE action='admission_update'"
        )
        .await,
        3
    );
}

#[tokio::test]
async fn pending_members_cannot_message_other_members() {
    let (_d, app) = fixture().await;
    sql(&app, "INSERT INTO channels(id,kind,position,created_at) VALUES(30,'direct',0,0); INSERT INTO dm_pairs VALUES(2,3,30); INSERT INTO dm_members VALUES(30,2,0,1),(30,3,0,1); INSERT INTO read_states(user_id,channel_id) VALUES(2,30),(3,30);").await;
    ok(cm(&app, 1, admission(true, &["Be kind"], false, vec![])).await);
    let code = invite(&app).await;
    ok(join(&app, 3, &code).await);
    let e = err(run(&app, 3, send(30, "psst")).await);
    assert_eq!(e.field.as_deref(), Some("screening"));
}

#[tokio::test]
async fn applications_are_reviewed_and_audited() {
    let (_d, app) = fixture().await;
    sql(&app, "INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(5,'user5','x','online',0); INSERT INTO sessions VALUES(x'0505050505050505050505050505050505050505050505050505050505050505',5,0,99999999999999,0);").await;
    let questions = vec![
        ApplicationQuestion {
            kind: QuestionKind::Short,
            prompt: "Why do you want to join?".into(),
            required: true,
            choices: vec![],
        },
        ApplicationQuestion {
            kind: QuestionKind::Choice,
            prompt: "Pick one".into(),
            required: false,
            choices: vec!["A".into(), "B".into()],
        },
    ];
    let bad = vec![ApplicationQuestion {
        kind: QuestionKind::Choice,
        prompt: "One choice".into(),
        required: false,
        choices: vec!["only".into()],
    }];
    assert_eq!(
        err(cm(&app, 1, admission(false, &[], true, bad)).await).code,
        ErrorCode::InvalidRequest
    );
    ok(cm(&app, 1, admission(true, &["Be kind"], true, questions)).await);
    let code = invite(&app).await;
    let e = err(join(&app, 3, &code).await);
    assert_eq!(e.field.as_deref(), Some("application"));
    let apply = |answers: &[&str], rules: bool| CommunityCommand::Apply {
        via: JoinVia::Invite { code: code.clone() },
        answers: answers.iter().map(|a| (*a).into()).collect(),
        accept_rules: rules,
    };
    assert_eq!(
        err(cm(&app, 3, apply(&["", "A"], true)).await)
            .field
            .as_deref(),
        Some("answers")
    );
    assert_eq!(
        err(cm(&app, 3, apply(&["Fun", "C"], true)).await).code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        err(cm(&app, 3, apply(&["Fun", "A"], false)).await)
            .field
            .as_deref(),
        Some("rules")
    );
    ok(cm(&app, 3, apply(&["Fun", "A"], true)).await);
    assert_eq!(
        err(cm(&app, 3, apply(&["Fun", "A"], true)).await).code,
        ErrorCode::AlreadyExists
    );
    let gate: JoinGate = get(&app, 3, &format!("/api/v1/invites/{code}/admission"))
        .await
        .unwrap();
    assert!(gate.applications && gate.screening);
    assert_eq!(gate.application.unwrap().status, ApplicationStatus::Pending);
    // Reviewing needs Kick Members.
    let review = |user: u64, approve: bool| CommunityCommand::Review {
        guild_id: Id(10),
        user_id: Id(user),
        approve,
        reason: Some("Thanks".into()),
    };
    assert_eq!(
        err(cm(&app, 2, review(3, true)).await).code,
        ErrorCode::Forbidden
    );
    assert_eq!(
        get::<ApplicationQueue>(&app, 2, "/api/v1/guilds/10/applications")
            .await
            .unwrap_err(),
        StatusCode::FORBIDDEN
    );
    give(&app, 2, 30, Permissions::KICK_MEMBERS, 1).await;
    let q: ApplicationQueue = get(&app, 2, "/api/v1/guilds/10/applications")
        .await
        .unwrap();
    assert_eq!(q.pending.len(), 1);
    assert_eq!(q.pending[0].answers, vec!["Fun".to_owned(), "A".to_owned()]);
    // An interview: one group conversation, applicant and reviewers.
    let ch = match ok(cm(
        &app,
        2,
        CommunityCommand::Interview {
            guild_id: Id(10),
            user_id: Id(3),
        },
    )
    .await)
    {
        CommandResult::Channel(ch) => ch,
        r => panic!("{r:?}"),
    };
    assert_eq!(ch.summary.kind, ChannelKind::GroupDm);
    assert_eq!(ch.summary.recipient_ids, vec![Id(2), Id(3)]);
    let again = match ok(cm(
        &app,
        1,
        CommunityCommand::Interview {
            guild_id: Id(10),
            user_id: Id(3),
        },
    )
    .await)
    {
        CommandResult::Channel(ch) => ch,
        r => panic!("{r:?}"),
    };
    assert_eq!(again.summary.id, ch.summary.id);
    assert_eq!(again.summary.recipient_ids, vec![Id(1), Id(2), Id(3)]);
    // Approval adds the member, with the rules already accepted.
    ok(cm(&app, 2, review(3, true)).await);
    assert!(!me(&app, 3).await.rules_pending);
    ok(run(&app, 3, send(11, "I'm in")).await);
    assert_eq!(scalar(&app, "SELECT COUNT(*) FROM audit_log WHERE action='application_approve' AND actor_id=2 AND target_id=3").await, 1);
    // Rejection, with a reason, and no new application for a day.
    ok(cm(&app, 4, apply(&["Hi", ""], true)).await);
    ok(cm(&app, 2, review(4, false)).await);
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM audit_log WHERE action='application_reject' AND reason='Thanks'"
        )
        .await,
        1
    );
    let mine: Vec<MyApplication> = get(&app, 4, "/api/v1/users/@me/applications")
        .await
        .unwrap();
    assert_eq!(mine[0].status, ApplicationStatus::Rejected);
    assert_eq!(
        err(cm(&app, 4, apply(&["Hi", ""], true)).await)
            .field
            .as_deref(),
        Some("application")
    );
    // Withdrawing.
    sql(&app, "DELETE FROM community_applications WHERE user_id=4").await;
    ok(cm(&app, 4, apply(&["Hi", ""], true)).await);
    ok(cm(&app, 4, CommunityCommand::Withdraw { guild_id: Id(10) }).await);
    assert_eq!(
        err(cm(&app, 4, CommunityCommand::Withdraw { guild_id: Id(10) }).await).code,
        ErrorCode::NotFound
    );
    // Banned people cannot apply.
    sql(&app, "INSERT INTO guild_bans VALUES(10,5,1,NULL,0)").await;
    assert_eq!(
        err(cm(&app, 5, apply(&["Hi", ""], true)).await).code,
        ErrorCode::Banned
    );
    // Accounts too new for the verification level cannot apply either.
    sql(&app, "DELETE FROM guild_bans; DELETE FROM community_applications WHERE user_id=4; INSERT INTO guild_safety(guild_id,verification_level) VALUES(10,'account_age')").await;
    assert_eq!(
        err(cm(&app, 4, apply(&["Hi", ""], true)).await)
            .field
            .as_deref(),
        Some("verification")
    );
    sql(&app, "UPDATE users SET created_at=0").await;
    // The queue is bounded.
    sql(&app, "WITH RECURSIVE n(i) AS (SELECT 1000 UNION ALL SELECT i+1 FROM n WHERE i<1499) INSERT INTO users(id,username,password_hash,desired_presence,created_at) SELECT i,'bulk'||i,'x','online',0 FROM n;").await;
    sql(&app, "INSERT INTO community_applications(guild_id,user_id,status,questions,answers,source,created_at) SELECT 10,id,'pending','[]','[]','invite',0 FROM users WHERE id>=1000").await;
    assert_eq!(
        err(cm(&app, 4, apply(&["Hi", ""], true)).await).code,
        ErrorCode::LimitExceeded
    );
}

fn question(
    id: u32,
    pre_join: bool,
    required: bool,
    answers: Vec<OnboardingAnswer>,
) -> OnboardingQuestion {
    OnboardingQuestion {
        id,
        title: format!("Question {id}"),
        multi: false,
        required,
        pre_join,
        answers,
    }
}
fn answer(id: u32, roles: &[u64], channels: &[u64]) -> OnboardingAnswer {
    OnboardingAnswer {
        id,
        label: format!("Answer {id}"),
        emoji: Some("🎮".into()),
        role_ids: roles.iter().map(|r| Id(*r)).collect(),
        channel_ids: channels.iter().map(|c| Id(*c)).collect(),
    }
}
fn onboarding(questions: Vec<OnboardingQuestion>, opt_in: &[u64]) -> CommunityCommand {
    CommunityCommand::SetOnboarding(Onboarding {
        guild_id: Id(10),
        enabled: true,
        questions,
        opt_in_channel_ids: opt_in.iter().map(|c| Id(*c)).collect(),
        welcome: WelcomeScreen {
            description: "Welcome!".into(),
            highlights: vec![Highlight {
                channel_id: Id(11),
                text: "Say hello".into(),
                emoji: None,
            }],
        },
        tasks: vec![],
        resources: vec![],
    })
}
fn respond(answers: &[(u32, u32)]) -> CommunityCommand {
    CommunityCommand::Respond {
        guild_id: Id(10),
        responses: answers
            .iter()
            .map(|(q, a)| OnboardingResponse {
                question_id: *q,
                answer_ids: vec![*a],
            })
            .collect(),
        finish: true,
    }
}
async fn roles_of(app: &App, u: u64) -> Vec<Id> {
    let mut c = app.0.pool.acquire().await.unwrap();
    acl::role_ids(&mut c, Id(10), Id(u)).await.unwrap()
}

#[tokio::test]
async fn onboarding_grants_safe_roles_and_channels() {
    let (_d, app) = fixture().await;
    sql(&app, "INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'text',10,'games',1,0),(13,'text',10,'art',2,0);
        INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,40,'Gamer',2,0),(10,41,'Mods',3,512),(10,42,'Top',9,0);").await;
    // 2 manages the server and roles, with a top role at position 5.
    give(
        &app,
        2,
        50,
        Permissions::MANAGE_GUILD | Permissions::MANAGE_ROLES,
        5,
    )
    .await;
    let flow = |role: u64| {
        onboarding(
            vec![question(
                1,
                true,
                true,
                vec![answer(1, &[role], &[12]), answer(2, &[], &[13])],
            )],
            &[12, 13],
        )
    };
    // Never a dangerous right, never above the configuring person's top role.
    assert_eq!(
        err(cm(&app, 2, flow(41)).await).field.as_deref(),
        Some("roles")
    );
    assert_eq!(
        err(cm(&app, 2, flow(42)).await).field.as_deref(),
        Some("roles")
    );
    assert_eq!(
        err(cm(&app, 2, flow(10)).await).field.as_deref(),
        Some("roles")
    );
    assert_eq!(err(cm(&app, 4, flow(40)).await).code, ErrorCode::NotFound);
    sql(&app, "INSERT INTO guild_members VALUES(10,4,'member',0); INSERT INTO read_states(user_id,channel_id) VALUES(4,11)").await;
    assert_eq!(err(cm(&app, 4, flow(40)).await).code, ErrorCode::Forbidden);
    // Granting roles also needs Manage Roles.
    give(&app, 4, 51, Permissions::MANAGE_GUILD, 4).await;
    assert_eq!(err(cm(&app, 4, flow(40)).await).code, ErrorCode::Forbidden);
    // Too many pre-join questions.
    let many = (1..=6)
        .map(|i| question(i, true, false, vec![answer(1, &[], &[])]))
        .collect();
    assert_eq!(
        err(cm(&app, 2, onboarding(many, &[])).await).code,
        ErrorCode::LimitExceeded
    );
    ok(cm(&app, 2, flow(40)).await);
    // A new member must answer the required pre-join question to talk.
    let code = invite(&app).await;
    ok(join(&app, 3, &code).await);
    assert!(me(&app, 3).await.questions_pending);
    assert_eq!(
        err(run(&app, 3, send(11, "hi")).await).field.as_deref(),
        Some("screening")
    );
    ok(cm(&app, 3, respond(&[(1, 1)])).await);
    let m = me(&app, 3).await;
    assert!(!m.questions_pending && m.onboarded && m.opted_in == vec![Id(12)]);
    assert!(roles_of(&app, 3).await.contains(&Id(40)));
    ok(run(&app, 3, send(11, "hi")).await);
    // Changing the answer takes the role and channel away again.
    ok(cm(&app, 3, respond(&[(1, 2)])).await);
    assert!(!roles_of(&app, 3).await.contains(&Id(40)));
    assert_eq!(me(&app, 3).await.opted_in, vec![Id(13)]);
    // Checked again when granting: a role that became dangerous is not given.
    sql(&app, "UPDATE roles SET permissions=512 WHERE id=40").await;
    ok(cm(&app, 3, respond(&[(1, 1)])).await);
    assert!(!roles_of(&app, 3).await.contains(&Id(40)));
    // ...nor one now above the configuring person.
    sql(
        &app,
        "UPDATE roles SET permissions=0,position=7 WHERE id=40",
    )
    .await;
    ok(cm(&app, 3, respond(&[(1, 2)])).await);
    ok(cm(&app, 3, respond(&[(1, 1)])).await);
    assert!(!roles_of(&app, 3).await.contains(&Id(40)));
    assert!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM audit_log WHERE action='member_roles_update' AND actor_id=3"
        )
        .await
            >= 2
    );
    // Required questions must be answered to finish; unknown answers are refused.
    assert_eq!(
        err(cm(
            &app,
            3,
            CommunityCommand::Respond {
                guild_id: Id(10),
                responses: vec![],
                finish: true
            }
        )
        .await)
        .field
        .as_deref(),
        Some("responses")
    );
    assert_eq!(
        err(cm(&app, 3, respond(&[(1, 9)])).await).code,
        ErrorCode::InvalidRequest
    );
    // Browse Channels: only opt-in channels, per member.
    let browse = |list: &[u64]| CommunityCommand::SetChannels {
        guild_id: Id(10),
        opted_in: list.iter().map(|c| Id(*c)).collect(),
        show_all: true,
    };
    assert_eq!(
        err(cm(&app, 3, browse(&[11])).await).code,
        ErrorCode::InvalidRequest
    );
    ok(cm(&app, 3, browse(&[12, 13])).await);
    let m = me(&app, 3).await;
    assert!(m.show_all && m.opted_in == vec![Id(12), Id(13)]);
    // Onboarding events reach the funnel.
    assert!(scalar(&app, "SELECT COALESCE(SUM(reached),0) FROM insight_onboarding WHERE guild_id=10 AND step='joined'").await >= 1);
}

#[tokio::test]
async fn server_guide_progress_is_tracked() {
    let (_d, app) = fixture().await;
    let mut cmd = onboarding(vec![], &[]);
    if let CommunityCommand::SetOnboarding(o) = &mut cmd {
        o.tasks = vec![
            GuideTask {
                id: 0,
                kind: GuideTaskKind::Read,
                channel_id: Id(11),
                title: "Read general".into(),
            },
            GuideTask {
                id: 0,
                kind: GuideTaskKind::Talk,
                channel_id: Id(11),
                title: "Say hello".into(),
            },
        ];
        o.resources = vec![GuideResource {
            channel_id: Id(11),
            title: "FAQ".into(),
            description: String::new(),
        }];
    }
    let o = match ok(cm(&app, 1, cmd).await) {
        CommandResult::Community(CommunityResult::Onboarding(o)) => o,
        r => panic!("{r:?}"),
    };
    let (read, talk) = (o.tasks[0].id, o.tasks[1].id);
    assert_ne!(read, talk);
    ok(run(&app, 2, send(11, "hello everyone")).await);
    assert!(me(&app, 2).await.guide_done.contains(&talk));
    assert_eq!(
        err(cm(
            &app,
            2,
            CommunityCommand::CompleteTask {
                guild_id: Id(10),
                task_id: talk
            }
        )
        .await)
        .code,
        ErrorCode::NotFound
    );
    ok(cm(
        &app,
        2,
        CommunityCommand::CompleteTask {
            guild_id: Id(10),
            task_id: read,
        },
    )
    .await);
    assert!(me(&app, 2).await.guide_done.contains(&read));
    assert_eq!(
        err(cm(
            &app,
            3,
            CommunityCommand::CompleteTask {
                guild_id: Id(10),
                task_id: read
            }
        )
        .await)
        .code,
        ErrorCode::NotFound
    );
}

fn draft() -> ListingDraft {
    ListingDraft {
        category: DirectoryCategory::Gaming,
        description: "A friendly guild".into(),
        tags: vec!["Rust".into(), "rust".into(), "games".into()],
        language: "en".into(),
        featured_emoji: vec![],
    }
}
async fn listed(app: &App, u: u8, q: &str) -> Vec<Id> {
    app.0.core.lock().await.community.directory = Default::default();
    let page: DirectoryPage = get(app, u, &format!("/api/v1/directory?{q}"))
        .await
        .unwrap();
    let list = if q.is_empty() {
        page.active
    } else {
        page.results
    };
    list.iter().map(|e| e.guild_id).collect()
}

#[tokio::test]
async fn directory_listing_requirements_search_and_admin() {
    let (_d, app) = fixture().await;
    sql(&app, "UPDATE users SET created_at=0").await;
    let set = |listing: Option<ListingDraft>| CommunityCommand::SetListing {
        guild_id: Id(10),
        listing,
    };
    assert_eq!(
        err(cm(&app, 1, set(Some(draft()))).await).field.as_deref(),
        Some("requirements")
    );
    ok(cm(&app, 1, admission(true, &["Be kind"], false, vec![])).await);
    ok(run(
        &app,
        1,
        Command::Launch(LaunchCommand::Moderation(
            ModerationCommand::SetVerificationLevel {
                guild_id: Id(10),
                level: VerificationLevel::AccountAge,
            },
        )),
    )
    .await);
    sql(&app, "INSERT INTO channel_age_restricted VALUES(11)").await;
    assert_eq!(
        err(cm(&app, 1, set(Some(draft()))).await).field.as_deref(),
        Some("requirements")
    );
    sql(&app, "DELETE FROM channel_age_restricted").await;
    assert_eq!(
        err(cm(&app, 2, set(Some(draft()))).await).code,
        ErrorCode::Forbidden
    );
    let mut long = draft();
    long.tags = vec!["a".into(); 6];
    assert_eq!(
        err(cm(&app, 1, set(Some(long))).await).code,
        ErrorCode::LimitExceeded
    );
    match ok(cm(&app, 1, set(Some(draft()))).await) {
        CommandResult::Community(CommunityResult::Listing(Some(l))) => {
            assert_eq!(l.draft.tags, vec!["rust".to_owned(), "games".to_owned()])
        }
        r => panic!("{r:?}"),
    }
    let admin: ListingAdmin = get(&app, 1, "/api/v1/guilds/10/listing").await.unwrap();
    assert!(admin.directory_enabled && admin.requirements.iter().all(|r| r.met));
    assert_eq!(
        get::<ListingAdmin>(&app, 2, "/api/v1/guilds/10/listing")
            .await
            .unwrap_err(),
        StatusCode::FORBIDDEN
    );
    // Browse and search as someone who is not a member.
    assert_eq!(listed(&app, 3, "").await, vec![Id(10)]);
    assert_eq!(listed(&app, 3, "q=gui").await, vec![Id(10)]);
    assert_eq!(listed(&app, 3, "q=rust").await, vec![Id(10)]);
    assert!(listed(&app, 3, "q=zzz").await.is_empty());
    assert!(listed(&app, 3, "category=music").await.is_empty());
    let preview: DirectoryPreview = get(&app, 3, "/api/v1/directory/10").await.unwrap();
    assert!(preview.gate.screening && !preview.entry.joined && preview.entry.member_count == 2);
    // Joining follows the server's admission: screening applies.
    ok(cm(
        &app,
        3,
        CommunityCommand::JoinDiscovered { guild_id: Id(10) },
    )
    .await);
    assert!(me(&app, 3).await.rules_pending);
    assert_eq!(
        scalar(
            &app,
            "SELECT joins FROM insight_sources WHERE source='discovery'"
        )
        .await,
        1
    );
    // The instance administrator hides, delists, or turns the directory off.
    let mut c = app.0.pool.acquire().await.unwrap();
    admin_cli(&mut c, &crate::admin::DirectoryAdmin::Hide { guild_id: 10 })
        .await
        .unwrap();
    assert!(listed(&app, 4, "").await.is_empty());
    assert_eq!(
        err(cm(
            &app,
            4,
            CommunityCommand::JoinDiscovered { guild_id: Id(10) }
        )
        .await)
        .code,
        ErrorCode::NotFound
    );
    admin_cli(
        &mut c,
        &crate::admin::DirectoryAdmin::Unhide { guild_id: 10 },
    )
    .await
    .unwrap();
    admin_cli(
        &mut c,
        &crate::admin::DirectoryAdmin::Feature { guild_id: 10 },
    )
    .await
    .unwrap();
    app.0.core.lock().await.community.directory = Default::default();
    let page: DirectoryPage = get(&app, 4, "/api/v1/directory").await.unwrap();
    assert_eq!(page.featured.len(), 1);
    admin_cli(
        &mut c,
        &crate::admin::DirectoryAdmin::Delist { guild_id: 10 },
    )
    .await
    .unwrap();
    assert_eq!(
        err(cm(&app, 1, set(Some(draft()))).await).field.as_deref(),
        Some("directory")
    );
    admin_cli(
        &mut c,
        &crate::admin::DirectoryAdmin::Relist { guild_id: 10 },
    )
    .await
    .unwrap();
    // Servers that take applications are applied to, not joined.
    ok(cm(&app, 1, admission(true, &["Be kind"], true, vec![])).await);
    assert_eq!(
        err(cm(
            &app,
            4,
            CommunityCommand::JoinDiscovered { guild_id: Id(10) }
        )
        .await)
        .field
        .as_deref(),
        Some("application")
    );
    ok(cm(
        &app,
        4,
        CommunityCommand::Apply {
            via: JoinVia::Discovery { guild_id: Id(10) },
            answers: vec![],
            accept_rules: true,
        },
    )
    .await);
    admin_cli(&mut c, &crate::admin::DirectoryAdmin::Disable)
        .await
        .unwrap();
    app.0.core.lock().await.community.directory = Default::default();
    let page: DirectoryPage = get(&app, 4, "/api/v1/directory").await.unwrap();
    assert!(!page.enabled && page.active.is_empty());
    assert_eq!(
        err(cm(&app, 1, set(Some(draft()))).await).field.as_deref(),
        Some("directory")
    );
    ok(cm(&app, 1, set(None)).await);
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM directory_listings").await,
        0
    );
}

#[tokio::test]
async fn templates_copy_structure_only_and_map_references() {
    let (_d, app) = fixture().await;
    sql(&app, "INSERT INTO roles(guild_id,id,name,color,position,permissions) VALUES(10,60,'Helper',1193046,1,32),(10,61,'Fan',0,2,0);
        INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(14,'category',10,'Info',1,0);
        INSERT INTO channels(id,kind,guild_id,parent_id,name,topic,position,created_at) VALUES(15,'text',10,14,'rules','Read me',0,0);
        INSERT INTO permission_overwrites VALUES(15,'role',60,2,0),(15,'role',10,0,2),(15,'member',2,2,0);
        INSERT INTO channel_slowmode VALUES(15,30);
        INSERT INTO guild_members VALUES(10,3,'member',0);
        INSERT INTO read_states(user_id,channel_id) VALUES(3,11);").await;
    ok(run(&app, 1, send(11, "secret message")).await);
    let mut flow = onboarding(
        vec![question(1, false, false, vec![answer(1, &[61], &[15])])],
        &[15],
    );
    if let CommunityCommand::SetOnboarding(o) = &mut flow {
        o.tasks = vec![GuideTask {
            id: 1,
            kind: GuideTaskKind::Talk,
            channel_id: Id(15),
            title: "Say hi".into(),
        }];
    }
    ok(cm(&app, 1, flow).await);
    ok(run(
        &app,
        1,
        Command::Launch(LaunchCommand::Moderation(
            ModerationCommand::CreateAutoModRule(AutoModRule {
                id: Id(0),
                guild_id: Id(10),
                name: "Words".into(),
                enabled: true,
                trigger: AutoModTrigger::Keyword {
                    keywords: vec!["badword".into()],
                    allow: vec![],
                },
                actions: AutoModActions {
                    block: true,
                    alert_channel_id: Some(Id(15)),
                    timeout_seconds: 0,
                    message: None,
                },
                exempt_roles: vec![Id(61)],
                exempt_channels: vec![Id(14)],
            }),
        )),
    )
    .await);
    let create = || CommunityCommand::CreateTemplate {
        guild_id: Id(10),
        name: "Starter".into(),
        description: "Our setup".into(),
    };
    assert_eq!(err(cm(&app, 2, create()).await).code, ErrorCode::Forbidden);
    let t = match ok(cm(&app, 1, create()).await) {
        CommandResult::Community(CommunityResult::Template(t)) => t,
        r => panic!("{r:?}"),
    };
    assert_eq!(t.snapshot.roles[0].id, 0);
    assert!(t.snapshot.roles.iter().all(|r| r.name != "Admin"));
    // Anyone with the link sees the preview, without where it came from.
    let preview: GuildTemplate = get(&app, 4, &format!("/api/v1/templates/{}", t.code))
        .await
        .unwrap();
    assert!(preview.source_guild_id.is_none() && preview.snapshot.channels.len() == 3);
    assert_eq!(
        get::<GuildTemplate>(&app, 4, "/api/v1/templates/nope")
            .await
            .unwrap_err(),
        StatusCode::NOT_FOUND
    );
    // Build a server from it.
    let g = match ok(cm(
        &app,
        4,
        CommunityCommand::CreateFromTemplate {
            template: TemplateRef::Code(t.code.clone()),
            name: "Copy".into(),
        },
    )
    .await)
    {
        CommandResult::Guild(d) => d.summary.guild.id,
        r => panic!("{r:?}"),
    };
    let mut c = app.0.pool.acquire().await.unwrap();
    assert_eq!(guild_users(&mut c, g).await.unwrap(), vec![Id(4)]);
    let roles = acl::roles(&mut c, g).await.unwrap();
    let helper = roles.iter().find(|r| r.name == "Helper").unwrap();
    let fan = roles.iter().find(|r| r.name == "Fan").unwrap();
    assert_eq!(
        (helper.color, helper.permissions),
        (0x123456, Permissions::MANAGE_MESSAGES)
    );
    assert!(helper.position < fan.position);
    let chans: Vec<(i64, String, Option<i64>)> =
        sqlx::query_as("SELECT id,name,parent_id FROM channels WHERE guild_id=? ORDER BY name")
            .bind(id(g).unwrap())
            .fetch_all(&mut *c)
            .await
            .unwrap();
    let rules = chans.iter().find(|c| c.1 == "rules").unwrap();
    let info = chans.iter().find(|c| c.1 == "Info").unwrap();
    assert_eq!(rules.2, Some(info.0));
    let ow = acl::overwrites(&mut c, Id(rules.0 as u64)).await.unwrap();
    assert_eq!(ow.len(), 2, "member overwrites are not copied");
    assert!(ow
        .iter()
        .any(|o| o.target_id == helper.id && o.allow == Permissions::SEND_MESSAGES));
    assert!(ow
        .iter()
        .any(|o| o.target_id == g && o.deny == Permissions::SEND_MESSAGES));
    let slow: i64 = sqlx::query_scalar("SELECT seconds FROM channel_slowmode WHERE channel_id=?")
        .bind(rules.0)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(slow, 30);
    let messages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages m JOIN channels ch ON ch.id=m.channel_id WHERE ch.guild_id=?").bind(id(g).unwrap()).fetch_one(&mut *c).await.unwrap();
    assert_eq!(messages, 0);
    let o = onboarding_settings(&mut c, g).await.unwrap();
    assert_eq!(o.questions[0].answers[0].role_ids, vec![fan.id]);
    assert_eq!(
        o.questions[0].answers[0].channel_ids,
        vec![Id(rules.0 as u64)]
    );
    assert_eq!(o.tasks[0].channel_id, Id(rules.0 as u64));
    let rule = &crate::moderation::automod::load(&mut c, g).await.unwrap()[0];
    assert_eq!(rule.exempt_roles, vec![fan.id]);
    assert_eq!(rule.exempt_channels, vec![Id(info.0 as u64)]);
    assert_eq!(rule.actions.alert_channel_id, Some(Id(rules.0 as u64)));
    drop(c);
    // Sync after the source changes.
    let list: Vec<GuildTemplate> = get(&app, 1, "/api/v1/guilds/10/templates").await.unwrap();
    assert!(!list[0].out_of_sync);
    sql(&app, "UPDATE channels SET name='the-rules' WHERE id=15").await;
    let list: Vec<GuildTemplate> = get(&app, 1, "/api/v1/guilds/10/templates").await.unwrap();
    assert!(list[0].out_of_sync);
    assert_eq!(
        err(cm(
            &app,
            3,
            CommunityCommand::SyncTemplate {
                code: t.code.clone()
            }
        )
        .await)
        .code,
        ErrorCode::Forbidden
    );
    ok(cm(
        &app,
        1,
        CommunityCommand::SyncTemplate {
            code: t.code.clone(),
        },
    )
    .await);
    let list: Vec<GuildTemplate> = get(&app, 1, "/api/v1/guilds/10/templates").await.unwrap();
    assert!(!list[0].out_of_sync && list[0].uses == 1);
    // A damaged snapshot is refused whole: no server is left behind.
    let guilds = scalar(&app, "SELECT COUNT(*) FROM guilds").await;
    let mut bad = t.snapshot.clone();
    bad.channels[1].parent = Some(99);
    sqlx::query("UPDATE guild_templates SET snapshot=? WHERE code=?")
        .bind(serde_json::to_string(&bad).unwrap())
        .bind(&t.code)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let e = err(cm(
        &app,
        3,
        CommunityCommand::CreateFromTemplate {
            template: TemplateRef::Code(t.code.clone()),
            name: "Broken".into(),
        },
    )
    .await);
    assert_eq!(e.field.as_deref(), Some("template"));
    let mut big = t.snapshot.clone();
    big.channels = (1..=MAX_CHANNELS_PER_GUILD as u32 + 1)
        .map(|i| TemplateChannel {
            id: i,
            parent: None,
            kind: ChannelKind::Text,
            name: format!("c{i}"),
            topic: None,
            slowmode: 0,
            age_restricted: false,
            overwrites: vec![],
        })
        .collect();
    sqlx::query("UPDATE guild_templates SET snapshot=? WHERE code=?")
        .bind(serde_json::to_string(&big).unwrap())
        .bind(&t.code)
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        err(cm(
            &app,
            3,
            CommunityCommand::CreateFromTemplate {
                template: TemplateRef::Code(t.code.clone()),
                name: "Big".into()
            }
        )
        .await)
        .field
        .as_deref(),
        Some("template")
    );
    let mut admin = t.snapshot.clone();
    admin.roles[0].permissions = Permissions::ADMINISTRATOR | Permissions::SEND_MESSAGES;
    sqlx::query("UPDATE guild_templates SET snapshot=? WHERE code=?")
        .bind(serde_json::to_string(&admin).unwrap())
        .bind(&t.code)
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(scalar(&app, "SELECT COUNT(*) FROM guilds").await, guilds);
    let g2 = match ok(cm(
        &app,
        3,
        CommunityCommand::CreateFromTemplate {
            template: TemplateRef::Code(t.code.clone()),
            name: "Admins".into(),
        },
    )
    .await)
    {
        CommandResult::Guild(d) => d.summary.guild.id,
        r => panic!("{r:?}"),
    };
    let mut c = app.0.pool.acquire().await.unwrap();
    let everyone = acl::roles(&mut c, g2)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == g2)
        .unwrap();
    assert!(!everyone.permissions.contains(Permissions::ADMINISTRATOR));
    drop(c);
    // Deleting needs Manage Server on the source.
    assert_eq!(
        err(cm(
            &app,
            2,
            CommunityCommand::DeleteTemplate {
                code: t.code.clone()
            }
        )
        .await)
        .code,
        ErrorCode::NotFound
    );
    ok(cm(
        &app,
        1,
        CommunityCommand::DeleteTemplate {
            code: t.code.clone(),
        },
    )
    .await);
    assert_eq!(scalar(&app, "SELECT COUNT(*) FROM audit_log WHERE action IN('template_create','template_update','template_delete')").await, 3);
}

#[tokio::test]
async fn starter_templates_build() {
    let (_d, app) = fixture().await;
    let list: Vec<GuildTemplate> = get(&app, 3, "/api/v1/templates").await.unwrap();
    assert_eq!(list.len(), 5);
    for t in list {
        let key = t.code.trim_start_matches("builtin:").to_owned();
        let g = match ok(cm(
            &app,
            3,
            CommunityCommand::CreateFromTemplate {
                template: TemplateRef::Builtin(key.clone()),
                name: t.name.clone(),
            },
        )
        .await)
        {
            CommandResult::Guild(d) => d,
            r => panic!("{key}: {r:?}"),
        };
        assert_eq!(g.channels.len(), t.snapshot.channels.len(), "{key}");
        let mut c = app.0.pool.acquire().await.unwrap();
        // The owner is never held by its own rules.
        assert!(!pending(&mut c, g.summary.guild.id, Id(3)).await.unwrap());
        if key == "club" {
            let o = onboarding_settings(&mut c, g.summary.guild.id)
                .await
                .unwrap();
            let member = acl::roles(&mut c, g.summary.guild.id)
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.name == "Member")
                .unwrap();
            assert_eq!(o.questions[0].answers[0].role_ids, vec![member.id]);
        }
    }
    assert_eq!(
        err(cm(
            &app,
            3,
            CommunityCommand::CreateFromTemplate {
                template: TemplateRef::Builtin("nope".into()),
                name: "x".into()
            }
        )
        .await)
        .code,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn insights_are_rolled_up_and_only_for_managers() {
    let (_d, app) = fixture().await;
    let code = invite(&app).await;
    ok(join(&app, 3, &code).await);
    ok(run(&app, 3, send(11, "one")).await);
    ok(run(&app, 3, send(11, "two")).await);
    ok(run(&app, 3, Command::LeaveGuild { guild_id: Id(10) }).await);
    {
        let mut c = app.0.pool.acquire().await.unwrap();
        insights::voice(&mut c, Id(10), Id(11), 5).await.unwrap();
        // Two older members: one stayed and talked, one did not.
        let day = 86_400_000_i64;
        let t = now() as i64 - 10 * day;
        sqlx::query("INSERT INTO insight_joins VALUES(10,1001,?,?,NULL),(10,1002,?,NULL,NULL)")
            .bind(t)
            .bind(t + 3_600_000)
            .bind(t)
            .execute(&mut *c)
            .await
            .unwrap();
        sqlx::query("INSERT INTO insight_daily(guild_id,day,joins) VALUES(10,1,7)")
            .execute(&mut *c)
            .await
            .unwrap();
        insights::trim(&mut c, Id(10), day_of(now())).await.unwrap();
    }
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM insight_daily WHERE day=1").await,
        0
    );
    assert_eq!(
        get::<Insights>(&app, 2, "/api/v1/guilds/10/insights")
            .await
            .unwrap_err(),
        StatusCode::FORBIDDEN
    );
    give(&app, 2, 30, Permissions::VIEW_AUDIT_LOG, 1).await;
    let i: Insights = get(&app, 2, "/api/v1/guilds/10/insights?days=30")
        .await
        .unwrap();
    assert_eq!(i.days.len(), 30);
    let today = i.days.last().unwrap();
    assert_eq!(
        (
            today.joins,
            today.leaves,
            today.messages,
            today.voice_minutes
        ),
        (1, 1, 2, 5)
    );
    assert_eq!(i.sources.len(), 1);
    assert_eq!(
        (
            i.sources[0].source,
            i.sources[0].code.as_str(),
            i.sources[0].joins
        ),
        (JoinSource::Invite, code.as_str(), 1)
    );
    assert_eq!(
        (i.channels[0].channel_id, i.channels[0].messages),
        (Id(11), 2)
    );
    let old = i.retention.iter().find(|r| r.joined == 2).unwrap();
    assert_eq!((old.retained_1d, old.retained_7d), (Some(1), Some(1)));
    let fresh = i.retention.iter().find(|r| r.day == i.today).unwrap();
    assert_eq!(fresh.retained_1d, None);
    assert_eq!(i.member_count, 2);
    // Messages stay only counted; nothing per member leaves this route.
    let raw = serde_json::to_string(&i).unwrap();
    assert!(!raw.contains("\"user_id\""));
}

/// Security review G4: onboarding gives ordinary member rights only. Mention
/// Everyone, voice moderation and the like are refused, and so is a role that
/// gains a forbidden right through a channel overwrite, at save and at grant.
#[tokio::test]
async fn onboarding_refuses_broadcast_and_moderation_rights_and_overwrites() {
    let (_d, app) = fixture().await;
    sql(&app, &format!("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'text',10,'help',1,0);
        INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,40,'Helpers',2,{}),(10,43,'Plain',2,{}),(10,44,'Sneaky',2,{});
        INSERT INTO permission_overwrites VALUES(12,'role',44,{},0);",
        (Permissions::MENTION_EVERYONE | Permissions::MOVE_MEMBERS).bits(),
        (Permissions::SEND_MESSAGES | Permissions::CONNECT).bits(),
        Permissions::SEND_MESSAGES.bits(),
        Permissions::MANAGE_MESSAGES.bits())).await;
    give(
        &app,
        2,
        50,
        Permissions::MANAGE_GUILD | Permissions::MANAGE_ROLES,
        5,
    )
    .await;
    let flow = |role: u64| {
        onboarding(
            vec![question(
                1,
                true,
                true,
                vec![answer(1, &[role], &[]), answer(2, &[], &[])],
            )],
            &[],
        )
    };
    for refused in [40, 44] {
        assert_eq!(
            err(cm(&app, 2, flow(refused)).await).field.as_deref(),
            Some("roles"),
            "role {refused}"
        );
    }
    ok(cm(&app, 2, flow(43)).await);
    // Granted while it is plain; not once an overwrite gives it more.
    let code = invite(&app).await;
    ok(join(&app, 3, &code).await);
    ok(cm(&app, 3, respond(&[(1, 1)])).await);
    assert!(roles_of(&app, 3).await.contains(&Id(43)));
    ok(cm(&app, 3, respond(&[(1, 2)])).await);
    assert!(!roles_of(&app, 3).await.contains(&Id(43)));
    sql(
        &app,
        &format!(
            "INSERT INTO permission_overwrites VALUES(12,'role',43,{},0)",
            Permissions::MENTION_EVERYONE.bits()
        ),
    )
    .await;
    ok(cm(&app, 3, respond(&[(1, 1)])).await);
    assert!(!roles_of(&app, 3).await.contains(&Id(43)));
}
