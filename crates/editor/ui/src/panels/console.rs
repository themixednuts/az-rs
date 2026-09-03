//! Console Panel
//!
//! Command input and log output window.
//! Inspired by Lumberyard's Console.

use gpui::{
    App, AppContext, ClipboardItem, Context, Entity, FocusHandle, Focusable, Global, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement,
    Render, StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle,
    Window, div, prelude::FluentBuilder, px, uniform_list,
};
use gpui_component::dock::Panel;
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex,
};

use crate::panels::kit;
use std::sync::Arc;

/// Console panel
///
/// Displays log messages and accepts command input.
pub struct Console {
    input: Entity<InputState>,
    filter_input: Entity<InputState>,
    input_cache: String,
    history_cursor: Option<usize>,
    copy_feedback_visible: bool,
    copy_epoch: usize,
    copy_feedback_task: Option<Task<()>>,
    log_scroll: UniformListScrollHandle,
    last_visible_count: usize,
    stick_to_bottom: bool,
    _subscriptions: Vec<Subscription>,
}

/// Which severities the console currently shows. Toggled from the filter
/// toolbar; all on by default.
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one independent visibility toggle per severity"
)]
struct ConsoleLevelFilter {
    error: bool,
    warn: bool,
    info: bool,
    debug: bool,
}

impl Default for ConsoleLevelFilter {
    fn default() -> Self {
        Self {
            error: true,
            warn: true,
            info: true,
            debug: true,
        }
    }
}

impl ConsoleLevelFilter {
    const fn shows(self, level: LogLevel) -> bool {
        match level {
            LogLevel::Error => self.error,
            LogLevel::Warn => self.warn,
            LogLevel::Info => self.info,
            LogLevel::Debug | LogLevel::Trace => self.debug,
        }
    }
}

/// Tallies per-severity message counts for the filter toolbar.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleLevelCounts {
    pub error: usize,
    pub warn: usize,
    pub info: usize,
    pub debug: usize,
}

impl ConsoleLevelCounts {
    #[must_use]
    pub fn tally(messages: &[Arc<LogMessage>]) -> Self {
        let mut counts = Self::default();
        for message in messages {
            match message.level {
                LogLevel::Error => counts.error += 1,
                LogLevel::Warn => counts.warn += 1,
                LogLevel::Info => counts.info += 1,
                LogLevel::Debug | LogLevel::Trace => counts.debug += 1,
            }
        }
        counts
    }

    #[must_use]
    pub const fn total(self) -> usize {
        self.error + self.warn + self.info + self.debug
    }

    #[must_use]
    pub const fn info_like(self) -> usize {
        self.info + self.debug
    }
}

/// Log message severity
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// A log message in the console
#[derive(Debug, Clone)]
pub struct LogMessage {
    pub level: LogLevel,
    pub source: String,
    pub message: String,
    pub timestamp: std::time::SystemTime,
    search_text: String,
}

impl LogMessage {
    fn new(level: LogLevel, source: impl Into<String>, message: impl Into<String>) -> Self {
        let source = source.into();
        let message = message.into();
        let search_text = console_search_text(&source, &message);
        Self {
            level,
            source,
            message,
            timestamp: std::time::SystemTime::now(),
            search_text,
        }
    }

    #[must_use]
    pub fn matches_query(&self, query: &str) -> bool {
        let query = normalize_console_query(query);
        if query.is_empty() {
            return true;
        }
        self.search_text.contains(&query)
    }
}

/// Shared console state rendered by the console panel.
#[derive(Debug, Clone)]
pub struct ConsoleState {
    messages: Vec<Arc<LogMessage>>,
    filtered_indices: Vec<usize>,
    history: Vec<String>,
    max_messages: usize,
    level_counts: ConsoleLevelCounts,
    level_filter: ConsoleLevelFilter,
    filter_query: String,
}

/// Maximum retained submitted commands. Oldest drop as new ones arrive;
/// recall only reaches back this far.
pub const MAX_COMMAND_HISTORY: usize = 100;

impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            filtered_indices: Vec::new(),
            history: Vec::new(),
            max_messages: 1000,
            level_counts: ConsoleLevelCounts::default(),
            level_filter: ConsoleLevelFilter::default(),
            filter_query: String::new(),
        }
    }
}

impl Global for ConsoleState {}

impl ConsoleState {
    /// Add a log message.
    pub fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        self.log_from_source(level, "editor", message);
    }

    /// Add a log message with a concrete source.
    pub fn log_from_source(
        &mut self,
        level: LogLevel,
        source: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.messages
            .push(Arc::new(LogMessage::new(level, source, message)));

        if self.messages.len() > self.max_messages {
            self.messages
                .drain(0..self.messages.len() - self.max_messages);
            self.rebuild_indexes();
            return;
        }

        let index = self.messages.len() - 1;
        self.count_level(level);
        if self.message_visible(index) {
            self.filtered_indices.push(index);
        }
    }

    /// Record a command submission and echo it into the log stream.
    pub fn record_command(&mut self, command: impl Into<String>) {
        let command = command.into();
        self.history.push(command.clone());
        if self.history.len() > MAX_COMMAND_HISTORY {
            self.history
                .drain(0..self.history.len() - MAX_COMMAND_HISTORY);
        }
        self.log_from_source(LogLevel::Info, "editor", format!("> {command}"));
    }

    /// Clear console output.
    pub fn clear(&mut self) {
        self.messages.clear();
        self.filtered_indices.clear();
        self.level_counts = ConsoleLevelCounts::default();
    }

    /// Get all messages.
    #[must_use]
    pub fn messages(&self) -> &[Arc<LogMessage>] {
        &self.messages
    }

    /// Messages matching the current console model filter, oldest first, as
    /// an Arc'd snapshot: copying this is pointer refcounts, not message
    /// deep-clones, so render can hand it to the row list without copying
    /// every visible entry.
    #[must_use]
    pub fn visible_messages(&self) -> Arc<Vec<Arc<LogMessage>>> {
        Arc::from(
            self.filtered_indices
                .iter()
                .filter_map(|&index| self.messages.get(index).cloned())
                .collect::<Vec<_>>(),
        )
    }

    /// Count messages by severity.
    #[must_use]
    pub const fn level_counts(&self) -> ConsoleLevelCounts {
        self.level_counts
    }

    const fn level_filter(&self) -> ConsoleLevelFilter {
        self.level_filter
    }

    #[must_use]
    pub fn filter_query(&self) -> &str {
        &self.filter_query
    }

    pub fn set_filter_query(&mut self, query: impl Into<String>) {
        let query = query.into();
        if self.filter_query == query {
            return;
        }
        self.filter_query = query;
        self.rebuild_filtered_indices();
    }

    pub fn clear_filter_query(&mut self) {
        self.set_filter_query(String::new());
    }

    pub fn toggle_level_filter(&mut self, level: LogLevel) {
        match level {
            LogLevel::Error => self.level_filter.error = !self.level_filter.error,
            LogLevel::Warn => self.level_filter.warn = !self.level_filter.warn,
            LogLevel::Info => self.level_filter.info = !self.level_filter.info,
            LogLevel::Debug | LogLevel::Trace => self.level_filter.debug = !self.level_filter.debug,
        }
        self.rebuild_filtered_indices();
    }

    /// Get submitted command history.
    #[must_use]
    pub fn history(&self) -> &[String] {
        &self.history
    }

    fn rebuild_indexes(&mut self) {
        self.level_counts = ConsoleLevelCounts::tally(&self.messages);
        self.rebuild_filtered_indices();
    }

    fn rebuild_filtered_indices(&mut self) {
        self.filtered_indices = (0..self.messages.len())
            .filter(|&index| self.message_visible(index))
            .collect();
    }

    fn message_visible(&self, index: usize) -> bool {
        self.messages.get(index).is_some_and(|message| {
            self.level_filter.shows(message.level) && message.matches_query(&self.filter_query)
        })
    }

    const fn count_level(&mut self, level: LogLevel) {
        match level {
            LogLevel::Error => self.level_counts.error += 1,
            LogLevel::Warn => self.level_counts.warn += 1,
            LogLevel::Info => self.level_counts.info += 1,
            LogLevel::Debug | LogLevel::Trace => self.level_counts.debug += 1,
        }
    }
}

