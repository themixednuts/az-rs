//! `AzVector2` — Lumberyard's `AZ::Vector2` exposed to Lua, backed by
//! `glam::Vec2`. Same surface as [`super::vector3`] minus the z axis.
//!
//! Imported UI usage: minimal — observed `Vector2(x, y)` constructor and
//! `Vector2:CreateZero()` only. Full method set provided so canvas
//! authoring scripts that touch UI sizes have what they need.

use bevy::math::Vec2;
use bevy_mod_scripting::lua::mlua::{
    self, Lua, MetaMethod, Result, Table, UserData, UserDataFields, UserDataMethods, Value,
    Variadic,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AzVector2(pub Vec2);

super::impl_from_lua_for_userdata!(AzVector2);

impl AzVector2 {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self(Vec2::new(x, y))
    }

    #[must_use]
    pub const fn zero() -> Self {
        Self(Vec2::ZERO)
    }
}

impl UserData for AzVector2 {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("x", |_, this| Ok(this.0.x));
        fields.add_field_method_set("x", |_, this, v: f32| {
            this.0.x = v;
            Ok(())
        });
        fields.add_field_method_get("y", |_, this| Ok(this.0.y));
        fields.add_field_method_set("y", |_, this, v: f32| {
            this.0.y = v;
            Ok(())
        });
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "the `__mul` scalar arrives as a Lua f64/i64 but AZ::Vector2 components are \
                  f32; narrowing at the Lua boundary is the conversion Lumberyard performs"
    )]
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("GetX", |_, this, ()| Ok(this.0.x));
        methods.add_method("GetY", |_, this, ()| Ok(this.0.y));
        methods.add_method_mut("SetX", |_, this, v: f32| {
            this.0.x = v;
            Ok(())
        });
        methods.add_method_mut("SetY", |_, this, v: f32| {
            this.0.y = v;
            Ok(())
        });
        methods.add_method("GetLength", |_, this, ()| Ok(this.0.length()));
        methods.add_method("GetLengthSq", |_, this, ()| Ok(this.0.length_squared()));
        methods.add_method("GetNormalized", |_, this, ()| {
            Ok(Self(this.0.normalize_or_zero()))
        });
        methods.add_method_mut("Normalize", |_, this, ()| {
            this.0 = this.0.normalize_or_zero();
            Ok(())
        });
        methods.add_method("Dot", |_, this, other: Self| Ok(this.0.dot(other.0)));
        methods.add_method("Clone", |_, this, ()| Ok(*this));

        methods.add_meta_method(MetaMethod::Add, |_, this, other: Self| {
            Ok(Self(this.0 + other.0))
        });
        methods.add_meta_method(MetaMethod::Sub, |_, this, other: Self| {
            Ok(Self(this.0 - other.0))
        });
        methods.add_meta_method(MetaMethod::Mul, |_, this, other: Value| match other {
            Value::Number(s) => Ok(Self(this.0 * s as f32)),
            Value::Integer(s) => Ok(Self(this.0 * s as f32)),
            Value::UserData(u) => {
                let v = u.borrow::<Self>()?;
                Ok(Self(this.0 * v.0))
            }
            other => Err(mlua::Error::external(format!(
                "Vector2 * expects number or Vector2, got {other:?}"
            ))),
        });
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: Self| Ok(this.0 == other.0));
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("(x={:.7},y={:.7})", this.0.x, this.0.y))
        });
    }
}

/// Register the `Vector2` namespace and its call-metatable constructor on
/// `globals`.
///
/// # Errors
///
/// Returns the [`mlua::Error`] raised while creating the namespace table, the
/// `CreateZero` / `ConstructFromValues` / constructor functions, or while
/// setting them — including the `raw_set` that publishes `Vector2` on
/// `globals`.
pub fn register(lua: &Lua, globals: &Table) -> Result<()> {
    let namespace = lua.create_table()?;
    namespace.set(
        "CreateZero",
        lua.create_function(|_, ()| Ok(AzVector2::zero()))?,
    )?;
    namespace.set(
        "ConstructFromValues",
        lua.create_function(|_, (x, y): (f32, f32)| Ok(AzVector2::new(x, y)))?,
    )?;

    let mt = lua.create_table()?;
    let ctor = lua.create_function(|_, args: Variadic<f32>| -> Result<AzVector2> {
        match args.len() {
            0 => Ok(AzVector2::zero()),
            2 => Ok(AzVector2::new(args[0], args[1])),
            n => Err(mlua::Error::external(format!(
                "Vector2() expects 0 or 2 numbers, got {n}"
            ))),
        }
    })?;
    mt.set(
        "__call",
        lua.create_function(move |_, args: Variadic<Value>| -> Result<AzVector2> {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_precision_loss,
                reason = "Lua numbers arrive as f64/i64 but AZ::Vector2 components are f32; \
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
            ctor.call::<AzVector2>(nums)
        })?,
    )?;
    namespace.set_metatable(Some(mt));

    globals.raw_set("Vector2", namespace)?;
    Ok(())
}
