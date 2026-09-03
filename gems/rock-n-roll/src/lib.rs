//! `RockNRoll` physics contracts, shape assets, and Bevy integration.

mod authoring;
mod binding;
mod character;
mod configuration;
mod continuous;
mod dynamics;
mod material;
mod physics_shape;
mod reference;
#[cfg(feature = "render")]
mod render;
mod shape_asset;
mod shape_codec;
mod sleep;
mod system;
mod terrain;

use az_gem_contract::{Contribution, GemContext, contribution};
use az_prefab::PrefabType;
use bevy::prelude::*;

pub use authoring::{RockNRollAuthoringSet, RockNRollBodyError};
pub use az_physics::{
    CharacterControllerCommands, CharacterSupportInfo, CharacterSupportState,
    LinkedSoftBodyAerodynamics, LinkedSoftBodyClusterConfiguration, LinkedSoftBodyCollisionFeature,
    LinkedSoftBodyConfiguration, LinkedSoftBodyMediumVelocityAnimation,
    LinkedSoftBodyMediumVelocityMode, LinkedSoftBodyStatus, LinkedSoftBodyStatusRef,
    LinkedSoftBodyVertexEnvelope, PhysicsLinkedSoftBodyBackend,
};
pub use binding::{
    ShapeAssetBinding, ShapeAssetPhysicsBinding, ShapeAssetPhysicsError, ShapeAssetReferenceError,
};
pub use character::*;
pub use configuration::*;
pub use continuous::*;
pub use dynamics::*;
pub use material::*;
pub use physics_shape::*;
pub use reference::*;
#[cfg(feature = "render")]
pub use render::{ShapeAssetChildren, ShapeAssetVisual, ShapeRenderConfig};
pub use shape_asset::*;
pub use sleep::*;
pub use system::*;
pub use terrain::*;

/// Register `RockNRoll` asset loaders.
pub struct RockNRollAssetPlugin;

impl Plugin for RockNRollAssetPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<az_terrain_runtime::TerrainRuntimePlugin>() {
            app.add_plugins(az_terrain_runtime::TerrainRuntimePlugin);
        }
        app.register_type::<ShapeAssetReference>()
            .init_asset_loader::<ShapeAssetLoader>();
    }
}

/// Register `RockNRoll` runtime types and shape-asset binding systems.
///
/// This plugin is renderer-independent and belongs on tools, clients, and
/// authoritative servers alike.
pub struct RockNRollPlugin;

impl Plugin for RockNRollPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<ShapeAsset>()
            .init_resource::<SleepListeners>()
            .register_type::<ContactMaterial>()
            .register_type::<Damping>()
            .register_type::<SleepCondition>()
            .register_type::<SleepConfiguration>()
            .register_type::<ContinuousPhysicsMode>()
            .register_type::<MotionType>()
            .register_type::<RigidBodyShapeSource>()
            .register_type::<RigidBodyMaterialConfiguration>()
            .register_type::<ContinuousBodyConfiguration>()
            .register_type::<RigidBodyConfiguration>()
            .register_type_data::<RigidBodyConfiguration, az_core::ReflectAzTypeInfo>()
            .register_type::<RigidBodyComponent>()
            .register_type_data::<RigidBodyComponent, az_core::ReflectAzTypeInfo>()
            .register_type_data::<RigidBodyComponent, az_core::ReflectAzRtti>()
            .register_type::<CharacterDesc>()
            .register_type_data::<CharacterDesc, az_core::ReflectAzTypeInfo>()
            .register_type::<CharacterControllerShapeSource>()
            .register_type::<CharacterControllerConfig>()
            .register_type_data::<CharacterControllerConfig, az_core::ReflectAzTypeInfo>()
            .register_type::<CharacterControllerComponent>()
            .register_type_data::<CharacterControllerComponent, az_core::ReflectAzTypeInfo>()
            .register_type_data::<CharacterControllerComponent, az_core::ReflectAzRtti>()
            .register_type::<CharacterSupportState>()
            .register_type::<CharacterSupportInfo>()
            .register_type_data::<CharacterSupportInfo, az_core::ReflectAzTypeInfo>()
            .register_type::<RockNRollSystemComponent>()
            .register_type_data::<RockNRollSystemComponent, az_core::ReflectAzTypeInfo>()
            .register_type_data::<RockNRollSystemComponent, az_core::ReflectAzRtti>()
            .register_type::<ShapeAssetReference>()
            .register_type::<az_physics::PhysicsColliderSet>()
            .add_systems(Update, authoring::apply_system_gravity);
        authoring::configure(app);
        terrain::configure(app);
    }
}

/// Register optional debug visualization for loaded `RockNRoll` shapes.
#[cfg(feature = "render")]
pub struct RockNRollRenderPlugin;

#[cfg(feature = "render")]
impl Plugin for RockNRollRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShapeRenderConfig>()
            .init_resource::<render::ShapeRenderAssets>()
            .register_type::<ShapeRenderConfig>()
            .register_type::<ShapeAssetVisual>()
            .add_systems(
                Update,
                (
                    render::sync_shape_asset_visuals,
                    render::cleanup_removed_shape_asset_bindings,
                )
                    .chain(),
            );
    }
}

/// The AZ types this gem owns.
#[must_use]
pub const fn types() -> [az_core::AzTypeRegistration; 3] {
    [
        RigidBodyComponent::REGISTRATION,
        CharacterControllerComponent::REGISTRATION,
        RockNRollSystemComponent::REGISTRATION,
    ]
}

/// The Bevy-native Prefab component types this gem owns.
///
/// Registration-only by construction: an entry carries the reflected type and
/// nothing else, so a host takes the list without booting physics, assets,
/// schedules, or render resources.
#[must_use]
pub fn prefab_types() -> [PrefabType; 3] {
    [
        PrefabType::of::<RigidBodyComponent>(),
        PrefabType::of::<CharacterControllerComponent>(),
        PrefabType::of::<RockNRollSystemComponent>(),
    ]
}

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// The three components' AZ identities and the shape asset type travel with the
/// gem everywhere it is linked, `asset-worker` included, which is why they are
/// here and not in the pipeline-only bundle below.
struct Package;

#[contribution(package)]
impl Contribution for Package {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<az_core::AssetTypeRegistration>()
            .register_many(asset_types());
        ctx.registrar::<az_core::AzTypeRegistration>()
            .register_many(types());
    }
}

/// Sealing is privacy: the generated `prefab_types_contribution` is the only
/// way in.
///
/// Reflected prefab types are read by the hosts that resolve prefab documents
/// without booting the runtime plugin graph — `project-host` through its type
/// registry, `asset-worker` through AZSCENE analysis — so they are the roles
/// this stanza names, and nothing here needs a Bevy world.
struct Prefabs;

#[contribution(prefab_types)]
impl Contribution for Prefabs {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<PrefabType>().register_many(prefab_types());
    }
}

#[cfg(test)]
mod prefab_registration_tests {
    use std::any::TypeId;

    use super::*;

    #[test]
    fn prefab_registration_is_pure_and_complete() {
        let mut registry = bevy::reflect::TypeRegistry::default();
        for entry in prefab_types() {
            entry.apply(&mut registry);
        }

        for type_id in [
            TypeId::of::<RigidBodyComponent>(),
            TypeId::of::<CharacterControllerComponent>(),
            TypeId::of::<RockNRollSystemComponent>(),
        ] {
            assert!(
                registry
                    .get(type_id)
                    .and_then(|registration| registration.data::<az_prefab::PrefabTypeData>())
                    .is_some(),
                "registered RockNRoll Prefab type must carry PrefabTypeData"
            );
        }
    }
}
