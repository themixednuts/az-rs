use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::thread;

use az_filesystem::AzothDataHome;
use az_proto_core::{Endpoint, EndpointKind};
use az_proto_session::{SessionSupervisorIdentity, session_capnp};
use az_rpc::AzRpcTransportError;
use futures::future::Either;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::oneshot;
use tokio::task::LocalSet;
use tracing::{error, info};
use uuid::Uuid;

use crate::status_broker::{
    SessionStatusBroker, SessionStatusPublisher, run_status_publications,
    session_status_publication_channel,
};
use crate::{SessionError, SessionSupervisorCommandSender, SessionSupervisorRpc};

#[derive(Debug, Error)]
pub enum SessionRpcTransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Rpc(#[from] AzRpcTransportError),

    #[error("session supervisor error: {0}")]
    Session(#[from] SessionError),

    #[error("endpoint kind `{0:?}` is not supported on this platform")]
    UnsupportedEndpoint(EndpointKind),

    #[error("RPC server thread failed to start")]
    StartupChannelClosed,

    #[error("session-supervisor identity lock was poisoned")]
    IdentityLockPoisoned,
}

pub struct SessionSupervisorRpcServer {
    endpoint: Endpoint,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
    service_run: Arc<RwLock<Uuid>>,
    supervision_identity: Arc<RwLock<Option<SessionSupervisorIdentity>>>,
    status_publisher: SessionStatusPublisher,
    #[cfg(any(test, feature = "test-support"))]
    _test_command_receiver: Option<crossbeam_channel::Receiver<crate::SessionSupervisorCommand>>,
}

impl SessionSupervisorRpcServer {
    #[must_use]
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn set_run(&self, run: Uuid) {
        *self
            .service_run
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = run;
    }

    /// Publishes the identity this transport reports to `supervisionIdentity`
    /// callers.
    ///
    /// # Errors
    ///
    /// Returns [`SessionRpcTransportError::IdentityLockPoisoned`] if a previous
    /// writer panicked while holding the identity lock.
    pub fn set_supervision_identity(
        &self,
        identity: SessionSupervisorIdentity,
    ) -> Result<(), SessionRpcTransportError> {
        *self
            .supervision_identity
            .write()
            .map_err(|_| SessionRpcTransportError::IdentityLockPoisoned)? = Some(identity);
        Ok(())
    }

    #[must_use]
    pub fn status_publisher(&self) -> SessionStatusPublisher {
        self.status_publisher.clone()
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

impl Drop for SessionSupervisorRpcServer {
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

/// Start the production RPC transport bound to the sessiond command owner.
///
/// # Errors
///
/// Returns [`SessionRpcTransportError::UnsupportedEndpoint`] if `endpoint` is
/// [`EndpointKind::InProcess`] or names a transport this build does not carry
/// (named pipes off Windows, unix sockets off Unix);
/// [`SessionRpcTransportError::Io`] if the listener cannot bind, be switched to
/// non-blocking mode, or report its bound address, or if the named pipe cannot
/// be created; [`SessionRpcTransportError::StartupChannelClosed`] if the
/// named-pipe listener thread dies before reporting that result; and
/// [`SessionRpcTransportError::Session`] if the transport was assembled without
/// a status publisher.
pub fn start_session_supervisor_rpc_server_with_command_sender(
    project_root: impl AsRef<Path>,
    data_home: AzothDataHome,
    endpoint: Endpoint,
    controlled_session: impl Into<String>,
    command_sender: SessionSupervisorCommandSender,
) -> Result<SessionSupervisorRpcServer, SessionRpcTransportError> {
    start_session_supervisor_rpc_server_with_options(
        project_root.as_ref().to_path_buf(),
        endpoint,
        SessionSupervisorRpcOptions {
            data_home,
            controlled_session: Some(controlled_session.into()),
            command_sender: Some(command_sender),
            service_run: Arc::new(RwLock::new(Uuid::nil())),
            supervision_identity: Arc::new(RwLock::new(None)),
            status_publisher: None,
            status_publications: None,
        },
    )
}

/// Start a transport that serves an already-built [`crate::SessionManager`],
/// bypassing the sessiond command owner. Test-support only.
///
/// # Errors
///
/// Returns [`SessionRpcTransportError::UnsupportedEndpoint`] unless `endpoint`
/// is [`EndpointKind::Tcp`], and [`SessionRpcTransportError::Io`] if the TCP
/// listener cannot bind, be switched to non-blocking mode, or report its bound
/// address.
#[cfg(any(test, feature = "test-support"))]
pub fn start_session_supervisor_rpc_server_with_manager(
    manager: crate::SessionManager,
    endpoint: Endpoint,
    controlled_session: impl Into<String>,
) -> Result<SessionSupervisorRpcServer, SessionRpcTransportError> {
    let (command_sender, command_receiver) = crate::session_supervisor_command_channel();
    match endpoint.kind {
        EndpointKind::Tcp => start_tcp_server_with_manager(
            manager,
            endpoint,
            controlled_session.into(),
            command_sender,
            command_receiver,
        ),
        EndpointKind::WindowsNamedPipe
        | EndpointKind::UnixDomainSocket
        | EndpointKind::InProcess => {
            Err(SessionRpcTransportError::UnsupportedEndpoint(endpoint.kind))
        }
    }
}

#[derive(Debug)]
struct SessionSupervisorRpcOptions {
    data_home: AzothDataHome,
    controlled_session: Option<String>,
    command_sender: Option<SessionSupervisorCommandSender>,
    service_run: Arc<RwLock<Uuid>>,
    supervision_identity: Arc<RwLock<Option<SessionSupervisorIdentity>>>,
    status_publisher: Option<SessionStatusPublisher>,
    status_publications:
        Option<tokio::sync::mpsc::Receiver<crate::status_broker::SessionStatusPublication>>,
}

fn start_session_supervisor_rpc_server_with_options(
    project_root: PathBuf,
    endpoint: Endpoint,
    mut options: SessionSupervisorRpcOptions,
) -> Result<SessionSupervisorRpcServer, SessionRpcTransportError> {
    let (status_publisher, status_publications) = session_status_publication_channel();
    options.status_publisher = Some(status_publisher);
    options.status_publications = Some(status_publications);
    match endpoint.kind {
        EndpointKind::Tcp => start_tcp_server(project_root, &endpoint, options),
        EndpointKind::WindowsNamedPipe => start_named_pipe_server(project_root, endpoint, options),
        EndpointKind::UnixDomainSocket => {
            start_unix_socket_server(project_root, &endpoint, options)
        }
        EndpointKind::InProcess => Err(SessionRpcTransportError::UnsupportedEndpoint(
            EndpointKind::InProcess,
        )),
    }
}

/// Dial `endpoint` and take its bootstrap session-supervisor capability.
///
/// # Errors
///
/// Returns [`SessionRpcTransportError::Rpc`] if the endpoint kind has no client
/// on this platform, the connection cannot be established, or the two-party
/// bootstrap handshake fails.
pub async fn connect_session_supervisor_rpc_client(
    endpoint: &Endpoint,
) -> Result<session_capnp::session_supervisor::Client, SessionRpcTransportError> {
    Ok(az_rpc::connect_twoparty_bootstrap(endpoint).await?)
}

fn session_supervisor_client(
    project_root: PathBuf,
    options: &SessionSupervisorRpcOptions,
    status_broker: Rc<SessionStatusBroker>,
) -> Result<session_capnp::session_supervisor::Client, SessionError> {
    let Some(controlled_session) = &options.controlled_session else {
        return Err(SessionError::InvalidSessionCommand {
            message: "endpoint-hosted session-supervisor RPC requires a controlled session"
                .to_string(),
        });
    };

    let command_sender =
        options
            .command_sender
            .as_ref()
            .ok_or_else(|| SessionError::InvalidSessionCommand {
                message: "endpoint-hosted session-supervisor RPC requires a command sender"
                    .to_string(),
            })?;
    Ok(SessionSupervisorRpc::new_with_command_sender(
        project_root,
        options.data_home.clone(),
        controlled_session.clone(),
        command_sender.clone(),
    )?
    .with_status_broker(status_broker)
    .with_service_run(Arc::clone(&options.service_run))
    .with_supervision_identity(Arc::clone(&options.supervision_identity))
    .with_descriptor_grants()
    .into_client())
}

fn start_tcp_server(
    project_root: PathBuf,
    endpoint: &Endpoint,
    options: SessionSupervisorRpcOptions,
) -> Result<SessionSupervisorRpcServer, SessionRpcTransportError> {
    let listener = std::net::TcpListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?.to_string();
    let endpoint = Endpoint::new(EndpointKind::Tcp, address);
    let service_run = Arc::clone(&options.service_run);
    let supervision_identity = Arc::clone(&options.supervision_identity);
    let status_publisher =
        options
            .status_publisher
            .clone()
            .ok_or_else(|| SessionError::InvalidSessionCommand {
                message: "session-supervisor transport has no status publisher".to_string(),
            })?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let listener = TcpListener::from_std(listener)?;
            run_tcp_listener(
                project_root,
                thread_endpoint,
                listener,
                shutdown_rx,
                options,
            )
            .await
        });
    });
    Ok(SessionSupervisorRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
        service_run,
        supervision_identity,
        status_publisher,
        #[cfg(any(test, feature = "test-support"))]
        _test_command_receiver: None,
    })
}

