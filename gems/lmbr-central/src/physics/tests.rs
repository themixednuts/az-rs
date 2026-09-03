use super::*;
use az_core::{AzRtti, AzTypeInfo};
use az_physics::LivingBodyConfiguration;

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn physics_component_defaults_match_lumberyard_runtime_defaults() {
    let primitive = PrimitiveColliderComponent::default();
    let rigid = RigidPhysicsComponent::default();
    let static_physics = StaticPhysicsComponent::default();

    assert_eq!(primitive.configuration.surface_type_name(), None);

    assert!(rigid.configuration.enabled_initially);
    assert_eq!(
        rigid.configuration.specify_mass_or_density,
        MassOrDensity::Density
    );
    assert!(!rigid.configuration.use_mass());
    assert!(rigid.configuration.use_density());
    assert_eq!(rigid.configuration.mass, 10.0);
    assert_eq!(rigid.configuration.density, 500.0);
    assert!(!rigid.configuration.at_rest_initially);
    assert!(rigid.configuration.enable_collision_response);
    assert!(rigid.configuration.interacts_with_triggers);
    assert_eq!(rigid.configuration.buoyancy_damping, 0.0);
    assert_eq!(rigid.configuration.buoyancy_density, 1.0);
    assert_eq!(rigid.configuration.buoyancy_resistance, 1.0);
    assert_eq!(rigid.configuration.simulation_damping, 0.0);
    assert_eq!(rigid.configuration.simulation_min_energy, 0.002);
    assert!(rigid.configuration.record_collisions);
    assert_eq!(rigid.configuration.recorded_collision_capacity(), 1);

    assert!(static_physics.configuration.enabled_initially);
    assert!(!static_physics.configuration.interacts_with_triggers);
    // The native constructor names this filter. This assertion predates that
    // evidence, when the field was an `Option` that defaulted to `None`.
    assert_eq!(static_physics.collision_filter(), Some("Structure"));
}

