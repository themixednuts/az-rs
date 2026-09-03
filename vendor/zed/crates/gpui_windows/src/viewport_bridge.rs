//! Process-wide viewport interaction and structural telemetry hooks.
//!
//! DirectComposition owns viewport composition. This module contains only the
//! lightweight Windows message/cursor bridge and diagnostics shared between the
//! GPUI window thread and the independent Bevy producer.

use parking_lot::Mutex;
use std::{
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering},
    },
    time::Instant,
};
use windows::Win32::{
    Foundation::{LPARAM, POINT, WPARAM},
    Graphics::Gdi::ScreenToClient,
    UI::{
        Input::KeyboardAndMouse::GetActiveWindow,
        WindowsAndMessaging::{GetCursorPos, PostMessageW},
    },
};
static VIEWPORT_PERF_SINK: OnceLock<fn(&'static str, u64)> = OnceLock::new();
static FRAME_PRESENTED_SINK: OnceLock<fn()> = OnceLock::new();
static PERF_EPOCH: OnceLock<Instant> = OnceLock::new();
static IMMEDIATE_FRAME_REQUEST_NS: AtomicU64 = AtomicU64::new(0);
static IMMEDIATE_FRAME_HANDLER_NS: AtomicU64 = AtomicU64::new(0);
static LAST_FRAME_PRESENTED_NS: AtomicU64 = AtomicU64::new(0);
static LAST_BEVY_PRESENTED_NS: AtomicU64 = AtomicU64::new(0);
static NEXT_IMMEDIATE_FRAME_NS: AtomicU64 = AtomicU64::new(0);
static IMMEDIATE_FRAME_INTERVAL_NS: AtomicU64 = AtomicU64::new(7_500_000);
static IMMEDIATE_FRAME_ARMED: ImmediateFrameGate = ImmediateFrameGate::new();
static ACTIVE_EDITOR_HWND: AtomicIsize = AtomicIsize::new(0);
static VIEWPORT_COMPOSITOR_COUNTERS: Mutex<ViewportCompositorCounters> =
    Mutex::new(ViewportCompositorCounters::new());

struct ImmediateFrameGate {
    armed: AtomicBool,
}

/// Monotonic structural diagnostics for the DirectComposition viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewportCompositorCounters {
    pub hole_rect_mismatch_count: u64,
    pub hole_commit_count: u64,
    pub acquired_texture_count: u64,
    pub presented_texture_count: u64,
    pub discarded_texture_count: u64,
    pub surface_replace_count: u64,
    pub reconfigure_while_acquired_count: u64,
    pub last_hole_rect: crate::ViewportDeviceRect,
    pub last_visual_rect: crate::ViewportDeviceRect,
    pub last_slot_id: u64,
    pub last_scene_generation: u64,
    pub bevy_present_extent: (u32, u32),
    pub present_id: u64,
    pub submission_id: u64,
    pub fence_value: u64,
    pub completed_fence_value: u64,
}

impl ViewportCompositorCounters {
    const fn new() -> Self {
        Self {
            hole_rect_mismatch_count: 0,
            hole_commit_count: 0,
            acquired_texture_count: 0,
            presented_texture_count: 0,
            discarded_texture_count: 0,
            surface_replace_count: 0,
            reconfigure_while_acquired_count: 0,
            last_hole_rect: crate::ViewportDeviceRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            last_visual_rect: crate::ViewportDeviceRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            last_slot_id: 0,
            last_scene_generation: 0,
            bevy_present_extent: (0, 0),
            present_id: 0,
            submission_id: 0,
            fence_value: 0,
            completed_fence_value: 0,
        }
    }
}

#[must_use]
pub fn viewport_compositor_counters() -> ViewportCompositorCounters {
    *VIEWPORT_COMPOSITOR_COUNTERS.lock()
}

pub(crate) fn record_dcomp_hole_rect(
    slot_id: u64,
    scene_generation: u64,
    rect: crate::ViewportDeviceRect,
) {
    let mut counters = VIEWPORT_COMPOSITOR_COUNTERS.lock();
    counters.last_slot_id = slot_id;
    counters.last_scene_generation = scene_generation;
    counters.last_hole_rect = rect;
}

pub(crate) fn record_dcomp_visual_commit(
    slot_id: u64,
    scene_generation: u64,
    rect: crate::ViewportDeviceRect,
) {
    let mut counters = VIEWPORT_COMPOSITOR_COUNTERS.lock();
    counters.hole_commit_count += 1;
    counters.last_visual_rect = rect;
    if counters.last_slot_id != slot_id
        || counters.last_scene_generation != scene_generation
        || counters.last_hole_rect != rect
    {
        counters.hole_rect_mismatch_count += 1;
    }
}

/// Record acquisition before the frame enters Bevy extraction.
pub fn record_dcomp_texture_acquired(extent: (u32, u32)) {
    let mut counters = VIEWPORT_COMPOSITOR_COUNTERS.lock();
    counters.acquired_texture_count += 1;
    counters.bevy_present_extent = extent;
}

