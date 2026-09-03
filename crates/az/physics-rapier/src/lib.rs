//! Rapier implementation of [`az_physics::PhysicsBackend`].

mod backend;
mod buoyancy;
pub(crate) mod convert;
mod deformable;
mod mesh_slice;
mod plugin;

pub use az_physics::{PhysicsStepConfiguration, PhysicsStepReport};
pub use backend::RapierPhysicsBackend;
pub use plugin::{PhysicsSchedule, RapierPhysicsPlugin};
