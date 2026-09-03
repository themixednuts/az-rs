use az_gem_legacy_terrain::{TerrainRegionAsset, TerrainSurfaceWeight, TerrainWorld};
use bevy::prelude::*;

use crate::instance::InstanceData;
use crate::surface::{
    VegetationSurfaceTag, VegetationSurfaceTagWeight, add_max_surface_weight,
    merge_max_surface_weights,
};

use super::system::{AreaSystemConfig, SectorId};

const HASH_COMBINE_64_CONSTANT: u64 = 0x9e37_79b9_7f4a_7c13;

/// Vegetation claim identifier.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/Ebuses/AreaRequestBus.h:24`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect)]
#[repr(transparent)]
pub struct ClaimHandle(pub u64);

impl ClaimHandle {
    #[must_use]
    pub const fn for_sector_index(sector_id: SectorId, index: u32) -> Self {
        // Sign-extending, matching the C++ `(AZ::u64)` cast on a signed sector
        // coordinate: negative sectors have to keep hashing distinctly.
        let seed = hash_combine_64(0, (sector_id.x as i64).cast_unsigned());
        let seed = hash_combine_64(seed, (sector_id.y as i64).cast_unsigned());
        Self(hash_combine_64(seed, index as u64))
    }
}

/// A surface point available for a vegetation area to claim.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/Ebuses/AreaRequestBus.h:58`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct ClaimPoint {
    pub handle: ClaimHandle,
    pub position: Vec3,
    pub normal: Vec3,
    pub masks: Vec<VegetationSurfaceTagWeight>,
}

impl ClaimPoint {
    #[must_use]
    pub fn instance_data(&self) -> InstanceData {
        InstanceData {
            position: self.position,
            normal: self.normal,
            masks: self.masks.clone(),
            ..Default::default()
        }
    }
}

/// Surface claim inputs for one vegetation area fill pass.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/Ebuses/AreaRequestBus.h:65`.
#[derive(Debug, Clone, Default, PartialEq, Reflect)]
pub struct ClaimContext {
    pub masks: Vec<VegetationSurfaceTagWeight>,
    pub available_points: Vec<ClaimPoint>,
}

impl ClaimContext {
    #[must_use]
    pub fn from_terrain_sector(
        config: &AreaSystemConfig,
        sector_id: SectorId,
        terrain_world: &TerrainWorld,
        terrain_assets: &Assets<TerrainRegionAsset>,
    ) -> Option<Self> {
        let sector_bounds = config.sector_bounds(sector_id)?;
        let points = config.sector_point_grid(sector_bounds)?;
        let mut context = Self::default();

        for (index, point) in (1u32..).zip(points) {
            let Some(claim_point) = terrain_claim_point_at_world(
                ClaimHandle::for_sector_index(sector_id, index),
                point.x,
                point.z,
                terrain_world,
                terrain_assets,
            ) else {
                continue;
            };
            merge_max_surface_weights(&mut context.masks, claim_point.masks.iter().copied());
            context.available_points.push(claim_point);
        }

        Some(context)
    }
}

fn terrain_claim_point_at_world(
    handle: ClaimHandle,
    x: f32,
    z: f32,
    terrain_world: &TerrainWorld,
    terrain_assets: &Assets<TerrainRegionAsset>,
) -> Option<ClaimPoint> {
    let height = terrain_world.height_at_world(x, z, terrain_assets)?;
    let normal = terrain_world
        .normal_at_world(x, z, terrain_assets)
        .unwrap_or(Vec3::Y);
    let surface_weight = terrain_world.surface_weight_at_world(x, z, terrain_assets);
    let tag = terrain_surface_tag(surface_weight);
    let mut masks = Vec::with_capacity(1);
    add_max_surface_weight(&mut masks, tag, 1.0);

    Some(ClaimPoint {
        handle,
        position: Vec3::new(x, height, z),
        normal,
        masks,
    })
}

const fn terrain_surface_tag(surface_weight: Option<TerrainSurfaceWeight>) -> VegetationSurfaceTag {
    match surface_weight {
        Some(weight) if weight.is_hole() => VegetationSurfaceTag::TERRAIN_HOLE,
        _ => VegetationSurfaceTag::TERRAIN,
    }
}

const fn hash_combine_64(seed: u64, value: u64) -> u64 {
    seed ^ value
        .wrapping_add(HASH_COMBINE_64_CONSTANT)
        .wrapping_add(seed << 12)
        .wrapping_add(seed >> 4)
}
