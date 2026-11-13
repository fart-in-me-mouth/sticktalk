use super::*;
use proptest::prelude::*;
use std::sync::OnceLock;

fn cases() -> proptest::test_runner::Config {
    proptest::test_runner::Config {
        cases: std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3072),
        ..Default::default()
    }
}
fn key(n: u64) -> VoiceSession {
    VoiceSession {
        user_id: Id(n),
        connection_id: Id(n + 100),
    }
}
fn sfu(config: Config) -> Sfu {
    let mut s = Sfu::new(
        Config {
            ports: 53200..=53999,
            ..config
        },
        Arc::new(|_: VoiceSession, _: Id| true),
    )
    .unwrap();
    static CERT: OnceLock<str0m::crypto::dtls::DtlsCert> = OnceLock::new();
    let cert = CERT.get_or_init(|| {
        str0m::crypto::from_feature_flags()
            .dtls_provider
            .generate_certificate()
            .unwrap()
    });
    s.rtc_template = rtc_config().set_dtls_cert(cert.clone());
    s
}
fn join(s: &mut Sfu, n: u64, room: u64) {
    s.handle_signal(
        key(n),
        1,
        VoiceSignal::Join {
            channel_id: Id(room),
            capabilities: VoiceCapabilities {
                video: vec![VideoCodec::H264],
                hardware_decode: vec![],
            },
        },
        s.now,
    )
    .unwrap();
    s.drain().for_each(drop);
}

#[test]
fn viewer_pli_and_fir_are_aggregated_per_publisher_source_once_per_second() {
    let mut s = sfu(Config::default());
    join(&mut s, 1, 10);
    let mid: Mid = "video".into();
    s.peers
        .get_mut(&key(1))
        .unwrap()
        .rtc
        .direct_api()
        .declare_media(mid, MediaKind::Video);
    s.collect(key(1)).unwrap();
    s.peers
        .get_mut(&key(1))
        .unwrap()
        .rtc
        .direct_api()
        .expect_stream_rx(1234.into(), None, mid, None);
    s.collect(key(1)).unwrap();
    for _ in 0..100 {
        s.keyframe(key(1), mid, None, KeyframeRequestKind::Pli)
            .unwrap();
        s.keyframe(key(1), mid, None, KeyframeRequestKind::Fir)
            .unwrap();
    }
    assert_eq!(s.peers[&key(1)].keyframe_forwarded, 1);
    assert_eq!(s.peers[&key(1)].keyframe_suppressed, 199);
    s.now += Duration::from_millis(999);
    s.keyframe(key(1), mid, None, KeyframeRequestKind::Pli)
        .unwrap();
    assert_eq!(s.peers[&key(1)].keyframe_forwarded, 1);
    s.now += Duration::from_millis(1);
    s.keyframe(key(1), mid, None, KeyframeRequestKind::Fir)
        .unwrap();
    assert_eq!(s.peers[&key(1)].keyframe_forwarded, 2);
    assert_eq!(s.peers[&key(1)].keyframe_times.len(), 1);
}
fn fixture() -> &'static str {
    static SDP: OnceLock<String> = OnceLock::new();
    SDP.get_or_init(|| {
        let mut rtc = rtc_config().build(Instant::now());
        while !matches!(rtc.poll_output().unwrap(), Output::Timeout(_)) {}
        let mut api = rtc.sdp_api();
        api.add_media(MediaKind::Video, Direction::RecvOnly, None, None, None);
        api.apply().unwrap().0.to_sdp_string()
    })
}
fn arbitrary_sdp() -> impl Strategy<Value = String> {
    prop_oneof![
        ".{0,2048}",
        (0u8..8, any::<u16>()).prop_map(|(mutation, n)| {
            let base = fixture();
            match mutation {
                0 => base.to_owned(),
                1 => base.replace("a=rtpmap:112", &format!("a=rtpmap:{n}")),
                2 => format!("{base}a=ssrc:{n} cname:x\r\n"),
                3 => format!("{base}a=mid:{}\r\n", "x".repeat(n as usize % 40)),
                4 => format!("{base}a=fmtp:112 apt={n}\r\n"),
                5 => base.replace("a=recvonly", "a=sendrecv"),
                6 => format!("{base}a=rid:{} send\r\n", "r".repeat(n as usize % 30)),
                _ => format!("{base}a=candidate:x 1 UDP {n} 127.0.0.1 45678 typ host\r\n"),
            }
        })
    ]
}
fn no_panic<T>(result: &Result<T, Error>) {
    assert!(
        !matches!(result, Err(Error::PeerFailed)),
        "upstream parser panicked"
    );
}

