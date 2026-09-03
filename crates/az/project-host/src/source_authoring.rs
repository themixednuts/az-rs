//! Generic project-host sessions for codec-owned source documents.
//!
//! This module deliberately has no filesystem fallback. The injected client is
//! the sole route to the Asset Processor, which in turn owns worker codec
//! execution, compare-and-swap, and canonical reload.

use std::{collections::BTreeMap, fmt};

use az_proto_asset::{
    SourceFileEditDocument, SourceFileEditOperation, SourceFileEditRequest, SourceFileEditSnapshot,
    SourceFileOpenRequest, SourceFileRestoreRequest, WorkspaceSourceFileRef, asset_capnp,
};
use futures::{FutureExt, future::LocalBoxFuture};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SourceAuthoringClientError {
    #[error("source authoring is unavailable: {0}")]
    Unavailable(String),
    #[error("source authoring transaction failed: {0}")]
    Transaction(String),
}

/// The only boundary project-host needs from Asset Processor.
///
/// `restore` accepts a structured canonical document, never source bytes. The
/// eventual RPC adapter must forward it to the worker's codec restore path.
pub trait SourceAuthoringClient {
    fn open<'a>(
        &'a mut self,
        session_id: &'a str,
        source: &'a WorkspaceSourceFileRef,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>>;

    fn apply<'a>(
        &'a mut self,
        session_id: &'a str,
        source: &'a WorkspaceSourceFileRef,
        expected_source_fingerprint: &'a [u8],
        operation: SourceFileEditOperation,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>>;

    fn restore<'a>(
        &'a mut self,
        session_id: &'a str,
        source: &'a WorkspaceSourceFileRef,
        expected_source_fingerprint: &'a [u8],
        document: &'a SourceFileEditDocument,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>>;

    fn close<'a>(
        &'a mut self,
        _session_id: &'a str,
        _source: &'a WorkspaceSourceFileRef,
    ) -> LocalBoxFuture<'a, Result<(), SourceAuthoringClientError>> {
        async { Ok(()) }.boxed_local()
    }
}

impl<T: SourceAuthoringClient + ?Sized> SourceAuthoringClient for Box<T> {
    fn open<'a>(
        &'a mut self,
        s: &'a str,
        r: &'a WorkspaceSourceFileRef,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>> {
        (**self).open(s, r)
    }
    fn apply<'a>(
        &'a mut self,
        s: &'a str,
        r: &'a WorkspaceSourceFileRef,
        f: &'a [u8],
        o: SourceFileEditOperation,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>> {
        (**self).apply(s, r, f, o)
    }
    fn restore<'a>(
        &'a mut self,
        s: &'a str,
        r: &'a WorkspaceSourceFileRef,
        f: &'a [u8],
        d: &'a SourceFileEditDocument,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>> {
        (**self).restore(s, r, f, d)
    }
    fn close<'a>(
        &'a mut self,
        s: &'a str,
        r: &'a WorkspaceSourceFileRef,
    ) -> LocalBoxFuture<'a, Result<(), SourceAuthoringClientError>> {
        (**self).close(s, r)
    }
}

/// Test-only fail-closed client for harnesses that do not launch Asset Processor.
#[cfg(any(test, feature = "test-support"))]
pub struct UnavailableSourceAuthoringClient;

/// Asset Processor RPC adapter. Its capability is brokered to `ProjectHost` and
/// intentionally never comes from the editor request.
#[derive(Clone)]
pub struct SourceAuthoringRpcClient {
    client: asset_capnp::asset_processor::Client,
    capability: az_proto_core::Capability,
}

impl SourceAuthoringRpcClient {
    #[must_use]
    pub const fn new(
        client: asset_capnp::asset_processor::Client,
        capability: az_proto_core::Capability,
    ) -> Self {
        Self { client, capability }
    }
}

