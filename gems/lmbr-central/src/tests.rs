use super::*;

#[test]
fn prefab_registration_is_pure_and_complete() {
    use std::any::TypeId;

    let mut registry = bevy::reflect::TypeRegistry::default();
    for entry in prefab_types() {
        entry.apply(&mut registry);
    }

    for type_id in [
        TypeId::of::<AttachmentComponent>(),
        TypeId::of::<MotionParameterSmoothingComponent>(),
        TypeId::of::<CharacterAnimationManagerComponent>(),
        TypeId::of::<SimpleAnimationComponent>(),
        TypeId::of::<MannequinComponent>(),
        TypeId::of::<MannequinScopeComponent>(),
        TypeId::of::<RigidPhysicsComponent>(),
        TypeId::of::<ForceVolumeComponent>(),
        TypeId::of::<StaticPhysicsComponent>(),
        TypeId::of::<PrimitiveColliderComponent>(),
        TypeId::of::<MeshColliderComponent>(),
        TypeId::of::<PhysicsSystemComponent>(),
        TypeId::of::<CharacterPhysicsComponent>(),
        TypeId::of::<CompoundShapeComponent>(),
        TypeId::of::<BoxShapeComponent>(),
        TypeId::of::<CapsuleShapeComponent>(),
        TypeId::of::<CylinderShapeComponent>(),
        TypeId::of::<SphereShapeComponent>(),
        TypeId::of::<TriggerAreaComponent>(),
        TypeId::of::<ParticleComponent>(),
    ] {
        assert!(
            registry
                .get(type_id)
                .and_then(|registration| registration.data::<az_prefab::PrefabTypeData>())
                .is_some(),
            "registered LmbrCentral Prefab type must carry PrefabTypeData"
        );
    }
}

#[test]
fn lmbr_central_type_ids_match_public_registrations() {
    rendering_component_type_ids_match_validated_registrations();
    mesh_component_type_ids_match_validated_registrations();
    audio_component_type_ids_match_validated_registrations();
    shape_component_type_ids_match_validated_registrations();
    spline_and_vertex_type_ids_match_validated_registrations();
    physics_component_type_ids_match_validated_registrations();
}

/// Rendering: fog volumes, decals, lens flares, particles, shadows and lights.
fn rendering_component_type_ids_match_validated_registrations() {
    assert_eq!(
        FOG_VOLUME_COMPONENT_TYPE_ID,
        "C01B9E8F-C015-46AC-9065-79445CE1408A"
    );
    assert_eq!(
        FOG_VOLUME_CONFIGURATION_TYPE_ID,
        "3B786BBB-0B1D-4EF2-9181-CC75C783C26E"
    );
    assert_eq!(
        DECAL_COMPONENT_TYPE_ID,
        "1C2CEAA8-786F-4684-8202-CA7D940D627B"
    );
    assert_eq!(
        DECAL_CONFIGURATION_TYPE_ID,
        "47082F75-428F-4353-AC82-FAE8AB017F3B"
    );
    assert_eq!(
        LENS_FLARE_COMPONENT_TYPE_ID,
        "07593109-4A57-473F-B868-C2DCF9270186"
    );
    assert_eq!(
        LENS_FLARE_CONFIGURATION_TYPE_ID,
        "1E28DADD-0BD4-4AD5-A94B-2665813BF346"
    );
    assert_eq!(
        PARTICLE_COMPONENT_TYPE_ID,
        "65BC817A-ABF6-440F-AD4F-581C40F92795"
    );
    assert_eq!(
        PARTICLE_EMITTER_SETTINGS_TYPE_ID,
        "A1E34557-30DB-4716-B4CE-39D52A113D0C"
    );
    assert_eq!(
        HIGH_QUALITY_SHADOW_COMPONENT_TYPE_ID,
        "B692F9D9-4850-4D6E-9A32-760901455E40"
    );
    assert_eq!(
        HIGH_QUALITY_SHADOW_CONFIG_TYPE_ID,
        "3B3CD21A-E61B-401A-8F54-B76FB6278B11"
    );
    assert_eq!(
        LIGHT_COMPONENT_TYPE_ID,
        "6B9AB512-CA8A-4D2B-B570-DF128EA7CE6A"
    );
    assert_eq!(
        LIGHT_CONFIGURATION_TYPE_ID,
        "F4CC7BB4-C541-480C-88FC-C5A8F37CC67F"
    );
}

