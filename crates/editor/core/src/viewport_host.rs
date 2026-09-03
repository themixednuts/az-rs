//! `DirectComposition` in-process viewport host.
//!
//! Owns the editor's in-process bevy renderer ([`EditorRenderApp`]) on a
//! dedicated production thread and connects it to the GPUI main thread through
//! latest-wins mailboxes:
//!
//! - **Boot**: [`boot_viewport_renderer`] receives the renderer-lifetime-bound
//!   visual capability after the GPUI window creates its composition tree.
//! - **Production**: Bevy owns acquisition, rendering, and presentation of its
//!   composition swapchain. Its own DXGI capacity paces the producer.
//! - **GPUI bridge**: the main thread publishes changed configuration, scene,
//!   and semantic input only, then consumes status/camera/commit completions.
//!   It never ticks Bevy or waits for the render product.
//! - **Picking**: the producer publishes an immutable triangle snapshot when
//!   camera/scene/gizmo geometry changes. Pointer-down resolves against that
//!   current-frame snapshot without waiting behind an in-flight render tick.
//! - **Content bridge**: loaded prefab snapshots are published without a
//!   component-specific reduction. The production thread adapts their value
//!   trees to the canonical component lowerer and overlays editor markers.
//! - **Input**: the viewport panel's [`EditorViewportInputQueue`] is drained
//!   each tick into camera orbit/pan/dolly and triangle-accurate picks that
//!   route into the authored-selection → inspector path.
#![cfg(target_os = "windows")]

use std::collections::{BTreeSet, HashSet};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::Duration;

use az_editor_inspector::ReflectedEditBinding;
use az_editor_ui::panels::{
    AnimationMannequinPanel, EditorActiveLevel, EditorAuthoredOutline, EditorBlendSpacePreview,
    EditorLayerVisibility, EditorMannequinPreview, EditorViewportCameraState,
    EditorViewportInputQueue, EditorViewportPanelFrame, EditorViewportRenderStatus, ProfilerPanel,
    ViewportCameraDragKind, ViewportCompositionLayout, ViewportInputEvent, ViewportPanel,
    ViewportSurfaceOwner, active_level_prefab_documents,
};
use az_editor_ui::{
    EditorScenePivot, EditorSceneToolKind, EditorSceneToolState, EditorSceneTransformSpace,
};
use az_proto_project::vnext::{
    PrefabEditCommand, PrefabSourceSnapshot, PrefabValueTarget, ReflectedPath,
    ReflectedValueEncoding, ReflectedValueEnvelope, TypeRegistrySnapshot,
};
use bevy::prelude::{Vec2, Vec3};
use gpui::{App, BorrowAppContext, Global, Rgba};
use gpui_component::ActiveTheme as _;
use tracing::{info, instrument, warn};

use crate::authored_edit::ReflectedPrefabEdit;
use crate::authored_selection::{EditorReflectedSelectionState, ReflectedPrefabSelection};
use crate::editor_render::gizmo::{
    GizmoCommit, GizmoCommitValue, GizmoMode, GizmoPivot, GizmoSnap, GizmoSpace,
};
use crate::editor_render::pick::{EditorPickSnapshot, EditorPickSnapshotResult};
use crate::editor_render::{
    EditorRenderApp, EditorRenderInitError, MannequinPlaybackStatus, ViewportRenderTheme,
};
use crate::error::EditorError;
use crate::gpu::{ViewportPickHit, ViewportScene, ViewportSourceSnapshot};
use crate::viewport_drag::{CameraDragSample, CameraDragTimeline, CameraDragTransition};
use crate::viewport_resize::{ResizeDecision, ViewportResizePolicy};

/// Default render-target size until the viewport panel reports its geometry.
pub const DEFAULT_VIEWPORT_SIZE: (u32, u32) = (1280, 720);
/// Lightweight GPUI mailbox polling remains slightly above 120 Hz on slower
/// displays so input/output handoff stays within one interaction frame.
const MIN_MAILBOX_PUMP_HZ: u32 = 125;
/// How stale the panel-published frame may be before the surface is treated as
/// hidden and released. The pump refreshes windows continuously while live, so
/// a visible panel repaints well within this window.
const PANEL_FRAME_MAX_AGE: Duration = Duration::from_millis(400);
const VIEWPORT_FORMAT_LABEL: &str = "bgra8UnormSrgb";
const VIEWPORT_BACKEND_LABEL: &str = "Dx12/bevy";
const TELEMETRY_INTERVAL: Duration = Duration::from_millis(250);
const DEVICE_LOSS_REBUILD_RETRY: Duration = Duration::from_millis(500);

#[derive(Debug, thiserror::Error)]
pub enum ViewportBootError {
    #[error("failed to spawn the az-viewport-production thread")]
    ThreadSpawn(#[source] std::io::Error),
    #[error("failed to initialize the Bevy renderer: {0}")]
    Renderer(#[from] EditorRenderInitError),
    #[error("the viewport-production thread disconnected during boot: {0}")]
    BootThreadDisconnected(String),
    #[error("the viewport-production thread exited before its start signal")]
    StartHandshakeDisconnected,
    #[error("failed to initialize the viewport-production COM apartment: {0}")]
    ComApartment(String),
    #[error("the active GPUI window did not expose its viewport visual slot")]
    MissingCompositionVisual,
    #[error("the viewport composition visual was invalidated during boot: {0}")]
    CompositionVisual(String),
}

struct ProducerComApartment;

impl ProducerComApartment {
    fn initialize() -> Result<Self, ViewportBootError> {
        unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            )
        }
        .ok()
        .map_err(|error| ViewportBootError::ComApartment(format!("{error:?}")))?;
        Ok(Self)
    }
}

impl Drop for ProducerComApartment {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoUninitialize() };
    }
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&'static str>()
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "non-string panic payload".to_owned())
}

#[derive(Debug, Clone, PartialEq)]
struct AssetDropPlacement {
    interaction_id: u64,
    source_path: String,
    position: Vec3,
}

#[derive(Debug, Clone)]
struct PendingViewportPick {
    interaction_id: u64,
    hit: ViewportPickHit,
}

#[derive(Clone, Copy, Debug)]
struct PointerGesture {
    interaction_id: u64,
    grabbed_gizmo: bool,
    picked_on_down: bool,
}

fn apply_camera_drag_sample(
    render_app: &mut EditorRenderApp,
    pick_snapshot_dirty: &mut bool,
    production_frame: u64,
    sample: CameraDragSample,
) {
    crate::perf::camera_drag_cursor_applied(sample.sampled_at);
    if sample.delta == Vec2::ZERO {
        return;
    }
    crate::perf::camera_drag_first_delta(
        sample.interaction_id,
        production_frame,
        sample.sampled_at,
    );
    match sample.kind {
        ViewportCameraDragKind::Orbit => render_app.camera_orbit(sample.delta.x, sample.delta.y),
        ViewportCameraDragKind::Pan => render_app.camera_pan(sample.delta.x, sample.delta.y),
    }
    render_app.set_pointer_present_sample(sample.interaction_id, sample.sampled_at);
    *pick_snapshot_dirty = true;
}

/// Whether the authored-scene bridge needs re-applying, and whether it is
/// currently cleared because an Animation-owned surface holds the world.
#[derive(Clone, Copy, Debug, Default)]
struct SceneBridgeState {
    /// Whether the scene (or its visibility filter) must be re-applied.
    dirty: bool,
    /// Whether the authored-scene bridge entities are currently cleared
    /// (Animation mode shows only the mannequin, not the scene).
    suppressed: bool,
}

/// State confined to the dedicated viewport-production thread. Bevy's `App`
/// is intentionally `!Send`, so it is constructed and used on this one thread
/// instead of being moved out of GPUI after construction.
struct ViewportProductionState {
    render_app: EditorRenderApp,
    frame_count: u64,
    status_key: Option<(u32, u32)>,
    /// Latest translated authored scene (unfiltered by layer visibility).
    scene: ViewportScene,
    /// Hidden-layer set last applied to the render app.
    applied_hidden: BTreeSet<String>,
    scene_bridge: SceneBridgeState,
    /// The mannequin/blend-space previews last applied to the render app while
    /// an Animation-owned surface was live (`None` while the mannequin is
    /// cleared — Scene mode must never show it).
    applied_previews: Option<(EditorMannequinPreview, EditorBlendSpacePreview)>,
    pointer_gesture: Option<PointerGesture>,
    active_asset_drag: Option<u64>,
    camera_drag: CameraDragTimeline,
    resize_policy: ViewportResizePolicy,
    last_frame_started: Option<std::time::Instant>,
    last_telemetry_at: std::time::Instant,
    frame_tick_started: Option<std::time::Instant>,
    composition_frame_acquired: bool,
    pick_snapshot_dirty: bool,
    refreshed_pick_snapshot: Option<EditorPickSnapshot>,
    applied_gizmo: Option<(GizmoMode, GizmoPivot, GizmoSpace, GizmoSnap)>,
}

/// Everything one production frame yields back to GPUI: the status line when it
/// changed, the camera pose, and the four command streams the consumer drains —
/// resolved picks, gizmo commits, asset drops to persist, and mannequin
/// playback. They are produced together by one pump and published together
/// under one `ProductionOutputs` lock.
type ProductionFrameOutcome = (
    Option<EditorViewportRenderStatus>,
    EditorViewportCameraState,
    Vec<PendingViewportPick>,
    Vec<GizmoCommit>,
    Vec<AssetDropPlacement>,
    Option<MannequinPlaybackStatus>,
);

impl ViewportProductionState {
    fn new(render_app: EditorRenderApp) -> Self {
        let now = std::time::Instant::now();
        let configured_extent = render_app.size();
        Self {
            render_app,
            frame_count: 0,
            status_key: None,
            scene: ViewportScene::default(),
            applied_hidden: BTreeSet::new(),
            scene_bridge: SceneBridgeState::default(),
            applied_previews: None,
            pointer_gesture: None,
            active_asset_drag: None,
            camera_drag: CameraDragTimeline::default(),
            resize_policy: ViewportResizePolicy::new(configured_extent, now),
            last_frame_started: None,
            last_telemetry_at: now,
            frame_tick_started: None,
            composition_frame_acquired: false,
            pick_snapshot_dirty: true,
            refreshed_pick_snapshot: None,
            applied_gizmo: None,
        }
    }

    fn pointer_down(
        render_app: &mut EditorRenderApp,
        pointer_gesture: &mut Option<PointerGesture>,
        interaction_id: u64,
        coord: Vec2,
    ) -> Option<PendingViewportPick> {
        let grabbed_gizmo = render_app.begin_gizmo_drag(coord);
        let pick = if grabbed_gizmo {
            crate::perf::selection_coalesced(interaction_id);
            None
        } else {
            crate::perf::selection_pick_issued(interaction_id, coord.x, coord.y);
            let hit = render_app.pick(coord);
            let payload = hit
                .as_ref()
                .map_or(0, |hit| crate::perf::stable_payload(&hit.id));
            crate::perf::selection_pick_resolved(interaction_id, coord.x, coord.y, payload);
            hit.map(|hit| {
                if let (Some(document_id), Some(object_id)) = (&hit.document_id, &hit.object_id) {
                    render_app.set_selected_authored(Some((document_id, object_id)));
                }
                crate::perf::selection_highlight_published(
                    interaction_id,
                    (f32::NAN, f32::NAN, f32::NAN),
                    payload,
                );
                PendingViewportPick {
                    interaction_id,
                    hit,
                }
            })
        };
        *pointer_gesture = Some(PointerGesture {
            interaction_id,
            grabbed_gizmo,
            picked_on_down: !grabbed_gizmo,
        });
        pick
    }

