use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use az_federation_crypto::{
    AuthorityReceipt, HostEvidence, MAX_SIGNING_KEY_REF_BYTES, MessageDigest, OpenKeyCustodyFuture,
    OpenSigningFuture, ProviderSignature, PublicKeyId, RawSignature, ScopedDigest, SignatureRoleId,
    SigningFailure, SigningFuture, SigningKeyCustody, SigningKeyHandle, SigningKeyRef,
    SigningKeyRefError, SigningPort, SigningRole, open_game_server_signer, open_role_signer,
};

/// Provider adapter fake: it owns its key privately and never returns material.
struct FakeProviderKey<R: SigningRole> {
    public_key: PublicKeyId,
    open_failure: Option<SigningFailure>,
    role: std::marker::PhantomData<fn() -> R>,
}

impl<R: SigningRole> FakeProviderKey<R> {
    const fn ready(public_key: PublicKeyId) -> Self {
        Self {
            public_key,
            open_failure: None,
            role: std::marker::PhantomData,
        }
    }

    const fn failing_open(failure: SigningFailure) -> Self {
        Self {
            public_key: PublicKeyId::from_bytes([0x11; 32]),
            open_failure: Some(failure),
            role: std::marker::PhantomData,
        }
    }
}

impl<R: SigningRole> SigningPort<R> for FakeProviderKey<R> {
    fn open(&self) -> OpenSigningFuture<'_> {
        let result = self.open_failure.clone().map_or(Ok(self.public_key), Err);
        Box::pin(async move { result })
    }

    fn sign(&self, _digest: ScopedDigest<R>) -> SigningFuture<'_> {
        let public_key = self.public_key;
        Box::pin(async move {
            Ok(ProviderSignature::new(
                public_key,
                RawSignature::Ed25519V1([0x7c; 64]),
            ))
        })
    }
}

/// Custody fake: records the exact key reference it was asked to resolve.
struct RecordingCustody<R: SigningRole> {
    requested: std::sync::Mutex<Vec<String>>,
    calls: AtomicUsize,
    outcome: Result<PublicKeyId, SigningFailure>,
    port_open_failure: Option<SigningFailure>,
    role: std::marker::PhantomData<fn() -> R>,
}

impl<R: SigningRole> RecordingCustody<R> {
    fn ready(public_key: PublicKeyId) -> Self {
        Self {
            requested: std::sync::Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            outcome: Ok(public_key),
            port_open_failure: None,
            role: std::marker::PhantomData,
        }
    }

    fn refusing(failure: SigningFailure) -> Self {
        Self {
            requested: std::sync::Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            outcome: Err(failure),
            port_open_failure: None,
            role: std::marker::PhantomData,
        }
    }

    fn with_unattestable_key(failure: SigningFailure) -> Self {
        Self {
            requested: std::sync::Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            outcome: Ok(PublicKeyId::from_bytes([0x11; 32])),
            port_open_failure: Some(failure),
            role: std::marker::PhantomData,
        }
    }

    fn requested_references(&self) -> Vec<String> {
        self.requested
            .lock()
            .expect("reference recorder lock")
            .clone()
    }
}

impl<R: SigningRole> SigningKeyCustody<R> for RecordingCustody<R> {
    fn open_key<'a>(&'a self, handle: &'a SigningKeyHandle<R>) -> OpenKeyCustodyFuture<'a, R> {
        self.requested
            .lock()
            .expect("reference recorder lock")
            .push(handle.key_ref().as_str().to_owned());
        self.calls.fetch_add(1, Ordering::Relaxed);
        let outcome = self.outcome.clone();
        let port_open_failure = self.port_open_failure.clone();
        Box::pin(async move {
            let public_key = outcome?;
            let port: Arc<dyn SigningPort<R>> = port_open_failure.map_or_else(
                || Arc::new(FakeProviderKey::<R>::ready(public_key)),
                |failure| Arc::new(FakeProviderKey::<R>::failing_open(failure)),
            );
            Ok(port)
        })
    }
}

fn host_handle(key_ref: &str) -> SigningKeyHandle<HostEvidence> {
    SigningKeyHandle::new(SigningKeyRef::parse(key_ref).expect("valid key reference"))
}

