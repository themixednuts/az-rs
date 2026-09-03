//! Typed runtime asset reference.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use az_core::{AssetData, AssetType, AzRtti, AzTypeInfo};
use bevy_reflect::{FromType, Reflect, ReflectDeserialize, ReflectSerialize, TypePath};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{AssetId, UntypedAssetRef};

/// Typed reference to an AZ asset product.
///
/// The canonical [`AssetId`] is the complete serialized identity. The optional
/// path hint is diagnostic metadata and does not participate in equality,
/// hashing, emptiness, or runtime resolution.
///
/// The product type is carried by `T` and therefore costs no runtime storage.
/// This is not a loaded runtime handle; loading remains owned by the
/// engine/framework layer.
#[derive(Reflect, Serialize, Deserialize)]
#[reflect(opaque, type_path = false, Serialize, Deserialize, AssetDependency)]
#[reflect(where)]
#[serde(bound = "")]
pub struct AssetRef<T: AssetData> {
    asset_id: AssetId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path_hint: Option<String>,
    #[serde(skip)]
    _marker: PhantomData<fn() -> T>,
}

/// Reflection metadata for extracting canonical product dependencies.
#[derive(Debug, Clone)]
pub struct ReflectAssetDependency {
    dependency_id: fn(&dyn Reflect) -> Option<AssetId>,
}

impl ReflectAssetDependency {
    #[must_use]
    pub fn dependency_id(&self, value: &dyn Reflect) -> Option<AssetId> {
        (self.dependency_id)(value)
    }
}

impl<T: AssetData> FromType<AssetRef<T>> for ReflectAssetDependency {
    fn from_type() -> Self {
        Self {
            dependency_id: |value| {
                value
                    .downcast_ref::<AssetRef<T>>()
                    .map(AssetRef::id)
                    .filter(|asset_id| asset_id.is_valid())
            },
        }
    }
}

impl FromType<UntypedAssetRef> for ReflectAssetDependency {
    fn from_type() -> Self {
        Self {
            dependency_id: |value| {
                value
                    .downcast_ref::<UntypedAssetRef>()
                    .map(|reference| reference.asset_id)
                    .filter(|asset_id| asset_id.is_valid())
            },
        }
    }
}

impl<T: AssetData> AssetRef<T> {
    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            asset_id: AssetId::nil(),
            path_hint: None,
            _marker: PhantomData,
        }
    }

    #[inline]
    #[must_use]
    pub fn to_untyped(&self) -> UntypedAssetRef {
        UntypedAssetRef::new(self.asset_id, T::ASSET_TYPE, self.path_hint.clone())
    }

    #[inline]
    #[must_use]
    pub fn into_untyped(self) -> UntypedAssetRef {
        UntypedAssetRef::new(self.asset_id, T::ASSET_TYPE, self.path_hint)
    }

    #[inline]
    #[must_use]
    pub const fn id(&self) -> AssetId {
        self.asset_id
    }

    #[inline]
    #[must_use]
    pub const fn asset_type(&self) -> AssetType {
        T::ASSET_TYPE
    }

    #[inline]
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.path_hint
            .as_deref()
            .map(str::trim)
            .filter(|hint| !hint.is_empty())
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.asset_id.is_nil()
    }

    #[inline]
    #[must_use]
    pub fn new(asset_id: AssetId, hint: Option<impl Into<String>>) -> Self {
        Self {
            asset_id,
            path_hint: hint.map(Into::into),
            _marker: PhantomData,
        }
    }

    #[inline]
    #[must_use]
    pub const fn from_id(asset_id: AssetId) -> Self {
        Self {
            asset_id,
            path_hint: None,
            _marker: PhantomData,
        }
    }

    /// # Errors
    ///
    /// Returns [`AssetTypeMismatch`] when `reference` carries a non-nil asset
    /// id whose asset type does not match `T::ASSET_TYPE`.
    #[inline]
    pub fn from_untyped(reference: UntypedAssetRef) -> Result<Self, AssetTypeMismatch> {
        let expected = T::ASSET_TYPE;
        if reference.asset_id.is_nil() || reference.asset_type == expected {
            Ok(Self {
                asset_id: reference.asset_id,
                path_hint: reference.hint,
                _marker: PhantomData,
            })
        } else {
            Err(AssetTypeMismatch {
                expected,
                actual: reference.asset_type,
            })
        }
    }
}

