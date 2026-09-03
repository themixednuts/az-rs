//! Animation mode: the editor's animation authoring layout.
//!
//! Left navigator (anim-graph nav / motions / motion sets / skeleton
//! sections plus always-visible parameters), center anim-graph workbench with
//! breadcrumb, right mannequin viewport + attributes, bottom motion event
//! tracks.
//!
//! Layout follows `design/aether-handoff/project/Aether Editor.dc.html`
//! (Animation Editor region). All data comes from the editor-core-published
//! globals ([`EditorAnimationPreviewCatalog`], [`EditorMannequinPreview`],
//! [`EditorMannequinAuthoringCatalog`], [`EditorBlendSpacePreview`] +
//! catalog, [`EditorGraphDocumentProjection`]); panels dispatch typed
//! animation/graph actions only.

use std::collections::{BTreeMap, HashMap};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Point, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px, relative,
};
use gpui_component::dock::{Panel, PanelEvent};
use gpui_component::scroll::ScrollableElement;
use gpui_component::theme::Theme;
use gpui_component::{ActiveTheme, ElementExt, StyledExt, h_flex, v_flex};

use crate::actions::{
    OpenGraphDocument, SaveGraphDocument, ScrubAnimationPreview, SelectAnimationBlendSpace,
    SelectAnimationMotion, SelectMannequinFragment, SetAnimationBlendSpaceParameter,
    SetAnimationPreviewLoop, SetAnimationPreviewPlaying, SetMannequinTag, StopAnimationPreview,
};
use crate::panels::graph::VisualGraphPanel;
use crate::panels::kit;
use crate::panels::{
    EditorAnimationMotionData, EditorAnimationPreviewCatalog, EditorBlendSpaceDimensionData,
    EditorBlendSpacePreview, EditorBlendSpacePreviewCatalog, EditorGraphDocumentProjection,
    EditorMannequinAuthoringCatalog, EditorMannequinPreview, EditorViewportPanelFrame,
    EditorViewportRenderStateData, EditorViewportRenderStatus, InProcessViewportSurface,
    ViewportSurfaceOwner, render_project_host_connection_placeholder,
};

/// Whether a registered graph type is an animation graph (state machine /
/// blend tree) the Animation workbench should host.
///
/// Today the node-graph catalog registers no animation graph type, so the
/// navigator's Anim Graph section reports that explicitly instead of listing
/// unrelated graphs.
#[must_use]
pub fn is_animation_graph_type(graph_type: &str) -> bool {
    graph_type.to_ascii_lowercase().contains("anim")
}

fn element_id(prefix: &str, key: &str) -> SharedString {
    SharedString::from(format!(
        "{prefix}-{}",
        key.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
    ))
}

fn format_millis(millis: u32) -> String {
    format!("{:.2}s", kit::ratio(millis, 1_000))
}

fn path_stem(path: &str) -> String {
    crate::naming::display_name(path).into_owned()
}

fn animation_set_label(set_path: &str) -> String {
    let label = path_stem(set_path);
    let label = label.trim();
    if label.is_empty() {
        "root".to_owned()
    } else {
        label.to_owned()
    }
}

// ---------------------------------------------------------------------------
// Left dock: sectioned navigator + always-visible parameters
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AnimationNavigatorTab {
    #[default]
    Graph,
    Motions,
    Sets,
    Skeleton,
}

impl AnimationNavigatorTab {
    const ALL: [Self; 4] = [Self::Graph, Self::Motions, Self::Sets, Self::Skeleton];

    const fn label(self) -> &'static str {
        match self {
            Self::Graph => "Graph",
            Self::Motions => "Motions",
            Self::Sets => "Sets",
            Self::Skeleton => "Skeleton",
        }
    }
}

/// Left navigator: Anim Graph / Motions / Motion Sets / Skeleton sections
/// behind tabs, with the blend-space/tag Parameters panel always visible at
/// the bottom (design dc.html Animation left dock).
pub struct AnimationNavigatorPanel {
    focus_handle: FocusHandle,
    tab: AnimationNavigatorTab,
    /// Per-dimension slider track bounds from the last prepaint, keyed by
    /// blend-space dimension name, for parameter scrubbing.
    param_bounds: HashMap<String, Bounds<Pixels>>,
    /// Dimension currently being scrubbed, if any.
    param_dragging: Option<String>,
}

impl AnimationNavigatorPanel {
    pub const NAME: &'static str = "animation-navigator";

