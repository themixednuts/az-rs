//! Visual graph panel.
//!
//! Renders editor-owned visual graph projection data. The panel deliberately
//! knows nothing about project-host, Cap'n Proto, `VisualGraphDocument`, or
//! graph validation; editor-core owns those boundaries and publishes
//! `EditorGraphDocumentProjection` for this panel to display.

use az_core::reflect::{ReflectedValueEncoding, ReflectedValueEnvelope};
use az_editor_inspector::{
    ReflectedScalar, ReflectedValue, WidgetFamily, decode_standalone_reflected_value,
    standalone_reflected_widget_family,
};
use gpui::{
    App, AppContext, Bounds, ClickEvent, Context, Entity, FocusHandle, Focusable, Global,
    InteractiveElement, IntoElement, KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, Pixels, Render, ScrollDelta,
    ScrollWheelEvent, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, canvas,
    div, point, prelude::FluentBuilder, px,
};
use gpui_component::dock::Panel;
use gpui_component::{
    ActiveTheme, ElementExt, Sizable, StyledExt, h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, PopupMenu},
    scroll::ScrollableElement,
    v_flex,
};

use crate::panels::kit;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EditorGraphDocumentProjection {
    pub document: Option<GraphDocumentProjectionData>,
    pub status_error: Option<String>,
    pub layout_job: Option<GraphLayoutJobData>,
    pub build_status: Option<GraphBuildStatusProjectionData>,
    pub graph_documents: GraphDocumentListProjectionData,
    pub creation_catalog: GraphCreationCatalogProjectionData,
    pub node_palette: GraphNodePaletteProjectionData,
}

impl EditorGraphDocumentProjection {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            document: None,
            status_error: None,
            layout_job: None,
            build_status: None,
            graph_documents: GraphDocumentListProjectionData::new(Vec::new()),
            creation_catalog: GraphCreationCatalogProjectionData::new(Vec::new()),
            node_palette: GraphNodePaletteProjectionData::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn document(document: GraphDocumentProjectionData) -> Self {
        Self {
            document: Some(document),
            status_error: None,
            layout_job: None,
            build_status: None,
            graph_documents: GraphDocumentListProjectionData::default(),
            creation_catalog: GraphCreationCatalogProjectionData::default(),
            node_palette: GraphNodePaletteProjectionData::default(),
        }
    }

    #[must_use]
    pub fn error(error: impl Into<String>) -> Self {
        Self {
            document: None,
            status_error: Some(error.into()),
            layout_job: None,
            build_status: None,
            graph_documents: GraphDocumentListProjectionData::default(),
            creation_catalog: GraphCreationCatalogProjectionData::default(),
            node_palette: GraphNodePaletteProjectionData::default(),
        }
    }

    #[must_use]
    pub fn with_graph_documents(
        mut self,
        graph_documents: GraphDocumentListProjectionData,
    ) -> Self {
        self.graph_documents = graph_documents;
        self
    }

    #[must_use]
    pub fn with_creation_catalog(mut self, catalog: GraphCreationCatalogProjectionData) -> Self {
        self.creation_catalog = catalog;
        self
    }

    #[must_use]
    pub fn with_node_palette(mut self, palette: GraphNodePaletteProjectionData) -> Self {
        self.node_palette = palette;
        self
    }

    #[must_use]
    pub fn with_build_status(
        mut self,
        build_status: Option<GraphBuildStatusProjectionData>,
    ) -> Self {
        self.build_status = build_status;
        self
    }
}

