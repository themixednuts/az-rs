//! Runtime-agnostic async surface.
//!
//! [`Concurrent`] mirrors the CPU runner's map families for the async world:
//! structured `join`/`try_join`, a bounded concurrent fan-out
//! ([`try_map_until_cancelled`](Concurrent::try_map_until_cancelled)) that
//! respects the context's fan-out policy and cooperative cancellation, and a
//! detaching [`spawn`](Concurrent::spawn) that requires an injected
//! [`AsyncExecutor`](crate::AsyncExecutor).
//!
//! The fan-out methods use only `futures` ([`FuturesUnordered`] /
//! `buffer_unordered`), so they run on ANY executor that drives the returned
//! future. No runtime is owned; this is why the core async surface is ungated.

use std::borrow::Cow;
use std::future::Future;

use futures::StreamExt;
use futures::stream::FuturesUnordered;

use crate::cancel::CancellationToken;
use crate::context::JobContext;
use crate::cpu::{JobBatch, collect_job_batch};
use crate::executor::{BoxFuture, JoinError, SpawnError};
use crate::progress::Progress;

/// In-flight fan-out item future carrying its input index, used so both the
/// priming and refill paths push the same `FuturesUnordered` element type.
type ItemFut<'a, R> = BoxFuture<'a, (usize, R)>;
/// Fallible variant of [`ItemFut`].
type TryItemFut<'a, R, E> = BoxFuture<'a, (usize, Result<R, E>)>;

/// Small owned label for an async task.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskName(Cow<'static, str>);

impl TaskName {
    /// Borrow the underlying label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&'static str> for TaskName {
    fn from(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }
}

impl From<String> for TaskName {
    fn from(value: String) -> Self {
        Self(Cow::Owned(value))
    }
}

impl std::fmt::Display for TaskName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The dominant resource a task uses, hinting fan-out width and pool choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TaskKind {
    /// CPU-bound work (decode, index, validate after bytes are local).
    Cpu,
    /// Blocking or async IO (filesystem, catalog reads).
    Io,
    /// Network round-trips (RPC, readiness probes).
    Network,
    /// A mix; sized like the most constrained resource.
    #[default]
    Mixed,
}

impl TaskKind {
    /// Stable lower-case label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Io => "io",
            Self::Network => "network",
            Self::Mixed => "mixed",
        }
    }
}

/// Per-item context threaded into context/progress-aware fan-out closures.
///
/// Mirrors what the CPU map families implicitly provide (cancellation), adding
/// a per-item [`Progress`] child and the input index.
#[derive(Debug)]
pub struct JobItemContext {
    /// Cooperative cancellation shared with the batch.
    pub cancel: CancellationToken,
    /// Per-item progress node (child of the batch's progress).
    pub progress: Progress,
    /// Zero-based index of this item in the input order.
    pub index: usize,
}

/// Abortable handle to a task spawned via [`Concurrent::spawn`].
pub struct TaskHandle<T> {
    result: futures::channel::oneshot::Receiver<T>,
    task: crate::executor::SpawnedTask,
}

impl<T> std::fmt::Debug for TaskHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskHandle").finish_non_exhaustive()
    }
}

impl<T> TaskHandle<T> {
    /// Await the task's value.
    ///
    /// # Errors
    ///
    /// Returns [`JoinError`] if the task was aborted or panicked before
    /// producing a value.
    pub async fn join(self) -> Result<T, JoinError> {
        // The executor join future resolves on completion/abort; the oneshot
        // carries the value. Await the value first; a dropped sender (abort or
        // panic) surfaces as a join error.
        match self.result.await {
            Ok(value) => Ok(value),
            Err(_canceled) => match self.task.join().await {
                Ok(()) => Err(JoinError::panicked()),
                Err(err) => Err(err),
            },
        }
    }

    /// Request abort of the underlying task.
    pub fn abort(&self) {
        self.task.abort();
    }
}

