use std::num::{NonZeroU64, NonZeroUsize};

use serde::{Deserialize, Serialize};

use crate::{
    AbsoluteDeadline, CanonicalBytes, CodecError, CodecId, CodecVersion, ContentDigest,
    DurableCodec, DurableProfileRequirement, DurableStateVersionError, DurableSubject,
    DurableSubjectId, OperationId, ProfileMismatch, ProvenProfileRef, WriterFenceError,
};

mod mutation;

pub use mutation::{MAX_MUTATION_CLAUSES, MutationBuildError, SubjectMutation};

/// Current durable state revision. Version zero is the initial state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DurableStateVersion(u64);

impl DurableStateVersion {
    /// Initial subject-state version.
    #[must_use]
    pub const fn initial() -> Self {
        Self(0)
    }

    /// Returns the numeric version.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Reconstructs a validated version read by a storage adapter.
    #[must_use]
    pub const fn from_store_value(value: u64) -> Self {
        Self(value)
    }

    pub(crate) const fn checked_next(self) -> Result<Self, DurableStateVersionError> {
        match self.0.checked_add(1) {
            Some(next) => Ok(Self(next)),
            None => Err(DurableStateVersionError),
        }
    }
}

/// Monotonic store-issued writer fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WriterFence(u64);

impl WriterFence {
    pub(crate) const ZERO: Self = Self(0);

    /// Returns the numeric fence.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Reconstructs a nonzero fence read or issued by a storage adapter.
    #[must_use]
    pub const fn from_store_value(value: NonZeroU64) -> Self {
        Self(value.get())
    }

    pub(crate) const fn checked_next(self) -> Result<Self, WriterFenceError> {
        match self.0.checked_add(1) {
            Some(next) => Ok(Self(next)),
            None => Err(WriterFenceError),
        }
    }
}

/// Durable lifecycle of one subject identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubjectLifecycle {
    /// No subject has ever been created.
    Absent,
    /// Subject state is active and mutable by the current writer.
    Active,
    /// Identity is terminal and retained as a tombstone.
    Retired,
}

/// Kind of runtime holding a writer lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WriterHolderKind {
    /// An operating-system process.
    Process,
    /// A long-lived service.
    Service,
    /// A provider-owned serial actor.
    ProviderActor,
}

/// Diagnostic attribution for a writer lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriterHolder {
    kind: WriterHolderKind,
    identity: crate::BoundedDiagnosticId,
}

impl WriterHolder {
    /// Constructs diagnostic holder attribution.
    #[must_use]
    pub const fn new(kind: WriterHolderKind, identity: crate::BoundedDiagnosticId) -> Self {
        Self { kind, identity }
    }

    /// Returns the holder category used for diagnostics.
    #[must_use]
    pub const fn kind(&self) -> WriterHolderKind {
        self.kind
    }

    /// Returns the bounded diagnostic holder identity.
    #[must_use]
    pub const fn identity(&self) -> &crate::BoundedDiagnosticId {
        &self.identity
    }
}

/// Non-cloneable local proof that a store issued one writer fence.
#[derive(Debug)]
pub struct WriterPermit<T: DurableSubject> {
    subject: DurableSubjectId<T>,
    fence: WriterFence,
    lease_generation: u64,
}

impl<T: DurableSubject> WriterPermit<T> {
    /// Constructs the linear permit corresponding to one persisted store grant.
    #[must_use]
    pub const fn from_store_grant(
        subject: DurableSubjectId<T>,
        fence: WriterFence,
        lease_generation: u64,
    ) -> Self {
        Self {
            subject,
            fence,
            lease_generation,
        }
    }

    /// Returns the subject this permit can address.
    #[must_use]
    pub const fn subject(&self) -> DurableSubjectId<T> {
        self.subject
    }

    /// Returns the store-issued fence.
    #[must_use]
    pub const fn fence(&self) -> WriterFence {
        self.fence
    }

    /// Returns the store-issued lease generation.
    #[must_use]
    pub const fn lease_generation(&self) -> u64 {
        self.lease_generation
    }
}