#[cfg(any(test, feature = "test-support"))]
fn start_tcp_server_with_manager(
    manager: crate::SessionManager,
    mut endpoint: Endpoint,
    controlled_session: String,
    command_sender: SessionSupervisorCommandSender,
    command_receiver: crossbeam_channel::Receiver<crate::SessionSupervisorCommand>,
) -> Result<SessionSupervisorRpcServer, SessionRpcTransportError> {
    let listener = std::net::TcpListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    // Rewrite the requested endpoint into the bound one: a `:0` request only
    // learns its port here.
    endpoint.kind = EndpointKind::Tcp;
    endpoint.address = listener.local_addr()?.to_string();
    let service_run = Arc::new(RwLock::new(Uuid::nil()));
    let supervision_identity = Arc::new(RwLock::new(None));
    let rpc_service_run = Arc::clone(&service_run);
    let rpc_supervision_identity = Arc::clone(&supervision_identity);
    let (status_publisher, _status_publications) = session_status_publication_channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let listener = TcpListener::from_std(listener)?;
            let bootstrap = SessionSupervisorRpc::with_manager_and_command_sender(
                manager,
                controlled_session,
                command_sender,
            )
            .with_service_run(rpc_service_run)
            .with_supervision_identity(rpc_supervision_identity)
            .with_descriptor_grants()
            .into_client();
            run_tcp_listener_with_bootstrap(thread_endpoint, listener, shutdown_rx, bootstrap).await
        });
    });
    Ok(SessionSupervisorRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
        service_run,
        supervision_identity,
        status_publisher,
        _test_command_receiver: Some(command_receiver),
    })
}

