//! DirectComposition ownership for one GPUI window and one typed viewport slot.
//!
//! A contentless root owns a Bevy child visual below GPUI's premultiplied-alpha
//! visual. The GPUI window thread exclusively mutates and commits this tree;
//! the producer receives only a lifetime-bound surface-creation capability.

use std::{
    cell::RefCell,
    ffi::c_void,
    fmt,
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result};
use windows::Win32::{
    Foundation::HWND,
    Graphics::{
        DirectComposition::{
            DCompositionCreateDevice, IDCompositionDevice, IDCompositionRectangleClip,
            IDCompositionScaleTransform, IDCompositionTarget, IDCompositionVisual,
        },
        Dxgi::{IDXGIDevice, IDXGISwapChain1},
    },
};
use windows::core::Interface;

/// One integer device-pixel rectangle. Width and height are always derived by
/// subtracting the edges; callers never publish a separately rounded extent.
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
            self.right.saturating_sub(self.left) as u32
        } else {
            0
        }
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        if self.bottom > self.top {
            self.bottom.saturating_sub(self.top) as u32
        } else {
            0
        }
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }
}

/// Identifies the single viewport visual slot without exposing a raw COM or
/// process-global integer slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewportVisualSlotId(pub u64);

pub const PRIMARY_VIEWPORT_VISUAL_SLOT: ViewportVisualSlotId = ViewportVisualSlotId(1);

/// Authoritative GPUI-scene layout passed unchanged to the renderer-bound slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportCompositionLayout {
    pub window_id: u64,
    pub slot_id: ViewportVisualSlotId,
    pub scene_generation: u64,
    pub device_rect: ViewportDeviceRect,
    pub scale_factor: f32,
    pub visible: bool,
    pub corner_radii: [f32; 4],
}

impl Default for ViewportCompositionLayout {
    fn default() -> Self {
        Self {
            window_id: 0,
            slot_id: PRIMARY_VIEWPORT_VISUAL_SLOT,
            scene_generation: 0,
            device_rect: ViewportDeviceRect::default(),
            scale_factor: 1.0,
            visible: false,
            corner_radii: [0.0; 4],
        }
    }
}

/// Failure from a renderer-lifetime-bound viewport slot operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewportVisualSlotError {
    Invalidated,
    WrongWindow { expected: u64, actual: u64 },
    WrongSlot(ViewportVisualSlotId),
    StaleLayout { committed: u64, incoming: u64 },
    Composition(String),
}

