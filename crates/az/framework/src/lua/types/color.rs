//! `AzColor` — Lumberyard's `AZ::Color` exposed to Lua, backed by
//! `glam::Vec4` (RGBA, normalised 0..=1 channels).
//!
//! Scripts can access `.r`, `.g`, `.b`, and `.a`, clone a value, and test for
//! zero. `Color(r, g, b, a)` accepts four normalized channels.
//!
//! Lumberyard reference: `MathReflection.cpp` Color ctor +
//! `MathReflection.cpp:900+` for `.r/.g/.b/.a` accessors.

use bevy::math::Vec4;
use bevy_mod_scripting::lua::mlua::{
    self, Lua, MetaMethod, Result, Table, UserData, UserDataFields, UserDataMethods, Value,
    Variadic,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AzColor(pub Vec4);

super::impl_from_lua_for_userdata!(AzColor);

impl AzColor {
    #[must_use]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self(Vec4::new(r, g, b, a))
    }

    #[must_use]
    pub const fn zero() -> Self {
        Self(Vec4::ZERO)
    }
}

impl UserData for AzColor {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("r", |_, this| Ok(this.0.x));
        fields.add_field_method_set("r", |_, this, v: f32| {
            this.0.x = v;
            Ok(())
        });
        fields.add_field_method_get("g", |_, this| Ok(this.0.y));
        fields.add_field_method_set("g", |_, this, v: f32| {
            this.0.y = v;
            Ok(())
        });
        fields.add_field_method_get("b", |_, this| Ok(this.0.z));
        fields.add_field_method_set("b", |_, this, v: f32| {
            this.0.z = v;
            Ok(())
        });
        fields.add_field_method_get("a", |_, this| Ok(this.0.w));
        fields.add_field_method_set("a", |_, this, v: f32| {
            this.0.w = v;
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("GetR", |_, this, ()| Ok(this.0.x));
        methods.add_method("GetG", |_, this, ()| Ok(this.0.y));
        methods.add_method("GetB", |_, this, ()| Ok(this.0.z));
        methods.add_method("GetA", |_, this, ()| Ok(this.0.w));
        methods.add_method_mut("SetR", |_, this, v: f32| {
            this.0.x = v;
            Ok(())
        });
        methods.add_method_mut("SetG", |_, this, v: f32| {
            this.0.y = v;
            Ok(())
        });
        methods.add_method_mut("SetB", |_, this, v: f32| {
            this.0.z = v;
            Ok(())
        });
        methods.add_method_mut("SetA", |_, this, v: f32| {
            this.0.w = v;
            Ok(())
        });
        methods.add_method("Clone", |_, this, ()| Ok(*this));
        methods.add_method("IsZero", |_, this, ()| Ok(this.0 == Vec4::ZERO));

        methods.add_meta_method(MetaMethod::Eq, |_, this, other: Self| Ok(this.0 == other.0));
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "(r={:.3},g={:.3},b={:.3},a={:.3})",
                this.0.x, this.0.y, this.0.z, this.0.w
            ))
        });
    }
}

/// Register the `Color` namespace and its call-metatable constructor on
/// `globals`.
///
/// # Errors
///
/// Returns the [`mlua::Error`] raised while creating the namespace table, the
/// `CreateZero` / `CreateOne` / constructor functions, or while setting them —
/// including the `raw_set` that publishes `Color` on `globals`.
pub fn register(lua: &Lua, globals: &Table) -> Result<()> {
    let namespace = lua.create_table()?;
    namespace.set(
        "CreateZero",
        lua.create_function(|_, ()| Ok(AzColor::zero()))?,
    )?;
    namespace.set(
        "CreateOne",
        lua.create_function(|_, ()| Ok(AzColor::new(1.0, 1.0, 1.0, 1.0)))?,
    )?;

    let mt = lua.create_table()?;
    let ctor = lua.create_function(|_, args: Variadic<f32>| -> Result<AzColor> {
        match args.len() {
            0 => Ok(AzColor::zero()),
            3 => Ok(AzColor::new(args[0], args[1], args[2], 1.0)),
            4 => Ok(AzColor::new(args[0], args[1], args[2], args[3])),
            n => Err(mlua::Error::external(format!(
                "Color() expects 0, 3, or 4 numbers, got {n}"
            ))),
        }
    })?;
    mt.set(
        "__call",
        lua.create_function(move |_, args: Variadic<Value>| -> Result<AzColor> {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_precision_loss,
                reason = "Lua numbers arrive as f64/i64 but AZ::Color channels are f32; \
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
            ctor.call::<AzColor>(nums)
        })?,
    )?;
    namespace.set_metatable(Some(mt));

    globals.raw_set("Color", namespace)?;
    Ok(())
}
