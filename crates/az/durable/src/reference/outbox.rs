use std::{collections::BTreeMap, num::NonZeroU16};

use crate::{
    AbsoluteDeadline, ClaimEffects, ClaimedEffect, ClaimedEffectPage, ContentDigest,
    DurableMessage, DurableSubject, EffectFinish, EffectFinishRefusal, EffectFinishResult,
    EffectId, EffectReceipt, FinishEffect, OutboxDeliveryPort, PagePressure, PoisonedEffect,
    RetryPolicy, StorageFailure, WriterFence, WriterHolder,
    outbox::{OutboxClause, bounded_claim_page, finish_digest},
};

use super::{InMemorySubject, OperationId};

#[derive(Debug, Clone)]
enum StoredEffectState {
    Pending {
        next_attempt_at: AbsoluteDeadline,
    },
    Claimed {
        lease_until: AbsoluteDeadline,
        _worker: WriterHolder,
    },
    Receipted,
    Poisoned,
}

#[derive(Debug, Clone)]
struct StoredFinish {
    fence: WriterFence,
    digest: ContentDigest,
    result: EffectFinishResult,
}

#[derive(Debug, Clone)]
pub struct StoredEffect {
    id: EffectId,
    causal_operation: OperationId,
    codec_id: crate::CodecId,
    codec_version: crate::CodecVersion,
    bytes: crate::CanonicalBytes,
    policy: RetryPolicy,
    attempts_started: u16,
    fence: WriterFence,
    state: StoredEffectState,
    finishes: Vec<StoredFinish>,
}

impl StoredEffect {
    fn eligible_at(&self, now: AbsoluteDeadline) -> bool {
        match &self.state {
            StoredEffectState::Pending { next_attempt_at } => *next_attempt_at <= now,
            StoredEffectState::Claimed { lease_until, .. } => *lease_until <= now,
            StoredEffectState::Receipted | StoredEffectState::Poisoned => false,
        }
    }
}

impl<T: DurableSubject> InMemorySubject<T> {
    /// Assigns causal effect identities without mutating published reference state.
    ///
    /// # Errors
    ///
    /// Fails closed if an operation-local effect ordinal cannot fit in `u16`.
    pub fn stage_outbox(
        operation: OperationId,
        clauses: impl IntoIterator<Item = OutboxClause>,
    ) -> Result<Vec<(EffectId, StoredEffect)>, StorageFailure> {
        clauses
            .into_iter()
            .enumerate()
            .map(|(index, clause)| {
                let index = u16::try_from(index).map_err(|_| StorageFailure::RecoveryRequired)?;
                let id = EffectId::new(operation, index);
                Ok((
                    id,
                    StoredEffect {
                        id,
                        causal_operation: operation,
                        codec_id: clause.codec_id,
                        codec_version: clause.codec_version,
                        bytes: clause.bytes,
                        policy: clause.policy,
                        attempts_started: 0,
                        fence: WriterFence::ZERO,
                        state: StoredEffectState::Pending {
                            next_attempt_at: AbsoluteDeadline::from_unix_millis(0),
                        },
                        finishes: Vec::new(),
                    },
                ))
            })
            .collect()
    }

