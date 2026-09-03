//! UI-neutral reflected inspector projection for the ADR-0022 vNext contract.
//!
//! This module consumes only the process-safe registry/value/path vocabulary.
//! It does not know about renderer layout or the legacy authored schema model.

use std::{collections::BTreeMap, fmt};

use az_core::reflect::{ReflectedValueEncoding, ReflectedValueEnvelope};
use az_proto_project::vnext::{
    DiagnosticSeverity, FieldConstraints, NumericRange, PrefabComponentSnapshot, PrefabDiagnostic,
    PrefabEditCommand, PrefabOverrideOperation, PrefabOverrideSnapshot, PrefabValueTarget,
    ReflectedFieldDescriptor, ReflectedPath, ReflectedPathSegment, ReflectedTypeDescriptor,
    ReflectedTypeKind, TypeRegistrySnapshot,
};
use ron::value::RawValue;
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use thiserror::Error;

/// One inspector-ready component projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedInspectionModel {
    pub schema_catalog_hash: Vec<u8>,
    pub entity_alias: String,
    pub type_path: String,
    pub type_label: String,
    pub category: Option<String>,
    pub icon: Option<String>,
    pub description: Option<String>,
    pub fields: Vec<ReflectedInspectionField>,
    pub actions: Vec<String>,
    pub validation: ReflectedValidationState,
    pub add_component: ReflectedAddComponent,
}

/// Stable identity of an entity inside one typed Prefab source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedPrefabSelection {
    pub source_path: String,
    pub entity_alias: String,
}

impl ReflectedPrefabSelection {
    #[must_use]
    pub fn new(source_path: impl Into<String>, entity_alias: impl Into<String>) -> Self {
        Self {
            source_path: source_path.into(),
            entity_alias: entity_alias.into(),
        }
    }
}

/// One selected component and its renderer-neutral reflected projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedComponentInspection {
    pub component: PrefabComponentSnapshot,
    pub model: ReflectedInspectionModel,
}

/// Complete inspection state for one selected Prefab entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedEntityInspection {
    pub selection: ReflectedPrefabSelection,
    pub registry_schema_catalog_hash: Vec<u8>,
    pub document_version: u32,
    pub type_versions: BTreeMap<String, u32>,
    pub revision: u64,
    pub components: Vec<ReflectedComponentInspection>,
    pub overrides: Vec<ReflectedOverrideOperation>,
    pub diagnostics: Vec<PrefabDiagnostic>,
}

impl ReflectedEntityInspection {
    #[must_use]
    pub fn component(&self, type_path: &str) -> Option<&ReflectedComponentInspection> {
        self.components
            .iter()
            .find(|component| component.component.type_path == type_path)
    }
}

/// Inputs projected by project-host for one component inspection.
#[derive(Debug, Clone, Copy)]
pub struct ReflectedInspectionInput<'a> {
    pub registry: &'a TypeRegistrySnapshot,
    pub component: &'a PrefabComponentSnapshot,
    pub reflected_default: Option<&'a ReflectedValueEnvelope>,
    pub diagnostics: &'a [PrefabDiagnostic],
    pub add_component_evaluation: Option<&'a AddComponentEvaluation>,
    pub add_component_capabilities: Option<&'a AddComponentCapabilities>,
}

impl<'a> ReflectedInspectionInput<'a> {
    #[must_use]
    pub const fn new(
        registry: &'a TypeRegistrySnapshot,
        component: &'a PrefabComponentSnapshot,
    ) -> Self {
        Self {
            registry,
            component,
            reflected_default: None,
            diagnostics: &[],
            add_component_evaluation: None,
            add_component_capabilities: None,
        }
    }

    #[must_use]
    pub const fn with_default(mut self, value: &'a ReflectedValueEnvelope) -> Self {
        self.reflected_default = Some(value);
        self
    }

    #[must_use]
    pub const fn with_diagnostics(mut self, diagnostics: &'a [PrefabDiagnostic]) -> Self {
        self.diagnostics = diagnostics;
        self
    }
}

impl ReflectedInspectionModel {
    /// Projects one component envelope into recursive inspector data.
    ///
    /// # Errors
    ///
    /// Returns an error when a referenced type is absent from the registry or
    /// a typed-RON envelope does not match its reflected structure.
    pub fn project(input: ReflectedInspectionInput<'_>) -> Result<Self, ReflectedProjectionError> {
        let descriptor = descriptor(input.registry, &input.component.type_path)?;
        let current = decode_envelope(input.registry, &input.component.sparse_value)?;
        let authored_default = input
            .reflected_default
            .map(|value| decode_envelope(input.registry, value))
            .transpose()?;
        let reflected_default = descriptor
            .reflected_default
            .as_ref()
            .map(|value| decode_envelope(input.registry, value))
            .transpose()?;
        let root_target = PrefabValueTarget {
            instance_alias_chain: Vec::new(),
            entity_alias: input.component.entity_alias.clone(),
            path: ReflectedPath {
                component_type_path: descriptor.type_path.clone(),
                segments: Vec::new(),
            },
        };
        let root_binding = ReflectedEditBinding::new(root_target);
        let current_fields = struct_fields(&current);
        let authored_default_fields = authored_default
            .as_ref()
            .map(struct_fields)
            .unwrap_or_default();
        let reflected_default_fields = reflected_default
            .as_ref()
            .map(struct_fields)
            .unwrap_or_default();
        let default_supported = descriptor.applicability.default_available;
        let fields = descriptor
            .fields
            .iter()
            .map(|field| {
                project_field(
                    input.registry,
                    field,
                    current_fields.get(field.name.as_str()).copied(),
                    authored_default_fields
                        .get(field.name.as_str())
                        .or_else(|| reflected_default_fields.get(field.name.as_str()))
                        .copied(),
                    default_supported,
                    &root_binding,
                    input.diagnostics,
                    false,
                    false,
                    0,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let attributes = &descriptor.editor_attributes;

        Ok(Self {
            schema_catalog_hash: input.registry.schema_catalog_hash.clone(),
            entity_alias: input.component.entity_alias.clone(),
            type_path: descriptor.type_path.clone(),
            type_label: attributes
                .label
                .clone()
                .unwrap_or_else(|| descriptor.short_path.clone()),
            category: attributes.category.clone(),
            icon: attributes.icon.clone(),
            description: attributes.description.clone(),
            fields,
            actions: attributes.action_ids.clone(),
            validation: validation_for(input.diagnostics, &root_binding.target),
            add_component: ReflectedAddComponent {
                editor_export: has_flag(descriptor, "ReflectComponent"),
                runtime_export: has_flag(descriptor, "Prefab")
                    && has_flag(descriptor, "ReflectComponent"),
                default_available: descriptor.applicability.default_available,
                evaluation: input.add_component_evaluation.cloned().map_or(
                    AddComponentEvaluationState::NotProjected,
                    AddComponentEvaluationState::Projected,
                ),
                capabilities: input
                    .add_component_capabilities
                    .cloned()
                    .unwrap_or_else(|| AddComponentCapabilities::Projected {
                        provides: descriptor.applicability.provides.clone(),
                        requires: descriptor.applicability.requires.clone(),
                        incompatible: descriptor.applicability.incompatible.clone(),
                    }),
            },
        })
    }
}

/// Presentation and edit data for one named reflected field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedInspectionField {
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    pub read_only: bool,
    pub hidden: bool,
    pub actions: Vec<String>,
    pub widget: WidgetSpec,
    pub validation: ReflectedValidationState,
    pub value: ReflectedValueNode,
}

/// A recursively inspectable reflected value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedValueNode {
    pub type_path: String,
    pub kind: ReflectedTypeKind,
    pub current: ReflectedCurrentValue,
    pub default: ReflectedDefaultValue,
    pub binding: ReflectedEditBinding,
    pub children: Vec<ReflectedInspectionChild>,
}

/// Authored and materialized views of a sparse field value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedCurrentValue {
    pub authored: Option<ReflectedValue>,
    pub effective: Option<ReflectedValue>,
}

/// Default projection state for a reflected field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedDefaultValue {
    pub availability: ReflectedDefaultAvailability,
    pub value: Option<ReflectedValue>,
}

/// Distinguishes an absent default from a supported value omitted by an older peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflectedDefaultAvailability {
    Projected,
    SupportedButNotProjected,
    Unavailable,
}

