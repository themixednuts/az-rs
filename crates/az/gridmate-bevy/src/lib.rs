//! Bevy integration crate for [`gridmate`].
//!
//! gridmate's transport, framing, and session-service are
//! framework-agnostic — they spawn through a [`gridmate::Spawner`]
//! abstraction and emit a plain [`gridmate::Event`] stream. This
//! crate is the Bevy-shaped wrapper:
//!
//! - [`GridMatePlugin`] bridges `gridmate::Event` into Bevy's
//!   messaging system as [`NetEvent`].
//! - With the `client` feature, [`GridMateSessionService`] makes the
//!   session-service handle a Bevy `Resource`.
//! - [`SessionConnectionRequest`] is a `Component` that, when
//!   spawned, kicks off a `create_session` call.
//!
//! Embedders using a non-Bevy runtime (tokio, smol, async-std) should
//! depend on `gridmate` directly and install their own
//! [`gridmate::Spawner`] via [`gridmate::set_spawner`].
//!
//! # Architectural mapping to Lumberyard's `GridMateImpl`
//!
//! The vendor C++ has a `class GridMateImpl : public IGridMate`
//! singleton that owns typed service slots, a `ServiceTable`,
//! per-frame `Update()`, etc. Our port replaces that with Bevy's
//! ECS plugin pattern: typed `Resource`s + scheduled systems. No
//! `GridMateImpl` struct is ported; [`GridMatePlugin`] is the
//! canonical owner of GridMate-level state in the Bevy world.

mod fragment_authority;
mod fragment_snapshot;
mod fragment_state;
mod state_bundle_sequence;

pub use fragment_authority::AuthoritativeFragment;
pub use fragment_snapshot::{FragmentReplicaKey, FragmentSnapshotStore};
pub use fragment_state::FragmentMergeError;
pub use state_bundle_sequence::ClientStateBundleSequence;

use az_gem_contract::Registries;
#[cfg(feature = "client")]
use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
#[cfg(feature = "client")]
use bevy::tasks::Task;
use gridmate::Event;
#[cfg(feature = "client")]
use gridmate::SessionServiceHandle;
#[cfg(feature = "client")]
use gridmate::az::TypeRegistry;
use gridmate::spawn::{BoxedFuture, Spawner};
#[cfg(feature = "client")]
use gridmate::{CarrierDesc, CarrierProtocolProfile};
use std::sync::Arc;
#[cfg(feature = "client")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "client")]
use tracing::trace;

/// [`Spawner`] adapter for Bevy's `IoTaskPool`.
///
/// Installed by [`GridMatePlugin::build`] before any gridmate handle is
/// constructed — non-Bevy embedders pick their own spawner (tokio, smol,
/// async-std) and call [`gridmate::set_spawner`] directly.
pub struct BevyIoTaskSpawner;

impl Spawner for BevyIoTaskSpawner {
    fn spawn(&self, future: BoxedFuture) {
        IoTaskPool::get().spawn(future).detach();
    }
}

/// `GridMate` networking plugin for Bevy.
///
/// Manages `GridMate` sessions as Bevy resources and bridges events
/// from the session service actor to Bevy's messaging system.
/// Supports unlimited concurrent connections through the actor-based
/// session service.
pub struct GridMatePlugin;

impl Plugin for GridMatePlugin {
    fn build(&self, app: &mut App) {
        logging_registry::register!();

        // Install the Bevy `IoTaskPool` spawner exactly once.
        // `set_spawner` returns `Err` if some other plugin /
        // embedder already registered one — we treat that as
        // intentional and skip silently.
        let _ = gridmate::set_spawner(Arc::new(BevyIoTaskSpawner));

        app.add_message::<NetEvent>();

        #[cfg(feature = "client")]
        app.init_resource::<GridMateSessionService>()
            .init_resource::<GridMateEventQueue>()
            .init_resource::<GridMateEventTask>()
            .add_systems(Startup, init_session_service)
            .add_systems(PostStartup, start_gridmate_event_task)
            .add_systems(Update, (create_session_on_request, drain_event_queue))
            .add_systems(Last, gridmate_shutdown_system);
    }
}

