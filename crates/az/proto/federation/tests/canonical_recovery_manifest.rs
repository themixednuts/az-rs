use az_federation::authority::{AuthorityRecoveryManifest, AuthorityRecoveryTarget};
use az_federation::{
    AuthorityAtomId, AuthorityEpoch, ContentDigest, FederationId, HistoryCheckpointRef, OperationId,
};
use az_proto_federation::{
    decode_recovery_manifest, encode_recovery_manifest, recovery_manifest_digest,
};
use uuid::Uuid;

const fn digest(byte: u8) -> ContentDigest {
    ContentDigest::from_bytes([byte; 32])
}

fn manifest() -> AuthorityRecoveryManifest {
    AuthorityRecoveryManifest::new(
        FederationId::from_digest(digest(1)),
        AuthorityEpoch::try_from(7).unwrap(),
        AuthorityEpoch::try_from(8).unwrap(),
        HistoryCheckpointRef::new(39, digest(2)),
        OperationId::try_from_uuid(Uuid::from_u128(1)).unwrap(),
        digest(3),
        digest(4),
        vec![AuthorityRecoveryTarget::new(
            AuthorityAtomId::from_digest(digest(5)),
            9,
            digest(6),
        )]
        .into_boxed_slice(),
    )
    .unwrap()
}

#[test]
fn canonical_manifest_round_trips_and_has_stable_digest() {
    let manifest = manifest();
    let envelope = encode_recovery_manifest(&manifest);
    assert_eq!(
        decode_recovery_manifest(envelope.as_bytes()).unwrap(),
        manifest
    );
    assert_eq!(
        recovery_manifest_digest(&manifest).as_bytes(),
        envelope.request_digest().as_bytes()
    );
}

#[test]
fn altered_target_bytes_change_the_approved_manifest_digest() {
    let original = manifest();
    let altered = AuthorityRecoveryManifest::new(
        original.federation(),
        original.current_epoch(),
        original.successor_epoch(),
        original.selected_checkpoint(),
        original.operation(),
        original.impact_inventory(),
        original.history_amendment(),
        vec![AuthorityRecoveryTarget::new(
            AuthorityAtomId::from_digest(digest(5)),
            10,
            digest(6),
        )]
        .into_boxed_slice(),
    )
    .unwrap();
    assert_ne!(
        recovery_manifest_digest(&original),
        recovery_manifest_digest(&altered)
    );
}
