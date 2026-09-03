#![allow(
    clippy::redundant_pub_crate,
    reason = "reference adapters construct sealed outbox values across this private module boundary"
)]

use std::{marker::PhantomData, num::NonZeroU16};

use serde::{Deserialize, Serialize};

use crate::{
    AbsoluteDeadline, BoundError, BoundedPage, BoundedPageSize, CanonicalBytes, CodecId,
    CodecVersion, ContentDigest, DurableCodec, DurableNamespaceId, DurableSubject,
    DurableSubjectId, EffectId, ErasedDurableSubjectId, FailureDisposition, MutationBuildError,
    OperationId, RetryPolicyError, SubjectMutation, WriterFence, WriterHolder,
};

/// Global cap including the first delivery attempt.
pub const MAX_RETRY_ATTEMPTS: u16 = 32;

/// Stored retry envelope for one owed effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetryPolicy {
    /// One delivery attempt; any failure becomes poison.
    Never,
    /// Retry until the bounded attempt count or overall deadline is exhausted.
    Bounded {
        /// Maximum attempts including the first delivery.
        maximum_attempts: NonZeroU16,
        /// Latest accepted persisted retry deadline.
        overall_deadline: AbsoluteDeadline,
    },
}

impl RetryPolicy {
    /// Constructs a retry policy with a hard maximum of 32 attempts.
    ///
    /// # Errors
    ///
    /// Returns [`RetryPolicyError`] when `maximum_attempts` exceeds the global cap.
    pub const fn bounded(
        maximum_attempts: NonZeroU16,
        overall_deadline: AbsoluteDeadline,
    ) -> Result<Self, RetryPolicyError> {
        if maximum_attempts.get() > MAX_RETRY_ATTEMPTS {
            return Err(RetryPolicyError::AttemptsExceedHardCap {
                actual: maximum_attempts.get(),
                maximum: MAX_RETRY_ATTEMPTS,
            });
        }
        Ok(Self::Bounded {
            maximum_attempts,
            overall_deadline,
        })
    }

    /// Returns the maximum attempts including the first delivery.
    #[must_use]
    pub const fn maximum_attempts(self) -> NonZeroU16 {
        match self {
            Self::Never => NonZeroU16::MIN,
            Self::Bounded {
                maximum_attempts, ..
            } => maximum_attempts,
        }
    }

    pub(crate) fn permits_retry(
        self,
        completed_attempt: NonZeroU16,
        next_attempt_at: AbsoluteDeadline,
    ) -> bool {
        match self {
            Self::Never => false,
            Self::Bounded {
                maximum_attempts,
                overall_deadline,
            } => {
                completed_attempt.get() < maximum_attempts.get()
                    && next_attempt_at <= overall_deadline
            }
        }
    }
}

/// Typed payload for one at-least-once owed effect.
pub trait DurableMessage: DurableCodec {
    /// Subject whose commit caused the effect.
    type Subject: DurableSubject;
}

#[derive(Debug, Clone)]
pub struct OutboxClause {
    pub codec_id: CodecId,
    pub codec_version: CodecVersion,
    pub bytes: CanonicalBytes,
    pub policy: RetryPolicy,
}

/// Seals one owed effect into a pending subject mutation.
///
/// The durable [`EffectId`] is assigned from the accepted commit operation and
/// clause ordinal. It does not exist while the mutation is being planned.
///
/// # Errors
///
/// Returns a bounded mutation or codec failure before any storage effect.
pub fn enqueue_effect<T, M>(
    mutation: &mut SubjectMutation<T>,
    message: &M,
    policy: RetryPolicy,
) -> Result<(), MutationBuildError>
where
    T: DurableSubject,
    M: DurableMessage<Subject = T>,
{
    mutation.push_outbox(OutboxClause {
        codec_id: M::CODEC_ID,
        codec_version: M::CURRENT_VERSION,
        bytes: message.encode_canonical()?,
        policy,
    })
}

