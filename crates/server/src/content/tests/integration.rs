//! Scheduled messages carry files (`docs/features/integration.md`): held from
//! scheduling to delivery, released on cancel, counted in the owner's quota.
use super::*;

fn schedule_files(ch: u64, text: &str, at: u64, files: Vec<Id>) -> ContentCommand {
    ContentCommand::ScheduleMessage {
        channel_id: Id(ch),
        content: text.into(),
        mention_ids: vec![],
        send_at: at,
        attachment_ids: files,
        attachments: vec![],
    }
}
async fn count(app: &App, q: &'static str) -> i64 {
    sqlx::query_scalar(q).fetch_one(&app.0.pool).await.unwrap()
}
/// What `commands::execute` does after every command: collect unattached uploads.
async fn collect(app: &App) -> Vec<String> {
    let mut c = app.0.pool.acquire().await.unwrap();
    db::gc_uploads(&mut c).await.unwrap()
}

#[tokio::test]
async fn scheduled_messages_hold_their_files_until_sent_or_cancelled() {
    let (_dir, app) = world().await;
    for f in 501..=504 {
        upload(&app, f, 2, 11, "image/png").await;
    }
    // Someone else's or another channel's upload is refused, as when sending.
    upload(&app, 505, 4, 11, "image/png").await;
    upload(&app, 506, 2, 21, "image/png").await;
    let later = now() + 3_600_000;
    for (n, bad) in [Id(505), Id(506), Id(999)].into_iter().enumerate() {
        assert_eq!(
            code(
                &content(
                    &app,
                    2,
                    1 + n as u64,
                    schedule_files(11, "x", later, vec![bad])
                )
                .await
            ),
            Some(ErrorCode::InvalidAttachment)
        );
    }
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM scheduled_messages").await,
        0
    );
    // Files alone are enough; the list shows them.
    let s = match ok(content(
        &app,
        2,
        10,
        schedule_files(11, "", later, vec![Id(501), Id(502)]),
    )
    .await)
    {
        CommandResult::Content(ContentResult::Scheduled(s)) => s,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        s.attachments.iter().map(|a| a.id).collect::<Vec<_>>(),
        vec![Id(501), Id(502)]
    );
    // Held: no expiry, never collected, still counted against the owner.
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM uploads WHERE id IN (501,502) AND expires_at IS NULL"
        )
        .await,
        2
    );
    assert!(collect(&app).await.is_empty());
    assert_eq!(
        count(
            &app,
            "SELECT COALESCE(SUM(byte_size),0) FROM uploads WHERE owner_id=2"
        )
        .await,
        4 * 5
    );
    // A held file cannot be sent or scheduled a second time.
    assert_eq!(
        code(
            &run(
                &app,
                2,
                11,
                Command::SendMessage(draft(11, "dup", vec![Id(501)]))
            )
            .await
        ),
        Some(ErrorCode::InvalidAttachment)
    );
    assert_eq!(
        code(&content(&app, 2, 12, schedule_files(11, "dup", later, vec![Id(502)])).await),
        Some(ErrorCode::InvalidAttachment)
    );
    // Editing replaces the list: 502 is released, 503 is held; omitting the
    // list keeps it.
    ok(content(
        &app,
        2,
        13,
        ContentCommand::UpdateScheduled {
            id: s.id,
            content: "with files".into(),
            mention_ids: vec![],
            send_at: later,
            attachment_ids: Some(vec![Id(501), Id(503)]),
        },
    )
    .await);
    // The command's own collection removed the released file.
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM uploads WHERE id=502").await,
        0
    );
    ok(content(
        &app,
        2,
        14,
        ContentCommand::UpdateScheduled {
            id: s.id,
            content: "still with files".into(),
            mention_ids: vec![],
            send_at: later,
            attachment_ids: None,
        },
    )
    .await);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM scheduled_attachments").await,
        2
    );
    // Delivery attaches them to the message, in order.
    sql(&app, "UPDATE scheduled_messages SET send_at=0").await;
    scheduled::deliver_due(&app).await.unwrap();
    let m: i64 = count(
        &app,
        "SELECT id FROM messages WHERE content='still with files' AND channel_id=11",
    )
    .await;
    let attached: Vec<i64> = sqlx::query_scalar(
        "SELECT attachment_id FROM message_attachments WHERE message_id=? ORDER BY position",
    )
    .bind(m)
    .fetch_all(&app.0.pool)
    .await
    .unwrap();
    assert_eq!(attached, vec![501, 503]);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM scheduled_attachments").await,
        0
    );
    assert!(collect(&app).await.is_empty());

    // Cancelling releases the files to the ordinary collection.
    let s = match ok(content(
        &app,
        2,
        20,
        schedule_files(11, "never", later, vec![Id(504)]),
    )
    .await)
    {
        CommandResult::Content(ContentResult::Scheduled(s)) => s,
        other => panic!("{other:?}"),
    };
    ok(content(&app, 2, 21, ContentCommand::CancelScheduled { id: s.id }).await);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM uploads WHERE id=504").await,
        0
    );

    // A refused delivery keeps its files held for a reschedule.
    upload(&app, 507, 2, 11, "image/png").await;
    ok(content(
        &app,
        2,
        30,
        schedule_files(11, "refused", later, vec![Id(507)]),
    )
    .await);
    sql(
        &app,
        "INSERT INTO permission_overwrites VALUES(11,'member',2,0,2)",
    )
    .await;
    sql(&app, "UPDATE scheduled_messages SET send_at=0").await;
    scheduled::deliver_due(&app).await.unwrap();
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM scheduled_attachments WHERE attachment_id=507"
        )
        .await,
        1
    );
    assert!(collect(&app).await.is_empty());
}

