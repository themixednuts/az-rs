use std::sync::Arc;

use az_federation_crypto::{
    CommitmentNonce, ForkReason, HistoryLog, HistoryWitnessMonitor, MerkleHistory,
    MonitorCheckpointOutcome, MonitorError, OpenSigningFuture, ProviderSignature, PublicKeyId,
    RawSignature, ReceiptCommitment, ReceiptDigest, ScopedDigest, Signer, SigningFuture,
    SigningPort, VerificationFuture, VerificationPort, VerificationRequest, Verifier,
    sign_history_checkpoint, verify_history_checkpoint,
};

#[derive(Clone, Copy)]
struct KeySigner(PublicKeyId);

impl SigningPort<HistoryLog> for KeySigner {
    fn open(&self) -> OpenSigningFuture<'_> {
        let key = self.0;
        Box::pin(async move { Ok(key) })
    }

    fn sign(&self, _: ScopedDigest<HistoryLog>) -> SigningFuture<'_> {
        let key = self.0;
        Box::pin(async move {
            Ok(ProviderSignature::new(
                key,
                RawSignature::Ed25519V1([0x5a; 64]),
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

fn history(bytes: &[u8]) -> MerkleHistory {
    let mut history = MerkleHistory::new();
    for byte in bytes {
        history
            .append(ReceiptCommitment::new(
                CommitmentNonce::from_bytes([byte.wrapping_add(0x40); 32]),
                ReceiptDigest::from_bytes([*byte; 32]),
            ))
            .expect("append");
    }
    history
}

fn verified(
    checkpoint: az_federation_crypto::HistoryCheckpoint,
    key: PublicKeyId,
) -> az_federation_crypto::VerifiedHistoryCheckpoint<HistoryLog> {
    let log_signer =
        futures::executor::block_on(Signer::open(Arc::new(KeySigner(key)))).expect("open signer");
    let signed_checkpoint =
        futures::executor::block_on(sign_history_checkpoint(checkpoint, &log_signer))
            .expect("sign checkpoint");
    futures::executor::block_on(verify_history_checkpoint(
        &Verifier::new(Arc::new(AcceptingVerifier)),
        signed_checkpoint,
    ))
    .expect("verify checkpoint")
}

#[test]
fn a_verified_consistent_checkpoint_advances_the_witness() {
    let key = PublicKeyId::from_bytes([0x31; 32]);
    let history = history(&[1, 2, 3, 4]);
    let trusted = verified(history.checkpoint_at(2).expect("trusted"), key);
    let mut monitor = HistoryWitnessMonitor::new(key, trusted).expect("monitor");
    let candidate = verified(history.checkpoint(), key);
    let proof = history.consistency_proof(2, 4).expect("proof");

    assert_eq!(
        monitor.observe(candidate, &proof).expect("advance"),
        MonitorCheckpointOutcome::Advanced(candidate)
    );
    assert_eq!(monitor.current(), candidate);

    let same = history.consistency_proof(4, 4).expect("same proof");
    assert_eq!(
        monitor.observe(candidate, &same).expect("existing"),
        MonitorCheckpointOutcome::Existing(candidate)
    );
}

#[test]
fn conflicting_signed_root_at_the_same_size_freezes_the_witness() {
    let key = PublicKeyId::from_bytes([0x31; 32]);
    let honest = history(&[1, 2, 3]);
    let fork = history(&[1, 2, 9]);
    let trusted = verified(honest.checkpoint(), key);
    let conflicting = verified(fork.checkpoint(), key);
    let proof = honest.consistency_proof(3, 3).expect("proof");
    let mut monitor = HistoryWitnessMonitor::new(key, trusted).expect("monitor");

    let failure = monitor
        .observe(conflicting, &proof)
        .expect_err("split view must freeze");
    let evidence = match failure {
        MonitorError::ForkDetected(evidence) => *evidence,
        other => panic!("expected fork evidence, got {other:?}"),
    };
    assert_eq!(evidence.reason(), ForkReason::ConflictingRootAtSameSize);
    assert_eq!(evidence.trusted(), trusted);
    assert_eq!(evidence.observed(), conflicting);

    let later = history(&[1, 2, 3, 4]);
    let candidate = verified(later.checkpoint(), key);
    let later_proof = later.consistency_proof(3, 4).expect("proof");
    assert_eq!(
        monitor.observe(candidate, &later_proof),
        Err(MonitorError::Frozen(Box::new(evidence)))
    );
}

#[test]
fn invalid_append_only_proof_freezes_and_wrong_key_only_refuses() {
    let key = PublicKeyId::from_bytes([0x31; 32]);
    let trusted_history = history(&[1, 2]);
    let trusted = verified(trusted_history.checkpoint(), key);
    let mut monitor = HistoryWitnessMonitor::new(key, trusted).expect("monitor");

    let honest = history(&[1, 2, 3]);
    let wrong_key_candidate = verified(honest.checkpoint(), PublicKeyId::from_bytes([0x32; 32]));
    let honest_proof = honest.consistency_proof(2, 3).expect("proof");
    assert!(matches!(
        monitor.observe(wrong_key_candidate, &honest_proof),
        Err(MonitorError::UnexpectedLogKey { .. })
    ));
    assert_eq!(monitor.current(), trusted);

    let fork = history(&[1, 9, 3]);
    let conflicting = verified(fork.checkpoint(), key);
    let unrelated_proof = honest.consistency_proof(2, 3).expect("proof");
    let failure = monitor
        .observe(conflicting, &unrelated_proof)
        .expect_err("invalid consistency must freeze");
    assert!(matches!(
        failure,
        MonitorError::ForkDetected(ref evidence)
            if evidence.reason() == ForkReason::InvalidConsistencyProof
    ));
}
