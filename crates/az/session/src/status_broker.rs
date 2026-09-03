use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;

use az_proto_session::{
    SessionSupervisorEvent, SessionSupervisorEventSubscriptionResult, SessionWorkspaceStatus,
    session_capnp,
};
use tracing::warn;

use crate::SessionError;

const STATUS_PUBLICATION_CAPACITY: usize = 1;

pub struct SessionStatusPublication {
    slug: String,
    status: SessionWorkspaceStatus,
    admitted: mpsc::SyncSender<()>,
}

/// Cross-thread handle into the transport-owned session status broker.
///
/// Publication is synchronous by design: the sessiond owner loop does not
/// resolve a terminal lifecycle command until the transport has admitted its
/// canonical post-transition snapshot. Remote sink acknowledgement never
/// blocks the owner loop.
#[derive(Debug, Clone)]
pub struct SessionStatusPublisher {
    publications: tokio::sync::mpsc::Sender<SessionStatusPublication>,
}

impl SessionStatusPublisher {
    /// Hands a canonical snapshot to the transport-owned broker and blocks
    /// until the broker has admitted it.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::StatusBrokerUnavailable`] if the transport's
    /// publication receiver has already been dropped, or if the transport is
    /// torn down before it signals admission of this snapshot.
    pub fn publish(
        &self,
        slug: impl Into<String>,
        status: SessionWorkspaceStatus,
    ) -> Result<(), SessionError> {
        let (admitted, admission) = mpsc::sync_channel(1);
        self.publications
            .blocking_send(SessionStatusPublication {
                slug: slug.into(),
                status,
                admitted,
            })
            .map_err(|_| SessionError::StatusBrokerUnavailable)?;
        admission
            .recv()
            .map_err(|_| SessionError::StatusBrokerUnavailable)
    }
}

pub fn session_status_publication_channel() -> (
    SessionStatusPublisher,
    tokio::sync::mpsc::Receiver<SessionStatusPublication>,
) {
    let (publications, receiver) = tokio::sync::mpsc::channel(STATUS_PUBLICATION_CAPACITY);
    (SessionStatusPublisher { publications }, receiver)
}

#[derive(Clone)]
struct SessionStatusSubscriber {
    id: u64,
    slug: String,
    sink: session_capnp::session_supervisor_event_sink::Client,
    in_flight: bool,
    pending: Option<SessionSupervisorEvent>,
}

/// LocalSet-owned latest-value broker for canonical session status snapshots.
pub struct SessionStatusBroker {
    subscribers: RefCell<Vec<SessionStatusSubscriber>>,
    next_subscriber_id: Cell<u64>,
    next_sequence: Cell<u64>,
}

impl std::fmt::Debug for SessionStatusBroker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionStatusBroker")
            .field("subscriber_count", &self.subscribers.borrow().len())
            .field("next_subscriber_id", &self.next_subscriber_id.get())
            .field("next_sequence", &self.next_sequence.get())
            .finish()
    }
}

impl SessionStatusBroker {
    #[must_use]
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            subscribers: RefCell::new(Vec::new()),
            next_subscriber_id: Cell::new(0),
            next_sequence: Cell::new(1),
        })
    }

    pub(crate) fn subscribe(
        self: &Rc<Self>,
        slug: String,
        initial_status: SessionWorkspaceStatus,
        sink: session_capnp::session_supervisor_event_sink::Client,
    ) -> SessionSupervisorEventSubscriptionResult {
        let id = self.next_subscriber_id.get();
        self.next_subscriber_id.set(id.saturating_add(1));
        self.subscribers.borrow_mut().push(SessionStatusSubscriber {
            id,
            slug,
            sink,
            in_flight: false,
            pending: None,
        });
        SessionSupervisorEventSubscriptionResult {
            subscribed: true,
            initial_sequence: self.next_sequence.get().saturating_sub(1),
            initial_status,
        }
    }

    pub(crate) fn publish(self: &Rc<Self>, slug: &str, status: SessionWorkspaceStatus) {
        let sequence = self.next_sequence.get();
        self.next_sequence.set(sequence.saturating_add(1));
        let event = SessionSupervisorEvent { sequence, status };
        let subscriber_ids = self
            .subscribers
            .borrow()
            .iter()
            .filter(|subscriber| subscriber.slug == slug)
            .map(|subscriber| subscriber.id)
            .collect::<Vec<_>>();
        for subscriber_id in subscriber_ids {
            self.enqueue(subscriber_id, event.clone());
        }
    }

    fn enqueue(self: &Rc<Self>, subscriber_id: u64, event: SessionSupervisorEvent) {
        let delivery = {
            let mut subscribers = self.subscribers.borrow_mut();
            let Some(subscriber) = subscribers
                .iter_mut()
                .find(|subscriber| subscriber.id == subscriber_id)
            else {
                return;
            };
            if subscriber.in_flight {
                subscriber.pending = Some(event);
                return;
            }
            subscriber.in_flight = true;
            (subscriber.sink.clone(), event)
        };
        self.deliver(subscriber_id, &delivery.0, &delivery.1);
    }

    fn deliver(
        self: &Rc<Self>,
        subscriber_id: u64,
        sink: &session_capnp::session_supervisor_event_sink::Client,
        event: &SessionSupervisorEvent,
    ) {
        let mut request = sink.update_request();
        if let Err(error) = event.to_capnp(request.get().init_event()) {
            warn!(subscriber = subscriber_id, %error, "failed to encode session status event; dropping subscription");
            self.remove(subscriber_id);
            return;
        }
        let broker = Rc::clone(self);
        tokio::task::spawn_local(async move {
            match request.send().promise.await {
                Ok(_) => broker.complete(subscriber_id),
                Err(error) => {
                    warn!(subscriber = subscriber_id, %error, "session status sink rejected update; dropping subscription");
                    broker.remove(subscriber_id);
                }
            }
        });
    }

    fn complete(self: &Rc<Self>, subscriber_id: u64) {
        let next = {
            let mut subscribers = self.subscribers.borrow_mut();
            let Some(subscriber) = subscribers
                .iter_mut()
                .find(|subscriber| subscriber.id == subscriber_id)
            else {
                return;
            };
            if let Some(event) = subscriber.pending.take() {
                Some((subscriber.sink.clone(), event))
            } else {
                subscriber.in_flight = false;
                None
            }
        };
        if let Some((sink, event)) = next {
            self.deliver(subscriber_id, &sink, &event);
        }
    }

    fn remove(&self, subscriber_id: u64) {
        self.subscribers
            .borrow_mut()
            .retain(|subscriber| subscriber.id != subscriber_id);
    }

    #[cfg(test)]
    pub(crate) fn subscriber_count(&self) -> usize {
        self.subscribers.borrow().len()
    }
}

