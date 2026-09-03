//! `SessionService` - Top-level session manager
//!
//! Implements an actor pattern where the actor owns session state and communicates
//! via message passing. Based on GridMate/Session/Session.h `SessionService` class.

#[cfg(feature = "client")]
use super::carrier::CarrierConnecting;
#[cfg(feature = "client")]
use super::session::{Connecting, GridSession};
use crate::Result;
#[cfg(feature = "client")]
use crate::az::TypeRegistry;
use crate::message::{ClientToServer, EgressPath, Sendable};
use crate::serialize::{WriteBuffer, buffer::CARRIER_ENDIAN};
pub use crate::session_types::{Event, SessionId};
#[cfg(feature = "client")]
use async_channel::{Receiver, Sender, bounded};
use bytes::Bytes;
#[cfg(feature = "client")]
use std::collections::HashSet;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
#[cfg(feature = "client")]
use std::sync::Arc;
#[cfg(feature = "client")]
use tracing::trace;

/// Messages sent to the `SessionService` actor
#[cfg(feature = "client")]
pub enum SessionServiceMessage {
    /// Create a new session
    CreateSession {
        desc: Box<super::carrier::CarrierDesc>,
        respond_to: Sender<Result<SessionId>>,
    },
    /// Send a message on a session
    SendMessage {
        session_id: SessionId,
        data: bytes::Bytes,
        channel: u8,
        reliability: super::carrier::DataReliability,
        priority: usize,
        respond_to: Sender<Result<()>>,
    },
    /// Get number of sessions
    GetNumSessions { respond_to: Sender<usize> },
    /// Disconnect all sessions
    DisconnectAll { respond_to: Sender<Result<()>> },
    /// Shutdown the actor
    Shutdown { respond_to: Sender<()> },
}

/// Per-session inbox item — pairs a carrier-level event with the
/// session it belongs to. Used internally by the per-session task to
/// tag every `CarrierEvent` with its `SessionId` before the
/// actor's multiplexed select loop picks it up.
#[cfg(feature = "client")]
type SessionCarrierEvent = (SessionId, super::carrier::thread_message::CarrierEvent);

/// Internal actor that owns session state and processes messages.
///
/// UUID → `type_index` resolution for inbound `IMessage` envelopes goes through
/// the composed class registry the handle was built with. The actor runs
/// detached, so it borrows a composition that outlives the process rather than
/// reaching for a process-global one.
#[cfg(feature = "client")]
struct SessionServiceActor {
    types: TypeRegistry<'static>,
    receiver: Receiver<SessionServiceMessage>,
    sessions: Vec<GridSession<Connecting, CarrierConnecting>>,
    event_tx: Sender<Event>,
    session_event_tx: Sender<SessionCarrierEvent>,
    session_event_rx: Receiver<SessionCarrierEvent>,
    /// Per-session translator tasks are detached; they self-terminate
    /// when the carrier event receiver closes. The set is kept only
    /// to mirror live sessions for diagnostics — membership is the
    /// whole payload, so it is a set rather than a map to `()`.
    session_tasks: HashSet<SessionId>,
    /// Wire-capture sidecar. No-op in release builds (the type is
    /// `()` then) and configured explicitly even in debug builds.
    /// Lives outside the actor's core
    /// responsibilities — capture is a tap, not orchestration.
    #[cfg(debug_assertions)]
    capture: crate::capture::CaptureCounters,
    /// Track which sessions have sent ready events using `HashSet` for
    /// efficiency — better than `Vec<bool>` for sparse indices and many
    /// sessions.
    ready_events_sent: HashSet<SessionId>,
}

