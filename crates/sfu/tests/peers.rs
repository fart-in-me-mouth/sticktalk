mod support;
use std::time::Duration;
use sticktalk_sfu::proto::*;
use str0m::{media::KeyframeRequestKind, Event};
use support::*;

#[test]
fn three_peers_audio_camera_screen_receive_mute_deafen_leave() {
    let mut n = Network::new(40_000_000);
    let a = n.add(true);
    let b = n.add(false);
    let c = n.add(false);
    for receiver in [b, c] {
        for kind in [TrackKind::Audio, TrackKind::Camera, TrackKind::Screen] {
            n.subscribe(receiver, a, kind);
        }
    }
    n.clear();
    for seq in 1..=8 {
        n.send(
            a,
            TrackKind::Audio,
            seq,
            seq as u32 * 960,
            &[0xf8, 0xff, 0xfe],
            false,
        );
        n.send(
            a,
            TrackKind::Camera,
            seq,
            seq as u32 * 3000,
            &[0x65, 1, 2, 3],
            true,
        );
        n.send(
            a,
            TrackKind::Screen,
            seq,
            seq as u32 * 1500,
            &[0x65, 4, 5, 6],
            true,
        );
        n.advance(Duration::from_millis(20));
    }
    for receiver in [b, c] {
        for payload in [
            &[0xf8, 0xff, 0xfe][..],
            &[0x65, 1, 2, 3][..],
            &[0x65, 4, 5, 6][..],
        ] {
            assert!(
                n.peers[receiver]
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::RtpPacket(p) if p.payload.as_ref()==payload)),
                "receiver {receiver}, missing {payload:?}, signals={:?}, received {:?}",
                n.signals,
                n.peers[receiver]
                    .events
                    .iter()
                    .filter_map(|e| if let Event::RtpPacket(p) = e {
                        Some(p.payload.as_ref())
                    } else {
                        None
                    })
                    .collect::<Vec<_>>()
            );
        }
    }
    assert!(n
        .signals
        .iter()
        .any(|(_, e)| matches!(e, VoiceEvent::Speaking { speaking: true, .. })));
    assert!(n.max_rtp <= 1200);
    n.signal(
        a,
        VoiceSignal::State {
            muted: true,
            deafened: false,
        },
    )
    .unwrap();
    n.advance(Duration::from_millis(100));
    n.clear();
    n.send(a, TrackKind::Audio, 9, 9000, &[0xf8, 0xff, 0xfe], false);
    n.advance(Duration::from_millis(30));
    assert!(!n.peers[b]
        .events
        .iter()
        .any(|e| matches!(e, Event::RtpPacket(_))));
    n.signal(
        a,
        VoiceSignal::State {
            muted: false,
            deafened: false,
        },
    )
    .unwrap();
    n.signal(
        b,
        VoiceSignal::State {
            muted: false,
            deafened: true,
        },
    )
    .unwrap();
    n.clear();
    n.send(a, TrackKind::Audio, 10, 10000, &[0xf8, 0xff, 0xfe], false);
    n.advance(Duration::from_millis(30));
    assert!(!n.peers[b]
        .events
        .iter()
        .any(|e| matches!(e, Event::RtpPacket(_))));
    assert!(n.peers[c]
        .events
        .iter()
        .any(|e| matches!(e, Event::RtpPacket(_))));
    n.signal(a, VoiceSignal::Leave).unwrap();
    assert_eq!(n.sfu.session_count(), 2);
    assert_eq!(n.sfu.route_count(), 0);
    assert!(n
        .signals
        .iter()
        .any(|(_, e)| matches!(e,VoiceEvent::Left{session:s} if *s==session(1))));
    for i in [b, c] {
        n.signal(i, VoiceSignal::Leave).unwrap();
    }
    assert_eq!(n.sfu.session_count(), 0);
    assert_eq!(n.sfu.next_timeout(), None);
}

