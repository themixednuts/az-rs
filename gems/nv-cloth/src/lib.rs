mod asset;
mod fabric;
mod runtime;
mod solver;

use asset::{ClothFabricAssetLoader, ClothMaterialAssetLoader};
use az_gem_contract::{Contribution, GemContext, contribution};
use az_nv_cloth::{
    ClothFabricAsset, ClothMaterial, ClothMaterialAsset, FabricPhaseConfigs,
    MotionConstraintConfig, PhaseConfig, SelfCollisionConfig, TetherConstraintConfig,
};
use bevy::prelude::*;

pub use fabric::{DistanceConstraint, SharedFabric, SharedFabricCache, TetherConstraint};
pub use runtime::{ClothInstance, ClothInstanceError, ClothWorldParameters};
pub use solver::{
    ClothAdvanceResult, ClothCapsuleCollider, ClothParticleTarget, ClothSimulationFrame,
    ClothSolver,
};

pub struct NvClothPlugin;

impl Plugin for NvClothPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<ClothFabricAsset>()
            .init_asset::<ClothMaterialAsset>()
            .init_asset_loader::<ClothFabricAssetLoader>()
            .init_asset_loader::<ClothMaterialAssetLoader>()
            .register_type::<ClothMaterialAsset>()
            .register_type::<ClothMaterial>()
            .register_type::<FabricPhaseConfigs>()
            .register_type::<PhaseConfig>()
            .register_type::<MotionConstraintConfig>()
            .register_type::<SelfCollisionConfig>()
            .register_type::<TetherConstraintConfig>();
        runtime::register_runtime(app);
    }
}

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// The cloth asset types, product formats, and build rules belong to
/// `az-nv-cloth`, and the engine's `builders` bundle is what composes that
/// crate's `register`. What this gem adds on top is a Bevy solver runtime and
/// two Bevy asset loaders, none of which is a registry entry, so an empty
/// `register` is the honest shape of it. The compose-seam test holds it to
/// empty so a registration added later has to be declared here.
struct Package;

#[contribution]
impl Contribution for Package {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}