    pub fn init(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            tab: AnimationNavigatorTab::default(),
            param_bounds: HashMap::new(),
            param_dragging: None,
        }
    }

    fn scrub_param(
        &self,
        dimension: &str,
        min: f32,
        max: f32,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(bounds) = self.param_bounds.get(dimension) else {
            return;
        };
        let width = f32::from(bounds.size.width).max(1.0);
        let fraction = (f32::from(position.x - bounds.origin.x) / width).clamp(0.0, 1.0);
        let value = min + fraction * (max - min);
        window.dispatch_action(
            Box::new(SetAnimationBlendSpaceParameter {
                dimension: dimension.to_owned(),
                value,
            }),
            cx,
        );
        cx.notify();
    }

    fn render_tabs(&self, theme: &Theme, cx: &Context<'_, Self>) -> impl IntoElement {
        h_flex()
            .flex_none()
            .h(px(30.0))
            .items_center()
            .gap_1()
            .px_1p5()
            .bg(theme.tab_bar)
            .border_b_1()
            .border_color(theme.border)
            .children(AnimationNavigatorTab::ALL.into_iter().map(|tab| {
                kit::filter_chip(
                    element_id("anim-nav-tab", tab.label()),
                    tab.label(),
                    None,
                    None,
                    None,
                    tab == self.tab,
                    theme,
                )
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.tab = tab;
                    cx.notify();
                }))
            }))
    }

    fn render_graph_section(theme: &Theme, cx: &App) -> gpui::AnyElement {
        let mut body = v_flex().gap_1();
        body = body.child(kit::section_label("Anim Graphs", theme));
        body = Self::extend_with_anim_graph_rows(body, theme, cx);
        body = body.child(kit::section_label("States · Mannequin Fragments", theme));
        body = Self::extend_with_fragment_rows(body, theme, cx);
        body.into_any_element()
    }

    /// Anim Graph section rows: one row per open animation graph document, or
    /// the explicit state that says why there are none.
    fn extend_with_anim_graph_rows(mut body: gpui::Div, theme: &Theme, cx: &App) -> gpui::Div {
        let Some(graph) = cx.try_global::<EditorGraphDocumentProjection>() else {
            return body.child(kit::hint_text(
                "Visual graph projection not published yet.",
                theme,
            ));
        };
        let has_animation_type = graph
            .creation_catalog
            .graph_types
            .iter()
            .any(|graph_type| is_animation_graph_type(&graph_type.graph_type));
        if !has_animation_type {
            return body.child(kit::hint_text(
                "No animation graph type is registered in the node-graph catalog.",
                theme,
            ));
        }
        let documents: Vec<_> = graph
            .graph_documents
            .documents
            .iter()
            .filter(|document| is_animation_graph_type(&document.graph_type))
            .collect();
        if documents.is_empty() {
            body = body.child(kit::hint_text("No animation graph documents open.", theme));
        }
        for document in documents {
            let document_id = document.document_id.clone();
            let label = path_stem(&document.source_path).clone();
            body = body.child(
                kit::list_row(
                    element_id("anim-graph-doc", &document.document_id),
                    theme,
                    document.current,
                )
                .child(kit::status_dot(theme.info))
                .child(kit::row_name(label, theme))
                .child(kit::meta_text(
                    if document.unsaved_changes {
                        "unsaved"
                    } else {
                        "saved"
                    },
                    theme,
                ))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(
                        Box::new(OpenGraphDocument {
                            document_id: document_id.clone(),
                        }),
                        cx,
                    );
                }),
            );
        }
        body
    }

    /// Mannequin Fragments section rows: one row per authored fragment, tinted
    /// by whether the selected one resolves.
    fn extend_with_fragment_rows(mut body: gpui::Div, theme: &Theme, cx: &App) -> gpui::Div {
        let authoring = cx.try_global::<EditorMannequinAuthoringCatalog>();
        let Some(authoring) = authoring.filter(|authoring| !authoring.fragments.is_empty()) else {
            return body.child(kit::hint_text(
                "No Mannequin fragment states loaded (animations/mannequin/*.adb.ron).",
                theme,
            ));
        };
        let selected = authoring.selected_fragment_key.clone();
        let resolved = authoring.resolved.clone();
        for fragment in &authoring.fragments {
            let active = selected.as_deref() == Some(fragment.key.as_str());
            let dot = if active {
                match &resolved {
                    Some(resolved) if !resolved.unresolved => theme.success,
                    Some(_) => theme.warning,
                    None => theme.muted_foreground,
                }
            } else {
                theme.muted_foreground
            };
            let fragment_key = fragment.key.clone();
            body = body.child(
                kit::list_row(element_id("anim-fragment", &fragment.key), theme, active)
                    .child(kit::status_dot(dot))
                    .child(kit::row_name(fragment.name.clone(), theme))
                    .child(kit::meta_text(
                        format!("{} opt", fragment.option_count),
                        theme,
                    ))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(
                            Box::new(SelectMannequinFragment {
                                fragment_key: fragment_key.clone(),
                            }),
                            cx,
                        );
                    }),
            );
        }
        body
    }

    fn render_motions_section(theme: &Theme, cx: &App) -> gpui::AnyElement {
        let catalog = cx.try_global::<EditorAnimationPreviewCatalog>();
        let preview = cx.try_global::<EditorMannequinPreview>();
        let blend_catalog = cx.try_global::<EditorBlendSpacePreviewCatalog>();
        let blend_preview = cx.try_global::<EditorBlendSpacePreview>();
        let motions = catalog.map(|c| c.motions.clone()).unwrap_or_default();
        let selected = preview.and_then(|preview| preview.motion_glb.clone());

        if motions.is_empty() && blend_catalog.is_none_or(|c| c.blend_spaces.is_empty()) {
            return kit::empty_state(
                "No animation motions in the asset-pipeline workspace.",
                Some(
                    "*.anim.glb sources tracked by the asset processor build this list".to_owned(),
                ),
                theme,
            )
            .into_any_element();
        }

        let mut body = v_flex().gap_0p5();
        for motion in &motions {
            body = body.child(render_motion_row(
                motion,
                selected.as_deref() == Some(motion.asset_path.as_str()),
                theme,
            ));
        }

        let blend_spaces = blend_catalog
            .map(|c| c.blend_spaces.clone())
            .unwrap_or_default();
        if !blend_spaces.is_empty() {
            let selected_blend = blend_preview.and_then(|preview| preview.bspace_ron_path.clone());
            body = body.child(kit::section_label("Blend Spaces", theme));
            for blend_space in blend_spaces {
                let active = selected_blend.as_deref() == Some(blend_space.asset_path.as_str());
                let bspace_ron_path = blend_space.asset_path.clone();
                body = body.child(
                    kit::list_row(
                        element_id("anim-bspace", &blend_space.asset_path),
                        theme,
                        active,
                    )
                    .child(kit::status_dot(if active {
                        theme.primary
                    } else {
                        theme.info
                    }))
                    .child(kit::row_name(blend_space.name.clone(), theme))
                    .child(kit::meta_text(
                        format!(
                            "{}d · {}",
                            blend_space.dimension_count, blend_space.example_count
                        ),
                        theme,
                    ))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(
                            Box::new(SelectAnimationBlendSpace {
                                bspace_ron_path: bspace_ron_path.clone(),
                            }),
                            cx,
                        );
                    }),
                );
            }
        }
        body.into_any_element()
    }

    fn render_sets_section(theme: &Theme, cx: &App) -> gpui::AnyElement {
        let catalog = cx.try_global::<EditorAnimationPreviewCatalog>();
        let preview = cx.try_global::<EditorMannequinPreview>();
        let motions = catalog.map(|c| c.motions.clone()).unwrap_or_default();
        if motions.is_empty() {
            return kit::empty_state(
                "No motion sets: the motion catalog is empty.",
                Some("sets group motions by their source folder".to_owned()),
                theme,
            )
            .into_any_element();
        }
        let selected = preview.and_then(|preview| preview.motion_glb.clone());

        let mut sets = BTreeMap::<String, Vec<&EditorAnimationMotionData>>::new();
        for motion in &motions {
            sets.entry(motion.set_path.clone())
                .or_default()
                .push(motion);
        }

        let mut body = v_flex().gap_1();
        for (set_path, set_motions) in sets {
            body = body.child(kit::section_label(animation_set_label(&set_path), theme));
            for motion in set_motions {
                body = body.child(render_motion_row(
                    motion,
                    selected.as_deref() == Some(motion.asset_path.as_str()),
                    theme,
                ));
            }
        }
        body.into_any_element()
    }

    fn render_skeleton_section(theme: &Theme, cx: &App) -> gpui::AnyElement {
        let catalog = cx.try_global::<EditorAnimationPreviewCatalog>();
        let preview = cx.try_global::<EditorMannequinPreview>();
        let joints = catalog
            .map(|c| c.skeleton_joints.clone())
            .unwrap_or_default();
        let character = preview.and_then(|preview| preview.character_glb.as_deref().map(path_stem));

        if joints.is_empty() {
            return kit::empty_state(
                "No skeleton loaded.",
                Some(character.map_or_else(
                    || "select a preview character".to_owned(),
                    |character| format!("character: {character}"),
                )),
                theme,
            )
            .into_any_element();
        }

        let mut body = v_flex().gap_0p5().child(kit::status_row(
            character.unwrap_or_else(|| "character".to_owned()),
            format!("{} joints", joints.len()),
            theme,
        ));
        for joint in joints {
            body = body.child(
                h_flex()
                    .h(px(kit::ROW_H))
                    .items_center()
                    .gap_1p5()
                    .child(div().flex_none().w(kit::indent(joint.depth, 10.0, 0.0)))
                    .child(kit::status_dot(if joint.depth == 0 {
                        theme.accent
                    } else {
                        theme.muted_foreground
                    }))
                    .child(kit::row_name(joint.name.clone(), theme)),
            );
        }
        body.into_any_element()
    }

    /// Always-visible bottom Parameters panel: blend-space dimension sliders
    /// (live [`SetAnimationBlendSpaceParameter`]) and Mannequin tag toggles
    /// (live [`SetMannequinTag`]).
    fn render_parameters(theme: &Theme, cx: &Context<'_, Self>) -> impl IntoElement {
        let blend_preview = cx.try_global::<EditorBlendSpacePreview>().cloned();
        let authoring = cx.try_global::<EditorMannequinAuthoringCatalog>().cloned();

        let dimensions: Vec<(EditorBlendSpaceDimensionData, f32)> = blend_preview
            .as_ref()
            .and_then(|preview| {
                preview.document.as_ref().map(|document| {
                    document
                        .dimensions
                        .iter()
                        .enumerate()
                        .map(|(index, dimension)| {
                            let value = preview
                                .param_values
                                .get(index)
                                .copied()
                                .unwrap_or_else(|| dimension.midpoint());
                            (dimension.clone(), value)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();
        let tags = authoring
            .as_ref()
            .map(|a| a.tags.clone())
            .unwrap_or_default();
        let enabled_tags = authoring
            .as_ref()
            .map(|a| a.enabled_tags.clone())
            .unwrap_or_default();

        let mut section = v_flex()
            .flex_none()
            .max_h(px(220.0))
            .overflow_y_scrollbar()
            .p_2()
            .gap_1()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.tab_bar)
            .child(kit::section_label("Parameters", theme));

        if dimensions.is_empty() && tags.is_empty() {
            return section.child(kit::hint_text(
                "No blend-space parameters or Mannequin tags loaded.",
                theme,
            ));
        }

        for (dimension, value) in dimensions {
            section = section.child(Self::render_param_slider(&dimension, value, theme, cx));
        }

        if !tags.is_empty() {
            section = section
                .child(kit::section_label("Mannequin Tags", theme))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1()
                        .children(tags.into_iter().map(|tag| {
                            let enabled = enabled_tags.contains(&tag.name);
                            let tag_name = tag.name.clone();
                            kit::option_chip(enabled, theme)
                                .id(element_id("anim-tag", &tag.name))
                                .cursor_pointer()
                                .child(match tag.group.as_deref() {
                                    Some(group) => format!("{group} · {}", tag.name),
                                    None => tag.name.clone(),
                                })
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    window.dispatch_action(
                                        Box::new(SetMannequinTag {
                                            tag: tag_name.clone(),
                                            enabled: !enabled,
                                        }),
                                        cx,
                                    );
                                })
                        })),
                );
        }
        section
    }

    fn render_param_slider(
        dimension: &EditorBlendSpaceDimensionData,
        value: f32,
        theme: &Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let name = dimension.name.clone();
        let min = dimension.min.min(dimension.max);
        let max = dimension.min.max(dimension.max);
        let fraction = dimension.normalized_value(value);
        let locked = dimension.locked;

        let track = div()
            .id(element_id("anim-param", &name))
            .flex_1()
            .h(px(6.0))
            .rounded(px(3.0))
            .bg(theme.muted)
            .relative()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .h_full()
                    .w(relative(fraction))
                    .rounded(px(3.0))
                    .bg(if locked {
                        theme.muted_foreground
                    } else {
                        theme.accent
                    }),
            );
        let track = if locked {
            track
        } else {
            let down_name = name.clone();
            let move_name = name.clone();
            let bounds_name = name;
            let entity = cx.entity();
            track
                .cursor_pointer()
                .on_prepaint(move |bounds, _, cx| {
                    entity.update(cx, |this, _| {
                        this.param_bounds.insert(bounds_name.clone(), bounds);
                    });
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.param_dragging = Some(down_name.clone());
                        this.scrub_param(&down_name, min, max, event.position, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_move(
                    cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                        if this.param_dragging.as_deref() == Some(move_name.as_str()) {
                            this.scrub_param(&move_name, min, max, event.position, window, cx);
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _window, cx| {
                        this.param_dragging = None;
                        cx.notify();
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, _window, cx| {
                        this.param_dragging = None;
                        cx.notify();
                    }),
                )
        };

        h_flex()
            .h(px(26.0))
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .w(px(78.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(px(11.0))
                    .text_color(theme.foreground)
                    .child(dimension.name.clone()),
            )
            .child(track)
            .child(kit::meta_text(format!("{value:.2}"), theme))
    }
}

fn render_motion_row(
    motion: &EditorAnimationMotionData,
    active: bool,
    theme: &Theme,
) -> impl IntoElement {
    let motion_glb = motion.asset_path.clone();
    kit::list_row(element_id("anim-motion", &motion.asset_path), theme, active)
        .child(kit::status_dot(if active {
            theme.primary
        } else {
            theme.success
        }))
        .child(kit::row_name(motion.name.clone(), theme))
        .when(!motion.events.is_empty(), |this| {
            this.child(kit::tag_chip(
                format!("{} ev", motion.events.len()),
                theme.info,
            ))
        })
        .children(
            motion
                .pipeline_status
                .clone()
                .map(|status| kit::meta_text(status, theme)),
        )
        .child(kit::meta_text(
            motion
                .duration_millis
                .map_or_else(|| "?".to_owned(), format_millis),
            theme,
        ))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(
                Box::new(SelectAnimationMotion {
                    motion_glb: motion_glb.clone(),
                }),
                cx,
            );
        })
}

impl Render for AnimationNavigatorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(placeholder) = render_project_host_connection_placeholder("Animation", cx) {
            return placeholder;
        }
        let theme = cx.theme().clone();

        let body = match self.tab {
            AnimationNavigatorTab::Graph => Self::render_graph_section(&theme, cx),
            AnimationNavigatorTab::Motions => Self::render_motions_section(&theme, cx),
            AnimationNavigatorTab::Sets => Self::render_sets_section(&theme, cx),
            AnimationNavigatorTab::Skeleton => Self::render_skeleton_section(&theme, cx),
        };

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .child(
                kit::panel_toolbar(&theme)
                    .child(kit::panel_title("Animation", &theme))
                    .child(kit::panel_subtitle(
                        "graph · motions · sets · skeleton",
                        &theme,
                    )),
            )
            .child(self.render_tabs(&theme, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_2()
                    .gap_1()
                    .child(body),
            )
            .child(Self::render_parameters(&theme, cx))
            .into_any_element()
    }
}

impl Focusable for AnimationNavigatorPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for AnimationNavigatorPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("account_tree"), "Navigator", kit::TabTone::Default)
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl gpui::EventEmitter<PanelEvent> for AnimationNavigatorPanel {}

// ---------------------------------------------------------------------------
// Center: anim-graph workbench (breadcrumb + graph canvas)
// ---------------------------------------------------------------------------

/// Center workbench: breadcrumb bar over the shared visual graph canvas.
///
/// The canvas hosts the graph controller's document whenever an animation
/// graph is open there (the same embedding the materials workbench uses);
/// explicit state otherwise.
///
/// The motion-event Time View is the bottom-dock
/// [`AnimationMotionTrackPanel`] — not duplicated here.
pub struct AnimationWorkbenchPanel {
    focus_handle: FocusHandle,
    graph_canvas: Entity<VisualGraphPanel>,
}

impl AnimationWorkbenchPanel {
    pub const NAME: &'static str = "animation-workbench";

    pub fn init(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let graph_canvas = cx.new(|cx| VisualGraphPanel::init(window, cx));
        Self {
            focus_handle: cx.focus_handle(),
            graph_canvas,
        }
    }

    /// Workbench body when no animation graph document is open: the explicit
    /// empty state, plus the counts that say what the preview *is* driven by.
    fn render_no_graph_body(
        selected_motion: Option<&EditorAnimationMotionData>,
        theme: &Theme,
        cx: &App,
    ) -> gpui::AnyElement {
        let catalog = cx.try_global::<EditorAnimationPreviewCatalog>();
        let authoring = cx.try_global::<EditorMannequinAuthoringCatalog>();
        let fragment_count = authoring.map_or(0, |a| a.fragments.len());
        let motion_count = catalog.map_or(0, |c| c.motions.len());
        let resolved = authoring.and_then(|a| a.resolved.clone());
        v_flex()
            .flex_1()
            .min_h_0()
            .p_4()
            .gap_2()
            .child(kit::empty_state(
                "No animation graph document is open.",
                Some(
                    "Mannequin fragments and blend spaces drive the preview; a registered \
                     animation graph type is an explicit follow-up seam."
                        .to_owned(),
                ),
                theme,
            ))
            .child(kit::status_row("Motions", motion_count.to_string(), theme))
            .child(kit::status_row(
                "Mannequin fragments",
                fragment_count.to_string(),
                theme,
            ))
            .child(kit::status_row(
                "Selected motion",
                selected_motion.map_or_else(|| "none".to_owned(), |motion| motion.name.clone()),
                theme,
            ))
            .children(resolved.map(|resolved| {
                kit::status_row(
                    "Resolved fragment",
                    match (&resolved.animation_ref, resolved.unresolved) {
                        (Some(animation_ref), false) => animation_ref.clone(),
                        (Some(animation_ref), true) => {
                            format!("{animation_ref} (unresolved)")
                        }
                        (None, _) => resolved
                            .reason
                            .clone()
                            .unwrap_or_else(|| "unresolved".to_owned()),
                    },
                    theme,
                )
            }))
            .into_any_element()
    }
}
impl Render for AnimationWorkbenchPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(placeholder) = render_project_host_connection_placeholder("Animation", cx) {
            return placeholder;
        }
        let theme = cx.theme().clone();
        let graph = cx.try_global::<EditorGraphDocumentProjection>();
        let animation_document = graph
            .and_then(|graph| graph.document.as_ref())
            .filter(|document| is_animation_graph_type(&document.graph_type));
        let hosts_graph = animation_document.is_some();
        let breadcrumb = animation_document.map_or_else(
            || "no animation graph open".to_owned(),
            |document| {
                format!(
                    "{} · {}",
                    path_stem(&document.document_id),
                    document.graph_type
                )
            },
        );

        let catalog = cx.try_global::<EditorAnimationPreviewCatalog>();
        let preview = cx.try_global::<EditorMannequinPreview>();
        let selected_motion = catalog
            .and_then(|catalog| catalog.selected_motion(preview))
            .cloned();

        let toolbar = kit::panel_toolbar(&theme)
            .child(kit::panel_title("Animation Graph", &theme))
            .child(kit::panel_subtitle(breadcrumb, &theme))
            .when(hosts_graph, |this| {
                this.child(kit::toolbar_icon_button(
                    "anim-save-graph",
                    gpui_component::IconName::Save,
                    "Save animation graph",
                    SaveGraphDocument,
                    &theme,
                ))
            });

        let body: gpui::AnyElement = if hosts_graph {
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_hidden()
                .child(self.graph_canvas.clone())
                .into_any_element()
        } else {
            Self::render_no_graph_body(selected_motion.as_ref(), &theme, cx)
        };

        let position = preview.map_or(0, |preview| preview.position_millis);
        let duration = selected_motion
            .as_ref()
            .and_then(|motion| motion.duration_millis)
            .unwrap_or(0);
        let footer = kit::count_footer(&theme).child(format!(
            "Motion Event Tracks: bottom dock · {} · {position} / {duration} ms",
            selected_motion
                .as_ref()
                .map_or("no motion selected", |motion| motion.name.as_str()),
        ));

        v_flex()
            .size_full()
            .bg(theme.background)
            .child(toolbar)
            .child(body)
            .child(footer)
            .into_any_element()
    }
}

