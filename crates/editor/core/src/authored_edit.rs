//! Reflected Prefab editing through the ADR-0022 vNext project-host contract.
//!
//! Commands are created from inspector-projected named-path bindings and are
//! sent without an editor-owned value model. Source-session lifecycle owns
//! persistence and history.

use az_editor_inspector::ReflectedEditBinding;
#[cfg(not(test))]
use az_proto_core::ServiceDescriptor;
use az_proto_project::vnext::{
    PrefabDiagnostic, PrefabEditCommand, PrefabRpcResult, ReflectedValueEnvelope,
    SourceSessionCommand, SourceSessionResult, TypeRegistrySnapshot, TypedActionResult,
};
use tracing::{info, instrument};
#[cfg(not(test))]
use uuid::Uuid;

use crate::attach::EditorAttachSession;
#[cfg(not(test))]
use crate::error::EditorError;
use crate::error::EditorResult;
use crate::project_host::ProjectHostClient;
#[cfg(not(test))]
use crate::session_supervisor::SessionSupervisorClient;

/// One optimistic-concurrency edit against a typed Prefab source session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedPrefabEdit {
    pub source_path: String,
    pub expected_revision: u64,
    pub command: PrefabEditCommand,
}

impl ReflectedPrefabEdit {
    #[must_use]
    pub fn new(
        source_path: impl Into<String>,
        expected_revision: u64,
        command: PrefabEditCommand,
    ) -> Self {
        Self {
            source_path: source_path.into(),
            expected_revision,
            command,
        }
    }

    /// Builds a scalar or aggregate replacement from an inspector binding.
    #[must_use]
    pub fn set_value(
        source_path: impl Into<String>,
        expected_revision: u64,
        binding: &ReflectedEditBinding,
        value: ReflectedValueEnvelope,
    ) -> Self {
        Self::new(source_path, expected_revision, binding.set_value(value))
    }
}

/// Direction of project-host-owned source-session history traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflectedHistoryDirection {
    Undo,
    Redo,
}

impl ReflectedHistoryDirection {
    const fn command(self) -> SourceSessionCommand {
        match self {
            Self::Undo => SourceSessionCommand::Undo,
            Self::Redo => SourceSessionCommand::Redo,
        }
    }
}

/// Editor-side bridge to one attached session's vNext project-host routes.
///
/// Production calls resolve the current project-host descriptor for every
/// operation so a preserved editor session does not retain a stale IPC client.
/// Tests may inject an in-process client directly.
#[derive(Clone)]
pub struct ReflectedPrefabEditSession {
    #[cfg(test)]
    client: ProjectHostClient,
    #[cfg(not(test))]
    session_id: Option<Uuid>,
    #[cfg(not(test))]
    session_slug: Option<String>,
    #[cfg(not(test))]
    supervisor_descriptor: Option<ServiceDescriptor>,
}

impl ReflectedPrefabEditSession {
    #[cfg(test)]
    #[must_use]
    pub const fn new(client: ProjectHostClient) -> Self {
        Self {
            client,
            #[cfg(not(test))]
            session_id: None,
            #[cfg(not(test))]
            session_slug: None,
            #[cfg(not(test))]
            supervisor_descriptor: None,
        }
    }

