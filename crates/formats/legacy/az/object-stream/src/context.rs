//! Reflection and serializer context for legacy `ObjectStreams`.
//!
//! Resolution mirrors Lumberyard/O3DE: choose the v2 lookup candidate, run
//! `SerializeContext::FindClassData`, then apply the registered class'
//! `GenericClassInfo`. A UUID alone is never treated as proof that `ClassData` or
//! an `IDataSerializer` exists.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arcstr::ArcStr;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::codec::{BuiltinSerializerDescriptor, ObjectStreamValueCodec, builtin_serializer_kind};
use crate::lookup::LumberyardHashes;
use crate::overlay::ObjectStreamDataOverlayProvider;
use crate::types;

/// Engine generation whose native v2 Asset specialization rules are applied.
///
/// Lumberyard always performs `FindClassData` with the raw Asset UUID. O3DE
/// retains the later element-version-dependent specialization lookup policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ObjectStreamDialect {
    Lumberyard,
    #[default]
    O3de,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ReflectedClassKey(u64);

impl ReflectedClassKey {
    #[cfg(test)]
    pub(crate) const fn new(test_id: Uuid) -> Self {
        Self(test_id.as_u64_pair().1)
    }
}

/// Native `GenericClassInfo` identity.
///
/// The default `CanStoreType` accepts the specialized/generic/legacy triple;
/// native smart-pointer families also accept their captured pointee type.
/// `uuidGenericMap` registration aliases remain separate context keys and never
/// widen this set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericClassInfo {
    class: ReflectedClassKey,
    specialized_type_id: Uuid,
    generic_type_id: Option<Uuid>,
    legacy_specialized_type_id: Option<Uuid>,
    smart_pointer_pointee_type_id: Option<Uuid>,
    templated_type_ids: Vec<Uuid>,
    non_type_template_arguments: Vec<u64>,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum GenericClassInfoError {
    #[error("generic type {generic_type_id:?} is not a captured smart-pointer family")]
    NotSmartPointer { generic_type_id: Option<Uuid> },
}

/// Captured behavioral family of a native `SerializeContext::IDataContainer`.
///
/// `ClassData::m_container != nullptr` alone is not enough to distinguish a
/// sequence, associative container, pair, or smart-pointer wrapper. Keeping
/// that distinction in reflection metadata prevents Rust target types or child
/// arity from being used to guess native container semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerShape {
    Sequence,
    Set,
    Map,
    Pair,
    Tuple,
    SmartPointer,
    Optional,
    Variant,
    Wrapper,
}

/// Captured native container size contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerCardinality {
    Variable,
    Bounded { capacity: usize },
    Fixed { size: usize },
}

impl GenericClassInfo {
    #[must_use]
    pub const fn new(
        class: ReflectedClassKey,
        specialized_type_id: Uuid,
        generic_type_id: Option<Uuid>,
        legacy_specialized_type_id: Option<Uuid>,
    ) -> Self {
        Self {
            class,
            specialized_type_id,
            generic_type_id,
            legacy_specialized_type_id,
            smart_pointer_pointee_type_id: None,
            templated_type_ids: Vec::new(),
            non_type_template_arguments: Vec::new(),
        }
    }

    /// Preserve captured type template arguments in declaration order.
    #[must_use]
    pub fn with_templated_type_ids(mut self, type_ids: impl Into<Vec<Uuid>>) -> Self {
        self.templated_type_ids = type_ids.into();
        self
    }

    /// Preserve captured non-type template arguments such as `fixed_vector`
    /// capacity and `array` size.
    #[must_use]
    pub fn with_non_type_template_arguments(mut self, arguments: impl Into<Vec<u64>>) -> Self {
        self.non_type_template_arguments = arguments.into();
        self
    }

    /// Capture the additional pointee type accepted by the three native
    /// smart-pointer `GenericClassInfo::CanStoreType` overrides.
    ///
    /// # Errors
    ///
    /// Returns [`GenericClassInfoError::NotSmartPointer`] unless the captured
    /// generic type UUID is one of the three native smart-pointer families —
    /// a pointee is meaningless on any other `GenericClassInfo`.
    pub fn with_smart_pointer_pointee(
        mut self,
        pointee_type_id: Uuid,
    ) -> Result<Self, GenericClassInfoError> {
        if self
            .generic_type_id
            .and_then(crate::container::container_shape_for_generic_uuid)
            != Some(ContainerShape::SmartPointer)
        {
            return Err(GenericClassInfoError::NotSmartPointer {
                generic_type_id: self.generic_type_id,
            });
        }
        self.smart_pointer_pointee_type_id = Some(pointee_type_id);
        Ok(self)
    }

    #[must_use]
    pub const fn class(&self) -> ReflectedClassKey {
        self.class
    }

    #[must_use]
    pub const fn specialized_type_id(&self) -> Uuid {
        self.specialized_type_id
    }

    #[must_use]
    pub const fn generic_type_id(&self) -> Option<Uuid> {
        self.generic_type_id
    }

    #[must_use]
    pub const fn legacy_specialized_type_id(&self) -> Option<Uuid> {
        self.legacy_specialized_type_id
    }

    /// Asset class carried by native `Asset<T>` reflection metadata.
    #[must_use]
    pub fn asset_type_id(&self) -> Option<&Uuid> {
        let is_asset =
            self.specialized_type_id == types::ASSET || self.generic_type_id == Some(types::ASSET);
        match (is_asset, self.templated_type_ids.as_slice()) {
            (true, [asset_type_id]) => Some(asset_type_id),
            _ => None,
        }
    }

    #[must_use]
    pub fn non_type_template_arguments(&self) -> &[u64] {
        &self.non_type_template_arguments
    }