impl Console {
    pub const NAME: &'static str = "console";

    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Run a session command..."));
        let filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter source or text..."));
        let subscriptions = vec![
            cx.subscribe_in(&input, window, Self::on_input_event),
            cx.subscribe_in(&filter_input, window, Self::on_filter_input_event),
        ];

        Self {
            input,
            filter_input,
            input_cache: String::new(),
            history_cursor: None,
            copy_feedback_visible: false,
            copy_epoch: 0,
            copy_feedback_task: None,
            log_scroll: UniformListScrollHandle::new(),
            last_visible_count: 0,
            stick_to_bottom: true,
            _subscriptions: subscriptions,
        }
    }

    pub fn init(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        Self::new(window, cx)
    }

    /// Echo a submitted command into shared console state.
    pub fn execute_command(cx: &mut gpui::App, command: impl Into<String>) {
        cx.default_global::<ConsoleState>().record_command(command);
        cx.refresh_windows();
    }

    /// Clear console
    pub fn clear(cx: &mut gpui::App) {
        cx.default_global::<ConsoleState>().clear();
        cx.refresh_windows();
    }

    /// Get the current command input.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input_cache
    }

    fn on_input_event(
        &mut self,
        state: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.input_cache = state.read(cx).value().to_string();
            }
            InputEvent::PressEnter { .. } => {
                if let Some(command) = normalize_submission(&state.read(cx).value()) {
                    self.history_cursor = None;
                    self.set_input_value("", state, window, cx);
                    window.dispatch_action(
                        Box::new(crate::actions::ExecuteConsoleCommand { command }),
                        cx,
                    );
                }
            }
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    // GPUI's `subscribe_in` takes this by function pointer, so the receiver is
    // part of the signature it must have even though the body reads globals.
    #[allow(clippy::unused_self)]
    fn on_filter_input_event(
        &mut self,
        state: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            let query = state.read(cx).value().to_string();
            cx.default_global::<ConsoleState>().set_filter_query(query);
            cx.notify();
        }
    }

    fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let input_focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        if !input_focused || event.keystroke.modifiers.modified() {
            return;
        }

        let handled = match event.keystroke.key.as_str() {
            "up" => self.recall_previous_history(window, cx),
            "down" => self.recall_next_history(window, cx),
            _ => false,
        };

        if handled {
            cx.stop_propagation();
        }
    }

    fn focus_input(
        &mut self,
        _event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn recall_previous_history(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let Some((index, command)) = self.previous_history_command(cx) else {
            return false;
        };

        self.history_cursor = Some(index);
        let input = self.input.clone();
        self.set_input_value(command, &input, window, cx);
        true
    }

    fn recall_next_history(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let Some(command) = self.next_history_command(cx) else {
            return false;
        };

        let input = self.input.clone();
        self.set_input_value(command, &input, window, cx);
        true
    }

    fn previous_history_command(&self, cx: &Context<'_, Self>) -> Option<(usize, String)> {
        let history = cx.try_global::<ConsoleState>()?.history();
        if history.is_empty() {
            return None;
        }

        let index = self
            .history_cursor
            .unwrap_or(history.len())
            .saturating_sub(1);
        Some((index, history[index].clone()))
    }

    fn next_history_command(&mut self, cx: &Context<'_, Self>) -> Option<String> {
        let history = cx.try_global::<ConsoleState>()?.history();
        let cursor = self.history_cursor?;
        let next = cursor + 1;
        if next >= history.len() {
            self.history_cursor = None;
            Some(String::new())
        } else {
            self.history_cursor = Some(next);
            Some(history[next].clone())
        }
    }

    fn set_input_value(
        &mut self,
        value: impl Into<String>,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let value = value.into();
        self.input_cache.clone_from(&value);
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }

    fn copy_text(&mut self, text: String, cx: &mut Context<'_, Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copy_feedback_visible = true;
        self.copy_epoch = self.copy_epoch.wrapping_add(1);
        let epoch = self.copy_epoch;
        self.copy_feedback_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(2_500))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.copy_epoch == epoch {
                    this.copy_feedback_visible = false;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }
}