    /// Creates a reconnecting vNext session bridge for an attached workspace.
    ///
    /// # Errors
    ///
    /// Returns any error [`ProjectHostClient::connect_for_session`] returns when
    /// the bridge eagerly dials an in-process client. The reconnecting
    /// production bridge only records `session`'s descriptors here and resolves
    /// the project host per operation instead, so it cannot fail at this point.
    #[instrument(
        skip(session),
        fields(session = %session.session_slug, session_id = %session.session_id)
    )]
    pub async fn connect_attached(session: &EditorAttachSession) -> EditorResult<Self> {
        #[cfg(test)]
        let client = ProjectHostClient::connect_for_session(
            &session.services.project_host,
            session.session_id,
        )
        .await?;

        let edit_session = Self {
            #[cfg(test)]
            client,
            #[cfg(not(test))]
            session_id: Some(session.session_id),
            #[cfg(not(test))]
            session_slug: Some(session.session_slug.clone()),
            #[cfg(not(test))]
            supervisor_descriptor: Some(session.session_supervisor.clone()),
        };
        info!(
            session = %session.session_slug,
            "configured reflected Prefab edit session"
        );
        Ok(edit_session)
    }

    /// Loads the authoritative reflected registry used by inspector projection.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::EditorError::ServiceDiscovery`] if the bridge
    /// carries no attached session supervisor descriptor, session id, or session
    /// slug, any error raised while reconnecting to the session supervisor and
    /// resolving the current project-host descriptor, or any error
    /// [`ProjectHostClient::type_registry_snapshot`] returns.
    #[instrument(skip(self))]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn type_registry_snapshot(&self) -> EditorResult<TypeRegistrySnapshot> {
        self.project_host_client()
            .await?
            .type_registry_snapshot()
            .await
    }

    /// Loads the current typed Prefab projection for `source_path`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::EditorError::ServiceDiscovery`] if the bridge
    /// carries no attached session supervisor descriptor, session id, or session
    /// slug, any error raised while reconnecting to the session supervisor and
    /// resolving the current project-host descriptor, or any error
    /// [`ProjectHostClient::prefab_source_snapshot`] returns for `source_path`.
    #[instrument(skip(self), fields(source_path))]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn source_snapshot(&self, source_path: &str) -> EditorResult<PrefabRpcResult> {
        self.project_host_client()
            .await?
            .prefab_source_snapshot(source_path)
            .await
    }

    /// Loads current validation diagnostics for a typed Prefab source.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::EditorError::ServiceDiscovery`] if the bridge
    /// carries no attached session supervisor descriptor, session id, or session
    /// slug, any error raised while reconnecting to the session supervisor and
    /// resolving the current project-host descriptor, or any error
    /// [`ProjectHostClient::prefab_diagnostics`] returns for `source_path`.
    #[instrument(skip(self), fields(source_path))]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn diagnostics(&self, source_path: &str) -> EditorResult<Vec<PrefabDiagnostic>> {
        self.project_host_client()
            .await?
            .prefab_diagnostics(source_path)
            .await
    }

    /// Applies one reflected structural command at an expected source revision.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::EditorError::ServiceDiscovery`] if the bridge
    /// carries no attached session supervisor descriptor, session id, or session
    /// slug, any error raised while reconnecting to the session supervisor and
    /// resolving the current project-host descriptor, or any error
    /// [`ProjectHostClient::apply_prefab_edit_command`] returns — including the
    /// optimistic-concurrency rejection when the source has moved past
    /// `edit.expected_revision`.
    #[instrument(
        skip(self, edit),
        fields(source_path = %edit.source_path, expected_revision = edit.expected_revision)
    )]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn apply(&self, edit: &ReflectedPrefabEdit) -> EditorResult<PrefabRpcResult> {
        let result = self
            .project_host_client()
            .await?
            .apply_prefab_edit_command(&edit.source_path, edit.expected_revision, &edit.command)
            .await?;
        info!(
            source_path = %edit.source_path,
            expected_revision = edit.expected_revision,
            snapshot_returned = result.snapshot.is_some(),
            diagnostic_count = result.diagnostics.len(),
            "applied reflected Prefab edit"
        );
        Ok(result)
    }

    /// Builds and applies a value replacement through the binding projected by
    /// the neutral inspector model.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::apply`] returns for the built
    /// [`PrefabEditCommand::SetValue`] command.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn set_value(
        &self,
        source_path: &str,
        expected_revision: u64,
        binding: &ReflectedEditBinding,
        value: ReflectedValueEnvelope,
    ) -> EditorResult<PrefabRpcResult> {
        self.apply(&ReflectedPrefabEdit::set_value(
            source_path,
            expected_revision,
            binding,
            value,
        ))
        .await
    }

    /// Invokes an action registered on the binding's reflected target.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::EditorError::ServiceDiscovery`] if the bridge
    /// carries no attached session supervisor descriptor, session id, or session
    /// slug, any error raised while reconnecting to the session supervisor and
    /// resolving the current project-host descriptor, or any error
    /// [`ProjectHostClient::invoke_typed_action`] returns — including an
    /// unregistered `action_id` and a stale `expected_revision`.
    #[instrument(skip(self, binding), fields(source_path, expected_revision, action_id))]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn invoke_action(
        &self,
        source_path: &str,
        expected_revision: u64,
        binding: &ReflectedEditBinding,
        action_id: &str,
    ) -> EditorResult<TypedActionResult> {
        self.project_host_client()
            .await?
            .invoke_typed_action(source_path, expected_revision, &binding.target, action_id)
            .await
    }

    /// Executes one source-session lifecycle transition.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::EditorError::ServiceDiscovery`] if the bridge
    /// carries no attached session supervisor descriptor, session id, or session
    /// slug, any error raised while reconnecting to the session supervisor and
    /// resolving the current project-host descriptor, or any error
    /// [`ProjectHostClient::source_session_lifecycle`] returns — including a
    /// `command` the source session refuses at `expected_revision`.
    #[instrument(
        skip(self),
        fields(source_path, ?command, expected_revision)
    )]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn lifecycle(
        &self,
        source_path: &str,
        command: SourceSessionCommand,
        expected_revision: u64,
    ) -> EditorResult<SourceSessionResult> {
        let result = self
            .project_host_client()
            .await?
            .source_session_lifecycle(source_path, command, expected_revision)
            .await?;
        info!(
            source_path,
            ?command,
            revision = result.status.revision,
            dirty = result.status.dirty,
            undo_depth = result.status.undo_depth,
            redo_depth = result.status.redo_depth,
            "completed reflected source-session lifecycle transition"
        );
        Ok(result)
    }

    /// Opens the typed source session for `source_path`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::lifecycle`] returns for
    /// [`SourceSessionCommand::Open`].
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn open(&self, source_path: &str) -> EditorResult<SourceSessionResult> {
        self.lifecycle(source_path, SourceSessionCommand::Open, 0)
            .await
    }

    /// Persists the source session for `source_path` at `expected_revision`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::lifecycle`] returns for
    /// [`SourceSessionCommand::Save`], including the rejection raised when the
    /// session has moved past `expected_revision`.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn save(
        &self,
        source_path: &str,
        expected_revision: u64,
    ) -> EditorResult<SourceSessionResult> {
        self.lifecycle(source_path, SourceSessionCommand::Save, expected_revision)
            .await
    }

    /// Traverses project-host-owned history in `direction`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::lifecycle`] returns for the lifecycle command
    /// `direction` maps to, including an exhausted undo or redo stack and a
    /// stale `expected_revision`.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn history(
        &self,
        source_path: &str,
        expected_revision: u64,
        direction: ReflectedHistoryDirection,
    ) -> EditorResult<SourceSessionResult> {
        self.lifecycle(source_path, direction.command(), expected_revision)
            .await
    }

    /// Steps the source session back one history entry.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::history`] returns for
    /// [`ReflectedHistoryDirection::Undo`].
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn undo(
        &self,
        source_path: &str,
        expected_revision: u64,
    ) -> EditorResult<SourceSessionResult> {
        self.history(
            source_path,
            expected_revision,
            ReflectedHistoryDirection::Undo,
        )
        .await
    }

    /// Steps the source session forward one history entry.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::history`] returns for
    /// [`ReflectedHistoryDirection::Redo`].
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn redo(
        &self,
        source_path: &str,
        expected_revision: u64,
    ) -> EditorResult<SourceSessionResult> {
        self.history(
            source_path,
            expected_revision,
            ReflectedHistoryDirection::Redo,
        )
        .await
    }

    // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    // The `cfg(test)` arm returns the injected client without awaiting; only
    // the production arm resolves a descriptor, and both callers await this.
    #[allow(clippy::future_not_send)]
    #[cfg_attr(test, allow(clippy::unused_async))]
    async fn project_host_client(&self) -> EditorResult<ProjectHostClient> {
        #[cfg(test)]
        {
            Ok(self.client.clone())
        }

        #[cfg(not(test))]
        {
            let supervisor_descriptor = self.supervisor_descriptor.as_ref().ok_or_else(|| {
                EditorError::ServiceDiscovery(
                    "reflected Prefab operations require an attached session supervisor".to_owned(),
                )
            })?;
            let session_id = self.session_id.ok_or_else(|| {
                EditorError::ServiceDiscovery(
                    "reflected Prefab operations require an attached session id".to_owned(),
                )
            })?;
            let session_slug = self.session_slug.as_deref().ok_or_else(|| {
                EditorError::ServiceDiscovery(
                    "reflected Prefab operations require an attached session slug".to_owned(),
                )
            })?;
            let supervisor =
                SessionSupervisorClient::connect_for_session(supervisor_descriptor, session_id)
                    .await?;
            let descriptor = supervisor
                .resolve_project_host_descriptor(session_slug)
                .await?;
            ProjectHostClient::connect_for_session(&descriptor, session_id).await
        }
    }
}

