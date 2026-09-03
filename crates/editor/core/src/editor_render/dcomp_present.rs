// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Resource, Clone)]
pub(super) struct DcompPresentFence {
    queue: ID3D12CommandQueue,
    fence: ID3D12Fence,
    next_submission_id: Arc<AtomicU64>,
    next_present_id: Arc<AtomicU64>,
}

impl DcompPresentFence {
    pub(super) fn new(
        render_device: &RenderDevice,
        render_queue: &RenderQueue,
    ) -> Result<Self, String> {
        let device_ptr = unsafe {
            render_device
                .wgpu_device()
                .as_hal::<wgpu::hal::api::Dx12>()
                .map(|device| com_ptr(device.raw_device()))
        }
        .ok_or_else(|| "Bevy render device is not D3D12".to_owned())?;
        let queue_ptr = unsafe {
            render_queue
                .as_hal::<wgpu::hal::api::Dx12>()
                .map(|queue| com_ptr(queue.as_raw()))
        }
        .ok_or_else(|| "Bevy render queue is not D3D12".to_owned())?;
        let device = unsafe { ID3D12Device::from_raw_borrowed(&device_ptr) }
            .ok_or_else(|| "D3D12 device pointer was null".to_owned())?;
        let queue = unsafe { ID3D12CommandQueue::from_raw_borrowed(&queue_ptr) }
            .ok_or_else(|| "D3D12 command queue pointer was null".to_owned())?
            .clone();
        let fence = unsafe { device.CreateFence::<ID3D12Fence>(0, D3D12_FENCE_FLAG_NONE) }
            .map_err(|error| format!("{error:?}"))?;
        Ok(Self {
            queue,
            fence,
            next_submission_id: Arc::new(AtomicU64::new(1)),
            next_present_id: Arc::new(AtomicU64::new(1)),
        })
    }
}

pub(super) struct AcquiredCompositionFrame {
    pub(super) texture: Mutex<Option<wgpu::SurfaceTexture>>,
    pub(super) extent: (u32, u32),
    pub(super) surface_target: gpui_windows::ViewportVisualSurfaceTarget,
    pub(super) display_telemetry: Arc<DcompDisplayTelemetry>,
    pub(super) pointer_sample: Mutex<Option<PointerPresentSample>>,
    pub(super) completed: Mutex<bool>,
    pub(super) completed_changed: Condvar,
}

impl AcquiredCompositionFrame {
    pub(super) const fn new(
        texture: wgpu::SurfaceTexture,
        extent: (u32, u32),
        surface_target: gpui_windows::ViewportVisualSurfaceTarget,
        display_telemetry: Arc<DcompDisplayTelemetry>,
        pointer_sample: Option<PointerPresentSample>,
    ) -> Self {
        Self {
            texture: Mutex::new(Some(texture)),
            extent,
            surface_target,
            display_telemetry,
            pointer_sample: Mutex::new(pointer_sample),
            completed: Mutex::new(false),
            completed_changed: Condvar::new(),
        }
    }

    pub(super) fn finish(&self) {
        *self
            .completed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.completed_changed.notify_all();
    }

    pub(super) fn wait(&self, timeout: Duration) -> bool {
        let completed = self
            .completed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *completed {
            return true;
        }
        let (completed, _) = self
            .completed_changed
            .wait_timeout_while(completed, timeout, |completed| !*completed)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *completed
    }