/// Meshes: the public Lumberyard mesh components and their render nodes.
fn mesh_component_type_ids_match_validated_registrations() {
    assert_eq!(
        MESH_COMPONENT_TYPE_ID,
        uuid::uuid!("2F4BAD46-C857-4DCB-A454-C412DE67852A")
    );
    assert_eq!(
        MESH_COMPONENT_RENDER_NODE_TYPE_ID,
        uuid::uuid!("46FF2BC4-BEF9-4CC4-9456-36C127C310D7")
    );
    assert_eq!(
        SKINNED_MESH_COMPONENT_TYPE_ID,
        uuid::uuid!("C99EB110-CA74-4D95-83F0-2FCDD1FF418B")
    );
}

/// Audio: every `LmbrCentral` audio component, live and deprecated.
fn audio_component_type_ids_match_validated_registrations() {
    assert_eq!(
        AUDIO_AREA_ENVIRONMENT_COMPONENT_TYPE_ID,
        uuid::uuid!("52300012-FFCD-4559-9479-20F463940320")
    );
    assert_eq!(
        AUDIO_ENVIRONMENT_COMPONENT_TYPE_ID,
        uuid::uuid!("D5085D04-2522-4585-9E65-D337C5BBB8A7")
    );
    assert_eq!(
        AUDIO_PRELOAD_COMPONENT_TYPE_ID,
        uuid::uuid!("CBBB1234-4DCA-427E-80FF-E2BB0866EEB1")
    );
    assert_eq!(
        AUDIO_LISTENER_COMPONENT_TYPE_ID,
        uuid::uuid!("00B5358C-3EEE-4012-93FC-6222B0004404")
    );
    assert_eq!(
        AUDIO_PROXY_COMPONENT_TYPE_ID,
        uuid::uuid!("0EE6EE0F-7939-4AB8-B0E3-F9B3925D61EE")
    );
    assert_eq!(
        AUDIO_TRIGGER_COMPONENT_TYPE_ID,
        uuid::uuid!("8CBBB54B-7435-4D33-844D-E7F201BD581A")
    );
    assert_eq!(
        DEPRECATED_AUDIO_TRIGGER_COMPONENT_TYPE_ID,
        uuid::uuid!("80089838-4444-4D67-9A89-66B0276BB916")
    );
    assert_eq!(
        DEPRECATED_AUDIO_COMPONENT_TYPE_ID,
        uuid::uuid!("53033C2C-EE40-4D19-A7F4-861D6AA820EB")
    );
    assert_eq!(
        AUDIO_RTPC_COMPONENT_TYPE_ID,
        uuid::uuid!("C54C7AE6-08AA-49E0-B6CD-E1BBB4950DAF")
    );
    assert_eq!(
        DEPRECATED_AUDIO_RTPC_COMPONENT_TYPE_ID,
        uuid::uuid!("4441F9C5-D4AD-40CF-B1DE-D5A296C03798")
    );
    assert_eq!(
        AUDIO_SWITCH_COMPONENT_TYPE_ID,
        uuid::uuid!("85FD9037-A5EA-4783-B49A-7959BBB34011")
    );
    assert_eq!(
        DEPRECATED_AUDIO_SWITCH_COMPONENT_TYPE_ID,
        uuid::uuid!("7A23B947-8EE8-4E6B-A772-C43BBBB4D090")
    );
}

/// Shapes: primitive shape components and their configurations.
fn shape_component_type_ids_match_validated_registrations() {
    assert_eq!(
        BOX_SHAPE_COMPONENT_TYPE_ID,
        uuid::uuid!("5EDF4B9E-0D3D-40B8-8C91-5142BCFC30A6")
    );
    assert_eq!(
        BOX_SHAPE_CONFIG_TYPE_ID,
        uuid::uuid!("F034FBA2-AC2F-4E66-8152-14DFB90D6283")
    );
    assert_eq!(
        SPHERE_SHAPE_COMPONENT_TYPE_ID,
        uuid::uuid!("E24CBFF0-2531-4F8D-A8AB-47AF4D54BCD2")
    );
    assert_eq!(
        SPHERE_SHAPE_CONFIG_TYPE_ID,
        uuid::uuid!("4AADFD75-48A7-4F31-8F30-FE4505F09E35")
    );
    assert_eq!(
        CAPSULE_SHAPE_COMPONENT_TYPE_ID,
        uuid::uuid!("967EC13D-364D-4696-AB5C-C00CC05A2305")
    );
    assert_eq!(
        CAPSULE_SHAPE_CONFIG_TYPE_ID,
        uuid::uuid!("00931AEB-2AD8-42CE-B1DC-FA4332F51501")
    );
    assert_eq!(
        CYLINDER_SHAPE_COMPONENT_TYPE_ID,
        uuid::uuid!("B0C6AA97-E754-4E33-8D32-33E267DB622F")
    );
    assert_eq!(
        CYLINDER_SHAPE_CONFIG_TYPE_ID,
        uuid::uuid!("53254779-82F1-441E-9116-81E1FACFECF4")
    );
    assert_eq!(
        COMPOUND_SHAPE_COMPONENT_TYPE_ID,
        uuid::uuid!("C0C817DE-843F-44C8-9FC1-989CDE66B662")
    );
    assert_eq!(
        COMPOUND_SHAPE_CONFIGURATION_TYPE_ID,
        uuid::uuid!("4CEB4E5C-4CBD-4A84-88BA-87B23C103F3F")
    );
}

