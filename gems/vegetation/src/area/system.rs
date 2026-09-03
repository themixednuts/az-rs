use std::collections::HashSet;

use az_gem_gradient_signal::GradientSourceQuery;
use az_gem_legacy_terrain::{TerrainRegionAsset, TerrainWorld};
use bevy::prelude::*;
use bevy::{math::Vec3A, math::bounding::Aabb3d};

use super::claim::ClaimContext;
use super::component::VegetationAreaComponent;
use super::info::VegetationAreaInfo;
use super::spawner::{
    SpawnerComponent, SpawnerFilterSet, SpawnerModifierSet, SpawnerProcessingSet,
};
use crate::descriptor::{DescriptorWeightSelectorComponent, VegetationDescriptorListComponent};
use crate::instance::InstanceData;
use crate::modifiers::{
    DistributionFilterComponent, PositionModifierComponent, RotationModifierComponent,
    ScaleModifierComponent, SlopeAlignmentModifierComponent, SurfaceAltitudeFilterComponent,
    SurfaceMaskFilterComponent, SurfaceSlopeFilterComponent,
};
use crate::{count_to_f32, to_f32, to_i32};

/// Integer vegetation sector coordinate.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/AreaSystemComponent.h:169`.
pub type SectorId = IVec2;

/// Sector point snap mode.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/AreaSystemComponent.h:33`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum SnapMode {
    #[default]
    Corner,
    Center,
}

/// Marks an instance spawned by the vegetation sector fill system.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct VegetationSectorInstance {
    pub area: Entity,
    pub sector: SectorId,
}

#[derive(Resource, Debug, Clone, Default)]
pub struct FilledVegetationSectors {
    sectors: HashSet<SectorId>,
}

/// Vegetation area system configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/AreaSystemComponent.h:43`.
#[derive(Debug, Clone, PartialEq, Eq, Resource, Reflect)]
pub struct AreaSystemConfig {
    pub view_rectangle_size: i32,
    pub sector_density: i32,
    pub sector_size_in_meters: i32,
    pub thread_processing_interval_ms: i32,
    pub sector_search_padding: i32,
    pub sector_point_snap_mode: SnapMode,
}

impl Default for AreaSystemConfig {
    fn default() -> Self {
        Self {
            view_rectangle_size: 13,
            sector_density: 20,
            sector_size_in_meters: 16,
            thread_processing_interval_ms: 500,
            sector_search_padding: 0,
            sector_point_snap_mode: SnapMode::Corner,
        }
    }
}

impl AreaSystemConfig {
    #[must_use]
    pub fn points_per_meter(&self) -> f32 {
        if self.sector_density <= 0 || self.sector_size_in_meters <= 0 {
            return 0.0;
        }
        to_f32(self.sector_density) / to_f32(self.sector_size_in_meters)
    }

    #[must_use]
    pub const fn instances_per_sector(&self) -> i32 {
        self.sector_density * self.sector_density
    }

    #[must_use]
    pub fn sector_point_step(&self) -> Option<f32> {
        if self.sector_density <= 0 || self.sector_size_in_meters <= 0 {
            return None;
        }
        Some(to_f32(self.sector_size_in_meters) / to_f32(self.sector_density))
    }

    #[must_use]
    pub fn world_to_sector_scale(&self) -> Option<f32> {
        (self.sector_size_in_meters > 0).then_some(1.0 / to_f32(self.sector_size_in_meters))
    }

    /// Convert a Bevy world-space X/Z position into a vegetation sector id.
    ///
    /// O3DE reference: `Gems/Vegetation/Code/Source/AreaSystemComponent.cpp:889`.
    #[must_use]
    pub fn sector_id_at_world(&self, position: Vec3) -> Option<SectorId> {
        let world_to_sector = self.world_to_sector_scale()?;
        Some(SectorId::new(
            to_i32((position.x * world_to_sector).floor()),
            to_i32((position.z * world_to_sector).floor()),
        ))
    }