impl SourceAuthoringClient for SourceAuthoringRpcClient {
    fn open<'a>(
        &'a mut self,
        session_id: &'a str,
        source: &'a WorkspaceSourceFileRef,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>> {
        let client = self.client.clone();
        let request = SourceFileOpenRequest {
            capability: self.capability.clone(),
            session_id: session_id.into(),
            source: source.clone(),
        };
        async move {
            let mut call = client.open_source_file_request();
            request
                .to_capnp(call.get().init_request())
                .map_err(transaction)?;
            let response = call.send().promise.await.map_err(transaction)?;
            Ok(az_proto_asset::SourceFileOpenResult::from_capnp(
                response
                    .get()
                    .map_err(transaction)?
                    .get_result()
                    .map_err(transaction)?,
            )
            .map_err(transaction)?
            .snapshot)
        }
        .boxed_local()
    }

    fn apply<'a>(
        &'a mut self,
        session_id: &'a str,
        source: &'a WorkspaceSourceFileRef,
        expected_source_fingerprint: &'a [u8],
        operation: SourceFileEditOperation,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>> {
        let client = self.client.clone();
        let request = SourceFileEditRequest {
            capability: self.capability.clone(),
            session_id: session_id.into(),
            source: source.clone(),
            expected_source_fingerprint: expected_source_fingerprint.to_vec(),
            operation,
        };
        async move {
            let mut call = client.edit_source_file_request();
            request
                .to_capnp(call.get().init_request())
                .map_err(transaction)?;
            let response = call.send().promise.await.map_err(transaction)?;
            Ok(az_proto_asset::SourceFileEditResult::from_capnp(
                response
                    .get()
                    .map_err(transaction)?
                    .get_result()
                    .map_err(transaction)?,
            )
            .map_err(transaction)?
            .snapshot)
        }
        .boxed_local()
    }

    fn restore<'a>(
        &'a mut self,
        session_id: &'a str,
        source: &'a WorkspaceSourceFileRef,
        expected_source_fingerprint: &'a [u8],
        document: &'a SourceFileEditDocument,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>> {
        let client = self.client.clone();
        let request = SourceFileRestoreRequest {
            capability: self.capability.clone(),
            session_id: session_id.into(),
            source: source.clone(),
            expected_source_fingerprint: expected_source_fingerprint.to_vec(),
            document: document.clone(),
        };
        async move {
            let mut call = client.restore_source_file_request();
            request
                .to_capnp(call.get().init_request())
                .map_err(transaction)?;
            let response = call.send().promise.await.map_err(transaction)?;
            Ok(az_proto_asset::SourceFileRestoreResult::from_capnp(
                response
                    .get()
                    .map_err(transaction)?
                    .get_result()
                    .map_err(transaction)?,
            )
            .map_err(transaction)?
            .snapshot)
        }
        .boxed_local()
    }
}

fn transaction(error: impl std::fmt::Display) -> SourceAuthoringClientError {
    SourceAuthoringClientError::Transaction(error.to_string())
}

