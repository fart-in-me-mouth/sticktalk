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
mod launch;
#[cfg(test)]
mod operations_tests;
pub mod permissions;
mod resumable;
mod state;
pub mod updates;
mod ws;
pub use admin::OperationsConfig;
pub use error::{Error, Result};
pub use state::{App, Config};
pub fn router(app: App) -> axum::Router {
    http::router(app)
}
#[cfg(test)]
mod launch_tests;