    fn claim_sync<M>(
        &self,
        request: ClaimEffects<M>,
    ) -> Result<ClaimedEffectPage<M>, StorageFailure>
    where
        M: DurableMessage<Subject = T>,
    {
        let mut state = self.lock();
        let mut eligible = state
            .effects
            .iter()
            .filter(|(_, effect)| {
                effect.codec_id == M::CODEC_ID
                    && effect.eligible_at(request.now())
                    && request.scope().matches(self.id)
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let pressure = if eligible.len() > request.limit().get() {
            PagePressure::MoreAvailable
        } else {
            PagePressure::Drained
        };
        eligible.truncate(request.limit().get());
        let mut updates = BTreeMap::new();
        let mut claimed = Vec::with_capacity(eligible.len());
        for id in eligible {
            let stored = state
                .effects
                .get(&id)
                .ok_or(StorageFailure::RecoveryRequired)?;
            let attempts_started = if matches!(stored.state, StoredEffectState::Pending { .. }) {
                if stored.attempts_started >= stored.policy.maximum_attempts().get() {
                    return Err(StorageFailure::RecoveryRequired);
                }
                stored
                    .attempts_started
                    .checked_add(1)
                    .ok_or(StorageFailure::RecoveryRequired)?
            } else {
                stored.attempts_started
            };
            let attempt =
                NonZeroU16::new(attempts_started).ok_or(StorageFailure::RecoveryRequired)?;
            let attempt_fence = stored
                .fence
                .checked_next()
                .map_err(|_| StorageFailure::RecoveryRequired)?;
            let message = M::decode_exact(stored.codec_version, stored.bytes.as_slice())
                .map_err(|_| StorageFailure::RecoveryRequired)?;
            updates.insert(id, (attempts_started, attempt_fence));
            claimed.push(ClaimedEffect::from_store_claim(
                stored.id,
                self.id,
                attempt,
                attempt_fence,
                stored.policy,
                message,
            ));
        }
        let page = bounded_claim_page(claimed, request.limit(), pressure)
            .map_err(|_| StorageFailure::RecoveryRequired)?;
        for (id, stored) in &mut state.effects {
            let Some(&(attempts_started, attempt_fence)) = updates.get(id) else {
                continue;
            };
            stored.attempts_started = attempts_started;
            stored.fence = attempt_fence;
            stored.state = StoredEffectState::Claimed {
                lease_until: request.lease_until(),
                _worker: request.worker().clone(),
            };
        }
        Ok(page)
    }

    fn finish_sync<M>(&self, request: FinishEffect<M>) -> Result<EffectFinishResult, StorageFailure>
    where
        M: DurableMessage<Subject = T>,
    {
        let (effect_id, subject, attempt_fence, finish) = request.into_parts();
        if subject != self.id {
            return Ok(EffectFinishResult::Refused(
                EffectFinishRefusal::SubjectMismatch,
            ));
        }
        let digest = finish_digest(finish);
        let mut state = self.lock();
        let Some(stored) = state.effects.get_mut(&effect_id) else {
            return Ok(EffectFinishResult::Refused(EffectFinishRefusal::NotFound));
        };
        if let Some(previous) = stored
            .finishes
            .iter()
            .find(|previous| previous.fence == attempt_fence)
        {
            return Ok(if previous.digest == digest {
                previous.result.clone()
            } else {
                EffectFinishResult::Refused(EffectFinishRefusal::FinishConflict)
            });
        }
        if stored.fence != attempt_fence {
            return Ok(EffectFinishResult::Refused(
                EffectFinishRefusal::StaleAttempt {
                    current: stored.fence,
                },
            ));
        }
        if !matches!(stored.state, StoredEffectState::Claimed { .. }) {
            return Ok(EffectFinishResult::Refused(EffectFinishRefusal::NotClaimed));
        }
        let attempt =
            NonZeroU16::new(stored.attempts_started).ok_or(StorageFailure::RecoveryRequired)?;
        if stored.finishes.len() >= usize::from(stored.policy.maximum_attempts().get()) {
            return Err(StorageFailure::RecoveryRequired);
        }
        let result = match finish {
            EffectFinish::Receipted {
                consumer_result_digest,
                completed_at,
            } => {
                let receipt = EffectReceipt::from_store_receipt(
                    stored.id,
                    consumer_result_digest,
                    completed_at,
                );
                stored.state = StoredEffectState::Receipted;
                EffectFinishResult::Receipted(receipt)
            }
            EffectFinish::Retry {
                failure,
                next_attempt_at,
            } if failure == crate::FailureDisposition::Retryable
                && stored.policy.permits_retry(attempt, next_attempt_at) =>
            {
                stored.state = StoredEffectState::Pending { next_attempt_at };
                EffectFinishResult::RetryScheduled {
                    effect: stored.id,
                    next_attempt_at,
                }
            }
            EffectFinish::Retry { failure, .. } | EffectFinish::Poison { failure } => {
                let poisoned = PoisonedEffect::from_store_poison(
                    stored.id,
                    self.id.erase(),
                    stored.causal_operation,
                    failure,
                    attempt,
                );
                stored.state = StoredEffectState::Poisoned;
                EffectFinishResult::Poisoned(poisoned)
            }
        };
        stored.finishes.push(StoredFinish {
            fence: attempt_fence,
            digest,
            result: result.clone(),
        });
        Ok(result)
    }
}

impl<T, M> OutboxDeliveryPort<M> for InMemorySubject<T>
where
    T: DurableSubject,
    M: DurableMessage<Subject = T>,
{
    async fn claim(
        &self,
        request: ClaimEffects<M>,
    ) -> Result<ClaimedEffectPage<M>, StorageFailure> {
        self.claim_sync(request)
    }

    async fn finish(&self, request: FinishEffect<M>) -> Result<EffectFinishResult, StorageFailure> {
        self.finish_sync(request)
    }
}
