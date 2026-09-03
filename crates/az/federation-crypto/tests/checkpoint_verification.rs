use std::sync::{Arc, Mutex};

use az_federation_crypto::{
    CommitmentNonce, HistoryLog, MerkleHistory, OpenSigningFuture, ProviderSignature, PublicKeyId,
    RawSignature, ReceiptCommitment, ReceiptDigest, ScopedDigest, SignatureRoleId, Signer,
    SigningFuture, SigningPort, VerificationFailure, VerificationFuture, VerificationPort,
    VerificationRequest, Verifier, sign_history_checkpoint, verify_history_checkpoint,
};

struct LogSigner;

impl SigningPort<HistoryLog> for LogSigner {
    fn open(&self) -> OpenSigningFuture<'_> {
        Box::pin(async { Ok(PublicKeyId::from_bytes([0x31; 32])) })
    }

    fn sign(&self, _: ScopedDigest<HistoryLog>) -> SigningFuture<'_> {
        Box::pin(async {
            Ok(ProviderSignature::new(
                PublicKeyId::from_bytes([0x31; 32]),
                RawSignature::Ed25519V1([0x5a; 64]),
            ))
        })
    }
}

struct RecordingVerifier {
    result: Result<(), VerificationFailure>,
    requests: Mutex<Vec<VerificationRequest>>,
}

impl RecordingVerifier {
    const fn accepting() -> Self {
        Self {
            result: Ok(()),
            requests: Mutex::new(Vec::new()),
        }
    }

    const fn rejecting(failure: VerificationFailure) -> Self {
        Self {
            result: Err(failure),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<VerificationRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

impl VerificationPort for RecordingVerifier {
    fn verify(&self, request: VerificationRequest) -> VerificationFuture<'_> {
        self.requests.lock().expect("request lock").push(request);
        let result = self.result.clone();
        Box::pin(async move { result })
    }
}

fn checkpoint() -> az_federation_crypto::HistoryCheckpoint {
    let mut history = MerkleHistory::new();
    history
        .append(ReceiptCommitment::new(
            CommitmentNonce::from_bytes([0x41; 32]),
            ReceiptDigest::from_bytes([0x01; 32]),
        ))
        .expect("append");
    history.checkpoint()
}

fn signed_checkpoint() -> az_federation_crypto::SignedHistoryCheckpoint<HistoryLog> {
    let signer =
        futures::executor::block_on(Signer::open(Arc::new(LogSigner))).expect("open log signer");
    futures::executor::block_on(sign_history_checkpoint(checkpoint(), &signer))
        .expect("sign checkpoint")
}

#[test]
fn verified_checkpoint_exists_only_after_crypto_verification() {
    let adapter = Arc::new(RecordingVerifier::accepting());
    let verifier = Verifier::new(adapter.clone());
    let signed = signed_checkpoint();

    let verified_checkpoint =
        futures::executor::block_on(verify_history_checkpoint(&verifier, signed))
            .expect("verify checkpoint");

    assert_eq!(verified_checkpoint.checkpoint(), checkpoint());
    assert_eq!(
        verified_checkpoint.public_key(),
        PublicKeyId::from_bytes([0x31; 32])
    );
    assert_eq!(verified_checkpoint.role(), SignatureRoleId::HistoryLog);
    let requests = adapter.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].role(), SignatureRoleId::HistoryLog);
    assert_eq!(
        requests[0].public_key(),
        PublicKeyId::from_bytes([0x31; 32])
    );
    assert_eq!(requests[0].raw(), RawSignature::Ed25519V1([0x5a; 64]));
}

#[test]
fn invalid_signature_never_produces_a_verified_checkpoint() {
    let verifier = Verifier::new(Arc::new(RecordingVerifier::rejecting(
        VerificationFailure::InvalidSignature,
    )));

    let failure =
        futures::executor::block_on(verify_history_checkpoint(&verifier, signed_checkpoint()))
            .expect_err("invalid signature");

    assert_eq!(failure, VerificationFailure::InvalidSignature);
}
