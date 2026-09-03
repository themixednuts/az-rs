#[cfg(any(test, feature = "test-support"))]
use std::path::Path;
use std::path::{Component, PathBuf};
use std::thread;

use az_gem_contract::{Composer, ProcessCompositionCleanupError};
use az_proto_asset::{
    ASSET_PROCESSOR_AUDIENCE, ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME,
    ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION, asset_capnp,
};
use az_proto_core::{
    Capability, CapabilityGrantRequirement, CapabilityGrantSet, CapabilityGrantSetValidationError,
    EDITOR_SERVICE_NAME, EDITOR_SERVICE_NAMESPACE, Endpoint, EndpointKind, ProtocolVersion,
    ServiceHealth, ServiceId, ServiceRole, decode_capability_grant_set,
};
use az_proto_project::{
    PROJECT_DOCUMENT_READ_PERMISSION, PROJECT_DOCUMENT_WRITE_PERMISSION, PROJECT_EDIT_PERMISSION,
    PROJECT_GAMEDATA_PERMISSION, PROJECT_GRAPH_CATALOG_PERMISSION, PROJECT_HOST_AUDIENCE,
    PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME, PROJECT_INVENTORY_PERMISSION,
    PROJECT_NODE_CATALOG_PERMISSION, PROJECT_RUNTIME_LAUNCH_PERMISSION, PROJECT_SCHEMA_PERMISSION,
    PROJECT_SOURCE_NAVIGATION_PERMISSION, project_capnp,
};
use az_proto_session::{SESSION_SUPERVISOR_NAMESPACE, SESSION_SUPERVISOR_SERVICE_NAME};
use az_rpc::AzRpcTransportError;
use futures::future::Either;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::oneshot;
use tokio::task::LocalSet;
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    Composition, ProjectHost, ProjectHostError, ProjectHostRpc, SourceAuthoringClient,
    SourceAuthoringRpcClient,
};

