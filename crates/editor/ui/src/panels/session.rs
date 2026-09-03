//!
//! Session supervision status panel.

use gpui::{
    Context, FocusHandle, Focusable, Global, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::dock::Panel;
use gpui_component::scroll::ScrollableElement;
use gpui_component::theme::Theme;
use gpui_component::{ActiveTheme, StyledExt, h_flex, v_flex};

use crate::actions::SessionServiceLogStream;
use crate::panels::kit;

/// Session status snapshot supplied by the editor shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorSessionStatus {
    pub session_id: String,
    pub project_id: String,
    pub session_slug: String,
    pub project_root: String,
    pub workspace_root: String,
    pub run_dir: String,
    pub state: EditorSessionStateData,
    pub failure_reason: Option<String>,
    pub services_count: usize,
    pub processes: Vec<SessionProcessData>,
}

impl Global for EditorSessionStatus {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorSessionStateData {
    Preparing,
    Active,
    FailedPreserved,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionProcessData {
    pub owner_id: String,
    pub owner_root: String,
    pub service_name: String,
    pub role: SessionServiceRoleData,
    pub run: uuid::Uuid,
    pub state: SessionProcessStateData,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub failure: Option<String>,
    pub structured_log: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionServiceRoleData {
    Unknown,
    Editor,
    Daemon,
    SessionSupervisor,
    ProjectHost,
    AssetProcessor,
    RuntimeHost,
    Worker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionProcessStateData {
    Planned,
    Starting,
    Running,
    Exited,
    Failed,
}

pub struct SessionPanel {
    focus: FocusHandle,
}

impl SessionPanel {
    pub const NAME: &'static str = "session";

    pub fn init(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
        }
    }
}

impl Render for SessionPanel {
    #[allow(
        clippy::too_many_lines,
        reason = "single cohesive session toolbar + body"
    )]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status = cx.try_global::<EditorSessionStatus>().cloned();
        let can_recover = status.as_ref().is_some_and(session_can_recover);

        let theme = theme.clone();
        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .child(
                kit::panel_toolbar(&theme).child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_1()
                        .overflow_x_scrollbar()
                        .child(session_action_button(
                            "session-refresh",
                            "Refresh",
                            crate::actions::RefreshSessionStatus,
                            false,
                            &theme,
                        ))
                        .child(session_action_button(
                            "session-start-services",
                            "Start",
                            crate::actions::StartSessionServices,
                            true,
                            &theme,
                        ))
                        .child(session_action_button(
                            "session-stop-services",
                            "Stop",
                            crate::actions::StopSessionServices,
                            false,
                            &theme,
                        ))
                        .when(can_recover, |this| {
                            this.child(session_action_button(
                                "session-recover",
                                "Recover",
                                crate::actions::RecoverSession,
                                false,
                                &theme,
                            ))
                            .child(session_action_button(
                                "session-force-recover",
                                "Force recover",
                                crate::actions::ForceRecoverSession,
                                false,
                                &theme,
                            ))
                        }),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_2()
                    .child(status.map_or_else(
                        || {
                            kit::empty_state("Session status unavailable", None, &theme)
                                .into_any_element()
                        },
                        |status| render_session_status(&status, cx).into_any_element(),
                    )),
            )
    }
}

fn render_session_status(
    status: &EditorSessionStatus,
    cx: &Context<'_, SessionPanel>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let border = theme.border;
    let foreground = theme.foreground;
    let muted_foreground = theme.muted_foreground;
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .child(session_badge(status.state.label(), &theme))
                .child(div().text_xs().text_color(muted_foreground).child(format!(
                    "{} process(es), {} service descriptor(s)",
                    status.processes.len(),
                    status.services_count
                ))),
        )
        .child(render_session_fields(status, foreground, muted_foreground))
        .when(
            status
                .failure_reason
                .as_ref()
                .is_some_and(|reason| !reason.trim().is_empty()),
            |this| {
                this.child(render_session_failure(
                    status,
                    foreground,
                    muted_foreground,
                    border,
                ))
            },
        )
        .child(render_session_services(
            status,
            foreground,
            muted_foreground,
            border,
            &theme,
        ))
}

fn render_session_fields(
    status: &EditorSessionStatus,
    foreground: gpui::Hsla,
    muted_foreground: gpui::Hsla,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(session_kv(
            "slug",
            &status.session_slug,
            foreground,
            muted_foreground,
        ))
        .child(session_kv(
            "workspace",
            &status.workspace_root,
            foreground,
            muted_foreground,
        ))
}

