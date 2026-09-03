use bevy::prelude::*;

use super::super::ids::WwiseBankId;

/// Parsed Wwise `BKHD` soundbank header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub struct WwiseBankHeader {
    pub version: u32,
    pub bank_id: WwiseBankId,
    pub language_id: Option<u32>,
    pub feedback_in_bank: Option<u32>,
}