#[derive(Debug, Error)]
pub enum ProjectHostRpcTransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Rpc(#[from] AzRpcTransportError),

    // Boxed: `ProjectHostError` is 120 bytes on its own, which is what pushed
    // every `Result<_, ProjectHostRpcTransportError>` in this module over
    // clippy's 128-byte `result_large_err` threshold.
    #[error("project-host error: {0}")]
    ProjectHost(#[source] Box<ProjectHostError>),

    #[error("capability grant decode error: {0}")]
    CapabilityGrantDecode(#[from] capnp::Error),

    #[error("project-host RPC endpoint requires at least one brokered capability grant")]
    EmptyCapabilityGrantSet,

    // Boxed for the same reason: `CapabilityGrantSetValidationError` is itself
    // 128 bytes.
    #[error("invalid project-host RPC capability grant set: {0}")]
    InvalidCapabilityGrantSet(#[source] Box<CapabilityGrantSetValidationError>),

    #[error("endpoint kind `{0:?}` is not supported on this platform")]
    UnsupportedEndpoint(EndpointKind),

    #[error("project-host side-channel root `{root:?}` must be absolute")]
    SideChannelRootNotAbsolute { root: PathBuf },

    #[error("project-host side-channel root `{root:?}` must not contain `.` or `..`")]
    SideChannelRootNotNormal { root: PathBuf },

    #[error("failed to create project-host side-channel root {root:?}: {source}")]
    SideChannelRootCreate {
        root: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to canonicalize project-host side-channel root {root:?}: {source}")]
    SideChannelRootCanonicalize {
        root: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("project-host RPC server thread failed to start")]
    StartupChannelClosed,

    #[error("project-host capability grants lack the Asset Processor authoring capability")]
    MissingAssetProcessorAuthoringCapability,

    #[error("project-host service run must be a non-nil UUID")]
    NilRun,

    #[error("Asset Processor health RPC failed: {0}")]
    AssetProcessorHealthRpc(#[source] capnp::Error),

    #[error("Asset Processor health is invalid: {reason}")]
    InvalidAssetProcessorHealth { reason: String },
}

// Hand-written because `#[from]` on the boxed field would derive
// `From<Box<ProjectHostError>>` and silently stop `?` from converting a bare
// `ProjectHostError`.
impl From<ProjectHostError> for ProjectHostRpcTransportError {
    fn from(error: ProjectHostError) -> Self {
        Self::ProjectHost(Box::new(error))
    }
}

// Same reason as the `ProjectHostError` conversion above.
impl From<CapabilityGrantSetValidationError> for ProjectHostRpcTransportError {
    fn from(error: CapabilityGrantSetValidationError) -> Self {
        Self::InvalidCapabilityGrantSet(Box::new(error))
    }
}

#[derive(Debug, Error)]
pub enum ProjectHostRpcShutdownFailure {
    #[error("could not close composition lease issuance: {0}")]
    BeginShutdown(#[source] ProcessCompositionCleanupError),
    #[error("project-host RPC listener thread panicked")]
    ListenerThreadPanicked,
    #[error("could not clean the project-host composition: {0}")]
    Cleanup(#[source] ProcessCompositionCleanupError),
}

#[derive(Debug, Error)]
#[error("project-host RPC shutdown had failures: {failures:?}")]
pub struct ProjectHostRpcShutdownError {
    pub failures: Vec<ProjectHostRpcShutdownFailure>,
}

pub struct ProjectHostRpcServer {
    endpoint: Endpoint,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
    composition: Option<Composition>,
}

impl ProjectHostRpcServer {
    #[must_use]
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Stop the listener thread and clean the composition it served.
    ///
    /// # Errors
    ///
    /// Returns a [`ProjectHostRpcShutdownError`] collecting every step that
    /// failed: [`ProjectHostRpcShutdownFailure::BeginShutdown`] when lease
    /// issuance cannot be closed, [`ProjectHostRpcShutdownFailure::
    /// ListenerThreadPanicked`] when the RPC thread did not unwind cleanly, and
    /// [`ProjectHostRpcShutdownFailure::Cleanup`] when the composition cannot
    /// be cleaned. Shutdown always runs all three steps; the error only reports
    /// them.
    pub fn stop(mut self) -> Result<(), ProjectHostRpcShutdownError> {
        let mut failures = Vec::new();
        if let Some(composition) = self.composition.as_mut()
            && let Err(error) = composition.begin_shutdown()
        {
            failures.push(ProjectHostRpcShutdownFailure::BeginShutdown(error));
        }
        self.shutdown();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            failures.push(ProjectHostRpcShutdownFailure::ListenerThreadPanicked);
        }
        if let Some(mut composition) = self.composition.take()
            && let Err(error) = composition.cleanup()
        {
            failures.push(ProjectHostRpcShutdownFailure::Cleanup(error));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(ProjectHostRpcShutdownError { failures })
        }
    }

    fn shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

impl Drop for ProjectHostRpcServer {
    fn drop(&mut self) {
        // The RPC thread owns registry leases, while this handle owns the
        // contribution lifecycle. Join first so no request can still read a
        // registry when finish/cleanup runs.
        if let Some(composition) = self.composition.as_mut() {
            let _ = composition.begin_shutdown();
        }
        self.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some(mut composition) = self.composition.take() {
            let _ = composition.cleanup();
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProjectHostServiceStartup {
    pub source_root: PathBuf,
    pub side_channel_root: PathBuf,
    pub endpoint: Endpoint,
    pub asset_processor_endpoint: Endpoint,
    pub project_id: String,
    pub run: Uuid,
    pub capability_grants_file: PathBuf,
}

#[derive(Debug, Clone)]
struct SourceAuthoringTransport {
    endpoint: Endpoint,
    capability: Capability,
}

struct ProjectHostServerBootstrap {
    host: ProjectHost,
    side_channel_root: PathBuf,
    project_id: Option<String>,
    session_id: Option<String>,
    run: Uuid,
    capability_grants: CapabilityGrantSet,
    source_authoring: Option<SourceAuthoringTransport>,
    composition: crate::CompositionView,
}

impl ProjectHostServerBootstrap {
    // Builds and holds a capnp-rpc client across awaits; capnp-rpc keeps its
    // connection state behind `Rc<RefCell<..>>`, so this future can never be
    // `Send`.
    #[allow(clippy::future_not_send)]
    async fn into_client(
        self,
    ) -> Result<project_capnp::project_host::Client, ProjectHostRpcTransportError> {
        let source_authoring: Box<dyn SourceAuthoringClient> = match self.source_authoring {
            Some(source_authoring) => {
                let client: asset_capnp::asset_processor::Client =
                    az_rpc::connect_twoparty_bootstrap(&source_authoring.endpoint).await?;
                validate_asset_processor_health(&client).await?;
                Box::new(SourceAuthoringRpcClient::new(
                    client,
                    source_authoring.capability,
                ))
            }
            None => {
                #[cfg(any(test, feature = "test-support"))]
                {
                    Box::new(crate::UnavailableSourceAuthoringClient)
                }
                #[cfg(not(any(test, feature = "test-support")))]
                {
                    return Err(
                        ProjectHostRpcTransportError::MissingAssetProcessorAuthoringCapability,
                    );
                }
            }
        };
        let rpc = match (self.project_id, self.session_id) {
            (Some(project_id), Some(session_id)) => ProjectHostRpc::for_project_session(
                self.host,
                self.side_channel_root,
                project_id,
                session_id,
                self.capability_grants,
                self.composition,
                source_authoring,
            ),
            (Some(project_id), None) => ProjectHostRpc::for_project(
                self.host,
                self.side_channel_root,
                project_id,
                self.capability_grants,
                self.composition,
                source_authoring,
            ),
            (None, Some(session_id)) => ProjectHostRpc::for_session(
                self.host,
                self.side_channel_root,
                session_id,
                self.capability_grants,
                self.composition,
                source_authoring,
            ),
            (None, None) => ProjectHostRpc::new(
                self.host,
                self.side_channel_root,
                self.capability_grants,
                self.composition,
                source_authoring,
            ),
        };
        Ok(rpc.with_service_run(self.run).into_client())
    }
}

/// Compose, validate, and start the production project-host RPC service.
///
/// # Errors
///
/// Returns [`ProjectHostRpcTransportError::ProjectHost`] when `composer` does
/// not finalize into a ready `ProjectHost` composition or the source root does
/// not open as `startup.project_id`,
/// [`ProjectHostRpcTransportError::NilRun`] for a nil service run,
/// [`ProjectHostRpcTransportError::Io`] when the capability-grant file cannot
/// be read, [`ProjectHostRpcTransportError::CapabilityGrantDecode`] when it
/// does not decode, [`ProjectHostRpcTransportError::InvalidCapabilityGrantSet`]
/// when the grants are not exactly the brokered set this host requires,
/// [`ProjectHostRpcTransportError::MissingAssetProcessorAuthoringCapability`]
/// when no grant carries the Asset Processor authoring capability, the
/// `SideChannelRoot*` variants when the side-channel root is not an absolute
/// normal directory that can be created and canonicalized, and
/// [`ProjectHostRpcTransportError::UnsupportedEndpoint`] or
/// [`ProjectHostRpcTransportError::StartupChannelClosed`] when the listener
/// cannot be brought up on `startup.endpoint`.
pub fn start_project_host_service(
    startup: ProjectHostServiceStartup,
    composer: Composer,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    let composition = Composition::new(composer)?;
    if startup.run == Uuid::nil() {
        return Err(ProjectHostRpcTransportError::NilRun);
    }
    let bytes = std::fs::read(&startup.capability_grants_file)?;
    let capability_grants = decode_capability_grant_set(&bytes)?;
    validate_project_host_rpc_capability_grants(&capability_grants, true)?;
    let source_authoring =
        source_authoring_transport(&capability_grants, startup.asset_processor_endpoint)?;
    let side_channel_root = normalize_project_host_side_channel_root(startup.side_channel_root)?;
    start_project_host_rpc_server_with_host_and_session(ProjectHostRpcServerSpec {
        host: ProjectHost::open_project_source_root(&startup.source_root, &startup.project_id)?,
        side_channel_root,
        endpoint: startup.endpoint,
        project_id: Some(startup.project_id),
        session_id: None,
        run: startup.run,
        capability_grants,
        source_authoring: Some(source_authoring),
        composition,
    })
}

/// Start a test project-host server over an already-decoded grant set.
///
/// # Errors
///
/// Returns [`ProjectHostRpcTransportError::ProjectHost`] when `source_root`
/// does not open, [`ProjectHostRpcTransportError::EmptyCapabilityGrantSet`]
/// when `capability_grants` is empty, the `SideChannelRoot*` variants when the
/// default side-channel root cannot be prepared, and
/// [`ProjectHostRpcTransportError::UnsupportedEndpoint`] or
/// [`ProjectHostRpcTransportError::StartupChannelClosed`] when the listener
/// cannot be brought up on `endpoint`.
#[cfg(any(test, feature = "test-support"))]
pub fn start_project_host_rpc_server_with_capability_grants(
    source_root: impl AsRef<Path>,
    endpoint: Endpoint,
    capability_grants: CapabilityGrantSet,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    start_project_host_rpc_server_with_host_and_session(ProjectHostRpcServerSpec {
        host: ProjectHost::open_source_root(source_root.as_ref())?,
        side_channel_root: default_project_host_side_channel_root(),
        endpoint,
        project_id: None,
        session_id: None,
        run: Uuid::now_v7(),
        capability_grants,
        source_authoring: None,
        composition: crate::test_project_host_composition(),
    })
}

/// Start a session-scoped test server, reading grants from a file.
///
/// # Errors
///
/// Returns [`ProjectHostRpcTransportError::Io`] when
/// `capability_grants_file` cannot be read,
/// [`ProjectHostRpcTransportError::CapabilityGrantDecode`] when it does not
/// decode, and any error
/// [`start_project_host_rpc_server_with_capability_grants`] returns for the
/// same source root and endpoint.
#[cfg(any(test, feature = "test-support"))]
pub fn start_project_host_rpc_server_for_session_with_capability_grant_file(
    source_root: impl AsRef<Path>,
    endpoint: Endpoint,
    session_id: impl Into<String>,
    capability_grants_file: impl AsRef<Path>,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    let bytes = std::fs::read(capability_grants_file.as_ref())?;
    let capability_grants = decode_capability_grant_set(&bytes)?;
    start_project_host_rpc_server_with_host_and_session(ProjectHostRpcServerSpec {
        host: ProjectHost::open_source_root(source_root.as_ref())?,
        side_channel_root: default_project_host_side_channel_root(),
        endpoint,
        project_id: None,
        session_id: Some(session_id.into()),
        run: Uuid::now_v7(),
        capability_grants,
        source_authoring: None,
        composition: crate::test_project_host_composition(),
    })
}

/// Start a project- and session-scoped test server, reading grants from a file.
///
/// # Errors
///
/// Returns [`ProjectHostRpcTransportError::Io`] when
/// `capability_grants_file` cannot be read,
/// [`ProjectHostRpcTransportError::CapabilityGrantDecode`] when it does not
/// decode, [`ProjectHostRpcTransportError::InvalidCapabilityGrantSet`] when the
/// grants are not exactly the brokered editor and session-supervisor set,
/// [`ProjectHostRpcTransportError::ProjectHost`] when `source_root` does not
/// open as `project_id`, and the listener-startup variants
/// [`ProjectHostRpcTransportError::UnsupportedEndpoint`] and
/// [`ProjectHostRpcTransportError::StartupChannelClosed`].
#[cfg(any(test, feature = "test-support"))]
pub fn start_project_host_rpc_server_for_project_session_with_capability_grant_file(
    source_root: impl AsRef<Path>,
    endpoint: Endpoint,
    project_id: impl Into<String>,
    session_id: impl Into<String>,
    capability_grants_file: impl AsRef<Path>,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    let bytes = std::fs::read(capability_grants_file.as_ref())?;
    let capability_grants = decode_capability_grant_set(&bytes)?;
    let project_id = project_id.into();
    let session_id = session_id.into();
    validate_project_host_rpc_capability_grants(&capability_grants, false)?;
    start_project_host_rpc_server_with_host_and_session(ProjectHostRpcServerSpec {
        host: ProjectHost::open_project_source_root(source_root.as_ref(), &project_id)?,
        side_channel_root: default_project_host_side_channel_root(),
        endpoint,
        project_id: Some(project_id),
        session_id: Some(session_id),
        run: Uuid::now_v7(),
        capability_grants,
        source_authoring: None,
        composition: crate::test_project_host_composition(),
    })
}

/// Start a session-scoped test server over an already-decoded grant set.
///
/// # Errors
///
/// Returns the same errors as
/// [`start_project_host_rpc_server_with_capability_grants`]: a source root that
/// does not open, an empty grant set, a side-channel root that cannot be
/// prepared, or a listener that cannot be brought up on `endpoint`.
#[cfg(any(test, feature = "test-support"))]
pub fn start_project_host_rpc_server_for_session_with_capability_grants(
    source_root: impl AsRef<Path>,
    endpoint: Endpoint,
    session_id: impl Into<String>,
    capability_grants: CapabilityGrantSet,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    start_project_host_rpc_server_with_host_and_session(ProjectHostRpcServerSpec {
        host: ProjectHost::open_source_root(source_root.as_ref())?,
        side_channel_root: default_project_host_side_channel_root(),
        endpoint,
        project_id: None,
        session_id: Some(session_id.into()),
        run: Uuid::now_v7(),
        capability_grants,
        source_authoring: None,
        composition: crate::test_project_host_composition(),
    })
}

/// Everything one project-host RPC server is brought up from.
///
/// These are not independent knobs. The grant set decides which service
/// identity — `project_id`, `session_id`, `run` — the host is allowed to
/// answer as and whether an authoring transport must accompany it; the
/// composition supplies the contributions the opened `host` serves; the
/// endpoint and side-channel root are where that one host becomes reachable.
/// The production service entry point and every test constructor fill in the
/// same set and hand it over whole, so it is one value rather than nine
/// arguments that must be kept in the right order.
struct ProjectHostRpcServerSpec {
    /// The opened project host the server serves.
    host: ProjectHost,
    /// Directory holding the server's side-channel files.
    side_channel_root: PathBuf,
    /// Endpoint the listener is brought up on.
    endpoint: Endpoint,
    /// Project this host answers for, when it is project-scoped.
    project_id: Option<String>,
    /// Session this host answers for, when it is session-scoped.
    session_id: Option<String>,
    /// Service run id stamped onto issued leases.
    run: Uuid,
    /// Brokered grants the server validates callers against.
    capability_grants: CapabilityGrantSet,
    /// Asset Processor authoring transport, when one was brokered.
    source_authoring: Option<SourceAuthoringTransport>,
    /// Gem composition whose lifetime the returned server owns.
    composition: Composition,
}

fn start_project_host_rpc_server_with_host_and_session(
    spec: ProjectHostRpcServerSpec,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    let ProjectHostRpcServerSpec {
        host,
        side_channel_root,
        endpoint,
        project_id,
        session_id,
        run,
        capability_grants,
        source_authoring,
        composition,
    } = spec;
    if capability_grants.is_empty() {
        return Err(ProjectHostRpcTransportError::EmptyCapabilityGrantSet);
    }
    let side_channel_root = normalize_project_host_side_channel_root(side_channel_root)?;
    let bootstrap = ProjectHostServerBootstrap {
        host,
        side_channel_root,
        project_id,
        session_id,
        run,
        capability_grants,
        source_authoring,
        composition: composition
            .rpc_view()
            .map_err(ProjectHostError::from)
            .map_err(ProjectHostRpcTransportError::from)?,
    };

    let mut server = match endpoint.kind {
        EndpointKind::Tcp => start_tcp_server(bootstrap, &endpoint),
        EndpointKind::WindowsNamedPipe => start_named_pipe_server(bootstrap, endpoint),
        EndpointKind::UnixDomainSocket => start_unix_socket_server(bootstrap, endpoint),
        EndpointKind::InProcess => Err(ProjectHostRpcTransportError::UnsupportedEndpoint(
            EndpointKind::InProcess,
        )),
    }?;
    server.composition = Some(composition);
    Ok(server)
}

fn normalize_project_host_side_channel_root(
    root: PathBuf,
) -> Result<PathBuf, ProjectHostRpcTransportError> {
    if !root.is_absolute() {
        return Err(ProjectHostRpcTransportError::SideChannelRootNotAbsolute { root });
    }
    if root
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(ProjectHostRpcTransportError::SideChannelRootNotNormal { root });
    }
    std::fs::create_dir_all(&root).map_err(|source| {
        ProjectHostRpcTransportError::SideChannelRootCreate {
            root: root.clone(),
            source,
        }
    })?;
    std::fs::canonicalize(&root).map_err(|source| {
        ProjectHostRpcTransportError::SideChannelRootCanonicalize { root, source }
    })
}

const EDITOR_PROJECT_HOST_PERMISSIONS: &[&str] = &[
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
];
const SESSION_SUPERVISOR_PROJECT_HOST_PERMISSIONS: &[&str] = &[PROJECT_DOCUMENT_WRITE_PERMISSION];

fn validate_project_host_rpc_capability_grants(
    capability_grants: &CapabilityGrantSet,
    require_asset_processor_authoring: bool,
) -> Result<(), ProjectHostRpcTransportError> {
    let mut requirements = vec![
        CapabilityGrantRequirement::new(
            EDITOR_SERVICE_NAMESPACE,
            EDITOR_SERVICE_NAME,
            ServiceRole::Editor,
            PROJECT_HOST_AUDIENCE,
            EDITOR_PROJECT_HOST_PERMISSIONS,
        ),
        CapabilityGrantRequirement::new(
            SESSION_SUPERVISOR_NAMESPACE,
            SESSION_SUPERVISOR_SERVICE_NAME,
            ServiceRole::SessionSupervisor,
            PROJECT_HOST_AUDIENCE,
            SESSION_SUPERVISOR_PROJECT_HOST_PERMISSIONS,
        ),
    ];
    if require_asset_processor_authoring {
        requirements.push(CapabilityGrantRequirement::new(
            PROJECT_HOST_NAMESPACE,
            PROJECT_HOST_SERVICE_NAME,
            ServiceRole::ProjectHost,
            ASSET_PROCESSOR_AUDIENCE,
            &[ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION],
        ));
    }
    capability_grants.validate_exact_brokered_for_project(&requirements)?;
    Ok(())
}

fn source_authoring_transport(
    capability_grants: &CapabilityGrantSet,
    endpoint: Endpoint,
) -> Result<SourceAuthoringTransport, ProjectHostRpcTransportError> {
    let capability = capability_grants
        .grants()
        .iter()
        .find(|capability| {
            capability.service.namespace == PROJECT_HOST_NAMESPACE
                && capability.service.name == PROJECT_HOST_SERVICE_NAME
                && capability.role == ServiceRole::ProjectHost
                && capability.audience == ASSET_PROCESSOR_AUDIENCE
                && capability.has_permissions(&[ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION])
        })
        .cloned()
        .ok_or(ProjectHostRpcTransportError::MissingAssetProcessorAuthoringCapability)?;
    Ok(SourceAuthoringTransport {
        endpoint,
        capability,
    })
}

// Holds a capnp-rpc client across awaits; capnp-rpc keeps its connection state
// behind `Rc<RefCell<..>>`, so this future can never be `Send`.
#[allow(clippy::future_not_send)]
async fn validate_asset_processor_health(
    client: &asset_capnp::asset_processor::Client,
) -> Result<(), ProjectHostRpcTransportError> {
    let response = client
        .health_request()
        .send()
        .promise
        .await
        .map_err(ProjectHostRpcTransportError::AssetProcessorHealthRpc)?;
    let health = ServiceHealth::from_capnp(
        response
            .get()
            .map_err(ProjectHostRpcTransportError::AssetProcessorHealthRpc)?
            .get_health()
            .map_err(ProjectHostRpcTransportError::AssetProcessorHealthRpc)?,
    )
    .map_err(ProjectHostRpcTransportError::AssetProcessorHealthRpc)?;
    let expected_service = ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME);
    if health.service != expected_service || health.role != ServiceRole::AssetProcessor {
        return Err(ProjectHostRpcTransportError::InvalidAssetProcessorHealth {
            reason: format!(
                "service `{}`/`{}` role {:?} does not identify `{}`/`{}` role AssetProcessor",
                health.service.namespace,
                health.service.name,
                health.role,
                expected_service.namespace,
                expected_service.name,
            ),
        });
    }
    health
        .require_protocol_version(ProtocolVersion::CURRENT)
        .map_err(
            |error| ProjectHostRpcTransportError::InvalidAssetProcessorHealth {
                reason: error.to_string(),
            },
        )?;
    if !health.ready {
        return Err(ProjectHostRpcTransportError::InvalidAssetProcessorHealth {
            reason: format!("service state {:?} is not ready", health.state),
        });
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
fn default_project_host_side_channel_root() -> PathBuf {
    std::env::temp_dir().join("azoth").join("project-host")
}

/// Connect to a running project-host RPC server and return its bootstrap client.
///
/// # Errors
///
/// Returns [`ProjectHostRpcTransportError::Rpc`] when the two-party connection
/// to `endpoint` cannot be established — an unreachable or unsupported address,
/// or a transport that fails during the bootstrap handshake.
pub async fn connect_project_host_rpc_client(
    endpoint: &Endpoint,
) -> Result<project_capnp::project_host::Client, ProjectHostRpcTransportError> {
    Ok(az_rpc::connect_twoparty_bootstrap(endpoint).await?)
}

type StartupSender = std::sync::mpsc::SyncSender<Result<(), ProjectHostRpcTransportError>>;
type StartupReceiver = std::sync::mpsc::Receiver<Result<(), ProjectHostRpcTransportError>>;

fn start_tcp_server(
    bootstrap: ProjectHostServerBootstrap,
    endpoint: &Endpoint,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    let listener = std::net::TcpListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?.to_string();
    let endpoint = Endpoint::new(EndpointKind::Tcp, address);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let listener = TcpListener::from_std(listener)?;
            run_tcp_listener(
                bootstrap,
                thread_endpoint,
                listener,
                shutdown_rx,
                startup_tx,
            )
            .await
        });
    });
    await_server_start(endpoint, shutdown_tx, thread, &startup_rx)
}

// Holds a capnp-rpc client across awaits; capnp-rpc keeps its connection state
// behind `Rc<RefCell<..>>`, so this future can never be `Send`.
#[allow(clippy::future_not_send)]
async fn run_tcp_listener(
    bootstrap: ProjectHostServerBootstrap,
    endpoint: Endpoint,
    listener: TcpListener,
    mut shutdown: oneshot::Receiver<()>,
    startup: StartupSender,
) -> Result<(), ProjectHostRpcTransportError> {
    let Some(client) = admit_project_host(bootstrap, startup).await else {
        return Ok(());
    };
    info!(endpoint = %endpoint.address, "project-host RPC listener started");

    loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut shutdown).await {
            Either::Left((Ok((stream, peer)), _)) => {
                info!(endpoint = %endpoint.address, peer = %peer, "project-host RPC client connected");
                // Dropping the join handle detaches the connection task, which
                // is what serving many clients from one listener wants.
                drop(az_rpc::spawn_twoparty_server(stream, client.client.clone()));
            }
            Either::Left((Err(error), _)) => return Err(error.into()),
            Either::Right((_, _)) => return Ok(()),
        }
    }
}

// Produces a capnp-rpc client; capnp-rpc keeps its connection state behind
// `Rc<RefCell<..>>`, so this future can never be `Send`.
#[allow(clippy::future_not_send)]
async fn admit_project_host(
    bootstrap: ProjectHostServerBootstrap,
    startup: StartupSender,
) -> Option<project_capnp::project_host::Client> {
    match bootstrap.into_client().await {
        Ok(client) => startup.send(Ok(())).ok().map(|()| client),
        Err(error) => {
            let _ = startup.send(Err(error));
            None
        }
    }
}

fn await_server_start(
    endpoint: Endpoint,
    shutdown: oneshot::Sender<()>,
    thread: thread::JoinHandle<()>,
    startup: &StartupReceiver,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    match startup.recv() {
        Ok(Ok(())) => Ok(ProjectHostRpcServer {
            endpoint,
            shutdown: Some(shutdown),
            thread: Some(thread),
            composition: None,
        }),
        Ok(Err(error)) => {
            drop(shutdown);
            let _ = thread.join();
            Err(error)
        }
        Err(_) => {
            drop(shutdown);
            let _ = thread.join();
            Err(ProjectHostRpcTransportError::StartupChannelClosed)
        }
    }
}

fn run_threaded_local<F>(future: F)
where
    F: std::future::Future<Output = Result<(), ProjectHostRpcTransportError>> + 'static,
{
    let runtime = match Builder::new_current_thread().enable_io().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            error!(error = %error, "project-host RPC runtime failed to start");
            return;
        }
    };
    let local = LocalSet::new();
    let result = runtime.block_on(local.run_until(future));
    if let Err(error) = result {
        error!(error = %error, "project-host RPC listener stopped with error");
    }
}

#[cfg(windows)]
fn start_named_pipe_server(
    bootstrap: ProjectHostServerBootstrap,
    endpoint: Endpoint,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let server_result = az_rpc::create_owner_only_named_pipe(
                ServerOptions::new().first_pipe_instance(true),
                &thread_endpoint.address,
            );
            let server = match server_result {
                Ok(server) => server,
                Err(error) => {
                    let _ = startup_tx.send(Err(error.into()));
                    return Ok(());
                }
            };
            run_named_pipe_listener(bootstrap, thread_endpoint, server, shutdown_rx, startup_tx)
                .await
        });
    });
    await_server_start(endpoint, shutdown_tx, thread, &startup_rx)
}

