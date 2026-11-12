//! Event-driven Windows integration. No desktop effects happen on import, construction
//! of data types, or in the dry-run examples. The host retains its one Tokio runtime.
pub mod activation;
pub mod notification;
pub mod sound;
pub mod tray;
pub mod updater;
#[cfg(windows)]
pub mod windows;

pub const APP_USER_MODEL_ID: &str = "StickTalk.Desktop";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid desktop input: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[cfg(windows)]
    #[error(transparent)]
    Windows(#[from] ::windows::core::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
