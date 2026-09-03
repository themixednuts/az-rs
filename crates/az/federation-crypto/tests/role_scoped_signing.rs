use std::sync::{Arc, Mutex};

use az_federation_crypto::{
    AuthorityReceipt, HostEvidence, MessageDigest, OpenSigningFuture, ProviderSignature,
    PublicKeyId, RawSignature, RootRecovery, ScopedDigest, SignatureDomainVersion, SignatureRoleId,
    SignatureSuite, Signer, SigningFailure, SigningFuture, SigningPort, SigningRole,
    scope_message_digest,
};

#[derive(Default)]
struct CallRecorder {
    digests: Mutex<Vec<[u8; 32]>>,
}

impl CallRecorder {
    fn record<R: SigningRole>(&self, digest: ScopedDigest<R>) {
        self.digests
            .lock()
            .expect("digest recorder lock")
            .push(*digest.as_bytes());
    }

    fn digests(&self) -> Vec<[u8; 32]> {
        self.digests.lock().expect("digest recorder lock").clone()
    }
}

struct HostSigningPort {
    recorder: Arc<CallRecorder>,
    open_failure: Option<SigningFailure>,
    sign_failure: Option<SigningFailure>,
    signing_key: PublicKeyId,
}

impl HostSigningPort {
    const fn ready(recorder: Arc<CallRecorder>) -> Self {
        Self {
            recorder,
            open_failure: None,
            sign_failure: None,
            signing_key: PublicKeyId::from_bytes([0x31; 32]),
        }
    }

    const fn failing_open(recorder: Arc<CallRecorder>, failure: SigningFailure) -> Self {
        Self {
            recorder,
            open_failure: Some(failure),
            sign_failure: None,
            signing_key: PublicKeyId::from_bytes([0x31; 32]),
        }
    }

    const fn failing_sign(recorder: Arc<CallRecorder>, failure: SigningFailure) -> Self {
        Self {
            recorder,
            open_failure: None,
            sign_failure: Some(failure),
            signing_key: PublicKeyId::from_bytes([0x31; 32]),
        }
    }

    const fn changed_key(recorder: Arc<CallRecorder>) -> Self {
        Self {
            recorder,
            open_failure: None,
            sign_failure: None,
            signing_key: PublicKeyId::from_bytes([0x33; 32]),
        }
    }
}

impl SigningPort<HostEvidence> for HostSigningPort {
    fn open(&self) -> OpenSigningFuture<'_> {
        let result = self
            .open_failure
            .clone()
            .map_or(Ok(PublicKeyId::from_bytes([0x31; 32])), Err);
        Box::pin(async move { result })
    }

    fn sign(&self, digest: ScopedDigest<HostEvidence>) -> SigningFuture<'_> {
        self.recorder.record(digest);
        let result = self.sign_failure.clone().map_or_else(
            || {
                Ok(ProviderSignature::new(
                    self.signing_key,
                    RawSignature::Ed25519V1([0x5a; 64]),
                ))
            },
            Err,
        );
        Box::pin(async move { result })
    }
}

struct AuthoritySigningPort {
    recorder: Arc<CallRecorder>,
}

impl SigningPort<AuthorityReceipt> for AuthoritySigningPort {
    fn open(&self) -> OpenSigningFuture<'_> {
        Box::pin(async { Ok(PublicKeyId::from_bytes([0x32; 32])) })
    }

    fn sign(&self, digest: ScopedDigest<AuthorityReceipt>) -> SigningFuture<'_> {
        self.recorder.record(digest);
        Box::pin(async {
            Ok(ProviderSignature::new(
                PublicKeyId::from_bytes([0x32; 32]),
                RawSignature::Ed25519V1([0xa5; 64]),
            ))
        })
    }
}

#[test]
fn signer_scopes_a_digest_before_the_provider_boundary() {
    let recorder = Arc::new(CallRecorder::default());
    let signer = futures::executor::block_on(Signer::<HostEvidence>::open(Arc::new(
        HostSigningPort::ready(recorder.clone()),
    )))
    .expect("open host signer");

    let signature = futures::executor::block_on(signer.sign(MessageDigest::from_bytes([0x17; 32])))
        .expect("sign host evidence");

    assert_eq!(signature.public_key(), &PublicKeyId::from_bytes([0x31; 32]));
    assert_eq!(signature.domain_version(), SignatureDomainVersion::V1);
    assert_eq!(signature.role(), SignatureRoleId::HostEvidence);
    assert_eq!(signature.suite(), SignatureSuite::Ed25519V1);
    assert_eq!(signature.raw(), &RawSignature::Ed25519V1([0x5a; 64]));
    assert_eq!(
        recorder.digests(),
        vec![[
            13, 63, 46, 226, 230, 252, 41, 144, 151, 24, 187, 92, 3, 120, 1, 25, 165, 115, 95, 216,
            241, 9, 166, 26, 22, 24, 249, 153, 10, 176, 4, 180,
        ]]
    );
}

