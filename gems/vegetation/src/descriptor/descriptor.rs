//! Vegetation descriptor data.

use bevy::prelude::*;

use crate::surface::{
    VegetationSurfaceTag, VegetationSurfaceTagDepth, VegetationSurfaceTagDistance,
    VegetationSurfaceTagOffset,
};

use super::mode::{BoundMode, OverrideMode};
use super::spawner::{InstanceSpawner, InstanceSpawnerKind};

/// Details used to create vegetation instances.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/Descriptor.h:63`.
// The `*_override_enabled` bools are the reflected field-for-field mirror of
// the Lumberyard descriptor; they are read back out of descriptor assets and
// prefab documents by name, so folding them into a flag set would change the
// serialized shape.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct VegetationDescriptor {
    pub instance_spawner: InstanceSpawner,
    pub weight: f32,
    pub advanced: bool,
    pub surface_tag_distance: VegetationSurfaceTagDistance,
    pub surface_offset_tags: Vec<VegetationSurfaceTagOffset>,
    pub surface_depth_tags: Vec<VegetationSurfaceTagDepth>,
    pub surface_filter_override_mode: OverrideMode,
    pub inclusive_surface_filter_tags: Vec<VegetationSurfaceTag>,
    pub exclusive_surface_filter_tags: Vec<VegetationSurfaceTag>,
    pub radius_override_enabled: bool,
    pub bound_mode: BoundMode,
    pub radius_min: f32,
    pub surface_alignment_override_enabled: bool,
    pub surface_alignment_min: f32,
    pub surface_alignment_max: f32,
    pub position_override_enabled: bool,
    pub position_min: Vec3,
    pub position_max: Vec3,
    pub rotation_override_enabled: bool,
    pub rotation_min_degrees: Vec3,
    pub rotation_max_degrees: Vec3,
    pub scale_override_enabled: bool,
    pub scale_min: f32,
    pub scale_max: f32,
    pub altitude_filter_override_enabled: bool,
    pub altitude_filter_min: f32,
    pub altitude_filter_max: f32,
    pub slope_filter_override_enabled: bool,
    pub slope_filter_min: f32,
    pub slope_filter_max: f32,
    pub user_data: VegetationDescriptorUserData,
}

impl Default for VegetationDescriptor {
    fn default() -> Self {
        Self {
            instance_spawner: InstanceSpawner::default(),
            weight: 1.0,
            advanced: false,
            surface_tag_distance: VegetationSurfaceTagDistance::default(),
            surface_offset_tags: Vec::new(),
            surface_depth_tags: Vec::new(),
            surface_filter_override_mode: OverrideMode::Disable,
            inclusive_surface_filter_tags: Vec::new(),
            exclusive_surface_filter_tags: Vec::new(),
            radius_override_enabled: false,
            bound_mode: BoundMode::Radius,
            radius_min: 0.0,
            surface_alignment_override_enabled: false,
            surface_alignment_min: 0.0,
            surface_alignment_max: 1.0,
            position_override_enabled: false,
            position_min: Vec3::new(-0.3, 0.0, -0.3),
            position_max: Vec3::new(0.3, 0.0, 0.3),
            rotation_override_enabled: false,
            rotation_min_degrees: Vec3::new(0.0, -180.0, 0.0),
            rotation_max_degrees: Vec3::new(0.0, 180.0, 0.0),
            scale_override_enabled: false,
            scale_min: 0.1,
            scale_max: 1.0,
            altitude_filter_override_enabled: false,
            altitude_filter_min: 0.0,
            altitude_filter_max: 128.0,
            slope_filter_override_enabled: false,
            slope_filter_min: 0.0,
            slope_filter_max: 20.0,
            user_data: VegetationDescriptorUserData,
        }
    }
}

/// Empty AZ `any` payload stored on reflected vegetation descriptors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub struct VegetationDescriptorUserData;

impl VegetationDescriptor {
    #[must_use]
    pub const fn spawner_kind(&self) -> InstanceSpawnerKind {
        self.instance_spawner.kind()
    }

    #[must_use]
    pub fn radius(&self) -> f32 {
        if self.bound_mode == BoundMode::MeshRadius {
            self.instance_spawner.radius()
        } else {
            self.radius_min
        }
    }

    #[must_use]
    pub fn has_empty_asset_references(&self) -> bool {
        self.instance_spawner.has_empty_asset_references()
    }

    #[must_use]
    pub fn scene_asset_path(&self) -> Option<&str> {
        self.instance_spawner.scene_asset_path()
    }

    #[must_use]
    pub fn scene_asset_variant(&self) -> Option<&str> {
        self.instance_spawner.scene_asset_variant()
    }
}
