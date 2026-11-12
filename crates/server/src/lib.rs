//! StickTalk Phase 1 service. Mutations and snapshots share one serialization gate.
pub mod admin;
#[cfg(test)]
mod audit_tests;
pub mod browser;
mod commands;
mod db;
mod error;
mod files;
pub mod gifs;
mod google;
mod history;
mod http;
#[cfg(test)]
mod operations_tests;
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
