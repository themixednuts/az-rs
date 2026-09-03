use std::fmt;

use crate::{
    HistoryCheckpointRole, MessageDigest, PublicKeyId, Signature, SignatureRoleId, Signer,
    SigningFailure, VerificationFailure, VerifiedSignature, Verifier,
    transparency::HistoryCheckpoint,
};

/// Domain separator for the canonical history-checkpoint statement.
pub const HISTORY_CHECKPOINT_V1_CONTEXT: &str = "azoth certified federation history checkpoint v1";

impl HistoryCheckpoint {
    /// Returns the digest of the exact version-one checkpoint statement.
    ///
    /// The statement is `AZHC || BE16(1) || BE64(tree_size) || root_32` under
    /// [`HISTORY_CHECKPOINT_V1_CONTEXT`]. A signer applies its distinct role
    /// domain after this common statement digest.
    #[must_use]
    pub fn message_digest(self) -> MessageDigest {
        let mut hasher = blake3::Hasher::new_derive_key(HISTORY_CHECKPOINT_V1_CONTEXT);
        hasher.update(b"AZHC");
        hasher.update(&1_u16.to_be_bytes());
        hasher.update(&self.tree_size.to_be_bytes());
        hasher.update(self.root.as_bytes());
        MessageDigest::from_bytes(*hasher.finalize().as_bytes())
    }
}

/// One exact checkpoint statement and its role-preserving signature.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SignedHistoryCheckpoint<R: HistoryCheckpointRole> {
    checkpoint: HistoryCheckpoint,
    signature: Signature<R>,
}

impl<R: HistoryCheckpointRole> SignedHistoryCheckpoint<R> {
    /// Returns the signed checkpoint.
    #[must_use]
    pub const fn checkpoint(&self) -> HistoryCheckpoint {
        self.checkpoint
    }

    /// Returns the unverified role-specific signature.
    #[must_use]
    pub const fn signature(&self) -> &Signature<R> {
        &self.signature
    }
}

impl<R: HistoryCheckpointRole> fmt::Debug for SignedHistoryCheckpoint<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SignedHistoryCheckpoint")
            .field("checkpoint", &self.checkpoint)
            .field("signature", &self.signature)
            .finish()
    }
}

/// Signs one canonical checkpoint under an authorized checkpoint role.
///
/// # Errors
///
/// Returns the signer's typed provider failure. The checkpoint has no partial
/// signed representation on failure.
pub async fn sign_history_checkpoint<R: HistoryCheckpointRole>(
    checkpoint: HistoryCheckpoint,
    signer: &Signer<R>,
) -> Result<SignedHistoryCheckpoint<R>, SigningFailure> {
    let signature = signer.sign(checkpoint.message_digest()).await?;
    Ok(SignedHistoryCheckpoint {
        checkpoint,
        signature,
    })
}

/// Checkpoint whose role-specific signature passed cryptographic verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedHistoryCheckpoint<R: HistoryCheckpointRole> {
    signed: SignedHistoryCheckpoint<R>,
    verification: VerifiedSignature<R>,
}

impl<R: HistoryCheckpointRole> VerifiedHistoryCheckpoint<R> {
    /// Returns the verified checkpoint statement.
    #[must_use]
    pub const fn checkpoint(self) -> HistoryCheckpoint {
        self.signed.checkpoint
    }

    /// Returns the cryptographically verified key identity.
    #[must_use]
    pub const fn public_key(self) -> PublicKeyId {
        self.verification.public_key()
    }

    /// Returns the verified signature role.
    #[must_use]
    pub const fn role(self) -> SignatureRoleId {
        self.verification.role()
    }

    /// Returns the original signed statement for persistence or forwarding.
    #[must_use]
    pub const fn signed(self) -> SignedHistoryCheckpoint<R> {
        self.signed
    }
}

/// Verifies a signed checkpoint through the selected cryptographic adapter.
///
/// # Errors
///
/// Returns a typed cryptographic or availability failure. Key authorization is
/// deliberately outside this function.
pub async fn verify_history_checkpoint<R: HistoryCheckpointRole>(
    verifier: &Verifier,
    signed: SignedHistoryCheckpoint<R>,
) -> Result<VerifiedHistoryCheckpoint<R>, VerificationFailure> {
    let verification = verifier
        .verify(signed.checkpoint.message_digest(), signed.signature)
        .await?;
    Ok(VerifiedHistoryCheckpoint {
        signed,
        verification,
    })
}
