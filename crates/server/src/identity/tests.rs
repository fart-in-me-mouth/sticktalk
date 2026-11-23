//! Identity: every command, route, permission check and limit.
use super::*;
use crate::{audit_tests::fixture, commands, state::App};
use axum::body::Body;
use base64::Engine as _;
use tower::ServiceExt;

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
async fn run(app: &App, u: u64, cmd: IdentityCommand) -> CommandOutcome {
    raw(app, u, Command::Identity(cmd)).await
}
#[track_caller]
fn ok(o: &CommandOutcome) -> CommandResult {
    match o {
        CommandOutcome::Ok(r) => r.clone(),
        CommandOutcome::Err(e) => panic!("expected success, got {e:?}"),
    }
}
#[track_caller]
fn err(o: &CommandOutcome) -> ApiError {
    match o {
        CommandOutcome::Err(e) => e.clone(),
        o => panic!("expected an error, got {o:?}"),
    }
}
async fn sql(app: &App, q: &str) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(q.to_owned()))
        .execute(&app.0.pool)
        .await
        .unwrap();
}
async fn scalar(app: &App, q: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(q.to_owned()))
        .fetch_one(&app.0.pool)
        .await
        .unwrap()
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
async fn request(app: &App, u: u8, method: &str, path: &str, body: Vec<u8>) -> (u16, Vec<u8>) {
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
            axum::http::Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {t}"))
                .header("x-sticktalk-csrf", "1")
                .header("origin", "http://localhost")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status().as_u16();
    let location = r
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap().as_bytes().to_vec());
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    (status, location.unwrap_or_else(|| bytes.to_vec()))
}
async fn get(app: &App, u: u8, path: &str) -> (u16, Vec<u8>) {
    request(app, u, "GET", path, vec![]).await
}
fn tag(text: &str) -> TagEdit {
    TagEdit {
        text: text.into(),
        glyph: "leaf".into(),
        color: 0x1C3A2E,
        color2: 0x3DDC97,
        invite_code: None,
    }
}
fn set_tag(text: &str) -> IdentityCommand {
    IdentityCommand::SetTag {
        guild_id: Id(10),
        tag: Some(tag(text)),
    }
}
/// A second server, 20, owned by user 3, with user 4 in it.
async fn second(app: &App) {
    sql(app, "INSERT INTO guilds VALUES(20,3,'other',NULL,0); INSERT INTO guild_members VALUES(20,3,'owner',0),(20,4,'member',0);").await;
}
async fn summary(app: &App, u: u64) -> UserSummary {
    user(&mut app.0.pool.acquire().await.unwrap(), Id(u))
        .await
        .unwrap()
}
/// Users 1 and 2 open server 10 (their streams then receive its events).
async fn open(app: &App, users: &[u64]) {
    for u in users {
        ok(&raw(
            app,
            *u,
            Command::SubscribeGuild {
                guild_id: Some(Id(10)),
                presence_user_ids: vec![],
            },
        )
        .await);
        ok(&raw(
            app,
            *u,
            Command::Launch(LaunchCommand::Moderation(ModerationCommand::Subscribe)),
        )
        .await);
    }
}
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
async fn invite(app: &App) -> String {
    match ok(&raw(
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

#[tokio::test]
async fn tags_are_validated_unique_reserved_and_screened() {
    let (_d, app) = fixture().await;
    second(&app).await;
    // Manage Server only.
    assert_eq!(
        err(&run(&app, 2, set_tag("GG")).await).code,
        ErrorCode::Forbidden
    );
    for bad in ["", "TOOLONG", "a b", "<i>", "!!", "é"] {
        assert_eq!(
            err(&run(&app, 1, set_tag(bad)).await).field.as_deref(),
            Some("tag"),
            "{bad}"
        );
    }
    for reserved in ["MOD", "m0d", "Ad-M", "STK", "5Y5"] {
        let e = err(&run(&app, 1, set_tag(reserved)).await);
        assert!(e.message.contains("reserved"), "{reserved}: {e:?}");
    }
    let mut bad_glyph = tag("GG");
    bad_glyph.glyph = "nope".into();
    let e = err(&run(
        &app,
        1,
        IdentityCommand::SetTag {
            guild_id: Id(10),
            tag: Some(bad_glyph),
        },
    )
    .await);
    assert_eq!(e.field.as_deref(), Some("glyph"));
    let mut faint = tag("GG");
    faint.color2 = 0x1D3B2F;
    let e = err(&run(
        &app,
        1,
        IdentityCommand::SetTag {
            guild_id: Id(10),
            tag: Some(faint),
        },
    )
    .await);
    assert_eq!(e.field.as_deref(), Some("color"));
    // An invite offered on the tag must be one of this server's.
    let mut stray = tag("GG");
    stray.invite_code = Some("abcdefghijklmnopqrstuv".into());
    let e = err(&run(
        &app,
        1,
        IdentityCommand::SetTag {
            guild_id: Id(10),
            tag: Some(stray),
        },
    )
    .await);
    assert_eq!(e.field.as_deref(), Some("invite"));

    ok(&run(&app, 1, set_tag("Gg")).await);
    // Unique on the instance, ignoring case.
    let e = err(&run(
        &app,
        3,
        IdentityCommand::SetTag {
            guild_id: Id(20),
            tag: Some(tag("gG")),
        },
    )
    .await);
    assert_eq!(e.code, ErrorCode::AlreadyExists);
    // The server's own AutoMod keyword rules apply, with no exemption.
    ok(&raw(
        &app,
        3,
        Command::Launch(LaunchCommand::Moderation(
            ModerationCommand::CreateAutoModRule(AutoModRule {
                id: Id(0),
                guild_id: Id(20),
                name: "Words".into(),
                enabled: true,
                trigger: AutoModTrigger::Keyword {
                    keywords: vec!["boo*".into()],
                    allow: vec![],
                },
                actions: AutoModActions {
                    block: true,
                    ..Default::default()
                },
                exempt_roles: vec![],
                exempt_channels: vec![],
            }),
        )),
    )
    .await);
    let e = err(&run(
        &app,
        3,
        IdentityCommand::SetTag {
            guild_id: Id(20),
            tag: Some(tag("BOOO")),
        },
    )
    .await);
    assert_eq!(e.field.as_deref(), Some("automod"));
    ok(&run(
        &app,
        3,
        IdentityCommand::SetTag {
            guild_id: Id(20),
            tag: Some(tag("CATS")),
        },
    )
    .await);
    // Every change is in the audit log.
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM audit_log WHERE action='server_tag_update'"
        )
        .await,
        2
    );
    // Removing frees the text.
    ok(&run(
        &app,
        1,
        IdentityCommand::SetTag {
            guild_id: Id(10),
            tag: None,
        },
    )
    .await);
    ok(&run(
        &app,
        3,
        IdentityCommand::SetTag {
            guild_id: Id(20),
            tag: Some(tag("GG")),
        },
    )
    .await);
}

#[tokio::test]
async fn members_wear_one_tag_everywhere_and_lose_it_when_they_leave() {
    let (_d, app) = fixture().await;
    second(&app).await;
    open(&app, &[1, 2]).await;
    ok(&run(&app, 1, set_tag("GG")).await);
    // Only members, and only servers with a tag.
    assert_eq!(
        err(&run(
            &app,
            3,
            IdentityCommand::WearTag {
                guild_id: Some(Id(10))
            }
        )
        .await)
        .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        err(&run(
            &app,
            4,
            IdentityCommand::WearTag {
                guild_id: Some(Id(20))
            }
        )
        .await)
        .field
        .as_deref(),
        Some("tag")
    );
    ok(&run(
        &app,
        2,
        IdentityCommand::WearTag {
            guild_id: Some(Id(10)),
        },
    )
    .await);
    let worn = summary(&app, 2).await.style.tag.unwrap();
    assert_eq!((worn.guild_id, worn.text.as_str()), (Id(10), "GG"));
    {
        let core = app.0.core.lock().await;
        assert!(ring(&core, 2)
            .iter()
            .any(|e| matches!(e, Event::SelfUpdated(m) if m.user.style.tag.is_some())));
        assert!(ring(&core, 1)
            .iter()
            .any(|e| matches!(e, Event::MemberUpsert(m) if m.user.style.tag.as_ref().is_some_and(|t| t.text == "GG"))));
    }
    let (s, body) = get(&app, 2, "/api/v1/users/@me/tags").await;
    assert_eq!(s, 200);
    let mine: MyTags = serde_json::from_slice(&body).unwrap();
    assert_eq!(mine.wearing, Some(Id(10)));
    assert_eq!(mine.available.len(), 1);
    // A changed tag reaches the copy every wearer carries.
    let mut new = tag("GGWP");
    new.glyph = "star".into();
    ok(&run(
        &app,
        1,
        IdentityCommand::SetTag {
            guild_id: Id(10),
            tag: Some(new),
        },
    )
    .await);
    let worn = summary(&app, 2).await.style.tag.unwrap();
    assert_eq!((worn.text.as_str(), worn.glyph.as_str()), ("GGWP", "star"));
    // The account export includes it.
    // Leaving drops it at once, and the leave tells everyone.
    ok(&raw(&app, 2, Command::LeaveGuild { guild_id: Id(10) }).await);
    assert!(summary(&app, 2).await.style.tag.is_none());
    assert_eq!(scalar(&app, "SELECT COUNT(*) FROM tag_drops").await, 0);
    {
        let core = app.0.core.lock().await;
        assert!(ring(&core, 2)
            .iter()
            .rev()
            .find_map(|e| match e {
                Event::SelfUpdated(m) => Some(m.user.style.tag.is_none()),
                _ => None,
            })
            .unwrap());
    }
    // Removing the tag takes it off everyone.
    sql(&app, "INSERT INTO guild_members VALUES(10,2,'member',0)").await;
    ok(&run(
        &app,
        2,
        IdentityCommand::WearTag {
            guild_id: Some(Id(10)),
        },
    )
    .await);
    ok(&run(
        &app,
        1,
        IdentityCommand::SetTag {
            guild_id: Id(10),
            tag: None,
        },
    )
    .await);
    assert!(summary(&app, 2).await.style.tag.is_none());
    // Kicked members lose it too.
    ok(&run(&app, 1, set_tag("GG")).await);
    ok(&run(
        &app,
        2,
        IdentityCommand::WearTag {
            guild_id: Some(Id(10)),
        },
    )
    .await);
    ok(&raw(
        &app,
        1,
        Command::Launch(LaunchCommand::Moderation(ModerationCommand::Kick {
            guild_id: Id(10),
            user_id: Id(2),
            reason: None,
        })),
    )
    .await);
    assert!(summary(&app, 2).await.style.tag.is_none());
    // Wearing none.
    sql(&app, "INSERT INTO guild_members VALUES(10,2,'member',0)").await;
    ok(&run(
        &app,
        2,
        IdentityCommand::WearTag {
            guild_id: Some(Id(10)),
        },
    )
    .await);
    ok(&run(&app, 2, IdentityCommand::WearTag { guild_id: None }).await);
    assert!(summary(&app, 2).await.style.tag.is_none());
}

#[tokio::test]
async fn tag_cards_publish_only_the_public_face_and_offer_a_way_in() {
    let (_d, app) = fixture().await;
    second(&app).await;
    // No tag: nothing to show.
    assert_eq!(get(&app, 3, "/api/v1/tags/10").await.0, 404);
    ok(&run(&app, 1, set_tag("GG")).await);
    let card: TagCard = serde_json::from_slice(&get(&app, 3, "/api/v1/tags/10").await.1).unwrap();
    assert_eq!((card.name.as_str(), card.member_count), ("guild", 2));
    assert_eq!(card.join, TagJoin::None);
    let me: TagCard = serde_json::from_slice(&get(&app, 2, "/api/v1/tags/10").await.1).unwrap();
    assert_eq!(me.join, TagJoin::Member);
    // With an invite configured for the tag, anyone who sees it may join.
    let code = invite(&app).await;
    let mut t = tag("GG");
    t.invite_code = Some(code.clone());
    ok(&run(
        &app,
        1,
        IdentityCommand::SetTag {
            guild_id: Id(10),
            tag: Some(t),
        },
    )
    .await);
    let card: TagCard = serde_json::from_slice(&get(&app, 3, "/api/v1/tags/10").await.1).unwrap();
    assert_eq!(
        card.join,
        TagJoin::Invite {
            code: code.clone(),
            apply: false
        }
    );
    // Not to someone banned.
    sql(&app, "INSERT INTO guild_bans VALUES(10,4,1,NULL,0)").await;
    let card: TagCard = serde_json::from_slice(&get(&app, 4, "/api/v1/tags/10").await.1).unwrap();
    assert_eq!(card.join, TagJoin::None);
    // A revoked invite is no longer offered.
    ok(&raw(
        &app,
        1,
        Command::RevokeInvite {
            guild_id: Id(10),
            code,
        },
    )
    .await);
    let card: TagCard = serde_json::from_slice(&get(&app, 3, "/api/v1/tags/10").await.1).unwrap();
    assert_eq!(card.join, TagJoin::None);
    // The tagged server's icon is visible to the people who see its tag.
    upload(&app, 500, 1, "image/png").await;
    sql(&app, "UPDATE uploads SET expires_at=NULL WHERE id=500; UPDATE guilds SET icon_id=500 WHERE id=10").await;
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(file_access(&mut c, Id(3), Id(500)).await.unwrap());
}

#[tokio::test]
async fn servers_can_hide_other_servers_tags() {
    let (_d, app) = fixture().await;
    open(&app, &[1, 2]).await;
    assert_eq!(
        err(&run(
            &app,
            2,
            IdentityCommand::SetHideTags {
                guild_id: Id(10),
                hide: true
            }
        )
        .await)
        .code,
        ErrorCode::Forbidden
    );
    ok(&run(
        &app,
        1,
        IdentityCommand::SetHideTags {
            guild_id: Id(10),
            hide: true,
        },
    )
    .await);
    let g = guild(&mut app.0.pool.acquire().await.unwrap(), Id(2), Id(10))
        .await
        .unwrap();
    assert!(g.guild.look.unwrap().hide_tags);
    let core = app.0.core.lock().await;
    assert!(ring(&core, 2).iter().any(
        |e| matches!(e, Event::GuildUpsert(g) if g.guild.look.as_ref().is_some_and(|l| l.hide_tags))
    ));
}

async fn make_role(app: &App, name: &str, color: u32) -> Id {
    let r = match ok(&raw(
        app,
        1,
        Command::Launch(LaunchCommand::CreateRole {
            guild_id: Id(10),
            name: name.into(),
        }),
    )
    .await)
    {
        CommandResult::Role(r) => r,
        r => panic!("{r:?}"),
    };
    ok(&raw(
        app,
        1,
        Command::Launch(LaunchCommand::UpdateRole {
            role: Role { color, ..r.clone() },
        }),
    )
    .await);
    r.id
}
fn gradient(c2: u32) -> Option<RoleStyle> {
    Some(RoleStyle {
        paint: RolePaint::Gradient,
        color2: Some(c2),
        color3: None,
    })
}

#[tokio::test]
async fn role_styles_need_manage_roles_rank_and_readable_colors() {
    let (_d, app) = fixture().await;
    open(&app, &[1, 2]).await;
    let plain = make_role(&app, "Plain", 0).await;
    let blue = make_role(&app, "Blue", 0x7AA2FF).await;
    let style = |role_id, style| IdentityCommand::SetRoleStyle {
        guild_id: Id(10),
        role_id,
        style,
    };
    assert_eq!(
        err(&run(&app, 2, style(blue, gradient(0xFFC0E0))).await).code,
        ErrorCode::Forbidden
    );
    // A gradient starts at the role color.
    assert_eq!(
        err(&run(&app, 1, style(plain, gradient(0xFFC0E0))).await)
            .field
            .as_deref(),
        Some("style")
    );
    assert_eq!(
        err(&run(&app, 1, style(blue, gradient(0x101010))).await)
            .field
            .as_deref(),
        Some("style")
    );
    // @everyone has no style.
    assert_eq!(
        err(&run(&app, 1, style(Id(10), gradient(0xFFC0E0))).await).code,
        ErrorCode::InvalidRequest
    );
    ok(&run(
        &app,
        1,
        style(
            blue,
            Some(RoleStyle {
                paint: RolePaint::Gradient,
                color2: Some(0xFFC0E0),
                color3: Some(0xF2B84B),
            }),
        ),
    )
    .await);
    ok(&run(
        &app,
        1,
        style(
            plain,
            Some(RoleStyle {
                paint: RolePaint::Holographic,
                color2: None,
                color3: None,
            }),
        ),
    )
    .await);
    let roles = crate::permissions::roles(&mut app.0.pool.acquire().await.unwrap(), Id(10))
        .await
        .unwrap();
    let b = roles.iter().find(|r| r.id == blue).unwrap();
    assert_eq!(
        b.look.as_ref().unwrap().style.unwrap().color3,
        Some(0xF2B84B)
    );
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM audit_log WHERE action='role_update' AND changes LIKE '%\"style\"%'").await,
        2
    );
    // Members receive it with the roles; UpdateRole leaves it alone.
    ok(&raw(
        &app,
        1,
        Command::Launch(LaunchCommand::UpdateRole {
            role: Role {
                name: "Bluer".into(),
                look: None,
                ..b.clone()
            },
        }),
    )
    .await);
    let roles = crate::permissions::roles(&mut app.0.pool.acquire().await.unwrap(), Id(10))
        .await
        .unwrap();
    assert!(roles.iter().find(|r| r.id == blue).unwrap().look.is_some());
    let core = app.0.core.lock().await;
    assert!(ring(&core, 2).iter().any(
        |e| matches!(e, Event::RolesUpdated { roles, .. } if roles.iter().any(|r| r.look.is_some()))
    ));
}