impl Console {
    /// Log a message to the global console
    pub fn log_global(cx: &mut gpui::App, level: LogLevel, message: String) {
        cx.default_global::<ConsoleState>().log(level, message);
        cx.refresh_windows();
    }
}

/// View component that renders the global console
pub struct ConsoleView;

impl Render for Console {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (counts, visible) = cx.try_global::<ConsoleState>().map_or_else(
            || (ConsoleLevelCounts::default(), Arc::default()),
            |state| (state.level_counts(), state.visible_messages()),
        );
        let visible_count = visible.len();
        if visible_count != self.last_visible_count {
            if self.stick_to_bottom && visible_count > 0 {
                self.log_scroll.scroll_to_bottom();
            }
            self.last_visible_count = visible_count;
        } else if let Some(at_end) = self.log_scroll.is_scrolled_to_end() {
            // Manual scrolling disengages follow mode; reaching the end again
            // re-enables it for the next appended log entry.
            self.stick_to_bottom = at_end;
        }

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .track_focus(&self.focus_handle(cx))
            .capture_key_down(cx.listener(Self::on_key_down))
            .child(self.render_filter_toolbar(counts, &theme, cx))
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .py(px(2.0))
                    .bg(theme.sidebar)
                    .when(visible.is_empty(), |this| {
                        this.child(kit::empty_state("No console output", None, &theme))
                    })
                    .when(!visible.is_empty(), |this| {
                        this.child(
                            div()
                                .size_full()
                                .child(
                                    div().size_full().overflow_x_scrollbar().child(
                                        uniform_list("console-rows", visible.len(), {
                                            let theme = theme.clone();
                                            cx.processor(
                                                move |_this,
                                                      range: std::ops::Range<usize>,
                                                      _window,
                                                      cx| {
                                                    range
                                                        .map(|index| {
                                                            let message = &visible[index];
                                                            let signature =
                                                                console_row_signature(message);
                                                            render_console_entry(
                                                                message, signature, &theme, cx,
                                                            )
                                                            .into_any_element()
                                                        })
                                                        .collect::<Vec<_>>()
                                                },
                                            )
                                        })
                                        .size_full()
                                        .track_scroll(&self.log_scroll),
                                    ),
                                )
                                .vertical_scrollbar(&self.log_scroll),
                        )
                    }),
            )
            .child(
                h_flex()
                    .h(px(31.0))
                    .w_full()
                    .flex_none()
                    .border_t_1()
                    .border_color(theme.border)
                    .bg(theme.input_background())
                    .items_center()
                    .px_2()
                    .gap_1p5()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::focus_input))
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .with_size(px(15.0))
                            .text_color(theme.accent),
                    )
                    .child(
                        div().flex_1().child(
                            Input::new(&self.input)
                                .small()
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false),
                        ),
                    ),
            )
    }
}

