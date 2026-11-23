//! Stream quality policy: max-min fair video allocation that respects what each
//! track actually needs, and the voice channel's audio bitrate ceiling.
//!
//! Without any declared need every video route gets an equal share, exactly as
//! before. When a publisher declares a track's need (`VoiceSignal::TrackQuality`,
//! from its chosen resolution and frame rate), a camera that needs 2.5 Mbps no
//! longer holds back half of a viewer's capacity from a 4K60 screen share: the
//! unused part goes to the tracks that need more.
use std::time::{Duration, Instant};

/// Max-min fair ("water-filling") split of `capacity` among routes whose needs
/// are `caps`. Every route gets at most its cap; routes that need less than an
/// equal share leave the rest to the others. Routes still above the final level
/// all get the same floor share, so equal caps give exactly `capacity / n`.
pub fn waterfill(capacity: u64, caps: &[u64]) -> Vec<u64> {
    let mut order: Vec<usize> = (0..caps.len()).collect();
    order.sort_by_key(|&i| caps[i]);
    let mut out = vec![0; caps.len()];
    let mut remaining = capacity;
    for (done, &i) in order.iter().enumerate() {
        let fair = remaining / (caps.len() - done) as u64;
        if caps[i] <= fair {
            out[i] = caps[i];
            remaining -= caps[i];
        } else {
            for &j in &order[done..] {
                out[j] = fair;
            }
            break;
        }
    }
    out
}

/// A token bucket on one microphone's Opus payload. It allows 1.5 times the
/// channel bitrate sustained (variable-rate Opus peaks above its average on
/// transients) with half a second of burst, so a compliant encoder is never
/// touched and a client that ignores the setting is held near it.
#[derive(Debug, Clone)]
pub struct AudioPolicer {
    bps: u64,
    tokens: f64,
    at: Instant,
}

impl AudioPolicer {
    pub fn new(bps: u64, now: Instant) -> Self {
        let mut p = Self {
            bps: 0,
            tokens: 0.,
            at: now,
        };
        p.set(bps);
        p.tokens = p.burst();
        p
    }
    fn rate(&self) -> f64 {
        self.bps as f64 * 1.5
    }
    fn burst(&self) -> f64 {
        (self.rate() / 2.).max(12_000. * 8.)
    }
    pub fn bps(&self) -> u64 {
        self.bps
    }
    pub fn set(&mut self, bps: u64) {
        self.bps = bps;
        self.tokens = self.tokens.min(self.burst());
    }
    /// True when this payload may be forwarded.
    pub fn admit(&mut self, payload_bytes: usize, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.at).min(Duration::from_secs(2));
        self.at = now;
        self.tokens = (self.tokens + elapsed.as_secs_f64() * self.rate()).min(self.burst());
        let bits = payload_bytes as f64 * 8.;
        if self.tokens >= bits {
            self.tokens -= bits;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_needs_split_evenly_like_before() {
        assert_eq!(waterfill(32_000_000, &[u64::MAX; 4]), vec![8_000_000; 4]);
        assert_eq!(waterfill(10, &[u64::MAX; 3]), vec![3; 3]);
        assert_eq!(waterfill(0, &[u64::MAX; 2]), vec![0; 2]);
        assert!(waterfill(5, &[]).is_empty());
    }

    #[test]
    fn small_needs_leave_their_share_to_large_ones() {
        // A 2.5 Mbps camera beside a 4K60 share: the share gets the rest.
        assert_eq!(
            waterfill(24_000_000, &[3_333_333, 29_333_333]),
            vec![3_333_333, 20_666_667]
        );
        // Everything fits: everyone gets exactly what they need.
        assert_eq!(waterfill(100, &[10, 20, 30]), vec![10, 20, 30]);
        // Order does not matter.
        assert_eq!(waterfill(30, &[100, 5, 100]), vec![12, 5, 12]);
    }

    #[test]
    fn policer_passes_a_compliant_encoder_and_holds_an_abusive_one() {
        let start = Instant::now();
        // 64 kbps Opus, 20 ms frames: 160-byte payloads, 50 per second.
        let mut p = AudioPolicer::new(64_000, start);
        let mut passed = 0;
        for i in 0..500u64 {
            if p.admit(160, start + Duration::from_millis(20 * i)) {
                passed += 1;
            }
        }
        assert_eq!(passed, 500);
        // 510 kbps against a 64 kbps channel: most of it is dropped.
        let mut p = AudioPolicer::new(64_000, start);
        let mut bits = 0u64;
        for i in 0..500u64 {
            if p.admit(1275, start + Duration::from_millis(20 * i)) {
                bits += 1275 * 8;
            }
        }
        let seconds = 10.;
        assert!((bits as f64 / seconds) < 64_000. * 1.7, "{bits}");
        // Raising the limit takes effect.
        p.set(510_000);
        let later = start + Duration::from_secs(20);
        assert!(p.admit(1275, later));
        assert_eq!(p.bps(), 510_000);
    }
}
