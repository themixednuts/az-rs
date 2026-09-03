//! Role-bound key handles and the custody seam that opens them.
//!
//! Linear ADR 0038 keeps credential resolution in `az-secrets` and key custody
//! inside a provider adapter. This module owns the narrow seam between them: a
//! composition root names exactly one role's key with a [`SigningKeyHandle`],
//! custody resolves that credential and binds its provider-private handle, and
//! the caller receives an attested [`Signer`] for that role only.
//!
//! No credential, key material, provider type, or resolver reaches this
//! contract. A game server therefore cannot obtain a federation root resolver
//! or a role-erased signer by holding a handle.
//!
//! A game-server composition root may open only its host-evidence signer:
//!
//! ```compile_fail
//! use az_federation_crypto::{
//!     AuthorityReceipt, SigningKeyCustody, SigningKeyHandle, open_game_server_signer,
//! };
//!
//! async fn open(
//!     custody: &dyn SigningKeyCustody<AuthorityReceipt>,
//!     handle: &SigningKeyHandle<AuthorityReceipt>,
//! ) {
//!     let _ = open_game_server_signer(custody, handle).await;
//! }
//! ```
//!
//! Custody for one role also refuses another role's handle:
//!
//! ```compile_fail
//! use az_federation_crypto::{
//!     AuthorityReceipt, HostEvidence, SigningKeyCustody, SigningKeyHandle, open_role_signer,
//! };
//!
//! async fn open(
//!     custody: &dyn SigningKeyCustody<HostEvidence>,
//!     authority: &SigningKeyHandle<AuthorityReceipt>,
//! ) {
//!     let _ = open_role_signer(custody, authority).await;
//! }
//! ```

use std::{fmt, future::Future, marker::PhantomData, pin::Pin, sync::Arc};

use arrayvec::ArrayString;

use crate::{HostEvidence, SignatureRoleId, Signer, SigningFailure, SigningPort, SigningRole};

/// Maximum canonical byte length of a signing-key reference.
pub const MAX_SIGNING_KEY_REF_BYTES: usize = 128;

/// A signing-key reference is empty, oversized, or contains a noncanonical byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SigningKeyRefError {
    /// A reference must name a key.
    #[error("signing-key reference must not be empty")]
    Empty,
    /// A reference exceeds the bound applied before any adapter sees it.
    #[error("signing-key reference exceeds {MAX_SIGNING_KEY_REF_BYTES} bytes")]
    TooLong,
    /// A reference contains whitespace, a control byte, or a non-ASCII byte.
    #[error("signing-key reference contains a noncanonical byte")]
    InvalidByte,
}

/// Provider-neutral name of exactly one signing key.
///
/// A key reference is a name, never key material. Its deployment meaning belongs to
/// the custody adapter: ADR 0038 gives `az-secrets` the `secret://` reference
/// grammar and mount routing, and other adapters may use their own naming.
/// This type only bounds and canonicalizes the name before it leaves the
/// composition root, so an unbounded or control-byte name cannot reach a
/// provider.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SigningKeyRef(ArrayString<MAX_SIGNING_KEY_REF_BYTES>);

impl SigningKeyRef {
    /// Parses a bounded, printable ASCII key name.
    ///
    /// # Errors
    ///
    /// Rejects empty and oversized names, and any byte outside printable
    /// ASCII, so whitespace and control bytes cannot reach a custody adapter.
    pub fn parse(value: &str) -> Result<Self, SigningKeyRefError> {
        if value.is_empty() {
            return Err(SigningKeyRefError::Empty);
        }
        if value.len() > MAX_SIGNING_KEY_REF_BYTES {
            return Err(SigningKeyRefError::TooLong);
        }
        if !value.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(SigningKeyRefError::InvalidByte);
        }
        let mut canonical = ArrayString::new();
        canonical
            .try_push_str(value)
            .map_err(|_| SigningKeyRefError::TooLong)?;
        Ok(Self(canonical))
    }

    /// Returns the canonical reference text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A composition root's reference to exactly one role's signing key.
///
/// The role travels in the type, so a host handle cannot be presented to
/// authority, history, or root-recovery custody.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SigningKeyHandle<R: SigningRole> {
    key_ref: SigningKeyRef,
    role: PhantomData<fn() -> R>,
}

impl<R: SigningRole> SigningKeyHandle<R> {
    /// Names one role's key without naming its credential or material.
    #[must_use]
    pub const fn new(key_ref: SigningKeyRef) -> Self {
        Self {
            key_ref,
            role: PhantomData,
        }
    }

    /// Returns the stable protocol identity of the named role.
    #[must_use]
    pub const fn role(&self) -> SignatureRoleId {
        R::ROLE
    }

    /// Returns the provider-neutral key name.
    #[must_use]
    pub const fn key_ref(&self) -> &SigningKeyRef {
        &self.key_ref
    }
}

impl<R: SigningRole> fmt::Debug for SigningKeyHandle<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SigningKeyHandle")
            .field("role", &R::ROLE)
            .field("key_ref", &self.key_ref)
            .finish()
    }
}

/// A custody adapter's asynchronous bind result.
pub type OpenKeyCustodyFuture<'a, R> =
    Pin<Box<dyn Future<Output = Result<Arc<dyn SigningPort<R>>, SigningFailure>> + Send + 'a>>;

/// Consumer-owned custody boundary for exactly one signing role.
///
/// An implementation resolves the credential for `handle` through whatever
/// secret backend its deployment routes to, binds the provider-private key
/// handle, and returns a port for that role. It returns no credential, key
/// material, provider client, or resolver, so a caller can neither rebind the
/// key to another role nor widen its own custody.
pub trait SigningKeyCustody<R: SigningRole>: Send + Sync {
    /// Resolves this role's credential and binds its provider-private handle.
    fn open_key<'a>(&'a self, handle: &'a SigningKeyHandle<R>) -> OpenKeyCustodyFuture<'a, R>;
}

/// Opens exactly one role's signer through custody and startup attestation.
///
/// Custody failure and provider attestation failure both fail closed, so an
/// executable never reaches readiness holding an unusable or mislabeled key.
///
/// # Errors
///
/// Returns the typed custody failure, or the provider's startup-attestation
/// failure reported through [`Signer::open`].
pub async fn open_role_signer<R: SigningRole>(
    custody: &dyn SigningKeyCustody<R>,
    handle: &SigningKeyHandle<R>,
) -> Result<Signer<R>, SigningFailure> {
    let provider = custody.open_key(handle).await?;
    Signer::open(provider).await
}

/// A signing role a certified game server may hold custody for.
///
/// ADR 0038 gives a game server only its own host/evidence signer. The trait
/// is closed: [`SigningRole`] is sealed, and both this trait and every role
/// type are owned here, so no downstream crate can widen the set.
pub trait GameServerRole: SigningRole {}

impl GameServerRole for HostEvidence {}

/// Opens the only signer a certified game server may hold.
///
/// This is [`open_role_signer`] narrowed to [`GameServerRole`], so a game
/// server's composition root cannot compile against authority-receipt,
/// history, or root-recovery custody.
///
/// # Errors
///
/// Returns the typed custody or startup-attestation failure.
pub async fn open_game_server_signer<R: GameServerRole>(
    custody: &dyn SigningKeyCustody<R>,
    handle: &SigningKeyHandle<R>,
) -> Result<Signer<R>, SigningFailure> {
    open_role_signer(custody, handle).await
}
