//! `DynamicBus` — name-keyed indexer.
//!
//! Lumberyard reference: `BehaviorContext` exposes `DynamicBus` as
//! a Lua-only indirection table. `DynamicBus.<Name>` (or
//! `DynamicBus[name]`) returns a freshly synthesised bus shape whose
//! `Connect` / `Broadcast` / `Event` route through a per-name backing
//! bus.
//!
//! Example:
//!
//! ```lua
//! DynamicBus.Globals.Connect(self.entityId, self)
//! DynamicBus.UITickBus.Connect(self.entityId, self)
//! DynamicBus.UIStateBus.Broadcast.onStateChanged(true)
//! DynamicBus[self.name].Connect(self.entityId, self)
//! ```
//!
//! We implement this by giving `DynamicBus` an `__index` metatable that
//! synthesises a bus-shaped sub-table on first access. Each sub-table
//! delegates to the same [`crate::ebus::EBusRegistry`] used by static
//! buses, but with a `dyn:<Name>` bus prefix so subscriptions don't
//! collide with statically-registered buses sharing the same name.
//!
//! Sub-tables are cached in a hidden `__cache` field so repeated
//! `DynamicBus.Globals` lookups return the same table — important for
//! Lua identity comparisons (`if otherBus == self.bus` etc.).

use bevy_mod_scripting::lua::mlua::{Lua, Result, Table, Value};

use super::bus::install_bus;

/// Prefix added to bus names registered through `DynamicBus[name]` to
/// keep them in their own namespace.
pub const DYNAMIC_BUS_PREFIX: &str = "dyn:";

/// Install the global `DynamicBus` indexer.
///
/// # Errors
///
/// Returns the `mlua::Error` raised while creating the `DynamicBus` table, its
/// `__cache` table, the `__index` closure, or while setting `DynamicBus` as a
/// global.
pub fn install(lua: &Lua) -> Result<()> {
    let dyn_bus = lua.create_table()?;
    let cache = lua.create_table()?;
    dyn_bus.raw_set("__cache", cache.clone())?;

    let mt = lua.create_table()?;
    let cache_for_index = cache;
    mt.raw_set(
        "__index",
        lua.create_function(
            move |lua, (_this, name): (Table, String)| -> Result<Value> {
                if let Ok(Value::Table(existing)) = cache_for_index.get::<Value>(name.clone()) {
                    return Ok(Value::Table(existing));
                }
                let bus_name = format!("{DYNAMIC_BUS_PREFIX}{name}");
                let bus = install_bus(lua, &bus_name)?;
                cache_for_index.set(name, bus.clone())?;
                Ok(Value::Table(bus))
            },
        )?,
    )?;

    dyn_bus.set_metatable(Some(mt));
    lua.globals().set("DynamicBus", dyn_bus)?;
    Ok(())
}
