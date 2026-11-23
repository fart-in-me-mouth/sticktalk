//! Profiles: every command, route, permission check and limit.
use super::*;
use crate::{audit_tests::fixture, commands};
use axum::body::Body;
use base64::Engine as _;
use tower::ServiceExt;

async fn run(app: &App, u: u64, command: ProfilesCommand) -> CommandOutcome {
    raw(app, u, Command::Launch(LaunchCommand::Profiles(command))).await
}
static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
async fn raw(app: &App, u: u64, command: Command) -> CommandOutcome {
    let mut core = app.0.core.lock().await;
    core.limits.clear();
    commands::execute(
        app,
        &mut core,
        &[u as u8; 32],
        Id(u),
        CommandRequest {
            nonce: NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            command,
        },
    )
    .await
    .unwrap()
    .outcome
}
fn ok(o: &CommandOutcome) {
    assert!(matches!(o, CommandOutcome::Ok(_)), "{o:?}");
}
fn err(o: &CommandOutcome) -> ErrorCode {
    match o {
        CommandOutcome::Err(e) => e.code,
        o => panic!("expected an error, got {o:?}"),
    }
}
async fn sql(app: &App, q: &'static str) {
    sqlx::query(q).execute(&app.0.pool).await.unwrap();
}
async fn scalar(app: &App, q: &'static str) -> i64 {
    sqlx::query_scalar(q).fetch_one(&app.0.pool).await.unwrap()
}
/// A fresh avatar upload `a` owned by `owner`.
async fn upload(app: &App, a: i64, owner: i64, mime: &str) {
    sqlx::query("INSERT INTO uploads(id,owner_id,purpose,filename,content_type,byte_size,storage_key,width,height,created_at,expires_at) VALUES(?,?,'avatar','a',?,4,?,64,64,0,?)")
        .bind(a)
        .bind(owner)
        .bind(mime)
        .bind(format!("u{a}.bin"))
        .bind((now() + 600_000) as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
}
fn edit() -> ProfileEdit {
    ProfileEdit {
        pronouns: "they/them".into(),
        about_me: "**hi** there".into(),
        banner_color: 0x112233,
        ..Default::default()
    }
}
fn ring(core: &Core, u: u8) -> Vec<Event> {
    core.streams
        .get(&vec![u; 32])
        .map(|s| {
            s.ring
                .iter()
                .filter_map(
                    |(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).unwrap() {
                        ServerMsg::Event(e) => Some(e.event),
                        _ => None,
                    },
                )
                .collect()
        })
        .unwrap_or_default()
}
async fn get(app: &App, u: u8, path: &str) -> (u16, Vec<u8>) {
    let t = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([u; 32]);
    sqlx::query("INSERT OR IGNORE INTO sessions VALUES(?,?,?,?,?)")
        .bind(crate::state::hash(t.as_bytes()))
        .bind(i64::from(u))
        .bind(now() as i64)
        .bind((now() + 600_000) as i64)
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    app.0.core.lock().await.limits.clear();
    let r = crate::http::router(app.clone())
        .oneshot(
            axum::http::Request::get(path)
                .header("authorization", format!("Bearer {t}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status().as_u16();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn global_profile_is_validated_saved_and_announced() {
    let (_dir, app) = fixture().await;
    // User 2 has an event stream and shares the server with user 1.
    let _ = raw(&app, 2, Command::Typing { channel_id: Id(11) }).await;
    {
        // ... and is looking at the server, with a current client.
        let mut core = app.0.core.lock().await;
        let s = core.streams.get_mut(&vec![2; 32]).unwrap();
        s.guild = Some(Id(10));
        s.launch_features = true;
    }
    let gradient = NameStyle {
        font: NameFont::Pixel,
        color: 0xFFFFFF,
        color2: Some(0x3DDC97),
        effect: NameEffect::Gradient,
    };
    // Illegible colors, unknown cosmetics, long text and bad colors are refused.
    for bad in [
        ProfileEdit {
            name: Some(NameStyle {
                color: 0x101010,
                ..gradient
            }),
            ..edit()
        },
        ProfileEdit {
            name: Some(NameStyle {
                color2: None,
                ..gradient
            }),
            ..edit()
        },
        ProfileEdit {
            decoration: Some("not_a_thing".into()),
            ..edit()
        },
        ProfileEdit {
            effect: Some("halo".into()),
            ..edit()
        },
        ProfileEdit {
            about_me: "x".repeat(MAX_ABOUT_ME_CHARS + 1),
            ..edit()
        },
        ProfileEdit {
            pronouns: "a\u{7}b".into(),
            ..edit()
        },
        ProfileEdit {
            primary: Some(0x1_000_000),
            ..edit()
        },
    ] {
        assert_eq!(
            err(&run(&app, 1, ProfilesCommand::Update(bad)).await),
            ErrorCode::InvalidRequest
        );
    }
    ok(&run(
        &app,
        1,
        ProfilesCommand::Update(ProfileEdit {
            name: Some(gradient),
            decoration: Some("halo".into()),
            nameplate: Some("forest".into()),
            effect: Some("aurora".into()),
            frame: Some("neon".into()),
            primary: Some(0xFF0000),
            accent: Some(0x00FF00),
            ..edit()
        }),
    )
    .await);
    let u = user(&mut app.0.pool.acquire().await.unwrap(), Id(1))
        .await
        .unwrap();
    assert_eq!(u.style.name, Some(gradient));
    assert_eq!(u.style.decoration.as_deref(), Some("halo"));
    {
        let core = app.0.core.lock().await;
        let events = ring(&core, 2);
        // Server members see the new look through the member (UserUpdated goes to friends).
        assert!(events.iter().any(|e| matches!(e, Event::MemberUpsert(m) if m.user.style.name.is_some() && m.user.style.nameplate.as_deref() == Some("forest"))));
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::ProfileUpdated(p) if p.pronouns == "they/them")));
    }
    let (status, body) = get(&app, 2, "/api/v1/users/1/card").await;
    assert_eq!(status, 200);
    let card: ProfileCard = serde_json::from_slice(&body).unwrap();
    assert_eq!(card.about_me, "**hi** there");
    assert_eq!(card.look.effect.as_deref(), Some("aurora"));
    assert_eq!(card.look.primary, Some(0xFF0000));
    // Someone who shares nothing with user 1 cannot read the card.
    assert_eq!(get(&app, 3, "/api/v1/users/1/card").await.0, 404);
    // A card opened from a server the viewer is not in is refused.
    assert_eq!(get(&app, 2, "/api/v1/users/1/card?guild=99").await.0, 404);
}

#[tokio::test]
async fn avatars_and_banners_use_own_fresh_uploads_and_are_shared_only_with_viewers() {
    let (_dir, app) = fixture().await;
    upload(&app, 500, 1, "image/gif").await;
    upload(&app, 501, 2, "image/png").await;
    upload(&app, 502, 1, "text/plain").await;
    let set = |guild: Option<u64>, slot, id: u64, crop: Option<Crop>| ProfilesCommand::SetImage {
        guild_id: guild.map(Id),
        slot,
        image: Some(ImageRef { id: Id(id), crop }),
    };
    // Someone else's upload, a non-image, and a bad crop are refused.
    assert_eq!(
        err(&run(&app, 1, set(None, ImageSlot::Avatar, 501, None)).await),
        ErrorCode::InvalidAttachment
    );
    assert_eq!(
        err(&run(&app, 1, set(None, ImageSlot::Avatar, 502, None)).await),
        ErrorCode::UnsupportedMediaType
    );
    let crop = Crop {
        zoom: 150,
        x: 20,
        y: 70,
    };
    assert_eq!(
        err(&run(
            &app,
            1,
            set(
                None,
                ImageSlot::Avatar,
                500,
                Some(Crop { zoom: 50, ..crop })
            )
        )
        .await),
        ErrorCode::InvalidRequest
    );
    ok(&run(&app, 1, set(None, ImageSlot::Avatar, 500, Some(crop))).await);
    let u = user(&mut app.0.pool.acquire().await.unwrap(), Id(1))
        .await
        .unwrap();
    assert_eq!(u.avatar_id, Some(Id(500)));
    assert_eq!(
        u.style.avatar,
        Some(ImageMeta {
            id: Id(500),
            animated: true,
            crop: Some(crop)
        })
    );
    // The same upload, now in use, may become the banner too (re-cropped).
    ok(&run(&app, 1, set(None, ImageSlot::Banner, 500, None)).await);
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM profile_uploads WHERE user_id=1 AND slot='banner'"
        )
        .await,
        1
    );
    // The avatar changes; the banner keeps the upload from garbage collection.
    ok(&run(
        &app,
        1,
        ProfilesCommand::SetImage {
            guild_id: None,
            slot: ImageSlot::Avatar,
            image: None,
        },
    )
    .await);
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(gc_uploads(&mut c).await.unwrap().is_empty());
    // Viewers who share a server may download it; strangers may not.
    assert!(file_access(&mut c, Id(2), Id(500)).await.unwrap());
    assert!(!file_access(&mut c, Id(3), Id(500)).await.unwrap());
    drop(c);
    // Unused now: collected with the command that freed it.
    ok(&run(
        &app,
        1,
        ProfilesCommand::SetImage {
            guild_id: None,
            slot: ImageSlot::Banner,
            image: None,
        },
    )
    .await);
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM uploads WHERE id=500").await,
        0
    );
}

