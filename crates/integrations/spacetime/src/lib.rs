//! Reconnecting `SpacetimeDB` clients driven by Bevy's frame schedule.
//!
//! Generated `SpacetimeDB` bindings expose a concrete `DbConnection` type, so
//! this crate keeps the connection generic and lets the caller provide the
//! generated builder and `frame_tick` calls. [`SpacetimeClient`] owns retry,
//! callback ordering, and readiness. Callers can use a connection only through
//! [`SpacetimeClient::with_ready`].

use bevy::prelude::*;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(5);

type BoxError = Box<dyn Error + Send + Sync>;
type Connector<C> = dyn Fn(SpacetimeReporter) -> Result<C, BoxError> + Send + Sync;
type FrameTick<C> = dyn Fn(&C) -> Result<(), BoxError> + Send + Sync;

/// Installs the frame driver for one generated `SpacetimeDB` connection type.
///
/// The driver runs in `PreUpdate`, before gameplay systems in `Update` can call
/// [`SpacetimeClient::with_ready`]. The client resource may be inserted later.
pub struct SpacetimePlugin<C>(PhantomData<fn() -> C>);

impl<C> Default for SpacetimePlugin<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<C> Plugin for SpacetimePlugin<C>
where
    C: Send + Sync + 'static,
{
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, drive_spacetime::<C>);
    }
}

/// Reports generated SDK callback facts to the owning [`SpacetimeClient`].
///
/// The connector receives a fresh reporter on every attempt. Register all SDK
/// callbacks and the grouped subscription with that reporter before returning
/// the generated connection.
#[derive(Clone)]
pub struct SpacetimeReporter {
    attempt: u64,
    reports: Sender<Report>,
}

impl SpacetimeReporter {
    pub fn connected(&self) {
        self.send(ReportKind::Connected);
    }

    pub fn subscriptions_ready(&self) {
        self.send(ReportKind::SubscriptionsReady);
    }

    pub fn connect_failed(&self, reason: impl Into<String>) {
        self.send(ReportKind::Unavailable(reason.into()));
    }

    pub fn subscriptions_failed(&self, reason: impl Into<String>) {
        self.send(ReportKind::Unavailable(reason.into()));
    }

    pub fn disconnected(&self, reason: impl Into<String>) {
        self.send(ReportKind::Unavailable(reason.into()));
    }

    fn send(&self, kind: ReportKind) {
        let _ = self.reports.send(Report {
            attempt: self.attempt,
            kind,
        });
    }
}

struct Report {
    attempt: u64,
    kind: ReportKind,
}

enum ReportKind {
    Connected,
    SubscriptionsReady,
    Unavailable(String),
}

/// Returned when no connection has completed both the SDK connect callback and
/// the caller's grouped subscription callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpacetimeUnavailable;

impl fmt::Display for SpacetimeUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SpacetimeDB is unavailable")
    }
}

impl Error for SpacetimeUnavailable {}

/// Owns one generated `SpacetimeDB` connection and reconnects it after failure.
///
/// Readiness requires both `connected` and `subscriptions_ready` reports. They
/// may arrive in either order. Connect, subscription, disconnect, and
/// `frame_tick` failures discard the connection and schedule a bounded retry.
#[derive(Resource)]
pub struct SpacetimeClient<C>
where
    C: Send + Sync + 'static,
{
    connector: Arc<Connector<C>>,
    frame_tick: Arc<FrameTick<C>>,
    reports_tx: Sender<Report>,
    reports_rx: Mutex<Receiver<Report>>,
    state: ClientState<C>,
    next_attempt: u64,
    backoff: RetryBackoff,
}

