//! The release distribution port the lifecycle reconciler consumes.
//!
//! A launch revision pins the digest of a signed immutable manifest and its
//! complete product closure, so release identity here is a content digest
//! rather than a mutable name or a "latest" guess. Signature verification,
//! caching, and artifact transport belong to the adapter; the lifecycle only
//! learns whether a release is verifiable and runnable.

use crate::PortFuture;

/// Digest identity of one signed immutable release manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReleaseManifestDigest([u8; 32]);

impl ReleaseManifestDigest {
    /// Constructs a release identity from canonical manifest bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Digest identity of one artifact inside a release's product closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactDigest([u8; 32]);

impl ArtifactDigest {
    /// Constructs an artifact identity from its content digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A release whose signature and closure the adapter has verified.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VerifiedRelease {
    manifest: ReleaseManifestDigest,
    closure_digest: [u8; 32],
}

impl VerifiedRelease {
    /// Records a verified manifest and the digest covering its full closure.
    #[must_use]
    pub const fn new(manifest: ReleaseManifestDigest, closure_digest: [u8; 32]) -> Self {
        Self {
            manifest,
            closure_digest,
        }
    }

    /// Returns the verified manifest identity.
    #[must_use]
    pub const fn manifest(&self) -> ReleaseManifestDigest {
        self.manifest
    }

    /// Returns the digest covering the complete product closure.
    #[must_use]
    pub const fn closure_digest(&self) -> &[u8; 32] {
        &self.closure_digest
    }
}

/// Closed failures returned by a release distribution adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DistributionError {
    /// No manifest exists for the requested release identity.
    #[error("release manifest is unknown")]
    UnknownRelease,
    /// The manifest signature or its product closure failed verification.
    #[error("release manifest failed verification")]
    VerificationFailed,
    /// The distribution plane is unreachable or refusing work.
    #[error("release distribution is unavailable")]
    Unavailable,
}

/// Verified access to signed immutable releases and their artifacts.
pub trait ReleaseDistribution: Send + Sync {
    /// Resolves and verifies one signed release manifest and its closure.
    fn verify(
        &self,
        manifest: ReleaseManifestDigest,
    ) -> PortFuture<'_, Result<VerifiedRelease, DistributionError>>;

    /// Reports whether one closure artifact is materialized and intact.
    fn artifact_present(
        &self,
        artifact: ArtifactDigest,
    ) -> PortFuture<'_, Result<bool, DistributionError>>;
}
