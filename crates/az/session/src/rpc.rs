//! Cap'n Proto adapter for the Lore-backed session supervisor.
//!
//! This is the daemon-facing service boundary: callers use the generated
//! `SessionSupervisor` client, while the implementation delegates to
//! `SessionManager` so workspace creation, failure preservation, and status
//! reporting keep one source of truth.

use std::collections::{BTreeSet, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use az_filesystem::AzothDataHome;
use az_observability::{format_log_record_for_console, read_observed_log_file_from_offset};
use az_proto_asset::{
    ASSET_PROCESSOR_AUDIENCE, ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME,
    ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION, AssetRootScope, ForceReprocessAssetRequest,
    ForceReprocessAssetResult, SourceAssetRecordRequest, SourceAssetRecordResult,
    WorkspaceEntryDiff, WorkspaceRoot, WorkspaceSnapshot, WorkspaceSnapshotRequest,
    WorkspaceSnapshotResult, asset_capnp,
};
use az_proto_core::{
    Capability, CapabilityGrantSet, ServiceDescriptor, ServiceHealth, ServiceHealthState,
    ServiceId, ServiceRole,
};
use az_proto_project::{
    FromCapnp as _, PROJECT_DOCUMENT_WRITE_PERMISSION, PROJECT_HOST_AUDIENCE,
    PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME, ProjectDocumentRequest, SaveDocumentResult,
    ToCapnp as _, project_capnp,
};
use az_proto_session::{
    ExecCommandRequest, ExecCommandResult, RecoverSessionRequest, RegisterServiceRequest,
    ResolveServiceRequest, RuntimeAssetPackageRootsRequest, RuntimeAssetPackageRootsResult,
    SESSION_EXEC_PERMISSION, SESSION_MANAGE_PERMISSION, SESSION_READ_PERMISSION,
    SESSION_SAVE_PERMISSION, SESSION_SUPERVISOR_AUDIENCE, SESSION_SUPERVISOR_NAMESPACE,
    SESSION_SUPERVISOR_SERVICE_NAME, SaveGraphDocumentRequest, SaveGraphDocumentResult,
    ServiceLogRequest, ServiceLogResult, ServiceLogStream, SessionCapabilityRequest,
    SessionSlugRequest, SessionSupervisorEventSubscriptionRequest, SessionSupervisorIdentity,
    StartServicesRequest, StartServicesResult, StopServicesRequest, StopServicesResult,
    session_capnp,
};
use az_service_supervision::{
    ServiceProcessRecord, ServiceProcessState, ServiceRecord, SupervisedServiceRole,
    previous_log_path,
};
use az_work::owned_command_output;
use capnp::Error;
use tracing::info;
use uuid::Uuid;

use crate::status_broker::SessionStatusBroker;
use crate::{
    CreateSessionRequest, SessionError, SessionManager, SessionManifest, SessionState,
    SessionStatus, SessionSupervisorCommandResult, SessionSupervisorCommandSender,
    StartServicesFilter, runtime_asset_package_roots_for_session,
};

#[derive(Debug)]
pub struct SessionSupervisorRpc {
    manager: SessionManager,
    controlled_session: Option<String>,
    command_sender: Option<SessionSupervisorCommandSender>,
    enforce_descriptor_grants: bool,
    service_run: Arc<RwLock<Uuid>>,
    supervision_identity: Arc<RwLock<Option<SessionSupervisorIdentity>>>,
    status_broker: Rc<SessionStatusBroker>,
    started_at: Instant,
}

impl SessionSupervisorRpc {
    /// Builds an adapter that serves every session in the project at
    /// `project_root`, with no controlled session and no command sender.
    ///
    /// # Errors
    ///
    /// Returns any error [`SessionManager::new`] returns.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new(project_root: impl AsRef<Path>) -> Result<Self, SessionError> {
        Ok(Self {
            manager: SessionManager::new(project_root)?,
            controlled_session: None,
            command_sender: None,
            enforce_descriptor_grants: false,
            service_run: Arc::new(RwLock::new(Uuid::nil())),
            supervision_identity: Arc::new(RwLock::new(None)),
            status_broker: SessionStatusBroker::new(),
            started_at: Instant::now(),
        })
    }

    /// Test-facing alias for [`Self::new`].
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::new`] returns.
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_new(project_root: impl AsRef<Path>) -> Result<Self, SessionError> {
        Self::new(project_root)
    }
}

impl SessionSupervisorRpc {
    pub(crate) fn new_with_command_sender(
        project_root: impl AsRef<Path>,
        data_home: AzothDataHome,
        controlled_session: impl Into<String>,
        command_sender: SessionSupervisorCommandSender,
    ) -> Result<Self, SessionError> {
        Ok(Self {
            manager: SessionManager::with_data_home(project_root, data_home)?,
            controlled_session: Some(controlled_session.into()),
            command_sender: Some(command_sender),
            enforce_descriptor_grants: false,
            service_run: Arc::new(RwLock::new(Uuid::nil())),
            supervision_identity: Arc::new(RwLock::new(None)),
            status_broker: SessionStatusBroker::new(),
            started_at: Instant::now(),
        })
    }
}

impl SessionSupervisorRpc {
    #[must_use]
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_manager(manager: SessionManager) -> Self {
        Self {
            manager,
            controlled_session: None,
            command_sender: None,
            enforce_descriptor_grants: false,
            service_run: Arc::new(RwLock::new(Uuid::nil())),
            supervision_identity: Arc::new(RwLock::new(None)),
            status_broker: SessionStatusBroker::new(),
            started_at: Instant::now(),
        }
    }

    #[must_use]
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_with_manager(manager: SessionManager) -> Self {
        Self::with_manager(manager)
    }

    #[must_use]
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_manager_and_command_sender(
        manager: SessionManager,
        controlled_session: impl Into<String>,
        command_sender: SessionSupervisorCommandSender,
    ) -> Self {
        Self {
            manager,
            controlled_session: Some(controlled_session.into()),
            command_sender: Some(command_sender),
            enforce_descriptor_grants: false,
            service_run: Arc::new(RwLock::new(Uuid::nil())),
            supervision_identity: Arc::new(RwLock::new(None)),
            status_broker: SessionStatusBroker::new(),
            started_at: Instant::now(),
        }
    }

    #[must_use]
    pub(crate) const fn with_descriptor_grants(mut self) -> Self {
        self.enforce_descriptor_grants = true;
        self
    }

    #[must_use]
    pub(crate) fn with_service_run(mut self, service_run: Arc<RwLock<Uuid>>) -> Self {
        self.service_run = service_run;
        self
    }

    #[must_use]
    pub(crate) fn with_supervision_identity(
        mut self,
        supervision_identity: Arc<RwLock<Option<SessionSupervisorIdentity>>>,
    ) -> Self {
        self.supervision_identity = supervision_identity;
        self
    }

    #[must_use]
    pub(crate) fn with_status_broker(mut self, status_broker: Rc<SessionStatusBroker>) -> Self {
        self.status_broker = status_broker;
        self
    }

    fn publish_manifest_status(
        self: &Rc<Self>,
        manifest: &SessionManifest,
        failure_reason: Option<String>,
    ) {
        self.status_broker.publish(
            &manifest.slug,
            session_workspace_status_to_proto(&SessionStatus {
                manifest: manifest.clone(),
                failure_reason,
            }),
        );
    }

    #[must_use]
    pub const fn manager(&self) -> &SessionManager {
        &self.manager
    }

    #[must_use]
    fn health_snapshot(&self) -> ServiceHealth {
        let busy = self
            .command_sender
            .as_ref()
            .is_some_and(SessionSupervisorCommandSender::has_pending_operation);
        ServiceHealth::ready(
            ServiceId::new(
                SESSION_SUPERVISOR_NAMESPACE,
                SESSION_SUPERVISOR_SERVICE_NAME,
            ),
            ServiceRole::SessionSupervisor,
            *self
                .service_run
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            az_proto_core::ProtocolVersion::CURRENT,
        )
        .with_state(if busy {
            ServiceHealthState::Busy
        } else {
            ServiceHealthState::Ready
        })
        .with_uptime_ms(duration_millis_u64(self.started_at.elapsed()))
        .with_active_operation(if busy { "lifecycle-command" } else { "" })
        .with_message(if busy {
            "session-supervisor lifecycle command pending"
        } else {
            "session-supervisor reachable"
        })
    }

    fn validate_capability(
        &self,
        capability: &Capability,
        required_permission: &str,
    ) -> Result<(), Error> {
        validate_session_capability(capability, required_permission)?;
        if self.enforce_descriptor_grants {
            let Some(controlled_session) = &self.controlled_session else {
                return Err(Error::failed(
                    "session-supervisor descriptor grants are enabled without a controlled session"
                        .to_string(),
                ));
            };
            let manifest = self
                .manager
                .session(controlled_session)
                .map_err(|error| session_error_to_capnp(&error))?;
            self.validate_descriptor_grant(capability, required_permission, &manifest)?;
        }
        Ok(())
    }

    fn validate_capability_for_manifest(
        &self,
        capability: &Capability,
        required_permission: &str,
        manifest: &SessionManifest,
    ) -> Result<(), Error> {
        validate_session_capability_for_manifest(capability, required_permission, manifest)?;
        if self.enforce_descriptor_grants {
            self.validate_descriptor_grant(capability, required_permission, manifest)?;
        }
        Ok(())
    }

    fn validate_descriptor_grant(
        &self,
        capability: &Capability,
        required_permission: &str,
        manifest: &SessionManifest,
    ) -> Result<(), Error> {
        let served_descriptor = self
            .supervision_identity
            .read()
            .map_err(|_| Error::failed("session-supervisor identity lock poisoned".into()))?
            .as_ref()
            .map(|identity| identity.descriptor.clone());
        let descriptor = served_descriptor
            .or_else(|| {
                manifest.service_descriptor(
                    &ServiceId::new(
                        SESSION_SUPERVISOR_NAMESPACE,
                        SESSION_SUPERVISOR_SERVICE_NAME,
                    ),
                    ServiceRole::SessionSupervisor,
                )
            })
            .ok_or_else(|| {
                Error::failed(format!(
                    "session-supervisor descriptor grants are enabled but `{}` has no session-supervisor descriptor",
                    manifest.slug
                ))
            })?;
        let broker_permission = if capability
            .permissions
            .iter()
            .any(|permission| permission == required_permission)
        {
            required_permission
        } else {
            SESSION_MANAGE_PERMISSION
        };
        CapabilityGrantSet::from_grants(descriptor.capabilities)
            .validate(capability, broker_permission)
            .map_err(|error| {
                Error::failed(format!("invalid session-supervisor capability: {error}"))
            })
    }
}

impl SessionSupervisorRpc {
    #[must_use]
    pub(crate) fn into_client(self) -> session_capnp::session_supervisor::Client {
        capnp_rpc::new_client(self)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn client_from_rc(this: &Rc<Self>) -> session_capnp::session_supervisor::Client {
        capnp_rpc::new_client_from_rc(Rc::clone(this))
    }
}

// Cannot be made `Send`: the capnp-generated `Server` trait hands every
// method `capnp::capability::Rc<Self>`, and `Self` owns an `Rc` status
// broker; these futures only ever run on the transport's `LocalSet`.
#[allow(clippy::future_not_send)]
impl session_capnp::session_supervisor::Server for SessionSupervisorRpc {
    async fn health(
        self: capnp::capability::Rc<Self>,
        _params: session_capnp::session_supervisor::HealthParams,
        mut results: session_capnp::session_supervisor::HealthResults,
    ) -> Result<(), Error> {
        (self.health_snapshot()).to_capnp(results.get().init_health())?;
        Ok(())
    }

    async fn supervision_identity(
        self: capnp::capability::Rc<Self>,
        _params: session_capnp::session_supervisor::SupervisionIdentityParams,
        mut results: session_capnp::session_supervisor::SupervisionIdentityResults,
    ) -> Result<(), Error> {
        let identity = self
            .supervision_identity
            .read()
            .map_err(|_| Error::failed("session-supervisor identity lock poisoned".into()))?
            .clone()
            .ok_or_else(|| Error::failed("session-supervisor identity is not published".into()))?;
        (identity).to_capnp(results.get().init_identity())?;
        Ok(())
    }

    async fn create(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::CreateParams,
        mut results: session_capnp::session_supervisor::CreateResults,
    ) -> Result<(), Error> {
        let params = params.get()?;
        self.validate_capability(
            &az_proto_core::Capability::from_capnp(params.get_capability()?)?,
            SESSION_MANAGE_PERMISSION,
        )?;
        let request = az_proto_session::CreateSessionRequest::from_capnp(params.get_request()?)?;
        let manifest = self
            .manager
            .create_session(CreateSessionRequest { name: request.name })
            .map_err(|error| session_error_to_capnp(&error))?;
        self.publish_manifest_status(&manifest, None);
        write_session_manifest(&manifest, results.get().init_manifest())?;
        Ok(())
    }

    async fn list(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::ListParams,
        mut results: session_capnp::session_supervisor::ListResults,
    ) -> Result<(), Error> {
        let request = SessionCapabilityRequest::from_capnp(params.get()?)?;
        self.validate_capability(&request.capability, SESSION_READ_PERMISSION)?;
        let sessions = self
            .manager
            .list_sessions()
            .map_err(|error| session_error_to_capnp(&error))?;
        let session_count = u32::try_from(sessions.len())
            .map_err(|_| Error::failed("session list is too large to encode".to_string()))?;
        let mut builder = results.get().init_sessions(session_count);
        for (index, manifest) in (0u32..).zip(sessions.iter()) {
            write_session_manifest(manifest, builder.reborrow().get(index))?;
        }
        Ok(())
    }

    async fn status(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::StatusParams,
        mut results: session_capnp::session_supervisor::StatusResults,
    ) -> Result<(), Error> {
        let request = SessionSlugRequest::from_capnp(params.get()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_READ_PERMISSION,
            &manifest,
        )?;
        let manager = self.manager.clone();
        let status = tokio::task::spawn_blocking(move || manager.status(&request.slug))
            .await
            .map_err(|error| Error::failed(format!("session status worker failed: {error}")))?
            .map_err(|error| session_error_to_capnp(&error))?;
        write_session_workspace_status(&status, results.get().init_status())?;
        Ok(())
    }

    async fn preserve_failed(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::PreserveFailedParams,
        mut results: session_capnp::session_supervisor::PreserveFailedResults,
    ) -> Result<(), Error> {
        let params = params.get()?;
        let slug = params.get_slug()?.to_str()?;
        let manifest = self
            .manager
            .session(slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &az_proto_core::Capability::from_capnp(params.get_capability()?)?,
            SESSION_MANAGE_PERMISSION,
            &manifest,
        )?;
        let reason = params.get_reason()?.to_string()?;
        let manifest = self
            .manager
            .mark_failed_preserved(slug, &reason)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.publish_manifest_status(&manifest, Some(reason));
        write_session_manifest(&manifest, results.get().init_manifest())?;
        Ok(())
    }

    async fn register_service(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::RegisterServiceParams,
        mut results: session_capnp::session_supervisor::RegisterServiceResults,
    ) -> Result<(), Error> {
        let request = RegisterServiceRequest::from_capnp(params.get()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_MANAGE_PERMISSION,
            &manifest,
        )?;
        self.manager
            .require_active_session(&request.slug, "register service")
            .map_err(|error| session_error_to_capnp(&error))?;
        let manifest = self
            .manager
            .register_service_descriptor(&request.slug, &request.descriptor)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.publish_manifest_status(&manifest, None);
        write_session_manifest(&manifest, results.get().init_manifest())?;
        Ok(())
    }

    async fn resolve_service(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::ResolveServiceParams,
        mut results: session_capnp::session_supervisor::ResolveServiceResults,
    ) -> Result<(), Error> {
        let request = ResolveServiceRequest::from_capnp(params.get()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_READ_PERMISSION,
            &manifest,
        )?;
        let descriptor = self
            .manager
            .service_descriptor(&request.slug, &request.id, request.role)
            .map_err(|error| session_error_to_capnp(&error))?
            .ok_or_else(|| {
                Error::failed(format!(
                    "session service `{}`/`{}` with role `{role:?}` was not registered",
                    request.id.namespace,
                    request.id.name,
                    role = request.role
                ))
            })?;
        az_proto_core::ServiceDescriptor::to_capnp(&descriptor, results.get().init_descriptor())?;
        Ok(())
    }

    async fn save_graph_document(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::SaveGraphDocumentParams,
        mut results: session_capnp::session_supervisor::SaveGraphDocumentResults,
    ) -> Result<(), Error> {
        let request = SaveGraphDocumentRequest::from_capnp(params.get()?.get_request()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_SAVE_PERMISSION,
            &manifest,
        )?;
        let result = save_graph_document_through_session_services(
            &self.manager,
            &request.slug,
            &request.document_id,
        )
        .await?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    async fn stop_services(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::StopServicesParams,
        mut results: session_capnp::session_supervisor::StopServicesResults,
    ) -> Result<(), Error> {
        let request = StopServicesRequest::from_capnp(params.get()?.get_request()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_MANAGE_PERMISSION,
            &manifest,
        )?;

        let Some(controlled_session) = &self.controlled_session else {
            return Err(Error::failed(
                "session-supervisor cannot stop services because this RPC endpoint is not bound to an az-sessiond shutdown flag".to_string(),
            ));
        };
        if controlled_session != &request.slug {
            return Err(Error::failed(format!(
                "session-supervisor for session `{controlled_session}` cannot stop services for session `{}`",
                request.slug
            )));
        }
        let reason = if request.reason.trim().is_empty() {
            "session-supervisor stopServices request".to_string()
        } else {
            request.reason
        };
        let command_sender = self.command_sender.as_ref().ok_or_else(|| {
            Error::failed("session-supervisor has no durable control command channel".to_string())
        })?;
        let completion = command_sender
            .request_stop(reason.clone())
            .map_err(|error| session_error_to_capnp(&error))?;
        let outcome = completion
            .await
            .map_err(|_| {
                Error::failed(
                    "session-supervisor command loop stopped before stop completed".to_string(),
                )
            })?
            .map_err(|error| session_error_to_capnp(&error))?;
        let SessionSupervisorCommandResult::Stopped { report, status } = outcome else {
            return Err(Error::failed(
                "session-supervisor returned a start outcome for stopServices".to_string(),
            ));
        };
        (StopServicesResult {
            status: session_workspace_status_to_proto(&status),
            stopped: report
                .stopped
                .into_iter()
                .map(|service| service.service_name)
                .collect(),
            skipped: report.skipped,
        })
        .to_capnp(results.get().init_result())?;
        info!(session = %controlled_session, reason = %reason, "session-supervisor services stopped over RPC");
        Ok(())
    }

    async fn start_services(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::StartServicesParams,
        mut results: session_capnp::session_supervisor::StartServicesResults,
    ) -> Result<(), Error> {
        let request = StartServicesRequest::from_capnp(params.get()?.get_request()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_MANAGE_PERMISSION,
            &manifest,
        )?;

        let Some(controlled_session) = &self.controlled_session else {
            return Err(Error::failed(
                "session-supervisor cannot start services because this RPC endpoint is not bound to an az-sessiond control loop".to_string(),
            ));
        };
        if controlled_session != &request.slug {
            return Err(Error::failed(format!(
                "session-supervisor for session `{controlled_session}` cannot start services for session `{}`",
                request.slug
            )));
        }
        let reason = if request.reason.trim().is_empty() {
            "session-supervisor startServices request".to_string()
        } else {
            request.reason
        };
        let service_filter = StartServicesFilter::named(request.service_names);
        let command_sender = self.command_sender.as_ref().ok_or_else(|| {
            Error::failed("session-supervisor has no durable control command channel".to_string())
        })?;
        let completion = command_sender
            .request_start(service_filter)
            .map_err(|error| session_error_to_capnp(&error))?;
        let outcome = completion
            .await
            .map_err(|_| {
                Error::failed(
                    "session-supervisor command loop stopped before start completed".to_string(),
                )
            })?
            .map_err(|error| session_error_to_capnp(&error))?;
        let SessionSupervisorCommandResult::Started { report, status } = outcome else {
            return Err(Error::failed(
                "session-supervisor returned a stop outcome for startServices".to_string(),
            ));
        };
        (StartServicesResult {
            status: session_workspace_status_to_proto(&status),
            started: report
                .started
                .into_iter()
                .map(|service| service.service_name)
                .collect(),
            skipped: report.skipped,
        })
        .to_capnp(results.get().init_result())?;
        info!(session = %controlled_session, reason = %reason, "session-supervisor services started over RPC");
        Ok(())
    }

    async fn shutdown_supervisor(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::ShutdownSupervisorParams,
    ) -> Result<(), Error> {
        let params = params.get()?;
        let slug = params.get_slug()?.to_string()?;
        let manifest = self
            .manager
            .session(&slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &az_proto_core::Capability::from_capnp(params.get_capability()?)?,
            SESSION_MANAGE_PERMISSION,
            &manifest,
        )?;
        let controlled_session = self.controlled_session.as_ref().ok_or_else(|| {
            Error::failed(
                "session-supervisor cannot terminate because this RPC endpoint is not bound to an az-sessiond control loop".to_string(),
            )
        })?;
        if controlled_session != &slug {
            return Err(Error::failed(format!(
                "session-supervisor for session `{controlled_session}` cannot terminate session `{slug}`"
            )));
        }
        let reason = params.get_reason()?.to_string()?;
        if reason.trim().is_empty() {
            return Err(Error::failed(
                "shutdown-supervisor reason cannot be empty".to_string(),
            ));
        }
        self.command_sender
            .as_ref()
            .ok_or_else(|| {
                Error::failed(
                    "session-supervisor has no durable control command channel".to_string(),
                )
            })?
            .request_shutdown(reason)
            .map_err(|error| session_error_to_capnp(&error))?;
        Ok(())
    }

    async fn subscribe_events(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::SubscribeEventsParams,
        mut results: session_capnp::session_supervisor::SubscribeEventsResults,
    ) -> Result<(), Error> {
        let params = params.get()?;
        let request = SessionSupervisorEventSubscriptionRequest::from_capnp(params.get_request()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_READ_PERMISSION,
            &manifest,
        )?;
        let status = self
            .manager
            .status(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        let result = self.status_broker.subscribe(
            request.slug,
            session_workspace_status_to_proto(&status),
            params.get_sink()?,
        );
        result.to_capnp(results.get().init_result())?;
        Ok(())
    }

    async fn recover(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::RecoverParams,
        mut results: session_capnp::session_supervisor::RecoverResults,
    ) -> Result<(), Error> {
        let request = RecoverSessionRequest::from_capnp(params.get()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_MANAGE_PERMISSION,
            &manifest,
        )?;
        let manifest = self
            .manager
            .recover_session(&request.slug, request.force)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.publish_manifest_status(&manifest, None);
        write_session_manifest(&manifest, results.get().init_manifest())?;
        Ok(())
    }

    async fn retire(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::RetireParams,
        mut results: session_capnp::session_supervisor::RetireResults,
    ) -> Result<(), Error> {
        let request = SessionSlugRequest::from_capnp(params.get()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_MANAGE_PERMISSION,
            &manifest,
        )?;
        let manifest = self
            .manager
            .retire_session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.publish_manifest_status(&manifest, None);
        write_session_manifest(&manifest, results.get().init_manifest())?;
        Ok(())
    }

    async fn service_log(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::ServiceLogParams,
        mut results: session_capnp::session_supervisor::ServiceLogResults,
    ) -> Result<(), Error> {
        let request = ServiceLogRequest::from_capnp(params.get()?.get_request()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_READ_PERMISSION,
            &manifest,
        )?;
        let result = read_session_service_log(&manifest, &request)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    async fn exec_command(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::ExecCommandParams,
        mut results: session_capnp::session_supervisor::ExecCommandResults,
    ) -> Result<(), Error> {
        let request = ExecCommandRequest::from_capnp(params.get()?.get_request()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_EXEC_PERMISSION,
            &manifest,
        )?;
        let manifest = self
            .manager
            .active_session_workspace(&request.slug, "exec")
            .map_err(|error| session_error_to_capnp(&error))?;
        let result = execute_session_command(&manifest, &request)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    async fn runtime_asset_package_roots(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor::RuntimeAssetPackageRootsParams,
        mut results: session_capnp::session_supervisor::RuntimeAssetPackageRootsResults,
    ) -> Result<(), Error> {
        let request = RuntimeAssetPackageRootsRequest::from_capnp(params.get()?.get_request()?)?;
        let manifest = self
            .manager
            .session(&request.slug)
            .map_err(|error| session_error_to_capnp(&error))?;
        self.validate_capability_for_manifest(
            &request.capability,
            SESSION_READ_PERMISSION,
            &manifest,
        )?;

        let roots = runtime_asset_package_roots_for_session(&manifest)
            .map_err(|error| session_error_to_capnp(&error))?;
        (RuntimeAssetPackageRootsResult { roots }).to_capnp(results.get().init_result())?;
        Ok(())
    }
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Writes `manifest` into a Cap'n Proto session-manifest builder.
///
/// # Errors
///
/// Returns [`Error`] if the message runs out of space while writing the
/// manifest's strings, service records, or process records.
pub fn write_session_manifest(
    manifest: &SessionManifest,
    builder: session_capnp::session_manifest::Builder<'_>,
) -> Result<(), Error> {
    (session_manifest_to_proto(manifest)).to_capnp(builder)
}

/// Writes `status` into a Cap'n Proto workspace-status builder.
///
/// # Errors
///
/// Returns [`Error`] if the message runs out of space while writing the
/// embedded manifest or the preserved failure reason.
pub fn write_session_workspace_status(
    status: &SessionStatus,
    builder: session_capnp::session_workspace_status::Builder<'_>,
) -> Result<(), Error> {
    (session_workspace_status_to_proto(status)).to_capnp(builder)
}

fn read_session_service_log(
    manifest: &SessionManifest,
    request: &ServiceLogRequest,
) -> Result<ServiceLogResult, Error> {
    let selected = select_service_log_process(manifest, request)?;
    let process = selected.process;
    let path = service_log_path(manifest, process, request.stream, selected.previous)?;
    let (lines, next_offset) = match request.stream {
        ServiceLogStream::Structured => read_structured_service_log_lines(
            &path,
            request.all,
            request.tail_lines as usize,
            request.offset,
        )?,
        ServiceLogStream::Stdout | ServiceLogStream::Stderr => read_service_log_lines(
            &path,
            request.all,
            request.tail_lines as usize,
            request.offset,
        )?,
    };

    Ok(ServiceLogResult {
        session_slug: manifest.slug.clone(),
        service_name: process.service_name.clone(),
        run: request.run.unwrap_or(process.run),
        stream: request.stream,
        path: path_to_protocol_string(&path),
        lines,
        next_offset,
    })
}

const DEFAULT_EXEC_OUTPUT_LIMIT_BYTES: usize = 64 * 1024;

fn execute_session_command(
    manifest: &SessionManifest,
    request: &ExecCommandRequest,
) -> Result<ExecCommandResult, Error> {
    if request.program.trim().is_empty() {
        return Err(Error::failed(
            "session exec command requires a non-empty program".to_string(),
        ));
    }

    let mut command = Command::new(&request.program);
    command
        .current_dir(&manifest.workspace_root)
        .args(&request.args);
    let output = owned_command_output(&mut command).map_err(|error| {
        Error::failed(format!(
            "failed to run `{}` in session `{}` workspace `{}`: {error}",
            request.program,
            manifest.slug,
            manifest.workspace_root.display()
        ))
    })?;
    let max_output_bytes = if request.max_output_bytes == 0 {
        DEFAULT_EXEC_OUTPUT_LIMIT_BYTES
    } else {
        request.max_output_bytes as usize
    };
    let (stdout, stdout_truncated) = bounded_utf8_lossy(&output.stdout, max_output_bytes);
    let (stderr, stderr_truncated) = bounded_utf8_lossy(&output.stderr, max_output_bytes);

    Ok(ExecCommandResult {
        success: output.status.success(),
        exited: output.status.code().is_some(),
        exit_code: output.status.code().unwrap_or_default(),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
}

fn bounded_utf8_lossy(bytes: &[u8], max_bytes: usize) -> (String, bool) {
    let limit = max_bytes.min(bytes.len());
    (
        String::from_utf8_lossy(&bytes[..limit]).into_owned(),
        bytes.len() > limit,
    )
}

struct SelectedServiceLog<'a> {
    process: &'a ServiceProcessRecord,
    previous: bool,
}

fn select_service_log_process<'a>(
    manifest: &'a SessionManifest,
    request: &ServiceLogRequest,
) -> Result<SelectedServiceLog<'a>, Error> {
    let process = manifest
        .processes
        .iter()
        .find(|process| {
            process.service_name == request.service_name
                && process.role == SupervisedServiceRole::RuntimeHost
        })
        .ok_or_else(|| {
            Error::failed(format!(
                "session `{}` has no process record for service `{}`",
                manifest.slug, request.service_name
            ))
        })?;
    match request.run {
        None => Ok(SelectedServiceLog {
            process,
            previous: false,
        }),
        Some(run) if run == process.run => Ok(SelectedServiceLog {
            process,
            previous: false,
        }),
        Some(run) if process.previous_run == Some(run) => Ok(SelectedServiceLog {
            process,
            previous: true,
        }),
        Some(run) => Err(Error::failed(format!(
            "session `{}` service `{}` does not retain run `{run}`",
            manifest.slug, request.service_name
        ))),
    }
}

fn service_log_path(
    manifest: &SessionManifest,
    process: &ServiceProcessRecord,
    stream: ServiceLogStream,
    previous: bool,
) -> Result<PathBuf, Error> {
    let raw_path = match stream {
        ServiceLogStream::Stdout => &process.stdout_log,
        ServiceLogStream::Stderr => &process.stderr_log,
        ServiceLogStream::Structured => &process.structured_log,
    };
    let raw_path = if previous {
        previous_log_path(raw_path)
    } else {
        raw_path.clone()
    };
    let path = if raw_path.is_absolute() {
        raw_path
    } else {
        manifest.run_dir.join(raw_path)
    };
    if path_has_parent_component(&path) || !path.starts_with(&manifest.run_dir) {
        return Err(Error::failed(format!(
            "refusing to read service log `{}` outside session run directory `{}`",
            path.display(),
            manifest.run_dir.display()
        )));
    }
    Ok(path)
}

fn path_has_parent_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn read_service_log_lines(
    path: &Path,
    all: bool,
    tail_lines: usize,
    offset: Option<u64>,
) -> Result<(Vec<String>, u64), Error> {
    let file = File::open(path).map_err(|error| {
        Error::failed(format!(
            "failed to open service log `{}`: {error}",
            path.display()
        ))
    })?;
    let file_len = file
        .metadata()
        .map_err(|error| {
            Error::failed(format!(
                "failed to stat service log `{}`: {error}",
                path.display()
            ))
        })?
        .len();
    let mut reader = BufReader::new(file);

    if let Some(offset) = offset {
        reader
            .seek(SeekFrom::Start(offset.min(file_len)))
            .map_err(|error| {
                Error::failed(format!(
                    "failed to seek service log `{}`: {error}",
                    path.display()
                ))
            })?;
        let lines = reader
            .by_ref()
            .lines()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                Error::failed(format!(
                    "failed to read service log `{}`: {error}",
                    path.display()
                ))
            })?;
        let next_offset = reader.stream_position().map_err(|error| {
            Error::failed(format!(
                "failed to read service log offset `{}`: {error}",
                path.display()
            ))
        })?;
        return Ok((lines, next_offset));
    }

    let lines = if all {
        reader
            .by_ref()
            .lines()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                Error::failed(format!(
                    "failed to read service log `{}`: {error}",
                    path.display()
                ))
            })?
    } else if tail_lines == 0 {
        reader.seek(SeekFrom::End(0)).map_err(|error| {
            Error::failed(format!(
                "failed to seek service log `{}`: {error}",
                path.display()
            ))
        })?;
        Vec::new()
    } else {
        let mut lines = VecDeque::with_capacity(tail_lines);
        for line in reader.by_ref().lines() {
            if lines.len() == tail_lines {
                lines.pop_front();
            }
            lines.push_back(line.map_err(|error| {
                Error::failed(format!(
                    "failed to read service log `{}`: {error}",
                    path.display()
                ))
            })?);
        }
        lines.into_iter().collect()
    };
    let next_offset = reader.stream_position().map_err(|error| {
        Error::failed(format!(
            "failed to read service log offset `{}`: {error}",
            path.display()
        ))
    })?;
    Ok((lines, next_offset))
}

fn read_structured_service_log_lines(
    path: &Path,
    all: bool,
    tail_records: usize,
    offset: Option<u64>,
) -> Result<(Vec<String>, u64), Error> {
    if offset.is_none() && !all && tail_records == 0 {
        let len = File::open(path)
            .and_then(|file| file.metadata())
            .map_err(|error| {
                Error::failed(format!(
                    "failed to stat structured service log `{}`: {error}",
                    path.display()
                ))
            })?
            .len();
        return Ok((Vec::new(), len));
    }

    let read = read_observed_log_file_from_offset(path, offset.unwrap_or(0)).map_err(|error| {
        Error::failed(format!(
            "failed to read structured service log `{}`: {error}",
            path.display()
        ))
    })?;
    let records = if offset.is_none() && !all {
        let keep = tail_records.min(read.records.len());
        let skip = read.records.len().saturating_sub(keep);
        read.records.into_iter().skip(skip).collect::<Vec<_>>()
    } else {
        read.records
    };
    let lines = records
        .iter()
        .map(format_log_record_for_console)
        .collect::<Vec<_>>();
    Ok((lines, read.next_offset))
}

/// The session manifest and the three descriptors a graph-document save needs
/// before it can talk to anyone.
struct SaveGraphDocumentTargets {
    manifest: SessionManifest,
    session_supervisor: ServiceDescriptor,
    project_host: ServiceDescriptor,
    asset_processor: ServiceDescriptor,
}

/// The three brokered capabilities a graph-document save presents.
struct SaveGraphDocumentCapabilities {
    project_write: Capability,
    asset_write: Capability,
    asset_read: Capability,
}

fn resolve_save_graph_document_targets(
    manager: &SessionManager,
    slug: &str,
) -> Result<SaveGraphDocumentTargets, Error> {
    let manifest = manager
        .require_active_session(slug, "save graph document")
        .map_err(|error| session_error_to_capnp(&error))?;
    let session_supervisor = required_service_descriptor(
        &manifest,
        &ServiceId::new(
            SESSION_SUPERVISOR_NAMESPACE,
            SESSION_SUPERVISOR_SERVICE_NAME,
        ),
        ServiceRole::SessionSupervisor,
    )?;
    let project_host = required_service_descriptor(
        &manifest,
        &ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
        ServiceRole::ProjectHost,
    )?;
    let asset_processor = required_service_descriptor(
        &manifest,
        &ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
        ServiceRole::AssetProcessor,
    )?;
    Ok(SaveGraphDocumentTargets {
        manifest,
        session_supervisor,
        project_host,
        asset_processor,
    })
}

fn save_graph_document_capabilities(
    targets: &SaveGraphDocumentTargets,
) -> Result<SaveGraphDocumentCapabilities, Error> {
    let project_write = descriptor_capability(
        &targets.manifest,
        &targets.project_host,
        &targets.session_supervisor.id,
        ServiceRole::SessionSupervisor,
        PROJECT_HOST_AUDIENCE,
        PROJECT_DOCUMENT_WRITE_PERMISSION,
    )?;
    let asset_write = descriptor_capability(
        &targets.manifest,
        &targets.asset_processor,
        &targets.session_supervisor.id,
        ServiceRole::SessionSupervisor,
        ASSET_PROCESSOR_AUDIENCE,
        ASSET_WRITE_PERMISSION,
    )?;
    let asset_read = descriptor_capability(
        &targets.manifest,
        &targets.asset_processor,
        &targets.session_supervisor.id,
        ServiceRole::SessionSupervisor,
        ASSET_PROCESSOR_AUDIENCE,
        ASSET_READ_PERMISSION,
    )?;
    Ok(SaveGraphDocumentCapabilities {
        project_write,
        asset_write,
        asset_read,
    })
}

/// Preserve the session with `reason` and surface `error`, or — when the
/// preservation itself fails — an error naming both failures.
fn preserve_session_and_fail(
    manager: &SessionManager,
    slug: &str,
    reason: &str,
    error: Error,
) -> Error {
    if let Err(preserve_error) = manager.mark_failed_preserved(slug, reason) {
        return Error::failed(format!(
            "{reason}; additionally failed to preserve session `{slug}`: {preserve_error}"
        ));
    }
    error
}

// Cannot be made `Send`: this future holds capnp-rpc `Client`
// capabilities, which are `!Send` by construction, and is only ever
// awaited on the session-supervisor transport's `LocalSet`.
#[allow(clippy::future_not_send)]
async fn save_graph_document_through_session_services(
    manager: &SessionManager,
    slug: &str,
    document_id: &str,
) -> Result<SaveGraphDocumentResult, Error> {
    let targets = resolve_save_graph_document_targets(manager, slug)?;
    let project_host_client = connect_project_host(&targets.project_host).await?;
    let asset_processor_client = connect_asset_processor(&targets.asset_processor).await?;
    let capabilities = save_graph_document_capabilities(&targets)?;
    let manifest = &targets.manifest;

    let project_assets_root =
        project_assets_root_context(&asset_processor_client, capabilities.asset_read, manifest)
            .await?;
    let saved = save_project_graph_document(
        &project_host_client,
        capabilities.project_write,
        document_id,
    )
    .await?;
    let asset_record_result = record_saved_source_asset(
        &asset_processor_client,
        manifest,
        capabilities.asset_write.clone(),
        &project_assets_root,
        &saved,
    )
    .await;
    let mut asset_record = match asset_record_result {
        Ok(asset_record) => asset_record,
        Err(error) => {
            let reason = format!(
                "saved {} `{}` revision {} but asset registry handoff failed: {error}",
                "graph document", saved.source_path, saved.revision.0
            );
            return Err(preserve_session_and_fail(
                manager,
                &manifest.slug,
                &reason,
                error,
            ));
        }
    };
    let mut forced_enqueued_jobs = None;
    if asset_record.entry.diff == WorkspaceEntryDiff::Clean {
        let force_result = force_reprocess_current_saved_source(
            &asset_processor_client,
            capabilities.asset_write,
            manifest,
            &project_assets_root,
            &saved,
        )
        .await;
        match force_result {
            Ok(result) => {
                forced_enqueued_jobs = Some(result.enqueued_jobs);
                asset_record = result.record;
            }
            Err(error) => {
                let reason = format!(
                    "saved {} `{}` revision {} and recorded source asset but automatic asset reprocess failed: {error}",
                    "graph document", saved.source_path, saved.revision.0
                );
                return Err(preserve_session_and_fail(
                    manager,
                    &manifest.slug,
                    &reason,
                    error,
                ));
            }
        }
    }

    info!(
        session = %manifest.slug,
        document = %document_id,
        revision = saved.revision.0,
        asset_guid = %asset_record.asset_guid,
        forced_enqueued_jobs = forced_enqueued_jobs.unwrap_or(0),
        "session supervisor saved graph document and recorded source asset"
    );

    Ok(SaveGraphDocumentResult {
        saved,
        asset_record,
    })
}

// Cannot be made `Send`: this future holds capnp-rpc `Client`
// capabilities, which are `!Send` by construction, and is only ever
// awaited on the session-supervisor transport's `LocalSet`.
#[allow(clippy::future_not_send)]
async fn save_project_graph_document(
    client: &project_capnp::project_host::Client,
    capability: Capability,
    document_id: &str,
) -> Result<az_proto_project::SavedDocument, Error> {
    let request = ProjectDocumentRequest::new(capability, document_id);
    let mut rpc = client.save_graph_document_request();
    request.to_capnp(rpc.get())?;
    let response = rpc.send().promise.await?;
    let saved = SaveDocumentResult::from_capnp(response.get()?)?.saved;
    ensure_saved_document_matches_request(&saved, document_id)?;
    Ok(saved)
}

// Cannot be made `Send`: this future holds capnp-rpc `Client`
// capabilities, which are `!Send` by construction, and is only ever
// awaited on the session-supervisor transport's `LocalSet`.
#[allow(clippy::future_not_send)]
async fn record_saved_source_asset(
    client: &asset_capnp::asset_processor::Client,
    manifest: &SessionManifest,
    write_capability: Capability,
    source_root: &ProjectAssetsRootContext,
    saved: &az_proto_project::SavedDocument,
) -> Result<az_proto_asset::SourceAssetRecordResult, Error> {
    let schema_type = (!saved.schema_type.is_empty()).then(|| saved.schema_type.clone());
    let request_dto = SourceAssetRecordRequest {
        capability: write_capability,
        session_id: manifest.id.0.to_string(),
        workspace_root_id: source_root.workspace_root_id,
        owner_id: source_root.owner_id.clone(),
        source_path: saved.source_path.clone(),
        schema_type,
        content_hash: saved.content_hash.clone(),
        changed_unix_ms: current_unix_ms_i64()?,
        diagnostics_count: 0,
    };

    let mut request = client.record_source_asset_request();
    (request_dto).to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let record = SourceAssetRecordResult::from_capnp(response.get()?.get_result()?)?;
    ensure_source_asset_record_matches_saved(&record, saved)?;
    Ok(record)
}

// Cannot be made `Send`: this future holds capnp-rpc `Client`
// capabilities, which are `!Send` by construction, and is only ever
// awaited on the session-supervisor transport's `LocalSet`.
#[allow(clippy::future_not_send)]
async fn force_reprocess_current_saved_source(
    client: &asset_capnp::asset_processor::Client,
    write_capability: Capability,
    manifest: &SessionManifest,
    source_root: &ProjectAssetsRootContext,
    saved: &az_proto_project::SavedDocument,
) -> Result<az_proto_asset::ForceReprocessAssetResult, Error> {
    let request_dto = ForceReprocessAssetRequest {
        capability: write_capability,
        session_id: manifest.id.0.to_string(),
        source_root: source_root.portable_key.clone(),
        source_path: saved.source_path.clone(),
    };

    let mut request = client.force_reprocess_asset_request();
    (request_dto).to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let result = ForceReprocessAssetResult::from_capnp(response.get()?.get_result()?)?;
    ensure_source_asset_record_matches_saved(&result.record, saved)?;
    Ok(result)
}

fn ensure_saved_document_matches_request(
    saved: &az_proto_project::SavedDocument,
    requested_document_id: &str,
) -> Result<(), Error> {
    if saved.document_id.as_str() != requested_document_id {
        return Err(Error::failed(format!(
            "project-host saved document `{}` but session requested `{requested_document_id}`",
            saved.document_id.as_str()
        )));
    }
    if saved.source_path != requested_document_id {
        return Err(Error::failed(format!(
            "project-host saved source path `{}` but session requested document `{requested_document_id}`",
            saved.source_path
        )));
    }
    if saved.schema_type.trim().is_empty()
        || saved.content_hash.len() != 32
        || saved.byte_length == 0
    {
        return Err(Error::failed(format!(
            "project-host saved document `{requested_document_id}` returned invalid source metadata"
        )));
    }
    Ok(())
}

fn ensure_source_asset_record_matches_saved(
    record: &az_proto_asset::SourceAssetRecordResult,
    saved: &az_proto_project::SavedDocument,
) -> Result<(), Error> {
    if record.asset_guid == Uuid::nil() {
        return Err(Error::failed(format!(
            "asset-processor record for saved document `{}` returned a nil asset guid",
            saved.document_id.as_str()
        )));
    }

    let entry = &record.entry;
    if entry.entry_id <= 0
        || entry.workspace_id <= 0
        || entry.asset_guid == Uuid::nil()
        || entry.root_id <= 0
    {
        return Err(Error::failed(format!(
            "asset-processor record for saved document `{}` did not carry DB-owned ids",
            saved.document_id.as_str()
        )));
    }

    if entry.source_path != saved.source_path
        || entry.schema_type.as_deref() != Some(saved.schema_type.as_str())
    {
        return Err(Error::failed(format!(
            "asset-processor record did not echo saved document `{}` source identity",
            saved.document_id.as_str()
        )));
    }

    let expected_content_hash = hex_lower_bytes(&saved.content_hash);
    if entry.content_hash != expected_content_hash {
        return Err(Error::failed(format!(
            "asset-processor record hash `{}` did not match saved document `{}` hash",
            entry.content_hash,
            saved.document_id.as_str()
        )));
    }

    if matches!(
        entry.diff,
        WorkspaceEntryDiff::Deleted | WorkspaceEntryDiff::Conflicted
    ) || entry.diagnostics_count < 0
    {
        return Err(Error::failed(format!(
            "asset-processor record for saved document `{}` returned invalid status metadata",
            saved.document_id.as_str()
        )));
    }

    Ok(())
}

async fn connect_project_host(
    descriptor: &ServiceDescriptor,
) -> Result<project_capnp::project_host::Client, Error> {
    az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint)
        .await
        .map_err(|error| Error::failed(format!("connect project-host failed: {error}")))
}

async fn connect_asset_processor(
    descriptor: &ServiceDescriptor,
) -> Result<asset_capnp::asset_processor::Client, Error> {
    az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint)
        .await
        .map_err(|error| Error::failed(format!("connect asset-processor failed: {error}")))
}

fn descriptor_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    service: &ServiceId,
    role: ServiceRole,
    audience: &str,
    permission: &str,
) -> Result<Capability, Error> {
    descriptor
        .validate_brokered_capability_templates()
        .map_err(|error| {
            Error::failed(format!(
                "service `{}`/`{}` has invalid brokered capability templates: {error}",
                descriptor.id.namespace, descriptor.id.name
            ))
        })?;

    descriptor
        .capabilities
        .iter()
        .find(|capability| {
            capability.service == *service
                && capability.matches_brokered_template_request(
                    role,
                    audience,
                    &[permission],
                    None,
                )
        })
        .cloned()
        .ok_or_else(|| {
            Error::failed(format!(
                "service `{}`/`{}` does not grant project-scoped capability `{permission}` to `{}`/`{}` with role `{role:?}` for audience `{audience}` while serving session `{}`",
                descriptor.id.namespace,
                descriptor.id.name,
                service.namespace,
                service.name,
                manifest.id.0,
            ))
        })
}

fn required_service_descriptor(
    manifest: &SessionManifest,
    id: &ServiceId,
    role: ServiceRole,
) -> Result<ServiceDescriptor, Error> {
    manifest.service_descriptor(id, role).ok_or_else(|| {
        Error::failed(format!(
            "session `{}` is missing service `{}`/`{}` with role `{role:?}`",
            manifest.slug, id.namespace, id.name
        ))
    })
}

#[derive(Clone, Debug)]
struct ProjectAssetsRootContext {
    workspace_root_id: i64,
    owner_id: String,
    portable_key: String,
}

// Cannot be made `Send`: this future holds capnp-rpc `Client`
// capabilities, which are `!Send` by construction, and is only ever
// awaited on the session-supervisor transport's `LocalSet`.
#[allow(clippy::future_not_send)]
async fn project_assets_root_context(
    client: &asset_capnp::asset_processor::Client,
    capability: Capability,
    manifest: &SessionManifest,
) -> Result<ProjectAssetsRootContext, Error> {
    let mut request = client.workspace_snapshot_request();
    (WorkspaceSnapshotRequest {
        capability,
        root_scope: AssetRootScope::All,
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let result = WorkspaceSnapshotResult::from_capnp(response.get()?.get_result()?)?;
    let snapshot = result.snapshot.ok_or_else(|| {
        Error::failed(format!(
            "asset registry has no workspace snapshot for session `{}`",
            manifest.id,
        ))
    })?;
    validate_project_asset_snapshot_matches_manifest(&snapshot, manifest)?;
    let portable_key = project_assets_portable_key(&manifest.project_id);
    validate_project_asset_roots_for_manifest(&snapshot, manifest, &portable_key)?;
    let source_root = snapshot
        .roots
        .into_iter()
        .find(|root| root.portable_key == portable_key)
        .ok_or_else(|| {
            Error::failed(format!(
                "asset registry workspace snapshot for session `{}` has no DB-owned project assets root `{portable_key}`",
                manifest.slug
            ))
        })?;
    project_assets_root_context_from_row(
        source_root,
        &portable_key,
        snapshot.workspace_id,
        &manifest.workspace_root,
        &manifest.project_id,
    )
}

fn validate_project_asset_snapshot_matches_manifest(
    snapshot: &WorkspaceSnapshot,
    manifest: &SessionManifest,
) -> Result<(), Error> {
    if snapshot.project_id != manifest.project_id {
        return Err(Error::failed(format!(
            "asset registry workspace snapshot project `{}` does not match session project `{}`",
            snapshot.project_id, manifest.project_id
        )));
    }
    let expected_workspace_root = manifest.workspace_root.to_string_lossy();
    if snapshot.workspace_root != expected_workspace_root {
        return Err(Error::failed(format!(
            "asset registry workspace snapshot root `{}` does not match the workspace referenced by session `{}`",
            snapshot.workspace_root, expected_workspace_root
        )));
    }
    Ok(())
}

fn validate_project_asset_roots_for_manifest(
    snapshot: &WorkspaceSnapshot,
    manifest: &SessionManifest,
    project_assets_key: &str,
) -> Result<(), Error> {
    if snapshot.roots.is_empty() {
        return Err(Error::failed(format!(
            "asset registry workspace snapshot `{}` has no DB-owned source roots",
            snapshot.workspace_id
        )));
    }

    let mut seen_source_root_ids = BTreeSet::new();
    let mut seen_portable_keys = BTreeSet::new();
    for root in &snapshot.roots {
        validate_workspace_asset_source_root(snapshot, root)?;
        if !seen_source_root_ids.insert(root.workspace_root_id) {
            return Err(Error::failed(format!(
                "asset registry workspace snapshot `{}` returned duplicate source root id {}",
                snapshot.workspace_id, root.workspace_root_id
            )));
        }
        if !seen_portable_keys.insert(root.portable_key.as_str()) {
            return Err(Error::failed(format!(
                "asset registry workspace snapshot `{}` returned duplicate source root key `{}`",
                snapshot.workspace_id, root.portable_key
            )));
        }
    }

    let project_assets = snapshot
        .roots
        .iter()
        .find(|root| root.portable_key == project_assets_key)
        .ok_or_else(|| {
            Error::failed(format!(
            "asset registry workspace snapshot for session `{}` has no DB-owned project assets root `{project_assets_key}`",
                manifest.slug
            ))
        })?;
    validate_project_assets_root(project_assets, project_assets_key, manifest)?;
    Ok(())
}

fn validate_workspace_asset_source_root(
    snapshot: &WorkspaceSnapshot,
    source_root: &WorkspaceRoot,
) -> Result<(), Error> {
    if source_root.workspace_id != snapshot.workspace_id {
        return Err(Error::failed(format!(
            "asset registry source root `{}` belongs to workspace snapshot {}, expected {}",
            source_root.portable_key, source_root.workspace_id, snapshot.workspace_id
        )));
    }
    if source_root.workspace_root_id <= 0 {
        return Err(Error::failed(format!(
            "asset registry source root `{}` has invalid id {}",
            source_root.portable_key, source_root.workspace_root_id
        )));
    }
    if source_root.root_id <= 0 {
        return Err(Error::failed(format!(
            "asset registry source root `{}` has invalid scan folder id {}",
            source_root.portable_key, source_root.root_id
        )));
    }
    if source_root.owner_id.trim().is_empty() {
        return Err(Error::failed(format!(
            "asset registry source root `{}` has an empty owner id",
            source_root.portable_key
        )));
    }
    if source_root.source_root.trim().is_empty() {
        return Err(Error::failed(format!(
            "asset registry source root `{}` has an empty path",
            source_root.portable_key
        )));
    }
    if source_root.portable_key.trim().is_empty() {
        return Err(Error::failed(
            "asset registry source root has an empty portable key".to_string(),
        ));
    }
    Ok(())
}

fn validate_project_assets_root(
    source_root: &WorkspaceRoot,
    portable_key: &str,
    manifest: &SessionManifest,
) -> Result<(), Error> {
    if source_root.owner_id != manifest.project_id {
        return Err(Error::failed(format!(
            "asset registry source root `{portable_key}` owner `{}` does not match session project `{}`",
            source_root.owner_id, manifest.project_id
        )));
    }
    let source_root_path = PathBuf::from(&source_root.source_root);
    if source_root_path == manifest.workspace_root
        || !source_root_path.starts_with(&manifest.workspace_root)
    {
        return Err(Error::failed(format!(
            "asset registry source root `{portable_key}` path `{}` must be inside the workspace referenced by the session `{}`",
            source_root_path.display(),
            manifest.workspace_root.display()
        )));
    }
    if !source_root.is_root {
        return Err(Error::failed(format!(
            "asset registry source root `{portable_key}` must be marked as a root source"
        )));
    }
    if !source_root.output_prefix.is_empty() {
        return Err(Error::failed(format!(
            "asset registry source root `{portable_key}` must use an empty output prefix"
        )));
    }
    Ok(())
}

fn project_assets_root_context_from_row(
    source_root: WorkspaceRoot,
    portable_key: &str,
    workspace_id: i64,
    expected_workspace_root: &Path,
    expected_owner_id: &str,
) -> Result<ProjectAssetsRootContext, Error> {
    if source_root.portable_key != portable_key {
        return Err(Error::failed(format!(
            "asset registry source root `{}` does not match expected project assets key `{portable_key}`",
            source_root.portable_key
        )));
    }
    if source_root.workspace_id != workspace_id {
        return Err(Error::failed(format!(
            "asset registry source root `{portable_key}` belongs to workspace snapshot {}, expected {workspace_id}",
            source_root.workspace_id
        )));
    }
    if source_root.source_root.trim().is_empty() {
        return Err(Error::failed(format!(
            "asset registry source root `{portable_key}` has an empty path"
        )));
    }
    if source_root.owner_id.trim().is_empty() {
        return Err(Error::failed(format!(
            "asset registry source root `{portable_key}` has an empty owner id"
        )));
    }
    if source_root.owner_id != expected_owner_id {
        return Err(Error::failed(format!(
            "asset registry source root `{portable_key}` owner `{}` does not match session project `{expected_owner_id}`",
            source_root.owner_id
        )));
    }
    let source_root_path = PathBuf::from(&source_root.source_root);
    if source_root_path == expected_workspace_root
        || !source_root_path.starts_with(expected_workspace_root)
    {
        return Err(Error::failed(format!(
            "asset registry source root `{portable_key}` path `{}` must be inside the workspace referenced by the session `{}`",
            source_root_path.display(),
            expected_workspace_root.display()
        )));
    }
    if source_root.workspace_root_id <= 0 {
        return Err(Error::failed(format!(
            "asset registry source root `{portable_key}` has invalid id {}",
            source_root.workspace_root_id
        )));
    }
    if source_root.root_id <= 0 {
        return Err(Error::failed(format!(
            "asset registry source root `{portable_key}` has invalid scan folder id {}",
            source_root.root_id
        )));
    }
    Ok(ProjectAssetsRootContext {
        workspace_root_id: source_root.workspace_root_id,
        owner_id: source_root.owner_id,
        portable_key: source_root.portable_key,
    })
}

fn project_assets_portable_key(project_id: &str) -> String {
    format!("project:{project_id}:assets")
}

fn hex_lower_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

fn current_unix_ms_i64() -> Result<i64, Error> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::failed(format!("system clock is before unix epoch: {error}")))?;
    let millis = elapsed.as_millis();
    i64::try_from(millis)
        .map_err(|_| Error::failed(format!("unix milliseconds `{millis}` exceed i64 range")))
}

#[must_use]
pub fn session_manifest_to_proto(manifest: &SessionManifest) -> az_proto_session::SessionManifest {
    let mut dto = az_proto_session::SessionManifest::new(
        manifest.id.0,
        manifest.project_id.clone(),
        manifest.slug.clone(),
        path_to_protocol_string(&manifest.project_root),
        path_to_protocol_string(&manifest.workspace_root),
        path_to_protocol_string(&manifest.run_dir),
        session_state_to_proto(manifest.state),
    );
    dto.services = manifest
        .services
        .iter()
        .map(ServiceRecord::to_descriptor)
        .collect();
    dto.processes = manifest
        .processes
        .iter()
        .map(service_process_to_proto)
        .collect();
    dto
}

fn service_process_to_proto(
    process: &ServiceProcessRecord,
) -> az_proto_session::ServiceProcessRecord {
    az_proto_session::ServiceProcessRecord {
        owner_id: process.owner_id.clone(),
        owner_root: process.owner_root.to_string_lossy().into_owned(),
        service_name: process.service_name.clone(),
        role: process.role.to_proto(),
        run: process.run,
        previous_run: process.previous_run,
        endpoint: az_proto_session::core::Endpoint::new(
            process.endpoint_kind.to_proto(),
            process.endpoint_address.clone(),
        ),
        program: process.program.clone(),
        program_artifact: process.program_artifact.map(|artifact| {
            az_proto_session::ServiceProgramArtifact {
                byte_length: artifact.byte_length,
                modified_unix_ns: artifact.modified_unix_ns,
                file_system_id: artifact.file_system_id,
                file_id: artifact.file_id,
            }
        }),
        cwd: path_to_protocol_string(&process.cwd),
        args: process.args.clone(),
        stdout_log: path_to_protocol_string(&process.stdout_log),
        stderr_log: path_to_protocol_string(&process.stderr_log),
        structured_log: path_to_protocol_string(&process.structured_log),
        state: service_process_state_to_proto(process.state),
        pid: process.pid,
        process_start_time: process.process_start_time,
        exit_code: process.exit_code,
        failure: process.failure.clone(),
        planned_unix_ms: saturating_u64(process.planned_unix_ms),
        updated_unix_ms: saturating_u64(process.updated_unix_ms),
        started_unix_ms: process.started_unix_ms.map(saturating_u64),
        exited_unix_ms: process.exited_unix_ms.map(saturating_u64),
    }
}

const fn service_process_state_to_proto(
    state: ServiceProcessState,
) -> az_proto_session::ServiceProcessState {
    match state {
        ServiceProcessState::Planned => az_proto_session::ServiceProcessState::Planned,
        ServiceProcessState::Starting => az_proto_session::ServiceProcessState::Starting,
        ServiceProcessState::Running => az_proto_session::ServiceProcessState::Running,
        ServiceProcessState::Exited => az_proto_session::ServiceProcessState::Exited,
        ServiceProcessState::Failed => az_proto_session::ServiceProcessState::Failed,
    }
}

fn saturating_u64(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[must_use]
pub fn session_workspace_status_to_proto(
    status: &SessionStatus,
) -> az_proto_session::SessionWorkspaceStatus {
    az_proto_session::SessionWorkspaceStatus {
        manifest: session_manifest_to_proto(&status.manifest),
        failure_reason: status.failure_reason.clone(),
    }
}

const fn session_state_to_proto(state: SessionState) -> az_proto_session::SessionState {
    match state {
        SessionState::Preparing => az_proto_session::SessionState::Preparing,
        SessionState::Active => az_proto_session::SessionState::Active,
        SessionState::FailedPreserved => az_proto_session::SessionState::FailedPreserved,
        SessionState::Removed => az_proto_session::SessionState::Removed,
    }
}

fn path_to_protocol_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn validate_session_capability(
    capability: &Capability,
    required_permission: &str,
) -> Result<(), Error> {
    capability.validate_lifetime().map_err(|error| {
        Error::failed(format!("invalid session-supervisor capability: {error}"))
    })?;

    if capability.audience != SESSION_SUPERVISOR_AUDIENCE {
        return Err(Error::failed(format!(
            "invalid session-supervisor capability: expected audience `{SESSION_SUPERVISOR_AUDIENCE}` but got `{}`",
            capability.audience
        )));
    }
    if !matches!(
        capability.role,
        ServiceRole::Editor | ServiceRole::Daemon | ServiceRole::SessionSupervisor
    ) {
        return Err(Error::failed(format!(
            "invalid session-supervisor capability: unsupported role {:?}",
            capability.role
        )));
    }
    let has_required = capability
        .permissions
        .iter()
        .any(|permission| permission == required_permission);
    let has_manage = capability
        .permissions
        .iter()
        .any(|permission| permission == SESSION_MANAGE_PERMISSION);
    if !(has_required || (required_permission == SESSION_READ_PERMISSION && has_manage)) {
        return Err(Error::failed(format!(
            "invalid session-supervisor capability: missing required permission `{required_permission}`"
        )));
    }
    Ok(())
}

fn validate_session_capability_for_manifest(
    capability: &Capability,
    required_permission: &str,
    manifest: &SessionManifest,
) -> Result<(), Error> {
    validate_session_capability(capability, required_permission)?;
    if capability.session != Some(manifest.id.0) {
        let actual = capability
            .session
            .map_or_else(|| "none".to_string(), |session| session.to_string());
        return Err(Error::failed(format!(
            "invalid session-supervisor capability: expected session `{}` for `{}` but got `{actual}`",
            manifest.id, manifest.slug
        )));
    }
    Ok(())
}

fn session_error_to_capnp(error: &SessionError) -> Error {
    Error::failed(format!("session supervisor failed: {error}"))
}

#[cfg(test)]
mod tests {
    use az_architecture_guard::{
        production_source_without_cfg_test_modules, public_functions_are_cfg_test_or_test_support,
    };

    use super::*;

    #[test]
    fn attached_project_service_capabilities_remain_project_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = SessionManifest::new(
            crate::SessionId::new(),
            "local.session_rpc".to_string(),
            "build".to_string(),
            temp.path().join("project"),
            temp.path().join("workspace"),
            temp.path().join("run"),
            0,
        );
        let descriptor = az_service_catalog::asset_processor_service_descriptor(
            Uuid::now_v7(),
            az_proto_core::Endpoint::in_process("asset-processor:project"),
        );

        let capability = descriptor_capability(
            &manifest,
            &descriptor,
            &ServiceId::new(
                SESSION_SUPERVISOR_NAMESPACE,
                SESSION_SUPERVISOR_SERVICE_NAME,
            ),
            ServiceRole::SessionSupervisor,
            ASSET_PROCESSOR_AUDIENCE,
            ASSET_READ_PERMISSION,
        )
        .unwrap();

        assert_eq!(capability.session, None);
    }

    #[test]
    fn session_status_source_control_scan_runs_off_the_rpc_executor() {
        let source = include_str!("rpc.rs");
        let status_start = source
            .find("    async fn status(")
            .expect("find SessionSupervisor.status");
        let status_end = source[status_start..]
            .find("\n    async fn preserve_failed(")
            .map(|offset| status_start + offset)
            .expect("find method after SessionSupervisor.status");
        let status_source = &source[status_start..status_end];

        assert!(
            status_source.contains("tokio::task::spawn_blocking("),
            "Lore status --scan must not block the session supervisor RPC executor"
        );
        assert!(
            status_source
                .find("validate_capability_for_manifest(")
                .unwrap()
                < status_source.find("tokio::task::spawn_blocking(").unwrap(),
            "capability validation must happen before dispatching the expensive status scan"
        );
    }

    #[test]
    fn in_process_session_supervisor_rpc_constructors_are_not_default_public_api() {
        let source = include_str!("rpc.rs");
        let production_source = production_source_without_cfg_test_modules(source);

        for signature in [
            "pub(crate) fn new_with_command_sender(",
            "pub(crate) const fn with_descriptor_grants(",
            "pub(crate) fn into_client(self)",
        ] {
            assert!(
                production_source.contains(signature),
                "`{signature}` must stay crate-private; production callers should start a session-supervisor RPC server instead of embedding it"
            );
        }

        for signature in [
            "pub fn new(",
            "pub fn test_new(",
            "pub fn with_manager(",
            "pub fn with_manager_and_command_sender(",
            "pub fn test_with_manager(",
            "pub fn client_from_rc(",
        ] {
            assert!(
                public_functions_are_cfg_test_or_test_support(&production_source, signature),
                "`{signature}` must require the explicit test-support feature; in-process session-supervisor clients are for tests/harnesses only"
            );
        }
    }
}
