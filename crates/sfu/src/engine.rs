use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::control::{Budget, Grant};
use crate::validation::{self, valid_id};
use sticktalk_proto::*;
use str0m::bwe::BweKind;
use str0m::channel::ChannelId as DataChannelId;
use str0m::format::{Codec, PayloadParams};
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, Mid, Rid};
use str0m::net::{Protocol, Receive};
use str0m::rtp::{ExtensionValues, RtpPacket, RtpWrite, Ssrc};
use str0m::{Candidate, Event, Input, Output, Rtc};

#[cfg(test)]
#[path = "overload_tests.rs"]
mod overload_tests;
#[cfg(test)]
#[path = "robustness_tests.rs"]
mod robustness_tests;

/// Synchronous, fast host ACL check. Called for EVERY signaling/media/control
/// action. The host must also disconnect on logout/WS loss and ACL revocation.
pub trait Authorizer: Send + Sync {
    fn can_join(&self, session: VoiceSession, channel: ChannelId) -> bool;
    fn permissions(&self, session: VoiceSession, channel: ChannelId) -> Permissions {
        if self.can_join(session, channel) {
            Permissions::all()
        } else {
            Permissions::empty()
        }
    }
    fn relocated_channel(&self, _session: VoiceSession) -> Option<ChannelId> {
        None
    }
    fn receive_audio(&self, _session: VoiceSession) -> bool {
        true
    }
    fn send_audio(&self, _session: VoiceSession) -> bool {
        true
    }
    /// Increment synchronously whenever a live policy changes. Old queued media is dropped.
    fn generation(&self) -> u64 {
        0
    }
    /// The Opus bitrate this session's call uses. Microphone audio well above it
    /// (a client ignoring the channel setting) is dropped. None: no limit.
    fn audio_bitrate(&self, _session: VoiceSession) -> Option<u64> {
        None
    }
    /// A Stage audience member: receives only. It never appears in another
    /// session's `Joined` or `Participant`, counts against the listener caps
    /// instead of the room cap, and anything it sends is dropped. Re-read on
    /// every `Refresh`, so a session moves between audience and speakers.
    fn listener(&self, _session: VoiceSession) -> bool {
        false
    }
}
impl<F: Fn(VoiceSession, ChannelId) -> bool + Send + Sync> Authorizer for F {
    fn can_join(&self, session: VoiceSession, channel: ChannelId) -> bool {
        self(session, channel)
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_ip: IpAddr,
    pub advertised_ip: IpAddr,
    /// One multiplexed UDP socket, bound to the first available port in range.
    pub ports: RangeInclusive<u16>,
    /// Global VIDEO egress budget, including RTX and probing, in UDP bytes/sec *8.
    pub video_egress_bps: u64,
    /// Maximum incoming and outgoing SRTP UDP payload, including RTX/auth tags.
    pub max_datagram_bytes: u16,
    pub max_sessions: usize,
    pub max_channel_sessions: usize,
    pub max_peer_tracks: usize,
    pub max_room_tracks: usize,
    pub peer_video_egress_bps: u64,
    pub room_video_egress_bps: u64,
    pub max_room_ice_candidates: usize,
    pub max_room_renegotiations_per_minute: usize,
    /// Listener (Stage audience) sessions in all rooms, apart from `max_sessions`.
    pub max_listeners: usize,
    /// Listener sessions in one room, apart from `max_channel_sessions`.
    pub max_room_listeners: usize,
}
/// What one listener's video subscription must be able to get. Listener video
/// subscriptions are refused (`Limit`) beyond `video_egress_bps` / this, so a
/// large audience cannot dilute everyone's picture to nothing.
pub const LISTENER_VIDEO_BPS: u64 = 500_000;
/// Source addresses remembered per peer for packet demultiplexing.
const PEER_ADDRESSES: usize = 4;
impl Default for Config {
    fn default() -> Self {
        Self {
            bind_ip: "127.0.0.1".parse().unwrap(),
            advertised_ip: "127.0.0.1".parse().unwrap(),
            ports: 50000..=50100,
            video_egress_bps: 32_000_000,
            max_datagram_bytes: DEFAULT_VOICE_UDP_BYTES,
            max_sessions: 64,
            max_channel_sessions: 16,
            max_peer_tracks: 4,
            max_room_tracks: 48,
            peer_video_egress_bps: 32_000_000,
            room_video_egress_bps: 64_000_000,
            max_room_ice_candidates: 1024,
            max_room_renegotiations_per_minute: 120,
            max_listeners: 1000,
            max_room_listeners: 1000,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("voice peer failed; leave and rejoin the call")]
    PeerFailed,
    #[error("SFU requires panic=unwind; build with CARGO_PROFILE_RELEASE_PANIC=unwind")]
    PanicStrategy,
    #[error("invalid SFU configuration")]
    Config,
    #[error("UDP: {0}")]
    Io(#[from] std::io::Error),
    #[error("WebRTC negotiation/transport failed")]
    Rtc(#[from] str0m::RtcError),
    #[error("invalid media signal")]
    Invalid,
    #[error("not authorized")]
    Forbidden,
    #[error("session/resource limit")]
    Limit,
    #[error("voice schema version/kind is unsupported")]
    Version,
    #[error("output queue full: drain after each operation")]
    Backpressure,
}
impl Error {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Forbidden => ErrorCode::Forbidden,
            Self::Limit => ErrorCode::LimitExceeded,
            Self::Version => ErrorCode::VersionMismatch,
            Self::Backpressure => ErrorCode::Unavailable,
            Self::PeerFailed | Self::PanicStrategy => ErrorCode::Unavailable,
            Self::Io(_) => ErrorCode::Unavailable,
            _ => ErrorCode::InvalidRequest,
        }
    }
}

#[derive(Debug)]
pub enum Out {
    Signal {
        to: VoiceSession,
        nonce: Nonce,
        event: VoiceEvent,
    },
    Datagram {
        to: SocketAddr,
        bytes: Vec<u8>,
    },
}

#[derive(Clone)]
struct Route {
    publisher: VoiceSession,
    source: Mid,
    target: Mid,
    rid: Option<Rid>,
    ssrc: Ssrc,
    video: bool,
    kind: TrackKind,
    first_source_seq: Option<u64>,
    // A new subscription starts only on a video keyframe.
    waiting_keyframe: bool,
    burst: FrameBurst,
    keyframe_ends: BTreeMap<u16, (u32, u64)>,
    rtx: Option<Ssrc>,
}

/// Diagnostic frame identity, including H.264 FU continuations. No payload retained.
#[derive(Clone, Default)]
struct FrameBurst {
    timestamp: Option<u32>,
    keyframe: bool,
    bytes: u64,
    keyframe_bytes: u64,
}
impl FrameBurst {
    fn observe(&mut self, timestamp: u32, keyframe: bool, bytes: usize) {
        if self.timestamp != Some(timestamp) {
            self.timestamp = Some(timestamp);
            self.keyframe = false;
            self.bytes = 0;
        }
        self.keyframe |= keyframe;
        self.bytes = self.bytes.saturating_add(bytes as u64);
        if self.keyframe {
            self.keyframe_bytes = self.bytes;
        }
    }
}

struct Peer {
    failed: bool,
    idr_starts: u64,
    media_cap_drops: u64,
    gated_packets: u64,
    keyframe_times: BTreeMap<(Mid, Option<String>), Instant>,
    keyframe_forwarded: u64,
    keyframe_suppressed: u64,
    negotiation_times: VecDeque<Instant>,
    negotiated_mids: BTreeMap<String, String>,
    candidates: BTreeSet<String>,
    ssrcs: BTreeSet<u32>,
    video_budget: Budget,
    pending_media: BTreeMap<Ssrc, VecDeque<(u16, Instant, usize)>>,
    queue_resets: u64,
    queue_trace_at: Instant,
    rtc: Rtc,
    channel: ChannelId,
    capabilities: VoiceCapabilities,
    tracks: Vec<PublishedTrack>,
    routes: BTreeMap<String, Route>,
    muted: bool,
    deafened: bool,
    control: Option<DataChannelId>,
    grant: Option<Grant>,
    request: Option<(VoiceSession, Instant)>,
    deadline: Instant,
    created: Instant,
    connected: bool,
    speaking_until: Option<Instant>,
    stats: VoiceStats,
    stats_at: Instant,
    tx_bytes: u64,
    wire_tx_bytes: u64,
    rx_bytes: u64,
    tx_frames: u64,
    rx_frames: u64,
    bwe: u64,
    bwe_target: u64,
    video_active: bool,
    limits: BTreeMap<String, u64>,
    /// What each published video track needs (TrackQuality), by MID.
    demands: BTreeMap<String, u64>,
    /// The call's microphone bitrate ceiling, when the host sets one.
    audio_policer: Option<crate::quality::AudioPolicer>,
    /// A Stage audience member (`Authorizer::listener`), as last applied.
    listener: bool,
    /// Source addresses this peer's packets were accepted from (newest last).
    addresses: VecDeque<SocketAddr>,
}
impl Peer {
    fn participant(&self, session: VoiceSession) -> VoiceParticipant {
        VoiceParticipant {
            session,
            muted: self.muted,
            deafened: self.deafened,
            tracks: self.tracks.clone(),
        }
    }
}

/// Single-owner sans-IO engine, with an optional Tokio UDP driver. No tasks or
/// timers are started by new(). Every operation must be followed by drain().
pub struct Sfu {
    pub(crate) socket: Option<UdpSocket>,
    config: Config,
    // Private template also permits parser tests to reuse a DTLS identity
    // without bypassing fresh ICE/session/negotiation state for each case.
    rtc_template: str0m::RtcConfig,
    acl: Arc<dyn Authorizer>,
    local: SocketAddr,
    peers: BTreeMap<VoiceSession, Peer>,
    out: VecDeque<(u64, Out)>,
    events: VecDeque<(VoiceSession, Event)>,
    budget: Budget,
    room_budgets: BTreeMap<ChannelId, Budget>,
    room_negotiations: BTreeMap<ChannelId, VecDeque<Instant>>,
    /// Listeners' offers have their own room budget: a large audience
    /// joining at an event's start must not lock its speakers out.
    listener_negotiations: BTreeMap<ChannelId, VecDeque<Instant>>,
    now: Instant,
    // Used to reject abusive signaling independently of the chat command rate.
    signals: BTreeMap<VoiceSession, (Instant, u16)>,
    failure_epoch: u64,
    video_trace: bool,
    overloaded_sessions: BTreeSet<VoiceSession>,
    /// Which peer last sent from each address: one `accepts` check per
    /// datagram instead of one per peer (a Stage has a thousand peers).
    sources: std::collections::HashMap<SocketAddr, VoiceSession>,
    /// The policy generation `synchronize` last looked at.
    synced: Option<u64>,
}

pub fn rtc_config() -> str0m::RtcConfig {
    let mut config = Rtc::builder()
        .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
        .set_rtp_mode(true)
        .set_mtu(usize::from(DEFAULT_VOICE_UDP_BYTES)..=usize::from(DEFAULT_VOICE_UDP_BYTES))
        .set_stats_interval(Some(Duration::from_secs(1)))
        .set_send_buffer_video(256)
        .set_send_buffer_audio(32)
        .set_sctp_max_message_size(512)
        .set_sctp_max_buffered_amount(16 * 1024)
        .enable_bwe(Some(str0m::bwe::Bitrate::bps(8_000_000)));
    let codecs = config.codec_config();
    codecs.clear();
    codecs.enable_opus(true, false);
    // One alternative per H.264 profile/packetization-mode, NOT per level.
    // str0m matches across levels: two local High entries can both claim the
    // same sparse remote PT in recvonly/sendrecv offers and panic.
    for (pt, rtx, mode, profile) in [
        (127, 121, true, 0x42001f),
        (125, 107, false, 0x42001f),
        (108, 109, true, 0x42e01f),
        (124, 120, false, 0x42e01f),
        (123, 119, true, 0x4d001f),
        (35, 36, false, 0x4d001f),
    ] {
        codecs.add_h264(pt.into(), Some(rtx.into()), mode, profile);
    }
    codecs.add_h264(112.into(), Some(113.into()), true, 0x640033);
    codecs.enable_h265(true);
    codecs.enable_av1(true);
    config
}

impl Sfu {
    pub fn new(config: Config, acl: Arc<dyn Authorizer>) -> Result<Self, Error> {
        // catch_unwind cannot contain an aborting dependency. Refuse unsafe
        // deployment rather than claiming peer isolation that doesn't exist.
        if cfg!(panic = "abort") {
            return Err(Error::PanicStrategy);
        }
        if config.ports.is_empty()
            || *config.ports.start() == 0
            || config.bind_ip.is_ipv4() != config.advertised_ip.is_ipv4()
            || config.advertised_ip.is_unspecified()
            || config.advertised_ip.is_multicast()
            || config.video_egress_bps < 96_000
            || !(1200..=1472).contains(&config.max_datagram_bytes)
            || config.max_sessions == 0
            || config.max_sessions > 256
            || config.max_channel_sessions == 0
            || config.max_channel_sessions > 32
            || config.max_peer_tracks == 0
            || config.max_peer_tracks > 4
            || config.max_room_tracks == 0
            || config.max_room_tracks > 128
            || !(96_000..=1_000_000_000).contains(&config.video_egress_bps)
            || !(96_000..=1_000_000_000).contains(&config.peer_video_egress_bps)
            || !(96_000..=1_000_000_000).contains(&config.room_video_egress_bps)
            || !(1..=4096).contains(&config.max_room_ice_candidates)
            || !(1..=960).contains(&config.max_room_renegotiations_per_minute)
            || config.max_listeners > 10_000
            || config.max_room_listeners > config.max_listeners
        {
            return Err(Error::Config);
        }
        let mut socket = None;
        let mut last_error = None;
        for port in config.ports.clone() {
            match UdpSocket::bind(SocketAddr::new(config.bind_ip, port)) {
                Ok(s) => {
                    socket = Some(s);
                    break;
                }
                Err(e) => last_error = Some(e),
            }
        }
        let socket = socket.ok_or_else(|| {
            Error::Io(last_error.unwrap_or_else(|| std::io::Error::other("no UDP ports")))
        })?;
        // Windows' small default UDP buffers can lose a fragmented keyframe
        // burst even on loopback. Fixed 256 KiB buffers bound memory while
        // allowing the shared socket to absorb a few milliseconds of work.
        let socket_ref = socket2::SockRef::from(&socket);
        socket_ref.set_recv_buffer_size(256 * 1024)?;
        socket_ref.set_send_buffer_size(256 * 1024)?;
        socket.set_nonblocking(true)?;
        let local = SocketAddr::new(config.advertised_ip, socket.local_addr()?.port());
        let now = Instant::now();
        Ok(Self {
            socket: Some(socket),
            budget: Budget::new(config.video_egress_bps, config.max_datagram_bytes, now),
            config,
            rtc_template: rtc_config(),
            acl,
            local,
            peers: BTreeMap::new(),
            out: VecDeque::new(),
            events: VecDeque::new(),
            now,
            signals: BTreeMap::new(),
            failure_epoch: 0,
            video_trace: std::env::var_os("STICKTALK_VIDEO_TRACE").is_some(),
            overloaded_sessions: BTreeSet::new(),
            room_budgets: BTreeMap::new(),
            room_negotiations: BTreeMap::new(),
            listener_negotiations: BTreeMap::new(),
            sources: std::collections::HashMap::new(),
            synced: None,
        })
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }
    pub fn sessions(&self) -> Vec<VoiceSession> {
        self.peers.keys().copied().collect()
    }
    pub(crate) fn has_output(&self) -> bool {
        !self.out.is_empty()
    }
    pub fn session_count(&self) -> usize {
        self.peers.len()
    }
    pub fn route_count(&self) -> usize {
        self.peers.values().map(|p| p.routes.len()).sum()
    }
    pub fn drain(&mut self) -> impl Iterator<Item = Out> + '_ {
        let acl = self.acl.clone();
        self.out.drain(..).filter_map(move |(generation, out)| {
            if matches!(&out, Out::Datagram { .. }) && generation != acl.generation() {
                None
            } else {
                Some(out)
            }
        })
    }
    pub fn policy_generation(&self) -> u64 {
        self.acl.generation()
    }
    pub fn next_timeout(&self) -> Option<Instant> {
        self.peers
            .values()
            .flat_map(|p| {
                [
                    Some(p.deadline),
                    p.request.map(|r| r.1),
                    p.speaking_until,
                    (!p.connected).then_some(p.created + Duration::from_secs(30)),
                ]
            })
            .flatten()
            .min()
    }
    pub(crate) fn take_overloaded(&mut self) -> BTreeSet<VoiceSession> {
        std::mem::take(&mut self.overloaded_sessions)
    }
    /// Cleanup gets a reserved, temporarily empty queue. Preserve unrelated
    /// output; the driver delivers these mandatory signals on its control lane.
    pub(crate) fn overload_cleanup(&mut self, session: VoiceSession) -> Vec<Out> {
        let mut pending = std::mem::take(&mut self.out);
        pending.retain(|(_, out)| !matches!(out, Out::Signal {to, ..} if *to == session));
        // The host synchronously revokes this session even if peer cleanup fails.
        let _ = self.disconnect(session);
        let cleanup = self.drain().collect();
        self.out = pending;
        cleanup
    }
    fn emit(&mut self, out: Out) -> Result<(), Error> {
        if self.out.len() >= 4096 {
            return Err(Error::Backpressure);
        }
        self.out.push_back((self.acl.generation(), out));
        Ok(())
    }
    fn send(&mut self, to: VoiceSession, nonce: Nonce, event: VoiceEvent) -> Result<(), Error> {
        self.emit(Out::Signal { to, nonce, event })
    }
    fn broadcast(&mut self, channel: ChannelId, event: VoiceEvent) -> Result<(), Error> {
        let keys: Vec<_> = self
            .peers
            .iter()
            .filter(|(s, p)| p.channel == channel && self.acl.can_join(**s, channel))
            .map(|(s, _)| *s)
            .collect();
        for key in keys {
            self.send(key, 0, event.clone())?;
        }
        Ok(())
    }
    fn authorized(&self, key: VoiceSession) -> Result<(), Error> {
        let p = self.peers.get(&key).ok_or(Error::Forbidden)?;
        if self.acl.can_join(key, p.channel) {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
    /// Compatible voice envelope decoder. Inner payload is capped before decoding.
    pub fn handle_envelope(
        &mut self,
        key: VoiceSession,
        envelope: VoicePayload,
        now: Instant,
    ) -> Result<(), Error> {
        let decoded = if envelope.version != VOICE_VERSION || envelope.kind != "signal" {
            Err(Error::Version)
        } else if envelope.payload.len() > 72 * 1024 {
            Err(Error::Limit)
        } else {
            crate::boundary::run(|| validation::decode(&envelope.payload))
        };
        let signal = match decoded {
            Ok(signal) => signal,
            Err(error) => {
                if matches!(error, Error::PeerFailed) {
                    return self.peer_operation(key, envelope.nonce, |_| Err(error));
                }
                self.send(
                    key,
                    envelope.nonce,
                    VoiceEvent::Error {
                        code: error.code(),
                        message: error.to_string(),
                    },
                )?;
                return Err(error);
            }
        };
        self.handle_signal(key, envelope.nonce, signal, now)
    }
    /// The host passes the authenticated login/WS connection identity, NEVER a
    /// user ID copied from client input. Errors are also emitted to that session.
    fn synchronize(&mut self) -> Result<(), Error> {
        // Moves come with a policy change; between changes there is nothing
        // to look for (and asking about every peer on every packet is what a
        // large Stage cannot afford).
        let generation = self.acl.generation();
        if self.synced == Some(generation) {
            return Ok(());
        }
        let moves: Vec<_> = self
            .peers
            .iter()
            .filter_map(|(s, p)| {
                self.acl
                    .relocated_channel(*s)
                    .filter(|ch| *ch != p.channel)
                    .map(|ch| (*s, ch))
            })
            .collect();
        for (s, ch) in moves {
            if let Err(error) = self.move_session(s, ch) {
                if matches!(error, Error::Backpressure) {
                    return Err(error);
                }
                self.send(
                    s,
                    0,
                    VoiceEvent::Error {
                        code: error.code(),
                        message: error.to_string(),
                    },
                )?;
                self.disconnect(s)?;
            }
        }
        self.synced = Some(generation);
        Ok(())
    }
    pub fn handle_signal(
        &mut self,
        key: VoiceSession,
        nonce: Nonce,
        signal: VoiceSignal,
        now: Instant,
    ) -> Result<(), Error> {
        self.now = self.now.max(now);
        self.synchronize()?;
        let result = self.peer_operation(key, nonce, |s| s.signal(key, nonce, signal));
        if let Err(error) = &result {
            if matches!(error, Error::PeerFailed | Error::Rtc(_)) {
                return result;
            }
            self.send(
                key,
                nonce,
                VoiceEvent::Error {
                    code: error.code(),
                    message: error.to_string(),
                },
            )?;
        }
        self.pump()?;
        result
    }
    fn peer_operation<T>(
        &mut self,
        key: VoiceSession,
        nonce: Nonce,
        operation: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let epoch = self.failure_epoch;
        let result = crate::boundary::run(|| operation(self));
        if matches!(result, Err(Error::Backpressure)) && self.peers.contains_key(&key) {
            self.overloaded_sessions.insert(key);
        }
        if epoch == self.failure_epoch && matches!(result, Err(Error::PeerFailed | Error::Rtc(_))) {
            self.failure_epoch = self.failure_epoch.wrapping_add(1);
            // Never log the upstream error/panic payload: it may contain SDP.
            tracing::warn!(?key, "SFU peer session failed; closing only this peer");
            self.send(
                key,
                nonce,
                VoiceEvent::Error {
                    code: ErrorCode::Unavailable,
                    message: Error::PeerFailed.to_string(),
                },
            )?;
            if let Some(peer) = self.peers.get_mut(&key) {
                peer.failed = true;
                peer.control = None;
            }
            self.disconnect(key)?;
        }
        result
    }
    fn signal(
        &mut self,
        key: VoiceSession,
        nonce: Nonce,
        signal: VoiceSignal,
    ) -> Result<(), Error> {
        if let VoiceSignal::Join {
            channel_id,
            capabilities,
        } = signal
        {
            return self.join(key, nonce, channel_id, capabilities);
        }
        self.authorized(key)?;
        let rate = self.signals.entry(key).or_insert((self.now, 0));
        if self.now.saturating_duration_since(rate.0) >= Duration::from_secs(1) {
            *rate = (self.now, 0);
        }
        if rate.1 >= 64 {
            return Err(Error::Limit);
        }
        rate.1 += 1;
        match signal {
            VoiceSignal::Leave => return self.disconnect(key),
            VoiceSignal::Offer { sdp, tracks } => return self.offer(key, nonce, sdp, tracks),
            VoiceSignal::Ice { candidate } => {
                let parsed = validation::candidate(&candidate)?;
                let channel = self.peers[&key].channel;
                let room_ice: usize = self
                    .peers
                    .values()
                    .filter(|p| p.channel == channel)
                    .map(|p| p.candidates.len())
                    .sum();
                let peer = self.peers.get_mut(&key).unwrap();
                if peer.candidates.contains(&candidate) {
                    return self.send(key, nonce, VoiceEvent::Ack);
                }
                if peer.candidates.len() >= validation::MAX_ICE
                    || room_ice >= self.config.max_room_ice_candidates
                {
                    return Err(Error::Limit);
                }
                peer.candidates.insert(candidate);
                self.peers
                    .get_mut(&key)
                    .unwrap()
                    .rtc
                    .add_remote_candidate(parsed);
                self.collect(key)?;
            }
            VoiceSignal::State { muted, deafened } => {
                let p = self.peers.get_mut(&key).unwrap();
                p.muted = muted;
                p.deafened = deafened;
                let channel = p.channel;
                let listener = p.listener;
                let participant = p.participant(key);
                if muted || deafened {
                    self.stop_speaking(key)?;
                }
                if !listener {
                    self.broadcast(channel, VoiceEvent::Participant(participant))?;
                }
            }
            VoiceSignal::Subscribe {
                publisher,
                source_mid,
                receiver_mid,
                rid,
            } => self.subscribe(key, publisher, source_mid, receiver_mid, rid)?,
            VoiceSignal::Unsubscribe { receiver_mid } => {
                self.remove_route(key, &receiver_mid)?;
                self.allocate()?;
            }
            VoiceSignal::RequestKeyframe { receiver_mid, fir } => {
                let r = self.peers[&key]
                    .routes
                    .get(&receiver_mid)
                    .cloned()
                    .ok_or(Error::Invalid)?;
                if self.video_trace {
                    eprintln!("VIDEO_TRACE viewer_keyframe_request peer={key:?} publisher={:?} fir={fir} reason=signal", r.publisher);
                }
                self.keyframe(
                    r.publisher,
                    r.source,
                    r.rid,
                    if fir {
                        KeyframeRequestKind::Fir
                    } else {
                        KeyframeRequestKind::Pli
                    },
                )?;
            }
            VoiceSignal::RequestControl { sharer } => self.request_control(key, sharer)?,
            VoiceSignal::DecideControl { requester, allow } => {
                self.decide_control(key, requester, allow)?
            }
            VoiceSignal::RevokeControl => {
                self.revoke(key)?;
                let sharers: Vec<_> = self
                    .peers
                    .iter()
                    .filter(|(_, p)| p.grant.as_ref().is_some_and(|g| g.controller == key))
                    .map(|(s, _)| *s)
                    .collect();
                for sharer in sharers {
                    self.revoke(sharer)?;
                }
            }
            VoiceSignal::Activity {
                voice_activity,
                priority,
            } => {
                let p = self.acl.permissions(key, self.peers[&key].channel);
                if (voice_activity && !p.contains(Permissions::USE_VOICE_ACTIVITY))
                    || (priority && !p.contains(Permissions::PRIORITY_SPEAKER))
                {
                    return Err(Error::Forbidden);
                }
            }
            // Host-authorized call signaling; never forwarded as untrusted SFU input.
            VoiceSignal::Moderate { .. }
            | VoiceSignal::Ring { .. }
            | VoiceSignal::DeclineCall { .. }
            | VoiceSignal::PlaySound { .. }
            | VoiceSignal::SetClipping { .. }
            | VoiceSignal::ClipTaken
            | VoiceSignal::Stage(_)
            | VoiceSignal::Activities(_) => return Err(Error::Forbidden),
            VoiceSignal::TrackQuality { mid, max_bps } => {
                let p = self.peers.get_mut(&key).unwrap();
                let published = p.tracks.iter().any(|t| t.mid == mid && is_video(t.kind));
                if !published || !(100_000..=MAX_TRACK_BITRATE).contains(&max_bps) {
                    return Err(Error::Invalid);
                }
                p.demands.insert(mid, max_bps);
                self.allocate()?;
            }
            VoiceSignal::Stats => {
                return self.send(
                    key,
                    nonce,
                    VoiceEvent::Stats(self.peers[&key].stats.clone()),
                )
            }
            VoiceSignal::Join { .. } => unreachable!(),
        }
        self.send(key, nonce, VoiceEvent::Ack)
    }
    fn join(
        &mut self,
        key: VoiceSession,
        nonce: Nonce,
        channel: ChannelId,
        capabilities: VoiceCapabilities,
    ) -> Result<(), Error> {
        if key.user_id.0 == 0
            || key.connection_id.0 == 0
            || channel.0 == 0
            || !self.acl.can_join(key, channel)
        {
            return Err(Error::Forbidden);
        }
        if self.peers.contains_key(&key) {
            return Err(Error::Invalid);
        }
        let listener = self.acl.listener(key);
        if !self.room_has_space(channel, listener, true) {
            return Err(Error::Limit);
        }
        if !capabilities.video.contains(&VideoCodec::H264)
            || capabilities.video.len() > 3
            || capabilities.hardware_decode.len() > 3
            || capabilities
                .hardware_decode
                .iter()
                .any(|c| !capabilities.video.contains(c))
        {
            return Err(Error::Invalid);
        }
        let participants = self
            .peers
            .iter()
            .filter(|(_, p)| p.channel == channel && !p.listener)
            .map(|(s, p)| p.participant(*s))
            .collect();
        let rtc = self
            .rtc_template
            .clone()
            .set_mtu(
                usize::from(self.config.max_datagram_bytes)
                    ..=usize::from(self.config.max_datagram_bytes),
            )
            .set_ice_lite(true)
            .build(self.now);
        let p = Peer {
            failed: false,
            idr_starts: 0,
            media_cap_drops: 0,
            gated_packets: 0,
            keyframe_times: BTreeMap::new(),
            keyframe_forwarded: 0,
            keyframe_suppressed: 0,
            negotiation_times: VecDeque::new(),
            negotiated_mids: BTreeMap::new(),
            candidates: BTreeSet::new(),
            ssrcs: BTreeSet::new(),
            pending_media: BTreeMap::new(),
            queue_resets: 0,
            queue_trace_at: self.now,
            video_budget: Budget::new(
                self.config.peer_video_egress_bps,
                self.config.max_datagram_bytes,
                self.now,
            ),
            rtc,
            channel,
            capabilities,
            tracks: vec![],
            routes: BTreeMap::new(),
            muted: false,
            deafened: false,
            control: None,
            grant: None,
            request: None,
            deadline: self.now,
            created: self.now,
            connected: false,
            speaking_until: None,
            stats: VoiceStats {
                tx_bps: 0,
                rx_bps: 0,
                tx_fps_milli: 0,
                rx_fps_milli: 0,
                loss_per_million: None,
                rtt_ms: None,
                path: MediaPath::Unknown,
                cap_dropped_packets: 0,
                queue_dropped_packets: 0,
                mtu_dropped_packets: 0,
            },
            stats_at: self.now,
            tx_bytes: 0,
            wire_tx_bytes: 0,
            rx_bytes: 0,
            tx_frames: 0,
            rx_frames: 0,
            bwe: self.config.video_egress_bps,
            bwe_target: 0,
            video_active: false,
            limits: BTreeMap::new(),
            demands: BTreeMap::new(),
            audio_policer: None,
            listener,
            addresses: VecDeque::new(),
        };
        self.peers.insert(key, p);
        self.room_budgets.entry(channel).or_insert_with(|| {
            Budget::new(
                self.config.room_video_egress_bps,
                self.config.max_datagram_bytes,
                self.now,
            )
        });
        self.peers
            .get_mut(&key)
            .unwrap()
            .rtc
            .add_local_candidate(Candidate::host(self.local, "udp").map_err(|_| Error::Config)?)
            .ok_or(Error::Config)?;
        self.collect(key)?;
        self.send(
            key,
            nonce,
            VoiceEvent::Joined {
                channel_id: channel,
                participants,
                max_rtp_bytes: self.config.max_datagram_bytes,
            },
        )?;
        if !listener {
            self.broadcast(
                channel,
                VoiceEvent::Participant(self.peers[&key].participant(key)),
            )?;
        }
        self.enforce_codecs(channel)
    }
    /// Whether one more session (a listener or not) fits the room cap and,
    /// for a new session, the global cap.
    fn room_has_space(&self, channel: ChannelId, listener: bool, global: bool) -> bool {
        let (all, room) = self
            .peers
            .values()
            .filter(|p| p.listener == listener)
            .fold((0, 0), |(all, room), p| {
                (all + 1, room + usize::from(p.channel == channel))
            });
        let (max_all, max_room) = if listener {
            (self.config.max_listeners, self.config.max_room_listeners)
        } else {
            (self.config.max_sessions, self.config.max_channel_sessions)
        };
        (!global || all < max_all) && room < max_room
    }
    /// Sends to everyone in the room except `key` itself.
    fn broadcast_others(
        &mut self,
        channel: ChannelId,
        key: VoiceSession,
        event: VoiceEvent,
    ) -> Result<(), Error> {
        let keys: Vec<_> = self
            .peers
            .iter()
            .filter(|(s, p)| **s != key && p.channel == channel && self.acl.can_join(**s, channel))
            .map(|(s, _)| *s)
            .collect();
        for to in keys {
            self.send(to, 0, event.clone())?;
        }
        Ok(())
    }
    fn accepts(&mut self, candidate: VoiceSession, input: &Input) -> Result<bool, Error> {
        match self.peer_operation(candidate, 0, |s| {
            Ok(s.peers
                .get(&candidate)
                .is_some_and(|p| p.rtc.accepts(input)))
        }) {
            Ok(accepted) => Ok(accepted),
            Err(Error::Backpressure) => Err(Error::Backpressure),
            Err(_) => Ok(false),
        }
    }
    /// Remembers that `key` sends from `from`, at most `PEER_ADDRESSES` each.
    fn remember_source(&mut self, key: VoiceSession, from: SocketAddr) {
        if self.sources.get(&from) == Some(&key) {
            return;
        }
        if let Some(old) = self.sources.insert(from, key) {
            if let Some(p) = self.peers.get_mut(&old) {
                p.addresses.retain(|a| *a != from);
            }
        }
        let Some(p) = self.peers.get_mut(&key) else {
            return;
        };
        p.addresses.push_back(from);
        let mut evicted = vec![];
        while p.addresses.len() > PEER_ADDRESSES {
            evicted.extend(p.addresses.pop_front());
        }
        for a in evicted {
            if self.sources.get(&a) == Some(&key) {
                self.sources.remove(&a);
            }
        }
    }
    fn codec_allowed(&self, channel: ChannelId, codec: VideoCodec) -> bool {
        codec == VideoCodec::H264
            || self
                .peers
                .values()
                .filter(|p| p.channel == channel)
                .all(|p| {
                    p.capabilities.video.contains(&codec)
                        && p.capabilities.hardware_decode.contains(&codec)
                })
    }
    fn enforce_codecs(&mut self, channel: ChannelId) -> Result<(), Error> {
        let incompatible: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, p)| p.channel == channel)
            .flat_map(|(key, p)| {
                p.tracks
                    .iter()
                    .filter(|t| t.codec.is_some_and(|c| !self.codec_allowed(channel, c)))
                    .map(|t| (*key, t.mid.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (key, mid) in incompatible {
            self.send(
                key,
                0,
                VoiceEvent::CodecRequired {
                    mid,
                    codec: VideoCodec::H264,
                },
            )?;
        }
        Ok(())
    }
    fn offer(
        &mut self,
        key: VoiceSession,
        nonce: Nonce,
        sdp: String,
        tracks: Vec<PublishedTrack>,
    ) -> Result<(), Error> {
        let peer = self.peers.get_mut(&key).ok_or(Error::Forbidden)?;
        while peer
            .negotiation_times
            .front()
            .is_some_and(|t| self.now.saturating_duration_since(*t) >= Duration::from_secs(60))
        {
            peer.negotiation_times.pop_front();
        }
        if peer.negotiation_times.len() >= validation::MAX_RENEGOTIATIONS {
            return Err(Error::Limit);
        }
        // Count attempts, including malformed offers; don't allow parse-CPU abuse.
        peer.negotiation_times.push_back(self.now);
        let channel = peer.channel;
        let (room_times, room_limit) = if peer.listener {
            (
                self.listener_negotiations.entry(channel).or_default(),
                self.config.max_room_listeners + self.config.max_room_renegotiations_per_minute,
            )
        } else {
            (
                self.room_negotiations.entry(channel).or_default(),
                self.config.max_room_renegotiations_per_minute,
            )
        };
        while room_times
            .front()
            .is_some_and(|t| self.now.saturating_duration_since(*t) >= Duration::from_secs(60))
        {
            room_times.pop_front();
        }
        if room_times.len() >= room_limit {
            return Err(Error::Limit);
        }
        room_times.push_back(self.now);
        let room_tracks: usize = self
            .peers
            .iter()
            .filter(|(s, p)| **s != key && p.channel == channel)
            .map(|(_, p)| p.tracks.len())
            .sum();
        if tracks.len() > self.config.max_peer_tracks
            || room_tracks + tracks.len() > self.config.max_room_tracks
        {
            return Err(Error::Limit);
        }
        // The audience publishes nothing, whatever its permissions say.
        if self.peers[&key].listener && !tracks.is_empty() {
            return Err(Error::Forbidden);
        }
        let mut mids = BTreeSet::new();
        let mut kinds = BTreeSet::new();
        for t in &tracks {
            if !self
                .acl
                .permissions(key, self.peers[&key].channel)
                .contains(track_permission(t.kind))
            {
                return Err(Error::Forbidden);
            }
            if !valid_id(&t.mid)
                || !mids.insert(&t.mid)
                || !kinds.insert(t.kind as u8)
                || t.layers.len() > 3
                || t.layers.iter().any(|r| !valid_id(r))
                || t.layers.iter().collect::<BTreeSet<_>>().len() != t.layers.len()
            {
                return Err(Error::Invalid);
            }
            let video = is_video(t.kind);
            if video != t.codec.is_some()
                || (t.kind == TrackKind::Screen) != t.surface.is_some()
                || (!video && !t.layers.is_empty())
            {
                return Err(Error::Invalid);
            }
            if t.codec
                .is_some_and(|c| !self.codec_allowed(self.peers[&key].channel, c))
            {
                return Err(Error::Invalid);
            }
        }
        let (offer, description) = validation::offer(&sdp)?;
        let peer = &self.peers[&key];
        let other_ice: usize = self
            .peers
            .iter()
            .filter(|(s, p)| **s != key && p.channel == channel)
            .map(|(_, p)| p.candidates.len())
            .sum();
        // str0m retains historical media slots. Bound lifetime MID growth, not
        // just m-lines in a single offer, and disallow changing a MID's kind.
        if peer
            .negotiated_mids
            .keys()
            .chain(description.mids.keys())
            .collect::<BTreeSet<_>>()
            .len()
            > validation::MAX_MEDIA
            || peer.candidates.union(&description.candidates).count() > validation::MAX_ICE
            || peer.ssrcs.union(&description.ssrcs).count() > validation::MAX_SSRC
            || other_ice + peer.candidates.union(&description.candidates).count()
                > self.config.max_room_ice_candidates
        {
            return Err(Error::Limit);
        }
        if description
            .mids
            .iter()
            .any(|(mid, kind)| peer.negotiated_mids.get(mid).is_some_and(|old| old != kind))
        {
            return Err(Error::Invalid);
        }
        let peer = self.peers.get_mut(&key).unwrap();
        peer.negotiated_mids.extend(description.mids);
        peer.candidates.extend(description.candidates);
        peer.ssrcs.extend(description.ssrcs);
        // Re-negotiation invalidates grants even when the same MID is reused.
        self.revoke(key)?;
        self.cancel_request(key)?;
        let answer = self
            .peers
            .get_mut(&key)
            .unwrap()
            .rtc
            .sdp_api()
            .accept_offer(offer);
        self.collect(key)?;
        let answer = answer?;
        let valid = tracks.iter().all(|t| {
            let mid: Mid = t.mid.as_str().into();
            let p = &self.peers[&key];
            p.rtc.media(mid).is_some_and(|m| {
                !m.disabled()
                    && m.kind() == media_kind(t.kind)
                    && matches!(m.direction(), Direction::RecvOnly | Direction::SendRecv)
                    && p.rtc.codec_config().params().iter().any(|param| {
                        m.remote_pts().contains(&param.pt()) && param.spec().codec == codec_of(t)
                    })
            })
        });
        if !valid {
            return Err(Error::PeerFailed);
        }
        // Remove subscriptions to withdrawn/reconfigured sources. Other sessions
        // receive the full publication set and can subscribe after their offer.
        self.peers
            .get_mut(&key)
            .unwrap()
            .keyframe_times
            .retain(|(mid, rid), _| {
                tracks.iter().any(|t| {
                    t.mid == mid.to_string()
                        && rid
                            .as_ref()
                            .is_none_or(|rid| t.layers.iter().any(|r| r == rid))
                })
            });
        let old = self.peers[&key].tracks.clone();
        let changed: Vec<_> = old
            .iter()
            .filter(|t| !tracks.contains(t))
            .map(|t| t.mid.clone())
            .collect();
        let stale: Vec<_> = self
            .peers
            .iter()
            .flat_map(|(s, p)| {
                p.routes
                    .iter()
                    .filter(|(_, r)| {
                        r.publisher == key && changed.iter().any(|mid| mid == &r.source.to_string())
                    })
                    .map(|(mid, _)| (*s, mid.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (s, mid) in stale {
            self.remove_route(s, &mid)?;
        }
        let p = self.peers.get_mut(&key).unwrap();
        // A need is kept only while its track is still published.
        p.demands
            .retain(|mid, _| tracks.iter().any(|t| t.mid == *mid && is_video(t.kind)));
        p.tracks = tracks;
        // A receiver's renegotiation can remove a local receiving slot.
        let stale: Vec<_> = self.peers[&key]
            .routes
            .iter()
            .filter(|(_, r)| {
                self.peers[&key].rtc.media(r.target).is_none_or(|m| {
                    m.disabled()
                        || !matches!(m.direction(), Direction::SendOnly | Direction::SendRecv)
                })
            })
            .map(|(mid, _)| mid.clone())
            .collect();
        for mid in stale {
            self.remove_route(key, &mid)?;
        }
        let sdp = answer.to_sdp_string();
        validation::answer(&sdp).map_err(|_| Error::PeerFailed)?;
        self.send(key, nonce, VoiceEvent::Answer { sdp })?;
        if !self.peers[&key].listener {
            self.broadcast(
                self.peers[&key].channel,
                VoiceEvent::Participant(self.peers[&key].participant(key)),
            )?;
        }
        self.allocate()
    }
    fn subscribe(
        &mut self,
        key: VoiceSession,
        publisher: VoiceSession,
        source_mid: String,
        receiver_mid: String,
        rid: Option<String>,
    ) -> Result<(), Error> {
        self.authorized(publisher)?;
        if key == publisher
            || !valid_id(&source_mid)
            || !valid_id(&receiver_mid)
            || rid.as_ref().is_some_and(|r| !valid_id(r))
        {
            return Err(Error::Invalid);
        }
        let source = &self.peers[&publisher];
        let target = &self.peers[&key];
        if source.channel != target.channel {
            return Err(Error::Forbidden);
        }
        let track = source
            .tracks
            .iter()
            .find(|t| t.mid == source_mid)
            .ok_or(Error::Invalid)?
            .clone();
        if track
            .codec
            .is_some_and(|c| !self.codec_allowed(source.channel, c))
        {
            return Err(Error::Invalid);
        }
        validation::layer(&source_mid, &receiver_mid, &track.layers, rid.as_deref())?;
        if target.routes.len() >= 32 && !target.routes.contains_key(&receiver_mid) {
            return Err(Error::Limit);
        }
        // Video to an audience is bounded by the server's video capacity: a
        // new seat only while every listener video route can get its minimum.
        if target.listener
            && is_video(track.kind)
            && !target.routes.get(&receiver_mid).is_some_and(|r| r.video)
        {
            let seats = (self.config.video_egress_bps / LISTENER_VIDEO_BPS).max(1) as usize;
            let used: usize = self
                .peers
                .values()
                .filter(|p| p.listener)
                .map(|p| p.routes.values().filter(|r| r.video).count())
                .sum();
            if used >= seats {
                return Err(Error::Limit);
            }
        }
        let source_mid: Mid = source_mid.as_str().into();
        let target_mid: Mid = receiver_mid.as_str().into();
        let video = is_video(track.kind);
        let m = target.rtc.media(target_mid).ok_or(Error::Invalid)?;
        if m.kind() != media_kind(track.kind)
            || m.disabled()
            || !matches!(m.direction(), Direction::SendOnly | Direction::SendRecv)
            || target.tracks.iter().any(|t| t.mid == receiver_mid)
        {
            return Err(Error::Invalid);
        }
        // Exact profile/format matching: never silently send High into a
        // baseline-only browser decoder, or HEVC into an AV1 slot.
        let b = source
            .rtc
            .codec_config()
            .params()
            .iter()
            .filter(|a| {
                a.spec().codec == codec_of(&track)
                    && source
                        .rtc
                        .media(source_mid)
                        .unwrap()
                        .remote_pts()
                        .contains(&a.pt())
            })
            .find_map(|a| self.params_exact(key, target_mid, *a))
            .ok_or(Error::Invalid)?;
        self.remove_route(key, &receiver_mid)?;
        // SDP creates a default TX SSRC. The pacer addresses streams by
        // MID/RID, so leaving that empty stream beside our rewritten SSRC
        // makes it pick an arbitrary stream and can starve actual media.
        loop {
            let old = self
                .peers
                .get_mut(&key)
                .unwrap()
                .rtc
                .direct_api()
                .stream_tx_by_mid(target_mid, None)
                .map(|s| s.ssrc());
            self.collect(key)?;
            let Some(old) = old else {
                break;
            };
            self.peers.get_mut(&key).unwrap().pending_media.remove(&old);
            self.peers
                .get_mut(&key)
                .unwrap()
                .rtc
                .direct_api()
                .remove_stream_tx(old);
            self.collect(key)?;
        }
        let rtc = &mut self.peers.get_mut(&key).unwrap().rtc;
        let ssrc = rtc.direct_api().new_ssrc();
        self.collect(key)?;
        let rtx = if video && b.resend().is_some() {
            let s = self
                .peers
                .get_mut(&key)
                .unwrap()
                .rtc
                .direct_api()
                .new_ssrc();
            self.collect(key)?;
            Some(s)
        } else {
            None
        };
        self.peers
            .get_mut(&key)
            .unwrap()
            .rtc
            .direct_api()
            .declare_stream_tx(ssrc, rtx, target_mid, None);
        self.collect(key)?;
        let rtc = &mut self.peers.get_mut(&key).unwrap().rtc;
        if let Some(stream) = rtc.direct_api().stream_tx(&ssrc) {
            stream.set_rtx_cache(
                if video { 256 } else { 32 },
                Duration::from_secs(1),
                Some(0.15),
            );
        }
        self.collect(key)?;
        let rid = rid.as_deref().map(Into::into);
        self.peers.get_mut(&key).unwrap().routes.insert(
            receiver_mid,
            Route {
                publisher,
                source: source_mid,
                target: target_mid,
                rid,
                ssrc,
                video,
                kind: track.kind,
                first_source_seq: None,
                waiting_keyframe: video,
                burst: FrameBurst::default(),
                keyframe_ends: BTreeMap::new(),
                rtx,
            },
        );
        if video {
            if self.video_trace {
                eprintln!("VIDEO_TRACE viewer_keyframe_request peer={key:?} publisher={publisher:?} fir=false reason=subscribe_or_reset");
            }
            self.keyframe(publisher, source_mid, rid, KeyframeRequestKind::Pli)?;
        }
        self.allocate()
    }
    fn remove_route(&mut self, key: VoiceSession, mid: &str) -> Result<(), Error> {
        let sharer = self
            .peers
            .get(&key)
            .and_then(|p| p.routes.get(mid))
            .map(|r| r.publisher);
        if let Some(sharer) = sharer {
            if self
                .peers
                .get(&sharer)
                .is_some_and(|p| p.grant.as_ref().is_some_and(|g| g.controller == key))
            {
                self.revoke(sharer)?;
            }
        }
        if let Some(p) = self.peers.get_mut(&key) {
            if let Some(r) = p.routes.remove(mid) {
                p.pending_media.remove(&r.ssrc);
                if !p.failed {
                    p.rtc.direct_api().remove_stream_tx(r.ssrc);
                    self.collect(key)?;
                }
            }
        }
        Ok(())
    }
    fn keyframe(
        &mut self,
        key: VoiceSession,
        mid: Mid,
        rid: Option<Rid>,
        kind: KeyframeRequestKind,
    ) -> Result<(), Error> {
        if self.video_trace {
            eprintln!("VIDEO_TRACE keyframe_request publisher={key:?} mid={mid} kind={kind:?}");
        }
        if self.authorized(key).is_err() {
            return Ok(());
        }
        if let Some(p) = self.peers.get_mut(&key) {
            let source = (mid, rid.map(|rid| rid.to_string()));
            if p.keyframe_times
                .get(&source)
                .is_some_and(|at| self.now.saturating_duration_since(*at) < Duration::from_secs(1))
            {
                p.keyframe_suppressed += 1;
                return Ok(());
            }
            if let Some(s) = p.rtc.direct_api().stream_rx_by_mid(mid, rid) {
                s.request_keyframe(kind);
                p.keyframe_times.insert(source, self.now);
                p.keyframe_forwarded += 1;
            }
            self.collect(key)?;
        }
        Ok(())
    }
    fn allocate(&mut self) -> Result<(), Error> {
        // Every video route: (viewer, receiver MID, publisher, source MID, room).
        let routes: Vec<(VoiceSession, String, VoiceSession, String, ChannelId)> = self
            .peers
            .iter()
            .flat_map(|(key, p)| {
                p.routes
                    .iter()
                    .filter(|(_, r)| r.video)
                    .map(move |(mid, r)| {
                        (
                            *key,
                            mid.clone(),
                            r.publisher,
                            r.source.to_string(),
                            p.channel,
                        )
                    })
            })
            .collect();
        // What each route's track needs, as wire rate: the payload limits below
        // keep 25% for RTP/SRTP headers, repair/probes and IDR bursts. Without a
        // declared need every route is equal, as it always was.
        let need: Vec<u64> = routes
            .iter()
            .map(|(_, _, publisher, source, _)| {
                self.peers
                    .get(publisher)
                    .and_then(|p| p.demands.get(source))
                    .map_or(u64::MAX, |bps| bps.saturating_mul(4) / 3)
            })
            .collect();
        // Max-min fair shares of the global and of each room's video budget.
        let global = crate::quality::waterfill(self.config.video_egress_bps, &need);
        let mut room = vec![0; routes.len()];
        // Routes grouped by room and by viewer in one pass each: a Stage can
        // hold many viewers, so nothing here may be routes x peers.
        let mut rooms: BTreeMap<ChannelId, Vec<usize>> = BTreeMap::new();
        let mut viewers: BTreeMap<VoiceSession, Vec<usize>> = BTreeMap::new();
        for (i, r) in routes.iter().enumerate() {
            rooms.entry(r.4).or_default().push(i);
            viewers.entry(r.0).or_default().push(i);
        }
        for members in rooms.into_values() {
            let caps: Vec<u64> = members.iter().map(|&i| need[i]).collect();
            for (k, share) in crate::quality::waterfill(self.config.room_video_egress_bps, &caps)
                .into_iter()
                .enumerate()
            {
                room[members[k]] = share;
            }
        }
        let cap: Vec<u64> = (0..routes.len()).map(|i| global[i].min(room[i])).collect();
        // Desired capacity must be explicit: str0m defaults it to zero,
        // otherwise an idle -> high-bitrate transition can stay pinned to
        // a tiny old estimate without probing toward the subscription budget.
        let targets: Vec<_> = self
            .peers
            .keys()
            .map(|key| {
                let sum = viewers.get(key).map_or(0, |mine| {
                    mine.iter().fold(0u64, |a, &i| a.saturating_add(cap[i]))
                });
                (*key, sum.min(self.config.peer_video_egress_bps))
            })
            .collect();
        for (key, desired) in targets {
            let p = self.peers.get_mut(&key).unwrap();
            let video_active = p.routes.values().any(|r| r.video);
            if video_active {
                // The configured peer ceiling is NOT an allocation. Otherwise
                // one viewer's over-estimated probes can spend the entire
                // global/room bucket before the next viewer gets a turn.
                p.video_budget
                    .set_rate(desired, self.config.max_datagram_bytes, self.now);
            }
            let starting = video_active && !p.video_active;
            p.video_active = video_active;
            if p.bwe_target != desired || starting {
                if starting {
                    // Do not let an audio-only estimate throttle a new video
                    // subscription before feedback for its first frame arrives.
                    let initial = desired.min(16_000_000);
                    p.rtc.bwe().reset(str0m::bwe::Bitrate::bps(initial));
                    p.bwe = initial;
                }
                p.bwe_target = desired;
                p.rtc.bwe().set_bitrate(
                    str0m::bwe::Bitrate::bps(0),
                    str0m::bwe::Bitrate::bps(desired),
                );
                self.collect(key)?;
            }
        }
        let mut limits: BTreeMap<(VoiceSession, String), u64> = BTreeMap::new();
        for (key, mine) in &viewers {
            let Some(p) = self.peers.get(key) else {
                continue;
            };
            // Each viewer's estimate, split fairly among what it watches. Two
            // 1440p60 viewers receive 12 Mbps payload each within the unchanged
            // 32 Mbps wire cap; a 2.5 Mbps camera leaves the rest to a 4K share.
            let caps: Vec<u64> = mine.iter().map(|&i| cap[i]).collect();
            let total = p.bwe.min(self.config.peer_video_egress_bps);
            for (k, allotted) in crate::quality::waterfill(total, &caps)
                .into_iter()
                .enumerate()
            {
                let r = &routes[mine[k]];
                let bps = allotted.saturating_mul(3) / 4;
                limits
                    .entry((r.2, r.3.clone()))
                    .and_modify(|b| *b = (*b).min(bps))
                    .or_insert(bps);
            }
        }
        for ((key, mid), bps) in limits {
            let old = self
                .peers
                .get_mut(&key)
                .unwrap()
                .limits
                .insert(mid.clone(), bps);
            if old != Some(bps) {
                self.send(
                    key,
                    0,
                    VoiceEvent::Bitrate {
                        mid: mid.clone(),
                        bps,
                    },
                )?;
            }
            let layers = self
                .peers
                .get(&key)
                .and_then(|p| p.tracks.iter().find(|t| t.mid == mid))
                .map(|t| t.layers.clone())
                .unwrap_or_default();
            let rids: Vec<Option<Rid>> = if layers.is_empty() {
                vec![None]
            } else {
                layers.iter().map(|s| Some(s.as_str().into())).collect()
            };
            for rid in rids {
                if let Some(p) = self.peers.get_mut(&key) {
                    if let Some(s) = p
                        .rtc
                        .direct_api()
                        .stream_rx_by_mid(mid.as_str().into(), rid)
                    {
                        s.request_remb(str0m::bwe::Bitrate::bps(bps));
                    }
                    self.collect(key)?;
                }
            }
        }
        Ok(())
    }
    /// Host-supplied Tailscale path telemetry (ICE cannot distinguish DERP).
    pub fn set_path(&mut self, key: VoiceSession, path: MediaPath) -> Result<(), Error> {
        self.peers.get_mut(&key).ok_or(Error::Forbidden)?.stats.path = path;
        Ok(())
    }
    /// Feed one UDP packet. Demultiplexing uses str0m's ICE/DTLS acceptance,
    /// NEVER trusts an unverified source address or user ID in the packet.
    pub fn handle_datagram(
        &mut self,
        from: SocketAddr,
        bytes: &[u8],
        now: Instant,
    ) -> Result<(), Error> {
        self.now = self.now.max(now);
        self.synchronize()?;
        let Ok(receive) = crate::boundary::run(|| {
            Receive::new(Protocol::Udp, from, self.local, bytes).map_err(|_| Error::Invalid)
        }) else {
            return Ok(());
        };
        let input = Input::Receive(self.now, receive);
        let mut key = None;
        // The peer that last sent from this address first: with a thousand
        // listeners, asking every peer about every packet would dominate.
        let hint = self
            .sources
            .get(&from)
            .copied()
            .filter(|s| self.peers.contains_key(s));
        if let Some(hint) = hint {
            if self.accepts(hint, &input)? {
                key = Some(hint);
            }
        }
        if key.is_none() {
            for candidate in self.sessions() {
                if Some(candidate) != hint && self.accepts(candidate, &input)? {
                    key = Some(candidate);
                    break;
                }
            }
        }
        if let Some(key) = key {
            self.remember_source(key, from);
            if self.authorized(key).is_err() {
                self.disconnect(key)?;
                return Ok(());
            }
            // Never accept fragmented/oversized media into forwarding queues.
            if is_rtp(bytes) && bytes.len() > usize::from(self.config.max_datagram_bytes) {
                self.peers.get_mut(&key).unwrap().stats.mtu_dropped_packets += 1;
                return Ok(());
            }
            let result = self.peer_operation(key, 0, |s| {
                s.peers
                    .get_mut(&key)
                    .ok_or(Error::Forbidden)?
                    .rtc
                    .handle_input(input)?;
                s.collect(key)
            });
            if matches!(result, Err(Error::Backpressure)) {
                return result;
            }
            self.pump()?;
        }
        Ok(())
    }
    pub fn handle_timeout(&mut self, now: Instant) -> Result<(), Error> {
        self.now = self.now.max(now);
        self.synchronize()?;
        let keys: Vec<_> = self.peers.keys().copied().collect();
        for key in keys {
            let Some(p) = self.peers.get(&key) else {
                continue;
            };
            let stale = !p.connected
                && self.now.saturating_duration_since(p.created) >= Duration::from_secs(30);
            // Only peers with something due: an idle listener costs nothing here.
            if !stale
                && p.deadline > self.now
                && p.request.is_none_or(|r| r.1 > self.now)
                && p.speaking_until.is_none_or(|t| t > self.now)
            {
                continue;
            }
            if stale || self.authorized(key).is_err() {
                self.disconnect(key)?;
                continue;
            }
            if self.peers[&key].request.is_some_and(|r| r.1 <= self.now) {
                self.cancel_request(key)?;
            }
            if self.peers[&key]
                .speaking_until
                .is_some_and(|t| t <= self.now)
            {
                self.stop_speaking(key)?;
            }
            if self.peers[&key].deadline <= self.now {
                let result = self.peer_operation(key, 0, |s| {
                    s.peers
                        .get_mut(&key)
                        .ok_or(Error::Forbidden)?
                        .rtc
                        .handle_input(Input::Timeout(s.now))?;
                    s.collect(key)?;
                    if s.peers.get(&key).is_some_and(|p| !p.rtc.is_alive()) {
                        s.disconnect(key)?;
                    }
                    Ok(())
                });
                if matches!(result, Err(Error::Backpressure)) {
                    return result;
                }
            }
        }
        self.pump()
    }
    fn collect(&mut self, key: VoiceSession) -> Result<(), Error> {
        self.peer_operation(key, 0, |s| s.collect_inner(key))
    }
    fn collect_inner(&mut self, key: VoiceSession) -> Result<(), Error> {
        loop {
            let generation = self.acl.generation();
            let Some(peer) = self.peers.get_mut(&key) else {
                return Ok(());
            };
            if peer.failed {
                return Ok(());
            }
            if peer.video_active {
                let bytes = usize::from(self.config.max_datagram_bytes);
                let room = self.room_budgets.get(&peer.channel).ok_or(Error::Config)?;
                let ready = peer
                    .video_budget
                    .ready_at(bytes, self.now)
                    .max(room.ready_at(bytes, self.now))
                    .max(self.budget.ready_at(bytes, self.now));
                if let Some(ready) = bounded_admission_deadline(ready, self.now) {
                    // Preflight probes/RTX too: dropping an encrypted probe
                    // invents TWCC loss after assigning its sequence number.
                    // str0m still prioritizes originals. The same strict wire
                    // caps and <=10 ms RTC deferral apply to every class.
                    peer.deadline = ready;
                    break;
                }
            }
            match peer.rtc.poll_output()? {
                Output::Timeout(deadline) => {
                    peer.deadline = deadline;
                    break;
                }
                Output::Event(event) => {
                    if self.events.len() >= 4096 {
                        return Err(Error::Backpressure);
                    }
                    self.events.push_back((key, event));
                }
                Output::Transmit(tx) => {
                    let bytes: &[u8] = tx.contents.as_ref();
                    if is_rtp(bytes) {
                        if !self.acl.can_join(key, peer.channel) {
                            continue;
                        }
                        if bytes.len() > usize::from(self.config.max_datagram_bytes) {
                            peer.stats.mtu_dropped_packets += 1;
                            continue;
                        }
                        let ssrc = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
                        // Recheck sender and receiver policy at encrypted egress (including RTX/pacer queues).
                        let route = peer
                            .routes
                            .values_mut()
                            .find(|r| *r.ssrc == ssrc || r.rtx.is_some_and(|s| *s == ssrc));
                        let Some(route) = route else {
                            continue;
                        };
                        if !self
                            .acl
                            .permissions(route.publisher, peer.channel)
                            .contains(track_permission(route.kind))
                            || (!route.video && !self.acl.send_audio(route.publisher))
                            || (!route.video && (peer.deafened || !self.acl.receive_audio(key)))
                        {
                            continue;
                        }
                        // Unmapped/withdrawn SSRCs (including probes) cannot escape
                        // publication authorization via a paced or retransmit queue.
                        let video = route.video;
                        // Padding/RTX share the wire cap, but not the original
                        // media backlog. Never discard at an artificial reserve
                        // after str0m has assigned a transport sequence number.
                        let repair = route.rtx.is_some_and(|r| *r == ssrc) || bytes[0] & 0x20 != 0;
                        let mut sent_keyframe = None;
                        if !repair {
                            let seq = u16::from_be_bytes([bytes[2], bytes[3]]);
                            sent_keyframe = route.keyframe_ends.remove(&seq);
                            if let Some(pending) = peer.pending_media.get_mut(&route.ssrc) {
                                if let Some(index) = pending.iter().position(|p| p.0 == seq) {
                                    pending.remove(index);
                                }
                            }
                        }
                        if video {
                            let room = self
                                .room_budgets
                                .get_mut(&peer.channel)
                                .ok_or(Error::Config)?;
                            if !peer.video_budget.available(bytes.len(), self.now)
                                || !room.available(bytes.len(), self.now)
                                || !self.budget.available(bytes.len(), self.now)
                            {
                                peer.stats.cap_dropped_packets += 1;
                                if !repair {
                                    peer.media_cap_drops += 1;
                                }
                                continue;
                            }
                            // Commit all three only after every preflight succeeds.
                            peer.video_budget.take(bytes.len(), self.now);
                            room.take(bytes.len(), self.now);
                            self.budget.take(bytes.len(), self.now);
                        }
                        if self.video_trace {
                            if let Some((timestamp, size)) = sent_keyframe {
                                eprintln!("VIDEO_TRACE keyframe_sent peer={key:?} seconds={:.3} timestamp={timestamp} bytes={size}", self.now.saturating_duration_since(peer.created).as_secs_f64());
                            }
                        }
                        if video && bytes[1] & 0x80 != 0 {
                            peer.tx_frames += 1;
                        }
                    }
                    peer.wire_tx_bytes += bytes.len() as u64;
                    self.emit(Out::Datagram {
                        to: tx.destination,
                        bytes: bytes.to_vec(),
                    })?;
                    if let Some((stamp, _)) = self.out.back_mut() {
                        *stamp = generation;
                    }
                }
            }
        }
        Ok(())
    }
    fn pump(&mut self) -> Result<(), Error> {
        while let Some((key, event)) = self.events.pop_front() {
            if !self.peers.contains_key(&key) {
                continue;
            }
            let result = self.peer_operation(key, 0, |s| s.peer_event(key, event));
            if matches!(result, Err(Error::Backpressure)) {
                return result;
            }
            if let Err(error) = result {
                if !matches!(error, Error::PeerFailed | Error::Rtc(_)) {
                    self.send(
                        key,
                        0,
                        VoiceEvent::Error {
                            code: error.code(),
                            message: error.to_string(),
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
    fn peer_event(&mut self, key: VoiceSession, event: Event) -> Result<(), Error> {
        match event {
            Event::Connected => {
                self.peers.get_mut(&key).unwrap().connected = true;
            }
            Event::Closed => {
                self.disconnect(key)?;
            }
            Event::RtpPacket(packet) => self.forward(key, packet)?,
            Event::KeyframeRequest(req) => {
                if let Some(route) = self.peers[&key]
                    .routes
                    .values()
                    .find(|r| r.target == req.mid)
                    .cloned()
                {
                    if self.video_trace {
                        eprintln!("VIDEO_TRACE viewer_keyframe_request peer={key:?} publisher={:?} kind={:?} reason=rtcp", route.publisher, req.kind);
                    }
                    self.keyframe(route.publisher, route.source, route.rid, req.kind)?;
                }
            }
            Event::EgressBitrateEstimate(estimate) => {
                if self.video_trace {
                    eprintln!(
                        "VIDEO_TRACE sfu_bwe peer={key:?} seconds={:.3} estimate={estimate:?}",
                        self.now
                            .saturating_duration_since(self.peers[&key].created)
                            .as_secs_f64()
                    );
                }
                let (BweKind::Twcc { estimate, .. } | BweKind::Remb { estimate, .. }) = estimate
                else {
                    return Ok(());
                };
                let p = self.peers.get_mut(&key).unwrap();
                p.bwe = estimate.as_u64();
                // An estimate only changes the split of what this peer watches.
                if p.routes.values().any(|r| r.video) {
                    self.allocate()?;
                }
            }
            Event::PeerStats(stats) => self.stats(key, stats)?,
            // Refresh REMB after a newly started stream becomes known.
            Event::MediaIngressStats(stats) => {
                if self.video_trace {
                    eprintln!("VIDEO_TRACE sfu_in peer={:?} mid={} seconds={:.3} bytes={} packets={} nack={} pli={} fir={}", key, stats.mid, self.now.saturating_duration_since(self.peers[&key].created).as_secs_f64(), stats.bytes, stats.packets, stats.nacks, stats.plis, stats.firs);
                }
                self.allocate()?;
            }
            Event::MediaEgressStats(stats) => {
                if self.video_trace {
                    eprintln!("VIDEO_TRACE sfu_out peer={:?} mid={} seconds={:.3} bytes={} packets={} nack={} pli={} fir={}", key, stats.mid, self.now.saturating_duration_since(self.peers[&key].created).as_secs_f64(), stats.bytes, stats.packets, stats.nacks, stats.plis, stats.firs);
                }
            }
            Event::ChannelOpen(id, label) => self.channel_open(key, id, &label)?,
            Event::ChannelClose(id) if self.peers[&key].control == Some(id) => {
                self.peers.get_mut(&key).unwrap().control = None;
                self.revoke(key)?;
                let sharers: Vec<_> = self
                    .peers
                    .iter()
                    .filter(|(_, p)| p.grant.as_ref().is_some_and(|g| g.controller == key))
                    .map(|(k, _)| *k)
                    .collect();
                for sharer in sharers {
                    self.revoke(sharer)?;
                }
            }
            Event::ChannelData(data) => self.control_data(key, data)?,
            _ => {}
        }
        Ok(())
    }
    fn forward(&mut self, key: VoiceSession, packet: RtpPacket) -> Result<(), Error> {
        if self.authorized(key).is_err() {
            return self.disconnect(key);
        }
        let stream = self
            .peers
            .get_mut(&key)
            .unwrap()
            .rtc
            .direct_api()
            .stream_rx(&packet.header.ssrc)
            .map(|s| (s.mid(), s.rid()));
        self.collect(key)?;
        let Some((mid, rid)) = stream else {
            return Ok(());
        };
        let Some(track) = self.peers[&key]
            .tracks
            .iter()
            .find(|t| t.mid == mid.to_string())
            .cloned()
        else {
            return Ok(());
        };
        let video = is_video(track.kind);
        if video && starts_keyframe(codec_of(&track), &packet.payload) {
            self.peers.get_mut(&key).unwrap().idr_starts += 1;
        }
        let p = &self.peers[&key];
        if p.listener
            || !self
                .acl
                .permissions(key, p.channel)
                .contains(track_permission(track.kind))
            || (!video && !self.acl.send_audio(key))
            || (!video && (p.muted || p.deafened))
            || track
                .codec
                .is_some_and(|c| !self.codec_allowed(p.channel, c))
        {
            return Ok(());
        }
        let Some(source_param) = p
            .rtc
            .codec_config()
            .params()
            .iter()
            .find(|param| param.pt() == packet.header.payload_type)
            .copied()
        else {
            return Ok(());
        };
        if source_param.spec().codec != codec_of(&track) {
            return Ok(());
        }
        // Admission is based on the actual authenticated UDP datagram, not an
        // arbitrary compressed-payload cutoff. Rewritten RTP/RTX is checked again
        // at the final encrypted egress boundary.
        if video && packet.header.marker {
            self.peers.get_mut(&key).unwrap().rx_frames += 1;
        }
        if track.kind == TrackKind::Audio {
            if let Some(level) = packet.header.ext_vals.audio_level {
                let speaking =
                    packet.header.ext_vals.voice_activity.unwrap_or(level > -50) && level > -60;
                let was_speaking = self.peers[&key].speaking_until.is_some();
                if speaking {
                    self.peers.get_mut(&key).unwrap().speaking_until =
                        Some(self.now + Duration::from_millis(300));
                }
                if speaking && !was_speaking {
                    self.broadcast(
                        self.peers[&key].channel,
                        VoiceEvent::Speaking {
                            session: key,
                            speaking: true,
                            level_dbov: level,
                        },
                    )?;
                }
            }
        }
        // The voice channel's bitrate is enforced here, not only requested of
        // the client's encoder: far-over-limit microphone audio is not sent on.
        if track.kind == TrackKind::Audio {
            if let Some(bps) = self.acl.audio_bitrate(key) {
                let now = self.now;
                let policer = self
                    .peers
                    .get_mut(&key)
                    .unwrap()
                    .audio_policer
                    .get_or_insert_with(|| crate::quality::AudioPolicer::new(bps, now));
                if policer.bps() != bps {
                    policer.set(bps);
                }
                if !policer.admit(packet.payload.len(), now) {
                    return Ok(());
                }
            }
        }
        let destinations: Vec<_> = self
            .peers
            .iter()
            .filter(|(s, p)| {
                self.acl.can_join(**s, p.channel)
                    && (!(!video && (p.deafened || !self.acl.receive_audio(**s))))
            })
            .flat_map(|(s, p)| {
                p.routes
                    .iter()
                    .filter(|(_, r)| r.publisher == key && r.source == mid && r.rid == rid)
                    .map(|(target, r)| (*s, target.clone(), r.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (to, target, route) in destinations {
            let Some(param) = self.params_exact(to, route.target, source_param) else {
                continue;
            };
            // queue_info includes synthetic probe padding and RTX, NOT just
            // unsent media. A large probe must never reset a healthy H.264
            // reference chain. Track original media until encrypted egress.
            self.collect(to)?;
            let keyframe = starts_keyframe(codec_of(&track), &packet.payload);
            let receiver = self.peers.get_mut(&to).unwrap();
            let videos = receiver.routes.values().filter(|r| r.video).count() as u64;
            let drain_bps = receiver.bwe.min(receiver.bwe_target) / videos.max(1);
            let r = receiver.routes.get_mut(&target).unwrap();
            if video {
                r.burst
                    .observe(packet.header.timestamp, keyframe, packet.payload.len());
            }
            let keyframe_bytes = r.burst.keyframe_bytes;
            if self.video_trace && video && packet.header.marker && r.burst.keyframe {
                eprintln!("VIDEO_TRACE keyframe_queued peer={to:?} seconds={:.3} timestamp={} bytes={keyframe_bytes}", self.now.saturating_duration_since(receiver.created).as_secs_f64(), packet.header.timestamp);
            }
            let pending = receiver.pending_media.entry(route.ssrc).or_default();
            let pending_count = pending.len();
            let overloaded = video && media_queue_overloaded(pending, drain_bps, self.now);
            if self.video_trace
                && video
                && self.now.saturating_duration_since(receiver.created) <= Duration::from_secs(15)
                && (overloaded
                    || self.now.saturating_duration_since(receiver.queue_trace_at)
                        >= Duration::from_millis(100))
            {
                receiver.queue_trace_at = self.now;
                let byte_limit = media_queue_byte_limit(drain_bps);
                eprintln!("VIDEO_TRACE startup_queue peer={to:?} seconds={:.3} packets={pending_count} bytes={} age_ms={:.3} resets={} bwe={} allocation={} keyframe_bytes={keyframe_bytes} byte_limit={byte_limit} pacing_budget_bytes={} pacing_ready_us={}",
                    self.now.saturating_duration_since(receiver.created).as_secs_f64(), pending.iter().map(|p| p.2).sum::<usize>(),
                    pending.front().map_or(0., |p| self.now.saturating_duration_since(p.1).as_secs_f64()*1000.),
                    receiver.queue_resets, receiver.bwe, receiver.bwe_target,
                    receiver.video_budget.available_bytes(self.now),
                    receiver.video_budget.ready_at(usize::from(self.config.max_datagram_bytes), self.now).saturating_duration_since(self.now).as_micros());
            }
            if overloaded {
                receiver.queue_resets += 1;
                if self.video_trace {
                    eprintln!("VIDEO_TRACE original_queue_reset peer={to:?} packets={pending_count} bytes={} age_ms={:.3} bwe={} rtt_ms={:?}",
                        pending.iter().map(|p| p.2).sum::<usize>(),
                        pending.front().map_or(0., |p| self.now.saturating_duration_since(p.1).as_secs_f64() * 1000.),
                        receiver.bwe, receiver.stats.rtt_ms);
                }
                self.peers.get_mut(&to).unwrap().stats.queue_dropped_packets +=
                    pending_count as u64 + 1;
                // Discard the stale queue/cache and restart on an IDR with a
                // fresh SSRC, rather than building latency indefinitely.
                self.subscribe(
                    to,
                    route.publisher,
                    route.source.to_string(),
                    target,
                    route.rid.map(|r| r.to_string()),
                )?;
                continue;
            }
            let config_packet = track.codec == Some(VideoCodec::H264)
                && matches!(packet.payload.first().map(|b| b & 31), Some(7 | 8));
            if route.waiting_keyframe && !keyframe && !config_packet {
                self.peers.get_mut(&to).unwrap().gated_packets += 1;
                continue;
            }
            let peer = self.peers.get_mut(&to).unwrap();
            let r = peer.routes.get_mut(&target).unwrap();
            if keyframe {
                r.waiting_keyframe = false;
            }
            let source_seq = *packet.seq_no;
            let first = *r.first_source_seq.get_or_insert(source_seq);
            if source_seq < first {
                continue;
            }
            let sequence = source_seq - first + 1;
            if self.video_trace && video && packet.header.marker && r.burst.keyframe {
                r.keyframe_ends
                    .insert(sequence as u16, (packet.header.timestamp, r.burst.bytes));
            }
            // Per-hop MID/SSRC/TWCC are regenerated, never copied upstream.
            let ext = ExtensionValues {
                audio_level: packet.header.ext_vals.audio_level,
                voice_activity: packet.header.ext_vals.voice_activity,
                video_orientation: packet.header.ext_vals.video_orientation,
                ..Default::default()
            };
            if let Some(stream) = peer.rtc.direct_api().stream_tx(&route.ssrc) {
                if video {
                    peer.pending_media
                        .entry(route.ssrc)
                        .or_default()
                        .push_back((sequence as u16, self.now, packet.payload.len()));
                }
                stream.write_rtp(
                    RtpWrite::new(
                        param.pt(),
                        sequence.into(),
                        packet.header.timestamp,
                        self.now,
                        packet.payload.clone(),
                    )
                    .marker(packet.header.marker)
                    .ext_vals(ext)
                    .nackable(video),
                );
            }
            self.collect(to)?;
            self.peers
                .get_mut(&to)
                .unwrap()
                .rtc
                .handle_input(Input::Timeout(self.now))?;
            self.collect(to)?;
        }
        Ok(())
    }
    fn params_exact(
        &self,
        key: VoiceSession,
        mid: Mid,
        source: PayloadParams,
    ) -> Option<PayloadParams> {
        let p = &self.peers[&key];
        let media = p.rtc.media(mid)?;
        p.rtc
            .codec_config()
            .params()
            .iter()
            .find(|param| media.remote_pts().contains(&param.pt()) && param.spec() == source.spec())
            .copied()
    }
    fn stop_speaking(&mut self, key: VoiceSession) -> Result<(), Error> {
        let Some(p) = self.peers.get_mut(&key) else {
            return Ok(());
        };
        let channel = p.channel;
        if p.speaking_until.take().is_some() {
            self.broadcast(
                channel,
                VoiceEvent::Speaking {
                    session: key,
                    speaking: false,
                    level_dbov: -127,
                },
            )?;
        }
        Ok(())
    }
    fn stats(&mut self, key: VoiceSession, stats: str0m::stats::PeerStats) -> Result<(), Error> {
        let p = self.peers.get_mut(&key).unwrap();
        let elapsed = stats
            .timestamp
            .saturating_duration_since(p.stats_at)
            .as_secs_f64()
            .max(0.001);
        if self.video_trace {
            eprintln!("VIDEO_TRACE sfu_peer peer={:?} seconds={:.3} bwe={} target={} rx_fps={:.1} tx_fps={:.1} wire_tx_bytes={} idr_starts={} gated_packets={} cap_drops={} media_cap_drops={} queue_drops={} mtu_drops={}", key, self.now.saturating_duration_since(p.created).as_secs_f64(), p.bwe, p.bwe_target, p.rx_frames as f64 / elapsed, p.tx_frames as f64 / elapsed, p.wire_tx_bytes, p.idr_starts, p.gated_packets, p.stats.cap_dropped_packets, p.media_cap_drops, p.stats.queue_dropped_packets, p.stats.mtu_dropped_packets);
        }
        p.stats.tx_bps =
            ((p.wire_tx_bytes.saturating_sub(p.tx_bytes)) as f64 * 8.0 / elapsed) as u64;
        p.stats.rx_bps =
            ((stats.peer_bytes_rx.saturating_sub(p.rx_bytes)) as f64 * 8.0 / elapsed) as u64;
        p.stats.tx_fps_milli = (p.tx_frames as f64 * 1000.0 / elapsed) as u32;
        p.stats.rx_fps_milli = (p.rx_frames as f64 * 1000.0 / elapsed) as u32;
        p.stats.loss_per_million = stats
            .egress_loss_fraction
            .or(stats.ingress_loss_fraction)
            .map(|v| (v.clamp(0.0, 1.0) * 1_000_000.0) as u32);
        p.stats.rtt_ms = stats
            .rtt
            .or_else(|| {
                stats
                    .selected_candidate_pair
                    .as_ref()
                    .and_then(|p| p.current_round_trip_time)
            })
            .map(|rtt| rtt.as_millis().min(u32::MAX as u128) as u32);
        p.tx_bytes = p.wire_tx_bytes;
        p.rx_bytes = stats.peer_bytes_rx;
        p.tx_frames = 0;
        p.rx_frames = 0;
        p.stats_at = stats.timestamp;
        if self.video_trace {
            eprintln!(
                "VIDEO_TRACE keyframe_requests peer={key:?} forwarded={} suppressed={}",
                p.keyframe_forwarded, p.keyframe_suppressed
            );
        }
        let event = VoiceEvent::Stats(p.stats.clone());
        self.send(key, 0, event)
    }
    fn channel_open(
        &mut self,
        key: VoiceSession,
        id: DataChannelId,
        label: &str,
    ) -> Result<(), Error> {
        let p = self.peers.get_mut(&key).unwrap();
        let valid = label == CONTROL_CHANNEL_LABEL
            && p.control.is_none()
            && p.rtc
                .channel(id)
                .and_then(|c| c.config().cloned())
                .is_some_and(|c| {
                    c.ordered && c.reliability == str0m::channel::Reliability::Reliable
                });
        if valid {
            p.control = Some(id);
        } else {
            p.rtc.direct_api().close_data_channel(id);
        }
        self.collect(key)
    }
    fn screen_active(&self, key: VoiceSession) -> bool {
        self.peers.get(&key).is_some_and(|p| {
            self.acl
                .permissions(key, p.channel)
                .contains(Permissions::STREAM)
                && p.tracks
                    .iter()
                    .any(|t| t.kind == TrackKind::Screen && t.surface.is_some())
        })
    }
    fn viewing_screen(&self, viewer: VoiceSession, sharer: VoiceSession) -> bool {
        self.peers.get(&viewer).is_some_and(|p| {
            p.routes.values().any(|r| {
                r.publisher == sharer
                    && self.peers.get(&sharer).is_some_and(|s| {
                        s.tracks
                            .iter()
                            .any(|t| t.mid == r.source.to_string() && t.kind == TrackKind::Screen)
                    })
            })
        })
    }
    fn request_control(
        &mut self,
        requester: VoiceSession,
        sharer: VoiceSession,
    ) -> Result<(), Error> {
        self.authorized(sharer)?;
        if requester == sharer
            || self.peers[&requester].channel != self.peers[&sharer].channel
            || !self.viewing_screen(requester, sharer)
            || !self.screen_active(sharer)
            || self.peers[&sharer].grant.is_some()
            || self.peers[&sharer].request.is_some()
            || self.peers[&requester].control.is_none()
            || self.peers[&sharer].control.is_none()
        {
            return Err(Error::Forbidden);
        }
        self.peers.get_mut(&sharer).unwrap().request =
            Some((requester, self.now + Duration::from_secs(30)));
        self.send(
            sharer,
            0,
            VoiceEvent::ControlRequested {
                requester,
                timeout_ms: 30_000,
            },
        )
    }
    fn decide_control(
        &mut self,
        sharer: VoiceSession,
        requester: VoiceSession,
        allow: bool,
    ) -> Result<(), Error> {
        self.authorized(requester)?;
        let request = self.peers[&sharer].request.ok_or(Error::Forbidden)?;
        if request.0 != requester
            || request.1 <= self.now
            || !self.viewing_screen(requester, sharer)
            || !self.screen_active(sharer)
            || self.peers[&sharer].grant.is_some()
            || self.peers[&requester].channel != self.peers[&sharer].channel
            || self.peers[&sharer].control.is_none()
            || self.peers[&requester].control.is_none()
        {
            return Err(Error::Forbidden);
        }
        self.peers.get_mut(&sharer).unwrap().request = None;
        if !allow {
            return self.send(requester, 0, VoiceEvent::ControlDenied { sharer });
        }
        let grant = Grant::new(requester, self.now);
        let id = grant.id;
        self.peers.get_mut(&sharer).unwrap().grant = Some(grant);
        tracing::info!(?sharer, ?requester, grant = id, "remote control granted");
        self.broadcast(
            self.peers[&sharer].channel,
            VoiceEvent::ControlGranted {
                sharer,
                controller: requester,
                grant: id,
            },
        )
    }
    fn cancel_request(&mut self, sharer: VoiceSession) -> Result<(), Error> {
        if let Some((requester, _)) = self.peers.get_mut(&sharer).and_then(|p| p.request.take()) {
            self.send(requester, 0, VoiceEvent::ControlDenied { sharer })?;
        }
        Ok(())
    }
    fn write_control(&mut self, to: VoiceSession, message: &ControlData) -> Result<bool, Error> {
        let data = rmp_serde::to_vec_named(message).map_err(|_| Error::Invalid)?;
        let p = self.peers.get_mut(&to).ok_or(Error::Forbidden)?;
        let Some(id) = p.control else {
            return Ok(false);
        };
        let accepted = if let Some(mut ch) = p.rtc.channel(id) {
            if ch.buffered_amount() > 8192 {
                false
            } else {
                ch.write(true, &data).unwrap_or(false)
            }
        } else {
            false
        };
        self.collect(to)?;
        Ok(accepted)
    }
    fn revoke(&mut self, sharer: VoiceSession) -> Result<(), Error> {
        let Some(grant) = self.peers.get_mut(&sharer).and_then(|p| p.grant.take()) else {
            return Ok(());
        };
        tracing::info!(?sharer, grant = grant.id, "remote control revoked");
        let _ = self.write_control(sharer, &ControlData::ReleaseAll { grant: grant.id })?;
        // WS revoke is the fallback if SCTP is blocked/closed. Client releases
        // on either transport, local revoke and WS disconnect, idempotently.
        self.broadcast(
            self.peers[&sharer].channel,
            VoiceEvent::ControlRevoked {
                sharer,
                grant: grant.id,
            },
        )
    }
    fn control_data(
        &mut self,
        sender: VoiceSession,
        data: str0m::channel::ChannelData,
    ) -> Result<(), Error> {
        if self.authorized(sender).is_err() || self.peers[&sender].control != Some(data.id) {
            return Err(Error::Forbidden);
        }
        let parsed = if data.binary {
            validation::control(&data.data)
        } else {
            Err(Error::Invalid)
        };
        let message = match parsed {
            Ok(message) => message,
            Err(error) => {
                let sharers: Vec<_> = self
                    .peers
                    .iter()
                    .filter(|(_, p)| p.grant.as_ref().is_some_and(|g| g.controller == sender))
                    .map(|(s, _)| *s)
                    .collect();
                for sharer in sharers {
                    self.revoke(sharer)?;
                }
                return Err(error);
            }
        };
        let ControlData::Input {
            sharer,
            grant,
            sequence,
            ref input,
        } = message
        else {
            unreachable!()
        };
        if self.authorized(sharer).is_err()
            || self.peers[&sharer].channel != self.peers[&sender].channel
            || !self.screen_active(sharer)
        {
            return Err(Error::Forbidden);
        }
        let matching = self.peers[&sharer]
            .grant
            .as_ref()
            .is_some_and(|g| g.controller == sender && g.id == grant);
        if !matching {
            return Err(Error::Forbidden);
        }
        let accepted = self
            .peers
            .get_mut(&sharer)
            .unwrap()
            .grant
            .as_mut()
            .unwrap()
            .accept(sender, grant, sequence, input, self.now);
        // Never silently drop a key-up while leaving control active.
        if accepted.is_err() || !self.write_control(sharer, &message)? {
            self.revoke(sharer)?;
            return Err(accepted.err().unwrap_or(Error::PeerFailed));
        }
        Ok(())
    }
    /// Apply host policy immediately; withdrawing a screen also revokes remote control.
    pub fn refresh_policy(&mut self, key: VoiceSession) -> Result<(), Error> {
        match self.peer_operation(key, 0, |s| s.refresh_policy_inner(key)) {
            Err(Error::PeerFailed | Error::Rtc(_)) => Ok(()),
            result => result,
        }
    }
    fn refresh_policy_inner(&mut self, key: VoiceSession) -> Result<(), Error> {
        let Some(peer) = self.peers.get(&key) else {
            return Ok(());
        };
        let channel = peer.channel;
        let permissions = self.acl.permissions(key, channel);
        if !self.acl.can_join(key, channel) {
            return self.disconnect(key);
        }
        let was_listener = peer.listener;
        let mut listener = self.acl.listener(key);
        if was_listener && !listener && !self.room_has_space(channel, false, false) {
            // The host checks room before promoting; this is the hard cap.
            listener = true;
            self.send(
                key,
                0,
                VoiceEvent::Error {
                    code: ErrorCode::LimitExceeded,
                    message: "There is no room left to speak.".into(),
                },
            )?;
        }
        let peer = &self.peers[&key];
        let removed: Vec<_> = peer
            .tracks
            .iter()
            .filter(|t| listener || !permissions.contains(track_permission(t.kind)))
            .map(|t| t.mid.clone())
            .collect();
        if listener || !permissions.contains(Permissions::STREAM) {
            self.revoke(key)?;
            self.cancel_request(key)?;
        }
        self.peers
            .get_mut(&key)
            .unwrap()
            .tracks
            .retain(|t| !removed.contains(&t.mid));
        let stale: Vec<_> = self
            .peers
            .iter()
            .flat_map(|(s, p)| {
                p.routes
                    .iter()
                    .filter(|(_, r)| r.publisher == key && removed.contains(&r.source.to_string()))
                    .map(|(mid, _)| (*s, mid.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (s, mid) in stale {
            self.remove_route(s, &mid)?;
        }
        self.stop_speaking(key)?;
        self.peers.get_mut(&key).unwrap().listener = listener;
        if listener {
            // Back to the audience: everyone else forgets this session.
            if !was_listener {
                self.broadcast_others(channel, key, VoiceEvent::Left { session: key })?;
            }
        } else {
            self.broadcast(
                channel,
                VoiceEvent::Participant(self.peers[&key].participant(key)),
            )?;
        }
        self.allocate()
    }
    /// Move the established transport; discard all old-room subscriptions and grants.
    pub fn move_session(&mut self, key: VoiceSession, channel: ChannelId) -> Result<(), Error> {
        self.peer_operation(key, 0, |s| s.move_session_inner(key, channel))
    }
    fn move_session_inner(&mut self, key: VoiceSession, channel: ChannelId) -> Result<(), Error> {
        let old = self.peers.get(&key).ok_or(Error::Forbidden)?.channel;
        if old == channel {
            return Ok(());
        }
        if !self.acl.can_join(key, channel) {
            return Err(Error::Forbidden);
        }
        if !self.room_has_space(channel, self.acl.listener(key), false) {
            return Err(Error::Limit);
        }
        let tracks: usize = self
            .peers
            .values()
            .filter(|p| p.channel == channel)
            .map(|p| p.tracks.len())
            .sum();
        let candidates: usize = self
            .peers
            .values()
            .filter(|p| p.channel == channel)
            .map(|p| p.candidates.len())
            .sum();
        if tracks + self.peers[&key].tracks.len() > self.config.max_room_tracks
            || candidates + self.peers[&key].candidates.len() > self.config.max_room_ice_candidates
        {
            return Err(Error::Limit);
        }
        self.revoke(key)?;
        self.cancel_request(key)?;
        let stale: Vec<_> = self
            .peers
            .iter()
            .flat_map(|(s, p)| {
                p.routes
                    .iter()
                    .filter(|(_, r)| *s == key || r.publisher == key)
                    .map(|(mid, _)| (*s, mid.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (s, mid) in stale {
            self.remove_route(s, &mid)?;
        }
        let grants: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, p)| p.grant.as_ref().is_some_and(|g| g.controller == key))
            .map(|(s, _)| *s)
            .collect();
        for sharer in grants {
            self.revoke(sharer)?;
        }
        let requests: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, p)| p.request.is_some_and(|r| r.0 == key))
            .map(|(s, _)| *s)
            .collect();
        for sharer in requests {
            self.cancel_request(sharer)?;
        }
        self.peers.get_mut(&key).unwrap().channel = channel;
        self.room_budgets.entry(channel).or_insert_with(|| {
            Budget::new(
                self.config.room_video_egress_bps,
                self.config.max_datagram_bytes,
                self.now,
            )
        });
        self.room_budgets
            .retain(|ch, _| self.peers.values().any(|p| p.channel == *ch));
        self.room_negotiations
            .retain(|ch, _| self.peers.values().any(|p| p.channel == *ch));
        self.listener_negotiations
            .retain(|ch, _| self.peers.values().any(|p| p.channel == *ch));
        if !self.peers[&key].listener {
            self.broadcast(old, VoiceEvent::Left { session: key })?;
        }
        self.refresh_policy(key)?;
        let participants = self
            .peers
            .iter()
            .filter(|(s, p)| **s != key && p.channel == channel && !p.listener)
            .map(|(s, p)| p.participant(*s))
            .collect();
        self.send(
            key,
            0,
            VoiceEvent::Joined {
                channel_id: channel,
                participants,
                max_rtp_bytes: self.config.max_datagram_bytes,
            },
        )
    }
    /// Call on WS disconnect, logout, kick or ACL change; idempotent.
    pub fn disconnect(&mut self, key: VoiceSession) -> Result<(), Error> {
        match self.peer_operation(key, 0, |s| s.disconnect_inner(key)) {
            Err(Error::PeerFailed | Error::Rtc(_)) => Ok(()),
            result => result,
        }
    }
    fn disconnect_inner(&mut self, key: VoiceSession) -> Result<(), Error> {
        if !self.peers.contains_key(&key) {
            return Ok(());
        }
        self.revoke(key)?;
        self.cancel_request(key)?;
        let sharers: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, p)| p.grant.as_ref().is_some_and(|g| g.controller == key))
            .map(|(s, _)| *s)
            .collect();
        for sharer in sharers {
            self.revoke(sharer)?;
        }
        let requests: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, p)| p.request.is_some_and(|r| r.0 == key))
            .map(|(s, _)| *s)
            .collect();
        for sharer in requests {
            self.cancel_request(sharer)?;
        }
        let routes: Vec<_> = self
            .peers
            .iter()
            .flat_map(|(s, p)| {
                p.routes
                    .iter()
                    .filter(|(_, r)| r.publisher == key)
                    .map(|(mid, _)| (*s, mid.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (s, mid) in routes {
            self.remove_route(s, &mid)?;
        }
        if !self.peers[&key].failed {
            self.peers.get_mut(&key).unwrap().rtc.disconnect();
            self.collect(key)?;
        }
        let p = self.peers.remove(&key).unwrap();
        for a in &p.addresses {
            if self.sources.get(a) == Some(&key) {
                self.sources.remove(a);
            }
        }
        if !self.peers.values().any(|peer| peer.channel == p.channel) {
            self.room_budgets.remove(&p.channel);
            self.room_negotiations.remove(&p.channel);
            self.listener_negotiations.remove(&p.channel);
        }
        self.signals.remove(&key);
        self.events.retain(|(s, _)| *s != key);
        // Nobody else knew a listener was there.
        if !p.listener {
            self.broadcast(p.channel, VoiceEvent::Left { session: key })?;
        }
        self.send(key, 0, VoiceEvent::Left { session: key })?;
        self.allocate()
    }
}

fn track_permission(kind: TrackKind) -> Permissions {
    match kind {
        TrackKind::Audio => Permissions::SPEAK,
        TrackKind::Camera => Permissions::VIDEO,
        TrackKind::Screen | TrackKind::SystemAudio => Permissions::STREAM,
    }
}
fn is_video(kind: TrackKind) -> bool {
    matches!(kind, TrackKind::Camera | TrackKind::Screen)
}
fn media_kind(kind: TrackKind) -> MediaKind {
    if is_video(kind) {
        MediaKind::Video
    } else {
        MediaKind::Audio
    }
}
fn codec_of(track: &PublishedTrack) -> Codec {
    match track.codec {
        None => Codec::Opus,
        Some(VideoCodec::H264) => Codec::H264,
        Some(VideoCodec::Hevc) => Codec::H265,
        Some(VideoCodec::Av1) => Codec::Av1,
    }
}
fn is_rtp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes[0] >> 6 == 2 && !(64..=95).contains(&(bytes[1] & 127))
}
fn starts_keyframe(codec: Codec, p: &[u8]) -> bool {
    match codec {
        Codec::H264 => match p.first().map(|b| b & 31) {
            Some(5) => true,
            Some(28) => p.get(1).is_some_and(|b| b & 0x80 != 0 && b & 31 == 5),
            Some(24) => {
                let mut offset = 1;
                while offset + 2 < p.len() {
                    let len = u16::from_be_bytes([p[offset], p[offset + 1]]) as usize;
                    offset += 2;
                    if len == 0 || offset + len > p.len() {
                        return false;
                    }
                    if p[offset] & 31 == 5 {
                        return true;
                    }
                    offset += len;
                }
                false
            }
            _ => false,
        },
        Codec::H265 => {
            p.first().is_some_and(|b| matches!((b >> 1) & 63, 16..=21))
                || (p.len() >= 3
                    && (p[0] >> 1) & 63 == 49
                    && p[2] & 0x80 != 0
                    && matches!(p[2] & 63, 16..=21))
        }
        Codec::Av1 => p.first().is_some_and(|b| b & 0x08 != 0),
        _ => false,
    }
}

/// Bound queued media by the time it takes to drain at the sustainable fair
/// wire rate, not by RTT. A localhost RTT approaching zero must not turn a
/// healthy 100-200 ms frame/startup burst into loss. The independent oldest-
/// packet age guard enforces the same 250 ms residence limit, and absolute
/// byte/packet ceilings apply even to arbitrarily large compressed frames.
fn media_queue_byte_limit(bps: u64) -> u64 {
    (bps.saturating_mul(250) / 8000).clamp(1232, 2 * 1024 * 1024)
}

/// Padding and RTX must not enter this accounting. Independent age/packet
/// bounds remain strict even when a source supplies arbitrarily large IDRs.
fn media_queue_overloaded(
    pending: &VecDeque<(u16, Instant, usize)>,
    bps: u64,
    now: Instant,
) -> bool {
    let bytes = media_queue_byte_limit(bps);
    pending.len() >= 2048
        || pending.iter().map(|p| p.2 as u64).sum::<u64>() >= bytes
        || pending
            .front()
            .is_some_and(|p| now.saturating_duration_since(p.1) > Duration::from_millis(250))
}

/// RTC also carries voice, consent/revocation and RTCP. At extremely low video
/// allocations, prefer explicit video loss to holding those behind a full-MTU
/// token refill. Normal two-viewer pacing is sub-millisecond; never defer the
/// whole RTC for more than 10 ms to protect a video packet.
fn bounded_admission_deadline(ready: Instant, now: Instant) -> Option<Instant> {
    (ready > now && ready - now <= Duration::from_millis(10)).then_some(ready)
}

#[cfg(test)]
mod policy_queue_tests {
    use super::*;
    struct Acl(std::sync::atomic::AtomicU64);
    impl Authorizer for Acl {
        fn can_join(&self, _s: VoiceSession, _ch: Id) -> bool {
            true
        }
        fn generation(&self) -> u64 {
            self.0.load(std::sync::atomic::Ordering::Acquire)
        }
    }
    #[test]
    fn a_tiny_video_quota_cannot_stall_voice_or_control_for_a_full_packet_refill() {
        let now = Instant::now();
        assert_eq!(bounded_admission_deadline(now, now), None);
        let normal = now + Duration::from_micros(617);
        assert_eq!(bounded_admission_deadline(normal, now), Some(normal));
        assert_eq!(
            bounded_admission_deadline(now + Duration::from_millis(103), now),
            None
        );
    }
    #[test]
    fn original_media_queue_is_bounded_by_fair_drain_time_age_and_memory() {
        let now = Instant::now();
        let mut pending = VecDeque::new();
        // Healthy startup delta backlog: the old RTT/256 KiB guard reset
        // this at only 108 ms of age, despite <140 ms of drain work remaining.
        for seq in 0..240 {
            pending.push_back((seq, now, 1200));
        }
        assert!(!media_queue_overloaded(
            &pending,
            16_000_000,
            now + Duration::from_millis(108)
        ));
        assert!(media_queue_overloaded(
            &pending,
            16_000_000,
            now + Duration::from_millis(251)
        ));
        // A low allocation cannot retain the same queue indefinitely.
        assert!(media_queue_overloaded(&pending, 8_000_000, now));
        pending.clear();
        for seq in 0..2048 {
            pending.push_back((seq, now, 1));
        }
        assert!(media_queue_overloaded(&pending, 100_000_000, now));
        assert_eq!(media_queue_byte_limit(16_000_000), 500_000);
        assert_eq!(media_queue_byte_limit(u64::MAX), 2 * 1024 * 1024);
        assert_eq!(media_queue_byte_limit(0), 1232);
    }
    #[test]
    fn idr_fragments_keep_frame_identity_without_relaxing_the_latency_bound() {
        let now = Instant::now();
        let mut burst = FrameBurst::default();
        let mut pending = VecDeque::new();
        for seq in 0..350 {
            burst.observe(9000, seq == 0, 1200);
            pending.push_back((seq, now, 1200));
        }
        assert_eq!(burst.keyframe_bytes, 420_000);
        assert!(!media_queue_overloaded(&pending, 16_000_000, now));
        assert!(media_queue_overloaded(
            &pending,
            16_000_000,
            now + Duration::from_millis(251)
        ));
        assert!(media_queue_overloaded(&pending, 8_000_000, now));
        burst.observe(10500, false, 30_000);
        assert_eq!(burst.keyframe_bytes, 420_000);
        assert!(!burst.keyframe);
    }
    #[test]
    fn policy_generation_discards_pre_revocation_encrypted_datagrams() {
        let acl = Arc::new(Acl(std::sync::atomic::AtomicU64::new(0)));
        let mut sfu = Sfu::new(
            Config {
                ports: 52000..=52999,
                ..Default::default()
            },
            acl.clone(),
        )
        .unwrap();
        sfu.emit(Out::Datagram {
            to: "127.0.0.1:1234".parse().unwrap(),
            bytes: vec![0x80; 32],
        })
        .unwrap();
        acl.0.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        assert_eq!(sfu.drain().count(), 0);
    }
}
