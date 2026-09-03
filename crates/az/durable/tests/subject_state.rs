use std::{
    num::{NonZeroU32, NonZeroU64},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use az_durable::{
    AbsoluteDeadline, AcquireWriter, AcquireWriterResult, ActiveSubject, BoundedDiagnosticId,
    CanonicalBytes, CodecError, CodecId, CodecVersion, CommitReceipt, CommitRefusal, CommitResult,
    CommitSubject, ConformanceEvidenceRef, ContentDigest, CreateSubject, DurableCodec,
    DurableNamespaceId, DurableNamespaceRegistry, DurableProfileId, DurableProfileRequirement,
    DurableStateVersion, DurableSubject, DurableSubjectId, InMemorySubject, InMemorySubjectFault,
    LoadedSubject, OperationCapacity, OperationId, OperationLookup, ProfileMismatch,
    ProvenProfileRef, RetireSubject, StorageFailure, SubjectLifecycle, SubjectMutation,
    SubjectOperationKey, SubjectOperationResult, SubjectStatePort, WriterAcquisitionRefusal,
    WriterFence, WriterGrantReceipt, WriterHolder, WriterHolderKind,
};
use futures::executor::block_on;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
struct AccountState(u64);

impl DurableCodec for AccountState {
    const CODEC_ID: CodecId = CodecId::from_u128(0x11);
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
        let encoded: [u8; 8] = bytes.try_into().map_err(|_| CodecError::Malformed)?;
        Ok(Self(u64::from_le_bytes(encoded)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Account;

impl DurableSubject for Account {
    const NAMESPACE: DurableNamespaceId = DurableNamespaceId::from_u128(0x21);
    type State = AccountState;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OtherAccount;

impl DurableSubject for OtherAccount {
    const NAMESPACE: DurableNamespaceId = DurableNamespaceId::from_u128(0x22);
    type State = AccountState;
}

#[derive(Debug)]
struct ChangingState {
    encode_calls: Arc<AtomicUsize>,
    encoded: u8,
}

impl DurableCodec for ChangingState {
    const CODEC_ID: CodecId = CodecId::from_u128(0x12);
    const CURRENT_VERSION: CodecVersion = CodecVersion::ONE;

    fn encode_canonical(&self) -> Result<CanonicalBytes, CodecError> {
        let call = self.encode_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let call = u8::try_from(call).map_err(|_| CodecError::Validation)?;
        CanonicalBytes::try_from_boxed(vec![self.encoded.saturating_add(call)].into())
    }

    fn decode_exact(version: CodecVersion, bytes: &[u8]) -> Result<Self, CodecError> {
        if version != Self::CURRENT_VERSION || bytes.len() != 1 {
            return Err(CodecError::Malformed);
        }
        Ok(Self {
            encode_calls: Arc::new(AtomicUsize::new(0)),
            encoded: bytes[0],
        })
    }
}

struct ChangingSubject;

impl DurableSubject for ChangingSubject {
    const NAMESPACE: DurableNamespaceId = DurableNamespaceId::from_u128(0x23);
    type State = ChangingState;
}

fn operation(value: u128) -> OperationId {
    OperationId::try_from_uuid(Uuid::from_u128(value)).expect("non-nil operation")
}

const fn digest(value: u8) -> ContentDigest {
    ContentDigest::from_bytes([value; 32])
}

fn subject() -> DurableSubjectId<Account> {
    DurableSubjectId::derive(&CanonicalBytes::try_from_boxed(vec![0x31].into()).expect("key"))
}

fn different_subject() -> DurableSubjectId<Account> {
    DurableSubjectId::derive(&CanonicalBytes::try_from_boxed(vec![0x32].into()).expect("key"))
}

fn profile() -> ProvenProfileRef {
    ProvenProfileRef::new(
        DurableProfileId::try_from_uuid(Uuid::from_u128(0x41)).expect("profile"),
        ConformanceEvidenceRef::new(digest(0x42)),
    )
}

fn requirement() -> DurableProfileRequirement {
    DurableProfileRequirement::exact(profile())
}

fn canonical_input(source: ContentDigest) -> CanonicalBytes {
    CanonicalBytes::try_from_boxed(source.as_bytes().to_vec().into_boxed_slice()).expect("input")
}

fn canonical_input_digest(source: ContentDigest) -> ContentDigest {
    ContentDigest::hash("azoth durable operation input v1", source.as_bytes())
}

fn capacity(value: usize) -> OperationCapacity {
    OperationCapacity::new(value).expect("nonzero operation capacity")
}

fn holder(name: &str) -> WriterHolder {
    WriterHolder::new(
        WriterHolderKind::Service,
        BoundedDiagnosticId::try_from(name).expect("bounded holder"),
    )
}

fn create_request(
    operation: OperationId,
    input: ContentDigest,
    value: u64,
) -> CreateSubject<Account> {
    CreateSubject::new(
        subject(),
        operation,
        canonical_input(input),
        &AccountState(value),
        requirement(),
    )
    .expect("canonical create")
}

fn acquire_request(
    operation: OperationId,
    input: ContentDigest,
    holder_name: &str,
) -> AcquireWriter<Account> {
    AcquireWriter::new(
        subject(),
        holder(holder_name),
        SubjectLifecycle::Active,
        AbsoluteDeadline::from_unix_millis(10_000),
        operation,
        canonical_input(input),
        requirement(),
    )
}

#[test]
fn canonical_values_and_namespace_reification_are_bounded_and_typed() {
    let oversized = vec![0_u8; az_durable::MAX_CANONICAL_VALUE_BYTES + 1].into_boxed_slice();
    assert!(matches!(
        CanonicalBytes::try_from_boxed(oversized),
        Err(CodecError::Oversized { .. })
    ));
    assert!(OperationId::try_from_uuid(Uuid::nil()).is_err());
    assert!(DurableSubjectId::<Account>::derive_from_entropy(Uuid::from_u128(1)).is_ok());

    let mut registry = DurableNamespaceRegistry::new();
    registry.register::<Account>().expect("register Account");
    let erased = subject().erase();
    assert_eq!(registry.reify::<Account>(erased).expect("reify"), subject());
    assert!(registry.reify::<OtherAccount>(erased).is_err());
    assert!(registry.register::<Account>().is_err());
}

#[test]
fn reads_never_create_and_create_is_operation_deduped() {
    let store = InMemorySubject::<Account>::new(subject(), profile(), 1_000, capacity(64));
    assert!(matches!(
        block_on(store.load(subject())).expect("load"),
        LoadedSubject::Absent { .. }
    ));

    let request = create_request(operation(1), digest(1), 7);
    let first = block_on(store.create(request.clone())).expect("create storage");
    let retry = block_on(store.create(request)).expect("retry storage");
    assert_eq!(first, retry);
    assert!(matches!(first, CommitResult::Accepted(_)));

    let conflict = block_on(store.create(create_request(operation(1), digest(2), 8)))
        .expect("conflict is a domain result");
    assert!(matches!(
        conflict,
        CommitResult::Refused(CommitRefusal::OperationConflict { .. })
    ));
    assert!(matches!(
        block_on(store.load(subject())).expect("load active"),
        LoadedSubject::Active(ActiveSubject {
            state: AccountState(7),
            ..
        })
    ));
}

#[test]
fn writer_acquisition_retries_one_grant_and_supersedes_old_permits() {
    let store = InMemorySubject::<Account>::new(subject(), profile(), 1_000, capacity(64));
    block_on(store.create(create_request(operation(2), digest(2), 10))).expect("create");

    let request = acquire_request(operation(3), digest(3), "writer-a");
    let first = block_on(store.acquire_writer(request.clone())).expect("acquire");
    let first_fence = match first {
        AcquireWriterResult::Acquired { permit, receipt } => {
            assert_eq!(permit.fence(), receipt.fence());
            receipt.fence()
        }
        AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    let retry = block_on(store.acquire_writer(request)).expect("retry acquire");
    let retry_fence = match retry {
        AcquireWriterResult::Acquired { receipt, .. } => receipt.fence(),
        AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    assert_eq!(retry_fence, first_fence);

    let changed =
        block_on(store.acquire_writer(acquire_request(operation(3), digest(4), "writer-a")))
            .expect("changed acquire");
    assert!(matches!(
        changed,
        AcquireWriterResult::Refused(WriterAcquisitionRefusal::OperationConflict { .. })
    ));

    store.set_now(10_001);
    let next = block_on(store.acquire_writer(acquire_request(operation(4), digest(5), "writer-b")))
        .expect("replacement acquire");
    let next_fence = match next {
        AcquireWriterResult::Acquired { receipt, .. } => receipt.fence(),
        AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    assert!(next_fence > first_fence);
}

#[test]
fn commits_check_fence_version_profile_and_operation_identity() {
    let store = InMemorySubject::<Account>::new(subject(), profile(), 1_000, capacity(64));
    block_on(store.create(create_request(operation(5), digest(5), 10))).expect("create");
    let acquired =
        block_on(store.acquire_writer(acquire_request(operation(6), digest(6), "writer-a")))
            .expect("acquire");
    let permit = match acquired {
        AcquireWriterResult::Acquired { permit, .. } => permit,
        AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    let request = CommitSubject::new(
        permit,
        DurableStateVersion::initial(),
        operation(7),
        canonical_input(digest(7)),
        requirement(),
        SubjectMutation::replace(&AccountState(11)).expect("mutation"),
    );
    let first = block_on(store.commit(request)).expect("commit");
    let receipt = match first {
        CommitResult::Accepted(receipt) => receipt,
        CommitResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    assert_eq!(receipt.version().get(), 1);

    let lookup = block_on(store.lookup(
        SubjectOperationKey::new(subject(), operation(7)),
        canonical_input_digest(digest(7)),
    ))
    .expect("lookup");
    assert!(matches!(
        lookup,
        OperationLookup::Existing(SubjectOperationResult::Commit(CommitResult::Accepted(_)))
    ));

    let stale =
        block_on(store.acquire_writer(acquire_request(operation(8), digest(8), "writer-b")))
            .expect("held result");
    assert!(matches!(
        stale,
        AcquireWriterResult::Refused(WriterAcquisitionRefusal::HeldUntil { .. })
    ));

    let mismatch = DurableProfileRequirement::exact(ProvenProfileRef::new(
        DurableProfileId::try_from_uuid(Uuid::from_u128(0x99)).expect("other profile"),
        ConformanceEvidenceRef::new(digest(0x42)),
    ));
    assert_eq!(
        mismatch.check(profile()),
        Err(ProfileMismatch {
            required: mismatch,
            actual: profile(),
        })
    );
}

#[test]
fn every_staged_commit_fault_leaves_no_half_write() {
    for fault in InMemorySubjectFault::ALL {
        let store = InMemorySubject::<Account>::new(subject(), profile(), 1_000, capacity(64));
        block_on(store.create(create_request(operation(9), digest(9), 20))).expect("create");
        let acquired =
            block_on(store.acquire_writer(acquire_request(operation(10), digest(10), "writer-a")))
                .expect("acquire");
        let permit = match acquired {
            AcquireWriterResult::Acquired { permit, .. } => permit,
            AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
        };
        store.fail_next_commit_at(fault);
        let result = block_on(store.commit(CommitSubject::new(
            permit,
            DurableStateVersion::initial(),
            operation(11),
            canonical_input(digest(11)),
            requirement(),
            SubjectMutation::replace(&AccountState(21)).expect("mutation"),
        )));
        assert_eq!(result, Err(StorageFailure::Unavailable));
        assert!(matches!(
            block_on(store.load(subject())).expect("load"),
            LoadedSubject::Active(ActiveSubject {
                version,
                state: AccountState(20),
                ..
            }) if version == DurableStateVersion::initial()
        ));
        assert!(matches!(
            block_on(store.lookup(
                SubjectOperationKey::new(subject(), operation(11)),
                canonical_input_digest(digest(11)),
            ))
            .expect("lookup"),
            OperationLookup::Absent
        ));
    }
}

#[test]
fn retirement_is_fenced_terminal_and_preserves_operation_evidence() {
    let store = InMemorySubject::<Account>::new(subject(), profile(), 1_000, capacity(64));
    block_on(store.create(create_request(operation(12), digest(12), 30))).expect("create");
    let acquired =
        block_on(store.acquire_writer(acquire_request(operation(13), digest(13), "writer-a")))
            .expect("acquire");
    let permit = match acquired {
        AcquireWriterResult::Acquired { permit, .. } => permit,
        AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    let result = block_on(store.retire(RetireSubject::new(
        permit,
        DurableStateVersion::initial(),
        operation(14),
        canonical_input(digest(14)),
        requirement(),
        SubjectMutation::replace(&AccountState(31)).expect("final mutation"),
    )))
    .expect("retire");
    assert!(matches!(result, CommitResult::Accepted(_)));
    assert!(matches!(
        block_on(store.load(subject())).expect("retired"),
        LoadedSubject::Retired(retired)
            if retired.retirement_operation() == operation(14)
                && retired.version().get() == 1
    ));
    assert!(matches!(
        block_on(store.create(create_request(operation(15), digest(15), 0)))
            .expect("terminal refusal"),
        CommitResult::Refused(CommitRefusal::Retired)
    ));
}

#[test]
fn lease_expiry_is_diagnostic_until_a_newer_fence_is_issued() {
    let store = InMemorySubject::<Account>::new(subject(), profile(), 1_000, capacity(64));
    block_on(store.create(create_request(operation(20), digest(20), 40))).expect("create");
    let request = acquire_request(operation(21), digest(21), "writer-a");
    let first = block_on(store.acquire_writer(request.clone())).expect("acquire");
    let replay = block_on(store.acquire_writer(request)).expect("replay");
    let current_permit = match first {
        AcquireWriterResult::Acquired { permit, .. } => permit,
        AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    let stale_permit = match replay {
        AcquireWriterResult::Acquired { permit, .. } => permit,
        AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };

    store.set_now(10_001);
    assert!(matches!(
        block_on(store.commit(CommitSubject::new(
            current_permit,
            DurableStateVersion::initial(),
            operation(22),
            canonical_input(digest(22)),
            requirement(),
            SubjectMutation::replace(&AccountState(41)).expect("mutation"),
        )))
        .expect("commit after diagnostic expiry"),
        CommitResult::Accepted(_)
    ));

    let replacement =
        block_on(store.acquire_writer(acquire_request(operation(23), digest(23), "writer-b")))
            .expect("replacement grant");
    assert!(matches!(replacement, AcquireWriterResult::Acquired { .. }));
    assert!(matches!(
        block_on(store.commit(CommitSubject::new(
            stale_permit,
            DurableStateVersion::from_store_value(1),
            operation(24),
            canonical_input(digest(24)),
            requirement(),
            SubjectMutation::unchanged(),
        )))
        .expect("stale commit result"),
        CommitResult::Refused(CommitRefusal::WriterSuperseded { .. })
    ));
}

#[test]
fn wrong_subject_requests_never_mutate_or_issue_a_permit() {
    let store = InMemorySubject::<Account>::new(subject(), profile(), 1_000, capacity(64));
    let wrong_create = CreateSubject::new(
        different_subject(),
        operation(25),
        canonical_input(digest(25)),
        &AccountState(50),
        requirement(),
    )
    .expect("create request");
    assert!(matches!(
        block_on(store.create(wrong_create)).expect("wrong create result"),
        CommitResult::Refused(CommitRefusal::SubjectMismatch)
    ));
    assert!(matches!(
        block_on(store.load(subject())).expect("load"),
        LoadedSubject::Absent { .. }
    ));

    block_on(store.create(create_request(operation(26), digest(26), 51))).expect("create");
    let wrong_acquire = AcquireWriter::new(
        different_subject(),
        holder("writer-a"),
        SubjectLifecycle::Active,
        AbsoluteDeadline::from_unix_millis(10_000),
        operation(27),
        canonical_input(digest(27)),
        requirement(),
    );
    assert!(matches!(
        block_on(store.acquire_writer(wrong_acquire)).expect("wrong acquire result"),
        AcquireWriterResult::Refused(WriterAcquisitionRefusal::SubjectMismatch)
    ));
}

#[test]
fn profile_evidence_codec_identity_and_operation_capacity_are_enforced() {
    let store = InMemorySubject::<Account>::new(subject(), profile(), 1_000, capacity(2));
    block_on(store.create(create_request(operation(28), digest(28), 60))).expect("create");
    let loaded = block_on(store.load(subject())).expect("load");
    assert!(matches!(
        loaded,
        LoadedSubject::Active(ActiveSubject { codec_id, .. })
            if codec_id == AccountState::CODEC_ID
    ));

    let acquire = acquire_request(operation(29), digest(29), "writer-a");
    let first = block_on(store.acquire_writer(acquire.clone())).expect("acquire");
    let replay = block_on(store.acquire_writer(acquire)).expect("replay");
    let permit = match first {
        AcquireWriterResult::Acquired { permit, .. } => permit,
        AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    let wrong_evidence = DurableProfileRequirement::exact(ProvenProfileRef::new(
        profile().profile(),
        ConformanceEvidenceRef::new(digest(0x99)),
    ));
    assert!(matches!(
        block_on(store.retire(RetireSubject::new(
            permit,
            DurableStateVersion::initial(),
            operation(30),
            canonical_input(digest(30)),
            wrong_evidence,
            SubjectMutation::unchanged(),
        )))
        .expect("profile refusal"),
        CommitResult::Refused(CommitRefusal::ProfileInsufficient(_))
    ));
    let replay_permit = match replay {
        AcquireWriterResult::Acquired { permit, .. } => permit,
        AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    assert!(matches!(
        block_on(store.commit(CommitSubject::new(
            replay_permit,
            DurableStateVersion::initial(),
            operation(31),
            canonical_input(digest(31)),
            requirement(),
            SubjectMutation::unchanged(),
        )))
        .expect("capacity refusal"),
        CommitResult::Refused(CommitRefusal::OperationCapacityExceeded)
    ));
}

#[test]
fn a_stored_codec_identity_mismatch_requires_recovery() {
    let store = InMemorySubject::<Account>::new(subject(), profile(), 1_000, capacity(64));
    block_on(store.create(create_request(operation(35), digest(35), 90))).expect("create");
    store.set_stored_codec_for_conformance(CodecId::from_u128(0xdead));
    assert_eq!(
        block_on(store.load(subject())).err(),
        Some(StorageFailure::RecoveryRequired)
    );
}

#[test]
fn mutation_and_operation_input_are_sealed_once() {
    let key = CanonicalBytes::try_from_boxed(vec![0x33].into()).expect("key");
    let id = DurableSubjectId::<ChangingSubject>::derive(&key);
    let store = InMemorySubject::<ChangingSubject>::new(id, profile(), 1_000, capacity(64));
    let initial_calls = Arc::new(AtomicUsize::new(0));
    let create = CreateSubject::new(
        id,
        operation(40),
        canonical_input(digest(40)),
        &ChangingState {
            encode_calls: initial_calls,
            encoded: 0,
        },
        requirement(),
    )
    .expect("create request");
    block_on(store.create(create)).expect("create");
    let acquired = block_on(store.acquire_writer(AcquireWriter::new(
        id,
        holder("changing-writer"),
        SubjectLifecycle::Active,
        AbsoluteDeadline::from_unix_millis(10_000),
        operation(41),
        canonical_input(digest(41)),
        requirement(),
    )))
    .expect("acquire");
    let permit = match acquired {
        AcquireWriterResult::Acquired { permit, .. } => permit,
        AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    let mutation_calls = Arc::new(AtomicUsize::new(0));
    let mutation = SubjectMutation::<ChangingSubject>::replace(&ChangingState {
        encode_calls: Arc::clone(&mutation_calls),
        encoded: 0,
    })
    .expect("mutation");
    assert_eq!(mutation_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        mutation.codec(),
        (ChangingState::CODEC_ID, ChangingState::CURRENT_VERSION)
    );
    assert_eq!(
        mutation.replacement().expect("replacement").as_slice(),
        &[1]
    );
    let request = CommitSubject::new(
        permit,
        DurableStateVersion::initial(),
        operation(42),
        canonical_input(digest(42)),
        requirement(),
        mutation,
    );
    assert_eq!(
        request.canonical_input_digest(),
        canonical_input_digest(digest(42))
    );
    assert_eq!(request.mutation().encoded_len(), 1);
    assert!(matches!(
        block_on(store.commit(request)).expect("commit"),
        CommitResult::Accepted(_)
    ));
    assert_eq!(mutation_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        block_on(store.load(id)).expect("load"),
        LoadedSubject::Active(ActiveSubject {
            state: ChangingState { encoded: 1, .. },
            ..
        })
    ));
}

struct ExternalAdapter;

impl SubjectStatePort<Account> for ExternalAdapter {
    async fn load(
        &self,
        subject: DurableSubjectId<Account>,
    ) -> Result<LoadedSubject<Account>, StorageFailure> {
        Ok(LoadedSubject::Absent { id: subject })
    }

    async fn create(
        &self,
        request: CreateSubject<Account>,
    ) -> Result<CommitResult<Account>, StorageFailure> {
        let (codec_id, codec_version) = request.codec();
        assert_eq!(
            (codec_id, codec_version),
            (AccountState::CODEC_ID, AccountState::CURRENT_VERSION)
        );
        let receipt = CommitReceipt::from_store_commit(
            request.id(),
            request.operation(),
            request.canonical_input_digest(),
            DurableStateVersion::initial(),
            ContentDigest::hash("state", request.initial_state().as_slice()),
            profile(),
        );
        Ok(CommitResult::Accepted(receipt))
    }

    async fn acquire_writer(
        &self,
        request: AcquireWriter<Account>,
    ) -> Result<AcquireWriterResult<Account>, StorageFailure> {
        let receipt = WriterGrantReceipt::from_store_grant(
            request.subject(),
            request.operation(),
            request.canonical_input_digest(),
            WriterFence::from_store_value(NonZeroU64::MIN),
            1,
            request.lease_until(),
            profile(),
        );
        let permit = receipt.issue_permit();
        Ok(AcquireWriterResult::Acquired { permit, receipt })
    }

    async fn lookup(
        &self,
        key: SubjectOperationKey<Account>,
        _canonical_input_digest: ContentDigest,
    ) -> Result<OperationLookup<Account>, StorageFailure> {
        let _ = (key.subject(), key.operation());
        Ok(OperationLookup::Absent)
    }

    async fn commit(
        &self,
        request: CommitSubject<Account>,
    ) -> Result<CommitResult<Account>, StorageFailure> {
        let _ = (
            request.permit().fence(),
            request.expected_version(),
            request.operation(),
            request.canonical_input(),
            request.required_profile(),
            request.mutation(),
        );
        Ok(CommitResult::Refused(CommitRefusal::NotFound))
    }

    async fn retire(
        &self,
        request: RetireSubject<Account>,
    ) -> Result<CommitResult<Account>, StorageFailure> {
        let _ = (
            request.permit().fence(),
            request.expected_version(),
            request.operation(),
            request.canonical_input(),
            request.required_profile(),
            request.final_mutation(),
        );
        Ok(CommitResult::Refused(CommitRefusal::NotFound))
    }
}

#[test]
fn external_adapter_can_use_native_async_and_construct_bound_results() {
    let adapter = ExternalAdapter;
    let created = block_on(adapter.create(create_request(operation(33), digest(33), 80)))
        .expect("external create");
    let receipt = match created {
        CommitResult::Accepted(receipt) => receipt,
        CommitResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    assert_eq!(receipt.subject(), subject());
    assert_eq!(receipt.operation(), operation(33));
    assert_eq!(receipt.profile(), profile());

    let acquired =
        block_on(adapter.acquire_writer(acquire_request(operation(34), digest(34), "external")))
            .expect("external acquire");
    let receipt = match acquired {
        AcquireWriterResult::Acquired { permit, receipt } => {
            assert_eq!(permit.subject(), receipt.subject());
            receipt
        }
        AcquireWriterResult::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    };
    assert_eq!(receipt.input_digest(), canonical_input_digest(digest(34)));
    assert_eq!(receipt.profile(), profile());
}

#[test]
fn codec_version_is_nonzero() {
    assert!(CodecVersion::try_from(0).is_err());
    assert_eq!(CodecVersion::try_from(1).expect("one"), CodecVersion::ONE);
    assert_eq!(CodecVersion::ONE.get(), NonZeroU32::new(1).unwrap().get());
}
