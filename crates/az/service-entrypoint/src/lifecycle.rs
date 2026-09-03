//! Generic, authenticated process-lifetime control for generated services.
//!
//! This module owns neither a role RPC nor a process supervisor. It only
//! authenticates a shutdown request, resolves the service-owned lifetime edge,
//! and owns the listener thread until the entrypoint has joined its role work.

use std::thread;
use std::{error::Error as StdError, fmt, net::SocketAddr, str::FromStr};

use az_proto_core::{
    Capability, CapabilityGrantSet, CapabilityGrantSetValidationError, Endpoint, EndpointKind,
    SERVICE_LIFECYCLE_AUDIENCE, SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION, core_capnp,
    decode_capability_grant_set,
};
use az_rpc::AzRpcTransportError;
use az_service_catalog::{
    PROJECT_SERVICE_LIFECYCLE_CAPABILITY_REQUIREMENTS,
    SESSION_SERVICE_LIFECYCLE_CAPABILITY_REQUIREMENTS,
};
use futures::future::Either;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::oneshot;
use tokio::task::LocalSet;

use crate::{ServiceArgs, ServiceScope};

#[derive(Debug, Error)]
pub enum ServiceLifecycleControlError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("capability grant decode error: {0}")]
    Decode(#[from] capnp::Error),
    // Boxed: inline this validation error is 128 bytes and set the size of
    // every `Result<_, ServiceLifecycleControlError>` in this module
    // (`clippy::result_large_err`). The hand-written `From` below keeps `?`
    // converting the unboxed error exactly as `#[from]` used to.
    #[error("invalid lifecycle capability grant set: {0}")]
    InvalidCapabilityGrantSet(#[source] Box<CapabilityGrantSetValidationError>),
    #[error(transparent)]
    Rpc(#[from] AzRpcTransportError),
    #[error("lifecycle endpoint kind `{0:?}` is not supported on this platform")]
    UnsupportedEndpoint(EndpointKind),
    #[error("lifecycle TCP endpoint `{0}` is not a socket address")]
    InvalidTcpEndpoint(String),
    #[error("lifecycle TCP endpoint `{0}` is not loopback")]
    NonLoopbackTcpEndpoint(SocketAddr),
    #[error("lifecycle listener startup channel closed")]
    StartupChannelClosed,
    #[error("lifecycle listener thread panicked")]
    ListenerThreadPanicked,
}

/// Boxes the payload on the way in, so `?` still converts a bare
/// [`CapabilityGrantSetValidationError`] the way `#[from]` did before the
/// variant was boxed.
impl From<CapabilityGrantSetValidationError> for ServiceLifecycleControlError {
    fn from(source: CapabilityGrantSetValidationError) -> Self {
        Self::InvalidCapabilityGrantSet(Box::new(source))
    }
}

#[derive(Debug, Error)]
pub enum ServiceLifetimeError {
    #[error("lifecycle control endpoint closed without an authenticated shutdown request")]
    ControlLost,

    #[error("lifecycle control endpoint failed: {0}")]
    ControlFailed(String),
}

/// One named failure observed while a generated service terminates.
#[derive(Debug)]
pub struct ServiceTerminationFailure {
    operation: &'static str,
    source: Box<dyn StdError + Send + Sync>,
}

impl ServiceTerminationFailure {
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    #[must_use]
    pub fn error(&self) -> &(dyn StdError + Send + Sync + 'static) {
        self.source.as_ref()
    }
}

impl fmt::Display for ServiceTerminationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} failed: {}", self.operation, self.source)
    }
}

impl StdError for ServiceTerminationFailure {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Complete terminal evidence from one generated service entrypoint.
#[derive(Debug)]
pub struct ServiceTerminationError {
    failures: Vec<ServiceTerminationFailure>,
}

impl ServiceTerminationError {
    #[must_use]
    pub fn failures(&self) -> &[ServiceTerminationFailure] {
        &self.failures
    }
}

impl fmt::Display for ServiceTerminationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("service termination had failures")?;
        for failure in &self.failures {
            write!(formatter, "; {failure}")?;
        }
        Ok(())
    }
}

impl StdError for ServiceTerminationError {}

/// Collects every terminal result before a generated service exits.
///
/// Role-owned stop methods keep their concrete error types. This report only
/// erases them after the operation has completed so generated entrypoints do
/// not need a generic error tree or caller-side boxing.
#[derive(Debug)]
pub struct ServiceTermination {
    failures: Vec<ServiceTerminationFailure>,
}

