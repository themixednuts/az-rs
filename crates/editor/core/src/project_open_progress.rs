//! Editor-side open-project progress sink.
//!
//! The editor hosts a [`capnp`] callback capability that the daemon drives with
//! live open-project progress during build/start/wait. Each wire event is
//! decoded into an [`OpenProgressUpdate`] and forwarded over an unbounded
//! channel back to the GPUI main thread, where it is translated into a
//! `running_with_progress` workflow status and rendered as a real progress bar.
//!
//! The sink server itself runs on the same single-threaded `LocalSet` that
//! hosts the daemon RPC connection (capnp capabilities are `!Send`), so the
//! channel sender is the only cross-thread hop.

use az_editor_ui::panels::project_workflow;
use az_proto_daemon::{OpenProjectPhase, ProjectOpenProgressEvent, daemon_capnp};
use tokio::sync::mpsc::UnboundedSender;

/// A decoded open-project progress update destined for the UI thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenProgressUpdate {
    /// Monotone sequence number; the UI drops any out-of-order/stale update.
    pub seq: u64,
    /// The active phase.
    pub phase: project_workflow::OpenPhase,
    /// Aggregate completion in basis points (`0..=10000`).
    pub done_bp: u32,
    /// Raw completed units for the active phase.
    pub phase_done: u64,
    /// Raw total units for the active phase; `None` means not yet known.
    pub phase_total: Option<u64>,
    /// Latest human-readable status message.
    pub message: String,
}

impl OpenProgressUpdate {
    /// Project this update into the UI progress snapshot.
    #[must_use]
    pub fn to_progress_data(&self) -> project_workflow::Progress {
        project_workflow::Progress {
            phase: self.phase,
            phase_done: self.phase_done,
            phase_total: self.phase_total,
            message: self.message.clone(),
        }
    }

    /// Synthesize an editor-driven update for a phase that runs AFTER the daemon
    /// RPC returns (Attach, `LoadCatalogs`). `seq` seeds from the last daemon seq
    /// to stay monotone; `done_bp` credits this phase's full weight.
    #[must_use]
    pub fn editor_phase(
        seq: u64,
        phase: project_workflow::OpenPhase,
        done_bp: u32,
        message: impl Into<String>,
    ) -> Self {
        Self {
            seq,
            phase,
            done_bp,
            phase_done: 1,
            phase_total: Some(1),
            message: message.into(),
        }
    }
}

/// Map the wire phase enum onto the UI phase enum.
#[must_use]
pub const fn ui_phase(phase: OpenProjectPhase) -> project_workflow::OpenPhase {
    match phase {
        OpenProjectPhase::ResolvePlan => project_workflow::OpenPhase::ResolvePlan,
        OpenProjectPhase::Build => project_workflow::OpenPhase::Build,
        OpenProjectPhase::StartServices => project_workflow::OpenPhase::StartServices,
        OpenProjectPhase::Attach => project_workflow::OpenPhase::Attach,
        OpenProjectPhase::LoadCatalogs => project_workflow::OpenPhase::LoadCatalogs,
    }
}

/// Aggregate basis points credited once `Attach` completes.
///
/// The daemon streams up to `StartServices` complete (`ResolvePlan` 300 +
/// Build 8200 + `StartServices` 800 = 9300); the editor advances the final two
/// phases locally against the same frozen weights: Attach (+400) -> 9700,
/// `LoadCatalogs` (+300) -> 10000.
pub const START_SERVICES_DONE_BP: u32 = 9_300;
pub const ATTACH_DONE_BP: u32 = 9_700;
/// Aggregate basis points once catalogs finish loading: full scale.
pub const LOAD_CATALOGS_DONE_BP: u32 = 10_000;

/// The capnp callback server the editor passes to the daemon. Every `update`
/// decodes the wire event and forwards it to the UI thread; a closed receiver
/// (the open task ended) is non-fatal.
pub struct EditorProgressSink {
    tx: UnboundedSender<OpenProgressUpdate>,
}

impl EditorProgressSink {
    /// Build a sink forwarding decoded updates over `tx`.
    #[must_use]
    pub const fn new(tx: UnboundedSender<OpenProgressUpdate>) -> Self {
        Self { tx }
    }

    /// Wrap this sink into a capnp client capability.
    #[must_use]
    pub fn into_client(self) -> daemon_capnp::project_open_progress_sink::Client {
        capnp_rpc::new_client(self)
    }
}

impl daemon_capnp::project_open_progress_sink::Server for EditorProgressSink {
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn update(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::project_open_progress_sink::UpdateParams,
        _results: daemon_capnp::project_open_progress_sink::UpdateResults,
    ) -> Result<(), capnp::Error> {
        let event = ProjectOpenProgressEvent::from_capnp(params.get()?.get_event()?)?;
        let update = OpenProgressUpdate {
            seq: event.seq,
            phase: ui_phase(event.phase),
            done_bp: event.done_bp,
            phase_done: event.phase_done,
            // total == 0 means the phase total is not yet known.
            phase_total: (event.phase_total != 0).then_some(event.phase_total),
            message: event.message,
        };
        // A dropped receiver (open task finished/cancelled) is non-fatal.
        let _ = self.tx.send(update);
        Ok(())
    }
}

/// Progress capability used by callers that intentionally discard updates.
pub struct NoopProjectOpenProgressSink;

impl NoopProjectOpenProgressSink {
    #[must_use]
    pub fn into_client(self) -> daemon_capnp::project_open_progress_sink::Client {
        capnp_rpc::new_client(self)
    }
}

impl daemon_capnp::project_open_progress_sink::Server for NoopProjectOpenProgressSink {
    fn update(
        self: capnp::capability::Rc<Self>,
        _params: daemon_capnp::project_open_progress_sink::UpdateParams,
        _results: daemon_capnp::project_open_progress_sink::UpdateResults,
    ) -> impl std::future::Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_phase_credits_full_phase() {
        let attach = OpenProgressUpdate::editor_phase(
            10,
            project_workflow::OpenPhase::Attach,
            ATTACH_DONE_BP,
            "attaching",
        );
        assert_eq!(attach.seq, 10);
        assert_eq!(attach.done_bp, 9_700);
        assert_eq!(attach.phase_total, Some(1));
        assert_eq!(attach.phase, project_workflow::OpenPhase::Attach);
    }

    #[test]
    fn ui_phase_round_trips_all_phases() {
        for (wire, ui) in [
            (
                OpenProjectPhase::ResolvePlan,
                project_workflow::OpenPhase::ResolvePlan,
            ),
            (OpenProjectPhase::Build, project_workflow::OpenPhase::Build),
            (
                OpenProjectPhase::StartServices,
                project_workflow::OpenPhase::StartServices,
            ),
            (
                OpenProjectPhase::Attach,
                project_workflow::OpenPhase::Attach,
            ),
            (
                OpenProjectPhase::LoadCatalogs,
                project_workflow::OpenPhase::LoadCatalogs,
            ),
        ] {
            assert_eq!(ui_phase(wire), ui);
        }
    }
}
