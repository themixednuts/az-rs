//! Strict lowering of captured AZ `SerializeContext` JSON into an `ObjectStream`
//! read context.
//!
//! Capture object identity (`$id`/`$ref`) is native reflection identity.
//! Class UUIDs are lookup data and may be shared by distinct `ClassData`
//! records, so this module performs an identity-preserving reserve/define pass
//! instead of deduplicating records by `m_typeId`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error as StdError;
use std::fmt::Display;
use std::sync::Arc;

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

use az_core::uuid as az_uuid;

use crate::codec::{BuiltinSerializerDescriptor, builtin_serializer_kind};
use crate::container::{
    GENERIC_ARRAY_TYPE_ID, GENERIC_FIXED_VECTOR_TYPE_ID, GENERIC_FORWARD_LIST_TYPE_ID,
    GENERIC_INTRUSIVE_PTR_TYPE_ID, GENERIC_LIST_TYPE_ID, GENERIC_MAP_TYPE_ID,
    GENERIC_OPTIONAL_TYPE_ID, GENERIC_PAIR_TYPE_ID, GENERIC_SET_TYPE_ID,
    GENERIC_SHARED_PTR_TYPE_ID, GENERIC_TUPLE_TYPE_ID, GENERIC_UNIQUE_PTR_TYPE_ID,
    GENERIC_UNORDERED_MAP_TYPE_ID, GENERIC_UNORDERED_SET_TYPE_ID, GENERIC_VECTOR_TYPE_ID,
    container_cardinality_for_class_data_uuid, container_cardinality_for_generic_uuid,
    container_shape_for_class_data_uuid, container_shape_for_generic_uuid,
};
use crate::context::{
    ContainerShape, GenericClassInfo, ObjectStreamDialect, ObjectStreamReadContext,
    ObjectStreamVersionConverter, ReflectedClass, ReflectedClassKey, ReflectedField,
    UnmatchedChildPolicy, UnregisteredClassPolicy,
};
use crate::lookup::LumberyardHashes;

const MAX_REF_DEPTH: usize = 32;

pub type Result<T> = std::result::Result<T, CaptureError>;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("{message}")]
    Invalid { message: String },
    #[error("{context}: {source}")]
    Context {
        context: String,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
}

impl CaptureError {
    fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            message: message.into(),
        }
    }
}

trait CaptureResultExt<T> {
    fn context(self, context: impl Display) -> Result<T>;

    fn with_context(self, context: impl FnOnce() -> String) -> Result<T>;
}

impl<T, E> CaptureResultExt<T> for std::result::Result<T, E>
where
    E: StdError + Send + Sync + 'static,
{
    fn context(self, context: impl Display) -> Result<T> {
        self.map_err(|source| CaptureError::Context {
            context: context.to_string(),
            source: Box::new(source),
        })
    }

    fn with_context(self, context: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|source| CaptureError::Context {
            context: context(),
            source: Box::new(source),
        })
    }
}

macro_rules! capture_error {
    ($($argument:tt)*) => {
        CaptureError::invalid(format!($($argument)*))
    };
}

macro_rules! bail {
    ($($argument:tt)*) => {
        return Err(capture_error!($($argument)*))
    };
}

macro_rules! ensure {
    ($condition:expr, $($argument:tt)*) => {
        if !$condition {
            bail!($($argument)*);
        }
    };
}

/// Owned, validated AZ `SerializeContext` capture.
///
/// Record vectors are sorted by capture identity, making each vector index a
/// stable identity for selection manifests without conflating it with a UUID.
#[derive(Debug, Clone, Serialize)]
pub struct CapturedSerializeContext {
    root: Value,
    class_record_ids: Vec<String>,
    generic_record_ids: Vec<String>,
}

impl CapturedSerializeContext {
    /// Parse a captured AZ `SerializeContext` JSON document.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Context`] wrapping the `serde_json` failure if
    /// `bytes` is not valid JSON, then any error [`Self::from_value`] returns.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let root: Value =
            serde_json::from_slice(bytes).context("parse captured AZ SerializeContext JSON")?;
        Self::from_value(root)
    }

    /// Validate an already-parsed capture document.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Invalid`] when the document's `$id`/`$ref` graph is
    /// malformed — a duplicate `$id`, a `$ref` with no target, or a record that is
    /// neither a `ClassData` nor a `GenericClassInfo` where one is required.
    pub fn from_value(root: Value) -> Result<Self> {
        let refs = References::new(&root)?;
        let class_record_ids = refs.class_data_ids();
        let generic_record_ids = refs.generic_info_ids();
        // Deliberately no "is this capture empty" check anywhere in this
        // type or in `ObjectStreamReadContext` — not here, and not moved to
        // `validate_complete()` either (that placement was tried and
        // reverted; see history). An empty capture, and an empty *context*
        // more generally, is not an invalid state: `ObjectStreamReadContext`
        // is used bare (`::default()`, zero classes) throughout this crate's
        // own test suite as a legitimate baseline —
        // `context::tests::generic_metadata_cannot_resolve_an_uncaptured_class`,
        // `deserialize::tests::context_aware_load_rejects_unresolved_v2_before_typed_decode`,
        // and `serialize::tests::context_writer_rejects_structural_data_overlay_before_touching_output`
        // all construct one directly and expect specific, narrower errors,
        // not a blanket "context has nothing registered" failure. And F1's
        // own seed-hook test
        // (`seeded_classes_register_absent_reflection_and_bind_a_seeded_field`)
        // deliberately builds a *zero-seed* context from this exact
        // zero-class capture shape and expects that `build_context` call to
        // **succeed**, asserting `None` only at the specific-type-resolution
        // point afterward. Four independent call sites agree: emptiness is
        // not the invariant. The real invariant — a needed type must resolve
        // at the point it's actually used — is already enforced per-element
        // by `UnresolvedElementType`, `MissingSerializer`, and friends; see
        // `empty_capture_and_no_seeds_fails_loudly_at_element_resolution`
        // below for a from-scratch demonstration that a genuinely broken or
        // deliberately-unseeded capture still fails loudly there rather than
        // silently producing wrong output.
        Ok(Self {
            root,
            class_record_ids,
            generic_record_ids,
        })
    }

    #[must_use]
    pub fn class_record_ids(&self) -> &[String] {
        &self.class_record_ids
    }

    /// Human-readable `name typeId` summaries for every identified `ClassData`
    /// record, sorted by name. Intended for diagnostics when a pinned
    /// selection manifest count drifts and the operator must see which
    /// records the closure actually retained.
    #[must_use]
    pub fn class_record_summaries(&self) -> Vec<String> {
        fn walk(value: &Value, out: &mut Vec<String>) {
            match value {
                Value::Object(map) => {
                    if map.contains_key("$id") && is_class_data(map) {
                        let name = map.get("name").and_then(Value::as_str).unwrap_or("?");
                        let type_id = map.get("typeId").and_then(Value::as_str).unwrap_or("?");
                        out.push(format!("{name} {type_id}"));
                    }
                    for child in map.values() {
                        walk(child, out);
                    }
                }
                Value::Array(values) => {
                    for child in values {
                        walk(child, out);
                    }
                }
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out.sort();
        out
    }

    #[must_use]
    pub fn generic_record_ids(&self) -> &[String] {
        &self.generic_record_ids
    }

    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.root
    }

    /// Serialize this owned capture in the same JSON shape accepted by
    /// [`Self::from_bytes`].
    ///
    /// This is intentionally the captured reflection document, not a second
    /// project-specific schema. A root-selected capture can therefore be
    /// embedded as an ordinary Azoth build product and lowered by the same
    /// engine path at runtime.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Context`] wrapping the `serde_json` failure. The
    /// document was parsed from JSON, so this is unreachable short of an
    /// allocation failure in the serializer.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&self.root).context("serialize captured AZ SerializeContext recipe")
    }

    /// Produce the transitive reflection closure required to resolve and
    /// decode the requested semantic root types.
    ///
    /// Closure follows exact `uuidMap`/`uuidGenericMap` registrations,
    /// ClassData/GenericClassInfo object identity, reflected UUID references,
    /// enum-underlying registrations, RTTI/attribute references, and all
    /// `$id`/`$ref` edges. It never uses reflected names or child position to
    /// infer a dependency. The selected document keeps every alias that points
    /// at a retained native record so Lumberyard lookup precedence and generic
    /// multimap ambiguity remain intact.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Invalid`] if `roots` is empty, if a requested root
    /// UUID has no `uuidMap` or `uuidGenericMap` registration, or if the capture is
    /// missing a `uuidMap`, `uuidGenericMap` or
    /// `enumTypeIdToUnderlyingTypeIdMap` member of the expected JSON shape.
    /// Returns [`CaptureError::Context`] naming the record being resolved when a
    /// `$ref` edge reached while walking the closure does not resolve to a
    /// captured record.
    pub fn select_roots(&self, roots: &[Uuid]) -> Result<Self> {
        ensure!(!roots.is_empty(), "ObjectStream root selection is empty");

        let refs = References::new(&self.root)?;
        let uuid_map = object(&self.root, "uuidMap", "SerializeContext")?;
        let generic_map = array(&self.root, "uuidGenericMap", "SerializeContext")?;
        let enum_map = object(
            &self.root,
            "enumTypeIdToUnderlyingTypeIdMap",
            "SerializeContext",
        )?;

        let index = index_root_registrations(&refs, uuid_map, generic_map, enum_map)?;
        let RootRegistrationIndex {
            direct_by_uuid,
            generic_by_uuid,
            enum_underlying,
        } = &index;

        let mut selected_ids = HashSet::<String>::new();
        let mut selected_uuids = HashSet::<Uuid>::new();
        let mut pending_ids = Vec::<String>::new();
        let mut pending_uuids = roots.to_vec();

        while !pending_ids.is_empty() || !pending_uuids.is_empty() {
            while let Some(type_id) = pending_uuids.pop() {
                if !selected_uuids.insert(type_id) {
                    continue;
                }
                if let Some(class_ids) = direct_by_uuid.get(&type_id) {
                    pending_ids.extend(class_ids.iter().cloned());
                }
                if let Some(generic_ids) = generic_by_uuid.get(&type_id) {
                    pending_ids.extend(generic_ids.iter().cloned());
                }
                if let Some(underlying) = enum_underlying.get(&type_id) {
                    pending_uuids.push(*underlying);
                }
            }

            while let Some(identity) = pending_ids.pop() {
                if !selected_ids.insert(identity.clone()) {
                    continue;
                }
                collect_capture_dependencies(
                    refs.by_id(&identity)?,
                    &mut pending_ids,
                    &mut pending_uuids,
                )?;
            }
        }

        for root in roots {
            ensure!(
                direct_by_uuid.contains_key(root) || generic_by_uuid.contains_key(root),
                "selected ObjectStream root {root} is not registered in uuidMap or uuidGenericMap"
            );
        }

        let document = selected_capture_document(
            &refs,
            CaptureMaps {
                uuids: uuid_map,
                generics: generic_map,
                enums: enum_map,
            },
            &selected_ids,
            &selected_uuids,
        )?;
        Self::from_value(document)
    }
}

/// The three captured registration maps of one `SerializeContext` document.
#[derive(Debug, Clone, Copy)]
struct CaptureMaps<'a> {
    uuids: &'a Map<String, Value>,
    generics: &'a [Value],
    enums: &'a Map<String, Value>,
}

/// Captured registrations indexed by the type UUID that reaches them.
struct RootRegistrationIndex {
    direct_by_uuid: HashMap<Uuid, Vec<String>>,
    generic_by_uuid: HashMap<Uuid, Vec<String>>,
    enum_underlying: HashMap<Uuid, Uuid>,
}

/// Index `uuidMap`, `uuidGenericMap` and the enum map by type UUID.
fn index_root_registrations(
    refs: &References<'_>,
    uuid_map: &Map<String, Value>,
    generic_map: &[Value],
    enum_map: &Map<String, Value>,
) -> Result<RootRegistrationIndex> {
    let mut direct_by_uuid = HashMap::<Uuid, Vec<String>>::new();
    for (raw_alias, raw_class) in uuid_map {
        let alias = uuid_text(raw_alias, "uuidMap key")?;
        direct_by_uuid.entry(alias).or_default().push(
            refs.identity(raw_class, "uuidMap ClassData")
                .with_context(|| format!("resolve uuidMap root candidate {alias}"))?,
        );
    }

    let mut generic_by_uuid = HashMap::<Uuid, Vec<String>>::new();
    for (index, entry) in generic_map.iter().enumerate() {
        let scope = format!("uuidGenericMap[{index}]");
        let (raw_alias, raw_generic) = pair(entry, &scope)?;
        let alias = uuid_value(raw_alias, &format!("{scope} key"))?;
        generic_by_uuid
            .entry(alias)
            .or_default()
            .push(refs.identity(raw_generic, &format!("{scope} GenericClassInfo"))?);
    }

    let mut enum_underlying = HashMap::<Uuid, Uuid>::new();
    for (raw_enum, raw_underlying) in enum_map {
        enum_underlying.insert(
            uuid_text(raw_enum, "enumTypeIdToUnderlyingTypeIdMap key")?,
            uuid_value(
                raw_underlying,
                "enumTypeIdToUnderlyingTypeIdMap underlying type",
            )?,
        );
    }

    Ok(RootRegistrationIndex {
        direct_by_uuid,
        generic_by_uuid,
        enum_underlying,
    })
}

/// Emit the sub-document holding exactly the selected closure.
///
/// Every alias that points at a retained record is kept, so Lumberyard lookup
/// precedence and generic multimap ambiguity survive the trim.
fn selected_capture_document(
    refs: &References<'_>,
    maps: CaptureMaps<'_>,
    selected_ids: &HashSet<String>,
    selected_uuids: &HashSet<Uuid>,
) -> Result<Value> {
    let mut selected_uuid_map = Map::new();
    for (raw_alias, raw_class) in maps.uuids {
        let identity = refs.identity(raw_class, "uuidMap ClassData")?;
        if selected_ids.contains(&identity) {
            selected_uuid_map.insert(raw_alias.clone(), captured_reference(&identity));
        }
    }

    let mut selected_generic_map = Vec::new();
    for (index, entry) in maps.generics.iter().enumerate() {
        let scope = format!("uuidGenericMap[{index}]");
        let (raw_alias, raw_generic) = pair(entry, &scope)?;
        let identity = refs.identity(raw_generic, &format!("{scope} GenericClassInfo"))?;
        if selected_ids.contains(&identity) {
            selected_generic_map.push(Value::Array(vec![
                raw_alias.clone(),
                captured_reference(&identity),
            ]));
        }
    }

    let mut selected_enum_map = Map::new();
    for (raw_enum, raw_underlying) in maps.enums {
        let enum_id = uuid_text(raw_enum, "enumTypeIdToUnderlyingTypeIdMap key")?;
        if selected_uuids.contains(&enum_id) {
            selected_enum_map.insert(raw_enum.clone(), raw_underlying.clone());
        }
    }

    let mut objects = selected_ids.iter().cloned().collect::<Vec<_>>();
    objects.sort();
    let objects = objects
        .iter()
        .map(|identity| flatten_captured_definition(refs.by_id(identity)?, identity))
        .collect::<Result<Vec<_>>>()?;

    let mut root = Map::new();
    root.insert("uuidMap".to_owned(), Value::Object(selected_uuid_map));
    root.insert(
        "uuidGenericMap".to_owned(),
        Value::Array(selected_generic_map),
    );
    root.insert(
        "enumTypeIdToUnderlyingTypeIdMap".to_owned(),
        Value::Object(selected_enum_map),
    );
    root.insert("selectedObjects".to_owned(), Value::Array(objects));
    Ok(Value::Object(root))
}

