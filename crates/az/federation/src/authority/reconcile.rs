//! Reconciliation of a commit whose outcome the caller never observed.
//!
//! Linear ADR 0051 owns generic durable operation dedupe and requires an
//! unknown outcome to reconcile its original operation before any repeat.
//! ADR 0053 keeps the meaning of an authority receipt here. This module is the
//! seam between them: federation reads the accepted record through a port it
//! owns, so no durable-substrate type, transaction, or cursor enters the
//! federation surface, and no generic durable receipt becomes an authority
//! receipt.

use std::{future::Future, pin::Pin};

use crate::{AuthorityAtomId, OperationId};

use super::{AuthorityAtomCommit, AuthorityReceipt, StorageFailure};

/// A lookup adapter's asynchronous accepted-operation result.
pub type AuthorityOperationLookupFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<AuthorityReceipt>, StorageFailure>> + Send + 'a>>;

/// Consumer-owned lookup of one previously accepted authority operation.
///
/// An adapter answers only for the atoms it owns. An operation belonging to
/// another atom reads as absent, because this port never guesses on behalf of
/// a different authority atom.
pub trait AuthorityOperationLookupPort: Send + Sync {
    /// Loads the immutable receipt an operation identity already produced.
    fn find_accepted_operation(
        &self,
        atom: AuthorityAtomId,
        operation: OperationId,
    ) -> AuthorityOperationLookupFuture<'_>;
}

/// What a caller learns about a commit whose result it never observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UncertainCommit {
    /// The original operation is accepted; its receipt is the only outcome.
    Accepted(Box<AuthorityReceipt>),
    /// The operation identity is already bound to different canonical bytes.
    Conflicting,
    /// No accepted record exists, so the original operation may be repeated.
    Unrecorded,
}

/// Resolves an ambiguous commit outcome against its original operation.
///
/// A caller that loses a commit response must call this with the exact
/// original request. Minting a new operation identity for the same intent is
/// how a duplicate portable effect is created, so this contract deliberately
/// offers no way to ask "did something like this commit".
///
/// # Errors
///
/// Returns the adapter's typed storage failure; an unavailable lookup leaves
/// the outcome unknown rather than reporting an absent record.
pub async fn reconcile_uncertain_commit(
    lookup: &dyn AuthorityOperationLookupPort,
    request: &AuthorityAtomCommit,
) -> Result<UncertainCommit, StorageFailure> {
    let accepted = lookup
        .find_accepted_operation(request.atom(), request.operation())
        .await?;
    Ok(match accepted {
        Some(receipt) if receipt.request_digest() == request.request_digest() => {
            UncertainCommit::Accepted(Box::new(receipt))
        }
        Some(_) => UncertainCommit::Conflicting,
        None => UncertainCommit::Unrecorded,
    })
}
