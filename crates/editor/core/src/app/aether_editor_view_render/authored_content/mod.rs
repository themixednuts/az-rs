//! Authored-content regions split by scene/prefab, graph/animation, and inspection ownership.

mod graphs_animation;
mod inspector;
pub(in crate::app) mod scene_prefab;
pub(super) mod schema_presentation;

pub(super) use graphs_animation::{mode_button_style, scene_snap_style};