/// Loaded subject with absent, active, and retired states made disjoint.
pub enum LoadedSubject<T: DurableSubject> {
    /// Reads never create an absent subject.
    Absent {
        /// Requested subject identity.
        id: DurableSubjectId<T>,
    },
    /// Active decoded state.
    Active(ActiveSubject<T>),
    /// Terminal tombstone.
    Retired(RetiredSubject<T>),
}

/// Active decoded subject state.
pub struct ActiveSubject<T: DurableSubject> {
    /// Stable subject identity.
    pub id: DurableSubjectId<T>,
    /// Current state revision.
    pub version: DurableStateVersion,
    /// Codec version stored with the state.
    pub codec_version: CodecVersion,
    /// Codec identity stored with the state.
    pub codec_id: CodecId,
    /// Exact decoded domain state.
    pub state: T::State,
}

/// Terminal subject tombstone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredSubject<T: DurableSubject> {
    id: DurableSubjectId<T>,
    version: DurableStateVersion,
    retirement_operation: OperationId,
    tombstone_digest: ContentDigest,
}

impl<T: DurableSubject> RetiredSubject<T> {
    /// Constructs a decoded tombstone returned by a storage adapter.
    #[must_use]
    pub const fn from_store_tombstone(
        id: DurableSubjectId<T>,
        version: DurableStateVersion,
        retirement_operation: OperationId,
        tombstone_digest: ContentDigest,
    ) -> Self {
        Self {
            id,
            version,
            retirement_operation,
            tombstone_digest,
        }
    }

    /// Returns the retired subject identity.
    #[must_use]
    pub const fn id(&self) -> DurableSubjectId<T> {
        self.id
    }

    /// Returns the terminal state revision.
    #[must_use]
    pub const fn version(&self) -> DurableStateVersion {
        self.version
    }

    /// Returns the operation that retired the identity.
    #[must_use]
    pub const fn retirement_operation(&self) -> OperationId {
        self.retirement_operation
    }

    /// Returns the immutable tombstone digest.
    #[must_use]
    pub const fn tombstone_digest(&self) -> ContentDigest {
        self.tombstone_digest
    }
}

/// Operation-deduped create request.
#[derive(Debug)]
pub struct CreateSubject<T: DurableSubject> {
    pub(crate) id: DurableSubjectId<T>,
    pub(crate) operation: OperationId,
    pub(crate) canonical_input: CanonicalBytes,
    pub(crate) canonical_input_digest: ContentDigest,
    pub(crate) initial_state: CanonicalBytes,
    pub(crate) codec_id: CodecId,
    pub(crate) codec_version: CodecVersion,
    pub(crate) required_profile: DurableProfileRequirement,
}

impl<T: DurableSubject> CreateSubject<T> {
    /// Constructs a fully bound create request.
    ///
    /// # Errors
    ///
    /// Returns a codec error if `initial_state` cannot be sealed canonically.
    pub fn new(
        id: DurableSubjectId<T>,
        operation: OperationId,
        canonical_input: CanonicalBytes,
        initial_state: &T::State,
        required_profile: DurableProfileRequirement,
    ) -> Result<Self, CodecError> {
        let initial_state = initial_state.encode_canonical()?;
        let canonical_input_digest = canonical_operation_digest(&canonical_input);
        Ok(Self {
            id,
            operation,
            canonical_input,
            canonical_input_digest,
            initial_state,
            codec_id: T::State::CODEC_ID,
            codec_version: T::State::CURRENT_VERSION,
            required_profile,
        })
    }

    /// Returns the subject identity.
    #[must_use]
    pub const fn id(&self) -> DurableSubjectId<T> {
        self.id
    }

    /// Returns the causal operation identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    /// Returns the sealed canonical operation input.
    #[must_use]
    pub const fn canonical_input(&self) -> &CanonicalBytes {
        &self.canonical_input
    }

    /// Returns the digest derived from the sealed canonical operation input.
    #[must_use]
    pub const fn canonical_input_digest(&self) -> ContentDigest {
        self.canonical_input_digest
    }

    /// Returns the sealed initial state bytes.
    #[must_use]
    pub const fn initial_state(&self) -> &CanonicalBytes {
        &self.initial_state
    }

