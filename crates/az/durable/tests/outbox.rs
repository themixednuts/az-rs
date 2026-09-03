use std::num::NonZeroU16;

use az_durable::{
    AbsoluteDeadline, AcquireWriter, AcquireWriterResult, BoundedDiagnosticId, BoundedPageSize,
    CanonicalBytes, ClaimEffects, ClaimedEffectPage, CodecError, CodecId, CodecVersion,
    CommitResult, CommitSubject, ConformanceEvidenceRef, ContentDigest, CreateSubject,
    DurableCodec, DurableMessage, DurableNamespaceId, DurableProfileId, DurableProfileRequirement,
    DurableStateVersion, DurableSubject, DurableSubjectId, EffectFinish, EffectFinishRefusal,
    EffectFinishResult, EffectId, EffectScope, FailureDisposition, FinishEffect, InMemorySubject,
    InMemorySubjectFault, MAX_RETRY_ATTEMPTS, OperationCapacity, OperationId, OutboxDeliveryPort,
    PagePressure, ProvenProfileRef, RetryPolicy, SubjectLifecycle, SubjectMutation,
    SubjectStatePort, WriterHolder, WriterHolderKind, enqueue_effect,
};
use futures::executor::block_on;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Counter(u64);

impl DurableCodec for Counter {
    const CODEC_ID: CodecId = CodecId::from_u128(0x701);
    const CURRENT_VERSION: CodecVersion = CodecVersion::ONE;

    fn encode_canonical(&self) -> Result<CanonicalBytes, CodecError> {
        CanonicalBytes::try_from_boxed(self.0.to_le_bytes().into())
    }

    fn decode_exact(version: CodecVersion, bytes: &[u8]) -> Result<Self, CodecError> {
        if version != Self::CURRENT_VERSION {
            return Err(CodecError::UnsupportedVersion {
                stored: version,
                current: Self::CURRENT_VERSION,
            });
        }
        let bytes: [u8; 8] = bytes.try_into().map_err(|_| CodecError::Malformed)?;
        Ok(Self(u64::from_le_bytes(bytes)))
    }
}

#[derive(Debug)]
struct Account;

impl DurableSubject for Account {
    const NAMESPACE: DurableNamespaceId = DurableNamespaceId::from_u128(0x702);
    type State = Counter;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Publish(u64);

impl DurableCodec for Publish {
    const CODEC_ID: CodecId = CodecId::from_u128(0x703);
    const CURRENT_VERSION: CodecVersion = CodecVersion::ONE;

    fn encode_canonical(&self) -> Result<CanonicalBytes, CodecError> {
        CanonicalBytes::try_from_boxed(self.0.to_le_bytes().into())
    }

    fn decode_exact(version: CodecVersion, bytes: &[u8]) -> Result<Self, CodecError> {
        if version != Self::CURRENT_VERSION {
            return Err(CodecError::UnsupportedVersion {
                stored: version,
                current: Self::CURRENT_VERSION,
            });
        }
        let bytes: [u8; 8] = bytes.try_into().map_err(|_| CodecError::Malformed)?;
        Ok(Self(u64::from_le_bytes(bytes)))
    }
}

impl DurableMessage for Publish {
    type Subject = Account;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DecodeGuard(u8);

impl DurableCodec for DecodeGuard {
    const CODEC_ID: CodecId = CodecId::from_u128(0x705);
    const CURRENT_VERSION: CodecVersion = CodecVersion::ONE;

    fn encode_canonical(&self) -> Result<CanonicalBytes, CodecError> {
        CanonicalBytes::try_from_boxed(vec![self.0].into())
    }

