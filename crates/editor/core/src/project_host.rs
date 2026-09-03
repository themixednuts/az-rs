//! Editor-side project-host client.
//!
//! The editor talks to project-host through the same Cap'n Proto interface
//! whether the host is in-process during tests or exported by the session
//! daemon over IPC.

use std::collections::BTreeSet;
use std::path::{Component, Path};

use az_node_graph::{GraphTypeCatalog, NodeSourceLink, NodeTypeCatalog};
use az_proto_core::{
    Capability, ServiceDescriptor, ServiceId, ServiceRole, SideChannelHandle,
    write_content_addressed_staging_file,
};
#[cfg(test)]
use az_proto_project::NodeSourceLinkPathKind;
use az_proto_project::vnext::{
    PrefabDiagnostic, PrefabEditCommand, PrefabRpcResult, PrefabValueTarget,
    SourceAuthoringSessionCommand, SourceAuthoringSessionResult, SourceSessionCommand,
    SourceSessionResult, TypeRegistrySnapshot, TypedActionResult, WorkspaceSourceFileRef,
};
use az_proto_project::{
    CreateGraphDocumentRequest, DocumentId, DocumentRevision, FromCapnp as _,
    GameDataCatalogSnapshot, GraphCommandBatchRequest, GraphCommandBatchSnapshot,
    GraphCommandStatusSnapshot, GraphDocumentSnapshot, NodeSourceLinkRequest, NodeSourceLinkTarget,
    PROJECT_DOCUMENT_READ_PERMISSION, PROJECT_DOCUMENT_WRITE_PERMISSION, PROJECT_EDIT_PERMISSION,
    PROJECT_GAMEDATA_PERMISSION, PROJECT_GRAPH_CATALOG_PERMISSION, PROJECT_HOST_AUDIENCE,
    PROJECT_INVENTORY_PERMISSION, PROJECT_NODE_CATALOG_PERMISSION,
    PROJECT_RUNTIME_LAUNCH_PERMISSION, PROJECT_SCHEMA_PERMISSION,
    PROJECT_SOURCE_NAVIGATION_PERMISSION, ProjectDocumentRequest, ProjectHostCapabilityRequest,
    ProjectInventoryReport, ProjectSideChannelResult, RuntimeLaunchSnapshotRequest,
    SaveDocumentResult, SavedDocument, ToCapnp as _, encode_graph_command_batch_snapshot,
    load_graph_command_status_side_channel, load_graph_document_side_channel, project_capnp,
};
use az_proto_runtime::{RuntimeAssetPackageRoot, RuntimeAssetSourceRoot, RuntimeRole};
use uuid::Uuid;

use crate::error::{EditorError, EditorResult};
use crate::graph_ui::{EditorGraphTypeCreationData, graph_document_id_from_creation_data};
use crate::service_descriptor::validate_descriptor_capability_templates;

pub const EDITOR_SERVICE_NAMESPACE: &str = "azoth";
pub const EDITOR_SERVICE_NAME: &str = "editor";
pub use az_proto_project::{
    PROJECT_HOST_NAMESPACE as PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME,
};

#[derive(Clone)]
pub struct ProjectHostClient {
    client: project_capnp::project_host::Client,
    editor_service: ServiceId,
    session_id: Option<Uuid>,
    capability_templates: Vec<Capability>,
    descriptor_capabilities_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRuntimeLaunchSnapshotContext {
    pub runtime_launch_capability: Capability,
    pub role: RuntimeRole,
    pub project_id: String,
    pub session_id: Uuid,
    pub session_slug: String,
    pub project_root: String,
    pub workspace_path: String,
    pub workspace_id: i64,
    pub include_unsaved_journal: bool,
    pub launch_profile: String,
    pub asset_source_roots: Vec<RuntimeAssetSourceRoot>,
    pub asset_package_roots: Vec<RuntimeAssetPackageRoot>,
}

impl ProjectHostClient {
    #[cfg(test)]
    #[must_use]
    pub fn new(client: project_capnp::project_host::Client) -> Self {
        Self {
            client,
            editor_service: ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            session_id: None,
            capability_templates: Vec::new(),
            descriptor_capabilities_required: false,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub const fn with_editor_service(
        client: project_capnp::project_host::Client,
        editor_service: ServiceId,
    ) -> Self {
        Self {
            client,
            editor_service,
            session_id: None,
            capability_templates: Vec::new(),
            descriptor_capabilities_required: false,
        }
    }

    #[must_use]
    pub(crate) const fn with_session_scope(mut self, session_id: Uuid) -> Self {
        self.session_id = Some(session_id);
        self
    }

    #[cfg(test)]
    #[must_use]
    pub fn with_capability_templates(mut self, capabilities: Vec<Capability>) -> Self {
        self.capability_templates = capabilities;
        self.descriptor_capabilities_required = true;
        self
    }

    /// Connects to a project-host descriptor and adopts its brokered capability
    /// templates.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if `descriptor` does not name
    /// the project-host service in the [`ServiceRole::ProjectHost`] role,
    /// advertises another protocol version, or carries capability templates
    /// that are not validly brokered, or [`EditorError::RpcTransport`] if the
    /// descriptor endpoint cannot be reached.
    #[cfg(test)]
    pub async fn connect(descriptor: &ServiceDescriptor) -> EditorResult<Self> {
        validate_project_host_descriptor(descriptor)?;
        let client = az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
        Ok(Self::from_descriptor_client(
            client,
            descriptor.capabilities.clone(),
        ))
    }

    /// Connects to a project-host descriptor and scopes every capability this
    /// client mints to the attached editor session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if `descriptor` fails the
    /// project-host identity, protocol-version, or brokered-template checks, or
    /// if `session_id` is nil, or [`EditorError::RpcTransport`] if the
    /// descriptor endpoint cannot be reached.
    pub async fn connect_for_session(
        descriptor: &ServiceDescriptor,
        session_id: Uuid,
    ) -> EditorResult<Self> {
        validate_project_host_descriptor_for_session(descriptor, session_id)?;
        let client = az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
        Ok(
            Self::from_descriptor_client(client, descriptor.capabilities.clone())
                .with_session_scope(session_id),
        )
    }

    #[must_use]
    pub const fn editor_service(&self) -> &ServiceId {
        &self.editor_service
    }

    /// Loads the authoritative ADR-0022 reflected type registry.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_SCHEMA_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if the capability cannot be encoded
    /// into the request, the call fails in flight, or the reply carries no
    /// decodable registry snapshot.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn type_registry_snapshot(&self) -> EditorResult<TypeRegistrySnapshot> {
        let mut request = self.client.type_registry_snapshot_request();
        self.editor_capability([PROJECT_SCHEMA_PERMISSION])?
            .to_capnp(request.get().init_capability())?;

        let response = request.send().promise.await?;
        Ok(az_proto_project::vnext::TypeRegistrySnapshot::from_capnp(
            response.get()?.get_snapshot()?,
        )?)
    }

    /// Reads an open typed Prefab source session through the vNext contract.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_DOCUMENT_READ_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if the call fails in flight or the
    /// reply carries no decodable Prefab result.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn prefab_source_snapshot(&self, source_path: &str) -> EditorResult<PrefabRpcResult> {
        let mut request = self.client.prefab_source_snapshot_request();
        {
            let mut params = request.get();
            self.editor_capability([PROJECT_DOCUMENT_READ_PERMISSION])?
                .to_capnp(params.reborrow().init_capability())?;
            params.set_source_path(source_path);
        }

        let response = request.send().promise.await?;
        Ok(az_proto_project::vnext::PrefabRpcResult::from_capnp(
            response.get()?.get_result()?,
        )?)
    }

