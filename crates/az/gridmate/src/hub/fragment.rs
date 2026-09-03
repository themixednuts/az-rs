//! `Amazon::Hub::IFragment` — base actor fragment interface.

use std::{any::Any, fmt::Debug};

use bevy_math::{Mat4, Vec3};
use uuid::Uuid;

use super::SequenceNumber;
use crate::az::{Class, TypeRegistry};
use crate::serialize::{Marshaler, MarshalerError, ReadBuffer, WriteBuffer};
use az_core::type_info::AzTypeInfo;
use az_gem_contract::{Registries, Registry, RegistryEntry, Unconditional};

/// RTTI type id for `Amazon::Hub::IFragment`
/// (`MF_RTTI(IFragment, "766994ea-5c1d-47bf-856c-8216052f5957")`).
pub const I_FRAGMENT_TYPE_ID: Uuid = Uuid::from_u128(0x766994ea_5c1d_47bf_856c_8216052f5957);

/// `Amazon::Hub::FragmentCategory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum FragmentCategory {
    #[default]
    Uncategorized = 0,
    PlayerCharacter = 1,
    NonPlayerCharacter = 2,
    ImportantNonPlayerCharacter = 3,
    Spell = 4,
    Projectile = 5,
    Buildable = 6,
    NumCategories = 7,
}

impl FragmentCategory {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uncategorized => "Uncategorized",
            Self::PlayerCharacter => "PlayerCharacter",
            Self::NonPlayerCharacter => "NonPlayerCharacter",
            Self::ImportantNonPlayerCharacter => "ImportantNonPlayerCharacter",
            Self::Spell => "Spell",
            Self::Projectile => "Projectile",
            Self::Buildable => "Buildable",
            Self::NumCategories => "NumCategories",
        }
    }

    /// Source `FragmentCategoryFromString`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Uncategorized" => Some(Self::Uncategorized),
            "PlayerCharacter" => Some(Self::PlayerCharacter),
            "NonPlayerCharacter" => Some(Self::NonPlayerCharacter),
            "ImportantNonPlayerCharacter" => Some(Self::ImportantNonPlayerCharacter),
            "Spell" => Some(Self::Spell),
            "Projectile" => Some(Self::Projectile),
            "Buildable" => Some(Self::Buildable),
            _ => None,
        }
    }
}

impl Marshaler for FragmentCategory {
    const MARSHAL_SIZE: usize = 1;

    fn marshal(&self, wb: &mut WriteBuffer) {
        (*self as u8).marshal(wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        match u8::unmarshal(rb)? {
            0 => Ok(Self::Uncategorized),
            1 => Ok(Self::PlayerCharacter),
            2 => Ok(Self::NonPlayerCharacter),
            3 => Ok(Self::ImportantNonPlayerCharacter),
            4 => Ok(Self::Spell),
            5 => Ok(Self::Projectile),
            6 => Ok(Self::Buildable),
            7 => Ok(Self::NumCategories),
            value => Err(MarshalerError::InvalidDiscriminant { value }),
        }
    }
}

/// Source `FragmentCategoryBitset`.
pub const NUM_FRAGMENT_CATEGORIES: usize = FragmentCategory::NumCategories as usize;
pub type FragmentCategoryBitset = [bool; NUM_FRAGMENT_CATEGORIES];

/// Source `FragmentCategoryToString`.
#[must_use]
pub const fn fragment_category_to_string(category: FragmentCategory) -> &'static str {
    category.as_str()
}

/// Source `FragmentCategoryFromString`.
#[must_use]
pub fn fragment_category_from_string(name: &str) -> Option<FragmentCategory> {
    FragmentCategory::from_name(name)
}

/// Source `kMaximumReplicationFragments`.
pub const MAXIMUM_REPLICATION_FRAGMENTS: usize = u8::MAX as usize;

/// Source `FragmentPoolAllocator::PoolSizeBytes`.
pub const FRAGMENT_POOL_SIZE_BYTES: usize = 3 * 1024;

/// Shared state carried by source `IFragment`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FragmentBase {
    correlation_id: Uuid,
}

