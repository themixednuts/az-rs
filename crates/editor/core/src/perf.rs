//! Lightweight editor interaction and frame timing aggregation.
//!
//! Recording accepts static stage names and writes into fixed-capacity storage:
//! hot paths do not allocate or emit per-event tracing. Summary construction is
//! explicitly on-demand and may allocate while sorting retained samples.

use std::fmt::Write as _;
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

const MAX_STAGES: usize = 64;
const SAMPLES_PER_STAGE: usize = 4096;
const MAX_PENDING_INTERACTIONS: usize = 8;
const CHROME_INTERACTIONS_PER_DUMP: u64 = 16;
const MAX_PENDING_SELECTIONS: usize = 64;
const MAX_PENDING_DRAGS: usize = 64;
const INPUT_TRACE_CAPACITY: usize = 8192;
const INPUT_TRACE_DUMP_ROWS: usize = 1024;

pub const SELECTION_POINTER_TO_PICK: &str = "selection.pointer_down_to_pick_resolved";
pub const SELECTION_PICK_TO_DISPATCH: &str = "selection.pick_resolved_to_dispatch";
pub const SELECTION_DISPATCH_TO_PUBLISH: &str = "selection.dispatch_to_authored_published";
pub const SELECTION_PUBLISH_TO_VISIBLE: &str = "selection.authored_published_to_visible_frame";
pub const SELECTION_POINTER_TO_VISIBLE: &str = "selection.pointer_down_to_visible_frame";
pub const SELECTION_POINTER_TO_HIGHLIGHT: &str = "selection.pointer_down_to_highlight_published";
pub const SELECTION_HIGHLIGHT_TO_VISIBLE: &str = "selection.highlight_published_to_visible_frame";
pub const ACTIVITY_MODE_TO_WORKSPACE: &str = "ui.activity_mode_to_workspace_swapped";
pub const TOOLBAR_TO_VISIBLE: &str = "ui.toolbar_toggle_to_visible";
pub const DOCK_TAB_TO_VISIBLE: &str = "ui.dock_tab_to_visible";
pub const DOCK_TAB_TO_ASSET_BROWSER: &str = "ui.dock_tab_to_visible.asset_browser";
pub const DOCK_TAB_TO_CONSOLE: &str = "ui.dock_tab_to_visible.console";
pub const DOCK_TAB_TO_OUTPUT_LOG: &str = "ui.dock_tab_to_visible.output_log";
pub const DOCK_TAB_TO_PROFILER: &str = "ui.dock_tab_to_visible.profiler";
pub const DOCK_TAB_TO_GEMS: &str = "ui.dock_tab_to_visible.gems";
pub const POPOVER_TO_VISIBLE: &str = "ui.popover_open_to_visible";
pub const FRAME_GPUI_RENDER: &str = "frame.gpui_render";
pub const FRAME_GPUI_PAINT: &str = "frame.gpui_paint";
pub const FRAME_BEVY_TICK: &str = "frame.bevy_render_app_tick";
pub const FRAME_BEVY_MAIN: &str = "frame.bevy_main_world";
pub const FRAME_BEVY_EXTRACT: &str = "frame.bevy_render_extract";
pub const FRAME_BEVY_RENDER: &str = "frame.bevy_render_schedule";
pub const FRAME_BEVY_RESIZE: &str = "frame.bevy_target_resize";
pub const FRAME_VIEWPORT_PUMP_INTERVAL: &str = "frame.viewport_pump_interval";
pub const FRAME_VIEWPORT_PUMP_WAKE_LATENESS: &str = "frame.viewport_pump_wake_lateness";
pub const CAMERA_DRAG_FRAME: &str = "camera_drag.production_frame";
pub const CAMERA_DRAG_EVENT: &str = "camera_drag.queued_pointer_event";
pub const CAMERA_DRAG_CURSOR_SAMPLE_TO_APPLY: &str = "camera_drag.cursor_sample_to_apply";
pub const CAMERA_DRAG_POINTER_TO_BEVY_PRESENT: &str = "camera_drag.pointer_sample_to_bevy_present";
pub const CAMERA_DRAG_ACKNOWLEDGED: &str = "camera_drag.acknowledged_transition";
pub const CAMERA_DRAG_UNACKNOWLEDGED: &str = "camera_drag.unacknowledged_transition";
pub const START_END_SAME_PRODUCTION_FRAME: &str = "start_end_same_production_frame";
pub const DRAG_START_TO_FIRST_DELTA: &str = "drag_start_to_first_delta";
pub const INTERVAL_OVER_2_REFRESHES: &str = "interval_over_2_refreshes";
pub const RESIZE_ALLOCATION_COUNT: &str = "resize_allocation_count";

#[derive(Clone, Copy)]
struct StageSlot {
    name: Option<&'static str>,
    samples: [u64; SAMPLES_PER_STAGE],
    next: usize,
    retained: usize,
    count: u64,
    max_ns: u64,
}

impl StageSlot {
    // The sample ring lives in the `static REGISTRY`, never on a stack frame;
    // boxing it would make `Registry::new` non-const and cost the hot path.
    #[allow(clippy::large_stack_arrays)] // static storage, not a stack allocation.
    const EMPTY: Self = Self {
        name: None,
        samples: [0; SAMPLES_PER_STAGE],
        next: 0,
        retained: 0,
        count: 0,
        max_ns: 0,
    };

