use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

/// A specification revision must be nonzero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("world-instance specification revision must be nonzero")]
pub struct InvalidWorldInstanceSpecRevision;

/// Monotonic revision of one logical instance's desired specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldInstanceSpecRevision(NonZeroU64);

impl WorldInstanceSpecRevision {
    /// Returns the initial revision.
    #[must_use]
    pub const fn initial() -> Self {
        Self(NonZeroU64::MIN)
    }

    /// Returns the integer representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Returns the next revision, or `None` after the maximum value.
    #[must_use]
    pub fn checked_next(self) -> Option<Self> {
        self.get()
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .map(Self)
    }
}

impl TryFrom<u64> for WorldInstanceSpecRevision {
    type Error = InvalidWorldInstanceSpecRevision;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(InvalidWorldInstanceSpecRevision)
    }
}

/// Immutable, sealed launch-product identity for a logical instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorldInstanceDefinition {
    launch_product_digest: [u8; 32],
}

impl WorldInstanceDefinition {
    /// Binds an instance to one sealed launch product.
    #[must_use]
    pub const fn new(launch_product_digest: [u8; 32]) -> Self {
        Self {
            launch_product_digest,
        }
    }

    /// Returns the sealed launch-product digest.
    #[must_use]
    pub const fn launch_product_digest(&self) -> &[u8; 32] {
        &self.launch_product_digest
    }
}

/// Declarative target state reconciled by the lifecycle owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorldInstanceDesiredState {
    /// The instance should converge on an admissible running placement.
    Running,
    /// The instance should stop new admission and converge on its barriers.
    Draining,
    /// The instance should converge on a terminal tombstone.
    Terminated,
}
