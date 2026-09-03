//! AZ `TypeRegistry` reflection and compact type-index lookup.
//!
//! The native runtime uses `Amazon::Hub::TypeRegistryInstance`: Rust types
//! register descriptors into it, and `typeindex.json` fills the compact
//! `typeIndex -> UUID` vector used by fragment and polymorphic-value IDs.

use crate::az::Class;
use crate::serialize::MarshalerError;
use crate::serialize::buffer::ReadBuffer;
use az_core::type_info::AzTypeInfo;
use az_gem_contract::{Registries, Registry, RegistryEntry, Unconditional};
use uuid::Uuid;

#[cfg(debug_assertions)]
mod generated {
    #![allow(dead_code)]

    /// One row from the selected native type-registry debug input.
    pub struct StaticEntry {
        pub uuid: [u8; 16],
        pub name: &'static str,
        pub class_index: u32,
        pub type_index: u32,
    }

    include!(concat!(env!("OUT_DIR"), "/type_registry_debug_data.rs"));
}

/// Registered descriptor for one reflected class.
///
/// `type_index` maps to the native JSON field `typeIndex`. Keep that
/// distinction explicit: `FragmentKey` values in state-bundle records are not
/// type indices.
#[derive(Clone, Copy)]
pub struct ClassDesc {
    pub uuid: Uuid,
    pub name: &'static str,
    pub type_index: u32,
    pub message: Option<MessageInfo>,
    pub unmarshal_body: UnmarshalBodyFn,
    #[cfg(debug_assertions)]
    pub native_index: Option<u32>,
}

impl core::fmt::Debug for ClassDesc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClassDesc")
            .field("uuid", &self.uuid)
            .field("name", &self.name)
            .field("type_index", &self.type_index)
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

impl ClassDesc {
    fn from_registration(reg: &ClassRegistration) -> Self {
        Self {
            uuid: reg.uuid,
            name: reg.name,
            type_index: reg.type_index,
            message: reg.message,
            unmarshal_body: reg.unmarshal_body,
            #[cfg(debug_assertions)]
            native_index: debug_snapshot_native_index(&reg.uuid),
        }
    }
}

/// Native-compatible type registry, as a borrowed view over one composed host's
/// [`ClassRegistration`] entries.
///
/// Native shape:
/// - `RegisterType` inserts type descriptors and assigns `localIndex`.
/// - `LoadTypeIndex` loads `typeindex.json` and fills `typeIndex -> UUID`.
/// - `StateFragmentTypeId` and polymorphic values resolve non-zero wire IDs
///   through that type-index vector.
///
/// There is no process-global registry: a host composes its own entries and
/// hands this view to the wire seams that decode compact type identity.
#[derive(Clone, Copy)]
pub struct TypeRegistry<'a> {
    classes: &'a Registry<ClassRegistration>,
}

impl core::fmt::Debug for TypeRegistry<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TypeRegistry")
            .field("classes", &self.classes.len())
            .finish()
    }
}

impl<'a> TypeRegistry<'a> {
    /// View over an already-resolved class registry.
    #[must_use]
    pub const fn new(classes: &'a Registry<ClassRegistration>) -> Self {
        Self { classes }
    }

    /// View over the class registry a host composed, or `None` when no
    /// contribution registered a class at all.
    #[must_use]
    pub fn of(registries: &'a Registries) -> Option<Self> {
        registries.get::<ClassRegistration>().map(Self::new)
    }

    /// Look up a registry entry by AZ UUID.
    #[must_use]
    pub fn class_desc(&self, uuid: &Uuid) -> Option<ClassDesc> {
        self.registration(uuid).map(ClassDesc::from_registration)
    }

    /// Alias for call sites that read more naturally as a registry lookup.
    #[must_use]
    pub fn entry(&self, uuid: &Uuid) -> Option<ClassDesc> {
        self.class_desc(uuid)
    }

    /// Native `typeIndex` lookup by UUID.
    #[must_use]
    pub fn type_index_of(&self, uuid: &Uuid) -> Option<u32> {
        self.registration(uuid)
            .map(|reg| reg.type_index)
            .filter(|type_index| *type_index != 0)
    }

