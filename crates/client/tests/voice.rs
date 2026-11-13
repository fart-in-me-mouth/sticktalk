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
async fn room(server: &Server, a: &AuthResponse, b: &AuthResponse) -> (Socket, Socket, Id) {
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
    (aw, bw, c.summary.id)
}
enum Input {
    Control(ControlData),
    Signal(VoiceSignal),
    Packet(TrackKind, u32, Arc<[u8]>, bool),
    Publish(TrackKind),
    Stop,
}
enum Received {
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
            let local = socket.local_addr().unwrap();
            let mut call = Call::default();
            let nonce = sticktalk::backend::nonce();
            let mut effects = call.join(channel, nonce);
            let mut peer: Option<Peer> = None;
            let mut buffer = [0; 65536];
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
                            PeerOutput::Control(data) => {
                                output.send(Received::Control(data)).await.unwrap()
                            }
                            _ => {}
                        }
                    }
                    p.reconcile().unwrap();
                    // Drain offers generated by reconcile before sleeping on deadlines.
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
                            _ => {}
                        }
                    }
                }
                let deadline = peer
                    .as_ref()
                    .map_or(Instant::now() + Duration::from_secs(30), Peer::deadline);
                tokio::select! {
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
                        Some(Input::Control(data))=>peer.as_mut().unwrap().control(&rmp_serde::to_vec_named(&data).unwrap()).unwrap(),
                        Some(Input::Signal(s))=>send(&mut ws,ClientMsg::Voice(sticktalk_client_core::voice::encode(nonce,s).unwrap())).await,
                        Some(Input::Packet(kind,time,data,marker))=>if let Some(p)=&mut peer {p.send(kind,time,&data,marker,-24,Instant::now()).unwrap();},
                        Some(Input::Publish(kind))=>peer.as_mut().unwrap().publish(kind,true,ShareSurface::Monitor).unwrap(),
                        _=>break,
                    },
                    _=tokio::time::sleep_until(deadline.into())=>if let Some(p)=&mut peer {p.timeout(Instant::now()).unwrap();},
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
        sticktalk::voice::control::DryRun::default(),
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
            input: ControlInput::Key {
                usage: 4,
                down: true,
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
        Some(&ControlInput::Key {
            usage: 4,
            down: true
        })
    );
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
    assert_eq!(
        injector.injector.events.last(),
        Some(&ControlInput::Key {
            usage: 4,
            down: false
        })
    );
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