// Signature is fixed by the `#[cfg(windows)]` twin above, which does consume
// the endpoint.
#[allow(clippy::needless_pass_by_value)]
#[cfg(not(windows))]
fn start_named_pipe_server(
    _bootstrap: ProjectHostServerBootstrap,
    endpoint: Endpoint,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    Err(ProjectHostRpcTransportError::UnsupportedEndpoint(
        endpoint.kind,
    ))
}

// Holds a capnp-rpc client across awaits; capnp-rpc keeps its connection state
// behind `Rc<RefCell<..>>`, so this future can never be `Send`.
#[allow(clippy::future_not_send)]
#[cfg(windows)]
async fn run_named_pipe_listener(
    bootstrap: ProjectHostServerBootstrap,
    endpoint: Endpoint,
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    mut shutdown: oneshot::Receiver<()>,
    startup: StartupSender,
) -> Result<(), ProjectHostRpcTransportError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let Some(client) = admit_project_host(bootstrap, startup).await else {
        return Ok(());
    };
    info!(endpoint = %endpoint.address, "project-host named-pipe RPC listener started");

    loop {
        let connected = {
            let connect = Box::pin(server.connect());
            match futures::future::select(connect, &mut shutdown).await {
                Either::Left((Ok(()), _)) => true,
                Either::Left((Err(error), _)) => return Err(error.into()),
                Either::Right((_, _)) => return Ok(()),
            }
        };
        if connected {
            let next =
                az_rpc::create_owner_only_named_pipe(&ServerOptions::new(), &endpoint.address)?;
            let connected = std::mem::replace(&mut server, next);
            // Dropping the join handle detaches the connection task, which is
            // what serving many clients from one listener wants.
            drop(az_rpc::spawn_twoparty_server(
                connected,
                client.client.clone(),
            ));
        }
    }
}

