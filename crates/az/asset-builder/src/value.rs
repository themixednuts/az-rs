//! Strong value types used by asset-builder descriptors and jobs.
//!
//! These keep the in-process builder API from treating every identifier as
//! a plain string. RPC, DB rows, and manifest files still serialize as text,
//! but builder authors work with domain values.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use az_core::ReflectedValueEnvelope;
use az_gem_contract::{Attributed, Registries, RegistryEntry, Unconditional};
use thiserror::Error;

/// Error returned when an asset-builder value fails its local invariant.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AssetBuilderValueError {
    #[error("{kind} must not be empty")]
    Empty { kind: &'static str },
    #[error("{kind} `{value}` must not have leading or trailing whitespace")]
    Trimmed { kind: &'static str, value: String },
    #[error("{kind} `{value}` must be a safe relative asset path")]
    UnsafeRelativePath { kind: &'static str, value: String },
}

/// Build platform identifier such as `pc` or `server`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct PlatformId(&'static str);

impl PlatformId {
    #[must_use]
    pub const fn new_static(value: &'static str) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for PlatformId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl AsRef<str> for PlatformId {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl PartialEq<&str> for PlatformId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<PlatformId> for &str {
    fn eq(&self, other: &PlatformId) -> bool {
        *self == other.0
    }
}

/// Logical source-root selector for files created under the project source root.
pub const PROJECT_SOURCE_ROOT: &str = "project:source-root";

/// Static authored/source schema type name claimed by a builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct SourceSchemaType(&'static str);

impl SourceSchemaType {
    #[must_use]
    #[doc(hidden)]
    pub const fn __from_static(value: &'static str) -> Self {
        Self(value)
    }

    /// Construct a source schema id at a wire/runtime boundary.
    ///
    /// Builder authoring should use `SourceFormat` descriptors and
    /// `SourceSchemaRegistration::for_source`; this escape hatch is for
    /// runtime-composed registries that have already obtained a stable
    /// process-lifetime schema string.
    #[must_use]
    pub const fn from_dynamic_parts(value: &'static str) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for SourceSchemaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl AsRef<str> for SourceSchemaType {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl PartialEq<&str> for SourceSchemaType {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<SourceSchemaType> for &str {
    fn eq(&self, other: &SourceSchemaType) -> bool {
        *self == other.0
    }
}

/// Metadata for one source/dev schema contributed by an engine, project, or
/// gem crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSchemaRegistration {
    schema_type: SourceSchemaType,
    source_patterns: &'static [crate::AssetBuilderPattern],
    label: &'static str,
    category: &'static str,
    authoring: SourceSchemaAuthoring,
}

/// Editor-visible workflow for a source schema backed by a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFileWorkflow {
    source_root: &'static str,
    default_path_prefix: &'static str,
    extensions: &'static [&'static str],
    can_create: bool,
    can_edit: bool,
}

impl SourceFileWorkflow {
    /// Register a source-file workflow. `default_path_prefix` is an asset-tree
    /// relative placement hint; an empty prefix means the source root itself.
    /// Extensions are written without a leading dot. Use `"*"` only for an
    /// explicit catch-all import family.
    #[must_use]
    pub const fn new(
        default_path_prefix: &'static str,
        extensions: &'static [&'static str],
    ) -> Self {
        Self::new_in_source_root(PROJECT_SOURCE_ROOT, default_path_prefix, extensions)
    }

    /// Register a source-file workflow rooted at a specific project graph
    /// source-root selector, such as `project:source-root` or
    /// `gem:azoth.physics:assets`.
    #[must_use]
    pub const fn new_in_source_root(
        source_root: &'static str,
        default_path_prefix: &'static str,
        extensions: &'static [&'static str],
    ) -> Self {
        Self {
            source_root,
            default_path_prefix,
            extensions,
            can_create: false,
            can_edit: false,
        }
    }

    /// Mark this file workflow as editor-creatable.
    #[must_use]
    pub const fn creatable(mut self) -> Self {
        self.can_create = true;
        self
    }

    /// Mark this file workflow as editable by a structured editor.
    #[must_use]
    pub const fn editable(mut self) -> Self {
        self.can_edit = true;
        self
    }

    /// Override the workflow's source-root selector.
    #[must_use]
    pub const fn in_source_root(mut self, source_root: &'static str) -> Self {
        self.source_root = source_root;
        self
    }

    #[must_use]
    pub const fn source_root(self) -> &'static str {
        self.source_root
    }

    #[must_use]
    pub const fn default_path_prefix(self) -> &'static str {
        self.default_path_prefix
    }

    #[must_use]
    pub const fn extensions(self) -> &'static [&'static str] {
        self.extensions
    }

    #[must_use]
    pub const fn can_create(self) -> bool {
        self.can_create
    }

    #[must_use]
    pub const fn can_edit(self) -> bool {
        self.can_edit
    }
}

/// Editor/source workflow supported by a registered source schema.
///
/// Product bytes alone are not enough for editor UX. A source schema must say
/// whether users create it through a project-host authored document, or whether
/// it is a file workflow with explicit create/edit/import capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceSchemaAuthoring {
    /// The editor can create this source by asking project-host to create the
    /// named document schema. The schema catalog owns the exact placement hint
    /// and default value.
    ProjectDocument { schema_type: &'static str },
    /// This source family is represented as a file source. The workflow
    /// metadata is the editor-visible create/edit/import contract for raw,
    /// legacy, generated, and typed source-file formats.
    File { workflow: SourceFileWorkflow },
}

/// Request passed to a project/gem-owned source-file template factory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFileTemplateRequest<'a> {
    pub schema_type: SourceSchemaType,
    pub source_path: &'a str,
}

/// Error returned by a source-file template factory.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceFileTemplateError {
    #[error("source file template is not supported for this request: {reason}")]
    Unsupported { reason: String },
    #[error("source file template failed: {reason}")]
    Failed { reason: String },
}

impl SourceFileTemplateError {
    #[must_use]
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::Unsupported {
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn failed(reason: impl Into<String>) -> Self {
        Self::Failed {
            reason: reason.into(),
        }
    }
}

pub type SourceFileTemplateResult = Result<Vec<u8>, SourceFileTemplateError>;

/// Factory for the initial bytes of an editor-created source file.
///
/// This belongs next to source-schema/build-rule registration because the
/// engine/editor should not hardcode project-specific file shapes.
pub type SourceFileTemplateFn = fn(&SourceFileTemplateRequest<'_>) -> SourceFileTemplateResult;

/// Host-composed state available to one source-file codec invocation.
///
/// Codec callbacks remain plain function pointers. Project and gem type
/// registrations therefore travel explicitly from the active worker instead
/// of being captured from a process-global or engine-only registry.
#[derive(Clone, Copy)]
pub struct SourceFileEditCodecContext<'a> {
    registries: &'a Registries,
}

impl<'a> SourceFileEditCodecContext<'a> {
    #[must_use]
    pub const fn new(registries: &'a Registries) -> Self {
        Self { registries }
    }