    /// Resolve a non-zero native `typeIndex` to its UUID.
    #[must_use]
    pub fn uuid_for_type_index(&self, type_index: u32) -> Option<Uuid> {
        self.by_type_index(type_index).map(|reg| reg.uuid)
    }

    /// Display name for a non-zero native `typeIndex`.
    #[must_use]
    pub fn name_for_type_index(&self, type_index: u32) -> Option<&'static str> {
        self.by_type_index(type_index).map(|reg| reg.name)
    }

    /// Top-level message metadata for a native `typeIndex`.
    #[must_use]
    pub fn message_info_for_type_index(&self, type_index: u32) -> Option<MessageInfo> {
        self.by_type_index(type_index).and_then(|reg| reg.message)
    }

    /// Native JSON `index` lookup by UUID.
    #[cfg(debug_assertions)]
    #[must_use]
    pub fn native_index_of(&self, uuid: &Uuid) -> Option<u32> {
        debug_snapshot_native_index(uuid)
    }

    /// Resolve the native JSON `index` field to a UUID.
    #[cfg(debug_assertions)]
    #[must_use]
    pub fn uuid_for_index(&self, index: u32) -> Option<Uuid> {
        let entry = generated::STATIC_ENTRIES.get(index as usize)?;
        (entry.class_index == index).then_some(Uuid::from_bytes(entry.uuid))
    }

    /// Release builds do not carry the native JSON debug table.
    #[cfg(not(debug_assertions))]
    #[must_use]
    pub fn uuid_for_index(&self, _index: u32) -> Option<Uuid> {
        None
    }

    #[must_use]
    pub const fn is_loaded(&self) -> bool {
        !self.classes.is_empty()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.classes.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }

    /// Every composed class, in composition order.
    pub fn entries(&self) -> impl Iterator<Item = &'a ClassRegistration> {
        self.classes.entries()
    }

    /// Walk the composed entries and report every way this host's class
    /// identity disagrees with itself or with the native debug snapshot.
    ///
    /// Duplicate UUIDs cannot appear here — that is the composer's duplicate
    /// key, and composition fails naming both contributions. A duplicate
    /// non-zero `type_index` under two different UUIDs is *not* expressible as
    /// the entry key, so it is reported here instead of decided by scan order.
    #[must_use]
    pub fn collect_diagnostics(&self) -> CollectDiagnostics {
        let mut diag = CollectDiagnostics::default();
        let mut claimed: Vec<(u32, &'static str)> = Vec::new();
        for reg in self.classes.entries() {
            diag.checked += 1;
            if reg.type_index == 0 {
                diag.missing_type_index.push(reg.name);
            } else if let Some((_, first)) = claimed
                .iter()
                .find(|(type_index, _)| *type_index == reg.type_index)
            {
                diag.duplicate_type_index.push(TypeIndexClash {
                    type_index: reg.type_index,
                    first,
                    second: reg.name,
                });
            } else {
                claimed.push((reg.type_index, reg.name));
            }
            if let Some(native_type_index) = debug_snapshot_type_index(&reg.uuid)
                && native_type_index != reg.type_index
            {
                diag.type_index_mismatches.push(NameMismatch {
                    name: reg.name,
                    rust: reg.type_index,
                    native: native_type_index,
                });
            }
            if !debug_snapshot_contains_uuid(&reg.uuid) {
                diag.unknown_to_native_debug.push(reg.name);
            }
        }
        diag
    }

    fn registration(self, uuid: &Uuid) -> Option<&'a ClassRegistration> {
        self.classes.entries().find(|reg| reg.uuid == *uuid)
    }

    fn by_type_index(self, type_index: u32) -> Option<&'a ClassRegistration> {
        if type_index == 0 {
            return None;
        }
        self.classes
            .entries()
            .find(|reg| reg.type_index == type_index)
    }
}

pub type UnmarshalBodyFn = for<'a> fn(&mut ReadBuffer<'a>) -> Result<(), MarshalerError>;