impl<T: AssetData> TypePath for AssetRef<T> {
    #[inline]
    fn type_path() -> &'static str {
        std::any::type_name::<Self>()
    }

    #[inline]
    fn short_type_path() -> &'static str {
        std::any::type_name::<Self>()
    }

    #[inline]
    fn type_ident() -> Option<&'static str> {
        Some("AssetRef")
    }

    #[inline]
    fn crate_name() -> Option<&'static str> {
        Some("az_asset")
    }

    #[inline]
    fn module_path() -> Option<&'static str> {
        Some("az_asset::asset")
    }
}

impl<T: AssetData> Clone for AssetRef<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            asset_id: self.asset_id,
            path_hint: self.path_hint.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T: AssetData> fmt::Debug for AssetRef<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AssetRef")
            .field("asset_id", &self.asset_id)
            .field("asset_type", &T::ASSET_TYPE)
            .field("path_hint", &self.path_hint)
            .finish()
    }
}

impl<T: AssetData> PartialEq for AssetRef<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.asset_id == other.asset_id
    }
}

impl<T: AssetData> Eq for AssetRef<T> {}

impl<T: AssetData> Hash for AssetRef<T> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.asset_id.hash(state);
    }
}

impl<T: AssetData> Default for AssetRef<T> {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl<T: AssetData> TryFrom<UntypedAssetRef> for AssetRef<T> {
    type Error = AssetTypeMismatch;

    #[inline]
    fn try_from(reference: UntypedAssetRef) -> Result<Self, Self::Error> {
        Self::from_untyped(reference)
    }
}

impl<T: AssetData> From<AssetRef<T>> for UntypedAssetRef {
    #[inline]
    fn from(asset: AssetRef<T>) -> Self {
        asset.into_untyped()
    }
}

impl<T: AssetData> AzTypeInfo for AssetRef<T> {
    const NAME: &'static str = "AZ::Data::Asset";
    const TYPE_ID: Uuid = az_core::uuid::az_data_asset(T::TYPE_ID);
}

impl<T: AssetData> AzRtti for AssetRef<T> {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("asset reference type {actual} does not match expected {expected}")]
pub struct AssetTypeMismatch {
    pub expected: AssetType,
    pub actual: AssetType,
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use bevy_reflect::{
        FromReflect, ReflectDeserialize, ReflectFromReflect, ReflectSerialize, TypeRegistry,
        serde::{ReflectDeserializer, ReflectSerializer},
    };
    use serde::de::DeserializeSeed;

    use super::*;

    #[derive(Debug)]
    struct MaterialAsset;

    impl AzTypeInfo for MaterialAsset {
        const NAME: &'static str = "LmbrCentral::MaterialAsset";
        const TYPE_ID: Uuid = uuid::uuid!("C2869E3B-DDA0-4E01-8FE3-6770D788866B");
    }

    impl AzRtti for MaterialAsset {}
    impl AssetData for MaterialAsset {
        const STABLE_NAME: &'static str = "az.lmbr-central.material";
    }

    #[test]
    fn typed_asset_ref_folds_asset_data_type_id() {
        assert_eq!(
            <AssetRef<MaterialAsset> as AzTypeInfo>::TYPE_ID,
            az_core::uuid::az_data_asset(<MaterialAsset as AzTypeInfo>::TYPE_ID)
        );
    }

    #[test]
    fn typed_asset_ref_sets_expected_asset_type() {
        let asset_id = AssetId::new(Uuid::from_u128(1), 7);
        let asset = AssetRef::<MaterialAsset>::new(asset_id, Some("materials/foo.mtl"));

        assert_eq!(asset.id(), asset_id);
        assert_eq!(
            asset.asset_type(),
            AssetType::new(<MaterialAsset as AzTypeInfo>::TYPE_ID)
        );
        assert_eq!(asset.hint(), Some("materials/foo.mtl"));
    }

