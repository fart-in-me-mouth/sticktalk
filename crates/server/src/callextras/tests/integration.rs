//! Soundboard and voice quality changes in the audit log, AutoMod on sound
//! names, and timeouts (`docs/features/integration.md`).
use super::*;

async fn audit(app: &App, action: AuditAction) -> Vec<(i64, String)> {
    sqlx::query("SELECT actor_id,changes FROM audit_log WHERE guild_id=10 AND action=? ORDER BY id")
        .bind(action.name())
        .fetch_all(&app.0.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect()
}

#[tokio::test]
async fn sound_changes_and_bitrate_are_audited_and_names_are_screened() {
    let (_dir, app) = fixture().await;
    world(&app).await;
    let owner = token(&app, 1).await;
    let member = token(&app, 2).await;
    // Member 2 manages the soundboard without Manage Server, so AutoMod applies.
    sqlx::query("UPDATE roles SET permissions=permissions|? WHERE guild_id=10 AND id=10")
        .bind(Permissions::MANAGE_SOUNDBOARD.bits() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let rule = AutoModRule {
        id: Id(0),
        guild_id: Id(10),
        name: "No swearing".into(),
        enabled: true,
        trigger: AutoModTrigger::Keyword {
            keywords: vec!["heck".into()],
            allow: vec![],
        },
        actions: AutoModActions {
            block: true,
            message: None,
            alert_channel_id: None,
            timeout_seconds: 0,
        },
        exempt_roles: vec![],
        exempt_channels: vec![],
    };
    sqlx::query("INSERT INTO automod_rules(guild_id,rule) VALUES(10,?)")
        .bind(serde_json::to_string(&rule).unwrap())
        .execute(&app.0.pool)
        .await
        .unwrap();
    let blocked = add_sound(&app, &member, 10, "heck").await;
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
    assert_eq!(json(blocked).await["field"], "automod");
    assert_eq!(audit(&app, AuditAction::AutomodBlock).await.len(), 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM soundboard_sounds")
            .fetch_one(&app.0.pool)
            .await
            .unwrap(),
        0
    );
    // The owner (Manage Server) is exempt, as for messages.
    let horn = sound_id(add_sound(&app, &member, 10, "horn").await).await;
    sound_id(add_sound(&app, &owner, 10, "heck").await).await;
    let created = audit(&app, AuditAction::SoundCreate).await;
    assert_eq!(created.len(), 2);
    assert_eq!(created[0].0, 2);
    // Renaming is screened too, and logged with the change.
    let rename = |name: &str| serde_json::to_vec(&serde_json::json!({ "name": name })).unwrap();
    let path = format!("/api/v1/soundboard/sounds/{}", horn.0);
    let r = request(
        &app,
        &member,
        "PATCH",
        &path,
        Some(rename("heck horn")),
        true,
    )
    .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = request(
        &app,
        &member,
        "PATCH",
        &path,
        Some(rename("air horn")),
        true,
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let updated = audit(&app, AuditAction::SoundUpdate).await;
    assert_eq!(updated.len(), 1);
    assert!(updated[0].1.contains("air horn"), "{}", updated[0].1);
    let r = request(&app, &member, "DELETE", &path, None, false).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(audit(&app, AuditAction::SoundDelete).await.len(), 1);

    // A timed-out member loses Manage Soundboard (and Use Soundboard) with everything else.
    sqlx::query("INSERT INTO member_timeouts(guild_id,user_id,until,actor_id) VALUES(10,2,?,1)")
        .bind((now() + 600_000) as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        add_sound(&app, &member, 10, "bell").await.status(),
        StatusCode::FORBIDDEN
    );
    let mut c = app.0.pool.acquire().await.unwrap();
    assert!(!permissions::evaluate(&mut c, Id(2), Id(10), Some(VOICE))
        .await
        .unwrap()
        .contains(Permissions::USE_SOUNDBOARD));
    drop(c);

    // A voice channel's bitrate change is a channel update in the log.
    let quality = format!("/api/v1/channels/{}/voice-quality", VOICE.0);
    let body = serde_json::to_vec(&serde_json::json!({ "bitrate": 128_000 })).unwrap();
    let r = request(&app, &owner, "PUT", &quality, Some(body.clone()), true).await;
    assert_eq!(r.status(), StatusCode::OK);
    // Setting the same value again changes nothing and logs nothing.
    request(&app, &owner, "PUT", &quality, Some(body), true).await;
    let e = audit(&app, AuditAction::ChannelUpdate).await;
    assert_eq!(e.len(), 1);
    assert!(
        e[0].1.contains("64 kbps") && e[0].1.contains("128 kbps"),
        "{}",
        e[0].1
    );
}