#[test]
fn pli_and_fir_reach_publisher_and_nack_rtx_repairs_loss() {
    let mut n = Network::new(40_000_000);
    let a = n.add(true);
    let b = n.add(false);
    n.subscribe(b, a, TrackKind::Camera);
    for seq in 1..=4 {
        n.send(
            a,
            TrackKind::Camera,
            seq,
            seq as u32 * 3000,
            &[0x65, 1, 2],
            true,
        );
        n.advance(Duration::from_millis(20));
    }
    for fir in [false, true] {
        n.clear();
        n.signal(
            b,
            VoiceSignal::RequestKeyframe {
                receiver_mid: n.peers[b].video.to_string(),
                fir,
            },
        )
        .unwrap();
        n.advance(Duration::from_millis(500));
        let expected = if fir {
            KeyframeRequestKind::Fir
        } else {
            KeyframeRequestKind::Pli
        };
        assert!(
            n.peers[a]
                .events
                .iter()
                .any(|e| matches!(e,Event::KeyframeRequest(r) if r.kind==expected)),
            "{expected:?} relayed upstream"
        );
    }
    n.clear();
    n.drop_next_video = true;
    for seq in 5..=14 {
        n.send(
            a,
            TrackKind::Camera,
            seq,
            seq as u32 * 3000,
            &[0x65, seq as u8, 9],
            true,
        );
        n.advance(Duration::from_millis(15));
    }
    n.advance(Duration::from_millis(300));
    assert_eq!(n.dropped, 1);
    assert!(
        n.peers[b]
            .events
            .iter()
            .any(|e| matches!(e,Event::RtpPacket(p) if p.payload.as_ref()==[0x65,5,9])),
        "lost video recovered through RTX"
    );
}

#[test]
fn global_cap_feedback_mtu_and_overlay_stats() {
    let cap = 600_000;
    let mut n = Network::new(cap);
    let a = n.add(true);
    let b = n.add(false);
    let c = n.add(false);
    n.subscribe(b, a, TrackKind::Camera);
    n.subscribe(c, a, TrackKind::Camera);
    assert!(n
        .signals
        .iter()
        .any(|(_, e)| matches!(e,VoiceEvent::Bitrate{bps,..} if *bps <= cap/2)));
    n.clear();
    let before = n.video_bytes;
    for seq in 1..=600 {
        let mut payload = vec![0; 1000];
        payload[0] = 0x65;
        n.send(a, TrackKind::Camera, seq, seq as u32 * 1500, &payload, true);
        n.advance(Duration::from_millis(2));
    }
    n.advance(Duration::from_millis(200));
    // 1.4 sec * cap/8 + 100ms token-bucket burst, including RTX/probes.
    assert!(
        n.video_bytes - before <= cap / 8 * 14 / 10 + cap / 80 + 2400,
        "{} bytes",
        n.video_bytes - before
    );
    assert!(n.max_rtp <= 1200);
    assert!(
        n.signals
            .iter()
            .any(|(_, e)| matches!(e,VoiceEvent::Stats(s) if s.cap_dropped_packets>0)),
        "video={} stats={:?}",
        n.video_bytes - before,
        n.signals
            .iter()
            .filter(|(_, e)| matches!(e, VoiceEvent::Stats(_)))
            .collect::<Vec<_>>()
    );
    assert!(n
        .signals
        .iter()
        .any(|(_, e)| matches!(e,VoiceEvent::Stats(s) if s.rx_bps>0)));
    n.sfu.set_path(n.peers[b].key, MediaPath::Relayed).unwrap();
    n.signal(b, VoiceSignal::Stats).unwrap();
    assert!(n
        .signals
        .iter()
        .any(|(_, e)| matches!(e,VoiceEvent::Stats(s) if s.path==MediaPath::Relayed)));
    // Oversized UDP packets are still rejected at the real path-MTU boundary.
    let payload = vec![0x65; 1400];
    n.send(a, TrackKind::Camera, 601, 999999, &payload, true);
    n.signal(a, VoiceSignal::Stats).unwrap();
    assert!(n
        .signals
        .iter()
        .any(|(_, e)| matches!(e,VoiceEvent::Stats(s) if s.mtu_dropped_packets>0)));
}

