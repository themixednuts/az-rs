//! Shared Cap'n Proto two-party transport helpers.
//!
//! Service crates own their domain implementations. This crate owns the common
//! endpoint-to-stream plumbing so editor, daemon, and session clients do not
//! need to depend on each other's process internals just to connect.
//!
//! # Handler contract: bridge blocking work via `spawn_blocking`
//!
//! Every service listener in Azoth (`azd`, `project-host`, `asset-processor`,
//! `runtime-host`, the session supervisor, `observability-control`, ...) runs
//! its Cap'n Proto RPC system on a single dedicated OS thread, driven by a
//! current-thread Tokio runtime and a single [`tokio::task::LocalSet`]. This
//! keeps `!Send` capnp client/server objects on one thread without extra
//! synchronization, but it also means the *entire* service — every in-flight
//! request and every connection's traffic — shares that one thread.
//!
//! **RPC handler implementations that block or run more than roughly 1ms of
//! CPU work must bridge to [`tokio::task::spawn_blocking`]** (or an
//! equivalent off-thread hop) rather than doing the work inline. A handler
//! that blocks the `LocalSet` thread stalls every other connection and request
//! the service is serving, including its own health/shutdown signaling. This
//! is a review-enforced contract (ADR 0031 Correction 5), not something a
//! guard lint checks for.
//!
//! Named-pipe listeners (Windows) should additionally create their pipes
//! through `az_rpc::create_owner_only_named_pipe` rather than
//! `ServerOptions::create`/`create_with_security_attributes_raw` directly, so
//! every service gets the same owner-only DACL instead of the platform
//! default.

use az_proto_core::{Endpoint, EndpointKind};
use capnp::Error as CapnpError;
use capnp::message::ReaderOptions;
use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use futures::io::{AsyncReadExt, BufReader, BufWriter};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::error;

#[cfg(windows)]
mod pipe_security;

#[cfg(unix)]
mod unix_socket;

#[cfg(windows)]
pub use pipe_security::create_owner_only_named_pipe;

#[cfg(unix)]
pub use unix_socket::{OwnedUnixListener, UnixSocketLease};

/// Traversal budget for trusted local service IPC.
///
/// Editor startup can legitimately move large catalogs across the service
/// boundary. The Cap'n Proto default is intentionally conservative for
/// untrusted network traffic; Azoth's project/session services are local,
/// authenticated endpoints whose payloads are validated by the protocol crates.
///
/// A 128 Mi-word limit permits at most 1 GiB of traversed Cap'n Proto words
/// while retaining a finite guard. Catalog transfer must be chunked before a
/// legitimate payload approaches this limit; raising it again is not a scaling
/// strategy.
pub const LOCAL_RPC_TRAVERSAL_LIMIT_IN_WORDS: usize = 128 * 1024 * 1024;
#[cfg(windows)]
const WINDOWS_NAMED_PIPE_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(30);

#[derive(Debug, Error)]
pub enum AzRpcTransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Cap'n Proto RPC error: {0}")]
    Capnp(#[from] CapnpError),

    #[error("endpoint kind `{0:?}` is not supported on this platform")]
    UnsupportedEndpoint(EndpointKind),
}

/// Connect to `endpoint` and return the remote vat's bootstrap capability.
///
/// # Errors
///
/// Returns [`AzRpcTransportError`] if the endpoint kind is unsupported on this
/// platform or the underlying stream cannot be connected.
pub async fn connect_twoparty_bootstrap<T>(endpoint: &Endpoint) -> Result<T, AzRpcTransportError>
where
    T: capnp::capability::FromClientHook,
{
    Ok(connect_twoparty_bootstrap_scoped(endpoint).await?.detach())
}

/// A client connection whose owner can gracefully flush and close its RPC
/// system before a short-lived local executor is torn down.
pub struct ScopedTwopartyClient<T> {
    client: T,
    disconnector: capnp_rpc::Disconnector<rpc_twoparty_capnp::Side>,
    closed: tokio::sync::watch::Receiver<()>,
}

impl<T> ScopedTwopartyClient<T> {
    #[must_use]
    pub const fn client(&self) -> &T {
        &self.client
    }

    #[must_use]
    pub fn detach(self) -> T {
        self.client
    }

