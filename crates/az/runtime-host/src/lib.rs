//! Runtime-host service adapter.
//!
//! This is the boundary around project-built runtime projection processes. The
//! host owns launch snapshots, runtime state, and viewport-frame handles;
//! project/gem runtime behavior arrives through the [`Composer`] the generated
//! service process transfers at startup.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
#[cfg(any(test, feature = "test-support"))]
use std::rc::Rc;
use std::time::{Duration, Instant};

use az_gem_contract::{
    ComposeError, Composer, GemTargetRole, ProcessComposition, ProcessCompositionCleanupError,
    Registries, RegistryLease, RegistryLeaseError,
};
use az_proto_core::{
    Capability, CapabilityGrantSet, ServiceHealth, ServiceId, ServiceRole, SideChannelHandle,
    SideChannelKind, require_side_channel_capability, validated_mmap_file_path,
    validated_staging_file_path,
};
use az_proto_runtime::{
    LaunchRuntimeRequest, RUNTIME_CONTROL_PERMISSION, RUNTIME_HOST_AUDIENCE,
    RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME, RUNTIME_READ_PERMISSION,
    RuntimeProjectionCatalogRequest, RuntimeProjectionCatalogResult, RuntimeProjectionDescriptor,
    RuntimeResolvedGem, RuntimeSnapshotSideChannelError, RuntimeStatusRequest, RuntimeStatusResult,
    RuntimeViewportRequest, RuntimeViewportResult, StopRuntimeRequest,
    load_runtime_launch_snapshot_side_channel, runtime_capnp,
};
use capnp::Error;
use thiserror::Error;
use tracing::{info, instrument};
use uuid::Uuid;

mod projection;
mod transport;

pub use az_proto_runtime::{
    RuntimeLaunchSnapshot, RuntimeRole, RuntimeState, RuntimeStatus, RuntimeViewportFrame,
    ViewportPixelFormat,
};
pub use projection::*;
pub use transport::*;

const PROJECT_HOST_SERVICE_NAMESPACE: &str = "azoth";
const PROJECT_HOST_SERVICE_NAME: &str = "project-host";

/// Runtime-host's one composition for its complete process lifetime.
///
/// The RPC adapter receives only its registries; it never becomes a second
/// registration authority or learns how the generated process composed them.
struct RuntimeHostComposition {
    process: ProcessComposition,
}

impl fmt::Debug for RuntimeHostComposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeHostComposition")
            .field("report", self.process.report())
            .finish_non_exhaustive()
    }
}

impl RuntimeHostComposition {
    /// Seal the generated process composition as a runtime-host composition.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeHostCompositionError::WrongRole`] before finalization,
    /// the composer's exact validation error, or `NotReady` when a composed
    /// contribution cannot admit service publication.
    fn from_composer(composer: Composer) -> Result<Self, RuntimeHostCompositionError> {
        let actual = composer.host().role();
        if actual != GemTargetRole::RuntimeHost {
            return Err(RuntimeHostCompositionError::WrongRole { actual });
        }
        let process = ProcessComposition::new(composer)?;
        if !process.is_ready() {
            return Err(RuntimeHostCompositionError::NotReady);
        }
        Ok(Self { process })
    }

    // Only the composition tests read the registries back off the composition;
    // the host itself reaches them through its own accessor.
    #[cfg(test)]
    #[must_use]
    pub fn registries(&self) -> &Registries {
        self.process.registries()
    }

    fn registry_lease(&self) -> Result<RegistryLease, RegistryLeaseError> {
        self.process.registry_lease()
    }

    fn begin_shutdown(&mut self) -> Result<(), ProcessCompositionCleanupError> {
        self.process.begin_shutdown()
    }

    fn cleanup(&mut self) -> Result<(), ProcessCompositionCleanupError> {
        self.process.cleanup()
    }
}

/// Runtime-host bootstrap refusal.
#[derive(Debug, Error)]
pub enum RuntimeHostCompositionError {
    #[error("runtime-host received a composition for `{actual}`")]
    WrongRole { actual: GemTargetRole },
    #[error(transparent)]
    Compose(#[from] ComposeError),
    #[error(transparent)]
    RegistryLease(#[from] RegistryLeaseError),
    #[error(transparent)]
    Cleanup(#[from] ProcessCompositionCleanupError),
    #[error("runtime-host composition is not ready")]
    NotReady,
}

/// The registry lease a runtime host serves.
///
/// Static registries remain available only to linked test hosts. Production
/// startup owns the composition inside the host that dispatches from it.
enum RuntimeHostRegistryLease {
    #[cfg(any(test, feature = "test-support"))]
    Static(&'static Registries),
    Composed(RegistryLease),
}

impl RuntimeHostRegistryLease {
    fn registries(&self) -> &Registries {
        match self {
            #[cfg(any(test, feature = "test-support"))]
            Self::Static(registries) => registries,
            Self::Composed(composition) => composition.registries(),
        }
    }
}

struct RuntimeProjectionInstance {
    name: &'static str,
    projection: Box<dyn RuntimeProjection>,
}

impl fmt::Debug for RuntimeProjectionInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeProjectionInstance")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

struct RuntimeInstance {
    status: RuntimeStatus,
    snapshot: RuntimeLaunchSnapshot,
    viewport_frame: Option<RuntimeViewportFrame>,
    projections: Vec<RuntimeProjectionInstance>,
}

impl fmt::Debug for RuntimeInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeInstance")
            .field("status", &self.status)
            .field("snapshot", &self.snapshot)
            .field("viewport_frame", &self.viewport_frame)
            .field("projections", &self.projections)
            .finish()
    }
}

pub struct RuntimeHost {
    service_session_id: String,
    capability_grants: CapabilityGrantSet,
    side_channel_root: PathBuf,
    instances: RefCell<BTreeMap<String, RuntimeInstance>>,
    registries: RuntimeHostRegistryLease,
}

impl fmt::Debug for RuntimeHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeHost")
            .field("service_session_id", &self.service_session_id)
            .field(
                "capability_grant_count",
                &self.capability_grants.grants().len(),
            )
            .field("side_channel_root", &self.side_channel_root)
            .field("instance_count", &self.instances.borrow().len())
            // `instances` and `registries` are summarized by count or omitted
            // outright: the registry lease has no useful debug shape and the
            // instance map would dump every live runtime.
            .finish_non_exhaustive()
    }
}