    /// Advance one pump frame: apply the owner-scoped content (authored scene
    /// bridge or mannequin/blend-space previews), selection/gizmo/input, tick
    /// the render app, and report status + camera
    /// + picks + gizmo commits + asset drops to persist + mannequin playback.
    fn advance_frame(
        &mut self,
        frame: &ProductionFrameConfig,
        inputs: Vec<ViewportInputEvent>,
        hover: Option<&ViewportInputEvent>,
    ) -> ProductionFrameOutcome {
        // Exhaustive so a new frame-config field forces a decision here.
        // `render_theme` is applied by the caller before the pump begins.
        let ProductionFrameConfig {
            layout,
            hidden,
            selection,
            selection_accent,
            render_theme: _,
            gizmo,
            previews,
        } = frame;
        let device_origin = (layout.device_rect.left, layout.device_rect.top);
        let device_size = (layout.device_rect.width(), layout.device_rect.height());
        let selection = selection.as_ref();
        let selection_accent = *selection_accent;
        let gizmo = *gizmo;
        let previews = previews.as_ref();

        let frame_started = std::time::Instant::now();
        if let Some(previous) = self.last_frame_started.replace(frame_started) {
            crate::perf::record_ns(
                crate::perf::FRAME_VIEWPORT_PUMP_INTERVAL,
                crate::perf::duration_ns(frame_started.duration_since(previous)),
            );
        }
        self.apply_layout_and_resize(device_size, frame_started);
        self.render_app
            .update_selection_bounds_color(selection_accent);
        if self.render_app.set_selected_authored(
            selection.map(|(document_id, object_id)| (document_id.as_str(), object_id.as_str())),
        ) {
            self.pick_snapshot_dirty = true;
        }
        self.apply_owner_content(previews, hidden);
        self.apply_gizmo_config(previews.is_some(), gizmo);
        self.acknowledge_camera_drag(&inputs, device_origin, device_size);

        // Run non-latency-sensitive main-world work while the previous GPU
        // frame is still in flight. Then acquire from Bevy's own swapchain and
        // sample input across that final blocking boundary. Camera methods
        // update both local and global transforms, so these late changes are
        // visible to extraction without a second full main-world schedule.
        let bevy_tick_started = std::time::Instant::now();
        self.render_app.update_main_world();
        self.frame_tick_started = Some(bevy_tick_started);
        self.composition_frame_acquired = self.render_app.acquire_composition_frame();
        let fresh_cursor = gpui_windows::sample_viewport_cursor(device_origin, device_size)
            .map(|(x, y, _, _)| (Vec2::new(x, y), std::time::Instant::now()));
        let fresh_cursor_position = fresh_cursor.map(|(position, _)| position);

        self.apply_camera_events(&inputs, fresh_cursor);
        let (picks, commits, asset_drops) =
            self.drain_pointer_events(inputs, fresh_cursor_position);
        match hover {
            Some(ViewportInputEvent::HoverMove { x, y }) => {
                self.render_app.set_hovered_at(Vec2::new(*x, *y));
            }
            Some(ViewportInputEvent::HoverLeave) => self.render_app.clear_hover(),
            _ => {}
        }
        self.apply_cursor_driven_updates(fresh_cursor_position);
        self.refresh_pick_snapshot();
        self.log_frame_summaries();

        let (width, height) = self.render_app.size();
        self.frame_count += 1;
        let status = self.publish_status(frame_started, width, height);
        let (yaw_radians, pitch_radians, distance, speed) = self.render_app.camera_pose();
        let camera = EditorViewportCameraState {
            yaw_radians,
            pitch_radians,
            distance,
            speed,
        };
        let playback = self.advance_playback();

        (status, camera, picks, commits, asset_drops, playback)
    }

    /// Publish the latest layout extent, then replace the physical composition
    /// surface when the bounded policy says a replacement is due and the
    /// producer's previous image has reached a terminal state.
    fn apply_layout_and_resize(
        &mut self,
        device_size: (u32, u32),
        frame_started: std::time::Instant,
    ) {
        let layout_resized = self.render_app.layout_size() != device_size;
        if layout_resized {
            self.pick_snapshot_dirty = true;
        }
        self.render_app
            .set_layout_extent(device_size.0, device_size.1);
        self.resize_policy
            .publish_desired(device_size, frame_started);
        if let ResizeDecision::ReplaceWith(extent) = self.resize_policy.decide(frame_started, true)
        {
            if self.render_app.wait_for_composition_surface_idle() {
                debug_assert!(self.render_app.composition_surface_idle());
                if self.render_app.resize(extent.0, extent.1) {
                    self.resize_policy.replaced(extent, frame_started);
                    crate::perf::record_ns(crate::perf::RESIZE_ALLOCATION_COUNT, 1);
                    gpui_windows::record_dcomp_surface_replaced();
                }
            } else {
                tracing::error!(
                    ?extent,
                    "timed out waiting for the composition surface before resize replacement"
                );
            }
        }
        if layout_resized {
            crate::perf::record_ns("viewport.layout_extent_changed", 1);
        }
    }

    /// Apply the content the live surface's owner is responsible for.
    fn apply_owner_content(
        &mut self,
        previews: Option<&(EditorMannequinPreview, EditorBlendSpacePreview)>,
        hidden: &BTreeSet<String>,
    ) {
        // Animation-owned surface: the mannequin/blend-space previews own the
        // world; the authored scene bridge is cleared while active.
        if let Some((mannequin, blend_space)) = previews {
            if self
                .applied_previews
                .as_ref()
                .is_none_or(|(applied_mannequin, applied_blend)| {
                    applied_mannequin != mannequin || applied_blend != blend_space
                })
            {
                self.render_app.apply_mannequin_preview(mannequin.clone());
                self.render_app
                    .apply_blend_space_preview(blend_space.clone());
                self.pick_snapshot_dirty = true;
                self.applied_previews = Some((mannequin.clone(), blend_space.clone()));
            }
            if !self.scene_bridge.suppressed {
                self.render_app
                    .apply_scene(&ViewportScene::default(), &BTreeSet::new());
                self.pick_snapshot_dirty = true;
                self.scene_bridge.suppressed = true;
                // Re-apply the authored scene when a Scene-owned surface takes
                // the slot back.
                self.scene_bridge.dirty = true;
            }
            return;
        }

        // Scene-owned surface: the mannequin must not render; clearing the
        // preview restores the neutral primitives under the scene bridge.
        if self.applied_previews.take().is_some() {
            self.render_app
                .apply_mannequin_preview(EditorMannequinPreview::empty());
        }
        self.scene_bridge.suppressed = false;
        if self.scene_bridge.dirty || self.applied_hidden != *hidden {
            self.render_app.apply_scene(&self.scene, hidden);
            self.pick_snapshot_dirty = true;
            self.applied_hidden.clone_from(hidden);
            self.scene_bridge.dirty = false;
        }
    }

    /// Attach or detach the transform gizmo per the active scene tool.
    /// Animation previews have no authored objects to transform.
    fn apply_gizmo_config(
        &mut self,
        previews_active: bool,
        gizmo: (GizmoMode, GizmoPivot, GizmoSpace, GizmoSnap),
    ) {
        let config = if previews_active {
            (
                GizmoMode::None,
                GizmoPivot::Pivot,
                GizmoSpace::World,
                GizmoSnap::NONE,
            )
        } else {
            gizmo
        };
        self.render_app
            .update_gizmo(config.0, config.1, config.2, config.3);
        if self.applied_gizmo != Some(config) {
            self.applied_gizmo = Some(config);
            self.pick_snapshot_dirty = true;
        }
    }

    /// Consume the semantic drag boundaries before any swapchain wait. A newly
    /// observed Start gets one immediate absolute sample, so it cannot sit
    /// behind a second producer-capacity boundary after mailbox pickup.
    fn acknowledge_camera_drag(
        &mut self,
        inputs: &[ViewportInputEvent],
        device_origin: (i32, i32),
        device_size: (u32, u32),
    ) {
        for event in inputs {
            match event {
                ViewportInputEvent::CameraDragStart {
                    interaction_id,
                    kind,
                    x,
                    y,
                    started_at,
                } => self.camera_drag.push(CameraDragTransition::Start {
                    interaction_id: *interaction_id,
                    kind: *kind,
                    position: Vec2::new(*x, *y),
                    at: *started_at,
                }),
                ViewportInputEvent::CameraDragEnd {
                    interaction_id,
                    x,
                    y,
                    ended_at,
                } => self.camera_drag.push(CameraDragTransition::End {
                    interaction_id: *interaction_id,
                    position: Vec2::new(*x, *y),
                    at: *ended_at,
                }),
                _ => {}
            }
        }
        let started_drag =
            self.camera_drag
                .acknowledge_next(self.frame_count)
                .inspect(|acknowledged| {
                    crate::perf::camera_drag_started(
                        acknowledged.interaction_id,
                        acknowledged.production_frame,
                        acknowledged.started_at,
                    );
                    crate::perf::camera_drag_acknowledged();
                });
        if self.camera_drag.is_active() {
            crate::perf::camera_drag_frame();
        }
        if started_drag.is_some() {
            let immediate_cursor = gpui_windows::sample_viewport_cursor(device_origin, device_size)
                .map(|(x, y, _, _)| (Vec2::new(x, y), std::time::Instant::now()));
            if let Some(sample) = self.camera_drag.sample(immediate_cursor) {
                apply_camera_drag_sample(
                    &mut self.render_app,
                    &mut self.pick_snapshot_dirty,
                    self.frame_count,
                    sample,
                );
            }
        }
    }

    /// Apply the camera events immediately; pointer, pick, and drop events are
    /// deferred until after the tick so camera and global transforms (and
    /// freshly spawned gizmo handles) are current when hit-testing.
    fn apply_camera_events(
        &mut self,
        inputs: &[ViewportInputEvent],
        fresh_cursor: Option<(Vec2, std::time::Instant)>,
    ) {
        for event in inputs {
            match event {
                ViewportInputEvent::Orbit { dx, dy } => {
                    crate::perf::camera_drag_event();
                    self.render_app.camera_orbit(*dx, *dy);
                    self.pick_snapshot_dirty = true;
                }
                ViewportInputEvent::Pan { dx, dy } => {
                    crate::perf::camera_drag_event();
                    self.render_app.camera_pan(*dx, *dy);
                    self.pick_snapshot_dirty = true;
                }
                ViewportInputEvent::Dolly { steps } => {
                    self.render_app.camera_dolly(*steps);
                    self.pick_snapshot_dirty = true;
                }
                ViewportInputEvent::FrameSelected => {
                    if self.render_app.frame_selected() {
                        self.pick_snapshot_dirty = true;
                    }
                }
                ViewportInputEvent::SetCameraView { view } => {
                    self.render_app.set_camera_view(*view);
                    self.pick_snapshot_dirty = true;
                }
                ViewportInputEvent::SetShadingMode { mode } => {
                    self.render_app.set_shading_mode(*mode);
                }
                ViewportInputEvent::SetVisibility { settings } => {
                    self.render_app.set_visibility(*settings);
                    self.pick_snapshot_dirty = true;
                }
                _ => {}
            }
        }
        if let Some(sample) = self.camera_drag.sample(fresh_cursor) {
            apply_camera_drag_sample(
                &mut self.render_app,
                &mut self.pick_snapshot_dirty,
                self.frame_count,
                sample,
            );
        }
        if let Some(finished) = self.camera_drag.finish_acknowledged(self.frame_count) {
            crate::perf::camera_drag_ended(
                finished.interaction_id,
                finished.production_frame,
                finished.ended_at,
            );
        }
        let unacknowledged = self.camera_drag.take_orphaned_end_count();
        debug_assert_eq!(unacknowledged, 0);
        crate::perf::camera_drag_unacknowledged(unacknowledged);
    }

