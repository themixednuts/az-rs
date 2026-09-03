use std::future::Future;

use crate::{
    AcquireWriter, AcquireWriterResult, ClaimEffects, ClaimedEffectPage, CommitResult,
    CommitSubject, ContentDigest, CreateSubject, DurableMessage, DurableRecord, DurableSnapshot,
    DurableSubject, DurableSubjectId, EffectFinishResult, FinishEffect, JournalPage, LoadedSubject,
    OperationLookup, RecordsAfter, RetireSubject, SnapshotRequest, StorageFailure,
    SubjectOperationKey,
};

/// Consumer-owned asynchronous Subject State atom.
pub trait SubjectStatePort<T: DurableSubject>: Send + Sync {
    /// Loads without creating.
    fn load(
        &self,
        subject: DurableSubjectId<T>,
    ) -> impl Future<Output = Result<LoadedSubject<T>, StorageFailure>> + Send;

    /// Creates one previously absent subject.
    fn create(
        &self,
        request: CreateSubject<T>,
    ) -> impl Future<Output = Result<CommitResult<T>, StorageFailure>> + Send;

    /// Acquires or idempotently replays one writer grant.
    fn acquire_writer(
        &self,
        request: AcquireWriter<T>,
    ) -> impl Future<Output = Result<AcquireWriterResult<T>, StorageFailure>> + Send;

    /// Looks up an operation after an unknown response.
    fn lookup(
        &self,
        key: SubjectOperationKey<T>,
        canonical_input_digest: ContentDigest,
    ) -> impl Future<Output = Result<OperationLookup<T>, StorageFailure>> + Send;

    /// Commits one fenced state mutation.
    fn commit(
        &self,
        request: CommitSubject<T>,
    ) -> impl Future<Output = Result<CommitResult<T>, StorageFailure>> + Send;

    /// Retires one subject permanently.
    fn retire(
        &self,
        request: RetireSubject<T>,
    ) -> impl Future<Output = Result<CommitResult<T>, StorageFailure>> + Send;
}

/// Consumer-owned asynchronous reads of typed durable snapshots.
pub trait SnapshotReadPort<T: DurableSubject>: Send + Sync {
    /// Loads the newest available snapshot at or before the requested version.
    fn snapshot(
        &self,
        request: SnapshotRequest<T>,
    ) -> impl Future<Output = Result<Option<DurableSnapshot<T>>, StorageFailure>> + Send;
}

/// Consumer-owned asynchronous reads from one typed subject journal.
pub trait JournalReadPort<E: DurableRecord>: Send + Sync {
    /// Loads one bounded page strictly after an exact record cursor.
    fn records_after(
        &self,
        request: RecordsAfter<E>,
    ) -> impl Future<Output = Result<JournalPage<E>, StorageFailure>> + Send;
}

/// Consumer-owned asynchronous delivery of one typed outbox message family.
pub trait OutboxDeliveryPort<M: DurableMessage>: Send + Sync {
    /// Claims one bounded due page under monotonic per-effect attempt fences.
    fn claim(
        &self,
        request: ClaimEffects<M>,
    ) -> impl Future<Output = Result<ClaimedEffectPage<M>, StorageFailure>> + Send;

    /// Persists an application receipt, exact retry time, or poison disposition.
    fn finish(
        &self,
        request: FinishEffect<M>,
    ) -> impl Future<Output = Result<EffectFinishResult, StorageFailure>> + Send;
}
