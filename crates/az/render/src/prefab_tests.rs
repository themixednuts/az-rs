use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
};

use az_core::{
    ApplicabilityContext, ApplicabilityTypeData, EditorFieldAttributes, EditorTypeAttributes,
    EditorWidget, ReflectedPath, ReflectedPathSegment, ValidationTypeData,
};
use az_prefab::{
    EntityAlias, PrefabCodec, PrefabDocument, PrefabEntity, PrefabTypeData, SparseValue,
};
use bevy_ecs::{
    entity::Entity, reflect::AppTypeRegistry, template::SceneEntityReferences, world::World,
};
use bevy_reflect::{
    TypeInfo, TypeRegistry, Typed, std_traits::ReflectDefault, structs::DynamicStruct,
};

use crate::{
    Camera, CameraProjection, DirectionalLight, MaterialAssignment, MaterialSlotOverride, Mesh,
    OrthographicCameraProjection, PerspectiveCameraProjection, PointLight, SpotLight,
    register_render_prefab_types,
};

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    az_transform::register_transform_prefab_types(&mut registry);
    register_render_prefab_types(&mut registry);
    registry
}

fn document_with_component(tag: &str, version: u32, value: SparseValue) -> PrefabDocument {
    let mut document = PrefabDocument::default();
    document.type_versions.insert(tag.to_owned(), version);
    document.entities.insert(
        EntityAlias::new("entity").unwrap(),
        PrefabEntity {
            entity_id: None,
            parent: None,
            components: BTreeMap::from([(tag.to_owned(), value)]),
        },
    );
    document
}

fn construct_and_insert(
    registry: &TypeRegistry,
    type_id: TypeId,
    value: &SparseValue,
) -> (World, Entity) {
    let registration = registry.get(type_id).expect("render Prefab registration");
    let prefab = registration
        .data::<PrefabTypeData>()
        .expect("render Prefab TypeData");
    let mut world = World::new();
    world.insert_resource(AppTypeRegistry::default());
    let entity = world.spawn_empty().id();
    let mut references = SceneEntityReferences::default();
    let built = {
        let mut entity_mut = world.entity_mut(entity);
        (prefab.construct)(
            registration,
            value.value(),
            &mut entity_mut,
            &mut references,
        )
        .expect("construct render Prefab")
    };
    {
        let mut entity_mut = world.entity_mut(entity);
        (prefab.insert)(
            registration,
            built.value.as_ref(),
            &mut entity_mut,
            registry,
        )
        .expect("insert render Prefab");
    }
    (world, entity)
}

#[test]
fn render_prefab_registration_is_role_stable_and_projects_editor_attributes() {
    let client = registry();
    let server = registry();
    assert_prefab_roles_are_stable(&client, &server);
    assert_mesh_editor_attributes();
    assert_light_editor_attributes(&client);
}

/// Every render Prefab type must register identically on client and server.
fn assert_prefab_roles_are_stable(client: &TypeRegistry, server: &TypeRegistry) {
    for (type_id, tag, version) in [
        (TypeId::of::<Mesh>(), "Mesh", 2),
        (TypeId::of::<MaterialAssignment>(), "MaterialAssignment", 1),
        (TypeId::of::<Camera>(), "Camera", 1),
        (TypeId::of::<DirectionalLight>(), "DirectionalLight", 1),
        (TypeId::of::<PointLight>(), "PointLight", 1),
        (TypeId::of::<SpotLight>(), "SpotLight", 1),
    ] {
        let registrations = [client, server]
            .map(|registry| registry.get(type_id).expect("render Prefab registration"));
        assert_eq!(
            registrations[0].type_info().type_path(),
            registrations[1].type_info().type_path()
        );
        for registration in registrations {
            let prefab = registration
                .data::<PrefabTypeData>()
                .expect("Prefab TypeData");
            assert_eq!(prefab.tag, tag);
            assert_eq!(prefab.source_version, version);
            let TypeInfo::Struct(info) = registration.type_info() else {
                panic!("render Prefab should reflect as a struct");
            };
            assert!(
                info.get_attribute::<EditorTypeAttributes>()
                    .and_then(|attributes| attributes.label.as_deref())
                    .is_some()
            );
            assert!(registration.data::<ApplicabilityTypeData>().is_some());
        }
    }
}

