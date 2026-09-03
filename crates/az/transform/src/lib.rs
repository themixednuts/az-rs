//! Engine-owned authored transform component.
//!
//! Prefab entity parenting owns hierarchy identity. [`Transform`] stores only
//! the entity's parent-relative translation, quaternion rotation, and
//! non-uniform scale. The Bevy feature registers that authored component
//! identity with a mapped inserter for Bevy's native `Transform` component.

use std::any::TypeId;

use az_core::{
    ApplicabilityContext, ApplicabilityResult, ApplicabilityTypeData, AzComponent, AzRtti,
    AzTypeInfo, DiagnosticSeverity, EditorFieldAttributes, EditorNumericRange,
    EditorTypeAttributes, EditorWidget, ReflectedPath, ReflectedPathSegment,
    ValidationCallbackError, ValidationDiagnostic, ValidationTypeData,
};
use az_prefab::{Prefab, ReflectPrefab};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::{PartialReflect, Reflect, TypeRegistry, std_traits::ReflectDefault};
use glam::{Quat, Vec3};
use uuid::{Uuid, uuid};

/// Stable authored schema name for the engine transform component.
pub const TRANSFORM_SCHEMA_NAME: &str = "azoth.transform.Transform";
/// Stable native component identity for the engine transform component.
pub const TRANSFORM_COMPONENT_TYPE_ID: Uuid = uuid!("60beb357-404c-4e51-834b-3c760b35a01f");

/// Locked field identities for [`Transform`].
pub mod ids {
    pub const POSITION: u32 = 2_471_448_074;
    pub const ROTATION: u32 = 564_937_055;
    pub const SCALE: u32 = 2_190_941_297;
}

/// Parent-relative authored transform attached to a prefab entity.
#[derive(Debug, Clone, Copy, PartialEq, Component, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("Transform")
    .in_group("Core")
    .with_icon("open_with")
    .with_description(
        "Parent-relative position, quaternion rotation, and non-uniform scale.",
    ))]
#[prefab(tag = "Transform", version = 1)]
pub struct Transform {
    #[reflect(@EditorFieldAttributes::new(
        "Position",
        EditorWidget::Vector { dimensions: 3 },
    ).with_range(EditorNumericRange {
        suffix: Some("m".to_owned()),
        ..EditorNumericRange::default()
    }))]
    pub position: Vec3,

    #[reflect(@EditorFieldAttributes::new(
        "Rotation",
        EditorWidget::Vector { dimensions: 4 },
    ))]
    pub rotation: Quat,

    #[reflect(@EditorFieldAttributes::new(
        "Scale",
        EditorWidget::Vector { dimensions: 3 },
    ))]
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl AzTypeInfo for Transform {
    const NAME: &'static str = TRANSFORM_SCHEMA_NAME;
    const TYPE_ID: Uuid = TRANSFORM_COMPONENT_TYPE_ID;
}

impl AzRtti for Transform {
    const BASE_TYPE_IDS: &'static [Uuid] = &[az_core::component::COMPONENT_TYPE_ID];
}
impl AzComponent for Transform {}

const TRANSFORM_CAPABILITY: &str = "azoth.transform";

/// Registers the Bevy-native Transform and its editor policy in a caller-owned
/// registry. Client, server, builder, and project-host composition all call
/// this same function.
///
/// # Panics
///
/// Panics if `Transform` is absent from the registry immediately after this
/// function registers it, which would mean the registry did not honour the
/// registration.
pub fn register_transform_prefab_types(registry: &mut TypeRegistry) {
    registry.register::<Vec3>();
    registry.register::<Quat>();
    registry.register::<Transform>();
    let registration = registry
        .get_mut(TypeId::of::<Transform>())
        .expect("a just-registered Transform must be present");
    registration.insert(ValidationTypeData {
        validate: validate_reflected_transform,
    });
    registration.insert(ApplicabilityTypeData {
        evaluate: transform_applicability,
        provides: &[TRANSFORM_CAPABILITY],
        requires: &[],
        incompatible: &[TRANSFORM_CAPABILITY],
    });
}

fn validate_reflected_transform(
    value: &dyn PartialReflect,
) -> Result<Vec<ValidationDiagnostic>, ValidationCallbackError> {
    let transform = value.try_downcast_ref::<Transform>().ok_or_else(|| {
        ValidationCallbackError::IncompatibleValue(value.reflect_type_path().to_owned())
    })?;
    let mut diagnostics = Vec::new();
    if !transform.rotation.is_finite() {
        diagnostics.push(field_diagnostic(
            "rotation",
            "transform.rotation.non_finite",
            "rotation must contain only finite values",
        ));
    }
    if !transform.scale.is_finite() {
        diagnostics.push(field_diagnostic(
            "scale",
            "transform.scale.non_finite",
            "scale must contain only finite values",
        ));
    }
    Ok(diagnostics)
}