/// Scope from which one delivery worker claims effects.
pub enum EffectScope<M: DurableMessage> {
    /// One exact subject.
    Subject(DurableSubjectId<M::Subject>),
    /// Every subject in one domain namespace.
    Namespace(DurableNamespaceId),
}

impl<M: DurableMessage> EffectScope<M> {
    /// Selects one exact subject.
    #[must_use]
    pub const fn subject(subject: DurableSubjectId<M::Subject>) -> Self {
        Self::Subject(subject)
    }

    pub(crate) fn matches(&self, subject: DurableSubjectId<M::Subject>) -> bool {
        match self {
            Self::Subject(expected) => {
                expected.namespace() == subject.namespace() && *expected == subject
            }
            Self::Namespace(namespace) => *namespace == subject.namespace(),
        }
    }
}

/// Bounded due-effect claim request.
pub struct ClaimEffects<M: DurableMessage> {
    scope: EffectScope<M>,
    now: AbsoluteDeadline,
    lease_until: AbsoluteDeadline,
    limit: BoundedPageSize,
    worker: WriterHolder,
}

impl<M: DurableMessage> ClaimEffects<M> {
    /// Constructs a bounded claim request.
    #[must_use]
    pub const fn new(
        scope: EffectScope<M>,
        now: AbsoluteDeadline,
        lease_until: AbsoluteDeadline,
        limit: BoundedPageSize,
        worker: WriterHolder,
    ) -> Self {
        Self {
            scope,
            now,
            lease_until,
            limit,
            worker,
        }
    }

    pub(crate) const fn scope(&self) -> &EffectScope<M> {
        &self.scope
    }

    pub(crate) const fn now(&self) -> AbsoluteDeadline {
        self.now
    }

    pub(crate) const fn lease_until(&self) -> AbsoluteDeadline {
        self.lease_until
    }

    pub(crate) const fn limit(&self) -> BoundedPageSize {
        self.limit
    }

    pub(crate) const fn worker(&self) -> &WriterHolder {
        &self.worker
    }
}

/// One typed effect claimed under a monotonic attempt fence.
#[derive(Debug, PartialEq, Eq)]
pub struct ClaimedEffect<M: DurableMessage> {
    id: EffectId,
    subject: DurableSubjectId<M::Subject>,
    attempt: NonZeroU16,
    attempt_fence: WriterFence,
    policy: RetryPolicy,
    message: M,
}

impl<M> Clone for ClaimedEffect<M>
where
    M: DurableMessage + Clone,
{
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            subject: self.subject,
            attempt: self.attempt,
            attempt_fence: self.attempt_fence,
            policy: self.policy,
            message: self.message.clone(),
        }
    }
}

impl<M: DurableMessage> ClaimedEffect<M> {
    pub(crate) const fn from_store_claim(
        id: EffectId,
        subject: DurableSubjectId<M::Subject>,
        attempt: NonZeroU16,
        attempt_fence: WriterFence,
        policy: RetryPolicy,
        message: M,
    ) -> Self {
        Self {
            id,
            subject,
            attempt,
            attempt_fence,
            policy,
            message,
        }
    }

    /// Returns the stable causal effect identity.
    #[must_use]
    pub const fn id(&self) -> EffectId {
        self.id
    }

    /// Returns the causal subject.
    #[must_use]
    pub const fn subject(&self) -> DurableSubjectId<M::Subject> {
        self.subject
    }

    /// Returns the current delivery ordinal.
    #[must_use]
    pub const fn attempt(&self) -> NonZeroU16 {
        self.attempt
    }

    /// Returns the monotonic fence required to finish this attempt.
    #[must_use]
    pub const fn attempt_fence(&self) -> WriterFence {
        self.attempt_fence
    }

    /// Returns the stored retry policy.
    #[must_use]
    pub const fn policy(&self) -> RetryPolicy {
        self.policy
    }

    /// Borrows the decoded message.
    #[must_use]
    pub const fn message(&self) -> &M {
        &self.message
    }
}

/// Whether another eligible item remained after a bounded claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagePressure {
    /// More eligible effects remain.
    MoreAvailable,
    /// The eligible set was drained.
    Drained,
}

