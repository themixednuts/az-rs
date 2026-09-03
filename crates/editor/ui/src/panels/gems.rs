//! Gems Panel
//!
//! Live engine-gem management for the attached project: enable/disable gems
//! against the workspace gem catalog. Changes are staged in the project
//! manifest and applied on the next rebuild, matching O3DE's staged workflow.
//!
//! The panel header also hosts the **New Gem** form, which drives the same
//! `azoth gem new` scaffold the CLI uses (including the ADR 0026 capability
//! templates) through the editor-owned project workflow controller.

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};
use gpui_component::dock::{Panel, PanelEvent};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, StyledExt, h_flex, v_flex};

use crate::actions::CreateGem;
use crate::gems::{
    EditorGemCatalog, EditorGemCreationStatus, EditorGemInfo, EditorGemRebuildState,
    EditorGemSelection, NewGemCapabilityChoice, derived_gem_id, is_valid_gem_name,
};
use crate::panels::kit;

/// Gems panel — create project gems and enable/disable workspace gems on the
/// attached project.
pub struct GemsPanel {
    focus_handle: FocusHandle,
    /// New-gem name field.
    name_input: Entity<InputState>,
    /// Optional new-gem id field (derived from the name when blank).
    id_input: Entity<InputState>,
    /// Live mirror of `name_input` for validation and the derived-id hint.
    name: String,
    /// Selected capability template for the new gem.
    capability: NewGemCapabilityChoice,
    /// Register the new gem with the attached project (on by default).
    register: bool,
    _subscriptions: Vec<Subscription>,
}

impl GemsPanel {
    pub const NAME: &'static str = "gems";

    pub fn init(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Gem name (e.g. combat-system)"));
        let id_input = cx
            .new(|cx| InputState::new(window, cx).placeholder("Gem id (optional — derived below)"));
        let subscription = cx.subscribe_in(&name_input, window, Self::on_name_input);
        Self {
            focus_handle: cx.focus_handle(),
            name_input,
            id_input,
            name: String::new(),
            capability: NewGemCapabilityChoice::None,
            register: true,
            _subscriptions: vec![subscription],
        }
    }

