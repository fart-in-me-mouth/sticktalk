//! Direct and group calls: a call that lives in a DM or group DM instead of a
//! guild voice channel. Media uses the ordinary voice path (`VoiceSignal::Join`
//! with the conversation's channel ID); these types carry the ringing and
//! "call in progress" state that a guild voice channel does not have.
use crate::*;
use serde::{Deserialize, Serialize};

/// How long a ring lasts before the server stops it on its own.
pub const CALL_RING_TIMEOUT_MS: u64 = 45_000;
/// People one `Ring` signal may name; a group DM never has more members.
pub const MAX_CALL_RING_TARGETS: usize = MAX_GROUP_PARTICIPANTS;

/// The live state of one conversation's call. A full replacement each time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmCall {
    pub channel_id: ChannelId,
    pub started_by: UserId,
    pub started_at: UnixMillis,
    /// Everyone connected to the call's media right now.
    pub participants: Vec<VoiceParticipant>,
    /// Members whose devices are ringing now, in ID order.
    pub ringing: Vec<UserId>,
}

/// The record a finished call leaves in its conversation, on a system message
/// authored by the person who started it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallSummary {
    pub started_at: UnixMillis,
    pub ended_at: UnixMillis,
    /// Nobody answered.
    pub missed: bool,
    /// Distinct people who were connected at some point, the starter included.
    pub participant_count: u16,
}

impl CallSummary {
    pub fn duration_ms(&self) -> u64 {
        self.ended_at.saturating_sub(self.started_at)
    }
}
