//! Editor-side `az-sessiond` client.
//!
//! The editor uses this boundary for the runtime-host and session lifecycle.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
#[cfg(not(test))]
use std::thread;

use az_editor_ui::actions::SessionServiceLogStream;
use az_editor_ui::panels::{
    ConsoleState, EditorSessionStateData, EditorSessionStatus, LogLevel, OutputLogState,
    SessionProcessData, SessionProcessStateData, SessionServiceRoleData,
};
use az_proto_asset::{ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME, WorkspaceEntryDiff};
use az_proto_core::{Capability, ServiceDescriptor, ServiceId, ServiceRole};
use az_proto_project::DocumentId;
#[cfg(test)]
use az_proto_project::{DocumentRevision, PROJECT_HOST_AUDIENCE};
use az_proto_session::{
    ExecCommandRequest, ExecCommandResult, RecoverSessionRequest, ResolveServiceRequest,
    RuntimeAssetPackageRootsRequest, RuntimeAssetPackageRootsResult, SESSION_EXEC_PERMISSION,
    SESSION_READ_PERMISSION, SESSION_SAVE_PERMISSION, SESSION_SUPERVISOR_AUDIENCE,
    SESSION_SUPERVISOR_NAMESPACE, SESSION_SUPERVISOR_SERVICE_NAME, SaveGraphDocumentRequest,
    SaveGraphDocumentResult, ServiceLogRequest, ServiceLogResult, ServiceLogStream,
    ServiceProcessState as ProtoServiceProcessState, SessionCapabilityRequest, SessionManifest,
    SessionSlugRequest, SessionState as ProtoSessionState, SessionSupervisorEvent,
    SessionSupervisorEventSubscriptionRequest, SessionSupervisorEventSubscriptionResult,
    SessionWorkspaceStatus, StartServicesRequest, StartServicesResult, StopServicesRequest,
    StopServicesResult, session_capnp,
};
use gpui::App;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::{error, info};
use uuid::Uuid;

use crate::error::{EditorError, EditorResult};
use crate::project_host::{
    EDITOR_SERVICE_NAME, EDITOR_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME,
    PROJECT_HOST_SERVICE_NAMESPACE,
};
use crate::runtime_host::runtime_host_service_id;
use crate::service_descriptor::{
    validate_descriptor_capability_templates, validate_session_descriptor_capability_templates,
};
use crate::watch_handle::WatchHandle;

/// Which slice of one service's log to read.
///
/// The service name, launch, and stream select the file; `all`, `tail_lines`,
/// and `offset` are a single windowing decision over it — a whole-stream read
/// ignores the tail window entirely. The supervisor echoes these back in its
/// reply so the answer can be checked against the question, which is why they
/// travel as one named query rather than as loose flags.
#[derive(Debug, Clone)]
pub struct ServiceLogQuery {
    pub service_name: String,
    pub run: Option<Uuid>,
    pub stream: ServiceLogStream,
    pub all: bool,
    pub tail_lines: u32,
    pub offset: Option<u64>,
}

#[derive(Clone)]
pub struct SessionSupervisorClient {
    client: session_capnp::session_supervisor::Client,
    editor_service: ServiceId,
    session_id: Option<Uuid>,
    capability_templates: Vec<Capability>,
    descriptor_capabilities_required: bool,
}

impl SessionSupervisorClient {
    #[cfg(test)]
    #[must_use]
    pub fn new(client: session_capnp::session_supervisor::Client) -> Self {
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
        client: session_capnp::session_supervisor::Client,
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

    /// Connect to a session-supervisor for cross-session discovery, leaving the
    /// client unbound to any one session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when `descriptor` does not name
    /// the session-supervisor service id and role, when it claims a protocol
    /// version other than the current one, or when its brokered capability
    /// templates are empty or tokenless. Returns [`EditorError::RpcTransport`]
    /// when the descriptor's endpoint cannot be dialed.
    pub async fn connect_for_discovery(descriptor: &ServiceDescriptor) -> EditorResult<Self> {
        validate_session_supervisor_descriptor(descriptor)?;
        let client = az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
        Ok(Self::from_descriptor_client(
            client,
            descriptor.capabilities.clone(),
        ))
    }

    /// Connect to a session-supervisor already scoped to `session_id`, so every
    /// later call writes a session-bound capability.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] for everything
    /// [`Self::connect_for_discovery`] rejects, and additionally when
    /// `session_id` is nil or any brokered capability template on the descriptor
    /// is unscoped or scoped to a different session. Returns
    /// [`EditorError::RpcTransport`] when the descriptor's endpoint cannot be
    /// dialed.
    pub async fn connect_for_session(
        descriptor: &ServiceDescriptor,
        session_id: Uuid,
    ) -> EditorResult<Self> {
        validate_session_supervisor_descriptor_for_session(descriptor, session_id)?;
        let client = az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
        Ok(
            Self::from_descriptor_client(client, descriptor.capabilities.clone())
                .with_session_scope(session_id),
        )
    }

    /// Ask the session-supervisor for the descriptor of one service registered
    /// in `session_slug`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client was built from
    /// a descriptor that granted no read capability. Returns
    /// [`EditorError::ServiceProtocol`] when the request cannot be encoded, the
    /// call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when the returned descriptor is
    /// for a different service id or role, when its capability templates are
    /// invalid, or when a template targets a session other than this client's.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn resolve_service_descriptor(
        &self,
        session_slug: &str,
        id: &ServiceId,
        role: ServiceRole,
    ) -> EditorResult<ServiceDescriptor> {
        let mut request = self.client.resolve_service_request();
        (ResolveServiceRequest {
            capability: self.session_capability([SESSION_READ_PERMISSION])?,
            slug: session_slug.to_string(),
            id: id.clone(),
            role,
        })
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        let descriptor =
            az_proto_core::ServiceDescriptor::from_capnp(response.get()?.get_descriptor()?)?;
        ensure_resolved_service_descriptor_matches_request(&descriptor, id, role, self.session_id)?;
        Ok(descriptor)
    }

