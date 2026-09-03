//! Unified job context bridging CPU data-parallel and async concurrent work.
//!
//! [`JobContext`] is the single handle every consumer threads through. It owns
//! a [`CpuRunner`] (Rayon data-parallel; `cpu()`/`runner()` alias), a
//! [`CancellationToken`] (atomic + async), a no-op-by-default [`Reporter`], and
//! an injected [`AsyncExecutor`] (no-op by default). The async surface is
//! reached through [`concurrent()`](JobContext::concurrent).

use std::num::NonZeroUsize;
use std::sync::Arc;

use crate::cancel::{CancellationToken, cancellation_token_with_signal_handlers};
use crate::concurrent::{Concurrent, TaskKind, TaskName};
use crate::cpu::{CpuRunner, JobRunnerBuildError, JobRunnerPolicy};
use crate::executor::{AsyncExecutor, NoopExecutor};
use crate::progress::Reporter;

/// Shared execution context for bounded local jobs.
///
/// Cheap to clone; clones share the runner, cancellation token, reporter, and
/// executor. Use [`child`](JobContext::child) to derive a nested context with
/// its own child cancellation token.
#[derive(Clone)]
pub struct JobContext {
    cpu: CpuRunner,
    cancel: CancellationToken,
    reporter: Reporter,
    executor: Arc<dyn AsyncExecutor>,
    name: TaskName,
}

impl std::fmt::Debug for JobContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobContext")
            .field("cpu", &self.cpu)
            .field("cancel", &self.cancel)
            .field("policy", &self.policy())
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Default for JobContext {
    fn default() -> Self {
        Self {
            cpu: CpuRunner::default(),
            cancel: CancellationToken::new(),
            reporter: Reporter::noop(),
            executor: Arc::new(NoopExecutor),
            name: TaskName::default(),
        }
    }
}

impl JobContext {
    /// Construct a job context from an explicit runner and cancellation token.
    ///
    /// Fills a no-op reporter and no-op executor; add real ones with
    /// [`with_reporter`](Self::with_reporter)/[`with_executor`](Self::with_executor).
    #[must_use]
    pub fn new(runner: CpuRunner, cancel: CancellationToken) -> Self {
        Self {
            cpu: runner,
            cancel,
            reporter: Reporter::noop(),
            executor: Arc::new(NoopExecutor),
            name: TaskName::default(),
        }
    }

    /// Construct a job context from an optional job count.
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
        Ok(Self::new(
            CpuRunner::from_jobs(jobs)?,
            CancellationToken::new(),
        ))
    }

    /// Construct a job context and install best-effort process signal
    /// handlers that request cooperative cancellation.
    ///
    /// # Errors
    ///
    /// Returns an error if Rayon rejects the requested worker count or
    /// cannot spawn the worker pool.
    pub fn from_jobs_with_signal_handlers(
        jobs: Option<usize>,
    ) -> Result<Self, JobRunnerBuildError> {
        Ok(Self::new(
            CpuRunner::from_jobs(jobs)?,
            cancellation_token_with_signal_handlers(),
        ))
    }

    /// Inject the async executor used by [`Concurrent::spawn`].
    #[must_use]
    pub fn with_executor(mut self, executor: Arc<dyn AsyncExecutor>) -> Self {
        self.executor = executor;
        self
    }

    /// Install a real progress reporter (defaults to no-op).
    #[must_use]
    pub fn with_reporter(mut self, reporter: Reporter) -> Self {
        self.reporter = reporter;
        self
    }

    /// Set the root task name for this context.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<TaskName>) -> Self {
        self.name = name.into();
        self
    }

    /// Runner used for data-parallel CPU work.
    #[must_use]
    pub const fn cpu(&self) -> &CpuRunner {
        &self.cpu
    }

    /// Compatibility alias for [`cpu`](Self::cpu).
    #[must_use]
    pub const fn runner(&self) -> &CpuRunner {
        self.cpu()
    }

    /// Cancellation token shared by this context.
    #[must_use]
    pub const fn cancel(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Clone of this context's progress reporter (no-op unless one was set).
    #[must_use]
    pub fn progress(&self) -> Reporter {
        self.reporter.clone()
    }

    /// Async surface borrowing this context.
    #[must_use]
    pub const fn concurrent(&self) -> Concurrent<'_> {
        Concurrent::new(self)
    }

    /// Injected async executor.
    #[must_use]
    pub fn executor(&self) -> &Arc<dyn AsyncExecutor> {
        &self.executor
    }

    /// Root task name.
    #[must_use]
    pub const fn name(&self) -> &TaskName {
        &self.name
    }

    /// Worker policy used by this context.
    #[must_use]
    pub fn policy(&self) -> JobRunnerPolicy {
        self.cpu.policy()
    }

    /// Bounded concurrency for async fan-out that follows this context's
    /// worker policy.
    ///
    /// Inline work uses one request at a time. Explicit worker counts cap
    /// fan-out at that count. Automatic sizing uses the host's available
    /// parallelism.
    #[must_use]
    pub fn concurrency_limit(&self, item_count: usize) -> usize {
        if item_count == 0 {
            return 0;
        }

        let requested = match self.policy() {
            JobRunnerPolicy::Inline => 1,
            JobRunnerPolicy::Automatic => {
                std::thread::available_parallelism().map_or(1, NonZeroUsize::get)
            }
            JobRunnerPolicy::Workers(workers) => workers.max(1),
        };
        requested.min(item_count)
    }

    /// Fan-out width for an async batch of `item_count` items of `kind`.
    ///
    /// Builds on [`concurrency_limit`](Self::concurrency_limit) (the policy
    /// width) and biases by task kind: network/IO work tolerates wider fan-out
    /// than the CPU worker count, while CPU work stays at the policy width.
    #[must_use]
    pub fn fanout_limit(&self, kind: TaskKind, item_count: usize) -> usize {
        let base = self.concurrency_limit(item_count);
        if base == 0 {
            return 0;
        }
        match kind {
            TaskKind::Cpu => base,
            // IO/Network/Mixed work is rarely CPU-bound; allow the host to keep
            // more requests in flight than there are CPU workers, but never run
            // more than there are items.
            TaskKind::Io | TaskKind::Network | TaskKind::Mixed => {
                base.saturating_mul(4).min(item_count.max(1))
            }
        }
    }

    /// Derive a nested context with a child cancellation token.
    ///
    /// Cancelling the parent cancels the child (and its descendants) but not
    /// vice-versa. Runner, reporter, and executor are shared.
    #[must_use]
    pub fn child(&self, name: impl Into<TaskName>) -> Self {
        Self {
            cpu: self.cpu.clone(),
            cancel: self.cancel.child_token(),
            reporter: self.reporter.clone(),
            executor: Arc::clone(&self.executor),
            name: name.into(),
        }
    }
}
