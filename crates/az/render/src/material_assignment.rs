use std::any::TypeId;

use az_core::{
    ApplicabilityContext, ApplicabilityResult, ApplicabilityTypeData, AssetPathBuf, AzComponent,
    AzRtti, AzTypeInfo, DiagnosticSeverity, EditorFieldAttributes, EditorTypeAttributes,
    EditorWidget, ReflectedPath, ValidationDiagnostic,
};
use az_prefab::{Prefab, ReflectPrefab};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::{Reflect, TypeRegistry, std_traits::ReflectDefault};
use uuid::{Uuid, uuid};

pub const MATERIAL_ASSIGNMENT_SCHEMA_NAME: &str = "azoth.render.MaterialAssignment";
pub const MATERIAL_SLOT_OVERRIDE_SCHEMA_NAME: &str = "azoth.render.MaterialSlotOverride";
pub const MATERIAL_ASSIGNMENT_COMPONENT_TYPE_ID: Uuid =
    uuid!("0bc6ff6e-eebd-4b0e-94af-11e68135f630");

/// One material override addressed by stable imported slot ID, label, or both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
#[reflect(Default)]
#[reflect(@EditorTypeAttributes::labeled("Material Slot Override").in_group("Rendering"))]
pub struct MaterialSlotOverride {
    #[reflect(@EditorFieldAttributes::new("Slot ID", EditorWidget::Number))]
    pub slot_id: Option<u32>,
    #[reflect(@EditorFieldAttributes::new("Slot Label", EditorWidget::Default))]
    pub slot_label: Option<String>,
    #[reflect(@EditorFieldAttributes::new(
        "Material",
        EditorWidget::AssetPicker {
            asset_type_path: "material".to_owned(),
        },
    ))]
    pub material: AssetPathBuf,
}

/// Default material plus sparse overrides for imported mesh slots.
#[derive(Debug, Clone, PartialEq, Eq, Component, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("Material Assignment")
    .in_group("Rendering")
    .with_icon("palette")
    .with_description("Default material and per-slot material overrides."))]
#[prefab(tag = "MaterialAssignment", version = 1)]
#[derive(Default)]
pub struct MaterialAssignment {
    #[reflect(@EditorFieldAttributes::new(
        "Default Material",
        EditorWidget::AssetPicker {
            asset_type_path: "material".to_owned(),
        },
    ))]
    pub default: Option<AssetPathBuf>,
    #[reflect(@EditorFieldAttributes::new(
        "Slot Overrides",
        EditorWidget::Default,
    ))]
    pub slots: Vec<MaterialSlotOverride>,
}

impl AzTypeInfo for MaterialAssignment {
    const NAME: &'static str = MATERIAL_ASSIGNMENT_SCHEMA_NAME;
    const TYPE_ID: Uuid = MATERIAL_ASSIGNMENT_COMPONENT_TYPE_ID;
}

impl AzRtti for MaterialAssignment {
    const BASE_TYPE_IDS: &'static [Uuid] = &[az_core::component::COMPONENT_TYPE_ID];
}

impl AzComponent for MaterialAssignment {}

pub fn register_prefab_types(registry: &mut TypeRegistry) {
    registry.register::<AssetPathBuf>();
    registry.register::<Option<AssetPathBuf>>();
    registry.register::<Option<u32>>();
    registry.register::<Option<String>>();
    registry.register::<MaterialSlotOverride>();
    registry.register::<Vec<MaterialSlotOverride>>();
    registry.register::<MaterialAssignment>();
    registry
        .get_mut(TypeId::of::<MaterialAssignment>())
        .expect("a just-registered MaterialAssignment must be present")
        .insert(ApplicabilityTypeData {
            evaluate: material_assignment_applicability,
            provides: &[],
            requires: &["azoth.renderable", "azoth.material-slots"],
            incompatible: &[],
        });
}

// Signature is fixed by `ErasedApplicabilityFn`, the fn-pointer type this is
// stored as; unwrapping the `Result` would not coerce.
#[allow(clippy::unnecessary_wraps)]
fn material_assignment_applicability(
    context: &ApplicabilityContext,
) -> Result<ApplicabilityResult, az_core::EditorPolicyError> {
    const RENDERABLE: &str = "azoth.renderable";
    const MATERIAL_SLOTS: &str = "azoth.material-slots";

    let missing = [RENDERABLE, MATERIAL_SLOTS]
        .into_iter()
        .filter(|capability| !context.capabilities.contains(*capability))
        .collect::<Vec<_>>();
    Ok(ApplicabilityResult {
        applicable: missing.is_empty(),
        diagnostics: missing
            .into_iter()
            .map(|capability| ValidationDiagnostic {
                path: ReflectedPath::default(),
                severity: DiagnosticSeverity::Error,
                code: "material_assignment.missing_capability".to_owned(),
                message: format!("material assignment requires `{capability}`"),
            })
            .collect(),
    })
}
