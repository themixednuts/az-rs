//! `AzUuid` — Lumberyard's `AZ::Uuid` exposed to Lua, backed by
//! `uuid::Uuid`.
//!
//! Lumberyard reference: `MathReflection.cpp:339` registers `Uuid` as a
//! `BehaviorContext::Class<>` with serializer + ctors. Imported UI usage
//! catalogued: `Uuid.Create()`, `Uuid.CreateNull()`, plus the implicit
//! `Uuid()` call form.
//!
//! `Uuid.CreateName(s)` mirrors `AzCore`'s name-UUID algorithm: SHA1 of
//! the input bytes with `AzCore`'s historical version/variant masks.

use az_core::uuid::AzUuidExt;
use bevy_mod_scripting::lua::mlua::{
    self, Lua, MetaMethod, Result, Table, UserData, UserDataMethods, Value, Variadic,
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AzUuid(pub Uuid);

super::impl_from_lua_for_userdata!(AzUuid);

impl AzUuid {
    #[must_use]
    pub const fn null() -> Self {
        Self(Uuid::nil())
    }

    #[must_use]
    pub fn create_random() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub fn create_name(name: &str) -> Self {
        Self(Uuid::create_name(name.as_bytes()))
    }

    pub fn from_string(s: &str) -> Option<Self> {
        Uuid::parse_str(s).ok().map(Self)
    }
}

impl UserData for AzUuid {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("IsNull", |_, this, ()| Ok(this.0.is_nil()));
        methods.add_method("ToString", |_, this, ()| Ok(this.0.to_string()));
        methods.add_method("Clone", |_, this, ()| Ok(*this));

        methods.add_meta_method(MetaMethod::Eq, |_, this, other: Self| Ok(this.0 == other.0));
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(this.0.to_string()));
    }
}

/// Register the `Uuid` namespace and its call-metatable constructor on
/// `globals`.
///
/// # Errors
///
/// Returns the [`mlua::Error`] raised while creating the namespace table, the
/// `Create` / `CreateNull` / `CreateName` / `CreateString` / constructor
/// functions, or while setting them — including the `raw_set` that publishes
/// `Uuid` on `globals`.
pub fn register(lua: &Lua, globals: &Table) -> Result<()> {
    let namespace = lua.create_table()?;
    namespace.set(
        "Create",
        lua.create_function(|_, ()| Ok(AzUuid::create_random()))?,
    )?;
    namespace.set(
        "CreateNull",
        lua.create_function(|_, ()| Ok(AzUuid::null()))?,
    )?;
    namespace.set(
        "CreateName",
        lua.create_function(|_, name: String| Ok(AzUuid::create_name(&name)))?,
    )?;
    namespace.set(
        "CreateString",
        lua.create_function(|_, s: String| {
            AzUuid::from_string(&s)
                .ok_or_else(|| mlua::Error::external(format!("invalid UUID: {s}")))
        })?,
    )?;

    let mt = lua.create_table()?;
    let ctor = lua.create_function(|_, args: Variadic<String>| -> Result<AzUuid> {
        match args.len() {
            0 => Ok(AzUuid::null()),
            1 => AzUuid::from_string(&args[0])
                .ok_or_else(|| mlua::Error::external(format!("invalid UUID: {}", args[0]))),
            n => Err(mlua::Error::external(format!(
                "Uuid() expects 0 or 1 strings, got {n}"
            ))),
        }
    })?;
    mt.set(
        "__call",
        lua.create_function(move |_, args: Variadic<Value>| -> Result<AzUuid> {
            let strs: Variadic<String> = args
                .into_iter()
                .skip(1)
                .filter_map(|v| match v {
                    Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
                    _ => None,
                })
                .collect();
            ctor.call::<AzUuid>(strs)
        })?,
    )?;
    namespace.set_metatable(Some(mt));

    globals.raw_set("Uuid", namespace)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_name_uses_azcore_name_hashing() {
        assert_eq!(
            AzUuid::create_name("hello").0.to_string(),
            "aaf4c61d-dcc5-58a2-9abe-de0f3b482cd9"
        );
    }

    #[test]
    fn create_name_does_not_apply_catalog_path_normalization() {
        assert_ne!(
            AzUuid::create_name("UI/Scripts/Bootstrap.luac"),
            AzUuid::create_name("ui/scripts/bootstrap.luac")
        );
    }
}