#[cfg(feature = "client")]
impl SessionServiceActor {
    async fn run(mut self) {
        use futures_util::future::{Either, select};

        loop {
            let recv_cmd = self.receiver.recv();
            let recv_evt = self.session_event_rx.recv();
            futures_util::pin_mut!(recv_cmd);
            futures_util::pin_mut!(recv_evt);

            match select(recv_cmd, recv_evt).await {
                Either::Left((msg_result, _pending_event)) => {
                    let Ok(msg) = msg_result else { break };
                    self.handle_message(msg).await;
                }
                Either::Right((event_result, _pending_msg)) => {
                    let Ok(event) = event_result else { break };
                    self.handle_carrier_event(event).await;
                }
            }
        }
    }

    async fn handle_message(&mut self, msg: SessionServiceMessage) {
        match msg {
            SessionServiceMessage::CreateSession { desc, respond_to } => {
                let result = self.handle_create_session(*desc).await;
                let _ = respond_to.send(result).await;
            }
            SessionServiceMessage::SendMessage {
                session_id,
                data,
                channel,
                reliability,
                priority,
                respond_to,
            } => {
                let result = self
                    .handle_send_message(session_id, data, channel, reliability, priority)
                    .await;
                let _ = respond_to.send(result).await;
            }
            SessionServiceMessage::GetNumSessions { respond_to } => {
                let _ = respond_to.send(self.sessions.len()).await;
            }
            SessionServiceMessage::DisconnectAll { respond_to } => {
                self.handle_disconnect_all();
                let _ = respond_to.send(Ok(())).await;
            }
            SessionServiceMessage::Shutdown { respond_to } => {
                self.handle_disconnect_all();
                let _ = respond_to.send(()).await;
            }
        }
    }

    async fn handle_carrier_event(&mut self, event: SessionCarrierEvent) {
        use super::carrier::thread_message::CarrierEvent;
        use super::session::CarrierChannel;

        let (session_id, msg) = event;
        match msg {
            CarrierEvent::Connected { version } => {
                trace!("Session {:?} connected (v{})", session_id, version);
                self.send_ready_if_needed(session_id).await;
            }
            CarrierEvent::Disconnected { reason } => {
                trace!("Session {:?} disconnected: {:?}", session_id, reason);
                let _ = self
                    .event_tx
                    .send(Event::Disconnected {
                        session: session_id,
                        reason: format!("{reason:?}"),
                    })
                    .await;
            }
            CarrierEvent::Error { description } => {
                trace!("Session {:?} carrier error: {}", session_id, description);
                let _ = self.event_tx.send(Event::Error { description }).await;
            }
            CarrierEvent::MessageReceived { channel, data } => {
                self.send_ready_if_needed(session_id).await;

                let is_typed_channel =
                    CarrierChannel::from_u8(channel).is_some_and(CarrierChannel::is_application);
                if is_typed_channel {
                    if self.sessions.get(*session_id).is_some() {
                        for event in self.dispatch_messages(session_id, channel, &data) {
                            let _ = self.event_tx.send(event).await;
                        }
                    } else {
                        trace!("Dropped message for missing session {:?}", session_id);
                    }
                } else {
                    let _ = self
                        .event_tx
                        .send(Event::Received {
                            session: session_id,
                            channel,
                            data,
                        })
                        .await;
                }
            }
        }
    }

    async fn send_ready_if_needed(&mut self, session_id: SessionId) {
        if self.ready_events_sent.contains(&session_id) {
            return;
        }

        trace!("Session {:?} ready", session_id);
        if let Err(e) = self
            .event_tx
            .send(Event::Ready {
                session: session_id,
            })
            .await
        {
            trace!("Failed to send SessionReady event: {}", e);
        }
        self.ready_events_sent.insert(session_id);
    }

