use std::{marker::PhantomData, num::NonZeroU64};

use crate::{
    BoundedPage, BoundedPageSize, CanonicalBytes, CodecId, CodecVersion, ContentDigest,
    DurableCodec, DurableStateVersion, DurableSubject, DurableSubjectId, ErasedDurableSubjectId,
    MutationBuildError, RecordId, SubjectMutation,
};

/// Typed immutable record attached to one durable subject domain.
pub trait DurableRecord: DurableCodec {
    /// Subject whose history contains this record family.
    type Subject: DurableSubject;
}

#[derive(Debug, Clone)]
pub struct JournalClause {
    pub codec_id: CodecId,
    pub codec_version: CodecVersion,
    pub bytes: CanonicalBytes,
}

/// Seals one typed journal clause into a pending subject mutation.
///
/// A durable [`RecordId`] does not exist until the adapter atomically assigns
/// its sequence and prior hash during commit.
///
/// # Errors
///
/// Returns a bounded mutation or codec failure before any storage effect.
pub fn append_record<T, E>(
    mutation: &mut SubjectMutation<T>,
    record: &E,
) -> Result<(), MutationBuildError>
where
    T: DurableSubject,
    E: DurableRecord<Subject = T>,
{
    mutation.push_journal(JournalClause {
        codec_id: E::CODEC_ID,
        codec_version: E::CURRENT_VERSION,
        bytes: record.encode_canonical()?,
    })
}

/// One verified typed record from a subject journal.
#[derive(Debug, PartialEq, Eq)]
pub struct JournalRecord<E: DurableRecord> {
    id: RecordId,
    subject: DurableSubjectId<E::Subject>,
    state_version: DurableStateVersion,
    codec_id: CodecId,
    codec_version: CodecVersion,
    prior_hash: ContentDigest,
    input_digest: ContentDigest,
    output_digest: ContentDigest,
    canonical: CanonicalBytes,
    record: E,
}

impl<E> Clone for JournalRecord<E>
where
    E: DurableRecord + Clone,
{
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            subject: self.subject,
            state_version: self.state_version,
            codec_id: self.codec_id,
            codec_version: self.codec_version,
            prior_hash: self.prior_hash,
            input_digest: self.input_digest,
            output_digest: self.output_digest,
            canonical: self.canonical.clone(),
            record: self.record.clone(),
        }
    }
}

impl<E: DurableRecord> JournalRecord<E> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn from_store_record(
        id: RecordId,
        subject: DurableSubjectId<E::Subject>,
        state_version: DurableStateVersion,
        codec_id: CodecId,
        codec_version: CodecVersion,
        prior_hash: ContentDigest,
        input_digest: ContentDigest,
        output_digest: ContentDigest,
        canonical: CanonicalBytes,
        record: E,
    ) -> Self {
        Self {
            id,
            subject,
            state_version,
            codec_id,
            codec_version,
            prior_hash,
            input_digest,
            output_digest,
            canonical,
            record,
        }
    }

    /// Returns the durable chain position.
    #[must_use]
    pub const fn id(&self) -> RecordId {
        self.id
    }

    /// Returns the owning subject.
    #[must_use]
    pub const fn subject(&self) -> DurableSubjectId<E::Subject> {
        self.subject
    }

    /// Returns the subject version committed by this record group.
    #[must_use]
    pub const fn state_version(&self) -> DurableStateVersion {
        self.state_version
    }

    /// Returns the exact stored record codec identity and version.
    #[must_use]
    pub const fn codec(&self) -> (CodecId, CodecVersion) {
        (self.codec_id, self.codec_version)
    }

    /// Returns the preceding chain hash.
    #[must_use]
    pub const fn prior_hash(&self) -> ContentDigest {
        self.prior_hash
    }

    /// Returns the canonical command/input digest bound by the commit.
    #[must_use]
    pub const fn input_digest(&self) -> ContentDigest {
        self.input_digest
    }

    /// Returns the resulting subject-state digest bound by the commit.
    #[must_use]
    pub const fn output_digest(&self) -> ContentDigest {
        self.output_digest
    }

    /// Borrows the decoded domain record.
    #[must_use]
    pub const fn record(&self) -> &E {
        &self.record
    }
}