    fn record(&mut self, duration_ns: u64) {
        self.samples[self.next] = duration_ns;
        self.next = (self.next + 1) % SAMPLES_PER_STAGE;
        self.retained = self.retained.saturating_add(1).min(SAMPLES_PER_STAGE);
        self.count = self.count.saturating_add(1);
        self.max_ns = self.max_ns.max(duration_ns);
    }
}

#[derive(Clone, Copy)]
struct PendingInteraction {
    name: Option<&'static str>,
    stage: Option<&'static str>,
    started: Option<Instant>,
}

impl PendingInteraction {
    const EMPTY: Self = Self {
        name: None,
        stage: None,
        started: None,
    };
}

#[derive(Clone, Copy)]
struct SelectionTrace {
    interaction_id: u64,
    pointer_down: Option<Instant>,
    pick_resolved: Option<Instant>,
    dispatched: Option<Instant>,
    authored_published: Option<Instant>,
    highlight_published: Option<Instant>,
}

#[derive(Clone, Copy)]
struct DragTrace {
    interaction_id: u64,
    started: Option<Instant>,
    start_production_frame: u64,
    first_delta_recorded: bool,
}

impl DragTrace {
    const EMPTY: Self = Self {
        interaction_id: 0,
        started: None,
        start_production_frame: 0,
        first_delta_recorded: false,
    };
}

impl SelectionTrace {
    const EMPTY: Self = Self {
        interaction_id: 0,
        pointer_down: None,
        pick_resolved: None,
        dispatched: None,
        authored_published: None,
        highlight_published: None,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputTraceKind {
    PointerDown,
    ClickCoalesced,
    PickIssued,
    PickResolved,
    SelectionDispatched,
    HierarchyPublished,
    HighlightPublished,
    VisibleFramePresented,
    AssetDragMove,
    AssetDragLeave,
    GhostCursorSampled,
    GhostUpdated,
    GhostVisibleFrame,
    GizmoCursorSampled,
    GizmoRendered,
    GizmoVisibleFrame,
    AssetDrop,
    OptimisticObjectPublished,
    OptimisticObjectVisibleFrame,
    AuthoredEntityReconciled,
}

impl InputTraceKind {
    const fn label(self) -> &'static str {
        match self {
            Self::PointerDown => "pointer_down",
            Self::ClickCoalesced => "click_coalesced",
            Self::PickIssued => "pick_issued",
            Self::PickResolved => "pick_resolved",
            Self::SelectionDispatched => "selection_dispatched",
            Self::HierarchyPublished => "hierarchy_published",
            Self::HighlightPublished => "highlight_published",
            Self::VisibleFramePresented => "visible_frame_presented",
            Self::AssetDragMove => "asset_drag_move",
            Self::AssetDragLeave => "asset_drag_leave",
            Self::GhostCursorSampled => "ghost_cursor_sampled",
            Self::GhostUpdated => "ghost_updated",
            Self::GhostVisibleFrame => "ghost_visible_frame",
            Self::GizmoCursorSampled => "gizmo_cursor_sampled",
            Self::GizmoRendered => "gizmo_rendered",
            Self::GizmoVisibleFrame => "gizmo_visible_frame",
            Self::AssetDrop => "asset_drop",
            Self::OptimisticObjectPublished => "optimistic_object_published",
            Self::OptimisticObjectVisibleFrame => "optimistic_object_visible_frame",
            Self::AuthoredEntityReconciled => "authored_entity_reconciled",
        }
    }
}

#[derive(Clone, Copy)]
struct InputTraceEvent {
    t_ns: u64,
    interaction_id: u64,
    kind: InputTraceKind,
    mouse_x: f32,
    mouse_y: f32,
    world_x: f32,
    world_y: f32,
    world_z: f32,
    payload: u64,
}

impl InputTraceEvent {
    const EMPTY: Self = Self {
        t_ns: 0,
        interaction_id: 0,
        kind: InputTraceKind::PointerDown,
        mouse_x: f32::NAN,
        mouse_y: f32::NAN,
        world_x: f32::NAN,
        world_y: f32::NAN,
        world_z: f32::NAN,
        payload: 0,
    };
}

struct Registry {
    stages: [StageSlot; MAX_STAGES],
    pending: [PendingInteraction; MAX_PENDING_INTERACTIONS],
    selections: [SelectionTrace; MAX_PENDING_SELECTIONS],
    drags: [DragTrace; MAX_PENDING_DRAGS],
    latest_dispatched_selection: u64,
    completed_selections: u64,
    last_selection_dump: u64,
    completed_chrome_interactions: u64,
    trace_origin: Option<Instant>,
    input_trace: [InputTraceEvent; INPUT_TRACE_CAPACITY],
    input_trace_next: usize,
    input_trace_retained: usize,
    pending_ghost_visible: Option<(u64, f32, f32, f32)>,
    pending_ghost_cursor_sample: Option<(u64, Instant)>,
    pending_gizmo_visible: Option<(u64, f32, f32, f32)>,
    pending_gizmo_cursor_sample: Option<(u64, Instant)>,
    pending_optimistic_visible: Option<(u64, f32, f32, f32)>,
    pending_drag_move: Option<Instant>,
    drag_interaction_id: u64,
    drag_started: Option<Instant>,
    last_drag_duration_ns: u64,
}

impl Registry {
    // Builds the one `static REGISTRY` value; the fixed-capacity rings are
    // static storage, and boxing them would forfeit the const initializer this
    // allocation-free recorder depends on.
    #[allow(clippy::large_stack_arrays, clippy::large_stack_frames)] // static storage, not a stack allocation.
    const fn new() -> Self {
        Self {
            stages: [StageSlot::EMPTY; MAX_STAGES],
            pending: [PendingInteraction::EMPTY; MAX_PENDING_INTERACTIONS],
            selections: [SelectionTrace::EMPTY; MAX_PENDING_SELECTIONS],
            drags: [DragTrace::EMPTY; MAX_PENDING_DRAGS],
            latest_dispatched_selection: 0,
            completed_selections: 0,
            last_selection_dump: 0,
            completed_chrome_interactions: 0,
            trace_origin: None,
            input_trace: [InputTraceEvent::EMPTY; INPUT_TRACE_CAPACITY],
            input_trace_next: 0,
            input_trace_retained: 0,
            pending_ghost_visible: None,
            pending_ghost_cursor_sample: None,
            pending_gizmo_visible: None,
            pending_gizmo_cursor_sample: None,
            pending_optimistic_visible: None,
            pending_drag_move: None,
            drag_interaction_id: 0,
            drag_started: None,
            last_drag_duration_ns: 0,
        }
    }

