use bevy::prelude::*;

use super::super::ids::WwiseSectionId;

/// Raw section location inside a Wwise `.bnk` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub struct WwiseBankSection {
    pub id: WwiseSectionId,
    /// Byte offset to the section payload, after the 8-byte section header.
    pub offset: u32,
    /// Section payload size in bytes.
    pub size: u32,
}
