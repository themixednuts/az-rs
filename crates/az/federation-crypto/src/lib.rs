//! Role-scoped signing contracts for Certified Federation.
//!
//! A provider adapter owns its key handle and implements [`SigningPort`] for
//! exactly the role it is configured to serve. [`Signer::open`] performs the
//! adapter's fallible startup attestation before a signer becomes available.
//!
//! Role typing prevents accidental authority substitution inside a Rust
//! composition root. It does not grant federation authority: an independent
//! verifier must still prove that the public key is authorized for the role,
//! scope, and epoch named by a signed grant.
//!
//! The version-one scoped digest is precisely:
//!
//! ```text
//! BLAKE3 derive_key(
//!   "azoth certified federation signature domain v1",
//!   LE64(role_label_length) || role_label || canonical_message_digest_32
//! )
//! ```
//!
//! The context and role labels are public below so independent implementations
//! can reproduce the fixed vectors.
//!
//! A host-role provider cannot be passed to an authority signer:
//!
//! ```compile_fail
//! use std::sync::Arc;
//! use az_federation_crypto::{AuthorityReceipt, HostEvidence, SigningPort};
//!
//! fn needs_authority(_: Arc<dyn SigningPort<AuthorityReceipt>>) {}
//!
//! fn host_process(host: Arc<dyn SigningPort<HostEvidence>>) {
//!     needs_authority(host);
//! }
//! ```
//!
//! A role-specific port also rejects another role's scoped digest:
//!
//! ```compile_fail
//! use az_federation_crypto::{AuthorityReceipt, HostEvidence, ScopedDigest, SigningPort};
//!
//! fn wrong_digest(
//!     host: &dyn SigningPort<HostEvidence>,
//!     authority: ScopedDigest<AuthorityReceipt>,
//! ) {
//!     host.sign(authority);
//! }
//! ```
//!
//! The resulting signers also remain distinct types:
//!
//! ```compile_fail
//! use az_federation_crypto::{AuthorityReceipt, HostEvidence, Signer};
//!
//! fn mint_authority_receipt(_: &Signer<AuthorityReceipt>) {}
//!
//! fn host_process(host_signer: &Signer<HostEvidence>) {
//!     mint_authority_receipt(host_signer);
//! }
//! ```

use std::{fmt, future::Future, marker::PhantomData, pin::Pin, sync::Arc};

mod key_handle;
mod transparency;
mod verification;

pub use key_handle::{
    GameServerRole, MAX_SIGNING_KEY_REF_BYTES, OpenKeyCustodyFuture, SigningKeyCustody,
    SigningKeyHandle, SigningKeyRef, SigningKeyRefError, open_game_server_signer, open_role_signer,
};
pub use transparency::{
    CommitmentNonce, ForkEvidence, ForkReason, HISTORY_CHECKPOINT_V1_CONTEXT, HistoryCheckpoint,
    HistoryConsistencyProof, HistoryError, HistoryRoot, HistoryWitnessMonitor, MerkleHistory,
    MonitorCheckpointOutcome, MonitorError, RECEIPT_COMMITMENT_V1_CONTEXT, ReceiptCommitment,
    ReceiptDigest, ReceiptInclusionProof, SignedHistoryCheckpoint, TRANSPARENCY_MERKLE_V1_CONTEXT,
    VerifiedHistoryCheckpoint, sign_history_checkpoint, verify_history_checkpoint,
    verify_history_consistency, verify_receipt_inclusion,
};
pub use verification::{
    Ed25519Verifier, VerificationFailure, VerificationFuture, VerificationPort,
    VerificationRequest, VerifiedSignature, Verifier,
};

/// BLAKE3 derive-key context for [`SignatureDomainVersion::V1`].
pub const SIGNATURE_DOMAIN_V1_CONTEXT: &str = "azoth certified federation signature domain v1";

mod sealed {
    pub trait Sealed {}
}

/// Version of the canonical signature-domain construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignatureDomainVersion {
    /// The BLAKE3 construction documented at crate level.
    V1,
}

/// Stable protocol identity of a signature role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignatureRoleId {
    /// Evidence produced by a game-server host.
    HostEvidence,
    /// Receipt produced by the Federation Authority.
    AuthorityReceipt,
    /// Checkpoint produced by the authoritative transparency log.
    HistoryLog,
    /// Checkpoint independently stored and countersigned by a witness.
    HistoryWitness,
    /// Offline or threshold approval for an explicit root/fork recovery.
    RootRecovery,
}

impl SignatureRoleId {
    /// Returns the exact role label used by the version-one digest construction.
    #[must_use]
    pub const fn canonical_label_v1(self) -> &'static [u8] {
        match self {
            Self::HostEvidence => b"host-evidence",
            Self::AuthorityReceipt => b"authority-receipt",
            Self::HistoryLog => b"history-log",
            Self::HistoryWitness => b"history-witness",
            Self::RootRecovery => b"root-recovery",
        }
    }
}

