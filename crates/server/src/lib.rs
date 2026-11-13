//! StickTalk Phase 1 service. Mutations and snapshots share one serialization gate.
mod account;
pub mod admin;
#[cfg(test)]
mod audit_tests;
pub mod backup;
pub mod blob;
pub mod browser;
mod commands;
mod custom_emoji;
mod db;
mod error;
mod files;
pub mod gifs;
mod google;
mod history;
mod http;
mod http_io;
mod launch;
mod legacy;
mod mentions;
mod reauth;
mod search;
mod slowmode;
pub mod storage_node;
#[cfg(test)]
mod storage_tests;
mod tier;
pub use mentions::Notification;
#[cfg(test)]
mod operations_tests;
pub mod permissions;
pub mod push;
mod resumable;
mod state;
pub mod updates;
mod upload_accounting;
pub mod voice;
mod ws;
pub use admin::OperationsConfig;
pub use error::{Error, Result};
pub use state::{App, Config};
pub fn router(app: App) -> axum::Router {
    voice::spawn(&app);
    push::spawn(&app);
    http::router(app)
}
#[cfg(test)]
mod launch_tests;
#[cfg(test)]
mod security_tests;