    pub(super) fn is_finished(&self) -> bool {
        *self
            .completed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PointerPresentSample {
    pub(super) interaction_id: u64,
    pub(super) sampled_at: std::time::Instant,
}

impl Drop for AcquiredCompositionFrame {
    fn drop(&mut self) {
        if self
            .texture
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .is_some()
        {
            // Dropping an unpresented wgpu SurfaceTexture calls texture_discard.
            gpui_windows::record_dcomp_texture_discarded();
        }
    }
}

#[derive(Resource, Clone, ExtractResource)]
pub(super) struct PendingCompositionFrame(pub(super) Option<Arc<AcquiredCompositionFrame>>);

pub(super) fn present_composition_frame(
    pending: Option<Res<PendingCompositionFrame>>,
    present_fence: Option<Res<DcompPresentFence>>,
) {
    let (Some(pending), Some(present_fence)) = (pending, present_fence) else {
        return;
    };
    let Some(frame) = pending.0.clone() else {
        return;
    };
    let texture = frame
        .texture
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let Some(texture) = texture else {
        return;
    };

    // Bevy's renderer submits its final command encoder in RenderSystems::Render.
    // This system runs in PostCleanup, so this raw queue signal is ordered after
    // every command buffer that can reference the acquired surface texture.
    let submission_id = present_fence
        .next_submission_id
        .fetch_add(1, Ordering::Relaxed);
    if let Err(error) = unsafe {
        present_fence
            .queue
            .Signal(&present_fence.fence, submission_id)
    } {
        tracing::error!(
            ?error,
            submission_id,
            "D3D12 post-submit fence signal failed"
        );
        gpui_windows::record_dcomp_texture_discarded();
        drop(texture);
        frame.finish();
        return;
    }
    let completed_fence_value = unsafe { present_fence.fence.GetCompletedValue() };
    let present_id = present_fence
        .next_present_id
        .fetch_add(1, Ordering::Relaxed);
    let present_started = std::time::Instant::now();
    texture.present();
    crate::perf::record_elapsed("frame.bevy_present_api", present_started);
    frame.display_telemetry.present_submitted();
    gpui_windows::record_dcomp_texture_presented(
        frame.extent,
        present_id,
        submission_id,
        submission_id,
        completed_fence_value,
    );
    let pointer_sample = frame
        .pointer_sample
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(sample) = pointer_sample {
        crate::perf::camera_drag_sample_presented(sample.interaction_id, sample.sampled_at);
    }
    if frame.surface_target.mark_presented(frame.extent) {
        gpui_windows::request_immediate_frame();
    }
    frame.finish();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DcompPresentPolicy {
    Fifo { latency: u32 },
    Mailbox { latency: u32 },
    Immediate { latency: u32 },
}

impl DcompPresentPolicy {
    fn from_env() -> Result<Self, EditorRenderInitError> {
        let value = match std::env::var("AZOTH_EDITOR_VIEWPORT_PRESENT_POLICY") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => "immediate-2".to_owned(),
            Err(error) => {
                return Err(EditorRenderInitError::CompositionSurface(format!(
                    "viewport present policy is not valid Unicode: {error}"
                )));
            }
        };
        let policy = match value.to_ascii_lowercase().as_str() {
            "fifo-1" => Self::Fifo { latency: 1 },
            "fifo-2" => Self::Fifo { latency: 2 },
            "mailbox-1" => Self::Mailbox { latency: 1 },
            "mailbox-2" => Self::Mailbox { latency: 2 },
            "immediate-1" => Self::Immediate { latency: 1 },
            "immediate-2" => Self::Immediate { latency: 2 },
            other => {
                return Err(EditorRenderInitError::CompositionSurface(format!(
                    "unknown viewport present policy {other:?}; expected fifo-1, fifo-2, mailbox-1, mailbox-2, immediate-1, or immediate-2"
                )));
            }
        };
        Ok(policy)
    }

    const fn present_mode(self) -> wgpu::PresentMode {
        match self {
            Self::Fifo { .. } => wgpu::PresentMode::Fifo,
            Self::Mailbox { .. } => wgpu::PresentMode::Mailbox,
            Self::Immediate { .. } => wgpu::PresentMode::Immediate,
        }
    }

    const fn latency(self) -> u32 {
        match self {
            Self::Fifo { latency } | Self::Mailbox { latency } | Self::Immediate { latency } => {
                latency
            }
        }
    }
}

pub(super) struct RetiredCompositionSurface {
    _surface: wgpu::Surface<'static>,
    _target: gpui_windows::ViewportVisualSurfaceTarget,
    until_committed: gpui_windows::ViewportVisualSurfaceTarget,
}

const DCOMP_PRESENT_HISTORY_CAPACITY: usize = 256;

#[derive(Clone, Copy, Default)]
pub(super) struct DcompPresentSubmission {
    present_count: u32,
    submitted_qpc: i64,
}

pub(super) struct DcompDisplayTelemetryInner {
    swap_chain: IDXGISwapChain3,
    media_swap_chain: Option<IDXGISwapChainMedia>,
    history: [DcompPresentSubmission; DCOMP_PRESENT_HISTORY_CAPACITY],
    history_next: usize,
    previous_statistics: Option<DXGI_FRAME_STATISTICS>,
    last_displayed_present: u32,
    qpc_ticks_per_refresh: f64,
    qpc_frequency: i64,
    statistics_failure_reported: bool,
}

/// DXGI timestamps from the Bevy composition swapchain itself. GPUI has an
/// independent swapchain/cadence, so its frame statistics cannot establish
/// whether the child visual missed a display opportunity.
pub(super) struct DcompDisplayTelemetry(Mutex<DcompDisplayTelemetryInner>);

impl DcompDisplayTelemetry {
    fn new(surface: &wgpu::Surface<'_>) -> Result<Arc<Self>, EditorRenderInitError> {
        let swap_chain = unsafe {
            surface
                .as_hal::<wgpu::hal::api::Dx12>()
                .and_then(|surface| surface.swap_chain())
        }
        .ok_or_else(|| {
            EditorRenderInitError::CompositionSurface(
                "Bevy DirectComposition surface did not expose its DXGI swapchain".to_owned(),
            )
        })?;
        let mut qpc_frequency = 0;
        unsafe { QueryPerformanceFrequency(&raw mut qpc_frequency) }.map_err(|error| {
            EditorRenderInitError::CompositionSurface(format!(
                "failed to query the DXGI performance-counter frequency: {error:?}"
            ))
        })?;
        Ok(Arc::new(Self(Mutex::new(DcompDisplayTelemetryInner {
            media_swap_chain: swap_chain.cast().ok(),
            swap_chain,
            history: [DcompPresentSubmission::default(); DCOMP_PRESENT_HISTORY_CAPACITY],
            history_next: 0,
            previous_statistics: None,
            last_displayed_present: 0,
            qpc_ticks_per_refresh: 0.0,
            qpc_frequency,
            statistics_failure_reported: false,
        }))))
    }

    fn present_submitted(&self) {
        let mut submitted_qpc = 0;
        if let Err(error) = unsafe { QueryPerformanceCounter(&raw mut submitted_qpc) } {
            tracing::warn!(?error, "failed to timestamp Bevy composition present");
            return;
        }
        let mut inner = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Ok(present_count) = (unsafe { inner.swap_chain.GetLastPresentCount() }) else {
            return;
        };
        let history_next = inner.history_next;
        inner.history[history_next] = DcompPresentSubmission {
            present_count,
            submitted_qpc,
        };
        inner.history_next = (history_next + 1) % DCOMP_PRESENT_HISTORY_CAPACITY;
        inner.record_displayed_frame_statistics();
    }
}

impl DcompDisplayTelemetryInner {
    fn record_displayed_frame_statistics(&mut self) {
        let mut statistics = DXGI_FRAME_STATISTICS::default();
        let standard_result = unsafe { self.swap_chain.GetFrameStatistics(&raw mut statistics) };
        if standard_result.is_err() || statistics.PresentCount == 0 {
            let media_result = self.media_swap_chain.as_ref().map(|swap_chain| {
                let mut media = DXGI_FRAME_STATISTICS_MEDIA::default();
                let result = unsafe { swap_chain.GetFrameStatisticsMedia(&raw mut media) };
                if result.is_ok() {
                    statistics = DXGI_FRAME_STATISTICS {
                        PresentCount: media.PresentCount,
                        PresentRefreshCount: media.PresentRefreshCount,
                        SyncRefreshCount: media.SyncRefreshCount,
                        SyncQPCTime: media.SyncQPCTime,
                        SyncGPUTime: media.SyncGPUTime,
                    };
                }
                result
            });
            if statistics.PresentCount == 0 {
                if !self.statistics_failure_reported {
                    self.statistics_failure_reported = true;
                    tracing::warn!(
                        ?standard_result,
                        ?media_result,
                        "DXGI display statistics unavailable for Bevy composition swapchain"
                    );
                }
                return;
            }
        }
        if statistics.PresentCount == self.last_displayed_present {
            return;
        }

        if let Some(previous) = self.previous_statistics {
            let refreshes = statistics
                .SyncRefreshCount
                .wrapping_sub(previous.SyncRefreshCount);
            crate::perf::camera_drag_display_interval(refreshes);
            let qpc_ticks = statistics.SyncQPCTime - previous.SyncQPCTime;
            if refreshes > 0 && qpc_ticks > 0 {
                self.qpc_ticks_per_refresh = counter_ticks_f64(qpc_ticks) / f64::from(refreshes);
            }
        }
        self.previous_statistics = Some(statistics);
        self.last_displayed_present = statistics.PresentCount;

        if self.qpc_ticks_per_refresh <= 0.0 {
            return;
        }
        let Some(submission) = self
            .history
            .iter()
            .find(|submission| submission.present_count == statistics.PresentCount)
        else {
            return;
        };
        let refresh_delta =
            i64::from(statistics.PresentRefreshCount) - i64::from(statistics.SyncRefreshCount);
        // Differencing in integer ticks first keeps the raw QPC timestamps out
        // of `f64` entirely; only the small submit-to-sync delta is widened.
        let submit_to_sync = statistics.SyncQPCTime - submission.submitted_qpc;
        let latency_qpc = counter_ticks_f64(refresh_delta).mul_add(
            self.qpc_ticks_per_refresh,
            counter_ticks_f64(submit_to_sync),
        );
        if latency_qpc < 0.0 {
            return;
        }
        let latency_ns = latency_qpc * 1_000_000_000.0 / counter_ticks_f64(self.qpc_frequency);
        if latency_ns <= 250_000_000.0 {
            crate::perf::record_ns(
                "frame.bevy_present_to_displayed_composition",
                bounded_nanoseconds(latency_ns),
            );
        }
    }
}

/// Widen a performance-counter tick delta (or the counter frequency itself) to
/// `f64`.
///
/// Every value reaching this helper is a delta across at most a handful of
/// display refreshes, or the QPC frequency itself (10 MHz on current Windows
/// hardware). Both stay far inside `i32`, so the saturating fallbacks are
/// unreachable and exist only to keep the widening a checked conversion.
fn counter_ticks_f64(ticks: i64) -> f64 {
    let saturated = if ticks < 0 { i32::MIN } else { i32::MAX };
    f64::from(i32::try_from(ticks).unwrap_or(saturated))
}

/// Round a display latency in nanoseconds to whole nanoseconds.
// The caller only records latencies in `0.0..=250_000_000.0`, so neither the
// truncation nor the sign case this narrowing warns about is reachable.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
const fn bounded_nanoseconds(latency_ns: f64) -> u64 {
    latency_ns.round().clamp(0.0, 250_000_000.0) as u64
}

pub(super) struct CompositionSurfaceOwner {
    pub(super) surface: wgpu::Surface<'static>,
    pub(super) config: wgpu::SurfaceConfiguration,
    pub(super) target: gpui_windows::ViewportVisualSurfaceTarget,
    pub(super) display_telemetry: Arc<DcompDisplayTelemetry>,
    pub(super) render_instance: RenderInstance,
    pub(super) previous_frame: Option<Arc<AcquiredCompositionFrame>>,
    pub(super) needs_reconfigure: bool,
    pub(super) retired: Vec<RetiredCompositionSurface>,
}

impl CompositionSurfaceOwner {
    pub(super) fn create_surface(
        render_instance: &RenderInstance,
        target: &gpui_windows::ViewportVisualSurfaceTarget,
    ) -> Result<wgpu::Surface<'static>, EditorRenderInitError> {
        // SAFETY: ViewportVisualSurfaceTarget strongly owns this exact visual;
        // the DX12 backend AddRefs it while creating the surface.
        unsafe {
            render_instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CompositionVisual(
                target.as_raw(),
            ))
        }
        .map_err(|error| EditorRenderInitError::CompositionSurface(format!("{error:?}")))
    }

