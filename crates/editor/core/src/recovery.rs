//! Typed source-session crash recovery.
//!
//! Project-host owns recovery persistence. The editor only requests a
//! `SaveRecovery` checkpoint for an open source session; a later `Open`
//! restores a matching checkpoint through the same typed source-session path.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use az_proto_project::vnext::{DiagnosticSeverity, SourceSessionCommand, SourceSessionResult};
use futures_timer::Delay;
use gpui::App;
use tracing::{error, info, instrument};

use crate::attach::EditorAttachSession;
use crate::authored_edit::ReflectedPrefabEditSession;
use crate::error::{EditorError, EditorResult};

pub const DEFAULT_AUTOSAVE_INTERVAL: Duration = Duration::from_mins(2);
pub const DEFAULT_AUTOSAVE_DEBOUNCE: Duration = Duration::from_secs(5);

/// Debounced bridge from reflected editor changes to project-host recovery.
pub struct EditorRecoveryController {
    source_session: ReflectedPrefabEditSession,
    autosave: Arc<Mutex<AutosaveState>>,
    /// Owned only by the aggregate-held controller. Command and autosave
    /// copies retain a receiver, never this sender, so replacing the slot
    /// stops delayed recovery work for the retired session.
    #[allow(
        dead_code,
        reason = "RAII cancellation guard; dropping it wakes cloned autosave workers"
    )]
    close: Option<tokio::sync::watch::Sender<()>>,
    close_rx: tokio::sync::watch::Receiver<()>,
}

impl Clone for EditorRecoveryController {
    fn clone(&self) -> Self {
        Self {
            source_session: self.source_session.clone(),
            autosave: Arc::clone(&self.autosave),
            // Only the slot-owned instance may keep the sender alive.
            close: None,
            close_rx: self.close_rx.clone(),
        }
    }
}

impl EditorRecoveryController {
    #[must_use]
    pub fn new(source_session: ReflectedPrefabEditSession) -> Self {
        let (close, close_rx) = tokio::sync::watch::channel(());
        Self {
            source_session,
            autosave: Arc::new(Mutex::new(AutosaveState::default())),
            close: Some(close),
            close_rx,
        }
    }

