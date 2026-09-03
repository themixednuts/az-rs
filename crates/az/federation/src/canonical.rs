use serde::{Deserialize, Serialize};

/// Digest of one complete, versioned canonical authority request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonicalRequestDigest([u8; 32]);

impl CanonicalRequestDigest {
    /// Restores a digest produced by the canonical protocol codec.
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
