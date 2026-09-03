//! Listener/transport plumbing for the `ObservabilityControl` server. Mirrors
//! the proven session-supervisor transport (a thread, a current-thread runtime,
//! a `LocalSet`, and an `az_rpc::spawn_twoparty_server` accept loop) since capnp
//! clients are `!Send` and must be built inside the serving thread.

use std::path::Path;
use std::thread;

use az_proto_core::{
    CapabilityGrantSet, Endpoint, EndpointKind, ServiceId, ServiceRole, decode_capability_grant_set,
};
use az_proto_observability::observability_capnp;
use az_rpc::AzRpcTransportError;
use futures::future::Either;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::oneshot;
use tokio::task::LocalSet;
use tracing::{error, info};
use uuid::Uuid;

use crate::{ObservabilityControlRpc, ObservabilityControlScope};

#[derive(Debug, Error)]
pub enum ObservabilityControlRpcTransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("capability grant decode error: {0}")]
    Decode(#[from] capnp::Error),

    #[error(transparent)]
    Rpc(#[from] AzRpcTransportError),

    #[error("endpoint kind `{0:?}` is not supported on this platform")]
    UnsupportedEndpoint(EndpointKind),

    #[error("RPC server thread failed to start")]
    StartupChannelClosed,
}

/// A running `ObservabilityControl` server. Dropping (or [`stop`](Self::stop))
/// signals the listener thread to exit and joins it, so the thread never
/// outlives this struct.
pub struct ObservabilityControlRpcServer {
    endpoint: Endpoint,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ObservabilityControlRpcServer {
    /// The bound endpoint (for TCP `:0`, the OS-assigned address).
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

impl Drop for ObservabilityControlRpcServer {
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

/// Decode the brokered capability grant set and start the server for its scope.
///
/// This is the entrypoint a service process calls at startup; the grant file
/// path arrives via `--observability-capability-grants`.
///
/// # Errors
/// Returns an error if the grant file cannot be read or decoded, the endpoint
/// cannot be bound, or the endpoint kind is unsupported on this platform.
pub fn start_observability_control_rpc_server_with_capability_grant_file(
    endpoint: Endpoint,
    scope: ObservabilityControlScope,
    service: ServiceId,
    role: ServiceRole,
    run: Uuid,
    capability_grants_file: impl AsRef<Path>,
) -> Result<ObservabilityControlRpcServer, ObservabilityControlRpcTransportError> {
    let bytes = std::fs::read(capability_grants_file.as_ref())?;
    let capability_grants = decode_capability_grant_set(&bytes)?;
    start_observability_control_rpc_server_with_grants(
        endpoint,
        scope,
        service,
        role,
        run,
        capability_grants,
    )
}

/// Start the server with an in-memory grant set (used by tests and callers that
/// already hold the brokered grants).
///
/// # Errors
/// Returns an error if the endpoint cannot be bound or the endpoint kind is
/// unsupported on this platform.
pub fn start_observability_control_rpc_server_with_grants(
    endpoint: Endpoint,
    scope: ObservabilityControlScope,
    service: ServiceId,
    role: ServiceRole,
    run: Uuid,
    capability_grants: CapabilityGrantSet,
) -> Result<ObservabilityControlRpcServer, ObservabilityControlRpcTransportError> {
    match endpoint.kind {
        EndpointKind::Tcp => {
            start_tcp_server(&endpoint, scope, service, role, run, capability_grants)
        }
        EndpointKind::WindowsNamedPipe => {
            start_named_pipe_server(endpoint, scope, service, role, run, capability_grants)
        }
        EndpointKind::UnixDomainSocket => {
            start_unix_socket_server(endpoint, scope, service, role, run, capability_grants)
        }
        EndpointKind::InProcess => Err(ObservabilityControlRpcTransportError::UnsupportedEndpoint(
            EndpointKind::InProcess,
        )),
    }
}

/// Connect a client to a process's `ObservabilityControl` endpoint (used by the
/// editor to dial a service).
///
/// # Errors
/// Returns an error if the endpoint cannot be reached or the endpoint kind is
/// unsupported on this platform.
pub async fn connect_observability_control_rpc_client(
    endpoint: &Endpoint,
) -> Result<observability_capnp::observability_control::Client, ObservabilityControlRpcTransportError>
{
    Ok(az_rpc::connect_twoparty_bootstrap(endpoint).await?)
}

fn bootstrap_client(
    scope: ObservabilityControlScope,
    capability_grants: CapabilityGrantSet,
    service: ServiceId,
    role: ServiceRole,
    run: Uuid,
) -> observability_capnp::observability_control::Client {
    ObservabilityControlRpc::new(scope, capability_grants, service, role, run).into_client()
}

fn run_threaded_local<F>(future: F)
where
    F: std::future::Future<Output = Result<(), ObservabilityControlRpcTransportError>> + 'static,
{
    let runtime = match Builder::new_current_thread().enable_io().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            error!(error = %error, "observability-control RPC runtime failed to start");
            return;
        }
    };
    let local = LocalSet::new();
    let result = runtime.block_on(local.run_until(future));
    if let Err(error) = result {
        error!(error = %error, "observability-control RPC listener stopped with error");
    }
}

fn start_tcp_server(
    endpoint: &Endpoint,
    scope: ObservabilityControlScope,
    service: ServiceId,
    role: ServiceRole,
    run: Uuid,
    capability_grants: CapabilityGrantSet,
) -> Result<ObservabilityControlRpcServer, ObservabilityControlRpcTransportError> {
    let listener = std::net::TcpListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?.to_string();
    let endpoint = Endpoint::new(EndpointKind::Tcp, address);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let listener = TcpListener::from_std(listener)?;
            let bootstrap = bootstrap_client(scope, capability_grants, service, role, run);
            run_tcp_listener(thread_endpoint, listener, shutdown_rx, bootstrap).await
        });
    });
    Ok(ObservabilityControlRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    })
}

