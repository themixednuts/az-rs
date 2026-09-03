use std::sync::Arc;

use az_federation::{
    AuthorityAtomId, AuthorityEpoch, AuthorityRecoveryManifest, AuthorityRecoveryStatement,
    AuthorityRecoveryTarget, AuthorityReplayPolicy, AuthorityReplaySnapshot, AuthorityReplayTarget,
    ContentDigest, FederationId, HistoryCheckpointRef, MAX_ROOT_RECOVERY_KEYS, OperationId,
    RecoveryReplayAuthorizationError, RootRecoveryKeyPolicy, RootRecoveryPolicyError,
    RootRecoveryQuorum, RootRecoveryRefusal, VerifiedAuthorityReplay,
    authorize_manifest_recovery_replay, authorize_root_recovery, verify_authority_replay,
};
use az_federation_crypto::{
    MessageDigest, OpenSigningFuture, ProviderSignature, PublicKeyId, RawSignature, RootRecovery,
    ScopedDigest, Signer, SigningFuture, SigningPort, VerificationFuture, VerificationPort,
    VerificationRequest, VerifiedSignature, Verifier,
};

struct RecoverySigner(PublicKeyId);

impl SigningPort<RootRecovery> for RecoverySigner {
    fn open(&self) -> OpenSigningFuture<'_> {
        let key = self.0;
        Box::pin(async move { Ok(key) })
    }

    fn sign(&self, _: ScopedDigest<RootRecovery>) -> SigningFuture<'_> {
        let key = self.0;
        Box::pin(async move {
            Ok(ProviderSignature::new(
                key,
                RawSignature::Ed25519V1([key.as_bytes()[0]; 64]),
            ))
        })
    }
}

struct AcceptingVerifier;

impl VerificationPort for AcceptingVerifier {
    fn verify(&self, _: VerificationRequest) -> VerificationFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}

const fn key(byte: u8) -> PublicKeyId {
    PublicKeyId::from_bytes([byte; 32])
}

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}

const fn checkpoint(size: u64, byte: u8) -> HistoryCheckpointRef {
    HistoryCheckpointRef::new(size, digest(byte))
}

fn statement(policy_digest: ContentDigest) -> AuthorityRecoveryStatement {
    AuthorityRecoveryStatement::new(
        FederationId::from_digest(digest(0x10)),
        AuthorityEpoch::try_from(7).expect("current epoch"),
        AuthorityEpoch::try_from(8).expect("successor epoch"),
        HistoryCheckpointRef::new(41, digest(0x21)),
        HistoryCheckpointRef::new(41, digest(0x22)),
        HistoryCheckpointRef::new(39, digest(0x23)),
        digest(0x31),
        policy_digest,
        digest(0x33),
    )
    .expect("recovery statement")
}

fn verified(key: PublicKeyId, message: MessageDigest) -> VerifiedSignature<RootRecovery> {
    let signer = futures::executor::block_on(Signer::open(Arc::new(RecoverySigner(key))))
        .expect("open recovery signer");
    let signature = futures::executor::block_on(signer.sign(message)).expect("sign recovery");
    futures::executor::block_on(
        Verifier::new(Arc::new(AcceptingVerifier)).verify(message, signature),
    )
    .expect("verify recovery signature")
}

#[test]
fn threshold_of_distinct_authorized_root_keys_approves_one_digest() {
    let message = MessageDigest::from_bytes([0x42; 32]);
    let policy = RootRecoveryKeyPolicy::new(digest(0x91), 2, &[key(0x11), key(0x12), key(0x13)])
        .expect("recovery key policy");

    let recovery = statement(digest(0x91));
    let quorum = authorize_root_recovery(
        &policy,
        recovery,
        message,
        &[verified(key(0x11), message), verified(key(0x12), message)],
    )
    .expect("root recovery quorum");

    assert_eq!(quorum.message_digest(), message);
    assert_eq!(quorum.policy_digest(), digest(0x91));
    assert_eq!(quorum.approval_count(), 2);
    assert_eq!(quorum.statement(), recovery);
}

#[test]
fn quorum_cannot_authorize_a_statement_naming_another_policy() {
    let message = MessageDigest::from_bytes([0x42; 32]);
    let policy =
        RootRecoveryKeyPolicy::new(digest(0x91), 1, &[key(0x11)]).expect("recovery key policy");

    let result = authorize_root_recovery(
        &policy,
        statement(digest(0x92)),
        message,
        &[verified(key(0x11), message)],
    );

    assert_eq!(result, Err(RootRecoveryRefusal::PolicyMismatch));
}