/// Async counterpart to the CPU runner, borrowed from a [`JobContext`].
#[derive(Debug)]
pub struct Concurrent<'a> {
    ctx: &'a JobContext,
}

impl<'a> Concurrent<'a> {
    pub(crate) const fn new(ctx: &'a JobContext) -> Self {
        Self { ctx }
    }

    /// Run two independent futures concurrently, returning both results.
    pub async fn join<A, B, FA, FB>(&self, a: FA, b: FB) -> (A, B)
    where
        FA: Future<Output = A>,
        FB: Future<Output = B>,
    {
        futures::join!(a, b)
    }

    /// Run two fallible futures concurrently, short-circuiting on the first
    /// error.
    ///
    /// # Errors
    ///
    /// Returns the first error produced by either future.
    pub async fn try_join<A, B, E, FA, FB>(&self, a: FA, b: FB) -> Result<(A, B), E>
    where
        FA: Future<Output = Result<A, E>>,
        FB: Future<Output = Result<B, E>>,
    {
        futures::try_join!(a, b)
    }

    /// Bounded concurrent fan-out over fallible per-item futures.
    ///
    /// Concurrency is capped at [`JobContext::fanout_limit`] for `kind` and the
    /// item count. Items that have not started after cancellation are skipped
    /// (mirroring the CPU `try_map_until_cancelled`). The first error requests
    /// cancellation so not-yet-started siblings are skipped; in-flight futures
    /// are allowed to finish. Results are returned in input order.
    ///
    /// # Errors
    ///
    /// Returns the first error produced by any item future.
    pub async fn try_map_until_cancelled<I, T, R, E, F, Fut>(
        &self,
        kind: TaskKind,
        items: I,
        f: F,
    ) -> Result<JobBatch<R>, E>
    where
        I: IntoIterator<Item = T>,
        F: Fn(JobItemContext, T) -> Fut + Send + Sync,
        Fut: Future<Output = Result<R, E>> + Send,
        T: Send,
        R: Send,
        E: Send,
    {
        let items: Vec<T> = items.into_iter().collect();
        let limit = self.ctx.fanout_limit(kind, items.len());
        let cancel = self.ctx.cancel().clone();
        let progress = self
            .ctx
            .progress()
            .root_with_cancel(kind.as_str(), cancel.clone());

        // (index, result-slot) drives input-order reassembly.
        let mut slots: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
        let mut error: Option<E> = None;

        // Box each item future to a single concrete type so both the priming
        // and the refill push the same `FuturesUnordered` element type. The box
        // is `Send` so the whole fan-out future stays `Send` and can itself be
        // awaited inside a spawned task.
        let make_future = |index: usize, item: T| -> Option<TryItemFut<'_, R, E>> {
            let item_cancel = cancel.clone();
            if item_cancel.is_cancelled() {
                return None;
            }
            let item_progress = progress.child(format!("{}#{index}", kind.as_str()));
            let fut = f(
                JobItemContext {
                    cancel: item_cancel,
                    progress: item_progress,
                    index,
                },
                item,
            );
            Some(Box::pin(async move { (index, fut.await) }))
        };

        let mut iter = items.into_iter().enumerate();
        let mut in_flight: FuturesUnordered<TryItemFut<'_, R, E>> = FuturesUnordered::new();

        // Prime up to `limit` futures.
        for (index, item) in iter.by_ref().take(limit.max(1)) {
            if let Some(fut) = make_future(index, item) {
                in_flight.push(fut);
            }
        }

        while let Some((index, result)) = in_flight.next().await {
            match result {
                Ok(value) => slots[index] = Some(value),
                Err(err) => {
                    cancel.cancel();
                    if error.is_none() {
                        error = Some(err);
                    }
                }
            }

            // Refill from the iterator, honoring cancellation.
            if let Some(fut) = iter
                .next()
                .and_then(|(index, item)| make_future(index, item))
            {
                in_flight.push(fut);
            }
        }

        progress.finish();