    /// Applies one named-path structural edit to an open typed Prefab source.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_EDIT_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if `command` cannot be encoded into the
    /// request, the call fails in flight, or the reply carries no decodable
    /// Prefab result.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn apply_prefab_edit_command(
        &self,
        source_path: &str,
        expected_revision: u64,
        command: &PrefabEditCommand,
    ) -> EditorResult<PrefabRpcResult> {
        let mut request = self.client.apply_prefab_edit_command_request();
        {
            let mut params = request.get();
            self.editor_capability([PROJECT_EDIT_PERMISSION])?
                .to_capnp(params.reborrow().init_capability())?;
            params.set_source_path(source_path);
            params.set_expected_revision(expected_revision);
            (command).to_capnp(params.init_command())?;
        }

        let response = request.send().promise.await?;
        Ok(az_proto_project::vnext::PrefabRpcResult::from_capnp(
            response.get()?.get_result()?,
        )?)
    }

    /// Invokes a project-host registered typed editor action.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_EDIT_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if `target` cannot be encoded into the
    /// request, the call fails in flight, or the reply carries no decodable
    /// action result. An action project-host does not recognize comes back as a
    /// diagnostic on the result rather than an error.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn invoke_typed_action(
        &self,
        source_path: &str,
        expected_revision: u64,
        target: &PrefabValueTarget,
        action_id: &str,
    ) -> EditorResult<TypedActionResult> {
        let mut request = self.client.invoke_typed_action_request();
        {
            let mut params = request.get();
            self.editor_capability([PROJECT_EDIT_PERMISSION])?
                .to_capnp(params.reborrow().init_capability())?;
            params.set_source_path(source_path);
            params.set_expected_revision(expected_revision);
            (target).to_capnp(params.reborrow().init_target())?;
            params.set_action_id(action_id);
        }

        let response = request.send().promise.await?;
        Ok(az_proto_project::vnext::TypedActionResult::from_capnp(
            response.get()?.get_result()?,
        )?)
    }

    /// Evaluates validation registered for an open typed Prefab source.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_DOCUMENT_READ_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if the call fails in flight, the reply
    /// carries no diagnostic list, or any diagnostic in that list fails to
    /// decode.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn prefab_diagnostics(
        &self,
        source_path: &str,
    ) -> EditorResult<Vec<PrefabDiagnostic>> {
        let mut request = self.client.prefab_diagnostics_request();
        {
            let mut params = request.get();
            self.editor_capability([PROJECT_DOCUMENT_READ_PERMISSION])?
                .to_capnp(params.reborrow().init_capability())?;
            params.set_source_path(source_path);
        }

        let response = request.send().promise.await?;
        response
            .get()?
            .get_diagnostics()?
            .iter()
            .map(az_proto_project::vnext::PrefabDiagnostic::from_capnp)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Opens, queries, saves, traverses history, or closes a typed source
    /// session. Read-only lifecycle commands request read authority; mutating
    /// commands request document-write authority.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants the permission this `command` selects —
    /// [`PROJECT_DOCUMENT_READ_PERMISSION`] for `Open` and `Status`,
    /// [`PROJECT_DOCUMENT_WRITE_PERMISSION`] for every mutating command — or
    /// [`EditorError::ServiceProtocol`] if the call fails in flight or the
    /// reply carries no decodable session result.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn source_session_lifecycle(
        &self,
        source_path: &str,
        command: SourceSessionCommand,
        expected_revision: u64,
    ) -> EditorResult<SourceSessionResult> {
        let permission = if matches!(
            command,
            SourceSessionCommand::Open | SourceSessionCommand::Status
        ) {
            PROJECT_DOCUMENT_READ_PERMISSION
        } else {
            PROJECT_DOCUMENT_WRITE_PERMISSION
        };
        let mut request = self.client.source_session_lifecycle_request();
        {
            let mut params = request.get();
            self.editor_capability([permission])?
                .to_capnp(params.reborrow().init_capability())?;
            params.set_source_path(source_path);
            params.set_command((command).to_capnp());
            params.set_expected_revision(expected_revision);
        }

        let response = request.send().promise.await?;
        Ok(az_proto_project::vnext::SourceSessionResult::from_capnp(
            response.get()?.get_result()?,
        )?)
    }

    /// Routes generic codec-owned source authoring through project-host.
    /// The editor never contacts Asset Processor or selects codecs directly.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if this client was not scoped
    /// to an editor session, or if no descriptor capability grants the
    /// permission this `command` selects —
    /// [`PROJECT_DOCUMENT_READ_PERMISSION`] for `Open` and `Status`,
    /// [`PROJECT_DOCUMENT_WRITE_PERMISSION`] otherwise. Returns
    /// [`EditorError::ServiceProtocol`] if the request payload cannot be
    /// encoded, the call fails in flight, or the reply carries no decodable
    /// authoring result.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn source_authoring_session(
        &self,
        source: &WorkspaceSourceFileRef,
        expected_revision: u64,
        command: SourceAuthoringSessionCommand,
    ) -> EditorResult<SourceAuthoringSessionResult> {
        let session_id = self.session_id.ok_or_else(|| {
            EditorError::ServiceDiscovery("source authoring requires an editor session".to_owned())
        })?;
        let permission = if matches!(
            command,
            SourceAuthoringSessionCommand::Open | SourceAuthoringSessionCommand::Status
        ) {
            PROJECT_DOCUMENT_READ_PERMISSION
        } else {
            PROJECT_DOCUMENT_WRITE_PERMISSION
        };
        let mut request = self.client.source_authoring_session_request();
        let payload = az_proto_project::vnext::SourceAuthoringSessionRequest {
            capability: self.editor_capability([permission])?,
            session_id: session_id.to_string(),
            source: source.clone(),
            expected_revision,
            command,
        };
        payload.to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        az_proto_project::vnext::SourceAuthoringSessionResult::from_capnp(
            response.get()?.get_result()?,
        )
        .map_err(Into::into)
    }

    /// Requests the staging-file handle that carries the `GameData` catalog.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_GAMEDATA_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if the call fails in flight, the reply
    /// carries no decodable handle, or the handle project-host returns is not
    /// bound to the capability this client sent.
    // TODO(rip): remove when the GameData panel migrates.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn gamedata_catalog_handle(&self) -> EditorResult<SideChannelHandle> {
        let mut request = self.client.gamedata_catalog_request();
        let capability = self.editor_capability([PROJECT_GAMEDATA_PERMISSION])?;
        ProjectHostCapabilityRequest {
            capability: capability.clone(),
        }
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        Ok(ProjectSideChannelResult::from_capnp((response.get()?, &capability))?.snapshot)
    }

    /// Reads the `GameData` catalog from the side channel project-host staged.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::gamedata_catalog_handle`] returns, or
    /// [`EditorError::ProjectHostGameDataCatalogSideChannel`] if the staged
    /// file is missing, fails its content-hash check, or does not decode as a
    /// catalog snapshot.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn load_gamedata_catalog(&self) -> EditorResult<GameDataCatalogSnapshot> {
        let handle = self.gamedata_catalog_handle().await?;
        Ok(az_proto_project::load_gamedata_catalog_side_channel(
            &handle,
        )?)
    }

    /// Requests the staging-file handle that carries the node type catalog.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_NODE_CATALOG_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if the call fails in flight, the reply
    /// carries no decodable handle, or the handle project-host returns is not
    /// bound to the capability this client sent.
    // TODO(rip): remove when the graph panel migrates.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn node_type_catalog_handle(&self) -> EditorResult<SideChannelHandle> {
        let mut request = self.client.node_type_catalog_request();
        let capability = self.editor_capability([PROJECT_NODE_CATALOG_PERMISSION])?;
        ProjectHostCapabilityRequest {
            capability: capability.clone(),
        }
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        Ok(ProjectSideChannelResult::from_capnp((response.get()?, &capability))?.snapshot)
    }

    /// Reads the node type catalog from the side channel project-host staged.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::node_type_catalog_handle`] returns, or
    /// [`EditorError::ProjectHostNodeTypeCatalogSideChannel`] if the staged
    /// file is missing, fails its content-hash check, or does not decode as a
    /// node type catalog.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn load_node_type_catalog(&self) -> EditorResult<NodeTypeCatalog> {
        let handle = self.node_type_catalog_handle().await?;
        Ok(az_proto_project::load_node_type_catalog_side_channel(
            &handle,
        )?)
    }

    /// Requests the staging-file handle that carries the graph type catalog.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_GRAPH_CATALOG_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if the call fails in flight, the reply
    /// carries no decodable handle, or the handle project-host returns is not
    /// bound to the capability this client sent.
    // TODO(rip): remove when the graph panel migrates.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn graph_type_catalog_handle(&self) -> EditorResult<SideChannelHandle> {
        let mut request = self.client.graph_type_catalog_request();
        let capability = self.editor_capability([PROJECT_GRAPH_CATALOG_PERMISSION])?;
        ProjectHostCapabilityRequest {
            capability: capability.clone(),
        }
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        Ok(ProjectSideChannelResult::from_capnp((response.get()?, &capability))?.snapshot)
    }

    /// Reads the graph type catalog from the side channel project-host staged.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::graph_type_catalog_handle`] returns, or
    /// [`EditorError::ProjectHostGraphTypeCatalogSideChannel`] if the staged
    /// file is missing, fails its content-hash check, or does not decode as a
    /// graph type catalog.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn load_graph_type_catalog(&self) -> EditorResult<GraphTypeCatalog> {
        let handle = self.graph_type_catalog_handle().await?;
        Ok(az_proto_project::load_graph_type_catalog_side_channel(
            &handle,
        )?)
    }

    /// Reads project-host's inventory of registered project contents.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_INVENTORY_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if the call fails in flight or the
    /// reply does not decode as an inventory report.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn project_inventory(&self) -> EditorResult<ProjectInventoryReport> {
        let mut request = self.client.project_inventory_request();
        let capability = self.editor_capability([PROJECT_INVENTORY_PERMISSION])?;
        ProjectHostCapabilityRequest {
            capability: capability.clone(),
        }
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        Ok(ProjectInventoryReport::from_capnp(response.get()?)?)
    }

    /// Resolves a node's source link to a navigable target on disk.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_SOURCE_NAVIGATION_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if `source_link` cannot be encoded into
    /// the request, the call fails in flight, or the reply does not decode as a
    /// navigation target.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn resolve_node_source_link(
        &self,
        source_link: NodeSourceLink,
    ) -> EditorResult<NodeSourceLinkTarget> {
        let capability = self.editor_capability([PROJECT_SOURCE_NAVIGATION_PERMISSION])?;
        let mut request = self.client.resolve_node_source_link_request();
        NodeSourceLinkRequest {
            capability,
            source_link,
        }
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        Ok(NodeSourceLinkTarget::from_capnp(response.get()?)?)
    }

    /// Stages a graph command batch and applies it through project-host.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_EDIT_PERMISSION`];
    /// [`EditorError::ServiceProtocol`] if `batch` fails transport validation
    /// or encoding, the call fails in flight, or the reply carries no handle
    /// bound to the capability this client sent;
    /// [`EditorError::GraphCommandBatchSideChannelWrite`] if the encoded batch
    /// cannot be written under `side_channel_root`;
    /// [`EditorError::ProjectHostGraphCommandStatusSideChannel`] if the status
    /// project-host staged cannot be read back; and
    /// [`EditorError::ServiceAuthorityMismatch`] if that status carries a
    /// document id that is not project-relative, names another document or
    /// client batch, reports applying more commands than `batch` contains, or
    /// accepts a revision that does not advance past the batch's expected
    /// revision.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn apply_graph_commands(
        &self,
        batch: &GraphCommandBatchSnapshot,
        side_channel_root: &Path,
    ) -> EditorResult<GraphCommandStatusSnapshot> {
        let capability = self.editor_capability([PROJECT_EDIT_PERMISSION])?;
        let bytes = encode_graph_command_batch_snapshot(batch)?;
        let written =
            write_content_addressed_staging_file(side_channel_root, "graph-command-batch", &bytes)
                .map_err(|error| EditorError::GraphCommandBatchSideChannelWrite {
                    path: error.path,
                    source: error.source,
                })?;
        let batch_handle = SideChannelHandle::staging_file(
            written.path.to_string_lossy(),
            written.byte_length,
            written.content_hash,
            std::env::consts::OS,
        );

        let mut request = self.client.apply_graph_commands_request();
        GraphCommandBatchRequest {
            capability: capability.clone(),
            batch: batch_handle,
        }
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        let status_handle =
            ProjectSideChannelResult::from_capnp((response.get()?, &capability))?.snapshot;
        let status = load_graph_command_status_side_channel(&status_handle)?;
        ensure_graph_command_status_matches_request(&status, batch)?;
        Ok(status)
    }

    /// Creates an empty graph document of `graph_type` at `document_id`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_DOCUMENT_WRITE_PERMISSION`];
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply carries no handle bound to the
    /// capability this client sent;
    /// [`EditorError::ProjectHostGraphDocumentSideChannel`] if the staged
    /// snapshot cannot be read back; and
    /// [`EditorError::ServiceAuthorityMismatch`] if that snapshot names another
    /// document or graph type, carries a document id that is not
    /// project-relative, has a zero document version, repeats a node,
    /// connection, route anchor, or comment id, carries non-finite layout,
    /// anchor, or comment geometry, or is not at initial revision 0.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn create_graph_document(
        &self,
        document_id: &DocumentId,
        graph_type: &str,
    ) -> EditorResult<GraphDocumentSnapshot> {
        let capability = self.editor_capability([PROJECT_DOCUMENT_WRITE_PERMISSION])?;
        let mut request = self.client.create_graph_document_request();
        CreateGraphDocumentRequest {
            capability: capability.clone(),
            document_id: document_id.clone(),
            graph_type: graph_type.to_string(),
        }
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        let snapshot_handle =
            ProjectSideChannelResult::from_capnp((response.get()?, &capability))?.snapshot;
        let snapshot = load_graph_document_side_channel(&snapshot_handle)?;
        ensure_graph_document_snapshot_matches_request(
            &snapshot,
            document_id,
            "createGraphDocument",
        )?;
        ensure_created_graph_document_snapshot_matches_request(&snapshot, graph_type)?;
        Ok(snapshot)
    }

    /// Derives the document id from a creation-catalog entry, then creates the
    /// graph document.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::GraphDocumentCreation`] if `document_name` is
    /// empty, is a path traversal, or otherwise does not yield a valid document
    /// id for `graph_type`, and otherwise any error
    /// [`Self::create_graph_document`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn create_graph_document_from_creation_data(
        &self,
        graph_type: &EditorGraphTypeCreationData,
        document_name: &str,
    ) -> EditorResult<GraphDocumentSnapshot> {
        let document_id = graph_document_id_from_creation_data(graph_type, document_name)?;
        self.create_graph_document(&document_id, &graph_type.graph_type)
            .await
    }

    /// Reads the current snapshot of one graph document.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_DOCUMENT_READ_PERMISSION`];
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply carries no handle bound to the
    /// capability this client sent;
    /// [`EditorError::ProjectHostGraphDocumentSideChannel`] if the staged
    /// snapshot cannot be read back; and
    /// [`EditorError::ServiceAuthorityMismatch`] if that snapshot names another
    /// document, carries a document id that is not project-relative, has a zero
    /// document version or empty graph type, repeats a node, connection, route
    /// anchor, or comment id, or carries non-finite layout, anchor, or comment
    /// geometry.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn graph_document_snapshot(
        &self,
        document_id: &DocumentId,
    ) -> EditorResult<GraphDocumentSnapshot> {
        let capability = self.editor_capability([PROJECT_DOCUMENT_READ_PERMISSION])?;
        let mut request = self.client.graph_document_snapshot_request();
        ProjectDocumentRequest {
            capability: capability.clone(),
            document_id: document_id.clone(),
        }
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        let snapshot_handle =
            ProjectSideChannelResult::from_capnp((response.get()?, &capability))?.snapshot;
        let snapshot = load_graph_document_side_channel(&snapshot_handle)?;
        ensure_graph_document_snapshot_matches_request(
            &snapshot,
            document_id,
            "graphDocumentSnapshot",
        )?;
        Ok(snapshot)
    }

    /// Saves a graph document and reports the revision it was written at.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::save_graph_document_record`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn save_graph_document(
        &self,
        document_id: &DocumentId,
    ) -> EditorResult<DocumentRevision> {
        Ok(self.save_graph_document_record(document_id).await?.revision)
    }

    /// Saves a graph document and returns the full record project-host wrote.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_DOCUMENT_WRITE_PERMISSION`];
    /// [`EditorError::ServiceProtocol`] if the request cannot be encoded, the
    /// call fails in flight, or the reply does not decode as a save result; and
    /// [`EditorError::ServiceAuthorityMismatch`] if the saved record names
    /// another document, carries a document id that is not project-relative,
    /// reports a source path that differs from the requested document, or
    /// carries a content hash that is not 32 bytes.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn save_graph_document_record(
        &self,
        document_id: &DocumentId,
    ) -> EditorResult<SavedDocument> {
        let capability = self.editor_capability([PROJECT_DOCUMENT_WRITE_PERMISSION])?;
        let mut request = self.client.save_graph_document_request();
        ProjectDocumentRequest {
            capability,
            document_id: document_id.clone(),
        }
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        let saved = SaveDocumentResult::from_capnp(response.get()?)?.saved;
        ensure_saved_document_matches_request(&saved, document_id, "saveGraphDocument")?;
        Ok(saved)
    }

    /// Requests the staging-file handle carrying a runtime launch snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if no descriptor capability
    /// grants [`PROJECT_RUNTIME_LAUNCH_PERMISSION`], or
    /// [`EditorError::ServiceProtocol`] if `context` cannot be encoded into the
    /// request, the call fails in flight, or the reply carries no handle bound
    /// to the runtime-launch capability carried in `context`.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn runtime_launch_snapshot(
        &self,
        context: ProjectRuntimeLaunchSnapshotContext,
    ) -> EditorResult<SideChannelHandle> {
        let mut request = self.client.runtime_launch_snapshot_request();
        let editor_capability = self.editor_capability([PROJECT_RUNTIME_LAUNCH_PERMISSION])?;
        let runtime_launch_capability = context.runtime_launch_capability.clone();
        RuntimeLaunchSnapshotRequest {
            capability: editor_capability,
            runtime_launch_capability: context.runtime_launch_capability,
            role: context.role,
            project_id: context.project_id,
            session_id: context.session_id,
            session_slug: context.session_slug,
            project_root: context.project_root,
            workspace_path: context.workspace_path,
            workspace_id: context.workspace_id,
            include_unsaved_journal: context.include_unsaved_journal,
            launch_profile: context.launch_profile,
            asset_source_roots: context.asset_source_roots,
            asset_package_roots: context.asset_package_roots,
        }
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        Ok(
            ProjectSideChannelResult::from_capnp((response.get()?, &runtime_launch_capability))?
                .snapshot,
        )
    }

    fn editor_capability(
        &self,
        permissions: impl IntoIterator<Item = &'static str>,
    ) -> EditorResult<Capability> {
        let permissions: Vec<&'static str> = permissions.into_iter().collect();
        if self.descriptor_capabilities_required {
            return self
                .capability_template(&permissions)
                .ok_or_else(|| Self::missing_capability_error(&permissions));
        }

        let capability = Capability::new(self.editor_service.clone(), ServiceRole::Editor);
        let capability = match self.session_id {
            Some(session_id) => capability.with_session(session_id),
            None => capability,
        };
        Ok(capability
            .with_audience(PROJECT_HOST_AUDIENCE)
            .with_permissions(permissions))
    }

    fn capability_template(&self, permissions: &[&str]) -> Option<Capability> {
        self.capability_templates
            .iter()
            .find(|capability| {
                capability.matches_brokered_template_request(
                    ServiceRole::Editor,
                    PROJECT_HOST_AUDIENCE,
                    permissions,
                    self.session_id,
                )
            })
            .cloned()
            .map(|capability| capability.scoped_to(self.session_id))
    }

    fn missing_capability_error(permissions: &[&str]) -> crate::error::EditorError {
        crate::error::EditorError::ServiceDiscovery(format!(
            "project-host descriptor did not grant editor capability `{}`",
            permissions.join(", ")
        ))
    }

    #[must_use]
    fn from_descriptor_client(
        client: project_capnp::project_host::Client,
        capability_templates: Vec<Capability>,
    ) -> Self {
        Self {
            client,
            editor_service: ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            session_id: None,
            capability_templates,
            descriptor_capabilities_required: true,
        }
    }
}

