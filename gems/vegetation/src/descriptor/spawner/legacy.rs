use bevy::prelude::*;

/// Legacy static mesh vegetation spawner.
///
/// Lumberyard reference: `dev/Gems/Vegetation/Code/Include/Vegetation/LegacyVegetationInstanceSpawner.h:37`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct LegacyVegetationInstanceSpawner {
    pub mesh_asset_path: Option<String>,
    pub material_asset_path: Option<String>,
    pub auto_merge: bool,
    pub use_terrain_color: bool,
    pub view_distance_ratio: f32,
    pub lod_distance_ratio: f32,
    pub wind_bending: f32,
    pub air_resistance: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub variance: f32,
    pub mesh_radius: f32,
}

impl Default for LegacyVegetationInstanceSpawner {
    fn default() -> Self {
        Self {
            mesh_asset_path: None,
            material_asset_path: None,
            auto_merge: true,
            use_terrain_color: false,
            view_distance_ratio: 1.0,
            lod_distance_ratio: 1.0,
            wind_bending: 0.1,
            air_resistance: 1.0,
            stiffness: 0.5,
            damping: 2.5,
            variance: 0.6,
            mesh_radius: 0.0,
        }
    }
}

impl LegacyVegetationInstanceSpawner {
    pub fn has_empty_asset_references(&self) -> bool {
        self.mesh_asset_path.as_deref().is_none_or(str::is_empty)
    }
}
