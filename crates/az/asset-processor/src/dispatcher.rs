//! Demand-shaped asset-job dispatch for the Asset Processor RPC `LocalSet`.
//!
//! One dispatcher owns every parked lease call and live grant. Database status
//! revisions and source-reconcile readiness are wake hints; the database claim
//! remains the durable authority. Worker renewals move only the process-local
//! monotonic deadline, while expiry is fenced by
//! `(attempt, owner, connection_id, grant_key)`.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::pending;
use std::num::NonZeroU64;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::time::{Duration, Instant};

use az_assetdb::{
    AbandonAttempts, AssetProcessorQueue, AttemptFence, ClaimReadyJob, ClaimReadyJobResult,
    ResolveIdleBlocked, SelectJobs, Status as DbStatus, Work as DbWork,
};
use az_proto_asset::{
    AssetProcessorEventKind, CompleteAssetJobAttemptRequest, LeaseAssetJobResult, LeasedAssetJob,
};
use az_proto_core::Capability;
use az_work::CancellationToken;
use futures::{
    FutureExt,
    future::{AbortHandle, Abortable},
    stream::{FuturesUnordered, StreamExt},
};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

#[cfg(test)]
use crate::PostCommitAttemptCompletion;
use crate::{
    AssetProcessor, AssetProcessorConsequenceFault, AssetProcessorError,
    AssetProcessorEventPublisher, AssetSourceServiceCoordination, DurableAttemptCompletion,
    LeasedAssetJobPreparation, MAX_ASSET_JOB_LEASE_DURATION_MS, PRIORITIZED_ASSET_LEASE_WINDOW,
    PreparedAttemptCompletion, invalid_worker_job_request, prepare_leased_asset_job_attempt,
};

type LeaseOutcome = Result<LeaseAssetJobResult, AssetProcessorError>;
type StagingOutcome = (u64, Result<LeasedAssetJob, AssetProcessorError>);
type StagingFuture = Pin<Box<dyn std::future::Future<Output = StagingOutcome>>>;
type CompletionFuture = Pin<Box<dyn std::future::Future<Output = CompletionWorkResult>>>;
const EXPIRATION_RETRY_DELAY: Duration = Duration::from_secs(1);
const DISPATCHER_COMMAND_CAPACITY: usize = 256;

#[derive(Debug, Clone)]
pub struct LeaseRequest {
    owner: String,
    duration: LeaseDuration,
    staging_root: PathBuf,
}

#[derive(Debug, Clone, Copy)]
struct LeaseDuration(NonZeroU64);

impl LeaseDuration {
    fn from_millis(milliseconds: u64) -> Result<Self, AssetProcessorError> {
        let value = NonZeroU64::new(milliseconds)
            .filter(|value| value.get() <= MAX_ASSET_JOB_LEASE_DURATION_MS);
        value.map(Self).ok_or_else(|| {
            invalid_worker_job_request(format!(
                "lease duration must be between 1 and {MAX_ASSET_JOB_LEASE_DURATION_MS} milliseconds"
            ))
        })
    }

    const fn duration(self) -> Duration {
        Duration::from_millis(self.0.get())
    }

    const fn milliseconds(self) -> u64 {
        self.0.get()
    }
}

#[derive(Debug, Clone)]
pub struct PayloadAuthority(Capability);

#[derive(Debug, Clone)]
pub struct LeaseEnvelope {
    request: LeaseRequest,
    payload_authority: PayloadAuthority,
}

impl LeaseRequest {
    pub(crate) fn validated(
        owner: String,
        duration_ms: u64,
        staging_root: PathBuf,
    ) -> Result<Self, AssetProcessorError> {
        Ok(Self {
            owner,
            duration: LeaseDuration::from_millis(duration_ms)?,
            staging_root,
        })
    }

    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }

    pub(crate) fn staging_root(&self) -> &Path {
        &self.staging_root
    }

    pub(crate) const fn duration_ms(&self) -> u64 {
        self.duration.milliseconds()
    }
}

impl PayloadAuthority {
    pub(crate) const fn validated(capability: Capability) -> Self {
        Self(capability)
    }

    pub(crate) const fn capability(&self) -> &Capability {
        &self.0
    }
}

impl LeaseEnvelope {
    pub(crate) const fn new(request: LeaseRequest, payload_authority: PayloadAuthority) -> Self {
        Self {
            request,
            payload_authority,
        }
    }

    pub(crate) const fn request(&self) -> &LeaseRequest {
        &self.request
    }

