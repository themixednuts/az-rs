//! AZ runtime type information (`AZ_RTTI`).
//!
//! Use [`az_derive::AzRtti`] for types that need polymorphic RTTI but are not
//! entity components. Components should use [`az_derive::AzComponent`] instead.

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, LinkedList};
use std::sync::Arc;

use crate::type_info::{self, AzTypeInfo, TypeInfo};
use uuid::Uuid;

/// Polymorphic AZ runtime type information (`AZ_RTTI`).
///
/// Native `AZ_RTTI` layers on top of `AZ_TYPE_INFO` and adds base-type
/// enumeration plus dynamic casting helpers.
///
/// Azoth keeps the zero-cost static surface here: every derived type exposes
/// its own [`AzTypeInfo::TYPE_ID`] and any direct base type ids declared in
/// `#[az_rtti("...", BaseA, BaseB)]` or `#[az_component("...", BaseA)]`.
/// Pointer-adjusting dynamic casts can grow separately when a caller actually
/// needs that native behavior.
pub trait AzRtti: AzTypeInfo {
    const BASE_TYPE_IDS: &'static [Uuid] = &[];

    #[inline]
    #[must_use]
    fn is_type_of(type_id: Uuid) -> bool {
        is_type_or_base(type_id, Self::TYPE_ID, Self::BASE_TYPE_IDS)
    }
}

/// The primitive and math types that carry AZ RTTI identity, plus the leaf
/// registrations `az-core` contributes for them.
///
/// One invocation writes both halves from one token list, which is why this
/// closed set needs no roll-call: a type cannot be given RTTI identity here and
/// then quietly left out of [`types`]. Foreign types cannot carry the derive's
/// inherent registration item, so this is the shape that replaces it.
macro_rules! leaf {
    ($($ty:ty),+ $(,)?) => {
        $(impl AzRtti for $ty {})+

        const LEAVES: &[AzTypeRegistration] = &[$(AzTypeRegistration::rtti::<$ty>()),+];
    };
}

leaf!(
    bool,
    i8,
    i16,
    i32,
    i64,
    u8,
    u16,
    u32,
    u64,
    f32,
    f64,
    (),
    crate::crc::Crc32,
    String,
    Uuid,
    glam::Vec2,
    glam::Vec3,
    glam::Vec4,
    glam::Quat,
);

impl<T: AzTypeInfo> AzRtti for Vec<T> {}

impl<T: AzTypeInfo> AzRtti for LinkedList<T> {}

#[allow(clippy::implicit_hasher)]
impl<T: AzTypeInfo> AzRtti for HashSet<T> {}

impl<T: AzTypeInfo> AzRtti for BTreeSet<T> {}

#[allow(clippy::implicit_hasher)]
impl<K: AzTypeInfo, V: AzTypeInfo> AzRtti for HashMap<K, V> {}

impl<K: AzTypeInfo, V: AzTypeInfo> AzRtti for BTreeMap<K, V> {}

impl<T: AzTypeInfo> AzRtti for Box<T> {}

impl<T: AzTypeInfo> AzRtti for Arc<T> {}

impl<T: AzTypeInfo> AzRtti for Option<T> {}

impl<T: AzTypeInfo, const N: usize> AzRtti for [T; N] {}

/// Kind of native AZ type registered into a host's composed type registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzTypeKind {
    Rtti,
    Component,
}

/// Decodes an authored field-id value tree onto one entity's commands.
#[cfg(feature = "bevy")]
pub type BevyComponentApplyValues = fn(
    &mut bevy_ecs::system::EntityCommands<'_>,
    &dyn BevyComponentValue,
) -> Result<(), BevyComponentValueError>;

