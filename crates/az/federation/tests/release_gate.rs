use az_federation::{
    ActivePolicyRef, BehaviorEligibility, CertifiedReleaseProof, ContentDigest,
    PublisherReleaseAuthorization, PublisherReleaseAuthorizationId, ReleaseBehavior,
    ReleaseEligibilityDecision, ReleaseEligibilityDecisionId, ReleaseEligibilitySnapshot,
    ReleaseGateRefusal, RuntimeReleaseBinding,
};

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}

const fn release(byte: u8) -> RuntimeReleaseBinding {
    RuntimeReleaseBinding::new(digest(byte))
}

const fn authorization(release: RuntimeReleaseBinding) -> PublisherReleaseAuthorization {
    PublisherReleaseAuthorization::new(
        PublisherReleaseAuthorizationId::from_digest(digest(0x31)),
        release,
        digest(0x32),
    )
}

fn eligibility(
    release: RuntimeReleaseBinding,
    publisher: PublisherReleaseAuthorizationId,
    snapshot: ReleaseEligibilitySnapshot,
) -> ReleaseEligibilityDecision {
    ReleaseEligibilityDecision::active(
        ReleaseEligibilityDecisionId::from_digest(digest(0x41)),
        release,
        publisher,
        ActivePolicyRef::new(
            digest(0x42),
            az_federation::PolicyRevision::try_from(3).expect("policy revision"),
        ),
        snapshot,
    )
}

#[test]
fn publisher_authorization_and_federation_eligibility_both_bind_one_release() {
    let release = release(0x21);
    let publisher = authorization(release);
    let federation = eligibility(
        release,
        publisher.id(),
        ReleaseEligibilitySnapshot::uniform(BehaviorEligibility::Allowed),
    );

    let proof = CertifiedReleaseProof::compose(publisher, federation).expect("dual authority");
    let placement = proof
        .authorize(ReleaseBehavior::Placement)
        .expect("placement eligible");

    assert_eq!(placement.release(), release);
    assert_eq!(placement.eligibility(), BehaviorEligibility::Allowed);
}

#[test]
fn neither_authority_can_substitute_for_the_other() {
    let publisher = authorization(release(0x21));
    let wrong_release = eligibility(
        release(0x22),
        publisher.id(),
        ReleaseEligibilitySnapshot::uniform(BehaviorEligibility::Allowed),
    );
    assert_eq!(
        CertifiedReleaseProof::compose(publisher, wrong_release),
        Err(ReleaseGateRefusal::ReleaseMismatch)
    );

    let publisher = authorization(release(0x21));
    let wrong_authorization = eligibility(
        release(0x21),
        PublisherReleaseAuthorizationId::from_digest(digest(0x33)),
        ReleaseEligibilitySnapshot::uniform(BehaviorEligibility::Allowed),
    );
    assert_eq!(
        CertifiedReleaseProof::compose(publisher, wrong_authorization),
        Err(ReleaseGateRefusal::PublisherAuthorizationMismatch)
    );
}

#[test]
fn eligibility_is_behavior_specific_and_retirement_closes_every_behavior() {
    let publisher = authorization(release(0x21));
    let snapshot = ReleaseEligibilitySnapshot::new(
        BehaviorEligibility::Denied,
        BehaviorEligibility::Denied,
        BehaviorEligibility::Denied,
        BehaviorEligibility::Denied,
        BehaviorEligibility::Allowed,
        BehaviorEligibility::Denied,
        BehaviorEligibility::Allowed,
    );
    let proof = CertifiedReleaseProof::compose(
        publisher,
        eligibility(release(0x21), publisher.id(), snapshot),
    )
    .expect("dual authority");

    assert_eq!(
        proof.authorize(ReleaseBehavior::PortableMutation),
        Err(ReleaseGateRefusal::BehaviorDenied {
            behavior: ReleaseBehavior::PortableMutation,
        })
    );
    assert!(proof.authorize(ReleaseBehavior::Closeout).is_ok());

    let retired = ReleaseEligibilityDecision::retired(
        ReleaseEligibilityDecisionId::from_digest(digest(0x43)),
        release(0x21),
        publisher.id(),
        ActivePolicyRef::new(
            digest(0x42),
            az_federation::PolicyRevision::try_from(4).expect("policy revision"),
        ),
    );
    let retired_proof = CertifiedReleaseProof::compose(publisher, retired).expect("dual authority");
    for behavior in ReleaseBehavior::ALL {
        assert!(matches!(
            retired_proof.authorize(behavior),
            Err(ReleaseGateRefusal::ReleaseRetired)
        ));
    }
}
