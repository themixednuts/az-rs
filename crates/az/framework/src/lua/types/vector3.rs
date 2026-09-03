//! `AzVector3` — Lumberyard's `AZ::Vector3` exposed to Lua, backed by
//! `glam::Vec3`.
//!
//! Lumberyard reference:
//! O3DE reference: `Code/Framework/AzCore/AzCore/Math/MathReflection.cpp:484-540` for
//! construction and line 2400 onward for method registration. The binding
//! exposes the methods used by Azoth scripts.
//!
//! Lua API:
//!
//! ```lua
//! local v = Vector3(1, 2, 3)               -- constructor (3 args)
//! local v = Vector3(5)                      -- splat constructor (1 arg)
//! local v = Vector3.CreateZero()            -- static factory
//! local v = Vector3.ConstructFromValues(1, 2, 3)
//! v:GetX(); v:SetX(10)                      -- per-axis accessors
//! v:GetLength(); v:GetLengthSq()            -- magnitude
//! v:GetNormalized(); v:Normalize()          -- normalisation
//! v:Dot(other); v:Cross(other)              -- vector ops
//! v:GetDistance(other)                      -- distance to other
//! v:Lerp(other, t)                          -- linear interpolation
//! tostring(v)                                -- "(x=..,y=..,z=..)"
//! v + w; v - w; v * scalar; -v               -- arithmetic
//! ```

use bevy::math::Vec3;
use bevy_mod_scripting::lua::mlua::{
    self, Lua, MetaMethod, Result, Table, UserData, UserDataFields, UserDataMethods, Value,
    Variadic,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AzVector3(pub Vec3);

super::impl_from_lua_for_userdata!(AzVector3);

impl AzVector3 {
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self(Vec3::new(x, y, z))
    }

    #[must_use]
    pub const fn splat(v: f32) -> Self {
        Self(Vec3::splat(v))
    }

    #[must_use]
    pub const fn zero() -> Self {
        Self(Vec3::ZERO)
    }
}

impl UserData for AzVector3 {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        // AZ Vector3 doesn't expose `.x/.y/.z` as direct fields in stock
        // Lumberyard reflection — scripts use `:GetX()` etc. We expose
        // both for convenience: `.x` / `.y` / `.z` mirror the glam fields,
        // `:GetX()` mirrors AZ.
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
        fields.add_field_method_get("z", |_, this| Ok(this.0.z));
        fields.add_field_method_set("z", |_, this, v: f32| {
            this.0.z = v;
            Ok(())
        });
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "the `__mul` scalar arrives as a Lua f64/i64 but AZ::Vector3 components are \
                  f32; narrowing at the Lua boundary is the conversion Lumberyard performs"
    )]
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Per-axis accessors (mirror AZ::Vector3::GetX etc.)
        methods.add_method("GetX", |_, this, ()| Ok(this.0.x));
        methods.add_method("GetY", |_, this, ()| Ok(this.0.y));
        methods.add_method("GetZ", |_, this, ()| Ok(this.0.z));
        methods.add_method_mut("SetX", |_, this, v: f32| {
            this.0.x = v;
            Ok(())
        });
        methods.add_method_mut("SetY", |_, this, v: f32| {
            this.0.y = v;
            Ok(())
        });
        methods.add_method_mut("SetZ", |_, this, v: f32| {
            this.0.z = v;
            Ok(())
        });
        methods.add_method_mut("Set", |_, this, args: Variadic<f32>| {
            match args.len() {
                1 => this.0 = Vec3::splat(args[0]),
                3 => this.0 = Vec3::new(args[0], args[1], args[2]),
                n => {
                    return Err(mlua::Error::external(format!(
                        "Vector3:Set expects 1 or 3 numbers, got {n}"
                    )));
                }
            }
            Ok(())
        });

        // Magnitude / normalisation
        methods.add_method("GetLength", |_, this, ()| Ok(this.0.length()));
        methods.add_method("GetLengthSq", |_, this, ()| Ok(this.0.length_squared()));
        methods.add_method("GetNormalized", |_, this, ()| {
            Ok(Self(this.0.normalize_or_zero()))
        });
        methods.add_method_mut("Normalize", |_, this, ()| {
            this.0 = this.0.normalize_or_zero();
            Ok(())
        });
        methods.add_method("IsZero", |_, this, ()| {
            Ok(this.0.length_squared() < f32::EPSILON)
        });

        // Vector ops
        methods.add_method("Dot", |_, this, other: Self| Ok(this.0.dot(other.0)));
        methods.add_method("Cross", |_, this, other: Self| {
            Ok(Self(this.0.cross(other.0)))
        });
        methods.add_method("CrossZAxis", |_, this, ()| {
            // AZ::Vector3::CrossZAxis = Cross(Vec3::Z) — convenience
            Ok(Self(this.0.cross(Vec3::Z)))
        });
        methods.add_method("GetDistance", |_, this, other: Self| {
            Ok((this.0 - other.0).length())
        });
        methods.add_method("GetDistanceSq", |_, this, other: Self| {
            Ok((this.0 - other.0).length_squared())
        });
        methods.add_method("Lerp", |_, this, (other, t): (Self, f32)| {
            Ok(Self(this.0.lerp(other.0, t)))
        });
        methods.add_method("Clone", |_, this, ()| Ok(*this));

        // Metamethods: arithmetic + tostring + equality
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
                "Vector3 * expects number or Vector3, got {other:?}"
            ))),
        });
        methods.add_meta_method(MetaMethod::Div, |_, this, scalar: f32| {
            Ok(Self(this.0 / scalar))
        });
        methods.add_meta_method(MetaMethod::Unm, |_, this, ()| Ok(Self(-this.0)));
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: Self| Ok(this.0 == other.0));
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "(x={:.7},y={:.7},z={:.7})",
                this.0.x, this.0.y, this.0.z
            ))
        });
    }
}