#[test]
fn one_root_key_cannot_count_twice_toward_threshold() {
    let message = MessageDigest::from_bytes([0x42; 32]);
    let policy = RootRecoveryKeyPolicy::new(digest(0x91), 2, &[key(0x11), key(0x12)])
        .expect("recovery key policy");
    let approval = verified(key(0x11), message);

    let result = authorize_root_recovery(
        &policy,
        statement(digest(0x91)),
        message,
        &[approval, approval],
    );

    assert_eq!(result, Err(RootRecoveryRefusal::DuplicateApproval));
}

#[test]
fn valid_signature_from_an_unauthorized_root_cannot_count() {
    let message = MessageDigest::from_bytes([0x42; 32]);
    let policy =
        RootRecoveryKeyPolicy::new(digest(0x91), 1, &[key(0x11)]).expect("recovery key policy");

    let result = authorize_root_recovery(
        &policy,
        statement(digest(0x91)),
        message,
        &[verified(key(0x12), message)],
    );

    assert_eq!(result, Err(RootRecoveryRefusal::UnauthorizedKey));
}

#[test]
fn signatures_for_another_recovery_statement_cannot_count() {
    let message = MessageDigest::from_bytes([0x42; 32]);
    let other_message = MessageDigest::from_bytes([0x43; 32]);
    let policy =
        RootRecoveryKeyPolicy::new(digest(0x91), 1, &[key(0x11)]).expect("recovery key policy");

    let result = authorize_root_recovery(
        &policy,
        statement(digest(0x91)),
        message,
        &[verified(key(0x11), other_message)],
    );

    assert_eq!(result, Err(RootRecoveryRefusal::MessageMismatch));
}

#[test]
fn recovery_policy_rejects_impossible_or_duplicate_custody() {
    assert_eq!(
        RootRecoveryKeyPolicy::new(digest(0x91), 0, &[key(0x11)]),
        Err(RootRecoveryPolicyError::ZeroThreshold)
    );
    assert_eq!(
        RootRecoveryKeyPolicy::new(digest(0x91), 2, &[key(0x11)]),
        Err(RootRecoveryPolicyError::ThresholdExceedsKeyCount {
            threshold: 2,
            key_count: 1,
        })
    );
    assert_eq!(
        RootRecoveryKeyPolicy::new(digest(0x91), 1, &[key(0x11), key(0x11)]),
        Err(RootRecoveryPolicyError::DuplicateKey)
    );
    assert_eq!(
        RootRecoveryKeyPolicy::new(digest(0x91), 1, &[key(0x11); MAX_ROOT_RECOVERY_KEYS + 1],),
        Err(RootRecoveryPolicyError::TooManyKeys {
            actual: MAX_ROOT_RECOVERY_KEYS + 1,
            maximum: MAX_ROOT_RECOVERY_KEYS,
        })
    );
}

struct IdentityReplay;

impl AuthorityReplayPolicy for IdentityReplay {
    type Event = ();
    type Rejection = ();
    type State = u8;

    fn state_digest(&self, state: &Self::State) -> ContentDigest {
        digest(*state)
    }

    fn event_digest(&self, (): &Self::Event) -> ContentDigest {
        digest(0)
    }

    fn apply(&self, _: &mut Self::State, (): &Self::Event) -> Result<(), Self::Rejection> {
        Ok(())
    }
}

fn quorum(recovery: &AuthorityRecoveryStatement) -> RootRecoveryQuorum {
    let message = MessageDigest::from_bytes([0x42; 32]);
    let policy = RootRecoveryKeyPolicy::new(recovery.recovery_policy(), 1, &[key(0x11)])
        .expect("recovery key policy");
    authorize_root_recovery(&policy, *recovery, message, &[verified(key(0x11), message)])
        .expect("root recovery quorum")
}

fn replay(
    federation: FederationId,
    epoch: AuthorityEpoch,
    selected_checkpoint: HistoryCheckpointRef,
) -> VerifiedAuthorityReplay<u8> {
    verify_authority_replay(
        &IdentityReplay,
        AuthorityReplaySnapshot::new(
            federation,
            AuthorityAtomId::from_digest(digest(0x41)),
            epoch,
            0,
            digest(7),
            7,
        ),
        AuthorityReplayTarget::new(
            federation,
            AuthorityAtomId::from_digest(digest(0x41)),
            epoch,
            selected_checkpoint,
            0,
            digest(7),
        ),
        &[],
    )
    .expect("verified replay")
}

fn recovery_manifest(
    recovery: &AuthorityRecoveryStatement,
    targets: impl Into<Box<[AuthorityRecoveryTarget]>>,
) -> AuthorityRecoveryManifest {
    AuthorityRecoveryManifest::new(
        recovery.federation(),
        recovery.current_epoch(),
        recovery.successor_epoch(),
        recovery.selected_checkpoint(),
        OperationId::try_from_uuid(uuid::Uuid::from_u128(1)).expect("recovery operation"),
        digest(0x51),
        digest(0x52),
        targets.into(),
    )
    .expect("recovery manifest")
}