    #[must_use]
    pub const fn registries(&self) -> &'a Registries {
        self.registries
    }

    #[must_use]
    pub fn registry<T: RegistryEntry>(&self) -> Option<&'a az_gem_contract::Registry<T>> {
        self.registries.get::<T>()
    }
}

impl fmt::Debug for SourceFileEditCodecContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceFileEditCodecContext")
            .finish_non_exhaustive()
    }
}

/// Request passed to a project/gem-owned source-file edit codec when loading
/// an existing source file into the structured editor value tree.
#[derive(Debug, Clone, Copy)]
pub struct SourceFileEditLoadRequest<'a> {
    pub schema_type: SourceSchemaType,
    pub source_path: &'a str,
    pub bytes: &'a [u8],
}

/// One authored object returned by a source-file edit codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileEditObject {
    pub object_id: String,
    pub schema: String,
    pub value: ReflectedValueEnvelope,
}

impl SourceFileEditObject {
    #[must_use]
    pub fn new(
        object_id: impl Into<String>,
        schema: impl Into<String>,
        value: ReflectedValueEnvelope,
    ) -> Self {
        Self {
            object_id: object_id.into(),
            schema: schema.into(),
            value,
        }
    }
}

/// Structured value returned by a source-file edit codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileEditDocument {
    pub root_object_id: Option<String>,
    pub root_schema: String,
    pub value: ReflectedValueEnvelope,
    pub objects: Vec<SourceFileEditObject>,
    /// Opaque codec-owned bytes preserved across the editor load/save cycle.
    pub codec_state: Vec<u8>,
}

impl SourceFileEditDocument {
    #[must_use]
    pub fn new(root_schema: impl Into<String>, value: ReflectedValueEnvelope) -> Self {
        let root_schema = root_schema.into();
        Self {
            root_object_id: None,
            root_schema: root_schema.clone(),
            value: value.clone(),
            objects: vec![SourceFileEditObject::new("", root_schema, value)],
            codec_state: Vec::new(),
        }
    }

    #[must_use]
    pub fn empty(root_schema: impl Into<String>) -> Self {
        let root_schema = root_schema.into();
        Self {
            root_object_id: Some(String::new()),
            value: ReflectedValueEnvelope::typed_ron(root_schema.clone(), "()"),
            root_schema,
            objects: Vec::new(),
            codec_state: Vec::new(),
        }
    }

    #[must_use]
    pub fn object(
        object_id: impl Into<String>,
        schema: impl Into<String>,
        value: ReflectedValueEnvelope,
    ) -> SourceFileEditObject {
        SourceFileEditObject::new(object_id, schema, value)
    }

    /// Construct a document from an explicit authored-object list.
    ///
    /// The root object id must name exactly one object in `objects`; that root
    /// keeps `root_schema` and `value` populated for single-root codecs.
    ///
    /// # Errors
    ///
    /// Returns an error when the object list is empty, has an empty or
    /// duplicate object id, or does not contain the requested root object.
    pub fn from_objects(
        root_object_id: impl Into<String>,
        objects: impl IntoIterator<Item = SourceFileEditObject>,
    ) -> SourceFileEditLoadResult {
        let root_object_id = root_object_id.into();
        if root_object_id.trim().is_empty() {
            return Err(SourceFileEditError::failed(
                "source-file edit document root object id must not be empty",
            ));
        }

        let objects = objects.into_iter().collect::<Vec<_>>();
        if objects.is_empty() {
            return Err(SourceFileEditError::failed(
                "source-file edit document must contain at least one object",
            ));
        }

        let mut object_ids = BTreeSet::new();
        let mut root = None;
        for object in &objects {
            if object.object_id.trim().is_empty() {
                return Err(SourceFileEditError::failed(
                    "source-file edit document object id must not be empty",
                ));
            }
            if !object_ids.insert(object.object_id.as_str()) {
                return Err(SourceFileEditError::failed(format!(
                    "source-file edit document contains duplicate object id `{}`",
                    object.object_id
                )));
            }
            if object.object_id == root_object_id {
                root = Some(object);
            }
        }

        let Some(root) = root else {
            return Err(SourceFileEditError::failed(format!(
                "source-file edit document root object `{root_object_id}` was not found"
            )));
        };

        Ok(Self {
            root_object_id: Some(root_object_id),
            root_schema: root.schema.clone(),
            value: root.value.clone(),
            objects,
            codec_state: Vec::new(),
        })
    }

    /// Attach opaque state that the codec needs when saving the document.
    #[must_use]
    pub fn with_codec_state(mut self, state: impl Into<Vec<u8>>) -> Self {
        self.codec_state = state.into();
        self
    }
}

/// Request passed to a project/gem-owned source-file edit codec when saving the
/// structured editor value tree back to source bytes.
#[derive(Debug, Clone, Copy)]
pub struct SourceFileEditSaveRequest<'a> {
    pub schema_type: SourceSchemaType,
    pub source_path: &'a str,
    pub root_schema: &'a str,
    pub value: &'a ReflectedValueEnvelope,
    pub objects: &'a [SourceFileEditObject],
    pub codec_state: &'a [u8],
}

impl<'a> SourceFileEditSaveRequest<'a> {
    #[must_use]
    pub const fn new(
        schema_type: SourceSchemaType,
        source_path: &'a str,
        root_schema: &'a str,
        value: &'a ReflectedValueEnvelope,
        objects: &'a [SourceFileEditObject],
        codec_state: &'a [u8],
    ) -> Self {
        Self {
            schema_type,
            source_path,
            root_schema,
            value,
            objects,
            codec_state,
        }
    }

    #[must_use]
    pub const fn objects(&self) -> &'a [SourceFileEditObject] {
        self.objects
    }
}

/// Error returned by a source-file edit codec.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceFileEditError {
    #[error("source file edit codec is not supported for this request: {reason}")]
    Unsupported { reason: String },
    #[error("source file edit codec failed: {reason}")]
    Failed { reason: String },
}

impl SourceFileEditError {
    #[must_use]
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::Unsupported {
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn failed(reason: impl Into<String>) -> Self {
        Self::Failed {
            reason: reason.into(),
        }
    }
}

pub type SourceFileEditLoadResult = Result<SourceFileEditDocument, SourceFileEditError>;
pub type SourceFileEditSaveResult = Result<Vec<u8>, SourceFileEditError>;

/// A structural operation requested against a source-file edit document.
///
/// The source-schema codec owns the meaning of every operation: its default
/// value, object identity, ordering, and whether a particular object can be
/// removed. The engine deliberately does not apply a generic list edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFileEditOperation {
    AppendDefault,
    Duplicate { object_id: String },
    Remove { object_id: String },
}

