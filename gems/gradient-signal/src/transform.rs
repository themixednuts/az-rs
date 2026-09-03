//! Gradient transform component data.

use az_prefab::{Prefab, ReflectPrefab};
use bevy::ecs::entity::MapEntities;
use bevy::math::{EulerRot, bounding::Aabb3d};
use bevy::prelude::*;

const UV_EPSILON: f32 = 0.001;

/// Wrapping mode for position-to-UVW conversion.
///
/// O3DE reference: `Gems/GradientSignal/Code/Include/GradientSignal/Util.h:27`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum WrappingType {
    #[default]
    None,
    ClampToEdge,
    Mirror,
    Repeat,
    ClampToZero,
}

impl WrappingType {
    #[must_use]
    pub const fn from_native_value(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::ClampToEdge),
            2 => Some(Self::Mirror),
            3 => Some(Self::Repeat),
            4 => Some(Self::ClampToZero),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> u8 {
        match self {
            Self::None => 0,
            Self::ClampToEdge => 1,
            Self::Mirror => 2,
            Self::Repeat => 3,
            Self::ClampToZero => 4,
        }
    }
}

/// Transform space used by a gradient transform component.
///
/// O3DE reference: `Gems/GradientSignal/Code/Include/GradientSignal/Util.h:36`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum TransformType {
    #[default]
    WorldThisEntity,
    LocalThisEntity,
    WorldReferenceEntity,
    LocalReferenceEntity,
    WorldOrigin,
    Relative,
}

impl TransformType {
    #[must_use]
    pub const fn from_native_value(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::WorldThisEntity),
            1 => Some(Self::LocalThisEntity),
            2 => Some(Self::WorldReferenceEntity),
            3 => Some(Self::LocalReferenceEntity),
            4 => Some(Self::WorldOrigin),
            5 => Some(Self::Relative),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> u8 {
        match self {
            Self::WorldThisEntity => 0,
            Self::LocalThisEntity => 1,
            Self::WorldReferenceEntity => 2,
            Self::LocalReferenceEntity => 3,
            Self::WorldOrigin => 4,
            Self::Relative => 5,
        }
    }
}

/// Gradient transform configuration.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/GradientTransformComponent.h:54`.
// The seven bools are the reflected field-for-field mirror of the Lumberyard
// component config; they are read back out of prefab documents by name, so
// collapsing them into an enum or bitflags would change the serialized shape.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Reflect, MapEntities)]
pub struct GradientTransformConfig {
    pub advanced_mode: bool,
    pub allow_reference: bool,
    #[entities]
    pub shape_reference: Option<Entity>,
    pub override_bounds: bool,
    pub bounds: Vec3,
    pub transform_type: TransformType,
    pub override_translate: bool,
    pub translate: Vec3,
    pub override_rotate: bool,
    pub rotate: Vec3,
    pub override_scale: bool,
    pub scale: Vec3,
    pub frequency_zoom: f32,
    pub adjust_frequency_to_bounds: bool,
    pub wrapping_type: WrappingType,
    pub normalize_output: bool,
    pub is_3d: bool,
}

impl Default for GradientTransformConfig {
    fn default() -> Self {
        Self {
            advanced_mode: false,
            allow_reference: false,
            shape_reference: None,
            override_bounds: false,
            bounds: Vec3::ONE,
            transform_type: TransformType::WorldThisEntity,
            override_translate: false,
            translate: Vec3::ZERO,
            override_rotate: false,
            rotate: Vec3::ZERO,
            override_scale: false,
            scale: Vec3::ONE,
            frequency_zoom: 1.0,
            adjust_frequency_to_bounds: false,
            wrapping_type: WrappingType::None,
            normalize_output: false,
            is_3d: false,
        }
    }
}

impl GradientTransformConfig {
    #[must_use]
    pub fn shape_entity(&self, owner: Entity) -> Entity {
        if self.advanced_mode && self.allow_reference {
            self.shape_reference.unwrap_or(owner)
        } else {
            owner
        }
    }

    #[must_use]
    pub fn local_bounds(&self) -> Aabb3d {
        Self::local_bounds_from_extents(self.bounds)
    }

    #[must_use]
    pub fn local_bounds_from_shape(&self, shape_bounds: Option<Aabb3d>) -> Aabb3d {
        if (!self.advanced_mode || !self.override_bounds)
            && let Some(shape_bounds) = shape_bounds
        {
            return Self::local_bounds_from_extents(Vec3::from(
                shape_bounds.max - shape_bounds.min,
            ));
        }

        self.local_bounds()
    }

    #[must_use]
    pub fn local_bounds_from_extents(extents: Vec3) -> Aabb3d {
        Aabb3d::new(Vec3::ZERO, extents.abs() * 0.5)
    }

    /// Transform a world position into gradient UVW space.
    ///
    /// O3DE reference: `Gems/GradientSignal/Code/Source/Components/GradientTransformComponent.cpp:338`.
    #[must_use]
    pub fn transform_position_to_uvw(
        &self,
        position: Vec3,
        transform: Transform,
        should_normalize_output: bool,
    ) -> GradientTransformResult {
        self.transform_position_to_uvw_in_bounds(
            position,
            transform,
            self.local_bounds(),
            should_normalize_output,
        )
    }

