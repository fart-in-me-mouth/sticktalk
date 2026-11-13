//! Shared, runtime-independent client behavior. No views or OS services.
pub mod backend;
pub mod datetime;
pub mod http;
pub mod io;
pub mod markdown;
pub mod mock;
pub mod model;
pub mod oauth;
pub mod protocol;
pub mod state;
pub mod upload;
pub mod voice;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub mod web;

#[cfg(test)]
mod tests;
