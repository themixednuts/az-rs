use std::{collections::BTreeMap, marker::PhantomData, num::NonZeroU64, sync::Mutex};

use crate::{
    AcquireWriter, AcquireWriterResult, ActiveSubject, CanonicalBytes, CommitReceipt,
    CommitRefusal, CommitResult, CommitSubject, ContentDigest, CreateSubject, DurableCodec,
    DurableRecord, DurableSnapshot, DurableStateVersion, DurableSubject, DurableSubjectId,
    EffectId, JournalPage, JournalReadPort, JournalRecord, LoadedSubject, OperationCapacity,
    OperationId, OperationLookup, ProvenProfileRef, RecordId, RecordsAfter, RetentionGap,
    RetireSubject, RetiredSubject, SnapshotReadPort, SnapshotRequest, StorageFailure,
    SubjectLifecycle, SubjectOperationKey, SubjectOperationResult, SubjectStatePort, WriterFence,
    WriterGrantReceipt, WriterPermit,
    bounds::BoundedPage,
    journal::{JournalClause, journal_genesis, record_hash, snapshot_digest},
};

mod journal;
mod outbox;

use self::outbox::StoredEffect;
use crate::outbox::OutboxClause;

/// Reference-only fault locations before one staged commit becomes visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InMemorySubjectFault {
    /// Fail after the complete shadow transaction is built but before publication.
    BeforePublish,
}

