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
        if !self.available(bytes, false, now) {
            return false;
        }
        self.tokens -= bytes as f64;
        true
    }
    /// Preflight without spending: peer/room/global admission must be atomic.
    /// Reserve half the burst for original media, so capacity probes/RTX cannot
    /// consume the last tokens and break an otherwise admissible reference chain.
    /// Both classes still share the SAME strict wire rate and burst ceiling.
    pub fn available(&mut self, bytes: usize, repair: bool, now: Instant) -> bool {
        self.tokens = (self.tokens
            + now.saturating_duration_since(self.last).as_secs_f64() * self.bps as f64 / 8.0)
            .min(self.capacity);
        self.last = now;
        self.tokens >= bytes as f64 + if repair { self.capacity * 0.5 } else { 0.0 }
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn probes_leave_media_headroom_without_increasing_wire_budget() {
        let now = Instant::now();
        let mut b = Budget::new(80_000, 1000, now); // 1000-byte burst
        assert!(b.available(500, true, now));
        assert!(b.take(500, now));
        assert!(!b.available(1, true, now));
        assert!(b.take(500, now));
        assert!(!b.take(1, now));
        // A rejected preflight never spends tokens in any other bucket.
        assert!(!b.available(1001, false, now + Duration::from_secs(1)));
        assert!(b.take(1000, now + Duration::from_secs(1)));
        assert!(!b.take(1, now + Duration::from_secs(1)));
    }
}
