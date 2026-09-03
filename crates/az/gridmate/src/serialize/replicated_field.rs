//! Native-compatible replicated-field serialization.
//!
//! Hosts [`ReplicatedFieldHandler<T, M>`], the Rust port of Lumberyard's
//! `MB::ReplicatedFieldHandler<FieldType, MarshallerType>` template. The
//! local source files name this layer in terms of fields, field groups, and
//! `MarshalContents`.
//!
//! This module implements the flat u8-per-8-fields presence mask used by
//! several message-body fixtures. Fixed-size hub fragments use the
//! [`crate::hub::fixed_replicated_state`] helpers instead.

use super::{
    buffer::{ReadBuffer, WriteBuffer},
    error::MarshalerError,
    marshaler::{Codec, DefaultMarshaler, Marshaler},
    quantize::{quantized_u8, quantized_u32, range_f32_u32},
};
use crate::hub::SequenceNumber;
use bevy_math::Vec3;
use std::any::Any;
use std::marker::PhantomData;

/// Source `ReplicatedFieldHandler::Equals` function pointer.
pub type ReplicatedFieldEquals<T> = fn(&T, &T) -> bool;

/// Source `DefaultReplicatedFieldEquals<T>`.
pub fn default_replicated_field_equals<T: PartialEq>(left: &T, right: &T) -> bool {
    left == right
}

#[macro_export]
macro_rules! unmarshal_replicated_fields {
    ($rb:expr, $($field:expr),+ $(,)?) => {{
        let mut bitmask = 0u8;
        let mut field_idx = 0usize;
        $(
            if field_idx % 8 == 0 {
                bitmask = $rb.read_u8()?;
            }
            if (bitmask & (1 << (field_idx % 8))) != 0 {
                $field = <_ as $crate::serialize::Marshaler>::unmarshal($rb)?;
            }
            field_idx += 1;
        )+
        let _ = field_idx;

        Ok::<(), $crate::serialize::MarshalerError>(())
    }};
}

#[macro_export]
macro_rules! marshal_replicated_fields {
    ($wb:expr, $($field:expr),+ $(,)?) => {{
        use $crate::serialize::Marshaler as _;

        let dirty = [$($field.is_dirty()),+];
        let mut group_start = 0usize;
        while group_start < dirty.len() {
            let batch = (dirty.len() - group_start).min(8);
            let mut bitmask = 0u8;
            for bit in 0..batch {
                if dirty[group_start + bit] {
                    bitmask |= 1 << bit;
                }
            }
            $wb.write_u8(bitmask);

            let mut field_idx = 0usize;
            $(
                if field_idx >= group_start && field_idx < group_start + batch && dirty[field_idx] {
                    $field.marshal($wb);
                }
                field_idx += 1;
            )+
            let _ = field_idx;

            group_start += batch;
        }
    }};
}

// ---------------------------------------------------------------------------
// ReplicatedFieldHandler — per-field wire + replication state
// ---------------------------------------------------------------------------
//
// Rust port of Lumberyard's
//
//   template<typename FieldType, typename MarshallerType =
//       Amazon::Pervasives::Marshaller<FieldType>>
//   class ReplicatedFieldHandler : public ReplicatedFieldHandlerBase { … };
//
// Each `MB::ReplicatedFieldHandler<T> m_foo;` slot inside a C++
// `ReplicatedState` maps to one Rust `ReplicatedFieldHandler<T, M>` field
// inside the corresponding `#[derive(ReplicatedState)]` struct. The C++ source
// shape is canonical; Rust names stay idiomatic while preserving the source
// concepts.
//
// # Wire shape
//
// Identical to C++: the field delegates its bytes to the configured
// marshaler policy. The `ReplicatedState`-level bitmask handles per-field
// presence; this type only handles the per-field bytes.
//
// # Replication bookkeeping
//
// Mirrors C++ field-by-field:
//
// | C++                                     | Rust                              |
// |-----------------------------------------|-----------------------------------|
// | `m_value: T`                            | `value: Option<T>`                |
// | `m_lastModified: Hub::SequenceNumber`   | [`last_modified: SequenceNumber`] |
// | `m_defaultValue: Maybe<T>`              | [`default_value: Option<T>`]      |
// | `m_hasNewNetworkData: bool` (mutable)   | [`has_new_network_data: bool`]    |
// | `m_marshaller: MarshallerType`          | `M: Codec<T>` (zero-sized policy) |
// | `m_equals: bool(*)(const T&, const T&)` | trait-bound `T: PartialEq`        |
//
// The Rust port keeps `m_value` private. Callers use source-shaped methods
// (`SetValue`, `Access`, `GetValue`, `HasValue`) so dirty/default/network
// bookkeeping cannot be bypassed by direct assignment.
//
// # Naming
//
// This struct used to be called `DataField`; it has been renamed to
// `ReplicatedFieldHandler` to match the vendor C++ source 1:1. No
// back-compat alias is provided — all call sites use the canonical
// name.

/// Object-safe surface of `MB::ReplicatedFieldHandlerBase` used by
/// `MB::FixedReplicatedState` field-group methods.
pub trait ReplicatedFieldHandlerBase: Any {
    fn is_default_value(&self) -> bool;
    fn set_current_value_as_default(&mut self) {}
    fn is_dirty(&self, baseline: SequenceNumber) -> bool;
    fn has_value(&self) -> bool;
    fn marshal_field(&self, wb: &mut WriteBuffer);
    fn marshal_field_since(&self, wb: &mut WriteBuffer, _baseline: SequenceNumber) {
        self.marshal_field(wb);
    }
    /// Decode this field's payload from `rb` and store it as the current value.
    ///
    /// # Errors
    ///
    /// Returns whatever the field's element codec returns — typically
    /// [`MarshalerError::BufferUnderrun`] if `rb` ends mid-value, and for
    /// bounded element types [`MarshalerError::InvalidRange`] or
    /// [`MarshalerError::ContainerOverflow`] when the decoded value falls
    /// outside the wire contract.
    fn unmarshal_field(&mut self, rb: &mut ReadBuffer) -> Result<(), MarshalerError>;
    fn merge_and_update_sequence(
        &mut self,
        old_value: &dyn ReplicatedFieldHandlerBase,
        new_value: &mut dyn ReplicatedFieldHandlerBase,
        seq: SequenceNumber,
        inherit_previous_network_data_status: bool,
    ) -> bool;
    fn last_modified(&self) -> SequenceNumber;
    fn set_last_modified(&mut self, seq: SequenceNumber);
    fn is_field_valid(&self) -> bool {
        self.last_modified().is_valid()
    }
    fn reset_has_new_network_data(&mut self);
    fn has_new_network_data(&self) -> bool;
}

