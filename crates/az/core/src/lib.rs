//! Foundational AZ types that don't have a Bevy / glam equivalent.
//!
//! When Bevy already supplies a type with matching semantics, use it
//! directly:
//!
//! - AABB → [`bevy::math::bounding::Aabb3d`]
//! - Bounding sphere → [`bevy::math::bounding::BoundingSphere`]
//! - Ray → [`bevy::math::Ray3d`]
//! - Color → [`bevy::color::Color`] / [`bevy::color::LinearRgba`] / etc.
//! - Vector / matrix / quaternion → [`glam`] (re-exported by `bevy::math`)
//! - Transform → [`bevy::transform::components::Transform`]
//!
//! What lives here is the AZ-specific surface that has no Bevy analog:
//! the case-sensitive / case-insensitive interned-name types, AZ-flavored
//! CRC-32, and legacy math shapes such as `AZ::Bounds`.

pub mod asset;
pub mod asset_path;
pub mod build_identity;
pub mod component;
pub mod crc;
pub mod entity;
pub mod math;
pub mod name;
pub mod reflect;
pub mod rtti;
pub mod serialization;
pub mod type_info;
pub mod uid;
pub mod uuid;

pub use asset::{
    AssetCatalogAsset, AssetData, AssetId, AssetIdBytesError, AssetIdParseError, AssetType,
    AssetTypeRegistration, AzAssetData, ConfigAsset, DynamicSliceAsset, PakArchiveAsset,
    ScriptAsset, SliceAsset, asset_types, composed_asset_type, composed_asset_type_by_name,
    composed_asset_types,
};
pub use asset_path::{AssetPathBuf, AssetPathError};
pub use build_identity::{BuildIdentity, BuildVersion};
pub use component::lowering::{ComponentExportPolicy, ComponentLoweringRegistration};
pub use component::{AzComponent, ComponentConfig, ComponentId, EntityId};
#[cfg(feature = "bevy")]
pub use component::{AzEntityComponentIdentityPlugin, AzEntityIndex, AzEntityIndexPlugin};
pub use reflect::editor::{
    EditorFieldAttributes, EditorFieldConstraints, EditorNumericRange, EditorTypeAttributes,
    EditorWidget, register_editor_builtins,
};
pub use reflect::validation::{
    ApplicabilityContext, ApplicabilityResult, ApplicabilityTypeData, DiagnosticSeverity,
    EditorActionId, EditorActionOutcome, EditorChangeNotification, EditorPathPolicy,
    EditorPolicyError, EditorPolicyResult, EditorPolicyTypeData, ReflectedPath,
    ReflectedPathSegment, ValidationCallbackError, ValidationDiagnostic, ValidationTypeData,
};
pub use reflect::{
    ReflectAzRtti, ReflectAzTypeInfo, ReflectedValueEncoding, ReflectedValueEnvelope,
};
pub use rtti::{AzRtti, AzTypeKind, AzTypeRegistration, TypeEntry};
pub use serialization::data_patch::{AddressType, AddressTypeElement, LegacyDataPatch};
pub use type_info::AzTypeInfo;
pub use uid::Uid;
