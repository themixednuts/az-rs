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
use glam::Vec4;
use uuid::Uuid;

use crate::generated;

pub const DIRECTIONAL_LIGHT_SCHEMA_NAME: &str = "azoth.render.DirectionalLight";
pub const POINT_LIGHT_SCHEMA_NAME: &str = "azoth.render.PointLight";
pub const SPOT_LIGHT_SCHEMA_NAME: &str = "azoth.render.SpotLight";
pub const DIRECTIONAL_LIGHT_COMPONENT_TYPE_ID: Uuid =
    generated::DIRECTIONAL_LIGHT_COMPONENT_TYPE_ID;
pub const POINT_LIGHT_COMPONENT_TYPE_ID: Uuid = generated::POINT_LIGHT_COMPONENT_TYPE_ID;
pub const SPOT_LIGHT_COMPONENT_TYPE_ID: Uuid = generated::SPOT_LIGHT_COMPONENT_TYPE_ID;

/// A sun-like light whose direction is supplied by the entity Transform rotation.
#[derive(Debug, Clone, Copy, PartialEq, Component, Reflect, Prefab)]
#[require(az_transform::Transform)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("Directional Light")
    .in_group("Lighting")
    .with_icon("lightbulb")
    .with_description("Transform rotation supplies the direction of a photometric sun light."))]
#[prefab(tag = "DirectionalLight", version = 1)]
pub struct DirectionalLight {
    #[reflect(@EditorFieldAttributes::new("Color", EditorWidget::Color))]
    pub color: Vec4,

    #[reflect(@EditorFieldAttributes::new("Illuminance", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0".to_owned()),
            maximum: None,
            step: Some("100".to_owned()),
            suffix: Some("lux".to_owned()),
        }))]
    pub illuminance_lux: f32,

    #[reflect(@EditorFieldAttributes::new("Cast Shadows", EditorWidget::Toggle))]
    pub shadows_enabled: bool,

    #[reflect(@EditorFieldAttributes::new("Angular Diameter", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0".to_owned()),
            maximum: Some("179".to_owned()),
            step: Some("0.01".to_owned()),
            suffix: Some("°".to_owned()),
        }))]
    pub angular_diameter_degrees: f32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            color: Vec4::ONE,
            illuminance_lux: 10_000.0,
            shadows_enabled: false,
            angular_diameter_degrees: 0.53,
        }
    }
}

/// An omnidirectional photometric light with a finite range.
#[derive(Debug, Clone, Copy, PartialEq, Component, Reflect, Prefab)]
#[require(az_transform::Transform)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("Point Light")
    .in_group("Lighting")
    .with_icon("lightbulb")
    .with_description("Omnidirectional photometric light with a finite range."))]
#[prefab(tag = "PointLight", version = 1)]
pub struct PointLight {
    #[reflect(@EditorFieldAttributes::new("Color", EditorWidget::Color))]
    pub color: Vec4,

    #[reflect(@EditorFieldAttributes::new("Intensity", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0".to_owned()),
            maximum: None,
            step: Some("10".to_owned()),
            suffix: Some("lm".to_owned()),
        }))]
    pub intensity_lumens: f32,

    #[reflect(@EditorFieldAttributes::new("Range", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0".to_owned()),
            maximum: None,
            step: Some("0.1".to_owned()),
            suffix: Some("m".to_owned()),
        }))]
    pub range: f32,

    #[reflect(@EditorFieldAttributes::new("Radius", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0".to_owned()),
            maximum: None,
            step: Some("0.01".to_owned()),
            suffix: Some("m".to_owned()),
        }))]
    pub radius: f32,

    #[reflect(@EditorFieldAttributes::new("Cast Shadows", EditorWidget::Toggle))]
    pub shadows_enabled: bool,
}

impl Default for PointLight {
    fn default() -> Self {
        Self {
            color: Vec4::ONE,
            intensity_lumens: 1_500.0,
            range: 10.0,
            radius: 0.0,
            shadows_enabled: false,
        }
    }
}

/// A cone light whose axis is supplied by the entity Transform rotation.
#[derive(Debug, Clone, Copy, PartialEq, Component, Reflect, Prefab)]
#[require(az_transform::Transform)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("Spot Light")
    .in_group("Lighting")
    .with_icon("lightbulb")
    .with_description("Transform rotation supplies the axis of a finite photometric cone light."))]
#[prefab(tag = "SpotLight", version = 1)]
pub struct SpotLight {
    #[reflect(@EditorFieldAttributes::new("Color", EditorWidget::Color))]
    pub color: Vec4,

    #[reflect(@EditorFieldAttributes::new("Intensity", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0".to_owned()),
            maximum: None,
            step: Some("10".to_owned()),
            suffix: Some("lm".to_owned()),
        }))]
    pub intensity_lumens: f32,

    #[reflect(@EditorFieldAttributes::new("Range", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0".to_owned()),
            maximum: None,
            step: Some("0.1".to_owned()),
            suffix: Some("m".to_owned()),
        }))]
    pub range: f32,

    #[reflect(@EditorFieldAttributes::new("Radius", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0".to_owned()),
            maximum: None,
            step: Some("0.01".to_owned()),
            suffix: Some("m".to_owned()),
        }))]
    pub radius: f32,

    #[reflect(@EditorFieldAttributes::new("Inner Cone Angle", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0".to_owned()),
            maximum: Some("179".to_owned()),
            step: Some("0.1".to_owned()),
            suffix: Some("°".to_owned()),
        }))]
    pub inner_angle_degrees: f32,

    #[reflect(@EditorFieldAttributes::new("Outer Cone Angle", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0".to_owned()),
            maximum: Some("179".to_owned()),
            step: Some("0.1".to_owned()),
            suffix: Some("°".to_owned()),
        }))]
    pub outer_angle_degrees: f32,

    #[reflect(@EditorFieldAttributes::new("Cast Shadows", EditorWidget::Toggle))]
    pub shadows_enabled: bool,
}

