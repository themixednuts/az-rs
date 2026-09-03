//! Editor-side runtime-host client and session controller.

use az_editor_ui::panels::{
    EditorRuntimeProjectionCatalog, EditorRuntimeStateData, EditorRuntimeStatus,
    EditorViewportRenderStatus, RuntimeProjectionData,
};
use az_editor_ui::viewport_texture::{SharedViewportTexture, ViewportFrameSource};
use az_proto_asset::{
    ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME, AssetRootScope, WorkspaceSnapshot,
};
use az_proto_core::{
    Capability, ServiceDescriptor, ServiceId, ServiceRole, SideChannelKind,
    validate_side_channel_capability_matches,
};
use az_proto_runtime::{
    LaunchRuntimeRequest, RUNTIME_CONTROL_PERMISSION, RUNTIME_HOST_AUDIENCE,
    RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME, RUNTIME_READ_PERMISSION,
    RuntimeAssetSourceRoot, RuntimeProjectionCatalogRequest, RuntimeProjectionCatalogResult,
    RuntimeProjectionDescriptor, RuntimeRole, RuntimeState, RuntimeStatus, RuntimeStatusRequest,
    RuntimeStatusResult, RuntimeViewportFrame, RuntimeViewportRequest, RuntimeViewportResult,
    StopRuntimeRequest, ViewportPixelFormat, runtime_capnp,
};
use az_proto_session::{
    ServiceProcessState as ProtoServiceProcessState, SessionManifest as ProtoSessionManifest,
    SessionWorkspaceStatus as ProtoSessionWorkspaceStatus,
};
use gpui::{App, BorrowAppContext};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::{error, info};
use uuid::Uuid;

use crate::asset_processor::AssetProcessorClient;
use crate::attach::{AttachedWorkspace, EditorAttachSession};
use crate::error::{EditorError, EditorResult};
use crate::gpu::EditorGpuState;
use crate::project_host::{
    EDITOR_SERVICE_NAME, EDITOR_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME,
    PROJECT_HOST_SERVICE_NAMESPACE, ProjectHostClient, ProjectRuntimeLaunchSnapshotContext,
};
use crate::service_descriptor::{
    validate_descriptor_capability_templates, validate_session_descriptor_capability_templates,
};
use crate::session_supervisor::SessionSupervisorClient;

pub const EDITOR_WORLD_RUNTIME_ID: &str = "editor-world";

#[derive(Clone)]
pub struct RuntimeHostClient {
    client: runtime_capnp::runtime_host::Client,
    editor_service: ServiceId,
    session_id: Option<Uuid>,
    capability_templates: Vec<Capability>,
    descriptor_capabilities_required: bool,
}