    async fn handle_create_session(
        &mut self,
        desc: super::carrier::CarrierDesc,
    ) -> Result<SessionId> {
        let mut session = GridSession::join(desc).await?;
        let index = self.sessions.len();
        let session_id = SessionId::new(index);

        let carrier = session
            .carrier_mut()
            .ok_or_else(|| crate::GridMateError::InvalidState("Carrier missing".into()))?;
        let Some(rx) = carrier.take_event_receiver() else {
            return Err(crate::GridMateError::InvalidState(
                "Carrier receiver unavailable".into(),
            ));
        };

        let event_tx = self.session_event_tx.clone();
        crate::spawn::spawn_detached(async move {
            use super::carrier::thread_message::CarrierEvent;
            // Forward every carrier event tagged with this task's
            // session id. The actor's multiplexed select loop picks
            // them up alongside command-channel traffic.
            while let Ok(msg) = rx.recv().await {
                if event_tx.send((session_id, msg)).await.is_err() {
                    break;
                }
            }
            // Channel closed unexpectedly — synthesise a disconnect
            // so the actor cleans up bookkeeping.
            let _ = event_tx
                .send((
                    session_id,
                    CarrierEvent::Disconnected {
                        reason: super::carrier::DisconnectReason::ShuttingDown,
                    },
                ))
                .await;
        });

        self.session_tasks.insert(session_id);
        self.sessions.push(session);
        trace!("Created GridMate session with id {:?}", session_id);
        Ok(session_id)
    }

    async fn handle_send_message(
        &mut self,
        session_id: SessionId,
        data: bytes::Bytes,
        channel: u8,
        reliability: super::carrier::DataReliability,
        priority: usize,
    ) -> Result<()> {
        let type_index = self.parse_send_type_index(data.as_ref());
        #[cfg(debug_assertions)]
        self.capture.capture_send(
            session_id,
            &data,
            channel,
            reliability,
            priority,
            type_index,
        );

        let _ = self
            .event_tx
            .send(Event::Sent {
                session: session_id,
                data: data.clone(),
                channel,
                reliability,
                priority,
                type_index,
            })
            .await;

        if let Some(session) = self.sessions.get_mut(*session_id) {
            // `data` is already `Bytes` — passes through zero-copy.
            session.send(channel, data, reliability, priority).await
        } else {
            Err(crate::GridMateError::InvalidState(format!(
                "Session {session_id:?} not found"
            )))
        }
    }