#[cfg(any(test, feature = "test-support"))]
impl SourceAuthoringClient for UnavailableSourceAuthoringClient {
    fn open<'a>(
        &'a mut self,
        _: &'a str,
        _: &'a WorkspaceSourceFileRef,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>> {
        async {
            Err(SourceAuthoringClientError::Unavailable(
                "Asset Processor client has not been injected".into(),
            ))
        }
        .boxed_local()
    }
    fn apply<'a>(
        &'a mut self,
        _: &'a str,
        _: &'a WorkspaceSourceFileRef,
        _: &'a [u8],
        _: SourceFileEditOperation,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>> {
        async {
            Err(SourceAuthoringClientError::Unavailable(
                "Asset Processor client has not been injected".into(),
            ))
        }
        .boxed_local()
    }
    fn restore<'a>(
        &'a mut self,
        _: &'a str,
        _: &'a WorkspaceSourceFileRef,
        _: &'a [u8],
        _: &'a SourceFileEditDocument,
    ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>> {
        async {
            Err(SourceAuthoringClientError::Unavailable(
                "Asset Processor client has not been injected".into(),
            ))
        }
        .boxed_local()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SourceAuthoringSessionError {
    #[error("source authoring session is not open")]
    NotOpen,
    #[error("source authoring revision conflict: expected {expected}, current {current}")]
    RevisionConflict { expected: u64, current: u64 },
    #[error("source authoring {direction} history is empty")]
    HistoryEmpty { direction: &'static str },
    #[error("Asset Processor returned a snapshot for a different source")]
    SourceMismatch,
    #[error(transparent)]
    Client(#[from] SourceAuthoringClientError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceAuthoringSessionStatus {
    pub open: bool,
    pub revision: u64,
    pub undo_depth: u32,
    pub redo_depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAuthoringSessionResult {
    pub status: SourceAuthoringSessionStatus,
    pub snapshot: Option<SourceFileEditSnapshot>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SessionKey {
    session_id: String,
    source_root_key: String,
    source_path: String,
    schema_type: String,
}

impl SessionKey {
    fn new(session_id: &str, source: &WorkspaceSourceFileRef) -> Self {
        Self {
            session_id: session_id.into(),
            source_root_key: source.source_root_key.clone(),
            source_path: source.source_path.clone(),
            schema_type: source.schema_type.clone(),
        }
    }
}

impl fmt::Debug for SessionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionKey")
            .field("session_id", &self.session_id)
            .field("source_root", &self.source_root_key)
            .field("source", &self.source_path)
            .field("schema", &self.schema_type)
            .finish()
    }
}

struct OpenSession {
    revision: u64,
    snapshot: SourceFileEditSnapshot,
    undo: Vec<SourceFileEditDocument>,
    redo: Vec<SourceFileEditDocument>,
}

/// One host-owned, source-schema-agnostic session service.
pub struct SourceAuthoringSessionService<C> {
    client: C,
    sessions: BTreeMap<SessionKey, OpenSession>,
}

impl<C: SourceAuthoringClient> SourceAuthoringSessionService<C> {
    #[must_use]
    pub const fn new(client: C) -> Self {
        Self {
            client,
            sessions: BTreeMap::new(),
        }
    }

    /// Open an Asset Processor editing session over `source`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceAuthoringSessionError::Client`] when the Asset Processor
    /// open call fails, or [`SourceAuthoringSessionError::SourceMismatch`] when
    /// the returned snapshot names a different source file than `source`.
    // Awaits the `LocalBoxFuture` the `SourceAuthoringClient` contract returns;
    // its production impl holds a capnp-rpc client, so this can never be `Send`
    // without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    pub async fn open(
        &mut self,
        session_id: &str,
        source: WorkspaceSourceFileRef,
    ) -> Result<SourceAuthoringSessionResult, SourceAuthoringSessionError> {
        let snapshot = self.client.open(session_id, &source).await?;
        ensure_source(&source, &snapshot)?;
        let key = SessionKey::new(session_id, &source);
        self.sessions.insert(
            key,
            OpenSession {
                revision: 0,
                snapshot: snapshot.clone(),
                undo: Vec::new(),
                redo: Vec::new(),
            },
        );
        Ok(result(
            &self.sessions[&SessionKey::new(session_id, &source)],
            Some(snapshot),
        ))
    }

    /// Apply one edit `operation` to the open session over `source`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceAuthoringSessionError::NotOpen`] when no session is open
    /// for `session_id` and `source`,
    /// [`SourceAuthoringSessionError::RevisionConflict`] when
    /// `expected_revision` is not the session's current revision,
    /// [`SourceAuthoringSessionError::Client`] when the Asset Processor edit
    /// call fails, and [`SourceAuthoringSessionError::SourceMismatch`] when the
    /// returned snapshot names a different source file.
    // Awaits the `LocalBoxFuture` the `SourceAuthoringClient` contract returns;
    // its production impl holds a capnp-rpc client, so this can never be `Send`
    // without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    pub async fn apply(
        &mut self,
        session_id: &str,
        source: WorkspaceSourceFileRef,
        expected_revision: u64,
        operation: SourceFileEditOperation,
    ) -> Result<SourceAuthoringSessionResult, SourceAuthoringSessionError> {
        let key = SessionKey::new(session_id, &source);
        let session = self
            .sessions
            .get_mut(&key)
            .ok_or(SourceAuthoringSessionError::NotOpen)?;
        check_revision(session, expected_revision)?;
        let snapshot = self
            .client
            .apply(
                session_id,
                &source,
                &session.snapshot.source_fingerprint,
                operation,
            )
            .await?;
        ensure_source(&source, &snapshot)?;
        session.undo.push(session.snapshot.document.clone());
        session.redo.clear();
        session.revision += 1;
        session.snapshot = snapshot.clone();
        Ok(result(session, Some(snapshot)))
    }

    /// Restore the most recent undo entry of the open session over `source`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::restore_history`] returns — including
    /// [`SourceAuthoringSessionError::HistoryEmpty`] with `direction: "undo"`
    /// when nothing has been applied yet.
    // Awaits the `LocalBoxFuture` the `SourceAuthoringClient` contract returns;
    // its production impl holds a capnp-rpc client, so this can never be `Send`
    // without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    pub async fn undo(
        &mut self,
        session_id: &str,
        source: WorkspaceSourceFileRef,
        expected_revision: u64,
    ) -> Result<SourceAuthoringSessionResult, SourceAuthoringSessionError> {
        self.restore_history(session_id, source, expected_revision, true)
            .await
    }
    /// Restore the most recent redo entry of the open session over `source`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::restore_history`] returns — including
    /// [`SourceAuthoringSessionError::HistoryEmpty`] with `direction: "redo"`
    /// when nothing has been undone yet.
    // Awaits the `LocalBoxFuture` the `SourceAuthoringClient` contract returns;
    // its production impl holds a capnp-rpc client, so this can never be `Send`
    // without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    pub async fn redo(
        &mut self,
        session_id: &str,
        source: WorkspaceSourceFileRef,
        expected_revision: u64,
    ) -> Result<SourceAuthoringSessionResult, SourceAuthoringSessionError> {
        self.restore_history(session_id, source, expected_revision, false)
            .await
    }

    /// Restore one history entry, `undo` selecting which stack it comes from.
    ///
    /// # Errors
    ///
    /// Returns [`SourceAuthoringSessionError::NotOpen`] when no session is open
    /// for `session_id` and `source`,
    /// [`SourceAuthoringSessionError::RevisionConflict`] when
    /// `expected_revision` is not the session's current revision,
    /// [`SourceAuthoringSessionError::HistoryEmpty`] when the selected stack is
    /// empty, [`SourceAuthoringSessionError::Client`] when the Asset Processor
    /// restore call fails, and
    /// [`SourceAuthoringSessionError::SourceMismatch`] when the returned
    /// snapshot names a different source file.
    ///
    /// # Panics
    ///
    /// Panics if the selected history entry disappeared while the restore RPC
    /// was in flight — it is read but deliberately left in place until the
    /// restore succeeds, and `&mut self` keeps anything else from popping it.
    // Awaits the `LocalBoxFuture` the `SourceAuthoringClient` contract returns;
    // its production impl holds a capnp-rpc client, so this can never be `Send`
    // without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    async fn restore_history(
        &mut self,
        session_id: &str,
        source: WorkspaceSourceFileRef,
        expected_revision: u64,
        undo: bool,
    ) -> Result<SourceAuthoringSessionResult, SourceAuthoringSessionError> {
        let key = SessionKey::new(session_id, &source);
        let session = self
            .sessions
            .get_mut(&key)
            .ok_or(SourceAuthoringSessionError::NotOpen)?;
        check_revision(session, expected_revision)?;
        // Keep the selected history entry in place while the worker restore is
        // in flight. An RPC future may be cancelled at any await point; popping
        // first would silently lose history even though no restore completed.
        let document = if undo {
            session.undo.last().cloned()
        } else {
            session.redo.last().cloned()
        }
        .ok_or(SourceAuthoringSessionError::HistoryEmpty {
            direction: if undo { "undo" } else { "redo" },
        })?;
        let snapshot = self
            .client
            .restore(
                session_id,
                &source,
                &session.snapshot.source_fingerprint,
                &document,
            )
            .await?;
        ensure_source(&source, &snapshot)?;
        if undo {
            let restored = session
                .undo
                .pop()
                .expect("undo history entry remains present until restore succeeds");
            debug_assert_eq!(restored, document);
            session.redo.push(session.snapshot.document.clone());
        } else {
            let restored = session
                .redo
                .pop()
                .expect("redo history entry remains present until restore succeeds");
            debug_assert_eq!(restored, document);
            session.undo.push(session.snapshot.document.clone());
        }
        session.revision += 1;
        session.snapshot = snapshot.clone();
        Ok(result(session, Some(snapshot)))
    }

    pub fn status(
        &self,
        session_id: &str,
        source: &WorkspaceSourceFileRef,
    ) -> SourceAuthoringSessionResult {
        self.sessions
            .get(&SessionKey::new(session_id, source))
            .map_or_else(
                || SourceAuthoringSessionResult {
                    status: SourceAuthoringSessionStatus {
                        open: false,
                        revision: 0,
                        undo_depth: 0,
                        redo_depth: 0,
                    },
                    snapshot: None,
                },
                |session| result(session, Some(session.snapshot.clone())),
            )
    }

    /// Retire the open session over `source` after the worker acknowledges it.
    ///
    /// # Errors
    ///
    /// Returns [`SourceAuthoringSessionError::NotOpen`] when no session is open
    /// for `session_id` and `source`,
    /// [`SourceAuthoringSessionError::RevisionConflict`] when
    /// `expected_revision` is stale — which is what keeps a late close from
    /// retiring a session that has advanced — or
    /// [`SourceAuthoringSessionError::Client`] when the Asset Processor close
    /// call fails.
    // Awaits the `LocalBoxFuture` the `SourceAuthoringClient` contract returns;
    // its production impl holds a capnp-rpc client, so this can never be `Send`
    // without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    pub async fn close(
        &mut self,
        session_id: &str,
        source: &WorkspaceSourceFileRef,
        expected_revision: u64,
    ) -> Result<SourceAuthoringSessionResult, SourceAuthoringSessionError> {
        let key = SessionKey::new(session_id, source);
        let session = self
            .sessions
            .get(&key)
            .ok_or(SourceAuthoringSessionError::NotOpen)?;
        check_revision(session, expected_revision)?;
        self.client.close(session_id, source).await?;
        self.sessions.remove(&key);
        Ok(self.status(session_id, source))
    }
}