/// Mesh and material-slot fields must project their asset-picker widgets.
fn assert_mesh_editor_attributes() {
    let TypeInfo::Struct(mesh) = Mesh::type_info() else {
        panic!("Mesh should reflect as a struct");
    };
    assert_eq!(
        mesh.field("mesh")
            .and_then(|field| field.get_attribute::<EditorFieldAttributes>())
            .map(|attributes| &attributes.widget),
        Some(&EditorWidget::AssetPicker {
            asset_type_path: "mesh".to_owned(),
        })
    );
    let TypeInfo::Struct(material_slot) = MaterialSlotOverride::type_info() else {
        panic!("MaterialSlotOverride should reflect as a struct");
    };
    assert_eq!(
        material_slot
            .field("material")
            .and_then(|field| field.get_attribute::<EditorFieldAttributes>())
            .map(|attributes| &attributes.widget),
        Some(&EditorWidget::AssetPicker {
            asset_type_path: "material".to_owned(),
        })
    );
}

/// Light fields must project their colour widget and unit suffixes, and
/// `SpotLight` must additionally carry validation data.
fn assert_light_editor_attributes(client: &TypeRegistry) {
    let TypeInfo::Struct(directional) = DirectionalLight::type_info() else {
        panic!("DirectionalLight should reflect as a struct");
    };
    assert_eq!(
        directional
            .field("color")
            .and_then(|field| field.get_attribute::<EditorFieldAttributes>())
            .map(|attributes| &attributes.widget),
        Some(&EditorWidget::Color)
    );
    assert_eq!(
        directional
            .field("illuminance_lux")
            .and_then(|field| field.get_attribute::<EditorFieldAttributes>())
            .and_then(|attributes| attributes.range.as_ref())
            .and_then(|range| range.suffix.as_deref()),
        Some("lux")
    );
    let TypeInfo::Struct(point) = PointLight::type_info() else {
        panic!("PointLight should reflect as a struct");
    };
    assert_eq!(
        point
            .field("intensity_lumens")
            .and_then(|field| field.get_attribute::<EditorFieldAttributes>())
            .and_then(|attributes| attributes.range.as_ref())
            .and_then(|range| range.suffix.as_deref()),
        Some("lm")
    );
    let TypeInfo::Struct(spot) = SpotLight::type_info() else {
        panic!("SpotLight should reflect as a struct");
    };
    assert_eq!(
        spot.field("outer_angle_degrees")
            .and_then(|field| field.get_attribute::<EditorFieldAttributes>())
            .and_then(|attributes| attributes.range.as_ref())
            .and_then(|range| range.suffix.as_deref()),
        Some("°")
    );
    assert!(
        client
            .get(TypeId::of::<SpotLight>())
            .and_then(|registration| registration.data::<ValidationTypeData>())
            .is_some()
    );
}

#[test]
fn light_prefab_applicability_requires_transform_and_rejects_existing_light() {
    let registry = registry();
    for type_id in [
        TypeId::of::<DirectionalLight>(),
        TypeId::of::<PointLight>(),
        TypeId::of::<SpotLight>(),
    ] {
        let applicability = registry
            .get(type_id)
            .and_then(|registration| registration.data::<ApplicabilityTypeData>())
            .expect("light applicability");
        let applicable = (applicability.evaluate)(&ApplicabilityContext {
            capabilities: BTreeSet::from(["azoth.transform".to_owned()]),
            ..ApplicabilityContext::default()
        })
        .expect("light applicability result");
        assert!(applicable.applicable);

        let missing_transform = (applicability.evaluate)(&ApplicabilityContext::default()).unwrap();
        assert!(!missing_transform.applicable);
        assert_eq!(
            missing_transform.diagnostics[0].code,
            "light.requires_transform"
        );

        let duplicate = (applicability.evaluate)(&ApplicabilityContext {
            capabilities: BTreeSet::from(["azoth.transform".to_owned(), "azoth.light".to_owned()]),
            ..ApplicabilityContext::default()
        })
        .unwrap();
        assert!(!duplicate.applicable);
        assert_eq!(duplicate.diagnostics[0].code, "light.already_present");
    }
}