/// Alt text and spoiler marks survive scheduling (task `qa2`): stored with the
/// files when scheduled, listed with them, and on the delivered message.
#[tokio::test]
async fn scheduled_files_keep_their_alt_text_and_spoiler_marks() {
    let (_dir, app) = world().await;
    for f in [511, 512] {
        upload(&app, f, 2, 11, "image/png").await;
    }
    let meta = |description: &str, spoiler| AttachmentMeta {
        description: (!description.is_empty()).then(|| description.into()),
        spoiler,
        ..Default::default()
    };
    let mut cmd = schedule_files(11, "look", now() + 3_600_000, vec![Id(511), Id(512)]);
    if let ContentCommand::ScheduleMessage { attachments, .. } = &mut cmd {
        *attachments = vec![
            AttachmentMetaPatch {
                attachment_id: Id(511),
                meta: meta("A red square", false),
            },
            AttachmentMetaPatch {
                attachment_id: Id(512),
                meta: meta("", true),
            },
        ];
    }
    let s = match ok(content(&app, 2, 1, cmd).await) {
        CommandResult::Content(ContentResult::Scheduled(s)) => s,
        other => panic!("{other:?}"),
    };
    let shown = |files: &[Attachment]| {
        files
            .iter()
            .map(|a| (a.id, a.meta.description.clone(), a.meta.spoiler))
            .collect::<Vec<_>>()
    };
    let expected = vec![
        (Id(511), Some("A red square".to_owned()), false),
        (Id(512), None, true),
    ];
    assert_eq!(shown(&s.attachments), expected);
    // A patch for a file that is not in the message is refused.
    let mut stray = schedule_files(11, "x", now() + 3_600_000, vec![]);
    if let ContentCommand::ScheduleMessage { attachments, .. } = &mut stray {
        *attachments = vec![AttachmentMetaPatch {
            attachment_id: Id(511),
            meta: meta("nope", false),
        }];
    }
    assert!(code(&content(&app, 2, 2, stray).await).is_some());
    let sent = message_of(ok(content(
        &app,
        2,
        3,
        ContentCommand::SendScheduledNow { id: s.id },
    )
    .await));
    assert_eq!(shown(&sent.attachments), expected);
}
