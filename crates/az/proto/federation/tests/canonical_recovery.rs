use az_federation::{
    AuthorityEpoch, AuthorityRecoveryStatement, ContentDigest, FederationId, HistoryCheckpointRef,
};
use az_proto_federation::{decode_authority_recovery, encode_authority_recovery};

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}

const fn checkpoint(size: u64, byte: u8) -> HistoryCheckpointRef {
    HistoryCheckpointRef::new(size, digest(byte))
}

fn statement() -> AuthorityRecoveryStatement {
    AuthorityRecoveryStatement::new(
        FederationId::from_digest(digest(0x10)),
        AuthorityEpoch::try_from(7).expect("current epoch"),
        AuthorityEpoch::try_from(9).expect("successor epoch"),
        checkpoint(41, 0x21),
        checkpoint(41, 0x22),
        checkpoint(39, 0x23),
        digest(0x31),
        digest(0x32),
        digest(0x33),
    )
    .expect("recovery statement")
}

#[test]
fn recovery_statement_has_stable_canonical_bytes_and_digest() {
    let expected = statement();
    let envelope = encode_authority_recovery(&expected);

    assert_eq!(decode_authority_recovery(envelope.as_bytes()), Ok(expected));
    assert_eq!(envelope.as_bytes().len(), 328);
    assert_eq!(
        envelope.request_digest().as_bytes(),
        &[
            196, 248, 21, 88, 4, 113, 8, 173, 59, 97, 134, 58, 253, 167, 13, 30, 241, 75, 192, 135,
            220, 139, 131, 79, 35, 228, 54, 45, 86, 146, 109, 109,
        ]
    );
}

#[test]
fn recovery_decoder_rejects_noncanonical_or_nonforward_statements() {
    let envelope = encode_authority_recovery(&statement());
    let mut reordered = envelope.as_bytes().to_vec();
    reordered[10..12].copy_from_slice(&2_u16.to_be_bytes());
    reordered[48..50].copy_from_slice(&1_u16.to_be_bytes());
    assert!(decode_authority_recovery(&reordered).is_err());

    let mut nonforward = envelope.as_bytes().to_vec();
    // Field 3 begins after the 10-byte header, field 1 (38 bytes), and field 2 (14 bytes).
    nonforward[68..76].copy_from_slice(&7_u64.to_be_bytes());
    assert!(decode_authority_recovery(&nonforward).is_err());
}