    fn record(&mut self, stage: &'static str, duration_ns: u64) {
        if let Some(slot) = self.stages.iter_mut().find(|slot| slot.name == Some(stage)) {
            slot.record(duration_ns);
            return;
        }
        if let Some(slot) = self.stages.iter_mut().find(|slot| slot.name.is_none()) {
            slot.name = Some(stage);
            slot.record(duration_ns);
        }
    }

    fn trace(
        &mut self,
        kind: InputTraceKind,
        interaction_id: u64,
        mouse: Option<(f32, f32)>,
        world: Option<(f32, f32, f32)>,
        payload: u64,
        now: Instant,
    ) {
        let origin = *self.trace_origin.get_or_insert(now);
        let (mouse_x, mouse_y) = mouse.unwrap_or((f32::NAN, f32::NAN));
        let (world_x, world_y, world_z) = world.unwrap_or((f32::NAN, f32::NAN, f32::NAN));
        self.input_trace[self.input_trace_next] = InputTraceEvent {
            t_ns: duration_ns(now.duration_since(origin)),
            interaction_id,
            kind,
            mouse_x,
            mouse_y,
            world_x,
            world_y,
            world_z,
            payload,
        };
        self.input_trace_next = (self.input_trace_next + 1) % INPUT_TRACE_CAPACITY;
        self.input_trace_retained = self
            .input_trace_retained
            .saturating_add(1)
            .min(INPUT_TRACE_CAPACITY);
        if kind == InputTraceKind::GhostCursorSampled {
            self.pending_ghost_cursor_sample = Some((interaction_id, now));
        } else if kind == InputTraceKind::GhostUpdated {
            self.record("asset_drag.ghost_updated", 1);
            if let Some(moved) = self.pending_drag_move.take() {
                self.record(
                    "asset_drag.move_to_ghost_update",
                    duration_ns(now.duration_since(moved)),
                );
            }
            self.pending_ghost_visible = Some((interaction_id, world_x, world_y, world_z));
        } else if kind == InputTraceKind::GizmoCursorSampled {
            self.pending_gizmo_cursor_sample = Some((interaction_id, now));
        } else if kind == InputTraceKind::GizmoRendered {
            self.pending_gizmo_visible = Some((interaction_id, world_x, world_y, world_z));
        } else if kind == InputTraceKind::AssetDragMove {
            self.record("asset_drag.move", 1);
            self.pending_drag_move = Some(now);
        } else if kind == InputTraceKind::OptimisticObjectPublished {
            self.pending_optimistic_visible = Some((interaction_id, world_x, world_y, world_z));
        }
    }

    fn selection_mut(&mut self, interaction_id: u64) -> Option<&mut SelectionTrace> {
        self.selections
            .iter_mut()
            .find(|selection| selection.interaction_id == interaction_id)
    }

    /// Close every chrome interaction still open at this Present boundary.
    /// Returns whether the completion count crossed a summary-dump multiple.
    fn complete_pending_chrome_interactions(&mut self, now: Instant) -> bool {
        let mut should_dump = false;
        for name in [
            ACTIVITY_MODE_TO_WORKSPACE,
            TOOLBAR_TO_VISIBLE,
            DOCK_TAB_TO_VISIBLE,
            POPOVER_TO_VISIBLE,
        ] {
            let (started, stage) = self
                .pending
                .iter_mut()
                .find(|slot| slot.name == Some(name))
                .map_or((None, name), |slot| {
                    let stage = slot.stage.take().unwrap_or(name);
                    (slot.started.take(), stage)
                });
            if let Some(started) = started {
                self.record(stage, duration_ns(now.duration_since(started)));
                self.completed_chrome_interactions =
                    self.completed_chrome_interactions.saturating_add(1);
                should_dump |= self
                    .completed_chrome_interactions
                    .is_multiple_of(CHROME_INTERACTIONS_PER_DUMP);
            }
        }
        should_dump
    }

    /// Retire every highlighted selection older than the newest one, and
    /// return that newest interaction id (0 when none is pending).
    fn coalesce_superseded_selections(&mut self, now: Instant) -> u64 {
        let latest_visible = self
            .selections
            .iter()
            .filter(|selection| selection.highlight_published.is_some())
            .map(|selection| selection.interaction_id)
            .max()
            .unwrap_or(0);
        let superseded: [u64; MAX_PENDING_SELECTIONS] = std::array::from_fn(|index| {
            let selection = self.selections[index];
            if selection.highlight_published.is_some() && selection.interaction_id != latest_visible
            {
                selection.interaction_id
            } else {
                0
            }
        });
        for interaction_id in superseded.into_iter().filter(|id| *id != 0) {
            self.trace(
                InputTraceKind::ClickCoalesced,
                interaction_id,
                None,
                None,
                0,
                now,
            );
            if let Some(selection) = self.selection_mut(interaction_id) {
                *selection = SelectionTrace::EMPTY;
            }
        }
        latest_visible
    }