#[test]
fn mesh_prefab_migrates_v1_alias_round_trips_and_inserts_with_transform() {
    let registry = registry();
    let registration = registry
        .get(TypeId::of::<Mesh>())
        .expect("Mesh registration");
    let prefab = registration
        .data::<PrefabTypeData>()
        .expect("Mesh Prefab TypeData");
    assert_eq!(prefab.aliases[0].tag, "MeshComponent");
    assert_eq!(prefab.migrations.len(), 1);
    let default = registration
        .data::<ReflectDefault>()
        .expect("Mesh ReflectDefault")
        .default();
    assert_eq!(default.downcast_ref::<Mesh>(), Some(&Mesh::default()));

    let codec = PrefabCodec::new(&registry).expect("Mesh codec");
    let migrated = codec
        .decode(
            r#"(
                version: 1,
                type_versions: {"MeshComponent": 1},
                entities: {
                    "entity": (components: {
                        "MeshComponent": (mesh: "meshes/crate.azmesh"),
                    }),
                },
                instances: {},
            )"#,
        )
        .expect("decode and migrate Mesh v1 alias");
    assert_eq!(
        migrated.type_versions,
        BTreeMap::from([("Mesh".to_owned(), 2)])
    );
    let fields = migrated.entities[&EntityAlias::new("entity").unwrap()].components["Mesh"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("migrated Mesh struct");
    for field in ["visible", "cast_shadows", "receive_shadows", "lod_bias"] {
        assert!(fields.field(field).is_some(), "missing migrated {field}");
    }

    let encoded = codec.encode(&migrated).expect("encode Mesh");
    let decoded = codec.decode(&encoded).expect("decode Mesh");
    let value = &decoded.entities[&EntityAlias::new("entity").unwrap()].components["Mesh"];
    assert!(encoded.contains("meshes/crate.azmesh"));
    assert!(encoded.contains("visible"));

    let mut world = World::new();
    world.insert_resource(AppTypeRegistry::default());
    let entity = world.spawn_empty().id();
    let mut references = SceneEntityReferences::default();
    let built = {
        let mut entity_mut = world.entity_mut(entity);
        (prefab.construct)(
            registration,
            value.value(),
            &mut entity_mut,
            &mut references,
        )
        .expect("construct Mesh")
    };
    {
        let mut entity_mut = world.entity_mut(entity);
        (prefab.insert)(
            registration,
            built.value.as_ref(),
            &mut entity_mut,
            &registry,
        )
        .expect("insert Mesh");
    }
    let mesh = world.get::<Mesh>(entity).expect("inserted Mesh");
    assert_eq!(mesh.mesh.as_str(), "meshes/crate.azmesh");
    assert!(mesh.visible && mesh.cast_shadows && mesh.receive_shadows);
    assert_eq!(mesh.lod_bias, 0);
    assert!(world.get::<az_transform::Transform>(entity).is_some());
}

#[test]
fn material_assignment_prefab_retains_explicit_none_round_trip_and_inserts() {
    let registry = registry();
    let registration = registry
        .get(TypeId::of::<MaterialAssignment>())
        .expect("MaterialAssignment registration");
    let prefab = registration
        .data::<PrefabTypeData>()
        .expect("MaterialAssignment Prefab TypeData");
    let default = registration
        .data::<ReflectDefault>()
        .expect("MaterialAssignment ReflectDefault")
        .default();
    assert_eq!(
        default.downcast_ref::<MaterialAssignment>(),
        Some(&MaterialAssignment::default())
    );

    let codec = PrefabCodec::new(&registry).expect("MaterialAssignment codec");
    let source = codec
        .decode(
            r#"(
                version: 1,
                type_versions: {"MaterialAssignment": 1},
                entities: {
                    "entity": (components: {
                        "MaterialAssignment": (
                            default: None,
                            slots: [(
                                slot_id: Some(7),
                                slot_label: Some("Body"),
                                material: "materials/body.azmaterial",
                            )],
                        ),
                    }),
                },
                instances: {},
            )"#,
        )
        .expect("decode MaterialAssignment source");
    let encoded = codec.encode(&source).expect("encode MaterialAssignment");
    let decoded = codec.decode(&encoded).expect("decode MaterialAssignment");
    let value =
        &decoded.entities[&EntityAlias::new("entity").unwrap()].components["MaterialAssignment"];
    let fields = value
        .value()
        .reflect_ref()
        .as_struct()
        .expect("sparse MaterialAssignment struct");
    assert!(fields.field("default").is_some());
    assert!(fields.field("slots").is_some());
    assert!(encoded.contains("default"));
    assert!(encoded.contains("materials/body.azmaterial"));

    let mut world = World::new();
    world.insert_resource(AppTypeRegistry::default());
    let entity = world.spawn_empty().id();
    let mut references = SceneEntityReferences::default();
    let built = {
        let mut entity_mut = world.entity_mut(entity);
        (prefab.construct)(
            registration,
            value.value(),
            &mut entity_mut,
            &mut references,
        )
        .expect("construct MaterialAssignment")
    };
    {
        let mut entity_mut = world.entity_mut(entity);
        (prefab.insert)(
            registration,
            built.value.as_ref(),
            &mut entity_mut,
            &registry,
        )
        .expect("insert MaterialAssignment");
    }
    let assignment = world
        .get::<MaterialAssignment>(entity)
        .expect("inserted MaterialAssignment");
    assert_eq!(assignment.default, None);
    assert_eq!(assignment.slots.len(), 1);
    assert_eq!(assignment.slots[0].slot_id, Some(7));
    assert_eq!(assignment.slots[0].slot_label.as_deref(), Some("Body"));
    assert_eq!(
        assignment.slots[0].material.as_str(),
        "materials/body.azmaterial"
    );
}

