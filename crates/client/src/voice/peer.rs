//! Native WebRTC endpoint. Client is always offerer; chat owns signaling.
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use sticktalk_proto::*;
use str0m::{
    change::{SdpAnswer, SdpPendingOffer},
    channel::{ChannelConfig, ChannelId as DataId},
    format::Codec,
    media::{Direction, MediaKind, Mid},
    net::{Protocol, Receive},
    rtp::{ExtensionValues, RtpPacket, RtpWrite},
    Candidate, Event, Input, Output, Rtc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub publisher: VoiceSession,
    pub track: PublishedTrack,
}
// RTP is the hot-path majority. Keep it inline rather than allocating a Box
// per packet; drain() bounds the transient vector to 4096 entries (<2 MiB).
#[allow(clippy::large_enum_variant)]
pub enum PeerOutput {
    Signal(VoiceSignal),
    Datagram(SocketAddr, Vec<u8>),
    Connected,
    Packet(Remote, RtpPacket),
    Control(Vec<u8>),
    Keyframe(TrackKind),
}
struct Slot {
    mid: Mid,
    kind: MediaKind,
    source: Option<Remote>,
}
pub struct Peer {
    rtc: Rtc,
    local: SocketAddr,
    pending: Option<SdpPendingOffer>,
    publications: Vec<(Mid, TrackKind, bool)>,
    slots: Vec<Slot>,
    desired: Vec<Remote>,
    control: DataId,
    deadline: Instant,
    signals: Vec<VoiceSignal>,
    sequences: BTreeMap<String, u64>,
    surface: ShareSurface,
    queued_publications: Vec<(TrackKind, bool, ShareSurface)>,
    send_budgets: BTreeMap<u8, u64>,
    bwe_target: u64,
    video_gates: BTreeMap<String, VideoGate>,
    staged_bytes: BTreeMap<String, usize>,
    keyframes: Vec<TrackKind>,
}
fn err(e: impl std::fmt::Display) -> String {
    format!("Media transport: {e}")
}
/// H.264 IDRs arrive in bursts. Match the SFU's bounded socket buffers; a
/// small OS-default receive buffer can lose an entire IDR even on loopback.
#[cfg(windows)]
pub fn configure_socket(
    socket: &impl std::os::windows::io::AsSocket,
) -> Result<(usize, usize), String> {
    let socket = socket2::SockRef::from(socket);
    let previous = socket.recv_buffer_size().map_err(err)?;
    socket.set_recv_buffer_size(256 * 1024).map_err(err)?;
    socket.set_send_buffer_size(256 * 1024).map_err(err)?;
    Ok((previous, socket.recv_buffer_size().map_err(err)?))
}
impl Peer {
    pub fn new(local: SocketAddr, mtu: u16, now: Instant) -> Result<Self, String> {
        if !(1200..=1472).contains(&mtu) || local.ip().is_unspecified() {
            return Err("Invalid media address/MTU".into());
        }
        let mut config = Rtc::builder()
            .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
            .set_rtp_mode(true)
            .set_mtu(usize::from(mtu)..=usize::from(mtu))
            .set_stats_interval(Some(Duration::from_secs(1)))
            .set_send_buffer_audio(32)
            .set_send_buffer_video(256)
            .set_sctp_max_message_size(512)
            .set_sctp_max_buffered_amount(16 * 1024)
            .enable_bwe(Some(str0m::bwe::Bitrate::bps(8_000_000)));
        let codecs = config.codec_config();
        codecs.clear();
        codecs.enable_opus(true, false);
        // Offer the SFU's full H.264 profile ladder. str0m 0.25 can otherwise
        // match both server High-profile levels to one remote PT on a later
        // recvonly offer, creating duplicate locked PTs in its answerer.
        codecs.enable_h264(true);
        codecs.add_h264(112.into(), Some(113.into()), true, 0x640033);
        let mut rtc = config.build(now);
        rtc.add_local_candidate(Candidate::host(local, "udp").map_err(err)?)
            .ok_or("Duplicate media candidate")?;
        // str0m requires poll_output exhaustion before another mutation.
        while !matches!(rtc.poll_output().map_err(err)?, Output::Timeout(_)) {}
        let mut api = rtc.sdp_api();
        let audio = api.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        let control = api.add_channel_with_config(ChannelConfig {
            label: CONTROL_CHANNEL_LABEL.into(),
            ..Default::default()
        });
        let (offer, pending) = api.apply().ok_or("Missing media offer")?;
        Ok(Self {
            rtc,
            local,
            pending: Some(pending),
            publications: vec![(audio, TrackKind::Audio, true)],
            slots: vec![],
            desired: vec![],
            control,
            deadline: now,
            signals: vec![VoiceSignal::Offer {
                sdp: offer.to_sdp_string(),
                tracks: vec![track(audio, TrackKind::Audio, ShareSurface::Monitor)],
            }],
            sequences: BTreeMap::new(),
            surface: ShareSurface::Monitor,
            queued_publications: vec![],
            send_budgets: BTreeMap::new(),
            bwe_target: 0,
            video_gates: BTreeMap::new(),
            staged_bytes: BTreeMap::new(),
            keyframes: vec![],
        })
    }
    /// Desired encoder payload rate, not a bandwidth floor. Congestion control
    /// remains authoritative; the desired rate enables probing after an idle call.
    pub fn set_send_bitrate(&mut self, kind: TrackKind, bps: u64) {
        self.send_budgets
            .insert(kind as u8, bps.clamp(100_000, 100_000_000));
    }
    fn desired_send_bps(&self) -> u64 {
        self.publications
            .iter()
            .filter(|(_, _, active)| *active)
            .map(|(_, kind, _)| {
                self.send_budgets.get(&(*kind as u8)).copied().unwrap_or(
                    if media_kind(*kind) == MediaKind::Video {
                        8_000_000
                    } else {
                        256_000
                    },
                )
            })
            .sum::<u64>()
            .saturating_mul(12)
            .saturating_div(10)
            .min(100_000_000)
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn publication_kind(&self, mid: &str) -> Option<TrackKind> {
        self.publications
            .iter()
            .find(|(m, _, _)| m.to_string() == mid)
            .map(|(_, k, _)| *k)
    }
    pub fn request_keyframe(&mut self, source: &Remote) {
        if let Some(slot) = self
            .slots
            .iter()
            .find(|s| s.source.as_ref() == Some(source))
        {
            self.signals.push(VoiceSignal::RequestKeyframe {
                receiver_mid: slot.mid.to_string(),
                fir: false,
            });
        }
    }
    pub fn timeout(&mut self, now: Instant) -> Result<(), String> {
        self.rtc.handle_input(Input::Timeout(now)).map_err(err)
    }
    pub fn datagram(&mut self, from: SocketAddr, bytes: &[u8], now: Instant) -> Result<(), String> {
        let receive = Receive::new(Protocol::Udp, from, self.local, bytes).map_err(err)?;
        self.rtc
            .handle_input(Input::Receive(now, receive))
            .map_err(err)
    }
    pub fn answer(&mut self, sdp: &str) -> Result<(), String> {
        let pending = self.pending.take().ok_or("Unexpected media answer")?;
        self.rtc
            .sdp_api()
            .accept_answer(pending, SdpAnswer::from_sdp_string(sdp).map_err(err)?)
            .map_err(err)?;
        for slot in &self.slots {
            if let Some(s) = &slot.source {
                self.signals.push(VoiceSignal::Subscribe {
                    publisher: s.publisher,
                    source_mid: s.track.mid.clone(),
                    receiver_mid: slot.mid.to_string(),
                    rid: s.track.layers.last().cloned(),
                });
            }
        }
        Ok(())
    }
    pub fn remote_tracks(
        &mut self,
        own: Option<VoiceSession>,
        participants: impl Iterator<Item = VoiceParticipant>,
    ) {
        self.desired = participants
            .take(32)
            .filter(|p| Some(p.session) != own)
            .flat_map(|p| {
                p.tracks.into_iter().take(4).map(move |track| Remote {
                    publisher: p.session,
                    track,
                })
            })
            .collect();
        // A video-heavy room must never consume all recv slots before later
        // participants' microphones. Participant and publication counts are
        // bounded by the call reducer (32 x 4).
        self.desired.sort_by_key(|s| match s.track.kind {
            TrackKind::Audio => 0,
            TrackKind::SystemAudio => 1,
            TrackKind::Screen => 2,
            TrackKind::Camera => 3,
        });
        self.desired.truncate(32);
    }
    pub fn publish(
        &mut self,
        kind: TrackKind,
        enabled: bool,
        surface: ShareSurface,
    ) -> Result<(), String> {
        if self.pending.is_some() {
            self.queued_publications.retain(|(k, _, _)| *k != kind);
            self.queued_publications.push((kind, enabled, surface));
            return Ok(());
        }
        if kind == TrackKind::Screen {
            self.surface = surface;
        }
        let mut api = self.rtc.sdp_api();
        if let Some((mid, _, active)) = self.publications.iter_mut().find(|(_, k, _)| *k == kind) {
            *active = enabled;
            api.set_direction(
                *mid,
                if enabled {
                    Direction::SendOnly
                } else {
                    Direction::Inactive
                },
            );
        } else if enabled {
            if self.publications.len() + self.slots.len() >= 32 {
                return Err("Media track limit".into());
            }
            let mid = api.add_media(media_kind(kind), Direction::SendOnly, None, None, None);
            self.publications.push((mid, kind, true));
        }
        if let Some((offer, pending)) = api.apply() {
            self.pending = Some(pending);
            self.signals.push(VoiceSignal::Offer {
                sdp: offer.to_sdp_string(),
                tracks: self.tracks(),
            });
        }
        Ok(())
    }
    fn tracks(&self) -> Vec<PublishedTrack> {
        self.publications
            .iter()
            .filter(|(_, _, active)| *active)
            .map(|(m, k, _)| track(*m, *k, self.surface))
            .collect()
    }
    /// Called only after draining RTC output. Serializes offers and reuses recv slots.
    pub fn reconcile(&mut self) -> Result<(), String> {
        let desired = self.desired_send_bps();
        if self.bwe_target != desired {
            self.bwe_target = desired;
            self.rtc.bwe().set_bitrate(
                str0m::bwe::Bitrate::bps(0),
                str0m::bwe::Bitrate::bps(desired),
            );
            // Caller drains output before the next SDP/RTP mutation.
            return Ok(());
        }
        if self.pending.is_some() {
            return Ok(());
        }
        if !self.queued_publications.is_empty() {
            let (kind, enabled, surface) = self.queued_publications.remove(0);
            self.publish(kind, enabled, surface)?;
            return Ok(());
        }
        for slot in &mut self.slots {
            if slot
                .source
                .as_ref()
                .is_some_and(|s| !self.desired.contains(s))
            {
                self.signals.push(VoiceSignal::Unsubscribe {
                    receiver_mid: slot.mid.to_string(),
                });
                slot.source = None;
            }
        }
        let mut changed = false;
        let mut api = self.rtc.sdp_api();
        for source in &self.desired {
            if self.slots.iter().any(|s| s.source.as_ref() == Some(source)) {
                continue;
            }
            let kind = media_kind(source.track.kind);
            let index = if let Some(i) = self
                .slots
                .iter()
                .position(|s| s.source.is_none() && s.kind == kind)
            {
                i
            } else {
                if self.slots.len() + self.publications.len() >= 32 {
                    break;
                }
                let mid = api.add_media(kind, Direction::RecvOnly, None, None, None);
                self.slots.push(Slot {
                    mid,
                    kind,
                    source: None,
                });
                self.slots.len() - 1
            };
            let slot = &mut self.slots[index];
            slot.source = Some(source.clone());
            // A direction update forces an offer only for a newly created m-line.
            api.set_direction(slot.mid, Direction::RecvOnly);
            changed = true;
        }
        if let Some((offer, pending)) = api.apply() {
            self.pending = Some(pending);
            self.signals.push(VoiceSignal::Offer {
                sdp: offer.to_sdp_string(),
                tracks: self.tracks(),
            });
        } else if changed {
            for slot in &self.slots {
                if let Some(s) = &slot.source {
                    self.signals.push(VoiceSignal::Subscribe {
                        publisher: s.publisher,
                        source_mid: s.track.mid.clone(),
                        receiver_mid: slot.mid.to_string(),
                        rid: s.track.layers.last().cloned(),
                    });
                }
            }
        }
        Ok(())
    }
    pub fn send(
        &mut self,
        kind: TrackKind,
        timestamp: u32,
        payload: &[u8],
        marker: bool,
        level: i8,
        now: Instant,
    ) -> Result<(), String> {
        if !self.rtc.is_connected() {
            return Ok(());
        }
        let mid = self
            .publications
            .iter()
            .find(|(_, k, a)| *k == kind && *a)
            .map(|(m, _, _)| *m);
        let Some(mid) = mid else {
            return if self
                .queued_publications
                .iter()
                .any(|(k, enabled, _)| *k == kind && *enabled)
            {
                Ok(())
            } else {
                Err("Track not published".into())
            };
        };
        // Capture can produce a first frame before the new publication's
        // answer arrives. Drop it, rather than terminating the ongoing call.
        // Subscription PLI requests the next IDR after negotiation completes.
        if self.pending.is_some()
            && self
                .rtc
                .media(mid)
                .is_none_or(|m| m.remote_pts().is_empty())
        {
            return Ok(());
        }
        let codec = if media_kind(kind) == MediaKind::Audio {
            Codec::Opus
        } else {
            Codec::H264
        };
        let pt = self
            .rtc
            .codec_config()
            .params()
            .iter()
            .find(|p| {
                p.spec().codec == codec
                    && (codec != Codec::H264 || p.spec().format.profile_level_id == Some(0x640033))
                    && self
                        .rtc
                        .media(mid)
                        .is_some_and(|m| m.remote_pts().contains(&p.pt()))
            })
            .map(|p| p.pt())
            .ok_or("No negotiated payload type")?;
        let ssrc = self
            .rtc
            .direct_api()
            .stream_tx_by_mid(mid, None)
            .map(|s| s.ssrc());
        let Some(ssrc) = ssrc else {
            return if self.pending.is_some() {
                Ok(())
            } else {
                Err("No media sender".into())
            };
        };
        let key = mid.to_string();
        if media_kind(kind) == MediaKind::Video {
            // RTX cache size does NOT bound str0m's unsent RTP queue. Use its
            // refreshed pacer snapshot plus this undrained batch's bytes.
            let queued = self.rtc.direct_api().stream_tx(&ssrc).and_then(|s| {
                s.queue_info()
                    .map(|q| (q.packet_count(), q.byte_size(), q.first_unsent()))
            });
            let staged = self.staged_bytes.get(&key).copied().unwrap_or(0);
            let full = queued.is_some_and(|(packets, bytes, first)| {
                packets >= 2048
                    || bytes.saturating_add(staged) >= 2 * 1024 * 1024
                    || first.is_some_and(|at| {
                        now.saturating_duration_since(at) > Duration::from_millis(250)
                    })
            }) || staged >= 2 * 1024 * 1024;
            let (admit, request) = self
                .video_gates
                .entry(key.clone())
                .or_default()
                .admit(timestamp, payload, full);
            if request && !self.keyframes.contains(&kind) {
                self.keyframes.push(kind);
            }
            if !admit {
                return Ok(());
            }
            *self.staged_bytes.entry(key.clone()).or_default() += payload.len();
        }
        let seq = self.sequences.entry(key).or_default();
        let mut api = self.rtc.direct_api();
        let stream = api.stream_tx(&ssrc).ok_or("No RTP sender")?;
        if media_kind(kind) == MediaKind::Audio {
            stream.set_unpaced(true);
        }
        stream.write_rtp(
            RtpWrite::new(pt, (*seq).into(), timestamp, now, payload)
                .marker(marker)
                .nackable(true)
                .ext_vals(ExtensionValues {
                    audio_level: (kind == TrackKind::Audio).then_some(level),
                    voice_activity: (kind == TrackKind::Audio).then_some(level > -60),
                    ..Default::default()
                }),
        );
        *seq += 1;
        Ok(())
    }
    pub fn control(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > 512 {
            return Err("Control frame too large".into());
        }
        if !self
            .rtc
            .channel(self.control)
            .ok_or("Control channel unavailable")?
            .write(true, bytes)
            .map_err(err)?
        {
            return Err("Control backpressure; revoke".into());
        }
        Ok(())
    }
    pub fn drain(&mut self) -> Result<Vec<PeerOutput>, String> {
        let mut out: Vec<_> = self.signals.drain(..).map(PeerOutput::Signal).collect();
        out.extend(self.keyframes.drain(..).map(PeerOutput::Keyframe));
        loop {
            match self.rtc.poll_output().map_err(err)? {
                Output::Timeout(t) => {
                    if !self.rtc.is_alive() {
                        return Err("Media transport closed; release control".into());
                    }
                    self.deadline = t;
                    break;
                }
                Output::Transmit(t) => {
                    out.push(PeerOutput::Datagram(t.destination, t.contents.to_vec()))
                }
                Output::Event(Event::Connected) => out.push(PeerOutput::Connected),
                Output::Event(Event::RtpPacket(packet)) => {
                    let mid = self
                        .rtc
                        .direct_api()
                        .stream_rx(&packet.header.ssrc)
                        .map(|s| s.mid());
                    if let Some(source) = self
                        .slots
                        .iter()
                        .find(|s| Some(s.mid) == mid)
                        .and_then(|s| s.source.clone())
                    {
                        out.push(PeerOutput::Packet(source, packet));
                    }
                }
                Output::Event(Event::ChannelClose(channel)) if channel == self.control => {
                    return Err("Control channel closed; release control".into());
                }
                Output::Event(Event::ChannelData(data)) if data.binary => {
                    out.push(PeerOutput::Control(data.data))
                }
                Output::Event(Event::KeyframeRequest(request)) => {
                    if let Some((_, kind, _)) = self
                        .publications
                        .iter()
                        .find(|(mid, _, _)| *mid == request.mid)
                    {
                        out.push(PeerOutput::Keyframe(*kind));
                    }
                }
                _ => {}
            }
            if out.len() > 4096 {
                return Err("Media output overload".into());
            }
        }
        self.staged_bytes.clear();
        Ok(out)
    }
}
fn media_kind(kind: TrackKind) -> MediaKind {
    match kind {
        TrackKind::Audio | TrackKind::SystemAudio => MediaKind::Audio,
        _ => MediaKind::Video,
    }
}
fn track(mid: Mid, kind: TrackKind, surface: ShareSurface) -> PublishedTrack {
    PublishedTrack {
        mid: mid.to_string(),
        kind,
        codec: (media_kind(kind) == MediaKind::Video).then_some(VideoCodec::H264),
        layers: vec![],
        surface: (kind == TrackKind::Screen).then_some(surface),
    }
}

/// Backpressure discards dependent frames until a fresh IDR, never a growing
/// video queue. Local packetization emits single NALs or FU-A (not STAP-A).
#[derive(Default)]
struct VideoGate {
    blocked_frame: Option<u32>,
}
impl VideoGate {
    fn admit(&mut self, timestamp: u32, payload: &[u8], full: bool) -> (bool, bool) {
        if full {
            let request = self.blocked_frame.is_none();
            self.blocked_frame = Some(timestamp);
            return (false, request);
        }
        let Some(blocked) = self.blocked_frame else {
            return (true, false);
        };
        if blocked == timestamp {
            return (false, false);
        }
        let nal = payload.first().copied().unwrap_or(0) & 31;
        let idr =
            nal == 5 || (nal == 28 && payload.get(1).is_some_and(|b| b & 0x80 != 0 && b & 31 == 5));
        if idr {
            self.blocked_frame = None;
            return (true, false);
        }
        (matches!(nal, 7 | 8), false)
    }
}
#[cfg(test)]
mod sender_tests {
    use super::*;
    #[test]
    fn a_closed_transport_fails_the_actor_instead_of_retaining_control() {
        let mut peer = Peer::new("127.0.0.1:23458".parse().unwrap(), 1200, Instant::now()).unwrap();
        peer.drain().unwrap();
        peer.rtc.disconnect();
        assert!(peer.drain().is_err());
    }
    #[cfg(windows)]
    #[test]
    fn loopback_socket_reserves_a_bounded_idr_burst_buffer() {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let (_, after) = configure_socket(&socket).unwrap();
        assert!((256 * 1024..=512 * 1024).contains(&after));
        assert_eq!(
            socket2::SockRef::from(&socket).send_buffer_size().unwrap(),
            256 * 1024
        );
    }
    #[test]
    fn idle_to_video_declares_probe_capacity_without_a_bandwidth_floor() {
        let mut peer = Peer::new("127.0.0.1:23456".parse().unwrap(), 1200, Instant::now()).unwrap();
        peer.reconcile().unwrap();
        assert_eq!(peer.bwe_target, 307_200);
        peer.drain().unwrap();
        peer.publications
            .push(("video".into(), TrackKind::Screen, true));
        peer.set_send_bitrate(TrackKind::Screen, 12_000_000);
        peer.reconcile().unwrap();
        assert_eq!(peer.bwe_target, 14_707_200);
    }
    #[test]
    fn congestion_discards_the_damaged_frame_and_resumes_only_on_new_idr() {
        let mut gate = VideoGate::default();
        assert_eq!(gate.admit(1, &[0x61], false), (true, false));
        assert_eq!(gate.admit(1, &[0x7c, 0x85], true), (false, true));
        assert_eq!(gate.admit(1, &[0x65], false), (false, false));
        assert_eq!(gate.admit(2, &[0x61], false), (false, false));
        assert_eq!(gate.admit(3, &[0x67], false), (true, false));
        assert_eq!(gate.admit(3, &[0x68], false), (true, false));
        assert_eq!(gate.admit(3, &[0x7c, 0x05], false), (false, false));
        assert_eq!(gate.admit(3, &[0x7c, 0x85], false), (true, false));
        assert_eq!(gate.admit(3, &[0x7c, 0x45], false), (true, false));
    }
}
