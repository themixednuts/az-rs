//! Output Log panel.
//!
//! A compact, read-only view over editor/service output events.

use std::sync::Arc;

use gpui::{
    App, Context, FocusHandle, Focusable, Global, IntoElement, ParentElement, Render, Styled,
    Window, div, px, uniform_list,
};
use gpui_component::dock::{Panel, PanelEvent};
use gpui_component::{ActiveTheme, v_flex};

use crate::panels::{LogLevel, kit};

/// Maximum retained output messages. Mirrors the console panel's cap:
/// older entries drop as new ones arrive so a chatty service cannot grow
/// the editor process without bound.
pub const MAX_OUTPUT_LOG_MESSAGES: usize = 1000;

/// A service/editor output event retained by the Output Log.
#[derive(Debug, Clone)]
pub struct OutputLogMessage {
    pub level: LogLevel,
    pub source: String,
    pub message: String,
    pub timestamp: std::time::SystemTime,
}

/// Shared Output Log state rendered by the Output Log panel and Aether tab.
#[derive(Debug, Clone, Default)]
pub struct OutputLogState {
    messages: Vec<Arc<OutputLogMessage>>,
}

impl Global for OutputLogState {}

impl OutputLogState {
    pub fn append_output(
        &mut self,
        level: LogLevel,
        source: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.messages.push(Arc::new(OutputLogMessage {
            level,
            source: source.into(),
            message: message.into(),
            timestamp: std::time::SystemTime::now(),
        }));
        if self.messages.len() > MAX_OUTPUT_LOG_MESSAGES {
            self.messages
                .drain(0..self.messages.len() - MAX_OUTPUT_LOG_MESSAGES);
        }
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    #[must_use]
    pub fn messages(&self) -> &[Arc<OutputLogMessage>] {
        &self.messages
    }

    #[must_use]
    pub fn latest(&self) -> Option<&Arc<OutputLogMessage>> {
        self.messages.last()
    }
}

pub struct OutputLogPanel {
    focus_handle: FocusHandle,
}

impl OutputLogPanel {
    pub const NAME: &'static str = "output-log";

    pub fn init(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Render for OutputLogPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        // Arc'd snapshot: copying the row list is pointer refcounts, not
        // message deep-clones; uniform_list then materializes only the
        // visible range per frame.
        let rows: Arc<Vec<Arc<OutputLogMessage>>> = cx
            .try_global::<OutputLogState>()
            .map(|state| Arc::from(state.messages().to_vec()))
            .unwrap_or_default();

        let mut body = v_flex()
            .flex_1()
            .min_h_0()
            .px(px(12.0))
            .py(px(8.0))
            .bg(theme.input_background())
            .font_family(theme.mono_font_family.clone())
            .text_size(gpui::Rems(0.62));

        if rows.is_empty() {
            body = body.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child("No output yet."),
            );
        } else {
            let list_theme = theme.clone();
            body = body.child(
                uniform_list(
                    "output-log-rows",
                    rows.len(),
                    cx.processor(move |_this, range: std::ops::Range<usize>, _window, _cx| {
                        range
                            .map(|index| output_log_line(&rows[index], &list_theme))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0(),
            );
        }

        v_flex().size_full().bg(theme.sidebar).child(body)
    }
}

impl Focusable for OutputLogPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for OutputLogPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        kit::tab_title(Some("subject"), "Output Log", kit::TabTone::Default)
    }
}

impl gpui::EventEmitter<PanelEvent> for OutputLogPanel {}

fn output_log_line(message: &OutputLogMessage, theme: &gpui_component::theme::Theme) -> gpui::Div {
    let color = match message.level {
        LogLevel::Trace | LogLevel::Debug => theme.muted_foreground,
        LogLevel::Info => theme.foreground,
        LogLevel::Warn => theme.warning,
        LogLevel::Error => theme.danger,
    };

    div()
        .min_w_full()
        .flex_none()
        .whitespace_nowrap()
        .py(px(1.0))
        .text_color(color)
        .child(message.message.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_output_caps_messages_dropping_oldest() {
        let mut state = OutputLogState::default();
        let total = MAX_OUTPUT_LOG_MESSAGES + 25;
        for index in 0..total {
            state.append_output(LogLevel::Info, "test", format!("line {index}"));
        }

        assert_eq!(state.messages().len(), MAX_OUTPUT_LOG_MESSAGES);
        assert_eq!(state.messages()[0].message, "line 25");
        assert_eq!(
            state.messages().last().unwrap().message,
            format!("line {}", total - 1)
        );

        state.clear();
        assert!(state.messages().is_empty());
    }
}