#[test]
fn physics_helpers_preserve_optional_strings() {
    let primitive = PrimitiveColliderConfig {
        surface_type_name: "mat_stone".to_string(),
    };
    let static_physics = StaticPhysicsComponent {
        collision_filter: "WorldStatic".to_string(),
        ..Default::default()
    };

    assert_eq!(primitive.surface_type_name(), Some("mat_stone"));
    assert_eq!(static_physics.collision_filter(), Some("WorldStatic"));

    let disabled_recording = RigidPhysicsConfig {
        record_collisions: false,
        max_recorded_collisions: 10,
        ..Default::default()
    };
    assert_eq!(disabled_recording.recorded_collision_capacity(), 0);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn lmbr_physics_components_keep_exact_native_type_relations() {
    assert_eq!(PhysicsComponent::TYPE_ID, PHYSICS_COMPONENT_TYPE_UUID);
    assert!(PhysicsComponent::BASE_TYPE_IDS.contains(&az_core::component::COMPONENT_TYPE_ID));
    assert_eq!(RigidPhysicsConfig::TYPE_ID, RIGID_PHYSICS_CONFIG_TYPE_UUID);
    assert_eq!(
        StaticPhysicsConfig::TYPE_ID,
        STATIC_PHYSICS_CONFIG_TYPE_UUID
    );
    assert_eq!(
        RigidPhysicsComponent::TYPE_ID,
        RIGID_PHYSICS_COMPONENT_TYPE_UUID
    );
    assert!(RigidPhysicsComponent::BASE_TYPE_IDS.contains(&PHYSICS_COMPONENT_TYPE_UUID));
    assert_eq!(
        StaticPhysicsComponent::TYPE_ID,
        STATIC_PHYSICS_COMPONENT_TYPE_UUID
    );
    assert!(StaticPhysicsComponent::BASE_TYPE_IDS.contains(&PHYSICS_COMPONENT_TYPE_UUID));
    assert_eq!(
        PrimitiveColliderConfig::TYPE_ID,
        PRIMITIVE_COLLIDER_CONFIG_TYPE_UUID
    );
    assert_eq!(
        PrimitiveColliderComponent::TYPE_ID,
        PRIMITIVE_COLLIDER_COMPONENT_TYPE_UUID
    );
    assert_eq!(
        MeshColliderComponent::TYPE_ID,
        MESH_COLLIDER_COMPONENT_TYPE_UUID
    );
    assert_eq!(
        ForceVolumeConfiguration::TYPE_ID,
        FORCE_VOLUME_CONFIGURATION_TYPE_UUID
    );
    assert_eq!(
        ForceVolumeComponent::TYPE_ID,
        FORCE_VOLUME_COMPONENT_TYPE_UUID
    );
    assert_eq!(
        VegetationPhysicsComponent::TYPE_ID,
        VEGETATION_PHYSICS_COMPONENT_TYPE_UUID
    );
    assert_eq!(VEGETATION_PHYSICS_SERVICE.value(), 0x6225_2473);

    let force = ForceVolumeConfiguration::default();
    assert_eq!(force.force_mode, super::ForceMode::Direction);
    assert_eq!(force.force_space, super::ForceSpace::World);
    assert!(force.force_mass_dependent);
    assert_eq!(force.force_direction, Vec3::ONE);
    assert_eq!(force.force_scale, 20.0);
    assert_eq!(force.volume_damping, 0.0);
    assert_eq!(force.volume_density, 0.0);
}

#[test]
fn cry_character_types_keep_native_az_identity_and_defaults() {
    assert_eq!(PlayerDimensions::TYPE_ID, PLAYER_DIMENSIONS_TYPE_ID);
    assert_eq!(PlayerDynamics::TYPE_ID, PLAYER_DYNAMICS_TYPE_ID);
    assert_eq!(
        CryPlayerPhysicsConfiguration::TYPE_ID,
        CRY_PLAYER_PHYSICS_CONFIGURATION_TYPE_ID
    );
    assert_eq!(
        CharacterPhysicsComponent::TYPE_ID,
        CHARACTER_PHYSICS_COMPONENT_TYPE_ID
    );
    assert!(
        CharacterPhysicsComponent::BASE_TYPE_IDS.contains(&az_core::component::COMPONENT_TYPE_ID)
    );

    let component = CharacterPhysicsComponent::default();
    let runtime = component.living_body_configuration();
    assert_eq!(runtime, LivingBodyConfiguration::default());
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn physics_system_and_raycast_types_keep_native_az_identity() {
    assert_eq!(
        PhysicsSystemComponent::TYPE_ID,
        PHYSICS_SYSTEM_COMPONENT_TYPE_UUID
    );
    assert!(PhysicsSystemComponent::BASE_TYPE_IDS.contains(&az_core::component::COMPONENT_TYPE_ID));
    assert_eq!(
        RayCastConfiguration::TYPE_ID,
        RAY_CAST_CONFIGURATION_TYPE_ID
    );
    assert_eq!(RayCastHit::TYPE_ID, RAY_CAST_HIT_TYPE_ID);
    assert_eq!(RayCastResult::TYPE_ID, RAY_CAST_RESULT_TYPE_ID);

    let query = RayCastConfiguration::default();
    assert_eq!(query.direction, Vec3::Y);
    assert_eq!(query.max_distance, 100.0);
    assert_eq!(query.max_hits, 1);
    assert_eq!(query.pierces_surfaces_greater_than, 15);
    assert_eq!(query.physical_entity_types, 31);

    let mut result = RayCastResult::default();
    assert_eq!(result.hit_count(), 0);
    assert!(!result.has_blocking_hit());
    result.add_piercing_hit(RayCastHit {
        distance: 1.0,
        ..RayCastHit::default()
    });
    result.set_blocking_hit(RayCastHit {
        distance: 2.0,
        ..RayCastHit::default()
    });
    assert_eq!(result.hit_count(), 2);
    assert_eq!(result.hit(0).map(|hit| hit.distance), Some(1.0));
    assert_eq!(result.hit(1).map(|hit| hit.distance), Some(2.0));
}
