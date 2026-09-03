//! `LyShineScriptBindRequestBus` — query bus reached during front-end
//! Lua boot for editor/cvar/canvas-unload lifecycle.
//!
//! Methods implemented by this adapter:
//!
//! - `Broadcast.IsEditor()`                               → configured runtime state
//! - `Broadcast.GetCVar(name)`                            → integer value or 0
//! - `Broadcast.AddLevelToUnloadCanvases(level: string)`  → record level
//! - `Broadcast.ReleasedCanvases()`                       → bool
//!
//! State lives in two Bevy resources:
//!
//! - [`LyShineCVarTable`] — sparse project-configured cvar map (name → integer).
//! - [`LyShineUnloadLevels`] — set of level names the script has tagged for
//!   canvas flush on transition.
//! - [`LyShineScriptRuntimeState`] — editor and asynchronous canvas-release
//!   state supplied by the composed host.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::prelude::*;
use bevy_mod_scripting::bindings::WorldGuard;
use bevy_mod_scripting::lua::mlua::{self, Lua, Value};

use az_framework::ebus::{EBusAddress, EBusRegistry, NativeBusHandler};

/// Sparse cvar table backing `GetCVar`.
///
/// Projects populate this table during composition; the engine supplies no
/// title-specific defaults.
#[derive(Resource, Debug, Default)]
pub struct LyShineCVarTable(pub HashMap<String, i64>);

/// Set of level names tagged for canvas unload.
#[derive(Resource, Debug, Default)]
pub struct LyShineUnloadLevels(pub HashSet<String>);

/// Host-supplied state returned by the environment and canvas lifecycle queries.
#[derive(Resource, Debug)]
pub struct LyShineScriptRuntimeState {
    /// Whether the composed host is running editor behavior.
    pub is_editor: bool,
    /// Whether asynchronous canvas teardown has completed.
    pub canvases_released: bool,
}

impl Default for LyShineScriptRuntimeState {
    fn default() -> Self {
        Self {
            is_editor: false,
            canvases_released: true,
        }
    }
}

/// Bevy plugin registering both resources and connecting the native
/// handler to [`EBusRegistry`].
pub struct LyShineScriptBindRequestPlugin;

impl Plugin for LyShineScriptBindRequestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LyShineCVarTable>();
        app.init_resource::<LyShineUnloadLevels>();
        app.init_resource::<LyShineScriptRuntimeState>();
        app.add_systems(Startup, connect_native_handler);
    }
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
fn connect_native_handler(registry: Res<EBusRegistry>) {
    registry.connect_native(
        "LyShineScriptBindRequestBus",
        EBusAddress::Broadcast,
        Arc::new(LyShineScriptBindRequestHandler),
    );
}

struct LyShineScriptBindRequestHandler;

impl NativeBusHandler for LyShineScriptBindRequestHandler {
    fn handles(&self, method: &str) -> bool {
        matches!(
            method,
            "IsEditor" | "GetCVar" | "AddLevelToUnloadCanvases" | "ReleasedCanvases"
        )
    }

    fn dispatch(
        &self,
        world: &WorldGuard<'_>,
        _lua: &Lua,
        _addr: &EBusAddress,
        method: &str,
        args: &[Value],
    ) -> mlua::Result<Value> {
        match method {
            "IsEditor" => world
                .with_resource(|state: &LyShineScriptRuntimeState| Value::Boolean(state.is_editor))
                .map_err(mlua::Error::external),

            "GetCVar" => {
                let name = match args.first() {
                    Some(Value::String(s)) => s.to_str()?.to_string(),
                    other => {
                        return Err(mlua::Error::external(format!(
                            "GetCVar(name): expected string, got {other:?}",
                        )));
                    }
                };
                let value = world
                    .with_resource(|table: &LyShineCVarTable| {
                        table.0.get(&name).copied().unwrap_or(0)
                    })
                    .map_err(mlua::Error::external)?;
                Ok(Value::Integer(value))
            }

            "AddLevelToUnloadCanvases" => {
                let level = match args.first() {
                    Some(Value::String(s)) => s.to_str()?.to_string(),
                    other => {
                        return Err(mlua::Error::external(format!(
                            "AddLevelToUnloadCanvases(level): expected string, got {other:?}",
                        )));
                    }
                };
                world
                    .with_resource_mut(
                        |mut levels: bevy::ecs::change_detection::Mut<LyShineUnloadLevels>| {
                            levels.0.insert(level);
                        },
                    )
                    .map_err(mlua::Error::external)?;
                Ok(Value::Nil)
            }

            "ReleasedCanvases" => world
                .with_resource(|state: &LyShineScriptRuntimeState| {
                    Value::Boolean(state.canvases_released)
                })
                .map_err(mlua::Error::external),

            _ => unreachable!("handles() filtered: {method}"),
        }
    }
}
