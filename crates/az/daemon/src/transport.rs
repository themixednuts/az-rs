use std::path::{Path, PathBuf};
use std::thread;

use az_proto_core::{Capability, Endpoint, EndpointKind, ProtocolVersion, ServiceId, ServiceRole};
use az_proto_daemon::{
    DAEMON_AUDIENCE, DAEMON_NAMESPACE, DAEMON_READ_PERMISSION, DAEMON_SERVICE_NAME,
    ListProjectsRequest, ListProjectsResult, daemon_capnp,
};
use az_rpc::AzRpcTransportError;
use futures::future::Either;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::oneshot;
use tokio::task::LocalSet;
use tracing::{error, info};
use uuid::Uuid;

use crate::{AzDaemon, AzDaemonRpc};

#[derive(Debug, Error)]
pub enum DaemonRpcTransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    // Boxed: `DaemonEndpointRecordError` is 128 bytes on its own, which made
    // every `Result<_, DaemonRpcTransportError>` an oversized `Err`.
    #[error(transparent)]
    EndpointRecord(Box<az_endpoint_discovery::DaemonEndpointRecordError>),

    #[error(transparent)]
    Rpc(#[from] AzRpcTransportError),

    #[error("azd unavailable until restarted: protocol preflight failed: {0}")]
    ProtocolPreflight(String),

    #[error("endpoint kind `{0:?}` is not supported on this platform")]
    UnsupportedEndpoint(EndpointKind),

    #[error("azd RPC server thread failed to start")]
    StartupChannelClosed,
}

// `EndpointRecord` carries a boxed payload, so `#[from]` would have derived
// `From<Box<DaemonEndpointRecordError>>` and broken `?` on the unboxed error.
impl From<az_endpoint_discovery::DaemonEndpointRecordError> for DaemonRpcTransportError {
    fn from(source: az_endpoint_discovery::DaemonEndpointRecordError) -> Self {
        Self::EndpointRecord(Box::new(source))
    }
}

