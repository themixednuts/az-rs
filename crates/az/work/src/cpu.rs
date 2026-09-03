//! Engine-facing wrapper around the shared `az-jobs` CPU runner.
//!
//! `az-jobs` owns the generic Rayon implementation. `az-work` adapts that
//! source primitive to Azoth's richer [`CancellationToken`](crate::CancellationToken)
//! and exposes the same engine API surface as before.

use crate::cancel::CancellationToken;

/// Executes homogeneous local work with a fixed worker policy.
#[derive(Debug, Clone, Default)]
pub struct CpuRunner {
    inner: az_jobs::JobRunner,
}

/// Result of a job batch that may have stopped early.
pub type JobBatch<R> = az_jobs::JobBatch<R>;

/// User-facing worker policy for reporting.
pub type JobRunnerPolicy = az_jobs::JobRunnerPolicy;

/// Error returned when a local work runner cannot be built.
pub type JobRunnerBuildError = az_jobs::JobRunnerBuildError;

impl CpuRunner {
    /// Construct a runner using Rayon's default global worker pool.
    #[must_use]
    pub const fn automatic() -> Self {
        Self {
            inner: az_jobs::JobRunner::automatic(),
        }
    }

    /// Construct a runner that runs work on the caller thread.
    #[must_use]
    pub const fn inline() -> Self {
        Self {
            inner: az_jobs::JobRunner::inline(),
        }
    }

    /// Construct a runner from an optional job count.
    ///
    /// `None` uses Rayon's automatic worker count. `Some(0)` runs work
    /// on the caller thread. `Some(n)` creates a local pool with `n`
    /// workers.
    ///
    /// # Errors
    ///
    /// Returns an error if Rayon rejects the requested worker count or
    /// cannot spawn the worker pool.
    pub fn from_jobs(jobs: Option<usize>) -> Result<Self, JobRunnerBuildError> {
        az_jobs::JobRunner::from_jobs(jobs).map(|inner| Self { inner })
    }

    /// Construct a runner with an explicit worker count.
    ///
    /// Passing `0` runs work on the caller thread.
    ///
    /// # Errors
    ///
    /// Returns an error if Rayon rejects the requested worker count or
    /// cannot spawn the worker pool.
    pub fn with_workers(workers: usize) -> Result<Self, JobRunnerBuildError> {
        az_jobs::JobRunner::with_workers(workers).map(|inner| Self { inner })
    }

    /// Whether this runner executes work on the caller thread.
    #[must_use]
    pub const fn is_inline(&self) -> bool {
        self.inner.is_inline()
    }

    /// Worker policy used by this runner.
    #[must_use]
    pub fn policy(&self) -> JobRunnerPolicy {
        self.inner.policy()
    }

    /// Number of worker lanes available to this runner.
    #[must_use]
    pub fn parallelism(&self) -> usize {
        self.inner.parallelism()
    }

    /// Run a closure inside this runner's execution policy.
    pub fn install<R, F>(&self, f: F) -> R
    where
        R: Send,
        F: FnOnce() -> R + Send,
    {
        self.inner.install(f)
    }

    /// Run two independent closures with this runner's execution policy.
    pub fn join<A, B, FA, FB>(&self, left: FA, right: FB) -> (A, B)
    where
        A: Send,
        B: Send,
        FA: FnOnce() -> A + Send,
        FB: FnOnce() -> B + Send,
    {
        self.inner.join(left, right)
    }

    /// Map work items and collect results.
    pub fn map<T, R, F>(&self, items: &[T], f: F) -> Vec<R>
    where
        T: Sync,
        R: Send,
        F: Fn(&T) -> R + Send + Sync,
    {
        self.inner.map(items, f)
    }

    /// Fallibly map work items and collect results in input order.
    ///
    /// # Errors
    ///
    /// Returns the first error produced by `f`.
    pub fn try_map<T, R, E, F>(&self, items: &[T], f: F) -> Result<Vec<R>, E>
    where
        T: Sync,
        R: Send,
        E: Send,
        F: Fn(&T) -> Result<R, E> + Send + Sync,
    {
        self.inner.try_map(items, f)
    }

    /// Map work items, skipping items that have not started after cancellation.
    pub fn map_until_cancelled<T, R, F>(
        &self,
        items: &[T],
        cancel: &CancellationToken,
        f: F,
    ) -> JobBatch<R>
    where
        T: Sync,
        R: Send,
        F: Fn(&T) -> R + Send + Sync,
    {
        self.inner.map_until_cancelled(items, cancel, f)
    }

