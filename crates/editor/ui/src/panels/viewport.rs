//! Viewport Panel
//!
//! Hosts the in-process editor viewport: the editor-owned bevy
//! renderer draws into a shared-device D3D12 texture that the GPUI Windows
//! backend composites at this panel's content rect, under the panel chrome.
//! The panel publishes its geometry ([`EditorViewportPanelFrame`]) and input
//! ([`EditorViewportInputQueue`]) for the editor-core viewport host to consume.
//! Runtime-host viewport streams (play-in-standalone) still publish metadata
//! through [`SharedViewportTexture`].

use crate::viewport_texture::SharedViewportTexture;
#[cfg(test)]
use crate::viewport_texture::ViewportFrameState;
use std::collections::BTreeSet;
#[cfg(test)]
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext as _, Bounds, Context, DragMoveEvent, FocusHandle, Focusable, Global,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Point, Render, ScrollWheelEvent, StatefulInteractiveElement, Styled,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::dock::Panel;
use gpui_component::theme::Theme;
use gpui_component::{ActiveTheme, ElementExt as _, Icon, IconName, Sizable, h_flex, v_flex};

use crate::panels::kit;
use crate::scene_tools::EditorSceneToolState;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorRuntimeProjectionCatalog {
    pub projections: Vec<RuntimeProjectionData>,
}

impl EditorRuntimeProjectionCatalog {
    #[must_use]
    pub const fn new(projections: Vec<RuntimeProjectionData>) -> Self {
        Self { projections }
    }
}

impl Global for EditorRuntimeProjectionCatalog {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorRuntimeStatus {
    pub runtime_id: String,
    pub state: EditorRuntimeStateData,
    pub role: Option<String>,
    pub project_id: Option<String>,
    pub session_slug: Option<String>,
    pub authored_revision: Option<u64>,
    pub diagnostics: Vec<String>,
}

impl EditorRuntimeStatus {
    #[must_use]
    pub fn unregistered(runtime_id: impl Into<String>) -> Self {
        Self {
            runtime_id: runtime_id.into(),
            state: EditorRuntimeStateData::Unregistered,
            role: None,
            project_id: None,
            session_slug: None,
            authored_revision: None,
            diagnostics: Vec::new(),
        }
    }
}

impl Global for EditorRuntimeStatus {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorGpuStatus {
    pub state: EditorGpuStateData,
    pub adapter_name: Option<String>,
    pub backend: Option<String>,
    pub device_type: Option<String>,
    pub driver: Option<String>,
    pub diagnostic: Option<String>,
}

impl EditorGpuStatus {
    #[must_use]
    pub const fn not_requested() -> Self {
        Self {
            state: EditorGpuStateData::NotRequested,
            adapter_name: None,
            backend: None,
            device_type: None,
            driver: None,
            diagnostic: None,
        }
    }

    #[must_use]
    pub const fn starting() -> Self {
        Self {
            state: EditorGpuStateData::Starting,
            adapter_name: None,
            backend: None,
            device_type: None,
            driver: None,
            diagnostic: None,
        }
    }

    #[must_use]
    pub fn ready(
        adapter_name: impl Into<String>,
        backend: impl Into<String>,
        device_type: impl Into<String>,
        driver: impl Into<String>,
    ) -> Self {
        Self {
            state: EditorGpuStateData::Ready,
            adapter_name: Some(adapter_name.into()),
            backend: Some(backend.into()),
            device_type: Some(device_type.into()),
            driver: Some(driver.into()),
            diagnostic: None,
        }
    }

    #[must_use]
    pub fn failed(diagnostic: impl Into<String>) -> Self {
        Self {
            state: EditorGpuStateData::Failed,
            adapter_name: None,
            backend: None,
            device_type: None,
            driver: None,
            diagnostic: Some(diagnostic.into()),
        }
    }
}

impl Default for EditorGpuStatus {
    fn default() -> Self {
        Self::not_requested()
    }
}

impl Global for EditorGpuStatus {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorViewportRenderStatus {
    pub state: EditorViewportRenderStateData,
    pub generation: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub format: Option<String>,
    pub backend: Option<String>,
    pub diagnostic: Option<String>,
    pub telemetry: Option<EditorViewportTelemetryData>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorViewportTelemetryData {
    pub fps: Option<u32>,
    pub frame_time_us: Option<u32>,
    pub draw_calls: Option<u64>,
    pub triangles: Option<u64>,
    pub vertices: Option<u64>,
    pub gpu_memory_bytes: Option<u64>,
}

impl EditorViewportRenderStatus {
    #[must_use]
    pub const fn waiting() -> Self {
        Self {
            state: EditorViewportRenderStateData::Waiting,
            generation: None,
            width: None,
            height: None,
            format: None,
            backend: None,
            diagnostic: None,
            telemetry: None,
        }
    }

    #[must_use]
    pub fn metadata_only(
        generation: u64,
        width: u32,
        height: u32,
        format: impl Into<String>,
    ) -> Self {
        Self::frame_status(
            EditorViewportRenderStateData::MetadataOnly,
            generation,
            width,
            height,
            format,
            None,
            None,
        )
    }

    #[must_use]
    pub fn editor_composition_surface(
        generation: u64,
        width: u32,
        height: u32,
        format: impl Into<String>,
        backend: impl Into<String>,
    ) -> Self {
        Self::frame_status(
            EditorViewportRenderStateData::EditorCompositionSurface,
            generation,
            width,
            height,
            format,
            Some(backend.into()),
            None,
        )
    }

    #[must_use]
    pub fn gpu_surface_handle(
        generation: u64,
        width: u32,
        height: u32,
        format: impl Into<String>,
        backend: impl Into<String>,
    ) -> Self {
        Self::frame_status(
            EditorViewportRenderStateData::GpuSurfaceHandle,
            generation,
            width,
            height,
            format,
            Some(backend.into()),
            None,
        )
    }

    #[must_use]
    pub fn failed(
        generation: u64,
        width: u32,
        height: u32,
        format: impl Into<String>,
        diagnostic: impl Into<String>,
    ) -> Self {
        Self::frame_status(
            EditorViewportRenderStateData::Failed,
            generation,
            width,
            height,
            format,
            None,
            Some(diagnostic.into()),
        )
    }

    fn frame_status(
        state: EditorViewportRenderStateData,
        generation: u64,
        width: u32,
        height: u32,
        format: impl Into<String>,
        backend: Option<String>,
        diagnostic: Option<String>,
    ) -> Self {
        Self {
            state,
            generation: Some(generation),
            width: Some(width),
            height: Some(height),
            format: Some(format.into()),
            backend,
            diagnostic,
            telemetry: None,
        }
    }

    #[must_use]
    pub const fn with_telemetry(mut self, telemetry: EditorViewportTelemetryData) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    /// The in-process renderer could not start; the
    /// viewport panel shows an explicit unavailable state instead of a surface.
    #[must_use]
    pub fn unavailable(diagnostic: impl Into<String>) -> Self {
        Self {
            state: EditorViewportRenderStateData::Failed,
            generation: None,
            width: None,
            height: None,
            format: None,
            backend: None,
            diagnostic: Some(diagnostic.into()),
            telemetry: None,
        }
    }
}

impl Default for EditorViewportRenderStatus {
    fn default() -> Self {
        Self::waiting()
    }
}

impl Global for EditorViewportRenderStatus {}

fn next_composition_hole_generation() -> u64 {
    static GENERATION: AtomicU64 = AtomicU64::new(1);
    GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// Convert the viewport panel's logical-pixel bounds into the device-pixel
/// rect the in-process render target must match. Pure so it is unit-testable.
#[must_use]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewportDeviceRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl ViewportDeviceRect {
    #[must_use]
    pub const fn width(self) -> u32 {
        if self.right > self.left {
            self.right.abs_diff(self.left)
        } else {
            0
        }
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        if self.bottom > self.top {
            self.bottom.abs_diff(self.top)
        } else {
            0
        }
    }
}

pub const PRIMARY_VIEWPORT_VISUAL_SLOT_ID: u64 = 1;

/// One scene-generation-bound viewport layout. The device extent is never
/// published separately from its integer edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportCompositionLayout {
    pub window_id: u64,
    pub slot_id: u64,
    pub scene_generation: u64,
    pub device_rect: ViewportDeviceRect,
    pub scale_factor: f32,
    pub visible: bool,
}

impl Default for ViewportCompositionLayout {
    fn default() -> Self {
        Self {
            window_id: 0,
            slot_id: PRIMARY_VIEWPORT_VISUAL_SLOT_ID,
            scene_generation: 0,
            device_rect: ViewportDeviceRect::default(),
            scale_factor: 1.0,
            visible: false,
        }
    }
}

/// Round a scaled logical coordinate to the device pixel it lands on.
///
/// Rust has no checked float-to-integer conversion, but `as` here is defined
/// to saturate at `i32`'s bounds and to map `NaN` to zero — which is the
/// clamp this wants anyway: a viewport edge that far outside `i32` is off
/// screen whichever bound it saturates to.
const fn device_pixel(scaled: f32) -> i32 {
    // The saturating behaviour above is exactly what a checked conversion
    // would have to fall back to, and no such conversion exists for f32.
    #[allow(clippy::cast_possible_truncation)]
    {
        scaled.round() as i32
    }
}

pub fn viewport_device_rect(
    origin: (f32, f32),
    size: (f32, f32),
    scale_factor: f32,
) -> ViewportDeviceRect {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let left = device_pixel(origin.0 * scale);
    let top = device_pixel(origin.1 * scale);
    let right = device_pixel((origin.0 + size.0.max(0.0)) * scale);
    let bottom = device_pixel((origin.1 + size.1.max(0.0)) * scale);
    ViewportDeviceRect {
        left,
        top,
        right: right.max(left),
        bottom: bottom.max(top),
    }
}

/// Which editor mode owns the in-process viewport surface for the frame being
/// painted.
///
/// The composition tree currently exposes one typed visual slot, so one panel
/// owns it at a time; the owner tag tells the editor-core viewport host which
/// content to render into it — the authored scene bridge (Scene) or the
/// mannequin/blend-space animation previews (Animation). The mannequin must
/// never render while a Scene-owned surface is live.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewportSurfaceOwner {
    #[default]
    Scene,
    Animation,
}

/// Panel-published geometry of the in-process viewport surface. The editor
/// core's viewport host reads this every pump tick to size the render target
/// and decide whether the surface is currently visible.
#[derive(Clone, Debug, Default)]
pub struct EditorViewportPanelFrame {
    /// Authoritative layout for the latest painted GPUI scene.
    pub layout: ViewportCompositionLayout,
    /// When the panel last painted the surface; `None` until the first paint.
    pub painted_at: Option<Instant>,
    /// Which mode's panel painted the surface (selects the pump's content).
    pub owner: ViewportSurfaceOwner,
}

impl EditorViewportPanelFrame {
    pub fn publish(
        &mut self,
        bounds: Bounds<Pixels>,
        scale_factor: f32,
        window_id: u64,
        owner: ViewportSurfaceOwner,
    ) {
        let device_rect = viewport_device_rect(
            (f32::from(bounds.origin.x), f32::from(bounds.origin.y)),
            (f32::from(bounds.size.width), f32::from(bounds.size.height)),
            scale_factor,
        );
        self.layout = ViewportCompositionLayout {
            window_id,
            slot_id: PRIMARY_VIEWPORT_VISUAL_SLOT_ID,
            scene_generation: self.layout.scene_generation.saturating_add(1),
            device_rect,
            scale_factor,
            visible: device_rect.width() > 0 && device_rect.height() > 0,
        };
        self.painted_at = Some(Instant::now());
        self.owner = owner;
    }