fn ensure_graph_command_status_matches_request(
    status: &GraphCommandStatusSnapshot,
    batch: &GraphCommandBatchSnapshot,
) -> EditorResult<()> {
    ensure_project_relative_document_id(&status.document_id, "applyGraphCommands")?;
    if status.document_id != batch.document_id {
        return Err(project_host_authority_mismatch(
            "applyGraphCommands",
            format!(
                "graph command status document `{}` does not match requested document `{}`",
                status.document_id.as_str(),
                batch.document_id.as_str()
            ),
        ));
    }
    if status.client_batch_id != batch.client_batch_id {
        return Err(project_host_authority_mismatch(
            "applyGraphCommands",
            format!(
                "graph command status batch `{}` does not match requested batch `{}`",
                status.client_batch_id, batch.client_batch_id
            ),
        ));
    }
    if usize::try_from(status.applied_command_count).is_ok_and(|count| count > batch.commands.len())
    {
        return Err(project_host_authority_mismatch(
            "applyGraphCommands",
            format!(
                "graph command status applied {} commands from a {} command batch",
                status.applied_command_count,
                batch.commands.len()
            ),
        ));
    }
    if let az_proto_project::GraphCommandStatusOutcome::Accepted { revision } = &status.outcome
        && let Some(expected_revision) = batch.expected_revision
        && *revision <= expected_revision
    {
        return Err(project_host_authority_mismatch(
            "applyGraphCommands",
            format!(
                "graph command accepted revision {} did not advance past expected revision {}",
                revision.0, expected_revision.0
            ),
        ));
    }
    Ok(())
}