/// Install `Vector3` global on the Lua state. Both call form
/// (`Vector3(x, y, z)`) and namespace form (`Vector3.CreateZero()`)
/// are supported — Lumberyard's `BehaviorContext` registers it as both.
///
/// # Errors
///
/// Returns the [`mlua::Error`] raised while creating the namespace table, the
/// `CreateZero` / `ConstructFromValues` / constructor functions, or while
/// setting them — including the `raw_set` that publishes `Vector3` on
/// `globals`.
pub fn register(lua: &Lua, globals: &Table) -> Result<()> {
    // Namespace table — holds static factory methods. Same table is
    // returned to scripts that do `Vector3.X(...)`.
    let namespace = lua.create_table()?;
    namespace.set(
        "CreateZero",
        lua.create_function(|_, ()| Ok(AzVector3::zero()))?,
    )?;
    namespace.set(
        "ConstructFromValues",
        lua.create_function(|_, (x, y, z): (f32, f32, f32)| Ok(AzVector3::new(x, y, z)))?,
    )?;

    // Make the namespace table also CALLABLE: `Vector3(x, y, z)` →
    // construct. Standard Lua metatable trick — `__call` on the
    // namespace fires the constructor.
    let mt = lua.create_table()?;
    let ctor = lua.create_function(|_, args: Variadic<f32>| -> Result<AzVector3> {
        match args.len() {
            0 => Ok(AzVector3::zero()),
            1 => Ok(AzVector3::splat(args[0])),
            3 => Ok(AzVector3::new(args[0], args[1], args[2])),
            n => Err(mlua::Error::external(format!(
                "Vector3() expects 0, 1, or 3 numbers, got {n}"
            ))),
        }
    })?;
    mt.set(
        "__call",
        lua.create_function(move |_, args: Variadic<Value>| -> Result<AzVector3> {
            // Drop the implicit `self` arg the call-form passes.
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_precision_loss,
                reason = "Lua numbers arrive as f64/i64 but AZ::Vector3 components are f32; \
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
            ctor.call::<AzVector3>(nums)
        })?,
    )?;
    namespace.set_metatable(Some(mt));

    globals.raw_set("Vector3", namespace)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_mod_scripting::lua::mlua::Lua;

    fn lua_with_vector3() -> Lua {
        let lua = Lua::new();
        register(&lua, &lua.globals()).unwrap();
        lua
    }

    #[test]
    fn constructor_three_args() {
        let lua = lua_with_vector3();
        let v: AzVector3 = lua.load("return Vector3(1, 2, 3)").eval().unwrap();
        assert_eq!(v.0, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn constructor_splat_one_arg() {
        let lua = lua_with_vector3();
        let v: AzVector3 = lua.load("return Vector3(7)").eval().unwrap();
        assert_eq!(v.0, Vec3::new(7.0, 7.0, 7.0));
    }

    #[test]
    fn create_zero_static() {
        let lua = lua_with_vector3();
        let v: AzVector3 = lua.load("return Vector3.CreateZero()").eval().unwrap();
        assert_eq!(v.0, Vec3::ZERO);
    }

    #[test]
    fn arithmetic_and_methods() {
        let lua = lua_with_vector3();
        let len: f32 = lua
            .load(
                r"
                local a = Vector3(1, 2, 2)  -- length 3
                local b = Vector3(0, 0, 4)
                local sum = a + b           -- (1, 2, 6)
                return sum:GetLength()      -- sqrt(1+4+36) = ~6.4031
            ",
            )
            .eval()
            .unwrap();
        assert!((len - (1.0_f32 + 4.0 + 36.0).sqrt()).abs() < 1e-5);
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "a dot product of two orthogonal unit axes is exactly zero, and an epsilon here would hide a wrong result"
    )]
    fn cross_and_dot() {
        let lua = lua_with_vector3();
        let dot: f32 = lua
            .load("return Vector3(1, 0, 0):Dot(Vector3(0, 1, 0))")
            .eval()
            .unwrap();
        assert_eq!(dot, 0.0);

        let cross: AzVector3 = lua
            .load("return Vector3(1, 0, 0):Cross(Vector3(0, 1, 0))")
            .eval()
            .unwrap();
        assert_eq!(cross.0, Vec3::Z);
    }

    #[test]
    fn field_access_xyz() {
        let lua = lua_with_vector3();
        let xyz: (f32, f32, f32) = lua
            .load(
                r"
                local v = Vector3(10, 20, 30)
                return v.x, v.y, v.z
            ",
            )
            .eval()
            .unwrap();
        assert_eq!(xyz, (10.0, 20.0, 30.0));
    }
}