impl Console {
    fn render_filter_toolbar(
        &self,
        counts: ConsoleLevelCounts,
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        kit::panel_toolbar(theme)
            .child(Self::render_console_clear_button(theme, cx))
            .child(div().w(px(1.0)).h(px(16.0)).bg(theme.border).mx_1())
            .child(Self::level_filter_chip(
                "console-flt-error",
                "Errors",
                IconName::CircleX,
                theme.danger,
                counts.error,
                cx.try_global::<ConsoleState>()
                    .is_none_or(|state| state.level_filter().error),
                cx.listener(|this, _, _, cx| {
                    let _ = this;
                    cx.default_global::<ConsoleState>()
                        .toggle_level_filter(LogLevel::Error);
                    cx.notify();
                }),
                theme,
            ))
            .child(Self::level_filter_chip(
                "console-flt-warn",
                "Warnings",
                IconName::TriangleAlert,
                theme.warning,
                counts.warn,
                cx.try_global::<ConsoleState>()
                    .is_none_or(|state| state.level_filter().warn),
                cx.listener(|this, _, _, cx| {
                    let _ = this;
                    cx.default_global::<ConsoleState>()
                        .toggle_level_filter(LogLevel::Warn);
                    cx.notify();
                }),
                theme,
            ))
            .child(Self::level_filter_chip(
                "console-flt-info",
                "Info",
                IconName::Info,
                theme.info,
                counts.info,
                cx.try_global::<ConsoleState>()
                    .is_none_or(|state| state.level_filter().info),
                cx.listener(|this, _, _, cx| {
                    let _ = this;
                    cx.default_global::<ConsoleState>()
                        .toggle_level_filter(LogLevel::Info);
                    cx.notify();
                }),
                theme,
            ))
            .child(Self::level_filter_chip(
                "console-flt-debug",
                "Debug",
                IconName::Bot,
                theme.muted_foreground,
                counts.debug,
                cx.try_global::<ConsoleState>()
                    .is_none_or(|state| state.level_filter().debug),
                cx.listener(|this, _, _, cx| {
                    let _ = this;
                    cx.default_global::<ConsoleState>()
                        .toggle_level_filter(LogLevel::Debug);
                    cx.notify();
                }),
                theme,
            ))
            .child(div().flex_1())
            .child(self.render_console_source_filter(theme, cx))
            .when(self.copy_feedback_visible, |this| {
                this.child(
                    div()
                        .id("console-copy-feedback")
                        .flex_none()
                        .text_size(px(11.0))
                        .text_color(theme.success)
                        .child("Copied"),
                )
            })
            .child(Self::render_console_copy_all_button(theme, cx))
    }

    /// Toolbar Clear button; empties the console buffer.
    fn render_console_clear_button(
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        div()
            .id("console-clear")
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .h(px(23.0))
            .px_2()
            .rounded(px(5.0))
            .text_size(px(11.0))
            .text_color(theme.muted_foreground)
            .hover(|this| this.bg(theme.secondary_hover).text_color(theme.foreground))
            .cursor_pointer()
            .child(Icon::new(IconName::Delete).with_size(px(15.0)))
            .child("Clear")
            .on_click(cx.listener(|_, _, _, cx| {
                cx.default_global::<ConsoleState>().clear();
                cx.refresh_windows();
            }))
    }

