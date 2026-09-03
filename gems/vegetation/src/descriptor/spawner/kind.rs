use bevy::prelude::*;

/// Instance spawner kind carried by a descriptor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum InstanceSpawnerKind {
    Empty,
    #[default]
    LegacyVegetation,
    DynamicSlice,
}
