//! Persistence and title-claims extension contracts.
//!
//! These let a project supply durable identity storage and title-specific
//! claims without the host knowing the storage backend or the game's claim
//! semantics.

use crate::identity::{Identity, ProviderSubject, SubjectId};
use crate::verification::{AuthError, ProviderClaims, VerifiedExternalIdentity};

/// Durable identity persistence. Supplied by a project storage contribution
/// (e.g. an STDB-backed store); the host resolves/creates the durable identity
/// for a verified provider subject.
pub trait IdentityStore: Send + Sync {
    /// Look up the durable identity a provider subject maps to, if any.
    ///
    /// # Errors
    /// Returns [`AuthError`] on a storage failure.
    fn resolve(&self, subject: &ProviderSubject) -> Result<Option<Identity>, AuthError>;

    /// Create (or return existing) a durable identity for a first-seen subject.
    ///
    /// # Errors
    /// Returns [`AuthError`] on a storage failure.
    fn get_or_create(&self, verified: &VerifiedExternalIdentity) -> Result<Identity, AuthError>;

    /// Link an additional provider subject to an existing identity.
    ///
    /// # Errors
    /// Returns [`AuthError`] if the identity is missing or the link violates
    /// policy.
    fn link(&self, id: &SubjectId, subject: ProviderSubject) -> Result<Identity, AuthError>;
}

/// Claims produced by a [`ClaimsAugmenter`]: extra scopes and provider-style
/// claims a title wants baked into the session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AugmentedClaims {
    pub scopes: Vec<crate::session::Scope>,
    pub claims: ProviderClaims,
}

/// Title-specific claim augmentation. Runs after identity resolution and before
/// signing; it may grant scopes or attach claims but cannot sign or mint.
pub trait ClaimsAugmenter: Send + Sync {
    /// Compute additional claims/scopes for a resolved identity.
    ///
    /// # Errors
    /// Returns [`AuthError`] if augmentation policy rejects the identity.
    fn augment(
        &self,
        identity: &Identity,
        verified: &VerifiedExternalIdentity,
    ) -> Result<AugmentedClaims, AuthError>;
}