/// A closed Certified Federation signature role.
pub trait SigningRole: sealed::Sealed + Copy + Send + Sync + 'static {
    /// Stable protocol identity of this role.
    const ROLE: SignatureRoleId;
}

/// Evidence signed by a game-server host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostEvidence;

impl sealed::Sealed for HostEvidence {}

impl SigningRole for HostEvidence {
    const ROLE: SignatureRoleId = SignatureRoleId::HostEvidence;
}

/// An authority's receipt for an accepted federation operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityReceipt;

impl sealed::Sealed for AuthorityReceipt {}

impl SigningRole for AuthorityReceipt {
    const ROLE: SignatureRoleId = SignatureRoleId::AuthorityReceipt;
}

/// A checkpoint emitted by the authoritative transparency log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLog;

impl sealed::Sealed for HistoryLog {}

impl SigningRole for HistoryLog {
    const ROLE: SignatureRoleId = SignatureRoleId::HistoryLog;
}

/// A checkpoint countersigned by an independent history witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryWitness;

impl sealed::Sealed for HistoryWitness {}

impl SigningRole for HistoryWitness {
    const ROLE: SignatureRoleId = SignatureRoleId::HistoryWitness;
}

/// A role authorized to sign the exact canonical history-checkpoint statement.
pub trait HistoryCheckpointRole: SigningRole {}

impl HistoryCheckpointRole for HistoryLog {}
impl HistoryCheckpointRole for HistoryWitness {}

/// Offline or threshold approval of a forward-only authority recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootRecovery;

impl sealed::Sealed for RootRecovery {}

impl SigningRole for RootRecovery {
    const ROLE: SignatureRoleId = SignatureRoleId::RootRecovery;
}

/// A canonical digest produced by federation protocol code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageDigest([u8; 32]);

impl MessageDigest {
    /// Creates a digest from canonical bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the canonical digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A canonical digest after applying a versioned signature role domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopedDigest<R: SigningRole> {
    version: SignatureDomainVersion,
    bytes: [u8; 32],
    role: PhantomData<fn() -> R>,
}

impl<R: SigningRole> ScopedDigest<R> {
    /// Returns the domain-construction version.
    #[must_use]
    pub const fn version(&self) -> SignatureDomainVersion {
        self.version
    }

    /// Returns the role encoded into the digest.
    #[must_use]
    pub const fn role(&self) -> SignatureRoleId {
        R::ROLE
    }

    /// Returns the provider-ready digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

/// Applies the public version-one role domain to a canonical message digest.
#[must_use]
pub fn scope_message_digest<R: SigningRole>(message: MessageDigest) -> ScopedDigest<R> {
    let role = R::ROLE;
    let label = role.canonical_label_v1();
    let mut hasher = blake3::Hasher::new_derive_key(SIGNATURE_DOMAIN_V1_CONTEXT);
    hasher.update(&(label.len() as u64).to_le_bytes());
    hasher.update(label);
    hasher.update(message.as_bytes());
    ScopedDigest {
        version: SignatureDomainVersion::V1,
        bytes: *hasher.finalize().as_bytes(),
        role: PhantomData,
    }
}

/// Stable identity of the public key corresponding to a signer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKeyId([u8; 32]);

impl PublicKeyId {
    /// Creates a public-key identity from canonical bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the canonical identity bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Stable identity of a supported signature suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignatureSuite {
    /// Ed25519 under the first federation suite contract.
    Ed25519V1,
}

/// A bounded signature in a federation-approved suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawSignature {
    /// An Ed25519 signature under the first federation suite contract.
    Ed25519V1([u8; 64]),
}

impl RawSignature {
    /// Returns the suite required to verify this signature.
    #[must_use]
    pub const fn suite(&self) -> SignatureSuite {
        match self {
            Self::Ed25519V1(_) => SignatureSuite::Ed25519V1,
        }
    }
}

/// The actual key identity and signature returned by a provider operation.
///
/// Returning the key identity on every operation lets [`Signer`] reject a
/// provider alias or handle that changed after startup attestation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderSignature {
    public_key: PublicKeyId,
    raw: RawSignature,
}

impl ProviderSignature {
    /// Creates a provider result from the actual signing key and signature.
    #[must_use]
    pub const fn new(public_key: PublicKeyId, raw: RawSignature) -> Self {
        Self { public_key, raw }
    }

    /// Returns the actual key used by the provider operation.
    #[must_use]
    pub const fn public_key(&self) -> PublicKeyId {
        self.public_key
    }

    /// Returns the suite-specific signature.
    #[must_use]
    pub const fn raw(&self) -> RawSignature {
        self.raw
    }
}