impl Focusable for AnimationWorkbenchPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for AnimationWorkbenchPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("directions_run"), "Animation", kit::TabTone::Default)
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl gpui::EventEmitter<PanelEvent> for AnimationWorkbenchPanel {}

// ---------------------------------------------------------------------------
// Right dock: mannequin viewport + attributes
// ---------------------------------------------------------------------------

/// Right dock: mannequin preview surface over motion/character attributes.
///
/// The live in-process mannequin preview surface (shared
/// [`InProcessViewportSurface`], Animation-owned so the frame pump applies
/// [`EditorMannequinPreview`] / blend-space previews instead of the authored
/// scene) with transport overlays, over the selected motion/character
/// attribute rows.
pub struct AnimationMannequinPanel {
    focus_handle: FocusHandle,
    surface: Entity<InProcessViewportSurface>,
}

impl AnimationMannequinPanel {
    pub const NAME: &'static str = "animation-mannequin";

    pub fn init(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            surface: cx
                .new(|_| InProcessViewportSurface::new(ViewportSurfaceOwner::Animation, false)),
        }
    }

    fn render_surface(&self, theme: &Theme, cx: &App) -> impl IntoElement {
        let render_status = cx.try_global::<EditorViewportRenderStatus>().cloned();
        let render_state = render_status
            .as_ref()
            .map_or(EditorViewportRenderStateData::Waiting, |status| {
                status.state
            });
        let preview = cx.try_global::<EditorMannequinPreview>().cloned();
        let catalog = cx.try_global::<EditorAnimationPreviewCatalog>().cloned();
        let motion = catalog
            .as_ref()
            .and_then(|catalog| catalog.selected_motion(preview.as_ref()).cloned());

        let content: gpui::AnyElement = match render_state {
            EditorViewportRenderStateData::EditorCompositionSurface => {
                self.surface.clone().into_any_element()
            }
            EditorViewportRenderStateData::Failed => kit::empty_state(
                "In-process viewport renderer unavailable.",
                render_status.and_then(|status| status.diagnostic),
                theme,
            )
            .into_any_element(),
            _ => kit::empty_state("Viewport renderer starting…", None, theme).into_any_element(),
        };

        let playing = preview.as_ref().is_some_and(|preview| preview.playing);
        let looping = preview.as_ref().is_some_and(|preview| preview.looping);
        let can_transport = motion.is_some();
        let position = preview
            .as_ref()
            .map_or(0, |preview| preview.position_millis);
        let duration = motion
            .as_ref()
            .and_then(|motion| motion.duration_millis)
            .unwrap_or(0);
        let motion_label = motion.as_ref().map_or_else(
            || "no motion selected".to_owned(),
            |motion| motion.name.clone(),
        );
        let character_label = preview
            .as_ref()
            .and_then(|preview| preview.character_glb.as_deref().map(path_stem))
            .unwrap_or_else(|| "no character".to_owned());

        div()
            .flex_none()
            .h(px(280.0))
            .relative()
            .overflow_hidden()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.background)
            // Publish the surface geometry every paint (in every state) so the
            // viewport host wakes and marks the surface Animation-owned before
            // its composition hole first renders.
            .on_prepaint(move |bounds, window, cx| {
                let scale_factor = window.scale_factor();
                let window_id = window.window_handle().window_id().as_u64();
                cx.default_global::<EditorViewportPanelFrame>().publish(
                    bounds,
                    scale_factor,
                    window_id,
                    ViewportSurfaceOwner::Animation,
                );
            })
            .child(content)
            .child(
                kit::viewport_overlay(theme)
                    .absolute()
                    .top(px(8.0))
                    .left(px(8.0))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .child(kit::status_dot(if playing {
                                theme.success
                            } else {
                                theme.accent
                            }))
                            .child(motion_label),
                    ),
            )
            .child(
                kit::viewport_overlay(theme)
                    .absolute()
                    .top(px(8.0))
                    .right(px(8.0))
                    .child(character_label),
            )
            .child(render_mannequin_transport_overlay(
                playing,
                looping,
                can_transport,
                position,
                duration,
                theme,
            ))
    }

    fn render_attributes(theme: &Theme, cx: &App) -> impl IntoElement {
        let preview = cx.try_global::<EditorMannequinPreview>().cloned();
        let catalog = cx.try_global::<EditorAnimationPreviewCatalog>().cloned();
        let authoring = cx.try_global::<EditorMannequinAuthoringCatalog>().cloned();
        let blend_preview = cx.try_global::<EditorBlendSpacePreview>().cloned();
        let motion = catalog
            .as_ref()
            .and_then(|catalog| catalog.selected_motion(preview.as_ref()).cloned());

        let mut body = v_flex().p_2p5().gap_1p5();

        body = Self::extend_with_motion_attributes(body, motion.as_ref(), theme);
        body =
            Self::extend_with_character_attributes(body, preview.as_ref(), catalog.as_ref(), theme);
        body = Self::extend_with_mannequin_attributes(body, authoring.as_ref(), theme);
        body = Self::extend_with_blend_space_attributes(body, blend_preview.as_ref(), theme);
        body.flex_1().min_h_0().overflow_y_scrollbar()
    }

    /// Motion section: the selected motion's name, duration, channel and
    /// event counts, source asset, and pipeline status.
    fn extend_with_motion_attributes(
        mut body: gpui::Div,
        motion: Option<&EditorAnimationMotionData>,
        theme: &Theme,
    ) -> gpui::Div {
        body = body.child(kit::section_label("Motion", theme));
        body = match motion {
            Some(motion) => body
                .child(kit::status_row("Name", motion.name.clone(), theme))
                .child(kit::status_row(
                    "Duration",
                    motion
                        .duration_millis
                        .map_or_else(|| "unknown".to_owned(), format_millis),
                    theme,
                ))
                .child(kit::status_row(
                    "Channels",
                    motion.channel_count.to_string(),
                    theme,
                ))
                .child(kit::status_row(
                    "Joint targets",
                    motion.joint_targets.len().to_string(),
                    theme,
                ))
                .child(kit::status_row(
                    "Events",
                    motion.events.len().to_string(),
                    theme,
                ))
                .child(kit::status_row("Asset", motion.asset_path.clone(), theme))
                .children(
                    motion
                        .pipeline_status
                        .clone()
                        .map(|status| kit::status_row("Pipeline", status, theme)),
                ),
            None => body.child(kit::hint_text("No motion selected.", theme)),
        };
        body
    }

    /// Character section: the preview character asset and its joint count.
    fn extend_with_character_attributes(
        mut body: gpui::Div,
        preview: Option<&EditorMannequinPreview>,
        catalog: Option<&EditorAnimationPreviewCatalog>,
        theme: &Theme,
    ) -> gpui::Div {
        body = body.child(kit::section_label("Character", theme));
        body = match preview
            .as_ref()
            .and_then(|preview| preview.character_glb.clone())
        {
            Some(character_glb) => body
                .child(kit::status_row("Asset", character_glb, theme))
                .child(kit::status_row(
                    "Skeleton joints",
                    catalog
                        .as_ref()
                        .map_or(0, |catalog| catalog.skeleton_joints.len())
                        .to_string(),
                    theme,
                )),
            None => body.child(kit::hint_text("No preview character selected.", theme)),
        };
        body
    }

    /// Mannequin section: the selected fragment, its option count, and what
    /// the fragment resolved to.
    fn extend_with_mannequin_attributes(
        mut body: gpui::Div,
        authoring: Option<&EditorMannequinAuthoringCatalog>,
        theme: &Theme,
    ) -> gpui::Div {
        body = body.child(kit::section_label("Mannequin", theme));
        body = match authoring
            .as_ref()
            .and_then(|authoring| authoring.selected_fragment())
        {
            Some(fragment) => {
                let resolved = authoring.as_ref().and_then(|a| a.resolved.clone());
                body.child(kit::status_row("Fragment", fragment.name.clone(), theme))
                    .child(kit::status_row(
                        "Options",
                        fragment.option_count.to_string(),
                        theme,
                    ))
                    .children(resolved.map(|resolved| {
                        kit::status_row(
                            "Resolved",
                            match (&resolved.animation_ref, resolved.unresolved) {
                                (Some(animation_ref), false) => animation_ref.clone(),
                                _ => resolved
                                    .reason
                                    .clone()
                                    .unwrap_or_else(|| "unresolved".to_owned()),
                            },
                            theme,
                        )
                    }))
            }
            None => body.child(kit::hint_text("No Mannequin fragment selected.", theme)),
        };
        body
    }

    /// Blend Space section: the source document and its current parameters.
    fn extend_with_blend_space_attributes(
        mut body: gpui::Div,
        blend_preview: Option<&EditorBlendSpacePreview>,
        theme: &Theme,
    ) -> gpui::Div {
        body = body.child(kit::section_label("Blend Space", theme));
        body = match blend_preview
            .as_ref()
            .and_then(|preview| preview.bspace_ron_path.clone())
        {
            Some(bspace_ron_path) => {
                let params = blend_preview
                    .as_ref()
                    .map(|preview| {
                        preview
                            .param_values
                            .iter()
                            .map(|value| format!("{value:.2}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                body.child(kit::status_row("Source", bspace_ron_path, theme))
                    .child(kit::status_row(
                        "Parameters",
                        if params.is_empty() {
                            "—".to_owned()
                        } else {
                            params
                        },
                        theme,
                    ))
            }
            None => body.child(kit::hint_text("No blend space selected.", theme)),
        };
        body
    }
}

fn transport_chip(
    id: &'static str,
    label: &'static str,
    active: bool,
    enabled: bool,
    theme: &Theme,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(8.0))
        .py(px(3.0))
        .rounded(px(4.0))
        .cursor_pointer()
        .bg(if active {
            theme.primary.opacity(0.2)
        } else {
            theme.secondary
        })
        .text_size(gpui::Rems(0.58))
        .text_color(if active {
            theme.primary
        } else {
            theme.foreground
        })
        .hover(|this| this.bg(theme.list_hover))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            if enabled {
                on_click(window, cx);
            }
        })
        .child(label)
}