    pub(super) fn new(
        render_instance: RenderInstance,
        render_device: &RenderDevice,
        target: gpui_windows::ViewportVisualSurfaceTarget,
        width: u32,
        height: u32,
    ) -> Result<Self, EditorRenderInitError> {
        let surface = Self::create_surface(&render_instance, &target)?;
        let policy = DcompPresentPolicy::from_env()?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            width: width.max(1),
            height: height.max(1),
            present_mode: policy.present_mode(),
            desired_maximum_frame_latency: policy.latency(),
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: Vec::new(),
        };
        surface.configure(render_device.wgpu_device(), &config);
        let display_telemetry = DcompDisplayTelemetry::new(&surface)?;
        tracing::info!(
            ?policy,
            present_mode = ?config.present_mode,
            maximum_frame_latency = config.desired_maximum_frame_latency,
            "configured independent Bevy DirectComposition present policy"
        );
        Ok(Self {
            surface,
            config,
            target,
            display_telemetry,
            render_instance,
            previous_frame: None,
            needs_reconfigure: false,
            retired: Vec::new(),
        })
    }

    pub(super) fn wait_for_previous(&self) -> bool {
        self.previous_frame
            .as_ref()
            .is_none_or(|frame| frame.wait(Duration::from_secs(5)))
    }

    pub(super) fn is_idle(&self) -> bool {
        self.previous_frame
            .as_ref()
            .is_none_or(|frame| frame.is_finished())
    }

    pub(super) fn retire_committed_surfaces(&mut self) {
        self.retired
            .retain(|surface| !surface.until_committed.is_committed());
    }

    pub(super) fn configure(
        &mut self,
        render_device: &RenderDevice,
        width: u32,
        height: u32,
    ) -> bool {
        self.retire_committed_surfaces();
        if !self.is_idle() {
            gpui_windows::record_dcomp_reconfigure_while_acquired();
            debug_assert!(false, "composition surface reconfigured while acquired");
            return false;
        }
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        // `ManualTextureViews` intentionally keeps the just-presented texture
        // view alive until the next extraction. DXGI therefore rejects
        // ResizeBuffers on this surface with DXGI_ERROR_INVALID_CALL even
        // though the SurfaceTexture itself has reached its terminal present.
        // Create a fresh composition swapchain for the same visual instead;
        // SetContent atomically replaces the old swapchain while its stale
        // views drain from the two Bevy worlds on the next extraction.
        let replacement_target = match self.target.replacement() {
            Ok(target) => target,
            Err(error) => {
                tracing::error!(%error, "failed to create replacement composition visual");
                return false;
            }
        };
        let surface = match Self::create_surface(&self.render_instance, &replacement_target) {
            Ok(surface) => surface,
            Err(error) => {
                tracing::error!(%error, "failed to recreate DirectComposition surface for resize");
                return false;
            }
        };
        surface.configure(render_device.wgpu_device(), &self.config);
        let display_telemetry = match DcompDisplayTelemetry::new(&surface) {
            Ok(telemetry) => telemetry,
            Err(error) => {
                tracing::error!(%error, "failed to attach display telemetry to replacement surface");
                return false;
            }
        };
        let old_surface = std::mem::replace(&mut self.surface, surface);
        let old_target = std::mem::replace(&mut self.target, replacement_target.clone());
        self.display_telemetry = display_telemetry;
        self.retired.push(RetiredCompositionSurface {
            _surface: old_surface,
            _target: old_target,
            until_committed: replacement_target,
        });
        self.previous_frame = None;
        true
    }

    pub(super) fn recreate(
        &mut self,
        render_device: &RenderDevice,
    ) -> Result<(), EditorRenderInitError> {
        if !self.is_idle() {
            return Err(EditorRenderInitError::CompositionSurface(
                "timed out waiting to recreate a lost composition surface".to_owned(),
            ));
        }
        self.configure(render_device, self.config.width, self.config.height)
            .then_some(())
            .ok_or_else(|| {
                EditorRenderInitError::CompositionSurface(
                    "failed to replace a lost composition surface".to_owned(),
                )
            })
    }
}

/// Read the raw COM pointer out of any windows-rs interface reference without
/// needing that crate's exact `windows` version in scope. windows-rs interfaces
/// are `#[repr(transparent)]` over a non-null pointer (this is what
/// `Interface::as_raw` returns); reading the first pointer-sized field yields the
/// same value, version-agnostically.
#[inline]
pub(super) const unsafe fn com_ptr<T>(interface: &T) -> *mut core::ffi::c_void {
    unsafe { *std::ptr::from_ref::<T>(interface).cast::<*mut core::ffi::c_void>() }
}