    /// Bounds for a Bevy X/Z vegetation sector.
    ///
    /// O3DE reference: `Gems/Vegetation/Code/Source/AreaSystemComponent.cpp:1045`.
    #[must_use]
    pub fn sector_bounds(&self, sector_id: SectorId) -> Option<Aabb3d> {
        if self.sector_size_in_meters <= 0 {
            return None;
        }
        Some(sector_bounds(sector_id, self.sector_size_in_meters, 1, 1))
    }

    /// Build the active sector rectangle around a Bevy world-space camera position.
    ///
    /// O3DE reference: `Gems/Vegetation/Code/Source/AreaSystemComponent.cpp:821`.
    #[must_use]
    pub fn view_rect_at(&self, camera_position: Vec3) -> Option<ViewRect> {
        let world_to_sector = self.world_to_sector_scale()?;
        if self.view_rectangle_size <= 0 {
            return None;
        }

        let half_view_size = self.view_rectangle_size >> 1;
        let offset = to_f32(half_view_size.saturating_mul(self.sector_size_in_meters));
        let x = to_i32((camera_position.x - offset) * world_to_sector);
        let y = to_i32((camera_position.z - offset) * world_to_sector);

        Some(ViewRect::new(
            x,
            y,
            self.view_rectangle_size,
            self.view_rectangle_size,
            self.sector_size_in_meters,
        ))
    }

    #[must_use]
    pub fn sector_point_grid(&self, sector_bounds: Aabb3d) -> Option<SectorPointGrid> {
        let step = self.sector_point_step()?;
        let density = usize::try_from(self.sector_density).ok()?;
        let snap_offset = match self.sector_point_snap_mode {
            SnapMode::Corner => 0.0,
            SnapMode::Center => 0.5,
        };
        let origin = sector_bounds.min + Vec3A::new(step * snap_offset, 0.0, step * snap_offset);

        Some(SectorPointGrid::new(origin.into(), step, density))
    }
}

/// Placement points for one vegetation sector.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/AreaSystemComponent.cpp:1094`.
#[derive(Debug, Clone)]
pub struct SectorPointGrid {
    origin: Vec3,
    step: f32,
    density: usize,
    next_index: usize,
}

impl SectorPointGrid {
    #[must_use]
    pub const fn new(origin: Vec3, step: f32, density: usize) -> Self {
        Self {
            origin,
            step,
            density,
            next_index: 0,
        }
    }

    #[must_use]
    pub const fn total_len(&self) -> usize {
        self.density.saturating_mul(self.density)
    }
}

impl Iterator for SectorPointGrid {
    type Item = Vec3;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_index >= self.total_len() {
            return None;
        }

        let index = self.next_index;
        self.next_index += 1;

        let x = index % self.density;
        let z = index / self.density;

        Some(Vec3::new(
            count_to_f32(x).mul_add(self.step, self.origin.x),
            self.origin.y,
            count_to_f32(z).mul_add(self.step, self.origin.z),
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len();
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for SectorPointGrid {
    fn len(&self) -> usize {
        self.total_len().saturating_sub(self.next_index)
    }
}

/// Scrolling rectangle of active vegetation sectors.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/AreaSystemComponent.h:231`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct ViewRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub bounds: Aabb3d,
}

impl Default for ViewRect {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            bounds: Aabb3d::from_min_max(Vec3A::ZERO, Vec3A::ZERO),
        }
    }
}