    #[test]
    fn reflected_asset_dependency_extracts_canonical_id() {
        let asset_id = AssetId::new(Uuid::from_u128(2), 9);
        let reference = AssetRef::<MaterialAsset>::from_id(asset_id);
        let metadata = <ReflectAssetDependency as FromType<AssetRef<MaterialAsset>>>::from_type();

        assert_eq!(metadata.dependency_id(&reference), Some(asset_id));
        assert_eq!(
            metadata.dependency_id(&AssetRef::<MaterialAsset>::empty()),
            None
        );
    }

    #[test]
    fn typed_asset_ref_rejects_wrong_reference_type() {
        let reference = UntypedAssetRef::from_id(
            AssetId::new(Uuid::from_u128(2), 0),
            AssetType::new(Uuid::from_u128(3)),
        );

        let error = AssetRef::<MaterialAsset>::from_untyped(reference).unwrap_err();
        assert_eq!(
            error.expected,
            AssetType::new(<MaterialAsset as AzTypeInfo>::TYPE_ID)
        );
        assert_eq!(error.actual, AssetType::new(Uuid::from_u128(3)));
    }

    #[test]
    fn typed_asset_ref_identity_ignores_path_hint_for_equality_and_hashing() {
        let asset_id = AssetId::new(Uuid::from_u128(4), 11);
        let correct = AssetRef::<MaterialAsset>::new(asset_id, Some("materials/correct.mtl"));
        let wrong = AssetRef::<MaterialAsset>::new(asset_id, Some("wrong/diagnostic-only.txt"));
        let empty = AssetRef::<MaterialAsset>::new(asset_id, Some(""));

        assert_eq!(correct, wrong);
        assert_eq!(correct, empty);
        assert_eq!(HashSet::from([correct, wrong, empty]).len(), 1);
    }

    #[test]
    fn typed_asset_ref_round_trips_typed_ron_with_optional_path_hint() {
        let asset_id = AssetId::new(Uuid::from_u128(5), 12);
        let with_hint = AssetRef::<MaterialAsset>::new(asset_id, Some("materials/diagnostic.mtl"));

        let with_hint_ron = ron::to_string(&with_hint).unwrap();
        let restored: AssetRef<MaterialAsset> = ron::from_str(&with_hint_ron).unwrap();

        assert_eq!(restored.id(), asset_id);
        assert_eq!(restored.hint(), Some("materials/diagnostic.mtl"));
        assert!(with_hint_ron.contains("asset_id"));
        assert!(with_hint_ron.contains("path_hint"));
        assert!(!with_hint_ron.contains("asset_type"));

        let without_hint = AssetRef::<MaterialAsset>::from_id(asset_id);
        let without_hint_ron = ron::to_string(&without_hint).unwrap();
        let restored: AssetRef<MaterialAsset> = ron::from_str(&without_hint_ron).unwrap();

        assert_eq!(restored.id(), asset_id);
        assert_eq!(restored.hint(), None);
        assert!(!without_hint_ron.contains("path_hint"));
    }

    #[test]
    fn typed_asset_ref_round_trips_reflected_ron_snapshot() {
        let asset = AssetRef::<MaterialAsset>::new(
            AssetId::new(Uuid::from_u128(6), 13),
            Some("materials/snapshot.mtl"),
        );
        let mut registry = TypeRegistry::new();
        registry.register::<AssetRef<MaterialAsset>>();
        let registration = registry
            .get(std::any::TypeId::of::<AssetRef<MaterialAsset>>())
            .expect("typed asset reference registration");
        assert!(registration.data::<ReflectSerialize>().is_some());
        assert!(registration.data::<ReflectDeserialize>().is_some());
        assert!(registration.data::<ReflectFromReflect>().is_some());
        assert!(registration.data::<ReflectAssetDependency>().is_some());

        let snapshot = ron::to_string(&ReflectSerializer::new(&asset, &registry)).unwrap();
        let mut deserializer = ron::Deserializer::from_str(&snapshot).unwrap();
        let reflected = ReflectDeserializer::new(&registry)
            .deserialize(&mut deserializer)
            .unwrap();
        let restored = AssetRef::<MaterialAsset>::from_reflect(reflected.as_partial_reflect())
            .expect("AssetRef FromReflect");

        assert_eq!(restored.id(), asset.id());
        assert_eq!(restored.hint(), asset.hint());
    }
}