fn unmarshal_class<T: Class>(rb: &mut ReadBuffer<'_>) -> Result<(), MarshalerError> {
    T::unmarshal(rb).map(|_| ())
}

/// One reflected class contributed to a host's type registry.
///
/// This is the Rust equivalent of native callers constructing a descriptor and
/// calling `Amazon::Hub::TypeRegistryInstance::RegisterType`.
///
/// # One entry per type
///
/// `#[derive(ClassDesc)]` and `#[derive(Message)]` used to submit *two*
/// registrations for the same type — the class one carrying whatever
/// `#[class_desc(type_index = N)]` said (often `0`) and the message one
/// carrying the resolved envelope index — and lookup papered over the pair by
/// preferring whichever entry had a non-zero `type_index`. Under composition a
/// repeated key is an error naming both contributors, so the derives no longer
/// register anything: whoever declares the type builds exactly one entry and
/// picks the constructor that matches the type's traits. [`Self::of_message`]
/// is the one to reach for whenever `T: Message`, and the bound is what makes
/// that choice checkable instead of a convention.
#[derive(Clone, Copy)]
pub struct ClassRegistration {
    pub uuid: Uuid,
    pub name: &'static str,
    /// Native envelope `typeIndex`; `0` when this type has no compact wire
    /// identity (helper classes reached only through a UUID).
    pub type_index: u32,
    pub message: Option<MessageInfo>,
    pub unmarshal_body: UnmarshalBodyFn,
}

impl core::fmt::Debug for ClassRegistration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClassRegistration")
            .field("uuid", &self.uuid)
            .field("name", &self.name)
            .field("type_index", &self.type_index)
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

impl RegistryEntry for ClassRegistration {
    type Key = Uuid;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "gridmate-class"
    }

    fn key(&self) -> Uuid {
        self.uuid
    }
}

/// Metadata owned by the top-level `IMessage` registration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MessageInfo {
    /// Body starts with the 16-byte native actor/facet routing header.
    pub actor_scoped: bool,
}

impl ClassRegistration {
    /// The entry for a reflected class that is not a top-level message.
    #[must_use]
    pub const fn of<T: Class>() -> Self {
        Self {
            uuid: <T as AzTypeInfo>::TYPE_ID,
            name: <T as AzTypeInfo>::NAME,
            type_index: <T as Class>::TYPE_INDEX,
            message: None,
            unmarshal_body: unmarshal_class::<T>,
        }
    }

    /// The entry for a top-level wire message.
    ///
    /// Carries the envelope index and the protocol metadata the `Message`
    /// derive recorded, so a registration site never repeats `actor_scoped`.
    #[must_use]
    pub const fn of_message<T: crate::message::Message>() -> Self {
        Self {
            uuid: <T as AzTypeInfo>::TYPE_ID,
            name: <T as AzTypeInfo>::NAME,
            type_index: <T as crate::message::Message>::TYPE_INDEX,
            message: Some(<T as crate::message::Message>::INFO),
            unmarshal_body: unmarshal_class::<T>,
        }
    }
}

#[derive(Debug, Default)]
pub struct CollectDiagnostics {
    pub checked: usize,
    pub missing_type_index: Vec<&'static str>,
    pub duplicate_type_index: Vec<TypeIndexClash>,
    pub type_index_mismatches: Vec<NameMismatch>,
    pub unknown_to_native_debug: Vec<&'static str>,
}

#[derive(Debug)]
pub struct NameMismatch {
    pub name: &'static str,
    pub rust: u32,
    pub native: u32,
}

/// Two distinct classes claiming one envelope index. The composer cannot catch
/// this — its key is type identity — so wire dispatch would silently follow
/// whichever entry composed first.
#[derive(Debug)]
pub struct TypeIndexClash {
    pub type_index: u32,
    pub first: &'static str,
    pub second: &'static str,
}

impl CollectDiagnostics {
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.missing_type_index.is_empty()
            && self.duplicate_type_index.is_empty()
            && self.type_index_mismatches.is_empty()
            && self.unknown_to_native_debug.is_empty()
    }
}

