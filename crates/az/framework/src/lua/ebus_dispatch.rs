//! Native-to-Lua EBus dispatch.

use std::sync::Arc;

use bevy::ecs::message::{MessageCursor, Messages};
use bevy::ecs::system::{Local, SystemState};
use bevy::ecs::world::World;
use bevy_mod_scripting::bindings::{
    ThreadScriptContext, ThreadWorldContainer, WorldAccessGuard, WorldGuard,
};
use bevy_mod_scripting::core::script::ScriptContext;
use bevy_mod_scripting::lua::mlua::{self, Lua, Value};
use bevy_mod_scripting::lua::{LuaContext, LuaScriptingPlugin};
use bevy_mod_scripting::script::ScriptAttachment;
use parking_lot::Mutex;

use crate::ebus::{EBusAddress, EBusArg, EBusCall, EBusRegistry, HandlerSnapshot};
use crate::lua::types::{
    entity_id::EntityId, quaternion::AzQuaternion, vector2::AzVector2, vector3::AzVector3,
    vector4::AzVector4,
};

#[allow(deprecated)]
pub(crate) fn dispatch_ebus_calls(
    world: &mut World,
    state: &mut SystemState<Local<MessageCursor<EBusCall>>>,
) {
    let mut cursor = match state.get_mut(world) {
        Ok(cursor) => cursor,
        Err(err) => {
            tracing::debug!(
                target: "az_framework::lua::ebus",
                error = %err,
                "skipping EBus calls because message cursor validation failed"
            );
            return;
        }
    };
    let guard = WorldAccessGuard::new_exclusive(world);

    let calls = match guard.with_resource(|events: &Messages<EBusCall>| {
        cursor.read(events).cloned().collect::<Vec<_>>()
    }) {
        Ok(calls) => calls,
        Err(_) => return,
    };
    if calls.is_empty() {
        return;
    }

    let contexts = match guard
        .with_resource(|contexts: &ScriptContext<LuaScriptingPlugin>| collect_contexts(contexts))
    {
        Ok(contexts) => contexts,
        Err(err) => {
            tracing::debug!(target: "az_framework::lua::ebus", error = %err, "skipping EBus calls without Lua contexts");
            return;
        }
    };
    if contexts.is_empty() {
        tracing::debug!(target: "az_framework::lua::ebus", count = calls.len(), "skipping EBus calls without loaded Lua contexts");
        return;
    }

    WorldAccessGuard::with_existing_static_guard(guard, |world| {
        for call in calls {
            if let Err(err) = dispatch_call(&world, &contexts, &call) {
                tracing::warn!(
                    target: "az_framework::lua::ebus",
                    bus = call.bus(),
                    method = call.method(),
                    error = %err,
                    "EBus dispatch failed",
                );
            }
        }
    });
}

struct LuaBusContext {
    residents: Vec<ScriptAttachment>,
    context: Arc<Mutex<LuaContext>>,
}

impl LuaBusContext {
    fn owns(&self, attachment: &ScriptAttachment) -> bool {
        self.residents.iter().any(|resident| resident == attachment)
    }

    fn representative(&self) -> &ScriptAttachment {
        &self.residents[0]
    }
}

fn collect_contexts(contexts: &ScriptContext<LuaScriptingPlugin>) -> Vec<LuaBusContext> {
    let mut out = Vec::<LuaBusContext>::new();
    let contexts = contexts.read();
    for (attachment, context) in contexts.all_residents() {
        if let Some(existing) = out
            .iter_mut()
            .find(|existing| Arc::ptr_eq(&existing.context, &context))
        {
            existing.residents.push(attachment);
        } else {
            out.push(LuaBusContext {
                residents: vec![attachment],
                context,
            });
        }
    }
    out
}

fn dispatch_call(
    world: &WorldGuard<'static>,
    contexts: &[LuaBusContext],
    call: &EBusCall,
) -> mlua::Result<()> {
    let snapshots = world
        .with_resource(|registry: &EBusRegistry| {
            registry.snapshot_for_dispatch(call.bus(), call.address())
        })
        .map_err(mlua::Error::external)?;

    if snapshots.is_empty() {
        return Ok(());
    }

    let address = call.address().cloned().unwrap_or(EBusAddress::Broadcast);
    let mut native_invoked = false;

    for context in contexts {
        let lua_context = context.context.lock();
        ThreadWorldContainer
            .set_context(ThreadScriptContext {
                world: world.clone(),
                attachment: context.representative().clone(),
            })
            .map_err(mlua::Error::external)?;

        let lua: &Lua = &lua_context;
        let args = call
            .call_args()
            .iter()
            .map(|arg| arg_to_lua(lua, arg))
            .collect::<mlua::Result<Vec<_>>>()?;

        if !native_invoked {
            for snapshot in snapshots.iter().filter(|snapshot| snapshot.is_native()) {
                invoke(snapshot, world, lua, &address, call.method(), &args)?;
            }
            native_invoked = true;
        }

        for snapshot in snapshots.iter().filter(|snapshot| {
            snapshot
                .lua_attachment()
                .is_some_and(|attachment| context.owns(attachment))
        }) {
            invoke(snapshot, world, lua, &address, call.method(), &args)?;
        }
    }

    Ok(())
}

fn invoke(
    snapshot: &HandlerSnapshot,
    world: &bevy_mod_scripting::bindings::WorldGuard<'_>,
    lua: &Lua,
    address: &EBusAddress,
    method: &str,
    args: &[Value],
) -> mlua::Result<()> {
    snapshot
        .invoke(world, lua, address, method, args)
        .map(|_| ())
}

fn arg_to_lua(lua: &Lua, arg: &EBusArg) -> mlua::Result<Value> {
    Ok(match arg {
        EBusArg::Nil => Value::Nil,
        EBusArg::Bool(value) => Value::Boolean(*value),
        EBusArg::Integer(value) => Value::Integer(*value),
        EBusArg::Number(value) => Value::Number(*value),
        EBusArg::String(value) => Value::String(lua.create_string(value)?),
        EBusArg::EntityId(value) => Value::UserData(lua.create_userdata(EntityId(*value))?),
        EBusArg::Address(value) => address_to_lua(lua, value)?,
        EBusArg::Vec2(value) => Value::UserData(lua.create_userdata(AzVector2(*value))?),
        EBusArg::Vec3(value) => Value::UserData(lua.create_userdata(AzVector3(*value))?),
        EBusArg::Vec4(value) => Value::UserData(lua.create_userdata(AzVector4(*value))?),
        EBusArg::Quat(value) => Value::UserData(lua.create_userdata(AzQuaternion(*value))?),
    })
}

fn address_to_lua(lua: &Lua, address: &EBusAddress) -> mlua::Result<Value> {
    Ok(match address {
        EBusAddress::Broadcast => Value::Nil,
        EBusAddress::Entity(entity) => Value::Integer(entity.to_bits() as i64),
        EBusAddress::Name(value) => Value::String(lua.create_string(value)?),
        EBusAddress::Integer(value) => Value::Integer(*value),
    })
}
