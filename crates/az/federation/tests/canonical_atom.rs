use std::sync::{Arc, Barrier};

use az_federation::{
    ActivePolicyRef, AuthorityAtomCommit, AuthorityAtomCommitResult, AuthorityAtomId,
    AuthorityAtomPort, AuthorityAtomSnapshot, AuthorityEpoch, CanonicalRequestDigest,
    CapabilityGrantRef, CommitRefusal, ContentDigest, FederationId, FederationPrincipalId,
    GrantRevision, InMemoryAuthorityAtom, InMemoryCommitFault, InstanceHostId, OperationId,
    OperatorId, PolicyRevision, PublicationOutcome, PublicationRefusal, PublicationStep,
    RevocationCursor, RulesetBinding, RulesetNamespace, RuntimeReleaseBinding,
    TransparencyOutboxPort,
};
use az_world_instance::{PlacementFence, PlacementGeneration, WorldInstanceId};
use uuid::Uuid;

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}

fn binding_with(
    placement_generation: u64,
    grant_revision: u64,
    revocation_cursor: u64,
) -> az_federation::CertifiedExecutionBinding {
    az_federation::CertifiedExecutionBinding::new(
        FederationId::from_digest(digest(0x10)),
        AuthorityEpoch::try_from(3).expect("epoch"),
        OperatorId::from_digest(digest(0x11)),
        InstanceHostId::from_digest(digest(0x12)),
        PlacementFence::new(
            WorldInstanceId::try_from_uuid(Uuid::from_bytes([0x13; 16])).expect("world instance"),
            PlacementGeneration::try_from(placement_generation).expect("generation"),
        ),
        CapabilityGrantRef::new(
            digest(0x14),
            GrantRevision::try_from(grant_revision).expect("grant revision"),
        ),
        RuntimeReleaseBinding::new(digest(0x15)),
        RulesetBinding::new(
            RulesetNamespace::parse("example/open-world").expect("ruleset namespace"),
            digest(0x16),
        ),
        ActivePolicyRef::new(
            digest(0x17),
            PolicyRevision::try_from(9).expect("policy revision"),
        ),
        RevocationCursor::new(revocation_cursor),
    )
}

fn binding() -> az_federation::CertifiedExecutionBinding {
    binding_with(7, 5, 11)
}

fn commit_with(
    operation_byte: u8,
    request_byte: u8,
    execution_binding: az_federation::CertifiedExecutionBinding,
    expected_state_version: u64,
) -> AuthorityAtomCommit {
    AuthorityAtomCommit::new(
        AuthorityAtomId::from_digest(digest(0x20)),
        OperationId::try_from_uuid(Uuid::from_bytes([operation_byte; 16])).expect("operation"),
        FederationPrincipalId::from_digest(digest(0x21)),
        execution_binding,
        expected_state_version,
        CanonicalRequestDigest::from_bytes([request_byte; 32]),
        digest(0x22),
        digest(0x23),
        digest(0x24),
    )
}

#[test]
fn nil_operation_identity_is_unrepresentable() {
    assert!(OperationId::try_from_uuid(Uuid::nil()).is_err());
}