pub struct AzDaemonRpcServer {
    endpoint: Endpoint,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

pub use az_endpoint_discovery::{DaemonEndpointRecord, DaemonEndpointRecordGuard};

impl AzDaemonRpcServer {
    #[must_use]
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn stop(mut self) {
        self.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    fn shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

impl Drop for AzDaemonRpcServer {
    fn drop(&mut self) {
        // Signal AND join: the listener thread must not outlive this struct.
        // `stop()` already does both explicitly; mirroring that here means a
        // server dropped without an explicit `stop()` call still tears its
        // thread down deterministically instead of leaking it past the
        // struct's lifetime.
        self.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start the `azd` RPC listener for a fresh in-process daemon.
///
/// # Errors
///
/// Returns any error [`start_az_daemon_rpc_server_with_daemon`] returns.
pub fn start_az_daemon_rpc_server(
    endpoint: Endpoint,
) -> Result<AzDaemonRpcServer, DaemonRpcTransportError> {
    start_az_daemon_rpc_server_with_daemon(AzDaemon::new(), endpoint)
}

/// Start the `azd` RPC listener in front of an existing daemon.
///
/// # Errors
///
/// Returns any error [`start_az_daemon_rpc_server_with_daemon_and_shutdown`]
/// returns.
pub fn start_az_daemon_rpc_server_with_daemon(
    daemon: AzDaemon,
    endpoint: Endpoint,
) -> Result<AzDaemonRpcServer, DaemonRpcTransportError> {
    start_az_daemon_rpc_server_with_daemon_and_shutdown(
        daemon,
        endpoint,
        az_work::CancellationToken::new(),
        Uuid::now_v7(),
    )
}

/// Start the `azd` RPC listener with an explicit shutdown token and run id.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::UnsupportedEndpoint`] when `endpoint` is
/// [`EndpointKind::InProcess`], or names a transport this platform does not
/// build (named pipes off Windows, unix sockets off unix). Returns
/// [`DaemonRpcTransportError::Io`] when the socket, port, or first pipe
/// instance cannot be bound, and
/// [`DaemonRpcTransportError::StartupChannelClosed`] when the named-pipe
/// listener thread drops its readiness channel before reporting.
pub fn start_az_daemon_rpc_server_with_daemon_and_shutdown(
    daemon: AzDaemon,
    endpoint: Endpoint,
    control_shutdown: az_work::CancellationToken,
    run: Uuid,
) -> Result<AzDaemonRpcServer, DaemonRpcTransportError> {
    match endpoint.kind {
        EndpointKind::Tcp => start_tcp_server(daemon, endpoint, control_shutdown, run),
        EndpointKind::WindowsNamedPipe => {
            start_named_pipe_server(daemon, endpoint, control_shutdown, run)
        }
        EndpointKind::UnixDomainSocket => {
            start_unix_socket_server(daemon, endpoint, control_shutdown, run)
        }
        EndpointKind::InProcess => Err(DaemonRpcTransportError::UnsupportedEndpoint(
            EndpointKind::InProcess,
        )),
    }
}

/// Connect an `azd` client and run the protocol-version preflight.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::Rpc`] when the two-party bootstrap
/// connection cannot be established, and
/// [`DaemonRpcTransportError::ProtocolPreflight`] when the probe
/// `listProjects` call cannot be encoded, does not complete, cannot be decoded,
/// or reports a protocol version incompatible with
/// [`ProtocolVersion::CURRENT`].
// capnp-rpc keeps its connection state behind `Rc<RefCell<..>>`, so the client
// handle and its in-flight request are `!Send` by construction.
#[allow(clippy::future_not_send)]
pub async fn connect_az_daemon_rpc_client(
    endpoint: &Endpoint,
) -> Result<daemon_capnp::az_daemon::Client, DaemonRpcTransportError> {
    let client: daemon_capnp::az_daemon::Client =
        az_rpc::connect_twoparty_bootstrap(endpoint).await?;
    let mut request = client.list_projects_request();
    (ListProjectsRequest {
        capability: Capability::new(
            ServiceId::new(DAEMON_NAMESPACE, DAEMON_SERVICE_NAME),
            ServiceRole::Daemon,
        )
        .with_audience(DAEMON_AUDIENCE)
        .with_permissions([DAEMON_READ_PERMISSION]),
    })
    .to_capnp(request.get().init_request())
    .map_err(daemon_protocol_preflight_error)?;
    let response = request
        .send()
        .promise
        .await
        .map_err(daemon_protocol_preflight_error)?;
    let result = ListProjectsResult::from_capnp(
        response
            .get()
            .map_err(daemon_protocol_preflight_error)?
            .get_result()
            .map_err(daemon_protocol_preflight_error)?,
    )
    .map_err(daemon_protocol_preflight_error)?;
    result
        .protocol_version
        .require(ProtocolVersion::CURRENT)
        .map_err(daemon_protocol_preflight_error)?;
    Ok(client)
}

fn daemon_protocol_preflight_error(error: impl std::fmt::Display) -> DaemonRpcTransportError {
    DaemonRpcTransportError::ProtocolPreflight(error.to_string())
}

#[must_use]
pub const fn default_daemon_endpoint_kind() -> EndpointKind {
    az_endpoint_discovery::default_daemon_endpoint_kind()
}

/// The machine-global `azd` endpoint for one transport kind.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::EndpointRecord`] wrapping any error
/// [`az_endpoint_discovery::default_daemon_endpoint`] returns: an unsupported
/// `kind`, or a machine-local runtime directory that cannot be created.
pub fn default_daemon_endpoint(kind: EndpointKind) -> Result<Endpoint, DaemonRpcTransportError> {
    Ok(az_endpoint_discovery::default_daemon_endpoint(kind)?)
}

/// The machine-global `azd` endpoint record path.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::EndpointRecord`] wrapping any error
/// [`az_endpoint_discovery::daemon_endpoint_record_path`] returns: the data
/// home or its runtime directory cannot be prepared.
pub fn daemon_endpoint_record_path() -> Result<PathBuf, DaemonRpcTransportError> {
    Ok(az_endpoint_discovery::daemon_endpoint_record_path()?)
}

/// Read the machine-global `azd` endpoint record, if one exists.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::EndpointRecord`] wrapping any error
/// [`az_endpoint_discovery::read_daemon_endpoint_record`] returns: the record
/// path cannot be resolved, or the record cannot be read, parsed, or mapped to
/// a supported endpoint kind.
pub fn read_daemon_endpoint_record() -> Result<Option<DaemonEndpointRecord>, DaemonRpcTransportError>
{
    Ok(az_endpoint_discovery::read_daemon_endpoint_record()?)
}

/// Publish the machine-global `azd` endpoint record and return its guard.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::EndpointRecord`] wrapping any error
/// [`az_endpoint_discovery::write_daemon_endpoint_record`] returns: the record
/// path cannot be resolved, or the record cannot be encoded or committed.
pub fn write_daemon_endpoint_record(
    endpoint: &Endpoint,
) -> Result<DaemonEndpointRecordGuard, DaemonRpcTransportError> {
    Ok(az_endpoint_discovery::write_daemon_endpoint_record(
        endpoint,
    )?)
}

/// Remove the machine-global `azd` endpoint record if it is present.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::EndpointRecord`] wrapping any error
/// [`az_endpoint_discovery::remove_daemon_endpoint_record`] returns: the record
/// path cannot be resolved, or an existing record cannot be deleted.
pub fn remove_daemon_endpoint_record() -> Result<(), DaemonRpcTransportError> {
    Ok(az_endpoint_discovery::remove_daemon_endpoint_record()?)
}

/// The per-project `azd` endpoint for one transport kind.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::EndpointRecord`] wrapping any error
/// [`az_endpoint_discovery::project_daemon_endpoint`] returns: an unsupported
/// `kind`, or a machine-local runtime directory that cannot be created.
pub fn project_daemon_endpoint(
    kind: EndpointKind,
    project_root: &Path,
) -> Result<Endpoint, DaemonRpcTransportError> {
    Ok(az_endpoint_discovery::project_daemon_endpoint(
        kind,
        project_root,
    )?)
}

/// Publish the per-project `azd` endpoint record and return its guard.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::EndpointRecord`] wrapping any error
/// [`az_endpoint_discovery::write_project_daemon_endpoint_record`] returns: the
/// project record path cannot be resolved, or the record cannot be encoded or
/// committed.
pub fn write_project_daemon_endpoint_record(
    project_root: &Path,
    endpoint: &Endpoint,
) -> Result<DaemonEndpointRecordGuard, DaemonRpcTransportError> {
    Ok(az_endpoint_discovery::write_project_daemon_endpoint_record(
        project_root,
        endpoint,
    )?)
}

/// Read an endpoint record from an explicit path, if one exists.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::EndpointRecord`] wrapping any error
/// [`az_endpoint_discovery::read_daemon_endpoint_record_at`] returns: the file
/// exists but cannot be read or parsed, or names an unsupported endpoint kind.
pub fn read_daemon_endpoint_record_at(
    path: &Path,
) -> Result<Option<DaemonEndpointRecord>, DaemonRpcTransportError> {
    Ok(az_endpoint_discovery::read_daemon_endpoint_record_at(path)?)
}

/// Remove an endpoint record at an explicit path, ignoring a missing file.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::EndpointRecord`] wrapping any error
/// [`az_endpoint_discovery::remove_daemon_endpoint_record_at`] returns: pending
/// transactions cannot be recovered, or an existing file cannot be deleted.
pub fn remove_daemon_endpoint_record_at(path: &Path) -> Result<(), DaemonRpcTransportError> {
    Ok(az_endpoint_discovery::remove_daemon_endpoint_record_at(
        path,
    )?)
}

/// Atomically write an endpoint record to an explicit path.
///
/// # Errors
///
/// Returns [`DaemonRpcTransportError::EndpointRecord`] wrapping any error
/// [`az_endpoint_discovery::write_daemon_endpoint_record_at`] returns:
/// `endpoint` uses a non-public kind, or the record cannot be encoded or
/// committed.
pub fn write_daemon_endpoint_record_at(
    path: &Path,
    endpoint: &Endpoint,
) -> Result<(), DaemonRpcTransportError> {
    Ok(az_endpoint_discovery::write_daemon_endpoint_record_at(
        path, endpoint,
    )?)
}

fn start_tcp_server(
    daemon: AzDaemon,
    mut endpoint: Endpoint,
    control_shutdown: az_work::CancellationToken,
    run: Uuid,
) -> Result<AzDaemonRpcServer, DaemonRpcTransportError> {
    let listener = std::net::TcpListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    // Port 0 requests an ephemeral port, so the bound address is the one
    // callers must be handed back.
    endpoint.address = listener.local_addr()?.to_string();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let listener = TcpListener::from_std(listener)?;
            run_tcp_listener(
                daemon,
                thread_endpoint,
                listener,
                shutdown_rx,
                control_shutdown,
                run,
            )
            .await
        });
    });
    Ok(AzDaemonRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    })
}

