//! First-class structured progress.
//!
//! Progress is a structured event stream, not logging. A [`Reporter`] wraps an
//! installable [`ProgressSink`]; the default sink is a no-op, so callers that
//! never set a reporter pay nothing. [`Progress`] handles form a tree (root +
//! children) and emit [`ProgressEvent`]s with a monotonic [`ProgressId`].
//!
//! Domain-specific progress models (e.g. the daemon's typed open-project phase
//! model) are built on top of this generic [`ProgressSink`]; az-work stays
//! domain-agnostic.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::cancel::CancellationToken;

/// Monotonic identity for a progress node, unique within a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProgressId(u64);

impl ProgressId {
    /// Allocate the next process-unique progress id.
    fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Raw numeric value, useful for transport over an RPC boundary.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Small owned label for a progress node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressName(Cow<'static, str>);

impl ProgressName {
    /// Borrow the underlying label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&'static str> for ProgressName {
    fn from(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }
}

impl From<String> for ProgressName {
    fn from(value: String) -> Self {
        Self(Cow::Owned(value))
    }
}

impl std::fmt::Display for ProgressName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A monotonic, integer-valued completion fraction.
///
/// Progress is reported as integers (never floats) so it survives an RPC
/// boundary without rounding drift. A fraction is either an exact ratio with a
/// known denominator, an open-ended `Unknown` (work is happening but the total
/// is not yet known), or `Complete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fraction {
    /// A known ratio. `done` is clamped to `total` by [`Fraction::exact`].
    Exact {
        /// Units completed.
        done: u64,
        /// Total units; never zero (use [`Fraction::Unknown`] for that).
        total: std::num::NonZeroU64,
    },
    /// Work in progress with an unknown total (e.g. a first-ever build).
    Unknown {
        /// Units completed so far.
        done: u64,
    },
    /// Fully complete; renders as 100%.
    Complete,
}

impl Fraction {
    /// Basis-point scale (10000 == 100%).
    pub const BASIS_POINTS: u32 = 10_000;

    /// Build an exact fraction, clamping `done` to `total` and collapsing a
    /// zero total to [`Fraction::Unknown`].
    #[must_use]
    pub fn exact(done: u64, total: u64) -> Self {
        std::num::NonZeroU64::new(total).map_or(Self::Unknown { done }, |total| Self::Exact {
            done: done.min(total.get()),
            total,
        })
    }

    /// Whether this fraction's denominator is known (`Exact` or `Complete`).
    #[must_use]
    pub const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown { .. })
    }

    /// Completion as basis points in `0..=10000`.
    ///
    /// `Unknown` reports `0` (the aggregate caller decides how to surface an
    /// indeterminate phase); `Complete` reports the full scale.
    #[must_use]
    pub fn to_basis_points(self) -> u32 {
        match self {
            Self::Complete => Self::BASIS_POINTS,
            Self::Unknown { .. } => 0,
            Self::Exact { done, total } => {
                // Widen to u128 so done * 10000 cannot overflow.
                let scaled =
                    (u128::from(done) * u128::from(Self::BASIS_POINTS)) / u128::from(total.get());
                // The min clamps to BASIS_POINTS (10000), so the value always
                // fits in u32; fall back to the full scale on the impossible
                // conversion failure rather than truncating.
                u32::try_from(scaled.min(u128::from(Self::BASIS_POINTS)))
                    .unwrap_or(Self::BASIS_POINTS)
            }
        }
    }

    /// The `(done, total)` pair for the wire, where `total == 0` means
    /// [`Fraction::Unknown`]. `Complete` reports `(done, done)` if it was built
    /// from a known total, otherwise `(0, 0)`; callers that need the original
    /// total should keep it alongside.
    #[must_use]
    pub const fn raw(self) -> (u64, u64) {
        match self {
            Self::Exact { done, total } => (done, total.get()),
            Self::Unknown { done } => (done, 0),
            Self::Complete => (0, 0),
        }
    }

    /// Reconstruct a fraction from a wire `(done, total)` pair where
    /// `total == 0` denotes [`Fraction::Unknown`].
    #[must_use]
    pub fn from_raw(done: u64, total: u64) -> Self {
        Self::exact(done, total)
    }
}

