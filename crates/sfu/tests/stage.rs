//! Stage audiences: listener sessions (`Authorizer::listener`).
//!
//! Real str0m peers over the in-process packet harness: ICE, DTLS and SRTP
//! for every listener, no sockets. `stage_audience_capacity` is the capacity
//! measurement (see `docs/features/stage.md`): set `STICKTALK_STAGE_LISTENERS`
//! for a larger audience and run it with `--release` for timings.
mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sticktalk_sfu::{proto::*, Authorizer, Config, Out};
use str0m::net::{Protocol, Receive};
use str0m::{Event, Input};
use support::*;

/// Live heap bytes, to measure what each listener costs the SFU.
struct Counting;
static LIVE: AtomicI64 = AtomicI64::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as i64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as i64, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        LIVE.fetch_add(size as i64 - layout.size() as i64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The host's role projection: listeners may only view, connect and listen.
#[derive(Default)]
struct StageAcl {
    listeners: Mutex<BTreeSet<VoiceSession>>,
    generation: AtomicU64,
}
impl StageAcl {
    fn set(&self, session: VoiceSession, listener: bool) {
        let mut l = self.listeners.lock().unwrap();
        if listener {
            l.insert(session);
        } else {
            l.remove(&session);
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
    fn is(&self, session: VoiceSession) -> bool {
        self.listeners.lock().unwrap().contains(&session)
    }
}
impl Authorizer for StageAcl {
    fn can_join(&self, _: VoiceSession, channel: Id) -> bool {
        channel == Id(1)
    }
    fn permissions(&self, session: VoiceSession, channel: Id) -> Permissions {
        if channel != Id(1) {
            Permissions::empty()
        } else if self.is(session) {
            Permissions::VIEW_CHANNEL | Permissions::CONNECT
        } else {
            Permissions::all()
        }
    }
    fn listener(&self, session: VoiceSession) -> bool {
        self.is(session)
    }
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
}

fn stage(config: Config) -> (Network, Arc<StageAcl>) {
    let acl = Arc::new(StageAcl::default());
    let config = Config {
        bind_ip: "127.0.0.1".parse().unwrap(),
        advertised_ip: "127.0.0.1".parse().unwrap(),
        ports: 53000..=53999,
        ..config
    };
    (Network::with_config(config, acl.clone()), acl)
}
fn listener(net: &mut Network, acl: &StageAcl) -> usize {
    acl.set(session(net.peers.len() as u64 + 1), true);
    net.add(false)
}
fn participants_to(net: &Network, to: usize) -> Vec<VoiceSession> {
    net.signals
        .iter()
        .filter(|(s, _)| *s == net.peers[to].key)
        .filter_map(|(_, e)| match e {
            VoiceEvent::Participant(p) => Some(p.session),
            _ => None,
        })
        .collect()
}
fn left_to(net: &Network, to: usize) -> Vec<VoiceSession> {
    net.signals
        .iter()
        .filter(|(s, _)| *s == net.peers[to].key)
        .filter_map(|(_, e)| match e {
            VoiceEvent::Left { session } => Some(*session),
            _ => None,
        })
        .collect()
}
fn rtp(peer: &Peer) -> usize {
    peer.events
        .iter()
        .filter(|e| matches!(e, Event::RtpPacket(_)))
        .count()
}

#[test]
fn the_audience_is_invisible_receives_and_cannot_publish() {
    let (mut net, acl) = stage(Config::default());
    let speaker = net.add(true);
    let a = listener(&mut net, &acl);
    let b = listener(&mut net, &acl);
    // The speaker never hears about listeners; listeners see only speakers.
    assert!(participants_to(&net, speaker)
        .iter()
        .all(|s| *s == net.peers[speaker].key));
    let joined: Vec<_> = net
        .signals
        .iter()
        .filter(|(s, _)| *s == net.peers[b].key)
        .filter_map(|(_, e)| match e {
            VoiceEvent::Joined { participants, .. } => {
                Some(participants.iter().map(|p| p.session).collect::<Vec<_>>())
            }
            _ => None,
        })
        .collect();
    assert_eq!(joined, vec![vec![net.peers[speaker].key]]);
    // An audience offer that publishes anything is refused.
    let refused = net.signal(
        a,
        VoiceSignal::Offer {
            sdp: "v=0".into(),
            tracks: vec![PublishedTrack {
                mid: "0".into(),
                kind: TrackKind::Audio,
                codec: None,
                layers: vec![],
                surface: None,
            }],
        },
    );
    assert!(matches!(refused, Err(sticktalk_sfu::Error::Forbidden)));
    for l in [a, b] {
        net.subscribe(l, speaker, TrackKind::Audio);
    }
    net.clear();
    for seq in 1..=20 {
        net.send(
            speaker,
            TrackKind::Audio,
            seq,
            seq as u32 * 960,
            &[1; 80],
            false,
        );
    }
    net.advance(Duration::from_millis(50));
    assert!(rtp(&net.peers[a]) >= 18 && rtp(&net.peers[b]) >= 18);
    // A listener leaving tells nobody else, and leaves nothing behind.
    net.clear();
    net.sfu.disconnect(net.peers[a].key).unwrap();
    net.collect_sfu();
    assert!(left_to(&net, speaker).is_empty() && left_to(&net, b).is_empty());
    assert_eq!(net.sfu.session_count(), 2);
    assert_eq!(net.sfu.route_count(), 1);
}

#[test]
fn moving_between_audience_and_speakers_and_dropping_audience_media() {
    let (mut net, acl) = stage(Config::default());
    // A moderator on the Stage who is listening, a guest speaking, an audience.
    let host = net.add(false);
    let guest = net.add(true);
    let ear = listener(&mut net, &acl);
    net.subscribe(ear, guest, TrackKind::Audio);
    net.subscribe(host, guest, TrackKind::Audio);
    // The guest goes back to the audience: others forget it, its tracks go.
    net.clear();
    acl.set(net.peers[guest].key, true);
    net.sfu.refresh_policy(net.peers[guest].key).unwrap();
    net.collect_sfu();
    net.flush();
    assert!(left_to(&net, host).contains(&net.peers[guest].key));
    assert!(left_to(&net, guest).is_empty(), "never its own Left");
    assert_eq!(net.sfu.route_count(), 0);
    // Whatever it still sends goes nowhere.
    net.clear();
    for seq in 1..=10 {
        net.send(
            guest,
            TrackKind::Audio,
            seq,
            seq as u32 * 960,
            &[2; 80],
            false,
        );
    }
    net.advance(Duration::from_millis(50));
    assert_eq!(rtp(&net.peers[host]) + rtp(&net.peers[ear]), 0);
    // Invited back to speak: everyone is told again.
    net.clear();
    acl.set(net.peers[guest].key, false);
    net.sfu.refresh_policy(net.peers[guest].key).unwrap();
    net.collect_sfu();
    assert!(participants_to(&net, host).contains(&net.peers[guest].key));
    assert!(participants_to(&net, ear).contains(&net.peers[guest].key));
}

#[test]
fn listeners_have_their_own_caps() {
    let (mut net, acl) = stage(Config {
        max_channel_sessions: 1,
        max_room_listeners: 3,
        max_listeners: 3,
        ..Config::default()
    });
    net.add(true);
    for _ in 0..3 {
        listener(&mut net, &acl);
    }
    assert_eq!(net.sfu.session_count(), 4);
    // A fourth listener, and a second speaker, are over their caps.
    acl.set(session(5), true);
    let full = net.sfu.handle_signal(
        session(5),
        1,
        VoiceSignal::Join {
            channel_id: Id(1),
            capabilities: caps(),
        },
        net.now,
    );
    assert!(matches!(full, Err(sticktalk_sfu::Error::Limit)));
    let full = net.sfu.handle_signal(
        session(6),
        1,
        VoiceSignal::Join {
            channel_id: Id(1),
            capabilities: caps(),
        },
        net.now,
    );
    assert!(matches!(full, Err(sticktalk_sfu::Error::Limit)));
    // A listener cannot be promoted past the room cap: it stays a listener.
    net.collect_sfu();
    acl.set(net.peers[1].key, false);
    net.sfu.refresh_policy(net.peers[1].key).unwrap();
    net.collect_sfu();
    assert!(net.signals.iter().any(|(s, e)| *s == net.peers[1].key
        && matches!(
            e,
            VoiceEvent::Error {
                code: ErrorCode::LimitExceeded,
                ..
            }
        )));
}

#[test]
fn audience_video_seats_follow_the_video_capacity() {
    // 1 Mbps of video capacity: two listener seats at LISTENER_VIDEO_BPS.
    let (mut net, acl) = stage(Config {
        video_egress_bps: 2 * sticktalk_sfu::LISTENER_VIDEO_BPS,
        ..Config::default()
    });
    let speaker = net.add(true);
    let seats: Vec<_> = (0..3).map(|_| listener(&mut net, &acl)).collect();
    net.subscribe(seats[0], speaker, TrackKind::Camera);
    net.subscribe(seats[1], speaker, TrackKind::Camera);
    let third = net.signal(
        seats[2],
        VoiceSignal::Subscribe {
            publisher: net.peers[speaker].key,
            source_mid: net.peers[speaker].video.to_string(),
            receiver_mid: net.peers[seats[2]].video.to_string(),
            rid: None,
        },
    );
    assert!(matches!(third, Err(sticktalk_sfu::Error::Limit)));
    // Audio is never limited by video seats.
    net.subscribe(seats[2], speaker, TrackKind::Audio);
}

/// One speaker's Opus (50 packets a second, 80-byte payloads) to an audience
/// of synthetic str0m receivers. Reports SFU time per second of audio, heap
/// per listener, and checks that every listener hears it and that departed
/// listeners cost nothing.
#[test]
fn stage_audience_capacity() {
    let listeners: usize = std::env::var("STICKTALK_STAGE_LISTENERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if cfg!(debug_assertions) { 40 } else { 1000 });
    let (mut net, acl) = stage(Config {
        max_listeners: listeners.max(1),
        max_room_listeners: listeners.max(1),
        ..Config::default()
    });
    let speaker = net.add(true);
    let heap_before = LIVE.load(Ordering::Relaxed);
    let setup = Instant::now();
    for n in 0..listeners {
        acl.set(session(net.peers.len() as u64 + 1), true);
        net.add_audio_listener();
        // Connect in batches: the harness advances every peer at once.
        if n % 50 == 49 || n + 1 == listeners {
            net.advance(Duration::from_millis(500));
        }
    }
    for l in 1..net.peers.len() {
        assert!(net.peers[l].rtc.is_connected(), "listener {l} connected");
        net.subscribe(l, speaker, TrackKind::Audio);
    }
    let setup = setup.elapsed();
    net.clear();
    // Measure: SFU calls only, through an address index (not the harness scan).
    let index: HashMap<_, _> = net
        .peers
        .iter()
        .enumerate()
        .map(|(i, p)| (p.address, i))
        .collect();
    let mut sfu_time = Duration::ZERO;
    let seconds = 3u64;
    let start_received: Vec<u64> = net.peers.iter().map(|p| p.received).collect();
    for seq in 1..=(50 * seconds) {
        write_audio(&mut net, speaker, seq);
        net.now += Duration::from_millis(20);
        pump(&mut net, &index, &mut sfu_time);
    }
    let heap_live = LIVE.load(Ordering::Relaxed);
    let heard = net
        .peers
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != speaker)
        .filter(|(i, p)| p.received - start_received[*i] >= 50 * seconds * 95 / 100)
        .count();
    let audio_us = sfu_time.as_secs_f64() * 1e6 / seconds as f64;
    let split = |n: &AtomicU64| n.load(Ordering::Relaxed) as f64 / 1e3 / seconds as f64;
    println!(
        "STAGE_SPLIT fanout_us_per_s={:.0} feedback_us_per_s={:.0} timer_us_per_s={:.0} feedback_packets_per_s={:.0}",
        split(&FANOUT_NS),
        split(&FEEDBACK_NS),
        split(&TIMER_NS),
        FEEDBACK_PACKETS.load(Ordering::Relaxed) as f64 / seconds as f64,
    );
    // Everyone leaves: the SFU holds nothing for them and does no work for them.
    let keys: Vec<_> = net.peers.iter().skip(1).map(|p| p.key).collect();
    for key in keys {
        net.sfu.disconnect(key).unwrap();
    }
    let _: Vec<Out> = net.sfu.drain().collect();
    let heap_after = LIVE.load(Ordering::Relaxed);
    let mut idle_time = Duration::ZERO;
    for seq in 1..=50 {
        write_audio(&mut net, speaker, 1000 + seq);
        net.now += Duration::from_millis(20);
        pump(&mut net, &index, &mut idle_time);
    }
    let per_listener_heap = (heap_live - heap_after) as f64 / listeners as f64;
    println!(
        "STAGE_CAPACITY listeners={listeners} heard={heard} setup_s={:.1} sfu_us_per_audio_second={audio_us:.0} \
         sfu_core_percent={:.2} per_listener_us={:.2} sfu_heap_per_listener_bytes={per_listener_heap:.0} \
         heap_growth_bytes={} after_leave_sessions={} after_leave_routes={} after_leave_us_per_second={:.0}",
        setup.as_secs_f64(),
        audio_us / 10_000.0,
        audio_us / listeners.max(1) as f64,
        heap_live - heap_before,
        net.sfu.session_count(),
        net.sfu.route_count(),
        idle_time.as_secs_f64() * 1e6,
    );
    assert_eq!(heard, listeners, "every listener hears the speaker");
    assert_eq!(net.sfu.session_count(), 1);
    assert_eq!(net.sfu.route_count(), 0);
}

fn write_audio(net: &mut Network, speaker: usize, seq: u64) {
    let peer = &mut net.peers[speaker];
    let mid = peer.audio;
    let pt = peer
        .rtc
        .codec_config()
        .params()
        .iter()
        .find(|p| {
            p.spec().codec == str0m::format::Codec::Opus
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
    let now = net.now;
    let mut api = peer.rtc.direct_api();
    let stream = api.stream_tx(&ssrc).unwrap();
    stream.set_unpaced(true);
    stream.write_rtp(
        str0m::rtp::RtpWrite::new(pt, seq.into(), seq as u32 * 960, now, &[7u8; 80][..]).ext_vals(
            str0m::rtp::ExtensionValues {
                audio_level: Some(-30),
                voice_activity: Some(true),
                ..Default::default()
            },
        ),
    );
    net.collect_peer(speaker);
}

/// SFU time by what it was doing: fanning a speaker's packet out, handling
/// listeners' feedback (RTCP), and timers.
static FANOUT_NS: AtomicU64 = AtomicU64::new(0);
static FEEDBACK_NS: AtomicU64 = AtomicU64::new(0);
static TIMER_NS: AtomicU64 = AtomicU64::new(0);
static FEEDBACK_PACKETS: AtomicU64 = AtomicU64::new(0);

/// Delivers every pending datagram and runs every due timer, timing the SFU.
fn pump(net: &mut Network, index: &HashMap<std::net::SocketAddr, usize>, sfu: &mut Duration) {
    let speaker = net.peers[0].address;
    for _ in 0..64 {
        let now = net.now;
        let t = Instant::now();
        net.sfu.handle_timeout(now).unwrap();
        *sfu += t.elapsed();
        TIMER_NS.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
        let t = Instant::now();
        net.collect_sfu();
        *sfu += t.elapsed();
        for i in 0..net.peers.len() {
            if net.peers[i].deadline <= now {
                net.peers[i].rtc.handle_input(Input::Timeout(now)).unwrap();
                net.collect_peer(i);
            }
        }
        if net.pending.is_empty() {
            break;
        }
        while let Some((from, to, bytes)) = net.pending.pop_front() {
            if to == net.sfu.local_addr() {
                let t = Instant::now();
                net.sfu.handle_datagram(from, &bytes, now).unwrap();
                net.collect_sfu();
                *sfu += t.elapsed();
                let ns = t.elapsed().as_nanos() as u64;
                if from == speaker {
                    FANOUT_NS.fetch_add(ns, Ordering::Relaxed);
                } else {
                    FEEDBACK_NS.fetch_add(ns, Ordering::Relaxed);
                    FEEDBACK_PACKETS.fetch_add(1, Ordering::Relaxed);
                }
            } else if let Some(&i) = index.get(&to) {
                let receive = Receive::new(Protocol::Udp, from, to, &bytes).unwrap();
                net.peers[i]
                    .rtc
                    .handle_input(Input::Receive(now, receive))
                    .unwrap();
                net.collect_peer(i);
            }
        }
        // The harness keeps every received packet; only the count matters here.
        for p in &mut net.peers {
            p.events.clear();
        }
    }
}