// capnp-rpc keeps its connection state behind `Rc<RefCell<..>>`, so the
// bootstrap client held across the accept await is `!Send` by construction.
#[allow(clippy::future_not_send)]
async fn run_tcp_listener(
    daemon: AzDaemon,
    endpoint: Endpoint,
    listener: TcpListener,
    mut shutdown: oneshot::Receiver<()>,
    control_shutdown: az_work::CancellationToken,
    run: Uuid,
) -> Result<(), DaemonRpcTransportError> {
    let bootstrap = AzDaemonRpc::with_shutdown(daemon, control_shutdown, run).into_client();
    info!(endpoint = %endpoint.address, "azd RPC listener started");

    loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut shutdown).await {
            Either::Left((Ok((stream, peer)), _)) => {
                info!(endpoint = %endpoint.address, peer = %peer, "azd RPC client connected");
                // The connection task owns its own lifetime; the listener keeps
                // accepting instead of joining it.
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

fn run_threaded_local<F>(future: F)
where
    F: std::future::Future<Output = Result<(), DaemonRpcTransportError>> + 'static,
{
    let runtime = match Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            error!(error = %error, "azd RPC runtime failed to start");
            return;
        }
    };
    let local = LocalSet::new();
    let result = runtime.block_on(local.run_until(future));
    if let Err(error) = result {
        error!(error = %error, "azd RPC listener stopped with error");
    }
}

