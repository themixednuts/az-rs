use super::{AssetProcessorEvent, Receiver, Sender, ServiceHealth, asset_capnp, channel, oneshot};

const EDITOR_ASSET_PROCESSOR_EVENT_CAPACITY: usize = 1;

/// The three causal edges of one editor Asset Processor subscription.
///
/// Asset updates use a capacity-one channel because the Asset Processor owns
/// the corresponding latest-value coalescing slot and does not send the next
/// sink request until this sink acknowledges the current one. Initial state
/// and terminal state are distinct one-shot edges, so neither can be dropped
/// or mistaken for an asset update under pressure.
pub(crate) struct EditorAssetProcessorEventInbox {
    pub(crate) initial: oneshot::Receiver<ServiceHealth>,
    pub(crate) events: Receiver<AssetProcessorEvent>,
    pub(crate) terminal: oneshot::Receiver<String>,
}

pub(crate) struct EditorAssetProcessorEventPublisher {
    initial: Option<oneshot::Sender<ServiceHealth>>,
    events: Sender<AssetProcessorEvent>,
    terminal: Option<oneshot::Sender<String>>,
}

impl EditorAssetProcessorEventPublisher {
    #[must_use]
    pub(crate) fn new() -> (Self, EditorAssetProcessorEventInbox) {
        let (initial_tx, initial) = oneshot::channel();
        let (events, event_rx) = channel(EDITOR_ASSET_PROCESSOR_EVENT_CAPACITY);
        let (terminal_tx, terminal) = oneshot::channel();
        (
            Self {
                initial: Some(initial_tx),
                events,
                terminal: Some(terminal_tx),
            },
            EditorAssetProcessorEventInbox {
                initial,
                events: event_rx,
                terminal,
            },
        )
    }

    #[must_use]
    pub(crate) fn event_sender(&self) -> Sender<AssetProcessorEvent> {
        self.events.clone()
    }

    /// The rejected `ServiceHealth` comes back boxed: it is a wide struct and
    /// only the receiver-gone case ever carries one, so the success path should
    /// not pay for its size.
    pub(crate) fn publish_initial(
        &mut self,
        health: ServiceHealth,
    ) -> Result<(), Box<ServiceHealth>> {
        self.initial
            .take()
            .expect("asset processor initial state publishes once")
            .send(health)
            .map_err(Box::new)
    }

    pub(crate) fn publish_terminal(&mut self, detail: String) {
        if let Some(terminal) = self.terminal.take() {
            let _ = terminal.send(detail);
        }
    }
}

pub struct EditorAssetProcessorEventSink {
    tx: Sender<AssetProcessorEvent>,
}

impl EditorAssetProcessorEventSink {
    #[must_use]
    pub const fn new(tx: Sender<AssetProcessorEvent>) -> Self {
        Self { tx }
    }

    #[must_use]
    pub fn into_client(self) -> asset_capnp::asset_processor_event_sink::Client {
        capnp_rpc::new_client(self)
    }
}

impl asset_capnp::asset_processor_event_sink::Server for EditorAssetProcessorEventSink {
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn update(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor_event_sink::UpdateParams,
        _results: asset_capnp::asset_processor_event_sink::UpdateResults,
    ) -> Result<(), capnp::Error> {
        let event = AssetProcessorEvent::from_capnp(params.get()?.get_event()?)?;
        self.tx.send(event).await.map_err(|_| {
            capnp::Error::failed(
                "editor asset processor event consumer is no longer attached".to_string(),
            )
        })
    }
}
