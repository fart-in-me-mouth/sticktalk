use crate::Error;
use std::time::{Duration, Instant};
use sticktalk_proto::*;

pub(crate) struct Grant {
    pub controller: VoiceSession,
    pub id: u64,
    pub sequence: Option<u64>,
    pub window: Instant,
    pub count: u16,
}

impl Grant {
    pub fn new(controller: VoiceSession, now: Instant) -> Self {
        Self {
            controller,
            id: rand::random(),
            sequence: None,
            window: now,
            count: 0,
        }
    }

    pub fn accept(
        &mut self,
        sender: VoiceSession,
        id: u64,
        sequence: u64,
        input: &ControlInput,
        now: Instant,
    ) -> Result<(), Error> {
        if sender != self.controller
            || id != self.id
            || self.sequence.is_some_and(|s| sequence <= s)
        {
            return Err(Error::Forbidden);
        }
        let valid = match input {
            ControlInput::Button { button, .. } => *button <= 4,
            ControlInput::Key { usage, .. } => (4..=0xe7).contains(usage),
            ControlInput::Wheel { x, y } => x.unsigned_abs() <= 4096 && y.unsigned_abs() <= 4096,
            ControlInput::Move { .. } => true,
        };
        if !valid {
            return Err(Error::Invalid);
        }
        if now.saturating_duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.count = 0;
        }
        if self.count >= 1000 {
            return Err(Error::Limit);
        }
        self.count += 1;
        self.sequence = Some(sequence);
        Ok(())
    }
}

/// A strict byte token bucket. Includes RTX/probes at the final UDP boundary.
/// One 100 ms burst is permitted; no unbounded media queue is created.
pub(crate) struct Budget {
    bps: u64,
    tokens: f64,
    capacity: f64,
    last: Instant,
}
impl Budget {
    pub fn new(bps: u64, max_datagram_bytes: u16, now: Instant) -> Self {
        let capacity = ((bps as f64 / 8.0) * 0.1).max(f64::from(max_datagram_bytes));
        Self {
            bps,
            tokens: capacity,
            capacity,
            last: now,
        }
    }
    pub fn take(&mut self, bytes: usize, now: Instant) -> bool {
        self.tokens = (self.tokens
            + now.saturating_duration_since(self.last).as_secs_f64() * self.bps as f64 / 8.0)
            .min(self.capacity);
        self.last = now;
        if self.tokens < bytes as f64 {
            return false;
        }
        self.tokens -= bytes as f64;
        true
    }
}
