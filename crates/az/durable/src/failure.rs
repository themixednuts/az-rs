/// Provider-neutral inability to satisfy a durable operation safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StorageFailure {
    /// The configured store is temporarily unavailable.
    #[error("durable storage is unavailable")]
    Unavailable,
    /// Stored state cannot be trusted until explicit recovery succeeds.
    #[error("durable storage requires recovery")]
    RecoveryRequired,
}

/// Closed delivery failure classification persisted by Durable Outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureDisposition {
    /// Delivery can never succeed under the current obligation.
    Terminal,
    /// Delivery may be retried only within its stored policy.
    Retryable,
}

/// Invalid bounded retry policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RetryPolicyError {
    /// Attempt count exceeded the global hard cap.
    #[error("retry policy allows {actual} attempts; maximum is {maximum}")]
    AttemptsExceedHardCap {
        /// Requested attempts including the first delivery.
        actual: u16,
        /// Global accepted maximum.
        maximum: u16,
    },
}

/// Durable state version exhausted its monotonic range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("durable state version is exhausted")]
pub struct DurableStateVersionError;

/// Writer fence exhausted its monotonic range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("writer fence is exhausted")]
pub struct WriterFenceError;