#[test]
fn custody_opens_exactly_the_named_role_key_and_attests_it() {
    let custody = RecordingCustody::<HostEvidence>::ready(PublicKeyId::from_bytes([0x41; 32]));
    let handle = host_handle("secret://project/host-evidence-key");

    let signer =
        futures::executor::block_on(open_role_signer(&custody, &handle)).expect("open host signer");
    let signature = futures::executor::block_on(signer.sign(MessageDigest::from_bytes([0x09; 32])))
        .expect("sign host evidence");

    assert_eq!(
        custody.requested_references(),
        vec!["secret://project/host-evidence-key".to_owned()],
        "custody must receive the exact key reference named by the composition root"
    );
    assert_eq!(signature.role(), SignatureRoleId::HostEvidence);
    assert_eq!(
        *signature.public_key(),
        PublicKeyId::from_bytes([0x41; 32]),
        "the attested custody key identity must reach the signature"
    );
}

#[test]
fn custody_refusal_fails_startup_closed() {
    let custody = RecordingCustody::<HostEvidence>::refusing(SigningFailure::PermissionDenied);
    let handle = host_handle("secret://project/host-evidence-key");

    assert_eq!(
        futures::executor::block_on(open_role_signer(&custody, &handle)).unwrap_err(),
        SigningFailure::PermissionDenied,
        "a credential refusal must fail startup rather than yield an unusable signer"
    );
}

#[test]
fn an_unattestable_provider_key_fails_startup_closed() {
    let custody = RecordingCustody::<HostEvidence>::with_unattestable_key(
        SigningFailure::RoleBindingRejected,
    );
    let handle = host_handle("secret://project/host-evidence-key");

    assert_eq!(
        futures::executor::block_on(open_role_signer(&custody, &handle)).unwrap_err(),
        SigningFailure::RoleBindingRejected,
        "custody success must still require the provider's role-binding attestation"
    );
}

#[test]
fn a_game_server_composition_root_opens_only_its_host_evidence_signer() {
    let custody = RecordingCustody::<HostEvidence>::ready(PublicKeyId::from_bytes([0x42; 32]));
    let handle = host_handle("secret://project/host-evidence-key");

    let signer = futures::executor::block_on(open_game_server_signer(&custody, &handle))
        .expect("open game-server signer");

    assert!(
        format!("{signer:?}").contains("HostEvidence"),
        "the game-server signer must stay role-typed"
    );
}

#[test]
fn a_handle_names_its_role_and_carries_no_material() {
    let handle = host_handle("secret://project/host-evidence-key");

    assert_eq!(handle.role(), SignatureRoleId::HostEvidence);
    let rendered = format!("{handle:?}");
    assert!(
        rendered.contains("HostEvidence") && rendered.contains("host-evidence-key"),
        "a handle must render its role and key reference for diagnostics: {rendered}"
    );
}

#[test]
fn authority_and_host_handles_stay_distinct_values() {
    let host = host_handle("secret://project/shared-key");
    let authority = SigningKeyHandle::<AuthorityReceipt>::new(
        SigningKeyRef::parse("secret://project/shared-key").expect("valid key reference"),
    );

    assert_eq!(host.key_ref(), authority.key_ref());
    assert_ne!(
        host.role(),
        authority.role(),
        "identical key references must still name different signing roles"
    );
}

#[test]
fn key_reference_parsing_bounds_and_rejects_noncanonical_names() {
    assert_eq!(SigningKeyRef::parse(""), Err(SigningKeyRefError::Empty));
    assert_eq!(
        SigningKeyRef::parse(&"a".repeat(MAX_SIGNING_KEY_REF_BYTES + 1)),
        Err(SigningKeyRefError::TooLong)
    );
    assert_eq!(
        SigningKeyRef::parse("secret://project/host key"),
        Err(SigningKeyRefError::InvalidByte),
        "whitespace and control bytes must not reach a custody adapter"
    );
    assert_eq!(
        SigningKeyRef::parse("secret://project/host\u{7f}key"),
        Err(SigningKeyRefError::InvalidByte)
    );
    assert_eq!(
        SigningKeyRef::parse(&"a".repeat(MAX_SIGNING_KEY_REF_BYTES))
            .expect("maximum-length key reference")
            .as_str()
            .len(),
        MAX_SIGNING_KEY_REF_BYTES
    );
}
