//! Native `EBus` calls queued from Bevy systems.
//!
//! Lua scripts can dispatch `EBuses` directly through the reflected
//! `Bus.Broadcast` / `Bus.Event` tables. Native Bevy systems use
//! [`EBusCall`] so they do not need to know about Lua contexts or `mlua`
//! values.

use az_core::EntityId;
use bevy::ecs::message::Message;
use glam::{Quat, Vec2, Vec3, Vec4};

use super::address::EBusAddress;

/// Prefix used for buses created by Lua's `DynamicBus[name]` indexer.
pub const DYNAMIC_BUS_PREFIX: &str = "dyn:";

/// A bus method call emitted by native runtime systems.
#[derive(Debug, Clone, Message)]
pub struct EBusCall {
    bus: String,
    address: Option<EBusAddress>,
    method: String,
    args: Vec<EBusArg>,
}

impl EBusCall {
    /// Call `Bus.Broadcast.method(...)`.
    pub fn broadcast(bus: impl Into<String>, method: impl Into<String>) -> Self {
        Self {
            bus: bus.into(),
            address: None,
            method: method.into(),
            args: Vec::new(),
        }
    }

    /// Call `Bus.Event.method(address, ...)`.
    pub fn event(bus: impl Into<String>, address: EBusAddress, method: impl Into<String>) -> Self {
        Self {
            bus: bus.into(),
            address: Some(address),
            method: method.into(),
            args: Vec::new(),
        }
    }

    /// Call `DynamicBus.name.Broadcast.method(...)`.
    pub fn dynamic(name: impl AsRef<str>, method: impl Into<String>) -> Self {
        Self::broadcast(dynamic_bus_name(name), method)
    }

    /// Call `DynamicBus.name.Event.method(address, ...)`.
    pub fn dynamic_event(
        name: impl AsRef<str>,
        address: EBusAddress,
        method: impl Into<String>,
    ) -> Self {
        Self::event(dynamic_bus_name(name), address, method)
    }

    /// Add one argument to the call.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<EBusArg>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Add multiple arguments to the call.
    #[must_use]
    pub fn args<I, A>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: Into<EBusArg>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Registered bus name.
    #[must_use]
    pub fn bus(&self) -> &str {
        &self.bus
    }

    /// Address for `Event` calls. `None` means `Broadcast`.
    #[must_use]
    pub const fn address(&self) -> Option<&EBusAddress> {
        self.address.as_ref()
    }

    /// Method name on the bus listener.
    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Arguments passed to the bus listener.
    #[must_use]
    pub fn call_args(&self) -> &[EBusArg] {
        &self.args
    }
}

fn dynamic_bus_name(name: impl AsRef<str>) -> String {
    format!("{DYNAMIC_BUS_PREFIX}{}", name.as_ref())
}

/// Argument value for a native-to-script bus call.
#[derive(Debug, Clone, PartialEq)]
pub enum EBusArg {
    Nil,
    Bool(bool),
    Integer(i64),
    Number(f64),
    String(String),
    EntityId(EntityId),
    Address(EBusAddress),
    Vec2(Vec2),
    Vec3(Vec3),
    Vec4(Vec4),
    Quat(Quat),
}

impl From<bool> for EBusArg {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

macro_rules! impl_integer_arg {
    ($($t:ty),* $(,)?) => {
        $(
            impl From<$t> for EBusArg {
                fn from(value: $t) -> Self {
                    Self::Integer(i64::from(value))
                }
            }
        )*
    };
}

impl_integer_arg!(i8, i16, i32, i64, u8, u16, u32);

impl From<f32> for EBusArg {
    fn from(value: f32) -> Self {
        Self::Number(f64::from(value))
    }
}

impl From<f64> for EBusArg {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<String> for EBusArg {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for EBusArg {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<EntityId> for EBusArg {
    fn from(value: EntityId) -> Self {
        Self::EntityId(value)
    }
}

impl From<EBusAddress> for EBusArg {
    fn from(value: EBusAddress) -> Self {
        Self::Address(value)
    }
}

impl From<Vec2> for EBusArg {
    fn from(value: Vec2) -> Self {
        Self::Vec2(value)
    }
}

impl From<Vec3> for EBusArg {
    fn from(value: Vec3) -> Self {
        Self::Vec3(value)
    }
}

impl From<Vec4> for EBusArg {
    fn from(value: Vec4) -> Self {
        Self::Vec4(value)
    }
}

impl From<Quat> for EBusArg {
    fn from(value: Quat) -> Self {
        Self::Quat(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_calls_use_registry_bus_name() {
        let call = EBusCall::dynamic("uiStateBus", "onStateChanged").arg(true);

        assert_eq!(call.bus(), "dyn:uiStateBus");
        assert_eq!(call.method(), "onStateChanged");
        assert_eq!(call.address(), None);
        assert_eq!(call.call_args(), &[EBusArg::Bool(true)]);
    }

    #[test]
    fn dynamic_event_calls_use_registry_bus_name_and_address() {
        let call =
            EBusCall::dynamic_event("uiStateBus", EBusAddress::Integer(0xfeed), "applyState")
                .arg(true);

        assert_eq!(call.bus(), "dyn:uiStateBus");
        assert_eq!(call.method(), "applyState");
        assert_eq!(call.address(), Some(&EBusAddress::Integer(0xfeed)));
        assert_eq!(call.call_args(), &[EBusArg::Bool(true)]);
    }
}
