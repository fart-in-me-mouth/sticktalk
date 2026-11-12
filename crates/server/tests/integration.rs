use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use sticktalk_proto::*;
use sticktalk_server::{App, Config};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as Frame},
    MaybeTlsStream, WebSocketStream,
};
struct TestServer {
    app: App,
    url: String,
    _dir: tempfile::TempDir,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl TestServer {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(Config {
            operations: Default::default(),
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
            voice: Default::default(),
        })
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let serving = app.clone();
        let task = tokio::spawn(async move { serving.serve(listener).await });
        Self {
            app,
            url,
            _dir: dir,
            task,
        }
    }
    async fn register(&self, s: &str) -> AuthResponse {
        let r = reqwest::Client::new()
            .post(format!("{}/api/v1/auth/register", self.url))
            .json(&RegisterRequest {
                username: s.into(),
                password: "correct horse battery".into(),
                display_name: None,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201, "{}", r.text().await.unwrap());
        r.json().await.unwrap()
    }
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str, token: &str) -> T {
        let r = reqwest::Client::new()
            .get(format!("{}/api/v1/{path}", self.url))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
        r.json().await.unwrap()
    }
    async fn stop(self) {
        self.app.shutdown();
        tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
struct Peer {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    cursor: ResumeCursor,
    nonce: u64,
    events: Vec<Event>,
}
impl Peer {
    async fn connect(
        s: &TestServer,
        auth: &AuthResponse,
        resume: Option<ResumeCursor>,
    ) -> (Self, ServerMsg) {
        let mut req = format!("{}/api/v1/ws", s.url.replace("http:", "ws:"))
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
        let (mut ws, _) = connect_async(req).await.unwrap();
        assert!(matches!(read(&mut ws).await, ServerMsg::Hello(_)));
        ws.send(Frame::Binary(
            rmp_serde::to_vec_named(&ClientMsg::Identify(Identify {
                version: 1,
                token: auth.token.clone(),
                resume,
            }))
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        let mut events = vec![];
        loop {
            let m = read(&mut ws).await;
            match &m {
                ServerMsg::Event(e) => events.push(e.event.clone()),
                ServerMsg::Ready(r) => {
                    return (
                        Self {
                            ws,
                            cursor: r.cursor.clone(),
                            nonce: rand::random::<u32>() as u64 + 1000,
                            events,
                        },
                        m,
                    );
                }
                ServerMsg::Resumed(r) => {
                    return (
                        Self {
                            ws,
                            cursor: r.cursor.clone(),
                            nonce: rand::random::<u32>() as u64 + 1000,
                            events,
                        },
                        m,
                    );
                }
                ServerMsg::ResyncRequired(_) => {}
                _ => panic!("unexpected handshake {m:?}"),
            }
        }
    }
    async fn command_nonce(&mut self, nonce: u64, command: Command) -> CommandResponse {
        self.ws
            .send(Frame::Binary(
                rmp_serde::to_vec_named(&ClientMsg::Command(CommandRequest { nonce, command }))
                    .unwrap()
                    .into(),
            ))
            .await
            .unwrap();
        loop {
            match self.recv().await {
                ServerMsg::Response(r) => {
                    assert_eq!(r.nonce, nonce);
                    return *r;
                }
                ServerMsg::Event(_) | ServerMsg::Voice(_) => {}
                m => panic!("unexpected {m:?}"),
            }
        }
    }
    async fn command(&mut self, command: Command) -> CommandResult {
        let nonce = self.nonce;
        self.nonce += 1;
        let r = self.command_nonce(nonce, command).await;
        match r.outcome {
            CommandOutcome::Ok(r) => r,
            CommandOutcome::Err(e) => panic!("command failed: {e:?}"),
        }
    }
    async fn recv(&mut self) -> ServerMsg {
        let m = read(&mut self.ws).await;
        if let ServerMsg::Event(e) = &m {
            assert!(e.seq > self.cursor.seq);
            self.cursor.seq = e.seq;
            self.events.push(e.event.clone());
        }
        m
    }
    async fn event(&mut self, p: impl Fn(&Event) -> bool) -> Event {
        loop {
            if let Some(i) = self.events.iter().position(&p) {
                return self.events.remove(i);
            }
            self.recv().await;
        }
    }
    async fn close(mut self) -> ResumeCursor {
        self.ws.close(None).await.unwrap();
        self.cursor
    }
}
async fn read(ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>) -> ServerMsg {
    let m = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("WS timeout")
        .expect("WS closed")
        .unwrap();
    match m {
        Frame::Binary(b) => rmp_serde::from_slice(&b).unwrap(),
        m => panic!("not binary: {m:?}"),
    }
}
fn draft(ch: Id, s: &str) -> Command {
    Command::SendMessage(MessageDraft {
        channel_id: ch,
        content: s.into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    })
}
#[tokio::test]
async fn phase1_end_to_end() {
    let s = TestServer::start().await;
    let a = s.register("Alice").await;
    let b = s.register("bob").await;
    let (mut pa, ready) = Peer::connect(&s, &a, None).await;
    assert!(matches!(ready,ServerMsg::Ready(r) if r.me.user.username=="alice"));
    let (mut pb, _) = Peer::connect(&s, &b, None).await;
    pa.command(Command::SendFriendRequest {
        username: "bob".into(),
    })
    .await;
    pb.event(
        |e| matches!(e,Event::RelationshipUpsert(r) if r.kind==RelationshipKind::IncomingRequest),
    )
    .await;
    pb.command(Command::AcceptFriendRequest {
        user_id: a.user.user.id,
    })
    .await;
    pa.event(|e|matches!(e,Event::PresenceUpdated(p) if p.user_id==b.user.user.id&&p.status==PresenceStatus::Online)).await;
    pb.command(Command::SetPresence {
        status: PresenceStatus::Idle,
    })
    .await;
    pa.event(|e|matches!(e,Event::PresenceUpdated(p) if p.user_id==b.user.user.id&&p.status==PresenceStatus::Idle)).await;
    let dm = match pa
        .command(Command::OpenDm {
            user_id: b.user.user.id,
        })
        .await
    {
        CommandResult::Channel(c) => c.summary.id,
        _ => panic!(),
    };
    let sent = match pa.command(draft(dm, "hello")).await {
        CommandResult::Message(m) => m,
        _ => panic!(),
    };
    pb.event(|e| matches!(e,Event::MessageCreated{message,..} if message.id==sent.id))
        .await;
    pa.command(Command::EditMessage(MessageEdit {
        channel_id: dm,
        message_id: sent.id,
        content: "edited".into(),
        mention_ids: vec![b.user.user.id],
    }))
    .await;
    pb.event(|e| matches!(e,Event::MessageUpdated(m) if m.content=="edited"))
        .await;
    let reaction = pb
        .command(Command::AddReaction {
            channel_id: dm,
            message_id: sent.id,
            emoji: "👍🏽".into(),
        })
        .await;
    assert!(matches!(reaction,CommandResult::Reaction(r) if r.reaction.me&&r.reaction.count==1));
    pa.event(|e| matches!(e,Event::ReactionUpdated(r) if !r.reaction.me&&r.reaction.count==1))
        .await;
    pb.command(Command::MarkRead {
        channel_id: dm,
        message_id: sent.id,
    })
    .await;
    let reads: Page<ReadState> = s.get("read-states", &b.token).await;
    let r = reads.items.iter().find(|r| r.channel_id == dm).unwrap();
    assert_eq!(r.unread_count, 0);
    assert_eq!(r.mention_count, 0);
    pa.command(Command::Typing { channel_id: dm }).await;
    pb.event(|e| matches!(e,Event::Typing{channel_id,..} if *channel_id==dm))
        .await;
    let second = match pa.command(draft(dm, "second")).await {
        CommandResult::Message(m) => m,
        _ => panic!(),
    };
    let page: Page<Message> = s
        .get(&format!("channels/{}/messages?limit=1", dm.0), &b.token)
        .await;
    assert_eq!(page.items[0].id, second.id);
    assert!(page.has_more);
    let older: Page<Message> = s
        .get(
            &format!("channels/{}/messages?before={}", dm.0, second.id.0),
            &b.token,
        )
        .await;
    assert_eq!(older.items[0].id, sent.id);
    pa.command(Command::DeleteMessage {
        channel_id: dm,
        message_id: sent.id,
    })
    .await;
    pb.event(|e| matches!(e,Event::MessageDeleted{message_id,..} if *message_id==sent.id))
        .await;
    let guild = match pa
        .command(Command::CreateGuild {
            name: "Test guild".into(),
            icon_id: None,
        })
        .await
    {
        CommandResult::Guild(g) => g,
        _ => panic!(),
    };
    let gid = guild.summary.guild.id;
    let invite = match pa
        .command(Command::CreateInvite {
            guild_id: gid,
            ttl_seconds: 60,
            max_uses: 5,
        })
        .await
    {
        CommandResult::Invite(i) => i,
        _ => panic!(),
    };
    pb.command(Command::JoinGuild {
        invite_code: invite.code,
    })
    .await;
    let ch = match pa
        .command(Command::CreateChannel {
            guild_id: gid,
            kind: ChannelKind::Text,
            name: "testing".into(),
            parent_id: None,
            topic: Some("topic".into()),
        })
        .await
    {
        CommandResult::Channel(ch) => ch.summary.id,
        _ => panic!(),
    };
    pb.event(|e| matches!(e,Event::ChannelUpsert(c) if c.summary.id==ch))
        .await;
    let denial = pb
        .command_nonce(900, Command::DeleteChannel { channel_id: ch })
        .await;
    assert!(matches!(
        denial.outcome,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::Forbidden,
            ..
        })
    ));
    let cursor = pb.close().await;
    pa.command(draft(ch, "while disconnected")).await;
    let (mut pb, resumed) = Peer::connect(&s, &b, Some(cursor)).await;
    assert!(matches!(resumed,ServerMsg::Resumed(r) if r.replayed>0));
    assert!(pb.events.iter().any(
        |e| matches!(e,Event::MessageCreated{message,..} if message.content=="while disconnected")
    ));
    let original = draft(ch, "deduplicated");
    let r = pa.command_nonce(10000, original.clone()).await;
    assert!(matches!(r.outcome, CommandOutcome::Ok(_)));
    let duplicate = pa.command_nonce(10000, original).await;
    assert!(duplicate.replayed);
    let conflict = pa.command_nonce(10000, draft(ch, "different")).await;
    assert!(matches!(
        conflict.outcome,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::Conflict,
            ..
        })
    ));
    let mut png = std::io::Cursor::new(vec![]);
    image::DynamicImage::new_rgb8(400, 200)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let form = reqwest::multipart::Form::new()
        .text("purpose", "message")
        .text("channel_id", ch.0.to_string())
        .part(
            "file",
            reqwest::multipart::Part::bytes(png.into_inner())
                .file_name("../../image.png")
                .mime_str("image/png")
                .unwrap(),
        );
    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/uploads", s.url))
        .bearer_auth(&a.token)
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201, "{}", response.text().await.unwrap());
    let upload: UploadResponse = response.json().await.unwrap();
    assert_eq!(upload.attachment.filename, "image.png");
    let thumb = upload.attachment.thumbnail.as_ref().unwrap();
    assert_eq!((thumb.width, thumb.height), (256, 128));
    let file_url = format!("{}/api/v1/files/{}", s.url, upload.attachment.id.0);
    let private = reqwest::Client::new()
        .get(&file_url)
        .bearer_auth(&b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(private.status(), 404);
    let thumb = reqwest::Client::new()
        .get(format!("{file_url}/thumbnail"))
        .bearer_auth(&a.token)
        .send()
        .await
        .unwrap();
    assert_eq!(thumb.status(), 200);
    assert_eq!(thumb.headers()["content-type"], "image/jpeg");
    assert!(!thumb.bytes().await.unwrap().is_empty());
    let etag = format!("\"{}-original\"", upload.attachment.id.0);
    let cached = reqwest::Client::new()
        .get(&file_url)
        .bearer_auth(&a.token)
        .header("if-none-match", etag)
        .send()
        .await
        .unwrap();
    assert_eq!(cached.status(), 304);
    let range = reqwest::Client::new()
        .get(&file_url)
        .bearer_auth(&a.token)
        .header("range", "bytes=0-9")
        .send()
        .await
        .unwrap();
    assert_eq!(range.status(), 206);
    assert_eq!(range.bytes().await.unwrap().len(), 10);
    let attach = Command::SendMessage(MessageDraft {
        channel_id: ch,
        content: "file".into(),
        reply_to: None,
        attachment_ids: vec![upload.attachment.id],
        mention_ids: vec![],
    });
    pa.command(attach).await;
    let accessible = reqwest::Client::new()
        .get(&file_url)
        .bearer_auth(&b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(accessible.status(), 200);
    let _: Snapshot<GuildDetail> = s.get(&format!("guilds/{}", gid.0), &b.token).await;
    let _: Page<Member> = s.get(&format!("guilds/{}/members", gid.0), &b.token).await;
    pa.close().await;
    pb.ws.close(None).await.unwrap();
    let login = reqwest::Client::new()
        .post(format!("{}/api/v1/auth/login", s.url))
        .json(&LoginRequest {
            username: "ALICE".into(),
            password: "correct horse battery".into(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200);
    let login: AuthResponse = login.json().await.unwrap();
    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/auth/logout", s.url))
        .bearer_auth(&login.token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    let revoked = reqwest::Client::new()
        .get(format!("{}/api/v1/me", s.url))
        .bearer_auth(login.token)
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 401);
    s.stop().await;
}
#[tokio::test]
async fn authorization_and_protocol_boundaries() {
    let s = TestServer::start().await;
    let a = s.register("alpha").await;
    let b = s.register("bravo").await;
    let (mut pa, _) = Peer::connect(&s, &a, None).await;
    let (mut pb, _) = Peer::connect(&s, &b, None).await;
    let denied = pa
        .command_nonce(
            1,
            Command::OpenDm {
                user_id: b.user.user.id,
            },
        )
        .await;
    assert!(matches!(
        denied.outcome,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::Forbidden,
            ..
        })
    ));
    let g = match pa
        .command_nonce(
            2,
            Command::CreateGuild {
                name: "Private".into(),
                icon_id: None,
            },
        )
        .await
        .outcome
    {
        CommandOutcome::Ok(CommandResult::Guild(g)) => g,
        _ => panic!(),
    };
    let ch = g.channels[0].id;
    let hidden = pb.command_nonce(1, draft(ch, "intruder")).await;
    assert!(matches!(
        hidden.outcome,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::NotFound,
            ..
        })
    ));
    let response = reqwest::Client::new()
        .get(format!("{}/api/v1/users?ids={}", s.url, a.user.user.id.0))
        .bearer_auth(&b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    pa.ws
        .send(Frame::Binary(
            rmp_serde::to_vec_named(&ClientMsg::Voice(VoicePayload {
                nonce: 3,
                version: 99,
                kind: "join".into(),
                payload: vec![],
            }))
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    loop {
        if let ServerMsg::Voice(v) = pa.recv().await {
            assert_eq!(v.nonce, 3);
            assert!(matches!(
                rmp_serde::from_slice::<VoiceEvent>(&v.payload).unwrap(),
                VoiceEvent::Error {
                    code: ErrorCode::VersionMismatch,
                    ..
                }
            ));
            break;
        }
    }
    pa.close().await;
    pb.close().await;
    s.stop().await;
}

#[tokio::test]
async fn group_lifecycle_and_relationship_policies() {
    let s = TestServer::start().await;
    let a = s.register("owner").await;
    let b = s.register("second").await;
    let c = s.register("third").await;
    let (mut pa, _) = Peer::connect(&s, &a, None).await;
    let (mut pb, _) = Peer::connect(&s, &b, None).await;
    let (mut pc, _) = Peer::connect(&s, &c, None).await;
    pa.command(Command::SendFriendRequest {
        username: "second".into(),
    })
    .await;
    pb.command(Command::AcceptFriendRequest {
        user_id: a.user.user.id,
    })
    .await;
    pa.command(Command::SendFriendRequest {
        username: "third".into(),
    })
    .await;
    pc.command(Command::AcceptFriendRequest {
        user_id: a.user.user.id,
    })
    .await;
    let group = match pa
        .command(Command::CreateGroupDm {
            name: "group".into(),
            user_ids: vec![b.user.user.id, c.user.user.id],
        })
        .await
    {
        CommandResult::Channel(c) => c,
        _ => panic!(),
    };
    let ch = group.summary.id;
    let bad = pb
        .command_nonce(
            700,
            Command::RemoveGroupRecipient {
                channel_id: ch,
                user_id: c.user.user.id,
            },
        )
        .await;
    assert!(matches!(
        bad.outcome,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::OwnerRequired,
            ..
        })
    ));
    let sent = match pc.command(draft(ch, "group history")).await {
        CommandResult::Message(m) => m,
        _ => panic!(),
    };
    let wrong = pa
        .command_nonce(
            701,
            Command::DeleteMessage {
                channel_id: ch,
                message_id: sent.id,
            },
        )
        .await;
    assert!(matches!(
        wrong.outcome,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::Forbidden,
            ..
        })
    ));
    pa.command(Command::RemoveGroupRecipient {
        channel_id: ch,
        user_id: c.user.user.id,
    })
    .await;
    let private = reqwest::Client::new()
        .get(format!("{}/api/v1/channels/{}/messages", s.url, ch.0))
        .bearer_auth(&c.token)
        .send()
        .await
        .unwrap();
    assert_eq!(private.status(), 404);
    pa.command(Command::AddGroupRecipient {
        channel_id: ch,
        user_id: c.user.user.id,
    })
    .await;
    let history: Page<Message> = s
        .get(&format!("channels/{}/messages", ch.0), &c.token)
        .await;
    assert_eq!(history.items[0].id, sent.id);
    pa.command(Command::UpdateChannel {
        channel_id: ch,
        patch: ChannelPatch {
            name: Patch::Set("renamed".into()),
            ..Default::default()
        },
    })
    .await;
    pa.command(Command::TransferGroupOwnership {
        channel_id: ch,
        user_id: b.user.user.id,
    })
    .await;
    pa.command(Command::LeaveGroupDm { channel_id: ch }).await;
    pb.command(Command::RemoveGroupRecipient {
        channel_id: ch,
        user_id: c.user.user.id,
    })
    .await;
    pb.command(Command::LeaveGroupDm { channel_id: ch }).await;
    pa.command(Command::BlockUser {
        user_id: b.user.user.id,
    })
    .await;
    let blocked = pb
        .command_nonce(
            702,
            Command::OpenDm {
                user_id: a.user.user.id,
            },
        )
        .await;
    assert!(matches!(
        blocked.outcome,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::Blocked,
            ..
        })
    ));
    pa.command(Command::UnblockUser {
        user_id: b.user.user.id,
    })
    .await;
    let profile = pa
        .command(Command::UpdateProfile(ProfilePatch {
            display_name: Patch::Set("Owner Name".into()),
            status_text: Patch::Set("here".into()),
            ..Default::default()
        }))
        .await;
    assert!(
        matches!(profile,CommandResult::Profile(u) if u.user.display_name.as_deref()==Some("Owner Name"))
    );
    pa.close().await;
    pb.close().await;
    pc.close().await;
    s.stop().await;
}
#[tokio::test]
async fn moderation_reorder_and_safe_replay_after_revocation() {
    let s = TestServer::start().await;
    let a = s.register("mod_owner").await;
    let b = s.register("mod_member").await;
    let (mut pa, _) = Peer::connect(&s, &a, None).await;
    let (mut pb, _) = Peer::connect(&s, &b, None).await;
    let g = match pa
        .command(Command::CreateGuild {
            name: "Moderation".into(),
            icon_id: None,
        })
        .await
    {
        CommandResult::Guild(g) => g,
        _ => panic!(),
    };
    let gid = g.summary.guild.id;
    let general = g.channels[0].id;
    let inv = match pa
        .command(Command::CreateInvite {
            guild_id: gid,
            ttl_seconds: 60,
            max_uses: 5,
        })
        .await
    {
        CommandResult::Invite(i) => i,
        _ => panic!(),
    };
    pb.command(Command::JoinGuild {
        invite_code: inv.code.clone(),
    })
    .await;
    pb.command(Command::SubscribeGuild {
        guild_id: Some(gid),
        presence_user_ids: vec![a.user.user.id],
    })
    .await;
    pa.command(Command::SetMemberRole {
        guild_id: gid,
        user_id: b.user.user.id,
        role: MemberRole::Admin,
    })
    .await;
    pb.event(|e| matches!(e,Event::MemberUpsert(m) if m.role==MemberRole::Admin))
        .await;
    let denied = pb
        .command_nonce(
            800,
            Command::KickMember {
                guild_id: gid,
                user_id: a.user.user.id,
            },
        )
        .await;
    assert!(matches!(
        denied.outcome,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::HierarchyViolation,
            ..
        })
    ));
    let category = match pb
        .command(Command::CreateChannel {
            guild_id: gid,
            kind: ChannelKind::Category,
            name: "Category".into(),
            parent_id: None,
            topic: None,
        })
        .await
    {
        CommandResult::Channel(c) => c.summary.id,
        _ => panic!(),
    };
    let child = match pb
        .command(Command::CreateChannel {
            guild_id: gid,
            kind: ChannelKind::Text,
            name: "child".into(),
            parent_id: Some(category),
            topic: None,
        })
        .await
    {
        CommandResult::Channel(c) => c.summary.id,
        _ => panic!(),
    };
    pb.command(Command::ReorderChannels {
        guild_id: gid,
        placements: vec![
            ChannelPlacement {
                channel_id: category,
                parent_id: None,
                position: 0,
            },
            ChannelPlacement {
                channel_id: general,
                parent_id: None,
                position: 1,
            },
            ChannelPlacement {
                channel_id: child,
                parent_id: Some(category),
                position: 0,
            },
        ],
    })
    .await;
    pb.command(Command::DeleteChannel {
        channel_id: category,
    })
    .await;
    let moved: Snapshot<Channel> = s.get(&format!("channels/{}", child.0), &a.token).await;
    assert_eq!(moved.data.summary.parent_id, None);
    assert_eq!(moved.data.summary.position, 1);
    let cursor = pb.close().await;
    pa.command(draft(general, "must not replay to revoked member"))
        .await;
    pa.command(Command::KickMember {
        guild_id: gid,
        user_id: b.user.user.id,
    })
    .await;
    let (mut pb, ready) = Peer::connect(&s, &b, Some(cursor)).await;
    assert!(matches!(ready,ServerMsg::Ready(r) if r.guilds.is_empty()));
    assert!(!pb
        .events
        .iter()
        .any(|e| matches!(e, Event::MessageCreated { .. })));
    pa.command(Command::BanMember {
        guild_id: gid,
        user_id: b.user.user.id,
        reason: Some("policy".into()),
    })
    .await;
    let bans: Page<Ban> = s.get(&format!("guilds/{}/bans", gid.0), &a.token).await;
    assert_eq!(bans.items[0].user.id, b.user.user.id);
    let banned = pb
        .command_nonce(
            801,
            Command::JoinGuild {
                invite_code: inv.code.clone(),
            },
        )
        .await;
    assert!(matches!(
        banned.outcome,
        CommandOutcome::Err(ApiError {
            code: ErrorCode::Banned,
            ..
        })
    ));
    pa.command(Command::UnbanMember {
        guild_id: gid,
        user_id: b.user.user.id,
    })
    .await;
    pb.command(Command::JoinGuild {
        invite_code: inv.code.clone(),
    })
    .await;
    pa.command(Command::TransferGuildOwnership {
        guild_id: gid,
        user_id: b.user.user.id,
    })
    .await;
    pa.command(Command::LeaveGuild { guild_id: gid }).await;
    pb.command(Command::RevokeInvite {
        guild_id: gid,
        code: inv.code,
    })
    .await;
    pb.command(Command::DeleteGuild { guild_id: gid }).await;
    pa.close().await;
    pb.close().await;
    s.stop().await;
}
#[tokio::test]
async fn receipts_survive_restart_and_streams_do_not() {
    let s = TestServer::start().await;
    let a = s.register("persistent").await;
    let (mut p, _) = Peer::connect(&s, &a, None).await;
    let command = Command::CreateGuild {
        name: "Persisted".into(),
        icon_id: None,
    };
    let result = p.command_nonce(77, command.clone()).await;
    let cursor = p.close().await;
    let TestServer {
        app,
        task,
        _dir: dir,
        ..
    } = s;
    app.shutdown();
    task.await.unwrap().unwrap();
    drop(app);
    let app = App::open(Config {
        operations: Default::default(),
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
        voice: Default::default(),
    })
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let serving = app.clone();
    let task = tokio::spawn(async move { serving.serve(listener).await });
    let s = TestServer {
        app,
        url,
        task,
        _dir: dir,
    };
    let (mut p, ready) = Peer::connect(&s, &a, Some(cursor.clone())).await;
    assert!(
        matches!(ready,ServerMsg::Ready(ref r) if r.cursor.stream_id!=cursor.stream_id&&r.guilds.len()==1)
    );
    let replay = p.command_nonce(77, command).await;
    assert!(replay.replayed);
    assert_eq!(replay.outcome, result.outcome);
    p.close().await;
    s.stop().await;
}