#[cfg(test)]
mod tests {
    use az_proto_project::vnext::{
        PrefabValueTarget, ReflectedPath, ReflectedPathSegment, ReflectedValueEncoding,
    };

    use super::*;

    fn binding() -> ReflectedEditBinding {
        ReflectedEditBinding::new(PrefabValueTarget {
            instance_alias_chain: Vec::new(),
            entity_alias: "root".to_owned(),
            path: ReflectedPath {
                component_type_path: "fixture::Transform".to_owned(),
                segments: Vec::new(),
            },
        })
        .field("translation")
        .field("x")
    }

    fn float(value: &str) -> ReflectedValueEnvelope {
        ReflectedValueEnvelope {
            type_path: "f32".to_owned(),
            encoding: ReflectedValueEncoding::TypedRon,
            payload: value.as_bytes().to_vec(),
        }
    }

    #[test]
    fn reflected_edit_uses_named_binding_and_typed_envelope() {
        let value = float("6.5");
        let edit =
            ReflectedPrefabEdit::set_value("levels/main.prefab.ron", 41, &binding(), value.clone());

        assert_eq!(edit.source_path, "levels/main.prefab.ron");
        assert_eq!(edit.expected_revision, 41);
        assert_eq!(
            edit.command,
            PrefabEditCommand::SetValue {
                target: PrefabValueTarget {
                    instance_alias_chain: Vec::new(),
                    entity_alias: "root".to_owned(),
                    path: ReflectedPath {
                        component_type_path: "fixture::Transform".to_owned(),
                        segments: vec![
                            ReflectedPathSegment::Field("translation".to_owned()),
                            ReflectedPathSegment::Field("x".to_owned()),
                        ],
                    },
                },
                value,
            }
        );
    }

    #[test]
    fn reflected_binding_builds_structural_collection_commands() {
        let list = binding().field("waypoints");
        let insert = ReflectedPrefabEdit::new(
            "levels/main.prefab.ron",
            9,
            list.list_insert(2, float("3.0")),
        );
        let moved = ReflectedPrefabEdit::new("levels/main.prefab.ron", 10, list.list_move(2, 0));

        assert!(matches!(
            insert.command,
            PrefabEditCommand::ListInsert { index: 2, .. }
        ));
        assert!(matches!(
            moved.command,
            PrefabEditCommand::ListMove { from: 2, to: 0, .. }
        ));
    }

    #[test]
    fn history_direction_maps_only_to_vnext_lifecycle_commands() {
        assert_eq!(
            ReflectedHistoryDirection::Undo.command(),
            SourceSessionCommand::Undo
        );
        assert_eq!(
            ReflectedHistoryDirection::Redo.command(),
            SourceSessionCommand::Redo
        );
    }
}
