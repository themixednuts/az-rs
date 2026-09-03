//! `Amazon::Hub::ReplicatedStateBundle` — native actor-fragment state stream.
//!
//! The top-level message is a small header plus `m_bundleBuffer`, a flat byte
//! stream of native `StateRecord`s:
//!
//! ```text
//! ReplicatedStateBundle:
//!   SequenceNumber seq
//!   u8             client_context_instance_id
//!   u8             bandwidth_mode
//!   bool           is_unreliable
//!   bool           includes_replication_control
//!   ReplicationControlData?  // only when the bool above is true
//!   VLQ32 bundle_buffer_len
//!   u8[bundle_buffer_len] bundle_buffer
//!
//! StateRecord:
//!   VLQ32 interest_id
//!   u8    fragment_count
//!   StateFragment[fragment_count]
//!
//! StateFragment:
//!   VLQ32 fragment_key
//!   StateFragmentTypeId
//!   IFragment::MarshalContents bytes
//! ```
//!
//! Fragment bodies are not length-prefixed. Readers must decode each body with
//! the resolved fragment descriptor before they can reach the next fragment.

use std::{cell::Cell, ops::AddAssign};

use arrayvec::ArrayVec;
use az_core::type_info::AzTypeInfo;
use uuid::Uuid;

use super::{
    BandwidthMode, ClientContextId, Fragment, FragmentKey, InterestId, MarshalContext,
    SequenceNumber,
};
use crate::az::{Class, TypeRegistry};
use crate::serialize::buffer::{CARRIER_ENDIAN, ReadBuffer, WriteBuffer};
use crate::serialize::error::MarshalerError;
use crate::serialize::marshaler::Marshaler;
use crate::serialize::vlq::VlqU32Marshaler;

pub const MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE: usize = 250 * 1024;
pub const MAX_REPLICATION_CONTROL_IDS: usize = 100;

/// `Amazon::Hub::ReplicationPerformanceData`.
#[derive(
    az_derive::AzTypeInfo,
    gridmate_derive::Marshaler,
    gridmate_derive::ClassDesc,
    gridmate_derive::Message,
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
)]
#[az_type_info(
    name = "Amazon::Hub::ReplicationPerformanceData",
    "64CEE0FB-B878-47B3-8377-B45A9C9BA884"
)]
#[class_desc(type_index = 18)]
#[message(server_to_client)]
pub struct ReplicationPerformanceData {
    pub collection_period_ms: u16,
    pub count_bundles: u32,
    pub count_throttled: u32,
    pub count_overflows: u32,
    pub count_characters_intended: u32,
    pub count_characters_actual: u32,
    pub characters_missed_due_to_rate_limiter: u32,
    pub objects_missed_due_to_rate_limiter: u32,
    pub characters_missed_due_to_overflow: u32,
    pub objects_missed_due_to_overflow: u32,
    pub characters_added_during_period: u32,
    pub objects_added_during_period: u32,
}

impl ReplicationPerformanceData {
    pub fn reset(&mut self) {
        let collection_period_ms = self.collection_period_ms;
        *self = Self {
            collection_period_ms,
            ..Self::default()
        };
    }

    pub fn add_counts(&mut self, rhs: &Self) {
        *self += rhs;
    }
}

impl AddAssign<&Self> for ReplicationPerformanceData {
    fn add_assign(&mut self, rhs: &Self) {
        self.collection_period_ms = rhs.collection_period_ms;
        self.count_bundles = self.count_bundles.wrapping_add(rhs.count_bundles);
        self.count_throttled = self.count_throttled.wrapping_add(rhs.count_throttled);
        self.count_overflows = self.count_overflows.wrapping_add(rhs.count_overflows);
        self.count_characters_intended = self
            .count_characters_intended
            .wrapping_add(rhs.count_characters_intended);
        self.count_characters_actual = self
            .count_characters_actual
            .wrapping_add(rhs.count_characters_actual);
        self.characters_missed_due_to_rate_limiter = self
            .characters_missed_due_to_rate_limiter
            .wrapping_add(rhs.characters_missed_due_to_rate_limiter);
        self.objects_missed_due_to_rate_limiter = self
            .objects_missed_due_to_rate_limiter
            .wrapping_add(rhs.objects_missed_due_to_rate_limiter);
        self.characters_missed_due_to_overflow = self
            .characters_missed_due_to_overflow
            .wrapping_add(rhs.characters_missed_due_to_overflow);
        self.objects_missed_due_to_overflow = self
            .objects_missed_due_to_overflow
            .wrapping_add(rhs.objects_missed_due_to_overflow);
        self.characters_added_during_period = self
            .characters_added_during_period
            .wrapping_add(rhs.characters_added_during_period);
        self.objects_added_during_period = self
            .objects_added_during_period
            .wrapping_add(rhs.objects_added_during_period);
    }
}

/// `Amazon::Hub::ReplicationControlData`.
///
/// Native stores one ID list plus `m_pauseStartIdx`: stop-replication ids first,
/// pause-replication ids after that split.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplicationControlData {
    pause_start_idx: u32,
    interest_ids: ArrayVec<InterestId, MAX_REPLICATION_CONTROL_IDS>,
}