/// Bounded claimed effect page.
pub struct ClaimedEffectPage<M: DurableMessage> {
    effects: BoundedPage<ClaimedEffect<M>>,
    pressure: PagePressure,
}

impl<M: DurableMessage> ClaimedEffectPage<M> {
    pub(crate) const fn from_store_page(
        effects: BoundedPage<ClaimedEffect<M>>,
        pressure: PagePressure,
    ) -> Self {
        Self { effects, pressure }
    }

    /// Borrows claimed effects.
    #[must_use]
    pub const fn effects(&self) -> &BoundedPage<ClaimedEffect<M>> {
        &self.effects
    }

    /// Returns whether more eligible effects remain.
    #[must_use]
    pub const fn pressure(&self) -> PagePressure {
        self.pressure
    }
}

/// Consumer result persisted as application-level delivery acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectReceipt {
    effect: EffectId,
    consumer_result_digest: ContentDigest,
    completed_at: AbsoluteDeadline,
}

impl EffectReceipt {
    pub(crate) const fn from_store_receipt(
        effect: EffectId,
        consumer_result_digest: ContentDigest,
        completed_at: AbsoluteDeadline,
    ) -> Self {
        Self {
            effect,
            consumer_result_digest,
            completed_at,
        }
    }

    /// Returns the completed effect.
    #[must_use]
    pub const fn effect(self) -> EffectId {
        self.effect
    }

    /// Returns the consumer's stable application-result digest.
    #[must_use]
    pub const fn consumer_result_digest(self) -> ContentDigest {
        self.consumer_result_digest
    }

    /// Returns the persisted completion time.
    #[must_use]
    pub const fn completed_at(self) -> AbsoluteDeadline {
        self.completed_at
    }
}

/// Terminal owed effect requiring an attributed forward disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoisonedEffect {
    effect: EffectId,
    subject: ErasedDurableSubjectId,
    causal_operation: OperationId,
    failure: FailureDisposition,
    attempts: NonZeroU16,
}

impl PoisonedEffect {
    pub(crate) const fn from_store_poison(
        effect: EffectId,
        subject: ErasedDurableSubjectId,
        causal_operation: OperationId,
        failure: FailureDisposition,
        attempts: NonZeroU16,
    ) -> Self {
        Self {
            effect,
            subject,
            causal_operation,
            failure,
            attempts,
        }
    }

    /// Returns the poisoned effect identity.
    #[must_use]
    pub const fn effect(self) -> EffectId {
        self.effect
    }

    /// Returns the causal subject without losing its namespace.
    #[must_use]
    pub const fn subject(self) -> ErasedDurableSubjectId {
        self.subject
    }

    /// Returns the causal commit operation.
    #[must_use]
    pub const fn causal_operation(self) -> OperationId {
        self.causal_operation
    }

    /// Returns the terminal delivery failure.
    #[must_use]
    pub const fn failure(self) -> FailureDisposition {
        self.failure
    }

    /// Returns attempts consumed before poison.
    #[must_use]
    pub const fn attempts(self) -> NonZeroU16 {
        self.attempts
    }
}

/// Requested terminal disposition of one claimed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectFinish {
    /// Consumer durably applied or returned the existing effect result.
    Receipted {
        /// Stable application result digest.
        consumer_result_digest: ContentDigest,
        /// Completion time persisted with the receipt.
        completed_at: AbsoluteDeadline,
    },
    /// Schedule another attempt at this already selected absolute time.
    Retry {
        /// Classified failure.
        failure: FailureDisposition,
        /// Persisted next attempt time.
        next_attempt_at: AbsoluteDeadline,
    },
    /// Stop automatic delivery and retain poison evidence.
    Poison {
        /// Terminal failure.
        failure: FailureDisposition,
    },
}

impl EffectFinish {
    /// Constructs an application receipt.
    #[must_use]
    pub const fn receipted(
        consumer_result_digest: ContentDigest,
        completed_at: AbsoluteDeadline,
    ) -> Self {
        Self::Receipted {
            consumer_result_digest,
            completed_at,
        }
    }

