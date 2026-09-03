//! `UiElementBus` — per-element queries.
//!
//! O3DE reference: `Gems/LyShine/Code/Include/LyShine/Bus/UiElementBus.h`,
//! and its `BehaviorContext::EBus<UiElementBus>` reflection.
//!
//! Method coverage (front-end Lua boot subset):
//!
//! - `Event.GetCanvas(entityId)` → canvas's `UiEntityId` (or nil)
//! - `Event.GetName(entityId)` → entity display name (string)
//! - `Event.GetParent(entityId)` → parent's `UiEntityId` (or nil)
//! - `Event.SetIsEnabled(entityId, bool)` → toggle UI node `Visibility`
//!
//! Address resolution: the `entityId` arg comes through as
//! [`EBusAddress::Integer`] (the [`crate::canvas::UiEntityId`] u64).
//! [`super::entity_index::LyShineEntityIndex`] maps that to a Bevy
//! `Entity`; we then read the requested data off the entity's components.

use std::sync::Arc;

use az_framework::ebus::{EBusAddress, EBusRegistry, NativeBusHandler};
use bevy::prelude::*;
use bevy_mod_scripting::bindings::WorldGuard;
use bevy_mod_scripting::lua::mlua::{self, Lua, Value};

use crate::LyShineUiEntity;
use crate::canvas::UiEntityId;
use crate::render::{LyShineCanvasRoot, LyShineUiEntityDebugInfo};

use super::convert;
use super::entity_index::LyShineEntityIndex;

pub struct UiElementPlugin;

impl Plugin for UiElementPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, connect_native_handler);
    }
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
fn connect_native_handler(registry: Res<EBusRegistry>) {
    registry.connect_native(
        "UiElementBus",
        EBusAddress::Broadcast,
        Arc::new(UiElementHandler),
    );
}

struct UiElementHandler;

impl NativeBusHandler for UiElementHandler {
    fn handles(&self, method: &str) -> bool {
        matches!(
            method,
            "GetCanvas" | "GetName" | "GetParent" | "SetIsEnabled"
        )
    }

    fn dispatch(
        &self,
        world: &WorldGuard<'_>,
        lua: &Lua,
        addr: &EBusAddress,
        method: &str,
        args: &[Value],
    ) -> mlua::Result<Value> {
        let Some(target) = resolve(world, addr)? else {
            return Ok(Value::Nil);
        };

        match method {
            "GetCanvas" => Ok(find_canvas_root(world, target)?
                .map_or(Value::Nil, |canvas_ui_id| {
                    Value::Integer(convert::lua_id(canvas_ui_id))
                })),
            "GetName" => {
                let name = world
                    .with_component(target, |dbg: Option<&LyShineUiEntityDebugInfo>| {
                        dbg.and_then(|d| d.name.as_deref().map(str::to_owned))
                    })
                    .map_err(mlua::Error::external)?
                    .unwrap_or_default();
                Ok(Value::String(lua.create_string(&name)?))
            }
            "GetParent" => Ok(
                find_ui_parent(world, target)?.map_or(Value::Nil, |parent_ui_id| {
                    Value::Integer(convert::lua_id(parent_ui_id))
                }),
            ),
            "SetIsEnabled" => {
                // TODO: bridge to Bevy `Visibility` component once we have a
                // canonical UiEntity → Visibility mapping (LyShine canvas
                // entities sometimes have `Visibility::Inherited` and
                // sometimes `Visible`; flipping at script-level requires
                // knowing the original semantics).
                let enabled = match args.first() {
                    Some(Value::Boolean(b)) => *b,
                    _ => return Ok(Value::Nil),
                };
                tracing::trace!(
                    target: "az_gem_lyshine::script::ui_element",
                    entity = ?target,
                    enabled,
                    "SetIsEnabled (no-op pending Visibility bridge)",
                );
                Ok(Value::Nil)
            }
            _ => unreachable!("handles() filtered: {method}"),
        }
    }
}

/// Resolve an `EBusAddress` to a Bevy entity via the `LyShine` index.
fn resolve(world: &WorldGuard<'_>, addr: &EBusAddress) -> mlua::Result<Option<Entity>> {
    let EBusAddress::Integer(id) = addr else {
        return Ok(None);
    };
    let ui_id = convert::entity_id(*id);
    world
        .with_resource(|index: &LyShineEntityIndex| index.get(ui_id))
        .map_err(mlua::Error::external)
}

/// Walk the Bevy parent chain looking for the [`LyShineCanvasRoot`].
/// Returns the canvas root's `UiEntityId` if found.
fn find_canvas_root(world: &WorldGuard<'_>, start: Entity) -> mlua::Result<Option<UiEntityId>> {
    let mut current = start;
    loop {
        // If the current entity itself is a canvas root, take it.
        let is_canvas = world
            .with_component(current, |root: Option<&LyShineCanvasRoot>| root.is_some())
            .map_err(mlua::Error::external)?;
        if is_canvas {
            // The canvas root entity has its own LyShineUiEntity (root id).
            let id = world
                .with_component(current, |ui: Option<&LyShineUiEntity>| {
                    ui.map(|u| u.entity_id)
                })
                .map_err(mlua::Error::external)?;
            return Ok(id);
        }
        // Step to parent.
        let Some(parent) = world
            .with_component(current, |child_of: Option<&ChildOf>| {
                child_of.map(ChildOf::parent)
            })
            .map_err(mlua::Error::external)?
        else {
            return Ok(None);
        };
        current = parent;
    }
}

/// Read the immediate Bevy parent and look up its [`UiEntityId`].
fn find_ui_parent(world: &WorldGuard<'_>, entity: Entity) -> mlua::Result<Option<UiEntityId>> {
    let Some(parent) = world
        .with_component(entity, |child_of: Option<&ChildOf>| {
            child_of.map(ChildOf::parent)
        })
        .map_err(mlua::Error::external)?
    else {
        return Ok(None);
    };
    world
        .with_component(parent, |ui: Option<&LyShineUiEntity>| {
            ui.map(|u| u.entity_id)
        })
        .map_err(mlua::Error::external)
}
