//! Tokio runtime bridge for editor-side Cap'n Proto clients.
//!
//! GPUI foreground tasks are not Tokio tasks. Cap'n Proto two-party clients
//! use Tokio IO resources plus a `!Send` RPC pump, so editor UI code must enter
//! a current-thread runtime with a `LocalSet` before it creates or uses IPC
//! clients.
//!
//! The runtime is constructed per `block_on_editor_rpc` call and dropped while
//! the calling thread is still alive. It must never live in a `thread_local!`:
//! dropping a `LocalSet`/current-thread runtime from a TLS destructor aborts
//! the process when Tokio's own TLS was destroyed first (destructor order
//! between thread locals is unspecified), which killed ephemeral
//! `spawn_editor_rpc` worker threads with "fatal runtime error: thread local
//! panicked on drop". Every call site reconnects inside its future (Cap'n
//! Proto clients are `!Send`), so nothing relies on cross-call runtime state.

use std::future::Future;
use std::rc::{Rc, Weak};
use std::thread;

use gpui::{App, Global};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::oneshot;
use tokio::task::LocalSet;
use tracing::error;

use crate::error::{EditorError, EditorResult};

struct EditorRpcThreadRuntime {
    runtime: Runtime,
    local: LocalSet,
}

impl EditorRpcThreadRuntime {
    fn new() -> EditorResult<Self> {
        Ok(Self {
            runtime: Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .map_err(|source| EditorError::RpcRuntime { source })?,
            local: LocalSet::new(),
        })
    }

    fn block_on<F, R>(&self, future: F) -> R
    where
        F: Future<Output = R>,
    {
        self.local.block_on(&self.runtime, future)
    }
}

pub fn block_on_editor_rpc<F, R>(future: F) -> EditorResult<R>
where
    F: Future<Output = EditorResult<R>>,
{
    let runtime = EditorRpcThreadRuntime::new()?;
    runtime.block_on(future)
}

/// The App's liveness token: the strong `Rc` is parked in the App's own global
/// map, so a `Weak` handed to a foreground task upgrades exactly while that App
/// is alive and fails once it has been dropped.
///
/// It exists because `AsyncApp` has no fallible update in this gpui rev:
/// `AsyncApp::update` reaches the App through
/// `.expect("app was released before async operation completed")`, so it is only
/// safe to call once the caller has established that the App is still there.
#[derive(Clone, Default)]
struct AppAlive(Rc<()>);

impl Global for AppAlive {}

impl AppAlive {
    /// A weak handle to `cx`'s token, installing it on first use.
    fn weak(cx: &mut App) -> Weak<()> {
        if cx.try_global::<Self>().is_none() {
            cx.set_global(Self::default());
        }
        Rc::downgrade(&cx.global::<Self>().0)
    }
}

/// The real spawner: an ephemeral worker thread owns the Tokio runtime, and the
/// result is handed back to the GPUI foreground executor over a oneshot.
///
/// Outside test builds this *is* `spawn_editor_rpc` (see the alias below). Under
/// `cfg(test)` the `spawn_editor_rpc` name resolves to the inline shim so the
/// crate's UI tests stay synchronous, and this function is reached by name from
/// the module's own tests.
pub fn spawn_editor_rpc_threaded<F, Fut, R, Ui>(
    cx: &mut App,
    name: &'static str,
    future: F,
    publish: Ui,
) where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = EditorResult<R>> + 'static,
    R: Send + 'static,
    Ui: FnOnce(&mut App, EditorResult<R>) + 'static,
{
    let (tx, rx) = oneshot::channel();
    match thread::Builder::new()
        .name(format!("az-editor-rpc-{name}"))
        .spawn(move || {
            let result = block_on_editor_rpc(future());
            let _ = tx.send(result);
        }) {
        Ok(_thread) => {}
        Err(source) => {
            publish(cx, Err(EditorError::RpcRuntime { source }));
            return;
        }
    }

    let alive = AppAlive::weak(cx);

    cx.spawn(async move |cx| match rx.await {
        // Both arms have to agree that app teardown is survivable. The `Err` arm
        // below logs and returns; this one must too, and it cannot simply call
        // `cx.update`, which panics on an App that was released while the RPC was
        // in flight. Nothing awaits between the upgrade and the update, and the
        // App is dropped on this same thread, so a live token here means a live
        // App inside `cx.update`.
        Ok(result) => {
            if alive.upgrade().is_some() {
                cx.update(move |cx| publish(cx, result));
            } else {
                error!(
                    name,
                    "editor RPC result dropped; app released before publish"
                );
            }
        }
        Err(error) => {
            error!(%error, name, "editor RPC worker dropped result");
        }
    })
    .detach();
}

