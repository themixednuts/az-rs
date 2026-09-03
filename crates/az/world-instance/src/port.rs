//! The async convention every consumer-owned effect port follows.
//!
//! Adapters are selected at executable composition, so effect ports must be
//! object safe. They return a named boxed future rather than hiding allocation
//! and object safety behind an async-trait macro. Concrete application services
//! in this crate stay ordinary `async fn`; only the seams where an external
//! effect happens are traits.
//!
//! A method that returns [`PortFuture`] is a *remote effect*: it leaves the
//! lifecycle control plane for a store, provider, ingress plane, or
//! distribution plane. No such method may be reachable from a fixed-tick or
//! replication schedule, which `tests/architecture.rs` enforces structurally.

use std::{future::Future, pin::Pin};

/// The result of one remote effect performed through a consumer-owned port.
pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
