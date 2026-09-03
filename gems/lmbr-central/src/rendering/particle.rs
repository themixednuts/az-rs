//! Particle component data and Bevy preview sync.

mod component;
mod ids;
mod request;
mod settings;
mod systems;

use bevy::prelude::*;

use systems::sync_particle_components;

pub use component::{ParticleComponent, ParticleEmitBoneLayer};
pub use ids::*;
pub use request::{
    ParticleComponentAction, ParticleComponentRequest, ParticleEmitterTrigger,
    ParticleEmitterTriggerKind,
};
pub use settings::ParticleEmitterSettings;

pub(super) fn register_particle_components(app: &mut App) {
    app.register_type::<ParticleEmitterSettings>()
        .register_type_data::<ParticleEmitterSettings, az_core::ReflectAzTypeInfo>()
        .register_type::<ParticleEmitBoneLayer>()
        .register_type_data::<ParticleEmitBoneLayer, az_core::ReflectAzTypeInfo>()
        .register_type::<Vec<ParticleEmitBoneLayer>>()
        .register_type::<ParticleComponent>()
        .register_type_data::<ParticleComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<ParticleComponent, az_core::ReflectAzRtti>()
        .add_message::<ParticleComponentRequest>()
        .add_message::<ParticleEmitterTrigger>()
        .add_systems(
            Update,
            request::route_particle_component_requests.before(sync_particle_components),
        )
        .add_systems(Update, sync_particle_components);
}