fn render_session_failure(
    status: &EditorSessionStatus,
    foreground: gpui::Hsla,
    muted_foreground: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement {
    div()
        .w_full()
        .border_1()
        .border_color(border)
        .rounded_sm()
        .p_2()
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(foreground)
                        .child("Failure"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_foreground)
                        .child(status.failure_reason.clone().unwrap_or_default()),
                ),
        )
}

fn render_session_services(
    status: &EditorSessionStatus,
    foreground: gpui::Hsla,
    muted_foreground: gpui::Hsla,
    border: gpui::Hsla,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .w_full()
        .border_1()
        .border_color(border)
        .rounded_sm()
        .p_2()
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(foreground)
                        .child("Services"),
                )
                .children(
                    status
                        .processes
                        .iter()
                        .map(|process| render_process_row(process, theme)),
                )
                .when(status.processes.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(muted_foreground)
                            .child("No service processes recorded"),
                    )
                }),
        )
}

fn session_action_button<A>(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    action: A,
    primary: bool,
    theme: &Theme,
) -> impl IntoElement
where
    A: gpui::Action + Clone + 'static,
{
    let (bg, fg, hover) = if primary {
        (theme.primary, theme.primary_foreground, theme.primary_hover)
    } else {
        (theme.secondary, theme.foreground, theme.secondary_hover)
    };
    div()
        .id(id)
        .flex_none()
        .h(px(23.0))
        .flex()
        .items_center()
        .px_2()
        .rounded(px(5.0))
        .border_1()
        .border_color(theme.border)
        .bg(bg)
        .text_color(fg)
        .hover(move |this| this.bg(hover))
        .cursor_pointer()
        .text_size(px(11.0))
        .whitespace_nowrap()
        .child(label)
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

fn session_badge(label: impl Into<String>, theme: &Theme) -> impl IntoElement {
    div()
        .px_2()
        .py(px(2.0))
        .rounded(px(4.0))
        .bg(theme.secondary)
        .text_size(px(11.0))
        .text_color(theme.foreground)
        .child(label.into())
}

fn session_kv(
    key: &'static str,
    value: &str,
    foreground: gpui::Hsla,
    muted_foreground: gpui::Hsla,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .child(
            div()
                .w_16()
                .text_xs()
                .text_color(muted_foreground)
                .child(key),
        )
        .child(
            div()
                .text_xs()
                .font_family("monospace")
                .text_color(foreground)
                .child(value.to_string()),
        )
}

fn process_row_service_label(process: &SessionProcessData) -> String {
    if process.owner_id.is_empty() {
        process.service_name.clone()
    } else {
        format!("{}:{}", process.owner_id, process.service_name)
    }
}

fn process_row_status(process: &SessionProcessData) -> String {
    let status = process.failure.as_ref().map_or_else(
        || process.state.label().to_string(),
        |failure| format!("{}: {failure}", process.state.label()),
    );
    if process.owner_root.trim().is_empty() {
        status
    } else {
        format!("{status} / owner root {}", process.owner_root)
    }
}

fn process_row_pid(process: &SessionProcessData) -> String {
    process
        .pid
        .map(|pid| format!("pid {pid}"))
        .or_else(|| process.exit_code.map(|code| format!("exit {code}")))
        .unwrap_or_else(|| "-".to_string())
}

fn process_log_action(
    process: &SessionProcessData,
    stream: SessionServiceLogStream,
) -> crate::actions::TailSessionServiceLog {
    crate::actions::TailSessionServiceLog {
        service_name: process.service_name.clone(),
        run: Some(process.run),
        stream,
    }
}

fn render_process_row(process: &SessionProcessData, theme: &Theme) -> impl IntoElement {
    let service_label = process_row_service_label(process);
    let status = process_row_status(process);
    let pid = process_row_pid(process);
    let stdout_action = process_log_action(process, SessionServiceLogStream::Stdout);
    let stderr_action = process_log_action(process, SessionServiceLogStream::Stderr);
    let structured_action = process_log_action(process, SessionServiceLogStream::Structured);

    h_flex()
        .gap_2()
        .py_1()
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .w_32()
                .text_xs()
                .font_family("monospace")
                .text_color(theme.foreground)
                .child(service_label),
        )
        .child(
            div()
                .w_24()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(process.role.label()),
        )
        .child(
            div()
                .w_16()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(process.run.to_string()[..8].to_string()),
        )
        .child(
            div()
                .w_20()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(pid),
        )
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(status),
        )
        .child(session_log_button(
            "out",
            (
                "session-log-out",
                session_log_button_id(
                    &process.service_name,
                    process.run,
                    SessionServiceLogStream::Stdout,
                ),
            ),
            stdout_action,
            theme,
        ))
        .child(session_log_button(
            "err",
            (
                "session-log-err",
                session_log_button_id(
                    &process.service_name,
                    process.run,
                    SessionServiceLogStream::Stderr,
                ),
            ),
            stderr_action,
            theme,
        ))
        .when(!process.structured_log.trim().is_empty(), |this| {
            this.child(session_log_button(
                "str",
                (
                    "session-log-structured",
                    session_log_button_id(
                        &process.service_name,
                        process.run,
                        SessionServiceLogStream::Structured,
                    ),
                ),
                structured_action,
                theme,
            ))
        })
}

