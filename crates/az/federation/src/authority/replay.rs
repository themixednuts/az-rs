use crate::{
    AuthorityAtomId, AuthorityEpoch, AuthoritySequence, ContentDigest, FederationId,
    HistoryCheckpointRef,
};

/// Game-owned deterministic semantics used to replay one authority atom.
pub trait AuthorityReplayPolicy {
    /// Canonical state reconstructed by replay.
    type State;
    /// One canonical event in authority order.
    type Event;
    /// Game-semantic refusal while applying an otherwise valid event.
    type Rejection;

    /// Computes the canonical digest of one state value.
    fn state_digest(&self, state: &Self::State) -> ContentDigest;

    /// Computes the canonical digest of one event payload.
    fn event_digest(&self, event: &Self::Event) -> ContentDigest;

    /// Applies one event under the release and Ruleset semantics that created it.
    ///
    /// # Errors
    ///
    /// Returns the game's typed refusal when the event is invalid under its
    /// pinned release and Ruleset.
    fn apply(&self, state: &mut Self::State, event: &Self::Event) -> Result<(), Self::Rejection>;
}

/// Restored state and its claimed position in one authority epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityReplaySnapshot<S> {
    federation: FederationId,
    atom: AuthorityAtomId,
    epoch: AuthorityEpoch,
    through_sequence: u64,
    state_digest: ContentDigest,
    state: S,
}

impl<S> AuthorityReplaySnapshot<S> {
    /// Creates a replay input. Verification recomputes `state_digest` before use.
    #[must_use]
    pub const fn new(
        federation: FederationId,
        atom: AuthorityAtomId,
        epoch: AuthorityEpoch,
        through_sequence: u64,
        state_digest: ContentDigest,
        state: S,
    ) -> Self {
        Self {
            federation,
            atom,
            epoch,
            through_sequence,
            state_digest,
            state,
        }
    }
}

/// Trusted atom state named by an authenticated recovery manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityReplayTarget {
    federation: FederationId,
    atom: AuthorityAtomId,
    epoch: AuthorityEpoch,
    checkpoint: HistoryCheckpointRef,
    through_sequence: u64,
    state_digest: ContentDigest,
}

impl AuthorityReplayTarget {
    /// Creates the exact endpoint that replay must reproduce.
    #[must_use]
    pub const fn new(
        federation: FederationId,
        atom: AuthorityAtomId,
        epoch: AuthorityEpoch,
        checkpoint: HistoryCheckpointRef,
        through_sequence: u64,
        state_digest: ContentDigest,
    ) -> Self {
        Self {
            federation,
            atom,
            epoch,
            checkpoint,
            through_sequence,
            state_digest,
        }
    }
}

/// One ordered authority event with its committed before/after state roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityReplayEvent<E> {
    sequence: AuthoritySequence,
    event_digest: ContentDigest,
    state_before: ContentDigest,
    state_after: ContentDigest,
    event: E,
}

impl<E> AuthorityReplayEvent<E> {
    /// Binds one semantic event to its authority order and state transition.
    #[must_use]
    pub const fn new(
        sequence: AuthoritySequence,
        event_digest: ContentDigest,
        state_before: ContentDigest,
        state_after: ContentDigest,
        event: E,
    ) -> Self {
        Self {
            sequence,
            event_digest,
            state_before,
            state_after,
            event,
        }
    }
}

/// Replay failure before a restored atom can become authoritative.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthorityReplayRefusal<R> {
    /// The restored state does not match its stored digest.
    #[error("authority recovery snapshot state digest does not match its contents")]
    SnapshotStateMismatch,
    /// Snapshot and target name different federations.
    #[error("authority recovery snapshot names the wrong federation")]
    WrongFederation,
    /// Snapshot and target name different serial authority owners.
    #[error("authority recovery snapshot names the wrong atom")]
    WrongAtom,
    /// Snapshot and target belong to different visible authority epochs.
    #[error("authority recovery snapshot names the wrong epoch")]
    WrongEpoch,
    /// The target cannot precede the restored snapshot.
    #[error("authority recovery target sequence {target} precedes snapshot {snapshot}")]
    TargetBeforeSnapshot { snapshot: u64, target: u64 },
    /// No authority order exists after the maximum sequence.
    #[error("authority recovery sequence is exhausted")]
    SequenceExhausted,
    /// An event was missing, duplicated, or reordered.
    #[error("authority recovery event sequence {actual} does not follow {expected}")]
    SequenceMismatch { expected: u64, actual: u64 },
    /// An event does not extend the state produced before it.
    #[error("authority recovery event {sequence} names the wrong prior state")]
    PriorStateMismatch { sequence: u64 },
    /// The canonical event contents differ from the committed event.
    #[error("authority recovery event {sequence} has the wrong event digest")]
    EventDigestMismatch { sequence: u64 },
    /// The game rejected an event under its pinned semantics.
    #[error("game semantics rejected authority recovery event {sequence}")]
    Apply { sequence: u64, rejection: R },
    /// Event contents do not produce the committed state root.
    #[error("authority recovery event {sequence} produces the wrong state root")]
    ResultStateMismatch { sequence: u64 },
    /// Replay ended before the trusted target or continued past it.
    #[error("authority recovery replay ended at sequence {actual}, expected {expected}")]
    TargetSequenceMismatch { expected: u64, actual: u64 },
    /// Ordered replay did not reproduce the trusted target state root.
    #[error("authority recovery replay produced the wrong target state root")]
    TargetStateMismatch,
}