    /// Read the current workspace status of `session_slug`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no read
    /// capability, and [`EditorError::ServiceProtocol`] when the request cannot
    /// be encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when the returned manifest is
    /// for another session, carries a nil id, empty identity or path fields, or
    /// duplicate service descriptors, or when a failed-preserved session comes
    /// back without a failure reason.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn status(&self, session_slug: &str) -> EditorResult<SessionWorkspaceStatus> {
        let mut request = self.client.status_request();
        SessionSlugRequest {
            capability: self.session_capability([SESSION_READ_PERMISSION])?,
            slug: session_slug.to_string(),
        }
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        let status = SessionWorkspaceStatus::from_capnp(response.get()?.get_status()?)?;
        ensure_session_workspace_status_matches_request(&status, session_slug, "status")?;
        Ok(status)
    }

    /// Read the DB-owned asset package roots a runtime launch for
    /// `session_slug` must mount.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no read
    /// capability, and [`EditorError::ServiceProtocol`] when the request cannot
    /// be encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when the returned roots do not
    /// pass runtime asset-package-root validation.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn runtime_asset_package_roots(
        &self,
        session_slug: &str,
    ) -> EditorResult<RuntimeAssetPackageRootsResult> {
        let mut request = self.client.runtime_asset_package_roots_request();
        (RuntimeAssetPackageRootsRequest {
            capability: self.session_capability([SESSION_READ_PERMISSION])?,
            slug: session_slug.to_string(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = RuntimeAssetPackageRootsResult::from_capnp(response.get()?.get_result()?)?;
        ensure_runtime_asset_package_roots_are_well_formed(&result, "runtimeAssetPackageRoots")?;
        Ok(result)
    }

    /// List every session manifest this supervisor owns.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no read
    /// capability, and [`EditorError::ServiceProtocol`] when the request cannot
    /// be encoded, the call fails, or any manifest in the response cannot be
    /// decoded. Returns [`EditorError::ServiceAuthorityMismatch`] when a listed
    /// manifest has a nil id, empty identity or path fields, an invalid or
    /// duplicated service descriptor, or when the list repeats a session id or
    /// slug.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn list_sessions(&self) -> EditorResult<Vec<SessionManifest>> {
        let mut request = self.client.list_request();
        (SessionCapabilityRequest {
            capability: self.session_capability([SESSION_READ_PERMISSION])?,
        })
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        let sessions = response
            .get()?
            .get_sessions()?
            .iter()
            .map(SessionManifest::from_capnp)
            .collect::<Result<Vec<_>, capnp::Error>>()?;
        ensure_session_manifest_list_is_well_formed(&sessions)?;
        Ok(sessions)
    }

    /// Pick one session's manifest out of the supervisor's session list.
    ///
    /// # Errors
    ///
    /// Returns anything [`Self::list_sessions`] returns, plus
    /// [`EditorError::ServiceDiscovery`] when the list contains no session whose
    /// slug is `session_slug`.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn session_manifest(&self, session_slug: &str) -> EditorResult<SessionManifest> {
        self.list_sessions()
            .await?
            .into_iter()
            .find(|manifest| manifest.slug == session_slug)
            .ok_or_else(|| {
                crate::error::EditorError::ServiceDiscovery(format!(
                    "session `{session_slug}` was not listed by session-supervisor"
                ))
            })
    }

    /// Resolve the project-host descriptor registered in `session_slug`.
    ///
    /// # Errors
    ///
    /// Returns anything [`Self::resolve_service_descriptor`] returns for the
    /// project-host service id and [`ServiceRole::ProjectHost`].
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn resolve_project_host_descriptor(
        &self,
        session_slug: &str,
    ) -> EditorResult<ServiceDescriptor> {
        self.resolve_service_descriptor(
            session_slug,
            &ServiceId::new(PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
        )
        .await
    }

    /// Resolve the asset-processor descriptor registered in `session_slug`.
    ///
    /// # Errors
    ///
    /// Returns anything [`Self::resolve_service_descriptor`] returns for the
    /// asset-processor service id and [`ServiceRole::AssetProcessor`].
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn resolve_asset_processor_descriptor(
        &self,
        session_slug: &str,
    ) -> EditorResult<ServiceDescriptor> {
        self.resolve_service_descriptor(
            session_slug,
            &ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
            ServiceRole::AssetProcessor,
        )
        .await
    }

    /// Resolve the runtime-host descriptor registered in `session_slug`.
    ///
    /// # Errors
    ///
    /// Returns anything [`Self::resolve_service_descriptor`] returns for the
    /// runtime-host service id and [`ServiceRole::RuntimeHost`].
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn resolve_runtime_host_descriptor(
        &self,
        session_slug: &str,
    ) -> EditorResult<ServiceDescriptor> {
        self.resolve_service_descriptor(
            session_slug,
            &runtime_host_service_id(),
            ServiceRole::RuntimeHost,
        )
        .await
    }

    /// Save one authored graph document through the session-supervisor, which
    /// owns the write and returns the resulting workspace asset record.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no save
    /// capability, and [`EditorError::ServiceProtocol`] when the request cannot
    /// be encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when the reply does not echo
    /// `document_id` as both the saved document and its source path, when the
    /// saved metadata is unusable (blank schema type, a content hash that is not
    /// 32 bytes, or a zero byte length), when the asset record carries a nil
    /// asset guid or non-positive entry, workspace, or root identifiers, when
    /// the record's source path, schema type, or content hash disagrees with the
    /// saved document, or when the workspace entry comes back deleted,
    /// conflicted, or with a negative diagnostics count.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn save_graph_document(
        &self,
        session_slug: &str,
        document_id: &DocumentId,
    ) -> EditorResult<SaveGraphDocumentResult> {
        let mut request = self.client.save_graph_document_request();
        (SaveGraphDocumentRequest {
            capability: self.session_capability([SESSION_SAVE_PERMISSION])?,
            slug: session_slug.to_string(),
            document_id: document_id.as_str().to_string(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = SaveGraphDocumentResult::from_capnp(response.get()?.get_result()?)?;
        ensure_save_graph_document_result_matches_request(&result, document_id)?;
        Ok(result)
    }

    /// Run one command inside `session_slug` and return its bounded output.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no exec
    /// capability, and [`EditorError::ServiceProtocol`] when the request cannot
    /// be encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when the result contradicts
    /// itself — a success that never exited or reports a non-zero exit code, a
    /// non-exited result carrying an exit code — or when stdout or stderr comes
    /// back longer than the lossy-UTF-8 headroom over `max_output_bytes`. The
    /// command's own non-zero exit is reported in the result, not as an error.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn exec_command(
        &self,
        session_slug: &str,
        program: impl Into<String>,
        args: impl IntoIterator<Item = String>,
        max_output_bytes: u32,
    ) -> EditorResult<ExecCommandResult> {
        let mut request = self.client.exec_command_request();
        (ExecCommandRequest {
            capability: self.session_capability([SESSION_EXEC_PERMISSION])?,
            slug: session_slug.to_string(),
            program: program.into(),
            args: args.into_iter().collect(),
            max_output_bytes,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = ExecCommandResult::from_capnp(response.get()?.get_result()?)?;
        ensure_exec_command_result_matches_request(&result, max_output_bytes)?;
        Ok(result)
    }

    /// Stop the services running in `session_slug`, recording `reason`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no
    /// manage capability, and [`EditorError::ServiceProtocol`] when the request
    /// cannot be encoded, the call fails, or the response cannot be decoded.
    /// Returns [`EditorError::ServiceAuthorityMismatch`] when the status in the
    /// reply is for another session, fails manifest identity validation, or is
    /// failed-preserved without a failure reason.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn stop_services(
        &self,
        session_slug: &str,
        reason: impl Into<String>,
    ) -> EditorResult<StopServicesResult> {
        let reason = reason.into();
        let mut request = self.client.stop_services_request();
        (StopServicesRequest {
            capability: self.session_capability([az_proto_session::SESSION_MANAGE_PERMISSION])?,
            slug: session_slug.to_string(),
            reason: reason.clone(),
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = StopServicesResult::from_capnp(response.get()?.get_result()?)?;
        ensure_session_workspace_status_matches_request(
            &result.status,
            session_slug,
            "stopServices",
        )?;
        Ok(result)
    }

    /// Start every service of `session_slug`, recording `reason`.
    ///
    /// # Errors
    ///
    /// Returns anything [`Self::start_services_for`] returns for an empty
    /// service-name list.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn start_services(
        &self,
        session_slug: &str,
        reason: impl Into<String>,
    ) -> EditorResult<StartServicesResult> {
        self.start_services_for(session_slug, reason, Vec::new())
            .await
    }

    /// Start the named services of `session_slug`, recording `reason`. An empty
    /// `service_names` starts the session's whole service set.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no
    /// manage capability, and [`EditorError::ServiceProtocol`] when the request
    /// cannot be encoded, the call fails, or the response cannot be decoded.
    /// Returns [`EditorError::ServiceAuthorityMismatch`] when the status in the
    /// reply is for another session, fails manifest identity validation, or is
    /// failed-preserved without a failure reason.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn start_services_for(
        &self,
        session_slug: &str,
        reason: impl Into<String>,
        service_names: Vec<String>,
    ) -> EditorResult<StartServicesResult> {
        let reason = reason.into();
        let mut request = self.client.start_services_request();
        (StartServicesRequest {
            capability: self.session_capability([az_proto_session::SESSION_MANAGE_PERMISSION])?,
            slug: session_slug.to_string(),
            reason: reason.clone(),
            service_names,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = StartServicesResult::from_capnp(response.get()?.get_result()?)?;
        ensure_session_workspace_status_matches_request(
            &result.status,
            session_slug,
            "startServices",
        )?;
        Ok(result)
    }

    /// Terminate the supervisor of `session_slug` after its services have
    /// already been stopped, recording `reason`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no
    /// manage capability, and [`EditorError::ServiceProtocol`] when the
    /// capability cannot be written into the request or the call fails in
    /// transport. There is no reply to validate, so nothing else can fail here.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn shutdown_supervisor(
        &self,
        session_slug: &str,
        reason: impl Into<String>,
    ) -> EditorResult<()> {
        let mut request = self.client.shutdown_supervisor_request();
        let mut builder = request.get();
        builder.set_slug(session_slug);
        builder.set_reason(reason.into());
        self.session_capability([az_proto_session::SESSION_MANAGE_PERMISSION])?
            .to_capnp(builder.init_capability())?;
        // A streaming call has no result payload. Awaiting this promise admits
        // transport flow control (and executes in-process test clients), not a
        // reply from the endpoint this request terminates.
        request.send().await?;
        Ok(())
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn subscribe_status_events(
        &self,
        session_slug: &str,
        events: CoalescedLatestSender<SessionSupervisorEvent>,
    ) -> EditorResult<SessionSupervisorEventSubscriptionResult> {
        let sink = capnp_rpc::new_client(EditorSessionSupervisorEventSink { events });
        let mut request = self.client.subscribe_events_request();
        {
            let mut params = request.get();
            (SessionSupervisorEventSubscriptionRequest {
                capability: self.session_capability([SESSION_READ_PERMISSION])?,
                slug: session_slug.to_string(),
            })
            .to_capnp(params.reborrow().init_request())?;
            params.set_sink(sink);
        }
        let response = request.send().promise.await?;
        let result =
            SessionSupervisorEventSubscriptionResult::from_capnp(response.get()?.get_result()?)?;
        if !result.subscribed {
            return Err(EditorError::ServiceDiscovery(
                "session-supervisor declined status subscription".to_string(),
            ));
        }
        ensure_session_workspace_status_matches_request(
            &result.initial_status,
            session_slug,
            "subscribeEvents",
        )?;
        Ok(result)
    }

    /// Recover a failed-preserved session and return its manifest.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no
    /// manage capability, and [`EditorError::ServiceProtocol`] when the request
    /// cannot be encoded, the call fails, or the response cannot be decoded.
    /// Returns [`EditorError::ServiceAuthorityMismatch`] when the recovered
    /// manifest is for another session, has a nil id, empty identity or path
    /// fields, or an invalid or duplicated service descriptor.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn recover(&self, session_slug: &str, force: bool) -> EditorResult<SessionManifest> {
        let mut request = self.client.recover_request();
        (RecoverSessionRequest {
            capability: self.session_capability([az_proto_session::SESSION_MANAGE_PERMISSION])?,
            slug: session_slug.to_string(),
            force,
        })
        .to_capnp(request.get())?;

        let response = request.send().promise.await?;
        let manifest = SessionManifest::from_capnp(response.get()?.get_manifest()?)?;
        ensure_session_manifest_matches_request(&manifest, session_slug, "recover")?;
        Ok(manifest)
    }

    /// Read one service's log for `session_slug`, either the tail or the whole
    /// stream.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this client holds no read
    /// capability, and [`EditorError::ServiceProtocol`] when the request cannot
    /// be encoded, the call fails, or the response cannot be decoded. Returns
    /// [`EditorError::ServiceAuthorityMismatch`] when the reply does not echo
    /// the requested session slug, service name, stream, or launch, or when it
    /// carries a nil launch identifier or a blank log path.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn service_log(
        &self,
        session_slug: &str,
        query: ServiceLogQuery,
    ) -> EditorResult<ServiceLogResult> {
        let ServiceLogQuery {
            service_name,
            run,
            stream,
            all,
            tail_lines,
            offset,
        } = query;
        let mut request = self.client.service_log_request();
        (ServiceLogRequest {
            capability: self.session_capability([SESSION_READ_PERMISSION])?,
            slug: session_slug.to_string(),
            service_name: service_name.clone(),
            run,
            stream,
            all,
            tail_lines,
            offset,
        })
        .to_capnp(request.get().init_request())?;

        let response = request.send().promise.await?;
        let result = ServiceLogResult::from_capnp(response.get()?.get_result()?)?;
        ensure_service_log_result_matches_request(
            &result,
            session_slug,
            &service_name,
            run,
            stream,
        )?;
        Ok(result)
    }

    fn session_capability(
        &self,
        permissions: impl IntoIterator<Item = &'static str>,
    ) -> EditorResult<Capability> {
        let permissions: Vec<&'static str> = permissions.into_iter().collect();
        if self.descriptor_capabilities_required {
            return self
                .capability_template(&permissions)
                .ok_or_else(|| Self::missing_capability_error(&permissions));
        }

        let capability = Capability::new(self.editor_service.clone(), ServiceRole::Editor)
            .with_audience(SESSION_SUPERVISOR_AUDIENCE)
            .with_permissions(permissions);
        Ok(match self.session_id {
            Some(session_id) => capability.with_session(session_id),
            None => capability,
        })
    }

    fn capability_template(&self, permissions: &[&str]) -> Option<Capability> {
        self.capability_templates
            .iter()
            .find(|capability| {
                capability.matches_brokered_template_request(
                    ServiceRole::Editor,
                    SESSION_SUPERVISOR_AUDIENCE,
                    permissions,
                    self.session_id,
                )
            })
            .cloned()
            .map(|capability| capability.scoped_to(self.session_id))
    }

    fn missing_capability_error(permissions: &[&str]) -> crate::error::EditorError {
        crate::error::EditorError::ServiceDiscovery(format!(
            "session-supervisor descriptor did not grant editor capability `{}`",
            permissions.join(", ")
        ))
    }

    #[must_use]
    fn from_descriptor_client(
        client: session_capnp::session_supervisor::Client,
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

struct EditorSessionSupervisorEventSink {
    events: CoalescedLatestSender<SessionSupervisorEvent>,
}

impl session_capnp::session_supervisor_event_sink::Server for EditorSessionSupervisorEventSink {
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn update(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor_event_sink::UpdateParams,
        _results: session_capnp::session_supervisor_event_sink::UpdateResults,
    ) -> Result<(), capnp::Error> {
        let event = SessionSupervisorEvent::from_capnp(params.get()?.get_event()?)?;
        self.events.publish(event).then_some(()).ok_or_else(|| {
            capnp::Error::failed("editor session-status receiver closed".to_string())
        })
    }
}

/// One-wake, latest-value handoff for a producer whose consumer only needs the
/// newest canonical snapshot. A full wake channel means a consumer is already
/// scheduled; it never grows a second backlog.
#[derive(Clone)]
struct CoalescedLatestSender<T> {
    latest: Arc<Mutex<Option<T>>>,
    wake: mpsc::Sender<()>,
}

struct CoalescedLatestReceiver<T> {
    latest: Arc<Mutex<Option<T>>>,
    wake: mpsc::Receiver<()>,
}

impl<T> CoalescedLatestSender<T> {
    fn channel() -> (Self, CoalescedLatestReceiver<T>) {
        let (wake, receiver) = mpsc::channel(1);
        let latest = Arc::new(Mutex::new(None));
        (
            Self {
                latest: Arc::clone(&latest),
                wake,
            },
            CoalescedLatestReceiver {
                latest,
                wake: receiver,
            },
        )
    }

    /// Replaces a stalled consumer's pending value and ensures exactly one
    /// wake is queued. `false` means the sole consumer has gone away.
    fn publish(&self, value: T) -> bool {
        let Ok(mut latest) = self.latest.lock() else {
            return false;
        };
        *latest = Some(value);
        drop(latest);
        match self.wake.try_send(()) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(())) => true,
            Err(mpsc::error::TrySendError::Closed(())) => false,
        }
    }
}

impl<T> CoalescedLatestReceiver<T> {
    async fn recv(&mut self) -> Option<T> {
        loop {
            match self.wake.recv().await {
                Some(()) => {
                    let Ok(mut latest) = self.latest.lock() else {
                        return None;
                    };
                    if let Some(value) = latest.take() {
                        return Some(value);
                    }
                }
                None => {
                    return self.latest.lock().ok()?.take();
                }
            }
        }
    }
}

#[derive(Debug)]
enum SessionStatusDelivery {
    Status(EditorSessionStatus),
    Terminal(EditorError),
}

type SessionStatusSender = CoalescedLatestSender<SessionStatusDelivery>;
type SessionStatusReceiver = CoalescedLatestReceiver<SessionStatusDelivery>;

impl SessionStatusDelivery {
    const fn status(status: EditorSessionStatus) -> Self {
        Self::Status(status)
    }

    const fn terminal(error: EditorError) -> Self {
        Self::Terminal(error)
    }
}

#[must_use]
pub fn session_supervisor_service_id() -> ServiceId {
    ServiceId::new(
        SESSION_SUPERVISOR_NAMESPACE,
        SESSION_SUPERVISOR_SERVICE_NAME,
    )
}

pub struct EditorSessionStatusController {
    #[cfg(test)]
    supervisor: Option<SessionSupervisorClient>,
    supervisor_descriptor: Option<ServiceDescriptor>,
    session_id: Option<Uuid>,
    session_slug: String,
    attached_identity: Option<AttachedSessionIdentity>,
    /// Cancellation handle for this controller's status watcher. Replacing
    /// the aggregate-owned controller slot drops the last reference and fires the
    /// watcher's shutdown channel. Held purely for that `Drop` effect, so it
    /// is deliberately never read.
    #[allow(
        dead_code,
        reason = "RAII cancellation guard; its effect is Drop, not reads"
    )]
    watch: Option<Arc<WatchHandle>>,
}

impl Clone for EditorSessionStatusController {
    fn clone(&self) -> Self {
        Self {
            #[cfg(test)]
            supervisor: self.supervisor.clone(),
            supervisor_descriptor: self.supervisor_descriptor.clone(),
            session_id: self.session_id,
            session_slug: self.session_slug.clone(),
            attached_identity: self.attached_identity.clone(),
            // Command copies must never keep the aggregate-owned watcher
            // cancellation handle alive.
            watch: None,
        }
    }
}

impl EditorSessionStatusController {
    #[cfg(test)]
    #[must_use]
    pub fn new(supervisor: SessionSupervisorClient, session_slug: impl Into<String>) -> Self {
        Self {
            supervisor: Some(supervisor),
            supervisor_descriptor: None,
            session_id: None,
            session_slug: session_slug.into(),
            attached_identity: None,
            watch: None,
        }
    }

    /// Build a status controller bound to the identity of an attached session.
    ///
    /// # Errors
    ///
    /// Never returns an error today: this only copies the attached session's
    /// descriptor and identity, and opens no transport. The fallible signature
    /// matches the other attached controllers, whose installers all handle the
    /// same shape, and it is where a future descriptor check would surface.
    pub fn connect_attached(session: &crate::EditorAttachSession) -> EditorResult<Self> {
        Ok(Self {
            #[cfg(test)]
            supervisor: None,
            supervisor_descriptor: Some(session.session_supervisor.clone()),
            session_id: Some(session.session_id),
            session_slug: session.session_slug.clone(),
            attached_identity: Some(AttachedSessionIdentity::from_attach_session(session)),
            watch: None,
        })
    }

    #[must_use]
    pub fn session_slug(&self) -> &str {
        &self.session_slug
    }

    /// Only the real (non-test) watcher installs a handle; under `cfg(test)`
    /// the watcher thread is not spawned, so this has no caller there.
    #[cfg_attr(test, allow(dead_code))]
    fn attach_watch(&mut self, watch: Arc<WatchHandle>) {
        self.watch = Some(watch);
    }

    /// The controller copy handed to the watcher thread, with the watch
    /// handle stripped so the thread cannot keep its own shutdown sender
    /// alive. Same `cfg(test)` caveat as [`Self::attach_watch`].
    #[cfg_attr(test, allow(dead_code))]
    fn watch_target(&self) -> Self {
        self.clone()
    }

    /// Read the attached session's status and project it into the UI shape.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this controller has no
    /// session-supervisor descriptor or session id to dial with, plus anything
    /// [`SessionSupervisorClient::connect_for_session`] and
    /// [`SessionSupervisorClient::status`] return. Returns
    /// [`EditorError::SessionDiscoveryMismatch`] when the status names a
    /// session id, slug, project, project root, workspace root, or run
    /// directory other than the attached session's.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn refresh(&self) -> EditorResult<EditorSessionStatus> {
        let supervisor = self.supervisor_client().await?;
        self.refresh_with(&supervisor).await
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn refresh_with(
        &self,
        supervisor: &SessionSupervisorClient,
    ) -> EditorResult<EditorSessionStatus> {
        let status = supervisor.status(&self.session_slug).await?;
        self.ensure_status_matches_attached_identity(&status, "session status refresh")?;
        Ok(session_workspace_status_to_ui(status))
    }

    /// Stop the attached session's services, recording `reason`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this controller has no
    /// session-supervisor descriptor or session id to dial with, plus anything
    /// [`SessionSupervisorClient::connect_for_session`] and
    /// [`SessionSupervisorClient::stop_services`] return.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn stop_services(
        &self,
        reason: impl Into<String>,
    ) -> EditorResult<StopServicesResult> {
        self.supervisor_client()
            .await?
            .stop_services(&self.session_slug, reason)
            .await
    }

    /// Terminate the attached session's supervisor, recording `reason`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this controller has no
    /// session-supervisor descriptor or session id to dial with, plus anything
    /// [`SessionSupervisorClient::connect_for_session`] and
    /// [`SessionSupervisorClient::shutdown_supervisor`] return.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn shutdown_supervisor(&self, reason: impl Into<String>) -> EditorResult<()> {
        self.supervisor_client()
            .await?
            .shutdown_supervisor(&self.session_slug, reason)
            .await
    }

    /// Start the attached session's services, recording `reason`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this controller has no
    /// session-supervisor descriptor or session id to dial with, plus anything
    /// [`SessionSupervisorClient::connect_for_session`] and
    /// [`SessionSupervisorClient::start_services`] return.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn start_services(
        &self,
        reason: impl Into<String>,
    ) -> EditorResult<StartServicesResult> {
        self.supervisor_client()
            .await?
            .start_services(&self.session_slug, reason)
            .await
    }

    /// Start the named services of the attached session, recording `reason`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this controller has no
    /// session-supervisor descriptor or session id to dial with, plus anything
    /// [`SessionSupervisorClient::connect_for_session`] and
    /// [`SessionSupervisorClient::start_services_for`] return.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn start_services_for(
        &self,
        reason: impl Into<String>,
        service_names: Vec<String>,
    ) -> EditorResult<StartServicesResult> {
        self.supervisor_client()
            .await?
            .start_services_for(&self.session_slug, reason, service_names)
            .await
    }

    /// Recover the attached session and return its manifest.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this controller has no
    /// session-supervisor descriptor or session id to dial with, plus anything
    /// [`SessionSupervisorClient::connect_for_session`] and
    /// [`SessionSupervisorClient::recover`] return. Returns
    /// [`EditorError::SessionDiscoveryMismatch`] when the recovered manifest
    /// names a session id, slug, project, project root, workspace root, or run
    /// directory other than the attached session's.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn recover(&self, force: bool) -> EditorResult<SessionManifest> {
        let supervisor = self.supervisor_client().await?;
        let manifest = supervisor.recover(&self.session_slug, force).await?;
        self.ensure_manifest_matches_attached_identity(&manifest, "session recovery")?;
        Ok(manifest)
    }

    /// Read the last lines of one service log for the attached session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this controller has no
    /// session-supervisor descriptor or session id to dial with, plus anything
    /// [`SessionSupervisorClient::connect_for_session`] and
    /// [`SessionSupervisorClient::service_log`] return. Returns
    /// [`EditorError::SessionDiscoveryMismatch`] when the reply names a
    /// different session slug, or when its log path is blank, contains a parent
    /// component, or resolves outside the attached session's run directory.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn service_log_tail(
        &self,
        service_name: impl Into<String>,
        run: Option<Uuid>,
        stream: SessionServiceLogStream,
    ) -> EditorResult<ServiceLogResult> {
        let supervisor = self.supervisor_client().await?;
        let result = supervisor
            .service_log(
                &self.session_slug,
                ServiceLogQuery {
                    service_name: service_name.into(),
                    run,
                    stream: service_log_stream_to_proto(stream),
                    all: false,
                    tail_lines: DEFAULT_SERVICE_LOG_TAIL_LINES,
                    offset: None,
                },
            )
            .await?;
        self.ensure_response_slug_matches_attached_identity(
            &result.session_slug,
            "session service log",
        )?;
        self.ensure_service_log_matches_attached_identity(&result, "session service log")?;
        Ok(result)
    }

    /// Read one service log for the attached session in full, from the start.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] when this controller has no
    /// session-supervisor descriptor or session id to dial with, plus anything
    /// [`SessionSupervisorClient::connect_for_session`] and
    /// [`SessionSupervisorClient::service_log`] return. Returns
    /// [`EditorError::SessionDiscoveryMismatch`] when the reply names a
    /// different session slug, or when its log path is blank, contains a parent
    /// component, or resolves outside the attached session's run directory.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn service_log_all(
        &self,
        service_name: impl Into<String>,
        run: Option<Uuid>,
        stream: SessionServiceLogStream,
    ) -> EditorResult<ServiceLogResult> {
        let supervisor = self.supervisor_client().await?;
        let result = supervisor
            .service_log(
                &self.session_slug,
                ServiceLogQuery {
                    service_name: service_name.into(),
                    run,
                    stream: service_log_stream_to_proto(stream),
                    all: true,
                    tail_lines: 0,
                    offset: None,
                },
            )
            .await?;
        self.ensure_response_slug_matches_attached_identity(
            &result.session_slug,
            "session service log",
        )?;
        self.ensure_service_log_matches_attached_identity(&result, "session service log")?;
        Ok(result)
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn supervisor_client(&self) -> EditorResult<SessionSupervisorClient> {
        #[cfg(test)]
        if let Some(supervisor) = &self.supervisor {
            return Ok(supervisor.clone());
        }

        let Some(descriptor) = &self.supervisor_descriptor else {
            return Err(EditorError::ServiceDiscovery(
                "session status controller requires a session-supervisor descriptor".to_string(),
            ));
        };
        let Some(session_id) = self.session_id else {
            return Err(EditorError::ServiceDiscovery(
                "session status controller requires an attached session id".to_string(),
            ));
        };
        SessionSupervisorClient::connect_for_session(descriptor, session_id).await
    }

    #[cfg(test)]
    #[must_use]
    fn with_attached_identity(mut self, identity: AttachedSessionIdentity) -> Self {
        self.attached_identity = Some(identity);
        self
    }

    fn ensure_status_matches_attached_identity(
        &self,
        status: &SessionWorkspaceStatus,
        operation: &'static str,
    ) -> EditorResult<()> {
        self.ensure_manifest_matches_attached_identity(&status.manifest, operation)
    }

    fn ensure_manifest_matches_attached_identity(
        &self,
        manifest: &SessionManifest,
        operation: &'static str,
    ) -> EditorResult<()> {
        self.attached_identity.as_ref().map_or_else(
            || Ok(()),
            |identity| identity.ensure_manifest(manifest, operation),
        )
    }

    fn ensure_response_slug_matches_attached_identity(
        &self,
        response_slug: &str,
        operation: &'static str,
    ) -> EditorResult<()> {
        self.attached_identity.as_ref().map_or_else(
            || Ok(()),
            |identity| identity.ensure_response_slug(response_slug, operation),
        )
    }

    fn ensure_service_log_matches_attached_identity(
        &self,
        result: &ServiceLogResult,
        operation: &'static str,
    ) -> EditorResult<()> {
        self.attached_identity.as_ref().map_or_else(
            || Ok(()),
            |identity| identity.ensure_service_log(result, operation),
        )
    }
}