#[test]
fn ruleset_deserialization_preserves_the_canonical_namespace_invariant() {
    assert!(serde_json::from_str::<RulesetNamespace>(r#""example/open-world""#).is_ok());
    assert!(serde_json::from_str::<RulesetNamespace>(r#""Example/OpenWorld""#).is_err());
}

fn commit(operation_byte: u8, request_byte: u8) -> AuthorityAtomCommit {
    commit_with(operation_byte, request_byte, binding(), 0)
}

fn atom() -> InMemoryAuthorityAtom {
    InMemoryAuthorityAtom::new(AuthorityAtomSnapshot::genesis(
        AuthorityAtomId::from_digest(digest(0x20)),
        binding(),
        digest(0x01),
    ))
}

#[test]
fn identical_retry_returns_existing_and_changed_bytes_conflict() {
    let atom = atom();
    let request = commit(0x31, 0x41);

    let first = futures::executor::block_on(atom.commit(&request)).expect("first commit");
    let retry = futures::executor::block_on(atom.commit(&request)).expect("retry");
    let changed = commit(0x31, 0x42);
    let conflict = futures::executor::block_on(atom.commit(&changed)).expect("conflict result");

    assert!(matches!(first, AuthorityAtomCommitResult::Committed(_)));
    assert!(matches!(retry, AuthorityAtomCommitResult::Existing(_)));
    assert!(matches!(
        conflict,
        AuthorityAtomCommitResult::OperationConflict { .. }
    ));
    assert_eq!(atom.snapshot().state_version(), 1);
    assert_eq!(atom.snapshot().transparency_outbox_len(), 1);
}

#[test]
fn stale_fence_grant_revocation_and_state_version_are_precise_refusals() {
    let cases = [
        (
            commit_with(0x32, 0x51, binding_with(6, 5, 11), 0),
            CommitRefusal::StalePlacementFence,
        ),
        (
            commit_with(0x33, 0x52, binding_with(7, 4, 11), 0),
            CommitRefusal::StaleGrantRevision,
        ),
        (
            commit_with(0x34, 0x53, binding_with(7, 5, 10), 0),
            CommitRefusal::StaleRevocationCursor,
        ),
        (
            commit_with(0x35, 0x54, binding(), 1),
            CommitRefusal::StaleStateVersion,
        ),
    ];

    for (request, expected) in cases {
        let result = futures::executor::block_on(atom().commit(&request)).expect("refusal result");
        assert_eq!(result, AuthorityAtomCommitResult::Rejected(expected));
    }
}

#[test]
fn injected_failure_at_every_staged_commit_point_leaves_no_half_commit() {
    for fault in InMemoryCommitFault::ALL {
        let atom = atom();
        atom.fail_next_commit_at(fault);

        let error = futures::executor::block_on(atom.commit(&commit(0x36, 0x61)))
            .expect_err("fault must abort commit");
        let snapshot = atom.snapshot();

        assert!(error.is_unavailable());
        assert_eq!(snapshot.state_version(), 0);
        assert_eq!(snapshot.operation_count(), 0);
        assert_eq!(snapshot.receipt_count(), 0);
        assert_eq!(snapshot.transparency_outbox_len(), 0);
    }
}

#[test]
fn concurrent_identical_requests_commit_once() {
    const CALLERS: usize = 8;

    let atom = Arc::new(atom());
    let barrier = Arc::new(Barrier::new(CALLERS));
    let request = Arc::new(commit(0x37, 0x62));
    let joins = std::array::from_fn::<_, CALLERS, _>(|_| {
        let atom = Arc::clone(&atom);
        let barrier = Arc::clone(&barrier);
        let request = Arc::clone(&request);
        std::thread::spawn(move || {
            barrier.wait();
            futures::executor::block_on(atom.commit(&request)).expect("commit result")
        })
    });
    let results: Vec<_> = joins
        .into_iter()
        .map(|join| join.join().expect("caller must not panic"))
        .collect();

    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, AuthorityAtomCommitResult::Committed(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, AuthorityAtomCommitResult::Existing(_)))
            .count(),
        CALLERS - 1
    );
    assert_eq!(atom.snapshot().state_version(), 1);
    assert_eq!(atom.snapshot().transparency_outbox_len(), 1);
}

#[test]
fn transparency_publication_is_ordered_idempotent_and_never_changes_the_receipt() {
    let atom = atom();
    let request = commit(0x38, 0x63);
    let committed = futures::executor::block_on(atom.commit(&request)).expect("commit");
    let AuthorityAtomCommitResult::Committed(receipt) = committed else {
        panic!("expected committed receipt");
    };
    let obligation = futures::executor::block_on(atom.load_publication(request.operation()))
        .expect("load publication")
        .expect("publication obligation");
    assert_eq!(obligation.commitment(), digest(0x24));

    assert_eq!(
        futures::executor::block_on(
            atom.record_publication(request.operation(), PublicationStep::Witness(digest(0x72)),)
        ),
        Ok(PublicationOutcome::Refused(
            PublicationRefusal::MissingLogInclusion
        ))
    );

    let inclusion = PublicationStep::LogInclusion(digest(0x71));
    assert!(matches!(
        futures::executor::block_on(atom.record_publication(request.operation(), inclusion)),
        Ok(PublicationOutcome::Advanced(_))
    ));
    assert!(matches!(
        futures::executor::block_on(atom.record_publication(request.operation(), inclusion)),
        Ok(PublicationOutcome::Existing(_))
    ));
    assert_eq!(
        futures::executor::block_on(atom.record_publication(
            request.operation(),
            PublicationStep::LogInclusion(digest(0x79)),
        )),
        Ok(PublicationOutcome::Refused(
            PublicationRefusal::EvidenceConflict
        ))
    );

    for step in [
        PublicationStep::Witness(digest(0x72)),
        PublicationStep::Anchor(digest(0x73)),
    ] {
        assert!(matches!(
            futures::executor::block_on(atom.record_publication(request.operation(), step)),
            Ok(PublicationOutcome::Advanced(_))
        ));
        assert!(matches!(
            futures::executor::block_on(atom.record_publication(request.operation(), step)),
            Ok(PublicationOutcome::Existing(_))
        ));
    }

    let snapshot = atom.snapshot();
    assert_eq!(snapshot.state_version(), receipt.state_version());
    assert_eq!(snapshot.receipt_count(), 1);
    assert_eq!(snapshot.transparency_outbox_len(), 0);
}