impl FragmentBase {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            correlation_id: Uuid::from_u128(0),
        }
    }

    #[must_use]
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    pub const fn set_correlation_id(&mut self, correlation_id: Uuid) {
        self.correlation_id = correlation_id;
    }
}

/// Source `Amazon::Hub::MarshalContext` data used by replicated-state
/// fragments when selecting dirty fields.
#[derive(Debug, Clone, Copy)]
pub struct MarshalContext<'a> {
    pub baseline_seq: SequenceNumber,
    pub filter_target: Option<u64>,
    pub group_baselines: Option<&'a [SequenceNumber]>,
}

impl Default for MarshalContext<'_> {
    fn default() -> Self {
        Self {
            baseline_seq: SequenceNumber::Invalid,
            filter_target: None,
            group_baselines: None,
        }
    }
}

/// Rust port of `Amazon::Hub::IFragment`.
///
/// Concrete fragment types implement this together with either
/// [`super::ReplicatedState`] or [`super::FixedReplicatedState`]. This trait
/// owns the hub-side merge, dirty, and marshal surface from the source
/// hierarchy.
pub trait Fragment: Any + Debug + Send + Sync {
    fn base(&self) -> &FragmentBase;
    fn base_mut(&mut self) -> &mut FragmentBase;

    /// Source `GetCorrelationID()`.
    fn correlation_id(&self) -> Uuid {
        self.base().correlation_id()
    }

    /// Source `SetCorrelationID()`. The C++ setter is `const` via mutable
    /// storage; Rust keeps this explicit on `&mut self`.
    fn set_correlation_id(&mut self, correlation_id: Uuid) {
        self.base_mut().set_correlation_id(correlation_id);
    }

    /// Source `MergeAndUpdateSequence`. Default `IFragment` has no merge body;
    /// replicated-state specializations provide concrete helpers.
    fn merge_and_update_sequence(
        &self,
        _new_fragment: &mut dyn Fragment,
        _seq: SequenceNumber,
        _inherit_previous_network_data_status: bool,
    ) -> Option<Box<dyn Fragment>> {
        None
    }

    fn is_fully_merged_state(&self) -> bool {
        true
    }

    fn has_new_network_data(&self) -> bool {
        false
    }

    fn detected_new_data_in_last_merge(&self) -> bool {
        false
    }

    fn reset_has_new_network_data(&mut self) {}

    fn set_has_new_network_data_on_initial_state(&mut self) {}

    fn update_sequence(&self) -> SequenceNumber {
        SequenceNumber::Invalid
    }

    fn is_fragment_dirty(&self, _baseline: SequenceNumber) -> bool {
        false
    }

    fn params_to_string(&self) -> String {
        "...".to_string()
    }

    fn fragment_to_string(&self) -> String {
        format!(
            "{}({})",
            std::any::type_name::<Self>(),
            self.params_to_string()
        )
    }

    fn is_metadata(&self) -> bool {
        false
    }

    fn category(&self) -> FragmentCategory {
        FragmentCategory::Uncategorized
    }

    fn has_world_position(&self) -> bool {
        false
    }

    fn world_position(&self) -> Option<Vec3> {
        None
    }

    fn transform(&self) -> Option<Mat4> {
        None
    }

    /// `IFragment::MarshalContents` — write delta payload bytes.
    fn marshal_contents(&self, wb: &mut WriteBuffer) -> bool;

    /// `IFragment::MarshalContents` with the source marshal context.
    fn marshal_contents_with(&self, _mc: &MarshalContext<'_>, wb: &mut WriteBuffer) -> bool {
        self.marshal_contents(wb)
    }

    /// `IFragment::UnmarshalContents` — read delta payload bytes.
    ///
    /// # Errors
    ///
    /// Returns whatever the implementor's field decoders raise — typically
    /// [`MarshalerError::BufferUnderrun`] when `rb` ends mid-field, or
    /// [`MarshalerError::InvalidDiscriminant`] on an out-of-range enum byte.
    fn unmarshal_contents(&mut self, rb: &mut ReadBuffer) -> Result<bool, MarshalerError>;

    /// `IFragment::MarshalAttributes`.
    fn marshal_attributes(&self, _mc: &MarshalContext<'_>, _wb: &mut WriteBuffer) -> bool {
        true
    }

