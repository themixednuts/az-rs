//! Wwise identifier types.

use std::fmt;

use bevy::prelude::*;

/// Wwise soundbank identifier from the `BKHD` section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect)]
#[repr(transparent)]
pub struct WwiseBankId(pub u32);

/// Wwise encoded media identifier from the `DIDX` section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect)]
#[repr(transparent)]
pub struct WwiseMediaId(pub u32);

/// Wwise hierarchy object identifier from the `HIRC` section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect)]
#[repr(transparent)]
pub struct WwiseObjectId(pub u32);

/// Wwise short identifier returned by name lookup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect)]
#[repr(transparent)]
pub struct WwiseNameId(pub u32);

/// Four-byte Wwise soundbank section identifier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect)]
#[repr(transparent)]
pub struct WwiseSectionId(pub u32);

impl WwiseNameId {
    pub const INVALID: Self = Self(0);

    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self(hash_name(name.as_bytes()))
    }

    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

impl WwiseSectionId {
    pub const BKHD: Self = Self::from_tag(*b"BKHD");
    pub const DIDX: Self = Self::from_tag(*b"DIDX");
    pub const DATA: Self = Self::from_tag(*b"DATA");
    pub const HIRC: Self = Self::from_tag(*b"HIRC");
    pub const STID: Self = Self::from_tag(*b"STID");
    pub const STMG: Self = Self::from_tag(*b"STMG");
    pub const INIT: Self = Self::from_tag(*b"INIT");
    pub const ENVS: Self = Self::from_tag(*b"ENVS");
    pub const FXPR: Self = Self::from_tag(*b"FXPR");
    pub const PLAT: Self = Self::from_tag(*b"PLAT");

    #[must_use]
    pub const fn from_tag(tag: [u8; 4]) -> Self {
        Self(u32::from_le_bytes(tag))
    }

    #[must_use]
    pub const fn tag(self) -> [u8; 4] {
        self.0.to_le_bytes()
    }

    #[must_use]
    pub fn tag_string(self) -> String {
        self.tag()
            .into_iter()
            .map(|byte| {
                if byte.is_ascii_graphic() || byte == b' ' {
                    char::from(byte)
                } else {
                    '.'
                }
            })
            .collect()
    }
}

impl fmt::Display for WwiseSectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.tag_string())
    }
}

const fn hash_name(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let byte = if byte >= b'A' && byte <= b'Z' {
            byte + 0x20
        } else {
            byte
        };
        hash = hash.wrapping_mul(0x0100_0193) ^ byte as u32;
        index += 1;
    }
    hash
}