proptest! {
    #![proptest_config(cases())]
    #[test]
    fn prop_sdp_offers(sdp in arbitrary_sdp()) {
        let result = crate::boundary::run(|| validation::offer(&sdp));
        no_panic(&result);
    }
    #[test]
    fn prop_sdp_answers(sdp in arbitrary_sdp()) {
        let result = crate::boundary::run(|| validation::answer(&sdp));
        no_panic(&result);
    }
    #[test]
    fn prop_ice_candidates(value in prop_oneof![".{0,1100}", (any::<u16>(), any::<u32>(), any::<bool>()).prop_map(|(port, priority, udp)| format!("candidate:x 1 {} {priority} 127.0.0.1 {port} typ host", if udp {"UDP"} else {"TCP"}))]) {
        let result = crate::boundary::run(|| validation::candidate(&value));
        no_panic(&result);
        if let Ok(candidate) = result { prop_assert_eq!(candidate.proto(), Protocol::Udp); prop_assert_ne!(candidate.addr().port(), 0); }
    }
    #[test]
    fn prop_control_parser(bytes in proptest::collection::vec(any::<u8>(), 0..600), x in -100i64..66000, y in -100i64..66000) {
        let result = crate::boundary::run(|| validation::control(&bytes));
        no_panic(&result);
        let data = serde_json::json!({"type":"input","data":{"sharer":{"user_id":1,"connection_id":101},"grant":1,"sequence":1,"input":{"type":"move","data":{"x":x,"y":y}}}});
        let data = rmp_serde::to_vec_named(&data).unwrap();
        let result = validation::control(&data);
        prop_assert_eq!(result.is_ok(), (0..=65535).contains(&x) && (0..=65535).contains(&y));
    }
    #[test]
    fn prop_control_grants_rate_and_bounds(authorized in any::<bool>(), correct_grant in any::<bool>(), button in any::<u8>(), wheel in any::<i16>(), x in any::<u16>(), y in any::<u16>()) {
        let now = Instant::now();
        let mut grant = Grant::new(key(1), now);
        let id = grant.id;
        let sender = if authorized {key(1)} else {key(2)};
        let result = grant.accept(sender, if correct_grant {id} else {id.wrapping_add(1)}, 0, &ControlInput::Move {x,y}, now);
        prop_assert_eq!(result.is_ok(), authorized && correct_grant);
        let mut grant = Grant::new(key(1), now);
        let id = grant.id;
        prop_assert_eq!(grant.accept(key(1), id, 1, &ControlInput::Button {button, down:true}, now).is_ok(), button <= 4);
        prop_assert_eq!(grant.accept(key(1), id, 2, &ControlInput::Wheel {x:wheel,y:0}, now).is_ok(), wheel.unsigned_abs() <= 4096);
        let mut grant = Grant::new(key(1), now);
        let id = grant.id;
        for seq in 0..1000 { prop_assert!(grant.accept(key(1),id,seq,&ControlInput::Move {x,y},now).is_ok(), "allowed event"); }
        prop_assert!(matches!(grant.accept(key(1),id,1000,&ControlInput::Move {x,y},now), Err(Error::Limit)), "rate rejected");
        prop_assert!(grant.accept(key(1),id,1000,&ControlInput::Move {x,y},now+Duration::from_secs(1)).is_ok(), "new rate window");
        prop_assert!(matches!(grant.accept(key(1),id,1000,&ControlInput::Move {x,y},now+Duration::from_secs(1)), Err(Error::Forbidden)), "replay rejected");
    }
    #[test]
    fn prop_layer_selection(source in prop_oneof![".{0,25}", Just("source".to_owned())], receiver in prop_oneof![".{0,25}", Just("receiver".to_owned())], rid in proptest::option::of(prop_oneof![".{0,25}", Just("low".to_owned()), Just("high".to_owned())])) {
        let layers = vec!["low".into(), "high".into()];
        let result = validation::layer(&source,&receiver,&layers,rid.as_deref());
        prop_assert_eq!(result.is_ok(), valid_id(&source) && valid_id(&receiver) && matches!(rid.as_deref(), Some("low" | "high")));
    }
    #[test]
    fn prop_renegotiation_sequences(operations in proptest::collection::vec((0u8..5, any::<u16>()), 1..12)) {
        let mut s = sfu(Config::default());
        join(&mut s,1,1);
        join(&mut s,2,2);
        let start = s.now;
        for (step,(op, n)) in operations.into_iter().enumerate() {
            let signal = match op {
                0 => VoiceSignal::Offer {sdp: fixture().into(), tracks:vec![]},
                1 => VoiceSignal::Offer {sdp: fixture().replace("a=rtpmap:112", &format!("a=rtpmap:{n}")), tracks:vec![]},
                2 => VoiceSignal::Ice {candidate:format!("candidate:x 1 UDP 1 127.0.0.1 {n} typ host")},
                3 => VoiceSignal::Subscribe {publisher:key(2),source_mid:"x".into(),receiver_mid:"x".into(),rid:Some(n.to_string())},
                _ => VoiceSignal::State {muted:true,deafened:false},
            };
            let result = s.handle_signal(key(1), 2, signal, start+Duration::from_millis(step as u64 * 100));
            no_panic(&result);
            prop_assert!(!matches!(result,Err(Error::Backpressure)));
            s.drain().for_each(drop);
            prop_assert!(s.sessions().contains(&key(2)));
        }
    }
}

