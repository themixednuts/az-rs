//! Typed receive-side storage for merged `GridMate` replica fragments.

use std::collections::HashMap;

use bevy::prelude::Resource;
use gridmate::{
    hub::{Fragment, FragmentKey, InterestId, SequenceNumber},
    session_service::SessionId,
};

use crate::fragment_state::{FragmentMergeError, merge_fragment};

/// Native identity of one portrayed fragment in one network session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FragmentReplicaKey {
    pub session: SessionId,
    pub interest_id: InterestId,
    pub fragment_key: FragmentKey,
}

impl FragmentReplicaKey {
    #[must_use]
    pub const fn new(
        session: SessionId,
        interest_id: InterestId,
        fragment_key: FragmentKey,
    ) -> Self {
        Self {
            session,
            interest_id,
            fragment_key,
        }
    }
}

/// Fully merged fragments for one concrete replicated-state class.
///
/// The concrete type is encoded by this resource's monomorph. This mirrors the
/// native per-replica fragment store while keeping project/domain code out of
/// `GridMate` delta reconciliation.
#[derive(Resource)]
pub struct FragmentSnapshotStore<C> {
    by_key: HashMap<FragmentReplicaKey, C>,
}

impl<C> Default for FragmentSnapshotStore<C> {
    fn default() -> Self {
        Self {
            by_key: HashMap::new(),
        }
    }
}

impl<C> FragmentSnapshotStore<C>
where
    C: Fragment + Default + Clone + 'static,
{
    /// Merge one decoded fragment body and return the authoritative snapshot.
    ///
    /// # Errors
    ///
    /// Returns any error [`merge_fragment`] returns when reconciling
    /// `incoming` at `sequence` against the snapshot retained for `key` —
    /// notably [`FragmentMergeError::NonIncreasingSequence`] when `sequence`
    /// does not advance past the retained one.
    pub fn merge(
        &mut self,
        key: FragmentReplicaKey,
        mut incoming: C,
        sequence: SequenceNumber,
    ) -> Result<C, FragmentMergeError> {
        let previous = self.by_key.get(&key).cloned().unwrap_or_default();
        let snapshot = merge_fragment(&previous, &mut incoming, sequence)?;
        self.by_key.insert(key, snapshot.clone());
        Ok(snapshot)
    }

    /// Remove every concrete fragment belonging to a stopped interest.
    pub fn stop_replication(&mut self, session: SessionId, interest_id: InterestId) {
        self.by_key
            .retain(|key, _| key.session != session || key.interest_id != interest_id);
    }

    /// Remove every concrete fragment belonging to a disconnected session.
    pub fn disconnect(&mut self, session: SessionId) {
        self.by_key.retain(|key, _| key.session != session);
    }
}