// Cannot be made `Send`: the bootstrap capability and the status broker are
// capnp-rpc / `Rc` values that are `!Send` by construction, and this future is
// only ever driven by `run_threaded_local`'s per-thread `LocalSet`.
#[allow(clippy::future_not_send)]
async fn run_tcp_listener(
    project_root: PathBuf,
    endpoint: Endpoint,
    listener: TcpListener,
    shutdown: oneshot::Receiver<()>,
    mut options: SessionSupervisorRpcOptions,
) -> Result<(), SessionRpcTransportError> {
    let status_broker = SessionStatusBroker::new();
    let publications =
        options
            .status_publications
            .take()
            .ok_or_else(|| SessionError::InvalidSessionCommand {
                message: "session-supervisor transport has no status publication receiver"
                    .to_string(),
            })?;
    tokio::task::spawn_local(run_status_publications(
        Rc::clone(&status_broker),
        publications,
    ));
    let bootstrap = session_supervisor_client(project_root, &options, status_broker)?;
    run_tcp_listener_with_bootstrap(endpoint, listener, shutdown, bootstrap).await
}

// Cannot be made `Send`: `bootstrap` is a capnp-rpc `Client`, which is `!Send`
// by construction; this future runs on `run_threaded_local`'s `LocalSet`.
#[allow(clippy::future_not_send)]
async fn run_tcp_listener_with_bootstrap(
    endpoint: Endpoint,
    listener: TcpListener,
    mut shutdown: oneshot::Receiver<()>,
    bootstrap: session_capnp::session_supervisor::Client,
) -> Result<(), SessionRpcTransportError> {
    info!(endpoint = %endpoint.address, "session-supervisor RPC listener started");
    loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut shutdown).await {
            Either::Left((Ok((stream, peer)), _)) => {
                info!(endpoint = %endpoint.address, peer = %peer, "session-supervisor RPC client connected");
                // Detached on purpose: the spawned RPC system owns the
                // connection until the peer goes away.
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
    F: std::future::Future<Output = Result<(), SessionRpcTransportError>> + 'static,
{
    let runtime = match Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            error!(error = %error, "session-supervisor RPC runtime failed to start");
            return;
        }
    };
    let local = LocalSet::new();
    let result = runtime.block_on(local.run_until(future));
    if let Err(error) = result {
        error!(error = %error, "session-supervisor RPC listener stopped with error");
    }
}