impl RuntimeHost {
    /// A host serving one composition's projections.
    ///
    /// This linked construction path remains for static engine/test
    /// compositions. Production startup owns a [`Composer`] instead.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        capability_grants: CapabilityGrantSet,
        side_channel_root: impl Into<PathBuf>,
        registries: &'static Registries,
    ) -> Self {
        Self::with_registry_lease(
            session_id,
            capability_grants,
            side_channel_root,
            RuntimeHostRegistryLease::Static(registries),
        )
    }

    /// Bootstrap the one generated runtime-host composition.
    ///
    /// # Errors
    ///
    /// Returns the exact compose/readiness refusal. A composer for another role
    /// is rejected before finalization.
    fn from_composer(
        session_id: impl Into<String>,
        capability_grants: CapabilityGrantSet,
        side_channel_root: impl Into<PathBuf>,
        composer: Composer,
    ) -> Result<(Self, RuntimeHostComposition), RuntimeHostCompositionError> {
        let composition = RuntimeHostComposition::from_composer(composer)?;
        let registries = composition.registry_lease()?;
        let host = Self::with_registry_lease(
            session_id,
            capability_grants,
            side_channel_root,
            RuntimeHostRegistryLease::Composed(registries),
        );
        Ok((host, composition))
    }

    fn with_registry_lease(
        session_id: impl Into<String>,
        capability_grants: CapabilityGrantSet,
        side_channel_root: impl Into<PathBuf>,
        registries: RuntimeHostRegistryLease,
    ) -> Self {
        let side_channel_root = side_channel_root.into();
        debug_assert!(
            validate_runtime_side_channel_root(&side_channel_root).is_ok(),
            "runtime-host side-channel root must be an absolute normal path"
        );
        Self {
            service_session_id: session_id.into(),
            capability_grants,
            side_channel_root,
            instances: RefCell::new(BTreeMap::new()),
            registries,
        }
    }

    /// The composed registry set this host serves.
    #[must_use]
    pub fn registries(&self) -> &Registries {
        self.registries.registries()
    }

    #[must_use]
    pub fn service_session_id(&self) -> &str {
        &self.service_session_id
    }

    #[must_use]
    pub const fn capability_grants(&self) -> &CapabilityGrantSet {
        &self.capability_grants
    }

    #[must_use]
    pub fn side_channel_root(&self) -> &Path {
        &self.side_channel_root
    }

    /// Launch, or re-attach to, the runtime instance `request` names.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeHostError::InvalidCapability`] if `request.capability`
    /// does not grant control authority for this service session,
    /// [`RuntimeHostError::InvalidRuntimeId`] if the runtime id is blank,
    /// [`RuntimeHostError::RoleMismatch`] if the requested role is not the role
    /// this host serves, and the launch-snapshot refusals
    /// ([`RuntimeHostError::MissingProjectId`],
    /// [`RuntimeHostError::MissingSessionSlug`],
    /// [`RuntimeHostError::MissingSessionId`],
    /// [`RuntimeHostError::MissingWorkspacePath`],
    /// [`RuntimeHostError::SnapshotSessionMismatch`],
    /// [`RuntimeHostError::SnapshotServiceSessionMismatch`],
    /// [`RuntimeHostError::UnboundSnapshotCapability`],
    /// [`RuntimeHostError::InvalidLaunchSnapshot`], or
    /// [`RuntimeHostError::Snapshot`]) when the attached snapshot does not
    /// describe this session's workspace.
    #[instrument(skip(self, request), fields(runtime_id = %request.runtime_id, role = ?request.role))]
    pub fn launch(
        &self,
        request: &LaunchRuntimeRequest,
    ) -> Result<RuntimeStatus, RuntimeHostError> {
        validate_control_capability(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )?;
        if request.runtime_id.trim().is_empty() {
            return Err(RuntimeHostError::InvalidRuntimeId);
        }
        validate_snapshot_handle_capability(
            &request.snapshot,
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )?;

        let snapshot = load_runtime_launch_snapshot_side_channel(&request.snapshot)?;
        validate_snapshot(
            &snapshot,
            request.role,
            request.capability.session,
            self.service_session_id(),
        )?;
        let mut status = RuntimeStatus {
            runtime_id: request.runtime_id.clone(),
            role: request.role,
            state: RuntimeState::Running,
            project_id: snapshot.project_id.clone(),
            session_slug: snapshot.session_slug.clone(),
            authored_revision: snapshot.authored_revision,
            diagnostic: String::new(),
        };
        let (projections, viewport_frame) =
            self.launch_runtime_projections(request, &snapshot, &mut status);
        self.instances.borrow_mut().insert(
            request.runtime_id.clone(),
            RuntimeInstance {
                status: status.clone(),
                snapshot,
                viewport_frame,
                projections,
            },
        );
        Ok(status)
    }

    /// Launch every projection registered for this role and launch profile,
    /// folding their updates into `status`.
    ///
    /// Returns the live projection instances and whichever viewport frame the
    /// first claiming projection published. A role with no registered
    /// projections is a metadata-only runtime, not a failure.
    fn launch_runtime_projections(
        &self,
        request: &LaunchRuntimeRequest,
        snapshot: &RuntimeLaunchSnapshot,
        status: &mut RuntimeStatus,
    ) -> (Vec<RuntimeProjectionInstance>, Option<RuntimeViewportFrame>) {
        let mut viewport_frame = None;
        let registrations =
            matching_runtime_projections(self.registries(), request.role, &snapshot.launch_profile);
        let projection_count = registrations.len();
        let mut projections = Vec::with_capacity(projection_count);
        if registrations.is_empty() {
            info!(runtime_id = %request.runtime_id, "runtime-host launched metadata runtime projection");
            return (projections, viewport_frame);
        }

        let mut diagnostics = Vec::new();
        let mut phase_viewport_claimed = false;
        let mut phase_failed = false;
        for registration in registrations {
            let projection_name = registration.name();
            let mut projection = registration.projection();
            let context = RuntimeProjectionLaunchContext {
                runtime_id: &request.runtime_id,
                role: request.role,
                side_channel_root: self.side_channel_root(),
                snapshot,
            };
            match projection.launch(&context) {
                Ok(update) => match apply_projection_update(
                    status,
                    &mut viewport_frame,
                    &mut phase_viewport_claimed,
                    self.side_channel_root(),
                    update,
                ) {
                    Ok((state, diagnostic)) => {
                        phase_failed |= state == RuntimeState::Failed;
                        push_projection_diagnostic(
                            &mut diagnostics,
                            projection_name,
                            projection_count,
                            diagnostic,
                        );
                    }
                    Err(error) => {
                        phase_failed = true;
                        mark_projection_invalid(status, projection_name, "launch", &error);
                        push_projection_diagnostic(
                            &mut diagnostics,
                            projection_name,
                            projection_count,
                            status.diagnostic.clone(),
                        );
                    }
                },
                Err(error) => {
                    phase_failed = true;
                    mark_projection_failed(status, projection_name, "launch", &error);
                    push_projection_diagnostic(
                        &mut diagnostics,
                        projection_name,
                        projection_count,
                        status.diagnostic.clone(),
                    );
                }
            }
            info!(
                runtime_id = %request.runtime_id,
                projection = projection_name,
                launch_profile = %snapshot.launch_profile,
                "runtime-host launched registered runtime projection"
            );
            projections.push(RuntimeProjectionInstance {
                name: projection_name,
                projection,
            });
        }
        if phase_failed {
            status.state = RuntimeState::Failed;
        }
        finish_projection_diagnostics(status, &diagnostics);
        (projections, viewport_frame)
    }

    /// Report the current status of one runtime instance, refreshing its
    /// projection state first. An unknown runtime is `Ok` with no status.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeHostError::InvalidCapability`] if `request.capability`
    /// does not grant read authority for this service session.
    #[instrument(skip(self, request), fields(runtime_id = %request.runtime_id))]
    pub fn status(
        &self,
        request: &RuntimeStatusRequest,
    ) -> Result<RuntimeStatusResult, RuntimeHostError> {
        validate_read_capability(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )?;
        let status = self
            .instances
            .borrow_mut()
            .get_mut(&request.runtime_id)
            .map(|instance| {
                refresh_projection_status(instance, self.side_channel_root());
                instance.status.clone()
            });
        Ok(RuntimeStatusResult { status })
    }

    /// Read the most recent viewport frame published for one runtime, if any.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeHostError::InvalidCapability`] if `request.capability`
    /// does not grant read authority for this service session.
    #[instrument(skip(self, request), fields(runtime_id = %request.runtime_id))]
    pub fn viewport_frame(
        &self,
        request: &RuntimeViewportRequest,
    ) -> Result<RuntimeViewportResult, RuntimeHostError> {
        validate_read_capability(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )?;
        let frame = self
            .instances
            .borrow_mut()
            .get_mut(&request.runtime_id)
            .and_then(|instance| {
                refresh_projection_viewport_frame(instance, self.side_channel_root());
                instance.viewport_frame.clone()
            });
        Ok(RuntimeViewportResult { frame })
    }

    /// List the runtime projections this host's registries compose.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeHostError::InvalidCapability`] if `request.capability`
    /// does not grant read authority for this service session.
    #[instrument(skip(self, request))]
    pub fn projection_catalog(
        &self,
        request: &RuntimeProjectionCatalogRequest,
    ) -> Result<RuntimeProjectionCatalogResult, RuntimeHostError> {
        validate_read_capability(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )?;
        let projections = projections(self.registries())
            .into_iter()
            .map(runtime_projection_to_descriptor)
            .collect();
        Ok(RuntimeProjectionCatalogResult { projections })
    }

    /// Record a viewport frame a runtime produced on its side channel.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeHostError::InvalidViewportFrame`] if the frame's side
    /// channel does not sit under this host's side-channel root or its
    /// dimensions are not usable, or [`RuntimeHostError::UnknownRuntime`] if no
    /// instance is registered under `frame.runtime_id`.
    #[instrument(skip(self, frame), fields(runtime_id = %frame.runtime_id, width = frame.width, height = frame.height))]
    pub fn publish_viewport_frame(
        &self,
        frame: RuntimeViewportFrame,
    ) -> Result<RuntimeViewportFrame, RuntimeHostError> {
        validate_viewport_frame(&frame, self.side_channel_root())?;
        let mut instances = self.instances.borrow_mut();
        let instance = instances.get_mut(&frame.runtime_id).ok_or_else(|| {
            RuntimeHostError::UnknownRuntime {
                runtime_id: frame.runtime_id.clone(),
            }
        })?;
        instance.viewport_frame = Some(frame.clone());
        info!(format = ?frame.format, "runtime-host published viewport frame side channel");
        Ok(frame)
    }

    /// Stop one runtime instance, optionally preserving its record.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeHostError::InvalidCapability`] if `request.capability`
    /// does not grant control authority for this service session, or
    /// [`RuntimeHostError::UnknownRuntime`] if no instance is registered under
    /// `request.runtime_id`.
    #[instrument(skip(self, request), fields(runtime_id = %request.runtime_id, preserve = request.preserve))]
    pub fn stop(&self, request: &StopRuntimeRequest) -> Result<RuntimeStatus, RuntimeHostError> {
        validate_control_capability(
            &request.capability,
            self.service_session_id(),
            self.capability_grants(),
        )?;
        let mut instances = self.instances.borrow_mut();
        let instance = instances.get_mut(&request.runtime_id).ok_or_else(|| {
            RuntimeHostError::UnknownRuntime {
                runtime_id: request.runtime_id.clone(),
            }
        })?;
        if instance.projections.is_empty() {
            instance.status.state = RuntimeState::Stopped;
            if request.preserve {
                instance.status.diagnostic = "stopped and preserved".to_string();
            } else {
                instance.status.diagnostic.clear();
            }
        } else {
            stop_projection(instance, request.preserve, self.side_channel_root());
        }
        info!("runtime-host stopped runtime projection");
        Ok(instance.status.clone())
    }

    #[must_use]
    pub fn launched_snapshot(&self, runtime_id: &str) -> Option<RuntimeLaunchSnapshot> {
        self.instances
            .borrow()
            .get(runtime_id)
            .map(|instance| instance.snapshot.clone())
    }
}

#[derive(Debug, Error)]
pub enum RuntimeHostError {
    #[error("invalid runtime-host capability: {reason}")]
    InvalidCapability { reason: String },

    #[error("runtime id cannot be empty")]
    InvalidRuntimeId,

    #[error("runtime `{runtime_id}` is not known")]
    UnknownRuntime { runtime_id: String },

    #[error(
        "runtime launch snapshot role {snapshot_role:?} does not match requested role {request_role:?}"
    )]
    RoleMismatch {
        snapshot_role: RuntimeRole,
        request_role: RuntimeRole,
    },

    #[error("runtime launch snapshot project id cannot be empty")]
    MissingProjectId,

    #[error("runtime launch snapshot session slug cannot be empty")]
    MissingSessionSlug,

    #[error("runtime launch snapshot session id cannot be nil")]
    MissingSessionId,

    #[error(
        "runtime launch snapshot session {snapshot_session} does not match capability session {capability_session}"
    )]
    SnapshotSessionMismatch {
        snapshot_session: Uuid,
        capability_session: Uuid,
    },

    #[error(
        "runtime launch snapshot session {snapshot_session} does not match runtime-host session {service_session}"
    )]
    SnapshotServiceSessionMismatch {
        snapshot_session: Uuid,
        service_session: String,
    },

    #[error("runtime launch snapshot workspace path cannot be empty")]
    MissingWorkspacePath,

    #[error("invalid runtime launch snapshot: {reason}")]
    InvalidLaunchSnapshot { reason: String },

    #[error("runtime launch snapshot side-channel handle is missing project-host capability")]
    UnboundSnapshotCapability,

    #[error("invalid viewport frame side channel: {reason}")]
    InvalidViewportFrame { reason: String },

    #[error("invalid runtime-host side-channel root: {reason}")]
    InvalidSideChannelRoot { reason: String },

    #[error("invalid runtime launch snapshot side channel: {0}")]
    Snapshot(#[from] RuntimeSnapshotSideChannelError),
}

pub struct RuntimeHostRpc {
    host: RuntimeHost,
    service_run: Uuid,
    started_at: Instant,
}

impl fmt::Debug for RuntimeHostRpc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeHostRpc")
            .field("host", &self.host)
            .field("service_run", &self.service_run)
            .field("uptime_ms", &duration_millis_u64(self.started_at.elapsed()))
            .finish()
    }
}

impl RuntimeHostRpc {
    #[must_use]
    pub(crate) fn new(host: RuntimeHost) -> Self {
        Self {
            host,
            service_run: Uuid::nil(),
            started_at: Instant::now(),
        }
    }