#[tokio::test]
async fn handshake_control_and_replacement() {
    let s = TestServer::start().await;
    let a = s.register("ws_user").await;
    let ws_url = format!("{}/api/v1/ws", s.url.replace("http:", "ws:"));
    let request = ws_url.clone().into_client_request().unwrap();
    let err = connect_async(request).await.unwrap_err();
    assert!(matches!(err,tokio_tungstenite::tungstenite::Error::Http(r) if r.status()==426));
    let mut request = ws_url.clone().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
    request
        .headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    let err = connect_async(request).await.unwrap_err();
    assert!(matches!(err,tokio_tungstenite::tungstenite::Error::Http(r) if r.status()==403));
    let mut request = ws_url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
    let (mut bad, _) = connect_async(request).await.unwrap();
    assert!(matches!(read(&mut bad).await, ServerMsg::Hello(_)));
    bad.send(Frame::Binary(
        rmp_serde::to_vec_named(&ClientMsg::Identify(Identify {
            version: 2,
            token: a.token.clone(),
            resume: None,
        }))
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    assert!(matches!(
        read(&mut bad).await,
        ServerMsg::Error(ApiError {
            code: ErrorCode::VersionMismatch,
            ..
        })
    ));
    assert!(
        matches!(bad.next().await.unwrap().unwrap(),Frame::Close(Some(c)) if u16::from(c.code)==4002)
    );
    let (mut p, _) = Peer::connect(&s, &a, None).await;
    let malformed = serde_json::json!({"type":"command","data":{"nonce":543,"command":{"type":"send_message","data":{"channel_id":"wrong type"}}}});
    p.ws.send(Frame::Binary(
        rmp_serde::to_vec_named(&malformed).unwrap().into(),
    ))
    .await
    .unwrap();
    loop {
        if let ServerMsg::Response(r) = p.recv().await {
            assert_eq!(r.nonce, 543);
            assert!(matches!(
                r.outcome,
                CommandOutcome::Err(ApiError {
                    code: ErrorCode::InvalidRequest,
                    ..
                })
            ));
            break;
        }
    }
    p.ws.send(Frame::Binary(
        rmp_serde::to_vec_named(&ClientMsg::Heartbeat(Heartbeat {
            nonce: 333,
            last_seq: p.cursor.seq,
        }))
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    loop {
        if let ServerMsg::HeartbeatAck(h) = p.recv().await {
            assert_eq!(h.nonce, 333);
            break;
        }
    }
    p.command(Command::SetPresence {
        status: PresenceStatus::Online,
    })
    .await;
    let cursor = p.close().await;
    let (mut older, resumed) = Peer::connect(&s, &a, Some(cursor)).await;
    assert!(matches!(resumed,ServerMsg::Resumed(r) if r.replayed==0));
    let (mut newer, _) = Peer::connect(&s, &a, None).await;
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), older.ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Frame::Close(Some(c)) = frame {
            assert_eq!(u16::from(c.code), 4009);
            break;
        }
    }
    let logout = reqwest::Client::new()
        .post(format!("{}/api/v1/auth/logout", s.url))
        .bearer_auth(&a.token)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), 204);
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), newer.ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Frame::Close(Some(c)) = frame {
            assert_eq!(u16::from(c.code), 4001);
            break;
        }
    }
    s.stop().await;
}
#[tokio::test]
async fn preauth_oversize_and_depth_use_size_close_code() {
    let s = TestServer::start().await;
    for payload in [
        None,
        Some({
            let mut b = vec![0x81, 0xa1, b'x'];
            b.extend(vec![0x91; 34]);
            b.push(0xc0);
            b
        }),
    ] {
        let mut req = format!("{}/api/v1/ws", s.url.replace("http:", "ws:"))
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
        let (mut ws, _) = connect_async(req).await.unwrap();
        assert!(matches!(read(&mut ws).await, ServerMsg::Hello(_)));
        if let Some(payload) = payload {
            ws.send(Frame::Binary(payload.into())).await.unwrap();
        } else {
            // Send only a masked oversized frame header: prove early rejection
            // without a large allocation or a TCP RST caused by unread payload.
            use tokio::io::AsyncWriteExt;
            let mut header = vec![0x82, 0xff];
            header.extend((u64::from(MAX_WS_FRAME_BYTES) + 1).to_be_bytes());
            header.extend([0; 4]);
            ws.get_mut().write_all(&header).await.unwrap();
        }
        assert!(matches!(
            read(&mut ws).await,
            ServerMsg::Error(ApiError {
                code: ErrorCode::PayloadTooLarge,
                ..
            })
        ));
        let close = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(close,Frame::Close(Some(c)) if u16::from(c.code)==1009));
    }
    s.stop().await;
}
#[tokio::test]
async fn single_use_invite_is_consumed_atomically() {
    let s = TestServer::start().await;
    let a = s.register("race_owner").await;
    let b = s.register("race_first").await;
    let c = s.register("race_second").await;
    let (mut pa, _) = Peer::connect(&s, &a, None).await;
    let (mut pb, _) = Peer::connect(&s, &b, None).await;
    let (mut pc, _) = Peer::connect(&s, &c, None).await;
    let CommandResult::Guild(g) = pa
        .command(Command::CreateGuild {
            name: "race".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let CommandResult::Invite(invite) = pa
        .command(Command::CreateInvite {
            guild_id: g.summary.guild.id,
            ttl_seconds: 60,
            max_uses: 1,
        })
        .await
    else {
        panic!()
    };
    let (b_result, c_result) = tokio::join!(
        pb.command_nonce(
            99,
            Command::JoinGuild {
                invite_code: invite.code.clone()
            }
        ),
        pc.command_nonce(
            99,
            Command::JoinGuild {
                invite_code: invite.code.clone()
            }
        )
    );
    let results = [b_result.outcome, c_result.outcome];
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, CommandOutcome::Ok(CommandResult::Guild(_))))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(
                r,
                CommandOutcome::Err(ApiError {
                    code: ErrorCode::InviteExhausted,
                    ..
                })
            ))
            .count(),
        1
    );
    let members: Page<Member> = s
        .get(
            &format!("guilds/{}/members", g.summary.guild.id.0),
            &a.token,
        )
        .await;
    assert_eq!(members.items.len(), 2);
    pa.close().await;
    pb.close().await;
    pc.close().await;
    s.stop().await;
}
#[tokio::test]
async fn graceful_shutdown_drains_authenticated_and_pre_auth_sockets() {
    let s = TestServer::start().await;
    let a = s.register("shutdown_user").await;
    let (mut peer, _) = Peer::connect(&s, &a, None).await;
    let mut req = format!("{}/api/v1/ws", s.url.replace("http:", "ws:"))
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
    let (mut preauth, _) = connect_async(req).await.unwrap();
    assert!(matches!(read(&mut preauth).await, ServerMsg::Hello(_)));
    s.app.shutdown();
    for ws in [&mut peer.ws, &mut preauth] {
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let Frame::Close(Some(c)) = frame {
                assert_eq!(u16::from(c.code), 1001);
                break;
            }
        }
    }
    s.stop().await;
}