impl RuntimeHostClient {
    #[cfg(test)]
    #[must_use]
    pub fn new(client: runtime_capnp::runtime_host::Client) -> Self {
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
        client: runtime_capnp::runtime_host::Client,
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

    /// Connect to a runtime-host already scoped to `session_id`, so every later
    /// call writes a session-bound capability.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when `descriptor` does not name
    /// the runtime-host service id and role, when it claims a protocol version
    /// other than the current one, when its brokered capability templates are
    /// empty or tokenless, or when `session_id` is nil or any template is
    /// unscoped or scoped to a different session. Returns
    /// [`EditorError::RpcTransport`] when the descriptor's endpoint cannot be
    /// dialed.
    pub async fn connect_for_session(
        descriptor: &ServiceDescriptor,
        session_id: Uuid,
    ) -> EditorResult<Self> {
        validate_runtime_host_descriptor_for_session(descriptor, session_id)?;
        let client = az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
        Ok(
            Self::from_descriptor_client(client, descriptor.capabilities.clone())
                .with_session_scope(session_id),
        )
    }

    /// Launch `runtime_id` in `role` from a project-host launch `snapshot`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no
    /// runtime control capability, and [`EditorError::ServiceProtocol`] when the
    /// request cannot be encoded, the call fails, or the response cannot be
    /// decoded. Returns [`EditorError::ServiceAuthorityMismatch`] when the
    /// returned status names a different runtime id, a blank one, a role other
    /// than `role`, or a blank project id or session slug.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn launch(
        &self,
        runtime_id: &str,
        role: RuntimeRole,
        snapshot: az_proto_core::SideChannelHandle,
    ) -> EditorResult<RuntimeStatus> {
        let mut request = self.client.launch_request();
        (LaunchRuntimeRequest {
            capability: self.runtime_control_capability()?,
            runtime_id: runtime_id.to_string(),
            role,
            snapshot,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let status = RuntimeStatus::from_capnp(response.get()?.get_status()?)?;
        ensure_runtime_status_matches_request(&status, runtime_id, Some(role), "launch")?;
        Ok(status)
    }

    /// Read the status of `runtime_id`, or `None` when the runtime-host has no
    /// runtime registered under that id.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no
    /// runtime read capability, and [`EditorError::ServiceProtocol`] when the
    /// request cannot be encoded, the call fails, or the response cannot be
    /// decoded. Returns [`EditorError::ServiceAuthorityMismatch`] when a status
    /// comes back for a different runtime id, with a blank id, or with a blank
    /// project id or session slug.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn status(&self, runtime_id: &str) -> EditorResult<Option<RuntimeStatus>> {
        let mut request = self.client.status_request();
        (RuntimeStatusRequest {
            capability: self.runtime_read_capability()?,
            runtime_id: runtime_id.to_string(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = RuntimeStatusResult::from_capnp(response.get()?.get_result()?)?;
        if let Some(status) = &result.status {
            ensure_runtime_status_matches_request(status, runtime_id, None, "status")?;
        }
        Ok(result.status)
    }

    /// Read the current viewport frame of `runtime_id`, or `None` when the
    /// runtime has not published one.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no
    /// runtime read capability, and [`EditorError::ServiceProtocol`] when the
    /// request cannot be encoded, the call fails, or the response cannot be
    /// decoded. Returns [`EditorError::ServiceAuthorityMismatch`] when a frame
    /// comes back for another runtime, with a blank id, with a zero width or
    /// height, with an unknown pixel format, with a row pitch or side-channel
    /// byte length below what the dimensions require (or one that overflows),
    /// with a blank side-channel locator or platform, or on any side-channel
    /// kind other than a GPU surface. Returns
    /// [`EditorError::SideChannelCapability`] when the frame's color handle is
    /// not covered by the read capability this call presented.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn viewport_frame(
        &self,
        runtime_id: &str,
    ) -> EditorResult<Option<RuntimeViewportFrame>> {
        let mut request = self.client.viewport_frame_request();
        let capability = self.runtime_read_capability()?;
        (RuntimeViewportRequest {
            capability: capability.clone(),
            runtime_id: runtime_id.to_string(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let frame = RuntimeViewportResult::from_capnp(response.get()?.get_result()?)?.frame;
        if let Some(frame) = &frame {
            ensure_runtime_viewport_frame_matches_request(frame, runtime_id, "viewportFrame")?;
            validate_runtime_viewport_frame_side_channel_capability(frame, &capability)?;
        }
        Ok(frame)
    }

    /// Read the runtime projections this runtime-host publishes.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no
    /// runtime read capability, and [`EditorError::ServiceProtocol`] when the
    /// request cannot be encoded, the call fails, or the response cannot be
    /// decoded. Returns [`EditorError::ServiceAuthorityMismatch`] when a
    /// projection has a blank name, a blank or repeated launch profile, or when
    /// the catalog repeats a projection name.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn projection_catalog(&self) -> EditorResult<RuntimeProjectionCatalogResult> {
        let mut request = self.client.projection_catalog_request();
        (RuntimeProjectionCatalogRequest {
            capability: self.runtime_read_capability()?,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = RuntimeProjectionCatalogResult::from_capnp(response.get()?.get_result()?)?;
        ensure_runtime_projection_catalog_matches_request(&result)?;
        Ok(result)
    }

    /// Stop `runtime_id`, keeping its state when `preserve` is set.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no
    /// runtime control capability, and [`EditorError::ServiceProtocol`] when the
    /// request cannot be encoded, the call fails, or the response cannot be
    /// decoded. Returns [`EditorError::ServiceAuthorityMismatch`] when the
    /// returned status names a different runtime id, a blank one, or a blank
    /// project id or session slug.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn stop(&self, runtime_id: &str, preserve: bool) -> EditorResult<RuntimeStatus> {
        let mut request = self.client.stop_request();
        (StopRuntimeRequest {
            capability: self.runtime_control_capability()?,
            runtime_id: runtime_id.to_string(),
            preserve,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let status = RuntimeStatus::from_capnp(response.get()?.get_status()?)?;
        ensure_runtime_status_matches_request(&status, runtime_id, None, "stop")?;
        Ok(status)
    }

    /// Turn an optional status result into a required one, for callers that
    /// only make sense against a registered runtime.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when `result` carries no
    /// status, meaning the runtime-host has nothing registered under
    /// `runtime_id`.
    pub fn require_status(
        result: RuntimeStatusResult,
        runtime_id: &str,
    ) -> EditorResult<RuntimeStatus> {
        result.status.ok_or_else(|| {
            EditorError::ServiceDiscovery(format!("runtime `{runtime_id}` was not registered"))
        })
    }

    fn runtime_read_capability(&self) -> EditorResult<Capability> {
        self.runtime_capability(&[RUNTIME_READ_PERMISSION])
    }

    fn runtime_control_capability(&self) -> EditorResult<Capability> {
        self.runtime_capability_for(
            self.editor_service.clone(),
            ServiceRole::Editor,
            &[RUNTIME_CONTROL_PERMISSION],
        )
    }

    fn project_host_runtime_control_capability(&self) -> EditorResult<Capability> {
        self.runtime_capability_for(
            ServiceId::new(PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
            &[RUNTIME_CONTROL_PERMISSION],
        )
    }

    fn runtime_capability(&self, permissions: &[&str]) -> EditorResult<Capability> {
        self.runtime_capability_for(
            self.editor_service.clone(),
            ServiceRole::Editor,
            permissions,
        )
    }

    fn runtime_capability_for(
        &self,
        service: ServiceId,
        role: ServiceRole,
        permissions: &[&str],
    ) -> EditorResult<Capability> {
        if self.descriptor_capabilities_required {
            return self
                .capability_template_for(&service, role, permissions)
                .ok_or_else(|| Self::missing_capability_error(permissions));
        }

        let capability = Capability::new(service, role);
        let capability = match self.session_id {
            Some(session_id) => capability.with_session(session_id),
            None => capability,
        };
        Ok(capability
            .with_audience(RUNTIME_HOST_AUDIENCE)
            .with_permissions(permissions.iter().copied()))
    }

    fn capability_template_for(
        &self,
        service: &ServiceId,
        role: ServiceRole,
        permissions: &[&str],
    ) -> Option<Capability> {
        self.capability_templates
            .iter()
            .find(|capability| {
                capability.service == *service
                    && capability.matches_brokered_template_request(
                        role,
                        RUNTIME_HOST_AUDIENCE,
                        permissions,
                        self.session_id,
                    )
            })
            .cloned()
            .map(|capability| capability.scoped_to(self.session_id))
    }

    fn missing_capability_error(permissions: &[&str]) -> EditorError {
        EditorError::ServiceDiscovery(format!(
            "runtime-host descriptor did not grant editor capability `{}`",
            permissions.join(", ")
        ))
    }

    #[must_use]
    fn from_descriptor_client(
        client: runtime_capnp::runtime_host::Client,
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

#[derive(Clone)]
pub struct EditorRuntimeController {
    session_id: Option<Uuid>,
    project_id: String,
    session_slug: String,
    project_root: String,
    workspace: AttachedWorkspace,
    run_dir: Option<String>,
    #[cfg(test)]
    project_host: ProjectHostClient,
    #[cfg(test)]
    runtime_host: Option<RuntimeHostClient>,
    supervisor_descriptor: Option<ServiceDescriptor>,
    #[cfg(test)]
    supervisor: Option<SessionSupervisorClient>,
    #[cfg(test)]
    asset_processor: Option<AssetProcessorClient>,
}

impl EditorRuntimeController {
    #[cfg(test)]
    #[must_use]
    pub fn new(
        project_id: impl Into<String>,
        session_slug: impl Into<String>,
        project_root: impl Into<String>,
        workspace_path: impl Into<String>,
        branch: impl Into<String>,
        project_host: ProjectHostClient,
        runtime_host: RuntimeHostClient,
    ) -> Self {
        let project_id = project_id.into();
        Self {
            session_id: None,
            project_id: project_id.clone(),
            session_slug: session_slug.into(),
            project_root: project_root.into(),
            workspace: AttachedWorkspace {
                project_id,
                workspace_root: workspace_path.into(),
                branch: branch.into(),
            },
            run_dir: None,
            project_host,
            runtime_host: Some(runtime_host),
            supervisor_descriptor: None,
            supervisor: None,
            asset_processor: None,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub const fn with_session_id(mut self, session_id: Uuid) -> Self {
        self.session_id = Some(session_id);
        self
    }

    #[cfg(test)]
    #[must_use]
    pub fn with_asset_processor(mut self, asset_processor: Option<AssetProcessorClient>) -> Self {
        self.asset_processor = asset_processor;
        self
    }

    #[cfg(test)]
    #[must_use]
    pub fn with_session_supervisor(mut self, supervisor: Option<SessionSupervisorClient>) -> Self {
        self.supervisor = supervisor;
        self
    }

    /// Build a runtime controller from an attached session. It records the
    /// session-supervisor descriptor rather than a runtime-host client, so a
    /// session whose runtime-host is not running yet still yields a controller.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::RpcTransport`] or
    /// [`EditorError::ServiceDiscovery`] only under `cfg(test)`, where this
    /// eagerly dials the attached project-host, asset-processor, and any
    /// registered runtime-host. The production build opens no transport here and
    /// always succeeds; `Option` is reserved for a session that cannot host a
    /// runtime at all.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    #[allow(clippy::unused_async)] // the body awaits only under cfg(test); callers await it either way.
    pub async fn connect_attached(session: &EditorAttachSession) -> EditorResult<Option<Self>> {
        #[cfg(test)]
        let project_host = ProjectHostClient::connect_for_session(
            &session.services.project_host,
            session.session_id,
        )
        .await?;
        #[cfg(test)]
        let runtime_host = match &session.services.runtime_host {
            Some(runtime_host) => Some(
                RuntimeHostClient::connect_for_session(runtime_host, session.session_id).await?,
            ),
            None => None,
        };
        #[cfg(test)]
        let asset_processor = AssetProcessorClient::connect_for_session(
            &session.services.asset_processor,
            session.session_id,
        )
        .await?;
        let controller = Self {
            session_id: Some(session.session_id),
            project_id: session.project_id.clone(),
            session_slug: session.session_slug.clone(),
            project_root: session.project_root.to_string_lossy().into_owned(),
            workspace: session.workspace.clone(),
            run_dir: Some(session.run_dir.to_string_lossy().into_owned()),
            #[cfg(test)]
            project_host,
            #[cfg(test)]
            runtime_host,
            supervisor_descriptor: Some(session.session_supervisor.clone()),
            #[cfg(test)]
            supervisor: None,
            #[cfg(test)]
            asset_processor: Some(asset_processor),
        };
        Ok(Some(controller))
    }

    /// Read the attached session's workspace snapshot and launch the editor
    /// world from it.
    ///
    /// # Errors
    ///
    /// Returns anything [`Self::session_workspace_snapshot`] and
    /// [`Self::launch_editor_world`] return.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn launch_attached_editor_world(
        &self,
        include_unsaved_journal: bool,
    ) -> EditorResult<RuntimeStatus> {
        let workspace_snapshot = self.session_workspace_snapshot().await?;
        self.launch_editor_world(&workspace_snapshot, include_unsaved_journal)
            .await
    }

    /// The DB id of the attached session's workspace snapshot.
    ///
    /// # Errors
    ///
    /// Returns anything [`Self::session_workspace_snapshot`] returns; this only
    /// projects the id out of the validated snapshot.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn session_workspace_snapshot_id(&self) -> EditorResult<i64> {
        Ok(self.session_workspace_snapshot().await?.workspace_id)
    }

    /// Read the attached session's workspace snapshot from asset-processor and
    /// hold it to this controller's identity.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this controller has no
    /// session-supervisor descriptor or session id to resolve asset-processor
    /// with, when asset-processor is not registered or reachable for the
    /// session, when it has no snapshot for this workspace root and branch, or
    /// when the snapshot fails validation — a non-positive workspace id, a
    /// project, workspace root, or branch other than the attached session's,
    /// negative or out-of-order timestamps, or a source root with a foreign
    /// workspace id, a non-positive root identifier, or a blank declared root
    /// id, owner id, physical root, or portable key. Returns
    /// [`EditorError::MissingRuntimeAssetSourceRoots`] when the snapshot carries
    /// no source roots at all.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn session_workspace_snapshot(&self) -> EditorResult<WorkspaceSnapshot> {
        let asset_processor = self.asset_processor_client_for_runtime_launch().await?;
        let snapshot = asset_processor
            .workspace_snapshot(AssetRootScope::All)
            .await?
            .ok_or_else(|| {
                EditorError::ServiceDiscovery(format!(
                    "asset-processor has no workspace snapshot for `{}` on branch `{}`",
                    self.workspace.workspace_root, self.workspace.branch
                ))
            })?;
        self.validate_runtime_launch_workspace_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    /// Launch the editor world against `workspace_snapshot`, starting the
    /// session's runtime-host first when one is not already answering.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this controller has no
    /// session id, when `workspace_snapshot` fails the identity and source-root
    /// validation [`Self::session_workspace_snapshot`] describes, when no
    /// session-supervisor descriptor is available to start runtime-host through,
    /// when the terminal start status does not report runtime-host registered
    /// and running, when the descriptor the supervisor then resolves disagrees
    /// with that status, or when the started runtime-host still refuses its
    /// readiness check. Returns [`EditorError::MissingRuntimeAssetSourceRoots`]
    /// for a snapshot with no source roots, and
    /// [`EditorError::SessionDiscoveryMismatch`] when the supervisor's status is
    /// for another session. Also returns anything project-host's runtime launch
    /// snapshot call and [`RuntimeHostClient::launch`] return.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn launch_editor_world(
        &self,
        workspace_snapshot: &WorkspaceSnapshot,
        include_unsaved_journal: bool,
    ) -> EditorResult<RuntimeStatus> {
        let session_id = self.validate_runtime_launch_workspace_snapshot(workspace_snapshot)?;
        let asset_package_roots = self.runtime_asset_package_roots_for_launch().await?;
        let runtime_host = self.runtime_host_client_for_launch().await?;
        let project_host = self.project_host_client_for_runtime_launch().await?;
        let snapshot = project_host
            .runtime_launch_snapshot(ProjectRuntimeLaunchSnapshotContext {
                runtime_launch_capability: runtime_host
                    .project_host_runtime_control_capability()?
                    .scoped_to(Some(session_id)),
                role: RuntimeRole::EditorWorld,
                project_id: self.project_id.clone(),
                session_id,
                session_slug: self.session_slug.clone(),
                project_root: self.project_root.clone(),
                workspace_path: self.workspace.workspace_root.clone(),
                workspace_id: workspace_snapshot.workspace_id,
                include_unsaved_journal,
                launch_profile: "editor".to_string(),
                asset_source_roots: runtime_asset_source_roots(workspace_snapshot),
                asset_package_roots,
            })
            .await?;
        runtime_host
            .launch(EDITOR_WORLD_RUNTIME_ID, RuntimeRole::EditorWorld, snapshot)
            .await
    }

    /// The editor world's runtime status, or `None` when no runtime-host is
    /// registered and running for this session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::RpcTransport`] when the session-supervisor or the
    /// registered runtime-host cannot be dialed, and
    /// [`EditorError::SessionDiscoveryMismatch`] when the supervisor's status is
    /// for another session. Returns anything [`RuntimeHostClient::status`]
    /// returns. A session with no runtime-host registered is `Ok(None)`, not an
    /// error.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn editor_world_status(&self) -> EditorResult<Option<RuntimeStatus>> {
        let Some(runtime_host) = self.runtime_host_client_for_query().await? else {
            return Ok(None);
        };
        runtime_host.status(EDITOR_WORLD_RUNTIME_ID).await
    }

    /// The editor world's current viewport frame, or `None` when no
    /// runtime-host is registered and running for this session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::RpcTransport`] when the session-supervisor or the
    /// registered runtime-host cannot be dialed, and
    /// [`EditorError::SessionDiscoveryMismatch`] when the supervisor's status is
    /// for another session. Returns anything
    /// [`RuntimeHostClient::viewport_frame`] returns, including the
    /// side-channel capability check on the returned frame.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn editor_world_viewport_frame(&self) -> EditorResult<Option<RuntimeViewportFrame>> {
        let Some(runtime_host) = self.runtime_host_client_for_query().await? else {
            return Ok(None);
        };
        runtime_host.viewport_frame(EDITOR_WORLD_RUNTIME_ID).await
    }

    /// The runtime projection catalog, projected into the UI shape. A session
    /// with no runtime-host running yields an empty catalog rather than an
    /// error.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::RpcTransport`] when the session-supervisor or the
    /// registered runtime-host cannot be dialed, and
    /// [`EditorError::SessionDiscoveryMismatch`] when the supervisor's status is
    /// for another session. Returns anything
    /// [`RuntimeHostClient::projection_catalog`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn load_projection_catalog(&self) -> EditorResult<EditorRuntimeProjectionCatalog> {
        let Some(runtime_host) = self.runtime_host_client_for_query().await? else {
            return Ok(EditorRuntimeProjectionCatalog::new(Vec::new()));
        };
        let catalog = runtime_host.projection_catalog().await?;
        Ok(runtime_projection_catalog_to_ui(catalog))
    }

    /// Stop the editor world, keeping its state when `preserve` is set.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when no runtime-host is
    /// registered and running for this session — unlike the query paths, there
    /// is nothing to stop, so this is an error rather than a quiet `None`.
    /// Returns [`EditorError::RpcTransport`] when the session-supervisor or
    /// runtime-host cannot be dialed, and
    /// [`EditorError::SessionDiscoveryMismatch`] when the supervisor's status is
    /// for another session. Returns anything [`RuntimeHostClient::stop`]
    /// returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn stop_editor_world(&self, preserve: bool) -> EditorResult<RuntimeStatus> {
        let Some(runtime_host) = self.runtime_host_client_for_query().await? else {
            return Err(EditorError::ServiceDiscovery(
                "runtime-host is not running for this editor session".to_string(),
            ));
        };
        runtime_host.stop(EDITOR_WORLD_RUNTIME_ID, preserve).await
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn runtime_host_client_for_query(&self) -> EditorResult<Option<RuntimeHostClient>> {
        if let Some(runtime_host) = self.runtime_host_client_from_supervisor().await? {
            return Ok(Some(runtime_host));
        }

        #[cfg(test)]
        {
            Ok(self.runtime_host.clone())
        }

        #[cfg(not(test))]
        {
            Ok(None)
        }
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn runtime_host_client_from_supervisor(&self) -> EditorResult<Option<RuntimeHostClient>> {
        let Some(supervisor) = self
            .optional_session_supervisor_client("runtime-host query", true)
            .await?
        else {
            return Ok(None);
        };

        let Some(descriptor) = self
            .runtime_host_descriptor_from_supervisor(&supervisor, "runtime-host query")
            .await?
        else {
            return Ok(None);
        };
        let client = self.connect_runtime_host_for_session(&descriptor).await?;
        Ok(Some(self.scope_runtime_host_client(client)))
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn runtime_host_client_for_launch(&self) -> EditorResult<RuntimeHostClient> {
        if let Some(supervisor) = self
            .optional_session_supervisor_client("runtime-host launch", true)
            .await?
            && let Some(descriptor) = self
                .runtime_host_descriptor_from_supervisor(&supervisor, "runtime-host launch")
                .await?
        {
            match self.connect_runtime_host_for_session(&descriptor).await {
                Ok(runtime_host) => match runtime_host.projection_catalog().await {
                    Ok(_) => {
                        info!(
                            session = %self.session_slug,
                            "reusing running runtime-host for editor launch"
                        );
                        return Ok(runtime_host);
                    }
                    Err(error) => {
                        info!(
                            error = %error,
                            session = %self.session_slug,
                            "resolved runtime-host is not ready for editor launch; requesting startup"
                        );
                    }
                },
                Err(error) => {
                    info!(
                        error = %error,
                        session = %self.session_slug,
                        "registered runtime-host is unreachable for editor launch; requesting startup"
                    );
                }
            }
        }

        #[cfg(test)]
        if self.supervisor.is_none()
            && self.supervisor_descriptor.is_none()
            && let Some(runtime_host) = &self.runtime_host
        {
            return Ok(runtime_host.clone());
        }

        let supervisor = self
            .session_supervisor_client("runtime-host launch startup")
            .await?;
        let result = supervisor
            .start_services_for(
                &self.session_slug,
                "editor requested runtime-host startup",
                vec![RUNTIME_HOST_SERVICE_NAME.to_string()],
            )
            .await?;
        self.ensure_supervisor_status_matches_controller(
            &result.status,
            "runtime-host terminal start",
        )?;
        let expected = optional_service_descriptor_from_status(
            &result.status.manifest,
            &runtime_host_service_id(),
            ServiceRole::RuntimeHost,
        )
        .ok_or_else(|| {
            EditorError::ServiceDiscovery(format!(
                "runtime-host service is not registered for session `{}` in terminal start status",
                self.session_slug
            ))
        })?;
        if !status_service_process_is_running(&result.status.manifest, expected)? {
            return Err(EditorError::ServiceDiscovery(format!(
                "runtime-host terminal start status is not running for session `{}`",
                self.session_slug
            )));
        }
        let descriptor = supervisor
            .resolve_runtime_host_descriptor(&self.session_slug)
            .await?;
        ensure_resolved_descriptor_matches_status(
            "runtime-host terminal start",
            &self.session_slug,
            expected,
            &descriptor,
        )?;
        let runtime_host = self.connect_runtime_host_for_session(&descriptor).await?;
        runtime_host.projection_catalog().await?;
        Ok(runtime_host)
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn runtime_host_descriptor_from_supervisor(
        &self,
        supervisor: &SessionSupervisorClient,
        operation: &'static str,
    ) -> EditorResult<Option<ServiceDescriptor>> {
        let status = supervisor.status(&self.session_slug).await?;
        self.ensure_supervisor_status_matches_controller(&status, operation)?;
        let Some(expected) = optional_service_descriptor_from_status(
            &status.manifest,
            &runtime_host_service_id(),
            ServiceRole::RuntimeHost,
        ) else {
            return Ok(None);
        };
        if !status_service_process_is_running(&status.manifest, expected)? {
            return Ok(None);
        }
        let resolved = supervisor
            .resolve_runtime_host_descriptor(&self.session_slug)
            .await?;
        ensure_resolved_descriptor_matches_status(
            operation,
            &self.session_slug,
            expected,
            &resolved,
        )?;
        Ok(Some(resolved))
    }

    fn ensure_supervisor_status_matches_controller(
        &self,
        status: &ProtoSessionWorkspaceStatus,
        operation: &'static str,
    ) -> EditorResult<()> {
        self.ensure_supervisor_manifest_matches_controller(&status.manifest, operation)?;
        Ok(())
    }

    fn ensure_supervisor_manifest_matches_controller(
        &self,
        manifest: &ProtoSessionManifest,
        operation: &'static str,
    ) -> EditorResult<()> {
        if let Some(session_id) = self.session_id
            && manifest.id != session_id
        {
            return Err(runtime_session_mismatch(
                operation,
                &manifest.slug,
                format!(
                    "response session id `{}` does not match attached session id `{}`",
                    manifest.id, session_id
                ),
            ));
        }
        if manifest.slug != self.session_slug {
            return Err(runtime_session_mismatch(
                operation,
                &manifest.slug,
                format!(
                    "response session slug `{}` does not match attached session slug `{}`",
                    manifest.slug, self.session_slug
                ),
            ));
        }
        if manifest.project_id != self.project_id {
            return Err(runtime_session_mismatch(
                operation,
                &manifest.slug,
                format!(
                    "response project `{}` does not match attached project `{}`",
                    manifest.project_id, self.project_id
                ),
            ));
        }
        if !same_protocol_path(
            Path::new(&manifest.project_root),
            Path::new(&self.project_root),
        ) {
            return Err(runtime_session_mismatch(
                operation,
                &manifest.slug,
                format!(
                    "response project root `{}` does not match attached project root `{}`",
                    manifest.project_root, self.project_root
                ),
            ));
        }
        if !same_protocol_path(
            Path::new(&manifest.workspace_root),
            Path::new(&self.workspace.workspace_root),
        ) {
            return Err(runtime_session_mismatch(
                operation,
                &manifest.slug,
                format!(
                    "response workspace `{}` does not match attached workspace `{}`",
                    manifest.workspace_root, self.workspace.workspace_root
                ),
            ));
        }
        if let Some(run_dir) = &self.run_dir
            && !same_protocol_path(Path::new(&manifest.run_dir), Path::new(run_dir))
        {
            return Err(runtime_session_mismatch(
                operation,
                &manifest.slug,
                format!(
                    "response run directory `{}` does not match attached run directory `{}`",
                    manifest.run_dir, run_dir
                ),
            ));
        }
        Ok(())
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn connect_runtime_host_for_session(
        &self,
        descriptor: &ServiceDescriptor,
    ) -> EditorResult<RuntimeHostClient> {
        let Some(session_id) = self.session_id else {
            return Err(EditorError::ServiceDiscovery(
                "runtime-host client resolution requires an attached session id".to_string(),
            ));
        };
        RuntimeHostClient::connect_for_session(descriptor, session_id).await
    }

    const fn scope_runtime_host_client(&self, client: RuntimeHostClient) -> RuntimeHostClient {
        match self.session_id {
            Some(session_id) => client.with_session_scope(session_id),
            None => client,
        }
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn project_host_client_for_runtime_launch(&self) -> EditorResult<ProjectHostClient> {
        #[cfg(test)]
        if self.supervisor.is_none() && self.supervisor_descriptor.is_none() {
            return Ok(self.project_host.clone());
        }

        let supervisor = self
            .session_supervisor_client("runtime project-host launch")
            .await?;

        let expected = self
            .attached_service_descriptor_from_status(
                &supervisor,
                project_host_service_id(),
                ServiceRole::ProjectHost,
                "runtime project-host launch",
            )
            .await?;
        let descriptor = supervisor
            .resolve_project_host_descriptor(&self.session_slug)
            .await?;
        ensure_resolved_descriptor_matches_status(
            "runtime project-host launch",
            &self.session_slug,
            &expected,
            &descriptor,
        )?;
        let Some(session_id) = self.session_id else {
            return Err(EditorError::ServiceDiscovery(
                "runtime project-host reads require an attached session id".to_string(),
            ));
        };
        ProjectHostClient::connect_for_session(&descriptor, session_id).await
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn asset_processor_client_for_runtime_launch(
        &self,
    ) -> EditorResult<AssetProcessorClient> {
        #[cfg(test)]
        if self.supervisor.is_none() && self.supervisor_descriptor.is_none() {
            return self.asset_processor.clone().ok_or_else(|| {
                EditorError::ServiceDiscovery(
                    "asset-processor is required to resolve the runtime launch workspace snapshot"
                        .to_string(),
                )
            });
        }

        let supervisor = self
            .session_supervisor_client("runtime asset-processor launch")
            .await?;

        let expected = self
            .attached_service_descriptor_from_status(
                &supervisor,
                asset_processor_service_id(),
                ServiceRole::AssetProcessor,
                "runtime asset-processor launch",
            )
            .await?;
        let descriptor = supervisor
            .resolve_asset_processor_descriptor(&self.session_slug)
            .await?;
        ensure_resolved_descriptor_matches_status(
            "runtime asset-processor launch",
            &self.session_slug,
            &expected,
            &descriptor,
        )?;
        let Some(session_id) = self.session_id else {
            return Err(EditorError::ServiceDiscovery(
                "runtime asset-processor reads require an attached session id".to_string(),
            ));
        };
        AssetProcessorClient::connect_for_session(&descriptor, session_id).await
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn runtime_asset_package_roots_for_launch(
        &self,
    ) -> EditorResult<Vec<az_proto_runtime::RuntimeAssetPackageRoot>> {
        #[cfg(test)]
        if self.supervisor.is_none() && self.supervisor_descriptor.is_none() {
            return Ok(Vec::new());
        }

        let supervisor = self
            .session_supervisor_client("runtime package root resolution")
            .await?;

        let result = supervisor
            .runtime_asset_package_roots(&self.session_slug)
            .await?;
        Ok(result.roots)
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn optional_session_supervisor_client(
        &self,
        operation: &'static str,
        allow_missing: bool,
    ) -> EditorResult<Option<SessionSupervisorClient>> {
        #[cfg(test)]
        if let Some(supervisor) = &self.supervisor {
            return Ok(Some(supervisor.clone()));
        }

        let Some(descriptor) = &self.supervisor_descriptor else {
            if allow_missing {
                return Ok(None);
            }
            return Err(EditorError::ServiceDiscovery(format!(
                "{operation} requires a session-supervisor descriptor"
            )));
        };
        let Some(session_id) = self.session_id else {
            if allow_missing {
                return Ok(None);
            }
            return Err(EditorError::ServiceDiscovery(format!(
                "{operation} requires an attached session id"
            )));
        };
        Ok(Some(
            SessionSupervisorClient::connect_for_session(descriptor, session_id).await?,
        ))
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn session_supervisor_client(
        &self,
        operation: &'static str,
    ) -> EditorResult<SessionSupervisorClient> {
        self.optional_session_supervisor_client(operation, false)
            .await?
            .ok_or_else(|| {
                EditorError::ServiceDiscovery(format!(
                    "{operation} requires a session-supervisor descriptor"
                ))
            })
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn attached_service_descriptor_from_status(
        &self,
        supervisor: &SessionSupervisorClient,
        service_id: ServiceId,
        role: ServiceRole,
        operation: &'static str,
    ) -> EditorResult<ServiceDescriptor> {
        let status = supervisor.status(&self.session_slug).await?;
        self.ensure_supervisor_status_matches_controller(&status, operation)?;
        let descriptor =
            required_service_descriptor_from_status(&status.manifest, &service_id, role)?;
        Ok(descriptor)
    }

    fn validate_runtime_launch_workspace_snapshot(
        &self,
        snapshot: &WorkspaceSnapshot,
    ) -> EditorResult<Uuid> {
        let Some(session_id) = self.session_id else {
            return Err(EditorError::ServiceDiscovery(
                "runtime controller is missing stable session id".to_string(),
            ));
        };
        ensure_runtime_launch_workspace_snapshot(
            &self.session_slug,
            &self.project_id,
            &self.workspace.workspace_root,
            &self.workspace.branch,
            snapshot,
        )?;
        Ok(session_id)
    }
}

fn optional_service_descriptor_from_status<'a>(
    manifest: &'a ProtoSessionManifest,
    service_id: &ServiceId,
    role: ServiceRole,
) -> Option<&'a ServiceDescriptor> {
    manifest
        .services
        .iter()
        .find(|descriptor| descriptor.id == *service_id && descriptor.role == role)
}

fn required_service_descriptor_from_status(
    manifest: &ProtoSessionManifest,
    service_id: &ServiceId,
    role: ServiceRole,
) -> EditorResult<ServiceDescriptor> {
    optional_service_descriptor_from_status(manifest, service_id, role)
        .cloned()
        .ok_or_else(|| EditorError::MissingSessionService {
            session: manifest.slug.clone(),
            service: service_label(service_id),
        })
}

fn status_service_process_state(
    manifest: &ProtoSessionManifest,
    descriptor: &ServiceDescriptor,
) -> EditorResult<Option<ProtoServiceProcessState>> {
    let Some(process) = manifest.processes.iter().find(|process| {
        process.service_name == descriptor.id.name && process.role == descriptor.role
    }) else {
        return Ok(None);
    };
    if process.endpoint != descriptor.endpoint {
        return Err(EditorError::ServiceDiscovery(format!(
            "{} descriptor endpoint {:?} `{}` does not match running process endpoint {:?} `{}` for session `{}`",
            service_label(&descriptor.id),
            descriptor.endpoint.kind,
            descriptor.endpoint.address,
            process.endpoint.kind,
            process.endpoint.address,
            manifest.slug
        )));
    }
    Ok(Some(process.state))
}

fn status_service_process_is_running(
    manifest: &ProtoSessionManifest,
    descriptor: &ServiceDescriptor,
) -> EditorResult<bool> {
    Ok(status_service_process_state(manifest, descriptor)?
        == Some(ProtoServiceProcessState::Running))
}

fn ensure_resolved_descriptor_matches_status(
    operation: &'static str,
    session: &str,
    expected: &ServiceDescriptor,
    resolved: &ServiceDescriptor,
) -> EditorResult<()> {
    if resolved.has_same_connection_contract(expected) {
        return Ok(());
    }
    Err(runtime_session_mismatch(
        operation,
        session,
        format!(
            "{} descriptor from resolveService endpoint {:?} `{}` does not match status endpoint {:?} `{}`",
            service_label(&expected.id),
            resolved.endpoint.kind,
            resolved.endpoint.address,
            expected.endpoint.kind,
            expected.endpoint.address
        ),
    ))
}

fn runtime_session_mismatch(operation: &'static str, session: &str, reason: String) -> EditorError {
    EditorError::SessionDiscoveryMismatch {
        operation,
        session: session.to_string(),
        reason,
    }
}

fn same_protocol_path(left: impl AsRef<Path>, right: impl AsRef<Path>) -> bool {
    let left = comparable_protocol_path(left.as_ref());
    let right = comparable_protocol_path(right.as_ref());
    #[cfg(windows)]
    {
        comparable_protocol_path_text(&left) == comparable_protocol_path_text(&right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn comparable_protocol_path(path: &Path) -> PathBuf {
    let normalized = if path.exists() {
        path.canonicalize().map_or_else(
            |_| cli_compatible_protocol_path(path.to_path_buf()),
            cli_compatible_protocol_path,
        )
    } else {
        cli_compatible_protocol_path(path.to_path_buf())
    };
    strip_trailing_path_separators(normalized)
}

fn strip_trailing_path_separators(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    let stripped = text.trim_end_matches(['/', '\\']);
    if stripped.is_empty() {
        path
    } else {
        PathBuf::from(stripped)
    }
}

#[cfg(windows)]
fn comparable_protocol_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', r"\")
        .to_ascii_lowercase()
}

#[cfg(windows)]
fn cli_compatible_protocol_path(path: PathBuf) -> PathBuf {
    let path_text = path.to_string_lossy();
    if let Some(rest) = path_text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = path_text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

#[cfg(not(windows))]
fn cli_compatible_protocol_path(path: PathBuf) -> PathBuf {
    path
}

fn ensure_runtime_launch_workspace_snapshot(
    session_slug: &str,
    project_id: &str,
    workspace_path: &str,
    branch: &str,
    snapshot: &WorkspaceSnapshot,
) -> EditorResult<()> {
    ensure_runtime_launch_snapshot_identity(project_id, workspace_path, branch, snapshot)?;
    ensure_runtime_launch_source_roots(session_slug, snapshot)?;
    ensure_runtime_launch_project_assets_root(project_id, workspace_path, snapshot)
}

/// The snapshot must name this editor's project, workspace root, and branch,
/// and carry a coherent DB identity.
fn ensure_runtime_launch_snapshot_identity(
    project_id: &str,
    workspace_path: &str,
    branch: &str,
    snapshot: &WorkspaceSnapshot,
) -> EditorResult<()> {
    if snapshot.workspace_id <= 0 {
        return Err(EditorError::ServiceDiscovery(format!(
            "runtime launch workspace snapshot {} must carry a positive DB id",
            snapshot.workspace_id
        )));
    }
    if snapshot.project_id != project_id {
        return Err(EditorError::ServiceDiscovery(format!(
            "runtime launch workspace snapshot {} project `{}` does not match editor project `{}`",
            snapshot.workspace_id, snapshot.project_id, project_id
        )));
    }
    if !same_protocol_path(
        Path::new(&snapshot.workspace_root),
        Path::new(workspace_path),
    ) {
        return Err(EditorError::ServiceDiscovery(format!(
            "runtime launch workspace snapshot {} root `{}` does not match editor workspace `{}`",
            snapshot.workspace_id, snapshot.workspace_root, workspace_path
        )));
    }
    if snapshot.branch != branch {
        return Err(EditorError::ServiceDiscovery(format!(
            "runtime launch workspace snapshot {} branch `{}` does not match editor branch `{}`",
            snapshot.workspace_id, snapshot.branch, branch
        )));
    }
    if snapshot.created_unix_ms < 0 || snapshot.updated_unix_ms < 0 {
        return Err(EditorError::ServiceDiscovery(format!(
            "runtime launch workspace snapshot {} timestamps cannot be negative",
            snapshot.workspace_id
        )));
    }
    if snapshot.updated_unix_ms < snapshot.created_unix_ms {
        return Err(EditorError::ServiceDiscovery(format!(
            "runtime launch workspace snapshot {} was updated before it was created",
            snapshot.workspace_id
        )));
    }
    Ok(())
}

/// Every asset source root must belong to this snapshot, be fully identified,
/// and appear exactly once by DB id and by portable key.
fn ensure_runtime_launch_source_roots(
    session_slug: &str,
    snapshot: &WorkspaceSnapshot,
) -> EditorResult<()> {
    if snapshot.roots.is_empty() {
        return Err(EditorError::MissingRuntimeAssetSourceRoots {
            session: session_slug.to_string(),
            workspace_id: snapshot.workspace_id,
        });
    }
    let mut seen_source_root_ids = HashSet::new();
    let mut seen_portable_keys = HashSet::new();
    for root in &snapshot.roots {
        if root.workspace_id != snapshot.workspace_id {
            return Err(EditorError::ServiceDiscovery(format!(
                "runtime launch asset source root `{}` belongs to workspace snapshot {}, expected {}",
                root.portable_key, root.workspace_id, snapshot.workspace_id
            )));
        }
        if root.workspace_root_id <= 0 {
            return Err(EditorError::ServiceDiscovery(format!(
                "runtime launch asset source root `{}` must carry a positive DB workspace source root id",
                root.portable_key
            )));
        }
        if root.root_id <= 0 {
            return Err(EditorError::ServiceDiscovery(format!(
                "runtime launch asset source root `{}` must carry a positive DB scan folder id",
                root.portable_key
            )));
        }
        if root.declared_root_id.trim().is_empty() {
            return Err(EditorError::ServiceDiscovery(format!(
                "runtime launch asset source root `{}` has no declared root id",
                root.portable_key
            )));
        }
        if root.owner_id.trim().is_empty() {
            return Err(EditorError::ServiceDiscovery(
                "runtime launch asset source root owner id cannot be empty".to_string(),
            ));
        }
        if root.source_root.trim().is_empty() {
            return Err(EditorError::ServiceDiscovery(format!(
                "runtime launch asset source root `{}` physical root cannot be empty",
                root.portable_key
            )));
        }
        if root.portable_key.trim().is_empty() {
            return Err(EditorError::ServiceDiscovery(
                "runtime launch asset source root portable key cannot be empty".to_string(),
            ));
        }
        if !seen_source_root_ids.insert(root.workspace_root_id) {
            return Err(EditorError::ServiceDiscovery(format!(
                "runtime launch workspace snapshot {} returned duplicate source root id {}",
                snapshot.workspace_id, root.workspace_root_id
            )));
        }
        if !seen_portable_keys.insert(root.portable_key.as_str()) {
            return Err(EditorError::ServiceDiscovery(format!(
                "runtime launch workspace snapshot {} returned duplicate source root key `{}`",
                snapshot.workspace_id, root.portable_key
            )));
        }
    }
    Ok(())
}

/// The DB-owned project assets root must exist, be owned by this project, and
/// sit strictly inside the editor workspace with no output prefix.
fn ensure_runtime_launch_project_assets_root(
    project_id: &str,
    workspace_path: &str,
    snapshot: &WorkspaceSnapshot,
) -> EditorResult<()> {
    let project_assets_key = runtime_project_assets_root_portable_key(project_id);
    let project_assets = snapshot
        .roots
        .iter()
        .find(|root| root.portable_key == project_assets_key)
        .ok_or_else(|| {
            EditorError::ServiceDiscovery(format!(
                "runtime launch workspace snapshot {} is missing DB-owned project assets root `{project_assets_key}`",
                snapshot.workspace_id
            ))
        })?;
    if project_assets.owner_id != project_id {
        return Err(EditorError::ServiceDiscovery(format!(
            "runtime launch project assets root `{project_assets_key}` owner `{}` does not match editor project `{}`",
            project_assets.owner_id, project_id
        )));
    }
    let project_assets_path = PathBuf::from(&project_assets.source_root);
    let workspace_path = PathBuf::from(workspace_path);
    if project_assets_path == workspace_path || !project_assets_path.starts_with(&workspace_path) {
        return Err(EditorError::ServiceDiscovery(format!(
            "runtime launch project assets root `{project_assets_key}` path `{}` must be inside editor workspace `{}`",
            project_assets.source_root,
            workspace_path.display()
        )));
    }
    if !project_assets.is_root {
        return Err(EditorError::ServiceDiscovery(format!(
            "runtime launch project assets root `{project_assets_key}` must be marked as a root source"
        )));
    }
    if !project_assets.output_prefix.is_empty() {
        return Err(EditorError::ServiceDiscovery(format!(
            "runtime launch project assets root `{project_assets_key}` must use an empty output prefix"
        )));
    }

    Ok(())
}

fn runtime_project_assets_root_portable_key(project_id: &str) -> String {
    format!("project:{project_id}:assets")
}

fn runtime_asset_source_roots(snapshot: &WorkspaceSnapshot) -> Vec<RuntimeAssetSourceRoot> {
    snapshot
        .roots
        .iter()
        .map(|root| RuntimeAssetSourceRoot {
            workspace_root_id: root.workspace_root_id,
            workspace_id: root.workspace_id,
            root_id: root.root_id,
            owner_id: root.owner_id.clone(),
            source_root: root.source_root.clone(),
            display_name: root.display_name.clone(),
            portable_key: root.portable_key.clone(),
            output_prefix: root.output_prefix.clone(),
            is_root: root.is_root,
        })
        .collect()
}

pub fn publish_runtime_viewport_frame(cx: &mut App, frame: &RuntimeViewportFrame) {
    let Some(texture) = cx.try_global::<SharedViewportTexture>().cloned() else {
        return;
    };
    if !matches!(frame.color.kind, SideChannelKind::GpuSurface) {
        error!(
            runtime_id = %frame.runtime_id,
            side_channel = side_channel_kind_label(frame.color.kind),
            "runtime viewport frame rejected because production viewport import requires a GPU surface side-channel"
        );
    }
    clear_editor_viewport_gpu_texture(cx);
    match texture.publish_frame_source(
        frame.width,
        frame.height,
        ViewportFrameSource {
            kind: side_channel_kind_label(frame.color.kind).to_string(),
            locator: frame.color.locator.clone(),
            platform: frame.color.platform.clone(),
            byte_length: frame.color.byte_length,
            pixel_format: viewport_pixel_format_label(frame.format).to_string(),
        },
    ) {
        Ok(frame_revision) => {
            cx.set_global(editor_viewport_render_status_from_frame(
                frame,
                frame_revision,
            ));
        }
        Err(error) => {
            error!(
                error = %error,
                runtime_id = %frame.runtime_id,
                "failed to publish runtime viewport frame to editor UI"
            );
        }
    }
}

fn editor_viewport_render_status_from_frame(
    frame: &RuntimeViewportFrame,
    frame_revision: u64,
) -> EditorViewportRenderStatus {
    let format = viewport_pixel_format_label(frame.format);
    if matches!(frame.color.kind, SideChannelKind::GpuSurface) {
        EditorViewportRenderStatus::gpu_surface_handle(
            frame_revision,
            frame.width,
            frame.height,
            format,
            side_channel_kind_label(frame.color.kind),
        )
    } else {
        EditorViewportRenderStatus::failed(
            frame_revision,
            frame.width,
            frame.height,
            format,
            format!(
                "runtime viewport requires gpuSurface side-channel, got {}",
                side_channel_kind_label(frame.color.kind)
            ),
        )
    }
}

fn clear_editor_viewport_gpu_texture(cx: &mut App) {
    if cx.try_global::<EditorGpuState>().is_some() {
        cx.update_global::<EditorGpuState, _>(|state, _| {
            state.clear_viewport_texture();
        });
    }
}

const fn side_channel_kind_label(kind: az_proto_core::SideChannelKind) -> &'static str {
    match kind {
        SideChannelKind::SharedMemory => "sharedMemory",
        SideChannelKind::CasBlob => "casBlob",
        SideChannelKind::StagingFile => "stagingFile",
        SideChannelKind::GpuSurface => "gpuSurface",
        SideChannelKind::MmapFile => "mmapFile",
    }
}

const fn viewport_pixel_format_label(format: ViewportPixelFormat) -> &'static str {
    match format {
        ViewportPixelFormat::Unknown => "unknown",
        ViewportPixelFormat::Bgra8Unorm => "bgra8Unorm",
        ViewportPixelFormat::Rgba8Unorm => "rgba8Unorm",
        ViewportPixelFormat::Rgba16Float => "rgba16Float",
        ViewportPixelFormat::Depth32Float => "depth32Float",
    }
}

pub fn runtime_projection_catalog_to_ui(
    catalog: RuntimeProjectionCatalogResult,
) -> EditorRuntimeProjectionCatalog {
    EditorRuntimeProjectionCatalog::new(
        catalog
            .projections
            .into_iter()
            .map(runtime_projection_to_ui)
            .collect(),
    )
}

fn runtime_projection_to_ui(projection: RuntimeProjectionDescriptor) -> RuntimeProjectionData {
    RuntimeProjectionData {
        name: projection.name,
        priority: projection.priority,
        roles: projection
            .roles
            .into_iter()
            .map(runtime_role_label)
            .map(str::to_string)
            .collect(),
        launch_profiles: projection.launch_profiles,
    }
}

#[must_use]
pub fn runtime_status_to_ui(
    runtime_id: impl Into<String>,
    status: Option<RuntimeStatus>,
) -> EditorRuntimeStatus {
    match status {
        Some(status) => EditorRuntimeStatus {
            runtime_id: status.runtime_id,
            state: runtime_state_to_ui(status.state),
            role: Some(runtime_role_label(status.role).to_string()),
            project_id: Some(status.project_id),
            session_slug: Some(status.session_slug),
            authored_revision: Some(status.authored_revision),
            diagnostics: runtime_diagnostics_to_ui(&status.diagnostic),
        },
        None => EditorRuntimeStatus::unregistered(runtime_id),
    }
}

const fn runtime_state_to_ui(state: RuntimeState) -> EditorRuntimeStateData {
    match state {
        RuntimeState::Stopped => EditorRuntimeStateData::Stopped,
        RuntimeState::Starting => EditorRuntimeStateData::Starting,
        RuntimeState::Running => EditorRuntimeStateData::Running,
        RuntimeState::Failed => EditorRuntimeStateData::Failed,
    }
}

fn runtime_diagnostics_to_ui(diagnostic: &str) -> Vec<String> {
    diagnostic
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

const fn runtime_role_label(role: RuntimeRole) -> &'static str {
    match role {
        RuntimeRole::EditorWorld => "editor-world",
        RuntimeRole::PlayPreview => "play-preview",
        RuntimeRole::ServerPreview => "server-preview",
        RuntimeRole::Validation => "validation",
        RuntimeRole::Thumbnail => "thumbnail",
        RuntimeRole::Bake => "bake",
    }
}

pub(crate) fn install_runtime_slot(
    cx: &mut App,
    session: EditorAttachSession,
    fence: crate::controller_set::ControllerFence,
) {
    if session.services.runtime_host.is_none() {
        let _ = crate::controller_set::mark_runtime_unavailable(cx, fence);
        return;
    }
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "runtime-install",
        move || async move {
            let Some(controller) = EditorRuntimeController::connect_attached(&session).await?
            else {
                return Ok(None);
            };
            let status = match controller.editor_world_status().await {
                Ok(status) => status,
                Err(error) => {
                    error!(error = %error, "failed to query editor-world runtime status");
                    None
                }
            };
            let viewport_frame = match controller.editor_world_viewport_frame().await {
                Ok(frame) => frame,
                Err(error) => {
                    error!(
                        error = %error,
                        "failed to query editor-world viewport frame side channel"
                    );
                    None
                }
            };
            let projection_catalog = match controller.load_projection_catalog().await {
                Ok(catalog) => catalog,
                Err(error) => {
                    error!(
                        error = %error,
                        "failed to query runtime projection catalog"
                    );
                    EditorRuntimeProjectionCatalog::default()
                }
            };
            Ok(Some((
                controller,
                status,
                viewport_frame,
                projection_catalog,
            )))
        },
        move |cx, result| match result {
            Ok(Some((controller, status, viewport_frame, projection_catalog))) => {
                let runtime_registered = status.is_some();
                let runtime_status = runtime_status_to_ui(EDITOR_WORLD_RUNTIME_ID, status);
                let projection_count = projection_catalog.projections.len();
                if !crate::controller_set::complete_runtime(cx, fence, controller) {
                    return;
                }
                if let Some(frame) = &viewport_frame {
                    publish_runtime_viewport_frame(cx, frame);
                }
                cx.set_global(runtime_status);
                cx.set_global(projection_catalog);
                info!(
                    runtime_registered,
                    viewport_frame_registered = viewport_frame.is_some(),
                    projection_count,
                    "installed editor runtime controller"
                );
            }
            Ok(None) => {
                let _ = crate::controller_set::mark_runtime_unavailable(cx, fence);
            }
            Err(error) => {
                crate::controller_set::fail_controller(cx, fence, error.to_string());
                error!(error = %error, "failed to install editor runtime controller");
            }
        },
    );
}

pub fn install_runtime_action_handlers(cx: &mut App) {
    cx.on_action(|_: &az_editor_ui::actions::LaunchEditorWorld, cx| {
        if let Err(error) = launch_editor_world_from_action(cx) {
            error!(error = %error, "failed to handle editor-world launch action");
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::RefreshRuntimeStatus, cx| {
        if let Err(error) = refresh_editor_world_status_from_action(cx) {
            error!(error = %error, "failed to handle editor-world status refresh action");
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::RefreshViewportFrame, cx| {
        if let Err(error) = refresh_editor_world_viewport_frame_from_action(cx) {
            error!(error = %error, "failed to handle viewport frame refresh action");
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::RefreshRuntimeProjections, cx| {
        if let Err(error) = refresh_runtime_projection_catalog_from_action(cx) {
            error!(error = %error, "failed to handle runtime projection catalog refresh action");
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::StopEditorWorld, cx| {
        if let Err(error) = stop_editor_world_from_action(cx, action.preserve) {
            error!(error = %error, preserve = action.preserve, "failed to handle editor-world stop action");
        }
    });
}

fn runtime_controller(
    cx: &App,
) -> EditorResult<crate::controller_set::AttachedController<EditorRuntimeController>> {
    crate::controller_set::runtime_controller(cx)
}

/// Spawn an editor-world launch, then publish its status, viewport frame, and
/// projection catalog into the app globals.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] when no editor controller
/// set is installed, or [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or [`EditorError::ControllerUnavailable`]
/// when the runtime slot is not ready. The spawned launch reports its own
/// failures through the log, not through this call.
pub fn launch_editor_world_from_action(cx: &mut App) -> EditorResult<()> {
    let attached = runtime_controller(cx)?;
    let fence = attached.fence;
    let controller = attached.controller;
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "runtime-launch-editor-world",
        move || async move {
            let status = controller.launch_attached_editor_world(true).await?;
            let viewport_frame = match controller.editor_world_viewport_frame().await {
                Ok(frame) => frame,
                Err(error) => {
                    error!(
                        error = %error,
                        runtime_id = %status.runtime_id,
                        "failed to query viewport frame after editor-world launch"
                    );
                    None
                }
            };
            let projection_catalog = match controller.load_projection_catalog().await {
                Ok(catalog) => catalog,
                Err(error) => {
                    error!(
                        error = %error,
                        runtime_id = %status.runtime_id,
                        "failed to refresh runtime projection catalog after editor-world launch"
                    );
                    EditorRuntimeProjectionCatalog::default()
                }
            };
            Ok((status, viewport_frame, projection_catalog))
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((status, viewport_frame, projection_catalog)) => {
                    let projection_count = projection_catalog.projections.len();
                    let runtime_status =
                        runtime_status_to_ui(EDITOR_WORLD_RUNTIME_ID, Some(status.clone()));
                    if let Some(frame) = &viewport_frame {
                        publish_runtime_viewport_frame(cx, frame);
                    }
                    cx.set_global(runtime_status);
                    cx.set_global(projection_catalog);
                    cx.refresh_windows();
                    info!(
                        runtime_id = %status.runtime_id,
                        projection_count,
                        "editor-world runtime launched"
                    );
                }
                Err(error) => {
                    error!(error = %error, "failed to launch editor-world runtime");
                }
            }
        },
    );
    Ok(())
}

/// Spawn a refresh of the editor-world runtime status global.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] when no editor controller
/// set is installed, or [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or [`EditorError::ControllerUnavailable`]
/// when the runtime slot is not ready. The spawned refresh reports its own
/// failures through the log, not through this call.
pub fn refresh_editor_world_status_from_action(cx: &mut App) -> EditorResult<()> {
    let attached = runtime_controller(cx)?;
    let fence = attached.fence;
    let controller = attached.controller;
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "runtime-refresh-status",
        move || async move { controller.editor_world_status().await },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(status) => {
                    let runtime_registered = status.is_some();
                    let runtime_status = runtime_status_to_ui(EDITOR_WORLD_RUNTIME_ID, status);
                    cx.set_global(runtime_status);
                    cx.refresh_windows();
                    info!(
                        runtime_id = %EDITOR_WORLD_RUNTIME_ID,
                        runtime_registered,
                        "refreshed editor-world runtime status"
                    );
                }
                Err(error) => {
                    error!(error = %error, "failed to refresh editor-world runtime status");
                }
            }
        },
    );
    Ok(())
}

/// Spawn a refresh of the runtime projection catalog global.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] when no editor controller
/// set is installed, or [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or [`EditorError::ControllerUnavailable`]
/// when the runtime slot is not ready. The spawned refresh reports its own
/// failures through the log, not through this call.
pub fn refresh_runtime_projection_catalog_from_action(cx: &mut App) -> EditorResult<()> {
    let attached = runtime_controller(cx)?;
    let fence = attached.fence;
    let controller = attached.controller;
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "runtime-refresh-projections",
        move || async move { controller.load_projection_catalog().await },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(catalog) => {
                    let projection_count = catalog.projections.len();
                    cx.set_global(catalog);
                    cx.refresh_windows();
                    info!(
                        runtime_id = %EDITOR_WORLD_RUNTIME_ID,
                        projection_count,
                        "refreshed runtime projection catalog"
                    );
                }
                Err(error) => {
                    error!(error = %error, "failed to refresh runtime projection catalog");
                }
            }
        },
    );
    Ok(())
}

/// Spawn a refresh of the editor-world viewport frame.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] when no editor controller
/// set is installed, or [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or [`EditorError::ControllerUnavailable`]
/// when the runtime slot is not ready. The spawned refresh reports its own
/// failures through the log, not through this call.
pub fn refresh_editor_world_viewport_frame_from_action(cx: &mut App) -> EditorResult<()> {
    let attached = runtime_controller(cx)?;
    let fence = attached.fence;
    let controller = attached.controller;
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "runtime-refresh-viewport-frame",
        move || async move { controller.editor_world_viewport_frame().await },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(Some(frame)) => {
                    publish_runtime_viewport_frame(cx, &frame);
                    cx.refresh_windows();
                }
                Ok(None) => {
                    info!("editor-world runtime has no viewport frame yet");
                }
                Err(error) => {
                    error!(error = %error, "failed to refresh editor-world viewport frame");
                }
            }
        },
    );
    Ok(())
}

/// Spawn a stop of the editor-world runtime and publish the resulting status.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] when no editor controller
/// set is installed, or [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or [`EditorError::ControllerUnavailable`]
/// when the runtime slot is not ready. The spawned stop reports its own
/// failures through the log, not through this call.
pub fn stop_editor_world_from_action(cx: &mut App, preserve: bool) -> EditorResult<()> {
    let attached = runtime_controller(cx)?;
    let fence = attached.fence;
    let controller = attached.controller;
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "runtime-stop-editor-world",
        move || async move { controller.stop_editor_world(preserve).await },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(status) => {
                    let runtime_status =
                        runtime_status_to_ui(EDITOR_WORLD_RUNTIME_ID, Some(status.clone()));
                    cx.set_global(runtime_status);
                    cx.refresh_windows();
                    info!(
                        runtime_id = %status.runtime_id,
                        preserve,
                        "editor-world runtime stopped"
                    );
                }
                Err(error) => {
                    error!(error = %error, preserve, "failed to stop editor-world runtime");
                }
            }
        },
    );
    Ok(())
}

fn validate_runtime_host_descriptor(descriptor: &ServiceDescriptor) -> EditorResult<()> {
    let expected = runtime_host_service_id();
    if descriptor.id != expected || descriptor.role != ServiceRole::RuntimeHost {
        return Err(crate::error::EditorError::ServiceDiscovery(format!(
            "expected runtime-host descriptor `{}`/`{}` with role {:?}, got `{}`/`{}` with role {:?}",
            expected.namespace,
            expected.name,
            ServiceRole::RuntimeHost,
            descriptor.id.namespace,
            descriptor.id.name,
            descriptor.role
        )));
    }
    validate_descriptor_capability_templates(descriptor, "runtime-host")?;
    Ok(())
}

fn validate_runtime_host_descriptor_for_session(
    descriptor: &ServiceDescriptor,
    session_id: Uuid,
) -> EditorResult<()> {
    validate_runtime_host_descriptor(descriptor)?;
    validate_session_descriptor_capability_templates(descriptor, "runtime-host", session_id)
}

fn validate_runtime_viewport_frame_side_channel_capability(
    frame: &RuntimeViewportFrame,
    expected: &Capability,
) -> EditorResult<()> {
    Ok(validate_side_channel_capability_matches(
        &frame.color,
        expected,
        "runtime viewport frame",
    )?)
}

fn ensure_runtime_status_matches_request(
    status: &RuntimeStatus,
    expected_runtime_id: &str,
    expected_role: Option<RuntimeRole>,
    operation: &'static str,
) -> EditorResult<()> {
    if status.runtime_id != expected_runtime_id {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime-host returned runtime `{}`, expected `{}`",
                status.runtime_id, expected_runtime_id
            ),
        ));
    }

    if status.runtime_id.trim().is_empty() {
        return Err(runtime_host_authority_mismatch(
            operation,
            "runtime status id cannot be empty".to_string(),
        ));
    }

    if let Some(expected_role) = expected_role
        && status.role != expected_role
    {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` reported role {:?}, expected {:?}",
                status.runtime_id, status.role, expected_role
            ),
        ));
    }

    if status.project_id.trim().is_empty() || status.session_slug.trim().is_empty() {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` status project/session identity cannot be empty",
                status.runtime_id
            ),
        ));
    }

    Ok(())
}

fn ensure_runtime_viewport_frame_matches_request(
    frame: &RuntimeViewportFrame,
    expected_runtime_id: &str,
    operation: &'static str,
) -> EditorResult<()> {
    if frame.runtime_id != expected_runtime_id {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime-host returned viewport frame for runtime `{}`, expected `{}`",
                frame.runtime_id, expected_runtime_id
            ),
        ));
    }