/// Splines, vertex containers and polygon prisms.
fn spline_and_vertex_type_ids_match_validated_registrations() {
    assert_eq!(
        SPLINE_COMMON_TYPE_ID,
        "91A31D7E-F63A-4AA8-BC50-909B37F0AD8B"
    );
    assert_eq!(
        SPLINE_COMPONENT_TYPE_ID,
        "F0905297-1E24-4044-BFDA-BDE3583F1E57"
    );
    assert_eq!(SPLINE_TYPE_ID, "6E2D31AF-5CB0-4A50-BD68-B00E2D2FD0A4");
    assert_eq!(
        LINEAR_SPLINE_TYPE_ID,
        "DD80E118-12C9-4F69-848B-4EA5DAA2E0EC"
    );
    assert_eq!(
        BEZIER_SPLINE_TYPE_ID,
        "C1A48956-5CBC-4124-AB49-61FFEEE9139A"
    );
    assert_eq!(BEZIER_DATA_TYPE_ID, "6C34069E-AEA2-44A2-877F-BED9CE07DA6B");
    assert_eq!(
        CATMULL_ROM_SPLINE_TYPE_ID,
        "B4AD0E71-92D8-4888-AB89-5C3B4A30759A"
    );
    assert_eq!(
        VERTEX_CONTAINER_VEC2_TYPE_ID,
        "EBE98B36-0783-5226-9739-064BD41EBB52"
    );
    assert_eq!(
        VERTEX_CONTAINER_VEC3_TYPE_ID,
        "A6F50685-C884-50C6-AD08-123028C77954"
    );
    assert_eq!(
        POLYGON_PRISM_TYPE_ID,
        uuid::uuid!("F01C8BDD-6F24-4344-8945-521A8750B30B")
    );
    assert_eq!(
        POLYGON_PRISM_COMMON_TYPE_ID,
        uuid::uuid!("BDB453DE-8A51-42D0-9237-13A9193BE724")
    );
    assert_eq!(
        POLYGON_PRISM_SHAPE_COMPONENT_TYPE_ID,
        uuid::uuid!("AD882674-1D5D-4E40-B079-449B47D2492C")
    );
}

/// Physics: the system component, colliders and body configurations.
fn physics_component_type_ids_match_validated_registrations() {
    assert_eq!(
        PHYSICS_SYSTEM_COMPONENT_TYPE_ID,
        "1586DBA1-F5F0-49AB-9F59-AE62C0E60AE0"
    );
    assert_eq!(
        PHYSICS_COMPONENT_TYPE_ID,
        "6C2A2397-C33D-4ACA-8813-42B99E7B84DB"
    );
    assert_eq!(
        PRIMITIVE_COLLIDER_CONFIG_TYPE_ID,
        "85AA27D6-E019-469F-8472-89862323DBF7"
    );
    assert_eq!(
        PRIMITIVE_COLLIDER_COMPONENT_TYPE_ID,
        "9CB3707A-73B3-4EE5-84EA-3CF86E0E3722"
    );
    assert_eq!(
        MESH_COLLIDER_COMPONENT_TYPE_ID,
        "2D559EB0-F6FE-46E0-9FCE-E8F375177724"
    );
    assert_eq!(
        RIGID_PHYSICS_CONFIG_TYPE_ID,
        "4D4211C2-4539-444F-A8AC-B0C8417AA579"
    );
    assert_eq!(
        RIGID_PHYSICS_MASS_OR_DENSITY_TYPE_ID,
        "0F5DBFB3-FD9A-4E83-B9B3-4713AB2241B4"
    );
    assert_eq!(
        RIGID_PHYSICS_COMPONENT_TYPE_ID,
        "BF2ED241-6364-4D78-8008-498EF2A2659C"
    );
    assert_eq!(
        STATIC_PHYSICS_CONFIG_TYPE_ID,
        "2129576B-A548-4F3E-A2A1-87851BF48838"
    );
    assert_eq!(
        STATIC_PHYSICS_COMPONENT_TYPE_ID,
        "95D89791-6397-41BC-AAC5-95282C8AD9D4"
    );
}