fn ensure_graph_document_snapshot_matches_request(
    snapshot: &GraphDocumentSnapshot,
    document_id: &DocumentId,
    operation: &'static str,
) -> EditorResult<()> {
    ensure_project_relative_document_id(&snapshot.document_id, operation)?;
    if &snapshot.document_id != document_id {
        return Err(project_host_authority_mismatch(
            operation,
            format!(
                "graph document snapshot `{}` does not match requested document `{}`",
                snapshot.document_id.as_str(),
                document_id.as_str()
            ),
        ));
    }
    if snapshot.document.document_version == 0 {
        return Err(project_host_authority_mismatch(
            operation,
            "graph document version cannot be zero".to_string(),
        ));
    }
    if snapshot.document.graph_type.trim().is_empty() {
        return Err(project_host_authority_mismatch(
            operation,
            "graph document type cannot be empty".to_string(),
        ));
    }

    let mut node_ids = BTreeSet::new();
    for node in &snapshot.document.nodes {
        if !node_ids.insert(node.id) {
            return Err(project_host_authority_mismatch(
                operation,
                format!("duplicate graph node `{}`", node.id),
            ));
        }
        if !node.layout.x.is_finite() || !node.layout.y.is_finite() {
            return Err(project_host_authority_mismatch(
                operation,
                format!("graph node `{}` has non-finite layout", node.id),
            ));
        }
    }

    let mut connection_ids = BTreeSet::new();
    for connection in &snapshot.document.connections {
        if !connection_ids.insert(connection.id) {
            return Err(project_host_authority_mismatch(
                operation,
                format!("duplicate graph connection `{}`", connection.id),
            ));
        }
        let mut anchor_ids = BTreeSet::new();
        for anchor in &connection.route.anchors {
            if !anchor_ids.insert(anchor.id) {
                return Err(project_host_authority_mismatch(
                    operation,
                    format!("duplicate route anchor `{}`", anchor.id),
                ));
            }
            if !anchor.position.x.is_finite() || !anchor.position.y.is_finite() {
                return Err(project_host_authority_mismatch(
                    operation,
                    format!("route anchor `{}` has non-finite position", anchor.id),
                ));
            }
        }
    }

    let mut comment_ids = BTreeSet::new();
    for comment in &snapshot.document.comments {
        if !comment_ids.insert(comment.id) {
            return Err(project_host_authority_mismatch(
                operation,
                format!("duplicate graph comment `{}`", comment.id),
            ));
        }
        if !comment.bounds.x.is_finite()
            || !comment.bounds.y.is_finite()
            || !comment.bounds.width.is_finite()
            || !comment.bounds.height.is_finite()
        {
            return Err(project_host_authority_mismatch(
                operation,
                format!("graph comment `{}` has non-finite bounds", comment.id),
            ));
        }
    }

    Ok(())
}

