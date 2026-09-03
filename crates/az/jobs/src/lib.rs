//! Local parallel work and cooperative cancellation primitives.
//!
//! `az-jobs` is Azoth's engine-owned fork of the generic job-runner
//! primitive: a Rayon-backed [`JobRunner`] data-parallel surface plus a
//! [`Cancellation`]-generic bounded blocking pipeline
//! ([`try_for_each_bounded_blocking`]). `az-work` adapts these source
//! primitives to its own richer `CancellationToken` and exposes them under
//! Azoth's engine-facing API.

use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crossbeam_channel::{Receiver, Sender, unbounded};
use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};
use thiserror::Error;

mod pipeline;

pub use pipeline::{
    BlockingPipelineConfig, BlockingPipelineError, BlockingPipelineStage, BlockingPipelineSummary,
    try_for_each_bounded_blocking,
};

#[derive(Debug, Clone, Default)]
pub struct JobRunner {
    execution: Execution,
}

#[derive(Debug)]
struct CancellationState {
    cancelled: AtomicBool,
    signal_sender: Mutex<Option<Sender<()>>>,
    signal_receiver: Receiver<()>,
    children: Mutex<Vec<Weak<Self>>>,
}

impl Default for CancellationState {
    fn default() -> Self {
        let (signal_sender, signal_receiver) = unbounded();
        Self {
            cancelled: AtomicBool::new(false),
            signal_sender: Mutex::new(Some(signal_sender)),
            signal_receiver,
            children: Mutex::new(Vec::new()),
        }
    }
}

impl CancellationState {
    fn cancel(&self) {
        if self.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        self.signal_sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let children = self
            .children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for child in children {
            child.cancel();
        }
    }
}

/// Cloneable event edge that becomes disconnected exactly once on cancellation.
///
/// Blocking primitives select on this edge alongside their ordinary work
/// channels, so cancellation has no sampling interval.
#[derive(Debug, Clone)]
pub struct CancellationSignal {
    receiver: Receiver<()>,
}

impl CancellationSignal {
    /// Borrow the disconnect edge for composition in a blocking selector.
    #[must_use]
    pub const fn receiver(&self) -> &Receiver<()> {
        &self.receiver
    }

    /// Block until cancellation disconnects the event edge.
    pub fn wait(&self) {
        let _ = self.receiver.recv();
    }
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.state.cancel();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn cancellation_signal(&self) -> CancellationSignal {
        CancellationSignal {
            receiver: self.state.signal_receiver.clone(),
        }
    }

    /// Create a token cancelled by its parent without propagating child
    /// cancellation back to the parent.
    #[must_use]
    pub fn child_token(&self) -> Self {
        let child = Self::new();
        let mut children = self
            .state
            .children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.is_cancelled() {
            child.cancel();
        } else {
            children.push(Arc::downgrade(&child.state));
        }
        child
    }
}

/// Minimal cooperative cancellation contract used by generic job primitives.
pub trait Cancellation: Clone + Send + Sync + 'static {
    fn cancel(&self);
    fn is_cancelled(&self) -> bool;
    /// Event edge disconnected by the same transition that makes
    /// [`is_cancelled`](Self::is_cancelled) true.
    fn cancellation_signal(&self) -> CancellationSignal;
}

impl Cancellation for CancellationToken {
    fn cancel(&self) {
        Self::cancel(self);
    }

    fn is_cancelled(&self) -> bool {
        Self::is_cancelled(self)
    }