#[tokio::test]
async fn server_profiles_override_and_leave_with_the_member() {
    let (_dir, app) = fixture().await;
    upload(&app, 600, 2, "image/png").await;
    let server_edit = ServerProfileEdit {
        about_me: Some("server bio".into()),
        decoration: Some("crown".into()),
        name: Some(NameStyle {
            font: NameFont::Script,
            color: 0xF2B84B,
            color2: None,
            effect: NameEffect::Glow,
        }),
        ..Default::default()
    };
    // Not a member of server 99.
    assert_eq!(
        err(&run(
            &app,
            2,
            ProfilesCommand::UpdateServer {
                guild_id: Id(99),
                edit: server_edit.clone()
            }
        )
        .await),
        ErrorCode::NotFound
    );
    assert_eq!(
        err(&run(
            &app,
            3,
            ProfilesCommand::UpdateServer {
                guild_id: Id(10),
                edit: server_edit.clone()
            }
        )
        .await),
        ErrorCode::NotFound
    );
    ok(&run(
        &app,
        2,
        ProfilesCommand::UpdateServer {
            guild_id: Id(10),
            edit: server_edit,
        },
    )
    .await);
    ok(&run(
        &app,
        2,
        ProfilesCommand::SetImage {
            guild_id: Some(Id(10)),
            slot: ImageSlot::Avatar,
            image: Some(ImageRef {
                id: Id(600),
                crop: None,
            }),
        },
    )
    .await);
    let m = member(&mut app.0.pool.acquire().await.unwrap(), Id(10), Id(2))
        .await
        .unwrap();
    assert_eq!(m.server_style.decoration.as_deref(), Some("crown"));
    assert_eq!(m.server_style.avatar.as_ref().map(|a| a.id), Some(Id(600)));
    assert!(!m.server_style.avatar.unwrap().animated);
    // The global look is untouched.
    assert!(m.user.style.is_empty());
    // Members of the server may download the server avatar; others may not.
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(file_access(&mut c, Id(1), Id(600)).await.unwrap());
    assert!(!file_access(&mut c, Id(3), Id(600)).await.unwrap());
    drop(c);
    let (status, body) = get(&app, 1, "/api/v1/users/2/card?guild=10").await;
    assert_eq!(status, 200);
    let card: ProfileCard = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        card.server.as_ref().and_then(|s| s.about_me.as_deref()),
        Some("server bio")
    );
    assert!(card.member_since.is_some());
    let (_, body) = get(&app, 2, "/api/v1/users/@me/profiles").await;
    let mine: MyProfiles = serde_json::from_slice(&body).unwrap();
    assert_eq!(mine.servers.len(), 1);
    // Leaving drops the server profile and frees its images.
    sql(
        &app,
        "DELETE FROM guild_members WHERE guild_id=10 AND user_id=2",
    )
    .await;
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM member_profiles").await,
        0
    );
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM profile_uploads").await,
        0
    );
}