impl Global for EditorGraphDocumentProjection {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphDocumentListProjectionData {
    pub documents: Vec<GraphDocumentListItemProjectionData>,
}

impl GraphDocumentListProjectionData {
    #[must_use]
    pub const fn new(documents: Vec<GraphDocumentListItemProjectionData>) -> Self {
        Self { documents }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphDocumentListItemProjectionData {
    pub document_id: String,
    pub graph_type: String,
    pub source_path: String,
    pub revision: u64,
    pub saved_revision: Option<u64>,
    pub unsaved_changes: bool,
    pub loaded: bool,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphBuildStatusProjectionData {
    pub document_id: String,
    pub source_path: String,
    pub asset_guid: String,
    pub source_status: GraphBuildSourceStatusData,
    pub entry_id: i64,
    pub content_hash: String,
    pub latest_job: Option<GraphBuildJobProjectionData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphBuildSourceStatusData {
    Clean,
    Added,
    Modified,
    Deleted,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphBuildJobProjectionData {
    pub job_id: i64,
    pub attempt_id: Option<i64>,
    pub job_key: String,
    pub platform: String,
    pub ordinal: Option<i64>,
    pub status: GraphBuildJobStatusData,
    pub error_count: i64,
    pub warning_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphBuildJobStatusData {
    Queued,
    Leased,
    Succeeded,
    Failed,
    Abandoned,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphCreationCatalogProjectionData {
    pub graph_types: Vec<GraphTypeCreationProjectionData>,
}

impl GraphCreationCatalogProjectionData {
    #[must_use]
    pub const fn new(graph_types: Vec<GraphTypeCreationProjectionData>) -> Self {
        Self { graph_types }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.graph_types.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphTypeCreationProjectionData {
    pub graph_type: String,
    pub label: String,
    pub category: String,
    pub default_path_prefix: String,
    pub default_extension: String,
    pub compiler_backend: Option<GraphCompilerBackendProjectionData>,
    pub runtime_product_asset_type: Option<String>,
    pub runtime_product_kind: Option<String>,
    pub runtime_product_streamable: Option<bool>,
    pub runtime_product_diffable_chunks: Option<bool>,
    pub runtime_execution_strategy: Option<GraphRuntimeExecutionStrategyProjectionData>,
    pub runtime_compiled: bool,
    pub editor_interpreted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCompilerBackendProjectionData {
    pub id: String,
    pub kind: GraphCompilerBackendKindProjectionData,
    pub capability_markers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphCompilerBackendKindProjectionData {
    GeneratedRust {
        package: String,
        entry_symbol: String,
        abi: GraphGeneratedRustAbiProjectionData,
    },
    PackedIr {
        ir_schema: String,
    },
    ShaderPipeline {
        pipeline_kind: String,
    },
    External {
        kind: String,
        locator: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphGeneratedRustAbiProjectionData {
    ContextSchedule,
    TypedDataflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphRuntimeExecutionStrategyProjectionData {
    PackedIr,
    AotCompiledCode {
        language: String,
        package: String,
        entry_symbol: String,
        context_type: String,
    },
    HotReloadedCompiledModule {
        abi: String,
        entry_symbol: String,
    },
    ShaderPipeline {
        pipeline_kind: String,
    },
    External {
        kind: String,
        locator: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphNodePaletteProjectionData {
    pub nodes: Vec<GraphNodePaletteItemData>,
}

impl GraphNodePaletteProjectionData {
    #[must_use]
    pub const fn new(nodes: Vec<GraphNodePaletteItemData>) -> Self {
        Self { nodes }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNodePaletteItemData {
    pub node_type: String,
    pub version: u32,
    pub label: String,
    pub category: String,
    pub description: Option<String>,
    pub input_count: usize,
    pub output_count: usize,
    pub default_input_count: usize,
    pub runtime_bound: bool,
    pub runtime_binding: Option<GraphNodeRuntimeBindingProjectionData>,
    pub source_link_count: usize,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphNodeRuntimeBindingProjectionData {
    RustSymbol {
        package: String,
        symbol: String,
        call_abi: GraphRustNodeCallAbiProjectionData,
    },
    AssetBuilder {
        builder_id: String,
    },
    RuntimeComponent {
        component_type: String,
    },
    External {
        kind: String,
        locator: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphRustNodeCallAbiProjectionData {
    ContextSchedule,
    TypedDataflow {
        parameter_count: usize,
        input_parameter_count: usize,
        by_value_parameter_count: usize,
        mutable_parameter_count: usize,
        output_count: usize,
        result: GraphRustCallResultProjectionData,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphRustCallResultProjectionData {
    Plain,
    Result,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphDocumentProjectionData {
    pub document_id: String,
    pub graph_type: String,
    pub graph_type_info: Option<GraphTypeCreationProjectionData>,
    pub revision: u64,
    pub saved_revision: Option<u64>,
    pub unsaved_changes: bool,
    pub catalog_version: u32,
    pub nodes: Vec<GraphNodeProjectionData>,
    pub connections: Vec<GraphConnectionProjectionData>,
    pub comments: Vec<GraphCommentProjectionData>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphCommentProjectionData {
    pub comment_id: String,
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphNodeProjectionData {
    pub node_id: String,
    pub node_type: String,
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub selected: bool,
    pub source_links: Vec<GraphNodeSourceLinkProjectionData>,
    pub ports: Vec<GraphPortProjectionData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNodeSourceLinkProjectionData {
    pub package: Option<String>,
    pub module_path: Option<String>,
    pub symbol_path: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub docs_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphPortProjectionData {
    pub port_id: u32,
    pub name: String,
    pub direction: GraphPortDirectionData,
    pub side: GraphPortSideData,
    pub value: Option<GraphInputValueProjectionData>,
    /// Node-local x coordinate measured from the node's top-left corner.
    pub x: f32,
    /// Node-local y coordinate measured from the node's top-left corner.
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphInputValueProjectionData {
    pub schema_type: String,
    pub current_value: Option<ReflectedValueEnvelope>,
    pub default_value: Option<ReflectedValueEnvelope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphPortDirectionData {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphPortSideData {
    North,
    East,
    South,
    West,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphConnectionProjectionData {
    pub connection_id: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub points: Vec<GraphPointProjectionData>,
    pub route_anchors: Vec<GraphRouteAnchorProjectionData>,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphPointProjectionData {
    pub x: f32,
    pub y: f32,
}

impl GraphPointProjectionData {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphRouteAnchorProjectionData {
    pub anchor_id: String,
    pub kind: GraphRouteAnchorKindData,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphRouteAnchorKindData {
    PortEndpoint,
    UserWaypoint,
    SolverWaypoint,
    Junction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLayoutJobData {
    pub phase: GraphLayoutJobPhaseData,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphLayoutJobPhaseData {
    Idle,
    Queued,
    Running,
    Failed,
}

pub struct VisualGraphPanel {
    focus: FocusHandle,
    new_graph_name_input: Entity<InputState>,
    new_graph_name: String,
    selected_node_id: Option<String>,
    selected_comment_id: Option<String>,
    pending_output_port: Option<PendingGraphPortConnection>,
    viewport: GraphViewportState,
    canvas_bounds: Bounds<Pixels>,
    space_pan_key_held: bool,
    active_pan_drag: Option<GraphPanDragState>,
    context_menu_document_position: Option<GraphPointProjectionData>,
    active_node_drag: Option<GraphNodeDragState>,
    active_route_anchor_drag: Option<GraphRouteAnchorDragState>,
    active_comment_drag: Option<GraphCommentDragState>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingGraphPortConnection {
    node_id: String,
    port_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphPortClickData {
    node_id: String,
    port_id: u32,
    direction: GraphPortDirectionData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_field_names)] // from_/to_ node/port naming mirrors the wire request; renaming ripples across many call sites
struct GraphPortConnectionRequest {
    from_node_id: String,
    from_port_id: u32,
    to_node_id: String,
    to_port_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphNodeClickData {
    node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphCommentClickData {
    comment_id: String,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::struct_field_names)] // node_ prefix distinguishes node coords from mouse coords used together
struct GraphNodeDragStartData {
    node_id: String,
    node_x: f32,
    node_y: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct GraphNodeDragState {
    node_id: String,
    start_node_x: f32,
    start_node_y: f32,
    start_mouse_x: f32,
    start_mouse_y: f32,
    preview_x: f32,
    preview_y: f32,
    moved: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct GraphRouteAnchorDragStartData {
    connection_id: String,
    anchor_id: String,
    anchor_x: f32,
    anchor_y: f32,
    draggable: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct GraphRouteAnchorDragState {
    connection_id: String,
    anchor_id: String,
    start_anchor_x: f32,
    start_anchor_y: f32,
    start_mouse_x: f32,
    start_mouse_y: f32,
    preview_x: f32,
    preview_y: f32,
    moved: bool,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::struct_field_names)] // comment_ prefix distinguishes comment geometry from mouse coords used together
struct GraphCommentDragStartData {
    comment_id: String,
    comment_x: f32,
    comment_y: f32,
    comment_width: f32,
    comment_height: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct GraphCommentDragState {
    comment_id: String,
    start_comment_x: f32,
    start_comment_y: f32,
    comment_width: f32,
    comment_height: f32,
    start_mouse_x: f32,
    start_mouse_y: f32,
    preview_x: f32,
    preview_y: f32,
    moved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GraphNodeNudgeControl {
    id_label: &'static str,
    label: &'static str,
    dx: f32,
    dy: f32,
}

const GRAPH_NODE_NUDGE_CONTROLS: [GraphNodeNudgeControl; 4] = [
    GraphNodeNudgeControl {
        id_label: "x-neg",
        label: "X-",
        dx: -24.0,
        dy: 0.0,
    },
    GraphNodeNudgeControl {
        id_label: "y-neg",
        label: "Y-",
        dx: 0.0,
        dy: -24.0,
    },
    GraphNodeNudgeControl {
        id_label: "y-pos",
        label: "Y+",
        dx: 0.0,
        dy: 24.0,
    },
    GraphNodeNudgeControl {
        id_label: "x-pos",
        label: "X+",
        dx: 24.0,
        dy: 0.0,
    },
];

#[derive(Debug, Clone, Copy, PartialEq)]
struct GraphViewportState {
    pan_x: f32,
    pan_y: f32,
    zoom: f32,
}

const GRAPH_CANVAS_PADDING: f32 = 32.0;
const GRAPH_VIEWPORT_PAN_STEP: f32 = 80.0;
const GRAPH_VIEWPORT_ZOOM_STEP: f32 = 0.1;
const GRAPH_VIEWPORT_ZOOM_MIN: f32 = 0.5;
const GRAPH_VIEWPORT_ZOOM_MAX: f32 = 2.0;
const GRAPH_NODE_DRAG_THRESHOLD: f32 = 3.0;
const GRAPH_DEFAULT_COMMENT_WIDTH: f32 = 220.0;
const GRAPH_DEFAULT_COMMENT_HEIGHT: f32 = 96.0;
/// Pixels of scroll-wheel travel treated as one wheel "line" when the
/// platform reports precise pixel deltas instead of line deltas.
const GRAPH_WHEEL_LINE_PIXELS: f32 = 20.0;
const GRAPH_CONNECTION_THICKNESS: f32 = 2.0;
const GRAPH_BEZIER_MIN_TANGENT: f32 = 40.0;

impl Default for GraphViewportState {
    fn default() -> Self {
        Self {
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
        }
    }
}

impl GraphViewportState {
    fn pan_by(self, dx: f32, dy: f32) -> Self {
        Self {
            pan_x: self.pan_x + dx,
            pan_y: self.pan_y + dy,
            ..self
        }
    }

    #[allow(clippy::unused_self)] // mirrors the by-value builder API of pan_by/zoom_by for chaining symmetry
    fn reset(self) -> Self {
        Self::default()
    }

    fn zoom_by(self, delta: f32) -> Self {
        Self {
            zoom: clamp_graph_zoom(self.zoom + delta),
            ..self
        }
    }
}

fn clamp_graph_zoom(zoom: f32) -> f32 {
    if !zoom.is_finite() {
        return 1.0;
    }
    let zoom = zoom.clamp(GRAPH_VIEWPORT_ZOOM_MIN, GRAPH_VIEWPORT_ZOOM_MAX);
    (zoom * 10.0).round() / 10.0
}

fn sanitize_graph_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() && zoom > 0.0 {
        zoom
    } else {
        1.0
    }
}

/// Zooms the viewport by `delta` while keeping the document point under the
/// canvas-local `anchor` position fixed on screen.
///
/// Screen mapping is `screen = (doc + offset) * zoom` with
/// `offset = padding - bounds_min + pan`, so holding the anchor fixed across
/// a zoom change requires `pan' = pan + anchor * (1/zoom' - 1/zoom)`.
fn zoom_viewport_anchored(
    viewport: GraphViewportState,
    anchor_x: f32,
    anchor_y: f32,
    delta: f32,
) -> GraphViewportState {
    let old_zoom = sanitize_graph_zoom(viewport.zoom);
    let new_zoom = clamp_graph_zoom(old_zoom + delta);
    // Not a nearness test: at a zoom bound `clamp_graph_zoom` returns
    // `old_zoom` bit for bit, and this asks whether the clamp swallowed the
    // delta. An epsilon here would drop real sub-epsilon pan corrections.
    #[allow(clippy::float_cmp)]
    let zoom_unchanged = new_zoom == old_zoom;
    if zoom_unchanged || !anchor_x.is_finite() || !anchor_y.is_finite() {
        return GraphViewportState {
            zoom: new_zoom,
            ..viewport
        };
    }
    let pan_scale = 1.0 / new_zoom - 1.0 / old_zoom;
    GraphViewportState {
        pan_x: anchor_x.mul_add(pan_scale, viewport.pan_x),
        pan_y: anchor_y.mul_add(pan_scale, viewport.pan_y),
        zoom: new_zoom,
    }
}

/// Converts a scroll-wheel delta into a zoom delta: one wheel line is one
/// zoom step, matching the toolbar +/- buttons.
fn wheel_zoom_delta(delta: ScrollDelta) -> f32 {
    let lines = match delta {
        ScrollDelta::Lines(lines) => lines.y,
        ScrollDelta::Pixels(pixels) => pixels_to_f32(pixels.y) / GRAPH_WHEEL_LINE_PIXELS,
    };
    if !lines.is_finite() {
        return 0.0;
    }
    lines.clamp(-3.0, 3.0) * GRAPH_VIEWPORT_ZOOM_STEP
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GraphCanvasTransform {
    offset_x: f32,
    offset_y: f32,
    zoom: f32,
}

impl GraphCanvasTransform {
    fn from_bounds(bounds: GraphCanvasBounds, viewport: GraphViewportState) -> Self {
        Self {
            offset_x: GRAPH_CANVAS_PADDING - bounds.min_x + viewport.pan_x,
            offset_y: GRAPH_CANVAS_PADDING - bounds.min_y + viewport.pan_y,
            zoom: viewport.zoom,
        }
    }

    fn point(self, point: GraphPointProjectionData) -> GraphPointProjectionData {
        GraphPointProjectionData::new(
            (point.x + self.offset_x) * self.zoom,
            (point.y + self.offset_y) * self.zoom,
        )
    }

    /// Inverse of [`Self::point`]: maps a canvas-local screen position back to
    /// document space.
    fn inverse_point(self, point: GraphPointProjectionData) -> GraphPointProjectionData {
        let zoom = sanitize_graph_zoom(self.zoom);
        GraphPointProjectionData::new(
            point.x / zoom - self.offset_x,
            point.y / zoom - self.offset_y,
        )
    }

    fn length(self, value: f32) -> f32 {
        value * self.zoom
    }
}

/// Active middle-mouse or space+left drag-pan of the graph viewport.
///
/// All four fields are captured once, when the drag begins: `pan_*` is the
/// viewport pan at that moment and `grab_*` the cursor position that grabbed
/// it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GraphPanDragState {
    pan_x: f32,
    pan_y: f32,
    grab_x: f32,
    grab_y: f32,
}

impl GraphPanDragState {
    const fn start(viewport: GraphViewportState, mouse_x: f32, mouse_y: f32) -> Self {
        Self {
            pan_x: viewport.pan_x,
            pan_y: viewport.pan_y,
            grab_x: mouse_x,
            grab_y: mouse_y,
        }
    }

    /// Pan is applied before the zoom multiply, so one pan unit moves the
    /// scene `zoom` screen pixels; the mouse delta is divided back out to
    /// keep the grab point under the cursor.
    fn panned_viewport(
        self,
        viewport: GraphViewportState,
        mouse_x: f32,
        mouse_y: f32,
    ) -> GraphViewportState {
        let zoom = sanitize_graph_zoom(viewport.zoom);
        GraphViewportState {
            pan_x: self.pan_x + (mouse_x - self.grab_x) / zoom,
            pan_y: self.pan_y + (mouse_y - self.grab_y) / zoom,
            zoom: viewport.zoom,
        }
    }
}

impl GraphNodeDragState {
    fn start(start: &GraphNodeDragStartData, mouse_x: f32, mouse_y: f32) -> Self {
        Self {
            node_id: start.node_id.clone(),
            start_node_x: start.node_x,
            start_node_y: start.node_y,
            start_mouse_x: mouse_x,
            start_mouse_y: mouse_y,
            preview_x: start.node_x,
            preview_y: start.node_y,
            moved: false,
        }
    }

    fn update(&mut self, mouse_x: f32, mouse_y: f32, viewport: GraphViewportState) {
        let moved = graph_node_drag_exceeds_threshold(
            self.start_mouse_x,
            self.start_mouse_y,
            mouse_x,
            mouse_y,
        );
        let (preview_x, preview_y) = graph_node_drag_preview_position(
            self.start_node_x,
            self.start_node_y,
            self.start_mouse_x,
            self.start_mouse_y,
            mouse_x,
            mouse_y,
            viewport.zoom,
        );
        self.preview_x = preview_x;
        self.preview_y = preview_y;
        self.moved |= moved;
    }

    const fn committed_position(&self) -> (f32, f32) {
        (self.preview_x, self.preview_y)
    }
}

impl GraphRouteAnchorDragState {
    fn start(start: &GraphRouteAnchorDragStartData, mouse_x: f32, mouse_y: f32) -> Self {
        Self {
            connection_id: start.connection_id.clone(),
            anchor_id: start.anchor_id.clone(),
            start_anchor_x: start.anchor_x,
            start_anchor_y: start.anchor_y,
            start_mouse_x: mouse_x,
            start_mouse_y: mouse_y,
            preview_x: start.anchor_x,
            preview_y: start.anchor_y,
            moved: false,
        }
    }

    fn update(&mut self, mouse_x: f32, mouse_y: f32, viewport: GraphViewportState) {
        let moved = graph_node_drag_exceeds_threshold(
            self.start_mouse_x,
            self.start_mouse_y,
            mouse_x,
            mouse_y,
        );
        let (preview_x, preview_y) = graph_node_drag_preview_position(
            self.start_anchor_x,
            self.start_anchor_y,
            self.start_mouse_x,
            self.start_mouse_y,
            mouse_x,
            mouse_y,
            viewport.zoom,
        );
        self.preview_x = preview_x;
        self.preview_y = preview_y;
        self.moved |= moved;
    }

    const fn committed_position(&self) -> (f32, f32) {
        (self.preview_x, self.preview_y)
    }
}

impl GraphCommentDragState {
    fn start(start: &GraphCommentDragStartData, mouse_x: f32, mouse_y: f32) -> Self {
        Self {
            comment_id: start.comment_id.clone(),
            start_comment_x: start.comment_x,
            start_comment_y: start.comment_y,
            comment_width: start.comment_width,
            comment_height: start.comment_height,
            start_mouse_x: mouse_x,
            start_mouse_y: mouse_y,
            preview_x: start.comment_x,
            preview_y: start.comment_y,
            moved: false,
        }
    }

    fn update(&mut self, mouse_x: f32, mouse_y: f32, viewport: GraphViewportState) {
        let moved = graph_node_drag_exceeds_threshold(
            self.start_mouse_x,
            self.start_mouse_y,
            mouse_x,
            mouse_y,
        );
        let (preview_x, preview_y) = graph_node_drag_preview_position(
            self.start_comment_x,
            self.start_comment_y,
            self.start_mouse_x,
            self.start_mouse_y,
            mouse_x,
            mouse_y,
            viewport.zoom,
        );
        self.preview_x = preview_x;
        self.preview_y = preview_y;
        self.moved |= moved;
    }

    const fn committed_bounds(&self) -> (f32, f32, f32, f32) {
        (
            self.preview_x,
            self.preview_y,
            self.comment_width,
            self.comment_height,
        )
    }
}

fn graph_node_drag_preview_position(
    start_node_x: f32,
    start_node_y: f32,
    start_mouse_x: f32,
    start_mouse_y: f32,
    mouse_x: f32,
    mouse_y: f32,
    zoom: f32,
) -> (f32, f32) {
    let zoom = if zoom.is_finite() && zoom > 0.0 {
        zoom
    } else {
        1.0
    };
    (
        start_node_x + (mouse_x - start_mouse_x) / zoom,
        start_node_y + (mouse_y - start_mouse_y) / zoom,
    )
}

fn graph_node_drag_exceeds_threshold(
    start_mouse_x: f32,
    start_mouse_y: f32,
    mouse_x: f32,
    mouse_y: f32,
) -> bool {
    let dx = mouse_x - start_mouse_x;
    let dy = mouse_y - start_mouse_y;
    dx.hypot(dy) >= GRAPH_NODE_DRAG_THRESHOLD
}

fn pixels_to_f32(value: Pixels) -> f32 {
    // invariant: UI pixel coordinates are small finite values; f64->f32 narrowing is the intended precision for layout
    #[allow(clippy::cast_possible_truncation)]
    {
        value.to_f64() as f32
    }
}

impl VisualGraphPanel {
    pub const NAME: &'static str = "visual-graph";

    pub fn init(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let new_graph_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Graph name"));
        let subscriptions =
            vec![cx.subscribe_in(&new_graph_name_input, window, Self::on_new_graph_name_input)];
        Self {
            focus: cx.focus_handle(),
            new_graph_name_input,
            new_graph_name: String::new(),
            selected_node_id: None,
            selected_comment_id: None,
            pending_output_port: None,
            viewport: GraphViewportState::default(),
            canvas_bounds: Bounds::default(),
            space_pan_key_held: false,
            active_pan_drag: None,
            context_menu_document_position: None,
            active_node_drag: None,
            active_route_anchor_drag: None,
            active_comment_drag: None,
            _subscriptions: subscriptions,
        }
    }

    fn on_new_graph_name_input(
        &mut self,
        state: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
            self.new_graph_name = state.read(cx).value().to_string();
            cx.notify();
        }
    }

    fn handle_graph_port_click(
        &mut self,
        click: &GraphPortClickData,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let transition = graph_port_click_transition(self.pending_output_port.take(), click);
        self.pending_output_port = transition.pending_output_port;
        if let Some(request) = transition.connection_request {
            window.dispatch_action(
                Box::new(crate::actions::ConnectGraphPorts {
                    from_node_id: request.from_node_id,
                    from_port_id: request.from_port_id,
                    to_node_id: request.to_node_id,
                    to_port_id: request.to_port_id,
                }),
                cx,
            );
        }
        cx.notify();
    }

    fn handle_graph_node_click(
        &mut self,
        click: &GraphNodeClickData,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.selected_node_id = Some(click.node_id.clone());
        self.selected_comment_id = None;
        cx.notify();
    }

    fn handle_graph_comment_click(
        &mut self,
        click: &GraphCommentClickData,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.selected_node_id = None;
        self.selected_comment_id = Some(click.comment_id.clone());
        cx.notify();
    }

    fn handle_graph_canvas_click(&mut self, cx: &mut Context<'_, Self>) {
        self.selected_node_id = None;
        self.selected_comment_id = None;
        self.pending_output_port = None;
        self.active_node_drag = None;
        self.active_route_anchor_drag = None;
        self.active_comment_drag = None;
        self.active_pan_drag = None;
        cx.notify();
    }

    fn canvas_local_position(&self, position: gpui::Point<Pixels>) -> GraphPointProjectionData {
        GraphPointProjectionData::new(
            pixels_to_f32(position.x - self.canvas_bounds.origin.x),
            pixels_to_f32(position.y - self.canvas_bounds.origin.y),
        )
    }

    fn handle_graph_canvas_scroll_zoom(
        &mut self,
        event: &ScrollWheelEvent,
        cx: &mut Context<'_, Self>,
    ) {
        let delta = wheel_zoom_delta(event.delta);
        if delta == 0.0 {
            return;
        }
        let anchor = self.canvas_local_position(event.position);
        self.viewport = zoom_viewport_anchored(self.viewport, anchor.x, anchor.y, delta);
        cx.notify();
    }

    fn handle_graph_canvas_pan_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        cx: &mut Context<'_, Self>,
    ) {
        self.active_node_drag = None;
        self.active_route_anchor_drag = None;
        self.active_comment_drag = None;
        self.active_pan_drag = Some(GraphPanDragState::start(
            self.viewport,
            pixels_to_f32(event.position.x),
            pixels_to_f32(event.position.y),
        ));
        cx.notify();
    }

    fn handle_graph_canvas_context_menu_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        cx: &Context<'_, Self>,
    ) {
        let document_position = cx
            .try_global::<EditorGraphDocumentProjection>()
            .and_then(|projection| projection.document.as_ref())
            .map(|document| {
                let transform =
                    GraphCanvasTransform::from_bounds(graph_canvas_bounds(document), self.viewport);
                transform.inverse_point(self.canvas_local_position(event.position))
            });
        self.context_menu_document_position = document_position;
    }

    fn handle_graph_pan_key(&mut self, held: bool, cx: &mut Context<'_, Self>) {
        if self.space_pan_key_held != held {
            self.space_pan_key_held = held;
            if !held {
                self.active_pan_drag = None;
            }
            cx.notify();
        }
    }

    fn handle_graph_palette_drop(
        &self,
        payload: &GraphPaletteDragPayload,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let drop_target = cx
            .try_global::<EditorGraphDocumentProjection>()
            .and_then(|projection| projection.document.as_ref())
            .map(|document| {
                let transform =
                    GraphCanvasTransform::from_bounds(graph_canvas_bounds(document), self.viewport);
                (transform, next_node_position(Some(document)))
            });
        let Some((transform, fallback)) = drop_target else {
            return;
        };
        let local = self.canvas_local_position(window.mouse_position());
        let (x, y) =
            graph_context_menu_add_position(Some(transform.inverse_point(local)), fallback);
        window.dispatch_action(
            Box::new(crate::actions::AddGraphNode {
                node_type: payload.node_type.clone(),
                node_type_version: payload.version,
                x,
                y,
            }),
            cx,
        );
        cx.notify();
    }

    fn handle_graph_viewport_pan(&mut self, dx: f32, dy: f32, cx: &mut Context<'_, Self>) {
        self.viewport = self.viewport.pan_by(dx, dy);
        cx.notify();
    }

    fn handle_graph_viewport_zoom(&mut self, delta: f32, cx: &mut Context<'_, Self>) {
        self.viewport = self.viewport.zoom_by(delta);
        cx.notify();
    }

    fn handle_graph_viewport_reset(&mut self, cx: &mut Context<'_, Self>) {
        self.viewport = self.viewport.reset();
        cx.notify();
    }

    fn handle_graph_node_mouse_down(
        &mut self,
        start: &GraphNodeDragStartData,
        event: &MouseDownEvent,
        cx: &mut Context<'_, Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        self.selected_node_id = Some(start.node_id.clone());
        self.selected_comment_id = None;
        self.pending_output_port = None;
        self.active_route_anchor_drag = None;
        self.active_comment_drag = None;
        self.active_node_drag = Some(GraphNodeDragState::start(
            start,
            pixels_to_f32(event.position.x),
            pixels_to_f32(event.position.y),
        ));
        cx.notify();
    }

    fn handle_graph_route_anchor_mouse_down(
        &mut self,
        start: &GraphRouteAnchorDragStartData,
        event: &MouseDownEvent,
        cx: &mut Context<'_, Self>,
    ) {
        if event.button != MouseButton::Left || !start.draggable {
            return;
        }
        self.selected_node_id = None;
        self.selected_comment_id = None;
        self.pending_output_port = None;
        self.active_node_drag = None;
        self.active_comment_drag = None;
        self.active_route_anchor_drag = Some(GraphRouteAnchorDragState::start(
            start,
            pixels_to_f32(event.position.x),
            pixels_to_f32(event.position.y),
        ));
        cx.notify();
    }

    fn handle_graph_comment_mouse_down(
        &mut self,
        start: &GraphCommentDragStartData,
        event: &MouseDownEvent,
        cx: &mut Context<'_, Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        self.selected_node_id = None;
        self.selected_comment_id = Some(start.comment_id.clone());
        self.pending_output_port = None;
        self.active_node_drag = None;
        self.active_route_anchor_drag = None;
        self.active_comment_drag = Some(GraphCommentDragState::start(
            start,
            pixels_to_f32(event.position.x),
            pixels_to_f32(event.position.y),
        ));
        cx.notify();
    }

    fn handle_graph_canvas_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<'_, Self>,
    ) {
        let mouse_x = pixels_to_f32(event.position.x);
        let mouse_y = pixels_to_f32(event.position.y);
        if let Some(pan) = self.active_pan_drag {
            self.viewport = pan.panned_viewport(self.viewport, mouse_x, mouse_y);
            cx.notify();
            return;
        }
        if !event.dragging() {
            return;
        }
        let viewport = self.viewport;
        // Update whichever drags are active; map each to a bool reporting
        // whether it ran so we only notify when at least one drag moved.
        let node_changed = self.active_node_drag.as_mut().is_some_and(|drag| {
            drag.update(mouse_x, mouse_y, viewport);
            true
        });
        let anchor_changed = self.active_route_anchor_drag.as_mut().is_some_and(|drag| {
            drag.update(mouse_x, mouse_y, viewport);
            true
        });
        let comment_changed = self.active_comment_drag.as_mut().is_some_and(|drag| {
            drag.update(mouse_x, mouse_y, viewport);
            true
        });
        if node_changed || anchor_changed || comment_changed {
            cx.notify();
        }
    }

    fn handle_graph_canvas_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.active_pan_drag.is_some()
            && matches!(event.button, MouseButton::Left | MouseButton::Middle)
        {
            self.active_pan_drag = None;
            cx.notify();
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        let mouse_x = pixels_to_f32(event.position.x);
        let mouse_y = pixels_to_f32(event.position.y);
        let Some(mut drag) = self.active_node_drag.take() else {
            if let Some(mut drag) = self.active_route_anchor_drag.take() {
                drag.update(mouse_x, mouse_y, self.viewport);
                if drag.moved {
                    let (x, y) = drag.committed_position();
                    window.dispatch_action(
                        Box::new(crate::actions::MoveGraphRouteAnchor {
                            connection_id: drag.connection_id,
                            anchor_id: drag.anchor_id,
                            x,
                            y,
                        }),
                        cx,
                    );
                    cx.stop_propagation();
                }
                cx.notify();
            }
            if let Some(mut drag) = self.active_comment_drag.take() {
                drag.update(mouse_x, mouse_y, self.viewport);
                if drag.moved {
                    let (x, y, width, height) = drag.committed_bounds();
                    window.dispatch_action(
                        Box::new(crate::actions::MoveGraphComment {
                            comment_id: drag.comment_id,
                            x,
                            y,
                            width,
                            height,
                        }),
                        cx,
                    );
                    cx.stop_propagation();
                }
                cx.notify();
            }
            return;
        };
        drag.update(mouse_x, mouse_y, self.viewport);
        if drag.moved {
            let (x, y) = drag.committed_position();
            window.dispatch_action(
                Box::new(crate::actions::MoveGraphNode {
                    node_id: drag.node_id,
                    x,
                    y,
                }),
                cx,
            );
            cx.stop_propagation();
        }
        cx.notify();
    }
}

/// Borrowed stand-in for an unpublished projection, so a repaint never allocates
/// an empty projection just to have something to reference.
static NO_GRAPH_PROJECTION: EditorGraphDocumentProjection = EditorGraphDocumentProjection::empty();

impl Render for VisualGraphPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(reason) = crate::panels::editor_project_host_failed_reason(cx) {
            return crate::panels::render_project_host_failed_placeholder(
                "Visual Graph",
                &reason,
                cx,
            )
            .into_any_element();
        }
        if crate::panels::editor_project_host_connecting(cx) {
            return crate::panels::render_project_host_connecting_placeholder("Visual Graph", cx)
                .into_any_element();
        }
        // The inspector's editable text inputs are the only elements in this tree
        // that need `&mut Window` / `&mut Context`. Building that section first
        // ends every mutable borrow before the projection and theme borrows are
        // taken, so the rest of the tree reads the published projection in place
        // instead of deep-cloning it on every pointer event.
        let inspector = render_graph_selection_inspector(
            self.selected_node_id.as_deref(),
            self.selected_comment_id.as_deref(),
            window,
            cx,
        );
        let theme = cx.theme();
        let projection = cx
            .try_global::<EditorGraphDocumentProjection>()
            .unwrap_or(&NO_GRAPH_PROJECTION);

        v_flex()
            .size_full()
            .bg(theme.background)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                if event.keystroke.key == "space" {
                    this.handle_graph_pan_key(true, cx);
                }
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _window, cx| {
                if event.keystroke.key == "space" {
                    this.handle_graph_pan_key(false, cx);
                }
            }))
            .child(render_graph_toolbar(
                projection,
                self.pending_output_port.as_ref(),
                self.viewport,
                theme,
                cx,
            ))
            .child(
                h_flex()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(render_graph_catalog_sidebar(
                        projection,
                        &self.new_graph_name_input,
                        &self.new_graph_name,
                        theme,
                    ))
                    .child(div().relative().flex_1().h_full().overflow_hidden().child(
                        render_graph_canvas(
                            projection,
                            self.selected_node_id.as_deref(),
                            self.selected_comment_id.as_deref(),
                            self.pending_output_port.as_ref(),
                            self.viewport,
                            self.active_node_drag.as_ref(),
                            self.active_route_anchor_drag.as_ref(),
                            self.active_comment_drag.as_ref(),
                            self.space_pan_key_held,
                            self.active_pan_drag.is_some(),
                            theme,
                            cx,
                        ),
                    ))
                    .child(inspector),
            )
            .into_any_element()
    }
}

impl Focusable for VisualGraphPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Panel for VisualGraphPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("account_tree"), "Graph", kit::TabTone::Default)
    }

    fn inner_padding(&self, _cx: &gpui::App) -> bool {
        false
    }
}

impl gpui::EventEmitter<gpui_component::dock::PanelEvent> for VisualGraphPanel {}

fn render_graph_toolbar(
    projection: &EditorGraphDocumentProjection,
    pending_output_port: Option<&PendingGraphPortConnection>,
    viewport: GraphViewportState,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, VisualGraphPanel>,
) -> impl IntoElement {
    let summary = projection
        .document
        .as_ref()
        .map_or_else(|| "No graph document".to_string(), graph_document_summary);
    let build_mode = projection
        .document
        .as_ref()
        .and_then(graph_document_build_mode_label);

    render_graph_toolbar_body(
        summary,
        build_mode,
        projection,
        pending_output_port,
        viewport,
        theme,
        cx,
    )
}

#[allow(clippy::too_many_arguments)] // toolbar assembly split out of render_graph_toolbar purely to satisfy too_many_lines
fn render_graph_toolbar_body(
    summary: String,
    build_mode: Option<String>,
    projection: &EditorGraphDocumentProjection,
    pending_output_port: Option<&PendingGraphPortConnection>,
    viewport: GraphViewportState,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, VisualGraphPanel>,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .p_2()
        .child(
            div()
                .flex_1()
                .text_sm()
                .font_semibold()
                .text_color(theme.foreground)
                .child(summary),
        )
        .when_some(build_mode, |this, build_mode| {
            this.child(render_graph_badge(build_mode, theme))
        })
        .when_some(projection.layout_job.as_ref(), |this, job| {
            this.child(render_layout_job_badge(job, theme))
        })
        .when_some(projection.build_status.as_ref(), |this, status| {
            this.child(render_graph_build_status_badge(status, theme))
        })
        .when_some(pending_output_port, |this, pending| {
            this.child(render_pending_connection_badge(pending, theme))
        })
        .child(render_graph_viewport_badge(viewport, theme))
        .children(graph_viewport_buttons(theme, cx))
        .children(graph_toolbar_action_buttons(projection, theme))
}

fn graph_viewport_buttons(
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, VisualGraphPanel>,
) -> Vec<gpui::AnyElement> {
    vec![
        graph_viewport_button(
            "graph-pan-left",
            "left",
            theme,
            cx.listener(|this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                this.handle_graph_viewport_pan(GRAPH_VIEWPORT_PAN_STEP, 0.0, cx);
            }),
        )
        .into_any_element(),
        graph_viewport_button(
            "graph-pan-up",
            "up",
            theme,
            cx.listener(|this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                this.handle_graph_viewport_pan(0.0, GRAPH_VIEWPORT_PAN_STEP, cx);
            }),
        )
        .into_any_element(),
        graph_viewport_button(
            "graph-pan-down",
            "down",
            theme,
            cx.listener(|this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                this.handle_graph_viewport_pan(0.0, -GRAPH_VIEWPORT_PAN_STEP, cx);
            }),
        )
        .into_any_element(),
        graph_viewport_button(
            "graph-pan-right",
            "right",
            theme,
            cx.listener(|this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                this.handle_graph_viewport_pan(-GRAPH_VIEWPORT_PAN_STEP, 0.0, cx);
            }),
        )
        .into_any_element(),
        graph_viewport_button(
            "graph-zoom-out",
            "-",
            theme,
            cx.listener(|this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                this.handle_graph_viewport_zoom(-GRAPH_VIEWPORT_ZOOM_STEP, cx);
            }),
        )
        .into_any_element(),
        graph_viewport_button(
            "graph-zoom-in",
            "+",
            theme,
            cx.listener(|this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                this.handle_graph_viewport_zoom(GRAPH_VIEWPORT_ZOOM_STEP, cx);
            }),
        )
        .into_any_element(),
        graph_viewport_button(
            "graph-viewport-reset",
            "reset",
            theme,
            cx.listener(|this, _: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                this.handle_graph_viewport_reset(cx);
            }),
        )
        .into_any_element(),
    ]
}

fn graph_toolbar_action_buttons(
    projection: &EditorGraphDocumentProjection,
    theme: &gpui_component::theme::Theme,
) -> Vec<gpui::AnyElement> {
    let mut buttons = vec![
        graph_action_button(
            "graph-refresh",
            "refresh",
            crate::actions::RefreshGraphDocument,
            theme,
        )
        .into_any_element(),
        graph_action_button(
            "graph-auto-layout",
            "layout",
            crate::actions::AutoLayoutGraph,
            theme,
        )
        .into_any_element(),
        graph_action_button(
            "graph-route-connections",
            "route",
            crate::actions::RouteGraphConnections,
            theme,
        )
        .into_any_element(),
    ];
    if let Some(document) = projection.document.as_ref() {
        let (x, y, width, height) = next_comment_bounds(document);
        buttons.push(
            graph_action_button(
                "graph-create-comment",
                "note",
                crate::actions::CreateGraphComment {
                    text: "Comment".to_string(),
                    x,
                    y,
                    width,
                    height,
                },
                theme,
            )
            .into_any_element(),
        );
    }
    buttons.push(
        graph_action_button(
            "graph-save",
            "save",
            crate::actions::SaveGraphDocument,
            theme,
        )
        .into_any_element(),
    );
    buttons.push(
        graph_action_button(
            "graph-build",
            "build",
            crate::actions::BuildGraphDocument,
            theme,
        )
        .into_any_element(),
    );
    buttons.push(
        graph_action_button(
            "graph-refresh-build-status",
            "status",
            crate::actions::RefreshGraphBuildStatus,
            theme,
        )
        .into_any_element(),
    );
    buttons
}

fn render_graph_catalog_sidebar(
    projection: &EditorGraphDocumentProjection,
    new_graph_name_input: &Entity<InputState>,
    new_graph_name: &str,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .w(px(304.0))
        .h_full()
        .border_r_1()
        .border_color(theme.border)
        .bg(theme.background)
        .overflow_hidden()
        .child(
            div().p_2().border_b_1().border_color(theme.border).child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .child("Graph Catalog"),
            ),
        )
        .child(
            div().flex_1().w_full().overflow_y_scrollbar().child(
                v_flex()
                    .gap_3()
                    .p_2()
                    .child(render_graph_creation_sidebar_section(
                        &projection.creation_catalog,
                        new_graph_name_input,
                        new_graph_name,
                        theme,
                    ))
                    .child(render_graph_document_sidebar_section(
                        &projection.graph_documents,
                        theme,
                    ))
                    .child(render_graph_palette_sidebar_section(
                        &projection.node_palette,
                        projection.document.as_ref(),
                        theme,
                    )),
            ),
        )
}

fn render_graph_creation_sidebar_section(
    catalog: &GraphCreationCatalogProjectionData,
    new_graph_name_input: &Entity<InputState>,
    new_graph_name: &str,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(render_sidebar_section_header(
            "New Graph",
            catalog.graph_types.len(),
            theme,
        ))
        .child(
            div()
                .id("graph-sidebar-new-name")
                .h(px(28.0))
                .px_2()
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .flex()
                .items_center()
                .child(
                    Input::new(new_graph_name_input)
                        .small()
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false),
                ),
        )
        .when(catalog.is_empty(), |this| {
            this.child(render_sidebar_empty("No graph types published", theme))
        })
        .children(
            catalog
                .graph_types
                .iter()
                .map(|graph_type| render_graph_type_catalog_row(graph_type, new_graph_name, theme)),
        )
}

fn render_graph_document_sidebar_section(
    documents: &GraphDocumentListProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(render_sidebar_section_header(
            "Open Graph",
            documents.documents.len(),
            theme,
        ))
        .when(documents.is_empty(), |this| {
            this.child(render_sidebar_empty("No graph documents yet", theme))
        })
        .children(
            documents
                .documents
                .iter()
                .map(|document| render_graph_document_row(document, theme)),
        )
}

fn render_graph_document_row(
    document: &GraphDocumentListItemProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let document_id = document.document_id.clone();
    let state = if document.current {
        "current"
    } else if document.unsaved_changes {
        "dirty"
    } else if document.loaded {
        "loaded"
    } else {
        "saved"
    };
    div()
        .id(gpui::SharedString::from(format!(
            "graph-sidebar-open-{}",
            graph_element_key(&document.document_id)
        )))
        .rounded_sm()
        .border_1()
        .border_color(if document.current {
            theme.accent
        } else {
            theme.border
        })
        .bg(if document.current {
            theme.muted
        } else {
            theme.popover
        })
        .p_2()
        .hover(|this| this.bg(theme.muted))
        .cursor_pointer()
        .child(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .text_color(theme.foreground)
                                .truncate()
                                .child(
                                    crate::naming::display_name(&document.source_path).into_owned(),
                                ),
                        )
                        .child(render_graph_badge(state, theme)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(document.graph_type.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(graph_document_revision_label(document)),
                ),
        )
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(
                Box::new(crate::actions::OpenGraphDocument {
                    document_id: document_id.clone(),
                }),
                cx,
            );
        })
}

fn render_graph_palette_sidebar_section(
    palette: &GraphNodePaletteProjectionData,
    document: Option<&GraphDocumentProjectionData>,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(render_sidebar_section_header(
            "Node Palette",
            palette.nodes.len(),
            theme,
        ))
        .when(palette.is_empty(), |this| {
            this.child(render_sidebar_empty(
                "No node types match this graph",
                theme,
            ))
        })
        .children(
            palette
                .nodes
                .iter()
                .map(|node| render_graph_node_catalog_row(node, document, theme)),
        )
}

fn render_sidebar_section_header(
    label: &'static str,
    count: usize,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_xs()
                .font_semibold()
                .text_color(theme.foreground)
                .child(label),
        )
        .child(
            div()
                .px_1()
                .py_0p5()
                .rounded_sm()
                .bg(theme.muted)
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(count.to_string()),
        )
}

fn render_sidebar_empty(
    message: &'static str,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .p_2()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(message)
}

fn render_graph_type_catalog_row(
    graph_type: &GraphTypeCreationProjectionData,
    new_graph_name: &str,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let graph_type_id = graph_type.graph_type.clone();
    let document_name = new_graph_name.trim().to_string();
    let disabled = document_name.is_empty();
    div()
        .id(gpui::SharedString::from(format!(
            "graph-sidebar-create-{}",
            graph_element_key(&graph_type.graph_type)
        )))
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .p_2()
        .when(!disabled, |this| {
            this.hover(|this| this.bg(theme.muted)).cursor_pointer()
        })
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .text_color(if disabled {
                            theme.muted_foreground
                        } else {
                            theme.foreground
                        })
                        .child(graph_type.label.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(graph_type.category.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(graph_type_execution_label(graph_type)),
                )
                .when_some(graph_type.compiler_backend.as_ref(), |this, backend| {
                    this.child(render_graph_backend_markers(backend, theme))
                }),
        )
        .when(!disabled, |this| {
            this.on_click(move |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(
                    Box::new(crate::actions::CreateGraphDocument {
                        graph_type: graph_type_id.clone(),
                        document_name: document_name.clone(),
                    }),
                    cx,
                );
            })
        })
}

fn render_graph_backend_markers(
    backend: &GraphCompilerBackendProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex().gap_1().children(
        backend
            .capability_markers
            .iter()
            .map(|marker| render_graph_badge(marker.clone(), theme)),
    )
}

/// Payload carried by a palette-row drag onto the graph canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphPaletteDragPayload {
    node_type: String,
    version: u32,
    label: String,
}

/// Drag preview view shown under the cursor while dragging a palette entry.
struct GraphPaletteDragPreview {
    label: String,
}

impl Render for GraphPaletteDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .border_1()
            .border_color(theme.accent)
            .bg(theme.popover)
            .shadow_sm()
            .text_xs()
            .text_color(theme.foreground)
            .child(self.label.clone())
    }
}

fn render_graph_node_catalog_row(
    node: &GraphNodePaletteItemData,
    document: Option<&GraphDocumentProjectionData>,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let node_type = node.node_type.clone();
    let node_type_version = node.version;
    let (x, y) = next_node_position(document);
    let disabled = document.is_none();
    let drag_payload = GraphPaletteDragPayload {
        node_type: node.node_type.clone(),
        version: node.version,
        label: node.label.clone(),
    };
    div()
        .id(gpui::SharedString::from(format!(
            "graph-sidebar-node-{}-v{}",
            graph_element_key(&node.node_type),
            node.version
        )))
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .p_2()
        .when(!disabled, |this| {
            this.hover(|this| this.bg(theme.muted)).cursor_pointer()
        })
        .child(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .text_color(if disabled {
                                    theme.muted_foreground
                                } else {
                                    theme.foreground
                                })
                                .child(node.label.clone()),
                        )
                        .child(render_graph_badge(graph_node_runtime_label(node), theme)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(node.category.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(graph_node_port_summary(node)),
                )
                .when_some(graph_node_runtime_detail(node), |this, detail| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    )
                })
                .when_some(node.description.as_ref(), |this, description| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(description.clone()),
                    )
                }),
        )
        .when(!disabled, |this| {
            this.on_drag(drag_payload, |payload, _offset, _window, cx| {
                let label = payload.label.clone();
                cx.new(|_| GraphPaletteDragPreview { label })
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(
                    Box::new(crate::actions::AddGraphNode {
                        node_type: node_type.clone(),
                        node_type_version,
                        x,
                        y,
                    }),
                    cx,
                );
            })
        })
}

fn render_graph_badge(
    label: impl Into<String>,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .px_1()
        .py_0p5()
        .rounded_sm()
        .bg(theme.muted)
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(label.into())
}

fn graph_node_runtime_label(node: &GraphNodePaletteItemData) -> String {
    match &node.runtime_binding {
        Some(GraphNodeRuntimeBindingProjectionData::RustSymbol { .. }) => "rust".to_string(),
        Some(GraphNodeRuntimeBindingProjectionData::AssetBuilder { .. }) => "builder".to_string(),
        Some(GraphNodeRuntimeBindingProjectionData::RuntimeComponent { .. }) => {
            "component".to_string()
        }
        Some(GraphNodeRuntimeBindingProjectionData::External { .. }) => "external".to_string(),
        None => "editor".to_string(),
    }
}

fn graph_node_port_summary(node: &GraphNodePaletteItemData) -> String {
    format!(
        "{} in / {} out / {} defaults",
        node.input_count, node.output_count, node.default_input_count
    )
}

fn graph_node_runtime_detail(node: &GraphNodePaletteItemData) -> Option<String> {
    let binding = node.runtime_binding.as_ref()?;
    Some(match binding {
        GraphNodeRuntimeBindingProjectionData::RustSymbol {
            symbol, call_abi, ..
        } => format!(
            "{} {}",
            graph_rust_node_call_abi_label(call_abi),
            symbol.rsplit("::").next().unwrap_or(symbol)
        ),
        GraphNodeRuntimeBindingProjectionData::AssetBuilder { builder_id } => {
            format!("asset builder {builder_id}")
        }
        GraphNodeRuntimeBindingProjectionData::RuntimeComponent { component_type } => {
            format!("runtime component {component_type}")
        }
        GraphNodeRuntimeBindingProjectionData::External { kind, locator } => {
            format!("{kind} {locator}")
        }
    })
}

fn graph_rust_node_call_abi_label(call_abi: &GraphRustNodeCallAbiProjectionData) -> String {
    match call_abi {
        GraphRustNodeCallAbiProjectionData::ContextSchedule => "context schedule".to_string(),
        GraphRustNodeCallAbiProjectionData::TypedDataflow {
            parameter_count,
            input_parameter_count,
            by_value_parameter_count,
            mutable_parameter_count,
            output_count,
            result,
        } => format!(
            "typed dataflow {}p/{}i/{}o {}{}{}",
            parameter_count,
            input_parameter_count,
            output_count,
            graph_rust_call_result_label(*result),
            graph_value_passing_suffix(" move", *by_value_parameter_count),
            graph_value_passing_suffix(" mut", *mutable_parameter_count),
        ),
    }
}

const fn graph_rust_call_result_label(result: GraphRustCallResultProjectionData) -> &'static str {
    match result {
        GraphRustCallResultProjectionData::Plain => "plain",
        GraphRustCallResultProjectionData::Result => "result",
    }
}

fn graph_value_passing_suffix(label: &'static str, count: usize) -> String {
    if count == 0 {
        String::new()
    } else {
        format!(" {count}{label}")
    }
}

fn next_node_position(document: Option<&GraphDocumentProjectionData>) -> (f32, f32) {
    let Some(document) = document else {
        return (0.0, 0.0);
    };
    // invariant: node counts in an authored graph are small; f32 precision loss is irrelevant for layout placement
    #[allow(clippy::cast_precision_loss)]
    let index = document.nodes.len() as f32;
    let x = (index % 4.0).mul_add(240.0, 80.0);
    let y = (index / 4.0).floor().mul_add(160.0, 80.0);
    (x, y)
}

fn next_comment_bounds(document: &GraphDocumentProjectionData) -> (f32, f32, f32, f32) {
    // invariant: comment counts in an authored graph are small; f32 precision loss is irrelevant for layout placement
    #[allow(clippy::cast_precision_loss)]
    let index = document.comments.len() as f32;
    (
        (index % 3.0).mul_add(240.0, 64.0),
        (index / 3.0).floor().mul_add(128.0, 32.0),
        GRAPH_DEFAULT_COMMENT_WIDTH,
        GRAPH_DEFAULT_COMMENT_HEIGHT,
    )
}

fn graph_type_execution_label(graph_type: &GraphTypeCreationProjectionData) -> String {
    let backend = graph_type
        .compiler_backend
        .as_ref()
        .map_or_else(|| "no compiler".to_string(), graph_compiler_backend_label);
    let runtime = graph_type
        .runtime_execution_strategy
        .as_ref()
        .map(graph_runtime_execution_label)
        .or_else(|| graph_type.runtime_product_kind.clone())
        .unwrap_or_else(|| {
            if graph_type.editor_interpreted {
                "editor interpreted".to_string()
            } else {
                "no runtime product".to_string()
            }
        });

    if graph_type.runtime_compiled {
        format!("{backend} -> {runtime}")
    } else {
        runtime
    }
}

fn graph_document_build_mode_label(document: &GraphDocumentProjectionData) -> Option<String> {
    document
        .graph_type_info
        .as_ref()
        .map(graph_type_build_mode_label)
}

fn graph_type_build_mode_label(graph_type: &GraphTypeCreationProjectionData) -> String {
    match (
        graph_type.compiler_backend.as_ref(),
        graph_type.runtime_execution_strategy.as_ref(),
    ) {
        (
            Some(GraphCompilerBackendProjectionData {
                kind: GraphCompilerBackendKindProjectionData::GeneratedRust { .. },
                capability_markers,
                ..
            }),
            Some(GraphRuntimeExecutionStrategyProjectionData::AotCompiledCode { .. }),
        ) if capability_markers
            .iter()
            .any(|marker| marker == "zero-cost") =>
        {
            "zero-cost Rust AOT".to_string()
        }
        (
            Some(GraphCompilerBackendProjectionData {
                kind: GraphCompilerBackendKindProjectionData::GeneratedRust { .. },
                ..
            }),
            Some(GraphRuntimeExecutionStrategyProjectionData::AotCompiledCode { .. }),
        ) => "generated Rust AOT".to_string(),
        (
            Some(GraphCompilerBackendProjectionData {
                kind: GraphCompilerBackendKindProjectionData::GeneratedRust { .. },
                ..
            }),
            _,
        ) => "generated Rust".to_string(),
        (
            Some(GraphCompilerBackendProjectionData {
                kind: GraphCompilerBackendKindProjectionData::PackedIr { .. },
                ..
            }),
            _,
        ) => "packed IR".to_string(),
        (
            Some(GraphCompilerBackendProjectionData {
                kind: GraphCompilerBackendKindProjectionData::ShaderPipeline { .. },
                ..
            }),
            _,
        ) => "shader graph".to_string(),
        (Some(backend), _) => graph_compiler_backend_label(backend),
        (None, Some(runtime)) => graph_runtime_execution_label(runtime),
        (None, None) if graph_type.editor_interpreted => "editor interpreted".to_string(),
        (None, None) => "descriptor".to_string(),
    }
}

fn graph_compiler_backend_label(backend: &GraphCompilerBackendProjectionData) -> String {
    match &backend.kind {
        GraphCompilerBackendKindProjectionData::GeneratedRust { abi, .. } => {
            format!("rust {}", graph_generated_rust_abi_label(*abi))
        }
        GraphCompilerBackendKindProjectionData::PackedIr { ir_schema } => {
            format!("packed ir {ir_schema}")
        }
        GraphCompilerBackendKindProjectionData::ShaderPipeline { pipeline_kind } => {
            format!("shader {pipeline_kind}")
        }
        GraphCompilerBackendKindProjectionData::External { kind, .. } => kind.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphGeneratedRustCompilerDetail {
    package: String,
    entry_symbol: String,
    abi: String,
}

fn graph_generated_rust_compiler_detail(
    graph_type: &GraphTypeCreationProjectionData,
) -> Option<GraphGeneratedRustCompilerDetail> {
    let backend = graph_type.compiler_backend.as_ref()?;
    let GraphCompilerBackendKindProjectionData::GeneratedRust {
        package,
        entry_symbol,
        abi,
    } = &backend.kind
    else {
        return None;
    };
    Some(GraphGeneratedRustCompilerDetail {
        package: package.clone(),
        entry_symbol: entry_symbol.clone(),
        abi: graph_generated_rust_abi_label(*abi).to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphAotRuntimeDetail {
    package: String,
    entry_symbol: String,
    context_type: String,
}

fn graph_aot_runtime_detail(
    graph_type: &GraphTypeCreationProjectionData,
) -> Option<GraphAotRuntimeDetail> {
    let strategy = graph_type.runtime_execution_strategy.as_ref()?;
    let GraphRuntimeExecutionStrategyProjectionData::AotCompiledCode {
        package,
        entry_symbol,
        context_type,
        ..
    } = strategy
    else {
        return None;
    };
    Some(GraphAotRuntimeDetail {
        package: package.clone(),
        entry_symbol: entry_symbol.clone(),
        context_type: context_type.clone(),
    })
}

const fn graph_generated_rust_abi_label(abi: GraphGeneratedRustAbiProjectionData) -> &'static str {
    match abi {
        GraphGeneratedRustAbiProjectionData::ContextSchedule => "context schedule",
        GraphGeneratedRustAbiProjectionData::TypedDataflow => "typed dataflow",
    }
}

fn graph_runtime_execution_label(strategy: &GraphRuntimeExecutionStrategyProjectionData) -> String {
    match strategy {
        GraphRuntimeExecutionStrategyProjectionData::PackedIr => "packed ir runtime".to_string(),
        GraphRuntimeExecutionStrategyProjectionData::AotCompiledCode {
            language,
            entry_symbol,
            ..
        } => format!("aot {language} {entry_symbol}"),
        GraphRuntimeExecutionStrategyProjectionData::HotReloadedCompiledModule {
            abi,
            entry_symbol,
        } => format!("hot reload {abi} {entry_symbol}"),
        GraphRuntimeExecutionStrategyProjectionData::ShaderPipeline { pipeline_kind } => {
            format!("shader {pipeline_kind}")
        }
        GraphRuntimeExecutionStrategyProjectionData::External { kind, .. } => kind.clone(),
    }
}

/// One element per authored node, positioned by the canvas transform.
fn graph_node_elements(
    document: &GraphDocumentProjectionData,
    transform: GraphCanvasTransform,
    selected_node_id: Option<&str>,
    pending_output_port: Option<&PendingGraphPortConnection>,
    active_node_drag: Option<&GraphNodeDragState>,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, VisualGraphPanel>,
) -> Vec<gpui::AnyElement> {
    document
        .nodes
        .iter()
        .map(|node| {
            render_graph_node(
                node,
                transform,
                selected_node_id,
                pending_output_port,
                active_node_drag,
                theme,
                cx,
            )
        })
        .collect()
}

/// One element per authored comment box, positioned by the canvas transform.
fn graph_comment_elements(
    document: &GraphDocumentProjectionData,
    transform: GraphCanvasTransform,
    selected_comment_id: Option<&str>,
    active_comment_drag: Option<&GraphCommentDragState>,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, VisualGraphPanel>,
) -> Vec<gpui::AnyElement> {
    document
        .comments
        .iter()
        .map(|comment| {
            render_graph_comment(
                comment,
                transform,
                selected_comment_id,
                active_comment_drag,
                theme,
                cx,
            )
        })
        .collect()
}

/// Canvas-level pointer wiring: click-to-clear, wheel zoom, the three pan /
/// context-menu button paths, drag tracking, and the palette drop target.
fn wire_graph_canvas_input(
    canvas: gpui::Stateful<gpui::Div>,
    drop_target: gpui::Hsla,
    cx: &Context<'_, VisualGraphPanel>,
) -> gpui::Stateful<gpui::Div> {
    canvas
        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
            this.handle_graph_canvas_click(cx);
        }))
        .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
            cx.stop_propagation();
            this.handle_graph_canvas_scroll_zoom(event, cx);
        }))
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
                this.handle_graph_canvas_pan_mouse_down(event, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                if this.space_pan_key_held {
                    cx.stop_propagation();
                    this.handle_graph_canvas_pan_mouse_down(event, cx);
                }
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                this.handle_graph_canvas_context_menu_mouse_down(event, cx);
            }),
        )
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
            this.handle_graph_canvas_mouse_move(event, cx);
        }))
        .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, window, cx| {
            this.handle_graph_canvas_mouse_up(event, window, cx);
        }))
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|this, event: &MouseUpEvent, window, cx| {
                this.handle_graph_canvas_mouse_up(event, window, cx);
            }),
        )
        .on_mouse_up_out(
            MouseButton::Middle,
            cx.listener(|this, event: &MouseUpEvent, window, cx| {
                this.handle_graph_canvas_mouse_up(event, window, cx);
            }),
        )
        .drag_over::<GraphPaletteDragPayload>(move |style, _, _, _| style.bg(drop_target))
        .on_drop(
            cx.listener(|this, payload: &GraphPaletteDragPayload, window, cx| {
                this.handle_graph_palette_drop(payload, window, cx);
            }),
        )
}
#[allow(clippy::too_many_arguments)]
// canvas render needs full selection/drag/viewport state; bundling into a struct adds churn without clarity
fn render_graph_canvas(
    projection: &EditorGraphDocumentProjection,
    selected_node_id: Option<&str>,
    selected_comment_id: Option<&str>,
    pending_output_port: Option<&PendingGraphPortConnection>,
    viewport: GraphViewportState,
    active_node_drag: Option<&GraphNodeDragState>,
    active_route_anchor_drag: Option<&GraphRouteAnchorDragState>,
    active_comment_drag: Option<&GraphCommentDragState>,
    space_pan_key_held: bool,
    panning: bool,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, VisualGraphPanel>,
) -> gpui::AnyElement {
    if let Some(error) = &projection.status_error {
        return render_graph_error(error, theme).into_any_element();
    }
    let Some(document) = projection.document.as_ref() else {
        return render_empty_graph_state(theme).into_any_element();
    };

    let bounds = graph_canvas_bounds(document);
    let transform = GraphCanvasTransform::from_bounds(bounds, viewport);
    let node_elements = graph_node_elements(
        document,
        transform,
        selected_node_id,
        pending_output_port,
        active_node_drag,
        theme,
        cx,
    );
    let comment_elements = graph_comment_elements(
        document,
        transform,
        selected_comment_id,
        active_comment_drag,
        theme,
        cx,
    );
    let panel = cx.entity().downgrade();

    div()
        .id("graph-canvas")
        .relative()
        .size_full()
        .bg(theme.background)
        .when(panning, gpui::Styled::cursor_grabbing)
        .when(space_pan_key_held && !panning, gpui::Styled::cursor_grab)
        .on_prepaint({
            let entity = cx.entity();
            move |bounds, _, cx| {
                entity.update(cx, |this, _| {
                    this.canvas_bounds = bounds;
                });
            }
        })
        .map(|canvas| wire_graph_canvas_input(canvas, theme.drop_target, cx))
        .children(comment_elements)
        .child(render_graph_bezier_wires(document, transform, theme))
        .children(
            document
                .connections
                .iter()
                .filter(|connection| connection_is_routed(connection))
                .flat_map(|connection| render_connection_segments(connection, transform, theme)),
        )
        .children(document.connections.iter().flat_map(|connection| {
            render_connection_route_anchors(
                connection,
                transform,
                active_route_anchor_drag,
                theme,
                cx,
            )
        }))
        .children(node_elements)
        .child(render_graph_diagnostics(document, theme))
        // Built when the menu is actually opened, not on every repaint: the
        // palette label/category/node-type strings are cloned once per open.
        .context_menu(move |menu, _window, cx| {
            let (items, fallback_position) = {
                let projection = cx
                    .try_global::<EditorGraphDocumentProjection>()
                    .unwrap_or(&NO_GRAPH_PROJECTION);
                (
                    graph_context_menu_items(&projection.node_palette),
                    next_node_position(projection.document.as_ref()),
                )
            };
            build_graph_canvas_context_menu(menu, &items, &panel, fallback_position, cx)
        })
        .into_any_element()
}