#[derive(Clone, Debug)]
struct AttachedSessionIdentity {
    session_id: Uuid,
    project_id: String,
    session_slug: String,
    project_root: PathBuf,
    workspace_root: PathBuf,
    run_dir: PathBuf,
}

impl AttachedSessionIdentity {
    fn from_attach_session(session: &crate::EditorAttachSession) -> Self {
        Self {
            session_id: session.session_id,
            project_id: session.project_id.clone(),
            session_slug: session.session_slug.clone(),
            project_root: session.project_root.clone(),
            workspace_root: PathBuf::from(&session.workspace.workspace_root),
            run_dir: session.run_dir.clone(),
        }
    }

    #[cfg(test)]
    fn from_manifest(manifest: &SessionManifest) -> Self {
        Self {
            session_id: manifest.id,
            project_id: manifest.project_id.clone(),
            session_slug: manifest.slug.clone(),
            project_root: manifest.project_root.clone().into(),
            workspace_root: manifest.workspace_root.clone().into(),
            run_dir: manifest.run_dir.clone().into(),
        }
    }

    fn ensure_manifest(
        &self,
        manifest: &SessionManifest,
        operation: &'static str,
    ) -> EditorResult<()> {
        if manifest.id != self.session_id {
            return Err(Self::mismatch(
                manifest,
                operation,
                format!(
                    "response session id `{}` does not match attached session id `{}`",
                    manifest.id, self.session_id
                ),
            ));
        }
        if manifest.slug != self.session_slug {
            return Err(Self::mismatch(
                manifest,
                operation,
                format!(
                    "response session slug `{}` does not match attached session slug `{}`",
                    manifest.slug, self.session_slug
                ),
            ));
        }
        if manifest.project_id != self.project_id {
            return Err(Self::mismatch(
                manifest,
                operation,
                format!(
                    "response project `{}` does not match attached project `{}`",
                    manifest.project_id, self.project_id
                ),
            ));
        }
        if !same_protocol_path(&manifest.project_root, &self.project_root) {
            return Err(Self::mismatch(
                manifest,
                operation,
                format!(
                    "response project root `{}` does not match attached project root `{}`",
                    manifest.project_root,
                    self.project_root.display()
                ),
            ));
        }
        if !same_protocol_path(&manifest.workspace_root, &self.workspace_root) {
            return Err(Self::mismatch(
                manifest,
                operation,
                format!(
                    "response workspace root `{}` does not match attached workspace root `{}`",
                    manifest.workspace_root,
                    self.workspace_root.display()
                ),
            ));
        }
        if !same_protocol_path(&manifest.run_dir, &self.run_dir) {
            return Err(Self::mismatch(
                manifest,
                operation,
                format!(
                    "response run directory `{}` does not match attached run directory `{}`",
                    manifest.run_dir,
                    self.run_dir.display()
                ),
            ));
        }
        Ok(())
    }

    fn ensure_response_slug(
        &self,
        response_slug: &str,
        operation: &'static str,
    ) -> EditorResult<()> {
        if response_slug == self.session_slug {
            Ok(())
        } else {
            Err(EditorError::SessionDiscoveryMismatch {
                operation,
                session: response_slug.to_string(),
                reason: format!(
                    "response session slug `{response_slug}` does not match attached session slug `{}`",
                    self.session_slug
                ),
            })
        }
    }

    fn ensure_service_log(
        &self,
        result: &ServiceLogResult,
        operation: &'static str,
    ) -> EditorResult<()> {
        self.ensure_response_slug(&result.session_slug, operation)?;

        let raw_path = Path::new(&result.path);
        if result.path.trim().is_empty() {
            return Err(EditorError::SessionDiscoveryMismatch {
                operation,
                session: result.session_slug.clone(),
                reason: "service log path cannot be empty".to_string(),
            });
        }

        let path = if raw_path.is_absolute() {
            raw_path.to_path_buf()
        } else {
            self.run_dir.join(raw_path)
        };
        if path_has_parent_component(&path) || !protocol_path_starts_with(&path, &self.run_dir) {
            return Err(EditorError::SessionDiscoveryMismatch {
                operation,
                session: result.session_slug.clone(),
                reason: format!(
                    "service log path `{}` is outside attached session run directory `{}`",
                    path.display(),
                    self.run_dir.display()
                ),
            });
        }

        Ok(())
    }

    fn mismatch(
        manifest: &SessionManifest,
        operation: &'static str,
        reason: String,
    ) -> EditorError {
        EditorError::SessionDiscoveryMismatch {
            operation,
            session: manifest.slug.clone(),
            reason,
        }
    }
}