/// Transport overlay pinned to the bottom of the mannequin preview: play/pause,
/// stop, loop, and the position readout.
fn render_mannequin_transport_overlay(
    playing: bool,
    looping: bool,
    can_transport: bool,
    position: u32,
    duration: u32,
    theme: &Theme,
) -> impl IntoElement {
    h_flex()
        .absolute()
        .bottom(px(8.0))
        .left(px(8.0))
        .right(px(8.0))
        .justify_center()
        .child(
            kit::viewport_overlay(theme).child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(transport_chip(
                        "anim-mannequin-play",
                        if playing { "PAUSE" } else { "PLAY" },
                        playing,
                        can_transport,
                        theme,
                        move |window, cx| {
                            window.dispatch_action(
                                Box::new(SetAnimationPreviewPlaying { playing: !playing }),
                                cx,
                            );
                        },
                    ))
                    .child(transport_chip(
                        "anim-mannequin-stop",
                        "STOP",
                        false,
                        can_transport,
                        theme,
                        |window, cx| {
                            window.dispatch_action(Box::new(StopAnimationPreview), cx);
                        },
                    ))
                    .child(transport_chip(
                        "anim-mannequin-loop",
                        if looping { "LOOP" } else { "ONCE" },
                        looping,
                        can_transport,
                        theme,
                        move |window, cx| {
                            window.dispatch_action(
                                Box::new(SetAnimationPreviewLoop { looping: !looping }),
                                cx,
                            );
                        },
                    ))
                    .child(div().child(format!("{position} / {duration} ms"))),
            ),
        )
}

