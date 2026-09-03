//! Lua-side `EBus` shape factory.
//!
//! [`install_bus`] creates a global Lua table mirroring Lumberyard's
//! `BehaviorContext::EBus<T>` reflection: `Connect`, `Disconnect`,
//! `Broadcast`, `Event`. Every dispatch funnels into the
//! [`crate::ebus::EBusRegistry`] resource via BMS's
//! [`bevy_mod_scripting::bindings::ThreadWorldContainer`].
//!
//! Per-bus method names are open-ended — Lua scripts can call any method on
//! `Bus.Broadcast.<M>`. We satisfy that with an `__index` metatable on
//! `Broadcast` / `Event` that synthesises a per-method dispatcher closure
//! on first access. The closure captures the bus name + method name +
//! "`is_addressed`" flag, then dispatches through the registry.
//!
//! On `Connect`, we register a Lua handler in the registry and return a
//! Lua handle table whose `:Disconnect()` removes the registration.

use bevy_mod_scripting::bindings::ThreadWorldContainer;
use bevy_mod_scripting::lua::mlua::{self, Function, Lua, Result, Table, Value, Variadic};

use crate::ebus::{EBusAddress, EBusRegistry};

/// Create the bus table for `name` and install it as a global on `lua`.
/// Returns the table for further configuration. Idempotent.
///
/// Uses `raw_get` / `raw_set` because `_G` may carry a metatable that
/// auto-registers buses on missing-key access (see
/// `install_bus_auto_register` in `super::mod`); going through the
/// metatable would re-enter this function infinitely.
///
/// # Errors
///
/// Returns the [`mlua::Error`] raised while building the bus shape — table
/// and closure creation, the `raw_set` calls that populate `Connect`,
/// `Disconnect`, `Broadcast` and `Event`, and the `raw_set` that installs the
/// bus as a global.
pub fn install_bus(lua: &Lua, name: &str) -> Result<Table> {
    let globals = lua.globals();
    if let Ok(Value::Table(existing)) = globals.raw_get::<Value>(name) {
        return Ok(existing);
    }

    let bus = lua.create_table()?;
    let bus_name = name.to_string();

    bus.raw_set("__bus_name", bus_name.clone())?;
    bus.raw_set("Connect", make_connect(lua, bus_name.clone())?)?;
    bus.raw_set("Disconnect", make_noop(lua)?)?;
    bus.raw_set(
        "Broadcast",
        make_dispatch_table(lua, bus_name.clone(), false)?,
    )?;
    bus.raw_set("Event", make_dispatch_table(lua, bus_name, true)?)?;

    globals.raw_set(name, bus.clone())?;
    Ok(bus)
}

/// `Bus.Connect(self [, addr])` — register a Lua listener and hand back a
/// disconnect-handle table whose `:Disconnect()` removes the subscription.
///
/// Legacy Lumberyard Lua mixes two calling conventions across buses:
///
/// - Regular `EBuses` (`UiInitializationBus`, `LyShineScriptBindNotificationBus`,
///   ...): `Bus.Connect(self [, addr])` — listener first, optional address
///   second (mirrors `BaseElement:BusConnect` in
///   `_common/baseelement.lua:69-78`).
/// - `DynamicBus[name]` and `UITickBus`: `Bus.Connect(addr, self)` — address
///   first, listener second.
///
/// We detect the convention by type: the listener MUST be a table (Lua
/// handler with method members), the address is anything else
/// (Integer, String, EntityId-as-int). Whichever arg is the table wins
/// the listener slot.
fn make_connect(lua: &Lua, bus: String) -> Result<Function> {
    lua.create_function(
        move |lua, (a, b): (Value, Option<Value>)| -> Result<Table> {
            let (listener_table, address_value) = match (a, b) {
                // `Bus.Connect(table, ?)` — listener-first convention.
                (Value::Table(t), addr) => (t, addr.unwrap_or(Value::Nil)),
                // `Bus.Connect(addr, table)` — address-first convention.
                (addr, Some(Value::Table(t))) => (t, addr),
                // Neither arg is a table → bail with a clear error.
                (a, b) => {
                    return Err(mlua::Error::external(format!(
                        "{bus}.Connect(...): no listener table found; got ({a:?}, {b:?})",
                    )));
                }
            };
            let address = match address_value {
                Value::Nil => EBusAddress::Broadcast,
                v => EBusAddress::from_lua(&v),
            };

            let id = with_registry(|registry| {
                Ok(registry.connect_lua(bus.clone(), address, listener_table.clone()))
            })?;

            // Capture `id` in a per-handle Disconnect closure rather than
            // stashing it in a Lua field — keeps the SubscriptionId type
            // on the Rust side and avoids round-tripping through Lua ints.
            let disconnect = lua.create_function(move |_, _: Variadic<Value>| -> Result<()> {
                with_registry(|registry| {
                    registry.disconnect(id);
                    Ok(())
                })
            })?;

            let handle = lua.create_table()?;
            handle.raw_set("Disconnect", disconnect)?;
            Ok(handle)
        },
    )
}

