use std::num::{NonZeroU64, NonZeroUsize};

use az_durable::{
    AbsoluteDeadline, AcquireWriter, AcquireWriterResult, BoundedDiagnosticId, BoundedPageSize,
    CanonicalBytes, CodecError, CodecId, CodecVersion, CommitResult, CommitSubject,
    ConformanceEvidenceRef, ContentDigest, CreateSubject, DurableCodec, DurableNamespaceId,
    DurableProfileId, DurableProfileRequirement, DurableRecord, DurableSnapshot,
    DurableStateVersion, DurableSubject, DurableSubjectId, InMemorySubject, InMemorySubjectFault,
    JournalReadPort, LoadedSubject, MAX_MUTATION_CLAUSES, OperationCapacity, OperationId,
    ProvenProfileRef, RecordId, RecordsAfter, ReplayPolicy, SnapshotReadPort, SnapshotRequest,
    StorageFailure, SubjectLifecycle, SubjectMutation, SubjectStatePort, VerifiedReplay,
    WriterHolder, WriterHolderKind, append_record, verify_replay,
};
use futures::executor::block_on;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Balance(u64);

impl DurableCodec for Balance {
    const CODEC_ID: CodecId = CodecId::from_u128(0x501);
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

#[derive(Debug, Clone)]
struct Account;

impl DurableSubject for Account {
    const NAMESPACE: DurableNamespaceId = DurableNamespaceId::from_u128(0x502);
    type State = Balance;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Credited(u64);

impl DurableCodec for Credited {
    const CODEC_ID: CodecId = CodecId::from_u128(0x503);
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

impl DurableRecord for Credited {
    type Subject = Account;
}

struct CreditReplay;

impl ReplayPolicy<Account, Credited> for CreditReplay {
    type Error = &'static str;