    #[must_use]
    pub(crate) const fn with_service_run(mut self, run: Uuid) -> Self {
        self.service_run = run;
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn test_new(host: RuntimeHost) -> Self {
        Self::new(host)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub const fn host(&self) -> &RuntimeHost {
        &self.host
    }

    #[must_use]
    pub(crate) fn into_client(self) -> runtime_capnp::runtime_host::Client {
        capnp_rpc::new_client(self)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn client_from_rc(this: &Rc<Self>) -> runtime_capnp::runtime_host::Client {
        capnp_rpc::new_client_from_rc(Rc::clone(this))
    }

    #[must_use]
    fn health_snapshot(&self) -> ServiceHealth {
        ServiceHealth::ready(
            ServiceId::new(RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME),
            ServiceRole::RuntimeHost,
            self.service_run,
            az_proto_core::ProtocolVersion::CURRENT,
        )
        .with_uptime_ms(duration_millis_u64(self.started_at.elapsed()))
        .with_message("runtime-host reachable")
    }
}

impl runtime_capnp::runtime_host::Server for RuntimeHostRpc {
    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    async fn health(
        self: capnp::capability::Rc<Self>,
        _params: runtime_capnp::runtime_host::HealthParams,
        mut results: runtime_capnp::runtime_host::HealthResults,
    ) -> Result<(), Error> {
        (self.health_snapshot()).to_capnp(results.get().init_health())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    async fn launch(
        self: capnp::capability::Rc<Self>,
        params: runtime_capnp::runtime_host::LaunchParams,
        mut results: runtime_capnp::runtime_host::LaunchResults,
    ) -> Result<(), Error> {
        let request = LaunchRuntimeRequest::from_capnp(params.get()?.get_request()?)?;
        let status = self
            .host
            .launch(&request)
            .map_err(|error| runtime_host_error_to_capnp(&error))?;
        (status).to_capnp(results.get().init_status())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    async fn status(
        self: capnp::capability::Rc<Self>,
        params: runtime_capnp::runtime_host::StatusParams,
        mut results: runtime_capnp::runtime_host::StatusResults,
    ) -> Result<(), Error> {
        let request = RuntimeStatusRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .host
            .status(&request)
            .map_err(|error| runtime_host_error_to_capnp(&error))?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    async fn stop(
        self: capnp::capability::Rc<Self>,
        params: runtime_capnp::runtime_host::StopParams,
        mut results: runtime_capnp::runtime_host::StopResults,
    ) -> Result<(), Error> {
        let request = StopRuntimeRequest::from_capnp(params.get()?.get_request()?)?;
        let status = self
            .host
            .stop(&request)
            .map_err(|error| runtime_host_error_to_capnp(&error))?;
        (status).to_capnp(results.get().init_status())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    async fn viewport_frame(
        self: capnp::capability::Rc<Self>,
        params: runtime_capnp::runtime_host::ViewportFrameParams,
        mut results: runtime_capnp::runtime_host::ViewportFrameResults,
    ) -> Result<(), Error> {
        let request = RuntimeViewportRequest::from_capnp(params.get()?.get_request()?)?;
        let mut result = self
            .host
            .viewport_frame(&request)
            .map_err(|error| runtime_host_error_to_capnp(&error))?;
        bind_viewport_frame_result_capability(&mut result, &request.capability);
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    async fn projection_catalog(
        self: capnp::capability::Rc<Self>,
        params: runtime_capnp::runtime_host::ProjectionCatalogParams,
        mut results: runtime_capnp::runtime_host::ProjectionCatalogResults,
    ) -> Result<(), Error> {
        let request = RuntimeProjectionCatalogRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .host
            .projection_catalog(&request)
            .map_err(|error| runtime_host_error_to_capnp(&error))?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn matching_runtime_projections<'a>(
    registries: &'a Registries,
    role: RuntimeRole,
    launch_profile: &str,
) -> Vec<&'a RuntimeProjectionRegistration> {
    projections(registries)
        .into_iter()
        .filter(|registration| registration.matches(role, launch_profile))
        .collect()
}

fn runtime_projection_to_descriptor(
    registration: &RuntimeProjectionRegistration,
) -> RuntimeProjectionDescriptor {
    RuntimeProjectionDescriptor {
        name: registration.name().to_string(),
        priority: registration.priority(),
        roles: registration.roles().to_vec(),
        launch_profiles: registration
            .launch_profiles()
            .iter()
            .map(|profile| (*profile).to_string())
            .collect(),
    }
}

fn refresh_projection_status(instance: &mut RuntimeInstance, side_channel_root: &Path) {
    if instance.projections.is_empty() {
        return;
    }

    let status = instance.status.clone();
    let snapshot = instance.snapshot.clone();
    let projection_count = instance.projections.len();
    let mut diagnostics = Vec::new();
    let mut phase_viewport_claimed = false;
    let mut phase_failed = false;
    for projection in &mut instance.projections {
        let projection_name = projection.name;
        let context = RuntimeProjectionStatusContext {
            runtime_id: &status.runtime_id,
            status: &status,
            side_channel_root,
            snapshot: &snapshot,
        };
        match projection.projection.status(&context) {
            Ok(Some(update)) => match apply_projection_update(
                &mut instance.status,
                &mut instance.viewport_frame,
                &mut phase_viewport_claimed,
                side_channel_root,
                update,
            ) {
                Ok((state, diagnostic)) => {
                    phase_failed |= state == RuntimeState::Failed;
                    push_projection_diagnostic(
                        &mut diagnostics,
                        projection_name,
                        projection_count,
                        diagnostic,
                    );
                }
                Err(error) => {
                    phase_failed = true;
                    mark_projection_invalid(
                        &mut instance.status,
                        projection_name,
                        "status",
                        &error,
                    );
                    push_projection_diagnostic(
                        &mut diagnostics,
                        projection_name,
                        projection_count,
                        instance.status.diagnostic.clone(),
                    );
                }
            },
            Ok(None) => {}
            Err(error) => {
                phase_failed = true;
                mark_projection_failed(&mut instance.status, projection_name, "status", &error);
                push_projection_diagnostic(
                    &mut diagnostics,
                    projection_name,
                    projection_count,
                    instance.status.diagnostic.clone(),
                );
            }
        }
    }
    if phase_failed {
        instance.status.state = RuntimeState::Failed;
    }
    finish_projection_diagnostics(&mut instance.status, &diagnostics);
}

fn refresh_projection_viewport_frame(instance: &mut RuntimeInstance, side_channel_root: &Path) {
    if instance.projections.is_empty() {
        return;
    }

    let status = instance.status.clone();
    let snapshot = instance.snapshot.clone();
    let projection_count = instance.projections.len();
    let mut diagnostics = Vec::new();
    let mut phase_viewport_claimed = false;
    let mut phase_failed = false;
    for projection in &mut instance.projections {
        let projection_name = projection.name;
        let context = RuntimeProjectionViewportContext {
            runtime_id: &status.runtime_id,
            status: &status,
            side_channel_root,
            snapshot: &snapshot,
        };
        match projection.projection.viewport_frame(&context) {
            Ok(Some(frame)) => {
                if let Err(error) = set_projection_viewport_frame(
                    &instance.status.runtime_id,
                    &mut instance.viewport_frame,
                    &mut phase_viewport_claimed,
                    side_channel_root,
                    frame,
                ) {
                    phase_failed = true;
                    mark_projection_invalid(
                        &mut instance.status,
                        projection_name,
                        "viewport",
                        &error,
                    );
                    push_projection_diagnostic(
                        &mut diagnostics,
                        projection_name,
                        projection_count,
                        instance.status.diagnostic.clone(),
                    );
                }
            }
            Ok(None) => {}
            Err(error) => {
                phase_failed = true;
                mark_projection_failed(&mut instance.status, projection_name, "viewport", &error);
                push_projection_diagnostic(
                    &mut diagnostics,
                    projection_name,
                    projection_count,
                    instance.status.diagnostic.clone(),
                );
            }
        }
    }
    if phase_failed {
        instance.status.state = RuntimeState::Failed;
    }
    finish_projection_diagnostics(&mut instance.status, &diagnostics);
}

fn stop_projection(instance: &mut RuntimeInstance, preserve: bool, side_channel_root: &Path) {
    if instance.projections.is_empty() {
        return;
    }

    let status = instance.status.clone();
    let snapshot = instance.snapshot.clone();
    let projection_count = instance.projections.len();
    let mut diagnostics = Vec::new();
    let mut phase_viewport_claimed = false;
    let mut phase_failed = false;
    for projection in &mut instance.projections {
        let projection_name = projection.name;
        let context = RuntimeProjectionStopContext {
            runtime_id: &status.runtime_id,
            status: &status,
            side_channel_root,
            snapshot: &snapshot,
            preserve,
        };
        match projection.projection.stop(&context) {
            Ok(update) => match apply_projection_update(
                &mut instance.status,
                &mut instance.viewport_frame,
                &mut phase_viewport_claimed,
                side_channel_root,
                update,
            ) {
                Ok((state, diagnostic)) => {
                    phase_failed |= state == RuntimeState::Failed;
                    push_projection_diagnostic(
                        &mut diagnostics,
                        projection_name,
                        projection_count,
                        diagnostic,
                    );
                }
                Err(error) => {
                    phase_failed = true;
                    mark_projection_invalid(&mut instance.status, projection_name, "stop", &error);
                    push_projection_diagnostic(
                        &mut diagnostics,
                        projection_name,
                        projection_count,
                        instance.status.diagnostic.clone(),
                    );
                }
            },
            Err(error) => {
                phase_failed = true;
                mark_projection_failed(&mut instance.status, projection_name, "stop", &error);
                push_projection_diagnostic(
                    &mut diagnostics,
                    projection_name,
                    projection_count,
                    instance.status.diagnostic.clone(),
                );
            }
        }
    }
    if phase_failed {
        instance.status.state = RuntimeState::Failed;
    }
    finish_projection_diagnostics(&mut instance.status, &diagnostics);
}

fn apply_projection_update(
    status: &mut RuntimeStatus,
    viewport_frame: &mut Option<RuntimeViewportFrame>,
    phase_viewport_claimed: &mut bool,
    side_channel_root: &Path,
    update: RuntimeProjectionUpdate,
) -> Result<(RuntimeState, String), RuntimeHostError> {
    let RuntimeProjectionUpdate {
        state,
        diagnostic,
        viewport_frame: next_viewport_frame,
    } = update;
    status.state = state;
    if let Some(frame) = next_viewport_frame {
        set_projection_viewport_frame(
            &status.runtime_id,
            viewport_frame,
            phase_viewport_claimed,
            side_channel_root,
            frame,
        )?;
    }
    Ok((state, diagnostic))
}

fn set_projection_viewport_frame(
    runtime_id: &str,
    viewport_frame: &mut Option<RuntimeViewportFrame>,
    phase_viewport_claimed: &mut bool,
    side_channel_root: &Path,
    frame: RuntimeViewportFrame,
) -> Result<(), RuntimeHostError> {
    validate_viewport_frame(&frame, side_channel_root)?;
    if frame.runtime_id != runtime_id {
        return Err(RuntimeHostError::InvalidViewportFrame {
            reason: format!(
                "viewport frame runtime id `{}` does not match projection runtime id `{runtime_id}`",
                frame.runtime_id
            ),
        });
    }
    if !*phase_viewport_claimed {
        *viewport_frame = Some(frame);
        *phase_viewport_claimed = true;
    }
    Ok(())
}

fn push_projection_diagnostic(
    diagnostics: &mut Vec<String>,
    projection_name: &'static str,
    projection_count: usize,
    diagnostic: String,
) {
    if diagnostic.trim().is_empty() {
        return;
    }
    if projection_count <= 1 || diagnostic.starts_with("runtime projection `") {
        diagnostics.push(diagnostic);
    } else {
        diagnostics.push(format!(
            "runtime projection `{projection_name}`: {diagnostic}"
        ));
    }
}

fn finish_projection_diagnostics(status: &mut RuntimeStatus, diagnostics: &[String]) {
    if !diagnostics.is_empty() {
        status.diagnostic = diagnostics.join("\n");
    }
}

fn mark_projection_failed(
    status: &mut RuntimeStatus,
    projection_name: &'static str,
    phase: &str,
    error: &RuntimeProjectionError,
) {
    status.state = RuntimeState::Failed;
    status.diagnostic = format!("runtime projection `{projection_name}` {phase} failed: {error}");
}

fn mark_projection_invalid(
    status: &mut RuntimeStatus,
    projection_name: &'static str,
    phase: &str,
    error: &RuntimeHostError,
) {
    status.state = RuntimeState::Failed;
    status.diagnostic =
        format!("runtime projection `{projection_name}` returned invalid {phase} data: {error}");
}

fn validate_snapshot(
    snapshot: &RuntimeLaunchSnapshot,
    request_role: RuntimeRole,
    capability_session: Option<Uuid>,
    service_session_id: &str,
) -> Result<(), RuntimeHostError> {
    if snapshot.role != request_role {
        return Err(RuntimeHostError::RoleMismatch {
            snapshot_role: snapshot.role,
            request_role,
        });
    }
    if snapshot.project_id.trim().is_empty() {
        return Err(RuntimeHostError::MissingProjectId);
    }
    if snapshot.session_slug.trim().is_empty() {
        return Err(RuntimeHostError::MissingSessionSlug);
    }
    if snapshot.session_id == Uuid::nil() {
        return Err(RuntimeHostError::MissingSessionId);
    }
    if let Some(capability_session) = capability_session
        && snapshot.session_id != capability_session
    {
        return Err(RuntimeHostError::SnapshotSessionMismatch {
            snapshot_session: snapshot.session_id,
            capability_session,
        });
    }
    if snapshot.session_id.to_string() != service_session_id {
        return Err(RuntimeHostError::SnapshotServiceSessionMismatch {
            snapshot_session: snapshot.session_id,
            service_session: service_session_id.to_string(),
        });
    }
    if snapshot.workspace_path.trim().is_empty() {
        return Err(RuntimeHostError::MissingWorkspacePath);
    }
    validate_runtime_snapshot_resolved_gems(&snapshot.resolved_gems)?;
    validate_runtime_snapshot_asset_roots(snapshot)?;
    validate_runtime_snapshot_asset_package_roots(&snapshot.asset_package_roots)?;
    Ok(())
}

fn validate_runtime_snapshot_resolved_gems(
    resolved_gems: &[RuntimeResolvedGem],
) -> Result<(), RuntimeHostError> {
    let mut seen_ids = BTreeSet::new();
    let mut seen_roots = BTreeSet::new();

    for gem in resolved_gems {
        if gem.id.trim().is_empty() {
            return Err(RuntimeHostError::InvalidLaunchSnapshot {
                reason: "resolved gem id cannot be empty".to_string(),
            });
        }
        if gem.name.trim().is_empty() {
            return Err(RuntimeHostError::InvalidLaunchSnapshot {
                reason: format!("resolved gem `{}` name cannot be empty", gem.id),
            });
        }
        if gem.version.trim().is_empty() {
            return Err(RuntimeHostError::InvalidLaunchSnapshot {
                reason: format!("resolved gem `{}` version cannot be empty", gem.id),
            });
        }
        if gem.root.trim().is_empty() {
            return Err(RuntimeHostError::InvalidLaunchSnapshot {
                reason: format!("resolved gem `{}` root cannot be empty", gem.id),
            });
        }
        if !seen_ids.insert(gem.id.as_str()) {
            return Err(RuntimeHostError::InvalidLaunchSnapshot {
                reason: format!("resolved gems contain duplicate id `{}`", gem.id),
            });
        }
        if !seen_roots.insert(gem.root.as_str()) {
            return Err(RuntimeHostError::InvalidLaunchSnapshot {
                reason: format!("resolved gems contain duplicate root `{}`", gem.root),
            });
        }
    }

    Ok(())
}

fn validate_runtime_snapshot_asset_roots(
    snapshot: &RuntimeLaunchSnapshot,
) -> Result<(), RuntimeHostError> {
    if snapshot.schema_catalog_hash.len() != 32 {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: format!(
                "schema catalog hash must be 32 bytes, got {}",
                snapshot.schema_catalog_hash.len()
            ),
        });
    }
    if snapshot.workspace_id <= 0 {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: "asset view id must be positive".to_string(),
        });
    }
    if snapshot.asset_source_roots.is_empty() {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: "asset source roots cannot be empty".to_string(),
        });
    }
    let mut seen_source_root_ids = BTreeSet::new();
    let mut seen_portable_keys = BTreeSet::new();
    for root in &snapshot.asset_source_roots {
        validate_runtime_snapshot_asset_root(snapshot.workspace_id, root)?;
        if !seen_source_root_ids.insert(root.workspace_root_id) {
            return Err(RuntimeHostError::InvalidLaunchSnapshot {
                reason: format!(
                    "asset view {} contains duplicate source root id {}",
                    snapshot.workspace_id, root.workspace_root_id
                ),
            });
        }
        if !seen_portable_keys.insert(root.portable_key.as_str()) {
            return Err(RuntimeHostError::InvalidLaunchSnapshot {
                reason: format!(
                    "asset view {} contains duplicate source root key `{}`",
                    snapshot.workspace_id, root.portable_key
                ),
            });
        }
    }

    let project_source_key = project_assets_portable_key(&snapshot.project_id);
    let project_source = snapshot
        .asset_source_roots
        .iter()
        .find(|root| root.portable_key == project_source_key)
        .ok_or_else(|| RuntimeHostError::InvalidLaunchSnapshot {
            reason: format!("missing DB-owned project asset root `{project_source_key}`"),
        })?;
    if project_source.owner_id != snapshot.project_id {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: format!(
                "project asset root `{project_source_key}` owner `{}` does not match snapshot project `{}`",
                project_source.owner_id, snapshot.project_id
            ),
        });
    }
    let source_root_path = Path::new(&project_source.source_root);
    let workspace_path = Path::new(&snapshot.workspace_path);
    if source_root_path == workspace_path || !source_root_path.starts_with(workspace_path) {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: format!(
                "project asset root `{project_source_key}` path `{}` must be inside workspace `{}` without being the workspace root",
                project_source.source_root, snapshot.workspace_path
            ),
        });
    }
    if !project_source.is_root {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: format!(
                "project asset root `{project_source_key}` must be marked as a root source"
            ),
        });
    }
    if !project_source.output_prefix.is_empty() {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: format!(
                "project asset root `{project_source_key}` must use an empty output prefix"
            ),
        });
    }

    Ok(())
}

