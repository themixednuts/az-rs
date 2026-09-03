//! Project-host-only validation, edit policy, and applicability callbacks.

use std::collections::BTreeSet;

use bevy_reflect::PartialReflect;
use thiserror::Error;

/// A reflected named path. Numeric collection operations remain explicit edit
/// commands and do not get encoded as Rust method names.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReflectedPath(pub Vec<ReflectedPathSegment>);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReflectedPathSegment {
    Field(String),
    Variant(String),
    TupleIndex(u32),
    ListIndex(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// A diagnostic addressable by the editor without knowing Rust implementation
/// details in another process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationDiagnostic {
    pub path: ReflectedPath,
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
}

pub type ErasedValidationFn =
    fn(&dyn PartialReflect) -> Result<Vec<ValidationDiagnostic>, ValidationCallbackError>;

/// Cross-field validation registered on one Bevy type registration.
#[derive(Clone, Copy)]
pub struct ValidationTypeData {
    pub validate: ErasedValidationFn,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ValidationCallbackError {
    #[error("validation callback received an incompatible reflected value: {0}")]
    IncompatibleValue(String),
    #[error("validation callback failed: {0}")]
    Failed(String),
}

/// Opaque action identity. Its value is a protocol token, never a Rust method
/// name to be invoked by the editor process.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EditorActionId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorPathPolicy {
    pub path: ReflectedPath,
    pub visible: bool,
    pub read_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorPolicyResult {
    pub paths: Vec<EditorPathPolicy>,
    pub action_ids: Vec<EditorActionId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorChangeNotification {
    pub refresh_paths: Vec<ReflectedPath>,
    pub diagnostics_changed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorActionOutcome {
    pub changed_paths: Vec<ReflectedPath>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EditorPolicyError {
    #[error("editor policy received an incompatible reflected value: {0}")]
    IncompatibleValue(String),
    #[error("unknown editor action `{0}`")]
    UnknownAction(String),
    #[error("editor policy callback failed: {0}")]
    Failed(String),
}

pub type ErasedEditorPolicyFn =
    fn(&dyn PartialReflect) -> Result<EditorPolicyResult, EditorPolicyError>;
pub type ErasedEditorChangeNotifyFn =
    fn(&dyn PartialReflect, &ReflectedPath) -> Result<EditorChangeNotification, EditorPolicyError>;
pub type ErasedEditorActionFn =
    fn(&mut dyn PartialReflect, &EditorActionId) -> Result<EditorActionOutcome, EditorPolicyError>;

/// Dynamic editor behavior executed only by project-host.
#[derive(Clone, Copy, Default)]
pub struct EditorPolicyTypeData {
    pub evaluate: Option<ErasedEditorPolicyFn>,
    pub notify_change: Option<ErasedEditorChangeNotifyFn>,
    pub invoke_action: Option<ErasedEditorActionFn>,
}

/// Selection capabilities supplied to component applicability callbacks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplicabilityContext {
    pub selected_type_paths: BTreeSet<String>,
    pub capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicabilityResult {
    pub applicable: bool,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

pub type ErasedApplicabilityFn =
    fn(&ApplicabilityContext) -> Result<ApplicabilityResult, EditorPolicyError>;

/// Dynamic Add Component compatibility registered with the component type.
#[derive(Clone, Copy)]
pub struct ApplicabilityTypeData {
    pub evaluate: ErasedApplicabilityFn,
    /// Capabilities supplied when the component is present.
    pub provides: &'static [&'static str],
    /// Capabilities required independently of Bevy required components.
    pub requires: &'static [&'static str],
    /// Capabilities which make the component inapplicable.
    pub incompatible: &'static [&'static str],
}

#[cfg(test)]
mod tests {
    use bevy_reflect::{Reflect, TypeRegistry};

    use super::*;

    #[derive(Reflect)]
    struct Limits {
        near: f32,
        far: f32,
    }

    fn validate(
        value: &dyn PartialReflect,
    ) -> Result<Vec<ValidationDiagnostic>, ValidationCallbackError> {
        let value = value.try_downcast_ref::<Limits>().ok_or_else(|| {
            ValidationCallbackError::IncompatibleValue(value.reflect_type_path().to_owned())
        })?;
        Ok((value.near >= value.far)
            .then(|| ValidationDiagnostic {
                path: ReflectedPath(vec![ReflectedPathSegment::Field("near".to_owned())]),
                severity: DiagnosticSeverity::Error,
                code: "near_before_far".to_owned(),
                message: "near must be less than far".to_owned(),
            })
            .into_iter()
            .collect())
    }

    #[test]
    fn validation_callback_is_registered_as_bevy_type_data() {
        let mut registry = TypeRegistry::default();
        registry.register::<Limits>();
        registry
            .get_mut(std::any::TypeId::of::<Limits>())
            .expect("Limits registration")
            .insert(ValidationTypeData { validate });

        let data = registry
            .get(std::any::TypeId::of::<Limits>())
            .and_then(|registration| registration.data::<ValidationTypeData>())
            .expect("validation type data");
        let diagnostics = (data.validate)(&Limits {
            near: 2.0,
            far: 1.0,
        })
        .expect("validation result");
        assert_eq!(diagnostics[0].code, "near_before_far");
    }
}
