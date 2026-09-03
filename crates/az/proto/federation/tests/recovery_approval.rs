use std::sync::Arc;

use az_federation::{
    AuthorityEpoch, AuthorityRecoveryStatement, ContentDigest, FederationId, HistoryCheckpointRef,
    RootRecoveryKeyPolicy, authorize_root_recovery,
};
use az_federation_crypto::{
    Ed25519Verifier, MessageDigest, PublicKeyId, RawSignature, RootRecovery, Signature,
    SignatureDomainVersion, Verifier,
};
use az_proto_federation::{
    RecoveryApprovalError, decode_root_recovery_approval, encode_authority_recovery,
    encode_root_recovery_approval,
};

const PUBLIC_KEY: [u8; 32] = [
    215, 90, 152, 1, 130, 177, 10, 183, 213, 75, 254, 211, 201, 100, 7, 58, 14, 225, 114, 243, 218,
    166, 35, 37, 175, 2, 26, 104, 247, 7, 81, 26,
];
const SIGNATURE: [u8; 64] = [
    55, 76, 65, 174, 132, 129, 160, 119, 65, 114, 183, 36, 89, 196, 5, 230, 53, 24, 76, 140, 138,
    88, 1, 163, 35, 129, 21, 135, 124, 64, 1, 96, 9, 60, 189, 36, 238, 54, 39, 173, 53, 100, 6,
    112, 128, 30, 12, 157, 107, 134, 90, 129, 99, 111, 133, 187, 110, 135, 133, 14, 181, 111, 230,
    7,
];
const SECOND_PUBLIC_KEY: [u8; 32] = [
    160, 154, 165, 244, 122, 103, 89, 128, 47, 249, 85, 248, 220, 45, 42, 20, 165, 201, 157, 35,
    190, 151, 248, 100, 18, 127, 249, 56, 52, 85, 164, 240,
];
const SECOND_SIGNATURE: [u8; 64] = [
    29, 103, 194, 110, 145, 211, 61, 10, 220, 102, 108, 208, 186, 171, 192, 45, 50, 73, 224, 46,
    90, 44, 104, 196, 67, 26, 45, 52, 22, 138, 237, 62, 117, 139, 68, 22, 63, 47, 164, 182, 70,
    180, 181, 238, 39, 128, 68, 151, 201, 108, 62, 39, 240, 19, 40, 208, 8, 25, 107, 167, 129, 166,
    177, 15,
];
const APPROVAL_BYTES: [u8; 108] = [
    65, 90, 82, 49, 0, 1, 0, 5, 0, 1, 0, 1, 215, 90, 152, 1, 130, 177, 10, 183, 213, 75, 254, 211,
    201, 100, 7, 58, 14, 225, 114, 243, 218, 166, 35, 37, 175, 2, 26, 104, 247, 7, 81, 26, 55, 76,
    65, 174, 132, 129, 160, 119, 65, 114, 183, 36, 89, 196, 5, 230, 53, 24, 76, 140, 138, 88, 1,
    163, 35, 129, 21, 135, 124, 64, 1, 96, 9, 60, 189, 36, 238, 54, 39, 173, 53, 100, 6, 112, 128,
    30, 12, 157, 107, 134, 90, 129, 99, 111, 133, 187, 110, 135, 133, 14, 181, 111, 230, 7,
];

const fn approval() -> Signature<RootRecovery> {
    Signature::from_parts(
        PublicKeyId::from_bytes(PUBLIC_KEY),
        SignatureDomainVersion::V1,
        RawSignature::Ed25519V1(SIGNATURE),
    )
}

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}

fn statement() -> AuthorityRecoveryStatement {
    AuthorityRecoveryStatement::new(
        FederationId::from_digest(digest(0x10)),
        AuthorityEpoch::try_from(7).expect("current epoch"),
        AuthorityEpoch::try_from(9).expect("successor epoch"),
        HistoryCheckpointRef::new(41, digest(0x21)),
        HistoryCheckpointRef::new(41, digest(0x22)),
        HistoryCheckpointRef::new(39, digest(0x23)),
        digest(0x31),
        digest(0x32),
        digest(0x33),
    )
    .expect("recovery statement")
}

#[test]
fn root_recovery_approval_has_one_fixed_canonical_record() {
    assert_eq!(
        decode_root_recovery_approval(&APPROVAL_BYTES),
        Ok(approval())
    );
    assert_eq!(encode_root_recovery_approval(approval()), APPROVAL_BYTES);
}

#[test]
fn approval_parser_rejects_role_suite_and_length_substitution() {
    let mut wrong_role = APPROVAL_BYTES;
    wrong_role[7] = 2;
    assert_eq!(
        decode_root_recovery_approval(&wrong_role),
        Err(RecoveryApprovalError::InvalidRole { found: 2 })
    );

    let mut wrong_suite = APPROVAL_BYTES;
    wrong_suite[11] = 2;
    assert_eq!(
        decode_root_recovery_approval(&wrong_suite),
        Err(RecoveryApprovalError::UnsupportedSuite { found: 2 })
    );
    assert_eq!(
        decode_root_recovery_approval(&APPROVAL_BYTES[..107]),
        Err(RecoveryApprovalError::InvalidLength {
            actual: 107,
            expected: 108,
        })
    );
}

#[test]
fn canonical_recovery_bytes_reach_real_crypto_and_policy_quorum() {
    let statement = statement();
    let envelope = encode_authority_recovery(&statement);
    let message = MessageDigest::from_bytes(*envelope.request_digest().as_bytes());
    let verifier = Verifier::new(Arc::new(Ed25519Verifier));
    let approvals = [
        approval(),
        Signature::from_parts(
            PublicKeyId::from_bytes(SECOND_PUBLIC_KEY),
            SignatureDomainVersion::V1,
            RawSignature::Ed25519V1(SECOND_SIGNATURE),
        ),
    ];
    let first = decode_root_recovery_approval(&encode_root_recovery_approval(approvals[0]))
        .expect("first canonical approval");
    let second = decode_root_recovery_approval(&encode_root_recovery_approval(approvals[1]))
        .expect("second canonical approval");
    let proofs = [
        futures::executor::block_on(verifier.verify(message, first)).expect("first root signature"),
        futures::executor::block_on(verifier.verify(message, second))
            .expect("second root signature"),
    ];
    let policy = RootRecoveryKeyPolicy::new(
        digest(0x32),
        2,
        &[
            PublicKeyId::from_bytes(PUBLIC_KEY),
            PublicKeyId::from_bytes(SECOND_PUBLIC_KEY),
        ],
    )
    .expect("root key policy");

    let quorum = authorize_root_recovery(&policy, statement, message, &proofs)
        .expect("authorized recovery quorum");

    assert_eq!(quorum.statement(), statement);
    assert_eq!(quorum.approval_count(), 2);
}