impl ServiceTermination {
    #[must_use]
    pub fn new(lifetime: Result<(), ServiceLifetimeError>) -> Self {
        let mut termination = Self {
            failures: Vec::new(),
        };
        termination.record("service lifetime", lifetime);
        termination
    }

    pub fn record<E>(&mut self, operation: &'static str, result: Result<(), E>)
    where
        E: StdError + Send + Sync + 'static,
    {
        if let Err(source) = result {
            self.failures.push(ServiceTerminationFailure {
                operation,
                source: Box::new(source),
            });
        }
    }

    pub fn wait_and_request_shutdown<E, F>(
        lifetime: ServiceLifetime,
        operation: &'static str,
        stop: F,
    ) -> ServiceTerminationWait
    where
        E: StdError + Send + Sync + 'static,
        F: FnOnce() -> Result<(), E> + Send + 'static,
    {
        ServiceTerminationWait {
            thread: thread::spawn(move || {
                let mut termination = Self::new(lifetime.wait());
                termination.record(operation, stop());
                termination
            }),
        }
    }

    /// Returns all recorded terminal failures as one concrete report.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceTerminationError`] when any recorded operation failed.
    pub fn finish(self) -> Result<(), ServiceTerminationError> {
        if self.failures.is_empty() {
            Ok(())
        } else {
            Err(ServiceTerminationError {
                failures: self.failures,
            })
        }
    }
}

#[derive(Debug, Error)]
#[error("service termination wait thread panicked")]
struct ServiceTerminationWaitPanicked;

/// Joins a service lifetime wait with the role-owned stop request it triggers.
pub struct ServiceTerminationWait {
    thread: thread::JoinHandle<ServiceTermination>,
}

impl ServiceTerminationWait {
    #[must_use]
    pub fn join(self) -> ServiceTermination {
        self.thread.join().unwrap_or_else(|_| {
            let mut termination = ServiceTermination {
                failures: Vec::new(),
            };
            termination.record(
                "service termination wait",
                Err(ServiceTerminationWaitPanicked),
            );
            termination
        })
    }
}

/// A service-owned terminal lifetime edge. It is not a process-global signal:
/// generated entrypoints wait on it after publishing their ready record.
pub struct ServiceLifetime {
    shutdown: std::sync::mpsc::Receiver<LifecycleEvent>,
}

enum LifecycleEvent {
    ShutdownRequested,
    ControlFailed(String),
}

impl ServiceLifetime {
    /// Block until an authenticated lifecycle RPC requests shutdown.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceLifetimeError::ControlFailed`] if the control listener
    /// reported a failure instead of a shutdown request, or
    /// [`ServiceLifetimeError::ControlLost`] if the control channel closed
    /// without either.
    pub fn wait(self) -> Result<(), ServiceLifetimeError> {
        match self.shutdown.recv() {
            Ok(LifecycleEvent::ShutdownRequested) => Ok(()),
            Ok(LifecycleEvent::ControlFailed(reason)) => {
                Err(ServiceLifetimeError::ControlFailed(reason))
            }
            Err(_) => Err(ServiceLifetimeError::ControlLost),
        }
    }
}

/// Listener owner. The entrypoint drops it only after its role listener/worker
/// has stopped and joined, so a late control call cannot outlive composition.
pub struct ServiceLifecycleControlServer {
    endpoint: Endpoint,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ServiceLifecycleControlServer {
    #[must_use]
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Signal the control listener and join its thread.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceLifecycleControlError::ListenerThreadPanicked`] if the
    /// listener thread panicked.
    pub fn stop(mut self) -> Result<(), ServiceLifecycleControlError> {
        self.shutdown();
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| ServiceLifecycleControlError::ListenerThreadPanicked)?;
        }
        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Drop for ServiceLifecycleControlServer {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start the entrypoint-owned lifecycle RPC and return its ready endpoint and
/// service-local lifetime token.
///
/// The current generated service matrix uses local TCP for this extra control
/// endpoint; unsupported transports refuse admission rather than silently
/// publishing an unactionable ready record.
///
/// # Errors
///
/// Returns [`ServiceLifecycleControlError::Io`] if the grant file cannot be
/// read or the listener cannot bind,
/// [`ServiceLifecycleControlError::Decode`] if the grant bytes are not a
/// capability grant set,
/// [`ServiceLifecycleControlError::InvalidCapabilityGrantSet`] if those grants
/// are not exactly the lifecycle requirements for this scope,
/// [`ServiceLifecycleControlError::UnsupportedEndpoint`] for a non-TCP
/// lifecycle endpoint, [`ServiceLifecycleControlError::InvalidTcpEndpoint`] or
/// [`ServiceLifecycleControlError::NonLoopbackTcpEndpoint`] if the address is
/// unparsable or not loopback, and
/// [`ServiceLifecycleControlError::StartupChannelClosed`] if the listener
/// thread dies before reporting readiness.
pub fn start_service_lifecycle_control(
    args: &ServiceArgs,
) -> Result<(ServiceLifecycleControlServer, ServiceLifetime), ServiceLifecycleControlError> {
    let grants = decode_capability_grant_set(&std::fs::read(&args.lifecycle_capability_grants)?)?;
    let scope = match &args.scope {
        ServiceScope::Project => None,
        ServiceScope::Session { id, .. } => Some(*id),
    };
    validate_lifecycle_capability_grants(scope, &grants)?;
    start_with_grants(&args.lifecycle_endpoint, scope, grants)
}

fn validate_lifecycle_capability_grants(
    session: Option<uuid::Uuid>,
    grants: &CapabilityGrantSet,
) -> Result<(), CapabilityGrantSetValidationError> {
    session.map_or_else(
        || {
            grants.validate_exact_brokered_for_project(
                PROJECT_SERVICE_LIFECYCLE_CAPABILITY_REQUIREMENTS,
            )
        },
        |session| {
            grants.validate_exact_brokered_for_session(
                session,
                SESSION_SERVICE_LIFECYCLE_CAPABILITY_REQUIREMENTS,
            )
        },
    )
}

fn start_with_grants(
    endpoint: &Endpoint,
    session: Option<uuid::Uuid>,
    grants: CapabilityGrantSet,
) -> Result<(ServiceLifecycleControlServer, ServiceLifetime), ServiceLifecycleControlError> {
    if endpoint.kind != EndpointKind::Tcp {
        return Err(ServiceLifecycleControlError::UnsupportedEndpoint(
            endpoint.kind,
        ));
    }
    let address = SocketAddr::from_str(&endpoint.address)
        .map_err(|_| ServiceLifecycleControlError::InvalidTcpEndpoint(endpoint.address.clone()))?;
    if !address.ip().is_loopback() {
        return Err(ServiceLifecycleControlError::NonLoopbackTcpEndpoint(
            address,
        ));
    }
    let listener = std::net::TcpListener::bind(address)?;
    listener.set_nonblocking(true)?;
    let endpoint = Endpoint::new(EndpointKind::Tcp, listener.local_addr()?.to_string());
    let (request_tx, request_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = oneshot::channel();
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
    let thread_endpoint = endpoint.clone();
    let failure_tx = request_tx.clone();
    let thread = thread::spawn(move || {
        let result = match Builder::new_current_thread().enable_io().build() {
            Ok(runtime) => {
                let local = LocalSet::new();
                runtime.block_on(local.run_until(async move {
                    let listener = TcpListener::from_std(listener)?;
                    let bootstrap = ServiceLifecycleControlRpc {
                        session,
                        grants,
                        shutdown: request_tx,
                    }
                    .into_client();
                    started_tx
                        .send(())
                        .map_err(|_| ServiceLifecycleControlError::StartupChannelClosed)?;
                    run_tcp_listener(thread_endpoint, listener, stop_rx, bootstrap).await
                }))
            }
            Err(error) => Err(error.into()),
        };
        if let Err(error) = result {
            tracing::error!(error = %error, "service lifecycle control listener failed");
            let _ = failure_tx.send(LifecycleEvent::ControlFailed(error.to_string()));
        }
    });
    started_rx
        .recv()
        .map_err(|_| ServiceLifecycleControlError::StartupChannelClosed)?;
    Ok((
        ServiceLifecycleControlServer {
            endpoint,
            stop: Some(stop_tx),
            thread: Some(thread),
        },
        ServiceLifetime {
            shutdown: request_rx,
        },
    ))
}

// Holds a capnp-rpc client across awaits; capnp-rpc keeps its connection
// state behind `Rc<RefCell<..>>`, so this future can never be `Send`.
#[allow(clippy::future_not_send)]
async fn run_tcp_listener(
    _endpoint: Endpoint,
    listener: TcpListener,
    mut stop: oneshot::Receiver<()>,
    bootstrap: core_capnp::service_lifecycle_control::Client,
) -> Result<(), ServiceLifecycleControlError> {
    loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut stop).await {
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

struct ServiceLifecycleControlRpc {
    session: Option<uuid::Uuid>,
    grants: CapabilityGrantSet,
    shutdown: std::sync::mpsc::Sender<LifecycleEvent>,
}

impl ServiceLifecycleControlRpc {
    fn into_client(self) -> core_capnp::service_lifecycle_control::Client {
        capnp_rpc::new_client(self)
    }

    fn authorize(&self, capability: &Capability) -> Result<(), capnp::Error> {
        capability
            .validate_lifetime()
            .map_err(|error| capnp::Error::failed(error.to_string()))?;
        if capability.audience != SERVICE_LIFECYCLE_AUDIENCE
            || !capability.has_permissions(&[SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION])
            || capability.session != self.session
        {
            return Err(capnp::Error::failed(
                "lifecycle capability is not authorized".to_string(),
            ));
        }
        self.grants
            .validate(capability, SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION)
            .map_err(|error| capnp::Error::failed(error.to_string()))
    }
}

impl core_capnp::service_lifecycle_control::Server for ServiceLifecycleControlRpc {
    // capnp-rpc server methods take `capnp::capability::Rc<Self>`, which is
    // not `Send`; this future can never be `Send` without replacing the RPC stack.
    #[allow(clippy::future_not_send)]
    async fn shutdown(
        self: capnp::capability::Rc<Self>,
        params: core_capnp::service_lifecycle_control::ShutdownParams,
        _results: core_capnp::service_lifecycle_control::ShutdownResults,
    ) -> Result<(), capnp::Error> {
        let capability = Capability::from_capnp(params.get()?.get_capability()?)?;
        self.authorize(&capability)?;
        let _ = self.shutdown.send(LifecycleEvent::ShutdownRequested);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_proto_core::{ServiceId, ServiceRole};

    #[test]
    fn lifecycle_grant_document_rejects_role_interface_authority() {
        let mut grants = az_service_catalog::asset_processor_service_descriptor(
            uuid::Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .capabilities;
        grants.push(
            Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
                .with_audience("asset-processor")
                .with_permissions(["asset.read"])
                .with_token_hash([8]),
        );

        let error =
            validate_lifecycle_capability_grants(None, &CapabilityGrantSet::from_grants(grants))
                .unwrap_err();

        assert!(matches!(
            error,
            CapabilityGrantSetValidationError::UnexpectedGrant(ref grant)
                if grant.audience == "asset-processor"
        ));
    }

    #[test]
    fn lifecycle_control_rejects_a_capability_from_the_wrong_controller() {
        let granted = Capability::new(ServiceId::new("azoth", "azd"), ServiceRole::Daemon)
            .with_audience(SERVICE_LIFECYCLE_AUDIENCE)
            .with_permissions([SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION])
            .with_token_hash([7]);
        let wrong = Capability::new(
            ServiceId::new("azoth", "session-supervisor"),
            ServiceRole::SessionSupervisor,
        )
        .with_audience(SERVICE_LIFECYCLE_AUDIENCE)
        .with_permissions([SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION])
        .with_token_hash([7]);
        let (shutdown, _lifetime) = std::sync::mpsc::channel();
        let control = ServiceLifecycleControlRpc {
            session: None,
            grants: CapabilityGrantSet::from_grants(vec![granted]),
            shutdown,
        };

        assert!(control.authorize(&wrong).is_err());
    }

    #[test]
    fn service_termination_preserves_every_terminal_failure() {
        let (events, shutdown) = std::sync::mpsc::channel();
        drop(events);
        let mut termination = ServiceTermination::wait_and_request_shutdown(
            ServiceLifetime { shutdown },
            "role service",
            || Err(std::io::Error::other("composition cleanup refused")),
        )
        .join();
        termination.record(
            "lifecycle control server",
            Err(std::io::Error::other("listener thread panicked")),
        );

        let error = termination.finish().unwrap_err();

        assert_eq!(error.failures().len(), 3);
        assert_eq!(error.failures()[0].operation(), "service lifetime");
        assert_eq!(error.failures()[1].operation(), "role service");
        assert_eq!(error.failures()[2].operation(), "lifecycle control server");
        assert!(error.to_string().contains("control endpoint closed"));
        assert!(error.to_string().contains("composition cleanup refused"));
        assert!(error.to_string().contains("listener thread panicked"));
    }

    #[test]
    fn lifecycle_listener_refuses_non_loopback_tcp_admission() {
        let Err(error) = start_with_grants(
            &Endpoint::new(EndpointKind::Tcp, "192.0.2.1:0"),
            None,
            CapabilityGrantSet::default(),
        ) else {
            panic!("non-loopback lifecycle endpoint was admitted");
        };

        assert!(matches!(
            error,
            ServiceLifecycleControlError::NonLoopbackTcpEndpoint(address)
                if address.to_string() == "192.0.2.1:0"
        ));
    }
}