#[cfg(unix)]
fn start_unix_socket_server(
    bootstrap: ProjectHostServerBootstrap,
    endpoint: Endpoint,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    let listener = az_rpc::OwnedUnixListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    let (listener, socket_lease) = listener.into_parts();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let listener = tokio::net::UnixListener::from_std(listener)?;
            let result = run_unix_socket_listener(
                bootstrap,
                thread_endpoint,
                listener,
                shutdown_rx,
                startup_tx,
            )
            .await;
            drop(socket_lease);
            result
        });
    });
    await_server_start(endpoint, shutdown_tx, thread, &startup_rx)
}

// Signature is fixed by the `#[cfg(unix)]` twin above, which does consume the
// endpoint.
#[allow(clippy::needless_pass_by_value)]
#[cfg(not(unix))]
fn start_unix_socket_server(
    _bootstrap: ProjectHostServerBootstrap,
    endpoint: Endpoint,
) -> Result<ProjectHostRpcServer, ProjectHostRpcTransportError> {
    Err(ProjectHostRpcTransportError::UnsupportedEndpoint(
        endpoint.kind,
    ))
}

// Holds a capnp-rpc client across awaits; capnp-rpc keeps its connection state
// behind `Rc<RefCell<..>>`, so this future can never be `Send`.
#[allow(clippy::future_not_send)]
#[cfg(unix)]
async fn run_unix_socket_listener(
    bootstrap: ProjectHostServerBootstrap,
    endpoint: Endpoint,
    listener: tokio::net::UnixListener,
    mut shutdown: oneshot::Receiver<()>,
    startup: StartupSender,
) -> Result<(), ProjectHostRpcTransportError> {
    let Some(client) = admit_project_host(bootstrap, startup).await else {
        return Ok(());
    };
    info!(endpoint = %endpoint.address, "project-host unix-socket RPC listener started");

    loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut shutdown).await {
            Either::Left((Ok((stream, _)), _)) => {
                // Dropping the join handle detaches the connection task, which
                // is what serving many clients from one listener wants.
                drop(az_rpc::spawn_twoparty_server(stream, client.client.clone()));
            }
            Either::Left((Err(error), _)) => return Err(error.into()),
            Either::Right((_, _)) => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use az_gem_contract::{
        Contribution, ContributionDescriptor, ContributionId, GemContext, GemId, ProductActivation,
        declare_caps,
    };
    use az_proto_core::{ServiceHealthState, encode_capability_grant_set};

    use super::*;

    const TEST_TOKEN: [u8; 4] = [0x70, 0x68, 0x2d, 0x74];

    declare_caps!(ProjectHostServiceLifecycleCaps:);

    struct ProjectHostServiceLifecycle {
        finished: Arc<AtomicUsize>,
        cleaned: Arc<AtomicUsize>,
    }

    impl Contribution for ProjectHostServiceLifecycle {
        type Caps = ProjectHostServiceLifecycleCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.project-host-service-lifecycle-test"),
                contribution: ContributionId::new("project-host"),
                roles: &[az_project::GemTargetRole::ProjectHost],
            }
        }

        fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}

        fn finish(&self) {
            self.finished.fetch_add(1, Ordering::SeqCst);
        }

        fn cleanup(&self) {
            self.cleaned.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn lifecycle_composer() -> (Composer, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let finished = Arc::new(AtomicUsize::new(0));
        let cleaned = Arc::new(AtomicUsize::new(0));
        let mut composer = Composer::new(az_project::GemTargetRole::ProjectHost);
        composer
            .add(
                ProjectHostServiceLifecycle {
                    finished: Arc::clone(&finished),
                    cleaned: Arc::clone(&cleaned),
                },
                ProductActivation::default(),
            )
            .unwrap();
        (composer, finished, cleaned)
    }

    struct FakeAssetProcessor {
        health: ServiceHealth,
    }

    impl asset_capnp::asset_processor::Server for FakeAssetProcessor {
        // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
        // not `Send`; this future can never be `Send` without replacing the RPC
        // stack.
        #[allow(clippy::future_not_send)]
        async fn health(
            self: capnp::capability::Rc<Self>,
            _params: asset_capnp::asset_processor::HealthParams,
            mut results: asset_capnp::asset_processor::HealthResults,
        ) -> Result<(), capnp::Error> {
            self.health.to_capnp(results.get().init_health())
        }
    }

    struct FakeAssetProcessorServer {
        endpoint: Endpoint,
        shutdown: Option<oneshot::Sender<()>>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl FakeAssetProcessorServer {
        fn start(health: ServiceHealth) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let endpoint = Endpoint::new(
                EndpointKind::Tcp,
                listener.local_addr().unwrap().to_string(),
            );
            let (shutdown, mut shutdown_rx) = oneshot::channel();
            let thread = thread::spawn(move || {
                let runtime = Builder::new_current_thread().enable_io().build().unwrap();
                let local = LocalSet::new();
                runtime.block_on(local.run_until(async move {
                    let listener = TcpListener::from_std(listener).unwrap();
                    let (stream, _) = tokio::select! {
                        accepted = listener.accept() => accepted.unwrap(),
                        _ = &mut shutdown_rx => return,
                    };
                    let client: asset_capnp::asset_processor::Client =
                        capnp_rpc::new_client(FakeAssetProcessor { health });
                    let connection = az_rpc::spawn_twoparty_server(stream, client.client.clone());
                    let _ = shutdown_rx.await;
                    connection.abort();
                }));
            });
            Self {
                endpoint,
                shutdown: Some(shutdown),
                thread: Some(thread),
            }
        }

        fn stop(mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
        }
    }

    impl Drop for FakeAssetProcessorServer {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn capability(
        service: ServiceId,
        role: ServiceRole,
        audience: &str,
        permissions: &[&str],
    ) -> Capability {
        Capability::new(service, role)
            .with_audience(audience)
            .with_permissions(permissions.iter().copied())
            .with_token_hash(TEST_TOKEN)
    }

    fn production_grants() -> CapabilityGrantSet {
        CapabilityGrantSet::from_grants(vec![
            capability(
                ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                ServiceRole::Editor,
                PROJECT_HOST_AUDIENCE,
                EDITOR_PROJECT_HOST_PERMISSIONS,
            ),
            capability(
                ServiceId::new(
                    SESSION_SUPERVISOR_NAMESPACE,
                    SESSION_SUPERVISOR_SERVICE_NAME,
                ),
                ServiceRole::SessionSupervisor,
                PROJECT_HOST_AUDIENCE,
                SESSION_SUPERVISOR_PROJECT_HOST_PERMISSIONS,
            ),
            capability(
                ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
                ServiceRole::ProjectHost,
                ASSET_PROCESSOR_AUDIENCE,
                &[ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION],
            ),
        ])
    }

    fn service_startup(
        temp: &tempfile::TempDir,
        asset_processor_endpoint: Endpoint,
        run: Uuid,
    ) -> ProjectHostServiceStartup {
        let project_id = "local.project_host_service_test";
        az_project::write_project_manifest(
            temp.path(),
            &az_project::ProjectManifest::new(project_id, "Project Host Service Test", "0.1.0"),
        )
        .unwrap();
        az_project::refresh_project_lock(temp.path()).unwrap();
        let grants = temp.path().join("project-host.grants.capnp");
        fs::write(
            &grants,
            encode_capability_grant_set(&production_grants()).unwrap(),
        )
        .unwrap();
        ProjectHostServiceStartup {
            source_root: temp.path().to_path_buf(),
            side_channel_root: temp.path().join("side-channel"),
            endpoint: Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            asset_processor_endpoint,
            project_id: project_id.to_owned(),
            run,
            capability_grants_file: grants,
        }
    }

    fn asset_processor_health(service: ServiceId) -> ServiceHealth {
        ServiceHealth::ready(
            service,
            ServiceRole::AssetProcessor,
            Uuid::now_v7(),
            ProtocolVersion::CURRENT,
        )
    }

    #[test]
    fn service_startup_rejects_nil_run_before_publication() {
        let temp = tempfile::tempdir().unwrap();
        let startup = service_startup(
            &temp,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:1"),
            Uuid::nil(),
        );

        let error = match start_project_host_service(
            startup,
            Composer::new(az_project::GemTargetRole::ProjectHost),
        ) {
            Ok(server) => {
                server.stop().unwrap();
                panic!("nil run unexpectedly started project-host")
            }
            Err(error) => error,
        };
        assert!(matches!(error, ProjectHostRpcTransportError::NilRun));
    }

    #[test]
    fn service_startup_waits_for_asset_processor_identity() {
        let asset_processor = FakeAssetProcessorServer::start(asset_processor_health(
            ServiceId::new("azoth", "not-asset-processor"),
        ));
        let temp = tempfile::tempdir().unwrap();
        let startup = service_startup(&temp, asset_processor.endpoint.clone(), Uuid::now_v7());

        let error = match start_project_host_service(
            startup,
            Composer::new(az_project::GemTargetRole::ProjectHost),
        ) {
            Ok(server) => {
                server.stop().unwrap();
                panic!("project-host published readiness for the wrong Asset Processor")
            }
            Err(error) => error,
        };
        assert!(matches!(
            error,
            ProjectHostRpcTransportError::InvalidAssetProcessorHealth { .. }
        ));
        asset_processor.stop();
    }

    #[test]
    fn service_startup_returns_only_after_asset_processor_is_ready() {
        let mut health = asset_processor_health(ServiceId::new(
            ASSET_PROCESSOR_NAMESPACE,
            ASSET_PROCESSOR_SERVICE_NAME,
        ));
        health = health.with_state(ServiceHealthState::Ready);
        let asset_processor = FakeAssetProcessorServer::start(health);
        let temp = tempfile::tempdir().unwrap();
        let startup = service_startup(&temp, asset_processor.endpoint.clone(), Uuid::now_v7());

        let (composer, finished, cleaned) = lifecycle_composer();
        let server = start_project_host_service(startup, composer).unwrap();
        assert_eq!(finished.load(Ordering::SeqCst), 0);
        assert_eq!(cleaned.load(Ordering::SeqCst), 0);
        server.stop().unwrap();
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
        asset_processor.stop();
    }

    #[cfg(unix)]
    #[test]
    fn unix_transport_recovers_stale_socket_and_cleans_up_on_stop() {
        use std::os::unix::net::{UnixListener, UnixStream};

        let asset_processor = FakeAssetProcessorServer::start(asset_processor_health(
            ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
        ));
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("project-host.sock");
        drop(UnixListener::bind(&path).unwrap());
        let mut startup = service_startup(&temp, asset_processor.endpoint.clone(), Uuid::now_v7());
        startup.endpoint = Endpoint::new(EndpointKind::UnixDomainSocket, path.to_string_lossy());

        let server = start_project_host_service(
            startup,
            Composer::new(az_project::GemTargetRole::ProjectHost),
        )
        .unwrap();

        assert!(UnixStream::connect(&path).is_ok());
        server.stop().unwrap();
        assert!(!path.exists());
        asset_processor.stop();
    }
}
