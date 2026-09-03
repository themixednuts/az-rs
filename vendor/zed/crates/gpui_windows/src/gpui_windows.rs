#![cfg(target_os = "windows")]

mod clipboard;
mod composition_tree;
mod destination_list;
mod direct_manipulation;
mod direct_write;
mod directx_atlas;
mod directx_devices;
mod directx_renderer;
mod dispatcher;
mod display;
mod events;
mod viewport_bridge;
mod keyboard;
mod platform;
mod system_settings;
mod util;
mod vsync;
mod window;
mod wrapper;

pub(crate) use clipboard::*;
pub(crate) use destination_list::*;
pub(crate) use direct_write::*;
pub(crate) use directx_atlas::*;
pub(crate) use directx_devices::*;
pub(crate) use directx_renderer::*;
pub(crate) use dispatcher::*;
pub(crate) use display::*;
pub(crate) use events::*;
pub(crate) use viewport_bridge::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
pub(crate) use system_settings::*;
pub(crate) use util::*;
pub(crate) use vsync::*;
pub(crate) use window::*;
pub(crate) use wrapper::*;

pub use composition_tree::{
    PRIMARY_VIEWPORT_VISUAL_SLOT, ViewportCompositionLayout, ViewportDeviceRect, ViewportVisualSlot,
    ViewportVisualSlotError, ViewportVisualSlotId, ViewportVisualSurfaceTarget,
};
pub use platform::WindowsPlatform;
pub use window::viewport_visual_slot;

/// DirectComposition viewport diagnostics and interaction bridge.
pub use viewport_bridge::{
    ViewportCompositorCounters, record_dcomp_reconfigure_while_acquired,
    record_dcomp_surface_replaced, record_dcomp_texture_acquired, record_dcomp_texture_discarded,
    record_dcomp_texture_presented, request_immediate_frame, sample_viewport_cursor,
    set_frame_presented_sink, set_immediate_frame_rate_hz, set_viewport_perf_sink,
    viewport_compositor_counters,
};

pub(crate) use windows::Win32::Foundation::HWND;
