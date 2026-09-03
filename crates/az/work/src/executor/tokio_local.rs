//! Tokio current-thread `LocalSet` executor adapter.
//!
//! Routes both `spawn` and `spawn_local` to [`tokio::task::spawn_local`], so it
//! must run inside a `LocalSet`'s context (e.g. `LocalSet::block_on`). This is
//! the editor's executor: the open flow runs on a current-thread runtime plus a
//! `LocalSet`, and `spawn_local` keeps non-`Send` editor futures local.

use crate::executor::{AsyncExecutor, BoxFuture, LocalBoxFuture, SpawnError, SpawnedTask};

/// Spawns futures onto the ambient tokio `LocalSet`.
///
/// Construct inside the `LocalSet` that drives the work. Both `Send` and
/// `!Send` futures route through `spawn_local`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokioLocalExecutor {
    _private: (),
}

impl TokioLocalExecutor {
    /// Adapter bound to the current `LocalSet` context.
    ///
    /// The returned executor must only be used from the thread running the
    /// `LocalSet`; spawning from elsewhere panics inside tokio.
    #[must_use]
    pub const fn current() -> Self {
        Self { _private: () }
    }
}

impl AsyncExecutor for TokioLocalExecutor {
    fn spawn(&self, future: BoxFuture<'static, ()>) -> Result<SpawnedTask, SpawnError> {
        // A Send future is trivially also spawnable locally.
        let jh = tokio::task::spawn_local(future);
        Ok(crate::executor::tokio::spawned_from_tokio(jh))
    }

    fn spawn_local(&self, future: LocalBoxFuture<'static, ()>) -> Result<SpawnedTask, SpawnError> {
        let jh = tokio::task::spawn_local(future);
        Ok(crate::executor::tokio::spawned_from_tokio(jh))
    }
}
