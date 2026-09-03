//! Provider-neutral fenced durable-subject primitives.

#![forbid(unsafe_code)]

mod bounds;
mod codec;
mod deadline;
mod failure;
mod identity;
mod journal;
mod outbox;
mod ports;
mod profile;
#[allow(
    clippy::needless_pass_by_value,
    reason = "durable port calls transfer owned typed requests across async boundaries"
)]
#[allow(
    clippy::significant_drop_tightening,
    reason = "the reference store holds each mutex guard through one atomic transition and receipt"
)]
mod reference;
mod subject;

pub use codec::{
    CanonicalBytes, CodecError, CodecId, CodecVersion, ContentDigest, DurableCodec,
    MAX_CANONICAL_VALUE_BYTES,
};
pub use deadline::AbsoluteDeadline;
pub use failure::{
    DurableStateVersionError, FailureDisposition, RetryPolicyError, StorageFailure,
    WriterFenceError,
};
pub use identity::{
    BoundedDiagnosticId, DiagnosticIdError, DurableNamespaceId, DurableNamespaceRegistry,
    DurableSubject, DurableSubjectId, DurableSubjectKey, EffectId, ErasedDurableSubjectId,
    InvalidDurableIdentity, NamespaceRegistrationError, OperationId, RecordId,
};
pub use journal::{
    DurableRecord, DurableSnapshot, JournalPage, JournalRecord, RecordsAfter, ReplayError,
    ReplayPolicy, RetentionGap, SnapshotRequest, VerifiedReplay, append_record, verify_replay,
};
pub use outbox::{
    ClaimEffects, ClaimedEffect, ClaimedEffectPage, DurableMessage, EffectFinish,
    EffectFinishRefusal, EffectFinishResult, EffectReceipt, EffectScope, FinishEffect,
    MAX_RETRY_ATTEMPTS, PagePressure, PoisonedEffect, RetryPolicy, enqueue_effect,
};
pub use ports::{JournalReadPort, OutboxDeliveryPort, SnapshotReadPort, SubjectStatePort};
pub use profile::{
    ConformanceEvidenceRef, DurableProfileId, DurableProfileRequirement, InvalidDurableProfileId,
    ProfileMismatch, ProvenProfileRef,
};
pub use reference::{InMemorySubject, InMemorySubjectFault};
pub use subject::{
    AcquireWriter, AcquireWriterResult, ActiveSubject, CommitReceipt, CommitRefusal, CommitResult,
    CommitSubject, CreateSubject, DurableStateVersion, LoadedSubject, MAX_MUTATION_CLAUSES,
    MutationBuildError, OperationCapacity, OperationCapacityError, OperationLookup, RetireSubject,
    RetiredSubject, SubjectLifecycle, SubjectMutation, SubjectOperationKey, SubjectOperationResult,
    WriterAcquisitionRefusal, WriterFence, WriterGrantReceipt, WriterHolder, WriterHolderKind,
    WriterPermit,
};

/// A writer permit cannot be cloned into concurrent local writers.
///
/// ```compile_fail
/// # use az_durable::{DurableSubject, WriterPermit};
/// # fn duplicate<T: DurableSubject>(permit: WriterPermit<T>) {
/// let second = permit.clone();
/// # }
/// ```
const _WRITER_PERMIT_IS_LINEAR: () = ();
pub use bounds::{BoundError, BoundedPage, BoundedPageSize, MAX_PAGE_ITEMS};