/// Recursive children selected from reflected structure and current values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectedInspectionChild {
    Field(Box<ReflectedInspectionField>),
    TupleElement {
        index: u32,
        value: Box<ReflectedValueNode>,
    },
    ListItem(ReflectedListItem),
    /// Boxed: inline this variant is 272 bytes against a 48-byte runner-up, so
    /// every child in a reflected tree paid for the map case.
    MapEntry(Box<ReflectedMapEntry>),
    Variant(ReflectedVariantSelection),
    OptionalSome(Box<ReflectedValueNode>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedListItem {
    pub index: u32,
    pub value: Box<ReflectedValueNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedMapEntry {
    pub key: ReflectedValue,
    pub value: Box<ReflectedValueNode>,
    pub binding: ReflectedMapEntryBinding,
    pub value_envelope: ReflectedValueEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedVariantSelection {
    pub name: String,
    pub fields: Vec<ReflectedInspectionChild>,
}

/// Transport-neutral value tree decoded from an envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectedValue {
    Scalar(ReflectedScalar),
    Struct(Vec<(String, Self)>),
    Tuple(Vec<Self>),
    List(Vec<Self>),
    Map(Vec<ReflectedMapValueEntry>),
    /// A selected variant and the fields its sparse value retains, each paired
    /// with the name the variant descriptor declares for it — exactly the
    /// pairing [`ReflectedValue::Struct`] carries. A sparse value may retain any
    /// subset of a struct-shaped variant's fields, so position within this list
    /// says nothing about which declared field a value belongs to; only the name
    /// does. A tuple-shaped variant's declared names are its indices (`"0"`,
    /// `"1"`, ...), so the pairing is uniform across variant shapes.
    Enum {
        variant: String,
        fields: Vec<(String, Self)>,
    },
    Optional(Option<Box<Self>>),
    Unit,
    OpaqueRon(String),
    Encoded(ReflectedValueEnvelope),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedMapValueEntry {
    pub key: ReflectedValue,
    pub value: ReflectedValue,
    pub key_envelope: ReflectedValueEnvelope,
    pub value_envelope: ReflectedValueEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectedScalar {
    Bool(bool),
    Signed(String),
    Unsigned(String),
    Float(String),
    String(String),
}

/// Decodes a standalone reflected envelope for controls that do not have a
/// complete registry projection, such as visual-graph ports.
///
/// Scalar Bevy type paths retain their typed widget semantics. Composite or
/// unknown values remain opaque RON until a registry descriptor is available.
///
/// # Errors
///
/// Returns [`ReflectedProjectionError::InvalidUtf8`] if a `TypedRon` payload
/// is not valid UTF-8, or [`ReflectedProjectionError::Decode`] if it is not
/// parseable RON. Envelopes in any other encoding are returned unchanged and
/// cannot fail.
pub fn decode_standalone_reflected_value(
    envelope: &ReflectedValueEnvelope,
) -> Result<ReflectedValue, ReflectedProjectionError> {
    if envelope.encoding != ReflectedValueEncoding::TypedRon {
        return Ok(ReflectedValue::Encoded(envelope.clone()));
    }
    let source = std::str::from_utf8(&envelope.payload).map_err(|error| {
        ReflectedProjectionError::InvalidUtf8 {
            type_path: envelope.type_path.clone(),
            message: error.to_string(),
        }
    })?;
    let raw = RawValue::from_ron(source).map_err(|error| ReflectedProjectionError::Decode {
        type_path: envelope.type_path.clone(),
        message: error.to_string(),
    })?;
    let short_path = envelope
        .type_path
        .rsplit("::")
        .next()
        .unwrap_or(&envelope.type_path);
    let decode_error = |error: ron::error::SpannedError| ReflectedProjectionError::Decode {
        type_path: envelope.type_path.clone(),
        message: error.to_string(),
    };

    match short_path {
        "bool" => raw
            .into_rust::<bool>()
            .map(ReflectedScalar::Bool)
            .map(ReflectedValue::Scalar)
            .map_err(decode_error),
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" => Ok(ReflectedValue::Scalar(
            ReflectedScalar::Signed(trim_numeric_suffix(raw.get_ron(), short_path)),
        )),
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" => Ok(ReflectedValue::Scalar(
            ReflectedScalar::Unsigned(trim_numeric_suffix(raw.get_ron(), short_path)),
        )),
        "f32" | "f64" => Ok(ReflectedValue::Scalar(ReflectedScalar::Float(
            trim_float_suffix(raw.get_ron()),
        ))),
        "String" | "str" => raw
            .into_rust::<String>()
            .map(ReflectedScalar::String)
            .map(ReflectedValue::Scalar)
            .map_err(decode_error),
        _ => decode_opaque(raw),
    }
}

/// Selects the same renderer family used by reflected inspector fields for a
/// standalone value projection.
#[must_use]
pub fn standalone_reflected_widget_family(value: &ReflectedValue) -> WidgetFamily {
    match value {
        ReflectedValue::Scalar(ReflectedScalar::Bool(_)) => WidgetFamily::Bool,
        ReflectedValue::Scalar(
            ReflectedScalar::Signed(_) | ReflectedScalar::Unsigned(_) | ReflectedScalar::Float(_),
        ) => WidgetFamily::Number,
        ReflectedValue::Scalar(ReflectedScalar::String(_)) => WidgetFamily::Text,
        ReflectedValue::Tuple(values)
            if (2..=4).contains(&values.len())
                && values.iter().all(|value| {
                    matches!(
                        value,
                        ReflectedValue::Scalar(
                            ReflectedScalar::Signed(_)
                                | ReflectedScalar::Unsigned(_)
                                | ReflectedScalar::Float(_)
                        )
                    )
                }) =>
        {
            // The guard restricts the arity to 2..=4, so this conversion always
            // succeeds; saturating keeps it total without an unreachable panic.
            WidgetFamily::Vector {
                dimensions: u8::try_from(values.len()).unwrap_or(u8::MAX),
            }
        }
        ReflectedValue::Struct(_) | ReflectedValue::Tuple(_) => WidgetFamily::Struct,
        ReflectedValue::List(_) => WidgetFamily::List,
        ReflectedValue::Map(_) => WidgetFamily::Map,
        ReflectedValue::Enum { .. } => WidgetFamily::Enum,
        ReflectedValue::Optional(_) => WidgetFamily::Optional,
        ReflectedValue::Unit | ReflectedValue::OpaqueRon(_) | ReflectedValue::Encoded(_) => {
            WidgetFamily::Opaque
        }
    }
}

/// Renderer-family selection independent of GPUI implementation details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetSpec {
    pub family: WidgetFamily,
    pub range: Option<NumericRange>,
    pub rows: Option<u32>,
    pub constraints: FieldConstraints,
    pub variants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WidgetFamily {
    Number,
    Slider,
    Vector { dimensions: u8 },
    Quaternion,
    Enum,
    Color,
    Asset { asset_type: String },
    Object { object_type: String },
    Multiline,
    Bool,
    Text,
    Struct,
    List,
    Map,
    Optional,
    Opaque,
}

/// Structural renderer family for a reflected type kind, before editor
/// attributes or type-path refinements are applied.
impl From<&ReflectedTypeKind> for WidgetFamily {
    fn from(kind: &ReflectedTypeKind) -> Self {
        match kind {
            ReflectedTypeKind::Bool => Self::Bool,
            ReflectedTypeKind::SignedInteger { .. }
            | ReflectedTypeKind::UnsignedInteger { .. }
            | ReflectedTypeKind::Float { .. } => Self::Number,
            ReflectedTypeKind::String => Self::Text,
            ReflectedTypeKind::Struct
            | ReflectedTypeKind::Tuple
            | ReflectedTypeKind::TupleStruct => Self::Struct,
            ReflectedTypeKind::List | ReflectedTypeKind::Array { .. } => Self::List,
            ReflectedTypeKind::Map => Self::Map,
            ReflectedTypeKind::Enum => Self::Enum,
            ReflectedTypeKind::Optional => Self::Optional,
            ReflectedTypeKind::Set | ReflectedTypeKind::Opaque => Self::Opaque,
        }
    }
}

/// Validation diagnostics relevant to one reflected path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReflectedValidationState {
    pub diagnostics: Vec<PrefabDiagnostic>,
}

impl ReflectedValidationState {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    }
}

/// Static Add Component facts available in the registry projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedAddComponent {
    pub editor_export: bool,
    pub runtime_export: bool,
    pub default_available: bool,
    pub evaluation: AddComponentEvaluationState,
    pub capabilities: AddComponentCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddComponentEvaluationState {
    Projected(AddComponentEvaluation),
    NotProjected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddComponentEvaluation {
    pub applicable: bool,
    pub diagnostics: Vec<PrefabDiagnostic>,
}

/// Capability lists are projected by the neutral server contract. `NotProjected`
/// remains available when reading an older peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddComponentCapabilities {
    Projected {
        provides: Vec<String>,
        requires: Vec<String>,
        incompatible: Vec<String>,
    },
    NotProjected,
}

/// UI-neutral override operation projected from a Prefab snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectedOverrideOperation {
    Set {
        target: PrefabValueTarget,
        value: ReflectedValueEnvelope,
    },
    Clear {
        target: PrefabValueTarget,
    },
    Insert {
        target: PrefabValueTarget,
        index: u32,
        value: ReflectedValueEnvelope,
    },
    Remove {
        target: PrefabValueTarget,
        index: u32,
    },
    Move {
        target: PrefabValueTarget,
        from: u32,
        to: u32,
    },
}

impl ReflectedOverrideOperation {
    #[must_use]
    pub fn project(snapshot: &PrefabOverrideSnapshot) -> Self {
        match &snapshot.operation {
            PrefabOverrideOperation::Set { target, value } => Self::Set {
                target: target.clone(),
                value: value.clone(),
            },
            PrefabOverrideOperation::Clear { target } => Self::Clear {
                target: target.clone(),
            },
            PrefabOverrideOperation::Insert {
                target,
                index,
                value,
            } => Self::Insert {
                target: target.clone(),
                index: *index,
                value: value.clone(),
            },
            PrefabOverrideOperation::Remove { target, index } => Self::Remove {
                target: target.clone(),
                index: *index,
            },
            PrefabOverrideOperation::Move { target, from, to } => Self::Move {
                target: target.clone(),
                from: *from,
                to: *to,
            },
        }
    }

    #[must_use]
    pub fn edit_command(&self) -> PrefabEditCommand {
        match self {
            Self::Set { target, value } => PrefabEditCommand::SetOverride {
                target: target.clone(),
                value: value.clone(),
            },
            Self::Clear { target } => PrefabEditCommand::ClearOverride {
                target: target.clone(),
            },
            Self::Insert {
                target,
                index,
                value,
            } => PrefabEditCommand::InsertOverride {
                target: target.clone(),
                index: *index,
                value: value.clone(),
            },
            Self::Remove { target, index } => PrefabEditCommand::RemoveOverrideItem {
                target: target.clone(),
                index: *index,
            },
            Self::Move { target, from, to } => PrefabEditCommand::MoveOverride {
                target: target.clone(),
                from: *from,
                to: *to,
            },
        }
    }
}

/// Named-path field binding used by 4b-2 command generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedEditBinding {
    pub target: PrefabValueTarget,
}

impl ReflectedEditBinding {
    #[must_use]
    pub const fn new(target: PrefabValueTarget) -> Self {
        Self { target }
    }

    #[must_use]
    pub fn field(&self, name: impl Into<String>) -> Self {
        self.segment(ReflectedPathSegment::Field(name.into()))
    }

    #[must_use]
    pub fn variant(&self, name: impl Into<String>) -> Self {
        self.segment(ReflectedPathSegment::Variant(name.into()))
    }

    #[must_use]
    pub fn tuple_index(&self, index: u32) -> Self {
        self.segment(ReflectedPathSegment::TupleIndex(index))
    }

    #[must_use]
    pub fn list_index(&self, index: u32) -> Self {
        self.segment(ReflectedPathSegment::ListIndex(index))
    }

    #[must_use]
    pub fn set_value(&self, value: ReflectedValueEnvelope) -> PrefabEditCommand {
        PrefabEditCommand::SetValue {
            target: self.target.clone(),
            value,
        }
    }

    #[must_use]
    pub fn list_insert(&self, index: u32, value: ReflectedValueEnvelope) -> PrefabEditCommand {
        PrefabEditCommand::ListInsert {
            target: self.target.clone(),
            index,
            value,
        }
    }

