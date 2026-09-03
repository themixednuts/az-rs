//! Engine-owned AZSCENE products.
//!
//! Authoring tools resolve sparse Prefab sources into a temporary Bevy world,
//! capture a [`DynamicWorld`], and encode it with this crate. Runtime only sees
//! the resulting versioned product and materializes registered components.

/// Fingerprint of this crate's own Rust sources, derived at build time by
/// `az-build-fingerprint`.
///
/// Asset build rules compose this into their analysis fingerprint so that
/// changing the code behind a product's bytes invalidates products built by
/// the older code. Nothing here is hand-maintained: editing any file under
/// `src/` changes the value.
pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");

mod asset_handles;
mod codec;
mod materialize;
mod reflected_value_wire;
mod runtime;
mod structural_serde;

use az_core::{AssetData, AssetType, AzRtti, AzTypeInfo, EntityId, component::ComponentId};
use bevy::{
    app::{App, Plugin},
    asset::{Asset, AssetApp},
    ecs::{
        entity::Entity,
        schedule::{IntoScheduleConfigs, SystemSet},
    },
    reflect::TypePath,
    world_serialization::{DynamicEntity, DynamicWorld},
};
use std::collections::BTreeMap;
use thiserror::Error;

pub use codec::{
    AZSCENE_FORMAT_VERSION, AZSCENE_MAGIC, AZSCENE_PRODUCT_EXTENSION, AzSceneAssetLoader,
    AzSceneCodecError, EncodedAzScene, encode_scene_asset, read_scene_asset_from_reader,
    read_scene_asset_with_loader, read_scene_metadata_from_reader, write_scene_asset,
};
pub use materialize::{AzSceneInstance, AzSceneMaterializeError};
pub use runtime::{
    AzSceneEntityResolver, AzSceneInstanceFailed, AzSceneInstanceMember, AzSceneInstanceSnapshot,
    AzSceneRoot, AzSceneRuntimeComponentTarget,
};

/// Runtime-safe AZ asset marker for processed scene products.
pub struct AzSceneAssetData;

impl AzTypeInfo for AzSceneAssetData {
    const NAME: &'static str = "Azoth::AzSceneAsset";
    const TYPE_ID: uuid::Uuid = uuid::Uuid::from_u128(0xd228_fbf5_b910_40fe_b1af_2c46_1b9c_006e);
}

impl AzRtti for AzSceneAssetData {}

impl AssetData for AzSceneAssetData {
    const STABLE_NAME: &'static str = "azoth.scene.azscene";
}

pub const AZSCENE_ASSET_TYPE_NAME: &str = AzSceneAssetData::STABLE_NAME;
pub const AZSCENE_ASSET_TYPE: AssetType = AzSceneAssetData::ASSET_TYPE;

/// The asset types this crate owns, for a host contribution to register.
#[must_use]
pub const fn asset_types() -> [az_core::AssetTypeRegistration; 1] {
    [az_core::AssetTypeRegistration::for_asset::<AzSceneAssetData>().with_owner("az-scene")]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<az_core::AssetTypeRegistration>()
        .register_many(asset_types());
}

/// Public schedule boundary after processed scene instances exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub struct AzSceneMaterializationSet;

/// Product-local entity identity. Values are dense indexes in canonical source
/// alias order. Generated entities use their stable flattened alias as the
/// lexical tie-breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalEntityId(u32);

impl LocalEntityId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Product-local identity for one authored source-instance scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalEntityScopeId(u32);

impl LocalEntityScopeId {
    /// The top-level Prefab source scope.
    pub const ROOT: Self = Self(0);

    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Structural parent link for one authored source-instance scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AzSceneSourceScopeMetadata {
    pub parent: Option<LocalEntityScopeId>,
}

/// Stable materialization facts for one product-local entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzSceneEntityMetadata {
    /// Full flattened source alias. Nested aliases include their instance path.
    pub source_alias: String,
    /// Product-local authored source-instance scope.
    pub source_scope: LocalEntityScopeId,
    /// Native authored identity retained for source-shaped entity references.
    pub source_entity_id: Option<EntityId>,
    /// Product-local parent identity, independent of Bevy allocator state.
    pub parent: Option<LocalEntityId>,
    /// Native component identities retained for runtime component addressing.
    /// Entries are strictly ordered by native type UUID and contain only valid
    /// ids.
    pub component_targets: Vec<AzSceneComponentTarget>,
}

/// One typed native component target processed beside an AZSCENE entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AzSceneComponentTarget {
    pub native_type_id: uuid::Uuid,
    pub component_id: ComponentId,
}

/// Product metadata kept beside the reflected dynamic world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzSceneMetadata {
    /// Canonical source-instance scope table. Entry zero is always the root.
    pub source_scopes: Vec<AzSceneSourceScopeMetadata>,
    /// Entries correspond one-for-one with `dynamic_world.entities`.
    pub entities: Vec<AzSceneEntityMetadata>,
}

