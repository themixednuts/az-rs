//! `UiTextBus` — addressed dispatch for text-element mutation.
//!
//! O3DE reference: `Gems/LyShine/Code/Include/LyShine/Bus/UiTextBus.h`,
//! `BehaviorContext::EBus<UiTextBus>` reflection. The implemented surface
//! includes getters and setters for text content, font, color,
//! sizing, alignment, wrap mode, etc.
//!
//! Method coverage in this implementation (rendering critical path):
//!
//! - `Event.SetText(addr, text: string)` → mutates `Text`
//! - `Event.GetText(addr) -> string`
//! - `Event.SetColor(addr, AzColor)` → mutates `TextColor`
//! - `Event.GetColor(addr) -> AzColor`
//! - `Event.SetFontSize(addr, f32)` → mutates `TextFont`
//! - `Event.GetFontSize(addr) -> f32`
//!
//! Methods we deliberately stub (return Nil) until they hit:
//! `Set/GetCharacterSpacing`, `Set/GetLineSpacing`,
//! `Set/GetTextCase`, `SetWrapText`, `SetHorizontalTextAlignment`,
//! `SetVerticalTextAlignment`, `SetOverflowMode`, `SetShrinkToFit`,
//! `SetIsMarkupEnabled`, `SetFont`, `SetFontEffect*`,
//! `Get/SetTextWidth/Height/Size`, `GetWrappedTextFromCache`. Each is a
//! one-liner upgrade once a script's behaviour depends on it.
//!
//! ## Address handling
//!
//! `UiTextBus` is a per-entity bus addressed by `UiEntityId`. The
//! Lua dispatcher passes the `addr` arg through to our `dispatch` as
//! `EBusAddress`. We resolve to a Bevy entity via
//! [`super::entity_index::LyShineEntityIndex`] then mutate its
//! `bevy_ui::Text` / `TextFont` / `TextColor` components.

use std::sync::Arc;

use az_framework::ebus::{EBusAddress, EBusRegistry, NativeBusHandler};
use az_framework::lua::types::color::AzColor;
use bevy::prelude::*;
use bevy::text::{FontSize, TextColor, TextFont};
use bevy::ui::widget::Text;
use bevy_mod_scripting::bindings::WorldGuard;
use bevy_mod_scripting::lua::mlua::{self, Lua, Value};

use super::convert;
use super::entity_index::LyShineEntityIndex;

/// Plugin: connect the native handler at Startup.
pub struct UiTextPlugin;

impl Plugin for UiTextPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, connect_native_handler);
    }
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
fn connect_native_handler(registry: Res<EBusRegistry>) {
    registry.connect_native("UiTextBus", EBusAddress::Broadcast, Arc::new(UiTextHandler));
}

struct UiTextHandler;