    /// Constructs a persisted retry request.
    #[must_use]
    pub const fn retry(failure: FailureDisposition, next_attempt_at: AbsoluteDeadline) -> Self {
        Self::Retry {
            failure,
            next_attempt_at,
        }
    }
}

/// Attempt-fenced finish request.
#[derive(Debug)]
pub struct FinishEffect<M: DurableMessage> {
    effect: EffectId,
    subject: DurableSubjectId<M::Subject>,
    attempt_fence: WriterFence,
    finish: EffectFinish,
    marker: PhantomData<fn() -> M>,
}

impl<M: DurableMessage> Clone for FinishEffect<M> {
    fn clone(&self) -> Self {
        Self {
            effect: self.effect,
            subject: self.subject,
            attempt_fence: self.attempt_fence,
            finish: self.finish,
            marker: PhantomData,
        }
    }
}

impl<M: DurableMessage> FinishEffect<M> {
    /// Constructs a finish request bound to one exact claimed attempt.
    #[must_use]
    pub const fn new(claim: &ClaimedEffect<M>, finish: EffectFinish) -> Self {
        Self {
            effect: claim.id,
            subject: claim.subject,
            attempt_fence: claim.attempt_fence,
            finish,
            marker: PhantomData,
        }
    }

    pub(crate) const fn into_parts(
        self,
    ) -> (
        EffectId,
        DurableSubjectId<M::Subject>,
        WriterFence,
        EffectFinish,
    ) {
        (self.effect, self.subject, self.attempt_fence, self.finish)
    }
}

/// Available-store refusal of a finish request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectFinishRefusal {
    /// Effect does not exist.
    NotFound,
    /// Request names another subject.
    SubjectMismatch,
    /// Effect is not currently claimed.
    NotClaimed,
    /// Attempt fence has been superseded.
    StaleAttempt {
        /// Current attempt fence.
        current: WriterFence,
    },
    /// Same attempt fence was already finished with different bytes.
    FinishConflict,
}

/// Durable finish outcome, including exact idempotent replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectFinishResult {
    /// Effect completed with an application receipt.
    Receipted(EffectReceipt),
    /// Retry time was persisted.
    RetryScheduled {
        /// Effect awaiting retry.
        effect: EffectId,
        /// Exact persisted wake time.
        next_attempt_at: AbsoluteDeadline,
    },
    /// Effect reached a durable poison state.
    Poisoned(PoisonedEffect),
    /// Available storage refused an invalid request.
    Refused(EffectFinishRefusal),
}

pub(crate) fn finish_digest(finish: EffectFinish) -> ContentDigest {
    let mut bytes = Vec::with_capacity(65);
    match finish {
        EffectFinish::Receipted {
            consumer_result_digest,
            completed_at,
        } => {
            bytes.push(0);
            bytes.extend_from_slice(consumer_result_digest.as_bytes());
            bytes.extend_from_slice(&completed_at.unix_millis().to_le_bytes());
        }
        EffectFinish::Retry {
            failure,
            next_attempt_at,
        } => {
            bytes.push(1);
            bytes.push(failure_tag(failure));
            bytes.extend_from_slice(&next_attempt_at.unix_millis().to_le_bytes());
        }
        EffectFinish::Poison { failure } => {
            bytes.push(2);
            bytes.push(failure_tag(failure));
        }
    }
    ContentDigest::hash("azoth durable effect finish v1", &bytes)
}

const fn failure_tag(failure: FailureDisposition) -> u8 {
    match failure {
        FailureDisposition::Terminal => 0,
        FailureDisposition::Retryable => 1,
    }
}

pub(crate) fn bounded_claim_page<M: DurableMessage>(
    effects: Vec<ClaimedEffect<M>>,
    limit: BoundedPageSize,
    pressure: PagePressure,
) -> Result<ClaimedEffectPage<M>, BoundError> {
    BoundedPage::try_from_boxed(effects.into_boxed_slice(), limit)
        .map(|effects| ClaimedEffectPage::from_store_page(effects, pressure))
}