    if frame.runtime_id.trim().is_empty() {
        return Err(runtime_host_authority_mismatch(
            operation,
            "runtime viewport frame id cannot be empty".to_string(),
        ));
    }

    if frame.width == 0 || frame.height == 0 {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport dimensions must be non-zero, got {}x{}",
                frame.runtime_id, frame.width, frame.height
            ),
        ));
    }

    let bytes_per_pixel = frame.format.bytes_per_pixel().ok_or_else(|| {
        runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport pixel format cannot be unknown",
                frame.runtime_id
            ),
        )
    })?;
    let min_row_pitch = frame.width.checked_mul(bytes_per_pixel).ok_or_else(|| {
        runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport row pitch overflow for {} pixels",
                frame.runtime_id, frame.width
            ),
        )
    })?;
    if frame.row_pitch < min_row_pitch {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport row pitch {} is smaller than minimum {}",
                frame.runtime_id, frame.row_pitch, min_row_pitch
            ),
        ));
    }

    let min_byte_length = u64::from(frame.row_pitch)
        .checked_mul(u64::from(frame.height))
        .ok_or_else(|| {
            runtime_host_authority_mismatch(
                operation,
                format!(
                    "runtime `{}` viewport byte length overflow for row pitch {} and height {}",
                    frame.runtime_id, frame.row_pitch, frame.height
                ),
            )
        })?;
    if frame.color.byte_length < min_byte_length {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport side channel length {} is smaller than minimum {}",
                frame.runtime_id, frame.color.byte_length, min_byte_length
            ),
        ));
    }

    if frame.color.locator.trim().is_empty() || frame.color.platform.trim().is_empty() {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport side-channel locator/platform cannot be empty",
                frame.runtime_id
            ),
        ));
    }

    match frame.color.kind {
        SideChannelKind::GpuSurface => {}
        kind => {
            return Err(runtime_host_authority_mismatch(
                operation,
                format!(
                    "runtime `{}` viewport frames must use gpuSurface side channels, got {}",
                    frame.runtime_id,
                    side_channel_kind_label(kind),
                ),
            ));
        }
    }

    Ok(())
}

