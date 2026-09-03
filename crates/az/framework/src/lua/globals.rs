//! AZ/Lumberyard Lua globals.
//!
//! Stock Lumberyard exposes these via `BehaviorContext::Class<T>("Name")`
//! reflection (constructors that return userdata) and as a `Debug` namespace
//! reflected from `ScriptDebug`.
//!
//! This module registers `Debug.Log` and the opaque `WallClockTimePoint`
//! constructor. Math and entity globals live in [`super::types`].

use bevy_mod_scripting::lua::mlua::{Lua, Result, Table, Value};

/// Register every AZ/Lumberyard Lua global on the given `lua` state. Idempotent.
///
/// Note: real implementations of `Vector2/3`, `Color`, `EntityId`, `Uuid`
/// land via [`super::types::register`] (called immediately after this in
/// [`super::init_az_lua_context`]).
///
/// # Errors
///
/// Returns the [`mlua::Error`] raised while creating or publishing the `Debug`
/// and `WallClockTimePoint` globals.
pub fn register(lua: &Lua) -> Result<()> {
    let globals = lua.globals();

    register_debug(lua, &globals)?;
    register_wall_clock(lua, &globals)?;
    Ok(())
}

/// `Debug.Log(text)` — the native logging primitive exposed to Lua.
/// `Scripts/_Common/Logger.lua` and `Scripts/_Common/Common.lua` both route
/// through this; once it's available `Log(...)`, `LogTable(...)`, and the rest
/// of the helper chain just work.
fn register_debug(lua: &Lua, globals: &Table) -> Result<()> {
    let debug_ns = lua.create_table()?;

    debug_ns.set(
        "Log",
        lua.create_function(|_, value: Value| {
            // Legacy Lua frequently passes pre-formatted strings, but
            // `Debug.Log` in Lumberyard accepts any value via tostring().
            let text = match value {
                Value::String(s) => s.to_str()?.to_string(),
                Value::Nil => "nil".to_string(),
                Value::Boolean(b) => b.to_string(),
                Value::Integer(i) => i.to_string(),
                Value::Number(n) => n.to_string(),
                other => format!("{other:?}"),
            };
            tracing::info!(target: "az_framework::lua::debug", "{}", text);
            Ok(())
        })?,
    )?;

    globals.set("Debug", debug_ns)?;
    Ok(())
}

/// `WallClockTimePoint()` — opaque marker. Imported UI scripts construct it (e.g.
/// `g_lastDropTime = WallClockTimePoint()`), never inspects it. A unit table
/// is enough until something downstream queries it.
fn register_wall_clock(lua: &Lua, globals: &Table) -> Result<()> {
    globals.set(
        "WallClockTimePoint",
        lua.create_function(|lua, (): ()| {
            let t = lua.create_table()?;
            t.set("__az_wallclock", true)?;
            Ok(t)
        })?,
    )?;
    Ok(())
}
