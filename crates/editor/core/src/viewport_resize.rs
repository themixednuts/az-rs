//! Bounded latest-wins resize policy for the `DirectComposition` producer.

use std::time::{Duration, Instant};

pub const RESIZE_QUIET_PERIOD: Duration = Duration::from_millis(50);
pub const MAX_SCALE_ERROR: f64 = 0.20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeDecision {
    KeepCurrent,
    ReplaceWith((u32, u32)),
}

/// Tracks the physical surface extent independently from the latest layout.
/// Layout publication is latest-wins; replacement is bounded to one quality
/// response per quiet-period interval plus the final settled replacement.
#[derive(Debug)]
pub struct ViewportResizePolicy {
    configured: (u32, u32),
    desired: (u32, u32),
    desired_changed_at: Instant,
    replaced_at: Instant,
}

impl ViewportResizePolicy {
    pub(crate) fn new(configured: (u32, u32), now: Instant) -> Self {
        let configured = valid_extent(configured);
        Self {
            configured,
            desired: configured,
            desired_changed_at: now,
            replaced_at: now.checked_sub(RESIZE_QUIET_PERIOD).unwrap_or(now),
        }
    }

    pub(crate) fn publish_desired(&mut self, desired: (u32, u32), now: Instant) {
        let desired = valid_extent(desired);
        if self.desired != desired {
            self.desired = desired;
            self.desired_changed_at = now;
        }
    }

    #[must_use]
    pub(crate) fn decide(&self, now: Instant, surface_idle: bool) -> ResizeDecision {
        if !surface_idle || self.desired == self.configured {
            return ResizeDecision::KeepCurrent;
        }
        let quiet = now.saturating_duration_since(self.desired_changed_at) >= RESIZE_QUIET_PERIOD;
        let quality_limited = scale_error(self.configured, self.desired) > MAX_SCALE_ERROR
            && now.saturating_duration_since(self.replaced_at) >= RESIZE_QUIET_PERIOD;
        if quiet || quality_limited {
            ResizeDecision::ReplaceWith(self.desired)
        } else {
            ResizeDecision::KeepCurrent
        }
    }

    pub(crate) fn replaced(&mut self, configured: (u32, u32), now: Instant) {
        self.configured = valid_extent(configured);
        self.replaced_at = now;
    }
}

fn valid_extent(extent: (u32, u32)) -> (u32, u32) {
    (extent.0.max(1), extent.1.max(1))
}

/// Relative extent mismatch on the worse axis. `f64` keeps every `u32` extent
/// exact, so the ratio never depends on a lossy widening.
fn scale_error(configured: (u32, u32), desired: (u32, u32)) -> f64 {
    let axis_error =
        |current: u32, next: u32| (f64::from(next) / f64::from(current.max(1)) - 1.0).abs();
    axis_error(configured.0, desired.0).max(axis_error(configured.1, desired.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_extent_wins_after_quiet_period() {
        let start = Instant::now();
        let mut policy = ViewportResizePolicy::new((1_000, 800), start);
        policy.publish_desired((1_050, 800), start + Duration::from_millis(10));
        policy.publish_desired((1_100, 800), start + Duration::from_millis(20));
        assert_eq!(
            policy.decide(start + Duration::from_millis(69), true),
            ResizeDecision::KeepCurrent
        );
        assert_eq!(
            policy.decide(start + Duration::from_millis(70), true),
            ResizeDecision::ReplaceWith((1_100, 800))
        );
    }

    #[test]
    fn quality_replacement_is_bounded_during_motion() {
        let start = Instant::now();
        let mut policy = ViewportResizePolicy::new((1_000, 800), start);
        policy.publish_desired((1_300, 800), start + Duration::from_millis(1));
        assert_eq!(
            policy.decide(start + Duration::from_millis(1), true),
            ResizeDecision::ReplaceWith((1_300, 800))
        );
        policy.replaced((1_300, 800), start + Duration::from_millis(1));
        policy.publish_desired((1_650, 800), start + Duration::from_millis(2));
        assert_eq!(
            policy.decide(start + Duration::from_millis(50), true),
            ResizeDecision::KeepCurrent
        );
        assert_eq!(
            policy.decide(start + Duration::from_millis(51), true),
            ResizeDecision::ReplaceWith((1_650, 800))
        );
    }

    #[test]
    fn acquired_surface_always_defers_replacement() {
        let start = Instant::now();
        let mut policy = ViewportResizePolicy::new((1_000, 800), start);
        policy.publish_desired((1_500, 900), start);
        assert_eq!(
            policy.decide(start + RESIZE_QUIET_PERIOD, false),
            ResizeDecision::KeepCurrent
        );
    }

    #[test]
    fn final_settle_does_not_require_large_mismatch() {
        let start = Instant::now();
        let mut policy = ViewportResizePolicy::new((1_000, 800), start);
        policy.publish_desired((1_001, 800), start);
        assert_eq!(
            policy.decide(start + RESIZE_QUIET_PERIOD, true),
            ResizeDecision::ReplaceWith((1_001, 800))
        );
    }
}