fn ensure_runtime_projection_catalog_matches_request(
    result: &RuntimeProjectionCatalogResult,
) -> EditorResult<()> {
    let mut seen_names = HashSet::new();
    for projection in &result.projections {
        ensure_runtime_projection_descriptor_matches_request(projection)?;
        if !seen_names.insert(projection.name.as_str()) {
            return Err(runtime_host_authority_mismatch(
                "projectionCatalog",
                format!(
                    "runtime-host returned duplicate projection `{}`",
                    projection.name
                ),
            ));
        }
    }
    Ok(())
}

fn ensure_runtime_projection_descriptor_matches_request(
    projection: &RuntimeProjectionDescriptor,
) -> EditorResult<()> {
    if projection.name.trim().is_empty() {
        return Err(runtime_host_authority_mismatch(
            "projectionCatalog",
            "runtime projection name cannot be empty".to_string(),
        ));
    }

    let mut seen_profiles = HashSet::new();
    for profile in &projection.launch_profiles {
        if profile.trim().is_empty() {
            return Err(runtime_host_authority_mismatch(
                "projectionCatalog",
                format!(
                    "runtime projection `{}` launch profile cannot be empty",
                    projection.name
                ),
            ));
        }
        if !seen_profiles.insert(profile.as_str()) {
            return Err(runtime_host_authority_mismatch(
                "projectionCatalog",
                format!(
                    "runtime projection `{}` returned duplicate launch profile `{}`",
                    projection.name, profile
                ),
            ));
        }
    }

    Ok(())
}

const fn runtime_host_authority_mismatch(operation: &'static str, reason: String) -> EditorError {
    EditorError::ServiceAuthorityMismatch {
        service: "runtime-host",
        operation,
        reason,
    }
}

