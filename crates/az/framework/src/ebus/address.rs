//! Bus addresses.
//!
//! Lumberyard `EBuses` are parameterised by an "address" type — most commonly
//! `EntityId` (per-entity buses) or "no address" (singleton/broadcast buses).
//! Lua scripts pass addresses verbatim to `.Event.<M>(addr, ...)`, so we
//! need an address type rich enough to round-trip whatever Lua hands us.
//!
//! The address universe needed by reflected public buses:
//!
//! - **None** — `Broadcast` form, or buses that are inherently singletons
//!   with no address type.
//! - **Entity** — most UI buses (`UiElementBus`, `UiCanvasBus`,
//!   `UiCanvasNotificationBus`). The Lua `entityId` is an opaque userdata
//!   wrapping `AZ::EntityId(u64)`. We map it to a Bevy `Entity` via the
//!   `LyShine` entity index.
//! - **Name** — a reflected bus whose address is a string.
//! - **Integer** — a reflected bus whose address is an integer.

use bevy::ecs::entity::Entity;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EBusAddress {
    /// Broadcast / unaddressed.
    Broadcast,
    /// A Bevy entity. Lua-side entity-id userdata is unwrapped here.
    Entity(Entity),
    /// A string-keyed address.
    Name(String),
    /// An integer key (rare; seen in input-action buses).
    Integer(i64),
}

impl EBusAddress {
    /// Best-effort conversion from an arbitrary Lua value:
    ///
    /// - `nil` → `Broadcast`
    /// - integer → `Integer(i)`
    /// - string → `Name(s)`
    /// - `EntityId` userdata → `Integer(id)` so `LyShine` bus handlers
    ///   can resolve via the entity index uniformly with the raw-int
    ///   form legacy UI scripts sometimes use (`UiTextBus.Event.SetText(123, ...)`).
    /// - anything else → `Broadcast`
    #[cfg(any(feature = "lua", feature = "lua54", feature = "luajit"))]
    #[must_use]
    pub fn from_lua(value: &bevy_mod_scripting::lua::mlua::Value) -> Self {
        use crate::lua::types::entity_id::EntityId;
        use bevy_mod_scripting::lua::mlua::Value;
        match value {
            Value::Integer(i) => Self::Integer(*i),
            Value::String(s) => s
                .to_str()
                .map_or(Self::Broadcast, |text| Self::Name(text.to_string())),
            Value::UserData(ud) => {
                // Recognise our typed EntityId — pass the contained
                // u64 through as `Integer` so handlers don't need to
                // know about the userdata wrapping.
                if let Ok(id) = ud.borrow::<EntityId>() {
                    return Self::Integer(id.value().cast_signed());
                }
                Self::Broadcast
            }
            // `Value::Nil` falls in here as well: nil and every other Lua value
            // are both "no address", i.e. `Broadcast`.
            _ => Self::Broadcast,
        }
    }
}