const DEFAULT_SERVICE_LOG_TAIL_LINES: u32 = 80;
const DEFAULT_EXEC_OUTPUT_LIMIT_BYTES: u32 = 64 * 1024;
pub fn install_session_status_action_handlers(cx: &mut App) {
    cx.on_action(|_: &az_editor_ui::actions::RefreshSessionStatus, cx| {
        if let Err(err) = refresh_session_status(cx) {
            error!(error = %err, "failed to handle session status refresh action");
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::StartSessionServices, cx| {
        if let Err(err) = start_session_services(cx) {
            error!(error = %err, "failed to handle session start-services action");
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::StopSessionServices, cx| {
        if let Err(err) = stop_session_services(cx) {
            error!(error = %err, "failed to handle session stop-services action");
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::RecoverSession, cx| {
        if let Err(err) = recover_session(cx, false) {
            error!(error = %err, "failed to handle session recover action");
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::ForceRecoverSession, cx| {
        if let Err(err) = recover_session(cx, true) {
            error!(error = %err, "failed to handle session force-recover action");
        }
    });
    cx.on_action(
        |action: &az_editor_ui::actions::TailSessionServiceLog, cx| {
            if let Err(err) = tail_session_service_log(cx, action.clone()) {
                error!(error = %err, "failed to handle session service-log action");
            }
        },
    );
}

pub(crate) fn install_session_status_slot(
    cx: &mut App,
    session: crate::EditorAttachSession,
    fence: crate::controller_set::ControllerFence,
) {
    let session_label = session.session_slug.clone();

    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "session-status-install",
        move || async move {
            let controller = EditorSessionStatusController::connect_attached(&session)?;
            let status = controller.refresh().await?;
            Ok((controller, status))
        },
        move |cx, result| match result {
            Ok((mut controller, status)) => {
                let process_count = status.processes.len();
                // Attach the new watcher before publishing the controller:
                // `set_global` then drops the previous controller (if any),
                // firing the previous watcher's shutdown channel.
                spawn_session_status_watch(cx, &mut controller, status.clone(), fence);
                if !crate::controller_set::complete_session_status(cx, fence, controller) {
                    return;
                }
                cx.set_global(status);
                cx.refresh_windows();
                info!(
                    session = %session_label,
                    process_count,
                    "installed session status controller"
                );
            }
            Err(err) => {
                crate::controller_set::fail_controller(cx, fence, err.to_string());
                error!(
                    error = %err,
                    session = %session_label,
                    "failed to connect session status controller"
                );
            }
        },
    );
}

// The real build mutates `controller` through `attach_watch`; only the
// `cfg(test)` build, which never spawns the watcher thread, leaves it unused.
#[cfg_attr(test, allow(clippy::needless_pass_by_ref_mut))] // cfg(test) skips the attach_watch mutation.
fn spawn_session_status_watch(
    cx: &App,
    controller: &mut EditorSessionStatusController,
    initial_status: EditorSessionStatus,
    fence: crate::controller_set::ControllerFence,
) {
    let session_slug = controller.session_slug().to_string();
    let (status_tx, mut status_rx): (SessionStatusSender, SessionStatusReceiver) =
        CoalescedLatestSender::channel();
    #[cfg(test)]
    {
        let _ = status_tx.publish(SessionStatusDelivery::status(initial_status));
    }
    #[cfg(not(test))]
    {
        let (watch_handle, shutdown_rx) = WatchHandle::channel();
        match thread::Builder::new()
            .name(format!("az-editor-session-status-{session_slug}"))
            .spawn({
                let watched = controller.watch_target();
                let thread_session_slug = session_slug.clone();
                move || {
                    run_session_status_watch(
                        watched,
                        initial_status,
                        status_tx,
                        thread_session_slug,
                        shutdown_rx,
                    );
                }
            }) {
            Ok(_thread) => {
                controller.attach_watch(watch_handle);
            }
            Err(source) => {
                error!(
                    error = %source,
                    session = %session_slug,
                    "failed to spawn session status watcher"
                );
            }
        }
    }

    cx.spawn(async move |cx| {
        while let Some(delivery) = status_rx.recv().await {
            match delivery {
                SessionStatusDelivery::Status(status) => {
                    publish_session_status(cx, fence, status, "observed session status change");
                }
                SessionStatusDelivery::Terminal(err) => {
                    error!(
                        error = %err,
                        session = %session_slug,
                        "failed to observe session status change"
                    );
                }
            }
        }
    })
    .detach();
}

/// Drives one session-status watcher to completion on the calling thread.
///
/// Owns one current-thread Tokio runtime, one supervisor connection, and one
/// transport subscription. The supervisor publishes full canonical snapshots;
/// the editor never samples lifecycle state on a cadence.
fn run_session_status_watch(
    controller: EditorSessionStatusController,
    initial_status: EditorSessionStatus,
    status_tx: SessionStatusSender,
    session_slug: String,
    mut shutdown: oneshot::Receiver<()>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(source) => {
            error!(
                error = %source,
                session = %session_slug,
                "session status watcher failed to build rpc runtime"
            );
            return;
        }
    };
    tokio::task::LocalSet::new().block_on(&runtime, async move {
        let client = tokio::select! {
            biased;
            _ = &mut shutdown => return,
            result = controller.supervisor_client() => match result {
                Ok(client) => client,
                Err(err) => {
                    let _ = status_tx.publish(SessionStatusDelivery::terminal(err));
                    return;
                }
            },
        };
        let (event_tx, mut event_rx) = CoalescedLatestSender::channel();
        let subscription = tokio::select! {
            biased;
            _ = &mut shutdown => return,
            result = client.subscribe_status_events(&session_slug, event_tx) => match result {
                Ok(subscription) => subscription,
                Err(err) => {
                    let _ = status_tx.publish(SessionStatusDelivery::terminal(err));
                    return;
                }
            },
        };
        let mut last_sequence = subscription.initial_sequence;
        let mut last_status = session_workspace_status_to_ui(subscription.initial_status);
        if last_status != initial_status
            && !status_tx.publish(SessionStatusDelivery::status(last_status.clone()))
        {
            return;
        }
        loop {
            let event = tokio::select! {
                biased;
                _ = &mut shutdown => return,
                event = event_rx.recv() => if let Some(event) = event { event } else {
                    let _ = status_tx.publish(SessionStatusDelivery::terminal(
                        EditorError::ServiceDiscovery(
                            "session-supervisor status subscription closed".to_string(),
                        ),
                    ));
                    return;
                },
            };
            if event.sequence <= last_sequence {
                let _ = status_tx.publish(SessionStatusDelivery::terminal(
                    EditorError::ServiceDiscovery(format!(
                        "session-supervisor status sequence regressed from {last_sequence} to {}",
                        event.sequence
                    )),
                ));
                return;
            }
            if let Err(err) = ensure_session_workspace_status_matches_request(
                &event.status,
                &session_slug,
                "subscribeEvents update",
            ) {
                let _ = status_tx.publish(SessionStatusDelivery::terminal(err));
                return;
            }
            last_sequence = event.sequence;
            let status = session_workspace_status_to_ui(event.status);
            if status != last_status {
                last_status = status.clone();
                if !status_tx.publish(SessionStatusDelivery::status(status)) {
                    return;
                }
            }
        }
    });
}

/// Spawn a background refresh of the attached session's status panel.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] when no editor controller
/// set is installed, or [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or [`EditorError::ControllerUnavailable`]
/// when the session-status slot is not ready. The spawned refresh itself
/// reports its own failures through the log, not through this call.
pub fn refresh_session_status(cx: &mut App) -> EditorResult<()> {
    let attached = crate::controller_set::session_status_controller(cx)?;
    spawn_session_status_refresh(cx, attached.fence, attached.controller);
    Ok(())
}

/// Spawn the terminal stop of the attached session's services, followed by
/// supervisor shutdown.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] when no editor controller
/// set is installed, or [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or [`EditorError::ControllerUnavailable`]
/// when the session-status slot is not ready. The spawned shutdown reports its
/// own failures through the log, not through this call.
pub fn stop_session_services(cx: &mut App) -> EditorResult<()> {
    let attached = crate::controller_set::session_status_controller(cx)?;
    spawn_session_services_stop(cx, attached.fence, attached.controller);
    Ok(())
}

/// Spawn a start of the attached session's services.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] when no editor controller
/// set is installed, or [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or [`EditorError::ControllerUnavailable`]
/// when the session-status slot is not ready. The spawned start reports its own
/// failures through the log, not through this call.
pub fn start_session_services(cx: &mut App) -> EditorResult<()> {
    let attached = crate::controller_set::session_status_controller(cx)?;
    spawn_session_services_start(cx, attached.fence, attached.controller);
    Ok(())
}

/// Spawn recovery of the attached session, then the verified reattach that
/// follows it.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] when no editor controller
/// set is installed, or when the controllers are there but no
/// `EditorAttachSession` global is — recovery needs the attach session to
/// reattach against. Returns [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or [`EditorError::ControllerUnavailable`]
/// when the session-status slot is not ready. The spawned recovery reports its
/// own failures through the log, not through this call.
pub fn recover_session(cx: &mut App, force: bool) -> EditorResult<()> {
    let attached = crate::controller_set::session_status_controller(cx)?;
    let attach_session = cx
        .try_global::<crate::EditorAttachSession>()
        .cloned()
        .ok_or(EditorError::MissingAttachedSession {
            operation: "recover an attached editor session",
        })?;
    spawn_session_recover(
        cx,
        attached.fence,
        attached.controller,
        attach_session,
        force,
    );
    Ok(())
}

/// Spawn a tail of one session service log into the output log and console.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] when no editor controller
/// set is installed, or [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or [`EditorError::ControllerUnavailable`]
/// when the session-status slot is not ready. The spawned tail reports its own
/// failures through the log, not through this call.
pub fn tail_session_service_log(
    cx: &mut App,
    request: az_editor_ui::actions::TailSessionServiceLog,
) -> EditorResult<()> {
    let attached = crate::controller_set::session_status_controller(cx)?;
    spawn_session_service_log_tail(cx, attached.fence, attached.controller, request);
    Ok(())
}

fn spawn_session_status_refresh(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorSessionStatusController,
) {
    let session_slug = controller.session_slug().to_string();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "session-status-refresh",
        move || async move { controller.refresh().await },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(status) => {
                    publish_session_status_in_app(cx, status, "refreshed session status");
                }
                Err(err) => {
                    error!(
                        error = %err,
                        session = %session_slug,
                        "failed to refresh session status"
                    );
                }
            }
        },
    );
}

fn spawn_session_services_stop(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorSessionStatusController,
) {
    let session_slug = controller.session_slug().to_string();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "session-services-stop",
        move || async move {
            let client = controller.supervisor_client().await?;
            let result = client
                .stop_services(
                    controller.session_slug(),
                    "editor requested session service shutdown",
                )
                .await?;
            client
                .shutdown_supervisor(
                    controller.session_slug(),
                    "editor completed terminal session service shutdown",
                )
                .await?;
            Ok(result)
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(result) => {
                    info!(
                        session = %session_slug,
                        stopped = result.stopped.len(),
                        skipped = result.skipped.len(),
                        "completed session service shutdown"
                    );
                    publish_session_status_in_app(
                        cx,
                        session_workspace_status_to_ui(result.status),
                        "observed terminal session status after stop",
                    );
                }
                Err(err) => {
                    error!(
                        error = %err,
                        session = %session_slug,
                        "failed to stop session services"
                    );
                }
            }
        },
    );
}

fn spawn_session_services_start(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorSessionStatusController,
) {
    let session_slug = controller.session_slug().to_string();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "session-services-start",
        move || async move {
            let result = controller
                .start_services("editor requested session service startup")
                .await?;
            Ok(result)
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(result) => {
                    info!(
                        session = %session_slug,
                        started = result.started.len(),
                        skipped = result.skipped.len(),
                        "completed session service startup"
                    );
                    publish_session_status_in_app(
                        cx,
                        session_workspace_status_to_ui(result.status),
                        "observed terminal session status after start",
                    );
                }
                Err(err) => {
                    error!(
                        error = %err,
                        session = %session_slug,
                        "failed to start session services"
                    );
                }
            }
        },
    );
}

fn spawn_session_recover(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorSessionStatusController,
    attach_session: crate::EditorAttachSession,
    force: bool,
) {
    let session_slug = controller.session_slug().to_string();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "session-recover",
        move || async move {
            let manifest = controller.recover(force).await?;
            Ok(manifest)
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(manifest) => {
                    info!(
                        session = %session_slug,
                        force,
                        state = ?manifest.state,
                        "recovered failed-preserved session"
                    );
                    spawn_verified_recovery_reattach(cx, fence, &attach_session);
                }
                Err(err) => {
                    error!(
                        error = %err,
                        session = %session_slug,
                        force,
                        "failed to recover session"
                    );
                }
            }
        },
    );
}

/// Recovery invalidates service descriptors and controller transports. Reattach
/// on a fresh worker thread (the attach helper owns its own Tokio runtime),
/// then route the verified result through the same aggregate installer as
/// ordinary project open. A failed reattach is deliberately reported at this
/// boundary rather than pretending a stale status refresh recovered the tools.
fn spawn_verified_recovery_reattach(
    cx: &App,
    fence: crate::controller_set::ControllerFence,
    prior: &crate::EditorAttachSession,
) {
    let project_root = prior.project_root.clone();
    let session_slug = prior.session_slug.clone();
    let daemon_endpoint = prior.daemon_endpoint.clone();
    let (result_tx, result_rx) = oneshot::channel();
    let spawn = std::thread::Builder::new()
        .name(format!("az-editor-session-reattach-{session_slug}"))
        .spawn(move || {
            let result = crate::attach_to_session_via_daemon(
                &project_root,
                Some(&session_slug),
                daemon_endpoint,
            );
            let _ = result_tx.send(result);
        });
    if let Err(error) = spawn {
        error!(
            %error,
            session = %prior.session_slug,
            project_root = %prior.project_root.display(),
            "recovery completed but could not start fresh verified editor-session reattach"
        );
        return;
    }
    cx.spawn(async move |cx| {
        let result = result_rx.await.unwrap_or_else(|_| {
            Err(EditorError::ServiceDiscovery(
                "recovery reattach worker stopped before returning a verified session".to_string(),
            ))
        });
        let () = cx.update(move |cx| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(session) => {
                    if let Err(error) = crate::project_workflow::install_verified_attached_session(cx, session) {
                        error!(
                            %error,
                            "recovery completed but fresh verified session installation failed"
                        );
                    } else {
                        info!("recovered session reattached through the editor controller aggregate");
                    }
                }
                Err(error) => {
                    error!(
                        %error,
                        "recovery completed but fresh verified session reattach failed; retaining scoped session failure evidence"
                    );
                }
            }
        });
    })
    .detach();
}