/// An import field-CRC rename applied while lowering a captured context.
///
/// Native Lumberyard renamed a `ClassElement` without a version bump and
/// without a converter: the old CRC binds to no current element, so
/// default-filter load silently drops its data. The importer instead rebinds
/// the old CRC onto the current field so the payload survives under the
/// canonical identity. Modeled at the recipe level (not as a per-asset shim)
/// because it is a property of the reflected type, not of any one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldRename {
    /// Type UUID of the class whose reached child is rebound.
    pub parent_type_id: Uuid,
    /// Captured (old) name CRC as it appears on the wire.
    pub from_name_crc: u32,
    /// Registered (current) name CRC the child must bind to.
    pub to_name_crc: u32,
}

/// Selects the dialect and seed names used to build one read context.
#[derive(Debug, Clone)]
pub struct ObjectStreamContextRecipe {
    dialect: ObjectStreamDialect,
    names: LumberyardHashes,
    generic_argument_aliases: LumberyardHashes,
    serializer_identities: HashMap<String, BuiltinSerializerDescriptor>,
    ranged_unsigned_serializer_families: HashSet<Uuid>,
    byte_stream_serializer_families: HashSet<Uuid>,
    byte_stream_serializer_specialized: HashSet<Uuid>,
    unregistered_class_policy: UnregisteredClassPolicy,
    unmatched_child_policy: UnmatchedChildPolicy,
    field_renames: Vec<FieldRename>,
    version_converters: Vec<(Uuid, Arc<dyn ObjectStreamVersionConverter>)>,
    seeded_classes: Vec<(Uuid, ReflectedClass)>,
    seeded_generic_fields: Vec<(Uuid, u32)>,
    seeded_rtti_bases: Vec<(Uuid, Uuid)>,
}