    pub(crate) const fn payload_authority(&self) -> &PayloadAuthority {
        &self.payload_authority
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantIdentity {
    attempt_id: i64,
    owner: String,
    connection_id: Uuid,
    key: Uuid,
}

impl GrantIdentity {
    pub(crate) const fn new(
        attempt_id: i64,
        owner: String,
        connection_id: Uuid,
        key: Uuid,
    ) -> Self {
        Self {
            attempt_id,
            owner,
            connection_id,
            key,
        }
    }
}

#[derive(Clone)]
pub struct AssetJobDispatcher {
    inner: Rc<DispatcherHandle>,
}

struct DispatcherHandle {
    commands: mpsc::Sender<DispatcherCommand>,
    next_waiter_id: Cell<u64>,
    connection_cancellations: RefCell<BTreeMap<Uuid, CancellationToken>>,
    wake: Rc<Notify>,
}

pub struct AssetJobDispatcherOwner {
    dispatcher: AssetJobDispatcher,
    shutdown: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl AssetJobDispatcherOwner {
    pub(crate) fn start(
        processor: Rc<AssetProcessor>,
        source_coordination: AssetSourceServiceCoordination,
        event_publisher: AssetProcessorEventPublisher,
    ) -> Self {
        let (commands, receiver) = mpsc::channel(DISPATCHER_COMMAND_CAPACITY);
        let wake = Rc::new(Notify::new());
        let shutdown = CancellationToken::new();

        Self {
            dispatcher: AssetJobDispatcher {
                inner: Rc::new(DispatcherHandle {
                    commands,
                    next_waiter_id: Cell::new(1),
                    connection_cancellations: RefCell::new(BTreeMap::new()),
                    wake: Rc::clone(&wake),
                }),
            },
            shutdown: shutdown.clone(),
            task: Some(tokio::task::spawn_local(run_dispatcher(
                processor,
                source_coordination,
                receiver,
                wake,
                shutdown,
                event_publisher,
            ))),
        }
    }

    pub(crate) fn dispatcher(&self) -> AssetJobDispatcher {
        self.dispatcher.clone()
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    pub(crate) async fn shutdown(mut self) -> Result<(), AssetProcessorError> {
        self.shutdown.cancel();
        self.dispatcher.inner.wake.notify_one();
        let Some(task) = self.task.take() else {
            return Ok(());
        };
        task.await
            .map_err(|error| AssetProcessorError::DispatcherTask { error })
    }
}

impl Drop for AssetJobDispatcherOwner {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.dispatcher.inner.wake.notify_one();
        if let Some(task) = self.task.take() {
            // Dropping a Tokio JoinHandle detaches the task. The shutdown
            // token makes the owner stop admitting work, then its own loop
            // drains any irreversible completion before exiting.
            drop(task);
        }
    }
}

impl AssetJobDispatcher {
    fn connection_cancellation(&self, connection_id: Uuid) -> CancellationToken {
        self.inner
            .connection_cancellations
            .borrow_mut()
            .entry(connection_id)
            .or_default()
            .clone()
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    pub(crate) async fn lease(
        &self,
        connection_id: Uuid,
        envelope: LeaseEnvelope,
    ) -> Result<LeaseAssetJobResult, AssetProcessorError> {
        let waiter_id = self.inner.next_waiter_id.get();
        self.inner.next_waiter_id.set(
            waiter_id
                .checked_add(1)
                .expect("asset lease waiter id overflow"),
        );
        let (result, receiver) = oneshot::channel();
        let cancellation = CancellationToken::new();
        let connection_cancellation = self.connection_cancellation(connection_id);
        self.inner
            .commands
            .send(DispatcherCommand::Park {
                waiter_id,
                connection_id,
                envelope,
                result,
                cancellation: cancellation.clone(),
                connection_cancellation,
            })
            .await
            .map_err(|_| dispatcher_stopped())?;
        let mut cancellation = WaiterCancellation {
            cancellation,
            wake: Rc::clone(&self.inner.wake),
            armed: true,
        };
        let result = receiver.await.map_err(|_| dispatcher_stopped())?;
        cancellation.armed = false;
        result
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    pub(crate) async fn renew(&self, identity: GrantIdentity) -> Result<bool, AssetProcessorError> {
        let (result, receiver) = oneshot::channel();
        self.inner
            .commands
            .send(DispatcherCommand::Renew { identity, result })
            .await
            .map_err(|_| dispatcher_stopped())?;
        receiver.await.map_err(|_| dispatcher_stopped())?
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    pub(crate) async fn complete(
        &self,
        identity: GrantIdentity,
        request: CompleteAssetJobAttemptRequest,
    ) -> Result<bool, AssetProcessorError> {
        let (result, receiver) = oneshot::channel();
        self.inner
            .commands
            .send(DispatcherCommand::Complete {
                identity,
                request: Box::new(request),
                result,
            })
            .await
            .map_err(|_| dispatcher_stopped())?;
        receiver.await.map_err(|_| dispatcher_stopped())?
    }

    pub(crate) fn disconnect_connection(&self, connection_id: Uuid) {
        if let Some(cancellation) = self
            .inner
            .connection_cancellations
            .borrow_mut()
            .remove(&connection_id)
        {
            cancellation.cancel();
            self.inner.wake.notify_one();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionReservation {
    identity: GrantIdentity,
}

enum CompletionWorkResult {
    Prepared {
        reservation: CompletionReservation,
        event_unix_ms: i64,
        preparation: PreparationEnd,
    },
    Committed {
        reservation: CompletionReservation,
        event_unix_ms: i64,
        completion: CommitEnd,
    },
    PostCommitted {
        attempt_id: i64,
        completion: PostCommitEnd,
    },
}

enum PreparationEnd {
    Finished(Result<PreparedAttemptCompletion, AssetProcessorError>),
    Panicked,
    Aborted,
}

enum CommitEnd {
    Finished(Result<DurableAttemptCompletion, AssetProcessorError>),
    Panicked,
}

enum PostCommitEnd {
    Finished(Result<(), AssetProcessorError>),
    Panicked,
}

struct CompletionControl {
    abort: AbortHandle,
}

struct WaiterCancellation {
    cancellation: CancellationToken,
    wake: Rc<Notify>,
    armed: bool,
}

impl Drop for WaiterCancellation {
    fn drop(&mut self) {
        if self.armed {
            self.cancellation.cancel();
            self.wake.notify_one();
        }
    }
}

enum DispatcherCommand {
    Park {
        waiter_id: u64,
        connection_id: Uuid,
        envelope: LeaseEnvelope,
        result: oneshot::Sender<LeaseOutcome>,
        cancellation: CancellationToken,
        connection_cancellation: CancellationToken,
    },
    Renew {
        identity: GrantIdentity,
        result: oneshot::Sender<Result<bool, AssetProcessorError>>,
    },
    Complete {
        identity: GrantIdentity,
        // Carries an inline product-manifest side channel and its capability,
        // which made this the command channel's largest variant by several
        // hundred bytes -- the size every queued `Park` and `Renew` paid too.
        request: Box<CompleteAssetJobAttemptRequest>,
        result: oneshot::Sender<Result<bool, AssetProcessorError>>,
    },
}

struct LeaseWaiter {
    connection_id: Uuid,
    envelope: LeaseEnvelope,
    result: oneshot::Sender<LeaseOutcome>,
    cancellation: CancellationToken,
    connection_cancellation: CancellationToken,
    phase: WaiterPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WaiterPhase {
    Waiting,
    Staging { identity: GrantIdentity },
}

struct LiveGrant {
    identity: GrantIdentity,
    lease_duration: Duration,
    connection_cancellation: CancellationToken,
    phase: GrantPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrantPhase {
    Staging { deadline: Instant },
    Active { deadline: Instant },
    CompletionPreparing { deadline: Instant },
    CompletionIrreversible,
    Expiring { retry_at: Instant },
}

impl LiveGrant {
    fn renew(&mut self, now: Instant) -> bool {
        match &mut self.phase {
            GrantPhase::Active { deadline } if *deadline > now => {
                *deadline = now + self.lease_duration;
                true
            }
            _ => false,
        }
    }

    fn arm_expiration_retry(&mut self, now: Instant) {
        self.phase = GrantPhase::Expiring {
            retry_at: now + EXPIRATION_RETRY_DELAY,
        };
    }

    const fn deadline(&self) -> Option<Instant> {
        match self.phase {
            GrantPhase::Active { deadline }
            | GrantPhase::Expiring { retry_at: deadline }
            | GrantPhase::Staging { deadline }
            | GrantPhase::CompletionPreparing { deadline } => Some(deadline),
            GrantPhase::CompletionIrreversible => None,
        }
    }

    const fn can_expire(&self) -> bool {
        !matches!(
            self.phase,
            GrantPhase::CompletionPreparing { .. } | GrantPhase::CompletionIrreversible
        )
    }
}

struct DispatcherState {
    waiters: BTreeMap<u64, LeaseWaiter>,
    waiting: VecDeque<u64>,
    grants: BTreeMap<i64, LiveGrant>,
    completion_results: BTreeMap<i64, oneshot::Sender<Result<bool, AssetProcessorError>>>,
    completion_controls: BTreeMap<i64, CompletionControl>,
    ready: ReadyWindow,
    idle: IdleResolution,
}

#[derive(Debug, Default)]
struct ReadyWindow {
    jobs: VecDeque<SelectJobs>,
    prioritized_after_asset_pk: Option<i64>,
    prioritized_exhausted: bool,
    plan_after_job_id: i64,
    plan_exhausted: bool,
    build_after_job_id: i64,
    build_exhausted: bool,
}

enum ReadyRefill {
    Ready,
    MorePriority,
    Exhausted,
}

#[derive(Debug, Default)]
struct IdleResolution {
    after_job_id: i64,
    stable: bool,
}

enum IdlePageOutcome {
    Progress,
    More { after_job_id: i64 },
    Stable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionDisposition {
    Unknown,
    Consumed,
    Expire,
}

fn finish_completion_disposition(
    state: &mut DispatcherState,
    reservation: &CompletionReservation,
    committed: bool,
) -> CompletionDisposition {
    let Some(grant) = state.grants.get_mut(&reservation.identity.attempt_id) else {
        return CompletionDisposition::Unknown;
    };
    if grant.identity != reservation.identity {
        return CompletionDisposition::Unknown;
    }
    if committed {
        state.grants.remove(&reservation.identity.attempt_id);
        CompletionDisposition::Consumed
    } else {
        match grant.phase {
            GrantPhase::CompletionPreparing { .. } | GrantPhase::CompletionIrreversible => {
                grant.phase = GrantPhase::Expiring {
                    retry_at: Instant::now(),
                };
                CompletionDisposition::Expire
            }
            _ => CompletionDisposition::Unknown,
        }
    }
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn run_dispatcher(
    processor: Rc<AssetProcessor>,
    source_coordination: AssetSourceServiceCoordination,
    mut commands: mpsc::Receiver<DispatcherCommand>,
    wake: Rc<Notify>,
    shutdown: CancellationToken,
    event_publisher: AssetProcessorEventPublisher,
) {
    let mut state = DispatcherState {
        waiters: BTreeMap::new(),
        waiting: VecDeque::new(),
        grants: BTreeMap::new(),
        completion_results: BTreeMap::new(),
        completion_controls: BTreeMap::new(),
        ready: ReadyWindow::default(),
        idle: IdleResolution::default(),
    };
    let mut stagings = FuturesUnordered::<StagingFuture>::new();
    let mut completions = FuturesUnordered::<CompletionFuture>::new();
    let mut status_changes = processor.processing_status_subscription();
    let mut sweep_changes = source_coordination
        .sweep_handle()
        .map(|sweep| sweep.subscribe());

    // A processor restart invalidates every worker connection and therefore
    // every durable lease from the previous process. This is the one blanket
    // recovery; later expiry is always targeted by a live GrantKey.
    if let Err(error) = processor.recover_expired_leases_at_startup().await {
        run_failed_dispatcher(&mut commands, error, &shutdown).await;
        return;
    }
    loop {
        dispatch_ready_waiters(
            &processor,
            &source_coordination,
            &wake,
            &mut state,
            &stagings,
        )
        .await;

        let deadline = state.grants.values().filter_map(LiveGrant::deadline).min();
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    shutdown_dispatcher(&processor, &event_publisher, &mut state, &mut stagings, &mut completions).await;
                    return;
                };
                handle_command(&processor, &mut state, &completions, command);
            }
            staging = stagings.next(), if !stagings.is_empty() => {
                if let Some((waiter_id, result)) = staging {
                    staging_finished(&processor, &mut state, waiter_id, result).await;
                }
            }
            completion = completions.next(), if !completions.is_empty() => {
                if let Some(completion) = completion {
                    completion_finished(
                        &processor,
                        &event_publisher,
                        &mut state,
                        &completions,
                        completion,
                    )
                    .await;
                }
            }
            changed = status_changes.changed() => {
                if !changed {
                    shutdown_dispatcher(&processor, &event_publisher, &mut state, &mut stagings, &mut completions).await;
                    return;
                }
                state.ready = ReadyWindow::default();
                state.idle = IdleResolution::default();
            }
            changed = wait_for_sweep_change(&mut sweep_changes) => {
                if !changed {
                    shutdown_dispatcher(&processor, &event_publisher, &mut state, &mut stagings, &mut completions).await;
                    return;
                }
                if let Some(sweep) = source_coordination.sweep_handle() {
                    processor
                        .prioritized_asset_identities
                        .borrow_mut()
                        .extend(sweep.take_priority());
                }
                state.ready = ReadyWindow::default();
                state.idle = IdleResolution::default();
            }
            () = wait_for_deadline(deadline) => {
                expire_due_grants(&processor, &mut state).await;
            }
            () = wake.notified() => {
                cancel_requested_work(&processor, &mut state).await;
            }
            () = shutdown.cancelled() => {
                shutdown_dispatcher(&processor, &event_publisher, &mut state, &mut stagings, &mut completions).await;
                return;
            }
        }
    }
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn dispatch_ready_waiters(
    processor: &AssetProcessor,
    source_coordination: &AssetSourceServiceCoordination,
    wake: &Notify,
    state: &mut DispatcherState,
    stagings: &FuturesUnordered<StagingFuture>,
) {
    loop {
        prune_cancelled_waiting(state);
        let Some(waiter_id) = state.waiting.front().copied() else {
            return;
        };
        let Some(waiter) = state.waiters.get(&waiter_id) else {
            state.waiting.pop_front();
            continue;
        };
        if waiter.result.is_closed()
            || waiter.cancellation.is_cancelled()
            || waiter.connection_cancellation.is_cancelled()
        {
            state.waiting.pop_front();
            state.waiters.remove(&waiter_id);
            continue;
        }
        let envelope = waiter.envelope.clone();
        let request = &envelope.request;
        let connection_id = waiter.connection_id;
        let duration = request.duration.duration();
        let connection_cancellation = waiter.connection_cancellation.clone();

        // Recheck closure immediately before the durable claim. The writer's
        // expected-state fence still wins races with planning/retirement.
        if state.waiters.get(&waiter_id).is_none_or(|waiter| {
            waiter.result.is_closed()
                || waiter.cancellation.is_cancelled()
                || waiter.connection_cancellation.is_cancelled()
        }) {
            state.waiting.pop_front();
            state.waiters.remove(&waiter_id);
            continue;
        }
        if state.ready.jobs.is_empty() {
            match refill_or_resolve_idle(processor, source_coordination, wake, state, waiter_id)
                .await
            {
                WaiterStep::Dispatch => {}
                WaiterStep::Yield => return,
                WaiterStep::NextWaiter => continue,
            }
        }
        let job = state
            .ready
            .jobs
            .pop_front()
            .expect("ready refill returned Ready without a candidate");
        match claim_ready_candidate(processor, &envelope, job).await {
            Ok(Some(claimed)) => {
                state.waiting.pop_front();
                let attempt_id = claimed.preparation.asset_job_attempt_id();
                let grant_key = Uuid::now_v7();
                let identity =
                    GrantIdentity::new(attempt_id, request.owner.clone(), connection_id, grant_key);
                state.grants.insert(
                    attempt_id,
                    LiveGrant {
                        identity: identity.clone(),
                        lease_duration: duration,
                        connection_cancellation,
                        phase: GrantPhase::Staging {
                            deadline: claimed.deadline,
                        },
                    },
                );
                if let Some(waiter) = state.waiters.get_mut(&waiter_id) {
                    waiter.phase = WaiterPhase::Staging { identity };
                }
                if state.waiters.get(&waiter_id).is_none_or(|waiter| {
                    waiter.result.is_closed()
                        || waiter.cancellation.is_cancelled()
                        || waiter.connection_cancellation.is_cancelled()
                }) {
                    cancel_waiter(processor, state, waiter_id).await;
                    continue;
                }
                stagings.push(Box::pin(async move {
                    (waiter_id, claimed.preparation.stage().await)
                }));
            }
            Ok(None) => {}
            Err(error) => {
                finish_waiter(state, waiter_id, Err(error));
            }
        }
    }
}

/// What the waiter loop does next after trying to fill its ready window.
enum WaiterStep {
    /// The window has candidates; dispatch the waiter at the front.
    Dispatch,
    /// Nothing left to do on this wake; hand control back to the loop.
    Yield,
    /// This waiter is finished; move on to the next one.
    NextWaiter,
}

/// Refills the ready window and, when there is nothing ready and nothing in
/// flight, spends this wake resolving one page of idle-blocked jobs instead.
///
/// The idle pass only runs with no live grants: a grant that is still out can
/// still make jobs ready on its own, so declaring the queue idle underneath it
/// would be wrong.
// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn refill_or_resolve_idle(
    processor: &AssetProcessor,
    source_coordination: &AssetSourceServiceCoordination,
    wake: &Notify,
    state: &mut DispatcherState,
    waiter_id: u64,
) -> WaiterStep {
    let demand = state.waiting.len();
    match refill_ready_window(processor, source_coordination, &mut state.ready, demand) {
        Ok(ReadyRefill::Ready) => WaiterStep::Dispatch,
        Ok(ReadyRefill::MorePriority) => {
            wake.notify_one();
            WaiterStep::Yield
        }
        Ok(ReadyRefill::Exhausted) => {
            if !state.grants.is_empty() || state.idle.stable {
                return WaiterStep::Yield;
            }
            match resolve_idle_blocked_page(processor, state.idle.after_job_id).await {
                Ok(IdlePageOutcome::Progress) => {
                    state.ready = ReadyWindow::default();
                    state.idle = IdleResolution::default();
                    wake.notify_one();
                    WaiterStep::Yield
                }
                Ok(IdlePageOutcome::More { after_job_id }) => {
                    state.idle.after_job_id = after_job_id;
                    wake.notify_one();
                    WaiterStep::Yield
                }
                Ok(IdlePageOutcome::Stable) => {
                    state.idle.stable = true;
                    WaiterStep::Yield
                }
                Err(error) => {
                    finish_waiter(state, waiter_id, Err(error));
                    WaiterStep::NextWaiter
                }
            }
        }
        Err(error) => {
            finish_waiter(state, waiter_id, Err(error));
            WaiterStep::NextWaiter
        }
    }
}

fn prune_cancelled_waiting(state: &mut DispatcherState) {
    let cancelled = state
        .waiting
        .iter()
        .copied()
        .filter(|waiter_id| {
            state.waiters.get(waiter_id).is_none_or(|waiter| {
                waiter.result.is_closed()
                    || waiter.cancellation.is_cancelled()
                    || waiter.connection_cancellation.is_cancelled()
                    || !matches!(waiter.phase, WaiterPhase::Waiting)
            })
        })
        .collect::<BTreeSet<_>>();
    if !cancelled.is_empty() {
        state
            .waiting
            .retain(|waiter_id| !cancelled.contains(waiter_id));
        for waiter_id in cancelled {
            state.waiters.remove(&waiter_id);
        }
    }
    let demand = state.waiting.len();
    if demand == 0 || state.ready.jobs.len() > demand {
        state.ready = ReadyWindow::default();
    }
}

fn refill_ready_window(
    processor: &AssetProcessor,
    source_coordination: &AssetSourceServiceCoordination,
    window: &mut ReadyWindow,
    demand: usize,
) -> Result<ReadyRefill, AssetProcessorError> {
    if !window.jobs.is_empty() {
        return Ok(ReadyRefill::Ready);
    }
    if demand == 0 {
        return Ok(ReadyRefill::Exhausted);
    }
    let db = processor.dispatch_db();
    let workspace_pk = processor.attached_workspace_id()?;

    if let Some(refill) = refill_prioritized_window(
        &db,
        processor,
        source_coordination,
        workspace_pk,
        window,
        demand,
    )? {
        return Ok(refill);
    }

    let demand_u32 =
        u32::try_from(demand - window.jobs.len()).expect("parked asset-job demand exceeds u32");
    if demand_u32 != 0 && !window.plan_exhausted {
        let jobs = db.ready_jobs(
            workspace_pk,
            DbWork::Plan,
            window.plan_after_job_id,
            demand_u32,
        )?;
        if let Some(last) = jobs.last() {
            window.plan_after_job_id = last.job_id;
        }
        window.plan_exhausted = jobs.len() < demand_u32 as usize;
        append_unique_ready_jobs(
            &mut window.jobs,
            jobs.into_iter()
                .filter(|job| job_root_is_admitted(&db, source_coordination, job).unwrap_or(false)),
            demand,
        );
    }
    let remaining = demand - window.jobs.len();
    if remaining != 0 && !window.build_exhausted {
        let limit = u32::try_from(remaining).expect("parked asset-job demand exceeds u32");
        let jobs = db.ready_jobs(
            workspace_pk,
            DbWork::Build,
            window.build_after_job_id,
            limit,
        )?;
        if let Some(last) = jobs.last() {
            window.build_after_job_id = last.job_id;
        }
        window.build_exhausted = jobs.len() < limit as usize;
        append_unique_ready_jobs(
            &mut window.jobs,
            jobs.into_iter()
                .filter(|job| job_root_is_admitted(&db, source_coordination, job).unwrap_or(false)),
            demand,
        );
    }
    drop(db);
    Ok(if window.jobs.is_empty() {
        ReadyRefill::Exhausted
    } else {
        ReadyRefill::Ready
    })
}

/// Fills `window` from the asset identities the processor is currently
/// prioritizing, one bounded page at a time.
///
/// Returns the refill result when the prioritized page settles the call, and
/// `None` when the caller should fall through to the ordinary ready queues.
fn refill_prioritized_window(
    db: &az_assetdb::AssetDb,
    processor: &AssetProcessor,
    source_coordination: &AssetSourceServiceCoordination,
    workspace_pk: i64,
    window: &mut ReadyWindow,
    demand: usize,
) -> Result<Option<ReadyRefill>, AssetProcessorError> {
    if window.prioritized_exhausted {
        return Ok(None);
    }
    let identities = processor.prioritized_asset_identities.borrow();
    let page = identities
        .iter()
        .copied()
        .filter(|asset_pk| {
            window
                .prioritized_after_asset_pk
                .is_none_or(|after| *asset_pk > after)
        })
        .take(PRIORITIZED_ASSET_LEASE_WINDOW)
        .collect::<Vec<_>>();
    drop(identities);
    for asset_pk in &page {
        window.prioritized_after_asset_pk = Some(*asset_pk);
        let mut jobs = db
            .jobs_for_asset(workspace_pk, *asset_pk)?
            .into_iter()
            .filter(|job| {
                job.status == DbStatus::Queued
                    && job.ready
                    && job_root_is_admitted(db, source_coordination, job).unwrap_or(false)
            })
            .collect::<Vec<_>>();
        jobs.sort_by_key(|job| {
            (
                match job.kind {
                    DbWork::Plan => 0_u8,
                    DbWork::Build => 1_u8,
                },
                job.job_id,
            )
        });
        for job in jobs {
            if window.jobs.len() == demand {
                break;
            }
            window.jobs.push_back(job);
        }
        if window.jobs.len() == demand {
            return Ok(Some(ReadyRefill::Ready));
        }
    }
    if page.len() == PRIORITIZED_ASSET_LEASE_WINDOW {
        return Ok(Some(if window.jobs.is_empty() {
            ReadyRefill::MorePriority
        } else {
            ReadyRefill::Ready
        }));
    }
    window.prioritized_exhausted = true;
    Ok(None)
}

fn append_unique_ready_jobs(
    window: &mut VecDeque<SelectJobs>,
    jobs: impl IntoIterator<Item = SelectJobs>,
    demand: usize,
) {
    for job in jobs {
        if window.len() == demand {
            return;
        }
        if !window
            .iter()
            .any(|candidate| candidate.job_id == job.job_id)
        {
            window.push_back(job);
        }
    }
}

fn job_root_is_admitted(
    db: &az_assetdb::AssetDb,
    source_coordination: &AssetSourceServiceCoordination,
    job: &SelectJobs,
) -> Result<bool, AssetProcessorError> {
    let Some(sweep) = source_coordination.sweep_handle() else {
        return Ok(true);
    };
    let entry = db.entry_by_asset(job.workspace_pk, job.asset_pk)?;
    Ok(entry.is_some_and(|entry| sweep.root_is_admitted(entry.root_pk)))
}

async fn wait_for_sweep_change(changes: &mut Option<tokio::sync::watch::Receiver<u64>>) -> bool {
    match changes {
        Some(changes) => changes.changed().await.is_ok(),
        None => pending::<bool>().await,
    }
}

struct ClaimedLeasePreparation {
    preparation: LeasedAssetJobPreparation,
    deadline: Instant,
}

fn lease_deadline_from_writer_claim(claimed_at: Instant, duration: Duration) -> Instant {
    claimed_at + duration
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn claim_ready_candidate(
    processor: &AssetProcessor,
    envelope: &LeaseEnvelope,
    job: SelectJobs,
) -> Result<Option<ClaimedLeasePreparation>, AssetProcessorError> {
    let request = envelope.request();
    let queue =
        AssetProcessorQueue::new(processor.dispatch_db(), processor.asset_db_writer().clone());
    let claimed = queue
        .claim(ClaimReadyJob {
            job_id: job.job_id,
            expected_attempts: job.attempts,
            owner: request.owner().to_owned(),
            lease_duration_ms: request.duration_ms(),
            staging: request.staging_root().to_string_lossy().into_owned(),
        })
        .await?;
    let ClaimReadyJobResult::Claimed { context } = claimed else {
        return Ok(None);
    };
    let attempt_id = context.attempt.attempt_id;
    let claimed_at = context.claimed_at;
    match (|| {
        let preparation = prepare_leased_asset_job_attempt(*context, envelope)?;
        let deadline = lease_deadline_from_writer_claim(claimed_at, request.duration.duration());
        Ok(ClaimedLeasePreparation {
            preparation,
            deadline,
        })
    })() {
        Ok(preparation) => {
            drop(queue);
            Ok(Some(preparation))
        }
        Err(error) => {
            let result = queue
                .abandon(AbandonAttempts {
                    attempts: vec![AttemptFence {
                        attempt_id,
                        owner: request.owner().to_owned(),
                    }],
                    finished: AssetProcessor::current_unix_ms()?,
                })
                .await?;
            drop(queue);
            tracing::warn!(
                attempt_id,
                requeued = result.requeued.len(),
                exhausted = result.exhausted.len(),
                error = %error,
                "asset processor abandoned a claim whose payload could not be staged"
            );
            Err(error)
        }
    }
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn resolve_idle_blocked_page(
    processor: &AssetProcessor,
    after_job_id: i64,
) -> Result<IdlePageOutcome, AssetProcessorError> {
    const PAGE_SIZE: u32 = 64;
    let workspace_pk = processor.attached_workspace_id()?;
    let db = processor.dispatch_db();
    let jobs = db.blocked_jobs_page(workspace_pk, after_job_id, PAGE_SIZE)?;
    drop(db);
    let Some(last_job_id) = jobs.last().map(|job| job.job_id) else {
        return Ok(IdlePageOutcome::Stable);
    };
    let result = processor
        .asset_db_writer()
        .resolve_idle_blocked(ResolveIdleBlocked {
            workspace_pk,
            job_ids: jobs.into_iter().map(|job| job.job_id).collect(),
        })
        .await?;
    let made_progress = !result.dropped_order_only_edges.is_empty()
        || !result.failed_jobs.is_empty()
        || !result.became_ready.is_empty();
    Ok(if made_progress {
        IdlePageOutcome::Progress
    } else {
        IdlePageOutcome::More {
            after_job_id: last_job_id,
        }
    })
}

fn handle_command(
    processor: &Rc<AssetProcessor>,
    state: &mut DispatcherState,
    completions: &FuturesUnordered<CompletionFuture>,
    command: DispatcherCommand,
) {
    match command {
        DispatcherCommand::Park {
            waiter_id,
            connection_id,
            envelope,
            result,
            cancellation,
            connection_cancellation,
        } => {
            if cancellation.is_cancelled() || connection_cancellation.is_cancelled() {
                let _ = result.send(Err(dispatcher_stopped()));
                return;
            }
            state.waiters.insert(
                waiter_id,
                LeaseWaiter {
                    connection_id,
                    envelope,
                    result,
                    cancellation,
                    connection_cancellation,
                    phase: WaiterPhase::Waiting,
                },
            );
            state.waiting.push_back(waiter_id);
        }
        DispatcherCommand::Renew { identity, result } => {
            let renewed = state
                .grants
                .get_mut(&identity.attempt_id)
                .filter(|grant| grant.identity == identity)
                .is_some_and(|grant| grant.renew(Instant::now()));
            let _ = result.send(Ok(renewed));
        }
        DispatcherCommand::Complete {
            identity,
            request,
            result,
        } => {
            let reservation = state
                .grants
                .get_mut(&identity.attempt_id)
                .and_then(|grant| {
                    if grant.identity != identity {
                        return None;
                    }
                    let GrantPhase::Active { deadline } = grant.phase else {
                        return None;
                    };
                    if deadline <= Instant::now() {
                        return None;
                    }
                    grant.phase = GrantPhase::CompletionPreparing { deadline };
                    Some(CompletionReservation {
                        identity: identity.clone(),
                    })
                });
            let Some(reservation) = reservation else {
                let _ = result.send(Err(invalid_worker_job_request(
                    "completion does not match an active dispatcher grant",
                )));
                return;
            };
            state.completion_results.insert(identity.attempt_id, result);
            let (abort, registration) = AbortHandle::new_pair();
            state
                .completion_controls
                .insert(identity.attempt_id, CompletionControl { abort });
            let processor = Rc::clone(processor);
            let event_unix_ms = request.finished_unix_ms;
            completions.push(Box::pin(async move {
                let completion = Abortable::new(
                    AssertUnwindSafe(processor.prepare_attempt_completion(&request)).catch_unwind(),
                    registration,
                )
                .await;
                let preparation = match completion {
                    Ok(Ok(result)) => PreparationEnd::Finished(result),
                    Ok(Err(_)) => PreparationEnd::Panicked,
                    Err(_) => PreparationEnd::Aborted,
                };
                CompletionWorkResult::Prepared {
                    reservation,
                    event_unix_ms,
                    preparation,
                }
            }));
        }
    }
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn cancel_requested_work(processor: &AssetProcessor, state: &mut DispatcherState) {
    let waiter_ids = state
        .waiters
        .iter()
        .filter_map(|(waiter_id, waiter)| {
            (waiter.cancellation.is_cancelled()
                || waiter.connection_cancellation.is_cancelled()
                || waiter.result.is_closed())
            .then_some(*waiter_id)
        })
        .collect::<Vec<_>>();
    for waiter_id in waiter_ids {
        cancel_waiter(processor, state, waiter_id).await;
    }

    let preparing = state
        .grants
        .values()
        .filter(|grant| {
            grant.connection_cancellation.is_cancelled()
                && matches!(grant.phase, GrantPhase::CompletionPreparing { .. })
        })
        .map(|grant| grant.identity.clone())
        .collect::<Vec<_>>();
    for identity in preparing {
        if timeout_completion(state, &identity) {
            expire_grant(processor, state, &identity).await;
        }
    }
    let grants = state
        .grants
        .values()
        .filter_map(|grant| {
            (grant.connection_cancellation.is_cancelled() && grant.can_expire())
                .then_some(grant.identity.clone())
        })
        .collect::<Vec<_>>();
    for identity in grants {
        expire_grant(processor, state, &identity).await;
    }
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn completion_finished(
    processor: &Rc<AssetProcessor>,
    event_publisher: &AssetProcessorEventPublisher,
    state: &mut DispatcherState,
    completions: &FuturesUnordered<CompletionFuture>,
    completed: CompletionWorkResult,
) {
    match completed {
        CompletionWorkResult::Prepared {
            reservation,
            event_unix_ms,
            preparation,
        } => {
            commit_prepared_completion(
                processor,
                state,
                completions,
                reservation,
                event_unix_ms,
                preparation,
            )
            .await;
        }
        CompletionWorkResult::Committed {
            reservation,
            event_unix_ms,
            completion,
        } => {
            consume_commit_end(
                processor,
                event_publisher,
                state,
                completions,
                &reservation,
                event_unix_ms,
                completion,
            )
            .await;
        }
        CompletionWorkResult::PostCommitted {
            attempt_id,
            completion,
        } => finish_post_commit(event_publisher, state, attempt_id, completion),
    }
}

/// Takes one prepared completion irreversible and queues its durable commit.
///
/// A preparation that did not finish cleanly ends the completion here; a
/// preparation that lost its grant window is expired instead of committed, so
/// nothing irreversible starts without the grant still being ours.
// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn commit_prepared_completion(
    processor: &Rc<AssetProcessor>,
    state: &mut DispatcherState,
    completions: &FuturesUnordered<CompletionFuture>,
    reservation: CompletionReservation,
    event_unix_ms: i64,
    preparation: PreparationEnd,
) {
    let identity = reservation.identity.clone();
    state.completion_controls.remove(&identity.attempt_id);
    let prepared = match preparation {
        PreparationEnd::Finished(Ok(prepared)) => prepared,
        PreparationEnd::Finished(Err(error)) => {
            finish_completion(processor, state, &reservation, Err(error)).await;
            return;
        }
        PreparationEnd::Panicked => {
            finish_completion(
                processor,
                state,
                &reservation,
                Err(AssetProcessorError::DispatcherCompletionPanicked {
                    attempt_id: identity.attempt_id,
                }),
            )
            .await;
            return;
        }
        PreparationEnd::Aborted => {
            finish_completion(
                processor,
                state,
                &reservation,
                Err(AssetProcessorError::DispatcherCompletionTimeout {
                    attempt_id: identity.attempt_id,
                }),
            )
            .await;
            return;
        }
    };
    let can_commit = begin_irreversible_completion(state, &identity, Instant::now());
    if !can_commit {
        if timeout_completion(state, &identity) {
            expire_grant(processor, state, &identity).await;
        }
        return;
    }
    let processor = Rc::clone(processor);
    completions.push(Box::pin(async move {
        let completion = AssertUnwindSafe(processor.commit_prepared_attempt_completion(prepared))
            .catch_unwind()
            .await;
        let completion = completion.map_or_else(|_| CommitEnd::Panicked, CommitEnd::Finished);
        CompletionWorkResult::Committed {
            reservation,
            event_unix_ms,
            completion,
        }
    }));
}

/// Consumes the grant a durable commit just settled and queues any post-commit
/// consequence the commit asked for.
///
/// A commit that found the attempt no longer ours drops the grant without
/// answering the caller; every other outcome answers exactly once.
// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn consume_commit_end(
    processor: &Rc<AssetProcessor>,
    event_publisher: &AssetProcessorEventPublisher,
    state: &mut DispatcherState,
    completions: &FuturesUnordered<CompletionFuture>,
    reservation: &CompletionReservation,
    event_unix_ms: i64,
    completion: CommitEnd,
) {
    let attempt_id = reservation.identity.attempt_id;
    match completion {
        CommitEnd::Finished(Ok(DurableAttemptCompletion::Committed(post_commit))) => {
            consume_committed_completion(
                processor,
                event_publisher,
                state,
                reservation,
                event_unix_ms,
            );
            if let Some(post_commit) = post_commit {
                let processor = Rc::clone(processor);
                completions.push(Box::pin(async move {
                    let completion =
                        AssertUnwindSafe(processor.post_commit_attempt_completion(*post_commit))
                            .catch_unwind()
                            .await;
                    let completion = completion
                        .map_or_else(|_| PostCommitEnd::Panicked, PostCommitEnd::Finished);
                    CompletionWorkResult::PostCommitted {
                        attempt_id,
                        completion,
                    }
                }));
            } else if let Some(result) = state.completion_results.remove(&attempt_id) {
                let _ = result.send(Ok(true));
            }
        }
        CommitEnd::Finished(Ok(DurableAttemptCompletion::NoLongerOwned)) => {
            consume_no_longer_owned_completion(state, reservation);
        }
        CommitEnd::Finished(Err(error)) => {
            finish_completion(processor, state, reservation, Err(error)).await;
        }
        CommitEnd::Panicked => {
            finish_completion(
                processor,
                state,
                reservation,
                Err(AssetProcessorError::DispatcherCompletionPanicked { attempt_id }),
            )
            .await;
        }
    }
}

/// Answers the caller once the post-commit consequence has run.
///
/// The durable commit already happened, so a failing consequence is recorded as
/// a fault and the caller still gets its success.
fn finish_post_commit(
    event_publisher: &AssetProcessorEventPublisher,
    state: &mut DispatcherState,
    attempt_id: i64,
    completion: PostCommitEnd,
) {
    let failure = match completion {
        PostCommitEnd::Finished(Ok(())) => None,
        PostCommitEnd::Finished(Err(error)) => Some(error),
        PostCommitEnd::Panicked => {
            Some(AssetProcessorError::DispatcherCompletionPanicked { attempt_id })
        }
    };
    if let Some(error) = failure {
        event_publisher.record_fault(AssetProcessorConsequenceFault::PostCommit {
            attempt_id,
            reason: error.to_string(),
        });
        tracing::error!(
            attempt_id,
            %error,
            "asset completion post-commit consequence failed after durable commit"
        );
    }
    if let Some(result) = state.completion_results.remove(&attempt_id) {
        let _ = result.send(Ok(true));
    }
}

fn consume_committed_completion(
    processor: &AssetProcessor,
    event_publisher: &AssetProcessorEventPublisher,
    state: &mut DispatcherState,
    reservation: &CompletionReservation,
    event_unix_ms: i64,
) {
    let identity = reservation.identity.clone();
    let disposition = finish_completion_disposition(state, reservation, true);
    debug_assert!(matches!(
        disposition,
        CompletionDisposition::Consumed | CompletionDisposition::Unknown
    ));
    match processor.event_snapshot_for_attempt(identity.attempt_id) {
        Ok(Some(entry)) => {
            event_publisher.publish(AssetProcessorEventKind::JobCompleted, event_unix_ms, entry);
        }
        Ok(None) => {
            event_publisher.record_fault(AssetProcessorConsequenceFault::JobCompletedProjection {
                attempt_id: identity.attempt_id,
                reason: "completed asset job had no event snapshot".to_owned(),
            });
            tracing::error!(
                attempt_id = identity.attempt_id,
                "completed asset job had no event snapshot"
            );
        }
        Err(error) => {
            event_publisher.record_fault(AssetProcessorConsequenceFault::JobCompletedProjection {
                attempt_id: identity.attempt_id,
                reason: error.to_string(),
            });
            tracing::error!(
                attempt_id = identity.attempt_id,
                %error,
                "failed to project completed asset job event"
            );
        }
    }
}

fn consume_no_longer_owned_completion(
    state: &mut DispatcherState,
    reservation: &CompletionReservation,
) {
    let attempt_id = reservation.identity.attempt_id;
    if state
        .grants
        .get(&attempt_id)
        .is_some_and(|grant| grant.identity == reservation.identity)
    {
        state.grants.remove(&attempt_id);
    }
    if let Some(result) = state.completion_results.remove(&attempt_id) {
        let _ = result.send(Ok(false));
    }
}

fn begin_irreversible_completion(
    state: &mut DispatcherState,
    identity: &GrantIdentity,
    now: Instant,
) -> bool {
    let Some(grant) = state
        .grants
        .get_mut(&identity.attempt_id)
        .filter(|grant| grant.identity == *identity)
    else {
        return false;
    };
    if grant.connection_cancellation.is_cancelled()
        || !matches!(grant.phase, GrantPhase::CompletionPreparing { deadline } if deadline > now)
    {
        return false;
    }
    grant.phase = GrantPhase::CompletionIrreversible;
    true
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn finish_completion(
    processor: &AssetProcessor,
    state: &mut DispatcherState,
    reservation: &CompletionReservation,
    completion: Result<bool, AssetProcessorError>,
) {
    let identity = reservation.identity.clone();
    let disposition = finish_completion_disposition(state, reservation, false);
    if let Some(result) = state.completion_results.remove(&identity.attempt_id) {
        let _ = result.send(completion);
    }
    if disposition == CompletionDisposition::Expire {
        expire_grant(processor, state, &identity).await;
    }
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn shutdown_dispatcher(
    processor: &Rc<AssetProcessor>,
    event_publisher: &AssetProcessorEventPublisher,
    state: &mut DispatcherState,
    stagings: &mut FuturesUnordered<StagingFuture>,
    completions: &mut FuturesUnordered<CompletionFuture>,
) {
    let waiter_ids = state.waiters.keys().copied().collect::<Vec<_>>();
    for waiter_id in waiter_ids {
        finish_waiter(state, waiter_id, Err(dispatcher_stopped()));
    }
    // A dropped staging future would detach its spawn_blocking task. Drain it
    // here so a completed side channel is observed and removed after its
    // waiter has been rejected, before the durable attempt is abandoned.
    while let Some((waiter_id, result)) = stagings.next().await {
        staging_finished(processor, state, waiter_id, result).await;
    }
    let preparing = state
        .grants
        .values()
        .filter(|grant| matches!(grant.phase, GrantPhase::CompletionPreparing { .. }))
        .map(|grant| grant.identity.clone())
        .collect::<Vec<_>>();
    for identity in preparing {
        if timeout_completion(state, &identity) {
            expire_grant(processor, state, &identity).await;
        }
    }
    while let Some(completion) = completions.next().await {
        completion_finished(processor, event_publisher, state, completions, completion).await;
    }
    let grants = state
        .grants
        .values()
        .filter(|grant| grant.can_expire())
        .map(|grant| grant.identity.clone())
        .collect::<Vec<_>>();
    for identity in grants {
        expire_grant(processor, state, &identity).await;
    }
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn cancel_waiter(processor: &AssetProcessor, state: &mut DispatcherState, waiter_id: u64) {
    state.waiting.retain(|queued| *queued != waiter_id);
    let Some(waiter) = state.waiters.remove(&waiter_id) else {
        return;
    };
    if let WaiterPhase::Staging { identity } = waiter.phase {
        expire_grant(processor, state, &identity).await;
    }
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn staging_finished(
    processor: &AssetProcessor,
    state: &mut DispatcherState,
    waiter_id: u64,
    result: Result<LeasedAssetJob, AssetProcessorError>,
) {
    let Some(waiter) = state.waiters.remove(&waiter_id) else {
        if let Ok(attempt) = result {
            report_staged_payload_cleanup(&attempt);
        }
        return;
    };
    let WaiterPhase::Staging { identity } = waiter.phase else {
        return;
    };

    // Recheck after off-thread staging and before delivery. Cancellation wins
    // even if the worker task produced a valid side channel.
    if waiter.result.is_closed() {
        if let Ok(attempt) = &result {
            report_staged_payload_cleanup(attempt);
        }
        expire_grant(processor, state, &identity).await;
        return;
    }
    match result {
        Ok(attempt) => match deliver_staged_grant(state, &identity, waiter.result, attempt) {
            StagedDelivery::Delivered => {}
            StagedDelivery::ReceiverClosed(attempt) => {
                report_staged_payload_cleanup(&attempt);
                expire_grant(processor, state, &identity).await;
            }
            StagedDelivery::NoLongerGranted(attempt) => {
                report_staged_payload_cleanup(&attempt);
            }
        },
        Err(error) => {
            expire_grant(processor, state, &identity).await;
            let _ = waiter.result.send(Err(error));
        }
    }
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn expire_due_grants(processor: &AssetProcessor, state: &mut DispatcherState) {
    let now = Instant::now();
    let due = due_grant_identities(state, now);
    for identity in due {
        if timeout_completion(state, &identity) {
            expire_grant(processor, state, &identity).await;
            continue;
        }
        let staging_waiter = state.waiters.iter().find_map(|(waiter_id, waiter)| {
            matches!(
                &waiter.phase,
                WaiterPhase::Staging {
                    identity: waiter_identity
                } if *waiter_identity == identity
            )
            .then_some(*waiter_id)
        });
        if let Some(waiter_id) = staging_waiter {
            finish_waiter(
                state,
                waiter_id,
                Err(AssetProcessorError::DispatcherStagingTimeout {
                    attempt_id: identity.attempt_id,
                }),
            );
        }
        expire_grant(processor, state, &identity).await;
    }
}

fn timeout_completion(state: &mut DispatcherState, identity: &GrantIdentity) -> bool {
    let Some(grant) = state
        .grants
        .get_mut(&identity.attempt_id)
        .filter(|grant| grant.identity == *identity)
    else {
        return false;
    };
    if !matches!(grant.phase, GrantPhase::CompletionPreparing { .. }) {
        return false;
    }
    grant.phase = GrantPhase::Expiring {
        retry_at: Instant::now(),
    };
    if let Some(result) = state.completion_results.remove(&identity.attempt_id) {
        let _ = result.send(Err(AssetProcessorError::DispatcherCompletionTimeout {
            attempt_id: identity.attempt_id,
        }));
    }
    if let Some(control) = state.completion_controls.get(&identity.attempt_id) {
        control.abort.abort();
    }
    true
}

fn due_grant_identities(state: &DispatcherState, now: Instant) -> Vec<GrantIdentity> {
    state
        .grants
        .values()
        .filter(|grant| grant.deadline().is_some_and(|deadline| deadline <= now))
        .map(|grant| grant.identity.clone())
        .collect()
}

enum StagedDelivery {
    Delivered,
    ReceiverClosed(LeasedAssetJob),
    NoLongerGranted(LeasedAssetJob),
}

fn deliver_staged_grant(
    state: &mut DispatcherState,
    identity: &GrantIdentity,
    result: oneshot::Sender<LeaseOutcome>,
    attempt: LeasedAssetJob,
) -> StagedDelivery {
    let staging_is_live = state
        .grants
        .get(&identity.attempt_id).as_ref().is_some_and(|grant| {
            grant.identity == *identity
                && matches!(grant.phase, GrantPhase::Staging { deadline } if deadline > Instant::now())
        });
    if !staging_is_live {
        let _ = result.send(Err(invalid_worker_job_request(
            "staged attempt is no longer granted to this worker connection",
        )));
        return StagedDelivery::NoLongerGranted(attempt);
    }

    match result.send(Ok(LeaseAssetJobResult {
        leased: attempt,
        grant_key: identity.key,
    })) {
        Ok(()) => {
            let grant = state
                .grants
                .get_mut(&identity.attempt_id)
                .expect("staging grant cannot change during non-yielding delivery");
            debug_assert_eq!(grant.identity, *identity);
            let GrantPhase::Staging { deadline } = grant.phase else {
                unreachable!("staging grant changed during non-yielding delivery")
            };
            grant.phase = GrantPhase::Active { deadline };
            StagedDelivery::Delivered
        }
        Err(Ok(outcome)) => StagedDelivery::ReceiverClosed(outcome.leased),
        Err(Err(_)) => unreachable!("staged delivery only sends a successful lease outcome"),
    }
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn expire_grant(
    processor: &AssetProcessor,
    state: &mut DispatcherState,
    identity: &GrantIdentity,
) {
    let Some(grant) = state.grants.get(&identity.attempt_id) else {
        return;
    };
    if grant.identity != *identity || !grant.can_expire() {
        return;
    }
    let lease_owner = identity.owner.clone();
    let now = match AssetProcessor::current_unix_ms() {
        Ok(now) => now,
        Err(error) => {
            arm_grant_expiration_retry(state, identity, Instant::now());
            tracing::error!(attempt_id = identity.attempt_id, error = %error, "asset dispatcher clock failed while expiring grant");
            return;
        }
    };
    match processor
        .expire_lease(identity.attempt_id, &lease_owner, now)
        .await
    {
        Ok(_) => {
            state.grants.remove(&identity.attempt_id);
        }
        Err(error) => {
            arm_grant_expiration_retry(state, identity, Instant::now());
            tracing::error!(attempt_id = identity.attempt_id, error = %error, "asset dispatcher failed to expire grant");
        }
    }
}

fn arm_grant_expiration_retry(state: &mut DispatcherState, identity: &GrantIdentity, now: Instant) {
    if let Some(grant) = state
        .grants
        .get_mut(&identity.attempt_id)
        .filter(|grant| grant.identity == *identity && grant.can_expire())
    {
        grant.arm_expiration_retry(now);
    }
}

fn finish_waiter(state: &mut DispatcherState, waiter_id: u64, outcome: LeaseOutcome) {
    state.waiting.retain(|queued| *queued != waiter_id);
    if let Some(waiter) = state.waiters.remove(&waiter_id) {
        let _ = waiter.result.send(outcome);
    }
}

async fn run_failed_dispatcher(
    commands: &mut mpsc::Receiver<DispatcherCommand>,
    error: AssetProcessorError,
    shutdown: &CancellationToken,
) {
    let reason = error.to_string();
    loop {
        let command = tokio::select! {
            command = commands.recv() => command,
            () = shutdown.cancelled() => return,
        };
        let Some(command) = command else { return };
        match command {
            DispatcherCommand::Park { result, .. } => {
                let _ = result.send(Err(AssetProcessorError::DispatcherInitialization {
                    reason: reason.clone(),
                }));
            }
            DispatcherCommand::Renew { result, .. }
            | DispatcherCommand::Complete { result, .. } => {
                let _ = result.send(Err(AssetProcessorError::DispatcherInitialization {
                    reason: reason.clone(),
                }));
            }
        }
    }
}

fn cleanup_staged_payload(attempt: &LeasedAssetJob) -> std::io::Result<()> {
    if let Some(payload) = &attempt.source_payload {
        match std::fs::remove_file(&payload.locator) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn report_staged_payload_cleanup(attempt: &LeasedAssetJob) {
    if let Err(error) = cleanup_staged_payload(attempt) {
        let locator = attempt
            .source_payload
            .as_ref()
            .map_or("<none>", |payload| payload.locator.as_str());
        tracing::error!(
            attempt_id = attempt.attempt_id,
            locator,
            error = %error,
            "asset dispatcher failed to remove an undelivered staged payload"
        );
    }
}

async fn wait_for_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
        None => pending::<()>().await,
    }
}

const fn dispatcher_stopped() -> AssetProcessorError {
    AssetProcessorError::DispatcherStopped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{SyntheticFixture, SyntheticSourceSpec};
    use az_assetdb::{
        ApplyPlanDelta, BuilderDescriptor, Coupling, Digest, JobEdgeInput, PlanDelta, PlannedJob,
        ReplaceBuilderCatalog, SelectAssets, Target, TargetPath, Work,
    };

    fn test_capability() -> az_proto_core::Capability {
        az_proto_core::Capability {
            session: Some(Uuid::now_v7()),
            service: az_proto_core::ServiceId::new("test", "worker"),
            role: az_proto_core::ServiceRole::Worker,
            permissions: vec!["test".to_owned()],
            audience: "test".to_owned(),
            expires_unix_ms: u64::MAX,
            token_hash: vec![1],
        }
    }

    fn identity(attempt_id: i64) -> GrantIdentity {
        GrantIdentity::new(
            attempt_id,
            "worker-a".to_owned(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        )
    }

    fn state_with(grant: LiveGrant) -> DispatcherState {
        DispatcherState {
            waiters: BTreeMap::new(),
            waiting: VecDeque::new(),
            grants: BTreeMap::from([(grant.identity.attempt_id, grant)]),
            completion_results: BTreeMap::new(),
            completion_controls: BTreeMap::new(),
            ready: ReadyWindow::default(),
            idle: IdleResolution::default(),
        }
    }

    fn empty_state() -> DispatcherState {
        DispatcherState {
            waiters: BTreeMap::new(),
            waiting: VecDeque::new(),
            grants: BTreeMap::new(),
            completion_results: BTreeMap::new(),
            completion_controls: BTreeMap::new(),
            ready: ReadyWindow::default(),
            idle: IdleResolution::default(),
        }
    }

    fn test_event_publisher() -> AssetProcessorEventPublisher {
        AssetProcessorEventPublisher {
            subscribers: Rc::new(RefCell::new(Vec::new())),
            next_event_seq: Rc::new(Cell::new(1)),
            consequence_health: crate::AssetProcessorConsequenceHealth::default(),
        }
    }

    #[test]
    fn delayed_claim_deadline_starts_from_writer_claim_timestamp() {
        let pre_commit_queue_started = Instant::now();
        let claimed_at = pre_commit_queue_started + Duration::from_secs(30);
        let duration = Duration::from_millis(250);
        let deadline = lease_deadline_from_writer_claim(claimed_at, duration);

        assert_eq!(
            deadline.duration_since(claimed_at),
            duration,
            "pre-commit queueing cannot consume a writer-owned monotonic lease"
        );
        let staging_after_claim = claimed_at + Duration::from_millis(175);
        assert_eq!(
            deadline.duration_since(staging_after_claim),
            Duration::from_millis(75),
            "payload staging after the returned durable claim consumes the lease"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn waiter_drop_cancellation_does_not_depend_on_command_queue_capacity() {
        let (sender, _receiver) = mpsc::channel::<DispatcherCommand>(1);
        let (result, _result_receiver) = oneshot::channel();
        sender
            .send(DispatcherCommand::Renew {
                identity: identity(1),
                result,
            })
            .await
            .unwrap();
        let cancellation = CancellationToken::new();
        let wake = Rc::new(Notify::new());
        let notified = wake.notified();
        {
            let _guard = WaiterCancellation {
                cancellation: cancellation.clone(),
                wake: Rc::clone(&wake),
                armed: true,
            };
        }

        assert!(cancellation.is_cancelled());
        notified.await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_owner_signals_shutdown_without_aborting_its_task() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (commands, _receiver) = mpsc::channel(DISPATCHER_COMMAND_CAPACITY);
                let wake = Rc::new(Notify::new());
                let shutdown = CancellationToken::new();
                let observed_shutdown = Rc::new(Cell::new(false));
                let observed = Rc::clone(&observed_shutdown);
                let task_shutdown = shutdown.clone();
                let task = tokio::task::spawn_local(async move {
                    task_shutdown.cancelled().await;
                    tokio::task::yield_now().await;
                    observed.set(true);
                });
                let owner = AssetJobDispatcherOwner {
                    dispatcher: AssetJobDispatcher {
                        inner: Rc::new(DispatcherHandle {
                            commands,
                            next_waiter_id: Cell::new(1),
                            connection_cancellations: RefCell::new(BTreeMap::new()),
                            wake,
                        }),
                    },
                    shutdown,
                    task: Some(task),
                };

                drop(owner);
                tokio::task::yield_now().await;
                tokio::task::yield_now().await;

                assert!(observed_shutdown.get());
            })
            .await;
    }

    #[test]
    fn cancelled_waiters_shrink_and_reset_a_prefetched_ready_window() {
        let mut state = DispatcherState {
            waiters: BTreeMap::new(),
            waiting: VecDeque::new(),
            grants: BTreeMap::new(),
            completion_results: BTreeMap::new(),
            completion_controls: BTreeMap::new(),
            ready: ReadyWindow {
                jobs: (1..=3)
                    .map(|job_id| SelectJobs {
                        job_id,
                        workspace_pk: 1,
                        asset_pk: job_id,
                        kind: DbWork::Plan,
                        builder: None,
                        key: format!("job-{job_id}"),
                        platform: "pc".to_owned(),
                        status: DbStatus::Queued,
                        ready: true,
                        attempts: 0,
                    })
                    .collect(),
                ..ReadyWindow::default()
            },
            idle: IdleResolution::default(),
        };
        // Each receiver is held for the whole test on purpose: dropping one
        // would close its waiter's reply channel, and `prune_cancelled_waiting`
        // would then drop that waiter for being closed rather than cancelled.
        let mut waiters = Vec::new();
        for waiter_id in 1..=3 {
            let cancellation = CancellationToken::new();
            let (result, receiver) = oneshot::channel();
            state.waiters.insert(
                waiter_id,
                LeaseWaiter {
                    connection_id: Uuid::now_v7(),
                    envelope: LeaseEnvelope::new(
                        LeaseRequest::validated(
                            format!("worker-{waiter_id}"),
                            1_000,
                            PathBuf::from("staging"),
                        )
                        .unwrap(),
                        PayloadAuthority::validated(test_capability()),
                    ),
                    result,
                    cancellation: cancellation.clone(),
                    connection_cancellation: CancellationToken::new(),
                    phase: WaiterPhase::Waiting,
                },
            );
            state.waiting.push_back(waiter_id);
            waiters.push((cancellation, receiver));
        }
        waiters[1].0.cancel();
        waiters[2].0.cancel();

        prune_cancelled_waiting(&mut state);

        assert_eq!(state.waiting, VecDeque::from([1]));
        assert_eq!(state.waiters.keys().copied().collect::<Vec<_>>(), vec![1]);
        assert!(state.ready.jobs.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn initialization_failure_resolves_queued_and_future_lease_calls_with_cause() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (sender, mut receiver) = mpsc::channel(DISPATCHER_COMMAND_CAPACITY);
                let (queued_result, queued_receiver) = oneshot::channel();
                let cancellation = CancellationToken::new();
                sender
                    .send(DispatcherCommand::Park {
                        waiter_id: 1,
                        connection_id: Uuid::now_v7(),
                        envelope: LeaseEnvelope::new(
                            LeaseRequest::validated(
                                "worker-a".to_owned(),
                                1_000,
                                PathBuf::from("staging"),
                            )
                            .unwrap(),
                            PayloadAuthority::validated(test_capability()),
                        ),
                        result: queued_result,
                        cancellation: cancellation.clone(),
                        connection_cancellation: cancellation,
                    })
                    .await
                    .unwrap();
                let owner = tokio::task::spawn_local(async move {
                    let shutdown = CancellationToken::new();
                    run_failed_dispatcher(
                        &mut receiver,
                        AssetProcessorError::DispatcherInitialization {
                            reason: "status subscription unavailable".to_owned(),
                        },
                        &shutdown,
                    )
                    .await;
                });
                let queued = queued_receiver.await.unwrap().unwrap_err().to_string();
                assert!(queued.contains("status subscription unavailable"));

                let (future_result, future_receiver) = oneshot::channel();
                sender
                    .send(DispatcherCommand::Renew {
                        identity: identity(7),
                        result: future_result,
                    })
                    .await
                    .unwrap();
                let future = future_receiver.await.unwrap().unwrap_err().to_string();
                assert!(future.contains("status subscription unavailable"));
                drop(sender);
                owner.await.unwrap();
            })
            .await;
    }

    #[test]
    fn expiration_retry_keeps_the_matching_grant_live() {
        let identity = identity(42);
        let initial_deadline = Instant::now();
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::Active {
                deadline: initial_deadline,
            },
        });

        arm_grant_expiration_retry(&mut state, &identity, initial_deadline);

        assert_eq!(
            state.grants[&42].phase,
            GrantPhase::Expiring {
                retry_at: initial_deadline + EXPIRATION_RETRY_DELAY
            }
        );
    }

    #[test]
    fn failed_completion_is_fenced_for_expiration_without_reopening_grant() {
        let identity = identity(42);
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::CompletionIrreversible,
        });

        let disposition =
            finish_completion_disposition(&mut state, &CompletionReservation { identity }, false);

        assert_eq!(disposition, CompletionDisposition::Expire);
        assert!(matches!(
            state.grants[&42].phase,
            GrantPhase::Expiring { .. }
        ));
    }

    #[test]
    fn prepared_completion_cannot_cross_an_elapsed_grant_deadline() {
        let identity = identity(42);
        let deadline = Instant::now();
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::CompletionPreparing { deadline },
        });

        assert!(!begin_irreversible_completion(
            &mut state, &identity, deadline
        ));
        assert_eq!(
            state.grants[&42].phase,
            GrantPhase::CompletionPreparing { deadline }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_longer_owned_completion_consumes_local_grant_and_returns_false() {
        let identity = identity(42);
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::CompletionIrreversible,
        });
        let (result, receiver) = oneshot::channel();
        state.completion_results.insert(42, result);

        consume_no_longer_owned_completion(&mut state, &CompletionReservation { identity });

        assert!(state.grants.is_empty());
        assert!(!receiver.await.unwrap().unwrap());
    }

    #[test]
    fn completed_event_projection_failure_degrades_process_health() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = SyntheticFixture::build(
            temp.path().join("workspace"),
            "local.dispatcher-event-health",
            std::iter::empty::<SyntheticSourceSpec>(),
        )
        .unwrap();
        let rpc = fixture.rpc_for_test();
        drop(fixture);
        let identity = identity(999_999);
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::CompletionIrreversible,
        });

        consume_committed_completion(
            rpc.processor(),
            &rpc.event_publisher(),
            &mut state,
            &CompletionReservation { identity },
            1,
        );

        let health = rpc.health_snapshot();
        drop(rpc);
        assert_eq!(health.state, az_proto_core::ServiceHealthState::Degraded);
        assert_eq!(health.active_operation, "job-completion-consequence");
        assert!(health.message.contains("completed-event projection failed"));
    }

    #[test]
    fn failed_expiration_arms_one_future_retry_and_fences_worker_actions() {
        let now = Instant::now();
        let mut grant = LiveGrant {
            identity: identity(42),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::Active { deadline: now },
        };

        grant.arm_expiration_retry(now);

        assert_eq!(
            grant.phase,
            GrantPhase::Expiring {
                retry_at: now + EXPIRATION_RETRY_DELAY
            }
        );
        assert!(!grant.renew(now));
    }

    #[test]
    fn renewal_cannot_resurrect_an_elapsed_grant() {
        let now = Instant::now();
        let mut grant = LiveGrant {
            identity: identity(42),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::Active { deadline: now },
        };

        assert!(!grant.renew(now));
        assert_eq!(grant.phase, GrantPhase::Active { deadline: now });
    }

    #[test]
    fn cancelled_waiter_rejects_delayed_staging_and_retains_expiration_fence() {
        let identity = identity(42);
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::Staging {
                deadline: Instant::now() + Duration::from_secs(30),
            },
        });
        let temp = tempfile::tempdir().unwrap();
        let payload_path = temp.path().join("attempt-42-source.ron");
        std::fs::write(&payload_path, b"delayed payload").unwrap();
        let attempt = LeasedAssetJob {
            attempt_id: 42,
            workspace_id: 1,
            owner: az_proto_asset::JobOwner::Build(Uuid::now_v7()),
            source_guid: Uuid::now_v7(),
            preserved_source_sub_id: None,
            source_path: "source.ron".to_owned(),
            source_root: temp.path().to_string_lossy().into_owned(),
            source_schema_type: Some("az.test.Source".to_owned()),
            job_key: "compile".to_owned(),
            platform: "pc".to_owned(),
            ordinal: 1,
            staging_root: temp.path().to_string_lossy().into_owned(),
            source_payload: Some(az_proto_core::SideChannelHandle::staging_file(
                payload_path.to_string_lossy(),
                15,
                blake3::hash(b"delayed payload").as_bytes().to_vec(),
                std::env::consts::OS,
            )),
        };
        let (sender, receiver) = oneshot::channel();
        drop(receiver);

        assert!(due_grant_identities(&state, Instant::now()).is_empty());
        let StagedDelivery::ReceiverClosed(attempt) =
            deliver_staged_grant(&mut state, &identity, sender, attempt)
        else {
            panic!("closed waiter must reject delayed staging")
        };
        cleanup_staged_payload(&attempt).unwrap();
        assert!(!payload_path.exists());
        assert!(matches!(
            state.grants[&42].phase,
            GrantPhase::Staging { .. }
        ));
        assert!(state.grants[&42].can_expire());
        assert_eq!(state.grants[&42].identity, identity);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn closed_staging_waiter_abandons_the_durable_attempt_and_requeues_its_job() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = SyntheticFixture::build(
            temp.path().join("workspace"),
            "local.dispatcher-expiration",
            [SyntheticSourceSpec::new(
                "sources/delayed.prefab.ron",
                b"(name: \"delayed\")".to_vec(),
            )],
        )
        .unwrap();
        let rpc = fixture.rpc_for_test();
        let envelope = LeaseEnvelope::new(
            LeaseRequest::validated("worker-a".to_owned(), 30_000, temp.path().join("staging"))
                .unwrap(),
            PayloadAuthority::validated(fixture.jobs_capability_for_test()),
        );
        drop(fixture);
        let preparation = rpc
            .processor()
            .claim_lease_once(&envelope)
            .await
            .unwrap()
            .expect("synthetic sweep queued a planner job");
        let attempt_id = preparation.asset_job_attempt_id();
        let staged = preparation.stage().await.unwrap();
        let payload_path = staged
            .source_payload
            .as_ref()
            .map(|payload| PathBuf::from(&payload.locator));
        let durable_attempt = rpc
            .processor()
            .db()
            .attempt_by_id(attempt_id)
            .unwrap()
            .unwrap();
        assert_eq!(durable_attempt.status, DbStatus::Leased);

        let connection_id = Uuid::now_v7();
        let identity = GrantIdentity::new(
            attempt_id,
            "worker-a".to_owned(),
            connection_id,
            Uuid::now_v7(),
        );
        let (result, receiver) = oneshot::channel();
        drop(receiver);
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::Staging {
                deadline: Instant::now() + Duration::from_secs(30),
            },
        });
        let cancellation = CancellationToken::new();
        state.waiters.insert(
            7,
            LeaseWaiter {
                connection_id,
                envelope,
                result,
                cancellation: cancellation.clone(),
                connection_cancellation: cancellation,
                phase: WaiterPhase::Staging { identity },
            },
        );

        staging_finished(rpc.processor(), &mut state, 7, Ok(staged)).await;

        assert!(state.grants.is_empty());
        if let Some(payload_path) = payload_path {
            assert!(!payload_path.exists());
        }
        let attempt = rpc
            .processor()
            .db()
            .attempt_by_id(attempt_id)
            .unwrap()
            .unwrap();
        assert_eq!(attempt.status, DbStatus::Abandoned);
        let job = rpc
            .processor()
            .db()
            .job_by_id(attempt.job_pk)
            .unwrap()
            .unwrap();
        assert_eq!(job.status, DbStatus::Queued);
        drop(rpc);
        assert!(job.ready);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn staging_deadline_abandons_the_durable_attempt_and_resolves_the_waiter() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = SyntheticFixture::build(
            temp.path().join("workspace"),
            "local.dispatcher-staging-deadline",
            [SyntheticSourceSpec::new(
                "sources/slow.prefab.ron",
                b"(name: \"slow\")".to_vec(),
            )],
        )
        .unwrap();
        let rpc = fixture.rpc_for_test();
        let envelope = LeaseEnvelope::new(
            LeaseRequest::validated("worker-a".to_owned(), 30_000, temp.path().join("staging"))
                .unwrap(),
            PayloadAuthority::validated(fixture.jobs_capability_for_test()),
        );
        drop(fixture);
        let preparation = rpc
            .processor()
            .claim_lease_once(&envelope)
            .await
            .unwrap()
            .expect("synthetic sweep queued a planner job");
        let attempt_id = preparation.asset_job_attempt_id();
        let connection_id = Uuid::now_v7();
        let identity = GrantIdentity::new(
            attempt_id,
            "worker-a".to_owned(),
            connection_id,
            Uuid::now_v7(),
        );
        let cancellation = CancellationToken::new();
        let (result, receiver) = oneshot::channel();
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: cancellation.clone(),
            phase: GrantPhase::Staging {
                deadline: Instant::now(),
            },
        });
        state.waiters.insert(
            9,
            LeaseWaiter {
                connection_id,
                envelope,
                result,
                cancellation: cancellation.clone(),
                connection_cancellation: cancellation,
                phase: WaiterPhase::Staging { identity },
            },
        );

        expire_due_grants(rpc.processor(), &mut state).await;

        let error = receiver.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            AssetProcessorError::DispatcherStagingTimeout {
                attempt_id: timed_out
            } if timed_out == attempt_id
        ));
        assert!(state.grants.is_empty());
        let attempt = rpc
            .processor()
            .db()
            .attempt_by_id(attempt_id)
            .unwrap()
            .unwrap();
        assert_eq!(attempt.status, DbStatus::Abandoned);
        let job = rpc
            .processor()
            .db()
            .job_by_id(attempt.job_pk)
            .unwrap()
            .unwrap();
        assert_eq!(job.status, DbStatus::Queued);
        drop(rpc);
        assert!(job.ready);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_waits_for_active_staging_work() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = SyntheticFixture::build(
            temp.path().join("workspace"),
            "local.dispatcher-staging-shutdown",
            [SyntheticSourceSpec::new(
                "sources/staging.prefab.ron",
                b"(name: \"staging\")".to_vec(),
            )],
        )
        .unwrap();
        let rpc = fixture.rpc_for_test();
        drop(fixture);
        let processor = Rc::clone(&rpc.processor);
        let publisher = rpc.event_publisher();
        drop(rpc);
        let mut state = empty_state();
        let mut stagings = FuturesUnordered::<StagingFuture>::new();
        let mut completions = FuturesUnordered::<CompletionFuture>::new();
        let (started, observed_start) = oneshot::channel();
        let (release, wait_for_release) = oneshot::channel::<()>();
        let observed_completion = Rc::new(Cell::new(false));
        let staged_completion = Rc::clone(&observed_completion);
        let completion_before_release = Rc::clone(&observed_completion);
        stagings.push(Box::pin(async move {
            let _ = started.send(());
            wait_for_release
                .await
                .expect("shutdown test releases staging work");
            staged_completion.set(true);
            (0, Err(AssetProcessorError::DispatcherStopped))
        }));

        let shutdown = shutdown_dispatcher(
            &processor,
            &publisher,
            &mut state,
            &mut stagings,
            &mut completions,
        );
        let release_staging = async move {
            observed_start
                .await
                .expect("shutdown polls active staging work");
            assert!(!completion_before_release.get());
            release
                .send(())
                .expect("shutdown retains active staging work until completion");
        };
        tokio::join!(shutdown, release_staging);

        assert!(observed_completion.get());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_aborts_pending_completion_and_durably_requeues_its_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = SyntheticFixture::build(
            temp.path().join("workspace"),
            "local.dispatcher-completion-shutdown",
            [SyntheticSourceSpec::new(
                "sources/pending.prefab.ron",
                b"(name: \"pending\")".to_vec(),
            )],
        )
        .unwrap();
        let rpc = fixture.rpc_for_test();
        let envelope = LeaseEnvelope::new(
            LeaseRequest::validated("worker-a".to_owned(), 30_000, temp.path().join("staging"))
                .unwrap(),
            PayloadAuthority::validated(fixture.jobs_capability_for_test()),
        );
        drop(fixture);
        let preparation = rpc
            .processor()
            .claim_lease_once(&envelope)
            .await
            .unwrap()
            .expect("synthetic sweep queued a planner job");
        let attempt_id = preparation.asset_job_attempt_id();
        let identity = GrantIdentity::new(
            attempt_id,
            "worker-a".to_owned(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::CompletionPreparing {
                deadline: Instant::now() + Duration::from_secs(30),
            },
        });
        let (result, receiver) = oneshot::channel();
        state.completion_results.insert(attempt_id, result);
        let (abort, registration) = AbortHandle::new_pair();
        state
            .completion_controls
            .insert(attempt_id, CompletionControl { abort });
        let reservation = CompletionReservation {
            identity: identity.clone(),
        };
        let mut completions = FuturesUnordered::<CompletionFuture>::new();
        let mut stagings = FuturesUnordered::<StagingFuture>::new();
        completions.push(Box::pin(async move {
            let preparation = match Abortable::new(pending::<()>(), registration).await {
                Ok(()) => unreachable!("test completion stays pending until shutdown"),
                Err(_) => PreparationEnd::Aborted,
            };
            CompletionWorkResult::Prepared {
                reservation,
                event_unix_ms: 0,
                preparation,
            }
        }));

        shutdown_dispatcher(
            &Rc::clone(&rpc.processor),
            &test_event_publisher(),
            &mut state,
            &mut stagings,
            &mut completions,
        )
        .await;

        assert!(matches!(
            receiver.await.unwrap(),
            Err(AssetProcessorError::DispatcherCompletionTimeout {
                attempt_id: timed_out
            }) if timed_out == attempt_id
        ));
        assert!(state.grants.is_empty());
        let attempt = rpc
            .processor()
            .db()
            .attempt_by_id(attempt_id)
            .unwrap()
            .unwrap();
        assert_eq!(attempt.status, DbStatus::Abandoned);
        let job = rpc
            .processor()
            .db()
            .job_by_id(attempt.job_pk)
            .unwrap()
            .unwrap();
        assert_eq!(job.status, DbStatus::Queued);
        drop(rpc);
        assert!(job.ready);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_drains_irreversible_completion_to_true_and_one_event() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = SyntheticFixture::build(
            temp.path().join("workspace"),
            "local.dispatcher-irrev-shutdown",
            [SyntheticSourceSpec::new(
                "sources/irreversible.prefab.ron",
                b"(name: \"irreversible\")".to_vec(),
            )],
        )
        .unwrap();
        let rpc = fixture.rpc_for_test();
        let processor = Rc::clone(&rpc.processor);
        let envelope = LeaseEnvelope::new(
            LeaseRequest::validated("worker-a".to_owned(), 1, temp.path().join("staging")).unwrap(),
            PayloadAuthority::validated(fixture.jobs_capability_for_test()),
        );
        let preparation = processor
            .claim_lease_once(&envelope)
            .await
            .unwrap()
            .expect("synthetic sweep queued a planner job");
        let attempt_id = preparation.asset_job_attempt_id();
        let identity = GrantIdentity::new(
            attempt_id,
            "worker-a".to_owned(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let request = CompleteAssetJobAttemptRequest {
            capability: fixture.jobs_capability_for_test(),
            asset_job_attempt_id: attempt_id,
            lease_owner: "worker-a".to_owned(),
            grant_key: identity.key,
            status: az_proto_asset::AttemptStatus::Failed,
            finished_unix_ms: AssetProcessor::current_unix_ms().unwrap(),
            error_count: 1,
            warning_count: 0,
            product_manifest: None,
        };
        drop(fixture);
        let prepared = processor
            .prepare_attempt_completion(&request)
            .await
            .unwrap();
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_millis(1),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::CompletionIrreversible,
        });
        let (result, receiver) = oneshot::channel();
        state.completion_results.insert(attempt_id, result);
        let publisher = rpc.event_publisher();
        drop(rpc);
        let mut stagings = FuturesUnordered::<StagingFuture>::new();
        let mut completions = FuturesUnordered::<CompletionFuture>::new();
        let completion_processor = Rc::clone(&processor);
        let (release_commit, await_commit) = oneshot::channel::<()>();
        completions.push(Box::pin(async move {
            await_commit
                .await
                .expect("shutdown test releases irreversible commit");
            let durable = completion_processor
                .commit_prepared_attempt_completion(prepared)
                .await;
            CompletionWorkResult::Committed {
                reservation: CompletionReservation { identity },
                event_unix_ms: request.finished_unix_ms,
                completion: CommitEnd::Finished(durable),
            }
        }));
        tokio::time::sleep(Duration::from_millis(2)).await;

        let shutdown = shutdown_dispatcher(
            &processor,
            &publisher,
            &mut state,
            &mut stagings,
            &mut completions,
        );
        let release = async move {
            tokio::task::yield_now().await;
            release_commit
                .send(())
                .expect("shutdown retains the pending irreversible completion");
        };
        tokio::join!(shutdown, release);

        assert!(receiver.await.unwrap().unwrap());
        assert!(state.grants.is_empty());
        assert_eq!(publisher.next_event_seq.get(), 2);
        let attempt = processor.db().attempt_by_id(attempt_id).unwrap().unwrap();
        assert_eq!(attempt.status, DbStatus::Failed);
        let job = processor.db().job_by_id(attempt.job_pk).unwrap().unwrap();
        drop(processor);
        assert_eq!(job.attempts, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disconnect_before_completion_admission_abandons_without_commit() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = SyntheticFixture::build(
            temp.path().join("workspace"),
            "local.dispatcher-completion-disconnect",
            [SyntheticSourceSpec::new(
                "sources/disconnected.prefab.ron",
                b"(name: \"disconnected\")".to_vec(),
            )],
        )
        .unwrap();
        let rpc = fixture.rpc_for_test();
        let envelope = LeaseEnvelope::new(
            LeaseRequest::validated("worker-a".to_owned(), 30_000, temp.path().join("staging"))
                .unwrap(),
            PayloadAuthority::validated(fixture.jobs_capability_for_test()),
        );
        drop(fixture);
        let preparation = rpc
            .processor()
            .claim_lease_once(&envelope)
            .await
            .unwrap()
            .expect("synthetic sweep queued a planner job");
        let attempt_id = preparation.asset_job_attempt_id();
        let cancellation = CancellationToken::new();
        let identity = GrantIdentity::new(
            attempt_id,
            "worker-a".to_owned(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: cancellation.clone(),
            phase: GrantPhase::CompletionPreparing {
                deadline: Instant::now() + Duration::from_secs(30),
            },
        });
        let (result, receiver) = oneshot::channel();
        state.completion_results.insert(attempt_id, result);
        let (abort, _registration) = AbortHandle::new_pair();
        state
            .completion_controls
            .insert(attempt_id, CompletionControl { abort });
        cancellation.cancel();

        cancel_requested_work(rpc.processor(), &mut state).await;

        assert!(matches!(
            receiver.await.unwrap(),
            Err(AssetProcessorError::DispatcherCompletionTimeout {
                attempt_id: timed_out
            }) if timed_out == attempt_id
        ));
        assert!(state.grants.is_empty());
        let attempt = rpc
            .processor()
            .db()
            .attempt_by_id(attempt_id)
            .unwrap()
            .unwrap();
        assert_eq!(attempt.status, DbStatus::Abandoned);
        let job = rpc
            .processor()
            .db()
            .job_by_id(attempt.job_pk)
            .unwrap()
            .unwrap();
        assert_eq!(job.status, DbStatus::Queued);
        drop(rpc);
        assert!(job.ready);
        assert_eq!(job.attempts, 1);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn connected_precommit_error_abandons_once_without_reopening_grant() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = SyntheticFixture::build(
            temp.path().join("workspace"),
            "local.dispatcher-completion-error",
            [SyntheticSourceSpec::new(
                "sources/error.prefab.ron",
                b"(name: \"error\")".to_vec(),
            )],
        )
        .unwrap();
        let rpc = fixture.rpc_for_test();
        let processor = Rc::clone(&rpc.processor);
        let envelope = LeaseEnvelope::new(
            LeaseRequest::validated("worker-a".to_owned(), 30_000, temp.path().join("staging"))
                .unwrap(),
            PayloadAuthority::validated(fixture.jobs_capability_for_test()),
        );
        drop(fixture);
        let preparation = processor
            .claim_lease_once(&envelope)
            .await
            .unwrap()
            .expect("synthetic sweep queued a planner job");
        let attempt_id = preparation.asset_job_attempt_id();
        let identity = GrantIdentity::new(
            attempt_id,
            "worker-a".to_owned(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::CompletionPreparing {
                deadline: Instant::now() + Duration::from_secs(30),
            },
        });
        let (result, receiver) = oneshot::channel();
        state.completion_results.insert(attempt_id, result);
        let completions = FuturesUnordered::<CompletionFuture>::new();

        completion_finished(
            &processor,
            &rpc.event_publisher(),
            &mut state,
            &completions,
            CompletionWorkResult::Prepared {
                reservation: CompletionReservation { identity },
                event_unix_ms: AssetProcessor::current_unix_ms().unwrap(),
                preparation: PreparationEnd::Finished(Err(
                    AssetProcessorError::MissingProductManifest,
                )),
            },
        )
        .await;

        assert!(matches!(
            receiver.await.unwrap(),
            Err(AssetProcessorError::MissingProductManifest)
        ));
        assert!(state.grants.is_empty());
        let attempt = processor.db().attempt_by_id(attempt_id).unwrap().unwrap();
        assert_eq!(attempt.status, DbStatus::Abandoned);
        let job = processor.db().job_by_id(attempt.job_pk).unwrap().unwrap();
        drop(processor);
        assert_eq!(job.status, DbStatus::Queued);
        assert!(job.ready);
        assert_eq!(job.attempts, 1);
    }

    /// Claims the fixture's one queued planner job, reports it failed, and
    /// commits that failure durably.
    ///
    /// Returns the attempt, the grant identity that owns it, and the terminal
    /// timestamp -- everything a post-commit consequence needs to run against.
    /// The fixture is consumed here: nothing after the commit needs it, and the
    /// test asserts through the processor's own handle.
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn commit_failed_attempt(
        workspace_root: PathBuf,
        staging_root: PathBuf,
    ) -> (Rc<crate::AssetProcessorRpc>, i64, GrantIdentity, i64) {
        let fixture = SyntheticFixture::build(
            workspace_root,
            "local.dispatcher-post-commit-failure",
            [SyntheticSourceSpec::new(
                "sources/terminal.prefab.ron",
                b"(name: \"terminal\")".to_vec(),
            )],
        )
        .unwrap();
        let rpc = fixture.rpc_for_test();
        let processor = Rc::clone(&rpc.processor);
        let envelope = LeaseEnvelope::new(
            LeaseRequest::validated("worker-a".to_owned(), 30_000, staging_root).unwrap(),
            PayloadAuthority::validated(fixture.jobs_capability_for_test()),
        );
        let preparation = processor
            .claim_lease_once(&envelope)
            .await
            .unwrap()
            .expect("synthetic sweep queued a planner job");
        let attempt_id = preparation.asset_job_attempt_id();
        let identity = GrantIdentity::new(
            attempt_id,
            "worker-a".to_owned(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let request = CompleteAssetJobAttemptRequest {
            capability: fixture.jobs_capability_for_test(),
            asset_job_attempt_id: attempt_id,
            lease_owner: "worker-a".to_owned(),
            grant_key: identity.key,
            status: az_proto_asset::AttemptStatus::Failed,
            finished_unix_ms: AssetProcessor::current_unix_ms().unwrap(),
            error_count: 1,
            warning_count: 0,
            product_manifest: None,
        };
        let finished_unix_ms = request.finished_unix_ms;
        drop(fixture);
        let prepared = processor
            .prepare_attempt_completion(&request)
            .await
            .unwrap();
        let DurableAttemptCompletion::Committed(None) = processor
            .commit_prepared_attempt_completion(prepared)
            .await
            .unwrap()
        else {
            panic!("failed completion must durably commit without post-commit work")
        };
        drop(processor);
        (rpc, attempt_id, identity, finished_unix_ms)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_commit_projection_failure_consumes_grant_and_preserves_terminal_event() {
        let temp = tempfile::tempdir().unwrap();
        let (rpc, attempt_id, identity, finished_unix_ms) =
            commit_failed_attempt(temp.path().join("workspace"), temp.path().join("staging")).await;
        let processor = Rc::clone(&rpc.processor);
        // Scope the read borrow to this block. Dropping the `Ref` is not enough
        // on its own: its storage would still be live at the awaits below, so
        // the borrow would be part of this future's state across them.
        let (job, workspace) = {
            let db = processor.db();
            let attempt = db.attempt_by_id(attempt_id).unwrap().unwrap();
            let job = db.job_by_id(attempt.job_pk).unwrap().unwrap();
            let workspace = db.workspace_by_id(job.workspace_pk).unwrap().unwrap();
            drop(db);
            (job, workspace)
        };
        let post_commit = PostCommitAttemptCompletion {
            attempt_id,
            promotion: None,
            workspace,
            job,
            project_data_paths: processor.project_data_paths().unwrap().clone(),
            product_cache_root: temp.path().join("unused-product-cache"),
            generated_rust_projection_affected: true,
            manifest_elapsed: Duration::ZERO,
            promote_elapsed: Duration::ZERO,
            submit_elapsed: Duration::ZERO,
            fail_for_test: true,
        };
        let mut state = state_with(LiveGrant {
            identity: identity.clone(),
            lease_duration: Duration::from_secs(30),
            connection_cancellation: CancellationToken::new(),
            phase: GrantPhase::CompletionIrreversible,
        });
        let (result, receiver) = oneshot::channel();
        state.completion_results.insert(attempt_id, result);
        let publisher = rpc.event_publisher();
        let mut completions = FuturesUnordered::<CompletionFuture>::new();

        completion_finished(
            &processor,
            &publisher,
            &mut state,
            &completions,
            CompletionWorkResult::Committed {
                reservation: CompletionReservation { identity },
                event_unix_ms: finished_unix_ms,
                completion: CommitEnd::Finished(Ok(DurableAttemptCompletion::Committed(Some(
                    Box::new(post_commit),
                )))),
            },
        )
        .await;
        let post_commit = completions.next().await.unwrap();
        completion_finished(
            &processor,
            &publisher,
            &mut state,
            &completions,
            post_commit,
        )
        .await;

        assert!(state.grants.is_empty());
        assert_eq!(publisher.next_event_seq.get(), 2);
        assert!(receiver.await.unwrap().unwrap());
        let (fault_count, fault) = publisher.consequence_health.snapshot().unwrap();
        assert_eq!(fault_count, 1);
        assert!(matches!(
            fault,
            AssetProcessorConsequenceFault::PostCommit {
                attempt_id: fault_attempt,
                ..
            } if fault_attempt == attempt_id
        ));
        let health = rpc.health_snapshot();
        drop(rpc);
        assert_eq!(health.state, az_proto_core::ServiceHealthState::Degraded);
        assert_eq!(health.active_operation, "job-completion-consequence");
        let attempt = processor.db().attempt_by_id(attempt_id).unwrap().unwrap();
        assert_eq!(attempt.status, DbStatus::Failed);
        let job = processor.db().job_by_id(attempt.job_pk).unwrap().unwrap();
        drop(processor);
        assert_eq!(job.status, DbStatus::Failed);
        assert!(job.ready);
        assert_eq!(job.attempts, 1);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn idle_resolution_advances_past_a_stable_first_page() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = SyntheticFixture::build(
            temp.path().join("workspace"),
            "local.dispatcher-idle-keyset",
            [SyntheticSourceSpec::new(
                "sources/blocked.prefab.ron",
                b"(name: \"blocked\")".to_vec(),
            )],
        )
        .unwrap();
        let rpc = fixture.rpc_for_test();
        drop(fixture);
        let processor = rpc.processor();
        let workspace_pk = processor.attached_workspace_id().unwrap();
        let db = processor.dispatch_db();
        let root = db.workspace_roots(workspace_pk).unwrap().remove(0);
        let (asset, _) = db
            .source_asset(workspace_pk, root.root_pk, "sources/blocked.prefab.ron")
            .unwrap()
            .unwrap();
        let builder = Uuid::now_v7();
        let generation = Digest::from(blake3::hash(b"dispatcher-idle-keyset-builder"));
        processor
            .asset_db_writer()
            .replace_builder_catalog(ReplaceBuilderCatalog {
                workspace_pk,
                expected: None,
                replacement: generation,
                builders: vec![BuilderDescriptor {
                    guid: builder,
                    name: "dispatcher idle keyset builder".to_owned(),
                    version: 1,
                    digest: generation,
                }],
                plan_delta: PlanDelta::default(),
                updated: 1,
            })
            .await
            .unwrap();
        processor
            .asset_db_writer()
            .apply_plan_delta(ApplyPlanDelta {
                workspace_pk,
                delta: PlanDelta {
                    replacements: stable_first_page_plan(&asset, builder),
                    ..PlanDelta::default()
                },
            })
            .await
            .unwrap();

        let IdlePageOutcome::More { after_job_id } =
            resolve_idle_blocked_page(processor, 0).await.unwrap()
        else {
            panic!("the stable first page must advance the keyset cursor");
        };
        assert!(matches!(
            resolve_idle_blocked_page(processor, after_job_id)
                .await
                .unwrap(),
            IdlePageOutcome::Progress
        ));
        let later = db
            .jobs_for_asset(workspace_pk, asset.asset_id)
            .unwrap()
            .into_iter()
            .find(|job| job.key == "later-order-only")
            .unwrap();
        assert!(later.ready);
        assert_eq!(later.status, DbStatus::Queued);
    }

    /// One dependency job, 64 order-coupled jobs behind it, and one last job
    /// blocked only by an order-only edge to a target that does not exist.
    ///
    /// The 64 fill the idle resolver's first page without producing any
    /// progress, so the last job is reachable only once the keyset cursor has
    /// advanced past that page.
    fn stable_first_page_plan(asset: &SelectAssets, builder: Uuid) -> Vec<PlannedJob> {
        let mut replacements = vec![PlannedJob {
            asset_pk: asset.asset_id,
            kind: Work::Build,
            builder: Some(builder),
            key: "dependency".to_owned(),
            platform: "pc".to_owned(),
            edges: Vec::new(),
        }];
        replacements.extend((0..64).map(|index| PlannedJob {
            asset_pk: asset.asset_id,
            kind: Work::Build,
            builder: Some(builder),
            key: format!("stable-{index:02}"),
            platform: "pc".to_owned(),
            edges: vec![JobEdgeInput {
                asset_pk: Some(asset.asset_id),
                target: Target::Guid(asset.guid),
                key: "dependency".to_owned(),
                platform: "pc".to_owned(),
                coupling: Coupling::Order,
            }],
        }));
        replacements.push(PlannedJob {
            asset_pk: asset.asset_id,
            kind: Work::Build,
            builder: Some(builder),
            key: "later-order-only".to_owned(),
            platform: "pc".to_owned(),
            edges: vec![JobEdgeInput {
                asset_pk: None,
                target: Target::Path(TargetPath::new("missing.ron").unwrap()),
                key: "missing".to_owned(),
                platform: "pc".to_owned(),
                coupling: Coupling::OrderOnly,
            }],
        });
        replacements
    }
}