fn input(sharer: VoiceSession, grant: u64, sequence: u64) -> ControlData {
    ControlData::Input {
        sharer,
        grant,
        sequence,
        input: ControlInput::Move { x: 123, y: 456 },
    }
}
fn grant(n: &Network) -> u64 {
    n.signals
        .iter()
        .find_map(|(_, e)| {
            if let VoiceEvent::ControlGranted { grant, .. } = e {
                Some(*grant)
            } else {
                None
            }
        })
        .unwrap()
}

#[test]
fn data_channel_control_requires_explicit_session_grant_and_revokes() {
    let mut n = Network::new(40_000_000);
    let a = n.add(true);
    let b = n.add(false);
    let c = n.add(false);
    n.subscribe(b, a, TrackKind::Screen);
    n.subscribe(c, a, TrackKind::Screen);
    let sharer = n.peers[a].key;
    n.clear();
    n.data(b, input(sharer, 999, 1));
    assert!(n.peers[a].control.is_empty());
    n.signal(b, VoiceSignal::RequestControl { sharer }).unwrap();
    assert!(n
        .signal(
            c,
            VoiceSignal::DecideControl {
                requester: n.peers[b].key,
                allow: true
            }
        )
        .is_err());
    n.signal(
        a,
        VoiceSignal::DecideControl {
            requester: n.peers[b].key,
            allow: true,
        },
    )
    .unwrap();
    let id = grant(&n);
    n.data(c, input(sharer, id, 1));
    assert!(n.peers[a].control.is_empty());
    n.data(b, input(sharer, id, 1));
    assert_eq!(n.peers[a].control, vec![input(sharer, id, 1)]);
    assert!(n.signal(c, VoiceSignal::RequestControl { sharer }).is_err());
    n.signal(a, VoiceSignal::RevokeControl).unwrap();
    n.advance(Duration::from_millis(10));
    assert!(n.peers[a]
        .control
        .contains(&ControlData::ReleaseAll { grant: id }));
    let count = n.peers[a].control.len();
    n.data(b, input(sharer, id, 2));
    assert_eq!(n.peers[a].control.len(), count);
    n.clear();
    n.signal(b, VoiceSignal::RequestControl { sharer }).unwrap();
    n.signal(
        a,
        VoiceSignal::DecideControl {
            requester: n.peers[b].key,
            allow: false,
        },
    )
    .unwrap();
    assert!(n
        .signals
        .iter()
        .any(|(_, e)| matches!(e, VoiceEvent::ControlDenied { .. })));
    n.clear();
    n.signal(b, VoiceSignal::RequestControl { sharer }).unwrap();
    n.advance(Duration::from_secs(31));
    assert!(n
        .signal(
            a,
            VoiceSignal::DecideControl {
                requester: n.peers[b].key,
                allow: true
            }
        )
        .is_err());
    assert!(n
        .signals
        .iter()
        .any(|(_, e)| matches!(e, VoiceEvent::ControlDenied { .. })));
    n.clear();
    n.signal(b, VoiceSignal::RequestControl { sharer }).unwrap();
    n.signal(
        a,
        VoiceSignal::DecideControl {
            requester: n.peers[b].key,
            allow: true,
        },
    )
    .unwrap();
    let id = grant(&n);
    n.signal(b, VoiceSignal::Leave).unwrap();
    n.advance(Duration::from_millis(20));
    assert!(n.peers[a]
        .control
        .contains(&ControlData::ReleaseAll { grant: id }));
}

