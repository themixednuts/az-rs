//! Vegetation render synchronization systems.

mod fallback_render;
mod scene_asset;
mod scene_roots;
mod transforms;

pub(super) use fallback_render::sync_instance_fallback_rendering;
pub(super) use scene_roots::sync_instance_scene_roots;
pub(super) use transforms::sync_instance_transforms;
