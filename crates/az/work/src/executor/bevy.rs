//! Bevy task-pool executor adapter.
//!
//! Routes `Send` futures onto a [`bevy_tasks::TaskPool`]. Defaults to the
//! global [`IoTaskPool`](bevy_tasks::IoTaskPool) which is the right home for the
//! IO/network-bound work az-work fans out (catalog loads, readiness waits);
//! CPU-heavy decode/index work belongs on the [`CpuRunner`](crate::CpuRunner),
//! not here.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy_tasks::TaskPool;

use crate::executor::{AsyncExecutor, BoxFuture, JoinError, SpawnError, SpawnedTask};

/// Spawns `Send` futures onto a bevy [`TaskPool`].
#[derive(Clone)]
pub struct BevyExecutor {
    pool: Arc<TaskPool>,
}

impl std::fmt::Debug for BevyExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BevyExecutor").finish_non_exhaustive()
    }
}

impl BevyExecutor {
    /// Adapter over an explicit task pool.
    #[must_use]
    pub const fn new(pool: Arc<TaskPool>) -> Self {
        Self { pool }
    }

    /// Adapter over the global [`IoTaskPool`](bevy_tasks::IoTaskPool).
    ///
    /// # Panics
    ///
    /// Panics if the global IO task pool has not been initialized.
    #[must_use]
    pub fn io() -> Self {
        // `IoTaskPool` derefs to the shared `TaskPool`.
        Self {
            pool: Arc::new((*bevy_tasks::IoTaskPool::get()).clone()),
        }
    }

    /// Adapter over the global [`ComputeTaskPool`](bevy_tasks::ComputeTaskPool).
    ///
    /// # Panics
    ///
    /// Panics if the global compute task pool has not been initialized.
    #[must_use]
    pub fn compute() -> Self {
        // `ComputeTaskPool` derefs to the shared `TaskPool`.
        Self {
            pool: Arc::new((*bevy_tasks::ComputeTaskPool::get()).clone()),
        }
    }
}

/// Cooperative single-poll yield: returns `Pending` once then `Ready`, waking
/// itself so the join poll-loop re-checks the completion flag without spinning.
async fn yield_once() {
    struct YieldOnce(bool);
    impl std::future::Future for YieldOnce {
        type Output = ();
        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<()> {
            if self.0 {
                std::task::Poll::Ready(())
            } else {
                self.0 = true;
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        }
    }
    YieldOnce(false).await;
}

impl AsyncExecutor for BevyExecutor {
    fn spawn(&self, future: BoxFuture<'static, ()>) -> Result<SpawnedTask, SpawnError> {
        let aborted = Arc::new(AtomicBool::new(false));
        let aborted_for_task = Arc::clone(&aborted);
        // bevy_tasks::Task cancels on drop. Detach so it runs to completion and
        // surface its completion through a flag the join future polls.
        let done = Arc::new(AtomicBool::new(false));
        let done_for_task = Arc::clone(&done);
        let task = self.pool.spawn(async move {
            future.await;
            done_for_task.store(true, Ordering::Release);
        });
        task.detach();

        let abort_flag = Arc::clone(&aborted);
        Ok(SpawnedTask::new(
            move || abort_flag.store(true, Ordering::Release),
            Box::pin(async move {
                loop {
                    if aborted_for_task.load(Ordering::Acquire) {
                        return Err(JoinError::aborted());
                    }
                    if done.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    yield_once().await;
                }
            }),
        ))
    }
}