/// Palette entry snapshot captured for the canvas right-click add-node menu.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphContextMenuItem {
    node_type: String,
    version: u32,
    label: String,
    category: String,
}

fn graph_context_menu_items(palette: &GraphNodePaletteProjectionData) -> Vec<GraphContextMenuItem> {
    palette
        .nodes
        .iter()
        .map(|node| GraphContextMenuItem {
            node_type: node.node_type.clone(),
            version: node.version,
            label: node.label.clone(),
            category: node.category.clone(),
        })
        .collect()
}

/// Picks the document-space position for a context-menu node insertion:
/// the recorded right-click position when available, otherwise the same
/// fan-out fallback the sidebar palette uses.
fn graph_context_menu_add_position(
    recorded: Option<GraphPointProjectionData>,
    fallback: (f32, f32),
) -> (f32, f32) {
    recorded
        .filter(|position| position.x.is_finite() && position.y.is_finite())
        .map_or(fallback, |position| (position.x, position.y))
}

fn build_graph_canvas_context_menu(
    menu: PopupMenu,
    items: &[GraphContextMenuItem],
    panel: &WeakEntity<VisualGraphPanel>,
    fallback_position: (f32, f32),
    cx: &App,
) -> PopupMenu {
    let recorded = panel
        .upgrade()
        .and_then(|panel| panel.read(cx).context_menu_document_position);
    let (x, y) = graph_context_menu_add_position(recorded, fallback_position);
    let mut menu = menu.scrollable(true).max_h(px(420.0)).label("Add Node");
    if items.is_empty() {
        return menu.label("No node types match this graph");
    }
    let mut last_category: Option<&str> = None;
    for item in items {
        if last_category != Some(item.category.as_str()) {
            menu = menu.separator().label(item.category.clone());
            last_category = Some(item.category.as_str());
        }
        menu = menu.menu(
            item.label.clone(),
            Box::new(crate::actions::AddGraphNode {
                node_type: item.node_type.clone(),
                node_type_version: item.version,
                x,
                y,
            }),
        );
    }
    menu
}

