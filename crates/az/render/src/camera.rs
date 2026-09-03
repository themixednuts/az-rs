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
use uuid::Uuid;

use crate::generated;

pub const CAMERA_SCHEMA_NAME: &str = "azoth.render.Camera";
pub const CAMERA_PROJECTION_SCHEMA_NAME: &str = "azoth.render.CameraProjection";
pub const PERSPECTIVE_PROJECTION_SCHEMA_NAME: &str = "azoth.render.PerspectiveCameraProjection";
pub const ORTHOGRAPHIC_PROJECTION_SCHEMA_NAME: &str = "azoth.render.OrthographicCameraProjection";
pub const CAMERA_COMPONENT_TYPE_ID: Uuid = generated::CAMERA_COMPONENT_TYPE_ID;

#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(Default)]
#[reflect(@EditorTypeAttributes::labeled("Perspective").in_group("Rendering"))]
pub struct PerspectiveCameraProjection {
    #[reflect(@EditorFieldAttributes::new("Vertical FOV", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("1".to_owned()),
            maximum: Some("179".to_owned()),
            step: Some("0.1".to_owned()),
            suffix: Some("°".to_owned()),
        }))]
    pub fov_y_degrees: f32,
}

impl Default for PerspectiveCameraProjection {
    fn default() -> Self {
        Self {
            fov_y_degrees: 60.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(Default)]
#[reflect(@EditorTypeAttributes::labeled("Orthographic").in_group("Rendering"))]
pub struct OrthographicCameraProjection {
    #[reflect(@EditorFieldAttributes::new("Half Height", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0.001".to_owned()),
            maximum: None,
            step: Some("0.1".to_owned()),
            suffix: Some("m".to_owned()),
        }))]
    pub half_height: f32,
}

impl Default for OrthographicCameraProjection {
    fn default() -> Self {
        Self { half_height: 5.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(Default)]
#[reflect(@EditorTypeAttributes::labeled("Projection").in_group("Rendering"))]
pub enum CameraProjection {
    Perspective(PerspectiveCameraProjection),
    Orthographic(OrthographicCameraProjection),
}

impl Default for CameraProjection {
    fn default() -> Self {
        Self::Perspective(PerspectiveCameraProjection::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Component, Reflect, Prefab)]
#[require(az_transform::Transform)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("Camera")
    .in_group("Rendering")
    .with_icon("video")
    .with_description("A Transform-oriented 3D camera with a native Bevy projection."))]
#[prefab(tag = "Camera", version = 1)]
pub struct Camera {
    #[reflect(@EditorFieldAttributes::new("Projection", EditorWidget::Default))]
    pub projection: CameraProjection,

    #[reflect(@EditorFieldAttributes::new("Near Clip", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0.001".to_owned()),
            maximum: None,
            step: Some("0.01".to_owned()),
            suffix: Some("m".to_owned()),
        }))]
    pub near: f32,

    #[reflect(@EditorFieldAttributes::new("Far Clip", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: Some("0.002".to_owned()),
            maximum: None,
            step: Some("1".to_owned()),
            suffix: Some("m".to_owned()),
        }))]
    pub far: f32,

    #[reflect(@EditorFieldAttributes::new("Active", EditorWidget::Toggle))]
    pub active: bool,

    #[reflect(@EditorFieldAttributes::new("Render Order", EditorWidget::Number)
        .with_range(EditorNumericRange {
            minimum: None,
            maximum: None,
            step: Some("1".to_owned()),
            suffix: None,
        }))]
    pub order: i32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            projection: CameraProjection::default(),
            near: 0.1,
            far: 1_000.0,
            active: true,
            order: 0,
        }
    }
}

impl AzTypeInfo for Camera {
    const NAME: &'static str = CAMERA_SCHEMA_NAME;
    const TYPE_ID: Uuid = CAMERA_COMPONENT_TYPE_ID;
}

impl AzRtti for Camera {
    const BASE_TYPE_IDS: &'static [Uuid] = &[az_core::component::COMPONENT_TYPE_ID];
}