/// Bevy-specific insertion metadata for a registered AZ component.
#[cfg(feature = "bevy")]
#[derive(Debug, Clone, Copy)]
pub struct BevyComponentRegistration {
    /// Insert the default Bevy-side representation of this AZ component.
    ///
    /// The inserted type can be the registered AZ component itself or a
    /// foreign/runtime type selected by a component descriptor registration.
    pub insert_default: fn(&mut bevy_ecs::system::EntityCommands<'_>),
    /// Read the serialized native component id from an instantiated Bevy
    /// component. Typed scene cookers use this to persist deterministic
    /// network/facet targets beside the reflected entity payload.
    pub component_id: Option<
        fn(
            &bevy_ecs::world::World,
            bevy_ecs::entity::Entity,
        ) -> Option<crate::component::ComponentId>,
    >,
    /// Decode an authored field-id value tree into the Bevy-side component.
    ///
    /// Components without an authored-value adapter leave this unset and use
    /// [`Self::insert_default`]. Engine-authored schemas should register an
    /// adapter so spawnable records are interpreted once at instantiation.
    pub apply_values: Option<BevyComponentApplyValues>,
    /// Optional deterministic policy applied once after an entity table has
    /// inserted every component. This is used for cross-entity native policy
    /// such as selecting one active camera without teaching the shared
    /// lowerer about a particular component schema.
    pub finalize_entity_table:
        Option<fn(&mut bevy_ecs::system::Commands<'_, '_>, &[bevy_ecs::entity::Entity])>,
}

/// Runtime shape exposed by a decoded authored component value tree.
///
/// The trait keeps AZ component registrations independent from the product
/// crate that owns the concrete tree. Registrations request only the shapes
/// they understand; the prefab runtime implements this view for its decoded
/// `SpawnableValue` without copying or re-decoding the tree.
#[cfg(feature = "bevy")]
pub trait BevyComponentValue {
    /// Human-readable value kind used in diagnostics.
    fn kind(&self) -> &'static str;

    /// Return a field from a field-id-addressed struct value.
    fn struct_field(&self, field_id: u32) -> Option<&dyn BevyComponentValue>;

    fn struct_len(&self) -> Option<usize>;

    fn struct_entry(&self, index: usize) -> Option<(u32, &dyn BevyComponentValue)>;

    /// Return the numeric leaf when this is a floating-point value.
    fn as_float(&self) -> Option<f64>;

    fn as_bool(&self) -> Option<bool>;

    fn as_signed(&self) -> Option<i64>;

    fn as_unsigned(&self) -> Option<u64>;

    fn as_string(&self) -> Option<&str>;

    /// Return the logical asset path when this is an asset-path value.
    fn as_asset_path(&self) -> Option<&str>;

    fn as_object_ref(&self) -> Option<&str>;

    fn list_len(&self) -> Option<usize>;

    fn list_item(&self, index: usize) -> Option<&dyn BevyComponentValue>;

    fn variant_id(&self) -> Option<u32>;

    fn variant_payload(&self) -> Option<&dyn BevyComponentValue>;

    fn map_len(&self) -> Option<usize>;

    fn map_entry(&self, index: usize) -> Option<(&str, &dyn BevyComponentValue)>;
}

/// Failure to apply a decoded authored value tree to a Bevy component.
#[cfg(feature = "bevy")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BevyComponentValueError {
    MissingField {
        field_path: Vec<u32>,
    },
    UnexpectedKind {
        field_path: Vec<u32>,
        expected: &'static str,
        actual: &'static str,
    },
    InvalidValue {
        field_path: Vec<u32>,
        reason: String,
    },
}

#[cfg(feature = "bevy")]
impl std::fmt::Display for BevyComponentValueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField { field_path } => {
                write!(
                    formatter,
                    "missing authored field {}",
                    display_field_path(field_path)
                )
            }
            Self::UnexpectedKind {
                field_path,
                expected,
                actual,
            } => write!(
                formatter,
                "authored field {} expected {expected}, found {actual}",
                display_field_path(field_path)
            ),
            Self::InvalidValue { field_path, reason } => write!(
                formatter,
                "authored field {} is invalid: {reason}",
                display_field_path(field_path)
            ),
        }
    }
}

#[cfg(feature = "bevy")]
impl std::error::Error for BevyComponentValueError {}