impl ReplicationControlData {
    /// Build the native single-list layout from the two id groups, stop ids
    /// first.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::ContainerOverflow`] if the two groups together
    /// hold more than [`MAX_REPLICATION_CONTROL_IDS`] ids, which is the
    /// native list's fixed capacity.
    pub fn from_ids(
        stop_replication_ids: impl IntoIterator<Item = impl Into<InterestId>>,
        pause_replication_ids: impl IntoIterator<Item = impl Into<InterestId>>,
    ) -> Result<Self, MarshalerError> {
        let stop_replication_ids = stop_replication_ids
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        let pause_replication_ids = pause_replication_ids
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        let len = stop_replication_ids.len() + pause_replication_ids.len();
        if len > MAX_REPLICATION_CONTROL_IDS {
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: MAX_REPLICATION_CONTROL_IDS,
            });
        }

        // Unreachable after the capacity check above; `try_from` keeps the
        // split index exact rather than trusting that invariant with a cast.
        let pause_start_idx = u32::try_from(stop_replication_ids.len()).map_err(|_| {
            MarshalerError::ContainerOverflow {
                len,
                capacity: MAX_REPLICATION_CONTROL_IDS,
            }
        })?;

        let mut interest_ids = ArrayVec::new();
        interest_ids.extend(stop_replication_ids.iter().copied());
        interest_ids.extend(pause_replication_ids.iter().copied());
        Ok(Self {
            pause_start_idx,
            interest_ids,
        })
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.interest_ids.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.interest_ids.is_empty()
    }

    #[must_use]
    pub const fn pause_start_idx(&self) -> u32 {
        self.pause_start_idx
    }

    #[must_use]
    pub const fn stop_replication_count(&self) -> u32 {
        self.pause_start_idx
    }

    #[must_use]
    pub fn pause_replication_count(&self) -> u32 {
        // `interest_ids` is an `ArrayVec` of capacity
        // `MAX_REPLICATION_CONTROL_IDS` (100), so the count always fits `u32`.
        u32::try_from(self.pause_replication_ids().len()).unwrap_or(u32::MAX)
    }

    #[must_use]
    pub fn stop_replication_ids(&self) -> &[InterestId] {
        &self.interest_ids[..self.pause_start_idx as usize]
    }

    #[must_use]
    pub fn pause_replication_ids(&self) -> &[InterestId] {
        &self.interest_ids[self.pause_start_idx as usize..]
    }

    /// Append a stop-replication id, keeping it before the pause split.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::ContainerOverflow`] if the list already holds
    /// [`MAX_REPLICATION_CONTROL_IDS`] ids. Nothing is inserted in that case.
    pub fn push_stop_replication_id(
        &mut self,
        interest_id: impl Into<InterestId>,
    ) -> Result<(), MarshalerError> {
        if self.interest_ids.len() == MAX_REPLICATION_CONTROL_IDS {
            return Err(MarshalerError::ContainerOverflow {
                len: self.interest_ids.len() + 1,
                capacity: MAX_REPLICATION_CONTROL_IDS,
            });
        }
        self.interest_ids
            .insert(self.pause_start_idx as usize, interest_id.into());
        self.pause_start_idx += 1;
        Ok(())
    }

    /// Append a pause-replication id after the pause split.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::ContainerOverflow`] if the list already holds
    /// [`MAX_REPLICATION_CONTROL_IDS`] ids. Nothing is pushed in that case.
    pub fn push_pause_replication_id(
        &mut self,
        interest_id: impl Into<InterestId>,
    ) -> Result<(), MarshalerError> {
        if self.interest_ids.len() == MAX_REPLICATION_CONTROL_IDS {
            return Err(MarshalerError::ContainerOverflow {
                len: self.interest_ids.len() + 1,
                capacity: MAX_REPLICATION_CONTROL_IDS,
            });
        }
        self.interest_ids.push(interest_id.into());
        Ok(())
    }

    pub fn reset(&mut self) {
        self.interest_ids.clear();
        self.pause_start_idx = 0;
    }

    pub fn remove_pending_stop_id(&mut self, interest_id: impl Into<InterestId>) -> bool {
        let interest_id = interest_id.into();
        let Some(idx) = self
            .stop_replication_ids()
            .iter()
            .position(|id| *id == interest_id)
        else {
            return false;
        };
        self.interest_ids.remove(idx);
        self.pause_start_idx -= 1;
        true
    }
}

impl Marshaler for ReplicationControlData {
    fn marshal(&self, wb: &mut WriteBuffer) {
        debug_assert!(
            self.pause_start_idx as usize <= self.interest_ids.len(),
            "ReplicationControlData split must be within the native id list"
        );
        VlqU32Marshaler.marshal(wb, self.pause_start_idx);
        self.interest_ids.marshal(wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let pause_start_idx = VlqU32Marshaler.unmarshal(rb)?;
        let interest_ids = ArrayVec::<InterestId, MAX_REPLICATION_CONTROL_IDS>::unmarshal(rb)?;
        if pause_start_idx as usize > interest_ids.len() {
            return Err(MarshalerError::ContainerOverflow {
                len: pause_start_idx as usize,
                capacity: interest_ids.len(),
            });
        }

        Ok(Self {
            pause_start_idx,
            interest_ids,
        })
    }
}

/// Owned native `ReplicatedStateBundle` message used when constructing sends.
#[derive(
    az_derive::AzTypeInfo,
    gridmate_derive::ClassDesc,
    gridmate_derive::Message,
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
)]
#[az_type_info(
    name = "Amazon::Hub::ReplicatedStateBundle",
    "8A40AEC2-AE07-4F92-9BF3-78FC0CC94FDF"
)]
#[class_desc(type_index = 8)]
#[message(server_to_client)]
pub struct ReplicatedStateBundle {
    /// Native `SequenceNumber`, wire-framed as `bool has + optional VLQ-u64`.
    pub seq: SequenceNumber,
    pub client_context_instance_id: u8,
    pub bandwidth_mode: u8,
    pub is_unreliable: bool,
    pub replication_control: Option<ReplicationControlData>,
    pub bundle_buffer: Vec<u8>,
}

impl ReplicatedStateBundle {
    #[must_use]
    pub fn new(client_context_instance_id: u8, bandwidth_mode: u8) -> Self {
        Self {
            client_context_instance_id,
            bandwidth_mode,
            ..Self::default()
        }
    }

