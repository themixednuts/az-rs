//! `EBusRegistry` — Bevy [`Resource`] that holds every AZ `EBus` subscription.
//!
//! ## Why a Bevy resource?
//!
//! Lua scripts and native code both need to:
//! - register handlers (Lua via `Bus.Connect`, native at plugin boot)
//! - dispatch calls (Lua via `Bus.Broadcast.<M>`, native via direct call)
//!
//! Putting the registry in Bevy ECS means it's reachable from both sides
//! through the standard `World` ↔ Lua bridge BMS already provides
//! ([`bevy_mod_scripting::bindings::ThreadWorldContainer`]).
//!
//! ## Subscription model
//!
//! Each subscription has a (`bus_name`, address) key and a [`HandlerKind`]:
//!
//! - [`HandlerKind::Native`] — boxed [`NativeBusHandler`] trait object.
//!   Plugins install these at boot. They get `&WorldGuard` access during
//!   dispatch so they can mutate Bevy state.
//! - [`HandlerKind::Lua`] — a Lua listener table. Dispatch calls
//!   `listener:method(args)` on the Lua side.
//!
//! [`SubscriptionId`] is a stable handle returned by `connect_*`; the Lua
//! `Bus.Connect` wrapper hands it back inside the disconnect-handle table
//! it returns to the script.
//!
//! ## Dispatch
//!
//! [`EBusRegistry::dispatch`] walks every subscription matching
//! (bus, address) and calls each handler in connect order. Returns the last
//! non-nil return value. Native handlers and Lua handlers participate
//! identically.
//!
//! Broadcast vs Event: a `Broadcast` call dispatches to every subscription
//! whose key starts with `bus` — across ALL addresses. An `Event(addr)`
//! call dispatches only to subscriptions at exactly `(bus, addr)`. This
//! mirrors Lumberyard's `EBusBroadcaster` vs `EBusEventer` semantics.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::ecs::resource::Resource;
use bevy_mod_scripting::bindings::WorldGuard;
use bevy_mod_scripting::lua::mlua::{self, Lua, Table, Value};
use parking_lot::RwLock;

use super::address::EBusAddress;

/// Stable handle for a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

/// Native-side `EBus` handler. Implementors decide which methods they care
/// about (via [`Self::handles`]) and how to execute them ([`Self::dispatch`]).
///
/// Only methods returning `true` from `handles` participate in dispatch —
/// this lets a single handler register multiple methods on the same bus
/// without accidentally hooking unknown methods.
pub trait NativeBusHandler: Send + Sync + 'static {
    /// Whether this handler implements `method`.
    fn handles(&self, method: &str) -> bool;

    /// Execute `method` with `args`. Receives [`WorldGuard`] for safe
    /// resource access (use [`WorldGuard::with_resource`] /
    /// [`WorldGuard::with_resource_mut`]).
    ///
    /// `addr` is the dispatch address — for `.Broadcast.<M>` calls this is
    /// [`EBusAddress::Broadcast`]; for `.Event.<M>(addr, ...)` calls it
    /// reflects the first Lua arg. Native handlers connected at
    /// `EBusAddress::Broadcast` receive every dispatch (matching all
    /// addresses) and use this parameter to decide which entity / canvas
    /// the call is targeting.
    ///
    /// # Errors
    ///
    /// Returns whatever [`mlua::Error`] the implementation raises while
    /// executing `method` — typically an argument-conversion failure from
    /// `args`, or an error propagated out of a Lua callback the handler
    /// invokes.
    fn dispatch(
        &self,
        world: &WorldGuard<'_>,
        lua: &Lua,
        addr: &EBusAddress,
        method: &str,
        args: &[Value],
    ) -> mlua::Result<Value>;
}

/// What a subscription dispatches into.
#[derive(Clone)]
enum HandlerKind {
    Native(Arc<dyn NativeBusHandler>),
    Lua { listener: Table },
}

#[derive(Clone)]
struct Subscription {
    bus: String,
    address: EBusAddress,
    handler: HandlerKind,
}

/// The registry resource itself. All public methods are `&self` — the
/// internal lock is granular enough to not block dispatch on connects.
#[derive(Resource, Default)]
pub struct EBusRegistry {
    inner: RwLock<EBusRegistryInner>,
}

#[derive(Default)]
struct EBusRegistryInner {
    next_id: u64,
    subs: HashMap<SubscriptionId, Subscription>,
    /// (bus, address) -> ordered subscription ids.
    by_address: HashMap<(String, EBusAddress), Vec<SubscriptionId>>,
}

impl EBusRegistry {
    /// Connect a native handler at `(bus, addr)`.
    pub fn connect_native(
        &self,
        bus: impl Into<String>,
        addr: EBusAddress,
        handler: Arc<dyn NativeBusHandler>,
    ) -> SubscriptionId {
        self.connect(bus.into(), addr, HandlerKind::Native(handler))
    }

    /// Connect a Lua listener at `(bus, addr)`. The `listener` table is
    /// retained for the subscription's lifetime; the Lua GC won't collect
    /// it while we hold the [`Table`] handle.
    pub fn connect_lua(
        &self,
        bus: impl Into<String>,
        addr: EBusAddress,
        listener: Table,
    ) -> SubscriptionId {
        self.connect(bus.into(), addr, HandlerKind::Lua { listener })
    }