#[cfg(windows)]
fn start_named_pipe_server(
    daemon: AzDaemon,
    endpoint: Endpoint,
    control_shutdown: az_work::CancellationToken,
    run: Uuid,
) -> Result<AzDaemonRpcServer, DaemonRpcTransportError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<(), std::io::Error>>(1);
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
                    let _ = ready_tx.send(Err(error));
                    return Ok(());
                }
            };
            let _ = ready_tx.send(Ok(()));
            run_named_pipe_listener(
                daemon,
                thread_endpoint,
                server,
                shutdown_rx,
                control_shutdown,
                run,
            )
            .await
        });
    });
    ready_rx
        .recv()
        .map_err(|_| DaemonRpcTransportError::StartupChannelClosed)??;
    Ok(AzDaemonRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    })
}

#[cfg(not(windows))]
fn start_named_pipe_server(
    _daemon: AzDaemon,
    endpoint: Endpoint,
    _control_shutdown: az_work::CancellationToken,
    _run: Uuid,
) -> Result<AzDaemonRpcServer, DaemonRpcTransportError> {
    let kind = endpoint.kind;
    // Takes the endpoint by value to mirror the `cfg(windows)` implementation,
    // which hands it to the listener thread; here there is no listener to
    // hand it to.
    drop(endpoint);
    Err(DaemonRpcTransportError::UnsupportedEndpoint(kind))
}

// capnp-rpc keeps its connection state behind `Rc<RefCell<..>>`, so the
// bootstrap client held across the connect await is `!Send` by construction.
#[allow(clippy::future_not_send)]
#[cfg(windows)]
async fn run_named_pipe_listener(
    daemon: AzDaemon,
    endpoint: Endpoint,
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    mut shutdown: oneshot::Receiver<()>,
    control_shutdown: az_work::CancellationToken,
    run: Uuid,
) -> Result<(), DaemonRpcTransportError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let bootstrap = AzDaemonRpc::with_shutdown(daemon, control_shutdown, run).into_client();
    info!(endpoint = %endpoint.address, "azd named-pipe RPC listener started");

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
            // The connection task owns its own lifetime; the listener keeps
            // accepting instead of joining it.
            drop(az_rpc::spawn_twoparty_server(
                connected,
                bootstrap.client.clone(),
            ));
        }
    }
}