/// `MB::ReplicatedFieldHandler<T, M>` — one Rust-managed slot of a
/// `ReplicatedState`.
///
/// `M` is a [`Codec<T>`] policy. The default uses `T`'s own
/// [`Marshaler`] impl through [`DefaultMarshaler`].
/// Custom wire shapes (half-float, VLQ, position anchor, …) supply a
/// zero-sized policy type implementing `Codec<T>`.
///
/// See the module-level commentary for the C++ → Rust field mapping.
#[derive(Debug, Clone)]
pub struct ReplicatedFieldHandler<T: Default, M: Codec<T> = DefaultMarshaler<T>> {
    /// Current value. `None` corresponds to the C++ invalid
    /// `m_lastModified` state before a value/default has been installed.
    value: Option<T>,
    /// `m_lastModified` — sequence number when `value` was last set.
    /// `INVALID` initially; transitions to `VALID_NON_SEQUENCE` on
    /// local `set_value`, or to `from_seq(N)` on a network update.
    last_modified: SequenceNumber,
    /// `m_defaultValue: Maybe<T>` — when present and `value == default`,
    /// `is_default_value()` returns true so replication can suppress
    /// the field (matches the C++ "set lastModified Invalid" behaviour
    /// inside `SetValue`).
    default_value: Option<T>,
    /// `m_hasNewNetworkData` — true after network unmarshal or a merge
    /// that detects new remote data. Local `set_value` deliberately does
    /// not set this flag; native C++ keeps local dirty state in
    /// `m_lastModified` and reserves this flag for network/merge status.
    has_new_network_data: bool,
    equals: Option<ReplicatedFieldEquals<T>>,
    marshaler: PhantomData<fn() -> M>,
}

impl<T: Default, M: Codec<T>> Default for ReplicatedFieldHandler<T, M> {
    fn default() -> Self {
        Self {
            value: None,
            last_modified: SequenceNumber::Invalid,
            default_value: None,
            has_new_network_data: false,
            equals: None,
            marshaler: PhantomData,
        }
    }
}

impl<T: Default, M: Codec<T>> ReplicatedFieldHandler<T, M> {
    /// Construct from an explicit optional value.
    /// `last_modified` follows the value's optionality:
    /// `Some(_)` ⇒ [`SequenceNumber::ValidNonSequence`],
    /// `None` ⇒ [`SequenceNumber::Invalid`].
    pub const fn new(value: Option<T>) -> Self {
        let last_modified = if value.is_some() {
            SequenceNumber::ValidNonSequence
        } else {
            SequenceNumber::Invalid
        };
        Self {
            value,
            last_modified,
            default_value: None,
            has_new_network_data: false,
            equals: None,
            marshaler: PhantomData,
        }
    }

    /// Construct with a source-style custom equality policy.
    pub const fn with_equals(equals: ReplicatedFieldEquals<T>) -> Self {
        Self {
            value: None,
            last_modified: SequenceNumber::Invalid,
            default_value: None,
            has_new_network_data: false,
            equals: Some(equals),
            marshaler: PhantomData,
        }
    }

    /// Construct from an optional value and source-style custom equality policy.
    pub const fn new_with_equals(value: Option<T>, equals: ReplicatedFieldEquals<T>) -> Self {
        let last_modified = if value.is_some() {
            SequenceNumber::ValidNonSequence
        } else {
            SequenceNumber::Invalid
        };
        Self {
            value,
            last_modified,
            default_value: None,
            has_new_network_data: false,
            equals: Some(equals),
            marshaler: PhantomData,
        }
    }

    /// Construct a present-valued handler. Shorthand for
    /// `ReplicatedFieldHandler::new(Some(value))`. Use for tests /
    /// fixtures; production code generally goes through
    /// [`set_value`](Self::set_value).
    pub const fn some(value: T) -> Self {
        Self::new(Some(value))
    }

    /// Borrow the current value, if any. Mirrors C++ `GetValue()` but
    /// returns `Option<&T>` instead of asserting `HasValue()` —
    /// callers express intent at the unwrap site.
    pub const fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// C++ `GetValue()`.
    ///
    /// # Panics
    ///
    /// Panics if [`Self::has_value`] is false — i.e. the field has never been
    /// assigned and carries no default, so there is no source-valid value to
    /// borrow. Use [`Self::value`] to get an `Option<&T>` instead.
    pub fn get_value(&self) -> &T
    where
        T: PartialEq + Clone,
    {
        assert!(
            self.has_value(),
            "GetValue() requires the caller to verify HasValue()"
        );
        self.value.as_ref().expect("source-valid value")
    }

    /// `IsFieldValid()` — `m_lastModified.IsValid()`.
    pub const fn is_field_valid(&self) -> bool {
        self.last_modified.is_valid()
    }

    /// `m_lastModified` — sequence the field was last set at.
    pub const fn last_modified(&self) -> SequenceNumber {
        self.last_modified
    }

    /// Override `m_lastModified` directly. Used by replication
    /// machinery (e.g. inside `merge_and_update_sequence`) to stamp the
    /// merged value with the merge sequence; production `set_value`
    /// already does this for the local case.
    pub const fn set_last_modified(&mut self, seq: SequenceNumber) {
        self.last_modified = seq;
    }

    /// `m_hasNewNetworkData` — true after a network read / merge until cleared.
    pub const fn has_new_network_data(&self) -> bool {
        self.has_new_network_data
    }

    /// Clear `m_hasNewNetworkData`. The replication driver calls this
    /// after a successful marshal pass so subsequent passes don't
    /// re-emit the same value.
    pub const fn reset_has_new_network_data(&mut self) {
        self.has_new_network_data = false;
    }

    /// Clear the current value and reset source-valid/network state.
    pub fn clear_value(&mut self) {
        self.value = None;
        self.last_modified = SequenceNumber::Invalid;
        self.has_new_network_data = false;
    }

    /// `IsDirty(baseline)` — "did this field change since `baseline`?"
    ///
    /// Source dirty state is driven by `m_lastModified`.
    pub fn is_dirty_since(&self, baseline: SequenceNumber) -> bool {
        self.last_modified.is_valid() && baseline < self.last_modified
    }

    /// Existing replicated-state bitmask signal — `true` iff a value is
    /// present (matches the historical "send when Some" wire
    /// behaviour). Distinct from [`is_dirty_since`](Self::is_dirty_since)
    /// which gates on a baseline sequence.
    pub const fn is_dirty(&self) -> bool {
        self.value.is_some()
    }

    /// Borrow the default value, if any.
    pub const fn default_value(&self) -> Option<&T> {
        self.default_value.as_ref()
    }

    /// Current source-style equality policy, if one was installed.
    pub fn equals_policy(&self) -> Option<ReplicatedFieldEquals<T>> {
        self.equals
    }

    /// Replace the source-style equality policy.
    pub fn set_equals_policy(&mut self, equals: ReplicatedFieldEquals<T>) {
        self.equals = Some(equals);
    }

    /// `SetDefaultValue(newValue)` — assign the initial default. Only
    /// meaningful before any `set_value` call; subsequent
    /// `set_default_value` invocations are silently ignored to match
    /// C++ `AZ_Assert(!m_lastModified.IsValid(), …)`.
    pub fn set_default_value(&mut self, default: T)
    where
        T: Clone,
    {
        if !self.last_modified.is_valid() {
            self.value = Some(default.clone());
            self.default_value = Some(default);
        }
    }
}

impl<T: Default + PartialEq + Clone, M: Codec<T>> ReplicatedFieldHandler<T, M> {
    fn values_equal(&self, left: &T, right: &T) -> bool {
        self.equals.unwrap_or(default_replicated_field_equals::<T>)(left, right)
    }

    /// Field contains a valid value or a default value. Mirrors C++
    /// `HasValue() == IsFieldValid() || IsDefaultValue()`.
    pub fn has_value(&self) -> bool {
        self.is_field_valid() || self.is_default_value()
    }