#[test]
fn control_rate_replay_and_1000_events_per_second() {
    let mut n = Network::new(40_000_000);
    let a = n.add(true);
    let b = n.add(false);
    n.subscribe(b, a, TrackKind::Screen);
    let sharer = n.peers[a].key;
    n.signal(b, VoiceSignal::RequestControl { sharer }).unwrap();
    n.signal(
        a,
        VoiceSignal::DecideControl {
            requester: n.peers[b].key,
            allow: true,
        },
    )
    .unwrap();
    let id = grant(&n);
    // Keep simulated time in one second; SCTP is progressed at each packet.
    for seq in 0..1001 {
        let bytes = rmp_serde::to_vec_named(&input(sharer, id, seq)).unwrap();
        let channel = n.peers[b].channel;
        assert!(n.peers[b]
            .rtc
            .channel(channel)
            .unwrap()
            .write(true, &bytes)
            .unwrap());
        n.collect_peer(b);
        n.flush();
    }
    n.advance(Duration::from_millis(100));
    let inputs = n.peers[a]
        .control
        .iter()
        .filter(|d| matches!(d, ControlData::Input { .. }))
        .count();
    assert_eq!(inputs, 1000);
    assert!(n.peers[a]
        .control
        .contains(&ControlData::ReleaseAll { grant: id }));
    n.clear();
    n.signal(b, VoiceSignal::RequestControl { sharer }).unwrap();
    n.signal(
        a,
        VoiceSignal::DecideControl {
            requester: n.peers[b].key,
            allow: true,
        },
    )
    .unwrap();
    let id = grant(&n);
    n.data(b, input(sharer, id, 1));
    n.data(b, input(sharer, id, 1));
    assert!(n.peers[a]
        .control
        .contains(&ControlData::ReleaseAll { grant: id }));
}

#[test]
fn acl_checked_after_join_and_old_envelopes_remain_decodable() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let allowed = Arc::new(AtomicBool::new(true));
    let access = allowed.clone();
    let mut n = Network::with_acl(
        32_000_000,
        Arc::new(move |_: VoiceSession, _: Id| access.load(Ordering::Relaxed)),
    );
    let a = n.add(true);
    allowed.store(false, Ordering::Relaxed);
    assert!(n.signal(a, VoiceSignal::Stats).is_err());
    n.advance(Duration::from_millis(50));
    assert_eq!(n.sfu.session_count(), 0);
    let old = VoicePayload {
        nonce: 13,
        version: 0,
        kind: "reserved".into(),
        payload: vec![0, 127, 255],
    };
    let bytes = rmp_serde::to_vec_named(&ClientMsg::Voice(old.clone())).unwrap();
    assert_eq!(
        rmp_serde::from_slice::<ClientMsg>(&bytes).unwrap(),
        ClientMsg::Voice(old)
    );
}

#[test]
fn per_receiver_simulcast_selection_and_keyframe_gated_switch() {
    use str0m::{format::Codec, rtp::ExtensionValues};
    let mut n = Network::new(40_000_000);
    let a = n.add_layers(true, caps(), VideoCodec::H264, &["lo", "hi"]);
    let b = n.add(false);
    let c = n.add(false);
    for (receiver, rid) in [(b, "lo"), (c, "hi")] {
        n.signal(
            receiver,
            VoiceSignal::Subscribe {
                publisher: n.peers[a].key,
                source_mid: n.peers[a].video.to_string(),
                receiver_mid: n.peers[receiver].video.to_string(),
                rid: Some(rid.into()),
            },
        )
        .unwrap();
    }
    let mid = n.peers[a].video;
    let pt = n.peers[a]
        .rtc
        .codec_config()
        .params()
        .iter()
        .find(|p| p.spec().codec == Codec::H264)
        .unwrap()
        .pt();
    n.clear();
    for seq in 1..=6 {
        for (rid, tag) in [("lo", 11), ("hi", 22)] {
            let ssrc = n.peers[a]
                .rtc
                .direct_api()
                .stream_tx_by_mid(mid, Some(rid.into()))
                .unwrap()
                .ssrc();
            n.collect_peer(a);
            n.write(
                a,
                ssrc,
                pt,
                seq,
                seq as u32 * 3000,
                &[0x65, tag, 9],
                true,
                ExtensionValues::default(),
            );
        }
        n.advance(Duration::from_millis(30));
    }
    for (receiver, own, other) in [(b, 11, 22), (c, 22, 11)] {
        assert!(n.peers[receiver]
            .events
            .iter()
            .any(|e| matches!(e,Event::RtpPacket(p) if p.payload.as_ref()==[0x65,own,9])));
        assert!(!n.peers[receiver]
            .events
            .iter()
            .any(|e| matches!(e,Event::RtpPacket(p) if p.payload.as_ref()==[0x65,other,9])));
    }
    n.signal(
        b,
        VoiceSignal::Subscribe {
            publisher: n.peers[a].key,
            source_mid: mid.to_string(),
            receiver_mid: n.peers[b].video.to_string(),
            rid: Some("hi".into()),
        },
    )
    .unwrap();
    n.clear();
    let ssrc = n.peers[a]
        .rtc
        .direct_api()
        .stream_tx_by_mid(mid, Some("hi".into()))
        .unwrap()
        .ssrc();
    n.collect_peer(a);
    n.write(
        a,
        ssrc,
        pt,
        7,
        21000,
        &[0x41, 33],
        true,
        ExtensionValues::default(),
    );
    n.advance(Duration::from_millis(50));
    assert!(!n.peers[b]
        .events
        .iter()
        .any(|e| matches!(e, Event::RtpPacket(_))));
    n.write(
        a,
        ssrc,
        pt,
        8,
        24000,
        &[0x65, 44],
        true,
        ExtensionValues::default(),
    );
    n.advance(Duration::from_millis(50));
    assert!(n.peers[b]
        .events
        .iter()
        .any(|e| matches!(e,Event::RtpPacket(p) if p.payload.as_ref()==[0x65,44])));
}