fn session_log_button_id(
    service_name: &str,
    run: uuid::Uuid,
    stream: SessionServiceLogStream,
) -> u64 {
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in service_name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    for byte in run.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash ^= session_log_stream_hash(stream);
    hash
}

const fn session_log_stream_hash(stream: SessionServiceLogStream) -> u64 {
    match stream {
        SessionServiceLogStream::Stdout => 0,
        SessionServiceLogStream::Stderr => 0x8000_0000_0000_0000,
        SessionServiceLogStream::Structured => 0x4000_0000_0000_0000,
    }
}

fn session_log_button(
    label: &'static str,
    id: impl Into<gpui::ElementId>,
    action: crate::actions::TailSessionServiceLog,
    theme: &Theme,
) -> impl IntoElement {
    let hover = theme.secondary_hover;
    div()
        .id(id)
        .px_1()
        .py_0p5()
        .rounded(px(4.0))
        .bg(theme.secondary)
        .hover(move |this| this.bg(hover))
        .cursor_pointer()
        .text_size(px(11.0))
        .text_color(theme.muted_foreground)
        .child(label)
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

#[must_use]
pub fn session_can_recover(status: &EditorSessionStatus) -> bool {
    status.state == EditorSessionStateData::FailedPreserved
}

impl Focusable for SessionPanel {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Panel for SessionPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(None, "Session", kit::TabTone::Default)
    }

    fn inner_padding(&self, _cx: &gpui::App) -> bool {
        false
    }
}

impl gpui::EventEmitter<gpui_component::dock::PanelEvent> for SessionPanel {}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> EditorSessionStatus {
        let project_root = std::env::temp_dir().join("azoth-session-panel-project");
        EditorSessionStatus {
            session_id: "018f0c5a-0000-7000-8000-000000000001".to_string(),
            project_id: "local.game".to_string(),
            session_slug: "editor".to_string(),
            project_root: project_root.to_string_lossy().into_owned(),
            workspace_root: project_root.to_string_lossy().into_owned(),
            run_dir: project_root
                .join(".azoth/runs/editor")
                .to_string_lossy()
                .into_owned(),
            state: EditorSessionStateData::Active,
            failure_reason: None,
            services_count: 3,
            processes: Vec::new(),
        }
    }

    #[test]
    fn process_and_role_labels_are_stable() {
        assert_eq!(EditorSessionStateData::Active.label(), "active");
        assert_eq!(SessionServiceRoleData::ProjectHost.label(), "project-host");
        assert_eq!(SessionProcessStateData::Running.label(), "running");
    }

    #[test]
    fn service_log_button_ids_distinguish_streams() {
        let run = uuid::Uuid::from_bytes([7; 16]);
        let stdout = session_log_button_id("project-host", run, SessionServiceLogStream::Stdout);
        let stderr = session_log_button_id("project-host", run, SessionServiceLogStream::Stderr);
        let structured =
            session_log_button_id("project-host", run, SessionServiceLogStream::Structured);

        assert_ne!(stdout, stderr);
        assert_ne!(stdout, structured);
        assert_ne!(stderr, structured);
    }

    #[test]
    fn failed_preserved_session_enables_recovery() {
        let mut status = status();
        status.state = EditorSessionStateData::FailedPreserved;

        assert!(session_can_recover(&status));
    }
}
