#![cfg_attr(windows, windows_subsystem = "windows")]
//! Installed helper. No windows, tray, sounds, browser, notification or elevation.
#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Never let an OS fault/error dialog interrupt the owner, including during
    // silent setup. Ordinary failures are reported solely through exit status.
    unsafe {
        use windows::Win32::System::Diagnostics::Debug::{
            SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX,
        };
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
    }
    use std::{
        os::windows::{fs::OpenOptionsExt, process::CommandExt},
        path::PathBuf,
        process::Command,
    };
    use sticktalk_desktop_platform::{
        updater::Updater,
        windows::{register_shortcut_aumid, wait_for_exit, Apartment},
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("shortcut") if args.len() == 2 => {
            let _apartment = Apartment::initialize()?;
            register_shortcut_aumid(&PathBuf::from(&args[1]))?;
        }
        Some("apply") if args.len() == 7 && args[6] == "--confirmed" => {
            // apply <origin> <installed-version> <staging-dir> <parent-pid> <installed-exe> --confirmed
            let updater = Updater::embedded(&args[1], &args[2], PathBuf::from(&args[3]))?;
            let parent: u32 = args[4].parse()?;
            let exe = PathBuf::from(&args[5]);
            if !exe.is_absolute()
                || exe.file_name().and_then(|n| n.to_str()) != Some("sticktalk.exe")
            {
                return Err("invalid restart executable".into());
            }
            wait_for_exit(parent)?;
            let stage = updater.pending().await?.ok_or("no pending update")?;
            // Keep the verified installer immutable through process creation/execution.
            let _installer_lock = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(stage.installer())?;
            let stage = updater.pending().await?.ok_or("no pending update")?;
            let status = Command::new(stage.installer())
                .arg("/S")
                // NSIS consumes /D= through end of command line; raw_arg avoids
                // Rust's outer quoting when an installation path contains spaces.
                .raw_arg(format!(
                    "/D={}",
                    exe.parent().ok_or("no install directory")?.display()
                ))
                .creation_flags(0x08000000)
                .status()?;
            if !status.success() {
                return Err("installer failed; stage retained for retry".into());
            }
            updater.clear_pending().await?;
            Command::new(exe).spawn()?;
        }
        _ => return Err(
            "expected shortcut <lnk> or apply <origin> <version> <stage> <pid> <exe> --confirmed"
                .into(),
        ),
    }
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("StickTalk maintenance is Windows-only");
}