    /// `IFragment::UnmarshalAttributes`.
    ///
    /// # Errors
    ///
    /// The base implementation never fails. Implementors that read filter-group
    /// or registered attributes return [`MarshalerError::BufferUnderrun`] when
    /// `_rb` ends before the attribute block is complete.
    fn unmarshal_attributes(&mut self, _rb: &mut ReadBuffer) -> Result<bool, MarshalerError> {
        Ok(true)
    }

    /// `IFragment::MarshalFieldMetadata`.
    fn marshal_field_metadata(&self, _mc: &MarshalContext<'_>, _wb: &mut WriteBuffer) -> bool {
        true
    }

    /// `IFragment::UnmarshalFieldMetadata`.
    ///
    /// # Errors
    ///
    /// The base implementation never fails. Implementors that read per-field
    /// `last_modified` sequence numbers return
    /// [`MarshalerError::BufferUnderrun`] when `_rb` ends before every field
    /// has yielded one.
    fn unmarshal_field_metadata(&mut self, _rb: &mut ReadBuffer) -> Result<bool, MarshalerError> {
        Ok(true)
    }

    /// `IFragment::FullMarshal`.
    fn full_marshal(&self, mc: &MarshalContext<'_>, wb: &mut WriteBuffer) -> bool {
        let wrote_contents = self.marshal_contents_with(mc, wb);
        let wrote_attributes = self.marshal_attributes(mc, wb);
        let wrote_metadata = self.marshal_field_metadata(mc, wb);
        wrote_contents || wrote_attributes || wrote_metadata
    }

    /// `IFragment::FullUnmarshal`.
    ///
    /// # Errors
    ///
    /// Returns the first error raised by [`Self::unmarshal_contents`],
    /// [`Self::unmarshal_attributes`], or [`Self::unmarshal_field_metadata`],
    /// which run in that order and stop at the first failure.
    fn full_unmarshal(&mut self, rb: &mut ReadBuffer) -> Result<bool, MarshalerError> {
        let read_contents = self.unmarshal_contents(rb)?;
        let read_attributes = self.unmarshal_attributes(rb)?;
        let read_metadata = self.unmarshal_field_metadata(rb)?;
        Ok(read_contents || read_attributes || read_metadata)
    }

    fn num_filter_groups(&self) -> usize {
        1
    }

    /// Source group-specific `ShouldSendToClient`; base `IFragment` returns
    /// false for a single group.
    fn should_send_to_client_group(&self, _target: u64, _group_idx: usize) -> bool {
        false
    }

    /// Source overload that checks every group.
    fn should_send_to_client_any_group(&self, target: u64) -> bool {
        (0..self.num_filter_groups())
            .any(|group_idx| self.should_send_to_client_group(target, group_idx))
    }

    /// Source `CreateNewInstance`. Implementors that support source-style
    /// merging return a default instance of their concrete fragment.
    fn create_new_instance(&self) -> Option<Box<dyn Fragment>> {
        None
    }
}

pub type FragmentContentsDecodeFn =
    for<'a> fn(&mut ReadBuffer<'a>) -> Result<Box<dyn Fragment>, MarshalerError>;
pub type FragmentContentsConsumeFn = for<'a> fn(&mut ReadBuffer<'a>) -> Result<(), MarshalerError>;

/// One concrete source `IFragment` type contributed to a host's replication
/// decoder.
#[derive(Clone, Copy)]
pub struct FragmentRegistration {
    pub uuid: Uuid,
    pub name: &'static str,
    pub decode_contents: FragmentContentsDecodeFn,
    pub consume_contents: FragmentContentsConsumeFn,
}

impl Debug for FragmentRegistration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FragmentRegistration")
            .field("uuid", &self.uuid)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl RegistryEntry for FragmentRegistration {
    type Key = Uuid;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "gridmate-fragment"
    }

    fn key(&self) -> Uuid {
        self.uuid
    }
}