    #[must_use]
    pub fn list_remove(&self, index: u32) -> PrefabEditCommand {
        PrefabEditCommand::ListRemove {
            target: self.target.clone(),
            index,
        }
    }

    #[must_use]
    pub fn list_move(&self, from: u32, to: u32) -> PrefabEditCommand {
        PrefabEditCommand::ListMove {
            target: self.target.clone(),
            from,
            to,
        }
    }

    #[must_use]
    pub fn map_insert(
        &self,
        key: ReflectedValueEnvelope,
        value: ReflectedValueEnvelope,
    ) -> PrefabEditCommand {
        PrefabEditCommand::MapInsert {
            target: self.target.clone(),
            key,
            value,
        }
    }

    #[must_use]
    pub fn map_remove(&self, key: ReflectedValueEnvelope) -> PrefabEditCommand {
        PrefabEditCommand::MapRemove {
            target: self.target.clone(),
            key,
        }
    }

    #[must_use]
    pub fn set_variant(
        &self,
        variant_name: impl Into<String>,
        value: Option<ReflectedValueEnvelope>,
    ) -> PrefabEditCommand {
        PrefabEditCommand::SetVariant {
            target: self.target.clone(),
            variant_name: variant_name.into(),
            value,
        }
    }

    /// Removes the authored override at this reflected path.
    #[must_use]
    pub fn remove_override(&self) -> PrefabEditCommand {
        PrefabEditCommand::RemoveOverride {
            target: self.target.clone(),
        }
    }

    fn segment(&self, segment: ReflectedPathSegment) -> Self {
        let mut target = self.target.clone();
        target.path.segments.push(segment);
        Self { target }
    }
}

/// Map values are replaced through `MapInsert` because reflected paths do not
/// carry typed map keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedMapEntryBinding {
    pub map: ReflectedEditBinding,
    pub key: ReflectedValueEnvelope,
}

impl ReflectedMapEntryBinding {
    #[must_use]
    pub fn set_value(&self, value: ReflectedValueEnvelope) -> PrefabEditCommand {
        self.map.map_insert(self.key.clone(), value)
    }

    #[must_use]
    pub fn remove(&self) -> PrefabEditCommand {
        self.map.map_remove(self.key.clone())
    }
}

#[derive(Debug, Error)]
pub enum ReflectedProjectionError {
    #[error("registry projection does not contain reflected type `{0}`")]
    MissingType(String),
    #[error("typed reflected envelope for `{type_path}` is not UTF-8: {message}")]
    InvalidUtf8 { type_path: String, message: String },
    #[error("cannot decode reflected value `{type_path}`: {message}")]
    Decode { type_path: String, message: String },
    #[error("reflected inspector recursion exceeded the depth limit at `{0}`")]
    RecursionLimit(String),
}

#[allow(clippy::too_many_arguments)]
fn project_field(
    registry: &TypeRegistrySnapshot,
    field: &ReflectedFieldDescriptor,
    current: Option<&ReflectedValue>,
    default: Option<&ReflectedValue>,
    default_supported: bool,
    parent_binding: &ReflectedEditBinding,
    diagnostics: &[PrefabDiagnostic],
    parent_read_only: bool,
    parent_hidden: bool,
    depth: usize,
) -> Result<ReflectedInspectionField, ReflectedProjectionError> {
    let binding = parent_binding.field(field.name.clone());
    let read_only = parent_read_only || field.editor_attributes.read_only;
    let hidden = parent_hidden || field.editor_attributes.hidden;
    Ok(ReflectedInspectionField {
        name: field.name.clone(),
        label: field
            .editor_attributes
            .label
            .clone()
            .unwrap_or_else(|| humanize(&field.name)),
        description: field.editor_attributes.description.clone(),
        read_only,
        hidden,
        actions: field.editor_attributes.action_ids.clone(),
        widget: widget_spec(registry, field),
        validation: validation_for(diagnostics, &binding.target),
        value: project_node(
            registry,
            &field.type_path,
            current,
            default,
            default_supported,
            binding,
            diagnostics,
            read_only,
            hidden,
            depth + 1,
        )?,
    })
}

#[allow(clippy::too_many_arguments)]
fn project_node(
    registry: &TypeRegistrySnapshot,
    type_path: &str,
    current: Option<&ReflectedValue>,
    default: Option<&ReflectedValue>,
    default_supported: bool,
    binding: ReflectedEditBinding,
    diagnostics: &[PrefabDiagnostic],
    read_only: bool,
    hidden: bool,
    depth: usize,
) -> Result<ReflectedValueNode, ReflectedProjectionError> {
    if depth > 32 {
        return Err(ReflectedProjectionError::RecursionLimit(
            type_path.to_owned(),
        ));
    }
    let descriptor = descriptor(registry, type_path)?;
    let effective = current.or(default).cloned();
    let availability = if default.is_some() {
        ReflectedDefaultAvailability::Projected
    } else if default_supported || has_flag(descriptor, "ReflectDefault") {
        ReflectedDefaultAvailability::SupportedButNotProjected
    } else {
        ReflectedDefaultAvailability::Unavailable
    };
    let children = project_children(
        registry,
        descriptor,
        current,
        default,
        default_supported,
        &binding,
        diagnostics,
        read_only,
        hidden,
        depth,
    )?;
    Ok(ReflectedValueNode {
        type_path: type_path.to_owned(),
        kind: descriptor.kind.clone(),
        current: ReflectedCurrentValue {
            authored: current.cloned(),
            effective,
        },
        default: ReflectedDefaultValue {
            availability,
            value: default.cloned(),
        },
        binding,
        children,
    })
}

#[allow(clippy::too_many_arguments)]
fn project_children(
    registry: &TypeRegistrySnapshot,
    descriptor: &ReflectedTypeDescriptor,
    current: Option<&ReflectedValue>,
    default: Option<&ReflectedValue>,
    default_supported: bool,
    binding: &ReflectedEditBinding,
    diagnostics: &[PrefabDiagnostic],
    read_only: bool,
    hidden: bool,
    depth: usize,
) -> Result<Vec<ReflectedInspectionChild>, ReflectedProjectionError> {
    let context = ChildProjection {
        registry,
        descriptor,
        binding,
        diagnostics,
        read_only,
        hidden,
        depth,
    };
    match descriptor.kind {
        ReflectedTypeKind::Struct => {
            project_struct_children(context, current, default, default_supported)
        }
        ReflectedTypeKind::Tuple | ReflectedTypeKind::TupleStruct => {
            project_tuple_children(context, current, default, default_supported)
        }
        ReflectedTypeKind::List | ReflectedTypeKind::Array { .. } => {
            project_list_children(context, current, default)
        }
        ReflectedTypeKind::Map => project_map_children(context, current, default),
        ReflectedTypeKind::Enum => project_enum_children(
            registry,
            descriptor,
            current.or(default),
            default,
            binding,
            diagnostics,
            read_only,
            hidden,
            depth,
        ),
        ReflectedTypeKind::Optional => project_optional_children(context, current, default),
        _ => Ok(Vec::new()),
    }
}

/// Everything a child projection needs except the value pair being projected.
///
/// [`project_children`] threads this through the per-kind helpers so each one
/// keeps a short signature instead of repeating ten positional parameters.
#[derive(Clone, Copy)]
struct ChildProjection<'a> {
    registry: &'a TypeRegistrySnapshot,
    descriptor: &'a ReflectedTypeDescriptor,
    binding: &'a ReflectedEditBinding,
    diagnostics: &'a [PrefabDiagnostic],
    read_only: bool,
    hidden: bool,
    depth: usize,
}

fn project_struct_children(
    context: ChildProjection<'_>,
    current: Option<&ReflectedValue>,
    default: Option<&ReflectedValue>,
    default_supported: bool,
) -> Result<Vec<ReflectedInspectionChild>, ReflectedProjectionError> {
    let current = current.map(struct_fields).unwrap_or_default();
    let default = default.map(struct_fields).unwrap_or_default();
    context
        .descriptor
        .fields
        .iter()
        .map(|field| {
            project_field(
                context.registry,
                field,
                current.get(field.name.as_str()).copied(),
                default.get(field.name.as_str()).copied(),
                default_supported,
                context.binding,
                context.diagnostics,
                context.read_only,
                context.hidden,
                context.depth,
            )
            .map(Box::new)
            .map(ReflectedInspectionChild::Field)
        })
        .collect()
}

fn project_tuple_children(
    context: ChildProjection<'_>,
    current: Option<&ReflectedValue>,
    default: Option<&ReflectedValue>,
    default_supported: bool,
) -> Result<Vec<ReflectedInspectionChild>, ReflectedProjectionError> {
    let current = tuple_values(current);
    let default = tuple_values(default);
    context
        .descriptor
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let index = reflected_index(&field.type_path, index)?;
            project_node(
                context.registry,
                &field.type_path,
                current.get(index as usize).copied(),
                default.get(index as usize).copied(),
                default_supported,
                context.binding.tuple_index(index),
                context.diagnostics,
                context.read_only,
                context.hidden,
                context.depth + 1,
            )
            .map(Box::new)
            .map(|value| ReflectedInspectionChild::TupleElement { index, value })
        })
        .collect()
}

fn project_list_children(
    context: ChildProjection<'_>,
    current: Option<&ReflectedValue>,
    default: Option<&ReflectedValue>,
) -> Result<Vec<ReflectedInspectionChild>, ReflectedProjectionError> {
    let item_type = generic_arguments(&context.descriptor.type_path)
        .into_iter()
        .next()
        .ok_or_else(|| ReflectedProjectionError::Decode {
            type_path: context.descriptor.type_path.clone(),
            message: "list type path has no item argument".to_owned(),
        })?;
    let current_items = list_values(current);
    let default_items = list_values(default);
    current_items
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let index = reflected_index(&item_type, index)?;
            project_node(
                context.registry,
                &item_type,
                Some(value),
                default_items.get(index as usize),
                false,
                context.binding.list_index(index),
                context.diagnostics,
                context.read_only,
                context.hidden,
                context.depth + 1,
            )
            .map(Box::new)
            .map(|value| ReflectedInspectionChild::ListItem(ReflectedListItem { index, value }))
        })
        .collect()
}