impl Render for AnimationMannequinPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(placeholder) = render_project_host_connection_placeholder("Mannequin", cx) {
            return placeholder;
        }
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .child(
                kit::panel_toolbar(&theme)
                    .child(kit::panel_title("Mannequin", &theme))
                    .child(kit::panel_subtitle("in-process preview", &theme)),
            )
            .child(self.render_surface(&theme, cx))
            .child(Self::render_attributes(&theme, cx))
            .into_any_element()
    }
}

impl Focusable for AnimationMannequinPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for AnimationMannequinPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("accessibility_new"), "Mannequin", kit::TabTone::Muted)
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl gpui::EventEmitter<PanelEvent> for AnimationMannequinPanel {}

// ---------------------------------------------------------------------------
// Bottom dock: motion event tracks (Time View)
// ---------------------------------------------------------------------------

/// Bottom-dock motion event track panel for the animation workbench profile.
///
/// Display + transport are hooked to `EditorMannequinPreview` via:
/// `ScrubAnimationPreview`, `SetAnimationPreviewPlaying`,
/// `StopAnimationPreview`, `SetAnimationPreviewLoop`. While playing, the
/// viewport host reads the bevy `AnimationPlayer` seek position back into
/// `position_millis`, so the playhead advances; scrubbing stays authoritative
/// when paused.
pub struct AnimationMotionTrackPanel {
    focus_handle: FocusHandle,
    scrub_bounds: Bounds<Pixels>,
    scrubbing: bool,
}