    /// Fallibly map work items, skipping items that have not started after cancellation.
    ///
    /// # Errors
    ///
    /// Returns the first error produced by `f`.
    pub fn try_map_until_cancelled<T, R, E, F>(
        &self,
        items: &[T],
        cancel: &CancellationToken,
        f: F,
    ) -> Result<JobBatch<R>, E>
    where
        T: Sync,
        R: Send,
        E: Send,
        F: Fn(&T) -> Result<R, E> + Send + Sync,
    {
        self.inner.try_map_until_cancelled(items, cancel, f)
    }

    /// Map work items with worker-local state.
    pub fn map_init<T, S, R, Init, F>(&self, items: &[T], init: Init, f: F) -> Vec<R>
    where
        T: Sync,
        S: Send,
        R: Send,
        Init: Fn() -> S + Send + Sync,
        F: Fn(&mut S, &T) -> R + Send + Sync,
    {
        self.inner.map_init(items, init, f)
    }

    /// Fallibly map work items with worker-local state.
    ///
    /// # Errors
    ///
    /// Returns the first error produced by `f`.
    pub fn try_map_init<T, S, R, E, Init, F>(
        &self,
        items: &[T],
        init: Init,
        f: F,
    ) -> Result<Vec<R>, E>
    where
        T: Sync,
        S: Send,
        R: Send,
        E: Send,
        Init: Fn() -> S + Send + Sync,
        F: Fn(&mut S, &T) -> Result<R, E> + Send + Sync,
    {
        self.inner.try_map_init(items, init, f)
    }

    /// Map work items with worker-local state, skipping not-yet-started items
    /// after cancellation.
    pub fn map_init_until_cancelled<T, S, R, Init, F>(
        &self,
        items: &[T],
        cancel: &CancellationToken,
        init: Init,
        f: F,
    ) -> JobBatch<R>
    where
        T: Sync,
        S: Send,
        R: Send,
        Init: Fn() -> S + Send + Sync,
        F: Fn(&mut S, &T) -> R + Send + Sync,
    {
        self.inner.map_init_until_cancelled(items, cancel, init, f)
    }

    /// Fallibly map work items with worker-local state, skipping not-yet-started
    /// items after cancellation.
    ///
    /// # Errors
    ///
    /// Returns the first error produced by `f`.
    pub fn try_map_init_until_cancelled<T, S, R, E, Init, F>(
        &self,
        items: &[T],
        cancel: &CancellationToken,
        init: Init,
        f: F,
    ) -> Result<JobBatch<R>, E>
    where
        T: Sync,
        S: Send,
        R: Send,
        E: Send,
        Init: Fn() -> S + Send + Sync,
        F: Fn(&mut S, &T) -> Result<R, E> + Send + Sync,
    {
        self.inner
            .try_map_init_until_cancelled(items, cancel, init, f)
    }

    /// Run `f` for its side effects on each item, skipping any items once
    /// `cancel` is tripped.
    pub fn for_each_until_cancelled<T, F>(
        &self,
        items: &[T],
        cancel: &CancellationToken,
        f: F,
    ) -> JobBatch<()>
    where
        T: Sync,
        F: Fn(&T) + Send + Sync,
    {
        self.inner.for_each_until_cancelled(items, cancel, f)
    }

    /// Run side-effecting work with one lazily initialized state per worker,
    /// without retaining a result for every input.
    pub fn for_each_init_until_cancelled<T, S, Init, F>(
        &self,
        items: &[T],
        cancel: &CancellationToken,
        init: Init,
        f: F,
    ) -> JobBatch<()>
    where
        T: Sync,
        S: Send,
        Init: Fn() -> S + Send + Sync,
        F: Fn(&mut S, &T) + Send + Sync,
    {
        self.inner
            .for_each_init_until_cancelled(items, cancel, init, f)
    }
}

// Shared by the async `Concurrent` fan-out (a sibling private module), so this
// must be crate-visible; the lint's "redundant" heuristic doesn't see the
// cross-module use.
#[allow(clippy::redundant_pub_crate)]
pub(crate) fn collect_job_batch<R>(mapped: Vec<Option<R>>, cancelled: bool) -> JobBatch<R> {
    let skipped = mapped.iter().filter(|item| item.is_none()).count();
    let completed = mapped.into_iter().flatten().collect();
    JobBatch::from_parts(completed, skipped, cancelled || skipped > 0)
}
