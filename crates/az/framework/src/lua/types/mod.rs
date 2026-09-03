//! Real Lua-bound versions of Lumberyard's `BehaviorContext::Class<T>`
//! reflected types.
//!
//! Lumberyard reference: `Code/Framework/AzCore/AzCore/Math/MathReflection.cpp`
//! lists every type registered. The current method surface is validated
//! against imported UI scripts, so only methods those scripts call are
//! exposed here. Each type is a small
//! `mlua::UserData` wrapper around a glam primitive (or
//! standalone for non-math types) registered as a global constructor +
//! namespace via [`register`].
//!
//! Types implemented:
//!
//! - [`vector3::AzVector3`] — `glam::Vec3`-backed, supports
//!   `Vector3(x, y, z)`, `Vector3.CreateZero()`, `:Get/SetX/Y/Z`,
//!   `:GetLength`, `:Dot`, `:Cross`, etc.
//! - [`vector2::AzVector2`] — `glam::Vec2`-backed, similar surface.
//! - [`color::AzColor`] — `glam::Vec4`-backed (rgba), `.r/.g/.b/.a`
//!   field access, `Color(r,g,b,a)`, `:Clone`, `:IsZero`.
//! - [`entity_id::EntityId`] — `u64` wrapper, `:IsValid`, `:Equal`,
//!   `:GetHashCode`, `EntityId(id)`.
//! - [`uuid::AzUuid`] — `uuid::Uuid` wrapper, `Uuid.Create()`,
//!   `Uuid.CreateNull()`, `Uuid.CreateName(s)` (`AzCore` SHA1 UUID).

pub mod color;
pub mod entity_id;
pub mod quaternion;
pub mod uuid;
pub mod vector2;
pub mod vector3;
pub mod vector4;

use bevy_mod_scripting::lua::mlua::{Lua, Result};

/// Helper macro: implement `FromLua` for a `UserData + Copy` type so it
/// can be received as a typed method argument (e.g.
/// `methods.add_method("Cross", |_, this, other: AzVector3| ...)`).
///
/// mlua doesn't blanket-impl `FromLua` for every `UserData` type — each
/// type needs an explicit conversion that downcasts via the standard
/// `Value::UserData` → `AnyUserData::borrow::<T>` path.
macro_rules! impl_from_lua_for_userdata {
    ($t:ty) => {
        impl bevy_mod_scripting::lua::mlua::FromLua for $t {
            fn from_lua(
                value: bevy_mod_scripting::lua::mlua::Value,
                _: &bevy_mod_scripting::lua::mlua::Lua,
            ) -> bevy_mod_scripting::lua::mlua::Result<Self> {
                match value {
                    bevy_mod_scripting::lua::mlua::Value::UserData(ud) => Ok(*ud.borrow::<$t>()?),
                    other => Err(
                        bevy_mod_scripting::lua::mlua::Error::FromLuaConversionError {
                            from: other.type_name(),
                            to: stringify!($t).into(),
                            message: Some(format!(
                                "expected {} userdata, got {:?}",
                                stringify!($t),
                                other
                            )),
                        },
                    ),
                }
            }
        }
    };
}
pub(crate) use impl_from_lua_for_userdata;

/// Install every real reflected type as a global constructor + namespace
/// on `lua`. Replaces the [`super::globals::no_op_proxy`] stubs for the
/// types we have real implementations for.
///
/// # Errors
///
/// Returns the `mlua::Error` raised by the first per-type `register` call that
/// fails — `Vector2`, `Vector3`, `Vector4`, `Color`, `Quaternion`, `EntityId`
/// or `Uuid` — while building its namespace table or publishing it on `_G`.
pub fn register(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    vector2::register(lua, &globals)?;
    vector3::register(lua, &globals)?;
    vector4::register(lua, &globals)?;
    color::register(lua, &globals)?;
    quaternion::register(lua, &globals)?;
    entity_id::register(lua, &globals)?;
    uuid::register(lua, &globals)?;
    Ok(())
}