    /// Toolbar source-filter control: the current source and its dropdown.
    fn render_console_source_filter(
        &self,
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        h_flex()
            .id("console-source-filter")
            .flex_none()
            .w(px(190.0))
            .h(px(23.0))
            .items_center()
            .gap(px(5.0))
            .px(px(8.0))
            .rounded(px(5.0))
            .bg(theme.input_background())
            .border_1()
            .border_color(theme.border)
            .text_color(theme.muted_foreground)
            .child(Icon::new(IconName::Search).with_size(px(14.0)))
            .child(
                div().flex_1().child(
                    Input::new(&self.filter_input)
                        .small()
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false),
                ),
            )
            .when(
                cx.try_global::<ConsoleState>()
                    .is_some_and(|state| !state.filter_query().trim().is_empty()),
                |this| {
                    this.child(
                        div()
                            .id("console-source-filter-clear")
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(16.0))
                            .rounded(px(4.0))
                            .hover(|this| this.bg(theme.secondary_hover))
                            .cursor_pointer()
                            .child(Icon::new(IconName::Close).with_size(px(12.0)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.default_global::<ConsoleState>().clear_filter_query();
                                let input = this.filter_input.clone();
                                input.update(cx, |input, cx| {
                                    input.set_value(String::new(), window, cx);
                                });
                                cx.notify();
                            })),
                    )
                },
            )
    }

    /// Toolbar Copy All button; puts the visible lines on the clipboard.
    fn render_console_copy_all_button(
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        div()
            .id("console-copy-all")
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(px(23.0))
            .rounded(px(5.0))
            .text_color(theme.muted_foreground)
            .hover(|this| this.bg(theme.secondary_hover).text_color(theme.foreground))
            .cursor_pointer()
            .tooltip(|window, cx| Tooltip::new("Copy visible console output").build(window, cx))
            .child(Icon::new(IconName::Copy).with_size(px(15.0)))
            .on_click(cx.listener(move |this, _, _, cx| {
                let text = cx
                    .try_global::<ConsoleState>()
                    .map(|state| {
                        state
                            .visible_messages()
                            .iter()
                            .map(|message| console_line_text(message))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                this.copy_text(text, cx);
            }))
    }
    #[allow(clippy::too_many_arguments)]
    fn level_filter_chip(
        id: &'static str,
        label: &'static str,
        icon: IconName,
        icon_color: Hsla,
        count: usize,
        active: bool,
        on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
        theme: &gpui_component::theme::Theme,
    ) -> impl IntoElement {
        h_flex()
            .id(id)
            .flex_none()
            .h(px(23.0))
            .items_center()
            .gap(px(5.0))
            .px(px(8.0))
            .rounded(px(5.0))
            .text_size(px(11.0))
            .when(active, |this| this.text_color(theme.foreground))
            .when(!active, |this| {
                this.text_color(theme.muted_foreground.opacity(0.55))
            })
            .hover(|this| this.bg(theme.secondary_hover))
            .cursor_pointer()
            .child(Icon::new(icon).with_size(px(14.0)).text_color(if active {
                icon_color
            } else {
                theme.muted_foreground
            }))
            .child(label)
            .child(
                div()
                    .font_family("monospace")
                    .text_size(px(10.0))
                    .text_color(theme.muted_foreground)
                    .child(count.to_string()),
            )
            .on_click(on_click)
    }
}

/// One console line: severity glyph, timestamp, then the monospace message.
fn render_console_entry(
    message: &LogMessage,
    signature: String,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, Console>,
) -> impl IntoElement {
    let (icon, icon_color, text_color) = match message.level {
        LogLevel::Error => (IconName::CircleX, theme.danger, theme.foreground),
        LogLevel::Warn => (IconName::TriangleAlert, theme.warning, theme.foreground),
        LogLevel::Info => (IconName::Info, theme.info, theme.foreground),
        LogLevel::Debug | LogLevel::Trace => (
            IconName::Bot,
            theme.muted_foreground,
            theme.muted_foreground,
        ),
    };

    let copied_text = console_line_text(message);
    h_flex()
        .id(format!("console-row-{signature}"))
        .group("console-row")
        .min_w_full()
        .h(px(20.0))
        .flex_none()
        .items_center()
        .gap_2()
        .px_2()
        .font_family("monospace")
        .text_size(px(11.0))
        .hover(|this| this.bg(theme.secondary_hover))
        .cursor_pointer()
        .child(
            Icon::new(icon)
                .with_size(px(13.0))
                .text_color(icon_color)
                .flex_shrink_0(),
        )
        .child(
            div()
                .flex_none()
                .text_color(theme.muted_foreground.opacity(0.7))
                .group_hover("console-row", |this| this.text_color(theme.foreground))
                .child(format_console_time(message.timestamp)),
        )
        .child(
            div()
                .flex_none()
                .max_w_full()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_color(text_color)
                .group_hover("console-row", |this| this.text_color(theme.foreground))
                .child(message.message.clone()),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            let _ = &signature;
            this.copy_text(copied_text.clone(), cx);
        }))
}

fn console_line_text(message: &LogMessage) -> String {
    format!(
        "{} [{}] {}",
        format_console_time(message.timestamp),
        message.source,
        message.message
    )
}

fn console_row_signature(message: &LogMessage) -> String {
    let millis = message
        .timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    format!("{millis}-{}", message.message)
}

