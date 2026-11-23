//! Test-only, behind the `media-tap` cargo feature (never in a normal build):
//! the browser suite for end-to-end encrypted calls (`crates/web/tests/e2ee.mjs`)
//! needs to see what the SFU forwards, to show it can only forward ciphertext,
//! and needs repeatable packet loss.
//!
//! - `STICKTALK_MEDIA_TAP=<dir>`: append each forwarded source packet to
//!   `<dir>/<user>-<connection>-<mid>-<kind>.rtp` as `u16 sequence, u32
//!   timestamp, u8 marker, u16 length, payload` (big-endian), at most 8 MiB a file.
//! - `STICKTALK_MEDIA_LOSS_PERMILLE=<0..1000>`: drop that share of forwarded
//!   media packets, pseudo-randomly with a fixed seed.
use std::{collections::HashMap, io::Write, path::PathBuf, sync::Mutex};
use sticktalk_proto::{TrackKind, VoiceSession};

struct State {
    dir: Option<PathBuf>,
    written: HashMap<PathBuf, usize>,
    loss: u64,
    seed: u64,
}
static STATE: Mutex<Option<State>> = Mutex::new(None);
const MAX_FILE: usize = 8 << 20;

/// False: drop this packet (test packet loss). Otherwise records it when tapping.
pub(crate) fn forward(
    session: VoiceSession,
    mid: &str,
    kind: TrackKind,
    sequence: u16,
    timestamp: u32,
    marker: bool,
    payload: &[u8],
) -> bool {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.get_or_insert_with(|| State {
        dir: std::env::var_os("STICKTALK_MEDIA_TAP").map(PathBuf::from),
        written: HashMap::new(),
        loss: std::env::var("STICKTALK_MEDIA_LOSS_PERMILLE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
            .min(1000),
        seed: 0x9e37_79b9_7f4a_7c15,
    });
    if s.loss > 0 {
        s.seed = s
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        if (s.seed >> 33) % 1000 < s.loss {
            return false;
        }
    }
    if let Some(dir) = &s.dir {
        let kind = match kind {
            TrackKind::Audio => "audio",
            TrackKind::Camera => "camera",
            TrackKind::Screen => "screen",
            TrackKind::SystemAudio => "system_audio",
        };
        let path = dir.join(format!(
            "{}-{}-{mid}-{kind}.rtp",
            session.user_id.0, session.connection_id.0
        ));
        let n = s.written.entry(path.clone()).or_default();
        if *n + payload.len() + 9 <= MAX_FILE {
            let mut record = Vec::with_capacity(payload.len() + 9);
            record.extend_from_slice(&sequence.to_be_bytes());
            record.extend_from_slice(&timestamp.to_be_bytes());
            record.push(u8::from(marker));
            record.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            record.extend_from_slice(payload);
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                if f.write_all(&record).is_ok() {
                    *n += record.len();
                }
            }
        }
    }
    true
}
