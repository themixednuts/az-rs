//! Federation consumes ADR 0051 durable and ADR 0052 lifecycle semantics
//! through ports it owns, so neither substrate's types enter its surface.

use az_federation::{
    ActivePolicyRef, AuthorityAtomCommit, AuthorityAtomId, AuthorityAtomPort,
    AuthorityAtomSnapshot, AuthorityEpoch, AuthorityOperationLookupFuture,
    AuthorityOperationLookupPort, AuthorityReceipt, CanonicalRequestDigest, CapabilityGrantRef,
    CertifiedExecutionBinding, ContentDigest, FederationId, FederationPrincipalId, GrantRevision,
    InMemoryAuthorityAtom, InstanceHostId, OperationId, OperatorId, PlacementAuthorityPort,
    PlacementAuthorityUnavailable, PlacementCurrency, PlacementFenceFuture, PolicyRevision,
    RevocationCursor, RulesetBinding, RulesetNamespace, RuntimeReleaseBinding, StorageFailure,
    UncertainCommit, reconcile_uncertain_commit, verify_placement_currency,
};
use az_world_instance::{PlacementFence, PlacementGeneration, WorldInstanceId};
use uuid::Uuid;

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}

fn world_instance() -> WorldInstanceId {
    WorldInstanceId::try_from_uuid(Uuid::from_bytes([0x13; 16])).expect("world instance")
}

fn binding_with(placement_generation: u64) -> CertifiedExecutionBinding {
    CertifiedExecutionBinding::new(
        FederationId::from_digest(digest(0x10)),
        AuthorityEpoch::try_from(3).expect("epoch"),
        OperatorId::from_digest(digest(0x11)),
        InstanceHostId::from_digest(digest(0x12)),
        PlacementFence::new(
            world_instance(),
            PlacementGeneration::try_from(placement_generation).expect("generation"),
        ),
        CapabilityGrantRef::new(digest(0x14), GrantRevision::try_from(5).expect("revision")),
        RuntimeReleaseBinding::new(digest(0x15)),
        RulesetBinding::new(
            RulesetNamespace::parse("example/open-world").expect("ruleset namespace"),
            digest(0x16),
        ),
        ActivePolicyRef::new(digest(0x17), PolicyRevision::try_from(9).expect("revision")),
        RevocationCursor::new(11),
    )
}

fn commit_with(operation_byte: u8, request_byte: u8) -> AuthorityAtomCommit {
    AuthorityAtomCommit::new(
        AuthorityAtomId::from_digest(digest(0x20)),
        OperationId::try_from_uuid(Uuid::from_bytes([operation_byte; 16])).expect("operation"),
        FederationPrincipalId::from_digest(digest(0x21)),
        binding_with(7),
        0,
        CanonicalRequestDigest::from_bytes([request_byte; 32]),
        digest(0x22),
        digest(0x23),
        digest(0x24),
    )
}

fn atom() -> InMemoryAuthorityAtom {
    InMemoryAuthorityAtom::new(AuthorityAtomSnapshot::genesis(
        AuthorityAtomId::from_digest(digest(0x20)),
        binding_with(7),
        digest(0x01),
    ))
}

/// A lookup adapter that is currently unable to answer.
struct UnavailableLookup;

impl AuthorityOperationLookupPort for UnavailableLookup {
    fn find_accepted_operation(
        &self,
        _atom: AuthorityAtomId,
        _operation: OperationId,
    ) -> AuthorityOperationLookupFuture<'_> {
        Box::pin(async { Err(StorageFailure::Unavailable) })
    }
}

/// A lifecycle owner fake: it answers one fact and exposes no lifecycle verbs.
struct FakePlacementAuthority {
    current: Option<PlacementFence>,
    available: bool,
}

impl PlacementAuthorityPort for FakePlacementAuthority {
    fn current_placement_fence(&self, world_instance: WorldInstanceId) -> PlacementFenceFuture<'_> {
        let answer = self
            .current
            .filter(|fence| fence.world_instance() == world_instance);
        let available = self.available;
        Box::pin(async move {
            if available {
                Ok(answer)
            } else {
                Err(PlacementAuthorityUnavailable)
            }
        })
    }
}

#[test]
fn an_uncertain_commit_reconciles_to_the_original_receipt() {
    let atom = atom();
    let request = commit_with(0x31, 0x41);
    let accepted = futures::executor::block_on(atom.commit(&request)).expect("commit");

    let reconciled =
        futures::executor::block_on(reconcile_uncertain_commit(&atom, &request)).expect("lookup");

    let UncertainCommit::Accepted(receipt) = reconciled else {
        panic!("an accepted operation must reconcile to its original receipt: {reconciled:?}");
    };
    assert_eq!(
        Some(receipt.as_ref()),
        match &accepted {
            az_federation::AuthorityAtomCommitResult::Committed(original) => Some(original),
            _ => None,
        },
        "reconciliation must return the original receipt, never a new decision"
    );
}