/// Request passed to a source-file edit codec for a structural document edit.
#[derive(Debug, Clone, Copy)]
pub struct SourceFileEditOperationRequest<'a> {
    /// The source schema that selected this codec.
    pub schema_type: SourceSchemaType,
    pub source_path: &'a str,
    pub document: &'a SourceFileEditDocument,
    pub operation: &'a SourceFileEditOperation,
}

impl<'a> SourceFileEditOperationRequest<'a> {
    #[must_use]
    pub const fn new(
        schema_type: SourceSchemaType,
        source_path: &'a str,
        document: &'a SourceFileEditDocument,
        operation: &'a SourceFileEditOperation,
    ) -> Self {
        Self {
            schema_type,
            source_path,
            document,
            operation,
        }
    }
}

pub type SourceFileEditOperationResult = Result<SourceFileEditDocument, SourceFileEditError>;

/// Function invoked by an authoring-capable asset worker to parse source bytes
/// into an editable schema value tree.
pub type SourceFileEditLoadFn =
    fn(SourceFileEditCodecContext<'_>, &SourceFileEditLoadRequest<'_>) -> SourceFileEditLoadResult;

/// Function invoked by an authoring-capable asset worker to serialize an
/// edited schema value tree back to source bytes.
pub type SourceFileEditSaveFn =
    fn(SourceFileEditCodecContext<'_>, &SourceFileEditSaveRequest<'_>) -> SourceFileEditSaveResult;

/// Function invoked by the editor through a schema's edit codec for a
/// structural document operation.
pub type SourceFileEditOperationFn = fn(
    SourceFileEditCodecContext<'_>,
    &SourceFileEditOperationRequest<'_>,
) -> SourceFileEditOperationResult;

/// Editor-visible create target supplied by a source-file template owner.
///
/// Some file-backed source schemas are concrete one-off formats, while others
/// are families. A `GameData` table source is the latter: one schema, many
/// generated table paths. Candidates let the project/gem publish those concrete
/// paths without teaching the editor the game-specific registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileTemplateCandidate {
    pub source_path: String,
    pub label: String,
    pub description: String,
}

impl SourceFileTemplateCandidate {
    #[must_use]
    pub fn new(source_path: impl AsRef<str>) -> Self {
        Self {
            source_path: az_filesystem::SourcePath::new(source_path).into_string(),
            label: String::new(),
            description: String::new(),
        }
    }

    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}

pub type SourceFileTemplateCandidatesFn = fn() -> Vec<SourceFileTemplateCandidate>;

#[derive(Clone, Copy)]
pub struct SourceFileTemplateRegistration {
    schema_type: SourceSchemaType,
    create: SourceFileTemplateFn,
    candidates: Option<SourceFileTemplateCandidatesFn>,
}

#[derive(Clone, Copy)]
pub struct SourceFileEditCodecRegistration {
    schema_type: SourceSchemaType,
    load: SourceFileEditLoadFn,
    save: SourceFileEditSaveFn,
    edit: Option<SourceFileEditOperationFn>,
}

impl SourceFileTemplateRegistration {
    /// Register a source template from its typed source descriptor.
    ///
    /// # Panics
    ///
    /// Panics when `T::SCHEMA` is `None`; path-only source formats have no
    /// schema identity to register a template against.
    #[must_use]
    pub const fn for_source<T: crate::SourceFormat>(create: SourceFileTemplateFn) -> Self {
        let Some(schema_type) = T::SCHEMA else {
            panic!("SourceFileTemplateRegistration::for_source requires SourceFormat::SCHEMA");
        };
        Self::from_dynamic_parts(schema_type, create)
    }

    /// Register a source template at a runtime/wire boundary.
    ///
    /// Normal builder authoring should use [`Self::for_source`].
    #[must_use]
    pub const fn from_dynamic_parts(
        schema_type: SourceSchemaType,
        create: SourceFileTemplateFn,
    ) -> Self {
        Self {
            schema_type,
            create,
            candidates: None,
        }
    }

    #[must_use]
    pub const fn with_candidates(mut self, candidates: SourceFileTemplateCandidatesFn) -> Self {
        self.candidates = Some(candidates);
        self
    }

    #[must_use]
    pub const fn schema_type(self) -> SourceSchemaType {
        self.schema_type
    }

    /// # Errors
    ///
    /// Forwards errors from the registered template function.
    pub fn create(self, request: &SourceFileTemplateRequest<'_>) -> SourceFileTemplateResult {
        (self.create)(request)
    }

    #[must_use]
    pub fn candidates(self) -> Vec<SourceFileTemplateCandidate> {
        self.candidates
            .map_or_else(Vec::new, |candidates| candidates())
    }
}

impl SourceFileEditCodecRegistration {
    /// Register a source edit codec from its typed source descriptor.
    ///
    /// # Panics
    ///
    /// Panics when `T::SCHEMA` is `None`; path-only source formats have no
    /// schema identity to register an edit codec against.
    #[must_use]
    pub const fn for_source<T: crate::SourceFormat>(
        load: SourceFileEditLoadFn,
        save: SourceFileEditSaveFn,
    ) -> Self {
        let Some(schema_type) = T::SCHEMA else {
            panic!("SourceFileEditCodecRegistration::for_source requires SourceFormat::SCHEMA");
        };
        Self::from_dynamic_parts(schema_type, load, save)
    }

    /// Register a source edit codec at a runtime/wire boundary.
    ///
    /// Normal builder authoring should use [`Self::for_source`].
    #[must_use]
    pub const fn from_dynamic_parts(
        schema_type: SourceSchemaType,
        load: SourceFileEditLoadFn,
        save: SourceFileEditSaveFn,
    ) -> Self {
        Self {
            schema_type,
            load,
            save,
            edit: None,
        }
    }

    /// Add structural editing owned by this source schema's codec.
    ///
    /// The callback, rather than the editor, creates defaults and determines
    /// object ids, ordering, and deletion rules.
    #[must_use]
    pub const fn with_edit(mut self, edit: SourceFileEditOperationFn) -> Self {
        self.edit = Some(edit);
        self
    }

    #[must_use]
    pub const fn schema_type(self) -> SourceSchemaType {
        self.schema_type
    }

    /// # Errors
    ///
    /// Forwards errors from the registered edit-codec load function.
    pub fn load(
        self,
        context: SourceFileEditCodecContext<'_>,
        request: &SourceFileEditLoadRequest<'_>,
    ) -> SourceFileEditLoadResult {
        (self.load)(context, request)
    }

    /// # Errors
    ///
    /// Forwards errors from the registered edit-codec save function.
    pub fn save(
        self,
        context: SourceFileEditCodecContext<'_>,
        request: &SourceFileEditSaveRequest<'_>,
    ) -> SourceFileEditSaveResult {
        (self.save)(context, request)
    }

