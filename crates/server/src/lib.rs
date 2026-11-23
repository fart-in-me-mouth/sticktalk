//! StickTalk Phase 1 service. Mutations and snapshots share one serialization gate.
mod account;
mod activities;
pub mod admin;
mod apps;
#[cfg(test)]
mod audit_tests;
mod authsec;
pub mod backup;
pub mod blob;
pub mod browser;
mod callextras;
mod calls;
mod capacity;
mod commands;
mod community;
mod content;
mod custom_emoji;
mod db;
mod error;
mod expression;
mod files;
mod folders;
pub mod gifs;
mod google;
mod history;
mod http;
mod http_io;
mod identity;
mod launch;
mod legacy;
mod mcp;
mod mentions;
mod moderation;
mod nav;
mod privacy;
pub mod profiles;
mod reauth;
mod schedule;
mod search;
mod slowmode;
mod stage;
pub mod storage_node;
#[cfg(test)]
mod storage_tests;
mod themes;
mod threads;
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
    nav::spawn(&app);
    content::spawn(&app);
    moderation::spawn(&app);
    authsec::spawn(&app);
    threads::spawn(&app);
    privacy::spawn(&app);
    stage::spawn(&app);
    schedule::spawn(&app);
    apps::spawn(&app);
    http::router(app)
}
#[cfg(test)]
mod launch_tests;
#[cfg(test)]
mod security_tests;
#[cfg(test)]
mod stack_tests;