impl InMemorySubjectFault {
    /// Every modeled internal commit boundary.
    pub const ALL: [Self; 1] = [Self::BeforePublish];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoredOperationKind {
    Create,
    AcquireWriter,
    Commit,
    Retire,
}

#[derive(Debug, Clone)]
struct StoredCommitReceipt {
    operation: OperationId,
    input_digest: ContentDigest,
    version: DurableStateVersion,
    state_digest: ContentDigest,
}

#[derive(Debug, Clone)]
struct StoredWriterReceipt {
    operation: OperationId,
    input_digest: ContentDigest,
    fence: WriterFence,
    lease_generation: u64,
    lease_until: crate::AbsoluteDeadline,
}

#[derive(Debug, Clone)]
pub struct StoredJournalRecord {
    id: RecordId,
    state_version: DurableStateVersion,
    prior_hash: ContentDigest,
    input_digest: ContentDigest,
    output_digest: ContentDigest,
    codec_id: crate::CodecId,
    codec_version: crate::CodecVersion,
    bytes: CanonicalBytes,
}

#[derive(Debug, Clone)]
struct StoredSnapshot {
    version: DurableStateVersion,
    codec_id: crate::CodecId,
    codec_version: crate::CodecVersion,
    state: CanonicalBytes,
    state_digest: ContentDigest,
    journal_position: Option<RecordId>,
    journal_root: ContentDigest,
}

struct StagedCommitExtensions {
    journal: Vec<StoredJournalRecord>,
    journal_sequence: u64,
    journal_position: Option<RecordId>,
    journal_root: ContentDigest,
    effects: Vec<(EffectId, StoredEffect)>,
}

#[derive(Debug, Clone, Copy)]
struct StoredRetentionGap {
    earliest_available: RecordId,
    required_snapshot: Option<ContentDigest>,
}

fn subject_tombstone_digest(operation: OperationId, state_digest: ContentDigest) -> ContentDigest {
    let mut canonical = Vec::with_capacity(64);
    canonical.extend_from_slice(operation.as_uuid().as_bytes());
    canonical.extend_from_slice(state_digest.as_bytes());
    ContentDigest::hash("azoth durable subject tombstone v1", &canonical)
}

#[derive(Debug, Clone)]
enum StoredOperationResult {
    Commit(StoredCommitReceipt),
    Writer(StoredWriterReceipt),
}

#[derive(Debug, Clone)]
struct StoredOperation {
    kind: StoredOperationKind,
    input_digest: ContentDigest,
    result: StoredOperationResult,
}

#[derive(Debug, Clone)]
enum StoredLifecycle {
    Absent,
    Active {
        version: DurableStateVersion,
        codec_id: crate::CodecId,
        codec_version: crate::CodecVersion,
        state: CanonicalBytes,
        state_digest: ContentDigest,
    },
    Retired {
        version: DurableStateVersion,
        operation: OperationId,
        tombstone_digest: ContentDigest,
    },
}

impl StoredLifecycle {
    const fn kind(&self) -> SubjectLifecycle {
        match self {
            Self::Absent => SubjectLifecycle::Absent,
            Self::Active { .. } => SubjectLifecycle::Active,
            Self::Retired { .. } => SubjectLifecycle::Retired,
        }
    }
}

#[derive(Debug, Clone)]
struct StoredLease {
    fence: WriterFence,
    generation: u64,
    until: crate::AbsoluteDeadline,
}

struct ValidatedActive {
    version: DurableStateVersion,
    codec_id: crate::CodecId,
    codec_version: crate::CodecVersion,
    state: CanonicalBytes,
}

enum ActiveValidationFailure<T: DurableSubject> {
    Refused(Box<CommitRefusal<T>>),
    Storage(StorageFailure),
}

#[derive(Debug)]
pub struct ReferenceState {
    lifecycle: StoredLifecycle,
    fence: WriterFence,
    lease_generation: u64,
    lease: Option<StoredLease>,
    operations: BTreeMap<OperationId, StoredOperation>,
    journal: Vec<StoredJournalRecord>,
    journal_sequence: u64,
    journal_position: Option<RecordId>,
    journal_root: ContentDigest,
    snapshots: BTreeMap<DurableStateVersion, StoredSnapshot>,
    retention_gap: Option<StoredRetentionGap>,
    effects: BTreeMap<EffectId, StoredEffect>,
    operation_capacity: OperationCapacity,
    now: u64,
    next_fault: Option<InMemorySubjectFault>,
}

/// Deterministic single-subject semantic reference adapter.
pub struct InMemorySubject<T: DurableSubject> {
    id: DurableSubjectId<T>,
    profile: ProvenProfileRef,
    state: Mutex<ReferenceState>,
    marker: PhantomData<fn() -> T>,
}

impl<T: DurableSubject> InMemorySubject<T> {
    fn stage_commit_extensions(
        state: &ReferenceState,
        journal: impl IntoIterator<Item = JournalClause>,
        outbox: impl IntoIterator<Item = OutboxClause>,
        operation: OperationId,
        state_version: DurableStateVersion,
        input_digest: ContentDigest,
        output_digest: ContentDigest,
    ) -> Result<StagedCommitExtensions, StorageFailure> {
        let (journal, journal_sequence, journal_position, journal_root) =
            Self::stage_journal(state, journal, state_version, input_digest, output_digest)?;
        let effects = Self::stage_outbox(operation, outbox)?;
        if effects
            .iter()
            .any(|(effect, _)| state.effects.contains_key(effect))
        {
            return Err(StorageFailure::RecoveryRequired);
        }
        Ok(StagedCommitExtensions {
            journal,
            journal_sequence,
            journal_position,
            journal_root,
            effects,
        })
    }

    fn publish_commit_extensions(
        state: &mut ReferenceState,
        staged: StagedCommitExtensions,
        version: DurableStateVersion,
        codec_id: crate::CodecId,
        codec_version: crate::CodecVersion,
        value: CanonicalBytes,
        state_digest: ContentDigest,
    ) {
        state.journal.extend(staged.journal);
        state.journal_sequence = staged.journal_sequence;
        state.journal_position = staged.journal_position;
        state.journal_root = staged.journal_root;
        state.snapshots.insert(
            version,
            StoredSnapshot {
                version,
                codec_id,
                codec_version,
                state: value,
                state_digest,
                journal_position: staged.journal_position,
                journal_root: staged.journal_root,
            },
        );
        state.effects.extend(staged.effects);
    }