impl AnimationMotionTrackPanel {
    pub const NAME: &'static str = "animation-motion-tracks";

    pub fn init(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            scrub_bounds: Bounds::default(),
            scrubbing: false,
        }
    }

    fn duration_millis(cx: &App) -> u32 {
        let preview = cx.try_global::<EditorMannequinPreview>();
        cx.try_global::<EditorAnimationPreviewCatalog>()
            .and_then(|catalog| {
                catalog
                    .selected_motion(preview)
                    .and_then(|motion| motion.duration_millis)
            })
            .unwrap_or(0)
            .max(1)
    }

    fn scrub_to_position(
        &self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let width = f32::from(self.scrub_bounds.size.width).max(1.0);
        let x = f32::from(position.x - self.scrub_bounds.origin.x).clamp(0.0, width);
        let fraction = x / width;
        let duration = Self::duration_millis(cx);
        let position_millis = kit::scaled(fraction, duration);
        window.dispatch_action(Box::new(ScrubAnimationPreview { position_millis }), cx);
        cx.notify();
    }
}

impl Render for AnimationMotionTrackPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(placeholder) = render_project_host_connection_placeholder("Motion Tracks", cx) {
            return placeholder;
        }

        let theme = cx.theme().clone();
        let catalog = cx.try_global::<EditorAnimationPreviewCatalog>().cloned();
        let preview = cx.try_global::<EditorMannequinPreview>().cloned();
        let motion = catalog
            .as_ref()
            .and_then(|c| c.selected_motion(preview.as_ref()).cloned());
        let duration = motion
            .as_ref()
            .and_then(|m| m.duration_millis)
            .unwrap_or(0)
            .max(1);
        let position = preview.as_ref().map_or(0, |p| p.position_millis);
        let progress = kit::ratio(position, duration).clamp(0.0, 1.0);
        let playing = preview.as_ref().is_some_and(|p| p.playing);
        let looping = preview.as_ref().is_some_and(|p| p.looping);
        let events = motion
            .as_ref()
            .map(|m| m.events.clone())
            .unwrap_or_default();
        let motion_name = motion
            .as_ref()
            .map_or_else(|| "No motion selected".to_owned(), |m| m.name.clone());
        let can_transport = motion.is_some();

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .child(render_motion_track_header(
                &motion_name,
                playing,
                looping,
                can_transport,
                position,
                duration,
                &theme,
            ))
            .child(render_motion_track_body(
                &events, progress, duration, &theme, cx,
            ))
            .into_any_element()
    }
}