    fn cancellation_signal(&self) -> CancellationSignal {
        Self::cancellation_signal(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobBatch<R> {
    completed: Vec<R>,
    skipped: usize,
    cancelled: bool,
}

impl<R> JobBatch<R> {
    const fn new(completed: Vec<R>, skipped: usize, cancelled: bool) -> Self {
        Self {
            completed,
            skipped,
            cancelled,
        }
    }

    #[must_use]
    pub const fn from_parts(completed: Vec<R>, skipped: usize, cancelled: bool) -> Self {
        Self::new(completed, skipped, cancelled)
    }

    #[must_use]
    pub fn completed(&self) -> &[R] {
        &self.completed
    }

    #[must_use]
    pub const fn skipped(&self) -> usize {
        self.skipped
    }

    #[must_use]
    pub const fn was_cancelled(&self) -> bool {
        self.cancelled
    }

    #[must_use]
    pub fn into_completed(self) -> Vec<R> {
        self.completed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobRunnerPolicy {
    Inline,
    Automatic,
    Workers(usize),
}

impl fmt::Display for JobRunnerPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inline => f.write_str("caller thread"),
            Self::Automatic => f.write_str("automatic worker pool"),
            Self::Workers(workers) => write!(f, "{workers} worker(s)"),
        }
    }
}

#[derive(Debug, Clone, Default)]
enum Execution {
    Inline,
    #[default]
    Global,
    Pool(Arc<ThreadPool>),
}

impl JobRunner {
    #[must_use]
    pub const fn automatic() -> Self {
        Self {
            execution: Execution::Global,
        }
    }

    #[must_use]
    pub const fn inline() -> Self {
        Self {
            execution: Execution::Inline,
        }
    }

    /// Build a runner from an optional worker count.
    ///
    /// `None` uses the global Rayon pool, `Some(0)` runs on the caller thread,
    /// and any other value creates a private worker pool.
    ///
    /// # Errors
    ///
    /// Returns [`JobRunnerBuildError`] if Rayon cannot create the requested
    /// private worker pool.
    pub fn from_jobs(jobs: Option<usize>) -> Result<Self, JobRunnerBuildError> {
        match jobs {
            None => Ok(Self::automatic()),
            Some(0) => Ok(Self::inline()),
            Some(workers) => Self::with_workers(workers),
        }
    }

    /// Build a runner backed by a private worker pool.
    ///
    /// `workers == 0` returns an inline runner.
    ///
    /// # Errors
    ///
    /// Returns [`JobRunnerBuildError`] if Rayon cannot create the requested
    /// private worker pool.
    pub fn with_workers(workers: usize) -> Result<Self, JobRunnerBuildError> {
        if workers == 0 {
            return Ok(Self::inline());
        }

        ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .map(|pool| Self {
                execution: Execution::Pool(Arc::new(pool)),
            })
            .map_err(JobRunnerBuildError)
    }

    #[must_use]
    pub const fn is_inline(&self) -> bool {
        matches!(self.execution, Execution::Inline)
    }

    #[must_use]
    pub fn policy(&self) -> JobRunnerPolicy {
        match &self.execution {
            Execution::Inline => JobRunnerPolicy::Inline,
            Execution::Global => JobRunnerPolicy::Automatic,
            Execution::Pool(pool) => JobRunnerPolicy::Workers(pool.current_num_threads()),
        }
    }

    /// Number of worker lanes available to this runner.
    #[must_use]
    pub fn parallelism(&self) -> usize {
        match &self.execution {
            Execution::Inline => 1,
            Execution::Global => rayon::current_num_threads().max(1),
            Execution::Pool(pool) => pool.current_num_threads().max(1),
        }
    }

    pub fn map<T, R, F>(&self, items: &[T], f: F) -> Vec<R>
    where
        T: Sync,
        R: Send,
        F: Fn(&T) -> R + Send + Sync,
    {
        match &self.execution {
            Execution::Inline => items.iter().map(f).collect(),
            Execution::Global => items.par_iter().map(f).collect(),
            Execution::Pool(pool) => pool.install(|| items.par_iter().map(f).collect()),
        }
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
        match &self.execution {
            Execution::Inline => items.iter().map(f).collect(),
            Execution::Global => items.par_iter().map(f).collect(),
            Execution::Pool(pool) => pool.install(|| items.par_iter().map(f).collect()),
        }
    }

    /// Map the integer range `0..len` without allocating an index vector.
    pub fn map_indexed<R, F>(&self, len: usize, f: F) -> Vec<R>
    where
        R: Send,
        F: Fn(usize) -> R + Send + Sync,
    {
        match &self.execution {
            Execution::Inline => (0..len).map(f).collect(),
            Execution::Global => (0..len).into_par_iter().map(f).collect(),
            Execution::Pool(pool) => pool.install(|| (0..len).into_par_iter().map(f).collect()),
        }
    }

    /// Fallible variant of [`Self::map_indexed`].
    ///
    /// # Errors
    ///
    /// Returns the first error produced by `f`.
    pub fn try_map_indexed<R, E, F>(&self, len: usize, f: F) -> Result<Vec<R>, E>
    where
        R: Send,
        E: Send,
        F: Fn(usize) -> Result<R, E> + Send + Sync,
    {
        match &self.execution {
            Execution::Inline => (0..len).map(f).collect(),
            Execution::Global => (0..len).into_par_iter().map(f).collect(),
            Execution::Pool(pool) => pool.install(|| (0..len).into_par_iter().map(f).collect()),
        }
    }

    pub fn install<R, F>(&self, f: F) -> R
    where
        R: Send,
        F: FnOnce() -> R + Send,
    {
        match &self.execution {
            Execution::Inline | Execution::Global => f(),
            Execution::Pool(pool) => pool.install(f),
        }
    }

    pub fn join<A, B, FA, FB>(&self, left: FA, right: FB) -> (A, B)
    where
        A: Send,
        B: Send,
        FA: FnOnce() -> A + Send,
        FB: FnOnce() -> B + Send,
    {
        match &self.execution {
            Execution::Inline => (left(), right()),
            Execution::Global => rayon::join(left, right),
            Execution::Pool(pool) => pool.install(|| rayon::join(left, right)),
        }
    }

    pub fn map_until_cancelled<T, R, F, C>(&self, items: &[T], cancel: &C, f: F) -> JobBatch<R>
    where
        T: Sync,
        R: Send,
        F: Fn(&T) -> R + Send + Sync,
        C: Cancellation,
    {
        let mapped: Vec<Option<R>> = match &self.execution {
            Execution::Inline => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(if cancel.is_cancelled() {
                        None
                    } else {
                        Some(f(item))
                    });
                }
                out
            }
            Execution::Global => items
                .par_iter()
                .map(|item| {
                    if cancel.is_cancelled() {
                        None
                    } else {
                        Some(f(item))
                    }
                })
                .collect(),
            Execution::Pool(pool) => pool.install(|| {
                items
                    .par_iter()
                    .map(|item| {
                        if cancel.is_cancelled() {
                            None
                        } else {
                            Some(f(item))
                        }
                    })
                    .collect()
            }),
        };
        collect_job_batch(mapped, cancel.is_cancelled())
    }

