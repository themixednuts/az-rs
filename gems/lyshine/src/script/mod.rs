//! `LyShine` native bus handlers.
//!
//! O3DE reference: `Gems/LyShine/Code/Source/LyShineSystemComponent.cpp` (the C++
//! reflection of `UiElementBus`, `UiCanvasBus`, and related public buses).
//! Each handler attaches itself to
//! [`az_framework::ebus::EBusRegistry`] at Bevy `Startup` and exposes the
//! method surface used by `LyShine` Lua scripts.
//!
//! Per-bus modules:
//!
//! - [`manager`] — `LyShineManagerBus`
//! - [`script_bind_request`] — `LyShineScriptBindRequestBus`
//! - [`ui_canvas`] — `UiCanvasBus`
//! - [`ui_element`] — `UiElementBus`
//!
//! Supporting modules:
//!
//! - [`bms_bridge`] — mounts a BMS `ScriptComponent` on every Bevy entity
//!   carrying a [`crate::LyShineUiScriptBinding`].
//! - [`entity_index`] — `UiEntityId → Bevy Entity` lookup that the
//!   per-element bus handlers consult to resolve script-passed addresses.

pub mod bms_bridge;
mod convert;
pub mod entity_index;
pub mod lua_init;
pub mod manager;
pub mod script_bind_request;
pub mod ui_canvas;
pub mod ui_element;
pub mod ui_image;
pub mod ui_text;
pub mod ui_transform;

pub use bms_bridge::LyShineBmsBridgePlugin;
pub use entity_index::{LyShineEntityIndex, LyShineEntityIndexPlugin};
pub use lua_init::{LYSHINE_LUA_BUSES, add_lyshine_bus_init, lyshine_bus_init};

use bevy::prelude::*;

/// Aggregate plugin: adds the entity index, every implemented `LyShine`
/// bus handler, and the BMS bridge attach system. Pull in from the
/// client app composition.
pub struct LyShineScriptingPlugin;

impl Plugin for LyShineScriptingPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(LyShineEntityIndexPlugin);
        app.add_plugins(manager::LyShineManagerPlugin);
        app.add_plugins(script_bind_request::LyShineScriptBindRequestPlugin);
        app.add_plugins(ui_canvas::UiCanvasPlugin);
        app.add_plugins(ui_element::UiElementPlugin);
        app.add_plugins(ui_image::UiImagePlugin);
        app.add_plugins(ui_text::UiTextPlugin);
        app.add_plugins(ui_transform::UiTransformPlugin);
        app.add_plugins(LyShineBmsBridgePlugin);
    }
}
