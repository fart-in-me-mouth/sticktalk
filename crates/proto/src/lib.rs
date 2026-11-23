//! Runtime-free protocol contracts. See `docs/PROTOCOL.md` for normative semantics.
//! Encode WebSocket messages using `rmp_serde::to_vec_named`, never `to_vec`.

mod browser;
mod calls;
pub use calls::*;
mod features3;
mod gifs;
mod ids;
pub use features3::*;
mod launch;
pub use launch::*;
mod models;
mod permissions;
mod rest;
mod ws;

pub use browser::*;
pub use gifs::*;
pub use ids::*;
pub use models::*;
pub use permissions::*;
pub use rest::*;
pub use ws::*;

pub const PROTOCOL_VERSION: u16 = 1;
pub const HEARTBEAT_INTERVAL_MS: u32 = 30_000;
pub const HEARTBEAT_TIMEOUT_MS: u32 = 75_000;
pub const MAX_UPLOAD_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_WS_FRAME_BYTES: u32 = 512 * 1024;
pub const DEFAULT_PAGE_SIZE: u16 = 50;
pub const MAX_PAGE_SIZE: u16 = 50;
pub const MAX_MESSAGE_CHARS: usize = 4_000;
pub const MAX_MESSAGE_ATTACHMENTS: usize = 4;
pub const MAX_MESSAGE_MENTIONS: usize = 100;
pub const MAX_GUILDS_PER_USER: usize = 10;
pub const MAX_CHANNELS_PER_GUILD: usize = 32;
pub const MAX_GUILD_MEMBERS: usize = 1_000;
pub const MAX_OPEN_DMS_PER_USER: usize = 64;
pub const MAX_GROUP_PARTICIPANTS: usize = 16;
pub const MAX_RELATIONSHIPS_PER_USER: usize = 100;
pub const MAX_SESSIONS_PER_USER: usize = 5;
pub const MAX_PRESENCE_INTERESTS: usize = 100;
pub const MAX_REACTIONS_PER_MESSAGE: usize = 20;
pub const SNOWFLAKE_EPOCH_MS: u64 = 1_735_689_600_000; // 2025-01-01 UTC