    pub fn with_seq(
        seq: impl Into<SequenceNumber>,
        client_context_instance_id: u8,
        bandwidth_mode: u8,
    ) -> Self {
        Self {
            seq: seq.into(),
            client_context_instance_id,
            bandwidth_mode,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_bundle_buffer(bundle_buffer: Vec<u8>) -> Self {
        Self {
            bundle_buffer,
            ..Self::default()
        }
    }

    #[must_use]
    pub const fn seq(&self) -> SequenceNumber {
        self.seq
    }

    pub fn set_seq(&mut self, seq: impl Into<SequenceNumber>) {
        self.seq = seq.into();
    }

    #[must_use]
    pub const fn client_context_instance_id(&self) -> u8 {
        self.client_context_instance_id
    }

    #[must_use]
    pub const fn client_context_id(&self) -> ClientContextId {
        ClientContextId::new(self.client_context_instance_id)
    }

    #[must_use]
    pub const fn bandwidth_mode(&self) -> u8 {
        self.bandwidth_mode
    }

    #[must_use]
    pub const fn bandwidth(&self) -> BandwidthMode {
        BandwidthMode::new(self.bandwidth_mode)
    }

    #[must_use]
    pub const fn is_unreliable(&self) -> bool {
        self.is_unreliable
    }

    pub const fn set_unreliable(&mut self) {
        self.is_unreliable = true;
    }

    #[must_use]
    pub const fn includes_replication_control(&self) -> bool {
        self.replication_control.is_some()
    }

    pub fn include_replication_control(&mut self) {
        self.ensure_replication_control();
    }

    #[must_use]
    pub const fn has_replication_control(&self) -> bool {
        self.replication_control.is_some()
    }

    pub fn replication_control_count(&self) -> usize {
        self.replication_control
            .as_ref()
            .map_or(0, ReplicationControlData::len)
    }

    #[must_use]
    pub const fn has_state_records_payload(&self) -> bool {
        !self.bundle_buffer.is_empty()
    }

    /// Record a stop-replication id on this bundle, creating the replication
    /// control block if the bundle has none yet.
    ///
    /// # Errors
    ///
    /// Returns any error
    /// [`ReplicationControlData::push_stop_replication_id`] returns —
    /// [`MarshalerError::ContainerOverflow`] once the list is full.
    pub fn add_stop_id(
        &mut self,
        interest_id: impl Into<InterestId>,
    ) -> Result<(), MarshalerError> {
        self.ensure_replication_control()
            .push_stop_replication_id(interest_id)
    }

    /// Record a pause-replication id on this bundle, creating the replication
    /// control block if the bundle has none yet.
    ///
    /// # Errors
    ///
    /// Returns any error
    /// [`ReplicationControlData::push_pause_replication_id`] returns —
    /// [`MarshalerError::ContainerOverflow`] once the list is full.
    pub fn add_pause_id(
        &mut self,
        interest_id: impl Into<InterestId>,
    ) -> Result<(), MarshalerError> {
        self.ensure_replication_control()
            .push_pause_replication_id(interest_id)
    }

    pub fn stop_replication_ids(&self) -> &[InterestId] {
        self.replication_control
            .as_ref()
            .map_or(&[], ReplicationControlData::stop_replication_ids)
    }

    pub fn pause_replication_ids(&self) -> &[InterestId] {
        self.replication_control
            .as_ref()
            .map_or(&[], ReplicationControlData::pause_replication_ids)
    }

    pub fn reset_replication_control(&mut self) {
        if let Some(replication_control) = &mut self.replication_control {
            replication_control.reset();
        }
    }

    pub fn remove_pending_stop_id(&mut self, interest_id: impl Into<InterestId>) -> bool {
        let interest_id = interest_id.into();
        self.replication_control
            .as_mut()
            .is_some_and(|replication_control| {
                replication_control.remove_pending_stop_id(interest_id)
            })
    }

    #[must_use]
    pub const fn written_size(&self) -> usize {
        self.bundle_buffer.len()
    }

    #[must_use]
    pub fn total_bundle_size(&self) -> usize {
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        self.marshal(&mut wb);
        wb.len()
    }

    /// Append one native `StateRecord` for `interest_id` to the bundle buffer,
    /// letting `write` emit its fragments.
    ///
    /// # Errors
    ///
    /// Returns whatever `write` returns as an error, and any error
    /// [`write_state_record`] itself raises. The record is truncated back off
    /// the buffer on failure, so the bundle is left as it was.
    pub fn write_record<R>(
        &mut self,
        interest_id: impl Into<InterestId>,
        write: impl FnOnce(&mut StateRecordWriter<'_>) -> Result<R, MarshalerError>,
    ) -> Result<R, MarshalerError> {
        let mut wb = WriteBuffer::from_vec(CARRIER_ENDIAN, std::mem::take(&mut self.bundle_buffer));
        let result = write_state_record(&mut wb, interest_id, write);
        self.bundle_buffer = wb.into_vec();
        result
    }

    /// Walk every fragment in this bundle's records.
    ///
    /// # Errors
    ///
    /// Returns any error
    /// [`ReplicatedStateBundleView::visit_fragments`] returns.
    pub fn visit_fragments<F>(&self, visit: F) -> Result<(), MarshalerError>
    where
        F: FnMut(
            StateRecordHeader,
            StateFragmentHeaderSpan,
            &mut ReadBuffer<'_>,
        ) -> Result<(), MarshalerError>,
    {
        ReplicatedStateBundleView {
            seq: self.seq,
            client_context_instance_id: self.client_context_instance_id,
            bandwidth_mode: self.bandwidth_mode,
            is_unreliable: self.is_unreliable,
            replication_control: self.replication_control.clone(),
            bundle_buffer: &self.bundle_buffer,
            total_bundle_size: self.total_bundle_size(),
        }
        .visit_fragments(visit)
    }

    fn ensure_replication_control(&mut self) -> &mut ReplicationControlData {
        self.replication_control
            .get_or_insert_with(Default::default)
    }
}

impl Marshaler for ReplicatedStateBundle {
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.seq.marshal(wb);
        self.client_context_instance_id.marshal(wb);
        self.bandwidth_mode.marshal(wb);
        self.is_unreliable.marshal(wb);
        self.replication_control.is_some().marshal(wb);
        if let Some(replication_control) = &self.replication_control {
            replication_control.marshal(wb);
        }
        marshal_bundle_buffer(&self.bundle_buffer, wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        ReplicatedStateBundleView::read_from(rb).map(ReplicatedStateBundleView::into_owned)
    }
}

/// Borrowed receive-side view. `bundle_buffer` points into the carrier message
/// body instead of allocating a second `Vec<u8>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicatedStateBundleView<'a> {
    pub seq: SequenceNumber,
    pub client_context_instance_id: u8,
    pub bandwidth_mode: u8,
    pub is_unreliable: bool,
    pub replication_control: Option<ReplicationControlData>,
    pub bundle_buffer: &'a [u8],
    pub total_bundle_size: usize,
}

impl<'a> ReplicatedStateBundleView<'a> {
    /// Decode a bundle header out of `rb`, borrowing the record payload in
    /// place rather than copying it.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if `rb` ends inside any
    /// header field or the borrowed payload, or
    /// [`MarshalerError::ContainerOverflow`] if the declared payload length
    /// exceeds [`MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE`].
    pub fn read_from(rb: &mut ReadBuffer<'a>) -> Result<Self, MarshalerError> {
        let start = rb.position();
        let seq = rb.field("seq", SequenceNumber::unmarshal)?;
        let client_context_instance_id =
            rb.field("client_context_instance_id", |rb| u8::unmarshal(rb))?;
        let bandwidth_mode = rb.field("bandwidth_mode", |rb| u8::unmarshal(rb))?;
        let is_unreliable = rb.field("is_unreliable", |rb| bool::unmarshal(rb))?;
        let includes_replication_control =
            rb.field("includes_replication_control", |rb| bool::unmarshal(rb))?;
        let replication_control = if includes_replication_control {
            Some(rb.field(
                "replication_control_data",
                ReplicationControlData::unmarshal,
            )?)
        } else {
            None
        };
        let bundle_buffer = rb.field("bundle_buffer", read_bundle_buffer)?;
        let total_bundle_size = rb.position() - start;
        Ok(Self {
            seq,
            client_context_instance_id,
            bandwidth_mode,
            is_unreliable,
            replication_control,
            bundle_buffer,
            total_bundle_size,
        })
    }

    #[must_use]
    pub fn into_owned(self) -> ReplicatedStateBundle {
        ReplicatedStateBundle {
            seq: self.seq,
            client_context_instance_id: self.client_context_instance_id,
            bandwidth_mode: self.bandwidth_mode,
            is_unreliable: self.is_unreliable,
            replication_control: self.replication_control,
            bundle_buffer: self.bundle_buffer.to_vec(),
        }
    }

    #[must_use]
    pub const fn client_context_id(&self) -> ClientContextId {
        ClientContextId::new(self.client_context_instance_id)
    }

    #[must_use]
    pub const fn bandwidth(&self) -> BandwidthMode {
        BandwidthMode::new(self.bandwidth_mode)
    }

    #[must_use]
    pub const fn has_replication_control(&self) -> bool {
        self.replication_control.is_some()
    }

    pub fn replication_control_count(&self) -> usize {
        self.replication_control
            .as_ref()
            .map_or(0, ReplicationControlData::len)
    }

    #[must_use]
    pub const fn total_bundle_size(&self) -> usize {
        self.total_bundle_size
    }

    #[must_use]
    pub const fn has_state_records_payload(&self) -> bool {
        !self.bundle_buffer.is_empty()
    }

    pub fn stop_replication_ids(&self) -> &[InterestId] {
        self.replication_control
            .as_ref()
            .map_or(&[], ReplicationControlData::stop_replication_ids)
    }

    pub fn pause_replication_ids(&self) -> &[InterestId] {
        self.replication_control
            .as_ref()
            .map_or(&[], ReplicationControlData::pause_replication_ids)
    }

    /// Walk every fragment in the borrowed record payload, in wire order.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if a record or fragment
    /// header runs past the end of the payload,
    /// [`MarshalerError::InvalidRange`] if a record declares an interest id
    /// wider than `u16`, or whatever `visit` returns for a fragment body.
    pub fn visit_fragments<F>(&self, mut visit: F) -> Result<(), MarshalerError>
    where
        F: FnMut(
            StateRecordHeader,
            StateFragmentHeaderSpan,
            &mut ReadBuffer<'a>,
        ) -> Result<(), MarshalerError>,
    {
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, self.bundle_buffer);
        while !rb.is_empty() {
            let record_header = read_state_record_header(&mut rb)?;
            for _ in 0..record_header.fragment_count {
                let fragment_header = read_state_fragment_header(&mut rb)?;
                visit(record_header, fragment_header, &mut rb)?;
            }
        }
        Ok(())
    }
}

pub fn marshal_bundle_buffer(bundle_buffer: &[u8], wb: &mut WriteBuffer) {
    debug_assert!(
        bundle_buffer.len() <= MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE,
        "ReplicatedStateBundle buffer exceeds native StateBundleWriteBuffer cap"
    );
    // Every writer caps the buffer at MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE
    // (250 KiB), asserted just above, so the fallback is unreachable; it exists
    // so an impossible length cannot silently wrap into a plausible one.
    VlqU32Marshaler.marshal(wb, u32::try_from(bundle_buffer.len()).unwrap_or(u32::MAX));
    wb.write_bytes(bundle_buffer);
}

/// Read a length-prefixed bundle payload, borrowing it out of `rb`.
///
/// # Errors
///
/// Returns [`MarshalerError::ContainerOverflow`] if the declared length exceeds
/// [`MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE`], or
/// [`MarshalerError::BufferUnderrun`] if the VLQ length or the payload itself
/// runs past the end of `rb`.
pub fn read_bundle_buffer<'a>(rb: &mut ReadBuffer<'a>) -> Result<&'a [u8], MarshalerError> {
    let len = VlqU32Marshaler.unmarshal(rb)? as usize;
    if len > MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE {
        return Err(MarshalerError::ContainerOverflow {
            len,
            capacity: MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE,
        });
    }
    rb.read_bytes(len)
}

