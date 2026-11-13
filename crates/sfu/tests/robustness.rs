use std::{sync::Arc, time::Instant};
use sticktalk_sfu::{proto::*, rtc_config, Config, Out, Sfu};
use str0m::{
    change::{SdpAnswer, SdpOffer},
    media::{Direction, MediaKind},
    Output, Rtc,
};

fn drained(rtc: &mut Rtc) {
    while !matches!(rtc.poll_output().unwrap(), Output::Timeout(_)) {}
}

// Pin the upstream failure separately from the fixed SFU regression. This is
// intentionally an in-process, socket-free reproduction of the old config.
#[test]
#[should_panic(expected = "Pt locked multiple times")]
fn upstream_duplicate_high_profiles_claim_one_sparse_remote_pt() {
    let now = Instant::now();
    let mut old = rtc_config();
    old.codec_config()
        .add_h264(114.into(), Some(115.into()), true, 0x64001f);
    let mut answerer = old.build(now);
    drained(&mut answerer);
    let mut config = rtc_config();
    config.codec_config().clear();
    config.codec_config().enable_opus(true, false);
    config
        .codec_config()
        .add_h264(112.into(), Some(113.into()), true, 0x640033);
    let mut client = config.build(now);
    drained(&mut client);
    for video in [false, true] {
        let mut api = client.sdp_api();
        api.add_media(
            if video {
                MediaKind::Video
            } else {
                MediaKind::Audio
            },
            Direction::RecvOnly,
            None,
            None,
            None,
        );
        let (offer, pending) = api.apply().unwrap();
        drained(&mut client);
        let answer = answerer.sdp_api().accept_offer(offer).unwrap();
        drained(&mut answerer);
        client.sdp_api().accept_answer(pending, answer).unwrap();
        drained(&mut client);
    }
}

#[test]
fn sparse_high_only_renegotiation() {
    let now = Instant::now();
    let mut config = rtc_config();
    config.codec_config().clear();
    config.codec_config().enable_opus(true, false);
    config
        .codec_config()
        .add_h264(112.into(), Some(113.into()), true, 0x640033);
    let mut client = config.build(now);
    drained(&mut client);
    let mut server = Sfu::new(
        Config {
            ports: 53100..=53199,
            ..Default::default()
        },
        Arc::new(|_: VoiceSession, _: Id| true),
    )
    .unwrap();
    let key = VoiceSession {
        user_id: Id(1),
        connection_id: Id(11),
    };
    server
        .handle_signal(
            key,
            1,
            VoiceSignal::Join {
                channel_id: Id(1),
                capabilities: VoiceCapabilities {
                    video: vec![VideoCodec::H264],
                    hardware_decode: vec![VideoCodec::H264],
                },
            },
            now,
        )
        .unwrap();
    server.drain().for_each(drop);
    for video in [false, true] {
        let mut api = client.sdp_api();
        api.add_media(
            if video {
                MediaKind::Video
            } else {
                MediaKind::Audio
            },
            Direction::RecvOnly,
            None,
            None,
            None,
        );
        let (offer, pending) = api.apply().unwrap();
        drained(&mut client);
        let sdp = offer.to_sdp_string();
        assert!(SdpOffer::from_sdp_string(&sdp).is_ok());
        server
            .handle_signal(
                key,
                2,
                VoiceSignal::Offer {
                    sdp,
                    tracks: vec![],
                },
                now,
            )
            .unwrap();
        let answer = server
            .drain()
            .find_map(|out| match out {
                Out::Signal {
                    event: VoiceEvent::Answer { sdp },
                    ..
                } => Some(sdp),
                _ => None,
            })
            .unwrap();
        client
            .sdp_api()
            .accept_answer(pending, SdpAnswer::from_sdp_string(&answer).unwrap())
            .unwrap();
        drained(&mut client);
    }
}
