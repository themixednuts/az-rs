//! Unified engine concurrency/job library.
//!
//! `az-work` is the engine-wide work abstraction covering BOTH Rayon CPU
//! data-parallel execution AND async/concurrent execution under one coherent,
//! runtime-agnostic surface, with first-class structured progress and shared
//! cancellation.
//!
//! # Two worlds, one context
//!
//! Everything threads through a single [`JobContext`]:
//!
//! - [`JobContext::cpu`] (alias [`JobContext::runner`]) — the [`CpuRunner`]
//!   (Rayon data-parallel map families). This is the original `JobRunner`
//!   verbatim; the contract is unchanged. Callers keep ownership of domain
//!   state and merge worker-local results explicitly.
//! - [`JobContext::concurrent`] — the [`Concurrent`] async surface: structured
//!   `join`, a bounded concurrent fan-out, and a detaching `spawn`.
//! - [`JobContext::blocking`] — a bounded channel pipeline adapter for blocking
//!   producer/worker/consumer jobs with current-thread consumption.
//! - [`JobContext::cancel`] — a [`CancellationToken`] with both the original
//!   cheap atomic poll (CPU callers + signal handlers) AND async wakeups /
//!   hierarchical child tokens (async surface).
//! - [`JobContext::progress`] — a no-op-by-default [`Reporter`] emitting
//!   structured [`ProgressEvent`]s.
//!
//! # Runtime-agnostic async
//!
//! The async surface is a THIN structured-concurrency/policy layer, never a
//! runtime owner. The fan-out methods run on whatever drives the returned
//! future (they use only `futures`), so the core surface is UNGATED. Only
//! [`Concurrent::spawn`] detaches a task, and for that the host injects an
//! [`AsyncExecutor`] via [`JobContext::with_executor`]. Concrete adapters
//! (tokio, tokio-local, bevy) are feature-gated so shared crates accreting a
//! [`JobContext`] never grow cfgs.

mod cancel;
mod concurrent;
mod context;
mod cpu;
mod executor;
mod pipeline;
#[cfg(feature = "process")]
mod process;
mod progress;

pub use cancel::{
    CancellationToken, SignalInstallError, cancellation_token_with_signal_handlers,
    install_signal_handlers,
};
pub use concurrent::{Concurrent, JobItemContext, TaskHandle, TaskKind, TaskName};
pub use context::JobContext;
pub use cpu::{CpuRunner, JobBatch, JobRunnerBuildError, JobRunnerPolicy};
pub use executor::{
    AsyncExecutor, BoxFuture, JoinError, LocalBoxFuture, NoopExecutor, SpawnError, SpawnedTask,
};
pub use pipeline::{
    Blocking, BlockingPipelineConfig, BlockingPipelineError, BlockingPipelineStage,
    BlockingPipelineSummary,
};
#[cfg(feature = "process")]
pub use process::{OwnedSynchronousChild, OwnedSynchronousCommandTree, owned_command_output};
pub use progress::{
    Fraction, Progress, ProgressEvent, ProgressId, ProgressKind, ProgressName, ProgressSink,
    Reporter,
};

#[cfg(feature = "bevy")]
pub use executor::BevyExecutor;
#[cfg(feature = "tokio")]
pub use executor::TokioExecutor;
#[cfg(feature = "tokio-local")]
pub use executor::TokioLocalExecutor;