fn ensure_source(
    expected: &WorkspaceSourceFileRef,
    snapshot: &SourceFileEditSnapshot,
) -> Result<(), SourceAuthoringSessionError> {
    if &snapshot.source == expected {
        Ok(())
    } else {
        Err(SourceAuthoringSessionError::SourceMismatch)
    }
}
const fn check_revision(
    session: &OpenSession,
    expected: u64,
) -> Result<(), SourceAuthoringSessionError> {
    if session.revision == expected {
        Ok(())
    } else {
        Err(SourceAuthoringSessionError::RevisionConflict {
            expected,
            current: session.revision,
        })
    }
}
fn result(
    session: &OpenSession,
    snapshot: Option<SourceFileEditSnapshot>,
) -> SourceAuthoringSessionResult {
    SourceAuthoringSessionResult {
        status: SourceAuthoringSessionStatus {
            open: true,
            revision: session.revision,
            undo_depth: session.undo.len().try_into().unwrap_or(u32::MAX),
            redo_depth: session.redo.len().try_into().unwrap_or(u32::MAX),
        },
        snapshot,
    }
}

#[cfg(test)]
mod tests {
    use std::task::{Context, Poll};

    use az_proto_asset::{ReflectedValueEnvelope, SourceFileEditObject};
    use futures::task::noop_waker_ref;