#[test]
fn camera_prefab_round_trips_valid_projection_and_inserts_required_transform() {
    let registry = registry();
    let registration = registry
        .get(TypeId::of::<Camera>())
        .expect("Camera registration");
    let prefab = registration
        .data::<PrefabTypeData>()
        .expect("Camera Prefab TypeData");
    let default = registration
        .data::<ReflectDefault>()
        .expect("Camera ReflectDefault")
        .default();
    assert_eq!(default.downcast_ref::<Camera>(), Some(&Camera::default()));
    assert!(registration.data::<ValidationTypeData>().is_some());

    let projection = CameraProjection::Orthographic(OrthographicCameraProjection::default());
    let mut sparse = DynamicStruct::default();
    sparse.set_represented_type(Some(Camera::type_info()));
    sparse.insert("projection", projection);
    sparse.insert("near", Camera::default().near);
    let codec = PrefabCodec::new(&registry).expect("Camera codec");
    let encoded = codec
        .encode(&document_with_component(
            "Camera",
            1,
            SparseValue::try_new(Box::new(sparse)).unwrap(),
        ))
        .expect("encode Camera");
    let decoded = codec.decode(&encoded).expect("decode Camera");
    let value = &decoded.entities[&EntityAlias::new("entity").unwrap()].components["Camera"];
    let fields = value
        .value()
        .reflect_ref()
        .as_struct()
        .expect("sparse Camera struct");
    assert!(fields.field("projection").is_some());
    assert!(fields.field("near").is_some());
    assert!(fields.field("far").is_none());

    let mut world = World::new();
    world.insert_resource(AppTypeRegistry::default());
    let entity = world.spawn_empty().id();
    let mut references = SceneEntityReferences::default();
    let built = {
        let mut entity_mut = world.entity_mut(entity);
        (prefab.construct)(
            registration,
            value.value(),
            &mut entity_mut,
            &mut references,
        )
        .expect("construct Camera")
    };
    {
        let mut entity_mut = world.entity_mut(entity);
        (prefab.insert)(
            registration,
            built.value.as_ref(),
            &mut entity_mut,
            &registry,
        )
        .expect("insert Camera");
    }
    assert!(matches!(
        world.get::<Camera>(entity).map(|camera| camera.projection),
        Some(CameraProjection::Orthographic(_))
    ));
    assert!(world.get::<az_transform::Transform>(entity).is_some());
}

