#![allow(dead_code)]
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sticktalk_sfu::{proto::*, rtc_config, Config, Out, Sfu};
use str0m::change::SdpAnswer;
use str0m::channel::{ChannelConfig, ChannelId};
use str0m::format::Codec;
use str0m::media::{Direction, MediaKind, Mid};
use str0m::net::{Protocol, Receive};
use str0m::rtp::{ExtensionValues, RtpWrite, Ssrc};
use str0m::{Candidate, Event, Input, Output, Rtc};

pub fn session(n: u64) -> VoiceSession {
    VoiceSession {
        user_id: Id(n),
        connection_id: Id(100 + n),
    }
}
pub fn caps() -> VoiceCapabilities {
    VoiceCapabilities {
        video: vec![VideoCodec::H264],
        hardware_decode: vec![VideoCodec::H264],
    }
}
pub struct Peer {
    pub rtc: Rtc,
    pub address: SocketAddr,
    pub key: VoiceSession,
    pub audio: Mid,
    pub video: Mid,
    pub screen: Mid,
    pub channel: ChannelId,
    pub events: Vec<Event>,
    pub deadline: Instant,
    pub received: u64,
    pub frames: u64,
    pub control: Vec<ControlData>,
}
pub struct Network {
    pub sfu: Sfu,
    pub peers: Vec<Peer>,
    pub now: Instant,
    pub pending: VecDeque<(SocketAddr, SocketAddr, Vec<u8>)>,
    pub signals: Vec<(VoiceSession, VoiceEvent)>,
    pub video_bytes: u64,
    pub max_rtp: usize,
    pub drop_next_video: bool,
    pub dropped: u64,
}
impl Network {
    pub fn new(cap: u64) -> Self {
        Self::with_acl(cap, Arc::new(|_: VoiceSession, c: Id| c == Id(1)))
    }
    pub fn with_acl(cap: u64, acl: Arc<dyn sticktalk_sfu::Authorizer>) -> Self {
        Self::with_mtu(cap, acl, DEFAULT_VOICE_UDP_BYTES)
    }
    pub fn with_mtu(cap: u64, acl: Arc<dyn sticktalk_sfu::Authorizer>, mtu: u16) -> Self {
        let config = Config {
            bind_ip: "127.0.0.1".parse().unwrap(),
            advertised_ip: "127.0.0.1".parse().unwrap(),
            ports: 52000..=52999,
            video_egress_bps: cap,
            max_datagram_bytes: mtu,
            ..Config::default()
        };
        Self::with_config(config, acl)
    }
    pub fn with_config(config: Config, acl: Arc<dyn sticktalk_sfu::Authorizer>) -> Self {
        Self {
            sfu: Sfu::new(config, acl).unwrap(),
            peers: vec![],
            now: Instant::now(),
            pending: VecDeque::new(),
            signals: vec![],
            video_bytes: 0,
            max_rtp: 0,
            drop_next_video: false,
            dropped: 0,
        }
    }
    pub fn add(&mut self, sender: bool) -> usize {
        self.add_caps(sender, caps(), VideoCodec::H264)
    }
    pub fn add_caps(
        &mut self,
        sender: bool,
        capabilities: VoiceCapabilities,
        codec: VideoCodec,
    ) -> usize {
        self.add_layers(sender, capabilities, codec, &[])
    }
    pub fn add_layers(
        &mut self,
        sender: bool,
        capabilities: VoiceCapabilities,
        codec: VideoCodec,
        layers: &[&str],
    ) -> usize {
        let index = self.peers.len();
        let key = session(index as u64 + 1);
        self.sfu
            .handle_signal(
                key,
                1,
                VoiceSignal::Join {
                    channel_id: Id(1),
                    capabilities,
                },
                self.now,
            )
            .unwrap();
        self.collect_sfu();
        let address: SocketAddr = format!("127.0.0.1:{}", 55000 + index).parse().unwrap();
        let mut rtc = rtc_config().build(self.now);
        rtc.add_local_candidate(Candidate::host(address, "udp").unwrap())
            .unwrap();
        drain_initial(&mut rtc);
        let mut api = rtc.sdp_api();
        let dir = if sender {
            Direction::SendOnly
        } else {
            Direction::RecvOnly
        };
        let audio = api.add_media(MediaKind::Audio, dir, None, None, None);
        let simulcast = if layers.is_empty() {
            None
        } else {
            let mut s = str0m::media::Simulcast::new();
            for rid in layers {
                s.add_send_layer(str0m::media::SimulcastLayer::new(rid));
            }
            Some(s)
        };
        let video = api.add_media(MediaKind::Video, dir, None, None, simulcast);
        let screen = api.add_media(MediaKind::Video, dir, None, None, None);
        let channel = api.add_channel_with_config(ChannelConfig {
            label: CONTROL_CHANNEL_LABEL.into(),
            ..Default::default()
        });
        let (offer, pending) = api.apply().unwrap();
        drain_initial(&mut rtc);
        let tracks = if sender {
            vec![
                PublishedTrack {
                    mid: audio.to_string(),
                    kind: TrackKind::Audio,
                    codec: None,
                    layers: vec![],
                    surface: None,
                },
                PublishedTrack {
                    mid: video.to_string(),
                    kind: TrackKind::Camera,
                    codec: Some(codec),
                    layers: layers.iter().map(|s| s.to_string()).collect(),
                    surface: None,
                },
                PublishedTrack {
                    mid: screen.to_string(),
                    kind: TrackKind::Screen,
                    codec: Some(codec),
                    layers: vec![],
                    surface: Some(ShareSurface::Monitor),
                },
            ]
        } else {
            vec![]
        };
        self.sfu
            .handle_signal(
                key,
                2,
                VoiceSignal::Offer {
                    sdp: offer.to_sdp_string(),
                    tracks,
                },
                self.now,
            )
            .unwrap();
        let output: Vec<_> = self.sfu.drain().collect();
        let sdp = output
            .iter()
            .find_map(|o| {
                if let Out::Signal {
                    to,
                    event: VoiceEvent::Answer { sdp },
                    ..
                } = o
                {
                    (*to == key).then_some(sdp.clone())
                } else {
                    None
                }
            })
            .expect("SDP answer");
        for out in output {
            self.queue_out(out);
        }
        rtc.sdp_api()
            .accept_answer(pending, SdpAnswer::from_sdp_string(&sdp).unwrap())
            .unwrap();
        self.peers.push(Peer {
            rtc,
            address,
            key,
            audio,
            video,
            screen,
            channel,
            events: vec![],
            deadline: self.now,
            received: 0,
            frames: 0,
            control: vec![],
        });
        self.collect_peer(index);
        self.advance(Duration::from_millis(500));
        assert!(
            self.peers[index].rtc.is_connected(),
            "peer must complete ICE/DTLS"
        );
        index
    }
    pub fn signal(
        &mut self,
        index: usize,
        signal: VoiceSignal,
    ) -> Result<(), sticktalk_sfu::Error> {
        let result = self
            .sfu
            .handle_signal(self.peers[index].key, 3, signal, self.now);
        self.collect_sfu();
        self.flush();
        result
    }
    pub fn subscribe(&mut self, receiver: usize, publisher: usize, kind: TrackKind) {
        let mid = |p: &Peer| match kind {
            TrackKind::Audio => p.audio,
            TrackKind::Screen => p.screen,
            _ => p.video,
        };
        self.signal(
            receiver,
            VoiceSignal::Subscribe {
                publisher: self.peers[publisher].key,
                source_mid: mid(&self.peers[publisher]).to_string(),
                receiver_mid: mid(&self.peers[receiver]).to_string(),
                rid: None,
            },
        )
        .unwrap();
    }
    pub fn collect_sfu(&mut self) {
        let outs: Vec<_> = self.sfu.drain().collect();
        for out in outs {
            self.queue_out(out);
        }
    }
    fn queue_out(&mut self, out: Out) {
        match out {
            Out::Signal { to, event, .. } => self.signals.push((to, event)),
            Out::Datagram { to, bytes } => {
                if is_rtp(&bytes) {
                    self.max_rtp = self.max_rtp.max(bytes.len());
                    // Tests measure video separately via negotiated H264 PTs
                    // (all peers use the same config in this fixture).
                    let pt = bytes[1] & 127;
                    if pt != 111 {
                        self.video_bytes += bytes.len() as u64;
                    }
                    if pt != 111 && self.drop_next_video {
                        self.drop_next_video = false;
                        self.dropped += 1;
                        return;
                    }
                }
                self.pending.push_back((self.sfu.local_addr(), to, bytes));
            }
        }
    }
    pub fn collect_peer(&mut self, index: usize) {
        loop {
            let peer = &mut self.peers[index];
            match peer.rtc.poll_output().unwrap() {
                Output::Timeout(t) => {
                    peer.deadline = t;
                    break;
                }
                Output::Transmit(tx) => {
                    self.pending
                        .push_back((peer.address, tx.destination, tx.contents.to_vec()))
                }
                Output::Event(Event::RtpPacket(p)) => {
                    peer.received += 1;
                    if p.header.marker {
                        peer.frames += 1;
                    }
                    peer.events.push(Event::RtpPacket(p));
                }
                Output::Event(Event::ChannelData(d)) => {
                    peer.control.push(rmp_serde::from_slice(&d.data).unwrap());
                }
                Output::Event(e) => peer.events.push(e),
            }
        }
    }
    pub fn flush(&mut self) {
        let mut rounds = 0;
        while let Some((from, to, bytes)) = self.pending.pop_front() {
            rounds += 1;
            assert!(rounds < 100000, "no busy network loop");
            if to == self.sfu.local_addr() {
                self.sfu.handle_datagram(from, &bytes, self.now).unwrap();
                self.collect_sfu();
            } else if let Some(index) = self.peers.iter().position(|p| p.address == to) {
                let receive = Receive::new(Protocol::Udp, from, to, &bytes).unwrap();
                self.peers[index]
                    .rtc
                    .handle_input(Input::Receive(self.now, receive))
                    .unwrap();
                self.collect_peer(index);
            }
        }
    }
    pub fn advance(&mut self, duration: Duration) {
        let end = self.now + duration;
        self.flush();
        let mut rounds = 0;
        while self.now < end {
            rounds += 1;
            assert!(rounds < 1000000, "timer progress");
            let next = self
                .peers
                .iter()
                .map(|p| p.deadline)
                .chain(self.sfu.next_timeout())
                .min()
                .unwrap_or(end)
                .min(end);
            self.now = next.max(self.now + Duration::from_micros(100)).min(end);
            self.sfu.handle_timeout(self.now).unwrap();
            self.collect_sfu();
            for index in 0..self.peers.len() {
                if self.peers[index].deadline <= self.now {
                    self.peers[index]
                        .rtc
                        .handle_input(Input::Timeout(self.now))
                        .unwrap();
                    self.collect_peer(index);
                }
            }
            self.flush();
        }
    }
    pub fn send(
        &mut self,
        index: usize,
        kind: TrackKind,
        seq: u64,
        time: u32,
        payload: &[u8],
        marker: bool,
    ) {
        let peer = &mut self.peers[index];
        let (mid, codec) = match kind {
            TrackKind::Audio => (peer.audio, Codec::Opus),
            TrackKind::Screen => (peer.screen, Codec::H264),
            _ => (peer.video, Codec::H264),
        };
        let pt = peer
            .rtc
            .codec_config()
            .params()
            .iter()
            .find(|p| {
                p.spec().codec == codec
                    && peer.rtc.media(mid).unwrap().remote_pts().contains(&p.pt())
            })
            .unwrap()
            .pt();
        let ssrc = peer
            .rtc
            .direct_api()
            .stream_tx_by_mid(mid, None)
            .unwrap()
            .ssrc();
        self.collect_peer(index);
        self.peers[index]
            .rtc
            .direct_api()
            .stream_tx(&ssrc)
            .unwrap()
            .set_unpaced(true);
        self.collect_peer(index);
        self.write(
            index,
            ssrc,
            pt,
            seq,
            time,
            payload,
            marker,
            ExtensionValues {
                audio_level: Some(-24),
                voice_activity: Some(true),
                ..Default::default()
            },
        );
    }
    #[allow(clippy::too_many_arguments)]
    pub fn write(
        &mut self,
        index: usize,
        ssrc: Ssrc,
        pt: str0m::media::Pt,
        seq: u64,
        time: u32,
        payload: &[u8],
        marker: bool,
        ext: ExtensionValues,
    ) {
        self.peers[index]
            .rtc
            .direct_api()
            .stream_tx(&ssrc)
            .unwrap()
            .write_rtp(
                RtpWrite::new(pt, seq.into(), time, self.now, payload)
                    .marker(marker)
                    .ext_vals(ext)
                    .nackable(true),
            );
        self.collect_peer(index);
        self.peers[index]
            .rtc
            .handle_input(Input::Timeout(self.now))
            .unwrap();
        self.collect_peer(index);
        self.flush();
    }
    pub fn data(&mut self, index: usize, message: ControlData) {
        let bytes = rmp_serde::to_vec_named(&message).unwrap();
        let channel = self.peers[index].channel;
        assert!(self.peers[index]
            .rtc
            .channel(channel)
            .unwrap()
            .write(true, &bytes)
            .unwrap());
        self.collect_peer(index);
        self.flush();
        self.advance(Duration::from_millis(5));
    }
    pub fn clear(&mut self) {
        self.signals.clear();
        for p in &mut self.peers {
            p.events.clear();
            p.control.clear();
        }
    }
}
fn drain_initial(rtc: &mut Rtc) {
    loop {
        match rtc.poll_output().unwrap() {
            Output::Timeout(_) => break,
            Output::Event(_) => {}
            Output::Transmit(_) => panic!("network before negotiation"),
        }
    }
}
fn is_rtp(b: &[u8]) -> bool {
    b.len() >= 12 && b[0] >> 6 == 2 && !(64..=95).contains(&(b[1] & 127))
}