    fn decode_exact(version: CodecVersion, bytes: &[u8]) -> Result<Self, CodecError> {
        if version != Self::CURRENT_VERSION {
            return Err(CodecError::UnsupportedVersion {
                stored: version,
                current: Self::CURRENT_VERSION,
            });
        }
        let [value] = bytes else {
            return Err(CodecError::Malformed);
        };
        if *value == 2 {
            return Err(CodecError::Malformed);
        }
        Ok(Self(*value))
    }
}

impl DurableMessage for DecodeGuard {
    type Subject = Account;
}

fn operation(value: u128) -> OperationId {
    OperationId::try_from_uuid(Uuid::from_u128(value)).expect("operation")
}

fn subject() -> DurableSubjectId<Account> {
    DurableSubjectId::derive(&CanonicalBytes::try_from_boxed(vec![0x71].into()).expect("key"))
}

fn profile() -> ProvenProfileRef {
    ProvenProfileRef::new(
        DurableProfileId::try_from_uuid(Uuid::from_u128(0x704)).expect("profile"),
        ConformanceEvidenceRef::new(ContentDigest::from_bytes([0x75; 32])),
    )
}

fn input(value: u8) -> CanonicalBytes {
    CanonicalBytes::try_from_boxed(vec![value].into()).expect("input")
}

fn holder() -> WriterHolder {
    WriterHolder::new(
        WriterHolderKind::Service,
        BoundedDiagnosticId::try_from("outbox-test").expect("holder"),
    )
}

fn new_store() -> InMemorySubject<Account> {
    let store = InMemorySubject::new(
        subject(),
        profile(),
        1_000,
        OperationCapacity::new(64).expect("capacity"),
    );
    let create = CreateSubject::new(
        subject(),
        operation(0x710),
        input(0x10),
        &Counter(0),
        DurableProfileRequirement::exact(profile()),
    )
    .expect("create");
    assert!(matches!(
        block_on(store.create(create)).expect("create"),
        CommitResult::Accepted(_)
    ));
    store
}

fn permit(store: &InMemorySubject<Account>) -> az_durable::WriterPermit<Account> {
    let request = AcquireWriter::new(
        subject(),
        holder(),
        SubjectLifecycle::Active,
        AbsoluteDeadline::MAX,
        operation(0x711),
        input(0x11),
        DurableProfileRequirement::exact(profile()),
    );
    match block_on(store.acquire_writer(request)).expect("acquire") {
        AcquireWriterResult::Acquired { permit, .. } => permit,
        AcquireWriterResult::Refused(refusal) => panic!("writer refused: {refusal:?}"),
    }
}

fn commit_effect(store: &InMemorySubject<Account>, operation_value: u128, policy: RetryPolicy) {
    let mut mutation = SubjectMutation::replace(&Counter(1)).expect("replacement");
    let value = u64::try_from(operation_value).expect("test operation fits payload");
    enqueue_effect(&mut mutation, &Publish(value), policy).expect("enqueue");
    let request = CommitSubject::new(
        permit(store),
        DurableStateVersion::initial(),
        operation(operation_value),
        input(operation_value.to_le_bytes()[0]),
        DurableProfileRequirement::exact(profile()),
        mutation,
    );
    assert!(matches!(
        block_on(store.commit(request)).expect("commit"),
        CommitResult::Accepted(_)
    ));
}

fn claim(
    store: &InMemorySubject<Account>,
    now: u64,
    lease_until: u64,
    limit: usize,
) -> ClaimedEffectPage<Publish> {
    block_on(store.claim(ClaimEffects::new(
        EffectScope::subject(subject()),
        AbsoluteDeadline::from_unix_millis(now),
        AbsoluteDeadline::from_unix_millis(lease_until),
        BoundedPageSize::new(limit).expect("limit"),
        holder(),
    )))
    .expect("claim")
}

#[test]
fn effect_is_causal_and_publishes_atomically_with_subject_state() {
    let store = new_store();
    commit_effect(&store, 0x720, RetryPolicy::Never);

    let page = claim(&store, 1_001, 2_000, 8);
    let [effect] = page.effects().as_slice() else {
        panic!("expected one effect")
    };
    assert_eq!(effect.id(), EffectId::new(operation(0x720), 0));
    assert_eq!(effect.subject(), subject());
    assert_eq!(effect.attempt(), NonZeroU16::MIN);
    assert_eq!(effect.message(), &Publish(0x720));
    assert_eq!(page.pressure(), PagePressure::Drained);

    let failed_store = new_store();
    let mut mutation = SubjectMutation::replace(&Counter(2)).expect("replacement");
    enqueue_effect(&mut mutation, &Publish(2), RetryPolicy::Never).expect("enqueue");
    failed_store.fail_next_commit_at(InMemorySubjectFault::BeforePublish);
    let request = CommitSubject::new(
        permit(&failed_store),
        DurableStateVersion::initial(),
        operation(0x721),
        input(0x21),
        DurableProfileRequirement::exact(profile()),
        mutation,
    );
    assert!(block_on(failed_store.commit(request)).is_err());
    assert!(
        claim(&failed_store, 1_001, 2_000, 8)
            .effects()
            .as_slice()
            .is_empty()
    );
}

#[test]
fn claim_decode_failure_publishes_no_attempt_or_fence() {
    let store = new_store();
    let mut mutation = SubjectMutation::unchanged();
    enqueue_effect(&mut mutation, &DecodeGuard(1), RetryPolicy::Never).expect("valid effect");
    enqueue_effect(&mut mutation, &DecodeGuard(2), RetryPolicy::Never).expect("invalid effect");
    let request = CommitSubject::new(
        permit(&store),
        DurableStateVersion::initial(),
        operation(0x725),
        input(0x25),
        DurableProfileRequirement::exact(profile()),
        mutation,
    );
    assert!(matches!(
        block_on(store.commit(request)).expect("commit"),
        CommitResult::Accepted(_)
    ));

    let failed: Result<ClaimedEffectPage<DecodeGuard>, _> =
        block_on(store.claim(ClaimEffects::new(
            EffectScope::subject(subject()),
            AbsoluteDeadline::from_unix_millis(1_001),
            AbsoluteDeadline::from_unix_millis(2_000),
            BoundedPageSize::new(2).expect("limit"),
            holder(),
        )));
    assert!(matches!(
        failed,
        Err(az_durable::StorageFailure::RecoveryRequired)
    ));

    let first: ClaimedEffectPage<DecodeGuard> = block_on(store.claim(ClaimEffects::new(
        EffectScope::subject(subject()),
        AbsoluteDeadline::from_unix_millis(1_001),
        AbsoluteDeadline::from_unix_millis(2_000),
        BoundedPageSize::new(1).expect("limit"),
        holder(),
    )))
    .expect("claim after failed transaction");
    let [effect] = first.effects().as_slice() else {
        panic!("the first effect must remain unclaimed")
    };
    assert_eq!(effect.message(), &DecodeGuard(1));
    assert_eq!(effect.attempt(), NonZeroU16::MIN);
}

#[test]
fn retry_is_persisted_and_a_stale_attempt_cannot_finish_its_successor() {
    let store = new_store();
    let policy = RetryPolicy::bounded(
        NonZeroU16::new(2).unwrap(),
        AbsoluteDeadline::from_unix_millis(10_000),
    )
    .expect("retry policy");
    commit_effect(&store, 0x730, policy);
    let first = claim(&store, 1_001, 2_000, 1).effects().as_slice()[0].clone();
    let retry = block_on(store.finish(az_durable::FinishEffect::new(
        &first,
        EffectFinish::retry(
            FailureDisposition::Retryable,
            AbsoluteDeadline::from_unix_millis(3_000),
        ),
    )))
    .expect("finish retry");
    assert!(matches!(retry, EffectFinishResult::RetryScheduled { .. }));
    assert!(
        claim(&store, 2_999, 3_500, 1)
            .effects()
            .as_slice()
            .is_empty()
    );
    let second = claim(&store, 3_000, 4_000, 1).effects().as_slice()[0].clone();
    assert_eq!(second.attempt(), NonZeroU16::new(2).unwrap());
    assert!(second.attempt_fence() > first.attempt_fence());

    let stale = block_on(store.finish(az_durable::FinishEffect::new(
        &first,
        EffectFinish::receipted(
            ContentDigest::from_bytes([0x31; 32]),
            AbsoluteDeadline::from_unix_millis(3_001),
        ),
    )))
    .expect("stale finish result");
    assert!(matches!(
        stale,
        EffectFinishResult::Refused(EffectFinishRefusal::FinishConflict)
    ));

    let finish = FinishEffect::new(
        &second,
        EffectFinish::receipted(
            ContentDigest::from_bytes([0x32; 32]),
            AbsoluteDeadline::from_unix_millis(3_002),
        ),
    );
    let accepted = block_on(store.finish(finish.clone())).expect("finish");
    let replayed = block_on(store.finish(finish)).expect("finish replay");
    assert_eq!(accepted, replayed);
    assert!(matches!(accepted, EffectFinishResult::Receipted(_)));

    let expired_store = new_store();
    commit_effect(&expired_store, 0x731, RetryPolicy::Never);
    let expired = claim(&expired_store, 1_001, 2_000, 1).effects().as_slice()[0].clone();
    let successor = claim(&expired_store, 2_000, 3_000, 1).effects().as_slice()[0].clone();
    assert!(successor.attempt_fence() > expired.attempt_fence());
    assert_eq!(successor.attempt(), expired.attempt());
    let stale = block_on(expired_store.finish(FinishEffect::new(
        &expired,
        EffectFinish::receipted(
            ContentDigest::from_bytes([0x33; 32]),
            AbsoluteDeadline::from_unix_millis(2_001),
        ),
    )))
    .expect("stale unfinished attempt");
    assert!(matches!(
        stale,
        EffectFinishResult::Refused(EffectFinishRefusal::StaleAttempt { .. })
    ));
}

#[test]
fn never_retry_policy_turns_a_failed_delivery_into_durable_poison() {
    let store = new_store();
    commit_effect(&store, 0x740, RetryPolicy::Never);
    let effect = claim(&store, 1_001, 2_000, 1).effects().as_slice()[0].clone();
    let result = block_on(store.finish(FinishEffect::new(
        &effect,
        EffectFinish::retry(
            FailureDisposition::Retryable,
            AbsoluteDeadline::from_unix_millis(3_000),
        ),
    )))
    .expect("poison finish");
    assert!(matches!(result, EffectFinishResult::Poisoned(_)));
    assert!(
        claim(&store, 3_000, 4_000, 1)
            .effects()
            .as_slice()
            .is_empty()
    );
}

#[test]
fn bounded_claim_reports_pressure_without_claiming_beyond_its_limit() {
    let store = new_store();
    let mut mutation = SubjectMutation::replace(&Counter(1)).expect("replacement");
    enqueue_effect(&mut mutation, &Publish(1), RetryPolicy::Never).expect("first effect");
    enqueue_effect(&mut mutation, &Publish(2), RetryPolicy::Never).expect("second effect");
    let request = CommitSubject::new(
        permit(&store),
        DurableStateVersion::initial(),
        operation(0x750),
        input(0x50),
        DurableProfileRequirement::exact(profile()),
        mutation,
    );
    assert!(matches!(
        block_on(store.commit(request)).expect("commit"),
        CommitResult::Accepted(_)
    ));

    let first_page = claim(&store, 1_001, 2_000, 1);
    assert_eq!(first_page.effects().as_slice().len(), 1);
    assert_eq!(first_page.pressure(), PagePressure::MoreAvailable);
}

#[test]
fn exhausted_retry_policy_poisons_the_final_attempt() {
    let store = new_store();
    let policy = RetryPolicy::bounded(
        NonZeroU16::new(2).expect("nonzero"),
        AbsoluteDeadline::from_unix_millis(10_000),
    )
    .expect("retry policy");
    commit_effect(&store, 0x760, policy);
    let first = claim(&store, 1_001, 2_000, 1).effects().as_slice()[0].clone();
    let scheduled = block_on(store.finish(FinishEffect::new(
        &first,
        EffectFinish::retry(
            FailureDisposition::Retryable,
            AbsoluteDeadline::from_unix_millis(3_000),
        ),
    )))
    .expect("schedule retry");
    assert!(matches!(
        scheduled,
        EffectFinishResult::RetryScheduled { .. }
    ));

    let second = claim(&store, 3_000, 4_000, 1).effects().as_slice()[0].clone();
    let exhausted = block_on(store.finish(FinishEffect::new(
        &second,
        EffectFinish::retry(
            FailureDisposition::Retryable,
            AbsoluteDeadline::from_unix_millis(5_000),
        ),
    )))
    .expect("finish exhausted retry");
    assert!(matches!(exhausted, EffectFinishResult::Poisoned(_)));
}

#[test]
fn outbox_port_supports_native_async_external_adapters() {
    struct External;

    impl OutboxDeliveryPort<Publish> for External {
        async fn claim(
            &self,
            _request: ClaimEffects<Publish>,
        ) -> Result<ClaimedEffectPage<Publish>, az_durable::StorageFailure> {
            Err(az_durable::StorageFailure::Unavailable)
        }

        async fn finish(
            &self,
            _request: FinishEffect<Publish>,
        ) -> Result<EffectFinishResult, az_durable::StorageFailure> {
            Err(az_durable::StorageFailure::Unavailable)
        }
    }

    assert!(
        block_on(External.claim(ClaimEffects::new(
            EffectScope::subject(subject()),
            AbsoluteDeadline::from_unix_millis(1),
            AbsoluteDeadline::from_unix_millis(2),
            BoundedPageSize::new(1).expect("limit"),
            holder(),
        )))
        .is_err()
    );
}

#[test]
fn retry_bound_accepts_one_attempt_and_rejects_above_the_hard_cap() {
    assert_eq!(
        RetryPolicy::bounded(NonZeroU16::MIN, AbsoluteDeadline::from_unix_millis(1))
            .expect("retry policy")
            .maximum_attempts(),
        NonZeroU16::MIN
    );
    assert!(
        RetryPolicy::bounded(
            NonZeroU16::new(MAX_RETRY_ATTEMPTS + 1).expect("nonzero"),
            AbsoluteDeadline::from_unix_millis(1),
        )
        .is_err()
    );
}