#[cfg(windows)]
fn start_named_pipe_server(
    project_root: PathBuf,
    endpoint: Endpoint,
    options: SessionSupervisorRpcOptions,
) -> Result<SessionSupervisorRpcServer, SessionRpcTransportError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let service_run = Arc::clone(&options.service_run);
    let supervision_identity = Arc::clone(&options.supervision_identity);
    let status_publisher =
        options
            .status_publisher
            .clone()
            .ok_or_else(|| SessionError::InvalidSessionCommand {
                message: "session-supervisor transport has no status publisher".to_string(),
            })?;
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
            run_named_pipe_listener(project_root, thread_endpoint, server, shutdown_rx, options)
                .await
        });
    });
    ready_rx
        .recv()
        .map_err(|_| SessionRpcTransportError::StartupChannelClosed)??;
    Ok(SessionSupervisorRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
        service_run,
        supervision_identity,
        status_publisher,
        #[cfg(any(test, feature = "test-support"))]
        _test_command_receiver: None,
    })
}

#[cfg(not(windows))]
fn start_named_pipe_server(
    _project_root: PathBuf,
    endpoint: Endpoint,
    _options: SessionSupervisorRpcOptions,
) -> Result<SessionSupervisorRpcServer, SessionRpcTransportError> {
    Err(SessionRpcTransportError::UnsupportedEndpoint(endpoint.kind))
}

// Cannot be made `Send`: the bootstrap capability and the status broker are
// capnp-rpc / `Rc` values that are `!Send` by construction, and this future is
// only ever driven by `run_threaded_local`'s per-thread `LocalSet`.
#[allow(clippy::future_not_send)]
#[cfg(windows)]
async fn run_named_pipe_listener(
    project_root: PathBuf,
    endpoint: Endpoint,
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    mut shutdown: oneshot::Receiver<()>,
    mut options: SessionSupervisorRpcOptions,
) -> Result<(), SessionRpcTransportError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let status_broker = SessionStatusBroker::new();
    let publications =
        options
            .status_publications
            .take()
            .ok_or_else(|| SessionError::InvalidSessionCommand {
                message: "session-supervisor transport has no status publication receiver"
                    .to_string(),
            })?;
    tokio::task::spawn_local(run_status_publications(
        Rc::clone(&status_broker),
        publications,
    ));
    let bootstrap = session_supervisor_client(project_root, &options, status_broker)?;
    info!(endpoint = %endpoint.address, "session-supervisor named-pipe RPC listener started");
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
            // Detached on purpose: the spawned RPC system owns the
            // connection until the peer goes away.
            drop(az_rpc::spawn_twoparty_server(
                connected,
                bootstrap.client.clone(),
            ));
        }
    }
}