#[test]
fn advanced_codecs_require_every_participant_and_hardware_capability() {
    use str0m::{format::Codec, rtp::ExtensionValues};
    for (codec, wire, payload) in [
        (VideoCodec::Hevc, Codec::H265, vec![0x26, 1, 9]),
        (VideoCodec::Av1, Codec::Av1, vec![0x08, 1, 9]),
    ] {
        let mut n = Network::new(40_000_000);
        let caps = VoiceCapabilities {
            video: vec![VideoCodec::H264, codec],
            hardware_decode: vec![VideoCodec::H264, codec],
        };
        let a = n.add_caps(true, caps.clone(), codec);
        let b = n.add_caps(false, caps, codec);
        n.subscribe(b, a, TrackKind::Camera);
        n.clear();
        let mid = n.peers[a].video;
        let pt = n.peers[a]
            .rtc
            .codec_config()
            .params()
            .iter()
            .find(|p| p.spec().codec == wire)
            .unwrap()
            .pt();
        let ssrc = n.peers[a]
            .rtc
            .direct_api()
            .stream_tx_by_mid(mid, None)
            .unwrap()
            .ssrc();
        n.collect_peer(a);
        for seq in 1..=4 {
            n.write(
                a,
                ssrc,
                pt,
                seq,
                seq as u32 * 3000,
                &payload,
                true,
                ExtensionValues::default(),
            );
            n.advance(Duration::from_millis(20));
        }
        assert!(n.peers[b]
            .events
            .iter()
            .any(|e| matches!(e,Event::RtpPacket(p) if p.payload.as_ref()==payload)));
        let _c = n.add(false);
        assert!(n.signals.iter().any(|(to, e)| *to == n.peers[a].key
            && matches!(
                e,
                VoiceEvent::CodecRequired {
                    codec: VideoCodec::H264,
                    ..
                }
            )));
        n.clear();
        n.write(
            a,
            ssrc,
            pt,
            5,
            15000,
            &payload,
            true,
            ExtensionValues::default(),
        );
        n.advance(Duration::from_millis(30));
        assert!(!n.peers[b]
            .events
            .iter()
            .any(|e| matches!(e, Event::RtpPacket(_))));
    }
}