#[test]
fn the_same_operation_with_changed_bytes_reconciles_to_a_conflict() {
    let atom = atom();
    let request = commit_with(0x31, 0x41);
    futures::executor::block_on(atom.commit(&request)).expect("commit");

    let changed = commit_with(0x31, 0x42);
    let reconciled =
        futures::executor::block_on(reconcile_uncertain_commit(&atom, &changed)).expect("lookup");

    assert!(
        matches!(reconciled, UncertainCommit::Conflicting),
        "changed canonical bytes must never adopt an earlier receipt: {reconciled:?}"
    );
}

#[test]
fn an_unrecorded_operation_stays_retryable_under_its_original_identity() {
    let atom = atom();
    let request = commit_with(0x31, 0x41);

    let reconciled =
        futures::executor::block_on(reconcile_uncertain_commit(&atom, &request)).expect("lookup");

    assert!(
        matches!(reconciled, UncertainCommit::Unrecorded),
        "an absent record must leave the original operation retryable: {reconciled:?}"
    );
}

#[test]
fn a_lookup_addressed_to_another_atom_reports_unrecorded() {
    let atom = atom();
    let request = commit_with(0x31, 0x41);
    futures::executor::block_on(atom.commit(&request)).expect("commit");

    let elsewhere = futures::executor::block_on(atom.find_accepted_operation(
        AuthorityAtomId::from_digest(digest(0x99)),
        request.operation(),
    ))
    .expect("lookup");

    assert_eq!(
        elsewhere, None,
        "one atom's adapter must not answer for another atom's operations"
    );
}

#[test]
fn lookup_unavailability_stays_a_typed_storage_failure() {
    let request = commit_with(0x31, 0x41);

    let failure =
        futures::executor::block_on(reconcile_uncertain_commit(&UnavailableLookup, &request))
            .expect_err("unavailable lookup");

    assert_eq!(failure, StorageFailure::Unavailable);
    assert!(
        failure.is_unavailable(),
        "an unavailable lookup must stay retryable rather than demand recovery"
    );
}

#[test]
fn a_current_placement_generation_verifies_against_the_lifecycle_owner() {
    let placement = FakePlacementAuthority {
        current: Some(PlacementFence::new(
            world_instance(),
            PlacementGeneration::try_from(7).expect("generation"),
        )),
        available: true,
    };

    let currency =
        futures::executor::block_on(verify_placement_currency(&placement, &binding_with(7)))
            .expect("placement answer");

    assert!(
        matches!(currency, PlacementCurrency::Current),
        "a binding on the live generation must verify: {currency:?}"
    );
}

#[test]
fn a_superseded_placement_generation_reports_the_live_generation() {
    let placement = FakePlacementAuthority {
        current: Some(PlacementFence::new(
            world_instance(),
            PlacementGeneration::try_from(9).expect("generation"),
        )),
        available: true,
    };

    let currency =
        futures::executor::block_on(verify_placement_currency(&placement, &binding_with(7)))
            .expect("placement answer");

    assert_eq!(
        currency,
        PlacementCurrency::Superseded {
            current: PlacementGeneration::try_from(9).expect("generation")
        },
        "a stale binding must learn the live generation without a lifecycle call"
    );
}

#[test]
fn an_instance_with_no_current_placement_reports_unplaced() {
    let placement = FakePlacementAuthority {
        current: None,
        available: true,
    };

    let currency =
        futures::executor::block_on(verify_placement_currency(&placement, &binding_with(7)))
            .expect("placement answer");

    assert_eq!(currency, PlacementCurrency::Unplaced);
}

#[test]
fn lifecycle_unavailability_never_becomes_a_placement_decision() {
    let placement = FakePlacementAuthority {
        current: None,
        available: false,
    };

    assert_eq!(
        futures::executor::block_on(verify_placement_currency(&placement, &binding_with(7)))
            .expect_err("unavailable lifecycle owner"),
        PlacementAuthorityUnavailable,
    );
}

#[test]
fn a_receipt_exposes_the_canonical_bytes_it_accepted() {
    let atom = atom();
    let request = commit_with(0x31, 0x41);
    futures::executor::block_on(atom.commit(&request)).expect("commit");

    let stored: Option<AuthorityReceipt> = futures::executor::block_on(
        atom.find_accepted_operation(request.atom(), request.operation()),
    )
    .expect("lookup");

    assert_eq!(
        stored.expect("accepted receipt").request_digest(),
        request.request_digest(),
        "reconciliation depends on the receipt naming its accepted canonical bytes"
    );
}