#[cfg(unix)]
fn start_unix_socket_server(
    project_root: PathBuf,
    endpoint: &Endpoint,
    options: SessionSupervisorRpcOptions,
) -> Result<SessionSupervisorRpcServer, SessionRpcTransportError> {
    let listener = az_rpc::OwnedUnixListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    let (listener, socket_lease) = listener.into_parts();
    let service_run = Arc::clone(&options.service_run);
    let supervision_identity = Arc::clone(&options.supervision_identity);
    let status_publisher =
        options
            .status_publisher
            .clone()
            .ok_or_else(|| SessionError::InvalidSessionCommand {
                message: "session-supervisor transport has no status publisher".to_string(),
            })?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let listener = tokio::net::UnixListener::from_std(listener)?;
            let result = run_unix_socket_listener(
                project_root,
                thread_endpoint,
                listener,
                shutdown_rx,
                options,
            )
            .await;
            drop(socket_lease);
            result
        });
    });
    Ok(SessionSupervisorRpcServer {
        endpoint: endpoint.clone(),
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
        service_run,
        supervision_identity,
        status_publisher,
        #[cfg(any(test, feature = "test-support"))]
        _test_command_receiver: None,
    })
}

#[cfg(not(unix))]
fn start_unix_socket_server(
    _project_root: PathBuf,
    endpoint: &Endpoint,
    _options: SessionSupervisorRpcOptions,
) -> Result<SessionSupervisorRpcServer, SessionRpcTransportError> {
    Err(SessionRpcTransportError::UnsupportedEndpoint(endpoint.kind))
}