    /// Apply one structural operation through this schema's codec.
    ///
    /// # Errors
    ///
    /// Returns [`SourceFileEditError::Unsupported`] when the request names a
    /// different source schema or this codec does not publish structural
    /// editing. Otherwise forwards the codec-owned operation error.
    pub fn edit(
        self,
        context: SourceFileEditCodecContext<'_>,
        request: &SourceFileEditOperationRequest<'_>,
    ) -> SourceFileEditOperationResult {
        if request.schema_type != self.schema_type {
            return Err(SourceFileEditError::unsupported(format!(
                "source-file edit operation schema `{}` does not match codec schema `{}`",
                request.schema_type, self.schema_type
            )));
        }
        let Some(edit) = self.edit else {
            return Err(SourceFileEditError::unsupported(format!(
                "source-file edit codec for schema `{}` does not support structural edits",
                self.schema_type
            )));
        };
        edit(context, request)
    }
}

impl SourceSchemaRegistration {
    /// Register a source schema from its typed descriptor.
    ///
    /// # Panics
    ///
    /// Panics when `T::SCHEMA` is `None`; path-only source formats carry no
    /// schema identity and must not be registered as source schemas.
    #[must_use]
    pub const fn for_source<T: crate::SourceFormat>() -> Self {
        let Some(schema_type) = T::SCHEMA else {
            panic!("SourceSchemaRegistration::for_source requires SourceFormat::SCHEMA");
        };
        Self {
            source_patterns: T::PATTERNS,
            ..Self::from_dynamic_parts(schema_type)
        }
    }

    /// Register a source schema at a runtime/wire boundary.
    ///
    /// Normal builder authoring should use [`Self::for_source`].
    #[must_use]
    pub const fn from_dynamic_parts(schema_type: SourceSchemaType) -> Self {
        Self {
            schema_type,
            source_patterns: &[],
            label: "",
            category: "",
            authoring: SourceSchemaAuthoring::File {
                workflow: SourceFileWorkflow::new("", &[]),
            },
        }
    }

    /// Record the crate, schema module, or importer that owns this source
    /// schema.
    /// Human-facing label for source/workflow palettes. Empty means callers may
    /// derive a label from the schema type.
    #[must_use]
    pub const fn with_label(mut self, label: &'static str) -> Self {
        self.label = label;
        self
    }

    /// Human-facing grouping for source/workflow palettes.
    #[must_use]
    pub const fn with_category(mut self, category: &'static str) -> Self {
        self.category = category;
        self
    }

    /// Mark this source as editor-creatable through the project-host authored
    /// document schema catalog.
    #[must_use]
    pub const fn with_creatable_document_schema(mut self, schema_type: &'static str) -> Self {
        self.authoring = SourceSchemaAuthoring::ProjectDocument { schema_type };
        self
    }

    /// Mark this source as an explicit imported/copied source-file family.
    #[must_use]
    pub const fn with_import_file(
        self,
        default_path_prefix: &'static str,
        extensions: &'static [&'static str],
    ) -> Self {
        self.with_import_file_in_source_root(PROJECT_SOURCE_ROOT, default_path_prefix, extensions)
    }

    /// Mark this source as an explicit imported/copied source-file family
    /// rooted at a project graph source-root selector.
    #[must_use]
    pub const fn with_import_file_in_source_root(
        mut self,
        source_root: &'static str,
        default_path_prefix: &'static str,
        extensions: &'static [&'static str],
    ) -> Self {
        self.authoring = SourceSchemaAuthoring::File {
            workflow: SourceFileWorkflow::new_in_source_root(
                source_root,
                default_path_prefix,
                extensions,
            ),
        };
        self
    }

    /// Mark this source as imported/copied and editable by a structured editor
    /// once it has been imported.
    #[must_use]
    pub const fn with_editable_file(
        self,
        default_path_prefix: &'static str,
        extensions: &'static [&'static str],
    ) -> Self {
        self.with_editable_file_in_source_root(PROJECT_SOURCE_ROOT, default_path_prefix, extensions)
    }

    /// Mark this source as imported/copied and editable under a specific
    /// project graph source-root selector.
    #[must_use]
    pub const fn with_editable_file_in_source_root(
        mut self,
        source_root: &'static str,
        default_path_prefix: &'static str,
        extensions: &'static [&'static str],
    ) -> Self {
        self.authoring = SourceSchemaAuthoring::File {
            workflow: SourceFileWorkflow::new_in_source_root(
                source_root,
                default_path_prefix,
                extensions,
            )
            .editable(),
        };
        self
    }

    /// Mark this source as a file that the editor can create and edit directly.
    #[must_use]
    pub const fn with_creatable_file(
        self,
        default_path_prefix: &'static str,
        extensions: &'static [&'static str],
    ) -> Self {
        self.with_creatable_file_in_source_root(
            PROJECT_SOURCE_ROOT,
            default_path_prefix,
            extensions,
        )
    }

    /// Mark this source as a file that the editor can create and edit directly
    /// under a specific project graph source-root selector.
    #[must_use]
    pub const fn with_creatable_file_in_source_root(
        mut self,
        source_root: &'static str,
        default_path_prefix: &'static str,
        extensions: &'static [&'static str],
    ) -> Self {
        self.authoring = SourceSchemaAuthoring::File {
            workflow: SourceFileWorkflow::new_in_source_root(
                source_root,
                default_path_prefix,
                extensions,
            )
            .creatable()
            .editable(),
        };
        self
    }

    #[must_use]
    pub const fn schema_type(self) -> SourceSchemaType {
        self.schema_type
    }

    /// Source-format patterns used to classify this schema without decoding
    /// its typed payload.
    #[must_use]
    pub const fn source_patterns(self) -> &'static [crate::AssetBuilderPattern] {
        self.source_patterns
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        self.label
    }

    #[must_use]
    pub const fn category(self) -> &'static str {
        self.category
    }

    #[must_use]
    pub const fn authoring(self) -> SourceSchemaAuthoring {
        self.authoring
    }
}

/// One source schema per identity: two contributions claiming
/// `azoth.project.Prefab` would give the editor two incompatible authoring
/// contracts for one document type, and the old first-match lookup picked
/// whichever the linker ordered first.
impl RegistryEntry for SourceSchemaRegistration {
    type Key = SourceSchemaType;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "source-schema"
    }

    fn key(&self) -> Self::Key {
        self.schema_type
    }
}

/// One template per schema. A schema offering several starting points
/// expresses that through [`SourceFileTemplateRegistration::with_candidates`],
/// not through a second registration.
impl RegistryEntry for SourceFileTemplateRegistration {
    type Key = SourceSchemaType;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "source-file-template"
    }

    fn key(&self) -> Self::Key {
        self.schema_type
    }
}

/// One codec per schema: the load/save pair is the schema's edit contract, so
/// a second one is an ambiguity, not an override.
impl RegistryEntry for SourceFileEditCodecRegistration {
    type Key = SourceSchemaType;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "source-file-edit-codec"
    }

    fn key(&self) -> Self::Key {
        self.schema_type
    }
}