fn render_graph_comment(
    comment: &GraphCommentProjectionData,
    transform: GraphCanvasTransform,
    selected_comment_id: Option<&str>,
    active_comment_drag: Option<&GraphCommentDragState>,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, VisualGraphPanel>,
) -> gpui::AnyElement {
    let position = transform.point(graph_comment_document_position(
        comment,
        active_comment_drag,
    ));
    let dragging = graph_comment_is_dragging(comment, active_comment_drag);
    let drag_start = GraphCommentDragStartData {
        comment_id: comment.comment_id.clone(),
        comment_x: comment.x,
        comment_y: comment.y,
        comment_width: comment.width,
        comment_height: comment.height,
    };
    let click = GraphCommentClickData {
        comment_id: comment.comment_id.clone(),
    };

    div()
        .id(gpui::SharedString::from(format!(
            "graph-comment-{}",
            graph_element_key(&comment.comment_id)
        )))
        .absolute()
        .left(px(position.x))
        .top(px(position.y))
        .w(px(transform.length(comment.width.max(96.0))))
        .h(px(transform.length(comment.height.max(56.0))))
        .rounded_sm()
        .border_1()
        .border_color(if dragging {
            theme.warning
        } else if graph_comment_is_selected(comment, selected_comment_id) {
            theme.accent
        } else {
            theme.border
        })
        .bg(theme.muted)
        .opacity(0.88)
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
                this.handle_graph_comment_mouse_down(&drag_start, event, cx);
            }),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            this.handle_graph_comment_click(&click, window, cx);
        }))
        .child(
            div()
                .size_full()
                .p_2()
                .text_xs()
                .text_color(theme.foreground)
                .child(comment.text.clone()),
        )
        .into_any_element()
}

fn graph_comment_is_dragging(
    comment: &GraphCommentProjectionData,
    active_comment_drag: Option<&GraphCommentDragState>,
) -> bool {
    active_comment_drag.is_some_and(|drag| drag.comment_id == comment.comment_id)
}

fn graph_comment_is_selected(
    comment: &GraphCommentProjectionData,
    selected_comment_id: Option<&str>,
) -> bool {
    comment.selected || selected_comment_id == Some(comment.comment_id.as_str())
}

fn graph_comment_document_position(
    comment: &GraphCommentProjectionData,
    active_comment_drag: Option<&GraphCommentDragState>,
) -> GraphPointProjectionData {
    active_comment_drag
        .filter(|drag| drag.comment_id == comment.comment_id)
        .map_or_else(
            || GraphPointProjectionData::new(comment.x, comment.y),
            |drag| GraphPointProjectionData::new(drag.preview_x, drag.preview_y),
        )
}

fn render_graph_node(
    node: &GraphNodeProjectionData,
    transform: GraphCanvasTransform,
    selected_node_id: Option<&str>,
    pending_output_port: Option<&PendingGraphPortConnection>,
    active_node_drag: Option<&GraphNodeDragState>,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, VisualGraphPanel>,
) -> gpui::AnyElement {
    let dragging = graph_node_is_dragging(node, active_node_drag);
    let border = if dragging {
        theme.warning
    } else if graph_node_is_selected(node, selected_node_id) {
        theme.accent
    } else {
        theme.border
    };
    let click = GraphNodeClickData {
        node_id: node.node_id.clone(),
    };
    let drag_start = GraphNodeDragStartData {
        node_id: node.node_id.clone(),
        node_x: node.x,
        node_y: node.y,
    };
    let top_left = transform.point(graph_node_document_position(node, active_node_drag));
    div()
        .id(gpui::SharedString::from(format!(
            "graph-node-{}",
            graph_element_key(&node.node_id)
        )))
        .absolute()
        .left(px(top_left.x))
        .top(px(top_left.y))
        .w(px(transform.length(node.width.max(120.0))))
        .h(px(transform.length(node.height.max(56.0))))
        .rounded_sm()
        .border_1()
        .border_color(border)
        .bg(theme.popover)
        .shadow_sm()
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
                this.handle_graph_node_mouse_down(&drag_start, event, cx);
            }),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            this.handle_graph_node_click(&click, window, cx);
        }))
        .child(
            v_flex()
                .size_full()
                .gap_1()
                .p_2()
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .text_color(theme.foreground)
                                .child(node.label.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(format!("{} ports", node.ports.len())),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(node.node_type.clone()),
                )
                .child(render_port_strip(node, theme)),
        )
        .children(render_port_handles(
            node,
            transform,
            active_node_drag,
            pending_output_port,
            theme,
            cx,
        ))
        .into_any_element()
}

fn render_node_nudge_controls(
    node: &GraphNodeProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex().items_center().gap_1().children(
        GRAPH_NODE_NUDGE_CONTROLS
            .iter()
            .map(|control| render_node_nudge_button(node, *control, theme).into_any_element()),
    )
}

fn render_node_nudge_button(
    node: &GraphNodeProjectionData,
    control: GraphNodeNudgeControl,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let node_id = node.node_id.clone();
    let (x, y) = moved_node_position(node, control.dx, control.dy);
    div()
        .id(gpui::SharedString::from(format!(
            "graph-node-{}-move-{}",
            graph_element_key(&node.node_id),
            control.id_label,
        )))
        .w(px(32.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .text_xs()
        .whitespace_nowrap()
        .text_color(theme.foreground)
        .hover(|this| this.bg(theme.muted))
        .cursor_pointer()
        .child(control.label)
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(
                Box::new(crate::actions::MoveGraphNode {
                    node_id: node_id.clone(),
                    x,
                    y,
                }),
                cx,
            );
        })
}

fn render_selected_graph_node_transform(
    node: &GraphNodeProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.foreground)
                        .child("Transform"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "x:{} y:{}",
                            graph_scalar_label(node.x),
                            graph_scalar_label(node.y)
                        )),
                ),
        )
        .child(render_node_nudge_controls(node, theme))
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!(
                    "w:{} h:{}",
                    graph_scalar_label(node.width),
                    graph_scalar_label(node.height)
                )),
        )
}

fn moved_node_position(node: &GraphNodeProjectionData, dx: f32, dy: f32) -> (f32, f32) {
    (node.x + dx, node.y + dy)
}

fn graph_node_is_selected(node: &GraphNodeProjectionData, selected_node_id: Option<&str>) -> bool {
    node.selected || selected_node_id == Some(node.node_id.as_str())
}

fn graph_node_is_dragging(
    node: &GraphNodeProjectionData,
    active_node_drag: Option<&GraphNodeDragState>,
) -> bool {
    active_node_drag.is_some_and(|drag| drag.node_id == node.node_id)
}

fn graph_node_document_position(
    node: &GraphNodeProjectionData,
    active_node_drag: Option<&GraphNodeDragState>,
) -> GraphPointProjectionData {
    active_node_drag
        .filter(|drag| drag.node_id == node.node_id)
        .map_or_else(
            || GraphPointProjectionData::new(node.x, node.y),
            |drag| GraphPointProjectionData::new(drag.preview_x, drag.preview_y),
        )
}

fn selected_graph_node<'a>(
    document: Option<&'a GraphDocumentProjectionData>,
    selected_node_id: Option<&str>,
) -> Option<&'a GraphNodeProjectionData> {
    let selected_node_id = selected_node_id?;
    document?
        .nodes
        .iter()
        .find(|node| node.node_id == selected_node_id)
}

fn selected_graph_comment<'a>(
    document: Option<&'a GraphDocumentProjectionData>,
    selected_comment_id: Option<&str>,
) -> Option<&'a GraphCommentProjectionData> {
    let selected_comment_id = selected_comment_id?;
    document?
        .comments
        .iter()
        .find(|comment| comment.comment_id == selected_comment_id)
}

/// The selected comment's editable text field, described without borrowing the
/// projection. These are the same two strings the input already cloned into its
/// keyed state and its enter-key handler.
struct GraphCommentTextInputRequest {
    comment_id: String,
    text: String,
}

/// One editable reflected port value on the selected node, described without
/// borrowing the projection. Every field here was already cloned by the input
/// itself; describing the input first only moves those clones earlier.
struct GraphPortValueInputRequest {
    node_id: String,
    port_id: u32,
    envelope: ReflectedValueEnvelope,
    family: WidgetFamily,
    edit_text: String,
}

#[derive(Default)]
struct GraphInspectorInputRequests {
    comment: Option<GraphCommentTextInputRequest>,
    ports: Vec<GraphPortValueInputRequest>,
}

/// Text-input elements acquired up front, drained as the inspector tree is
/// assembled from borrows.
#[derive(Default)]
struct GraphInspectorInputs {
    comment: Option<gpui::AnyElement>,
    ports: Vec<(u32, gpui::AnyElement)>,
}

impl GraphInspectorInputs {
    fn take_port(&mut self, port_id: u32) -> Option<gpui::AnyElement> {
        let index = self.ports.iter().position(|(id, _)| *id == port_id)?;
        Some(self.ports.remove(index).1)
    }
}

/// Describes the inspector's editable inputs for the current selection, using
/// the same node-wins-over-comment precedence the inspector body renders with.
fn graph_inspector_input_requests(
    document: Option<&GraphDocumentProjectionData>,
    selected_node_id: Option<&str>,
    selected_comment_id: Option<&str>,
) -> GraphInspectorInputRequests {
    if let Some(node) = selected_graph_node(document, selected_node_id) {
        return GraphInspectorInputRequests {
            comment: None,
            ports: node
                .ports
                .iter()
                .filter_map(|port| graph_port_value_input_request(node, port))
                .collect(),
        };
    }
    let Some(comment) = selected_graph_comment(document, selected_comment_id) else {
        return GraphInspectorInputRequests::default();
    };
    GraphInspectorInputRequests {
        comment: Some(GraphCommentTextInputRequest {
            comment_id: comment.comment_id.clone(),
            text: comment.text.clone(),
        }),
        ports: Vec::new(),
    }
}

/// A port gets a text input only when it carries a decodable typed-RON value
/// that is not rendered by the boolean toggle instead.
fn graph_port_value_input_request(
    node: &GraphNodeProjectionData,
    port: &GraphPortProjectionData,
) -> Option<GraphPortValueInputRequest> {
    let value = port.value.as_ref()?;
    let envelope = graph_input_value_active(value)?;
    let (family, edit_text) = graph_reflected_value_edit_state(envelope)?;
    if family == WidgetFamily::Bool {
        return None;
    }
    Some(GraphPortValueInputRequest {
        node_id: node.node_id.clone(),
        port_id: port.port_id,
        envelope: envelope.clone(),
        family,
        edit_text,
    })
}

fn acquire_graph_inspector_inputs(
    requests: GraphInspectorInputRequests,
    window: &mut Window,
    cx: &mut Context<'_, VisualGraphPanel>,
) -> GraphInspectorInputs {
    GraphInspectorInputs {
        comment: requests
            .comment
            .map(|request| acquire_graph_comment_text_input(request, window, cx)),
        ports: requests
            .ports
            .into_iter()
            .map(|request| {
                let port_id = request.port_id;
                (port_id, acquire_graph_port_value_input(request, window, cx))
            })
            .collect(),
    }
}

/// Renders the right-hand inspector column in three phases.
///
/// The editable text inputs are the only elements in this panel that need
/// `&mut Window` / `&mut Context`. Phase 1 describes them under a short-lived
/// projection borrow, phase 2 acquires them, and phase 3 assembles the tree
/// while borrowing the projection and theme — so no phase has to own a copy of
/// the projection.
fn render_graph_selection_inspector(
    selected_node_id: Option<&str>,
    selected_comment_id: Option<&str>,
    window: &mut Window,
    cx: &mut Context<'_, VisualGraphPanel>,
) -> gpui::AnyElement {
    let requests = {
        let projection = cx
            .try_global::<EditorGraphDocumentProjection>()
            .unwrap_or(&NO_GRAPH_PROJECTION);
        graph_inspector_input_requests(
            projection.document.as_ref(),
            selected_node_id,
            selected_comment_id,
        )
    };
    let mut inputs = acquire_graph_inspector_inputs(requests, window, cx);

    let theme = cx.theme();
    let projection = cx
        .try_global::<EditorGraphDocumentProjection>()
        .unwrap_or(&NO_GRAPH_PROJECTION);
    let document = projection.document.as_ref();
    let build_status = projection.build_status.as_ref();
    let selected_node = selected_graph_node(document, selected_node_id);
    let selected_comment = selected_graph_comment(document, selected_comment_id);
    let content = match (selected_node, selected_comment) {
        (Some(node), _) => render_selected_graph_node_inspector(node, theme, &mut inputs),
        (None, Some(comment)) => {
            render_selected_graph_comment_inspector(comment, theme, &mut inputs)
        }
        (None, None) => document.map_or_else(
            || {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("No selection")
                    .into_any_element()
            },
            |document| render_graph_document_inspector(document, theme).into_any_element(),
        ),
    };
    div()
        .w(px(300.0))
        .h_full()
        .border_l_1()
        .border_color(theme.border)
        .bg(theme.background)
        .p_2()
        .overflow_hidden()
        .child(
            v_flex()
                .size_full()
                .gap_2()
                .child(content)
                .when_some(build_status, |this, status| {
                    this.child(render_graph_build_status_inspector(status, theme))
                }),
        )
        .into_any_element()
}

fn render_graph_document_inspector(
    document: &GraphDocumentProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .size_full()
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .text_color(theme.foreground)
                        .truncate()
                        .child(crate::naming::display_name(&document.document_id).into_owned()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(document.graph_type.clone()),
                ),
        )
        .child(render_graph_document_revision_detail(document, theme))
        .when_some(document.graph_type_info.as_ref(), |this, graph_type| {
            this.child(render_graph_type_contract_detail(graph_type, theme))
        })
        .child(render_graph_document_counts(document, theme))
}

fn render_graph_document_revision_detail(
    document: &GraphDocumentProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let saved = document
        .saved_revision
        .map_or_else(|| "none".to_string(), |saved| saved.to_string());
    v_flex()
        .gap_1()
        .child(render_graph_detail_row(
            "Revision",
            format!("current {} / saved {saved}", document.revision),
            theme,
        ))
        .child(render_graph_detail_row(
            "State",
            if document.unsaved_changes {
                "dirty"
            } else {
                "saved"
            },
            theme,
        ))
        .child(render_graph_detail_row(
            "Catalog",
            document.catalog_version.to_string(),
            theme,
        ))
}

fn graph_type_contract_compiler_label(graph_type: &GraphTypeCreationProjectionData) -> String {
    graph_type
        .compiler_backend
        .as_ref()
        .map_or_else(|| "none".to_string(), graph_compiler_backend_label)
}

fn graph_type_contract_runtime_label(graph_type: &GraphTypeCreationProjectionData) -> String {
    graph_type
        .runtime_execution_strategy
        .as_ref()
        .map(graph_runtime_execution_label)
        .or_else(|| graph_type.runtime_product_kind.clone())
        .unwrap_or_else(|| {
            if graph_type.editor_interpreted {
                "editor interpreted".to_string()
            } else {
                "none".to_string()
            }
        })
}

