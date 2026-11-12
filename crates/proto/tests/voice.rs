use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;
use sticktalk_proto::*;
fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: T) {
    assert_eq!(
        serde_json::from_slice::<T>(&serde_json::to_vec(&value).unwrap()).unwrap(),
        value
    );
    assert_eq!(
        rmp_serde::from_slice::<T>(&rmp_serde::to_vec_named(&value).unwrap()).unwrap(),
        value
    );
}
fn session() -> VoiceSession {
    VoiceSession {
        user_id: Id(100),
        connection_id: Id(200),
    }
}
#[test]
fn voice_contract_shapes_and_legacy_envelope() {
    let track = PublishedTrack {
        mid: "screen".into(),
        kind: TrackKind::Screen,
        codec: Some(VideoCodec::H264),
        layers: vec!["lo".into(), "hi".into()],
        surface: Some(ShareSurface::Window),
    };
    for signal in [
        VoiceSignal::Join {
            channel_id: Id(1),
            capabilities: VoiceCapabilities {
                video: vec![VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1],
                hardware_decode: vec![VideoCodec::H264],
            },
        },
        VoiceSignal::Leave,
        VoiceSignal::Offer {
            sdp: "v=0\r\n".into(),
            tracks: vec![track.clone()],
        },
        VoiceSignal::Ice {
            candidate: "candidate:1 1 UDP 1 100.1.2.3 50000 typ host".into(),
        },
        VoiceSignal::State {
            muted: true,
            deafened: false,
        },
        VoiceSignal::Subscribe {
            publisher: session(),
            source_mid: "screen".into(),
            receiver_mid: "view".into(),
            rid: Some("lo".into()),
        },
        VoiceSignal::Unsubscribe {
            receiver_mid: "view".into(),
        },
        VoiceSignal::RequestKeyframe {
            receiver_mid: "view".into(),
            fir: true,
        },
        VoiceSignal::RequestControl { sharer: session() },
        VoiceSignal::DecideControl {
            requester: session(),
            allow: true,
        },
        VoiceSignal::RevokeControl,
        VoiceSignal::Stats,
    ] {
        roundtrip(signal);
    }
    let participant = VoiceParticipant {
        session: session(),
        muted: false,
        deafened: true,
        tracks: vec![track],
    };
    for event in [
        VoiceEvent::Joined {
            channel_id: Id(1),
            participants: vec![participant.clone()],
            max_rtp_bytes: 1200,
        },
        VoiceEvent::Participant(participant),
        VoiceEvent::Left { session: session() },
        VoiceEvent::Answer {
            sdp: "v=0\r\n".into(),
        },
        VoiceEvent::Speaking {
            session: session(),
            speaking: true,
            level_dbov: -30,
        },
        VoiceEvent::Bitrate {
            mid: "screen".into(),
            bps: 8_000_000,
        },
        VoiceEvent::CodecRequired {
            mid: "screen".into(),
            codec: VideoCodec::H264,
        },
        VoiceEvent::ControlRequested {
            requester: session(),
            timeout_ms: 30_000,
        },
        VoiceEvent::ControlDenied { sharer: session() },
        VoiceEvent::ControlGranted {
            sharer: session(),
            controller: session(),
            grant: u64::MAX,
        },
        VoiceEvent::ControlRevoked {
            sharer: session(),
            grant: 1,
        },
        VoiceEvent::Stats(VoiceStats {
            tx_bps: 1,
            rx_bps: 2,
            tx_fps_milli: 60000,
            rx_fps_milli: 30000,
            loss_per_million: Some(1234),
            rtt_ms: None,
            path: MediaPath::Relayed,
            cap_dropped_packets: 12,
            queue_dropped_packets: 0,
            mtu_dropped_packets: 1,
        }),
        VoiceEvent::Ack,
        VoiceEvent::Error {
            code: ErrorCode::Forbidden,
            message: "not authorized".into(),
        },
    ] {
        roundtrip(event);
    }
    for input in [
        ControlInput::Move { x: 0, y: u16::MAX },
        ControlInput::Button {
            button: 4,
            down: true,
        },
        ControlInput::Wheel { x: -100, y: 200 },
        ControlInput::Key {
            usage: 0xe0,
            down: false,
        },
    ] {
        roundtrip(ControlData::Input {
            sharer: session(),
            grant: 99,
            sequence: u64::MAX,
            input,
        });
    }
    roundtrip(ControlData::ReleaseAll { grant: 99 });
    roundtrip(ChannelKind::Voice);
    // Exact pre-Phase-2 map shape continues to deserialize and re-serialize.
    let legacy = serde_json::json!({"type":"voice","data":{"nonce":13,"version":0,"kind":"reserved","payload":[0,127,255]}});
    let decoded: ClientMsg = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
}