/// Debug-build wire-introspection registry entry: name, `type_index`,
/// pretty-printer, and per-field byte ranges for one message type.
#[cfg(debug_assertions)]
pub struct DebugIntrospect {
    pub name: &'static str,
    pub type_index: u32,
    pub pretty_body: for<'a> fn(&mut ReadBuffer<'a>) -> Result<String, MarshalerError>,
    pub fields_body:
        for<'a> fn(
            &mut ReadBuffer<'a>,
        ) -> Result<Vec<crate::serialize::marshaler::DebugField>, MarshalerError>,
}

#[cfg(debug_assertions)]
impl RegistryEntry for DebugIntrospect {
    type Key = u32;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "gridmate-introspect"
    }

    fn key(&self) -> u32 {
        self.type_index
    }
}

#[cfg(debug_assertions)]
impl core::fmt::Debug for DebugIntrospect {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DebugIntrospect")
            .field("name", &self.name)
            .field("type_index", &self.type_index)
            .finish_non_exhaustive()
    }
}

#[cfg(debug_assertions)]
impl DebugIntrospect {
    #[must_use]
    pub const fn of<T: crate::message::Message>() -> Self {
        Self {
            name: <T as AzTypeInfo>::NAME,
            type_index: <T as crate::message::Message>::TYPE_INDEX,
            pretty_body: |rb| {
                let value = <T as crate::serialize::marshaler::Marshaler>::unmarshal(rb)?;
                Ok(format!("{value:#?}\ntrailing_bytes={}", rb.left()))
            },
            fields_body: |rb| {
                let mut rec_rb = crate::serialize::buffer::ReadBuffer::with_recorder(
                    rb.endian(),
                    rb.remaining(),
                );
                let _ = <T as crate::serialize::marshaler::Marshaler>::unmarshal(&mut rec_rb)?;
                Ok(rec_rb
                    .take_recorder()
                    .map(super::super::serialize::buffer::DebugRecorder::into_fields)
                    .unwrap_or_default())
            },
        }
    }
}

/// Wire-introspection entry for one envelope index, from a composed registry.
#[cfg(debug_assertions)]
#[must_use]
pub fn introspect(
    registry: &Registry<DebugIntrospect>,
    type_index: u32,
) -> Option<&DebugIntrospect> {
    registry
        .entries()
        .find(|entry| entry.type_index == type_index)
}

#[cfg(debug_assertions)]
fn debug_snapshot_contains_uuid(uuid: &Uuid) -> bool {
    generated::BY_UUID.get(uuid.as_bytes()).is_some()
}

#[cfg(not(debug_assertions))]
fn debug_snapshot_contains_uuid(_uuid: &Uuid) -> bool {
    true
}

#[cfg(debug_assertions)]
fn debug_snapshot_type_index(uuid: &Uuid) -> Option<u32> {
    let idx = generated::BY_UUID.get(uuid.as_bytes())?;
    Some(generated::STATIC_ENTRIES[*idx].type_index)
}

#[cfg(not(debug_assertions))]
fn debug_snapshot_type_index(_uuid: &Uuid) -> Option<u32> {
    None
}

#[cfg(debug_assertions)]
fn debug_snapshot_native_index(uuid: &Uuid) -> Option<u32> {
    let idx = generated::BY_UUID.get(uuid.as_bytes())?;
    Some(generated::STATIC_ENTRIES[*idx].class_index)
}

#[cfg(debug_assertions)]
const _: () = {
    if !generated::STATIC_ENTRIES.is_empty() {
        let first = &generated::STATIC_ENTRIES[0];
        assert!(
            first.class_index == 0,
            "STATIC_ENTRIES[0].class_index must be 0 (typeregistry invariant)"
        );
        let zero = [0u8; 16];
        let mut i = 0;
        while i < 16 {
            assert!(
                first.uuid[i] == zero[i],
                "STATIC_ENTRIES[0] must be NullType (00000000-0000-0000-0000-000000000000)"
            );
            i += 1;
        }
    }
};
