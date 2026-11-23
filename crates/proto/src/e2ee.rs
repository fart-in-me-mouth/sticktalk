//! End-to-end encrypted calls: the server's part of the wire (docs/E2EE.md,
//! docs/features/e2ee.md). The server relays these and stores public keys; it
//! never sees a media key. Byte fields are unpadded base64url text, so the same
//! shapes travel as MessagePack and through the web app's JSON bridge.
use crate::{ChannelId, VoiceSession};
use serde::{Deserialize, Serialize};

/// The protocol version a client declares in `E2eeSignal::Hello`.
pub const E2EE_VERSION: u8 = 1;
/// 65-byte uncompressed P-256 public keys, as base64url.
pub const E2EE_KEY_TEXT: usize = 87;
/// 64-byte ECDSA P-256 signatures (r || s), as base64url.
pub const E2EE_SIGNATURE_TEXT: usize = 86;
/// The largest pairwise message: 256 bytes, as base64url.
pub const E2EE_RELAY_TEXT: usize = 342;
/// What the frame trailer and sender signatures add to a microphone's Opus
/// payload (24 bytes a packet at 100 packets a second, plus signatures). The
/// host adds it to the bitrate the SFU's audio policer allows.
pub const E2EE_AUDIO_OVERHEAD_BPS: u64 = 24_000;

/// A device's signed key announcement for one call lifetime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct E2eeAnnounce {
    /// The device's long-term ECDSA identity key.
    pub identity: String,
    /// This call's ephemeral ECDH key.
    pub dh: String,
    /// This call's ephemeral ECDSA key for sender signatures.
    pub sig: String,
    /// ECDSA(identity) over the announcement bytes (docs/E2EE.md 5.1).
    pub signature: String,
}

/// Host-handled; the SFU rejects these like `Ring`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum E2eeSignal {
    /// Sent just before `Join`: this connection can do end-to-end encryption.
    Hello { version: u8 },
    /// After `Welcome`: this device's keys for the call (once per voice session).
    Announce(E2eeAnnounce),
    /// An opaque pairwise message for one other session in the same call.
    Relay { to: VoiceSession, data: String },
}

/// One session in a call, as the server reports it. Clients verify `announce`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct E2eeMember {
    pub session: VoiceSession,
    /// `session_devices.id`: 32 lowercase hex characters.
    pub device_id: String,
    /// Server-reported, informational ("Chrome · Windows").
    pub device_name: String,
    /// The connection sent `Hello`.
    pub supported: bool,
    pub announce: Option<E2eeAnnounce>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum E2eeEvent {
    /// To the joining connection: its session and device, and the call's policy.
    Welcome {
        channel_id: ChannelId,
        /// The call's server; 0 for a direct or group call.
        guild_id: crate::GuildId,
        session: VoiceSession,
        device_id: String,
        allow_unencrypted: bool,
    },
    /// Full replacement, to every session in the call whenever it changes.
    Roster {
        channel_id: ChannelId,
        allow_unencrypted: bool,
        members: Vec<E2eeMember>,
    },
    /// A pairwise message from another session in the call.
    Relay { from: VoiceSession, data: String },
}

/// `GET`/`PUT /api/v1/guilds/{id}/e2ee` and `/api/v1/channels/{id}/e2ee`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct E2eeSettings {
    /// Let clients that cannot encrypt join; the call is then not end-to-end
    /// encrypted and everyone in it is told. Off by default.
    pub allow_unencrypted: bool,
    /// Read-only: may this user change it.
    #[serde(default)]
    pub can_manage: bool,
}
