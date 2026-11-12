//! StickTalk Phase 1 service. Mutations and snapshots share one serialization gate.
pub mod admin;
#[cfg(test)]
mod audit_tests;
mod commands;
mod db;
mod error;
mod files;
pub mod gifs;
mod google;
mod http;
#[cfg(test)]
mod operations_tests;
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
    http::router(app)
}