    /// Record the pointer-to-visible stages for the selection this frame made
    /// visible, then free its trace slot.
    fn complete_visible_selection(&mut self, interaction_id: u64, now: Instant) {
        if interaction_id == 0 {
            return;
        }
        let Some(index) = self
            .selections
            .iter()
            .position(|selection| selection.interaction_id == interaction_id)
        else {
            return;
        };
        let selection = self.selections[index];
        if let Some(pointer_down) = selection.pointer_down {
            self.record(
                SELECTION_POINTER_TO_VISIBLE,
                duration_ns(now.duration_since(pointer_down)),
            );
        }
        if let Some(published) = selection.authored_published {
            self.record(
                SELECTION_PUBLISH_TO_VISIBLE,
                duration_ns(now.duration_since(published)),
            );
        }
        if let Some(highlight) = selection.highlight_published {
            self.record(
                SELECTION_HIGHLIGHT_TO_VISIBLE,
                duration_ns(now.duration_since(highlight)),
            );
        }
        self.trace(
            InputTraceKind::VisibleFramePresented,
            interaction_id,
            None,
            None,
            0,
            now,
        );
        self.selections[index] = SelectionTrace::EMPTY;
        self.completed_selections = self.completed_selections.saturating_add(1);
    }

    /// Close the drag ghost, gizmo, and optimistic-object frames this Present
    /// made visible.
    fn complete_pending_drag_visuals(&mut self, now: Instant) {
        if let Some((interaction_id, x, y, z)) = self.pending_ghost_visible.take() {
            if let Some((sample_interaction_id, sampled)) = self.pending_ghost_cursor_sample.take()
                && sample_interaction_id == interaction_id
            {
                self.record(
                    "asset_drag.cursor_sample_to_visible",
                    duration_ns(now.duration_since(sampled)),
                );
            }
            self.trace(
                InputTraceKind::GhostVisibleFrame,
                interaction_id,
                None,
                Some((x, y, z)),
                0,
                now,
            );
        }
        if let Some((interaction_id, x, y, z)) = self.pending_gizmo_visible.take() {
            if let Some((sample_interaction_id, sampled)) = self.pending_gizmo_cursor_sample.take()
                && sample_interaction_id == interaction_id
            {
                self.record(
                    "gizmo_drag.cursor_sample_to_visible",
                    duration_ns(now.duration_since(sampled)),
                );
            }
            self.trace(
                InputTraceKind::GizmoVisibleFrame,
                interaction_id,
                None,
                Some((x, y, z)),
                0,
                now,
            );
        }
        if let Some((interaction_id, x, y, z)) = self.pending_optimistic_visible.take() {
            self.trace(
                InputTraceKind::OptimisticObjectVisibleFrame,
                interaction_id,
                None,
                Some((x, y, z)),
                0,
                now,
            );
        }
    }
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry::new());
static SUMMARY_DUMP_REQUESTED: AtomicBool = AtomicBool::new(false);
static INITIAL_INTERACTION_DUMP_REQUESTED: AtomicBool = AtomicBool::new(false);

fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[must_use]
pub fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Map an interaction id onto a fixed-capacity trace ring slot.
///
/// `capacity` is a small compile-time constant, so the remainder always fits a
/// `usize` and neither fallback is reachable.
fn ring_slot(interaction_id: u64, capacity: usize) -> usize {
    let capacity = u64::try_from(capacity).unwrap_or(u64::MAX);
    usize::try_from(interaction_id % capacity).unwrap_or(0)
}

/// Widen a recorded nanosecond count or event count for float formatting.
///
/// Timing values stay far below `2^53` nanoseconds (about 104 days), so the
/// widening is exact for every value this module records.
#[allow(clippy::cast_precision_loss)] // no lossless u64 -> f64 conversion exists; values stay below 2^53.
const fn as_f64(value: u64) -> f64 {
    value as f64
}

/// Record one duration without allocating or logging.
pub fn record_ns(stage: &'static str, duration_ns: u64) {
    registry().record(stage, duration_ns);
}

/// Record elapsed time since `started` without allocating or logging.
pub fn record_elapsed(stage: &'static str, started: Instant) {
    record_ns(stage, duration_ns(started.elapsed()));
}

/// Clear retained frame samples at a deliberate measurement boundary.
///
/// This is used once after renderer/shader warm-up so the sustained table is
/// not contaminated by first-use pipeline compilation or project bootstrap
/// work.
pub fn reset_frame_samples() {
    let mut registry = registry();
    for slot in &mut registry.stages {
        if slot.name.is_some_and(|name| name.starts_with("frame.")) {
            slot.samples.fill(0);
            slot.next = 0;
            slot.retained = 0;
            slot.count = 0;
            slot.max_ns = 0;
        }
    }
}

fn begin_drag_measurement(interaction_id: u64, production_frame: u64, now: Instant) {
    let mut registry = registry();
    let trace_index = registry
        .drags
        .iter()
        .position(|trace| trace.interaction_id == interaction_id)
        .or_else(|| {
            registry
                .drags
                .iter()
                .position(|trace| trace.interaction_id == 0)
        })
        .unwrap_or_else(|| ring_slot(interaction_id, MAX_PENDING_DRAGS));
    registry.drags[trace_index] = DragTrace {
        interaction_id,
        started: Some(now),
        start_production_frame: production_frame,
        first_delta_recorded: false,
    };
    registry.trace_origin.get_or_insert(now);
    registry.drag_interaction_id = interaction_id;
    registry.drag_started.get_or_insert(now);
}

fn finish_drag_measurement(interaction_id: u64, production_frame: u64, now: Instant) {
    let mut registry = registry();
    let trace = registry
        .drags
        .iter_mut()
        .find(|trace| trace.interaction_id == interaction_id)
        .map(|trace| {
            let value = *trace;
            *trace = DragTrace::EMPTY;
            value
        });
    if let Some(trace) = trace {
        if trace.start_production_frame == production_frame {
            registry.record(START_END_SAME_PRODUCTION_FRAME, 1);
        }
        if let Some(started) = trace.started {
            registry.last_drag_duration_ns = duration_ns(now.duration_since(started));
        }
    }
}

/// Begin a bounded camera-drag measurement window. Recording remains
/// allocation-free; only the requested summary at gesture end formats data.
pub fn camera_drag_started(interaction_id: u64, production_frame: u64, started_at: Instant) {
    begin_drag_measurement(interaction_id, production_frame, started_at);
}

/// Count one queued camera pointer event in the active measurement window.
pub fn camera_drag_event() {
    record_ns(CAMERA_DRAG_EVENT, 1);
}

/// Count one viewport production frame while a camera drag is active.
pub fn camera_drag_frame() {
    record_ns(CAMERA_DRAG_FRAME, 1);
}

/// Record same-tick cursor sample-to-camera application latency.
pub fn camera_drag_cursor_applied(sampled_at: Instant) {
    record_elapsed(CAMERA_DRAG_CURSOR_SAMPLE_TO_APPLY, sampled_at);
}

pub fn camera_drag_acknowledged() {
    record_ns(CAMERA_DRAG_ACKNOWLEDGED, 1);
}

pub fn camera_drag_unacknowledged(count: u64) {
    if count != 0 {
        record_ns(CAMERA_DRAG_UNACKNOWLEDGED, count);
    }
}

/// Count a missed display interval only while a camera gesture is active. The
/// refresh delta comes from the Bevy composition swapchain, not GPUI chrome.
pub fn camera_drag_display_interval(refreshes: u32) {
    if refreshes <= 2 {
        return;
    }
    let mut registry = registry();
    if registry.drags.iter().any(|trace| trace.interaction_id != 0) {
        registry.record(INTERVAL_OVER_2_REFRESHES, 1);
    }
}

/// Record the end-to-end producer portion at the real wgpu present boundary.
pub fn camera_drag_sample_presented(_interaction_id: u64, sampled_at: Instant) {
    // Presentation is asynchronous with respect to the producer transition
    // timeline. The matching End may already be acknowledged when this exact
    // frame reaches Present, but its carried sample remains valid evidence.
    record_elapsed(CAMERA_DRAG_POINTER_TO_BEVY_PRESENT, sampled_at);
}

/// Record the first non-zero camera delta for a gesture exactly once.
pub fn camera_drag_first_delta(interaction_id: u64, _production_frame: u64, _sampled_at: Instant) {
    let now = Instant::now();
    let mut registry = registry();
    let started = registry
        .drags
        .iter_mut()
        .find(|trace| trace.interaction_id == interaction_id && !trace.first_delta_recorded)
        .and_then(|trace| {
            trace.first_delta_recorded = true;
            trace.started
        });
    if let Some(started) = started {
        registry.record(
            DRAG_START_TO_FIRST_DELTA,
            duration_ns(now.duration_since(started)),
        );
    }
}

/// Finish the active camera-drag measurement and request its compact summary.
pub fn camera_drag_ended(interaction_id: u64, production_frame: u64, ended_at: Instant) {
    finish_drag_measurement(interaction_id, production_frame, ended_at);
    request_summary_dump();
}

/// Start or replace a named cross-callback interaction timer.
pub fn begin_interaction(name: &'static str) {
    begin_interaction_as(name, name);
}

fn begin_interaction_as(name: &'static str, stage: &'static str) {
    let mut registry = registry();
    let slot_index = registry
        .pending
        .iter()
        .position(|slot| slot.name == Some(name))
        .or_else(|| registry.pending.iter().position(|slot| slot.name.is_none()));
    if let Some(slot_index) = slot_index {
        let slot = &mut registry.pending[slot_index];
        slot.name = Some(name);
        slot.stage = Some(stage);
        slot.started = Some(Instant::now());
    }
    drop(registry);
    #[cfg(target_os = "windows")]
    gpui_windows::request_immediate_frame();
}

/// Complete a named interaction into the supplied aggregate stage.
#[must_use]
pub fn complete_interaction(name: &'static str, stage: &'static str) -> bool {
    let mut registry = registry();
    let started = registry
        .pending
        .iter_mut()
        .find(|slot| slot.name == Some(name))
        .and_then(|slot| slot.started.take());
    let completed = started.is_some_and(|started| {
        registry.record(stage, duration_ns(started.elapsed()));
        true
    });
    drop(registry);
    completed
}

pub fn dock_tab_activation_started(target_panel: &'static str) {
    begin_interaction_as(DOCK_TAB_TO_VISIBLE, dock_tab_stage(target_panel));
}

const fn dock_tab_stage(target_panel: &str) -> &'static str {
    match target_panel.as_bytes() {
        b"asset_browser" => DOCK_TAB_TO_ASSET_BROWSER,
        b"console" => DOCK_TAB_TO_CONSOLE,
        b"output-log" => DOCK_TAB_TO_OUTPUT_LOG,
        b"profiler" => DOCK_TAB_TO_PROFILER,
        b"gems" => DOCK_TAB_TO_GEMS,
        _ => DOCK_TAB_TO_VISIBLE,
    }
}

