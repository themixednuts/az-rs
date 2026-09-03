//! Status vocabulary shared by every editor surface.
//!
//! One owner for the state → label and state → tone tables. Panels, the Aether
//! renderer, and editor-core projections read their words and severity from
//! here, so a session, runtime, asset, or build state reads the same wherever
//! it is drawn. Domain phrasing that composes these words (build status lines,
//! health messages, activity summaries) stays with its family.

use gpui::Hsla;
use gpui_component::theme::Theme;

use crate::panels::{
    AssetBrowserEntryStatus, AssetBrowserJobStatus, EditorRuntimeStateData, EditorSessionStateData,
    GraphBuildJobStatusData, GraphBuildSourceStatusData, SessionProcessStateData,
    SessionServiceRoleData,
};
use crate::project_build::EditorProjectBuildPhase;

/// Severity a status carries when it is rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusTone {
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

impl StatusTone {
    /// Theme token this tone resolves to.
    #[must_use]
    pub fn color(self, theme: &Theme) -> Hsla {
        match self {
            Self::Neutral => theme.muted_foreground,
            Self::Info => theme.info,
            Self::Success => theme.success,
            Self::Warning => theme.warning,
            Self::Danger => theme.danger,
        }
    }
}

/// Health a project-owned service reports for itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ServiceHealthStateData {
    #[default]
    Unknown,
    Starting,
    Ready,
    Busy,
    Degraded,
    Stopping,
    Failed,
}

impl ServiceHealthStateData {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Busy => "busy",
            Self::Degraded => "degraded",
            Self::Stopping => "stopping",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub const fn tone(self) -> StatusTone {
        match self {
            Self::Unknown | Self::Stopping => StatusTone::Neutral,
            Self::Starting | Self::Busy | Self::Degraded => StatusTone::Warning,
            Self::Ready => StatusTone::Success,
            Self::Failed => StatusTone::Danger,
        }
    }

    /// Whether the service can answer requests. Degraded and busy services
    /// still serve; they are slow or partial, not absent.
    #[must_use]
    pub const fn ready(self) -> bool {
        matches!(self, Self::Ready | Self::Busy | Self::Degraded)
    }

    #[must_use]
    pub const fn busy(self) -> bool {
        matches!(self, Self::Busy)
    }

    #[must_use]
    pub const fn degraded(self) -> bool {
        matches!(self, Self::Degraded)
    }
}

impl EditorSessionStateData {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Active => "active",
            Self::FailedPreserved => "failed preserved",
            Self::Removed => "removed",
        }
    }
}

impl SessionProcessStateData {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Exited => "exited",
            Self::Failed => "failed",
        }
    }
}

impl SessionServiceRoleData {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Editor => "editor",
            Self::Daemon => "daemon",
            Self::SessionSupervisor => "session-supervisor",
            Self::ProjectHost => "project-host",
            Self::AssetProcessor => "asset-processor",
            Self::RuntimeHost => "runtime-host",
            Self::Worker => "worker",
        }
    }
}

impl EditorRuntimeStateData {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unregistered => "unregistered",
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub const fn tone(self) -> StatusTone {
        match self {
            Self::Unregistered | Self::Stopped => StatusTone::Neutral,
            Self::Starting => StatusTone::Warning,
            Self::Running => StatusTone::Success,
            Self::Failed => StatusTone::Danger,
        }
    }
}

impl AssetBrowserJobStatus {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Leased => "leased",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Abandoned => "abandoned",
        }
    }

    #[must_use]
    pub const fn tone(self) -> StatusTone {
        match self {
            Self::Queued | Self::Leased => StatusTone::Neutral,
            Self::Succeeded => StatusTone::Success,
            Self::Failed => StatusTone::Danger,
            Self::Abandoned => StatusTone::Warning,
        }
    }
}

impl AssetBrowserEntryStatus {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Conflicted => "conflicted",
        }
    }

    #[must_use]
    pub const fn tone(self) -> StatusTone {
        match self {
            Self::Clean => StatusTone::Success,
            Self::Added => StatusTone::Info,
            Self::Modified => StatusTone::Warning,
            Self::Deleted | Self::Conflicted => StatusTone::Danger,
        }
    }
}

impl EditorProjectBuildPhase {
    /// Build has no shared label: its user-facing text is phrase composition
    /// (`build_status_line_from_state`), not a single word.
    #[must_use]
    pub const fn tone(self) -> StatusTone {
        match self {
            Self::Idle => StatusTone::Neutral,
            Self::Planning | Self::Running => StatusTone::Warning,
            Self::Planned | Self::Succeeded => StatusTone::Success,
            Self::Failed => StatusTone::Danger,
        }
    }
}

impl GraphBuildSourceStatusData {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Conflicted => "conflicted",
        }
    }
}

impl GraphBuildJobStatusData {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Leased => "leased",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Abandoned => "abandoned",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The predicates replace string parsing of the health label that used to
    /// live in `EditorAssetProcessorActivity::new`; they must classify the same
    /// states it did.
    #[test]
    fn service_health_predicates_match_the_retired_string_parsing() {
        for state in [
            ServiceHealthStateData::Unknown,
            ServiceHealthStateData::Starting,
            ServiceHealthStateData::Ready,
            ServiceHealthStateData::Busy,
            ServiceHealthStateData::Degraded,
            ServiceHealthStateData::Stopping,
            ServiceHealthStateData::Failed,
        ] {
            let label = state.label();
            assert_eq!(state.busy(), label == "busy");
            assert_eq!(state.degraded(), label == "degraded");
            assert_eq!(
                state.ready(),
                matches!(label, "ready" | "busy" | "degraded"),
                "`{label}` readiness diverged from the label vocabulary"
            );
        }
    }

    #[test]
    fn asset_and_graph_families_share_one_vocabulary() {
        assert_eq!(
            AssetBrowserEntryStatus::Conflicted.label(),
            GraphBuildSourceStatusData::Conflicted.label()
        );
        assert_eq!(
            AssetBrowserJobStatus::Succeeded.label(),
            GraphBuildJobStatusData::Succeeded.label()
        );
    }
}
