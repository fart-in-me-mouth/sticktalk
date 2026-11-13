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
            voice: sticktalk_server::voice::VoiceOptions {
                voice_bind_ip: "127.0.0.1".parse().unwrap(),
                voice_advertised_ip: Some("127.0.0.1".parse().unwrap()),
                ..Default::default()
            },
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
    #[allow(dead_code)]
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
    voice: Vec<VoiceEvent>,
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
                launch_features: false,
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
                            voice: vec![],
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
                            voice: vec![],
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
        if let ServerMsg::Voice(v) = &m {
            let event = rmp_serde::from_slice(&v.payload).unwrap();
            self.voice.push(event);
        }
        if let ServerMsg::Event(e) = &m {
            assert!(e.seq > self.cursor.seq);
            self.cursor.seq = e.seq;
            self.events.push(e.event.clone());
        }
        m
    }
    #[allow(dead_code)]
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

impl Peer {
    async fn signal(&mut self, signal: VoiceSignal) -> u64 {
        self.nonce += 1;
        let envelope = sticktalk_sfu::encode_signal(self.nonce, &signal).unwrap();
        self.ws
            .send(Frame::Binary(
                rmp_serde::to_vec_named(&ClientMsg::Voice(envelope))
                    .unwrap()
                    .into(),
            ))
            .await
            .unwrap();
        self.nonce
    }
    async fn voice(&mut self, predicate: impl Fn(&VoiceEvent) -> bool) -> VoiceEvent {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(i) = self.voice.iter().position(&predicate) {
                    return self.voice.remove(i);
                }
                self.recv().await;
            }
        })
        .await
        .expect("voice event deadline")
    }
    async fn ack(&mut self, signal: VoiceSignal) {
        self.signal(signal).await;
        self.voice(|e| matches!(e, VoiceEvent::Ack)).await;
    }
    async fn join(&mut self, user: Id, channel: Id) -> Media {
        self.signal(VoiceSignal::Join {
            channel_id: channel,
            capabilities: VoiceCapabilities {
                video: vec![VideoCodec::H264],
                hardware_decode: vec![VideoCodec::H264],
            },
        })
        .await;
        self.voice(|v| matches!(v, VoiceEvent::Joined { .. })).await;
        // Joined is correlated; its following self Participant identifies this
        // fresh call, not an older GuildState still buffered by the chat socket.
        self.voice
            .retain(|v| !matches!(v,VoiceEvent::Participant(p) if p.session.user_id==user));
        let VoiceEvent::Participant(participant) = self
            .voice(|v| matches!(v, VoiceEvent::Participant(p) if p.session.user_id == user))
            .await
        else {
            unreachable!()
        };
        let session = participant.session;
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut rtc = sticktalk_sfu::rtc_config().build(std::time::Instant::now());
        rtc.add_local_candidate(
            str0m::Candidate::host(socket.local_addr().unwrap(), "udp").unwrap(),
        )
        .unwrap();
        drain_initial(&mut rtc);
        let mut api = rtc.sdp_api();
        let audio = api.add_media(
            str0m::media::MediaKind::Audio,
            str0m::media::Direction::SendOnly,
            None,
            None,
            None,
        );
        let camera = api.add_media(
            str0m::media::MediaKind::Video,
            str0m::media::Direction::SendOnly,
            None,
            None,
            None,
        );
        let screen = api.add_media(
            str0m::media::MediaKind::Video,
            str0m::media::Direction::SendOnly,
            None,
            None,
            None,
        );
        let receive_audio = api.add_media(
            str0m::media::MediaKind::Audio,
            str0m::media::Direction::RecvOnly,
            None,
            None,
            None,
        );
        let receive_camera = api.add_media(
            str0m::media::MediaKind::Video,
            str0m::media::Direction::RecvOnly,
            None,
            None,
            None,
        );
        let receive_screen = api.add_media(
            str0m::media::MediaKind::Video,
            str0m::media::Direction::RecvOnly,
            None,
            None,
            None,
        );
        let control = api.add_channel_with_config(str0m::channel::ChannelConfig {
            label: CONTROL_CHANNEL_LABEL.into(),
            ..Default::default()
        });
        let (offer, pending) = api.apply().unwrap();
        drain_initial(&mut rtc);
        let tracks = [
            (audio, TrackKind::Audio, None),
            (camera, TrackKind::Camera, Some(VideoCodec::H264)),
            (screen, TrackKind::Screen, Some(VideoCodec::H264)),
        ]
        .into_iter()
        .map(|(mid, kind, codec)| PublishedTrack {
            mid: mid.to_string(),
            kind,
            codec,
            layers: vec![],
            surface: (kind == TrackKind::Screen).then_some(ShareSurface::Window),
        })
        .collect();
        self.signal(VoiceSignal::Offer {
            sdp: offer.to_sdp_string(),
            tracks,
        })
        .await;
        let VoiceEvent::Answer { sdp } =
            self.voice(|v| matches!(v, VoiceEvent::Answer { .. })).await
        else {
            unreachable!()
        };
        rtc.sdp_api()
            .accept_answer(
                pending,
                str0m::change::SdpAnswer::from_sdp_string(&sdp).unwrap(),
            )
            .unwrap();
        let (commands, rx) = tokio::sync::mpsc::channel(32);
        let (tx, events) = tokio::sync::mpsc::channel(128);
        let mids = [audio, camera, screen];
        let task = tokio::spawn(media_loop(rtc, socket, mids, control, rx, tx));
        let mut media = Media {
            session,
            mids,
            receive: [receive_audio, receive_camera, receive_screen],
            commands,
            events,
            buffered: vec![],
            task,
        };
        media.wait(|e| matches!(e, MediaEvent::Channel)).await;
        media
    }
    async fn subscribe(&mut self, receiver: &Media, sender: &Media, kind: usize) {
        self.ack(VoiceSignal::Subscribe {
            publisher: sender.session,
            source_mid: sender.mids[kind].to_string(),
            receiver_mid: receiver.receive[kind].to_string(),
            rid: None,
        })
        .await;
    }
}
fn drain_initial(rtc: &mut str0m::Rtc) {
    while !matches!(rtc.poll_output().unwrap(), str0m::Output::Timeout(_)) {}
}
#[derive(Debug)]
enum MediaEvent {
    Rtp(Vec<u8>),
    Data(ControlData),
    Channel,
}
enum MediaCommand {
    Rtp(usize, u64, Vec<u8>),
    Data(ControlData),
}
struct Media {
    session: VoiceSession,
    mids: [str0m::media::Mid; 3],
    receive: [str0m::media::Mid; 3],
    commands: tokio::sync::mpsc::Sender<MediaCommand>,
    events: tokio::sync::mpsc::Receiver<MediaEvent>,
    buffered: Vec<MediaEvent>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Media {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Media {
    async fn wait(&mut self, predicate: impl Fn(&MediaEvent) -> bool) -> MediaEvent {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(i) = self.buffered.iter().position(&predicate) {
                    return self.buffered.remove(i);
                }
                self.buffered
                    .push(self.events.recv().await.expect("media driver stopped"));
            }
        })
        .await
        .expect("media event timeout")
    }
    async fn send(&self, kind: usize, seq: u64, data: &[u8]) {
        self.commands
            .send(MediaCommand::Rtp(kind, seq, data.to_vec()))
            .await
            .unwrap();
    }
    async fn data(&self, message: ControlData) {
        self.commands
            .send(MediaCommand::Data(message))
            .await
            .unwrap();
    }
    fn clear(&mut self) {
        self.buffered.clear();
        while self.events.try_recv().is_ok() {}
    }
    async fn no_input(&mut self) {
        tokio::time::timeout(Duration::from_millis(250), async {
            while let Some(event) = self.events.recv().await {
                assert!(
                    !matches!(event, MediaEvent::Data(ControlData::Input { .. })),
                    "unauthorized input forwarded"
                );
            }
        })
        .await
        .expect_err("driver unexpectedly closed");
    }
    async fn no_rtp(&mut self, payload: &[u8]) {
        tokio::time::timeout(Duration::from_millis(250), async {
            while let Some(event) = self.events.recv().await {
                assert!(
                    !matches!(event,MediaEvent::Rtp(p) if p == payload),
                    "revoked media forwarded"
                );
            }
        })
        .await
        .expect_err("driver unexpectedly closed");
    }
}
async fn media_loop(
    mut rtc: str0m::Rtc,
    socket: tokio::net::UdpSocket,
    mids: [str0m::media::Mid; 3],
    control: str0m::channel::ChannelId,
    mut commands: tokio::sync::mpsc::Receiver<MediaCommand>,
    events: tokio::sync::mpsc::Sender<MediaEvent>,
) {
    use str0m::{Event, Input, Output};
    let mut bytes = vec![0; 65536];
    loop {
        let deadline = loop {
            match rtc.poll_output().unwrap() {
                Output::Timeout(t) => break t,
                Output::Transmit(tx) => {
                    assert!(tx.contents.len() <= 1472);
                    socket.send_to(&tx.contents, tx.destination).await.unwrap();
                }
                Output::Event(Event::RtpPacket(p)) => events
                    .try_send(MediaEvent::Rtp(p.payload.to_vec()))
                    .unwrap(),
                Output::Event(Event::ChannelData(d)) => events
                    .try_send(MediaEvent::Data(rmp_serde::from_slice(&d.data).unwrap()))
                    .unwrap(),
                Output::Event(Event::ChannelOpen(_, _)) => {
                    events.try_send(MediaEvent::Channel).unwrap()
                }
                _ => {}
            }
        };
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(MediaCommand::Rtp(kind,seq,payload)) => {
                        let mid=mids[kind];
                        let codec=if kind==0 {str0m::format::Codec::Opus} else {str0m::format::Codec::H264};
                        let pt=rtc.codec_config().params().iter().find(|p| p.spec().codec==codec && rtc.media(mid).unwrap().remote_pts().contains(&p.pt())).unwrap().pt();
                        let ssrc=rtc.direct_api().stream_tx_by_mid(mid,None).unwrap().ssrc();
                        let mut direct=rtc.direct_api();
                        let stream=direct.stream_tx(&ssrc).unwrap();
                        stream.set_unpaced(true);
                        stream.write_rtp(str0m::rtp::RtpWrite::new(pt,seq.into(),seq as u32 * if kind==0 {960} else {1500},std::time::Instant::now(),payload).marker(kind!=0).ext_vals(str0m::rtp::ExtensionValues { audio_level:Some(-24), voice_activity:Some(true), ..Default::default() }).nackable(kind!=0));
                    }
                    Some(MediaCommand::Data(message)) => { assert!(rtc.channel(control).unwrap().write(true,&rmp_serde::to_vec_named(&message).unwrap()).unwrap()); }
                    None => return,
                }
                // Drain after writes before feeding another str0m input.
            }
            received=socket.recv_from(&mut bytes) => {
                let (len,from)=received.unwrap();
                let receive=str0m::net::Receive::new(str0m::net::Protocol::Udp,from,socket.local_addr().unwrap(),&bytes[..len]).unwrap();
                rtc.handle_input(Input::Receive(std::time::Instant::now(),receive)).unwrap();
            }
            _=tokio::time::sleep_until(deadline.into()) => { rtc.handle_input(Input::Timeout(std::time::Instant::now())).unwrap(); }
        }
    }
}
async fn guild(owner: &mut Peer, members: &mut [&mut Peer]) -> (Id, Id) {
    let CommandResult::Guild(g) = owner
        .command(Command::CreateGuild {
            name: "Voice guild".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let id = g.summary.guild.id;
    let CommandResult::Invite(invite) = owner
        .command(Command::CreateInvite {
            guild_id: id,
            ttl_seconds: 3600,
            max_uses: 10,
        })
        .await
    else {
        panic!()
    };
    for peer in members {
        peer.command(Command::JoinGuild {
            invite_code: invite.code.clone(),
        })
        .await;
    }
    let CommandResult::Channel(ch) = owner
        .command(Command::CreateChannel {
            guild_id: id,
            kind: ChannelKind::Voice,
            name: "Lounge".into(),
            parent_id: None,
            topic: None,
        })
        .await
    else {
        panic!()
    };
    (id, ch.summary.id)
}
fn input(sharer: VoiceSession, grant: u64, sequence: u64) -> ControlData {
    ControlData::Input {
        sharer,
        grant,
        sequence,
        input: ControlInput::Key {
            usage: 4,
            down: true,
        },
    }
}

#[tokio::test]
async fn full_server_real_peers_media_control_revoke_kick_and_reconnect() {
    let s = TestServer::start().await;
    let a = s.register("voice_owner").await;
    let b = s.register("voice_second").await;
    let c = s.register("voice_third").await;
    let (mut pa, _) = Peer::connect(&s, &a, None).await;
    let (mut pb, _) = Peer::connect(&s, &b, None).await;
    let (mut pc, _) = Peer::connect(&s, &c, None).await;
    let (guild_id, channel) = guild(&mut pa, &mut [&mut pb, &mut pc]).await;
    let mut ma = pa.join(a.user.user.id, channel).await;
    let mut mb = pb.join(b.user.user.id, channel).await;
    // Even a member not in the call sees both participants and publications.
    for session in [ma.session, mb.session] {
        pc.voice(|e| matches!(e,VoiceEvent::GuildState(v) if v.participant.session==session))
            .await;
    }
    let mut mc = pc.join(c.user.user.id, channel).await;
    pa.subscribe(&ma, &mb, 0).await;
    pb.subscribe(&mb, &ma, 0).await;
    pc.subscribe(&mc, &ma, 1).await;
    pc.subscribe(&mc, &ma, 2).await;
    ma.send(0, 1, &[0xf8, 0xff, 0xfe]).await;
    mb.send(0, 1, &[0xf8, 0x22, 0x11]).await;
    mb.wait(|e| matches!(e,MediaEvent::Rtp(p) if p==&[0xf8,0xff,0xfe]))
        .await;
    ma.wait(|e| matches!(e,MediaEvent::Rtp(p) if p==&[0xf8,0x22,0x11]))
        .await;
    // Standard-browser-sized encrypted RTP (~1200 UDP bytes), >1060 payload.
    let mut browser_packet = vec![7; 1180];
    browser_packet[0] = 0x65;
    ma.send(2, 1, &browser_packet).await;
    mc.wait(|e| matches!(e,MediaEvent::Rtp(p) if p==&browser_packet))
        .await;
    ma.send(1, 1, &[0x65, 42, 43]).await;
    mc.wait(|e| matches!(e,MediaEvent::Rtp(p) if p==&[0x65,42,43]))
        .await;
    pa.ack(VoiceSignal::State {
        muted: true,
        deafened: true,
    })
    .await;
    pc.voice(|e| matches!(e,VoiceEvent::GuildState(v) if v.participant.session==ma.session && v.participant.muted && v.participant.deafened && v.participant.tracks.iter().any(|t|t.kind==TrackKind::Screen) && v.participant.tracks.iter().any(|t|t.kind==TrackKind::Camera))).await;
    ma.send(0, 2, &[0xf8, 0x70, 0x71]).await;
    mb.no_rtp(&[0xf8, 0x70, 0x71]).await;
    mb.send(0, 2, &[0xf8, 0x72, 0x73]).await;
    ma.no_rtp(&[0xf8, 0x72, 0x73]).await;
    let (observer, ready) = Peer::connect(&s, &a, None).await; // replaces old owner's WS
    let ServerMsg::Ready(ready) = ready else {
        panic!()
    };
    assert!(ready
        .voice_states
        .iter()
        .any(|v| v.participant.session == mb.session));
    // Reconnect has no old SFU identity or grants and rejoins with fresh crypto.
    let old_session = ma.session;
    drop(ma);
    drop(pa);
    pa = observer;
    ma = pa.join(a.user.user.id, channel).await;
    assert_ne!(old_session, ma.session);
    pc.subscribe(&mc, &ma, 2).await;
    ma.send(2, 1, &[0x65, 91, 92]).await;
    mc.wait(|e| matches!(e,MediaEvent::Rtp(p) if p==&[0x65,91,92]))
        .await;

    // No pending request => unsolicited consent cannot grant control.
    pa.signal(VoiceSignal::DecideControl {
        requester: mc.session,
        allow: true,
    })
    .await;
    pa.voice(|e| matches!(e, VoiceEvent::Error { .. })).await;
    mc.data(input(ma.session, 1234, 1)).await;
    ma.no_input().await;
    pc.ack(VoiceSignal::RequestControl { sharer: ma.session })
        .await;
    pa.voice(|e| matches!(e,VoiceEvent::ControlRequested {requester,..} if *requester==mc.session))
        .await;
    pa.ack(VoiceSignal::DecideControl {
        requester: mc.session,
        allow: true,
    })
    .await;
    let VoiceEvent::ControlGranted { grant, .. } = pc
        .voice(
            |e| matches!(e,VoiceEvent::ControlGranted {controller,..} if *controller==mc.session),
        )
        .await
    else {
        panic!()
    };
    mb.data(input(ma.session, grant, 1)).await;
    ma.no_input().await;
    mc.data(input(ma.session, grant, 1)).await;
    ma.wait(|e| matches!(e,MediaEvent::Data(ControlData::Input {grant:g,..}) if *g==grant))
        .await;
    pa.ack(VoiceSignal::RevokeControl).await;
    ma.wait(|e| matches!(e,MediaEvent::Data(ControlData::ReleaseAll {grant:g}) if *g==grant))
        .await;
    mc.data(input(ma.session, grant, 2)).await;
    ma.no_input().await;

    // A live Connect revocation closes both media directions and control grants.
    pc.ack(VoiceSignal::RequestControl { sharer: ma.session })
        .await;
    pa.voice(|e| matches!(e, VoiceEvent::ControlRequested { .. }))
        .await;
    pa.ack(VoiceSignal::DecideControl {
        requester: mc.session,
        allow: true,
    })
    .await;
    let VoiceEvent::ControlGranted { grant, .. } = pc
        .voice(|e| matches!(e, VoiceEvent::ControlGranted { .. }))
        .await
    else {
        panic!()
    };
    pa.command(Command::SetVoicePermission {
        channel_id: channel,
        user_id: c.user.user.id,
        connect: false,
    })
    .await;
    pc.voice(|e| matches!(e,VoiceEvent::Left {session} if *session==mc.session))
        .await;
    ma.wait(|e| matches!(e,MediaEvent::Data(ControlData::ReleaseAll {grant:g}) if *g==grant))
        .await;
    mc.clear();
    ma.send(2, 2, &[0x65, 93, 94]).await;
    mc.no_rtp(&[0x65, 93, 94]).await;
    mc.data(input(ma.session, grant, 3)).await;
    ma.no_input().await;
    pc.signal(VoiceSignal::Join {
        channel_id: channel,
        capabilities: VoiceCapabilities {
            video: vec![VideoCodec::H264],
            hardware_decode: vec![VideoCodec::H264],
        },
    })
    .await;
    pc.voice(|e| {
        matches!(
            e,
            VoiceEvent::Error {
                code: ErrorCode::Forbidden,
                ..
            }
        )
    })
    .await;
    pa.command(Command::SetVoicePermission {
        channel_id: channel,
        user_id: c.user.user.id,
        connect: true,
    })
    .await;
    drop(mc);
    mc = pc.join(c.user.user.id, channel).await;
    pc.subscribe(&mc, &ma, 2).await;
    ma.send(2, 3, &[0x65, 95, 96]).await;
    mc.wait(|e| matches!(e,MediaEvent::Rtp(p) if p==&[0x65,95,96]))
        .await;
    pa.command(Command::KickMember {
        guild_id,
        user_id: c.user.user.id,
    })
    .await;
    pc.voice(|e| matches!(e,VoiceEvent::Left {session} if *session==mc.session))
        .await;
    mc.clear();
    ma.send(2, 4, &[0x65, 97, 98]).await;
    mc.no_rtp(&[0x65, 97, 98]).await;
    // Leave and same-socket rejoin must not be removed by delayed old Left events.
    pb.signal(VoiceSignal::Leave).await;
    pb.voice(|e| matches!(e,VoiceEvent::Left {session} if *session==mb.session))
        .await;
    let old = mb.session;
    drop(mb);
    mb = pb.join(b.user.user.id, channel).await;
    assert_ne!(old, mb.session);
    pb.subscribe(&mb, &ma, 0).await;
    ma.send(0, 2, &[0xf8, 0x55, 0x66]).await;
    mb.wait(|e| matches!(e,MediaEvent::Rtp(p) if p==&[0xf8,0x55,0x66]))
        .await;
    let cursor = pb.close().await;
    let old = mb.session;
    drop(mb);
    let (resumed, handshake) = Peer::connect(&s, &b, Some(cursor)).await;
    assert!(matches!(handshake, ServerMsg::Resumed(_)));
    pb = resumed;
    let VoiceEvent::GuildSnapshot { states } = pb
        .voice(|e| matches!(e, VoiceEvent::GuildSnapshot { .. }))
        .await
    else {
        panic!()
    };
    assert!(!states.iter().any(|v| v.participant.session == old));
    assert!(states.iter().any(|v| v.participant.session == ma.session));
    mb = pb.join(b.user.user.id, channel).await;
    pb.subscribe(&mb, &ma, 0).await;
    ma.send(0, 3, &[0xf8, 0x75, 0x76]).await;
    mb.wait(|e| matches!(e,MediaEvent::Rtp(p) if p==&[0xf8,0x75,0x76]))
        .await;
    pa.close().await;
    pb.close().await;
    pc.close().await;
    drop((ma, mb, mc));
    s.stop().await;
}

#[tokio::test]
async fn voice_channel_permissions_delete_logout_and_session_expiry() {
    let s = TestServer::start().await;
    let a = s.register("lifecycle_owner").await;
    let b = s.register("lifecycle_guest").await;
    let (mut pa, _) = Peer::connect(&s, &a, None).await;
    let (mut pb, _) = Peer::connect(&s, &b, None).await;
    let (g, ch) = guild(&mut pa, &mut [&mut pb]).await;
    let denied = pb
        .command_nonce(
            50,
            Command::CreateChannel {
                guild_id: g,
                kind: ChannelKind::Voice,
                name: "No".into(),
                parent_id: None,
                topic: None,
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
    for (nonce, command) in [
        (
            51,
            Command::UpdateChannel {
                channel_id: ch,
                patch: ChannelPatch {
                    name: Patch::Set("Not allowed".into()),
                    topic: Patch::Keep,
                },
            },
        ),
        (52, Command::DeleteChannel { channel_id: ch }),
        (
            53,
            Command::SetVoicePermission {
                channel_id: ch,
                user_id: a.user.user.id,
                connect: false,
            },
        ),
    ] {
        assert!(matches!(
            pb.command_nonce(nonce, command).await.outcome,
            CommandOutcome::Err(ApiError {
                code: ErrorCode::Forbidden,
                ..
            })
        ));
    }
    pa.command(Command::UpdateChannel {
        channel_id: ch,
        patch: ChannelPatch {
            name: Patch::Set("Renamed voice".into()),
            topic: Patch::Keep,
        },
    })
    .await;
    let mut media = pb.join(b.user.user.id, ch).await;
    pa.command(Command::DeleteChannel { channel_id: ch }).await;
    pb.voice(|e| matches!(e,VoiceEvent::Left {session} if *session==media.session))
        .await;
    media.clear();
    pa.voice(|e| matches!(e,VoiceEvent::GuildLeft {session,..} if *session==media.session))
        .await;
    drop(media);
    let CommandResult::Channel(ch) = pa
        .command(Command::CreateChannel {
            guild_id: g,
            kind: ChannelKind::Voice,
            name: "Next".into(),
            parent_id: None,
            topic: None,
        })
        .await
    else {
        panic!()
    };
    let media = pb.join(b.user.user.id, ch.summary.id).await;
    // Real session timeout via the persisted session clock, maintenance and ACL.
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite:{}",
        s._dir.path().join("sticktalk.sqlite3").display()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE sessions SET expires_at=1 WHERE user_id=?")
        .bind(b.user.user.id.0 as i64)
        .execute(&pool)
        .await
        .unwrap();
    s.app.housekeeping().await.unwrap();
    pa.voice(|e| matches!(e,VoiceEvent::GuildLeft {session,..} if *session==media.session))
        .await;
    drop(media);
    drop(pb);
    let media = pa.join(a.user.user.id, ch.summary.id).await;
    let r = reqwest::Client::new()
        .post(format!("{}/api/v1/auth/logout", s.url))
        .bearer_auth(&a.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    drop(media);
    drop(pa);
    pool.close().await;
    s.stop().await;
}

#[tokio::test]
async fn role_overwrites_and_moderation_stop_real_media_and_move_without_reconnect() {
    let s = TestServer::start().await;
    let a = s.register("role_voice_owner").await;
    let b = s.register("role_voice_member").await;
    let (mut pa, _) = Peer::connect(&s, &a, None).await;
    let (mut pb, _) = Peer::connect(&s, &b, None).await;
    let (g, ch) = guild(&mut pa, &mut [&mut pb]).await;
    let mut ma = pa.join(a.user.user.id, ch).await;
    let mut mb = pb.join(b.user.user.id, ch).await;
    pa.subscribe(&ma, &mb, 0).await;
    pa.subscribe(&ma, &mb, 2).await;
    pb.subscribe(&mb, &ma, 0).await;
    mb.send(0, 1, &[0xf8, 9, 1]).await;
    ma.wait(|e| matches!(e,MediaEvent::Rtp(p) if p==&[0xf8,9,1]))
        .await;
    pa.ack(VoiceSignal::Moderate {
        session: mb.session,
        action: VoiceModeration::Mute { muted: true },
    })
    .await;
    pb.ack(VoiceSignal::State {
        muted: false,
        deafened: false,
    })
    .await;
    ma.clear();
    mb.send(0, 2, &[0xf8, 9, 2]).await;
    ma.no_rtp(&[0xf8, 9, 2]).await;
    pa.ack(VoiceSignal::Moderate {
        session: mb.session,
        action: VoiceModeration::Mute { muted: false },
    })
    .await;
    pa.ack(VoiceSignal::Moderate {
        session: mb.session,
        action: VoiceModeration::Deafen { deafened: true },
    })
    .await;
    mb.clear();
    ma.send(0, 3, &[0xf8, 9, 3]).await;
    mb.no_rtp(&[0xf8, 9, 3]).await;
    pa.ack(VoiceSignal::Moderate {
        session: mb.session,
        action: VoiceModeration::Deafen { deafened: false },
    })
    .await;
    pa.command(Command::Launch(LaunchCommand::SetOverwrites {
        channel_id: ch,
        overwrites: vec![PermissionOverwrite {
            kind: OverwriteKind::Member,
            target_id: b.user.user.id,
            allow: Permissions::empty(),
            deny: Permissions::SPEAK | Permissions::STREAM,
        }],
    }))
    .await;
    ma.clear();
    mb.send(0, 4, &[0xf8, 9, 4]).await;
    mb.send(2, 4, &[0x65, 9, 4]).await;
    ma.no_rtp(&[0xf8, 9, 4]).await;
    ma.no_rtp(&[0x65, 9, 4]).await;
    // Restore rights, then move the existing transport into a second room.
    pa.command(Command::Launch(LaunchCommand::SetOverwrites {
        channel_id: ch,
        overwrites: vec![],
    }))
    .await;
    let CommandResult::Channel(next) = pa
        .command(Command::CreateChannel {
            guild_id: g,
            kind: ChannelKind::Voice,
            name: "Destination".into(),
            parent_id: None,
            topic: None,
        })
        .await
    else {
        panic!()
    };
    pa.ack(VoiceSignal::Moderate {
        session: mb.session,
        action: VoiceModeration::Move {
            channel_id: next.summary.id,
        },
    })
    .await;
    pb.voice(|e| matches!(e,VoiceEvent::Joined{channel_id,..} if *channel_id==next.summary.id))
        .await;
    pb.signal(VoiceSignal::Stats).await;
    pb.voice(|e| matches!(e, VoiceEvent::Stats(_))).await;
    pa.ack(VoiceSignal::Moderate {
        session: mb.session,
        action: VoiceModeration::Disconnect,
    })
    .await;
    pb.voice(|e| matches!(e,VoiceEvent::Left{session} if *session==mb.session))
        .await;
    drop(ma);
    drop(mb);
    drop(pa);
    drop(pb);
    s.stop().await;
}

#[tokio::test]
async fn failed_sfu_negotiation_reports_clean_error_and_other_calls_continue() {
    let s = TestServer::start().await;
    let a = s.register("robust_owner").await;
    let b = s.register("robust_receiver").await;
    let c = s.register("robust_bad").await;
    let (mut pa, _) = Peer::connect(&s, &a, None).await;
    let (mut pb, _) = Peer::connect(&s, &b, None).await;
    let (mut pc, _) = Peer::connect(&s, &c, None).await;
    let (_, channel) = guild(&mut pa, &mut [&mut pb, &mut pc]).await;
    let ma = pa.join(a.user.user.id, channel).await;
    let mut mb = pb.join(b.user.user.id, channel).await;
    let mc = pc.join(c.user.user.id, channel).await;
    pb.subscribe(&mb, &ma, 0).await;
    let mut rtc = sticktalk_sfu::rtc_config().build(std::time::Instant::now());
    drain_initial(&mut rtc);
    let mut api = rtc.sdp_api();
    api.add_media(
        str0m::media::MediaKind::Audio,
        str0m::media::Direction::RecvOnly,
        None,
        None,
        None,
    );
    let (offer, _) = api.apply().unwrap();
    let sdp = offer
        .to_sdp_string()
        .lines()
        .map(|line| {
            if line.starts_with("a=group:BUNDLE") {
                "a=group:BUNDLE private_invalid_mid"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n";
    pc.signal(VoiceSignal::Offer {
        sdp,
        tracks: vec![],
    })
    .await;
    let event = pc
        .voice(|event| {
            matches!(
                event,
                VoiceEvent::Error {
                    code: ErrorCode::Unavailable,
                    ..
                }
            )
        })
        .await;
    let VoiceEvent::Error { message, .. } = event else {
        unreachable!()
    };
    assert_eq!(message, "voice peer failed; leave and rejoin the call");
    pc.voice(|event| matches!(event,VoiceEvent::Left {session} if *session==mc.session))
        .await;
    ma.send(0, 1, &[0xf8, 0x11, 0x22]).await;
    mb.wait(|event| matches!(event,MediaEvent::Rtp(bytes) if bytes==&[0xf8,0x11,0x22]))
        .await;
    pa.ack(VoiceSignal::State {
        muted: false,
        deafened: false,
    })
    .await;
    pb.ack(VoiceSignal::State {
        muted: false,
        deafened: false,
    })
    .await;
    drop(ma);
    drop(mb);
    drop(mc);
    pa.close().await;
    pb.close().await;
    pc.close().await;
    s.stop().await;
}
