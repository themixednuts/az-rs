use std::fmt;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Canonical authored row identity.
///
/// Imported rows receive deterministic GUIDs during transform. Runtime data
/// should not treat project-specific CRCs or compiled row order as the canonical row
/// identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(transparent)]
pub struct RowGuid(Uuid);

impl RowGuid {
    #[inline]
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    #[inline]
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for RowGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Hot compiled row reference.
///
/// This is one-based so `Option<RowIndex>` can use the `NonZeroU32` niche. It
/// is valid only within a single validated table snapshot and must not be
/// stored in durable player/account state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(transparent)]
pub struct RowIndex(NonZeroU32);

impl RowIndex {
    #[inline]
    #[must_use]
    pub const fn new(index: NonZeroU32) -> Self {
        Self(index)
    }

    #[inline]
    #[must_use]
    pub fn from_one_based(index: u32) -> Option<Self> {
        NonZeroU32::new(index).map(Self)
    }

    #[inline]
    #[must_use]
    pub const fn one_based(self) -> NonZeroU32 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn zero_based(self) -> u32 {
        self.0.get() - 1
    }
}