#[test]
fn peer_boundary_discards_poisoned_rtc_and_preserves_rooms() {
    let mut s = sfu(Config::default());
    join(&mut s, 1, 1);
    join(&mut s, 2, 1);
    join(&mut s, 3, 2);
    let result: Result<(), Error> =
        s.peer_operation(key(1), 77, |_| panic!("private SDP must never be logged"));
    assert!(matches!(result, Err(Error::PeerFailed)));
    assert_eq!(s.sessions(), vec![key(2), key(3)]);
    let out: Vec<_> = s.drain().collect();
    let error = out.iter().position(|out| matches!(out,Out::Signal {to,nonce:77,event:VoiceEvent::Error {code:ErrorCode::Unavailable,message}} if *to==key(1) && !message.contains("private SDP"))).unwrap();
    let left = out
        .iter()
        .position(
            |out| matches!(out,Out::Signal {to,event:VoiceEvent::Left {..},..} if *to==key(1)),
        )
        .unwrap();
    assert!(
        error < left,
        "host must see error before own-session Left removes routing"
    );
    for n in [2, 3] {
        s.handle_signal(key(n), 78, VoiceSignal::Stats, s.now)
            .unwrap();
        s.drain().for_each(drop);
    }
}

#[test]
fn limits_are_atomic_and_bounded() {
    let mut s = sfu(Config {
        max_sessions: 3,
        max_channel_sessions: 2,
        max_peer_tracks: 1,
        max_room_tracks: 1,
        ..Default::default()
    });
    join(&mut s, 1, 1);
    join(&mut s, 2, 1);
    join(&mut s, 3, 2);
    let result = s.handle_signal(
        key(4),
        1,
        VoiceSignal::Join {
            channel_id: Id(2),
            capabilities: VoiceCapabilities {
                video: vec![VideoCodec::H264],
                hardware_decode: vec![],
            },
        },
        s.now,
    );
    assert!(matches!(result, Err(Error::Limit)));
    s.drain().for_each(drop);
    let track = PublishedTrack {
        mid: "v".into(),
        kind: TrackKind::Camera,
        codec: Some(VideoCodec::H264),
        layers: vec![],
        surface: None,
    };
    assert!(matches!(
        s.offer(
            key(1),
            2,
            fixture().into(),
            vec![track.clone(), track.clone()]
        ),
        Err(Error::Limit)
    ));
    s.peers.get_mut(&key(2)).unwrap().tracks = vec![track.clone()];
    assert!(matches!(
        s.offer(key(1), 2, fixture().into(), vec![track]),
        Err(Error::Limit)
    ));
    for n in 0..validation::MAX_ICE {
        let signal = VoiceSignal::Ice {
            candidate: format!("candidate:x 1 UDP 1 127.0.0.1 {} typ host", 40000 + n),
        };
        s.handle_signal(key(1), 3, signal, s.now + Duration::from_millis(20))
            .unwrap();
        s.drain().for_each(drop);
        s.now += Duration::from_millis(20);
    }
    assert!(matches!(
        s.signal(
            key(1),
            3,
            VoiceSignal::Ice {
                candidate: "candidate:x 1 UDP 1 127.0.0.1 49999 typ host".into()
            }
        ),
        Err(Error::Limit)
    ));
    assert_eq!(s.peers[&key(1)].candidates.len(), validation::MAX_ICE);
    assert!(matches!(validation::control(&[0; 513]), Err(Error::Limit)));
}

