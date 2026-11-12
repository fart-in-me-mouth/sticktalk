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
    None => "http://127.0.0.1:8080",
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