fn project_map_children(
    context: ChildProjection<'_>,
    current: Option<&ReflectedValue>,
    default: Option<&ReflectedValue>,
) -> Result<Vec<ReflectedInspectionChild>, ReflectedProjectionError> {
    let arguments = generic_arguments(&context.descriptor.type_path);
    let value_type = arguments
        .get(1)
        .ok_or_else(|| ReflectedProjectionError::Decode {
            type_path: context.descriptor.type_path.clone(),
            message: "map type path has no value argument".to_owned(),
        })?;
    let current_entries = map_values(current);
    let default_entries = map_values(default);
    current_entries
        .iter()
        .map(|entry| {
            let default = default_entries
                .iter()
                .find(|candidate| candidate.key == entry.key)
                .map(|candidate| &candidate.value);
            project_node(
                context.registry,
                value_type,
                Some(&entry.value),
                default,
                false,
                context.binding.clone(),
                context.diagnostics,
                context.read_only,
                context.hidden,
                context.depth + 1,
            )
            .map(Box::new)
            .map(|value| {
                ReflectedInspectionChild::MapEntry(Box::new(ReflectedMapEntry {
                    key: entry.key.clone(),
                    value,
                    binding: ReflectedMapEntryBinding {
                        map: context.binding.clone(),
                        key: entry.key_envelope.clone(),
                    },
                    value_envelope: entry.value_envelope.clone(),
                }))
            })
        })
        .collect()
}

fn project_optional_children(
    context: ChildProjection<'_>,
    current: Option<&ReflectedValue>,
    default: Option<&ReflectedValue>,
) -> Result<Vec<ReflectedInspectionChild>, ReflectedProjectionError> {
    let inner_type = generic_arguments(&context.descriptor.type_path)
        .into_iter()
        .next()
        .ok_or_else(|| ReflectedProjectionError::Decode {
            type_path: context.descriptor.type_path.clone(),
            message: "Option type path has no inner argument".to_owned(),
        })?;
    let Some(values) = optional_child_values(current, default) else {
        return Ok(Vec::new());
    };
    Ok(vec![ReflectedInspectionChild::OptionalSome(Box::new(
        project_node(
            context.registry,
            &inner_type,
            values.current,
            values.default,
            false,
            context.binding.variant("Some").tuple_index(0),
            context.diagnostics,
            context.read_only,
            context.hidden,
            context.depth + 1,
        )?,
    ))])
}