impl<C> SpacetimeClient<C>
where
    C: Send + Sync + 'static,
{
    /// Creates a frame-driven client around generated connection operations.
    pub fn frame_driven<Connect, ConnectError, Tick, TickError>(
        connector: Connect,
        frame_tick: Tick,
    ) -> Self
    where
        Connect: Fn(SpacetimeReporter) -> Result<C, ConnectError> + Send + Sync + 'static,
        ConnectError: Error + Send + Sync + 'static,
        Tick: Fn(&C) -> Result<(), TickError> + Send + Sync + 'static,
        TickError: Error + Send + Sync + 'static,
    {
        Self::with_backoff(
            INITIAL_RETRY_DELAY,
            MAX_RETRY_DELAY,
            move |reporter| connector(reporter).map_err(|error| Box::new(error) as BoxError),
            move |connection| frame_tick(connection).map_err(|error| Box::new(error) as BoxError),
        )
    }

    /// Returns true only while the current connection is subscribed and ready.
    pub const fn is_ready(&self) -> bool {
        matches!(
            self.state,
            ClientState::Active(ActiveConnection { ready: true, .. })
        )
    }

    /// Runs `operation` against the current ready connection.
    ///
    /// The connection reference cannot outlive this call, which prevents a
    /// caller from retaining an old generated connection across reconnects.
    ///
    /// # Errors
    ///
    /// Returns [`SpacetimeUnavailable`] if the client is not in the active
    /// state (it is still connecting, or waiting out a reconnect backoff), or
    /// if it is active but its subscription has not yet been applied. In both
    /// cases `operation` is not called.
    pub fn with_ready<R>(
        &self,
        operation: impl FnOnce(&C) -> R,
    ) -> Result<R, SpacetimeUnavailable> {
        let ClientState::Active(active) = &self.state else {
            return Err(SpacetimeUnavailable);
        };
        if !active.ready {
            return Err(SpacetimeUnavailable);
        }
        Ok(operation(&active.connection))
    }

    fn with_backoff<Connect, Tick>(
        initial_retry: Duration,
        max_retry: Duration,
        connector: Connect,
        frame_tick: Tick,
    ) -> Self
    where
        Connect: Fn(SpacetimeReporter) -> Result<C, BoxError> + Send + Sync + 'static,
        Tick: Fn(&C) -> Result<(), BoxError> + Send + Sync + 'static,
    {
        let (reports_tx, reports_rx) = mpsc::channel();
        Self {
            connector: Arc::new(connector),
            frame_tick: Arc::new(frame_tick),
            reports_tx,
            reports_rx: Mutex::new(reports_rx),
            state: ClientState::Waiting {
                retry_at: Instant::now(),
            },
            next_attempt: 1,
            backoff: RetryBackoff::new(initial_retry, max_retry),
        }
    }

    fn drive(&mut self) {
        self.apply_reports();
        self.connect_if_due();
        self.apply_reports();
        self.tick_active();
        self.apply_reports();
    }

    fn connect_if_due(&mut self) {
        let ClientState::Waiting { retry_at } = self.state else {
            return;
        };
        if Instant::now() < retry_at {
            return;
        }

        let attempt = self.next_attempt;
        self.next_attempt = self.next_attempt.saturating_add(1);
        tracing::info!(attempt, "SpacetimeDB connection attempt");
        let reporter = SpacetimeReporter {
            attempt,
            reports: self.reports_tx.clone(),
        };
        match (self.connector)(reporter) {
            Ok(connection) => {
                self.state = ClientState::Active(ActiveConnection {
                    attempt,
                    connection,
                    connected: false,
                    subscriptions_ready: false,
                    ready: false,
                });
            }
            Err(error) => self.schedule_retry(false, &format!("connection build failed: {error}")),
        }
    }

    fn tick_active(&mut self) {
        let result = match &self.state {
            ClientState::Active(active) => (self.frame_tick)(&active.connection),
            ClientState::Waiting { .. } => return,
        };
        if let Err(error) = result {
            let was_ready = self.is_ready();
            self.schedule_retry(was_ready, &format!("frame_tick failed: {error}"));
        }
    }

    fn apply_reports(&mut self) {
        let reports: Vec<_> = self.reports_rx.lock().unwrap().try_iter().collect();
        for report in reports {
            self.apply_report(report);
        }
    }

    fn apply_report(&mut self, report: Report) {
        let mut unavailable = None;
        let mut became_ready = false;
        {
            let ClientState::Active(active) = &mut self.state else {
                return;
            };
            if active.attempt != report.attempt {
                return;
            }
            match report.kind {
                ReportKind::Connected => active.connected = true,
                ReportKind::SubscriptionsReady => active.subscriptions_ready = true,
                ReportKind::Unavailable(reason) => unavailable = Some((active.ready, reason)),
            }
            if unavailable.is_none()
                && !active.ready
                && active.connected
                && active.subscriptions_ready
            {
                active.ready = true;
                became_ready = true;
            }
        }

        if let Some((was_ready, reason)) = unavailable {
            self.schedule_retry(was_ready, &reason);
        } else if became_ready {
            self.backoff.reset();
            tracing::info!(attempt = report.attempt, "SpacetimeDB client ready");
        }
    }

    fn schedule_retry(&mut self, was_ready: bool, reason: &str) {
        let delay = self.backoff.take();
        self.state = ClientState::Waiting {
            retry_at: Instant::now() + delay,
        };
        if was_ready {
            tracing::warn!(%reason, "SpacetimeDB client unavailable");
        } else {
            tracing::warn!(%reason, "SpacetimeDB connection attempt failed");
        }
        tracing::info!(
            retry_in_ms = delay.as_millis(),
            "SpacetimeDB reconnect scheduled"
        );
    }
}

enum ClientState<C> {
    Waiting { retry_at: Instant },
    Active(ActiveConnection<C>),
}

