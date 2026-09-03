//! Cooperative cancellation shared by local work items.
//!
//! [`CancellationToken`] exposes hierarchical
//! [`child_token`](CancellationToken::child_token), an async
//! [`cancelled`](CancellationToken::cancelled) future, and the blocking event
//! edge consumed through the `az-jobs` cancellation contract.
//!
//! The token composes [`tokio_util::sync::CancellationToken`] for async wakeup
//! with `az-jobs`' disconnect edge for blocking channel selection. Both edges
//! transition together, including parent-to-child propagation and OS signals.

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
struct CancellationState {
    asynchronous: tokio_util::sync::CancellationToken,
    blocking: az_jobs::CancellationToken,
    signal_bridge_started: AtomicBool,
}

impl Default for CancellationState {
    fn default() -> Self {
        Self {
            asynchronous: tokio_util::sync::CancellationToken::new(),
            blocking: az_jobs::CancellationToken::new(),
            signal_bridge_started: AtomicBool::new(false),
        }
    }
}

impl CancellationState {
    fn cancel(&self) {
        self.asynchronous.cancel();
        self.blocking.cancel();
    }
}

/// Cooperative cancellation token shared by local work items.
///
/// Cheap to clone (`Arc`-backed). Cancellation is monotonic: once requested it
/// stays requested for the lifetime of the token tree.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

impl CancellationToken {
    /// Construct a cancellation token in the running state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation for future cooperative work checks.
    ///
    /// Both the async and blocking event edges transition before this returns.
    pub fn cancel(&self) {
        self.state.cancel();
    }

    /// Whether cancellation has been requested.
    ///
    /// Returns `true` if this token or an ancestor was cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.asynchronous.is_cancelled()
    }

    /// Create a child token that is cancelled when this token is cancelled,
    /// but whose own cancellation does not propagate back to the parent.
    ///
    #[must_use]
    pub fn child_token(&self) -> Self {
        Self {
            state: Arc::new(CancellationState {
                asynchronous: self.state.asynchronous.child_token(),
                blocking: self.state.blocking.child_token(),
                signal_bridge_started: AtomicBool::new(false),
            }),
        }
    }

    /// Future that completes when this token is cancelled via
    /// [`cancel`](Self::cancel) or ancestor propagation.
    ///
    /// Async waiters and blocking selectors observe the same monotonic
    /// transition, including cancellation initiated by an OS signal.
    pub fn cancelled(&self) -> impl Future<Output = ()> + Send + '_ {
        self.state.asynchronous.cancelled()
    }

    /// Return the blocking event edge driven by the same cancellation state.
    #[must_use]
    pub fn cancellation_signal(&self) -> az_jobs::CancellationSignal {
        self.state.blocking.cancellation_signal()
    }
}

impl az_jobs::Cancellation for CancellationToken {
    fn cancel(&self) {
        Self::cancel(self);
    }

    fn is_cancelled(&self) -> bool {
        Self::is_cancelled(self)
    }

    fn cancellation_signal(&self) -> az_jobs::CancellationSignal {
        Self::cancellation_signal(self)
    }
}

/// Register SIGINT/SIGTERM handlers that cancel `cancel`.
///
/// # Errors
///
/// Returns an error if the platform refuses a signal registration.
pub fn install_signal_handlers(cancel: &CancellationToken) -> Result<(), SignalInstallError> {
    if cancel
        .state
        .signal_bridge_started
        .swap(true, Ordering::AcqRel)
    {
        return Ok(());
    }
    if let Err(error) = az_jobs::install_signal_handlers(&cancel.state.blocking) {
        cancel
            .state
            .signal_bridge_started
            .store(false, Ordering::Release);
        return Err(SignalInstallError::Register(error));
    }
    let signal = cancel.state.blocking.cancellation_signal();
    let state = Arc::downgrade(&cancel.state);
    if let Err(source) = std::thread::Builder::new()
        .name("az-work-cancellation-signal".to_string())
        .spawn(move || {
            signal.wait();
            if let Some(state) = state.upgrade() {
                state.cancel();
            }
        })
    {
        cancel
            .state
            .signal_bridge_started
            .store(false, Ordering::Release);
        return Err(SignalInstallError::ThreadSpawn(source));
    }
    Ok(())
}

/// Construct a cancellation token and best-effort CLI signal handlers.
#[must_use]
pub fn cancellation_token_with_signal_handlers() -> CancellationToken {
    let cancel = CancellationToken::new();
    if let Err(error) = install_signal_handlers(&cancel) {
        tracing::debug!("failed to register cancellation signals: {error}");
    }
    cancel
}

/// Error returned when a cancellation signal handler cannot be registered.
#[derive(Debug)]
pub enum SignalInstallError {
    Register(az_jobs::SignalInstallError),
    ThreadSpawn(std::io::Error),
}

impl fmt::Display for SignalInstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Register(source) => {
                write!(f, "failed to register cancellation signals: {source}")
            }
            Self::ThreadSpawn(source) => {
                write!(f, "failed to spawn cancellation signal listener: {source}")
            }
        }
    }
}

impl Error for SignalInstallError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Register(source) => Some(source),
            Self::ThreadSpawn(source) => Some(source),
        }
    }
}