#[tokio::test]
async fn moderators_reset_server_images_with_permission_hierarchy_and_audit() {
    let (_dir, app) = fixture().await;
    sql(&app, "INSERT INTO guild_members VALUES(10,3,'member',0)").await;
    upload(&app, 700, 2, "image/png").await;
    upload(&app, 701, 2, "image/png").await;
    for (slot, a) in [(ImageSlot::Avatar, 700), (ImageSlot::Banner, 701)] {
        ok(&run(
            &app,
            2,
            ProfilesCommand::SetImage {
                guild_id: Some(Id(10)),
                slot,
                image: Some(ImageRef {
                    id: Id(a),
                    crop: None,
                }),
            },
        )
        .await);
    }
    let reset = |by_user| ProfilesCommand::Reset {
        guild_id: Id(10),
        user_id: Id(by_user),
        avatar: true,
        banner: true,
        reason: Some("rules".into()),
    };
    // Without Time Out Members: refused.
    assert_eq!(err(&run(&app, 3, reset(2)).await), ErrorCode::Forbidden);
    // With it, but not above the member: refused.
    sqlx::query(
        "INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,30,'mods',2,?)",
    )
    .bind(Permissions::MODERATE_MEMBERS.bits() as i64)
    .execute(&app.0.pool)
    .await
    .unwrap();
    sql(&app, "INSERT INTO member_roles VALUES(10,3,30)").await;
    sql(
        &app,
        "INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(10,31,'senior',3,0)",
    )
    .await;
    sql(&app, "INSERT INTO member_roles VALUES(10,2,31)").await;
    assert_eq!(
        err(&run(&app, 3, reset(2)).await),
        ErrorCode::HierarchyViolation
    );
    // The owner may, and it is audited.
    ok(&run(&app, 1, reset(2)).await);
    let p = server_profile(&mut app.0.pool.acquire().await.unwrap(), Id(10), Id(2))
        .await
        .unwrap();
    assert!(p.avatar.is_none() && p.banner.is_none());
    let entry: (i64, i64, String, String) = sqlx::query_as("SELECT actor_id,target_id,reason,changes FROM audit_log WHERE action='member_profile_reset'")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!((entry.0, entry.1, entry.2.as_str()), (1, 2, "rules"));
    assert!(entry.3.contains("server_avatar") && entry.3.contains("server_banner"));
    // Nothing to reset at all is not a request.
    assert_eq!(
        err(&run(
            &app,
            1,
            ProfilesCommand::Reset {
                guild_id: Id(10),
                user_id: Id(2),
                avatar: false,
                banner: false,
                reason: None
            }
        )
        .await),
        ErrorCode::InvalidRequest
    );
    // Nicknames reset through the existing command, which needs Manage Nicknames.
    ok(&raw(
        &app,
        2,
        Command::Launch(LaunchCommand::SetNickname {
            guild_id: Id(10),
            user_id: Id(2),
            nickname: Some("Nick".into()),
        }),
    )
    .await);
    assert_eq!(
        err(&raw(
            &app,
            3,
            Command::Launch(LaunchCommand::SetNickname {
                guild_id: Id(10),
                user_id: Id(2),
                nickname: None
            })
        )
        .await),
        ErrorCode::Forbidden
    );
}