fn project_assets_portable_key(project_id: &str) -> String {
    format!("project:{project_id}:assets")
}

fn validate_runtime_snapshot_asset_root(
    workspace_id: i64,
    root: &az_proto_runtime::RuntimeAssetSourceRoot,
) -> Result<(), RuntimeHostError> {
    if root.workspace_id != workspace_id {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: format!(
                "asset source root `{}` belongs to asset view {}, expected {}",
                root.portable_key, root.workspace_id, workspace_id
            ),
        });
    }
    if root.workspace_root_id <= 0 {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: format!(
                "asset source root `{}` must carry a positive DB workspace source root id",
                root.portable_key
            ),
        });
    }
    if root.root_id <= 0 {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: format!(
                "asset source root `{}` must carry a positive DB scan folder id",
                root.portable_key
            ),
        });
    }
    if root.owner_id.trim().is_empty() {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: "asset source root owner id cannot be empty".to_string(),
        });
    }
    if root.source_root.trim().is_empty() {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: format!(
                "asset source root `{}` physical root cannot be empty",
                root.portable_key
            ),
        });
    }
    if root.portable_key.trim().is_empty() {
        return Err(RuntimeHostError::InvalidLaunchSnapshot {
            reason: "asset source root portable key cannot be empty".to_string(),
        });
    }

    Ok(())
}

fn validate_runtime_snapshot_asset_package_roots(
    package_roots: &[az_proto_runtime::RuntimeAssetPackageRoot],
) -> Result<(), RuntimeHostError> {
    az_proto_runtime::validate_runtime_asset_package_roots(package_roots).map_err(|error| {
        RuntimeHostError::InvalidLaunchSnapshot {
            reason: error.to_string(),
        }
    })
}

fn validate_snapshot_handle_capability(
    handle: &SideChannelHandle,
    launch_capability: &Capability,
    service_session_id: &str,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), RuntimeHostError> {
    let capability = require_side_channel_capability(handle, "runtime launch snapshot")
        .map_err(|_| RuntimeHostError::UnboundSnapshotCapability)?;
    if capability.role != ServiceRole::ProjectHost {
        return Err(RuntimeHostError::InvalidCapability {
            reason: format!(
                "runtime launch snapshot side-channel must be bound to a project-host capability, got {:?}",
                capability.role
            ),
        });
    }
    if capability.service.namespace != PROJECT_HOST_SERVICE_NAMESPACE
        || capability.service.name != PROJECT_HOST_SERVICE_NAME
    {
        return Err(RuntimeHostError::InvalidCapability {
            reason: format!(
                "runtime launch snapshot side-channel capability must be issued to `{PROJECT_HOST_SERVICE_NAMESPACE}/{PROJECT_HOST_SERVICE_NAME}`, got `{}/{}`",
                capability.service.namespace, capability.service.name
            ),
        });
    }
    if capability.session.is_none() {
        return Err(RuntimeHostError::InvalidCapability {
            reason: "runtime launch snapshot side-channel capability must be session-bound"
                .to_string(),
        });
    }
    if let Some(launch_session) = launch_capability.session
        && capability.session != Some(launch_session)
    {
        return Err(RuntimeHostError::InvalidCapability {
            reason: format!(
                "runtime launch snapshot side-channel session must match launch capability session `{launch_session}`"
            ),
        });
    }
    validate_control_capability(capability, service_session_id, capability_grants)
}

fn bind_viewport_frame_result_capability(
    result: &mut RuntimeViewportResult,
    capability: &Capability,
) {
    if let Some(frame) = &mut result.frame {
        frame.color.capability = Some(capability.clone());
    }
}

fn validate_viewport_frame(
    frame: &RuntimeViewportFrame,
    side_channel_root: &Path,
) -> Result<(), RuntimeHostError> {
    if frame.runtime_id.trim().is_empty() {
        return Err(RuntimeHostError::InvalidRuntimeId);
    }
    if frame.width == 0 || frame.height == 0 {
        return Err(RuntimeHostError::InvalidViewportFrame {
            reason: format!(
                "viewport dimensions must be non-zero, got {}x{}",
                frame.width, frame.height
            ),
        });
    }
    let bytes_per_pixel =
        frame
            .format
            .bytes_per_pixel()
            .ok_or_else(|| RuntimeHostError::InvalidViewportFrame {
                reason: "viewport pixel format cannot be unknown".to_string(),
            })?;
    let min_row_pitch = frame.width.checked_mul(bytes_per_pixel).ok_or_else(|| {
        RuntimeHostError::InvalidViewportFrame {
            reason: format!(
                "viewport row pitch overflow for {} pixels at {} bytes per pixel",
                frame.width, bytes_per_pixel
            ),
        }
    })?;
    if frame.row_pitch < min_row_pitch {
        return Err(RuntimeHostError::InvalidViewportFrame {
            reason: format!(
                "viewport row pitch {} is smaller than minimum {}",
                frame.row_pitch, min_row_pitch
            ),
        });
    }
    let min_byte_length = u64::from(frame.row_pitch)
        .checked_mul(u64::from(frame.height))
        .ok_or_else(|| RuntimeHostError::InvalidViewportFrame {
            reason: format!(
                "viewport byte length overflow for row pitch {} and height {}",
                frame.row_pitch, frame.height
            ),
        })?;
    if frame.color.byte_length < min_byte_length {
        return Err(RuntimeHostError::InvalidViewportFrame {
            reason: format!(
                "viewport side-channel byte length {} is smaller than minimum {}",
                frame.color.byte_length, min_byte_length
            ),
        });
    }
    if frame.color.locator.trim().is_empty() {
        return Err(RuntimeHostError::InvalidViewportFrame {
            reason: "viewport side-channel locator cannot be empty".to_string(),
        });
    }
    match frame.color.kind {
        SideChannelKind::SharedMemory | SideChannelKind::GpuSurface => {}
        SideChannelKind::MmapFile => {
            let path = validated_mmap_file_path(&frame.color).map_err(|error| {
                RuntimeHostError::InvalidViewportFrame {
                    reason: error.to_string(),
                }
            })?;
            validate_viewport_file_under_root("mmap-file", &path, side_channel_root)?;
        }
        SideChannelKind::StagingFile => {
            let path = validated_staging_file_path(&frame.color).map_err(|error| {
                RuntimeHostError::InvalidViewportFrame {
                    reason: error.to_string(),
                }
            })?;
            validate_viewport_file_under_root("staging-file", &path, side_channel_root)?;
        }
        SideChannelKind::CasBlob => {
            return Err(RuntimeHostError::InvalidViewportFrame {
                reason: "viewport frames must use live side channels, not CAS blobs".to_string(),
            });
        }
    }
    Ok(())
}

fn validate_viewport_file_under_root(
    kind: &str,
    path: &Path,
    side_channel_root: &Path,
) -> Result<(), RuntimeHostError> {
    if !path.starts_with(side_channel_root) {
        return Err(RuntimeHostError::InvalidViewportFrame {
            reason: format!(
                "viewport {kind} side-channel `{}` must live under runtime-host side-channel root `{}`",
                path.display(),
                side_channel_root.display()
            ),
        });
    }
    Ok(())
}

/// Require a runtime-host side-channel root to be an absolute, normal path.
///
/// # Errors
///
/// Returns [`RuntimeHostError::InvalidSideChannelRoot`] if `side_channel_root`
/// is relative or contains `.`/`..`/prefix components.
pub fn validate_runtime_side_channel_root(
    side_channel_root: &Path,
) -> Result<(), RuntimeHostError> {
    if !is_normal_absolute_path(side_channel_root) {
        return Err(RuntimeHostError::InvalidSideChannelRoot {
            reason: format!(
                "runtime-host side-channel root `{}` must be an absolute normal path",
                side_channel_root.display()
            ),
        });
    }
    Ok(())
}