    /// Binds a recovery controller to the typed source session of an attached
    /// editor session.
    ///
    /// # Errors
    ///
    /// Returns whatever [`ReflectedPrefabEditSession::connect_attached`]
    /// returns: [`EditorError::RpcTransport`] when the attached session's
    /// project-host cannot be dialed, or the discovery error it raises when the
    /// session descriptor does not resolve. Nothing here fails on its own.
    #[instrument(
        skip(session),
        fields(session = %session.session_slug, session_id = %session.session_id)
    )]
    pub async fn connect_attached(session: &EditorAttachSession) -> EditorResult<Self> {
        let source_session = ReflectedPrefabEditSession::connect_attached(session).await?;
        info!(session = %session.session_slug, "configured typed source-session recovery");
        Ok(Self::new(source_session))
    }

    pub fn note_dirty(&self, source_path: String, revision: u64, cx: &mut App) {
        self.note_dirty_with_timing(
            source_path,
            revision,
            cx,
            DEFAULT_AUTOSAVE_INTERVAL,
            DEFAULT_AUTOSAVE_DEBOUNCE,
        );
    }

    pub fn note_dirty_with_timing(
        &self,
        source_path: String,
        revision: u64,
        cx: &mut App,
        interval: Duration,
        debounce: Duration,
    ) {
        if !self.mark_dirty_and_should_spawn(&source_path, revision) {
            return;
        }

        let controller = self.clone();
        cx.spawn(async move |_| {
            controller
                .run_autosave_loop(source_path, interval, debounce)
                .await;
        })
        .detach();
    }

    /// Records one dirty edit and reports whether its caller must spawn the
    /// autosave loop: exactly one loop runs per source until a successful
    /// checkpoint or removal finishes it.
    fn mark_dirty_and_should_spawn(&self, source_path: &str, revision: u64) -> bool {
        let mut autosave = self.autosave.lock().expect("autosave state mutex poisoned");
        let entry = autosave.sources.entry(source_path.to_owned()).or_default();
        entry.last_edit = Some(Instant::now());
        entry.revision = revision;
        let should_spawn = !entry.task_running;
        entry.task_running = true;
        drop(autosave);
        should_spawn
    }

    /// Drops the autosave bookkeeping for `source_path` once it has been saved,
    /// so the in-flight loop finishes without writing another checkpoint.
    ///
    /// # Panics
    ///
    /// Panics if the autosave state mutex is poisoned, which only happens when
    /// another thread panicked while holding it.
    pub fn note_saved(&self, source_path: &str) {
        self.autosave
            .lock()
            .expect("autosave state mutex poisoned")
            .sources
            .remove(source_path);
    }

    /// Persists the current typed working value in project-host recovery storage.
    ///
    /// # Errors
    ///
    /// Returns whatever [`ReflectedPrefabEditSession::lifecycle`] returns for
    /// the `SaveRecovery` command — [`EditorError::ServiceDiscovery`] when the
    /// attached session cannot be resolved to a project-host, or the transport
    /// error from the round-trip itself. Returns
    /// [`EditorError::InvalidArgument`] when project-host answers but reports
    /// error-severity diagnostics for `source_path`, since a checkpoint that
    /// diagnosed an error did not persist.
    #[instrument(skip(self), fields(source_path, expected_revision))]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn save_recovery(
        &self,
        source_path: &str,
        expected_revision: u64,
    ) -> EditorResult<SourceSessionResult> {
        let result = self
            .source_session
            .lifecycle(
                source_path,
                SourceSessionCommand::SaveRecovery,
                expected_revision,
            )
            .await?;
        ensure_source_session_success(source_path, "save recovery", result)
    }

    /// Opens a typed source session. Project-host restores an eligible crash
    /// checkpoint while opening it.
    ///
    /// # Errors
    ///
    /// Returns whatever [`ReflectedPrefabEditSession::lifecycle`] returns for
    /// the `Open` command — [`EditorError::ServiceDiscovery`] when the attached
    /// session cannot be resolved to a project-host, or the transport error from
    /// the round-trip itself. Returns [`EditorError::InvalidArgument`] when
    /// project-host answers but reports error-severity diagnostics, which is how
    /// a refused or unusable checkpoint arrives.
    #[instrument(skip(self), fields(source_path))]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn restore(&self, source_path: &str) -> EditorResult<SourceSessionResult> {
        let result = self
            .source_session
            .lifecycle(source_path, SourceSessionCommand::Open, 0)
            .await?;
        ensure_source_session_success(source_path, "restore recovery", result)
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn run_autosave_loop(self, source_path: String, interval: Duration, debounce: Duration) {
        self.run_autosave_loop_with(
            source_path,
            interval,
            debounce,
            |controller, path, revision| {
                Box::pin(async move { controller.save_recovery(&path, revision).await })
            },
        )
        .await;
    }

    /// The autosave state machine, parameterized on the checkpoint call so
    /// tests can characterize ordering/retry/teardown without project-host.
    /// Production passes [`Self::save_recovery`].
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn run_autosave_loop_with<S>(
        &self,
        source_path: String,
        interval: Duration,
        debounce: Duration,
        mut save: S,
    ) where
        S: for<'a> FnMut(
            &'a Self,
            String,
            u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = EditorResult<SourceSessionResult>> + 'a>,
        >,
    {
        let mut close_rx = self.close_rx.clone();
        loop {
            tokio::select! {
                _ = close_rx.changed() => return,
                () = Delay::new(interval) => {}
            }
            let Some(revision) = self
                .wait_for_debounce(&source_path, debounce, &mut close_rx)
                .await
            else {
                return;
            };

            let result = tokio::select! {
                _ = close_rx.changed() => return,
                result = save(self, source_path.clone(), revision) => result,
            };
            match result {
                Ok(result) => {
                    info!(
                        source_path,
                        revision = result.status.revision,
                        "saved typed source-session recovery checkpoint"
                    );
                    self.finish_autosave_task(&source_path);
                    return;
                }
                Err(error) => {
                    error!(
                        %error,
                        source_path,
                        revision,
                        "failed to save typed source-session recovery checkpoint"
                    );
                }
            }
        }
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn wait_for_debounce(
        &self,
        source_path: &str,
        debounce: Duration,
        close_rx: &mut tokio::sync::watch::Receiver<()>,
    ) -> Option<u64> {
        loop {
            let (remaining, revision) = {
                let autosave = self.autosave.lock().expect("autosave state mutex poisoned");
                let state = autosave.sources.get(source_path)?;
                let last_edit = state.last_edit?;
                let revision = state.revision;
                drop(autosave);
                (debounce.checked_sub(last_edit.elapsed()), revision)
            };
            let Some(remaining) = remaining else {
                return Some(revision);
            };
            tokio::select! {
                _ = close_rx.changed() => return None,
                () = Delay::new(remaining) => {}
            }
        }
    }

    fn finish_autosave_task(&self, source_path: &str) {
        if let Some(state) = self
            .autosave
            .lock()
            .expect("autosave state mutex poisoned")
            .sources
            .get_mut(source_path)
        {
            state.task_running = false;
        }
    }
}

