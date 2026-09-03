//! Vegetation instance data.

use bevy::prelude::*;

use crate::surface::VegetationSurfaceTagWeight;
use crate::{f32_close, quat_close, vec3_close};

pub const MAX_INSTANCE_ID: InstanceId = InstanceId(u64::MAX - 1);
pub const INVALID_INSTANCE_ID: InstanceId = InstanceId(u64::MAX);

/// Vegetation instance identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect)]
#[repr(transparent)]
pub struct InstanceId(pub u64);

impl Default for InstanceId {
    fn default() -> Self {
        INVALID_INSTANCE_ID
    }
}

impl InstanceId {
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 != INVALID_INSTANCE_ID.0
    }
}

/// Data used to create or compare one vegetation instance.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/InstanceData.h:30`.
#[derive(Component, Debug, Clone, PartialEq, Reflect)]
#[reflect(Component)]
pub struct InstanceData {
    pub entity: Option<Entity>,
    pub instance_id: InstanceId,
    pub change_index: u32,
    pub position: Vec3,
    pub normal: Vec3,
    pub rotation: Quat,
    pub alignment: Quat,
    pub scale: f32,
    pub masks: Vec<VegetationSurfaceTagWeight>,
    pub descriptor_index: Option<usize>,
}

impl Default for InstanceData {
    fn default() -> Self {
        Self {
            entity: None,
            instance_id: INVALID_INSTANCE_ID,
            change_index: 0,
            position: Vec3::ZERO,
            normal: Vec3::Y,
            rotation: Quat::IDENTITY,
            alignment: Quat::IDENTITY,
            scale: 1.0,
            masks: Vec::new(),
            descriptor_index: None,
        }
    }
}

impl InstanceData {
    /// Build the Bevy transform used by vegetation spawners.
    ///
    /// Lumberyard reference: `dev/Gems/Vegetation/Code/Source/LegacyVegetationInstanceSpawner.cpp:492`.
    #[must_use]
    pub fn transform(&self) -> Transform {
        Transform {
            translation: self.position,
            rotation: self.alignment * self.rotation,
            scale: Vec3::splat(self.scale),
        }
    }

    #[must_use]
    pub fn is_same_instance_data(&self, rhs: &Self) -> bool {
        self.entity == rhs.entity
            && vec3_close(self.position, rhs.position)
            && quat_close(self.rotation, rhs.rotation)
            && quat_close(self.alignment, rhs.alignment)
            && f32_close(self.scale, rhs.scale)
            && self.descriptor_index == rhs.descriptor_index
    }
}

pub fn register_instance_components(app: &mut App) {
    app.register_type::<InstanceId>()
        .register_type::<InstanceData>();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instance_data_comparison_uses_stable_fields() {
        let a = InstanceData::default();
        let mut b = a.clone();
        b.instance_id = InstanceId(42);
        b.change_index = 7;

        assert!(a.is_same_instance_data(&b));
    }
}
