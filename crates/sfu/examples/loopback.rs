//! Headless real-UDP benchmark. A separate child process runs ONLY the SFU;
//! sender and four receiver str0m peers run in this parent. No capture or UI.
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sticktalk_sfu::{encode_signal, proto::*, rtc_config, Command, Config, Sfu};
use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::media::{Direction, MediaKind, Mid};
use str0m::net::{Protocol, Receive};
use str0m::rtp::{RtpWrite, Ssrc};
use str0m::{Candidate, Event, Input, Output, Rtc};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

type AnyError = Box<dyn std::error::Error + Send + Sync>;
#[derive(Serialize, Deserialize)]
enum Request {
    Signal(VoiceSession, VoiceSignal),
    Stop,
}
#[derive(Serialize, Deserialize)]
enum Reply {
    Ready(std::net::SocketAddr),
    Signal(VoiceSession, VoiceEvent),
}
fn key(index: usize) -> VoiceSession {
    VoiceSession {
        user_id: Id(index as u64 + 1),
        connection_id: Id(index as u64 + 101),
    }
}
fn capabilities() -> VoiceCapabilities {
    VoiceCapabilities {
        video: vec![VideoCodec::H264],
        hardware_decode: vec![VideoCodec::H264],
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), AnyError> {
    if std::env::args().any(|a| a == "--sfu-child") {
        return server().await;
    }
    let seconds = std::env::args()
        .skip_while(|a| a != "--seconds")
        .nth(1)
        .and_then(|a| a.parse::<u64>().ok())
        .unwrap_or(30)
        .clamp(2, 600);
    benchmark(seconds).await
}
async fn server() -> Result<(), AnyError> {
    let config = Config {
        bind_ip: "127.0.0.1".parse()?,
        advertised_ip: "127.0.0.1".parse()?,
        ports: 53500..=53599,
        video_egress_bps: 64_000_000,
        ..Config::default()
    };
    let sfu = Sfu::new(config, Arc::new(|_: VoiceSession, c: Id| c == Id(1)))?;
    println!(
        "{}",
        serde_json::to_string(&Reply::Ready(sfu.local_addr()))?
    );
    std::io::stdout().flush()?;
    let (tx, rx) = mpsc::channel(128);
    let (signals, mut received) = mpsc::channel::<sticktalk_sfu::SignalOut>(128);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let command = match serde_json::from_str::<Request>(&line) {
                Ok(Request::Signal(session, s)) => Command::Signal {
                    session,
                    envelope: encode_signal(1, &s).unwrap(),
                },
                _ => Command::Shutdown,
            };
            if tx.blocking_send(command).is_err() {
                break;
            }
        }
    });
    let writer = tokio::spawn(async move {
        while let Some(out) = received.recv().await {
            let event: VoiceEvent = rmp_serde::from_slice(&out.envelope.payload).unwrap();
            println!(
                "{}",
                serde_json::to_string(&Reply::Signal(out.session, event)).unwrap()
            );
            std::io::stdout().flush().unwrap();
        }
    });
    sfu.run(rx, signals).await?;
    writer.await?;
    Ok(())
}
struct Server {
    child: Child,
    input: std::process::ChildStdin,
    replies: std::sync::mpsc::Receiver<Reply>,
}
impl Server {
    fn start() -> Result<(Self, std::net::SocketAddr), AnyError> {
        let mut cmd = ProcessCommand::new(std::env::current_exe()?);
        cmd.arg("--sfu-child")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let mut child = cmd.spawn()?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if let Ok(reply) = serde_json::from_str::<Reply>(&line) {
                    if tx.send(reply).is_err() {
                        break;
                    }
                }
            }
        });
        let Reply::Ready(address) = rx.recv_timeout(Duration::from_secs(30))? else {
            return Err("missing SFU ready".into());
        };
        Ok((
            Self {
                child,
                input,
                replies: rx,
            },
            address,
        ))
    }
    fn send(&mut self, key: VoiceSession, signal: VoiceSignal) -> Result<(), AnyError> {
        writeln!(
            self.input,
            "{}",
            serde_json::to_string(&Request::Signal(key, signal))?
        )?;
        self.input.flush()?;
        Ok(())
    }
    fn answer(&self, key: VoiceSession) -> Result<String, AnyError> {
        loop {
            match self.replies.recv_timeout(Duration::from_secs(30))? {
                Reply::Signal(to, VoiceEvent::Answer { sdp }) if key == to => return Ok(sdp),
                Reply::Signal(to, VoiceEvent::Error { message, .. }) => {
                    return Err(format!("{to:?}: {message}").into())
                }
                _ => {}
            }
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = writeln!(
            self.input,
            "{}",
            serde_json::to_string(&Request::Stop).unwrap()
        );
        let _ = self.input.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct Peer {
    rtc: Rtc,
    socket: UdpSocket,
    mid: Mid,
    deadline: Instant,
    frames: u64,
    bytes: u64,
    packets: u64,
    max_datagram: usize,
}
impl Peer {
    async fn new(sender: bool) -> Result<(Self, String, SdpPendingOffer), AnyError> {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
        let socket_ref = socket2::SockRef::from(&socket);
        socket_ref.set_recv_buffer_size(256 * 1024)?;
        socket_ref.set_send_buffer_size(256 * 1024)?;
        socket.set_nonblocking(true)?;
        let socket = UdpSocket::from_std(socket)?;
        let now = Instant::now();
        let mut rtc = rtc_config().build(now);
        rtc.add_local_candidate(Candidate::host(socket.local_addr()?, "udp")?)
            .unwrap();
        while !matches!(rtc.poll_output()?, Output::Timeout(_)) {}
        let mut api = rtc.sdp_api();
        let mid = api.add_media(
            MediaKind::Video,
            if sender {
                Direction::SendOnly
            } else {
                Direction::RecvOnly
            },
            None,
            None,
            None,
        );
        let (offer, pending) = api.apply().unwrap();
        while !matches!(rtc.poll_output()?, Output::Timeout(_)) {}
        Ok((
            Self {
                rtc,
                socket,
                mid,
                deadline: now,
                frames: 0,
                bytes: 0,
                packets: 0,
                max_datagram: 0,
            },
            offer.to_sdp_string(),
            pending,
        ))
    }
    async fn drain(&mut self) -> Result<(), AnyError> {
        loop {
            match self.rtc.poll_output()? {
                Output::Timeout(t) => {
                    self.deadline = t;
                    break;
                }
                Output::Transmit(tx) => {
                    self.socket
                        .send_to(tx.contents.as_ref(), tx.destination)
                        .await?;
                }
                Output::Event(Event::RtpPacket(p)) => {
                    self.bytes += p.payload.len() as u64;
                    self.packets += 1;
                    if p.header.marker {
                        self.frames += 1;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    async fn tick(&mut self, now: Instant) -> Result<(), AnyError> {
        let mut buf = vec![0; 65536];
        while let Ok((len, from)) = self.socket.try_recv_from(&mut buf) {
            self.max_datagram = self.max_datagram.max(len);
            self.rtc.handle_input(Input::Receive(
                now,
                Receive::new(Protocol::Udp, from, self.socket.local_addr()?, &buf[..len])?,
            ))?;
            self.drain().await?;
        }
        if self.deadline <= now {
            self.rtc.handle_input(Input::Timeout(now))?;
            self.drain().await?;
        }
        Ok(())
    }
}
async fn benchmark(seconds: u64) -> Result<(), AnyError> {
    let (mut server, _address) = Server::start()?;
    let mut peers = vec![];
    for index in 0..5 {
        let (mut peer, offer, pending) = Peer::new(index == 0).await?;
        server.send(
            key(index),
            VoiceSignal::Join {
                channel_id: Id(1),
                capabilities: capabilities(),
            },
        )?;
        let tracks = if index == 0 {
            vec![PublishedTrack {
                mid: peer.mid.to_string(),
                kind: TrackKind::Screen,
                codec: Some(VideoCodec::H264),
                layers: vec![],
                surface: Some(ShareSurface::Monitor),
            }]
        } else {
            vec![]
        };
        server.send(key(index), VoiceSignal::Offer { sdp: offer, tracks })?;
        peer.rtc.sdp_api().accept_answer(
            pending,
            SdpAnswer::from_sdp_string(&server.answer(key(index))?)?,
        )?;
        peer.drain().await?;
        peers.push(peer);
        let end = Instant::now() + Duration::from_millis(250);
        while Instant::now() < end {
            for p in &mut peers {
                p.tick(Instant::now()).await?;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    for index in 1..5 {
        server.send(
            key(index),
            VoiceSignal::Subscribe {
                publisher: key(0),
                source_mid: peers[0].mid.to_string(),
                receiver_mid: peers[index].mid.to_string(),
                rid: None,
            },
        )?;
    }
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end {
        for p in &mut peers {
            p.tick(Instant::now()).await?;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    if peers.iter().any(|p| !p.rtc.is_connected()) {
        return Err("loopback ICE/DTLS incomplete".into());
    }
    let mid = peers[0].mid;
    let ssrc: Ssrc = peers[0]
        .rtc
        .direct_api()
        .stream_tx_by_mid(mid, None)
        .unwrap()
        .ssrc();
    peers[0].drain().await?;
    peers[0]
        .rtc
        .direct_api()
        .stream_tx(&ssrc)
        .unwrap()
        .set_unpaced(true);
    peers[0].drain().await?;
    let pt = peers[0]
        .rtc
        .codec_config()
        .params()
        .iter()
        .find(|p| p.spec().codec == str0m::format::Codec::H264)
        .unwrap()
        .pt();
    let mut before = process_metrics(&server.child);
    let epoch = Instant::now();
    let measurement_target = epoch + Duration::from_secs(3);
    let end = measurement_target + Duration::from_secs(seconds);
    let mut measured_start = None;
    let mut warmup_frames = 0;
    let mut frame = 0u64;
    let mut sequence = 1u64;
    let mut cap_drops = 0;
    let mut peak_rss = before.1;
    while Instant::now() < end {
        if measured_start.is_none() && Instant::now() >= measurement_target {
            before = process_metrics(&server.child);
            peak_rss = before.1;
            measured_start = Some(Instant::now());
            warmup_frames = frame;
            for peer in &mut peers {
                peer.bytes = 0;
                peer.frames = 0;
                peer.packets = 0;
            }
        }
        let target = epoch + Duration::from_secs_f64(frame as f64 / 60.0);
        if Instant::now() >= target {
            let mut remaining = 16_666usize;
            let mut first = true;
            while remaining > 0 {
                let len = remaining.min(1000);
                let mut payload = vec![0u8; len];
                payload[0] = 0x7c;
                payload[1] = 0x05
                    | if first {
                        0x80
                    } else if remaining == len {
                        0x40
                    } else {
                        0
                    };
                peers[0]
                    .rtc
                    .direct_api()
                    .stream_tx(&ssrc)
                    .unwrap()
                    .write_rtp(
                        RtpWrite::new(
                            pt,
                            sequence.into(),
                            (frame * 1500) as u32,
                            Instant::now(),
                            payload,
                        )
                        .marker(remaining == len)
                        .nackable(true),
                    );
                peers[0].drain().await?;
                peers[0].rtc.handle_input(Input::Timeout(Instant::now()))?;
                peers[0].drain().await?;
                sequence += 1;
                remaining -= len;
                first = false;
            }
            frame += 1;
        }
        for p in &mut peers {
            p.tick(Instant::now()).await?;
        }
        while let Ok(reply) = server.replies.try_recv() {
            if let Reply::Signal(_, VoiceEvent::Stats(s)) = reply {
                cap_drops = cap_drops.max(s.cap_dropped_packets);
            }
        }
        peak_rss = peak_rss.max(process_metrics(&server.child).1);
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let elapsed = measured_start
        .expect("measurement after warmup")
        .elapsed()
        .as_secs_f64();
    let after = process_metrics(&server.child);
    println!("SFU child PID: {}", server.child.id());
    println!("duration={elapsed:.3}s; SFU CPU={:.3}% of one logical core; SFU working set={:.2} MiB; peak observed={:.2} MiB",(after.0-before.0)/elapsed*100.0,after.1 as f64/1048576.0,peak_rss as f64/1048576.0);
    println!("synthetic H264 RTP: 1920x1080/60-sized workload, 8 Mbps payload, 4 receivers; NO capture/encode/decode; input_frames={}; warmup=3s",frame-warmup_frames);
    for (i, p) in peers.iter().enumerate().skip(1) {
        println!(
            "receiver {i}: {:.3} Mbps payload, {:.2} fps marker rate, packets={}, max UDP={} bytes",
            p.bytes as f64 * 8.0 / elapsed / 1e6,
            p.frames as f64 / elapsed,
            p.packets,
            p.max_datagram
        );
    }
    println!("max per-session cap drops={cap_drops}; configured global video cap=64 Mbps");
    Ok(())
}

#[cfg(windows)]
fn process_metrics(child: &Child) -> (f64, u64) {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Memory {
        cb: u32,
        faults: u32,
        peak: usize,
        working: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_nonpaged: usize,
        quota_nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetProcessTimes(
            p: *mut c_void,
            c: *mut FileTime,
            e: *mut FileTime,
            k: *mut FileTime,
            u: *mut FileTime,
        ) -> i32;
    }
    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(p: *mut c_void, m: *mut Memory, cb: u32) -> i32;
    }
    let (mut c, mut e, mut k, mut u) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    let mut m = Memory {
        cb: std::mem::size_of::<Memory>() as u32,
        ..Default::default()
    };
    // SAFETY: OS-owned process handle; pointers refer to initialized, correctly
    // sized repr(C) Win32 output structures and live for each synchronous call.
    unsafe {
        if GetProcessTimes(child.as_raw_handle(), &mut c, &mut e, &mut k, &mut u) == 0
            || GetProcessMemoryInfo(child.as_raw_handle(), &mut m, m.cb) == 0
        {
            return (0.0, 0);
        }
    }
    let time = |t: FileTime| ((u64::from(t.high) << 32) | u64::from(t.low)) as f64 / 10_000_000.0;
    (time(k) + time(u), m.working as u64)
}
#[cfg(not(windows))]
fn process_metrics(child: &Child) -> (f64, u64) {
    // Linux /proc process accounting; not a system-wide or parent metric.
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.id())).unwrap_or_default();
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .map(|(_, s)| s.split_whitespace().collect())
        .unwrap_or_default();
    let ticks = fields
        .get(11)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        + fields
            .get(12)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
    let status =
        std::fs::read_to_string(format!("/proc/{}/status", child.id())).unwrap_or_default();
    let rss = status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        * 1024;
    // Linux USER_HZ is 100 on the intended Pi/standard Linux targets.
    (ticks as f64 / 100.0, rss)
}
