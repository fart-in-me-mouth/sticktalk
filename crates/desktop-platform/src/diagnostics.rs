//! Bounded local diagnostics; no account identifiers, tokens or panic payloads.
use std::{
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
static PREVIOUS_CRASH: AtomicBool = AtomicBool::new(false);
/// Only the primary, non-mock process installs this. A panic payload may contain
/// credentials, so deliberately log source location/backtrace, never the payload.
pub struct Session(std::path::PathBuf);
impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub fn initialize(root: &Path) -> Session {
    let logs = root.join("Logs");
    let _ = std::fs::create_dir_all(&logs);
    let marker = logs.join("crash.flag");
    let running = logs.join("running.flag");
    let abnormal = running.exists();
    PREVIOUS_CRASH.store(marker.exists() || abnormal, Ordering::Relaxed);
    if abnormal && !logs.join("crash.log").exists() {
        let _ = std::fs::write(logs.join("crash.log"), b"The previous desktop process did not shut down normally. No account data was recorded.");
    }
    let _ = std::fs::write(&running, b"running");
    let _ = std::fs::remove_file(&marker);
    std::panic::set_hook(Box::new(move |info| {
        let current = logs.join("crash.log");
        let _ = std::fs::rename(&current, logs.join("crash.previous.log"));
        if let Ok(mut file) = std::fs::File::create(current) {
            let _ = writeln!(
                file,
                "StickTalk {}\nTime: {:?}\nLocation: {:?}\n{}",
                env!("CARGO_PKG_VERSION"),
                std::time::SystemTime::now(),
                info.location(),
                std::backtrace::Backtrace::force_capture()
            );
            let _ = file.sync_all();
        }
        let _ = std::fs::write(&marker, b"panic");
    }));
    Session(running)
}
pub fn previous_crash() -> bool {
    PREVIOUS_CRASH.load(Ordering::Relaxed)
}
/// Runtime initialization failures are recorded without request URLs or secrets.
pub fn record_failure(root: &Path, category: &str) {
    let logs = root.join("Logs");
    let _ = std::fs::create_dir_all(&logs);
    let _ = std::fs::write(logs.join("startup.log"), category);
    let _ = std::fs::write(logs.join("crash.flag"), b"startup");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graceful_exit_clears_marker_and_startup_failure_is_local() {
        let root = tempfile::tempdir().unwrap();
        let old = std::panic::take_hook();
        let session = initialize(root.path());
        assert!(root.path().join("Logs/running.flag").exists());
        let panic = std::panic::catch_unwind(|| panic!("sensitive-payload-must-not-be-recorded"));
        assert!(panic.is_err());
        let log = std::fs::read_to_string(root.path().join("Logs/crash.log")).unwrap();
        assert!(log.contains("Location:"));
        assert!(!log.contains("sensitive-payload-must-not-be-recorded"));
        drop(session);
        assert!(!root.path().join("Logs/running.flag").exists());
        record_failure(root.path(), "Window initialization failed");
        assert!(root.path().join("Logs/crash.flag").exists());
        let _next = initialize(root.path());
        assert!(previous_crash());
        assert!(!root.path().join("Logs/crash.flag").exists());
        std::panic::set_hook(old);
    }
}