#[tokio::test]
async fn around_history_is_bounded_bidirectional_and_accepts_deleted_gaps() {
    let server = TestServer::start().await;
    let auth = server.register("around_owner").await;
    let (mut peer, _) = Peer::connect(&server, &auth, None).await;
    let CommandResult::Guild(detail) = peer
        .command(Command::CreateGuild {
            name: "Around".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let channel = detail
        .channels
        .iter()
        .find(|c| c.kind == ChannelKind::Text)
        .unwrap()
        .id;
    let empty: Page<Message> = server
        .get(
            &format!("channels/{}/messages?around=1", channel.0),
            &auth.token,
        )
        .await;
    assert!(empty.items.is_empty() && !empty.has_more);
    let mut messages = vec![];
    for n in 0..9 {
        let CommandResult::Message(message) = peer
            .command(Command::SendMessage(MessageDraft {
                channel_id: channel,
                content: format!("message {n}"),
                reply_to: None,
                attachment_ids: vec![],
                mention_ids: vec![],
            }))
            .await
        else {
            panic!()
        };
        messages.push(message);
    }
    let page: Page<Message> = server
        .get(
            &format!(
                "channels/{}/messages?around={}&limit=4",
                channel.0, messages[4].id.0
            ),
            &auth.token,
        )
        .await;
    assert_eq!(
        page.items.iter().map(|m| m.id).collect::<Vec<_>>(),
        [
            messages[6].id,
            messages[5].id,
            messages[4].id,
            messages[3].id
        ]
    );
    assert_eq!(page.next_before, Some(messages[3].id));
    assert_eq!(page.next_after, Some(messages[6].id));
    for pivot in [messages[0].id, messages[8].id] {
        let edge: Page<Message> = server
            .get(
                &format!("channels/{}/messages?around={}&limit=4", channel.0, pivot.0),
                &auth.token,
            )
            .await;
        assert_eq!(edge.items.len(), 4);
        assert!(edge.items.iter().any(|m| m.id == pivot));
        assert_eq!(edge.next_before.is_some(), pivot == messages[8].id);
        assert_eq!(edge.next_after.is_some(), pivot == messages[0].id);
    }
    peer.command(Command::DeleteMessage {
        channel_id: channel,
        message_id: messages[4].id,
    })
    .await;
    let gap: Page<Message> = server
        .get(
            &format!(
                "channels/{}/messages?around={}&limit=4",
                channel.0, messages[4].id.0
            ),
            &auth.token,
        )
        .await;
    assert_eq!(gap.items.len(), 4);
    assert!(!gap.items.iter().any(|m| m.id == messages[4].id));
    let time: Page<Message> = server
        .get(
            &format!(
                "channels/{}/messages?around_timestamp={}&limit=3",
                channel.0, messages[4].created_at
            ),
            &auth.token,
        )
        .await;
    assert_eq!(time.items.len(), 3);
    server.stop().await;
}
