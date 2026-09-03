use std::sync::{Arc, Mutex};

use az_federation::{
    ActivePolicyRef, AuthorityTime, CertifiedHostBinding, ChallengeVerificationFailure,
    ChallengeVerificationFuture, ChallengeVerificationPort, ContentDigest, EvidenceSnapshotId,
    HostChallengeId, HostChallengeNonce, HostChallengePlan, HostChallengeRefusal,
    HostChallengeSubmission, InstanceHostId, OperatorId, PolicyRevision, PortableMutationCeiling,
    PortableMutationClass, PortableMutationClasses, PortableMutationGrantRequest, RulesetBinding,
    RulesetNamespace, RuntimeReleaseBinding, verify_host_challenge,
};

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}

fn request() -> PortableMutationGrantRequest {
    PortableMutationGrantRequest::new(
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
        ),
        PortableMutationCeiling::new(
            PortableMutationClasses::only(PortableMutationClass::Transfer),
            std::num::NonZeroU32::new(10).expect("limit"),
            0,
        )
        .expect("ceiling"),
        EvidenceSnapshotId::from_digest(digest(0x20)),
        ActivePolicyRef::new(
            digest(0x21),
            PolicyRevision::try_from(4).expect("policy revision"),
        ),
    )
}

fn plan() -> HostChallengePlan {
    HostChallengePlan::new(
        HostChallengeId::from_digest(digest(0x30)),
        request(),
        HostChallengeNonce::from_bytes([0x31; 32]),
        AuthorityTime::from_unix_micros(1_000),
        AuthorityTime::from_unix_micros(2_000),
    )
    .expect("challenge plan")
}

#[derive(Default)]
struct RecordingVerifier {
    calls: Mutex<usize>,
}

impl ChallengeVerificationPort for RecordingVerifier {
    fn verify(
        &self,
        _: &HostChallengePlan,
        _: &HostChallengeSubmission,
    ) -> ChallengeVerificationFuture<'_> {
        *self.calls.lock().expect("calls lock") += 1;
        Box::pin(async { Ok(()) })
    }
}

#[test]
fn fresh_matching_challenge_produces_the_only_grant_input() {
    let plan = plan();
    let adapter = Arc::new(RecordingVerifier::default());
    let submission = HostChallengeSubmission::new(
        plan.id(),
        plan.nonce(),
        EvidenceSnapshotId::from_digest(digest(0x20)),
        digest(0x32),
    );

    let verified_challenge = futures::executor::block_on(verify_host_challenge(
        plan,
        submission,
        AuthorityTime::from_unix_micros(1_500),
        adapter.clone(),
    ))
    .expect("verified challenge");

    assert_eq!(verified_challenge.request(), &request());
    assert_eq!(*adapter.calls.lock().expect("calls lock"), 1);
}

#[test]
fn mismatched_or_expired_challenges_fail_before_provider_verification() {
    let cases = [
        (
            HostChallengeSubmission::new(
                HostChallengeId::from_digest(digest(0x39)),
                HostChallengeNonce::from_bytes([0x31; 32]),
                EvidenceSnapshotId::from_digest(digest(0x20)),
                digest(0x32),
            ),
            AuthorityTime::from_unix_micros(1_500),
            HostChallengeRefusal::WrongChallenge,
        ),
        (
            HostChallengeSubmission::new(
                HostChallengeId::from_digest(digest(0x30)),
                HostChallengeNonce::from_bytes([0x39; 32]),
                EvidenceSnapshotId::from_digest(digest(0x20)),
                digest(0x32),
            ),
            AuthorityTime::from_unix_micros(1_500),
            HostChallengeRefusal::WrongNonce,
        ),
        (
            HostChallengeSubmission::new(
                HostChallengeId::from_digest(digest(0x30)),
                HostChallengeNonce::from_bytes([0x31; 32]),
                EvidenceSnapshotId::from_digest(digest(0x29)),
                digest(0x32),
            ),
            AuthorityTime::from_unix_micros(1_500),
            HostChallengeRefusal::WrongEvidenceSnapshot,
        ),
        (
            HostChallengeSubmission::new(
                HostChallengeId::from_digest(digest(0x30)),
                HostChallengeNonce::from_bytes([0x31; 32]),
                EvidenceSnapshotId::from_digest(digest(0x20)),
                digest(0x32),
            ),
            AuthorityTime::from_unix_micros(2_000),
            HostChallengeRefusal::Expired,
        ),
    ];

    for (submission, now, expected) in cases {
        let adapter = Arc::new(RecordingVerifier::default());
        let failure = futures::executor::block_on(verify_host_challenge(
            plan(),
            submission,
            now,
            adapter.clone(),
        ))
        .expect_err("challenge must fail closed");
        assert_eq!(failure, ChallengeVerificationFailure::Refused(expected));
        assert_eq!(*adapter.calls.lock().expect("calls lock"), 0);
    }
}