/// Reusable, path-aware readers for component lowering implementations.
#[cfg(feature = "bevy")]
pub mod bevy_value {
    use super::{BevyComponentValue, BevyComponentValueError};

    fn unexpected(
        value: &dyn BevyComponentValue,
        field_path: &[u32],
        expected: &'static str,
    ) -> BevyComponentValueError {
        BevyComponentValueError::UnexpectedKind {
            field_path: field_path.to_vec(),
            expected,
            actual: value.kind(),
        }
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a struct.
    pub fn required_struct<'a>(
        value: &'a dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<&'a dyn BevyComponentValue, BevyComponentValueError> {
        if value.kind() == "struct" {
            Ok(value)
        } else {
            Err(unexpected(value, field_path, "struct"))
        }
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a struct.
    /// A struct that simply omits `field_id` yields `Ok(None)`.
    pub fn optional_field<'a>(
        value: &'a dyn BevyComponentValue,
        field_id: u32,
        field_path: &[u32],
    ) -> Result<Option<&'a dyn BevyComponentValue>, BevyComponentValueError> {
        required_struct(value, field_path)?;
        Ok(value.struct_field(field_id))
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a struct,
    /// or [`BevyComponentValueError::MissingField`] when it omits `field_id`.
    pub fn required_field<'a>(
        value: &'a dyn BevyComponentValue,
        field_id: u32,
        field_path: &[u32],
    ) -> Result<&'a dyn BevyComponentValue, BevyComponentValueError> {
        optional_field(
            value,
            field_id,
            &field_path[..field_path.len().saturating_sub(1)],
        )?
        .ok_or_else(|| BevyComponentValueError::MissingField {
            field_path: field_path.to_vec(),
        })
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a number,
    /// or [`BevyComponentValueError::InvalidValue`] when it does not fit a
    /// finite `f32`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the guard above rejects non-finite values and anything outside \
                  f32::MIN..=f32::MAX, so the only loss left is the mantissa \
                  narrowing this function exists to perform"
    )]
    pub fn f32(
        value: &dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<::core::primitive::f32, BevyComponentValueError> {
        let number = value
            .as_float()
            .ok_or_else(|| unexpected(value, field_path, "float"))?;
        if !number.is_finite()
            || number < ::core::primitive::f32::MIN.into()
            || number > ::core::primitive::f32::MAX.into()
        {
            return Err(BevyComponentValueError::InvalidValue {
                field_path: field_path.to_vec(),
                reason: "value is not a finite f32".to_string(),
            });
        }
        Ok(number as ::core::primitive::f32)
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a bool.
    pub fn bool(
        value: &dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<::core::primitive::bool, BevyComponentValueError> {
        value
            .as_bool()
            .ok_or_else(|| unexpected(value, field_path, "bool"))
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a signed
    /// integer.
    pub fn signed(
        value: &dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<i64, BevyComponentValueError> {
        value
            .as_signed()
            .ok_or_else(|| unexpected(value, field_path, "signed integer"))
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not an
    /// unsigned integer.
    pub fn unsigned(
        value: &dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<u64, BevyComponentValueError> {
        value
            .as_unsigned()
            .ok_or_else(|| unexpected(value, field_path, "unsigned integer"))
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a string.
    pub fn string<'a>(
        value: &'a dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<&'a str, BevyComponentValueError> {
        value
            .as_string()
            .ok_or_else(|| unexpected(value, field_path, "string"))
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not an asset
    /// path.
    pub fn asset_path<'a>(
        value: &'a dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<&'a str, BevyComponentValueError> {
        value
            .as_asset_path()
            .ok_or_else(|| unexpected(value, field_path, "asset path"))
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not an object
    /// reference.
    pub fn object_ref<'a>(
        value: &'a dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<&'a str, BevyComponentValueError> {
        value
            .as_object_ref()
            .ok_or_else(|| unexpected(value, field_path, "object reference"))
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a struct,
    /// or whatever [`f32`] reports for a component that is missing or not a
    /// finite number.
    pub fn vec3(
        value: &dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<glam::Vec3, BevyComponentValueError> {
        required_struct(value, field_path)?;
        Ok(glam::Vec3::new(
            struct_f32(value, 1, field_path)?,
            struct_f32(value, 2, field_path)?,
            struct_f32(value, 3, field_path)?,
        ))
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a struct,
    /// or whatever [`f32`] reports for a component that is missing or not a
    /// finite number.
    pub fn vec4(
        value: &dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<glam::Vec4, BevyComponentValueError> {
        required_struct(value, field_path)?;
        Ok(glam::Vec4::new(
            struct_f32(value, 1, field_path)?,
            struct_f32(value, 2, field_path)?,
            struct_f32(value, 3, field_path)?,
            struct_f32(value, 4, field_path)?,
        ))
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a struct,
    /// or whatever [`f32`] reports for a component that is missing or not a
    /// finite number.
    pub fn quat(
        value: &dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<glam::Quat, BevyComponentValueError> {
        required_struct(value, field_path)?;
        Ok(glam::Quat::from_xyzw(
            struct_f32(value, 1, field_path)?,
            struct_f32(value, 2, field_path)?,
            struct_f32(value, 3, field_path)?,
            struct_f32(value, 4, field_path)?,
        ))
    }

    fn struct_f32(
        value: &dyn BevyComponentValue,
        field_id: u32,
        parent_path: &[u32],
    ) -> Result<::core::primitive::f32, BevyComponentValueError> {
        let mut path = parent_path.to_vec();
        path.push(field_id);
        f32(required_field(value, field_id, &path)?, &path)
    }

    #[derive(Clone, Copy)]
    pub struct List<'a>(&'a dyn BevyComponentValue);

    impl<'a> List<'a> {
        #[must_use]
        pub fn len(self) -> usize {
            self.0.list_len().unwrap_or_default()
        }

        #[must_use]
        pub fn is_empty(self) -> bool {
            self.len() == 0
        }

        #[must_use]
        pub fn item(self, index: usize) -> Option<&'a dyn BevyComponentValue> {
            self.0.list_item(index)
        }
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a list.
    pub fn list<'a>(
        value: &'a dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<List<'a>, BevyComponentValueError> {
        value
            .list_len()
            .map(|_| List(value))
            .ok_or_else(|| unexpected(value, field_path, "list"))
    }

    #[derive(Clone, Copy)]
    pub struct Variant<'a> {
        pub id: u32,
        pub payload: Option<&'a dyn BevyComponentValue>,
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a
    /// variant.
    pub fn variant<'a>(
        value: &'a dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<Variant<'a>, BevyComponentValueError> {
        value
            .variant_id()
            .map(|id| Variant {
                id,
                payload: value.variant_payload(),
            })
            .ok_or_else(|| unexpected(value, field_path, "variant"))
    }

    #[derive(Clone, Copy)]
    pub struct Map<'a>(&'a dyn BevyComponentValue);

    impl<'a> Map<'a> {
        #[must_use]
        pub fn len(self) -> usize {
            self.0.map_len().unwrap_or_default()
        }

        #[must_use]
        pub fn is_empty(self) -> bool {
            self.len() == 0
        }

        #[must_use]
        pub fn entry(self, index: usize) -> Option<(&'a str, &'a dyn BevyComponentValue)> {
            self.0.map_entry(index)
        }
    }

    /// # Errors
    ///
    /// [`BevyComponentValueError::UnexpectedKind`] when `value` is not a map.
    pub fn map<'a>(
        value: &'a dyn BevyComponentValue,
        field_path: &[u32],
    ) -> Result<Map<'a>, BevyComponentValueError> {
        value
            .map_len()
            .map(|_| Map(value))
            .ok_or_else(|| unexpected(value, field_path, "map"))
    }
}

#[cfg(feature = "bevy")]
fn display_field_path(field_path: &[u32]) -> String {
    if field_path.is_empty() {
        return "<root>".to_string();
    }
    field_path
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// Composed native AZ type registration.
///
/// Registrations are for types a host must be able to resolve by AZ identity,
/// which is why they are enumerated by the crate that owns them and handed to a
/// composing host. Plain `AzRtti` derivation still exposes static native
/// identity through [`AzTypeInfo`] and [`AzRtti`]; `register` is what adds the
/// entry.
#[derive(Debug, Clone, Copy)]
pub struct AzTypeRegistration {
    pub name: &'static str,
    pub native_type_id: Uuid,
    pub base_type_ids: &'static [Uuid],
    pub rust_type_id: fn() -> TypeId,
    pub kind: AzTypeKind,
}

impl AzTypeRegistration {
    #[inline]
    #[must_use]
    pub const fn rtti<T>() -> Self
    where
        T: AzRtti + 'static,
    {
        Self {
            name: <T as AzTypeInfo>::NAME,
            native_type_id: <T as AzTypeInfo>::TYPE_ID,
            base_type_ids: T::BASE_TYPE_IDS,
            rust_type_id: rust_type_id::<T>,
            kind: AzTypeKind::Rtti,
        }
    }

    #[inline]
    #[must_use]
    pub const fn type_entry(self) -> TypeEntry {
        TypeEntry::with_bases(self.name, self.native_type_id, self.base_type_ids)
    }

    #[inline]
    #[must_use]
    pub const fn is_component(self) -> bool {
        matches!(self.kind, AzTypeKind::Component)
    }
}

const fn rust_type_id<T: 'static>() -> TypeId {
    TypeId::of::<T>()
}

/// One registration per AZ type id. The id is the identity a serialized stream
/// carries and every tool looks types up by; two contributions claiming one id
/// disagree about what those bytes deserialize into, which link order used to
/// settle silently.
impl az_gem_contract::RegistryEntry for AzTypeRegistration {
    type Key = Uuid;
    type Requires = az_gem_contract::Unconditional;

    fn registry_name() -> &'static str {
        "az-type"
    }

    fn key(&self) -> Self::Key {
        self.native_type_id
    }
}

/// The types `az-core` itself registers: the RTTI leaves plus its own
/// `#[az_rtti(..., register)]` types.
///
/// Each named const is the item that derive emitted. Dropping one from this
/// list does not compile.
#[must_use]
pub fn types() -> Vec<AzTypeRegistration> {
    LEAVES.iter().chain(DERIVED).copied().collect()
}

const DERIVED: &[AzTypeRegistration] = &[
    crate::asset::AzAssetData::REGISTRATION,
    crate::component::Component::REGISTRATION,
    crate::component::ComponentConfig::REGISTRATION,
    crate::entity::LocalEntityRef::REGISTRATION,
    crate::serialization::data_patch::AddressType::REGISTRATION,
    crate::serialization::data_patch::AddressTypeElement::REGISTRATION,
    crate::serialization::data_patch::LegacyDataPatch::REGISTRATION,
];

/// Register `az-core`'s own AZ type registrations into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<AzTypeRegistration>().register_many(types());
}

/// Every AZ type a host composed: the entries registered directly, then the
/// type half of each composed component lowering.
///
/// The two halves are one view because a lowering carries the type identity of
/// the component it lowers — the chain the linked iterator produced, now over a
/// host's own composition instead of whatever happened to be linked in.
#[must_use]
pub fn composed(registries: &az_gem_contract::Registries) -> Vec<AzTypeRegistration> {
    let direct = registries
        .get::<AzTypeRegistration>()
        .into_iter()
        .flat_map(az_gem_contract::Registry::entries)
        .copied();
    let lowered = crate::component::lowering::composed(registries)
        .into_iter()
        .map(|lowering| lowering.type_registration);
    direct.chain(lowered).collect()
}

/// AZ type identity recorded as catalog data.
///
/// Use this for native AZ/C++ types that need to be visible to tools but do
/// not have a distinct Rust representation. Rust types should still implement
/// [`AzTypeInfo`] / [`AzRtti`] directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeEntry {
    pub type_info: TypeInfo,
    pub base_type_ids: &'static [Uuid],
    pub canonical_type_id: Option<Uuid>,
}

impl TypeEntry {
    #[inline]
    #[must_use]
    pub const fn new(name: &'static str, type_id: Uuid) -> Self {
        Self {
            type_info: TypeInfo::new(name, type_id),
            base_type_ids: &[],
            canonical_type_id: None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn with_bases(
        name: &'static str,
        type_id: Uuid,
        base_type_ids: &'static [Uuid],
    ) -> Self {
        Self {
            type_info: TypeInfo::new(name, type_id),
            base_type_ids,
            canonical_type_id: None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn alias(name: &'static str, type_id: Uuid, canonical_type_id: Uuid) -> Self {
        Self {
            type_info: TypeInfo::new(name, type_id),
            base_type_ids: &[],
            canonical_type_id: Some(canonical_type_id),
        }
    }

    #[inline]
    #[must_use]
    pub const fn of<T: AzTypeInfo>() -> Self {
        Self::new(T::NAME, T::TYPE_ID)
    }

    #[inline]
    #[must_use]
    pub const fn rtti<T: AzRtti>() -> Self {
        Self {
            type_info: type_info::type_info::<T>(),
            base_type_ids: T::BASE_TYPE_IDS,
            canonical_type_id: None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.type_info.name
    }

    #[inline]
    #[must_use]
    pub const fn type_id(self) -> Uuid {
        self.type_info.type_id
    }

    #[inline]
    #[must_use]
    pub const fn is_alias(self) -> bool {
        self.canonical_type_id.is_some()
    }
}

/// Native AZ RTTI identity as a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RttiInfo {
    pub type_info: TypeInfo,
    pub base_type_ids: &'static [Uuid],
}

impl RttiInfo {
    #[inline]
    #[must_use]
    pub const fn new(type_info: TypeInfo, base_type_ids: &'static [Uuid]) -> Self {
        Self {
            type_info,
            base_type_ids,
        }
    }

    #[inline]
    #[must_use]
    pub const fn of<T: AzRtti>() -> Self {
        Self::new(type_info::type_info::<T>(), T::BASE_TYPE_IDS)
    }

    #[inline]
    #[must_use]
    pub const fn is_type_of(self, type_id: Uuid) -> bool {
        is_type_or_base(type_id, self.type_info.type_id, self.base_type_ids)
    }
}

#[inline]
#[must_use]
pub const fn base_type_ids<T: AzRtti>() -> &'static [Uuid] {
    T::BASE_TYPE_IDS
}

#[inline]
#[must_use]
pub const fn rtti_info<T: AzRtti>() -> RttiInfo {
    RttiInfo::of::<T>()
}

#[inline]
#[must_use]
pub const fn is_type_of<T: AzRtti>(type_id: Uuid) -> bool {
    is_type_or_base(type_id, T::TYPE_ID, T::BASE_TYPE_IDS)
}

#[inline]
#[must_use]
pub const fn is_type_or_base(
    type_id: Uuid,
    concrete_type_id: Uuid,
    base_type_ids: &[Uuid],
) -> bool {
    if type_id.as_u128() == concrete_type_id.as_u128() {
        return true;
    }

    let mut index = 0;
    while index < base_type_ids.len() {
        if type_id.as_u128() == base_type_ids[index].as_u128() {
            return true;
        }
        index += 1;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::uuid;

    #[derive(az_derive::AzRtti)]
    #[az_rtti("A1B2C3D4-E5F6-7890-ABCD-EF1234567890")]
    struct ExampleRttiType;

    #[derive(az_derive::AzTypeInfo)]
    #[az_type_info("{B1B2C3D4-E5F6-7890-ABCD-EF1234567890}")]
    struct ExampleRttiBase;

    #[derive(az_derive::AzRtti)]
    #[az_rtti("C1B2C3D4-E5F6-7890-ABCD-EF1234567890", ExampleRttiBase, register)]
    struct ExampleDerivedRttiType;

    #[test]
    fn az_rtti_derive_impls_type_info_and_rtti() {
        fn assert_az_rtti<T: AzRtti>() {}

        assert_az_rtti::<ExampleRttiType>();
        assert_eq!(
            <ExampleRttiType as AzTypeInfo>::TYPE_ID,
            uuid!("A1B2C3D4-E5F6-7890-ABCD-EF1234567890")
        );
        assert_eq!(<ExampleRttiType as AzTypeInfo>::NAME, "ExampleRttiType");
    }

    #[test]
    fn az_rtti_derive_records_direct_base_type_ids() {
        assert_eq!(
            <ExampleDerivedRttiType as AzRtti>::BASE_TYPE_IDS,
            &[<ExampleRttiBase as AzTypeInfo>::TYPE_ID]
        );
        assert_eq!(
            base_type_ids::<ExampleDerivedRttiType>(),
            &[<ExampleRttiBase as AzTypeInfo>::TYPE_ID]
        );
        assert_eq!(
            rtti_info::<ExampleDerivedRttiType>(),
            RttiInfo::new(
                TypeInfo::new(
                    "ExampleDerivedRttiType",
                    <ExampleDerivedRttiType as AzTypeInfo>::TYPE_ID
                ),
                &[<ExampleRttiBase as AzTypeInfo>::TYPE_ID],
            )
        );
        assert!(ExampleDerivedRttiType::is_type_of(
            <ExampleDerivedRttiType as AzTypeInfo>::TYPE_ID
        ));
        assert!(ExampleDerivedRttiType::is_type_of(
            <ExampleRttiBase as AzTypeInfo>::TYPE_ID
        ));
        assert!(!ExampleDerivedRttiType::is_type_of(uuid!(
            "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF"
        )));
    }

    az_gem_contract::declare_caps!(EngineCaps:);

    /// Stands in for the host contribution the gem sweep writes: `az-core`'s
    /// own types plus the one this module's tests define.
    struct Engine;

    impl az_gem_contract::Contribution for Engine {
        type Caps = EngineCaps;

        fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
            az_gem_contract::ContributionDescriptor {
                gem: az_gem_contract::GemId::new("azoth.core"),
                contribution: az_gem_contract::ContributionId::new("rtti"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, EngineCaps>) {
            super::register(ctx);
            ctx.registrar::<AzTypeRegistration>()
                .register(ExampleDerivedRttiType::REGISTRATION);
        }
    }

    fn composed_engine() -> az_gem_contract::Composer {
        let mut composer =
            az_gem_contract::Composer::new(az_gem_contract::GemTargetRole::AssetWorker);
        composer
            .add(Engine, az_gem_contract::ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    #[test]
    fn az_rtti_derive_registers_native_identity() {
        let composer = composed_engine();
        let registration = composed(composer.registries())
            .into_iter()
            .find(|registration| registration.name == "ExampleDerivedRttiType")
            .expect("derived RTTI type should be registered");

        assert_eq!(
            registration.native_type_id,
            uuid!("C1B2C3D4-E5F6-7890-ABCD-EF1234567890")
        );
        assert_eq!(
            registration.base_type_ids,
            &[<ExampleRttiBase as AzTypeInfo>::TYPE_ID]
        );
        assert_eq!(
            (registration.rust_type_id)(),
            std::any::TypeId::of::<ExampleDerivedRttiType>()
        );
        assert_eq!(
            registration.type_entry(),
            TypeEntry::with_bases(
                "ExampleDerivedRttiType",
                <ExampleDerivedRttiType as AzTypeInfo>::TYPE_ID,
                &[<ExampleRttiBase as AzTypeInfo>::TYPE_ID],
            )
        );
    }

    /// A host sees the types its own composition registered — not every type
    /// linked into the process, which is what the inventory chain returned.
    #[test]
    fn a_host_sees_exactly_what_it_composed() {
        let composer = composed_engine();
        let registrations = composed(composer.registries());

        assert_eq!(registrations.len(), types().len() + 1);
        assert!(
            registrations.iter().any(|registration| {
                registration.native_type_id == <bool as AzTypeInfo>::TYPE_ID
            })
        );

        let report = composer.finalize().expect("composition is valid");
        assert!(report.entries.iter().all(|entry| {
            entry.registry != "az-type"
                || (entry.instance.gem.as_str() == "azoth.core"
                    && entry.instance.contribution.as_str() == "rtti")
        }));
    }

    /// Two crates claiming one AZ type id disagree about what a stream carrying
    /// that id deserializes into. Link order used to pick a winner in silence.
    #[test]
    fn two_contributions_claiming_one_type_id_fail_composition() {
        struct Rival;

        impl az_gem_contract::Contribution for Rival {
            type Caps = EngineCaps;

            fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
                az_gem_contract::ContributionDescriptor {
                    gem: az_gem_contract::GemId::new("azoth.core-rival"),
                    contribution: az_gem_contract::ContributionId::new("rtti"),
                    roles: &[],
                }
            }

            fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, EngineCaps>) {
                ctx.registrar::<AzTypeRegistration>()
                    .register(ExampleDerivedRttiType::REGISTRATION);
            }
        }

        let mut composer = composed_engine();
        composer
            .add(Rival, az_gem_contract::ProductActivation::default())
            .expect("an empty floor composes");

        let az_gem_contract::ComposeError::Duplicate {
            registry,
            key,
            first,
            second,
        } = composer
            .finalize()
            .expect_err("a repeated AZ type id fails")
        else {
            panic!("a repeated AZ type id must fail composition");
        };

        assert_eq!(registry, "az-type");
        assert_eq!(key, "c1b2c3d4-e5f6-7890-abcd-ef1234567890");
        assert_eq!(first.gem.as_str(), "azoth.core");
        assert_eq!(second.gem.as_str(), "azoth.core-rival");
    }

    #[test]
    fn az_rtti_helpers_are_const() {
        const DERIVED_RTTI: RttiInfo = rtti_info::<ExampleDerivedRttiType>();
        const IS_BASE: bool =
            is_type_of::<ExampleDerivedRttiType>(<ExampleRttiBase as AzTypeInfo>::TYPE_ID);
        const IS_UNKNOWN: bool =
            is_type_of::<ExampleDerivedRttiType>(uuid!("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF"));

        assert_eq!(
            DERIVED_RTTI.base_type_ids,
            &[<ExampleRttiBase as AzTypeInfo>::TYPE_ID]
        );
        assert!(DERIVED_RTTI.is_type_of(<ExampleRttiBase as AzTypeInfo>::TYPE_ID));
        const { assert!(IS_BASE) };
        const { assert!(!IS_UNKNOWN) };
    }

    #[test]
    fn native_rust_mappings_have_zero_base_rtti() {
        fn assert_az_rtti<T: AzRtti>() {}

        assert_az_rtti::<u8>();
        assert_az_rtti::<crate::crc::Crc32>();
        assert_az_rtti::<Vec<u8>>();
        assert_az_rtti::<LinkedList<u8>>();
        assert_az_rtti::<HashSet<u8>>();
        assert_az_rtti::<BTreeSet<u8>>();
        assert_az_rtti::<HashMap<String, u8>>();
        assert_az_rtti::<BTreeMap<String, u8>>();
        assert_az_rtti::<Box<u8>>();
        assert_az_rtti::<Arc<String>>();
        assert_az_rtti::<Option<Uuid>>();
        assert_az_rtti::<[u8; 3]>();

        assert_eq!(<u8 as AzTypeInfo>::TYPE_ID, crate::uuid::type_ids::U8);
        assert!(<u8 as AzRtti>::BASE_TYPE_IDS.is_empty());
        assert_eq!(
            <Vec<u8> as AzTypeInfo>::TYPE_ID,
            crate::uuid::azstd_vector(crate::uuid::type_ids::U8)
        );
        assert_eq!(
            <[u8; 3] as AzTypeInfo>::TYPE_ID,
            crate::uuid::azstd_array(crate::uuid::type_ids::U8, 3)
        );
        assert_eq!(
            <crate::crc::Crc32 as AzTypeInfo>::TYPE_ID,
            crate::uuid::type_ids::CRC32
        );
    }
}