#[derive(Debug, Default)]
struct AutosaveState {
    sources: BTreeMap<String, AutosaveSourceState>,
}

#[derive(Debug, Default)]
struct AutosaveSourceState {
    revision: u64,
    last_edit: Option<Instant>,
    task_running: bool,
}

pub(crate) fn install_recovery_slot(
    cx: &mut App,
    session: EditorAttachSession,
    fence: crate::controller_set::ControllerFence,
) {
    let session_slug = session.session_slug.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "source-session-recovery-install",
        move || async move { EditorRecoveryController::connect_attached(&session).await },
        move |cx, result| match result {
            Ok(controller) => {
                if !crate::controller_set::complete_recovery(cx, fence, controller) {
                    return;
                }
                info!(session = %session_slug, "installed typed source-session recovery controller");
            }
            Err(error) => {
                crate::controller_set::fail_controller(cx, fence, error.to_string());
                error!(%error, session = %session_slug, "failed to install typed source-session recovery controller");
            }
        },
    );
}

/// Records one reflected source change for the debounced recovery checkpoint.
pub fn note_source_session_dirty(cx: &mut App, source_path: impl Into<String>, revision: u64) {
    if let Ok(attached) = crate::controller_set::recovery_controller(cx) {
        attached
            .controller
            .note_dirty(source_path.into(), revision, cx);
    }
}

pub fn note_source_session_saved(cx: &mut App, source_path: &str) {
    if let Ok(attached) = crate::controller_set::recovery_controller(cx) {
        attached.controller.note_saved(source_path);
    }
}