fn render_graph_type_contract_detail(
    graph_type: &GraphTypeCreationProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let compiler = graph_type_contract_compiler_label(graph_type);
    let runtime = graph_type_contract_runtime_label(graph_type);

    v_flex()
        .gap_1()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .text_color(theme.foreground)
                        .truncate()
                        .child(graph_type.label.clone()),
                )
                .child(render_graph_badge(
                    graph_graph_type_mode_label(graph_type),
                    theme,
                )),
        )
        .child(render_graph_detail_row(
            "Category",
            graph_type.category.clone(),
            theme,
        ))
        .child(render_graph_detail_row(
            "Source",
            graph_type_source_label(graph_type),
            theme,
        ))
        .child(render_graph_detail_row("Compiler", compiler, theme))
        .child(render_graph_detail_row("Runtime", runtime, theme))
        .when_some(
            graph_type.runtime_product_asset_type.as_ref(),
            |this, asset_type| {
                this.child(render_graph_detail_row(
                    "Asset Type",
                    asset_type.clone(),
                    theme,
                ))
            },
        )
        .when_some(graph_type.runtime_product_kind.as_ref(), |this, product| {
            this.child(render_graph_detail_row("Product", product.clone(), theme))
        })
        .when_some(
            graph_runtime_product_traits_label(graph_type),
            |this, traits| this.child(render_graph_detail_row("Traits", traits, theme)),
        )
        .when_some(
            graph_generated_rust_compiler_detail(graph_type),
            |this, detail| {
                this.child(render_graph_detail_row("Rust ABI", detail.abi, theme))
                    .child(render_graph_detail_row(
                        "Compiler Pkg",
                        detail.package,
                        theme,
                    ))
                    .child(render_graph_detail_row(
                        "Compiler Fn",
                        detail.entry_symbol,
                        theme,
                    ))
            },
        )
        .when_some(graph_aot_runtime_detail(graph_type), |this, detail| {
            this.child(render_graph_detail_row(
                "Runtime Pkg",
                detail.package,
                theme,
            ))
            .child(render_graph_detail_row(
                "Runtime Fn",
                detail.entry_symbol,
                theme,
            ))
            .child(render_graph_detail_row(
                "Context",
                detail.context_type,
                theme,
            ))
        })
        .when_some(graph_type.compiler_backend.as_ref(), |this, backend| {
            this.when(!backend.capability_markers.is_empty(), |this| {
                this.child(render_graph_backend_markers(backend, theme))
            })
        })
}

fn render_graph_document_counts(
    document: &GraphDocumentProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(render_graph_detail_row(
            "Nodes",
            document.nodes.len().to_string(),
            theme,
        ))
        .child(render_graph_detail_row(
            "Edges",
            document.connections.len().to_string(),
            theme,
        ))
        .child(render_graph_detail_row(
            "Notes",
            document.comments.len().to_string(),
            theme,
        ))
}

fn render_graph_build_status_inspector(
    status: &GraphBuildStatusProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .text_color(theme.foreground)
                .child("Build"),
        )
        .child(render_graph_detail_row(
            "Source",
            status.source_path.clone(),
            theme,
        ))
        .child(render_graph_detail_row(
            "Status",
            status.source_status.label(),
            theme,
        ))
        .child(render_graph_detail_row(
            "Asset",
            status.asset_guid.clone(),
            theme,
        ))
        .child(render_graph_detail_row(
            "Entry",
            status.entry_id.to_string(),
            theme,
        ))
        .child(render_graph_detail_row(
            "Hash",
            status.content_hash.clone(),
            theme,
        ))
        .when_some(status.latest_job.as_ref(), |this, job| {
            this.child(render_graph_detail_row(
                "Job",
                graph_build_job_label(job),
                theme,
            ))
        })
}

fn render_graph_detail_row(
    label: &'static str,
    value: impl Into<String>,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .items_start()
        .justify_between()
        .gap_2()
        .py_0p5()
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .w(px(76.0))
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(theme.foreground)
                .child(value.into()),
        )
}

const fn graph_graph_type_mode_label(graph_type: &GraphTypeCreationProjectionData) -> &'static str {
    match (graph_type.runtime_compiled, graph_type.editor_interpreted) {
        (true, true) => "runtime+editor",
        (true, false) => "runtime",
        (false, true) => "editor",
        (false, false) => "descriptor",
    }
}

fn graph_runtime_product_traits_label(
    graph_type: &GraphTypeCreationProjectionData,
) -> Option<String> {
    let streamable = graph_type.runtime_product_streamable?;
    let diffable_chunks = graph_type.runtime_product_diffable_chunks?;
    Some(format!(
        "{} / {}",
        if streamable {
            "streamable"
        } else {
            "not streamable"
        },
        if diffable_chunks {
            "diffable chunks"
        } else {
            "monolithic chunks"
        }
    ))
}

fn graph_build_job_label(job: &GraphBuildJobProjectionData) -> String {
    format!(
        "{}:{} #{} {}",
        job.job_key,
        job.platform,
        job.ordinal
            .map_or_else(|| "-".to_owned(), |ordinal| ordinal.to_string()),
        job.status.label()
    )
}