    fn apply(&self, state: &Balance, record: &Credited) -> Result<Balance, Self::Error> {
        state
            .0
            .checked_add(record.0)
            .map(Balance)
            .ok_or("balance overflow")
    }
}

fn operation(value: u128) -> OperationId {
    OperationId::try_from_uuid(Uuid::from_u128(value)).expect("non-nil operation")
}

fn subject() -> DurableSubjectId<Account> {
    DurableSubjectId::derive(
        &CanonicalBytes::try_from_boxed(vec![0x51].into()).expect("canonical key"),
    )
}

fn profile() -> ProvenProfileRef {
    ProvenProfileRef::new(
        DurableProfileId::try_from_uuid(Uuid::from_u128(0x504)).expect("profile"),
        ConformanceEvidenceRef::new(ContentDigest::from_bytes([0x55; 32])),
    )
}

fn input(value: u8) -> CanonicalBytes {
    CanonicalBytes::try_from_boxed(vec![value].into()).expect("canonical input")
}

fn store() -> InMemorySubject<Account> {
    InMemorySubject::new(
        subject(),
        profile(),
        1_000,
        OperationCapacity::new(64).expect("capacity"),
    )
}

fn create(store: &InMemorySubject<Account>) {
    let request = CreateSubject::new(
        subject(),
        operation(0x510),
        input(0x10),
        &Balance(0),
        DurableProfileRequirement::exact(profile()),
    )
    .expect("create request");
    assert!(matches!(
        block_on(store.create(request)).expect("create"),
        CommitResult::Accepted(_)
    ));
}

fn permit(store: &InMemorySubject<Account>) -> az_durable::WriterPermit<Account> {
    let request = AcquireWriter::new(
        subject(),
        WriterHolder::new(
            WriterHolderKind::Service,
            BoundedDiagnosticId::try_from("journal-test").expect("holder"),
        ),
        SubjectLifecycle::Active,
        AbsoluteDeadline::MAX,
        operation(0x5ff),
        input(0x5f),
        DurableProfileRequirement::exact(profile()),
    );
    match block_on(store.acquire_writer(request)).expect("acquire") {
        AcquireWriterResult::Acquired { permit, .. } => permit,
        AcquireWriterResult::Refused(refusal) => panic!("writer refused: {refusal:?}"),
    }
}

fn commit_credit(
    store: &InMemorySubject<Account>,
    expected_version: DurableStateVersion,
    operation_value: u128,
    amount: u64,
) {
    let balance = match block_on(store.load(subject())).expect("load before commit") {
        LoadedSubject::Active(active) => active.state.0,
        LoadedSubject::Absent { .. } | LoadedSubject::Retired(_) => panic!("subject not active"),
    };
    let mut mutation =
        SubjectMutation::replace(&Balance(balance + amount)).expect("state replacement");
    append_record(&mut mutation, &Credited(amount)).expect("journal clause");
    let request = CommitSubject::new(
        permit(store),
        expected_version,
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

#[test]
fn journal_record_and_state_publish_as_one_hash_linked_atom() {
    let store = store();
    create(&store);

    let mut mutation = SubjectMutation::replace(&Balance(7)).expect("replacement");
    append_record(&mut mutation, &Credited(7)).expect("journal clause");
    let request = CommitSubject::new(
        permit(&store),
        DurableStateVersion::initial(),
        operation(0x521),
        input(0x21),
        DurableProfileRequirement::exact(profile()),
        mutation,
    );
    let receipt = match block_on(store.commit(request)).expect("commit") {
        CommitResult::Accepted(receipt) => receipt,
        CommitResult::Refused(refusal) => panic!("commit refused: {refusal:?}"),
    };

    let page = block_on(store.records_after(RecordsAfter::<Credited>::new(
        subject(),
        None,
        BoundedPageSize::new(8).expect("page size"),
    )))
    .expect("journal page");
    let [record] = page.records().as_slice() else {
        panic!("expected exactly one record")
    };
    assert_eq!(record.id().sequence(), NonZeroU64::MIN);
    assert_eq!(record.subject(), subject());
    assert_eq!(record.prior_hash(), ContentDigest::from_bytes([0; 32]));
    assert_eq!(record.output_digest(), receipt.state_digest());
    assert_eq!(record.state_version(), receipt.version());
    assert_eq!(
        record.codec(),
        (Credited::CODEC_ID, Credited::CURRENT_VERSION)
    );
    assert_eq!(record.record(), &Credited(7));
    assert_eq!(page.terminal_root(), record.id().hash());

    let mut failed = SubjectMutation::replace(&Balance(9)).expect("replacement");
    append_record(&mut failed, &Credited(2)).expect("journal clause");
    store.fail_next_commit_at(InMemorySubjectFault::BeforePublish);
    let failed = CommitSubject::new(
        permit(&store),
        receipt.version(),
        operation(0x523),
        input(0x23),
        DurableProfileRequirement::exact(profile()),
        failed,
    );
    assert_eq!(
        block_on(store.commit(failed)).expect_err("injected failure"),
        StorageFailure::Unavailable
    );
    let page = block_on(store.records_after(RecordsAfter::<Credited>::new(
        subject(),
        None,
        BoundedPageSize::new(8).expect("page size"),
    )))
    .expect("journal page");
    assert_eq!(page.records().as_slice().len(), 1);
    assert!(matches!(
        block_on(store.load(subject())).expect("load"),
        LoadedSubject::Active(active) if active.state == Balance(7)
    ));
}

#[test]
fn bounded_pages_replay_from_snapshot_and_report_retention_gaps() {
    let store = store();
    create(&store);
    let snapshot: DurableSnapshot<Account> = block_on(store.snapshot(SnapshotRequest::new(
        subject(),
        Some(DurableStateVersion::initial()),
    )))
    .expect("snapshot read")
    .expect("initial snapshot");
    assert_eq!(
        snapshot.codec(),
        (Balance::CODEC_ID, Balance::CURRENT_VERSION)
    );

    commit_credit(&store, DurableStateVersion::initial(), 0x530, 1);
    commit_credit(&store, DurableStateVersion::from_store_value(1), 0x531, 2);
    commit_credit(&store, DurableStateVersion::from_store_value(2), 0x532, 3);

    let first = block_on(store.records_after(RecordsAfter::<Credited>::new(
        subject(),
        snapshot.journal_position(),
        BoundedPageSize::new(2).expect("page size"),
    )))
    .expect("first page");
    assert_eq!(first.records().as_slice().len(), 2);
    let cursor = first.next_after().expect("more records");
    let second = block_on(store.records_after(RecordsAfter::<Credited>::new(
        subject(),
        Some(cursor),
        BoundedPageSize::new(2).expect("page size"),
    )))
    .expect("second page");
    assert_eq!(second.records().as_slice().len(), 1);
    assert_eq!(second.next_after(), None);

    let records = first
        .records()
        .as_slice()
        .iter()
        .chain(second.records().as_slice())
        .cloned()
        .collect::<Box<[_]>>();
    let replayed: VerifiedReplay<Account> =
        verify_replay(snapshot, &records, &CreditReplay).expect("verified replay");
    assert_eq!(replayed.terminal_state(), &Balance(6));
    assert_eq!(
        replayed.terminal_version(),
        DurableStateVersion::from_store_value(3)
    );
    assert_eq!(replayed.terminal_root(), second.terminal_root());

    let earliest: RecordId = first.records().as_slice()[1].id();
    store.retain_journal_from_for_conformance(earliest);
    let gap = block_on(store.records_after(RecordsAfter::<Credited>::new(
        subject(),
        None,
        BoundedPageSize::new(8).expect("page size"),
    )))
    .expect("gap page")
    .retention_gap()
    .expect("retention gap");
    assert_eq!(gap.earliest_available(), earliest);
    assert!(gap.required_snapshot().is_some());
}

#[test]
fn journal_read_port_supports_native_async_external_adapters() {
    struct External;

    impl SnapshotReadPort<Account> for External {
        async fn snapshot(
            &self,
            _request: SnapshotRequest<Account>,
        ) -> Result<Option<DurableSnapshot<Account>>, StorageFailure> {
            Ok(None)
        }
    }

    impl JournalReadPort<Credited> for External {
        async fn records_after(
            &self,
            _request: RecordsAfter<Credited>,
        ) -> Result<az_durable::JournalPage<Credited>, StorageFailure> {
            Err(StorageFailure::Unavailable)
        }
    }

    assert!(
        block_on(External.snapshot(SnapshotRequest::new(subject(), None)))
            .expect("external snapshot")
            .is_none()
    );
}

#[test]
fn altered_journal_bytes_are_refused_before_decode_or_replay() {
    let store = store();
    create(&store);
    commit_credit(&store, DurableStateVersion::initial(), 0x540, 4);

    assert!(store.corrupt_journal_record_for_conformance(0));
    let result = block_on(store.records_after(RecordsAfter::<Credited>::new(
        subject(),
        None,
        BoundedPageSize::new(8).expect("page size"),
    )));
    assert!(matches!(result, Err(StorageFailure::RecoveryRequired)));
}

#[test]
fn multiple_records_share_one_committed_state_version_and_replay_as_a_group() {
    let store = store();
    create(&store);
    let snapshot = block_on(store.snapshot(SnapshotRequest::new(
        subject(),
        Some(DurableStateVersion::initial()),
    )))
    .expect("snapshot read")
    .expect("initial snapshot");

    let mut mutation = SubjectMutation::replace(&Balance(5)).expect("replacement");
    append_record(&mut mutation, &Credited(2)).expect("first record");
    append_record(&mut mutation, &Credited(3)).expect("second record");
    let request = CommitSubject::new(
        permit(&store),
        DurableStateVersion::initial(),
        operation(0x550),
        input(0x50),
        DurableProfileRequirement::exact(profile()),
        mutation,
    );
    assert!(matches!(
        block_on(store.commit(request)).expect("commit"),
        CommitResult::Accepted(_)
    ));
    let page = block_on(store.records_after(RecordsAfter::<Credited>::new(
        subject(),
        None,
        BoundedPageSize::new(8).expect("page size"),
    )))
    .expect("journal page");
    let [first, second] = page.records().as_slice() else {
        panic!("expected two records")
    };
    assert_eq!(first.state_version(), second.state_version());
    assert_eq!(first.output_digest(), second.output_digest());
    let replayed =
        verify_replay(snapshot, page.records().as_slice(), &CreditReplay).expect("group replay");
    assert_eq!(replayed.terminal_state(), &Balance(5));
}

#[test]
fn mutation_clause_count_is_hard_bounded_before_storage() {
    let mut mutation = SubjectMutation::<Account>::unchanged();
    for _ in 0..MAX_MUTATION_CLAUSES {
        append_record(&mut mutation, &Credited(1)).expect("within clause bound");
    }
    assert!(append_record(&mut mutation, &Credited(1)).is_err());
    assert_eq!(mutation.clause_count(), MAX_MUTATION_CLAUSES);
}

#[test]
fn page_bounds_are_nonzero_and_capped() {
    assert!(BoundedPageSize::new(0).is_err());
    assert!(BoundedPageSize::new(4_097).is_err());
    assert_eq!(
        BoundedPageSize::new(4_096).expect("hard cap").get(),
        NonZeroUsize::new(4_096).unwrap().get()
    );
}