#[test]
fn identical_messages_have_distinct_host_and_authority_scopes() {
    let host_recorder = Arc::new(CallRecorder::default());
    let authority_recorder = Arc::new(CallRecorder::default());
    let host = futures::executor::block_on(Signer::<HostEvidence>::open(Arc::new(
        HostSigningPort::ready(host_recorder.clone()),
    )))
    .expect("open host signer");
    let authority = futures::executor::block_on(Signer::<AuthorityReceipt>::open(Arc::new(
        AuthoritySigningPort {
            recorder: authority_recorder.clone(),
        },
    )))
    .expect("open authority signer");
    let message = MessageDigest::from_bytes([0x42; 32]);

    futures::executor::block_on(host.sign(message)).expect("sign host evidence");
    futures::executor::block_on(authority.sign(message)).expect("sign authority receipt");

    assert_eq!(
        host_recorder.digests(),
        vec![[
            241, 105, 131, 219, 2, 247, 227, 8, 70, 238, 25, 122, 194, 73, 106, 254, 163, 181, 36,
            194, 219, 99, 143, 1, 16, 161, 139, 71, 88, 104, 221, 18,
        ]]
    );
    assert_eq!(
        authority_recorder.digests(),
        vec![[
            31, 46, 151, 56, 25, 28, 150, 173, 159, 251, 103, 42, 65, 70, 96, 68, 205, 247, 65, 65,
            180, 137, 228, 243, 163, 141, 70, 190, 117, 87, 50, 228,
        ]]
    );
}

#[test]
fn root_recovery_cannot_reuse_an_online_signature_domain() {
    let message = MessageDigest::from_bytes([0x42; 32]);

    let recovery = scope_message_digest::<RootRecovery>(message);
    let authority = scope_message_digest::<AuthorityReceipt>(message);
    let log = scope_message_digest::<az_federation_crypto::HistoryLog>(message);

    assert_eq!(recovery.role(), SignatureRoleId::RootRecovery);
    assert_ne!(recovery.as_bytes(), authority.as_bytes());
    assert_ne!(recovery.as_bytes(), log.as_bytes());
}

#[test]
fn signer_is_not_ready_when_provider_attestation_fails() {
    let recorder = Arc::new(CallRecorder::default());
    let error = futures::executor::block_on(Signer::<HostEvidence>::open(Arc::new(
        HostSigningPort::failing_open(recorder.clone(), SigningFailure::PermissionDenied),
    )))
    .expect_err("permission failure must prevent readiness");

    assert_eq!(error, SigningFailure::PermissionDenied);
    assert!(recorder.digests().is_empty());
}

#[test]
fn signing_failure_stays_typed_after_readiness() {
    let recorder = Arc::new(CallRecorder::default());
    let signer = futures::executor::block_on(Signer::<HostEvidence>::open(Arc::new(
        HostSigningPort::failing_sign(recorder.clone(), SigningFailure::Unavailable),
    )))
    .expect("open host signer");

    let error = futures::executor::block_on(signer.sign(MessageDigest::from_bytes([0x99; 32])))
        .expect_err("signing failure must reach caller");

    assert_eq!(error, SigningFailure::Unavailable);
    assert_eq!(recorder.digests().len(), 1);
}

#[test]
fn provider_key_change_after_attestation_is_rejected() {
    let recorder = Arc::new(CallRecorder::default());
    let signer = futures::executor::block_on(Signer::<HostEvidence>::open(Arc::new(
        HostSigningPort::changed_key(recorder),
    )))
    .expect("open host signer");

    let error = futures::executor::block_on(signer.sign(MessageDigest::from_bytes([0x44; 32])))
        .expect_err("changed provider key must not be mislabeled");

    assert_eq!(error, SigningFailure::KeyIdentityChanged);
}