    fn on_name_input(
        &mut self,
        state: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
            self.name = state.read(cx).value().to_string();
            cx.notify();
        }
    }

    fn select_capability(
        &mut self,
        capability: NewGemCapabilityChoice,
        cx: &mut Context<'_, Self>,
    ) {
        if self.capability != capability {
            self.capability = capability;
            cx.notify();
        }
    }

    fn toggle_register(&mut self, cx: &mut Context<'_, Self>) {
        self.register = !self.register;
        cx.notify();
    }

    /// The New Gem form rendered under the panel header.
    fn render_new_gem_form(
        &self,
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let name = self.name.trim().to_string();
        let name_valid = is_valid_gem_name(&name);
        let derived_id = derived_gem_id(&name);
        let feedback = Self::new_gem_feedback(&name, name_valid, theme, cx);

        v_flex()
            .gap(px(8.0))
            .px(px(14.0))
            .py(px(12.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_size(gpui::Rems(0.6))
                    .font_semibold()
                    .text_color(theme.sidebar_foreground)
                    .child("NEW GEM"),
            )
            .child(new_gem_text_field("new-gem-name", &self.name_input, theme))
            .child(new_gem_text_field("new-gem-id", &self.id_input, theme))
            .child(
                div()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(gpui::Rems(0.58))
                    .text_color(theme.muted_foreground)
                    .child(format!("Defaults to {derived_id}")),
            )
            .child(self.new_gem_capability_options(theme, cx))
            .child(self.new_gem_register_toggle(theme, cx))
            .child(self.new_gem_create_button(&name, name_valid, theme, cx))
            .when_some(feedback, |this, (color, message)| {
                this.child(
                    h_flex()
                        .items_start()
                        .gap(px(6.0))
                        .text_color(color)
                        .child(Icon::new(IconName::Info).size(px(13.0)))
                        .child(div().flex_1().text_size(gpui::Rems(0.6)).child(message)),
                )
            })
    }

    /// Capability template options (human names, one-line descriptions).
    fn new_gem_capability_options(
        &self,
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let selected_capability = self.capability;
        v_flex()
            .gap(px(6.0))
            .children(NewGemCapabilityChoice::ALL.into_iter().map(move |choice| {
                let selected = selected_capability == choice;
                div()
                    .id(SharedString::from(format!(
                        "new-gem-cap-{}",
                        choice.label()
                    )))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .p(px(8.0))
                    .rounded(px(6.0))
                    .bg(theme.colors.list)
                    .border_1()
                    .border_color(if selected {
                        theme.list_active_border
                    } else {
                        theme.border
                    })
                    .hover(|s| s.border_color(theme.accent))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.select_capability(choice, cx)))
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                Icon::new(if selected {
                                    IconName::CircleCheck
                                } else {
                                    IconName::Circle
                                })
                                .size(px(13.0))
                                .text_color(if selected {
                                    theme.primary
                                } else {
                                    theme.muted_foreground
                                }),
                            )
                            .child(
                                div()
                                    .text_size(gpui::Rems(0.68))
                                    .font_semibold()
                                    .text_color(theme.foreground)
                                    .child(choice.label()),
                            ),
                    )
                    .child(
                        div()
                            .pl(px(19.0))
                            .text_size(gpui::Rems(0.6))
                            .text_color(theme.muted_foreground)
                            .child(choice.description()),
                    )
            }))
    }

    /// Register-with-project toggle.
    fn new_gem_register_toggle(
        &self,
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let register_on = self.register;
        let (track, knob) = if register_on {
            (theme.primary, theme.primary_foreground)
        } else {
            (theme.secondary_active, theme.muted_foreground)
        };
        h_flex()
            .id("new-gem-register")
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.toggle_register(cx)))
            .child(
                div()
                    .text_size(gpui::Rems(0.64))
                    .text_color(theme.foreground)
                    .child("Register with project"),
            )
            .child(
                div()
                    .w(px(30.0))
                    .h(px(17.0))
                    .rounded(px(9.0))
                    .bg(track)
                    .child(
                        div()
                            .mt(px(2.0))
                            .ml(if register_on { px(15.0) } else { px(2.0) })
                            .size(px(13.0))
                            .rounded_full()
                            .bg(knob),
                    ),
            )
    }

    /// The Create button; dispatches [`CreateGem`] when the name is valid.
    fn new_gem_create_button(
        &self,
        name: &str,
        can_create: bool,
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let action = CreateGem {
            name: name.to_string(),
            id: {
                let raw = self.id_input.read(cx).value().trim().to_string();
                (!raw.is_empty()).then_some(raw)
            },
            capability: self.capability.capability_id().map(str::to_string),
            register: self.register,
        };
        div()
            .id("new-gem-create")
            .flex()
            .items_center()
            .justify_center()
            .h(px(28.0))
            .rounded(px(6.0))
            .bg(if can_create {
                theme.primary
            } else {
                theme.secondary_active
            })
            .text_size(gpui::Rems(0.68))
            .font_semibold()
            .text_color(if can_create {
                theme.primary_foreground
            } else {
                theme.muted_foreground
            })
            .when(can_create, |this| {
                this.cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(action.clone()), cx);
                    })
            })
            .child("Create Gem")
    }

    /// Inline validation / scaffold feedback shown beneath the form.
    fn new_gem_feedback(
        name: &str,
        name_valid: bool,
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> Option<(gpui::Hsla, String)> {
        if !name.is_empty() && !name_valid {
            return Some((
                theme.danger,
                "Use letters, digits, '-' or '_'; cannot start with a digit.".to_string(),
            ));
        }
        match cx
            .try_global::<EditorGemCreationStatus>()
            .and_then(|status| status.outcome.clone())
        {
            Some(Ok(message)) => Some((theme.success, message)),
            Some(Err(message)) => Some((theme.danger, message)),
            None => None,
        }
    }
}