/// Bounded snapshot lookup.
pub struct SnapshotRequest<T: DurableSubject> {
    subject: DurableSubjectId<T>,
    at_or_before: Option<DurableStateVersion>,
}

impl<T: DurableSubject> SnapshotRequest<T> {
    /// Constructs a snapshot lookup.
    #[must_use]
    pub const fn new(
        subject: DurableSubjectId<T>,
        at_or_before: Option<DurableStateVersion>,
    ) -> Self {
        Self {
            subject,
            at_or_before,
        }
    }

    pub(crate) const fn subject(&self) -> DurableSubjectId<T> {
        self.subject
    }

    pub(crate) const fn at_or_before(&self) -> Option<DurableStateVersion> {
        self.at_or_before
    }
}

/// Bounded journal-page lookup.
pub struct RecordsAfter<E: DurableRecord> {
    subject: DurableSubjectId<E::Subject>,
    after: Option<RecordId>,
    limit: BoundedPageSize,
    marker: PhantomData<fn() -> E>,
}

impl<E: DurableRecord> RecordsAfter<E> {
    /// Constructs an exact cursor lookup.
    #[must_use]
    pub const fn new(
        subject: DurableSubjectId<E::Subject>,
        after: Option<RecordId>,
        limit: BoundedPageSize,
    ) -> Self {
        Self {
            subject,
            after,
            limit,
            marker: PhantomData,
        }
    }

    pub(crate) const fn subject(&self) -> DurableSubjectId<E::Subject> {
        self.subject
    }

    pub(crate) const fn after(&self) -> Option<RecordId> {
        self.after
    }

    pub(crate) const fn limit(&self) -> BoundedPageSize {
        self.limit
    }
}

/// Explicit indication that a requested journal prefix is no longer retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionGap {
    earliest_available: RecordId,
    required_snapshot: Option<ContentDigest>,
}

impl RetentionGap {
    pub(crate) const fn new(
        earliest_available: RecordId,
        required_snapshot: Option<ContentDigest>,
    ) -> Self {
        Self {
            earliest_available,
            required_snapshot,
        }
    }

    /// Returns the first retained record.
    #[must_use]
    pub const fn earliest_available(self) -> RecordId {
        self.earliest_available
    }

    /// Returns the snapshot digest required to bridge the missing prefix.
    #[must_use]
    pub const fn required_snapshot(self) -> Option<ContentDigest> {
        self.required_snapshot
    }
}

/// One bounded page plus the immutable current terminal root.
pub struct JournalPage<E: DurableRecord> {
    records: BoundedPage<JournalRecord<E>>,
    next_after: Option<RecordId>,
    terminal_root: ContentDigest,
    retention_gap: Option<RetentionGap>,
}

impl<E: DurableRecord> JournalPage<E> {
    pub(crate) const fn from_store_page(
        records: BoundedPage<JournalRecord<E>>,
        next_after: Option<RecordId>,
        terminal_root: ContentDigest,
        retention_gap: Option<RetentionGap>,
    ) -> Self {
        Self {
            records,
            next_after,
            terminal_root,
            retention_gap,
        }
    }

    /// Borrows the bounded records.
    #[must_use]
    pub const fn records(&self) -> &BoundedPage<JournalRecord<E>> {
        &self.records
    }

    /// Returns the exact cursor for the next page when more records exist.
    #[must_use]
    pub const fn next_after(&self) -> Option<RecordId> {
        self.next_after
    }

    /// Returns the current journal terminal root.
    #[must_use]
    pub const fn terminal_root(&self) -> ContentDigest {
        self.terminal_root
    }

    /// Returns an explicit missing-prefix fact.
    #[must_use]
    pub const fn retention_gap(&self) -> Option<RetentionGap> {
        self.retention_gap
    }
}

/// Typed bounded replay checkpoint.
pub struct DurableSnapshot<T: DurableSubject> {
    subject: DurableSubjectId<T>,
    version: DurableStateVersion,
    codec_id: CodecId,
    codec_version: CodecVersion,
    state: T::State,
    journal_position: Option<RecordId>,
    journal_root: ContentDigest,
}

impl<T: DurableSubject> DurableSnapshot<T> {
    pub(crate) const fn from_store_snapshot(
        subject: DurableSubjectId<T>,
        version: DurableStateVersion,
        codec_id: CodecId,
        codec_version: CodecVersion,
        state: T::State,
        journal_position: Option<RecordId>,
        journal_root: ContentDigest,
    ) -> Self {
        Self {
            subject,
            version,
            codec_id,
            codec_version,
            state,
            journal_position,
            journal_root,
        }
    }

