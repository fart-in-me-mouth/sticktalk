//! Process-restart preference persistence through the actual native application.
//! Synthetic loopback account; no registry, notifications, keyring, or sound.
#![cfg(windows)]
use std::{os::windows::process::CommandExt, time::Duration};
use sticktalk::{backend, backend::Http};
use sticktalk_client_core::settings::{Density, Preferences};
use sticktalk_proto::NotificationLevel;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "opens sequential native windows on a secondary monitor"]
async fn every_non_voice_preference_survives_native_process_restart() {
    let server_dir = tempfile::tempdir().unwrap();
    let app = sticktalk_server::App::open(sticktalk_server::Config {
        operations: sticktalk_server::OperationsConfig {
            min_free_disk_gb: 0,
            ..Default::default()
        },
        browser: Default::default(),
        bind: "127.0.0.1:0".parse().unwrap(),
        data_dir: server_dir.path().into(),
        public_url: "http://localhost".into(),
        max_upload_mb: 0,
        max_total_upload_mb: 0,
        worker_id: 0,
        trusted_proxy: "127.0.0.1".parse().unwrap(),
        google_client_id: None,
        mock_google_jwks: None,
        mock_google_issuer: None,
        gifs: Default::default(),
        voice: Default::default(),
    })
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let serving_app = app.clone();
    let serving = tokio::spawn(async move { serving_app.serve(listener).await });
    let auth = backend::authenticate_unstored(
        &server,
        "persist_owner".into(),
        "correct horse battery staple".into(),
        true,
        "Preference QA".into(),
    )
    .await
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        density: Density::Compact,
        chat_font_scale: 1.25,
        saturation: 0.5,
        underline_links: false,
        developer_mode: true,
        allow_direct_messages: false,
        friend_requests: false,
        notification_level: NotificationLevel::Nothing,
        notification_sounds: true,
        start_with_windows: true,
        ..Default::default()
    };
    preferences
        .keybinds
        .insert("Quick Switcher".into(), "Alt+K".into());
    let expected = serde_json::to_value(&preferences).unwrap();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for pass in 0..2 {
        let report = home.path().join(format!("preferences-{pass}.json"));
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_sticktalk"));
        command
            .creation_flags(0x08000000)
            .args(["--size", "1280x800", "--capture"])
            .arg(home.path().join(format!("view-{pass}.png")))
            .env("LOCALAPPDATA", home.path())
            .env("STICKTALK_AGENT_WINDOW", "secondary")
            .env("STICKTALK_QA_SERVER", &server)
            .env("STICKTALK_QA_TOKEN", &auth.token)
            .env("STICKTALK_QA_PREFERENCES_REPORT", &report)
            .env_remove("STICKTALK_QA_SEND")
            .env_remove("STICKTALK_QA_CHANNEL")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if pass == 0 {
            command.env(
                "STICKTALK_QA_PREFERENCES",
                serde_json::to_string(&preferences).unwrap(),
            );
        } else {
            command.env_remove("STICKTALK_QA_PREFERENCES");
        }
        let mut child = Child(command.spawn().unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native preference gate timed out"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let actual: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
        assert_eq!(actual, expected, "native pass {pass}");
        let saved: serde_json::Value = serde_json::from_slice(
            &std::fs::read(home.path().join("StickTalk/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved, expected, "production settings file on pass {pass}");
    }
    Http::new(&server, &auth.token).unwrap().logout().await;
    app.shutdown();
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