    /// Opens an absent subject under one exact proved profile and deterministic time.
    #[must_use]
    pub fn new(
        id: DurableSubjectId<T>,
        profile: ProvenProfileRef,
        now: u64,
        operation_capacity: OperationCapacity,
    ) -> Self {
        Self {
            id,
            profile,
            state: Mutex::new(ReferenceState {
                lifecycle: StoredLifecycle::Absent,
                fence: WriterFence::ZERO,
                lease_generation: 0,
                lease: None,
                operations: BTreeMap::new(),
                journal: Vec::new(),
                journal_sequence: 0,
                journal_position: None,
                journal_root: journal_genesis(),
                snapshots: BTreeMap::new(),
                retention_gap: None,
                effects: BTreeMap::new(),
                operation_capacity,
                now,
                next_fault: None,
            }),
            marker: PhantomData,
        }
    }

    /// Sets deterministic reference time.
    pub fn set_now(&self, now: u64) {
        self.lock().now = now;
    }

    /// Fails the next commit before publishing its staged transaction.
    pub fn fail_next_commit_at(&self, fault: InMemorySubjectFault) {
        self.lock().next_fault = Some(fault);
    }

    /// Replaces the stored codec identity to exercise corruption recovery paths.
    pub fn set_stored_codec_for_conformance(&self, codec_id: crate::CodecId) {
        if let StoredLifecycle::Active {
            codec_id: stored, ..
        } = &mut self.lock().lifecycle
        {
            *stored = codec_id;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ReferenceState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn load_sync(
        &self,
        requested: DurableSubjectId<T>,
    ) -> Result<LoadedSubject<T>, StorageFailure> {
        if requested != self.id {
            return Ok(LoadedSubject::Absent { id: requested });
        }
        let state = self.lock();
        match &state.lifecycle {
            StoredLifecycle::Absent => Ok(LoadedSubject::Absent { id: self.id }),
            StoredLifecycle::Active {
                version,
                codec_id,
                codec_version,
                state,
                state_digest,
            } => {
                if *codec_id != T::State::CODEC_ID {
                    return Err(StorageFailure::RecoveryRequired);
                }
                if *state_digest
                    != ContentDigest::hash("azoth durable subject state v1", state.as_slice())
                {
                    return Err(StorageFailure::RecoveryRequired);
                }
                T::State::decode_exact(*codec_version, state.as_slice())
                    .map(|decoded| {
                        LoadedSubject::Active(ActiveSubject {
                            id: self.id,
                            version: *version,
                            codec_id: *codec_id,
                            codec_version: *codec_version,
                            state: decoded,
                        })
                    })
                    .map_err(|_| StorageFailure::RecoveryRequired)
            }
            StoredLifecycle::Retired {
                version,
                operation,
                tombstone_digest,
            } => Ok(LoadedSubject::Retired(
                RetiredSubject::from_store_tombstone(
                    self.id,
                    *version,
                    *operation,
                    *tombstone_digest,
                ),
            )),
        }
    }

    fn create_sync(&self, request: CreateSubject<T>) -> CommitResult<T> {
        if request.id != self.id {
            return CommitResult::Refused(CommitRefusal::SubjectMismatch);
        }
        let encoded = request.initial_state;
        let state_digest =
            ContentDigest::hash("azoth durable subject state v1", encoded.as_slice());
        let snapshot_state = encoded.clone();
        let mut state = self.lock();

        if let Some(existing) = state.operations.get(&request.operation) {
            return self.replay_commit_or_conflict(
                existing,
                StoredOperationKind::Create,
                request.canonical_input_digest,
            );
        }
        if let Err(mismatch) = request.required_profile.check(self.profile) {
            return CommitResult::Refused(CommitRefusal::ProfileInsufficient(mismatch));
        }
        if state.operations.len() >= state.operation_capacity.get() {
            return CommitResult::Refused(CommitRefusal::OperationCapacityExceeded);
        }
        match state.lifecycle {
            StoredLifecycle::Absent => {}
            StoredLifecycle::Active { .. } => {
                return CommitResult::Refused(CommitRefusal::CreateConflict);
            }
            StoredLifecycle::Retired { .. } => {
                return CommitResult::Refused(CommitRefusal::Retired);
            }
        }

        let receipt = StoredCommitReceipt {
            operation: request.operation,
            input_digest: request.canonical_input_digest,
            version: DurableStateVersion::initial(),
            state_digest,
        };
        state.lifecycle = StoredLifecycle::Active {
            version: DurableStateVersion::initial(),
            codec_id: request.codec_id,
            codec_version: request.codec_version,
            state: encoded,
            state_digest,
        };
        state.snapshots.insert(
            DurableStateVersion::initial(),
            StoredSnapshot {
                version: DurableStateVersion::initial(),
                codec_id: request.codec_id,
                codec_version: request.codec_version,
                state: snapshot_state,
                state_digest,
                journal_position: None,
                journal_root: journal_genesis(),
            },
        );
        state.operations.insert(
            request.operation,
            StoredOperation {
                kind: StoredOperationKind::Create,
                input_digest: request.canonical_input_digest,
                result: StoredOperationResult::Commit(receipt.clone()),
            },
        );
        CommitResult::Accepted(self.materialize_commit_receipt(&receipt))
    }

    fn acquire_sync(
        &self,
        request: AcquireWriter<T>,
    ) -> Result<AcquireWriterResult<T>, StorageFailure> {
        if request.subject != self.id {
            return Ok(AcquireWriterResult::Refused(
                crate::WriterAcquisitionRefusal::SubjectMismatch,
            ));
        }
        let mut state = self.lock();
        if let Some(existing) = state.operations.get(&request.operation) {
            return Ok(self.replay_writer_or_conflict(existing, request.canonical_input_digest));
        }
        if let Err(mismatch) = request.required_profile.check(self.profile) {
            return Ok(AcquireWriterResult::Refused(
                crate::WriterAcquisitionRefusal::ProfileInsufficient(mismatch),
            ));
        }
        if state.operations.len() >= state.operation_capacity.get() {
            return Ok(AcquireWriterResult::Refused(
                crate::WriterAcquisitionRefusal::OperationCapacityExceeded,
            ));
        }
        let actual = state.lifecycle.kind();
        if actual != request.expected_lifecycle {
            return Ok(AcquireWriterResult::Refused(
                crate::WriterAcquisitionRefusal::LifecycleMismatch { actual },
            ));
        }
        match actual {
            SubjectLifecycle::Absent => {
                return Ok(AcquireWriterResult::Refused(
                    crate::WriterAcquisitionRefusal::NotFound,
                ));
            }
            SubjectLifecycle::Retired => {
                return Ok(AcquireWriterResult::Refused(
                    crate::WriterAcquisitionRefusal::Retired,
                ));
            }
            SubjectLifecycle::Active => {}
        }
        if let Some(lease) = &state.lease
            && !lease.until.elapsed_at(state.now)
        {
            return Ok(AcquireWriterResult::Refused(
                crate::WriterAcquisitionRefusal::HeldUntil {
                    deadline: lease.until,
                },
            ));
        }
        let fence = state
            .fence
            .checked_next()
            .map_err(|_| StorageFailure::RecoveryRequired)?;
        let lease_generation = state
            .lease_generation
            .checked_add(1)
            .ok_or(StorageFailure::RecoveryRequired)?;
        let stored = StoredWriterReceipt {
            operation: request.operation,
            input_digest: request.canonical_input_digest,
            fence,
            lease_generation,
            lease_until: request.lease_until,
        };
        state.fence = fence;
        state.lease_generation = lease_generation;
        state.lease = Some(StoredLease {
            fence,
            generation: lease_generation,
            until: request.lease_until,
        });
        state.operations.insert(
            request.operation,
            StoredOperation {
                kind: StoredOperationKind::AcquireWriter,
                input_digest: request.canonical_input_digest,
                result: StoredOperationResult::Writer(stored.clone()),
            },
        );
        let receipt = self.materialize_writer_receipt(&stored);
        Ok(AcquireWriterResult::Acquired {
            permit: receipt.issue_permit(),
            receipt,
        })
    }

    fn validate_fenced_active(
        &self,
        state: &ReferenceState,
        permit: &WriterPermit<T>,
        expected_version: DurableStateVersion,
    ) -> Result<ValidatedActive, ActiveValidationFailure<T>> {
        if permit.subject() != self.id {
            return Err(ActiveValidationFailure::Refused(Box::new(
                CommitRefusal::SubjectMismatch,
            )));
        }
        let active = match &state.lifecycle {
            StoredLifecycle::Absent => {
                return Err(ActiveValidationFailure::Refused(Box::new(
                    CommitRefusal::NotFound,
                )));
            }
            StoredLifecycle::Retired { .. } => {
                return Err(ActiveValidationFailure::Refused(Box::new(
                    CommitRefusal::Retired,
                )));
            }
            StoredLifecycle::Active {
                version,
                codec_id,
                codec_version,
                state,
                ..
            } => ValidatedActive {
                version: *version,
                codec_id: *codec_id,
                codec_version: *codec_version,
                state: state.clone(),
            },
        };
        if active.codec_id != T::State::CODEC_ID
            || T::State::decode_exact(active.codec_version, active.state.as_slice()).is_err()
        {
            return Err(ActiveValidationFailure::Storage(
                StorageFailure::RecoveryRequired,
            ));
        }
        let Some(lease) = &state.lease else {
            return Err(ActiveValidationFailure::Refused(Box::new(
                CommitRefusal::WriterUnavailable { retry_after: None },
            )));
        };
        if lease.fence != permit.fence() || lease.generation != permit.lease_generation() {
            return Err(ActiveValidationFailure::Refused(Box::new(
                CommitRefusal::WriterSuperseded {
                    current: state.fence,
                },
            )));
        }
        if expected_version != active.version {
            return Err(ActiveValidationFailure::Refused(Box::new(
                CommitRefusal::StateVersionConflict {
                    current: active.version,
                },
            )));
        }
        Ok(active)
    }

    fn commit_sync(&self, request: CommitSubject<T>) -> Result<CommitResult<T>, StorageFailure> {
        let replacement = request.mutation.replacement;
        let replacement_codec = (request.mutation.codec_id, request.mutation.codec_version);
        let journal = request.mutation.journal;
        let outbox = request.mutation.outbox;
        let mut state = self.lock();
        if let Some(existing) = state.operations.get(&request.operation) {
            return Ok(self.replay_commit_or_conflict(
                existing,
                StoredOperationKind::Commit,
                request.canonical_input_digest,
            ));
        }
        if let Err(mismatch) = request.required_profile.check(self.profile) {
            return Ok(CommitResult::Refused(CommitRefusal::ProfileInsufficient(
                mismatch,
            )));
        }
        if state.operations.len() >= state.operation_capacity.get() {
            return Ok(CommitResult::Refused(
                CommitRefusal::OperationCapacityExceeded,
            ));
        }
        let active =
            match self.validate_fenced_active(&state, &request.permit, request.expected_version) {
                Ok(active) => active,
                Err(ActiveValidationFailure::Refused(refusal)) => {
                    return Ok(CommitResult::Refused(*refusal));
                }
                Err(ActiveValidationFailure::Storage(failure)) => return Err(failure),
            };
        let next_version = active
            .version
            .checked_next()
            .map_err(|_| StorageFailure::RecoveryRequired)?;
        let (next_bytes, next_codec_id, next_codec_version) = replacement.map_or(
            (active.state, active.codec_id, active.codec_version),
            |bytes| (bytes, replacement_codec.0, replacement_codec.1),
        );
        let state_digest =
            ContentDigest::hash("azoth durable subject state v1", next_bytes.as_slice());
        let staged = Self::stage_commit_extensions(
            &state,
            journal,
            outbox,
            request.operation,
            next_version,
            request.canonical_input_digest,
            state_digest,
        )?;
        let receipt = StoredCommitReceipt {
            operation: request.operation,
            input_digest: request.canonical_input_digest,
            version: next_version,
            state_digest,
        };
        if state.next_fault.take().is_some() {
            return Err(StorageFailure::Unavailable);
        }
        state.lifecycle = StoredLifecycle::Active {
            version: next_version,
            codec_id: next_codec_id,
            codec_version: next_codec_version,
            state: next_bytes.clone(),
            state_digest,
        };
        Self::publish_commit_extensions(
            &mut state,
            staged,
            next_version,
            next_codec_id,
            next_codec_version,
            next_bytes,
            state_digest,
        );
        state.operations.insert(
            request.operation,
            StoredOperation {
                kind: StoredOperationKind::Commit,
                input_digest: request.canonical_input_digest,
                result: StoredOperationResult::Commit(receipt.clone()),
            },
        );
        Ok(CommitResult::Accepted(
            self.materialize_commit_receipt(&receipt),
        ))
    }

    fn retire_sync(&self, request: RetireSubject<T>) -> Result<CommitResult<T>, StorageFailure> {
        let replacement = request.final_mutation.replacement;
        let replacement_codec = (
            request.final_mutation.codec_id,
            request.final_mutation.codec_version,
        );
        let journal = request.final_mutation.journal;
        let outbox = request.final_mutation.outbox;
        let mut state = self.lock();
        if let Some(existing) = state.operations.get(&request.operation) {
            return Ok(self.replay_commit_or_conflict(
                existing,
                StoredOperationKind::Retire,
                request.canonical_input_digest,
            ));
        }
        if let Err(mismatch) = request.required_profile.check(self.profile) {
            return Ok(CommitResult::Refused(CommitRefusal::ProfileInsufficient(
                mismatch,
            )));
        }
        if state.operations.len() >= state.operation_capacity.get() {
            return Ok(CommitResult::Refused(
                CommitRefusal::OperationCapacityExceeded,
            ));
        }
        let active =
            match self.validate_fenced_active(&state, &request.permit, request.expected_version) {
                Ok(active) => active,
                Err(ActiveValidationFailure::Refused(refusal)) => {
                    return Ok(CommitResult::Refused(*refusal));
                }
                Err(ActiveValidationFailure::Storage(failure)) => return Err(failure),
            };
        let next_version = active
            .version
            .checked_next()
            .map_err(|_| StorageFailure::RecoveryRequired)?;
        let next_fence = state
            .fence
            .checked_next()
            .map_err(|_| StorageFailure::RecoveryRequired)?;
        let (next_bytes, next_codec_id, next_codec_version) = replacement.map_or(
            (active.state, active.codec_id, active.codec_version),
            |bytes| (bytes, replacement_codec.0, replacement_codec.1),
        );
        let state_digest =
            ContentDigest::hash("azoth durable subject state v1", next_bytes.as_slice());
        let staged = Self::stage_commit_extensions(
            &state,
            journal,
            outbox,
            request.operation,
            next_version,
            request.canonical_input_digest,
            state_digest,
        )?;
        let tombstone_digest = subject_tombstone_digest(request.operation, state_digest);
        let receipt = StoredCommitReceipt {
            operation: request.operation,
            input_digest: request.canonical_input_digest,
            version: next_version,
            state_digest,
        };
        if state.next_fault.take().is_some() {
            return Err(StorageFailure::Unavailable);
        }
        state.lifecycle = StoredLifecycle::Retired {
            version: next_version,
            operation: request.operation,
            tombstone_digest,
        };
        state.fence = next_fence;
        state.lease = None;
        Self::publish_commit_extensions(
            &mut state,
            staged,
            next_version,
            next_codec_id,
            next_codec_version,
            next_bytes,
            state_digest,
        );
        state.operations.insert(
            request.operation,
            StoredOperation {
                kind: StoredOperationKind::Retire,
                input_digest: request.canonical_input_digest,
                result: StoredOperationResult::Commit(receipt.clone()),
            },
        );
        Ok(CommitResult::Accepted(
            self.materialize_commit_receipt(&receipt),
        ))
    }

    fn lookup_sync(
        &self,
        key: SubjectOperationKey<T>,
        input_digest: ContentDigest,
    ) -> OperationLookup<T> {
        if key.subject != self.id {
            return OperationLookup::Absent;
        }
        let state = self.lock();
        let Some(stored) = state.operations.get(&key.operation) else {
            return OperationLookup::Absent;
        };
        if stored.input_digest != input_digest {
            return OperationLookup::Conflict {
                original_digest: stored.input_digest,
            };
        }
        OperationLookup::Existing(match &stored.result {
            StoredOperationResult::Commit(receipt) => SubjectOperationResult::Commit(
                CommitResult::Accepted(self.materialize_commit_receipt(receipt)),
            ),
            StoredOperationResult::Writer(receipt) => {
                SubjectOperationResult::WriterGrant(self.materialize_writer_receipt(receipt))
            }
        })
    }

    fn replay_commit_or_conflict(
        &self,
        existing: &StoredOperation,
        expected_kind: StoredOperationKind,
        input_digest: ContentDigest,
    ) -> CommitResult<T> {
        if existing.kind != expected_kind || existing.input_digest != input_digest {
            return CommitResult::Refused(CommitRefusal::OperationConflict {
                original_digest: existing.input_digest,
            });
        }
        match &existing.result {
            StoredOperationResult::Commit(receipt) => {
                CommitResult::Accepted(self.materialize_commit_receipt(receipt))
            }
            StoredOperationResult::Writer(_) => {
                CommitResult::Refused(CommitRefusal::OperationConflict {
                    original_digest: existing.input_digest,
                })
            }
        }
    }

    fn replay_writer_or_conflict(
        &self,
        existing: &StoredOperation,
        input_digest: ContentDigest,
    ) -> AcquireWriterResult<T> {
        if existing.kind != StoredOperationKind::AcquireWriter
            || existing.input_digest != input_digest
        {
            return AcquireWriterResult::Refused(
                crate::WriterAcquisitionRefusal::OperationConflict {
                    original_digest: existing.input_digest,
                },
            );
        }
        match &existing.result {
            StoredOperationResult::Writer(stored) => {
                let receipt = self.materialize_writer_receipt(stored);
                AcquireWriterResult::Acquired {
                    permit: receipt.issue_permit(),
                    receipt,
                }
            }
            StoredOperationResult::Commit(_) => {
                AcquireWriterResult::Refused(crate::WriterAcquisitionRefusal::OperationConflict {
                    original_digest: existing.input_digest,
                })
            }
        }
    }

    const fn materialize_commit_receipt(&self, stored: &StoredCommitReceipt) -> CommitReceipt<T> {
        CommitReceipt::from_store_commit(
            self.id,
            stored.operation,
            stored.input_digest,
            stored.version,
            stored.state_digest,
            self.profile,
        )
    }

    const fn materialize_writer_receipt(
        &self,
        stored: &StoredWriterReceipt,
    ) -> WriterGrantReceipt<T> {
        WriterGrantReceipt::from_store_grant(
            self.id,
            stored.operation,
            stored.input_digest,
            stored.fence,
            stored.lease_generation,
            stored.lease_until,
            self.profile,
        )
    }
}

impl<T: DurableSubject> SubjectStatePort<T> for InMemorySubject<T> {
    async fn load(&self, subject: DurableSubjectId<T>) -> Result<LoadedSubject<T>, StorageFailure> {
        self.load_sync(subject)
    }

    async fn create(&self, request: CreateSubject<T>) -> Result<CommitResult<T>, StorageFailure> {
        Ok(self.create_sync(request))
    }

    async fn acquire_writer(
        &self,
        request: AcquireWriter<T>,
    ) -> Result<AcquireWriterResult<T>, StorageFailure> {
        self.acquire_sync(request)
    }

    async fn lookup(
        &self,
        key: SubjectOperationKey<T>,
        canonical_input_digest: ContentDigest,
    ) -> Result<OperationLookup<T>, StorageFailure> {
        Ok(self.lookup_sync(key, canonical_input_digest))
    }

    async fn commit(&self, request: CommitSubject<T>) -> Result<CommitResult<T>, StorageFailure> {
        self.commit_sync(request)
    }

    async fn retire(&self, request: RetireSubject<T>) -> Result<CommitResult<T>, StorageFailure> {
        self.retire_sync(request)
    }
}
