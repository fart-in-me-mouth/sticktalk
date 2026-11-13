//! Real loopback HTTP/WS/SRTP endpoints with FILE fixtures. No devices/windows.
#![cfg(windows)]
use futures_util::{SinkExt, StreamExt};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use sticktalk::voice::{
    peer::{Peer, PeerOutput},
    rtp,
};
use sticktalk_client_core::voice::{Call, Effect};
use sticktalk_media::voice::{OpusEncoder, StereoDecoder};
use sticktalk_proto::*;
use sticktalk_server::{App, Config};
use tokio::{
    net::{TcpListener, TcpStream, UdpSocket},
    sync::mpsc,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as Frame},
    MaybeTlsStream, WebSocketStream,
};
#[path = "support/gpu.rs"]
mod gpu_usage;
#[path = "support/input_window.rs"]
mod input_window;
// Compile the exact production SendInput seam, without publishing an unchecked
// injection API in the shipping client. Only the ignored opt-in test uses it.
use sticktalk::voice::control::{Injector, Surface};
#[path = "../src/voice/control_native.rs"]
mod native_injection;
#[derive(Default)]
struct TestInjector {
    events: Vec<ControlInput>,
    native: Option<native_injection::NativeInjector>,
}
impl Injector for TestInjector {
    fn input(&mut self, input: &ControlInput, surface: Surface) -> Result<(), String> {
        if let Some(native) = &mut self.native {
            native.input(input, surface)?;
        }
        self.events.push(input.clone());
        Ok(())
    }
}
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Server {
    app: App,
    url: String,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
    _dir: tempfile::TempDir,
}
impl Server {
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let serving = app.clone();
        let task = tokio::spawn(async move { serving.serve(listener).await });
        Self {
            app,
            url,
            task,
            _dir: dir,
        }
    }
    async fn account(&self, name: &str) -> AuthResponse {
        reqwest::Client::new()
            .post(format!("{}/api/v1/auth/register", self.url))
            .json(&RegisterRequest {
                username: name.into(),
                password: "correct horse battery".into(),
                display_name: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }
    async fn connect(&self, auth: &AuthResponse) -> Socket {
        let mut request = format!("{}/api/v1/ws", self.url.replace("http:", "ws:"))
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
        let (mut ws, _) = connect_async(request).await.unwrap();
        assert!(matches!(read(&mut ws).await, ServerMsg::Hello(_)));
        send(
            &mut ws,
            ClientMsg::Identify(Identify {
                token: auth.token.clone(),
                ..Default::default()
            }),
        )
        .await;
        loop {
            if matches!(read(&mut ws).await, ServerMsg::Ready(_)) {
                return ws;
            }
        }
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
async fn send(ws: &mut Socket, msg: ClientMsg) {
    ws.send(Frame::Binary(rmp_serde::to_vec_named(&msg).unwrap().into()))
        .await
        .unwrap();
}
async fn read(ws: &mut Socket) -> ServerMsg {
    loop {
        let f = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Frame::Binary(b) = f {
            return sticktalk::backend::decode(&b).unwrap();
        }
    }
}
async fn command(ws: &mut Socket, command: Command) -> CommandResult {
    let nonce = sticktalk::backend::nonce();
    send(ws, ClientMsg::Command(CommandRequest { nonce, command })).await;
    loop {
        if let ServerMsg::Response(r) = read(ws).await {
            if r.nonce == nonce {
                return match r.outcome {
                    CommandOutcome::Ok(v) => v,
                    CommandOutcome::Err(e) => panic!("{e:?}"),
                };
            }
        }
    }
}
async fn room_details(
    server: &Server,
    a: &AuthResponse,
    b: &AuthResponse,
) -> (Socket, Socket, Id, Id) {
    let mut aw = server.connect(a).await;
    let mut bw = server.connect(b).await;
    let CommandResult::Guild(g) = command(
        &mut aw,
        Command::CreateGuild {
            name: "Native call".into(),
            icon_id: None,
        },
    )
    .await
    else {
        panic!()
    };
    let id = g.summary.guild.id;
    let CommandResult::Invite(i) = command(
        &mut aw,
        Command::CreateInvite {
            guild_id: id,
            ttl_seconds: 3600,
            max_uses: 10,
        },
    )
    .await
    else {
        panic!()
    };
    command(
        &mut bw,
        Command::JoinGuild {
            invite_code: i.code,
        },
    )
    .await;
    let CommandResult::Channel(c) = command(
        &mut aw,
        Command::CreateChannel {
            guild_id: id,
            kind: ChannelKind::Voice,
            name: "Lounge".into(),
            parent_id: None,
            topic: None,
        },
    )
    .await
    else {
        panic!()
    };
    (aw, bw, c.summary.id, id)
}
async fn room(server: &Server, a: &AuthResponse, b: &AuthResponse) -> (Socket, Socket, Id) {
    let (aw, bw, channel, _) = room_details(server, a, b).await;
    (aw, bw, channel)
}
enum Input {
    Unpublish(TrackKind),
    Target(TrackKind, u64),
    Control(ControlData),
    Signal(VoiceSignal),
    Packet(TrackKind, u32, Arc<[u8]>, bool),
    Publish(TrackKind),
    Stop,
}
enum Received {
    Keyframe(TrackKind),
    Control(Vec<u8>),
    Signal(VoiceEvent),
    Packet(TrackKind, u32, Vec<u8>, bool, u64),
    Connected(VoiceSession),
}
struct Endpoint {
    input: mpsc::Sender<Input>,
    received: mpsc::Receiver<Received>,
    task: tokio::task::JoinHandle<()>,
}
impl Endpoint {
    fn start(mut ws: Socket, user: Id, channel: Id) -> Self {
        let (input, mut commands) = mpsc::channel(128);
        let (output, received) = mpsc::channel(512);
        let task = tokio::spawn(async move {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let (before, after) = sticktalk::voice::peer::configure_socket(&socket).unwrap();
            println!("bounded UDP receive buffer: before={before} after={after}; send=262144");
            let local = socket.local_addr().unwrap();
            let mut call = Call::default();
            let nonce = sticktalk::backend::nonce();
            let mut effects = call.join(channel, nonce);
            let mut peer: Option<Peer> = None;
            let mut heartbeat = tokio::time::interval(Duration::from_secs(25));
            let mut buffer = [0; 65536];
            let mut clock = sticktalk_sfu::pacing::Clock::new().unwrap();
            loop {
                for e in effects.drain(..) {
                    match e {
                        Effect::Signal(s) => {
                            send(
                                &mut ws,
                                ClientMsg::Voice(
                                    sticktalk_client_core::voice::encode(nonce, s).unwrap(),
                                ),
                            )
                            .await
                        }
                        Effect::StartTransport { max_rtp_bytes } => {
                            peer = Some(Peer::new(local, max_rtp_bytes, Instant::now()).unwrap())
                        }
                        Effect::StopTransport => peer = None,
                        _ => {}
                    }
                }
                if let Some(p) = &mut peer {
                    for stage in 0..2 {
                        for e in p.drain().unwrap() {
                            match e {
                                PeerOutput::Signal(s) => {
                                    send(
                                        &mut ws,
                                        ClientMsg::Voice(
                                            sticktalk_client_core::voice::encode(nonce, s).unwrap(),
                                        ),
                                    )
                                    .await
                                }
                                PeerOutput::Datagram(to, b) => {
                                    socket.send_to(&b, to).await.unwrap();
                                }
                                PeerOutput::Connected => {
                                    output
                                        .send(Received::Connected(call.own.unwrap()))
                                        .await
                                        .unwrap();
                                }
                                PeerOutput::Packet(remote, p) => {
                                    output
                                        .send(Received::Packet(
                                            remote.track.kind,
                                            p.header.timestamp,
                                            p.payload.to_vec(),
                                            p.header.marker,
                                            *p.seq_no,
                                        ))
                                        .await
                                        .unwrap();
                                }
                                PeerOutput::Keyframe(kind) => {
                                    output.send(Received::Keyframe(kind)).await.unwrap();
                                }
                                PeerOutput::Control(data) => {
                                    output.send(Received::Control(data)).await.unwrap()
                                }
                            }
                        }
                        if stage == 0 {
                            p.reconcile().unwrap();
                        }
                    }
                }
                let deadline = peer
                    .as_ref()
                    .map_or(Instant::now() + Duration::from_secs(30), Peer::deadline);
                tokio::select! {
                    _=heartbeat.tick()=>send(&mut ws,ClientMsg::Heartbeat(Heartbeat{nonce:sticktalk::backend::nonce(),last_seq:0})).await,
                    frame=ws.next()=>{
                        let Some(Ok(Frame::Binary(b)))=frame else {break;};
                        if let ServerMsg::Voice(v)=sticktalk::backend::decode(&b).unwrap() {
                            let event=sticktalk_client_core::voice::decode(&v).unwrap();
                            effects=call.event(v.nonce,&event,user,0);
                            if let Some(p)=&mut peer {
                                if let VoiceEvent::Answer {sdp}=&event {p.answer(sdp).unwrap();}
                                p.remote_tracks(call.own,call.participants.values().cloned());
                            }
                            if let VoiceEvent::Error {ref message,..}=event {panic!("SFU error: {message}");}
                            output.send(Received::Signal(event)).await.unwrap();
                        }
                    }
                    packet=socket.recv_from(&mut buffer)=>{let (n,from)=packet.unwrap();if let Some(p)=&mut peer {p.datagram(from,&buffer[..n],Instant::now()).unwrap();}}
                    c=commands.recv()=>match c {
                        Some(Input::Target(kind,bps))=>if let Some(p)=&mut peer { p.set_send_bitrate(kind,bps); },
                        Some(Input::Control(data))=>peer.as_mut().unwrap().control(&rmp_serde::to_vec_named(&data).unwrap()).unwrap(),
                        Some(Input::Signal(s))=>send(&mut ws,ClientMsg::Voice(sticktalk_client_core::voice::encode(nonce,s).unwrap())).await,
                        Some(Input::Packet(kind,time,data,marker))=>if let Some(p)=&mut peer {p.send(kind,time,&data,marker,-24,Instant::now()).unwrap();},
                        Some(Input::Publish(kind))=>peer.as_mut().unwrap().publish(kind,true,ShareSurface::Monitor).unwrap(),
                        Some(Input::Unpublish(kind))=>peer.as_mut().unwrap().publish(kind,false,ShareSurface::Monitor).unwrap(),
                        _=>break,
                    },
                    timer=clock.wait(deadline)=> { timer.unwrap(); if let Some(p)=&mut peer {p.timeout(Instant::now()).unwrap();} },
                }
            }
        });
        Self {
            input,
            received,
            task,
        }
    }
    async fn connected(&mut self) -> VoiceSession {
        loop {
            if let Received::Connected(own) = self.recv().await {
                return own;
            }
        }
    }
    async fn voice(&mut self, predicate: impl Fn(&VoiceEvent) -> bool) -> VoiceEvent {
        loop {
            if let Received::Signal(event) = self.recv().await {
                if predicate(&event) {
                    return event;
                }
            }
        }
    }
    async fn recv(&mut self) -> Received {
        tokio::time::timeout(Duration::from_secs(10), self.received.recv())
            .await
            .unwrap()
            .expect("endpoint stopped")
    }
    async fn stop(self) {
        let _ = self.input.send(Input::Stop).await;
        self.task.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_native_endpoints_exchange_file_opus_through_real_server() {
    file_audio_and_control(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "real SendInput to an isolated secondary-monitor target; explicit invocation only"]
async fn real_control_input_reaches_secondary_window_and_releases_on_revoke() {
    file_audio_and_control(true).await;
}
async fn file_audio_and_control(real_input: bool) {
    let target = real_input.then(input_window::Target::start);
    let server = Server::start().await;
    let a = server.account("native_a").await;
    let b = server.account("native_b").await;
    let (aw, bw, channel) = room(&server, &a, &b).await;
    let mut sender = Endpoint::start(aw, a.user.user.id, channel);
    let mut receiver = Endpoint::start(bw, b.user.user.id, channel);
    let own_sender = sender.connected().await;
    let own_receiver = receiver.connected().await;
    // Let recvonly offer/answer/subscription finish, consuming all signal events.
    tokio::time::sleep(Duration::from_millis(500)).await;
    receiver
        .input
        .send(Input::Signal(VoiceSignal::State {
            muted: true,
            deafened: false,
        }))
        .await
        .unwrap();
    receiver.voice(|e| matches!(e, VoiceEvent::GuildState(s) if s.participant.session == own_receiver && s.participant.muted)).await;
    let dir = tempfile::tempdir().unwrap();
    let fixture = dir.path().join("voice.f32");
    let pcm: Vec<f32> = (0..960)
        // Exact cycles make this looping file continuous across packet edges.
        .map(|i| ((i as f32 * std::f32::consts::TAU * 500. / 48000.).sin()) * 0.2)
        .collect();
    std::fs::write(
        &fixture,
        pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>(),
    )
    .unwrap();
    let pcm: Vec<f32> = std::fs::read(fixture)
        .unwrap()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let mut encoder = OpusEncoder::new(false, 64000).unwrap();
    let mut decoder = StereoDecoder::new().unwrap();
    let mut count = 0;
    let mut best = 0f64;
    let mut speaking = false;
    for seq in 0..75 {
        let packet = encoder.encode(&pcm, seq * 960).unwrap();
        sender
            .input
            .send(Input::Packet(
                TrackKind::Audio,
                packet.timestamp as u32,
                packet.data.into(),
                true,
            ))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        while let Ok(event) = receiver.received.try_recv() {
            match event {
                Received::Packet(TrackKind::Audio, _, bytes, _, _) => {
                    let decoded = decoder.decode(&bytes, false).unwrap();
                    count += 1;
                    // Opus algorithmic delay: maximize normalized correlation over phase.
                    for lag in 0..120 {
                        let mut xy = 0.;
                        let mut xx = 0.;
                        let mut yy = 0.;
                        for i in 120..900 {
                            let x = f64::from(pcm[i - lag]);
                            let y = f64::from(decoded[i * 2]);
                            xy += x * y;
                            xx += x * x;
                            yy += y * y;
                        }
                        best = best.max(xy / (xx * yy).sqrt().max(1e-12));
                    }
                }
                Received::Signal(VoiceEvent::Speaking { speaking: true, .. }) => speaking = true,
                _ => {}
            }
        }
        while sender.received.try_recv().is_ok() {}
    }
    assert!(count >= 65, "only {count} Opus frames");
    assert!(best > 0.9, "correlation {best}");
    assert!(speaking);
    // Real SCTP/server grant/revoke; OS injection remains a dry-run sink.
    sender
        .input
        .send(Input::Publish(TrackKind::Screen))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    receiver
        .input
        .send(Input::Signal(VoiceSignal::RequestControl {
            sharer: own_sender,
        }))
        .await
        .unwrap();
    let requested = sender
        .voice(|e| matches!(e, VoiceEvent::ControlRequested { .. }))
        .await;
    assert!(
        matches!(requested,VoiceEvent::ControlRequested {requester,..} if requester==own_receiver)
    );
    sender
        .input
        .send(Input::Signal(VoiceSignal::DecideControl {
            requester: own_receiver,
            allow: true,
        }))
        .await
        .unwrap();
    let VoiceEvent::ControlGranted { grant, .. } = sender
        .voice(|e| matches!(e, VoiceEvent::ControlGranted { .. }))
        .await
    else {
        unreachable!()
    };
    let mut consent = sticktalk_client_core::voice::control::Consent::default();
    consent.request(own_receiver, 0, 30_000);
    consent.decide(1, true);
    consent.granted(own_sender, own_receiver, grant);
    let mut injector = sticktalk::voice::control::Receiver::new(
        TestInjector {
            native: target
                .as_ref()
                .map(|target| native_injection::NativeInjector {
                    source: target.source.clone(),
                }),
            ..Default::default()
        },
        sticktalk::voice::control::Surface {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        },
    );
    receiver
        .input
        .send(Input::Control(ControlData::Input {
            sharer: own_sender,
            grant,
            sequence: 1,
            input: if real_input {
                ControlInput::Move { x: 32768, y: 32768 }
            } else {
                ControlInput::Key {
                    usage: 4,
                    down: true,
                }
            },
        }))
        .await
        .unwrap();
    loop {
        if let Received::Control(bytes) = sender.recv().await {
            let data: ControlData = rmp_serde::from_slice(&bytes).unwrap();
            injector.receive(&consent, own_sender, data, 2).unwrap();
            break;
        }
    }
    assert_eq!(
        injector.injector.events.last(),
        Some(&if real_input {
            ControlInput::Move { x: 32768, y: 32768 }
        } else {
            ControlInput::Key {
                usage: 4,
                down: true,
            }
        })
    );
    // Mouse and keyboard traverse the reliable data channel and SFU grant,
    // then the same receiver and SendInput code as production. Never target
    // a personal application; the dedicated fixture is always secondary.
    for (sequence, input) in [
        (2, ControlInput::Move { x: 32768, y: 32768 }),
        (
            3,
            ControlInput::Button {
                button: 0,
                down: true,
            },
        ),
        (
            4,
            ControlInput::Key {
                usage: 4,
                down: true,
            },
        ),
    ] {
        receiver
            .input
            .send(Input::Control(ControlData::Input {
                sharer: own_sender,
                grant,
                sequence,
                input,
            }))
            .await
            .unwrap();
        loop {
            if let Received::Control(bytes) = sender.recv().await {
                injector
                    .receive(
                        &consent,
                        own_sender,
                        rmp_serde::from_slice(&bytes).unwrap(),
                        3,
                    )
                    .unwrap();
                break;
            }
        }
    }
    receiver
        .input
        .send(Input::Signal(VoiceSignal::RevokeControl))
        .await
        .unwrap();
    sender
        .voice(|e| matches!(e, VoiceEvent::ControlRevoked { .. }))
        .await;
    consent.clear();
    injector.release().unwrap();
    assert!(injector.injector.events.contains(&ControlInput::Key {
        usage: 4,
        down: false
    }));
    assert!(injector.injector.events.contains(&ControlInput::Button {
        button: 0,
        down: false
    }));
    if let Some(target) = &target {
        tokio::time::sleep(Duration::from_millis(100)).await;
        target.assert_received();
        println!("secondary target received real mouse/key input and revoke released both");
    }
    // Every new lifetime requires a NEW consent prompt, including after Deny.
    for allow in [false, true] {
        receiver
            .input
            .send(Input::Signal(VoiceSignal::RequestControl {
                sharer: own_sender,
            }))
            .await
            .unwrap();
        sender.voice(|e| matches!(e, VoiceEvent::ControlRequested { requester, .. } if *requester == own_receiver)).await;
        sender
            .input
            .send(Input::Signal(VoiceSignal::DecideControl {
                requester: own_receiver,
                allow,
            }))
            .await
            .unwrap();
        if allow {
            let VoiceEvent::ControlGranted { grant: fresh, .. } = sender
                .voice(|e| matches!(e, VoiceEvent::ControlGranted { .. }))
                .await
            else {
                unreachable!()
            };
            assert_ne!(fresh, grant);
            sender
                .input
                .send(Input::Unpublish(TrackKind::Screen))
                .await
                .unwrap();
            sender
                .voice(|e| matches!(e, VoiceEvent::ControlRevoked { grant, .. } if *grant == fresh))
                .await;
        } else {
            receiver
                .voice(|e| matches!(e, VoiceEvent::ControlDenied { .. }))
                .await;
        }
    }
    sender
        .input
        .send(Input::Signal(VoiceSignal::State {
            muted: true,
            deafened: true,
        }))
        .await
        .unwrap();
    println!(
        "file audio: {count}/75 frames, correlation={best:.4}, speaking and guild state received"
    );
    sender.stop().await;
    receiver.stop().await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "60-second generated FILE video + hardware encoder/decoder; no capture devices"]
async fn file_1080p60_decodes_at_least_58_fps_for_sixty_seconds() {
    use sticktalk_media::{
        CameraFrameEncoder, CameraOptions, Decoder, EncodedFrame, FrameRate, Size,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("screen.h264-frames");
    let fixture_path = path.clone();
    tokio::task::spawn_blocking(move || {
        let size = Size {
            width: 1920,
            height: 1080,
        };
        let mut encoder = CameraFrameEncoder::new(CameraOptions {
            size,
            fps: FrameRate::Fps60,
            bitrate: 8_000_000,
        })
        .unwrap();
        let mut nv12 = vec![128; 1920 * 1080 * 3 / 2];
        let mut file = std::fs::File::create(fixture_path).unwrap();
        use std::io::Write;
        for i in 0..60 {
            // Generated moving luminance block; no desktop/camera/privacy access.
            nv12[..1920 * 1080].fill(64);
            for y in 200..400 {
                let x = 100 + i * 12;
                nv12[y * 1920 + x..y * 1920 + x + 200].fill(192);
            }
            if i == 0 {
                encoder.request_keyframe().unwrap();
            }
            let f = encoder
                .encode_nv12(&nv12, 1920, i as i64 * 166_667)
                .unwrap();
            file.write_all(&(f.data.len() as u32).to_le_bytes())
                .unwrap();
            file.write_all(&f.data).unwrap();
        }
    })
    .await
    .unwrap();
    let bytes = std::fs::read(path).unwrap();
    let mut file = bytes.as_slice();
    let mut frames = vec![];
    while file.len() >= 4 {
        let n = u32::from_le_bytes(file[..4].try_into().unwrap()) as usize;
        file = &file[4..];
        frames.push(file[..n].to_vec());
        file = &file[n..];
    }
    assert_eq!(frames.len(), 60);
    let server = Server::start().await;
    let a = server.account("video_a").await;
    let b = server.account("video_b").await;
    let (aw, bw, channel) = room(&server, &a, &b).await;
    let mut sender = Endpoint::start(aw, a.user.user.id, channel);
    let mut receiver = Endpoint::start(bw, b.user.user.id, channel);
    sender.connected().await;
    receiver.connected().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    sender
        .input
        .send(Input::Publish(TrackKind::Screen))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (tx, rx) = std::sync::mpsc::sync_channel::<EncodedFrame>(4);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let warmup = frames.iter().take(3).cloned().collect::<Vec<_>>();
    let decode = std::thread::spawn(move || {
        let mut decoder = Decoder::new().unwrap();
        // Exclude MFT/device cold start, not sustained decoder work. This is
        // file-only: no camera/capture endpoint is activated during warmup.
        for (i, data) in warmup.into_iter().enumerate() {
            decoder
                .decode(EncodedFrame {
                    data,
                    timestamp: i as i64 * 166_667,
                    duration: 166_667,
                    keyframe: i == 0,
                    size: Size {
                        width: 1920,
                        height: 1080,
                    },
                    generation: 0,
                    encode_latency: Duration::ZERO,
                })
                .unwrap();
        }
        ready_tx.send(()).unwrap();
        let mut decoded = 0;
        while let Ok(frame) = rx.recv() {
            decoded += decoder.decode(frame).unwrap().len();
        }
        decoded
    });
    ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut assembler = rtp::Reassembler::default();
    let mut seq = 0u64;
    let mut access_units = 0;
    let mut queue_drops = 0;
    let mut interval = tokio::time::interval(Duration::from_nanos(1_000_000_000 / 60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let start = Instant::now();
    let stop = start + Duration::from_secs(60);
    let before = sticktalk_media::process_usage().unwrap();
    loop {
        tokio::select! {
            _=tokio::time::sleep_until(stop.into())=>break,
            _=interval.tick()=>{
                let packets=rtp::packetize(&frames[seq as usize%60],1180).unwrap();let n=packets.len();
                for (i,p) in packets.into_iter().enumerate() {sender.input.send(Input::Packet(TrackKind::Screen,(90_000+seq*1500) as u32,p.into(),i+1==n)).await.unwrap();}
                seq+=1;
            }
            e=receiver.received.recv()=>if let Some(Received::Packet(TrackKind::Screen,timestamp,bytes,marker,sequence))=e {
                if let Some((data,keyframe))=assembler.push(sequence,timestamp,marker,&bytes) {
                    if keyframe {assert_eq!(rtp::dimensions(&data),Some(Size {width:1920,height:1080}));}
                    let f=EncodedFrame {data,timestamp:i64::from(timestamp)*1000/9,duration:166_667,keyframe,size:Size {width:1920,height:1080},generation:0,encode_latency:Duration::ZERO};
                    match tx.try_send(f) {
                        Ok(()) => {},
                        Err(std::sync::mpsc::TrySendError::Full(_)) => queue_drops+=1,
                        Err(e) => panic!("decoder stopped: {e}"),
                    }
                    access_units+=1;
                }
            },
            _=sender.received.recv()=>{},
        }
    }
    drop(tx);
    let decoded = decode.join().unwrap();
    let after = sticktalk_media::process_usage().unwrap();
    let fps = decoded as f64 / 60.;
    println!("1080p60 FILE/SRTP/hardware-decode: sent={seq}, access_units={access_units}, decoded={decoded}, queue_drops={queue_drops}, fps={fps:.3}; process before={before:?}, after={after:?}");
    sender.stop().await;
    receiver.stop().await;
    server.stop().await;
    assert!(fps >= 58., "hardware decoded only {fps} fps");
}

struct BenchChild(std::process::Child);
impl std::ops::Deref for BenchChild {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for BenchChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for BenchChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Live monitor capture, real hardware encoder/SFU/two decoder *processes*.
/// No windows, preview, camera, microphone, sounds, notifications or input injection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "10-minute invisible live WGC acceptance; explicit invocation only"]
async fn live_calls2_two_viewers_ten_minutes() {
    if let Ok(role) = std::env::var("STICKTALK_CALLS2_ROLE") {
        live_calls2_worker(&role).await;
        return;
    }
    let server = Server::start().await;
    let a = server.account("live_sharer").await;
    let b = server.account("live_viewer_b").await;
    let c = server.account("live_viewer_c").await;
    let (mut aw, bw, channel, guild_id) = room_details(&server, &a, &b).await;
    let mut cw = server.connect(&c).await;
    let CommandResult::Invite(invite) = command(
        &mut aw,
        Command::CreateInvite {
            guild_id,
            ttl_seconds: 3600,
            max_uses: 10,
        },
    )
    .await
    else {
        panic!()
    };
    command(
        &mut cw,
        Command::JoinGuild {
            invite_code: invite.code,
        },
    )
    .await;
    drop((aw, bw, cw));
    let dir = tempfile::tempdir().unwrap();
    let mut workers = vec![];
    use std::os::windows::process::CommandExt;
    for (role, auth) in [("sharer", a), ("viewer_b", b), ("viewer_c", c)] {
        let log = std::fs::File::create(dir.path().join(format!("{role}.log"))).unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "live_calls2_two_viewers_ten_minutes",
                "--ignored",
                "--nocapture",
            ])
            .env("STICKTALK_CALLS2_ROLE", role)
            .env("STICKTALK_CALLS2_URL", &server.url)
            .env(
                "STICKTALK_CALLS2_AUTH",
                serde_json::to_string(&auth).unwrap(),
            )
            .env("STICKTALK_CALLS2_CHANNEL", channel.0.to_string())
            .env("STICKTALK_CALLS2_DIR", dir.path())
            .creation_flags(0x08000000)
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        workers.push((role, BenchChild(child)));
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    while !["sharer", "viewer_b", "viewer_c"]
        .iter()
        .all(|r| dir.path().join(format!("{r}.ready")).exists())
    {
        for (role, child) in &mut workers {
            assert!(
                child.try_wait().unwrap().is_none(),
                "{role} stopped: {}",
                std::fs::read_to_string(dir.path().join(format!("{role}.log"))).unwrap()
            );
        }
        assert!(Instant::now() < deadline, "workers not ready");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // GPU counters live ONLY in the coordinator. The provider has substantial
    // allocations; charging those to a client would invalidate its RAM budget.
    let mut gpu = gpu_usage::GpuCounters::new();
    let pids: Vec<_> = workers.iter().map(|(_, c)| c.id()).collect();
    let mut gpu_samples = std::collections::BTreeMap::<(u32, String), (f64, u64)>::new();
    // Warm capture, negotiation, keyframes, hardware decoder and process caches.
    tokio::time::sleep(Duration::from_secs(8)).await;
    std::fs::write(dir.path().join("measure"), []).unwrap();
    let seconds = bench_seconds();
    let end = Instant::now() + Duration::from_secs(seconds + 30);
    loop {
        let mut done = true;
        for (_, child) in &mut workers {
            if child.try_wait().unwrap().is_none() {
                done = false;
            }
        }
        if done {
            break;
        }
        if let Some(values) = gpu.as_mut().and_then(|g| g.sample(&pids)) {
            for (key, value) in values {
                let entry = gpu_samples.entry(key).or_default();
                entry.0 += value;
                entry.1 += 1;
            }
        }
        assert!(Instant::now() < end, "acceptance process timed out");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let output = std::env::var_os("STICKTALK_CALLS2_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("bench/share-fix-2026-11-13/default")
        });
    std::fs::create_dir_all(&output).unwrap();
    let mut gpu_report=String::from("Per-process GPU engine means; GPU provider runs outside the measured clients. Values sum matching engines, not whole-adapter utilization.\n");
    for (role, child) in &workers {
        for ((pid, kind), (sum, n)) in &gpu_samples {
            if *pid == child.id() {
                gpu_report.push_str(&format!(
                    "{role} {kind}: {:.4}% ({} samples)\n",
                    sum / *n as f64,
                    n
                ));
            }
        }
    }
    println!("{gpu_report}");
    std::fs::write(output.join("gpu-engines.log"), gpu_report).unwrap();
    let mut failed = Vec::new();
    for (role, mut child) in workers {
        let log = std::fs::read_to_string(dir.path().join(format!("{role}.log"))).unwrap();
        println!("{log}");
        std::fs::write(output.join(format!("{role}.log")), log).unwrap();
        let metrics = dir.path().join(format!("{role}.metrics.json"));
        if metrics.exists() {
            std::fs::copy(metrics, output.join(format!("{role}.metrics.json"))).unwrap();
        }
        if !child.wait().unwrap().success() {
            failed.push(role);
        }
    }
    server.stop().await;
    assert!(failed.is_empty(), "acceptance failed for: {failed:?}");
}

fn bench_quality() -> sticktalk_media::Quality {
    use sticktalk_media::{FrameRate, Quality, Resolution};
    Quality {
        resolution: match std::env::var("STICKTALK_CALLS2_QUALITY")
            .as_deref()
            .unwrap_or("1080")
        {
            "480" => Resolution::P480,
            "720" => Resolution::P720,
            "1080" => Resolution::P1080,
            "1440" => Resolution::P1440,
            "source" => Resolution::Source,
            other => panic!("unknown quality {other}"),
        },
        fps: match std::env::var("STICKTALK_CALLS2_FPS")
            .as_deref()
            .unwrap_or("60")
        {
            "15" => FrameRate::Fps15,
            "30" => FrameRate::Fps30,
            "60" => FrameRate::Fps60,
            other => panic!("unknown frame rate {other}"),
        },
    }
}
fn bench_seconds() -> u64 {
    let seconds = std::env::var("STICKTALK_CALLS2_SECONDS")
        .map_or(600, |s| s.parse().expect("duration seconds"));
    assert!((10..=600).contains(&seconds));
    seconds
}
async fn live_calls2_worker(role: &str) {
    use sticktalk_media::{process_usage, Decoder, EncodedFrame, ShareSession};
    let url = std::env::var("STICKTALK_CALLS2_URL").unwrap();
    assert!(url.starts_with("http://127.0.0.1:"));
    let auth: AuthResponse =
        serde_json::from_str(&std::env::var("STICKTALK_CALLS2_AUTH").unwrap()).unwrap();
    let channel = Id(std::env::var("STICKTALK_CALLS2_CHANNEL")
        .unwrap()
        .parse()
        .unwrap());
    let dir = std::path::PathBuf::from(std::env::var("STICKTALK_CALLS2_DIR").unwrap());
    let mut request = format!("{}/api/v1/ws", url.replace("http:", "ws:"))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
    let (mut ws, _) = connect_async(request).await.unwrap();
    assert!(matches!(read(&mut ws).await, ServerMsg::Hello(_)));
    send(
        &mut ws,
        ClientMsg::Identify(Identify {
            token: auth.token,
            ..Default::default()
        }),
    )
    .await;
    assert!(matches!(read(&mut ws).await, ServerMsg::Ready(_)));
    let mut endpoint = Endpoint::start(ws, auth.user.user.id, channel);
    endpoint.connected().await;
    let baseline = process_usage().unwrap();
    let quality = bench_quality();
    let share = if role == "sharer" {
        let source = sticktalk_media::list_sources()
            .unwrap()
            .into_iter()
            .find(|s| {
                if std::env::var("STICKTALK_CALLS2_SOURCE").is_ok_and(|v| v == "wgc-motion") {
                    s.kind == sticktalk_media::SourceKind::Window
                        && s.name == "StickTalk GPU benchmark pattern (closes automatically)"
                } else {
                    s.kind == sticktalk_media::SourceKind::Monitor && s.primary
                }
            })
            .unwrap();
        let bitrate = quality.recommended_bitrate(source.size).unwrap();
        let session = if std::env::var("STICKTALK_CALLS2_SOURCE").is_ok_and(|s| s == "synthetic") {
            ShareSession::start_synthetic(
                source.size,
                sticktalk_media::ShareOptions {
                    quality,
                    bitrate,
                    ..Default::default()
                },
            )
            .unwrap()
        } else {
            ShareSession::start(source, quality, bitrate).unwrap()
        };
        println!(
            "capture source={} size={:?} quality={quality:?} hardware encoder={:?}, GPU={}",
            std::env::var("STICKTALK_CALLS2_SOURCE").unwrap_or_else(|_| "WGC-primary".into()),
            session.size,
            session.encoder_kind,
            session.gpu_name
        );
        endpoint
            .input
            .send(Input::Target(TrackKind::Screen, u64::from(bitrate)))
            .await
            .unwrap();
        endpoint
            .input
            .send(Input::Publish(TrackKind::Screen))
            .await
            .unwrap();
        Some(session)
    } else {
        None
    };
    let mut decoder = if share.is_none() {
        Some(Decoder::new().unwrap())
    } else {
        None
    };
    let mut assembler = rtp::Reassembler::default();
    let mut coded_size = sticktalk_media::Size {
        width: 1920,
        height: 1080,
    };
    std::fs::write(dir.join(format!("{role}.ready")), []).unwrap();
    let mut measure: Option<(Instant, sticktalk_media::ProcessUsage)> = None;
    let mut count = 0u64;
    let seconds = bench_seconds();
    let mut buckets = vec![0u32; seconds as usize];
    let mut raw_packets = 0u64;
    let mut raw_bytes = 0u64;
    let mut access_units = 0u64;
    let mut decode_errors = 0u64;
    let mut decoded_keyframes = 0u64;
    let mut traced_at = Instant::now();
    let mut bytes = 0u64;
    let mut timing = vec![];
    let mut first: Option<(Instant, u32)> = None;
    let mut last_usage = baseline;
    println!(
        "stage after media startup: working_set_mib={:.2} private_mib={:.2}",
        process_usage().unwrap().rss_bytes as f64 / 1048576.,
        process_usage().unwrap().private_bytes as f64 / 1048576.
    );
    // Sharer CPU/RAM do not need in-process GPU provider instrumentation.
    let mut tick = tokio::time::interval(Duration::from_millis(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let limit = Instant::now() + Duration::from_secs(seconds + 120);
    loop {
        assert!(Instant::now() < limit, "worker deadline");
        if measure.is_none() && dir.join("measure").exists() {
            measure = Some((Instant::now(), process_usage().unwrap()));
            bytes = 0;
        }
        if measure
            .as_ref()
            .is_some_and(|(t, _)| t.elapsed() >= Duration::from_secs(seconds))
        {
            break;
        }
        tokio::select! {
            _=tick.tick()=>{
                if let Some(s)=&share {
                    while let Ok(frame)=s.recv_timeout(Duration::ZERO) {
                        let packets=rtp::packetize(&frame.data,1180).unwrap(); let n=packets.len();
                        for (i,p) in packets.into_iter().enumerate() {
                            endpoint.input.send(Input::Packet(TrackKind::Screen,(frame.timestamp as u64*9/1000) as u32,p.into(),i+1==n)).await.unwrap();
                        }
                        if let Some((t,_))=&measure {count+=1;buckets[(t.elapsed().as_secs() as usize).min(seconds as usize - 1)]+=1;bytes+=frame.data.len() as u64;}
                    }
                }
            },
            event=endpoint.received.recv()=>match event {
                Some(Received::Signal(VoiceEvent::Bitrate {mid:_,bps})) if share.is_some() => {
                    if let Some(s) = &share {
                        s.set_bitrate(bps.clamp(100_000,100_000_000) as u32).unwrap();
                        // The desired transport quality remains independent
                        // of a temporary congestion ceiling.
                    }
                },
                Some(Received::Keyframe(TrackKind::Screen)) => { if let Some(s)=&share { s.request_keyframe(); } },
                Some(Received::Signal(VoiceEvent::Stats(stats))) => { if std::env::var_os("STICKTALK_VIDEO_TRACE").is_some() { println!("VIDEO_TRACE server_stats role={role} {stats:?}"); } },
                Some(Received::Packet(TrackKind::Screen,ts,data,marker,seq)) => {
                raw_packets += 1; raw_bytes += data.len() as u64;
                if let Some((data,keyframe))=assembler.push(seq,ts,marker,&data) {
                    access_units += 1; decoded_keyframes += u64::from(keyframe);
                    if let Some(size) = rtp::dimensions(&data) { coded_size = size; }
                    let frame=EncodedFrame {data,timestamp:i64::from(ts)*1000/9,duration:166_667,keyframe,size:coded_size,generation:0,encode_latency:Duration::ZERO};
                    bytes+=frame.data.len() as u64;
                    if let Some(decoder)=&mut decoder {
                        match decoder.decode(frame) { Ok(frames) => {
                            if let Some((t,_))=&measure {
                                count+=frames.len() as u64; buckets[(t.elapsed().as_secs() as usize).min(seconds as usize - 1)]+=frames.len() as u32;
                                if !frames.is_empty() {
                                    let (at,origin)=*first.get_or_insert((Instant::now(),ts));
                                    timing.push((t.elapsed().as_secs_f64(),at.elapsed().as_secs_f64()-f64::from(ts.wrapping_sub(origin))/90_000.));
                                }
                            }
                        }, Err(error) => { decode_errors += 1; if decode_errors <= 3 { println!("decode error: {error}"); } } }
                    }
                }
                },
                _ => {},
            },
        }
        if traced_at.elapsed() >= Duration::from_secs(1) {
            traced_at = Instant::now();
            last_usage = process_usage().unwrap();
            if let Some(share) = &share {
                println!(
                    "VIDEO_TRACE encoder_out seconds={:.3} stats={:?}",
                    measure
                        .as_ref()
                        .map_or(0., |(t, _)| t.elapsed().as_secs_f64()),
                    share.stats()
                );
            }
        }
    }
    let after = process_usage().unwrap();
    let (start, before) = measure.unwrap();
    let elapsed = start.elapsed().as_secs_f64();
    let cpu = (after.cpu_seconds - before.cpu_seconds) / elapsed / after.logical_cpus as f64 * 100.;
    let mean = |low: f64, high: f64| {
        let s: Vec<_> = timing
            .iter()
            .filter(|(t, _)| *t >= low && *t < high)
            .map(|(_, d)| *d)
            .collect();
        s.iter().sum::<f64>() / s.len().max(1) as f64
    };
    let window = seconds as f64 / 10.;
    let drift = (mean(seconds as f64 - window, seconds as f64) - mean(0., window)) * 1000.;
    let mut offsets = vec![(0.0, 0u32); seconds as usize];
    for (time, offset) in &timing {
        let bucket = &mut offsets[(*time as usize).min(seconds as usize - 1)];
        bucket.0 += offset * 1000.;
        bucket.1 += 1;
    }
    let offsets: Vec<Option<f64>> = offsets
        .into_iter()
        .map(|(sum, n)| (n != 0).then(|| sum / f64::from(n)))
        .collect();
    std::fs::write(dir.join(format!("{role}.metrics.json")), serde_json::to_vec_pretty(&serde_json::json!({
        "role": role, "seconds": elapsed, "frames": count, "fps_by_second": buckets,
        "latency_offset_ms_by_second": offsets, "drift_ms": drift, "cpu_machine_pct": cpu,
        "rss_bytes": after.rss_bytes, "rss_delta_bytes": after.rss_bytes.saturating_sub(baseline.rss_bytes),
        "private_bytes": after.private_bytes, "private_delta_bytes": after.private_bytes.saturating_sub(baseline.private_bytes),
        "payload_bytes": bytes, "raw_packets_with_warmup": raw_packets, "raw_payload_bytes_with_warmup": raw_bytes,
        "assembled_frames_with_warmup": access_units, "decode_errors": decode_errors,
        "encoder_reconfigurations": share.as_ref().map(|s| s.stats().reconfigurations),
        "source": std::env::var("STICKTALK_CALLS2_SOURCE").unwrap_or_else(|_| "wgc".into()),
        "quality": format!("{quality:?}"), "decoder_size": format!("{coded_size:?}")
    })).unwrap()).unwrap();
    println!("VIDEO_TRACE totals raw_packets={raw_packets} raw_payload_bytes={raw_bytes} assembled_access_units={access_units} received_keyframes={decoded_keyframes} decode_errors={decode_errors}");
    println!("CALLS2 role={role} seconds={elapsed:.3} frames={count} avg_fps={:.3} min_1s_fps={} latency_drift_ms={drift:.3} cpu_machine_pct={cpu:.3} working_set_mib={:.2} working_set_delta_mib={:.2} peak_working_set_mib={:.2} bitrate_mbps={:.3} last_working_set_mib={:.2}",
        count as f64/elapsed,buckets.iter().min().unwrap(),after.rss_bytes as f64/1048576.,(after.rss_bytes as f64-baseline.rss_bytes as f64)/1048576.,after.peak_rss_bytes as f64/1048576.,bytes as f64*8./elapsed/1_000_000.,last_usage.rss_bytes as f64/1048576.);
    println!(
        "private_bytes_mib={:.2} private_delta_mib={:.2}",
        after.private_bytes as f64 / 1048576.,
        (after.private_bytes as f64 - baseline.private_bytes as f64) / 1048576.
    );
    if let Some(share) = &share {
        println!("capture stats: {:?}", share.stats());
        assert_eq!(
            share.stats().reconfigurations,
            0,
            "encoder format reconfiguration thrash"
        );
    }
    drop(share);
    drop(decoder);
    endpoint.stop().await;
    let target_fps = f64::from(quality.fps.get());
    assert!(
        count as f64 / elapsed >= target_fps * 58. / 60.,
        "below target average fps"
    );
    assert!(drift.abs() < 20., "latency drift exceeds 20ms");
    assert!(
        f64::from(*buckets.iter().min().unwrap()) >= target_fps * 45. / 60.,
        "one-second window below 75% of target fps"
    );
    if role == "sharer" {
        assert!(cpu < 5.);
        assert!(after.rss_bytes.saturating_sub(baseline.rss_bytes) < 150_000_000);
    }
}