/// Component marking an entity that should create a `GridMate` session.
#[cfg(feature = "client")]
#[derive(Component, Debug)]
pub struct SessionConnectionRequest {
    /// Server address (host:port).
    pub server: String,
    /// Optional path to CA certificate file.
    pub cert_path: Option<String>,
    /// Optional path to SSL keylog file (for debugging).
    pub ssl_keylog: Option<String>,
    /// Carrier wire profile selected by the project/runtime.
    pub protocol_profile: CarrierProtocolProfile,
}

#[cfg(feature = "client")]
impl SessionConnectionRequest {
    /// Create a connection request with the framework default carrier
    /// profile. Projects targeting a compatibility protocol should set
    /// [`Self::protocol_profile`] explicitly.
    pub fn new(server: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            cert_path: None,
            ssl_keylog: None,
            protocol_profile: CarrierProtocolProfile::default(),
        }
    }
}

/// Creates a `GridMate` session when a `SessionConnectionRequest`
/// component is added.
#[cfg(feature = "client")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn create_session_on_request(
    session_service: Res<GridMateSessionService>,
    connection_requests: Query<&SessionConnectionRequest, Added<SessionConnectionRequest>>,
) {
    for request in connection_requests.iter() {
        let desc = build_carrier_desc(request);
        let Some(handle) = session_service.handle.clone() else {
            return;
        };

        IoTaskPool::get()
            .spawn(async move {
                match handle.create_session(desc).await {
                    Ok(index) => {
                        trace!("Created GridMate session {}", index);
                    }
                    Err(e) => {
                        trace!("Failed to create GridMate session: {}", e);
                    }
                }
            })
            .detach();
    }
}

/// Builds a `CarrierDesc` from a connection request.
#[cfg(feature = "client")]
fn build_carrier_desc(request: &SessionConnectionRequest) -> CarrierDesc {
    let mut desc = CarrierDesc::new(&request.server);
    desc.with_protocol_profile(request.protocol_profile);

    if let Some(cert) = &request.cert_path
        && !cert.is_empty()
    {
        desc.with_ca_cert(cert.clone());
    }

    if let Some(keylog) = &request.ssl_keylog
        && !keylog.is_empty()
    {
        desc.with_ssl_keylog(std::path::PathBuf::from(keylog));
    }

    desc
}

