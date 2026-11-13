//! Bound ORIGINAL unsent video, not str0m's aggregate pacer snapshot (which
//! includes old cached RTX and virtual probe padding). No payload copies.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct MediaQueue {
    pending: VecDeque<(u16, usize, Instant)>,
    bytes: usize,
}
impl MediaQueue {
    pub fn snapshot(&self) -> (usize, usize, Option<Instant>) {
        (
            self.pending.len(),
            self.bytes,
            self.pending.front().map(|p| p.2),
        )
    }
    pub fn full(&self, additional: usize, now: Instant) -> bool {
        self.pending.len() >= 2048
            || self.bytes.saturating_add(additional) > 2 * 1024 * 1024
            || self
                .pending
                .front()
                .is_some_and(|p| now.saturating_duration_since(p.2) > Duration::from_millis(250))
    }
    pub fn push(&mut self, sequence: u16, bytes: usize, now: Instant) -> bool {
        if self.full(bytes, now) {
            return false;
        }
        self.pending.push_back((sequence, bytes, now));
        self.bytes += bytes;
        true
    }
    /// Called when RTC emits ORIGINAL RTP, before final wire admission. A
    /// cap-dropped packet is no longer queued either; RTX has a separate SSRC.
    pub fn transmitted(&mut self, sequence: u16) {
        // Healthy FIFO is O(1); tolerate reordered enqueue/output and ignore
        // duplicates. The bounded queue is much smaller than a u16 wrap.
        let index = if self.pending.front().is_some_and(|p| p.0 == sequence) {
            Some(0)
        } else {
            self.pending.iter().position(|p| p.0 == sequence)
        };
        if let Some(index) = index {
            if let Some((_, bytes, _)) = self.pending.remove(index) {
                self.bytes -= bytes;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_unsent_originals_age_and_sequence_wrap_is_safe() {
        let t = Instant::now();
        let mut q = MediaQueue::default();
        assert!(q.push(u16::MAX, 100, t));
        assert!(q.push(0, 200, t + Duration::from_millis(10)));
        q.transmitted(0); // reordered output
        q.transmitted(0); // duplicate/repair cannot remove another packet
        assert_eq!(q.snapshot(), (1, 100, Some(t)));
        assert!(q.full(1, t + Duration::from_millis(251)));
        q.transmitted(u16::MAX);
        // Old RTX cache/probe timestamps must not create a stale media queue.
        assert!(!q.full(1232, t + Duration::from_secs(10)));
        assert_eq!(q.snapshot(), (0, 0, None));
    }
    #[test]
    fn packet_and_byte_caps_include_the_undrained_batch() {
        let t = Instant::now();
        let mut q = MediaQueue::default();
        for sequence in 0..2048 {
            assert!(q.push(sequence, 1, t));
        }
        assert!(!q.push(2048, 1, t));
        let mut q = MediaQueue::default();
        assert!(q.push(0, 2 * 1024 * 1024, t));
        assert!(!q.push(1, 1, t));
        q.transmitted(0);
        assert!(q.push(1, 1, t));
    }
}