// capnp's client hooks are `!Send` by construction; this listener runs on a
// `LocalSet`.
#[allow(clippy::future_not_send)]
async fn run_tcp_listener(
    endpoint: Endpoint,
    listener: TcpListener,
    mut shutdown: oneshot::Receiver<()>,
    bootstrap: observability_capnp::observability_control::Client,
) -> Result<(), ObservabilityControlRpcTransportError> {
    info!(endpoint = %endpoint.address, "observability-control RPC listener started");

    loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut shutdown).await {
            Either::Left((Ok((stream, peer)), _)) => {
                info!(endpoint = %endpoint.address, peer = %peer, "observability-control RPC client connected");
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

#[cfg(windows)]
fn start_named_pipe_server(
    endpoint: Endpoint,
    scope: ObservabilityControlScope,
    service: ServiceId,
    role: ServiceRole,
    run: Uuid,
    capability_grants: CapabilityGrantSet,
) -> Result<ObservabilityControlRpcServer, ObservabilityControlRpcTransportError> {
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
            let bootstrap = bootstrap_client(scope, capability_grants, service, role, run);
            run_named_pipe_listener(thread_endpoint, server, shutdown_rx, bootstrap).await
        });
    });
    ready_rx
        .recv()
        .map_err(|_| ObservabilityControlRpcTransportError::StartupChannelClosed)??;
    Ok(ObservabilityControlRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    })
}

#[cfg(not(windows))]
fn start_named_pipe_server(
    endpoint: Endpoint,
    _scope: ObservabilityControlScope,
    _service: ServiceId,
    _role: ServiceRole,
    _run: Uuid,
    _capability_grants: CapabilityGrantSet,
) -> Result<ObservabilityControlRpcServer, ObservabilityControlRpcTransportError> {
    Err(ObservabilityControlRpcTransportError::UnsupportedEndpoint(
        endpoint.kind,
    ))
}

#[cfg(windows)]
// capnp's client hooks are `!Send` by construction; this listener runs on a
// `LocalSet`.
#[allow(clippy::future_not_send)]
async fn run_named_pipe_listener(
    endpoint: Endpoint,
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    mut shutdown: oneshot::Receiver<()>,
    bootstrap: observability_capnp::observability_control::Client,
) -> Result<(), ObservabilityControlRpcTransportError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    info!(endpoint = %endpoint.address, "observability-control named-pipe RPC listener started");

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
    endpoint: Endpoint,
    scope: ObservabilityControlScope,
    service: ServiceId,
    role: ServiceRole,
    run: Uuid,
    capability_grants: CapabilityGrantSet,
) -> Result<ObservabilityControlRpcServer, ObservabilityControlRpcTransportError> {
    let listener = az_rpc::OwnedUnixListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    let (listener, socket_lease) = listener.into_parts();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let listener = tokio::net::UnixListener::from_std(listener)?;
            let bootstrap = bootstrap_client(scope, capability_grants, service, role, run);
            let result =
                run_unix_socket_listener(thread_endpoint, listener, shutdown_rx, bootstrap).await;
            drop(socket_lease);
            result
        });
    });
    Ok(ObservabilityControlRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    })
}

