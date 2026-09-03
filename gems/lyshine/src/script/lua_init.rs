//! Install engine-owned `LyShine` bus globals in a Lua context.
//!
//! Use [`add_lyshine_lua_init`] to chain it onto a base
//! [`bevy_mod_scripting::lua::LuaScriptingPlugin`]:
//!
//! ```ignore
//! let plugin = az_gem_lyshine::add_lyshine_bus_init(
//!     az_framework::lua::AzLuaScriptingPlugin::lua_plugin(),
//! );
//! app.add_plugins(BMSPlugin.build().set(plugin));
//! ```

use bevy_mod_scripting::bindings::InteropError;
use bevy_mod_scripting::core::ConfigureScriptPlugin;
use bevy_mod_scripting::lua::{LuaContext, LuaScriptingPlugin, mlua};
use bevy_mod_scripting::script::ScriptAttachment;

/// Engine-owned `LyShine` bus names installed in the Lua global environment.
///
pub const LYSHINE_LUA_BUSES: &[&str] = &[
    "LyShineManagerBus",
    "LyShineScriptBindRequestBus",
    "UiInitializationBus",
    "UiElementBus",
    "UiCanvasBus",
    "UiCanvasNotificationBus",
    "UiTextBus",
];

/// BMS context initializer that pre-installs `LyShine`'s known front-end bus
/// globals. Native bus handlers still register independently at Bevy startup.
///
/// # Errors
///
/// Returns an [`InteropError`] wrapping the first failure from
/// [`az_framework::lua::bus::install_bus`], including Lua errors raised while
/// writing one of [`LYSHINE_LUA_BUSES`] into the globals table.
pub fn lyshine_bus_init(
    _attachment: &ScriptAttachment,
    context: &mut LuaContext,
) -> Result<(), InteropError> {
    let lua: &mlua::Lua = context;
    for name in LYSHINE_LUA_BUSES {
        az_framework::lua::bus::install_bus(lua, name).map_err(InteropError::external)?;
    }
    Ok(())
}

/// Add [`lyshine_bus_init`] to a base [`LuaScriptingPlugin`].
pub fn add_lyshine_bus_init(plugin: LuaScriptingPlugin) -> LuaScriptingPlugin {
    plugin.add_context_initializer(lyshine_bus_init)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_native_lyshine_handler_has_a_lua_global() {
        assert!(LYSHINE_LUA_BUSES.contains(&"LyShineManagerBus"));
        assert!(LYSHINE_LUA_BUSES.contains(&"LyShineScriptBindRequestBus"));
    }
}