/// Record the proven post-submit fence and the matching present terminal action.
pub fn record_dcomp_texture_presented(
    extent: (u32, u32),
    present_id: u64,
    submission_id: u64,
    fence_value: u64,
    completed_fence_value: u64,
) {
    let now = perf_now_ns();
    let previous = LAST_BEVY_PRESENTED_NS.swap(now, Ordering::AcqRel);
    if previous != 0 {
        record_viewport_perf("frame.bevy_present_interval", now.saturating_sub(previous));
    }
    record_viewport_perf("frame.bevy_presented", 1);
    let mut counters = VIEWPORT_COMPOSITOR_COUNTERS.lock();
    counters.presented_texture_count += 1;
    counters.bevy_present_extent = extent;
    counters.present_id = present_id;
    counters.submission_id = submission_id;
    counters.fence_value = fence_value;
    counters.completed_fence_value = completed_fence_value;
}

/// Record the only other legal terminal action for an acquired surface texture.
pub fn record_dcomp_texture_discarded() {
    VIEWPORT_COMPOSITOR_COUNTERS.lock().discarded_texture_count += 1;
}

/// Count a policy-approved composition surface replacement.
pub fn record_dcomp_surface_replaced() {
    VIEWPORT_COMPOSITOR_COUNTERS.lock().surface_replace_count += 1;
    record_viewport_perf("dcomp.surface_replace", 1);
}

/// This invariant counter must remain zero: replacement is illegal while a
/// `SurfaceTexture` is still owned by the pipelined render frame.
pub fn record_dcomp_reconfigure_while_acquired() {
    VIEWPORT_COMPOSITOR_COUNTERS
        .lock()
        .reconfigure_while_acquired_count += 1;
    record_viewport_perf("dcomp.reconfigure_while_acquired", 1);
}

impl ImmediateFrameGate {
    const fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
        }
    }

    fn arm(&self) -> bool {
        self.armed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn disarm(&self) {
        self.armed.store(false, Ordering::Release);
    }
}

fn perf_now_ns() -> u64 {
    PERF_EPOCH
        .get_or_init(Instant::now)
        .elapsed()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64
        + 1
}

fn following_immediate_frame_ns(now_ns: u64, scheduled_ns: u64, interval_ns: u64) -> u64 {
    let interval_ns = interval_ns.max(1);
    if scheduled_ns == 0 {
        return now_ns.saturating_add(interval_ns);
    }
    let intervals_elapsed = now_ns.saturating_sub(scheduled_ns) / interval_ns + 1;
    scheduled_ns.saturating_add(intervals_elapsed.saturating_mul(interval_ns))
}

/// Queue a normal-priority frame request for the active GPUI window.
///
/// Windows delivers `WM_PAINT` only after higher-priority queued work. The
/// editor's continuous viewport pump can therefore starve an invalidation
/// during dense pointer input. A posted application message preserves GPUI's
/// non-reentrant draw boundary while ensuring an interaction frame is not
/// parked behind the low-priority paint queue.
pub fn request_immediate_frame() {
    let now_ns = perf_now_ns();
    let next_ns = NEXT_IMMEDIATE_FRAME_NS.load(Ordering::Acquire);
    if next_ns != 0 && now_ns < next_ns {
        return;
    }
    if !IMMEDIATE_FRAME_ARMED.arm() {
        return;
    }
    let mut hwnd = unsafe { GetActiveWindow() };
    if hwnd.is_invalid() {
        let cached = ACTIVE_EDITOR_HWND.load(Ordering::Acquire);
        if cached != 0 {
            hwnd = windows::Win32::Foundation::HWND(cached as *mut _);
        }
    } else {
        ACTIVE_EDITOR_HWND.store(hwnd.0 as isize, Ordering::Release);
    }
    if hwnd.is_invalid() {
        IMMEDIATE_FRAME_ARMED.disarm();
        return;
    }
    IMMEDIATE_FRAME_REQUEST_NS.store(now_ns, Ordering::Release);
    unsafe {
        if PostMessageW(
            Some(hwnd),
            crate::events::WM_GPUI_FORCE_UPDATE_WINDOW,
            WPARAM::default(),
            LPARAM::default(),
        )
        .is_ok()
        {
            let interval_ns = IMMEDIATE_FRAME_INTERVAL_NS.load(Ordering::Acquire);
            let following_ns = following_immediate_frame_ns(now_ns, next_ns, interval_ns);
            NEXT_IMMEDIATE_FRAME_NS.store(following_ns, Ordering::Release);
            record_viewport_perf("frame.immediate_request_posted", 1);
        } else {
            IMMEDIATE_FRAME_REQUEST_NS.store(0, Ordering::Release);
            IMMEDIATE_FRAME_ARMED.disarm();
        }
    }
}