#[test]
fn camera_validation_reports_scalar_cross_field_and_projection_paths() {
    let registry = registry();
    let validation = registry
        .get(TypeId::of::<Camera>())
        .and_then(|registration| registration.data::<ValidationTypeData>())
        .expect("Camera validation");
    let diagnostics = (validation.validate)(&Camera {
        projection: CameraProjection::Perspective(PerspectiveCameraProjection {
            fov_y_degrees: 180.0,
        }),
        near: 2.0,
        far: 1.0,
        ..Camera::default()
    })
    .expect("Camera validation result");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "camera.near_before_far"
            && diagnostic.path
                == ReflectedPath(vec![ReflectedPathSegment::Field("near".to_owned())])
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "camera.projection.perspective.fov"
            && diagnostic.path.0
                == vec![
                    ReflectedPathSegment::Field("projection".to_owned()),
                    ReflectedPathSegment::Variant("Perspective".to_owned()),
                    ReflectedPathSegment::TupleIndex(0),
                    ReflectedPathSegment::Field("fov_y_degrees".to_owned()),
                ]
    }));

    let diagnostics = (validation.validate)(&Camera {
        projection: CameraProjection::Orthographic(OrthographicCameraProjection {
            half_height: f32::NAN,
        }),
        ..Camera::default()
    })
    .expect("orthographic validation result");
    assert_eq!(
        diagnostics[0].path.0,
        vec![
            ReflectedPathSegment::Field("projection".to_owned()),
            ReflectedPathSegment::Variant("Orthographic".to_owned()),
            ReflectedPathSegment::TupleIndex(0),
            ReflectedPathSegment::Field("half_height".to_owned()),
        ]
    );
    assert!(
        (validation.validate)(&Camera::default())
            .expect("valid default Camera")
            .is_empty()
    );
}

#[test]
fn directional_light_prefab_retains_explicit_default_round_trips_and_inserts_transform() {
    let registry = registry();
    let registration = registry
        .get(TypeId::of::<DirectionalLight>())
        .expect("DirectionalLight registration");
    let default = registration
        .data::<ReflectDefault>()
        .expect("DirectionalLight ReflectDefault")
        .default();
    assert_eq!(
        default.downcast_ref::<DirectionalLight>(),
        Some(&DirectionalLight::default())
    );

    let mut sparse = DynamicStruct::default();
    sparse.set_represented_type(Some(DirectionalLight::type_info()));
    sparse.insert(
        "illuminance_lux",
        DirectionalLight::default().illuminance_lux,
    );
    sparse.insert("shadows_enabled", true);
    let codec = PrefabCodec::new(&registry).expect("DirectionalLight codec");
    let encoded = codec
        .encode(&document_with_component(
            "DirectionalLight",
            1,
            SparseValue::try_new(Box::new(sparse)).unwrap(),
        ))
        .expect("encode DirectionalLight");
    let decoded = codec.decode(&encoded).expect("decode DirectionalLight");
    let value =
        &decoded.entities[&EntityAlias::new("entity").unwrap()].components["DirectionalLight"];
    let fields = value
        .value()
        .reflect_ref()
        .as_struct()
        .expect("sparse DirectionalLight struct");
    assert!(fields.field("illuminance_lux").is_some());
    assert!(fields.field("shadows_enabled").is_some());
    assert!(fields.field("color").is_none());
    assert!(encoded.contains("illuminance_lux"));

    let (world, entity) = construct_and_insert(&registry, TypeId::of::<DirectionalLight>(), value);
    assert_eq!(
        world.get::<DirectionalLight>(entity),
        Some(&DirectionalLight {
            shadows_enabled: true,
            ..DirectionalLight::default()
        })
    );
    assert!(world.get::<az_transform::Transform>(entity).is_some());
}

#[test]
fn point_light_prefab_retains_explicit_default_round_trips_and_inserts_transform() {
    let registry = registry();
    let registration = registry
        .get(TypeId::of::<PointLight>())
        .expect("PointLight registration");
    let default = registration
        .data::<ReflectDefault>()
        .expect("PointLight ReflectDefault")
        .default();
    assert_eq!(
        default.downcast_ref::<PointLight>(),
        Some(&PointLight::default())
    );

    let mut sparse = DynamicStruct::default();
    sparse.set_represented_type(Some(PointLight::type_info()));
    sparse.insert("radius", PointLight::default().radius);
    sparse.insert("intensity_lumens", 2_250.0_f32);
    let codec = PrefabCodec::new(&registry).expect("PointLight codec");
    let encoded = codec
        .encode(&document_with_component(
            "PointLight",
            1,
            SparseValue::try_new(Box::new(sparse)).unwrap(),
        ))
        .expect("encode PointLight");
    let decoded = codec.decode(&encoded).expect("decode PointLight");
    let value = &decoded.entities[&EntityAlias::new("entity").unwrap()].components["PointLight"];
    let fields = value
        .value()
        .reflect_ref()
        .as_struct()
        .expect("sparse PointLight struct");
    assert!(fields.field("radius").is_some());
    assert!(fields.field("intensity_lumens").is_some());
    assert!(fields.field("range").is_none());
    assert!(encoded.contains("radius"));

    let (world, entity) = construct_and_insert(&registry, TypeId::of::<PointLight>(), value);
    assert_eq!(
        world.get::<PointLight>(entity),
        Some(&PointLight {
            intensity_lumens: 2_250.0,
            ..PointLight::default()
        })
    );
    assert!(world.get::<az_transform::Transform>(entity).is_some());
}