    /// Returns the initial state codec identity and version.
    #[must_use]
    pub const fn codec(&self) -> (CodecId, CodecVersion) {
        (self.codec_id, self.codec_version)
    }

    /// Returns the exact required adapter profile and evidence.
    #[must_use]
    pub const fn required_profile(&self) -> DurableProfileRequirement {
        self.required_profile
    }
}

impl<T: DurableSubject> Clone for CreateSubject<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            operation: self.operation,
            canonical_input: self.canonical_input.clone(),
            canonical_input_digest: self.canonical_input_digest,
            initial_state: self.initial_state.clone(),
            codec_id: self.codec_id,
            codec_version: self.codec_version,
            required_profile: self.required_profile,
        }
    }
}

/// Operation-deduped writer acquisition request.
pub struct AcquireWriter<T: DurableSubject> {
    pub(crate) subject: DurableSubjectId<T>,
    pub(crate) holder: WriterHolder,
    pub(crate) expected_lifecycle: SubjectLifecycle,
    pub(crate) lease_until: AbsoluteDeadline,
    pub(crate) operation: OperationId,
    pub(crate) canonical_input: CanonicalBytes,
    pub(crate) canonical_input_digest: ContentDigest,
    pub(crate) required_profile: DurableProfileRequirement,
}

impl<T: DurableSubject> Clone for AcquireWriter<T> {
    fn clone(&self) -> Self {
        Self {
            subject: self.subject,
            holder: self.holder.clone(),
            expected_lifecycle: self.expected_lifecycle,
            lease_until: self.lease_until,
            operation: self.operation,
            canonical_input: self.canonical_input.clone(),
            canonical_input_digest: self.canonical_input_digest,
            required_profile: self.required_profile,
        }
    }
}

impl<T: DurableSubject> AcquireWriter<T> {
    /// Returns the subject whose writer authority is requested.
    #[must_use]
    pub const fn subject(&self) -> DurableSubjectId<T> {
        self.subject
    }

    /// Returns the identity to which the writer permit will be attributed.
    #[must_use]
    pub const fn holder(&self) -> &WriterHolder {
        &self.holder
    }

    /// Returns the expected subject lifecycle.
    #[must_use]
    pub const fn expected_lifecycle(&self) -> SubjectLifecycle {
        self.expected_lifecycle
    }

    /// Returns the diagnostic lease deadline.
    #[must_use]
    pub const fn lease_until(&self) -> AbsoluteDeadline {
        self.lease_until
    }

    /// Returns the causal operation identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    /// Returns the sealed canonical operation input.
    #[must_use]
    pub const fn canonical_input(&self) -> &CanonicalBytes {
        &self.canonical_input
    }

    /// Returns the digest derived from the sealed canonical operation input.
    #[must_use]
    pub const fn canonical_input_digest(&self) -> ContentDigest {
        self.canonical_input_digest
    }

    /// Returns the exact required adapter profile and evidence.
    #[must_use]
    pub const fn required_profile(&self) -> DurableProfileRequirement {
        self.required_profile
    }
}

impl<T: DurableSubject> AcquireWriter<T> {
    /// Constructs a fully bound acquisition request.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        subject: DurableSubjectId<T>,
        holder: WriterHolder,
        expected_lifecycle: SubjectLifecycle,
        lease_until: AbsoluteDeadline,
        operation: OperationId,
        canonical_input: CanonicalBytes,
        required_profile: DurableProfileRequirement,
    ) -> Self {
        let canonical_input_digest = canonical_operation_digest(&canonical_input);
        Self {
            subject,
            holder,
            expected_lifecycle,
            lease_until,
            operation,
            canonical_input,
            canonical_input_digest,
            required_profile,
        }
    }
}

/// Fully bound subject commit request.
pub struct CommitSubject<T: DurableSubject> {
    pub(crate) permit: WriterPermit<T>,
    pub(crate) expected_version: DurableStateVersion,
    pub(crate) operation: OperationId,
    pub(crate) canonical_input: CanonicalBytes,
    pub(crate) canonical_input_digest: ContentDigest,
    pub(crate) required_profile: DurableProfileRequirement,
    pub(crate) mutation: SubjectMutation<T>,
}