/// Complete pending chrome interactions at the first post-Present API boundary.
/// Called by the Windows renderer; takes one registry lock and does not allocate.
pub fn visible_frame_presented() {
    let now = Instant::now();
    let mut registry = registry();
    let mut should_dump = registry.complete_pending_chrome_interactions(now);
    let latest_visible = registry.coalesce_superseded_selections(now);
    registry.complete_visible_selection(latest_visible, now);
    registry.complete_pending_drag_visuals(now);
    if registry.completed_selections != registry.last_selection_dump
        && registry.completed_selections.is_multiple_of(16)
    {
        registry.last_selection_dump = registry.completed_selections;
        should_dump = true;
    }
    drop(registry);
    if should_dump {
        request_summary_dump();
    }
}

/// Request a summarized dump from the frame pump. Safe and allocation-free
/// from any editor interaction callback.
pub fn request_summary_dump() {
    SUMMARY_DUMP_REQUESTED.store(true, Ordering::Release);
}

/// Request the process's first interaction-baseline dump exactly once. This is
/// useful for a visible-state boundary without turning every UI action into a
/// log event.
pub fn request_initial_interaction_summary_dump() {
    if !INITIAL_INTERACTION_DUMP_REQUESTED.swap(true, Ordering::AcqRel) {
        request_summary_dump();
    }
}

