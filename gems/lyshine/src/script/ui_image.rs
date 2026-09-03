//! `UiImageBus` — addressed UI-image queries.
//!
//! O3DE reference: `Gems/LyShine/Code/Include/LyShine/Bus/UiImageBus.h`.
//! The implemented surface covers sprite path swaps, color, alpha,
//! image type, fill modes, UV overrides.
//!
//! Method coverage (rendering-critical subset):
//!
//! - `Get/SetSpritePathname(addr [, "path"])` — swap the sprite asset.
//!   Currently records the requested path on a `LyShineImageBinding`;
//!   the existing `bind_loaded_canvas_images` system in
//!   `gems/lyshine/src/render.rs` translates that into a
//!   loaded `Handle<Image>` on the entity's `ImageNode`.
//! - `Get/SetColor(addr [, AzColor])` — mutate `ImageNode.color`.
//! - `SetAlpha(addr, alpha)` — preserve color, set alpha channel.
//!
//! Methods accepted but currently no-op (per-script-call upgrade as
//! observed): `GetFillAmount`, `SetFillAmount`, `SetFillType`,
//! `SetFillClockwise`, `SetEdgeFillOrigin`, `SetRadialFillStartAngle`,
//! `Get/SetImageType`, `SetUVOverrides`, `SetRenderTargetName`,
//! `UnloadTexture`, `SetSpritePathnameIfExists`, `SetSpritePathnamePool`.

use std::sync::Arc;

use az_framework::ebus::{EBusAddress, EBusRegistry, NativeBusHandler};
use az_framework::lua::types::color::AzColor;
use bevy::prelude::*;
use bevy_mod_scripting::bindings::WorldGuard;
use bevy_mod_scripting::lua::mlua::{self, Lua, Value};

use crate::render::LyShineImageBinding;

use super::convert;
use super::entity_index::LyShineEntityIndex;

pub struct UiImagePlugin;

impl Plugin for UiImagePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, connect_native_handler);
    }
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
fn connect_native_handler(registry: Res<EBusRegistry>) {
    registry.connect_native(
        "UiImageBus",
        EBusAddress::Broadcast,
        Arc::new(UiImageHandler),
    );
}

struct UiImageHandler;

impl NativeBusHandler for UiImageHandler {
    fn handles(&self, method: &str) -> bool {
        matches!(
            method,
            "GetSpritePathname"
                | "SetSpritePathname"
                | "SetSpritePathnameIfExists"
                | "SetSpritePathnamePool"
                | "GetColor"
                | "SetColor"
                | "SetAlpha"
                // Accepted but no-op below.
                | "GetFillAmount"
                | "SetFillAmount"
                | "SetFillType"
                | "SetFillClockwise"
                | "SetEdgeFillOrigin"
                | "SetRadialFillStartAngle"
                | "GetImageType"
                | "SetImageType"
                | "SetUVOverrides"
                | "SetRenderTargetName"
                | "UnloadTexture"
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
            "GetSpritePathname" => {
                let path = world
                    .with_component(target, |opt: Option<&LyShineImageBinding>| {
                        opt.map(|b| b.sprite_path.to_string()).unwrap_or_default()
                    })
                    .map_err(mlua::Error::external)?;
                Ok(Value::String(lua.create_string(&path)?))
            }
            "SetSpritePathname" | "SetSpritePathnameIfExists" => {
                let new_path = match args.first() {
                    Some(Value::String(s)) => s.to_str()?.to_string(),
                    _ => return Ok(Value::Nil),
                };
                world
                    .with_component_mut(
                        target,
                        |opt: Option<bevy::ecs::change_detection::Mut<LyShineImageBinding>>| {
                            if let Some(mut binding) = opt {
                                binding.sprite_path = new_path.clone().into_boxed_str();
                            }
                        },
                    )
                    .map_err(mlua::Error::external)?;
                tracing::debug!(
                    target: "az_gem_lyshine::script::ui_image",
                    bevy_entity = ?target,
                    new_path = %new_path,
                    "SetSpritePathname",
                );
                Ok(Value::Nil)
            }
            "SetColor" => {
                let color = match args.first() {
                    Some(Value::UserData(ud)) => match ud.borrow::<AzColor>() {
                        Ok(c) => *c,
                        Err(_) => return Ok(Value::Nil),
                    },
                    _ => return Ok(Value::Nil),
                };
                // Bevy 0.18 UI image: the canvas spawn flow attaches an
                // `ImageNode` (or equivalent) — we mutate its `.color`
                // tint. Falls through silently if the entity has no
                // image node attached.
                world
                    .with_component_mut(
                        target,
                        |opt: Option<bevy::ecs::change_detection::Mut<ImageNode>>| {
                            if let Some(mut img) = opt {
                                img.color = bevy::color::LinearRgba::new(
                                    color.0.x, color.0.y, color.0.z, color.0.w,
                                )
                                .into();
                            }
                        },
                    )
                    .map_err(mlua::Error::external)?;
                Ok(Value::Nil)
            }
            "GetColor" => {
                let color = world
                    .with_component(target, |opt: Option<&ImageNode>| {
                        opt.map(|img| {
                            let lin = img.color.to_linear();
                            AzColor(bevy::math::Vec4::new(
                                lin.red, lin.green, lin.blue, lin.alpha,
                            ))
                        })
                    })
                    .map_err(mlua::Error::external)?;
                match color {
                    Some(c) => Ok(Value::UserData(lua.create_userdata(c)?)),
                    None => Ok(Value::Nil),
                }
            }
            "SetAlpha" => {
                let Some(alpha) = convert::scalar(args, 0) else {
                    return Ok(Value::Nil);
                };
                world
                    .with_component_mut(
                        target,
                        |opt: Option<bevy::ecs::change_detection::Mut<ImageNode>>| {
                            if let Some(mut img) = opt {
                                let lin = img.color.to_linear();
                                img.color = bevy::color::LinearRgba::new(
                                    lin.red, lin.green, lin.blue, alpha,
                                )
                                .into();
                            }
                        },
                    )
                    .map_err(mlua::Error::external)?;
                Ok(Value::Nil)
            }
            // No-op fall-throughs.
            _ => Ok(Value::Nil),
        }
    }
}

fn resolve(world: &WorldGuard<'_>, addr: &EBusAddress) -> mlua::Result<Option<Entity>> {
    let EBusAddress::Integer(id) = addr else {
        return Ok(None);
    };
    let ui_id = convert::entity_id(*id);
    world
        .with_resource(|index: &LyShineEntityIndex| index.get(ui_id))
        .map_err(mlua::Error::external)
}