impl<T: DurableSubject> CommitSubject<T> {
    /// Constructs a fenced commit request.
    #[must_use]
    pub fn new(
        permit: WriterPermit<T>,
        expected_version: DurableStateVersion,
        operation: OperationId,
        canonical_input: CanonicalBytes,
        required_profile: DurableProfileRequirement,
        mutation: SubjectMutation<T>,
    ) -> Self {
        let canonical_input_digest = canonical_operation_digest(&canonical_input);
        Self {
            permit,
            expected_version,
            operation,
            canonical_input,
            canonical_input_digest,
            required_profile,
            mutation,
        }
    }

    /// Returns the linear writer permit.
    #[must_use]
    pub const fn permit(&self) -> &WriterPermit<T> {
        &self.permit
    }

    /// Returns the expected durable state version.
    #[must_use]
    pub const fn expected_version(&self) -> DurableStateVersion {
        self.expected_version
    }

    /// Returns the causal operation identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    /// Returns the sealed canonical operation input.
    #[must_use]
    pub const fn canonical_input(&self) -> &CanonicalBytes {
        &self.canonical_input
    }

    /// Returns the digest derived from the sealed canonical operation input.
    #[must_use]
    pub const fn canonical_input_digest(&self) -> ContentDigest {
        self.canonical_input_digest
    }

    /// Returns the exact required adapter profile and evidence.
    #[must_use]
    pub const fn required_profile(&self) -> DurableProfileRequirement {
        self.required_profile
    }

    /// Returns the sealed mutation.
    #[must_use]
    pub const fn mutation(&self) -> &SubjectMutation<T> {
        &self.mutation
    }
}

/// Fully bound terminal subject request.
pub struct RetireSubject<T: DurableSubject> {
    pub(crate) permit: WriterPermit<T>,
    pub(crate) expected_version: DurableStateVersion,
    pub(crate) operation: OperationId,
    pub(crate) canonical_input: CanonicalBytes,
    pub(crate) canonical_input_digest: ContentDigest,
    pub(crate) required_profile: DurableProfileRequirement,
    pub(crate) final_mutation: SubjectMutation<T>,
}

impl<T: DurableSubject> RetireSubject<T> {
    /// Constructs a fenced terminal request.
    #[must_use]
    pub fn new(
        permit: WriterPermit<T>,
        expected_version: DurableStateVersion,
        operation: OperationId,
        canonical_input: CanonicalBytes,
        required_profile: DurableProfileRequirement,
        final_mutation: SubjectMutation<T>,
    ) -> Self {
        let canonical_input_digest = canonical_operation_digest(&canonical_input);
        Self {
            permit,
            expected_version,
            operation,
            canonical_input,
            canonical_input_digest,
            required_profile,
            final_mutation,
        }
    }

    /// Returns the linear writer permit.
    #[must_use]
    pub const fn permit(&self) -> &WriterPermit<T> {
        &self.permit
    }

    /// Returns the expected durable state version.
    #[must_use]
    pub const fn expected_version(&self) -> DurableStateVersion {
        self.expected_version
    }

    /// Returns the causal operation identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    /// Returns the sealed canonical operation input.
    #[must_use]
    pub const fn canonical_input(&self) -> &CanonicalBytes {
        &self.canonical_input
    }

    /// Returns the digest derived from the sealed canonical operation input.
    #[must_use]
    pub const fn canonical_input_digest(&self) -> ContentDigest {
        self.canonical_input_digest
    }

    /// Returns the exact required adapter profile and evidence.
    #[must_use]
    pub const fn required_profile(&self) -> DurableProfileRequirement {
        self.required_profile
    }

    /// Returns the sealed final mutation.
    #[must_use]
    pub const fn final_mutation(&self) -> &SubjectMutation<T> {
        &self.final_mutation
    }
}

/// Stable lookup key for a subject-scoped operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubjectOperationKey<T: DurableSubject> {
    pub(crate) subject: DurableSubjectId<T>,
    pub(crate) operation: OperationId,
}