#[cfg(unix)]
fn start_unix_socket_server(
    daemon: AzDaemon,
    endpoint: Endpoint,
    control_shutdown: az_work::CancellationToken,
    run: Uuid,
) -> Result<AzDaemonRpcServer, DaemonRpcTransportError> {
    let path = std::path::Path::new(&endpoint.address);
    let listener = az_rpc::OwnedUnixListener::bind(path)?;
    listener.set_nonblocking(true)?;
    let (listener, socket_lease) = listener.into_parts();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let listener = tokio::net::UnixListener::from_std(listener)?;
            let result = run_unix_socket_listener(
                daemon,
                thread_endpoint,
                listener,
                shutdown_rx,
                control_shutdown,
                run,
            )
            .await;
            drop(socket_lease);
            result
        });
    });
    Ok(AzDaemonRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    })
}

#[cfg(not(unix))]
fn start_unix_socket_server(
    _daemon: AzDaemon,
    endpoint: Endpoint,
    _control_shutdown: az_work::CancellationToken,
    _run: Uuid,
) -> Result<AzDaemonRpcServer, DaemonRpcTransportError> {
    let kind = endpoint.kind;
    // Takes the endpoint by value to mirror the `cfg(unix)` implementation,
    // which hands it to the listener thread; here there is no listener to
    // hand it to.
    drop(endpoint);
    Err(DaemonRpcTransportError::UnsupportedEndpoint(kind))
}

