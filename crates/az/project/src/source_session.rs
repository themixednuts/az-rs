//! Typed source-session state shared by project-host orchestration.

use std::{
    fmt,
    path::{Path, PathBuf},
};

use thiserror::Error;
use tracing::{info, instrument};

/// Stable fingerprint of authoritative source bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceFingerprint([u8; 32]);

impl SourceFingerprint {
    #[must_use]
    pub fn for_bytes(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SourceFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SourceFingerprint({self})")
    }
}

impl fmt::Display for SourceFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Monotonic session revision published only after source persistence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceRevision(u64);

impl SourceRevision {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    #[must_use]
    const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// Metadata persisted beside an optional typed crash-recovery value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRecoveryMetadata {
    pub recovery_id: String,
    pub source_fingerprint: SourceFingerprint,
    pub revision: SourceRevision,
    pub dirty: bool,
}

/// Typed recovery record. It is eligible only while its source fingerprint
/// still matches the saved file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRecoverySnapshot<T> {
    pub metadata: SourceRecoveryMetadata,
    pub value: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceJournalKind {
    Commit,
    Undo,
    Redo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceJournalEntry {
    pub revision: SourceRevision,
    pub kind: SourceJournalKind,
    pub source_fingerprint: SourceFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceHistoryRecord<T> {
    pub before: T,
    pub after: T,
}

/// Typed candidate held outside the session until persistence succeeds.
pub struct StagedSource<T> {
    before: T,
    candidate: T,
    kind: SourceJournalKind,
}

impl<T> StagedSource<T> {
    #[must_use]
    pub const fn candidate(&self) -> &T {
        &self.candidate
    }

    #[must_use]
    pub const fn candidate_mut(&mut self) -> &mut T {
        &mut self.candidate
    }
}

/// Source-authoritative typed editing state.
pub struct SourceSession<T> {
    source_path: PathBuf,
    value: T,
    base_fingerprint: SourceFingerprint,
    source_fingerprint: SourceFingerprint,
    revision: SourceRevision,
    dirty: bool,
    undo: Vec<SourceHistoryRecord<T>>,
    redo: Vec<SourceHistoryRecord<T>>,
    journal: Vec<SourceJournalEntry>,
    recovery: SourceRecoveryMetadata,
}

impl<T: Clone> SourceSession<T> {
    /// Opens from authoritative saved bytes, accepting a recovery value only
    /// when it was based on those exact bytes.
    #[instrument(skip_all, fields(source = %source_path.as_ref().display()))]
    pub fn open(
        source_path: impl AsRef<Path>,
        saved_value: T,
        saved_bytes: &[u8],
        recovery: Option<SourceRecoverySnapshot<T>>,
    ) -> Self {
        let source_path = source_path.as_ref().to_path_buf();
        let fingerprint = SourceFingerprint::for_bytes(saved_bytes);
        let (value, revision, dirty, recovery_id) = match recovery {
            Some(recovery) if recovery.metadata.source_fingerprint == fingerprint => (
                recovery.value,
                recovery.metadata.revision,
                recovery.metadata.dirty,
                recovery.metadata.recovery_id,
            ),
            _ => (
                saved_value,
                SourceRevision::default(),
                false,
                recovery_id(&source_path),
            ),
        };
        info!(
            source = %source_path.display(),
            revision = revision.value(),
            dirty,
            "opened typed source session"
        );
        Self {
            source_path,
            value,
            base_fingerprint: fingerprint,
            source_fingerprint: fingerprint,
            revision,
            dirty,
            undo: Vec::new(),
            redo: Vec::new(),
            journal: Vec::new(),
            recovery: SourceRecoveryMetadata {
                recovery_id,
                source_fingerprint: fingerprint,
                revision,
                dirty,
            },
        }
    }

    #[must_use]
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    #[must_use]
    pub const fn revision(&self) -> SourceRevision {
        self.revision
    }

    #[must_use]
    pub const fn base_fingerprint(&self) -> SourceFingerprint {
        self.base_fingerprint
    }

    #[must_use]
    pub const fn source_fingerprint(&self) -> SourceFingerprint {
        self.source_fingerprint
    }

    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    #[must_use]
    pub const fn undo_len(&self) -> usize {
        self.undo.len()
    }

    #[must_use]
    pub const fn redo_len(&self) -> usize {
        self.redo.len()
    }

    #[must_use]
    pub fn journal(&self) -> &[SourceJournalEntry] {
        &self.journal
    }

    /// Stage the in-memory value for a write, claiming authority over the file.
    ///
    /// # Errors
    ///
    /// Returns [`SourceSessionError::RevisionConflict`] if `expected_revision`
    /// is not this session's current revision, or
    /// [`SourceSessionError::ExternalSourceChange`] if `observed_source` differs
    /// from the fingerprint this session last persisted.
    pub fn stage_commit(
        &self,
        expected_revision: SourceRevision,
        observed_source: SourceFingerprint,
    ) -> Result<StagedSource<T>, SourceSessionError> {
        self.check_authority(expected_revision, observed_source)?;
        Ok(StagedSource {
            before: self.value.clone(),
            candidate: self.value.clone(),
            kind: SourceJournalKind::Commit,
        })
    }

    /// Stage the previous value from the undo stack for a write.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::verify_authority`] returns, or
    /// [`SourceSessionError::NothingToUndo`] if the undo stack is empty.
    pub fn stage_undo(
        &self,
        expected_revision: SourceRevision,
        observed_source: SourceFingerprint,
    ) -> Result<StagedSource<T>, SourceSessionError> {
        self.check_authority(expected_revision, observed_source)?;
        let record = self.undo.last().ok_or(SourceSessionError::NothingToUndo)?;
        Ok(StagedSource {
            before: self.value.clone(),
            candidate: record.before.clone(),
            kind: SourceJournalKind::Undo,
        })
    }

    /// Stage the next value from the redo stack for a write.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::verify_authority`] returns, or
    /// [`SourceSessionError::NothingToRedo`] if the redo stack is empty.
    pub fn stage_redo(
        &self,
        expected_revision: SourceRevision,
        observed_source: SourceFingerprint,
    ) -> Result<StagedSource<T>, SourceSessionError> {
        self.check_authority(expected_revision, observed_source)?;
        let record = self.redo.last().ok_or(SourceSessionError::NothingToRedo)?;
        Ok(StagedSource {
            before: self.value.clone(),
            candidate: record.after.clone(),
            kind: SourceJournalKind::Redo,
        })
    }

    /// Rechecks source authority after potentially expensive validation and
    /// serialization, immediately before the caller replaces the file.
    ///
    /// # Errors
    ///
    /// Returns [`SourceSessionError::RevisionConflict`] if `expected_revision`
    /// is not this session's current revision, or
    /// [`SourceSessionError::ExternalSourceChange`] if `observed_source` differs
    /// from the fingerprint this session last persisted.
    pub fn verify_authority(
        &self,
        expected_revision: SourceRevision,
        observed_source: SourceFingerprint,
    ) -> Result<(), SourceSessionError> {
        self.check_authority(expected_revision, observed_source)
    }

    /// Publishes a staged value after the caller atomically replaced the source.
    ///
    /// # Panics
    ///
    /// Panics if `staged` was produced by [`Self::stage_undo`] or
    /// [`Self::stage_redo`] and the matching stack has since been emptied: the
    /// staged kind is popped back off, and staging established that the record
    /// existed.
    pub fn publish_persisted(&mut self, staged: StagedSource<T>, source_bytes: &[u8]) {
        let fingerprint = SourceFingerprint::for_bytes(source_bytes);
        let record = SourceHistoryRecord {
            before: staged.before,
            after: staged.candidate.clone(),
        };
        match staged.kind {
            SourceJournalKind::Commit => {
                self.undo.push(record);
                self.redo.clear();
            }
            SourceJournalKind::Undo => {
                let prior = self.undo.pop().expect("stage_undo requires an undo record");
                self.redo.push(prior);
            }
            SourceJournalKind::Redo => {
                let prior = self.redo.pop().expect("stage_redo requires a redo record");
                self.undo.push(prior);
            }
        }
        self.value = staged.candidate;
        self.revision = self.revision.next();
        self.source_fingerprint = fingerprint;
        self.dirty = false;
        self.recovery.source_fingerprint = fingerprint;
        self.recovery.revision = self.revision;
        self.recovery.dirty = false;
        self.journal.push(SourceJournalEntry {
            revision: self.revision,
            kind: staged.kind,
            source_fingerprint: fingerprint,
        });
    }

    /// Replaces only the working value, for a recoverable unsaved editor state.
    pub fn replace_unsaved(&mut self, value: T) {
        self.value = value;
        self.dirty = true;
        self.recovery.dirty = true;
    }

    #[must_use]
    pub fn recovery_snapshot(&self) -> SourceRecoverySnapshot<T> {
        SourceRecoverySnapshot {
            metadata: self.recovery.clone(),
            value: self.value.clone(),
        }
    }

    fn check_authority(
        &self,
        expected_revision: SourceRevision,
        observed_source: SourceFingerprint,
    ) -> Result<(), SourceSessionError> {
        if expected_revision != self.revision {
            return Err(SourceSessionError::RevisionConflict {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        if observed_source != self.source_fingerprint {
            return Err(SourceSessionError::ExternalSourceChange {
                expected: self.source_fingerprint,
                actual: observed_source,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SourceSessionError {
    #[error("source revision conflict: expected {expected:?}, actual {actual:?}")]
    RevisionConflict {
        expected: SourceRevision,
        actual: SourceRevision,
    },
    #[error("saved source changed externally: expected {expected}, actual {actual}")]
    ExternalSourceChange {
        expected: SourceFingerprint,
        actual: SourceFingerprint,
    },
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
}

fn recovery_id(path: &Path) -> String {
    blake3::hash(path.to_string_lossy().as_bytes())
        .to_hex()
        .to_string()
}
