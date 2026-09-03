use crate::{
    HistoryConsistencyProof, HistoryLog, PublicKeyId, VerifiedHistoryCheckpoint,
    verify_history_consistency,
};

/// Cryptographic reason an authorized log checkpoint froze a witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkReason {
    /// The log signed two roots for one tree size.
    ConflictingRootAtSameSize,
    /// A larger signed checkpoint did not extend the trusted checkpoint.
    InvalidConsistencyProof,
}

/// Durable evidence needed to diagnose or recover from a split view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForkEvidence {
    trusted: VerifiedHistoryCheckpoint<HistoryLog>,
    observed: VerifiedHistoryCheckpoint<HistoryLog>,
    reason: ForkReason,
}

impl ForkEvidence {
    /// Returns the checkpoint trusted before the conflict.
    #[must_use]
    pub const fn trusted(self) -> VerifiedHistoryCheckpoint<HistoryLog> {
        self.trusted
    }

    /// Returns the conflicting checkpoint signed by the expected log key.
    #[must_use]
    pub const fn observed(self) -> VerifiedHistoryCheckpoint<HistoryLog> {
        self.observed
    }

    /// Returns the fork classification.
    #[must_use]
    pub const fn reason(self) -> ForkReason {
        self.reason
    }
}

/// Successful result of observing an authorized log checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorCheckpointOutcome {
    /// The witness had already accepted this exact checkpoint.
    Existing(VerifiedHistoryCheckpoint<HistoryLog>),
    /// The witness advanced to a larger append-only checkpoint.
    Advanced(VerifiedHistoryCheckpoint<HistoryLog>),
}

/// Closed witness-monitor failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MonitorError {
    /// A cryptographically valid checkpoint used a key other than the expected log key.
    #[error("history checkpoint used unexpected log key")]
    UnexpectedLogKey {
        /// Configured authoritative log key.
        expected: PublicKeyId,
        /// Key that signed the observed checkpoint.
        observed: PublicKeyId,
    },
    /// A smaller checkpoint cannot move the witness backward.
    #[error("history checkpoint is stale")]
    StaleCheckpoint {
        /// Current witnessed size.
        current_size: u64,
        /// Observed older size.
        observed_size: u64,
    },
    /// A newly detected split view froze the monitor.
    #[error("history fork detected")]
    ForkDetected(Box<ForkEvidence>),
    /// The monitor was already frozen by earlier fork evidence.
    #[error("history witness is frozen")]
    Frozen(Box<ForkEvidence>),
}

/// Monotonic witness state for one configured authoritative history log.
///
/// The monitor accepts only cryptographically verified log checkpoints. It
/// never unfreezes itself. Root recovery must create a new authority epoch and
/// a new monitor from separately authorized recovery evidence.
#[derive(Debug, Clone, Copy)]
pub struct HistoryWitnessMonitor {
    expected_log_key: PublicKeyId,
    current: VerifiedHistoryCheckpoint<HistoryLog>,
    frozen: Option<ForkEvidence>,
}

impl HistoryWitnessMonitor {
    /// Starts a witness from one trusted, verified log checkpoint.
    ///
    /// # Errors
    ///
    /// Returns [`MonitorError::UnexpectedLogKey`] when the trusted checkpoint
    /// does not match the configured log key.
    pub fn new(
        expected_log_key: PublicKeyId,
        trusted: VerifiedHistoryCheckpoint<HistoryLog>,
    ) -> Result<Self, MonitorError> {
        if trusted.public_key() != expected_log_key {
            return Err(MonitorError::UnexpectedLogKey {
                expected: expected_log_key,
                observed: trusted.public_key(),
            });
        }
        Ok(Self {
            expected_log_key,
            current: trusted,
            frozen: None,
        })
    }

    /// Returns the latest accepted checkpoint.
    #[must_use]
    pub const fn current(self) -> VerifiedHistoryCheckpoint<HistoryLog> {
        self.current
    }

    /// Observes one verified checkpoint and its append-only proof.
    ///
    /// A conflicting root or invalid consistency proof stores fork evidence
    /// and permanently freezes this monitor instance.
    ///
    /// # Errors
    ///
    /// Returns a typed key, staleness, newly detected fork, or already-frozen
    /// failure without moving the trusted checkpoint backward.
    pub fn observe(
        &mut self,
        candidate: VerifiedHistoryCheckpoint<HistoryLog>,
        proof: &HistoryConsistencyProof,
    ) -> Result<MonitorCheckpointOutcome, MonitorError> {
        if let Some(evidence) = self.frozen {
            return Err(MonitorError::Frozen(Box::new(evidence)));
        }
        if candidate.public_key() != self.expected_log_key {
            return Err(MonitorError::UnexpectedLogKey {
                expected: self.expected_log_key,
                observed: candidate.public_key(),
            });
        }

        let current = self.current.checkpoint();
        let observed = candidate.checkpoint();
        if observed.tree_size() < current.tree_size() {
            return Err(MonitorError::StaleCheckpoint {
                current_size: current.tree_size(),
                observed_size: observed.tree_size(),
            });
        }
        if observed.tree_size() == current.tree_size() {
            if observed == current {
                return Ok(MonitorCheckpointOutcome::Existing(candidate));
            }
            return Err(self.freeze(candidate, ForkReason::ConflictingRootAtSameSize));
        }
        if !verify_history_consistency(current, observed, proof) {
            return Err(self.freeze(candidate, ForkReason::InvalidConsistencyProof));
        }

        self.current = candidate;
        Ok(MonitorCheckpointOutcome::Advanced(candidate))
    }

    #[allow(clippy::missing_const_for_fn)] // Boxing keeps the public error type small.
    fn freeze(
        &mut self,
        observed: VerifiedHistoryCheckpoint<HistoryLog>,
        reason: ForkReason,
    ) -> MonitorError {
        let evidence = ForkEvidence {
            trusted: self.current,
            observed,
            reason,
        };
        self.frozen = Some(evidence);
        MonitorError::ForkDetected(Box::new(evidence))
    }
}