impl ObjectStreamContextRecipe {
    #[must_use]
    pub fn new(dialect: ObjectStreamDialect) -> Self {
        Self {
            dialect,
            names: LumberyardHashes::new(),
            generic_argument_aliases: LumberyardHashes::new(),
            serializer_identities: HashMap::new(),
            ranged_unsigned_serializer_families: HashSet::new(),
            byte_stream_serializer_families: HashSet::new(),
            byte_stream_serializer_specialized: HashSet::new(),
            unregistered_class_policy: UnregisteredClassPolicy::default(),
            unmatched_child_policy: UnmatchedChildPolicy::default(),
            field_renames: Vec::new(),
            version_converters: Vec::new(),
            seeded_classes: Vec::new(),
            seeded_generic_fields: Vec::new(),
            seeded_rtti_bases: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_names(mut self, names: LumberyardHashes) -> Self {
        self.names = names;
        self
    }

    /// Register canonical display aliases used only while composing captured
    /// generic type names.
    ///
    /// Captured reflection often spells a type using a short C++ name such as
    /// `EntityId`, while its public serialized identity is `AZ::EntityId`.
    /// Keeping these aliases separate preserves the captured class name for
    /// direct lookups and applies the public spelling only inside generated
    /// container names. This does not register another class or type identity.
    #[must_use]
    pub fn with_generic_argument_aliases<'a>(
        mut self,
        aliases: impl IntoIterator<Item = (Uuid, &'a str)>,
    ) -> Self {
        self.generic_argument_aliases.extend_type_names(aliases);
        self
    }

    /// Rebind a captured (old) field CRC onto a current registered field CRC on
    /// one class type. See [`FieldRename`]. The target must be a real
    /// `ClassElement` on the named class or `build_context` fails.
    #[must_use]
    pub fn with_field_rename(mut self, rename: FieldRename) -> Self {
        self.field_renames.push(rename);
        self
    }

    /// Register an executable version converter for a captured class type,
    /// selected by its type UUID. Makes a "deferred converter family" (a
    /// `ClassData` whose native `converter` pointer was captured but has no engine
    /// implementation) loadable: old-version elements of this type run through
    /// `converter` before their descendants are re-resolved. The type must be a
    /// registered class or `build_context` fails.
    #[must_use]
    pub fn with_version_converter(
        mut self,
        type_id: Uuid,
        converter: Arc<dyn ObjectStreamVersionConverter>,
    ) -> Self {
        self.version_converters.push((type_id, converter));
        self
    }

    /// Trust one build-specific captured serializer identity as an exact
    /// executable descriptor.
    #[must_use]
    pub fn with_serializer_identity(
        mut self,
        identity: impl Into<String>,
        descriptor: BuiltinSerializerDescriptor,
    ) -> Self {
        self.serializer_identities
            .insert(identity.into(), descriptor);
        self
    }

    /// Trust one captured generic family as Lumberyard's ranged-unsigned
    /// serializer, deriving its storage width from the first captured
    /// template type. Range parameters remain type identity only.
    #[must_use]
    pub fn with_ranged_unsigned_serializer_family(mut self, generic_type_id: Uuid) -> Self {
        self.ranged_unsigned_serializer_families
            .insert(generic_type_id);
        self
    }

    /// Trust one captured generic family as an opaque byte-stream serializer.
    /// The captured native serializer stores an opaque byte payload; its text
    /// form is hex, matching [`BuiltinSerializerKind::ByteStream`].
    ///
    /// [`BuiltinSerializerKind::ByteStream`]: crate::codec::BuiltinSerializerKind::ByteStream
    #[must_use]
    pub fn with_byte_stream_serializer_family(mut self, generic_type_id: Uuid) -> Self {
        self.byte_stream_serializer_families.insert(generic_type_id);
        self
    }

    /// Trust one captured specialized (non-generic) type as an opaque
    /// byte-stream serializer, keyed on its stable reflected type UUID rather
    /// than on a captured serializer identity or a generic family.
    ///
    /// Use this when the captured native `m_serializer` identity is a per-run
    /// heap pointer (not stable across recaptures) and `GetGenericTypeId()` is
    /// null, so neither [`with_serializer_identity`](Self::with_serializer_identity)
    /// nor [`with_byte_stream_serializer_family`](Self::with_byte_stream_serializer_family)
    /// can match it (e.g. `Amazon::Pervasives::UID`, a fixed 16-byte opaque id).
    /// The reflected type UUID is stable, so it is the durable key. The type
    /// must carry a captured serializer or the descriptor is never installed.
    #[must_use]
    pub fn with_byte_stream_serializer_specialized(mut self, type_id: Uuid) -> Self {
        self.byte_stream_serializer_specialized.insert(type_id);
        self
    }

    /// Seed one reflected class the captured registry omits, keyed by the
    /// `uuidMap` id the loader resolves for it. The class is registered exactly
    /// like a captured `ClassData` (direct `uuidMap` alias included), so its
    /// fields, container shape/cardinality, and version participate in normal
    /// type resolution and child binding.
    ///
    /// Use only for types proven absent from the captured retail registry yet
    /// still reachable on disk (a client build compiled a component out of
    /// reflection but shipping slices still carry it). A seed that duplicates a
    /// captured `uuidMap` id fails `build_context`, so a seed becoming redundant
    /// after a recapture is loud rather than silent. Container seeds must set
    /// both a container shape and cardinality and carry their `element`
    /// `ClassElement`, mirroring a captured `IDataContainer` family.
    #[must_use]
    pub fn with_seeded_class(mut self, map_id: Uuid, class: ReflectedClass) -> Self {
        self.seeded_classes.push((map_id, class));
        self
    }

    /// Bind a seeded class field to the captured `GenericClassInfo` registered
    /// for that field's declared specialized type.
    ///
    /// Native `ClassElement::m_genericClassInfo` is required when a wire value
    /// uses a generic or legacy-specialized UUID instead of the field's current
    /// specialized UUID. Seeded `ClassData` must request that pointer explicitly;
    /// the recipe never infers generic semantics from a UUID or field shape.
    #[must_use]
    pub fn with_seeded_generic_field(mut self, parent_type_id: Uuid, field_name_crc: u32) -> Self {
        self.seeded_generic_fields
            .push((parent_type_id, field_name_crc));
        self
    }

    /// Restore one RTTI inheritance edge omitted with seeded reflection.
    ///
    /// Serialized base-class fields contribute their RTTI edges automatically.
    /// This hook is for a real C++ base that participates in polymorphic
    /// pointer/container validation but has no persisted `ClassElement` (most
    /// commonly an abstract, fieldless interface base). Both UUIDs must resolve
    /// to registered direct `ClassData` after captured and seeded classes lower.
    #[must_use]
    pub fn with_seeded_rtti_base(mut self, derived_type_id: Uuid, base_type_id: Uuid) -> Self {
        self.seeded_rtti_bases.push((derived_type_id, base_type_id));
        self
    }

    /// Select how the built context treats a reached child that binds to no
    /// native `ClassElement`. Defaults to strict ([`UnmatchedChildPolicy::Fail`]).
    #[must_use]
    pub const fn with_unmatched_child_policy(mut self, policy: UnmatchedChildPolicy) -> Self {
        self.unmatched_child_policy = policy;
        self
    }

    /// Select how the built context treats a reached element whose type has no
    /// registered `ClassData`. Defaults to strict
    /// ([`UnregisteredClassPolicy::Fail`]).
    #[must_use]
    pub const fn with_unregistered_class_policy(mut self, policy: UnregisteredClassPolicy) -> Self {
        self.unregistered_class_policy = policy;
        self
    }

    /// Parse a capture document and lower it to a read context.
    ///
    /// # Errors
    ///
    /// Returns any error [`CapturedSerializeContext::from_bytes`] returns while
    /// parsing `bytes`, then any error [`Self::build_context`] returns.
    pub fn lower_bytes(&self, bytes: &[u8]) -> Result<ObjectStreamReadContext> {
        self.build_context(&CapturedSerializeContext::from_bytes(bytes)?)
    }

    /// Validate an already-parsed capture document and lower it to a read context.
    ///
    /// # Errors
    ///
    /// Returns any error [`CapturedSerializeContext::from_value`] returns, then any
    /// error [`Self::build_context`] returns.
    pub fn lower_value(&self, root: &Value) -> Result<ObjectStreamReadContext> {
        self.build_context(&CapturedSerializeContext::from_value(root.clone())?)
    }

    /// Seed the reflected names table from every captured record.
    ///
    /// Also collects the RTTI base edges the capture spells out, which the
    /// `ClassData` lowering pass extends with `FLG_BASE_CLASS` fields.
    fn harvest_capture_names(
        &self,
        refs: &References<'_>,
        class_ids: &[String],
        generics: &GenericRecordIndex<'_>,
        explicit_generic_aliases: &HashMap<String, Vec<Uuid>>,
    ) -> Result<(LumberyardHashes, HashSet<(Uuid, Uuid)>)> {
        let mut names = self.names.clone();
        // `AZStd::any` and `DynamicSerializableField` serialize as
        // `{TypeId, m_data}` (findings §5). Neither field name is guaranteed to
        // appear as a captured ClassElement (`any` carries no elements; native
        // fabricates `m_data`), yet the lowering seeds both edges, so seed their
        // names too for downstream CRC -> name resolution and text output.
        names
            .crcs
            .entry(crate::field_name_crc("m_data"))
            .or_insert_with(|| arcstr::ArcStr::from("m_data"));
        names
            .crcs
            .entry(crate::field_name_crc("TypeId"))
            .or_insert_with(|| arcstr::ArcStr::from("TypeId"));
        let mut rtti_bases = HashSet::new();
        for class_id in class_ids {
            harvest_class_names(refs.by_id(class_id)?, refs, &mut names, &mut rtti_bases)
                .with_context(|| format!("harvest ClassData names {class_id}"))?;
        }
        for (class_id, elements) in &generics.element_evidence {
            harvest_element_names(elements, refs, &mut names, &mut rtti_bases)
                .with_context(|| format!("harvest GenericClassInfo element names {class_id}"))?;
        }
        harvest_generic_type_names(
            &generics.metadata,
            explicit_generic_aliases,
            &self.generic_argument_aliases,
            &mut names,
        );
        Ok((names, rtti_bases))
    }

    /// Bind an executable codec to every `ClassData` whose capture recorded a
    /// native serializer.
    fn register_captured_serializers(
        &self,
        context: &mut ObjectStreamReadContext,
        candidates: Vec<CodecCandidate>,
        serializer_evidence: &HashMap<String, SerializerGenericEvidence>,
    ) -> Result<()> {
        for candidate in candidates {
            let CodecCandidate {
                key,
                class_id,
                type_id,
                current_version,
                captured_serializer,
                serializer_identity,
            } = candidate;
            if !captured_serializer {
                continue;
            }
            if let Some(kind) = builtin_serializer_kind(type_id) {
                context
                    .insert_builtin_codec_descriptor(
                        key,
                        type_id,
                        BuiltinSerializerDescriptor::new(kind, current_version),
                    )
                    .with_context(|| format!("register captured builtin codec for {type_id}"))?;
                continue;
            }
            let descriptor = serializer_identity
                .as_deref()
                .and_then(|identity| self.serializer_identities.get(identity))
                .copied()
                .or_else(|| {
                    let evidence = serializer_evidence.get(&class_id)?;
                    let generic = evidence.generic_type_id?;
                    self.ranged_unsigned_serializer_families
                        .contains(&generic)
                        .then_some(())?;
                    let underlying = *evidence.templated_type_ids.first()?;
                    let kind = crate::codec::BuiltinSerializerKind::ranged_from_underlying(
                        builtin_serializer_kind(underlying)?,
                    )?;
                    Some(BuiltinSerializerDescriptor::new(kind, current_version))
                })
                .or_else(|| {
                    // A captured generic family whose native serializer stores an
                    // opaque byte payload (e.g. BitSet<N> word buffers). Its text
                    // form is hex, matching BuiltinSerializerKind::ByteStream.
                    let evidence = serializer_evidence.get(&class_id)?;
                    let generic = evidence.generic_type_id?;
                    self.byte_stream_serializer_families
                        .contains(&generic)
                        .then_some(())?;
                    Some(BuiltinSerializerDescriptor::new(
                        crate::codec::BuiltinSerializerKind::ByteStream,
                        current_version,
                    ))
                })
                .or_else(|| {
                    // A captured specialized (non-generic) type whose native
                    // serializer stores an opaque byte payload but whose captured
                    // identity is only a per-run heap pointer (e.g.
                    // Amazon::Pervasives::UID, a fixed 16-byte opaque id). Its
                    // GetGenericTypeId() is null, so key on the stable reflected
                    // type UUID; its text form is hex, matching ByteStream.
                    self.byte_stream_serializer_specialized
                        .contains(&type_id)
                        .then(|| {
                            BuiltinSerializerDescriptor::new(
                                crate::codec::BuiltinSerializerKind::ByteStream,
                                current_version,
                            )
                        })
                });
            if let Some(descriptor) = descriptor {
                context
                    .insert_captured_serializer_descriptor(key, descriptor)
                    .with_context(|| {
                        format!("register captured serializer descriptor for {type_id}")
                    })?;
            }
        }
        Ok(())
    }

    /// Register the recipe's seeded classes.
    ///
    /// These are reflected classes the captured retail registry omits but
    /// shipping slices still carry. They go in before completion validation so
    /// seeds that reference one another resolve.
    fn apply_seeded_classes(&self, context: &mut ObjectStreamReadContext) -> Result<()> {
        // Recipe seeded classes: register reflected classes the captured retail
        // registry omits but shipping slices still carry. Inserted before
        // completion validation so seeds that reference one another (a seeded
        // component's seeded container member, its seeded element type) resolve.
        // `insert_class` adds the `uuidMap` direct alias, so a duplicate of a
        // captured type fails loudly instead of silently shadowing it.
        let mut bound_seeded_generic_fields = HashSet::new();
        for (map_id, class) in &self.seeded_classes {
            let mut class = class.clone();
            for &(parent_type_id, field_name_crc) in &self.seeded_generic_fields {
                if parent_type_id != class.type_id {
                    continue;
                }
                ensure!(
                    bound_seeded_generic_fields.insert((parent_type_id, field_name_crc)),
                    "seeded generic field {parent_type_id}::{field_name_crc:#010x} is registered more than once"
                );
                let field = class.fields.get_mut(&field_name_crc).ok_or_else(|| {
                    capture_error!(
                        "seeded generic field CRC {field_name_crc:#010x} is not registered on {parent_type_id}"
                    )
                })?;
                ensure!(
                    field.generic.is_none(),
                    "seeded generic field {parent_type_id}::{field_name_crc:#010x} already carries GenericClassInfo"
                );
                field.generic = Some(
                    context
                        .find_generic_info(field.type_id)
                        .cloned()
                        .ok_or_else(|| {
                            capture_error!(
                                "seeded generic field {parent_type_id}::{field_name_crc:#010x} has no unique captured GenericClassInfo for {}",
                                field.type_id
                            )
                        })?,
                );
            }
            for field in class.fields.values() {
                if field.is_base_class && field.type_id != class.type_id {
                    context.insert_rtti_base(class.type_id, field.type_id);
                }
            }
            context
                .insert_class(*map_id, class)
                .with_context(|| format!("register seeded class {map_id}"))?;
        }
        ensure!(
            bound_seeded_generic_fields.len() == self.seeded_generic_fields.len(),
            "one or more seeded generic fields target an unregistered seeded ClassData"
        );
        Ok(())
    }

    /// Apply the recipe's seeded RTTI edges, import field renames and version
    /// converters against the classes already defined on `context`.
    fn apply_recipe_bindings(&self, context: &mut ObjectStreamReadContext) -> Result<()> {
        let mut registered_seeded_rtti_bases = HashSet::new();
        for &(derived_type_id, base_type_id) in &self.seeded_rtti_bases {
            ensure!(
                derived_type_id != base_type_id,
                "seeded RTTI edge {derived_type_id} cannot name itself as a base"
            );
            ensure!(
                registered_seeded_rtti_bases.insert((derived_type_id, base_type_id)),
                "seeded RTTI edge {derived_type_id} -> {base_type_id} is registered more than once"
            );
            ensure!(
                context.direct_class_key(derived_type_id).is_some(),
                "seeded RTTI derived type {derived_type_id} has no registered direct ClassData"
            );
            ensure!(
                context.direct_class_key(base_type_id).is_some(),
                "seeded RTTI base type {base_type_id} has no registered direct ClassData"
            );
            context.insert_rtti_base(derived_type_id, base_type_id);
        }

        // Recipe import field renames: rebind old field CRCs onto current
        // ClassElements. Validated against the defined classes so a rename that
        // names a missing target fails the build instead of silently doing
        // nothing.
        for rename in &self.field_renames {
            ensure!(
                rename.from_name_crc != rename.to_name_crc,
                "field rename on {} maps CRC {:#010x} onto itself",
                rename.parent_type_id,
                rename.from_name_crc
            );
            ensure!(
                context.class_field_exists(rename.parent_type_id, rename.to_name_crc),
                "field rename target CRC {:#010x} is not a registered ClassElement on {}",
                rename.to_name_crc,
                rename.parent_type_id
            );
            context.insert_field_rename(
                rename.parent_type_id,
                rename.from_name_crc,
                rename.to_name_crc,
            );
        }

        // Recipe version converters: bind an executable converter to the exact
        // ClassData the loader resolves for each captured type UUID.
        for (type_id, converter) in &self.version_converters {
            let key = context.direct_class_key(*type_id).ok_or_else(|| {
                capture_error!("version converter target class {type_id} is not registered")
            })?;
            context
                .insert_version_converter(key, Arc::clone(converter))
                .with_context(|| format!("register version converter for {type_id}"))?;
        }
        Ok(())
    }

    /// Lower a validated capture into an [`ObjectStreamReadContext`].
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Invalid`] when the capture does not describe a
    /// context this crate can execute: a missing or wrongly shaped `uuidMap`,
    /// `uuidGenericMap` or `enumTypeIdToUnderlyingTypeIdMap`; a `ClassData`
    /// or `GenericClassInfo` record missing a field the lowering needs; a reflected
    /// UUID that does not parse; a class element nesting deeper than the supported
    /// limit; a duplicate field that disagrees with the one already lowered; or a
    /// container/serializer descriptor whose native family this crate has no
    /// executable counterpart for. Returns [`CaptureError::Context`] naming the
    /// record under resolution when a `$ref` edge does not resolve to a captured
    /// record of the required kind.
    pub fn build_context(
        &self,
        capture: &CapturedSerializeContext,
    ) -> Result<ObjectStreamReadContext> {
        let root = capture.value();
        let refs = References::new(root)?;
        let uuid_map = object(root, "uuidMap", "SerializeContext")?;
        let generic_map = array(root, "uuidGenericMap", "SerializeContext")?;
        let enum_map = object(root, "enumTypeIdToUnderlyingTypeIdMap", "SerializeContext")?;

        let direct_aliases = lower_direct_aliases(&refs, uuid_map)?;
        let explicit_generic_aliases = lower_generic_aliases(&refs, generic_map)?;

        let class_ids = capture.class_record_ids();
        let generic_ids = capture.generic_record_ids();
        let generics = index_generic_records(&refs, generic_ids)?;

        let (names, mut rtti_bases) =
            self.harvest_capture_names(&refs, class_ids, &generics, &explicit_generic_aliases)?;

        let mut context = ObjectStreamReadContext::new(names, self.dialect)
            .with_unregistered_class_policy(self.unregistered_class_policy)
            .with_unmatched_child_policy(self.unmatched_child_policy);
        let mut class_keys = HashMap::<String, ReflectedClassKey>::with_capacity(class_ids.len());
        for class_id in class_ids {
            class_keys.insert(class_id.clone(), context.reserve_class_data());
        }

        let codec_candidates = lower_class_records(
            &refs,
            class_ids,
            &class_keys,
            &generics,
            &mut rtti_bases,
            &mut context,
        )?;

        for (alias, class_id) in direct_aliases {
            context
                .insert_direct_class_alias(alias, key_for(&class_keys, &class_id, "uuidMap")?)
                .with_context(|| format!("register uuidMap alias {alias} -> {class_id}"))?;
        }

        register_generic_infos(
            &mut context,
            &generics.metadata,
            &explicit_generic_aliases,
            &class_keys,
        )?;
        register_enum_underlying_types(&mut context, enum_map)?;
        for (derived, base) in rtti_bases {
            context.insert_rtti_base(derived, base);
        }

        self.register_captured_serializers(
            &mut context,
            codec_candidates,
            &generics.serializer_evidence,
        )?;
        self.apply_seeded_classes(&mut context)?;
        self.apply_recipe_bindings(&mut context)?;

        context
            .into_complete()
            .context("validate complete captured ObjectStream context")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContainerEvidence {
    generic_type_id: Option<Uuid>,
    non_type_capacity: Option<usize>,
    templated_argument_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SerializerGenericEvidence {
    generic_type_id: Option<Uuid>,
    templated_type_ids: Vec<Uuid>,
}

impl ContainerEvidence {
    fn from_metadata(metadata: &GenericMetadata) -> Self {
        Self {
            generic_type_id: metadata.generic_type_id,
            non_type_capacity: metadata
                .non_type_template_arguments
                .first()
                .and_then(|value| usize::try_from(*value).ok()),
            templated_argument_count: metadata.templated_type_ids.len(),
        }
    }
}

#[derive(Debug, Clone)]
struct GenericMetadata {
    class_data_id: String,
    class_name: Option<String>,
    registered_type_ids: Vec<Uuid>,
    templated_type_ids: Vec<Uuid>,
    specialized_type_id: Uuid,
    generic_type_id: Option<Uuid>,
    legacy_specialized_type_id: Option<Uuid>,
    non_type_template_arguments: Vec<u64>,
}

fn parse_generic_metadata(raw: &Value, refs: &References<'_>) -> Result<GenericMetadata> {
    let map = resolved_object(raw, refs, "GenericClassInfo")?;
    let templated_argument_count = usize_member(map, "templatedArgumentCount", "GenericClassInfo")?;
    let templated_type_ids = array_member(map, "templatedTypeIds", "GenericClassInfo")?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            uuid_value(
                value,
                &format!("GenericClassInfo.templatedTypeIds[{index}]"),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let non_type_template_arguments =
        non_type_arguments(member(map, "nonTypeTemplateArguments", "GenericClassInfo")?)?;
    ensure!(
        templated_argument_count == templated_type_ids.len()
            || templated_argument_count
                == templated_type_ids.len() + non_type_template_arguments.len(),
        "GenericClassInfo templatedArgumentCount is {templated_argument_count}, but capture contains {} type and {} non-type arguments",
        templated_type_ids.len(),
        non_type_template_arguments.len()
    );

    let registered_type_ids = match member(map, "registeredTypeIds", "GenericClassInfo")? {
        Value::Null => Vec::new(),
        Value::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                uuid_value(
                    value,
                    &format!("GenericClassInfo.registeredTypeIds[{index}]"),
                )
            })
            .collect::<Result<Vec<_>>>()?,
        _ => bail!("GenericClassInfo.registeredTypeIds must be null or an array"),
    };
    let class_data_id = refs.identity(
        member(map, "classData", "GenericClassInfo")?,
        "GenericClassInfo.classData",
    )?;
    let class_data = refs.object_by_id(&class_data_id)?;
    let class_name =
        string_member(class_data, "name", "GenericClassInfo.classData")?.map(str::to_owned);
    optional_bool_value(
        map.get("elementsApplyToClassData"),
        "GenericClassInfo.elementsApplyToClassData",
    )?;
    let class_data_element_count = optional_usize_value(
        map.get("classDataElementCount"),
        "GenericClassInfo.classDataElementCount",
    )?;
    if let Some(expected) = class_data_element_count {
        let actual = array_member(class_data, "elements", "GenericClassInfo.classData")?.len();
        ensure!(
            actual == expected,
            "GenericClassInfo.classDataElementCount is {expected}, but ClassData contains {actual} element(s)"
        );
    }

    let mut metadata = GenericMetadata {
        class_data_id,
        class_name,
        registered_type_ids,
        templated_type_ids,
        specialized_type_id: uuid_member(map, "specializedTypeId", "GenericClassInfo")?,
        generic_type_id: optional_uuid(map, "genericTypeId", "GenericClassInfo")?,
        legacy_specialized_type_id: optional_uuid(
            map,
            "legacySpecializedTypeId",
            "GenericClassInfo",
        )?,
        non_type_template_arguments,
    };
    metadata.generic_type_id = canonical_generic_type_id(&metadata)?;
    Ok(metadata)
}

fn canonical_generic_type_id(metadata: &GenericMetadata) -> Result<Option<Uuid>> {
    if metadata.generic_type_id.is_some_and(|id| !id.is_nil()) {
        return Ok(metadata.generic_type_id);
    }

    let mut matches = Vec::new();
    match metadata.templated_type_ids.as_slice() {
        [element] => {
            for (generic, specialized) in [
                (GENERIC_VECTOR_TYPE_ID, az_uuid::azstd_vector(*element)),
                (GENERIC_LIST_TYPE_ID, az_uuid::azstd_list(*element)),
                (
                    GENERIC_FORWARD_LIST_TYPE_ID,
                    az_uuid::azstd_forward_list(*element),
                ),
                (GENERIC_SET_TYPE_ID, az_uuid::azstd_set(*element)),
                (
                    GENERIC_UNORDERED_SET_TYPE_ID,
                    az_uuid::azstd_unordered_set(*element),
                ),
                (
                    GENERIC_SHARED_PTR_TYPE_ID,
                    az_uuid::azstd_shared_ptr(*element),
                ),
                (
                    GENERIC_INTRUSIVE_PTR_TYPE_ID,
                    az_uuid::azstd_intrusive_ptr(*element),
                ),
                (
                    GENERIC_UNIQUE_PTR_TYPE_ID,
                    az_uuid::azstd_unique_ptr(*element),
                ),
                (GENERIC_OPTIONAL_TYPE_ID, az_uuid::azstd_optional(*element)),
            ] {
                if specialized == metadata.specialized_type_id {
                    matches.push(generic);
                }
            }
            if let Some(capacity) = metadata
                .non_type_template_arguments
                .first()
                .and_then(|value| usize::try_from(*value).ok())
            {
                for (generic, specialized) in [
                    (
                        GENERIC_FIXED_VECTOR_TYPE_ID,
                        az_uuid::azstd_fixed_vector(*element, capacity),
                    ),
                    (
                        GENERIC_ARRAY_TYPE_ID,
                        az_uuid::azstd_array(*element, capacity),
                    ),
                ] {
                    if specialized == metadata.specialized_type_id {
                        matches.push(generic);
                    }
                }
            }
        }
        [first, second] => {
            for (generic, specialized) in [
                (GENERIC_PAIR_TYPE_ID, az_uuid::azstd_pair(*first, *second)),
                (GENERIC_MAP_TYPE_ID, az_uuid::azstd_map(*first, *second)),
                (
                    GENERIC_UNORDERED_MAP_TYPE_ID,
                    az_uuid::azstd_unordered_map(*first, *second),
                ),
            ] {
                if specialized == metadata.specialized_type_id {
                    matches.push(generic);
                }
            }
        }
        _ => {}
    }
    if let Some(specialized) = az_uuid::azstd_tuple(&metadata.templated_type_ids)
        && specialized == metadata.specialized_type_id
    {
        matches.push(GENERIC_TUPLE_TYPE_ID);
    }
    matches.sort_unstable();
    matches.dedup();
    match matches.as_slice() {
        [] => Ok(metadata.generic_type_id.filter(|id| !id.is_nil())),
        [generic] => Ok(Some(*generic)),
        _ => bail!(
            "GenericClassInfo {} matches multiple canonical generic families {:?}",
            metadata.specialized_type_id,
            matches
        ),
    }
}

fn build_generic_info(
    metadata: &GenericMetadata,
    class_keys: &HashMap<String, ReflectedClassKey>,
) -> Result<GenericClassInfo> {
    let mut info = GenericClassInfo::new(
        key_for(
            class_keys,
            &metadata.class_data_id,
            "GenericClassInfo.classData",
        )?,
        metadata.specialized_type_id,
        metadata.generic_type_id,
        metadata.legacy_specialized_type_id,
    )
    .with_templated_type_ids(metadata.templated_type_ids.clone())
    .with_non_type_template_arguments(metadata.non_type_template_arguments.clone());

    if metadata
        .generic_type_id
        .and_then(container_shape_for_generic_uuid)
        == Some(ContainerShape::SmartPointer)
    {
        let [pointee] = metadata.templated_type_ids.as_slice() else {
            bail!(
                "smart-pointer GenericClassInfo {} must have exactly one pointee type, found {}",
                metadata.specialized_type_id,
                metadata.templated_type_ids.len()
            );
        };
        info = info
            .with_smart_pointer_pointee(*pointee)
            .context("attach captured smart-pointer pointee")?;
    }
    Ok(info)
}

/// Captured container `ClassData` records own no `elements`; their element
/// `ClassElements` live on the capturing `GenericClassInfo` instead. Returns that
/// generic-level element evidence when present.
fn generic_level_elements<'a>(
    raw: &'a Value,
    refs: &References<'a>,
) -> Result<Option<&'a Vec<Value>>> {
    let map = resolved_object(raw, refs, "GenericClassInfo")?;
    if optional_bool_value(
        map.get("elementsApplyToClassData"),
        "GenericClassInfo.elementsApplyToClassData",
    )? == Some(false)
    {
        return Ok(None);
    }
    match map.get("elements") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_array()
            .map(|elements| (!elements.is_empty()).then_some(elements))
            .ok_or_else(|| capture_error!("GenericClassInfo.elements must be null or array")),
    }
}

fn parse_class_data(
    raw: &Value,
    refs: &References<'_>,
    class_keys: &HashMap<String, ReflectedClassKey>,
    container_evidence: Option<&ContainerEvidence>,
    generic_elements: Option<&Vec<Value>>,
) -> Result<ReflectedClass> {
    let map = resolved_object(raw, refs, "ClassData")?;
    let type_id = uuid_member(map, "typeId", "ClassData")?;
    let mut class =
        ReflectedClass::new(type_id).with_version(u32_member(map, "version", "ClassData")?);
    if let Some(name) = string_member(map, "name", "ClassData")? {
        class = class.with_name(name);
    }
    if class.current_version == u32::MAX {
        class = class.deprecated();
    }
    if pointer(map, "converter", "ClassData")? {
        class = class.with_captured_version_converter();
    }
    if pointer(map, "dataConverter", "ClassData")? {
        class = class.with_captured_data_converter();
    }
    if pointer(map, "serializer", "ClassData")? {
        class = class.with_captured_serializer();
    }
    // `AZStd::any` and `DynamicSerializableField` are the two dynamic
    // single-value families. `any` carries a non-null `container`, but its
    // native `IDataContainer` fabricates one `m_data` element via `EnumElements`
    // instead of enumerating a collection (findings §5). Modeling it as an
    // opaque container would leave `container_shape` unresolvable and reject
    // every asset that stores an `any`, so seed the `{TypeId, m_data}` field
    // shape below and never mark it a collection container.
    let is_dynamic_value_family =
        type_id == crate::types::AZSTD_ANY || type_id == crate::types::DYNAMIC_SERIALIZABLE_FIELD;
    if pointer(map, "container", "ClassData")? && !is_dynamic_value_family {
        class = apply_container_evidence(class, type_id, container_evidence);
    }

    let own_elements = array_member(map, "elements", "ClassData")?;
    for element in own_elements
        .iter()
        .chain(generic_elements.into_iter().flatten())
    {
        parse_class_element(&mut class, element, refs, class_keys, type_id)?;
    }
    if is_dynamic_value_family {
        // Complete the `{TypeId, m_data}` shape native reflection fabricates at
        // load time (findings §5). `TypeId` is an `AZ::Uuid` value (already
        // present on `DynamicSerializableField`; absent on `any`). `m_data` is a
        // dynamic pointer field: its concrete type is the instance's stored
        // TypeId, so it carries no static reflected type and binds any resolved
        // child through the `is_dynamic_field` acceptance path.
        class
            .fields
            .entry(crate::field_name_crc("TypeId"))
            .or_insert_with(|| {
                ReflectedField::new(crate::types::AZ_UUID).with_field_name("TypeId")
            });
        class.fields.insert(
            crate::field_name_crc("m_data"),
            ReflectedField::new(Uuid::nil())
                .with_field_name("m_data")
                .as_dynamic_field()
                .as_pointer(),
        );
    }
    Ok(class)
}

/// Lower one captured `IDataContainer` into a shape and cardinality contract.
///
/// Prefer the owning `GenericClassInfo`'s family; fall back to the `ClassData`
/// UUID for containers registered without one.
fn apply_container_evidence(
    class: ReflectedClass,
    type_id: Uuid,
    container_evidence: Option<&ContainerEvidence>,
) -> ReflectedClass {
    let mut class = class.as_container();
    let generic_shape = container_evidence
        .and_then(|evidence| evidence.generic_type_id)
        .and_then(container_shape_for_generic_uuid);
    if let Some(shape) = generic_shape.or_else(|| container_shape_for_class_data_uuid(type_id)) {
        class = class.with_container_shape(shape);
    }

    let generic_cardinality = container_evidence
        .and_then(|evidence| evidence.generic_type_id.map(|generic| (generic, evidence)))
        .and_then(|(generic, evidence)| {
            container_cardinality_for_generic_uuid(
                generic,
                evidence.non_type_capacity,
                Some(evidence.templated_argument_count),
            )
        });
    let cardinality =
        generic_cardinality.or_else(|| container_cardinality_for_class_data_uuid(type_id));
    if let Some(cardinality) = cardinality {
        class = class.with_container_cardinality(cardinality);
    }
    class
}

/// Lower one captured `ClassElement` onto `class`.
fn parse_class_element(
    class: &mut ReflectedClass,
    element: &Value,
    refs: &References<'_>,
    class_keys: &HashMap<String, ReflectedClassKey>,
    owner_type_id: Uuid,
) -> Result<()> {
    let element = resolved_object(element, refs, "ClassElement")?;
    let crc = u32_member(element, "nameCrc", "ClassElement")?;
    let mut field = ReflectedField::new(uuid_member(element, "typeId", "ClassElement")?);
    // `m_nameCrc` is case-folded, so the registration spelling survives
    // only on the class that reflected it. Keep it per field rather than
    // letting the global CRC → name table arbitrate between classes that
    // spell one hash differently.
    if let Some(name) = string_member(element, "name", "ClassElement")? {
        field = field.with_field_name(name);
    }
    if bool_member(element, "is_pointer", "ClassElement")? {
        field = field.as_pointer();
    }
    if bool_member(element, "is_base_class", "ClassElement")? {
        field = field.as_base_class();
    }
    if bool_member(element, "is_dynamic_field", "ClassElement")? {
        field = field.as_dynamic_field();
    }
    if let Some(enum_type_id) = class_element_enum_type(element, refs)? {
        field = field.with_enum_type(enum_type_id);
    }
    if let Some(raw_generic) = element
        .get("genericClassInfo")
        .filter(|value| !value.is_null())
    {
        let generic_id = refs.identity(raw_generic, "ClassElement.genericClassInfo")?;
        refs.require_generic_info(&generic_id)?;
        let metadata = parse_generic_metadata(raw_generic, refs)?;
        field = field.with_generic(build_generic_info(&metadata, class_keys)?);
    }
    // A capture may repeat one element verbatim. Identical records describe
    // one field; conflicting duplicates stay fail-closed.
    match class.fields.get(&crc) {
        None => {
            class.fields.insert(crc, field);
        }
        Some(existing) if *existing == field => {}
        Some(_) => {
            bail!(
                "conflicting duplicate ClassElement CRC {crc:#010x} on ClassData {owner_type_id}"
            );
        }
    }
    Ok(())
}

fn harvest_class_names(
    raw: &Value,
    refs: &References<'_>,
    names: &mut LumberyardHashes,
    rtti_bases: &mut HashSet<(Uuid, Uuid)>,
) -> Result<()> {
    let map = resolved_object(raw, refs, "ClassData")?;
    let type_id = uuid_member(map, "typeId", "ClassData")?;
    if let Some(name) = string_member(map, "name", "ClassData")? {
        names.insert_type_name(type_id, name);
    }
    harvest_rtti(map.get("azRtti"), refs, names, rtti_bases)?;
    harvest_element_names(
        array_member(map, "elements", "ClassData")?,
        refs,
        names,
        rtti_bases,
    )
}

fn harvest_element_names(
    elements: &[Value],
    refs: &References<'_>,
    names: &mut LumberyardHashes,
    rtti_bases: &mut HashSet<(Uuid, Uuid)>,
) -> Result<()> {
    for raw_element in elements {
        let element = resolved_object(raw_element, refs, "ClassElement")?;
        let crc = u32_member(element, "nameCrc", "ClassElement")?;
        if let Some(name) = string_member(element, "name", "ClassElement")? {
            names.crcs.insert(crc, name.into());
        }
        harvest_rtti(element.get("azRtti"), refs, names, rtti_bases)?;
    }
    Ok(())
}

fn harvest_rtti(
    raw: Option<&Value>,
    refs: &References<'_>,
    names: &mut LumberyardHashes,
    bases: &mut HashSet<(Uuid, Uuid)>,
) -> Result<()> {
    let Some(raw) = raw.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let map = resolved_object(raw, refs, "AZ RTTI")?;
    let derived = uuid_member(map, "typeId", "AZ RTTI")?;
    if let Some(name) = string_member(map, "typeName", "AZ RTTI")? {
        names.insert_type_name(derived, name);
    }
    // The capture always emits `hierarchy` (an empty array when a helper
    // enumerates no bases), so a missing or null `hierarchy` on a present
    // `azRtti` is a malformed record rather than a tolerated omission.
    let entries = array_member(map, "hierarchy", "AZ RTTI")?;
    for raw_entry in entries {
        let entry = resolved_object(raw_entry, refs, "AZ RTTI hierarchy entry")?;
        let base = uuid_member(entry, "typeId", "AZ RTTI hierarchy entry")?;
        if let Some(name) = string_member(entry, "typeName", "AZ RTTI hierarchy entry")? {
            names.insert_type_name(base, name);
        }
        if base != derived {
            bases.insert((derived, base));
        }
    }
    Ok(())
}

fn harvest_generic_type_names(
    generic_metadata: &BTreeMap<String, GenericMetadata>,
    explicit_generic_aliases: &HashMap<String, Vec<Uuid>>,
    generic_argument_aliases: &LumberyardHashes,
    names: &mut LumberyardHashes,
) {
    let mut owners = HashMap::<Uuid, Vec<&str>>::new();
    for (identity, metadata) in generic_metadata {
        let mut aliases = metadata.registered_type_ids.clone();
        aliases.push(metadata.specialized_type_id);
        aliases.extend(
            explicit_generic_aliases
                .get(identity)
                .into_iter()
                .flatten()
                .copied(),
        );
        if let Some(legacy) = metadata.legacy_specialized_type_id {
            aliases.push(legacy);
        }
        aliases.sort_unstable();
        aliases.dedup();
        for alias in aliases {
            owners.entry(alias).or_default().push(identity.as_str());
        }
    }
    for identities in owners.values_mut() {
        identities.sort_unstable();
        identities.dedup();
    }

    let mut memo = HashMap::<String, Option<String>>::new();
    let aliases = owners.keys().copied().collect::<Vec<_>>();
    for alias in aliases {
        let Some(identities) = owners.get(&alias) else {
            continue;
        };
        let resolved = identities
            .iter()
            .filter_map(|identity| {
                resolve_generic_type_name(
                    identity,
                    generic_metadata,
                    &owners,
                    generic_argument_aliases,
                    names,
                    &mut memo,
                    &mut HashSet::new(),
                )
            })
            .collect::<HashSet<_>>();
        if resolved.len() == 1
            && let Some(name) = resolved.into_iter().next()
        {
            names.insert_type_name(alias, name);
        }
    }
}

fn resolve_generic_type_name(
    identity: &str,
    generic_metadata: &BTreeMap<String, GenericMetadata>,
    owners: &HashMap<Uuid, Vec<&str>>,
    generic_argument_aliases: &LumberyardHashes,
    names: &LumberyardHashes,
    memo: &mut HashMap<String, Option<String>>,
    visiting: &mut HashSet<String>,
) -> Option<String> {
    if let Some(name) = memo.get(identity) {
        return name.clone();
    }
    if !visiting.insert(identity.to_owned()) {
        return None;
    }
    let result = (|| {
        let metadata = generic_metadata.get(identity)?;
        let base = metadata.class_name.as_deref()?.trim();
        if base.contains('<') || matches!(base, "AZStd::string" | "ByteStream") {
            return Some(base.to_owned());
        }
        let mut arguments = Vec::with_capacity(
            metadata.templated_type_ids.len() + metadata.non_type_template_arguments.len(),
        );
        for type_id in &metadata.templated_type_ids {
            let nested = owners.get(type_id).and_then(|identities| {
                let candidates = identities
                    .iter()
                    .filter_map(|nested| {
                        resolve_generic_type_name(
                            nested,
                            generic_metadata,
                            owners,
                            generic_argument_aliases,
                            names,
                            memo,
                            visiting,
                        )
                    })
                    .collect::<HashSet<_>>();
                (candidates.len() == 1)
                    .then(|| candidates.into_iter().next())
                    .flatten()
            });
            arguments.push(
                nested
                    .or_else(|| {
                        generic_argument_aliases
                            .type_name(type_id)
                            .map(ToString::to_string)
                    })
                    .or_else(|| names.type_name(type_id).map(ToString::to_string))?,
            );
        }
        arguments.extend(
            metadata
                .non_type_template_arguments
                .iter()
                .map(ToString::to_string),
        );
        (!arguments.is_empty()).then(|| format!("{base}<{}>", arguments.join(", ")))
    })();
    visiting.remove(identity);
    memo.insert(identity.to_owned(), result.clone());
    result
}

fn optional_bool_value(value: Option<&Value>, scope: &str) -> Result<Option<bool>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(capture_error!("{scope} must be null or bool")),
    }
}

fn optional_usize_value(value: Option<&Value>, scope: &str) -> Result<Option<usize>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .and_then(|value| value.try_into().ok())
            .map(Some)
            .ok_or_else(|| capture_error!("{scope} must be null or usize")),
        Some(_) => Err(capture_error!("{scope} must be null or usize")),
    }
}

fn non_type_arguments(value: &Value) -> Result<Vec<u64>> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value.as_u64().ok_or_else(|| {
                    capture_error!("nonTypeTemplateArguments[{index}] must be an unsigned integer")
                })
            })
            .collect(),
        Value::Object(values) => {
            if values.len() == 1
                && let Some(capacity) = values.get("capacity")
            {
                return Ok(vec![unsigned_integer(
                    capacity,
                    "nonTypeTemplateArguments.capacity",
                )?]);
            }
            if values.len() == 1
                && let Some(arguments) = values.get("values").and_then(Value::as_array)
            {
                return arguments
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        unsigned_integer(
                            value,
                            &format!("nonTypeTemplateArguments.values[{index}]"),
                        )
                    })
                    .collect();
            }
            bail!("nonTypeTemplateArguments object must contain only capacity or values")
        }
        _ => bail!("nonTypeTemplateArguments must be null, an integer array, or a capacity object"),
    }
}

fn unsigned_integer(value: &Value, scope: &str) -> Result<u64> {
    match value {
        Value::Number(value) => value
            .as_u64()
            .ok_or_else(|| capture_error!("{scope} must be an unsigned integer")),
        Value::String(value) => value
            .parse()
            .with_context(|| format!("{scope} must be an unsigned integer")),
        _ => bail!("{scope} must be an unsigned integer"),
    }
}

fn key_for(
    keys: &HashMap<String, ReflectedClassKey>,
    identity: &str,
    scope: &str,
) -> Result<ReflectedClassKey> {
    keys.get(identity)
        .copied()
        .ok_or_else(|| capture_error!("{scope} refers to undiscovered ClassData {identity}"))
}

fn captured_reference(identity: &str) -> Value {
    let mut reference = Map::new();
    reference.insert("$ref".to_owned(), Value::String(identity.to_owned()));
    Value::Object(reference)
}

fn collect_capture_dependencies(
    value: &Value,
    pending_ids: &mut Vec<String>,
    pending_uuids: &mut Vec<Uuid>,
) -> Result<()> {
    fn visit(
        value: &Value,
        is_definition_root: bool,
        pending_ids: &mut Vec<String>,
        pending_uuids: &mut Vec<Uuid>,
    ) -> Result<()> {
        match value {
            Value::Object(map) => {
                if let Some(raw_ref) = map.get("$ref") {
                    pending_ids.push(reference_key(raw_ref, "$ref")?);
                    return Ok(());
                }
                if !is_definition_root && let Some(raw_id) = map.get("$id") {
                    pending_ids.push(reference_key(raw_id, "$id")?);
                    return Ok(());
                }
                for child in map.values() {
                    visit(child, false, pending_ids, pending_uuids)?;
                }
            }
            Value::Array(values) => {
                for child in values {
                    visit(child, false, pending_ids, pending_uuids)?;
                }
            }
            Value::String(text) => {
                if let Ok(type_id) = Uuid::parse_str(text) {
                    pending_uuids.push(type_id);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
        Ok(())
    }

    visit(value, true, pending_ids, pending_uuids)
}

fn flatten_captured_definition(value: &Value, owner_identity: &str) -> Result<Value> {
    fn flatten(value: &Value, owner_identity: &str, is_definition_root: bool) -> Result<Value> {
        match value {
            Value::Object(map) => {
                if !is_definition_root && let Some(raw_id) = map.get("$id") {
                    let identity = reference_key(raw_id, "$id")?;
                    return Ok(captured_reference(&identity));
                }
                let mut flattened = Map::new();
                for (key, child) in map {
                    flattened.insert(
                        key.clone(),
                        flatten(child, owner_identity, false).with_context(|| {
                            format!("flatten captured definition {owner_identity}.{key}")
                        })?,
                    );
                }
                Ok(Value::Object(flattened))
            }
            Value::Array(values) => values
                .iter()
                .map(|child| flatten(child, owner_identity, false))
                .collect::<Result<Vec<_>>>()
                .map(Value::Array),
            scalar => Ok(scalar.clone()),
        }
    }

    flatten(value, owner_identity, true)
}

#[derive(Debug)]
struct References<'a> {
    by_id: BTreeMap<String, &'a Value>,
}

impl<'a> References<'a> {
    fn new(root: &'a Value) -> Result<Self> {
        let mut by_id = BTreeMap::new();
        collect_references(root, &mut by_id)?;
        let refs = Self { by_id };
        validate_references(root, &refs)?;
        Ok(refs)
    }

    fn resolve(&self, mut value: &'a Value) -> Result<&'a Value> {
        let mut visited = HashSet::new();
        let mut depth = 0;
        while let Some(map) = value.as_object() {
            let Some(raw_ref) = map.get("$ref") else {
                break;
            };
            ensure!(
                map.keys().all(|key| key == "$id" || key == "$ref"),
                "$ref object may contain only $id and $ref metadata"
            );
            depth += 1;
            ensure!(
                depth <= MAX_REF_DEPTH,
                "$ref chain exceeds maximum depth {MAX_REF_DEPTH}"
            );
            let key = reference_key(raw_ref, "$ref")?;
            ensure!(visited.insert(key.clone()), "cyclic $ref chain at {key}");
            value = self
                .by_id
                .get(&key)
                .copied()
                .ok_or_else(|| capture_error!("dangling $ref {key}"))?;
        }
        Ok(value)
    }

    fn identity(&self, raw: &'a Value, scope: &str) -> Result<String> {
        let value = self.resolve(raw)?;
        let map = value
            .as_object()
            .ok_or_else(|| capture_error!("{scope} must resolve to an object"))?;
        let raw_id = member(map, "$id", scope)?;
        let id = reference_key(raw_id, &format!("{scope}.$id"))?;
        ensure!(
            self.by_id
                .get(&id)
                .is_some_and(|indexed| std::ptr::eq(*indexed, value)),
            "{scope} has unindexed or mismatched identity {id}"
        );
        Ok(id)
    }

    fn by_id(&self, identity: &str) -> Result<&'a Value> {
        self.by_id
            .get(identity)
            .copied()
            .ok_or_else(|| capture_error!("unknown captured object identity {identity}"))
    }

    fn object_by_id(&self, identity: &str) -> Result<&'a Map<String, Value>> {
        self.resolve(self.by_id(identity)?)?
            .as_object()
            .ok_or_else(|| capture_error!("captured object {identity} must resolve to an object"))
    }

    fn class_data_ids(&self) -> Vec<String> {
        self.by_id
            .iter()
            .filter_map(|(identity, value)| {
                value
                    .as_object()
                    .filter(|map| is_class_data(map))
                    .map(|_| identity.clone())
            })
            .collect()
    }

    fn generic_info_ids(&self) -> Vec<String> {
        self.by_id
            .iter()
            .filter_map(|(identity, value)| {
                value
                    .as_object()
                    .filter(|map| is_generic_info(map))
                    .map(|_| identity.clone())
            })
            .collect()
    }

    fn require_class_data(&self, identity: &str) -> Result<()> {
        ensure!(
            is_class_data(self.object_by_id(identity)?),
            "captured object {identity} is not ClassData"
        );
        Ok(())
    }

    fn require_generic_info(&self, identity: &str) -> Result<()> {
        ensure!(
            is_generic_info(self.object_by_id(identity)?),
            "captured object {identity} is not GenericClassInfo"
        );
        Ok(())
    }
}

fn collect_references<'a>(value: &'a Value, by_id: &mut BTreeMap<String, &'a Value>) -> Result<()> {
    match value {
        Value::Object(map) => {
            if let Some(raw_id) = map.get("$id") {
                let id = reference_key(raw_id, "$id")?;
                ensure!(
                    by_id.insert(id.clone(), value).is_none(),
                    "duplicate captured $id {id}"
                );
            }
            for child in map.values() {
                collect_references(child, by_id)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_references(child, by_id)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_references<'a>(value: &'a Value, refs: &References<'a>) -> Result<()> {
    match value {
        Value::Object(map) => {
            if map.contains_key("$ref") {
                refs.resolve(value)?;
            }
            for child in map.values() {
                validate_references(child, refs)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                validate_references(child, refs)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn reference_key(value: &Value, scope: &str) -> Result<String> {
    match value {
        Value::String(value) if value.starts_with('#') && value.len() > 1 => Ok(value.clone()),
        Value::String(value) if !value.is_empty() => Ok(format!("#{value}")),
        Value::Number(value) if value.as_u64().is_some() => Ok(format!("#{value}")),
        _ => bail!("{scope} must be a nonempty string or unsigned integer reference identity"),
    }
}

fn is_class_data(map: &Map<String, Value>) -> bool {
    map.contains_key("typeId")
        && map.contains_key("version")
        && map.contains_key("elements")
        && map.contains_key("container")
}

fn is_generic_info(map: &Map<String, Value>) -> bool {
    map.contains_key("specializedTypeId")
        && map.contains_key("registeredTypeIds")
        && map.contains_key("templatedTypeIds")
        && map.contains_key("classData")
}

fn resolved_object<'a>(
    raw: &'a Value,
    refs: &References<'a>,
    scope: &str,
) -> Result<&'a Map<String, Value>> {
    refs.resolve(raw)?
        .as_object()
        .ok_or_else(|| capture_error!("{scope} must resolve to an object"))
}

fn member<'a>(map: &'a Map<String, Value>, key: &str, scope: &str) -> Result<&'a Value> {
    map.get(key)
        .ok_or_else(|| capture_error!("{scope} missing required {key}"))
}

fn object<'a>(root: &'a Value, key: &str, scope: &str) -> Result<&'a Map<String, Value>> {
    member(
        root.as_object()
            .ok_or_else(|| capture_error!("{scope} must be object"))?,
        key,
        scope,
    )?
    .as_object()
    .ok_or_else(|| capture_error!("{scope}.{key} must be object"))
}

fn array<'a>(root: &'a Value, key: &str, scope: &str) -> Result<&'a Vec<Value>> {
    member(
        root.as_object()
            .ok_or_else(|| capture_error!("{scope} must be object"))?,
        key,
        scope,
    )?
    .as_array()
    .ok_or_else(|| capture_error!("{scope}.{key} must be array"))
}

fn array_member<'a>(map: &'a Map<String, Value>, key: &str, scope: &str) -> Result<&'a Vec<Value>> {
    member(map, key, scope)?
        .as_array()
        .ok_or_else(|| capture_error!("{scope}.{key} must be array"))
}

fn uuid_member(map: &Map<String, Value>, key: &str, scope: &str) -> Result<Uuid> {
    uuid_value(member(map, key, scope)?, &format!("{scope}.{key}"))
}

/// Recover a `ClassElement`'s semantic enum identity from its captured attribute
/// list.
///
/// Native stores the enum on the element's `EnumType` attribute rather than in
/// `typeId`, which remains the underlying integral type — so without this the
/// discriminant reaches serde as a bare integer and no enum identity survives.
/// The attribute array holds `[attributeId, object]` pairs and the objects use
/// the same `$id`/`$ref` indirection as everything else, so entries resolve
/// through [`resolved_object`].
///
/// Returns `Ok(None)` when the element has no attributes or none of them is an
/// `EnumType`; a malformed `EnumType` entry is an error rather than a silent
/// skip, since a half-read attribute would drop the identity we are here to
/// recover.
fn class_element_enum_type<'a>(
    element: &'a Map<String, Value>,
    refs: &References<'a>,
) -> Result<Option<Uuid>> {
    let Some(raw_attributes) = element.get("attributes").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let Some(attributes) = raw_attributes.as_array() else {
        return Err(capture_error!("ClassElement.attributes must be array"));
    };
    for entry in attributes {
        // Each entry is an `[attributeId, attribute]` pair.
        let Some(raw_attribute) = entry.as_array().and_then(|pair| pair.get(1)) else {
            continue;
        };
        let attribute = resolved_object(raw_attribute, refs, "ClassElement.attributes")?;
        let name = member(attribute, "attributeName", "ClassElement.attributes")?.as_str();
        if name != Some("EnumType") {
            continue;
        }
        let value = member(attribute, "value", "ClassElement.attributes.EnumType")?
            .as_object()
            .ok_or_else(|| {
                capture_error!("ClassElement.attributes.EnumType.value must be object")
            })?;
        return uuid_member(value, "value", "ClassElement.attributes.EnumType.value").map(Some);
    }
    Ok(None)
}

fn uuid_value(value: &Value, scope: &str) -> Result<Uuid> {
    value
        .as_str()
        .ok_or_else(|| capture_error!("{scope} must be UUID string"))
        .and_then(|value| uuid_text(value, scope))
}

fn uuid_text(value: &str, scope: &str) -> Result<Uuid> {
    Uuid::parse_str(value).with_context(|| format!("invalid UUID {value} in {scope}"))
}

fn optional_uuid(map: &Map<String, Value>, key: &str, scope: &str) -> Result<Option<Uuid>> {
    match member(map, key, scope)? {
        Value::Null => Ok(None),
        value => uuid_value(value, &format!("{scope}.{key}")).map(Some),
    }
}

fn u32_member(map: &Map<String, Value>, key: &str, scope: &str) -> Result<u32> {
    member(map, key, scope)?
        .as_u64()
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| capture_error!("{scope}.{key} must be u32"))
}

fn usize_member(map: &Map<String, Value>, key: &str, scope: &str) -> Result<usize> {
    member(map, key, scope)?
        .as_u64()
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| capture_error!("{scope}.{key} must be usize"))
}

fn bool_member(map: &Map<String, Value>, key: &str, scope: &str) -> Result<bool> {
    member(map, key, scope)?
        .as_bool()
        .ok_or_else(|| capture_error!("{scope}.{key} must be bool"))
}

fn string_member<'a>(
    map: &'a Map<String, Value>,
    key: &str,
    scope: &str,
) -> Result<Option<&'a str>> {
    match member(map, key, scope)? {
        Value::Null => Ok(None),
        Value::String(value) if !value.is_empty() => Ok(Some(value)),
        _ => bail!("{scope}.{key} must be null or nonempty string"),
    }
}

fn pointer(map: &Map<String, Value>, key: &str, scope: &str) -> Result<bool> {
    pointer_identity(map, key, scope).map(|identity| identity.is_some())
}

fn pointer_identity<'a>(
    map: &'a Map<String, Value>,
    key: &str,
    scope: &str,
) -> Result<Option<&'a str>> {
    match member(map, key, scope)? {
        Value::Null => Ok(None),
        Value::String(value) if !value.is_empty() => Ok(Some(value)),
        _ => bail!("{scope}.{key} must be null or pointer string"),
    }
}

fn pair<'a>(value: &'a Value, scope: &str) -> Result<(&'a Value, &'a Value)> {
    match value
        .as_array()
        .ok_or_else(|| capture_error!("{scope} must be pair"))?
        .as_slice()
    {
        [left, right] => Ok((left, right)),
        _ => bail!("{scope} must have two values"),
    }
}

/// Lower every captured `uuidMap` entry to an `(alias, ClassData identity)` pair.
fn lower_direct_aliases(
    refs: &References<'_>,
    uuid_map: &Map<String, Value>,
) -> Result<Vec<(Uuid, String)>> {
    let mut direct_aliases = Vec::with_capacity(uuid_map.len());
    for (raw_alias, raw_class) in uuid_map {
        let alias = uuid_text(raw_alias, "uuidMap key")?;
        let class_id = refs
            .identity(raw_class, "uuidMap ClassData")
            .with_context(|| format!("resolve uuidMap ClassData {alias}"))?;
        refs.require_class_data(&class_id)?;
        direct_aliases.push((alias, class_id));
    }
    Ok(direct_aliases)
}

/// Lower every captured `uuidGenericMap` entry, keyed by `GenericClassInfo`
/// identity so one record can carry several registration aliases.
fn lower_generic_aliases(
    refs: &References<'_>,
    generic_map: &[Value],
) -> Result<HashMap<String, Vec<Uuid>>> {
    let mut explicit_generic_aliases = HashMap::<String, Vec<Uuid>>::new();
    for (index, entry) in generic_map.iter().enumerate() {
        let scope = format!("uuidGenericMap[{index}]");
        let (raw_alias, raw_generic) = pair(entry, &scope)?;
        let alias = uuid_value(raw_alias, &format!("{scope} key"))?;
        let generic_id = refs
            .identity(raw_generic, &format!("{scope} GenericClassInfo"))
            .with_context(|| format!("resolve {scope}"))?;
        refs.require_generic_info(&generic_id)?;
        explicit_generic_aliases
            .entry(generic_id)
            .or_default()
            .push(alias);
    }
    Ok(explicit_generic_aliases)
}

/// Captured `GenericClassInfo` records, indexed for `ClassData` lowering.
struct GenericRecordIndex<'a> {
    metadata: BTreeMap<String, GenericMetadata>,
    container_evidence: HashMap<String, ContainerEvidence>,
    serializer_evidence: HashMap<String, SerializerGenericEvidence>,
    element_evidence: HashMap<String, &'a Vec<Value>>,
}

/// Parse every captured `GenericClassInfo` and fold it into per-`ClassData`
/// container, serializer and element evidence.
///
/// Conflicting evidence for one `ClassData` is fail-closed rather than
/// last-write-wins.
fn index_generic_records<'a>(
    refs: &References<'a>,
    generic_ids: &[String],
) -> Result<GenericRecordIndex<'a>> {
    let mut generic_metadata = BTreeMap::new();
    let mut container_evidence = HashMap::<String, ContainerEvidence>::new();
    let mut serializer_generic_evidence = HashMap::<String, SerializerGenericEvidence>::new();
    let mut generic_element_evidence = HashMap::<String, &Vec<Value>>::new();
    for generic_id in generic_ids {
        let metadata = parse_generic_metadata(refs.by_id(generic_id)?, refs)
            .with_context(|| format!("parse GenericClassInfo {generic_id}"))?;
        refs.require_class_data(&metadata.class_data_id)?;
        // Captured GenericClassInfo records carry the container's element
        // ClassElements at the generic level while their owned ClassData
        // `elements` is empty; retain that evidence for class lowering.
        if let Some(elements) = generic_level_elements(refs.by_id(generic_id)?, refs)
            .with_context(|| format!("parse GenericClassInfo {generic_id} elements"))?
        {
            match generic_element_evidence.entry(metadata.class_data_id.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(elements);
                }
                std::collections::hash_map::Entry::Occupied(entry) if *entry.get() == elements => {}
                std::collections::hash_map::Entry::Occupied(_entry) => {
                    bail!(
                        "ClassData {} has conflicting captured generic element evidence",
                        metadata.class_data_id
                    );
                }
            }
        }
        let serializer_evidence = SerializerGenericEvidence {
            generic_type_id: metadata.generic_type_id,
            templated_type_ids: metadata.templated_type_ids.clone(),
        };
        match serializer_generic_evidence.entry(metadata.class_data_id.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(serializer_evidence);
            }
            std::collections::hash_map::Entry::Occupied(entry)
                if entry.get() == &serializer_evidence => {}
            std::collections::hash_map::Entry::Occupied(_entry) => {
                bail!(
                    "ClassData {} has conflicting captured generic serializer evidence",
                    metadata.class_data_id
                );
            }
        }
        let class_map = refs.object_by_id(&metadata.class_data_id)?;
        if pointer(class_map, "container", "ClassData")? {
            let evidence = ContainerEvidence::from_metadata(&metadata);
            match container_evidence.entry(metadata.class_data_id.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(evidence);
                }
                std::collections::hash_map::Entry::Occupied(entry) if entry.get() == &evidence => {}
                std::collections::hash_map::Entry::Occupied(entry) => {
                    bail!(
                        "ClassData {} has conflicting native container evidence: {:?} vs {:?}",
                        metadata.class_data_id,
                        entry.get(),
                        evidence
                    );
                }
            }
        }
        generic_metadata.insert(generic_id.clone(), metadata);
    }
    Ok(GenericRecordIndex {
        metadata: generic_metadata,
        container_evidence,
        serializer_evidence: serializer_generic_evidence,
        element_evidence: generic_element_evidence,
    })
}

/// One `ClassData` record lowered far enough to decide its serializer.
struct CodecCandidate {
    key: ReflectedClassKey,
    class_id: String,
    type_id: Uuid,
    current_version: u32,
    captured_serializer: bool,
    serializer_identity: Option<String>,
}

/// Lower every captured `ClassData` into the context and collect the
/// serializer evidence needed to bind codecs afterwards.
fn lower_class_records(
    refs: &References<'_>,
    class_ids: &[String],
    class_keys: &HashMap<String, ReflectedClassKey>,
    generics: &GenericRecordIndex<'_>,
    rtti_bases: &mut HashSet<(Uuid, Uuid)>,
    context: &mut ObjectStreamReadContext,
) -> Result<Vec<CodecCandidate>> {
    let mut codec_candidates = Vec::new();
    for class_id in class_ids {
        let class = parse_class_data(
            refs.by_id(class_id)?,
            refs,
            class_keys,
            generics.container_evidence.get(class_id),
            generics.element_evidence.get(class_id).copied(),
        )
        .with_context(|| format!("lower ClassData {class_id}"))?;
        let key = key_for(class_keys, class_id, "ClassData")?;
        let class_map = refs.object_by_id(class_id)?;
        codec_candidates.push(CodecCandidate {
            key,
            class_id: class_id.clone(),
            type_id: class.type_id,
            current_version: class.current_version,
            captured_serializer: class.captured_serializer,
            serializer_identity: pointer_identity(class_map, "serializer", "ClassData")?
                .map(str::to_owned),
        });
        // O3DE `Code/Framework/AzCore/AzCore/Serialization/SerializeContext.cpp`
        // makes `SerializeContext::CanDowncast` fall back to FLG_BASE_CLASS
        // ClassElements when azRtti carries no hierarchy. Register every captured
        // base-class field as an edge so `is_rtti_derived_from` can traverse the
        // omitted hierarchy. `rtti_bases` is drained into the context after this
        // loop.
        for field in class.fields.values() {
            if field.is_base_class && field.type_id != class.type_id {
                rtti_bases.insert((class.type_id, field.type_id));
            }
        }
        context
            .define_class_data(key, class)
            .with_context(|| format!("define ClassData {class_id}"))?;
    }
    Ok(codec_candidates)
}

/// Register every captured `GenericClassInfo` under all of its aliases, then
/// reconstruct the legacy specialized-UUID redirects.
fn register_generic_infos(
    context: &mut ObjectStreamReadContext,
    generic_metadata: &BTreeMap<String, GenericMetadata>,
    explicit_generic_aliases: &HashMap<String, Vec<Uuid>>,
    class_keys: &HashMap<String, ReflectedClassKey>,
) -> Result<()> {
    let mut legacy_redirects = Vec::new();
    for (generic_id, metadata) in generic_metadata {
        let info = build_generic_info(metadata, class_keys)
            .with_context(|| format!("lower GenericClassInfo {generic_id}"))?;
        let mut aliases = metadata.registered_type_ids.clone();
        aliases.extend(
            explicit_generic_aliases
                .get(generic_id)
                .into_iter()
                .flatten()
                .copied(),
        );
        aliases.sort_unstable();
        aliases.dedup();
        for alias in &aliases {
            context.insert_generic(*alias, info.clone());
        }
        if let Some(legacy) = metadata.legacy_specialized_type_id {
            for alias in aliases {
                legacy_redirects.push((legacy, alias));
            }
        }
    }
    for (legacy, target) in legacy_redirects {
        context
            .insert_legacy_generic_redirect(legacy, target)
            .with_context(|| format!("register legacy generic redirect {legacy} -> {target}"))?;
    }
    Ok(())
}

/// Register the captured `enumTypeIdToUnderlyingTypeIdMap`.
fn register_enum_underlying_types(
    context: &mut ObjectStreamReadContext,
    enum_map: &Map<String, Value>,
) -> Result<()> {
    for (enum_id, underlying) in enum_map {
        let enum_id = uuid_text(enum_id, "enumTypeIdToUnderlyingTypeIdMap key")?;
        let underlying = uuid_value(
            underlying,
            "enumTypeIdToUnderlyingTypeIdMap underlying type",
        )?;
        context
            .insert_enum_underlying_type(enum_id, underlying)
            .with_context(|| format!("register enum underlying type {enum_id} -> {underlying}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn duplicate_reference_identity_fails_closed() {
        let root = json!([{"$id": 1}, {"$id": "1"}]);
        assert!(
            References::new(&root)
                .unwrap_err()
                .to_string()
                .contains("duplicate captured $id #1")
        );
    }

    #[test]
    fn dangling_reference_fails_closed() {
        let root = json!({"value": {"$ref": "#404"}});
        assert!(
            References::new(&root)
                .unwrap_err()
                .to_string()
                .contains("dangling $ref #404")
        );
    }

    #[test]
    fn reference_cycle_fails_closed() {
        let root = json!([
            {"$id": 1, "$ref": "#2"},
            {"$id": 2, "$ref": "#1"}
        ]);
        assert!(
            References::new(&root)
                .unwrap_err()
                .to_string()
                .contains("cyclic $ref chain")
        );
    }

    #[test]
    fn excessive_reference_depth_fails_closed() {
        let mut values = (0..=MAX_REF_DEPTH)
            .map(|index| json!({"$id": index, "$ref": format!("#{}", index + 1)}))
            .collect::<Vec<_>>();
        values.push(json!({"$id": MAX_REF_DEPTH + 1, "value": true}));
        assert!(
            References::new(&Value::Array(values))
                .unwrap_err()
                .to_string()
                .contains("exceeds maximum depth")
        );
    }

    #[test]
    fn identical_duplicate_class_element_lowers_to_one_field() {
        let type_id = Uuid::from_u128(0x72F23CE6_385D_4FF4_A494_C42F9069C686);
        let element = json!({
            "name": "m_rarityLevel",
            "nameCrc": 0xE91C_7E5F_u32,
            "typeId": crate::types::UNSIGNED_INT,
            "is_pointer": false,
            "is_base_class": false,
            "is_dynamic_field": false,
            "azRtti": null,
            "genericClassInfo": null
        });
        let raw = json!({
            "name": "ContractItemSimpleData",
            "typeId": type_id,
            "version": 2,
            "converter": null,
            "serializer": null,
            "dataConverter": null,
            "container": null,
            "azRtti": null,
            "elements": [element.clone(), element]
        });
        let refs = References::new(&raw).unwrap();
        let class = parse_class_data(&raw, &refs, &HashMap::new(), None, None).unwrap();
        assert_eq!(class.fields.len(), 1);

        let mut conflicting = element.clone();
        conflicting["is_pointer"] = json!(true);
        let raw = json!({
            "name": "ContractItemSimpleData",
            "typeId": type_id,
            "version": 2,
            "converter": null,
            "serializer": null,
            "dataConverter": null,
            "container": null,
            "azRtti": null,
            "elements": [element, conflicting]
        });
        let refs = References::new(&raw).unwrap();
        let error = parse_class_data(&raw, &refs, &HashMap::new(), None, None).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("conflicting duplicate ClassElement CRC")
        );
    }

    #[test]
    fn rtti_without_hierarchy_harvests_as_no_base_evidence() {
        let derived = Uuid::from_u128(0xFDC58CF7_8109_48F2_8D5D_BCBAF774ABB7);
        // The capture always emits `hierarchy` (empty when no bases are
        // enumerated); a helper record with no bases now carries `[]`.
        let root = json!({
            "typeId": derived,
            "typeName": "AnimationData",
            "isAbstract": false,
            "hierarchy": []
        });
        let refs = References::new(&root).unwrap();
        let mut names = LumberyardHashes::default();
        let mut bases = HashSet::new();
        harvest_rtti(Some(&root), &refs, &mut names, &mut bases).unwrap();
        assert_eq!(
            names.uuids.get(&derived).map(arcstr::ArcStr::as_str),
            Some("AnimationData")
        );
        assert!(bases.is_empty());
    }

    #[test]
    fn generic_level_elements_lower_onto_the_container_class() {
        let element_type = Uuid::from_u128(0xEDFCB2CF_F75D_43BE_B26B_F35821B29247);
        let specialized = az_uuid::azstd_vector(element_type);
        let element_crc = crate::field_name_crc("element");
        let root = json!({
            "uuidMap": {
                (element_type.to_string()): {
                    "$id": 1,
                    "name": "Component",
                    "typeId": element_type,
                    "version": 0,
                    "converter": null,
                    "serializer": null,
                    "dataConverter": null,
                    "container": null,
                    "azRtti": null,
                    "elements": []
                }
            },
            "uuidGenericMap": [[
                specialized.to_string(),
                {
                    "$id": 2,
                    "typeId": specialized,
                    "registeredTypeIds": [specialized],
                    "templatedArgumentCount": 1,
                    "templatedTypeIds": [element_type],
                    "specializedTypeId": specialized,
                    "genericTypeId": GENERIC_VECTOR_TYPE_ID,
                    "legacySpecializedTypeId": null,
                    "nonTypeTemplateArguments": null,
                    "classData": {
                        "$id": 3,
                        "name": "AZStd::vector",
                        "typeId": specialized,
                        "version": 1,
                        "converter": null,
                        "serializer": null,
                        "dataConverter": null,
                        "container": "0x1",
                        "azRtti": null,
                        "elements": []
                    },
                    "elements": [{
                        "name": "element",
                        "nameCrc": element_crc,
                        "typeId": element_type,
                        "is_pointer": true,
                        "is_base_class": false,
                        "is_dynamic_field": false,
                        "azRtti": null,
                        "genericClassInfo": null
                    }]
                }
            ]],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        });
        let capture = CapturedSerializeContext::from_value(root).unwrap();
        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .build_context(&capture)
            .unwrap();

        let parent = context
            .resolve_root_type(3, specialized, None, 1)
            .class
            .expect("vector ClassData resolves");
        let child = context.resolve_root_type(3, element_type, None, 0);
        context
            .validate_reachable_child(parent, Some(element_crc), element_type, child)
            .expect("generic-level element field accepts container child");
        assert_eq!(
            context
                .names()
                .crcs
                .get(&element_crc)
                .map(arcstr::ArcStr::as_str),
            Some("element")
        );
    }

    #[test]
    fn generic_alias_names_retain_nested_template_arguments() {
        let entity = Uuid::from_u128(1);
        let address = Uuid::from_u128(2);
        let byte = Uuid::from_u128(3);
        let inner = Uuid::from_u128(4);
        let outer = Uuid::from_u128(5);
        let outer_alias = Uuid::from_u128(6);
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "inner".to_owned(),
            GenericMetadata {
                class_data_id: "inner-class".to_owned(),
                class_name: Some("AZStd::unordered_map".to_owned()),
                registered_type_ids: vec![inner],
                templated_type_ids: vec![address, byte],
                specialized_type_id: inner,
                generic_type_id: Some(GENERIC_UNORDERED_MAP_TYPE_ID),
                legacy_specialized_type_id: None,
                non_type_template_arguments: Vec::new(),
            },
        );
        metadata.insert(
            "outer".to_owned(),
            GenericMetadata {
                class_data_id: "outer-class".to_owned(),
                class_name: Some("AZStd::unordered_map".to_owned()),
                registered_type_ids: vec![outer],
                templated_type_ids: vec![entity, inner],
                specialized_type_id: outer,
                generic_type_id: Some(GENERIC_UNORDERED_MAP_TYPE_ID),
                legacy_specialized_type_id: None,
                non_type_template_arguments: Vec::new(),
            },
        );
        let explicit = HashMap::from([("outer".to_owned(), vec![outer_alias])]);
        let mut names = LumberyardHashes::new();
        names.insert_type_name(entity, "EntityId");
        names.insert_type_name(address, "AddressType");
        names.insert_type_name(byte, "unsigned char");
        let mut generic_argument_aliases = LumberyardHashes::new();
        generic_argument_aliases.insert_type_name(entity, "AZ::EntityId");
        generic_argument_aliases.insert_type_name(address, "DataPatch::AddressType");
        generic_argument_aliases.insert_type_name(byte, "u8");

        harvest_generic_type_names(&metadata, &explicit, &generic_argument_aliases, &mut names);

        assert_eq!(
            names.type_name(&outer_alias).map(arcstr::ArcStr::as_str),
            Some(
                "AZStd::unordered_map<AZ::EntityId, AZStd::unordered_map<DataPatch::AddressType, u8>>"
            )
        );
        assert_eq!(
            names.type_name(&entity).map(arcstr::ArcStr::as_str),
            Some("EntityId")
        );
    }

    #[test]
    fn selected_root_recipe_round_trips_and_builds_the_same_semantic_closure() {
        let root_type = Uuid::from_u128(0x11111111_2222_3333_4444_555555555555);
        let u32_type = crate::types::UNSIGNED_INT;
        let class = |id: u64, name: &str, type_id: Uuid, serializer: Value, elements: Value| {
            json!({
                "$id": id,
                "name": name,
                "typeId": type_id,
                "version": 0,
                "converter": null,
                "serializer": serializer,
                "container": null,
                "azRtti": null,
                "dataConverter": null,
                "elements": elements
            })
        };
        let root_class = class(
            1,
            "Root",
            root_type,
            Value::Null,
            json!([{
                "$id": 2,
                "name": "Value",
                "nameCrc": 1,
                "typeId": u32_type,
                "is_pointer": false,
                "is_base_class": false,
                "is_dynamic_field": false,
                "azRtti": null,
                "genericClassInfo": null
            }]),
        );
        let u32_class = class(3, "unsigned int", u32_type, json!("builtin-u32"), json!([]));
        let capture = CapturedSerializeContext::from_value(json!({
            "uuidMap": {
                (root_type.to_string()): root_class,
                (u32_type.to_string()): u32_class
            },
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        }))
        .unwrap();

        let selected = capture.select_roots(&[root_type]).unwrap();
        let encoded = selected.to_bytes().unwrap();
        let decoded = CapturedSerializeContext::from_bytes(&encoded).unwrap();
        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .build_context(&decoded)
            .unwrap();

        assert_eq!(
            context.resolve_root_type(3, root_type, None, 0).type_id,
            Some(root_type)
        );
        assert_eq!(
            context.resolve_root_type(3, u32_type, None, 0).type_id,
            Some(u32_type)
        );
    }

    fn leaf_class(id: u64, name: &str, type_id: Uuid, elements: &Value) -> Value {
        json!({
            "$id": id,
            "name": name,
            "typeId": type_id,
            "version": 0,
            "converter": null,
            "serializer": null,
            "dataConverter": null,
            "container": null,
            "azRtti": null,
            "elements": elements
        })
    }

    fn base_class_element(name_crc: u32, type_id: Uuid) -> Value {
        json!({
            "name": "BaseClass1",
            "nameCrc": name_crc,
            "typeId": type_id,
            "is_pointer": false,
            "is_base_class": true,
            "is_dynamic_field": false,
            "azRtti": null,
            "genericClassInfo": null
        })
    }

    #[test]
    fn base_class_elements_supply_native_downcast_evidence() {
        // Mirrors the FLG_BASE_CLASS fallback in native CanDowncast: azRtti omits
        // the hierarchy, but base-class ClassElements still supply downcast edges,
        // including transitively (derived -> mid -> base).
        let base_type = Uuid::from_u128(0x0643CDC7_1111_2222_3333_444444444444);
        let mid_type = Uuid::from_u128(0x0643CDC7_5555_6666_7777_888888888888);
        let derived_type = Uuid::from_u128(0x0643CDC7_9999_AAAA_BBBB_CCCCCCCCCCCC);
        let holder_type = Uuid::from_u128(0x11112222_3333_4444_5555_666677778888);
        let base_crc = crate::field_name_crc("BaseClass1");
        let slot_crc = crate::field_name_crc("slot");

        let root = json!({
            "uuidMap": {
                (base_type.to_string()): leaf_class(1, "BaseFacet", base_type, &json!([])),
                (mid_type.to_string()): leaf_class(
                    2,
                    "FacetedComponent",
                    mid_type,
                    &json!([base_class_element(base_crc, base_type)]),
                ),
                (derived_type.to_string()): leaf_class(
                    3,
                    "AlignToTerrainComponent",
                    derived_type,
                    &json!([base_class_element(base_crc, mid_type)]),
                ),
                (holder_type.to_string()): leaf_class(
                    4,
                    "Holder",
                    holder_type,
                    &json!([{
                        "name": "slot",
                        "nameCrc": slot_crc,
                        "typeId": base_type,
                        "is_pointer": true,
                        "is_base_class": false,
                        "is_dynamic_field": false,
                        "azRtti": null,
                        "genericClassInfo": null
                    }]),
                )
            },
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        });

        let capture = CapturedSerializeContext::from_value(root).unwrap();
        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .build_context(&capture)
            .unwrap();

        let holder = context
            .resolve_root_type(3, holder_type, None, 0)
            .class
            .expect("holder ClassData resolves");

        let mid_child = context.resolve_root_type(3, mid_type, None, 0);
        context
            .validate_reachable_child(holder, Some(slot_crc), mid_type, mid_child)
            .expect("one-hop base-class element supplies downcast evidence");

        let derived_child = context.resolve_root_type(3, derived_type, None, 0);
        context
            .validate_reachable_child(holder, Some(slot_crc), derived_type, derived_child)
            .expect("two-hop base-class chain supplies transitive downcast evidence");
    }

    #[test]
    fn native_skip_policy_discards_unknown_field_children() {
        let parent_type = Uuid::from_u128(0xAABBCCDD_0000_1111_2222_333344445555);
        let u32_type = crate::types::UNSIGNED_INT;
        let known_crc = crate::field_name_crc("known");

        let root = json!({
            "uuidMap": {
                (parent_type.to_string()): json!({
                    "$id": 1,
                    "name": "AoiComponent",
                    "typeId": parent_type,
                    "version": 0,
                    "converter": null,
                    "serializer": null,
                    "dataConverter": null,
                    "container": null,
                    "azRtti": null,
                    "elements": [{
                        "name": "known",
                        "nameCrc": known_crc,
                        "typeId": u32_type,
                        "is_pointer": false,
                        "is_base_class": false,
                        "is_dynamic_field": false,
                        "azRtti": null,
                        "genericClassInfo": null
                    }]
                }),
                (u32_type.to_string()): json!({
                    "$id": 2,
                    "name": "unsigned int",
                    "typeId": u32_type,
                    "version": 0,
                    "converter": null,
                    "serializer": "0x1",
                    "dataConverter": null,
                    "container": null,
                    "azRtti": null,
                    "elements": []
                })
            },
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        });
        let capture = CapturedSerializeContext::from_value(root).unwrap();

        // A JSON stream whose parent carries a child field CRC absent from the
        // captured ClassData — the FAMILY A+E signature.
        let json = format!(
            r#"{{"name":"ObjectStream","version":3,"Objects":[{{"typeId":"{parent}","Objects":[{{"field":"phantom","typeId":"{u32}","value":"5"}}]}}]}}"#,
            parent = parent_type.as_braced().to_string().to_uppercase(),
            u32 = u32_type.as_braced().to_string().to_uppercase(),
        );

        let skip_context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .with_unmatched_child_policy(UnmatchedChildPolicy::NativeSkip)
            .build_context(&capture)
            .unwrap();
        let stream = crate::ObjectStream::from_bytes_with_context(json.as_bytes(), &skip_context)
            .expect("native-skip load succeeds by discarding the unbound child");
        assert_eq!(stream.elements().len(), 1);
        assert_eq!(stream.elements()[0].elements.len(), 0);
        assert_eq!(stream.skipped_children, 1);

        let fail_context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .build_context(&capture)
            .unwrap();
        let error = crate::ObjectStream::from_bytes_with_context(json.as_bytes(), &fail_context)
            .expect_err("strict default rejects the unbound child");
        assert!(matches!(
            error,
            crate::ObjectStreamError::UnexpectedReachableChild { .. }
        ));
    }

    #[test]
    fn native_skip_policy_keeps_hard_errors() {
        let parent_type = Uuid::from_u128(0x12340000_1111_2222_3333_444455556666);
        let target_type = Uuid::from_u128(0x56780000_1111_2222_3333_444455556666);
        let u32_type = crate::types::UNSIGNED_INT;
        let slot_crc = crate::field_name_crc("slot");

        let root = json!({
            "uuidMap": {
                (parent_type.to_string()): json!({
                    "$id": 1,
                    "name": "Parent",
                    "typeId": parent_type,
                    "version": 0,
                    "converter": null,
                    "serializer": null,
                    "dataConverter": null,
                    "container": null,
                    "azRtti": null,
                    "elements": [{
                        "name": "slot",
                        "nameCrc": slot_crc,
                        "typeId": target_type,
                        "is_pointer": false,
                        "is_base_class": false,
                        "is_dynamic_field": false,
                        "azRtti": null,
                        "genericClassInfo": null
                    }]
                }),
                (target_type.to_string()): json!({
                    "$id": 2,
                    "name": "Target",
                    "typeId": target_type,
                    "version": 0,
                    "converter": null,
                    "serializer": null,
                    "dataConverter": "0x1",
                    "container": null,
                    "azRtti": null,
                    "elements": []
                }),
                (u32_type.to_string()): json!({
                    "$id": 3,
                    "name": "unsigned int",
                    "typeId": u32_type,
                    "version": 0,
                    "converter": null,
                    "serializer": "0x1",
                    "dataConverter": null,
                    "container": null,
                    "azRtti": null,
                    "elements": []
                })
            },
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        });
        let capture = CapturedSerializeContext::from_value(root).unwrap();

        // The `slot` child is a u32 that needs a captured (but not executable)
        // dataConverter — OUR gap, not something native default-filter skips.
        let json = format!(
            r#"{{"name":"ObjectStream","version":3,"Objects":[{{"typeId":"{parent}","Objects":[{{"field":"slot","typeId":"{u32}","value":"5"}}]}}]}}"#,
            parent = parent_type.as_braced().to_string().to_uppercase(),
            u32 = u32_type.as_braced().to_string().to_uppercase(),
        );

        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .with_unmatched_child_policy(UnmatchedChildPolicy::NativeSkip)
            .build_context(&capture)
            .unwrap();
        let error = crate::ObjectStream::from_bytes_with_context(json.as_bytes(), &context)
            .expect_err("captured-only dataConverter stays a hard error under NativeSkip");
        assert!(matches!(
            error,
            crate::ObjectStreamError::UnsupportedDataConversion { .. }
        ));
    }

    #[test]
    fn m_data_field_name_is_seeded() {
        let u32_type = crate::types::UNSIGNED_INT;
        let root = json!({
            "uuidMap": {
                (u32_type.to_string()): json!({
                    "$id": 1,
                    "name": "unsigned int",
                    "typeId": u32_type,
                    "version": 0,
                    "converter": null,
                    "serializer": "0x1",
                    "dataConverter": null,
                    "container": null,
                    "azRtti": null,
                    "elements": []
                })
            },
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        });
        let capture = CapturedSerializeContext::from_value(root).unwrap();
        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .build_context(&capture)
            .unwrap();
        assert_eq!(
            context
                .names()
                .field_name(crate::field_name_crc("m_data"))
                .map(arcstr::ArcStr::as_str),
            Some("m_data")
        );
    }

    #[test]
    fn any_and_dynamic_serializable_field_seed_typeid_and_m_data_shape() {
        let any_id = crate::types::AZSTD_ANY;
        let dsf_id = crate::types::DYNAMIC_SERIALIZABLE_FIELD;
        let stored_id = crate::types::UNSIGNED_INT;
        let typeid_crc = crate::field_name_crc("TypeId");
        let m_data_crc = crate::field_name_crc("m_data");
        let root = json!({
            "uuidMap": {
                (any_id.to_string()): {
                    "$id": 1, "name": "any", "typeId": any_id, "version": 0,
                    "converter": null, "serializer": null, "dataConverter": null,
                    // `any` carries a non-null container; lowering must not treat
                    // it as an opaque collection. The additive `containerInfo`
                    // key must be tolerated.
                    "container": "capture://container/1", "azRtti": null,
                    "containerInfo": {
                        "vtable": "capture://vtable/1", "slotCount": 29, "slots": []
                    },
                    "elements": []
                },
                (dsf_id.to_string()): {
                    "$id": 2, "name": "DynamicSerializableField", "typeId": dsf_id,
                    "version": 0, "converter": null, "serializer": null,
                    "dataConverter": null, "container": null, "azRtti": null,
                    "elements": [{
                        "name": "TypeId", "nameCrc": typeid_crc,
                        "typeId": crate::types::AZ_UUID, "is_pointer": false,
                        "is_base_class": false, "is_dynamic_field": false,
                        "azRtti": null, "genericClassInfo": null
                    }]
                },
                (stored_id.to_string()): {
                    "$id": 3, "name": "unsigned int", "typeId": stored_id, "version": 0,
                    "converter": null, "serializer": "0x1", "dataConverter": null,
                    "container": null, "azRtti": null, "elements": []
                },
                (crate::types::AZ_UUID.to_string()): {
                    "$id": 4, "name": "Uuid", "typeId": crate::types::AZ_UUID, "version": 0,
                    "converter": null, "serializer": "0x2", "dataConverter": null,
                    "container": null, "azRtti": null, "elements": []
                }
            },
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        });
        let capture = CapturedSerializeContext::from_value(root).unwrap();
        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .build_context(&capture)
            .unwrap();

        // `any` is modeled as a `{TypeId, m_data}` class, never an opaque
        // container: an opaque container with no shape would reject every asset.
        let any = context.resolve_root_type(3, any_id, None, 0);
        assert!(!any.is_container, "any must not be an opaque container");
        let any_class = any.class.expect("any resolves to ClassData");
        let any_reflected = context.class(any_class).expect("any ClassData present");
        let m_data = any_reflected
            .fields
            .get(&m_data_crc)
            .expect("any seeds an m_data field");
        assert!(m_data.is_dynamic_field && m_data.is_pointer);
        assert!(
            any_reflected.fields.contains_key(&typeid_crc),
            "any seeds a TypeId field"
        );

        // The dynamic m_data child accepts any resolved class; the TypeId child
        // binds to the seeded AZ::Uuid field.
        let stored = context.resolve_root_type(3, stored_id, None, 0);
        context
            .validate_reachable_child(any_class, Some(m_data_crc), stored_id, stored)
            .expect("any m_data accepts the stored value type");
        let type_id_child = context.resolve_root_type(3, crate::types::AZ_UUID, None, 0);
        context
            .validate_reachable_child(
                any_class,
                Some(typeid_crc),
                crate::types::AZ_UUID,
                type_id_child,
            )
            .expect("any TypeId child binds to the seeded field");

        // DynamicSerializableField keeps its captured TypeId and gains m_data.
        let dsf = context.resolve_root_type(3, dsf_id, None, 0);
        let dsf_class = dsf.class.expect("DSF resolves to ClassData");
        let dsf_reflected = context.class(dsf_class).expect("DSF ClassData present");
        assert!(dsf_reflected.fields.contains_key(&typeid_crc));
        assert!(
            dsf_reflected
                .fields
                .get(&m_data_crc)
                .is_some_and(|field| field.is_dynamic_field)
        );
        let dsf_stored = context.resolve_root_type(3, stored_id, None, 0);
        context
            .validate_reachable_child(dsf_class, Some(m_data_crc), stored_id, dsf_stored)
            .expect("DSF m_data accepts the stored value type");
    }

    #[test]
    fn byte_stream_serializer_family_descriptor() {
        // Captured `BitSet<96>` family: genericTypeId matches the byte-stream
        // family, captured serializer pointer present, raw word-buffer payload.
        let bitset_generic = Uuid::from_u128(0x1c0270b7_f5e1_4bd6_b7bc_8d25a74b79b4);
        let specialized = Uuid::from_u128(0x0f0e0d0c_0b0a_0908_0706_050403020100);
        let root = json!({
            "uuidMap": {},
            "uuidGenericMap": [[
                specialized.to_string(),
                {
                    "$id": 1,
                    "typeId": specialized,
                    "registeredTypeIds": [specialized],
                    "templatedArgumentCount": 1,
                    "templatedTypeIds": [],
                    "specializedTypeId": specialized,
                    "genericTypeId": bitset_generic,
                    "legacySpecializedTypeId": null,
                    "nonTypeTemplateArguments": [96],
                    "classData": {
                        "$id": 2,
                        "name": "BitSet",
                        "typeId": specialized,
                        "version": 0,
                        "converter": null,
                        "serializer": "0x1",
                        "dataConverter": null,
                        "container": null,
                        "azRtti": null,
                        "elements": []
                    },
                    "elements": []
                }
            ]],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        });
        let capture = CapturedSerializeContext::from_value(root).unwrap();
        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .with_byte_stream_serializer_family(bitset_generic)
            .build_context(&capture)
            .unwrap();

        let resolved = context.resolve_root_type(3, specialized, None, 0);
        assert_eq!(
            resolved.builtin_serializer,
            Some(BuiltinSerializerDescriptor::new(
                crate::codec::BuiltinSerializerKind::ByteStream,
                0,
            ))
        );
    }

    #[test]
    fn byte_stream_serializer_specialized_binds_by_type_id() {
        // `Amazon::Pervasives::UID` analog (F5): a captured non-generic class
        // whose native serializer identity is only a per-run heap pointer, so no
        // `with_serializer_identity` match is possible and its `genericTypeId` is
        // null (the byte-stream family hook cannot reach it). The specialized
        // hook keys its opaque byte-stream codec on the stable reflected type
        // UUID instead.
        let uid_type = Uuid::from_u128(0x3485f20a_98c0_5315_876b_21bcd23a7bc0);
        let root = json!({
            "uuidMap": {
                (uid_type.to_string()): {
                    "$id": 1, "name": "Amazon::Pervasives::UID", "typeId": uid_type,
                    "version": 0, "converter": null,
                    "serializer": "0x27d48ecbb98",
                    "dataConverter": null, "container": null, "azRtti": null,
                    "elements": []
                }
            },
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        });
        let capture = CapturedSerializeContext::from_value(root).unwrap();

        // Without the hook the captured serializer has no executable codec.
        let bare = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .build_context(&capture)
            .unwrap();
        assert_eq!(
            bare.resolve_root_type(3, uid_type, None, 0)
                .builtin_serializer,
            None
        );

        // With it, the opaque payload lowers to the length-agnostic byte stream.
        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .with_byte_stream_serializer_specialized(uid_type)
            .build_context(&capture)
            .unwrap();
        assert_eq!(
            context
                .resolve_root_type(3, uid_type, None, 0)
                .builtin_serializer,
            Some(BuiltinSerializerDescriptor::new(
                crate::codec::BuiltinSerializerKind::ByteStream,
                0,
            ))
        );
    }

    #[test]
    fn seeded_classes_register_absent_reflection_and_bind_a_seeded_field() {
        // F1 analog: two reflected classes the retail capture omits (a component
        // and its configuration member) that cooked slices still carry. Seeding
        // registers both ClassData plus their direct uuidMap aliases so the
        // parent's field resolves to the seeded child and binds instead of
        // failing type resolution.
        let parent_type = Uuid::from_u128(0x173ad9dc_dd96_459f_b64e_1ea470767abe);
        let config_type = Uuid::from_u128(0xf5047b8d_a81f_4c8c_930e_ff1711535d89);
        let cfg_crc = crate::field_name_crc("Configuration");
        let capture = CapturedSerializeContext::from_value(json!({
            "uuidMap": {},
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        }))
        .unwrap();

        let config = ReflectedClass::new(config_type)
            .with_name("Config")
            .with_version(0);
        let mut parent = ReflectedClass::new(parent_type)
            .with_name("Component")
            .with_version(0);
        parent.insert_field(cfg_crc, ReflectedField::new(config_type));

        // Unseeded: the parent type is unregistered and fails resolution.
        let bare = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .build_context(&capture)
            .unwrap();
        assert_eq!(
            bare.resolve_root_type(3, parent_type, None, 0).type_id,
            None
        );

        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .with_seeded_class(config_type, config)
            .with_seeded_class(parent_type, parent)
            .build_context(&capture)
            .unwrap();
        assert_eq!(
            context.resolve_root_type(3, parent_type, None, 0).type_id,
            Some(parent_type)
        );

        // The seeded `Configuration` field binds the seeded child, which is
        // retained under strict load rather than dropped as unmatched.
        let mut child = crate::Element::new(config_type);
        child.name_crc = Some(cfg_crc);
        child.version = Some(0);
        let mut element = crate::Element::new(parent_type);
        element.version = Some(0);
        element.elements = vec![child];
        context.resolve_element_tree(3, &mut element).unwrap();
        assert_eq!(element.elements.len(), 1);
        assert_eq!(element.elements[0].name_crc, Some(cfg_crc));
    }

    #[test]
    fn seeded_generic_field_binds_captured_generic_wire_uuid() {
        let parent_type = Uuid::from_u128(0x806f21d9_11ea_47fc_8b89_fdb67aade4ff);
        let specialized_type = Uuid::from_u128(0x03aaab3f_5c47_5a66_9ebc_d5fa4db353c9);
        let generic_type = Uuid::from_u128(0xef8ff807_ddee_4eb0_b678_4ca3a2c490a4);
        let legacy_type = Uuid::from_u128(0x9046d56f_f77f_0000_3dd0_5677f77f0000);
        let field_crc = crate::field_name_crc("Input Name");
        let capture = CapturedSerializeContext::from_value(json!({
            "uuidMap": {},
            "uuidGenericMap": [[
                specialized_type,
                {
                    "$id": 1,
                    "typeId": specialized_type,
                    "registeredTypeIds": [specialized_type],
                    "templatedArgumentCount": 1,
                    "templatedTypeIds": [crate::types::CHAR],
                    "specializedTypeId": specialized_type,
                    "genericTypeId": generic_type,
                    "legacySpecializedTypeId": legacy_type,
                    "nonTypeTemplateArguments": null,
                    "classData": {
                        "$id": 2,
                        "name": "AZStd::string",
                        "typeId": specialized_type,
                        "version": 0,
                        "converter": null,
                        "serializer": null,
                        "dataConverter": null,
                        "container": null,
                        "azRtti": null,
                        "elements": []
                    },
                    "elements": []
                }
            ]],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        }))
        .unwrap();

        let mut parent = ReflectedClass::new(parent_type);
        parent.insert_field(field_crc, ReflectedField::new(specialized_type));
        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .with_seeded_class(parent_type, parent)
            .with_seeded_generic_field(parent_type, field_crc)
            .build_context(&capture)
            .unwrap();

        let mut child = crate::Element::new(generic_type);
        child.name_crc = Some(field_crc);
        let mut root = crate::Element::new(parent_type).with_children([child]);
        context.resolve_element_tree(3, &mut root).unwrap();
        assert_eq!(
            root.children()[0].resolved_type_id(),
            Some(&specialized_type)
        );
    }

    #[test]
    fn seeded_rtti_edges_retain_derived_pointer_elements() {
        let base_type = Uuid::from_u128(0x100);
        let intermediate_type = Uuid::from_u128(0x101);
        let derived_type = Uuid::from_u128(0x102);
        let vector_type = Uuid::from_u128(0x103);
        let base_crc = crate::field_name_crc("BaseClass1");
        let element_crc = crate::field_name_crc("element");
        let capture = CapturedSerializeContext::from_value(json!({
            "uuidMap": {},
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        }))
        .unwrap();

        let base = ReflectedClass::new(base_type);
        let intermediate = ReflectedClass::new(intermediate_type);
        let mut derived = ReflectedClass::new(derived_type);
        derived.insert_field(
            base_crc,
            ReflectedField::new(intermediate_type).as_base_class(),
        );
        let mut vector = ReflectedClass::new(vector_type)
            .with_container_shape(ContainerShape::Sequence)
            .with_container_cardinality(crate::context::ContainerCardinality::Variable);
        vector.insert_field(element_crc, ReflectedField::new(base_type).as_pointer());

        let recipe = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .with_seeded_class(base_type, base)
            .with_seeded_class(intermediate_type, intermediate)
            .with_seeded_class(derived_type, derived)
            .with_seeded_class(vector_type, vector);
        let without_abstract_base = recipe.build_context(&capture).unwrap();
        let mut rejected_child = crate::Element::new(derived_type).with_name_crc(element_crc);
        rejected_child.version = Some(0);
        let mut rejected_root = crate::Element::new(vector_type).with_children([rejected_child]);
        assert!(
            without_abstract_base
                .resolve_element_tree(3, &mut rejected_root)
                .is_err()
        );

        let context = recipe
            .with_seeded_rtti_base(intermediate_type, base_type)
            .build_context(&capture)
            .unwrap();
        let mut child = crate::Element::new(derived_type).with_name_crc(element_crc);
        child.version = Some(0);
        let mut root = crate::Element::new(vector_type).with_children([child]);
        context.resolve_element_tree(3, &mut root).unwrap();
        assert_eq!(root.children().len(), 1);
        assert_eq!(root.children()[0].resolved_type_id(), Some(&derived_type));
    }

    #[test]
    fn seeded_container_class_decodes_sequence_elements() {
        // The seed hook must also register container classes (shape + element
        // type): an `AZStd::vector<T>` instantiation the retail capture compiled
        // out. Its `element` field binds each positional child, matching how
        // captured generic vectors fold their generic-level element into fields.
        let vec_type = Uuid::from_u128(0x6434a20b_a21d_5575_aec5_5a7319fbc3fb);
        let elem_type = Uuid::from_u128(0x11703d44_ceb8_4a13_a7ce_2134265beb8a);
        let element_crc = crate::field_name_crc("element");
        let capture = CapturedSerializeContext::from_value(json!({
            "uuidMap": {},
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        }))
        .unwrap();

        let elem = ReflectedClass::new(elem_type)
            .with_name("Element")
            .with_version(1);
        let mut vector = ReflectedClass::new(vec_type)
            .with_name("Vector")
            .with_container_shape(ContainerShape::Sequence)
            .with_container_cardinality(crate::context::ContainerCardinality::Variable);
        vector.insert_field(element_crc, ReflectedField::new(elem_type));

        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .with_seeded_class(elem_type, elem)
            .with_seeded_class(vec_type, vector)
            .build_context(&capture)
            .unwrap();

        let resolved = context.resolve_root_type(3, vec_type, None, 0);
        assert_eq!(resolved.type_id, Some(vec_type));
        assert!(resolved.is_container);
        assert_eq!(resolved.container_shape, Some(ContainerShape::Sequence));

        // Two positional elements bind the seeded `element` field and survive.
        let mut first = crate::Element::new(elem_type);
        first.name_crc = Some(element_crc);
        first.version = Some(1);
        let mut second = crate::Element::new(elem_type);
        second.name_crc = Some(element_crc);
        second.version = Some(1);
        let mut root = crate::Element::new(vec_type);
        root.elements = vec![first, second];
        context.resolve_element_tree(3, &mut root).unwrap();
        assert_eq!(root.elements.len(), 2);
    }

    #[test]
    fn empty_capture_and_no_seeds_fails_loudly_at_element_resolution() {
        // Demonstrates the claim in `CapturedSerializeContext::from_value`'s
        // doc comment: a capture with zero identified ClassData records —
        // whether the file is genuinely broken or the retail build simply
        // compiled every relevant class out and nothing seeded the gap — does
        // not silently produce wrong output. `build_context` succeeds (empty
        // is a legitimate context), but decoding real data through it still
        // fails hard the moment a specific type actually needs resolving.
        let unregistered_type = Uuid::from_u128(0xC0FFEE00_0000_0000_0000_000000000001);
        let capture = CapturedSerializeContext::from_value(json!({
            "uuidMap": {},
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        }))
        .unwrap();

        let context = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .build_context(&capture)
            .unwrap();

        let json = format!(
            r#"{{"name":"ObjectStream","version":3,"Objects":[{{"typeId":"{type_id}","value":"1"}}]}}"#,
            type_id = unregistered_type.as_braced().to_string().to_uppercase(),
        );
        let error = crate::ObjectStream::from_bytes_with_context(json.as_bytes(), &context)
            .expect_err("an empty context must not silently decode an unregistered type");
        assert!(matches!(
            error,
            crate::ObjectStreamError::UnresolvedElementType { type_id } if type_id == unregistered_type
        ));
    }

    #[test]
    fn field_rename_rebinds_child_onto_registered_field() {
        let parent_id = Uuid::from_u128(0xDD164EEF_7DBD_4F7C_BCEF_B01A344A33F2);
        let child_type = Uuid::from_u128(0x287CEE87_6FF3_52FC_9D32_38255E2C7FE9);
        let from_crc = crate::field_name_crc("m_disabledAttackTypes");
        let to_crc = crate::field_name_crc("m_disabledAttackTypeCrcs");
        // Lock the exact CRCs the AZ_CRC folding must produce for the alias.
        assert_eq!(from_crc, 0xf2e7_cb07);
        assert_eq!(to_crc, 0x6cde_4578);

        let root = json!({
            "uuidMap": {
                (parent_id.to_string()): {
                    "$id": 1, "name": "AbilityComponent", "typeId": parent_id, "version": 1,
                    "converter": null, "serializer": null, "dataConverter": null,
                    "container": null, "azRtti": null,
                    "elements": [{
                        "name": "m_disabledAttackTypeCrcs", "nameCrc": to_crc,
                        "typeId": child_type, "is_pointer": false, "is_base_class": false,
                        "is_dynamic_field": false, "azRtti": null, "genericClassInfo": null
                    }]
                },
                (child_type.to_string()): {
                    "$id": 2, "name": "AttackTypeCrcs", "typeId": child_type, "version": 0,
                    "converter": null, "serializer": null, "dataConverter": null,
                    "container": null, "azRtti": null, "elements": []
                }
            },
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        });
        let capture = CapturedSerializeContext::from_value(root).unwrap();

        let build = |rename: bool| {
            let mut recipe = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard);
            if rename {
                recipe = recipe.with_field_rename(FieldRename {
                    parent_type_id: parent_id,
                    from_name_crc: from_crc,
                    to_name_crc: to_crc,
                });
            }
            recipe.build_context(&capture).unwrap()
        };

        let root_element = || {
            let mut child = crate::Element::new(child_type);
            child.name_crc = Some(from_crc);
            let mut element = crate::Element::new(parent_id);
            element.version = Some(1);
            element.elements = vec![child];
            element
        };

        // Without the rename the old CRC binds to no ClassElement: strict load
        // fails rather than silently dropping the field.
        let strict = build(false);
        let mut without = root_element();
        assert!(strict.resolve_element_tree(3, &mut without).is_err());

        // With the rename the child is rebound onto the registered field and
        // retained under the canonical identity.
        let renamed = build(true);
        let mut with = root_element();
        renamed.resolve_element_tree(3, &mut with).unwrap();
        assert_eq!(with.elements.len(), 1);
        assert_eq!(with.elements[0].name_crc, Some(to_crc));
        assert_eq!(
            with.elements[0].field().map(arcstr::ArcStr::as_str),
            Some("m_disabledAttackTypeCrcs")
        );
    }

    #[derive(Debug)]
    struct DropChildByCrc(u32);

    impl ObjectStreamVersionConverter for DropChildByCrc {
        fn convert(
            &self,
            element: &mut crate::Element,
            _from: u32,
            _to: u32,
        ) -> std::result::Result<(), crate::ObjectStreamError> {
            element.remove_child_by_name_crc(self.0);
            Ok(())
        }
    }

    #[test]
    fn recipe_version_converter_makes_deferred_family_loadable() {
        let class_id = Uuid::from_u128(0xAFD304E4_1773_47C8_855A_8B622398934F);
        let child_type = Uuid::from_u128(0xC1);
        let keep_crc = crate::field_name_crc("Entities");
        let drop_crc = crate::field_name_crc("ComponentAssetDependencies");

        let root = json!({
            "uuidMap": {
                (class_id.to_string()): {
                    "$id": 1, "name": "SliceComponent", "typeId": class_id, "version": 5,
                    // Non-null converter marks the captured deferred family the
                    // recipe must make executable.
                    "converter": "capture://converter/1", "serializer": null,
                    "dataConverter": null, "container": null, "azRtti": null,
                    "elements": [{
                        "name": "Entities", "nameCrc": keep_crc, "typeId": child_type,
                        "is_pointer": false, "is_base_class": false,
                        "is_dynamic_field": false, "azRtti": null, "genericClassInfo": null
                    }]
                },
                (child_type.to_string()): {
                    "$id": 2, "name": "Child", "typeId": child_type, "version": 0,
                    "converter": null, "serializer": null, "dataConverter": null,
                    "container": null, "azRtti": null, "elements": []
                }
            },
            "uuidGenericMap": [],
            "enumTypeIdToUnderlyingTypeIdMap": {}
        });
        let capture = CapturedSerializeContext::from_value(root).unwrap();

        let old_root = || {
            let mut keep = crate::Element::new(child_type);
            keep.name_crc = Some(keep_crc);
            let mut drop = crate::Element::new(child_type);
            drop.name_crc = Some(drop_crc);
            let mut element = crate::Element::new(class_id);
            element.version = Some(4); // on-disk v4 < current v5
            element.elements = vec![keep, drop];
            element
        };

        // A captured deferred converter family cannot load an older version
        // without an executable converter.
        let deferred = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .build_context(&capture)
            .unwrap();
        let mut blocked = old_root();
        assert!(deferred.resolve_element_tree(3, &mut blocked).is_err());

        // Registering the converter runs it: the removed child leaves before
        // reachability binding and the element advances to the current version.
        let executable = ObjectStreamContextRecipe::new(ObjectStreamDialect::Lumberyard)
            .with_version_converter(class_id, Arc::new(DropChildByCrc(drop_crc)))
            .build_context(&capture)
            .unwrap();
        let mut converted = old_root();
        executable.resolve_element_tree(3, &mut converted).unwrap();
        assert_eq!(converted.version, Some(5));
        assert_eq!(converted.elements.len(), 1);
        assert_eq!(converted.elements[0].name_crc, Some(keep_crc));
    }
}