/// Composed source schemas in deterministic order, each with the
/// contribution that owns it.
#[must_use]
pub fn composed_source_schemas(
    registries: &Registries,
) -> Vec<Attributed<SourceSchemaRegistration>> {
    sorted_by_schema(registries, SourceSchemaRegistration::schema_type)
}

/// Composed source-file templates in deterministic order.
#[must_use]
pub fn composed_source_file_templates(
    registries: &Registries,
) -> Vec<Attributed<SourceFileTemplateRegistration>> {
    sorted_by_schema(registries, SourceFileTemplateRegistration::schema_type)
}

/// The composed source-file template for one source schema type.
#[must_use]
pub fn composed_source_file_template(
    registries: &Registries,
    schema_type: SourceSchemaType,
) -> Option<Attributed<SourceFileTemplateRegistration>> {
    composed_source_file_templates(registries)
        .into_iter()
        .find(|attributed| attributed.entry.schema_type == schema_type)
}

/// Composed source-file edit codecs in deterministic order.
#[must_use]
pub fn composed_source_file_edit_codecs(
    registries: &Registries,
) -> Vec<Attributed<SourceFileEditCodecRegistration>> {
    sorted_by_schema(registries, SourceFileEditCodecRegistration::schema_type)
}

/// The composed source-file edit codec for one source schema type.
#[must_use]
pub fn composed_source_file_edit_codec(
    registries: &Registries,
    schema_type: SourceSchemaType,
) -> Option<Attributed<SourceFileEditCodecRegistration>> {
    composed_source_file_edit_codecs(registries)
        .into_iter()
        .find(|attributed| attributed.entry.schema_type == schema_type)
}

/// Composed entries of one schema-keyed registry, ordered by schema type.
///
/// Composition order is contribution order, which is stable but says nothing
/// about the family; every consumer of these three registries wants them
/// keyed, so they are sorted once here.
fn sorted_by_schema<T>(
    registries: &Registries,
    schema_type: fn(T) -> SourceSchemaType,
) -> Vec<Attributed<T>>
where
    T: RegistryEntry + Copy,
{
    let mut registrations = registries
        .get::<T>()
        .map(|registry| registry.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    registrations.sort_by_key(|attributed| schema_type(attributed.entry));
    registrations
}

/// Registered byte-format identifier for a build product.
///
/// This describes the product's serialized byte contract, not the runtime
/// asset meaning. For example, two products can share one `AssetType` while
/// using different product formats across dev cache and packaged output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct ProductFormatId(&'static str);

impl ProductFormatId {
    #[must_use]
    #[doc(hidden)]
    pub const fn __from_static(value: &'static str) -> Self {
        Self(value)
    }

    /// Construct a product format id at a wire/runtime boundary.
    ///
    /// Builder authoring should use `ProductFormat` descriptors and
    /// `ProductFormatRegistration::for_format`.
    #[must_use]
    pub const fn from_dynamic_parts(value: &'static str) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for ProductFormatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl AsRef<str> for ProductFormatId {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl PartialEq<&str> for ProductFormatId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<ProductFormatId> for &str {
    fn eq(&self, other: &ProductFormatId) -> bool {
        *self == other.0
    }
}

/// Metadata for one product byte format contributed by an engine, project, or
/// gem crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductFormatRegistration {
    id: ProductFormatId,
    current_version: u32,
}

impl ProductFormatRegistration {
    /// Register a product byte format from its typed descriptor.
    #[must_use]
    pub const fn for_format<T: crate::ProductFormat>() -> Self {
        Self::from_dynamic_parts(T::ID, T::VERSION)
    }

    /// Register a product byte format at a runtime/wire boundary.
    ///
    /// Normal builder authoring should use [`Self::for_format`].
    #[must_use]
    pub const fn from_dynamic_parts(id: ProductFormatId, current_version: u32) -> Self {
        Self {
            id,
            current_version,
        }
    }
    #[must_use]
    pub const fn id(self) -> ProductFormatId {
        self.id
    }

    #[must_use]
    pub const fn current_version(self) -> u32 {
        self.current_version
    }
}

/// One byte contract per format id. The id names the encoding, and the
/// current writer version travels with it, so a second registration under one
/// id means two crates disagree about how those bytes are written — the old
/// first-match lookup resolved that by link order.
impl RegistryEntry for ProductFormatRegistration {
    type Key = ProductFormatId;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "product-format"
    }

    fn key(&self) -> Self::Key {
        self.id
    }
}

