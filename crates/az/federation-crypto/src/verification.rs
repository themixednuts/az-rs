use std::{fmt, future::Future, marker::PhantomData, pin::Pin, sync::Arc};

use crate::{
    MessageDigest, PublicKeyId, RawSignature, ScopedDigest, Signature, SignatureDomainVersion,
    SignatureRoleId, SigningRole, scope_message_digest,
};

/// Local verifier for the federation's strict Ed25519 version-one suite.
#[derive(Debug, Default, Clone, Copy)]
pub struct Ed25519Verifier;

impl VerificationPort for Ed25519Verifier {
    fn verify(&self, request: VerificationRequest) -> VerificationFuture<'_> {
        Box::pin(async move {
            let public_key = request.public_key();
            let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(public_key.as_bytes())
                .map_err(|_| VerificationFailure::InvalidSignature)?;
            let RawSignature::Ed25519V1(bytes) = request.raw();
            let signature = ed25519_dalek::Signature::from_bytes(&bytes);
            verifying_key
                .verify_strict(request.digest(), &signature)
                .map_err(|_| VerificationFailure::InvalidSignature)
        })
    }
}

/// Provider-neutral request for one cryptographic signature check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationRequest {
    public_key: PublicKeyId,
    role: SignatureRoleId,
    domain_version: SignatureDomainVersion,
    digest: [u8; 32],
    raw: RawSignature,
}

impl VerificationRequest {
    /// Returns the claimed signing-key identity.
    #[must_use]
    pub const fn public_key(self) -> PublicKeyId {
        self.public_key
    }

    /// Returns the type-proven signature role.
    #[must_use]
    pub const fn role(self) -> SignatureRoleId {
        self.role
    }

    /// Returns the signature-domain version.
    #[must_use]
    pub const fn domain_version(self) -> SignatureDomainVersion {
        self.domain_version
    }

    /// Returns the already role-scoped digest.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// Returns the suite-specific signature bytes.
    #[must_use]
    pub const fn raw(self) -> RawSignature {
        self.raw
    }
}

/// Closed failures from a cryptographic verification adapter.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerificationFailure {
    /// The signature does not verify for the key and scoped digest.
    #[error("signature is invalid")]
    InvalidSignature,
    /// The configured verifier does not support the signature suite.
    #[error("signature suite is unsupported")]
    UnsupportedSuite,
    /// Verification cannot currently complete.
    #[error("signature verifier is unavailable")]
    Unavailable,
    /// Verification failed without a more specific public classification.
    #[error("signature verification failed")]
    Failed,
}

/// A verification adapter's asynchronous result.
pub type VerificationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), VerificationFailure>> + Send + 'a>>;

/// Consumer-owned cryptographic signature verification boundary.
///
/// Implementations may use a local suite library, HSM, or remote verifier.
/// They verify only cryptography. Federation role authorization remains a
/// separate policy decision above this port.
pub trait VerificationPort: Send + Sync {
    /// Verifies one fully scoped request.
    fn verify(&self, request: VerificationRequest) -> VerificationFuture<'_>;
}

/// Runtime-selected verifier that applies role scoping before its adapter.
pub struct Verifier {
    adapter: Arc<dyn VerificationPort>,
}

impl Verifier {
    /// Binds one runtime-selected cryptographic adapter.
    #[must_use]
    pub const fn new(adapter: Arc<dyn VerificationPort>) -> Self {
        Self { adapter }
    }

    /// Verifies one signature over a canonical message digest.
    ///
    /// # Errors
    ///
    /// Returns the adapter's typed cryptographic or availability failure.
    pub async fn verify<R: SigningRole>(
        &self,
        message: MessageDigest,
        signature: Signature<R>,
    ) -> Result<VerifiedSignature<R>, VerificationFailure> {
        let scoped = scope_message_digest::<R>(message);
        self.adapter
            .verify(request_for_signature(scoped, signature))
            .await?;
        Ok(VerifiedSignature {
            public_key: signature.public_key,
            domain_version: signature.domain_version,
            message_digest: message,
            role: PhantomData,
        })
    }
}

impl fmt::Debug for Verifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Verifier").finish_non_exhaustive()
    }
}

/// Type-level proof that a signature passed cryptographic verification.
///
/// This value does not prove that the key has federation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedSignature<R: SigningRole> {
    public_key: PublicKeyId,
    domain_version: SignatureDomainVersion,
    message_digest: MessageDigest,
    role: PhantomData<fn() -> R>,
}

impl<R: SigningRole> VerifiedSignature<R> {
    /// Returns the cryptographically verified public-key identity.
    #[must_use]
    pub const fn public_key(self) -> PublicKeyId {
        self.public_key
    }

    /// Returns the verified signature-domain version.
    #[must_use]
    pub const fn domain_version(self) -> SignatureDomainVersion {
        self.domain_version
    }

    /// Returns the exact canonical message digest authenticated by the proof.
    #[must_use]
    pub const fn message_digest(self) -> MessageDigest {
        self.message_digest
    }

    /// Returns the verified role.
    #[must_use]
    pub const fn role(self) -> SignatureRoleId {
        R::ROLE
    }
}

const fn request_for_signature<R: SigningRole>(
    digest: ScopedDigest<R>,
    signature: Signature<R>,
) -> VerificationRequest {
    VerificationRequest {
        public_key: signature.public_key,
        role: R::ROLE,
        domain_version: signature.domain_version,
        digest: *digest.as_bytes(),
        raw: signature.raw,
    }
}
