use serde::{Deserialize, Serialize};

/// Absolute time in an adapter-declared clock domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AbsoluteDeadline(u64);

impl AbsoluteDeadline {
    /// A deadline that does not expire in the represented domain.
    pub const MAX: Self = Self(u64::MAX);

    /// Constructs an absolute Unix-millisecond deadline.
    #[must_use]
    pub const fn from_unix_millis(value: u64) -> Self {
        Self(value)
    }

    /// Returns the represented absolute time.
    #[must_use]
    pub const fn unix_millis(self) -> u64 {
        self.0
    }

    /// Reports whether `now` has reached or passed this deadline.
    #[must_use]
    pub const fn elapsed_at(self, now: u64) -> bool {
        now >= self.0
    }
}