/// What happened to a progress node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressKind {
    /// The node started; emitted once when created.
    Started,
    /// The node's expected total work was set or revised.
    SetTotal(u64),
    /// The node advanced by `delta` units of completed work.
    Advance(u64),
    /// A human-readable status message for the node.
    Message(String),
    /// The node finished; no further events follow.
    Finished,
}

/// A single structured progress event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressEvent {
    /// Identity of the node this event describes.
    pub id: ProgressId,
    /// Parent node, if any (root nodes have `None`).
    pub parent: Option<ProgressId>,
    /// Label of the node.
    pub name: ProgressName,
    /// The event payload.
    pub kind: ProgressKind,
}

/// Receiver for structured progress events.
///
/// Implementations must be cheap to call and tolerate events from multiple
/// threads. The daemon's RPC progress sink implements this to forward typed
/// open-project events to the editor.
pub trait ProgressSink: Send + Sync + 'static {
    /// Handle one structured progress event.
    fn event(&self, event: ProgressEvent);
}

struct NoopSink;

impl ProgressSink for NoopSink {
    fn event(&self, _event: ProgressEvent) {}
}

/// Cheaply-clonable handle to an installed [`ProgressSink`].
#[derive(Clone)]
pub struct Reporter {
    sink: Arc<dyn ProgressSink>,
}

impl std::fmt::Debug for Reporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reporter").finish_non_exhaustive()
    }
}

impl Default for Reporter {
    fn default() -> Self {
        Self::noop()
    }
}

impl Reporter {
    /// Construct a reporter backed by `sink`.
    #[must_use]
    pub fn new(sink: Arc<dyn ProgressSink>) -> Self {
        Self { sink }
    }

    /// Construct a reporter whose sink discards every event.
    #[must_use]
    pub fn noop() -> Self {
        Self {
            sink: Arc::new(NoopSink),
        }
    }

    /// Begin a root progress node, emitting a [`ProgressKind::Started`] event.
    #[must_use]
    pub fn root(&self, name: impl Into<ProgressName>) -> Progress {
        self.node(None, name.into(), CancellationToken::new())
    }

    /// Begin a root progress node tied to a cancellation token.
    #[must_use]
    pub fn root_with_cancel(
        &self,
        name: impl Into<ProgressName>,
        cancel: CancellationToken,
    ) -> Progress {
        self.node(None, name.into(), cancel)
    }

    fn node(
        &self,
        parent: Option<ProgressId>,
        name: ProgressName,
        cancel: CancellationToken,
    ) -> Progress {
        let id = ProgressId::next();
        self.sink.event(ProgressEvent {
            id,
            parent,
            name: name.clone(),
            kind: ProgressKind::Started,
        });
        Progress {
            reporter: self.clone(),
            id,
            name,
            cancel,
        }
    }
}

/// A live progress node. Drop without [`finish`](Progress::finish) does not
/// emit a `Finished` event; callers should call `finish()` explicitly.
#[derive(Debug)]
pub struct Progress {
    reporter: Reporter,
    id: ProgressId,
    name: ProgressName,
    cancel: CancellationToken,
}

impl Progress {
    /// Identity of this node.
    #[must_use]
    pub const fn id(&self) -> ProgressId {
        self.id
    }

    /// Cancellation token associated with this node's operation.
    #[must_use]
    pub const fn cancel(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Begin a child progress node nested under this one.
    #[must_use]
    pub fn child(&self, name: impl Into<ProgressName>) -> Self {
        self.reporter
            .node(Some(self.id), name.into(), self.cancel.child_token())
    }

    /// Set or revise the total amount of work for this node.
    pub fn set_total(&self, total: u64) {
        self.emit(ProgressKind::SetTotal(total));
    }

    /// Advance completed work by `delta` units.
    pub fn advance(&self, delta: u64) {
        self.emit(ProgressKind::Advance(delta));
    }

    /// Emit a human-readable status message.
    pub fn message(&self, msg: impl Into<String>) {
        self.emit(ProgressKind::Message(msg.into()));
    }

    /// Mark this node finished. No further events should be emitted.
    pub fn finish(&self) {
        self.emit(ProgressKind::Finished);
    }

    fn emit(&self, kind: ProgressKind) {
        self.reporter.sink.event(ProgressEvent {
            id: self.id,
            parent: None,
            name: self.name.clone(),
            kind,
        });
    }
}