    /// Returns the owning subject.
    #[must_use]
    pub const fn subject(&self) -> DurableSubjectId<T> {
        self.subject
    }

    /// Returns the snapshotted state version.
    #[must_use]
    pub const fn version(&self) -> DurableStateVersion {
        self.version
    }

    /// Returns the stored state codec version.
    #[must_use]
    pub const fn codec_version(&self) -> CodecVersion {
        self.codec_version
    }

    /// Returns the exact stored state codec identity and version.
    #[must_use]
    pub const fn codec(&self) -> (CodecId, CodecVersion) {
        (self.codec_id, self.codec_version)
    }

    /// Borrows the decoded state.
    #[must_use]
    pub const fn state(&self) -> &T::State {
        &self.state
    }

    /// Returns the last record included by the snapshot.
    #[must_use]
    pub const fn journal_position(&self) -> Option<RecordId> {
        self.journal_position
    }

    /// Returns the journal root included by the snapshot.
    #[must_use]
    pub const fn journal_root(&self) -> ContentDigest {
        self.journal_root
    }
}

/// Pure domain reducer used only while verifying a journal replay.
pub trait ReplayPolicy<T, E>
where
    T: DurableSubject,
    E: DurableRecord<Subject = T>,
{
    /// Domain rejection produced by the pure reducer.
    type Error;

    /// Applies one immutable record to state without effects.
    ///
    /// # Errors
    ///
    /// Returns a domain rejection when the record cannot follow `state`.
    fn apply(&self, state: &T::State, record: &E) -> Result<T::State, Self::Error>;
}

/// Verified terminal result of a bounded replay.
pub struct VerifiedReplay<T: DurableSubject> {
    subject: DurableSubjectId<T>,
    terminal_version: DurableStateVersion,
    terminal_state: T::State,
    terminal_root: ContentDigest,
}

impl<T: DurableSubject> VerifiedReplay<T> {
    /// Returns the verified subject.
    #[must_use]
    pub const fn subject(&self) -> DurableSubjectId<T> {
        self.subject
    }

    /// Returns the last verified state version.
    #[must_use]
    pub const fn terminal_version(&self) -> DurableStateVersion {
        self.terminal_version
    }

    /// Borrows the replayed terminal state.
    #[must_use]
    pub const fn terminal_state(&self) -> &T::State {
        &self.terminal_state
    }

    /// Returns the verified terminal journal root.
    #[must_use]
    pub const fn terminal_root(&self) -> ContentDigest {
        self.terminal_root
    }
}

/// Pure replay verification failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplayError<E> {
    /// A record belongs to another durable subject.
    #[error("journal record belongs to another subject")]
    SubjectMismatch,
    /// A record cursor is missing, repeated, or out of order.
    #[error("journal sequence is not contiguous")]
    SequenceGap,
    /// A record does not extend the preceding verified root.
    #[error("journal hash chain does not verify")]
    HashMismatch,
    /// Record codec identity or bytes do not match the requested family.
    #[error("journal record codec does not verify")]
    Codec,
    /// Replayed state does not match the output digest committed by the record group.
    #[error("journal replay output digest does not verify")]
    OutputDigestMismatch,
    /// Record version did not advance from the checkpoint.
    #[error("journal state version did not advance")]
    VersionRegression,
    /// The domain reducer rejected a record.
    #[error("journal replay policy rejected a record")]
    Policy(E),
}