/// A borderless single-line field wrapped in the panel's input chrome.
fn new_gem_text_field(
    id: &'static str,
    input: &Entity<InputState>,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .id(id)
        .h(px(28.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .child(
            Input::new(input)
                .small()
                .appearance(false)
                .bordered(false)
                .focus_bordered(false),
        )
}

impl Render for GemsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme().clone();

        let mut gems: Vec<EditorGemInfo> = cx
            .try_global::<EditorGemCatalog>()
            .map(|catalog| catalog.gems.clone())
            .unwrap_or_default();
        gems.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
        let discovery_error = cx
            .try_global::<EditorGemCatalog>()
            .and_then(|catalog| catalog.status_error.clone());
        let enabled_ids: Vec<String> = cx
            .try_global::<EditorGemSelection>()
            .map(|selection| selection.enabled.clone())
            .unwrap_or_default();
        let is_enabled = |id: &str| enabled_ids.iter().any(|gem| gem == id);
        let enabled_count = gems.iter().filter(|gem| is_enabled(&gem.id)).count();
        let rebuild_pending = cx
            .try_global::<EditorGemRebuildState>()
            .is_some_and(|state| state.rebuild_pending);

        let new_gem_form = self.render_new_gem_form(&theme, cx);

        // Scrollable body, grouped by category.
        let mut body = v_flex()
            .id("gems-panel-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .px(px(14.0))
            .py(px(12.0))
            .gap(px(14.0));

        if let Some(error) = discovery_error {
            body = body.child(
                h_flex()
                    .items_center()
                    .gap(px(7.0))
                    .text_color(theme.danger)
                    .child(Icon::new(IconName::TriangleAlert).size(px(15.0)))
                    .child(format!("Could not resolve the engine gem catalog: {error}")),
            );
        }

        let mut current_category: Option<String> = None;
        for gem in &gems {
            if current_category.as_deref() != Some(gem.category.as_str()) {
                current_category = Some(gem.category.clone());
                body = body.child(
                    div()
                        .text_size(gpui::Rems(0.6))
                        .font_semibold()
                        .text_color(theme.sidebar_foreground)
                        .child(gem.category.to_uppercase()),
                );
            }

            body = body.child(render_engine_gem_row(gem, is_enabled(&gem.id), &theme));
        }

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            // Header
            .child(render_gems_header(enabled_count, gems.len(), &theme))
            // Rebuild-pending banner
            .when(rebuild_pending, |this| {
                this.child(
                    h_flex()
                        .flex_none()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(14.0))
                        .py(px(7.0))
                        .bg(theme.warning.opacity(0.12))
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .size(px(15.0))
                                .text_color(theme.warning),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(gpui::Rems(0.66))
                                .text_color(theme.warning)
                                .child("Gem changes staged — rebuild to apply."),
                        ),
                )
            })
            // New Gem form
            .child(new_gem_form)
            .child(body)
    }
}

/// One engine-gem catalog row with its (display-only) enabled toggle.
fn render_engine_gem_row(
    gem: &EditorGemInfo,
    on: bool,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let (track, knob) = if on {
        (theme.primary, theme.primary_foreground)
    } else {
        (theme.secondary_active, theme.muted_foreground)
    };
    let toggle = div()
        .w(px(30.0))
        .h(px(17.0))
        .rounded(px(9.0))
        .bg(track)
        .child(
            div()
                .mt(px(2.0))
                .ml(if on { px(15.0) } else { px(2.0) })
                .size(px(13.0))
                .rounded_full()
                .bg(knob),
        );

    h_flex()
        .id(SharedString::from(gem.id.clone()))
        .items_start()
        .gap(px(10.0))
        .p(px(10.0))
        .rounded(px(8.0))
        .bg(theme.colors.list)
        .border_1()
        .border_color(if on {
            theme.list_active_border
        } else {
            theme.border
        })
        .hover(|s| s.border_color(theme.accent))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.0))
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(7.0))
                        .child(
                            div()
                                .text_size(gpui::Rems(0.72))
                                .font_semibold()
                                .text_color(theme.foreground)
                                .child(gem.name.clone()),
                        )
                        .when(gem.is_deprecated(), |this| {
                            this.child(
                                div()
                                    .px(px(6.0))
                                    .py(px(2.0))
                                    .rounded(px(4.0))
                                    .bg(theme.warning.opacity(0.14))
                                    .text_size(gpui::Rems(0.52))
                                    .font_semibold()
                                    .text_color(theme.warning)
                                    .child("DEPRECATED"),
                            )
                        })
                        .child(
                            div()
                                .font_family(theme.mono_font_family.clone())
                                .text_size(gpui::Rems(0.58))
                                .text_color(theme.muted_foreground)
                                .child(format!("v{}", gem.version)),
                        ),
                )
                .child(
                    div()
                        .text_size(gpui::Rems(0.64))
                        .text_color(theme.muted_foreground)
                        .child(gem.display_description()),
                ),
        )
        .child(toggle)
}

/// Panel header: the title and the enabled-of-total count.
fn render_gems_header(
    enabled_count: usize,
    total_count: usize,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .px(px(14.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(gpui::Rems(0.82))
                .font_semibold()
                .text_color(theme.foreground)
                .child("Gems"),
        )
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(gpui::Rems(0.64))
                .text_color(theme.muted_foreground)
                .child(format!("{enabled_count}/{total_count} enabled")),
        )
        .child(div().flex_1())
}

impl Focusable for GemsPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for GemsPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        kit::tab_title(Some("extension"), "Gems", kit::TabTone::Default)
    }
}

impl gpui::EventEmitter<PanelEvent> for GemsPanel {}