/// A failure returned by a role-specific signing provider adapter.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SigningFailure {
    /// The provider denied opening or using the selected key.
    #[error("signing permission denied")]
    PermissionDenied,
    /// The provider could not currently serve the request.
    #[error("signing provider unavailable")]
    Unavailable,
    /// Provider configuration did not identify a usable key.
    #[error("signing provider configuration is invalid")]
    InvalidConfiguration,
    /// The provider rejected the configured role binding.
    #[error("signing provider rejected the role binding")]
    RoleBindingRejected,
    /// The provider used a different key than it attested at startup.
    #[error("signing provider key identity changed after startup attestation")]
    KeyIdentityChanged,
    /// The provider failed without a more specific public classification.
    #[error("signing operation failed")]
    Failed,
}

/// A provider adapter's asynchronous startup-attestation result.
pub type OpenSigningFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PublicKeyId, SigningFailure>> + Send + 'a>>;

/// A provider adapter's asynchronous signing result.
pub type SigningFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ProviderSignature, SigningFailure>> + Send + 'a>>;

/// A provider-private key binding for exactly one signing role.
///
/// The adapter retains its handle and credentials. `open` must authenticate the
/// provider session, verify permission and the configured role binding, resolve
/// the actual public-key identity, and fail closed before reporting readiness.
pub trait SigningPort<R: SigningRole>: Send + Sync {
    /// Attests that the role-specific key is ready and returns its actual ID.
    fn open(&self) -> OpenSigningFuture<'_>;

    /// Signs one digest already scoped to `R`.
    fn sign(&self, digest: ScopedDigest<R>) -> SigningFuture<'_>;
}

/// A provider-backed signer that can mint exactly one cryptographic role.
///
/// This is not proof that the key has federation authority. Verification must
/// separately resolve a signed role, scope, and epoch authorization for the
/// returned [`PublicKeyId`].
pub struct Signer<R: SigningRole> {
    provider: Arc<dyn SigningPort<R>>,
    public_key: PublicKeyId,
}

impl<R: SigningRole> Signer<R> {
    /// Opens and attests a role-specific provider key before returning ready.
    ///
    /// # Errors
    ///
    /// Returns a typed adapter failure when authentication, permission, role
    /// binding, key discovery, or provider readiness fails.
    pub async fn open(provider: Arc<dyn SigningPort<R>>) -> Result<Self, SigningFailure> {
        let public_key = provider.open().await?;
        Ok(Self {
            provider,
            public_key,
        })
    }

    /// Applies the role domain and asks the opened provider to sign it.
    ///
    /// # Errors
    ///
    /// Returns the typed failure supplied by the provider adapter.
    pub async fn sign(&self, message: MessageDigest) -> Result<Signature<R>, SigningFailure> {
        let digest = scope_message_digest::<R>(message);
        let provider_signature = self.provider.sign(digest).await?;
        if provider_signature.public_key() != self.public_key {
            return Err(SigningFailure::KeyIdentityChanged);
        }
        Ok(Signature {
            public_key: self.public_key,
            domain_version: digest.version(),
            raw: provider_signature.raw(),
            role: PhantomData,
        })
    }
}

impl<R: SigningRole> fmt::Debug for Signer<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Signer")
            .field("role", &R::ROLE)
            .field("public_key", &self.public_key)
            .finish_non_exhaustive()
    }
}

/// A cryptographic signature whose protocol role is preserved in its type.
///
/// Callers must not treat this value as an authority decision until a verifier
/// has validated both the signature and the key's signed role authorization.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature<R: SigningRole> {
    public_key: PublicKeyId,
    domain_version: SignatureDomainVersion,
    raw: RawSignature,
    role: PhantomData<fn() -> R>,
}

impl<R: SigningRole> Signature<R> {
    /// Restores an unverified signature from protocol or durable storage.
    ///
    /// The role is supplied by `R`; this constructor grants no authority and
    /// performs no cryptographic check. Pass the result to [`Verifier`].
    #[must_use]
    pub const fn from_parts(
        public_key: PublicKeyId,
        domain_version: SignatureDomainVersion,
        raw: RawSignature,
    ) -> Self {
        Self {
            public_key,
            domain_version,
            raw,
            role: PhantomData,
        }
    }

    /// Returns the identity of the key that made the signature.
    #[must_use]
    pub const fn public_key(&self) -> &PublicKeyId {
        &self.public_key
    }

    /// Returns the canonical signature-domain version.
    #[must_use]
    pub const fn domain_version(&self) -> SignatureDomainVersion {
        self.domain_version
    }

    /// Returns the role encoded by this signature type.
    #[must_use]
    pub const fn role(&self) -> SignatureRoleId {
        R::ROLE
    }

    /// Returns the signature suite.
    #[must_use]
    pub const fn suite(&self) -> SignatureSuite {
        self.raw.suite()
    }

    /// Returns the suite-specific signature.
    #[must_use]
    pub const fn raw(&self) -> &RawSignature {
        &self.raw
    }
}

impl<R: SigningRole> fmt::Debug for Signature<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Signature")
            .field("role", &R::ROLE)
            .field("domain_version", &self.domain_version)
            .field("public_key", &self.public_key)
            .field("raw", &self.raw)
            .finish()
    }
}
