//! Cancellation handles for editor background watcher threads.

use std::sync::Arc;
use tokio::sync::oneshot;

/// Shutdown signal for one background watcher thread.
///
/// Stored behind an [`Arc`] on whatever owns the watcher (usually a gpui
/// global): dropping the last reference fires the shutdown channel and the
/// watcher exits at its next `select!` point. No join — joining from the UI
/// thread could block for up to one watch interval, and these watchers hold
/// nothing that needs reclamation beyond their own client.
pub struct WatchHandle {
    shutdown: Option<oneshot::Sender<()>>,
}

impl WatchHandle {
    /// Creates the handle together with the receiver the watcher thread
    /// selects against.
    pub(crate) fn channel() -> (Arc<Self>, oneshot::Receiver<()>) {
        let (shutdown, rx) = oneshot::channel();
        (
            Arc::new(Self {
                shutdown: Some(shutdown),
            }),
            rx,
        )
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    #[test]
    fn dropping_last_handle_fires_shutdown() {
        let (handle, mut shutdown_rx) = super::WatchHandle::channel();
        let held_elsewhere = std::sync::Arc::clone(&handle);

        // With another holder alive the sender has not fired.
        drop(handle);
        assert!(shutdown_rx.try_recv().is_err());

        // Dropping the last reference fires the channel: the receiver
        // observes the shutdown signal rather than hanging.
        drop(held_elsewhere);
        assert!(
            block_on(&mut shutdown_rx).is_ok(),
            "expected shutdown signal after last handle dropped"
        );
    }
}
