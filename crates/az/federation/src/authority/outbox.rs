use std::{future::Future, pin::Pin};

use crate::{ContentDigest, OperationId};

use super::StorageFailure;

/// One independently verifiable transparency publication step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationStep {
    /// Proof that the receipt commitment is present in an exact log checkpoint.
    LogInclusion(ContentDigest),
    /// Independent witness evidence for that checkpoint lineage.
    Witness(ContentDigest),
    /// Public-network evidence that commits to the checkpoint.
    Anchor(ContentDigest),
}

/// Closed status of one authority receipt's transparency obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationStatus {
    /// No log inclusion evidence is durable yet.
    Pending,
    /// Inclusion is durable; witness and anchor evidence remain outstanding.
    LogIncluded { inclusion: ContentDigest },
    /// Inclusion and witness evidence are durable; anchor remains outstanding.
    Witnessed {
        inclusion: ContentDigest,
        witness: ContentDigest,
    },
    /// Inclusion and anchor evidence are durable; witness remains outstanding.
    Anchored {
        inclusion: ContentDigest,
        anchor: ContentDigest,
    },
    /// Inclusion, witness, and anchor evidence are independently durable.
    Complete {
        inclusion: ContentDigest,
        witness: ContentDigest,
        anchor: ContentDigest,
    },
}

impl PublicationStatus {
    pub(crate) const fn is_complete(self) -> bool {
        matches!(self, Self::Complete { .. })
    }
}

/// Domain refusal from an otherwise available transparency outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationRefusal {
    /// No committed receipt owns the operation identity.
    UnknownOperation,
    /// Witness or anchor evidence arrived before log inclusion.
    MissingLogInclusion,
    /// The same publication step supplied different immutable evidence.
    EvidenceConflict,
}

/// Observable result of recording one publication step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationOutcome {
    /// New evidence advanced the durable publication status.
    Advanced(PublicationStatus),
    /// The identical evidence was already durable.
    Existing(PublicationStatus),
    /// Available storage refused an invalid domain transition.
    Refused(PublicationRefusal),
}

/// Asynchronous result from a transparency outbox adapter.
pub type TransparencyOutboxFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PublicationOutcome, StorageFailure>> + Send + 'a>>;

/// Asynchronous lookup of one publication obligation.
pub type TransparencyLoadFuture<'a> = Pin<
    Box<dyn Future<Output = Result<Option<TransparencyPublication>, StorageFailure>> + Send + 'a>,
>;

/// Immutable view of one receipt's current publication obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransparencyPublication {
    operation: OperationId,
    commitment: ContentDigest,
    status: PublicationStatus,
}

impl TransparencyPublication {
    /// Returns the authority operation that created the obligation.
    #[must_use]
    pub const fn operation(self) -> OperationId {
        self.operation
    }

    /// Returns the randomized receipt commitment to publish.
    #[must_use]
    pub const fn commitment(self) -> ContentDigest {
        self.commitment
    }

    /// Returns the independently advancing publication status.
    #[must_use]
    pub const fn status(self) -> PublicationStatus {
        self.status
    }
}

/// Consumer-owned durable publication boundary.
pub trait TransparencyOutboxPort: Send + Sync {
    /// Loads one exact obligation for a durable worker or reconciliation call.
    fn load_publication(&self, operation: OperationId) -> TransparencyLoadFuture<'_>;

    /// Records one independently verified publication step idempotently.
    fn record_publication(
        &self,
        operation: OperationId,
        step: PublicationStep,
    ) -> TransparencyOutboxFuture<'_>;
}

#[derive(Debug, Clone, Copy)]
pub struct TransparencyOutboxItem {
    pub commitment: ContentDigest,
    pub status: PublicationStatus,
}

impl TransparencyOutboxItem {
    pub const fn pending(commitment: ContentDigest) -> Self {
        Self {
            commitment,
            status: PublicationStatus::Pending,
        }
    }

    pub const fn view(self, operation: OperationId) -> TransparencyPublication {
        TransparencyPublication {
            operation,
            commitment: self.commitment,
            status: self.status,
        }
    }

    pub(crate) fn record(&mut self, step: PublicationStep) -> PublicationOutcome {
        let current = self.status;
        let next = match (current, step) {
            (PublicationStatus::Pending, PublicationStep::LogInclusion(inclusion)) => {
                PublicationStatus::LogIncluded { inclusion }
            }
            (PublicationStatus::Pending, _) => {
                return PublicationOutcome::Refused(PublicationRefusal::MissingLogInclusion);
            }
            (PublicationStatus::LogIncluded { inclusion }, PublicationStep::Witness(witness)) => {
                PublicationStatus::Witnessed { inclusion, witness }
            }
            (PublicationStatus::LogIncluded { inclusion }, PublicationStep::Anchor(anchor)) => {
                PublicationStatus::Anchored { inclusion, anchor }
            }
            (
                PublicationStatus::Witnessed { inclusion, witness },
                PublicationStep::Anchor(anchor),
            )
            | (
                PublicationStatus::Anchored { inclusion, anchor },
                PublicationStep::Witness(witness),
            ) => PublicationStatus::Complete {
                inclusion,
                witness,
                anchor,
            },
            (status, repeated) => return compare_existing(status, repeated),
        };
        self.status = next;
        PublicationOutcome::Advanced(next)
    }
}

fn compare_existing(status: PublicationStatus, repeated: PublicationStep) -> PublicationOutcome {
    let matches = match (status, repeated) {
        (
            PublicationStatus::LogIncluded { inclusion }
            | PublicationStatus::Witnessed { inclusion, .. }
            | PublicationStatus::Anchored { inclusion, .. }
            | PublicationStatus::Complete { inclusion, .. },
            PublicationStep::LogInclusion(candidate),
        ) => inclusion == candidate,
        (
            PublicationStatus::Witnessed { witness, .. }
            | PublicationStatus::Complete { witness, .. },
            PublicationStep::Witness(candidate),
        ) => witness == candidate,
        (
            PublicationStatus::Anchored { anchor, .. } | PublicationStatus::Complete { anchor, .. },
            PublicationStep::Anchor(candidate),
        ) => anchor == candidate,
        _ => false,
    };
    if matches {
        PublicationOutcome::Existing(status)
    } else {
        PublicationOutcome::Refused(PublicationRefusal::EvidenceConflict)
    }
}
