//! StickTalk Phase 1 service. Mutations and snapshots share one serialization gate.
pub mod admin;
#[cfg(test)]
mod audit_tests;
pub mod backup;
pub mod blob;
pub mod browser;
mod commands;
mod db;
mod error;
mod files;
pub mod gifs;
mod google;
mod history;
mod http;
mod launch;
mod legacy;
mod mentions;
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
