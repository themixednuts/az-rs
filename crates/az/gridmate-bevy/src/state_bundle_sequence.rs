//! Bevy component adapter for the Hub state-bundle receive sequence.

use bevy::prelude::Component;
use gridmate::hub::{SequenceNumber, StateBundleLane, StateBundleSequenceTracker};

/// Per-client ECS state for accepting application-level state-bundle
/// sequences.
///
/// Hub owns the protocol state machine. This component only gives it an ECS
/// lifetime on the entity representing a connected client session.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientStateBundleSequence {
    tracker: StateBundleSequenceTracker,
}

impl ClientStateBundleSequence {
    #[must_use]
    pub const fn expected(&self, lane: StateBundleLane) -> SequenceNumber {
        self.tracker.expected(lane)
    }

    pub fn accept(&mut self, lane: StateBundleLane, sequence: SequenceNumber) -> bool {
        self.tracker.accept(lane, sequence)
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::World;

    use super::*;

    #[test]
    fn component_uses_the_hub_sequence_tracker() {
        let mut world = World::new();
        let entity = world.spawn(ClientStateBundleSequence::default()).id();
        let mut sequence = world
            .get_mut::<ClientStateBundleSequence>(entity)
            .expect("sequence component");

        assert!(sequence.accept(StateBundleLane::Unreliable, SequenceNumber::Seq(1)));
        assert!(!sequence.accept(StateBundleLane::Unreliable, SequenceNumber::Seq(1)));
        assert_eq!(
            sequence.expected(StateBundleLane::Unreliable),
            SequenceNumber::Seq(2)
        );
    }
}
