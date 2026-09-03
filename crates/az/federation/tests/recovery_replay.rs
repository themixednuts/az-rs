use az_federation::{
    AuthorityAtomId, AuthorityEpoch, AuthorityReplayEvent, AuthorityReplayPolicy,
    AuthorityReplayRefusal, AuthorityReplaySnapshot, AuthorityReplayTarget, AuthoritySequence,
    ContentDigest, FederationId, HistoryCheckpointRef, verify_authority_replay,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CounterEvent {
    Add(u8),
    Multiply(u8),
}

struct CounterReplay;

impl AuthorityReplayPolicy for CounterReplay {
    type Event = CounterEvent;
    type Rejection = ();
    type State = u8;

    fn state_digest(&self, state: &Self::State) -> ContentDigest {
        ContentDigest::from_bytes([*state; 32])
    }

    fn event_digest(&self, event: &Self::Event) -> ContentDigest {
        match event {
            CounterEvent::Add(value) => ContentDigest::from_bytes([0xa0 | value; 32]),
            CounterEvent::Multiply(value) => ContentDigest::from_bytes([0xb0 | value; 32]),
        }
    }

    fn apply(&self, state: &mut Self::State, event: &Self::Event) -> Result<(), Self::Rejection> {
        *state = match event {
            CounterEvent::Add(value) => state.checked_add(*value).ok_or(())?,
            CounterEvent::Multiply(value) => state.checked_mul(*value).ok_or(())?,
        };
        Ok(())
    }
}

const fn digest(value: u8) -> ContentDigest {
    ContentDigest::from_bytes([value; 32])
}

const fn atom() -> AuthorityAtomId {
    AuthorityAtomId::from_digest(digest(0x41))
}

const fn federation() -> FederationId {
    FederationId::from_digest(digest(0x10))
}

const fn checkpoint() -> HistoryCheckpointRef {
    HistoryCheckpointRef::new(39, digest(0x23))
}

fn epoch() -> AuthorityEpoch {
    AuthorityEpoch::try_from(7).expect("authority epoch")
}

fn sequence(value: u64) -> AuthoritySequence {
    AuthoritySequence::try_from(value).expect("authority sequence")
}

#[test]
fn snapshot_plus_ordered_replay_reproduces_the_trusted_state_root() {
    let snapshot = AuthorityReplaySnapshot::new(federation(), atom(), epoch(), 1, digest(3), 3);
    let events = [
        AuthorityReplayEvent::new(
            sequence(2),
            digest(0xa4),
            digest(3),
            digest(7),
            CounterEvent::Add(4),
        ),
        AuthorityReplayEvent::new(
            sequence(3),
            digest(0xb2),
            digest(7),
            digest(14),
            CounterEvent::Multiply(2),
        ),
    ];
    let target =
        AuthorityReplayTarget::new(federation(), atom(), epoch(), checkpoint(), 3, digest(14));

    let replay = verify_authority_replay(&CounterReplay, snapshot, target, &events)
        .expect("verified authority replay");

    assert_eq!(replay.state(), &14);
    assert_eq!(replay.state_digest(), digest(14));
    assert_eq!(replay.through_sequence(), 3);
    assert_eq!(replay.federation(), federation());
    assert_eq!(replay.checkpoint(), checkpoint());
}

#[test]
fn semantically_equivalent_event_substitution_is_still_altered_history() {
    let snapshot = AuthorityReplaySnapshot::new(federation(), atom(), epoch(), 1, digest(3), 3);
    let altered = [AuthorityReplayEvent::new(
        sequence(2),
        digest(0xa0),
        digest(3),
        digest(3),
        CounterEvent::Multiply(1),
    )];
    let target =
        AuthorityReplayTarget::new(federation(), atom(), epoch(), checkpoint(), 2, digest(3));

    let result = verify_authority_replay(&CounterReplay, snapshot, target, &altered);

    assert_eq!(
        result,
        Err(AuthorityReplayRefusal::EventDigestMismatch { sequence: 2 })
    );
}

#[test]
fn missing_or_reordered_events_refuse_recovery() {
    let snapshot = AuthorityReplaySnapshot::new(federation(), atom(), epoch(), 1, digest(3), 3);
    let event_two = AuthorityReplayEvent::new(
        sequence(2),
        digest(0xa4),
        digest(3),
        digest(7),
        CounterEvent::Add(4),
    );
    let event_three = AuthorityReplayEvent::new(
        sequence(3),
        digest(0xb2),
        digest(7),
        digest(14),
        CounterEvent::Multiply(2),
    );
    let target =
        AuthorityReplayTarget::new(federation(), atom(), epoch(), checkpoint(), 3, digest(14));

    assert_eq!(
        verify_authority_replay(
            &CounterReplay,
            snapshot.clone(),
            target,
            std::slice::from_ref(&event_two),
        ),
        Err(AuthorityReplayRefusal::TargetSequenceMismatch {
            expected: 3,
            actual: 2,
        })
    );
    assert_eq!(
        verify_authority_replay(&CounterReplay, snapshot, target, &[event_three, event_two]),
        Err(AuthorityReplayRefusal::SequenceMismatch {
            expected: 2,
            actual: 3,
        })
    );
}

#[test]
fn altered_snapshot_or_state_chain_refuses_recovery() {
    let target =
        AuthorityReplayTarget::new(federation(), atom(), epoch(), checkpoint(), 2, digest(7));
    let event = AuthorityReplayEvent::new(
        sequence(2),
        digest(0xa4),
        digest(3),
        digest(7),
        CounterEvent::Add(4),
    );

    assert_eq!(
        verify_authority_replay(
            &CounterReplay,
            AuthorityReplaySnapshot::new(federation(), atom(), epoch(), 1, digest(4), 3),
            target,
            &[event],
        ),
        Err(AuthorityReplayRefusal::SnapshotStateMismatch)
    );

    let broken_link = AuthorityReplayEvent::new(
        sequence(2),
        digest(0xa4),
        digest(4),
        digest(7),
        CounterEvent::Add(4),
    );
    assert_eq!(
        verify_authority_replay(
            &CounterReplay,
            AuthorityReplaySnapshot::new(federation(), atom(), epoch(), 1, digest(3), 3),
            target,
            &[broken_link],
        ),
        Err(AuthorityReplayRefusal::PriorStateMismatch { sequence: 2 })
    );
}
