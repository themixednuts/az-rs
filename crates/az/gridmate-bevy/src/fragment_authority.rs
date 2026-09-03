//! Server-authoritative `GridMate` fragment state stored on Bevy entities.

use bevy::prelude::Component;
use gridmate::hub::{Fragment, FragmentKey, SequenceNumber};

use crate::fragment_state::{FragmentMergeError, merge_fragment};

/// Canonical merged state for one actor fragment.
///
/// Domain systems publish partial concrete fragments at the authoritative
/// simulation sequence. The component owns source `MergeAndUpdateSequence`
/// semantics and exposes only the resulting immutable state to send systems.
#[derive(Component, Debug, Clone)]
pub struct AuthoritativeFragment<C: Send + Sync + 'static> {
    fragment_key: FragmentKey,
    state: C,
}

impl<C: Send + Sync + 'static> AuthoritativeFragment<C> {
    #[must_use]
    pub fn new(fragment_key: impl Into<FragmentKey>, initial_state: C) -> Self {
        Self {
            fragment_key: fragment_key.into(),
            state: initial_state,
        }
    }

    #[must_use]
    pub const fn fragment_key(&self) -> FragmentKey {
        self.fragment_key
    }

    #[must_use]
    pub const fn state(&self) -> &C {
        &self.state
    }
}

impl<C> AuthoritativeFragment<C>
where
    C: Fragment + Send + Sync + 'static,
{
    /// Merge one domain projection at its authoritative simulation sequence.
    ///
    /// Returns the published sequence only when at least one replicated field
    /// changed. A no-op projection leaves the prior canonical update sequence
    /// intact.
    ///
    /// # Errors
    ///
    /// Returns [`FragmentMergeError::NonIncreasingSequence`] if the current
    /// update sequence is valid and `sequence` does not advance past it, plus
    /// any error [`merge_fragment`] returns while reconciling `candidate`
    /// against the retained state.
    pub fn publish(
        &mut self,
        mut candidate: C,
        sequence: SequenceNumber,
    ) -> Result<Option<SequenceNumber>, FragmentMergeError> {
        let current = self.state.update_sequence();
        if current.is_valid() && sequence <= current {
            return Err(FragmentMergeError::NonIncreasingSequence {
                current,
                incoming: sequence,
            });
        }
        let merged = merge_fragment(&self.state, &mut candidate, sequence)?;
        if !merged.detected_new_data_in_last_merge() {
            return Ok(None);
        }
        self.state = merged;
        Ok(Some(sequence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridmate::hub::{FragmentBase, MarshalContext};
    use gridmate::serialize::{MarshalerError, ReadBuffer, WriteBuffer};

    #[derive(Debug, Clone, Default)]
    struct TestFragment {
        base: FragmentBase,
        value: u8,
        sequence: SequenceNumber,
        changed: bool,
    }

    impl Fragment for TestFragment {
        fn base(&self) -> &FragmentBase {
            &self.base
        }

        fn base_mut(&mut self) -> &mut FragmentBase {
            &mut self.base
        }

        fn merge_and_update_sequence(
            &self,
            incoming: &mut dyn Fragment,
            sequence: SequenceNumber,
            _inherit_previous_network_data_status: bool,
        ) -> Option<Box<dyn Fragment>> {
            let incoming_any: &mut dyn std::any::Any = incoming;
            let incoming = incoming_any.downcast_mut::<Self>()?;
            Some(Box::new(Self {
                value: incoming.value,
                sequence,
                changed: incoming.value != self.value,
                ..self.clone()
            }))
        }

        fn detected_new_data_in_last_merge(&self) -> bool {
            self.changed
        }

        fn update_sequence(&self) -> SequenceNumber {
            self.sequence
        }

        fn marshal_contents(&self, _wb: &mut WriteBuffer) -> bool {
            false
        }

        fn marshal_contents_with(&self, _mc: &MarshalContext<'_>, _wb: &mut WriteBuffer) -> bool {
            false
        }

        fn unmarshal_contents(&mut self, _rb: &mut ReadBuffer) -> Result<bool, MarshalerError> {
            Ok(false)
        }
    }

    #[test]
    fn publication_advances_only_when_concrete_state_changes() {
        let mut authoritative = AuthoritativeFragment::new(16, TestFragment::default());

        assert_eq!(
            authoritative
                .publish(TestFragment::default(), SequenceNumber::Seq(1))
                .unwrap(),
            None
        );
        assert_eq!(
            authoritative
                .publish(
                    TestFragment {
                        value: 7,
                        ..Default::default()
                    },
                    SequenceNumber::Seq(2),
                )
                .unwrap(),
            Some(SequenceNumber::Seq(2))
        );
        assert_eq!(authoritative.state().value, 7);
        assert_eq!(
            authoritative.state().update_sequence(),
            SequenceNumber::Seq(2)
        );
    }
}