    fn parse_send_type_index(&self, data: &[u8]) -> Option<u32> {
        use crate::serialize::buffer::{CARRIER_ENDIAN, ReadBuffer};
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, data).with_types(self.types);
        let (_metadata, envelope) = crate::message::read_correlated_message(&mut rb).ok()?;
        match envelope.type_id {
            crate::message::MessageTypeId::TypeIndex(type_index) => Some(type_index),
            crate::message::MessageTypeId::Uuid(uuid) => self.types.type_index_of(&uuid),
        }
    }

    /// Clear every session's carrier. Dropping each one is what performs
    /// the graceful disconnect, so there is nothing here that can fail —
    /// the `Result` the caller reports back to `disconnect_all` is
    /// synthesised at the reply, not produced here.
    fn handle_disconnect_all(&mut self) {
        // Disconnect all sessions by clearing carriers
        // Disconnect will happen via Drop when carriers are dropped
        for session in &mut self.sessions {
            // Clear carrier - Drop will handle disconnect
            // Take carrier via inner_mut - will trigger Drop for graceful shutdown
            let _ = session.inner_mut().carrier.take();
        }
    }

    /// Dispatch messages from a buffer containing multiple concatenated messages.
    ///
    /// Wire format recovered from the current native implementation:
    /// ```text
    /// [vlq_body_size_1][message_1][vlq_body_size_2][message_2]...
    /// ```
    ///
    /// Each message envelope:
    /// ```text
    /// [outer_flags: 1 byte]     - bit 0: has field1, bit 1: has field2
    /// [field1: 8 bytes]         - optional, if outer_flags & 1
    /// [field2: 8 bytes]         - optional, if outer_flags & 2
    /// [envelope_flags: 1 byte]  - 0=empty, 1=has message, >=2 error
    /// [vlq_type_index: var]     - 0=UUID follows, non-zero=direct type index
    /// [uuid: 16 bytes]          - only if vlq_type_index == 0
    /// [body: remaining]         - message payload
    /// ```
    fn dispatch_messages(
        &mut self,
        session_id: SessionId,
        channel: u8,
        data: &Bytes,
    ) -> Vec<Event> {
        use super::serialize::buffer::{CARRIER_ENDIAN, ReadBuffer};

        let mut events = Vec::new();

        if data.is_empty() {
            return events;
        }

        trace!(
            "CARRIER_RECV: total_size={} hex={}",
            data.len(),
            hex::encode(data.as_ref())
        );

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, data.as_ref()).with_types(self.types);

        // Decode concatenated envelopes until no data remains.
        while !rb.is_empty() {
            match crate::message::read_sized_message(&mut rb) {
                Ok((metadata, view)) => {
                    trace!("Message body_size: {}", metadata.envelope_size);
                    if let Some(event) = self.event_from_message_envelope(session_id, channel, view)
                    {
                        events.push(event);
                    }
                }
                Err(crate::serialize::error::MarshalerError::EmptyEnvelope) => {
                    trace!("Empty envelope (flags=0)");
                }
                Err(err) => {
                    trace!("Invalid message stream item: {}", err);
                    break;
                }
            }
        }

        trace!("Dispatched {} messages from buffer", events.len());
        events
    }

    /// Convert a parsed message envelope into a typed event.
    fn event_from_message_envelope(
        &mut self,
        session_id: SessionId,
        channel: u8,
        envelope: crate::message::MessageEnvelopeView<'_>,
    ) -> Option<Event> {
        let type_registry = self.types;

        trace!("Outer flags: 0x{:02x}", envelope.outer_flags);

        let type_index = match envelope.type_id {
            crate::message::MessageTypeId::TypeIndex(type_index) => type_index,
            crate::message::MessageTypeId::Uuid(uuid) => {
                trace!("Full UUID bytes: {:02x?}", uuid.as_bytes());
                let Some(type_index) = type_registry.type_index_of(&uuid) else {
                    trace!("Unknown UUID: {:02x?}", uuid.as_bytes());
                    return None;
                };
                type_index
            }
        };

        let message_data = if envelope.body.is_empty() {
            Bytes::new()
        } else {
            Bytes::copy_from_slice(envelope.body)
        };

        trace!(
            "Parsed message: type_index={}, payload_size={}",
            type_index,
            message_data.len()
        );

        #[cfg(debug_assertions)]
        self.capture
            .capture_recv(session_id, type_index, envelope.raw, &message_data);

        Some(Event::TypedReceived {
            session: session_id,
            channel,
            type_index,
            correlation_id: None,
            data: message_data,
        })
    }
}

/// Handle for communicating with `SessionService` actor
///
/// The handle is Clone and communicates with the actor via message channels.
#[derive(Clone)]
#[cfg(feature = "client")]
pub struct SessionServiceHandle {
    sender: Sender<SessionServiceMessage>,
    event_rx: Receiver<Event>,
    /// Zero-sized ref counter used solely to detect the last handle
    /// drop — at that point the Drop impl signals graceful shutdown.
    /// The actor task itself runs detached on the embedder-registered
    /// spawner.
    handle_ref: Arc<()>,
    shutdown_complete: Receiver<()>,
}