// Cannot be made `Send`: the bootstrap capability and the status broker are
// capnp-rpc / `Rc` values that are `!Send` by construction, and this future is
// only ever driven by `run_threaded_local`'s per-thread `LocalSet`.
#[allow(clippy::future_not_send)]
#[cfg(unix)]
async fn run_unix_socket_listener(
    project_root: PathBuf,
    endpoint: Endpoint,
    listener: tokio::net::UnixListener,
    mut shutdown: oneshot::Receiver<()>,
    mut options: SessionSupervisorRpcOptions,
) -> Result<(), SessionRpcTransportError> {
    let status_broker = SessionStatusBroker::new();
    let publications =
        options
            .status_publications
            .take()
            .ok_or_else(|| SessionError::InvalidSessionCommand {
                message: "session-supervisor transport has no status publication receiver"
                    .to_string(),
            })?;
    tokio::task::spawn_local(run_status_publications(
        Rc::clone(&status_broker),
        publications,
    ));
    let bootstrap = session_supervisor_client(project_root, &options, status_broker)?;
    info!(endpoint = %endpoint.address, "session-supervisor unix-socket RPC listener started");
    loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut shutdown).await {
            Either::Left((Ok((stream, _)), _)) => {
                // Detached on purpose: the spawned RPC system owns the
                // connection until the peer goes away.
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
    use az_filesystem::AzothDataHome;
    use az_proto_core::{Capability, ServiceDescriptor, ServiceRole};
    use az_proto_session::{SESSION_READ_PERMISSION, SESSION_SUPERVISOR_AUDIENCE};

    use crate::{SESSION_MANIFEST_FILE, SessionManifest, SessionState};

    use super::*;

    fn write_request_capability(
        capability: &Capability,
        builder: az_proto_core::core_capnp::capability::Builder<'_>,
    ) {
        (capability).to_capnp(builder).unwrap();
    }

    fn editor_supervisor_capability(
        descriptor: &ServiceDescriptor,
        manifest: &SessionManifest,
        permissions: &[&str],
    ) -> Capability {
        descriptor
            .brokered_capability_template(
                ServiceRole::Editor,
                SESSION_SUPERVISOR_AUDIENCE,
                permissions,
                Some(manifest.id.0),
            )
            .expect("session-supervisor descriptor grants editor capability")
    }

    fn write_manifest(manifest: &SessionManifest) {
        std::fs::write(
            manifest.run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(manifest).unwrap(),
        )
        .unwrap();
    }

    fn write_active_manifest(root: &Path, slug: &str) -> SessionManifest {
        az_project::write_project_manifest(
            root,
            &az_project::ProjectManifest::new(
                "local.az_session_transport",
                "Session Transport Test",
                "0.1.0",
            ),
        )
        .unwrap();
        az_project::refresh_project_lock(root).unwrap();
        let session_id = crate::SessionId::new();
        let data_home = AzothDataHome::new(root.join("azoth-home"));
        let data_paths = data_home.project("Session Transport Test", root);
        data_paths.prepare().unwrap();
        let run_dir = data_paths.sessions_dir().join(session_id.to_string());
        std::fs::create_dir_all(&run_dir).unwrap();
        let mut manifest = SessionManifest::new(
            session_id,
            "local.az_session_transport".to_string(),
            slug.to_string(),
            root.to_path_buf(),
            root.to_path_buf(),
            run_dir,
            0,
        );
        manifest.state = SessionState::Active;
        write_manifest(&manifest);
        manifest
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
            "session-supervisor RPC runtime must enable timers for timeout-based RPC handlers"
        );
    }

    #[test]
    fn tcp_transport_serves_session_supervisor_rpc() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = write_active_manifest(temp.path(), "editor");
        let mut descriptor = az_service_catalog::session_supervisor_service_descriptor(
            manifest.id.0,
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        manifest.upsert_service_descriptor(&descriptor, 1).unwrap();
        write_manifest(&manifest);

        let manager = crate::SessionManager::with_data_home(
            temp.path(),
            AzothDataHome::new(temp.path().join("azoth-home")),
        )
        .unwrap();
        let server = start_session_supervisor_rpc_server_with_manager(
            manager,
            descriptor.endpoint.clone(),
            &manifest.slug,
        )
        .unwrap();
        descriptor.endpoint = server.endpoint().clone();
        manifest.upsert_service_descriptor(&descriptor, 2).unwrap();
        write_manifest(&manifest);
        let endpoint = server.endpoint().clone();
        let read_capability =
            editor_supervisor_capability(&descriptor, &manifest, &[SESSION_READ_PERMISSION]);
        let wrong_token_capability = read_capability.clone().with_token_hash([0xff]);

        let runtime = Builder::new_current_thread().enable_io().build().unwrap();
        let local = LocalSet::new();
        local.block_on(&runtime, async move {
            let client = connect_session_supervisor_rpc_client(&endpoint)
                .await
                .unwrap();
            let mut request = client.list_request();
            write_request_capability(&read_capability, request.get().init_capability());
            let response = request.send().promise.await.unwrap();
            let sessions = response.get().unwrap().get_sessions().unwrap();
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions.get(0).get_slug().unwrap(), "editor");

            let mut request = client.list_request();
            write_request_capability(&wrong_token_capability, request.get().init_capability());
            match request.send().promise.await {
                Ok(_) => panic!("session-supervisor accepted a mismatched token"),
                Err(error) => assert!(
                    error.to_string().contains("not brokered"),
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
        let path = temp.path().join("session-supervisor.sock");
        drop(UnixListener::bind(&path).unwrap());
        let (command_sender, _command_receiver) = crate::session_supervisor_command_channel();

        let server = start_session_supervisor_rpc_server_with_command_sender(
            temp.path(),
            AzothDataHome::new(temp.path().join("azoth-home")),
            Endpoint::new(EndpointKind::UnixDomainSocket, path.to_string_lossy()),
            "editor",
            command_sender,
        )
        .unwrap();

        assert!(UnixStream::connect(&path).is_ok());
        server.stop();
        assert!(!path.exists());
    }

    #[test]
    fn transport_has_one_causal_control_constructor() {
        let source = az_architecture_guard::production_source_without_cfg_test_modules(
            include_str!("transport.rs"),
        );
        assert!(source.contains("start_session_supervisor_rpc_server_with_command_sender"));
        for removed in [
            "start_session_supervisor_rpc_server_with_shutdown_request",
            "start_session_supervisor_rpc_server_with_service_requests",
            "StartServicesRequestState",
        ] {
            assert!(
                !source.contains(removed),
                "legacy control surface remains: {removed}"
            );
        }
    }
}
