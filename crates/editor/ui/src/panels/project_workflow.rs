//! Project workflow data contract shared with editor-core.
//!
//! This module holds the neutral data types editor-core publishes as a global
//! to drive the project launcher UI, plus the stateless progress-bar renderer.
//! It deliberately depends only on gpui + theme + plain data — no project,
//! daemon, or bevy code — so the architecture-boundary tests stay meaningful.

use gpui::{
    Global, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder as _, px, relative,
};
use gpui_component::{h_flex, v_flex};

/// Project workflow state supplied by editor-core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub mode: Mode,
    pub phase: Phase,
    pub operation: Option<Operation>,
    pub project_root: Option<String>,
    pub message: Option<String>,
    pub status_error: Option<String>,
    pub next_steps: Vec<NextStep>,
    pub attached_session: Option<AttachedSession>,
    /// Live open-project progress, when an open operation is streaming events.
    pub progress: Option<Progress>,
}

/// A snapshot of streamed open-project progress for the UI progress bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// The active open-project phase.
    pub phase: OpenPhase,
    /// Raw completed units for the active phase.
    pub phase_done: u64,
    /// Raw total units for the active phase; `None` means not yet known.
    pub phase_total: Option<u64>,
    /// Latest human-readable status message.
    pub message: String,
}

impl Progress {
    /// Current phase completion percentage, only when the phase has a real
    /// denominator. Open-ended build progress intentionally returns `None`
    /// instead of manufacturing an aggregate percent.
    #[must_use]
    pub fn phase_percent(&self) -> Option<u32> {
        let total = self.phase_total?;
        if total == 0 {
            return None;
        }
        let scaled = (u128::from(self.phase_done.min(total)) * 100) / u128::from(total);
        Some(u32::try_from(scaled.min(100)).unwrap_or(100))
    }
}

/// Editor-side mirror of the daemon open-project phase taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenPhase {
    ResolvePlan,
    Build,
    StartServices,
    Attach,
    LoadCatalogs,
}

impl OpenPhase {
    /// Stable human label for the phase.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ResolvePlan => "Resolving",
            Self::Build => "Building",
            Self::StartServices => "Starting services",
            Self::Attach => "Attaching",
            Self::LoadCatalogs => "Loading catalogs",
        }
    }
}

impl Default for Status {
    fn default() -> Self {
        Self::idle()
    }
}

impl Status {
    #[must_use]
    pub const fn idle() -> Self {
        Self {
            mode: Mode::Overview,
            phase: Phase::Idle,
            operation: None,
            project_root: None,
            message: None,
            status_error: None,
            next_steps: Vec::new(),
            attached_session: None,
            progress: None,
        }
    }

    #[must_use]
    pub const fn new_project_requested() -> Self {
        Self {
            mode: Mode::NewProject,
            phase: Phase::Idle,
            operation: None,
            project_root: None,
            message: None,
            status_error: None,
            next_steps: Vec::new(),
            attached_session: None,
            progress: None,
        }
    }

    #[must_use]
    pub const fn existing_project_requested() -> Self {
        Self {
            mode: Mode::ExistingProject,
            phase: Phase::Idle,
            operation: None,
            project_root: None,
            message: None,
            status_error: None,
            next_steps: Vec::new(),
            attached_session: None,
            progress: None,
        }
    }

    #[must_use]
    pub fn running(operation: Operation, project_root: impl Into<String>) -> Self {
        Self {
            mode: operation.mode(),
            phase: Phase::Running,
            operation: Some(operation),
            project_root: Some(project_root.into()),
            message: None,
            status_error: None,
            next_steps: Vec::new(),
            attached_session: None,
            progress: None,
        }
    }

    /// A running open operation carrying a live progress snapshot. The phase
    /// stays `Running` (so the in-progress gate keeps inputs disabled) while the
    /// progress bar renders the streamed aggregate completion.
    #[must_use]
    pub fn running_with_progress(
        operation: Operation,
        project_root: impl Into<String>,
        progress: Progress,
    ) -> Self {
        Self {
            mode: operation.mode(),
            phase: Phase::Running,
            operation: Some(operation),
            project_root: Some(project_root.into()),
            message: Some(progress.message.clone()),
            status_error: None,
            next_steps: Vec::new(),
            attached_session: None,
            progress: Some(progress),
        }
    }

    #[must_use]
    pub fn succeeded(
        operation: Operation,
        project_root: impl Into<String>,
        message: impl Into<String>,
        next_steps: Vec<NextStep>,
    ) -> Self {
        Self {
            mode: operation.mode(),
            phase: Phase::Succeeded,
            operation: Some(operation),
            project_root: Some(project_root.into()),
            message: Some(message.into()),
            status_error: None,
            next_steps,
            attached_session: None,
            progress: None,
        }
    }

    #[must_use]
    pub fn succeeded_with_attached_session(
        operation: Operation,
        project_root: impl Into<String>,
        message: impl Into<String>,
        next_steps: Vec<NextStep>,
        attached_session: AttachedSession,
    ) -> Self {
        Self {
            mode: operation.mode(),
            phase: Phase::Succeeded,
            operation: Some(operation),
            project_root: Some(project_root.into()),
            message: Some(message.into()),
            status_error: None,
            next_steps,
            attached_session: Some(attached_session),
            progress: None,
        }
    }

    #[must_use]
    pub fn failed(
        operation: Operation,
        project_root: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self::failed_with_next_steps(operation, project_root, error, Vec::new())
    }