fn make_noop(lua: &Lua) -> Result<Function> {
    lua.create_function(|_, _args: Variadic<Value>| Ok(Value::Nil))
}

/// Build the `Broadcast` (or `Event`) sub-table. Looking up any method
/// name returns a dispatcher closure for that method.
///
/// The closure: snapshot subscriptions out of the registry under a brief
/// read lock, drop the lock, then iterate the snapshot calling each
/// handler with the world guard. Snapshot-then-dispatch ensures handlers
/// can re-enter the registry (a Lua handler invoking another bus call)
/// without deadlocking, and lets native handlers freely use
/// [`bevy_mod_scripting::bindings::WorldGuard::with_resource`] /
/// `with_resource_mut` because the registry's resource lock is no longer
/// held.
fn make_dispatch_table(lua: &Lua, bus: String, by_address: bool) -> Result<Table> {
    let dispatcher = lua.create_table()?;
    let mt = lua.create_table()?;

    let bus_for_index = bus;
    let make_method = lua.create_function(
        move |lua, (_this, method): (Table, String)| -> Result<Function> {
            let bus = bus_for_index.clone();
            lua.create_function(move |lua, args: Variadic<Value>| -> Result<Value> {
                let mut iter = args.into_iter();
                let address = if by_address {
                    EBusAddress::from_lua(&iter.next().unwrap_or(Value::Nil))
                } else {
                    EBusAddress::Broadcast
                };
                let rest: Vec<Value> = iter.collect();

                let world = ThreadWorldContainer
                    .try_get_context()
                    .map_err(mlua::Error::external)?
                    .world;

                let snapshots = world
                    .with_resource(|registry: &EBusRegistry| {
                        registry.snapshot_for_dispatch(
                            bus.as_str(),
                            if by_address { Some(&address) } else { None },
                        )
                    })
                    .map_err(mlua::Error::external)?;

                let mut last = Value::Nil;
                for snap in snapshots {
                    if let Some(ret) = snap.invoke(&world, lua, &address, method.as_str(), &rest)?
                        && !matches!(ret, Value::Nil)
                    {
                        last = ret;
                    }
                }
                Ok(last)
            })
        },
    )?;

    mt.raw_set("__index", make_method)?;
    dispatcher.set_metatable(Some(mt));
    Ok(dispatcher)
}

/// Helper: run a closure with a borrow of the [`EBusRegistry`] resource.
/// Pulls the world via [`ThreadWorldContainer`].
fn with_registry<F, R>(f: F) -> Result<R>
where
    F: FnOnce(&EBusRegistry) -> Result<R>,
{
    let world = ThreadWorldContainer
        .try_get_context()
        .map_err(mlua::Error::external)?
        .world;
    world
        .with_resource(|registry: &EBusRegistry| f(registry))
        .map_err(mlua::Error::external)?
}