/// Composed product byte formats in deterministic order.
#[must_use]
pub fn composed_product_formats(
    registries: &Registries,
) -> Vec<Attributed<ProductFormatRegistration>> {
    let mut registrations = registries
        .get::<ProductFormatRegistration>()
        .map(|registry| registry.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    registrations.sort_by_key(|attributed| attributed.entry.id());
    registrations
}

/// The composed product byte format with this typed id.
#[must_use]
pub fn composed_product_format(
    registries: &Registries,
    id: ProductFormatId,
) -> Option<Attributed<ProductFormatRegistration>> {
    composed_product_formats(registries)
        .into_iter()
        .find(|attributed| attributed.entry.id == id)
}

/// The composed product byte format with this runtime/string id.
#[must_use]
pub fn composed_product_format_by_id(
    registries: &Registries,
    id: &str,
) -> Option<Attributed<ProductFormatRegistration>> {
    composed_product_formats(registries)
        .into_iter()
        .find(|attributed| attributed.entry.id.as_str() == id)
}

/// Job key within a builder/platform pair.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct JobKey(String);

impl JobKey {
    /// # Errors
    ///
    /// Returns [`AssetBuilderValueError`] when `value` is empty.
    pub fn new(value: impl Into<String>) -> Result<Self, AssetBuilderValueError> {
        let value = value.into();
        validate_required_text("job key", &value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn default_key() -> Self {
        Self("default".to_string())
    }

    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for JobKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for JobKey {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<&str> for JobKey {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<JobKey> for &str {
    fn eq(&self, other: &JobKey) -> bool {
        *self == other.as_str()
    }
}

/// Product path relative to a platform cache root.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct ProductPath(String);

impl ProductPath {
    /// # Errors
    ///
    /// Returns [`AssetBuilderValueError`] when the normalized `value` is empty
    /// or not asset-relative.
    pub fn new(value: impl AsRef<str>) -> Result<Self, AssetBuilderValueError> {
        let value = az_filesystem::normalize_source_path(value.as_ref());
        validate_asset_relative_path("product path", &value)?;
        Ok(Self(value))
    }

    /// Construct a product path when the caller has already validated the
    /// source invariant. Use this for generated/static builder paths; prefer
    /// [`Self::new`] when accepting user or external input.
    ///
    /// # Panics
    ///
    /// Panics if `value` violates asset-builder path invariants.
    #[must_use]
    pub fn from_trusted(value: impl AsRef<str>) -> Self {
        let value = value.as_ref();
        Self::new(value).unwrap_or_else(|error| {
            panic!("trusted product path `{value}` violated asset-builder invariants: {error}")
        })
    }

    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for ProductPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for ProductPath {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<Path> for ProductPath {
    #[inline]
    fn as_ref(&self) -> &Path {
        Path::new(self.as_str())
    }
}

impl PartialEq<&str> for ProductPath {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<ProductPath> for &str {
    fn eq(&self, other: &ProductPath) -> bool {
        *self == other.as_str()
    }
}

fn validate_required_text(kind: &'static str, value: &str) -> Result<(), AssetBuilderValueError> {
    if value.is_empty() {
        return Err(AssetBuilderValueError::Empty { kind });
    }
    if value.trim() != value {
        return Err(AssetBuilderValueError::Trimmed {
            kind,
            value: value.to_string(),
        });
    }
    Ok(())
}

fn validate_asset_relative_path(
    kind: &'static str,
    value: &str,
) -> Result<(), AssetBuilderValueError> {
    validate_required_text(kind, value)?;
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains(':')
    {
        return Err(AssetBuilderValueError::UnsafeRelativePath {
            kind,
            value: value.to_string(),
        });
    }

    for component in value.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(AssetBuilderValueError::UnsafeRelativePath {
                kind,
                value: value.to_string(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use az_gem_contract::{
        ComposeError, Composer, Contribution, ContributionDescriptor, ContributionId, GemContext,
        GemId, GemTargetRole, ProductActivation, declare_caps,
    };

    use super::*;

    const TEST_SOURCE_SCHEMA: SourceSchemaType =
        SourceSchemaType::__from_static("az.test.SourceSchema");

    struct TestSourceFormat;

    impl crate::SourceFormat for TestSourceFormat {
        const SCHEMA: Option<SourceSchemaType> = Some(TEST_SOURCE_SCHEMA);
        const SCHEMA_TYPES: &'static [SourceSchemaType] = &[TEST_SOURCE_SCHEMA];
        const PATTERNS: &'static [crate::AssetBuilderPattern] = &[];
    }

    fn source_schema() -> SourceSchemaRegistration {
        SourceSchemaRegistration::for_source::<TestSourceFormat>()
            .with_label("Test Source")
            .with_category("Tests")
            .with_creatable_document_schema("az.test.SourceSchema")
    }

    // Registered as a `SourceFileTemplateFn` function pointer, so the `Result`
    // return is fixed by that type alias; dropping it would not compile.
    #[allow(clippy::unnecessary_wraps)]
    fn source_template(request: &SourceFileTemplateRequest<'_>) -> SourceFileTemplateResult {
        Ok(format!(
            "schema={};path={}\n",
            request.schema_type, request.source_path
        )
        .into_bytes())
    }

    fn source_template_candidates() -> Vec<SourceFileTemplateCandidate> {
        vec![
            SourceFileTemplateCandidate::new("tests/example.ron")
                .with_label("Example")
                .with_description("Example source template"),
        ]
    }

    fn source_file_template() -> SourceFileTemplateRegistration {
        SourceFileTemplateRegistration::for_source::<TestSourceFormat>(source_template)
            .with_candidates(source_template_candidates)
    }

    fn source_edit_load(
        _context: SourceFileEditCodecContext<'_>,
        request: &SourceFileEditLoadRequest<'_>,
    ) -> SourceFileEditLoadResult {
        let value = String::from_utf8(request.bytes.to_vec())
            .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
        Ok(SourceFileEditDocument::new(
            request.schema_type.as_str(),
            ReflectedValueEnvelope::typed_ron("alloc::string::String", format!("{value:?}")),
        ))
    }

    fn source_edit_save(
        _context: SourceFileEditCodecContext<'_>,
        request: &SourceFileEditSaveRequest<'_>,
    ) -> SourceFileEditSaveResult {
        if request.value.type_path != "alloc::string::String" {
            return Err(SourceFileEditError::failed(
                "expected reflected String value",
            ));
        }
        let payload = std::str::from_utf8(&request.value.payload)
            .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
        let value = payload
            .strip_prefix('"')
            .and_then(|payload| payload.strip_suffix('"'))
            .ok_or_else(|| SourceFileEditError::failed("expected quoted String RON payload"))?;
        Ok(format!(
            "schema={};root={};path={};value={value}",
            request.schema_type, request.root_schema, request.source_path
        )
        .into_bytes())
    }

    fn source_edit_operation(
        _context: SourceFileEditCodecContext<'_>,
        request: &SourceFileEditOperationRequest<'_>,
    ) -> SourceFileEditOperationResult {
        let root_object_id = request
            .document
            .root_object_id
            .as_deref()
            .ok_or_else(|| SourceFileEditError::failed("test document has no root object"))?;
        let mut objects = request.document.objects.clone();
        match request.operation {
            SourceFileEditOperation::AppendDefault => {
                objects.push(SourceFileEditObject::new(
                    format!("generated-{}", objects.len() + 1),
                    &request.document.root_schema,
                    ReflectedValueEnvelope::typed_ron("alloc::string::String", r#""default""#),
                ));
            }
            SourceFileEditOperation::Duplicate { object_id } => {
                let index = objects
                    .iter()
                    .position(|object| object.object_id == *object_id)
                    .ok_or_else(|| {
                        SourceFileEditError::failed(format!(
                            "test document has no object `{object_id}` to duplicate"
                        ))
                    })?;
                let original = &objects[index];
                let copy_id = format!("{object_id}-copy");
                if objects.iter().any(|object| object.object_id == copy_id) {
                    return Err(SourceFileEditError::failed(format!(
                        "test document already has object `{copy_id}`"
                    )));
                }
                objects.insert(
                    index + 1,
                    SourceFileEditObject::new(copy_id, &original.schema, original.value.clone()),
                );
            }
            SourceFileEditOperation::Remove { object_id } => {
                if object_id == root_object_id {
                    return Err(SourceFileEditError::failed(
                        "test document root object cannot be removed",
                    ));
                }
                let index = objects
                    .iter()
                    .position(|object| object.object_id == *object_id)
                    .ok_or_else(|| {
                        SourceFileEditError::failed(format!(
                            "test document has no object `{object_id}` to remove"
                        ))
                    })?;
                objects.remove(index);
            }
        }

        SourceFileEditDocument::from_objects(root_object_id, objects)
            .map(|document| document.with_codec_state(request.document.codec_state.clone()))
    }

    fn source_file_edit_codec() -> SourceFileEditCodecRegistration {
        SourceFileEditCodecRegistration::for_source::<TestSourceFormat>(
            source_edit_load,
            source_edit_save,
        )
        .with_edit(source_edit_operation)
    }

    fn editable_test_document() -> SourceFileEditDocument {
        SourceFileEditDocument::from_objects(
            "root",
            [
                SourceFileEditObject::new(
                    "root",
                    "az.test.SourceSchema",
                    ReflectedValueEnvelope::typed_ron("alloc::string::String", r#""root""#),
                ),
                SourceFileEditObject::new(
                    "second",
                    "az.test.SourceSchema",
                    ReflectedValueEnvelope::typed_ron("alloc::string::String", r#""second""#),
                ),
            ],
        )
        .expect("test document is valid")
        .with_codec_state([0x5a])
    }

    declare_caps!(SourceCaps:);

    const SOURCES: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.asset-builder-tests"),
        contribution: ContributionId::new("sources"),
        roles: &[],
    };
    const RIVAL: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.asset-builder-rival"),
        contribution: ContributionId::new("sources"),
        roles: &[],
    };

    /// One contribution feeding all three schema-keyed registries, the way a
    /// format crate's `register` does.
    struct Sources;

    impl Contribution for Sources {
        type Caps = SourceCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            SOURCES
        }

        fn register(&self, ctx: &mut GemContext<'_, SourceCaps>) {
            ctx.registrar::<SourceSchemaRegistration>()
                .register(source_schema());
            ctx.registrar::<SourceFileTemplateRegistration>()
                .register(source_file_template());
            ctx.registrar::<SourceFileEditCodecRegistration>()
                .register(source_file_edit_codec());
            ctx.registrar::<ProductFormatRegistration>().register(
                ProductFormatRegistration::from_dynamic_parts(
                    ProductFormatId::__from_static("az.test.value"),
                    2,
                ),
            );
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Sources, ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    #[test]
    fn product_path_accepts_safe_relative_asset_paths() {
        let path = ProductPath::new("textures/brick.dds").unwrap();
        assert_eq!(path.as_str(), "textures/brick.dds");
        assert_eq!(
            ProductPath::new(r"Textures\Brick.DDS").unwrap().as_str(),
            "textures/brick.dds"
        );
    }

    #[test]
    fn product_path_rejects_paths_that_escape_cache_root() {
        let absolute = std::env::temp_dir().join("abs.dds");
        let absolute = absolute.to_string_lossy();
        for path in ["", absolute.as_ref(), "../brick.dds"] {
            assert!(ProductPath::new(path).is_err(), "{path}");
        }
    }

    #[test]
    fn job_key_rejects_empty_or_padded_values() {
        assert!(JobKey::new("").is_err());
        assert!(JobKey::new(" default").is_err());
        assert_eq!(JobKey::new("compile").unwrap(), "compile");
    }

    #[test]
    fn composed_product_formats_are_sorted_and_searchable() {
        let composer = composed();
        let registries = composer.registries();
        let ids = composed_product_formats(registries)
            .iter()
            .map(|attributed| attributed.entry.id().as_str())
            .collect::<Vec<_>>();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort_unstable();
        assert_eq!(ids, sorted_ids);

        assert!(composed_product_format_by_id(registries, "azoth.unknown").is_none());
        assert_eq!(
            composed_product_format_by_id(registries, "az.test.value")
                .map(|attributed| attributed.entry.current_version()),
            Some(2)
        );
    }

    /// ADR 0010 end state: the SDK registers no product formats of its own.
    /// The `test-support` fixture format is opt-in and reaches a host only
    /// through a harness contribution that calls its `register`, so every
    /// composed format is attributed to whoever asked for it.
    #[test]
    fn the_sdk_composes_no_product_formats_of_its_own() {
        let composer = composed();
        assert!(
            composed_product_formats(composer.registries())
                .iter()
                .all(|attributed| attributed.instance.gem == SOURCES.gem),
            "the asset-builder SDK must not contribute product formats"
        );
    }

    #[test]
    fn every_composed_entry_is_attributed_to_its_contribution() {
        let report = composed().finalize().expect("composition is valid");
        for registry in [
            "source-schema",
            "source-file-template",
            "source-file-edit-codec",
            "product-format",
        ] {
            let entries = report
                .entries
                .iter()
                .filter(|entry| entry.registry == registry)
                .collect::<Vec<_>>();
            assert_eq!(entries.len(), 1, "{registry}");
            assert_eq!(entries[0].instance.contribution.as_str(), "sources");
        }
    }

    #[test]
    fn two_contributions_claiming_one_schema_fail_composition() {
        struct Rival;

        impl Contribution for Rival {
            type Caps = SourceCaps;

            fn descriptor(&self) -> ContributionDescriptor {
                RIVAL
            }

            fn register(&self, ctx: &mut GemContext<'_, SourceCaps>) {
                ctx.registrar::<SourceSchemaRegistration>()
                    .register(source_schema());
            }
        }

        let mut composer = composed();
        composer.add(Rival, ProductActivation::default()).unwrap();

        let ComposeError::Duplicate {
            registry,
            key,
            first,
            second,
        } = composer.finalize().unwrap_err()
        else {
            panic!("a repeated source schema must fail composition");
        };
        assert_eq!(registry, "source-schema");
        assert_eq!(key, "az.test.SourceSchema");
        assert_eq!(first.gem.as_str(), "azoth.asset-builder-tests");
        assert_eq!(second.gem.as_str(), "azoth.asset-builder-rival");
    }

    #[test]
    fn composed_source_schemas_are_sorted_and_searchable() {
        let composer = composed();
        let registries = composer.registries();
        let registrations = composed_source_schemas(registries);
        let ids = registrations
            .iter()
            .map(|attributed| attributed.entry.schema_type().as_str())
            .collect::<Vec<_>>();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort_unstable();
        assert_eq!(ids, sorted_ids);

        let composed_entry = registrations
            .iter()
            .find(|attributed| attributed.entry.schema_type() == TEST_SOURCE_SCHEMA)
            .expect("test source schema is composed");
        // The owner is the contribution that registered it, not a string the
        // registration restates about itself.
        assert_eq!(composed_entry.instance.gem, SOURCES.gem);
        assert_eq!(composed_entry.instance.contribution, SOURCES.contribution);
        let source_schema = composed_entry.entry;
        assert_eq!(source_schema.label(), "Test Source");
        assert_eq!(source_schema.category(), "Tests");
        assert_eq!(
            source_schema.authoring(),
            SourceSchemaAuthoring::ProjectDocument {
                schema_type: "az.test.SourceSchema"
            }
        );
    }

    #[test]
    fn the_composed_source_file_template_creates_its_schemas_source() {
        let composer = composed();
        let template = composed_source_file_template(composer.registries(), TEST_SOURCE_SCHEMA)
            .expect("test source file template is composed")
            .entry;
        let bytes = template
            .create(&SourceFileTemplateRequest {
                schema_type: TEST_SOURCE_SCHEMA,
                source_path: "tests/example.ron",
            })
            .unwrap();
        assert_eq!(
            bytes,
            b"schema=az.test.SourceSchema;path=tests/example.ron\n"
        );
        assert_eq!(
            template.candidates(),
            vec![
                SourceFileTemplateCandidate::new("tests/example.ron")
                    .with_label("Example")
                    .with_description("Example source template")
            ]
        );
    }

    #[test]
    fn the_composed_source_file_edit_codec_round_trips_its_schemas_source() {
        let composer = composed();
        let codec = composed_source_file_edit_codec(composer.registries(), TEST_SOURCE_SCHEMA)
            .expect("test source file edit codec is composed")
            .entry;
        let context = SourceFileEditCodecContext::new(composer.registries());
        let document = codec
            .load(
                context,
                &SourceFileEditLoadRequest {
                    schema_type: TEST_SOURCE_SCHEMA,
                    source_path: "tests/example.ron",
                    bytes: b"editable",
                },
            )
            .unwrap();
        assert_eq!(document.root_schema, "az.test.SourceSchema");
        assert_eq!(document.value.type_path, "alloc::string::String");
        assert_eq!(document.value.payload, br#""editable""#);

        let saved = codec
            .save(
                context,
                &SourceFileEditSaveRequest::new(
                    TEST_SOURCE_SCHEMA,
                    "tests/example.ron",
                    &document.root_schema,
                    &document.value,
                    &document.objects,
                    &document.codec_state,
                ),
            )
            .unwrap();
        assert_eq!(
            saved,
            b"schema=az.test.SourceSchema;root=az.test.SourceSchema;path=tests/example.ron;value=editable"
        );
    }

    #[test]
    fn source_file_edit_codec_owns_structural_operations() {
        let composer = composed();
        let codec = composed_source_file_edit_codec(composer.registries(), TEST_SOURCE_SCHEMA)
            .expect("test source file edit codec is composed")
            .entry;
        let context = SourceFileEditCodecContext::new(composer.registries());
        let document = editable_test_document();

        let duplicate = SourceFileEditOperation::Duplicate {
            object_id: "second".to_string(),
        };
        let duplicated = codec
            .edit(
                context,
                &SourceFileEditOperationRequest::new(
                    TEST_SOURCE_SCHEMA,
                    "tests/example.ron",
                    &document,
                    &duplicate,
                ),
            )
            .expect("codec duplicates the requested object");
        assert_eq!(
            duplicated
                .objects
                .iter()
                .map(|object| object.object_id.as_str())
                .collect::<Vec<_>>(),
            vec!["root", "second", "second-copy"]
        );
        assert_eq!(duplicated.codec_state, vec![0x5a]);

        let append = SourceFileEditOperation::AppendDefault;
        let appended = codec
            .edit(
                context,
                &SourceFileEditOperationRequest::new(
                    TEST_SOURCE_SCHEMA,
                    "tests/example.ron",
                    &document,
                    &append,
                ),
            )
            .expect("codec appends its own default object");
        assert_eq!(
            appended
                .objects
                .iter()
                .map(|object| object.object_id.as_str())
                .collect::<Vec<_>>(),
            vec!["root", "second", "generated-3"]
        );

        let remove = SourceFileEditOperation::Remove {
            object_id: "second".to_string(),
        };
        let removed = codec
            .edit(
                context,
                &SourceFileEditOperationRequest::new(
                    TEST_SOURCE_SCHEMA,
                    "tests/example.ron",
                    &document,
                    &remove,
                ),
            )
            .expect("codec removes the requested non-root object");
        assert_eq!(
            removed
                .objects
                .iter()
                .map(|object| object.object_id.as_str())
                .collect::<Vec<_>>(),
            vec!["root"]
        );
    }

    #[test]
    fn source_file_edit_codec_rejects_unpublished_or_mismatched_operations() {
        let composer = composed();
        let context = SourceFileEditCodecContext::new(composer.registries());
        let document = editable_test_document();
        let append = SourceFileEditOperation::AppendDefault;
        let unsupported = SourceFileEditCodecRegistration::for_source::<TestSourceFormat>(
            source_edit_load,
            source_edit_save,
        )
        .edit(
            context,
            &SourceFileEditOperationRequest::new(
                TEST_SOURCE_SCHEMA,
                "tests/example.ron",
                &document,
                &append,
            ),
        )
        .expect_err("the codec did not publish structural operations");
        assert!(matches!(
            unsupported,
            SourceFileEditError::Unsupported { .. }
        ));

        let mismatched = SourceSchemaType::__from_static("az.test.OtherSourceSchema");
        let mismatch = source_file_edit_codec()
            .edit(
                context,
                &SourceFileEditOperationRequest::new(
                    mismatched,
                    "tests/example.ron",
                    &document,
                    &append,
                ),
            )
            .expect_err("the codec is selected only by its source schema");
        assert!(matches!(mismatch, SourceFileEditError::Unsupported { .. }));
    }

    #[test]
    fn source_schema_file_workflow_records_editor_file_contract() {
        let registration = SourceSchemaRegistration::from_dynamic_parts(
            SourceSchemaType::__from_static("az.test.LegacyMaterialSource"),
        )
        .with_import_file("materials", &["mtl"]);

        let SourceSchemaAuthoring::File { workflow } = registration.authoring() else {
            panic!("expected file workflow");
        };
        assert_eq!(workflow.source_root(), PROJECT_SOURCE_ROOT);
        assert_eq!(workflow.default_path_prefix(), "materials");
        assert_eq!(workflow.extensions(), &["mtl"]);
        assert!(!workflow.can_create());
        assert!(!workflow.can_edit());
    }

    #[test]
    fn source_schema_file_workflow_can_be_editor_creatable() {
        let registration = SourceSchemaRegistration::from_dynamic_parts(
            SourceSchemaType::__from_static("az.test.NativeTableSource"),
        )
        .with_creatable_file("tables", &["table.ron"]);

        let SourceSchemaAuthoring::File { workflow } = registration.authoring() else {
            panic!("expected file workflow");
        };
        assert_eq!(workflow.source_root(), PROJECT_SOURCE_ROOT);
        assert!(workflow.can_create());
        assert!(workflow.can_edit());
    }

    #[test]
    fn source_schema_file_workflow_can_target_gem_source_root() {
        let registration = SourceSchemaRegistration::from_dynamic_parts(
            SourceSchemaType::__from_static("az.test.GemMaterialSource"),
        )
        .with_creatable_file_in_source_root(
            "gem:azoth.render:assets",
            "materials",
            &["material.ron"],
        );

        let SourceSchemaAuthoring::File { workflow } = registration.authoring() else {
            panic!("expected file workflow");
        };
        assert_eq!(workflow.source_root(), "gem:azoth.render:assets");
        assert_eq!(workflow.default_path_prefix(), "materials");
        assert_eq!(workflow.extensions(), &["material.ron"]);
        assert!(workflow.can_create());
        assert!(workflow.can_edit());
    }
}