    /// Whether the panel painted the surface recently enough to treat it as
    /// visible (the viewport host refreshes windows continuously while live).
    #[must_use]
    pub fn is_fresh(&self, max_age: Duration) -> bool {
        self.painted_at
            .is_some_and(|painted_at| painted_at.elapsed() <= max_age)
            && self.layout.visible
            && self.layout.device_rect.width() > 0
            && self.layout.device_rect.height() > 0
    }
}

impl Global for EditorViewportPanelFrame {}

/// One user-input event captured over the in-process viewport surface,
/// forwarded to the editor-core viewport host.
///
/// Coordinates and deltas are normalized to the surface size (`[0, 1]` for
/// positions, fractions of the surface for drag deltas) so the host stays
/// resolution-independent.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewportInputEvent {
    /// Left click at a normalized `[0, 1]` surface coordinate (origin top-left).
    Pick { x: f32, y: f32 },
    /// Left button pressed at a normalized `[0, 1]` surface coordinate. The host
    /// tries to grab a transform-gizmo handle here; selection is deferred until
    /// mouse-up proves the gesture remained a click.
    PointerDown { interaction_id: u64, x: f32, y: f32 },
    /// Left button held and moved to a normalized `[0, 1]` surface coordinate
    /// (absolute, not a delta) — drives an in-progress gizmo drag.
    PointerMove { interaction_id: u64, x: f32, y: f32 },
    /// Left button released at a normalized `[0, 1]` surface coordinate; ends an
    /// in-progress gizmo drag.
    PointerUp {
        interaction_id: u64,
        x: f32,
        y: f32,
        is_click: bool,
    },
    /// A completed click superseded by a newer completed click before the host
    /// could pump. It is retained only for the fixed-size input trace.
    ClickCoalesced { interaction_id: u64 },
    /// Coalesced hover probe. At most the latest coordinate is consumed by a
    /// rendered frame.
    HoverMove { x: f32, y: f32 },
    /// Pointer left the viewport; remove any hover highlight.
    HoverLeave,
    /// Right-drag orbit delta as a fraction of the surface size.
    Orbit { dx: f32, dy: f32 },
    /// Middle-drag pan delta as a fraction of the surface size.
    Pan { dx: f32, dy: f32 },
    /// Semantic start of a camera drag. The producer uses the interaction id
    /// to bound a fixed-capacity performance sample window.
    CameraDragStart {
        interaction_id: u64,
        kind: ViewportCameraDragKind,
        x: f32,
        y: f32,
        started_at: Instant,
    },
    /// Semantic end of the active camera drag, retaining the final absolute
    /// cursor position and timestamp even if the producer has not yet sampled
    /// the corresponding start.
    CameraDragEnd {
        interaction_id: u64,
        x: f32,
        y: f32,
        ended_at: Instant,
    },
    /// Scroll-wheel dolly steps; positive moves the camera toward the focus.
    Dolly { steps: f32 },
    /// Set a named editor camera pose.
    SetCameraView { view: ViewportCameraView },
    /// Set the renderer's material/wireframe presentation.
    SetShadingMode { mode: ViewportShadingMode },
    /// Apply semantic viewport visibility switches.
    SetVisibility {
        settings: ViewportVisibilitySettings,
    },
    /// Frame the selected authored entity from its resolved mesh bounds.
    FrameSelected,
    /// A dragged asset first crossed into the viewport.
    AssetDragEnter {
        interaction_id: u64,
        source_path: String,
        x: f32,
        y: f32,
    },
    /// The dragged cursor left the viewport without dropping.
    AssetDragLeave { interaction_id: u64 },
    /// An asset dropped from the Asset Browser at a normalized `[0, 1]` surface
    /// coordinate, carrying the asset's source path.
    DropAsset {
        interaction_id: u64,
        source_path: String,
        x: f32,
        y: f32,
    },
}

/// Camera manipulation selected by a semantic pointer gesture. The producer
/// samples the absolute cursor once per render tick and derives deltas there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportCameraDragKind {
    Orbit,
    Pan,
}

/// Allocation-free UI-side trace events forwarded to editor-core's perf ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewportUiInputTrace {
    PointerDown { interaction_id: u64, x: f32, y: f32 },
    AssetDragMove { interaction_id: u64, x: f32, y: f32 },
    AssetDragLeave { interaction_id: u64 },
    AssetDrop { interaction_id: u64, x: f32, y: f32 },
}

static VIEWPORT_INPUT_TRACE_SINK: OnceLock<fn(ViewportUiInputTrace)> = OnceLock::new();

/// Install editor-core's zero-allocation input trace sink.
pub fn set_viewport_input_trace_sink(sink: fn(ViewportUiInputTrace)) {
    let _ = VIEWPORT_INPUT_TRACE_SINK.set(sink);
}

fn trace_viewport_input(event: ViewportUiInputTrace) {
    if let Some(sink) = VIEWPORT_INPUT_TRACE_SINK.get() {
        sink(event);
    }
}

static NEXT_VIEWPORT_INTERACTION_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

fn next_viewport_interaction_id() -> u64 {
    NEXT_VIEWPORT_INTERACTION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

const CLICK_DRAG_THRESHOLD_PX: f32 = 4.0;

#[must_use]
fn exceeds_click_drag_threshold(dx: f32, dy: f32) -> bool {
    dx.mul_add(dx, dy * dy) > CLICK_DRAG_THRESHOLD_PX * CLICK_DRAG_THRESHOLD_PX
}

/// Drag payload published by Asset Browser rows for a drag into the viewport.
/// Doubles as its own drag-preview view.
#[derive(Clone)]
pub struct ViewportAssetDrag {
    pub source_path: String,
    pub interaction_id: u64,
}

impl ViewportAssetDrag {
    #[must_use]
    pub fn new(source_path: String) -> Self {
        Self {
            source_path,
            interaction_id: next_viewport_interaction_id(),
        }
    }
}

impl Render for ViewportAssetDrag {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme();
        let name = self
            .source_path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(self.source_path.as_str())
            .to_string();
        div()
            .px_2()
            .py_1()
            .rounded(px(4.0))
            .bg(theme.secondary)
            .border_1()
            .border_color(theme.list_active_border)
            .text_size(px(11.0))
            .text_color(theme.foreground)
            .child(name)
    }
}

/// Queue of viewport input events drained by the viewport host each pump tick.
#[derive(Debug, Default)]
pub struct EditorViewportInputQueue {
    events: Vec<ViewportInputEvent>,
    hover: Option<ViewportInputEvent>,
    asset_drag: Option<ViewportInputEvent>,
}

impl EditorViewportInputQueue {
    pub fn push(&mut self, event: ViewportInputEvent) {
        if matches!(
            event,
            ViewportInputEvent::HoverMove { .. } | ViewportInputEvent::HoverLeave
        ) {
            self.hover = Some(event);
            return;
        }
        if matches!(event, ViewportInputEvent::AssetDragLeave { .. }) {
            self.asset_drag = Some(event);
            return;
        }
        // Bound the queue so a stalled pump can't grow it without limit.
        if self.events.len() < 1024 {
            self.events.push(event);
        }
    }

    #[must_use]
    pub fn drain(&mut self) -> Vec<ViewportInputEvent> {
        let mut events = std::mem::take(&mut self.events);
        if let Some(asset_drag) = self.asset_drag.take() {
            events.push(asset_drag);
        }

        // Completed click gestures are latest-wins. Preserve camera/gizmo
        // drags, but collapse older click pairs into trace-only markers so a
        // stalled renderer can never replay a visible selection backlog.
        let latest_click = events.iter().rev().find_map(|event| match event {
            ViewportInputEvent::PointerUp {
                interaction_id,
                is_click: true,
                ..
            } => Some(*interaction_id),
            _ => None,
        });
        if let Some(latest_click) = latest_click {
            let mut coalesced = [0_u64; 64];
            let mut coalesced_len = 0;
            events.retain(|event| match event {
                ViewportInputEvent::PointerDown { interaction_id, .. }
                | ViewportInputEvent::PointerMove { interaction_id, .. }
                | ViewportInputEvent::PointerUp {
                    interaction_id,
                    is_click: true,
                    ..
                } if *interaction_id != latest_click => {
                    if coalesced_len < coalesced.len()
                        && !coalesced[..coalesced_len].contains(interaction_id)
                    {
                        coalesced[coalesced_len] = *interaction_id;
                        coalesced_len += 1;
                    }
                    false
                }
                _ => true,
            });
            events.extend(
                coalesced[..coalesced_len]
                    .iter()
                    .copied()
                    .map(|interaction_id| ViewportInputEvent::ClickCoalesced { interaction_id }),
            );
        }
        events
    }

    /// Take the single coalesced hover probe without growing the event vector.
    #[must_use]
    pub const fn take_hover(&mut self) -> Option<ViewportInputEvent> {
        self.hover.take()
    }
}

impl Global for EditorViewportInputQueue {}

/// Live pose of the in-process editor camera, published by the viewport host
/// every frame so the panel's badges and orientation triad reflect reality.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorViewportCameraState {
    pub yaw_radians: f32,
    pub pitch_radians: f32,
    pub distance: f32,
    pub speed: f32,
}

impl Default for EditorViewportCameraState {
    fn default() -> Self {
        Self {
            yaw_radians: 0.0,
            pitch_radians: 0.0,
            distance: 10.0,
            speed: 4.0,
        }
    }
}

impl Global for EditorViewportCameraState {}

/// Named editor camera views exposed by the viewport camera pill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewportCameraView {
    #[default]
    Perspective,
    Top,
    Front,
    Side,
    Game,
}

impl ViewportCameraView {
    const ALL: [Self; 5] = [
        Self::Perspective,
        Self::Top,
        Self::Front,
        Self::Side,
        Self::Game,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Perspective => "Perspective",
            Self::Top => "Top",
            Self::Front => "Front",
            Self::Side => "Side",
            Self::Game => "Game",
        }
    }
}

/// Render shading exposed by the viewport shading pill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewportShadingMode {
    #[default]
    Lit,
    Unlit,
    Wireframe,
}

impl ViewportShadingMode {
    const ALL: [Self; 3] = [Self::Lit, Self::Unlit, Self::Wireframe];

    const fn label(self) -> &'static str {
        match self {
            Self::Lit => "Lit",
            Self::Unlit => "Unlit",
            Self::Wireframe => "Wireframe",
        }
    }
}

/// Visibility switches shared by viewport chrome and the production renderer.
/// Unsupported gizmo categories stay out of this state so the UI cannot
/// pretend a backend exists.
// One named `bool` per switch is the cross-crate contract: az-editor-core's
// renderer reads `.skybox`, `.bounds` and `.bounding_boxes` off this struct by
// field, so collapsing them into a set is a change to that crate, not this one.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportVisibilitySettings {
    pub grid: bool,
    pub stats_overlay: bool,
    pub bounds: bool,
    pub skybox: bool,
    pub bounding_boxes: bool,
}

impl Default for ViewportVisibilitySettings {
    fn default() -> Self {
        Self {
            grid: true,
            stats_overlay: true,
            bounds: true,
            skybox: true,
            bounding_boxes: false,
        }
    }
}

/// Screen-space offsets (x right, y down, unit length) of the world X/Y/Z axes
/// under an orbit camera with the given yaw/pitch, for the orientation triad.
/// Pure math so it is unit-testable.
#[must_use]
pub fn triad_axis_directions(yaw_radians: f32, pitch_radians: f32) -> [(f32, f32); 3] {
    let (sin_yaw, cos_yaw) = yaw_radians.sin_cos();
    let (sin_pitch, cos_pitch) = pitch_radians.sin_cos();
    // Orbit camera basis: the camera sits at `focus + distance * dir` with
    // `dir = (cos(pitch)sin(yaw), sin(pitch), cos(pitch)cos(yaw))`, looking at
    // the focus with +Y up. Screen x is the camera right vector, screen y is
    // minus the camera up vector (screen y grows downward).
    let right = [cos_yaw, 0.0, -sin_yaw];
    let up = [-sin_pitch * sin_yaw, cos_pitch, -sin_pitch * cos_yaw];
    let project = |axis: [f32; 3]| {
        (
            axis[2].mul_add(right[2], axis[1].mul_add(right[1], axis[0] * right[0])),
            -axis[2].mul_add(up[2], axis[1].mul_add(up[1], axis[0] * up[0])),
        )
    };
    [
        project([1.0, 0.0, 0.0]),
        project([0.0, 1.0, 0.0]),
        project([0.0, 0.0, 1.0]),
    ]
}

pub const DEFAULT_MANNEQUIN_CHARACTER_GLB: &str =
    "characters/player/female/marauderfaction/female_marauderfaction_chest.glb";
pub const DEFAULT_MANNEQUIN_MOTION_GLB: &str =
    "animations/gameplay/character/player/male/dual_daggers/combat/dual_daggers_prim_1.anim.glb";

/// Minimal editor-to-viewport content bridge state for the mannequin preview.
///
/// Paths are project-asset-root relative. An explicit `None` character renders
/// the neutral viewport scene; a selected character replaces the neutral
/// primitives with the authored glTF scene and, optionally, an animation clip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinPreview {
    pub project_asset_root: Option<PathBuf>,
    pub character_glb: Option<String>,
    pub motion_glb: Option<String>,
    pub playing: bool,
    pub looping: bool,
    pub position_millis: u32,
}

impl EditorMannequinPreview {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            project_asset_root: None,
            character_glb: None,
            motion_glb: None,
            playing: false,
            looping: false,
            position_millis: 0,
        }
    }

    #[must_use]
    pub fn default_for_project_root(project_root: impl AsRef<Path>) -> Self {
        Self::default_for_project_asset_root(project_root.as_ref().join("assets"))
    }

    #[must_use]
    pub fn default_for_project_asset_root(project_asset_root: impl Into<PathBuf>) -> Self {
        Self {
            project_asset_root: Some(project_asset_root.into()),
            character_glb: Some(DEFAULT_MANNEQUIN_CHARACTER_GLB.to_owned()),
            motion_glb: Some(DEFAULT_MANNEQUIN_MOTION_GLB.to_owned()),
            playing: true,
            looping: true,
            position_millis: 0,
        }
    }

    pub fn select_character(&mut self, character_glb: impl Into<String>) {
        let character_glb = character_glb.into();
        if character_glb.trim().is_empty() {
            self.character_glb = None;
        } else {
            self.character_glb = Some(character_glb);
        }
        self.position_millis = 0;
    }

    pub fn select_motion(&mut self, motion_glb: impl Into<String>) {
        let motion_glb = motion_glb.into();
        if motion_glb.trim().is_empty() {
            self.motion_glb = None;
            self.playing = false;
        } else {
            self.motion_glb = Some(motion_glb);
            self.playing = true;
        }
        self.position_millis = 0;
    }

    pub const fn set_playing(&mut self, playing: bool) {
        self.playing = playing && self.motion_glb.is_some();
    }

    pub const fn stop(&mut self) {
        self.playing = false;
        self.position_millis = 0;
    }

    pub const fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    pub const fn seek_millis(&mut self, position_millis: u32) {
        self.position_millis = position_millis;
    }

    #[must_use]
    pub fn resolve_character_glb(&self) -> Option<PathBuf> {
        self.resolve_asset_path(self.character_glb.as_deref()?)
    }

    #[must_use]
    pub fn resolve_motion_glb(&self) -> Option<PathBuf> {
        self.motion_glb
            .as_deref()
            .and_then(|path| self.resolve_asset_path(path))
    }

    fn resolve_asset_path(&self, asset_path: &str) -> Option<PathBuf> {
        let root = self.project_asset_root.as_ref()?;
        if asset_path.trim().is_empty() {
            return None;
        }
        Some(root.join(Path::new(asset_path)))
    }
}