    /// Resolve the deferred pointer, pick, and asset-drop events against the
    /// world this frame already ticked.
    fn drain_pointer_events(
        &mut self,
        inputs: Vec<ViewportInputEvent>,
        fresh_cursor_position: Option<Vec2>,
    ) -> (
        Vec<PendingViewportPick>,
        Vec<GizmoCommit>,
        Vec<AssetDropPlacement>,
    ) {
        let mut picks = Vec::new();
        let mut commits = Vec::new();
        let mut asset_drops = Vec::new();
        for event in inputs {
            match event {
                ViewportInputEvent::Pick { x, y } => {
                    if let Some(hit) = self.render_app.pick(Vec2::new(x, y)) {
                        picks.push(PendingViewportPick {
                            interaction_id: 0,
                            hit,
                        });
                    }
                }
                ViewportInputEvent::PointerDown {
                    interaction_id,
                    x,
                    y,
                } => {
                    if let Some(pick) = Self::pointer_down(
                        &mut self.render_app,
                        &mut self.pointer_gesture,
                        interaction_id,
                        Vec2::new(x, y),
                    ) {
                        picks.push(pick);
                    }
                }
                ViewportInputEvent::PointerUp {
                    interaction_id,
                    x,
                    y,
                    is_click,
                } => {
                    let (pick, commit) = self.resolve_pointer_up(interaction_id, x, y, is_click);
                    picks.extend(pick);
                    commits.extend(commit);
                }
                ViewportInputEvent::ClickCoalesced { interaction_id } => {
                    crate::perf::selection_coalesced(interaction_id);
                }
                ViewportInputEvent::HoverMove { x, y } => {
                    self.render_app.set_hovered_at(Vec2::new(x, y));
                }
                ViewportInputEvent::HoverLeave => self.render_app.clear_hover(),
                ViewportInputEvent::AssetDragEnter { interaction_id, .. } => {
                    self.active_asset_drag = Some(interaction_id);
                }
                ViewportInputEvent::AssetDragLeave { .. } => {
                    self.active_asset_drag = None;
                    self.render_app.clear_asset_ghost();
                }
                ViewportInputEvent::DropAsset {
                    interaction_id,
                    source_path,
                    x: event_x,
                    y: event_y,
                } => {
                    self.active_asset_drag = None;
                    let cursor =
                        fresh_cursor_position.unwrap_or_else(|| Vec2::new(event_x, event_y));
                    asset_drops.extend(self.resolve_asset_drop(
                        interaction_id,
                        source_path,
                        cursor,
                    ));
                }
                ViewportInputEvent::PointerMove { .. }
                | ViewportInputEvent::Orbit { .. }
                | ViewportInputEvent::Pan { .. }
                | ViewportInputEvent::CameraDragStart { .. }
                | ViewportInputEvent::CameraDragEnd { .. }
                | ViewportInputEvent::Dolly { .. }
                | ViewportInputEvent::SetCameraView { .. }
                | ViewportInputEvent::SetShadingMode { .. }
                | ViewportInputEvent::SetVisibility { .. }
                | ViewportInputEvent::FrameSelected => {}
            }
        }
        (picks, commits, asset_drops)
    }

    /// Finish the gesture a pointer-up ends: commit an in-flight gizmo drag, or
    /// resolve the click's selection when pointer-down did not already pick.
    fn resolve_pointer_up(
        &mut self,
        interaction_id: u64,
        x: f32,
        y: f32,
        is_click: bool,
    ) -> (Option<PendingViewportPick>, Option<GizmoCommit>) {
        let Some(gesture) = self.pointer_gesture.take() else {
            return (None, None);
        };
        if gesture.grabbed_gizmo {
            self.render_app.update_gizmo_drag(Vec2::new(x, y));
            return (None, self.render_app.end_gizmo_drag());
        }
        if !is_click || gesture.picked_on_down {
            return (None, None);
        }
        crate::perf::selection_pick_issued(interaction_id, x, y);
        let hit = self.render_app.pick(Vec2::new(x, y));
        let payload = hit
            .as_ref()
            .map_or(0, |hit| crate::perf::stable_payload(&hit.id));
        crate::perf::selection_pick_resolved(interaction_id, x, y, payload);
        let Some(hit) = hit else {
            return (None, None);
        };
        if let (Some(document_id), Some(object_id)) = (&hit.document_id, &hit.object_id) {
            self.render_app
                .set_selected_authored(Some((document_id, object_id)));
        }
        crate::perf::selection_highlight_published(
            interaction_id,
            (f32::NAN, f32::NAN, f32::NAN),
            payload,
        );
        (
            Some(PendingViewportPick {
                interaction_id,
                hit,
            }),
            None,
        )
    }

    /// Place a dropped asset optimistically on the ground plane so the object is
    /// visible before authored persistence reconciles it.
    fn resolve_asset_drop(
        &mut self,
        interaction_id: u64,
        source_path: String,
        cursor: Vec2,
    ) -> Option<AssetDropPlacement> {
        let position = self.render_app.dropped_asset_position(cursor)?;
        self.render_app.update_asset_ghost(interaction_id, position);
        let _ = self.render_app.solidify_asset_ghost(interaction_id);
        crate::perf::input_trace(
            crate::perf::InputTraceKind::OptimisticObjectPublished,
            interaction_id,
            Some((cursor.x, cursor.y)),
            Some((position.x, position.y, position.z)),
            0,
        );
        crate::perf::request_summary_dump();
        Some(AssetDropPlacement {
            interaction_id,
            source_path,
            position,
        })
    }

    /// GPUI pointer/drag events are semantic transitions only. The rendered
    /// drag position always comes from the absolute cursor sampled after the
    /// DXGI frame-latency wait in this same production tick.
    fn apply_cursor_driven_updates(&mut self, fresh_cursor_position: Option<Vec2>) {
        if self
            .pointer_gesture
            .is_some_and(|gesture| gesture.grabbed_gizmo)
            && let Some(cursor) = fresh_cursor_position
        {
            let interaction_id = self
                .pointer_gesture
                .map_or(0, |gesture| gesture.interaction_id);
            crate::perf::input_trace(
                crate::perf::InputTraceKind::GizmoCursorSampled,
                interaction_id,
                Some((cursor.x, cursor.y)),
                None,
                self.frame_count,
            );
            self.render_app.update_gizmo_drag(cursor);
            if let Some(position) = self.render_app.gizmo_drag_rendered_translation() {
                crate::perf::input_trace(
                    crate::perf::InputTraceKind::GizmoRendered,
                    interaction_id,
                    Some((cursor.x, cursor.y)),
                    Some((position.x, position.y, position.z)),
                    self.frame_count,
                );
            }
        }
        if let (Some(interaction_id), Some(cursor)) =
            (self.active_asset_drag, fresh_cursor_position)
            && let Some(position) = self.render_app.dropped_asset_position(cursor)
        {
            crate::perf::input_trace(
                crate::perf::InputTraceKind::GhostCursorSampled,
                interaction_id,
                Some((cursor.x, cursor.y)),
                None,
                self.frame_count,
            );
            self.render_app.update_asset_ghost(interaction_id, position);
            crate::perf::input_trace(
                crate::perf::InputTraceKind::GhostUpdated,
                interaction_id,
                Some((cursor.x, cursor.y)),
                Some((position.x, position.y, position.z)),
                self.frame_count,
            );
        }
    }

    /// Republish the immutable pick snapshot when camera, scene, or gizmo
    /// geometry changed this frame.
    fn refresh_pick_snapshot(&mut self) {
        if self.pick_snapshot_dirty
            && let Some(snapshot) = self.render_app.pick_snapshot()
        {
            self.refreshed_pick_snapshot = Some(snapshot);
            self.pick_snapshot_dirty = false;
        }
    }

    /// Keep a cold-start table, then begin a clean sustained window after
    /// first-use render pipelines and project bootstrap have settled.
    fn log_frame_summaries(&self) {
        if self.frame_count == 299 {
            crate::perf::log_summary();
            crate::perf::reset_frame_samples();
        } else if self.frame_count == 599 {
            crate::perf::request_summary_dump();
        }
        if crate::perf::take_summary_dump_request() {
            crate::perf::log_summary();
        }
    }

    /// Republish the render status when the surface extent changed or the
    /// bounded telemetry interval elapsed.
    fn publish_status(
        &mut self,
        frame_started: std::time::Instant,
        width: u32,
        height: u32,
    ) -> Option<EditorViewportRenderStatus> {
        let status_key = (width, height);
        let telemetry_due =
            frame_started.duration_since(self.last_telemetry_at) >= TELEMETRY_INTERVAL;
        if telemetry_due {
            self.last_telemetry_at = frame_started;
        }
        (self.status_key != Some(status_key) || telemetry_due).then(|| {
            self.status_key = Some(status_key);
            EditorViewportRenderStatus::editor_composition_surface(
                self.frame_count,
                width,
                height,
                VIEWPORT_FORMAT_LABEL,
                VIEWPORT_BACKEND_LABEL,
            )
            .with_telemetry(self.render_app.telemetry())
        })
    }

    /// Playhead advance: while an animation preview is playing, read the bevy
    /// `AnimationPlayer`'s seek position back so the Motion Tracks playhead
    /// animates. The read-back is folded into the applied preview so publishing
    /// it to the global does not re-trigger a seek — a user scrub (a position
    /// change not matching the read-back) stays authoritative.
    fn advance_playback(&mut self) -> Option<MannequinPlaybackStatus> {
        let (mannequin, _) = self.applied_previews.as_mut()?;
        if !mannequin.playing {
            return None;
        }
        let playback = self.render_app.mannequin_playback_status();
        if let Some(playback) = playback {
            mannequin.position_millis = playback.position_millis;
            if playback.finished && !mannequin.looping {
                mannequin.playing = false;
            }
        }
        playback
    }

    /// Extract the frame after GPUI has synchronously published any pick result.
    /// This keeps the logical and measured chain ordered while retaining the
    /// same-frame render product.
    fn finish_frame(&mut self) {
        if !self.composition_frame_acquired {
            return;
        }
        self.render_app.render_current_world();
        if let Some(started) = self.frame_tick_started.take() {
            crate::perf::record_elapsed(crate::perf::FRAME_BEVY_TICK, started);
        }
    }

