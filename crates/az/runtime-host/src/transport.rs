#[cfg(any(test, feature = "test-support"))]
use std::path::Path;
use std::thread;

#[cfg(any(test, feature = "test-support"))]
use az_gem_contract::Registries;
use az_gem_contract::{Composer, ProcessCompositionCleanupError};
#[cfg(any(test, feature = "test-support"))]
use az_proto_core::CapabilityGrantSet;
use az_proto_core::{
    CapabilityGrantRequirement, CapabilityGrantSetValidationError, EDITOR_SERVICE_NAME,
    EDITOR_SERVICE_NAMESPACE, Endpoint, EndpointKind, ServiceRole, decode_capability_grant_set,
};
use az_proto_project::{PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME};
use az_proto_runtime::runtime_capnp;
use az_proto_runtime::{
    RUNTIME_CONTROL_PERMISSION, RUNTIME_HOST_AUDIENCE, RUNTIME_READ_PERMISSION,
};
use az_rpc::AzRpcTransportError;
use futures::future::Either;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::oneshot;
use tokio::task::LocalSet;
use tracing::info;
use uuid::Uuid;

use crate::{
    RuntimeHost, RuntimeHostComposition, RuntimeHostCompositionError, RuntimeHostError,
    RuntimeHostRpc, validate_runtime_side_channel_root,
};