/// Header for one native `StateRecord` in `m_bundleBuffer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateRecordHeader {
    pub interest_id: InterestId,
    pub fragment_count: usize,
    pub start: usize,
    pub header_end: usize,
}

/// Header and byte positions for one native `StateFragment`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateFragmentHeaderSpan {
    pub fragment_key: FragmentKey,
    pub type_id: StateFragmentTypeId,
    pub start: usize,
    pub header_end: usize,
    pub body_start: usize,
}

/// Byte span produced by one successful typed fragment write.
///
/// This is an observation surface for diagnostics and indexing. The fragment
/// body is still assembled exclusively by [`Fragment::marshal_contents_with`]
/// through [`StateRecordWriter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateFragmentWriteSpan {
    header: StateFragmentHeaderSpan,
    body_end: usize,
}

impl StateFragmentWriteSpan {
    #[inline]
    #[must_use]
    pub const fn header(self) -> StateFragmentHeaderSpan {
        self.header
    }

    #[inline]
    #[must_use]
    pub const fn body_end(self) -> usize {
        self.body_end
    }

    #[inline]
    #[must_use]
    pub const fn body_len(self) -> usize {
        self.body_end - self.header.body_start
    }
}

/// Native `Amazon::Hub::StateFragmentTypeId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateFragmentTypeId {
    TypeIndex(u32),
    RawUuid(Uuid),
}

