use std::borrow::Borrow;

use az_asset::{AssetId, AssetRef, AssetTypeMismatch, UntypedAssetRef};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ShapeAsset;

/// Canonical typed identity of a cooked Rock'n'Roll shape product.
pub type ShapeAssetRef = AssetRef<ShapeAsset>;

/// ECS component selecting a shape product by its canonical asset identity.
///
/// The optional hint carried by [`ShapeAssetRef`] is diagnostic metadata. It
/// never participates in equality, hashing, or runtime resolution.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct ShapeAssetReference(ShapeAssetRef);

impl ShapeAssetReference {
    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self(ShapeAssetRef::empty())
    }

    #[inline]
    #[must_use]
    pub const fn from_id(asset_id: AssetId) -> Self {
        Self(ShapeAssetRef::from_id(asset_id))
    }

    #[inline]
    #[must_use]
    pub const fn id(&self) -> AssetId {
        self.0.id()
    }

    #[inline]
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.0.hint()
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    #[must_use]
    pub fn into_inner(self) -> ShapeAssetRef {
        self.0
    }
}

impl AsRef<ShapeAssetRef> for ShapeAssetReference {
    #[inline]
    fn as_ref(&self) -> &ShapeAssetRef {
        &self.0
    }
}

impl Borrow<ShapeAssetRef> for ShapeAssetReference {
    #[inline]
    fn borrow(&self) -> &ShapeAssetRef {
        &self.0
    }
}

impl From<ShapeAssetRef> for ShapeAssetReference {
    #[inline]
    fn from(value: ShapeAssetRef) -> Self {
        Self(value)
    }
}

impl TryFrom<UntypedAssetRef> for ShapeAssetReference {
    type Error = ShapeAssetReferenceConversionError;

    #[inline]
    fn try_from(value: UntypedAssetRef) -> Result<Self, Self::Error> {
        if value.asset_id.is_nil() {
            return value
                .is_empty()
                .then(Self::empty)
                .ok_or(ShapeAssetReferenceConversionError::MissingAssetId);
        }
        ShapeAssetRef::try_from(value)
            .map(Self)
            .map_err(ShapeAssetReferenceConversionError::from)
    }
}

impl TryFrom<&UntypedAssetRef> for ShapeAssetReference {
    type Error = ShapeAssetReferenceConversionError;

    #[inline]
    fn try_from(value: &UntypedAssetRef) -> Result<Self, Self::Error> {
        Self::try_from(value.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShapeAssetReferenceConversionError {
    #[error("RockNRoll shape reference has source metadata but no AssetId")]
    MissingAssetId,
    #[error(transparent)]
    TypeMismatch(#[from] AssetTypeMismatch),
}

#[cfg(test)]
mod tests {
    use az_core::AssetType;

    use super::*;
    use crate::SHAPE_ASSET_TYPE;

    #[test]
    fn identity_does_not_depend_on_diagnostic_hint() {
        let asset_id = AssetId::new(uuid::uuid!("5F1668B9-7A80-4DA9-926C-7D7CFB90DBB5"), 9);
        let with_hint = ShapeAssetReference::try_from(UntypedAssetRef::new(
            asset_id,
            SHAPE_ASSET_TYPE,
            Some("objects/physics/barrier.rnr"),
        ))
        .unwrap();
        let without_hint = ShapeAssetReference::from_id(asset_id);

        assert_eq!(with_hint, without_hint);
        assert_eq!(with_hint.id(), asset_id);
        assert_eq!(with_hint.hint(), Some("objects/physics/barrier.rnr"));
    }

    #[test]
    fn rejects_a_different_asset_type() {
        let reference = UntypedAssetRef::new(
            AssetId::new(uuid::uuid!("5F1668B9-7A80-4DA9-926C-7D7CFB90DBB5"), 9),
            AssetType::new(uuid::uuid!("1FC869E8-9DA0-4A20-B912-45CA8BB665D1")),
            None::<String>,
        );

        assert!(ShapeAssetReference::try_from(reference).is_err());
    }

    #[test]
    fn rejects_a_path_hint_without_an_asset_identity() {
        assert_eq!(
            ShapeAssetReference::try_from(UntypedAssetRef::from_hint(
                "objects/physics/barrier.rnr"
            )),
            Err(ShapeAssetReferenceConversionError::MissingAssetId)
        );
    }
}
