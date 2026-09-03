//! Project-host orchestration for typed [`SourceSession`](az_project::SourceSession) values.

use std::{
    fs,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use az_filesystem::{FileTransaction, FileWrite};
use az_project::{
    SourceFingerprint, SourceRecoverySnapshot, SourceRevision, SourceSession, SourceSessionError,
    StagedSource,
};
use thiserror::Error;
use tracing::{info, instrument};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDiagnosticPhase {
    Edit,
    WholeDocument,
    PrefabSemantics,
    Dependency,
    Serialize,
    Recovery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDiagnostic {
    pub phase: SourceDiagnosticPhase,
    pub path: Option<String>,
    pub message: String,
}

impl SourceDiagnostic {
    #[must_use]
    pub fn new(phase: SourceDiagnosticPhase, message: impl Into<String>) -> Self {
        Self {
            phase,
            path: None,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn at_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

/// Registered typed editing and validation policy owned by project-host.
pub trait TypedSourcePolicy {
    type Value: Clone;
    type Command;

    /// Decode the on-disk source bytes into the typed value.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceDiagnostic`] when `bytes` are not a well-formed
    /// document for this policy.
    fn decode(&self, bytes: &[u8]) -> Result<Self::Value, SourceDiagnostic>;
    /// Apply one typed edit command to the staged candidate in place.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceDiagnostic`] when `command` does not address anything
    /// `candidate` holds, or when applying it would leave the candidate
    /// malformed. `candidate` is then abandoned rather than persisted.
    fn apply(
        &self,
        candidate: &mut Self::Value,
        command: &Self::Command,
    ) -> Result<(), SourceDiagnostic>;
    fn validate_document(&self, candidate: &Self::Value) -> Vec<SourceDiagnostic>;
    fn validate_prefab_semantics(&self, candidate: &Self::Value) -> Vec<SourceDiagnostic>;
    fn validate_dependencies(&self, candidate: &Self::Value) -> Vec<SourceDiagnostic>;
    /// Serialize the validated candidate back to source bytes.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceDiagnostic`] when `candidate` cannot be rendered as
    /// source text — the commit is then abandoned before anything is written.
    fn serialize(&self, candidate: &Self::Value) -> Result<Vec<u8>, SourceDiagnostic>;
}

/// Source-file boundary, separated for deterministic rollback tests.
pub trait TypedSourcePersistence {
    /// Read the authoritative bytes currently stored at `path`.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when `path` cannot be read, or when
    /// replaying an interrupted authored write fails first.
    fn read(&mut self, path: &Path) -> Result<Vec<u8>, String>;
    /// Replace the document at `path` with `bytes` as one all-or-nothing write.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when the replacement does not become
    /// durable, or when replaying an interrupted authored write fails first.
    /// The document at `path` is then left at its previous contents.
    fn atomically_replace(&mut self, path: &Path, bytes: &[u8]) -> Result<(), String>;
}

/// DB-backed recovery seam. Recovery errors are intentionally best effort.
pub trait TypedSourceRecoveryStore<T> {
    /// Load the unsaved-edit snapshot recorded for `path`, if any.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when the store cannot be queried.
    /// Callers treat that as "no snapshot" and open the saved source instead.
    fn load(&mut self, path: &Path) -> Result<Option<SourceRecoverySnapshot<T>>, String>;
    /// Record `snapshot` as the recoverable state for `path`.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when the store rejects the write. The
    /// commit that produced `snapshot` still stands; the caller surfaces the
    /// failure as a [`SourceDiagnosticPhase::Recovery`] warning.
    fn write(&mut self, path: &Path, snapshot: &SourceRecoverySnapshot<T>) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoSourceRecoveryStore<T>(PhantomData<fn() -> T>);

impl<T> NoSourceRecoveryStore<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> TypedSourceRecoveryStore<T> for NoSourceRecoveryStore<T> {
    fn load(&mut self, _path: &Path) -> Result<Option<SourceRecoverySnapshot<T>>, String> {
        Ok(None)
    }

    fn write(&mut self, _path: &Path, _snapshot: &SourceRecoverySnapshot<T>) -> Result<(), String> {
        Ok(())
    }
}

/// Production source persistence using the engine's recoverable file transaction.
#[derive(Debug, Clone)]
pub struct FileSourcePersistence {
    transactions: FileTransaction,
}

impl FileSourcePersistence {
    #[must_use]
    pub fn new(transaction_root: impl Into<PathBuf>) -> Self {
        Self {
            transactions: FileTransaction::new(transaction_root),
        }
    }

    /// Replay any authored write that a crash interrupted after its marker
    /// became durable.
    ///
    /// [`FileTransaction`] recovery is forward-only: once the marker reaches
    /// `applying` the staged bytes are the committed intent, and the only way
    /// they reach the document is a later `recover_pending`. Nothing else owns
    /// this transaction root, so this boundary is the only place that can
    /// replay it. This is the one persistence path carrying user-authored
    /// content, so a dropped replay is authored data loss rather than work that
    /// reprocessing can rebuild.
    ///
    /// Both the read and the write path recover, which is deliberate. The write
    /// path follows `az-daemon`'s project-service store and `gems/auth`'s host
    /// secret store, which recover immediately before each commit. The read
    /// path follows `az-session`'s supervisor lease, which recovers in
    /// `read_unlocked`; without it a reopened session decodes the superseded
    /// bytes and the author then edits on top of a write that is still pending.
    fn recover_pending(&self) -> Result<(), String> {
        let reports = self.transactions.recover_pending().map_err(|error| {
            format!(
                "failed to recover pending source writes under {}: {error}",
                self.transactions.root().display()
            )
        })?;
        for report in reports {
            info!(
                transaction = %report.id,
                write_count = report.recovered_write_count,
                root = %self.transactions.root().display(),
                "replayed an interrupted authored source write"
            );
        }
        Ok(())
    }
}

impl TypedSourcePersistence for FileSourcePersistence {
    fn read(&mut self, path: &Path) -> Result<Vec<u8>, String> {
        self.recover_pending()?;
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))
    }

    fn atomically_replace(&mut self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        self.recover_pending()?;
        self.transactions
            .commit([FileWrite::new(path, bytes)])
            .map(|receipt| {
                debug_assert_eq!(receipt.write_count, 1);
            })
            .map_err(|error| format!("failed to replace {}: {error}", path.display()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCommitReceipt {
    pub revision: SourceRevision,
    pub source_fingerprint: SourceFingerprint,
    pub recovery_warning: Option<SourceDiagnostic>,
}

/// Owns typed edit policy, persistence, validation, and recovery orchestration.
pub struct SourceSessionHost<Policy, Persistence, Recovery> {
    policy: Policy,
    persistence: Persistence,
    recovery: Recovery,
}

impl<Policy, Persistence, Recovery> SourceSessionHost<Policy, Persistence, Recovery> {
    #[must_use]
    pub const fn new(policy: Policy, persistence: Persistence, recovery: Recovery) -> Self {
        Self {
            policy,
            persistence,
            recovery,
        }
    }

    #[must_use]
    pub const fn persistence(&self) -> &Persistence {
        &self.persistence
    }

    #[must_use]
    pub const fn policy(&self) -> &Policy {
        &self.policy
    }

    #[must_use]
    pub const fn policy_mut(&mut self) -> &mut Policy {
        &mut self.policy
    }

    #[must_use]
    pub const fn persistence_mut(&mut self) -> &mut Persistence {
        &mut self.persistence
    }

    #[must_use]
    pub const fn recovery(&self) -> &Recovery {
        &self.recovery
    }
}

impl<Policy, Persistence, Recovery> SourceSessionHost<Policy, Persistence, Recovery>
where
    Policy: TypedSourcePolicy,
    Persistence: TypedSourcePersistence,
    Recovery: TypedSourceRecoveryStore<Policy::Value>,
{
    /// Open a typed editing session over the source file at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceSessionHostError::Read`] when the persistence layer
    /// cannot read `path`, or [`SourceSessionHostError::Diagnostics`] carrying
    /// the policy's decode diagnostic when those bytes are not a well-formed
    /// document. A failing recovery-store load is not an error: it is logged
    /// and the saved source is opened instead.
    #[instrument(skip_all, fields(source = %path.as_ref().display()))]
    pub fn open(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<SourceSession<Policy::Value>, SourceSessionHostError> {
        let path = path.as_ref();
        let bytes = self
            .persistence
            .read(path)
            .map_err(SourceSessionHostError::Read)?;
        let saved = self
            .policy
            .decode(&bytes)
            .map_err(|diagnostic| SourceSessionHostError::Diagnostics(vec![diagnostic]))?;
        let recovery = match self.recovery.load(path) {
            Ok(recovery) => recovery,
            Err(error) => {
                info!(source = %path.display(), %error, "typed recovery load failed; opening saved source");
                None
            }
        };
        Ok(SourceSession::open(path, saved, &bytes, recovery))
    }

    #[instrument(skip_all, fields(source = %session.source_path().display(), expected_revision = expected_revision.value()))]
    /// Apply `command` to `session` and persist the result.
    ///
    /// # Errors
    ///
    /// Returns [`SourceSessionHostError::Read`] when the on-disk fingerprint
    /// cannot be re-read, [`SourceSessionHostError::Session`] when
    /// `expected_revision` does not match or the file changed underneath the
    /// session, [`SourceSessionHostError::Diagnostics`] when the policy rejects
    /// the command or the resulting document, and
    /// [`SourceSessionHostError::Persist`] when the replacement write fails.
    pub fn commit(
        &mut self,
        session: &mut SourceSession<Policy::Value>,
        expected_revision: SourceRevision,
        command: &Policy::Command,
    ) -> Result<SourceCommitReceipt, SourceSessionHostError> {
        let observed = self.read_fingerprint(session.source_path())?;
        let mut staged = session.stage_commit(expected_revision, observed)?;
        self.policy
            .apply(staged.candidate_mut(), command)
            .map_err(|diagnostic| SourceSessionHostError::Diagnostics(vec![diagnostic]))?;
        self.validate_and_persist(session, staged)
    }

    #[instrument(skip_all, fields(source = %session.source_path().display(), expected_revision = expected_revision.value()))]
    /// Step `session` back one revision and persist the result.
    ///
    /// # Errors
    ///
    /// Returns [`SourceSessionHostError::Read`] when the on-disk fingerprint
    /// cannot be re-read, [`SourceSessionHostError::Session`] when
    /// `expected_revision` does not match, the file changed underneath the
    /// session, or there is nothing left to undo, plus any
    /// [`SourceSessionHostError::Diagnostics`] or
    /// [`SourceSessionHostError::Persist`] the shared validate-and-persist tail
    /// raises.
    pub fn undo(
        &mut self,
        session: &mut SourceSession<Policy::Value>,
        expected_revision: SourceRevision,
    ) -> Result<SourceCommitReceipt, SourceSessionHostError> {
        let observed = self.read_fingerprint(session.source_path())?;
        let staged = session.stage_undo(expected_revision, observed)?;
        self.validate_and_persist(session, staged)
    }

    #[instrument(skip_all, fields(source = %session.source_path().display(), expected_revision = expected_revision.value()))]
    /// Step `session` forward one undone revision and persist the result.
    ///
    /// # Errors
    ///
    /// Returns [`SourceSessionHostError::Read`] when the on-disk fingerprint
    /// cannot be re-read, [`SourceSessionHostError::Session`] when
    /// `expected_revision` does not match, the file changed underneath the
    /// session, or there is nothing left to redo, plus any
    /// [`SourceSessionHostError::Diagnostics`] or
    /// [`SourceSessionHostError::Persist`] the shared validate-and-persist tail
    /// raises.
    pub fn redo(
        &mut self,
        session: &mut SourceSession<Policy::Value>,
        expected_revision: SourceRevision,
    ) -> Result<SourceCommitReceipt, SourceSessionHostError> {
        let observed = self.read_fingerprint(session.source_path())?;
        let staged = session.stage_redo(expected_revision, observed)?;
        self.validate_and_persist(session, staged)
    }

    #[instrument(skip_all, fields(source = %session.source_path().display(), revision = session.revision().value()))]
    pub fn save_recovery(
        &mut self,
        session: &SourceSession<Policy::Value>,
    ) -> Option<SourceDiagnostic> {
        self.recovery
            .write(session.source_path(), &session.recovery_snapshot())
            .err()
            .map(|message| SourceDiagnostic::new(SourceDiagnosticPhase::Recovery, message))
    }

    #[instrument(skip_all, fields(source = %session.source_path().display(), revision = session.revision().value()))]
    pub fn close(&mut self, session: &SourceSession<Policy::Value>) -> Option<SourceDiagnostic> {
        self.save_recovery(session)
    }

    fn validate_and_persist(
        &mut self,
        session: &mut SourceSession<Policy::Value>,
        staged: StagedSource<Policy::Value>,
    ) -> Result<SourceCommitReceipt, SourceSessionHostError> {
        let mut diagnostics = self.policy.validate_document(staged.candidate());
        diagnostics.extend(self.policy.validate_prefab_semantics(staged.candidate()));
        diagnostics.extend(self.policy.validate_dependencies(staged.candidate()));
        if !diagnostics.is_empty() {
            return Err(SourceSessionHostError::Diagnostics(diagnostics));
        }
        let bytes = self
            .policy
            .serialize(staged.candidate())
            .map_err(|diagnostic| SourceSessionHostError::Diagnostics(vec![diagnostic]))?;
        let observed = self.read_fingerprint(session.source_path())?;
        session.verify_authority(session.revision(), observed)?;
        self.persistence
            .atomically_replace(session.source_path(), &bytes)
            .map_err(SourceSessionHostError::Persist)?;

        session.publish_persisted(staged, &bytes);
        let recovery_warning = self.save_recovery(session);
        info!(
            source = %session.source_path().display(),
            revision = session.revision().value(),
            "committed typed source session"
        );
        Ok(SourceCommitReceipt {
            revision: session.revision(),
            source_fingerprint: session.source_fingerprint(),
            recovery_warning,
        })
    }

    fn read_fingerprint(
        &mut self,
        path: &Path,
    ) -> Result<SourceFingerprint, SourceSessionHostError> {
        self.persistence
            .read(path)
            .map(|bytes| SourceFingerprint::for_bytes(&bytes))
            .map_err(SourceSessionHostError::Read)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SourceSessionHostError {
    #[error(transparent)]
    Session(#[from] SourceSessionError),
    #[error("failed to read authoritative source: {0}")]
    Read(String),
    #[error("typed source validation failed")]
    Diagnostics(Vec<SourceDiagnostic>),
    #[error("typed source persistence failed: {0}")]
    Persist(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestDocument {
        value: i32,
    }

    #[derive(Debug, Clone, Copy)]
    enum TestCommand {
        Add(i32),
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct TestPolicy;

    impl TypedSourcePolicy for TestPolicy {
        type Value = TestDocument;
        type Command = TestCommand;

        fn decode(&self, bytes: &[u8]) -> Result<Self::Value, SourceDiagnostic> {
            let value = std::str::from_utf8(bytes)
                .ok()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| {
                    SourceDiagnostic::new(
                        SourceDiagnosticPhase::Serialize,
                        "invalid integer source",
                    )
                })?;
            Ok(TestDocument { value })
        }

        fn apply(
            &self,
            candidate: &mut TestDocument,
            command: &TestCommand,
        ) -> Result<(), SourceDiagnostic> {
            match command {
                TestCommand::Add(amount) => candidate.value += amount,
            }
            Ok(())
        }

        fn validate_document(&self, candidate: &TestDocument) -> Vec<SourceDiagnostic> {
            (candidate.value < 0)
                .then(|| {
                    SourceDiagnostic::new(
                        SourceDiagnosticPhase::WholeDocument,
                        "value must be non-negative",
                    )
                    .at_path(".value")
                })
                .into_iter()
                .collect()
        }

        fn validate_prefab_semantics(&self, _candidate: &TestDocument) -> Vec<SourceDiagnostic> {
            Vec::new()
        }

        fn validate_dependencies(&self, _candidate: &TestDocument) -> Vec<SourceDiagnostic> {
            Vec::new()
        }

        fn serialize(&self, candidate: &TestDocument) -> Result<Vec<u8>, SourceDiagnostic> {
            Ok(candidate.value.to_string().into_bytes())
        }
    }

    #[derive(Debug, Clone)]
    struct MemoryPersistence {
        bytes: Vec<u8>,
        persist_count: usize,
        fail_persist: bool,
    }

    impl MemoryPersistence {
        fn new(value: i32) -> Self {
            Self {
                bytes: value.to_string().into_bytes(),
                persist_count: 0,
                fail_persist: false,
            }
        }
    }

    impl TypedSourcePersistence for MemoryPersistence {
        fn read(&mut self, _path: &Path) -> Result<Vec<u8>, String> {
            Ok(self.bytes.clone())
        }

        fn atomically_replace(&mut self, _path: &Path, bytes: &[u8]) -> Result<(), String> {
            self.persist_count += 1;
            if self.fail_persist {
                return Err("injected persistence failure".to_owned());
            }
            self.bytes = bytes.to_vec();
            Ok(())
        }
    }

    #[derive(Debug, Clone, Default)]
    struct MemoryRecovery {
        snapshot: Option<SourceRecoverySnapshot<TestDocument>>,
        write_count: usize,
        fail_write: bool,
    }

    impl TypedSourceRecoveryStore<TestDocument> for MemoryRecovery {
        fn load(
            &mut self,
            _path: &Path,
        ) -> Result<Option<SourceRecoverySnapshot<TestDocument>>, String> {
            Ok(self.snapshot.clone())
        }

        fn write(
            &mut self,
            _path: &Path,
            snapshot: &SourceRecoverySnapshot<TestDocument>,
        ) -> Result<(), String> {
            self.write_count += 1;
            if self.fail_write {
                return Err("injected recovery failure".to_owned());
            }
            self.snapshot = Some(snapshot.clone());
            Ok(())
        }
    }

    type TestHost = SourceSessionHost<TestPolicy, MemoryPersistence, MemoryRecovery>;

    fn host(value: i32) -> TestHost {
        SourceSessionHost::new(
            TestPolicy,
            MemoryPersistence::new(value),
            MemoryRecovery::default(),
        )
    }

    /// Leave the transaction root in exactly the state a crash between
    /// `status=applying` and the forward apply produces: the staged authored
    /// bytes and the marker are durable, and the destination still holds the
    /// previous document.
    ///
    /// A directory at the destination makes the forward apply fail on both
    /// Windows (`MoveFileExW` refuses to replace a directory) and Unix
    /// (`rename(2)` reports `EISDIR`), so the real commit path aborts after the
    /// marker is durable. That is the same on-disk state a power loss leaves,
    /// produced without reaching into the transaction's private layout.
    fn stage_interrupted_authored_write(
        transaction_root: &Path,
        source: &Path,
        previous: &[u8],
        authored: &[u8],
    ) {
        std::fs::create_dir_all(source).expect("block the destination");
        FileSourcePersistence::new(transaction_root)
            .atomically_replace(source, authored)
            .expect_err("the forward apply must not succeed");
        std::fs::remove_dir(source).expect("unblock the destination");
        fs::write(source, previous).expect("restore the previous document");
    }

    fn pending_transaction_count(transaction_root: &Path) -> usize {
        match std::fs::read_dir(transaction_root) {
            Ok(entries) => entries
                .collect::<Result<Vec<_>, _>>()
                .expect("read every transaction-root entry")
                .len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => panic!("failed to read {}: {error}", transaction_root.display()),
        }
    }

    #[test]
    fn file_source_persistence_recovers_authored_write_interrupted_by_crash() {
        let temp = tempfile::tempdir().expect("tempdir");
        let transaction_root = temp.path().join("transactions");
        let source = temp.path().join("authored.prefab.ron");
        stage_interrupted_authored_write(&transaction_root, &source, b"1", b"42");

        assert_eq!(
            fs::read(&source).expect("previous document"),
            b"1",
            "the interrupted write must not have reached the destination yet"
        );
        assert_eq!(
            pending_transaction_count(&transaction_root),
            1,
            "the crash must leave one pending transaction to replay"
        );

        // Reopening the session stands in for the process restart. The authored
        // edit must be rolled forward, not stranded in the staging directory.
        let mut host = SourceSessionHost::new(
            TestPolicy,
            FileSourcePersistence::new(&transaction_root),
            MemoryRecovery::default(),
        );
        let session = host.open(&source).expect("open the recovered source");

        assert_eq!(
            session.value().value,
            42,
            "the reopened session must decode the recovered authored document"
        );
        assert_eq!(
            fs::read(&source).expect("recovered document"),
            b"42",
            "the authored bytes must be on disk after recovery"
        );
        assert_eq!(
            pending_transaction_count(&transaction_root),
            0,
            "a replayed transaction must not be left to replay again"
        );
    }

    #[test]
    fn file_source_persistence_replays_pending_write_before_the_next_commit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let transaction_root = temp.path().join("transactions");
        let source = temp.path().join("authored.prefab.ron");
        stage_interrupted_authored_write(&transaction_root, &source, b"1", b"42");

        let mut persistence = FileSourcePersistence::new(&transaction_root);
        persistence
            .atomically_replace(&source, b"43")
            .expect("commit the next authored revision");

        assert_eq!(
            fs::read(&source).expect("document"),
            b"43",
            "the newest authored revision wins"
        );
        assert_eq!(
            pending_transaction_count(&transaction_root),
            0,
            "the stale pending write must be drained, not left to resurrect later"
        );
    }

    #[test]
    fn source_session_rejects_revision_conflict_before_edit() {
        let mut host = host(1);
        let mut session = host.open("test.prefab.ron").expect("open");
        let error = host
            .commit(&mut session, SourceRevision::new(7), &TestCommand::Add(1))
            .expect_err("revision conflict");
        assert!(matches!(
            error,
            SourceSessionHostError::Session(SourceSessionError::RevisionConflict { .. })
        ));
        assert_eq!(session.value().value, 1);
        assert_eq!(host.persistence.persist_count, 0);
    }

    #[test]
    fn source_session_rejects_external_source_change() {
        let mut host = host(1);
        let mut session = host.open("test.prefab.ron").expect("open");
        host.persistence.bytes = b"9".to_vec();
        let error = host
            .commit(&mut session, SourceRevision::new(0), &TestCommand::Add(1))
            .expect_err("external source change");
        assert!(matches!(
            error,
            SourceSessionHostError::Session(SourceSessionError::ExternalSourceChange { .. })
        ));
        assert_eq!(session.value().value, 1);
        assert_eq!(host.persistence.persist_count, 0);
    }

    #[test]
    fn source_session_routes_validation_diagnostics_without_persisting() {
        let mut host = host(1);
        let mut session = host.open("test.prefab.ron").expect("open");
        let error = host
            .commit(&mut session, SourceRevision::new(0), &TestCommand::Add(-2))
            .expect_err("validation failure");
        let SourceSessionHostError::Diagnostics(diagnostics) = error else {
            panic!("expected routed diagnostics");
        };
        assert_eq!(diagnostics[0].phase, SourceDiagnosticPhase::WholeDocument);
        assert_eq!(diagnostics[0].path.as_deref(), Some(".value"));
        assert_eq!(session.value().value, 1);
        assert_eq!(host.persistence.persist_count, 0);
    }

    #[test]
    fn source_session_persistence_failure_rolls_back_candidate_and_revision() {
        let mut host = host(1);
        let mut session = host.open("test.prefab.ron").expect("open");
        host.persistence.fail_persist = true;
        let error = host
            .commit(&mut session, SourceRevision::new(0), &TestCommand::Add(2))
            .expect_err("persistence failure");
        assert!(matches!(error, SourceSessionHostError::Persist(_)));
        assert_eq!(session.value().value, 1);
        assert_eq!(session.revision(), SourceRevision::new(0));
        assert_eq!(session.undo_len(), 0);
        assert_eq!(host.persistence.bytes, b"1");
    }

    #[test]
    fn source_session_undo_redo_are_typed_and_persisted() {
        let mut host = host(1);
        let mut session = host.open("test.prefab.ron").expect("open");
        host.commit(&mut session, SourceRevision::new(0), &TestCommand::Add(2))
            .expect("commit");
        host.undo(&mut session, SourceRevision::new(1))
            .expect("undo");
        assert_eq!(session.value().value, 1);
        assert_eq!(session.redo_len(), 1);
        host.redo(&mut session, SourceRevision::new(2))
            .expect("redo");
        assert_eq!(session.value().value, 3);
        assert_eq!(session.undo_len(), 1);
        assert_eq!(host.persistence.persist_count, 3);
    }

    #[test]
    fn source_session_commit_persists_exactly_once_then_writes_recovery() {
        let mut host = host(1);
        let mut session = host.open("test.prefab.ron").expect("open");
        let receipt = host
            .commit(&mut session, SourceRevision::new(0), &TestCommand::Add(1))
            .expect("commit");
        assert_eq!(receipt.revision, SourceRevision::new(1));
        assert_eq!(host.persistence.persist_count, 1);
        assert_eq!(host.recovery.write_count, 1);
        assert_eq!(host.persistence.bytes, b"2");
    }

    #[test]
    fn source_session_recovery_is_best_effort_after_successful_source_commit() {
        let mut host = host(1);
        host.recovery.fail_write = true;
        let mut session = host.open("test.prefab.ron").expect("open");
        let receipt = host
            .commit(&mut session, SourceRevision::new(0), &TestCommand::Add(1))
            .expect("source commit remains successful");
        assert!(receipt.recovery_warning.is_some());
        assert_eq!(session.value().value, 2);
        assert_eq!(host.persistence.bytes, b"2");
    }

    #[test]
    fn source_session_opens_matching_crash_recovery() {
        let mut host = host(1);
        let fingerprint = SourceFingerprint::for_bytes(b"1");
        host.recovery.snapshot = Some(SourceRecoverySnapshot {
            metadata: az_project::SourceRecoveryMetadata {
                recovery_id: "recovery".to_owned(),
                source_fingerprint: fingerprint,
                revision: SourceRevision::new(4),
                dirty: true,
            },
            value: TestDocument { value: 9 },
        });
        let session = host.open("test.prefab.ron").expect("open recovery");
        assert_eq!(session.value().value, 9);
        assert_eq!(session.revision(), SourceRevision::new(4));
        assert!(session.is_dirty());
    }

    #[test]
    fn source_file_precedes_stale_db_recovery() {
        let mut host = host(2);
        host.recovery.snapshot = Some(SourceRecoverySnapshot {
            metadata: az_project::SourceRecoveryMetadata {
                recovery_id: "stale".to_owned(),
                source_fingerprint: SourceFingerprint::for_bytes(b"1"),
                revision: SourceRevision::new(4),
                dirty: true,
            },
            value: TestDocument { value: 9 },
        });
        let session = host.open("test.prefab.ron").expect("open saved source");
        assert_eq!(session.value().value, 2);
        assert_eq!(session.revision(), SourceRevision::new(0));
        assert!(!session.is_dirty());
    }
}