impl<T: DurableSubject> SubjectOperationKey<T> {
    /// Constructs an exact subject operation key.
    #[must_use]
    pub const fn new(subject: DurableSubjectId<T>, operation: OperationId) -> Self {
        Self { subject, operation }
    }

    /// Returns the subject identity.
    #[must_use]
    pub const fn subject(&self) -> DurableSubjectId<T> {
        self.subject
    }

    /// Returns the operation identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }
}

/// Explicit bound on retained operation identities in a reference adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationCapacity(NonZeroUsize);

impl OperationCapacity {
    /// Constructs a nonzero retained-operation capacity.
    ///
    /// # Errors
    ///
    /// Returns [`OperationCapacityError`] when `value` is zero.
    pub const fn new(value: usize) -> Result<Self, OperationCapacityError> {
        match NonZeroUsize::new(value) {
            Some(value) => Ok(Self(value)),
            None => Err(OperationCapacityError),
        }
    }

    /// Returns the numeric capacity.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

/// Invalid zero retained-operation capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("operation capacity must be nonzero")]
pub struct OperationCapacityError;

/// Observable result of writer acquisition.
#[derive(Debug)]
pub enum AcquireWriterResult<T: DurableSubject> {
    /// A new or idempotently replayed grant issued a local permit.
    Acquired {
        /// Non-cloneable local permit.
        permit: WriterPermit<T>,
        /// Durable grant receipt.
        receipt: WriterGrantReceipt<T>,
    },
    /// Available storage refused the request.
    Refused(WriterAcquisitionRefusal),
}

/// Durable writer-grant receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterGrantReceipt<T: DurableSubject> {
    subject: DurableSubjectId<T>,
    operation: OperationId,
    input_digest: ContentDigest,
    fence: WriterFence,
    lease_generation: u64,
    lease_until: AbsoluteDeadline,
    profile: ProvenProfileRef,
}

impl<T: DurableSubject> WriterGrantReceipt<T> {
    /// Constructs a durable grant receipt from persisted adapter facts.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn from_store_grant(
        subject: DurableSubjectId<T>,
        operation: OperationId,
        input_digest: ContentDigest,
        fence: WriterFence,
        lease_generation: u64,
        lease_until: AbsoluteDeadline,
        profile: ProvenProfileRef,
    ) -> Self {
        Self {
            subject,
            operation,
            input_digest,
            fence,
            lease_generation,
            lease_until,
            profile,
        }
    }

    /// Returns the issued writer fence.
    #[must_use]
    pub const fn fence(&self) -> WriterFence {
        self.fence
    }

    /// Returns the subject identity.
    #[must_use]
    pub const fn subject(&self) -> DurableSubjectId<T> {
        self.subject
    }

    /// Returns the causal operation identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    /// Returns the bound canonical input digest.
    #[must_use]
    pub const fn input_digest(&self) -> ContentDigest {
        self.input_digest
    }

    /// Returns the store-issued lease generation.
    #[must_use]
    pub const fn lease_generation(&self) -> u64 {
        self.lease_generation
    }

    /// Returns the diagnostic lease deadline.
    #[must_use]
    pub const fn lease_until(&self) -> AbsoluteDeadline {
        self.lease_until
    }

    /// Returns the proved adapter profile and evidence.
    #[must_use]
    pub const fn profile(&self) -> ProvenProfileRef {
        self.profile
    }

    /// Issues the linear local permit represented by this durable grant.
    #[must_use]
    pub const fn issue_permit(&self) -> WriterPermit<T> {
        WriterPermit::from_store_grant(self.subject, self.fence, self.lease_generation)
    }
}

/// Closed writer acquisition refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterAcquisitionRefusal {
    /// Request addresses another subject than this adapter instance.
    SubjectMismatch,
    /// Subject has never been created.
    NotFound,
    /// Subject identity is terminal.
    Retired,
    /// Caller expected another lifecycle.
    LifecycleMismatch {
        /// Actual lifecycle.
        actual: SubjectLifecycle,
    },
    /// Another current holder owns the lease.
    HeldUntil {
        /// Earliest known retry time.
        deadline: AbsoluteDeadline,
    },
    /// Monotonic fence cannot advance.
    FenceExhausted,
    /// Operation ID was bound to different canonical input.
    OperationConflict {
        /// Original input digest.
        original_digest: ContentDigest,
    },
    /// Adapter profile does not satisfy the request.
    ProfileInsufficient(ProfileMismatch),
    /// Retained operation history reached its declared bound.
    OperationCapacityExceeded,
}