#[tokio::test]
async fn badges_are_earned_awarded_chosen_and_served() {
    let (_dir, app) = fixture().await;
    let year = 31_557_600_000u64;
    // User 1: 2.5 years old, early; user 2: brand new.
    sqlx::query("UPDATE users SET created_at=? WHERE id=1")
        .bind((now() - year * 5 / 2) as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut c = app.0.pool.acquire().await.unwrap();
    let ids = |b: Vec<Badge>| b.into_iter().map(|b| b.id).collect::<Vec<_>>();
    assert_eq!(
        ids(badges::earned(&mut c, Id(1)).await.unwrap()),
        ["early_member", "member_years"]
    );
    let years = badges::earned(&mut c, Id(1)).await.unwrap().pop().unwrap();
    assert_eq!(
        (years.icon.as_deref(), years.name.as_str()),
        (Some("years_2"), "Member for 2 Years")
    );
    // The administrator moves the early cutoff before everyone, and awards badges.
    badges::run(
        &mut c,
        &badges::BadgeCommand::EarlyCutoff {
            date: "2020-01-01".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(badges::parse_date("2025-01-01").unwrap(), 1_735_689_600_000);
    assert!(badges::parse_date("2025-13-01").is_err());
    badges::run(
        &mut c,
        &badges::BadgeCommand::Award {
            badge: "bug_reporter".into(),
            username: "user1".into(),
        },
    )
    .await
    .unwrap();
    badges::run(
        &mut c,
        &badges::BadgeCommand::Award {
            badge: "instance_admin".into(),
            username: "user1".into(),
        },
    )
    .await
    .unwrap();
    // Computed badges cannot be awarded; unknown custom badges neither.
    assert!(badges::run(
        &mut c,
        &badges::BadgeCommand::Award {
            badge: "early_member".into(),
            username: "user1".into()
        }
    )
    .await
    .is_err());
    assert!(badges::run(
        &mut c,
        &badges::BadgeCommand::Award {
            badge: "c9".into(),
            username: "user1".into()
        }
    )
    .await
    .is_err());
    let dir = tempfile::tempdir().unwrap();
    let icon = dir.path().join("icon.png");
    image::RgbaImage::from_pixel(32, 32, image::Rgba([200, 10, 10, 255]))
        .save(&icon)
        .unwrap();
    badges::run(
        &mut c,
        &badges::BadgeCommand::Create {
            name: "Helper".into(),
            description: "Helps out".into(),
            icon: icon.clone(),
        },
    )
    .await
    .unwrap();
    let custom: i64 = sqlx::query_scalar("SELECT id FROM badges")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    badges::run(
        &mut c,
        &badges::BadgeCommand::Award {
            badge: format!("c{custom}"),
            username: "user1".into(),
        },
    )
    .await
    .unwrap();
    // A big server makes its owner a Server Owner.
    for u in 100..(100 + SERVER_OWNER_BADGE_MEMBERS as i64) {
        sqlx::query("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(?,?,'x','online',0)")
            .bind(u)
            .bind(format!("bulk{u}"))
            .execute(&mut *c)
            .await
            .unwrap();
        sqlx::query("INSERT INTO guild_members VALUES(10,?,'member',0)")
            .bind(u)
            .execute(&mut *c)
            .await
            .unwrap();
    }
    assert_eq!(
        ids(badges::earned(&mut c, Id(1)).await.unwrap()),
        [
            "instance_admin",
            "server_owner",
            "member_years",
            "bug_reporter",
            &format!("c{custom}")
        ]
    );
    drop(c);
    // Choosing: only earned, at most six, no repeats.
    let pick = |v: &[&str]| ProfilesCommand::SetBadges {
        shown: v.iter().map(|s| s.to_string()).collect(),
    };
    assert_eq!(
        err(&run(&app, 1, pick(&["contributor"])).await),
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        err(&run(&app, 1, pick(&["bug_reporter", "bug_reporter"])).await),
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        err(&run(&app, 1, pick(&["a"; 7])).await),
        ErrorCode::LimitExceeded
    );
    let custom_id = format!("c{custom}");
    ok(&run(&app, 1, pick(&[custom_id.as_str(), "bug_reporter"])).await);
    let (_, body) = get(&app, 2, "/api/v1/users/1/card").await;
    let card: ProfileCard = serde_json::from_slice(&body).unwrap();
    assert_eq!(ids(card.badges), [custom_id.as_str(), "bug_reporter"]);
    let (_, body) = get(&app, 1, "/api/v1/users/@me/profiles").await;
    let mine: MyProfiles = serde_json::from_slice(&body).unwrap();
    assert!(mine.chosen);
    assert_eq!(mine.earned.len(), 5);
    assert_eq!(mine.earned[0].id, custom_id);
    // The custom icon is a 64x64 PNG for any signed-in user.
    let (status, body) = get(&app, 3, &format!("/api/v1/badges/{custom}/icon")).await;
    assert_eq!(status, 200);
    assert_eq!(image::load_from_memory(&body).unwrap().width(), 64);
    assert_eq!(get(&app, 3, "/api/v1/badges/999/icon").await.0, 404);
    // Revoking and deleting take badges away again.
    let mut c = app.0.pool.acquire().await.unwrap();
    badges::run(&mut c, &badges::BadgeCommand::Delete { id: custom as u64 })
        .await
        .unwrap();
    badges::run(
        &mut c,
        &badges::BadgeCommand::Revoke {
            badge: "bug_reporter".into(),
            username: "user1".into(),
        },
    )
    .await
    .unwrap();
    assert!(badges::run(
        &mut c,
        &badges::BadgeCommand::Revoke {
            badge: "bug_reporter".into(),
            username: "user1".into()
        }
    )
    .await
    .is_err());
    drop(c);
    let (_, body) = get(&app, 2, "/api/v1/users/1/card").await;
    let card: ProfileCard = serde_json::from_slice(&body).unwrap();
    assert!(card.badges.is_empty());
}

#[tokio::test]
async fn erasing_an_account_clears_its_profile() {
    let (_dir, app) = fixture().await;
    ok(&run(
        &app,
        2,
        ProfilesCommand::Update(ProfileEdit {
            decoration: Some("orbit".into()),
            ..edit()
        }),
    )
    .await);
    ok(&run(
        &app,
        2,
        ProfilesCommand::UpdateServer {
            guild_id: Id(10),
            edit: ServerProfileEdit {
                about_me: Some("x".into()),
                ..Default::default()
            },
        },
    )
    .await);
    let mut c = app.0.pool.acquire().await.unwrap();
    crate::account::erase(&mut c, Id(2)).await.unwrap();
    assert!(user_style(&mut c, Id(2)).await.unwrap().is_empty());
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM member_profiles").await,
        0
    );
}
