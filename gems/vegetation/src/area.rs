//! Vegetation area and spawner component data.

use bevy::prelude::*;

mod blender;
mod blocker;
mod claim;
mod component;
mod constants;
mod info;
mod reference_shape;
mod spawner;
mod system;

pub use blender::{
    AreaBlenderComponent, AreaBlenderConfig, VEGETATION_AREA_BLENDER_COMPONENT_TYPE_ID,
    VEGETATION_AREA_BLENDER_CONFIG_TYPE_ID,
};
pub use blocker::{
    VEGETATION_BLOCKER_COMPONENT_TYPE_ID, VEGETATION_BLOCKER_CONFIG_TYPE_ID,
    VegetationBlockerComponent, VegetationBlockerConfig,
};
pub use claim::{ClaimContext, ClaimHandle, ClaimPoint};
pub use component::{
    VEGETATION_AREA_COMPONENT_TYPE_ID, VEGETATION_AREA_CONFIG_TYPE_ID, VegetationAreaComponent,
    VegetationAreaConfig, VegetationAreaType, VegetationPlacementModeTypeFlag,
};
pub use constants::*;
pub use info::VegetationAreaInfo;
pub use reference_shape::{
    ReferenceShapeComponent, ReferenceShapeConfig, VEGETATION_REFERENCE_SHAPE_COMPONENT_TYPE_ID,
    VEGETATION_REFERENCE_SHAPE_CONFIG_TYPE_ID,
};
pub use spawner::{
    AreaConfig, SpawnerComponent, SpawnerConfig, SpawnerFilterSet, SpawnerModifierSet,
    SpawnerProcessingSet,
};
pub use system::{
    AreaSystemConfig, SectorId, SectorPointGrid, SnapMode, VegetationSectorInstance, ViewRect,
};
use system::{FilledVegetationSectors, spawn_terrain_vegetation_sectors};

pub fn register_area_components(app: &mut App) {
    app.init_resource::<AreaSystemConfig>()
        .init_resource::<FilledVegetationSectors>()
        .register_type::<SnapMode>()
        .register_type::<ViewRect>()
        .register_type::<VegetationSectorInstance>()
        .register_type::<ClaimHandle>()
        .register_type::<ClaimPoint>()
        .register_type::<ClaimContext>()
        .register_type::<VegetationAreaType>()
        .register_type::<VegetationPlacementModeTypeFlag>()
        .register_type::<VegetationAreaConfig>()
        .register_type::<VegetationAreaComponent>()
        .register_type::<AreaConfig>()
        .register_type::<AreaBlenderConfig>()
        .register_type::<AreaBlenderComponent>()
        .register_type::<VegetationBlockerConfig>()
        .register_type::<VegetationBlockerComponent>()
        .register_type::<ReferenceShapeConfig>()
        .register_type::<ReferenceShapeComponent>()
        .register_type::<SpawnerConfig>()
        .register_type::<SpawnerComponent>()
        .register_type::<AreaSystemConfig>()
        .register_type::<VegetationAreaInfo>()
        .add_systems(Update, spawn_terrain_vegetation_sectors);
}

#[cfg(test)]
mod tests;