/// Replays and verifies a contiguous record slice from one snapshot.
///
/// # Errors
///
/// Fails closed on cursor, chain, codec, version, reducer, or state-digest mismatch.
pub fn verify_replay<T, E, P>(
    snapshot: DurableSnapshot<T>,
    records: &[JournalRecord<E>],
    policy: &P,
) -> Result<VerifiedReplay<T>, ReplayError<P::Error>>
where
    T: DurableSubject,
    E: DurableRecord<Subject = T>,
    P: ReplayPolicy<T, E>,
{
    let mut state = snapshot.state;
    let mut version = snapshot.version;
    let mut prior = snapshot.journal_root;
    let mut sequence = snapshot
        .journal_position
        .map_or(0, |position| position.sequence().get());
    let subject = snapshot.subject;
    let mut group_output = None;

    for record in records {
        if record.subject != subject {
            return Err(ReplayError::SubjectMismatch);
        }
        let expected_sequence = sequence.checked_add(1).ok_or(ReplayError::SequenceGap)?;
        if record.id.sequence().get() != expected_sequence || record.prior_hash != prior {
            return Err(ReplayError::SequenceGap);
        }
        if record.codec_id != E::CODEC_ID {
            return Err(ReplayError::Codec);
        }
        let decoded = E::decode_exact(record.codec_version, record.canonical.as_slice())
            .map_err(|_| ReplayError::Codec)?;
        if record.id.hash()
            != record_hash(
                record.id.sequence(),
                record.prior_hash,
                record.state_version,
                record.input_digest,
                record.output_digest,
                record.codec_id,
                record.codec_version,
                &record.canonical,
            )
        {
            return Err(ReplayError::HashMismatch);
        }
        if record.state_version <= version {
            return Err(ReplayError::VersionRegression);
        }
        if let Some((group_version, expected_output)) = group_output {
            if group_version != record.state_version {
                verify_state_digest::<T, P::Error>(&state, expected_output)?;
                version = group_version;
            } else if expected_output != record.output_digest {
                return Err(ReplayError::OutputDigestMismatch);
            }
        }
        state = policy
            .apply(&state, &decoded)
            .map_err(ReplayError::Policy)?;
        group_output = Some((record.state_version, record.output_digest));
        sequence = expected_sequence;
        prior = record.id.hash();
    }
    if let Some((group_version, expected_output)) = group_output {
        verify_state_digest::<T, P::Error>(&state, expected_output)?;
        version = group_version;
    }
    Ok(VerifiedReplay {
        subject,
        terminal_version: version,
        terminal_state: state,
        terminal_root: prior,
    })
}

fn verify_state_digest<T: DurableSubject, E>(
    state: &T::State,
    expected: ContentDigest,
) -> Result<(), ReplayError<E>> {
    let bytes = state.encode_canonical().map_err(|_| ReplayError::Codec)?;
    let actual = ContentDigest::hash("azoth durable subject state v1", bytes.as_slice());
    if actual == expected {
        Ok(())
    } else {
        Err(ReplayError::OutputDigestMismatch)
    }
}

pub const fn journal_genesis() -> ContentDigest {
    ContentDigest::from_bytes([0; 32])
}

#[allow(clippy::too_many_arguments)]
pub fn record_hash(
    sequence: NonZeroU64,
    prior_hash: ContentDigest,
    state_version: DurableStateVersion,
    input_digest: ContentDigest,
    output_digest: ContentDigest,
    codec_id: CodecId,
    codec_version: CodecVersion,
    bytes: &CanonicalBytes,
) -> ContentDigest {
    let mut canonical = Vec::with_capacity(8 + 32 + 8 + 32 + 32 + 16 + 4 + bytes.len());
    canonical.extend_from_slice(&sequence.get().to_le_bytes());
    canonical.extend_from_slice(prior_hash.as_bytes());
    canonical.extend_from_slice(&state_version.get().to_le_bytes());
    canonical.extend_from_slice(input_digest.as_bytes());
    canonical.extend_from_slice(output_digest.as_bytes());
    canonical.extend_from_slice(codec_id.as_uuid().as_bytes());
    canonical.extend_from_slice(&codec_version.get().to_le_bytes());
    canonical.extend_from_slice(bytes.as_slice());
    ContentDigest::hash("azoth durable journal record v1", &canonical)
}

pub fn snapshot_digest(
    subject: ErasedDurableSubjectId,
    version: DurableStateVersion,
    root: ContentDigest,
    state: &CanonicalBytes,
) -> ContentDigest {
    let mut canonical = Vec::with_capacity(32 + 16 + 8 + 32 + state.len());
    canonical.extend_from_slice(subject.namespace.as_uuid().as_bytes());
    canonical.extend_from_slice(subject.key.as_bytes());
    canonical.extend_from_slice(&version.get().to_le_bytes());
    canonical.extend_from_slice(root.as_bytes());
    canonical.extend_from_slice(state.as_slice());
    ContentDigest::hash("azoth durable snapshot v1", &canonical)
}