    /// Flush queued outgoing messages and gracefully close the RPC system.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection is already broken, or if the queued
    /// outgoing messages cannot be flushed before the peer goes away.
    // capnp-rpc is single-threaded by construction: `Disconnector` holds the
    // connection state behind `Rc<RefCell<..>>`, so this future can never be
    // `Send` without replacing the RPC stack. Callers drive it on a `LocalSet`.
    #[allow(clippy::future_not_send)]
    pub async fn disconnect(self) -> Result<(), CapnpError> {
        self.disconnector.await
    }

    /// Passive notification for a peer-initiated or transport-level close.
    ///
    /// The receiver has no liveness role: it is only an edge for a stream
    /// owner that already holds this scoped connection.
    #[must_use]
    pub fn connection_closed(&self) -> tokio::sync::watch::Receiver<()> {
        self.closed.clone()
    }
}

/// Connect to `endpoint` and retain explicit ownership of graceful shutdown.
///
/// Use this form for clients hosted by a short-lived [`tokio::task::LocalSet`]
/// that must deliver a connection-terminal streaming call before returning.
///
/// # Errors
///
/// Returns [`AzRpcTransportError::UnsupportedEndpoint`] for an in-process
/// endpoint, which has no socket to connect to, or a transport error if the
/// TCP, named-pipe, or Unix-socket connection cannot be established.
pub async fn connect_twoparty_bootstrap_scoped<T>(
    endpoint: &Endpoint,
) -> Result<ScopedTwopartyClient<T>, AzRpcTransportError>
where
    T: capnp::capability::FromClientHook,
{
    match endpoint.kind {
        EndpointKind::Tcp => {
            let stream = TcpStream::connect(&endpoint.address).await?;
            stream.set_nodelay(true)?;
            Ok(scoped_client_from_stream(stream))
        }
        EndpointKind::WindowsNamedPipe => connect_named_pipe_client_scoped(endpoint).await,
        EndpointKind::UnixDomainSocket => connect_unix_socket_client_scoped(endpoint).await,
        EndpointKind::InProcess => Err(AzRpcTransportError::UnsupportedEndpoint(
            EndpointKind::InProcess,
        )),
    }
}

/// Spawn one server-side Cap'n Proto connection.
///
/// The returned task is the connection's lifetime. A listener may retain it
/// for its own connection accounting; dropping the handle preserves the
/// historical detached long-lived-server behavior.
pub fn spawn_twoparty_server<S>(
    stream: S,
    bootstrap: capnp::capability::Client,
) -> tokio::task::JoinHandle<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    spawn_rpc_system(stream, Some(bootstrap), rpc_twoparty_capnp::Side::Server)
}

fn scoped_client_from_stream<S, T>(stream: S) -> ScopedTwopartyClient<T>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
    T: capnp::capability::FromClientHook,
{
    let (reader, writer) = stream.compat().split();
    let network = Box::new(twoparty::VatNetwork::new(
        BufReader::new(reader),
        BufWriter::new(writer),
        rpc_twoparty_capnp::Side::Client,
        local_rpc_reader_options(),
    ));
    let mut rpc_system = RpcSystem::new(network, None);
    let disconnector = rpc_system.get_disconnector();
    let client = rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);
    let (closed_tx, closed) = tokio::sync::watch::channel(());
    tokio::task::spawn_local(async move {
        if let Err(error) = rpc_system.await {
            error!(error = %error, "Cap'n Proto RPC client failed");
        }
        drop(closed_tx);
    });
    ScopedTwopartyClient {
        client,
        disconnector,
        closed,
    }
}

fn spawn_rpc_system<S>(
    stream: S,
    bootstrap: Option<capnp::capability::Client>,
    side: rpc_twoparty_capnp::Side,
) -> tokio::task::JoinHandle<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    let (reader, writer) = stream.compat().split();
    let network = Box::new(twoparty::VatNetwork::new(
        BufReader::new(reader),
        BufWriter::new(writer),
        side,
        local_rpc_reader_options(),
    ));
    let rpc_system = RpcSystem::new(network, bootstrap);
    tokio::task::spawn_local(async move {
        if let Err(error) = rpc_system.await {
            error!(error = %error, "Cap'n Proto RPC connection failed");
        }
    })
}

fn local_rpc_reader_options() -> ReaderOptions {
    let mut options = ReaderOptions::new();
    options.traversal_limit_in_words(Some(LOCAL_RPC_TRAVERSAL_LIMIT_IN_WORDS));
    options
}

