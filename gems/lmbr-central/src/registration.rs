//! What `azoth.lmbr-central` hands a composing host.
//!
//! Three lists, one per registry: the AZ types the gem's `#[az_rtti(...,
//! register)]` derives minted, the component lowerings its `AzComponent`
//! derives minted, and the reflected Prefab types the gem owns. Each derived
//! entry is named here by the `REGISTRATION` const its derive emitted, which is
//! crate-private and `deny(dead_code)` — a type that declares a registration
//! and is left out of a list below does not compile, and the error names the
//! type.
//!
//! The lists are content, not contributions: identity attaches at the marked
//! `impl Contribution` blocks in `lib.rs` that call them.

use az_core::{AzTypeRegistration, ComponentLoweringRegistration};
use az_prefab::PrefabType;

/// The AZ types this gem registers.
#[must_use]
pub const fn types() -> [AzTypeRegistration; 29] {
    [
        crate::SimpleAssetReferenceMannequinAnimationDatabaseAsset::REGISTRATION,
        crate::SimpleAssetReferenceMannequinControllerDefinitionAsset::REGISTRATION,
        crate::AudioProxyComponent::REGISTRATION,
        crate::CharacterPhysicsComponent::REGISTRATION,
        crate::PrimitiveColliderConfig::REGISTRATION,
        crate::PrimitiveColliderComponent::REGISTRATION,
        crate::MeshColliderComponent::REGISTRATION,
        crate::ForceVolumeComponent::REGISTRATION,
        crate::RigidPhysicsConfig::REGISTRATION,
        crate::RigidPhysicsComponent::REGISTRATION,
        crate::PhysicsComponent::REGISTRATION,
        crate::StaticPhysicsConfig::REGISTRATION,
        crate::StaticPhysicsComponent::REGISTRATION,
        crate::PhysicsSystemComponent::REGISTRATION,
        crate::VegetationPhysicsComponent::REGISTRATION,
        crate::RandomTimedSpawnerConfiguration::REGISTRATION,
        crate::RandomTimedSpawnerComponent::REGISTRATION,
        crate::SpawnerComponent::REGISTRATION,
        crate::TriggerAreaComponent::REGISTRATION,
        crate::CompoundShapeConfiguration::REGISTRATION,
        crate::CompoundShapeComponent::REGISTRATION,
        crate::BoxShapeConfig::REGISTRATION,
        crate::BoxShapeComponent::REGISTRATION,
        crate::CapsuleShapeConfig::REGISTRATION,
        crate::CapsuleShapeComponent::REGISTRATION,
        crate::CylinderShapeConfig::REGISTRATION,
        crate::CylinderShapeComponent::REGISTRATION,
        crate::SphereShapeConfig::REGISTRATION,
        crate::SphereShapeComponent::REGISTRATION,
    ]
}

/// The component lowerings this gem registers.
#[must_use]
pub const fn lowerings() -> [ComponentLoweringRegistration; 7] {
    [
        crate::AttachmentComponent::REGISTRATION,
        crate::MannequinComponent::REGISTRATION,
        crate::MannequinScopeComponent::REGISTRATION,
        crate::MotionParameterSmoothingComponent::REGISTRATION,
        crate::CharacterAnimationManagerComponent::REGISTRATION,
        crate::SimpleAnimationComponent::REGISTRATION,
        crate::ParticleComponent::REGISTRATION,
    ]
}

/// The Bevy-native Prefab component types this gem owns.
///
/// Registration-only by construction: an entry carries the reflected type and
/// nothing else, so project-host, asset-worker, and offline import tools take
/// the list without booting the runtime plugin graph or requiring an
/// `AssetServer`.
#[must_use]
pub fn prefab_types() -> [PrefabType; 39] {
    [
        PrefabType::of::<crate::AttachmentComponent>(),
        PrefabType::of::<crate::MotionParameterSmoothingComponent>(),
        PrefabType::of::<crate::CharacterAnimationManagerComponent>(),
        PrefabType::of::<crate::SimpleAnimationComponent>(),
        PrefabType::of::<crate::MannequinComponent>(),
        PrefabType::of::<crate::MannequinScopeComponent>(),
        PrefabType::of::<crate::RigidPhysicsComponent>(),
        PrefabType::of::<crate::VegetationPhysicsComponent>(),
        PrefabType::of::<crate::ForceVolumeComponent>(),
        PrefabType::of::<crate::StaticPhysicsComponent>(),
        PrefabType::of::<crate::PrimitiveColliderComponent>(),
        PrefabType::of::<crate::MeshColliderComponent>(),
        PrefabType::of::<crate::PhysicsSystemComponent>(),
        PrefabType::of::<crate::CharacterPhysicsComponent>(),
        PrefabType::of::<crate::CompoundShapeComponent>(),
        PrefabType::of::<crate::BoxShapeComponent>(),
        PrefabType::of::<crate::CapsuleShapeComponent>(),
        PrefabType::of::<crate::CylinderShapeComponent>(),
        PrefabType::of::<crate::SphereShapeComponent>(),
        PrefabType::of::<crate::TriggerAreaComponent>(),
        PrefabType::of::<crate::ParticleComponent>(),
        // MeshComponent is a genuine native Prefab type (see mesh/static_mesh's
        // #[prefab(...)] attribute) — the earlier register-without-PrefabTypeData
        // "Policy B" placeholder was superseded once corpus-scale import showed
        // ~28% of prefabs carry a mesh.
        PrefabType::of::<crate::MeshComponent>(),
        PrefabType::of::<crate::InstancedMeshComponent>(),
        PrefabType::of::<crate::DecalComponent>(),
        PrefabType::of::<crate::LightComponent>(),
        PrefabType::of::<crate::PolygonPrismShapeComponent>(),
        PrefabType::of::<crate::SplineComponent>(),
        PrefabType::of::<crate::TagComponent>(),
        PrefabType::of::<crate::AudioProxyComponent>(),
        PrefabType::of::<crate::AudioAreaEnvironmentComponent>(),
        PrefabType::of::<crate::AudioEnvironmentComponent>(),
        PrefabType::of::<crate::AudioPreloadComponent>(),
        PrefabType::of::<crate::AudioRtpcComponent>(),
        PrefabType::of::<crate::AudioShapeComponent>(),
        PrefabType::of::<crate::AudioSplineComponent>(),
        PrefabType::of::<crate::AudioSwitchComponent>(),
        PrefabType::of::<crate::AudioTriggerComponent>(),
        PrefabType::of::<crate::SpawnerComponent>(),
        PrefabType::of::<crate::RandomTimedSpawnerComponent>(),
    ]
}
