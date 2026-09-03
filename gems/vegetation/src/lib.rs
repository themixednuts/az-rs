//! Vegetation Gem component data and Bevy integration.

mod area;
mod audio;
mod descriptor;
mod distribution_asset;
mod instance;
mod math;
mod modifiers;
mod path;
mod physics;
mod plugin;
mod render;
mod surface;

use az_gem_contract::{Contribution, GemContext, contribution};
use az_prefab::PrefabType;

pub(crate) use math::{count_to_f32, f32_close, quat_close, to_f32, to_i32, vec3_close};
pub(crate) use path::non_empty_path;

pub use area::*;
pub use audio::*;
pub use descriptor::*;
pub use distribution_asset::*;
pub use instance::*;
pub use modifiers::*;
pub use physics::*;
pub use plugin::*;
pub use render::*;
pub use surface::*;

/// The asset types this gem owns.
#[must_use]
pub const fn asset_types() -> [az_core::AssetTypeRegistration; 1] {
    [
        az_core::AssetTypeRegistration::for_asset::<VegetationDistributionAsset>()
            .with_owner("az-gem-vegetation"),
    ]
}

/// The AZ types this gem owns.
#[must_use]
pub const fn types() -> [az_core::AzTypeRegistration; 2] {
    [SpawnerConfig::REGISTRATION, SpawnerComponent::REGISTRATION]
}

/// The Bevy-native Prefab component types this gem owns.
///
/// Registration-only by construction: an entry carries the reflected type and
/// nothing else, so a host takes the list without booting the runtime plugin
/// graph.
#[must_use]
pub fn prefab_types() -> [PrefabType; 15] {
    [
        PrefabType::of::<VegetationAreaComponent>(),
        PrefabType::of::<VegetationBlockerComponent>(),
        PrefabType::of::<ReferenceShapeComponent>(),
        PrefabType::of::<AreaBlenderComponent>(),
        PrefabType::of::<SpawnerComponent>(),
        PrefabType::of::<VegetationDescriptorListComponent>(),
        PrefabType::of::<DescriptorWeightSelectorComponent>(),
        PrefabType::of::<PositionModifierComponent>(),
        PrefabType::of::<RotationModifierComponent>(),
        PrefabType::of::<ScaleModifierComponent>(),
        PrefabType::of::<SlopeAlignmentModifierComponent>(),
        PrefabType::of::<SurfaceAltitudeFilterComponent>(),
        PrefabType::of::<DistributionFilterComponent>(),
        PrefabType::of::<SurfaceMaskFilterComponent>(),
        PrefabType::of::<SurfaceSlopeFilterComponent>(),
    ]
}

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// The spawner's AZ identity and the distribution asset type travel with the
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
mod tests;
