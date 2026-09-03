use bevy::prelude::*;

use super::super::ids::WwiseMediaId;

/// Parsed Wwise `DIDX` embedded-media entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub struct WwiseMediaEntry {
    pub id: WwiseMediaId,
    /// Byte offset relative to the start of the `DATA` section payload.
    pub offset: u32,
    /// Encoded media size in bytes.
    pub size: u32,
}

impl WwiseMediaEntry {
    #[must_use]
    pub const fn end_offset(self) -> Option<u32> {
        self.offset.checked_add(self.size)
    }
}
