use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Mutex, MutexGuard},
};

use crate::{
    AuthorityAtomId, AuthoritySequence, CanonicalRequestDigest, CertifiedExecutionBinding,
    ContentDigest, FederationPrincipalId, OperationId,
};

use super::{
    AuthorityOperationLookupFuture, AuthorityOperationLookupPort, PublicationOutcome,
    PublicationStep, TransparencyLoadFuture, TransparencyOutboxFuture, TransparencyOutboxPort,
    outbox::TransparencyOutboxItem,
};

/// Provider-neutral inability to satisfy the atomic commit contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StorageFailure {
    /// The provider cannot currently serve the request.
    #[error("authority storage is unavailable")]
    Unavailable,
    /// Stored authority state cannot be trusted without recovery.
    #[error("authority storage requires recovery")]
    RecoveryRequired,
}

impl StorageFailure {
    /// Reports whether retry or reconciliation may succeed after availability returns.
    #[must_use]
    pub const fn is_unavailable(self) -> bool {
        matches!(self, Self::Unavailable)
    }
}

/// Precise authority refusal caused by stale or mismatched command context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitRefusal {
    /// The request addresses another authority atom.
    WrongAuthorityAtom,
    /// The request's lifecycle fence is no longer current.
    StalePlacementFence,
    /// The bounded grant revision is no longer current.
    StaleGrantRevision,
    /// The request predates the current revocation view.
    StaleRevocationCursor,
    /// The atom changed after the caller read it.
    StaleStateVersion,
    /// Another certified execution fact no longer matches current authority.
    CertifiedBindingMismatch,
}

/// Complete proposal for one atomic portable-state commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityAtomCommit {
    atom: AuthorityAtomId,
    operation: OperationId,
    principal: FederationPrincipalId,
    binding: CertifiedExecutionBinding,
    expected_state_version: u64,
    request_digest: CanonicalRequestDigest,
    state_after: ContentDigest,
    decision: ContentDigest,
    transparency_commitment: ContentDigest,
}

impl AuthorityAtomCommit {
    /// Creates a fully bound atomic commit proposal.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        atom: AuthorityAtomId,
        operation: OperationId,
        principal: FederationPrincipalId,
        binding: CertifiedExecutionBinding,
        expected_state_version: u64,
        request_digest: CanonicalRequestDigest,
        state_after: ContentDigest,
        decision: ContentDigest,
        transparency_commitment: ContentDigest,
    ) -> Self {
        Self {
            atom,
            operation,
            principal,
            binding,
            expected_state_version,
            request_digest,
            state_after,
            decision,
            transparency_commitment,
        }
    }

    /// Returns the addressed atom.
    #[must_use]
    pub const fn atom(&self) -> AuthorityAtomId {
        self.atom
    }
    /// Returns the caller's stable operation identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }
    /// Returns the authenticated principal.
    #[must_use]
    pub const fn principal(&self) -> FederationPrincipalId {
        self.principal
    }
    /// Returns the complete execution binding.
    #[must_use]
    pub const fn binding(&self) -> &CertifiedExecutionBinding {
        &self.binding
    }
    /// Returns the expected atom version.
    #[must_use]
    pub const fn expected_state_version(&self) -> u64 {
        self.expected_state_version
    }
    /// Returns the canonical request digest used for idempotency.
    #[must_use]
    pub const fn request_digest(&self) -> CanonicalRequestDigest {
        self.request_digest
    }
}

/// Immutable receipt for one accepted atom commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityReceipt {
    operation: OperationId,
    principal: FederationPrincipalId,
    request_digest: CanonicalRequestDigest,
    binding: CertifiedExecutionBinding,
    state_version: u64,
    authority_sequence: AuthoritySequence,
    state_after: ContentDigest,
    decision: ContentDigest,
}

impl AuthorityReceipt {
    /// Returns the accepted operation identity.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }
    /// Returns the canonical bytes this receipt accepted.
    ///
    /// Reconciliation of an unknown outcome compares this against the original
    /// request, so a repeat that changed its bytes can never adopt the receipt.
    #[must_use]
    pub const fn request_digest(&self) -> CanonicalRequestDigest {
        self.request_digest
    }
    /// Returns the resulting atom state version.
    #[must_use]
    pub const fn state_version(&self) -> u64 {
        self.state_version
    }
    /// Returns the resulting authority order.
    #[must_use]
    pub const fn authority_sequence(&self) -> AuthoritySequence {
        self.authority_sequence
    }
}

/// Observable result of an atomic commit attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityAtomCommitResult {
    /// This call committed the operation.
    Committed(AuthorityReceipt),
    /// An identical earlier call already committed the operation.
    Existing(AuthorityReceipt),
    /// The operation identity was previously bound to different canonical bytes.
    OperationConflict { operation: OperationId },
    /// Current authority facts refuse the proposal.
    Rejected(CommitRefusal),
}

