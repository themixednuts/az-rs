//! Amazon platform-wide identity values.

// `#[reflect(Hash, PartialEq, ...)]` expands to `FromType` impls that land as
// siblings of `Uid`, so their lint level comes from this module, not the item.
#![allow(clippy::option_if_let_else)]

use bevy_reflect::{Reflect, std_traits::ReflectDefault};

#[cfg(feature = "serde")]
use bevy_reflect::{ReflectDeserialize, ReflectSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// `Amazon::Pervasives::UID`, carried as one 16-byte UUID.
///
/// The value is reflected opaquely because native UID values are map and set
/// keys. Structural tuple reflection does not provide the concrete hash and
/// equality behavior those containers require.
#[derive(az_derive::AzRtti, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Reflect)]
#[az_rtti("3485F20A-98C0-5315-876B-21BCD23A7BC0")]
#[reflect(opaque)]
#[reflect(Hash, PartialEq, Debug, Clone, Default)]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[repr(transparent)]
pub struct Uid(Uuid);

impl Uid {
    pub const BYTE_LEN: usize = 16;
    pub const ZERO: Self = Self(Uuid::from_bytes([0; Self::BYTE_LEN]));

    #[inline]
    #[must_use]
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: [u8; Self::BYTE_LEN]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    #[inline]
    #[must_use]
    pub const fn value(self) -> Uuid {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    #[inline]
    #[must_use]
    pub const fn bytes(self) -> [u8; Self::BYTE_LEN] {
        *self.0.as_bytes()
    }

    #[inline]
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0.is_nil()
    }
}

impl Default for Uid {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}
