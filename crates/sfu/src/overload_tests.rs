use super::*;
#[test]
fn s5_full_internal_output_reserves_cleanup_and_preserves_other_sessions() {
    let mut s = Sfu::new(Config::default(), Arc::new(|_: VoiceSession, _: Id| true)).unwrap();
    let a = VoiceSession {
        user_id: Id(1),
        connection_id: Id(10),
    };
    let b = VoiceSession {
        user_id: Id(2),
        connection_id: Id(20),
    };
    for key in [a, b] {
        s.handle_signal(
            key,
            1,
            VoiceSignal::Join {
                channel_id: Id(1),
                capabilities: VoiceCapabilities {
                    video: vec![VideoCodec::H264],
                    hardware_decode: vec![],
                },
            },
            Instant::now(),
        )
        .unwrap();
        s.drain().for_each(drop);
    }
    for _ in 0..4096 {
        s.out.push_back((
            0,
            Out::Signal {
                to: b,
                nonce: 0,
                event: VoiceEvent::Ack,
            },
        ));
    }
    let result: Result<(), Error> = s.peer_operation(a, 0, |_| Err(Error::Backpressure));
    assert!(matches!(result, Err(Error::Backpressure)));
    assert_eq!(s.take_overloaded(), BTreeSet::from([a]));
    let cleanup = s.overload_cleanup(a);
    assert_eq!(s.sessions(), vec![b]);
    assert_eq!(s.out.len(), 4096);
    assert!(cleanup
        .iter()
        .any(|out| matches!(out,Out::Signal{event:VoiceEvent::Left{session},..} if *session==a)));
}
