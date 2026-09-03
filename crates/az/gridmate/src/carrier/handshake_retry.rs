//! Handshake retry timing.
//!
//! The carrier driver consults [`retry_interval`] before re-sending a
//! connection request that has not been acknowledged. Defaults and backoff
//! semantics follow Lumberyard `GridMate`'s
//! `dev/Code/Framework/GridMate/GridMate/Carrier/Carrier.h` and `Carrier.cpp`.

use std::time::Duration;

/// Base retry interval (`CarrierDesc::m_connectionRetryIntervalBase`).
pub const RETRY_BASE_MS: u64 = 10;

/// Maximum retry interval (`CarrierDesc::m_connectionRetryIntervalMax`).
pub const RETRY_MAX_MS: u64 = 1000;

/// Get the retry interval for the given retry count, with
/// exponential backoff capped at [`RETRY_MAX_MS`].
///
/// `GridMate` computes `min(max, base * (1 << retry_count))`.
///
#[must_use]
pub fn retry_interval(num_retries: u32) -> Duration {
    let multiplier = 1_u64.checked_shl(num_retries).unwrap_or(u64::MAX);
    let interval_ms = RETRY_BASE_MS.saturating_mul(multiplier).min(RETRY_MAX_MS);
    Duration::from_millis(interval_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay_doubles_until_it_reaches_the_cap() {
        let delays = (0..=8).map(retry_interval).collect::<Vec<_>>();

        assert_eq!(
            delays,
            [10, 20, 40, 80, 160, 320, 640, 1000, 1000].map(Duration::from_millis)
        );
    }

    #[test]
    fn retry_delay_stays_capped_for_large_counts() {
        assert_eq!(retry_interval(u32::MAX), Duration::from_secs(1));
    }
}