impl Default for AzSceneMetadata {
    fn default() -> Self {
        Self {
            source_scopes: vec![AzSceneSourceScopeMetadata { parent: None }],
            entities: Vec::new(),
        }
    }
}

/// A loaded AZSCENE product ready for registered materialization.
#[derive(Asset, TypePath)]
pub struct AzSceneAsset {
    pub dynamic_world: DynamicWorld,
    /// Canonical, deduplicated asset paths observed by the handle processors.
    pub dependencies: Vec<String>,
    pub metadata: AzSceneMetadata,
}

impl AzSceneAsset {
    #[must_use]
    pub const fn new(dynamic_world: DynamicWorld, metadata: AzSceneMetadata) -> Self {
        Self {
            dynamic_world,
            dependencies: Vec::new(),
            metadata,
        }
    }

    /// Builds an asset whose dynamic entities follow a caller-owned canonical
    /// order. `DynamicWorldBuilder` orders by Bevy's allocator identity, which
    /// is unsuitable for a stable product format, so processors must supply the
    /// corresponding source-alias order explicitly.
    ///
    /// # Errors
    ///
    /// Returns an error if `entity_order` disagrees with the metadata or the
    /// captured world: a differing entry count, an ordered entity absent from
    /// the capture, or a captured entity absent from the order.
    pub fn new_in_entity_order(
        mut dynamic_world: DynamicWorld,
        entity_order: &[Entity],
        metadata: AzSceneMetadata,
    ) -> Result<Self, AzSceneEntityOrderError> {
        if entity_order.len() != metadata.entities.len() {
            return Err(AzSceneEntityOrderError::MetadataCount {
                order: entity_order.len(),
                metadata: metadata.entities.len(),
            });
        }
        let mut extracted = dynamic_world
            .entities
            .drain(..)
            .map(|entity| (entity.entity, entity))
            .collect::<BTreeMap<Entity, DynamicEntity>>();
        if extracted.len() != entity_order.len() {
            return Err(AzSceneEntityOrderError::ExtractedCount {
                order: entity_order.len(),
                extracted: extracted.len(),
            });
        }
        let mut ordered = Vec::with_capacity(entity_order.len());
        for entity in entity_order {
            let dynamic = extracted
                .remove(entity)
                .ok_or(AzSceneEntityOrderError::MissingEntity { entity: *entity })?;
            ordered.push(dynamic);
        }
        if let Some(entity) = extracted.into_keys().next() {
            return Err(AzSceneEntityOrderError::UnexpectedEntity { entity });
        }
        dynamic_world.entities = ordered;
        Ok(Self::new(dynamic_world, metadata))
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AzSceneEntityOrderError {
    #[error("canonical entity order has {order} entries but metadata has {metadata}")]
    MetadataCount { order: usize, metadata: usize },
    #[error("canonical entity order has {order} entries but capture has {extracted}")]
    ExtractedCount { order: usize, extracted: usize },
    #[error("canonical entity order contains entity {entity:?} absent from capture")]
    MissingEntity { entity: Entity },
    #[error("capture contains entity {entity:?} absent from canonical entity order")]
    UnexpectedEntity { entity: Entity },
}

impl std::fmt::Debug for AzSceneAsset {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AzSceneAsset")
            .field("entity_count", &self.dynamic_world.entities.len())
            .field("resource_count", &self.dynamic_world.resources.len())
            .field("dependencies", &self.dependencies)
            .field("metadata", &self.metadata)
            .finish()
    }
}

/// Registers the v1 product asset and loader. Adding this plugin is an explicit
/// runtime composition choice; the Prefab builder does not install it.
pub struct AzScenePlugin;

impl Plugin for AzScenePlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<AzSceneAsset>()
            .init_asset_loader::<AzSceneAssetLoader>()
            .register_type::<AzSceneRoot>()
            .init_resource::<AzSceneEntityResolver>()
            .init_resource::<runtime::AzSceneRuntime>()
            .add_systems(
                bevy::app::Update,
                runtime::materialize_az_scene_roots.in_set(AzSceneMaterializationSet),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, declare_caps,
    };

    const OWNER: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.scene-tests"),
        contribution: ContributionId::new("assets"),
        roles: &[],
    };

    declare_caps!(SceneCaps:);

    struct Scene;

    impl Contribution for Scene {
        type Caps = SceneCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            OWNER
        }

        fn register(&self, ctx: &mut GemContext<'_, SceneCaps>) {
            register(ctx);
        }
    }

    #[test]
    fn runtime_scene_crate_owns_azscene_asset_registration() {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Scene, ProductActivation::default())
            .expect("an empty floor composes");
        let registries = composer.registries();

        let registration = az_core::composed_asset_type(registries, AZSCENE_ASSET_TYPE)
            .expect("AZSCENE asset type should be composed by the runtime-safe scene crate");

        assert_eq!(registration.stable_name(), AZSCENE_ASSET_TYPE_NAME);
        assert_eq!(registration.owner(), "az-scene");
    }
}