fn graph_type_source_label(graph_type: &GraphTypeCreationProjectionData) -> String {
    let extension = graph_type.default_extension.trim().trim_start_matches('.');
    let prefix = graph_type
        .default_path_prefix
        .trim()
        .trim_matches('/')
        .trim_matches('\\');
    match (prefix.is_empty(), extension.is_empty()) {
        (true, true) => "project document".to_string(),
        (true, false) => format!("*.{extension}"),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}/*.{extension}"),
    }
}

fn render_selected_graph_node_inspector(
    node: &GraphNodeProjectionData,
    theme: &gpui_component::theme::Theme,
    inputs: &mut GraphInspectorInputs,
) -> gpui::AnyElement {
    v_flex()
        .size_full()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .text_color(theme.foreground)
                .child(node.label.clone()),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .truncate()
                .child(node.node_type.clone()),
        )
        .child(render_graph_node_remove_button(node, theme))
        .child(render_selected_graph_node_transform(node, theme))
        .child(render_selected_graph_node_source_links(node, theme))
        .child(render_selected_graph_node_ports(node, theme, inputs))
        .into_any_element()
}

fn render_selected_graph_comment_inspector(
    comment: &GraphCommentProjectionData,
    theme: &gpui_component::theme::Theme,
    inputs: &mut GraphInspectorInputs,
) -> gpui::AnyElement {
    v_flex()
        .size_full()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .text_color(theme.foreground)
                .child("Comment"),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .truncate()
                .child(comment.comment_id.clone()),
        )
        .children(inputs.comment.take())
        .child(render_graph_comment_remove_button(comment, theme))
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!(
                    "x:{} y:{} w:{} h:{}",
                    graph_scalar_label(comment.x),
                    graph_scalar_label(comment.y),
                    graph_scalar_label(comment.width),
                    graph_scalar_label(comment.height)
                )),
        )
        .into_any_element()
}

fn render_graph_node_remove_button(
    node: &GraphNodeProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let node_id = node.node_id.clone();
    graph_remove_button(
        gpui::SharedString::from(format!(
            "graph-node-{}-remove",
            graph_element_key(&node.node_id)
        )),
        "delete node",
        theme,
        move |window, cx| {
            window.dispatch_action(
                Box::new(crate::actions::RemoveGraphNode {
                    node_id: node_id.clone(),
                }),
                cx,
            );
        },
    )
}

fn render_graph_comment_remove_button(
    comment: &GraphCommentProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let comment_id = comment.comment_id.clone();
    graph_remove_button(
        gpui::SharedString::from(format!(
            "graph-comment-{}-remove",
            graph_element_key(&comment.comment_id)
        )),
        "delete comment",
        theme,
        move |window, cx| {
            window.dispatch_action(
                Box::new(crate::actions::RemoveGraphComment {
                    comment_id: comment_id.clone(),
                }),
                cx,
            );
        },
    )
}

fn graph_remove_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    theme: &gpui_component::theme::Theme,
    on_remove: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_sm()
        .border_1()
        .border_color(theme.warning)
        .bg(theme.background)
        .hover(|this| this.bg(theme.muted))
        .cursor_pointer()
        .text_xs()
        .text_color(theme.warning)
        .child(label)
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_remove(window, cx);
        })
}

fn acquire_graph_comment_text_input(
    request: GraphCommentTextInputRequest,
    window: &mut Window,
    cx: &mut Context<'_, VisualGraphPanel>,
) -> gpui::AnyElement {
    let GraphCommentTextInputRequest { comment_id, text } = request;
    let key = gpui::SharedString::from(format!(
        "graph-comment-text-input-{}",
        graph_element_key(&comment_id)
    ));
    let state = window.use_keyed_state(key, cx, {
        let text = text.clone();
        move |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
            let subscription = cx.subscribe_in(&input, window, {
                move |_: &mut GraphTextInputState,
                      input: &Entity<InputState>,
                      event: &InputEvent,
                      window: &mut Window,
                      cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        window.dispatch_action(
                            Box::new(crate::actions::SetGraphCommentText {
                                comment_id: comment_id.clone(),
                                text: input.read(cx).value().to_string(),
                            }),
                            cx,
                        );
                    }
                }
            });
            GraphTextInputState {
                input,
                _subscription: subscription,
            }
        }
    });

    let input = state.read(cx).input.clone();
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    if !focused && input.read(cx).value() != text {
        input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
        });
    }
    let theme = cx.theme();

    div()
        .h(px(28.0))
        .px_2()
        .border_1()
        .border_color(theme.border)
        .bg(theme.input_background())
        .flex()
        .items_center()
        .child(
            Input::new(&input)
                .small()
                .appearance(false)
                .bordered(false)
                .focus_bordered(false),
        )
        .into_any_element()
}

fn render_selected_graph_node_source_links(
    node: &GraphNodeProjectionData,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    if node.source_links.is_empty() {
        return div().into_any_element();
    }

    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_semibold()
                .text_color(theme.foreground)
                .child(format!("Source Links {}", node.source_links.len())),
        )
        .children(
            node.source_links
                .iter()
                .enumerate()
                .map(|(index, link)| render_graph_source_link_row(node, index, link, theme)),
        )
        .into_any_element()
}

fn render_graph_source_link_row(
    node: &GraphNodeProjectionData,
    index: usize,
    link: &GraphNodeSourceLinkProjectionData,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    let action = crate::actions::OpenGraphNodeSourceLink {
        package: link.package.clone(),
        module_path: link.module_path.clone(),
        symbol_path: link.symbol_path.clone(),
        file: link.file.clone(),
        line: link.line,
        column: link.column,
        docs_url: link.docs_url.clone(),
    };
    div()
        .id(gpui::SharedString::from(format!(
            "graph-node-{}-source-{index}",
            graph_element_key(&node.node_id)
        )))
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .p_2()
        .hover(|this| this.bg(theme.muted))
        .cursor_pointer()
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.foreground)
                        .truncate()
                        .child(graph_source_link_label(link)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(graph_source_link_detail(link)),
                ),
        )
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(Box::new(action.clone()), cx);
        })
        .into_any_element()
}

fn graph_source_link_label(link: &GraphNodeSourceLinkProjectionData) -> String {
    link.symbol_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            link.module_path
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            link.file
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            link.docs_url
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or("source")
        .to_string()
}

fn graph_source_link_detail(link: &GraphNodeSourceLinkProjectionData) -> String {
    let mut parts = Vec::new();
    if let Some(package) = link
        .package
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(package.to_string());
    }
    if let Some(file) = link
        .file
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(graph_source_file_location(file, link.line, link.column));
    } else if let Some(module_path) = link
        .module_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(module_path.to_string());
    }
    if let Some(docs_url) = link
        .docs_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(docs_url.to_string());
    }
    if parts.is_empty() {
        "descriptor source target".to_string()
    } else {
        parts.join(" | ")
    }
}

fn graph_source_file_location(file: &str, line: Option<u32>, column: Option<u32>) -> String {
    match (line, column) {
        (Some(line), Some(column)) => format!("{file}:{line}:{column}"),
        (Some(line), None) => format!("{file}:{line}"),
        _ => file.to_string(),
    }
}

fn render_selected_graph_node_ports(
    node: &GraphNodeProjectionData,
    theme: &gpui_component::theme::Theme,
    inputs: &mut GraphInspectorInputs,
) -> gpui::AnyElement {
    let input_count = node
        .ports
        .iter()
        .filter(|port| port.direction == GraphPortDirectionData::Input)
        .count();
    let output_count = node.ports.len().saturating_sub(input_count);
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_semibold()
                .text_color(theme.foreground)
                .child(format!("Ports {input_count}/{output_count}")),
        )
        .child(render_input_value_controls(node, theme, inputs))
        .into_any_element()
}

fn graph_scalar_label(value: f32) -> String {
    if value.fract().abs() <= f32::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn render_input_value_controls(
    node: &GraphNodeProjectionData,
    theme: &gpui_component::theme::Theme,
    inputs: &mut GraphInspectorInputs,
) -> gpui::AnyElement {
    v_flex()
        .gap_1()
        .children(node.ports.iter().filter_map(|port| {
            let value = port.value.as_ref()?;
            Some(render_input_value_control(node, port, value, theme, inputs))
        }))
        .into_any_element()
}

fn render_input_value_control(
    node: &GraphNodeProjectionData,
    port: &GraphPortProjectionData,
    value: &GraphInputValueProjectionData,
    theme: &gpui_component::theme::Theme,
    inputs: &mut GraphInspectorInputs,
) -> gpui::AnyElement {
    let current_label = value
        .current_value
        .as_ref()
        .map_or_else(|| "unset".to_string(), graph_reflected_value_label);
    let toggle = graph_input_value_toggle_edit(value);
    let decrement = graph_input_value_step_edit(value, -1);
    let increment = graph_input_value_step_edit(value, 1);
    let reflected_input = inputs.take_port(port.port_id);
    h_flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .truncate()
                .child(format!("{} = {current_label}", port.name)),
        )
        .when_some(reflected_input, gpui::ParentElement::child)
        .when_some(decrement, |this, next_value| {
            this.child(render_input_value_action_button(
                node,
                port,
                Some(next_value),
                "-",
                false,
                theme,
            ))
        })
        .when_some(toggle, |this, next_value| {
            this.child(render_input_value_action_button(
                node,
                port,
                Some(next_value),
                "toggle",
                false,
                theme,
            ))
        })
        .when_some(increment, |this, next_value| {
            this.child(render_input_value_action_button(
                node,
                port,
                Some(next_value),
                "+",
                false,
                theme,
            ))
        })
        .when_some(value.default_value.as_ref(), |this, default_value| {
            let disabled = value.current_value.as_ref() == Some(default_value);
            this.child(render_input_value_action_button(
                node,
                port,
                Some(default_value.clone()),
                "default",
                disabled,
                theme,
            ))
        })
        .when(value.current_value.is_some(), |this| {
            this.child(render_input_value_action_button(
                node, port, None, "clear", false, theme,
            ))
        })
        .into_any_element()
}

fn acquire_graph_port_value_input(
    request: GraphPortValueInputRequest,
    window: &mut Window,
    cx: &mut Context<'_, VisualGraphPanel>,
) -> gpui::AnyElement {
    let GraphPortValueInputRequest {
        node_id,
        port_id,
        envelope,
        family,
        edit_text,
    } = request;

    let key = gpui::SharedString::from(format!(
        "graph-node-{}-input-{}-reflected-value",
        graph_element_key(&node_id),
        port_id
    ));
    let state = window.use_keyed_state(key, cx, {
        let edit_text = edit_text.clone();
        move |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(edit_text.clone()));
            let subscription = cx.subscribe_in(&input, window, {
                move |_: &mut GraphTextInputState,
                      input: &Entity<InputState>,
                      event: &InputEvent,
                      window: &mut Window,
                      cx| {
                    if !matches!(event, InputEvent::PressEnter { .. }) {
                        return;
                    }
                    let source = input.read(cx).value().to_string();
                    let Some(value) = graph_reflected_value_from_edit(&envelope, &family, &source)
                    else {
                        return;
                    };
                    window.dispatch_action(
                        Box::new(crate::actions::SetReflectedGraphPortValue {
                            node_id: node_id.clone(),
                            port_id,
                            value: Some(value),
                        }),
                        cx,
                    );
                }
            });
            GraphTextInputState {
                input,
                _subscription: subscription,
            }
        }
    });

    let input = state.read(cx).input.clone();
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    if !focused && input.read(cx).value() != edit_text {
        input.update(cx, |input, cx| {
            input.set_value(edit_text, window, cx);
        });
    }
    let theme = cx.theme();
    div()
        .w(px(104.0))
        .h(px(24.0))
        .px_1()
        .border_1()
        .border_color(theme.border)
        .bg(theme.input_background())
        .flex()
        .items_center()
        .child(
            Input::new(&input)
                .small()
                .appearance(false)
                .bordered(false)
                .focus_bordered(false),
        )
        .into_any_element()
}

fn graph_reflected_value_edit_state(
    envelope: &ReflectedValueEnvelope,
) -> Option<(WidgetFamily, String)> {
    if envelope.encoding != ReflectedValueEncoding::TypedRon {
        return None;
    }
    let reflected = decode_standalone_reflected_value(envelope).ok()?;
    let family = standalone_reflected_widget_family(&reflected);
    let edit_text = match &reflected {
        ReflectedValue::Scalar(ReflectedScalar::String(value)) => value.clone(),
        _ => std::str::from_utf8(&envelope.payload).ok()?.to_string(),
    };
    Some((family, edit_text))
}

fn graph_reflected_value_from_edit(
    envelope: &ReflectedValueEnvelope,
    family: &WidgetFamily,
    source: &str,
) -> Option<ReflectedValueEnvelope> {
    if envelope.encoding != ReflectedValueEncoding::TypedRon {
        return None;
    }
    let payload = if *family == WidgetFamily::Text {
        ron::to_string(source).ok()?
    } else {
        source.trim().to_string()
    };
    ron::value::RawValue::from_ron(&payload).ok()?;
    Some(ReflectedValueEnvelope::typed_ron(
        envelope.type_path.clone(),
        payload,
    ))
}

fn render_input_value_action_button(
    node: &GraphNodeProjectionData,
    port: &GraphPortProjectionData,
    value: Option<ReflectedValueEnvelope>,
    label: &'static str,
    disabled: bool,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let node_id = node.node_id.clone();
    let port_id = port.port_id;
    div()
        .id(gpui::SharedString::from(format!(
            "graph-node-{}-input-{}-{label}",
            graph_element_key(&node.node_id),
            port.port_id
        )))
        .px_1()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .text_xs()
        .whitespace_nowrap()
        .text_color(if disabled {
            theme.muted_foreground
        } else {
            theme.foreground
        })
        .child(label)
        .when(!disabled, |this| {
            this.hover(|this| this.bg(theme.muted))
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(
                        Box::new(crate::actions::SetReflectedGraphPortValue {
                            node_id: node_id.clone(),
                            port_id,
                            value: value.clone(),
                        }),
                        cx,
                    );
                })
        })
}

fn graph_input_value_active(
    value: &GraphInputValueProjectionData,
) -> Option<&ReflectedValueEnvelope> {
    value
        .current_value
        .as_ref()
        .or(value.default_value.as_ref())
}

fn graph_input_value_toggle_edit(
    value: &GraphInputValueProjectionData,
) -> Option<ReflectedValueEnvelope> {
    let envelope = graph_input_value_active(value)?;
    let reflected = decode_standalone_reflected_value(envelope).ok()?;
    if standalone_reflected_widget_family(&reflected) != WidgetFamily::Bool {
        return None;
    }
    let ReflectedValue::Scalar(ReflectedScalar::Bool(current)) = reflected else {
        return None;
    };
    graph_typed_ron_edit(envelope, (!current).to_string())
}

fn graph_input_value_step_edit(
    value: &GraphInputValueProjectionData,
    direction: i8,
) -> Option<ReflectedValueEnvelope> {
    let step = direction.signum();
    if step == 0 {
        return None;
    }

    let envelope = graph_input_value_active(value)?;
    let reflected = decode_standalone_reflected_value(envelope).ok()?;
    if standalone_reflected_widget_family(&reflected) != WidgetFamily::Number {
        return None;
    }
    let payload = match reflected {
        ReflectedValue::Scalar(ReflectedScalar::Signed(current)) => {
            step_signed_reflected_value(&envelope.type_path, &current, step)?
        }
        ReflectedValue::Scalar(ReflectedScalar::Unsigned(current)) => {
            step_unsigned_reflected_value(&envelope.type_path, &current, step)?
        }
        ReflectedValue::Scalar(ReflectedScalar::Float(current)) => {
            step_float_reflected_value(&envelope.type_path, &current, step)?
        }
        _ => return None,
    };
    graph_typed_ron_edit(envelope, payload)
}

fn graph_typed_ron_edit(
    envelope: &ReflectedValueEnvelope,
    payload: String,
) -> Option<ReflectedValueEnvelope> {
    (envelope.encoding == ReflectedValueEncoding::TypedRon)
        .then(|| ReflectedValueEnvelope::typed_ron(envelope.type_path.clone(), payload))
}

fn step_signed_reflected_value(type_path: &str, value: &str, step: i8) -> Option<String> {
    let current = value.parse::<i128>().ok()?;
    let (minimum, maximum) = match reflected_short_type_path(type_path) {
        "i8" => (i128::from(i8::MIN), i128::from(i8::MAX)),
        "i16" => (i128::from(i16::MIN), i128::from(i16::MAX)),
        "i32" => (i128::from(i32::MIN), i128::from(i32::MAX)),
        "i64" => (i128::from(i64::MIN), i128::from(i64::MAX)),
        "i128" => (i128::MIN, i128::MAX),
        "isize" => (isize::MIN as i128, isize::MAX as i128),
        _ => return None,
    };
    let next = if step.is_negative() {
        current.saturating_sub(1).max(minimum)
    } else {
        current.saturating_add(1).min(maximum)
    };
    (next != current).then(|| next.to_string())
}

fn step_unsigned_reflected_value(type_path: &str, value: &str, step: i8) -> Option<String> {
    let current = value.parse::<u128>().ok()?;
    let maximum = match reflected_short_type_path(type_path) {
        "u8" => u128::from(u8::MAX),
        "u16" => u128::from(u16::MAX),
        "u32" => u128::from(u32::MAX),
        "u64" => u128::from(u64::MAX),
        "u128" => u128::MAX,
        "usize" => usize::MAX as u128,
        _ => return None,
    };
    let next = if step.is_negative() {
        current.saturating_sub(1)
    } else {
        current.saturating_add(1).min(maximum)
    };
    (next != current).then(|| next.to_string())
}

fn step_float_reflected_value(type_path: &str, value: &str, step: i8) -> Option<String> {
    let current = value.parse::<f64>().ok()?;
    if !current.is_finite() {
        return None;
    }
    let next = current + f64::from(step);
    let finite_for_type = match reflected_short_type_path(type_path) {
        // The narrowing is the question being asked: an f64 outside f32's
        // range saturates to an infinity, which is what `is_finite` rejects.
        // Rust has no checked f64 -> f32 conversion to ask it with instead.
        #[allow(clippy::cast_possible_truncation)]
        "f32" => (next as f32).is_finite(),
        "f64" => next.is_finite(),
        _ => false,
    };
    finite_for_type.then(|| {
        let value = next.to_string();
        if value
            .chars()
            .any(|character| matches!(character, '.' | 'e' | 'E'))
        {
            value
        } else {
            format!("{value}.0")
        }
    })
}

fn reflected_short_type_path(type_path: &str) -> &str {
    type_path.rsplit("::").next().unwrap_or(type_path)
}

fn graph_reflected_value_label(envelope: &ReflectedValueEnvelope) -> String {
    decode_standalone_reflected_value(envelope).map_or_else(
        |_| format!("<invalid {}>", envelope.type_path),
        |value| match value {
            ReflectedValue::Scalar(ReflectedScalar::Bool(value)) => value.to_string(),
            // A numeric scalar and an opaque RON blob both already hold the
            // exact text the producer sent, so the label shows it verbatim.
            ReflectedValue::Scalar(
                ReflectedScalar::Signed(value)
                | ReflectedScalar::Unsigned(value)
                | ReflectedScalar::Float(value),
            )
            | ReflectedValue::OpaqueRon(value) => value,
            ReflectedValue::Scalar(ReflectedScalar::String(value)) => format!("\"{value}\""),
            ReflectedValue::Struct(fields) => format!("struct{{{}}}", fields.len()),
            ReflectedValue::Tuple(values) => format!("tuple({})", values.len()),
            ReflectedValue::List(values) => format!("list[{}]", values.len()),
            ReflectedValue::Map(entries) => format!("map{{{}}}", entries.len()),
            ReflectedValue::Enum { variant, fields } => {
                if fields.is_empty() {
                    variant
                } else {
                    format!("{variant}(...)")
                }
            }
            ReflectedValue::Optional(Some(_)) => "Some(...)".to_string(),
            ReflectedValue::Optional(None) => "None".to_string(),
            ReflectedValue::Unit => "()".to_string(),
            ReflectedValue::Encoded(value) => format!("<{:?}>", value.encoding),
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphPortClickTransition {
    pending_output_port: Option<PendingGraphPortConnection>,
    connection_request: Option<GraphPortConnectionRequest>,
}

struct GraphTextInputState {
    input: Entity<InputState>,
    _subscription: Subscription,
}

fn graph_port_click_transition(
    pending_output_port: Option<PendingGraphPortConnection>,
    click: &GraphPortClickData,
) -> GraphPortClickTransition {
    match click.direction {
        GraphPortDirectionData::Output => {
            let clicked = PendingGraphPortConnection {
                node_id: click.node_id.clone(),
                port_id: click.port_id,
            };
            let pending_output_port = if pending_output_port.as_ref() == Some(&clicked) {
                None
            } else {
                Some(clicked)
            };
            GraphPortClickTransition {
                pending_output_port,
                connection_request: None,
            }
        }
        GraphPortDirectionData::Input => {
            let connection_request =
                pending_output_port.map(|pending| GraphPortConnectionRequest {
                    from_node_id: pending.node_id,
                    from_port_id: pending.port_id,
                    to_node_id: click.node_id.clone(),
                    to_port_id: click.port_id,
                });
            GraphPortClickTransition {
                pending_output_port: None,
                connection_request,
            }
        }
    }
}

fn render_port_strip(
    node: &GraphNodeProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let input_count = node
        .ports
        .iter()
        .filter(|port| port.direction == GraphPortDirectionData::Input)
        .count();
    let output_count = node.ports.len().saturating_sub(input_count);
    h_flex()
        .items_center()
        .gap_1()
        .child(render_port_count("in", input_count, theme))
        .child(render_port_count("out", output_count, theme))
}

fn render_port_count(
    label: &'static str,
    count: usize,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .px_1()
        .py_0p5()
        .rounded_sm()
        .bg(theme.muted)
        .text_xs()
        .text_color(theme.foreground)
        .child(format!("{label}:{count}"))
}

fn render_port_handles(
    node: &GraphNodeProjectionData,
    transform: GraphCanvasTransform,
    active_node_drag: Option<&GraphNodeDragState>,
    pending_output_port: Option<&PendingGraphPortConnection>,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, VisualGraphPanel>,
) -> Vec<gpui::AnyElement> {
    node.ports
        .iter()
        .map(|port| {
            let position = port_canvas_position(node, port, transform, active_node_drag);
            let size = transform.length(8.0);
            let click = GraphPortClickData {
                node_id: node.node_id.clone(),
                port_id: port.port_id,
                direction: port.direction,
            };
            let pending = pending_output_port.is_some_and(|pending| {
                pending.node_id == node.node_id && pending.port_id == port.port_id
            });
            let color = match port.direction {
                GraphPortDirectionData::Input => theme.foreground,
                GraphPortDirectionData::Output => theme.accent,
            };
            div()
                .id(gpui::SharedString::from(format!(
                    "graph-port-{}-{}",
                    graph_element_key(&node.node_id),
                    port.port_id
                )))
                .absolute()
                .left(px(size.mul_add(-0.5, position.x)))
                .top(px(size.mul_add(-0.5, position.y)))
                .w(px(size))
                .h(px(size))
                .rounded_sm()
                .border_1()
                .border_color(if pending {
                    theme.warning
                } else {
                    theme.background
                })
                .bg(color)
                .hover(|this| this.bg(theme.warning))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.handle_graph_port_click(&click, window, cx);
                }))
                .into_any_element()
        })
        .collect()
}

fn port_canvas_position(
    node: &GraphNodeProjectionData,
    port: &GraphPortProjectionData,
    transform: GraphCanvasTransform,
    active_node_drag: Option<&GraphNodeDragState>,
) -> GraphPointProjectionData {
    let node_position = graph_node_document_position(node, active_node_drag);
    transform.point(GraphPointProjectionData::new(
        node_position.x + port.x,
        node_position.y + port.y,
    ))
}

/// A connection is "routed" once it carries any solver- or user-authored
/// waypoints; routed connections keep the orthogonal polyline divs so their
/// anchors stay meaningful and hit-testable. Connections with only port
/// endpoints render as bezier wires.
fn connection_is_routed(connection: &GraphConnectionProjectionData) -> bool {
    connection
        .route_anchors
        .iter()
        .any(|anchor| anchor.kind != GraphRouteAnchorKindData::PortEndpoint)
}

/// Canvas-local geometry for one cubic bezier wire between two port anchors.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GraphBezierWireGeometry {
    start: GraphPointProjectionData,
    control_start: GraphPointProjectionData,
    control_end: GraphPointProjectionData,
    end: GraphPointProjectionData,
    thickness: f32,
}

/// Horizontal tangent length for a wire's control points: half the horizontal
/// span, but never shorter than a zoom-scaled minimum so short wires still
/// bow outward from their ports (React-Flow style).
fn bezier_tangent_offset(start_x: f32, end_x: f32, zoom: f32) -> f32 {
    ((end_x - start_x).abs() * 0.5).max(GRAPH_BEZIER_MIN_TANGENT * sanitize_graph_zoom(zoom))
}

fn bezier_wire_geometry(
    connection: &GraphConnectionProjectionData,
    transform: GraphCanvasTransform,
) -> Option<GraphBezierWireGeometry> {
    if connection.points.len() < 2 {
        return None;
    }
    let start = transform.point(*connection.points.first()?);
    let end = transform.point(*connection.points.last()?);
    if ![start.x, start.y, end.x, end.y]
        .iter()
        .all(|value| value.is_finite())
    {
        return None;
    }
    let offset = bezier_tangent_offset(start.x, end.x, transform.zoom);
    Some(GraphBezierWireGeometry {
        start,
        control_start: GraphPointProjectionData::new(start.x + offset, start.y),
        control_end: GraphPointProjectionData::new(end.x - offset, end.y),
        end,
        thickness: transform.length(GRAPH_CONNECTION_THICKNESS),
    })
}

/// Paints all unrouted connections as cubic bezier wires through one
/// `canvas()` layer. Geometry and colors are resolved at render time; the
/// paint closure only tessellates and strokes.
fn render_graph_bezier_wires(
    document: &GraphDocumentProjectionData,
    transform: GraphCanvasTransform,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    let wires = document
        .connections
        .iter()
        .filter(|connection| !connection_is_routed(connection))
        .filter_map(|connection| {
            bezier_wire_geometry(connection, transform).map(|geometry| {
                let color = if connection.selected {
                    theme.accent
                } else {
                    theme.muted_foreground
                };
                (geometry, color)
            })
        })
        .collect::<Vec<_>>();
    if wires.is_empty() {
        return div().absolute().into_any_element();
    }

    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let origin = bounds.origin;
            for (geometry, color) in &wires {
                let mut builder = PathBuilder::stroke(px(geometry.thickness));
                builder.move_to(origin + point(px(geometry.start.x), px(geometry.start.y)));
                builder.cubic_bezier_to(
                    origin + point(px(geometry.end.x), px(geometry.end.y)),
                    origin + point(px(geometry.control_start.x), px(geometry.control_start.y)),
                    origin + point(px(geometry.control_end.x), px(geometry.control_end.y)),
                );
                if let Ok(path) = builder.build() {
                    window.paint_path(path, *color);
                }
            }
        },
    )
    .absolute()
    .size_full()
    .into_any_element()
}

fn render_connection_segments(
    connection: &GraphConnectionProjectionData,
    transform: GraphCanvasTransform,
    theme: &gpui_component::theme::Theme,
) -> Vec<gpui::AnyElement> {
    let color = if connection.selected {
        theme.accent
    } else {
        theme.muted_foreground
    };
    connection_segment_rects(connection, transform)
        .into_iter()
        .map(|rect| {
            div()
                .absolute()
                .left(px(rect.x))
                .top(px(rect.y))
                .w(px(rect.width))
                .h(px(rect.height))
                .bg(color)
                .rounded_sm()
                .into_any_element()
        })
        .collect()
}

fn render_connection_route_anchors(
    connection: &GraphConnectionProjectionData,
    transform: GraphCanvasTransform,
    active_route_anchor_drag: Option<&GraphRouteAnchorDragState>,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, VisualGraphPanel>,
) -> Vec<gpui::AnyElement> {
    connection
        .route_anchors
        .iter()
        .map(|anchor| {
            let color = route_anchor_color(anchor.kind, theme);
            let size = transform.length(route_anchor_size(anchor.kind));
            let route_anchor =
                route_anchor_document_position(connection, anchor, active_route_anchor_drag);
            let position = transform.point(route_anchor);
            let dragging = route_anchor_is_dragging(connection, anchor, active_route_anchor_drag);
            let drag_start = GraphRouteAnchorDragStartData {
                connection_id: connection.connection_id.clone(),
                anchor_id: anchor.anchor_id.clone(),
                anchor_x: anchor.x,
                anchor_y: anchor.y,
                draggable: anchor.kind == GraphRouteAnchorKindData::UserWaypoint,
            };
            div()
                .id(graph_route_anchor_element_id(
                    &connection.connection_id,
                    &anchor.anchor_id,
                ))
                .absolute()
                .left(px(size.mul_add(-0.5, position.x)))
                .top(px(size.mul_add(-0.5, position.y)))
                .w(px(size))
                .h(px(size))
                .rounded_sm()
                .border_1()
                .border_color(if dragging {
                    theme.warning
                } else {
                    theme.background
                })
                .bg(color)
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                        this.handle_graph_route_anchor_mouse_down(&drag_start, event, cx);
                    }),
                )
                .into_any_element()
        })
        .collect()
}

/// Element id for one route-anchor handle.
///
/// Every connection re-emits every anchor on every repaint, so this id is on the
/// hot path and must not allocate: [`gpui::ElementId::NamedInteger`] pairs a
/// `&'static` name with a digest of the owning connection and anchor ids. The
/// digest is also a strictly finer discriminator than the sanitized string id it
/// replaces — [`graph_element_key`] folds every non-alphanumeric byte to `-`, so
/// `a/b` and `a.b` already shared one id.
fn graph_route_anchor_element_id(connection_id: &str, anchor_id: &str) -> gpui::ElementId {
    gpui::ElementId::NamedInteger(
        gpui::SharedString::new_static("graph-route-anchor"),
        graph_element_digest(&[connection_id, anchor_id]),
    )
}

/// FNV-1a over `parts`, separated by `0xff`.
///
/// `0xff` never appears in UTF-8, so no concatenation of parts can be confused
/// with a different split of the same bytes.
fn graph_element_digest(parts: &[&str]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    const SEPARATOR: u64 = 0xff;

    let mut digest = OFFSET_BASIS;
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            digest = (digest ^ SEPARATOR).wrapping_mul(PRIME);
        }
        for byte in part.as_bytes() {
            digest = (digest ^ u64::from(*byte)).wrapping_mul(PRIME);
        }
    }
    digest
}

fn route_anchor_is_dragging(
    connection: &GraphConnectionProjectionData,
    anchor: &GraphRouteAnchorProjectionData,
    active_route_anchor_drag: Option<&GraphRouteAnchorDragState>,
) -> bool {
    active_route_anchor_drag.is_some_and(|drag| {
        drag.connection_id == connection.connection_id && drag.anchor_id == anchor.anchor_id
    })
}

fn route_anchor_document_position(
    connection: &GraphConnectionProjectionData,
    anchor: &GraphRouteAnchorProjectionData,
    active_route_anchor_drag: Option<&GraphRouteAnchorDragState>,
) -> GraphPointProjectionData {
    active_route_anchor_drag
        .filter(|drag| {
            drag.connection_id == connection.connection_id && drag.anchor_id == anchor.anchor_id
        })
        .map_or_else(
            || GraphPointProjectionData::new(anchor.x, anchor.y),
            |drag| GraphPointProjectionData::new(drag.preview_x, drag.preview_y),
        )
}

fn route_anchor_color(
    kind: GraphRouteAnchorKindData,
    theme: &gpui_component::theme::Theme,
) -> gpui::Hsla {
    match kind {
        GraphRouteAnchorKindData::PortEndpoint => theme.foreground,
        GraphRouteAnchorKindData::UserWaypoint => theme.accent,
        GraphRouteAnchorKindData::SolverWaypoint => theme.muted_foreground,
        GraphRouteAnchorKindData::Junction => theme.warning,
    }
}

const fn route_anchor_size(kind: GraphRouteAnchorKindData) -> f32 {
    match kind {
        GraphRouteAnchorKindData::UserWaypoint => 10.0,
        GraphRouteAnchorKindData::Junction => 9.0,
        GraphRouteAnchorKindData::PortEndpoint | GraphRouteAnchorKindData::SolverWaypoint => 7.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GraphSegmentRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn connection_segment_rects(
    connection: &GraphConnectionProjectionData,
    transform: GraphCanvasTransform,
) -> Vec<GraphSegmentRect> {
    const EPSILON: f32 = 0.01;
    let thickness = transform.length(2.0);

    connection
        .points
        .windows(2)
        .filter_map(|points| {
            let start = transform.point(points[0]);
            let end = transform.point(points[1]);
            let dx = (start.x - end.x).abs();
            let dy = (start.y - end.y).abs();
            if dx <= EPSILON && dy <= EPSILON {
                return None;
            }
            if dy <= EPSILON {
                return Some(GraphSegmentRect {
                    x: start.x.min(end.x),
                    y: start.y - thickness * 0.5,
                    width: dx.max(thickness),
                    height: thickness,
                });
            }
            if dx <= EPSILON {
                return Some(GraphSegmentRect {
                    x: start.x - thickness * 0.5,
                    y: start.y.min(end.y),
                    width: thickness,
                    height: dy.max(thickness),
                });
            }
            None
        })
        .collect()
}

fn render_graph_diagnostics(
    document: &GraphDocumentProjectionData,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    if document.diagnostics.is_empty() {
        return div().into_any_element();
    }

    div()
        .absolute()
        .right_2()
        .bottom_2()
        .max_w_96()
        .p_2()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .children(document.diagnostics.iter().map(|diagnostic| {
            div()
                .text_xs()
                .text_color(theme.foreground)
                .child(diagnostic.clone())
        }))
        .into_any_element()
}

fn render_empty_graph_state(theme: &gpui_component::theme::Theme) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child("No graph document")
}

fn render_graph_error(error: &str, theme: &gpui_component::theme::Theme) -> impl IntoElement {
    div()
        .m_2()
        .p_2()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.muted)
        .text_sm()
        .text_color(theme.foreground)
        .child(format!("Graph document error: {error}"))
}

fn render_layout_job_badge(
    job: &GraphLayoutJobData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .rounded_sm()
        .bg(theme.muted)
        .text_xs()
        .text_color(match job.phase {
            GraphLayoutJobPhaseData::Failed => theme.warning,
            _ => theme.foreground,
        })
        .child(format!(
            "{}: {}",
            graph_layout_job_phase_label(job.phase),
            job.label
        ))
}

fn render_graph_build_status_badge(
    status: &GraphBuildStatusProjectionData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let label = status.latest_job.as_ref().map_or_else(
        || format!("build: source {}", status.source_status.label()),
        |job| format!("build: {} {}", job.job_key, job.status.label()),
    );

    div()
        .px_2()
        .py_1()
        .rounded_sm()
        .bg(theme.muted)
        .text_xs()
        .text_color(match status.latest_job.as_ref().map(|job| job.status) {
            Some(GraphBuildJobStatusData::Failed | GraphBuildJobStatusData::Abandoned) => {
                theme.warning
            }
            _ => theme.foreground,
        })
        .child(label)
}

fn render_pending_connection_badge(
    pending: &PendingGraphPortConnection,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .rounded_sm()
        .bg(theme.muted)
        .text_xs()
        .text_color(theme.warning)
        .child(format!("connect: {}:{}", pending.node_id, pending.port_id))
}

fn render_graph_viewport_badge(
    viewport: GraphViewportState,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .rounded_sm()
        .bg(theme.muted)
        .text_xs()
        .text_color(theme.foreground)
        .child(format!(
            "{}% x:{} y:{}",
            (viewport.zoom * 100.0).round(),
            graph_scalar_label(viewport.pan_x),
            graph_scalar_label(viewport.pan_y)
        ))
}

fn graph_viewport_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    theme: &gpui_component::theme::Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .hover(|this| this.bg(theme.muted))
        .cursor_pointer()
        .text_xs()
        .text_color(theme.foreground)
        .child(label)
        .on_click(on_click)
}

fn graph_action_button<A>(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    action: A,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement
where
    A: gpui::Action + Clone + 'static,
{
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .hover(|this| this.bg(theme.muted))
        .cursor_pointer()
        .text_xs()
        .text_color(theme.foreground)
        .child(label)
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GraphCanvasBounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

fn graph_canvas_bounds(document: &GraphDocumentProjectionData) -> GraphCanvasBounds {
    let mut bounds = GraphCanvasBounds {
        min_x: 0.0,
        min_y: 0.0,
        max_x: 0.0,
        max_y: 0.0,
    };
    let mut initialized = false;

    for node in &document.nodes {
        expand_bounds(&mut bounds, &mut initialized, node.x, node.y);
        expand_bounds(
            &mut bounds,
            &mut initialized,
            node.x + node.width,
            node.y + node.height,
        );
    }
    for connection in &document.connections {
        for point in &connection.points {
            expand_bounds(&mut bounds, &mut initialized, point.x, point.y);
        }
        for anchor in &connection.route_anchors {
            expand_bounds(&mut bounds, &mut initialized, anchor.x, anchor.y);
        }
    }
    for comment in &document.comments {
        expand_bounds(&mut bounds, &mut initialized, comment.x, comment.y);
        expand_bounds(
            &mut bounds,
            &mut initialized,
            comment.x + comment.width,
            comment.y + comment.height,
        );
    }

    bounds
}

const fn expand_bounds(bounds: &mut GraphCanvasBounds, initialized: &mut bool, x: f32, y: f32) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    if !*initialized {
        bounds.min_x = x;
        bounds.min_y = y;
        bounds.max_x = x;
        bounds.max_y = y;
        *initialized = true;
        return;
    }
    bounds.min_x = bounds.min_x.min(x);
    bounds.min_y = bounds.min_y.min(y);
    bounds.max_x = bounds.max_x.max(x);
    bounds.max_y = bounds.max_y.max(y);
}

fn graph_document_summary(document: &GraphDocumentProjectionData) -> String {
    let dirty = if document.unsaved_changes { " *" } else { "" };
    format!(
        "{} rev {}{} | {} nodes | {} edges | {} notes",
        document.document_id,
        document.revision,
        dirty,
        document.nodes.len(),
        document.connections.len(),
        document.comments.len()
    )
}

fn graph_document_revision_label(document: &GraphDocumentListItemProjectionData) -> String {
    match document.saved_revision {
        Some(saved) if saved == document.revision => format!("rev {} saved", document.revision),
        Some(saved) => format!("rev {} saved {saved}", document.revision),
        None => format!("rev {} unsaved", document.revision),
    }
}

fn graph_element_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

const fn graph_layout_job_phase_label(phase: GraphLayoutJobPhaseData) -> &'static str {
    match phase {
        GraphLayoutJobPhaseData::Idle => "idle",
        GraphLayoutJobPhaseData::Queued => "queued",
        GraphLayoutJobPhaseData::Running => "running",
        GraphLayoutJobPhaseData::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_summary_includes_dirty_marker_and_counts() {
        let document = sample_document(true);

        assert_eq!(
            graph_document_summary(&document),
            "graphs/test.azgraph.ron rev 7 * | 2 nodes | 1 edges | 1 notes"
        );
    }

    #[test]
    fn graph_canvas_bounds_include_nodes_and_connection_points() {
        let document = sample_document(false);

        let bounds = graph_canvas_bounds(&document);

        assert_eq!(
            bounds,
            GraphCanvasBounds {
                min_x: -40.0,
                min_y: -10.0,
                max_x: 420.0,
                max_y: 180.0,
            }
        );
    }

    #[test]
    fn next_comment_bounds_fans_out_from_existing_comments() {
        let document = sample_document(false);

        assert_eq!(
            next_comment_bounds(&document),
            (
                304.0,
                32.0,
                GRAPH_DEFAULT_COMMENT_WIDTH,
                GRAPH_DEFAULT_COMMENT_HEIGHT
            )
        );
    }

    #[test]
    fn graph_layout_job_phase_labels_are_stable() {
        assert_eq!(
            graph_layout_job_phase_label(GraphLayoutJobPhaseData::Idle),
            "idle"
        );
        assert_eq!(
            graph_layout_job_phase_label(GraphLayoutJobPhaseData::Queued),
            "queued"
        );
        assert_eq!(
            graph_layout_job_phase_label(GraphLayoutJobPhaseData::Running),
            "running"
        );
        assert_eq!(
            graph_layout_job_phase_label(GraphLayoutJobPhaseData::Failed),
            "failed"
        );
    }

    #[test]
    fn graph_projection_can_carry_creation_catalog() {
        let projection = EditorGraphDocumentProjection::empty().with_creation_catalog(
            GraphCreationCatalogProjectionData::new(vec![GraphTypeCreationProjectionData {
                graph_type: "az.editor.tests.logic".to_string(),
                label: "Logic".to_string(),
                category: "Tests/Logic".to_string(),
                default_path_prefix: "graphs".to_string(),
                default_extension: "azgraph.ron".to_string(),
                compiler_backend: Some(GraphCompilerBackendProjectionData {
                    id: "az.editor.tests.logic.compiler".to_string(),
                    kind: GraphCompilerBackendKindProjectionData::GeneratedRust {
                        package: "game".to_string(),
                        entry_symbol: "run_logic_graph".to_string(),
                        abi: GraphGeneratedRustAbiProjectionData::TypedDataflow,
                    },
                    capability_markers: vec!["zero-cost".to_string()],
                }),
                runtime_product_asset_type: Some("azoth.graph.aot-manifest".to_string()),
                runtime_product_kind: Some("azoth.graph.aot-manifest".to_string()),
                runtime_product_streamable: Some(true),
                runtime_product_diffable_chunks: Some(true),
                runtime_execution_strategy: Some(
                    GraphRuntimeExecutionStrategyProjectionData::AotCompiledCode {
                        language: "rust".to_string(),
                        package: "game".to_string(),
                        entry_symbol: "run_logic_graph".to_string(),
                        context_type: "game::LogicContext".to_string(),
                    },
                ),
                runtime_compiled: true,
                editor_interpreted: false,
            }]),
        );

        assert_eq!(projection.creation_catalog.graph_types.len(), 1);
        assert_eq!(
            projection.creation_catalog.graph_types[0].graph_type,
            "az.editor.tests.logic"
        );
        assert_eq!(
            graph_type_execution_label(&projection.creation_catalog.graph_types[0]),
            "rust typed dataflow -> aot rust run_logic_graph"
        );
        let graph_type = &projection.creation_catalog.graph_types[0];
        assert_eq!(
            graph_type_build_mode_label(graph_type),
            "zero-cost Rust AOT"
        );
        assert_eq!(
            graph_generated_rust_compiler_detail(graph_type).unwrap(),
            GraphGeneratedRustCompilerDetail {
                package: "game".to_string(),
                entry_symbol: "run_logic_graph".to_string(),
                abi: "typed dataflow".to_string(),
            }
        );
        assert_eq!(
            graph_aot_runtime_detail(graph_type).unwrap(),
            GraphAotRuntimeDetail {
                package: "game".to_string(),
                entry_symbol: "run_logic_graph".to_string(),
                context_type: "game::LogicContext".to_string(),
            }
        );
        assert_eq!(
            graph_runtime_product_traits_label(graph_type).as_deref(),
            Some("streamable / diffable chunks")
        );
        let mut document = sample_document(false);
        document.graph_type_info = Some(graph_type.clone());
        assert_eq!(
            graph_document_build_mode_label(&document).as_deref(),
            Some("zero-cost Rust AOT")
        );
    }

    #[test]
    fn graph_projection_can_carry_openable_document_list() {
        let projection = EditorGraphDocumentProjection::empty().with_graph_documents(
            GraphDocumentListProjectionData::new(vec![GraphDocumentListItemProjectionData {
                document_id: "graphs/combat.azgraph.ron".to_string(),
                graph_type: "az.editor.tests.logic".to_string(),
                source_path: "graphs/combat.azgraph.ron".to_string(),
                revision: 4,
                saved_revision: Some(3),
                unsaved_changes: true,
                loaded: true,
                current: true,
            }]),
        );

        assert_eq!(projection.graph_documents.documents.len(), 1);
        assert_eq!(
            projection.graph_documents.documents[0].document_id,
            "graphs/combat.azgraph.ron"
        );
        assert_eq!(
            graph_document_revision_label(&projection.graph_documents.documents[0]),
            "rev 4 saved 3"
        );
    }

    #[test]
    fn graph_projection_can_carry_build_status() {
        let projection = EditorGraphDocumentProjection::empty().with_build_status(Some(
            GraphBuildStatusProjectionData {
                document_id: "graphs/combat.azgraph.ron".to_string(),
                source_path: "graphs/combat.azgraph.ron".to_string(),
                asset_guid: "8d8d3389-8f6a-42dc-82b2-2b35f7ff1726".to_string(),
                source_status: GraphBuildSourceStatusData::Added,
                entry_id: 7,
                content_hash: "ab".repeat(32),
                latest_job: Some(GraphBuildJobProjectionData {
                    job_id: 41,
                    attempt_id: Some(42),
                    job_key: "compile-graph-runtime-product".to_string(),
                    platform: "pc".to_string(),
                    ordinal: Some(1),
                    status: GraphBuildJobStatusData::Queued,
                    error_count: 0,
                    warning_count: 0,
                }),
            },
        ));

        let status = projection.build_status.as_ref().unwrap();
        assert_eq!(status.source_status.label(), "added");
        assert_eq!(
            status
                .latest_job
                .as_ref()
                .map(graph_build_job_label)
                .as_deref(),
            Some("compile-graph-runtime-product:pc #1 queued")
        );
    }

    #[test]
    fn graph_projection_can_carry_node_palette() {
        let projection = EditorGraphDocumentProjection::empty().with_node_palette(
            GraphNodePaletteProjectionData::new(vec![GraphNodePaletteItemData {
                node_type: "az.editor.tests.print".to_string(),
                version: 1,
                label: "Print".to_string(),
                category: "Logic/Debug".to_string(),
                description: Some("Writes a debug line".to_string()),
                input_count: 1,
                output_count: 1,
                default_input_count: 1,
                runtime_bound: true,
                runtime_binding: Some(GraphNodeRuntimeBindingProjectionData::RustSymbol {
                    package: "az_editor_tests".to_string(),
                    symbol: "az_editor_tests::debug::print".to_string(),
                    call_abi: GraphRustNodeCallAbiProjectionData::TypedDataflow {
                        parameter_count: 2,
                        input_parameter_count: 1,
                        by_value_parameter_count: 1,
                        mutable_parameter_count: 0,
                        output_count: 1,
                        result: GraphRustCallResultProjectionData::Result,
                    },
                }),
                source_link_count: 1,
                tags: vec!["debug".to_string()],
            }]),
        );

        assert_eq!(projection.node_palette.nodes.len(), 1);
        assert_eq!(
            projection.node_palette.nodes[0].node_type,
            "az.editor.tests.print"
        );
        assert!(projection.node_palette.nodes[0].runtime_bound);
        assert_eq!(
            graph_node_runtime_label(&projection.node_palette.nodes[0]),
            "rust"
        );
        assert_eq!(
            graph_node_runtime_detail(&projection.node_palette.nodes[0]).as_deref(),
            Some("typed dataflow 2p/1i/1o result 1 move print")
        );
    }

    #[test]
    fn graph_port_projection_can_carry_input_values() {
        let value = GraphInputValueProjectionData {
            schema_type: "f32".to_string(),
            current_value: Some(reflected_value("f32", "2.5")),
            default_value: Some(reflected_value("f32", "1.0")),
        };

        let port = GraphPortProjectionData {
            port_id: 1,
            name: "speed".to_string(),
            direction: GraphPortDirectionData::Input,
            side: GraphPortSideData::West,
            value: Some(value.clone()),
            x: 0.0,
            y: 16.0,
        };

        assert_eq!(port.value, Some(value));
    }

    #[test]
    fn graph_source_link_labels_prefer_symbol_and_compact_location() {
        let link = GraphNodeSourceLinkProjectionData {
            package: Some("az-editor-tests".to_string()),
            module_path: Some("az_editor_tests::nodes".to_string()),
            symbol_path: Some("az_editor_tests::nodes::SourceNode::run".to_string()),
            file: Some("crates/editor/tests/src/nodes.rs".to_string()),
            line: Some(42),
            column: Some(9),
            docs_url: None,
        };

        assert_eq!(
            graph_source_link_label(&link),
            "az_editor_tests::nodes::SourceNode::run"
        );
        assert_eq!(
            graph_source_link_detail(&link),
            "az-editor-tests | crates/editor/tests/src/nodes.rs:42:9"
        );
    }

    #[test]
    fn graph_reflected_value_labels_are_compact_for_node_cards() {
        assert_eq!(
            graph_reflected_value_label(&reflected_value("bool", "true")),
            "true"
        );
        assert_eq!(
            graph_reflected_value_label(&reflected_value("f64", "1.25")),
            "1.25"
        );
        assert_eq!(
            graph_reflected_value_label(&reflected_value("alloc::string::String", r#""fast""#)),
            "\"fast\""
        );
        assert_eq!(
            graph_reflected_value_label(&reflected_value("alloc::vec::Vec<f32>", "[]")),
            "[]"
        );
    }

    #[test]
    fn graph_input_value_scalar_controls_emit_typed_values() {
        let boolean = graph_input_value(reflected_value("bool", "true"), None);
        assert_eq!(
            graph_input_value_toggle_edit(&boolean),
            Some(reflected_value("bool", "false"))
        );
        assert_eq!(graph_input_value_step_edit(&boolean, 1), None);

        let signed = graph_input_value(reflected_value("i32", "-2"), None);
        assert_eq!(
            graph_input_value_step_edit(&signed, -1),
            Some(reflected_value("i32", "-3"))
        );
        assert_eq!(
            graph_input_value_step_edit(&signed, 1),
            Some(reflected_value("i32", "-1"))
        );

        let unsigned = graph_input_value(reflected_value("u32", "0"), None);
        assert_eq!(graph_input_value_step_edit(&unsigned, -1), None);
        assert_eq!(
            graph_input_value_step_edit(&unsigned, 1),
            Some(reflected_value("u32", "1"))
        );

        let float = graph_input_value(reflected_value("f32", "2.5"), None);
        assert_eq!(
            graph_input_value_step_edit(&float, -1),
            Some(reflected_value("f32", "1.5"))
        );
        assert_eq!(
            graph_input_value_step_edit(&float, 1),
            Some(reflected_value("f32", "3.5"))
        );
    }

    #[test]
    fn graph_input_value_controls_use_default_when_current_is_unset() {
        let value = GraphInputValueProjectionData {
            schema_type: "bool".to_string(),
            current_value: None,
            default_value: Some(reflected_value("bool", "false")),
        };

        assert_eq!(
            graph_input_value_toggle_edit(&value),
            Some(reflected_value("bool", "true"))
        );
    }

    #[test]
    fn graph_input_value_controls_reject_unsupported_or_non_finite_values() {
        let string = graph_input_value(reflected_value("alloc::string::String", r#""fast""#), None);
        assert_eq!(graph_input_value_toggle_edit(&string), None);
        assert_eq!(graph_input_value_step_edit(&string, 1), None);

        let non_finite = graph_input_value(reflected_value("f64", "inf"), None);
        assert_eq!(graph_input_value_step_edit(&non_finite, 1), None);
    }

    #[test]
    fn graph_reflected_value_inputs_preserve_type_paths_and_typed_ron() {
        let string = reflected_value("alloc::string::String", r#""fast""#);
        assert_eq!(
            graph_reflected_value_edit_state(&string),
            Some((WidgetFamily::Text, "fast".to_string()))
        );
        assert_eq!(
            graph_reflected_value_from_edit(&string, &WidgetFamily::Text, "very fast"),
            Some(reflected_value("alloc::string::String", r#""very fast""#))
        );

        let list = reflected_value("alloc::vec::Vec<i32>", "[1, 2]");
        assert_eq!(
            graph_reflected_value_from_edit(&list, &WidgetFamily::Opaque, "[3, 4]"),
            Some(reflected_value("alloc::vec::Vec<i32>", "[3, 4]"))
        );
        assert_eq!(
            graph_reflected_value_from_edit(&list, &WidgetFamily::Opaque, "["),
            None
        );
    }

    #[test]
    fn selected_graph_node_resolves_local_panel_selection() {
        let document = sample_document(false);

        let selected = selected_graph_node(Some(&document), Some("node-b")).unwrap();

        assert_eq!(selected.node_id, "node-b");
        assert!(selected_graph_node(Some(&document), Some("missing")).is_none());
        assert!(selected_graph_node(None, Some("node-b")).is_none());
        assert!(selected_graph_node(Some(&document), None).is_none());
    }

    #[test]
    fn selected_graph_comment_resolves_local_panel_selection() {
        let document = sample_document(false);

        let selected = selected_graph_comment(Some(&document), Some("comment-a")).unwrap();

        assert_eq!(selected.text, "Comment");
        assert!(selected_graph_comment(Some(&document), Some("missing")).is_none());
        assert!(selected_graph_comment(None, Some("comment-a")).is_none());
        assert!(selected_graph_comment(Some(&document), None).is_none());
    }

    #[test]
    fn graph_node_is_selected_uses_projection_or_local_selection() {
        let document = sample_document(false);

        assert!(graph_node_is_selected(&document.nodes[0], None));
        assert!(graph_node_is_selected(&document.nodes[1], Some("node-b")));
        assert!(!graph_node_is_selected(&document.nodes[1], Some("node-a")));
    }

    #[test]
    fn graph_node_document_position_prefers_active_drag_preview() {
        let document = sample_document(false);
        let drag = GraphNodeDragState {
            node_id: "node-b".to_string(),
            start_node_x: 300.0,
            start_node_y: 92.0,
            start_mouse_x: 10.0,
            start_mouse_y: 10.0,
            preview_x: 360.0,
            preview_y: 128.0,
            moved: true,
        };

        assert_eq!(
            graph_node_document_position(&document.nodes[0], Some(&drag)),
            GraphPointProjectionData::new(0.0, 0.0)
        );
        assert_eq!(
            graph_node_document_position(&document.nodes[1], Some(&drag)),
            GraphPointProjectionData::new(360.0, 128.0)
        );
        assert!(graph_node_is_dragging(&document.nodes[1], Some(&drag)));
        assert!(!graph_node_is_dragging(&document.nodes[0], Some(&drag)));
    }

    #[test]
    fn graph_node_nudge_controls_are_axis_aligned() {
        assert_eq!(
            GRAPH_NODE_NUDGE_CONTROLS.map(|control| control.label),
            ["X-", "Y-", "Y+", "X+"]
        );
        assert_eq!(
            GRAPH_NODE_NUDGE_CONTROLS.map(|control| (control.dx, control.dy)),
            [(-24.0, 0.0), (0.0, -24.0), (0.0, 24.0), (24.0, 0.0)]
        );

        let document = sample_document(false);
        assert_eq!(
            moved_node_position(&document.nodes[1], GRAPH_NODE_NUDGE_CONTROLS[3].dx, 0.0),
            (324.0, 92.0)
        );
    }

    #[test]
    fn graph_scalar_label_keeps_canvas_numbers_compact() {
        assert_eq!(graph_scalar_label(12.0), "12");
        assert_eq!(graph_scalar_label(12.25), "12.2");
    }

    #[test]
    fn graph_viewport_zoom_is_clamped_and_rounded() {
        let viewport = GraphViewportState::default();

        assert!((viewport.zoom_by(0.26).zoom - 1.3).abs() < 1e-6);
        assert!((viewport.zoom_by(10.0).zoom - GRAPH_VIEWPORT_ZOOM_MAX).abs() < 1e-6);
        assert!((viewport.zoom_by(-10.0).zoom - GRAPH_VIEWPORT_ZOOM_MIN).abs() < 1e-6);
        assert!((clamp_graph_zoom(f32::NAN) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn graph_canvas_transform_applies_bounds_pan_and_zoom() {
        let bounds = graph_canvas_bounds(&sample_document(false));
        let viewport = GraphViewportState {
            pan_x: 8.0,
            pan_y: -4.0,
            zoom: 1.5,
        };

        let transform = GraphCanvasTransform::from_bounds(bounds, viewport);

        assert_eq!(
            transform,
            GraphCanvasTransform {
                offset_x: 80.0,
                offset_y: 38.0,
                zoom: 1.5,
            }
        );
        assert_eq!(
            transform.point(GraphPointProjectionData::new(-40.0, -10.0)),
            GraphPointProjectionData::new(60.0, 42.0)
        );
        assert!((transform.length(8.0) - 12.0).abs() < 1e-6);
    }

    #[test]
    fn graph_node_drag_preview_position_uses_screen_delta_scaled_by_zoom() {
        assert!(!graph_node_drag_exceeds_threshold(10.0, 10.0, 11.0, 11.0));
        assert!(graph_node_drag_exceeds_threshold(10.0, 10.0, 13.0, 10.0));

        assert_eq!(
            graph_node_drag_preview_position(100.0, 40.0, 10.0, 20.0, 50.0, 80.0, 2.0),
            (120.0, 70.0)
        );
        assert_eq!(
            graph_node_drag_preview_position(100.0, 40.0, 10.0, 20.0, 50.0, 80.0, 0.0),
            (140.0, 100.0)
        );
    }

    #[test]
    fn graph_node_drag_state_tracks_preview_and_commit_position() {
        let start = GraphNodeDragStartData {
            node_id: "node-a".to_string(),
            node_x: 100.0,
            node_y: 20.0,
        };
        let mut drag = GraphNodeDragState::start(&start, 10.0, 10.0);

        drag.update(
            34.0,
            58.0,
            GraphViewportState {
                pan_x: 80.0,
                pan_y: -40.0,
                zoom: 2.0,
            },
        );

        assert!(drag.moved);
        assert_eq!(drag.committed_position(), (112.0, 44.0));
    }

    #[test]
    fn graph_route_anchor_drag_state_tracks_preview_and_commit_position() {
        let start = GraphRouteAnchorDragStartData {
            connection_id: "edge".to_string(),
            anchor_id: "anchor-1".to_string(),
            anchor_x: 180.0,
            anchor_y: 140.0,
            draggable: true,
        };
        let mut drag = GraphRouteAnchorDragState::start(&start, 20.0, 10.0);

        drag.update(
            60.0,
            70.0,
            GraphViewportState {
                pan_x: 0.0,
                pan_y: 0.0,
                zoom: 2.0,
            },
        );

        assert!(drag.moved);
        assert_eq!(drag.committed_position(), (200.0, 170.0));
    }

    #[test]
    fn next_node_position_uses_existing_node_count() {
        let document = sample_document(false);

        assert_eq!(next_node_position(Some(&document)), (560.0, 80.0));
        assert_eq!(next_node_position(None), (0.0, 0.0));
    }

    #[test]
    fn moved_node_position_offsets_current_layout() {
        let node = GraphNodeProjectionData {
            node_id: "018f0000-0000-7000-8000-000000000001".to_string(),
            node_type: "azoth.test.float".to_string(),
            label: "Float".to_string(),
            x: 320.0,
            y: 64.0,
            width: 180.0,
            height: 80.0,
            selected: false,
            source_links: Vec::new(),
            ports: Vec::new(),
        };

        assert_eq!(moved_node_position(&node, -24.0, 24.0), (296.0, 88.0));
    }

    #[test]
    fn graph_port_click_transition_primes_output_then_connects_input() {
        let source = GraphPortClickData {
            node_id: "018f0000-0000-7000-8000-000000000001".to_string(),
            port_id: 2,
            direction: GraphPortDirectionData::Output,
        };
        let target = GraphPortClickData {
            node_id: "018f0000-0000-7000-8000-000000000002".to_string(),
            port_id: 1,
            direction: GraphPortDirectionData::Input,
        };

        let primed = graph_port_click_transition(None, &source);
        assert_eq!(
            primed.pending_output_port,
            Some(PendingGraphPortConnection {
                node_id: source.node_id.clone(),
                port_id: source.port_id,
            })
        );
        assert!(primed.connection_request.is_none());

        let connected = graph_port_click_transition(primed.pending_output_port, &target);
        assert_eq!(connected.pending_output_port, None);
        assert_eq!(
            connected.connection_request,
            Some(GraphPortConnectionRequest {
                from_node_id: source.node_id,
                from_port_id: 2,
                to_node_id: target.node_id,
                to_port_id: 1,
            })
        );
    }

    #[test]
    fn graph_port_click_transition_toggles_matching_output() {
        let click = GraphPortClickData {
            node_id: "018f0000-0000-7000-8000-000000000001".to_string(),
            port_id: 2,
            direction: GraphPortDirectionData::Output,
        };
        let pending = Some(PendingGraphPortConnection {
            node_id: click.node_id.clone(),
            port_id: click.port_id,
        });

        let transition = graph_port_click_transition(pending, &click);

        assert_eq!(transition.pending_output_port, None);
        assert!(transition.connection_request.is_none());
    }

    #[test]
    fn graph_element_keys_are_stable_for_graph_type_ids() {
        assert_eq!(
            graph_element_key("az.editor.tests/logic graph"),
            "az-editor-tests-logic-graph"
        );
    }

    #[test]
    fn connection_segment_rects_render_only_orthogonal_spans() {
        let mut document = sample_document(false);
        document.connections[0]
            .points
            .push(GraphPointProjectionData::new(360.0, 180.0));
        let transform = GraphCanvasTransform {
            offset_x: 32.0,
            offset_y: 32.0,
            zoom: 1.0,
        };

        let rects = connection_segment_rects(&document.connections[0], transform);

        assert_eq!(
            rects,
            vec![
                GraphSegmentRect {
                    x: -8.0,
                    y: 21.0,
                    width: 220.0,
                    height: 2.0,
                },
                GraphSegmentRect {
                    x: 211.0,
                    y: 22.0,
                    width: 2.0,
                    height: 150.0,
                },
                GraphSegmentRect {
                    x: 212.0,
                    y: 171.0,
                    width: 120.0,
                    height: 2.0,
                },
            ]
        );
    }

    #[test]
    fn port_canvas_position_uses_node_local_port_coordinates() {
        let mut document = sample_document(false);
        let node = &mut document.nodes[0];
        node.ports.push(GraphPortProjectionData {
            port_id: 1,
            name: "out".to_string(),
            direction: GraphPortDirectionData::Output,
            side: GraphPortSideData::East,
            value: None,
            x: 180.0,
            y: 44.0,
        });
        let transform = GraphCanvasTransform {
            offset_x: 32.0,
            offset_y: 32.0,
            zoom: 1.0,
        };

        assert_eq!(
            port_canvas_position(node, &node.ports[0], transform, None),
            GraphPointProjectionData::new(212.0, 76.0)
        );
    }

    #[test]
    fn route_anchor_document_position_prefers_active_drag_preview() {
        let document = sample_document(false);
        let connection = &document.connections[0];
        let anchor = &connection.route_anchors[0];
        let drag = GraphRouteAnchorDragState {
            connection_id: connection.connection_id.clone(),
            anchor_id: anchor.anchor_id.clone(),
            start_anchor_x: anchor.x,
            start_anchor_y: anchor.y,
            start_mouse_x: 0.0,
            start_mouse_y: 0.0,
            preview_x: 220.0,
            preview_y: 180.0,
            moved: true,
        };

        assert_eq!(
            route_anchor_document_position(connection, anchor, Some(&drag)),
            GraphPointProjectionData::new(220.0, 180.0)
        );
        assert!(route_anchor_is_dragging(connection, anchor, Some(&drag)));
    }

    #[test]
    fn graph_comment_drag_state_tracks_preview_and_commit_bounds() {
        let start = GraphCommentDragStartData {
            comment_id: "comment-a".to_string(),
            comment_x: 20.0,
            comment_y: 30.0,
            comment_width: 200.0,
            comment_height: 80.0,
        };
        let mut drag = GraphCommentDragState::start(&start, 100.0, 120.0);

        drag.update(
            160.0,
            156.0,
            GraphViewportState {
                pan_x: 0.0,
                pan_y: 0.0,
                zoom: 2.0,
            },
        );

        assert!(drag.moved);
        assert_eq!(drag.committed_bounds(), (50.0, 48.0, 200.0, 80.0));
    }

    #[test]
    fn graph_comment_document_position_prefers_active_drag_preview() {
        let document = sample_document(false);
        let comment = &document.comments[0];
        let drag = GraphCommentDragState {
            comment_id: comment.comment_id.clone(),
            start_comment_x: comment.x,
            start_comment_y: comment.y,
            comment_width: comment.width,
            comment_height: comment.height,
            start_mouse_x: 0.0,
            start_mouse_y: 0.0,
            preview_x: 84.0,
            preview_y: 36.0,
            moved: true,
        };

        assert_eq!(
            graph_comment_document_position(comment, Some(&drag)),
            GraphPointProjectionData::new(84.0, 36.0)
        );
        assert!(graph_comment_is_dragging(comment, Some(&drag)));
    }

    #[test]
    fn graph_canvas_transform_inverse_round_trips_points() {
        let transform = GraphCanvasTransform {
            offset_x: 80.0,
            offset_y: 38.0,
            zoom: 1.5,
        };
        let document_point = GraphPointProjectionData::new(-40.0, -10.0);

        let round_trip = transform.inverse_point(transform.point(document_point));

        assert!((round_trip.x - document_point.x).abs() < 1e-3);
        assert!((round_trip.y - document_point.y).abs() < 1e-3);

        let screen_point = GraphPointProjectionData::new(150.0, 90.0);
        let round_trip = transform.point(transform.inverse_point(screen_point));
        assert!((round_trip.x - screen_point.x).abs() < 1e-3);
        assert!((round_trip.y - screen_point.y).abs() < 1e-3);
    }

    #[test]
    fn zoom_viewport_anchored_keeps_anchor_document_point_fixed() {
        let bounds = graph_canvas_bounds(&sample_document(false));
        let viewport = GraphViewportState {
            pan_x: 8.0,
            pan_y: -4.0,
            zoom: 1.0,
        };
        let anchor = GraphPointProjectionData::new(150.0, 90.0);
        let before = GraphCanvasTransform::from_bounds(bounds, viewport);
        let document_under_anchor = before.inverse_point(anchor);

        let zoomed = zoom_viewport_anchored(viewport, anchor.x, anchor.y, 0.5);

        assert!((zoomed.zoom - 1.5).abs() < 1e-6);
        let after = GraphCanvasTransform::from_bounds(bounds, zoomed);
        let screen = after.point(document_under_anchor);
        assert!((screen.x - anchor.x).abs() < 1e-2);
        assert!((screen.y - anchor.y).abs() < 1e-2);
    }

    #[test]
    fn zoom_viewport_anchored_leaves_pan_untouched_at_zoom_bounds() {
        let viewport = GraphViewportState {
            pan_x: 12.0,
            pan_y: -6.0,
            zoom: GRAPH_VIEWPORT_ZOOM_MAX,
        };

        let zoomed = zoom_viewport_anchored(viewport, 100.0, 100.0, GRAPH_VIEWPORT_ZOOM_STEP);

        assert_eq!(zoomed, viewport);
    }

    #[test]
    fn wheel_zoom_delta_maps_lines_and_pixels_to_zoom_steps() {
        assert!((wheel_zoom_delta(ScrollDelta::Lines(point(0.0, 1.0))) - 0.1).abs() < 1e-6);
        assert!(
            (wheel_zoom_delta(ScrollDelta::Pixels(point(px(0.0), px(-40.0)))) + 0.2).abs() < 1e-6
        );
        // Large flings clamp to three steps per event.
        assert!((wheel_zoom_delta(ScrollDelta::Lines(point(0.0, 10.0))) - 0.3).abs() < 1e-6);
        assert!(wheel_zoom_delta(ScrollDelta::Lines(point(0.0, f32::NAN))).abs() < 1e-6);
    }

    #[test]
    fn pan_drag_moves_viewport_by_zoom_scaled_mouse_delta() {
        let viewport = GraphViewportState {
            pan_x: 10.0,
            pan_y: 20.0,
            zoom: 2.0,
        };
        let drag = GraphPanDragState::start(viewport, 100.0, 100.0);

        let panned = drag.panned_viewport(viewport, 130.0, 90.0);

        assert_eq!(
            panned,
            GraphViewportState {
                pan_x: 25.0,
                pan_y: 15.0,
                zoom: 2.0,
            }
        );
    }

    #[test]
    fn connection_is_routed_ignores_port_endpoint_anchors() {
        let mut document = sample_document(false);
        assert!(connection_is_routed(&document.connections[0]));

        document.connections[0].route_anchors[0].kind = GraphRouteAnchorKindData::PortEndpoint;
        assert!(!connection_is_routed(&document.connections[0]));

        document.connections[0].route_anchors.clear();
        assert!(!connection_is_routed(&document.connections[0]));
    }

    #[test]
    fn bezier_wire_geometry_spans_port_anchors_with_horizontal_tangents() {
        let document = sample_document(false);
        let transform = GraphCanvasTransform {
            offset_x: 32.0,
            offset_y: 32.0,
            zoom: 1.0,
        };

        let geometry = bezier_wire_geometry(&document.connections[0], transform).unwrap();

        assert_eq!(geometry.start, GraphPointProjectionData::new(-8.0, 22.0));
        assert_eq!(geometry.end, GraphPointProjectionData::new(332.0, 172.0));
        // Tangent offset is half the 340px horizontal span.
        assert_eq!(
            geometry.control_start,
            GraphPointProjectionData::new(162.0, 22.0)
        );
        assert_eq!(
            geometry.control_end,
            GraphPointProjectionData::new(162.0, 172.0)
        );
        assert!((geometry.thickness - GRAPH_CONNECTION_THICKNESS).abs() < 1e-6);

        // Short wires still bow outward by the zoom-scaled minimum tangent.
        assert!((bezier_tangent_offset(0.0, 10.0, 2.0) - 80.0).abs() < 1e-6);

        let mut degenerate = document.connections[0].clone();
        degenerate.points.truncate(1);
        assert!(bezier_wire_geometry(&degenerate, transform).is_none());
    }

    #[test]
    fn graph_context_menu_add_position_prefers_recorded_document_point() {
        let recorded = Some(GraphPointProjectionData::new(120.0, 64.0));

        assert_eq!(
            graph_context_menu_add_position(recorded, (560.0, 80.0)),
            (120.0, 64.0)
        );
        assert_eq!(
            graph_context_menu_add_position(None, (560.0, 80.0)),
            (560.0, 80.0)
        );
        assert_eq!(
            graph_context_menu_add_position(
                Some(GraphPointProjectionData::new(f32::NAN, 64.0)),
                (560.0, 80.0)
            ),
            (560.0, 80.0)
        );
    }

    #[test]
    fn graph_context_menu_items_capture_palette_identity() {
        let palette = GraphNodePaletteProjectionData::new(vec![GraphNodePaletteItemData {
            node_type: "az.test.Print".to_string(),
            version: 3,
            label: "Print".to_string(),
            category: "Debug".to_string(),
            description: None,
            input_count: 1,
            output_count: 1,
            default_input_count: 0,
            runtime_bound: true,
            runtime_binding: None,
            source_link_count: 0,
            tags: Vec::new(),
        }]);

        let items = graph_context_menu_items(&palette);

        assert_eq!(
            items,
            vec![GraphContextMenuItem {
                node_type: "az.test.Print".to_string(),
                version: 3,
                label: "Print".to_string(),
                category: "Debug".to_string(),
            }]
        );
    }

    #[test]
    fn graph_route_anchor_element_ids_are_stable_and_distinct() {
        let id = graph_route_anchor_element_id("edge", "anchor-1");

        assert_eq!(id, graph_route_anchor_element_id("edge", "anchor-1"));
        assert_ne!(id, graph_route_anchor_element_id("edge", "anchor-2"));
        assert_ne!(id, graph_route_anchor_element_id("other-edge", "anchor-1"));
        assert_ne!(
            graph_route_anchor_element_id("ab", "c"),
            graph_route_anchor_element_id("a", "bc"),
            "the digest separator must keep a shifted split from colliding"
        );
    }

    #[test]
    fn graph_route_anchor_element_ids_separate_ids_the_old_string_key_folded_together() {
        assert_eq!(
            graph_element_key("a/b"),
            graph_element_key("a.b"),
            "control: the sanitized string ids these replaced could not tell these apart"
        );
        assert_ne!(
            graph_route_anchor_element_id("edge", "a/b"),
            graph_route_anchor_element_id("edge", "a.b")
        );
    }

    #[test]
    fn graph_inspector_input_requests_describe_only_editable_port_values() {
        let mut document = sample_document(false);
        document.nodes[0].ports = vec![
            input_port(
                1,
                "label",
                Some(reflected_value("alloc::string::String", r#""hi""#)),
            ),
            input_port(2, "enabled", Some(reflected_value("bool", "true"))),
            input_port(3, "unbound", None),
        ];

        let requests = graph_inspector_input_requests(Some(&document), Some("node-a"), None);

        assert!(requests.comment.is_none());
        let described = requests
            .ports
            .iter()
            .map(|request| (request.port_id, request.edit_text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            described,
            vec![(1, "hi")],
            "bool ports render as a toggle and valueless ports have nothing to edit"
        );
        assert_eq!(requests.ports[0].node_id, "node-a");
        assert_eq!(requests.ports[0].family, WidgetFamily::Text);
    }

    #[test]
    fn graph_inspector_input_requests_prefer_the_selected_node_over_a_selected_comment() {
        let mut document = sample_document(false);
        document.nodes[0].ports = vec![input_port(
            1,
            "label",
            Some(reflected_value("alloc::string::String", r#""hi""#)),
        )];

        let node_selected =
            graph_inspector_input_requests(Some(&document), Some("node-a"), Some("comment-a"));
        assert!(node_selected.comment.is_none());
        assert_eq!(node_selected.ports.len(), 1);

        let comment_selected =
            graph_inspector_input_requests(Some(&document), None, Some("comment-a"));
        let comment = comment_selected
            .comment
            .expect("selecting only a comment describes its text input");
        assert_eq!(comment.comment_id, "comment-a");
        assert_eq!(comment.text, "Comment");
        assert!(comment_selected.ports.is_empty());
    }

    #[test]
    fn graph_inspector_input_requests_are_empty_without_a_selection() {
        let document = sample_document(false);

        let nothing_selected = graph_inspector_input_requests(Some(&document), None, None);
        assert!(nothing_selected.comment.is_none());
        assert!(nothing_selected.ports.is_empty());

        let no_document = graph_inspector_input_requests(None, Some("node-a"), Some("comment-a"));
        assert!(no_document.comment.is_none());
        assert!(no_document.ports.is_empty());
    }

    #[test]
    fn graph_inspector_inputs_hand_each_port_its_own_element() {
        let mut inputs = GraphInspectorInputs {
            comment: None,
            ports: vec![(7, div().into_any_element()), (9, div().into_any_element())],
        };

        assert!(inputs.take_port(8).is_none());
        assert!(inputs.take_port(9).is_some());
        assert!(
            inputs.take_port(9).is_none(),
            "an acquired input belongs to exactly one port row"
        );
        assert!(inputs.take_port(7).is_some());
    }

    fn input_port(
        port_id: u32,
        name: &str,
        current_value: Option<ReflectedValueEnvelope>,
    ) -> GraphPortProjectionData {
        GraphPortProjectionData {
            port_id,
            name: name.to_string(),
            direction: GraphPortDirectionData::Input,
            side: GraphPortSideData::West,
            value: current_value.map(|value| graph_input_value(value, None)),
            x: 0.0,
            y: 16.0,
        }
    }

    fn sample_document(unsaved_changes: bool) -> GraphDocumentProjectionData {
        GraphDocumentProjectionData {
            document_id: "graphs/test.azgraph.ron".to_string(),
            graph_type: "azoth.graph.test".to_string(),
            graph_type_info: None,
            revision: 7,
            saved_revision: Some(6),
            unsaved_changes,
            catalog_version: 1,
            nodes: vec![
                GraphNodeProjectionData {
                    node_id: "node-a".to_string(),
                    node_type: "az.test.Source".to_string(),
                    label: "Source".to_string(),
                    x: 0.0,
                    y: 0.0,
                    width: 180.0,
                    height: 88.0,
                    selected: true,
                    source_links: vec![GraphNodeSourceLinkProjectionData {
                        package: Some("az-editor-tests".to_string()),
                        module_path: Some("az_editor_tests::nodes".to_string()),
                        symbol_path: Some("az_editor_tests::nodes::SourceNode::run".to_string()),
                        file: Some("crates/editor/tests/src/nodes.rs".to_string()),
                        line: Some(42),
                        column: Some(9),
                        docs_url: None,
                    }],
                    ports: Vec::new(),
                },
                GraphNodeProjectionData {
                    node_id: "node-b".to_string(),
                    node_type: "az.test.Target".to_string(),
                    label: "Target".to_string(),
                    x: 300.0,
                    y: 92.0,
                    width: 120.0,
                    height: 88.0,
                    selected: false,
                    source_links: Vec::new(),
                    ports: Vec::new(),
                },
            ],
            connections: vec![GraphConnectionProjectionData {
                connection_id: "edge".to_string(),
                from_node_id: "node-a".to_string(),
                to_node_id: "node-b".to_string(),
                points: vec![
                    GraphPointProjectionData::new(-40.0, -10.0),
                    GraphPointProjectionData::new(180.0, -10.0),
                    GraphPointProjectionData::new(180.0, 140.0),
                    GraphPointProjectionData::new(300.0, 140.0),
                ],
                route_anchors: vec![GraphRouteAnchorProjectionData {
                    anchor_id: "anchor-1".to_string(),
                    kind: GraphRouteAnchorKindData::UserWaypoint,
                    x: 180.0,
                    y: 140.0,
                }],
                selected: false,
            }],
            comments: vec![GraphCommentProjectionData {
                comment_id: "comment-a".to_string(),
                text: "Comment".to_string(),
                x: 20.0,
                y: 20.0,
                width: 100.0,
                height: 40.0,
                selected: false,
            }],
            diagnostics: Vec::new(),
        }
    }

    fn graph_input_value(
        current_value: ReflectedValueEnvelope,
        default_value: Option<ReflectedValueEnvelope>,
    ) -> GraphInputValueProjectionData {
        let schema_type = current_value.type_path.clone();
        GraphInputValueProjectionData {
            schema_type,
            current_value: Some(current_value),
            default_value,
        }
    }

    fn reflected_value(type_path: &str, payload: &str) -> ReflectedValueEnvelope {
        ReflectedValueEnvelope::typed_ron(type_path, payload)
    }
}