// Cannot be made `Send`: the broker is `Rc`-owned and holds capnp-rpc `Client`
// capabilities, which are `!Send` by construction; this task only ever runs on
// the transport's `LocalSet`.
#[allow(clippy::future_not_send)]
pub async fn run_status_publications(
    broker: Rc<SessionStatusBroker>,
    mut publications: tokio::sync::mpsc::Receiver<SessionStatusPublication>,
) {
    while let Some(publication) = publications.recv().await {
        broker.publish(&publication.slug, publication.status);
        let _ = publication.admitted.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_proto_session::{SessionManifest, SessionState};
    use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
    use uuid::Uuid;

    struct RecordingSink {
        events: UnboundedSender<SessionSupervisorEvent>,
        reject: bool,
    }

    // Cannot be made `Send`: the capnp-generated `Server` trait hands the
    // method `capnp::capability::Rc<Self>`, which is `!Send` by construction.
    #[allow(clippy::future_not_send)]
    impl session_capnp::session_supervisor_event_sink::Server for RecordingSink {
        async fn update(
            self: capnp::capability::Rc<Self>,
            params: session_capnp::session_supervisor_event_sink::UpdateParams,
            _results: session_capnp::session_supervisor_event_sink::UpdateResults,
        ) -> Result<(), capnp::Error> {
            if self.reject {
                return Err(capnp::Error::failed("sink rejected update".to_string()));
            }
            let event = SessionSupervisorEvent::from_capnp(params.get()?.get_event()?)?;
            self.events
                .send(event)
                .map_err(|_| capnp::Error::failed("recording sink receiver closed".to_string()))
        }
    }

    fn status(slug: &str, state: SessionState) -> SessionWorkspaceStatus {
        let project_root = std::env::temp_dir().join("azoth-status-broker-project");
        let run_dir = project_root.join(".azoth/session");
        SessionWorkspaceStatus {
            manifest: SessionManifest::new(
                Uuid::from_bytes([0x41; 16]),
                "local.status_broker",
                slug,
                project_root.to_string_lossy(),
                project_root.to_string_lossy(),
                run_dir.to_string_lossy(),
                state,
            ),
            failure_reason: None,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn subscription_is_atomic_and_coalesces_to_latest_canonical_status() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let broker = SessionStatusBroker::new();
                let (events, mut received) = unbounded_channel();
                let initial = status("editor", SessionState::Active);
                let subscription = broker.subscribe(
                    "editor".to_string(),
                    initial.clone(),
                    capnp_rpc::new_client(RecordingSink {
                        events,
                        reject: false,
                    }),
                );

                assert!(subscription.subscribed);
                assert_eq!(subscription.initial_sequence, 0);
                assert_eq!(subscription.initial_status, initial);

                broker.publish("editor", status("editor", SessionState::Preparing));
                broker.publish("editor", status("editor", SessionState::FailedPreserved));
                broker.publish("editor", status("editor", SessionState::Removed));

                let first = received.recv().await.unwrap();
                let latest = received.recv().await.unwrap();
                assert_eq!(first.sequence, 1);
                assert_eq!(first.status.manifest.state, SessionState::Preparing);
                assert_eq!(latest.sequence, 3);
                assert_eq!(latest.status.manifest.state, SessionState::Removed);
                assert_eq!(broker.subscriber_count(), 1);
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_sink_is_pruned() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let broker = SessionStatusBroker::new();
                let (events, _received) = unbounded_channel();
                broker.subscribe(
                    "editor".to_string(),
                    status("editor", SessionState::Active),
                    capnp_rpc::new_client(RecordingSink {
                        events,
                        reject: true,
                    }),
                );
                broker.publish("editor", status("editor", SessionState::Preparing));
                tokio::task::yield_now().await;
                tokio::task::yield_now().await;
                assert_eq!(broker.subscriber_count(), 0);
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cross_thread_publication_is_admitted_before_owner_continues() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let broker = SessionStatusBroker::new();
                let (events, mut received) = unbounded_channel();
                broker.subscribe(
                    "editor".to_string(),
                    status("editor", SessionState::Active),
                    capnp_rpc::new_client(RecordingSink {
                        events,
                        reject: false,
                    }),
                );
                let (publisher, publications) = session_status_publication_channel();
                tokio::task::spawn_local(run_status_publications(Rc::clone(&broker), publications));
                let publisher_thread = std::thread::spawn(move || {
                    publisher
                        .publish("editor", status("editor", SessionState::Preparing))
                        .unwrap();
                });

                let event = received.recv().await.unwrap();
                assert_eq!(event.status.manifest.state, SessionState::Preparing);
                publisher_thread.join().unwrap();
            })
            .await;
    }
}