fn spawn_session_service_log_tail(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorSessionStatusController,
    request: az_editor_ui::actions::TailSessionServiceLog,
) {
    let session_slug = controller.session_slug().to_string();
    let service_name = request.service_name.clone();
    let stream = request.stream;
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "session-service-log",
        move || async move {
            controller
                .service_log_all(request.service_name, request.run, request.stream)
                .await
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(result) => {
                    let events = service_log_output_events(&result);
                    if stream == az_editor_ui::actions::SessionServiceLogStream::Structured {
                        publish_service_log_to_console(
                            cx.default_global::<ConsoleState>(),
                            &events,
                        );
                    } else {
                        publish_service_log_to_output_log(
                            cx.default_global::<OutputLogState>(),
                            &events,
                        );
                    }
                    cx.refresh_windows();
                }
                Err(err) => {
                    error!(
                        error = %err,
                        session = %session_slug,
                        service = %service_name,
                        "failed to read session service log"
                    );
                }
            }
        },
    );
}

const fn service_log_stream_to_proto(stream: SessionServiceLogStream) -> ServiceLogStream {
    match stream {
        SessionServiceLogStream::Stdout => ServiceLogStream::Stdout,
        SessionServiceLogStream::Stderr => ServiceLogStream::Stderr,
        SessionServiceLogStream::Structured => ServiceLogStream::Structured,
    }
}

fn publish_service_log_to_output_log(
    state: &mut OutputLogState,
    events: &[(LogLevel, String, String)],
) {
    for (level, source, line) in events {
        state.append_output(*level, source, line);
    }
}

fn publish_service_log_to_console(state: &mut ConsoleState, events: &[(LogLevel, String, String)]) {
    for (level, source, line) in events {
        state.log_from_source(*level, source, line);
    }
}

#[must_use]
pub fn service_log_output_events(result: &ServiceLogResult) -> Vec<(LogLevel, String, String)> {
    let stream_label = match result.stream {
        ServiceLogStream::Stdout => "stdout",
        ServiceLogStream::Stderr => "stderr",
        ServiceLogStream::Structured => "structured",
    };
    let default_level = match result.stream {
        ServiceLogStream::Stdout | ServiceLogStream::Structured => LogLevel::Info,
        ServiceLogStream::Stderr => LogLevel::Warn,
    };
    let source = format!("{}:{stream_label}", result.service_name);
    let mut lines = Vec::with_capacity(result.lines.len().saturating_add(2));
    lines.push((
        LogLevel::Info,
        source.clone(),
        format!("run {} ({})", result.run, result.path),
    ));
    if result.lines.is_empty() {
        lines.push((LogLevel::Info, source, "[no log lines]".to_string()));
    } else {
        lines.extend(result.lines.iter().map(|line| {
            (
                service_log_line_level(default_level, line),
                source.clone(),
                line.clone(),
            )
        }));
    }
    lines
}

fn service_log_line_level(default_level: LogLevel, line: &str) -> LogLevel {
    let normalized = line.trim_start().to_ascii_lowercase();
    let normalized = normalized
        .trim_start_matches(['[', '(', '{', '"', '\''])
        .trim_start();

    if normalized.starts_with("error")
        || normalized.starts_with("err ")
        || normalized.starts_with("level=error")
        || normalized.contains(" level=error")
        || normalized.contains("[error]")
    {
        LogLevel::Error
    } else if normalized.starts_with("warn")
        || normalized.starts_with("warning")
        || normalized.starts_with("level=warn")
        || normalized.contains(" level=warn")
        || normalized.contains("[warn]")
        || normalized.contains("[warning]")
    {
        LogLevel::Warn
    } else if normalized.starts_with("debug")
        || normalized.starts_with("level=debug")
        || normalized.contains(" level=debug")
        || normalized.contains("[debug]")
    {
        LogLevel::Debug
    } else if normalized.starts_with("trace")
        || normalized.starts_with("level=trace")
        || normalized.contains(" level=trace")
        || normalized.contains("[trace]")
    {
        LogLevel::Trace
    } else if normalized.starts_with("info")
        || normalized.starts_with("level=info")
        || normalized.contains(" level=info")
        || normalized.contains("[info]")
    {
        LogLevel::Info
    } else {
        default_level
    }
}

#[must_use]
pub fn service_log_output_lines(result: &ServiceLogResult) -> Vec<(LogLevel, String)> {
    service_log_output_events(result)
        .into_iter()
        .map(|(level, source, line)| (level, format!("[{source}] {line}")))
        .collect()
}

#[must_use]
pub fn service_log_console_lines(result: &ServiceLogResult) -> Vec<(LogLevel, String)> {
    if result.stream == ServiceLogStream::Structured {
        service_log_output_lines(result)
    } else {
        Vec::new()
    }
}

fn publish_session_status(
    cx: &gpui::AsyncApp,
    fence: crate::controller_set::ControllerFence,
    status: EditorSessionStatus,
    message: &'static str,
) {
    cx.update(move |cx| {
        if !crate::controller_set::is_current_fence(cx, fence) {
            return;
        }
        publish_session_status_in_app(cx, status, message);
    });
}

fn publish_session_status_in_app(
    cx: &mut App,
    status: EditorSessionStatus,
    message: &'static str,
) -> bool {
    let session_id = status.session_id.clone();
    let session_slug = status.session_slug.clone();
    let process_count = status.processes.len();
    if let Some(current) = cx.try_global::<EditorSessionStatus>()
        && current.session_id != session_id
    {
        return false;
    }
    cx.set_global(status);
    cx.refresh_windows();
    info!(
        session = %session_slug,
        process_count,
        "{message}"
    );
    true
}

#[must_use]
pub fn session_workspace_status_to_ui(status: SessionWorkspaceStatus) -> EditorSessionStatus {
    let manifest = status.manifest;
    let processes = manifest
        .processes
        .iter()
        .map(session_process_to_ui)
        .collect::<Vec<_>>();
    EditorSessionStatus {
        session_id: manifest.id.to_string(),
        project_id: manifest.project_id,
        session_slug: manifest.slug,
        project_root: manifest.project_root,
        workspace_root: manifest.workspace_root,
        run_dir: manifest.run_dir,
        state: session_state_to_ui(manifest.state),
        failure_reason: status.failure_reason,
        services_count: manifest.services.len(),
        processes,
    }
}

fn session_process_to_ui(process: &az_proto_session::ServiceProcessRecord) -> SessionProcessData {
    SessionProcessData {
        owner_id: process.owner_id.clone(),
        owner_root: process.owner_root.clone(),
        service_name: process.service_name.clone(),
        role: service_role_to_ui(process.role),
        run: process.run,
        state: process_state_to_ui(process.state),
        pid: process.pid,
        exit_code: process.exit_code,
        failure: process.failure.clone(),
        structured_log: process.structured_log.clone(),
    }
}

const fn session_state_to_ui(state: ProtoSessionState) -> EditorSessionStateData {
    match state {
        ProtoSessionState::Preparing => EditorSessionStateData::Preparing,
        ProtoSessionState::Active => EditorSessionStateData::Active,
        ProtoSessionState::FailedPreserved => EditorSessionStateData::FailedPreserved,
        ProtoSessionState::Removed => EditorSessionStateData::Removed,
    }
}

const fn process_state_to_ui(state: ProtoServiceProcessState) -> SessionProcessStateData {
    match state {
        ProtoServiceProcessState::Planned => SessionProcessStateData::Planned,
        ProtoServiceProcessState::Starting => SessionProcessStateData::Starting,
        ProtoServiceProcessState::Running => SessionProcessStateData::Running,
        ProtoServiceProcessState::Exited => SessionProcessStateData::Exited,
        ProtoServiceProcessState::Failed => SessionProcessStateData::Failed,
    }
}