/// Consume a pending summary request. The caller owns the allocation-bearing
/// summary/log step and should call this at a non-hot boundary.
#[must_use]
pub fn take_summary_dump_request() -> bool {
    SUMMARY_DUMP_REQUESTED.swap(false, Ordering::AcqRel)
}

pub fn selection_pointer_down(interaction_id: u64, x: f32, y: f32) {
    let now = Instant::now();
    let mut registry = registry();
    let slot = registry
        .selections
        .iter()
        .position(|selection| selection.interaction_id == 0)
        .unwrap_or_else(|| ring_slot(interaction_id, MAX_PENDING_SELECTIONS));
    registry.selections[slot] = SelectionTrace {
        interaction_id,
        pointer_down: Some(now),
        ..SelectionTrace::EMPTY
    };
    registry.trace(
        InputTraceKind::PointerDown,
        interaction_id,
        Some((x, y)),
        None,
        0,
        now,
    );
    let should_dump = interaction_id.is_multiple_of(64);
    drop(registry);
    if should_dump {
        request_summary_dump();
    }
}

/// One fast pick resolved against the published pick snapshot, measured end to
/// end.
///
/// The three instants are only meaningful relative to each other — the
/// pointer-down that opened the interaction, the moment the pick was issued,
/// and the moment it resolved — and the cursor position, payload, and
/// highlight flag describe that same pick. Every trace row this records keys
/// off the one `interaction_id`, so the whole set is one sample rather than
/// eight loose readings.
pub struct SelectionFastPick {
    pub interaction_id: u64,
    pub x: f32,
    pub y: f32,
    pub pointer_down: Instant,
    pub pick_issued: Instant,
    pub pick_resolved: Instant,
    pub payload: u64,
    pub highlighted: bool,
}

pub fn selection_fast_pick(sample: &SelectionFastPick) {
    let &SelectionFastPick {
        interaction_id,
        x,
        y,
        pointer_down,
        pick_issued,
        pick_resolved,
        payload,
        highlighted,
    } = sample;
    let mut registry = registry();
    let slot = registry
        .selections
        .iter()
        .position(|selection| selection.interaction_id == 0)
        .unwrap_or_else(|| ring_slot(interaction_id, MAX_PENDING_SELECTIONS));
    registry.selections[slot] = SelectionTrace {
        interaction_id,
        pointer_down: Some(pointer_down),
        pick_resolved: Some(pick_resolved),
        highlight_published: highlighted.then_some(pick_resolved),
        ..SelectionTrace::EMPTY
    };
    registry.record(
        SELECTION_POINTER_TO_PICK,
        duration_ns(pick_resolved.duration_since(pointer_down)),
    );
    if highlighted {
        registry.record(
            SELECTION_POINTER_TO_HIGHLIGHT,
            duration_ns(pick_resolved.duration_since(pointer_down)),
        );
    }
    registry.trace(
        InputTraceKind::PointerDown,
        interaction_id,
        Some((x, y)),
        None,
        0,
        pointer_down,
    );
    registry.trace(
        InputTraceKind::PickIssued,
        interaction_id,
        Some((x, y)),
        None,
        0,
        pick_issued,
    );
    registry.trace(
        InputTraceKind::PickResolved,
        interaction_id,
        Some((x, y)),
        None,
        payload,
        pick_resolved,
    );
    if highlighted {
        registry.trace(
            InputTraceKind::HighlightPublished,
            interaction_id,
            None,
            Some((f32::NAN, f32::NAN, f32::NAN)),
            payload,
            pick_resolved,
        );
    }
    let should_dump = interaction_id.is_multiple_of(64);
    drop(registry);
    if should_dump {
        request_summary_dump();
    }
}

