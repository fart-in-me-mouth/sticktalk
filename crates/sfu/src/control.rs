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
    pub(crate) fn available_bytes(&self, now: Instant) -> u64 {
        (self.tokens
            + now.saturating_duration_since(self.last).as_secs_f64() * self.bps as f64 / 8.0)
            .min(self.capacity) as u64
    }
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
        if !self.available(bytes, now) {
            return false;
        }
        self.tokens -= bytes as f64;
        true
    }
    /// Reallocate a viewer's fair wire share without resetting/minting tokens.
    pub fn set_rate(&mut self, bps: u64, max_datagram_bytes: u16, now: Instant) {
        let tokens = (self.tokens
            + now.saturating_duration_since(self.last).as_secs_f64() * self.bps as f64 / 8.0)
            .min(self.capacity);
        self.bps = bps;
        self.capacity = ((bps as f64 / 8.0) * 0.1).max(f64::from(max_datagram_bytes));
        self.tokens = tokens.min(self.capacity);
        self.last = now;
    }
    /// Earliest admission for original media. Used BEFORE polling str0m, so
    /// an encrypted original is never popped and thrown away at our own cap.
    /// The existing bounded RTC queue retains it; a one-shot deadline resumes
    /// draining without a second queue, busy wait, or increased burst ceiling.
    pub fn ready_at(&self, bytes: usize, now: Instant) -> Instant {
        let tokens = (self.tokens
            + now.saturating_duration_since(self.last).as_secs_f64() * self.bps as f64 / 8.0)
            .min(self.capacity);
        let missing = (bytes as f64 - tokens).max(0.0);
        if missing == 0.0 {
            now
        } else {
            // Refill uses floating-point seconds. One conservative nanosecond
            // avoids re-arming an already-expired deadline when roundoff puts
            // the balance just below a complete datagram at the exact boundary.
            now + Duration::from_nanos(
                (missing * 8_000_000_000.0 / self.bps as f64).ceil() as u64 + 1,
            )
        }
    }
    /// Preflight without spending: peer/room/global admission must be atomic.
    /// All video classes spend the same cap after pre-dequeue admission.
    pub fn available(&mut self, bytes: usize, now: Instant) -> bool {
        self.tokens = (self.tokens
            + now.saturating_duration_since(self.last).as_secs_f64() * self.bps as f64 / 8.0)
            .min(self.capacity);
        self.last = now;
        self.tokens >= bytes as f64
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn fair_share_reallocation_cannot_refill_a_depleted_bucket() {
        let now = Instant::now();
        let mut b = Budget::new(32_000_000, 1232, now);
        assert!(b.take(400_000, now));
        for _ in 0..100 {
            b.set_rate(16_000_000, 1232, now);
            assert!(!b.available(1, now));
        }
        assert!(b.take(200_000, now + Duration::from_millis(100)));
        assert!(!b.take(1, now + Duration::from_millis(100)));
        b.set_rate(32_000_000, 1232, now + Duration::from_millis(100));
        assert!(!b.take(1, now + Duration::from_millis(100)));
    }

    #[test]
    fn depleted_wire_cap_yields_a_precise_admission_deadline_without_spending() {
        let now = Instant::now();
        let mut b = Budget::new(32_000_000, 1232, now);
        assert!(b.take(400_000, now));
        let ready = b.ready_at(1232, now);
        assert_eq!(ready - now, Duration::from_nanos(308_001));
        assert!(!b.available(1232, ready - Duration::from_nanos(2)));
        assert!(b.available(1232, ready));
        assert!(b.take(1232, ready));
        assert!(!b.available(1, ready));
    }

    #[test]
    fn shared_probe_and_media_budget_never_mints_tokens() {
        let now = Instant::now();
        let mut b = Budget::new(80_000, 1000, now); // 1000-byte burst
        assert!(b.available(100, now));
        assert!(b.take(100, now));
        assert!(b.available(900, now));
        assert!(b.take(900, now));
        assert!(!b.take(1, now));
        // A rejected preflight never spends tokens in any other bucket.
        assert!(!b.available(1001, now + Duration::from_secs(1)));
        assert!(b.take(1000, now + Duration::from_secs(1)));
        assert!(!b.take(1, now + Duration::from_secs(1)));
    }

    #[test]
    fn a_low_rate_bucket_can_still_send_one_full_mtu_repair() {
        let now = Instant::now();
        let mut b = Budget::new(80_000, 1232, now);
        assert!(b.available(1232, now));
        assert!(b.take(1232, now));
        assert!(!b.available(1, now));
        assert!(!b.available(1233, now + Duration::from_secs(1)));
        assert!(b.available(1232, now + Duration::from_secs(1)));
    }
}