    #[must_use]
    pub fn failed_with_next_steps(
        operation: Operation,
        project_root: impl Into<String>,
        error: impl Into<String>,
        next_steps: Vec<NextStep>,
    ) -> Self {
        Self {
            mode: operation.mode(),
            phase: Phase::Failed,
            operation: Some(operation),
            project_root: Some(project_root.into()),
            message: None,
            status_error: Some(error.into()),
            next_steps,
            attached_session: None,
            progress: None,
        }
    }
}

impl Global for Status {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Overview,
    NewProject,
    ExistingProject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    CreateProject,
    InitializeProject,
    EnsureProjectSession,
    PrepareProjectServices,
    OpenEditorSession,
}

impl Operation {
    const fn mode(self) -> Mode {
        match self {
            Self::CreateProject => Mode::NewProject,
            Self::InitializeProject
            | Self::EnsureProjectSession
            | Self::PrepareProjectServices
            | Self::OpenEditorSession => Mode::ExistingProject,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextStep {
    pub kind: NextStepKind,
    pub label: String,
    pub command: String,
}

impl NextStep {
    #[must_use]
    pub fn new(kind: NextStepKind, label: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            kind,
            label: label.into(),
            command: command.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextStepKind {
    CreateLoreRepository,
    CommitProjectWorkflow,
    CreateMainSession,
    OpenEditorSession,
    InspectSessionServices,
    RunProject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedSession {
    pub project_id: String,
    pub session_slug: String,
    pub running_service_names: Vec<String>,
}

impl AttachedSession {
    #[must_use]
    pub fn new(
        project_id: impl Into<String>,
        session_slug: impl Into<String>,
        running_service_names: Vec<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            session_slug: session_slug.into(),
            running_service_names,
        }
    }
}

/// Render the streamed open-project progress as a determinate bar plus a
/// phase/percent sub-label. Stateless: it reads only the supplied snapshot.
#[must_use]
pub fn render_progress(
    progress: &Progress,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let phase_percent = progress.phase_percent();
    #[allow(
        clippy::cast_precision_loss,
        reason = "phase_percent is bounded to 0..=100 before conversion"
    )]
    let fraction = phase_percent.map(|percent| percent as f32 / 100.0);
    let detail = progress.phase_total.map_or_else(
        || {
            if progress.phase_done > 0 {
                format!(
                    "{}: {} unit(s)",
                    progress.phase.label(),
                    progress.phase_done
                )
            } else {
                format!("{}: waiting for output", progress.phase.label())
            }
        },
        |total| {
            format!(
                "{}: {}/{}",
                progress.phase.label(),
                progress.phase_done,
                total
            )
        },
    );

    v_flex()
        .gap_1()
        .child(
            div()
                .w_full()
                .h(px(8.0))
                .rounded(px(4.0))
                .overflow_hidden()
                .bg(theme.muted)
                .when_some(fraction, |this, fraction| {
                    this.child(div().h_full().w(relative(fraction)).bg(theme.primary))
                }),
        )
        .child(
            h_flex()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(detail),
                )
                .child(
                    div()
                        .text_xs()
                        .font_family("monospace")
                        .text_color(theme.muted_foreground)
                        .child(
                            phase_percent
                                .map_or_else(|| "live".to_owned(), |percent| format!("{percent}%")),
                        ),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_track_mode_phase_and_operation() {
        let project_root = std::env::temp_dir()
            .join("azoth-project-workflow-sample")
            .to_string_lossy()
            .into_owned();
        let running = Status::running(Operation::CreateProject, &project_root);
        assert_eq!(running.mode, Mode::NewProject);
        assert_eq!(running.phase, Phase::Running);
        assert_eq!(running.operation, Some(Operation::CreateProject));

        let failed = Status::failed(
            Operation::InitializeProject,
            &project_root,
            "missing Cargo.toml",
        );
        assert_eq!(failed.mode, Mode::ExistingProject);
        assert_eq!(failed.phase, Phase::Failed);
        assert_eq!(failed.status_error.as_deref(), Some("missing Cargo.toml"));

        let succeeded = Status::succeeded(
            Operation::CreateProject,
            &project_root,
            "created",
            vec![NextStep::new(
                NextStepKind::OpenEditorSession,
                "Open editor session",
                format!("azoth editor --session main --project {project_root}"),
            )],
        );
        assert_eq!(succeeded.phase, Phase::Succeeded);
        assert_eq!(succeeded.next_steps.len(), 1);
        assert_eq!(
            succeeded.next_steps[0].kind,
            NextStepKind::OpenEditorSession
        );
        assert_eq!(succeeded.next_steps[0].label, "Open editor session");

        let attached = Status::succeeded_with_attached_session(
            Operation::OpenEditorSession,
            &project_root,
            "opened",
            Vec::new(),
            AttachedSession::new(
                "local.sample_game",
                "main",
                vec!["project-host".to_string(), "asset-processor".to_string()],
            ),
        );
        assert_eq!(attached.mode, Mode::ExistingProject);
        assert_eq!(
            attached
                .attached_session
                .as_ref()
                .map(|session| session.project_id.as_str()),
            Some("local.sample_game")
        );
        assert_eq!(
            attached
                .attached_session
                .as_ref()
                .map(|session| session.running_service_names.len()),
            Some(2)
        );
    }

    #[test]
    fn progress_phase_percent_requires_known_total() {
        let unknown = Progress {
            phase: OpenPhase::Build,
            phase_done: 17,
            phase_total: None,
            message: "compiling az-editor".to_string(),
        };
        assert_eq!(unknown.phase_percent(), None);

        let known = Progress {
            phase_done: 21,
            phase_total: Some(50),
            ..unknown
        };
        assert_eq!(known.phase_percent(), Some(42));
    }
}