fn ensure_created_graph_document_snapshot_matches_request(
    snapshot: &GraphDocumentSnapshot,
    graph_type: &str,
) -> EditorResult<()> {
    if snapshot.document.graph_type != graph_type {
        return Err(project_host_authority_mismatch(
            "createGraphDocument",
            format!(
                "graph document type `{}` does not match requested type `{graph_type}`",
                snapshot.document.graph_type
            ),
        ));
    }
    if snapshot.revision != DocumentRevision::new(0) {
        return Err(project_host_authority_mismatch(
            "createGraphDocument",
            format!(
                "new graph document snapshot must be initial revision 0, got {}",
                snapshot.revision.0
            ),
        ));
    }
    Ok(())
}

fn ensure_saved_document_matches_request(
    saved: &SavedDocument,
    requested_document: &DocumentId,
    operation: &'static str,
) -> EditorResult<()> {
    ensure_project_relative_document_id(&saved.document_id, operation)?;
    if &saved.document_id != requested_document {
        return Err(project_host_authority_mismatch(
            operation,
            format!(
                "saved document `{}` does not match requested document `{}`",
                saved.document_id.as_str(),
                requested_document.as_str()
            ),
        ));
    }

    if saved.source_path != requested_document.as_str() {
        return Err(project_host_authority_mismatch(
            operation,
            format!(
                "saved source path `{}` does not match requested document `{}`",
                saved.source_path,
                requested_document.as_str()
            ),
        ));
    }

    if saved.content_hash.len() != 32 {
        return Err(project_host_authority_mismatch(
            operation,
            format!(
                "saved content hash has {} bytes, expected 32",
                saved.content_hash.len()
            ),
        ));
    }

    Ok(())
}

fn ensure_project_relative_document_id(
    document_id: &DocumentId,
    operation: &'static str,
) -> EditorResult<()> {
    let value = document_id.as_str();
    if value.trim().is_empty() {
        return Err(project_host_authority_mismatch(
            operation,
            "document id cannot be empty".to_string(),
        ));
    }

    let path = Path::new(value);
    let mut has_normal_component = false;
    for component in path.components() {
        match component {
            Component::Normal(segment) if !segment.to_string_lossy().trim().is_empty() => {
                has_normal_component = true;
            }
            Component::Normal(_)
            | Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(project_host_authority_mismatch(
                    operation,
                    format!("document id `{value}` must be a project-relative source path"),
                ));
            }
        }
    }

    if !has_normal_component {
        return Err(project_host_authority_mismatch(
            operation,
            format!("document id `{value}` must include a source path component"),
        ));
    }

    Ok(())
}

const fn project_host_authority_mismatch(operation: &'static str, reason: String) -> EditorError {
    EditorError::ServiceAuthorityMismatch {
        service: "project-host",
        operation,
        reason,
    }
}

fn validate_project_host_descriptor(descriptor: &ServiceDescriptor) -> EditorResult<()> {
    let expected = ServiceId::new(PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME);
    if descriptor.id != expected || descriptor.role != ServiceRole::ProjectHost {
        return Err(crate::error::EditorError::ServiceDiscovery(format!(
            "expected project-host descriptor `{}`/`{}` with role {:?}, got `{}`/`{}` with role {:?}",
            expected.namespace,
            expected.name,
            ServiceRole::ProjectHost,
            descriptor.id.namespace,
            descriptor.id.name,
            descriptor.role
        )));
    }
    validate_descriptor_capability_templates(descriptor, "project-host")?;
    Ok(())
}

