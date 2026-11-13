use std::{sync::Arc, time::Duration};
use sticktalk_sfu::{proto::*, Command, Config, Sfu};
#[tokio::test]
async fn udp_driver_correlates_bad_schema_and_shuts_down_without_polling() {
    let sfu = Sfu::new(
        Config {
            bind_ip: "127.0.0.1".parse().unwrap(),
            advertised_ip: "127.0.0.1".parse().unwrap(),
            ports: 53000..=53099,
            ..Config::default()
        },
        Arc::new(|_: VoiceSession, _: Id| true),
    )
    .unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let (out, mut received) = tokio::sync::mpsc::channel(8);
    let task = tokio::spawn(sfu.run(rx, out));
    let key = VoiceSession {
        user_id: Id(1),
        connection_id: Id(10),
    };
    tx.send(Command::Signal {
        session: key,
        envelope: VoicePayload {
            nonce: 77,
            version: 0,
            kind: "reserved".into(),
            payload: vec![],
        },
    })
    .await
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.session, key);
    assert_eq!(result.envelope.nonce, 77);
    assert!(matches!(
        rmp_serde::from_slice::<VoiceEvent>(&result.envelope.payload).unwrap(),
        VoiceEvent::Error {
            code: ErrorCode::VersionMismatch,
            ..
        }
    ));
    tx.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn s5_signaling_overload_uses_reserved_cleanup_without_stopping_actor() {
    let sfu = Sfu::new(
        Config {
            ports: 53000..=53099,
            ..Default::default()
        },
        Arc::new(|_: VoiceSession, _: Id| true),
    )
    .unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(128);
    let (out, _unread) = tokio::sync::mpsc::channel(1);
    let (_cleanup, cleanup_rx) = tokio::sync::mpsc::channel(128);
    let (revoked, mut revoked_rx) = tokio::sync::mpsc::channel(128);
    let task = tokio::spawn(sfu.run_with_control(rx, out, cleanup_rx, revoked));
    let session = VoiceSession {
        user_id: Id(1),
        connection_id: Id(10),
    };
    for nonce in 1..=1000 {
        tx.send(Command::Signal {
            session,
            envelope: VoicePayload {
                nonce,
                version: 0,
                kind: "reserved".into(),
                payload: vec![],
            },
        })
        .await
        .unwrap();
        if nonce > 1 {
            assert!(
                matches!(tokio::time::timeout(Duration::from_secs(2),revoked_rx.recv()).await.unwrap(),Some(sticktalk_sfu::ControlOut::Revoked(found)) if found==session)
            );
        }
    }
    assert!(!task.is_finished());
    tx.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