/// Format a console timestamp as `HH:MM:SS` in local wall-clock terms.
#[must_use]
pub fn format_console_time(timestamp: std::time::SystemTime) -> String {
    let secs = timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let day_secs = secs % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        day_secs / 3600,
        (day_secs % 3600) / 60,
        day_secs % 60
    )
}

impl Focusable for Console {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Panel for Console {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("terminal"), "Console", kit::TabTone::Default)
    }

    fn inner_padding(&self, _cx: &gpui::App) -> bool {
        false
    }
}

impl gpui::EventEmitter<gpui_component::dock::PanelEvent> for Console {}

fn normalize_submission(value: &str) -> Option<String> {
    let command = value.trim();
    (!command.is_empty()).then(|| command.to_string())
}

fn normalize_console_query(query: &str) -> String {
    query.trim().to_ascii_lowercase()
}

fn console_search_text(source: &str, message: &str) -> String {
    format!("{source}\n{message}").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_console_submission() {
        assert_eq!(
            normalize_submission("  cargo check -p az-editor  "),
            Some("cargo check -p az-editor".to_string())
        );
        assert_eq!(normalize_submission(" \t "), None);
    }

    #[test]
    fn command_history_caps_at_the_limit_keeping_newest() {
        let mut state = ConsoleState::default();
        let total = MAX_COMMAND_HISTORY + 30;
        for index in 0..total {
            state.record_command(format!("cmd-{index}"));
        }

        assert_eq!(state.history().len(), MAX_COMMAND_HISTORY);
        assert_eq!(state.history()[0], "cmd-30");
        assert_eq!(
            state.history().last().unwrap(),
            &format!("cmd-{}", total - 1)
        );
    }

    #[test]
    fn visible_console_messages_are_not_render_truncated() {
        let mut state = ConsoleState::default();
        for index in 1..=128 {
            state.log(LogLevel::Info, format!("message-{index:03}"));
        }

        let messages = state.visible_messages();

        assert_eq!(messages.len(), 128);
        assert_eq!(
            messages.first().map(|msg| msg.message.as_str()),
            Some("message-001")
        );
        assert_eq!(
            messages.last().map(|msg| msg.message.as_str()),
            Some("message-128")
        );
    }

    #[test]
    fn console_log_message_matches_source_or_text_query() {
        let mut state = ConsoleState::default();
        state.log_from_source(
            LogLevel::Warn,
            "asset-processor:stderr",
            "schema load failed",
        );
        let message = state.messages().first().unwrap();

        assert!(message.matches_query("asset-processor"));
        assert!(message.matches_query("schema load"));
        assert!(message.matches_query("ASSET"));
        assert!(!message.matches_query("project-host"));
    }

    #[test]
    fn console_visibility_filters_by_level_and_source_or_text_query() {
        let mut state = ConsoleState::default();
        state.log_from_source(LogLevel::Warn, "asset-processor:stderr", "stale dependency");

        state.set_filter_query("asset-processor");
        assert_eq!(state.visible_messages().len(), 1);

        state.set_filter_query("dependency");
        assert_eq!(state.visible_messages().len(), 1);

        state.set_filter_query("project-host");
        assert!(state.visible_messages().is_empty());

        state.clear_filter_query();
        state.toggle_level_filter(LogLevel::Warn);
        assert!(state.visible_messages().is_empty());
    }

    #[test]
    fn console_filter_query_can_backspace_and_clear_without_rebuilding_in_render() {
        let mut state = ConsoleState::default();
        state.log_from_source(LogLevel::Info, "asset-processor", "processed texture");
        state.log_from_source(LogLevel::Info, "project-host", "opened prefab");

        state.set_filter_query("asset-proc");
        assert_eq!(state.visible_messages().len(), 1);

        state.set_filter_query("asset");
        assert_eq!(state.visible_messages().len(), 1);

        state.clear_filter_query();
        assert_eq!(state.visible_messages().len(), 2);
    }
}
