use az_asset::AssetRef;
use az_core::{AssetId, name::AzNameCaseInsensitive};
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

use crate::{
    AnimationAssetData,
    blend_space_asset::{BlendSpaceAsset, CombinedBlendSpaceAsset},
};

pub type AnimationMotionRef = AssetRef<AnimationAssetData>;
pub type BlendSpaceRef = AssetRef<BlendSpaceAsset>;
pub type CombinedBlendSpaceRef = AssetRef<CombinedBlendSpaceAsset>;

/// Processed motion product selected by one Cry animation-set entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Reflect)]
pub enum AnimationProductRef {
    Motion(AnimationMotionRef),
    BlendSpace(BlendSpaceRef),
    CombinedBlendSpace(CombinedBlendSpaceRef),
}

impl Default for AnimationProductRef {
    fn default() -> Self {
        Self::Motion(AnimationMotionRef::default())
    }
}

impl AnimationProductRef {
    #[must_use]
    pub const fn id(&self) -> AssetId {
        match self {
            Self::Motion(reference) => reference.id(),
            Self::BlendSpace(reference) => reference.id(),
            Self::CombinedBlendSpace(reference) => reference.id(),
        }
    }

    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        match self {
            Self::Motion(reference) => reference.hint(),
            Self::BlendSpace(reference) => reference.hint(),
            Self::CombinedBlendSpace(reference) => reference.hint(),
        }
    }
}

/// One entry in a character-specific Cry animation set.
///
/// Cry retains the case-insensitive animation alias even when no product can
/// be selected without a concrete character animation set. Offline conversion
/// attaches a product only when the source identity is unambiguous; otherwise
/// the semantic alias remains available for the character link step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Reflect)]
pub struct AnimationRef {
    pub alias: AzNameCaseInsensitive,
    pub product: Option<AnimationProductRef>,
}

impl AnimationRef {
    #[must_use]
    pub fn new(alias: impl AsRef<str>, product: impl Into<AnimationProductRef>) -> Self {
        Self {
            alias: AzNameCaseInsensitive::new(alias),
            product: Some(product.into()),
        }
    }

    #[must_use]
    pub fn alias(alias: impl AsRef<str>) -> Self {
        Self {
            alias: AzNameCaseInsensitive::new(alias),
            product: None,
        }
    }

    #[must_use]
    pub fn id(&self) -> Option<AssetId> {
        self.product.as_ref().map(AnimationProductRef::id)
    }

    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.product.as_ref().and_then(AnimationProductRef::hint)
    }

    /// Whether two entries select the same processed product, or the same
    /// unresolved animation-set alias when neither has a product yet.
    #[must_use]
    pub fn references_same_motion(&self, other: &Self) -> bool {
        match (self.id(), other.id()) {
            (Some(left), Some(right)) => left == right,
            (None, None) => self.alias == other.alias,
            (Some(_), None) | (None, Some(_)) => false,
        }
    }
}

impl From<AnimationMotionRef> for AnimationProductRef {
    fn from(value: AnimationMotionRef) -> Self {
        Self::Motion(value)
    }
}

impl From<BlendSpaceRef> for AnimationProductRef {
    fn from(value: BlendSpaceRef) -> Self {
        Self::BlendSpace(value)
    }
}

impl From<CombinedBlendSpaceRef> for AnimationProductRef {
    fn from(value: CombinedBlendSpaceRef) -> Self {
        Self::CombinedBlendSpace(value)
    }
}
