//! `AzQuaternion` — Lumberyard's `AZ::Quaternion`, glam-backed.
//!
//! Lumberyard reference: `MathReflection.cpp:324`. Imported UI usage is
//! sparse (mostly transforms and camera bus methods returning quats).

use bevy::math::{Quat, Vec3};
use bevy_mod_scripting::lua::mlua::{
    self, Lua, MetaMethod, Result, Table, UserData, UserDataMethods, Value, Variadic,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AzQuaternion(pub Quat);

super::impl_from_lua_for_userdata!(AzQuaternion);

impl Default for AzQuaternion {
    fn default() -> Self {
        Self(Quat::IDENTITY)
    }
}

impl AzQuaternion {
    #[must_use]
    pub const fn identity() -> Self {
        Self(Quat::IDENTITY)
    }

    #[must_use]
    pub const fn from_xyzw(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self(Quat::from_xyzw(x, y, z, w))
    }
}

impl UserData for AzQuaternion {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("GetX", |_, this, ()| Ok(this.0.x));
        methods.add_method("GetY", |_, this, ()| Ok(this.0.y));
        methods.add_method("GetZ", |_, this, ()| Ok(this.0.z));
        methods.add_method("GetW", |_, this, ()| Ok(this.0.w));
        methods.add_method("GetInverseFull", |_, this, ()| Ok(Self(this.0.inverse())));
        methods.add_method("Clone", |_, this, ()| Ok(*this));

        methods.add_meta_method(MetaMethod::Mul, |_, this, other: Self| {
            Ok(Self(this.0 * other.0))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "Quat(x={:.7},y={:.7},z={:.7},w={:.7})",
                this.0.x, this.0.y, this.0.z, this.0.w
            ))
        });
    }
}

/// Register the `Quaternion` namespace and its call-metatable constructor on
/// `globals`.
///
/// # Errors
///
/// Returns the [`mlua::Error`] raised while creating the namespace table, the
/// `CreateIdentity` / `CreateRotationX` / `CreateRotationY` /
/// `CreateRotationZ` / `CreateFromAxisAngle` / constructor functions, or while
/// setting them — including the `raw_set` that publishes `Quaternion` on
/// `globals`.
pub fn register(lua: &Lua, globals: &Table) -> Result<()> {
    let namespace = lua.create_table()?;
    namespace.set(
        "CreateIdentity",
        lua.create_function(|_, ()| Ok(AzQuaternion::identity()))?,
    )?;
    namespace.set(
        "CreateRotationX",
        lua.create_function(|_, rad: f32| Ok(AzQuaternion(Quat::from_rotation_x(rad))))?,
    )?;
    namespace.set(
        "CreateRotationY",
        lua.create_function(|_, rad: f32| Ok(AzQuaternion(Quat::from_rotation_y(rad))))?,
    )?;
    namespace.set(
        "CreateRotationZ",
        lua.create_function(|_, rad: f32| Ok(AzQuaternion(Quat::from_rotation_z(rad))))?,
    )?;
    namespace.set(
        "CreateFromAxisAngle",
        lua.create_function(|_, args: Variadic<f32>| -> Result<AzQuaternion> {
            if args.len() != 4 {
                return Err(mlua::Error::external(
                    "Quaternion.CreateFromAxisAngle expects (x, y, z, angle)",
                ));
            }
            Ok(AzQuaternion(Quat::from_axis_angle(
                Vec3::new(args[0], args[1], args[2]),
                args[3],
            )))
        })?,
    )?;

    let mt = lua.create_table()?;
    let ctor = lua.create_function(|_, args: Variadic<f32>| -> Result<AzQuaternion> {
        match args.len() {
            0 => Ok(AzQuaternion::identity()),
            4 => Ok(AzQuaternion::from_xyzw(args[0], args[1], args[2], args[3])),
            n => Err(mlua::Error::external(format!(
                "Quaternion() expects 0 or 4 numbers, got {n}"
            ))),
        }
    })?;
    mt.set(
        "__call",
        lua.create_function(move |_, args: Variadic<Value>| -> Result<AzQuaternion> {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_precision_loss,
                reason = "Lua numbers arrive as f64/i64 but AZ::Quaternion components are f32; \
                          narrowing at the Lua boundary is the conversion Lumberyard performs"
            )]
            let nums: Variadic<f32> = args
                .into_iter()
                .skip(1)
                .filter_map(|v| match v {
                    Value::Number(n) => Some(n as f32),
                    Value::Integer(i) => Some(i as f32),
                    _ => None,
                })
                .collect();
            ctor.call::<AzQuaternion>(nums)
        })?,
    )?;
    namespace.set_metatable(Some(mt));

    globals.raw_set("Quaternion", namespace)?;
    Ok(())
}