    /// Fallibly map work items, skipping items that have not started after cancellation.
    ///
    /// # Errors
    ///
    /// Returns the first error produced by `f`.
    pub fn try_map_until_cancelled<T, R, E, F, C>(
        &self,
        items: &[T],
        cancel: &C,
        f: F,
    ) -> Result<JobBatch<R>, E>
    where
        T: Sync,
        R: Send,
        E: Send,
        F: Fn(&T) -> Result<R, E> + Send + Sync,
        C: Cancellation,
    {
        let mapped: Vec<Option<R>> = match &self.execution {
            Execution::Inline => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    if cancel.is_cancelled() {
                        out.push(None);
                        continue;
                    }

                    match f(item) {
                        Ok(value) => out.push(Some(value)),
                        Err(error) => {
                            cancel.cancel();
                            return Err(error);
                        }
                    }
                }
                out
            }
            Execution::Global => items
                .par_iter()
                .map(|item| try_map_item_until_cancelled(item, cancel, &f))
                .collect::<Result<Vec<_>, _>>()?,
            Execution::Pool(pool) => pool.install(|| {
                items
                    .par_iter()
                    .map(|item| try_map_item_until_cancelled(item, cancel, &f))
                    .collect::<Result<Vec<_>, _>>()
            })?,
        };
        Ok(collect_job_batch(mapped, cancel.is_cancelled()))
    }

    pub fn map_init<T, S, R, Init, F>(&self, items: &[T], init: Init, f: F) -> Vec<R>
    where
        T: Sync,
        S: Send,
        R: Send,
        Init: Fn() -> S + Send + Sync,
        F: Fn(&mut S, &T) -> R + Send + Sync,
    {
        match &self.execution {
            Execution::Inline => {
                let mut state = init();
                items.iter().map(|item| f(&mut state, item)).collect()
            }
            Execution::Global => items.par_iter().map_init(init, f).collect(),
            Execution::Pool(pool) => pool.install(|| items.par_iter().map_init(init, f).collect()),
        }
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
        match &self.execution {
            Execution::Inline => {
                let mut state = init();
                items.iter().map(|item| f(&mut state, item)).collect()
            }
            Execution::Global => items.par_iter().map_init(init, f).collect(),
            Execution::Pool(pool) => pool.install(|| items.par_iter().map_init(init, f).collect()),
        }
    }

    pub fn map_init_until_cancelled<T, S, R, Init, F, C>(
        &self,
        items: &[T],
        cancel: &C,
        init: Init,
        f: F,
    ) -> JobBatch<R>
    where
        T: Sync,
        S: Send,
        R: Send,
        Init: Fn() -> S + Send + Sync,
        F: Fn(&mut S, &T) -> R + Send + Sync,
        C: Cancellation,
    {
        let mapped: Vec<Option<R>> = match &self.execution {
            Execution::Inline => {
                let mut state = init();
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(if cancel.is_cancelled() {
                        None
                    } else {
                        Some(f(&mut state, item))
                    });
                }
                out
            }
            Execution::Global => items
                .par_iter()
                .map_init(init, |state, item| {
                    if cancel.is_cancelled() {
                        None
                    } else {
                        Some(f(state, item))
                    }
                })
                .collect(),
            Execution::Pool(pool) => pool.install(|| {
                items
                    .par_iter()
                    .map_init(init, |state, item| {
                        if cancel.is_cancelled() {
                            None
                        } else {
                            Some(f(state, item))
                        }
                    })
                    .collect()
            }),
        };
        collect_job_batch(mapped, cancel.is_cancelled())
    }

    /// Run `f` for its side effects on each item, skipping any items once
    /// `cancel` is tripped.
    ///
    /// Unlike [`Self::map_until_cancelled`], this never builds a per-item
    /// `Vec<Option<_>>`; completed and skipped counts are tracked with atomics
    /// inside the parallel closure. The returned [`JobBatch`] carries a
    /// zero-sized `Vec<()>` whose length equals the number of items actually
    /// run, so `completed().len()` stays consistent with the other methods at
    /// no heap cost.
    pub fn for_each_until_cancelled<T, F, C>(&self, items: &[T], cancel: &C, f: F) -> JobBatch<()>
    where
        T: Sync,
        F: Fn(&T) + Send + Sync,
        C: Cancellation,
    {
        use std::sync::atomic::AtomicUsize;

        let completed = AtomicUsize::new(0);
        let skipped = AtomicUsize::new(0);

        let run = |item: &T| {
            if cancel.is_cancelled() {
                skipped.fetch_add(1, Ordering::Relaxed);
            } else {
                f(item);
                completed.fetch_add(1, Ordering::Relaxed);
            }
        };

        match &self.execution {
            Execution::Inline => items.iter().for_each(run),
            Execution::Global => items.par_iter().for_each(run),
            Execution::Pool(pool) => pool.install(|| items.par_iter().for_each(run)),
        }

        let completed = completed.load(Ordering::Relaxed);
        let skipped = skipped.load(Ordering::Relaxed);
        let cancelled = cancel.is_cancelled() || skipped > 0;
        JobBatch::new(vec![(); completed], skipped, cancelled)
    }

    /// Run `f` for its side effects with one lazily initialized state per
    /// worker, skipping items once `cancel` is tripped.
    ///
    /// This is the side-effecting counterpart to
    /// [`Self::map_init_until_cancelled`]. It preserves worker-local reuse
    /// without retaining a result for every item, which is important for
    /// transforms whose intermediate outputs can be large.
    pub fn for_each_init_until_cancelled<T, S, Init, F, C>(
        &self,
        items: &[T],
        cancel: &C,
        init: Init,
        f: F,
    ) -> JobBatch<()>
    where
        T: Sync,
        S: Send,
        Init: Fn() -> S + Send + Sync,
        F: Fn(&mut S, &T) + Send + Sync,
        C: Cancellation,
    {
        use std::sync::atomic::AtomicUsize;

        let completed = AtomicUsize::new(0);
        let skipped = AtomicUsize::new(0);

        let run = |state: &mut S, item: &T| {
            if cancel.is_cancelled() {
                skipped.fetch_add(1, Ordering::Relaxed);
            } else {
                f(state, item);
                completed.fetch_add(1, Ordering::Relaxed);
            }
        };

        match &self.execution {
            Execution::Inline => {
                let mut state = init();
                for item in items {
                    run(&mut state, item);
                }
            }
            Execution::Global => items.par_iter().for_each_init(init, run),
            Execution::Pool(pool) => {
                pool.install(|| items.par_iter().for_each_init(init, run));
            }
        }

        let completed = completed.load(Ordering::Relaxed);
        let skipped = skipped.load(Ordering::Relaxed);
        let cancelled = cancel.is_cancelled() || skipped > 0;
        JobBatch::new(vec![(); completed], skipped, cancelled)
    }

    /// Fallibly map work items with worker-local state, skipping not-yet-started
    /// items after cancellation.
    ///
    /// # Errors
    ///
    /// Returns the first error produced by `f`.
    pub fn try_map_init_until_cancelled<T, S, R, E, Init, F, C>(
        &self,
        items: &[T],
        cancel: &C,
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
        C: Cancellation,
    {
        let mapped: Vec<Option<R>> = match &self.execution {
            Execution::Inline => {
                let mut state = init();
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    if cancel.is_cancelled() {
                        out.push(None);
                        continue;
                    }

                    match f(&mut state, item) {
                        Ok(value) => out.push(Some(value)),
                        Err(error) => {
                            cancel.cancel();
                            return Err(error);
                        }
                    }
                }
                out
            }
            Execution::Global => items
                .par_iter()
                .map_init(init, |state, item| {
                    try_map_init_item_until_cancelled(state, item, cancel, &f)
                })
                .collect::<Result<Vec<_>, _>>()?,
            Execution::Pool(pool) => pool.install(|| {
                items
                    .par_iter()
                    .map_init(init, |state, item| {
                        try_map_init_item_until_cancelled(state, item, cancel, &f)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })?,
        };
        Ok(collect_job_batch(mapped, cancel.is_cancelled()))
    }
}

/// Register SIGINT/SIGTERM handlers that cancel `cancel`.
///
/// # Errors
///
/// Returns [`SignalInstallError`] if the platform refuses a signal
/// registration.
pub fn install_signal_handlers(cancel: &CancellationToken) -> Result<(), SignalInstallError> {
    static SIGNAL_SUBSCRIBERS: OnceLock<Mutex<Vec<Weak<CancellationState>>>> = OnceLock::new();
    static SIGNAL_HANDLER: OnceLock<Result<(), String>> = OnceLock::new();

    SIGNAL_SUBSCRIBERS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(Arc::downgrade(&cancel.state));
    SIGNAL_HANDLER
        .get_or_init(|| {
            ctrlc::set_handler(|| {
                let Some(subscribers) = SIGNAL_SUBSCRIBERS.get() else {
                    return;
                };
                subscribers
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .retain(|subscriber| {
                        let Some(state) = subscriber.upgrade() else {
                            return false;
                        };
                        state.cancel();
                        true
                    });
            })
            .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(|message| SignalInstallError::Register(std::io::Error::other(message)))
}

#[must_use]
pub fn cancellation_token_with_signal_handlers() -> CancellationToken {
    let cancel = CancellationToken::new();
    if let Err(error) = install_signal_handlers(&cancel) {
        tracing::debug!("failed to register cancellation signals: {error}");
    }
    cancel
}

fn collect_job_batch<R>(mapped: Vec<Option<R>>, cancelled: bool) -> JobBatch<R> {
    let skipped = mapped.iter().filter(|item| item.is_none()).count();
    let completed = mapped.into_iter().flatten().collect();
    JobBatch::new(completed, skipped, cancelled || skipped > 0)
}

fn try_map_item_until_cancelled<T, R, E, F, C>(item: &T, cancel: &C, f: &F) -> Result<Option<R>, E>
where
    F: Fn(&T) -> Result<R, E>,
    C: Cancellation,
{
    if cancel.is_cancelled() {
        return Ok(None);
    }
    match f(item) {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            cancel.cancel();
            Err(error)
        }
    }
}

fn try_map_init_item_until_cancelled<T, S, R, E, F, C>(
    state: &mut S,
    item: &T,
    cancel: &C,
    f: &F,
) -> Result<Option<R>, E>
where
    F: Fn(&mut S, &T) -> Result<R, E>,
    C: Cancellation,
{
    if cancel.is_cancelled() {
        return Ok(None);
    }
    match f(state, item) {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            cancel.cancel();
            Err(error)
        }
    }
}

#[derive(Debug)]
pub struct JobRunnerBuildError(rayon::ThreadPoolBuildError);

impl fmt::Display for JobRunnerBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Error for JobRunnerBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

#[derive(Debug, Error)]
pub enum SignalInstallError {
    #[error("failed to register cancellation signals: {0}")]
    Register(std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_map_keeps_order() {
        let runner = JobRunner::inline();
        assert_eq!(runner.map(&[1, 2, 3], |value| value * 2), vec![2, 4, 6]);
    }

    #[test]
    fn indexed_maps_do_not_allocate_input_indices_and_keep_order() {
        let runner = JobRunner::with_workers(2).unwrap();
        assert_eq!(runner.map_indexed(4, |index| index * 3), vec![0, 3, 6, 9]);
        assert_eq!(
            runner.try_map_indexed(4, |index| Ok::<_, ()>(index * 2)),
            Ok(vec![0, 2, 4, 6])
        );
        assert_eq!(runner.parallelism(), 2);
    }

    #[test]
    fn cancellation_skips_inline_items_after_flag() {
        let runner = JobRunner::inline();
        let cancel = CancellationToken::new();
        let batch = runner.map_until_cancelled(&[1, 2, 3], &cancel, |value| {
            if *value == 1 {
                cancel.cancel();
            }
            *value
        });

        assert_eq!(batch.completed(), &[1]);
        assert_eq!(batch.skipped(), 2);
        assert!(batch.was_cancelled());
    }

    #[test]
    fn explicit_zero_jobs_is_inline() {
        let runner = JobRunner::from_jobs(Some(0)).unwrap();
        assert!(runner.is_inline());
    }

    #[test]
    fn install_runs_work_on_runner() {
        let runner = JobRunner::with_workers(2).unwrap();
        assert_eq!(runner.install(|| 42), 42);
    }

    #[test]
    fn join_runs_both_sides() {
        let runner = JobRunner::with_workers(2).unwrap();
        assert_eq!(runner.join(|| 20, || 22), (20, 22));
    }

    #[test]
    fn inline_for_each_runs_side_effects() {
        use std::sync::Mutex;

        let runner = JobRunner::inline();
        let cancel = CancellationToken::new();
        let seen = Mutex::new(Vec::new());

        let batch = runner.for_each_until_cancelled(&[1, 2, 3], &cancel, |value| {
            seen.lock().unwrap().push(*value);
        });

        assert_eq!(*seen.lock().unwrap(), vec![1, 2, 3]);
        assert_eq!(batch.completed().len(), 3);
        assert_eq!(batch.skipped(), 0);
        assert!(!batch.was_cancelled());
    }

    #[test]
    fn inline_for_each_init_reuses_worker_state_without_collecting_outputs() {
        use std::sync::Mutex;

        let runner = JobRunner::inline();
        let cancel = CancellationToken::new();
        let running_totals = Mutex::new(Vec::new());

        let batch = runner.for_each_init_until_cancelled(
            &[1, 2, 3],
            &cancel,
            || 0,
            |total, value| {
                *total += *value;
                running_totals.lock().unwrap().push(*total);
            },
        );

        assert_eq!(*running_totals.lock().unwrap(), vec![1, 3, 6]);
        assert_eq!(batch.completed().len(), 3);
        assert_eq!(batch.skipped(), 0);
        assert!(!batch.was_cancelled());
    }

    #[test]
    fn for_each_cancellation_skips_remaining_items() {
        let runner = JobRunner::inline();
        let cancel = CancellationToken::new();

        let batch = runner.for_each_until_cancelled(&[1, 2, 3], &cancel, |value| {
            if *value == 1 {
                cancel.cancel();
            }
        });

        assert_eq!(batch.completed().len(), 1);
        assert_eq!(batch.skipped(), 2);
        assert!(batch.was_cancelled());
    }

    #[test]
    fn pool_for_each_runs_all_items() {
        use std::sync::atomic::AtomicUsize;

        let runner = JobRunner::with_workers(2).unwrap();
        let cancel = CancellationToken::new();
        let items: Vec<usize> = (0..64).collect();
        let total = AtomicUsize::new(0);

        let batch = runner.for_each_until_cancelled(&items, &cancel, |value| {
            total.fetch_add(*value, Ordering::Relaxed);
        });

        assert_eq!(total.load(Ordering::Relaxed), items.iter().sum());
        assert_eq!(batch.completed().len(), items.len());
        assert_eq!(batch.skipped(), 0);
        assert!(!batch.was_cancelled());
    }
}
