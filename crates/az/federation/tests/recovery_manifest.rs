use az_federation::authority::{
    AuthorityRecoveryManifest, AuthorityRecoveryTarget, RecoveryManifestError,
};
use az_federation::{
    AuthorityAtomId, AuthorityEpoch, ContentDigest, FederationId, HistoryCheckpointRef, OperationId,
};
use uuid::Uuid;

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}
fn epoch(value: u64) -> AuthorityEpoch {
    AuthorityEpoch::try_from(value).unwrap()
}
fn operation() -> OperationId {
    OperationId::try_from_uuid(Uuid::from_u128(1)).unwrap()
}
fn target(byte: u8) -> AuthorityRecoveryTarget {
    AuthorityRecoveryTarget::new(
        AuthorityAtomId::from_digest(digest(byte)),
        u64::from(byte),
        digest(byte + 20),
    )
}
fn manifest(
    targets: Box<[AuthorityRecoveryTarget]>,
) -> Result<AuthorityRecoveryManifest, RecoveryManifestError> {
    AuthorityRecoveryManifest::new(
        FederationId::from_digest(digest(1)),
        epoch(7),
        epoch(8),
        HistoryCheckpointRef::new(39, digest(2)),
        operation(),
        digest(3),
        digest(4),
        targets,
    )
}

#[test]
fn manifest_names_an_exact_canonical_target_set() {
    let manifest = manifest(vec![target(5), target(9)].into_boxed_slice()).unwrap();
    assert_eq!(manifest.targets(), &[target(5), target(9)]);
    assert_eq!(manifest.target(target(9).atom()), Some(target(9)));
    assert_eq!(manifest.target(target(7).atom()), None);
}

#[test]
fn empty_duplicate_or_reordered_target_sets_are_rejected() {
    assert_eq!(
        manifest(Box::new([])),
        Err(RecoveryManifestError::EmptyTargets)
    );
    assert_eq!(
        manifest(vec![target(5), target(5)].into_boxed_slice()),
        Err(RecoveryManifestError::NonCanonicalTargetOrder)
    );
    assert_eq!(
        manifest(vec![target(9), target(5)].into_boxed_slice()),
        Err(RecoveryManifestError::NonCanonicalTargetOrder)
    );
}
