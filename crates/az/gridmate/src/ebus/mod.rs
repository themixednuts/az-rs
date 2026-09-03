//! Event dispatch with O3DE `AZ::EBus` addressing and handler semantics.
//!
//! A general-purpose pub/sub system for dispatching events and handling requests.
//! Components connect to buses as handlers and receive events when they're broadcast
//! or addressed to their specific ID.
//!
//! # Architecture
//!
//! O3DE reference: `Code/Framework/AzCore/AzCore/EBus/EBus.h`.
//!
//! ```text
//! EBus<Interface, Traits>
//!   ├── AddressPolicy: Single (global) or ById (per entity)
//!   ├── HandlerPolicy: Single or Multiple handlers per address
//!   ├── Handler: connects/disconnects, receives events
//!   ├── Broadcast(): send to all handlers
//!   └── Event(id): send to handlers at a specific address
//! ```
//!
//! # Rust Design:
//!
//! Instead of C++ template metaprogramming, we use traits + type maps:
//! - `EBusInterface` trait defines the event methods
//! - `EBusHandler` trait for types that handle events
//! - `EBusRegistry` manages connections and dispatch
//! - `EBusAddress` enum for Single vs `ById` addressing

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Address policy: how events are routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressPolicy {
    /// Single global address — all handlers receive all events.
    Single,
    /// Addressed by ID — events are sent to handlers at a specific address.
    ById,
}

/// Handler policy: how many handlers per address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerPolicy {
    /// Only one handler per address.
    Single,
    /// Multiple handlers per address.
    Multiple,
}

/// An `EBus` address — either global or a specific entity ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusAddress {
    /// Global broadcast address.
    Global,
    /// Specific entity address.
    Entity(u64),
}

/// Trait for an `EBus` interface. Each bus defines its event signature.
///
/// This corresponds to O3DE's `Interface` template parameter of
/// `EBus<Interface>`.
pub trait EBusInterface: 'static {
    /// The address policy for this bus.
    const ADDRESS_POLICY: AddressPolicy = AddressPolicy::Single;
    /// The handler policy for this bus.
    const HANDLER_POLICY: HandlerPolicy = HandlerPolicy::Multiple;
    /// The event data type carried by this bus.
    type Event: Send + 'static;
}

/// Trait for a handler that can connect to an `EBus` and receive events.
pub trait EBusHandler<Bus: EBusInterface>: Send + 'static {
    /// Handle an event from the bus.
    fn on_event(&mut self, event: &Bus::Event);
}

/// Type-erased handler wrapper.
type DispatchFn = Box<dyn Fn(&mut dyn Any, &dyn Any) + Send>;

struct AnyHandler {
    handler: Box<dyn Any + Send>,
    dispatch: DispatchFn,
}

/// Registry entry for a single bus.
#[derive(Default)]
struct BusEntry {
    /// Handlers indexed by address.
    handlers: HashMap<BusAddress, Vec<AnyHandler>>,
}

/// Central `EBus` registry — manages all bus connections and dispatches events.
///
/// O3DE manages this role through `EBusEnvironment`; Azoth keeps it in the
/// session or application context.
pub struct EBusRegistry {
    buses: HashMap<TypeId, BusEntry>,
}

impl EBusRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            buses: HashMap::new(),
        }
    }

    /// Connect a handler to a bus at the given address.
    pub fn connect<Bus, H>(&mut self, address: BusAddress, handler: H)
    where
        Bus: EBusInterface,
        H: EBusHandler<Bus>,
    {
        let bus_id = TypeId::of::<Bus>();
        let entry = self.buses.entry(bus_id).or_default();

        let dispatch: DispatchFn = Box::new(|handler_any: &mut dyn Any, event_any: &dyn Any| {
            if let Some(handler) = handler_any.downcast_mut::<H>()
                && let Some(event) = event_any.downcast_ref::<Bus::Event>()
            {
                handler.on_event(event);
            }
        });

        entry.handlers.entry(address).or_default().push(AnyHandler {
            handler: Box::new(handler),
            dispatch,
        });
    }

    /// Broadcast an event to all handlers on a bus (Single address policy).
    pub fn broadcast<Bus: EBusInterface>(&mut self, event: &Bus::Event) {
        let bus_id = TypeId::of::<Bus>();
        if let Some(entry) = self.buses.get_mut(&bus_id) {
            for handlers in entry.handlers.values_mut() {
                for any_handler in handlers.iter_mut() {
                    (any_handler.dispatch)(any_handler.handler.as_mut(), event);
                }
            }
        }
    }

    /// Send an event to handlers at a specific address (`ById` address policy).
    pub fn event<Bus: EBusInterface>(&mut self, address: BusAddress, event: &Bus::Event) {
        let bus_id = TypeId::of::<Bus>();
        if let Some(entry) = self.buses.get_mut(&bus_id)
            && let Some(handlers) = entry.handlers.get_mut(&address)
        {
            for any_handler in handlers.iter_mut() {
                (any_handler.dispatch)(any_handler.handler.as_mut(), event);
            }
        }
    }
}

impl Default for EBusRegistry {
    fn default() -> Self {
        Self::new()
    }
}
