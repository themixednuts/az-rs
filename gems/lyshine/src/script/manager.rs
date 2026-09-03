//! `LyShineManagerBus` — central UI manager bus reached during
//! front-end Lua boot.
//!
//! Lumberyard reference:
//! O3DE reference: `Gems/LyShine/Code/Source/LyShineSystemComponent.cpp`.
//!
//! Methods implemented by this adapter:
//!
//! - `Broadcast.PushMemoryContext(name: string)`     → push allocation
//!   tracking context. We mirror the stack on a Bevy resource for
//!   future budget-attribution but treat the call as advisory (no
//!   real allocator hook yet).
//! - `Broadcast.PopMemoryContext()`                  → pop the stack.
//! - `Broadcast.DeregisterAllScreens()`              → clear the
//!   per-canvas screen registry on level transitions. Currently a
//!   no-op until the screen registry lands.
//! - `Broadcast.CreateSpriteCachePool(size: integer)` → returns a
//!   stable pool id (u32) for sprite caching.
//!
//! Pool ids are dispensed from a counter resource so they survive
//! script reloads and can be looked up later.

use std::sync::Arc;

use az_framework::ebus::{EBusAddress, EBusRegistry, NativeBusHandler};
use bevy::prelude::*;
use bevy_mod_scripting::bindings::WorldGuard;
use bevy_mod_scripting::lua::mlua::{self, Lua, Value};

use super::convert;

/// Mirror of the Lumberyard manager's allocator-tagging stack.
///
/// Pushed and popped by Lua through `PushMemoryContext` /
/// `PopMemoryContext`; read by debug tools that want to attribute
/// allocations to a script scope.
#[derive(Resource, Default, Debug)]
pub struct LyShineMemoryContextStack(pub Vec<String>);

/// Allocator for sprite cache pool ids. Each pool reserves up to `size`
/// bytes of GPU and CPU sprite cache; scripts pass the id to later
/// sprite-load calls so caching can be scoped per pool.
#[derive(Resource, Default, Debug)]
pub struct LyShineSpriteCachePools {
    next_id: u32,
    /// `(id, size_bytes)` for diagnostic visibility.
    pools: Vec<(u32, u64)>,
}

impl LyShineSpriteCachePools {
    pub fn allocate(&mut self, size_bytes: u64) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pools.push((id, size_bytes));
        id
    }

    #[must_use]
    pub fn pools(&self) -> &[(u32, u64)] {
        &self.pools
    }
}

/// Plugin: install resources and connect the native handler.
pub struct LyShineManagerPlugin;

impl Plugin for LyShineManagerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LyShineMemoryContextStack>();
        app.init_resource::<LyShineSpriteCachePools>();
        app.add_systems(Startup, connect_native_handler);
    }
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
fn connect_native_handler(registry: Res<EBusRegistry>) {
    registry.connect_native(
        "LyShineManagerBus",
        EBusAddress::Broadcast,
        Arc::new(LyShineManagerHandler),
    );
}

struct LyShineManagerHandler;

impl NativeBusHandler for LyShineManagerHandler {
    fn handles(&self, method: &str) -> bool {
        matches!(
            method,
            "PushMemoryContext"
                | "PopMemoryContext"
                | "DeregisterAllScreens"
                | "CreateSpriteCachePool"
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
            "PushMemoryContext" => {
                let name = match args.first() {
                    Some(Value::String(s)) => s.to_str()?.to_string(),
                    _ => String::new(),
                };
                world
                    .with_resource_mut(
                        |mut stack: bevy::ecs::change_detection::Mut<LyShineMemoryContextStack>| {
                            stack.0.push(name);
                        },
                    )
                    .map_err(mlua::Error::external)?;
                Ok(Value::Nil)
            }
            "PopMemoryContext" => {
                world
                    .with_resource_mut(
                        |mut stack: bevy::ecs::change_detection::Mut<LyShineMemoryContextStack>| {
                            stack.0.pop();
                        },
                    )
                    .map_err(mlua::Error::external)?;
                Ok(Value::Nil)
            }
            "DeregisterAllScreens" => {
                // TODO: clear LyShine screen registry once it exists.
                tracing::trace!(
                    target: "az_gem_lyshine::script::manager",
                    "DeregisterAllScreens (no-op pending screen registry)",
                );
                Ok(Value::Nil)
            }
            "CreateSpriteCachePool" => {
                let Some(size_bytes) = convert::count(args, 0) else {
                    let other = args.first();
                    return Err(mlua::Error::external(format!(
                        "CreateSpriteCachePool(size): expected integer, got {other:?}",
                    )));
                };
                let id = world
                    .with_resource_mut(
                        |mut pools: bevy::ecs::change_detection::Mut<LyShineSpriteCachePools>| {
                            pools.allocate(size_bytes)
                        },
                    )
                    .map_err(mlua::Error::external)?;
                Ok(Value::Integer(i64::from(id)))
            }
            _ => unreachable!("handles() filtered: {method}"),
        }
    }
}