    const fn take_refreshed_pick_snapshot(&mut self) -> Option<EditorPickSnapshot> {
        self.refreshed_pick_snapshot.take()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ProductionFrameConfig {
    layout: ViewportCompositionLayout,
    hidden: BTreeSet<String>,
    selection: Option<(String, String)>,
    selection_accent: [f32; 4],
    render_theme: ViewportRenderTheme,
    gizmo: (GizmoMode, GizmoPivot, GizmoSpace, GizmoSnap),
    previews: Option<(EditorMannequinPreview, EditorBlendSpacePreview)>,
}

#[derive(Default)]
struct ProductionMailbox {
    config: Mutex<Option<ProductionFrameConfig>>,
    scene: Mutex<Option<ViewportScene>>,
    inputs: Mutex<Vec<ViewportInputEvent>>,
    hover: Mutex<Option<ViewportInputEvent>>,
    commands: Mutex<Vec<ProductionCommand>>,
    pick_snapshot: Mutex<Option<Arc<EditorPickSnapshot>>>,
    fast_pick_ids: Mutex<HashSet<u64>>,
    fast_picks: Mutex<Vec<PendingViewportPick>>,
    fast_pick_enabled: AtomicBool,
    live: AtomicBool,
    shutdown: AtomicBool,
}

static ACTIVE_PRODUCTION_MAILBOX: Mutex<Option<Arc<ProductionMailbox>>> = Mutex::new(None);

/// What resolving a pointer-down against the published pick snapshot produced.
enum FastPickOutcome {
    /// Fast picking is disabled, or no snapshot is published yet: the producer
    /// resolves this pointer-down against its own world.
    Unavailable,
    /// The snapshot resolved a gizmo handle, so the producer owns the gesture.
    Gizmo,
    /// The snapshot resolved the pick on the GPUI thread, with these timings.
    Resolved {
        pick_issued: std::time::Instant,
        pick_resolved: std::time::Instant,
        payload: u64,
        highlighted: bool,
    },
}

/// Resolve a pointer-down against the current-frame pick snapshot without
/// waiting behind an in-flight render tick.
fn resolve_fast_pick(
    mailbox: &ProductionMailbox,
    interaction_id: u64,
    coord: Vec2,
) -> FastPickOutcome {
    if !mailbox.fast_pick_enabled.load(Ordering::Acquire) {
        return FastPickOutcome::Unavailable;
    }
    let published = lock(&mailbox.pick_snapshot).clone();
    let Some(snapshot) = published else {
        return FastPickOutcome::Unavailable;
    };
    let pick_issued = std::time::Instant::now();
    let hit = match snapshot.pick(coord) {
        EditorPickSnapshotResult::Gizmo => return FastPickOutcome::Gizmo,
        EditorPickSnapshotResult::Scene(hit) => Some(hit),
        EditorPickSnapshotResult::Miss => None,
    };
    let pick_resolved = std::time::Instant::now();
    let payload = hit
        .as_ref()
        .map_or(0, |hit| crate::perf::stable_payload(&hit.id));
    lock(&mailbox.fast_pick_ids).insert(interaction_id);
    let highlighted = hit.is_some();
    if let Some(hit) = hit {
        lock(&mailbox.fast_picks).push(PendingViewportPick {
            interaction_id,
            hit,
        });
    }
    FastPickOutcome::Resolved {
        pick_issued,
        pick_resolved,
        payload,
        highlighted,
    }
}

pub(crate) fn publish_urgent_pointer_down(
    interaction_id: u64,
    x: f32,
    y: f32,
    pointer_down: std::time::Instant,
) -> bool {
    let active = lock(&ACTIVE_PRODUCTION_MAILBOX).clone();
    let Some(mailbox) = active else {
        return false;
    };
    let outcome = resolve_fast_pick(&mailbox, interaction_id, Vec2::new(x, y));
    lock(&mailbox.inputs).push(ViewportInputEvent::PointerDown {
        interaction_id,
        x,
        y,
    });
    match outcome {
        FastPickOutcome::Resolved {
            pick_issued,
            pick_resolved,
            payload,
            highlighted,
        } => {
            crate::perf::selection_fast_pick(&crate::perf::SelectionFastPick {
                interaction_id,
                x,
                y,
                pointer_down,
                pick_issued,
                pick_resolved,
                payload,
                highlighted,
            });
            true
        }
        FastPickOutcome::Unavailable | FastPickOutcome::Gizmo => false,
    }
}

enum ProductionCommand {
    MarkAssetAuthored {
        interaction_id: u64,
        authored_object_id: String,
    },
}

#[derive(Default)]
struct ProductionOutputs {
    status: Option<EditorViewportRenderStatus>,
    camera: Option<EditorViewportCameraState>,
    picks: Vec<PendingViewportPick>,
    commits: Vec<GizmoCommit>,
    asset_drops: Vec<AssetDropPlacement>,
    playback: Option<MannequinPlaybackStatus>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

const fn rgba_array(color: Rgba) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

/// Handle retained by GPUI while the `!Send` Bevy app remains confined to its
/// dedicated production thread.
pub struct EditorViewportProducer {
    mailbox: Arc<ProductionMailbox>,
    outputs: Arc<Mutex<ProductionOutputs>>,
    thread: Option<JoinHandle<()>>,
    visual_slot: gpui_windows::ViewportVisualSlot,
    product_root: Option<std::path::PathBuf>,
}

impl EditorViewportProducer {
    fn publish_config(&self, config: ProductionFrameConfig) {
        *lock(&self.mailbox.config) = Some(config);
    }

    fn set_live(&self, live: bool) {
        let was_live = self.mailbox.live.swap(live, Ordering::AcqRel);
        if live
            && !was_live
            && let Some(thread) = self.thread.as_ref()
        {
            thread.thread().unpark();
        }
    }

    fn publish_scene(&self, scene: ViewportScene) {
        *lock(&self.mailbox.scene) = Some(scene);
    }

    fn publish_inputs(
        &self,
        mut inputs: Vec<ViewportInputEvent>,
        hover: Option<ViewportInputEvent>,
    ) {
        if !inputs.is_empty() {
            lock(&self.mailbox.inputs).append(&mut inputs);
        }
        if hover.is_some() {
            *lock(&self.mailbox.hover) = hover;
        }
    }

    fn take_outputs(&self) -> ProductionOutputs {
        std::mem::take(&mut *lock(&self.outputs))
    }

    fn command(&self, command: ProductionCommand) {
        lock(&self.mailbox.commands).push(command);
    }
}

impl Drop for EditorViewportProducer {
    fn drop(&mut self) {
        self.mailbox.shutdown.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

/// Lightweight GPUI global: authored-scene fetch bookkeeping plus the mailbox
/// handle. It never owns or ticks Bevy.
pub struct EditorViewportHost {
    producer: EditorViewportProducer,
    scene: ViewportScene,
    scene_signature: Vec<(String, u64)>,
    fetch_signature: Option<Vec<(String, u64)>>,
    last_config: Option<ProductionFrameConfig>,
    last_rebuild_attempt: Option<std::time::Instant>,
}

impl Global for EditorViewportHost {}

struct ViewportRebuildRequest {
    product_root: Option<std::path::PathBuf>,
    window_id: u64,
}

fn device_loss_rebuild_due(
    visual_slot_valid: bool,
    last_attempt: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    !visual_slot_valid
        && last_attempt
            .is_none_or(|last| now.saturating_duration_since(last) >= DEVICE_LOSS_REBUILD_RETRY)
}

impl EditorViewportHost {
    fn new(producer: EditorViewportProducer) -> Self {
        Self {
            producer,
            scene: ViewportScene::default(),
            scene_signature: Vec::new(),
            fetch_signature: None,
            last_config: None,
            last_rebuild_attempt: None,
        }
    }

    fn prepare_device_loss_rebuild(
        &mut self,
        now: std::time::Instant,
    ) -> Option<ViewportRebuildRequest> {
        if !device_loss_rebuild_due(
            self.producer.visual_slot.is_valid(),
            self.last_rebuild_attempt,
            now,
        ) {
            return None;
        }
        self.last_rebuild_attempt = Some(now);
        self.producer.set_live(false);
        Some(ViewportRebuildRequest {
            product_root: self.producer.product_root.clone(),
            window_id: self.producer.visual_slot.window_id(),
        })
    }

    fn replace_after_device_loss(&mut self, producer: EditorViewportProducer) {
        producer.publish_scene(self.scene.clone());
        self.producer = producer;
        self.last_config = None;
        self.last_rebuild_attempt = None;
    }
}

/// Publish the picks the GPUI thread already resolved against the snapshot,
/// mirroring the selection they imply into the production world and into the
/// frame config the next tick will read.
fn apply_fast_picks(
    state: &mut ViewportProductionState,
    mailbox: &ProductionMailbox,
    outputs: &Mutex<ProductionOutputs>,
    config: &mut Option<ProductionFrameConfig>,
) {
    let fast_picks = std::mem::take(&mut *lock(&mailbox.fast_picks));
    if fast_picks.is_empty() {
        return;
    }
    let mut completed = lock(outputs);
    for pick in fast_picks {
        if let (Some(document_id), Some(object_id)) = (&pick.hit.document_id, &pick.hit.object_id) {
            state
                .render_app
                .set_selected_authored(Some((document_id, object_id)));
            if let Some(config) = config.as_mut() {
                config.selection = Some((document_id.clone(), object_id.clone()));
            }
        }
        completed.picks.push(pick);
    }
}

/// Apply the commands the GPUI thread queued for the production world.
fn apply_production_commands(state: &mut ViewportProductionState, mailbox: &ProductionMailbox) {
    let commands = std::mem::take(&mut *lock(&mailbox.commands));
    for command in commands {
        match command {
            ProductionCommand::MarkAssetAuthored {
                interaction_id,
                authored_object_id,
            } => state
                .render_app
                .mark_asset_authored(interaction_id, authored_object_id),
        }
    }
}

/// Service pointer-down hit testing against the current production world while
/// DXGI is between frames, keeping the proven local pick path independent of
/// render cadence.
fn drain_urgent_pointer_downs(
    state: &mut ViewportProductionState,
    mailbox: &ProductionMailbox,
    outputs: &Mutex<ProductionOutputs>,
    pending_inputs: &mut Vec<ViewportInputEvent>,
) {
    let mut urgent_picks = Vec::new();
    let mut input_index = 0;
    while input_index < pending_inputs.len() {
        let Some((interaction_id, x, y)) = (match &pending_inputs[input_index] {
            ViewportInputEvent::PointerDown {
                interaction_id,
                x,
                y,
            } => Some((*interaction_id, *x, *y)),
            _ => None,
        }) else {
            input_index += 1;
            continue;
        };
        pending_inputs.remove(input_index);
        if lock(&mailbox.fast_pick_ids).remove(&interaction_id) {
            state.pointer_gesture = Some(PointerGesture {
                interaction_id,
                grabbed_gizmo: false,
                picked_on_down: true,
            });
            continue;
        }
        if let Some(pick) = ViewportProductionState::pointer_down(
            &mut state.render_app,
            &mut state.pointer_gesture,
            interaction_id,
            Vec2::new(x, y),
        ) {
            urgent_picks.push(pick);
        }
    }
    if !urgent_picks.is_empty() {
        lock(outputs).picks.extend(urgent_picks);
    }
}

/// Assert and log the `DirectComposition` acquire/present/discard invariants at
/// a bounded cadence.
fn log_compositor_invariants(frame_count: u64) {
    if !frame_count.is_multiple_of(300) {
        return;
    }
    let counters = gpui_windows::viewport_compositor_counters();
    let terminal_count = counters
        .presented_texture_count
        .saturating_add(counters.discarded_texture_count);
    debug_assert!(terminal_count <= counters.acquired_texture_count);
    debug_assert!(
        counters
            .acquired_texture_count
            .saturating_sub(terminal_count)
            <= 1
    );
    info!(
        ?counters,
        terminal_count, "DirectComposition viewport invariant snapshot"
    );
}

fn run_viewport_producer(
    render_app: EditorRenderApp,
    mailbox: &ProductionMailbox,
    outputs: &Mutex<ProductionOutputs>,
) {
    let mut state = ViewportProductionState::new(render_app);
    let mut config: Option<ProductionFrameConfig> = None;
    let mut pending_inputs = Vec::new();

    while !mailbox.shutdown.load(Ordering::Acquire) {
        if !mailbox.live.load(Ordering::Acquire) {
            std::thread::park();
            continue;
        }
        apply_fast_picks(&mut state, mailbox, outputs, &mut config);
        pending_inputs.append(&mut std::mem::take(&mut *lock(&mailbox.inputs)));
        apply_production_commands(&mut state, mailbox);
        drain_urgent_pointer_downs(&mut state, mailbox, outputs, &mut pending_inputs);

        let next_config = lock(&mailbox.config).take();
        if let Some(next) = next_config {
            config = Some(next);
        }
        let Some(frame) = config.as_ref() else {
            continue;
        };
        mailbox.fast_pick_enabled.store(true, Ordering::Release);
        let next_scene = lock(&mailbox.scene).take();
        if let Some(scene) = next_scene {
            state.scene = scene;
            state.scene_bridge.dirty = true;
            state.pick_snapshot_dirty = true;
        }
        let inputs = std::mem::take(&mut pending_inputs);
        let hover = lock(&mailbox.hover).take();
        state.render_app.set_render_theme(frame.render_theme);

        let (status, camera, picks, commits, asset_drops, playback) =
            state.advance_frame(frame, inputs, hover.as_ref());
        if let Some(snapshot) = state.take_refreshed_pick_snapshot() {
            *lock(&mailbox.pick_snapshot) = Some(Arc::new(snapshot));
        }
        state.finish_frame();
        log_compositor_invariants(state.frame_count);

        {
            let mut completed = lock(outputs);
            if status.is_some() {
                completed.status = status;
            }
            completed.camera = Some(camera);
            completed.picks.extend(picks);
            completed.commits.extend(commits);
            completed.asset_drops.extend(asset_drops);
            if playback.is_some() {
                completed.playback = playback;
            }
        }
    }
}

/// Build the producer only after GPUI has created the renderer-bound child
/// visual.
///
/// Preinitialized D3D12 resources move into the MTA producer thread, which
/// owns Bevy construction, acquire, render, and present.
///
/// # Errors
///
/// Returns [`ViewportBootError::CompositionVisual`] if the GPUI visual slot
/// cannot be cloned into or rebound to the producer thread,
/// [`ViewportBootError::ThreadSpawn`] if the `az-viewport-production` thread
/// cannot be spawned, [`ViewportBootError::Renderer`] if Bevy renderer
/// initialization fails on that thread,
/// [`ViewportBootError::BootThreadDisconnected`] if the thread dies before
/// reporting its boot result, or
/// [`ViewportBootError::StartHandshakeDisconnected`] if it exits before
/// acknowledging the start signal.
#[instrument(skip(slot, bootstrap))]
pub fn boot_viewport_renderer(
    product_root: Option<std::path::PathBuf>,
    slot: gpui_windows::ViewportVisualSlot,
    bootstrap: crate::editor_render::EditorRenderBootstrap,
) -> Result<EditorViewportProducer, ViewportBootError> {
    let target = slot
        .surface_target()
        .map_err(|error| ViewportBootError::CompositionVisual(error.to_string()))?;
    let producer_product_root = product_root.clone();
    let (width, height) = DEFAULT_VIEWPORT_SIZE;
    let mailbox = Arc::new(ProductionMailbox::default());
    let outputs = Arc::new(Mutex::new(ProductionOutputs::default()));
    let (boot_tx, boot_rx) = std::sync::mpsc::sync_channel(1);
    let (start_tx, start_rx) = std::sync::mpsc::sync_channel(0);
    let thread_mailbox = mailbox.clone();
    let thread_outputs = outputs.clone();
    let thread = std::thread::Builder::new()
        .name("az-viewport-production".to_owned())
        .spawn(move || {
            let _com_apartment = match ProducerComApartment::initialize() {
                Ok(apartment) => apartment,
                Err(error) => {
                    let _ = boot_tx.send(Err(error));
                    return;
                }
            };
            let render_app = match EditorRenderApp::new_dcomp_with_product_root(
                width,
                height,
                product_root,
                target,
                bootstrap,
            ) {
                Ok(render_app) => render_app,
                Err(error) => {
                    let _ = boot_tx.send(Err(ViewportBootError::Renderer(error)));
                    return;
                }
            };
            if boot_tx.send(Ok(())).is_err() {
                return;
            }
            if start_rx.recv().is_ok() {
                run_viewport_producer(render_app, &thread_mailbox, &thread_outputs);
            }
        })
        .map_err(ViewportBootError::ThreadSpawn)?;
    match boot_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = thread.join();
            return Err(error);
        }
        Err(_) => {
            let detail = thread.join().err().map_or_else(
                || "boot channel closed without a result".to_owned(),
                |payload| panic_payload_message(&*payload),
            );
            return Err(ViewportBootError::BootThreadDisconnected(detail));
        }
    }
    slot.commit()
        .map_err(|error| ViewportBootError::CompositionVisual(error.to_string()))?;
    start_tx
        .send(())
        .map_err(|_| ViewportBootError::StartHandshakeDisconnected)?;
    info!(
        width,
        height, "dedicated viewport producer created on GPUI DirectComposition child visual"
    );
    Ok(EditorViewportProducer {
        mailbox,
        outputs,
        thread: Some(thread),
        visual_slot: slot,
        product_root: producer_product_root,
    })
}

/// Install the viewport host and its lightweight GPUI mailbox poller. Call once
/// during GPUI initialization with the producer from [`boot_viewport_renderer`].
#[instrument(skip_all)]
pub fn install_viewport_host(
    cx: &mut App,
    producer: Result<EditorViewportProducer, ViewportBootError>,
) {
    gpui_windows::set_viewport_perf_sink(crate::perf::record_ns);
    gpui_windows::set_frame_presented_sink(crate::perf::visible_frame_presented);
    gpui_component::dock::set_dock_tab_activation_perf_sink(
        crate::perf::dock_tab_activation_started,
    );
    az_editor_ui::panels::set_viewport_input_trace_sink(crate::perf::record_viewport_ui_trace);
    cx.on_action(|_: &az_editor_ui::actions::PumpViewportInput, cx| {
        let _ = pump_frame(cx);
    });
    cx.on_action(|_: &az_editor_ui::actions::FrameSelected, cx| {
        cx.default_global::<EditorViewportInputQueue>()
            .push(ViewportInputEvent::FrameSelected);
        let _ = pump_frame(cx);
    });
    let producer = match producer {
        Ok(producer) => producer,
        Err(error) => {
            let diagnostic = format!("DirectComposition viewport renderer boot failed: {error}");
            tracing::error!(error = %error, "DirectComposition viewport renderer boot failed");
            cx.set_global(EditorViewportRenderStatus::unavailable(diagnostic));
            return;
        }
    };
    *lock(&ACTIVE_PRODUCTION_MAILBOX) = Some(producer.mailbox.clone());

    cx.set_global(EditorViewportCameraState::default());
    info!("installing DirectComposition viewport host");
    cx.set_global(EditorViewportHost::new(producer));

    let (display_hz, pump_hz, pump_interval) = viewport_pump_interval();
    gpui_windows::set_immediate_frame_rate_hz(pump_hz);
    cx.spawn(async move |cx| {
        let mut deadline = std::time::Instant::now();
        loop {
            let actual_wake = std::time::Instant::now();
            if actual_wake > deadline {
                crate::perf::record_ns(
                    crate::perf::FRAME_VIEWPORT_PUMP_WAKE_LATENESS,
                    crate::perf::duration_ns(actual_wake.duration_since(deadline)),
                );
            }
            if !cx.update(pump_frame) {
                break;
            }
            deadline += pump_interval;
            let now = std::time::Instant::now();
            if deadline <= now {
                // Mailbox polling never catches up: production cadence is
                // independently governed by the DXGI waitable object.
                deadline = now + pump_interval;
            }
            // Await the scheduler timer directly. `BackgroundExecutor::timer`
            // wraps this in another background task, adding a second thread-pool
            // dispatch before the foreground waker can post back to the GPUI
            // main thread.
            cx.background_executor()
                .scheduler_executor()
                .timer(deadline.saturating_duration_since(now))
                .await;
        }
    })
    .detach();
    info!(
        display_hz,
        pump_hz, "installed dedicated viewport producer and lightweight GPUI mailbox pump"
    );
}

/// Read the current primary display mode once at viewport startup. The GPUI
/// window can later move between monitors, but using the live Windows mode here
/// is still preferable to a fixed sleep and keeps the common single-display
/// editor path aligned to refresh.
fn viewport_pump_interval() -> (u32, u32, Duration) {
    use windows::Win32::Graphics::Gdi::{DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW};
    use windows::core::PCWSTR;

    let mut mode = DEVMODEW {
        // DEVMODEW is a couple of hundred bytes, so the fallback is unreachable
        // and only keeps the narrowing checked.
        dmSize: u16::try_from(std::mem::size_of::<DEVMODEW>()).unwrap_or(u16::MAX),
        ..Default::default()
    };
    let detected = unsafe {
        EnumDisplaySettingsW(PCWSTR::null(), ENUM_CURRENT_SETTINGS, &raw mut mode).as_bool()
    };
    let display_hz = if detected && (24..=1000).contains(&mode.dmDisplayFrequency) {
        mode.dmDisplayFrequency
    } else {
        MIN_MAILBOX_PUMP_HZ
    };
    // Snap nominal high-refresh modes (Windows commonly reports 239 for 240)
    // to the nearest integer divisor. This keeps 144/165 Hz panels native while
    // avoiding redundant mailbox polling on 240 Hz-class panels. Bevy's own
    // composition swapchain remains independently paced.
    let refresh_divisor = ((display_hz + MIN_MAILBOX_PUMP_HZ / 2) / MIN_MAILBOX_PUMP_HZ).max(1);
    let pump_hz = (display_hz / refresh_divisor).max(MIN_MAILBOX_PUMP_HZ);
    (
        display_hz,
        pump_hz,
        Duration::from_secs_f64(1.0 / f64::from(pump_hz)),
    )
}

/// Rebuild the producer after a GPUI device loss invalidated the visual slot.
/// Returns `true` when the pump should skip this tick because the rebuild was
/// deferred to a later retry.
fn rebuild_after_device_loss(cx: &mut App) -> bool {
    let rebuild = cx.update_global::<EditorViewportHost, _>(|host, _| {
        host.prepare_device_loss_rebuild(std::time::Instant::now())
    });
    let Some(rebuild) = rebuild else {
        return false;
    };
    let replacement = crate::editor_render::EditorRenderBootstrap::initialize()
        .map_err(ViewportBootError::Renderer)
        .and_then(|bootstrap| {
            gpui_windows::viewport_visual_slot(rebuild.window_id)
                .ok_or(ViewportBootError::MissingCompositionVisual)
                .and_then(|slot| boot_viewport_renderer(rebuild.product_root, slot, bootstrap))
        });
    match replacement {
        Ok(producer) => {
            let mailbox = producer.mailbox.clone();
            cx.update_global::<EditorViewportHost, _>(|host, _| {
                host.replace_after_device_loss(producer);
            });
            *lock(&ACTIVE_PRODUCTION_MAILBOX) = Some(mailbox);
            cx.set_global(EditorViewportRenderStatus::waiting());
            info!("rebuilt DirectComposition viewport producer after GPUI device loss");
            false
        }
        Err(error) => {
            warn!(%error, "DirectComposition viewport device-loss rebuild deferred");
            true
        }
    }
}

/// Collect the theme, selection, tool, and owner-scoped preview state this
/// frame publishes, together with the drained semantic input queue.
fn production_frame_config(
    cx: &mut App,
    panel_frame: &EditorViewportPanelFrame,
) -> (
    ProductionFrameConfig,
    Vec<ViewportInputEvent>,
    Option<ViewportInputEvent>,
) {
    let hidden = cx
        .try_global::<EditorLayerVisibility>()
        .map(|visibility| visibility.hidden.clone())
        .unwrap_or_default();
    let selection = current_authored_selection(cx);
    let accent = Rgba::from(cx.theme().accent);
    let selection_accent = [accent.r, accent.g, accent.b, accent.a];
    let render_theme = ViewportRenderTheme {
        background: rgba_array(Rgba::from(cx.theme().background)),
        skybox: rgba_array(Rgba::from(cx.theme().muted)),
        grid_minor: rgba_array(Rgba::from(cx.theme().muted_foreground.opacity(0.28))),
        grid_major: rgba_array(Rgba::from(cx.theme().accent.opacity(0.42))),
    };
    let gizmo = cx.try_global::<EditorSceneToolState>().map_or(
        (
            GizmoMode::None,
            GizmoPivot::Pivot,
            GizmoSpace::World,
            GizmoSnap::NONE,
        ),
        gizmo_config_from_tool_state,
    );
    let (mut inputs, hover) = {
        let queue = cx.default_global::<EditorViewportInputQueue>();
        (queue.drain(), queue.take_hover())
    };
    // Pointer-down was forwarded directly from the allocation-free UI trace
    // sink, before GPUI action dispatch. Do not replay it through this mailbox
    // synchronization pass.
    inputs.retain(|input| !matches!(input, ViewportInputEvent::PointerDown { .. }));
    // The frame's owner tag selects the pump's content: the mannequin/
    // blend-space previews render only while an Animation-mode panel owns the
    // surface; Scene-owned surfaces render the authored scene bridge.
    let previews = animation_previews_for_owner(
        panel_frame.owner,
        cx.try_global::<EditorMannequinPreview>().cloned(),
        cx.try_global::<EditorBlendSpacePreview>().cloned(),
    );
    (
        ProductionFrameConfig {
            layout: panel_frame.layout,
            hidden,
            selection,
            selection_accent,
            render_theme,
            gizmo,
            previews,
        },
        inputs,
        hover,
    )
}

/// Publish the producer's status and camera readings, reporting whether either
/// actually changed what the UI shows.
fn publish_status_and_camera(
    cx: &mut App,
    status: Option<EditorViewportRenderStatus>,
    camera: Option<EditorViewportCameraState>,
) -> (bool, bool) {
    let status_changed = status.as_ref().is_some_and(|status| {
        cx.try_global::<EditorViewportRenderStatus>()
            .is_none_or(|current| !render_status_ui_eq(current, status))
    });
    if let Some(status) = status.filter(|_| status_changed) {
        cx.set_global(status);
    }
    let camera_changed = camera.as_ref().is_some_and(|camera| {
        cx.try_global::<EditorViewportCameraState>()
            .is_none_or(|current| current != camera)
    });
    if let Some(camera) = camera.filter(|_| camera_changed) {
        cx.set_global(camera);
    }
    (status_changed, camera_changed)
}

/// Publish the read-back playhead so the timeline advances while playing;
/// scrubbing while paused stays authoritative (no read-back then).
fn publish_playback(cx: &mut App, playback: Option<MannequinPlaybackStatus>) -> bool {
    let playback_changed = playback.is_some();
    if let Some(playback) = playback
        && cx.try_global::<EditorMannequinPreview>().is_some()
    {
        cx.update_global::<EditorMannequinPreview, _>(|preview, _| {
            if preview.playing {
                preview.position_millis = playback.position_millis;
                if playback.finished && !preview.looping {
                    preview.playing = false;
                }
            }
        });
    }
    playback_changed
}

/// Persist the optimistic asset placements the producer reported, reporting
/// whether any reached the authored path.
fn apply_asset_drops(cx: &mut App, asset_drops: Vec<AssetDropPlacement>) -> bool {
    let active_level_document_id = cx
        .try_global::<EditorActiveLevel>()
        .and_then(|active| active.document_id.as_deref());
    let asset_drop_document = cx
        .try_global::<EditorAuthoredOutline>()
        .and_then(|outline| asset_drop_document_id(active_level_document_id, outline));
    let had_asset_drops = !asset_drops.is_empty();
    for placement in asset_drops {
        let Some(document_id) = asset_drop_document.clone() else {
            warn!(
                source = %placement.source_path,
                "viewport asset drop has no unambiguous loaded prefab document"
            );
            continue;
        };
        commit_asset_drop(cx, document_id, placement);
    }
    had_asset_drops
}

/// Route the producer's picks into the authored-selection → inspector path,
/// reporting whether any pick was dispatched.
fn apply_picks(cx: &mut App, picks: Vec<PendingViewportPick>) -> bool {
    let had_picks = !picks.is_empty();
    for pick in picks {
        let PendingViewportPick {
            interaction_id,
            hit: pick,
        } = pick;
        let ViewportPickHit {
            id,
            document_id: Some(document_id),
            object_id: Some(object_id),
        } = pick
        else {
            // Placeholder primitives (ground, neutral scene) have no authored
            // backing; a click on them is not a selection change.
            continue;
        };
        let payload = crate::perf::stable_payload(&id);
        crate::perf::selection_dispatched(interaction_id, payload);
        info!(pick = %id, document = %document_id, object = %object_id, "viewport pick");
        if let Err(error) = crate::authored_selection::select_reflected_entity(
            cx,
            document_id.clone(),
            object_id.clone(),
        ) {
            warn!(
                %error,
                document = %document_id,
                object = %object_id,
                "viewport pick could not select authored object"
            );
        }
    }
    had_picks
}

/// Which of the pump's published readings actually changed this tick.
#[derive(Clone, Copy, Debug, Default)]
struct PublishedChanges {
    status: bool,
    camera: bool,
    playback: bool,
}

impl PublishedChanges {
    const fn any(self) -> bool {
        self.status || self.camera || self.playback
    }
}

/// Repaint as narrowly as this frame's changes allow: a broad content change
/// refreshes every window, while a status/camera/playback change only notifies
/// the retained panels that display it.
fn notify_changed_panels(cx: &mut App, broad_ui_changed: bool, changes: PublishedChanges) {
    if broad_ui_changed {
        cx.refresh_windows();
        return;
    }
    if !changes.any() {
        return;
    }
    let viewport_notified =
        crate::workspace::dock::notify_cached_panel::<ViewportPanel>(ViewportPanel::NAME, cx);
    if changes.status {
        let _ =
            crate::workspace::dock::notify_cached_panel::<ProfilerPanel>(ProfilerPanel::NAME, cx);
    }
    if changes.playback {
        let _ = crate::workspace::dock::notify_cached_panel::<AnimationMannequinPanel>(
            AnimationMannequinPanel::NAME,
            cx,
        );
    }
    if !viewport_notified {
        // The workspace may still be booting and therefore have no retained
        // viewport panel. Preserve correctness until it exists.
        cx.refresh_windows();
    }
}

/// One frame pump tick. Returns `false` to stop the pump (host gone).
fn pump_frame(cx: &mut App) -> bool {
    if cx.try_global::<EditorViewportHost>().is_none() {
        return false;
    }
    if rebuild_after_device_loss(cx) {
        return true;
    }

    // The viewport panel publishes its device-pixel geometry every paint. A
    // stale frame means the producer should park; the GPUI renderer separately
    // hides the sibling visual when no composition hole appears in its scene.
    let panel_frame = cx
        .try_global::<EditorViewportPanelFrame>()
        .cloned()
        .unwrap_or_default();
    if !panel_frame.is_fresh(PANEL_FRAME_MAX_AGE) {
        cx.update_global::<EditorViewportHost, _>(|host, _| host.producer.set_live(false));
        return true;
    }

    maybe_start_scene_fetch(cx);
    let (config, inputs, hover) = production_frame_config(cx, &panel_frame);
    let ProductionOutputs {
        status,
        camera,
        picks,
        commits,
        asset_drops,
        playback,
    } = cx.update_global::<EditorViewportHost, _>(|host, _| {
        host.producer.set_live(panel_frame.layout.visible);
        if host.last_config.as_ref() != Some(&config) {
            host.producer.publish_config(config.clone());
            host.last_config = Some(config);
        }
        host.producer.publish_inputs(inputs, hover);
        host.producer.take_outputs()
    });

    let (status_changed, camera_changed) = publish_status_and_camera(cx, status, camera);
    let playback_changed = publish_playback(cx, playback);

    let had_commits = !commits.is_empty();
    for commit in commits {
        commit_gizmo_transform(cx, commit);
    }
    let had_asset_drops = apply_asset_drops(cx, asset_drops);
    let had_picks = apply_picks(cx, picks);

    notify_changed_panels(
        cx,
        had_commits || had_asset_drops || had_picks,
        PublishedChanges {
            status: status_changed,
            camera: camera_changed,
            playback: playback_changed,
        },
    );
    true
}

fn render_status_ui_eq(
    left: &EditorViewportRenderStatus,
    right: &EditorViewportRenderStatus,
) -> bool {
    left.state == right.state
        && left.width == right.width
        && left.height == right.height
        && left.format == right.format
        && left.backend == right.backend
        && left.diagnostic == right.diagnostic
        && left.telemetry == right.telemetry
}

fn current_authored_selection(cx: &App) -> Option<(String, String)> {
    if let Some(selection) = cx
        .try_global::<EditorAuthoredOutline>()
        .and_then(|outline| {
            outline.data.documents.iter().find_map(|document| {
                document.objects.iter().find_map(|object| {
                    object
                        .selected
                        .then(|| (document.document_id.clone(), object.object_id.clone()))
                })
            })
        })
    {
        return Some(selection);
    }
    let selection = &cx
        .try_global::<EditorReflectedSelectionState>()?
        .current()?
        .selection;
    Some((
        selection.source_path.clone(),
        selection.entity_alias.clone(),
    ))
}

/// Select the root prefab document owned by the active level for an asset drop.
#[must_use]
fn asset_drop_document_id(
    active_level_document_id: Option<&str>,
    outline: &EditorAuthoredOutline,
) -> Option<String> {
    active_level_prefab_documents(&outline.data, active_level_document_id)
        .into_iter()
        .next()
        .filter(|document| document.loaded && document.valid)
        .map(|document| document.document_id.clone())
}

/// `(document_id, revision)` of the active level's root prefab and unique
/// nested prefab dependencies, in hierarchy traversal order.
fn scene_document_signature(
    outline: &EditorAuthoredOutline,
    active_level_document_id: Option<&str>,
) -> Vec<(String, u64)> {
    active_level_prefab_documents(&outline.data, active_level_document_id)
        .into_iter()
        .filter(|document| document.loaded && document.valid)
        .map(|document| (document.document_id.clone(), document.revision))
        .collect()
}

/// Start an async snapshot fetch when the outline's scene-document signature
/// diverges from the applied scene, publishing the translated [`ViewportScene`]
/// back into the host when it completes.
fn maybe_start_scene_fetch(cx: &mut App) {
    let Some(outline) = cx.try_global::<EditorAuthoredOutline>() else {
        return;
    };
    let active_level_document_id = cx
        .try_global::<EditorActiveLevel>()
        .and_then(|active| active.document_id.as_deref());
    let signature = scene_document_signature(outline, active_level_document_id);

    let should_fetch = cx.update_global::<EditorViewportHost, _>(|host, _| {
        if host.scene_signature == signature || host.fetch_signature.as_ref() == Some(&signature) {
            false
        } else {
            host.fetch_signature = Some(signature.clone());
            true
        }
    });
    if !should_fetch {
        return;
    }

    let Ok(attached) = crate::controller_set::reflected_selection_controller(cx) else {
        cx.update_global::<EditorViewportHost, _>(|host, _| host.fetch_signature = None);
        return;
    };
    let fence = attached.fence;
    let controller = attached.controller;

    let worker_signature = signature.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "viewport-scene-fetch",
        move || async move {
            let registry = controller.edit_session().type_registry_snapshot().await?;
            let mut sources = Vec::new();
            for (source_path, _) in &worker_signature {
                let result = controller
                    .edit_session()
                    .source_snapshot(source_path)
                    .await?;
                let snapshot = result.snapshot.ok_or_else(|| {
                    EditorError::InvalidArgument(format!(
                        "project host returned no snapshot for `{source_path}`"
                    ))
                })?;
                sources.push((source_path.clone(), snapshot));
            }
            Ok((registry, sources))
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            if cx.try_global::<EditorViewportHost>().is_none() {
                return;
            }
            match result {
                Ok((registry, snapshots)) => {
                    let scene = viewport_scene_from_snapshots(registry, snapshots);
                    cx.update_global::<EditorViewportHost, _>(|host, _| {
                        // Only apply if a newer fetch has not superseded this one.
                        if host.fetch_signature.as_ref() == Some(&signature) {
                            info!(
                                documents = signature.len(),
                                objects = scene.entity_count(),
                                "viewport authored-scene bridge updated"
                            );
                            host.producer.publish_scene(scene.clone());
                            host.scene = scene;
                            host.scene_signature.clone_from(&signature);
                            host.fetch_signature = None;
                        }
                    });
                }
                Err(error) => {
                    warn!(%error, "viewport scene snapshot fetch failed");
                    cx.update_global::<EditorViewportHost, _>(|host, _| {
                        if host.fetch_signature.as_ref() == Some(&signature) {
                            host.fetch_signature = None;
                        }
                    });
                }
            }
        },
    );
}

/// Preserve authored snapshots for canonical lowering on the production
/// thread. No component values are interpreted on the GPUI thread.
#[must_use]
pub fn viewport_scene_from_snapshots(
    registry: TypeRegistrySnapshot,
    snapshots: Vec<(String, PrefabSourceSnapshot)>,
) -> ViewportScene {
    ViewportScene {
        registry,
        sources: snapshots
            .into_iter()
            .map(|(source_path, snapshot)| ViewportSourceSnapshot {
                source_path,
                snapshot,
            })
            .collect(),
    }
}

/// Which mannequin/blend-space previews the pump applies for the live
/// surface's owner.
///
/// Pure decision: Animation-owned surfaces render the previews (falling back
/// to empty previews until a project attaches); Scene-owned surfaces never
/// render the mannequin — the mannequin globals default to a selected
/// character on attach, so gating on the surface owner is what keeps Scene
/// mode mannequin-free.
#[must_use]
pub fn animation_previews_for_owner(
    owner: ViewportSurfaceOwner,
    mannequin: Option<EditorMannequinPreview>,
    blend_space: Option<EditorBlendSpacePreview>,
) -> Option<(EditorMannequinPreview, EditorBlendSpacePreview)> {
    match owner {
        ViewportSurfaceOwner::Animation => Some((
            mannequin.unwrap_or_else(EditorMannequinPreview::empty),
            blend_space.unwrap_or_else(EditorBlendSpacePreview::empty),
        )),
        ViewportSurfaceOwner::Scene => None,
    }
}

/// Map the editor scene tool state onto the viewport gizmo config: mode from the
/// active tool (Select detaches the gizmo), transform space, and per-channel
/// snap steps (enabled toggles gate the step). Pure.
#[must_use]
fn gizmo_config_from_tool_state(
    tool_state: &EditorSceneToolState,
) -> (GizmoMode, GizmoPivot, GizmoSpace, GizmoSnap) {
    let mode = match tool_state.tool {
        EditorSceneToolKind::Select => GizmoMode::None,
        EditorSceneToolKind::Move => GizmoMode::Translate,
        EditorSceneToolKind::Rotate => GizmoMode::Rotate,
        EditorSceneToolKind::Scale => GizmoMode::Scale,
        EditorSceneToolKind::Transform => GizmoMode::Universal,
    };
    let space = match tool_state.space {
        EditorSceneTransformSpace::World => GizmoSpace::World,
        EditorSceneTransformSpace::Local => GizmoSpace::Local,
    };
    let pivot = match tool_state.pivot {
        EditorScenePivot::Pivot => GizmoPivot::Pivot,
        EditorScenePivot::Center => GizmoPivot::Center,
    };
    let snap = GizmoSnap {
        translate_step_meters: tool_state
            .grid_snap
            .enabled
            .then_some(tool_state.grid_snap.step_meters),
        rotate_step_degrees: tool_state
            .angle_snap
            .enabled
            .then_some(tool_state.angle_snap.degrees),
    };
    (mode, pivot, space, snap)
}

fn reflected_component_binding(
    registry: &TypeRegistrySnapshot,
    entity_alias: &str,
    component_type_path: &str,
    field_name: &str,
) -> Option<(ReflectedEditBinding, String)> {
    let descriptor = registry
        .types
        .iter()
        .find(|descriptor| descriptor.type_path == component_type_path)?;
    let field = descriptor
        .fields
        .iter()
        .find(|field| field.name == field_name)?;
    let binding = ReflectedEditBinding::new(PrefabValueTarget {
        instance_alias_chain: Vec::new(),
        entity_alias: entity_alias.to_owned(),
        path: ReflectedPath {
            component_type_path: component_type_path.to_owned(),
            segments: Vec::new(),
        },
    })
    .field(field_name);
    Some((binding, field.type_path.clone()))
}

fn gizmo_path_edit(
    registry: &TypeRegistrySnapshot,
    snapshot: &PrefabSourceSnapshot,
    entity_alias: &str,
    value: GizmoCommitValue,
) -> Option<(PrefabEditCommand, &'static str)> {
    let (field_name, payload) = match value {
        GizmoCommitValue::Position(value) => ("position", ron::ser::to_string(&value).ok()?),
        GizmoCommitValue::Rotation(value) => ("rotation", ron::ser::to_string(&value).ok()?),
        GizmoCommitValue::Scale(value) => ("scale", ron::ser::to_string(&value).ok()?),
    };
    let (binding, type_path) = reflected_component_binding(
        registry,
        entity_alias,
        std::any::type_name::<az_transform::Transform>(),
        field_name,
    )?;
    snapshot
        .components
        .iter()
        .any(|component| {
            component.entity_alias == entity_alias
                && component.type_path == std::any::type_name::<az_transform::Transform>()
        })
        .then_some(())?;
    Some((
        binding.set_value(ReflectedValueEnvelope {
            type_path,
            encoding: ReflectedValueEncoding::TypedRon,
            payload: payload.into_bytes(),
        }),
        field_name,
    ))
}

fn replace_viewport_source_snapshot(
    cx: &mut App,
    source_path: &str,
    snapshot: PrefabSourceSnapshot,
) {
    if cx.try_global::<EditorViewportHost>().is_none() {
        return;
    }
    cx.update_global::<EditorViewportHost, _>(|host, _| {
        if let Some(source) = host
            .scene
            .sources
            .iter_mut()
            .find(|source| source.source_path == source_path)
        {
            source.snapshot = snapshot;
            host.producer.publish_scene(host.scene.clone());
        }
    });
}

fn reconcile_cached_viewport_scene(cx: &mut App) {
    if cx.try_global::<EditorViewportHost>().is_some() {
        cx.update_global::<EditorViewportHost, _>(|host, _| {
            host.producer.publish_scene(host.scene.clone());
        });
    }
}

/// Persist a gizmo commit through the same reflected Prefab command path as the inspector.
fn commit_gizmo_transform(cx: &mut App, commit: GizmoCommit) {
    let attached = match crate::controller_set::reflected_selection_controller(cx) {
        Ok(attached) => attached,
        Err(error) => {
            warn!(%error, "viewport gizmo commit skipped: reflected selection unavailable");
            return;
        }
    };
    let fence = attached.fence;
    let controller = attached.controller;
    let source_path = commit.document_id.clone();
    let entity_alias = commit.object_id.clone();
    let commit_value = commit.value;
    let log_document = commit.document_id;

    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "viewport-gizmo-commit",
        move || async move {
            let edit_session = controller.edit_session();
            let registry = edit_session.type_registry_snapshot().await?;
            let current = edit_session.source_snapshot(&source_path).await?;
            let snapshot = current.snapshot.ok_or_else(|| {
                EditorError::InvalidArgument(format!(
                    "project host returned no snapshot for `{source_path}`"
                ))
            })?;
            let Some((command, channel)) =
                gizmo_path_edit(&registry, &snapshot, &entity_alias, commit_value)
            else {
                return Err(EditorError::InvalidArgument(format!(
                    "Prefab entity `{entity_alias}` has no compatible reflected transform field for the gizmo commit"
                )));
            };
            let result = edit_session
                .apply(&ReflectedPrefabEdit::new(
                    source_path.clone(),
                    snapshot.revision,
                    command,
                ))
                .await?;
            let updated = result.snapshot.ok_or_else(|| {
                EditorError::InvalidArgument(format!(
                    "project host returned no edited snapshot for `{source_path}`"
                ))
            })?;
            let inspection = controller
                .inspect(ReflectedPrefabSelection::new(
                    source_path.clone(),
                    entity_alias,
                ))
                .await?;
            Ok((inspection, updated, channel))
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((inspection, snapshot, channel)) => {
                    replace_viewport_source_snapshot(cx, &log_document, snapshot);
                    crate::authored_selection::publish_reflected_inspection(cx, inspection);
                    info!(document = %log_document, channel, "persisted viewport gizmo transform");
                }
                Err(error) => {
                    // The Bevy transform changed optimistically during the drag.
                    // Reapply the authoritative cached snapshot if persistence did
                    // not succeed so unsupported channels never remain visible.
                    reconcile_cached_viewport_scene(cx);
                    warn!(%error, document = %log_document, "viewport gizmo transform commit failed");
                }
            }
        },
    );
}