/// Observable subject commit result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitResult<T: DurableSubject> {
    /// The operation was accepted, including exact idempotent replay.
    Accepted(CommitReceipt<T>),
    /// Available storage refused a domain-invalid operation.
    Refused(CommitRefusal<T>),
}

/// Immutable receipt for a create, commit, or retirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReceipt<T: DurableSubject> {
    subject: DurableSubjectId<T>,
    operation: OperationId,
    input_digest: ContentDigest,
    version: DurableStateVersion,
    state_digest: ContentDigest,
    profile: ProvenProfileRef,
}

impl<T: DurableSubject> CommitReceipt<T> {
    /// Constructs an immutable receipt from persisted adapter facts.
    #[must_use]
    pub const fn from_store_commit(
        subject: DurableSubjectId<T>,
        operation: OperationId,
        input_digest: ContentDigest,
        version: DurableStateVersion,
        state_digest: ContentDigest,
        profile: ProvenProfileRef,
    ) -> Self {
        Self {
            subject,
            operation,
            input_digest,
            version,
            state_digest,
            profile,
        }
    }

    /// Returns the resulting durable state version.
    #[must_use]
    pub const fn version(&self) -> DurableStateVersion {
        self.version
    }

    /// Returns the subject identity.
    #[must_use]
    pub const fn subject(&self) -> DurableSubjectId<T> {
        self.subject
    }

    /// Returns the causal operation identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    /// Returns the canonical operation input digest.
    #[must_use]
    pub const fn input_digest(&self) -> ContentDigest {
        self.input_digest
    }

    /// Returns the resulting state digest.
    #[must_use]
    pub const fn state_digest(&self) -> ContentDigest {
        self.state_digest
    }

    /// Returns the proved adapter profile and evidence.
    #[must_use]
    pub const fn profile(&self) -> ProvenProfileRef {
        self.profile
    }
}

/// Closed create/commit/retire refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitRefusal<T: DurableSubject> {
    /// Request addresses another subject than this adapter instance.
    SubjectMismatch,
    /// Subject has never been created.
    NotFound,
    /// Exact prior creation is already durable.
    AlreadyExists(CommitReceipt<T>),
    /// Another operation already created the identity.
    CreateConflict,
    /// Subject identity is terminal.
    Retired,
    /// No current writer can accept this request.
    WriterUnavailable {
        /// Earliest known retry time.
        retry_after: Option<AbsoluteDeadline>,
    },
    /// Permit fence is no longer current.
    WriterSuperseded {
        /// Current store fence.
        current: WriterFence,
    },
    /// Subject changed after the caller read it.
    StateVersionConflict {
        /// Current state version.
        current: DurableStateVersion,
    },
    /// Operation ID was bound to different canonical input or operation kind.
    OperationConflict {
        /// Original input digest.
        original_digest: ContentDigest,
    },
    /// Monotonic fence cannot advance.
    FenceExhausted,
    /// Adapter profile does not satisfy the request.
    ProfileInsufficient(ProfileMismatch),
    /// Retained operation history reached its declared bound.
    OperationCapacityExceeded,
}

fn canonical_operation_digest(input: &CanonicalBytes) -> ContentDigest {
    ContentDigest::hash("azoth durable operation input v1", input.as_slice())
}

/// Result of reconciling an operation after an unknown response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationLookup<T: DurableSubject> {
    /// Original durable result.
    Existing(SubjectOperationResult<T>),
    /// No operation with this identity is durable.
    Absent,
    /// The identity is bound to different canonical bytes.
    Conflict {
        /// Original input digest.
        original_digest: ContentDigest,
    },
}

/// Durable subject operation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectOperationResult<T: DurableSubject> {
    /// Writer acquisition receipt.
    WriterGrant(WriterGrantReceipt<T>),
    /// Create/commit/retirement result.
    Commit(CommitResult<T>),
}
