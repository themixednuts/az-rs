use az_federation::{
    AuthorityEpoch, AuthorityRecoveryStatement, ContentDigest, FederationId, HistoryCheckpointRef,
    RecoveryStatementError,
};

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}

const fn checkpoint(size: u64, byte: u8) -> HistoryCheckpointRef {
    HistoryCheckpointRef::new(size, digest(byte))
}

#[test]
fn recovery_statement_names_the_fork_and_advances_visible_authority() {
    let statement = AuthorityRecoveryStatement::new(
        FederationId::from_digest(digest(0x10)),
        AuthorityEpoch::try_from(7).expect("current epoch"),
        AuthorityEpoch::try_from(8).expect("successor epoch"),
        checkpoint(41, 0x21),
        checkpoint(41, 0x22),
        checkpoint(39, 0x23),
        digest(0x31),
        digest(0x32),
        digest(0x33),
    )
    .expect("forward recovery statement");

    assert_eq!(statement.current_epoch().get(), 7);
    assert_eq!(statement.successor_epoch().get(), 8);
    assert_eq!(statement.trusted_checkpoint(), checkpoint(41, 0x21));
    assert_eq!(statement.divergent_checkpoint(), checkpoint(41, 0x22));
    assert_eq!(statement.selected_checkpoint(), checkpoint(39, 0x23));
    assert_eq!(statement.successor_key_policy(), digest(0x31));
    assert_eq!(statement.recovery_policy(), digest(0x32));
    assert_eq!(statement.recovery_manifest(), digest(0x33));
}

#[test]
fn recovery_never_reuses_or_moves_back_an_authority_epoch() {
    for successor in [6, 7] {
        let result = AuthorityRecoveryStatement::new(
            FederationId::from_digest(digest(0x10)),
            AuthorityEpoch::try_from(7).expect("current epoch"),
            AuthorityEpoch::try_from(successor).expect("successor epoch"),
            checkpoint(41, 0x21),
            checkpoint(41, 0x22),
            checkpoint(39, 0x23),
            digest(0x31),
            digest(0x32),
            digest(0x33),
        );

        assert_eq!(
            result,
            Err(RecoveryStatementError::NonSuccessorEpoch {
                current: 7,
                proposed: successor,
            })
        );
    }
}

#[test]
fn recovery_requires_actual_divergence_evidence() {
    let same = checkpoint(41, 0x21);
    let result = AuthorityRecoveryStatement::new(
        FederationId::from_digest(digest(0x10)),
        AuthorityEpoch::try_from(7).expect("current epoch"),
        AuthorityEpoch::try_from(8).expect("successor epoch"),
        same,
        same,
        checkpoint(39, 0x23),
        digest(0x31),
        digest(0x32),
        digest(0x33),
    );

    assert_eq!(result, Err(RecoveryStatementError::NoForkDivergence));
}
