//! Bevy-native Prefab authoring and processing primitives.

/// Fingerprint of this crate's own Rust sources, derived at build time by
/// `az-build-fingerprint`.
///
/// Asset build rules compose this into their analysis fingerprint so that
/// changing the code behind a product's bytes invalidates products built by
/// the older code. Nothing here is hand-maintained: editing any file under
/// `src/` changes the value.
pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");

extern crate self as az_prefab;

pub mod codec;
pub mod collection;
pub mod document;
pub mod migration;
pub mod multi_instance;
pub mod registration;
pub mod scene;
#[path = "semantics/typed/mod.rs"]
mod semantics;
pub mod type_data;

pub use az_prefab_derive::Prefab;
pub use codec::{PrefabCodec, PrefabCodecError};
pub use collection::{
    PREFAB_COLLECTION_SOURCE_TYPE, PREFAB_COLLECTION_VERSION, PrefabCollection,
    PrefabCollectionCodec, PrefabCollectionEntry, PrefabCollectionError,
};
pub use document::{
    EntityAlias, InstanceAlias, OverrideAction as TypedOverrideAction, OverrideOperation,
    OverrideTarget as TypedOverrideTarget, PREFAB_DOCUMENT_VERSION, PREFAB_SOURCE_TYPE,
    PrefabAssetPath, PrefabCatalogAlias, PrefabDocument, PrefabDocumentError, PrefabEntity,
    PrefabInstance, ReflectedPath, SparseValue,
};
pub use migration::{PrefabMigrationError, PrefabRegistry};
pub use multi_instance::{MULTI_INSTANCE_OF_TAG, MultiInstanceOf, register_core_prefab_types};
pub use registration::PrefabType;
pub use scene::{SCENE_EXTENSION, SCENE_PATH_PREFIX, SCENE_SOURCE_ROOT, SCENE_SOURCE_TYPE, Scene};
pub use semantics::{
    ResolvedOverride, ResolvedOverrideTarget, TypedPrefabSemantics, TypedPrefabSemanticsError,
    TypedPrefabSourceResolver,
};
pub use type_data::{
    ErasedPrefabConstructFn, ErasedPrefabInsertFn, ErasedPrefabMigrationFn, ErasedPrefabValue,
    PrefabBuildError, PrefabConstruction, PrefabMigrationStep, PrefabProductPolicy, PrefabTagAlias,
    PrefabTypeData,
};

/// Bevy's `#[reflect(Prefab)]` naming convention resolves this alias and asks
/// the derive-generated `FromType<T>` implementation for [`PrefabTypeData`].
pub type ReflectPrefab = PrefabTypeData;

#[doc(hidden)]
pub mod __private {
    pub use bevy_ecs;
    pub use bevy_reflect;
}
