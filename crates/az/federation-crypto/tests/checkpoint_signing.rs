use std::sync::{Arc, Mutex};

use az_federation_crypto::{
    CommitmentNonce, HistoryLog, HistoryWitness, MerkleHistory, OpenSigningFuture,
    ProviderSignature, PublicKeyId, RawSignature, ReceiptCommitment, ReceiptDigest, ScopedDigest,
    SignatureRoleId, Signer, SigningFuture, SigningPort, SigningRole, sign_history_checkpoint,
};

#[derive(Default)]
struct RecordingPort {
    digests: Mutex<Vec<[u8; 32]>>,
}

impl RecordingPort {
    fn digests(&self) -> Vec<[u8; 32]> {
        self.digests.lock().expect("digest lock").clone()
    }
}

impl<R: SigningRole> SigningPort<R> for RecordingPort {
    fn open(&self) -> OpenSigningFuture<'_> {
        Box::pin(async { Ok(PublicKeyId::from_bytes([0x31; 32])) })
    }

    fn sign(&self, digest: ScopedDigest<R>) -> SigningFuture<'_> {
        self.digests
            .lock()
            .expect("digest lock")
            .push(*digest.as_bytes());
        Box::pin(async {
            Ok(ProviderSignature::new(
                PublicKeyId::from_bytes([0x31; 32]),
                RawSignature::Ed25519V1([0x5a; 64]),
            ))
        })
    }
}

fn checkpoint() -> az_federation_crypto::HistoryCheckpoint {
    let mut history = MerkleHistory::new();
    for byte in 1..=3 {
        history
            .append(ReceiptCommitment::new(
                CommitmentNonce::from_bytes([byte + 0x40; 32]),
                ReceiptDigest::from_bytes([byte; 32]),
            ))
            .expect("append");
    }
    history.checkpoint()
}

#[test]
fn checkpoint_digest_is_stable_and_log_signature_keeps_its_role() {
    let checkpoint = checkpoint();
    assert_eq!(
        checkpoint.message_digest().as_bytes(),
        &[
            177, 141, 2, 29, 87, 77, 229, 92, 191, 9, 28, 82, 207, 104, 3, 100, 215, 187, 84, 183,
            94, 251, 111, 222, 225, 199, 110, 63, 18, 148, 112, 112,
        ]
    );

    let recorder = Arc::new(RecordingPort::default());
    let log_signer = futures::executor::block_on(Signer::<HistoryLog>::open(recorder.clone()))
        .expect("open log signer");
    let signed_checkpoint =
        futures::executor::block_on(sign_history_checkpoint(checkpoint, &log_signer))
            .expect("sign checkpoint");

    assert_eq!(signed_checkpoint.checkpoint(), checkpoint);
    assert_eq!(
        signed_checkpoint.signature().role(),
        SignatureRoleId::HistoryLog
    );
    assert_eq!(recorder.digests().len(), 1);
}

#[test]
fn log_and_witness_signatures_use_distinct_domains() {
    let checkpoint = checkpoint();
    let log_recorder = Arc::new(RecordingPort::default());
    let witness_recorder = Arc::new(RecordingPort::default());
    let log = futures::executor::block_on(Signer::<HistoryLog>::open(log_recorder.clone()))
        .expect("open log signer");
    let witness =
        futures::executor::block_on(Signer::<HistoryWitness>::open(witness_recorder.clone()))
            .expect("open witness signer");

    futures::executor::block_on(sign_history_checkpoint(checkpoint, &log))
        .expect("sign log checkpoint");
    futures::executor::block_on(sign_history_checkpoint(checkpoint, &witness))
        .expect("sign witness checkpoint");

    assert_ne!(log_recorder.digests(), witness_recorder.digests());
}