/// Header strip over the motion event tracks: the title, the motion name,
/// the transport chips, and the position readout.
fn render_motion_track_header(
    motion_name: &str,
    playing: bool,
    looping: bool,
    can_transport: bool,
    position: u32,
    duration: u32,
    theme: &Theme,
) -> impl IntoElement {
    h_flex()
        .flex_none()
        .items_center()
        .gap(px(10.0))
        .px(px(12.0))
        .h(px(30.0))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(gpui::Rems(0.68))
                .font_semibold()
                .text_color(theme.foreground)
                .child("Motion Event Tracks"),
        )
        .child(
            div()
                .text_size(gpui::Rems(0.6))
                .font_family(theme.mono_font_family.clone())
                .text_color(theme.muted_foreground)
                .child(motion_name.to_owned()),
        )
        .child(render_motion_track_transport(
            playing,
            looping,
            can_transport,
            theme,
        ))
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(gpui::Rems(0.58))
                .text_color(theme.foreground)
                .child(format!("{position} / {duration} ms")),
        )
}

/// The motion-track transport chips: play/pause, stop, and loop.
fn render_motion_track_transport(
    playing: bool,
    looping: bool,
    can_transport: bool,
    theme: &Theme,
) -> impl IntoElement {
    h_flex()
        .ml_auto()
        .items_center()
        .gap(px(4.0))
        .child(
            div()
                .id("anim-track-play")
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .bg(if playing {
                    theme.success.opacity(0.2)
                } else {
                    theme.secondary
                })
                .text_size(gpui::Rems(0.58))
                .text_color(if playing {
                    theme.success
                } else {
                    theme.foreground
                })
                .hover(|this| this.bg(theme.list_hover))
                .on_mouse_down(MouseButton::Left, {
                    move |_, window, cx| {
                        if !can_transport {
                            return;
                        }
                        window.dispatch_action(
                            Box::new(SetAnimationPreviewPlaying { playing: !playing }),
                            cx,
                        );
                    }
                })
                .child(if playing { "PAUSE" } else { "PLAY" }),
        )
        .child(
            div()
                .id("anim-track-stop")
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .bg(theme.secondary)
                .text_size(gpui::Rems(0.58))
                .text_color(theme.foreground)
                .hover(|this| this.bg(theme.list_hover))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    if !can_transport {
                        return;
                    }
                    window.dispatch_action(Box::new(StopAnimationPreview), cx);
                })
                .child("STOP"),
        )
        .child(
            div()
                .id("anim-track-loop")
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .bg(if looping {
                    theme.primary.opacity(0.2)
                } else {
                    theme.secondary
                })
                .text_size(gpui::Rems(0.58))
                .text_color(if looping {
                    theme.primary
                } else {
                    theme.muted_foreground
                })
                .hover(|this| this.bg(theme.list_hover))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    if !can_transport {
                        return;
                    }
                    window.dispatch_action(
                        Box::new(SetAnimationPreviewLoop { looping: !looping }),
                        cx,
                    );
                })
                .child(if looping { "LOOP" } else { "ONCE" }),
        )
}

