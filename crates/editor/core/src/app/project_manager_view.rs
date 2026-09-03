//! Hand-owned project-manager layout composed from focused screen regions.
//!
//! Design reference material informs region-level changes only; it never
//! replaces this runtime implementation wholesale.

mod footer;
mod nav_rail;
mod new_project;
mod placeholder;
mod recent;
mod summary;
mod titlebar;

use super::project_manager::ProjectManagerView;
use gpui::{Context, IntoElement, ParentElement as _, Styled as _, Window};
use gpui_component::ActiveTheme as _;

impl ProjectManagerView {
    pub(super) fn render_aether_manager(
        &self,
        _window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().clone();
        let screen = if self.view_recent() {
            recent::render_recent_region(self, &theme, cx)
        } else if self.view_new() {
            new_project::render_new_project_region(self, &theme, cx)
        } else {
            placeholder::render_placeholder_region(self, &theme)
        };

        gpui::div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .text_size(gpui::px(12.0))
            .line_height(gpui::relative(1.4))
            .child(titlebar::render_titlebar_region(&theme))
            .child(
                gpui::div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(nav_rail::render_nav_rail_region(self, &theme, cx))
                    .child(screen),
            )
    }
}
