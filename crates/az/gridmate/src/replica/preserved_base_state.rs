//! `PreservedBaseState` — a native base-state body kept around for round-trip
//! rather than consumed and discarded.
//!
//! Source base-state readers consume
//! `[u8 has_rev][opt RawU64Revision][MaskChain]`. A handful of fragments
//! (`AttributeComponentReplicatedState`,
//! `ChatReplicatedState`, `CooldownTimersComponentReplicatedState`,
//! `MusicalPerformancePlayerReplicatedState`) need to *preserve* the
//! revision and mask chain — they all used to carry duplicated
//! `base_revision: Option<u64>` + `base_state_masks: MaskChain` fields and
//! identical override bodies.
//!
//! This type collapses that pair into one preserved-body unit. A fragment
//! that needs preservation embeds it as `pub base_state: PreservedBaseState`
//! and delegates its base-state methods to
//! [`PreservedBaseState::unmarshal_into`] / [`PreservedBaseState::marshal`].

use crate::serialize::{
    MarshalerError, MaskChain, RawU64Revision, ReadBuffer, WriteBuffer, marshaler::Marshaler,
};

/// Group 2 (`BaseState`) wire body, preserved verbatim for round-tripping.
///
/// Wire shape: `[u8 has_rev][opt RawU64Revision][MaskChain]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreservedBaseState {
    /// `Some(rev)` when the wire carried a base revision; `None` for
    /// fragments whose server snapshot left the revision unset.
    pub revision: Option<u64>,
    /// The base-state replicated field sub-field mask chain at chunk+0x680.
    pub masks: MaskChain,
}

impl PreservedBaseState {
    /// Read the base-state body from `rb` into `self`. Returns the parsed
    /// revision for callers that need to mirror the source method result.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if `rb` runs out before the
    /// `has_rev` byte, the optional [`RawU64Revision`], or the trailing
    /// [`MaskChain`] is complete, and any other error
    /// [`MaskChain::unmarshal`] raises for a malformed mask chain.
    pub fn unmarshal_into(&mut self, rb: &mut ReadBuffer) -> Result<Option<u64>, MarshalerError> {
        self.revision = Option::<RawU64Revision>::unmarshal(rb)?.map(|r| r.0);
        self.masks = MaskChain::unmarshal(rb)?;
        Ok(self.revision)
    }

    /// Write the preserved body to `wb`.
    pub fn marshal(&self, wb: &mut WriteBuffer) {
        self.revision.map(RawU64Revision).marshal(wb);
        self.masks.marshal(wb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::buffer::CARRIER_ENDIAN;

    #[test]
    fn empty_round_trip() {
        let original = PreservedBaseState::default();

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        original.marshal(&mut wb);
        // [None][MaskChain::empty() = 0x00] → 2 bytes
        assert_eq!(wb.as_slice(), &[0, 0]);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, wb.as_slice());
        let mut decoded = PreservedBaseState::default();
        let revision = decoded.unmarshal_into(&mut rb).unwrap();
        assert_eq!(rb.left(), 0);
        assert_eq!(revision, None);
        assert_eq!(decoded, original);
    }

    #[test]
    fn revision_and_masks_round_trip() {
        let original = PreservedBaseState {
            revision: Some(0x0102_0304_0506_0708),
            masks: MaskChain::from_dirty_fields(&[true, false, true, false, true, false, true]),
        };

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        original.marshal(&mut wb);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, wb.as_slice());
        let mut decoded = PreservedBaseState::default();
        let revision = decoded.unmarshal_into(&mut rb).unwrap();
        assert_eq!(rb.left(), 0);
        assert_eq!(revision, Some(0x0102_0304_0506_0708));
        assert_eq!(decoded, original);
    }
}