#[cfg(feature = "client")]
impl SessionServiceHandle {
    /// Create new `SessionService` actor and handle.
    ///
    /// `types` is the host's composed class registry: UUID → `type_index`
    /// resolution for inbound envelopes reads it, and every buffer the actor
    /// decodes is bound to it. The actor task is detached, so the composition
    /// has to outlive it.
    #[must_use]
    pub fn new(types: TypeRegistry<'static>) -> Self {
        // Sized for client-side: typically O(1) sessions, but events
        // can burst (e.g., dispatching many TypedReceived from a
        // single inbound buffer). Bounded so a slow drain
        // backpressures rather than growing memory.
        let (sender, receiver) = bounded(256);
        let (event_tx, event_rx) = bounded(4096);
        let (shutdown_tx, shutdown_rx) = bounded(1);
        let (session_event_tx, session_event_rx) = bounded(2048);

        let actor = SessionServiceActor {
            types,
            receiver,
            sessions: Vec::new(),
            event_tx,
            session_event_tx,
            session_event_rx,
            session_tasks: HashSet::new(),
            #[cfg(debug_assertions)]
            capture: crate::capture::CaptureCounters::default(),
            ready_events_sent: HashSet::new(),
        };

        crate::spawn::spawn_detached(async move {
            actor.run().await;
            let _ = shutdown_tx.send(()).await;
        });

        Self {
            sender,
            event_rx,
            handle_ref: Arc::new(()),
            shutdown_complete: shutdown_rx,
        }
    }

    /// Create a new session
    ///
    /// # Errors
    ///
    /// Returns [`GridMateError::ConnectionClosed`](crate::GridMateError::ConnectionClosed)
    /// if the actor task has exited — either the command channel or the
    /// one-shot reply channel is closed. Otherwise returns whatever the
    /// actor's session creation produced, which is
    /// [`GridMateError::Driver`](crate::GridMateError::Driver) when the
    /// carrier cannot reach `desc.server_address`.
    pub async fn create_session(&self, desc: super::carrier::CarrierDesc) -> Result<SessionId> {
        let (tx, rx) = bounded(1);
        self.sender
            .send(SessionServiceMessage::CreateSession {
                desc: Box::new(desc),
                respond_to: tx,
            })
            .await
            .map_err(|_| crate::GridMateError::ConnectionClosed)?;
        rx.recv()
            .await
            .map_err(|_| crate::GridMateError::ConnectionClosed)?
    }

    /// Update all sessions (legacy no-op for async actor)
    ///
    /// # Errors
    ///
    /// Never fails. The actor drives sessions on its own task, so there
    /// is nothing to pump from the handle; the `Result` is retained
    /// only because this is published API that callers still `?` on.
    // `async` here is part of that same published signature. Removing
    // it changes the public API of a legacy shim rather than fixing a
    // defect, so the shape is preserved deliberately — see the note
    // above about this being a no-op retained for compatibility.
    #[allow(clippy::unused_async)]
    pub async fn update(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(feature = "client")]
impl SessionServiceHandle {
    /// Typed send: encode `msg` and enqueue it on `session_id`. The
    /// wire framing path `P` is selected by type inference from the
    /// message's `Sendable<P>` impls — single-direction messages
    /// resolve without turbofish; dual-direction messages (`PingMsg`)
    /// need an explicit `send::<PingMsg, ServerToClient>` at the call
    /// site.
    ///
    /// Returns a builder so callers can attach path-specific
    /// framing context (e.g. `.for_actor(actor)` on `ClientToServer`) or
    /// override defaults before `.await`-ing it (or calling
    /// `.try_send_now()` for a sync, fire-and-forget enqueue).
    pub fn send<M, P>(&self, session_id: SessionId, msg: M) -> Outgoing<'_, M, Self, P>
    where
        M: Sendable<P>,
        P: EgressPath,
    {
        Outgoing::new(self, session_id, msg)
    }