impl FragmentRegistration {
    #[must_use]
    pub const fn of<T>() -> Self
    where
        T: Fragment + Class + Default + Debug + 'static,
    {
        Self {
            uuid: <T as AzTypeInfo>::TYPE_ID,
            name: <T as AzTypeInfo>::NAME,
            decode_contents: |rb| {
                let mut fragment = T::default();
                fragment.unmarshal_contents(rb)?;
                Ok(Box::new(fragment))
            },
            consume_contents: |rb| {
                let mut fragment = T::default();
                fragment.unmarshal_contents(rb)?;
                Ok(())
            },
        }
    }
}

/// A host's composed replication decoder: the fragment entries plus the class
/// registry that turns a wire `type_index` into the UUID they are keyed on.
///
/// Both halves come from the same composition, so a fragment that resolves
/// here is guaranteed to be one this host actually registered.
#[derive(Clone, Copy)]
pub struct Fragments<'a> {
    entries: &'a Registry<FragmentRegistration>,
    types: TypeRegistry<'a>,
}

impl Debug for Fragments<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Fragments")
            .field("entries", &self.entries.len())
            .field("types", &self.types)
            .finish()
    }
}

impl<'a> Fragments<'a> {
    #[must_use]
    pub const fn new(entries: &'a Registry<FragmentRegistration>, types: TypeRegistry<'a>) -> Self {
        Self { entries, types }
    }

    /// The composed decoder, or `None` when this host registered no fragment
    /// types or no classes at all.
    #[must_use]
    pub fn of(registries: &'a Registries) -> Option<Self> {
        Some(Self::new(
            registries.get::<FragmentRegistration>()?,
            TypeRegistry::of(registries)?,
        ))
    }

    #[must_use]
    pub const fn types(&self) -> TypeRegistry<'a> {
        self.types
    }

    #[must_use]
    pub fn by_uuid(&self, uuid: Uuid) -> Option<&'a FragmentRegistration> {
        self.entries.entries().find(|entry| entry.uuid == uuid)
    }

    #[must_use]
    pub fn by_type_index(&self, type_index: u32) -> Option<&'a FragmentRegistration> {
        self.by_uuid(self.types.uuid_for_type_index(type_index)?)
    }

    #[must_use]
    pub fn name(&self, type_index: u32) -> Option<&'static str> {
        self.by_type_index(type_index).map(|entry| entry.name)
    }

    /// Every registered fragment type's envelope index, in composition order.
    ///
    /// Composition order is the order contributions ran, so this no longer
    /// sorts or dedups: a repeated fragment UUID is a compose error, and two
    /// fragments sharing an envelope index is a
    /// [`TypeIndexClash`](crate::az::TypeIndexClash) that
    /// [`TypeRegistry::collect_diagnostics`] reports by name.
    #[must_use]
    pub fn type_indices(&self) -> Vec<u32> {
        self.entries
            .entries()
            .filter_map(|entry| self.types.type_index_of(&entry.uuid))
            .collect()
    }

    /// Read one fragment's contents out of `rb` and discard them, advancing the
    /// cursor past the record.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::UnknownTypeIndex`] if `type_index` names no
    /// registered fragment, otherwise any error the registration's
    /// `consume_contents` decoder raises (typically
    /// [`MarshalerError::BufferUnderrun`] on a truncated record).
    pub fn consume(&self, type_index: u32, rb: &mut ReadBuffer<'_>) -> Result<(), MarshalerError> {
        let registration = self
            .by_type_index(type_index)
            .ok_or(MarshalerError::UnknownTypeIndex { type_index })?;
        (registration.consume_contents)(rb)
    }

    /// Decode one fragment's contents out of `rb` into a boxed concrete
    /// fragment.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::UnknownTypeIndex`] if `type_index` names no
    /// registered fragment, otherwise any error the registration's
    /// `decode_contents` decoder raises (typically
    /// [`MarshalerError::BufferUnderrun`] on a truncated record).
    pub fn decode(
        &self,
        type_index: u32,
        rb: &mut ReadBuffer<'_>,
    ) -> Result<Box<dyn Fragment>, MarshalerError> {
        let registration = self
            .by_type_index(type_index)
            .ok_or(MarshalerError::UnknownTypeIndex { type_index })?;
        (registration.decode_contents)(rb)
    }
}