#[cfg(not(test))]
pub use spawn_editor_rpc_threaded as spawn_editor_rpc;

#[cfg(test)]
pub fn spawn_editor_rpc<F, Fut, R, Ui>(cx: &mut App, _name: &'static str, future: F, publish: Ui)
where
    F: FnOnce() -> Fut + 'static,
    Fut: Future<Output = EditorResult<R>> + 'static,
    R: 'static,
    Ui: FnOnce(&mut App, EditorResult<R>) + 'static,
{
    let result = block_on_editor_rpc(future());
    publish(cx, result);
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::task;

    use super::*;

    /// The module's headline invariant: the runtime is built per call and dropped
    /// before the call returns, so nothing survives into the next call.
    ///
    /// Both assertions distinguish per-call construction from a cached runtime
    /// (the `thread_local!` shape the module doc forbids). Call 1 parks a
    /// `spawn_local` task that owns a oneshot receiver. If the runtime and its
    /// `LocalSet` were reused, that task would still be alive after call 1
    /// returns: `tx.send` would succeed, and call 2's yields would poll the task
    /// and flip `resumed`. Because the runtime is dropped inside call 1, the task
    /// and its receiver are gone by the time call 1 returns.
    #[test]
    fn block_on_editor_rpc_drops_its_runtime_before_returning() {
        let (tx, rx) = oneshot::channel::<()>();
        let resumed = Arc::new(AtomicBool::new(false));

        let first = {
            let resumed = Arc::clone(&resumed);
            block_on_editor_rpc(async move {
                task::spawn_local(async move {
                    if rx.await.is_ok() {
                        resumed.store(true, Ordering::SeqCst);
                    }
                });
                Ok(1_u32)
            })
            .expect("first call completes")
        };
        assert_eq!(first, 1);

        assert!(
            tx.send(()).is_err(),
            "call 1's LocalSet outlived the call, so its parked task still holds \
             the receiver; the runtime must be dropped before block_on returns"
        );

        let second = block_on_editor_rpc(async {
            for _ in 0..16 {
                task::yield_now().await;
            }
            Ok(2_u32)
        })
        .expect("second call completes");
        assert_eq!(second, 2, "each call resolves to its own future's value");

        assert!(
            !resumed.load(Ordering::SeqCst),
            "call 2 polled a task left over from call 1, so the runtime is shared"
        );
    }

    /// The builder enables both the time and IO drivers. Editor RPC futures reach
    /// for `tokio::time` (request timeouts) and Tokio IO resources (Cap'n Proto
    /// two-party clients), and both panic at runtime when their driver is absent.
    ///
    /// Counterfactual: drop `.enable_time()` and `sleep` panics with "time driver
    /// disabled"; drop `.enable_io()` and binding the listener panics with "no
    /// reactor running". Either way the future never returns a value and the
    /// panic unwinds out of `block_on_editor_rpc`.
    #[test]
    fn block_on_editor_rpc_runtime_drives_time_and_io() {
        let port = block_on_editor_rpc(async {
            // A real (tiny) elapse: proves the timer is driven, not merely built.
            tokio::time::sleep(Duration::from_millis(1)).await;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("loopback bind needs the IO driver");
            Ok(listener.local_addr().expect("bound addr").port())
        })
        .expect("call completes");

        assert_ne!(port, 0, "the listener bound to a concrete ephemeral port");
    }

    /// `block_on_editor_rpc` is transparent to the future's own error: it returns
    /// the `Err` unchanged rather than folding it into `EditorError::RpcRuntime`
    /// (which is reserved for runtime construction and thread-spawn failures).
    #[test]
    fn block_on_editor_rpc_returns_the_future_error_unchanged() {
        let result: EditorResult<()> =
            block_on_editor_rpc(async { Err(EditorError::NoActiveEditorSession) });

        assert!(
            matches!(result, Err(EditorError::NoActiveEditorSession)),
            "expected the future's own error variant, got {result:?}"
        );
    }

    /// A drop-order probe: records its label when it is destroyed.
    struct Marker {
        log: Arc<Mutex<Vec<&'static str>>>,
        label: &'static str,
    }

    impl Marker {
        fn new(log: &Arc<Mutex<Vec<&'static str>>>, label: &'static str) -> Self {
            Self {
                log: Arc::clone(log),
                label,
            }
        }
    }

    impl Drop for Marker {
        fn drop(&mut self) {
            self.log.lock().expect("marker log").push(self.label);
        }
    }

    thread_local! {
        static THREAD_EXIT_MARKER: RefCell<Option<Marker>> = const { RefCell::new(None) };
    }

    /// The hazard the module doc records, pinned as an ordering fact on the kind
    /// of thread that hit it: an ephemeral `spawn_editor_rpc` worker tears its
    /// runtime down *inside* the call, while the thread is still running normal
    /// code -- never from a thread-local destructor at thread exit.
    ///
    /// The worker parks a task inside the runtime holding a marker, so that
    /// marker records exactly when the `LocalSet` is destroyed. It must land
    /// before the plain statement that follows the call. Cache the runtime in a
    /// `thread_local!` (the shape the doc forbids) and the `LocalSet` survives
    /// the call, so "runtime teardown" slides past "worker body resumed" to
    /// thread-exit time and the sequence inverts -- which is where the doc's
    /// "thread local panicked on drop" abort came from, since by then Tokio's own
    /// thread-locals may already be gone.
    ///
    /// The trailing "thread exit" entry is not the discriminating assertion; it
    /// is there to show the worker ran its thread-local destructors and
    /// terminated rather than being torn down mid-flight.
    #[test]
    fn ephemeral_worker_thread_drops_its_runtime_inside_the_call() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let worker = {
            let log = Arc::clone(&log);
            thread::Builder::new()
                .name("az-editor-rpc-teardown".to_owned())
                .spawn(move || {
                    THREAD_EXIT_MARKER.with(|slot| {
                        *slot.borrow_mut() = Some(Marker::new(&log, "thread exit"));
                    });

                    let inside_runtime = Marker::new(&log, "runtime teardown");
                    block_on_editor_rpc(async move {
                        task::spawn_local(async move {
                            let _inside_runtime = inside_runtime;
                            std::future::pending::<()>().await;
                        });
                        Ok(())
                    })
                    .expect("worker call completes");

                    log.lock().expect("marker log").push("worker body resumed");
                })
                .expect("worker thread spawns")
        };

        worker
            .join()
            .expect("worker thread exits without panicking");

        assert_eq!(
            log.lock().expect("marker log").as_slice(),
            ["runtime teardown", "worker body resumed", "thread exit"],
            "the runtime must be dropped inside block_on_editor_rpc, before the \
             worker thread runs another statement"
        );
    }

    /// Fires when the closure that owns it is dropped. Handed to `publish`, that
    /// is exactly when the app-side task ends -- whether or not `publish` ran.
    struct CompletionSignal(Option<oneshot::Sender<()>>);

    impl Drop for CompletionSignal {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

    /// Captures ERROR-level events on the current thread. `spawn_editor_rpc`'s
    /// orphan branch is the only place the module logs, and it logs from the
    /// GPUI foreground task, which the test scheduler steps on this thread.
    #[derive(Clone, Default)]
    struct ErrorLogCapture(Arc<Mutex<Vec<String>>>);

    struct FieldVisitor(String);

    impl tracing::field::Visit for FieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            use std::fmt::Write;
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }

    impl tracing::Subscriber for ErrorLogCapture {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

        fn event(&self, event: &tracing::Event<'_>) {
            if *event.metadata().level() != tracing::Level::ERROR {
                return;
            }
            let mut visitor = FieldVisitor(String::new());
            event.record(&mut visitor);
            self.0.lock().expect("captured events").push(visitor.0);
        }

        fn enter(&self, _span: &tracing::span::Id) {}

        fn exit(&self, _span: &tracing::span::Id) {}
    }

    /// Drives the GPUI test scheduler until `future` resolves. `allow_parking`
    /// is what lets the scheduler wait on a wakeup that arrives from a real
    /// worker thread; it also caps the wait, panicking after 15s.
    fn settle<R>(cx: &gpui::TestAppContext, future: impl Future<Output = R>) -> R {
        cx.foreground_executor().block_test(future)
    }

    /// The worker's `Ok` reaches `publish` with a live `&mut App`, on the app
    /// thread, carrying the value the future produced.
    ///
    /// Counterfactual: drop the `cx.spawn`/`rx.await` hand-off and nothing ever
    /// wakes the foreground task, so `settle` never resolves and the scheduler's
    /// 15s parking cap fires; publish the wrong side and the value mismatches.
    #[gpui::test]
    fn spawn_editor_rpc_publishes_a_worker_success_into_the_app(cx: &gpui::TestAppContext) {
        cx.executor().allow_parking();

        let (published_tx, published_rx) = oneshot::channel::<EditorResult<u32>>();
        cx.update(|app| {
            spawn_editor_rpc_threaded(
                app,
                "publishes-ok",
                || async { Ok(7_u32) },
                move |_app, result| {
                    let _ = published_tx.send(result);
                },
            );
        });

        let published = settle(cx, published_rx).expect("publish ran on the app thread");

        assert_eq!(published.expect("worker result"), 7);
    }

    /// Failures take the same route as successes: the worker's `Err` is handed
    /// to `publish` intact rather than being dropped, retried, or turned into a
    /// runtime error.
    ///
    /// Counterfactual: swallow the error on the worker side and `settle` hangs
    /// to the parking cap; wrap it and the variant match fails.
    #[gpui::test]
    fn spawn_editor_rpc_publishes_a_worker_error_into_the_app(cx: &gpui::TestAppContext) {
        cx.executor().allow_parking();

        let (published_tx, published_rx) = oneshot::channel::<EditorResult<u32>>();
        cx.update(|app| {
            spawn_editor_rpc_threaded(
                app,
                "publishes-err",
                || async { Err(EditorError::NoActiveEditorSession) },
                move |_app, result| {
                    let _ = published_tx.send(result);
                },
            );
        });

        let published = settle(cx, published_rx).expect("publish ran on the app thread");

        assert!(
            matches!(published, Err(EditorError::NoActiveEditorSession)),
            "expected the worker's own error variant, got {published:?}"
        );
    }

    /// A worker that dies without sending drops its half of the oneshot. The
    /// app-side task must log and end, not panic and not publish -- and the app
    /// must still serve the next RPC.
    ///
    /// Counterfactual: `rx.await`'s `Err` arm is what keeps this quiet. Replace
    /// it with `.expect(..)` and the panic lands on the app thread inside
    /// `settle`, failing the test; drop the `error!` and the log assertion
    /// fails; let the app-side task die in a way that poisons the foreground
    /// executor and the follow-up RPC never publishes.
    #[gpui::test]
    fn spawn_editor_rpc_logs_and_survives_a_worker_that_never_sends(cx: &gpui::TestAppContext) {
        cx.executor().allow_parking();

        let capture = ErrorLogCapture::default();
        let records = Arc::clone(&capture.0);
        let log_guard = tracing::subscriber::set_default(capture);

        let published = Rc::new(Cell::new(false));
        let (finished_tx, finished_rx) = oneshot::channel::<()>();

        cx.update({
            let published = Rc::clone(&published);
            move |app| {
                let signal = CompletionSignal(Some(finished_tx));
                spawn_editor_rpc_threaded(
                    app,
                    "orphan",
                    || async { panic!("editor RPC worker died mid-call") },
                    move |_app: &mut App, _result: EditorResult<u32>| {
                        drop(signal);
                        published.set(true);
                    },
                );
            }
        });

        settle(cx, finished_rx).expect("the app-side task ran to completion");
        drop(log_guard);

        assert!(
            !published.get(),
            "publish must not run when the worker never sent a result"
        );
        let records = records.lock().expect("captured events");
        assert!(
            records
                .iter()
                .any(|event| event.contains("editor RPC worker dropped result")),
            "the orphan branch must log; captured {records:?}"
        );
        drop(records);

        let (second_tx, second_rx) = oneshot::channel::<EditorResult<u32>>();
        cx.update(|app| {
            spawn_editor_rpc_threaded(
                app,
                "after-orphan",
                || async { Ok(9_u32) },
                move |_app, result| {
                    let _ = second_tx.send(result);
                },
            );
        });

        let second =
            settle(cx, second_rx).expect("the app still publishes after an orphaned worker");
        assert_eq!(
            second.expect("second worker result"),
            9,
            "the app must keep serving RPCs after a worker dies"
        );
    }

    /// The far side of the same hand-off: a result that arrives *after* the App is
    /// released must log and drop it, exactly as the orphaned-worker arm above
    /// does. The two arms have to agree that app teardown is survivable.
    ///
    /// `#[gpui::test]` cannot reach this -- that harness owns the App for the
    /// whole test -- so this one owns its `TestAppContext`, drops it while the
    /// worker is still parked, and only then releases the worker. The executors
    /// are cloned first and outlive the App, so the wakeup still schedules the
    /// app-side task; it just has no app left to publish into.
    ///
    /// Counterfactual: publish through `AsyncApp::update` (this module's shape
    /// before ticket 045) and `block_test` panics on this thread with "app was
    /// released before async operation completed"; drop the `error!` and the log
    /// assertion fails; publish anyway and `published` flips.
    #[test]
    fn spawn_editor_rpc_logs_and_drops_a_result_when_the_app_is_released() {
        let capture = ErrorLogCapture::default();
        let records = Arc::clone(&capture.0);
        let log_guard = tracing::subscriber::set_default(capture);

        let cx = gpui::TestAppContext::single();
        cx.executor().allow_parking();
        let foreground = cx.foreground_executor().clone();

        let published = Rc::new(Cell::new(false));
        let (finished_tx, finished_rx) = oneshot::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();

        cx.update({
            let published = Rc::clone(&published);
            move |app| {
                let signal = CompletionSignal(Some(finished_tx));
                spawn_editor_rpc_threaded(
                    app,
                    "released-app",
                    move || async move {
                        release_rx.recv().expect("the test releases the worker");
                        Ok(7_u32)
                    },
                    move |_app: &mut App, _result: EditorResult<u32>| {
                        drop(signal);
                        published.set(true);
                    },
                );
            }
        });

        drop(cx);
        release_tx.send(()).expect("the worker is still parked");

        foreground
            .block_test(finished_rx)
            .expect("the app-side task ran to completion");
        drop(log_guard);

        assert!(
            !published.get(),
            "publish must not run once the App has been released"
        );
        let records = records.lock().expect("captured events");
        let logged_release = records
            .iter()
            .any(|event| event.contains("app released before publish"));
        let captured = format!("{records:?}");
        drop(records);
        assert!(
            logged_release,
            "the released-app branch must log; captured {captured}"
        );
    }
}