    /// `SetValue(newValue)` — assigns the value and updates local dirty
    /// replication bookkeeping:
    /// - If `newValue == default_value`, store it but mark
    ///   `last_modified = INVALID` (replication suppressed).
    /// - Otherwise mark `last_modified = VALID_NON_SEQUENCE`.
    ///
    /// This intentionally does **not** set `has_new_network_data`; native
    /// C++ only sets that flag from `Unmarshal` / `MergeAndUpdateSequence`.
    pub fn set_value(&mut self, new_value: T) {
        if matches!(self.default_value.as_ref(), Some(d) if self.values_equal(d, &new_value)) {
            // Equals default — keep the value but suppress replication.
            self.value = Some(new_value);
            self.last_modified = SequenceNumber::Invalid;
        } else {
            self.value = Some(new_value);
            self.last_modified = SequenceNumber::ValidNonSequence;
        }
    }

    /// Assign or clear the value while preserving source field invariants.
    pub fn set_optional_value(&mut self, new_value: Option<T>) {
        match new_value {
            Some(value) => self.set_value(value),
            None => self.clear_value(),
        }
    }

    /// `Access(cb)` — give the caller `&mut T` and update the
    /// replication bookkeeping if the callback returns `true`. Used
    /// when mutating a non-trivially-cloned value in place (vectors,
    /// strings, maps) so the C++ avoids copying.
    pub fn access<F>(&mut self, cb: F)
    where
        F: FnOnce(&mut T) -> bool,
    {
        let value = self.value.get_or_insert_with(Default::default);
        if cb(value) {
            let value = value.clone();
            self.set_value(value);
        }
    }

    /// `IsDefaultValue()` — `true` iff a default was set and `value`
    /// equals it.
    pub fn is_default_value(&self) -> bool {
        match (self.default_value.as_ref(), self.value.as_ref()) {
            (Some(d), Some(v)) => self.values_equal(d, v),
            _ => false,
        }
    }

    /// `IsFieldEqual(rhs)` — `has_value && value == rhs`. Useful for
    /// "only set the replicated value if it differs from the
    /// component-side value" patterns from the C++ docstring.
    pub fn is_field_equal(&self, rhs: &T) -> bool {
        self.has_value() && matches!(self.value.as_ref(), Some(v) if self.values_equal(v, rhs))
    }

    /// Hub-to-hub default propagation helper. Mirrors native
    /// `SetCurrentValueAsDefault`: keep the current value as the default
    /// and mark the field invalid so ordinary client replication suppresses
    /// it until it changes away from that default.
    pub fn set_current_value_as_default(&mut self) {
        let value = self.value.clone().unwrap_or_default();
        self.value = Some(value.clone());
        self.default_value = Some(value);
        self.last_modified = SequenceNumber::Invalid;
    }

    /// Rust port of `MB::ReplicatedFieldHandler::MergeAndUpdateSequence`.
    ///
    /// The caller supplies the previous merged value (`old_value`), the new
    /// incoming value, the sequence stamp to apply when a real change is
    /// detected, and the native `inheritPreviousNetworkDataStatus` flag.
    /// The return value is `detectedNewData`.
    pub fn merge_and_update_sequence(
        &mut self,
        old_value: &Self,
        new_value: &Self,
        seq: SequenceNumber,
        inherit_previous_network_data_status: bool,
    ) -> bool {
        let old_seq = old_value.last_modified();
        self.has_new_network_data = false;
        let mut detected_new_data = true;

        if std::ptr::eq(old_value, new_value) {
            if new_value.last_modified.is_valid() {
                self.last_modified = seq;
                self.value.clone_from(&new_value.value);
                self.has_new_network_data = true;
            } else if new_value.default_value.is_some() {
                self.value.clone_from(&new_value.value);
                self.default_value.clone_from(&new_value.default_value);
            }
            return detected_new_data;
        }

        self.default_value = old_value
            .default_value
            .clone()
            .or_else(|| new_value.default_value.clone());

        if new_value.is_default_value() {
            let old_effective = old_value.effective_value();
            let new_effective = new_value.effective_value();
            let old_equals_new = self.values_equal(&new_effective, &old_effective);

            if !old_value.last_modified.is_valid() && old_equals_new {
                self.last_modified = SequenceNumber::Invalid;
                if inherit_previous_network_data_status {
                    self.has_new_network_data = old_value.has_new_network_data();
                }
                detected_new_data = false;
            } else if old_equals_new {
                self.last_modified = old_seq;
                self.has_new_network_data = old_value.has_new_network_data();
                detected_new_data = false;
            } else {
                self.last_modified = seq;
                self.has_new_network_data = true;
            }

            self.value = Some(new_effective);
            return detected_new_data;
        }

        if new_value.last_modified.is_valid() {
            let old_effective = old_value.effective_value();
            let new_effective = new_value.effective_value();
            if old_seq.is_valid() && self.values_equal(&new_effective, &old_effective) {
                self.last_modified = old_seq;
                self.value.clone_from(&old_value.value);
                if inherit_previous_network_data_status {
                    self.has_new_network_data = old_value.has_new_network_data();
                }
                detected_new_data = false;
            } else {
                self.last_modified = seq;
                self.value.clone_from(&new_value.value);
                self.has_new_network_data = true;
            }
        } else {
            self.last_modified = old_seq;
            self.value.clone_from(&old_value.value);
            if inherit_previous_network_data_status {
                self.has_new_network_data = old_value.has_new_network_data();
            }
            detected_new_data = false;
        }

        detected_new_data
    }

    fn effective_value(&self) -> T {
        self.value.clone().unwrap_or_default()
    }
}

impl<T: Default + PartialEq + Clone, M: Codec<T>> PartialEq for ReplicatedFieldHandler<T, M> {
    fn eq(&self, rhs: &Self) -> bool {
        self.has_value() == rhs.has_value()
            && (!self.has_value() || self.value.as_ref() == rhs.value.as_ref())
    }
}

impl<T: Default + Eq + Clone, M: Codec<T>> Eq for ReplicatedFieldHandler<T, M> {}

impl<T: Default + PartialEq + Clone + 'static, M: Codec<T> + 'static> ReplicatedFieldHandlerBase
    for ReplicatedFieldHandler<T, M>
{
    fn is_default_value(&self) -> bool {
        Self::is_default_value(self)
    }

    fn set_current_value_as_default(&mut self) {
        Self::set_current_value_as_default(self);
    }

    fn is_dirty(&self, baseline: SequenceNumber) -> bool {
        self.is_dirty_since(baseline)
    }

    fn has_value(&self) -> bool {
        Self::has_value(self)
    }

    fn marshal_field(&self, wb: &mut WriteBuffer) {
        self.marshal(wb);
    }

    fn unmarshal_field(&mut self, rb: &mut ReadBuffer) -> Result<(), MarshalerError> {
        *self = Self::unmarshal(rb)?;
        Ok(())
    }

    fn merge_and_update_sequence(
        &mut self,
        old_value: &dyn ReplicatedFieldHandlerBase,
        new_value: &mut dyn ReplicatedFieldHandlerBase,
        seq: SequenceNumber,
        inherit_previous_network_data_status: bool,
    ) -> bool {
        let Some(old_value) = <dyn Any>::downcast_ref::<Self>(old_value) else {
            debug_assert!(false, "old replicated field handler type mismatch");
            return false;
        };
        let Some(new_value) = <dyn Any>::downcast_mut::<Self>(new_value) else {
            debug_assert!(false, "new replicated field handler type mismatch");
            return false;
        };
        Self::merge_and_update_sequence(
            self,
            old_value,
            &*new_value,
            seq,
            inherit_previous_network_data_status,
        )
    }

    fn last_modified(&self) -> SequenceNumber {
        self.last_modified()
    }

    fn set_last_modified(&mut self, seq: SequenceNumber) {
        Self::set_last_modified(self, seq);
    }

    fn reset_has_new_network_data(&mut self) {
        Self::reset_has_new_network_data(self);
    }

    fn has_new_network_data(&self) -> bool {
        self.has_new_network_data()
    }
}

