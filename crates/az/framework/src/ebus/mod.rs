//! AZ/Lumberyard `EBus` runtime.
//!
//! Lumberyard reference: `Code/Framework/AzCore/AzCore/EBus/EBus.h` (the
//! C++ template-heavy core) plus
//! `AzCore/Script/ScriptContext.cpp::BindEBus` for the Lua-side shape we
//! mirror. The latter eagerly populates `Bus.Broadcast.<M>` and
//! `Bus.Event.<M>` tables from a `BehaviorContext::EBus<T>` reflection
//! at `ScriptContext::BindTo`; we synthesise the same shape lazily via
//! `__index` metatables in [`crate::lua::bus`].
//!
//! ## Architecture
//!
//! - [`address::EBusAddress`] — bus key (`Broadcast | Entity | Name | Integer`).
//! - [`registry::EBusRegistry`] — Bevy `Resource` with `(bus, address)
//!   -> [SubscriptionId]` plus per-id storage.
//! - [`registry::NativeBusHandler`] — trait native handlers implement;
//!   gets the world (via `WorldGuard`) + Lua state at dispatch time.
//! - [`registry::HandlerSnapshot`] — a single subscription lifted out of
//!   the registry for lock-free dispatch.
//!
//! ## Snapshot-then-dispatch
//!
//! `EBusRegistry::snapshot_for_dispatch` returns a `Vec<HandlerSnapshot>`
//! while holding only a brief read lock. Dispatch happens after the lock
//! is dropped — handlers can re-enter the registry (a Lua handler firing
//! another bus call) without deadlocking, and native handlers can freely
//! call `WorldGuard::with_resource_mut`.

pub mod address;
pub mod call;
#[cfg(any(feature = "lua", feature = "lua54", feature = "luajit"))]
pub mod registry;

pub use address::EBusAddress;
pub use call::{DYNAMIC_BUS_PREFIX, EBusArg, EBusCall};
#[cfg(any(feature = "lua", feature = "lua54", feature = "luajit"))]
pub use registry::{EBusRegistry, HandlerSnapshot, NativeBusHandler, SubscriptionId};