#[cfg(not(unix))]
// Signature must mirror the `cfg(unix)` twin above, which does consume the
// endpoint; this stub only reports that the kind is unsupported.
#[allow(clippy::needless_pass_by_value)]
fn start_unix_socket_server(
    endpoint: Endpoint,
    _scope: ObservabilityControlScope,
    _service: ServiceId,
    _role: ServiceRole,
    _run: Uuid,
    _capability_grants: CapabilityGrantSet,
) -> Result<ObservabilityControlRpcServer, ObservabilityControlRpcTransportError> {
    Err(ObservabilityControlRpcTransportError::UnsupportedEndpoint(
        endpoint.kind,
    ))
}

#[cfg(unix)]
async fn run_unix_socket_listener(
    endpoint: Endpoint,
    listener: tokio::net::UnixListener,
    mut shutdown: oneshot::Receiver<()>,
    bootstrap: observability_capnp::observability_control::Client,
) -> Result<(), ObservabilityControlRpcTransportError> {
    info!(endpoint = %endpoint.address, "observability-control unix-socket RPC listener started");

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

#[cfg(test)]
mod tests {
    use az_proto_core::{Capability, CapabilityGrantSet, ServiceId, ServiceRole};
    use az_proto_observability::{OBSERVABILITY_AUDIENCE, OBSERVABILITY_CONTROL_PERMISSION};
    use tokio::runtime::Builder;
    use tokio::task::LocalSet;

    use super::*;

    #[test]
    fn tcp_transport_serves_and_rejects_unbrokered_capability() {
        // Empty grant set: the listener should still serve, the request should
        // reach `authorize`, and the brokered-grant check should reject it. This
        // proves the accept loop + capability gate are wired end-to-end without
        // needing the full session brokering machinery (covered in az-session).
        let server = start_observability_control_rpc_server_with_grants(
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            ObservabilityControlScope::Session(uuid::Uuid::from_bytes([0x0d; 16])),
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Uuid::now_v7(),
            CapabilityGrantSet::new(),
        )
        .unwrap();
        let endpoint = server.endpoint().clone();

        let capability = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience(OBSERVABILITY_AUDIENCE)
            .with_session(uuid::Uuid::from_bytes([0x0d; 16]))
            .with_permissions([OBSERVABILITY_CONTROL_PERMISSION]);

        let runtime = Builder::new_current_thread().enable_io().build().unwrap();
        let local = LocalSet::new();
        local.block_on(&runtime, async move {
            let client = connect_observability_control_rpc_client(&endpoint)
                .await
                .unwrap();
            let mut request = client.list_channel_levels_request();
            (capability)
                .to_capnp(request.get().init_capability())
                .unwrap();
            match request.send().promise.await {
                Ok(_) => panic!("observability-control accepted an unbrokered capability"),
                Err(error) => assert!(
                    error.to_string().to_ascii_lowercase().contains("grant")
                        || error.to_string().to_ascii_lowercase().contains("brokered")
                        || error.to_string().to_ascii_lowercase().contains("empty"),
                    "unexpected error: {error}"
                ),
            }
        });

        server.stop();
    }

    #[cfg(unix)]
    #[test]
    fn unix_transport_recovers_stale_socket_and_cleans_up_on_stop() {
        use std::os::unix::net::{UnixListener, UnixStream};

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("observability-control.sock");
        drop(UnixListener::bind(&path).unwrap());

        let server = start_observability_control_rpc_server_with_grants(
            Endpoint::new(EndpointKind::UnixDomainSocket, path.to_string_lossy()),
            ObservabilityControlScope::Session(Uuid::from_bytes([0x0e; 16])),
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
            Uuid::now_v7(),
            CapabilityGrantSet::new(),
        )
        .unwrap();

        assert!(UnixStream::connect(&path).is_ok());
        server.stop();
        assert!(!path.exists());
    }
}