#[allow(clippy::too_many_arguments)]
fn project_enum_children(
    registry: &TypeRegistrySnapshot,
    descriptor: &ReflectedTypeDescriptor,
    selected: Option<&ReflectedValue>,
    default: Option<&ReflectedValue>,
    binding: &ReflectedEditBinding,
    diagnostics: &[PrefabDiagnostic],
    read_only: bool,
    hidden: bool,
    depth: usize,
) -> Result<Vec<ReflectedInspectionChild>, ReflectedProjectionError> {
    let Some(ReflectedValue::Enum { variant, fields }) = selected else {
        return Ok(Vec::new());
    };
    // Retained variant fields are keyed by the name they were declared under,
    // never by their position in the decoded list: a sparse value that retains
    // only a later field decodes to a shorter list, and indexing it would put
    // that value on an earlier field's slot.
    let selected_fields = variant_fields(fields);
    let default_fields = match default {
        Some(ReflectedValue::Enum {
            variant: default_variant,
            fields,
        }) if default_variant == variant => variant_fields(fields),
        _ => BTreeMap::new(),
    };
    let variant_descriptor = descriptor
        .variants
        .iter()
        .find(|candidate| candidate.name == *variant)
        .ok_or_else(|| ReflectedProjectionError::Decode {
            type_path: descriptor.type_path.clone(),
            message: format!("unknown reflected variant `{variant}`"),
        })?;
    let variant_binding = binding.variant(variant.clone());
    let children = variant_descriptor
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let element_index = reflected_index(&field.type_path, index)?;
            let field_binding = if field.name == index.to_string() {
                variant_binding.tuple_index(element_index)
            } else {
                variant_binding.field(field.name.clone())
            };
            project_node(
                registry,
                &field.type_path,
                selected_fields.get(field.name.as_str()).copied(),
                default_fields.get(field.name.as_str()).copied(),
                false,
                field_binding,
                diagnostics,
                read_only || field.editor_attributes.read_only,
                hidden || field.editor_attributes.hidden,
                depth + 1,
            )
            .map(Box::new)
            .map(|value| ReflectedInspectionChild::TupleElement {
                index: element_index,
                value,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(vec![ReflectedInspectionChild::Variant(
        ReflectedVariantSelection {
            name: variant.clone(),
            fields: children,
        },
    )])
}

fn widget_spec(registry: &TypeRegistrySnapshot, field: &ReflectedFieldDescriptor) -> WidgetSpec {
    let descriptor = registry
        .types
        .iter()
        .find(|descriptor| descriptor.type_path == field.type_path);
    let widget = field
        .editor_attributes
        .widget
        .as_deref()
        .unwrap_or_default();
    let mut rows = None;
    let family = if widget == "slider" {
        WidgetFamily::Slider
    } else if widget == "number" {
        WidgetFamily::Number
    } else if widget == "checkbox" || widget == "toggle" {
        WidgetFamily::Bool
    } else if widget == "color" {
        WidgetFamily::Color
    } else if let Some(asset_type) = widget.strip_prefix("asset:") {
        WidgetFamily::Asset {
            asset_type: asset_type.to_owned(),
        }
    } else if let Some(object_type) = widget.strip_prefix("object:") {
        WidgetFamily::Object {
            object_type: object_type.to_owned(),
        }
    } else if let Some(dimensions) = widget.strip_prefix("vector:") {
        if is_quaternion(&field.type_path) {
            WidgetFamily::Quaternion
        } else {
            WidgetFamily::Vector {
                dimensions: dimensions.parse().unwrap_or(0),
            }
        }
    } else if let Some(value) = widget.strip_prefix("multiline:") {
        rows = value.parse().ok();
        WidgetFamily::Multiline
    } else if widget == "multiline" {
        WidgetFamily::Multiline
    } else {
        descriptor.map_or(WidgetFamily::Opaque, |descriptor| {
            let structural = WidgetFamily::from(&descriptor.kind);
            if structural == WidgetFamily::Opaque && is_quaternion(&field.type_path) {
                WidgetFamily::Quaternion
            } else {
                structural
            }
        })
    };
    WidgetSpec {
        family,
        range: field.editor_attributes.range.clone(),
        rows,
        constraints: field.editor_attributes.constraints.clone(),
        variants: descriptor.map_or_else(Vec::new, |descriptor| {
            descriptor
                .variants
                .iter()
                .filter(|variant| {
                    field
                        .editor_attributes
                        .constraints
                        .allowed_variants
                        .is_empty()
                        || field
                            .editor_attributes
                            .constraints
                            .allowed_variants
                            .contains(&variant.name)
                })
                .map(|variant| variant.name.clone())
                .collect()
        }),
    }
}

fn decode_envelope(
    registry: &TypeRegistrySnapshot,
    envelope: &ReflectedValueEnvelope,
) -> Result<ReflectedValue, ReflectedProjectionError> {
    if envelope.encoding != ReflectedValueEncoding::TypedRon {
        return Ok(ReflectedValue::Encoded(envelope.clone()));
    }
    let source = std::str::from_utf8(&envelope.payload).map_err(|error| {
        ReflectedProjectionError::InvalidUtf8 {
            type_path: envelope.type_path.clone(),
            message: error.to_string(),
        }
    })?;
    let raw = RawValue::from_ron(source).map_err(|error| ReflectedProjectionError::Decode {
        type_path: envelope.type_path.clone(),
        message: error.to_string(),
    })?;
    decode_raw(registry, &envelope.type_path, raw)
}

/// Decodes one typed reflected envelope against the authoritative registry.
///
/// Domain projections use this when they must preserve an aggregate default
/// while constructing a named-path structural edit.
///
/// # Errors
///
/// Returns [`ReflectedProjectionError::InvalidUtf8`] if a `TypedRon` payload
/// is not valid UTF-8, [`ReflectedProjectionError::Decode`] if it is not
/// parseable RON or does not match the descriptor's shape, or
/// [`ReflectedProjectionError::MissingType`] if the registry publishes no
/// descriptor for the envelope's type path. Envelopes in any other encoding
/// are returned unchanged and cannot fail.
pub fn decode_reflected_envelope(
    registry: &TypeRegistrySnapshot,
    envelope: &ReflectedValueEnvelope,
) -> Result<ReflectedValue, ReflectedProjectionError> {
    decode_envelope(registry, envelope)
}

fn trim_numeric_suffix(source: &str, suffix: &str) -> String {
    source
        .trim()
        .strip_suffix(suffix)
        .unwrap_or_else(|| source.trim())
        .trim()
        .to_owned()
}

fn decode_raw(
    registry: &TypeRegistrySnapshot,
    type_path: &str,
    raw: &RawValue,
) -> Result<ReflectedValue, ReflectedProjectionError> {
    let descriptor = descriptor(registry, type_path)?;
    let decode_error = |error: ron::error::SpannedError| ReflectedProjectionError::Decode {
        type_path: type_path.to_owned(),
        message: error.to_string(),
    };
    if matches!(descriptor.kind, ReflectedTypeKind::Struct) && is_math_tuple(type_path) {
        return decode_opaque(raw);
    }
    match descriptor.kind {
        ReflectedTypeKind::Struct => {
            // A sparse struct that retains no field is emitted as the RON unit
            // payload `()`, which untyped RON classifies as a unit value rather
            // than a zero-length named-field map. The authoritative descriptor
            // is what disambiguates it: only a `Struct` kind reaches this arm,
            // so resolving the unit payload here reads an empty sparse struct
            // exactly as a parsed empty field map would, and leaves how every
            // other reflected kind classifies `()` untouched.
            if is_unit_payload(raw) {
                return Ok(ReflectedValue::Struct(Vec::new()));
            }
            let fields = raw.into_rust::<RawNamedFields>().map_err(decode_error)?.0;
            descriptor
                .fields
                .iter()
                .filter_map(|field| fields.get(&field.name).map(|raw| (field, raw)))
                .map(|(field, raw)| {
                    decode_raw(registry, &field.type_path, raw)
                        .map(|value| (field.name.clone(), value))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(ReflectedValue::Struct)
        }
        ReflectedTypeKind::Tuple | ReflectedTypeKind::TupleStruct => {
            let values = raw.into_rust::<RawSequence>().map_err(decode_error)?.0;
            descriptor
                .fields
                .iter()
                .zip(values.iter())
                .map(|(field, raw)| decode_raw(registry, &field.type_path, raw))
                .collect::<Result<Vec<_>, _>>()
                .map(ReflectedValue::Tuple)
        }
        ReflectedTypeKind::List | ReflectedTypeKind::Array { .. } => {
            let item_type = generic_arguments(type_path)
                .into_iter()
                .next()
                .ok_or_else(|| ReflectedProjectionError::Decode {
                    type_path: type_path.to_owned(),
                    message: "list type path has no item argument".to_owned(),
                })?;
            raw.into_rust::<RawSequence>()
                .map_err(decode_error)?
                .0
                .iter()
                .map(|raw| decode_raw(registry, &item_type, raw))
                .collect::<Result<Vec<_>, _>>()
                .map(ReflectedValue::List)
        }
        ReflectedTypeKind::Map => {
            let arguments = generic_arguments(type_path);
            let (key_type, value_type) =
                arguments.first().zip(arguments.get(1)).ok_or_else(|| {
                    ReflectedProjectionError::Decode {
                        type_path: type_path.to_owned(),
                        message: "map type path must carry key and value arguments".to_owned(),
                    }
                })?;
            raw.into_rust::<RawEntries>()
                .map_err(decode_error)?
                .0
                .iter()
                .map(|(key, value)| {
                    Ok(ReflectedMapValueEntry {
                        key: decode_raw(registry, key_type, key)?,
                        value: decode_raw(registry, value_type, value)?,
                        key_envelope: raw_envelope(key_type, key),
                        value_envelope: raw_envelope(value_type, value),
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .map(ReflectedValue::Map)
        }
        ReflectedTypeKind::Enum => decode_enum(registry, descriptor, raw),
        ReflectedTypeKind::Optional => decode_option(registry, descriptor, raw),
        ReflectedTypeKind::Bool => raw
            .into_rust::<bool>()
            .map(ReflectedScalar::Bool)
            .map(ReflectedValue::Scalar)
            .map_err(decode_error),
        ReflectedTypeKind::SignedInteger { .. } => Ok(ReflectedValue::Scalar(
            ReflectedScalar::Signed(raw.get_ron().trim().to_owned()),
        )),
        ReflectedTypeKind::UnsignedInteger { .. } => Ok(ReflectedValue::Scalar(
            ReflectedScalar::Unsigned(raw.get_ron().trim().to_owned()),
        )),
        ReflectedTypeKind::Float { .. } => Ok(ReflectedValue::Scalar(ReflectedScalar::Float(
            trim_float_suffix(raw.get_ron()),
        ))),
        ReflectedTypeKind::String => raw
            .into_rust::<String>()
            .map(ReflectedScalar::String)
            .map(ReflectedValue::Scalar)
            .map_err(decode_error),
        ReflectedTypeKind::Opaque | ReflectedTypeKind::Set => decode_opaque(raw),
    }
}

fn decode_enum(
    registry: &TypeRegistrySnapshot,
    descriptor: &ReflectedTypeDescriptor,
    raw: &RawValue,
) -> Result<ReflectedValue, ReflectedProjectionError> {
    let (variant, payload) =
        variant_parts(raw.get_ron()).ok_or_else(|| ReflectedProjectionError::Decode {
            type_path: descriptor.type_path.clone(),
            message: "enum value has no named variant".to_owned(),
        })?;
    let variant_descriptor = descriptor
        .variants
        .iter()
        .find(|candidate| candidate.name == variant)
        .ok_or_else(|| ReflectedProjectionError::Decode {
            type_path: descriptor.type_path.clone(),
            message: format!("unknown enum variant `{variant}`"),
        })?;
    let fields = if variant_descriptor.fields.is_empty() {
        Vec::new()
    } else if variant_descriptor.fields.len() == 1 && variant_descriptor.fields[0].name == "0" {
        let field = &variant_descriptor.fields[0];
        let raw = owned_raw(payload.unwrap_or("()"), &descriptor.type_path)?;
        vec![(
            field.name.clone(),
            decode_raw(registry, &field.type_path, &raw)?,
        )]
    } else {
        let wrapped = format!("({})", payload.unwrap_or_default());
        let raw = owned_raw(&wrapped, &descriptor.type_path)?;
        if variant_descriptor
            .fields
            .iter()
            .enumerate()
            .all(|(index, field)| field.name == index.to_string())
        {
            let values = raw.into_rust::<RawSequence>().map_err(|error| {
                ReflectedProjectionError::Decode {
                    type_path: descriptor.type_path.clone(),
                    message: error.to_string(),
                }
            })?;
            variant_descriptor
                .fields
                .iter()
                .zip(values.0.iter())
                .map(|(field, raw)| {
                    decode_raw(registry, &field.type_path, raw)
                        .map(|value| (field.name.clone(), value))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else if is_empty_variant_body(payload) {
            // A struct-shaped variant that retains no field is emitted as
            // `Named()`, whose body wraps to the RON unit payload `()`; untyped
            // RON classifies that as a unit value rather than a zero-length
            // named-field map. The authoritative variant descriptor is what
            // disambiguates it: this arm is reached only once the descriptor
            // has ruled the variant struct-shaped, so reading the empty body
            // here yields exactly what a parsed empty field map would, and
            // leaves tuple-shaped variants untouched. A unit variant carries no
            // body at all, so `Named` still fails here just as before.
            Vec::new()
        } else {
            let values = raw.into_rust::<RawNamedFields>().map_err(|error| {
                ReflectedProjectionError::Decode {
                    type_path: descriptor.type_path.clone(),
                    message: error.to_string(),
                }
            })?;
            // A sparse value retains any subset of the declared fields, so the
            // decoded list is shorter than the declaration whenever a field is
            // omitted. Each retained value carries the name it was declared
            // under; nothing downstream may recover that from its position.
            variant_descriptor
                .fields
                .iter()
                .filter_map(|field| values.0.get(&field.name).map(|raw| (field, raw)))
                .map(|(field, raw)| {
                    decode_raw(registry, &field.type_path, raw)
                        .map(|value| (field.name.clone(), value))
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(ReflectedValue::Enum {
        variant: variant.to_owned(),
        fields,
    })
}

fn decode_option(
    registry: &TypeRegistrySnapshot,
    descriptor: &ReflectedTypeDescriptor,
    raw: &RawValue,
) -> Result<ReflectedValue, ReflectedProjectionError> {
    let source = raw.get_ron().trim();
    let source = source.strip_prefix("r#").unwrap_or(source);
    if source == "None" {
        return Ok(ReflectedValue::Optional(None));
    }
    let inner = source
        .strip_prefix("Some(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(source);
    let inner_type = generic_arguments(&descriptor.type_path)
        .into_iter()
        .next()
        .ok_or_else(|| ReflectedProjectionError::Decode {
            type_path: descriptor.type_path.clone(),
            message: "Option type path has no inner argument".to_owned(),
        })?;
    let inner = owned_raw(inner, &descriptor.type_path)?;
    Ok(ReflectedValue::Optional(Some(Box::new(decode_raw(
        registry,
        &inner_type,
        &inner,
    )?))))
}

fn decode_opaque(raw: &RawValue) -> Result<ReflectedValue, ReflectedProjectionError> {
    let source = raw.get_ron().trim();
    if source.starts_with('"') {
        return raw
            .into_rust::<String>()
            .map(ReflectedScalar::String)
            .map(ReflectedValue::Scalar)
            .map_err(|error| ReflectedProjectionError::Decode {
                type_path: "opaque".to_owned(),
                message: error.to_string(),
            });
    }
    if source.starts_with('(')
        && let Ok(values) = raw.into_rust::<RawSequence>()
    {
        return Ok(ReflectedValue::Tuple(
            values
                .0
                .iter()
                .map(|value| {
                    ReflectedValue::Scalar(ReflectedScalar::Float(trim_float_suffix(
                        value.get_ron(),
                    )))
                })
                .collect(),
        ));
    }
    Ok(ReflectedValue::OpaqueRon(source.to_owned()))
}

/// Narrows a positional field or element index to the `u32` the reflected
/// binding wire uses.
///
/// A descriptor carrying more than `u32::MAX` fields cannot be addressed by a
/// binding at all, so this reports a decode error instead of truncating the
/// index and silently addressing the wrong element.
fn reflected_index(type_path: &str, index: usize) -> Result<u32, ReflectedProjectionError> {
    u32::try_from(index).map_err(|_| ReflectedProjectionError::Decode {
        type_path: type_path.to_owned(),
        message: format!("index {index} exceeds the reflected binding index range"),
    })
}

fn descriptor<'a>(
    registry: &'a TypeRegistrySnapshot,
    type_path: &str,
) -> Result<&'a ReflectedTypeDescriptor, ReflectedProjectionError> {
    registry
        .types
        .iter()
        .find(|descriptor| descriptor.type_path == type_path)
        .ok_or_else(|| ReflectedProjectionError::MissingType(type_path.to_owned()))
}

fn has_flag(descriptor: &ReflectedTypeDescriptor, flag: &str) -> bool {
    descriptor
        .type_data_flags
        .iter()
        .any(|candidate| candidate == flag)
}

fn validation_for(
    diagnostics: &[PrefabDiagnostic],
    target: &PrefabValueTarget,
) -> ReflectedValidationState {
    ReflectedValidationState {
        diagnostics: diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.target.as_ref().is_some_and(|candidate| {
                    candidate.entity_alias == target.entity_alias
                        && candidate.instance_alias_chain == target.instance_alias_chain
                        && candidate.path.component_type_path == target.path.component_type_path
                        && candidate.path.segments.starts_with(&target.path.segments)
                })
            })
            .cloned()
            .collect(),
    }
}

fn struct_fields(value: &ReflectedValue) -> BTreeMap<&str, &ReflectedValue> {
    match value {
        ReflectedValue::Struct(fields) => fields
            .iter()
            .map(|(name, value)| (name.as_str(), value))
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// Keys the fields a variant retains by their declared name, the same lookup
/// [`struct_fields`] gives a sparse struct.
fn variant_fields(fields: &[(String, ReflectedValue)]) -> BTreeMap<&str, &ReflectedValue> {
    fields
        .iter()
        .map(|(name, value)| (name.as_str(), value))
        .collect()
}

fn tuple_values(value: Option<&ReflectedValue>) -> Vec<&ReflectedValue> {
    match value {
        Some(ReflectedValue::Tuple(values)) => values.iter().collect(),
        _ => Vec::new(),
    }
}

fn list_values(value: Option<&ReflectedValue>) -> &[ReflectedValue] {
    match value {
        Some(ReflectedValue::List(values)) => values,
        _ => &[],
    }
}

fn map_values(value: Option<&ReflectedValue>) -> &[ReflectedMapValueEntry] {
    match value {
        Some(ReflectedValue::Map(values)) => values,
        _ => &[],
    }
}

struct OptionalChildValues<'a> {
    current: Option<&'a ReflectedValue>,
    default: Option<&'a ReflectedValue>,
}

/// Selects inner editor inputs without collapsing explicit `None` into field
/// absence. Only genuine absence may materialize a `Some` default.
fn optional_child_values<'a>(
    current: Option<&'a ReflectedValue>,
    default: Option<&'a ReflectedValue>,
) -> Option<OptionalChildValues<'a>> {
    let default = optional_some_value(default);
    match current {
        Some(ReflectedValue::Optional(Some(current))) => Some(OptionalChildValues {
            current: Some(current),
            default,
        }),
        None => default.map(|default| OptionalChildValues {
            current: None,
            default: Some(default),
        }),
        // An explicit `Optional(None)` is a real "unset" value rather than
        // absence, so it never materializes the default; neither does a
        // non-optional value.
        Some(_) => None,
    }
}

fn optional_some_value(value: Option<&ReflectedValue>) -> Option<&ReflectedValue> {
    match value {
        Some(ReflectedValue::Optional(Some(value))) => Some(value),
        _ => None,
    }
}

fn generic_arguments(type_path: &str) -> Vec<String> {
    let Some(start) = type_path.find('<') else {
        return Vec::new();
    };
    let Some(end) = type_path.rfind('>') else {
        return Vec::new();
    };
    let mut depth = 0_u32;
    let mut item_start = start + 1;
    let mut values = Vec::new();
    for (offset, character) in type_path[start + 1..end].char_indices() {
        match character {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let item_end = start + 1 + offset;
                values.push(type_path[item_start..item_end].trim().to_owned());
                item_start = item_end + 1;
            }
            _ => {}
        }
    }
    values.push(type_path[item_start..end].trim().to_owned());
    values
}

fn variant_parts(source: &str) -> Option<(&str, Option<&str>)> {
    let source = source.trim();
    let open = source.find('(');
    match open {
        Some(open) if source.ends_with(')') => Some((
            source[..open].trim(),
            Some(&source[open + 1..source.len() - 1]),
        )),
        None if !source.is_empty() => Some((source, None)),
        _ => None,
    }
}

/// True when `raw` is the canonical RON unit payload `()`.
fn is_unit_payload(raw: &RawValue) -> bool {
    raw.get_ron().trim() == "()"
}

/// True when an enum variant body is present but empty, the producer's
/// `Named()` spelling for a variant retaining no field. A unit variant is
/// spelled without a body at all, so an absent body is deliberately not empty.
fn is_empty_variant_body(body: Option<&str>) -> bool {
    body.is_some_and(|body| body.trim().is_empty())
}

fn owned_raw(source: &str, type_path: &str) -> Result<Box<RawValue>, ReflectedProjectionError> {
    RawValue::from_boxed_ron(source.to_owned().into_boxed_str()).map_err(|error| {
        ReflectedProjectionError::Decode {
            type_path: type_path.to_owned(),
            message: error.to_string(),
        }
    })
}

fn raw_envelope(type_path: &str, raw: &RawValue) -> ReflectedValueEnvelope {
    ReflectedValueEnvelope {
        type_path: type_path.to_owned(),
        encoding: ReflectedValueEncoding::TypedRon,
        payload: raw.get_ron().as_bytes().to_vec(),
    }
}

fn trim_float_suffix(source: &str) -> String {
    source
        .trim()
        .strip_suffix("f32")
        .or_else(|| source.trim().strip_suffix("f64"))
        .unwrap_or_else(|| source.trim())
        .to_owned()
}

fn is_quaternion(type_path: &str) -> bool {
    type_path.rsplit("::").next() == Some("Quat")
}

fn is_math_tuple(type_path: &str) -> bool {
    matches!(
        type_path.rsplit("::").next(),
        Some("Vec2" | "Vec3" | "Vec4" | "Quat")
    )
}

fn humanize(name: &str) -> String {
    let mut output = String::with_capacity(name.len());
    let mut uppercase = true;
    for character in name.chars() {
        if character == '_' || character == '-' {
            output.push(' ');
            uppercase = true;
        } else if uppercase {
            output.extend(character.to_uppercase());
            uppercase = false;
        } else {
            output.push(character);
        }
    }
    output
}

struct RawNamedFields(BTreeMap<String, Box<RawValue>>);

impl<'de> Deserialize<'de> for RawNamedFields {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RawNamedFieldsVisitor)
    }
}

struct RawNamedFieldsVisitor;

impl<'de> Visitor<'de> for RawNamedFieldsVisitor {
    type Value = RawNamedFields;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a reflected struct")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut fields = BTreeMap::new();
        while let Some((name, value)) = map.next_entry()? {
            fields.insert(name, value);
        }
        Ok(RawNamedFields(fields))
    }
}

struct RawSequence(Vec<Box<RawValue>>);

impl<'de> Deserialize<'de> for RawSequence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RawSequenceVisitor)
    }
}

struct RawSequenceVisitor;

impl<'de> Visitor<'de> for RawSequenceVisitor {
    type Value = RawSequence;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a reflected sequence")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(RawSequence(values))
    }
}

struct RawEntries(Vec<(Box<RawValue>, Box<RawValue>)>);

impl<'de> Deserialize<'de> for RawEntries {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RawEntriesVisitor)
    }
}

struct RawEntriesVisitor;

impl<'de> Visitor<'de> for RawEntriesVisitor {
    type Value = RawEntries;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a reflected map")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::new();
        while let Some(entry) = map.next_entry()? {
            entries.push(entry);
        }
        Ok(RawEntries(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_proto_project::vnext::{
        ApplicabilityDescriptor, EditorAttributes, ReflectedVariantDescriptor,
    };

    fn descriptor(type_path: &str, kind: ReflectedTypeKind) -> ReflectedTypeDescriptor {
        ReflectedTypeDescriptor {
            type_path: type_path.to_owned(),
            short_path: type_path.to_owned(),
            kind,
            fields: Vec::new(),
            variants: Vec::new(),
            editor_attributes: EditorAttributes::default(),
            type_data_flags: Vec::new(),
            applicability: ApplicabilityDescriptor::default(),
            reflected_default: None,
        }
    }

    fn float(value: &str) -> ReflectedValue {
        ReflectedValue::Scalar(ReflectedScalar::Float(value.to_owned()))
    }

    fn optional_some(value: ReflectedValue) -> ReflectedValue {
        ReflectedValue::Optional(Some(Box::new(value)))
    }

    #[test]
    fn generic_arguments_preserve_nested_type_paths() {
        assert_eq!(
            generic_arguments(
                "std::collections::BTreeMap<alloc::string::String, alloc::vec::Vec<f32>>"
            ),
            ["alloc::string::String", "alloc::vec::Vec<f32>"]
        );
    }

    #[test]
    fn sparse_decode_distinguishes_explicit_option_none_from_absent_field() {
        let option_path = "core::option::Option<f32>";
        let mut component = descriptor("fixture::OptionalComponent", ReflectedTypeKind::Struct);
        component.fields.extend([
            ReflectedFieldDescriptor {
                name: "marker".to_owned(),
                type_path: "bool".to_owned(),
                editor_attributes: EditorAttributes::default(),
            },
            ReflectedFieldDescriptor {
                name: "nested".to_owned(),
                type_path: option_path.to_owned(),
                editor_attributes: EditorAttributes::default(),
            },
        ]);
        let registry = TypeRegistrySnapshot {
            schema_catalog_hash: Vec::new(),
            types: vec![
                component,
                descriptor("bool", ReflectedTypeKind::Bool),
                descriptor(option_path, ReflectedTypeKind::Optional),
                descriptor("f32", ReflectedTypeKind::Float { bits: 32 }),
            ],
        };
        let envelope = |payload: &[u8]| ReflectedValueEnvelope {
            type_path: "fixture::OptionalComponent".to_owned(),
            encoding: ReflectedValueEncoding::TypedRon,
            payload: payload.to_vec(),
        };

        assert_eq!(
            decode_reflected_envelope(&registry, &envelope(b"(marker: true, nested: None)"))
                .expect("decode explicit None"),
            ReflectedValue::Struct(vec![
                (
                    "marker".to_owned(),
                    ReflectedValue::Scalar(ReflectedScalar::Bool(true)),
                ),
                ("nested".to_owned(), ReflectedValue::Optional(None)),
            ])
        );
        assert_eq!(
            decode_reflected_envelope(&registry, &envelope(b"(marker: true)"))
                .expect("decode absent field"),
            ReflectedValue::Struct(vec![(
                "marker".to_owned(),
                ReflectedValue::Scalar(ReflectedScalar::Bool(true)),
            )])
        );
    }

    /// The producer emits `()` for a sparse struct retaining no field, and RON
    /// classifies that payload as a unit value. Only the authoritative `Struct`
    /// descriptor turns it into an empty struct; every other kind keeps reading
    /// `()` exactly as it did before.
    #[test]
    fn unit_payload_decodes_as_an_empty_struct_only_under_a_struct_descriptor() {
        let option_path = "core::option::Option<f32>";
        let mut component = descriptor("fixture::EmptySparse", ReflectedTypeKind::Struct);
        component.fields.push(ReflectedFieldDescriptor {
            name: "marker".to_owned(),
            type_path: "bool".to_owned(),
            editor_attributes: EditorAttributes::default(),
        });
        let mut tuple_struct = descriptor("fixture::TupleStruct", ReflectedTypeKind::TupleStruct);
        tuple_struct.fields.push(ReflectedFieldDescriptor {
            name: "0".to_owned(),
            type_path: "f32".to_owned(),
            editor_attributes: EditorAttributes::default(),
        });
        let mut tuple = descriptor("fixture::Tuple", ReflectedTypeKind::Tuple);
        tuple.fields.push(ReflectedFieldDescriptor {
            name: "0".to_owned(),
            type_path: "f32".to_owned(),
            editor_attributes: EditorAttributes::default(),
        });
        let registry = TypeRegistrySnapshot {
            schema_catalog_hash: Vec::new(),
            types: vec![
                component,
                tuple_struct,
                tuple,
                descriptor("bool", ReflectedTypeKind::Bool),
                descriptor(option_path, ReflectedTypeKind::Optional),
                descriptor("f32", ReflectedTypeKind::Float { bits: 32 }),
                descriptor("fixture::Opaque", ReflectedTypeKind::Opaque),
                descriptor("glam::Vec3", ReflectedTypeKind::Struct),
            ],
        };
        let unit = |type_path: &str| ReflectedValueEnvelope {
            type_path: type_path.to_owned(),
            encoding: ReflectedValueEncoding::TypedRon,
            payload: b"()".to_vec(),
        };

        // The fix: a `Struct` descriptor reads `()` as the empty sparse struct.
        assert_eq!(
            decode_reflected_envelope(&registry, &unit("fixture::EmptySparse"))
                .expect("decode empty sparse struct"),
            ReflectedValue::Struct(Vec::new()),
        );
        // ...and a struct that does retain a field is untouched.
        assert_eq!(
            decode_reflected_envelope(
                &registry,
                &ReflectedValueEnvelope {
                    type_path: "fixture::EmptySparse".to_owned(),
                    encoding: ReflectedValueEncoding::TypedRon,
                    payload: b"(marker: true)".to_vec(),
                },
            )
            .expect("decode populated sparse struct"),
            ReflectedValue::Struct(vec![(
                "marker".to_owned(),
                ReflectedValue::Scalar(ReflectedScalar::Bool(true)),
            )]),
        );

        // Negative controls: pinned, unchanged behavior for every other kind.
        assert!(
            decode_reflected_envelope(&registry, &unit("fixture::TupleStruct")).is_err(),
            "a tuple struct still rejects a unit payload",
        );
        assert!(
            decode_reflected_envelope(&registry, &unit("fixture::Tuple")).is_err(),
            "a tuple still rejects a unit payload",
        );
        assert_eq!(
            decode_reflected_envelope(&registry, &unit("fixture::Opaque"))
                .expect("an opaque unit payload still decodes verbatim"),
            ReflectedValue::OpaqueRon("()".to_owned()),
        );
        assert_eq!(
            decode_reflected_envelope(&registry, &unit(option_path))
                .expect("an Option unit payload still decodes as before"),
            optional_some(float("()")),
        );
        // A math tuple carries a `Struct` descriptor but is routed to opaque
        // decoding ahead of the struct branch; that routing is unchanged.
        assert_eq!(
            decode_reflected_envelope(&registry, &unit("glam::Vec3"))
                .expect("a math tuple unit payload still decodes opaquely"),
            ReflectedValue::OpaqueRon("()".to_owned()),
        );
        assert_eq!(
            decode_reflected_envelope(
                &registry,
                &ReflectedValueEnvelope {
                    type_path: option_path.to_owned(),
                    encoding: ReflectedValueEncoding::TypedRon,
                    payload: b"None".to_vec(),
                },
            )
            .expect("explicit None is untouched"),
            ReflectedValue::Optional(None),
        );
    }

    /// The producer spells a struct-shaped variant that retains no field
    /// `Named()`, and that empty body wraps to the RON unit payload `()`. Only
    /// the variant descriptor's named fields turn it into the variant carrying
    /// no field; tuple-shaped variants and bodiless payloads keep the meaning
    /// they always had.
    #[test]
    fn an_empty_variant_body_decodes_as_a_struct_variant_only_under_named_variant_fields() {
        let field = |name: &str, type_path: &str| ReflectedFieldDescriptor {
            name: name.to_owned(),
            type_path: type_path.to_owned(),
            editor_attributes: EditorAttributes::default(),
        };
        let declared =
            |name: &str, fields: Vec<ReflectedFieldDescriptor>| ReflectedVariantDescriptor {
                name: name.to_owned(),
                fields,
                editor_attributes: EditorAttributes::default(),
            };
        let mut mode = descriptor("fixture::Mode", ReflectedTypeKind::Enum);
        mode.variants = vec![
            declared("Marker", Vec::new()),
            declared("Fieldless", Vec::new()),
            declared("Single", vec![field("0", "f32")]),
            declared("Pair", vec![field("0", "f32"), field("1", "f32")]),
            declared("Named", vec![field("alpha", "f32"), field("beta", "bool")]),
        ];
        let registry = TypeRegistrySnapshot {
            schema_catalog_hash: Vec::new(),
            types: vec![
                mode,
                descriptor("f32", ReflectedTypeKind::Float { bits: 32 }),
                descriptor("bool", ReflectedTypeKind::Bool),
            ],
        };
        let decode = |payload: &str| {
            decode_reflected_envelope(
                &registry,
                &ReflectedValueEnvelope {
                    type_path: "fixture::Mode".to_owned(),
                    encoding: ReflectedValueEncoding::TypedRon,
                    payload: payload.as_bytes().to_vec(),
                },
            )
        };
        let selected = |variant: &str, fields: Vec<(&str, ReflectedValue)>| ReflectedValue::Enum {
            variant: variant.to_owned(),
            fields: fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        };

        // The fix: named variant fields read the empty body as that variant
        // carrying no field.
        assert_eq!(
            decode("Named()").expect("an empty struct variant decodes"),
            selected("Named", Vec::new()),
        );
        // ...and a struct-shaped variant that does retain a field is untouched,
        // the retained value carrying the name it was declared under.
        assert_eq!(
            decode("Named(beta:true)").expect("a partly retained struct variant decodes"),
            selected(
                "Named",
                vec![("beta", ReflectedValue::Scalar(ReflectedScalar::Bool(true)))],
            ),
        );

        // Negative controls: pinned, unchanged behavior for every other shape.
        assert_eq!(
            decode("Marker").expect("a unit variant decodes"),
            selected("Marker", Vec::new()),
        );
        assert_eq!(
            decode("Fieldless()").expect("a variant declaring no field decodes"),
            selected("Fieldless", Vec::new()),
        );
        assert_eq!(
            decode("Single(1.0)").expect("a newtype variant decodes"),
            selected("Single", vec![("0", float("1.0"))]),
        );
        assert_eq!(
            decode("Pair(1.0,2.0)").expect("a tuple variant decodes"),
            selected("Pair", vec![("0", float("1.0")), ("1", float("2.0"))]),
        );
        assert!(
            decode("Pair()").is_err(),
            "a tuple-shaped variant still rejects an empty body",
        );
        assert!(
            decode("Named").is_err(),
            "a struct-shaped variant still requires a body",
        );
    }

    /// A sparse value retains any subset of a struct-shaped variant's declared
    /// fields, so the decoded list is shorter than the declaration whenever one
    /// is omitted. Each retained value is keyed by the name it was declared
    /// under and projects onto that field's own slot; every omitted field reads
    /// as absent — authored `None`, the same absence an omitted struct field
    /// projects, and never an explicit `None` value. Tuple-shaped variants
    /// declare their fields under index names, so they keep the positional
    /// reading they always had.
    const VARIANT_FIXTURE_COMPONENT_PATH: &str = "fixture::VariantComponent";
    const VARIANT_FIXTURE_OPTION_PATH: &str = "core::option::Option<bool>";

    /// Registry fixture for the partially-retained-variant test: one component
    /// with a single `mode` enum field whose variants cover the marker, tuple,
    /// named and mixed-optional shapes.
    fn variant_fixture_registry() -> TypeRegistrySnapshot {
        let field = |name: &str, type_path: &str| ReflectedFieldDescriptor {
            name: name.to_owned(),
            type_path: type_path.to_owned(),
            editor_attributes: EditorAttributes::default(),
        };
        let declared =
            |name: &str, fields: Vec<ReflectedFieldDescriptor>| ReflectedVariantDescriptor {
                name: name.to_owned(),
                fields,
                editor_attributes: EditorAttributes::default(),
            };
        let option_path = VARIANT_FIXTURE_OPTION_PATH;
        let mut mode = descriptor("fixture::Mode", ReflectedTypeKind::Enum);
        mode.variants = vec![
            declared("Marker", Vec::new()),
            declared("Pair", vec![field("0", "f32"), field("1", "bool")]),
            declared("Named", vec![field("alpha", "f32"), field("beta", "bool")]),
            declared(
                "Tri",
                vec![field("flag", option_path), field("beta", "bool")],
            ),
        ];
        let mut component = descriptor(VARIANT_FIXTURE_COMPONENT_PATH, ReflectedTypeKind::Struct);
        component.fields.push(field("mode", "fixture::Mode"));
        TypeRegistrySnapshot {
            schema_catalog_hash: Vec::new(),
            types: vec![
                component,
                mode,
                descriptor("bool", ReflectedTypeKind::Bool),
                descriptor(option_path, ReflectedTypeKind::Optional),
                descriptor("f32", ReflectedTypeKind::Float { bits: 32 }),
            ],
        }
    }

    /// The projected value in one declared slot of a variant selection,
    /// asserting the slot keeps its declared position.
    fn variant_slot(selection: &ReflectedVariantSelection, at: usize) -> ReflectedValueNode {
        let ReflectedInspectionChild::TupleElement { index, value } = &selection.fields[at] else {
            panic!("variant fields project as indexed elements")
        };
        assert_eq!(
            *index,
            u32::try_from(at).expect("test slot indices fit in u32"),
            "slots keep the order the variant declares",
        );
        (**value).clone()
    }

    #[test]
    fn a_partially_retained_variant_projects_each_value_onto_its_own_field() {
        let component_path = VARIANT_FIXTURE_COMPONENT_PATH;
        let registry = variant_fixture_registry();
        let selection = |payload: &[u8]| {
            let component = PrefabComponentSnapshot {
                entity_alias: "root".to_owned(),
                type_path: component_path.to_owned(),
                sparse_value: ReflectedValueEnvelope {
                    type_path: component_path.to_owned(),
                    encoding: ReflectedValueEncoding::TypedRon,
                    payload: payload.to_vec(),
                },
            };
            let model = ReflectedInspectionModel::project(ReflectedInspectionInput::new(
                &registry, &component,
            ))
            .expect("project the variant fixture");
            let [ReflectedInspectionChild::Variant(selection)] =
                model.fields[0].value.children.as_slice()
            else {
                panic!("an enum field projects exactly one variant selection")
            };
            selection.clone()
        };
        let slot = variant_slot;
        let path = |variant: &str, field: ReflectedPathSegment| {
            vec![
                ReflectedPathSegment::Field("mode".to_owned()),
                ReflectedPathSegment::Variant(variant.to_owned()),
                field,
            ]
        };
        let named = |name: &str| path("Named", ReflectedPathSegment::Field(name.to_owned()));
        let boolean = |value: bool| ReflectedValue::Scalar(ReflectedScalar::Bool(value));

        // The defect: retaining only the SECOND declared field put its value on
        // the FIRST field's slot, where the type does not even match.
        let second_only = selection(b"(mode:Named(beta:true))");
        assert_eq!(second_only.name, "Named");
        let alpha = slot(&second_only, 0);
        assert_eq!(alpha.type_path, "f32");
        assert_eq!(alpha.binding.target.path.segments, named("alpha"));
        assert_eq!(alpha.current.authored, None, "alpha is not retained");
        assert_eq!(alpha.current.effective, None);
        let beta = slot(&second_only, 1);
        assert_eq!(beta.type_path, "bool");
        assert_eq!(beta.binding.target.path.segments, named("beta"));
        assert_eq!(beta.current.authored, Some(boolean(true)));
        assert_eq!(beta.current.effective, Some(boolean(true)));

        // Retaining only the first declared field is the mirror case.
        let first_only = selection(b"(mode:Named(alpha:1.0))");
        let alpha = slot(&first_only, 0);
        assert_eq!(alpha.binding.target.path.segments, named("alpha"));
        assert_eq!(alpha.current.authored, Some(float("1.0")));
        let beta = slot(&first_only, 1);
        assert_eq!(beta.binding.target.path.segments, named("beta"));
        assert_eq!(beta.current.authored, None, "beta is not retained");

        // Retaining both keeps each value on its own field.
        let both = selection(b"(mode:Named(alpha:1.0,beta:true))");
        assert_eq!(slot(&both, 0).current.authored, Some(float("1.0")));
        assert_eq!(slot(&both, 1).current.authored, Some(boolean(true)));

        // Ticket 046's case stays correct: retaining nothing leaves every
        // declared field absent, with its own binding.
        let none_retained = selection(b"(mode:Named())");
        let alpha = slot(&none_retained, 0);
        assert_eq!(alpha.binding.target.path.segments, named("alpha"));
        assert_eq!(alpha.current.authored, None);
        let beta = slot(&none_retained, 1);
        assert_eq!(beta.binding.target.path.segments, named("beta"));
        assert_eq!(beta.current.authored, None);

        // An omitted `Option` field is absent, not `None`: the tri-state the
        // inspector projects for a struct field holds inside a variant too.
        let absent_option = selection(b"(mode:Tri(beta:true))");
        let flag = slot(&absent_option, 0);
        assert_eq!(flag.type_path, VARIANT_FIXTURE_OPTION_PATH);
        assert_eq!(flag.current.authored, None, "an omitted Option is absent");
        assert_eq!(flag.current.effective, None);
        assert!(flag.children.is_empty());
        let explicit_none = selection(b"(mode:Tri(flag:None,beta:true))");
        let flag = slot(&explicit_none, 0);
        assert_eq!(
            flag.current.authored,
            Some(ReflectedValue::Optional(None)),
            "an explicitly retained None is a value, not absence",
        );

        // Negative control: a tuple-shaped variant reads positionally, under
        // the index names it declares.
        let pair = selection(b"(mode:Pair(1.0,true))");
        let first = slot(&pair, 0);
        assert_eq!(first.type_path, "f32");
        assert_eq!(
            first.binding.target.path.segments,
            path("Pair", ReflectedPathSegment::TupleIndex(0)),
        );
        assert_eq!(first.current.authored, Some(float("1.0")));
        let second = slot(&pair, 1);
        assert_eq!(second.type_path, "bool");
        assert_eq!(
            second.binding.target.path.segments,
            path("Pair", ReflectedPathSegment::TupleIndex(1)),
        );
        assert_eq!(second.current.authored, Some(boolean(true)));

        // Negative control: a unit variant still projects no field at all.
        let marker = selection(b"(mode:Marker)");
        assert_eq!(marker.name, "Marker");
        assert!(marker.fields.is_empty());
    }

    #[test]
    fn optional_projection_preserves_absent_none_and_some() {
        let component_path = "fixture::OptionalComponent";
        let option_path = "core::option::Option<f32>";
        let mut component = descriptor(component_path, ReflectedTypeKind::Struct);
        component.fields.extend([
            ReflectedFieldDescriptor {
                name: "marker".to_owned(),
                type_path: "bool".to_owned(),
                editor_attributes: EditorAttributes::default(),
            },
            ReflectedFieldDescriptor {
                name: "nested".to_owned(),
                type_path: option_path.to_owned(),
                editor_attributes: EditorAttributes::default(),
            },
        ]);
        component.applicability.default_available = true;
        let registry = TypeRegistrySnapshot {
            schema_catalog_hash: Vec::new(),
            types: vec![
                component,
                descriptor("bool", ReflectedTypeKind::Bool),
                descriptor(option_path, ReflectedTypeKind::Optional),
                descriptor("f32", ReflectedTypeKind::Float { bits: 32 }),
            ],
        };
        let component = |payload: &[u8]| PrefabComponentSnapshot {
            entity_alias: "root".to_owned(),
            type_path: component_path.to_owned(),
            sparse_value: ReflectedValueEnvelope {
                type_path: component_path.to_owned(),
                encoding: ReflectedValueEncoding::TypedRon,
                payload: payload.to_vec(),
            },
        };
        let default = component(b"(marker: false, nested: Some(7.0))");
        let project = |payload: &[u8]| {
            let current = component(payload);
            ReflectedInspectionModel::project(
                ReflectedInspectionInput::new(&registry, &current)
                    .with_default(&default.sparse_value),
            )
            .expect("project optional fixture")
        };

        let explicit_none = project(b"(marker: true, nested: None)");
        let node = &explicit_none.fields[1].value;
        assert_eq!(node.current.authored, Some(ReflectedValue::Optional(None)));
        assert_eq!(node.current.effective, Some(ReflectedValue::Optional(None)));
        assert!(node.children.is_empty());

        let absent = project(b"(marker: true)");
        let node = &absent.fields[1].value;
        assert_eq!(node.current.authored, None);
        assert_eq!(node.current.effective, Some(optional_some(float("7.0"))));
        let [ReflectedInspectionChild::OptionalSome(child)] = node.children.as_slice() else {
            panic!("absent Option should project its Some default")
        };
        assert_eq!(child.current.authored, None);
        assert_eq!(child.current.effective, Some(float("7.0")));

        let explicit_some = project(b"(marker: true, nested: Some(2.0))");
        let node = &explicit_some.fields[1].value;
        assert_eq!(node.current.authored, Some(optional_some(float("2.0"))));
        assert_eq!(node.current.effective, Some(optional_some(float("2.0"))));
        let [ReflectedInspectionChild::OptionalSome(child)] = node.children.as_slice() else {
            panic!("explicit Some should project its inner value")
        };
        assert_eq!(child.current.authored, Some(float("2.0")));
        assert_eq!(child.current.effective, Some(float("2.0")));
        assert_eq!(child.default.value, Some(float("7.0")));
    }

    #[test]
    fn reflected_binding_uses_named_paths_and_typed_map_keys() {
        let binding = ReflectedEditBinding::new(PrefabValueTarget {
            instance_alias_chain: Vec::new(),
            entity_alias: "root".to_owned(),
            path: ReflectedPath {
                component_type_path: "example::Component".to_owned(),
                segments: Vec::new(),
            },
        })
        .field("values");
        let key = ReflectedValueEnvelope {
            type_path: "u32".to_owned(),
            encoding: ReflectedValueEncoding::TypedRon,
            payload: b"7".to_vec(),
        };
        let command = ReflectedMapEntryBinding {
            map: binding,
            key: key.clone(),
        }
        .remove();

        assert_eq!(
            command,
            PrefabEditCommand::MapRemove {
                target: PrefabValueTarget {
                    instance_alias_chain: Vec::new(),
                    entity_alias: "root".to_owned(),
                    path: ReflectedPath {
                        component_type_path: "example::Component".to_owned(),
                        segments: vec![ReflectedPathSegment::Field("values".to_owned())],
                    },
                },
                key,
            }
        );
    }

    #[test]
    fn reflected_override_operations_map_to_typed_edit_commands() {
        let target = PrefabValueTarget {
            instance_alias_chain: vec!["instance".to_owned()],
            entity_alias: "root".to_owned(),
            path: ReflectedPath {
                component_type_path: "fixture::Component".to_owned(),
                segments: vec![ReflectedPathSegment::Field("values".to_owned())],
            },
        };
        let value = ReflectedValueEnvelope {
            type_path: "f32".to_owned(),
            encoding: ReflectedValueEncoding::TypedRon,
            payload: b"1.0".to_vec(),
        };
        let snapshots = [
            PrefabOverrideSnapshot {
                operation: PrefabOverrideOperation::Set {
                    target: target.clone(),
                    value: value.clone(),
                },
            },
            PrefabOverrideSnapshot {
                operation: PrefabOverrideOperation::Clear {
                    target: target.clone(),
                },
            },
            PrefabOverrideSnapshot {
                operation: PrefabOverrideOperation::Insert {
                    target: target.clone(),
                    index: 1,
                    value,
                },
            },
            PrefabOverrideSnapshot {
                operation: PrefabOverrideOperation::Remove {
                    target: target.clone(),
                    index: 2,
                },
            },
            PrefabOverrideSnapshot {
                operation: PrefabOverrideOperation::Move {
                    target,
                    from: 3,
                    to: 4,
                },
            },
        ];
        let commands = snapshots
            .iter()
            .map(ReflectedOverrideOperation::project)
            .map(|operation| operation.edit_command())
            .collect::<Vec<_>>();
        assert!(matches!(commands[0], PrefabEditCommand::SetOverride { .. }));
        assert!(matches!(
            commands[1],
            PrefabEditCommand::ClearOverride { .. }
        ));
        assert!(matches!(
            commands[2],
            PrefabEditCommand::InsertOverride { .. }
        ));
        assert!(matches!(
            commands[3],
            PrefabEditCommand::RemoveOverrideItem { .. }
        ));
        assert!(matches!(
            commands[4],
            PrefabEditCommand::MoveOverride { .. }
        ));
    }
}