impl StateFragmentTypeId {
    #[must_use]
    pub const fn for_class<C>() -> Self
    where
        C: Class,
    {
        match C::TYPE_INDEX {
            0 => Self::RawUuid(<C as AzTypeInfo>::TYPE_ID),
            type_index => Self::TypeIndex(type_index),
        }
    }

    /// Resolve to a UUID, going through `types` only for the compact form.
    #[must_use]
    pub fn uuid(self, types: TypeRegistry<'_>) -> Option<Uuid> {
        match self {
            Self::TypeIndex(type_index) => types.uuid_for_type_index(type_index),
            Self::RawUuid(uuid) => Some(uuid),
        }
    }

    /// Resolve to an envelope index, going through `types` only for the raw
    /// form.
    #[must_use]
    pub fn type_index(self, types: TypeRegistry<'_>) -> Option<u32> {
        match self {
            Self::TypeIndex(type_index) => Some(type_index),
            Self::RawUuid(uuid) => types.type_index_of(&uuid),
        }
    }

    #[must_use]
    pub const fn raw_uuid(self) -> Option<Uuid> {
        match self {
            Self::TypeIndex(_) => None,
            Self::RawUuid(uuid) => Some(uuid),
        }
    }

    /// Decode a fragment type id — a compact envelope index, or a raw UUID
    /// when the index reads zero.
    ///
    /// # Errors
    ///
    /// Returns any error [`read_state_fragment_type_id`] returns.
    pub fn read_from(rb: &mut ReadBuffer<'_>) -> Result<Self, MarshalerError> {
        read_state_fragment_type_id(rb)
    }

    pub fn write_to(self, wb: &mut WriteBuffer) {
        write_state_fragment_type_id(wb, self);
    }
}

/// Read one native `StateRecord` header: its interest id and fragment count.
///
/// # Errors
///
/// Returns [`MarshalerError::InvalidRange`] if the VLQ interest id does not fit
/// the wire's 16-bit id width, or [`MarshalerError::BufferUnderrun`] if `rb`
/// ends inside the interest id or the fragment count.
pub fn read_state_record_header(
    rb: &mut ReadBuffer<'_>,
) -> Result<StateRecordHeader, MarshalerError> {
    let start = rb.position();
    let raw_interest_id = VlqU32Marshaler.unmarshal(rb)?;
    let interest_id = u16::try_from(raw_interest_id).map_err(|_| MarshalerError::InvalidRange {
        value: u64::from(raw_interest_id),
        min: 0,
        max: u64::from(u16::MAX),
    })?;
    let fragment_count = usize::from(rb.read_u8()?);
    Ok(StateRecordHeader {
        interest_id: InterestId::new(interest_id),
        fragment_count,
        start,
        header_end: rb.position(),
    })
}

/// Read one native `StateFragment` header: its fragment key and type id, plus
/// the byte positions that bracket it.
///
/// # Errors
///
/// Returns [`MarshalerError::BufferUnderrun`] if `rb` ends inside the VLQ
/// fragment key or inside the type id read by
/// [`read_state_fragment_type_id`].
pub fn read_state_fragment_header(
    rb: &mut ReadBuffer<'_>,
) -> Result<StateFragmentHeaderSpan, MarshalerError> {
    let start = rb.position();
    let fragment_key = VlqU32Marshaler.unmarshal(rb)?;
    let type_id = read_state_fragment_type_id(rb)?;
    let header_end = rb.position();
    Ok(StateFragmentHeaderSpan {
        fragment_key: FragmentKey::new(fragment_key),
        type_id,
        start,
        header_end,
        body_start: header_end,
    })
}

/// Read a fragment type id: a compact envelope index, or the 16 raw UUID bytes
/// that follow a zero index.
///
/// # Errors
///
/// Returns [`MarshalerError::BufferUnderrun`] if `rb` ends inside the VLQ index
/// or inside the 16 UUID bytes of the raw form.
pub fn read_state_fragment_type_id(
    rb: &mut ReadBuffer<'_>,
) -> Result<StateFragmentTypeId, MarshalerError> {
    let type_index = VlqU32Marshaler.unmarshal(rb)?;
    if type_index == 0 {
        let bytes = rb.read_bytes(16)?;
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(bytes);
        return Ok(StateFragmentTypeId::RawUuid(Uuid::from_bytes(uuid)));
    }

    Ok(StateFragmentTypeId::TypeIndex(type_index))
}

