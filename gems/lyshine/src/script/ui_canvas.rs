//! `UiCanvasBus` — per-canvas queries.
//!
//! O3DE reference: `Gems/LyShine/Code/Include/LyShine/Bus/UiCanvasBus.h`,
//! `BehaviorContext::EBus<UiCanvasBus>` reflection.
//!
//! Method coverage:
//!
//! - `Event.GetCanvasName(canvasId)` → returns the canvas's content path
//!   (for example, `"ui/intro/intro.uicanvas"`). The value must remain
//!   stable across sessions, so we serve
//!   [`crate::LyShineCanvasRoot::asset_path`] directly.
//!
//! ## Address handling
//!
//! `canvasId` is whatever `UiElementBus.Event.GetCanvas(entity)` returned.
//! For our flow that's the `UiEntityId` of the canvas entity (passed
//! through as [`EBusAddress::Integer`] by the Lua bridge). We resolve it
//! via [`super::entity_index::LyShineEntityIndex`] to a Bevy entity and
//! read its `LyShineCanvasRoot.asset_path`.

use std::sync::Arc;

use az_framework::ebus::{EBusAddress, EBusRegistry, NativeBusHandler};
use bevy::prelude::*;
use bevy_mod_scripting::bindings::WorldGuard;
use bevy_mod_scripting::lua::mlua::{self, Lua, Value};

use crate::render::LyShineCanvasRoot;

use super::convert;
use super::entity_index::LyShineEntityIndex;

/// Plugin: connect the native handler at Startup.
pub struct UiCanvasPlugin;

impl Plugin for UiCanvasPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, connect_native_handler);
    }
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
fn connect_native_handler(registry: Res<EBusRegistry>) {
    // Connected at Broadcast so a single handler receives every Event(addr)
    // dispatch; we filter on `addr` per-call.
    registry.connect_native(
        "UiCanvasBus",
        EBusAddress::Broadcast,
        Arc::new(UiCanvasHandler),
    );
}

struct UiCanvasHandler;

impl NativeBusHandler for UiCanvasHandler {
    fn handles(&self, method: &str) -> bool {
        matches!(method, "GetCanvasName")
    }

    fn dispatch(
        &self,
        world: &WorldGuard<'_>,
        lua: &Lua,
        addr: &EBusAddress,
        method: &str,
        _args: &[Value],
    ) -> mlua::Result<Value> {
        match method {
            "GetCanvasName" => match resolve_canvas_path(world, addr)? {
                Some(asset_path) => Ok(Value::String(lua.create_string(asset_path)?)),
                None => Ok(Value::Nil),
            },
            _ => unreachable!("handles() filtered: {method}"),
        }
    }
}

/// Resolve `addr` into the canvas entity's content asset path.
fn resolve_canvas_path(world: &WorldGuard<'_>, addr: &EBusAddress) -> mlua::Result<Option<String>> {
    let EBusAddress::Integer(id) = addr else {
        return Ok(None);
    };
    let ui_id = convert::entity_id(*id);
    let bevy_entity = world
        .with_resource(|index: &LyShineEntityIndex| index.get(ui_id))
        .map_err(mlua::Error::external)?;
    let Some(entity) = bevy_entity else {
        return Ok(None);
    };
    // Read LyShineCanvasRoot.asset_path off the resolved entity.
    // `with_component` returns `Option<&T>` to handle entities missing
    // the component without erroring.
    let path = world
        .with_component(entity, |root: Option<&LyShineCanvasRoot>| {
            root.map(|r| r.asset_path.to_string())
        })
        .map_err(mlua::Error::external)?;
    Ok(path)
}