impl ViewRect {
    #[must_use]
    pub fn new(x: i32, y: i32, width: i32, height: i32, sector_size_in_meters: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            bounds: sector_bounds(SectorId::new(x, y), sector_size_in_meters, width, height),
        }
    }

    #[must_use]
    pub const fn min_sector(&self) -> SectorId {
        SectorId::new(self.x, self.y)
    }

    #[must_use]
    pub const fn max_sector(&self) -> SectorId {
        SectorId::new(
            self.x.saturating_add(self.width).saturating_sub(1),
            self.y.saturating_add(self.height).saturating_sub(1),
        )
    }

    #[must_use]
    pub const fn is_inside(&self, sector_id: SectorId) -> bool {
        let min = self.min_sector();
        let max = self.max_sector();
        sector_id.x >= min.x && sector_id.x <= max.x && sector_id.y >= min.y && sector_id.y <= max.y
    }

    #[must_use]
    pub fn overlap(&self, other: &Self, sector_size_in_meters: i32) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let max_x = self
            .x
            .saturating_add(self.width)
            .min(other.x.saturating_add(other.width));
        let max_y = self
            .y
            .saturating_add(self.height)
            .min(other.y.saturating_add(other.height));

        Self::new(
            x,
            y,
            max_x.saturating_sub(x).max(0),
            max_y.saturating_sub(y).max(0),
            sector_size_in_meters,
        )
    }

    #[must_use]
    pub const fn num_sectors(&self) -> usize {
        if self.width <= 0 || self.height <= 0 {
            return 0;
        }
        // Both are positive here, so `unsigned_abs` is the identity.
        (self.width.unsigned_abs() as usize).saturating_mul(self.height.unsigned_abs() as usize)
    }

    pub fn sector_ids(&self) -> impl Iterator<Item = SectorId> {
        let min_x = self.x;
        let min_y = self.y;
        let max_x = self.x.saturating_add(self.width.max(0));
        let max_y = self.y.saturating_add(self.height.max(0));

        (min_y..max_y).flat_map(move |y| (min_x..max_x).map(move |x| SectorId::new(x, y)))
    }
}

fn sector_bounds(
    sector_id: SectorId,
    sector_size_in_meters: i32,
    width: i32,
    height: i32,
) -> Aabb3d {
    let size = to_f32(sector_size_in_meters.max(0));
    let max_sector = SectorId::new(
        sector_id.x.saturating_add(width.max(0)),
        sector_id.y.saturating_add(height.max(0)),
    );
    Aabb3d::from_min_max(
        Vec3A::new(to_f32(sector_id.x) * size, 0.0, to_f32(sector_id.y) * size),
        Vec3A::new(
            to_f32(max_sector.x) * size,
            0.0,
            to_f32(max_sector.y) * size,
        ),
    )
}

