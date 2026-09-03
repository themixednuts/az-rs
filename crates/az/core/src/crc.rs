//! `AZ::Crc32` — 32-bit CRC of a byte/string used as a fast,
//! const-evaluable name hash for events, settings, and tags.
//!
//! Polynomial: standard CRC-32 (IEEE 802.3 / ISO/IEC 13239), reflected
//! input + output, initial value `0xFFFFFFFF`, final XOR `0xFFFFFFFF`
//! — bit-identical to Lumberyard `AZ::Crc32::CalculateCrc`.
//!
//! [`Crc32`] opts into `#[reflect(PartialEq)]`, whose `bevy_reflect` expansion
//! is an `if let Some(value) = ..downcast_ref() {..} else {..}` this crate does
//! not author and cannot rewrite. The generated `impl` lands at module scope,
//! so the suppression has to live here rather than on the struct.
#![expect(
    clippy::option_if_let_else,
    reason = "fires only inside the bevy_reflect reflect(PartialEq) expansion"
)]

use bevy_reflect::std_traits::ReflectDefault;

#[cfg(feature = "serde")]
use bevy_reflect::{ReflectDeserialize, ReflectSerialize};

const CRC32_TABLE: [u32; 256] = const {
    let mut table = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut c = i;
        let mut j = 0;
        while j < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            j += 1;
        }
        table[i as usize] = c;
        i += 1;
    }
    table
};

/// Computes the IEEE 802.3 CRC-32 of `bytes`.
#[inline]
#[must_use]
pub const fn crc32(bytes: &[u8]) -> u32 {
    let mut c: u32 = 0xFFFF_FFFF;
    let mut i = 0;
    while i < bytes.len() {
        c = CRC32_TABLE[((c ^ bytes[i] as u32) & 0xFF) as usize] ^ (c >> 8);
        i += 1;
    }
    c ^ 0xFFFF_FFFF
}

/// Computes a case-insensitive CRC-32 by lowercasing ASCII bytes
/// before hashing. Matches Lumberyard's `AZ::Crc32` default which
/// lowercases by default.
#[inline]
#[must_use]
pub const fn crc32_lower(bytes: &[u8]) -> u32 {
    let mut c: u32 = 0xFFFF_FFFF;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let lower = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
        c = CRC32_TABLE[((c ^ lower as u32) & 0xFF) as usize] ^ (c >> 8);
        i += 1;
    }
    c ^ 0xFFFF_FFFF
}

/// Newtype around a 32-bit CRC. Carries the case-insensitive value
/// computed by [`crc32_lower`] when constructed from a `&str`/byte
/// slice — the canonical form Lumberyard uses for tag/event lookup.
///
/// Reflected **opaque**: a `Crc32` is a single 32-bit hash value used as a
/// `HashSet` element / `HashMap` key. The derived tuple-struct reflection would
/// represent it as a `DynamicTupleStruct` whose `reflect_hash()` is `None`,
/// panicking `DynamicSet`/`DynamicMap` hashing. Opaque reflection plus
/// `reflect(Hash, PartialEq)` routes hashing/equality through the concrete
/// impls and makes `to_dynamic()` clone the concrete value.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, bevy_reflect::Reflect,
)]
#[reflect(opaque)]
#[reflect(Hash, PartialEq, Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
#[repr(transparent)]
pub struct Crc32(pub u32);

impl Crc32 {
    /// Empty / zero CRC — matches `AZ::Crc32{}` default-constructed value.
    pub const ZERO: Self = Self(0);

    /// Construct from a string, lowercasing ASCII bytes (Lumberyard default).
    #[inline]
    #[must_use]
    pub const fn from_str_lower(s: &str) -> Self {
        Self(crc32_lower(s.as_bytes()))
    }

    /// Construct from a byte slice, lowercasing ASCII bytes.
    #[inline]
    #[must_use]
    pub const fn from_bytes_lower(bytes: &[u8]) -> Self {
        Self(crc32_lower(bytes))
    }

    /// Construct from raw bytes without case folding.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: &[u8]) -> Self {
        Self(crc32(bytes))
    }

    /// Construct from an explicit u32 value (e.g. when reading off the wire).
    #[inline]
    #[must_use]
    pub const fn from_u32(value: u32) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl From<&str> for Crc32 {
    fn from(s: &str) -> Self {
        Self::from_str_lower(s)
    }
}

impl From<u32> for Crc32 {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<Crc32> for u32 {
    fn from(c: Crc32) -> Self {
        c.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values cross-checked against Lumberyard `AZ::Crc32`
    /// (ASCII lowercase mode).
    #[test]
    fn crc32_lower_known_vectors() {
        assert_eq!(crc32_lower(b""), 0);
        // "test" — well-known IEEE 802.3 CRC-32.
        assert_eq!(crc32_lower(b"test"), 0xD87F_7E0C);
        // Case-insensitivity: "TEST" hashes the same as "test".
        assert_eq!(crc32_lower(b"TEST"), crc32_lower(b"test"));
    }

    #[test]
    fn crc32_distinguishes_distinct_inputs() {
        let a = Crc32::from_str_lower("Foo");
        let b = Crc32::from_str_lower("Bar");
        assert_ne!(a, b);
    }

    #[test]
    fn crc32_const_evaluable() {
        const TAG: Crc32 = Crc32::from_str_lower("StaticTag");
        // Value is identical between const and runtime evaluation.
        assert_eq!(TAG, Crc32::from_str_lower("statictag"));
    }
}