#[derive(Debug, Error)]
pub enum RuntimeHostRpcTransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Rpc(#[from] AzRpcTransportError),

    #[error("capability grant decode error: {0}")]
    CapabilityGrantDecode(#[from] capnp::Error),

    #[error("runtime-host RPC endpoint requires at least one brokered capability grant")]
    EmptyCapabilityGrantSet,

    #[error("invalid session id `{session_id}` for runtime-host RPC grant set: {source}")]
    InvalidSessionId {
        session_id: String,
        #[source]
        source: uuid::Error,
    },

    #[error("runtime-host RPC session id `{session_id}` must not be the nil UUID")]
    NilSessionId { session_id: String },

    // Boxed: inline this validation error is 128 bytes and set the size of
    // every `Result<_, RuntimeHostRpcTransportError>` in this module
    // (`clippy::result_large_err`). The hand-written `From` below keeps `?`
    // converting the unboxed error exactly as `#[from]` used to.
    #[error("invalid runtime-host RPC capability grant set: {0}")]
    InvalidCapabilityGrantSet(#[source] Box<CapabilityGrantSetValidationError>),

    #[error("invalid runtime-host RPC side-channel root: {0}")]
    InvalidSideChannelRoot(#[from] RuntimeHostError),

    #[error(transparent)]
    Composition(#[from] RuntimeHostCompositionError),

    #[error("runtime-host RPC service run must not be the nil UUID")]
    NilRun,

    #[error("endpoint kind `{0:?}` is not supported on this platform")]
    UnsupportedEndpoint(EndpointKind),

    #[error("runtime-host RPC server thread failed to start")]
    StartupChannelClosed,
}

/// Boxes the payload on the way in, so `?` still converts a bare
/// [`CapabilityGrantSetValidationError`] the way `#[from]` did before the
/// variant was boxed.
impl From<CapabilityGrantSetValidationError> for RuntimeHostRpcTransportError {
    fn from(source: CapabilityGrantSetValidationError) -> Self {
        Self::InvalidCapabilityGrantSet(Box::new(source))
    }
}

#[derive(Debug, Error)]
pub enum RuntimeHostRpcShutdownFailure {
    #[error("could not close composition lease issuance: {0}")]
    BeginShutdown(#[source] ProcessCompositionCleanupError),
    #[error("runtime-host RPC listener thread panicked")]
    ListenerThreadPanicked,
    #[error("could not clean the runtime-host composition: {0}")]
    Cleanup(#[source] ProcessCompositionCleanupError),
}

#[derive(Debug, Error)]
#[error("runtime-host RPC shutdown had failures: {failures:?}")]
pub struct RuntimeHostRpcShutdownError {
    pub failures: Vec<RuntimeHostRpcShutdownFailure>,
}

pub struct RuntimeHostRpcServer {
    endpoint: Endpoint,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
    // Kept on the spawning thread because App-less roles must not make Bevy's
    // non-Send `App` container Send merely to serve immutable registries.
    // Drop joins the RPC thread before this lifecycle owner is released.
    composition: Option<RuntimeHostComposition>,
}

impl RuntimeHostRpcServer {
    #[must_use]
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Signal the listener thread, join it, and tear the composition down.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeHostRpcShutdownError`] collecting every step that
    /// failed: [`RuntimeHostRpcShutdownFailure::BeginShutdown`] if lease
    /// issuance could not be closed,
    /// [`RuntimeHostRpcShutdownFailure::ListenerThreadPanicked`] if the RPC
    /// thread panicked, and [`RuntimeHostRpcShutdownFailure::Cleanup`] if the
    /// composition could not be cleaned. Shutdown continues past each failure.
    pub fn stop(mut self) -> Result<(), RuntimeHostRpcShutdownError> {
        let mut failures = Vec::new();
        if let Some(composition) = self.composition.as_mut()
            && let Err(error) = composition.begin_shutdown()
        {
            failures.push(RuntimeHostRpcShutdownFailure::BeginShutdown(error));
        }
        self.shutdown();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            failures.push(RuntimeHostRpcShutdownFailure::ListenerThreadPanicked);
        }
        if let Some(composition) = self.composition.as_mut()
            && let Err(error) = composition.cleanup()
        {
            failures.push(RuntimeHostRpcShutdownFailure::Cleanup(error));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(RuntimeHostRpcShutdownError { failures })
        }
    }

    fn shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

impl Drop for RuntimeHostRpcServer {
    fn drop(&mut self) {
        // Signal AND join: the listener thread must not outlive this struct.
        // `stop()` already does both explicitly; mirroring that here means a
        // server dropped without an explicit `stop()` call still tears its
        // thread down deterministically instead of leaking it past the
        // struct's lifetime.
        if let Some(composition) = self.composition.as_mut() {
            let _ = composition.begin_shutdown();
        }
        self.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some(composition) = self.composition.as_mut() {
            let _ = composition.cleanup();
        }
    }
}

/// Immutable process inputs required to publish one runtime-host service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeHostServiceStartup {
    pub endpoint: Endpoint,
    pub session_id: Uuid,
    pub side_channel_root: std::path::PathBuf,
    pub run: Uuid,
    pub capability_grants_file: std::path::PathBuf,
}

/// Own one generated runtime-host composition and serve it over RPC.
///
/// The generated process supplies its [`Composer`] directly. This boundary
/// fixes the role, validates service identity and brokered grants, finalizes
/// the composition once, and keeps its lifecycle owner in the returned server
/// handle. The RPC thread receives only an immutable registry lease.
///
/// # Errors
///
/// Returns a typed refusal for invalid service identity, grants, side-channel
/// ownership, composition, or endpoint publication.
pub fn start_runtime_host_service(
    startup: RuntimeHostServiceStartup,
    composer: Composer,
) -> Result<RuntimeHostRpcServer, RuntimeHostRpcTransportError> {
    if startup.session_id.is_nil() {
        return Err(RuntimeHostRpcTransportError::NilSessionId {
            session_id: startup.session_id.to_string(),
        });
    }
    if startup.run.is_nil() {
        return Err(RuntimeHostRpcTransportError::NilRun);
    }
    let bytes = std::fs::read(&startup.capability_grants_file)?;
    let capability_grants = decode_capability_grant_set(&bytes)?;
    validate_runtime_host_rpc_capability_grants(startup.session_id, &capability_grants)?;
    validate_runtime_side_channel_root(&startup.side_channel_root)?;
    let (host, composition) = RuntimeHost::from_composer(
        startup.session_id.to_string(),
        capability_grants,
        startup.side_channel_root,
        composer,
    )?;
    start_runtime_host_rpc_server_with_host(host, startup.endpoint, startup.run, Some(composition))
}

/// Linked-composition adapter retained for integration tests.
///
/// Production generated targets must call [`start_runtime_host_service`] so
/// the server owns the composition rather than borrowing leaked registries.
///
/// # Errors
///
/// Returns [`RuntimeHostRpcTransportError::Io`] if the grant file cannot be
/// read, [`RuntimeHostRpcTransportError::CapabilityGrantDecode`] if its bytes
/// are not a capability grant set,
/// [`RuntimeHostRpcTransportError::InvalidSessionId`] or
/// [`RuntimeHostRpcTransportError::NilSessionId`] if `session_id` is not a
/// non-nil UUID, [`RuntimeHostRpcTransportError::InvalidCapabilityGrantSet`] if
/// the grants do not cover this host's required permissions,
/// [`RuntimeHostRpcTransportError::InvalidSideChannelRoot`] if
/// `side_channel_root` is not an absolute normal path, and any error
/// [`start_runtime_host_service`] returns while publishing `endpoint`.
#[cfg(any(test, feature = "test-support"))]
pub fn start_runtime_host_rpc_server_for_session_with_capability_grant_file(
    endpoint: Endpoint,
    session_id: impl Into<String>,
    side_channel_root: impl AsRef<Path>,
    run: Uuid,
    capability_grants_file: impl AsRef<Path>,
    registries: &'static Registries,
) -> Result<RuntimeHostRpcServer, RuntimeHostRpcTransportError> {
    let bytes = std::fs::read(capability_grants_file.as_ref())?;
    let capability_grants = decode_capability_grant_set(&bytes)?;
    let session_id = session_id.into();
    let parsed_session = parse_runtime_host_session_id(&session_id)?;
    validate_runtime_host_rpc_capability_grants(parsed_session, &capability_grants)?;
    validate_runtime_side_channel_root(side_channel_root.as_ref())?;
    start_runtime_host_rpc_server_with_host(
        RuntimeHost::new(
            session_id,
            capability_grants,
            side_channel_root.as_ref().to_path_buf(),
            registries,
        ),
        endpoint,
        run,
        None,
    )
}

/// Linked-composition adapter retained for integration tests, taking an
/// already-decoded grant set.
///
/// # Errors
///
/// Returns [`RuntimeHostRpcTransportError::InvalidSideChannelRoot`] if
/// `side_channel_root` is not an absolute normal path,
/// [`RuntimeHostRpcTransportError::EmptyCapabilityGrantSet`] if
/// `capability_grants` is empty, and
/// [`RuntimeHostRpcTransportError::Io`],
/// [`RuntimeHostRpcTransportError::UnsupportedEndpoint`], or
/// [`RuntimeHostRpcTransportError::StartupChannelClosed`] if `endpoint` cannot
/// be bound and served.
#[cfg(any(test, feature = "test-support"))]
pub fn start_runtime_host_rpc_server_for_session_with_capability_grants(
    endpoint: Endpoint,
    session_id: impl Into<String>,
    side_channel_root: impl AsRef<Path>,
    capability_grants: CapabilityGrantSet,
    registries: &'static Registries,
) -> Result<RuntimeHostRpcServer, RuntimeHostRpcTransportError> {
    validate_runtime_side_channel_root(side_channel_root.as_ref())?;
    start_runtime_host_rpc_server_with_host(
        RuntimeHost::new(
            session_id,
            capability_grants,
            side_channel_root.as_ref().to_path_buf(),
            registries,
        ),
        endpoint,
        Uuid::now_v7(),
        None,
    )
}

fn start_runtime_host_rpc_server_with_host(
    host: RuntimeHost,
    endpoint: Endpoint,
    service_run: Uuid,
    composition: Option<RuntimeHostComposition>,
) -> Result<RuntimeHostRpcServer, RuntimeHostRpcTransportError> {
    if host.capability_grants().is_empty() {
        return Err(RuntimeHostRpcTransportError::EmptyCapabilityGrantSet);
    }

    match endpoint.kind {
        EndpointKind::Tcp => start_tcp_server(host, &endpoint, service_run, composition),
        EndpointKind::WindowsNamedPipe => {
            start_named_pipe_server(host, endpoint, service_run, composition)
        }
        EndpointKind::UnixDomainSocket => {
            start_unix_socket_server(host, endpoint, service_run, composition)
        }
        EndpointKind::InProcess => Err(RuntimeHostRpcTransportError::UnsupportedEndpoint(
            EndpointKind::InProcess,
        )),
    }
}

const EDITOR_RUNTIME_HOST_PERMISSIONS: &[&str] =
    &[RUNTIME_READ_PERMISSION, RUNTIME_CONTROL_PERMISSION];
const PROJECT_HOST_RUNTIME_HOST_PERMISSIONS: &[&str] = &[RUNTIME_CONTROL_PERMISSION];

fn validate_runtime_host_rpc_capability_grants(
    session: Uuid,
    capability_grants: &az_proto_core::CapabilityGrantSet,
) -> Result<(), RuntimeHostRpcTransportError> {
    if session.is_nil() {
        return Err(RuntimeHostRpcTransportError::NilSessionId {
            session_id: session.to_string(),
        });
    }
    capability_grants.validate_exact_brokered_for_session(
        session,
        &[
            CapabilityGrantRequirement::new(
                EDITOR_SERVICE_NAMESPACE,
                EDITOR_SERVICE_NAME,
                ServiceRole::Editor,
                RUNTIME_HOST_AUDIENCE,
                EDITOR_RUNTIME_HOST_PERMISSIONS,
            ),
            CapabilityGrantRequirement::new(
                PROJECT_HOST_NAMESPACE,
                PROJECT_HOST_SERVICE_NAME,
                ServiceRole::ProjectHost,
                RUNTIME_HOST_AUDIENCE,
                PROJECT_HOST_RUNTIME_HOST_PERMISSIONS,
            ),
        ],
    )?;
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
fn parse_runtime_host_session_id(session_id: &str) -> Result<Uuid, RuntimeHostRpcTransportError> {
    Uuid::parse_str(session_id).map_err(|source| RuntimeHostRpcTransportError::InvalidSessionId {
        session_id: session_id.to_string(),
        source,
    })
}

/// Connect a client to a published runtime-host RPC endpoint.
///
/// # Errors
///
/// Returns [`RuntimeHostRpcTransportError::Rpc`] if the endpoint cannot be
/// dialed or its bootstrap capability cannot be resolved.
pub async fn connect_runtime_host_rpc_client(
    endpoint: &Endpoint,
) -> Result<runtime_capnp::runtime_host::Client, RuntimeHostRpcTransportError> {
    Ok(az_rpc::connect_twoparty_bootstrap(endpoint).await?)
}

fn start_tcp_server(
    host: RuntimeHost,
    endpoint: &Endpoint,
    service_run: Uuid,
    composition: Option<RuntimeHostComposition>,
) -> Result<RuntimeHostRpcServer, RuntimeHostRpcTransportError> {
    let listener = std::net::TcpListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?.to_string();
    let endpoint = Endpoint::new(EndpointKind::Tcp, address);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
    let startup_failure = startup_tx.clone();
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        let result = run_threaded_local(async move {
            let listener = TcpListener::from_std(listener)?;
            run_tcp_listener(
                host,
                thread_endpoint,
                listener,
                shutdown_rx,
                service_run,
                startup_tx,
            )
            .await
        });
        if let Err(error) = result {
            tracing::error!(%error, "runtime-host RPC listener stopped with error");
            let _ = startup_failure.try_send(Err(error));
        }
    });
    await_server_start(endpoint, shutdown_tx, thread, &startup_rx, composition)
}

// Holds a capnp-rpc client across awaits; capnp-rpc keeps its connection
// state behind `Rc<RefCell<..>>`, so this future can never be `Send`.
#[allow(clippy::future_not_send)]
async fn run_tcp_listener(
    host: RuntimeHost,
    endpoint: Endpoint,
    listener: TcpListener,
    mut shutdown: oneshot::Receiver<()>,
    service_run: Uuid,
    startup: StartupSender,
) -> Result<(), RuntimeHostRpcTransportError> {
    let bootstrap = RuntimeHostRpc::new(host)
        .with_service_run(service_run)
        .into_client();
    startup
        .send(Ok(()))
        .map_err(|_| RuntimeHostRpcTransportError::StartupChannelClosed)?;
    info!(endpoint = %endpoint.address, "runtime-host RPC listener started");

    loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut shutdown).await {
            Either::Left((Ok((stream, peer)), _)) => {
                info!(endpoint = %endpoint.address, peer = %peer, "runtime-host RPC client connected");
                drop(az_rpc::spawn_twoparty_server(
                    stream,
                    bootstrap.client.clone(),
                ));
            }
            Either::Left((Err(error), _)) => return Err(error.into()),
            Either::Right((_, _)) => return Ok(()),
        }
    }
}

fn run_threaded_local<F>(future: F) -> Result<(), RuntimeHostRpcTransportError>
where
    F: std::future::Future<Output = Result<(), RuntimeHostRpcTransportError>> + 'static,
{
    let runtime = Builder::new_current_thread().enable_io().build()?;
    let local = LocalSet::new();
    runtime.block_on(local.run_until(future))
}

#[cfg(windows)]
fn start_named_pipe_server(
    host: RuntimeHost,
    endpoint: Endpoint,
    service_run: Uuid,
    composition: Option<RuntimeHostComposition>,
) -> Result<RuntimeHostRpcServer, RuntimeHostRpcTransportError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
    let startup_failure = startup_tx.clone();
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        let result = run_threaded_local(async move {
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
            run_named_pipe_listener(
                host,
                thread_endpoint,
                server,
                shutdown_rx,
                service_run,
                startup_tx,
            )
            .await
        });
        if let Err(error) = result {
            tracing::error!(%error, "runtime-host RPC listener stopped with error");
            let _ = startup_failure.try_send(Err(error));
        }
    });
    await_server_start(endpoint, shutdown_tx, thread, &startup_rx, composition)
}

#[cfg(not(windows))]
fn start_named_pipe_server(
    _host: RuntimeHost,
    endpoint: Endpoint,
    _service_run: Uuid,
    composition: Option<RuntimeHostComposition>,
) -> Result<RuntimeHostRpcServer, RuntimeHostRpcTransportError> {
    Err(RuntimeHostRpcTransportError::UnsupportedEndpoint(
        endpoint.kind,
    ))
}

#[cfg(windows)]
// Holds a capnp-rpc client across awaits; capnp-rpc keeps its connection
// state behind `Rc<RefCell<..>>`, so this future can never be `Send`.
#[allow(clippy::future_not_send)]
async fn run_named_pipe_listener(
    host: RuntimeHost,
    endpoint: Endpoint,
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    mut shutdown: oneshot::Receiver<()>,
    service_run: Uuid,
    startup: StartupSender,
) -> Result<(), RuntimeHostRpcTransportError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let bootstrap = RuntimeHostRpc::new(host)
        .with_service_run(service_run)
        .into_client();
    startup
        .send(Ok(()))
        .map_err(|_| RuntimeHostRpcTransportError::StartupChannelClosed)?;
    info!(endpoint = %endpoint.address, "runtime-host named-pipe RPC listener started");

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
            drop(az_rpc::spawn_twoparty_server(
                connected,
                bootstrap.client.clone(),
            ));
        }
    }
}

#[cfg(unix)]
fn start_unix_socket_server(
    host: RuntimeHost,
    endpoint: Endpoint,
    service_run: Uuid,
    composition: Option<RuntimeHostComposition>,
) -> Result<RuntimeHostRpcServer, RuntimeHostRpcTransportError> {
    let listener = az_rpc::OwnedUnixListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    let (listener, socket_lease) = listener.into_parts();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
    let startup_failure = startup_tx.clone();
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        let result = run_threaded_local(async move {
            let listener = tokio::net::UnixListener::from_std(listener)?;
            let result = run_unix_socket_listener(
                host,
                thread_endpoint,
                listener,
                shutdown_rx,
                service_run,
                startup_tx,
            )
            .await;
            drop(socket_lease);
            result
        });
        if let Err(error) = result {
            tracing::error!(%error, "runtime-host RPC listener stopped with error");
            let _ = startup_failure.try_send(Err(error));
        }
    });
    await_server_start(endpoint, shutdown_tx, thread, &startup_rx, composition)
}

// `endpoint` must stay by value to match the `cfg(unix)` twin above, which
// consumes it; taking it by reference here would desynchronize the pair.
#[allow(clippy::needless_pass_by_value)]
#[cfg(not(unix))]
fn start_unix_socket_server(
    _host: RuntimeHost,
    endpoint: Endpoint,
    _service_run: Uuid,
    _composition: Option<RuntimeHostComposition>,
) -> Result<RuntimeHostRpcServer, RuntimeHostRpcTransportError> {
    Err(RuntimeHostRpcTransportError::UnsupportedEndpoint(
        endpoint.kind,
    ))
}

#[cfg(unix)]
async fn run_unix_socket_listener(
    host: RuntimeHost,
    endpoint: Endpoint,
    listener: tokio::net::UnixListener,
    mut shutdown: oneshot::Receiver<()>,
    service_run: Uuid,
    startup: StartupSender,
) -> Result<(), RuntimeHostRpcTransportError> {
    let bootstrap = RuntimeHostRpc::new(host)
        .with_service_run(service_run)
        .into_client();
    startup
        .send(Ok(()))
        .map_err(|_| RuntimeHostRpcTransportError::StartupChannelClosed)?;
    info!(endpoint = %endpoint.address, "runtime-host unix-socket RPC listener started");

    loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut shutdown).await {
            Either::Left((Ok((stream, _)), _)) => {
                drop(az_rpc::spawn_twoparty_server(
                    stream,
                    bootstrap.client.clone(),
                ));
            }
            Either::Left((Err(error), _)) => return Err(error.into()),
            Either::Right((_, _)) => return Ok(()),
        }
    }
}

type StartupSender = std::sync::mpsc::SyncSender<Result<(), RuntimeHostRpcTransportError>>;
type StartupReceiver = std::sync::mpsc::Receiver<Result<(), RuntimeHostRpcTransportError>>;

fn await_server_start(
    endpoint: Endpoint,
    shutdown: oneshot::Sender<()>,
    thread: thread::JoinHandle<()>,
    startup: &StartupReceiver,
    composition: Option<RuntimeHostComposition>,
) -> Result<RuntimeHostRpcServer, RuntimeHostRpcTransportError> {
    match startup.recv() {
        Ok(Ok(())) => Ok(RuntimeHostRpcServer {
            endpoint,
            shutdown: Some(shutdown),
            thread: Some(thread),
            composition,
        }),
        Ok(Err(error)) => {
            drop(shutdown);
            let _ = thread.join();
            Err(error)
        }
        Err(_) => {
            drop(shutdown);
            let _ = thread.join();
            Err(RuntimeHostRpcTransportError::StartupChannelClosed)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use az_gem_contract::{
        Contribution, ContributionDescriptor, ContributionId, GemContext, GemId, GemTargetRole,
        ProductActivation,
    };
    use az_proto_core::{
        Capability, CapabilityGrantSet, CapabilityGrantSetValidationError, ServiceId, ServiceRole,
        encode_capability_grant_set,
    };
    use az_proto_runtime::{
        RUNTIME_CONTROL_PERMISSION, RUNTIME_READ_PERMISSION, RuntimeStatusRequest,
        RuntimeStatusResult,
    };

    use super::*;

    az_gem_contract::declare_caps!(LifecycleCaps:);

    /// The transport tests exercise endpoints, not projections, so they serve
    /// an empty composition — which is a legitimate composition, not a hole:
    /// a host with nothing composed launches metadata-only runtimes.
    fn transport_registries() -> &'static Registries {
        static REGISTRIES: std::sync::OnceLock<Registries> = std::sync::OnceLock::new();
        REGISTRIES.get_or_init(Registries::new)
    }

    fn read_capability(session: Uuid) -> Capability {
        Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(session)
            .with_audience(crate::RUNTIME_HOST_AUDIENCE)
            .with_permissions([RUNTIME_READ_PERMISSION])
            .with_token_hash([0x72, 0x48])
    }

    fn editor_capability(session: Uuid) -> Capability {
        Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(session)
            .with_audience(crate::RUNTIME_HOST_AUDIENCE)
            .with_permissions([RUNTIME_READ_PERMISSION, RUNTIME_CONTROL_PERMISSION])
            .with_token_hash([0x72, 0x48])
    }

    fn project_host_capability(session: Uuid) -> Capability {
        Capability::new(
            ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
        )
        .with_session(session)
        .with_audience(crate::RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_CONTROL_PERMISSION])
        .with_token_hash([0x72, 0x49])
    }

    fn service_startup(
        temp: &tempfile::TempDir,
        session_id: Uuid,
        run: Uuid,
        grants: &CapabilityGrantSet,
    ) -> RuntimeHostServiceStartup {
        let capability_grants_file = temp.path().join("capability-grants.bin");
        std::fs::write(
            &capability_grants_file,
            encode_capability_grant_set(grants).unwrap(),
        )
        .unwrap();
        RuntimeHostServiceStartup {
            endpoint: Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            session_id,
            side_channel_root: temp.path().join("runtime-host"),
            run,
            capability_grants_file,
        }
    }

    #[test]
    fn listener_startup_failure_is_returned_before_a_server_handle() {
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0");
        let (shutdown, _shutdown_rx) = oneshot::channel();
        let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            startup_tx
                .send(Err(RuntimeHostRpcTransportError::Io(
                    std::io::Error::other("listener setup failed"),
                )))
                .unwrap();
        });

        let Err(error) = await_server_start(endpoint, shutdown, thread, &startup_rx, None) else {
            panic!("a failed listener must not publish a server handle");
        };

        assert!(matches!(error, RuntimeHostRpcTransportError::Io(_)));
    }

    struct LifecycleContribution {
        ready: bool,
        finished: Arc<AtomicUsize>,
        cleaned: Arc<AtomicUsize>,
    }

    impl Contribution for LifecycleContribution {
        type Caps = LifecycleCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.runtime-host-lifecycle-test"),
                contribution: ContributionId::new("runtime-host"),
                roles: &[GemTargetRole::RuntimeHost],
            }
        }

        fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}

        fn ready(&self) -> bool {
            self.ready
        }

        fn finish(&self) {
            self.finished.fetch_add(1, Ordering::SeqCst);
        }

        fn cleanup(&self) {
            self.cleaned.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn tcp_transport_serves_runtime_host_rpc() {
        let temp = tempfile::tempdir().unwrap();
        let session = Uuid::from_bytes([0x71; 16]);
        let read_grant = read_capability(session);
        let server = start_runtime_host_rpc_server_for_session_with_capability_grants(
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            session.to_string(),
            temp.path().join("runtime-host"),
            CapabilityGrantSet::from_grants(vec![read_grant.clone()]),
            transport_registries(),
        )
        .unwrap();
        let endpoint = server.endpoint().clone();

        let runtime = Builder::new_current_thread().enable_io().build().unwrap();
        let local = LocalSet::new();
        local.block_on(&runtime, async move {
            let client = connect_runtime_host_rpc_client(&endpoint).await.unwrap();
            let mut request = client.status_request();
            (RuntimeStatusRequest {
                capability: read_grant.clone(),
                runtime_id: "missing".to_string(),
            })
            .to_capnp(request.get().init_request())
            .unwrap();

            let response = request.send().promise.await.unwrap();
            let result =
                RuntimeStatusResult::from_capnp(response.get().unwrap().get_result().unwrap())
                    .unwrap();
            assert_eq!(result.status, None);

            let mut rejected = client.status_request();
            (RuntimeStatusRequest {
                capability: read_grant.with_token_hash([0xff]),
                runtime_id: "missing".to_string(),
            })
            .to_capnp(rejected.get().init_request())
            .unwrap();
            let Err(error) = rejected.send().promise.await else {
                panic!("runtime-host transport accepted an ungranted capability");
            };
            assert!(
                error.to_string().contains("not brokered"),
                "unexpected error: {error}"
            );
        });

        server.stop().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_transport_recovers_stale_socket_and_cleans_up_on_stop() {
        use std::os::unix::net::{UnixListener, UnixStream};

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime-host.sock");
        drop(UnixListener::bind(&path).unwrap());
        let session = Uuid::from_bytes([0x77; 16]);
        let grant = read_capability(session);

        let server = start_runtime_host_rpc_server_for_session_with_capability_grants(
            Endpoint::new(EndpointKind::UnixDomainSocket, path.to_string_lossy()),
            session.to_string(),
            temp.path().join("runtime-host"),
            CapabilityGrantSet::from_grants(vec![grant]),
            transport_registries(),
        )
        .unwrap();

        assert!(UnixStream::connect(&path).is_ok());
        server.stop().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn tcp_transport_rejects_empty_capability_grant_set() {
        let temp = tempfile::tempdir().unwrap();
        let result = start_runtime_host_rpc_server_for_session_with_capability_grants(
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            Uuid::from_bytes([0x72; 16]).to_string(),
            temp.path().join("runtime-host"),
            CapabilityGrantSet::new(),
            transport_registries(),
        );

        assert!(matches!(
            result,
            Err(RuntimeHostRpcTransportError::EmptyCapabilityGrantSet)
        ));
    }

    #[test]
    fn production_session_transport_rejects_incomplete_brokered_grants() {
        let temp = tempfile::tempdir().unwrap();
        let session = Uuid::from_bytes([0x71; 16]);
        let grants = CapabilityGrantSet::from_grants(vec![read_capability(session)]);
        let startup = service_startup(&temp, session, Uuid::from_bytes([0x81; 16]), &grants);
        let result = start_runtime_host_service(startup, Composer::new(GemTargetRole::RuntimeHost));

        let Err(RuntimeHostRpcTransportError::InvalidCapabilityGrantSet(source)) = result else {
            panic!("expected the grant set to be refused as invalid");
        };
        assert!(matches!(
            *source,
            CapabilityGrantSetValidationError::MissingRequiredGrant(..)
        ));
    }

    #[test]
    fn production_session_transport_rejects_nil_session_id() {
        let temp = tempfile::tempdir().unwrap();
        let session = Uuid::from_bytes([0x73; 16]);
        let grants = CapabilityGrantSet::from_grants(vec![
            editor_capability(session),
            project_host_capability(session),
        ]);
        let startup = service_startup(&temp, Uuid::nil(), Uuid::from_bytes([0x82; 16]), &grants);
        let result = start_runtime_host_service(startup, Composer::new(GemTargetRole::RuntimeHost));

        assert!(matches!(
            result,
            Err(RuntimeHostRpcTransportError::NilSessionId { session_id })
                if session_id == Uuid::nil().to_string()
        ));
    }

    #[test]
    fn production_session_transport_rejects_nil_run() {
        let temp = tempfile::tempdir().unwrap();
        let session = Uuid::from_bytes([0x74; 16]);
        let grants = CapabilityGrantSet::from_grants(vec![
            editor_capability(session),
            project_host_capability(session),
        ]);
        let startup = service_startup(&temp, session, Uuid::nil(), &grants);

        let result = start_runtime_host_service(startup, Composer::new(GemTargetRole::RuntimeHost));

        assert!(matches!(result, Err(RuntimeHostRpcTransportError::NilRun)));
    }

    #[test]
    fn production_server_owns_composition_lifecycle_until_stop() {
        let temp = tempfile::tempdir().unwrap();
        let session = Uuid::from_bytes([0x75; 16]);
        let grants = CapabilityGrantSet::from_grants(vec![
            editor_capability(session),
            project_host_capability(session),
        ]);
        let startup = service_startup(&temp, session, Uuid::from_bytes([0x85; 16]), &grants);
        let finished = Arc::new(AtomicUsize::new(0));
        let cleaned = Arc::new(AtomicUsize::new(0));
        let mut composer = Composer::new(GemTargetRole::RuntimeHost);
        composer
            .add(
                LifecycleContribution {
                    ready: true,
                    finished: Arc::clone(&finished),
                    cleaned: Arc::clone(&cleaned),
                },
                ProductActivation::default(),
            )
            .unwrap();

        let server = start_runtime_host_service(startup, composer).unwrap();
        assert_eq!(finished.load(Ordering::SeqCst), 0);
        assert_eq!(cleaned.load(Ordering::SeqCst), 0);

        server.stop().unwrap();

        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn production_server_refuses_unready_composition_and_cleans_it_up() {
        let temp = tempfile::tempdir().unwrap();
        let session = Uuid::from_bytes([0x76; 16]);
        let grants = CapabilityGrantSet::from_grants(vec![
            editor_capability(session),
            project_host_capability(session),
        ]);
        let startup = service_startup(&temp, session, Uuid::from_bytes([0x86; 16]), &grants);
        let finished = Arc::new(AtomicUsize::new(0));
        let cleaned = Arc::new(AtomicUsize::new(0));
        let mut composer = Composer::new(GemTargetRole::RuntimeHost);
        composer
            .add(
                LifecycleContribution {
                    ready: false,
                    finished: Arc::clone(&finished),
                    cleaned: Arc::clone(&cleaned),
                },
                ProductActivation::default(),
            )
            .unwrap();

        let result = start_runtime_host_service(startup, composer);

        assert!(matches!(
            result,
            Err(RuntimeHostRpcTransportError::Composition(
                RuntimeHostCompositionError::NotReady
            ))
        ));
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }
}