impl AzComponent for Camera {}

pub fn register_prefab_types(registry: &mut TypeRegistry) {
    registry.register::<PerspectiveCameraProjection>();
    registry.register::<OrthographicCameraProjection>();
    registry.register::<CameraProjection>();
    registry.register::<Camera>();
    let registration = registry
        .get_mut(TypeId::of::<Camera>())
        .expect("a just-registered Camera must be present");
    registration.insert(ValidationTypeData {
        validate: validate_reflected_camera,
    });
    registration.insert(ApplicabilityTypeData {
        evaluate: camera_applicability,
        provides: &["azoth.camera"],
        requires: &[],
        incompatible: &["azoth.camera"],
    });
}

fn validate_reflected_camera(
    value: &dyn PartialReflect,
) -> Result<Vec<ValidationDiagnostic>, ValidationCallbackError> {
    let camera = value.try_downcast_ref::<Camera>().ok_or_else(|| {
        ValidationCallbackError::IncompatibleValue(value.reflect_type_path().to_owned())
    })?;
    let mut diagnostics = Vec::new();

    if !camera.near.is_finite() || camera.near < 0.001 {
        diagnostics.push(camera_diagnostic(
            vec![field("near")],
            "camera.near.invalid",
            "near clip must be finite and at least 0.001",
        ));
    }
    if !camera.far.is_finite() || camera.far < 0.002 {
        diagnostics.push(camera_diagnostic(
            vec![field("far")],
            "camera.far.invalid",
            "far clip must be finite and at least 0.002",
        ));
    }
    if camera.near.is_finite() && camera.far.is_finite() && camera.near >= camera.far {
        diagnostics.push(camera_diagnostic(
            vec![field("near")],
            "camera.near_before_far",
            "near clip must be less than far clip",
        ));
    }

    match camera.projection {
        CameraProjection::Perspective(projection) => {
            if !projection.fov_y_degrees.is_finite()
                || !(1.0..=179.0).contains(&projection.fov_y_degrees)
            {
                diagnostics.push(camera_diagnostic(
                    vec![
                        field("projection"),
                        ReflectedPathSegment::Variant("Perspective".to_owned()),
                        ReflectedPathSegment::TupleIndex(0),
                        field("fov_y_degrees"),
                    ],
                    "camera.projection.perspective.fov",
                    "vertical FOV must be finite and within 1..=179 degrees",
                ));
            }
        }
        CameraProjection::Orthographic(projection) => {
            if !projection.half_height.is_finite() || projection.half_height < 0.001 {
                diagnostics.push(camera_diagnostic(
                    vec![
                        field("projection"),
                        ReflectedPathSegment::Variant("Orthographic".to_owned()),
                        ReflectedPathSegment::TupleIndex(0),
                        field("half_height"),
                    ],
                    "camera.projection.orthographic.half_height",
                    "orthographic half height must be finite and at least 0.001",
                ));
            }
        }
    }

    Ok(diagnostics)
}

// Signature is fixed by `ErasedApplicabilityFn`, the fn-pointer type this is
// stored as; unwrapping the `Result` would not coerce.
#[allow(clippy::unnecessary_wraps)]
fn camera_applicability(
    context: &ApplicabilityContext,
) -> Result<ApplicabilityResult, az_core::EditorPolicyError> {
    const TRANSFORM: &str = "azoth.transform";
    const CAMERA: &str = "azoth.camera";

    let has_transform = context.capabilities.contains(TRANSFORM);
    let duplicate = context.capabilities.contains(CAMERA);
    let mut diagnostics = Vec::new();
    if !has_transform {
        diagnostics.push(camera_diagnostic(
            Vec::new(),
            "camera.requires_transform",
            "camera requires a transform capability",
        ));
    }
    if duplicate {
        diagnostics.push(camera_diagnostic(
            Vec::new(),
            "camera.already_present",
            "the selection already has a camera capability",
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

fn camera_diagnostic(
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