pub fn selection_coalesced(interaction_id: u64) {
    let now = Instant::now();
    let mut registry = registry();
    registry.trace(
        InputTraceKind::ClickCoalesced,
        interaction_id,
        None,
        None,
        0,
        now,
    );
    if let Some(selection) = registry.selection_mut(interaction_id) {
        *selection = SelectionTrace::EMPTY;
    }
}

pub fn selection_pick_issued(interaction_id: u64, x: f32, y: f32) {
    let now = Instant::now();
    registry().trace(
        InputTraceKind::PickIssued,
        interaction_id,
        Some((x, y)),
        None,
        0,
        now,
    );
}

pub fn selection_pick_resolved(interaction_id: u64, x: f32, y: f32, payload: u64) {
    let now = Instant::now();
    let mut registry = registry();
    let pointer_down = registry
        .selection_mut(interaction_id)
        .and_then(|selection| selection.pointer_down);
    if let Some(started) = pointer_down {
        registry.record(
            SELECTION_POINTER_TO_PICK,
            duration_ns(now.duration_since(started)),
        );
        if let Some(selection) = registry.selection_mut(interaction_id) {
            selection.pick_resolved = Some(now);
        }
    }
    registry.trace(
        InputTraceKind::PickResolved,
        interaction_id,
        Some((x, y)),
        None,
        payload,
        now,
    );
}

pub fn selection_dispatched(interaction_id: u64, payload: u64) {
    let now = Instant::now();
    let mut registry = registry();
    let pick_resolved = registry
        .selection_mut(interaction_id)
        .and_then(|selection| selection.pick_resolved);
    if let Some(started) = pick_resolved {
        registry.record(
            SELECTION_PICK_TO_DISPATCH,
            duration_ns(now.duration_since(started)),
        );
        if let Some(selection) = registry.selection_mut(interaction_id) {
            selection.dispatched = Some(now);
        }
        registry.latest_dispatched_selection = interaction_id;
    }
    registry.trace(
        InputTraceKind::SelectionDispatched,
        interaction_id,
        None,
        None,
        payload,
        now,
    );
}

pub fn selection_authored_published() {
    let now = Instant::now();
    let mut registry = registry();
    let interaction_id = registry.latest_dispatched_selection;
    let dispatched = registry
        .selection_mut(interaction_id)
        .and_then(|selection| selection.dispatched);
    let already_published = registry
        .selection_mut(interaction_id)
        .is_none_or(|selection| selection.authored_published.is_some());
    if !already_published && let Some(started) = dispatched {
        registry.record(
            SELECTION_DISPATCH_TO_PUBLISH,
            duration_ns(now.duration_since(started)),
        );
        if let Some(selection) = registry.selection_mut(interaction_id) {
            selection.authored_published = Some(now);
        }
        registry.trace(
            InputTraceKind::HierarchyPublished,
            interaction_id,
            None,
            None,
            0,
            now,
        );
    }
}

pub fn selection_highlight_published(interaction_id: u64, world: (f32, f32, f32), payload: u64) {
    let now = Instant::now();
    let mut registry = registry();
    let pointer_down = registry
        .selection_mut(interaction_id)
        .and_then(|selection| selection.pointer_down);
    if let Some(pointer_down) = pointer_down {
        registry.record(
            SELECTION_POINTER_TO_HIGHLIGHT,
            duration_ns(now.duration_since(pointer_down)),
        );
    }
    if let Some(selection) = registry.selection_mut(interaction_id) {
        selection.highlight_published = Some(now);
    }
    registry.trace(
        InputTraceKind::HighlightPublished,
        interaction_id,
        None,
        Some(world),
        payload,
        now,
    );
}

pub fn input_trace(
    kind: InputTraceKind,
    interaction_id: u64,
    mouse: Option<(f32, f32)>,
    world: Option<(f32, f32, f32)>,
    payload: u64,
) {
    registry().trace(kind, interaction_id, mouse, world, payload, Instant::now());
}

pub fn record_viewport_ui_trace(event: az_editor_ui::panels::ViewportUiInputTrace) {
    use az_editor_ui::panels::ViewportUiInputTrace;
    match event {
        ViewportUiInputTrace::PointerDown {
            interaction_id,
            x,
            y,
        } => {
            #[cfg(target_os = "windows")]
            {
                let pointer_down = Instant::now();
                if !crate::viewport_host::publish_urgent_pointer_down(
                    interaction_id,
                    x,
                    y,
                    pointer_down,
                ) {
                    selection_pointer_down(interaction_id, x, y);
                }
            }
            #[cfg(not(target_os = "windows"))]
            selection_pointer_down(interaction_id, x, y);
        }
        ViewportUiInputTrace::AssetDragMove {
            interaction_id,
            x,
            y,
        } => input_trace(
            {
                begin_drag_measurement(interaction_id, 0, Instant::now());
                InputTraceKind::AssetDragMove
            },
            interaction_id,
            Some((x, y)),
            None,
            0,
        ),
        ViewportUiInputTrace::AssetDragLeave { interaction_id } => input_trace(
            InputTraceKind::AssetDragLeave,
            interaction_id,
            None,
            None,
            0,
        ),
        ViewportUiInputTrace::AssetDrop {
            interaction_id,
            x,
            y,
        } => {
            let now = Instant::now();
            input_trace(
                InputTraceKind::AssetDrop,
                interaction_id,
                Some((x, y)),
                None,
                0,
            );
            finish_drag_measurement(interaction_id, 0, now);
        }
    }
}

