use az_core::crc::Crc32;
use az_derive::AzTypeInfo;
use bevy::prelude::*;
use gridmate::Marshaler;
use serde::{Deserialize, Serialize};

/// Invalid ATL control identifier.
pub const INVALID_AUDIO_CONTROL_ID: AudioControlId = AudioControlId(0);

/// ATL control identifier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
#[repr(transparent)]
pub struct AudioControlId(pub u64);

impl AudioControlId {
    pub const INVALID: Self = INVALID_AUDIO_CONTROL_ID;

    #[must_use]
    pub fn from_name(name: &str) -> Self {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            Self::INVALID
        } else {
            Self(u64::from(Crc32::from_str_lower(trimmed).value()))
        }
    }

    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

/// Audio obstruction and occlusion calculation mode.
#[repr(u32)]
#[derive(
    AzTypeInfo,
    Marshaler,
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Reflect,
    Serialize,
    Deserialize,
)]
#[az_type_info(
    name = "EAudioObjectObstructionCalcType",
    "60824763-3B5B-4993-BF27-E405B95F115F"
)]
#[reflect(Serialize, Deserialize)]
pub enum AudioObstructionType {
    Ignore = 0,
    SingleRay = 1,
    MultiRay = 2,
    ScatterRaySmall = 3,
    ScatterRayLarge = 4,
    #[default]
    None = 5,
    UseLinkedProxy = 6,
}

impl AudioObstructionType {
    pub const NATIVE_NAMES: [&'static str; 7] = [
        "eAOOCT_IGNORE",
        "eAOOCT_SINGLE_RAY",
        "eAOOCT_MULTI_RAY",
        "eAOOCT_SCATTER_RAY_SMALL",
        "eAOOCT_SCATTER_RAY_LARGE",
        "eAOOCT_NONE",
        "eAOOCT_USE_LINKED_PROXY",
    ];

    #[inline]
    #[must_use]
    pub const fn from_native_value(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Ignore),
            1 => Some(Self::SingleRay),
            2 => Some(Self::MultiRay),
            3 => Some(Self::ScatterRaySmall),
            4 => Some(Self::ScatterRayLarge),
            5 => Some(Self::None),
            6 => Some(Self::UseLinkedProxy),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn native_value(self) -> u32 {
        self as u32
    }

    #[inline]
    #[must_use]
    pub const fn native_name(self) -> &'static str {
        Self::NATIVE_NAMES[self as usize]
    }
}

impl From<AudioObstructionType> for u32 {
    #[inline]
    fn from(value: AudioObstructionType) -> Self {
        value.native_value()
    }
}

impl TryFrom<u32> for AudioObstructionType {
    type Error = u32;

    #[inline]
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::from_native_value(value).ok_or(value)
    }
}

impl AsRef<str> for AudioObstructionType {
    #[inline]
    fn as_ref(&self) -> &str {
        self.native_name()
    }
}

impl std::str::FromStr for AudioObstructionType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::NATIVE_NAMES
            .iter()
            .position(|candidate| *candidate == value)
            .and_then(|index| u32::try_from(index).ok())
            .and_then(Self::from_native_value)
            .ok_or_else(|| value.to_owned())
    }
}

impl std::fmt::Display for AudioObstructionType {
    #[inline]
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.native_name())
    }
}