fn ensure_source_session_success(
    source_path: &str,
    operation: &'static str,
    result: SourceSessionResult,
) -> EditorResult<SourceSessionResult> {
    let errors = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(result)
    } else {
        Err(EditorError::InvalidArgument(format!(
            "failed to {operation} for `{source_path}`: {}",
            errors.join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_host::test_project_host_client;

    fn test_controller() -> EditorRecoveryController {
        EditorRecoveryController::new(ReflectedPrefabEditSession::new(test_project_host_client()))
    }

    fn recovery_saved(revision: u64) -> SourceSessionResult {
        SourceSessionResult {
            status: az_proto_project::vnext::SourceSessionStatus {
                open: true,
                revision,
                dirty: false,
                undo_depth: 0,
                redo_depth: 0,
            },
            snapshot: None,
            diagnostics: Vec::new(),
        }
    }

    fn task_running(controller: &EditorRecoveryController, source_path: &str) -> bool {
        controller
            .autosave
            .lock()
            .expect("autosave state mutex poisoned")
            .sources
            .get(source_path)
            .is_some_and(|state| state.task_running)
    }

    #[test]
    fn dirty_notes_spawn_one_task_until_it_finishes() {
        let controller = test_controller();

        // First dirty note spawns the loop; continued editing while it runs
        // only refreshes the debounced revision.
        assert!(controller.mark_dirty_and_should_spawn("prefab.ron", 3));
        assert!(!controller.mark_dirty_and_should_spawn("prefab.ron", 4));
        assert_eq!(
            controller.autosave.lock().unwrap().sources["prefab.ron"].revision,
            4
        );

        // A finished (or saved-away) source becomes spawnable again.
        controller.finish_autosave_task("prefab.ron");
        assert!(controller.mark_dirty_and_should_spawn("prefab.ron", 5));

        // note_saved removes the entry entirely; the next note starts fresh.
        controller.note_saved("prefab.ron");
        assert!(controller.mark_dirty_and_should_spawn("prefab.ron", 6));
    }

    #[test]
    fn wait_for_debounce_returns_the_latest_revision_once_elapsed() {
        let controller = test_controller();
        let mut close_rx = controller.close_rx.clone();
        controller.mark_dirty_and_should_spawn("prefab.ron", 3);

        // Simulate an edit landing mid-debounce: newer revision, timer reset.
        {
            let mut autosave = controller.autosave.lock().unwrap();
            let entry = autosave.sources.get_mut("prefab.ron").unwrap();
            entry.revision = 9;
            entry.last_edit = Some(Instant::now());
            drop(autosave);
        }

        let revision = futures::executor::block_on(controller.wait_for_debounce(
            "prefab.ron",
            Duration::from_millis(30),
            &mut close_rx,
        ))
        .expect("debounce must resolve for a live source");

        assert_eq!(revision, 9, "the checkpoint must carry the latest edit");
    }

    #[test]
    fn command_copies_do_not_keep_the_slot_owned_autosave_cancellation_alive() {
        let controller = test_controller();
        let command_copy = controller.clone();
        let mut worker_shutdown = command_copy.close_rx.clone();

        drop(controller);

        assert!(
            futures::executor::block_on(worker_shutdown.changed()).is_err(),
            "dropping the aggregate-held controller must cancel delayed autosave work even when a command copy exists"
        );
        drop(command_copy);
    }

    #[test]
    fn note_saved_during_flight_ends_the_loop_without_saving() {
        let controller = test_controller();
        controller.mark_dirty_and_should_spawn("prefab.ron", 3);
        controller.note_saved("prefab.ron");

        futures::executor::block_on(controller.run_autosave_loop_with(
            "prefab.ron".to_string(),
            Duration::from_millis(1),
            Duration::from_millis(1),
            |_controller, _path, _revision| {
                Box::pin(async move {
                    unreachable!("no checkpoint may run after note_saved removed the source")
                })
            },
        ));

        assert!(
            !controller
                .autosave
                .lock()
                .unwrap()
                .sources
                .contains_key("prefab.ron"),
            "note_saved must keep the source removed"
        );
    }

    #[test]
    fn autosave_retries_failed_checkpoints_then_finishes_on_success() {
        let controller = test_controller();
        controller.mark_dirty_and_should_spawn("prefab.ron", 7);

        let attempts = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&attempts);
        futures::executor::block_on(controller.run_autosave_loop_with(
            "prefab.ron".to_string(),
            Duration::from_millis(1),
            Duration::from_millis(1),
            move |_controller, _path, revision| {
                let attempts = Arc::clone(&recorded);
                Box::pin(async move {
                    attempts.lock().unwrap().push(revision);
                    if attempts.lock().unwrap().len() < 3 {
                        Err(EditorError::InvalidArgument("checkpoint failed".to_owned()))
                    } else {
                        Ok(recovery_saved(revision))
                    }
                })
            },
        ));

        assert_eq!(*attempts.lock().unwrap(), vec![7, 7, 7]);
        assert!(
            !task_running(&controller, "prefab.ron"),
            "a successful checkpoint must finish the autosave task"
        );
    }

    #[test]
    fn finish_autosave_task_tolerates_a_removed_source() {
        let controller = test_controller();
        controller.mark_dirty_and_should_spawn("prefab.ron", 1);
        controller.note_saved("prefab.ron");

        // The in-flight loop can observe removal between debounce and save;
        // finishing must be a no-op rather than a panic or resurrect.
        controller.finish_autosave_task("prefab.ron");

        assert!(
            !controller
                .autosave
                .lock()
                .unwrap()
                .sources
                .contains_key("prefab.ron")
        );
    }
}
