//! `AzVector4` — Lumberyard's `AZ::Vector4`, glam-backed.

use bevy::math::Vec4;
use bevy_mod_scripting::lua::mlua::{
    self, Lua, MetaMethod, Result, Table, UserData, UserDataFields, UserDataMethods, Value,
    Variadic,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AzVector4(pub Vec4);

super::impl_from_lua_for_userdata!(AzVector4);

impl AzVector4 {
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self(Vec4::new(x, y, z, w))
    }

    #[must_use]
    pub const fn zero() -> Self {
        Self(Vec4::ZERO)
    }
}

impl UserData for AzVector4 {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("x", |_, this| Ok(this.0.x));
        fields.add_field_method_get("y", |_, this| Ok(this.0.y));
        fields.add_field_method_get("z", |_, this| Ok(this.0.z));
        fields.add_field_method_get("w", |_, this| Ok(this.0.w));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("GetX", |_, this, ()| Ok(this.0.x));
        methods.add_method("GetY", |_, this, ()| Ok(this.0.y));
        methods.add_method("GetZ", |_, this, ()| Ok(this.0.z));
        methods.add_method("GetW", |_, this, ()| Ok(this.0.w));
        methods.add_method("GetLength", |_, this, ()| Ok(this.0.length()));
        methods.add_method("Dot", |_, this, other: Self| Ok(this.0.dot(other.0)));
        methods.add_method("Clone", |_, this, ()| Ok(*this));

        methods.add_meta_method(MetaMethod::Add, |_, this, other: Self| {
            Ok(Self(this.0 + other.0))
        });
        methods.add_meta_method(MetaMethod::Sub, |_, this, other: Self| {
            Ok(Self(this.0 - other.0))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "(x={:.7},y={:.7},z={:.7},w={:.7})",
                this.0.x, this.0.y, this.0.z, this.0.w
            ))
        });
    }
}

/// Register the `Vector4` namespace and its call-metatable constructor on
/// `globals`.
///
/// # Errors
///
/// Returns the [`mlua::Error`] raised while creating the namespace table, the
/// `CreateZero` / constructor functions, or while setting them — including the
/// `raw_set` that publishes `Vector4` on `globals`.
pub fn register(lua: &Lua, globals: &Table) -> Result<()> {
    let namespace = lua.create_table()?;
    namespace.set(
        "CreateZero",
        lua.create_function(|_, ()| Ok(AzVector4::zero()))?,
    )?;

    let mt = lua.create_table()?;
    let ctor = lua.create_function(|_, args: Variadic<f32>| -> Result<AzVector4> {
        match args.len() {
            0 => Ok(AzVector4::zero()),
            4 => Ok(AzVector4::new(args[0], args[1], args[2], args[3])),
            n => Err(mlua::Error::external(format!(
                "Vector4() expects 0 or 4 numbers, got {n}"
            ))),
        }
    })?;
    mt.set(
        "__call",
        lua.create_function(move |_, args: Variadic<Value>| -> Result<AzVector4> {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_precision_loss,
                reason = "Lua numbers arrive as f64/i64 but AZ::Vector4 components are f32; \
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
            ctor.call::<AzVector4>(nums)
        })?,
    )?;
    namespace.set_metatable(Some(mt));

    globals.raw_set("Vector4", namespace)?;
    Ok(())
}