const fn service_role_to_ui(role: ServiceRole) -> SessionServiceRoleData {
    match role {
        ServiceRole::Unknown => SessionServiceRoleData::Unknown,
        ServiceRole::Editor => SessionServiceRoleData::Editor,
        ServiceRole::Daemon => SessionServiceRoleData::Daemon,
        ServiceRole::SessionSupervisor => SessionServiceRoleData::SessionSupervisor,
        ServiceRole::ProjectHost => SessionServiceRoleData::ProjectHost,
        ServiceRole::AssetProcessor => SessionServiceRoleData::AssetProcessor,
        ServiceRole::RuntimeHost => SessionServiceRoleData::RuntimeHost,
        ServiceRole::Worker => SessionServiceRoleData::Worker,
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

fn protocol_path_starts_with(path: impl AsRef<Path>, base: impl AsRef<Path>) -> bool {
    let path = comparable_protocol_path(path.as_ref());
    let base = comparable_protocol_path(base.as_ref());
    #[cfg(windows)]
    {
        let path = comparable_protocol_path_text(&path);
        let base = comparable_protocol_path_text(&base);
        path == base
            || path
                .strip_prefix(&base)
                .is_some_and(is_path_separator_prefix)
    }
    #[cfg(not(windows))]
    {
        path.starts_with(base)
    }
}

#[cfg(windows)]
fn is_path_separator_prefix(value: &str) -> bool {
    value.starts_with('\\') || value.starts_with('/')
}

fn path_has_parent_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
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

fn ensure_resolved_service_descriptor_matches_request(
    descriptor: &ServiceDescriptor,
    expected_id: &ServiceId,
    expected_role: ServiceRole,
    expected_session_id: Option<Uuid>,
) -> EditorResult<()> {
    if &descriptor.id != expected_id || descriptor.role != expected_role {
        return Err(session_supervisor_authority_mismatch(
            "resolveService",
            format!(
                "session-supervisor returned descriptor `{}`/`{}` role {:?}, expected `{}`/`{}` role {:?}",
                descriptor.id.namespace,
                descriptor.id.name,
                descriptor.role,
                expected_id.namespace,
                expected_id.name,
                expected_role
            ),
        ));
    }
    descriptor
        .validate_brokered_capability_templates()
        .map_err(|error| {
            session_supervisor_authority_mismatch(
                "resolveService",
                format!(
                    "descriptor `{}`/`{}` capability templates are invalid: {error}",
                    descriptor.id.namespace, descriptor.id.name
                ),
            )
        })?;
    if let Some(expected_session_id) = expected_session_id {
        let project_scoped_service = matches!(
            expected_role,
            ServiceRole::ProjectHost | ServiceRole::AssetProcessor | ServiceRole::Worker
        );
        for (index, capability) in descriptor.capabilities.iter().enumerate() {
            // Project-scoped services bind every one of their contracts,
            // including observability and lifecycle control, to the project
            // instance. Their brokered templates therefore remain unscoped
            // when a session attaches to the descriptor.
            if (project_scoped_service && capability.session.is_none())
                || capability.session == Some(expected_session_id)
            {
                continue;
            }
            let actual = capability
                .session
                .map_or_else(|| "<unscoped>".to_string(), |session| session.to_string());
            return Err(session_supervisor_authority_mismatch(
                "resolveService",
                format!(
                    "descriptor `{}`/`{}` capability template {index} targets session `{actual}`, expected `{expected_session_id}`",
                    descriptor.id.namespace, descriptor.id.name
                ),
            ));
        }
    }
    Ok(())
}

fn ensure_session_workspace_status_matches_request(
    status: &SessionWorkspaceStatus,
    expected_slug: &str,
    operation: &'static str,
) -> EditorResult<()> {
    ensure_session_manifest_matches_request(&status.manifest, expected_slug, operation)?;
    if status.manifest.state == ProtoSessionState::FailedPreserved
        && status
            .failure_reason
            .as_deref()
            .is_none_or(|reason| reason.trim().is_empty())
    {
        return Err(session_supervisor_authority_mismatch(
            operation,
            format!(
                "failed-preserved session `{}` must include a failure reason",
                status.manifest.slug
            ),
        ));
    }
    Ok(())
}

fn ensure_runtime_asset_package_roots_are_well_formed(
    result: &RuntimeAssetPackageRootsResult,
    operation: &'static str,
) -> EditorResult<()> {
    az_proto_runtime::validate_runtime_asset_package_roots(&result.roots).map_err(|error| {
        session_supervisor_authority_mismatch(operation, format!("runtime {error}"))
    })
}

fn ensure_session_manifest_list_is_well_formed(sessions: &[SessionManifest]) -> EditorResult<()> {
    let mut seen_ids = HashSet::new();
    let mut seen_slugs = HashSet::new();
    for manifest in sessions {
        ensure_session_manifest_identity(manifest, "listSessions")?;
        if !seen_ids.insert(manifest.id) {
            return Err(session_supervisor_authority_mismatch(
                "listSessions",
                format!(
                    "session list returned duplicate session id `{}`",
                    manifest.id
                ),
            ));
        }
        if !seen_slugs.insert(manifest.slug.as_str()) {
            return Err(session_supervisor_authority_mismatch(
                "listSessions",
                format!("session list returned duplicate slug `{}`", manifest.slug),
            ));
        }
    }
    Ok(())
}

fn ensure_session_manifest_matches_request(
    manifest: &SessionManifest,
    expected_slug: &str,
    operation: &'static str,
) -> EditorResult<()> {
    ensure_session_manifest_identity(manifest, operation)?;
    if manifest.slug != expected_slug {
        return Err(session_supervisor_authority_mismatch(
            operation,
            format!(
                "session-supervisor returned session `{}`, expected `{}`",
                manifest.slug, expected_slug
            ),
        ));
    }
    Ok(())
}

fn ensure_session_manifest_identity(
    manifest: &SessionManifest,
    operation: &'static str,
) -> EditorResult<()> {
    if manifest.id == Uuid::nil() {
        return Err(session_supervisor_authority_mismatch(
            operation,
            format!("session `{}` id cannot be nil", manifest.slug),
        ));
    }
    if manifest.project_id.trim().is_empty()
        || manifest.slug.trim().is_empty()
        || manifest.project_root.trim().is_empty()
        || manifest.workspace_root.trim().is_empty()
        || manifest.run_dir.trim().is_empty()
    {
        return Err(session_supervisor_authority_mismatch(
            operation,
            format!(
                "session `{}` identity/path fields cannot be empty",
                manifest.slug
            ),
        ));
    }

    let mut seen_descriptors = HashSet::new();
    for descriptor in &manifest.services {
        if descriptor.id.namespace.trim().is_empty() || descriptor.id.name.trim().is_empty() {
            return Err(session_supervisor_authority_mismatch(
                operation,
                format!(
                    "session `{}` contains an invalid service descriptor",
                    manifest.slug
                ),
            ));
        }
        if !seen_descriptors.insert((
            descriptor.id.namespace.as_str(),
            descriptor.id.name.as_str(),
            descriptor.role,
        )) {
            return Err(session_supervisor_authority_mismatch(
                operation,
                format!(
                    "session `{}` contains duplicate descriptor `{}`/`{}` role {:?}",
                    manifest.slug, descriptor.id.namespace, descriptor.id.name, descriptor.role
                ),
            ));
        }
    }

    Ok(())
}

pub(crate) fn ensure_save_graph_document_result_matches_request(
    result: &SaveGraphDocumentResult,
    expected_document_id: &DocumentId,
) -> EditorResult<()> {
    let saved = &result.saved;
    if saved.document_id.as_str() != expected_document_id.as_str() {
        return Err(session_supervisor_authority_mismatch(
            "saveGraphDocument",
            format!(
                "session-supervisor saved document `{}`, expected `{}`",
                saved.document_id.as_str(),
                expected_document_id.as_str()
            ),
        ));
    }

    if saved.source_path != expected_document_id.as_str() {
        return Err(session_supervisor_authority_mismatch(
            "saveGraphDocument",
            format!(
                "saved source path `{}` does not match requested document `{}`",
                saved.source_path,
                expected_document_id.as_str()
            ),
        ));
    }

    if saved.schema_type.trim().is_empty()
        || saved.content_hash.len() != 32
        || saved.byte_length == 0
    {
        return Err(session_supervisor_authority_mismatch(
            "saveGraphDocument",
            format!(
                "saved document `{}` returned invalid source metadata",
                saved.document_id.as_str()
            ),
        ));
    }

    let asset_record = &result.asset_record;
    let entry = &asset_record.entry;
    if asset_record.asset_guid == Uuid::nil() {
        return Err(session_supervisor_authority_mismatch(
            "saveGraphDocument",
            format!(
                "asset record for saved document `{}` returned a nil asset guid",
                saved.document_id.as_str()
            ),
        ));
    }

    if entry.entry_id <= 0 || entry.workspace_id <= 0 || entry.root_id <= 0 {
        return Err(session_supervisor_authority_mismatch(
            "saveGraphDocument",
            format!(
                "asset record for saved document `{}` did not carry valid entry identifiers",
                saved.document_id.as_str()
            ),
        ));
    }

    if entry.source_path != saved.source_path
        || entry.schema_type.as_deref() != Some(saved.schema_type.as_str())
    {
        return Err(session_supervisor_authority_mismatch(
            "saveGraphDocument",
            format!(
                "asset record did not echo saved document `{}` source identity",
                saved.document_id.as_str()
            ),
        ));
    }

    let expected_content_hash = hex_lower_bytes(&saved.content_hash);
    if entry.content_hash != expected_content_hash {
        return Err(session_supervisor_authority_mismatch(
            "saveGraphDocument",
            format!(
                "asset record hash `{}` did not match saved document `{}` hash",
                entry.content_hash,
                saved.document_id.as_str()
            ),
        ));
    }

    if matches!(
        entry.diff,
        WorkspaceEntryDiff::Deleted | WorkspaceEntryDiff::Conflicted
    ) || entry.diagnostics_count < 0
    {
        return Err(session_supervisor_authority_mismatch(
            "saveGraphDocument",
            format!(
                "asset record for saved document `{}` returned invalid entry diff metadata",
                saved.document_id.as_str()
            ),
        ));
    }

    Ok(())
}

fn hex_lower_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

fn ensure_exec_command_result_matches_request(
    result: &ExecCommandResult,
    max_output_bytes: u32,
) -> EditorResult<()> {
    if result.success && !result.exited {
        return Err(session_supervisor_authority_mismatch(
            "execCommand",
            "successful command result must report an exited process".to_string(),
        ));
    }
    if result.success && result.exit_code != 0 {
        return Err(session_supervisor_authority_mismatch(
            "execCommand",
            format!(
                "successful command result must report exit code 0, got {}",
                result.exit_code
            ),
        ));
    }
    if !result.exited && result.exit_code != 0 {
        return Err(session_supervisor_authority_mismatch(
            "execCommand",
            format!(
                "non-exited command result cannot carry exit code {}",
                result.exit_code
            ),
        ));
    }

    let max_lossy_utf8_len = effective_exec_output_limit(max_output_bytes).saturating_mul(3);
    if result.stdout.len() > max_lossy_utf8_len {
        return Err(session_supervisor_authority_mismatch(
            "execCommand",
            format!(
                "stdout length {} exceeds bounded output limit {}",
                result.stdout.len(),
                max_lossy_utf8_len
            ),
        ));
    }
    if result.stderr.len() > max_lossy_utf8_len {
        return Err(session_supervisor_authority_mismatch(
            "execCommand",
            format!(
                "stderr length {} exceeds bounded output limit {}",
                result.stderr.len(),
                max_lossy_utf8_len
            ),
        ));
    }

    Ok(())
}

const fn effective_exec_output_limit(max_output_bytes: u32) -> usize {
    if max_output_bytes == 0 {
        DEFAULT_EXEC_OUTPUT_LIMIT_BYTES as usize
    } else {
        max_output_bytes as usize
    }
}

fn ensure_service_log_result_matches_request(
    result: &ServiceLogResult,
    expected_slug: &str,
    expected_service_name: &str,
    expected_run: Option<Uuid>,
    expected_stream: ServiceLogStream,
) -> EditorResult<()> {
    if result.session_slug != expected_slug {
        return Err(session_supervisor_authority_mismatch(
            "serviceLog",
            format!(
                "service log returned session `{}`, expected `{}`",
                result.session_slug, expected_slug
            ),
        ));
    }
    if result.service_name != expected_service_name {
        return Err(session_supervisor_authority_mismatch(
            "serviceLog",
            format!(
                "service log returned service `{}`, expected `{}`",
                result.service_name, expected_service_name
            ),
        ));
    }
    if result.stream != expected_stream {
        return Err(session_supervisor_authority_mismatch(
            "serviceLog",
            format!(
                "service log returned stream {:?}, expected {:?}",
                result.stream, expected_stream
            ),
        ));
    }
    if let Some(expected_run) = expected_run
        && result.run != expected_run
    {
        return Err(session_supervisor_authority_mismatch(
            "serviceLog",
            format!(
                "service log returned run {}, expected {}",
                result.run, expected_run
            ),
        ));
    }
    if result.run.is_nil() || result.path.trim().is_empty() {
        return Err(session_supervisor_authority_mismatch(
            "serviceLog",
            "service log result must carry a non-nil run and non-empty path".to_string(),
        ));
    }
    Ok(())
}

const fn session_supervisor_authority_mismatch(
    operation: &'static str,
    reason: String,
) -> EditorError {
    EditorError::ServiceAuthorityMismatch {
        service: "session-supervisor",
        operation,
        reason,
    }
}

pub(crate) fn validate_session_supervisor_descriptor(
    descriptor: &ServiceDescriptor,
) -> EditorResult<()> {
    let expected = session_supervisor_service_id();
    if descriptor.id != expected || descriptor.role != ServiceRole::SessionSupervisor {
        return Err(crate::error::EditorError::ServiceDiscovery(format!(
            "expected session-supervisor descriptor `{}`/`{}` with role {:?}, got `{}`/`{}` with role {:?}",
            expected.namespace,
            expected.name,
            ServiceRole::SessionSupervisor,
            descriptor.id.namespace,
            descriptor.id.name,
            descriptor.role
        )));
    }
    validate_descriptor_capability_templates(descriptor, "session-supervisor")?;
    Ok(())
}

fn validate_session_supervisor_descriptor_for_session(
    descriptor: &ServiceDescriptor,
    session_id: Uuid,
) -> EditorResult<()> {
    validate_session_supervisor_descriptor(descriptor)?;
    validate_session_descriptor_capability_templates(descriptor, "session-supervisor", session_id)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use az_filesystem::AzothDataHome;
    use az_proto_core::{Endpoint, EndpointKind};
    use az_service_catalog::{asset_processor_service_descriptor, project_host_service_descriptor};
    use az_session::{
        CreateSessionRequest, SessionManager, SessionSupervisorCommand,
        SessionSupervisorCommandResult, SessionSupervisorRpc, StartServicesReport,
        StopServicesReport, session_supervisor_command_channel,
    };
    use uuid::Uuid;

    use super::*;

    fn temp_project(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "az-editor-session-supervisor-test-{name}-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        write_test_project_manifest(
            &path,
            &az_project::ProjectManifest::new(
                format!("local.az_editor_session_supervisor_{name}"),
                "Editor Session Supervisor Test",
                "0.1.0",
            ),
        );
        path
    }

    fn test_session_manager(project_root: &Path) -> SessionManager {
        SessionManager::with_data_home(
            project_root,
            AzothDataHome::new(project_root.join(".azoth-test-home")),
        )
        .unwrap()
    }

    fn write_test_project_manifest(root: &Path, manifest: &az_project::ProjectManifest) {
        az_project::write_project_manifest(root, manifest).unwrap();
        az_project::refresh_project_lock(root).unwrap();
    }

    fn test_tcp_endpoint(label: impl AsRef<str>) -> Endpoint {
        let hash = label.as_ref().bytes().fold(0_u16, |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(u16::from(byte))
        });
        Endpoint::new(
            EndpointKind::Tcp,
            format!("127.0.0.1:{}", 30_000 + hash % 20_000),
        )
    }

    fn session_descriptor_capability(
        session: Uuid,
        audience: &'static str,
        permission: &'static str,
    ) -> Capability {
        Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_session(session)
        .with_audience(audience)
        .with_permissions([permission])
        .with_token_hash(vec![0x42])
    }

    fn valid_session_manifest(slug: &str) -> SessionManifest {
        valid_session_manifest_with_id(slug, Uuid::from_bytes([0x51; 16]))
    }

    fn valid_session_manifest_with_id(slug: &str, id: Uuid) -> SessionManifest {
        let mut manifest = az_proto_session::SessionManifest::new(
            id,
            "local.editor_session",
            slug,
            "projects/example",
            "projects/example",
            format!("projects/example/.azoth/runs/{slug}"),
            ProtoSessionState::Active,
        );
        manifest.services.push(
            ServiceDescriptor::new(
                ServiceId::new(PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
                ServiceRole::ProjectHost,
                Endpoint::in_process(format!("project-host:{slug}")),
            )
            .with_run(Uuid::now_v7()),
        );
        manifest
    }

    fn valid_session_workspace_status(slug: &str) -> SessionWorkspaceStatus {
        SessionWorkspaceStatus {
            manifest: valid_session_manifest(slug),
            failure_reason: None,
        }
    }

    fn valid_save_graph_document_result(document_id: &str) -> SaveGraphDocumentResult {
        let schema_type = "az.test.Prefab".to_string();
        SaveGraphDocumentResult {
            saved: az_proto_project::SavedDocument {
                document_id: DocumentId::new(document_id),
                revision: DocumentRevision::new(1),
                source_path: document_id.to_string(),
                schema_type: schema_type.clone(),
                content_hash: vec![0xab; 32],
                byte_length: 16,
            },
            asset_record: az_proto_asset::SourceAssetRecordResult {
                asset_guid: Uuid::from_bytes([0x52; 16]),
                entry: az_proto_asset::WorkspaceEntry {
                    entry_id: 1,
                    workspace_id: 2,
                    asset_guid: Uuid::from_bytes([0x52; 16]),
                    root_id: 4,
                    source_path: document_id.to_string(),
                    schema_type: Some(schema_type),
                    content_hash: "ab".repeat(32),
                    diff: az_proto_asset::WorkspaceEntryDiff::Clean,
                    diagnostics_count: 0,
                    updated_unix_ms: 5,
                    jobs: Vec::new(),
                },
            },
        }
    }

    fn assert_session_supervisor_authority_mismatch(
        error: EditorError,
        expected_operation: &'static str,
        expected_reason: &str,
    ) {
        match error {
            EditorError::ServiceAuthorityMismatch {
                service,
                operation,
                reason,
            } => {
                assert_eq!(service, "session-supervisor");
                assert_eq!(operation, expected_operation);
                assert!(
                    reason.contains(expected_reason),
                    "expected reason containing `{expected_reason}`, got `{reason}`"
                );
            }
            other => panic!("expected session-supervisor authority mismatch, got {other:?}"),
        }
    }

    #[test]
    fn maps_session_workspace_status_to_session_panel_snapshot() {
        let session_id = Uuid::from_bytes([0x62; 16]);
        let mut manifest = az_proto_session::SessionManifest::new(
            session_id,
            "local.editor_session",
            "editor",
            "projects/example",
            "projects/example",
            "projects/example/.azoth/runs/editor",
            ProtoSessionState::Active,
        );
        let runtime_run = Uuid::now_v7();
        manifest.services.push(
            ServiceDescriptor::new(
                runtime_host_service_id(),
                ServiceRole::RuntimeHost,
                Endpoint::in_process("runtime-host:editor"),
            )
            .with_run(runtime_run),
        );
        manifest
            .processes
            .push(az_proto_session::ServiceProcessRecord {
                owner_id: "local.editor_session".to_string(),
                service_name: "runtime-host".to_string(),
                role: ServiceRole::RuntimeHost,
                run: runtime_run,
                previous_run: None,
                endpoint: Endpoint::in_process("runtime-host:editor"),
                program: "az-runtime-host".to_string(),
                // A running process has always been through `capture_program_artifact`.
                program_artifact: Some(az_proto_session::ServiceProgramArtifact {
                    byte_length: 4096,
                    modified_unix_ns: 1_700_000_000_000_000_000,
                    file_system_id: Some(7),
                    file_id: Some(11),
                }),
                cwd: "projects/example".to_string(),
                owner_root: "projects/example".to_string(),
                args: Vec::new(),
                stdout_log: "project-host.stdout.log".to_string(),
                stderr_log: "project-host.stderr.log".to_string(),
                structured_log: "runtime-host.capnp.log".to_string(),
                state: ProtoServiceProcessState::Running,
                pid: Some(4242),
                process_start_time: Some(1_700_000_000),
                exit_code: None,
                failure: None,
                planned_unix_ms: 1,
                updated_unix_ms: 2,
                started_unix_ms: Some(2),
                exited_unix_ms: None,
            });

        let status = session_workspace_status_to_ui(SessionWorkspaceStatus {
            manifest,
            failure_reason: Some("runtime-host exited".to_string()),
        });

        assert_eq!(status.session_id, session_id.to_string());
        assert_eq!(status.project_id, "local.editor_session");
        assert_eq!(status.session_slug, "editor");
        assert_eq!(status.state, EditorSessionStateData::Active);
        assert_eq!(status.services_count, 1);
        assert_eq!(
            status.failure_reason.as_deref(),
            Some("runtime-host exited")
        );
        assert_eq!(status.processes.len(), 1);
        assert_eq!(status.processes[0].owner_id, "local.editor_session");
        assert_eq!(status.processes[0].owner_root, "projects/example");
        assert_eq!(
            status.processes[0].role,
            SessionServiceRoleData::RuntimeHost
        );
        assert_eq!(status.processes[0].state, SessionProcessStateData::Running);
        assert_eq!(status.processes[0].pid, Some(4242));
        assert_eq!(status.processes[0].structured_log, "runtime-host.capnp.log");
    }

    #[test]
    fn formats_stdout_or_stderr_service_log_tail_events() {
        let result = ServiceLogResult {
            session_slug: "editor".to_string(),
            service_name: "project-host".to_string(),
            run: Uuid::now_v7(),
            stream: ServiceLogStream::Stderr,
            path: "logs/project-host.stderr.log".to_string(),
            lines: vec!["schema load failed".to_string()],
            next_offset: 128,
        };

        let events = service_log_output_events(&result);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, LogLevel::Info);
        assert_eq!(events[0].1, "project-host:stderr");
        assert!(events[0].2.contains("run "));
        assert_eq!(events[1].0, LogLevel::Warn);
        assert_eq!(events[1].1, "project-host:stderr");
        assert_eq!(events[1].2, "schema load failed");
    }

    #[test]
    fn formats_structured_service_log_tail_events_for_console() {
        let result = ServiceLogResult {
            session_slug: "editor".to_string(),
            service_name: "project-host".to_string(),
            run: Uuid::now_v7(),
            stream: ServiceLogStream::Structured,
            path: "logs/project-host.capnp.log".to_string(),
            lines: vec![
                "[az_project_host::save] saved authored document document=prefabs/door.prefab.ron"
                    .to_string(),
            ],
            next_offset: 256,
        };

        let events = service_log_output_events(&result);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, LogLevel::Info);
        assert_eq!(events[0].1, "project-host:structured");
        assert!(events[0].2.contains("run "));
        assert_eq!(events[1].0, LogLevel::Info);
        assert_eq!(events[1].1, "project-host:structured");
        assert_eq!(
            events[1].2,
            "[az_project_host::save] saved authored document document=prefabs/door.prefab.ron"
        );
    }

    #[test]
    fn service_log_output_events_parse_line_severity() {
        let result = ServiceLogResult {
            session_slug: "editor".to_string(),
            service_name: "asset-processor".to_string(),
            run: Uuid::now_v7(),
            stream: ServiceLogStream::Stdout,
            path: "logs/asset-processor.stdout.log".to_string(),
            lines: vec![
                "INFO indexed 3 assets".to_string(),
                "[warn] stale dependency".to_string(),
                "level=error failed to compile material".to_string(),
                "DEBUG cache hit".to_string(),
            ],
            next_offset: 512,
        };

        let events = service_log_output_events(&result);
        assert_eq!(events[1].0, LogLevel::Info);
        assert_eq!(events[2].0, LogLevel::Warn);
        assert_eq!(events[3].0, LogLevel::Error);
        assert_eq!(events[4].0, LogLevel::Debug);
        assert_eq!(events[3].1, "asset-processor:stdout");
    }

    #[test]
    fn stdout_service_log_tail_is_output_log_only() {
        let result = ServiceLogResult {
            session_slug: "editor".to_string(),
            service_name: "asset-processor".to_string(),
            run: Uuid::now_v7(),
            stream: ServiceLogStream::Stdout,
            path: "logs/asset-processor.stdout.log".to_string(),
            lines: vec!["INFO indexed 3 assets".to_string()],
            next_offset: 512,
        };

        assert!(service_log_console_lines(&result).is_empty());
        assert!(!service_log_output_lines(&result).is_empty());
    }

    #[test]
    fn publishes_service_log_events_to_console_state_with_source() {
        let events = vec![(
            LogLevel::Error,
            "project-host:stderr".to_owned(),
            "level=error failed to save".to_owned(),
        )];
        let mut console = ConsoleState::default();

        publish_service_log_to_console(&mut console, &events);

        let message = console.messages().first().unwrap();
        assert_eq!(message.level, LogLevel::Error);
        assert_eq!(message.source, "project-host:stderr");
        assert_eq!(message.message, "level=error failed to save");
    }

    #[test]
    fn maps_ui_service_log_stream_to_proto_stream() {
        assert_eq!(
            service_log_stream_to_proto(SessionServiceLogStream::Stdout),
            ServiceLogStream::Stdout
        );
        assert_eq!(
            service_log_stream_to_proto(SessionServiceLogStream::Stderr),
            ServiceLogStream::Stderr
        );
        assert_eq!(
            service_log_stream_to_proto(SessionServiceLogStream::Structured),
            ServiceLogStream::Structured
        );
    }

    #[test]
    fn session_supervisor_resolved_descriptor_must_echo_requested_service() {
        let session = Uuid::from_bytes([0x45; 16]);
        let descriptor = ServiceDescriptor::new(
            ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
            ServiceRole::AssetProcessor,
            Endpoint::in_process("asset-processor:editor"),
        )
        .with_capability(session_descriptor_capability(
            session,
            "asset-processor",
            "asset.read",
        ));

        let err = ensure_resolved_service_descriptor_matches_request(
            &descriptor,
            &ServiceId::new(PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
            Some(session),
        )
        .unwrap_err();

        assert_session_supervisor_authority_mismatch(
            err,
            "resolveService",
            "expected `azoth`/`project-host` role ProjectHost",
        );
    }

    #[test]
    fn session_supervisor_resolved_descriptor_capabilities_must_match_session() {
        let session = Uuid::from_bytes([0x46; 16]);
        let other_session = Uuid::from_bytes([0x47; 16]);

        // Session-scoped services must echo the requesting session on every
        // capability template.
        let runtime_host = ServiceDescriptor::new(
            ServiceId::new("azoth", "runtime-host"),
            ServiceRole::RuntimeHost,
            Endpoint::in_process("runtime-host:editor"),
        )
        .with_capability(session_descriptor_capability(
            other_session,
            "runtime-host",
            "runtime.control",
        ));

        let err = ensure_resolved_service_descriptor_matches_request(
            &runtime_host,
            &ServiceId::new("azoth", "runtime-host"),
            ServiceRole::RuntimeHost,
            Some(session),
        )
        .unwrap_err();

        assert_session_supervisor_authority_mismatch(
            err,
            "resolveService",
            &format!("expected `{session}`"),
        );

        // Project-scoped services (project-host, asset-processor) ship
        // unscoped templates by contract; session equality does not apply.
        let project_host = ServiceDescriptor::new(
            ServiceId::new(PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:editor"),
        )
        .with_capability(
            Capability::new(
                ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                ServiceRole::Editor,
            )
            .with_audience(PROJECT_HOST_AUDIENCE)
            .with_permissions(["project.read"])
            .with_token_hash(vec![0x42]),
        );

        ensure_resolved_service_descriptor_matches_request(
            &project_host,
            &ServiceId::new(PROJECT_HOST_SERVICE_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
            Some(session),
        )
        .unwrap();
    }

    #[test]
    fn session_supervisor_status_must_echo_requested_slug() {
        let status = valid_session_workspace_status("other");

        let err = ensure_session_workspace_status_matches_request(&status, "editor", "status")
            .unwrap_err();

        assert_session_supervisor_authority_mismatch(err, "status", "expected `editor`");
    }

    #[test]
    fn session_supervisor_status_requires_failed_reason_text() {
        let mut status = valid_session_workspace_status("editor");
        status.manifest.state = ProtoSessionState::FailedPreserved;
        status.failure_reason = Some("   ".to_string());

        let err = ensure_session_workspace_status_matches_request(&status, "editor", "status")
            .unwrap_err();

        assert_session_supervisor_authority_mismatch(err, "status", "failure reason");
    }

    #[test]
    fn session_supervisor_list_rejects_duplicate_session_slugs() {
        let first = valid_session_manifest_with_id("editor", Uuid::from_bytes([0x48; 16]));
        let second = valid_session_manifest_with_id("editor", Uuid::from_bytes([0x49; 16]));

        let err = ensure_session_manifest_list_is_well_formed(&[first, second]).unwrap_err();

        assert_session_supervisor_authority_mismatch(err, "listSessions", "duplicate slug");
    }

    #[test]
    fn session_supervisor_service_log_must_echo_request() {
        let result = ServiceLogResult {
            session_slug: "editor".to_string(),
            service_name: "project-host".to_string(),
            run: Uuid::from_bytes([3; 16]),
            stream: ServiceLogStream::Stdout,
            path: "logs/project-host.stdout.log".to_string(),
            lines: Vec::new(),
            next_offset: 0,
        };

        let err = ensure_service_log_result_matches_request(
            &result,
            "editor",
            "project-host",
            Some(Uuid::from_bytes([4; 16])),
            ServiceLogStream::Stdout,
        )
        .unwrap_err();

        assert_session_supervisor_authority_mismatch(
            err,
            "serviceLog",
            &Uuid::from_bytes([4; 16]).to_string(),
        );
    }

    #[test]
    fn session_supervisor_save_graph_document_must_echo_requested_document() {
        let result = valid_save_graph_document_result("prefabs/door.prefab.ron");

        let err = ensure_save_graph_document_result_matches_request(
            &result,
            &DocumentId::new("prefabs/window.prefab.ron"),
        )
        .unwrap_err();

        assert_session_supervisor_authority_mismatch(
            err,
            "saveGraphDocument",
            "prefabs/window.prefab.ron",
        );
    }

    #[test]
    fn session_supervisor_save_graph_document_must_echo_source_path() {
        let requested_document = DocumentId::new("prefabs/door.prefab.ron");
        let mut result = valid_save_graph_document_result(requested_document.as_str());
        result.saved.source_path = "prefabs/window.prefab.ron".to_string();
        result.asset_record.entry.source_path = result.saved.source_path.clone();

        let err = ensure_save_graph_document_result_matches_request(&result, &requested_document)
            .unwrap_err();

        assert_session_supervisor_authority_mismatch(err, "saveGraphDocument", "saved source path");
    }

    #[test]
    fn session_supervisor_save_graph_document_must_carry_valid_entry_identifiers() {
        let requested_document = DocumentId::new("prefabs/door.prefab.ron");
        let mut result = valid_save_graph_document_result(requested_document.as_str());
        result.asset_record.entry.entry_id = 0;

        let err = ensure_save_graph_document_result_matches_request(&result, &requested_document)
            .unwrap_err();

        assert_session_supervisor_authority_mismatch(err, "saveGraphDocument", "entry identifiers");
    }

    #[test]
    fn session_supervisor_save_graph_document_must_echo_asset_hash() {
        let requested_document = DocumentId::new("prefabs/door.prefab.ron");
        let mut result = valid_save_graph_document_result(requested_document.as_str());
        result.asset_record.entry.content_hash = "cd".repeat(32);

        let err = ensure_save_graph_document_result_matches_request(&result, &requested_document)
            .unwrap_err();

        assert_session_supervisor_authority_mismatch(err, "saveGraphDocument", "did not match");
    }

    #[test]
    fn session_supervisor_save_graph_document_rejects_conflicted_workspace_entry_diff() {
        let requested_document = DocumentId::new("prefabs/door.prefab.ron");
        let mut result = valid_save_graph_document_result(requested_document.as_str());
        result.asset_record.entry.diff = az_proto_asset::WorkspaceEntryDiff::Conflicted;

        let err = ensure_save_graph_document_result_matches_request(&result, &requested_document)
            .unwrap_err();

        assert_session_supervisor_authority_mismatch(
            err,
            "saveGraphDocument",
            "invalid entry diff metadata",
        );
    }

    #[test]
    fn session_supervisor_exec_result_must_be_consistent() {
        let result = ExecCommandResult {
            success: true,
            exited: false,
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let err = ensure_exec_command_result_matches_request(&result, 0).unwrap_err();

        assert_session_supervisor_authority_mismatch(err, "execCommand", "exited process");

        let mut result = result;
        result.success = false;
        result.exit_code = 7;
        let err = ensure_exec_command_result_matches_request(&result, 1024).unwrap_err();
        assert_session_supervisor_authority_mismatch(err, "execCommand", "non-exited");
    }

    #[test]
    fn session_supervisor_exec_result_must_stay_inside_output_bound() {
        let result = ExecCommandResult {
            success: false,
            exited: true,
            exit_code: 1,
            stdout: "abcd".to_string(),
            stderr: String::new(),
            stdout_truncated: true,
            stderr_truncated: false,
        };

        let err = ensure_exec_command_result_matches_request(&result, 1).unwrap_err();

        assert_session_supervisor_authority_mismatch(err, "execCommand", "bounded output limit");
    }

    #[test]
    fn session_supervisor_client_uses_descriptor_capability_templates() {
        let session = Uuid::from_bytes([0x35; 16]);
        let root = temp_project("capability-template");
        let manager = test_session_manager(&root);
        let rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
        let client = SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(&rpc))
            .with_session_scope(session)
            .with_capability_templates(vec![
                Capability::new(
                    ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                    ServiceRole::Editor,
                )
                .with_session(session)
                .with_audience(SESSION_SUPERVISOR_AUDIENCE)
                .with_permissions([SESSION_READ_PERMISSION])
                .with_token_hash([0x35]),
            ]);

        let capability = client
            .session_capability([SESSION_READ_PERMISSION])
            .unwrap();
        assert_eq!(capability.token_hash, vec![0x35]);
        assert!(
            client
                .session_capability([SESSION_SAVE_PERMISSION])
                .is_err()
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_supervisor_client_rejects_empty_descriptor_capabilities() {
        let root = temp_project("empty-capability-template");
        let manager = test_session_manager(&root);
        let rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
        let client = SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(&rpc))
            .with_capability_templates(Vec::new());

        assert!(
            client
                .session_capability([SESSION_READ_PERMISSION])
                .is_err()
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_project_host_and_asset_processor_through_session_supervisor() {
        let root = temp_project("resolve");
        let manager = test_session_manager(&root);
        let manifest = manager
            .create_session(CreateSessionRequest::new("Editor Resolve"))
            .unwrap();
        let project_host = project_host_service_descriptor(
            Uuid::now_v7(),
            test_tcp_endpoint("project-host:editor-resolve"),
        );
        let asset_processor = asset_processor_service_descriptor(
            Uuid::now_v7(),
            test_tcp_endpoint("asset-processor:editor-resolve"),
        );
        manager
            .attach_project_service_descriptors(
                "editor-resolve",
                &[project_host.clone(), asset_processor.clone()],
            )
            .unwrap();

        let rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
        let client = SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(&rpc))
            .with_session_scope(manifest.id.0);

        let resolved_project_host =
            futures::executor::block_on(client.resolve_project_host_descriptor("editor-resolve"))
                .unwrap();
        assert_eq!(resolved_project_host, project_host);

        let resolved_asset_processor = futures::executor::block_on(
            client.resolve_asset_processor_descriptor("editor-resolve"),
        )
        .unwrap();
        assert_eq!(resolved_asset_processor, asset_processor);
        assert_eq!(
            session_supervisor_service_id(),
            ServiceId::new(
                SESSION_SUPERVISOR_NAMESPACE,
                SESSION_SUPERVISOR_SERVICE_NAME
            )
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runs_session_command_through_session_supervisor() {
        let root = temp_project("exec-command");
        let manager = test_session_manager(&root);
        let manifest = manager
            .create_session(CreateSessionRequest::new("Editor Exec"))
            .unwrap();
        std::fs::create_dir_all(&manifest.workspace_root).unwrap();
        let rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
        let client = SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(&rpc))
            .with_session_scope(manifest.id.0);
        let (program, args) = portable_echo_command("editor-exec");

        let result =
            futures::executor::block_on(client.exec_command("editor-exec", program, args, 64))
                .unwrap();

        assert!(result.success);
        assert!(result.stdout.contains("editor-exec"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn attached_controller_rejects_status_for_different_session_identity() {
        let root = temp_project("attached-status-mismatch");
        let manager = test_session_manager(&root);
        let manifest = manager
            .create_session(CreateSessionRequest::new("Attached Status Mismatch"))
            .unwrap();
        let proto_manifest = az_session::session_manifest_to_proto(&manifest);
        let rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
        let client = SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(&rpc))
            .with_session_scope(manifest.id.0);
        let mut attached_identity = AttachedSessionIdentity::from_manifest(&proto_manifest);
        attached_identity.project_id = "local.other_project".to_string();
        let controller = EditorSessionStatusController::new(client, "attached-status-mismatch")
            .with_attached_identity(attached_identity);

        // The supervisor RPC serves status through `spawn_blocking`, so the
        // refresh must run inside a Tokio reactor context.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let err = local.block_on(&runtime, async { controller.refresh().await.unwrap_err() });

        match err {
            EditorError::SessionDiscoveryMismatch {
                operation,
                session,
                reason,
            } => {
                assert_eq!(operation, "session status refresh");
                assert_eq!(session, "attached-status-mismatch");
                assert!(reason.contains("does not match attached project"));
            }
            other => panic!("expected session discovery mismatch, got {other:?}"),
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn attached_identity_rejects_service_log_outside_run_dir() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = valid_session_manifest("editor");
        manifest.project_root = temp.path().join("project").to_string_lossy().into_owned();
        manifest.workspace_root = temp.path().join("workspace").to_string_lossy().into_owned();
        manifest.run_dir = temp
            .path()
            .join("runs/editor")
            .to_string_lossy()
            .into_owned();
        let identity = AttachedSessionIdentity::from_manifest(&manifest);
        let result = ServiceLogResult {
            session_slug: "editor".to_string(),
            service_name: "project-host".to_string(),
            run: Uuid::now_v7(),
            stream: ServiceLogStream::Stdout,
            path: temp
                .path()
                .join("other/project-host.stdout.log")
                .to_string_lossy()
                .into_owned(),
            lines: Vec::new(),
            next_offset: 0,
        };

        let err = identity
            .ensure_service_log(&result, "session service log")
            .unwrap_err();

        match err {
            EditorError::SessionDiscoveryMismatch {
                operation,
                session,
                reason,
            } => {
                assert_eq!(operation, "session service log");
                assert_eq!(session, "editor");
                assert!(reason.contains("outside attached session run directory"));
            }
            other => panic!("expected session discovery mismatch, got {other:?}"),
        }
    }

    #[test]
    fn recovers_failed_preserved_session_through_session_supervisor() {
        let root = temp_project("editor-recover");
        let manager = test_session_manager(&root);
        let manifest = manager
            .create_session(CreateSessionRequest::new("Editor Recover"))
            .unwrap();
        manager
            .mark_failed_preserved("editor-recover", "project-host exited")
            .unwrap();
        let rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
        let client = SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(&rpc))
            .with_session_scope(manifest.id.0);

        let recovered =
            futures::executor::block_on(client.recover("editor-recover", false)).unwrap();

        assert_eq!(recovered.state, ProtoSessionState::Active);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_status_watch_publishes_changes_then_exits_on_shutdown() {
        let root = temp_project("status-watch");
        let thread_root = root.clone();
        let (status_tx, mut status_rx): (SessionStatusSender, SessionStatusReceiver) =
            CoalescedLatestSender::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        // Everything RPC-shaped lives on the watcher thread: the fake
        // supervisor client is only usable where it was created.
        let watcher = std::thread::spawn(move || {
            let manager = test_session_manager(&thread_root);
            let manifest = manager
                .create_session(CreateSessionRequest::new("Editor Watch"))
                .unwrap();
            let rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
            let client = SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(&rpc))
                .with_session_scope(manifest.id.0);
            let controller = EditorSessionStatusController::new(client, "editor-watch");
            // A hand-built snapshot carries a different session id than the
            // manager serves, so the subscription's atomic initial snapshot
            // differs and gets published.
            let initial =
                session_workspace_status_to_ui(valid_session_workspace_status("editor-watch"));
            run_session_status_watch(
                controller,
                initial,
                status_tx,
                "editor-watch".to_string(),
                shutdown_rx,
            );
        });

        let published = futures::executor::block_on(async {
            match status_rx.recv().await {
                Some(SessionStatusDelivery::Status(status)) => status,
                Some(SessionStatusDelivery::Terminal(err)) => {
                    panic!("watcher refresh failed: {err}")
                }
                None => panic!("watcher exited before publishing a status"),
            }
        });
        assert_eq!(published.session_slug, "editor-watch");

        // Firing shutdown retires the watcher, which drops its sender and
        // closes the channel.
        drop(shutdown_tx);
        let closed = futures::executor::block_on(status_rx.recv());
        assert!(
            closed.is_none(),
            "expected channel close after shutdown, got {closed:?}"
        );
        watcher.join().unwrap();

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn session_status_delivery_coalesces_a_stalled_consumer_to_the_latest_snapshot() {
        let (sender, mut receiver) = CoalescedLatestSender::channel();
        let first = session_workspace_status_to_ui(valid_session_workspace_status("first"));
        let latest = session_workspace_status_to_ui(valid_session_workspace_status("latest"));

        assert!(sender.publish(SessionStatusDelivery::status(first)));
        assert!(sender.publish(SessionStatusDelivery::status(latest.clone())));

        match futures::executor::block_on(receiver.recv()) {
            Some(SessionStatusDelivery::Status(observed)) => assert_eq!(observed, latest),
            Some(SessionStatusDelivery::Terminal(error)) => {
                panic!("expected a status snapshot, got terminal error: {error}")
            }
            None => panic!("coalesced status receiver closed"),
        }
    }

    #[test]
    fn session_status_delivery_rejects_a_closed_consumer() {
        let (sender, receiver): (SessionStatusSender, SessionStatusReceiver) =
            CoalescedLatestSender::channel();
        drop(receiver);

        let status = session_workspace_status_to_ui(valid_session_workspace_status("closed"));
        assert!(
            !sender.publish(SessionStatusDelivery::status(status)),
            "closed UI delivery must fail the sink so the broker can prune it"
        );
    }

    #[test]
    fn session_status_delivery_keeps_terminal_errors_distinct_from_snapshots() {
        let (sender, mut receiver): (SessionStatusSender, SessionStatusReceiver) =
            CoalescedLatestSender::channel();
        let status = session_workspace_status_to_ui(valid_session_workspace_status("before-error"));

        assert!(sender.publish(SessionStatusDelivery::status(status)));
        assert!(sender.publish(SessionStatusDelivery::terminal(
            EditorError::ServiceDiscovery("subscription closed".to_string()),
        )));

        assert!(matches!(
            futures::executor::block_on(receiver.recv()),
            Some(SessionStatusDelivery::Terminal(EditorError::ServiceDiscovery(reason)))
                if reason == "subscription closed"
        ));
    }

    #[cfg(windows)]
    fn portable_echo_command(text: &str) -> (String, Vec<String>) {
        (
            "cmd".to_string(),
            vec!["/C".to_string(), format!("echo {text}")],
        )
    }

    #[cfg(not(windows))]
    fn portable_echo_command(text: &str) -> (String, Vec<String>) {
        (
            "sh".to_string(),
            vec!["-c".to_string(), format!("printf {text}")],
        )
    }

    #[test]
    fn requests_service_start_through_session_supervisor() {
        let root = temp_project("start-services");
        let manager = test_session_manager(&root);
        let manifest = manager
            .create_session(CreateSessionRequest::new("Editor Start"))
            .unwrap();
        let status = manager.status("editor-start").unwrap();
        let (command_sender, command_receiver) = session_supervisor_command_channel();
        let rpc = Rc::new(SessionSupervisorRpc::with_manager_and_command_sender(
            manager,
            "editor-start",
            command_sender,
        ));
        let command_thread = std::thread::spawn(move || match command_receiver.recv().unwrap() {
            SessionSupervisorCommand::Start { filter, completion } => {
                assert!(filter.is_all());
                completion
                    .send(Ok(SessionSupervisorCommandResult::Started {
                        report: StartServicesReport {
                            started: Vec::new(),
                            skipped: vec!["already-ready".to_string()],
                        },
                        status,
                    }))
                    .unwrap();
            }
            other => panic!("expected terminal start command, got {other:?}"),
        });
        let client = SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(&rpc))
            .with_session_scope(manifest.id.0);

        let result =
            futures::executor::block_on(client.start_services("editor-start", "editor startup"))
                .unwrap();

        assert!(result.started.is_empty());
        assert_eq!(result.skipped, ["already-ready"]);
        assert_eq!(result.status.manifest.slug, "editor-start");
        command_thread.join().unwrap();

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn requests_service_shutdown_through_session_supervisor() {
        let root = temp_project("stop-services");
        let manager = test_session_manager(&root);
        let manifest = manager
            .create_session(CreateSessionRequest::new("Editor Stop"))
            .unwrap();
        let status = manager.status("editor-stop").unwrap();
        let (command_sender, command_receiver) = session_supervisor_command_channel();
        let rpc = Rc::new(SessionSupervisorRpc::with_manager_and_command_sender(
            manager,
            "editor-stop",
            command_sender,
        ));
        let command_thread = std::thread::spawn(move || {
            match command_receiver.recv().unwrap() {
                SessionSupervisorCommand::Stop { reason, completion } => {
                    assert_eq!(reason, "editor shutdown");
                    completion
                        .send(Ok(SessionSupervisorCommandResult::Stopped {
                            report: StopServicesReport {
                                stopped: Vec::new(),
                                skipped: vec!["already-stopped".to_string()],
                            },
                            status,
                        }))
                        .unwrap();
                }
                other => panic!("expected terminal stop command, got {other:?}"),
            }
            match command_receiver.recv().unwrap() {
                SessionSupervisorCommand::Shutdown { reason } => {
                    assert_eq!(reason, "editor shutdown");
                }
                other => panic!("expected one-way shutdown command, got {other:?}"),
            }
        });
        let client = SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(&rpc))
            .with_session_scope(manifest.id.0);

        let result =
            futures::executor::block_on(client.stop_services("editor-stop", "editor shutdown"))
                .unwrap();

        assert!(result.stopped.is_empty());
        assert_eq!(result.skipped, ["already-stopped"]);
        assert_eq!(result.status.manifest.slug, "editor-stop");
        futures::executor::block_on(client.shutdown_supervisor("editor-stop", "editor shutdown"))
            .unwrap();
        command_thread.join().unwrap();

        std::fs::remove_dir_all(root).unwrap();
    }
}