// Signature is fixed by `ErasedApplicabilityFn`, the fn-pointer type this is
// stored as; unwrapping the `Result` would not coerce.
#[allow(clippy::unnecessary_wraps)]
fn transform_applicability(
    context: &ApplicabilityContext,
) -> Result<ApplicabilityResult, az_core::EditorPolicyError> {
    let duplicate = context.capabilities.contains(TRANSFORM_CAPABILITY);
    Ok(ApplicabilityResult {
        applicable: !duplicate,
        diagnostics: duplicate
            .then(|| {
                field_diagnostic(
                    "",
                    "transform.already_present",
                    "the selection already has a transform capability",
                )
            })
            .into_iter()
            .collect(),
    })
}

fn field_diagnostic(field: &str, code: &str, message: &str) -> ValidationDiagnostic {
    ValidationDiagnostic {
        path: ReflectedPath(
            (!field.is_empty())
                .then(|| ReflectedPathSegment::Field(field.to_owned()))
                .into_iter()
                .collect(),
        ),
        severity: DiagnosticSeverity::Error,
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

/// The component lowerings this crate owns.
///
/// Which adapter [`Transform`] gets is this crate's `bevy` feature, which
/// forwards `az-core/bevy`: without it there is no Bevy component to insert, so
/// the entry carries identity alone.
#[must_use]
pub const fn lowerings() -> [az_core::ComponentLoweringRegistration; 1] {
    #[cfg(not(feature = "bevy"))]
    {
        [az_core::ComponentLoweringRegistration::component::<Transform>()]
    }
    #[cfg(feature = "bevy")]
    {
        [
            az_core::ComponentLoweringRegistration::mapped_bevy_component_with_values::<
                Transform,
                bevy::prelude::Transform,
            >(apply_bevy_transform),
        ]
    }
}

/// Register this crate's component lowerings into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<az_core::ComponentLoweringRegistration>()
        .register_many(lowerings());
}

#[cfg(feature = "bevy")]
fn apply_bevy_transform(
    entity: &mut bevy::ecs::system::EntityCommands<'_>,
    value: &dyn az_core::rtti::BevyComponentValue,
) -> Result<(), az_core::rtti::BevyComponentValueError> {
    use az_core::rtti::bevy_value::{optional_field, quat, required_struct, vec3};

    let value = required_struct(value, &[])?;
    let mut transform = bevy::prelude::Transform::default();
    if let Some(position) = optional_field(value, ids::POSITION, &[])? {
        transform.translation = vec3(position, &[ids::POSITION])?;
    }
    if let Some(rotation) = optional_field(value, ids::ROTATION, &[])? {
        transform.rotation = quat(rotation, &[ids::ROTATION])?;
    }
    if let Some(scale) = optional_field(value, ids::SCALE, &[])? {
        transform.scale = vec3(scale, &[ids::SCALE])?;
    }
    entity.insert(transform);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{any::TypeId, collections::BTreeMap};

    use super::*;
    use az_core::{EditorFieldAttributes, ValidationTypeData};
    use az_prefab::{
        EntityAlias, PrefabCodec, PrefabDocument, PrefabEntity, PrefabTypeData, SparseValue,
    };
    use bevy_ecs::{reflect::AppTypeRegistry, template::SceneEntityReferences, world::World};
    use bevy_reflect::{
        TypeInfo, TypeRegistry, Typed, std_traits::ReflectDefault, structs::DynamicStruct,
    };

    #[test]
    fn transform_defaults_to_parent_relative_identity() {
        assert_eq!(
            Transform::default(),
            Transform {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            }
        );
    }

    az_gem_contract::declare_caps!(TransformCaps:);

    /// Stands in for the host contribution the gem sweep writes for this crate.
    struct Engine;

    impl az_gem_contract::Contribution for Engine {
        type Caps = TransformCaps;

        fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
            az_gem_contract::ContributionDescriptor {
                gem: az_gem_contract::GemId::new("azoth.transform"),
                contribution: az_gem_contract::ContributionId::new("components"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, TransformCaps>) {
            register(ctx);
        }
    }

    fn composed() -> az_gem_contract::Composer {
        let mut composer =
            az_gem_contract::Composer::new(az_gem_contract::GemTargetRole::AssetWorker);
        composer
            .add(Engine, az_gem_contract::ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    fn composed_transform() -> az_core::ComponentLoweringRegistration {
        let composer = composed();
        az_core::component::lowering::composed(composer.registries())
            .into_iter()
            .find(|registration| registration.type_registration.name == TRANSFORM_SCHEMA_NAME)
            .expect("transform component registration")
    }

    #[test]
    fn component_registry_uses_transform_schema_identity() {
        let registration = composed_transform();
        assert_eq!(
            registration.type_registration.native_type_id,
            TRANSFORM_COMPONENT_TYPE_ID
        );
        assert_eq!(
            registration.type_registration.base_type_ids,
            &[az_core::component::COMPONENT_TYPE_ID]
        );
        assert_eq!(
            (registration.type_registration.rust_type_id)(),
            std::any::TypeId::of::<Transform>()
        );
    }

    #[test]
    fn transform_bevy_registry_projects_prefab_defaults_and_editor_attributes() {
        let mut client = TypeRegistry::default();
        register_transform_prefab_types(&mut client);
        let mut server = TypeRegistry::default();
        register_transform_prefab_types(&mut server);

        for registry in [&client, &server] {
            let registration = registry
                .get(TypeId::of::<Transform>())
                .expect("Transform registration");
            let prefab = registration
                .data::<PrefabTypeData>()
                .expect("Prefab TypeData");
            assert_eq!(prefab.tag, "Transform");
            assert_eq!(prefab.source_version, 1);
            let default = registration
                .data::<ReflectDefault>()
                .expect("ReflectDefault")
                .default();
            let default = default
                .downcast_ref::<Transform>()
                .expect("typed Transform default");
            assert_eq!(default.scale, Vec3::ONE);
            assert!(registration.data::<ValidationTypeData>().is_some());

            let TypeInfo::Struct(info) = registration.type_info() else {
                panic!("Transform should reflect as a struct");
            };
            assert_eq!(
                info.get_attribute::<EditorTypeAttributes>()
                    .and_then(|attributes| attributes.label.as_deref()),
                Some("Transform")
            );
            assert_eq!(
                info.field("position")
                    .and_then(|field| field.get_attribute::<EditorFieldAttributes>())
                    .map(|attributes| &attributes.widget),
                Some(&EditorWidget::Vector { dimensions: 3 })
            );
        }
    }

    #[test]
    fn transform_prefab_sparse_round_trip_retains_explicit_default_and_inserts() {
        let mut registry = TypeRegistry::default();
        register_transform_prefab_types(&mut registry);
        let mut sparse = DynamicStruct::default();
        sparse.set_represented_type(Some(Transform::type_info()));
        sparse.insert("scale", Vec3::ONE);
        let mut document = PrefabDocument::default();
        document.type_versions.insert("Transform".to_owned(), 1);
        document.entities.insert(
            EntityAlias::new("root").unwrap(),
            PrefabEntity {
                entity_id: None,
                parent: None,
                components: BTreeMap::from([(
                    "Transform".to_owned(),
                    SparseValue::try_new(Box::new(sparse)).unwrap(),
                )]),
            },
        );

        let codec = PrefabCodec::new(&registry).expect("Transform codec");
        let encoded = codec.encode(&document).expect("encode Transform");
        let decoded = codec
            .decode(&encoded)
            .unwrap_or_else(|error| panic!("decode Transform from `{encoded}`: {error}"));
        let value = &decoded.entities[&EntityAlias::new("root").unwrap()].components["Transform"];
        let fields = value
            .value()
            .reflect_ref()
            .as_struct()
            .expect("sparse Transform struct");
        assert!(fields.field("scale").is_some());
        assert!(fields.field("position").is_none());
        assert!(encoded.contains("scale"));

        let registration = registry
            .get(TypeId::of::<Transform>())
            .expect("Transform registration");
        let prefab = registration
            .data::<PrefabTypeData>()
            .expect("Prefab TypeData");
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
            .expect("construct Transform")
        };
        {
            let mut entity_mut = world.entity_mut(entity);
            (prefab.insert)(
                registration,
                built.value.as_ref(),
                &mut entity_mut,
                &registry,
            )
            .expect("insert Transform");
        }
        assert_eq!(world.get::<Transform>(entity), Some(&Transform::default()));
    }

    #[test]
    fn transform_validation_reports_named_non_finite_paths() {
        let mut registry = TypeRegistry::default();
        register_transform_prefab_types(&mut registry);
        let validation = registry
            .get(TypeId::of::<Transform>())
            .and_then(|registration| registration.data::<ValidationTypeData>())
            .expect("Transform validation");
        let diagnostics = (validation.validate)(&Transform {
            rotation: Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0),
            scale: Vec3::new(1.0, f32::INFINITY, 1.0),
            ..Transform::default()
        })
        .expect("validation result");
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics[0].path,
            ReflectedPath(vec![ReflectedPathSegment::Field("rotation".to_owned())])
        );
        assert_eq!(
            diagnostics[1].path,
            ReflectedPath(vec![ReflectedPathSegment::Field("scale".to_owned())])
        );
    }

    #[cfg(feature = "bevy")]
    #[test]
    fn bevy_registration_inserts_native_bevy_transform() {
        use bevy::ecs::system::Commands;
        use bevy::ecs::world::{CommandQueue, World};

        let registration = composed_transform();
        let inserter = registration
            .bevy_component
            .expect("mapped Bevy transform registration");
        assert!(inserter.apply_values.is_some());
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let entity = {
            let mut commands = Commands::new(&mut queue, &world);
            let mut entity = commands.spawn_empty();
            (inserter.insert_default)(&mut entity);
            entity.id()
        };
        queue.apply(&mut world);

        assert_eq!(
            world.get::<bevy::prelude::Transform>(entity),
            Some(&bevy::prelude::Transform::default())
        );
    }
}