#[cfg(windows)]
async fn connect_named_pipe_client_scoped<T>(
    endpoint: &Endpoint,
) -> Result<ScopedTwopartyClient<T>, AzRpcTransportError>
where
    T: capnp::capability::FromClientHook,
{
    use std::time::Instant;
    use tokio::net::windows::named_pipe::ClientOptions;

    let deadline = Instant::now() + WINDOWS_NAMED_PIPE_BUSY_TIMEOUT;
    let client = loop {
        match ClientOptions::new().open(&endpoint.address) {
            Ok(client) => break client,
            Err(error) if is_windows_named_pipe_busy(&error) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("timed out waiting for named pipe `{}`", endpoint.address),
                    )
                    .into());
                }
                let address = endpoint.address.clone();
                tokio::task::spawn_blocking(move || wait_named_pipe_available(&address, remaining))
                    .await
                    .map_err(|error| {
                        std::io::Error::other(format!(
                            "named-pipe availability wait task failed: {error}"
                        ))
                    })??;
            }
            Err(error) => return Err(error.into()),
        }
    };
    Ok(scoped_client_from_stream(client))
}

#[cfg(windows)]
fn wait_named_pipe_available(address: &str, timeout: std::time::Duration) -> std::io::Result<()> {
    use windows::Win32::System::Pipes::WaitNamedPipeW;
    use windows::core::HSTRING;

    let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: `name` owns a NUL-terminated UTF-16 buffer for the duration of
    // the synchronous OS wait.
    let available = unsafe { WaitNamedPipeW(&HSTRING::from(address), timeout_ms) };
    if available.as_bool() {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn is_windows_named_pipe_busy(error: &std::io::Error) -> bool {
    const ERROR_PIPE_BUSY: i32 = 231;
    error.raw_os_error() == Some(ERROR_PIPE_BUSY)
}

#[cfg(not(windows))]
async fn connect_named_pipe_client_scoped<T>(
    endpoint: &Endpoint,
) -> Result<ScopedTwopartyClient<T>, AzRpcTransportError>
where
    T: capnp::capability::FromClientHook,
{
    Err(AzRpcTransportError::UnsupportedEndpoint(endpoint.kind))
}

#[cfg(unix)]
async fn connect_unix_socket_client_scoped<T>(
    endpoint: &Endpoint,
) -> Result<ScopedTwopartyClient<T>, AzRpcTransportError>
where
    T: capnp::capability::FromClientHook,
{
    let stream = tokio::net::UnixStream::connect(&endpoint.address).await?;
    Ok(scoped_client_from_stream(stream))
}

#[cfg(not(unix))]
#[expect(
    clippy::unused_async,
    reason = "signature parity with the unix implementation awaited by connect_twoparty_bootstrap"
)]
async fn connect_unix_socket_client_scoped<T>(
    endpoint: &Endpoint,
) -> Result<ScopedTwopartyClient<T>, AzRpcTransportError>
where
    T: capnp::capability::FromClientHook,
{
    Err(AzRpcTransportError::UnsupportedEndpoint(endpoint.kind))
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_rpc_reader_options_allow_editor_catalog_payloads() {
        assert_eq!(
            super::local_rpc_reader_options().traversal_limit_in_words,
            Some(super::LOCAL_RPC_TRAVERSAL_LIMIT_IN_WORDS)
        );
        assert!(
            super::LOCAL_RPC_TRAVERSAL_LIMIT_IN_WORDS
                > capnp::message::DEFAULT_READER_OPTIONS
                    .traversal_limit_in_words
                    .expect("default traversal limit")
        );
        const { assert!(super::LOCAL_RPC_TRAVERSAL_LIMIT_IN_WORDS == 128 * 1024 * 1024) };
    }

    #[cfg(windows)]
    #[test]
    fn windows_named_pipe_busy_is_retryable() {
        let error = std::io::Error::from_raw_os_error(231);
        assert!(super::is_windows_named_pipe_busy(&error));

        let error = std::io::Error::from_raw_os_error(2);
        assert!(!super::is_windows_named_pipe_busy(&error));
    }

    #[cfg(windows)]
    #[test]
    fn windows_named_pipe_busy_wait_covers_long_local_rpc() {
        assert!(super::WINDOWS_NAMED_PIPE_BUSY_TIMEOUT.as_secs() >= 1800);
    }
}