/// State proven by deterministic, ordered replay to one trusted target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAuthorityReplay<S> {
    federation: FederationId,
    atom: AuthorityAtomId,
    epoch: AuthorityEpoch,
    checkpoint: HistoryCheckpointRef,
    through_sequence: u64,
    state_digest: ContentDigest,
    state: S,
}

impl<S> VerifiedAuthorityReplay<S> {
    /// Returns the federation whose state was replayed.
    #[must_use]
    pub const fn federation(&self) -> FederationId {
        self.federation
    }

    /// Returns the serial authority atom whose state was replayed.
    #[must_use]
    pub const fn atom(&self) -> AuthorityAtomId {
        self.atom
    }

    /// Returns the authority epoch reconstructed by replay.
    #[must_use]
    pub const fn epoch(&self) -> AuthorityEpoch {
        self.epoch
    }

    /// Returns the selected history checkpoint that supplied the target.
    #[must_use]
    pub const fn checkpoint(&self) -> HistoryCheckpointRef {
        self.checkpoint
    }

    /// Returns the verified reconstructed state.
    #[must_use]
    pub const fn state(&self) -> &S {
        &self.state
    }

    /// Returns the verified terminal state root.
    #[must_use]
    pub const fn state_digest(&self) -> ContentDigest {
        self.state_digest
    }

    /// Returns the last authority sequence included in the proof.
    #[must_use]
    pub const fn through_sequence(&self) -> u64 {
        self.through_sequence
    }

    /// Transfers the verified reconstructed state to its next owner.
    #[must_use]
    pub fn into_state(self) -> S {
        self.state
    }
}

/// Replays one snapshot to an authenticated atom target.
///
/// # Errors
///
/// Refuses altered snapshots, wrong identity or epoch, missing or reordered
/// events, broken state links, semantic rejection, and target mismatch.
pub fn verify_authority_replay<P: AuthorityReplayPolicy>(
    policy: &P,
    snapshot: AuthorityReplaySnapshot<P::State>,
    target: AuthorityReplayTarget,
    events: &[AuthorityReplayEvent<P::Event>],
) -> Result<VerifiedAuthorityReplay<P::State>, AuthorityReplayRefusal<P::Rejection>> {
    if policy.state_digest(&snapshot.state) != snapshot.state_digest {
        return Err(AuthorityReplayRefusal::SnapshotStateMismatch);
    }
    if snapshot.federation != target.federation {
        return Err(AuthorityReplayRefusal::WrongFederation);
    }
    if snapshot.atom != target.atom {
        return Err(AuthorityReplayRefusal::WrongAtom);
    }
    if snapshot.epoch != target.epoch {
        return Err(AuthorityReplayRefusal::WrongEpoch);
    }
    if snapshot.through_sequence > target.through_sequence {
        return Err(AuthorityReplayRefusal::TargetBeforeSnapshot {
            snapshot: snapshot.through_sequence,
            target: target.through_sequence,
        });
    }

    let mut state = snapshot.state;
    let mut state_digest = snapshot.state_digest;
    let mut through_sequence = snapshot.through_sequence;
    for event in events {
        let expected = through_sequence
            .checked_add(1)
            .ok_or(AuthorityReplayRefusal::SequenceExhausted)?;
        let actual = event.sequence.get();
        if actual != expected {
            return Err(AuthorityReplayRefusal::SequenceMismatch { expected, actual });
        }
        if event.state_before != state_digest {
            return Err(AuthorityReplayRefusal::PriorStateMismatch { sequence: actual });
        }
        if policy.event_digest(&event.event) != event.event_digest {
            return Err(AuthorityReplayRefusal::EventDigestMismatch { sequence: actual });
        }
        policy
            .apply(&mut state, &event.event)
            .map_err(|rejection| AuthorityReplayRefusal::Apply {
                sequence: actual,
                rejection,
            })?;
        let calculated = policy.state_digest(&state);
        if calculated != event.state_after {
            return Err(AuthorityReplayRefusal::ResultStateMismatch { sequence: actual });
        }
        state_digest = calculated;
        through_sequence = actual;
    }

    if through_sequence != target.through_sequence {
        return Err(AuthorityReplayRefusal::TargetSequenceMismatch {
            expected: target.through_sequence,
            actual: through_sequence,
        });
    }
    if state_digest != target.state_digest {
        return Err(AuthorityReplayRefusal::TargetStateMismatch);
    }
    Ok(VerifiedAuthorityReplay {
        federation: target.federation,
        atom: target.atom,
        epoch: target.epoch,
        checkpoint: target.checkpoint,
        through_sequence,
        state_digest,
        state,
    })
}