#[derive(Debug, Clone, PartialEq)]
struct AssetDropPlan {
    mesh_product_path: String,
    position: Vec3,
}

fn asset_drop_plan(source_path: &str, position: Vec3) -> AssetDropPlan {
    let product_path = az_mesh_builder::mesh_product_path(source_path);
    AssetDropPlan {
        mesh_product_path: product_path,
        position,
    }
}

fn asset_drop_commands(
    registry: &TypeRegistrySnapshot,
    entity_alias: &str,
    plan: &AssetDropPlan,
) -> Result<Vec<PrefabEditCommand>, EditorError> {
    let transform_type = std::any::type_name::<az_transform::Transform>();
    let mesh_type = std::any::type_name::<az_render::Mesh>();
    let (position_binding, position_type) =
        reflected_component_binding(registry, entity_alias, transform_type, "position")
            .ok_or_else(|| {
                EditorError::InvalidArgument(
                    "reflected Transform.position is not registered".into(),
                )
            })?;
    let (mesh_binding, mesh_path_type) =
        reflected_component_binding(registry, entity_alias, mesh_type, "mesh").ok_or_else(
            || EditorError::InvalidArgument("reflected Mesh.mesh is not registered".into()),
        )?;
    let position = ron::ser::to_string(&plan.position).map_err(|error| {
        EditorError::InvalidArgument(format!("failed to encode dropped position: {error}"))
    })?;
    let mesh_path = ron::ser::to_string(&plan.mesh_product_path).map_err(|error| {
        EditorError::InvalidArgument(format!("failed to encode dropped mesh path: {error}"))
    })?;
    Ok(vec![
        PrefabEditCommand::AddEntity {
            alias: entity_alias.to_owned(),
            parent_alias: None,
        },
        PrefabEditCommand::AddComponent {
            entity_alias: entity_alias.to_owned(),
            component_type_path: transform_type.to_owned(),
            initial_value: None,
        },
        position_binding.set_value(ReflectedValueEnvelope {
            type_path: position_type,
            encoding: ReflectedValueEncoding::TypedRon,
            payload: position.into_bytes(),
        }),
        PrefabEditCommand::AddComponent {
            entity_alias: entity_alias.to_owned(),
            component_type_path: mesh_type.to_owned(),
            initial_value: None,
        },
        mesh_binding.set_value(ReflectedValueEnvelope {
            type_path: mesh_path_type,
            encoding: ReflectedValueEncoding::TypedRon,
            payload: mesh_path.into_bytes(),
        }),
    ])
}

