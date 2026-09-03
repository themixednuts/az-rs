//! Executor abstraction for gridmate's async tasks.
//!
//! gridmate spawns long-running tasks for every per-peer carrier
//! driver, the DTLS demux loop, the multi-peer event bridge, the
//! per-session translator, and the session-service actor. Every
//! one of those routes through [`spawn_detached`]; gridmate never
//! touches a concrete executor directly. That keeps the core crate
//! framework-agnostic.
//!
//! [`Spawner`] is the seam. Embedders register one of:
//!
//! - `gridmate_bevy::BevyIoTaskSpawner` (Bevy's `IoTaskPool`) — used
//!   by an embedding server or client via the
//!   `GridMatePlugin`, which installs the spawner during `build()`.
//! - A custom tokio/smol/async-std spawner — embedders implement
//!   the trait and call [`set_spawner`] once at startup.
//!
//! [`spawn_detached`] panics if no spawner has been installed and
//! no default is compiled in. The Bevy plugin handles registration
//! transparently; non-Bevy embedders must call [`set_spawner`]
//! before constructing any gridmate handle.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

/// Type-erased detached future. Boxed because the spawner is dynamic.
pub type BoxedFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Spawn detached async tasks on an executor of the embedder's
/// choice. Trait-object friendly: gridmate stores an
/// `Arc<dyn Spawner>` in a process-global slot.
pub trait Spawner: Send + Sync + 'static {
    /// Spawn `future`, detached. The spawner is responsible for
    /// keeping the task alive until completion — gridmate never
    /// holds a join handle.
    fn spawn(&self, future: BoxedFuture);
}

static SPAWNER: OnceLock<Arc<dyn Spawner>> = OnceLock::new();

/// Install the process-global [`Spawner`].
///
/// Must be called before any gridmate handle is constructed. The
/// in-tree `gridmate-bevy::GridMatePlugin` calls this during
/// `build()`; non-Bevy embedders call it once at startup.
///
/// # Errors
///
/// Returns `Err(spawner)`, handing back the rejected value, if a
/// spawner was already installed — the slot is a [`OnceLock`] and
/// only the first caller wins.
pub fn set_spawner(spawner: Arc<dyn Spawner>) -> Result<(), Arc<dyn Spawner>> {
    SPAWNER.set(spawner)
}

/// Convenience: the canonical entry point every gridmate-internal
/// task spawn must route through. Accepts any `Send + 'static`
/// future, boxes it, and hands it to the registered spawner.
///
/// # Panics
///
/// Panics if no spawner has been installed via [`set_spawner`] —
/// this is a startup-ordering bug, not a runtime error mode. Bevy
/// consumers don't see it because `GridMatePlugin::build()` installs
/// `BevyIoTaskSpawner` before any gridmate handle is constructed.
pub fn spawn_detached<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    let spawner = SPAWNER.get().expect(
        "no gridmate Spawner installed — call gridmate::set_spawner before \
         creating any gridmate handle (Bevy consumers: add gridmate_bevy::GridMatePlugin)",
    );
    spawner.spawn(Box::pin(future));
}

/// One-shot "parent dropped" handshake for detached tasks.
///
/// gridmate's per-listener loops (DTLS demux, multi-peer bridge)
/// can't observe their parent dropping directly — they hold their own
/// `Arc`-shaped state and `recv_from` blocks until a datagram
/// arrives. The pattern is the same in every case:
///
/// - parent owns a `Sender<()>` whose Drop is the exit signal,
/// - spawned task selects on its real work and `Receiver<()>::recv`,
///   returning when `recv` errors with the closed-channel state.
///
/// `ShutdownSignal` names that handshake. The parent holds the
/// `ShutdownSignal` (its Drop closes the channel); the task takes the
/// `Receiver<()>` returned by [`ShutdownSignal::new`].
///
/// ```ignore
/// let (signal, rx) = ShutdownSignal::new();
/// spawn_detached(async move {
///     loop {
///         let step = futures_lite::future::or(
///             work(),
///             async { let _ = rx.recv().await; Step::Shutdown },
///         ).await;
///         if let Step::Shutdown = step { break }
///     }
/// });
/// // signal lives on the parent; dropping it ends the task.
/// ```
pub struct ShutdownSignal {
    _tx: async_channel::Sender<()>,
}

impl ShutdownSignal {
    /// Create a new handshake. Returns the parent-owned signal and
    /// the task-owned receiver. The signal's Drop closes the channel;
    /// the task observes the close on its `recv` branch.
    #[must_use]
    pub fn new() -> (Self, async_channel::Receiver<()>) {
        let (tx, rx) = async_channel::bounded(1);
        (Self { _tx: tx }, rx)
    }
}
