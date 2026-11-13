//! Transactional per-user NSIS application with an explicit UI-startup handshake.
//! No process polling: the helper waits once on the process handle for the grace period.
use crate::{updater::Updater, Error, Result};
use std::{
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::Path,
    process::Command,
};
use windows::Win32::{
    Foundation::{HANDLE, WAIT_TIMEOUT},
    System::Threading::WaitForSingleObject,
};
const OWNED: &[&str] = &[
    "sticktalk.exe",
    "sticktalk-maintenance.exe",
    "Uninstall.exe",
    "sticktalk.ico",
    "Twemoji-NOTICE.txt",
    "Twemoji-LICENSE.txt",
    "concrt140.dll",
    "msvcp140_1.dll",
    "msvcp140_2.dll",
    "msvcp140_atomic_wait.dll",
    "msvcp140_codecvt_ids.dll",
    "msvcp140.dll",
    "vccorlib140.dll",
    "vcruntime140_1.dll",
    "vcruntime140_threads.dll",
    "vcruntime140.dll",
];
const GRACE_MS: u32 = 15_000;
/// Called by the client only after its actual window/UI has initialized. Merely
/// creating a process is not evidence that wgpu, preferences or the UI started.
pub fn acknowledge_startup() {
    let (Some(path), Some(nonce)) = (
        std::env::var_os("STICKTALK_UPDATE_HEALTH_FILE"),
        std::env::var_os("STICKTALK_UPDATE_HEALTH_NONCE"),
    ) else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    if path.is_absolute()
        && path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("health-"))
    {
        let _ = std::fs::write(path, nonce.to_string_lossy().as_bytes());
    }
}
pub async fn apply(
    updater: &Updater,
    exe: &Path,
    staging: &Path,
    installed_version: &str,
) -> Result<()> {
    use ring::rand::{SecureRandom, SystemRandom};
    use std::os::windows::fs::OpenOptionsExt;
    let dir = exe.parent().ok_or(Error::Invalid("no install directory"))?;
    let stage = updater
        .pending()
        .await?
        .ok_or(Error::Invalid("no pending update"))?;
    // Deny writes/deletes from verification until the installer exits.
    let _lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(stage.installer())?;
    let stage = updater
        .pending()
        .await?
        .ok_or(Error::Invalid("no pending update"))?;
    let backup = staging.join("rollback");
    if backup.exists() {
        if backup.join("version.txt").exists() {
            return Err(Error::Invalid(
                "unfinished update backup; recover before retrying",
            ));
        }
        // A crash while PREPARING never changed installed files. Discard only
        // known partial backup files before retrying the transaction.
        remove_backup(&backup)?;
    }
    std::fs::create_dir(&backup)?;
    for name in OWNED {
        let source = dir.join(name);
        if source.is_file() {
            std::fs::copy(&source, backup.join(name))?;
            std::fs::OpenOptions::new()
                .write(true)
                .open(backup.join(name))?
                .sync_all()?;
        }
    }
    // Only a COMPLETE, durable backup becomes eligible for recovery.
    std::fs::write(backup.join("version.tmp"), installed_version)?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(backup.join("version.tmp"))?
        .sync_all()?;
    std::fs::rename(backup.join("version.tmp"), backup.join("version.txt"))?;
    let install = Command::new(stage.installer())
        .arg("/S")
        .raw_arg(format!("/D={}", dir.display()))
        .creation_flags(0x08000000)
        .status();
    let mut child = None;
    let mut healthy = false;
    if install.as_ref().is_ok_and(|s| s.success()) {
        let mut nonce_bytes = [0; 32];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| Error::Invalid("health nonce"))?;
        let nonce = nonce_bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let health = staging.join(format!("health-{nonce}"));
        let launched = Command::new(exe)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .env("STICKTALK_UPDATE_HEALTH_FILE", &health)
            .env("STICKTALK_UPDATE_HEALTH_NONCE", &nonce)
            .creation_flags(0x08000000)
            .spawn();
        if let Ok(mut process) = launched {
            let wait = unsafe { WaitForSingleObject(HANDLE(process.as_raw_handle()), GRACE_MS) };
            healthy =
                wait == WAIT_TIMEOUT && std::fs::read(&health).is_ok_and(|v| v == nonce.as_bytes());
            if !healthy {
                let _ = process.kill();
                let _ = process.wait();
            }
            child = Some(process);
        }
        let _ = std::fs::remove_file(health);
    }
    if healthy {
        // Commit the healthy release BEFORE deleting any backup contents. A
        // power loss during cleanup must never restore a half-deleted backup.
        std::fs::remove_file(backup.join("version.txt"))?;
        updater.clear_pending().await?;
        remove_backup(&backup)?;
        std::fs::write(
            staging.join("last-update.log"),
            format!("Updated to {}", stage.version()),
        )?;
        // The healthy child keeps running; dropping Child never kills it.
        if let Some(process) = child {
            std::fs::write(staging.join("last-update.pid"), process.id().to_string())?;
        }
        Ok(())
    } else {
        restore(&backup, dir, installed_version)?;
        let _ = std::fs::rename(
            staging.join("pending.json"),
            staging.join("failed-update.json"),
        );
        std::fs::write(
            staging.join("last-update.log"),
            format!(
                "Rolled back failed startup of {} to {installed_version}",
                stage.version()
            ),
        )?;
        let previous = Command::new(exe)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .env("STICKTALK_UPDATE_ROLLED_BACK", "1")
            .creation_flags(0x08000000)
            .spawn()?;
        std::fs::write(staging.join("last-update.pid"), previous.id().to_string())?;
        Err(Error::Invalid(
            "update failed to start; restored previous version",
        ))
    }
}
fn remove_backup(backup: &Path) -> Result<()> {
    // Never recursively delete a path derived from IPC/HTTP/command-line input.
    // First remove the recovery marker; all later deletions are mere cleanup.
    for name in ["version.txt", "version.tmp"]
        .into_iter()
        .chain(OWNED.iter().copied())
    {
        match std::fs::remove_file(backup.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    std::fs::remove_dir(backup)?;
    Ok(())
}
fn restore(backup: &Path, dir: &Path, version: &str) -> Result<()> {
    for name in OWNED {
        let saved = backup.join(name);
        if saved.is_file() {
            std::fs::copy(saved, dir.join(name))?;
        }
    }
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\StickTalk",
        winreg::enums::KEY_WRITE,
    )?;
    key.set_value("DisplayVersion", &version)?;
    remove_backup(backup)
}
/// A power loss/helper crash can leave a backup. Recover before opening any UI.
/// Only the installed helper invokes this using the current executable directory.
pub fn recover(exe: &Path, staging: &Path) -> Result<bool> {
    let backup = staging.join("rollback");
    if !backup.is_dir() {
        return Ok(false);
    }
    let version = std::fs::read_to_string(backup.join("version.txt"))?;
    semver::Version::parse(version.trim()).map_err(|_| Error::Invalid("rollback version"))?;
    restore(
        &backup,
        exe.parent().ok_or(Error::Invalid("no install directory"))?,
        version.trim(),
    )?;
    let _ = std::fs::rename(
        staging.join("pending.json"),
        staging.join("failed-update.json"),
    );
    Ok(true)
}