/// Commit a viewport asset drop through vNext Prefab structural commands.
fn commit_asset_drop(cx: &mut App, source_path: String, placement: AssetDropPlacement) {
    let attached = match crate::controller_set::reflected_selection_controller(cx) {
        Ok(attached) => attached,
        Err(error) => {
            warn!(%error, "viewport asset drop skipped: reflected selection unavailable");
            return;
        }
    };
    let fence = attached.fence;
    let controller = attached.controller;
    let plan = asset_drop_plan(&placement.source_path, placement.position);
    let interaction_id = placement.interaction_id;
    let log_document = source_path.clone();
    let log_source = placement.source_path;

    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "viewport-asset-drop",
        move || async move {
            let session = controller.edit_session();
            let registry = session.type_registry_snapshot().await?;
            let current = session.source_snapshot(&source_path).await?;
            let mut snapshot = current.snapshot.ok_or_else(|| {
                EditorError::InvalidArgument(format!(
                    "project host returned no snapshot for `{source_path}`"
                ))
            })?;
            let entity_alias = format!("entity-{}", uuid::Uuid::now_v7().simple());
            for command in asset_drop_commands(&registry, &entity_alias, &plan)? {
                let result = session
                    .apply(&ReflectedPrefabEdit::new(
                        source_path.clone(),
                        snapshot.revision,
                        command,
                    ))
                    .await?;
                snapshot = result.snapshot.ok_or_else(|| {
                    EditorError::InvalidArgument(format!(
                        "project host returned no edited snapshot for `{source_path}`"
                    ))
                })?;
            }
            let inspection = controller
                .inspect(ReflectedPrefabSelection::new(
                    source_path.clone(),
                    entity_alias.clone(),
                ))
                .await?;
            Ok((inspection, snapshot, entity_alias))
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((inspection, snapshot, entity_alias)) => {
                    if cx.try_global::<EditorViewportHost>().is_some() {
                        cx.update_global::<EditorViewportHost, _>(|host, _| {
                            host.producer.command(ProductionCommand::MarkAssetAuthored {
                                interaction_id,
                                authored_object_id: entity_alias,
                            });
                        });
                    }
                    replace_viewport_source_snapshot(cx, &log_document, snapshot);
                    crate::authored_selection::publish_reflected_inspection(cx, inspection);
                    info!(
                        document = %log_document,
                        source = %log_source,
                        "persisted viewport asset drop"
                    );
                }
                Err(error) => {
                    warn!(
                        %error,
                        document = %log_document,
                        source = %log_source,
                        "viewport asset drop commit failed"
                    );
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_editor_ui::panels::{
        AuthoredDocumentOutlineData, AuthoredObjectOutlineData, AuthoredOutlineData,
    };

    fn outline_document(
        document_id: &str,
        schema_type: &str,
        loaded: bool,
        valid: bool,
        prefab_source_path: Option<&str>,
    ) -> AuthoredDocumentOutlineData {
        AuthoredDocumentOutlineData {
            document_id: document_id.to_string(),
            source_path: document_id.to_string(),
            schema_type: schema_type.to_string(),
            revision: 1,
            saved_revision: Some(1),
            unsaved_changes: false,
            object_count: 1,
            journal_entry_count: 0,
            loaded,
            valid,
            diagnostic: String::new(),
            objects: vec![AuthoredObjectOutlineData {
                object_id: format!("{document_id}:root"),
                schema_type: schema_type.to_string(),
                selected: false,
                display_name: None,
                prefab_parent_entity_object_id: None,
                prefab_component_object_ids: Vec::new(),
                prefab_owner_entity_object_id: None,
                prefab_source_path: prefab_source_path.map(str::to_owned),
            }],
        }
    }

    #[test]
    fn active_level_drives_viewport_and_asset_drop_document_resolution() {
        let outline = EditorAuthoredOutline::new(AuthoredOutlineData {
            documents: vec![
                outline_document(
                    "scenes/a.scene.ron",
                    az_prefab::SCENE_SOURCE_TYPE,
                    true,
                    true,
                    Some("prefabs/a.prefab.ron"),
                ),
                outline_document(
                    "scenes/b.scene.ron",
                    az_prefab::SCENE_SOURCE_TYPE,
                    true,
                    true,
                    Some("prefabs/b.prefab.ron"),
                ),
                outline_document(
                    "prefabs/a.prefab.ron",
                    az_prefab::PREFAB_SOURCE_TYPE,
                    true,
                    true,
                    None,
                ),
                outline_document(
                    "prefabs/b.prefab.ron",
                    az_prefab::PREFAB_SOURCE_TYPE,
                    true,
                    true,
                    None,
                ),
                outline_document(
                    "prefabs/standalone.prefab.ron",
                    az_prefab::PREFAB_SOURCE_TYPE,
                    true,
                    true,
                    None,
                ),
            ],
        });

        assert_eq!(
            asset_drop_document_id(Some("scenes/a.scene.ron"), &outline).as_deref(),
            Some("prefabs/a.prefab.ron")
        );
        assert_eq!(
            asset_drop_document_id(Some("scenes/b.scene.ron"), &outline).as_deref(),
            Some("prefabs/b.prefab.ron")
        );
        assert_eq!(
            scene_document_signature(&outline, Some("scenes/a.scene.ron")),
            vec![("prefabs/a.prefab.ron".to_owned(), 1)]
        );
        assert_eq!(asset_drop_document_id(None, &outline), None);
        assert!(scene_document_signature(&outline, None).is_empty());
    }

    #[test]
    fn scene_owned_surfaces_never_apply_the_mannequin_preview() {
        // The mannequin globals default to a selected character on attach; the
        // Scene-owned surface must still render the authored scene only.
        let populated = EditorMannequinPreview::default_for_project_asset_root("assets");
        assert!(populated.character_glb.is_some());
        assert_eq!(
            animation_previews_for_owner(
                ViewportSurfaceOwner::Scene,
                Some(populated),
                Some(EditorBlendSpacePreview::empty()),
            ),
            None
        );
    }

    #[test]
    fn animation_owned_surfaces_apply_previews_with_empty_fallbacks() {
        let populated = EditorMannequinPreview::default_for_project_asset_root("assets");
        let (mannequin, blend_space) = animation_previews_for_owner(
            ViewportSurfaceOwner::Animation,
            Some(populated.clone()),
            None,
        )
        .expect("animation owner applies previews");
        assert_eq!(mannequin, populated);
        assert_eq!(blend_space, EditorBlendSpacePreview::empty());

        // No globals yet (before attach): still Some, with empty previews so
        // the render app keeps its neutral scene.
        let (mannequin, _) =
            animation_previews_for_owner(ViewportSurfaceOwner::Animation, None, None)
                .expect("animation owner applies previews");
        assert_eq!(mannequin, EditorMannequinPreview::empty());
    }

    #[test]
    fn gizmo_config_maps_tools_spaces_and_snaps() {
        let mut state = EditorSceneToolState::default();
        // Defaults: Move / World / grid+angle snap enabled.
        let (mode, pivot, space, snap) = gizmo_config_from_tool_state(&state);
        assert_eq!(mode, GizmoMode::Translate);
        assert_eq!(pivot, GizmoPivot::Pivot);
        assert_eq!(space, GizmoSpace::World);
        assert_eq!(snap.translate_step_meters, Some(0.5));
        assert_eq!(snap.rotate_step_degrees, Some(15.0));

        state.set_tool(EditorSceneToolKind::Select);
        assert_eq!(gizmo_config_from_tool_state(&state).0, GizmoMode::None);
        state.set_tool(EditorSceneToolKind::Rotate);
        assert_eq!(gizmo_config_from_tool_state(&state).0, GizmoMode::Rotate);
        state.set_tool(EditorSceneToolKind::Scale);
        assert_eq!(gizmo_config_from_tool_state(&state).0, GizmoMode::Scale);
        state.set_tool(EditorSceneToolKind::Transform);
        assert_eq!(gizmo_config_from_tool_state(&state).0, GizmoMode::Universal);

        // Disabling the snap toggles clears the steps.
        state.set_space(EditorSceneTransformSpace::Local);
        state.toggle_grid_snap();
        state.toggle_angle_snap();
        state.set_pivot(EditorScenePivot::Center);
        let (_, pivot, space, snap) = gizmo_config_from_tool_state(&state);
        assert_eq!(pivot, GizmoPivot::Center);
        assert_eq!(space, GizmoSpace::Local);
        assert_eq!(snap.translate_step_meters, None);
        assert_eq!(snap.rotate_step_degrees, None);
    }

    #[test]
    fn invalidated_visual_slot_rebuilds_with_bounded_retry() {
        let now = std::time::Instant::now();
        assert!(!device_loss_rebuild_due(true, None, now));
        assert!(device_loss_rebuild_due(false, None, now));
        assert!(!device_loss_rebuild_due(
            false,
            Some(now),
            (now + DEVICE_LOSS_REBUILD_RETRY)
                .checked_sub(Duration::from_millis(1))
                .unwrap(),
        ));
        assert!(device_loss_rebuild_due(
            false,
            Some(now),
            now + DEVICE_LOSS_REBUILD_RETRY,
        ));
    }
}