    #[must_use]
    pub fn can_store_type(&self, type_id: Uuid) -> bool {
        type_id == self.specialized_type_id
            || self.generic_type_id == Some(type_id)
            || self.legacy_specialized_type_id == Some(type_id)
            || self.smart_pointer_pointee_type_id == Some(type_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedField {
    /// Exact `ClassElement::m_name` spelling this class registered for the
    /// slot.
    ///
    /// `AZ::Crc32` folds ASCII case before hashing, so `m_nameCrc` proves slot
    /// identity but carries no case: `crc32("debugname")` is the stored value
    /// for `DebugName`, `debugName`, and `DEBUGNAME` alike. Two classes may
    /// therefore register different spellings of one CRC (`AZ::Entity::Id` vs
    /// `GDEID::id`), which a global CRC → name table cannot represent. The
    /// registration name is per-class evidence and belongs here.
    pub name: Option<ArcStr>,
    pub type_id: Uuid,
    /// Semantic enum identity declared by the field's captured `EnumType`
    /// attribute. Native records the enum on the `ClassElement`'s attribute list
    /// rather than in `typeId`, which stays the underlying integral type, so
    /// this is the only evidence that a bare discriminant belongs to an enum.
    pub enum_type_id: Option<Uuid>,
    pub generic: Option<GenericClassInfo>,
    pub is_base_class: bool,
    pub is_pointer: bool,
    pub is_dynamic_field: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReflectedEdge {
    pub is_base_class: bool,
    pub is_pointer: bool,
    pub is_dynamic_field: bool,
    pub asset_type_id: Option<Uuid>,
}

impl ReflectedField {
    #[must_use]
    pub const fn new(type_id: Uuid) -> Self {
        Self {
            name: None,
            type_id,
            enum_type_id: None,
            generic: None,
            is_base_class: false,
            is_pointer: false,
            is_dynamic_field: false,
        }
    }

    /// Record the exact `ClassElement::m_name` spelling registered for this
    /// slot. See [`ReflectedField::name`] for why the CRC cannot supply it.
    #[must_use]
    pub fn with_field_name(mut self, name: impl Into<ArcStr>) -> Self {
        self.name = Some(name.into());
        self
    }

    #[must_use]
    pub const fn with_enum_type(mut self, enum_type_id: Uuid) -> Self {
        self.enum_type_id = Some(enum_type_id);
        self
    }

    #[must_use]
    pub fn with_generic(mut self, generic: GenericClassInfo) -> Self {
        self.generic = Some(generic);
        self
    }

    #[must_use]
    pub const fn as_base_class(mut self) -> Self {
        self.is_base_class = true;
        self
    }

    #[must_use]
    pub const fn as_pointer(mut self) -> Self {
        self.is_pointer = true;
        self
    }

    #[must_use]
    pub const fn as_dynamic_field(mut self) -> Self {
        self.is_dynamic_field = true;
        self
    }
}

// Each flag mirrors one captured native `ClassData` member (`m_container`,
// the 0xFFFF_FFFF deprecation version, `m_converter`, `m_dataConverter`,
// `m_serializer`); grouping them would stop this type from naming the capture.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedClass {
    /// `ClassData::m_typeId`, which can differ from the `uuidMap` key.
    pub type_id: Uuid,
    pub name: Option<ArcStr>,
    pub is_container: bool,
    /// Captured `IDataContainer` behavior. `None` means only the existence of
    /// `m_container` is known, which is insufficient for collection decoding.
    pub container_shape: Option<ContainerShape>,
    pub container_cardinality: Option<ContainerCardinality>,
    pub current_version: u32,
    pub deprecated: bool,
    /// Capture evidence only. A native converter pointer is not executable
    /// support and therefore never authorizes typed decoding by itself.
    pub captured_version_converter: bool,
    /// Captured non-null `ClassData::m_dataConverter` evidence. It is not
    /// executable support and therefore cannot authorize conversion.
    pub captured_data_converter: bool,
    /// A non-null native `ClassData::m_serializer` was captured, but no
    /// executable codec is implied by that evidence.
    pub captured_serializer: bool,
    pub fields: HashMap<u32, ReflectedField>,
}

impl ReflectedClass {
    #[must_use]
    pub fn new(type_id: Uuid) -> Self {
        Self {
            type_id,
            name: None,
            is_container: false,
            container_shape: None,
            container_cardinality: None,
            current_version: 0,
            deprecated: false,
            captured_version_converter: false,
            captured_data_converter: false,
            captured_serializer: false,
            fields: HashMap::new(),
        }
    }

    #[must_use]
    pub const fn as_container(mut self) -> Self {
        self.is_container = true;
        self
    }

    #[must_use]
    pub const fn with_container_shape(mut self, shape: ContainerShape) -> Self {
        self.is_container = true;
        self.container_shape = Some(shape);
        self
    }

    #[must_use]
    pub const fn with_container_cardinality(mut self, cardinality: ContainerCardinality) -> Self {
        self.is_container = true;
        self.container_cardinality = Some(cardinality);
        self
    }

    #[must_use]
    pub fn with_name(mut self, name: impl Into<ArcStr>) -> Self {
        self.name = Some(name.into());
        self
    }

    #[must_use]
    pub const fn with_version(mut self, version: u32) -> Self {
        self.current_version = version;
        self
    }

    #[must_use]
    pub const fn deprecated(mut self) -> Self {
        self.deprecated = true;
        self
    }

    #[must_use]
    pub const fn with_captured_version_converter(mut self) -> Self {
        self.captured_version_converter = true;
        self
    }

    #[must_use]
    pub const fn with_captured_data_converter(mut self) -> Self {
        self.captured_data_converter = true;
        self
    }

    #[must_use]
    pub const fn with_captured_serializer(mut self) -> Self {
        self.captured_serializer = true;
        self
    }

    pub fn insert_field(&mut self, name_crc: u32, field: ReflectedField) {
        self.fields.insert(name_crc, field);
    }

    /// Register `field` under `AZ::Crc32(name)`, retaining `name` as this
    /// class's exact registration spelling for the slot.
    pub fn insert_named_field(&mut self, name: &str, field: ReflectedField) {
        self.fields
            .insert(crate::field_name_crc(name), field.with_field_name(name));
    }

    fn find_field(&self, name_crc: Option<u32>) -> Option<&ReflectedField> {
        // IDataContainer::GetElement and ordinary ClassData lookup both require
        // the exact serialized name CRC. Never infer a sole container slot.
        name_crc.and_then(|crc| self.fields.get(&crc))
    }

    fn edge(&self, name_crc: Option<u32>) -> Option<ReflectedEdge> {
        self.find_field(name_crc).map(|field| ReflectedEdge {
            is_base_class: field.is_base_class,
            is_pointer: field.is_pointer,
            is_dynamic_field: field.is_dynamic_field,
            asset_type_id: field
                .generic
                .as_ref()
                .and_then(GenericClassInfo::asset_type_id)
                .copied(),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResolveTypeInput {
    pub stream_version: u32,
    pub raw_type_id: Uuid,
    pub specialization_type_id: Option<Uuid>,
    pub element_version: u32,
    pub name_crc: Option<u32>,
    pub parent: Option<ReflectedClassKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedType {
    /// Semantic id exists only when `FindClassData` succeeded.
    pub type_id: Option<Uuid>,
    pub class: Option<ReflectedClassKey>,
    pub builtin_serializer: Option<crate::codec::BuiltinSerializerDescriptor>,
    pub is_container: bool,
    pub container_shape: Option<ContainerShape>,
    /// Native multimap lookup had multiple distinct `GenericClassInfo` targets.
    /// Callers must fail rather than select unordered-map iteration order.
    pub ambiguous_generic: bool,
}

pub trait ObjectStreamTypeResolver {
    fn resolve_type(&self, input: ResolveTypeInput) -> ResolvedType;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionConversionState {
    #[default]
    Current,
    /// Native `ObjectStream::LoadClass` discards a `ClassData` whose version is
    /// `0xFFFF_FFFF` when no deprecation converter is registered. The entire
    /// serialized subtree is ignored without resolving its descendants.
    DeprecatedDiscard,
    SerializerCompatibleOld {
        from: u32,
        to: u32,
    },
    RegisteredConverter {
        from: u32,
        to: u32,
    },
    StrictDefaultStructural {
        from: u32,
        to: u32,
    },
}

impl VersionConversionState {
    #[must_use]
    pub const fn requires_dom_converter(self) -> bool {
        matches!(self, Self::RegisteredConverter { .. })
    }

    #[must_use]
    pub const fn discards_subtree(self) -> bool {
        matches!(self, Self::DeprecatedDiscard)
    }
}

pub trait ObjectStreamVersionConverter: std::fmt::Debug + Send + Sync {
    /// Rewrite `element` in place from class version `from` to version `to`.
    ///
    /// # Errors
    ///
    /// Implementation-defined: any [`crate::ObjectStreamError`] the converter
    /// wants to fail the load with. Returning an error aborts the whole read;
    /// a converter that cannot handle this particular version pair should say so
    /// rather than leave the element half-converted.
    fn convert(
        &self,
        element: &mut crate::Element,
        from: u32,
        to: u32,
    ) -> Result<(), crate::ObjectStreamError>;
}

pub trait ObjectStreamDataConverter: std::fmt::Debug + Send + Sync {
    /// Rewrite `element` in place from `source_type_id` to `target_type_id`.
    ///
    /// # Errors
    ///
    /// Implementation-defined: any [`crate::ObjectStreamError`] the converter
    /// wants to fail the load with. The caller re-resolves `element` afterwards,
    /// so a converter that leaves it unresolvable produces
    /// [`crate::ObjectStreamError::UnresolvedElementType`] rather than a silent
    /// skip.
    fn convert(
        &self,
        element: &mut crate::Element,
        source_type_id: Uuid,
        target_type_id: Uuid,
    ) -> Result<(), crate::ObjectStreamError>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SerializerRegistrationError {
    #[error("ClassData {class:?} is not registered")]
    UnknownClass { class: ReflectedClassKey },
    #[error("ClassData {class:?} is deprecated")]
    DeprecatedClass { class: ReflectedClassKey },
    #[error("captured ClassData serializer type {type_id} is not implemented")]
    UnsupportedSerializer { type_id: Uuid },
    #[error("serializer descriptor kind {actual:?} does not match {expected:?} for type {type_id}")]
    DescriptorKindMismatch {
        type_id: Uuid,
        expected: crate::codec::BuiltinSerializerKind,
        actual: crate::codec::BuiltinSerializerKind,
    },
    #[error("AssetSerializer version {version} is unsupported")]
    UnsupportedAssetVersion { version: u32 },
    #[error("ClassData {class:?} has no captured m_dataConverter")]
    DataConverterNotCaptured { class: ReflectedClassKey },
    #[error("native uuidMap key {map_id} is already bound to ClassData {class:?}")]
    DuplicateDirectClassAlias {
        map_id: Uuid,
        class: ReflectedClassKey,
    },
    #[error("native uuidMap key {map_id} is bound to ClassData {existing:?}, not {requested:?}")]
    ConflictingDirectClassAlias {
        map_id: Uuid,
        existing: ReflectedClassKey,
        requested: ReflectedClassKey,
    },
    #[error("legacy generic UUID {legacy_id} redirects to unknown generic UUID {target_id}")]
    UnknownGenericRedirectTarget { legacy_id: Uuid, target_id: Uuid },
    #[error("ClassData key {class:?} was not reserved by this context")]
    UnreservedClassData { class: ReflectedClassKey },
    #[error("ClassData key {class:?} was finalized more than once")]
    DuplicateClassDataDefinition { class: ReflectedClassKey },
    #[error(
        "native uuidGenericMap key {map_id} has conflicting ClassData bindings {existing:?} and {requested:?}"
    )]
    ConflictingGenericAlias {
        map_id: Uuid,
        existing: ReflectedClassKey,
        requested: ReflectedClassKey,
    },
    #[error("legacy generic UUID {legacy_id} has conflicting redirects {existing} and {requested}")]
    ConflictingLegacyGenericRedirect {
        legacy_id: Uuid,
        existing: Uuid,
        requested: Uuid,
    },
    #[error("reserved ClassData key {class:?} was never defined")]
    UndefinedReservedClassData { class: ReflectedClassKey },
    #[error("native uuidMap key {map_id} refers to undefined ClassData {class:?}")]
    UndefinedDirectClassAlias {
        map_id: Uuid,
        class: ReflectedClassKey,
    },
    #[error("native uuidGenericMap key {map_id} refers to undefined ClassData {class:?}")]
    UndefinedGenericClass {
        map_id: Uuid,
        class: ReflectedClassKey,
    },
    #[error(
        "ClassData {owner:?} field CRC {name_crc:#010x} refers to undefined generic ClassData {class:?}"
    )]
    UndefinedFieldGenericClass {
        owner: ReflectedClassKey,
        name_crc: u32,
        class: ReflectedClassKey,
    },
    #[error("enum {enum_id} has conflicting underlying types {existing} and {requested}")]
    ConflictingEnumUnderlyingType {
        enum_id: Uuid,
        existing: Uuid,
        requested: Uuid,
    },
    #[error("enum {enum_id} refers to unregistered underlying ClassData type {underlying_id}")]
    UndefinedEnumUnderlyingType { enum_id: Uuid, underlying_id: Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutableSupportIssue {
    CapturedSerializerMissingCodec {
        class: ReflectedClassKey,
        type_id: Uuid,
    },
    CapturedVersionConverterMissingImplementation {
        class: ReflectedClassKey,
        type_id: Uuid,
        deprecated: bool,
    },
    CapturedDataConverterMissingImplementation {
        class: ReflectedClassKey,
        type_id: Uuid,
    },
    UnclassifiedContainer {
        class: ReflectedClassKey,
        type_id: Uuid,
    },
    MissingContainerCardinality {
        class: ReflectedClassKey,
        type_id: Uuid,
    },
}

impl ExecutableSupportIssue {
    const fn class(&self) -> ReflectedClassKey {
        match *self {
            Self::CapturedSerializerMissingCodec { class, .. }
            | Self::CapturedVersionConverterMissingImplementation { class, .. }
            | Self::CapturedDataConverterMissingImplementation { class, .. }
            | Self::UnclassifiedContainer { class, .. }
            | Self::MissingContainerCardinality { class, .. } => class,
        }
    }
}

/// Inventory of captured native behavior that this context cannot execute.
///
/// This is deliberately separate from [`ObjectStreamReadContext::validate_complete`]:
/// identity lowering must always be complete, while a missing converter only
/// blocks an asset that actually contains an old/deprecated node requiring it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutableSupportAudit {
    pub issues: Vec<ExecutableSupportIssue>,
}

impl ExecutableSupportAudit {
    #[must_use]
    pub const fn is_fully_supported(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Fidelity mode for a stream child that fails native `ClassElement` binding
/// (`classElement == nullptr` in Lumberyard `ObjectStream::LoadClass`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UnmatchedChildPolicy {
    /// Strict-filter parity: any unbindable child fails the load.
    #[default]
    Fail,
    /// Default-filter parity (`FilterDescriptor` flags = 0): the child subtree
    /// is discarded and loading continues, as the retail runtime does.
    NativeSkip,
}

/// Fidelity mode for an `ObjectStream` element whose type has no registered
/// `ClassData` (`SerializeContext::FindClassData` returns null).
///
/// This is deliberately separate from [`UnmatchedChildPolicy`]: an
/// unregistered class cannot reach native `ClassElement` binding at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UnregisteredClassPolicy {
    /// Preserve the context-aware import boundary: an unregistered reached
    /// class fails the load.
    #[default]
    Fail,
    /// Default-filter parity (`FilterDescriptor` flags = 0): discard the
    /// complete unregistered subtree and continue loading.
    NativeSkip,
}

/// Outcome of resolving one reached `ObjectStream` element against `ClassData`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ElementFate {
    /// `ClassData` resolved and the element proceeds through typed decoding.
    Retain,
    /// `ClassData` is unregistered and native default-filter loading discards
    /// the complete subtree.
    Skip,
}

/// Outcome of finalizing one reachable child against its parent `ClassData`.
///
/// [`ChildFate::Skip`] is only ever produced under
/// [`UnmatchedChildPolicy::NativeSkip`]; the strict default returns an error
/// instead of a skip.
/// The parent edge a data-converted child is rebound under.
#[derive(Debug, Clone, Copy)]
struct ConvertedChildEdge {
    parent: ReflectedClassKey,
    parent_type_id: Uuid,
    stream_version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildFate {
    /// The child bound to a native field (or converter) and stays in the DOM.
    Retain,
    /// No native `ClassElement` bound the child; native default-filter loading
    /// discards the subtree and continues.
    Skip,
}

/// Captured native reflection and serializer metadata used for typed import.
///
/// [`Default`] intentionally remains an empty O3DE context for tests and raw
/// diagnostics. Production importers should call [`Self::new`] so the source
/// engine dialect is an explicit construction input.
#[derive(Debug, Clone, Default)]
pub struct ObjectStreamReadContext {
    names: LumberyardHashes,
    dialect: ObjectStreamDialect,
    classes: HashMap<ReflectedClassKey, ReflectedClass>,
    reserved_class_keys: HashSet<ReflectedClassKey>,
    direct_classes: HashMap<Uuid, ReflectedClassKey>,
    generic_infos: HashMap<Uuid, Vec<GenericClassInfo>>,
    legacy_generic_redirects: HashMap<Uuid, Vec<Uuid>>,
    enum_underlying_types: HashMap<Uuid, Uuid>,
    codecs: HashMap<ReflectedClassKey, Arc<dyn ObjectStreamValueCodec>>,
    builtin_serializers: HashMap<ReflectedClassKey, crate::codec::BuiltinSerializerDescriptor>,
    converters: HashMap<ReflectedClassKey, Arc<dyn ObjectStreamVersionConverter>>,
    data_converters: HashMap<ReflectedClassKey, Arc<dyn ObjectStreamDataConverter>>,
    overlay_providers: HashMap<u32, Arc<dyn ObjectStreamDataOverlayProvider>>,
    rtti_bases: HashMap<Uuid, Vec<Uuid>>,
    // Per-class import field-CRC renames: a reached child of the outer key's
    // class type whose name CRC matches an inner key is rebound to the inner
    // value before it binds to a ClassElement. Models a native rename that
    // carried no version bump and no converter (native default-filter drops the
    // old field; the importer keeps its data by rebinding it).
    field_renames: HashMap<Uuid, HashMap<u32, u32>>,
    unregistered_class_policy: UnregisteredClassPolicy,
    unmatched_child_policy: UnmatchedChildPolicy,
    next_class_key: u64,
}

impl ObjectStreamReadContext {
    pub(crate) const fn ensure_unambiguous(
        resolved: ResolvedType,
        raw_type_id: Uuid,
    ) -> Result<(), crate::ObjectStreamError> {
        if resolved.ambiguous_generic {
            Err(crate::ObjectStreamError::AmbiguousGenericType {
                type_id: raw_type_id,
            })
        } else {
            Ok(())
        }
    }

    /// Resolve whether a reached element has executable `ClassData` or follows
    /// native default-filter behavior for an unregistered class.
    ///
    /// Ambiguous generic identity is never skippable: it means the registry
    /// contains multiple possible meanings, not that the client lacks the
    /// class.
    pub(crate) fn resolved_element_fate(
        &self,
        resolved: ResolvedType,
        raw_type_id: Uuid,
    ) -> Result<ElementFate, crate::ObjectStreamError> {
        Self::ensure_unambiguous(resolved, raw_type_id)?;
        if resolved.type_id.is_none() || resolved.class.is_none() {
            return match self.unregistered_class_policy {
                UnregisteredClassPolicy::NativeSkip => Ok(ElementFate::Skip),
                UnregisteredClassPolicy::Fail => {
                    Err(crate::ObjectStreamError::UnresolvedElementType {
                        type_id: raw_type_id,
                    })
                }
            };
        }
        Ok(ElementFate::Retain)
    }

    #[must_use]
    pub fn new(names: LumberyardHashes, dialect: ObjectStreamDialect) -> Self {
        Self {
            names,
            dialect,
            ..Self::default()
        }
    }

    #[must_use]
    pub const fn names(&self) -> &LumberyardHashes {
        &self.names
    }

    #[must_use]
    pub const fn dialect(&self) -> ObjectStreamDialect {
        self.dialect
    }

    pub(crate) fn resolved_edge(
        &self,
        parent: Option<ReflectedClassKey>,
        name_crc: Option<u32>,
    ) -> Option<ReflectedEdge> {
        parent
            .and_then(|parent| self.classes.get(&parent))
            .and_then(|class| class.edge(name_crc))
    }

    pub(crate) fn resolved_enum_type(
        &self,
        parent: Option<ReflectedClassKey>,
        name_crc: Option<u32>,
        semantic_type_id: Uuid,
    ) -> Option<Uuid> {
        if self.enum_underlying_types.contains_key(&semantic_type_id) {
            return Some(semantic_type_id);
        }
        let field = parent
            .and_then(|parent| self.classes.get(&parent))
            .and_then(|class| class.find_field(name_crc))?;
        if self.enum_underlying_types.get(&field.type_id) == Some(&semantic_type_id) {
            return Some(field.type_id);
        }
        // Captured streams do not put the enum id in the ClassElement's own
        // `typeId` — that stays the underlying integral type — so neither branch
        // above can fire. The identity is carried on the element's `EnumType`
        // attribute, recovered into `enum_type_id` during capture.
        field.enum_type_id
    }

    #[must_use]
    pub const fn with_dialect(mut self, dialect: ObjectStreamDialect) -> Self {
        self.dialect = dialect;
        self
    }

    /// Select how a reached element with no registered `ClassData` is handled.
    /// Defaults to [`UnregisteredClassPolicy::Fail`].
    #[must_use]
    pub const fn with_unregistered_class_policy(mut self, policy: UnregisteredClassPolicy) -> Self {
        self.unregistered_class_policy = policy;
        self
    }

    #[must_use]
    pub const fn unregistered_class_policy(&self) -> UnregisteredClassPolicy {
        self.unregistered_class_policy
    }

    /// Select how a reached child that binds to no native `ClassElement` is
    /// handled. Defaults to [`UnmatchedChildPolicy::Fail`] (strict filter).
    #[must_use]
    pub const fn with_unmatched_child_policy(mut self, policy: UnmatchedChildPolicy) -> Self {
        self.unmatched_child_policy = policy;
        self
    }

    #[must_use]
    pub const fn unmatched_child_policy(&self) -> UnmatchedChildPolicy {
        self.unmatched_child_policy
    }

    /// Resolve the fate of a reached child that bound to no native
    /// `ClassElement`. Mirrors Lumberyard `ObjectStream::LoadClass`: the strict
    /// filter fails the load, the default filter (flags = 0) discards the
    /// subtree and continues.
    const fn unmatched_child_fate(
        &self,
        parent_type_id: Uuid,
        child_type_id: Uuid,
        name_crc: Option<u32>,
    ) -> Result<ChildFate, crate::ObjectStreamError> {
        match self.unmatched_child_policy {
            UnmatchedChildPolicy::NativeSkip => Ok(ChildFate::Skip),
            UnmatchedChildPolicy::Fail => Err(crate::ObjectStreamError::UnexpectedReachableChild {
                parent_type_id,
                child_type_id,
                name_crc,
            }),
        }
    }

    #[must_use]
    pub fn class(&self, key: ReflectedClassKey) -> Option<&ReflectedClass> {
        self.classes.get(&key)
    }

    /// Resolve a top-level `ObjectStream` element through native
    /// `SerializeContext::FindClassData` precedence.
    ///
    /// This is the semantic root lookup counterpart to parent-aware field
    /// resolution: no parent `ClassData` or field CRC participates, while v2
    /// specialization and dialect-specific Asset handling remain identical to
    /// ordinary parsing.
    #[must_use]
    pub fn resolve_root_type(
        &self,
        stream_version: u32,
        raw_type_id: Uuid,
        specialization_type_id: Option<Uuid>,
        element_version: u32,
    ) -> ResolvedType {
        self.resolve_type(ResolveTypeInput {
            stream_version,
            raw_type_id,
            specialization_type_id,
            element_version,
            name_crc: None,
            parent: None,
        })
    }

    /// Register captured `ClassData` under one native `uuidMap` key.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::DuplicateDirectClassAlias`] if
    /// `map_id` is already bound. The `ClassData` record itself is only stored
    /// after that check, so a rejected call leaves the context unchanged.
    pub fn insert_class(
        &mut self,
        map_id: Uuid,
        class: ReflectedClass,
    ) -> Result<ReflectedClassKey, SerializerRegistrationError> {
        if let Some(existing) = self.direct_classes.get(&map_id).copied() {
            return Err(SerializerRegistrationError::DuplicateDirectClassAlias {
                map_id,
                class: existing,
            });
        }
        let key = self.insert_class_data(class);
        self.insert_direct_class_alias(map_id, key)?;
        Ok(key)
    }

    /// Store captured `ClassData` without adding a native `uuidMap` alias.
    /// `GenericClassInfo` may own `ClassData` that is reachable only through the
    /// parent field or `uuidGenericMap`; promoting it to direct lookup changes
    /// Lumberyard precedence.
    ///
    /// # Panics
    ///
    /// Panics if the key it just reserved is somehow already defined, which
    /// would mean [`Self::reserve_class_data`] handed out a duplicate.
    pub fn insert_class_data(&mut self, class: ReflectedClass) -> ReflectedClassKey {
        let key = self.reserve_class_data();
        self.define_class_data(key, class)
            .expect("freshly reserved ClassData key is defined exactly once");
        key
    }

    /// Allocate opaque native `ClassData` identity before lowering its fields.
    /// This permits recursive and cyclic reflection graphs without imposing a
    /// crate build order.
    ///
    /// # Panics
    ///
    /// Panics once `u64::MAX` `ClassData` records have been reserved on one
    /// context, exhausting the registration identity space.
    #[must_use]
    pub fn reserve_class_data(&mut self) -> ReflectedClassKey {
        let key = ReflectedClassKey(self.next_class_key);
        self.next_class_key = self
            .next_class_key
            .checked_add(1)
            .expect("ClassData registration identity space is not exhausted");
        self.reserved_class_keys.insert(key);
        key
    }

    /// Finalize one previously reserved `ClassData` record exactly once.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::UnreservedClassData`] if `key` did not
    /// come from [`Self::reserve_class_data`] on this context, or
    /// [`SerializerRegistrationError::DuplicateClassDataDefinition`] if that key was
    /// already finalized.
    pub fn define_class_data(
        &mut self,
        key: ReflectedClassKey,
        class: ReflectedClass,
    ) -> Result<(), SerializerRegistrationError> {
        if !self.reserved_class_keys.contains(&key) {
            return Err(SerializerRegistrationError::UnreservedClassData { class: key });
        }
        if self.classes.contains_key(&key) {
            return Err(SerializerRegistrationError::DuplicateClassDataDefinition { class: key });
        }
        self.classes.insert(key, class);
        Ok(())
    }

    /// Bind an additional captured native `uuidMap` key to existing
    /// `ClassData` without cloning or changing its `m_typeId` identity.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::UnknownClass`] if `class` is not a
    /// defined `ClassData` key,
    /// [`SerializerRegistrationError::DuplicateDirectClassAlias`] if `map_id` is
    /// already bound to that same `ClassData`, and
    /// [`SerializerRegistrationError::ConflictingDirectClassAlias`] if it is bound
    /// to a different one.
    pub fn insert_direct_class_alias(
        &mut self,
        map_id: Uuid,
        class: ReflectedClassKey,
    ) -> Result<(), SerializerRegistrationError> {
        if !self.classes.contains_key(&class) {
            return Err(SerializerRegistrationError::UnknownClass { class });
        }
        match self.direct_classes.entry(map_id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(class);
                Ok(())
            }
            std::collections::hash_map::Entry::Occupied(entry) if *entry.get() == class => {
                Err(SerializerRegistrationError::DuplicateDirectClassAlias { map_id, class })
            }
            std::collections::hash_map::Entry::Occupied(entry) => {
                Err(SerializerRegistrationError::ConflictingDirectClassAlias {
                    map_id,
                    existing: *entry.get(),
                    requested: class,
                })
            }
        }
    }

    /// Builder form of [`Self::insert_class`].
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::insert_class`] returns; the context is dropped
    /// rather than returned when it does.
    pub fn with_class(
        mut self,
        map_id: Uuid,
        class: ReflectedClass,
    ) -> Result<Self, SerializerRegistrationError> {
        self.insert_class(map_id, class)?;
        Ok(self)
    }

    /// Validate that the two-pass native `ClassData` lowering is complete.
    ///
    /// Context-aware readers and writers call this gate before touching a
    /// payload, so an incomplete capture cannot appear to work merely because
    /// the dangling `ClassData` edge was not exercised by that particular asset.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::UndefinedReservedClassData`] for a
    /// key that [`Self::reserve_class_data`] handed out but
    /// [`Self::define_class_data`] never filled in,
    /// [`SerializerRegistrationError::UndefinedDirectClassAlias`] and
    /// [`SerializerRegistrationError::UndefinedGenericClass`] for a `uuidMap` or
    /// `uuidGenericMap` entry pointing at an undefined record,
    /// [`SerializerRegistrationError::UndefinedFieldGenericClass`] for a field whose
    /// `GenericClassInfo` owns an undefined record, and
    /// [`SerializerRegistrationError::UndefinedEnumUnderlyingType`] for an enum
    /// whose underlying type has no direct registration.
    pub fn validate_complete(&self) -> Result<(), SerializerRegistrationError> {
        for &class in &self.reserved_class_keys {
            if !self.classes.contains_key(&class) {
                return Err(SerializerRegistrationError::UndefinedReservedClassData { class });
            }
        }
        for (&map_id, &class) in &self.direct_classes {
            if !self.classes.contains_key(&class) {
                return Err(SerializerRegistrationError::UndefinedDirectClassAlias {
                    map_id,
                    class,
                });
            }
        }
        for (&map_id, infos) in &self.generic_infos {
            for info in infos {
                if !self.classes.contains_key(&info.class()) {
                    return Err(SerializerRegistrationError::UndefinedGenericClass {
                        map_id,
                        class: info.class(),
                    });
                }
            }
        }
        for (&owner, class_data) in &self.classes {
            for (&name_crc, field) in &class_data.fields {
                if let Some(generic) = &field.generic
                    && !self.classes.contains_key(&generic.class())
                {
                    return Err(SerializerRegistrationError::UndefinedFieldGenericClass {
                        owner,
                        name_crc,
                        class: generic.class(),
                    });
                }
            }
        }
        for (&enum_id, &underlying_id) in &self.enum_underlying_types {
            if !self.direct_classes.contains_key(&underlying_id) {
                return Err(SerializerRegistrationError::UndefinedEnumUnderlyingType {
                    enum_id,
                    underlying_id,
                });
            }
        }
        Ok(())
    }

    /// Consume and return only a complete context. Capture adapters should use
    /// this as their final lowering step before publishing the context.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::validate_complete`] returns; the incomplete
    /// context is dropped rather than returned.
    pub fn into_complete(self) -> Result<Self, SerializerRegistrationError> {
        self.validate_complete()?;
        Ok(self)
    }

    /// Enumerate every captured callback/container capability that has no
    /// executable engine implementation. Pointer equality never promotes a
    /// codec or converter; only explicit registration removes an issue.
    #[must_use]
    pub fn executable_support_audit(&self) -> ExecutableSupportAudit {
        let mut issues = Vec::new();
        for (&class, data) in &self.classes {
            if data.captured_serializer
                && !self.codecs.contains_key(&class)
                && !self.builtin_serializers.contains_key(&class)
            {
                issues.push(ExecutableSupportIssue::CapturedSerializerMissingCodec {
                    class,
                    type_id: data.type_id,
                });
            }
            if data.captured_version_converter && !self.converters.contains_key(&class) {
                issues.push(
                    ExecutableSupportIssue::CapturedVersionConverterMissingImplementation {
                        class,
                        type_id: data.type_id,
                        deprecated: data.deprecated,
                    },
                );
            }
            if data.captured_data_converter && !self.data_converters.contains_key(&class) {
                issues.push(
                    ExecutableSupportIssue::CapturedDataConverterMissingImplementation {
                        class,
                        type_id: data.type_id,
                    },
                );
            }
            if data.is_container && data.container_shape.is_none() {
                issues.push(ExecutableSupportIssue::UnclassifiedContainer {
                    class,
                    type_id: data.type_id,
                });
            }
            if data.is_container && data.container_cardinality.is_none() {
                issues.push(ExecutableSupportIssue::MissingContainerCardinality {
                    class,
                    type_id: data.type_id,
                });
            }
        }
        issues.sort_by_key(ExecutableSupportIssue::class);
        ExecutableSupportAudit { issues }
    }

    /// Preflight one already-resolved DOM before any output is opened. Missing
    /// callbacks are rejected only when this graph actually requires them.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ObjectStreamError::IncompleteReadContext`] for an incomplete
    /// context, then, per element of `stream`,
    /// [`crate::ObjectStreamError::UnresolvedElementType`] for an unresolved node,
    /// [`crate::ObjectStreamError::UnsupportedSerializer`] or
    /// [`crate::ObjectStreamError::MissingSerializer`] for a payload with no
    /// executable codec,
    /// [`crate::ObjectStreamError::UnsupportedContainerSemantics`] and
    /// [`crate::ObjectStreamError::MissingContainerCardinality`] for a container
    /// with incomplete captured semantics,
    /// [`crate::ObjectStreamError::InvalidContainerCardinality`] for a child count
    /// the captured contract forbids, and
    /// [`crate::ObjectStreamError::UnexpectedReachableChild`] for an edge the parent
    /// `ClassData` does not reflect.
    pub fn validate_stream_executable_support(
        &self,
        stream: &crate::ObjectStream,
    ) -> Result<(), crate::ObjectStreamError> {
        self.validate_complete()?;
        crate::serialize::validate_context_graph(stream, self)
    }

    /// Insert one actual `uuidGenericMap` key. Callers insert every captured
    /// `registeredTypeIds` alias explicitly; aliases do not widen `CanStoreType`.
    ///
    /// Re-inserting an identical [`GenericClassInfo`] under the same key is a
    /// no-op rather than an error.
    pub fn insert_generic(&mut self, map_id: Uuid, info: GenericClassInfo) {
        let infos = self.generic_infos.entry(map_id).or_default();
        if !infos.contains(&info) {
            infos.push(info);
        }
    }

    /// Reconstruct native `m_legacySpecializeTypeIdToTypeIdMap` without
    /// widening the exact `uuidGenericMap` key set.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::UnknownGenericRedirectTarget`] if
    /// `target_id` is not itself a registered `uuidGenericMap` key. Repeating an
    /// existing redirect is a no-op rather than an error.
    pub fn insert_legacy_generic_redirect(
        &mut self,
        legacy_id: Uuid,
        target_id: Uuid,
    ) -> Result<(), SerializerRegistrationError> {
        if !self.generic_infos.contains_key(&target_id) {
            return Err(SerializerRegistrationError::UnknownGenericRedirectTarget {
                legacy_id,
                target_id,
            });
        }
        let targets = self.legacy_generic_redirects.entry(legacy_id).or_default();
        if !targets.contains(&target_id) {
            targets.push(target_id);
        }
        Ok(())
    }

    /// Record a captured `enumTypeIdToUnderlyingTypeIdMap` entry.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::ConflictingEnumUnderlyingType`] when
    /// `enum_id` is already mapped to a different underlying type. Repeating the
    /// same mapping is a no-op rather than an error.
    pub fn insert_enum_underlying_type(
        &mut self,
        enum_id: Uuid,
        underlying_id: Uuid,
    ) -> Result<(), SerializerRegistrationError> {
        match self.enum_underlying_types.entry(enum_id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(underlying_id);
                Ok(())
            }
            std::collections::hash_map::Entry::Occupied(entry) if *entry.get() == underlying_id => {
                Ok(())
            }
            std::collections::hash_map::Entry::Occupied(entry) => {
                Err(SerializerRegistrationError::ConflictingEnumUnderlyingType {
                    enum_id,
                    existing: *entry.get(),
                    requested: underlying_id,
                })
            }
        }
    }

    pub fn insert_rtti_base(&mut self, derived: Uuid, base: Uuid) {
        self.rtti_bases.entry(derived).or_default().push(base);
    }

    /// Resolve a captured `uuidMap` type identity to its registered `ClassData`
    /// key. This is the same resolution the runtime uses to turn a serialized
    /// type UUID into `ClassData`, so recipe features that name a class (version
    /// converters, field renames) bind to the exact `ClassData` the loader would.
    #[must_use]
    pub(crate) fn direct_class_key(&self, type_id: Uuid) -> Option<ReflectedClassKey> {
        self.direct_classes.get(&type_id).copied()
    }

    /// `true` iff the `ClassData` registered for `type_id` reflects a `ClassElement`
    /// with `name_crc`. Used to prove a field-rename target is real before the
    /// rename is accepted.
    #[must_use]
    pub(crate) fn class_field_exists(&self, type_id: Uuid, name_crc: u32) -> bool {
        self.direct_class_key(type_id)
            .and_then(|key| self.classes.get(&key))
            .is_some_and(|class| class.find_field(Some(name_crc)).is_some())
    }

    /// Register an import field-CRC rename on one class type. A reached child of
    /// `parent_type_id` whose name CRC is `from_name_crc` is rebound to
    /// `to_name_crc` during finalization.
    pub(crate) fn insert_field_rename(&mut self, parent_type_id: Uuid, from: u32, to: u32) {
        self.field_renames
            .entry(parent_type_id)
            .or_default()
            .insert(from, to);
    }

    /// The rebound name CRC for a reached child, if this parent class type has a
    /// registered import rename for the child's captured name CRC.
    fn renamed_child_name_crc(&self, parent_type_id: Uuid, child_name_crc: u32) -> Option<u32> {
        self.field_renames
            .get(&parent_type_id)
            .and_then(|renames| renames.get(&child_name_crc))
            .copied()
    }

    /// Register the executable counterpart of a captured
    /// `ClassData::m_converter`.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::UnknownClass`] if `class` is not
    /// registered, or [`SerializerRegistrationError::DeprecatedClass`] if it is
    /// deprecated without captured version-converter evidence — a deprecated class
    /// with no captured converter is discarded by native loading, so giving it one
    /// here would change the load result.
    pub fn insert_version_converter(
        &mut self,
        class: ReflectedClassKey,
        converter: Arc<dyn ObjectStreamVersionConverter>,
    ) -> Result<(), SerializerRegistrationError> {
        let class_data = self
            .classes
            .get_mut(&class)
            .ok_or(SerializerRegistrationError::UnknownClass { class })?;
        if class_data.deprecated && !class_data.captured_version_converter {
            return Err(SerializerRegistrationError::DeprecatedClass { class });
        }
        self.converters.insert(class, converter);
        Ok(())
    }

    /// Register the executable counterpart of a captured
    /// `ClassData::m_dataConverter`.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::UnknownClass`] if `class` is not
    /// registered, or [`SerializerRegistrationError::DataConverterNotCaptured`] if
    /// the capture recorded no `m_dataConverter` on it — this registration mirrors
    /// captured evidence and never invents a conversion edge.
    pub fn insert_data_converter(
        &mut self,
        class: ReflectedClassKey,
        converter: Arc<dyn ObjectStreamDataConverter>,
    ) -> Result<(), SerializerRegistrationError> {
        let class_data = self
            .classes
            .get_mut(&class)
            .ok_or(SerializerRegistrationError::UnknownClass { class })?;
        if !class_data.captured_data_converter {
            return Err(SerializerRegistrationError::DataConverterNotCaptured { class });
        }
        self.data_converters.insert(class, converter);
        Ok(())
    }

    /// Register the executable handler for one native
    /// `DataOverlayProviderBus` address.
    pub fn insert_data_overlay_provider(
        &mut self,
        provider_id: u32,
        provider: Arc<dyn ObjectStreamDataOverlayProvider>,
    ) -> Option<Arc<dyn ObjectStreamDataOverlayProvider>> {
        self.overlay_providers.insert(provider_id, provider)
    }

    /// Register the executable counterpart of a captured
    /// `ClassData::m_serializer`.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::UnknownClass`] if `class` is not
    /// registered, or [`SerializerRegistrationError::DeprecatedClass`] if it is
    /// deprecated — a deprecated class is discarded rather than decoded.
    pub fn insert_codec(
        &mut self,
        class: ReflectedClassKey,
        codec: Arc<dyn ObjectStreamValueCodec>,
    ) -> Result<(), SerializerRegistrationError> {
        let class_data = self
            .classes
            .get_mut(&class)
            .ok_or(SerializerRegistrationError::UnknownClass { class })?;
        if class_data.deprecated {
            return Err(SerializerRegistrationError::DeprecatedClass { class });
        }
        class_data.captured_serializer = true;
        self.builtin_serializers.remove(&class);
        self.codecs.insert(class, codec);
        Ok(())
    }

    /// Register a built-in codec only after the adapter has observed a
    /// serializer on this `ClassData`.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::UnsupportedSerializer`] if `type_id`
    /// is not one of the AZ builtin serializer UUIDs, plus every error
    /// [`Self::insert_builtin_codec_descriptor`] returns.
    pub fn insert_builtin_codec(
        &mut self,
        class: ReflectedClassKey,
        type_id: Uuid,
        serializer_version: u32,
    ) -> Result<(), SerializerRegistrationError> {
        let Some(kind) = builtin_serializer_kind(type_id) else {
            return Err(SerializerRegistrationError::UnsupportedSerializer { type_id });
        };
        self.insert_builtin_codec_descriptor(
            class,
            type_id,
            BuiltinSerializerDescriptor::new(kind, serializer_version),
        )
    }

    /// Register the exact captured platform/serializer descriptor on `ClassData`.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::UnsupportedSerializer`] if `type_id`
    /// is not one of the AZ builtin serializer UUIDs,
    /// [`SerializerRegistrationError::UnknownClass`] if `class` is not registered,
    /// [`SerializerRegistrationError::DeprecatedClass`] if it is deprecated,
    /// [`SerializerRegistrationError::DescriptorKindMismatch`] if `descriptor`
    /// names a different serializer kind than `type_id` implies, and
    /// [`SerializerRegistrationError::UnsupportedAssetVersion`] for an
    /// `AssetSerializer` descriptor above version 2.
    pub fn insert_builtin_codec_descriptor(
        &mut self,
        class: ReflectedClassKey,
        type_id: Uuid,
        descriptor: BuiltinSerializerDescriptor,
    ) -> Result<(), SerializerRegistrationError> {
        let Some(kind) = builtin_serializer_kind(type_id) else {
            return Err(SerializerRegistrationError::UnsupportedSerializer { type_id });
        };
        let class_data = self
            .classes
            .get(&class)
            .ok_or(SerializerRegistrationError::UnknownClass { class })?;
        if class_data.deprecated {
            return Err(SerializerRegistrationError::DeprecatedClass { class });
        }
        if descriptor.kind != kind {
            return Err(SerializerRegistrationError::DescriptorKindMismatch {
                type_id,
                expected: kind,
                actual: descriptor.kind,
            });
        }
        if kind == crate::codec::BuiltinSerializerKind::Asset && descriptor.current_version > 2 {
            return Err(SerializerRegistrationError::UnsupportedAssetVersion {
                version: descriptor.current_version,
            });
        }
        self.insert_codec(class, Arc::new(descriptor.codec()))?;
        self.builtin_serializers.insert(class, descriptor);
        Ok(())
    }

    /// Register an executable descriptor for a captured serializer whose
    /// semantic `ClassData` UUID is not one of the primitive UUIDs (for example
    /// `ranged_int<T, Min, Max>` or native `EmptySerializer`). The caller must
    /// provide exact captured serializer evidence; names and payload width do
    /// not authorize this path.
    ///
    /// # Errors
    ///
    /// Returns [`SerializerRegistrationError::UnknownClass`] if `class` is not
    /// registered, [`SerializerRegistrationError::DeprecatedClass`] if it is
    /// deprecated, and [`SerializerRegistrationError::UnsupportedSerializer`] if the
    /// capture recorded no `m_serializer` on that `ClassData` — this path needs
    /// captured serializer evidence, since the UUID itself is not a primitive.
    pub fn insert_captured_serializer_descriptor(
        &mut self,
        class: ReflectedClassKey,
        descriptor: BuiltinSerializerDescriptor,
    ) -> Result<(), SerializerRegistrationError> {
        let class_data = self
            .classes
            .get(&class)
            .ok_or(SerializerRegistrationError::UnknownClass { class })?;
        if class_data.deprecated {
            return Err(SerializerRegistrationError::DeprecatedClass { class });
        }
        if !class_data.captured_serializer {
            return Err(SerializerRegistrationError::UnsupportedSerializer {
                type_id: class_data.type_id,
            });
        }
        self.insert_codec(class, Arc::new(descriptor.codec()))?;
        self.builtin_serializers.insert(class, descriptor);
        Ok(())
    }

    #[must_use]
    pub fn codec(&self, class: ReflectedClassKey) -> Option<&dyn ObjectStreamValueCodec> {
        self.classes
            .get(&class)
            .filter(|class| !class.deprecated)
            .and_then(|_| self.codecs.get(&class))
            .map(AsRef::as_ref)
    }

    /// Check that `class` can legally carry (or omit) an inline payload.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ObjectStreamError::UnresolvedClassData`] if `class` is not
    /// registered, [`crate::ObjectStreamError::UnsupportedContainerSemantics`] and
    /// [`crate::ObjectStreamError::MissingContainerCardinality`] when a container's
    /// captured `IDataContainer` shape or size contract is missing,
    /// [`crate::ObjectStreamError::UnsupportedSerializer`] when a captured
    /// serializer has no registered executable codec, and
    /// [`crate::ObjectStreamError::MissingSerializer`] when `has_payload` is true on
    /// a class the capture records no serializer for.
    pub fn validate_payload_support(
        &self,
        class: ReflectedClassKey,
        has_payload: bool,
    ) -> Result<(), crate::ObjectStreamError> {
        let class_data = self
            .classes
            .get(&class)
            .ok_or(crate::ObjectStreamError::UnresolvedClassData { class })?;
        if class_data.is_container && class_data.container_shape.is_none() {
            return Err(crate::ObjectStreamError::UnsupportedContainerSemantics {
                type_id: class_data.type_id,
            });
        }
        if class_data.is_container && class_data.container_cardinality.is_none() {
            return Err(crate::ObjectStreamError::MissingContainerCardinality {
                type_id: class_data.type_id,
            });
        }
        if class_data.captured_serializer {
            return if self.codecs.contains_key(&class) {
                Ok(())
            } else {
                Err(crate::ObjectStreamError::UnsupportedSerializer {
                    type_id: class_data.type_id,
                })
            };
        }
        if has_payload {
            return Err(crate::ObjectStreamError::MissingSerializer {
                type_id: class_data.type_id,
            });
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns [`crate::ObjectStreamError::UnresolvedClassData`] if `class` is not
    /// registered, or [`crate::ObjectStreamError::InvalidContainerCardinality`] when
    /// `actual` exceeds a captured `Bounded` capacity or differs from a captured
    /// `Fixed` size. A `Variable` or absent contract accepts any count.
    pub fn validate_container_cardinality(
        &self,
        class: ReflectedClassKey,
        actual: usize,
    ) -> Result<(), crate::ObjectStreamError> {
        let class_data = self
            .classes
            .get(&class)
            .ok_or(crate::ObjectStreamError::UnresolvedClassData { class })?;
        let valid = match class_data.container_cardinality {
            None | Some(ContainerCardinality::Variable) => true,
            Some(ContainerCardinality::Bounded { capacity }) => actual <= capacity,
            Some(ContainerCardinality::Fixed { size }) => actual == size,
        };
        if valid {
            Ok(())
        } else {
            Err(crate::ObjectStreamError::InvalidContainerCardinality {
                type_id: class_data.type_id,
                expected: class_data.container_cardinality,
                actual,
            })
        }
    }

    /// Decide how a serialized element version reaches the current `ClassData`
    /// version.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ObjectStreamError::UnresolvedClassData`] if `class` is not
    /// registered, [`crate::ObjectStreamError::NewerClassVersion`] if
    /// `element_version` is above the reflected current version, and
    /// [`crate::ObjectStreamError::UnsupportedVersionConversion`] for an older
    /// element whose class captured a native version converter that this context has
    /// no executable registration for. A deprecated class with no converter is
    /// [`VersionConversionState::DeprecatedDiscard`], not an error.
    pub fn validate_class_version(
        &self,
        class: ReflectedClassKey,
        element_version: u32,
    ) -> Result<VersionConversionState, crate::ObjectStreamError> {
        let Some(class_data) = self.classes.get(&class) else {
            return Err(crate::ObjectStreamError::UnresolvedClassData { class });
        };
        if class_data.deprecated && !self.converters.contains_key(&class) {
            // Native LoadClass discards a version-0xFFFF_FFFF ClassData subtree
            // when its converter pointer is null. Only deprecated classes with
            // a converter enter the conversion path.
            return Ok(VersionConversionState::DeprecatedDiscard);
        }
        let current_version = class_data.current_version;
        if element_version > current_version {
            return Err(crate::ObjectStreamError::NewerClassVersion {
                type_id: class_data.type_id,
                element_version,
                current_version,
            });
        }
        if class_data.deprecated {
            return Ok(VersionConversionState::RegisteredConverter {
                from: element_version,
                to: current_version,
            });
        }
        // IDataSerializer::Load receives the serialized version and owns its
        // backward compatibility. Otherwise an executable registered
        // converter runs, or strict default structural matching validates that
        // native conversion would not discard any reachable child.
        if element_version < current_version {
            if self.codecs.contains_key(&class) {
                return Ok(VersionConversionState::SerializerCompatibleOld {
                    from: element_version,
                    to: current_version,
                });
            }
            if self.converters.contains_key(&class) {
                return Ok(VersionConversionState::RegisteredConverter {
                    from: element_version,
                    to: current_version,
                });
            }
            if class_data.captured_version_converter {
                return Err(crate::ObjectStreamError::UnsupportedVersionConversion {
                    type_id: class_data.type_id,
                    element_version,
                    current_version,
                });
            }
            return Ok(VersionConversionState::StrictDefaultStructural {
                from: element_version,
                to: current_version,
            });
        }
        Ok(VersionConversionState::Current)
    }

    /// Resolve and validate a programmatically constructed raw DOM using this
    /// captured `SerializeContext`. This is the only supported way for callers
    /// to obtain typed-reader proof without parsing an `ObjectStream` payload.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ObjectStreamError::IncompleteReadContext`] for an incomplete
    /// context, then every failure the reader's resolution pass can raise on this
    /// subtree: [`crate::ObjectStreamError::UnresolvedElementType`],
    /// [`crate::ObjectStreamError::AmbiguousGenericType`],
    /// [`crate::ObjectStreamError::DeprecatedClass`],
    /// [`crate::ObjectStreamError::NewerClassVersion`],
    /// [`crate::ObjectStreamError::UnsupportedVersionConversion`],
    /// [`crate::ObjectStreamError::UnsupportedDataConversion`],
    /// [`crate::ObjectStreamError::UnsupportedSerializer`],
    /// [`crate::ObjectStreamError::MissingSerializer`],
    /// [`crate::ObjectStreamError::InvalidContainerCardinality`],
    /// [`crate::ObjectStreamError::UnexpectedReachableChild`],
    /// [`crate::ObjectStreamError::InvalidDataOverlay`] and
    /// [`crate::ObjectStreamError::MissingDataOverlayProvider`].
    pub fn resolve_element_tree(
        &self,
        stream_version: u32,
        element: &mut crate::Element,
    ) -> Result<(), crate::ObjectStreamError> {
        self.validate_complete()?;
        crate::validate_stream_version(stream_version)?;
        // A programmatically-built DOM does not carry the source encoding.
        // Unlike the XML/JSON readers, it therefore cannot reproduce their
        // distinct native v2 missing-specialization behavior. Require the
        // specialization explicitly, matching the unambiguous binary form.
        if stream_version == 2 && element.specialization.is_none() {
            return Err(crate::ObjectStreamError::MissingV2Specialization {
                type_id: element.id,
            });
        }
        crate::validate_element_specialization(stream_version, element.id, element.specialization)?;
        let resolved = self.resolve_type(ResolveTypeInput {
            stream_version,
            raw_type_id: element.id,
            specialization_type_id: element.specialization,
            element_version: element.version.unwrap_or(0),
            name_crc: element.name_crc,
            parent: None,
        });
        Self::ensure_unambiguous(resolved, element.id)?;
        let (Some(type_id), Some(class)) = (resolved.type_id, resolved.class) else {
            element.resolution = crate::TypeResolution::Unresolved;
            return Err(crate::ObjectStreamError::UnresolvedElementType {
                type_id: element.id,
            });
        };
        let state = self.validate_class_version(class, element.version.unwrap_or(0))?;
        if state.discards_subtree() {
            return Err(crate::ObjectStreamError::DeprecatedClass { type_id });
        }
        self.validate_payload_support(class, element.data.is_some())?;
        element.resolution = crate::TypeResolution::Resolved {
            type_id,
            class,
            edge: None,
            enum_type_id: self.resolved_enum_type(None, element.name_crc, type_id),
            builtin_serializer: resolved.builtin_serializer,
            is_container: resolved.is_container,
            container_shape: resolved.container_shape,
        };
        element.version_state = state;
        if state.requires_dom_converter()
            || matches!(
                state,
                VersionConversionState::StrictDefaultStructural { .. }
            )
        {
            self.convert_element(class, state, element, stream_version, None)?;
        } else {
            self.reresolve_descendants(class, stream_version, &mut element.elements)?;
        }
        self.validate_container_cardinality(class, element.elements.len())?;
        if element.resolved_type_id() == Some(&types::DATA_OVERLAY_INFO) {
            *element = self.materialize_data_overlay(element, stream_version, None)?;
        }
        Ok(())
    }

    pub(crate) fn materialize_data_overlay(
        &self,
        overlay: &crate::Element,
        stream_version: u32,
        parent: Option<ReflectedClassKey>,
    ) -> Result<crate::Element, crate::ObjectStreamError> {
        let request = crate::overlay::decode_request(overlay)?;
        let provider = self.overlay_providers.get(&request.provider_id).ok_or(
            crate::ObjectStreamError::MissingDataOverlayProvider {
                provider_id: request.provider_id,
            },
        )?;
        let mut replacement = provider.fill_overlay_data(&request)?;

        // Lumberyard `Code/Framework/AzCore/AzCore/Serialization/ObjectStream.cpp`
        // restores the placeholder's m_name/m_nameCrc on the provider-produced
        // node before resuming LoadClass under the parent.
        replacement.field.clone_from(&overlay.field);
        replacement.name_crc = overlay.name_crc;

        if let Some(parent) = parent {
            let mut replacement_list = vec![replacement];
            self.reresolve_descendants(parent, stream_version, &mut replacement_list)?;
            Ok(replacement_list
                .pop()
                .expect("one provider replacement remains after resolution"))
        } else {
            self.resolve_programmatic_root(stream_version, &mut replacement)?;
            Ok(replacement)
        }
    }

    fn resolve_programmatic_root(
        &self,
        stream_version: u32,
        element: &mut crate::Element,
    ) -> Result<(), crate::ObjectStreamError> {
        crate::validate_element_specialization(stream_version, element.id, element.specialization)?;
        let resolved = self.resolve_type(ResolveTypeInput {
            stream_version,
            raw_type_id: element.id,
            specialization_type_id: element.specialization,
            element_version: element.version.unwrap_or(0),
            name_crc: element.name_crc,
            parent: None,
        });
        Self::ensure_unambiguous(resolved, element.id)?;
        let (Some(type_id), Some(class)) = (resolved.type_id, resolved.class) else {
            return Err(crate::ObjectStreamError::UnresolvedElementType {
                type_id: element.id,
            });
        };
        element.resolution = crate::TypeResolution::Resolved {
            type_id,
            class,
            edge: None,
            enum_type_id: self.resolved_enum_type(None, element.name_crc, type_id),
            builtin_serializer: resolved.builtin_serializer,
            is_container: resolved.is_container,
            container_shape: resolved.container_shape,
        };
        let state = self.validate_class_version(class, element.version.unwrap_or(0))?;
        if state.discards_subtree() {
            return Err(crate::ObjectStreamError::DeprecatedClass { type_id });
        }
        self.validate_payload_support(class, element.data.is_some())?;
        element.version_state = state;
        if matches!(
            state,
            VersionConversionState::RegisteredConverter { .. }
                | VersionConversionState::StrictDefaultStructural { .. }
        ) {
            self.convert_element(class, state, element, stream_version, None)?;
        } else {
            self.reresolve_descendants(class, stream_version, &mut element.elements)?;
        }
        self.validate_container_cardinality(
            element
                .resolved_class()
                .expect("resolved provider root retains ClassData"),
            element.elements.len(),
        )
    }

    /// Advance `element` to its current `ClassData` version and return the
    /// `ClassData` it ends up bound to.
    ///
    /// # Errors
    ///
    /// For a strict default structural pass, returns whatever re-resolving the
    /// descendants raises — chiefly
    /// [`crate::ObjectStreamError::UnresolvedElementType`] and
    /// [`crate::ObjectStreamError::UnexpectedReachableChild`], because this
    /// conversion is lossless where native loading would silently drop the stale
    /// child. For a registered converter, returns
    /// [`crate::ObjectStreamError::UnsupportedVersionConversion`] when no executable
    /// converter is registered or when the converter's output still needs another
    /// conversion, whatever the converter itself returns,
    /// [`crate::ObjectStreamError::MissingV2Specialization`] or
    /// [`crate::ObjectStreamError::UnexpectedSpecialization`] if it left an illegal
    /// specialization slot, [`crate::ObjectStreamError::UnresolvedElementType`] if
    /// its output no longer resolves, and every error
    /// [`Self::validate_payload_support`] and [`Self::validate_class_version`]
    /// return for the converted element.
    pub fn convert_element(
        &self,
        class: ReflectedClassKey,
        state: VersionConversionState,
        element: &mut crate::Element,
        stream_version: u32,
        parent: Option<ReflectedClassKey>,
    ) -> Result<ReflectedClassKey, crate::ObjectStreamError> {
        if let VersionConversionState::StrictDefaultStructural { to, .. } = state {
            // Native ConvertOldVersion advances the DataElementNode after its
            // default structural pass. The importer performs the same pass
            // losslessly: every descendant must still resolve through the
            // current ClassData graph, otherwise conversion fails instead of
            // discarding the stale child as non-strict runtime loading does.
            self.reresolve_descendants(class, stream_version, &mut element.elements)?;
            element.version = Some(to);
            element.version_state = VersionConversionState::Current;
            return Ok(class);
        }
        if let VersionConversionState::RegisteredConverter { from, to } = state {
            let source_class = class;
            let converter = self.converters.get(&class).ok_or(
                crate::ObjectStreamError::UnsupportedVersionConversion {
                    type_id: element.id,
                    element_version: from,
                    current_version: to,
                },
            )?;
            let source_version = element.version;
            converter.convert(element, from, to)?;
            crate::validate_element_specialization(
                stream_version,
                element.id,
                element.specialization,
            )?;
            let callback_supplied_version = element.version != source_version;
            let resolution_version = if callback_supplied_version {
                element.version.unwrap_or(0)
            } else {
                from
            };
            let resolved = self.resolve_type(ResolveTypeInput {
                stream_version,
                raw_type_id: element.id,
                specialization_type_id: element.specialization,
                element_version: resolution_version,
                name_crc: element.name_crc,
                parent,
            });
            let (Some(type_id), Some(class)) = (resolved.type_id, resolved.class) else {
                return Err(crate::ObjectStreamError::UnresolvedElementType {
                    type_id: element.id,
                });
            };
            self.validate_payload_support(class, element.data.is_some())?;
            let converted_version = if class == source_class {
                to
            } else if callback_supplied_version {
                element.version.unwrap_or(
                    self.classes
                        .get(&class)
                        .ok_or(crate::ObjectStreamError::UnresolvedElementType { type_id })?
                        .current_version,
                )
            } else {
                self.classes
                    .get(&class)
                    .map(|class| class.current_version)
                    .ok_or(crate::ObjectStreamError::UnresolvedElementType { type_id })?
            };
            element.version = Some(converted_version);
            let next_state = self.validate_class_version(class, converted_version)?;
            if let VersionConversionState::RegisteredConverter { from, to } = next_state {
                return Err(crate::ObjectStreamError::UnsupportedVersionConversion {
                    type_id,
                    element_version: from,
                    current_version: to,
                });
            }
            element.resolution = crate::TypeResolution::Resolved {
                type_id,
                class,
                edge: self.resolved_edge(parent, element.name_crc),
                enum_type_id: self.resolved_enum_type(parent, element.name_crc, type_id),
                builtin_serializer: resolved.builtin_serializer,
                is_container: resolved.is_container,
                container_shape: resolved.container_shape,
            };
            element.version_state = next_state;
            if matches!(
                next_state,
                VersionConversionState::StrictDefaultStructural { .. }
            ) {
                self.convert_element(class, next_state, element, stream_version, parent)?;
            } else {
                self.reresolve_descendants(class, stream_version, &mut element.elements)?;
            }
            return Ok(class);
        }
        Ok(class)
    }

    fn reresolve_descendants(
        &self,
        parent: ReflectedClassKey,
        stream_version: u32,
        children: &mut Vec<crate::Element>,
    ) -> Result<(), crate::ObjectStreamError> {
        // Native default-filter loading may discard an unbindable child; rebuild
        // the list retaining only the children `finalize_reachable_child` keeps.
        let mut retained = Vec::with_capacity(children.len());
        for mut child in std::mem::take(children) {
            // Provider output is produced by enumerating an in-memory
            // instance, matching Lumberyard's DataOverlayTarget; it cannot
            // contain another placeholder inserted by ObjectStream save.
            if child.id == types::DATA_OVERLAY_INFO {
                return Err(crate::ObjectStreamError::InvalidDataOverlay(
                    "provider replacement contains a nested DataOverlayInfo placeholder".into(),
                ));
            }
            crate::validate_element_specialization(stream_version, child.id, child.specialization)?;
            let resolved = self.resolve_type(ResolveTypeInput {
                stream_version,
                raw_type_id: child.id,
                specialization_type_id: child.specialization,
                element_version: child.version.unwrap_or(0),
                name_crc: child.name_crc,
                parent: Some(parent),
            });
            let (Some(type_id), Some(class)) = (resolved.type_id, resolved.class) else {
                return Err(crate::ObjectStreamError::UnresolvedElementType { type_id: child.id });
            };
            child.resolution = crate::TypeResolution::Resolved {
                type_id,
                class,
                edge: self.resolved_edge(Some(parent), child.name_crc),
                enum_type_id: self.resolved_enum_type(Some(parent), child.name_crc, type_id),
                builtin_serializer: resolved.builtin_serializer,
                is_container: resolved.is_container,
                container_shape: resolved.container_shape,
            };
            let state = self.validate_class_version(class, child.version.unwrap_or(0))?;
            if state.discards_subtree() {
                continue;
            }
            self.validate_payload_support(class, child.data.is_some())?;
            child.version_state = state;
            if matches!(
                state,
                VersionConversionState::RegisteredConverter { .. }
                    | VersionConversionState::StrictDefaultStructural { .. }
            ) {
                self.convert_element(class, state, &mut child, stream_version, Some(parent))?;
            } else {
                self.reresolve_descendants(class, stream_version, &mut child.elements)?;
            }
            self.validate_container_cardinality(class, child.elements.len())?;
            match self.finalize_reachable_child(parent, &mut child, stream_version)? {
                ChildFate::Retain => retained.push(child),
                ChildFate::Skip => {}
            }
        }
        *children = retained;
        Ok(())
    }

    /// Validate a parent/child DOM edge, allowing a registered data converter to
    /// account for a type mismatch.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ObjectStreamError::UnresolvedClassData`] if `parent` is not
    /// registered, [`crate::ObjectStreamError::UnresolvedElementType`] if `child`
    /// carries no resolved identity,
    /// [`crate::ObjectStreamError::UnsupportedDataConversion`] when the parent's
    /// field would need a captured `m_dataConverter` this context cannot execute,
    /// and [`crate::ObjectStreamError::UnexpectedReachableChild`] when the parent
    /// `ClassData` reflects no field this child can bind to — unless the context's
    /// unmatched-child policy is set to skip.
    pub fn validate_reachable_child(
        &self,
        parent: ReflectedClassKey,
        child_name_crc: Option<u32>,
        child_raw_type: Uuid,
        child: ResolvedType,
    ) -> Result<(), crate::ObjectStreamError> {
        self.validate_reachable_child_impl(parent, child_name_crc, child_raw_type, child, true)
    }

    /// Validate a DOM edge that must already be in its final converted form.
    /// Writers use this path because preflight is read-only and must never
    /// treat the mere availability of a data converter as if it had run.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::validate_reachable_child`], except that a
    /// mismatch a data converter could have fixed is
    /// [`crate::ObjectStreamError::UnexpectedReachableChild`] here: this path is
    /// read-only preflight and must not treat an unrun converter as if it had run.
    pub fn validate_finalized_reachable_child(
        &self,
        parent: ReflectedClassKey,
        child_name_crc: Option<u32>,
        child_raw_type: Uuid,
        child: ResolvedType,
    ) -> Result<(), crate::ObjectStreamError> {
        self.validate_reachable_child_impl(parent, child_name_crc, child_raw_type, child, false)
    }

    fn validate_reachable_child_impl(
        &self,
        parent: ReflectedClassKey,
        child_name_crc: Option<u32>,
        child_raw_type: Uuid,
        child: ResolvedType,
        allow_data_conversion: bool,
    ) -> Result<(), crate::ObjectStreamError> {
        let parent_class = self
            .classes
            .get(&parent)
            .ok_or(crate::ObjectStreamError::UnresolvedClassData { class: parent })?;
        let child_type = child
            .type_id
            .ok_or(crate::ObjectStreamError::UnresolvedElementType {
                type_id: child_raw_type,
            })?;
        if parent_class.type_id == types::DYNAMIC_SERIALIZABLE_FIELD
            && child_name_crc == Some(crate::field_name_crc("m_data"))
        {
            return child
                .class
                .filter(|class| self.classes.contains_key(class))
                .map(|_| ())
                .ok_or(crate::ObjectStreamError::UnresolvedElementType {
                    type_id: child_raw_type,
                });
        }
        let field = parent_class.find_field(child_name_crc).ok_or(
            crate::ObjectStreamError::UnexpectedReachableChild {
                parent_type_id: parent_class.type_id,
                child_type_id: child_raw_type,
                name_crc: child_name_crc,
            },
        )?;
        if field.is_dynamic_field {
            return child
                .class
                .filter(|class| self.classes.contains_key(class))
                .map(|_| ())
                .ok_or(crate::ObjectStreamError::UnresolvedElementType {
                    type_id: child_raw_type,
                });
        }
        let generic_match = field
            .generic
            .as_ref()
            .is_some_and(|generic| generic.can_store_type(child_type));
        let enum_match = self.enum_underlying_types.get(&field.type_id) == Some(&child_type)
            || self.enum_underlying_types.get(&child_type) == Some(&field.type_id);
        // Lumberyard gates derived-object acceptance on FLG_POINTER. A base
        // class element is still stored by value and must match its reflected
        // type exactly; FLG_BASE_CLASS controls hierarchy traversal, not
        // polymorphic storage.
        let rtti_match = field.is_pointer && self.is_rtti_derived_from(child_type, field.type_id);
        if field.type_id == child_type || generic_match || enum_match || rtti_match {
            Ok(())
        } else {
            let target_class = self.find_class_data(field.type_id, Some(parent), child_name_crc);
            if allow_data_conversion
                && target_class.is_some_and(|class| self.data_converters.contains_key(&class))
            {
                return Ok(());
            }
            if allow_data_conversion
                && target_class
                    .and_then(|class| self.classes.get(&class))
                    .is_some_and(|class| class.captured_data_converter)
            {
                return Err(crate::ObjectStreamError::UnsupportedDataConversion {
                    target_type_id: field.type_id,
                    source_type_id: child_type,
                });
            }
            Err(crate::ObjectStreamError::UnexpectedReachableChild {
                parent_type_id: parent_class.type_id,
                child_type_id: child_type,
                name_crc: child_name_crc,
            })
        }
    }

    /// Apply native `ClassData` field compatibility after a complete child DOM
    /// subtree exists. This is the import-DOM analogue of `CanConvertFromType`
    /// plus `ConvertFromType`; it changes semantic identity only through an
    /// explicitly registered executable converter.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ObjectStreamError::UnresolvedClassData`] if `parent` is not
    /// registered, [`crate::ObjectStreamError::UnresolvedElementType`] if `child`
    /// does not resolve before or after conversion,
    /// [`crate::ObjectStreamError::UnsupportedDataConversion`] when the target field
    /// captured an `m_dataConverter` with no executable registration,
    /// [`crate::ObjectStreamError::UnexpectedReachableChild`] when no field accepts
    /// the child and the unmatched-child policy is strict, whatever the data
    /// converter itself returns, and every error
    /// [`Self::validate_payload_support`] and
    /// [`Self::validate_container_cardinality`] return for the converted subtree.
    pub fn finalize_reachable_child(
        &self,
        parent: ReflectedClassKey,
        child: &mut crate::Element,
        stream_version: u32,
    ) -> Result<ChildFate, crate::ObjectStreamError> {
        self.apply_import_field_rename(parent, child);
        let resolved = self.resolve_type(ResolveTypeInput {
            stream_version,
            raw_type_id: child.id,
            specialization_type_id: child.specialization,
            element_version: child.version.unwrap_or(0),
            name_crc: child.name_crc,
            parent: Some(parent),
        });
        let (Some(source_type_id), Some(source_class)) = (resolved.type_id, resolved.class) else {
            return Err(crate::ObjectStreamError::UnresolvedElementType { type_id: child.id });
        };
        child.resolution = crate::TypeResolution::Resolved {
            type_id: source_type_id,
            class: source_class,
            edge: self.resolved_edge(Some(parent), child.name_crc),
            enum_type_id: self.resolved_enum_type(Some(parent), child.name_crc, source_type_id),
            builtin_serializer: resolved.builtin_serializer,
            is_container: resolved.is_container,
            container_shape: resolved.container_shape,
        };
        let parent_data = self
            .classes
            .get(&parent)
            .ok_or(crate::ObjectStreamError::UnresolvedClassData { class: parent })?;
        // Canonicalize the child's field spelling onto the owning class'
        // registration. The wire carries only the case-folded `AZ::Crc32`, so
        // the header-time name came from the global CRC → name table, which
        // holds a single spelling for a hash several classes register
        // differently (`TimelineInstanceDebugDescriptor::DebugName` vs
        // `FEDebugName::debugName`). Only the parent ClassData knows which
        // spelling this slot was reflected under.
        if let Some(name) = parent_data
            .find_field(child.name_crc)
            .and_then(|field| field.name.as_ref())
        {
            child.field = Some(name.clone());
        }
        if parent_data.type_id == types::DYNAMIC_SERIALIZABLE_FIELD
            && child.name_crc == Some(crate::field_name_crc("m_data"))
        {
            return Ok(ChildFate::Retain);
        }
        let Some(field) = parent_data.find_field(child.name_crc) else {
            // Native `classElement == nullptr`: strict fails, default-filter skips.
            return self.unmatched_child_fate(parent_data.type_id, child.id, child.name_crc);
        };
        if field.is_dynamic_field || self.field_accepts_type(field, source_type_id) {
            return Ok(ChildFate::Retain);
        }
        let target_class = self
            .find_class_data(field.type_id, Some(parent), child.name_crc)
            .filter(|class| self.classes.contains_key(class));
        let Some(target_class) = target_class else {
            return self.unmatched_child_fate(parent_data.type_id, source_type_id, child.name_crc);
        };
        let Some(converter) = self.data_converters.get(&target_class) else {
            if self
                .classes
                .get(&target_class)
                .is_some_and(|class| class.captured_data_converter)
            {
                // A captured native dataConverter we cannot execute is OUR gap,
                // not something native default-filter loading would skip.
                return Err(crate::ObjectStreamError::UnsupportedDataConversion {
                    target_type_id: field.type_id,
                    source_type_id,
                });
            }
            return self.unmatched_child_fate(parent_data.type_id, source_type_id, child.name_crc);
        };
        let edge = ConvertedChildEdge {
            parent,
            parent_type_id: parent_data.type_id,
            stream_version,
        };
        let field_type_id = field.type_id;
        let accepts = |type_id| self.field_accepts_type(field, type_id);
        converter.convert(child, source_type_id, field_type_id)?;
        self.rebind_converted_child(edge, child, accepts)
    }

    /// Rewrite a reached child's captured name CRC onto the registered field
    /// spelling when the recipe declares an import rename for this parent class.
    ///
    /// Without it the payload would be dropped as an unmatched child.
    /// Idempotent: a second finalize pass sees the already-canonical CRC and
    /// makes no change.
    fn apply_import_field_rename(&self, parent: ReflectedClassKey, child: &mut crate::Element) {
        if let (Some(parent_type_id), Some(child_name_crc)) = (
            self.classes.get(&parent).map(|class| class.type_id),
            child.name_crc,
        ) && let Some(renamed) = self.renamed_child_name_crc(parent_type_id, child_name_crc)
        {
            child.name_crc = Some(renamed);
            if let Some(name) = self.names.crcs.get(&renamed) {
                child.field = Some(name.clone());
            }
        }
    }

    /// Re-resolve a child that a native `ClassData` data converter just rewrote,
    /// then rebind it and its subtree to the converted `ClassData`.
    fn rebind_converted_child(
        &self,
        edge: ConvertedChildEdge,
        child: &mut crate::Element,
        field_accepts: impl Fn(Uuid) -> bool,
    ) -> Result<ChildFate, crate::ObjectStreamError> {
        let ConvertedChildEdge {
            parent,
            parent_type_id,
            stream_version,
        } = edge;
        crate::validate_element_specialization(stream_version, child.id, child.specialization)?;
        let rebound = self.resolve_type(ResolveTypeInput {
            stream_version,
            raw_type_id: child.id,
            specialization_type_id: child.specialization,
            element_version: child.version.unwrap_or(0),
            name_crc: child.name_crc,
            parent: Some(parent),
        });
        let (Some(type_id), Some(class)) = (rebound.type_id, rebound.class) else {
            return Err(crate::ObjectStreamError::UnresolvedElementType { type_id: child.id });
        };
        self.validate_payload_support(class, child.data.is_some())?;
        if !field_accepts(type_id) {
            return self.unmatched_child_fate(parent_type_id, type_id, child.name_crc);
        }
        child.version = Some(
            self.classes
                .get(&class)
                .ok_or(crate::ObjectStreamError::UnresolvedElementType { type_id })?
                .current_version,
        );
        child.version_state = VersionConversionState::Current;
        child.resolution = crate::TypeResolution::Resolved {
            type_id,
            class,
            edge: self.resolved_edge(Some(parent), child.name_crc),
            enum_type_id: self.resolved_enum_type(Some(parent), child.name_crc, type_id),
            builtin_serializer: rebound.builtin_serializer,
            is_container: rebound.is_container,
            container_shape: rebound.container_shape,
        };
        self.reresolve_descendants(class, stream_version, &mut child.elements)?;
        self.validate_container_cardinality(class, child.elements.len())?;
        Ok(ChildFate::Retain)
    }

    fn field_accepts_type(&self, field: &ReflectedField, child_type: Uuid) -> bool {
        field.type_id == child_type
            || field
                .generic
                .as_ref()
                .is_some_and(|generic| generic.can_store_type(child_type))
            || self.enum_underlying_types.get(&field.type_id) == Some(&child_type)
            || self.enum_underlying_types.get(&child_type) == Some(&field.type_id)
            || (field.is_pointer && self.is_rtti_derived_from(child_type, field.type_id))
    }

    fn is_rtti_derived_from(&self, derived: Uuid, base: Uuid) -> bool {
        let mut pending = vec![derived];
        let mut visited = std::collections::HashSet::new();
        while let Some(current) = pending.pop() {
            if !visited.insert(current) {
                continue;
            }
            if current == base {
                return true;
            }
            if let Some(bases) = self.rtti_bases.get(&current) {
                pending.extend(bases.iter().copied());
            }
        }
        false
    }

    fn find_class_data(
        &self,
        candidate: Uuid,
        parent: Option<ReflectedClassKey>,
        name_crc: Option<u32>,
    ) -> Option<ReflectedClassKey> {
        // Native precedence: uuidMap always wins over parent/generic aliases.
        if let Some(class) = self.direct_classes.get(&candidate) {
            return Some(*class);
        }

        if let Some(parent) = parent
            && let Some(field) = self
                .classes
                .get(&parent)
                .and_then(|class| class.find_field(name_crc))
            && let Some(generic) = field.generic.as_ref()
            && generic.can_store_type(candidate)
        {
            return Some(generic.class());
        }

        if let Some(generic) = self.find_generic_info(candidate) {
            return Some(generic.class());
        }

        self.enum_underlying_types
            .get(&candidate)
            .and_then(|underlying| self.direct_classes.get(underlying))
            .copied()
    }

    pub(crate) fn find_generic_info(&self, type_id: Uuid) -> Option<&GenericClassInfo> {
        fn unique_info(infos: &[GenericClassInfo]) -> Option<&GenericClassInfo> {
            let first = infos.first()?;
            infos.iter().all(|info| info == first).then_some(first)
        }

        if let Some(infos) = self.generic_infos.get(&type_id) {
            return unique_info(infos);
        }
        let targets = self.legacy_generic_redirects.get(&type_id)?;
        let mut selected: Option<&GenericClassInfo> = None;
        for target in targets {
            for info in self.generic_infos.get(target)? {
                match selected {
                    None => selected = Some(info),
                    Some(existing) if existing == info => {}
                    Some(_) => return None,
                }
            }
        }
        selected
    }

    fn is_ambiguous_generic(&self, type_id: Uuid) -> bool {
        fn has_distinct(infos: impl IntoIterator<Item = GenericClassInfo>) -> bool {
            let mut infos = infos.into_iter();
            let Some(first) = infos.next() else {
                return false;
            };
            infos.any(|info| info != first)
        }

        if let Some(infos) = self.generic_infos.get(&type_id) {
            return has_distinct(infos.iter().cloned());
        }
        let Some(targets) = self.legacy_generic_redirects.get(&type_id) else {
            return false;
        };
        has_distinct(
            targets
                .iter()
                .filter_map(|target| self.generic_infos.get(target))
                .flatten()
                .cloned(),
        )
    }
}

impl ObjectStreamTypeResolver for ObjectStreamReadContext {
    fn resolve_type(&self, input: ResolveTypeInput) -> ResolvedType {
        let is_lumberyard_asset =
            self.dialect == ObjectStreamDialect::Lumberyard && input.raw_type_id == types::ASSET;
        let mut candidate = if input.stream_version == 2 && !is_lumberyard_asset {
            input.specialization_type_id.unwrap_or(input.raw_type_id)
        } else {
            input.raw_type_id
        };
        let should_lookup_specialization = match self.dialect {
            ObjectStreamDialect::Lumberyard => input.raw_type_id != types::ASSET,
            ObjectStreamDialect::O3de => {
                !(input.raw_type_id == types::ASSET && input.element_version <= 2)
            }
        };

        if input.stream_version == 2
            && should_lookup_specialization
            && let Some(parent) = input.parent
            && let Some(parent_class) = self.classes.get(&parent)
            && parent_class.is_container
            && let Some(generic) = parent_class
                .find_field(input.name_crc)
                .and_then(|field| field.generic.as_ref())
            && generic.can_store_type(candidate)
        {
            candidate = generic.specialized_type_id();
        }

        let class = self
            .find_class_data(candidate, input.parent, input.name_crc)
            .filter(|class| self.classes.contains_key(class));
        let mut ambiguous_generic = class.is_none() && self.is_ambiguous_generic(candidate);
        if should_lookup_specialization
            && let Some(class) = class
            && let Some(class_data) = self.classes.get(&class)
        {
            if let Some(generic) = self.find_generic_info(class_data.type_id) {
                candidate = generic.specialized_type_id();
            } else {
                ambiguous_generic |= self.is_ambiguous_generic(class_data.type_id);
            }
        }

        ResolvedType {
            type_id: class.map(|_| candidate),
            builtin_serializer: class
                .and_then(|class| self.builtin_serializers.get(&class).copied()),
            is_container: class
                .and_then(|class| self.classes.get(&class))
                .is_some_and(|class| class.is_container),
            container_shape: class
                .and_then(|class| self.classes.get(&class))
                .and_then(|class| class.container_shape),
            ambiguous_generic,
            class,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct ReplaceType(Uuid);

    impl ObjectStreamVersionConverter for ReplaceType {
        fn convert(
            &self,
            element: &mut crate::Element,
            _from: u32,
            _to: u32,
        ) -> Result<(), crate::ObjectStreamError> {
            element.id = self.0;
            element.specialization = None;
            Ok(())
        }
    }

    #[derive(Debug)]
    struct ConvertType(Uuid);

    impl ObjectStreamDataConverter for ConvertType {
        fn convert(
            &self,
            element: &mut crate::Element,
            _source_type_id: Uuid,
            _target_type_id: Uuid,
        ) -> Result<(), crate::ObjectStreamError> {
            element.id = self.0;
            element.specialization = None;
            Ok(())
        }
    }

    fn generic(
        class: Uuid,
        specialized: Uuid,
        generic: Uuid,
        legacy: Option<Uuid>,
    ) -> GenericClassInfo {
        GenericClassInfo::new(
            ReflectedClassKey::new(class),
            specialized,
            Some(generic),
            legacy,
        )
    }

    fn input(type_id: Uuid) -> ResolveTypeInput {
        ResolveTypeInput {
            stream_version: 3,
            raw_type_id: type_id,
            specialization_type_id: None,
            element_version: 0,
            name_crc: None,
            parent: None,
        }
    }

    #[test]
    fn direct_class_lookup_precedes_parent_generic() {
        let parent_id = Uuid::from_u128(1);
        let candidate = Uuid::from_u128(2);
        let generic_class = Uuid::from_u128(3);
        let mut context = ObjectStreamReadContext::default();
        let parent_key = context.reserve_class_data();
        let candidate_key = context.reserve_class_data();
        let generic_key = context.reserve_class_data();
        let mut parent = ReflectedClass::new(parent_id);
        parent.insert_field(
            7,
            ReflectedField::new(generic_class).with_generic(GenericClassInfo::new(
                generic_key,
                candidate,
                Some(Uuid::from_u128(4)),
                None,
            )),
        );
        context.define_class_data(parent_key, parent).unwrap();
        context
            .define_class_data(candidate_key, ReflectedClass::new(candidate))
            .unwrap();
        context
            .define_class_data(generic_key, ReflectedClass::new(generic_class))
            .unwrap();
        context
            .insert_direct_class_alias(parent_id, parent_key)
            .unwrap();
        context
            .insert_direct_class_alias(candidate, candidate_key)
            .unwrap();

        let resolved = context.resolve_type(ResolveTypeInput {
            parent: Some(parent_key),
            name_crc: Some(7),
            ..input(candidate)
        });
        assert_eq!(resolved.class, Some(candidate_key));
    }

    #[test]
    fn generic_metadata_cannot_resolve_an_uncaptured_class() {
        let alias = Uuid::from_u128(5);
        let missing_class = Uuid::from_u128(6);
        let mut context = ObjectStreamReadContext::default();
        context.insert_generic(
            alias,
            generic(missing_class, alias, Uuid::from_u128(7), None),
        );

        let resolved = context.resolve_type(input(alias));

        assert_eq!(resolved.type_id, None);
        assert_eq!(resolved.class, None);
        assert!(matches!(
            context.validate_complete(),
            Err(SerializerRegistrationError::UndefinedGenericClass {
                map_id,
                class,
            }) if map_id == alias && class == ReflectedClassKey::new(missing_class)
        ));
    }

    #[test]
    fn absent_generic_type_id_does_not_authorize_nil_or_unrelated_aliases() {
        let class = ReflectedClassKey::new(Uuid::from_u128(8));
        let specialized = Uuid::from_u128(9);
        let info = GenericClassInfo::new(class, specialized, None, None);

        assert!(info.can_store_type(specialized));
        assert!(!info.can_store_type(Uuid::nil()));
        assert!(!info.can_store_type(Uuid::from_u128(10)));
    }

    #[test]
    fn generic_only_classdata_is_not_promoted_to_direct_uuid_map() {
        let class_type_id = Uuid::from_u128(12);
        let alias = Uuid::from_u128(13);
        let mut context = ObjectStreamReadContext::default();
        let generic_class =
            context.insert_class_data(ReflectedClass::new(class_type_id).as_container());
        context.insert_generic(
            alias,
            GenericClassInfo::new(generic_class, alias, None, None),
        );

        assert_eq!(context.resolve_type(input(class_type_id)).class, None);
        assert_eq!(
            context.resolve_type(input(alias)).class,
            Some(generic_class)
        );

        let direct = context
            .insert_class(class_type_id, ReflectedClass::new(class_type_id))
            .unwrap();
        assert_eq!(
            context.resolve_type(input(class_type_id)).class,
            Some(direct)
        );
    }

    #[test]
    fn resolver_preserves_captured_container_family() {
        let sequence = Uuid::from_u128(14);
        let opaque_container = Uuid::from_u128(15);
        let mut context = ObjectStreamReadContext::default();
        context
            .insert_class(
                sequence,
                ReflectedClass::new(sequence).with_container_shape(ContainerShape::Sequence),
            )
            .unwrap();
        context
            .insert_class(
                opaque_container,
                ReflectedClass::new(opaque_container).as_container(),
            )
            .unwrap();

        let resolved_sequence = context.resolve_type(input(sequence));
        assert!(resolved_sequence.is_container);
        assert_eq!(
            resolved_sequence.container_shape,
            Some(ContainerShape::Sequence)
        );

        let unresolved_family = context.resolve_type(input(opaque_container));
        assert!(unresolved_family.is_container);
        assert_eq!(unresolved_family.container_shape, None);
    }

    #[test]
    fn class_version_defaults_to_zero_and_rejects_future_elements() {
        let type_id = Uuid::from_u128(16);
        let mut context = ObjectStreamReadContext::default();
        let class = context
            .insert_class(type_id, ReflectedClass::new(type_id))
            .unwrap();

        assert_eq!(
            context.validate_class_version(class, 0).unwrap(),
            VersionConversionState::Current
        );
        assert!(matches!(
            context.validate_class_version(class, 1),
            Err(crate::ObjectStreamError::NewerClassVersion {
                element_version: 1,
                current_version: 0,
                ..
            })
        ));
    }

    #[test]
    fn deprecated_class_uses_executable_converter_even_at_version_zero() {
        let deprecated_id = Uuid::from_u128(17);
        let replacement_id = Uuid::from_u128(18);
        let mut context = ObjectStreamReadContext::default();
        let deprecated = context
            .insert_class(
                deprecated_id,
                ReflectedClass::new(deprecated_id)
                    .deprecated()
                    .with_captured_version_converter(),
            )
            .unwrap();
        context
            .insert_class(replacement_id, ReflectedClass::new(replacement_id))
            .unwrap();
        context
            .insert_version_converter(deprecated, Arc::new(ReplaceType(replacement_id)))
            .unwrap();
        let state = context.validate_class_version(deprecated, 0).unwrap();
        assert_eq!(
            state,
            VersionConversionState::RegisteredConverter { from: 0, to: 0 }
        );

        let mut element = crate::Element::new(deprecated_id);
        element.version = Some(0);
        context
            .convert_element(deprecated, state, &mut element, 3, None)
            .unwrap();
        assert_eq!(element.resolved_type_id(), Some(&replacement_id));
    }

    #[test]
    fn deprecated_class_without_native_converter_is_a_supported_discard() {
        let deprecated_id = Uuid::from_u128(19);
        let mut context = ObjectStreamReadContext::default();
        let deprecated = context
            .insert_class(
                deprecated_id,
                ReflectedClass::new(deprecated_id).deprecated(),
            )
            .unwrap();

        assert_eq!(
            context.validate_class_version(deprecated, 0).unwrap(),
            VersionConversionState::DeprecatedDiscard
        );
        assert!(
            context.executable_support_audit().is_fully_supported(),
            "native discard requires no executable converter"
        );
    }

    #[test]
    fn container_lookup_rejects_missing_or_wrong_crc() {
        let parent_id = Uuid::from_u128(10);
        let child_id = Uuid::from_u128(11);
        let alias = Uuid::from_u128(12);
        let mut parent = ReflectedClass::new(parent_id).as_container();
        parent.insert_field(
            9,
            ReflectedField::new(child_id).with_generic(generic(
                child_id,
                alias,
                Uuid::from_u128(13),
                None,
            )),
        );
        let mut context = ObjectStreamReadContext::default();
        let parent = context.insert_class(parent_id, parent).unwrap();
        context
            .insert_class(child_id, ReflectedClass::new(child_id))
            .unwrap();

        for name_crc in [None, Some(8)] {
            let resolved = context.resolve_type(ResolveTypeInput {
                parent: Some(parent),
                name_crc,
                ..input(alias)
            });
            assert_eq!(
                resolved,
                ResolvedType {
                    type_id: None,
                    class: None,
                    builtin_serializer: None,
                    is_container: false,
                    container_shape: None,
                    ambiguous_generic: false,
                }
            );
        }
    }

    #[test]
    fn can_store_uses_exact_native_triple_and_alias_map_is_separate() {
        let class_id = Uuid::from_u128(20);
        let specialized = Uuid::from_u128(21);
        let generic_id = Uuid::from_u128(22);
        let legacy = Uuid::from_u128(23);
        let registered_alias = Uuid::from_u128(24);
        let mut context = ObjectStreamReadContext::default();
        let class = context
            .insert_class(class_id, ReflectedClass::new(class_id))
            .unwrap();
        let info = GenericClassInfo::new(class, specialized, Some(generic_id), Some(legacy));
        assert!(info.can_store_type(specialized));
        assert!(info.can_store_type(generic_id));
        assert!(info.can_store_type(legacy));
        assert!(!info.can_store_type(registered_alias));

        context.insert_generic(registered_alias, info);
        assert_eq!(
            context.resolve_type(input(registered_alias)).class,
            Some(class)
        );
    }

    #[test]
    fn lumberyard_v2_asset_ignores_specialization_for_every_element_version() {
        let specialized = Uuid::from_u128(31);
        let mut context =
            ObjectStreamReadContext::default().with_dialect(ObjectStreamDialect::Lumberyard);
        let asset = context
            .insert_class(types::ASSET, ReflectedClass::new(types::ASSET))
            .unwrap();
        context
            .insert_class(specialized, ReflectedClass::new(specialized))
            .unwrap();

        let resolved = context.resolve_type(ResolveTypeInput {
            stream_version: 2,
            raw_type_id: types::ASSET,
            specialization_type_id: Some(specialized),
            element_version: 9,
            name_crc: None,
            parent: None,
        });

        assert_eq!(resolved.type_id, Some(types::ASSET));
        assert_eq!(resolved.class, Some(asset));
    }

    #[test]
    fn o3de_v2_asset_keeps_versioned_specialization_policy() {
        let parent_id = Uuid::from_u128(40);
        let serialized_specialization = Uuid::from_u128(41);
        let rebound_specialization = Uuid::from_u128(42);
        let mut parent = ReflectedClass::new(parent_id).as_container();
        parent.insert_field(
            7,
            ReflectedField::new(rebound_specialization).with_generic(generic(
                rebound_specialization,
                rebound_specialization,
                serialized_specialization,
                None,
            )),
        );
        let mut context = ObjectStreamReadContext::default();
        let parent = context.insert_class(parent_id, parent).unwrap();
        context
            .insert_class(types::ASSET, ReflectedClass::new(types::ASSET))
            .unwrap();
        context
            .insert_class(
                serialized_specialization,
                ReflectedClass::new(serialized_specialization),
            )
            .unwrap();
        context
            .insert_class(
                rebound_specialization,
                ReflectedClass::new(rebound_specialization),
            )
            .unwrap();

        let old = context.resolve_type(ResolveTypeInput {
            stream_version: 2,
            raw_type_id: types::ASSET,
            specialization_type_id: Some(serialized_specialization),
            element_version: 2,
            name_crc: Some(7),
            parent: Some(parent),
        });
        let current = context.resolve_type(ResolveTypeInput {
            element_version: 3,
            ..ResolveTypeInput {
                stream_version: 2,
                raw_type_id: types::ASSET,
                specialization_type_id: Some(serialized_specialization),
                element_version: 0,
                name_crc: Some(7),
                parent: Some(parent),
            }
        });

        assert_eq!(old.type_id, Some(serialized_specialization));
        assert_eq!(current.type_id, Some(rebound_specialization));
    }

    #[test]
    fn older_asset_version_is_owned_by_registered_serializer_load() {
        let mut context = ObjectStreamReadContext::default();
        let class = context
            .insert_class(
                types::ASSET,
                ReflectedClass::new(types::ASSET).with_version(1),
            )
            .unwrap();
        context
            .insert_builtin_codec(class, types::ASSET, 1)
            .unwrap();

        context.validate_class_version(class, 0).unwrap();
    }

    #[test]
    fn captured_converter_pointer_without_implementation_fails_closed() {
        let id = Uuid::from_u128(51);
        let mut context = ObjectStreamReadContext::default();
        let class = context
            .insert_class(
                id,
                ReflectedClass::new(id)
                    .with_version(2)
                    .with_captured_version_converter(),
            )
            .unwrap();

        assert!(matches!(
            context.validate_class_version(class, 1),
            Err(crate::ObjectStreamError::UnsupportedVersionConversion {
                element_version: 1,
                current_version: 2,
                ..
            })
        ));
    }

    #[test]
    fn captured_data_converter_without_implementation_fails_closed() {
        let parent_id = Uuid::from_u128(52);
        let target_id = Uuid::from_u128(53);
        let source_id = Uuid::from_u128(54);
        let mut parent = ReflectedClass::new(parent_id);
        parent.insert_field(7, ReflectedField::new(target_id));
        let mut context = ObjectStreamReadContext::default();
        let parent = context.insert_class(parent_id, parent).unwrap();
        context
            .insert_class(
                target_id,
                ReflectedClass::new(target_id).with_captured_data_converter(),
            )
            .unwrap();
        let source = context
            .insert_class(source_id, ReflectedClass::new(source_id))
            .unwrap();

        assert!(matches!(
            context.validate_reachable_child(
                parent,
                Some(7),
                source_id,
                ResolvedType {
                    type_id: Some(source_id),
                    class: Some(source),
                    builtin_serializer: None,
                    is_container: false,
                    container_shape: None,
                    ambiguous_generic: false,
                },
            ),
            Err(crate::ObjectStreamError::UnsupportedDataConversion {
                target_type_id,
                source_type_id,
            }) if target_type_id == Uuid::from_u128(53)
                && source_type_id == Uuid::from_u128(54)
        ));
    }

    #[test]
    fn executable_data_converter_replaces_and_reresolves_dom_identity() {
        let parent_id = Uuid::from_u128(57);
        let target_id = Uuid::from_u128(58);
        let source_id = Uuid::from_u128(59);
        let mut parent = ReflectedClass::new(parent_id);
        parent.insert_field(7, ReflectedField::new(target_id));
        let mut context = ObjectStreamReadContext::default();
        let parent = context.insert_class(parent_id, parent).unwrap();
        let target = context
            .insert_class(
                target_id,
                ReflectedClass::new(target_id).with_captured_data_converter(),
            )
            .unwrap();
        context
            .insert_class(source_id, ReflectedClass::new(source_id))
            .unwrap();
        context
            .insert_data_converter(target, Arc::new(ConvertType(target_id)))
            .unwrap();
        let mut child = crate::Element::new(source_id);
        child.name_crc = Some(7);

        context
            .finalize_reachable_child(parent, &mut child, 3)
            .unwrap();

        assert_eq!(child.resolved_type_id(), Some(&target_id));
        assert_eq!(child.resolved_class(), Some(target));
    }

    /// `AZ::Crc32` folds ASCII case, so one hash covers every spelling of a
    /// field name and the global CRC → name table can hold only one of them.
    /// Shipping reflection registers `TimelineInstanceDebugDescriptor::
    /// DebugName` and `FEDebugName::debugName` under the same
    /// `crc32("debugname")`; each child must bind to its own class' spelling,
    /// not to whichever the table happens to carry.
    #[test]
    fn child_field_name_comes_from_the_owning_class_registration() {
        let debug_name_crc = crate::field_name_crc("debugName");
        assert_eq!(debug_name_crc, crate::field_name_crc("DebugName"));

        let string_id = Uuid::from_u128(0x9001);
        let upper_parent_id = Uuid::from_u128(0x9002);
        let lower_parent_id = Uuid::from_u128(0x9003);

        let mut upper_parent = ReflectedClass::new(upper_parent_id);
        upper_parent.insert_named_field("DebugName", ReflectedField::new(string_id));
        let mut lower_parent = ReflectedClass::new(lower_parent_id);
        lower_parent.insert_named_field("debugName", ReflectedField::new(string_id));

        // The table resolves the hash to exactly one spelling, as the harvested
        // capture does.
        let names = LumberyardHashes::new()
            .with_crcs(HashMap::from([(debug_name_crc, ArcStr::from("DebugName"))]));
        let mut context = ObjectStreamReadContext::new(names, ObjectStreamDialect::default());
        context
            .insert_class(string_id, ReflectedClass::new(string_id))
            .unwrap();
        let upper = context.insert_class(upper_parent_id, upper_parent).unwrap();
        let lower = context.insert_class(lower_parent_id, lower_parent).unwrap();

        let resolved_field_name = |parent| {
            let mut child = crate::Element::new(string_id).with_field("DebugName");
            child.name_crc = Some(debug_name_crc);
            context
                .finalize_reachable_child(parent, &mut child, 3)
                .unwrap();
            child.field().map(ToString::to_string)
        };

        assert_eq!(
            resolved_field_name(upper).as_deref(),
            Some("DebugName"),
            "the class that registered `DebugName` keeps it"
        );
        assert_eq!(
            resolved_field_name(lower).as_deref(),
            Some("debugName"),
            "the class that registered `debugName` must not inherit the table's spelling"
        );
    }

    #[test]
    fn raw_wire_id_cannot_authorize_incompatible_post_specialization_type() {
        let parent_id = Uuid::from_u128(65);
        let expected_id = Uuid::from_u128(66);
        let incompatible_id = Uuid::from_u128(67);
        let mut parent = ReflectedClass::new(parent_id);
        parent.insert_field(7, ReflectedField::new(expected_id));
        let mut context = ObjectStreamReadContext::default();
        let parent = context.insert_class(parent_id, parent).unwrap();
        let incompatible = context
            .insert_class(incompatible_id, ReflectedClass::new(incompatible_id))
            .unwrap();

        assert!(matches!(
            context.validate_reachable_child(
                parent,
                Some(7),
                expected_id,
                ResolvedType {
                    type_id: Some(incompatible_id),
                    class: Some(incompatible),
                    builtin_serializer: None,
                    is_container: false,
                    container_shape: None,
                    ambiguous_generic: false,
                },
            ),
            Err(crate::ObjectStreamError::UnexpectedReachableChild { .. })
        ));
    }

    #[test]
    fn captured_unimplemented_serializer_is_distinct_from_structural_class() {
        let serializer_id = Uuid::from_u128(68);
        let structural_id = Uuid::from_u128(69);
        let mut context = ObjectStreamReadContext::default();
        let serializer = context
            .insert_class(
                serializer_id,
                ReflectedClass::new(serializer_id).with_captured_serializer(),
            )
            .unwrap();
        let structural = context
            .insert_class(structural_id, ReflectedClass::new(structural_id))
            .unwrap();

        assert!(matches!(
            context.validate_payload_support(serializer, true),
            Err(crate::ObjectStreamError::UnsupportedSerializer { type_id })
                if type_id == serializer_id
        ));
        assert!(matches!(
            context.validate_payload_support(serializer, false),
            Err(crate::ObjectStreamError::UnsupportedSerializer { type_id })
                if type_id == serializer_id
        ));
        assert!(matches!(
            context.validate_payload_support(structural, true),
            Err(crate::ObjectStreamError::MissingSerializer { type_id })
                if type_id == structural_id
        ));
        context.validate_payload_support(structural, false).unwrap();
    }

    #[test]
    fn dynamic_field_preserves_resolved_dom_without_native_allocation() {
        let parent_id = Uuid::from_u128(55);
        let child_id = Uuid::from_u128(56);
        let mut parent = ReflectedClass::new(parent_id);
        parent.insert_field(7, ReflectedField::new(child_id).as_dynamic_field());
        let mut context = ObjectStreamReadContext::default();
        let parent = context.insert_class(parent_id, parent).unwrap();
        let child = context
            .insert_class(child_id, ReflectedClass::new(child_id))
            .unwrap();

        context
            .validate_reachable_child(
                parent,
                Some(7),
                child_id,
                ResolvedType {
                    type_id: Some(child_id),
                    class: Some(child),
                    builtin_serializer: None,
                    is_container: false,
                    container_shape: None,
                    ambiguous_generic: false,
                },
            )
            .unwrap();
    }

    #[test]
    fn native_dynamic_serializable_field_m_data_accepts_any_resolved_class() {
        let child_id = Uuid::from_u128(57);
        let mut context = ObjectStreamReadContext::default();
        let dynamic = context
            .insert_class(
                types::DYNAMIC_SERIALIZABLE_FIELD,
                ReflectedClass::new(types::DYNAMIC_SERIALIZABLE_FIELD),
            )
            .unwrap();
        let child_class = context
            .insert_class(child_id, ReflectedClass::new(child_id))
            .unwrap();
        let crc = crate::field_name_crc("m_data");
        let resolved = context.resolve_type(ResolveTypeInput {
            parent: Some(dynamic),
            name_crc: Some(crc),
            ..input(child_id)
        });
        assert_eq!(resolved.class, Some(child_class));
        context
            .validate_reachable_child(dynamic, Some(crc), child_id, resolved)
            .unwrap();

        let mut child = crate::Element::new(child_id);
        child.name_crc = Some(crc);
        context
            .finalize_reachable_child(dynamic, &mut child, 3)
            .unwrap();
        assert_eq!(child.resolved_type_id(), Some(&child_id));
    }

    #[test]
    fn composite_class_cannot_be_misregistered_as_builtin_serializer() {
        let mut context = ObjectStreamReadContext::default();
        let class = context
            .insert_class(types::ENTITY_ID, ReflectedClass::new(types::ENTITY_ID))
            .unwrap();

        assert_eq!(
            context.insert_builtin_codec(class, types::ENTITY_ID, 0),
            Err(SerializerRegistrationError::UnsupportedSerializer {
                type_id: types::ENTITY_ID,
            })
        );
    }

    #[test]
    fn strict_default_structural_conversion_rejects_unmapped_child() {
        let parent_id = Uuid::from_u128(61);
        let child_id = Uuid::from_u128(62);
        let mut parent = ReflectedClass::new(parent_id).with_version(1);
        parent.insert_field(7, ReflectedField::new(child_id));
        let mut context = ObjectStreamReadContext::default();
        let parent_class = context.insert_class(parent_id, parent).unwrap();
        let child_class = context
            .insert_class(child_id, ReflectedClass::new(child_id))
            .unwrap();

        assert_eq!(
            context.validate_class_version(parent_class, 0).unwrap(),
            VersionConversionState::StrictDefaultStructural { from: 0, to: 1 }
        );
        assert!(matches!(
            context.validate_reachable_child(
                parent_class,
                Some(8),
                child_id,
                ResolvedType {
                    type_id: Some(child_id),
                    class: Some(child_class),
                    builtin_serializer: None,
                    is_container: false,
                    container_shape: None,
                    ambiguous_generic: false,
                },
            ),
            Err(crate::ObjectStreamError::UnexpectedReachableChild {
                name_crc: Some(8),
                ..
            })
        ));
    }

    #[test]
    fn strict_default_structural_conversion_advances_after_lossless_validation() {
        let parent_id = Uuid::from_u128(63);
        let child_id = Uuid::from_u128(64);
        let mut parent = ReflectedClass::new(parent_id).with_version(2);
        parent.insert_field(7, ReflectedField::new(child_id));
        let mut context = ObjectStreamReadContext::default();
        let parent_class = context.insert_class(parent_id, parent).unwrap();
        context
            .insert_class(child_id, ReflectedClass::new(child_id))
            .unwrap();
        let mut element = crate::Element::new(parent_id);
        element.version = Some(1);
        let mut child = crate::Element::new(child_id);
        child.name_crc = Some(7);
        element.elements.push(child);
        let state = context.validate_class_version(parent_class, 1).unwrap();

        context
            .convert_element(parent_class, state, &mut element, 3, None)
            .unwrap();

        assert_eq!(element.version, Some(2));
        assert_eq!(element.version_state, VersionConversionState::Current);
        assert_eq!(element.elements[0].resolved_type_id(), Some(&child_id));
    }

    #[test]
    fn pointer_field_accepts_registered_rtti_derived_type_but_value_field_rejects_it() {
        let pointer_parent_id = Uuid::from_u128(71);
        let value_parent_id = Uuid::from_u128(72);
        let base_id = Uuid::from_u128(73);
        let derived_id = Uuid::from_u128(74);
        let mut pointer_parent = ReflectedClass::new(pointer_parent_id);
        pointer_parent.insert_field(1, ReflectedField::new(base_id).as_pointer());
        let mut value_parent = ReflectedClass::new(value_parent_id);
        value_parent.insert_field(1, ReflectedField::new(base_id));
        let mut context = ObjectStreamReadContext::default();
        let pointer_parent = context
            .insert_class(pointer_parent_id, pointer_parent)
            .unwrap();
        let value_parent = context.insert_class(value_parent_id, value_parent).unwrap();
        let derived = context
            .insert_class(derived_id, ReflectedClass::new(derived_id))
            .unwrap();
        context.insert_rtti_base(derived_id, base_id);
        let child = ResolvedType {
            type_id: Some(derived_id),
            class: Some(derived),
            builtin_serializer: None,
            is_container: false,
            container_shape: None,
            ambiguous_generic: false,
        };

        context
            .validate_reachable_child(pointer_parent, Some(1), derived_id, child)
            .unwrap();
        assert!(matches!(
            context.validate_reachable_child(value_parent, Some(1), derived_id, child),
            Err(crate::ObjectStreamError::UnexpectedReachableChild { .. })
        ));
    }

    #[test]
    fn exact_base_enum_underlying_and_generic_candidates_are_reachable() {
        let parent_id = Uuid::from_u128(81);
        let base_id = Uuid::from_u128(82);
        let enum_id = Uuid::from_u128(83);
        let underlying_id = Uuid::from_u128(84);
        let generic_class_id = Uuid::from_u128(85);
        let generic_alias = Uuid::from_u128(86);
        let generic_field_id = Uuid::from_u128(87);
        let mut parent = ReflectedClass::new(parent_id);
        parent.insert_field(1, ReflectedField::new(base_id).as_base_class());
        parent.insert_field(2, ReflectedField::new(enum_id));
        parent.insert_field(
            3,
            ReflectedField::new(generic_field_id).with_generic(generic(
                generic_class_id,
                generic_class_id,
                generic_alias,
                None,
            )),
        );
        let mut context = ObjectStreamReadContext::default();
        let parent = context.insert_class(parent_id, parent).unwrap();
        let base = context
            .insert_class(base_id, ReflectedClass::new(base_id))
            .unwrap();
        let underlying = context
            .insert_class(underlying_id, ReflectedClass::new(underlying_id))
            .unwrap();
        let generic_class = context
            .insert_class(generic_class_id, ReflectedClass::new(generic_class_id))
            .unwrap();
        context
            .insert_enum_underlying_type(enum_id, underlying_id)
            .unwrap();

        for (crc, raw, class) in [
            (1, base_id, base),
            (2, underlying_id, underlying),
            (3, generic_alias, generic_class),
        ] {
            context
                .validate_reachable_child(
                    parent,
                    Some(crc),
                    raw,
                    ResolvedType {
                        type_id: Some(raw),
                        class: Some(class),
                        builtin_serializer: None,
                        is_container: false,
                        container_shape: None,
                        ambiguous_generic: false,
                    },
                )
                .unwrap();
        }
    }

    #[test]
    fn non_pointer_base_field_rejects_derived_serialized_type() {
        let parent_id = Uuid::from_u128(88);
        let base_id = Uuid::from_u128(89);
        let derived_id = Uuid::from_u128(90);
        let mut parent = ReflectedClass::new(parent_id);
        parent.insert_field(1, ReflectedField::new(base_id).as_base_class());
        let mut context = ObjectStreamReadContext::default();
        let parent = context.insert_class(parent_id, parent).unwrap();
        let base = context
            .insert_class(base_id, ReflectedClass::new(base_id))
            .unwrap();
        let derived = context
            .insert_class(derived_id, ReflectedClass::new(derived_id))
            .unwrap();
        context.insert_rtti_base(derived_id, base_id);

        context
            .validate_reachable_child(
                parent,
                Some(1),
                base_id,
                ResolvedType {
                    type_id: Some(base_id),
                    class: Some(base),
                    builtin_serializer: None,
                    is_container: false,
                    container_shape: None,
                    ambiguous_generic: false,
                },
            )
            .unwrap();
        assert!(matches!(
            context.validate_reachable_child(
                parent,
                Some(1),
                derived_id,
                ResolvedType {
                    type_id: Some(derived_id),
                    class: Some(derived),
                    builtin_serializer: None,
                    is_container: false,
                    container_shape: None,
                    ambiguous_generic: false,
                },
            ),
            Err(crate::ObjectStreamError::UnexpectedReachableChild { .. })
        ));
    }

    #[test]
    fn executable_converter_re_resolves_replacement_class_identity() {
        let old_id = Uuid::from_u128(91);
        let replacement_id = Uuid::from_u128(92);
        let mut context = ObjectStreamReadContext::default();
        let old_class = context
            .insert_class(
                old_id,
                ReflectedClass::new(old_id)
                    .with_version(1)
                    .with_captured_version_converter(),
            )
            .unwrap();
        let replacement_class = context
            .insert_class(
                replacement_id,
                ReflectedClass::new(replacement_id).with_version(2),
            )
            .unwrap();
        context
            .insert_version_converter(old_class, Arc::new(ReplaceType(replacement_id)))
            .unwrap();
        let mut element = crate::Element::new(old_id);
        element.version = Some(0);
        let state = context.validate_class_version(old_class, 0).unwrap();

        let resolved = context
            .convert_element(old_class, state, &mut element, 3, None)
            .unwrap();

        assert_eq!(resolved, replacement_class);
        assert_eq!(element.resolved_type_id(), Some(&replacement_id));
        assert_eq!(element.resolved_class(), Some(replacement_class));
        assert_eq!(element.version, Some(2));
        assert_eq!(element.version_state, VersionConversionState::Current);
    }

    #[test]
    fn ambiguous_legacy_multimap_fails_root_but_parent_generic_disambiguates() {
        let legacy = Uuid::from_u128(0xa0);
        let registered_a = Uuid::from_u128(0xa1);
        let registered_b = Uuid::from_u128(0xa2);
        let specialized_a = Uuid::from_u128(0xa3);
        let specialized_b = Uuid::from_u128(0xa4);
        let parent_id = Uuid::from_u128(0xa5);
        let mut context = ObjectStreamReadContext::default();
        let class_a = context.insert_class_data(ReflectedClass::new(specialized_a));
        let class_b = context.insert_class_data(ReflectedClass::new(specialized_b));
        let info_a =
            GenericClassInfo::new(class_a, specialized_a, Some(registered_a), Some(legacy));
        let info_b =
            GenericClassInfo::new(class_b, specialized_b, Some(registered_b), Some(legacy));
        context.insert_generic(registered_a, info_a.clone());
        context.insert_generic(registered_b, info_b);
        context
            .insert_legacy_generic_redirect(legacy, registered_a)
            .unwrap();
        context
            .insert_legacy_generic_redirect(legacy, registered_b)
            .unwrap();
        context = context.with_unregistered_class_policy(UnregisteredClassPolicy::NativeSkip);

        let root = context.resolve_type(input(legacy));
        assert!(root.ambiguous_generic);
        assert!(matches!(
            context.resolved_element_fate(root, legacy),
            Err(crate::ObjectStreamError::AmbiguousGenericType { type_id }) if type_id == legacy
        ));

        let mut parent = ReflectedClass::new(parent_id).as_container();
        parent.insert_field(7, ReflectedField::new(specialized_a).with_generic(info_a));
        let parent = context.insert_class(parent_id, parent).unwrap();
        let nested = context.resolve_type(ResolveTypeInput {
            parent: Some(parent),
            name_crc: Some(7),
            ..input(legacy)
        });
        assert!(!nested.ambiguous_generic);
        assert_eq!(nested.class, Some(class_a));
    }

    #[derive(Debug)]
    struct RecordingOverlayProvider {
        seen: std::sync::Mutex<Option<crate::overlay::DataOverlayRequest>>,
        replacement_type: Uuid,
    }

    impl crate::overlay::ObjectStreamDataOverlayProvider for RecordingOverlayProvider {
        fn fill_overlay_data(
            &self,
            request: &crate::overlay::DataOverlayRequest,
        ) -> Result<crate::Element, crate::ObjectStreamError> {
            *self.seen.lock().expect("overlay request lock") = Some(request.clone());
            Ok(crate::Element::new(self.replacement_type).with_data(99_u32.to_be_bytes()))
        }
    }

    fn reflected_field(mut element: crate::Element, name: &str) -> crate::Element {
        element.field = Some(name.into());
        element.name_crc = Some(crate::field_name_crc(name));
        element
    }

    #[test]
    fn data_overlay_provider_replaces_placeholder_and_restores_field_identity() {
        const PROVIDER_ID: u32 = 0x1234_5678;
        let token_id = Uuid::from_u128(0xb1);
        let uri_vector_id = Uuid::from_u128(0xb2);
        let mut context = ObjectStreamReadContext::default();

        let u32_class = context
            .insert_class(
                types::UNSIGNED_INT,
                ReflectedClass::new(types::UNSIGNED_INT).with_captured_serializer(),
            )
            .unwrap();
        context
            .insert_builtin_codec(u32_class, types::UNSIGNED_INT, 0)
            .unwrap();
        let u8_class = context
            .insert_class(
                types::UNSIGNED_CHAR,
                ReflectedClass::new(types::UNSIGNED_CHAR).with_captured_serializer(),
            )
            .unwrap();
        context
            .insert_builtin_codec(u8_class, types::UNSIGNED_CHAR, 0)
            .unwrap();

        let mut uri_vector = ReflectedClass::new(uri_vector_id)
            .with_container_shape(ContainerShape::Sequence)
            .with_container_cardinality(ContainerCardinality::Variable);
        uri_vector.insert_field(
            crate::field_name_crc("element"),
            ReflectedField::new(types::UNSIGNED_CHAR),
        );
        context.insert_class(uri_vector_id, uri_vector).unwrap();

        let mut token = ReflectedClass::new(token_id);
        token.insert_field(
            crate::field_name_crc("Uri"),
            ReflectedField::new(uri_vector_id),
        );
        context.insert_class(token_id, token).unwrap();

        let mut overlay_info = ReflectedClass::new(types::DATA_OVERLAY_INFO);
        overlay_info.insert_field(
            crate::field_name_crc("ProviderId"),
            ReflectedField::new(types::UNSIGNED_INT),
        );
        overlay_info.insert_field(
            crate::field_name_crc("DataToken"),
            ReflectedField::new(token_id),
        );
        context
            .insert_class(types::DATA_OVERLAY_INFO, overlay_info)
            .unwrap();

        let provider = Arc::new(RecordingOverlayProvider {
            seen: std::sync::Mutex::new(None),
            replacement_type: types::UNSIGNED_INT,
        });
        context.insert_data_overlay_provider(PROVIDER_ID, provider.clone());

        let uri = reflected_field(
            crate::Element::new(uri_vector_id).with_children(vec![
                reflected_field(
                    crate::Element::new(types::UNSIGNED_CHAR).with_data([0xaa]),
                    "element",
                ),
                reflected_field(
                    crate::Element::new(types::UNSIGNED_CHAR).with_data([0xbb]),
                    "element",
                ),
            ]),
            "Uri",
        );
        let token = reflected_field(
            crate::Element::new(token_id).with_children(vec![uri]),
            "DataToken",
        );
        let provider_id = reflected_field(
            crate::Element::new(types::UNSIGNED_INT).with_data(PROVIDER_ID.to_be_bytes()),
            "ProviderId",
        );
        let mut overlay = reflected_field(
            crate::Element::new(types::DATA_OVERLAY_INFO).with_children(vec![provider_id, token]),
            "Payload",
        );

        context.resolve_element_tree(3, &mut overlay).unwrap();

        assert_eq!(overlay.resolved_type_id(), Some(&types::UNSIGNED_INT));
        assert_eq!(overlay.field().map(arcstr::ArcStr::as_str), Some("Payload"));
        assert_eq!(overlay.name_crc(), Some(crate::field_name_crc("Payload")));
        assert_eq!(overlay.data(), Some(99_u32.to_be_bytes().as_slice()));
        assert_eq!(
            provider.seen.lock().unwrap().as_ref(),
            Some(&crate::overlay::DataOverlayRequest {
                provider_id: PROVIDER_ID,
                token: crate::overlay::DataOverlayToken {
                    data_uri: vec![0xaa, 0xbb],
                },
            })
        );
    }
}
