//! `AZ::EntityId` exposed to Lua, backed by [`az_core::EntityId`].
//!
//! In imported `LyShine` Lua, `self.entityId` carries a UI entity id that bus
//! handlers resolve against `LyShineEntityIndex` (see
//! `gems/lyshine/src/script/entity_index.rs`). The `bms_bridge`
//! bootstrap currently sets `self.entityId = <u64>` raw — once this type
//! is registered we'll switch the bootstrap to wrap it in `EntityId(id)`
//! so address conversions can pull the typed value out of the Lua arg.
//!
//! Lua API:
//!
//! ```lua
//! local e = EntityId(0xdeadbeef)
//! e:IsValid()                  -- true unless id == 0
//! e:Equal(other)
//! e:GetHashCode()
//! tostring(e)                  -- "[EntityId 0xdeadbeef]"
//! EntityId.InvalidEntityId     -- static: id 0
//! ```

use az_core::EntityId as CoreEntityId;
use bevy_mod_scripting::lua::mlua::{
    self, Lua, MetaMethod, Result, Table, UserData, UserDataFields, UserDataMethods, Value,
    Variadic,
};

/// Lua userdata wrapper around [`CoreEntityId`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EntityId(pub CoreEntityId);

super::impl_from_lua_for_userdata!(EntityId);

impl EntityId {
    pub const INVALID: Self = Self(CoreEntityId::INVALID);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(CoreEntityId::new(value))
    }

    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0.is_valid()
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0.value()
    }
}

impl UserData for EntityId {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("value", |_, this| Ok(this.value()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("IsValid", |_, this, ()| Ok(this.is_valid()));
        methods.add_method("Equal", |_, this, other: Self| Ok(*this == other));
        methods.add_method("GetHashCode", |_, this, ()| Ok(this.value()));
        methods.add_method("Clone", |_, this, ()| Ok(*this));

        methods.add_meta_method(MetaMethod::Eq, |_, this, other: Self| Ok(*this == other));
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("[EntityId {:#x}]", this.value()))
        });
    }
}

/// Register the `EntityId` namespace and its call-metatable constructor on
/// `globals`.
///
/// # Errors
///
/// Returns the [`mlua::Error`] raised while creating the namespace table, the
/// `CreateInvalid` / constructor functions, or while setting them — including
/// the `raw_set` that publishes `EntityId` on `globals`.
pub fn register(lua: &Lua, globals: &Table) -> Result<()> {
    let namespace = lua.create_table()?;
    namespace.set("InvalidEntityId", EntityId::INVALID)?;
    namespace.set(
        "CreateInvalid",
        lua.create_function(|_, ()| Ok(EntityId::INVALID))?,
    )?;

    let mt = lua.create_table()?;
    let ctor = lua.create_function(|_, args: Variadic<i64>| -> Result<EntityId> {
        match args.len() {
            0 => Ok(EntityId::INVALID),
            1 => Ok(EntityId::new(args[0].cast_unsigned())),
            n => Err(mlua::Error::external(format!(
                "EntityId() expects 0 or 1 numbers, got {n}"
            ))),
        }
    })?;
    mt.set(
        "__call",
        lua.create_function(move |_, args: Variadic<Value>| -> Result<EntityId> {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a Lua float entity id has no fractional part to keep; truncating to \
                          i64 matches how legacy scripts pass raw numeric ids"
            )]
            let ints: Variadic<i64> = args
                .into_iter()
                .skip(1)
                .filter_map(|v| match v {
                    Value::Integer(i) => Some(i),
                    Value::Number(n) => Some(n as i64),
                    _ => None,
                })
                .collect();
            ctor.call(ints)
        })?,
    )?;
    namespace.set_metatable(Some(mt));

    globals.raw_set("EntityId", namespace)?;
    Ok(())
}
