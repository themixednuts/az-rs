//! Project-host connection state.
//!
//! The editor shell can switch into a loaded-project workspace *before* the
//! project-host services finish building/attaching. While that open workflow is
//! still running, project-host-dependent panels (inspector, outliner,
//! schema-driven authoring UI, the in-process viewport scene) must render a
//! disabled "Connecting to project services…" placeholder instead of stale or
//! empty live data. Panels that do not need project-host (menus, filesystem/file
//! tree, output log, settings, window chrome) stay fully usable.
//!
//! If the open workflow fails (a per-project service fails to start, attach
//! validation fails, or the daemon RPC errors), the state moves to `Failed`
//! with the failure reason instead of getting stuck on `Connecting` forever.
//! Project-host-dependent panels render that failure distinctly, with the
//! reason and a way back to the project launcher
//! ([`crate::actions::BackToProjectLauncher`]).
//!
//! This is data-only: editor-core owns the lifecycle and sets the global; the UI
//! crate only reads it and renders the gate.

use gpui::{
    AnyElement, App, Global, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable as _, StyledExt, v_flex};

/// Whether the loaded-project workspace is still connecting to project-host
/// services, failed to connect, or has a live attached session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorProjectConnectionStateData {
    /// The shell switched into the workspace early; project-host services are
    /// still building/starting/attaching. Project-host panels are gated.
    Connecting,
    /// The open workflow failed (service start, attach validation, or the
    /// daemon RPC itself). Project-host panels render the failure and a way
    /// back to the project launcher instead of gating forever.
    Failed { reason: String },
    /// Project-host is attached; live data is flowing. Panels render normally.
    Connected,
}

impl EditorProjectConnectionStateData {
    /// True while project-host-dependent panels must show the connecting gate.
    #[must_use]
    pub const fn is_connecting(&self) -> bool {
        matches!(self, Self::Connecting)
    }

    /// True when the open workflow failed.
    #[must_use]
    pub const fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    /// The failure reason, if the open workflow failed.
    #[must_use]
    pub const fn failure_reason(&self) -> Option<&str> {
        match self {
            Self::Failed { reason } => Some(reason.as_str()),
            Self::Connecting | Self::Connected => None,
        }
    }
}

/// GPUI global mirror of the connection state, set by editor-core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorProjectConnectionState {
    pub state: EditorProjectConnectionStateData,
}

impl EditorProjectConnectionState {
    #[must_use]
    pub const fn connecting() -> Self {
        Self {
            state: EditorProjectConnectionStateData::Connecting,
        }
    }

    #[must_use]
    pub const fn connected() -> Self {
        Self {
            state: EditorProjectConnectionStateData::Connected,
        }
    }

    /// The open workflow failed; `reason` is a human-readable description
    /// surfaced verbatim in the gated panels.
    #[must_use]
    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            state: EditorProjectConnectionStateData::Failed {
                reason: reason.into(),
            },
        }
    }
}

impl Global for EditorProjectConnectionState {}

/// True when project-host-dependent panels should render the connecting gate.
///
/// Defaults to connected when no state is published (e.g. a directly-attached
/// editor launch) so live panels are never spuriously disabled.
#[must_use]
pub fn editor_project_host_connecting(cx: &App) -> bool {
    cx.try_global::<EditorProjectConnectionState>()
        .is_some_and(|state| state.state.is_connecting())
}

/// `Some(reason)` when the open workflow failed and project-host-dependent
/// panels should render the failure gate instead of the connecting gate.
#[must_use]
pub fn editor_project_host_failed_reason(cx: &App) -> Option<String> {
    cx.try_global::<EditorProjectConnectionState>()
        .and_then(|state| state.state.failure_reason())
        .map(str::to_string)
}

/// Disabled placeholder rendered by project-host-dependent panels while the
/// workspace is still connecting to project services.
#[must_use]
pub fn render_project_host_connecting_placeholder(panel_title: &str, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(10.0))
        .bg(theme.background)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(40.0))
                .h(px(40.0))
                .rounded(px(10.0))
                .bg(theme.muted)
                .child(
                    Icon::new(IconName::LoaderCircle)
                        .with_size(px(20.0))
                        .text_color(theme.muted_foreground),
                ),
        )
        .when(!panel_title.is_empty(), |this| {
            this.child(
                div()
                    .text_sm()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .child(panel_title.to_string()),
            )
        })
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Connecting to project services…"),
        )
}

/// Disabled placeholder rendered by project-host-dependent panels when the
/// open workflow failed.
///
/// Distinct from [`render_project_host_connecting_placeholder`]: an error
/// tint, the failure reason, and a "Back to Project Launcher" button so the
/// panel is never a dead end.
#[must_use]
pub fn render_project_host_failed_placeholder(
    panel_title: &str,
    reason: &str,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(10.0))
        .p(px(16.0))
        .bg(theme.background)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(40.0))
                .h(px(40.0))
                .rounded(px(10.0))
                .bg(theme.danger.opacity(0.12))
                .child(
                    Icon::new(IconName::CircleX)
                        .with_size(px(20.0))
                        .text_color(theme.danger),
                ),
        )
        .when(!panel_title.is_empty(), |this| {
            this.child(
                div()
                    .text_sm()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .child(panel_title.to_string()),
            )
        })
        .child(
            div()
                .text_xs()
                .font_semibold()
                .text_color(theme.danger)
                .child("Failed to open project"),
        )
        .child(
            div()
                .max_w(px(360.0))
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(reason.to_string()),
        )
        .child(
            div()
                .id("project-host-failed-back-to-launcher")
                .mt(px(4.0))
                .px(px(10.0))
                .py(px(5.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .text_color(theme.foreground)
                .cursor_pointer()
                .text_size(px(11.0))
                .font_semibold()
                .hover(|this| this.bg(theme.secondary_hover))
                .child("Back to Project Launcher")
                .on_click(|_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(crate::actions::BackToProjectLauncher), cx);
                }),
        )
}

/// Renders the appropriate project-host gate, preferring a terminal failure
/// over the transient connecting state. Returns `None` once services are live.
#[must_use]
pub fn render_project_host_connection_placeholder(
    panel_title: &str,
    cx: &App,
) -> Option<AnyElement> {
    if let Some(reason) = editor_project_host_failed_reason(cx) {
        return Some(
            render_project_host_failed_placeholder(panel_title, &reason, cx).into_any_element(),
        );
    }
    editor_project_host_connecting(cx)
        .then(|| render_project_host_connecting_placeholder(panel_title, cx).into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connecting_state_is_connecting_only() {
        let state = EditorProjectConnectionState::connecting();
        assert!(state.state.is_connecting());
        assert!(!state.state.is_failed());
        assert_eq!(state.state.failure_reason(), None);
    }

    #[test]
    fn connected_state_is_neither_connecting_nor_failed() {
        let state = EditorProjectConnectionState::connected();
        assert!(!state.state.is_connecting());
        assert!(!state.state.is_failed());
        assert_eq!(state.state.failure_reason(), None);
    }

    #[test]
    fn failed_state_carries_reason_and_is_not_connecting() {
        let state = EditorProjectConnectionState::failed("asset-processor failed to start");
        assert!(!state.state.is_connecting());
        assert!(state.state.is_failed());
        assert_eq!(
            state.state.failure_reason(),
            Some("asset-processor failed to start")
        );
    }
}
