use sticktalk_desktop_platform::updater::Updater;
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Default is offline/dry-run. Explicit --check <https-origin> opts into HTTP,
    // but never downloads, executes an installer, or restarts the app.
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("--check") {
        println!("Dry run: launch check + one 6 h timer; authenticate -> download/resume -> stage -> confirmed restart. Run cargo test -p sticktalk-desktop-platform --test updater for a local mock demo.");
        return Ok(());
    }
    let stage = std::env::temp_dir().join("sticktalk-update-demo");
    let updater = Updater::embedded(
        args.get(1).ok_or("missing origin")?,
        env!("CARGO_PKG_VERSION"),
        stage,
    )?;
    println!("{:?}", updater.check().await?.map(|v| v.manifest().clone()));
    Ok(())
}
