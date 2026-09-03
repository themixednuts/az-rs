//! Runtime-agnostic async executor injection.
//!
//! az-work is a thin structured-concurrency/policy layer, NOT a runtime owner.
//! The core async surface ([`Concurrent`](crate::Concurrent) fan-out) runs on
//! whatever drives the returned future and needs no executor. Only
//! [`Concurrent::spawn`](crate::Concurrent::spawn) detaches a task, and for that
//! the host injects an [`AsyncExecutor`] (tokio in the editor/daemon/cli, bevy
//! tasks in the engine).
//!
//! Concrete adapters live in the `executor::` submodules behind features so a
//! shared crate accreting [`JobContext`](crate::JobContext) does not grow cfgs:
//! only the adapter structs are gated, never the trait or the core surface.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

#[cfg(feature = "tokio")]
pub mod tokio;
#[cfg(feature = "tokio")]
pub use self::tokio::TokioExecutor;

#[cfg(feature = "tokio-local")]
pub mod tokio_local;
#[cfg(feature = "tokio-local")]
pub use self::tokio_local::TokioLocalExecutor;

#[cfg(feature = "bevy")]
pub mod bevy;
#[cfg(feature = "bevy")]
pub use self::bevy::BevyExecutor;

/// A `Send` future boxed for spawning.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// A `!Send` future boxed for local spawning.
pub type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Why a spawn could not be performed.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnError {
    /// No executor was injected; the context uses the default no-op executor.
    NoExecutor,
    /// The backend cannot spawn a `!Send`/local future (e.g. a multi-thread
    /// pool was asked for `spawn_local`).
    LocalUnsupported,
    /// The executor refused because its runtime is shutting down.
    Shutdown,
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoExecutor => f.write_str("no async executor injected into the job context"),
            Self::LocalUnsupported => {
                f.write_str("the injected executor cannot spawn local (!Send) futures")
            }
            Self::Shutdown => f.write_str("the injected executor is shutting down"),
        }
    }
}

impl std::error::Error for SpawnError {}

/// Error returned when joining a spawned task that panicked or was aborted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinError {
    aborted: bool,
}

impl JoinError {
    /// A task that was aborted before completing.
    #[must_use]
    pub const fn aborted() -> Self {
        Self { aborted: true }
    }

    /// A task that ended abnormally (panic) rather than abort.
    #[must_use]
    pub const fn panicked() -> Self {
        Self { aborted: false }
    }

    /// Whether the failure was due to an explicit abort.
    #[must_use]
    pub const fn is_aborted(self) -> bool {
        self.aborted
    }
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.aborted {
            f.write_str("spawned task was aborted")
        } else {
            f.write_str("spawned task panicked")
        }
    }
}

impl std::error::Error for JoinError {}

/// An abortable handle to a detached task owned by the host runtime.
///
/// The adapter wraps its native join handle behind these closures so the rest
/// of az-work stays runtime-agnostic. `join` resolves the task's completion;
/// `abort` requests early termination.
pub struct SpawnedTask {
    abort: Box<dyn Fn() + Send + Sync>,
    join: BoxFuture<'static, Result<(), JoinError>>,
}

impl fmt::Debug for SpawnedTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpawnedTask").finish_non_exhaustive()
    }
}

impl SpawnedTask {
    /// Construct a spawned-task handle from its abort and join primitives.
    #[must_use]
    pub fn new(
        abort: impl Fn() + Send + Sync + 'static,
        join: BoxFuture<'static, Result<(), JoinError>>,
    ) -> Self {
        Self {
            abort: Box::new(abort),
            join,
        }
    }

    /// Request abort of the underlying task.
    pub fn abort(&self) {
        (self.abort)();
    }

    /// Await completion of the underlying task.
    ///
    /// # Errors
    ///
    /// Returns [`JoinError`] if the task was aborted or ended abnormally.
    pub async fn join(self) -> Result<(), JoinError> {
        self.join.await
    }
}

/// Host-injected runtime adapter that detaches tasks.
///
/// Implementors bridge az-work to a concrete runtime (tokio, `bevy_tasks`, ...).
/// The trait is intentionally minimal: spawn a boxed future, optionally a local
/// one. Everything else in the async surface is runtime-free.
pub trait AsyncExecutor: Send + Sync + 'static {
    /// Spawn a `Send` future onto the host runtime.
    ///
    /// # Errors
    ///
    /// Returns [`SpawnError::Shutdown`] if the runtime refuses the task.
    fn spawn(&self, future: BoxFuture<'static, ()>) -> Result<SpawnedTask, SpawnError>;

    /// Spawn a `!Send`/local future. Defaults to [`SpawnError::LocalUnsupported`].
    ///
    /// # Errors
    ///
    /// Returns [`SpawnError::LocalUnsupported`] unless the backend overrides
    /// this to support local spawning (e.g. a tokio `LocalSet`).
    fn spawn_local(&self, _future: LocalBoxFuture<'static, ()>) -> Result<SpawnedTask, SpawnError> {
        Err(SpawnError::LocalUnsupported)
    }
}

/// The default executor: refuses every spawn with [`SpawnError::NoExecutor`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopExecutor;

impl AsyncExecutor for NoopExecutor {
    fn spawn(&self, _future: BoxFuture<'static, ()>) -> Result<SpawnedTask, SpawnError> {
        Err(SpawnError::NoExecutor)
    }
}
