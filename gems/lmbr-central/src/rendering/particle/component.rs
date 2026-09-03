use az_asset::AssetId;
use az_core::component::Component as AzComponentBase;
use az_derive::{AzComponent, AzTypeInfo};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::borrow::{Borrow, BorrowMut};

use super::settings::ParticleEmitterSettings;
use super::{PARTICLE_COMPONENT_TYPE_UUID, PARTICLE_EMIT_BONE_LAYER_TYPE_UUID};

/// Mesh-particle bone layer binding used by skinned particle emitters.
#[derive(AzTypeInfo, Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[az_type_info(name = "ParticleEmitBoneLayer", PARTICLE_EMIT_BONE_LAYER_TYPE_UUID)]
pub struct ParticleEmitBoneLayer {
    #[serde(rename = "Joint name", default)]
    pub joint_name: String,
    #[serde(rename = "Enable Layer", default)]
    pub enable_layer: bool,
    #[serde(rename = "AffectedIndices", default)]
    pub affected_indices: Vec<u32>,
}

impl ParticleEmitBoneLayer {
    pub const SERIALIZE_VERSION: u32 = 1;
}

/// Runtime particle component.
#[derive(
    Component,
    AzComponent,
    Debug,
    Clone,
    Default,
    PartialEq,
    Reflect,
    Serialize,
    Deserialize,
    Prefab,
)]
#[az_component(name = "ParticleComponent", PARTICLE_COMPONENT_TYPE_UUID)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.ParticleComponent", version = 1)]
pub struct ParticleComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponentBase,
    #[serde(rename = "Particle", default)]
    pub settings: ParticleEmitterSettings,
    #[serde(rename = "ParticleLibraryAssetId", default)]
    #[reflect(ignore)]
    pub particle_library_asset_id: AssetId,
    #[serde(rename = "MeshParticle", default)]
    pub mesh_particle: Vec<ParticleEmitBoneLayer>,
    #[serde(rename = "Load Emitter On Activate", default)]
    pub load_emitter_on_activate: bool,
}

impl ParticleComponent {
    pub const SERIALIZE_VERSION: u32 = 3;
}

impl AsRef<ParticleEmitterSettings> for ParticleComponent {
    fn as_ref(&self) -> &ParticleEmitterSettings {
        &self.settings
    }
}

impl AsMut<ParticleEmitterSettings> for ParticleComponent {
    fn as_mut(&mut self) -> &mut ParticleEmitterSettings {
        &mut self.settings
    }
}

impl Borrow<ParticleEmitterSettings> for ParticleComponent {
    fn borrow(&self) -> &ParticleEmitterSettings {
        &self.settings
    }
}

impl BorrowMut<ParticleEmitterSettings> for ParticleComponent {
    fn borrow_mut(&mut self) -> &mut ParticleEmitterSettings {
        &mut self.settings
    }
}