/// Limit cached-scene viewport Presents to the active display cadence. Bevy's
/// offscreen production rate is independent and may be substantially higher.
pub fn set_immediate_frame_rate_hz(refresh_hz: u32) {
    let interval_ns = 1_000_000_000_u64.div_ceil(u64::from(refresh_hz.max(1)));
    IMMEDIATE_FRAME_INTERVAL_NS.store(interval_ns, Ordering::Release);
    NEXT_IMMEDIATE_FRAME_NS.store(0, Ordering::Release);
}

/// Remember the editor window used by the production thread. `GetActiveWindow`
/// is thread-local and therefore returns null on that dedicated thread.
pub(crate) fn remember_interaction_window(hwnd: windows::Win32::Foundation::HWND) {
    if !hwnd.is_invalid() {
        ACTIVE_EDITOR_HWND.store(hwnd.0 as isize, Ordering::Release);
    }
}

/// Sample the absolute Windows cursor as late as possible and normalize it to
/// the most recently painted viewport rectangle. This deliberately bypasses
/// coalesced `WM_MOUSEMOVE`/GPUI drag events.
#[must_use]
pub fn sample_viewport_cursor(
    device_origin: (i32, i32),
    device_size: (u32, u32),
) -> Option<(f32, f32, i32, i32)> {
    if device_size.0 == 0 || device_size.1 == 0 {
        return None;
    }
    let raw = ACTIVE_EDITOR_HWND.load(Ordering::Acquire);
    if raw == 0 {
        return None;
    }
    let hwnd = windows::Win32::Foundation::HWND(raw as *mut _);
    let mut point = POINT::default();
    unsafe {
        GetCursorPos(&mut point).ok()?;
        ScreenToClient(hwnd, &mut point).ok().ok()?;
    }
    let x = (point.x - device_origin.0) as f32 / device_size.0 as f32;
    let y = (point.y - device_origin.1) as f32 / device_size.1 as f32;
    Some((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0), point.x, point.y))
}

pub(crate) fn record_immediate_frame_message_received() {
    let now = perf_now_ns();
    record_viewport_perf("frame.immediate_request_handled", 1);
    let requested = IMMEDIATE_FRAME_REQUEST_NS.swap(0, Ordering::AcqRel);
    if requested != 0 {
        record_viewport_perf(
            "frame.immediate_request_to_handler",
            now.saturating_sub(requested),
        );
    }
    IMMEDIATE_FRAME_HANDLER_NS.store(now, Ordering::Release);
}

pub(crate) fn record_immediate_frame_present_submitted() {
    let now = perf_now_ns();
    let handled = IMMEDIATE_FRAME_HANDLER_NS.swap(0, Ordering::AcqRel);
    if handled != 0 {
        record_viewport_perf(
            "frame.immediate_handler_to_present",
            now.saturating_sub(handled),
        );
    }
}

/// Install the editor's zero-allocation frame timing sink. Repeated installs
/// keep the first sink because the Windows renderer is process-global.
pub fn set_viewport_perf_sink(sink: fn(&'static str, u64)) {
    let _ = VIEWPORT_PERF_SINK.set(sink);
}

/// Install an allocation-free callback invoked after a GPUI frame has been
/// submitted to the swap chain. The editor uses this as its real-Present API
/// boundary; DXGI frame statistics separately measure compositor display time.
pub fn set_frame_presented_sink(sink: fn()) {
    let _ = FRAME_PRESENTED_SINK.set(sink);
}

pub(crate) fn record_viewport_perf(stage: &'static str, duration_ns: u64) {
    if let Some(sink) = VIEWPORT_PERF_SINK.get() {
        sink(stage, duration_ns);
    }
}

pub(crate) fn record_frame_presented() {
    let now = perf_now_ns();
    let previous = LAST_FRAME_PRESENTED_NS.swap(now, Ordering::AcqRel);
    if previous != 0 {
        record_viewport_perf("frame.present_interval", now.saturating_sub(previous));
    }
    record_viewport_perf("frame.presented", 1);
    // Release after the real Present boundary. The next overlay or surface-
    // handoff update may now post one coalesced window-thread frame request.
    IMMEDIATE_FRAME_ARMED.disarm();
    if let Some(sink) = FRAME_PRESENTED_SINK.get() {
        sink();
    }
}

#[cfg(test)]
mod tests {
    use super::{ImmediateFrameGate, following_immediate_frame_ns};

    #[test]
    fn immediate_frame_gate_allows_one_request_until_present_disarms_it() {
        let gate = ImmediateFrameGate::new();
        assert!(gate.arm());
        for _ in 0..1_000 {
            assert!(!gate.arm());
        }
        gate.disarm();
        assert!(gate.arm());
    }

    #[test]
    fn immediate_frame_deadline_retains_display_phase_after_a_late_producer_tick() {
        assert_eq!(following_immediate_frame_ns(1_000, 0, 100), 1_100);
        assert_eq!(following_immediate_frame_ns(1_250, 1_100, 100), 1_300);
        assert_eq!(following_immediate_frame_ns(1_300, 1_300, 100), 1_400);
    }
}