fn is_normal_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
}

fn validate_control_capability(
    capability: &Capability,
    service_session_id: &str,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), RuntimeHostError> {
    validate_capability(
        capability,
        RUNTIME_CONTROL_PERMISSION,
        service_session_id,
        capability_grants,
    )
}

fn validate_read_capability(
    capability: &Capability,
    service_session_id: &str,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), RuntimeHostError> {
    validate_capability(
        capability,
        RUNTIME_READ_PERMISSION,
        service_session_id,
        capability_grants,
    )
}

fn validate_capability(
    capability: &Capability,
    required_permission: &str,
    service_session_id: &str,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), RuntimeHostError> {
    if let Err(error) = capability.validate_lifetime() {
        return Err(RuntimeHostError::InvalidCapability {
            reason: error.to_string(),
        });
    }

    if capability.audience != RUNTIME_HOST_AUDIENCE {
        return Err(RuntimeHostError::InvalidCapability {
            reason: format!(
                "expected audience `{RUNTIME_HOST_AUDIENCE}` but got `{}`",
                capability.audience
            ),
        });
    }
    if !matches!(
        capability.role,
        ServiceRole::Editor
            | ServiceRole::SessionSupervisor
            | ServiceRole::ProjectHost
            | ServiceRole::RuntimeHost
    ) {
        return Err(RuntimeHostError::InvalidCapability {
            reason: format!("unsupported role {:?}", capability.role),
        });
    }
    let has_required = capability
        .permissions
        .iter()
        .any(|permission| permission == required_permission);
    let has_control = capability
        .permissions
        .iter()
        .any(|permission| permission == RUNTIME_CONTROL_PERMISSION);
    if !(has_required || (required_permission == RUNTIME_READ_PERMISSION && has_control)) {
        return Err(RuntimeHostError::InvalidCapability {
            reason: format!("missing required permission `{required_permission}`"),
        });
    }

    validate_service_session_scope(capability, service_session_id, required_permission)?;

    let broker_permission = if has_required {
        required_permission
    } else {
        RUNTIME_CONTROL_PERMISSION
    };
    capability_grants
        .validate(capability, broker_permission)
        .map_err(|error| RuntimeHostError::InvalidCapability {
            reason: error.to_string(),
        })?;

    Ok(())
}

fn validate_service_session_scope(
    capability: &Capability,
    service_session_id: &str,
    required_permission: &str,
) -> Result<(), RuntimeHostError> {
    let Some(capability_session) = capability.session else {
        return Err(RuntimeHostError::InvalidCapability {
            reason: format!(
                "missing session scope for `{required_permission}` on session `{service_session_id}`"
            ),
        });
    };

    let capability_session = capability_session.to_string();
    if capability_session != service_session_id {
        return Err(RuntimeHostError::InvalidCapability {
            reason: format!(
                "capability session `{capability_session}` cannot use `{required_permission}` on session `{service_session_id}`"
            ),
        });
    }

    Ok(())
}

fn runtime_host_error_to_capnp(error: &RuntimeHostError) -> Error {
    Error::failed(format!("runtime-host failed: {error}"))
}