    use super::*;

    #[derive(Default)]
    struct FakeClient {
        pending_restore: bool,
    }

    impl SourceAuthoringClient for FakeClient {
        fn open<'a>(
            &'a mut self,
            _: &'a str,
            source: &'a WorkspaceSourceFileRef,
        ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>>
        {
            futures::future::ready(Ok(snapshot(source, "initial", 1))).boxed_local()
        }

        fn apply<'a>(
            &'a mut self,
            _: &'a str,
            source: &'a WorkspaceSourceFileRef,
            _: &'a [u8],
            _: SourceFileEditOperation,
        ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>>
        {
            futures::future::ready(Ok(snapshot(source, "edited", 2))).boxed_local()
        }

        fn restore<'a>(
            &'a mut self,
            _: &'a str,
            source: &'a WorkspaceSourceFileRef,
            _: &'a [u8],
            document: &'a SourceFileEditDocument,
        ) -> LocalBoxFuture<'a, Result<SourceFileEditSnapshot, SourceAuthoringClientError>>
        {
            if self.pending_restore {
                futures::future::pending().boxed_local()
            } else {
                futures::future::ready(Ok(SourceFileEditSnapshot {
                    source: source.clone(),
                    source_fingerprint: vec![3],
                    document: document.clone(),
                }))
                .boxed_local()
            }
        }
    }

    fn source() -> WorkspaceSourceFileRef {
        WorkspaceSourceFileRef {
            source_root_key: "project:test:assets".to_string(),
            source_path: "gamedata/test.ron".to_string(),
            schema_type: "game::GameDataTable".to_string(),
        }
    }

    fn document(label: &str) -> SourceFileEditDocument {
        let value =
            ReflectedValueEnvelope::typed_ron("game::GameDataTable", format!("(label: {label:?})"));
        SourceFileEditDocument {
            root_object_id: Some("root".to_string()),
            root_schema: "game::GameDataTable".to_string(),
            value: value.clone(),
            objects: vec![SourceFileEditObject {
                object_id: "root".to_string(),
                schema: "game::GameDataTable".to_string(),
                value,
            }],
            codec_state: Vec::new(),
        }
    }

    fn snapshot(
        source: &WorkspaceSourceFileRef,
        label: &str,
        fingerprint: u8,
    ) -> SourceFileEditSnapshot {
        SourceFileEditSnapshot {
            source: source.clone(),
            source_fingerprint: vec![fingerprint],
            document: document(label),
        }
    }

    #[test]
    fn cancelling_pending_restore_keeps_history_entry() {
        let source = source();
        let mut service = SourceAuthoringSessionService::new(FakeClient::default());
        futures::executor::block_on(async {
            service.open("editor", source.clone()).await.unwrap();
            service
                .apply(
                    "editor",
                    source.clone(),
                    0,
                    SourceFileEditOperation::AppendDefault,
                )
                .await
                .unwrap();
        });
        service.client.pending_restore = true;

        let mut undo = Box::pin(service.undo("editor", source.clone(), 1));
        let mut context = Context::from_waker(noop_waker_ref());
        assert!(matches!(undo.as_mut().poll(&mut context), Poll::Pending));
        drop(undo);

        let status = service.status("editor", &source);
        assert_eq!(status.status.revision, 1);
        assert_eq!(status.status.undo_depth, 1);
        assert_eq!(status.status.redo_depth, 0);
        assert_eq!(
            status.snapshot.unwrap().document,
            document("edited"),
            "cancellation must not publish a partial restore"
        );
    }

    #[test]
    fn stale_close_does_not_retire_newer_session() {
        let source = source();
        let mut service = SourceAuthoringSessionService::new(FakeClient::default());
        futures::executor::block_on(async {
            service.open("editor", source.clone()).await.unwrap();
            service
                .apply(
                    "editor",
                    source.clone(),
                    0,
                    SourceFileEditOperation::AppendDefault,
                )
                .await
                .unwrap();
            assert!(matches!(
                service.close("editor", &source, 0).await,
                Err(SourceAuthoringSessionError::RevisionConflict {
                    expected: 0,
                    current: 1,
                })
            ));
        });
        assert!(service.status("editor", &source).status.open);
    }
}