    /// Send raw data on a session
    ///
    /// # Errors
    ///
    /// Returns [`GridMateError::ConnectionClosed`](crate::GridMateError::ConnectionClosed)
    /// if the actor task has exited before the command or its reply
    /// could be delivered, and
    /// [`GridMateError::InvalidState`](crate::GridMateError::InvalidState)
    /// if `session_id` names no live session. Carrier-level send
    /// failures surface as
    /// [`GridMateError::Channel`](crate::GridMateError::Channel).
    pub async fn send_message(
        &self,
        session_id: SessionId,
        data: bytes::Bytes,
        channel: u8,
        reliability: super::carrier::DataReliability,
        priority: usize,
    ) -> Result<()> {
        let (tx, rx) = bounded(1);
        self.sender
            .send(SessionServiceMessage::SendMessage {
                session_id,
                data,
                channel,
                reliability,
                priority,
                respond_to: tx,
            })
            .await
            .map_err(|_| crate::GridMateError::ConnectionClosed)?;
        rx.recv()
            .await
            .map_err(|_| crate::GridMateError::ConnectionClosed)?
    }

    /// Enqueue raw data on a session without waiting for the actor to flush it.
    ///
    /// The session-service actor processes this command channel FIFO, so callers
    /// that invoke this from ordered Bevy systems preserve wire enqueue order
    /// without spawning per-message tasks that can race each other.
    ///
    /// # Errors
    ///
    /// Returns [`GridMateError::Channel`](crate::GridMateError::Channel)
    /// with "session service command queue full" if the actor's command
    /// queue is at capacity — this is the non-blocking counterpart of
    /// [`Self::send_message`], so backpressure is reported rather than
    /// awaited. Returns
    /// [`GridMateError::ConnectionClosed`](crate::GridMateError::ConnectionClosed)
    /// if the actor task has exited. Because the reply channel is
    /// dropped, a per-session send failure is *not* reported here.
    pub fn try_send_message_detached(
        &self,
        session_id: SessionId,
        data: bytes::Bytes,
        channel: u8,
        reliability: super::carrier::DataReliability,
        priority: usize,
    ) -> Result<()> {
        let (tx, _rx) = bounded(1);
        self.sender
            .try_send(SessionServiceMessage::SendMessage {
                session_id,
                data,
                channel,
                reliability,
                priority,
                respond_to: tx,
            })
            .map_err(|e| {
                if e.is_full() {
                    crate::GridMateError::Channel("session service command queue full".into())
                } else {
                    crate::GridMateError::ConnectionClosed
                }
            })
    }
}

#[cfg(feature = "client")]
impl SessionServiceHandle {
    /// Get number of sessions
    pub async fn num_sessions(&self) -> usize {
        let (tx, rx) = bounded(1);
        if self
            .sender
            .send(SessionServiceMessage::GetNumSessions { respond_to: tx })
            .await
            .is_err()
        {
            return 0;
        }
        rx.recv().await.unwrap_or(0)
    }

    /// Disconnect all sessions
    ///
    /// # Errors
    ///
    /// Returns [`GridMateError::ConnectionClosed`](crate::GridMateError::ConnectionClosed)
    /// if the actor task has exited before the command or its reply
    /// could be delivered. The disconnect itself cannot fail — carriers
    /// are dropped, and `Drop` performs the graceful shutdown.
    pub async fn disconnect_all(&self) -> Result<()> {
        let (tx, rx) = bounded(1);
        self.sender
            .send(SessionServiceMessage::DisconnectAll { respond_to: tx })
            .await
            .map_err(|_| crate::GridMateError::ConnectionClosed)?;
        rx.recv()
            .await
            .map_err(|_| crate::GridMateError::ConnectionClosed)?
    }

    /// Shutdown the actor (graceful)
    ///
    /// # Errors
    ///
    /// Returns [`GridMateError::ConnectionClosed`](crate::GridMateError::ConnectionClosed)
    /// if the actor task has already exited — the command channel or
    /// its reply channel is closed, which means the shutdown this call
    /// was asking for has effectively already happened.
    pub async fn shutdown(&self) -> Result<()> {
        let (tx, rx) = bounded(1);
        self.sender
            .send(SessionServiceMessage::Shutdown { respond_to: tx })
            .await
            .map_err(|_| crate::GridMateError::ConnectionClosed)?;
        rx.recv()
            .await
            .map_err(|_| crate::GridMateError::ConnectionClosed)?;
        Ok(())
    }

