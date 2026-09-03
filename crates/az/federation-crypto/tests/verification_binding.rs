use std::sync::Arc;

use az_federation_crypto::{
    MessageDigest, OpenSigningFuture, ProviderSignature, PublicKeyId, RawSignature, RootRecovery,
    ScopedDigest, Signer, SigningFuture, SigningPort, VerificationFuture, VerificationPort,
    VerificationRequest, Verifier,
};

struct RecoverySigner;

impl SigningPort<RootRecovery> for RecoverySigner {
    fn open(&self) -> OpenSigningFuture<'_> {
        Box::pin(async { Ok(PublicKeyId::from_bytes([0x31; 32])) })
    }

    fn sign(&self, _: ScopedDigest<RootRecovery>) -> SigningFuture<'_> {
        Box::pin(async {
            Ok(ProviderSignature::new(
                PublicKeyId::from_bytes([0x31; 32]),
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

#[test]
fn verified_signature_remembers_the_exact_canonical_message() {
    let message = MessageDigest::from_bytes([0x42; 32]);
    let signer =
        futures::executor::block_on(Signer::<RootRecovery>::open(Arc::new(RecoverySigner)))
            .expect("open recovery signer");
    let signature = futures::executor::block_on(signer.sign(message)).expect("sign recovery");
    let verifier = Verifier::new(Arc::new(AcceptingVerifier));

    let proof = futures::executor::block_on(verifier.verify(message, signature))
        .expect("verify recovery signature");

    assert_eq!(proof.message_digest(), message);
}
