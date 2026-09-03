use std::any::TypeId;

use az_core::{
    ApplicabilityContext, ApplicabilityResult, ApplicabilityTypeData, AssetPathBuf, AzComponent,
    AzRtti, AzTypeInfo, DiagnosticSeverity, EditorFieldAttributes, EditorNumericRange,
    EditorTypeAttributes, EditorWidget, ReflectedPath, ValidationDiagnostic,
};
use az_prefab::{ErasedPrefabValue, Prefab, PrefabBuildError, ReflectPrefab};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::{Reflect, TypeRegistry, Typed, std_traits::ReflectDefault, structs::Struct};
use uuid::Uuid;

use crate::generated;

pub const MESH_SCHEMA_NAME: &str = "azoth.render.Mesh";
pub const MESH_COMPONENT_TYPE_ID: Uuid = generated::MESH_COMPONENT_TYPE_ID;

/// Authored reference to a cooked Azoth mesh product.
#[derive(Debug, Clone, PartialEq, Eq, Component, Reflect, Prefab)]
#[require(az_transform::Transform)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("Mesh")
    .in_group("Rendering")
    .with_icon("deployed_code")
    .with_description("Renderable cooked mesh/model product attached to an entity."))]
#[prefab(
    tag = "Mesh",
    version = 2,
    alias(tag = "MeshComponent", version = 1, migrate = migrate_mesh_v1_to_v2)
)]
pub struct Mesh {
    #[reflect(@EditorFieldAttributes::new(
        "Mesh",
        EditorWidget::AssetPicker {
            asset_type_path: "mesh".to_owned(),
        },
    ))]
    pub mesh: AssetPathBuf,
    #[reflect(@EditorFieldAttributes::new("Visible", EditorWidget::Toggle))]
    pub visible: bool,
    #[reflect(@EditorFieldAttributes::new("Cast Shadows", EditorWidget::Toggle))]
    pub cast_shadows: bool,
    #[reflect(@EditorFieldAttributes::new(
        "Receive Shadows",
        EditorWidget::Toggle,
    ))]
    pub receive_shadows: bool,
    #[reflect(@EditorFieldAttributes::new("LOD Bias", EditorWidget::Slider)
        .with_range(EditorNumericRange {
            minimum: Some("-2".to_owned()),
            maximum: Some("2".to_owned()),
            step: Some("1".to_owned()),
            suffix: None,
        }))]
    pub lod_bias: i32,
}

impl Default for Mesh {
    fn default() -> Self {
        Self {
            mesh: AssetPathBuf::default(),
            visible: true,
            cast_shadows: true,
            receive_shadows: true,
            lod_bias: 0,
        }
    }
}

impl AzTypeInfo for Mesh {
    const NAME: &'static str = MESH_SCHEMA_NAME;
    const TYPE_ID: Uuid = MESH_COMPONENT_TYPE_ID;
}

impl AzRtti for Mesh {
    const BASE_TYPE_IDS: &'static [Uuid] = &[az_core::component::COMPONENT_TYPE_ID];
}

impl AzComponent for Mesh {}

fn migrate_mesh_v1_to_v2(
    mut value: ErasedPrefabValue,
) -> Result<ErasedPrefabValue, PrefabBuildError> {
    let mut sparse = value
        .value
        .reflect_ref()
        .as_struct()
        .map(Struct::to_dynamic_struct)
        .map_err(|_| PrefabBuildError::ApplyFailed {
            type_path: Mesh::type_info().type_path(),
            message: "expected sparse Mesh struct".to_owned(),
        })?;
    sparse.insert("visible", true);
    sparse.insert("cast_shadows", true);
    sparse.insert("receive_shadows", true);
    sparse.insert("lod_bias", 0_i32);
    value.value = Box::new(sparse);
    Ok(value)
}

pub fn register_prefab_type(registry: &mut TypeRegistry) {
    registry.register::<AssetPathBuf>();
    registry.register::<Mesh>();
    registry
        .get_mut(TypeId::of::<Mesh>())
        .expect("a just-registered Mesh must be present")
        .insert(ApplicabilityTypeData {
            evaluate: mesh_applicability,
            provides: &[
                "azoth.renderable",
                "azoth.material-slots",
                "azoth.bounds-provider",
            ],
            requires: &[],
            incompatible: &["azoth.renderable"],
        });
}

// Signature is fixed by `ErasedApplicabilityFn`, the fn-pointer type this is
// stored as; unwrapping the `Result` would not coerce.
#[allow(clippy::unnecessary_wraps)]
fn mesh_applicability(
    context: &ApplicabilityContext,
) -> Result<ApplicabilityResult, az_core::EditorPolicyError> {
    const TRANSFORM: &str = "azoth.transform";
    const RENDERABLE: &str = "azoth.renderable";

    let has_transform = context.capabilities.contains(TRANSFORM);
    let duplicate = context.capabilities.contains(RENDERABLE);
    let mut diagnostics = Vec::new();
    if !has_transform {
        diagnostics.push(applicability_diagnostic(
            "mesh.requires_transform",
            "mesh requires a transform capability",
        ));
    }
    if duplicate {
        diagnostics.push(applicability_diagnostic(
            "mesh.renderable_conflict",
            "the selection already has a renderable capability",
        ));
    }
    Ok(ApplicabilityResult {
        applicable: has_transform && !duplicate,
        diagnostics,
    })
}

fn applicability_diagnostic(code: &str, message: &str) -> ValidationDiagnostic {
    ValidationDiagnostic {
        path: ReflectedPath::default(),
        severity: DiagnosticSeverity::Error,
        code: code.to_owned(),
        message: message.to_owned(),
    }
}