    /// Receive next event (awaits data)
    pub async fn recv_event(&self) -> Option<Event> {
        self.event_rx.recv().await.ok()
    }
}

#[cfg(feature = "client")]
impl Drop for SessionServiceHandle {
    fn drop(&mut self) {
        let is_last = Arc::strong_count(&self.handle_ref) == 1;

        if is_last {
            trace!("SessionService: initiating graceful shutdown");

            let (tx, rx) = bounded(1);
            let shutdown_sent = self
                .sender
                .try_send(SessionServiceMessage::Shutdown { respond_to: tx })
                .is_ok();

            if shutdown_sent {
                let shutdown_rx = self.shutdown_complete.clone();
                // NOTE: block_on() in Drop is intentional for graceful shutdown.
                // This ensures the actor completes cleanup before the handle is dropped.
                futures_lite::future::block_on(async move {
                    let _ = rx.recv().await;
                    let _ = shutdown_rx.recv().await;
                });
            } else {
                self.sender.close();
            }
        }
        // Do NOT close sender when dropping a cloned handle - other handles still need it!
    }
}

/// Behavior an outbound destination must provide to be a typed-send target.
///
/// Both [`SessionServiceHandle`] (client) and
/// [`crate::ServerListenerHandle`] (server) implement this; the
/// [`Outgoing`] builder is generic over the trait so the two sides
/// share one builder type instead of mirrored near-clones.
///
/// `enqueue` uses async fn in trait (AFIT) — the returned future is
/// monomorphised against the concrete sink, no boxed future at the
/// trait boundary. The only `Box::pin` left in the outbound path is
/// inside `Outgoing`'s `IntoFuture::into_future` impl, where it's
/// structurally required (associated type can't be `impl Future`
/// on stable).
pub trait OutboundSink: Send + Sync {
    /// Async enqueue. Backpressures on full bounded channels.
    fn enqueue(
        &self,
        session: SessionId,
        bytes: Bytes,
        channel: u8,
        reliability: super::carrier::DataReliability,
        priority: usize,
    ) -> impl Future<Output = Result<()>> + Send + '_;

    /// Sync fire-and-forget enqueue.
    ///
    /// # Errors
    ///
    /// Returns [`GridMateError::Channel`](crate::GridMateError::Channel) if the
    /// outbound queue is full — this variant never backpressures — or if the
    /// receiving actor has shut the channel down.
    fn try_enqueue(
        &self,
        session: SessionId,
        bytes: Bytes,
        channel: u8,
        reliability: super::carrier::DataReliability,
        priority: usize,
    ) -> Result<()>;
}

#[cfg(feature = "client")]
impl OutboundSink for SessionServiceHandle {
    async fn enqueue(
        &self,
        session: SessionId,
        bytes: Bytes,
        channel: u8,
        reliability: super::carrier::DataReliability,
        priority: usize,
    ) -> Result<()> {
        self.send_message(session, bytes, channel, reliability, priority)
            .await
    }

    fn try_enqueue(
        &self,
        session: SessionId,
        bytes: Bytes,
        channel: u8,
        reliability: super::carrier::DataReliability,
        priority: usize,
    ) -> Result<()> {
        self.try_send_message_detached(session, bytes, channel, reliability, priority)
    }
}

/// Typed-send builder produced by `handle.send(session, msg)` on
/// either [`SessionServiceHandle`] or [`crate::ServerListenerHandle`].
///
/// Awaiting it (or calling [`Outgoing::try_send_now`]) encodes the
/// message through the [`EgressPath`] `P` and enqueues it on the
/// target session. The path is a generic parameter (not an
/// associated type on the message) so dual-direction messages like
/// `PingMsg` can pick a direction at the call site.
///
/// Path-specific affordances are gated by `where` clauses:
/// [`Outgoing::for_actor`] only exists when `P = ClientToServer`
/// (the CRC-and-correlation outer frame). Path-agnostic knobs
/// ([`Outgoing::on_channel`], [`Outgoing::with_reliability`],
/// [`Outgoing::with_priority`]) work uniformly.
pub struct Outgoing<'a, M, S: OutboundSink, P: EgressPath> {
    sink: &'a S,
    session_id: SessionId,
    msg: M,
    context: P::Context,
    channel: u8,
    reliability: super::carrier::DataReliability,
    priority: usize,
}