#[must_use]
pub fn runtime_host_service_id() -> ServiceId {
    ServiceId::new(RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME)
}

fn project_host_service_id() -> ServiceId {
    ServiceId::new(PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME)
}

fn asset_processor_service_id() -> ServiceId {
    ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME)
}

fn service_label(id: &ServiceId) -> String {
    format!("{}/{}", id.namespace, id.name)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use az_editor_ui::panels::EditorViewportRenderStateData;
    use az_project_host::{ProjectHost, ProjectHostRpc};
    use az_proto_asset::{ASSET_READ_PERMISSION, asset_capnp};
    use az_proto_core::{
        CapabilityGrantSet, Endpoint, EndpointKind, SideChannelHandle, SideChannelKind,
    };
    use az_proto_project::{
        PROJECT_DOCUMENT_READ_PERMISSION, PROJECT_DOCUMENT_WRITE_PERMISSION,
        PROJECT_EDIT_PERMISSION, PROJECT_HOST_AUDIENCE, PROJECT_RUNTIME_LAUNCH_PERMISSION,
        PROJECT_SCHEMA_PERMISSION,
    };
    use az_proto_runtime::{
        RuntimeAssetSourceRoot, RuntimeLaunchSnapshot, RuntimeProjectionCatalogResult,
        RuntimeProjectionDescriptor, RuntimeState, ViewportPixelFormat,
        encode_runtime_launch_snapshot,
    };
    use az_runtime_host::{
        RuntimeHost, RuntimeHostRpc, RuntimeHostRpcServer, RuntimeProjection,
        RuntimeProjectionError, RuntimeProjectionLaunchContext, RuntimeProjectionRegistration,
        RuntimeProjectionUpdate, start_runtime_host_rpc_server_for_session_with_capability_grants,
    };

    use super::*;
    use az_filesystem::AzothDataHome;
    use az_service_catalog::{project_host_service_descriptor, runtime_host_service_descriptor};
    use az_service_supervision::{
        ProcessIdentity, ServiceProcessKey, ServiceProcessRecord, SupervisedServiceRole,
    };
    use az_session::{
        CreateSessionRequest, SESSION_MANIFEST_FILE, SessionId, SessionManager, SessionManifest,
        SessionServiceLaunchCommand, SessionState, SessionSupervisorRpc,
    };

    fn write_test_project_manifest(root: &std::path::Path, manifest: &az_project::ProjectManifest) {
        az_project::write_project_manifest(root, manifest).unwrap();
        az_project::refresh_project_lock(root).unwrap();
    }

    #[test]
    fn runtime_host_launch_uses_terminal_start_status_without_polling() {
        let source = include_str!("runtime_host.rs");
        let launch_source = source
            .split("async fn runtime_host_client_for_launch(")
            .nth(1)
            .expect("find runtime_host_client_for_launch")
            .split("async fn runtime_host_descriptor_from_supervisor(")
            .next()
            .expect("find runtime_host_descriptor_from_supervisor after launch function");

        assert!(
            az_architecture_guard::symbols_contain(launch_source, "start_services_for("),
            "runtime-host launch should still request startup through the session supervisor"
        );
        assert_eq!(
            az_architecture_guard::symbols_count(
                launch_source,
                r"runtime_host_descriptor_from_supervisor(&supervisor, _)",
            ),
            1,
            "runtime-host launch should inspect status once before requesting a terminal start"
        );
        assert!(
            az_architecture_guard::symbols_contain(launch_source, "&result.status"),
            "runtime-host launch must consume the terminal start snapshot"
        );
        assert!(
            az_architecture_guard::symbols_contain(
                launch_source,
                "status_service_process_is_running(&result.status.manifest, expected)"
            ),
            "runtime-host launch must require the terminal snapshot to report a running process"
        );
        assert!(
            !az_architecture_guard::symbols_contain(launch_source, "sleep("),
            "runtime-host launch must not poll after a terminal start result"
        );
    }

    #[test]
    fn runtime_service_resolution_uses_status_before_resolve_service() {
        let source = include_str!("runtime_host.rs");
        let runtime_descriptor_source = source
            .split("async fn runtime_host_descriptor_from_supervisor(")
            .nth(1)
            .expect("find runtime_host_descriptor_from_supervisor")
            .split("fn ensure_supervisor_status_matches_controller(")
            .next()
            .expect("find status validator after runtime descriptor helper");
        assert!(
            az_architecture_guard::symbols_contain(
                runtime_descriptor_source,
                "supervisor.status(&self.session_slug).await?"
            ),
            "runtime-host descriptor resolution must read live SessionSupervisor.status"
        );
        assert!(
            az_architecture_guard::symbols_contain(
                runtime_descriptor_source,
                "status_service_process_is_running"
            ),
            "runtime-host descriptor resolution must require a running process record before connect"
        );

        for (function_name, resolve_call) in [
            (
                "project_host_client_for_runtime_launch",
                ".resolve_project_host_descriptor(&self.session_slug)",
            ),
            (
                "asset_processor_client_for_runtime_launch",
                ".resolve_asset_processor_descriptor(&self.session_slug)",
            ),
        ] {
            let helper_source = source
                .split(&format!("async fn {function_name}("))
                .nth(1)
                .unwrap_or_else(|| panic!("find {function_name}"))
                .split("    fn validate_runtime_launch_workspace_snapshot(")
                .next()
                .unwrap_or_else(|| panic!("find end of {function_name}"));
            assert!(
                az_architecture_guard::symbols_contain(
                    helper_source,
                    "attached_service_descriptor_from_status"
                ),
                "{function_name} must validate the project attachment before resolveService"
            );
            let helper_skeleton = az_architecture_guard::symbol_skeleton(helper_source);
            let validate_at = helper_skeleton
                .find(&az_architecture_guard::symbol_skeleton(
                    "attached_service_descriptor_from_status",
                ))
                .expect("attachment validation must exist");
            let resolve_at = helper_skeleton
                .find(&az_architecture_guard::symbol_skeleton(resolve_call))
                .unwrap_or_else(|| panic!("{function_name} must compare resolveService"));
            assert!(
                validate_at < resolve_at,
                "{function_name} must compare resolveService against the status descriptor"
            );
        }
    }

    fn asset_read_grant() -> Capability {
        Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_audience(az_proto_asset::ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_READ_PERMISSION])
        .with_token_hash([0xea, 0xad])
    }

    struct SnapshotAssetProcessor {
        snapshot: Option<WorkspaceSnapshot>,
    }

    impl asset_capnp::asset_processor::Server for SnapshotAssetProcessor {
        #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
        async fn workspace_snapshot(
            self: capnp::capability::Rc<Self>,
            params: asset_capnp::asset_processor::WorkspaceSnapshotParams,
            mut results: asset_capnp::asset_processor::WorkspaceSnapshotResults,
        ) -> Result<(), capnp::Error> {
            let _request =
                az_proto_asset::WorkspaceSnapshotRequest::from_capnp(params.get()?.get_request()?)?;
            az_proto_asset::WorkspaceSnapshotResult {
                snapshot: self.snapshot.clone(),
            }
            .to_capnp(results.get().init_result())
        }
    }

    fn asset_client_with_workspace_snapshot(
        snapshot: WorkspaceSnapshot,
        session_id: Uuid,
    ) -> AssetProcessorClient {
        let client: asset_capnp::asset_processor::Client =
            capnp_rpc::new_client(SnapshotAssetProcessor {
                snapshot: Some(snapshot),
            });
        AssetProcessorClient::new(client)
            .with_session_scope(session_id)
            .with_capability_templates(vec![asset_read_grant()])
    }

    fn project_host_grant_for_session(session_id: Uuid) -> Capability {
        Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_audience(PROJECT_HOST_AUDIENCE)
        .with_session(session_id)
        .with_permissions([
            PROJECT_SCHEMA_PERMISSION,
            PROJECT_EDIT_PERMISSION,
            PROJECT_DOCUMENT_READ_PERMISSION,
            PROJECT_DOCUMENT_WRITE_PERMISSION,
            PROJECT_RUNTIME_LAUNCH_PERMISSION,
        ])
        .with_token_hash([0x70, 0x48])
    }

    fn project_host_grants_for_session(session_id: Uuid) -> CapabilityGrantSet {
        CapabilityGrantSet::from_grants(vec![project_host_grant_for_session(session_id)])
    }

    fn project_host_rpc_for_session(
        host: ProjectHost,
        _root: impl Into<std::path::PathBuf>,
        session_id: Uuid,
    ) -> ProjectHostRpc {
        ProjectHostRpc::test_new(host, project_host_grants_for_session(session_id))
    }

    fn project_host_client_from_rpc_for_session(
        rpc: &Rc<ProjectHostRpc>,
        session_id: Uuid,
    ) -> ProjectHostClient {
        ProjectHostClient::new(ProjectHostRpc::client_from_rc(rpc))
            .with_session_scope(session_id)
            .with_capability_templates(vec![project_host_grant_for_session(session_id)])
    }

    fn runtime_read_grant(session_id: Uuid) -> Capability {
        Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_session(session_id)
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_READ_PERMISSION])
        .with_token_hash([0x72, 0x48])
    }

    fn runtime_control_grant(session_id: Uuid) -> Capability {
        Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_session(session_id)
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_CONTROL_PERMISSION])
        .with_token_hash([0x72, 0x49])
    }

    fn project_host_runtime_control_grant(session_id: Uuid) -> Capability {
        Capability::new(
            ServiceId::new(PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
        )
        .with_session(session_id)
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_CONTROL_PERMISSION])
        .with_token_hash([0x35])
    }

    fn runtime_host_grants(session_id: Uuid) -> CapabilityGrantSet {
        let grants = vec![
            runtime_read_grant(session_id),
            runtime_control_grant(session_id),
            project_host_runtime_control_grant(session_id),
        ];
        CapabilityGrantSet::from_grants(grants)
    }

    fn runtime_host_rpc(session_id: Uuid) -> RuntimeHostRpc {
        RuntimeHostRpc::test_new(RuntimeHost::new(
            session_id.to_string(),
            runtime_host_grants(session_id),
            std::env::temp_dir().join(format!("az-editor-runtime-host-test-{session_id}")),
            composed(),
        ))
    }

    fn runtime_host_client_from_rpc(
        rpc: &Rc<RuntimeHostRpc>,
        session_id: Uuid,
    ) -> RuntimeHostClient {
        RuntimeHostClient::new(RuntimeHostRpc::client_from_rc(rpc))
            .with_capability_templates(vec![
                runtime_read_grant(session_id),
                runtime_control_grant(session_id),
                project_host_runtime_control_grant(session_id),
            ])
            .with_session_scope(session_id)
    }

    fn start_descriptor_runtime_host(
        session_id: SessionId,
    ) -> (ServiceDescriptor, RuntimeHostRpcServer) {
        let run = Uuid::now_v7();
        let mut descriptor = runtime_host_service_descriptor(
            session_id.0,
            run,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        let server = start_runtime_host_rpc_server_for_session_with_capability_grants(
            descriptor.endpoint.clone(),
            session_id.0.to_string(),
            std::env::temp_dir().join(format!(
                "az-editor-runtime-host-test-{}-{run}",
                session_id.0,
            )),
            CapabilityGrantSet::from_grants(descriptor.capabilities.clone()),
            composed(),
        )
        .unwrap();
        descriptor.endpoint = server.endpoint().clone();
        (descriptor, server)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "test helper whose arguments are the loose fields of the ServiceProcessRecord it fabricates; a params struct would restate that record"
    )]
    fn add_running_service_process(
        manifest: &mut SessionManifest,
        descriptor: &ServiceDescriptor,
        service_name: &str,
        role: SupervisedServiceRole,
        owner_id: &str,
        owner_root: &Path,
        cwd: &Path,
        pid: u32,
    ) {
        let log_name = service_name.replace('-', "_");
        let mut process = ServiceProcessRecord::planned(
            service_name,
            role,
            descriptor.run,
            &descriptor.endpoint,
            service_name,
            cwd.to_path_buf(),
            Vec::new(),
            manifest.run_dir.join(format!("{log_name}.stdout.log")),
            manifest.run_dir.join(format!("{log_name}.stderr.log")),
            manifest.run_dir.join(format!("{log_name}.capnp.log")),
            None,
            1,
        );
        process.owner_id = owner_id.to_string();
        process.owner_root = owner_root.to_path_buf();
        process
            .mark_running(
                ProcessIdentity {
                    process_id: pid,
                    process_start_time: 9_001,
                },
                2,
            )
            .unwrap();
        manifest.upsert_process_record(process, 3);
    }

    /// The on-disk session layout the runtime-host controller tests share.
    struct RuntimeHostSessionFixture {
        session_id: SessionId,
        session_slug: String,
        project_id: String,
        project_root: PathBuf,
        workspace_path: PathBuf,
        run_dir: PathBuf,
    }

    /// Create the session's run and workspace directories and capture the
    /// identity every manifest and controller in these tests is built from.
    fn runtime_host_session_fixture(
        temp: &tempfile::TempDir,
        manager: &SessionManager,
        session_id: SessionId,
        session_slug: &str,
        project_id: &str,
    ) -> RuntimeHostSessionFixture {
        let run_dir = manager.sessions_dir().join(session_id.to_string());
        let workspace_path = temp.path().to_path_buf();
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::create_dir_all(&workspace_path).unwrap();
        RuntimeHostSessionFixture {
            session_id,
            session_slug: session_slug.to_string(),
            project_id: project_id.to_string(),
            project_root: temp.path().to_path_buf(),
            workspace_path,
            run_dir,
        }
    }

    /// Write an active session manifest that already advertises `descriptor`
    /// with a running runtime-host process, so a controller reading the
    /// manifest resolves that descriptor as the live one.
    fn write_running_runtime_host_manifest(
        fixture: &RuntimeHostSessionFixture,
        descriptor: &ServiceDescriptor,
        endpoint: &Endpoint,
    ) {
        let mut manifest = SessionManifest::new(
            fixture.session_id,
            fixture.project_id.clone(),
            fixture.session_slug.clone(),
            fixture.project_root.clone(),
            fixture.workspace_path.clone(),
            fixture.run_dir.clone(),
            0,
        );
        manifest.state = SessionState::Active;
        manifest.upsert_service_descriptor(descriptor, 1).unwrap();
        let mut process = ServiceProcessRecord::planned(
            RUNTIME_HOST_SERVICE_NAME,
            SupervisedServiceRole::RuntimeHost,
            descriptor.run,
            endpoint,
            "runtime-host",
            fixture.workspace_path.clone(),
            Vec::new(),
            fixture.run_dir.join("runtime-host.stdout.log"),
            fixture.run_dir.join("runtime-host.stderr.log"),
            fixture.run_dir.join("runtime-host.capnp.log"),
            None,
            1,
        );
        process.owner_id = fixture.project_id.clone();
        process.owner_root = fixture.project_root.clone();
        process
            .mark_running(ProcessIdentity::current().unwrap(), 2)
            .unwrap();
        manifest.upsert_process_record(process, 3);
        std::fs::write(
            fixture.run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();
    }

    /// Build the session-scoped controller shape these tests exercise: the
    /// attached identity from `fixture` plus the supplied service clients.
    fn session_scoped_runtime_controller(
        fixture: &RuntimeHostSessionFixture,
        project_host: ProjectHostClient,
        runtime_host: Option<RuntimeHostClient>,
        supervisor: Option<SessionSupervisorClient>,
    ) -> EditorRuntimeController {
        EditorRuntimeController {
            session_id: Some(fixture.session_id.0),
            project_id: fixture.project_id.clone(),
            session_slug: fixture.session_slug.clone(),
            project_root: fixture.project_root.to_string_lossy().into_owned(),
            workspace: AttachedWorkspace {
                project_id: fixture.project_id.clone(),
                workspace_root: fixture.workspace_path.to_string_lossy().into_owned(),
                branch: "main".to_string(),
            },
            run_dir: Some(fixture.run_dir.to_string_lossy().into_owned()),
            project_host,
            runtime_host,
            supervisor,
            supervisor_descriptor: None,
            asset_processor: None,
        }
    }

    /// Create every directory the launch test reads from, then write the real
    /// project manifest into the workspace and a decoy into the stale root.
    fn prepare_launch_test_project_tree(
        directories: &[&Path],
        workspace_path: &Path,
        stale_project_root: &Path,
        project_manifest: &az_project::ProjectManifest,
    ) {
        for directory in directories {
            std::fs::create_dir_all(directory).unwrap();
        }
        write_test_project_manifest(workspace_path, project_manifest);
        write_test_project_manifest(
            stale_project_root,
            &az_project::ProjectManifest::new(
                "local.stale_editor_runtime",
                "Stale Editor Runtime",
                "0.1.0",
            ),
        );
    }

    /// Start a project-host server and return its descriptor with the bound
    /// endpoint.
    ///
    /// The client mints project-host capabilities from the descriptor's
    /// unscoped brokered templates and scopes them to the attach session, so
    /// the server's grant set must be the session-scoped mirror of those
    /// templates for exact-match validation to succeed.
    fn start_session_scoped_project_host(
        workspace_path: &Path,
        session_id: SessionId,
    ) -> (ServiceDescriptor, az_project_host::ProjectHostRpcServer) {
        let mut descriptor = project_host_service_descriptor(
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        let session_scoped_grants = CapabilityGrantSet::from_grants(
            descriptor
                .capabilities
                .iter()
                .map(|capability| capability.clone().scoped_to(Some(session_id.0)))
                .collect::<Vec<_>>(),
        );
        let server =
            az_project_host::start_project_host_rpc_server_for_session_with_capability_grants(
                workspace_path,
                descriptor.endpoint.clone(),
                session_id.0.to_string(),
                session_scoped_grants,
            )
            .unwrap();
        descriptor.endpoint = server.endpoint().clone();
        (descriptor, server)
    }

    /// Register the session-owned runtime-host descriptor, attach the
    /// project-owned project-host descriptor through the separate
    /// project-service store, and record a running runtime-host process.
    fn register_launch_test_session_services(
        manager: &SessionManager,
        session_slug: &str,
        runtime_descriptor: &ServiceDescriptor,
        project_host_descriptor: ServiceDescriptor,
        owner_id: &str,
        workspace_path: &Path,
        run_dir: &Path,
    ) {
        manager
            .register_service_descriptor(session_slug, runtime_descriptor)
            .unwrap();
        manager
            .attach_project_service_descriptors(session_slug, &[project_host_descriptor])
            .unwrap();
        let launch_command = SessionServiceLaunchCommand {
            owner_id: owner_id.to_string(),
            owner_root: workspace_path.to_path_buf(),
            build_output_root: run_dir.to_path_buf(),
            service_name: RUNTIME_HOST_SERVICE_NAME.to_string(),
            role: ServiceRole::RuntimeHost,
            endpoint: runtime_descriptor.endpoint.clone(),
            program: run_dir
                .join("runtime-host-test-bin")
                .to_string_lossy()
                .into_owned(),
            cwd: workspace_path.to_path_buf(),
            args: Vec::new(),
        };
        manager
            .record_service_launch_plan(session_slug, &[launch_command])
            .unwrap();
        let runtime_host_process = ServiceProcessKey::new(
            RUNTIME_HOST_SERVICE_NAME,
            SupervisedServiceRole::RuntimeHost,
        );
        manager
            .mark_service_running(
                session_slug,
                &runtime_host_process,
                ProcessIdentity {
                    process_id: 94,
                    process_start_time: 9_001,
                },
            )
            .unwrap();
    }

    struct EditorCatalogTestProjection;

    impl RuntimeProjection for EditorCatalogTestProjection {
        fn launch(
            &mut self,
            _context: &RuntimeProjectionLaunchContext<'_>,
        ) -> Result<RuntimeProjectionUpdate, RuntimeProjectionError> {
            Ok(RuntimeProjectionUpdate::running())
        }
    }

    fn editor_catalog_test_projection() -> Box<dyn RuntimeProjection> {
        Box::new(EditorCatalogTestProjection)
    }

    const EDITOR_TEST_PROJECTION: RuntimeProjectionRegistration =
        RuntimeProjectionRegistration::new(
            "az.editor.test.runtime-projection",
            &[RuntimeRole::EditorWorld],
            &["editor"],
            editor_catalog_test_projection,
        )
        .with_priority(50);

    az_gem_contract::declare_caps!(EditorHostCaps:);

    /// The projection these tests expect a runtime host to serve, contributed
    /// the way a project gem contributes its own.
    struct Projections;

    impl az_gem_contract::Contribution for Projections {
        type Caps = EditorHostCaps;

        fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
            az_gem_contract::ContributionDescriptor {
                gem: az_gem_contract::GemId::new("azoth.editor-tests"),
                contribution: az_gem_contract::ContributionId::new("projections"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, EditorHostCaps>) {
            ctx.registrar::<RuntimeProjectionRegistration>()
                .register(EDITOR_TEST_PROJECTION);
        }
    }

    /// A runtime-host process composes once for its whole life, so the tests
    /// hold one composition the same way a real service does.
    fn composed() -> &'static az_gem_contract::Registries {
        static REGISTRIES: std::sync::OnceLock<&'static az_gem_contract::Registries> =
            std::sync::OnceLock::new();
        REGISTRIES.get_or_init(|| {
            let mut composer =
                az_gem_contract::Composer::new(az_gem_contract::GemTargetRole::RuntimeHost);
            composer
                .add(Projections, az_gem_contract::ProductActivation::default())
                .expect("an empty floor composes");
            composer
                .finalize()
                .expect("the projection composition is valid");
            Box::leak(Box::new(composer)).registries()
        })
    }

    fn snapshot_handle(temp: &tempfile::TempDir) -> SideChannelHandle {
        let project_root = temp.path().join("editor-runtime");
        let workspace_root = project_root.join(".azoth/workspaces/lighting");
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.editor_runtime",
            Uuid::from_bytes([0x65; 16]),
            "lighting",
            RuntimeRole::EditorWorld,
            project_root.to_string_lossy(),
            workspace_root.to_string_lossy(),
        );
        snapshot.authored_revision = 99;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 77;
        snapshot.asset_source_roots.push(RuntimeAssetSourceRoot {
            workspace_root_id: 101,
            workspace_id: 77,
            root_id: 202,
            owner_id: "local.editor_runtime".to_string(),
            source_root: workspace_root.join("assets").to_string_lossy().into_owned(),
            display_name: "Project Assets".to_string(),
            portable_key: "project:local.editor_runtime:assets".to_string(),
            output_prefix: String::new(),
            is_root: true,
        });
        let bytes = encode_runtime_launch_snapshot(&snapshot).unwrap();
        let hash = blake3::hash(&bytes).as_bytes().to_vec();
        let path = temp.path().join("runtime-launch.capnp.packed");
        std::fs::write(&path, bytes).unwrap();
        let byte_length = std::fs::metadata(&path).unwrap().len();
        SideChannelHandle::staging_file(
            path.to_string_lossy(),
            byte_length,
            hash,
            std::env::consts::OS,
        )
        .with_capability(project_host_runtime_control_grant(snapshot.session_id))
    }

    fn viewport_frame() -> RuntimeViewportFrame {
        RuntimeViewportFrame {
            runtime_id: EDITOR_WORLD_RUNTIME_ID.to_string(),
            width: 1280,
            height: 720,
            row_pitch: 5120,
            format: ViewportPixelFormat::Bgra8Unorm,
            color: SideChannelHandle {
                kind: SideChannelKind::GpuSurface,
                capability: None,
                locator: "dxgi://adapter/0/editor-world/3".to_string(),
                byte_length: 3_686_400,
                content_hash: Vec::new(),
                platform: "windows".to_string(),
            },
        }
    }

    fn runtime_status(runtime_id: impl Into<String>, role: RuntimeRole) -> RuntimeStatus {
        RuntimeStatus {
            runtime_id: runtime_id.into(),
            role,
            state: RuntimeState::Running,
            project_id: "local.editor_runtime".to_string(),
            session_slug: "lighting".to_string(),
            authored_revision: 42,
            diagnostic: String::new(),
        }
    }

    fn staging_viewport_frame(temp: &tempfile::TempDir) -> RuntimeViewportFrame {
        let bytes = vec![0, 10, 20, 255, 30, 40, 50, 128];
        let hash = blake3::hash(&bytes).as_bytes().to_vec();
        let path = temp.path().join("viewport-frame.bgra");
        std::fs::write(&path, &bytes).unwrap();
        RuntimeViewportFrame {
            runtime_id: EDITOR_WORLD_RUNTIME_ID.to_string(),
            width: 2,
            height: 1,
            row_pitch: 8,
            format: ViewportPixelFormat::Bgra8Unorm,
            color: SideChannelHandle::staging_file(
                path.to_string_lossy(),
                bytes.len() as u64,
                hash,
                std::env::consts::OS,
            ),
        }
    }

    fn test_workspace_snapshot(
        workspace_id: i64,
        workspace_root: impl Into<String>,
        source_root: impl Into<String>,
    ) -> WorkspaceSnapshot {
        let workspace_root = workspace_root.into();
        let source_root = source_root.into();
        WorkspaceSnapshot {
            workspace_id,
            project_id: "local.editor_runtime".to_string(),
            workspace_root: workspace_root.clone(),
            branch: "az/editor-world".to_string(),
            created_unix_ms: 10,
            updated_unix_ms: 20,
            roots: vec![
                az_proto_asset::WorkspaceRoot {
                    workspace_root_id: workspace_id + 100,
                    workspace_id,
                    root_id: workspace_id + 200,
                    declared_root_id: "project.assets".to_string(),
                    owner_id: "local.editor_runtime".to_string(),
                    source_root,
                    display_name: "Project Assets".to_string(),
                    portable_key: "project:local.editor_runtime:assets".to_string(),
                    mount: "@assets@".to_string(),
                    recursive: true,
                    watch: true,
                    writable: true,
                    output_prefix: String::new(),
                    is_root: true,
                },
                az_proto_asset::WorkspaceRoot {
                    workspace_root_id: workspace_id + 101,
                    workspace_id,
                    root_id: workspace_id + 201,
                    declared_root_id: "gem.azoth.physics.assets".to_string(),
                    owner_id: "azoth.physics".to_string(),
                    source_root: format!("{workspace_root}/gems/physics/assets"),
                    display_name: "Physics Assets".to_string(),
                    portable_key: "gem:azoth.physics:assets".to_string(),
                    mount: "@gems/physics/assets@".to_string(),
                    recursive: true,
                    watch: true,
                    writable: false,
                    output_prefix: "gems/azoth.physics".to_string(),
                    is_root: false,
                },
            ],
        }
    }

    #[test]
    fn runtime_host_client_uses_descriptor_capability_templates() {
        let session = Uuid::from_bytes([0x34; 16]);
        let rpc = Rc::new(runtime_host_rpc(session));
        let client = RuntimeHostClient::new(RuntimeHostRpc::client_from_rc(&rpc))
            .with_session_scope(session)
            .with_capability_templates(vec![
                Capability::new(
                    ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                    ServiceRole::Editor,
                )
                .with_session(session)
                .with_audience(RUNTIME_HOST_AUDIENCE)
                .with_permissions([RUNTIME_READ_PERMISSION])
                .with_token_hash([0x34]),
                Capability::new(
                    ServiceId::new(PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
                    ServiceRole::ProjectHost,
                )
                .with_session(session)
                .with_audience(RUNTIME_HOST_AUDIENCE)
                .with_permissions([RUNTIME_CONTROL_PERMISSION])
                .with_token_hash([0x35]),
            ]);

        let capability = client.runtime_read_capability().unwrap();
        assert_eq!(capability.token_hash, vec![0x34]);
        let project_host_capability = client.project_host_runtime_control_capability().unwrap();
        assert_eq!(project_host_capability.token_hash, vec![0x35]);
        assert!(client.runtime_control_capability().is_err());
    }

    #[test]
    fn runtime_host_client_rejects_empty_descriptor_capabilities() {
        let session = Uuid::from_bytes([0x34; 16]);
        let rpc = Rc::new(runtime_host_rpc(session));
        let client = RuntimeHostClient::new(RuntimeHostRpc::client_from_rc(&rpc))
            .with_capability_templates(Vec::new())
            .with_session_scope(session);

        assert!(client.runtime_read_capability().is_err());
    }

    #[test]
    fn runtime_host_client_requires_bound_viewport_frame_capabilities() {
        let expected = Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_READ_PERMISSION])
        .with_token_hash([0xab, 0xcd]);
        let frame = viewport_frame();

        let missing =
            validate_runtime_viewport_frame_side_channel_capability(&frame, &expected).unwrap_err();
        assert!(matches!(
            missing,
            EditorError::SideChannelCapability(
                az_proto_core::SideChannelCapabilityError::Missing { .. }
            )
        ));

        let mut mismatched = frame.clone();
        mismatched.color.capability = Some(expected.clone().with_token_hash([0x99]));
        let mismatch =
            validate_runtime_viewport_frame_side_channel_capability(&mismatched, &expected)
                .unwrap_err();
        assert!(matches!(
            mismatch,
            EditorError::SideChannelCapability(
                az_proto_core::SideChannelCapabilityError::Mismatch { .. }
            )
        ));

        let mut matched = frame;
        matched.color.capability = Some(expected.clone());
        validate_runtime_viewport_frame_side_channel_capability(&matched, &expected).unwrap();
    }

    #[test]
    fn runtime_status_result_must_echo_requested_runtime() {
        let status = runtime_status("other-runtime", RuntimeRole::EditorWorld);

        let error =
            ensure_runtime_status_matches_request(&status, EDITOR_WORLD_RUNTIME_ID, None, "status")
                .unwrap_err();

        assert_runtime_host_authority_mismatch(error, "status", "expected `editor-world`");
    }

    #[test]
    fn runtime_launch_result_must_echo_requested_role() {
        let status = runtime_status(EDITOR_WORLD_RUNTIME_ID, RuntimeRole::PlayPreview);

        let error = ensure_runtime_status_matches_request(
            &status,
            EDITOR_WORLD_RUNTIME_ID,
            Some(RuntimeRole::EditorWorld),
            "launch",
        )
        .unwrap_err();

        assert_runtime_host_authority_mismatch(error, "launch", "expected EditorWorld");
    }

    #[test]
    fn runtime_viewport_frame_must_echo_requested_runtime() {
        let mut frame = viewport_frame();
        frame.runtime_id = "other-runtime".to_string();

        let error = ensure_runtime_viewport_frame_matches_request(
            &frame,
            EDITOR_WORLD_RUNTIME_ID,
            "viewportFrame",
        )
        .unwrap_err();

        assert_runtime_host_authority_mismatch(error, "viewportFrame", "expected `editor-world`");
    }

    #[test]
    fn runtime_viewport_frame_must_have_valid_layout() {
        let mut frame = viewport_frame();
        frame.row_pitch = 1;

        let error = ensure_runtime_viewport_frame_matches_request(
            &frame,
            EDITOR_WORLD_RUNTIME_ID,
            "viewportFrame",
        )
        .unwrap_err();

        assert_runtime_host_authority_mismatch(error, "viewportFrame", "smaller than minimum");
    }

    #[test]
    fn runtime_viewport_frame_must_use_gpu_surface_side_channel() {
        let temp = tempfile::tempdir().unwrap();
        let frame = staging_viewport_frame(&temp);

        let error = ensure_runtime_viewport_frame_matches_request(
            &frame,
            EDITOR_WORLD_RUNTIME_ID,
            "viewportFrame",
        )
        .unwrap_err();

        assert_runtime_host_authority_mismatch(error, "viewportFrame", "got stagingFile");
    }

    #[test]
    fn viewport_render_status_rejects_cpu_readable_side_channels() {
        let temp = tempfile::tempdir().unwrap();
        let frame = staging_viewport_frame(&temp);

        let render_status = editor_viewport_render_status_from_frame(&frame, 4);

        assert_eq!(render_status.state, EditorViewportRenderStateData::Failed);
        assert_eq!(render_status.generation, Some(4));
        assert_eq!(render_status.width, Some(frame.width));
        assert_eq!(render_status.height, Some(frame.height));
        assert_eq!(render_status.format.as_deref(), Some("bgra8Unorm"));
        assert!(render_status.backend.is_none());
        assert_eq!(
            render_status.diagnostic.as_deref(),
            Some("runtime viewport requires gpuSurface side-channel, got stagingFile")
        );
    }

    #[test]
    fn viewport_render_status_keeps_gpu_surface_handles_distinct() {
        let frame = viewport_frame();

        let render_status = editor_viewport_render_status_from_frame(&frame, 3);

        assert_eq!(
            render_status.state,
            EditorViewportRenderStateData::GpuSurfaceHandle
        );
        assert_eq!(render_status.generation, Some(3));
        assert_eq!(render_status.width, Some(frame.width));
        assert_eq!(render_status.height, Some(frame.height));
        assert_eq!(render_status.format.as_deref(), Some("bgra8Unorm"));
        assert_eq!(render_status.backend.as_deref(), Some("gpuSurface"));
        assert!(render_status.diagnostic.is_none());
    }

    #[test]
    fn runtime_projection_catalog_rejects_duplicate_projection_names() {
        let mut catalog = valid_runtime_projection_catalog();
        catalog.projections.push(catalog.projections[0].clone());

        let error = ensure_runtime_projection_catalog_matches_request(&catalog).unwrap_err();

        assert_runtime_host_authority_mismatch(error, "projectionCatalog", "duplicate projection");
    }

    #[test]
    fn runtime_projection_catalog_rejects_invalid_launch_profiles() {
        let mut catalog = valid_runtime_projection_catalog();
        catalog.projections[0].launch_profiles[0].clear();

        let error = ensure_runtime_projection_catalog_matches_request(&catalog).unwrap_err();

        assert_runtime_host_authority_mismatch(
            error,
            "projectionCatalog",
            "launch profile cannot be empty",
        );
    }

    #[test]
    fn editor_launches_queries_and_stops_runtime_host_over_rpc() {
        let temp = tempfile::tempdir().unwrap();
        let session = Uuid::from_bytes([0x65; 16]);
        let rpc = Rc::new(runtime_host_rpc(session));
        let client = runtime_host_client_from_rpc(&rpc, session);

        let launched = futures::executor::block_on(client.launch(
            "editor-world",
            RuntimeRole::EditorWorld,
            snapshot_handle(&temp),
        ))
        .unwrap();
        assert_eq!(launched.state, RuntimeState::Running);
        assert_eq!(launched.authored_revision, 99);

        let status = futures::executor::block_on(client.status("editor-world")).unwrap();
        assert_eq!(status, Some(launched));

        let mut expected_frame = rpc.host().publish_viewport_frame(viewport_frame()).unwrap();
        expected_frame.color.capability = Some(client.runtime_read_capability().unwrap());
        let viewport_frame =
            futures::executor::block_on(client.viewport_frame("editor-world")).unwrap();
        assert_eq!(viewport_frame, Some(expected_frame));

        let stopped = futures::executor::block_on(client.stop("editor-world", true)).unwrap();
        assert_eq!(stopped.state, RuntimeState::Stopped);
        assert_eq!(
            runtime_host_service_id(),
            ServiceId::new(RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME)
        );
    }

    #[test]
    fn editor_reads_runtime_projection_catalog_through_rpc() {
        let session = Uuid::from_bytes([0x65; 16]);
        let rpc = Rc::new(runtime_host_rpc(session));
        let client = runtime_host_client_from_rpc(&rpc, session);

        let catalog = futures::executor::block_on(client.projection_catalog()).unwrap();

        let projection = catalog
            .projections
            .iter()
            .find(|projection| projection.name == "az.editor.test.runtime-projection")
            .expect("editor test runtime projection should be reported through runtime-host RPC");
        assert_eq!(projection.priority, 50);
        assert_eq!(projection.roles, vec![RuntimeRole::EditorWorld]);
        assert_eq!(projection.launch_profiles, vec!["editor"]);
    }

    fn assert_runtime_host_authority_mismatch(
        error: EditorError,
        expected_operation: &'static str,
        expected_reason: &str,
    ) {
        assert!(matches!(
            error,
            EditorError::ServiceAuthorityMismatch {
                service: "runtime-host",
                operation,
                reason,
            } if operation == expected_operation && reason.contains(expected_reason)
        ));
    }

    fn valid_runtime_projection_catalog() -> RuntimeProjectionCatalogResult {
        RuntimeProjectionCatalogResult {
            projections: vec![RuntimeProjectionDescriptor {
                name: "az.test.editor-world".to_string(),
                priority: 25,
                roles: vec![RuntimeRole::EditorWorld, RuntimeRole::PlayPreview],
                launch_profiles: vec!["editor".to_string(), "play".to_string()],
            }],
        }
    }

    #[test]
    fn maps_runtime_projection_catalog_to_viewport_ui() {
        let catalog = valid_runtime_projection_catalog();

        let ui_catalog = runtime_projection_catalog_to_ui(catalog);

        assert_eq!(ui_catalog.projections.len(), 1);
        assert_eq!(ui_catalog.projections[0].name, "az.test.editor-world");
        assert_eq!(ui_catalog.projections[0].priority, 25);
        assert_eq!(
            ui_catalog.projections[0].roles,
            vec!["editor-world", "play-preview"]
        );
        assert_eq!(
            ui_catalog.projections[0].launch_profiles,
            vec!["editor", "play"]
        );
    }

    #[test]
    fn maps_runtime_status_to_viewport_ui() {
        let status = RuntimeStatus {
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            state: RuntimeState::Failed,
            project_id: "az.test".to_string(),
            session_slug: "lighting".to_string(),
            authored_revision: 42,
            diagnostic: "launch failed\n\n viewport failed ".to_string(),
        };

        let ui_status = runtime_status_to_ui("editor-world", Some(status));

        assert_eq!(ui_status.runtime_id, "editor-world");
        assert_eq!(ui_status.state, EditorRuntimeStateData::Failed);
        assert_eq!(ui_status.role.as_deref(), Some("editor-world"));
        assert_eq!(ui_status.project_id.as_deref(), Some("az.test"));
        assert_eq!(ui_status.session_slug.as_deref(), Some("lighting"));
        assert_eq!(ui_status.authored_revision, Some(42));
        assert_eq!(
            ui_status.diagnostics,
            vec!["launch failed", "viewport failed"]
        );

        let missing = runtime_status_to_ui("editor-world", None);
        assert_eq!(missing.runtime_id, "editor-world");
        assert_eq!(missing.state, EditorRuntimeStateData::Unregistered);
    }

    #[test]
    fn editor_runtime_controller_reuses_running_runtime_host_before_start_request() {
        let temp = tempfile::tempdir().unwrap();
        let project_manifest =
            az_project::ProjectManifest::new("local.editor_runtime", "Editor Runtime", "0.1.0");
        write_test_project_manifest(temp.path(), &project_manifest);
        let session_id = SessionId::new();
        let manager = SessionManager::with_data_home(
            temp.path(),
            AzothDataHome::new(temp.path().join("azoth-test-home")),
        )
        .unwrap();
        let fixture = runtime_host_session_fixture(
            &temp,
            &manager,
            session_id,
            "lighting",
            &project_manifest.project.id,
        );

        let mut descriptor = runtime_host_service_descriptor(
            session_id.0,
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        let runtime_server = start_runtime_host_rpc_server_for_session_with_capability_grants(
            descriptor.endpoint.clone(),
            session_id.0.to_string(),
            fixture.run_dir.join("side-channels").join("runtime-host"),
            CapabilityGrantSet::from_grants(descriptor.capabilities.clone()),
            composed(),
        )
        .unwrap();
        descriptor.endpoint = runtime_server.endpoint().clone();
        write_running_runtime_host_manifest(&fixture, &descriptor, runtime_server.endpoint());

        let supervisor_rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
        let supervisor = crate::SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(
            &supervisor_rpc,
        ))
        .with_session_scope(session_id.0);
        let project_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::new(),
            temp.path().join("project-host"),
            session_id.0,
        ));
        let controller = session_scoped_runtime_controller(
            &fixture,
            project_host_client_from_rpc_for_session(&project_rpc, session_id.0),
            None,
            Some(supervisor),
        );

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let catalog = local.block_on(&runtime, async {
            let runtime_client = controller.runtime_host_client_for_launch().await.unwrap();
            runtime_client.projection_catalog().await.unwrap()
        });

        assert!(
            catalog
                .projections
                .iter()
                .any(|projection| projection.name == "az.editor.test.runtime-projection")
        );
        runtime_server.stop().expect("stop runtime-host server");
    }

    #[test]
    fn editor_runtime_controller_queries_current_runtime_host_from_session_supervisor() {
        let temp = tempfile::tempdir().unwrap();
        let project_manifest =
            az_project::ProjectManifest::new("local.editor_runtime", "Editor Runtime", "0.1.0");
        write_test_project_manifest(temp.path(), &project_manifest);
        let session_id = SessionId(Uuid::from_bytes([0x65; 16]));
        let manager = SessionManager::with_data_home(
            temp.path(),
            AzothDataHome::new(temp.path().join("azoth-test-home")),
        )
        .unwrap();
        let fixture = runtime_host_session_fixture(
            &temp,
            &manager,
            session_id,
            "runtime-reconnect",
            &project_manifest.project.id,
        );
        let run_dir = fixture.run_dir.clone();

        let (fresh_descriptor, fresh_server) = start_descriptor_runtime_host(session_id);
        write_running_runtime_host_manifest(&fixture, &fresh_descriptor, fresh_server.endpoint());

        let supervisor_rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
        let supervisor = crate::SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(
            &supervisor_rpc,
        ))
        .with_session_scope(session_id.0);
        let stale_descriptor = runtime_host_service_descriptor(
            session_id.0,
            Uuid::now_v7(),
            Endpoint::in_process("runtime-host:stale"),
        );
        let stale_runtime_rpc = Rc::new(RuntimeHostRpc::test_new(RuntimeHost::new(
            session_id.0.to_string(),
            CapabilityGrantSet::from_grants(stale_descriptor.capabilities.clone()),
            run_dir.join("side-channels").join("stale-runtime-host"),
            composed(),
        )));
        let stale_runtime_client =
            RuntimeHostClient::new(RuntimeHostRpc::client_from_rc(&stale_runtime_rpc))
                .with_capability_templates(stale_descriptor.capabilities)
                .with_session_scope(session_id.0);
        let stale_status_client = stale_runtime_client.clone();
        let mut stale_snapshot = snapshot_handle(&temp);
        stale_snapshot.capability = Some(
            stale_runtime_client
                .project_host_runtime_control_capability()
                .unwrap(),
        );
        let project_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::new(),
            temp.path().join("project-host"),
            session_id.0,
        ));

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&runtime, async {
            stale_runtime_client
                .launch(
                    EDITOR_WORLD_RUNTIME_ID,
                    RuntimeRole::EditorWorld,
                    stale_snapshot,
                )
                .await
                .unwrap();
        });

        let controller = session_scoped_runtime_controller(
            &fixture,
            project_host_client_from_rpc_for_session(&project_rpc, session_id.0),
            Some(stale_runtime_client),
            Some(supervisor),
        );

        let status = local
            .block_on(&runtime, controller.editor_world_status())
            .unwrap();
        let stale_status = local
            .block_on(
                &runtime,
                stale_status_client.status(EDITOR_WORLD_RUNTIME_ID),
            )
            .unwrap();

        assert_eq!(
            stale_status.as_ref().map(|status| status.state),
            Some(RuntimeState::Running)
        );
        assert_eq!(
            status, None,
            "runtime queries must follow the session supervisor's current descriptor instead of a stale harness client"
        );
        fresh_server.stop().expect("stop fresh runtime-host server");
    }

    #[test]
    fn editor_runtime_status_fails_for_registered_unreachable_runtime_host() {
        let temp = tempfile::tempdir().unwrap();
        let project_manifest =
            az_project::ProjectManifest::new("local.editor_runtime", "Editor Runtime", "0.1.0");
        write_test_project_manifest(temp.path(), &project_manifest);
        let session_id = SessionId(Uuid::from_bytes([0x6a; 16]));
        let session_slug = "runtime-stale-descriptor";
        let manager = SessionManager::with_data_home(
            temp.path(),
            AzothDataHome::new(temp.path().join("azoth-test-home")),
        )
        .unwrap();
        let run_dir = manager.sessions_dir().join(session_id.to_string());
        let workspace_path = temp.path().to_path_buf();
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::create_dir_all(&workspace_path).unwrap();

        let stale_descriptor = runtime_host_service_descriptor(
            session_id.0,
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:1"),
        );
        let mut manifest = SessionManifest::new(
            session_id,
            project_manifest.project.id.clone(),
            session_slug.to_string(),
            temp.path().to_path_buf(),
            workspace_path.clone(),
            run_dir.clone(),
            0,
        );
        manifest.state = SessionState::Active;
        manifest
            .upsert_service_descriptor(&stale_descriptor, 1)
            .unwrap();
        add_running_service_process(
            &mut manifest,
            &stale_descriptor,
            RUNTIME_HOST_SERVICE_NAME,
            SupervisedServiceRole::RuntimeHost,
            &project_manifest.project.id,
            temp.path(),
            &workspace_path,
            std::process::id(),
        );
        std::fs::write(
            run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let supervisor_rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
        let supervisor = crate::SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(
            &supervisor_rpc,
        ))
        .with_session_scope(session_id.0);
        let project_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::new(),
            temp.path().join("project-host"),
            session_id.0,
        ));
        let stale_runtime_rpc = Rc::new(runtime_host_rpc(session_id.0));
        let controller = EditorRuntimeController {
            session_id: Some(session_id.0),
            project_id: project_manifest.project.id.clone(),
            session_slug: session_slug.to_string(),
            project_root: temp.path().to_string_lossy().into_owned(),
            workspace: AttachedWorkspace {
                project_id: project_manifest.project.id.clone(),
                workspace_root: workspace_path.to_string_lossy().into_owned(),
                branch: "main".to_string(),
            },
            run_dir: Some(run_dir.to_string_lossy().into_owned()),
            project_host: project_host_client_from_rpc_for_session(&project_rpc, session_id.0),
            runtime_host: Some(runtime_host_client_from_rpc(
                &stale_runtime_rpc,
                session_id.0,
            )),
            supervisor: Some(supervisor),
            supervisor_descriptor: None,
            asset_processor: None,
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let error = local
            .block_on(&runtime, controller.editor_world_status())
            .unwrap_err();

        match error {
            EditorError::RpcTransport(_) => {}
            other => panic!("expected stale runtime descriptor transport error, got {other:?}"),
        }
    }

    #[test]
    fn editor_runtime_controller_reads_workspace_snapshot_from_attached_rpc() {
        let temp = tempfile::tempdir().unwrap();
        let session_id = Uuid::from_bytes([0x55; 16]);
        let workspace_path = temp.path().join("workspace").to_string_lossy().into_owned();
        let project_assets_root = PathBuf::from(&workspace_path).join("assets");
        let workspace_snapshot = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            project_assets_root.to_string_lossy(),
        );
        let project_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::new(),
            temp.path().join("project-host"),
            session_id,
        ));
        let runtime_rpc = Rc::new(runtime_host_rpc(session_id));
        let controller = EditorRuntimeController::new(
            "local.editor_runtime",
            "lighting",
            temp.path().to_string_lossy(),
            workspace_path,
            "az/editor-world",
            project_host_client_from_rpc_for_session(&project_rpc, session_id),
            runtime_host_client_from_rpc(&runtime_rpc, session_id),
        )
        .with_session_id(session_id)
        .with_asset_processor(Some(asset_client_with_workspace_snapshot(
            workspace_snapshot.clone(),
            session_id,
        )));

        let returned =
            futures::executor::block_on(controller.session_workspace_snapshot()).unwrap();

        assert_eq!(returned, workspace_snapshot);
    }

    #[test]
    #[ignore = "awaiting az_asset_processor::test_support::SyntheticCorpus"]
    fn editor_runtime_controller_reads_workspace_snapshot_through_real_asset_processor_boundary() {
        panic!("SyntheticCorpus is required to exercise the shipping asset-processor writer");
    }

    #[test]
    fn editor_runtime_controller_launch_uses_current_project_host_from_session_supervisor() {
        let temp = tempfile::tempdir().unwrap();
        // `TEMP` can be an 8.3 short path. The session manager records a
        // canonical workspace root, and `path_is_within_or_equal` compares a
        // planned program (not yet on disk, so only lexically normalized)
        // against its build output root (on disk, so canonicalized); both
        // have to descend from the same spelling for the launch plan to
        // validate.
        let root = az_filesystem::normalize(temp.path());
        let project_manifest =
            az_project::ProjectManifest::new("local.editor_runtime", "Editor Runtime", "0.1.0");
        write_test_project_manifest(&root, &project_manifest);
        let manager =
            SessionManager::with_data_home(&root, AzothDataHome::new(root.join("azoth-test-home")))
                .unwrap();
        let manifest = manager
            .create_session(CreateSessionRequest::new("Runtime Project Host Reconnect"))
            .unwrap();
        let session_id = manifest.id;
        let session_slug = manifest.slug.clone();
        let branch = "main".to_string();
        let run_dir = manager.sessions_dir().join(session_id.to_string());
        let workspace_path = PathBuf::from(&manifest.workspace_root);
        let stale_project_root = root.join("stale-project-host");
        let assets_root = workspace_path.join("assets");
        prepare_launch_test_project_tree(
            &[&run_dir, &workspace_path, &stale_project_root, &assets_root],
            &workspace_path,
            &stale_project_root,
            &project_manifest,
        );

        let (runtime_descriptor, runtime_server) = start_descriptor_runtime_host(session_id);
        let (project_host_descriptor, project_host_server) =
            start_session_scoped_project_host(&workspace_path, session_id);

        register_launch_test_session_services(
            &manager,
            &session_slug,
            &runtime_descriptor,
            project_host_descriptor,
            &project_manifest.project.id,
            &workspace_path,
            &run_dir,
        );

        let supervisor_rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
        let supervisor = crate::SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(
            &supervisor_rpc,
        ))
        .with_session_scope(session_id.0);
        let stale_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::with_source_root(&stale_project_root),
            root.join("stale-project-host-side-channels"),
            session_id.0,
        ));
        let mut workspace_snapshot = test_workspace_snapshot(
            99,
            workspace_path.to_string_lossy(),
            assets_root.to_string_lossy(),
        );
        workspace_snapshot.branch = branch.clone();
        let controller = EditorRuntimeController {
            session_id: Some(session_id.0),
            project_id: project_manifest.project.id.clone(),
            session_slug: session_slug.clone(),
            project_root: root.to_string_lossy().into_owned(),
            workspace: AttachedWorkspace {
                project_id: project_manifest.project.id,
                workspace_root: workspace_path.to_string_lossy().into_owned(),
                branch,
            },
            run_dir: Some(run_dir.to_string_lossy().into_owned()),
            project_host: project_host_client_from_rpc_for_session(&stale_rpc, session_id.0),
            runtime_host: None,
            supervisor: Some(supervisor),
            supervisor_descriptor: None,
            asset_processor: None,
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let launched = local
            .block_on(
                &runtime,
                controller.launch_editor_world(&workspace_snapshot, false),
            )
            .unwrap();

        assert_eq!(launched.runtime_id, EDITOR_WORLD_RUNTIME_ID);
        assert_eq!(launched.state, RuntimeState::Running);
        assert_eq!(launched.project_id, "local.editor_runtime");
        assert_eq!(launched.session_slug, session_slug);
        assert_eq!(launched.authored_revision, 0);

        project_host_server
            .stop()
            .expect("stop project-host server");
        runtime_server.stop().expect("stop runtime-host server");
    }

    #[test]
    fn editor_runtime_controller_launches_editor_world_from_project_host_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let session_id = Uuid::from_bytes([0x54; 16]);
        let project_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::new(),
            temp.path().join("project-host"),
            session_id,
        ));
        let runtime_rpc = Rc::new(runtime_host_rpc(session_id));
        let workspace_root = temp.path().join("workspace");
        let workspace_path = workspace_root.to_string_lossy().to_string();
        let project_assets_root = workspace_root.join("assets");
        let workspace_snapshot = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            project_assets_root.to_string_lossy(),
        );
        let controller = EditorRuntimeController::new(
            "local.editor_runtime",
            "lighting",
            temp.path().to_string_lossy(),
            workspace_path,
            "az/editor-world",
            project_host_client_from_rpc_for_session(&project_rpc, session_id),
            runtime_host_client_from_rpc(&runtime_rpc, session_id),
        )
        .with_session_id(session_id);

        let launched =
            futures::executor::block_on(controller.launch_editor_world(&workspace_snapshot, true))
                .unwrap();

        assert_eq!(launched.runtime_id, EDITOR_WORLD_RUNTIME_ID);
        assert_eq!(launched.state, RuntimeState::Running);
        assert_eq!(launched.project_id, "local.editor_runtime");
        assert_eq!(launched.session_slug, "lighting");

        let snapshot = runtime_rpc
            .host()
            .launched_snapshot(EDITOR_WORLD_RUNTIME_ID)
            .unwrap();
        assert_eq!(snapshot.workspace_id, 77);
        assert_eq!(snapshot.asset_source_roots.len(), 2);
        assert!(
            snapshot
                .asset_source_roots
                .iter()
                .any(|root| root.portable_key == "project:local.editor_runtime:assets")
        );
        assert!(snapshot.include_unsaved_journal);
        assert_eq!(snapshot.schema_catalog_hash.len(), 32);

        let mut expected_frame = runtime_rpc
            .host()
            .publish_viewport_frame(viewport_frame())
            .unwrap();
        expected_frame.color.capability = Some(
            controller
                .runtime_host
                .as_ref()
                .unwrap()
                .runtime_read_capability()
                .unwrap(),
        );
        let actual_frame =
            futures::executor::block_on(controller.editor_world_viewport_frame()).unwrap();
        assert_eq!(actual_frame, Some(expected_frame));
    }

    #[test]
    fn editor_runtime_controller_rejects_workspace_snapshot_without_roots() {
        let temp = tempfile::tempdir().unwrap();
        let session_id = Uuid::from_bytes([0x57; 16]);
        let project_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::new(),
            temp.path().join("project-host"),
            session_id,
        ));
        let runtime_rpc = Rc::new(runtime_host_rpc(session_id));
        let workspace_path = temp.path().join("workspace").to_string_lossy().to_string();
        let mut workspace_snapshot = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            temp.path().join("assets").to_string_lossy(),
        );
        workspace_snapshot.roots.clear();
        let controller = EditorRuntimeController::new(
            "local.editor_runtime",
            "lighting",
            temp.path().to_string_lossy(),
            workspace_path,
            "az/editor-world",
            project_host_client_from_rpc_for_session(&project_rpc, session_id),
            runtime_host_client_from_rpc(&runtime_rpc, session_id),
        )
        .with_session_id(session_id);

        let error =
            futures::executor::block_on(controller.launch_editor_world(&workspace_snapshot, true))
                .unwrap_err();

        match error {
            EditorError::MissingRuntimeAssetSourceRoots {
                session,
                workspace_id,
            } => {
                assert_eq!(session, "lighting");
                assert_eq!(workspace_id, 77);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn editor_runtime_controller_rejects_workspace_snapshot_without_root_identity() {
        let temp = tempfile::tempdir().unwrap();
        let session_id = Uuid::from_bytes([0x59; 16]);
        let project_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::new(),
            temp.path().join("project-host"),
            session_id,
        ));
        let runtime_rpc = Rc::new(runtime_host_rpc(session_id));
        let workspace_path = temp.path().join("workspace").to_string_lossy().to_string();
        let mut workspace_snapshot = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            temp.path().join("assets").to_string_lossy(),
        );
        workspace_snapshot.roots[0].workspace_root_id = 0;
        let controller = EditorRuntimeController::new(
            "local.editor_runtime",
            "lighting",
            temp.path().to_string_lossy(),
            workspace_path,
            "az/editor-world",
            project_host_client_from_rpc_for_session(&project_rpc, session_id),
            runtime_host_client_from_rpc(&runtime_rpc, session_id),
        )
        .with_session_id(session_id);

        assert!(matches!(
            controller.validate_runtime_launch_workspace_snapshot(&workspace_snapshot),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("positive DB workspace source root id")
        ));

        workspace_snapshot.roots[0].workspace_root_id = 177;
        workspace_snapshot.roots[0].root_id = 0;
        assert!(matches!(
            controller.validate_runtime_launch_workspace_snapshot(&workspace_snapshot),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("positive DB scan folder id")
        ));
    }

    #[test]
    fn editor_runtime_controller_rejects_workspace_snapshot_without_identity() {
        let temp = tempfile::tempdir().unwrap();
        let workspace_path = temp.path().join("workspace").to_string_lossy().to_string();
        let mut workspace_snapshot = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            temp.path().join("assets").to_string_lossy(),
        );

        workspace_snapshot.workspace_id = 0;
        assert!(matches!(
            ensure_runtime_launch_workspace_snapshot(
                "lighting",
                "local.editor_runtime",
                &workspace_path,
                "az/editor-world",
                &workspace_snapshot,
            ),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("positive DB id")
        ));

        let mut workspace_snapshot = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            temp.path().join("assets").to_string_lossy(),
        );
        workspace_snapshot.updated_unix_ms = workspace_snapshot.created_unix_ms - 1;
        assert!(matches!(
            ensure_runtime_launch_workspace_snapshot(
                "lighting",
                "local.editor_runtime",
                &workspace_path,
                "az/editor-world",
                &workspace_snapshot,
            ),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("updated before it was created")
        ));
    }

    #[test]
    fn editor_runtime_controller_rejects_duplicate_runtime_workspace_roots() {
        let temp = tempfile::tempdir().unwrap();
        let workspace_path = temp.path().join("workspace").to_string_lossy().to_string();
        let mut duplicate_id = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            temp.path().join("assets").to_string_lossy(),
        );
        duplicate_id.roots[1].workspace_root_id = duplicate_id.roots[0].workspace_root_id;

        assert!(matches!(
            ensure_runtime_launch_workspace_snapshot(
                "lighting",
                "local.editor_runtime",
                &workspace_path,
                "az/editor-world",
                &duplicate_id,
            ),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("duplicate source root id")
        ));

        let mut duplicate_key = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            temp.path().join("assets").to_string_lossy(),
        );
        duplicate_key.roots[1].portable_key = duplicate_key.roots[0].portable_key.clone();

        assert!(matches!(
            ensure_runtime_launch_workspace_snapshot(
                "lighting",
                "local.editor_runtime",
                &workspace_path,
                "az/editor-world",
                &duplicate_key,
            ),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("duplicate source root key")
        ));
    }

    #[test]
    fn editor_runtime_controller_rejects_workspace_snapshot_without_project_root() {
        let temp = tempfile::tempdir().unwrap();
        let session_id = Uuid::from_bytes([0x5a; 16]);
        let project_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::new(),
            temp.path().join("project-host"),
            session_id,
        ));
        let runtime_rpc = Rc::new(runtime_host_rpc(session_id));
        let workspace_path = temp.path().join("workspace").to_string_lossy().to_string();
        let mut workspace_snapshot = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            temp.path().join("assets").to_string_lossy(),
        );
        workspace_snapshot
            .roots
            .retain(|root| root.portable_key != "project:local.editor_runtime:assets");
        let controller = EditorRuntimeController::new(
            "local.editor_runtime",
            "lighting",
            temp.path().to_string_lossy(),
            workspace_path,
            "az/editor-world",
            project_host_client_from_rpc_for_session(&project_rpc, session_id),
            runtime_host_client_from_rpc(&runtime_rpc, session_id),
        )
        .with_session_id(session_id);

        assert!(matches!(
            controller.validate_runtime_launch_workspace_snapshot(&workspace_snapshot),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("missing DB-owned project assets root")
        ));
    }

    #[test]
    fn editor_runtime_controller_rejects_project_assets_owner_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let workspace_path = temp.path().join("workspace").to_string_lossy().to_string();
        let mut workspace_snapshot = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            temp.path().join("assets").to_string_lossy(),
        );
        workspace_snapshot.roots[0].owner_id = "azoth.physics".to_string();

        assert!(matches!(
            ensure_runtime_launch_workspace_snapshot(
                "lighting",
                "local.editor_runtime",
                &workspace_path,
                "az/editor-world",
                &workspace_snapshot,
            ),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("owner")
                    && message.contains("azoth.physics")
                    && message.contains("local.editor_runtime")
        ));
    }

    #[test]
    fn editor_runtime_controller_rejects_project_assets_root_shape_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let workspace_path = temp.path().join("workspace").to_string_lossy().to_string();
        let project_assets_root = PathBuf::from(&workspace_path).join("assets");
        let mut not_root = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            project_assets_root.to_string_lossy(),
        );
        not_root.roots[0].is_root = false;

        assert!(matches!(
            ensure_runtime_launch_workspace_snapshot(
                "lighting",
                "local.editor_runtime",
                &workspace_path,
                "az/editor-world",
                &not_root,
            ),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("marked as a root source")
        ));

        let mut prefixed = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            project_assets_root.to_string_lossy(),
        );
        prefixed.roots[0].output_prefix = "prefabs".to_string();

        assert!(matches!(
            ensure_runtime_launch_workspace_snapshot(
                "lighting",
                "local.editor_runtime",
                &workspace_path,
                "az/editor-world",
                &prefixed,
            ),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("empty output prefix")
        ));
    }

    #[test]
    fn editor_runtime_controller_rejects_mismatched_runtime_workspace_snapshot_identity() {
        let temp = tempfile::tempdir().unwrap();
        let session_id = Uuid::from_bytes([0x58; 16]);
        let project_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::new(),
            temp.path().join("project-host"),
            session_id,
        ));
        let runtime_rpc = Rc::new(runtime_host_rpc(session_id));
        let workspace_path = temp.path().join("workspace").to_string_lossy().to_string();
        let project_assets_root = PathBuf::from(&workspace_path).join("assets");
        let workspace_snapshot = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            project_assets_root.to_string_lossy(),
        );
        let controller = EditorRuntimeController::new(
            "local.editor_runtime",
            "lighting",
            temp.path().to_string_lossy(),
            workspace_path,
            "az/editor-world",
            project_host_client_from_rpc_for_session(&project_rpc, session_id),
            runtime_host_client_from_rpc(&runtime_rpc, session_id),
        )
        .with_session_id(session_id);
        controller
            .validate_runtime_launch_workspace_snapshot(&workspace_snapshot)
            .unwrap();

        let mut wrong_project = workspace_snapshot.clone();
        wrong_project.project_id = "local.other".to_string();
        assert!(matches!(
            controller.validate_runtime_launch_workspace_snapshot(&wrong_project),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("project") && message.contains("local.other")
        ));

        let mut wrong_root = workspace_snapshot.clone();
        wrong_root.workspace_root = temp.path().join("stale").to_string_lossy().into_owned();
        assert!(matches!(
            controller.validate_runtime_launch_workspace_snapshot(&wrong_root),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("root") && message.contains("stale")
        ));

        let mut wrong_branch = workspace_snapshot;
        wrong_branch.branch = "az/session/stale".to_string();
        assert!(matches!(
            controller.validate_runtime_launch_workspace_snapshot(&wrong_branch),
            Err(EditorError::ServiceDiscovery(message))
                if message.contains("branch") && message.contains("stale")
        ));
    }

    #[test]
    fn editor_runtime_controller_rejects_snapshot_without_roots_from_attached_rpc() {
        let temp = tempfile::tempdir().unwrap();
        let session_id = Uuid::from_bytes([0x56; 16]);
        let workspace_path = temp.path().join("workspace").to_string_lossy().into_owned();
        let project_assets_root = PathBuf::from(&workspace_path).join("assets");
        let mut workspace_snapshot = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            project_assets_root.to_string_lossy(),
        );
        workspace_snapshot.roots.clear();
        let project_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::new(),
            temp.path().join("project-host"),
            session_id,
        ));
        let runtime_rpc = Rc::new(runtime_host_rpc(session_id));
        let controller = EditorRuntimeController::new(
            "local.editor_runtime",
            "lighting",
            temp.path().to_string_lossy(),
            workspace_path,
            "az/editor-world",
            project_host_client_from_rpc_for_session(&project_rpc, session_id),
            runtime_host_client_from_rpc(&runtime_rpc, session_id),
        )
        .with_session_id(session_id)
        .with_asset_processor(Some(asset_client_with_workspace_snapshot(
            workspace_snapshot,
            session_id,
        )));

        assert!(matches!(
            futures::executor::block_on(controller.session_workspace_snapshot_id()),
            Err(EditorError::ServiceProtocol(error))
                if error.to_string().contains("workspace 77 must include roots")
        ));
    }

    #[test]
    fn editor_runtime_controller_launches_from_attached_workspace_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let session_id = Uuid::from_bytes([0x55; 16]);
        let workspace_path = temp.path().join("workspace").to_string_lossy().into_owned();
        let project_assets_root = PathBuf::from(&workspace_path).join("assets");
        let workspace_snapshot = test_workspace_snapshot(
            77,
            workspace_path.clone(),
            project_assets_root.to_string_lossy(),
        );
        let project_rpc = Rc::new(project_host_rpc_for_session(
            ProjectHost::new(),
            temp.path().join("project-host"),
            session_id,
        ));
        let runtime_rpc = Rc::new(runtime_host_rpc(session_id));
        let controller = EditorRuntimeController::new(
            "local.editor_runtime",
            "lighting",
            temp.path().to_string_lossy(),
            workspace_path,
            "az/editor-world",
            project_host_client_from_rpc_for_session(&project_rpc, session_id),
            runtime_host_client_from_rpc(&runtime_rpc, session_id),
        )
        .with_session_id(session_id)
        .with_asset_processor(Some(asset_client_with_workspace_snapshot(
            workspace_snapshot,
            session_id,
        )));

        let launched =
            futures::executor::block_on(controller.launch_attached_editor_world(false)).unwrap();

        assert_eq!(launched.runtime_id, EDITOR_WORLD_RUNTIME_ID);
        let snapshot = runtime_rpc
            .host()
            .launched_snapshot(EDITOR_WORLD_RUNTIME_ID)
            .unwrap();
        assert_eq!(snapshot.workspace_id, 77);
        assert_eq!(snapshot.asset_source_roots.len(), 2);
        assert!(
            snapshot
                .asset_source_roots
                .iter()
                .any(|root| root.portable_key == "project:local.editor_runtime:assets")
        );
        assert!(!snapshot.include_unsaved_journal);
    }

    #[test]
    fn connects_to_runtime_host_over_endpoint_transport() {
        let session_id = Uuid::from_bytes([0x65; 16]);
        let mut descriptor = ServiceDescriptor::new(
            runtime_host_service_id(),
            ServiceRole::RuntimeHost,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .with_run(Uuid::now_v7())
        .with_capability(
            Capability::new(
                ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                ServiceRole::Editor,
            )
            .with_session(session_id)
            .with_audience(RUNTIME_HOST_AUDIENCE)
            .with_permissions([RUNTIME_READ_PERMISSION])
            .with_token_hash([0x72, 0x65]),
        );
        let server = start_runtime_host_rpc_server_for_session_with_capability_grants(
            descriptor.endpoint.clone(),
            session_id.to_string(),
            std::env::temp_dir().join(format!("az-editor-runtime-host-test-{session_id}")),
            CapabilityGrantSet::from_grants(descriptor.capabilities.clone()),
            composed(),
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
            let client = RuntimeHostClient::connect_for_session(&descriptor, session_id)
                .await
                .unwrap();
            let status = client.status("missing").await.unwrap();
            assert_eq!(status, None);
        });

        server.stop().expect("stop runtime-host server");
    }
}
