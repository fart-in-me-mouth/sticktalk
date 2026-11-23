//! Independent audit fixtures: seed only storage, exercise the real services.
use crate::{commands, db, state::*, App, Config};
use sqlx::Row;
use std::sync::Arc;
use sticktalk_proto::*;

pub(crate) async fn fixture() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(Config {
        operations: crate::OperationsConfig {
            min_free_disk_gb: 0,
            ..Default::default()
        },
        browser: Default::default(),
        bind: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.path().into(),
        public_url: "http://localhost".into(),
        max_upload_mb: 25,
        max_total_upload_mb: 10240,
        worker_id: 0,
        trusted_proxy: "127.0.0.1".parse().unwrap(),
        google_client_id: None,
        mock_google_jwks: None,
        mock_google_issuer: None,
        gifs: Default::default(),
        voice: crate::voice::VoiceOptions {
            voice_bind_ip: "127.0.0.1".parse().unwrap(),
            voice_advertised_ip: Some("127.0.0.1".parse().unwrap()),
            ..Default::default()
        },
    })
    .await
    .unwrap();
    for u in 1..=4_i64 {
        sqlx::query("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(?,?,?,'online',?)")
            .bind(u).bind(format!("user{u}")).bind(&app.0.dummy_hash).bind(now() as i64).execute(&app.0.pool).await.unwrap();
        sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
            .bind(vec![u as u8; 32])
            .bind(u)
            .bind(now() as i64)
            .bind((now() + 86_400_000) as i64)
            .bind(now() as i64)
            .execute(&app.0.pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO guilds VALUES(10,1,'guild',NULL,0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO guild_members VALUES(10,1,'owner',0),(10,2,'member',0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(11,'text',10,'general',0,0)").execute(&app.0.pool).await.unwrap();
    sqlx::query("INSERT INTO read_states(user_id,channel_id) VALUES(1,11),(2,11)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    (dir, app)
}
async fn command(app: &App, core: &mut Core, u: u64, nonce: u64, cmd: Command) -> CommandResponse {
    core.limits.clear(); // Isolate authorization from intentionally independent rate policy.
    commands::execute(
        app,
        core,
        &[u as u8; 32],
        Id(u),
        CommandRequest {
            nonce,
            command: cmd,
        },
    )
    .await
    .unwrap()
}
fn events(core: &Core, key: &[u8]) -> Vec<Event> {
    core.streams[key]
        .ring
        .iter()
        .filter_map(
            |(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).unwrap() {
                ServerMsg::Event(e) => Some(e.event),
                _ => None,
            },
        )
        .collect()
}

#[tokio::test]
async fn rest_streams_are_offline_and_metadata_is_bounded() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    core.cursor(&[1; 32], Id(1));
    let mut c = app.0.pool.acquire().await.unwrap();
    assert_eq!(
        db::presence(&mut c, &core, Id(2), Id(1))
            .await
            .unwrap()
            .status,
        PresenceStatus::Offline
    );
    for n in 0..5000_u64 {
        core.cursor(&n.to_be_bytes(), Id(1));
    }
    assert_eq!(core.streams.len(), 4096);
}

#[tokio::test]
async fn readable_historical_authors_can_be_resolved_but_not_enumerated() {
    let (_dir, app) = fixture().await;
    let mut c = app.0.pool.acquire().await.unwrap();
    sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(12,11,3,'old member',0)").execute(&mut *c).await.unwrap();
    db::profile_access(&mut c, Id(2), Id(3)).await.unwrap();
    assert_eq!(
        db::profile_access(&mut c, Id(4), Id(3))
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::NotFound
    );
    sqlx::query("DELETE FROM messages WHERE id=12")
        .execute(&mut *c)
        .await
        .unwrap();
    assert_eq!(
        db::profile_access(&mut c, Id(2), Id(3))
            .await
            .unwrap_err()
            .0
            .code,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn read_counts_never_silently_wrap() {
    let (_dir, app) = fixture().await;
    sqlx::query("UPDATE read_states SET unread_count=4294967296 WHERE user_id=1 AND channel_id=11")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let mut c = app.0.pool.acquire().await.unwrap();
    assert_eq!(
        db::read(&mut c, Id(1), Id(11)).await.unwrap_err().0.code,
        ErrorCode::LimitExceeded
    );
}

#[tokio::test]
async fn profile_events_obey_each_sessions_projection() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    core.cursor(&[1; 32], Id(1));
    core.cursor(&[2; 32], Id(2));
    core.cursor(&[9; 32], Id(2));
    core.streams.get_mut(&vec![9; 32]).unwrap().guild = Some(Id(10));
    let response = command(
        &app,
        &mut core,
        1,
        1,
        Command::UpdateProfile(ProfilePatch {
            display_name: Patch::Set("new name".into()),
            ..Default::default()
        }),
    )
    .await;
    assert!(matches!(response.outcome, CommandOutcome::Ok(_)));
    assert!(!events(&core, &[2; 32])
        .iter()
        .any(|e| matches!(e, Event::UserUpdated(_))));
    assert!(events(&core, &[9; 32])
        .iter()
        .any(|e| matches!(e, Event::UserUpdated(_))));
}

#[tokio::test]
async fn presence_visibility_changes_emit_snapshots_and_offline() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    for u in 1..=3_u64 {
        core.cursor(&[u as u8; 32], Id(u));
        core.streams
            .get_mut(&vec![u as u8; 32])
            .unwrap()
            .ever_connected = true;
    }
    sqlx::query("INSERT INTO relationships VALUES(1,2,'friend',0),(2,1,'friend',0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    command(
        &app,
        &mut core,
        1,
        1,
        Command::RemoveFriend { user_id: Id(2) },
    )
    .await;
    assert!(events(&core, &[2;32]).iter().any(|e| matches!(e,Event::PresenceUpdated(p) if p.user_id==Id(1)&&p.status==PresenceStatus::Offline)));
    // Existing pair may reopen without friendship and must get initial presence.
    sqlx::query("INSERT INTO channels(id,kind,position,created_at) VALUES(20,'direct',0,0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO dm_pairs VALUES(1,2,20)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO dm_members VALUES(20,1,0,0),(20,2,0,0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO read_states(user_id,channel_id) VALUES(1,20),(2,20)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    command(&app, &mut core, 1, 2, Command::OpenDm { user_id: Id(2) }).await;
    assert!(events(&core, &[1;32]).iter().any(|e| matches!(e,Event::PresenceUpdated(p) if p.user_id==Id(2)&&p.status==PresenceStatus::Online)));
    core.streams.get_mut(&vec![1; 32]).unwrap().clear_replay();
    command(
        &app,
        &mut core,
        1,
        3,
        Command::CloseDm { channel_id: Id(20) },
    )
    .await;
    assert!(events(&core, &[1;32]).iter().any(|e| matches!(e,Event::PresenceUpdated(p) if p.user_id==Id(2)&&p.status==PresenceStatus::Offline)));
    // Blocking a custom-only interest must hide presence on those streams too.
    core.streams.get_mut(&vec![1; 32]).unwrap().interests = vec![Id(2)];
    core.streams.get_mut(&vec![1; 32]).unwrap().clear_replay();
    command(&app, &mut core, 1, 4, Command::BlockUser { user_id: Id(2) }).await;
    assert!(events(&core, &[1;32]).iter().any(|e| matches!(e,Event::PresenceUpdated(p) if p.user_id==Id(2)&&p.status==PresenceStatus::Offline)));
    assert!(!events(&core, &[3; 32])
        .iter()
        .any(|e| matches!(e, Event::PresenceUpdated(_))));
}

#[tokio::test]
async fn every_channel_mutation_rechecks_current_membership() {
    let (_dir, app) = fixture().await;
    sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(12,11,1,'private',0)").execute(&app.0.pool).await.unwrap();
    let mut core = app.0.core.lock().await;
    let commands = vec![
        Command::SendMessage(MessageDraft {
            channel_id: Id(11),
            content: "intrusion".into(),
            reply_to: None,
            attachment_ids: vec![],
            mention_ids: vec![],
        }),
        Command::EditMessage(MessageEdit {
            channel_id: Id(11),
            message_id: Id(12),
            content: "overwritten".into(),
            mention_ids: vec![],
        }),
        Command::DeleteMessage {
            channel_id: Id(11),
            message_id: Id(12),
        },
        Command::AddReaction {
            channel_id: Id(11),
            message_id: Id(12),
            emoji: "👍".into(),
        },
        Command::RemoveReaction {
            channel_id: Id(11),
            message_id: Id(12),
            emoji: "👍".into(),
        },
        Command::Typing { channel_id: Id(11) },
        Command::MarkRead {
            channel_id: Id(11),
            message_id: Id(12),
        },
        Command::UpdateChannel {
            channel_id: Id(11),
            patch: ChannelPatch::default(),
        },
        Command::DeleteChannel { channel_id: Id(11) },
        Command::SubscribeGuild {
            guild_id: Some(Id(10)),
            presence_user_ids: vec![Id(1)],
        },
    ];
    for (i, cmd) in commands.into_iter().enumerate() {
        let r = command(&app, &mut core, 3, i as u64 + 1, cmd.clone()).await;
        assert!(
            matches!(
                r.outcome,
                CommandOutcome::Err(ApiError {
                    code: ErrorCode::NotFound,
                    ..
                })
            ),
            "{cmd:?}: {:?}",
            r.outcome
        );
    }
    let row = sqlx::query("SELECT content,deleted_at FROM messages WHERE id=12")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("content"), "private");
    assert!(row.get::<Option<i64>, _>("deleted_at").is_none());
}

#[tokio::test]
async fn http_admission_and_global_upload_reservation_are_bounded() {
    let (_dir, mut app) = fixture().await;
    Arc::get_mut(&mut app.0).unwrap().config.max_total_upload_mb = 1;
    let token = token();
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=1")
        .bind(hash(token.as_bytes()))
        .execute(&app.0.pool)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let serving = app.clone();
    let task = tokio::spawn(async move {
        serving.serve(listener).await.unwrap();
    });
    let http = reqwest::Client::new();
    for username in ["user1", "not_registered", "!invalid!"] {
        let response = http
            .post(format!("{url}/api/v1/auth/login"))
            .json(&LoginRequest {
                username: username.into(),
                password: "wrong password with length".into(),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
        assert_eq!(
            response.json::<ApiError>().await.unwrap().code,
            ErrorCode::InvalidCredentials
        );
    }
    let slots = app
        .0
        .requests
        .clone()
        .acquire_many_owned(128)
        .await
        .unwrap();
    assert_eq!(
        http.get(format!("{url}/api/v1/health"))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    drop(slots);
    assert_eq!(
        http.get(format!("{url}/api/v1/health"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let slots = app.0.uploads.clone().acquire_many_owned(16).await.unwrap();
    assert_eq!(
        http.post(format!("{url}/api/v1/uploads"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    drop(slots);
    let body = reqwest::multipart::Form::new()
        .text("purpose", "avatar")
        .part(
            "file",
            reqwest::multipart::Part::bytes(vec![0; 1024 * 1024]).file_name("file"),
        );
    let response = http
        .post(format!("{url}/api/v1/uploads"))
        .bearer_auth(&token)
        .multipart(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(
        response.json::<ApiError>().await.unwrap().code,
        ErrorCode::LimitExceeded
    );
    // An error response is not a barrier for UploadGuard's cancellation-safe
    // asynchronous Drop cleanup. Await the invariant, not an arbitrary sleep;
    // a real leaked reservation still fails this bounded check.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_reservations")
                .fetch_one(&app.0.pool)
                .await
                .unwrap();
            if count == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("failed upload reservation was not released");
    app.shutdown();
    task.await.unwrap();
}

#[tokio::test]
async fn stalled_http_body_has_a_deadline_and_releases_admission() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_dir, app) = fixture().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let serving = app.clone();
    let task = tokio::spawn(async move {
        serving.serve(listener).await.unwrap();
    });
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    client.write_all(b"POST /api/v1/auth/login HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{").await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(18),
        client.read_to_end(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 503"), "{response}");
    assert!(response.contains("unavailable"));
    assert_eq!(app.0.requests.available_permits(), 128);
    app.shutdown();
    task.await.unwrap();
}

#[test]
fn deployment_security_invariants() {
    let install = include_str!("../../../deploy/install.sh");
    assert!(install.contains("rustup_script=$(mktemp)"));
    assert!(!install.contains("/tmp/sticktalk-rustup.sh"));
    let backup = include_str!("../../../deploy/backup.sh");
    assert!(backup.contains("trap restart EXIT"));
    assert!(backup.contains("umask 077"));
    assert!(backup.contains("PRAGMA quick_check"));
    let service = include_str!("../../../deploy/sticktalk.service");
    for required in [
        "UMask=0077",
        "NoNewPrivileges=true",
        "ProtectSystem=strict",
        "MemoryMax=1G",
        "Environment=STICKTALK_BIND=127.0.0.1:8080",
    ] {
        assert!(service.contains(required));
    }
    let caddy = include_str!("../../../deploy/Caddyfile");
    for required in [
        "read_header 10s",
        "read_body 120s",
        "write 15m",
        "read_timeout 30s",
        "write_timeout 30s",
        "127.0.0.1:8080",
    ] {
        assert!(caddy.contains(required));
    }
}
#[cfg(windows)]
#[tokio::test]
async fn slow_initial_replay_has_one_deadline() {
    use futures_util::{SinkExt, StreamExt};
    use std::os::windows::io::AsRawSocket;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{client::IntoClientRequest, Message as Frame},
        MaybeTlsStream,
    };
    let (_dir, app) = fixture().await;
    let token = token();
    let key = hash(token.as_bytes());
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=1")
        .bind(&key)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let resume = {
        let mut core = app.0.core.lock().await;
        let cursor = core.cursor(&key, Id(1));
        for n in 0..400 {
            core.emit(
                &[Id(1)],
                Event::MessageCreated {
                    message: Message {
                        id: Id(100 + n),
                        channel_id: Id(11),
                        author_id: Id(2),
                        content: "x".repeat(4000),
                        created_at: 0,
                        edited_at: None,
                        reply_to: None,
                        attachments: vec![],
                        mention_ids: vec![],
                        call: None,
                        reactions: vec![],
                        extra: Default::default(),
                    },
                    author: UserSummary {
                        id: Id(2),
                        username: "user2".into(),
                        display_name: None,
                        avatar_id: None,
                        status_text: None,
                        status_emoji: None,
                    },
                },
            )
            .unwrap();
        }
        cursor
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let serving = app.clone();
    let task = tokio::spawn(async move {
        serving.serve(listener).await.unwrap();
    });
    let mut req = format!("ws://{address}/api/v1/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
    let (mut ws, _) = connect_async(req).await.unwrap();
    assert!(matches!(
        ws.next().await.unwrap().unwrap(),
        Frame::Binary(_)
    ));
    let MaybeTlsStream::Plain(tcp) = ws.get_ref() else {
        panic!("loopback must be plain")
    };
    #[link(name = "ws2_32")]
    extern "system" {
        fn setsockopt(socket: usize, level: i32, name: i32, value: *const i8, length: i32) -> i32;
    }
    let receive_buffer = 4096_i32;
    // SAFETY: the Tokio socket remains owned by ws; the 4-byte synchronous
    // SO_RCVBUF input is valid for the full call. We never close this handle.
    assert_eq!(
        unsafe {
            setsockopt(
                tcp.as_raw_socket() as usize,
                0xffff,
                0x1002,
                (&receive_buffer as *const i32).cast(),
                4,
            )
        },
        0
    );
    ws.send(Frame::Binary(
        rmp_serde::to_vec_named(&ClientMsg::Identify(Identify {
            text_features: false,
            launch_features: false,
            version: 1,
            token,
            resume: Some(resume),
        }))
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(app.0.core.lock().await.streams[&key].connection.is_some());
    // Never drain the replay. Per-frame timeouts could keep this stream pinned
    // for hours; the whole-batch deadline must retire it in ~10 seconds.
    tokio::time::timeout(std::time::Duration::from_secs(12), async {
        while app.0.core.lock().await.streams[&key].connection.is_some() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    drop(ws);
    app.shutdown();
    task.await.unwrap();
}
#[tokio::test]
async fn rest_acl_matrix_and_authorization_before_304() {
    let (_dir, app) = fixture().await;
    let member_token = token();
    let outsider_token = token();
    for (u, t) in [(2, &member_token), (3, &outsider_token)] {
        sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=?")
            .bind(hash(t.as_bytes()))
            .bind(u)
            .execute(&app.0.pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(12,11,1,'private',0)").execute(&app.0.pool).await.unwrap();
    sqlx::query("INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,thumbnail_key,thumbnail_width,thumbnail_height,thumbnail_size,thumbnail_type,created_at,expires_at) VALUES(30,1,'message',11,'file','application/octet-stream',7,'private.bin','private.jpg',1,1,3,'image/jpeg',0,NULL)").execute(&app.0.pool).await.unwrap();
    sqlx::query("INSERT INTO message_attachments VALUES(12,30,0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    tokio::fs::write(app.0.config.data_dir.join("files/private.bin"), b"private")
        .await
        .unwrap();
    tokio::fs::write(app.0.config.data_dir.join("files/private.jpg"), b"jpg")
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let serving = app.clone();
    let task = tokio::spawn(async move {
        serving.serve(listener).await.unwrap();
    });
    let http = reqwest::Client::new();
    for path in [
        "guilds/10",
        "guilds/10/channels",
        "guilds/10/members",
        "guilds/10/invites",
        "guilds/10/bans",
        "channels/11",
        "channels/11/messages",
        "channels/11/messages/12",
        "users?ids=1,3",
        "files/30",
        "files/30/thumbnail",
    ] {
        let response = http
            .get(format!("{url}/api/v1/{path}"))
            .bearer_auth(&outsider_token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "{path}");
        assert_eq!(
            response.json::<ApiError>().await.unwrap().code,
            ErrorCode::NotFound
        );
    }
    for path in ["guilds", "dms", "relationships", "read-states"] {
        let response = http
            .get(format!("{url}/api/v1/{path}"))
            .bearer_auth(&outsider_token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let page: serde_json::Value = response.json().await.unwrap();
        assert_eq!(page["items"].as_array().unwrap().len(), 0);
    }
    assert_eq!(
        http.get(format!("{url}/api/v1/guilds/10/bans"))
            .bearer_auth(&member_token)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        http.delete(format!("{url}/api/v1/uploads/30"))
            .bearer_auth(&outsider_token)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let response = http
        .get(format!("{url}/api/v1/files/30"))
        .bearer_auth(&member_token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let etag = response.headers()["etag"].to_str().unwrap().to_owned();
    assert_eq!(
        http.get(format!("{url}/api/v1/files/30"))
            .bearer_auth(&member_token)
            .header("if-none-match", &etag)
            .send()
            .await
            .unwrap()
            .status(),
        304
    );
    sqlx::query("DELETE FROM guild_members WHERE user_id=2")
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        http.get(format!("{url}/api/v1/files/30"))
            .bearer_auth(&member_token)
            .header("if-none-match", &etag)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    app.shutdown();
    task.await.unwrap();
}
#[tokio::test]
async fn cross_channel_target_errors_do_not_leak_hidden_message_ids() {
    let (_dir, app) = fixture().await;
    sqlx::query("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(14,'text',10,'other',1,0)").execute(&app.0.pool).await.unwrap();
    sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(12,11,1,'private',0)").execute(&app.0.pool).await.unwrap();
    let mut core = app.0.core.lock().await;
    let commands = vec![
        Command::EditMessage(MessageEdit {
            channel_id: Id(14),
            message_id: Id(12),
            content: "wrong room".into(),
            mention_ids: vec![],
        }),
        Command::DeleteMessage {
            channel_id: Id(14),
            message_id: Id(12),
        },
        Command::MarkRead {
            channel_id: Id(14),
            message_id: Id(12),
        },
        Command::AddReaction {
            channel_id: Id(14),
            message_id: Id(12),
            emoji: "👍".into(),
        },
        Command::RemoveReaction {
            channel_id: Id(14),
            message_id: Id(12),
            emoji: "👍".into(),
        },
    ];
    for (i, cmd) in commands.into_iter().enumerate() {
        for (user, code) in [(1, ErrorCode::InvalidRequest), (3, ErrorCode::NotFound)] {
            let response = command(&app, &mut core, user, i as u64 + 1, cmd.clone()).await;
            assert!(matches!(response.outcome,CommandOutcome::Err(e) if e.code==code));
        }
    }
}
#[tokio::test]
async fn dm_and_group_mutations_cannot_use_nonparticipant_ids() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    for (ch, kind) in [(20_i64, "direct"), (30, "group_dm")] {
        sqlx::query(
            "INSERT INTO channels(id,kind,name,position,owner_id,created_at) VALUES(?,?,?,0,?,0)",
        )
        .bind(ch)
        .bind(kind)
        .bind(if kind == "group_dm" {
            Some("group")
        } else {
            None
        })
        .bind(if kind == "group_dm" {
            Some(1_i64)
        } else {
            None
        })
        .execute(&app.0.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO dm_members VALUES(?,1,0,1),(?,2,0,1)")
            .bind(ch)
            .bind(ch)
            .execute(&app.0.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(?,?,1,'private DM',0)").bind(ch+1).bind(ch).execute(&app.0.pool).await.unwrap();
        let channel_id = Id(ch as u64);
        let message_id = Id(ch as u64 + 1);
        let commands = vec![
            Command::SendMessage(MessageDraft {
                channel_id,
                content: "intrusion".into(),
                reply_to: None,
                attachment_ids: vec![],
                mention_ids: vec![],
            }),
            Command::EditMessage(MessageEdit {
                channel_id,
                message_id,
                content: "changed".into(),
                mention_ids: vec![],
            }),
            Command::DeleteMessage {
                channel_id,
                message_id,
            },
            Command::AddReaction {
                channel_id,
                message_id,
                emoji: "👍".into(),
            },
            Command::RemoveReaction {
                channel_id,
                message_id,
                emoji: "👍".into(),
            },
            Command::MarkRead {
                channel_id,
                message_id,
            },
            Command::Typing { channel_id },
            Command::UpdateChannel {
                channel_id,
                patch: ChannelPatch::default(),
            },
            Command::AddGroupRecipient {
                channel_id,
                user_id: Id(4),
            },
            Command::RemoveGroupRecipient {
                channel_id,
                user_id: Id(2),
            },
            Command::TransferGroupOwnership {
                channel_id,
                user_id: Id(2),
            },
            Command::LeaveGroupDm { channel_id },
            Command::CloseDm { channel_id },
        ];
        for (n, cmd) in commands.into_iter().enumerate() {
            let response =
                command(&app, &mut core, 3, ch as u64 * 100 + n as u64, cmd.clone()).await;
            assert!(
                matches!(
                    response.outcome,
                    CommandOutcome::Err(ApiError {
                        code: ErrorCode::NotFound,
                        ..
                    })
                ),
                "{kind}: {cmd:?}: {:?}",
                response.outcome
            );
        }
    }
}
#[tokio::test]
async fn guild_commands_enforce_role_and_hierarchy_boundaries() {
    let (_dir, app) = fixture().await;
    let mut core = app.0.core.lock().await;
    let commands = vec![
        (
            Command::UpdateGuild {
                guild_id: Id(10),
                patch: GuildPatch::default(),
            },
            ErrorCode::Forbidden,
        ),
        (
            Command::DeleteGuild { guild_id: Id(10) },
            ErrorCode::OwnerRequired,
        ),
        (
            Command::TransferGuildOwnership {
                guild_id: Id(10),
                user_id: Id(2),
            },
            ErrorCode::OwnerRequired,
        ),
        (
            Command::SetMemberRole {
                guild_id: Id(10),
                user_id: Id(2),
                role: MemberRole::Admin,
            },
            ErrorCode::OwnerRequired,
        ),
        (
            Command::KickMember {
                guild_id: Id(10),
                user_id: Id(4),
            },
            ErrorCode::Forbidden,
        ),
        (
            Command::BanMember {
                guild_id: Id(10),
                user_id: Id(4),
                reason: None,
            },
            ErrorCode::Forbidden,
        ),
        (
            Command::UnbanMember {
                guild_id: Id(10),
                user_id: Id(4),
            },
            ErrorCode::Forbidden,
        ),
        (
            Command::CreateChannel {
                guild_id: Id(10),
                kind: ChannelKind::Text,
                name: "new".into(),
                parent_id: None,
                topic: None,
            },
            ErrorCode::Forbidden,
        ),
        (
            Command::ReorderChannels {
                guild_id: Id(10),
                placements: vec![ChannelPlacement {
                    channel_id: Id(11),
                    parent_id: None,
                    position: 0,
                }],
            },
            ErrorCode::Forbidden,
        ),
    ];
    for (n, (cmd, member_error)) in commands.into_iter().enumerate() {
        for (user, code) in [(2, member_error), (3, ErrorCode::NotFound)] {
            let response = command(&app, &mut core, user, n as u64 + 1, cmd.clone()).await;
            assert!(matches!(response.outcome,CommandOutcome::Err(e) if e.code==code));
        }
    }
    sqlx::query("INSERT INTO guild_members VALUES(10,3,'admin',0),(10,4,'admin',0)")
        .execute(&app.0.pool)
        .await
        .unwrap();
    for target in [1, 3, 4] {
        for (n, cmd) in [
            Command::KickMember {
                guild_id: Id(10),
                user_id: Id(target),
            },
            Command::BanMember {
                guild_id: Id(10),
                user_id: Id(target),
                reason: None,
            },
        ]
        .into_iter()
        .enumerate()
        {
            let response = command(&app, &mut core, 3, 100 + target * 10 + n as u64, cmd).await;
            assert!(matches!(
                response.outcome,
                CommandOutcome::Err(ApiError {
                    code: ErrorCode::HierarchyViolation,
                    ..
                })
            ));
        }
    }
}
#[tokio::test]
async fn failed_ready_detaches_stream_and_is_not_an_auth_failure() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{client::IntoClientRequest, Message as Frame},
    };
    let (_dir, app) = fixture().await;
    let token = token();
    let key = hash(token.as_bytes());
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=1")
        .bind(&key)
        .execute(&app.0.pool)
        .await
        .unwrap();
    // Inject a storage/projection failure, not an invalid credential.
    sqlx::query("DELETE FROM read_states WHERE user_id=1")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let serving = app.clone();
    let task = tokio::spawn(async move {
        serving.serve(listener).await.unwrap();
    });
    let mut req = format!("ws://{address}/api/v1/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
    let (mut ws, _) = connect_async(req).await.unwrap();
    let _hello = ws.next().await.unwrap().unwrap();
    ws.send(Frame::Binary(
        rmp_serde::to_vec_named(&ClientMsg::Identify(Identify {
            text_features: false,
            launch_features: true,
            version: 1,
            token,
            resume: None,
        }))
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    let Frame::Binary(bytes) = ws.next().await.unwrap().unwrap() else {
        panic!("missing error")
    };
    assert!(matches!(
        rmp_serde::from_slice::<ServerMsg>(&bytes).unwrap(),
        ServerMsg::Error(ApiError {
            code: ErrorCode::Internal,
            ..
        })
    ));
    assert!(
        matches!(ws.next().await.unwrap().unwrap(),Frame::Close(Some(c)) if u16::from(c.code)==1011)
    );
    assert!(app.0.core.lock().await.streams[&key].connection.is_none());
    app.shutdown();
    task.await.unwrap();
}