// capnp-rpc keeps its connection state behind `Rc<RefCell<..>>`, so the
// bootstrap client held across the accept await is `!Send` by construction.
#[allow(clippy::future_not_send)]
#[cfg(unix)]
async fn run_unix_socket_listener(
    daemon: AzDaemon,
    endpoint: Endpoint,
    listener: tokio::net::UnixListener,
    mut shutdown: oneshot::Receiver<()>,
    control_shutdown: az_work::CancellationToken,
    run: Uuid,
) -> Result<(), DaemonRpcTransportError> {
    let bootstrap = AzDaemonRpc::with_shutdown(daemon, control_shutdown, run).into_client();
    info!(endpoint = %endpoint.address, "azd unix-socket RPC listener started");

    loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut shutdown).await {
            Either::Left((Ok((stream, _)), _)) => {
                // The connection task owns its own lifetime; the listener keeps
                // accepting instead of joining it.
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

#[cfg(test)]
mod tests {
    use az_project::{ProjectManifest, refresh_project_lock, write_project_manifest};
    use az_proto_core::{Capability, ServiceId, ServiceRole};
    use az_proto_daemon::{
        DAEMON_AUDIENCE, DAEMON_CONTROL_PERMISSION, DAEMON_PROJECTS_PERMISSION, ProjectRecord,
        RegisterProjectRootRequest, ShutdownDaemonRequest, ShutdownDaemonResult,
    };

    use super::*;

    fn project_capability() -> Capability {
        Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience(DAEMON_AUDIENCE)
            .with_permissions([DAEMON_PROJECTS_PERMISSION])
    }

    #[test]
    fn threaded_rpc_runtime_enables_timers() {
        let source = include_str!("transport.rs");
        let body = source
            .split("fn run_threaded_local<F>")
            .nth(1)
            .unwrap()
            .split("#[cfg(windows)]")
            .next()
            .unwrap();

        assert!(
            body.contains(".enable_io()") && body.contains(".enable_time()"),
            "azd RPC runtime must enable timers because daemon RPC handlers use tokio::time::timeout"
        );
    }

    #[test]
    fn tcp_transport_serves_az_daemon_rpc() {
        let server =
            start_az_daemon_rpc_server(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0")).unwrap();
        let endpoint = server.endpoint().clone();
        let temp = tempfile::tempdir().unwrap();
        write_project_manifest(
            temp.path(),
            &ProjectManifest::new("local.azd_transport", "Transport", "0.1.0"),
        )
        .unwrap();
        refresh_project_lock(temp.path()).unwrap();

        let runtime = Builder::new_current_thread().enable_io().build().unwrap();
        let local = LocalSet::new();
        local.block_on(&runtime, async move {
            let client = connect_az_daemon_rpc_client(&endpoint).await.unwrap();
            let mut request = client.register_project_root_request();
            (RegisterProjectRootRequest {
                capability: project_capability(),
                root: temp.path().to_string_lossy().into_owned(),
            })
            .to_capnp(request.get().init_request())
            .unwrap();
            let response = request.send().promise.await.unwrap();
            let project =
                ProjectRecord::from_capnp(response.get().unwrap().get_project().unwrap()).unwrap();
            assert_eq!(project.project_id, "local.azd_transport");
        });

        server.stop();
    }

    #[test]
    fn tcp_transport_shutdown_request_cancels_shared_token() {
        let shutdown = az_work::CancellationToken::new();
        let server = start_az_daemon_rpc_server_with_daemon_and_shutdown(
            AzDaemon::new(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            shutdown.clone(),
            Uuid::now_v7(),
        )
        .unwrap();
        let endpoint = server.endpoint().clone();

        let runtime = Builder::new_current_thread().enable_io().build().unwrap();
        let local = LocalSet::new();
        local.block_on(&runtime, async move {
            let client = connect_az_daemon_rpc_client(&endpoint).await.unwrap();
            let mut request = client.shutdown_request();
            (ShutdownDaemonRequest {
                capability: Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
                    .with_audience(DAEMON_AUDIENCE)
                    .with_permissions([DAEMON_CONTROL_PERMISSION]),
                reason: "transport test".to_string(),
            })
            .to_capnp(request.get().init_request())
            .unwrap();
            let response = request.send().promise.await.unwrap();
            let result =
                ShutdownDaemonResult::from_capnp(response.get().unwrap().get_result().unwrap())
                    .unwrap();
            assert!(result.accepted);
        });

        assert!(shutdown.is_cancelled());
        server.stop();
    }

    #[test]
    fn daemon_endpoint_record_round_trips_endpoint_and_pid() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.endpoint.toml");
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37612");

        write_daemon_endpoint_record_at(&path, &endpoint).unwrap();
        let record = read_daemon_endpoint_record_at(&path).unwrap().unwrap();

        assert_eq!(record.endpoint, endpoint);
        assert_eq!(record.process_id, std::process::id());
        assert_eq!(
            record.protocol_version,
            az_proto_core::ProtocolVersion::CURRENT
        );
    }

    #[test]
    fn daemon_endpoint_record_missing_file_is_absent() {
        let temp = tempfile::tempdir().unwrap();

        assert_eq!(
            read_daemon_endpoint_record_at(&temp.path().join("missing.toml")).unwrap(),
            None
        );
    }

    #[test]
    fn daemon_endpoint_record_guard_removes_file_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.endpoint.toml");
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37612");
        write_daemon_endpoint_record_at(&path, &endpoint).unwrap();
        let guard = DaemonEndpointRecordGuard::new(path.clone());

        drop(guard);

        assert!(!path.exists());
    }

    #[test]
    fn remove_daemon_endpoint_record_at_ignores_missing_record() {
        let temp = tempfile::tempdir().unwrap();

        remove_daemon_endpoint_record_at(&temp.path().join("missing.toml")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_transport_recovers_stale_socket_and_cleans_up_on_stop() {
        use std::os::unix::net::UnixListener;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.sock");
        let stale_listener = UnixListener::bind(&path).unwrap();
        drop(stale_listener);
        assert!(path.exists());

        let server = start_az_daemon_rpc_server(Endpoint::new(
            EndpointKind::UnixDomainSocket,
            path.to_string_lossy(),
        ))
        .unwrap();

        assert!(std::os::unix::net::UnixStream::connect(&path).is_ok());
        server.stop();
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn unix_transport_does_not_replace_live_socket() {
        use std::io::ErrorKind;
        use std::os::unix::net::{UnixListener, UnixStream};

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.sock");
        let live_listener = UnixListener::bind(&path).unwrap();

        let error = start_az_daemon_rpc_server(Endpoint::new(
            EndpointKind::UnixDomainSocket,
            path.to_string_lossy(),
        ))
        .err()
        .expect("a live listener must retain its endpoint");

        assert!(matches!(
            error,
            DaemonRpcTransportError::Io(ref error)
                if error.kind() == ErrorKind::AddrInUse
        ));
        assert!(UnixStream::connect(&path).is_ok());
        drop(live_listener);
    }
}