/// Compatibility alias for the renamed [`CpuRunner`].
///
/// Existing callers reference `az_work::JobRunner`; this keeps their paths
/// resolving with zero behavioral change.
pub type JobRunner = CpuRunner;

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn cpu_map_families_unchanged() {
        let ctx = JobContext::new(CpuRunner::inline(), CancellationToken::new());
        let doubled = ctx.runner().map(&[1, 2, 3], |n| n * 2);
        assert_eq!(doubled, vec![2, 4, 6]);

        let cancel = CancellationToken::new();
        let batch: JobBatch<i32> = ctx
            .cpu()
            .try_map_until_cancelled(&[1, 2, 3], &cancel, |n| Ok::<_, ()>(n * 10))
            .unwrap();
        assert_eq!(batch.completed(), &[10, 20, 30]);
        assert!(!batch.was_cancelled());
    }

    #[test]
    fn cpu_try_map_cancels_on_error() {
        let ctx = JobContext::new(CpuRunner::inline(), CancellationToken::new());
        let cancel = CancellationToken::new();
        let result: Result<JobBatch<i32>, &str> =
            ctx.cpu().try_map_until_cancelled(&[1, 2, 3], &cancel, |n| {
                if *n == 2 { Err("boom") } else { Ok(*n) }
            });
        assert_eq!(result, Err("boom"));
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn runner_alias_resolves() {
        let ctx = JobContext::default();
        // Both names point at the same runner.
        assert_eq!(ctx.cpu().policy(), ctx.runner().policy());
    }

    #[test]
    fn cancellation_child_propagates_one_way() {
        let parent = CancellationToken::new();
        let child = parent.child_token();
        assert!(!child.is_cancelled());
        parent.cancel();
        assert!(child.is_cancelled());

        // Child cancellation does not flow back to the parent.
        let parent2 = CancellationToken::new();
        let child2 = parent2.child_token();
        child2.cancel();
        assert!(child2.is_cancelled());
        assert!(!parent2.is_cancelled());
    }

    #[test]
    fn blocking_cancellation_edge_fires_with_async_state() {
        let token = CancellationToken::new();
        let signal = az_jobs::Cancellation::cancellation_signal(&token);
        let waiter = std::thread::spawn(move || signal.wait());
        assert!(!token.is_cancelled());
        token.cancel();
        waiter.join().unwrap();
        assert!(token.is_cancelled());
    }

    #[test]
    fn fanout_limit_respects_policy_and_kind() {
        let inline = JobContext::new(CpuRunner::inline(), CancellationToken::new());
        assert_eq!(inline.fanout_limit(TaskKind::Cpu, 10), 1);
        // IO biases wider but never beyond item count.
        assert_eq!(inline.fanout_limit(TaskKind::Io, 10), 4);
        assert_eq!(inline.fanout_limit(TaskKind::Io, 2), 2);
        assert_eq!(inline.fanout_limit(TaskKind::Cpu, 0), 0);

        let workers = JobContext::new(
            CpuRunner::with_workers(2).unwrap(),
            CancellationToken::new(),
        );
        assert_eq!(workers.fanout_limit(TaskKind::Cpu, 10), 2);
        assert_eq!(workers.fanout_limit(TaskKind::Network, 100), 8);
    }

    // A trivial inline executor that drives futures to completion synchronously
    // so async tests need no real runtime.
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        futures::executor::block_on(fut)
    }

    // Cooperative single-poll yield: returns Pending exactly once.
    async fn yield_now() {
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

    #[test]
    fn async_bounded_fanout_completes_in_order() {
        let ctx = JobContext::new(
            CpuRunner::with_workers(4).unwrap(),
            CancellationToken::new(),
        );
        let batch: JobBatch<usize> = block_on(async {
            ctx.concurrent()
                .try_map_until_cancelled(TaskKind::Io, 0..10usize, |item_ctx, n| async move {
                    assert_eq!(item_ctx.index, n);
                    Ok::<_, ()>(n * 2)
                })
                .await
                .unwrap()
        });
        assert_eq!(
            batch.into_completed(),
            (0..10usize).map(|n| n * 2).collect::<Vec<_>>()
        );
    }

    #[test]
    fn async_fanout_never_exceeds_limit() {
        let ctx = JobContext::new(
            CpuRunner::with_workers(2).unwrap(),
            CancellationToken::new(),
        );
        // CPU kind => limit == workers == 2.
        let max_concurrent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let current = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let max = Arc::clone(&max_concurrent);
        let cur = Arc::clone(&current);
        let _batch: JobBatch<()> = block_on(async move {
            ctx.concurrent()
                .map_until_cancelled(TaskKind::Cpu, 0..20usize, move |_item_ctx, _n| {
                    let max = Arc::clone(&max);
                    let cur = Arc::clone(&cur);
                    async move {
                        let now = cur.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                        max.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                        // Yield once so other primed items can interleave before
                        // this one releases its concurrency slot.
                        yield_now().await;
                        cur.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    }
                })
                .await
        });
        assert!(max_concurrent.load(std::sync::atomic::Ordering::SeqCst) <= 2);
    }

    #[test]
    fn async_fanout_skips_after_error() {
        let ctx = JobContext::new(CpuRunner::inline(), CancellationToken::new());
        let result: Result<JobBatch<usize>, &str> = block_on(async {
            ctx.concurrent()
                .try_map_until_cancelled(TaskKind::Cpu, 0..10usize, |_item_ctx, n| async move {
                    if n == 3 { Err("boom") } else { Ok(n) }
                })
                .await
        });
        assert_eq!(result, Err("boom"));
        assert!(ctx.cancel().is_cancelled());
    }

    #[test]
    fn progress_events_emitted_in_order() {
        #[derive(Default)]
        struct Recorder {
            events: Mutex<Vec<ProgressKind>>,
        }
        impl ProgressSink for Recorder {
            fn event(&self, event: ProgressEvent) {
                self.events.lock().unwrap().push(event.kind);
            }
        }

        let recorder = Arc::new(Recorder::default());
        let reporter = Reporter::new(recorder.clone());
        let root = reporter.root("build");
        root.set_total(3);
        root.advance(1);
        root.message("compiling");
        root.advance(2);
        root.finish();

        let events = recorder.events.lock().unwrap().clone();
        assert_eq!(
            events,
            [
                ProgressKind::Started,
                ProgressKind::SetTotal(3),
                ProgressKind::Advance(1),
                ProgressKind::Message("compiling".to_string()),
                ProgressKind::Advance(2),
                ProgressKind::Finished,
            ]
        );
    }

    #[test]
    fn fraction_basis_points_are_monotonic_and_clamped() {
        // Unknown reads as 0 bp; Complete reads as full scale.
        assert_eq!(Fraction::Unknown { done: 5 }.to_basis_points(), 0);
        assert_eq!(Fraction::Complete.to_basis_points(), Fraction::BASIS_POINTS);

        // Exact fractions convert without float drift and stay monotone.
        let quarter = Fraction::exact(1, 4);
        let half = Fraction::exact(2, 4);
        let full = Fraction::exact(4, 4);
        assert_eq!(quarter.to_basis_points(), 2_500);
        assert_eq!(half.to_basis_points(), 5_000);
        assert_eq!(full.to_basis_points(), 10_000);
        assert!(quarter.to_basis_points() < half.to_basis_points());
        assert!(half.to_basis_points() < full.to_basis_points());

        // done > total clamps; total == 0 collapses to Unknown.
        assert_eq!(Fraction::exact(9, 4).to_basis_points(), 10_000);
        assert!(matches!(
            Fraction::exact(3, 0),
            Fraction::Unknown { done: 3 }
        ));

        // Large counts cannot overflow the basis-point conversion (floor
        // division gives 4999 for just-under-half).
        let big = Fraction::exact(u64::MAX / 2, u64::MAX);
        assert_eq!(big.to_basis_points(), 4_999);
    }

    #[test]
    fn fraction_wire_roundtrip() {
        let cases = [
            Fraction::exact(3, 10),
            Fraction::Unknown { done: 7 },
            Fraction::exact(10, 10),
        ];
        for case in cases {
            let (done, total) = case.raw();
            let restored = Fraction::from_raw(done, total);
            // Exact and Unknown survive the round-trip exactly.
            assert_eq!(restored, case, "roundtrip mismatch for {case:?}");
        }
        // Complete loses its (absent) total on the wire and reads as Unknown{0};
        // the daemon aggregator re-derives Complete from the phase finish event.
        assert!(matches!(
            Fraction::from_raw(0, 0),
            Fraction::Unknown { done: 0 }
        ));
    }

    #[test]
    fn cancelled_future_fires_on_cancel() {
        let token = CancellationToken::new();
        let waiter = token.clone();
        let fired = block_on(async move {
            token.cancel();
            // Already cancelled: the future resolves immediately.
            waiter.cancelled().await;
            true
        });
        assert!(fired);
    }

    #[test]
    fn spawn_without_executor_errors() {
        let ctx = JobContext::default();
        let result = ctx
            .concurrent()
            .spawn(TaskKind::Io, "noop", |_child| async move { 1u32 });
        assert!(matches!(result.err(), Some(SpawnError::NoExecutor)));
    }

    #[test]
    fn blocking_pipeline_streams_outputs_to_consumer() {
        let ctx = JobContext::new(
            CpuRunner::with_workers(4).unwrap(),
            CancellationToken::new(),
        );
        let mut outputs = Vec::new();
        let summary = ctx
            .blocking()
            .try_for_each(
                &BlockingPipelineConfig::for_kind(TaskKind::Cpu)
                    .with_worker_count(2)
                    .with_input_capacity(2)
                    .with_output_capacity(2),
                (0..8).map(Ok::<_, &'static str>),
                |item, _cancel| Ok(Some(item * 2)),
                |item| {
                    outputs.push(item);
                    Ok(())
                },
            )
            .unwrap();

        outputs.sort_unstable();
        assert_eq!(outputs, vec![0, 2, 4, 6, 8, 10, 12, 14]);
        assert_eq!(summary.produced, 8);
        assert_eq!(summary.consumed, 8);
        assert_eq!(summary.workers, 2);
    }

    #[test]
    fn blocking_pipeline_reports_worker_errors_and_cancels() {
        let ctx = JobContext::new(
            CpuRunner::with_workers(4).unwrap(),
            CancellationToken::new(),
        );
        let error = ctx
            .blocking()
            .try_for_each(
                &BlockingPipelineConfig::for_kind(TaskKind::Cpu).with_worker_count(2),
                (0..10).map(Ok::<_, &'static str>),
                |item, _cancel| {
                    if item == 3 {
                        Err("boom")
                    } else {
                        Ok(Some(item))
                    }
                },
                |_item| Ok(()),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            BlockingPipelineError::Stage {
                stage: BlockingPipelineStage::Worker,
                source: "boom"
            }
        ));
        assert!(!ctx.cancel().is_cancelled());
    }
}
