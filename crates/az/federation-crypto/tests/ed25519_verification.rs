use std::sync::Arc;

use az_federation_crypto::{
    Ed25519Verifier, MessageDigest, PublicKeyId, RawSignature, RootRecovery, Signature,
    SignatureDomainVersion, VerificationFailure, Verifier,
};

const PUBLIC_KEY: [u8; 32] = [
    215, 90, 152, 1, 130, 177, 10, 183, 213, 75, 254, 211, 201, 100, 7, 58, 14, 225, 114, 243, 218,
    166, 35, 37, 175, 2, 26, 104, 247, 7, 81, 26,
];
const SIGNATURE: [u8; 64] = [
    8, 221, 135, 188, 142, 128, 232, 195, 30, 136, 27, 218, 200, 231, 80, 229, 111, 79, 151, 78,
    188, 14, 50, 124, 1, 66, 198, 72, 251, 98, 143, 43, 158, 222, 196, 115, 48, 116, 111, 177, 40,
    117, 166, 122, 245, 239, 244, 188, 131, 154, 221, 138, 86, 90, 175, 114, 238, 8, 242, 11, 213,
    174, 132, 7,
];

const fn signature(public_key: [u8; 32], bytes: [u8; 64]) -> Signature<RootRecovery> {
    Signature::from_parts(
        PublicKeyId::from_bytes(public_key),
        SignatureDomainVersion::V1,
        RawSignature::Ed25519V1(bytes),
    )
}

#[test]
fn ed25519_adapter_verifies_an_independent_root_recovery_vector() {
    let message = MessageDigest::from_bytes([0x42; 32]);
    let signature = signature(PUBLIC_KEY, SIGNATURE);

    let proof = futures::executor::block_on(
        Verifier::new(Arc::new(Ed25519Verifier)).verify(message, signature),
    )
    .expect("verify independent Ed25519 vector");

    assert_eq!(proof.public_key(), PublicKeyId::from_bytes(PUBLIC_KEY));
    assert_eq!(proof.message_digest(), message);
}

#[test]
fn ed25519_adapter_rejects_changed_signature_message_or_key() {
    let verifier = Verifier::new(Arc::new(Ed25519Verifier));
    let message = MessageDigest::from_bytes([0x42; 32]);
    let mut changed = SIGNATURE;
    changed[17] ^= 1;

    for (candidate_message, candidate_signature) in [
        (message, signature(PUBLIC_KEY, changed)),
        (
            MessageDigest::from_bytes([0x43; 32]),
            signature(PUBLIC_KEY, SIGNATURE),
        ),
        (message, signature([0xff; 32], SIGNATURE)),
    ] {
        let failure =
            futures::executor::block_on(verifier.verify(candidate_message, candidate_signature))
                .expect_err("invalid Ed25519 proof");

        assert_eq!(failure, VerificationFailure::InvalidSignature);
    }
}
