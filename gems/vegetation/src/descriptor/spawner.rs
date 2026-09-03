//! Vegetation instance spawner data.

mod dynamic_slice;
mod empty;
mod instance;
mod kind;
mod legacy;

pub use dynamic_slice::DynamicSliceInstanceSpawner;
pub use empty::EmptyInstanceSpawner;
pub use instance::InstanceSpawner;
pub use kind::InstanceSpawnerKind;
pub use legacy::LegacyVegetationInstanceSpawner;
