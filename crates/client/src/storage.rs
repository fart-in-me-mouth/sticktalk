//! Non-secret preference on disk; bearer tokens only in the OS credential store.
use std::path::PathBuf;
pub fn root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("StickTalk")
}
pub const DEFAULT_SERVER: &str = match option_env!("STICKTALK_DEFAULT_SERVER") {
    Some(value) => value,
    // An unpackaged first launch asks for an administrator-provided address.
    // Do not silently connect to a developer's local server.
    None => "",
};
pub fn server() -> String {
    std::fs::read_to_string(root().join("server.txt"))
        .ok()
        .filter(|s| !s.trim().is_empty() && s.len() < 2048)
        .unwrap_or_else(|| DEFAULT_SERVER.into())
}
pub fn save_server(server: &str) {
    let _ = std::fs::create_dir_all(root());
    let _ = std::fs::write(root().join("server.txt"), server);
}
pub fn save_token(server: &str, token: &str) -> Result<(), String> {
    keyring::Entry::new("StickTalk", server)
        .and_then(|v| v.set_password(token))
        .map_err(|_| "Could not save your session. Please try again.".into())
}
pub fn token(server: &str) -> Option<String> {
    keyring::Entry::new("StickTalk", server)
        .ok()?
        .get_password()
        .ok()
}
pub fn forget(server: &str) {
    if let Ok(v) = keyring::Entry::new("StickTalk", server) {
        let _ = v.delete_credential();
    }
}

/// Releases pin updates independently of the user's selected chat server.
pub fn update_origin() -> String {
    option_env!("STICKTALK_UPDATE_ORIGIN")
        .unwrap_or(DEFAULT_SERVER)
        .to_string()
}

pub fn start_minimized() -> bool {
    std::fs::read(root().join("desktop.json"))
        .ok()
        .filter(|b| b.len() <= 128)
        .and_then(|b| serde_json::from_slice::<bool>(&b).ok())
        .unwrap_or(false)
}
pub fn save_start_minimized(enabled: bool) -> Result<(), String> {
    std::fs::create_dir_all(root()).map_err(|_| "Could not save desktop settings")?;
    std::fs::write(
        root().join("desktop.json"),
        if enabled {
            b"true".as_slice()
        } else {
            b"false".as_slice()
        },
    )
    .map_err(|_| "Could not save desktop settings".into())
}
