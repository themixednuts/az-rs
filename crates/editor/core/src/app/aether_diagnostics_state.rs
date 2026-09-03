//! Aether-local console and diagnostics presentation controls.

use super::AetherConsoleFilter;
use crate::app::aether_common::replace_string_if_changed;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) struct AetherConsolePresentation<'a> {
    pub(in crate::app) filter: AetherConsoleFilter,
    pub(in crate::app) query: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) struct AetherDiagnosticsPresentation<'a> {
    pub(in crate::app) console: AetherConsolePresentation<'a>,
    pub(in crate::app) show_stats: bool,
}

/// Presentation-only controls. Runtime and mannequin playback truth remain in
/// their controller-published projections.
#[derive(Debug, Clone)]
pub(super) struct AetherDiagnosticsState {
    console_filter: AetherConsoleFilter,
    console_query: String,
    show_stats: bool,
}

impl Default for AetherDiagnosticsState {
    fn default() -> Self {
        Self {
            console_filter: AetherConsoleFilter::All,
            console_query: String::new(),
            show_stats: true,
        }
    }
}

impl AetherDiagnosticsState {
    pub(super) fn presentation(&self) -> AetherDiagnosticsPresentation<'_> {
        AetherDiagnosticsPresentation {
            console: AetherConsolePresentation {
                filter: self.console_filter,
                query: &self.console_query,
            },
            show_stats: self.show_stats,
        }
    }

    pub(super) fn select_console_filter(&mut self, filter: AetherConsoleFilter) -> bool {
        let changed = self.console_filter != filter;
        self.console_filter = filter;
        changed
    }

    pub(super) fn set_console_query(&mut self, query: &str) -> bool {
        replace_string_if_changed(&mut self.console_query, query)
    }

    pub(super) fn clear_console_query(&mut self) -> bool {
        self.set_console_query("")
    }

    pub(super) fn toggle_stats(&mut self) {
        self.show_stats = !self.show_stats;
    }
}