struct ActiveConnection<C> {
    attempt: u64,
    connection: C,
    connected: bool,
    subscriptions_ready: bool,
    ready: bool,
}

struct RetryBackoff {
    initial: Duration,
    maximum: Duration,
    next: Duration,
}

impl RetryBackoff {
    const fn new(initial: Duration, maximum: Duration) -> Self {
        Self {
            initial,
            maximum,
            next: initial,
        }
    }

    const fn reset(&mut self) {
        self.next = self.initial;
    }

    fn take(&mut self) -> Duration {
        let delay = self.next;
        self.next = self.next.saturating_mul(2).min(self.maximum);
        delay
    }
}

fn drive_spacetime<C>(client: Option<ResMut<SpacetimeClient<C>>>)
where
    C: Send + Sync + 'static,
{
    let Some(mut client) = client else {
        return;
    };
    client.drive();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct TestConnection;

    fn client_with_reporters(
        reporters: Arc<Mutex<Vec<SpacetimeReporter>>>,
        connects: Arc<AtomicUsize>,
    ) -> SpacetimeClient<TestConnection> {
        SpacetimeClient::with_backoff(
            Duration::from_millis(10),
            Duration::from_millis(80),
            move |reporter| {
                connects.fetch_add(1, Ordering::SeqCst);
                reporters.lock().unwrap().push(reporter);
                Ok::<_, BoxError>(TestConnection)
            },
            |_connection| Ok::<_, BoxError>(()),
        )
    }

    #[test]
    fn readiness_accepts_callbacks_in_either_order() {
        for subscriptions_first in [false, true] {
            let reporters = Arc::new(Mutex::new(Vec::new()));
            let connects = Arc::new(AtomicUsize::new(0));
            let mut client = client_with_reporters(reporters.clone(), connects);
            client.drive();
            let reporter = reporters.lock().unwrap()[0].clone();

            if subscriptions_first {
                reporter.subscriptions_ready();
            } else {
                reporter.connected();
            }
            client.drive();
            assert!(!client.is_ready());

            if subscriptions_first {
                reporter.connected();
            } else {
                reporter.subscriptions_ready();
            }
            client.drive();
            assert!(client.is_ready());
            assert!(client.with_ready(|_| 7).is_ok());
        }
    }

    #[test]
    fn disconnect_schedules_once_and_reinstalls_callbacks_on_retry() {
        let reporters = Arc::new(Mutex::new(Vec::new()));
        let connects = Arc::new(AtomicUsize::new(0));
        let mut client = client_with_reporters(reporters.clone(), connects.clone());
        client.drive();
        let first = reporters.lock().unwrap()[0].clone();
        first.connected();
        first.subscriptions_ready();
        client.drive();
        assert!(client.is_ready());

        first.disconnected("test disconnect");
        first.disconnected("duplicate callback");
        client.drive();
        assert!(!client.is_ready());
        assert_eq!(client.backoff.next, Duration::from_millis(20));
        assert_eq!(connects.load(Ordering::SeqCst), 1);

        if let ClientState::Waiting { retry_at } = &mut client.state {
            *retry_at = Instant::now();
        }
        client.drive();
        assert_eq!(connects.load(Ordering::SeqCst), 2);
        let second = reporters.lock().unwrap()[1].clone();
        second.connected();
        second.subscriptions_ready();
        client.drive();
        assert!(client.is_ready());
    }

    #[test]
    fn frame_tick_failure_discards_ready_connection() {
        let reporter_slot = Arc::new(Mutex::new(None));
        let reporter_for_connect = reporter_slot.clone();
        let ticks = Arc::new(AtomicUsize::new(0));
        let ticks_for_frame = ticks;
        let mut client = SpacetimeClient::with_backoff(
            Duration::from_secs(1),
            Duration::from_secs(1),
            move |reporter| {
                *reporter_for_connect.lock().unwrap() = Some(reporter);
                Ok::<_, BoxError>(TestConnection)
            },
            move |_connection| {
                if ticks_for_frame.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::other("tick failed").into())
                }
            },
        );
        client.drive();
        let reporter = reporter_slot.lock().unwrap().clone().unwrap();
        reporter.connected();
        reporter.subscriptions_ready();
        client.drive();

        assert!(!client.is_ready());
        assert_eq!(client.with_ready(|_| ()).unwrap_err(), SpacetimeUnavailable);
    }

    #[test]
    fn public_constructor_accepts_generated_style_errors() {
        let client = SpacetimeClient::frame_driven(
            |_reporter| Ok::<_, std::io::Error>(TestConnection),
            |_connection| Ok::<_, Infallible>(()),
        );
        assert!(!client.is_ready());
    }
}