#[test]
fn renegotiations_use_rolling_minute_and_count_invalid_attempts() {
    let mut s = sfu(Config::default());
    join(&mut s, 1, 1);
    let start = s.now;
    for _ in 0..validation::MAX_RENEGOTIATIONS {
        assert!(matches!(
            s.offer(key(1), 2, "invalid".into(), vec![]),
            Err(Error::Invalid)
        ));
    }
    assert!(matches!(
        s.offer(key(1), 2, fixture().into(), vec![]),
        Err(Error::Limit)
    ));
    s.now = start + Duration::from_secs(60);
    assert!(s.offer(key(1), 2, fixture().into(), vec![]).is_ok());
    assert_eq!(s.peers[&key(1)].negotiation_times.len(), 1);
}

#[test]
fn raw_sdp_and_lifetime_limits() {
    assert!(matches!(
        validation::offer(&"x".repeat(65537)),
        Err(Error::Limit)
    ));
    let many_ssrc = format!(
        "{}{}",
        fixture(),
        (1..=129)
            .map(|n| format!("a=ssrc:{n} cname:x\r\n"))
            .collect::<String>()
    );
    assert!(matches!(validation::offer(&many_ssrc), Err(Error::Limit)));
    let many_media = format!(
        "{}{}",
        fixture(),
        "m=video 9 UDP/TLS/RTP/SAVPF 112\r\na=mid:x\r\n".repeat(33)
    );
    assert!(validation::offer(&many_media).is_err());
    let mut s = sfu(Config::default());
    join(&mut s, 1, 1);
    s.peers.get_mut(&key(1)).unwrap().negotiated_mids = (0..33)
        .map(|n| (format!("old{n}"), "video".into()))
        .collect();
    assert!(matches!(
        s.offer(key(1), 1, fixture().into(), vec![]),
        Err(Error::Limit)
    ));
    let mut bytes = rmp_serde::to_vec_named(&VoiceSignal::Stats).unwrap();
    bytes.push(0);
    assert!(matches!(
        validation::decode::<VoiceSignal>(&bytes),
        Err(Error::Invalid)
    ));
}

#[test]
fn peer_and_room_wire_budgets_bound_bytes_with_1232_mtu() {
    let mut s = sfu(Config {
        peer_video_egress_bps: 96_000,
        room_video_egress_bps: 192_000,
        ..Default::default()
    });
    join(&mut s, 1, 1);
    join(&mut s, 2, 1);
    join(&mut s, 3, 2);
    let now = s.now;
    let mut peer_bytes = 0;
    let mut room_bytes = 0;
    for _ in 0..100 {
        if s.peers
            .get_mut(&key(1))
            .unwrap()
            .video_budget
            .take(1232, now)
        {
            peer_bytes += 1232;
        }
        if s.room_budgets.get_mut(&Id(1)).unwrap().take(1232, now) {
            room_bytes += 1232;
        }
    }
    assert_eq!(peer_bytes, 1232);
    assert_eq!(room_bytes, 1232);
    assert!(s.room_budgets.get_mut(&Id(2)).unwrap().take(1232, now));
    assert!(!s.room_budgets.get_mut(&Id(1)).unwrap().take(1232, now));
    assert!(s
        .peers
        .get_mut(&key(1))
        .unwrap()
        .video_budget
        .take(1232, now + Duration::from_secs(1)));
    assert_eq!(s.config.max_datagram_bytes, 1232);
}