#[cfg(test)]
mod architecture_tests;

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use az_architecture_guard::{
        production_source_without_cfg_test_modules, public_functions_are_cfg_test_or_test_support,
    };
    use az_proto_core::{CapabilityGrantSet, ServiceId, SideChannelHandle, SideChannelKind};
    use az_proto_runtime::{
        LaunchRuntimeRequest, RuntimeAssetSourceRoot, RuntimeProjectionCatalogRequest,
        RuntimeProjectionCatalogResult, RuntimeStatus, RuntimeStatusRequest, RuntimeStatusResult,
        RuntimeViewportFrame, RuntimeViewportRequest, RuntimeViewportResult, StopRuntimeRequest,
        ViewportPixelFormat, encode_runtime_launch_snapshot,
    };
    use futures::executor;
    use uuid::Uuid;

    use super::*;

    const DEFAULT_SNAPSHOT_SESSION: [u8; 16] = [0x77; 16];
    const RUNTIME_TOKEN: [u8; 2] = [0x72, 0x48];
    const PROJECT_HOST_TOKEN: [u8; 2] = [0x70, 0x48];

    #[test]
    fn bootstrap_refuses_a_composer_for_another_role_before_finalization() {
        let error =
            RuntimeHostComposition::from_composer(Composer::new(GemTargetRole::AssetWorker))
                .unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostCompositionError::WrongRole {
                actual: GemTargetRole::AssetWorker
            }
        ));
    }

    #[test]
    fn bootstrap_retains_an_empty_runtime_host_composition() {
        let composition =
            RuntimeHostComposition::from_composer(Composer::new(GemTargetRole::RuntimeHost))
                .unwrap();

        assert_eq!(
            composition.process.report().role,
            GemTargetRole::RuntimeHost
        );
    }

    #[test]
    fn bootstrap_separates_rpc_read_access_from_lifecycle_ownership() {
        let temp = tempfile::tempdir().unwrap();
        let (host, composition) = RuntimeHost::from_composer(
            Uuid::from_bytes(DEFAULT_SNAPSHOT_SESSION).to_string(),
            CapabilityGrantSet::new(),
            temp.path().join("runtime-host"),
            Composer::new(GemTargetRole::RuntimeHost),
        )
        .unwrap();

        assert!(
            host.registries()
                .get::<RuntimeProjectionRegistration>()
                .is_none()
        );
        assert_eq!(composition.process.role(), GemTargetRole::RuntimeHost);
        assert!(std::ptr::eq(host.registries(), composition.registries()));
    }

    fn capability(permission: &str) -> Capability {
        scoped_capability(permission, Uuid::from_bytes(DEFAULT_SNAPSHOT_SESSION))
    }

    fn unscoped_capability(permission: &str) -> Capability {
        Capability::new(
            ServiceId::new("azoth", "session-supervisor"),
            ServiceRole::SessionSupervisor,
        )
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([permission])
        .with_token_hash(RUNTIME_TOKEN)
    }

    fn scoped_capability(permission: &str, session: Uuid) -> Capability {
        unscoped_capability(permission).with_session(session)
    }

    fn project_host_snapshot_capability(session: Uuid) -> Capability {
        Capability::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
        )
        .with_session(session)
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_CONTROL_PERMISSION])
        .with_token_hash(PROJECT_HOST_TOKEN)
    }

    az_gem_contract::declare_caps!(HostCaps:);

    /// The three projections these tests dispatch, contributed the way a
    /// project gem contributes its own.
    struct Projections;

    impl az_gem_contract::Contribution for Projections {
        type Caps = HostCaps;

        fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
            az_gem_contract::ContributionDescriptor {
                gem: az_gem_contract::GemId::new("azoth.runtime-host-tests"),
                contribution: az_gem_contract::ContributionId::new("projections"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, HostCaps>) {
            // Deliberately out of dispatch order: what a host serves is
            // precedence, not the order a contribution happened to write.
            ctx.registrar::<RuntimeProjectionRegistration>()
                .register_many([COMPOSITE_GEM, REGISTERED_TEST, COMPOSITE_PROJECT]);
        }
    }

    /// A runtime-host process composes once for its whole life, so the tests
    /// hold one composition the same way a real service does.
    fn composed() -> &'static Registries {
        static REGISTRIES: std::sync::OnceLock<&'static Registries> = std::sync::OnceLock::new();
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

    fn runtime_host() -> RuntimeHost {
        runtime_host_for_session(Uuid::from_bytes(DEFAULT_SNAPSHOT_SESSION))
    }

    fn runtime_host_for_session(session: Uuid) -> RuntimeHost {
        runtime_host_for_session_with_side_channel_root(
            session,
            std::env::temp_dir().join(format!("az-runtime-host-test-{session}")),
        )
    }

    fn runtime_host_for_session_with_side_channel_root(
        session: Uuid,
        side_channel_root: impl Into<PathBuf>,
    ) -> RuntimeHost {
        let mut grants = vec![
            scoped_capability(RUNTIME_READ_PERMISSION, session),
            scoped_capability(RUNTIME_CONTROL_PERMISSION, session),
        ];
        grants.push(project_host_snapshot_capability(session));
        RuntimeHost::new(
            session.to_string(),
            CapabilityGrantSet::from_grants(grants),
            side_channel_root,
            composed(),
        )
    }

    fn snapshot_handle(temp: &tempfile::TempDir, role: RuntimeRole) -> SideChannelHandle {
        snapshot_handle_with_profile(temp, role, "editor")
    }

    fn snapshot_handle_with_profile(
        temp: &tempfile::TempDir,
        role: RuntimeRole,
        launch_profile: &str,
    ) -> SideChannelHandle {
        snapshot_handle_with_profile_and_session(
            temp,
            role,
            launch_profile,
            Uuid::from_bytes([0x77; 16]),
        )
    }

    fn snapshot_handle_with_profile_and_session(
        temp: &tempfile::TempDir,
        role: RuntimeRole,
        launch_profile: &str,
        session_id: Uuid,
    ) -> SideChannelHandle {
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            session_id,
            "lighting",
            role,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.authored_revision = 12;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 77;
        snapshot.launch_profile = launch_profile.to_string();
        add_project_asset_source_root(&mut snapshot);
        snapshot_handle_from_snapshot(
            temp,
            format!("runtime-launch-{launch_profile}.capnp.packed"),
            &snapshot,
        )
    }

    fn snapshot_handle_from_snapshot(
        temp: &tempfile::TempDir,
        file_name: impl AsRef<str>,
        snapshot: &RuntimeLaunchSnapshot,
    ) -> SideChannelHandle {
        let bytes = encode_runtime_launch_snapshot(snapshot).unwrap();
        let hash = blake3::hash(&bytes).as_bytes().to_vec();
        let path = temp.path().join(file_name.as_ref());
        std::fs::write(&path, bytes).unwrap();
        let byte_length = std::fs::metadata(&path).unwrap().len();
        SideChannelHandle::staging_file(
            path.to_string_lossy(),
            byte_length,
            hash,
            std::env::consts::OS,
        )
        .with_capability(project_host_snapshot_capability(snapshot.session_id))
    }

    fn add_project_asset_source_root(snapshot: &mut RuntimeLaunchSnapshot) {
        snapshot.asset_source_roots.push(RuntimeAssetSourceRoot {
            workspace_root_id: 101,
            workspace_id: snapshot.workspace_id,
            root_id: 202,
            owner_id: snapshot.project_id.clone(),
            source_root: Path::new(&snapshot.workspace_path)
                .join("assets")
                .to_string_lossy()
                .into_owned(),
            display_name: "Project Assets".to_string(),
            portable_key: project_assets_portable_key(&snapshot.project_id),
            output_prefix: String::new(),
            is_root: true,
        });
    }

    struct RegisteredTestProjection;

    impl RuntimeProjection for RegisteredTestProjection {
        fn launch(
            &mut self,
            context: &RuntimeProjectionLaunchContext<'_>,
        ) -> Result<RuntimeProjectionUpdate, RuntimeProjectionError> {
            Ok(RuntimeProjectionUpdate::running()
                .with_diagnostic(format!(
                    "registered projection launched asset view {}",
                    context.snapshot.workspace_id
                ))
                .with_viewport_frame(RuntimeViewportFrame {
                    runtime_id: context.runtime_id.to_string(),
                    width: 64,
                    height: 32,
                    row_pitch: 256,
                    format: ViewportPixelFormat::Bgra8Unorm,
                    color: SideChannelHandle {
                        kind: SideChannelKind::GpuSurface,
                        capability: None,
                        locator: "dxgi://adapter/0/texture/registered".to_string(),
                        byte_length: 8_192,
                        content_hash: Vec::new(),
                        platform: "windows".to_string(),
                    },
                }))
        }

        fn stop(
            &mut self,
            context: &RuntimeProjectionStopContext<'_>,
        ) -> Result<RuntimeProjectionUpdate, RuntimeProjectionError> {
            let diagnostic = if context.preserve {
                format!(
                    "registered projection preserved runtime {}",
                    context.runtime_id
                )
            } else {
                format!(
                    "registered projection stopped runtime {}",
                    context.runtime_id
                )
            };
            Ok(RuntimeProjectionUpdate::new(RuntimeState::Stopped).with_diagnostic(diagnostic))
        }
    }

    fn registered_test_projection() -> Box<dyn RuntimeProjection> {
        Box::new(RegisteredTestProjection)
    }

    const REGISTERED_TEST: RuntimeProjectionRegistration = RuntimeProjectionRegistration::new(
        "az.test.registered-runtime-projection",
        &[RuntimeRole::Validation],
        &["registered-test"],
        registered_test_projection,
    )
    .with_priority(100);

    struct CompositeProjectProjection;

    impl RuntimeProjection for CompositeProjectProjection {
        fn launch(
            &mut self,
            context: &RuntimeProjectionLaunchContext<'_>,
        ) -> Result<RuntimeProjectionUpdate, RuntimeProjectionError> {
            Ok(RuntimeProjectionUpdate::running()
                .with_diagnostic("project projection launched")
                .with_viewport_frame(RuntimeViewportFrame {
                    runtime_id: context.runtime_id.to_string(),
                    width: 96,
                    height: 54,
                    row_pitch: 384,
                    format: ViewportPixelFormat::Bgra8Unorm,
                    color: SideChannelHandle {
                        kind: SideChannelKind::GpuSurface,
                        capability: None,
                        locator: "dxgi://adapter/0/texture/composite-project".to_string(),
                        byte_length: 20_736,
                        content_hash: Vec::new(),
                        platform: "windows".to_string(),
                    },
                }))
        }

        fn stop(
            &mut self,
            _context: &RuntimeProjectionStopContext<'_>,
        ) -> Result<RuntimeProjectionUpdate, RuntimeProjectionError> {
            Ok(RuntimeProjectionUpdate::stopped(false)
                .with_diagnostic("project projection stopped"))
        }
    }

    fn composite_project_projection() -> Box<dyn RuntimeProjection> {
        Box::new(CompositeProjectProjection)
    }

    const COMPOSITE_PROJECT: RuntimeProjectionRegistration = RuntimeProjectionRegistration::new(
        "az.test.composite-project",
        &[RuntimeRole::PlayPreview],
        &["composite-test"],
        composite_project_projection,
    )
    .with_priority(20);

    struct CompositeGemProjection;

    impl RuntimeProjection for CompositeGemProjection {
        fn launch(
            &mut self,
            context: &RuntimeProjectionLaunchContext<'_>,
        ) -> Result<RuntimeProjectionUpdate, RuntimeProjectionError> {
            Ok(RuntimeProjectionUpdate::running()
                .with_diagnostic("gem projection launched")
                .with_viewport_frame(RuntimeViewportFrame {
                    runtime_id: context.runtime_id.to_string(),
                    width: 32,
                    height: 18,
                    row_pitch: 128,
                    format: ViewportPixelFormat::Bgra8Unorm,
                    color: SideChannelHandle {
                        kind: SideChannelKind::GpuSurface,
                        capability: None,
                        locator: "dxgi://adapter/0/texture/composite-gem".to_string(),
                        byte_length: 2_304,
                        content_hash: Vec::new(),
                        platform: "windows".to_string(),
                    },
                }))
        }

        fn stop(
            &mut self,
            _context: &RuntimeProjectionStopContext<'_>,
        ) -> Result<RuntimeProjectionUpdate, RuntimeProjectionError> {
            Ok(RuntimeProjectionUpdate::stopped(false).with_diagnostic("gem projection stopped"))
        }
    }

    fn composite_gem_projection() -> Box<dyn RuntimeProjection> {
        Box::new(CompositeGemProjection)
    }

    const COMPOSITE_GEM: RuntimeProjectionRegistration = RuntimeProjectionRegistration::new(
        "az.test.composite-gem",
        &[RuntimeRole::PlayPreview],
        &["composite-test"],
        composite_gem_projection,
    )
    .with_priority(10);

    fn viewport_frame() -> RuntimeViewportFrame {
        RuntimeViewportFrame {
            runtime_id: "editor-world".to_string(),
            width: 1920,
            height: 1080,
            row_pitch: 7680,
            format: ViewportPixelFormat::Bgra8Unorm,
            color: SideChannelHandle {
                kind: SideChannelKind::GpuSurface,
                capability: None,
                locator: "dxgi://adapter/0/texture/2".to_string(),
                byte_length: 8_294_400,
                content_hash: Vec::new(),
                platform: "windows".to_string(),
            },
        }
    }

    #[test]
    fn in_process_runtime_host_rpc_constructors_are_not_default_public_api() {
        let source = include_str!("lib.rs");
        let production_source = production_source_without_cfg_test_modules(source);

        for signature in [
            "pub(crate) fn new(host: RuntimeHost)",
            "pub(crate) fn into_client(self)",
        ] {
            assert!(
                source.contains(signature),
                "`{signature}` must stay crate-private; production callers should start a runtime-host RPC server instead of embedding it"
            );
        }
        assert!(
            !production_source.contains("pub fn runtime_host_client(")
                && !production_source.contains("pub(crate) fn runtime_host_client("),
            "runtime-host must not expose a default in-process client helper; use endpoint-hosted RPC or explicit test-support constructors"
        );

        for signature in ["pub fn test_new(", "pub fn host(", "pub fn client_from_rc("] {
            assert!(
                public_functions_are_cfg_test_or_test_support(&production_source, signature),
                "`{signature}` must require the explicit test-support feature; in-process runtime-host clients are for tests/harnesses only"
            );
        }
    }

    #[test]
    fn runtime_host_rpc_server_starters_are_session_bound() {
        let source = include_str!("transport.rs");
        let production_source = production_source_without_cfg_test_modules(source);

        assert!(
            !source.contains("pub fn start_runtime_host_rpc_server_with_capability_grants("),
            "runtime-host must not expose a sessionless RPC server starter"
        );
        assert!(
            public_functions_are_cfg_test_or_test_support(
                &production_source,
                "pub fn start_runtime_host_rpc_server_for_session_with_capability_grants("
            ),
            "`start_runtime_host_rpc_server_for_session_with_capability_grants` must require explicit test-support"
        );
        assert!(
            public_functions_are_cfg_test_or_test_support(
                &production_source,
                "pub fn start_runtime_host_rpc_server_for_session_with_capability_grant_file("
            ),
            "the linked-registry grant-file adapter must require explicit test-support"
        );
        assert!(
            production_source.contains("pub fn start_runtime_host_service(")
                && production_source.contains("startup: RuntimeHostServiceStartup")
                && production_source.contains("composer: Composer"),
            "runtime-host production startup must consume its composer through the owning service boundary"
        );
        assert!(
            production_source
                .contains("validate_runtime_side_channel_root(&startup.side_channel_root)")
                && production_source.contains("if startup.run.is_nil()"),
            "runtime-host production startup must validate its explicit session side-channel root and run"
        );
    }

    #[test]
    fn runtime_host_domain_launches_from_snapshot_and_tracks_status() {
        let temp = tempfile::tempdir().unwrap();
        let host = runtime_host();
        let request = LaunchRuntimeRequest {
            capability: capability(RUNTIME_CONTROL_PERMISSION),
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            snapshot: snapshot_handle(&temp, RuntimeRole::EditorWorld),
        };

        let status = host.launch(&request).unwrap();

        assert_eq!(status.state, RuntimeState::Running);
        assert_eq!(status.authored_revision, 12);
        assert_eq!(
            host.launched_snapshot("editor-world")
                .unwrap()
                .schema_catalog_hash,
            vec![0xab; 32]
        );

        let queried = host
            .status(&RuntimeStatusRequest {
                capability: capability(RUNTIME_READ_PERMISSION),
                runtime_id: "editor-world".to_string(),
            })
            .unwrap();
        assert_eq!(queried.status, Some(status));
    }

    #[test]
    fn runtime_host_rejects_launch_snapshot_without_db_source_roots() {
        let session_id = Uuid::from_bytes([0x77; 16]);
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            session_id,
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.authored_revision = 12;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 77;

        let session_scope = session_id.to_string();
        let error = validate_snapshot(
            &snapshot,
            RuntimeRole::EditorWorld,
            Some(session_id),
            &session_scope,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidLaunchSnapshot { reason }
                if reason.contains("asset source roots cannot be empty")
        ));
    }

    #[test]
    fn runtime_host_rejects_launch_snapshot_without_project_source_root() {
        let session_id = Uuid::from_bytes([0x77; 16]);
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            session_id,
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.authored_revision = 12;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 77;
        snapshot.asset_source_roots.push(RuntimeAssetSourceRoot {
            workspace_root_id: 101,
            workspace_id: 77,
            root_id: 202,
            owner_id: "azoth.physics".to_string(),
            source_root: "projects/runtime/gems/physics/assets".to_string(),
            display_name: "Physics Assets".to_string(),
            portable_key: "gem:azoth.physics:assets".to_string(),
            output_prefix: "gems/azoth.physics".to_string(),
            is_root: true,
        });

        let session_scope = session_id.to_string();
        let error = validate_snapshot(
            &snapshot,
            RuntimeRole::EditorWorld,
            Some(session_id),
            &session_scope,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidLaunchSnapshot { reason }
                if reason.contains("missing DB-owned project asset root")
        ));
    }

    #[test]
    fn runtime_host_rejects_launch_snapshot_with_duplicate_asset_source_roots() {
        let session_id = Uuid::from_bytes([0x77; 16]);
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            session_id,
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.authored_revision = 12;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 77;
        add_project_asset_source_root(&mut snapshot);
        let mut duplicate_id = snapshot.asset_source_roots[0].clone();
        duplicate_id.portable_key = "project:local.runtime:assets".to_string();
        duplicate_id.source_root = "projects/runtime/assets".to_string();
        snapshot.asset_source_roots.push(duplicate_id);

        let session_scope = session_id.to_string();
        let error = validate_snapshot(
            &snapshot,
            RuntimeRole::EditorWorld,
            Some(session_id),
            &session_scope,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidLaunchSnapshot { reason }
                if reason.contains("duplicate source root id")
        ));

        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            session_id,
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.authored_revision = 12;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 77;
        add_project_asset_source_root(&mut snapshot);
        let mut duplicate_key = snapshot.asset_source_roots[0].clone();
        duplicate_key.workspace_root_id += 1;
        duplicate_key.root_id += 1;
        duplicate_key.source_root = "projects/runtime/assets".to_string();
        snapshot.asset_source_roots.push(duplicate_key);

        let error = validate_snapshot(
            &snapshot,
            RuntimeRole::EditorWorld,
            Some(session_id),
            &session_scope,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidLaunchSnapshot { reason }
                if reason.contains("duplicate source root key")
        ));
    }

    #[test]
    fn runtime_host_rejects_launch_snapshot_with_duplicate_resolved_gems() {
        let session_id = Uuid::from_bytes([0x77; 16]);
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            session_id,
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.authored_revision = 12;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 77;
        add_project_asset_source_root(&mut snapshot);
        snapshot.resolved_gems.push(RuntimeResolvedGem {
            id: "azoth.physics".to_string(),
            name: "Physics".to_string(),
            version: "0.1.0".to_string(),
            root: "projects/runtime/gems/physics".to_string(),
        });
        snapshot.resolved_gems.push(RuntimeResolvedGem {
            id: "azoth.physics".to_string(),
            name: "Physics Copy".to_string(),
            version: "0.1.0".to_string(),
            root: "projects/runtime/gems/physics-copy".to_string(),
        });

        let session_scope = session_id.to_string();
        let error = validate_snapshot(
            &snapshot,
            RuntimeRole::EditorWorld,
            Some(session_id),
            &session_scope,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidLaunchSnapshot { reason }
                if reason.contains("duplicate id `azoth.physics`")
        ));
    }

    #[test]
    fn runtime_host_rejects_project_source_owner_mismatch() {
        let session_id = Uuid::from_bytes([0x77; 16]);
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            session_id,
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.authored_revision = 12;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 77;
        add_project_asset_source_root(&mut snapshot);
        snapshot.asset_source_roots[0].owner_id = "azoth.physics".to_string();

        let session_scope = session_id.to_string();
        let error = validate_snapshot(
            &snapshot,
            RuntimeRole::EditorWorld,
            Some(session_id),
            &session_scope,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidLaunchSnapshot { reason }
                if reason.contains("owner")
                    && reason.contains("azoth.physics")
                    && reason.contains("local.runtime")
        ));
    }

    #[test]
    fn runtime_host_rejects_project_source_root_shape_mismatch() {
        let session_id = Uuid::from_bytes([0x77; 16]);
        let mut not_root = RuntimeLaunchSnapshot::new(
            "local.runtime",
            session_id,
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        not_root.authored_revision = 12;
        not_root.schema_catalog_hash = vec![0xab; 32];
        not_root.workspace_id = 77;
        add_project_asset_source_root(&mut not_root);
        not_root.asset_source_roots[0].is_root = false;

        let session_scope = session_id.to_string();
        let error = validate_snapshot(
            &not_root,
            RuntimeRole::EditorWorld,
            Some(session_id),
            &session_scope,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidLaunchSnapshot { reason }
                if reason.contains("marked as a root source")
        ));

        let mut prefixed = RuntimeLaunchSnapshot::new(
            "local.runtime",
            session_id,
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        prefixed.authored_revision = 12;
        prefixed.schema_catalog_hash = vec![0xab; 32];
        prefixed.workspace_id = 77;
        add_project_asset_source_root(&mut prefixed);
        prefixed.asset_source_roots[0].output_prefix = "prefabs".to_string();

        let error = validate_snapshot(
            &prefixed,
            RuntimeRole::EditorWorld,
            Some(session_id),
            &session_scope,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidLaunchSnapshot { reason }
                if reason.contains("empty output prefix")
        ));
    }

    #[test]
    fn runtime_host_rejects_unbound_launch_snapshot_side_channel() {
        let temp = tempfile::tempdir().unwrap();
        let host = runtime_host();
        let mut snapshot = snapshot_handle(&temp, RuntimeRole::EditorWorld);
        snapshot.capability = None;

        let error = host
            .launch(&LaunchRuntimeRequest {
                capability: capability(RUNTIME_CONTROL_PERMISSION),
                runtime_id: "editor-world".to_string(),
                role: RuntimeRole::EditorWorld,
                snapshot,
            })
            .unwrap_err();

        assert!(matches!(error, RuntimeHostError::UnboundSnapshotCapability));
    }

    #[test]
    fn runtime_host_accepts_unsaved_policy_without_parallel_payload() {
        let temp = tempfile::tempdir().unwrap();
        let host = runtime_host();
        let mut snapshot = RuntimeLaunchSnapshot::new(
            "local.runtime",
            Uuid::from_bytes([0x77; 16]),
            "lighting",
            RuntimeRole::EditorWorld,
            "projects/runtime",
            "projects/runtime/.azoth/workspaces/lighting",
        );
        snapshot.authored_revision = 12;
        snapshot.schema_catalog_hash = vec![0xab; 32];
        snapshot.workspace_id = 77;
        snapshot.include_unsaved_journal = true;
        add_project_asset_source_root(&mut snapshot);

        let status = host
            .launch(&LaunchRuntimeRequest {
                capability: capability(RUNTIME_CONTROL_PERMISSION),
                runtime_id: "editor-world".to_string(),
                role: RuntimeRole::EditorWorld,
                snapshot: snapshot_handle_from_snapshot(
                    &temp,
                    "runtime-launch-unsaved-policy.capnp.packed",
                    &snapshot,
                ),
            })
            .unwrap();

        assert_eq!(status.state, RuntimeState::Running);
    }

    #[test]
    fn runtime_host_rejects_launch_snapshot_from_noncanonical_project_host_service() {
        let temp = tempfile::tempdir().unwrap();
        let host = runtime_host();
        let mut snapshot = snapshot_handle(&temp, RuntimeRole::EditorWorld);
        snapshot.capability.as_mut().unwrap().service =
            ServiceId::new("azoth", "other-project-host");

        let error = host
            .launch(&LaunchRuntimeRequest {
                capability: capability(RUNTIME_CONTROL_PERMISSION),
                runtime_id: "editor-world".to_string(),
                role: RuntimeRole::EditorWorld,
                snapshot,
            })
            .unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidCapability { reason }
                if reason.contains("must be issued to `azoth/project-host`")
                    && reason.contains("other-project-host")
        ));
    }

    #[test]
    fn runtime_host_tracks_latest_viewport_frame_side_channel() {
        let temp = tempfile::tempdir().unwrap();
        let host = runtime_host();
        host.launch(&LaunchRuntimeRequest {
            capability: capability(RUNTIME_CONTROL_PERMISSION),
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            snapshot: snapshot_handle(&temp, RuntimeRole::EditorWorld),
        })
        .unwrap();

        let request = RuntimeViewportRequest {
            capability: capability(RUNTIME_READ_PERMISSION),
            runtime_id: "editor-world".to_string(),
        };
        assert_eq!(host.viewport_frame(&request).unwrap().frame, None);

        let expected = host.publish_viewport_frame(viewport_frame()).unwrap();
        assert_eq!(host.viewport_frame(&request).unwrap().frame, Some(expected));
    }

    #[test]
    fn runtime_host_rejects_undersized_viewport_frame_side_channel() {
        let temp = tempfile::tempdir().unwrap();
        let host = runtime_host();
        host.launch(&LaunchRuntimeRequest {
            capability: capability(RUNTIME_CONTROL_PERMISSION),
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            snapshot: snapshot_handle(&temp, RuntimeRole::EditorWorld),
        })
        .unwrap();
        let mut frame = viewport_frame();
        frame.color.byte_length = 1;

        let error = host.publish_viewport_frame(frame).unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidViewportFrame { .. }
        ));
    }

    #[test]
    fn runtime_host_rejects_viewport_frame_cas_blob_side_channel() {
        let temp = tempfile::tempdir().unwrap();
        let host = runtime_host();
        host.launch(&LaunchRuntimeRequest {
            capability: capability(RUNTIME_CONTROL_PERMISSION),
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            snapshot: snapshot_handle(&temp, RuntimeRole::EditorWorld),
        })
        .unwrap();
        let mut frame = viewport_frame();
        frame.color.kind = SideChannelKind::CasBlob;
        frame.color.locator = "cas://frames/editor-world/2".to_string();

        let error = host.publish_viewport_frame(frame).unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidViewportFrame { .. }
        ));
    }

    #[test]
    fn runtime_host_rejects_viewport_file_side_channel_outside_session_root() {
        let temp = tempfile::tempdir().unwrap();
        let side_channel_root = temp.path().join("runtime-host");
        let host = runtime_host_for_session_with_side_channel_root(
            Uuid::from_bytes(DEFAULT_SNAPSHOT_SESSION),
            &side_channel_root,
        );
        host.launch(&LaunchRuntimeRequest {
            capability: capability(RUNTIME_CONTROL_PERMISSION),
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            snapshot: snapshot_handle(&temp, RuntimeRole::EditorWorld),
        })
        .unwrap();
        let mut frame = viewport_frame();
        frame.color = SideChannelHandle::staging_file(
            temp.path()
                .join("outside")
                .join("viewport-frame.capnp.packed")
                .to_string_lossy()
                .into_owned(),
            frame.color.byte_length,
            vec![0; 32],
            std::env::consts::OS,
        );

        let error = host.publish_viewport_frame(frame).unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidViewportFrame { reason }
                if reason.contains("must live under runtime-host side-channel root")
        ));
    }

    #[test]
    fn runtime_host_accepts_viewport_file_side_channel_under_session_root() {
        let temp = tempfile::tempdir().unwrap();
        let side_channel_root = temp.path().join("runtime-host");
        let host = runtime_host_for_session_with_side_channel_root(
            Uuid::from_bytes(DEFAULT_SNAPSHOT_SESSION),
            &side_channel_root,
        );
        host.launch(&LaunchRuntimeRequest {
            capability: capability(RUNTIME_CONTROL_PERMISSION),
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            snapshot: snapshot_handle(&temp, RuntimeRole::EditorWorld),
        })
        .unwrap();
        let mut frame = viewport_frame();
        frame.color = SideChannelHandle::mmap_file(
            side_channel_root
                .join("viewport-frame.mmap")
                .to_string_lossy()
                .into_owned(),
            frame.color.byte_length,
            std::env::consts::OS,
        );

        let accepted = host.publish_viewport_frame(frame.clone()).unwrap();

        assert_eq!(accepted.color, frame.color);
    }

    #[test]
    fn runtime_host_reports_registered_runtime_projection_catalog() {
        let host = runtime_host();

        let catalog = host
            .projection_catalog(&RuntimeProjectionCatalogRequest {
                capability: capability(RUNTIME_READ_PERMISSION),
            })
            .unwrap();

        let registered = catalog
            .projections
            .iter()
            .find(|projection| projection.name == "az.test.registered-runtime-projection")
            .expect("test projection should be reported");
        assert_eq!(registered.priority, 100);
        assert_eq!(registered.roles, vec![RuntimeRole::Validation]);
        assert_eq!(registered.launch_profiles, vec!["registered-test"]);

        // The catalog is the composition read in dispatch order, not the
        // order the contribution wrote its registrations in.
        assert_eq!(
            catalog
                .projections
                .iter()
                .map(|projection| projection.name.as_str())
                .collect::<Vec<_>>(),
            [
                "az.test.registered-runtime-projection",
                "az.test.composite-project",
                "az.test.composite-gem",
            ]
        );
    }

    #[test]
    fn every_composed_projection_is_attributed_to_its_contribution() {
        let entries = composed()
            .get::<RuntimeProjectionRegistration>()
            .expect("the harness composed projections");
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|attributed| {
            attributed.instance.gem.as_str() == "azoth.runtime-host-tests"
                && attributed.instance.contribution.as_str() == "projections"
        }));
    }

    #[test]
    fn two_projections_under_one_name_fail_composition() {
        struct Rival;

        impl az_gem_contract::Contribution for Rival {
            type Caps = HostCaps;

            fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
                az_gem_contract::ContributionDescriptor {
                    gem: az_gem_contract::GemId::new("azoth.runtime-host-rival"),
                    contribution: az_gem_contract::ContributionId::new("projections"),
                    roles: &[],
                }
            }

            fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, HostCaps>) {
                // A different role and profile under the same name: the name
                // is what a launch diagnostic blames, so this collides.
                ctx.registrar::<RuntimeProjectionRegistration>().register(
                    RuntimeProjectionRegistration::new(
                        "az.test.composite-gem",
                        &[RuntimeRole::Validation],
                        &["rival-test"],
                        composite_gem_projection,
                    ),
                );
            }
        }

        let mut composer =
            az_gem_contract::Composer::new(az_gem_contract::GemTargetRole::RuntimeHost);
        composer
            .add(Projections, az_gem_contract::ProductActivation::default())
            .expect("an empty floor composes");
        composer
            .add(Rival, az_gem_contract::ProductActivation::default())
            .expect("an empty floor composes");

        let error = composer.finalize().expect_err("the name collides");
        let az_gem_contract::ComposeError::Duplicate {
            registry,
            key,
            first,
            second,
        } = error
        else {
            panic!("expected a duplicate-key failure, got {error}");
        };
        assert_eq!(registry, "runtime-projection");
        assert_eq!(key, "az.test.composite-gem");
        assert_eq!(first.gem.as_str(), "azoth.runtime-host-tests");
        assert_eq!(second.gem.as_str(), "azoth.runtime-host-rival");
    }

    #[test]
    fn runtime_host_uses_registered_project_projection() {
        let temp = tempfile::tempdir().unwrap();
        let host = runtime_host();
        let request = LaunchRuntimeRequest {
            capability: capability(RUNTIME_CONTROL_PERMISSION),
            runtime_id: "registered-runtime".to_string(),
            role: RuntimeRole::Validation,
            snapshot: snapshot_handle_with_profile(
                &temp,
                RuntimeRole::Validation,
                "registered-test",
            ),
        };

        let status = host.launch(&request).unwrap();

        assert_eq!(status.state, RuntimeState::Running);
        assert_eq!(
            status.diagnostic,
            "registered projection launched asset view 77"
        );
        let frame = host
            .viewport_frame(&RuntimeViewportRequest {
                capability: capability(RUNTIME_READ_PERMISSION),
                runtime_id: "registered-runtime".to_string(),
            })
            .unwrap()
            .frame
            .unwrap();
        assert_eq!(frame.runtime_id, "registered-runtime");
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 32);

        let stopped = host
            .stop(&StopRuntimeRequest {
                capability: capability(RUNTIME_CONTROL_PERMISSION),
                runtime_id: "registered-runtime".to_string(),
                preserve: true,
            })
            .unwrap();
        assert_eq!(stopped.state, RuntimeState::Stopped);
        assert_eq!(
            stopped.diagnostic,
            "registered projection preserved runtime registered-runtime"
        );
    }

    #[test]
    fn runtime_host_composes_matching_project_and_gem_projections() {
        let temp = tempfile::tempdir().unwrap();
        let host = runtime_host();
        let request = LaunchRuntimeRequest {
            capability: capability(RUNTIME_CONTROL_PERMISSION),
            runtime_id: "composite-runtime".to_string(),
            role: RuntimeRole::PlayPreview,
            snapshot: snapshot_handle_with_profile(
                &temp,
                RuntimeRole::PlayPreview,
                "composite-test",
            ),
        };

        let status = host.launch(&request).unwrap();

        assert_eq!(status.state, RuntimeState::Running);
        assert_eq!(
            status.diagnostic,
            "runtime projection `az.test.composite-project`: project projection launched\nruntime projection `az.test.composite-gem`: gem projection launched"
        );
        let frame = host
            .viewport_frame(&RuntimeViewportRequest {
                capability: capability(RUNTIME_READ_PERMISSION),
                runtime_id: "composite-runtime".to_string(),
            })
            .unwrap()
            .frame
            .unwrap();
        assert_eq!(frame.width, 96);
        assert_eq!(frame.height, 54);

        let stopped = host
            .stop(&StopRuntimeRequest {
                capability: capability(RUNTIME_CONTROL_PERMISSION),
                runtime_id: "composite-runtime".to_string(),
                preserve: false,
            })
            .unwrap();
        assert_eq!(stopped.state, RuntimeState::Stopped);
        assert_eq!(
            stopped.diagnostic,
            "runtime projection `az.test.composite-project`: project projection stopped\nruntime projection `az.test.composite-gem`: gem projection stopped"
        );
    }

    #[test]
    fn runtime_host_rejects_role_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let host = runtime_host();
        let error = host
            .launch(&LaunchRuntimeRequest {
                capability: capability(RUNTIME_CONTROL_PERMISSION),
                runtime_id: "editor-world".to_string(),
                role: RuntimeRole::EditorWorld,
                snapshot: snapshot_handle(&temp, RuntimeRole::PlayPreview),
            })
            .unwrap_err();

        assert!(matches!(error, RuntimeHostError::RoleMismatch { .. }));
    }

    #[test]
    fn runtime_host_rejects_cross_session_launch_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let session = Uuid::from_bytes([0x88; 16]);
        let other_session = Uuid::from_bytes([0x89; 16]);
        let host = runtime_host_for_session(session);

        let error = host
            .launch(&LaunchRuntimeRequest {
                capability: scoped_capability(RUNTIME_CONTROL_PERMISSION, session),
                runtime_id: "editor-world".to_string(),
                role: RuntimeRole::EditorWorld,
                snapshot: snapshot_handle_with_profile_and_session(
                    &temp,
                    RuntimeRole::EditorWorld,
                    "editor",
                    other_session,
                ),
            })
            .unwrap_err();

        assert!(matches!(
            error,
            RuntimeHostError::InvalidCapability { .. }
                | RuntimeHostError::SnapshotSessionMismatch { .. }
                | RuntimeHostError::SnapshotServiceSessionMismatch { .. }
        ));

        let status = host
            .launch(&LaunchRuntimeRequest {
                capability: scoped_capability(RUNTIME_CONTROL_PERMISSION, session),
                runtime_id: "editor-world".to_string(),
                role: RuntimeRole::EditorWorld,
                snapshot: snapshot_handle_with_profile_and_session(
                    &temp,
                    RuntimeRole::EditorWorld,
                    "editor",
                    session,
                ),
            })
            .unwrap();
        assert_eq!(status.state, RuntimeState::Running);
        assert_eq!(
            host.launched_snapshot("editor-world").unwrap().session_id,
            session
        );
    }

    #[test]
    fn runtime_host_rejects_expired_capabilities() {
        let host = runtime_host();
        let error = host
            .status(&RuntimeStatusRequest {
                capability: capability(RUNTIME_READ_PERMISSION).with_expires_unix_ms(1),
                runtime_id: "editor-world".to_string(),
            })
            .unwrap_err();

        assert!(matches!(error, RuntimeHostError::InvalidCapability { .. }));
    }

    #[test]
    fn runtime_host_session_bound_capabilities_require_matching_session() {
        let session = Uuid::from_bytes([0x33; 16]);
        let other_session = Uuid::from_bytes([0x44; 16]);
        let host = runtime_host_for_session(session);

        let unscoped = host
            .status(&RuntimeStatusRequest {
                capability: unscoped_capability(RUNTIME_READ_PERMISSION),
                runtime_id: "editor-world".to_string(),
            })
            .unwrap_err();
        assert!(matches!(
            unscoped,
            RuntimeHostError::InvalidCapability { .. }
        ));

        let wrong_session = host
            .status(&RuntimeStatusRequest {
                capability: scoped_capability(RUNTIME_READ_PERMISSION, other_session),
                runtime_id: "editor-world".to_string(),
            })
            .unwrap_err();
        assert!(matches!(
            wrong_session,
            RuntimeHostError::InvalidCapability { .. }
        ));

        let scoped = host
            .status(&RuntimeStatusRequest {
                capability: scoped_capability(RUNTIME_READ_PERMISSION, session),
                runtime_id: "editor-world".to_string(),
            })
            .unwrap();
        assert_eq!(scoped.status, None);
    }

    #[test]
    fn runtime_host_session_bound_capabilities_require_brokered_grant_token() {
        let session = Uuid::from_bytes([0x55; 16]);
        let valid = scoped_capability(RUNTIME_READ_PERMISSION, session).with_token_hash([0x66]);
        let grants = CapabilityGrantSet::from_grants(vec![valid.clone()]);
        let host = RuntimeHost::new(
            session.to_string(),
            grants,
            std::env::temp_dir().join(format!("az-runtime-host-test-{session}")),
            composed(),
        );

        host.status(&RuntimeStatusRequest {
            capability: valid.clone(),
            runtime_id: "editor-world".to_string(),
        })
        .unwrap();

        let mut synthetic = valid.clone();
        synthetic.token_hash.clear();
        assert!(matches!(
            host.status(&RuntimeStatusRequest {
                capability: synthetic,
                runtime_id: "editor-world".to_string(),
            }),
            Err(RuntimeHostError::InvalidCapability { .. })
        ));

        let mut wrong_token = valid;
        wrong_token.token_hash = vec![0x99];
        assert!(matches!(
            host.status(&RuntimeStatusRequest {
                capability: wrong_token,
                runtime_id: "editor-world".to_string(),
            }),
            Err(RuntimeHostError::InvalidCapability { .. })
        ));
    }

    #[test]
    fn runtime_host_rpc_routes_launch_status_and_stop() {
        let temp = tempfile::tempdir().unwrap();
        let rpc = Rc::new(RuntimeHostRpc::new(runtime_host()));
        let client = RuntimeHostRpc::client_from_rc(&rpc);

        let mut launch = client.launch_request();
        (LaunchRuntimeRequest {
            capability: capability(RUNTIME_CONTROL_PERMISSION),
            runtime_id: "editor-world".to_string(),
            role: RuntimeRole::EditorWorld,
            snapshot: snapshot_handle(&temp, RuntimeRole::EditorWorld),
        })
        .to_capnp(launch.get().init_request())
        .unwrap();
        let launch_response = executor::block_on(launch.send().promise).unwrap();
        let launched =
            RuntimeStatus::from_capnp(launch_response.get().unwrap().get_status().unwrap())
                .unwrap();
        assert_eq!(launched.state, RuntimeState::Running);

        let mut status = client.status_request();
        (RuntimeStatusRequest {
            capability: capability(RUNTIME_READ_PERMISSION),
            runtime_id: "editor-world".to_string(),
        })
        .to_capnp(status.get().init_request())
        .unwrap();
        let status_response = executor::block_on(status.send().promise).unwrap();
        let queried =
            RuntimeStatusResult::from_capnp(status_response.get().unwrap().get_result().unwrap())
                .unwrap();
        assert_eq!(queried.status, Some(launched));

        let mut expected_frame = rpc.host().publish_viewport_frame(viewport_frame()).unwrap();
        let viewport_capability = capability(RUNTIME_READ_PERMISSION);
        let mut viewport = client.viewport_frame_request();
        (RuntimeViewportRequest {
            capability: viewport_capability.clone(),
            runtime_id: "editor-world".to_string(),
        })
        .to_capnp(viewport.get().init_request())
        .unwrap();
        let viewport_response = executor::block_on(viewport.send().promise).unwrap();
        let viewport_result = RuntimeViewportResult::from_capnp(
            viewport_response.get().unwrap().get_result().unwrap(),
        )
        .unwrap();
        expected_frame.color.capability = Some(viewport_capability);
        assert_eq!(viewport_result.frame, Some(expected_frame));

        let mut catalog = client.projection_catalog_request();
        (RuntimeProjectionCatalogRequest {
            capability: capability(RUNTIME_READ_PERMISSION),
        })
        .to_capnp(catalog.get().init_request())
        .unwrap();
        let catalog_response = executor::block_on(catalog.send().promise).unwrap();
        let catalog = RuntimeProjectionCatalogResult::from_capnp(
            catalog_response.get().unwrap().get_result().unwrap(),
        )
        .unwrap();
        assert!(
            catalog
                .projections
                .iter()
                .any(|projection| projection.name == "az.test.registered-runtime-projection")
        );

        let mut stop = client.stop_request();
        (StopRuntimeRequest {
            capability: capability(RUNTIME_CONTROL_PERMISSION),
            runtime_id: "editor-world".to_string(),
            preserve: true,
        })
        .to_capnp(stop.get().init_request())
        .unwrap();
        let stop_response = executor::block_on(stop.send().promise).unwrap();
        let stopped =
            RuntimeStatus::from_capnp(stop_response.get().unwrap().get_status().unwrap()).unwrap();
        assert_eq!(stopped.state, RuntimeState::Stopped);
        assert_eq!(stopped.diagnostic, "stopped and preserved");
    }
}
