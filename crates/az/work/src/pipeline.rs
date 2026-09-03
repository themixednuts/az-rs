//! Engine-facing blocking pipeline adapter.
//!
//! `az-jobs` owns the generic bounded channel pipeline. This module adds
//! Azoth task-kind policy and child cancellation so callers can keep routing
//! work through [`JobContext`](crate::JobContext).

use crate::cancel::CancellationToken;
use crate::concurrent::TaskKind;
use crate::context::JobContext;

const DEFAULT_PIPELINE_CHANNEL_CAPACITY: usize = 1024;

pub use az_jobs::{BlockingPipelineError, BlockingPipelineStage, BlockingPipelineSummary};

/// Engine policy for a blocking producer/worker/consumer pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockingPipelineConfig {
    kind: TaskKind,
    worker_count: Option<usize>,
    max_workers: Option<usize>,
    input_capacity: usize,
    output_capacity: usize,
    thread_name_prefix: String,
}

impl Default for BlockingPipelineConfig {
    fn default() -> Self {
        Self {
            kind: TaskKind::Mixed,
            worker_count: None,
            max_workers: None,
            input_capacity: DEFAULT_PIPELINE_CHANNEL_CAPACITY,
            output_capacity: DEFAULT_PIPELINE_CHANNEL_CAPACITY,
            thread_name_prefix: "az-work-pipeline".to_string(),
        }
    }
}

impl BlockingPipelineConfig {
    /// Start a config for a resource class.
    #[must_use]
    pub fn for_kind(kind: TaskKind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }

    /// Use an exact worker count instead of deriving one from [`JobContext`].
    #[must_use]
    pub fn with_worker_count(mut self, worker_count: usize) -> Self {
        self.worker_count = Some(worker_count.max(1));
        self
    }

    /// Cap the worker count after explicit or context-derived sizing.
    #[must_use]
    pub fn with_max_workers(mut self, max_workers: usize) -> Self {
        self.max_workers = Some(max_workers.max(1));
        self
    }

    /// Set the bounded input-channel capacity.
    #[must_use]
    pub fn with_input_capacity(mut self, input_capacity: usize) -> Self {
        self.input_capacity = input_capacity.max(1);
        self
    }

    /// Set the bounded output-channel capacity.
    #[must_use]
    pub fn with_output_capacity(mut self, output_capacity: usize) -> Self {
        self.output_capacity = output_capacity.max(1);
        self
    }

    /// Set the OS thread name prefix used by the source-owned pipeline.
    #[must_use]
    pub fn with_thread_name_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.thread_name_prefix = prefix.into();
        if self.thread_name_prefix.is_empty() {
            self.thread_name_prefix = "az-work-pipeline".to_string();
        }
        self
    }

    /// Resource class used to derive default worker count.
    #[must_use]
    pub const fn kind(&self) -> TaskKind {
        self.kind
    }

    /// Explicit worker count, if set.
    #[must_use]
    pub const fn worker_count(&self) -> Option<usize> {
        self.worker_count
    }

    /// Maximum worker count, if set.
    #[must_use]
    pub const fn max_workers(&self) -> Option<usize> {
        self.max_workers
    }

    /// Bounded input-channel capacity.
    #[must_use]
    pub const fn input_capacity(&self) -> usize {
        self.input_capacity
    }

    /// Bounded output-channel capacity.
    #[must_use]
    pub const fn output_capacity(&self) -> usize {
        self.output_capacity
    }

    /// OS thread name prefix.
    #[must_use]
    pub fn thread_name_prefix(&self) -> &str {
        &self.thread_name_prefix
    }

    /// Resolve the effective worker count against the supplied job context.
    #[must_use]
    pub fn resolved_worker_count(&self, ctx: &JobContext) -> usize {
        let policy_count = ctx.fanout_limit(self.kind, usize::MAX / 16).max(1);
        let mut workers = self.worker_count.unwrap_or(policy_count).max(1);
        if let Some(max_workers) = self.max_workers {
            workers = workers.min(max_workers.max(1));
        }
        workers
    }

    fn to_az_jobs(&self, ctx: &JobContext) -> az_jobs::BlockingPipelineConfig {
        az_jobs::BlockingPipelineConfig::new(self.resolved_worker_count(ctx))
            .with_input_capacity(self.input_capacity)
            .with_output_capacity(self.output_capacity)
            .with_thread_name_prefix(self.thread_name_prefix.clone())
    }
}

/// Blocking work adapter bound to a [`JobContext`].
#[derive(Debug, Clone, Copy)]
pub struct Blocking<'a> {
    ctx: &'a JobContext,
}

impl<'a> Blocking<'a> {
    pub(crate) const fn new(ctx: &'a JobContext) -> Self {
        Self { ctx }
    }

    /// Stream an input iterator through bounded worker threads and consume
    /// outputs on the current thread.
    ///
    /// # Errors
    ///
    /// Returns the first domain, thread-spawn, or panic error observed.
    pub fn try_for_each<I, O, E, Inputs, Work, Consume>(
        self,
        config: &BlockingPipelineConfig,
        inputs: Inputs,
        work: Work,
        consume: Consume,
    ) -> Result<BlockingPipelineSummary, BlockingPipelineError<E>>
    where
        I: Send + 'static,
        O: Send + 'static,
        E: Send + 'static,
        Inputs: IntoIterator<Item = Result<I, E>> + Send + 'static,
        Inputs::IntoIter: Send,
        Work: Fn(I, &CancellationToken) -> Result<Option<O>, E> + Send + Sync + 'static,
        Consume: FnMut(O) -> Result<(), E>,
    {
        az_jobs::try_for_each_bounded_blocking(
            &config.to_az_jobs(self.ctx),
            self.ctx.cancel().child_token(),
            inputs,
            work,
            consume,
        )
    }
}

impl JobContext {
    /// Blocking producer/worker/consumer pipeline adapter.
    #[must_use]
    pub const fn blocking(&self) -> Blocking<'_> {
        Blocking::new(self)
    }
}