pub fn write_state_fragment_type_id(wb: &mut WriteBuffer, type_id: StateFragmentTypeId) {
    match type_id {
        StateFragmentTypeId::TypeIndex(type_index) => {
            debug_assert!(type_index != 0, "zero typeIndex must be encoded as RawUuid");
            VlqU32Marshaler.marshal(wb, type_index);
        }
        StateFragmentTypeId::RawUuid(uuid) => {
            VlqU32Marshaler.marshal(wb, 0);
            wb.write_bytes(uuid.as_bytes());
        }
    }
}

/// Decode a fragment body into a default-constructed `T`.
///
/// # Errors
///
/// Returns any error [`Fragment::unmarshal_contents`] returns for `T` —
/// typically [`MarshalerError::BufferUnderrun`] on a truncated body.
pub fn decode_state_fragment_contents<T>(rb: &mut ReadBuffer<'_>) -> Result<T, MarshalerError>
where
    T: Fragment + Default,
{
    let mut fragment = T::default();
    fragment.unmarshal_contents(rb)?;
    Ok(fragment)
}

/// Emit one native `StateRecord` — interest id, fragment count prefix, then
/// whatever `write` appends.
///
/// # Errors
///
/// Returns whatever `write` returns as an error, in practice the
/// [`MarshalerError::ContainerOverflow`] raised by
/// [`StateRecordWriter::write_fragment`] on a full fragment slot or an
/// oversized bundle buffer. The partial record is truncated back off `wb`
/// before returning, and also when `write` succeeds without emitting any
/// fragment.
pub fn write_state_record<R>(
    wb: &mut WriteBuffer,
    interest_id: impl Into<InterestId>,
    write: impl FnOnce(&mut StateRecordWriter<'_>) -> Result<R, MarshalerError>,
) -> Result<R, MarshalerError> {
    let record_start = wb.len();
    VlqU32Marshaler.marshal(wb, u32::from(interest_id.into().get()));

    let count = Cell::new(0u8);
    let result = wb.with_fixed_prefix(
        1,
        |wb| {
            let mut record = StateRecordWriter { wb, count: 0 };
            let result = write(&mut record);
            count.set(record.count);
            result
        },
        |prefix, _body| {
            prefix[0] = count.get();
        },
    );

    if result.is_err() || count.get() == 0 {
        wb.truncate(record_start);
    }
    result
}

/// In-progress native `StateRecord`, scoped by [`ReplicatedStateBundle::write_record`].
pub struct StateRecordWriter<'a> {
    wb: &'a mut WriteBuffer,
    count: u8,
}

impl StateRecordWriter<'_> {
    /// Write one typed fragment into the open record.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::write_fragment_with_context_report`] returns.
    pub fn write_fragment<C>(
        &mut self,
        fragment_key: impl Into<FragmentKey>,
        fragment: &C,
    ) -> Result<(), MarshalerError>
    where
        C: Fragment + Class,
    {
        self.write_fragment_with_context(fragment_key, fragment, &MarshalContext::default())
    }

    /// Context-aware form of [`Self::write_fragment`].
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::write_fragment_with_context_report`] returns.
    pub fn write_fragment_with_context<C>(
        &mut self,
        fragment_key: impl Into<FragmentKey>,
        fragment: &C,
        marshal_context: &MarshalContext<'_>,
    ) -> Result<(), MarshalerError>
    where
        C: Fragment + Class,
    {
        self.write_fragment_with_context_report(fragment_key, fragment, marshal_context)
            .map(|_| ())
    }

    /// Writes one typed fragment and reports its exact encoded span.
    ///
    /// `None` is the native `MarshalContents == false` outcome: no fragment
    /// header or body remains in the record.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::write_fragment_with_context_report`] returns.
    pub fn write_fragment_report<C>(
        &mut self,
        fragment_key: impl Into<FragmentKey>,
        fragment: &C,
    ) -> Result<Option<StateFragmentWriteSpan>, MarshalerError>
    where
        C: Fragment + Class,
    {
        self.write_fragment_with_context_report(fragment_key, fragment, &MarshalContext::default())
    }

    /// Context-aware form of [`Self::write_fragment_report`].
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::ContainerOverflow`] if the record already
    /// holds `u8::MAX` fragments, or if appending this fragment would push the
    /// bundle buffer past [`MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE`]. In the
    /// latter case the partial fragment is truncated back off the buffer first.
    pub fn write_fragment_with_context_report<C>(
        &mut self,
        fragment_key: impl Into<FragmentKey>,
        fragment: &C,
        marshal_context: &MarshalContext<'_>,
    ) -> Result<Option<StateFragmentWriteSpan>, MarshalerError>
    where
        C: Fragment + Class,
    {
        let fragment_key = fragment_key.into();
        self.reserve_fragment_slot()?;
        let fragment_start = self.wb.len();
        VlqU32Marshaler.marshal(self.wb, fragment_key.get());
        let type_id = StateFragmentTypeId::for_class::<C>();
        type_id.write_to(self.wb);
        let body_start = self.wb.len();
        let wrote_payload = fragment.marshal_contents_with(marshal_context, self.wb);
        if !wrote_payload {
            self.wb.truncate(fragment_start);
            return Ok(None);
        }
        if self.wb.len() > MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE {
            let len = self.wb.len();
            self.wb.truncate(fragment_start);
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE,
            });
        }
        self.count = self.count.saturating_add(1);
        Ok(Some(StateFragmentWriteSpan {
            header: StateFragmentHeaderSpan {
                fragment_key,
                type_id,
                start: fragment_start,
                header_end: body_start,
                body_start,
            },
            body_end: self.wb.len(),
        }))
    }

    /// Borrows the body emitted for a span returned by this writer.
    #[inline]
    #[must_use]
    pub fn written_fragment_body(&self, span: StateFragmentWriteSpan) -> &[u8] {
        debug_assert!(span.header.body_start <= span.body_end);
        debug_assert!(span.body_end <= self.wb.len());
        &self.wb.as_slice()[span.header.body_start..span.body_end]
    }

    /// Write a fragment whose body is already encoded, bypassing
    /// [`Fragment::marshal_contents_with`].
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::ContainerOverflow`] if the record already
    /// holds `u8::MAX` fragments, or if `body` would push the bundle buffer
    /// past [`MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE`]. In the latter case the
    /// partial fragment is truncated back off the buffer first.
    pub fn write_raw_fragment(
        &mut self,
        fragment_key: impl Into<FragmentKey>,
        type_id: StateFragmentTypeId,
        body: &[u8],
    ) -> Result<(), MarshalerError> {
        let fragment_key = fragment_key.into();
        self.reserve_fragment_slot()?;
        let fragment_start = self.wb.len();
        VlqU32Marshaler.marshal(self.wb, fragment_key.get());
        type_id.write_to(self.wb);
        self.wb.write_bytes(body);
        if self.wb.len() > MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE {
            let len = self.wb.len();
            self.wb.truncate(fragment_start);
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: MAX_REPLICATED_STATE_BUNDLE_BUFFER_SIZE,
            });
        }
        self.count = self.count.saturating_add(1);
        Ok(())
    }

    fn reserve_fragment_slot(&self) -> Result<(), MarshalerError> {
        if self.count == u8::MAX {
            return Err(MarshalerError::ContainerOverflow {
                len: usize::from(self.count) + 1,
                capacity: usize::from(u8::MAX),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::FragmentBase;

    #[derive(
        az_derive::AzTypeInfo,
        gridmate_derive::Marshaler,
        gridmate_derive::ClassDesc,
        Debug,
        Default,
    )]
    #[az_type_info("11111111-1111-4111-8111-111111111111")]
    #[class_desc(type_index = 65_000)]
    struct EmptyFragment {
        #[marshal(skip)]
        base: FragmentBase,
    }

    impl Fragment for EmptyFragment {
        fn base(&self) -> &FragmentBase {
            &self.base
        }

        fn base_mut(&mut self) -> &mut FragmentBase {
            &mut self.base
        }

        fn marshal_contents(&self, _wb: &mut WriteBuffer) -> bool {
            false
        }

        fn unmarshal_contents(&mut self, _rb: &mut ReadBuffer) -> Result<bool, MarshalerError> {
            Ok(false)
        }
    }

    #[derive(
        az_derive::AzTypeInfo,
        gridmate_derive::Marshaler,
        gridmate_derive::ClassDesc,
        Debug,
        Default,
    )]
    #[az_type_info("22222222-2222-4222-8222-222222222222")]
    #[class_desc(type_index = 65_001)]
    struct ByteFragment {
        #[marshal(skip)]
        base: FragmentBase,
        value: u8,
    }

    impl Fragment for ByteFragment {
        fn base(&self) -> &FragmentBase {
            &self.base
        }

        fn base_mut(&mut self) -> &mut FragmentBase {
            &mut self.base
        }

        fn marshal_contents(&self, wb: &mut WriteBuffer) -> bool {
            wb.write_u8(self.value);
            true
        }

        fn unmarshal_contents(&mut self, rb: &mut ReadBuffer) -> Result<bool, MarshalerError> {
            self.value = rb.read_u8()?;
            Ok(true)
        }
    }

    #[test]
    fn bundle_buffer_is_vlq_len_then_raw_bytes() {
        let msg = ReplicatedStateBundle {
            bundle_buffer: vec![0xaa, 0xbb, 0xcc],
            ..Default::default()
        };

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        msg.marshal(&mut wb);

        assert_eq!(wb.as_slice(), &[0, 0, 0, 0, 0, 3, 0xaa, 0xbb, 0xcc]);
    }

    #[test]
    fn view_borrows_bundle_buffer() {
        let msg = ReplicatedStateBundle {
            bundle_buffer: vec![0xaa, 0xbb],
            ..Default::default()
        };
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        msg.marshal(&mut wb);
        let bytes = wb.into_vec();

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let view = ReplicatedStateBundleView::read_from(&mut rb).unwrap();

        assert_eq!(view.bundle_buffer, &[0xaa, 0xbb]);
        assert_eq!(view.total_bundle_size(), bytes.len());
        assert_eq!(rb.left(), 0);
    }

    #[test]
    fn replication_control_partitions_stop_and_pause_ids() {
        let data = ReplicationControlData::from_ids([10, 11], [20, 21, 22]).unwrap();
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        data.marshal(&mut wb);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, wb.as_slice());
        let decoded = ReplicationControlData::unmarshal(&mut rb).unwrap();

        assert_eq!(
            decoded.stop_replication_ids(),
            &[InterestId::new(10), InterestId::new(11)]
        );
        assert_eq!(
            decoded.pause_replication_ids(),
            &[
                InterestId::new(20),
                InterestId::new(21),
                InterestId::new(22)
            ]
        );
        assert_eq!(decoded.len(), 5);
    }

    #[test]
    fn replication_control_removes_pending_stop_ids_only() {
        let mut data = ReplicationControlData::from_ids([10, 11, 12], [20, 21]).unwrap();

        assert!(data.remove_pending_stop_id(11));
        assert_eq!(
            data.stop_replication_ids(),
            &[InterestId::new(10), InterestId::new(12)]
        );
        assert_eq!(
            data.pause_replication_ids(),
            &[InterestId::new(20), InterestId::new(21)]
        );
        assert_eq!(data.pause_start_idx(), 2);

        assert!(!data.remove_pending_stop_id(21));
        assert_eq!(
            data.stop_replication_ids(),
            &[InterestId::new(10), InterestId::new(12)]
        );
        assert_eq!(
            data.pause_replication_ids(),
            &[InterestId::new(20), InterestId::new(21)]
        );

        data.reset();
        assert!(data.is_empty());
        assert_eq!(data.pause_start_idx(), 0);
    }

    #[test]
    fn replication_performance_data_roundtrips_source_field_order() {
        let msg = ReplicationPerformanceData {
            collection_period_ms: 250,
            count_bundles: 1,
            count_throttled: 2,
            count_overflows: 3,
            count_characters_intended: 4,
            count_characters_actual: 5,
            characters_missed_due_to_rate_limiter: 6,
            objects_missed_due_to_rate_limiter: 7,
            characters_missed_due_to_overflow: 8,
            objects_missed_due_to_overflow: 9,
            characters_added_during_period: 10,
            objects_added_during_period: 11,
        };

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        msg.marshal(&mut wb);
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, wb.as_slice());
        let decoded = ReplicationPerformanceData::unmarshal(&mut rb).unwrap();

        assert_eq!(decoded, msg);
        assert_eq!(rb.left(), 0);
        assert_eq!(
            <ReplicationPerformanceData as crate::message::Message>::TYPE_INDEX,
            <ReplicationPerformanceData as Class>::TYPE_INDEX
        );
    }

    #[test]
    fn replication_performance_data_uses_source_counter_semantics() {
        let mut lhs = ReplicationPerformanceData {
            collection_period_ms: 100,
            count_bundles: u32::MAX,
            count_throttled: 1,
            ..Default::default()
        };
        let rhs = ReplicationPerformanceData {
            collection_period_ms: 250,
            count_bundles: 2,
            count_throttled: 3,
            ..Default::default()
        };

        lhs += &rhs;
        assert_eq!(lhs.collection_period_ms, 250);
        assert_eq!(lhs.count_bundles, 1);
        assert_eq!(lhs.count_throttled, 4);

        lhs.reset();
        assert_eq!(lhs.collection_period_ms, 250);
        assert_eq!(lhs.count_bundles, 0);
        assert_eq!(lhs.count_throttled, 0);
    }

    #[test]
    fn replicated_state_bundle_source_accessors_manage_flags_and_control_data() {
        let mut bundle = ReplicatedStateBundle::with_seq(SequenceNumber::Seq(7), 3, 1);

        assert_eq!(bundle.seq(), SequenceNumber::Seq(7));
        assert_eq!(bundle.client_context_instance_id(), 3);
        assert_eq!(bundle.bandwidth_mode(), 1);
        assert!(!bundle.is_unreliable());
        assert!(!bundle.includes_replication_control());

        bundle.set_unreliable();
        bundle.add_stop_id(10).unwrap();
        bundle.add_pause_id(20).unwrap();

        assert!(bundle.is_unreliable());
        assert!(bundle.includes_replication_control());
        assert_eq!(bundle.stop_replication_ids(), &[InterestId::new(10)]);
        assert_eq!(bundle.pause_replication_ids(), &[InterestId::new(20)]);
        assert_eq!(bundle.replication_control_count(), 2);

        assert!(bundle.remove_pending_stop_id(10));
        assert_eq!(bundle.stop_replication_ids(), &[] as &[InterestId]);
        assert_eq!(bundle.pause_replication_ids(), &[InterestId::new(20)]);

        bundle.reset_replication_control();
        assert!(bundle.includes_replication_control());
        assert_eq!(bundle.replication_control_count(), 0);
    }

    #[test]
    fn write_record_roundtrips_raw_fragment_headers() {
        let uuid = Uuid::parse_str("11223344-5566-7788-99aa-bbccddeeff00").unwrap();
        let mut bundle = ReplicatedStateBundle::default();
        bundle
            .write_record(2, |record| {
                record.write_raw_fragment(3, StateFragmentTypeId::RawUuid(uuid), &[0xcc])
            })
            .unwrap();

        let mut seen = Vec::new();
        bundle
            .visit_fragments(|record, fragment, rb| {
                seen.push((record.interest_id, fragment.fragment_key, fragment.type_id));
                assert_eq!(rb.read_u8()?, 0xcc);
                Ok(())
            })
            .unwrap();

        assert_eq!(
            seen,
            vec![(
                InterestId::new(2),
                FragmentKey::new(3),
                StateFragmentTypeId::RawUuid(uuid)
            )]
        );
    }

    #[test]
    fn write_record_rolls_back_fragments_without_payload() {
        let mut bundle = ReplicatedStateBundle::default();
        bundle
            .write_record(7, |record| {
                record.write_fragment(12, &EmptyFragment::default())
            })
            .unwrap();

        assert!(bundle.bundle_buffer.is_empty());
    }

    #[test]
    fn typed_fragment_write_report_borrows_the_emitted_body() {
        let mut bundle = ReplicatedStateBundle::default();
        bundle
            .write_record(7, |record| {
                assert!(
                    record
                        .write_fragment_report(12, &EmptyFragment::default())?
                        .is_none()
                );

                let span = record
                    .write_fragment_report(
                        13,
                        &ByteFragment {
                            value: 0xcc,
                            ..Default::default()
                        },
                    )?
                    .expect("non-empty fragment is retained");
                assert_eq!(span.header().fragment_key, FragmentKey::new(13));
                assert_eq!(
                    span.header().type_id,
                    StateFragmentTypeId::TypeIndex(ByteFragment::TYPE_INDEX)
                );
                assert_eq!(span.body_len(), 1);
                assert_eq!(record.written_fragment_body(span), &[0xcc]);
                Ok(())
            })
            .unwrap();

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bundle.bundle_buffer);
        let record = read_state_record_header(&mut rb).unwrap();
        assert_eq!(record.fragment_count, 1);
    }

    #[test]
    fn failed_record_write_rolls_back() {
        let mut bundle = ReplicatedStateBundle::with_bundle_buffer(vec![0xaa]);
        let err = bundle
            .write_record(2, |record| -> Result<(), MarshalerError> {
                record.write_raw_fragment(3, StateFragmentTypeId::TypeIndex(4), &[0xcc])?;
                Err(MarshalerError::InvalidDiscriminant { value: 99 })
            })
            .unwrap_err();

        assert!(matches!(
            err,
            MarshalerError::InvalidDiscriminant { value: 99 }
        ));
        assert_eq!(bundle.bundle_buffer, vec![0xaa]);
    }
}
