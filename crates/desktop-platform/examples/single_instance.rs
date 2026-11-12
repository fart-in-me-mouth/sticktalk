#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use sticktalk_desktop_platform::{
        activation::Activation,
        windows::instance::{Instance, Launch},
    };
    let dir = tempfile::tempdir()?;
    let Launch::Primary(mut primary) = Instance::launch(dir.path(), Activation::Open).await? else {
        panic!("first instance")
    };
    let path = dir.path().to_owned();
    let secondary = tokio::spawn(async move {
        Instance::launch(&path, Activation::Invite("demo-invite".into())).await
    });
    println!(
        "Received {:?}; no actual window is focused",
        primary.receive().await?
    );
    assert!(matches!(secondary.await??, Launch::Forwarded));
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    println!("Windows-only IPC demo");
}