fn validate_project_host_descriptor_for_session(
    descriptor: &ServiceDescriptor,
    session_id: Uuid,
) -> EditorResult<()> {
    validate_project_host_descriptor(descriptor)?;
    // Project-host capability templates ship unscoped (see
    // `project_host_service_descriptor` in service-catalog); the session is
    // bound when capabilities are minted from those templates, not on the
    // templates themselves. Only assert that the attach session is present.
    if session_id.is_nil() {
        return Err(EditorError::ServiceDiscovery(
            "project-host descriptor session id must not be nil".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) use self::tests::{test_project_host_client, test_project_host_client_at};

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, declare_caps,
    };
    use az_node_graph::{
        GraphCommand, GraphCompilerBackendDescriptor, GraphNode, GraphNodeCatalogRequirement,
        GraphNodeId, GraphNodeLayout, GraphSourceWorkflow, GraphTypeDescriptor,
        GraphTypeRegistration, NodePortDescriptor, NodePortDirection, NodePortId, NodePortValue,
        NodeTypeDescriptor, NodeTypeRegistration, RuntimeGraphExecutionStrategy,
        RuntimeGraphProductDescriptor,
    };
    use az_project::{ProjectManifest, refresh_project_lock, write_project_manifest};
    use az_project_host::{
        Composition, ProjectHost, ProjectHostRpc,
        start_project_host_rpc_server_with_capability_grants,
    };
    use az_proto_core::{CapabilityGrantSet, Endpoint, EndpointKind, ServiceDescriptor};

    fn project_host_client_test_node_type() -> NodeTypeDescriptor {
        NodeTypeDescriptor::new("az.editor.tests.Print", 1, "Print")
            .with_category_path(["Tests".to_string()])
            .with_port(NodePortDescriptor::new(
                NodePortId::new(1),
                "value",
                NodePortDirection::Input,
                NodePortValue::Data {
                    schema_type: "core.string".to_string(),
                },
            ))
    }

    fn project_host_client_test_graph_type() -> GraphTypeDescriptor {
        GraphTypeDescriptor::runtime_compiled(
            "az.editor.tests.logic-graph",
            1,
            "Editor Test Logic Graph",
            GraphSourceWorkflow::file("az.editor.tests.logic-graph.source", "azgraph.ron")
                .with_default_path_prefix("graphs"),
            GraphCompilerBackendDescriptor::packed_ir(
                "az.editor.tests.logic-graph.compiler",
                "azoth.graph.logic-ir/v1",
            )
            .with_capability_marker("zero-cost"),
            RuntimeGraphProductDescriptor::new(
                "azoth.graph.packed-ir",
                "azoth.graph.logic-ir",
                RuntimeGraphExecutionStrategy::PackedIr,
            ),
        )
        .with_node_catalog(GraphNodeCatalogRequirement::new("az.editor.tests.nodes"))
        .with_tag("test")
    }

    declare_caps!(EditorTestCaps:);

    /// The node and graph types these tests expect the project-host to serve.
    /// They were link-time inventory submissions; a composed project-host
    /// receives them from a contribution instead.
    struct Fixtures;

    impl Contribution for Fixtures {
        type Caps = EditorTestCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.editor-tests"),
                contribution: ContributionId::new("graph"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, EditorTestCaps>) {
            ctx.registrar::<NodeTypeRegistration>()
                .register(NodeTypeRegistration::new(
                    project_host_client_test_node_type(),
                ));
            ctx.registrar::<GraphTypeRegistration>()
                .register(GraphTypeRegistration::new(
                    project_host_client_test_graph_type(),
                ));
        }
    }

    fn composition() -> Composition {
        let mut composer = Composer::new(GemTargetRole::ProjectHost);
        composer
            .add(Fixtures, ProductActivation::default())
            .expect("an empty capability floor composes");
        Composition::new(composer).expect("editor test composition is valid and ready")
    }

    fn project_host_grant() -> Capability {
        Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_audience(PROJECT_HOST_AUDIENCE)
        .with_session(uuid::Uuid::from_bytes([0x44; 16]))
        .with_permissions([
            PROJECT_SCHEMA_PERMISSION,
            PROJECT_GAMEDATA_PERMISSION,
            PROJECT_NODE_CATALOG_PERMISSION,
            PROJECT_GRAPH_CATALOG_PERMISSION,
            PROJECT_INVENTORY_PERMISSION,
            PROJECT_EDIT_PERMISSION,
            PROJECT_DOCUMENT_READ_PERMISSION,
            PROJECT_DOCUMENT_WRITE_PERMISSION,
            PROJECT_RUNTIME_LAUNCH_PERMISSION,
            PROJECT_SOURCE_NAVIGATION_PERMISSION,
        ])
        .with_token_hash([0x70, 0x48])
    }

    fn project_host_grants() -> CapabilityGrantSet {
        CapabilityGrantSet::from_grants(vec![project_host_grant()])
    }

    fn write_test_project_source(root: &std::path::Path) {
        write_project_manifest(
            root,
            &ProjectManifest::new("local.editor_project_host", "Editor Project Host", "0.1.0"),
        )
        .unwrap();
        refresh_project_lock(root).unwrap();
    }

    fn open_test_project_host(root: &std::path::Path) -> ProjectHost {
        write_test_project_source(root);
        ProjectHost::open_project_source_root(root, "local.editor_project_host").unwrap()
    }

    struct TestProjectHostRpc {
        rpc: Rc<ProjectHostRpc>,
        _composition: Composition,
    }

    impl std::ops::Deref for TestProjectHostRpc {
        type Target = ProjectHostRpc;

        fn deref(&self) -> &Self::Target {
            &self.rpc
        }
    }

    fn project_host_rpc(host: ProjectHost) -> TestProjectHostRpc {
        project_host_rpc_with(host, composition())
    }

    fn project_host_rpc_with(host: ProjectHost, composition: Composition) -> TestProjectHostRpc {
        let rpc = Rc::new(ProjectHostRpc::test_new_composed(
            host,
            project_host_grants(),
            &composition,
        ));
        TestProjectHostRpc {
            rpc,
            _composition: composition,
        }
    }

    fn project_host_client_from_rpc(rpc: &TestProjectHostRpc) -> ProjectHostClient {
        ProjectHostClient::new(ProjectHostRpc::client_from_rc(&rpc.rpc))
            .with_capability_templates(vec![project_host_grant()])
    }

    /// Canonical in-process project-host client for other crates' modules'
    /// tests (e.g. recovery) that need a `ReflectedPrefabEditSession` value
    /// without standing up real IPC.
    pub fn test_project_host_client() -> ProjectHostClient {
        let temp = tempfile::tempdir().expect("create temp directory");
        write_test_project_source(temp.path());
        let host = open_test_project_host(temp.path());
        project_host_client_from_rpc(&project_host_rpc(host))
    }

    /// The same in-process project-host, rooted at a directory the caller keeps
    /// alive. Graph tests need that: they write command-batch side channels and
    /// read the resulting documents back across several RPCs.
    pub fn test_project_host_client_at(root: &std::path::Path) -> ProjectHostClient {
        let host = open_test_project_host(root);
        project_host_client_from_rpc(&project_host_rpc(host))
    }

    #[test]
    fn vnext_client_methods_decode_real_project_host_capnp_responses() {
        const SOURCE: &str = "client-vnext.prefab.ron";
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(SOURCE),
            r#"(
                version: 1,
                type_versions: {"Transform": 1},
                entities: {"root": (components: {"Transform": ()})},
                instances: {},
            )"#,
        )
        .unwrap();
        write_test_project_source(temp.path());
        let rpc = project_host_rpc(ProjectHost::with_source_root(temp.path()));
        let client = project_host_client_from_rpc(&rpc);

        let registry = futures::executor::block_on(client.type_registry_snapshot()).unwrap();
        let transform = registry
            .types
            .iter()
            .find(|descriptor| descriptor.editor_attributes.label.as_deref() == Some("Transform"))
            .expect("Transform registry projection");
        let opened = futures::executor::block_on(client.source_session_lifecycle(
            SOURCE,
            SourceSessionCommand::Open,
            0,
        ))
        .unwrap();
        assert!(opened.status.open);
        let initial = futures::executor::block_on(client.prefab_source_snapshot(SOURCE)).unwrap();
        assert!(initial.diagnostics.is_empty());

        let edited = futures::executor::block_on(client.apply_prefab_edit_command(
            SOURCE,
            opened.status.revision,
            &PrefabEditCommand::AddEntity {
                alias: "child".to_owned(),
                parent_alias: Some("root".to_owned()),
            },
        ))
        .unwrap();
        let edited = edited.snapshot.expect("edited Prefab snapshot");
        assert!(edited.entities.iter().any(|entity| entity.alias == "child"));

        let target = PrefabValueTarget {
            instance_alias_chain: Vec::new(),
            entity_alias: "root".to_owned(),
            path: az_proto_project::vnext::ReflectedPath {
                component_type_path: transform.type_path.clone(),
                segments: Vec::new(),
            },
        };
        let action = futures::executor::block_on(client.invoke_typed_action(
            SOURCE,
            edited.revision,
            &target,
            "editor.tests.unknown",
        ))
        .unwrap();
        assert!(!action.diagnostics.is_empty());
        let diagnostics = futures::executor::block_on(client.prefab_diagnostics(SOURCE)).unwrap();
        assert!(diagnostics.is_empty());

        let undone = futures::executor::block_on(client.source_session_lifecycle(
            SOURCE,
            SourceSessionCommand::Undo,
            edited.revision,
        ))
        .unwrap();
        assert_eq!(undone.status.redo_depth, 1);
        let redone = futures::executor::block_on(client.source_session_lifecycle(
            SOURCE,
            SourceSessionCommand::Redo,
            undone.status.revision,
        ))
        .unwrap();
        assert!(redone.status.open);
    }

    #[test]
    fn project_host_client_uses_descriptor_capability_templates() {
        let session = uuid::Uuid::from_bytes([0x31; 16]);
        let rpc = project_host_rpc(ProjectHost::new());
        let client = ProjectHostClient::new(ProjectHostRpc::client_from_rc(&rpc.rpc))
            .with_session_scope(session)
            .with_capability_templates(vec![
                Capability::new(
                    ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                    ServiceRole::Editor,
                )
                .with_session(session)
                .with_audience(PROJECT_HOST_AUDIENCE)
                .with_permissions([PROJECT_SCHEMA_PERMISSION])
                .with_token_hash([0xab, 0xcd]),
            ]);

        let capability = client
            .editor_capability([PROJECT_SCHEMA_PERMISSION])
            .unwrap();
        assert_eq!(capability.token_hash, vec![0xab, 0xcd]);
        assert!(client.editor_capability([PROJECT_EDIT_PERMISSION]).is_err());
    }

    #[test]
    fn project_host_client_rejects_empty_descriptor_capabilities() {
        let rpc = project_host_rpc(ProjectHost::new());
        let client = ProjectHostClient::new(ProjectHostRpc::client_from_rc(&rpc.rpc))
            .with_capability_templates(Vec::new());

        assert!(
            client
                .editor_capability([PROJECT_SCHEMA_PERMISSION])
                .is_err()
        );
    }

    #[test]
    fn project_host_client_rejects_unbrokered_descriptor_capability_templates() {
        let rpc = project_host_rpc(ProjectHost::new());
        let client = ProjectHostClient::new(ProjectHostRpc::client_from_rc(&rpc.rpc))
            .with_capability_templates(vec![
                Capability::new(
                    ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                    ServiceRole::Editor,
                )
                .with_audience(PROJECT_HOST_AUDIENCE)
                .with_permissions([PROJECT_SCHEMA_PERMISSION]),
            ]);

        assert!(
            client
                .editor_capability([PROJECT_SCHEMA_PERMISSION])
                .is_err()
        );
    }

    #[test]
    fn loads_node_type_catalog_from_project_host_side_channel() {
        let rpc = project_host_rpc(ProjectHost::new());
        let client = project_host_client_from_rpc(&rpc);

        let handle = futures::executor::block_on(client.node_type_catalog_handle()).unwrap();
        let expected_capability = client
            .editor_capability([PROJECT_NODE_CATALOG_PERMISSION])
            .unwrap();
        assert_eq!(handle.capability, Some(expected_capability));

        let catalog = futures::executor::block_on(client.load_node_type_catalog()).unwrap();
        let node_type = catalog
            .node_type(&"az.editor.tests.Print".into())
            .expect("test node type registered in project-host node catalog");

        assert_eq!(node_type.version, 1);
        assert_eq!(node_type.ports.len(), 1);
    }

    #[test]
    fn loads_graph_type_catalog_from_project_host_side_channel() {
        let rpc = project_host_rpc(ProjectHost::new());
        let client = project_host_client_from_rpc(&rpc);

        let handle = futures::executor::block_on(client.graph_type_catalog_handle()).unwrap();
        let expected_capability = client
            .editor_capability([PROJECT_GRAPH_CATALOG_PERMISSION])
            .unwrap();
        assert_eq!(handle.capability, Some(expected_capability));

        let catalog = futures::executor::block_on(client.load_graph_type_catalog()).unwrap();
        let graph_type = catalog
            .graph_type(&"az.editor.tests.logic-graph".into())
            .expect("test graph type registered in project-host graph catalog");

        assert_eq!(graph_type.version, 1);
        assert!(graph_type.compiler_backend.is_some());
        assert!(graph_type.runtime_product.is_some());
    }

    #[test]
    fn resolves_node_source_link_through_project_host() {
        let temp = tempfile::tempdir().unwrap();
        let manifest =
            ProjectManifest::new("local.editor_source_nav", "Editor Source Nav", "0.1.0");
        write_project_manifest(temp.path(), &manifest).unwrap();
        refresh_project_lock(temp.path()).unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/nodes.rs"), "fn run() {}\n").unwrap();

        let rpc = project_host_rpc(ProjectHost::open_source_root(temp.path()).unwrap());
        let client = project_host_client_from_rpc(&rpc);

        let target = futures::executor::block_on(client.resolve_node_source_link(
            NodeSourceLink::rust_symbol(
                "local.editor_source_nav",
                "editor_source_nav::nodes",
                "editor_source_nav::nodes::run",
                "src/nodes.rs",
                1,
                1,
            ),
        ))
        .unwrap();

        assert_eq!(target.path_kind, NodeSourceLinkPathKind::PackageRelative);
        assert_eq!(
            target.package_id.as_deref(),
            Some("local.editor_source_nav")
        );
        assert!(target.exists);
    }

    #[test]
    fn creates_graph_document_through_project_host_side_channel() {
        let temp = tempfile::tempdir().unwrap();
        let document_id = DocumentId::new("graphs/editor-created.visual.ron");
        let rpc = project_host_rpc(open_test_project_host(temp.path()));
        let client = project_host_client_from_rpc(&rpc);

        let snapshot = futures::executor::block_on(
            client.create_graph_document(&document_id, "az.editor.tests.logic-graph"),
        )
        .unwrap();

        assert_eq!(snapshot.document_id, document_id);
        assert_eq!(snapshot.revision, DocumentRevision::new(0));
        assert_eq!(snapshot.document.graph_type, "az.editor.tests.logic-graph");
        assert!(snapshot.document.nodes.is_empty());
        assert!(
            rpc.host()
                .graph_document(&DocumentId::new("graphs/editor-created.visual.ron"))
                .is_some()
        );
    }

    #[test]
    fn creates_graph_document_from_graph_type_creation_data() {
        let temp = tempfile::tempdir().unwrap();
        let rpc = project_host_rpc(open_test_project_host(temp.path()));
        let client = project_host_client_from_rpc(&rpc);
        let catalog = futures::executor::block_on(client.load_graph_type_catalog()).unwrap();
        let creation = crate::graph_creation_catalog_from_graph_type_catalog(&catalog);
        let graph_type = creation
            .graph_type("az.editor.tests.logic-graph")
            .expect("registered editor graph type in creation catalog");

        let snapshot = futures::executor::block_on(
            client.create_graph_document_from_creation_data(graph_type, "combat"),
        )
        .unwrap();

        assert_eq!(snapshot.document_id.as_str(), "graphs/combat.azgraph.ron");
        assert_eq!(snapshot.document.graph_type, "az.editor.tests.logic-graph");
        assert_eq!(
            rpc.host()
                .graph_document(&DocumentId::new("graphs/combat.azgraph.ron"))
                .unwrap()
                .graph_type,
            "az.editor.tests.logic-graph"
        );
    }

    #[test]
    fn loads_graph_document_snapshot_through_project_host_side_channel() {
        let document_id = DocumentId::new("graphs/editor-loaded.visual.ron");
        let node_id = GraphNodeId::new(uuid::Uuid::from_bytes([0x35; 16]));
        let mut node = GraphNode::new(node_id, "az.editor.tests.Print", 1);
        node.layout = GraphNodeLayout { x: 20.0, y: 40.0 };
        let temp = tempfile::tempdir().unwrap();
        let composition = composition();
        let rpc = project_host_rpc_with(open_test_project_host(temp.path()), composition);
        let client = project_host_client_from_rpc(&rpc);
        futures::executor::block_on(
            client.create_graph_document(&document_id, "az.editor.tests.logic-graph"),
        )
        .unwrap();
        futures::executor::block_on(client.apply_graph_commands(
            &GraphCommandBatchSnapshot {
                document_id: document_id.clone(),
                expected_revision: Some(DocumentRevision::new(0)),
                client_batch_id: "project-host-existing-graph".to_string(),
                commands: vec![GraphCommand::AddNode { node: node.clone() }],
            },
            &temp.path().join("editor-side-channels"),
        ))
        .unwrap();

        let snapshot =
            futures::executor::block_on(client.graph_document_snapshot(&document_id)).unwrap();

        assert_eq!(snapshot.document_id, document_id);
        assert_eq!(snapshot.revision, DocumentRevision::new(1));
        assert_eq!(snapshot.document.nodes, vec![node]);
    }

    #[test]
    fn applies_graph_commands_through_project_host_side_channels() {
        let temp = tempfile::tempdir().unwrap();
        let document_id = DocumentId::new("graphs/editor.visual.ron");
        let composition = composition();
        let rpc = project_host_rpc_with(open_test_project_host(temp.path()), composition);
        let client = project_host_client_from_rpc(&rpc);
        futures::executor::block_on(
            client.create_graph_document(&document_id, "az.editor.tests.logic-graph"),
        )
        .unwrap();
        let node_id = GraphNodeId::new(uuid::Uuid::from_bytes([0x33; 16]));
        let mut node = GraphNode::new(node_id, "az.editor.tests.Print", 1);
        node.layout = GraphNodeLayout { x: 14.0, y: 28.0 };
        let batch = GraphCommandBatchSnapshot {
            document_id: document_id.clone(),
            expected_revision: Some(DocumentRevision::new(0)),
            client_batch_id: "editor-graph-batch-1".to_string(),
            commands: vec![GraphCommand::AddNode { node: node.clone() }],
        };

        let status = futures::executor::block_on(
            client.apply_graph_commands(&batch, &temp.path().join("editor-side-channels")),
        )
        .unwrap();

        assert_eq!(status.document_id, document_id);
        assert_eq!(status.client_batch_id, "editor-graph-batch-1");
        assert_eq!(status.applied_command_count, 1);
        assert_eq!(
            status.outcome,
            az_proto_project::GraphCommandStatusOutcome::Accepted {
                revision: DocumentRevision::new(1)
            }
        );
        let host = rpc.host();
        assert_eq!(host.graph_document(&document_id).unwrap().nodes, vec![node]);
    }

    #[test]
    fn saves_graph_document_through_project_host_rpc() {
        let temp = tempfile::tempdir().unwrap();
        let document_id = DocumentId::new("graphs/editor-save.visual.ron");
        write_project_manifest(
            temp.path(),
            &ProjectManifest::new("local.editor_db_project", "Editor DB Project", "0.1.0"),
        )
        .unwrap();
        refresh_project_lock(temp.path()).unwrap();
        let composition = composition();
        let host =
            ProjectHost::open_project_source_root(temp.path(), "local.editor_db_project").unwrap();
        let rpc = project_host_rpc_with(host, composition);
        let client = project_host_client_from_rpc(&rpc);
        futures::executor::block_on(
            client.create_graph_document(&document_id, "az.editor.tests.logic-graph"),
        )
        .unwrap();
        let node_id = GraphNodeId::new(uuid::Uuid::from_bytes([0xe1; 16]));
        let node = GraphNode::new(node_id, "az.editor.tests.Print", 1);
        futures::executor::block_on(client.apply_graph_commands(
            &GraphCommandBatchSnapshot {
                document_id: document_id.clone(),
                expected_revision: Some(DocumentRevision::new(0)),
                client_batch_id: "editor-graph-save-1".to_string(),
                commands: vec![GraphCommand::AddNode { node }],
            },
            &temp.path().join("editor-side-channels"),
        ))
        .unwrap();

        let saved =
            futures::executor::block_on(client.save_graph_document_record(&document_id)).unwrap();

        assert_eq!(saved.document_id, document_id);
        assert_eq!(saved.revision, DocumentRevision::new(1));
        assert_eq!(saved.source_path, "graphs/editor-save.visual.ron");
        assert_eq!(saved.schema_type, "az.editor.tests.logic-graph");
        assert_eq!(saved.content_hash.len(), blake3::OUT_LEN);
        assert!(saved.byte_length > 0);

        let written_bytes = std::fs::read(temp.path().join(&saved.source_path)).unwrap();
        assert_eq!(written_bytes.len() as u64, saved.byte_length);
        assert_eq!(
            blake3::hash(&written_bytes).as_bytes().to_vec(),
            saved.content_hash
        );
    }

    #[test]
    fn connects_to_project_host_over_endpoint_transport() {
        let temp = tempfile::tempdir().unwrap();
        write_test_project_source(temp.path());
        let mut descriptor = ServiceDescriptor::new(
            ServiceId::new(PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .with_capability(
            Capability::new(
                ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                ServiceRole::Editor,
            )
            .with_audience(PROJECT_HOST_AUDIENCE)
            .with_session(uuid::Uuid::from_bytes([0x44; 16]))
            .with_permissions([PROJECT_SCHEMA_PERMISSION])
            .with_token_hash([0x70, 0x48]),
        );
        let server = start_project_host_rpc_server_with_capability_grants(
            temp.path(),
            descriptor.endpoint.clone(),
            CapabilityGrantSet::from_grants(descriptor.capabilities.clone()),
        )
        .unwrap();
        descriptor.endpoint = server.endpoint().clone();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&runtime, async {
            let client = ProjectHostClient::connect(&descriptor).await.unwrap();
            let registry = client.type_registry_snapshot().await.unwrap();
            assert!(!registry.types.is_empty());
        });

        server.stop().expect("stop project-host server");
    }
}