const fn replay_target() -> AuthorityRecoveryTarget {
    AuthorityRecoveryTarget::new(AuthorityAtomId::from_digest(digest(0x41)), 0, digest(7))
}

#[test]
fn root_quorum_authorizes_only_its_selected_replay_lineage() {
    let recovery = statement(digest(0x91));
    let quorum = quorum(&recovery);
    let manifest = recovery_manifest(&recovery, [replay_target()]);
    let replay = replay(
        recovery.federation(),
        recovery.current_epoch(),
        recovery.selected_checkpoint(),
    );

    let authorized =
        authorize_manifest_recovery_replay(quorum, &manifest, recovery.recovery_manifest(), replay)
            .expect("authorized atom recovery");

    assert_eq!(authorized.successor_epoch(), recovery.successor_epoch());
    assert_eq!(authorized.state(), &7);
}

#[test]
fn replay_lineage_must_match_federation_epoch_and_selected_checkpoint() {
    let recovery = statement(digest(0x91));
    let manifest = recovery_manifest(&recovery, [replay_target()]);
    let other_federation = FederationId::from_digest(digest(0x19));
    let other_epoch = AuthorityEpoch::try_from(6).expect("other epoch");
    let other_checkpoint = checkpoint(38, 0x28);

    assert_eq!(
        authorize_manifest_recovery_replay(
            quorum(&recovery),
            &manifest,
            recovery.recovery_manifest(),
            replay(
                other_federation,
                recovery.current_epoch(),
                recovery.selected_checkpoint(),
            ),
        ),
        Err(RecoveryReplayAuthorizationError::WrongFederation)
    );
    assert_eq!(
        authorize_manifest_recovery_replay(
            quorum(&recovery),
            &manifest,
            recovery.recovery_manifest(),
            replay(
                recovery.federation(),
                other_epoch,
                recovery.selected_checkpoint(),
            ),
        ),
        Err(RecoveryReplayAuthorizationError::WrongCurrentEpoch)
    );
    assert_eq!(
        authorize_manifest_recovery_replay(
            quorum(&recovery),
            &manifest,
            recovery.recovery_manifest(),
            replay(
                recovery.federation(),
                recovery.current_epoch(),
                other_checkpoint,
            ),
        ),
        Err(RecoveryReplayAuthorizationError::WrongSelectedCheckpoint)
    );
}

#[test]
fn root_quorum_authorizes_only_the_exact_manifest_and_atom_target() {
    let recovery = statement(digest(0x91));
    let replay = || {
        replay(
            recovery.federation(),
            recovery.current_epoch(),
            recovery.selected_checkpoint(),
        )
    };
    let manifest = recovery_manifest(&recovery, [replay_target()]);

    assert_eq!(
        authorize_manifest_recovery_replay(quorum(&recovery), &manifest, digest(0x34), replay(),),
        Err(RecoveryReplayAuthorizationError::WrongManifestDigest)
    );

    let wrong_lineage = AuthorityRecoveryManifest::new(
        FederationId::from_digest(digest(0x19)),
        recovery.current_epoch(),
        recovery.successor_epoch(),
        recovery.selected_checkpoint(),
        manifest.operation(),
        manifest.impact_inventory(),
        manifest.history_amendment(),
        [replay_target()].into(),
    )
    .expect("other-lineage manifest");
    assert_eq!(
        authorize_manifest_recovery_replay(
            quorum(&recovery),
            &wrong_lineage,
            recovery.recovery_manifest(),
            replay(),
        ),
        Err(RecoveryReplayAuthorizationError::WrongManifestLineage)
    );

    let missing_atom = recovery_manifest(
        &recovery,
        [AuthorityRecoveryTarget::new(
            AuthorityAtomId::from_digest(digest(0x42)),
            0,
            digest(7),
        )],
    );
    assert_eq!(
        authorize_manifest_recovery_replay(
            quorum(&recovery),
            &missing_atom,
            recovery.recovery_manifest(),
            replay(),
        ),
        Err(RecoveryReplayAuthorizationError::AtomNotInManifest)
    );

    let wrong_target = recovery_manifest(
        &recovery,
        [AuthorityRecoveryTarget::new(
            AuthorityAtomId::from_digest(digest(0x41)),
            1,
            digest(8),
        )],
    );
    assert_eq!(
        authorize_manifest_recovery_replay(
            quorum(&recovery),
            &wrong_target,
            recovery.recovery_manifest(),
            replay(),
        ),
        Err(RecoveryReplayAuthorizationError::WrongManifestTarget)
    );
}