impl<'a, M, S, P> Outgoing<'a, M, S, P>
where
    M: Sendable<P>,
    S: OutboundSink,
    P: EgressPath,
{
    /// Construct an `Outgoing` from a sink, session, and message.
    /// Defaults: channel 0, Reliable, priority 0, default context.
    pub fn new(sink: &'a S, session_id: SessionId, msg: M) -> Self {
        Self {
            sink,
            session_id,
            msg,
            context: <P::Context as Default>::default(),
            channel: 0,
            reliability: super::carrier::DataReliability::Reliable,
            priority: 0,
        }
    }

    /// Override the carrier channel (default 0).
    #[inline]
    #[must_use]
    pub const fn on_channel(mut self, channel: u8) -> Self {
        self.channel = channel;
        self
    }

    /// Override the reliability flag (default Reliable).
    #[inline]
    #[must_use]
    pub const fn with_reliability(mut self, reliability: super::carrier::DataReliability) -> Self {
        self.reliability = reliability;
        self
    }

    /// Override the priority slot (default 0).
    #[inline]
    #[must_use]
    pub const fn with_priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }

    /// Encode via `P::marshal_framed`. Shared between the async
    /// (`.await`) and sync (`try_send_now`) paths so the framing
    /// logic lives in one place — and is owned entirely by the path
    /// trait, not by this builder.
    fn encode(self) -> (SessionId, Bytes, u8, super::carrier::DataReliability, usize) {
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        P::marshal_framed::<M>(self.msg, self.context, &mut wb);
        let data = Bytes::from(wb.into_vec());
        (
            self.session_id,
            data,
            self.channel,
            self.reliability,
            self.priority,
        )
    }

    /// Sync, fire-and-forget enqueue. Use from Bevy systems that
    /// cannot await but need FIFO ordering.
    ///
    /// # Errors
    ///
    /// Returns any error [`OutboundSink::try_enqueue`] returns — encoding
    /// itself cannot fail, so every failure is a full or closed outbound
    /// queue.
    pub fn try_send_now(self) -> Result<()> {
        let sink = self.sink;
        let (session_id, data, channel, reliability, priority) = self.encode();
        sink.try_enqueue(session_id, data, channel, reliability, priority)
    }
}

/// `for_actor` only exists on paths whose framing context is `Uuid`
/// — today that's [`ClientToServer`] (the C→S `CorrelatedMetadata`
/// frame). Calling it on sized-only paths is a compile error, not a
/// silent no-op, because their `Context` type is `()`.
impl<M, S> Outgoing<'_, M, S, ClientToServer>
where
    M: Sendable<ClientToServer>,
    S: OutboundSink,
{
    /// Route this send through the wire correlation slot for `actor`.
    /// The target actor's UUID occupies this slot; the framing layer covers
    /// it with the outer CRC32 alongside the envelope body.
    #[inline]
    #[must_use]
    pub const fn for_actor(mut self, actor: uuid::Uuid) -> Self {
        self.context = actor;
        self
    }
}

impl<'a, M, S, P> IntoFuture for Outgoing<'a, M, S, P>
where
    M: Sendable<P>,
    S: OutboundSink,
    P: EgressPath,
{
    type Output = Result<()>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        let sink = self.sink;
        let (session_id, data, channel, reliability, priority) = self.encode();
        Box::pin(async move {
            sink.enqueue(session_id, data, channel, reliability, priority)
                .await
        })
    }
}