impl<T: Default, M: Codec<T>> Marshaler for ReplicatedFieldHandler<T, M> {
    fn marshal(&self, wb: &mut WriteBuffer) {
        if let Some(ref v) = self.value {
            M::marshal(v, wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self {
            value: Some(M::unmarshal(rb)?),
            last_modified: SequenceNumber::ValidNonSequence,
            default_value: None,
            has_new_network_data: true,
            equals: None,
            marshaler: PhantomData,
        })
    }
}

// ---------------------------------------------------------------------------
// Per-field marshaler policies
// ---------------------------------------------------------------------------
//
// The shape — a zero-sized struct that implements `Marshaler<T>` for some
// underlying field type `T` — was already established by `DeltaMarshaler`
// (this file), `Vec3CompMarshaler` / `QuatCompNormMarshaler`
// (`compression_marshal.rs`), `HalfF32` (`utility_marshal.rs`), and
// `VlqU32Marshaler` (`vlq.rs`). The two policies below extend the catalogue
// to the remaining wire shapes that previously lived as bespoke
// `XxxField` structs duplicating `ReplicatedFieldHandler`.
//
// Together with [`HalfF32`](super::utility_marshal::HalfF32) (f16 BE) and
// [`VlqU32Marshaler`](super::vlq::VlqU32Marshaler) (VLQ u32) they cover
// every replicated-field wire shape that doesn't already have a self-marshaling
// `T`.

/// `f32` as f16 BE — half-precision scalar wire form.
///
/// Use as `ReplicatedFieldHandler<f32, HalfF32Marshaler>`.
/// The wrapper struct [`HalfF32`](super::utility_marshal::HalfF32) covers the
/// `#[marshal(as = "HalfF32")]` field-attribute case; this policy covers the
/// `ReplicatedFieldHandler` slot.
#[derive(Debug, Clone, Copy, Default)]
pub struct HalfF32Marshaler;

impl Codec<f32> for HalfF32Marshaler {
    fn marshal(value: &f32, wb: &mut WriteBuffer) {
        wb.write_u16(half::f16::from_f32(*value).to_bits());
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<f32, MarshalerError> {
        let bits = rb.read_u16()?;
        Ok(half::f16::from_bits(bits).to_f32())
    }
}

/// `Vec3` / `(f32, f32, f32)` as 3 × f16 BE — used for velocity, offset
/// vectors, magnet root offsets, and similar half-precision quantities.
///
/// Prefer `ReplicatedFieldHandler<Vec3, HalfVec3Marshaler>`. The tuple
/// `Codec` remains for transitional call sites.
#[derive(Debug, Clone, Copy, Default)]
pub struct HalfVec3Marshaler;

impl Codec<Vec3> for HalfVec3Marshaler {
    fn marshal(value: &Vec3, wb: &mut WriteBuffer) {
        wb.write_u16(half::f16::from_f32(value.x).to_bits());
        wb.write_u16(half::f16::from_f32(value.y).to_bits());
        wb.write_u16(half::f16::from_f32(value.z).to_bits());
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Vec3, MarshalerError> {
        let x = half::f16::from_bits(rb.read_u16()?).to_f32();
        let y = half::f16::from_bits(rb.read_u16()?).to_f32();
        let z = half::f16::from_bits(rb.read_u16()?).to_f32();
        Ok(Vec3::new(x, y, z))
    }
}

impl Codec<(f32, f32, f32)> for HalfVec3Marshaler {
    fn marshal(value: &(f32, f32, f32), wb: &mut WriteBuffer) {
        <Self as Codec<Vec3>>::marshal(&Vec3::new(value.0, value.1, value.2), wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<(f32, f32, f32), MarshalerError> {
        let value = <Self as Codec<Vec3>>::unmarshal(rb)?;
        Ok((value.x, value.y, value.z))
    }
}

/// World-position layout: `f32 BE (x) + f32 BE (y) + f16 BE (height)`.
///
/// Public value is `Vec3 { x, y, z: height }` in native horizontal axes
/// (not Bevy Y-up). Prefer
/// `ReplicatedFieldHandler<Vec3, PositionAnchorMarshaler>`. The tuple
/// `Codec` remains for transitional call sites.
#[derive(Debug, Clone, Copy, Default)]
pub struct PositionAnchorMarshaler;

impl Codec<Vec3> for PositionAnchorMarshaler {
    fn marshal(value: &Vec3, wb: &mut WriteBuffer) {
        value.x.marshal(wb);
        value.y.marshal(wb);
        wb.write_u16(half::f16::from_f32(value.z).to_bits());
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Vec3, MarshalerError> {
        let x = f32::unmarshal(rb)?;
        let y = f32::unmarshal(rb)?;
        let h = half::f16::from_bits(rb.read_u16()?).to_f32();
        Ok(Vec3::new(x, y, h))
    }
}

impl Codec<(f32, f32, f32)> for PositionAnchorMarshaler {
    fn marshal(value: &(f32, f32, f32), wb: &mut WriteBuffer) {
        <Self as Codec<Vec3>>::marshal(&Vec3::new(value.0, value.1, value.2), wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<(f32, f32, f32), MarshalerError> {
        let value = <Self as Codec<Vec3>>::unmarshal(rb)?;
        Ok((value.x, value.y, value.z))
    }
}

// ---------------------------------------------------------------------------
// Per-field marshaler policies (continued)
// ---------------------------------------------------------------------------

/// `MB::IntegerOmitLowerByteMarshaller<u16>` — wire codec for `u16` that
/// **emits the high byte only** and reads it back
/// shifted left into the high half (low byte stays zero on
/// unmarshal).
///
/// Used as the absolute-portion codec for
/// [`DeltaCompressedCounterHandler<u16>`], where the lower byte is
/// reconstructed entirely from the delta-portion's signed-modulo-256
/// counter rather than carried separately.
///
/// Currently specialized for `u16` (the only instantiation the
/// vendor code ships); generalize to other widths if a future
/// `MB::DeltaCompressedCounterHandler<u32>` shows up in a capture.
#[derive(Debug, Clone, Copy, Default)]
pub struct IntegerOmitLowerByteMarshaler;

/// Source `GetBitsPerByte()`.
pub const BITS_PER_BYTE: u32 = 8;

impl Codec<u16> for IntegerOmitLowerByteMarshaler {
    fn marshal(value: &u16, wb: &mut WriteBuffer) {
        wb.write_u8((*value >> BITS_PER_BYTE) as u8);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<u16, MarshalerError> {
        let byte = rb.read_u8()?;
        Ok(u16::from(byte) << BITS_PER_BYTE)
    }
}

/// `MB::DeltaIntegerMarshaller<T>` — an integer carried as one raw byte.
///
/// Truncating cast on send, zero-extending on receive. Used as the
/// relative-portion codec for [`DeltaCompressedCounterHandler<u16>`].
///
/// Functionally equivalent to writing a `u8` directly; named
/// separately so call-sites that need the C++ semantic
/// ("relative-portion of a delta counter") can pick the right
/// policy at a glance.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeltaIntegerMarshaler;

impl Codec<u16> for DeltaIntegerMarshaler {
    fn marshal(value: &u16, wb: &mut WriteBuffer) {
        // Truncation to the low byte is this codec's contract; the mask makes
        // it explicit instead of implicit in the cast.
        wb.write_u8((*value & 0xff) as u8);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<u16, MarshalerError> {
        Ok(u16::from(rb.read_u8()?))
    }
}

impl Codec<u32> for DeltaIntegerMarshaler {
    fn marshal(value: &u32, wb: &mut WriteBuffer) {
        // Truncation to the low byte is this codec's contract; the mask makes
        // it explicit instead of implicit in the cast.
        wb.write_u8((*value & 0xff) as u8);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<u32, MarshalerError> {
        Ok(u32::from(rb.read_u8()?))
    }
}

/// Source `GetValuesInByte()`.
pub const VALUES_IN_BYTE: u16 = 255;

fn quantize_delta<const DELTA_RANGE: u32>(value: f32) -> u8 {
    let delta_range = range_f32_u32(DELTA_RANGE);
    let quantized = (value + delta_range) * 255.0 / (2.0 * delta_range);
    quantized_u8(quantized.clamp(0.0, 255.0))
}

fn unquantize_delta<const DELTA_RANGE: u32>(quantized: u8) -> f32 {
    let delta_range = range_f32_u32(DELTA_RANGE);
    2.0 * delta_range * f32::from(quantized) / 255.0 - delta_range
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DeltaMarshaler<const DELTA_RANGE: u32, T>(PhantomData<fn() -> T>);

impl<const DELTA_RANGE: u32> DeltaMarshaler<DELTA_RANGE, f32> {
    /// Source `MB::Helper::GetQuantized<DeltaRange>` for float deltas.
    #[must_use]
    pub fn quantized(value: f32) -> u8 {
        quantize_delta::<DELTA_RANGE>(value)
    }

    /// Source `MB::Helper::GetUnquantized<DeltaRange>` for float deltas.
    #[must_use]
    pub fn unquantized(quantized: u8) -> f32 {
        unquantize_delta::<DELTA_RANGE>(quantized)
    }
}

impl<const DELTA_RANGE: u32> DeltaMarshaler<DELTA_RANGE, Vec3> {
    /// Source `MB::Helper::GetQuantized<DeltaRange>` for vector delta components.
    #[must_use]
    pub fn quantized_component(value: f32) -> u8 {
        quantize_delta::<DELTA_RANGE>(value)
    }

    /// Source `MB::Helper::GetUnquantized<DeltaRange>` for vector delta components.
    #[must_use]
    pub fn unquantized_component(quantized: u8) -> f32 {
        unquantize_delta::<DELTA_RANGE>(quantized)
    }
}

impl<const DELTA_RANGE: u32> Codec<f32> for DeltaMarshaler<DELTA_RANGE, f32> {
    fn marshal(value: &f32, wb: &mut WriteBuffer) {
        wb.write_u8(quantize_delta::<DELTA_RANGE>(*value));
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<f32, MarshalerError> {
        Ok(unquantize_delta::<DELTA_RANGE>(rb.read_u8()?))
    }
}

impl<const DELTA_RANGE: u32> Codec<Vec3> for DeltaMarshaler<DELTA_RANGE, Vec3> {
    fn marshal(value: &Vec3, wb: &mut WriteBuffer) {
        wb.write_u8(quantize_delta::<DELTA_RANGE>(value.x));
        wb.write_u8(quantize_delta::<DELTA_RANGE>(value.y));
        wb.write_u8(quantize_delta::<DELTA_RANGE>(value.z));
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Vec3, MarshalerError> {
        Ok(Vec3::new(
            unquantize_delta::<DELTA_RANGE>(rb.read_u8()?),
            unquantize_delta::<DELTA_RANGE>(rb.read_u8()?),
            unquantize_delta::<DELTA_RANGE>(rb.read_u8()?),
        ))
    }
}

pub trait DeltaRangeValue:
    Copy + Default + PartialEq + std::ops::Add<Output = Self> + std::ops::Sub<Output = Self>
{
    fn is_within_delta(base: Self, other: Self, delta_range: u32) -> bool;
}

impl DeltaRangeValue for f32 {
    fn is_within_delta(base: Self, other: Self, delta_range: u32) -> bool {
        (base - other).abs() < range_f32_u32(delta_range)
    }
}

impl DeltaRangeValue for Vec3 {
    fn is_within_delta(base: Self, other: Self, delta_range: u32) -> bool {
        let abs_diff = (base - other).abs();
        let delta_range = range_f32_u32(delta_range);
        abs_diff.x < delta_range && abs_diff.y < delta_range && abs_diff.z < delta_range
    }
}

/// Rust port of Lumberyard's `GridMate::DeltaCompressedReplicatedFieldHandler`.
///
/// This preserves the wire-level absolute/relative split and set logic. The
/// original callback/update-time layer (`BindInterface`, dispatch overrides,
/// `GetLastUpdateTime`) is intentionally left out because the current Rust
/// field model does not track per-field network timestamps yet.
///
/// The combined value is **always** computed on demand from the two
/// portions — there is no cached field. Mutate parts through their
/// `ReplicatedFieldHandler` methods so source sequence/default invariants
/// stay local to each slot.
#[derive(Debug, Clone)]
pub struct DeltaCompressedReplicatedFieldHandler<
    T: DeltaRangeValue,
    const DELTA_RANGE: u32,
    AbsoluteM: Codec<T> = DefaultMarshaler<T>,
    RelativeM: Codec<T> = DeltaMarshaler<DELTA_RANGE, T>,
> {
    pub absolute_portion: ReplicatedFieldHandler<T, AbsoluteM>,
    pub relative_portion: ReplicatedFieldHandler<T, RelativeM>,
}

impl<T: DeltaRangeValue, const DELTA_RANGE: u32, AbsoluteM: Codec<T>, RelativeM: Codec<T>> Default
    for DeltaCompressedReplicatedFieldHandler<T, DELTA_RANGE, AbsoluteM, RelativeM>
{
    fn default() -> Self {
        Self {
            absolute_portion: ReplicatedFieldHandler::default(),
            relative_portion: ReplicatedFieldHandler::default(),
        }
    }
}

impl<T: DeltaRangeValue, const DELTA_RANGE: u32, AbsoluteM: Codec<T>, RelativeM: Codec<T>>
    DeltaCompressedReplicatedFieldHandler<T, DELTA_RANGE, AbsoluteM, RelativeM>
{
    pub fn new(initial: T) -> Self {
        Self {
            absolute_portion: ReplicatedFieldHandler::some(initial),
            relative_portion: ReplicatedFieldHandler::some(T::default()),
        }
    }

    /// `IsAbsoluteValid()`.
    pub const fn is_absolute_valid(&self) -> bool {
        self.absolute_portion.is_field_valid()
    }

    /// `IsRelativeValid()`.
    pub const fn is_relative_valid(&self) -> bool {
        self.relative_portion.is_field_valid()
    }

    /// `IsFieldValid()`.
    pub const fn is_field_valid(&self) -> bool {
        self.is_absolute_valid()
    }

    /// Set the combined value, recomputing the absolute/relative split
    /// against the current absolute portion.
    pub fn set_value(&mut self, value: T) {
        if !self.is_absolute_valid() {
            self.absolute_portion.set_value(value);
            self.relative_portion.set_value(T::default());
            return;
        }

        let absolute = self.absolute_portion.value.unwrap_or_default();
        if T::is_within_delta(absolute, value, DELTA_RANGE) {
            self.relative_portion.set_value(value - absolute);
        } else {
            self.absolute_portion.set_value(value);
            self.relative_portion.set_value(T::default());
        }
    }

    /// Back-compat alias for the earlier Rust name.
    pub fn set(&mut self, value: T) {
        self.set_value(value);
    }

    /// Combined value — `absolute + relative`, computed on demand from
    /// whatever the parts currently hold. Returns `T::default()` when both
    /// portions are absent.
    pub fn get_value(&self) -> T {
        self.absolute_portion.value.unwrap_or_default()
            + self.relative_portion.value.unwrap_or_default()
    }

    /// Back-compat alias for the earlier Rust name.
    pub fn get(&self) -> T {
        self.get_value()
    }
}

/// `MB::DeltaCompressedCounterHandler<T>` — final concrete delta-compressed
/// counter for integer types.
///
/// Wire shape: two `ReplicatedFieldHandler<T>` slots — absolute
/// (high byte, via [`IntegerOmitLowerByteMarshaler`]) plus relative
/// (low byte cast, via [`DeltaIntegerMarshaler`]). Reconstructed
/// value is `absolute + relative` with modulo-256 wrap on the
/// relative portion.
///
/// `SetValue` semantics:
/// - If `IsWithinDelta(newValue)` (i.e. `0 <= newValue - absolute <= 255`):
///   only the relative portion changes.
/// - Otherwise re-anchor: relative = `newValue % 256`,
///   absolute = `newValue - relative`.
///
/// Used by `m_stateId` / `m_timeOffsetMilliseconds` etc. in
/// `MB::ALCReplicatedState`; Rust stores the source-shaped handler and
/// keeps the two wire portions inside it.
#[derive(Debug, Clone, Default)]
pub struct DeltaCompressedCounterHandler {
    pub absolute_portion: ReplicatedFieldHandler<u16, IntegerOmitLowerByteMarshaler>,
    pub relative_portion: ReplicatedFieldHandler<u16, DeltaIntegerMarshaler>,
}

impl DeltaCompressedCounterHandler {
    /// `IsAbsoluteValid()` — the absolute slot has been set at least once.
    #[must_use]
    pub const fn is_absolute_valid(&self) -> bool {
        self.absolute_portion.is_field_valid()
    }

    /// `IsWithinDelta(newValue)` — `newValue` lies in
    /// `[absolute, absolute + 255]`.
    #[must_use]
    pub fn is_within_delta(&self, new_value: u16) -> bool {
        let abs = self.absolute_portion.value.unwrap_or(0);
        abs <= new_value && (new_value - abs) <= VALUES_IN_BYTE
    }

    /// `SetValue(newValue)` — assigns the combined value, re-anchoring
    /// the absolute portion when the new value would exceed the
    /// relative range or no absolute has been set yet.
    pub fn set_value(&mut self, new_value: u16) {
        if !self.is_absolute_valid() || !self.is_within_delta(new_value) {
            let rel = new_value % (VALUES_IN_BYTE + 1);
            self.absolute_portion.set_value(new_value - rel);
            self.relative_portion.set_value(rel);
        } else {
            let abs = self.absolute_portion.value.unwrap_or(0);
            self.relative_portion.set_value(new_value - abs);
        }
    }

    /// Reconstructed combined value. Returns `0` when no absolute has
    /// been set; mirrors the C++ "absolute invalid" path's warning +
    /// default-construct return.
    #[must_use]
    pub fn value(&self) -> u16 {
        self.absolute_portion.value.unwrap_or(0) + self.relative_portion.value.unwrap_or(0)
    }

    /// `GetValue()`.
    #[must_use]
    pub fn get_value(&self) -> u16 {
        self.value()
    }
}

/// `MB::QuantizedRelativePosition` — 3-byte sentinel-encoded delta vector.
///
/// The `[255, 255, 255]` sentinel indicates "no relative
/// portion present" (i.e. the absolute portion alone carries the value);
/// any other triple is read as three quantized `u8` axis deltas.
///
/// Used by [`DynamicDeltaReplicatedFieldHandler`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuantizedRelativePosition {
    pub quantized_values: [u8; 3],
}

impl QuantizedRelativePosition {
    pub const ZERO_SENTINEL: Self = Self {
        quantized_values: [255, 255, 255],
    };

    #[must_use]
    pub const fn new(quantized_values: [u8; 3]) -> Self {
        Self { quantized_values }
    }

    /// `IsZero()` — all three components equal the `255` sentinel,
    /// signaling "no relative portion." Default-constructed positions
    /// are zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.quantized_values == [255, 255, 255]
    }
}

impl Default for QuantizedRelativePosition {
    fn default() -> Self {
        Self::ZERO_SENTINEL
    }
}

impl From<[u8; 3]> for QuantizedRelativePosition {
    fn from(value: [u8; 3]) -> Self {
        Self::new(value)
    }
}

impl From<QuantizedRelativePosition> for [u8; 3] {
    fn from(value: QuantizedRelativePosition) -> Self {
        value.quantized_values
    }
}

/// Source `Helper::GetQuantizedWithRange`.
#[must_use]
pub fn get_quantized_with_range(value: f32, delta_range: f32) -> u8 {
    let quantized = (value + delta_range) * 255.0 / (2.0 * delta_range);
    quantized_u8(quantized.clamp(0.0, 255.0))
}

/// Source `Helper::GetUnquantizedWithRange`.
#[must_use]
pub fn get_unquantized_with_range(quantized: u8, delta_range: f32) -> f32 {
    2.0 * delta_range * f32::from(quantized) / 255.0 - delta_range
}

impl Marshaler for QuantizedRelativePosition {
    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_u8(self.quantized_values[0]);
        wb.write_u8(self.quantized_values[1]);
        wb.write_u8(self.quantized_values[2]);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self {
            quantized_values: [rb.read_u8()?, rb.read_u8()?, rb.read_u8()?],
        })
    }
}

/// `MB::DynamicDeltaReplicatedFieldHandler<M>` — a delta-compressed `Vec3`
/// with a **runtime-supplied** quantization scale.
///
/// Wire shape: three slots —
/// - `absolute_portion: ReplicatedFieldHandler<Vec3, M>` (default
///   `M = Vec3`, three raw f32 components);
/// - `quantized_relative_portion: ReplicatedFieldHandler<QuantizedRelativePosition>`
///   (3 bytes, see [`QuantizedRelativePosition`]);
/// - `quantization: ReplicatedFieldHandler<f32>` (single f32 scale).
///
/// Used by `m_worldPos` in `MB::ALCReplicatedState` —
/// `PackedPositionMarshaller<MIN_H, MAX_H>` plays the role of `M`
/// over there.
///
/// `set_value(new_value, quantization)`:
/// - If absolute is unset, or `quantization` differs, or the new
///   value falls outside the per-component `< quantization` range:
///   re-anchor `absolute = new_value`, relative goes to the sentinel,
///   quantization slot stores the new scale.
/// - Otherwise the relative portion stores the quantized delta and
///   the absolute / quantization slots stay put.
#[derive(Debug, Clone, Default)]
pub struct DynamicDeltaReplicatedFieldHandler<AbsoluteM: Codec<Vec3> = DefaultMarshaler<Vec3>> {
    pub absolute_portion: ReplicatedFieldHandler<Vec3, AbsoluteM>,
    pub quantized_relative_portion: ReplicatedFieldHandler<QuantizedRelativePosition>,
    pub quantization: ReplicatedFieldHandler<f32>,
}

impl<AbsoluteM: Codec<Vec3>> DynamicDeltaReplicatedFieldHandler<AbsoluteM> {
    const fn is_absolute_valid(&self) -> bool {
        self.absolute_portion.is_field_valid()
    }

    fn is_relative_zero(&self) -> bool {
        self.quantized_relative_portion
            .value
            .is_none_or(|v| v.is_zero())
    }

    fn is_within_delta(&self, new_value: Vec3, quantization: f32) -> bool {
        let abs = self.absolute_portion.value.unwrap_or(Vec3::ZERO);
        let abs_diff = (abs - new_value).abs();
        // Strict `<` matches C++: the limits 0 and 255 stay reserved
        // for the sentinel.
        abs_diff.x < quantization && abs_diff.y < quantization && abs_diff.z < quantization
    }

    fn quantize(value: f32, quantization: f32) -> u8 {
        get_quantized_with_range(value, quantization)
    }

    fn unquantize(q: u8, quantization: f32) -> f32 {
        get_unquantized_with_range(q, quantization)
    }

    /// Reconstructed combined value. When the relative portion is the
    /// sentinel (`is_zero()`) only the absolute is used.
    #[must_use]
    pub fn value(&self) -> Vec3 {
        let Some(abs) = self.absolute_portion.value else {
            return Vec3::ZERO;
        };
        if self.is_relative_zero() || !self.quantization.has_value() {
            return abs;
        }
        let quantization = self.quantization.value.unwrap_or(0.0);
        let rel = self
            .quantized_relative_portion
            .value
            .unwrap_or_default()
            .quantized_values;
        Vec3::new(
            abs.x + Self::unquantize(rel[0], quantization),
            abs.y + Self::unquantize(rel[1], quantization),
            abs.z + Self::unquantize(rel[2], quantization),
        )
    }

    /// `GetValue()`.
    #[must_use]
    pub fn get_value(&self) -> Vec3 {
        self.value()
    }

    /// `IsFieldValid()`.
    #[must_use]
    pub const fn is_field_valid(&self) -> bool {
        self.is_absolute_valid()
    }

    pub fn set_value(&mut self, new_value: Vec3, quantization: f32) {
        let needs_anchor = !self.is_absolute_valid()
            || (self.quantization.value != Some(quantization))
            || !self.is_within_delta(new_value, quantization);

        if needs_anchor {
            self.absolute_portion.set_value(new_value);
            self.quantized_relative_portion
                .set_value(QuantizedRelativePosition::default());
            self.quantization.set_value(quantization);
            return;
        }

        let abs = self.absolute_portion.value.unwrap_or(Vec3::ZERO);
        let diff = new_value - abs;
        let quantized = QuantizedRelativePosition {
            quantized_values: [
                Self::quantize(diff.x, quantization),
                Self::quantize(diff.y, quantization),
                Self::quantize(diff.z, quantization),
            ],
        };
        self.quantized_relative_portion.set_value(quantized);
    }
}

/// A delta-encoded float timer split into four `ReplicatedFieldHandler<u8>` slots.
///
/// Ports `MB::FloatTimerDeltaReplicatedField<Quantization, RolloverThreshold,
/// MarshallerType, DeltaMarshallerType>`.
///
/// `set_value(seconds)`:
/// - Quantize: `q = (seconds % RolloverThreshold) / (1 / Quantization)`
///   stored as `u32`.
/// - Diff against the previous quantized value and only set the bytes
///   that changed (LSB-first with fall-through, mirroring the C++
///   switch).
///
/// `value()` reassembles the four bytes into a u32 and reverses the
/// quantization.
///
/// Used by `m_scopeTimeBlobs[N]` in `MB::ALCReplicatedState` (each
/// blob contributes 4 wire slots). Our `AlcReplicatedState` currently
/// carries the four bytes as flat `ReplicatedFieldHandler<u8>` fields;
/// this type lets future ports collapse them back into a single named
/// timer field if needed.
#[derive(Debug, Clone)]
pub struct FloatTimerDeltaReplicatedField<
    const QUANTIZATION: u32 = 30,
    const ROLLOVER_THRESHOLD: u32 = 65535,
> {
    pub data: [ReplicatedFieldHandler<u8>; 4],
}

impl<const QUANTIZATION: u32, const ROLLOVER_THRESHOLD: u32> Default
    for FloatTimerDeltaReplicatedField<QUANTIZATION, ROLLOVER_THRESHOLD>
{
    fn default() -> Self {
        Self {
            data: std::array::from_fn(|_| ReplicatedFieldHandler::default()),
        }
    }
}

impl<const QUANTIZATION: u32, const ROLLOVER_THRESHOLD: u32>
    FloatTimerDeltaReplicatedField<QUANTIZATION, ROLLOVER_THRESHOLD>
{
    const QUANTIZATION_RATIO: f32 = 1.0 / range_f32_u32(QUANTIZATION);

    const fn rollover_divisor() -> u32 {
        if ROLLOVER_THRESHOLD > 0 {
            ROLLOVER_THRESHOLD
        } else {
            1
        }
    }

    fn quantize_value(value_in: f32) -> u32 {
        let divisor = Self::rollover_divisor();
        let divisions = quantized_u32(value_in) / divisor;
        let clamped_time =
            range_f32_u32(divisions).mul_add(-range_f32_u32(ROLLOVER_THRESHOLD), value_in);
        quantized_u32(clamped_time / Self::QUANTIZATION_RATIO)
    }

    fn normalize_quantized_value(q: u32) -> f32 {
        range_f32_u32(q) * Self::QUANTIZATION_RATIO
    }

    fn raw_quantized_value(&self) -> u32 {
        let b0 = u32::from(self.data[0].value.unwrap_or(0));
        let b1 = u32::from(self.data[1].value.unwrap_or(0));
        let b2 = u32::from(self.data[2].value.unwrap_or(0));
        let b3 = u32::from(self.data[3].value.unwrap_or(0));
        (b3 << 24) | (b2 << 16) | (b1 << 8) | b0
    }

    const fn bytes_required(new_value: u32, old_value: u32) -> usize {
        let diff = new_value ^ old_value;
        if (diff & 0xff00_0000) != 0 {
            4
        } else if (diff & 0xffff_0000) != 0 {
            3
        } else if (diff & 0x0000_ff00) != 0 {
            2
        } else {
            1
        }
    }

    /// `IsFieldValid()` — every byte slot has been set at least once.
    #[must_use]
    pub fn is_field_valid(&self) -> bool {
        self.data.iter().all(ReplicatedFieldHandler::is_field_valid)
    }

    #[must_use]
    pub fn value(&self) -> f32 {
        Self::normalize_quantized_value(self.raw_quantized_value())
    }

    pub fn set_value(&mut self, new_value: f32) {
        let quantized_new = Self::quantize_value(new_value);
        let bytes_needed = if self.is_field_valid() {
            let quantized_old = self.raw_quantized_value();
            Self::bytes_required(quantized_new, quantized_old)
        } else {
            4
        };

        // LSB-first with fall-through, mirroring the C++ switch.
        if bytes_needed >= 4 {
            self.data[3].set_value(((quantized_new >> 24) & 0xff) as u8);
        }
        if bytes_needed >= 3 {
            self.data[2].set_value(((quantized_new >> 16) & 0xff) as u8);
        }
        if bytes_needed >= 2 {
            self.data[1].set_value(((quantized_new >> 8) & 0xff) as u8);
        }
        self.data[0].set_value((quantized_new & 0xff) as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::buffer::CARRIER_ENDIAN;

    fn roundtrip_field<T, M>(value: T) -> T
    where
        T: Default,
        M: Codec<T>,
    {
        let field = ReplicatedFieldHandler::<T, M> {
            value: Some(value),
            ..Default::default()
        };

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        field.marshal(&mut wb);
        let bytes = wb.into_vec();

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let decoded = ReplicatedFieldHandler::<T, M>::unmarshal(&mut rb).expect("unmarshal");
        assert_eq!(rb.left(), 0, "all bytes consumed");
        decoded.value.expect("decoded value")
    }

    #[test]
    fn test_delta_marshaler_f32_endpoint_roundtrip() {
        let min = roundtrip_field::<f32, DeltaMarshaler<10, f32>>(-10.0);
        let max = roundtrip_field::<f32, DeltaMarshaler<10, f32>>(10.0);
        assert!((min + 10.0).abs() < f32::EPSILON);
        assert!((max - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_delta_marshaler_vec3_roundtrip() {
        let decoded = roundtrip_field::<Vec3, DeltaMarshaler<4, Vec3>>(Vec3::new(-4.0, 0.0, 4.0));
        assert!((decoded.x + 4.0).abs() < f32::EPSILON);
        assert!((decoded.y - (-0.015_686_035)).abs() < 0.0001);
        assert!((decoded.z - 4.0).abs() < f32::EPSILON);
    }

    #[test]
    fn replicated_field_set_value_is_local_dirty_not_network_data() {
        let mut field = ReplicatedFieldHandler::<u32>::default();
        field.set_value(42);

        assert_eq!(field.value, Some(42));
        assert_eq!(field.last_modified(), SequenceNumber::ValidNonSequence);
        assert!(field.is_dirty_since(SequenceNumber::Invalid));
        assert!(!field.has_new_network_data());
    }

    #[test]
    fn replicated_field_unmarshal_marks_new_network_data() {
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        42u32.marshal(&mut wb);
        let bytes = wb.into_vec();

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let field = ReplicatedFieldHandler::<u32>::unmarshal(&mut rb).expect("unmarshal");

        assert_eq!(field.value, Some(42));
        assert_eq!(field.last_modified(), SequenceNumber::ValidNonSequence);
        assert!(field.has_new_network_data());
    }

    #[test]
    fn replicated_field_merge_stamps_changed_value() {
        let mut old = ReplicatedFieldHandler::<u32>::default();
        old.set_value(1);
        old.set_last_modified(SequenceNumber::Seq(7));

        let mut new = ReplicatedFieldHandler::<u32>::default();
        new.set_value(2);

        let mut merged = ReplicatedFieldHandler::<u32>::default();
        let detected = merged.merge_and_update_sequence(&old, &new, SequenceNumber::Seq(9), false);

        assert!(detected);
        assert_eq!(merged.value, Some(2));
        assert_eq!(merged.last_modified(), SequenceNumber::Seq(9));
        assert!(merged.has_new_network_data());
    }

    #[test]
    fn replicated_field_merge_preserves_equal_old_sequence() {
        let mut old = ReplicatedFieldHandler::<u32>::default();
        old.set_value(1);
        old.set_last_modified(SequenceNumber::Seq(7));

        let mut new = ReplicatedFieldHandler::<u32>::default();
        new.set_value(1);

        let mut merged = ReplicatedFieldHandler::<u32>::default();
        let detected = merged.merge_and_update_sequence(&old, &new, SequenceNumber::Seq(9), false);

        assert!(!detected);
        assert_eq!(merged.value, Some(1));
        assert_eq!(merged.last_modified(), SequenceNumber::Seq(7));
        assert!(!merged.has_new_network_data());
    }

    #[test]
    // 10.0, 3.0 and 13.0 are all exactly representable and the sum is exact,
    // so this pins the reconstructed value rather than approximating it.
    #[allow(clippy::float_cmp)]
    fn test_delta_compressed_field_handler_set_within_delta() {
        let mut field = DeltaCompressedReplicatedFieldHandler::<f32, 5>::new(10.0);
        field.set(13.0);
        assert_eq!(field.absolute_portion.value, Some(10.0));
        assert_eq!(field.relative_portion.value, Some(3.0));
        assert_eq!(field.get(), 13.0);
        assert_eq!(
            field.relative_portion.last_modified(),
            SequenceNumber::ValidNonSequence
        );
        assert!(!field.relative_portion.has_new_network_data());
    }

    #[test]
    // 20.0 and 0.0 are exactly representable and the sum is exact, so this
    // pins the reconstructed value rather than approximating it.
    #[allow(clippy::float_cmp)]
    fn test_delta_compressed_field_handler_set_outside_delta() {
        let mut field = DeltaCompressedReplicatedFieldHandler::<f32, 5>::new(10.0);
        field.set(20.0);
        assert_eq!(field.absolute_portion.value, Some(20.0));
        assert_eq!(field.relative_portion.value, Some(0.0));
        assert_eq!(field.get(), 20.0);
        assert_eq!(
            field.absolute_portion.last_modified(),
            SequenceNumber::ValidNonSequence
        );
        assert_eq!(
            field.relative_portion.last_modified(),
            SequenceNumber::ValidNonSequence
        );
    }

    #[test]
    fn test_delta_compressed_field_handler_set_initializes_empty_absolute() {
        let mut field = DeltaCompressedReplicatedFieldHandler::<f32, 5>::default();
        field.set(13.0);

        assert_eq!(field.absolute_portion.value, Some(13.0));
        assert_eq!(field.relative_portion.value, Some(0.0));
        assert_eq!(
            field.absolute_portion.last_modified(),
            SequenceNumber::ValidNonSequence
        );
        assert_eq!(
            field.relative_portion.last_modified(),
            SequenceNumber::ValidNonSequence
        );
    }

    /// Confirms that mutating the parts directly is reflected by `get()`
    /// without any explicit "sync" step — the prior `combined_value`
    /// cache + `sync_from_parts` ceremony was the bug class this test
    /// used to verify; now the read-through structure makes the bug
    /// unrepresentable.
    #[test]
    fn test_delta_compressed_field_handler_get_reads_through_parts() {
        let mut field = DeltaCompressedReplicatedFieldHandler::<Vec3, 8>::default();
        field
            .absolute_portion
            .set_value(Vec3::new(100.0, 200.0, 300.0));
        field.relative_portion.set_value(Vec3::new(1.0, -2.0, 3.0));
        assert_eq!(field.get(), Vec3::new(101.0, 198.0, 303.0));
    }

    #[test]
    fn dynamic_delta_quantization_matches_source_range_mapping() {
        let mut handler = DynamicDeltaReplicatedFieldHandler::<DefaultMarshaler<Vec3>>::default();
        handler.set_value(Vec3::new(10.0, 20.0, 30.0), 4.0);

        handler.set_value(Vec3::new(8.0, 20.0, 32.0), 4.0);
        assert_eq!(
            handler.quantized_relative_portion.value.unwrap(),
            QuantizedRelativePosition {
                quantized_values: [63, 127, 191],
            }
        );

        let decoded = handler.value();
        assert!((decoded.x - 7.976_470_5).abs() < 0.001, "{decoded:?}");
        assert!((decoded.y - 19.984_314).abs() < 0.001, "{decoded:?}");
        assert!((decoded.z - 31.992_157).abs() < 0.001, "{decoded:?}");
    }
}