/// The wire registries this host was composed with.
///
/// The session actor runs detached and decodes for as long as the process
/// lives, so it borrows the composition rather than owning a copy — the host
/// that composed it keeps it alive (a `OnceLock`, as `az-asset-processor`
/// does). Nothing here reaches for a process-global registry.
#[derive(Resource, Clone, Copy)]
pub struct Composition(pub &'static Registries);

/// Wrapper for `SessionServiceHandle` as a Bevy resource.
///
/// The handle uses an actor pattern internally, providing thread-safe
/// access to session management operations without requiring mutex
/// locks.
#[cfg(feature = "client")]
#[derive(Resource, Default)]
pub struct GridMateSessionService {
    pub handle: Option<SessionServiceHandle>,
}

/// Initialize `SessionServiceHandle` after Bevy task pools are ready.
///
/// Also walks the composed class registry once so we get a startup line for
/// descriptor registration and debug-only typeregistry validation.
///
/// # Panics
///
/// Panics when no [`Composition`] was inserted, or when it carries no classes.
/// A client whose type registry is empty decodes nothing and reports every
/// inbound envelope as an unknown type — silently. Refusing to start is the
/// only honest reading of that state.
#[cfg(feature = "client")]
fn init_session_service(
    mut session_service: ResMut<GridMateSessionService>,
    composition: Option<Res<Composition>>,
) {
    if session_service.handle.is_some() {
        return;
    }
    let composition = composition.expect(
        "gridmate: no Composition resource — the host must insert its composed \
         Registries before GridMatePlugin starts",
    );
    let types = TypeRegistry::of(composition.0).expect(
        "gridmate: the composed host registered no gridmate classes — wire decode \
         would reject every inbound envelope",
    );

    let diag = types.collect_diagnostics();
    trace!(
        "TypeRegistry: {} class registration(s) walked, \
         {} missing typeIndex, {} duplicate typeIndex, {} typeIndex mismatch(es), \
         {} unknown to debug table",
        diag.checked,
        diag.missing_type_index.len(),
        diag.duplicate_type_index.len(),
        diag.type_index_mismatches.len(),
        diag.unknown_to_native_debug.len(),
    );
    session_service.handle = Some(SessionServiceHandle::new(types));
}

/// Bevy message wrapper for `GridMate` network events.
///
/// Thin wrapper around `gridmate::Event` for Bevy's messaging system.
/// Events are bridged directly from the session service actor with
/// zero-copy semantics. The `Bytes` type uses `Arc` internally, so
/// when Bevy clones messages for broadcasting, only the reference
/// counter is incremented.
#[derive(Debug, Clone, Message)]
pub struct NetEvent(pub Event);

#[cfg(feature = "client")]
const EVENT_QUEUE_CAPACITY: usize = 4096;
#[cfg(feature = "client")]
const MAX_EVENTS_PER_FRAME: usize = 512;

#[cfg(feature = "client")]
#[derive(Resource)]
struct GridMateEventQueue {
    sender: async_channel::Sender<Event>,
    receiver: async_channel::Receiver<Event>,
    pending: Arc<AtomicBool>,
}

#[cfg(feature = "client")]
impl Default for GridMateEventQueue {
    fn default() -> Self {
        let (sender, receiver) = async_channel::bounded(EVENT_QUEUE_CAPACITY);
        Self {
            sender,
            receiver,
            pending: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Long-lived task handle for the `GridMate` event bridge.
#[cfg(feature = "client")]
#[derive(Default, Resource)]
struct GridMateEventTask {
    /// Held only so the spawned bridge task stays alive; dropping the
    /// [`Task`] would cancel it. Never read back.
    task: Option<Arc<Task<()>>>,
}

/// Starts the long-lived `GridMate` event bridge task.
///
/// This task waits on the actor's event channel and forwards events
/// into Bevy without polling or sleeps.
#[cfg(feature = "client")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn start_gridmate_event_task(
    session_service: Res<GridMateSessionService>,
    event_queue: Res<GridMateEventQueue>,
    mut event_task: ResMut<GridMateEventTask>,
) {
    let Some(handle) = session_service.handle.clone() else {
        trace!("GridMate event task: handle not available!");
        return;
    };

    trace!("Starting GridMate event task");

    let sender = event_queue.sender.clone();
    let pending = event_queue.pending.clone();
    let task = IoTaskPool::get().spawn(async move {
        while let Some(event) = handle.recv_event().await {
            if sender.send(event).await.is_ok() {
                pending.store(true, Ordering::Release);
            }
        }
    });

    event_task.task = Some(Arc::new(task));
}

#[cfg(feature = "client")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn drain_event_queue(event_queue: Res<GridMateEventQueue>, mut writer: MessageWriter<NetEvent>) {
    if !event_queue.pending.swap(false, Ordering::Acquire) {
        return;
    }

    let mut drained = 0usize;
    while drained < MAX_EVENTS_PER_FRAME {
        match event_queue.receiver.try_recv() {
            Ok(event) => {
                writer.write(NetEvent(event));
                drained += 1;
            }
            Err(async_channel::TryRecvError::Empty | async_channel::TryRecvError::Closed) => break,
        }
    }

    if !event_queue.receiver.is_empty() {
        event_queue.pending.store(true, Ordering::Release);
    }
}

/// Handles graceful shutdown of all `GridMate` sessions.
///
/// Listens for `AppExit` messages and disconnects all sessions
/// cleanly before the application terminates. Ensures proper cleanup
/// of network resources.
#[cfg(feature = "client")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn gridmate_shutdown_system(
    session_service: Res<GridMateSessionService>,
    mut exit_reader: MessageReader<AppExit>,
    mut disconnected: Local<bool>,
) {
    if *disconnected {
        return;
    }

    if exit_reader.read().next().is_some() {
        *disconnected = true;
        trace!("Initiating graceful shutdown of all GridMate sessions");

        let Some(handle) = session_service.handle.clone() else {
            return;
        };

        IoTaskPool::get()
            .spawn(async move {
                if let Err(e) = handle.disconnect_all().await {
                    trace!("Error during graceful shutdown: {}", e);
                } else {
                    let count = handle.num_sessions().await;
                    trace!("Gracefully disconnected {} session(s)", count);
                }
            })
            .detach();
    }
}
