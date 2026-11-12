#![cfg(windows)]
use sticktalk_desktop_platform::{
    activation::Activation,
    windows::instance::{Instance, Launch},
};

#[tokio::test]
async fn second_launch_and_invite_forward_without_any_window() {
    let dir = tempfile::tempdir().unwrap();
    let Launch::Primary(mut primary) = Instance::launch(dir.path(), Activation::Open)
        .await
        .unwrap()
    else {
        panic!("first instance");
    };
    for activation in [
        Activation::Open,
        Activation::Invite("abc-123".into()),
        Activation::Message {
            channel_id: 1,
            message_id: 2,
        },
    ] {
        let path = dir.path().to_owned();
        let send = activation.clone();
        let secondary = tokio::spawn(async move {
            assert!(matches!(
                Instance::launch(&path, send).await.unwrap(),
                Launch::Forwarded
            ));
        });
        assert_eq!(primary.receive().await.unwrap(), activation);
        secondary.await.unwrap();
    }
    drop(primary);
    assert!(matches!(
        Instance::launch(dir.path(), Activation::Open)
            .await
            .unwrap(),
        Launch::Primary(_)
    ));
}