#[derive(Debug, Clone)]
struct StoredOperation {
    request_digest: CanonicalRequestDigest,
    receipt: AuthorityReceipt,
}

/// Inspectable reference state for one authority atom.
#[derive(Debug, Clone)]
pub struct AuthorityAtomSnapshot {
    atom: AuthorityAtomId,
    current_binding: CertifiedExecutionBinding,
    state_digest: ContentDigest,
    state_version: u64,
    authority_sequence: u64,
    operations: BTreeMap<OperationId, StoredOperation>,
    receipts: Vec<AuthorityReceipt>,
    transparency_outbox: BTreeMap<OperationId, TransparencyOutboxItem>,
}

impl AuthorityAtomSnapshot {
    /// Creates a version-zero atom with no receipts or pending publication.
    #[must_use]
    pub const fn genesis(
        atom: AuthorityAtomId,
        current_binding: CertifiedExecutionBinding,
        state_digest: ContentDigest,
    ) -> Self {
        Self {
            atom,
            current_binding,
            state_digest,
            state_version: 0,
            authority_sequence: 0,
            operations: BTreeMap::new(),
            receipts: Vec::new(),
            transparency_outbox: BTreeMap::new(),
        }
    }

    /// Returns the current state version.
    #[must_use]
    pub const fn state_version(&self) -> u64 {
        self.state_version
    }
    /// Returns the number of committed operations.
    #[must_use]
    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }
    /// Returns the number of immutable receipts.
    #[must_use]
    pub const fn receipt_count(&self) -> usize {
        self.receipts.len()
    }
    /// Returns the number of publication obligations.
    #[must_use]
    pub fn transparency_outbox_len(&self) -> usize {
        self.transparency_outbox
            .values()
            .filter(|item| !item.status.is_complete())
            .count()
    }
}

/// A provider adapter's asynchronous atom result.
pub type AuthorityAtomFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AuthorityAtomCommitResult, StorageFailure>> + Send + 'a>>;

/// Consumer-owned port for one exact atomic authority transition.
pub trait AuthorityAtomPort: Send + Sync {
    /// Rechecks authority facts and commits state, receipt, dedupe, and outbox together.
    fn commit<'a>(&'a self, request: &'a AuthorityAtomCommit) -> AuthorityAtomFuture<'a>;
}

/// Reference-only fault locations before the staged transaction becomes visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InMemoryCommitFault {
    /// Fail after staging canonical state.
    AfterState,
    /// Fail after staging the idempotency record.
    AfterOperation,
    /// Fail after staging the receipt.
    AfterReceipt,
    /// Fail after staging the transparency obligation.
    AfterOutbox,
}

impl InMemoryCommitFault {
    /// Every modeled internal write boundary.
    pub const ALL: [Self; 4] = [
        Self::AfterState,
        Self::AfterOperation,
        Self::AfterReceipt,
        Self::AfterOutbox,
    ];
}

/// Deterministic, transactional reference adapter for conformance tests.
pub struct InMemoryAuthorityAtom {
    state: Mutex<AuthorityAtomSnapshot>,
    next_fault: Mutex<Option<InMemoryCommitFault>>,
}

impl InMemoryAuthorityAtom {
    /// Opens an isolated reference atom.
    #[must_use]
    pub const fn new(snapshot: AuthorityAtomSnapshot) -> Self {
        Self {
            state: Mutex::new(snapshot),
            next_fault: Mutex::new(None),
        }
    }

    /// Injects one failure before the next staged commit publishes.
    pub fn fail_next_commit_at(&self, fault: InMemoryCommitFault) {
        *lock_recover(&self.next_fault) = Some(fault);
    }

    /// Returns an atomic clone of the current reference state.
    #[must_use]
    pub fn snapshot(&self) -> AuthorityAtomSnapshot {
        lock_recover(&self.state).clone()
    }