impl NativeBusHandler for UiTextHandler {
    fn handles(&self, method: &str) -> bool {
        matches!(
            method,
            "SetText"
                | "GetText"
                | "SetColor"
                | "GetColor"
                | "SetFontSize"
                | "GetFontSize"
                // Methods we accept but currently no-op so the script
                // chain keeps moving. Each becomes a real op when needed.
                | "SetCharacterSpacing"
                | "GetCharacterSpacing"
                | "SetLineSpacing"
                | "SetTextCase"
                | "GetTextCase"
                | "SetWrapText"
                | "GetWrapText"
                | "SetHorizontalTextAlignment"
                | "GetHorizontalTextAlignment"
                | "SetVerticalTextAlignment"
                | "GetVerticalTextAlignment"
                | "SetOverflowMode"
                | "SetShrinkToFit"
                | "SetIsMarkupEnabled"
                | "SetFont"
                | "GetFont"
                | "SetFontEffect"
                | "SetFontEffectByName"
                | "GetTextWidth"
                | "GetTextHeight"
                | "GetTextSize"
                | "GetTextWidthFromCache"
                | "GetWrappedTextFromCache"
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
        let Some(target) = resolve_logged(world, addr, method)? else {
            return Ok(Value::Nil);
        };

        match method {
            "SetText" => set_text(world, target, args),
            "GetText" => get_text(world, lua, target),
            "SetColor" => set_color(world, target, args),
            "GetColor" => get_color(world, lua, target),
            "SetFontSize" => set_font_size(world, target, args),
            "GetFontSize" => get_font_size(world, target),
            // No-op methods — all accepted to keep the script chain
            // moving without surfacing missing features as errors.
            _ => Ok(Value::Nil),
        }
    }
}

/// Resolve `addr` and record which way it went.
///
/// A miss is a script addressing an element we never spawned, which is
/// worth a warning but not an error — the handler answers `nil`.
fn resolve_logged(
    world: &WorldGuard<'_>,
    addr: &EBusAddress,
    method: &str,
) -> mlua::Result<Option<Entity>> {
    let Some(target) = resolve(world, addr)? else {
        tracing::warn!(
            target: "az_gem_lyshine::script::ui_text",
            ?addr,
            method,
            "could not resolve address (entity not in LyShineEntityIndex)",
        );
        return Ok(None);
    };
    tracing::debug!(
        target: "az_gem_lyshine::script::ui_text",
        bevy_entity = ?target,
        ?addr,
        method,
        "resolved address",
    );
    Ok(Some(target))
}

/// `SetText(addr, text)` — overwrite the entity's `Text` content.
fn set_text(world: &WorldGuard<'_>, target: Entity, args: &[Value]) -> mlua::Result<Value> {
    let text_value = match args.first() {
        Some(Value::String(s)) => s.to_str()?.to_string(),
        Some(Value::Nil) | None => String::new(),
        Some(other) => format!("{other:?}"),
    };
    let mutated = world
        .with_component_mut(
            target,
            |opt: Option<bevy::ecs::change_detection::Mut<Text>>| {
                if let Some(mut text) = opt {
                    text.0.clone_from(&text_value);
                    true
                } else {
                    false
                }
            },
        )
        .map_err(mlua::Error::external)?;
    tracing::debug!(
        target: "az_gem_lyshine::script::ui_text",
        bevy_entity = ?target,
        new_text = %text_value,
        mutated,
        "SetText",
    );
    Ok(Value::Nil)
}

/// `GetText(addr)` — the entity's `Text` content, empty if it has none.
fn get_text(world: &WorldGuard<'_>, lua: &Lua, target: Entity) -> mlua::Result<Value> {
    let text = world
        .with_component(target, |opt: Option<&Text>| {
            opt.map(|t| t.0.clone()).unwrap_or_default()
        })
        .map_err(mlua::Error::external)?;
    Ok(Value::String(lua.create_string(&text)?))
}

/// `SetColor(addr, AzColor)` — tint the entity's `TextColor`.
fn set_color(world: &WorldGuard<'_>, target: Entity, args: &[Value]) -> mlua::Result<Value> {
    let color = match args.first() {
        Some(Value::UserData(ud)) => match ud.borrow::<AzColor>() {
            Ok(c) => *c,
            Err(_) => return Ok(Value::Nil),
        },
        _ => return Ok(Value::Nil),
    };
    world
        .with_component_mut(
            target,
            |opt: Option<bevy::ecs::change_detection::Mut<TextColor>>| {
                if let Some(mut tc) = opt {
                    tc.0 = bevy::color::LinearRgba::new(color.0.x, color.0.y, color.0.z, color.0.w)
                        .into();
                }
            },
        )
        .map_err(mlua::Error::external)?;
    Ok(Value::Nil)
}

/// `GetColor(addr)` — the entity's `TextColor` as an `AzColor` userdata.
fn get_color(world: &WorldGuard<'_>, lua: &Lua, target: Entity) -> mlua::Result<Value> {
    let color = world
        .with_component(target, |opt: Option<&TextColor>| {
            opt.map(|tc| {
                let lin = tc.0.to_linear();
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

/// `SetFontSize(addr, size)` — clamped to at least one pixel.
fn set_font_size(world: &WorldGuard<'_>, target: Entity, args: &[Value]) -> mlua::Result<Value> {
    let Some(size) = convert::scalar(args, 0) else {
        return Ok(Value::Nil);
    };
    world
        .with_component_mut(
            target,
            |opt: Option<bevy::ecs::change_detection::Mut<TextFont>>| {
                if let Some(mut tf) = opt {
                    tf.font_size = FontSize::Px(size.max(1.0));
                }
            },
        )
        .map_err(mlua::Error::external)?;
    Ok(Value::Nil)
}

/// `GetFontSize(addr)` — the entity's font size in pixels.
fn get_font_size(world: &WorldGuard<'_>, target: Entity) -> mlua::Result<Value> {
    let size = world
        .with_component(target, |opt: Option<&TextFont>| {
            opt.map(|tf| f64::from(font_size_pixels(tf.font_size)))
        })
        .map_err(mlua::Error::external)?;
    Ok(size.map_or(Value::Nil, Value::Number))
}

/// The scalar carried by a [`FontSize`], whatever unit it is in.
///
/// `LyShine` only ever authors pixel sizes; the viewport- and
/// root-relative variants carry their number in the same slot, so the
/// bus reports it unconverted rather than answering nil.
const fn font_size_pixels(size: FontSize) -> f32 {
    match size {
        FontSize::Px(value)
        | FontSize::Vw(value)
        | FontSize::Vh(value)
        | FontSize::VMin(value)
        | FontSize::VMax(value)
        | FontSize::Rem(value) => value,
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