#[must_use]
pub fn stable_payload(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerfSummary {
    pub stage: &'static str,
    pub count: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
}

/// Snapshot all aggregate rows. This is the allocation-bearing, on-demand API.
#[must_use]
pub fn summary() -> Vec<PerfSummary> {
    let registry = registry();
    registry
        .stages
        .iter()
        .filter_map(|slot| {
            let stage = slot.name?;
            if slot.retained == 0 {
                return None;
            }
            let mut samples = slot.samples[..slot.retained].to_vec();
            samples.sort_unstable();
            Some(PerfSummary {
                stage,
                count: slot.count,
                p50_ns: percentile(&samples, 50),
                p95_ns: percentile(&samples, 95),
                p99_ns: percentile(&samples, 99),
                max_ns: slot.max_ns,
            })
        })
        .collect()
}

fn percentile(sorted: &[u64], percent: usize) -> u64 {
    let index = (sorted.len().saturating_sub(1) * percent) / 100;
    sorted[index]
}

/// Render a compact timing table suitable for one summarized tracing event.
#[must_use]
pub fn summary_table() -> String {
    let (drag_interaction_id, drag_duration_ns) = {
        let registry = registry();
        (registry.drag_interaction_id, registry.last_drag_duration_ns)
    };
    let rows = summary();
    let mut table = format!(
        "editor perf summary (durations; drag_id={drag_interaction_id}; drag_duration={})\n\
stage | count | rate_hz | p50 | p95 | p99 | max\n",
        format_duration(drag_duration_ns),
    );
    for row in rows {
        let rate_hz = if drag_duration_ns == 0 {
            0.0
        } else {
            as_f64(row.count) * 1_000_000_000.0 / as_f64(drag_duration_ns)
        };
        let _ = writeln!(
            table,
            "{} | {} | {:.1} | {} | {} | {} | {}",
            row.stage,
            row.count,
            rate_hz,
            format_duration(row.p50_ns),
            format_duration(row.p95_ns),
            format_duration(row.p99_ns),
            format_duration(row.max_ns),
        );
    }
    table
}

/// Render the retained tail of the fixed-size input trace ring. Allocation and
/// formatting happen only at an explicit summary boundary.
#[must_use]
pub fn input_trace_table() -> String {
    // The registry lock is released before formatting: only the retained tail
    // is copied out, and rendering it is the allocation-bearing step.
    let events = {
        let registry = registry();
        let retained = registry.input_trace_retained.min(INPUT_TRACE_DUMP_ROWS);
        let start =
            (registry.input_trace_next + INPUT_TRACE_CAPACITY - retained) % INPUT_TRACE_CAPACITY;
        let events = (0..retained)
            .map(|offset| registry.input_trace[(start + offset) % INPUT_TRACE_CAPACITY])
            .collect::<Vec<_>>();
        drop(registry);
        events
    };
    let mut table = String::from(
        "editor input trace (t relative to first retained session event)\n\
t_ns | id | event | mouse_x | mouse_y | world_x | world_y | world_z | payload\n",
    );
    for event in events {
        let _ = writeln!(
            table,
            "{} | {} | {} | {:.5} | {:.5} | {:.4} | {:.4} | {:.4} | {:016x}",
            event.t_ns,
            event.interaction_id,
            event.kind.label(),
            event.mouse_x,
            event.mouse_y,
            event.world_x,
            event.world_y,
            event.world_z,
            event.payload,
        );
    }
    table
}

fn format_duration(ns: u64) -> String {
    if ns >= 1_000_000 {
        format!("{:.3}ms", as_f64(ns) / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.3}us", as_f64(ns) / 1_000.0)
    } else {
        format!("{ns}ns")
    }
}

/// Emit one summarized event; never call this per frame or pointer event.
pub fn log_summary() {
    tracing::info!(target: "az_editor::perf", "{}", summary_table());
    let trace = input_trace_table();
    if !trace.ends_with("payload\n") {
        tracing::info!(target: "az_editor::perf", "{}", trace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_rows_report_count_p50_p95_p99_and_max() {
        let mut slot = StageSlot::EMPTY;
        slot.name = Some("test.stage");
        for duration in 1..=100 {
            slot.record(duration);
        }
        let mut samples = slot.samples[..slot.retained].to_vec();
        samples.sort_unstable();
        assert_eq!(slot.count, 100);
        assert_eq!(percentile(&samples, 50), 50);
        assert_eq!(percentile(&samples, 95), 95);
        assert_eq!(percentile(&samples, 99), 99);
        assert_eq!(slot.max_ns, 100);
    }

    #[test]
    fn duration_conversion_saturates() {
        assert_eq!(duration_ns(Duration::from_nanos(42)), 42);
    }

    #[test]
    fn dock_tab_stages_identify_each_cached_destination() {
        assert_eq!(dock_tab_stage("asset_browser"), DOCK_TAB_TO_ASSET_BROWSER);
        assert_eq!(dock_tab_stage("console"), DOCK_TAB_TO_CONSOLE);
        assert_eq!(dock_tab_stage("output-log"), DOCK_TAB_TO_OUTPUT_LOG);
        assert_eq!(dock_tab_stage("profiler"), DOCK_TAB_TO_PROFILER);
        assert_eq!(dock_tab_stage("gems"), DOCK_TAB_TO_GEMS);
        assert_eq!(dock_tab_stage("third-party"), DOCK_TAB_TO_VISIBLE);
    }
}
