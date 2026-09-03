//! `LmbrCentral` Gem component data and Bevy integration.

mod animation;
mod audio;
mod mesh;
mod path;
mod physics;
mod registration;
mod rendering;
mod scene;
mod scripting;
mod shape;

pub mod material;

use az_gem_contract::prelude::*;
use bevy::prelude::*;

// The `register_*` helpers below arrive through the public glob re-exports at
// the bottom of this list; importing them privately as well would shadow those
// re-exports and make the public names unreachable.
pub(crate) use path::non_empty_path;

pub use animation::*;
pub use audio::*;
pub use material::*;
pub use mesh::*;
pub use physics::*;
pub use registration::{lowerings, prefab_types, types};
pub use rendering::*;
pub use scene::*;
pub use scripting::*;
pub use shape::*;

/// Register `LmbrCentral` engine asset loaders.
pub struct LmbrCentralAssetPlugin;

impl Plugin for LmbrCentralAssetPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<az_mesh_builder::MeshAssetPlugin>() {
            app.add_plugins(az_mesh_builder::MeshAssetPlugin);
        }
        app.init_asset::<MaterialAsset>()
            .init_asset::<MaterialOverrideAsset>()
            .init_asset::<VertexShapeAsset>()
            .init_asset_loader::<MaterialAssetLoader>()
            .init_asset_loader::<MaterialOverrideAssetLoader>()
            .init_asset_loader::<VertexShapeAssetLoader>();
    }
}

/// Register `LmbrCentral` components and systems.
pub struct LmbrCentralPlugin;

impl Plugin for LmbrCentralPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<az_core::AzEntityIndexPlugin>() {
            app.add_plugins(az_core::AzEntityIndexPlugin);
        }
        if !app.is_plugin_added::<az_gem_animation::CryAnimationPlugin>() {
            app.add_plugins(az_gem_animation::CryAnimationPlugin);
        }
        if !app.is_plugin_added::<CharacterAnimationEventsPlugin>() {
            app.add_plugins(CharacterAnimationEventsPlugin);
        }
        if !app.is_plugin_added::<LmbrCentralPhysicsPlugin>() {
            app.add_plugins(LmbrCentralPhysicsPlugin);
        }
        register_animation_components(app);
        register_audio_components(app);
        register_rendering_components(app);
        register_mesh_components(app);
        register_scene_components(app);
        register_scripting_components(app);

        app.register_type::<MaterialAsset>()
            .register_type::<MaterialOverrideAsset>()
            .register_type::<MaterialOverrideTarget>()
            .register_type::<MaterialOverrideSubTarget>()
            .register_type::<MaterialOverrideSwitch>()
            .register_type::<MaterialOverrideParamBlock>()
            .register_type::<MaterialOverrideParam>()
            .register_type::<MaterialOverrideValueKind>()
            .register_type::<MaterialAssetBinding>()
            .register_type::<MaterialDefinition>()
            .register_type::<MaterialTextureMap>()
            .register_type::<MaterialTextureFilter>()
            .register_type::<MaterialTextureType>()
            .register_type::<MaterialTextureReference>()
            .register_type::<MaterialPublicParam>();
    }
}

/// Register only renderer-independent `LmbrCentral` shape and `CryPhysics` data.
///
/// Servers and physics-focused tools use this plugin without pulling the
/// rendering, audio, mesh, or scene systems owned by the full gem plugin.
pub struct LmbrCentralPhysicsPlugin;

impl Plugin for LmbrCentralPhysicsPlugin {
    fn build(&self, app: &mut App) {
        if app.world().contains_resource::<AssetServer>()
            && !app.is_plugin_added::<az_mesh_builder::MeshAssetPlugin>()
        {
            app.add_plugins(az_mesh_builder::MeshAssetPlugin);
        }
        register_physics_components(app);
        register_shape_components(app);
        register_static_model_physics_components(app);
    }
}

/// The gem's runtime contribution: the AZ types and component lowerings every
/// role that links `LmbrCentral` needs.
///
/// Sealing is privacy: the generated `package_contribution` is the only way in.
struct Package;

#[contribution(package)]
impl Contribution for Package {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<az_core::AzTypeRegistration>()
            .register_many(types());
        ctx.registrar::<az_core::ComponentLoweringRegistration>()
            .register_many(lowerings());
    }
}

/// The gem's asset-worker contribution: the reflected Prefab types it owns.
///
/// Separate from `package` because the role sets differ, not because the types
/// do — a `client` never processes a prefab source.
struct Prefab;

#[contribution(prefab_types)]
impl Contribution for Prefab {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<az_prefab::PrefabType>()
            .register_many(prefab_types());
    }
}

#[cfg(test)]
mod tests;