        if let Some(err) = error {
            return Err(err);
        }
        Ok(collect_job_batch(slots, cancel.is_cancelled()))
    }

    /// Bounded concurrent fan-out over infallible per-item futures.
    ///
    /// Concurrency is capped like [`try_map_until_cancelled`](Self::try_map_until_cancelled).
    /// Items not started after cancellation are skipped. Results are returned in
    /// input order.
    pub async fn map_until_cancelled<I, T, R, F, Fut>(
        &self,
        kind: TaskKind,
        items: I,
        f: F,
    ) -> JobBatch<R>
    where
        I: IntoIterator<Item = T>,
        F: Fn(JobItemContext, T) -> Fut + Send + Sync,
        Fut: Future<Output = R> + Send,
        T: Send,
        R: Send,
    {
        let result: Result<JobBatch<R>, std::convert::Infallible> = self
            .try_map_until_cancelled(kind, items, |item_ctx, item| {
                let fut = f(item_ctx, item);
                async move { Ok(fut.await) }
            })
            .await;
        match result {
            Ok(batch) => batch,
            Err(infallible) => match infallible {},
        }
    }

    /// Bounded concurrent fan-out collecting every result in input order.
    ///
    /// Unlike the `*_until_cancelled` variants this does not skip on
    /// cancellation; callers use it when every item must run.
    pub async fn join_all<I, T, R, F, Fut>(&self, kind: TaskKind, items: I, f: F) -> Vec<R>
    where
        I: IntoIterator<Item = T>,
        F: Fn(JobItemContext, T) -> Fut + Send + Sync,
        Fut: Future<Output = R> + Send,
        T: Send,
        R: Send,
    {
        let items: Vec<T> = items.into_iter().collect();
        let limit = self.ctx.fanout_limit(kind, items.len());
        let cancel = self.ctx.cancel().clone();
        let progress = self
            .ctx
            .progress()
            .root_with_cancel(kind.as_str(), cancel.clone());

        let mut slots: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
        let mut iter = items.into_iter().enumerate();

        let make_future = |index: usize, item: T| -> ItemFut<'_, R> {
            let item_progress = progress.child(format!("{}#{index}", kind.as_str()));
            let fut = f(
                JobItemContext {
                    cancel: cancel.clone(),
                    progress: item_progress,
                    index,
                },
                item,
            );
            Box::pin(async move { (index, fut.await) })
        };

        let mut in_flight: FuturesUnordered<ItemFut<'_, R>> = FuturesUnordered::new();
        for (index, item) in iter.by_ref().take(limit.max(1)) {
            in_flight.push(make_future(index, item));
        }

        while let Some((index, value)) = in_flight.next().await {
            slots[index] = Some(value);
            if let Some((index, item)) = iter.next() {
                in_flight.push(make_future(index, item));
            }
        }

        progress.finish();
        // Every slot is filled: one future is pushed per index and each yields
        // exactly once. `flatten` drops the `Option` wrapper without panicking.
        slots.into_iter().flatten().collect()
    }

    /// Spawn a detached task on the injected executor.
    ///
    /// The closure receives a child [`JobContext`] (child cancellation, shared
    /// reporter/executor). Requires an executor to have been injected via
    /// [`JobContext::with_executor`]; otherwise returns
    /// [`SpawnError::NoExecutor`].
    ///
    /// # Errors
    ///
    /// Returns [`SpawnError`] when no executor is injected, the backend cannot
    /// spawn, or the runtime is shutting down.
    pub fn spawn<T, F, Fut>(
        &self,
        _kind: TaskKind,
        name: impl Into<TaskName>,
        f: F,
    ) -> Result<TaskHandle<T>, SpawnError>
    where
        F: FnOnce(JobContext) -> Fut + Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let child = self.ctx.child(name.into());
        let (tx, rx) = futures::channel::oneshot::channel();
        let future = Box::pin(async move {
            let value = f(child).await;
            let _ = tx.send(value);
        });
        let task = self.ctx.executor().spawn(future)?;
        Ok(TaskHandle { result: rx, task })
    }
}