// `needless_pass_by_value`: every parameter here is a Bevy system parameter.
// `Res`, `ResMut`, `Query`, `Commands` and `SystemParam` bundles are owned
// wrappers, and a borrowed signature stops satisfying `IntoSystem`, so the
// system no longer registers.
#[allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::needless_pass_by_value
)]
pub fn spawn_terrain_vegetation_sectors(
    mut commands: Commands,
    config: Res<AreaSystemConfig>,
    terrain_world: Option<Res<TerrainWorld>>,
    terrain_assets: Option<Res<Assets<TerrainRegionAsset>>>,
    mut filled: ResMut<FilledVegetationSectors>,
    cameras: Query<&Transform, With<Camera>>,
    gradients: GradientSourceQuery,
    areas: Query<(
        Entity,
        &SpawnerComponent,
        &VegetationDescriptorListComponent,
        Option<&VegetationAreaComponent>,
        Option<&VegetationAreaInfo>,
        Option<&DistributionFilterComponent>,
        Option<&SurfaceAltitudeFilterComponent>,
        Option<&SurfaceMaskFilterComponent>,
        Option<&SurfaceSlopeFilterComponent>,
        Option<&DescriptorWeightSelectorComponent>,
        Option<&PositionModifierComponent>,
        Option<&RotationModifierComponent>,
        Option<&ScaleModifierComponent>,
        Option<&SlopeAlignmentModifierComponent>,
    )>,
) {
    let Some(terrain_world) = terrain_world else {
        return;
    };
    let Some(terrain_assets) = terrain_assets else {
        return;
    };
    let Some(camera_transform) = cameras.iter().next() else {
        return;
    };
    let Some(view_rect) = config.view_rect_at(camera_transform.translation) else {
        return;
    };

    for sector in view_rect.sector_ids() {
        if filled.sectors.contains(&sector) {
            continue;
        }
        let Some(sector_bounds) = config.sector_bounds(sector) else {
            continue;
        };
        let Some(mut context) =
            ClaimContext::from_terrain_sector(&config, sector, &terrain_world, &terrain_assets)
        else {
            continue;
        };
        if context.available_points.is_empty() {
            filled.sectors.insert(sector);
            continue;
        }

        let mut active_areas = Vec::new();
        for (
            area,
            spawner,
            descriptors,
            vegetation_area,
            area_info,
            distribution_filter,
            altitude_filter,
            surface_mask_filter,
            slope_filter,
            descriptor_weight_selector,
            position_modifier,
            rotation_modifier,
            scale_modifier,
            slope_alignment_modifier,
        ) in &areas
        {
            if area_info.is_some_and(|area_info| !aabb_overlaps_xz(area_info.bounds, sector_bounds))
            {
                continue;
            }

            let area_config = vegetation_area.map_or_else(
                || spawner.configuration.area.clone(),
                VegetationAreaComponent::area_config,
            );
            active_areas.push(ActiveArea {
                entity: area,
                layer: area_config.layer,
                priority: area_config.priority,
                spawner,
                descriptors,
                processing: SpawnerProcessingSet {
                    selector: descriptor_weight_selector.map(|selector| &selector.configuration),
                    filters: SpawnerFilterSet {
                        distribution: distribution_filter.map(|filter| &filter.configuration),
                        altitude: altitude_filter.map(|filter| &filter.configuration),
                        surface_mask: surface_mask_filter.map(|filter| &filter.configuration),
                        slope: slope_filter.map(|filter| &filter.configuration),
                    },
                    modifiers: SpawnerModifierSet {
                        position: position_modifier.map(|modifier| &modifier.configuration),
                        rotation: rotation_modifier.map(|modifier| &modifier.configuration),
                        scale: scale_modifier.map(|modifier| &modifier.configuration),
                        slope_alignment: slope_alignment_modifier
                            .map(|modifier| &modifier.configuration),
                    },
                },
            });
        }

        active_areas.sort_by_key(|area| std::cmp::Reverse((area.layer, area.priority)));

        for active_area in active_areas {
            if context.available_points.is_empty() {
                break;
            }

            let instances = active_area.spawner.claim_positions_with_gradient_sources(
                Some(active_area.entity),
                &mut context,
                &active_area.descriptors.configuration,
                active_area.processing,
                &gradients,
            );

            for instance in instances {
                spawn_sector_instance(&mut commands, active_area.entity, sector, instance);
            }
        }

        filled.sectors.insert(sector);
    }
}

struct ActiveArea<'a> {
    entity: Entity,
    layer: u32,
    priority: u32,
    spawner: &'a SpawnerComponent,
    descriptors: &'a VegetationDescriptorListComponent,
    processing: SpawnerProcessingSet<'a>,
}

fn spawn_sector_instance(
    commands: &mut Commands,
    area: Entity,
    sector: SectorId,
    instance: InstanceData,
) {
    commands.spawn((
        Name::new(format!("Vegetation Sector {},{}", sector.x, sector.y)),
        VegetationSectorInstance { area, sector },
        instance,
    ));
}

fn aabb_overlaps_xz(lhs: Aabb3d, rhs: Aabb3d) -> bool {
    lhs.min.x <= rhs.max.x
        && lhs.max.x >= rhs.min.x
        && lhs.min.z <= rhs.max.z
        && lhs.max.z >= rhs.min.z
}