#[test]
fn stopping_share_invalidates_control_and_releases_held_input() {
    use str0m::{change::SdpAnswer, media::Direction};
    let mut n = Network::new(40_000_000);
    let a = n.add(true);
    let b = n.add(false);
    n.subscribe(b, a, TrackKind::Screen);
    let sharer = n.peers[a].key;
    n.signal(b, VoiceSignal::RequestControl { sharer }).unwrap();
    n.signal(
        a,
        VoiceSignal::DecideControl {
            requester: n.peers[b].key,
            allow: true,
        },
    )
    .unwrap();
    let id = grant(&n);
    n.data(
        b,
        ControlData::Input {
            sharer,
            grant: id,
            sequence: 1,
            input: ControlInput::Key {
                usage: 4,
                down: true,
            },
        },
    );
    let screen = n.peers[a].screen;
    let mut api = n.peers[a].rtc.sdp_api();
    api.set_direction(screen, Direction::Inactive);
    let (offer, pending) = api.apply().unwrap();
    n.collect_peer(a);
    n.signal(
        a,
        VoiceSignal::Offer {
            sdp: offer.to_sdp_string(),
            tracks: vec![],
        },
    )
    .unwrap();
    let answer = n
        .signals
        .iter()
        .rev()
        .find_map(|(to, e)| {
            if *to == sharer {
                if let VoiceEvent::Answer { sdp } = e {
                    Some(sdp.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap();
    n.peers[a]
        .rtc
        .sdp_api()
        .accept_answer(pending, SdpAnswer::from_sdp_string(&answer).unwrap())
        .unwrap();
    n.collect_peer(a);
    n.advance(Duration::from_millis(50));
    assert_eq!(n.sfu.route_count(), 0);
    assert!(n.peers[a]
        .control
        .contains(&ControlData::ReleaseAll { grant: id }));
    let count = n.peers[a].control.len();
    n.data(b, input(sharer, id, 2));
    assert_eq!(n.peers[a].control.len(), count);
}

#[test]
fn browser_sized_rtp_passes_and_custom_path_mtu_is_enforced() {
    let mut n = Network::new(40_000_000);
    let a = n.add(true);
    let b = n.add(false);
    n.subscribe(b, a, TrackKind::Screen);
    let mut payload = vec![9; 1180];
    payload[0] = 0x65;
    n.send(a, TrackKind::Screen, 1, 1500, &payload, true);
    n.advance(Duration::from_millis(100));
    assert!(n.peers[b]
        .events
        .iter()
        .any(|e| matches!(e,Event::RtpPacket(p) if p.payload.as_ref()==payload)));
    assert!(n.max_rtp > 1200 && n.max_rtp <= 1232, "{}", n.max_rtp);
    n.signal(a, VoiceSignal::Stats).unwrap();
    assert!(n
        .signals
        .iter()
        .any(|(_, e)| matches!(e,VoiceEvent::Stats(s) if s.mtu_dropped_packets==0)));
    let mut ipv4 = Network::with_mtu(
        40_000_000,
        std::sync::Arc::new(|_: VoiceSession, _: Id| true),
        1252,
    );
    let sender = ipv4.add(true);
    let receiver = ipv4.add(false);
    ipv4.subscribe(receiver, sender, TrackKind::Screen);
    let mut payload = vec![8; 1200];
    payload[0] = 0x65;
    ipv4.send(sender, TrackKind::Screen, 1, 1500, &payload, true);
    ipv4.advance(Duration::from_millis(100));
    assert!(ipv4.peers[receiver]
        .events
        .iter()
        .any(|e| matches!(e,Event::RtpPacket(p) if p.payload.as_ref()==payload)));
    assert!(ipv4.max_rtp > 1232 && ipv4.max_rtp <= 1252);
    n.clear();
    n.send(a, TrackKind::Screen, 2, 3000, &payload, true);
    n.advance(Duration::from_millis(100));
    assert!(!n.peers[b]
        .events
        .iter()
        .any(|e| matches!(e,Event::RtpPacket(p) if p.payload.as_ref()==payload)));
    n.signal(a, VoiceSignal::Stats).unwrap();
    assert!(n
        .signals
        .iter()
        .any(|(_, e)| matches!(e,VoiceEvent::Stats(s) if s.mtu_dropped_packets>0)));
}
