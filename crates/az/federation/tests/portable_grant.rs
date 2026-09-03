use std::{num::NonZeroU32, sync::Arc};

use az_federation::{
    ActivePolicyRef, AuthorityTime, CapabilityGrantRef, CertifiedHostBinding,
    ChallengeVerificationFuture, ChallengeVerificationPort, ContentDigest, EvidenceSnapshotId,
    GrantRevision, GrantUseRefusal, HostChallengeId, HostChallengeNonce, HostChallengePlan,
    HostChallengeSubmission, InstanceHostId, OperatorId, PolicyRevision, PortableMutationCeiling,
    PortableMutationClass, PortableMutationClasses, PortableMutationGrant,
    PortableMutationGrantError, PortableMutationGrantRequest, PortableMutationGrantStanding,
    RevocationCursor, RulesetBinding, RulesetNamespace, RuntimeReleaseBinding,
    VerifiedHostChallenge, verify_host_challenge,
};

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}

fn subject() -> CertifiedHostBinding {
    CertifiedHostBinding::new(
        OperatorId::from_digest(digest(0x10)),
        InstanceHostId::from_digest(digest(0x11)),
        RuntimeReleaseBinding::new(digest(0x12)),
        RulesetBinding::new(
            RulesetNamespace::parse("example/open-world").expect("ruleset"),
            digest(0x13),
        ),
        digest(0x14),
        digest(0x15),
    )
}

fn ceiling(
    classes: PortableMutationClasses,
    per_window: u32,
    offline: u32,
) -> PortableMutationCeiling {
    PortableMutationCeiling::new(
        classes,
        NonZeroU32::new(per_window).expect("nonzero limit"),
        offline,
    )
    .expect("nonempty classes")
}

fn request(requested: PortableMutationCeiling) -> PortableMutationGrantRequest {
    PortableMutationGrantRequest::new(
        subject(),
        requested,
        EvidenceSnapshotId::from_digest(digest(0x20)),
        ActivePolicyRef::new(
            digest(0x21),
            PolicyRevision::try_from(4).expect("policy revision"),
        ),
    )
}

struct AcceptChallenge;

impl ChallengeVerificationPort for AcceptChallenge {
    fn verify<'a>(
        &'a self,
        _: &'a HostChallengePlan,
        _: &'a HostChallengeSubmission,
    ) -> ChallengeVerificationFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

fn verified(request: PortableMutationGrantRequest) -> VerifiedHostChallenge {
    let plan = HostChallengePlan::new(
        HostChallengeId::from_digest(digest(0x40)),
        request,
        HostChallengeNonce::from_bytes([0x41; 32]),
        AuthorityTime::from_unix_micros(1),
        AuthorityTime::from_unix_micros(3),
    )
    .expect("plan");
    let submission = HostChallengeSubmission::new(
        plan.id(),
        plan.nonce(),
        EvidenceSnapshotId::from_digest(digest(0x20)),
        digest(0x42),
    );
    futures::executor::block_on(verify_host_challenge(
        plan,
        submission,
        AuthorityTime::from_unix_micros(2),
        Arc::new(AcceptChallenge),
    ))
    .expect("verified challenge")
}

#[test]
fn grant_is_never_wider_than_request_evidence_or_operator_consent() {
    let requested = PortableMutationClasses::only(PortableMutationClass::Mint)
        .with(PortableMutationClass::Transfer);
    let federation = requested.with(PortableMutationClass::Adjust);
    let operator = PortableMutationClasses::only(PortableMutationClass::Transfer);

    let grant = PortableMutationGrant::issue(
        CapabilityGrantRef::new(
            digest(0x30),
            GrantRevision::try_from(2).expect("grant revision"),
        ),
        verified(request(ceiling(requested, 100, 10))),
        ceiling(federation, 80, 5),
        ceiling(operator, 50, 0),
        RevocationCursor::new(9),
    )
    .expect("narrow grant");

    assert_eq!(
        grant.grant(),
        CapabilityGrantRef::new(
            digest(0x30),
            GrantRevision::try_from(2).expect("grant revision")
        )
    );
    assert_eq!(grant.subject(), &subject());
    assert_eq!(grant.revocation_view(), RevocationCursor::new(9));
    assert_eq!(
        grant.ceiling().classes(),
        PortableMutationClasses::only(PortableMutationClass::Transfer)
    );
    assert_eq!(grant.ceiling().max_operations_per_window().get(), 50);
    assert_eq!(grant.ceiling().max_offline_operations(), 0);
}

#[test]
fn no_shared_capability_refuses_instead_of_minting_an_empty_grant() {
    let result = PortableMutationGrant::issue(
        CapabilityGrantRef::new(
            digest(0x30),
            GrantRevision::try_from(2).expect("grant revision"),
        ),
        verified(request(ceiling(
            PortableMutationClasses::only(PortableMutationClass::Mint),
            100,
            10,
        ))),
        ceiling(
            PortableMutationClasses::only(PortableMutationClass::Transfer),
            80,
            5,
        ),
        ceiling(
            PortableMutationClasses::only(PortableMutationClass::Transfer),
            50,
            0,
        ),
        RevocationCursor::new(9),
    );

    assert_eq!(result, Err(PortableMutationGrantError::NoSharedAuthority));
}

#[test]
fn stale_revocation_view_and_contraction_close_new_work() {
    let classes = PortableMutationClasses::only(PortableMutationClass::Transfer);
    let mut grant = PortableMutationGrant::issue(
        CapabilityGrantRef::new(
            digest(0x30),
            GrantRevision::try_from(2).expect("grant revision"),
        ),
        verified(request(ceiling(classes, 100, 10))),
        ceiling(classes, 80, 5),
        ceiling(classes, 50, 0),
        RevocationCursor::new(9),
    )
    .expect("grant");

    assert!(
        grant
            .authorize_new_work(PortableMutationClass::Transfer, RevocationCursor::new(9))
            .is_ok()
    );
    assert_eq!(
        grant.authorize_new_work(PortableMutationClass::Transfer, RevocationCursor::new(10)),
        Err(GrantUseRefusal::StaleRevocationView)
    );
    assert_eq!(
        grant.authorize_new_work(PortableMutationClass::Mint, RevocationCursor::new(9)),
        Err(GrantUseRefusal::CapabilityNotGranted)
    );

    grant
        .suspend(RevocationCursor::new(10))
        .expect("forward suspension");
    assert_eq!(grant.standing(), PortableMutationGrantStanding::Suspended);
    assert_eq!(
        grant.authorize_new_work(PortableMutationClass::Transfer, RevocationCursor::new(10)),
        Err(GrantUseRefusal::Suspended)
    );
    assert_eq!(
        grant.suspend(RevocationCursor::new(10)),
        Err(PortableMutationGrantError::NonForwardRevocation)
    );

    grant
        .revoke(RevocationCursor::new(11))
        .expect("forward revocation");
    assert_eq!(grant.standing(), PortableMutationGrantStanding::Revoked);
    assert_eq!(
        grant.authorize_new_work(PortableMutationClass::Transfer, RevocationCursor::new(11)),
        Err(GrantUseRefusal::Revoked)
    );
}