impl fmt::Display for ViewportVisualSlotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalidated => formatter.write_str("viewport visual slot was invalidated"),
            Self::WrongWindow { expected, actual } => {
                write!(
                    formatter,
                    "viewport visual slot belongs to window {expected}, not {actual}"
                )
            }
            Self::WrongSlot(slot) => write!(formatter, "unknown viewport visual slot {slot:?}"),
            Self::StaleLayout {
                committed,
                incoming,
            } => write!(
                formatter,
                "viewport layout generation {incoming} is older than {committed}"
            ),
            Self::Composition(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ViewportVisualSlotError {}

/// A weak capability: the renderer owns all COM objects, while editor code can
/// only mutate them for as long as that exact renderer/device generation lives.
#[derive(Clone)]
pub struct ViewportVisualSlot {
    tree: Weak<RefCell<CompositionTreeState>>,
    window_id: u64,
}

/// Strong, sendable capability for creating one wgpu composition surface.
/// The COM visual remains renderer-owned; this clone only keeps the exact
/// visual generation alive until wgpu has taken its own reference.
#[derive(Clone)]
pub struct ViewportVisualSurfaceTarget {
    device: IDCompositionDevice,
    visual: IDCompositionVisual,
    bridge: Arc<CompositionSurfaceBridge>,
    surface_id: u64,
    first_presented: Arc<AtomicBool>,
}

// SAFETY: DirectComposition device objects are free-threaded. The capability is
// moved (not shared) to the producer's MTA and retains a strong COM reference;
// wgpu AddRefs the visual again during surface creation.
unsafe impl Send for ViewportVisualSurfaceTarget {}
// SAFETY: the same free-threaded DirectComposition interfaces may be retained
// by the producer and pipelined render threads. Per-surface publication is
// guarded by atomics and the bridge mutex.
unsafe impl Sync for ViewportVisualSurfaceTarget {}

impl ViewportVisualSurfaceTarget {
    /// Raw `IDCompositionVisual` pointer accepted by
    /// `wgpu::SurfaceTargetUnsafe::CompositionVisual`.
    #[must_use]
    pub fn as_raw(&self) -> *mut c_void {
        self.visual.as_raw()
    }

    /// Create an off-tree content visual for a replacement wgpu surface. The
    /// current completed visual remains attached until this target reports its
    /// first successful present and the GPUI window thread commits the swap.
    pub fn replacement(&self) -> std::result::Result<Self, ViewportVisualSlotError> {
        let visual = unsafe { self.device.CreateVisual() }.map_err(|error| {
            ViewportVisualSlotError::Composition(format!(
                "creating replacement viewport content visual: {error:?}"
            ))
        })?;
        Ok(Self {
            device: self.device.clone(),
            visual,
            bridge: self.bridge.clone(),
            surface_id: self.bridge.next_surface_id.fetch_add(1, Ordering::Relaxed),
            first_presented: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Publish the first complete buffer from this surface. Returns `true`
    /// exactly once, when a window-thread composition commit is required.
    #[must_use]
    pub fn mark_presented(&self, extent: (u32, u32)) -> bool {
        if self.first_presented.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.bridge.publish_ready(ReadyCompositionSurface {
            id: self.surface_id,
            visual: self.visual.clone(),
            extent,
        });
        true
    }

    /// Whether the window thread has committed this surface into the live
    /// visual tree. The producer uses this to retire the prior wgpu surface
    /// only after DirectComposition no longer displays it.
    #[must_use]
    pub fn is_committed(&self) -> bool {
        self.bridge.committed_surface_id.load(Ordering::Acquire) >= self.surface_id
    }

    #[must_use]
    pub fn surface_id(&self) -> u64 {
        self.surface_id
    }
}

struct ReadyCompositionSurface {
    id: u64,
    visual: IDCompositionVisual,
    extent: (u32, u32),
}

// SAFETY: DirectComposition device and visual interfaces are free-threaded.
// Ready visuals only cross from the producer MTA to the renderer window thread
// and are mutated there after transfer.
unsafe impl Send for ReadyCompositionSurface {}

struct CompositionSurfaceBridge {
    next_surface_id: AtomicU64,
    committed_surface_id: AtomicU64,
    ready: Mutex<Option<ReadyCompositionSurface>>,
}

impl CompositionSurfaceBridge {
    fn new(initial_surface_id: u64) -> Self {
        Self {
            next_surface_id: AtomicU64::new(initial_surface_id + 1),
            committed_surface_id: AtomicU64::new(0),
            ready: Mutex::new(None),
        }
    }

    fn publish_ready(&self, ready: ReadyCompositionSurface) {
        let mut pending = self
            .ready
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pending.as_ref().is_none_or(|current| ready.id > current.id) {
            *pending = Some(ready);
        }
    }

    fn take_ready(&self) -> Option<ReadyCompositionSurface> {
        self.ready
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

impl ViewportVisualSlot {
    /// Stable GPUI window identity this renderer-generation capability targets.
    #[must_use]
    pub fn window_id(&self) -> u64 {
        self.window_id
    }

    fn with_state<T>(
        &self,
        operation: impl FnOnce(&mut CompositionTreeState) -> Result<T>,
    ) -> std::result::Result<T, ViewportVisualSlotError> {
        let tree = self
            .tree
            .upgrade()
            .ok_or(ViewportVisualSlotError::Invalidated)?;
        operation(&mut tree.borrow_mut())
            .map_err(|error| ViewportVisualSlotError::Composition(format!("{error:#}")))
    }

    /// Stage the authoritative integer rectangle, transform, clip, and
    /// visibility. Call [`Self::commit`] once after all changes for the scene.
    pub fn update_layout(
        &self,
        layout: ViewportCompositionLayout,
    ) -> std::result::Result<(), ViewportVisualSlotError> {
        if layout.window_id != self.window_id {
            return Err(ViewportVisualSlotError::WrongWindow {
                expected: self.window_id,
                actual: layout.window_id,
            });
        }
        if layout.slot_id != PRIMARY_VIEWPORT_VISUAL_SLOT {
            return Err(ViewportVisualSlotError::WrongSlot(layout.slot_id));
        }
        let incoming = layout.scene_generation;
        self.with_state(|state| {
            if incoming < state.committed_generation {
                return Err(anyhow::anyhow!(ViewportVisualSlotError::StaleLayout {
                    committed: state.committed_generation,
                    incoming,
                }));
            }
            state.update_layout(layout)
        })
        .map_err(|error| match error {
            ViewportVisualSlotError::Composition(message) if message.contains("older than") => {
                ViewportVisualSlotError::StaleLayout {
                    committed: self
                        .tree
                        .upgrade()
                        .map_or(0, |tree| tree.borrow().committed_generation),
                    incoming,
                }
            }
            other => other,
        })
    }

    /// Stage the child visual as visible. It remains empty in phases 0/1.
    pub fn show(&self) -> std::result::Result<(), ViewportVisualSlotError> {
        self.with_state(|state| state.set_visible(true))
    }

    /// Stage the child visual as hidden.
    pub fn hide(&self) -> std::result::Result<(), ViewportVisualSlotError> {
        self.with_state(|state| state.set_visible(false))
    }

    /// Atomically submit all staged DirectComposition changes.
    pub fn commit(&self) -> std::result::Result<(), ViewportVisualSlotError> {
        self.with_state(CompositionTreeState::commit)
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.tree.upgrade().is_some()
    }

    /// Obtain the typed surface-creation capability for this renderer
    /// generation. No process-global raw pointer is published.
    pub fn surface_target(
        &self,
    ) -> std::result::Result<ViewportVisualSurfaceTarget, ViewportVisualSlotError> {
        self.with_state(|state| {
            Ok(ViewportVisualSurfaceTarget {
                device: state.device.clone(),
                visual: state.active_surface_visual.clone(),
                bridge: state.surface_bridge.clone(),
                surface_id: state.active_surface_id,
                first_presented: Arc::new(AtomicBool::new(false)),
            })
        })
    }
}

pub(crate) struct DirectComposition {
    tree: Rc<RefCell<CompositionTreeState>>,
}

impl DirectComposition {
    pub(crate) fn new(dxgi_device: &IDXGIDevice, hwnd: HWND) -> Result<Self> {
        let device: IDCompositionDevice = unsafe { DCompositionCreateDevice(dxgi_device) }
            .context("creating DirectComposition device")?;
        let target = unsafe { device.CreateTargetForHwnd(hwnd, true) }
            .context("creating DirectComposition HWND target")?;
        Ok(Self {
            tree: Rc::new(RefCell::new(CompositionTreeState::new(device, target)?)),
        })
    }

    pub(crate) fn set_swap_chain(&self, swap_chain: &IDXGISwapChain1) -> Result<()> {
        self.tree.borrow_mut().set_gpui_swap_chain(swap_chain)
    }

    pub(crate) fn viewport_visual_slot(&self, window_id: u64) -> ViewportVisualSlot {
        self.tree.borrow_mut().bind_window(window_id);
        ViewportVisualSlot {
            tree: Rc::downgrade(&self.tree),
            window_id,
        }
    }

    pub(crate) fn update_composition_hole(
        &self,
        slot_id: u64,
        scene_generation: u64,
        device_rect: ViewportDeviceRect,
        corner_radii: [f32; 4],
    ) -> Result<()> {
        if slot_id != PRIMARY_VIEWPORT_VISUAL_SLOT.0 {
            anyhow::bail!("unknown viewport visual slot {slot_id}");
        }
        let window_id = self.tree.borrow().window_id.unwrap_or(0);
        self.tree
            .borrow_mut()
            .update_layout(ViewportCompositionLayout {
            window_id,
            slot_id: PRIMARY_VIEWPORT_VISUAL_SLOT,
            scene_generation,
            device_rect,
            scale_factor: 1.0,
            visible: !device_rect.is_empty(),
            corner_radii,
        })
    }

    pub(crate) fn hide_composition_hole(&self) -> Result<()> {
        self.tree.borrow_mut().set_visible(false)
    }

    pub(crate) fn commit_composition_batch(&self) -> Result<()> {
        self.tree.borrow_mut().commit()
    }

    pub(crate) fn composition_hole_layout_changed(&self) -> bool {
        self.tree.borrow().composition_hole_layout_changed()
    }

    pub(crate) fn acknowledge_surface_handoff(&self) -> Result<()> {
        self.tree.borrow_mut().acknowledge_surface_handoff()
    }
}

pub(crate) struct CompositionTreeState {
    device: IDCompositionDevice,
    target: IDCompositionTarget,
    root_visual: IDCompositionVisual,
    gpui_visual: IDCompositionVisual,
    bevy_visual: IDCompositionVisual,
    bevy_content_host: IDCompositionVisual,
    bevy_clip: IDCompositionRectangleClip,
    bevy_scale: IDCompositionScaleTransform,
    active_surface_visual: IDCompositionVisual,
    active_surface_id: u64,
    active_surface_extent: Option<(u32, u32)>,
    staged_ready_surface_id: Option<u64>,
    pending_surface_handoff_id: Option<u64>,
    surface_bridge: Arc<CompositionSurfaceBridge>,
    window_id: Option<u64>,
    staged_generation: u64,
    committed_generation: u64,
    dirty: bool,
    staged_rect: ViewportDeviceRect,
    committed_rect: ViewportDeviceRect,
    staged_corner_radii: [f32; 4],
    requested_visible: bool,
    staged_visible: bool,
    committed_visible: bool,
}

impl CompositionTreeState {
    fn new(device: IDCompositionDevice, target: IDCompositionTarget) -> Result<Self> {
        let root_visual = unsafe { device.CreateVisual() }.context("creating DComp root visual")?;
        let gpui_visual = unsafe { device.CreateVisual() }.context("creating DComp GPUI visual")?;
        let bevy_visual = unsafe { device.CreateVisual() }.context("creating DComp Bevy visual")?;
        let bevy_content_host =
            unsafe { device.CreateVisual() }.context("creating DComp viewport content host")?;
        let active_surface_visual =
            unsafe { device.CreateVisual() }.context("creating initial viewport content visual")?;
        let bevy_clip = unsafe { device.CreateRectangleClip() }
            .context("creating DComp viewport rectangle clip")?;
        let bevy_scale = unsafe { device.CreateScaleTransform() }
            .context("creating DComp viewport scale transform")?;

        unsafe {
            bevy_visual.SetClip(&bevy_clip)?;
            bevy_content_host.SetTransform(&bevy_scale)?;
            bevy_clip.SetRight2(0.0)?;
            bevy_clip.SetBottom2(0.0)?;
            bevy_content_host.AddVisual(
                &active_surface_visual,
                false,
                None::<&IDCompositionVisual>,
            )?;
            bevy_visual.AddVisual(&bevy_content_host, false, None::<&IDCompositionVisual>)?;
            // Explicit ordering: Bevy first, then GPUI above it.
            root_visual.AddVisual(&bevy_visual, false, None::<&IDCompositionVisual>)?;
            root_visual.AddVisual(&gpui_visual, true, &bevy_visual)?;
            target.SetRoot(&root_visual)?;
        }
        crate::viewport_bridge::record_viewport_perf("dcomp.target_created", 1);
        crate::viewport_bridge::record_viewport_perf("dcomp.root_created", 1);
        crate::viewport_bridge::record_viewport_perf("dcomp.capability_created", 1);

        Ok(Self {
            device,
            target,
            root_visual,
            gpui_visual,
            bevy_visual,
            bevy_content_host,
            bevy_clip,
            bevy_scale,
            active_surface_visual,
            active_surface_id: 1,
            active_surface_extent: None,
            staged_ready_surface_id: None,
            pending_surface_handoff_id: None,
            surface_bridge: Arc::new(CompositionSurfaceBridge::new(1)),
            window_id: None,
            staged_generation: 0,
            committed_generation: 0,
            dirty: true,
            staged_rect: ViewportDeviceRect::default(),
            committed_rect: ViewportDeviceRect::default(),
            staged_corner_radii: [0.0; 4],
            requested_visible: false,
            staged_visible: false,
            committed_visible: false,
        })
    }

    fn set_gpui_swap_chain(&mut self, swap_chain: &IDXGISwapChain1) -> Result<()> {
        unsafe {
            self.gpui_visual.SetContent(swap_chain)?;
        }
        self.dirty = true;
        self.commit()
    }

    fn bind_window(&mut self, window_id: u64) {
        debug_assert!(
            self.window_id.is_none_or(|bound| bound == window_id),
            "a composition tree cannot be rebound to another GPUI window"
        );
        self.window_id = Some(window_id);
    }

    fn stage_ready_surface(&mut self) -> Result<()> {
        let Some(ready) = self.surface_bridge.take_ready() else {
            return Ok(());
        };
        if ready.id < self.active_surface_id {
            return Ok(());
        }
        if ready.id != self.active_surface_id {
            unsafe {
                self.bevy_content_host.AddVisual(
                    &ready.visual,
                    false,
                    None::<&IDCompositionVisual>,
                )?;
                self.bevy_content_host
                    .RemoveVisual(&self.active_surface_visual)?;
            }
            self.active_surface_visual = ready.visual;
            self.active_surface_id = ready.id;
        }
        self.active_surface_extent = Some(ready.extent);
        self.staged_ready_surface_id = Some(ready.id);
        let width = self.staged_rect.width();
        let height = self.staged_rect.height();
        let visible = self.requested_visible && !self.staged_rect.is_empty();
        unsafe {
            self.bevy_scale
                .SetScaleX2(width as f32 / ready.extent.0.max(1) as f32)?;
            self.bevy_scale
                .SetScaleY2(height as f32 / ready.extent.1.max(1) as f32)?;
            self.bevy_clip
                .SetRight2(if visible { width as f32 } else { 0.0 })?;
            self.bevy_clip
                .SetBottom2(if visible { height as f32 } else { 0.0 })?;
        }
        self.staged_visible = visible;
        self.dirty = true;
        Ok(())
    }

    fn update_layout(&mut self, layout: ViewportCompositionLayout) -> Result<()> {
        if let Some(expected) = self.window_id
            && layout.window_id != expected
        {
            anyhow::bail!(ViewportVisualSlotError::WrongWindow {
                expected,
                actual: layout.window_id,
            });
        }
        if layout.scene_generation < self.committed_generation {
            anyhow::bail!(ViewportVisualSlotError::StaleLayout {
                committed: self.committed_generation,
                incoming: layout.scene_generation,
            });
        }
        self.stage_ready_surface()?;
        let rect = layout.device_rect;
        let width = rect.width();
        let height = rect.height();
        debug_assert_eq!(width as i64, i64::from(rect.right) - i64::from(rect.left));
        debug_assert_eq!(height as i64, i64::from(rect.bottom) - i64::from(rect.top));
        self.requested_visible = layout.visible && !rect.is_empty();
        let visible = self.requested_visible && self.active_surface_extent.is_some();
        let (scale_x, scale_y) = self.active_surface_extent.map_or((1.0, 1.0), |extent| {
            (
                width as f32 / extent.0.max(1) as f32,
                height as f32 / extent.1.max(1) as f32,
            )
        });
        let property_changed = rect != self.staged_rect
            || visible != self.staged_visible
            || layout.corner_radii != self.staged_corner_radii;
        if property_changed {
            unsafe {
                self.bevy_visual.SetOffsetX2(rect.left as f32)?;
                self.bevy_visual.SetOffsetY2(rect.top as f32)?;
                self.bevy_clip.SetLeft2(0.0)?;
                self.bevy_clip.SetTop2(0.0)?;
                self.bevy_clip
                    .SetRight2(if visible { width as f32 } else { 0.0 })?;
                self.bevy_clip
                    .SetBottom2(if visible { height as f32 } else { 0.0 })?;
                self.bevy_scale.SetScaleX2(scale_x)?;
                self.bevy_scale.SetScaleY2(scale_y)?;
                self.bevy_scale.SetCenterX2(0.0)?;
                self.bevy_scale.SetCenterY2(0.0)?;
                let [top_left, top_right, bottom_right, bottom_left] = layout.corner_radii;
                self.bevy_clip.SetTopLeftRadiusX2(top_left)?;
                self.bevy_clip.SetTopLeftRadiusY2(top_left)?;
                self.bevy_clip.SetTopRightRadiusX2(top_right)?;
                self.bevy_clip.SetTopRightRadiusY2(top_right)?;
                self.bevy_clip.SetBottomRightRadiusX2(bottom_right)?;
                self.bevy_clip.SetBottomRightRadiusY2(bottom_right)?;
                self.bevy_clip.SetBottomLeftRadiusX2(bottom_left)?;
                self.bevy_clip.SetBottomLeftRadiusY2(bottom_left)?;
            }
            self.dirty = true;
        }
        self.staged_rect = rect;
        self.staged_corner_radii = layout.corner_radii;
        self.staged_visible = visible;
        self.staged_generation = layout.scene_generation;
        Ok(())
    }

    fn set_visible(&mut self, visible: bool) -> Result<()> {
        self.stage_ready_surface()?;
        self.requested_visible = visible && !self.staged_rect.is_empty();
        let visible = self.requested_visible && self.active_surface_extent.is_some();
        let width = self.staged_rect.width() as f32;
        let height = self.staged_rect.height() as f32;
        unsafe {
            self.bevy_clip
                .SetRight2(if visible { width } else { 0.0 })?;
            self.bevy_clip
                .SetBottom2(if visible { height } else { 0.0 })?;
        }
        self.staged_visible = visible;
        self.dirty = true;
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        self.stage_ready_surface()?;
        if self.dirty {
            unsafe { self.device.Commit() }.context("committing viewport composition tree")?;
            self.committed_generation = self.staged_generation;
            self.committed_rect = self.staged_rect;
            self.committed_visible = self.staged_visible;
            self.dirty = false;
            if let Some(surface_id) = self.staged_ready_surface_id.take() {
                self.pending_surface_handoff_id = Some(surface_id);
                crate::viewport_bridge::record_viewport_perf("dcomp.surface_handoff_submitted", 1);
            }
            if self.staged_visible && self.staged_generation != 0 {
                crate::viewport_bridge::record_dcomp_visual_commit(
                    PRIMARY_VIEWPORT_VISUAL_SLOT.0,
                    self.staged_generation,
                    self.staged_rect,
                );
            }
            crate::viewport_bridge::record_viewport_perf("dcomp.layout_commit", 1);
        } else {
            self.committed_generation = self.committed_generation.max(self.staged_generation);
        }
        Ok(())
    }

    fn acknowledge_surface_handoff(&mut self) -> Result<()> {
        let Some(surface_id) = self.pending_surface_handoff_id else {
            return Ok(());
        };
        unsafe { self.device.WaitForCommitCompletion() }
            .context("waiting for viewport surface handoff commit")?;
        self.surface_bridge
            .committed_surface_id
            .store(surface_id, Ordering::Release);
        self.pending_surface_handoff_id = None;
        crate::viewport_bridge::record_viewport_perf("dcomp.surface_handoff_commit", 1);
        Ok(())
    }

    fn composition_hole_layout_changed(&self) -> bool {
        self.dirty
            && (self.staged_rect != self.committed_rect
                || self.staged_visible != self.committed_visible)
    }
}

impl Drop for CompositionTreeState {
    fn drop(&mut self) {
        // Holding these fields documents the one-target/one-root ownership even
        // though COM releases them automatically after this callback returns.
        let _ = (&self.target, &self.root_visual);
        crate::viewport_bridge::record_viewport_perf("dcomp.capability_invalidated", 1);
        crate::viewport_bridge::record_viewport_perf("dcomp.capability_dropped", 1);
        crate::viewport_bridge::record_viewport_perf("dcomp.root_dropped", 1);
        crate::viewport_bridge::record_viewport_perf("dcomp.target_dropped", 1);
    }
}

#[cfg(test)]
mod tests {
    use super::{ViewportDeviceRect, ViewportVisualSlot, ViewportVisualSlotError};
    use std::rc::Weak;

    #[test]
    fn device_extent_is_derived_from_edges() {
        let rect = ViewportDeviceRect {
            left: 101,
            top: 37,
            right: 904,
            bottom: 638,
        };
        assert_eq!(rect.width(), 803);
        assert_eq!(rect.height(), 601);
    }

    #[test]
    fn stale_capability_cannot_commit() {
        let slot = ViewportVisualSlot {
            tree: Weak::new(),
            window_id: 7,
        };
        assert_eq!(slot.commit(), Err(ViewportVisualSlotError::Invalidated));
    }
}
