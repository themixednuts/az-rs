use std::num::NonZeroU8;

use arrayvec::ArrayVec;
use az_federation_crypto::{MessageDigest, PublicKeyId, RootRecovery, VerifiedSignature};

use crate::{AuthorityRecoveryStatement, ContentDigest};

/// Maximum root keys in one threshold-ready recovery policy.
pub const MAX_ROOT_RECOVERY_KEYS: usize = 5;

/// Invalid root-recovery key policy configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RootRecoveryPolicyError {
    /// At least one separately custodied root key is required.
    #[error("root recovery policy requires at least one key")]
    EmptyKeySet,
    /// Version one deliberately caps custody at five keys.
    #[error("root recovery policy has {actual} keys, maximum is {maximum}")]
    TooManyKeys { actual: usize, maximum: usize },
    /// A zero threshold would authorize recovery without a root holder.
    #[error("root recovery threshold must be nonzero")]
    ZeroThreshold,
    /// The configured threshold cannot be reached by the configured keys.
    #[error("root recovery threshold {threshold} exceeds key count {key_count}")]
    ThresholdExceedsKeyCount { threshold: u8, key_count: usize },
    /// Counting the same key twice would fake independent custody.
    #[error("root recovery policy contains a duplicate key")]
    DuplicateKey,
}

/// Bounded, versioned key authorization for forward root recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRecoveryKeyPolicy {
    digest: ContentDigest,
    threshold: NonZeroU8,
    keys: ArrayVec<PublicKeyId, MAX_ROOT_RECOVERY_KEYS>,
}

impl RootRecoveryKeyPolicy {
    /// Creates one policy after validating its threshold and distinct key set.
    ///
    /// # Errors
    ///
    /// Rejects an empty, oversized, duplicate, zero-threshold, or impossible
    /// policy before it can authorize recovery.
    pub fn new(
        digest: ContentDigest,
        threshold: u8,
        keys: &[PublicKeyId],
    ) -> Result<Self, RootRecoveryPolicyError> {
        if keys.is_empty() {
            return Err(RootRecoveryPolicyError::EmptyKeySet);
        }
        if keys.len() > MAX_ROOT_RECOVERY_KEYS {
            return Err(RootRecoveryPolicyError::TooManyKeys {
                actual: keys.len(),
                maximum: MAX_ROOT_RECOVERY_KEYS,
            });
        }
        let Some(threshold) = NonZeroU8::new(threshold) else {
            return Err(RootRecoveryPolicyError::ZeroThreshold);
        };
        if usize::from(threshold.get()) > keys.len() {
            return Err(RootRecoveryPolicyError::ThresholdExceedsKeyCount {
                threshold: threshold.get(),
                key_count: keys.len(),
            });
        }

        let mut authorized = ArrayVec::new();
        for key in keys {
            if authorized.contains(key) {
                return Err(RootRecoveryPolicyError::DuplicateKey);
            }
            authorized.push(*key);
        }
        Ok(Self {
            digest,
            threshold,
            keys: authorized,
        })
    }
}

/// Policy refusal while authorizing one canonical recovery digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RootRecoveryRefusal {
    /// The statement names a different immutable recovery policy.
    #[error("root recovery statement names a different policy")]
    PolicyMismatch,
    /// More approvals were supplied than the policy can authorize.
    #[error("root recovery supplied {actual} approvals, policy has {maximum} keys")]
    TooManyApprovals { actual: usize, maximum: usize },
    /// A signature authenticated a different recovery statement.
    #[error("root recovery signature authenticates a different message")]
    MessageMismatch,
    /// A valid cryptographic key has no authority under this policy.
    #[error("root recovery signature key is not authorized by policy")]
    UnauthorizedKey,
    /// One custodian's key cannot count more than once.
    #[error("root recovery contains a duplicate approval")]
    DuplicateApproval,
    /// Distinct authorized approvals did not reach the policy threshold.
    #[error("root recovery has {actual} approvals, requires {required}")]
    InsufficientApprovals { actual: usize, required: u8 },
}

/// Type-level proof that one digest reached its root-recovery threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootRecoveryQuorum {
    statement: AuthorityRecoveryStatement,
    message: MessageDigest,
    policy_digest: ContentDigest,
    approval_count: u8,
}

impl RootRecoveryQuorum {
    /// Returns the exact recovery statement authorized by the quorum.
    #[must_use]
    pub const fn statement(self) -> AuthorityRecoveryStatement {
        self.statement
    }

    /// Returns the only canonical message authorized by this quorum.
    #[must_use]
    pub const fn message_digest(self) -> MessageDigest {
        self.message
    }

    /// Returns the immutable key policy used to authorize the message.
    #[must_use]
    pub const fn policy_digest(self) -> ContentDigest {
        self.policy_digest
    }

    /// Returns the number of distinct authorized custodians counted.
    #[must_use]
    pub const fn approval_count(self) -> u8 {
        self.approval_count
    }
}

/// Applies recovery policy to cryptographically verified root signatures.
///
/// # Errors
///
/// Refuses wrong-message, unauthorized, duplicate, oversized, or insufficient
/// approvals. Cryptographic verification must already have succeeded.
pub fn authorize_root_recovery(
    policy: &RootRecoveryKeyPolicy,
    statement: AuthorityRecoveryStatement,
    message: MessageDigest,
    approvals: &[VerifiedSignature<RootRecovery>],
) -> Result<RootRecoveryQuorum, RootRecoveryRefusal> {
    if statement.recovery_policy() != policy.digest {
        return Err(RootRecoveryRefusal::PolicyMismatch);
    }
    if approvals.len() > policy.keys.len() {
        return Err(RootRecoveryRefusal::TooManyApprovals {
            actual: approvals.len(),
            maximum: policy.keys.len(),
        });
    }

    let mut counted = ArrayVec::<PublicKeyId, MAX_ROOT_RECOVERY_KEYS>::new();
    for approval in approvals {
        if approval.message_digest() != message {
            return Err(RootRecoveryRefusal::MessageMismatch);
        }
        let key = approval.public_key();
        if !policy.keys.contains(&key) {
            return Err(RootRecoveryRefusal::UnauthorizedKey);
        }
        if counted.contains(&key) {
            return Err(RootRecoveryRefusal::DuplicateApproval);
        }
        counted.push(key);
    }

    if counted.len() < usize::from(policy.threshold.get()) {
        return Err(RootRecoveryRefusal::InsufficientApprovals {
            actual: counted.len(),
            required: policy.threshold.get(),
        });
    }

    let approval_count =
        u8::try_from(counted.len()).map_err(|_| RootRecoveryRefusal::TooManyApprovals {
            actual: counted.len(),
            maximum: MAX_ROOT_RECOVERY_KEYS,
        })?;
    Ok(RootRecoveryQuorum {
        statement,
        message,
        policy_digest: policy.digest,
        approval_count,
    })
}
