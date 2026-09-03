use bevy::prelude::*;
use uuid::{Uuid, uuid};

/// `VegetationPhysicsComponent` AZ type UUID.
pub const VEGETATION_PHYSICS_COMPONENT_TYPE_ID: Uuid =
    uuid!("D221EB6B-85D9-4CB2-96EC-F6BEA2FD017A");

/// Marks a vegetation entity as participating in vegetation physics.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct VegetationPhysicsComponent;

impl VegetationPhysicsComponent {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

pub fn register_physics_components(app: &mut App) {
    app.register_type::<VegetationPhysicsComponent>();
}