#[test]
fn aggregate_room_ice_offer_and_move_limits_do_not_touch_other_rooms() {
    let mut s = sfu(Config {
        max_room_ice_candidates: 1,
        max_room_renegotiations_per_minute: 2,
        max_room_tracks: 1,
        ..Default::default()
    });
    join(&mut s, 1, 1);
    join(&mut s, 2, 1);
    join(&mut s, 3, 2);
    let candidate = VoiceSignal::Ice {
        candidate: "candidate:x 1 UDP 1 127.0.0.1 40001 typ host".into(),
    };
    s.signal(key(1), 1, candidate.clone()).unwrap();
    s.drain().for_each(drop);
    assert!(matches!(
        s.signal(key(2), 1, candidate.clone()),
        Err(Error::Limit)
    ));
    s.drain().for_each(drop);
    s.signal(key(3), 1, candidate).unwrap();
    s.drain().for_each(drop);
    for n in [1, 2] {
        assert!(matches!(
            s.offer(key(n), 2, "bad".into(), vec![]),
            Err(Error::Invalid)
        ));
    }
    assert!(matches!(
        s.offer(key(1), 2, fixture().into(), vec![]),
        Err(Error::Limit)
    ));
    assert!(matches!(
        s.offer(key(3), 2, "bad".into(), vec![]),
        Err(Error::Invalid)
    ));
    let track = PublishedTrack {
        mid: "v".into(),
        kind: TrackKind::Camera,
        codec: Some(VideoCodec::H264),
        layers: vec![],
        surface: None,
    };
    s.peers.get_mut(&key(1)).unwrap().tracks = vec![track.clone()];
    s.peers.get_mut(&key(3)).unwrap().tracks = vec![track];
    assert!(matches!(s.move_session(key(3), Id(1)), Err(Error::Limit)));
    assert_eq!(s.peers[&key(3)].channel, Id(2));
    assert_eq!(s.session_count(), 3);
}

#[test]
fn answers_are_not_client_signals_and_control_requires_a_grant() {
    let bytes =
        rmp_serde::to_vec_named(&serde_json::json!({"type":"answer","data":{"sdp":fixture()}}))
            .unwrap();
    assert!(matches!(
        validation::decode::<VoiceSignal>(&bytes),
        Err(Error::Invalid)
    ));
    let bytes = rmp_serde::to_vec_named(&ControlData::ReleaseAll { grant: 1 }).unwrap();
    assert!(matches!(validation::control(&bytes), Err(Error::Invalid)));
    let mut s = sfu(Config::default());
    join(&mut s, 1, 1);
    join(&mut s, 2, 1);
    // A negotiated channel alone is never consent; valid Input is denied
    // without an explicit sharer-bound grant, even with correct coordinates.
    let mut api = s.peers.get_mut(&key(1)).unwrap().rtc.sdp_api();
    let id = api.add_channel_with_config(str0m::channel::ChannelConfig {
        label: CONTROL_CHANNEL_LABEL.into(),
        ..Default::default()
    });
    drop(api);
    s.peers.get_mut(&key(1)).unwrap().control = Some(id);
    s.peers.get_mut(&key(2)).unwrap().tracks = vec![PublishedTrack {
        mid: "screen".into(),
        kind: TrackKind::Screen,
        codec: Some(VideoCodec::H264),
        layers: vec![],
        surface: Some(ShareSurface::Monitor),
    }];
    let message = ControlData::Input {
        sharer: key(2),
        grant: 1,
        sequence: 1,
        input: ControlInput::Move { x: 0, y: 65535 },
    };
    let data = str0m::channel::ChannelData {
        id,
        binary: true,
        data: rmp_serde::to_vec_named(&message).unwrap(),
    };
    assert!(matches!(
        s.control_data(key(1), data),
        Err(Error::Forbidden)
    ));
}
