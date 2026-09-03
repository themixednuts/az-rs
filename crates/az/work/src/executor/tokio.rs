//! Tokio multi-thread executor adapter.

use std::sync::Arc;

use tokio::runtime::Handle;
use tokio::task::AbortHandle;

use crate::executor::{AsyncExecutor, BoxFuture, JoinError, SpawnError, SpawnedTask};

/// Spawns `Send` futures onto a tokio runtime via a [`Handle`].
///
/// Suitable for the daemon (multi-thread runtime). `spawn_local` is
/// unsupported; use [`TokioLocalExecutor`](crate::executor::TokioLocalExecutor)
/// inside a `LocalSet` when local spawning is required.
#[derive(Debug, Clone)]
pub struct TokioExecutor {
    handle: Handle,
}

impl TokioExecutor {
    /// Adapter for the current tokio runtime.
    ///
    /// # Panics
    ///
    /// Panics if called outside a tokio runtime context.
    #[must_use]
    pub fn current() -> Self {
        Self {
            handle: Handle::current(),
        }
    }

    /// Adapter for an explicit tokio runtime handle.
    #[must_use]
    pub const fn new(handle: Handle) -> Self {
        Self { handle }
    }
}

impl AsyncExecutor for TokioExecutor {
    fn spawn(&self, future: BoxFuture<'static, ()>) -> Result<SpawnedTask, SpawnError> {
        let jh = self.handle.spawn(future);
        Ok(spawned_from_tokio(jh))
    }
}

/// Build a [`SpawnedTask`] from a tokio [`JoinHandle`](tokio::task::JoinHandle).
pub(super) fn spawned_from_tokio(jh: tokio::task::JoinHandle<()>) -> SpawnedTask {
    let abort: Arc<AbortHandle> = Arc::new(jh.abort_handle());
    let abort_for_closure = Arc::clone(&abort);
    SpawnedTask::new(
        move || abort_for_closure.abort(),
        Box::pin(async move {
            match jh.await {
                Ok(()) => Ok(()),
                Err(err) if err.is_cancelled() => Err(JoinError::aborted()),
                Err(_) => Err(JoinError::panicked()),
            }
        }),
    )
}