/// Body under the header: the scrub bar and one row per motion event.
fn render_motion_track_body(
    events: &[crate::panels::viewport::EditorAnimationEventData],
    progress: f32,
    duration: u32,
    theme: &Theme,
    cx: &Context<'_, AnimationMotionTrackPanel>,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .p(px(10.0))
        .gap(px(8.0))
        .child(render_motion_scrub_bar(progress, theme, cx))
        .child(render_motion_event_rows(events, duration, theme))
}

/// The interactive scrub bar over the motion timeline; a press or drag
/// dispatches [`ScrubAnimationPreview`] at the pressed fraction.
fn render_motion_scrub_bar(
    progress: f32,
    theme: &Theme,
    cx: &Context<'_, AnimationMotionTrackPanel>,
) -> impl IntoElement {
    // Interactive timeline scrub bar → ScrubAnimationPreview
    div()
        .id("anim-timeline-scrub")
        .w_full()
        .h(px(16.0))
        .rounded(px(5.0))
        .bg(theme.muted)
        .relative()
        .cursor_pointer()
        .on_prepaint({
            let entity = cx.entity();
            move |bounds, _, cx| {
                entity.update(cx, |this, _| {
                    this.scrub_bounds = bounds;
                });
            }
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                this.scrubbing = true;
                this.scrub_to_position(event.position, window, cx);
                cx.stop_propagation();
            }),
        )
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
            if this.scrubbing {
                this.scrub_to_position(event.position, window, cx);
                cx.stop_propagation();
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _event, _window, cx| {
                this.scrubbing = false;
                cx.notify();
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|this, _event, _window, cx| {
                this.scrubbing = false;
                cx.notify();
            }),
        )
        .child(
            div()
                .absolute()
                .top(px(3.0))
                .left_0()
                .h(px(10.0))
                .w(gpui::relative(progress))
                .rounded(px(5.0))
                .bg(theme.primary),
        )
        .child(
            div()
                .absolute()
                .top(px(0.0))
                .left(gpui::relative(progress))
                .ml(px(-4.0))
                .w(px(8.0))
                .h(px(16.0))
                .rounded(px(2.0))
                .bg(theme.danger)
                .id("anim-playhead"),
        )
}

/// One row per motion event: name, position marker, span, and parameter.
fn render_motion_event_rows(
    events: &[crate::panels::viewport::EditorAnimationEventData],
    duration: u32,
    theme: &Theme,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .gap(px(4.0))
        .children(if events.is_empty() {
            vec![
                div()
                    .text_size(gpui::Rems(0.65))
                    .text_color(theme.muted_foreground)
                    .child("No animation events on the selected motion.")
                    .into_any_element(),
            ]
        } else {
            events
                .iter()
                .map(|event| {
                    let start = event.time_millis;
                    let end = event.end_time_millis;
                    let duration = duration.max(1);
                    let start_f = kit::ratio(start, duration).clamp(0.0, 1.0);
                    h_flex()
                        .id(format!("anim-event-{}", event.name))
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(4.0))
                        .rounded(px(4.0))
                        .bg(theme.secondary)
                        .cursor_pointer()
                        .hover(|this| this.bg(theme.list_hover))
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(ScrubAnimationPreview {
                                    position_millis: start,
                                }),
                                cx,
                            );
                        })
                        .child(
                            div()
                                .w(px(120.0))
                                .text_size(gpui::Rems(0.62))
                                .text_color(theme.foreground)
                                .child(event.name.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .h(px(4.0))
                                .rounded(px(2.0))
                                .bg(theme.muted)
                                .relative()
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left(gpui::relative(start_f))
                                        .w(px(6.0))
                                        .h_full()
                                        .rounded(px(2.0))
                                        .bg(theme.info),
                                ),
                        )
                        .child(
                            div()
                                .font_family(theme.mono_font_family.clone())
                                .text_size(gpui::Rems(0.58))
                                .text_color(theme.muted_foreground)
                                .child(format!("{start}–{end} ms")),
                        )
                        .child(
                            div()
                                .text_size(gpui::Rems(0.58))
                                .text_color(theme.info)
                                .child(event.parameter.clone()),
                        )
                        .into_any_element()
                })
                .collect()
        })
}

impl Focusable for AnimationMotionTrackPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for AnimationMotionTrackPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("movie"), "Motion Tracks", kit::TabTone::Success)
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl gpui::EventEmitter<PanelEvent> for AnimationMotionTrackPanel {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_graph_type_detection_matches_animation_ids_only() {
        assert!(is_animation_graph_type("azoth.animation.graph"));
        assert!(is_animation_graph_type("AnimGraph"));
        assert!(!is_animation_graph_type("azoth.material.graph"));
        assert!(!is_animation_graph_type("azoth.script.graph"));
    }

    #[test]
    fn motion_set_labels_group_by_folder_stem_with_root_fallback() {
        assert_eq!(animation_set_label("animations/locomotion"), "locomotion");
        assert_eq!(animation_set_label(""), "root");
        assert_eq!(animation_set_label("  "), "root");
    }
}