impl Default for EditorMannequinPreview {
    fn default() -> Self {
        Self::empty()
    }
}

impl Global for EditorMannequinPreview {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorBlendSpacePreviewCatalog {
    pub project_asset_root: Option<PathBuf>,
    pub blend_spaces: Vec<EditorBlendSpaceAssetData>,
    pub diagnostics: Vec<String>,
}

impl EditorBlendSpacePreviewCatalog {
    #[must_use]
    pub const fn new(
        project_asset_root: Option<PathBuf>,
        blend_spaces: Vec<EditorBlendSpaceAssetData>,
        diagnostics: Vec<String>,
    ) -> Self {
        Self {
            project_asset_root,
            blend_spaces,
            diagnostics,
        }
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self {
            project_asset_root: None,
            blend_spaces: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

impl Global for EditorBlendSpacePreviewCatalog {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorBlendSpaceAssetKind {
    BlendSpace,
    CombinedBlendSpace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorBlendSpaceAssetData {
    pub asset_path: String,
    pub source_path: String,
    pub name: String,
    pub asset_kind: EditorBlendSpaceAssetKind,
    pub dimension_count: usize,
    pub example_count: usize,
    pub has_vgrid: bool,
    pub member_paths: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorBlendSpacePreview {
    pub project_asset_root: Option<PathBuf>,
    pub bspace_ron_path: Option<String>,
    pub param_values: Vec<f32>,
    pub document: Option<EditorBlendSpaceData>,
    pub weights: Vec<EditorBlendSpaceWeightData>,
    pub diagnostics: Vec<String>,
}

impl EditorBlendSpacePreview {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            project_asset_root: None,
            bspace_ron_path: None,
            param_values: Vec::new(),
            document: None,
            weights: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_project_asset_root(project_asset_root: impl Into<PathBuf>) -> Self {
        Self {
            project_asset_root: Some(project_asset_root.into()),
            ..Self::empty()
        }
    }

    pub fn set_document(
        &mut self,
        bspace_ron_path: impl Into<String>,
        document: EditorBlendSpaceData,
        diagnostics: Vec<String>,
    ) {
        self.bspace_ron_path = Some(bspace_ron_path.into());
        self.param_values = document.default_param_values_with_previous(&self.param_values);
        self.weights = document.weights_for_params(&self.param_values);
        self.document = Some(document);
        self.diagnostics = diagnostics;
    }

    pub fn clear_selection(&mut self) {
        self.bspace_ron_path = None;
        self.param_values.clear();
        self.document = None;
        self.weights.clear();
        self.diagnostics.clear();
    }

    #[must_use]
    pub fn resolve_bspace_ron_path(&self) -> Option<PathBuf> {
        let root = self.project_asset_root.as_ref()?;
        let path = self.bspace_ron_path.as_deref()?.trim();
        if path.is_empty() {
            return None;
        }
        Some(root.join(Path::new(path)))
    }

    pub fn set_param_value(&mut self, dimension: &str, value: f32) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let Some(index) = document.dimension_index(dimension) else {
            return false;
        };
        let Some(param) = self.param_values.get_mut(index) else {
            return false;
        };
        let next = document.dimensions[index].clamp_value(value);
        if (*param - next).abs() <= f32::EPSILON {
            return false;
        }
        *param = next;
        self.weights = document.weights_for_params(&self.param_values);
        true
    }

    pub fn set_param_values(&mut self, values: &[f32]) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let next = document.clamp_param_values(values);
        if self.param_values == next {
            return false;
        }
        self.param_values = next;
        self.weights = document.weights_for_params(&self.param_values);
        true
    }
}

impl Global for EditorBlendSpacePreview {}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorBlendSpaceData {
    pub source_path: String,
    pub dimensions: Vec<EditorBlendSpaceDimensionData>,
    pub examples: Vec<EditorBlendSpaceExampleData>,
    pub virtual_examples: Vec<EditorBlendSpaceVirtualExampleData>,
}

impl EditorBlendSpaceData {
    #[must_use]
    pub fn dimension_index(&self, dimension: &str) -> Option<usize> {
        if let Ok(index) = dimension.parse::<usize>()
            && index < self.dimensions.len()
        {
            return Some(index);
        }
        self.dimensions
            .iter()
            .position(|candidate| candidate.name == dimension)
    }

    #[must_use]
    pub fn default_param_values(&self) -> Vec<f32> {
        self.dimensions
            .iter()
            .map(EditorBlendSpaceDimensionData::midpoint)
            .collect()
    }

    #[must_use]
    pub fn default_param_values_with_previous(&self, previous: &[f32]) -> Vec<f32> {
        if previous.len() == self.dimensions.len() {
            self.clamp_param_values(previous)
        } else {
            self.default_param_values()
        }
    }

    #[must_use]
    pub fn clamp_param_values(&self, values: &[f32]) -> Vec<f32> {
        self.dimensions
            .iter()
            .enumerate()
            .map(|(index, dimension)| {
                values.get(index).copied().map_or_else(
                    || dimension.midpoint(),
                    |value| dimension.clamp_value(value),
                )
            })
            .collect()
    }

    #[must_use]
    pub fn weights_for_params(&self, param_values: &[f32]) -> Vec<EditorBlendSpaceWeightData> {
        let values = self.clamp_param_values(param_values);
        let weights = self.weight_values_for_params(&values);
        self.examples
            .iter()
            .enumerate()
            .map(|(index, example)| EditorBlendSpaceWeightData {
                example_index: index,
                animation_name: example.animation_name.clone(),
                motion_path: example.motion_path.clone(),
                weight: weights.get(index).copied().unwrap_or_default(),
            })
            .collect()
    }

    #[must_use]
    pub fn weight_values_for_params(&self, param_values: &[f32]) -> Vec<f32> {
        if self.examples.is_empty() {
            return Vec::new();
        }
        if let Some(weights) = self.vgrid_weight_values_for_params(param_values) {
            return normalize_weights(weights);
        }
        normalize_weights(self.inverse_distance_weight_values_for_params(param_values))
    }

    fn vgrid_weight_values_for_params(&self, param_values: &[f32]) -> Option<Vec<f32>> {
        if self.virtual_examples.is_empty() || self.dimensions.is_empty() {
            return None;
        }
        let cells = self
            .dimensions
            .iter()
            .map(|dimension| dimension.cells.max(1))
            .collect::<Vec<_>>();
        let expected = cells
            .iter()
            .try_fold(1usize, |product, cells| product.checked_mul(*cells))?;
        if expected != self.virtual_examples.len() {
            return None;
        }

        let mut strides = Vec::with_capacity(cells.len());
        let mut stride = 1usize;
        for cell_count in &cells {
            strides.push(stride);
            stride = stride.checked_mul(*cell_count)?;
        }

        let values = self.clamp_param_values(param_values);
        let mut bases = Vec::with_capacity(self.dimensions.len());
        let mut fractions = Vec::with_capacity(self.dimensions.len());
        for ((dimension, cell_count), value) in self.dimensions.iter().zip(&cells).zip(&values) {
            if *cell_count <= 1 {
                bases.push(0usize);
                fractions.push(0.0);
                continue;
            }
            let last_cell = *cell_count - 1;
            let scaled = dimension.normalized_value(*value) * kit::count(last_cell);
            let floor = scaled.floor().clamp(0.0, kit::count(last_cell));
            // `floor` is clamped into 0.0..=last_cell just above, so this
            // narrowing can neither go negative nor exceed the cell count;
            // Rust offers no checked f32 -> usize conversion to say that with.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let base = floor as usize;
            let next = (base + 1).min(last_cell);
            bases.push(base.min(next));
            fractions.push((scaled - kit::count(base)).clamp(0.0, 1.0));
        }

        let mut weights = vec![0.0; self.examples.len()];
        let corner_count = u32::try_from(self.dimensions.len())
            .ok()
            .and_then(|dimensions| 1usize.checked_shl(dimensions))
            .unwrap_or(0);
        if corner_count == 0 {
            return None;
        }
        for corner in 0..corner_count {
            let mut corner_weight = 1.0;
            let mut grid_index = 0usize;
            for dimension_index in 0..self.dimensions.len() {
                let use_upper = ((corner >> dimension_index) & 1) == 1;
                let cell_count = cells[dimension_index];
                let base = bases[dimension_index];
                let fraction = fractions[dimension_index];
                let coordinate = if use_upper {
                    corner_weight *= fraction;
                    (base + 1).min(cell_count - 1)
                } else {
                    corner_weight *= 1.0 - fraction;
                    base
                };
                grid_index += coordinate * strides[dimension_index];
            }
            if corner_weight <= f32::EPSILON {
                continue;
            }
            let virtual_example = self.virtual_examples.get(grid_index)?;
            for (example_index, weight) in virtual_example.iter_weights() {
                if let Some(slot) = weights.get_mut(example_index) {
                    *slot = weight.mul_add(corner_weight, *slot);
                }
            }
        }
        Some(weights)
    }

    fn inverse_distance_weight_values_for_params(&self, param_values: &[f32]) -> Vec<f32> {
        let values = self.clamp_param_values(param_values);
        let candidates = self.surrounding_example_indices(&values);
        let indices = if candidates.is_empty() {
            (0..self.examples.len()).collect::<Vec<_>>()
        } else {
            candidates
        };

        let mut weights = vec![0.0; self.examples.len()];
        let mut weighted = Vec::new();
        for index in indices {
            let distance = self.example_distance(index, &values);
            if distance <= 0.0001 {
                weights[index] = 1.0;
                return weights;
            }
            weighted.push((index, 1.0 / distance.max(0.0001).powi(2)));
        }
        for (index, weight) in weighted {
            weights[index] = weight;
        }
        weights
    }

    fn surrounding_example_indices(&self, param_values: &[f32]) -> Vec<usize> {
        let bounds = self
            .dimensions
            .iter()
            .enumerate()
            .map(|(dimension_index, dimension)| {
                let value = param_values
                    .get(dimension_index)
                    .copied()
                    .unwrap_or_else(|| dimension.midpoint());
                let mut lower = None::<f32>;
                let mut upper = None::<f32>;
                for example in &self.examples {
                    let coordinate = example.coordinate_for_dimension(&dimension.name)?;
                    if coordinate <= value {
                        lower = Some(lower.map_or(coordinate, |current| current.max(coordinate)));
                    }
                    if coordinate >= value {
                        upper = Some(upper.map_or(coordinate, |current| current.min(coordinate)));
                    }
                }
                Some((dimension.name.as_str(), lower, upper))
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();

        if bounds.len() != self.dimensions.len() {
            return Vec::new();
        }

        self.examples
            .iter()
            .enumerate()
            .filter_map(|(index, example)| {
                let inside = bounds.iter().all(|(dimension, lower, upper)| {
                    let Some(coordinate) = example.coordinate_for_dimension(dimension) else {
                        return false;
                    };
                    lower.is_some_and(|lower| nearly_equal(coordinate, lower))
                        || upper.is_some_and(|upper| nearly_equal(coordinate, upper))
                });
                inside.then_some(index)
            })
            .collect()
    }

    fn example_distance(&self, example_index: usize, param_values: &[f32]) -> f32 {
        let Some(example) = self.examples.get(example_index) else {
            return f32::INFINITY;
        };
        self.dimensions
            .iter()
            .enumerate()
            .map(|(dimension_index, dimension)| {
                let target = param_values
                    .get(dimension_index)
                    .copied()
                    .unwrap_or_else(|| dimension.midpoint());
                let coordinate = example
                    .coordinate_for_dimension(&dimension.name)
                    .unwrap_or_else(|| dimension.midpoint());
                let range = (dimension.max - dimension.min).abs().max(0.0001);
                ((coordinate - target) / range).powi(2)
            })
            .sum::<f32>()
            .sqrt()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorBlendSpaceDimensionData {
    pub name: String,
    pub parameter_id: Option<u32>,
    pub min: f32,
    pub max: f32,
    pub cells: usize,
    pub locked: bool,
}

impl EditorBlendSpaceDimensionData {
    #[must_use]
    pub fn midpoint(&self) -> f32 {
        (self.min + self.max) * 0.5
    }

    #[must_use]
    pub fn clamp_value(&self, value: f32) -> f32 {
        if value.is_finite() {
            value.clamp(self.min.min(self.max), self.min.max(self.max))
        } else {
            self.midpoint()
        }
    }

    #[must_use]
    pub fn normalized_value(&self, value: f32) -> f32 {
        let range = self.max - self.min;
        if range.abs() <= f32::EPSILON {
            0.0
        } else {
            ((self.clamp_value(value) - self.min) / range).clamp(0.0, 1.0)
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorBlendSpaceExampleData {
    pub animation_name: String,
    pub motion_path: String,
    pub coordinates: Vec<EditorBlendSpaceCoordinateData>,
    pub playback_scale: Option<f32>,
}

impl EditorBlendSpaceExampleData {
    #[must_use]
    pub fn coordinate_for_dimension(&self, dimension: &str) -> Option<f32> {
        self.coordinates
            .iter()
            .find(|coordinate| coordinate.dimension == dimension)
            .map(|coordinate| coordinate.value)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorBlendSpaceCoordinateData {
    pub dimension: String,
    pub value: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorBlendSpaceVirtualExampleData {
    pub indices: Vec<usize>,
    pub weights: Vec<f32>,
}

impl EditorBlendSpaceVirtualExampleData {
    fn iter_weights(&self) -> impl Iterator<Item = (usize, f32)> + '_ {
        self.indices
            .iter()
            .copied()
            .zip(self.weights.iter().copied())
            .filter(|(_, weight)| *weight > 0.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorBlendSpaceWeightData {
    pub example_index: usize,
    pub animation_name: String,
    pub motion_path: String,
    pub weight: f32,
}

fn normalize_weights(mut weights: Vec<f32>) -> Vec<f32> {
    let total = weights
        .iter()
        .copied()
        .filter(|weight| weight.is_finite() && *weight > 0.0)
        .sum::<f32>();
    if total <= f32::EPSILON {
        if let Some(first) = weights.first_mut() {
            *first = 1.0;
        }
        return weights;
    }
    for weight in &mut weights {
        if weight.is_finite() && *weight > 0.0 {
            *weight /= total;
        } else {
            *weight = 0.0;
        }
    }
    weights
}

fn nearly_equal(left: f32, right: f32) -> bool {
    (left - right).abs() <= 0.0001
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorAnimationPreviewCatalog {
    pub project_asset_root: Option<PathBuf>,
    pub motions: Vec<EditorAnimationMotionData>,
    pub skeleton_joints: Vec<EditorAnimationJointData>,
    pub diagnostics: Vec<String>,
}

impl EditorAnimationPreviewCatalog {
    #[must_use]
    pub const fn new(
        project_asset_root: Option<PathBuf>,
        motions: Vec<EditorAnimationMotionData>,
        skeleton_joints: Vec<EditorAnimationJointData>,
        diagnostics: Vec<String>,
    ) -> Self {
        Self {
            project_asset_root,
            motions,
            skeleton_joints,
            diagnostics,
        }
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self {
            project_asset_root: None,
            motions: Vec::new(),
            skeleton_joints: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[must_use]
    pub fn selected_motion(
        &self,
        preview: Option<&EditorMannequinPreview>,
    ) -> Option<&EditorAnimationMotionData> {
        let motion_glb = preview?.motion_glb.as_deref()?;
        self.motions
            .iter()
            .find(|motion| motion.asset_path == motion_glb)
    }
}

impl Global for EditorAnimationPreviewCatalog {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorMannequinAuthoringCatalog {
    pub project_asset_root: Option<PathBuf>,
    pub fragments: Vec<EditorMannequinFragmentData>,
    pub tags: Vec<EditorMannequinTagData>,
    pub tag_groups: Vec<EditorMannequinTagGroupData>,
    pub fragment_blends: Vec<EditorMannequinFragmentBlendData>,
    pub fragment_definitions: Vec<EditorMannequinFragmentDefinitionData>,
    pub scope_contexts: Vec<EditorMannequinScopeContextData>,
    pub scopes: Vec<EditorMannequinScopeData>,
    pub selected_fragment_key: Option<String>,
    pub enabled_tags: BTreeSet<String>,
    pub resolved: Option<EditorMannequinResolvedAnimationData>,
    pub diagnostics: Vec<String>,
}

impl EditorMannequinAuthoringCatalog {
    #[must_use]
    pub fn new(project_asset_root: Option<PathBuf>) -> Self {
        Self {
            project_asset_root,
            ..Self::default()
        }
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self {
            project_asset_root: None,
            fragments: Vec::new(),
            tags: Vec::new(),
            tag_groups: Vec::new(),
            fragment_blends: Vec::new(),
            fragment_definitions: Vec::new(),
            scope_contexts: Vec::new(),
            scopes: Vec::new(),
            selected_fragment_key: None,
            enabled_tags: BTreeSet::new(),
            resolved: None,
            diagnostics: Vec::new(),
        }
    }

    #[must_use]
    pub fn selected_fragment(&self) -> Option<&EditorMannequinFragmentData> {
        let selected = self.selected_fragment_key.as_deref()?;
        self.fragments
            .iter()
            .find(|fragment| fragment.key == selected)
    }

    #[must_use]
    pub fn tag_group_for_tag(&self, tag_name: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|tag| tag.name == tag_name)
            .and_then(|tag| tag.group.as_deref())
    }

    #[must_use]
    pub fn has_tag(&self, tag_name: &str) -> bool {
        self.tags.iter().any(|tag| tag.name == tag_name)
    }
}

impl Global for EditorMannequinAuthoringCatalog {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinFragmentData {
    pub key: String,
    pub name: String,
    pub source_path: String,
    pub option_count: usize,
    pub options: Vec<EditorMannequinFragmentOptionData>,
    pub scopes: Vec<String>,
    pub flags: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinFragmentOptionData {
    pub key: String,
    pub index: usize,
    pub required_tags: Vec<String>,
    pub excluded_tags: Vec<String>,
    pub fragment_tags: Vec<String>,
    pub animation_refs: Vec<EditorMannequinAnimationRefData>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinAnimationRefData {
    pub original: String,
    pub motion_glb: Option<String>,
    pub unresolved: bool,
    pub layer_index: usize,
    pub animation_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinTagData {
    pub name: String,
    pub group: Option<String>,
    pub priority: Option<i32>,
    pub sub_tag_definition: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinTagGroupData {
    pub name: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinFragmentBlendData {
    pub key: String,
    pub source_path: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub variant_count: usize,
    pub fragment_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinFragmentDefinitionData {
    pub name: String,
    pub scopes: Vec<String>,
    pub flags: Option<String>,
    pub overrides: Vec<EditorMannequinFragmentOverrideData>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinFragmentOverrideData {
    pub tags: Vec<String>,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinScopeContextData {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinScopeData {
    pub name: String,
    pub layer: i32,
    pub num_layers: i32,
    pub context: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorMannequinResolvedAnimationData {
    pub fragment_key: String,
    pub option_key: String,
    pub animation_ref: Option<String>,
    pub motion_glb: Option<String>,
    pub unresolved: bool,
    pub reason: Option<String>,
    pub required_tags: Vec<String>,
    pub excluded_tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorAnimationMotionData {
    pub asset_path: String,
    pub name: String,
    pub set_path: String,
    pub duration_millis: Option<u32>,
    pub channel_count: usize,
    pub joint_targets: Vec<String>,
    pub events: Vec<EditorAnimationEventData>,
    /// Asset-pipeline truth for this motion source: workspace entry status plus
    /// the latest job outcome (e.g. `current · job succeeded`), from the asset
    /// processor's workspace view. `None` only in legacy/test fixtures built
    /// without pipeline status.
    pub pipeline_status: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorAnimationEventData {
    pub name: String,
    pub animation: String,
    pub time_millis: u32,
    pub end_time_millis: u32,
    pub parameter: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorAnimationJointData {
    pub name: String,
    pub depth: u32,
    pub index: u32,
    pub parent: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorGpuStateData {
    NotRequested,
    Starting,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorViewportRenderStateData {
    Waiting,
    MetadataOnly,
    GpuSurfaceHandle,
    EditorCompositionSurface,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorRuntimeStateData {
    Unregistered,
    Stopped,
    Starting,
    Running,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeProjectionData {
    pub name: String,
    pub priority: i32,
    pub roles: Vec<String>,
    pub launch_profiles: Vec<String>,
}

/// Which camera drag is active over the viewport surface.
/// Whether viewport diagnostic overlays are enabled. Resolved once per
/// surface at construction instead of on every render frame.
fn viewport_diagnostic_enabled() -> bool {
    std::env::var("AZOTH_EDITOR_VIEWPORT_DIAGNOSTIC")
        .ok()
        .is_some_and(|value| diagnostic_flag_value(&value))
}

fn diagnostic_flag_value(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Where a left-button gesture over the viewport surface has got to.
///
/// `Down` becomes `Dragging` the first time movement exceeds the click
/// threshold and never goes back, so a mouse-up out of `Dragging` can never
/// be reported to the host as a click-pick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LeftGesture {
    #[default]
    Idle,
    Down,
    Dragging,
}

impl LeftGesture {
    /// Whether a press is in flight, so moves and the release belong to it.
    const fn is_active(self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// Whether releasing now is a click-pick rather than the end of a drag.
    const fn is_click(self) -> bool {
        matches!(self, Self::Down)
    }

    /// Advance the gesture for a move that did or did not clear the drag
    /// threshold. Only `Down` ever transitions; `Dragging` is terminal.
    const fn moved(self, exceeded_threshold: bool) -> Self {
        match self {
            Self::Down if exceeded_threshold => Self::Dragging,
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// The shared in-process viewport surface element.
///
/// A typed transparent composition hole above the Bevy sibling visual, plus
/// camera (orbit/pan/dolly) and pointer (pick / gizmo drag) input capture into
/// [`EditorViewportInputQueue`], publishing its geometry + [`owner`] tag to
/// [`EditorViewportPanelFrame`] every paint. Embedded by the scene
/// [`ViewportPanel`] and the Animation mode mannequin panel — the frame pump
/// in editor-core stays the single consumer; panels only differ in the owner
/// tag (which content the pump renders) and whether asset drops are accepted.
///
/// [`owner`]: ViewportSurfaceOwner
pub struct InProcessViewportSurface {
    owner: ViewportSurfaceOwner,
    /// Whether Asset Browser drags may be dropped onto this surface (scene
    /// placement only; the animation preview has no drop semantics).
    accept_asset_drops: bool,
    /// Whether diagnostic corner overlays render. Resolved once at
    /// construction; the process environment cannot change mid-session.
    diagnostic: bool,
    /// Active camera drag over the surface, if any.
    drag: Option<ViewportCameraDragKind>,
    /// Stable id for the current camera drag gesture.
    camera_drag_interaction_id: Option<u64>,
    /// Latest normalized absolute cursor retained across drag-out release.
    camera_drag_position: (f32, f32),
    /// Where the left button is in the PointerDown/PointerMove/PointerUp
    /// sequence the host uses for gizmo drags and click-picks.
    left_gesture: LeftGesture,
    /// Window-logical press origin used to classify click vs drag.
    left_down_position: Point<Pixels>,
    /// Stable id for the current left-button gesture.
    left_interaction_id: u64,
    /// Drag payload currently inside this surface, if any.
    asset_drag_interaction_id: Option<u64>,
    /// Surface bounds from the last prepaint, for input normalization.
    surface_bounds: Bounds<Pixels>,
}

impl InProcessViewportSurface {
    #[must_use]
    pub fn new(owner: ViewportSurfaceOwner, accept_asset_drops: bool) -> Self {
        Self {
            owner,
            accept_asset_drops,
            diagnostic: viewport_diagnostic_enabled(),
            drag: None,
            camera_drag_interaction_id: None,
            camera_drag_position: (0.0, 0.0),
            left_gesture: LeftGesture::Idle,
            left_down_position: Point::default(),
            left_interaction_id: 0,
            asset_drag_interaction_id: None,
            surface_bounds: Bounds::default(),
        }
    }

    fn normalized_surface_position(&self, position: Point<Pixels>) -> Option<(f32, f32)> {
        let width = f32::from(self.surface_bounds.size.width);
        let height = f32::from(self.surface_bounds.size.height);
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let x = (f32::from(position.x) - f32::from(self.surface_bounds.origin.x)) / width;
        let y = (f32::from(position.y) - f32::from(self.surface_bounds.origin.y)) / height;
        ((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)).then_some((x, y))
    }

    /// Normalized surface position clamped to `[0, 1]` so an in-progress drag or
    /// a drop that drifts a hair outside the surface still resolves. `None` only
    /// when the surface has no area.
    fn clamped_surface_position(&self, position: Point<Pixels>) -> Option<(f32, f32)> {
        let width = f32::from(self.surface_bounds.size.width);
        let height = f32::from(self.surface_bounds.size.height);
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let x = ((f32::from(position.x) - f32::from(self.surface_bounds.origin.x)) / width)
            .clamp(0.0, 1.0);
        let y = ((f32::from(position.y) - f32::from(self.surface_bounds.origin.y)) / height)
            .clamp(0.0, 1.0);
        Some((x, y))
    }

    /// Left press: arm the click/drag gesture and tell the host, which tries a
    /// gizmo grab first and falls back to a click-pick on mouse-up if the
    /// gesture stayed still.
    fn begin_left_pointer_gesture(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some((x, y)) = self.normalized_surface_position(event.position) else {
            return;
        };
        self.left_gesture = LeftGesture::Down;
        self.left_down_position = event.position;
        self.left_interaction_id = next_viewport_interaction_id();
        trace_viewport_input(ViewportUiInputTrace::PointerDown {
            interaction_id: self.left_interaction_id,
            x,
            y,
        });
        cx.default_global::<EditorViewportInputQueue>()
            .push(ViewportInputEvent::PointerDown {
                interaction_id: self.left_interaction_id,
                x,
                y,
            });
        window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
        cx.stop_propagation();
    }

    /// Right/middle press: open a camera drag of `kind`. Only the start and end
    /// are semantic events; the producer samples the cursor itself in between.
    fn begin_camera_drag(
        &mut self,
        kind: ViewportCameraDragKind,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some((x, y)) = self.normalized_surface_position(event.position) else {
            return;
        };
        let interaction_id = next_viewport_interaction_id();
        self.drag = Some(kind);
        self.camera_drag_interaction_id = Some(interaction_id);
        self.camera_drag_position = (x, y);
        cx.default_global::<EditorViewportInputQueue>()
            .push(ViewportInputEvent::CameraDragStart {
                interaction_id,
                kind,
                x,
                y,
                started_at: Instant::now(),
            });
        window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
        cx.stop_propagation();
    }

    /// Pointer move over the surface: retain a camera drag position, advance a
    /// left gesture, or publish a plain hover.
    fn on_surface_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // Camera drags are semantic start/end transitions only. The producer
        // samples GetCursorPos after its frame-latency wait, so GPUI pointer-
        // event cadence cannot stall or bunch orbit.
        if self.drag.is_some() {
            if let Some(position) = self.clamped_surface_position(event.position) {
                self.camera_drag_position = position;
            }
            cx.stop_propagation();
        } else if self.left_gesture.is_active() {
            let dx = f32::from(event.position.x) - f32::from(self.left_down_position.x);
            let dy = f32::from(event.position.y) - f32::from(self.left_down_position.y);
            self.left_gesture = self
                .left_gesture
                .moved(exceeds_click_drag_threshold(dx, dy));
            if let Some((x, y)) = self.clamped_surface_position(event.position) {
                cx.default_global::<EditorViewportInputQueue>().push(
                    ViewportInputEvent::PointerMove {
                        interaction_id: self.left_interaction_id,
                        x,
                        y,
                    },
                );
            }
            window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
        } else if let Some((x, y)) = self.normalized_surface_position(event.position) {
            cx.default_global::<EditorViewportInputQueue>()
                .push(ViewportInputEvent::HoverMove { x, y });
            window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
        }
    }

    /// The cursor left the surface: end the hover, and cancel any asset drag
    /// that was still inside it.
    // `on_hover` hands its listener the flag by reference; the signature is
    // GPUI's, not a choice this method makes.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn on_surface_hover_changed(
        &mut self,
        hovered: &bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if *hovered {
            return;
        }
        cx.default_global::<EditorViewportInputQueue>()
            .push(ViewportInputEvent::HoverLeave);
        if let Some(interaction_id) = self.asset_drag_interaction_id.take() {
            trace_viewport_input(ViewportUiInputTrace::AssetDragLeave { interaction_id });
            cx.default_global::<EditorViewportInputQueue>()
                .push(ViewportInputEvent::AssetDragLeave { interaction_id });
            cx.refresh_windows();
            window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
        }
    }

    /// Wheel over the surface: one 24px notch is one dolly step.
    // `cx.listener` takes this by function pointer, so the receiver is part of
    // the signature it must have even though a dolly needs no surface state.
    #[allow(clippy::unused_self)]
    fn on_surface_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let steps = f32::from(event.delta.pixel_delta(px(24.0)).y) / 24.0;
        // Not a nearness test: a wheel event that resolved to no whole notch
        // has nothing to publish, and the sentinel it is compared against is
        // the zero the division produced.
        #[allow(clippy::float_cmp)]
        let moved = steps != 0.0;
        if moved {
            cx.default_global::<EditorViewportInputQueue>()
                .push(ViewportInputEvent::Dolly { steps });
            window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
        }
    }

    /// An Asset Browser drag moved over the surface; the enter edge publishes
    /// the payload once so the host can show a placement preview.
    fn on_asset_drag_move(
        &mut self,
        event: &DragMoveEvent<ViewportAssetDrag>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some((x, y)) = self.normalized_surface_position(event.event.position) else {
            return;
        };
        let (interaction_id, entering, source_path) = {
            let drag = event.drag(cx);
            let entering = self.asset_drag_interaction_id != Some(drag.interaction_id);
            (
                drag.interaction_id,
                entering,
                entering.then(|| drag.source_path.clone()),
            )
        };
        trace_viewport_input(ViewportUiInputTrace::AssetDragMove {
            interaction_id,
            x,
            y,
        });
        let queue = cx.default_global::<EditorViewportInputQueue>();
        if entering {
            self.asset_drag_interaction_id = Some(interaction_id);
            queue.push(ViewportInputEvent::AssetDragEnter {
                interaction_id,
                source_path: source_path.unwrap_or_default(),
                x,
                y,
            });
            window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
        }
    }

    /// An Asset Browser drag was released over the surface: place the asset at
    /// the cursor's normalized surface coordinate.
    fn on_asset_drop(
        &mut self,
        drag: &ViewportAssetDrag,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some((x, y)) = self.clamped_surface_position(window.mouse_position()) else {
            return;
        };
        let source_path = drag.source_path.clone();
        trace_viewport_input(ViewportUiInputTrace::AssetDrop {
            interaction_id: drag.interaction_id,
            x,
            y,
        });
        cx.default_global::<EditorViewportInputQueue>()
            .push(ViewportInputEvent::DropAsset {
                interaction_id: drag.interaction_id,
                source_path,
                x,
                y,
            });
        self.asset_drag_interaction_id = None;
        cx.refresh_windows();
        window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
        cx.stop_propagation();
    }

    fn finish_left_pointer_gesture(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if event.button != MouseButton::Left || !self.left_gesture.is_active() {
            return;
        }

        let dx = f32::from(event.position.x) - f32::from(self.left_down_position.x);
        let dy = f32::from(event.position.y) - f32::from(self.left_down_position.y);
        let finished = self
            .left_gesture
            .moved(exceeds_click_drag_threshold(dx, dy));
        self.left_gesture = LeftGesture::Idle;
        if let Some((x, y)) = self.clamped_surface_position(event.position) {
            cx.default_global::<EditorViewportInputQueue>()
                .push(ViewportInputEvent::PointerUp {
                    interaction_id: self.left_interaction_id,
                    x,
                    y,
                    is_click: finished.is_click(),
                });
            cx.refresh_windows();
            window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
        }
    }

    fn finish_camera_drag(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let matches = matches!(
            (event.button, self.drag),
            (MouseButton::Right, Some(ViewportCameraDragKind::Orbit))
                | (MouseButton::Middle, Some(ViewportCameraDragKind::Pan))
        );
        if !matches {
            return;
        }
        self.drag = None;
        if let Some(interaction_id) = self.camera_drag_interaction_id.take() {
            if let Some(position) = self.clamped_surface_position(event.position) {
                self.camera_drag_position = position;
            }
            let (x, y) = self.camera_drag_position;
            cx.default_global::<EditorViewportInputQueue>().push(
                ViewportInputEvent::CameraDragEnd {
                    interaction_id,
                    x,
                    y,
                    ended_at: Instant::now(),
                },
            );
            window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
        }
        cx.stop_propagation();
    }
}

impl Render for InProcessViewportSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        // The dedicated viewport producer posts every required cached-scene
        // Present. This view must not request a parallel GPUI animation loop:
        // doing so invalidates the editor root at viewport cadence.
        let owner = self.owner;
        let accept_asset_drops = self.accept_asset_drops;
        let diagnostic = self.diagnostic;
        let composition_generation = next_composition_hole_generation();
        let entity = cx.entity();

        div()
            .id("viewport-inprocess-surface")
            .size_full()
            .overflow_hidden()
            .child(
                gpui::composition_hole(PRIMARY_VIEWPORT_VISUAL_SLOT_ID, composition_generation)
                    .absolute()
                    .size_full(),
            )
            .when(diagnostic, |surface| {
                surface.child(viewport_diagnostic_layout_corners())
            })
            // Publish the surface geometry + owner tag every paint so the
            // viewport host knows the surface is visible, how large the render
            // target must be, and which mode's content to render.
            .on_prepaint(move |bounds, window, cx| {
                let scale_factor = window.scale_factor();
                entity.update(cx, |this, _| {
                    this.surface_bounds = bounds;
                });
                let window_id = window.window_handle().window_id().as_u64();
                cx.default_global::<EditorViewportPanelFrame>().publish(
                    bounds,
                    scale_factor,
                    window_id,
                    owner,
                );
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(Self::begin_left_pointer_gesture),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.begin_camera_drag(ViewportCameraDragKind::Orbit, event, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.begin_camera_drag(ViewportCameraDragKind::Pan, event, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(Self::on_surface_mouse_move))
            .on_hover(cx.listener(Self::on_surface_hover_changed))
            .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, window, cx| {
                this.finish_left_pointer_gesture(event, window, cx);
            }))
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.finish_camera_drag(event, window, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.finish_camera_drag(event, window, cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.finish_camera_drag(event, window, cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.finish_camera_drag(event, window, cx);
                }),
            )
            .on_scroll_wheel(cx.listener(Self::on_surface_scroll_wheel))
            // Asset Browser rows publish a `ViewportAssetDrag`; on drop we place
            // the asset at the cursor's normalized surface coordinate (scene
            // surfaces only — the animation preview has no drop semantics).
            .when(accept_asset_drops, |this| {
                this.drag_over::<ViewportAssetDrag>(|style, _, _, _| style)
                    .on_drag_move(cx.listener(Self::on_asset_drag_move))
                    .on_drop(cx.listener(Self::on_asset_drop))
            })
    }
}

fn viewport_diagnostic_layout_corners() -> gpui::Div {
    const CORNER_RGBA: u32 = 0x39FF_14FF;
    let marker = || {
        div()
            .absolute()
            .w(px(5.0))
            .h(px(5.0))
            .bg(gpui::rgba(CORNER_RGBA))
    };
    div()
        .absolute()
        .size_full()
        .child(marker().top_0().left_0())
        .child(marker().top_0().right_0())
        .child(marker().bottom_0().left_0())
        .child(marker().bottom_0().right_0())
}

/// Viewport panel
///
/// Hosts the in-process editor viewport surface and its chrome.
pub struct ViewportPanel {
    focus: FocusHandle,
    texture: SharedViewportTexture,
    last_generation: u64,
    /// The shared composition-hole surface element (scene-owned, accepts drops).
    surface: gpui::Entity<InProcessViewportSurface>,
    camera_view: ViewportCameraView,
    shading_mode: ViewportShadingMode,
    visibility: ViewportVisibilitySettings,
    open_pill_menu: Option<ViewportPillMenu>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewportPillMenu {
    Camera,
    Shading,
    Show,
    Gizmos,
}

#[derive(Clone, Copy)]
enum ViewportVisibilityToggle {
    Grid,
    StatsOverlay,
    Bounds,
    Skybox,
    BoundingBoxes,
}

impl ViewportPanel {
    pub const NAME: &'static str = "viewport";

    pub fn init(cx: &mut Context<'_, Self>) -> Self {
        // Retrieve the shared texture from global state
        // This assumes SharedViewportTexture has been initialized and set as global in the app
        let texture = cx.global::<SharedViewportTexture>().clone();

        Self {
            focus: cx.focus_handle(),
            texture,
            last_generation: 0,
            surface: cx.new(|_| InProcessViewportSurface::new(ViewportSurfaceOwner::Scene, true)),
            camera_view: ViewportCameraView::default(),
            shading_mode: ViewportShadingMode::default(),
            visibility: ViewportVisibilitySettings::default(),
            open_pill_menu: None,
        }
    }

    fn toggle_visibility(
        &mut self,
        toggle: ViewportVisibilityToggle,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match toggle {
            ViewportVisibilityToggle::Grid => self.visibility.grid = !self.visibility.grid,
            ViewportVisibilityToggle::StatsOverlay => {
                self.visibility.stats_overlay = !self.visibility.stats_overlay;
            }
            ViewportVisibilityToggle::Bounds => self.visibility.bounds = !self.visibility.bounds,
            ViewportVisibilityToggle::Skybox => self.visibility.skybox = !self.visibility.skybox,
            ViewportVisibilityToggle::BoundingBoxes => {
                self.visibility.bounding_boxes = !self.visibility.bounding_boxes;
            }
        }
        cx.default_global::<EditorViewportInputQueue>()
            .push(ViewportInputEvent::SetVisibility {
                settings: self.visibility,
            });
        window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
        cx.notify();
    }
}

/// Content shown while the in-process composition surface is not the live
/// renderer: the failure state with its diagnostic, or the starting state the
/// runtime-host stream shows before its first frame.
fn render_viewport_placeholder(
    state: EditorViewportRenderStateData,
    render_status: Option<&EditorViewportRenderStatus>,
    has_frame: bool,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    // The in-process renderer could not start: explicit unavailable state
    // (diagnostic shown by the metadata overlay).
    if state == EditorViewportRenderStateData::Failed {
        let render_failure = render_status
            .and_then(|status| status.diagnostic.clone())
            .unwrap_or_else(|| "In-process viewport renderer unavailable".to_owned());
        return div()
            .size_full()
            .bg(theme.muted)
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(render_failure),
            )
            .into_any_element();
    }
    // Runtime-host stream states (play-in-standalone) and startup.
    if has_frame {
        return div().size_full().bg(theme.muted).into_any_element();
    }
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Viewport renderer starting..."),
        )
        .into_any_element()
}

/// The always-on overlays stacked over the viewport content, in paint order:
/// runtime status, the navigation gizmo cluster, and the orientation triad.
fn render_viewport_overlays(
    runtime_status: Option<&EditorRuntimeStatus>,
    scene_tools: &EditorSceneToolState,
    camera: EditorViewportCameraState,
    theme: &gpui_component::theme::Theme,
) -> [gpui::AnyElement; 3] {
    [
        render_runtime_status(runtime_status, theme).into_any_element(),
        render_viewport_nav_overlay(scene_tools, camera, theme).into_any_element(),
        render_viewport_orientation_triad(camera, theme).into_any_element(),
    ]
}

impl Render for ViewportPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        // Unlike the data panels, the viewport does not gate on project-host
        // connectivity: the in-process renderer draws the editor
        // world locally, and authored scene content streams in once the
        // session attaches. Session state is shown by the status overlays.
        let theme = cx.theme().clone();
        let background = theme.background;
        let texture = self.texture.clone();
        let runtime_status = cx.try_global::<EditorRuntimeStatus>().cloned();
        let render_status = cx.try_global::<EditorViewportRenderStatus>().cloned();

        // Get current texture state
        let texture_state = texture.get();
        // Update generation tracking
        if let Some(frame) = texture_state.as_ref() {
            self.last_generation = frame.generation;
        }

        let scene_tools = cx
            .try_global::<EditorSceneToolState>()
            .cloned()
            .unwrap_or_default();
        let camera = cx
            .try_global::<EditorViewportCameraState>()
            .copied()
            .unwrap_or_default();

        let render_state = render_status
            .as_ref()
            .map_or(EditorViewportRenderStateData::Waiting, |status| {
                status.state
            });
        let content = match render_state {
            // The in-process renderer is live: paint the transparent hole above
            // the Bevy-owned DirectComposition sibling visual.
            EditorViewportRenderStateData::EditorCompositionSurface => {
                self.surface.clone().into_any_element()
            }
            other => render_viewport_placeholder(
                other,
                render_status.as_ref(),
                texture_state.is_some(),
                &theme,
            ),
        };

        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(background)
            .child(render_viewport_tab_strip(&theme))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .overflow_hidden()
                    // Publish the content-area geometry every paint (in every
                    // state) so the viewport host knows the surface size and
                    // that the panel is visible before the surface element
                    // first renders (the surface publishes again once live).
                    .on_prepaint(move |bounds, window, cx| {
                        let scale_factor = window.scale_factor();
                        let window_id = window.window_handle().window_id().as_u64();
                        cx.default_global::<EditorViewportPanelFrame>().publish(
                            bounds,
                            scale_factor,
                            window_id,
                            ViewportSurfaceOwner::Scene,
                        );
                    })
                    .child(content)
                    .child(render_viewport_pills(
                        self.camera_view,
                        self.shading_mode,
                        self.visibility,
                        self.open_pill_menu,
                        &theme,
                        cx,
                    ))
                    .child(render_viewport_controls(&theme))
                    .when(self.visibility.stats_overlay, |this| {
                        this.child(render_viewport_stats(
                            render_status
                                .as_ref()
                                .and_then(|status| status.telemetry.as_ref()),
                            &theme,
                        ))
                    })
                    .children(render_viewport_overlays(
                        runtime_status.as_ref(),
                        &scene_tools,
                        camera,
                        &theme,
                    )),
            )
            .into_any_element()
    }
}

fn render_viewport_tab_strip(theme: &Theme) -> impl IntoElement {
    h_flex()
        .h(px(31.0))
        .flex_none()
        .items_center()
        .bg(theme.tab_bar)
        .border_b_1()
        .border_color(theme.border)
        .pl(px(2.0))
        .child(
            h_flex()
                .h(px(30.0))
                .items_center()
                .gap_1()
                .px_2()
                .border_r_1()
                .border_color(theme.border)
                .bg(theme.background)
                .text_size(px(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.foreground)
                .child(Icon::new(IconName::LayoutDashboard).with_size(px(15.0)))
                .child("Viewport")
                .child(
                    Icon::new(IconName::Close)
                        .with_size(px(14.0))
                        .text_color(theme.muted_foreground),
                ),
        )
        .child(
            div()
                .ml_auto()
                .h_full()
                .flex()
                .items_center()
                .gap(px(1.0))
                .pr(px(5.0))
                .child(viewport_chrome_icon(IconName::LayoutDashboard, theme))
                .child(viewport_chrome_icon(IconName::Maximize, theme)),
        )
}

fn render_viewport_pills(
    camera_view: ViewportCameraView,
    shading_mode: ViewportShadingMode,
    visibility: ViewportVisibilitySettings,
    open_menu: Option<ViewportPillMenu>,
    theme: &Theme,
    cx: &Context<'_, ViewportPanel>,
) -> impl IntoElement {
    h_flex()
        .absolute()
        .top(px(9.0))
        .left(px(9.0))
        .gap_1()
        .child(
            viewport_pill(
                "viewport-camera-pill",
                IconName::GalleryVerticalEnd,
                camera_view.label(),
                open_menu == Some(ViewportPillMenu::Camera),
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_pill_menu = if this.open_pill_menu == Some(ViewportPillMenu::Camera) {
                    None
                } else {
                    Some(ViewportPillMenu::Camera)
                };
                cx.notify();
            }))
            .when(open_menu == Some(ViewportPillMenu::Camera), |this| {
                this.child(render_camera_view_menu(camera_view, theme, cx))
            }),
        )
        .child(
            viewport_pill(
                "viewport-shading-pill",
                IconName::Sun,
                shading_mode.label(),
                open_menu == Some(ViewportPillMenu::Shading),
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_pill_menu = if this.open_pill_menu == Some(ViewportPillMenu::Shading) {
                    None
                } else {
                    Some(ViewportPillMenu::Shading)
                };
                cx.notify();
            }))
            .when(open_menu == Some(ViewportPillMenu::Shading), |this| {
                this.child(render_shading_menu(shading_mode, theme, cx))
            }),
        )
        .child(
            viewport_pill(
                "viewport-show-pill",
                IconName::Eye,
                "Show",
                open_menu == Some(ViewportPillMenu::Show),
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_pill_menu = if this.open_pill_menu == Some(ViewportPillMenu::Show) {
                    None
                } else {
                    Some(ViewportPillMenu::Show)
                };
                cx.notify();
            }))
            .when(open_menu == Some(ViewportPillMenu::Show), |this| {
                this.child(render_show_menu(visibility, theme, cx))
            }),
        )
        .child(
            viewport_pill(
                "viewport-gizmos-pill",
                IconName::Boxes,
                "Gizmos",
                open_menu == Some(ViewportPillMenu::Gizmos),
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_pill_menu = if this.open_pill_menu == Some(ViewportPillMenu::Gizmos) {
                    None
                } else {
                    Some(ViewportPillMenu::Gizmos)
                };
                cx.notify();
            }))
            .when(open_menu == Some(ViewportPillMenu::Gizmos), |this| {
                this.child(render_gizmos_menu(visibility, theme, cx))
            }),
        )
}

fn viewport_pill(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    open: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    h_flex()
        .id(id)
        .relative()
        .h(px(25.0))
        .items_center()
        .gap_1()
        .px_2()
        .rounded(px(5.0))
        .border_1()
        .border_color(if open { theme.accent } else { theme.border })
        .bg(theme.background.opacity(0.82))
        .text_size(px(11.0))
        .text_color(theme.foreground)
        .child(Icon::new(icon).with_size(px(14.0)))
        .child(label)
        .child(
            Icon::new(IconName::ChevronDown)
                .with_size(px(14.0))
                .text_color(theme.muted_foreground),
        )
}

fn render_camera_view_menu(
    active: ViewportCameraView,
    theme: &Theme,
    cx: &Context<'_, ViewportPanel>,
) -> impl IntoElement {
    viewport_pill_menu(theme).children(ViewportCameraView::ALL.into_iter().map(|view| {
        viewport_pill_menu_item(view.label(), view == active, theme).on_click(cx.listener(
            move |this, _, window, cx| {
                this.camera_view = view;
                this.open_pill_menu = None;
                cx.default_global::<EditorViewportInputQueue>()
                    .push(ViewportInputEvent::SetCameraView { view });
                window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
                cx.notify();
                cx.stop_propagation();
            },
        ))
    }))
}

fn render_shading_menu(
    active: ViewportShadingMode,
    theme: &Theme,
    cx: &Context<'_, ViewportPanel>,
) -> impl IntoElement {
    viewport_pill_menu(theme).children(ViewportShadingMode::ALL.into_iter().map(|mode| {
        viewport_pill_menu_item(mode.label(), mode == active, theme).on_click(cx.listener(
            move |this, _, window, cx| {
                this.shading_mode = mode;
                this.open_pill_menu = None;
                cx.default_global::<EditorViewportInputQueue>()
                    .push(ViewportInputEvent::SetShadingMode { mode });
                window.dispatch_action(Box::new(crate::actions::PumpViewportInput), cx);
                cx.notify();
                cx.stop_propagation();
            },
        ))
    }))
}

fn render_show_menu(
    visibility: ViewportVisibilitySettings,
    theme: &Theme,
    cx: &Context<'_, ViewportPanel>,
) -> impl IntoElement {
    viewport_pill_menu(theme)
        .child(
            viewport_multi_menu_item(
                "show-grid",
                "Grid",
                IconName::GridView,
                visibility.grid,
                true,
                theme,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_visibility(ViewportVisibilityToggle::Grid, window, cx);
                cx.stop_propagation();
            })),
        )
        .child(
            viewport_multi_menu_item(
                "show-stats",
                "Stats Overlay",
                IconName::Gauge,
                visibility.stats_overlay,
                true,
                theme,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_visibility(ViewportVisibilityToggle::StatsOverlay, window, cx);
                cx.stop_propagation();
            })),
        )
        .child(
            viewport_multi_menu_item(
                "show-bounds",
                "Bounds",
                IconName::Scan,
                visibility.bounds,
                true,
                theme,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_visibility(ViewportVisibilityToggle::Bounds, window, cx);
                cx.stop_propagation();
            })),
        )
        .child(
            viewport_multi_menu_item(
                "show-skybox",
                "Skybox",
                IconName::Cloud,
                visibility.skybox,
                true,
                theme,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_visibility(ViewportVisibilityToggle::Skybox, window, cx);
                cx.stop_propagation();
            })),
        )
}

fn render_gizmos_menu(
    visibility: ViewportVisibilitySettings,
    theme: &Theme,
    cx: &Context<'_, ViewportPanel>,
) -> impl IntoElement {
    viewport_pill_menu(theme)
        .child(viewport_multi_menu_item(
            "gizmos-lights",
            "Light Icons",
            IconName::Lightbulb,
            false,
            false,
            theme,
        ))
        .child(viewport_multi_menu_item(
            "gizmos-cameras",
            "Camera Icons",
            IconName::Video,
            false,
            false,
            theme,
        ))
        .child(viewport_multi_menu_item(
            "gizmos-colliders",
            "Colliders",
            IconName::Box,
            false,
            false,
            theme,
        ))
        .child(
            viewport_multi_menu_item(
                "gizmos-bounds",
                "Bounding Boxes",
                IconName::Scan,
                visibility.bounding_boxes,
                true,
                theme,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_visibility(ViewportVisibilityToggle::BoundingBoxes, window, cx);
                cx.stop_propagation();
            })),
        )
}

fn viewport_multi_menu_item(
    id: &'static str,
    label: &'static str,
    icon: IconName,
    active: bool,
    enabled: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    h_flex()
        .id(id)
        .h(px(27.0))
        .items_center()
        .gap_2()
        .px_2()
        .rounded(px(4.0))
        .text_size(px(11.0))
        .text_color(if enabled {
            theme.foreground
        } else {
            theme.muted_foreground
        })
        .opacity(if enabled { 1.0 } else { 0.45 })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
        })
        .child(Icon::new(icon).with_size(px(14.0)))
        .child(div().flex_1().child(label))
        .child(
            Icon::new(if active {
                IconName::Check
            } else {
                IconName::Minus
            })
            .with_size(px(13.0))
            .text_color(if active {
                theme.accent
            } else {
                theme.muted_foreground
            }),
        )
}

fn viewport_pill_menu(theme: &Theme) -> gpui::Div {
    v_flex()
        .absolute()
        .top(px(29.0))
        .left_0()
        .w(px(150.0))
        .p_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
}

fn viewport_pill_menu_item(
    label: &'static str,
    active: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    h_flex()
        .id(gpui::SharedString::from(format!(
            "viewport-pill-item-{label}"
        )))
        .h(px(25.0))
        .items_center()
        .px_2()
        .rounded(px(4.0))
        .text_size(px(11.0))
        .text_color(if active {
            theme.accent
        } else {
            theme.foreground
        })
        .hover(|this| this.bg(theme.secondary_hover))
        .cursor_pointer()
        .child(div().flex_1().child(label))
        .when(active, |this| {
            this.child(Icon::new(IconName::Check).with_size(px(14.0)))
        })
}

fn render_viewport_stats(
    telemetry: Option<&EditorViewportTelemetryData>,
    theme: &Theme,
) -> impl IntoElement {
    let fps = telemetry.and_then(|data| data.fps);
    let frame_time = telemetry.and_then(|data| data.frame_time_us);
    let triangles = telemetry.and_then(|data| data.triangles);
    let draws = telemetry.and_then(|data| data.draw_calls);
    let vertices = telemetry.and_then(|data| data.vertices);

    v_flex()
        .absolute()
        .top(px(9.0))
        .right(px(9.0))
        .w(px(142.0))
        .gap(px(3.0))
        .p_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.background.opacity(0.84))
        .font_family("monospace")
        .text_size(px(10.5))
        .child(viewport_stat_row(
            "FPS",
            fps.map_or_else(|| "—".to_string(), |value| value.to_string()),
            fps.map_or(theme.muted_foreground, |_| theme.success),
            theme,
        ))
        .child(viewport_stat_row(
            "Frame ms",
            frame_time.map_or_else(|| "—".to_string(), format_millis_from_micros),
            theme.foreground,
            theme,
        ))
        .child(viewport_stat_row(
            "Tris",
            format_stat_count(triangles),
            theme.foreground,
            theme,
        ))
        .child(viewport_stat_row(
            "Draws",
            format_stat_count(draws),
            theme.foreground,
            theme,
        ))
        .child(viewport_stat_row(
            "Verts",
            format_stat_count(vertices),
            theme.foreground,
            theme,
        ))
}

fn viewport_stat_row(
    label: &'static str,
    value: String,
    value_color: gpui::Hsla,
    theme: &Theme,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .child(
            div()
                .flex_1()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(div().text_color(value_color).child(value))
}

/// Widen a counter for a display label rounded to one decimal place.
///
/// `f64` holds every integer below 2^53 exactly. Every caller passes a
/// per-frame tally or a byte total — draw calls, triangles, texture memory —
/// and nine quadrillion of any of those is not a number this editor can
/// reach, let alone one whose tenths digit anybody reads.
const fn approximate(value: u64) -> f64 {
    // Rust has no lossless u64 -> f64 conversion; the bound above keeps every
    // caller inside the exactly-representable range.
    #[allow(clippy::cast_precision_loss)]
    {
        value as f64
    }
}

fn format_stat_count(value: Option<u64>) -> String {
    let Some(value) = value else {
        return "—".to_string();
    };
    if value >= 1_000_000 {
        format!("{:.1}M", approximate(value) / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", approximate(value) / 1_000.0)
    } else {
        value.to_string()
    }
}

fn render_viewport_nav_overlay(
    tools: &EditorSceneToolState,
    camera: EditorViewportCameraState,
    theme: &Theme,
) -> impl IntoElement {
    h_flex()
        .absolute()
        .bottom(px(9.0))
        .left(px(9.0))
        .items_center()
        .gap_1p5()
        .font_family("monospace")
        .text_size(px(10.5))
        .text_color(theme.muted_foreground)
        .child(viewport_nav_badge(
            IconName::Eye,
            format!("Cam Speed {:.1}", camera.speed),
            theme,
        ))
        .child(viewport_nav_badge(
            IconName::Move,
            tools.tool.label().to_string(),
            theme,
        ))
}

fn viewport_nav_badge(icon: IconName, label: String, theme: &Theme) -> impl IntoElement {
    h_flex()
        .h(px(23.0))
        .items_center()
        .gap_1()
        .px_2()
        .rounded(px(5.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.background.opacity(0.82))
        .child(Icon::new(icon).with_size(px(14.0)))
        .child(label)
}

fn render_viewport_orientation_triad(
    camera: EditorViewportCameraState,
    theme: &Theme,
) -> impl IntoElement {
    // Project the world axes through the live camera pose; 54px box, labels
    // orbit the center dot at a fixed radius.
    const CENTER: f32 = 24.0;
    const RADIUS: f32 = 20.0;
    let [x_axis, y_axis, z_axis] = triad_axis_directions(camera.yaw_radians, camera.pitch_radians);
    let place = |direction: (f32, f32)| {
        (
            px(direction.0.mul_add(RADIUS, CENTER)),
            px(direction.1.mul_add(RADIUS, CENTER) - 3.0),
        )
    };
    let (x_left, x_top) = place(x_axis);
    let (y_left, y_top) = place(y_axis);
    let (z_left, z_top) = place(z_axis);
    div()
        .absolute()
        .bottom(px(14.0))
        .right(px(18.0))
        .size(px(54.0))
        .child(axis_label("X", x_left, x_top, theme.danger))
        .child(axis_label("Y", y_left, y_top, theme.success))
        .child(axis_label("Z", z_left, z_top, theme.info))
        .child(
            div()
                .absolute()
                .left(px(24.0))
                .top(px(24.0))
                .size(px(6.0))
                .rounded_full()
                .bg(theme.foreground),
        )
}

fn axis_label(
    label: &'static str,
    left: gpui::Pixels,
    top: gpui::Pixels,
    color: gpui::Hsla,
) -> impl IntoElement {
    div()
        .absolute()
        .left(left)
        .top(top)
        .font_family("monospace")
        .text_size(px(10.0))
        .text_color(color)
        .child(label)
}

fn viewport_chrome_icon(icon: IconName, theme: &Theme) -> impl IntoElement {
    div()
        .size(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .text_color(theme.muted_foreground)
        .hover(|this| this.bg(theme.secondary_hover).text_color(theme.foreground))
        .child(Icon::new(icon).with_size(px(16.0)))
}

fn render_viewport_controls(theme: &Theme) -> impl IntoElement {
    kit::viewport_overlay(theme)
        .flex_row()
        .items_center()
        .gap_0p5()
        .py_0p5()
        .px_0p5()
        .absolute()
        .top_2()
        .right_2()
        .child(viewport_action_button(
            "viewport-frame-selected",
            IconName::Maximize,
            "Frame selected (real mesh bounds)",
            theme.foreground,
            crate::actions::FrameSelected,
            theme,
        ))
        .child(viewport_action_button(
            "viewport-launch-editor-world",
            IconName::Play,
            "Launch editor world",
            theme.success,
            crate::actions::LaunchEditorWorld,
            theme,
        ))
        .child(viewport_action_button(
            "viewport-stop-editor-world",
            IconName::Stop,
            "Stop editor world",
            theme.danger,
            crate::actions::StopEditorWorld { preserve: false },
            theme,
        ))
}

fn render_runtime_status(status: Option<&EditorRuntimeStatus>, theme: &Theme) -> impl IntoElement {
    let Some(status) = status.filter(|status| {
        matches!(
            status.state,
            EditorRuntimeStateData::Failed | EditorRuntimeStateData::Unregistered
        )
    }) else {
        return div().into_any_element();
    };
    let detail_labels = runtime_status_detail_labels(status);

    kit::viewport_overlay(theme)
        .absolute()
        .right_2()
        .bottom_2()
        .max_w_96()
        .text_color(status.state.tone().color(theme))
        .children([runtime_status_summary_label(status)])
        .children(detail_labels.into_iter().map(|label| div().child(label)))
        .into_any_element()
}

#[cfg(test)]
fn viewport_metadata_labels(
    frame: Option<&ViewportFrameState>,
    gpu: Option<&EditorGpuStatus>,
    render_status: Option<&EditorViewportRenderStatus>,
    dimensions: (u32, u32),
) -> Vec<String> {
    let mut labels = vec![viewport_frame_metadata_label(frame, dimensions)];
    if let Some(render_status) = render_status {
        labels.extend(viewport_render_status_labels(render_status));
    }
    if let Some(gpu) = gpu {
        labels.extend(gpu_status_labels(gpu));
    }
    labels
}

#[cfg(test)]
fn viewport_frame_metadata_label(
    frame: Option<&ViewportFrameState>,
    (width, height): (u32, u32),
) -> String {
    let Some(frame) = frame else {
        return format!("Viewport: {width}x{height} waiting");
    };
    let source_label = frame.source.as_ref().map_or_else(
        || "metadata only".to_string(),
        |source| {
            format!(
                "{} / {} / {} bytes",
                source.kind, source.pixel_format, source.byte_length
            )
        },
    );
    let payload_label = frame
        .source
        .as_ref()
        .map_or("metadata", |source| source.kind.as_str());

    format!(
        "Viewport: {}x{} gen {} / {} / {}",
        frame.width, frame.height, frame.generation, payload_label, source_label
    )
}

#[cfg(test)]
fn gpu_status_labels(status: &EditorGpuStatus) -> Vec<String> {
    let mut labels = vec![gpu_status_summary_label(status)];
    if let Some(adapter) = status.adapter_name.as_deref() {
        labels.push(format!("adapter {adapter}"));
    }
    if let Some(driver) = status.driver.as_deref() {
        labels.push(format!("driver {driver}"));
    }
    if let Some(diagnostic) = status.diagnostic.as_deref() {
        labels.push(format!("diag {diagnostic}"));
    }
    labels
}

#[cfg(test)]
fn viewport_render_status_labels(status: &EditorViewportRenderStatus) -> Vec<String> {
    let mut labels = vec![viewport_render_status_summary_label(status)];
    if let Some(telemetry) = status.telemetry.as_ref() {
        labels.extend(viewport_telemetry_labels(telemetry));
    }
    if let Some(diagnostic) = status.diagnostic.as_deref() {
        labels.push(format!("render diag {diagnostic}"));
    }
    labels
}

#[cfg(test)]
fn viewport_render_status_summary_label(status: &EditorViewportRenderStatus) -> String {
    let mut label = format!(
        "Viewport render: {}",
        viewport_render_state_label(status.state)
    );
    if let (Some(width), Some(height), Some(generation)) =
        (status.width, status.height, status.generation)
    {
        let _ = write!(label, " / {width}x{height} gen {generation}");
    }
    if let Some(format) = status.format.as_deref() {
        label.push_str(" / ");
        label.push_str(format);
    }
    if let Some(backend) = status.backend.as_deref() {
        label.push_str(" / ");
        label.push_str(backend);
    }
    label
}

#[cfg(test)]
fn viewport_telemetry_labels(telemetry: &EditorViewportTelemetryData) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(fps) = telemetry.fps {
        labels.push(format!("{fps} fps"));
    }
    if let Some(frame_time_us) = telemetry.frame_time_us {
        labels.push(format!("{} ms", format_millis_from_micros(frame_time_us)));
    }
    if let Some(draw_calls) = telemetry.draw_calls {
        labels.push(format!("{draw_calls} draw calls"));
    }
    if let Some(triangles) = telemetry.triangles {
        labels.push(format!("{triangles} triangles"));
    }
    if let Some(vertices) = telemetry.vertices {
        labels.push(format!("{vertices} vertices"));
    }
    if let Some(gpu_memory_bytes) = telemetry.gpu_memory_bytes {
        labels.push(format!("{} GPU memory", format_bytes(gpu_memory_bytes)));
    }
    labels
}

fn format_millis_from_micros(micros: u32) -> String {
    let whole = micros / 1_000;
    let fraction = micros % 1_000;
    format!("{whole}.{fraction:03}")
}

#[cfg(test)]
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", approximate(bytes) / approximate(GIB))
    } else if bytes >= MIB {
        format!("{:.1} MiB", approximate(bytes) / approximate(MIB))
    } else if bytes >= KIB {
        format!("{:.1} KiB", approximate(bytes) / approximate(KIB))
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
fn gpu_status_summary_label(status: &EditorGpuStatus) -> String {
    let mut label = format!("GPU: {}", gpu_state_label(status.state));
    if let Some(backend) = status.backend.as_deref() {
        label.push_str(" / ");
        label.push_str(backend);
    }
    if let Some(device_type) = status.device_type.as_deref() {
        label.push_str(" / ");
        label.push_str(device_type);
    }
    label
}

#[cfg(test)]
const fn viewport_render_state_label(state: EditorViewportRenderStateData) -> &'static str {
    match state {
        EditorViewportRenderStateData::Waiting => "waiting",
        EditorViewportRenderStateData::MetadataOnly => "metadata",
        EditorViewportRenderStateData::GpuSurfaceHandle => "gpu surface handle",
        EditorViewportRenderStateData::EditorCompositionSurface => "editor composition surface",
        EditorViewportRenderStateData::Failed => "failed",
    }
}

fn runtime_status_summary_label(status: &EditorRuntimeStatus) -> String {
    format!("Runtime: {} / {}", status.runtime_id, status.state.label())
}

fn runtime_status_detail_labels(status: &EditorRuntimeStatus) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(role) = status.role.as_deref() {
        labels.push(format!("role {role}"));
    }
    if let Some(authored_revision) = status.authored_revision {
        labels.push(format!("authored rev {authored_revision}"));
    }
    if let Some(project_id) = status.project_id.as_deref() {
        labels.push(format!("project {project_id}"));
    }
    if let Some(session_slug) = status.session_slug.as_deref() {
        labels.push(format!("session {session_slug}"));
    }
    labels.extend(runtime_status_diagnostic_labels(status));
    labels
}

#[cfg(test)]
fn runtime_projection_catalog_labels(catalog: &EditorRuntimeProjectionCatalog) -> Vec<String> {
    if catalog.projections.is_empty() {
        return vec!["none".to_string()];
    }

    catalog
        .projections
        .iter()
        .map(runtime_projection_label)
        .collect()
}

fn runtime_status_diagnostic_labels(
    status: &EditorRuntimeStatus,
) -> impl Iterator<Item = String> + '_ {
    status
        .diagnostics
        .iter()
        .map(|diagnostic| format!("diag {diagnostic}"))
}

#[cfg(test)]
const fn gpu_state_label(state: EditorGpuStateData) -> &'static str {
    match state {
        EditorGpuStateData::NotRequested => "not requested",
        EditorGpuStateData::Starting => "starting",
        EditorGpuStateData::Ready => "ready",
        EditorGpuStateData::Failed => "failed",
    }
}

#[cfg(test)]
fn runtime_projection_label(projection: &RuntimeProjectionData) -> String {
    format!(
        "{} p{} [{}] ({})",
        projection.name,
        projection.priority,
        runtime_projection_roles_label(&projection.roles),
        runtime_projection_profiles_label(&projection.launch_profiles)
    )
}

#[cfg(test)]
fn runtime_projection_roles_label(roles: &[String]) -> String {
    if roles.is_empty() {
        "all roles".to_string()
    } else {
        roles.join(", ")
    }
}

#[cfg(test)]
fn runtime_projection_profiles_label(profiles: &[String]) -> String {
    if profiles.is_empty() {
        "all profiles".to_string()
    } else {
        profiles.join(", ")
    }
}

fn viewport_action_button<A>(
    id: impl Into<gpui::ElementId>,
    icon: IconName,
    tooltip: &'static str,
    icon_color: gpui::Hsla,
    action: A,
    theme: &Theme,
) -> impl IntoElement
where
    A: gpui::Action + Clone + 'static,
{
    let hover = theme.secondary_hover;
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(26.0))
        .rounded(px(5.0))
        .text_color(icon_color)
        .hover(move |this| this.bg(hover))
        .cursor_pointer()
        .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tooltip).build(window, cx))
        .child(Icon::new(icon).with_size(px(16.0)))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

impl Focusable for ViewportPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Panel for ViewportPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("view_in_ar"), "Viewport", kit::TabTone::Default)
    }

    fn inner_padding(&self, _cx: &gpui::App) -> bool {
        false
    }
}

impl gpui::EventEmitter<gpui_component::dock::PanelEvent> for ViewportPanel {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_flag_accepts_truthy_values_case_insensitively() {
        for value in ["1", "true", "TRUE", "Yes", "ON"] {
            assert!(diagnostic_flag_value(value), "expected {value} to enable");
        }
        for value in ["", "0", "false", "off", "enabled "] {
            assert!(
                !diagnostic_flag_value(value),
                "expected {value:?} to disable"
            );
        }
    }

    #[test]
    fn runtime_projection_label_preserves_roles_and_launch_profiles() {
        let projection = RuntimeProjectionData {
            name: "az.test.editor-world".to_string(),
            priority: 25,
            roles: vec!["editor-world".to_string(), "play-preview".to_string()],
            launch_profiles: vec!["editor".to_string(), "play".to_string()],
        };

        assert_eq!(
            runtime_projection_label(&projection),
            "az.test.editor-world p25 [editor-world, play-preview] (editor, play)"
        );
    }

    #[test]
    fn runtime_projection_catalog_labels_are_not_truncated() {
        let catalog = EditorRuntimeProjectionCatalog::new(
            (1..=8)
                .map(|index| RuntimeProjectionData {
                    name: format!("az.test.projection-{index:02}"),
                    priority: index,
                    roles: vec![format!("role-{index:02}")],
                    launch_profiles: vec![format!("profile-{index:02}")],
                })
                .collect(),
        );

        let labels = runtime_projection_catalog_labels(&catalog);

        assert_eq!(labels.len(), 8);
        assert_eq!(
            labels.first().map(String::as_str),
            Some("az.test.projection-01 p1 [role-01] (profile-01)")
        );
        assert_eq!(
            labels.last().map(String::as_str),
            Some("az.test.projection-08 p8 [role-08] (profile-08)")
        );
    }

    #[test]
    fn runtime_status_labels_include_state_and_all_diagnostics() {
        let status = EditorRuntimeStatus {
            runtime_id: "editor-world".to_string(),
            state: EditorRuntimeStateData::Failed,
            role: Some("editor-world".to_string()),
            project_id: Some("az.test".to_string()),
            session_slug: Some("lighting".to_string()),
            authored_revision: Some(42),
            diagnostics: vec![
                "launch failed".to_string(),
                "viewport failed".to_string(),
                "stop failed".to_string(),
                "cleanup failed".to_string(),
            ],
        };

        assert_eq!(
            runtime_status_summary_label(&status),
            "Runtime: editor-world / failed"
        );
        assert_eq!(
            runtime_status_detail_labels(&status),
            vec![
                "role editor-world",
                "authored rev 42",
                "project az.test",
                "session lighting",
                "diag launch failed",
                "diag viewport failed",
                "diag stop failed",
                "diag cleanup failed",
            ]
        );
    }

    #[test]
    fn gpu_status_labels_show_launch_state_and_adapter_details() {
        let status =
            EditorGpuStatus::ready("NVIDIA Test Adapter", "Dx12", "DiscreteGpu", "test-driver");

        assert_eq!(
            gpu_status_labels(&status),
            vec![
                "GPU: ready / Dx12 / DiscreteGpu",
                "adapter NVIDIA Test Adapter",
                "driver test-driver",
            ]
        );
    }

    #[test]
    fn gpu_status_labels_include_failures_without_backend_details() {
        let status = EditorGpuStatus::failed("adapter unavailable");

        assert_eq!(
            gpu_status_labels(&status),
            vec!["GPU: failed", "diag adapter unavailable"]
        );
    }

    #[test]
    fn viewport_render_status_labels_show_composition_surface_and_failures() {
        let gpu_status = EditorViewportRenderStatus::editor_composition_surface(
            7,
            1280,
            720,
            "bgra8Unorm",
            "Dx12",
        );

        assert_eq!(
            viewport_render_status_labels(&gpu_status),
            vec![
                "Viewport render: editor composition surface / 1280x720 gen 7 / bgra8Unorm / Dx12"
            ]
        );

        let failed =
            EditorViewportRenderStatus::failed(8, 1280, 720, "bgra8Unorm", "upload failed");

        assert_eq!(
            viewport_render_status_labels(&failed),
            vec![
                "Viewport render: failed / 1280x720 gen 8 / bgra8Unorm",
                "render diag upload failed",
            ]
        );
    }

    #[test]
    fn viewport_render_status_labels_show_gpu_surface_handles() {
        let gpu_status = EditorViewportRenderStatus::gpu_surface_handle(
            9,
            1280,
            720,
            "bgra8Unorm",
            "gpuSurface",
        );

        assert_eq!(
            viewport_render_status_labels(&gpu_status),
            vec!["Viewport render: gpu surface handle / 1280x720 gen 9 / bgra8Unorm / gpuSurface"]
        );
    }

    #[test]
    fn viewport_render_status_labels_include_optional_telemetry() {
        let gpu_status = EditorViewportRenderStatus::gpu_surface_handle(
            9,
            1280,
            720,
            "bgra8Unorm",
            "gpuSurface",
        )
        .with_telemetry(EditorViewportTelemetryData {
            fps: Some(144),
            frame_time_us: Some(6_944),
            draw_calls: Some(42),
            triangles: Some(10_240),
            vertices: Some(6_400),
            gpu_memory_bytes: Some(2 * 1024 * 1024 * 1024),
        });

        assert_eq!(
            viewport_render_status_labels(&gpu_status),
            vec![
                "Viewport render: gpu surface handle / 1280x720 gen 9 / bgra8Unorm / gpuSurface",
                "144 fps",
                "6.944 ms",
                "42 draw calls",
                "10240 triangles",
                "6400 vertices",
                "2.0 GiB GPU memory",
            ]
        );
    }

    #[test]
    fn viewport_metadata_labels_combine_frame_and_gpu_status() {
        let status = EditorGpuStatus::starting();
        let render_status = EditorViewportRenderStatus::gpu_surface_handle(
            4,
            1920,
            1080,
            "bgra8Unorm",
            "gpuSurface",
        );

        assert_eq!(
            viewport_metadata_labels(None, Some(&status), Some(&render_status), (1920, 1080)),
            vec![
                "Viewport: 1920x1080 waiting",
                "Viewport render: gpu surface handle / 1920x1080 gen 4 / bgra8Unorm / gpuSurface",
                "GPU: starting"
            ]
        );
    }

    #[test]
    fn mannequin_preview_select_motion_autoplays_and_resets_position() {
        let mut preview = EditorMannequinPreview::default_for_project_asset_root("assets");
        preview.playing = false;
        preview.position_millis = 440;

        preview.select_motion("animations/locomotion/walk.anim.glb");

        assert_eq!(
            preview.motion_glb.as_deref(),
            Some("animations/locomotion/walk.anim.glb")
        );
        assert!(preview.playing);
        assert_eq!(preview.position_millis, 0);
    }

    #[test]
    fn mannequin_preview_transport_state_is_explicit() {
        let mut preview = EditorMannequinPreview::default_for_project_asset_root("assets");

        preview.set_playing(false);
        preview.set_looping(false);
        preview.seek_millis(250);

        assert!(!preview.playing);
        assert!(!preview.looping);
        assert_eq!(preview.position_millis, 250);

        preview.stop();

        assert!(!preview.playing);
        assert_eq!(preview.position_millis, 0);
    }

    #[test]
    fn viewport_device_rect_scales_and_rounds_to_device_pixels() {
        assert_eq!(
            viewport_device_rect((10.0, 20.0), (640.0, 360.0), 1.5),
            ViewportDeviceRect {
                left: 15,
                top: 30,
                right: 975,
                bottom: 570,
            }
        );
        // Fractional results round to the nearest device pixel.
        assert_eq!(
            viewport_device_rect((10.3, 20.7), (100.4, 99.6), 1.0),
            ViewportDeviceRect {
                left: 10,
                top: 21,
                right: 111,
                bottom: 120,
            }
        );
        // Nonsense scale factors fall back to 1.0 instead of poisoning sizes.
        assert_eq!(
            viewport_device_rect((0.0, 0.0), (100.0, 50.0), f32::NAN),
            ViewportDeviceRect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 50,
            }
        );
    }

    #[test]
    fn viewport_panel_frame_freshness_requires_recent_paint_and_nonzero_size() {
        let mut frame = EditorViewportPanelFrame::default();
        assert!(!frame.is_fresh(Duration::from_secs(1)));

        frame.layout.device_rect = ViewportDeviceRect {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 720,
        };
        frame.layout.visible = true;
        frame.painted_at = Some(Instant::now());
        assert!(frame.is_fresh(Duration::from_secs(1)));

        frame.layout.device_rect.right = 0;
        assert!(!frame.is_fresh(Duration::from_secs(1)));
    }

    #[test]
    fn viewport_input_queue_drains_in_order_and_is_bounded() {
        let mut queue = EditorViewportInputQueue::default();
        queue.push(ViewportInputEvent::Pick { x: 0.5, y: 0.5 });
        queue.push(ViewportInputEvent::Dolly { steps: 1.0 });

        assert_eq!(
            queue.drain(),
            vec![
                ViewportInputEvent::Pick { x: 0.5, y: 0.5 },
                ViewportInputEvent::Dolly { steps: 1.0 },
            ]
        );
        assert!(queue.drain().is_empty());

        for _ in 0..2048 {
            queue.push(ViewportInputEvent::Dolly { steps: 1.0 });
        }
        assert_eq!(queue.drain().len(), 1024);
    }

    #[test]
    fn viewport_hover_moves_coalesce_to_one_probe_per_frame() {
        let mut queue = EditorViewportInputQueue::default();
        queue.push(ViewportInputEvent::HoverMove { x: 0.1, y: 0.2 });
        queue.push(ViewportInputEvent::HoverMove { x: 0.7, y: 0.8 });

        assert_eq!(
            queue.take_hover(),
            Some(ViewportInputEvent::HoverMove { x: 0.7, y: 0.8 })
        );
        assert!(queue.drain().is_empty());

        queue.push(ViewportInputEvent::HoverMove { x: 0.4, y: 0.5 });
        queue.push(ViewportInputEvent::HoverLeave);
        assert_eq!(queue.take_hover(), Some(ViewportInputEvent::HoverLeave));
    }

    #[test]
    fn viewport_drag_threshold_requires_more_than_four_pixels() {
        assert!(!exceeds_click_drag_threshold(4.0, 0.0));
        assert!(!exceeds_click_drag_threshold(2.0, 3.0));
        assert!(exceeds_click_drag_threshold(4.01, 0.0));
        assert!(exceeds_click_drag_threshold(3.0, 3.0));
    }

    #[test]
    fn triad_axis_directions_match_identity_camera() {
        // Camera on +Z looking at the origin: world X points screen-right,
        // world Y points screen-up (negative screen y), world Z toward viewer.
        let [x_axis, y_axis, z_axis] = triad_axis_directions(0.0, 0.0);
        assert!((x_axis.0 - 1.0).abs() < 1e-5 && x_axis.1.abs() < 1e-5);
        assert!(y_axis.0.abs() < 1e-5 && (y_axis.1 + 1.0).abs() < 1e-5);
        assert!(z_axis.0.abs() < 1e-5 && z_axis.1.abs() < 1e-5);

        // Quarter turn of yaw swings world X toward the viewer and world Z
        // to screen-left; Y stays up.
        let [x_axis, y_axis, z_axis] = triad_axis_directions(std::f32::consts::FRAC_PI_2, 0.0);
        assert!(x_axis.0.abs() < 1e-5 && x_axis.1.abs() < 1e-5);
        assert!((y_axis.1 + 1.0).abs() < 1e-5);
        assert!((z_axis.0 + 1.0).abs() < 1e-5 && z_axis.1.abs() < 1e-5);
    }

    #[test]
    fn animation_catalog_selected_motion_uses_preview_motion() {
        let catalog = EditorAnimationPreviewCatalog::new(
            Some("assets".into()),
            vec![EditorAnimationMotionData {
                asset_path: "animations/locomotion/walk.anim.glb".to_owned(),
                name: "Walk".to_owned(),
                set_path: "animations/locomotion".to_owned(),
                duration_millis: Some(1250),
                channel_count: 1,
                joint_targets: vec!["hips".to_owned()],
                events: Vec::new(),
                pipeline_status: Some("current".to_owned()),
            }],
            Vec::new(),
            Vec::new(),
        );
        let mut preview = EditorMannequinPreview::default_for_project_asset_root("assets");
        preview.motion_glb = Some("animations/locomotion/walk.anim.glb".to_owned());

        assert_eq!(
            catalog
                .selected_motion(Some(&preview))
                .map(|motion| motion.name.as_str()),
            Some("Walk")
        );
    }
}