#[test]
fn spot_light_prefab_retains_explicit_default_round_trips_and_inserts_transform() {
    let registry = registry();
    let registration = registry
        .get(TypeId::of::<SpotLight>())
        .expect("SpotLight registration");
    let default = registration
        .data::<ReflectDefault>()
        .expect("SpotLight ReflectDefault")
        .default();
    assert_eq!(
        default.downcast_ref::<SpotLight>(),
        Some(&SpotLight::default())
    );

    let mut sparse = DynamicStruct::default();
    sparse.set_represented_type(Some(SpotLight::type_info()));
    sparse.insert(
        "inner_angle_degrees",
        SpotLight::default().inner_angle_degrees,
    );
    sparse.insert("outer_angle_degrees", 60.0_f32);
    let codec = PrefabCodec::new(&registry).expect("SpotLight codec");
    let encoded = codec
        .encode(&document_with_component(
            "SpotLight",
            1,
            SparseValue::try_new(Box::new(sparse)).unwrap(),
        ))
        .expect("encode SpotLight");
    let decoded = codec.decode(&encoded).expect("decode SpotLight");
    let value = &decoded.entities[&EntityAlias::new("entity").unwrap()].components["SpotLight"];
    let fields = value
        .value()
        .reflect_ref()
        .as_struct()
        .expect("sparse SpotLight struct");
    assert!(fields.field("inner_angle_degrees").is_some());
    assert!(fields.field("outer_angle_degrees").is_some());
    assert!(fields.field("intensity_lumens").is_none());
    assert!(encoded.contains("inner_angle_degrees"));

    let (world, entity) = construct_and_insert(&registry, TypeId::of::<SpotLight>(), value);
    assert_eq!(
        world.get::<SpotLight>(entity),
        Some(&SpotLight {
            outer_angle_degrees: 60.0,
            ..SpotLight::default()
        })
    );
    assert!(world.get::<az_transform::Transform>(entity).is_some());
}

#[test]
fn spot_light_validation_rejects_invalid_angles_with_named_paths() {
    let registry = registry();
    let validation = registry
        .get(TypeId::of::<SpotLight>())
        .and_then(|registration| registration.data::<ValidationTypeData>())
        .expect("SpotLight validation");

    let inner_after_outer = (validation.validate)(&SpotLight {
        inner_angle_degrees: 60.0,
        outer_angle_degrees: 45.0,
        ..SpotLight::default()
    })
    .expect("SpotLight relationship validation");
    assert_eq!(inner_after_outer.len(), 1);
    assert_eq!(
        inner_after_outer[0].path,
        ReflectedPath(vec![ReflectedPathSegment::Field(
            "inner_angle_degrees".to_owned()
        )])
    );
    assert_eq!(inner_after_outer[0].code, "spot_light.inner_before_outer");

    let outer_at_limit = (validation.validate)(&SpotLight {
        outer_angle_degrees: 180.0,
        ..SpotLight::default()
    })
    .expect("SpotLight outer angle validation");
    assert_eq!(outer_at_limit.len(), 1);
    assert_eq!(
        outer_at_limit[0].path,
        ReflectedPath(vec![ReflectedPathSegment::Field(
            "outer_angle_degrees".to_owned()
        )])
    );
    assert_eq!(outer_at_limit[0].code, "spot_light.outer_angle.invalid");

    let negative_inner = (validation.validate)(&SpotLight {
        inner_angle_degrees: -0.1,
        ..SpotLight::default()
    })
    .expect("SpotLight inner angle validation");
    assert_eq!(negative_inner.len(), 1);
    assert_eq!(
        negative_inner[0].path,
        ReflectedPath(vec![ReflectedPathSegment::Field(
            "inner_angle_degrees".to_owned()
        )])
    );
    assert_eq!(negative_inner[0].code, "spot_light.inner_angle.invalid");
    assert!(
        (validation.validate)(&SpotLight::default())
            .expect("valid default SpotLight")
            .is_empty()
    );
}
