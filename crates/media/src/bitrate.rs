//! Bitrate updates never invalidate the codec's reference chain.
use std::time::Duration;

pub(crate) const BITRATE_INTERVAL: Duration = Duration::from_secs(2);

pub(crate) fn update_due(applied: u32, requested: u32, elapsed: Duration) -> bool {
    elapsed >= BITRATE_INTERVAL
        && u64::from(applied.abs_diff(requested)) * 100 > u64::from(applied) * 15
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn noise_does_not_reset_codec_or_update_bitrate() {
        for value in [6_800_000, 7_999_999, 8_000_001, 9_200_000] {
            assert!(!update_due(8_000_000, value, Duration::from_secs(600)));
        }
    }
    #[test]
    fn large_changes_are_coalesced_for_two_seconds_in_both_directions() {
        for value in [1_000_000, 12_000_000] {
            assert!(!update_due(8_000_000, value, Duration::from_millis(1999)));
            assert!(update_due(8_000_000, value, BITRATE_INTERVAL));
        }
    }
}