    fn connect(&self, bus: String, addr: EBusAddress, handler: HandlerKind) -> SubscriptionId {
        let mut inner = self.inner.write();
        let id = SubscriptionId(inner.next_id);
        inner.next_id += 1;

        inner.subs.insert(
            id,
            Subscription {
                bus: bus.clone(),
                address: addr.clone(),
                handler,
            },
        );
        inner.by_address.entry((bus, addr)).or_default().push(id);
        id
    }

    /// Drop a subscription. Idempotent — disconnecting twice is a no-op.
    pub fn disconnect(&self, id: SubscriptionId) {
        let mut inner = self.inner.write();
        let Some(sub) = inner.subs.remove(&id) else {
            return;
        };
        if let Some(list) = inner
            .by_address
            .get_mut(&(sub.bus.clone(), sub.address.clone()))
        {
            list.retain(|x| *x != id);
            if list.is_empty() {
                inner.by_address.remove(&(sub.bus, sub.address));
            }
        }
    }

    /// Snapshot the subscriptions matching (bus, addr). `addr = None` is
    /// the `Broadcast` form (every address); `Some(addr)` is the
    /// `Event(addr)` form.
    ///
    /// For an addressed dispatch, returns handlers at exactly `addr` PLUS
    /// handlers at [`EBusAddress::Broadcast`] — the latter act as
    /// type-level wildcard listeners (the C++ pattern where a native
    /// component is reflected as the bus's default dispatcher and
    /// receives every event regardless of address). This matches
    /// Lumberyard's `EBusBroadcaster` semantics where the implementation
    /// table sits at the "no address" slot.
    ///
    /// Returned snapshot can be dispatched without holding the registry
    /// lock — important because handlers may re-enter the registry (e.g.
    /// a Lua handler that triggers another bus call).
    pub fn snapshot_for_dispatch(
        &self,
        bus: &str,
        addr: Option<&EBusAddress>,
    ) -> Vec<HandlerSnapshot> {
        let inner = self.inner.read();
        addr.map_or_else(
            || {
                inner
                    .by_address
                    .iter()
                    .filter(|((b, _), _)| b == bus)
                    .flat_map(|(_, ids)| collect_snapshots(&inner, ids))
                    .collect()
            },
            |addr| {
                let mut out = Vec::new();
                if let Some(ids) = inner.by_address.get(&(bus.to_string(), addr.clone())) {
                    out.extend(collect_snapshots(&inner, ids));
                }
                // Wildcard fallback: handlers at Broadcast accept any
                // addressed dispatch on this bus.
                if !matches!(addr, EBusAddress::Broadcast)
                    && let Some(ids) = inner
                        .by_address
                        .get(&(bus.to_string(), EBusAddress::Broadcast))
                {
                    out.extend(collect_snapshots(&inner, ids));
                }
                out
            },
        )
    }
}

fn collect_snapshots(inner: &EBusRegistryInner, ids: &[SubscriptionId]) -> Vec<HandlerSnapshot> {
    ids.iter()
        .filter_map(|id| {
            inner
                .subs
                .get(id)
                .map(|s| s.handler.clone().into_snapshot())
        })
        .collect()
}

/// One subscription's handler, lifted out of the registry for dispatch.
/// Dispatch the snapshot via [`HandlerSnapshot::invoke`] without holding
/// the registry lock.
#[derive(Clone)]
pub enum HandlerSnapshot {
    Native(Arc<dyn NativeBusHandler>),
    Lua(Table),
}

impl HandlerSnapshot {
    /// Invoke the handler. Returns `Some(value)` if the handler accepted
    /// the call, `None` if it didn't (no method match).
    ///
    /// # Errors
    ///
    /// Propagates the [`mlua::Error`] returned by
    /// [`NativeBusHandler::dispatch`] for a native handler, or by the Lua
    /// listener's own method call for a Lua handler. A listener that simply
    /// has no such method is not an error — it yields `Ok(None)`.
    pub fn invoke(
        &self,
        world: &WorldGuard<'_>,
        lua: &Lua,
        addr: &EBusAddress,
        method: &str,
        args: &[Value],
    ) -> mlua::Result<Option<Value>> {
        match self {
            Self::Native(h) => {
                if !h.handles(method) {
                    return Ok(None);
                }
                Ok(Some(h.dispatch(world, lua, addr, method, args)?))
            }
            Self::Lua(listener) => match listener.get::<Value>(method) {
                Ok(Value::Function(f)) => {
                    let mut call_args: Vec<Value> = Vec::with_capacity(args.len() + 1);
                    call_args.push(Value::Table(listener.clone()));
                    call_args.extend_from_slice(args);
                    Ok(Some(f.call::<Value>(mlua::Variadic::from_iter(call_args))?))
                }
                _ => Ok(None),
            },
        }
    }
}

impl HandlerKind {
    fn into_snapshot(self) -> HandlerSnapshot {
        match self {
            Self::Native(h) => HandlerSnapshot::Native(h),
            Self::Lua { listener } => HandlerSnapshot::Lua(listener),
        }
    }
}