    fn commit_sync(
        &self,
        request: &AuthorityAtomCommit,
    ) -> Result<AuthorityAtomCommitResult, StorageFailure> {
        let mut current = self
            .state
            .lock()
            .map_err(|_| StorageFailure::RecoveryRequired)?;
        if let Some(stored) = current.operations.get(&request.operation) {
            return Ok(if stored.request_digest == request.request_digest {
                AuthorityAtomCommitResult::Existing(stored.receipt.clone())
            } else {
                AuthorityAtomCommitResult::OperationConflict {
                    operation: request.operation,
                }
            });
        }
        if request.atom != current.atom {
            return Ok(AuthorityAtomCommitResult::Rejected(
                CommitRefusal::WrongAuthorityAtom,
            ));
        }
        if request.binding.placement() != current.current_binding.placement() {
            return Ok(AuthorityAtomCommitResult::Rejected(
                CommitRefusal::StalePlacementFence,
            ));
        }
        if request.binding.grant() != current.current_binding.grant() {
            return Ok(AuthorityAtomCommitResult::Rejected(
                CommitRefusal::StaleGrantRevision,
            ));
        }
        if request.binding.revocation_view() != current.current_binding.revocation_view() {
            return Ok(AuthorityAtomCommitResult::Rejected(
                CommitRefusal::StaleRevocationCursor,
            ));
        }
        if request.expected_state_version != current.state_version {
            return Ok(AuthorityAtomCommitResult::Rejected(
                CommitRefusal::StaleStateVersion,
            ));
        }
        if binding_without_fences_differs(&request.binding, &current.current_binding) {
            return Ok(AuthorityAtomCommitResult::Rejected(
                CommitRefusal::CertifiedBindingMismatch,
            ));
        }

        let next_version = current
            .state_version
            .checked_add(1)
            .ok_or(StorageFailure::RecoveryRequired)?;
        let next_sequence_value = current
            .authority_sequence
            .checked_add(1)
            .ok_or(StorageFailure::RecoveryRequired)?;
        let next_sequence = AuthoritySequence::try_from(next_sequence_value)
            .map_err(|_| StorageFailure::RecoveryRequired)?;
        let fault = lock_recover(&self.next_fault).take();
        let mut staged = current.clone();

        staged.state_digest = request.state_after;
        staged.state_version = next_version;
        staged.authority_sequence = next_sequence_value;
        fail_if(fault, InMemoryCommitFault::AfterState)?;

        let receipt = AuthorityReceipt {
            operation: request.operation,
            principal: request.principal,
            request_digest: request.request_digest,
            binding: request.binding.clone(),
            state_version: next_version,
            authority_sequence: next_sequence,
            state_after: request.state_after,
            decision: request.decision,
        };
        staged.operations.insert(
            request.operation,
            StoredOperation {
                request_digest: request.request_digest,
                receipt: receipt.clone(),
            },
        );
        fail_if(fault, InMemoryCommitFault::AfterOperation)?;
        staged.receipts.push(receipt.clone());
        fail_if(fault, InMemoryCommitFault::AfterReceipt)?;
        staged.transparency_outbox.insert(
            request.operation,
            TransparencyOutboxItem::pending(request.transparency_commitment),
        );
        fail_if(fault, InMemoryCommitFault::AfterOutbox)?;

        *current = staged;
        drop(current);
        Ok(AuthorityAtomCommitResult::Committed(receipt))
    }
}

impl AuthorityAtomPort for InMemoryAuthorityAtom {
    fn commit<'a>(&'a self, request: &'a AuthorityAtomCommit) -> AuthorityAtomFuture<'a> {
        Box::pin(async move { self.commit_sync(request) })
    }
}

impl AuthorityOperationLookupPort for InMemoryAuthorityAtom {
    fn find_accepted_operation(
        &self,
        atom: AuthorityAtomId,
        operation: OperationId,
    ) -> AuthorityOperationLookupFuture<'_> {
        Box::pin(async move {
            let state = self
                .state
                .lock()
                .map_err(|_| StorageFailure::RecoveryRequired)?;
            if state.atom != atom {
                return Ok(None);
            }
            Ok(state
                .operations
                .get(&operation)
                .map(|stored| stored.receipt.clone()))
        })
    }
}

impl TransparencyOutboxPort for InMemoryAuthorityAtom {
    fn load_publication(&self, operation: OperationId) -> TransparencyLoadFuture<'_> {
        Box::pin(async move {
            let state = self
                .state
                .lock()
                .map_err(|_| StorageFailure::RecoveryRequired)?;
            Ok(state
                .transparency_outbox
                .get(&operation)
                .copied()
                .map(|item| item.view(operation)))
        })
    }

    fn record_publication(
        &self,
        operation: OperationId,
        step: PublicationStep,
    ) -> TransparencyOutboxFuture<'_> {
        Box::pin(async move {
            let mut state = self
                .state
                .lock()
                .map_err(|_| StorageFailure::RecoveryRequired)?;
            let Some(item) = state.transparency_outbox.get_mut(&operation) else {
                return Ok(PublicationOutcome::Refused(
                    super::PublicationRefusal::UnknownOperation,
                ));
            };
            let outcome = item.record(step);
            drop(state);
            Ok(outcome)
        })
    }
}

fn binding_without_fences_differs(
    left: &CertifiedExecutionBinding,
    right: &CertifiedExecutionBinding,
) -> bool {
    left.federation() != right.federation()
        || left.authority_epoch() != right.authority_epoch()
        || left.operator() != right.operator()
        || left.host() != right.host()
        || left.runtime_release() != right.runtime_release()
        || left.ruleset() != right.ruleset()
        || left.active_policy() != right.active_policy()
}

fn fail_if(
    selected: Option<InMemoryCommitFault>,
    current: InMemoryCommitFault,
) -> Result<(), StorageFailure> {
    if selected == Some(current) {
        Err(StorageFailure::Unavailable)
    } else {
        Ok(())
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