impl Default for SpotLight {
    fn default() -> Self {
        Self {
            color: Vec4::ONE,
            intensity_lumens: 1_500.0,
            range: 10.0,
            radius: 0.0,
            inner_angle_degrees: 30.0,
            outer_angle_degrees: 45.0,
            shadows_enabled: false,
        }
    }
}

macro_rules! impl_light_component {
    ($ty:ty, $name:expr, $type_id:expr) => {
        impl AzTypeInfo for $ty {
            const NAME: &'static str = $name;
            const TYPE_ID: Uuid = $type_id;
        }

        impl AzRtti for $ty {
            const BASE_TYPE_IDS: &'static [Uuid] = &[az_core::component::COMPONENT_TYPE_ID];
        }

        impl AzComponent for $ty {}
    };
}

impl_light_component!(
    DirectionalLight,
    DIRECTIONAL_LIGHT_SCHEMA_NAME,
    DIRECTIONAL_LIGHT_COMPONENT_TYPE_ID
);
impl_light_component!(
    PointLight,
    POINT_LIGHT_SCHEMA_NAME,
    POINT_LIGHT_COMPONENT_TYPE_ID
);
impl_light_component!(
    SpotLight,
    SPOT_LIGHT_SCHEMA_NAME,
    SPOT_LIGHT_COMPONENT_TYPE_ID
);

const TRANSFORM_CAPABILITY: &str = "azoth.transform";
const LIGHT_CAPABILITY: &str = "azoth.light";

pub fn register_prefab_types(registry: &mut TypeRegistry) {
    registry.register::<Vec4>();
    registry.register::<DirectionalLight>();
    registry.register::<PointLight>();
    registry.register::<SpotLight>();

    for type_id in [
        TypeId::of::<DirectionalLight>(),
        TypeId::of::<PointLight>(),
        TypeId::of::<SpotLight>(),
    ] {
        registry
            .get_mut(type_id)
            .expect("a just-registered light must be present")
            .insert(ApplicabilityTypeData {
                evaluate: light_applicability,
                provides: &[LIGHT_CAPABILITY],
                requires: &[],
                incompatible: &[LIGHT_CAPABILITY],
            });
    }
    registry
        .get_mut(TypeId::of::<SpotLight>())
        .expect("a just-registered SpotLight must be present")
        .insert(ValidationTypeData {
            validate: validate_reflected_spot_light,
        });
}

fn validate_reflected_spot_light(
    value: &dyn PartialReflect,
) -> Result<Vec<ValidationDiagnostic>, ValidationCallbackError> {
    let light = value.try_downcast_ref::<SpotLight>().ok_or_else(|| {
        ValidationCallbackError::IncompatibleValue(value.reflect_type_path().to_owned())
    })?;
    let mut diagnostics = Vec::new();

    if !light.inner_angle_degrees.is_finite() || light.inner_angle_degrees < 0.0 {
        diagnostics.push(light_diagnostic(
            vec![field("inner_angle_degrees")],
            "spot_light.inner_angle.invalid",
            "inner cone angle must be finite and at least 0 degrees",
        ));
    }
    if !light.outer_angle_degrees.is_finite()
        || light.outer_angle_degrees < 0.0
        || light.outer_angle_degrees >= 180.0
    {
        diagnostics.push(light_diagnostic(
            vec![field("outer_angle_degrees")],
            "spot_light.outer_angle.invalid",
            "outer cone angle must be finite, at least 0 degrees, and less than 180 degrees",
        ));
    }
    if light.inner_angle_degrees.is_finite()
        && light.outer_angle_degrees.is_finite()
        && light.inner_angle_degrees >= 0.0
        && light.outer_angle_degrees < 180.0
        && light.inner_angle_degrees > light.outer_angle_degrees
    {
        diagnostics.push(light_diagnostic(
            vec![field("inner_angle_degrees")],
            "spot_light.inner_before_outer",
            "inner cone angle must be less than or equal to outer cone angle",
        ));
    }

    Ok(diagnostics)
}

// Signature is fixed by `ErasedApplicabilityFn`, the fn-pointer type this is
// stored as; unwrapping the `Result` would not coerce.
#[allow(clippy::unnecessary_wraps)]
fn light_applicability(
    context: &ApplicabilityContext,
) -> Result<ApplicabilityResult, az_core::EditorPolicyError> {
    let has_transform = context.capabilities.contains(TRANSFORM_CAPABILITY);
    let duplicate = context.capabilities.contains(LIGHT_CAPABILITY);
    let mut diagnostics = Vec::new();
    if !has_transform {
        diagnostics.push(light_diagnostic(
            Vec::new(),
            "light.requires_transform",
            "light requires a transform capability",
        ));
    }
    if duplicate {
        diagnostics.push(light_diagnostic(
            Vec::new(),
            "light.already_present",
            "the selection already has a light capability",
        ));
    }
    Ok(ApplicabilityResult {
        applicable: has_transform && !duplicate,
        diagnostics,
    })
}

fn field(name: &str) -> ReflectedPathSegment {
    ReflectedPathSegment::Field(name.to_owned())
}

fn light_diagnostic(
    path: Vec<ReflectedPathSegment>,
    code: &str,
    message: &str,
) -> ValidationDiagnostic {
    ValidationDiagnostic {
        path: ReflectedPath(path),
        severity: DiagnosticSeverity::Error,
        code: code.to_owned(),
        message: message.to_owned(),
    }
}