#[tokio::test]
async fn role_icons_are_emoji_server_emoji_or_validated_pictures_for_members() {
    let (_d, app) = fixture().await;
    second(&app).await;
    let r = make_role(&app, "Star", 0x7AA2FF).await;
    let icon = |icon| IdentityCommand::SetRoleIcon {
        guild_id: Id(10),
        role_id: r,
        icon,
    };
    assert_eq!(
        err(&run(
            &app,
            1,
            icon(Some(RoleIconRef::Emoji {
                emoji: "abc".into()
            }))
        )
        .await)
        .code,
        ErrorCode::InvalidRequest
    );
    ok(&run(
        &app,
        1,
        icon(Some(RoleIconRef::Emoji {
            emoji: "⭐".into()
        })),
    )
    .await);
    // A custom emoji must be this server's.
    sql(&app, "INSERT INTO guild_emoji(id,guild_id,name,animated,content_type,data,preview) VALUES(900,20,'theirs',0,'image/png',x'00',x'00'),(901,10,'ours',0,'image/png',x'00',x'00')").await;
    assert_eq!(
        err(&run(
            &app,
            1,
            icon(Some(RoleIconRef::Custom { emoji_id: Id(900) }))
        )
        .await)
        .field
        .as_deref(),
        Some("icon")
    );
    ok(&run(
        &app,
        1,
        icon(Some(RoleIconRef::Custom { emoji_id: Id(901) })),
    )
    .await);
    // An uploaded picture is decoded and fitted into 64x64.
    let mut png = std::io::Cursor::new(vec![]);
    image::RgbaImage::from_pixel(200, 100, image::Rgba([10, 200, 30, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let path = format!("/api/v1/guilds/10/roles/{}/icon", r.0);
    assert_eq!(
        request(&app, 2, "POST", &path, png.get_ref().clone())
            .await
            .0,
        403
    );
    assert_eq!(
        request(&app, 1, "POST", &path, b"GIF89a nope".to_vec())
            .await
            .0,
        415
    );
    let (s, body) = request(&app, 1, "POST", &path, png.get_ref().clone()).await;
    assert_eq!(s, 200, "{}", String::from_utf8_lossy(&body));
    let look: RoleLook = serde_json::from_slice(&body).unwrap();
    assert_eq!(look.icon, Some(RoleIcon::image(1)));
    let (s, body) = get(&app, 2, &format!("{path}?v=1")).await;
    assert_eq!(s, 200);
    let back = image::load_from_memory(&body).unwrap();
    assert_eq!((back.width(), back.height()), (64, 64));
    // Only for members.
    assert_eq!(get(&app, 3, &path).await.0, 404);
    assert!(role_icon_check_too_large());
    // Removing it removes the picture.
    ok(&run(&app, 1, icon(None)).await);
    assert_eq!(get(&app, 2, &path).await.0, 404);
}
fn role_icon_check_too_large() -> bool {
    looks::role_icon_rendition(&vec![0u8; MAX_ROLE_ICON_BYTES + 1])
        .is_err_and(|e| e.0.code == ErrorCode::PayloadTooLarge)
}

#[tokio::test]
async fn server_images_are_owned_uploads_kept_and_shown_to_members() {
    let (_d, app) = fixture().await;
    second(&app).await;
    open(&app, &[1, 2]).await;
    upload(&app, 600, 1, "image/gif").await;
    upload(&app, 601, 2, "image/png").await;
    upload(&app, 602, 1, "image/jpeg").await;
    let set = |slot, image| IdentityCommand::SetImage {
        guild_id: Id(10),
        slot,
        image,
    };
    let r = |id| {
        Some(ImageRef {
            id: Id(id),
            crop: Some(Crop {
                zoom: 150,
                x: 20,
                y: 80,
            }),
        })
    };
    assert_eq!(
        err(&run(&app, 2, set(GuildImageSlot::Banner, r(601))).await).code,
        ErrorCode::Forbidden
    );
    // Someone else's upload cannot be used.
    assert_eq!(
        err(&run(&app, 1, set(GuildImageSlot::Banner, r(601))).await).code,
        ErrorCode::InvalidAttachment
    );
    ok(&run(&app, 1, set(GuildImageSlot::Banner, r(600))).await);
    ok(&run(&app, 1, set(GuildImageSlot::Icon, r(600))).await);
    ok(&run(&app, 1, set(GuildImageSlot::InviteBackground, r(602))).await);
    let g = guild(&mut app.0.pool.acquire().await.unwrap(), Id(2), Id(10))
        .await
        .unwrap();
    let look = g.guild.look.unwrap();
    assert_eq!(g.guild.icon_id, Some(Id(600)));
    // A GIF counts as moving when this process did not see it uploaded.
    assert!(look.banner.as_ref().unwrap().animated);
    assert_eq!(look.icon.as_ref().unwrap().crop.unwrap().zoom, 150);
    // Upload collection keeps them.
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(gc_uploads(&mut c).await.unwrap().is_empty());
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM uploads WHERE id IN(600,602) AND expires_at IS NULL"
        )
        .await,
        2
    );
    // Members may download them; others may not, and the invite background
    // stays with the invite.
    assert!(file_access(&mut c, Id(2), Id(602)).await.unwrap());
    assert!(!file_access(&mut c, Id(3), Id(602)).await.unwrap());
    assert_eq!(scalar(&app, "SELECT COUNT(*) FROM audit_log WHERE action='guild_update' AND changes LIKE '%banner%'").await, 1);
    // An invite holder sees the card with all three.
    let code = invite(&app).await;
    let (s, body) = get(&app, 3, &format!("/api/v1/invites/{code}/card")).await;
    assert_eq!(s, 200);
    let card: InviteCard = serde_json::from_slice(&body).unwrap();
    assert_eq!(card.guild_id, Id(10));
    assert!(card.banner.is_some() && card.background.is_some() && card.icon.is_some());
    assert!(!card.joined && !card.vanity);
    // Nothing for a code that does not work.
    assert_eq!(
        get(&app, 3, "/api/v1/invites/abcdefghijklmnopqrstuv/card")
            .await
            .0,
        404
    );
    assert_eq!(
        get(&app, 3, "/api/v1/invites/abcdefghijklmnopqrstuv/art/banner")
            .await
            .0,
        404
    );
    // Removing an image frees it.
    ok(&run(&app, 1, set(GuildImageSlot::InviteBackground, None)).await);
    assert_eq!(
        scalar(&app, "SELECT COUNT(*) FROM guild_uploads WHERE guild_id=10").await,
        2
    );
}

#[tokio::test]
async fn vanity_links_are_unique_held_rate_limited_and_join_like_invites() {
    let (_d, app) = fixture().await;
    second(&app).await;
    let set = |g: u64, code: Option<&str>| IdentityCommand::SetVanity {
        guild_id: Id(g),
        code: code.map(str::to_owned),
    };
    assert_eq!(
        err(&run(&app, 2, set(10, Some("sticks"))).await).code,
        ErrorCode::Forbidden
    );
    for bad in ["ab", "-ab", "a b", "a--b", "admin"] {
        assert!(
            matches!(
                err(&run(&app, 1, set(10, Some(bad))).await)
                    .field
                    .as_deref(),
                Some("vanity")
            ),
            "{bad}"
        );
    }
    ok(&run(&app, 1, set(10, Some("Sticks"))).await);
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM vanity_codes WHERE code='sticks' AND guild_id=10"
        )
        .await,
        1
    );
    assert_eq!(
        err(&run(&app, 3, set(20, Some("STICKS"))).await).code,
        ErrorCode::AlreadyExists
    );
    // It works as an invite, in any case, through every invite path.
    let preview: Snapshot<InvitePreview> =
        serde_json::from_slice(&get(&app, 3, "/api/v1/invites/STICKS/preview").await.1).unwrap();
    assert_eq!(preview.data.name, "guild");
    let card: InviteCard =
        serde_json::from_slice(&get(&app, 3, "/api/v1/invites/sticks/card").await.1).unwrap();
    assert!(card.vanity);
    ok(&raw(
        &app,
        3,
        Command::JoinGuild {
            invite_code: "Sticks".into(),
        },
    )
    .await);
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM guild_members WHERE guild_id=10 AND user_id=3"
        )
        .await,
        1
    );
    // Raid pauses apply to it.
    ok(&raw(
        &app,
        1,
        Command::Launch(LaunchCommand::Moderation(ModerationCommand::PauseInvites {
            guild_id: Id(10),
            seconds: 3600,
        })),
    )
    .await);
    let e = err(&raw(
        &app,
        4,
        Command::JoinGuild {
            invite_code: "sticks".into(),
        },
    )
    .await);
    assert_eq!(e.field.as_deref(), Some("invites_paused"));
    ok(&raw(
        &app,
        1,
        Command::Launch(LaunchCommand::Moderation(ModerationCommand::PauseInvites {
            guild_id: Id(10),
            seconds: 0,
        })),
    )
    .await);
    // The invite list cannot revoke it.
    let e = err(&raw(
        &app,
        1,
        Command::RevokeInvite {
            guild_id: Id(10),
            code: "sticks".into(),
        },
    )
    .await);
    assert_eq!(e.field.as_deref(), Some("vanity"));
    // Admins see the use count; it keeps counting past the invite table's cap.
    sql(&app, "UPDATE invites SET uses=950 WHERE code='sticks'").await;
    ok(&raw(
        &app,
        4,
        Command::JoinGuild {
            invite_code: "sticks".into(),
        },
    )
    .await);
    let admin: IdentityAdmin =
        serde_json::from_slice(&get(&app, 1, "/api/v1/guilds/10/identity").await.1).unwrap();
    assert_eq!(admin.vanity.as_ref().unwrap().uses, 951);
    assert_eq!(get(&app, 2, "/api/v1/guilds/10/identity").await.0, 403);
    // Released: held 30 days for everyone else, and the shareable link redirects.
    ok(&run(&app, 1, set(10, None)).await);
    assert_eq!(get(&app, 3, "/api/v1/invites/sticks/preview").await.0, 404);
    assert_eq!(
        err(&run(&app, 3, set(20, Some("sticks"))).await).code,
        ErrorCode::Conflict
    );
    // The server that released it may take it back; three changes an hour.
    ok(&run(&app, 1, set(10, Some("sticks"))).await);
    let e = err(&run(&app, 1, set(10, None)).await);
    assert_eq!(e.code, ErrorCode::RateLimited);
    let admin: IdentityAdmin =
        serde_json::from_slice(&get(&app, 1, "/api/v1/guilds/10/identity").await.1).unwrap();
    assert!(admin.vanity_wait_ms > 0);
    assert_eq!(
        scalar(
            &app,
            "SELECT COUNT(*) FROM audit_log WHERE action='vanity_update'"
        )
        .await,
        3
    );
    sql(
        &app,
        "UPDATE audit_log SET created_at=0 WHERE action='vanity_update'",
    )
    .await;
    // After the hold, another server may claim a released code.
    ok(&run(&app, 1, set(10, Some("sticks-two"))).await);
    sql(
        &app,
        "UPDATE vanity_codes SET held_until=0 WHERE code='sticks'",
    )
    .await;
    ok(&run(&app, 3, set(20, Some("sticks"))).await);
    // A deleted server's code is held too.
    ok(&raw(&app, 1, Command::DeleteGuild { guild_id: Id(10) }).await);
    assert!(
        scalar(
            &app,
            "SELECT held_until FROM vanity_codes WHERE code='sticks-two'"
        )
        .await
            > now() as i64
    );
    let (s, location) = get(&app, 3, "/invite/sticks").await;
    assert_eq!(
        (s, String::from_utf8(location).unwrap().as_str()),
        (303, "/?invite=sticks")
    );
}

#[tokio::test]
async fn the_worn_tag_is_exported_and_erased_with_the_account() {
    let (_d, app) = fixture().await;
    ok(&run(&app, 1, set_tag("GG")).await);
    ok(&run(
        &app,
        2,
        IdentityCommand::WearTag {
            guild_id: Some(Id(10)),
        },
    )
    .await);
    let r = sqlx::query("SELECT tag FROM users WHERE id=2")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(export_row(&r)["text"], "GG");
    let mut c = app.0.pool.acquire().await.unwrap();
    erase(&mut c, Id(2)).await.unwrap();
    assert!(summary(&app, 2).await.style.tag.is_none());
}