    /// Transform a world position into UVW space using resolved shape bounds.
    ///
    /// O3DE reference: `Gems/GradientSignal/Code/Source/Components/GradientTransformComponent.cpp:338`.
    #[must_use]
    pub fn transform_position_to_uvw_in_bounds(
        &self,
        position: Vec3,
        transform: Transform,
        bounds: Aabb3d,
        should_normalize_output: bool,
    ) -> GradientTransformResult {
        let inverse = transform.to_matrix().inverse();
        let mut uvw = inverse.transform_point3(position);

        if !self.advanced_mode || !self.is_3d {
            uvw.z = 0.0;
        }

        let mut rejected = false;
        uvw = match self.wrapping_type {
            WrappingType::None => uvw,
            WrappingType::ClampToEdge => clamp_point_in_aabb(uvw, bounds),
            WrappingType::ClampToZero => {
                rejected = !aabb_contains_min_inclusive_max_exclusive(uvw, bounds);
                clamp_point_in_aabb(uvw, bounds)
            }
            WrappingType::Mirror => mirror_point_in_aabb(uvw, bounds),
            WrappingType::Repeat => wrap_point_in_aabb(uvw, bounds),
        };

        uvw *= self.frequency_zoom;
        if should_normalize_output || self.normalize_output {
            uvw = normalize_point_in_aabb(uvw, bounds);
        }

        GradientTransformResult { uvw, rejected }
    }

    #[must_use]
    pub fn transform_from_bevy(
        &self,
        selected: Transform,
        owner: Transform,
        reference: Transform,
    ) -> Transform {
        let source = match self.transform_type {
            TransformType::WorldOrigin => Transform::IDENTITY,
            TransformType::LocalThisEntity
            | TransformType::WorldThisEntity
            | TransformType::LocalReferenceEntity
            | TransformType::WorldReferenceEntity => selected,
            TransformType::Relative => {
                let matrix = reference.to_matrix().inverse() * owner.to_matrix();
                Transform::from_matrix(matrix)
            }
        };

        Transform {
            translation: if self.advanced_mode && self.override_translate {
                self.translate
            } else {
                source.translation
            },
            rotation: if self.advanced_mode && self.override_rotate {
                Quat::from_euler(
                    EulerRot::XYZ,
                    self.rotate.x.to_radians(),
                    self.rotate.y.to_radians(),
                    self.rotate.z.to_radians(),
                )
            } else {
                source.rotation
            },
            scale: if self.advanced_mode && self.override_scale {
                self.scale
            } else {
                source.scale
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientTransformResult {
    pub uvw: Vec3,
    pub rejected: bool,
}

/// Runtime gradient transform component.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/GradientTransformComponent.h:76`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, MapEntities, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.gradient_signal.GradientTransformComponent", version = 1)]
pub struct GradientTransformComponent {
    #[entities]
    pub configuration: GradientTransformConfig,
}

impl GradientTransformComponent {
    #[must_use]
    pub fn transform_position_to_uvw(
        &self,
        position: Vec3,
        transform: Transform,
        should_normalize_output: bool,
    ) -> GradientTransformResult {
        self.configuration
            .transform_position_to_uvw(position, transform, should_normalize_output)
    }

    #[must_use]
    pub fn transform_position_to_uvw_in_bounds(
        &self,
        position: Vec3,
        transform: Transform,
        bounds: Aabb3d,
        should_normalize_output: bool,
    ) -> GradientTransformResult {
        self.configuration.transform_position_to_uvw_in_bounds(
            position,
            transform,
            bounds,
            should_normalize_output,
        )
    }
}

fn clamp_point_in_aabb(point: Vec3, bounds: Aabb3d) -> Vec3 {
    let min = Vec3::from(bounds.min);
    let max = Vec3::from(bounds.max) - Vec3::splat(UV_EPSILON);
    point.clamp(min, max)
}

fn mirror_point_in_aabb(point: Vec3, bounds: Aabb3d) -> Vec3 {
    let min = Vec3::from(bounds.min);
    let max = Vec3::from(bounds.max);
    Vec3::new(
        mirror_axis(point.x, min.x, max.x),
        mirror_axis(point.y, min.y, max.y),
        mirror_axis(point.z, min.z, max.z),
    )
}

fn wrap_point_in_aabb(point: Vec3, bounds: Aabb3d) -> Vec3 {
    let min = Vec3::from(bounds.min);
    let max = Vec3::from(bounds.max);
    Vec3::new(
        wrap_axis(point.x, min.x, max.x),
        wrap_axis(point.y, min.y, max.y),
        wrap_axis(point.z, min.z, max.z),
    )
}

fn normalize_point_in_aabb(point: Vec3, bounds: Aabb3d) -> Vec3 {
    let min = Vec3::from(bounds.min);
    let max = Vec3::from(bounds.max);
    Vec3::new(
        inverse_lerp(min.x, max.x, point.x),
        inverse_lerp(min.y, max.y, point.y),
        inverse_lerp(min.z, max.z, point.z),
    )
}

fn aabb_contains_min_inclusive_max_exclusive(point: Vec3, bounds: Aabb3d) -> bool {
    let min = Vec3::from(bounds.min);
    let max = Vec3::from(bounds.max);
    point.cmpge(min).all() && point.cmplt(max).all()
}

fn mirror_axis(value: f32, min: f32, max: f32) -> f32 {
    let range = max - min;
    if range <= 0.0 {
        return min;
    }

    let range_x2 = range * 2.0;
    let mut relative_value = value - min;
    if relative_value < 0.0 {
        relative_value = range_x2 - (-relative_value % range_x2);
    } else {
        relative_value %= range_x2;
    }
    if relative_value >= range {
        relative_value = range_x2 - (relative_value + UV_EPSILON);
    }

    relative_value + min
}

fn wrap_axis(value: f32, min: f32, max: f32) -> f32 {
    let range = max - min;
    if range <= 0.0 {
        return min;
    }

    (value - min).rem_euclid(range) + min
}

fn inverse_lerp(min: f32, max: f32, value: f32) -> f32 {
    let range = max - min;
    if range == 0.0 {
        return if value <= min { 0.0 } else { 1.0 };
    }
    ((value - min) / range).clamp(0.0, 1.0)
}
