//! Process-owned source sweeps and per-root source mutation claims.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::ErrorKind;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use az_asset_builder::DEFAULT_PLATFORM_ID;
use az_assetdb::{
    ApplySweepDelta, AssetDb, AssetDbWriter, Diff as DbDiff, SelectEntries, SweepEntry,
    SweepPlannerJob, SweepRecord, SweepRemoval,
};
use az_filesystem::{SourcePath, safe_join};
use tokio::sync::{oneshot, watch};
use tracing::{error, info};

use crate::{
    ASSET_PLANNER_JOB_KEY, AssetBuilderCatalogResult, AssetProcessorError, ExistingSourceFacts,
    RegisteredSourceAssetsReconcileSummary, RegisteredSourceRoot, SourceAssetClassifiers,
    SourceRootScanCandidate,
};

const SWEEP_COMMAND_CAPACITY: usize = 256;
const MAX_ADMITTED_SWEEP_COMMANDS: usize = SWEEP_COMMAND_CAPACITY;
const MAX_SCOPED_SWEEP_PATHS: usize = 4_096;

/// Validated native spelling for a canonical source path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NativeRelativePath(String);

impl NativeRelativePath {
    fn validated(source: &SourcePath, value: String) -> Result<Self, AssetProcessorError> {
        let path = Path::new(&value);
        if value.is_empty()
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::Prefix(_) | Component::RootDir | Component::ParentDir
                )
            })
        {
            return Err(AssetProcessorError::InvalidNativeSourcePath {
                path: path.to_path_buf(),
                reason:
                    "native source path must be a non-empty relative path without parent traversal"
                        .to_owned(),
            });
        }
        if SourcePath::new(&value) != *source {
            return Err(AssetProcessorError::InvalidNativeSourcePath {
                path: path.to_path_buf(),
                reason: format!(
                    "native source path resolves to `{}`, not `{source}`",
                    SourcePath::new(&value)
                ),
            });
        }
        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

/// Filesystem evidence retained for one registered source root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SweepScope {
    Root,
    Paths(BTreeMap<SourcePath, NativeRelativePath>),
}

impl SweepScope {
    /// A scope for one already-canonical path. Production always arrives from a
    /// filesystem event and goes through [`Self::observed_path`], which keeps
    /// the native spelling; only tests build a scope from the canonical path
    /// alone.
    #[cfg(test)]
    pub(crate) fn path(path: SourcePath) -> Self {
        let native = path.as_str().to_owned();
        Self::observed_path(path, native)
            .expect("a canonical SourcePath is also a valid native relative path")
    }

    pub(crate) fn observed_path(
        path: SourcePath,
        native_relative: String,
    ) -> Result<Self, AssetProcessorError> {
        let native_relative = NativeRelativePath::validated(&path, native_relative)?;
        Ok(Self::Paths(BTreeMap::from([(path, native_relative)])))
    }

    pub(crate) fn merge(&mut self, incoming: Self) {
        match (&mut *self, incoming) {
            (Self::Root, _) | (_, Self::Root) => *self = Self::Root,
            (Self::Paths(current), Self::Paths(paths)) => {
                current.extend(paths);
                if current.len() > MAX_SCOPED_SWEEP_PATHS {
                    *self = Self::Root;
                }
            }
        }
    }

    const fn is_paths(&self) -> bool {
        matches!(self, Self::Paths(_))
    }

    const fn kind(&self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Paths(_) => "paths",
        }
    }

    fn path_count(&self) -> usize {
        match self {
            Self::Root => 0,
            Self::Paths(paths) => paths.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SweepRoot(i64);

impl SweepRoot {
    pub(crate) const fn registered(root: &RegisteredSourceRoot) -> Self {
        Self(root.workspace_root_pk)
    }

    pub(crate) const fn workspace_root(workspace_root_pk: i64) -> Self {
        Self(workspace_root_pk)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SweepProvenance {
    Startup,
    Watcher,
    Explicit { session: String },
}

impl SweepProvenance {
    const fn kind(&self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Watcher => "watcher",
            Self::Explicit { .. } => "explicit",
        }
    }

    fn changed_by_session(&self) -> Option<&str> {
        match self {
            Self::Explicit { session } => Some(session),
            Self::Startup | Self::Watcher => None,
        }
    }

    fn merge(&mut self, incoming: Self) {
        if matches!(incoming, Self::Explicit { .. }) || matches!(self, Self::Watcher) {
            *self = incoming;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepRequest {
    pub(crate) root: SweepRoot,
    pub(crate) scope: SweepScope,
    pub(crate) provenance: SweepProvenance,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepEffect {
    pub(crate) summary: RegisteredSourceAssetsReconcileSummary,
    pub(crate) changed_assets: Vec<i64>,
    pub(crate) wrote: bool,
}

type SweepReply = oneshot::Sender<Result<SweepEffect, AssetProcessorError>>;

struct SweepAdmission {
    returns: crossbeam_channel::Sender<()>,
}

impl Drop for SweepAdmission {
    fn drop(&mut self) {
        let _ = self.returns.try_send(());
    }
}

struct SweepWaiter {
    reply: SweepReply,
    _admission: SweepAdmission,
}

struct MutationWaiter {
    reply: oneshot::Sender<Result<SweepAdmission, AssetProcessorError>>,
    admission: SweepAdmission,
}

enum SweepCommand {
    Schedule(SweepRequest),
    Run(SweepRequest, SweepWaiter),
    AcquireMutation(SweepRoot, MutationWaiter),
    CatalogChanged,
    Shutdown(std::sync::mpsc::SyncSender<()>),
}

struct PendingSweep {
    scope: SweepScope,
    provenance: SweepProvenance,
    // Every waiter owns one admission until its sweep reaches a terminal
    // result, so this vector is bounded even after the ingress is drained.
    waiters: Vec<SweepWaiter>,
    parked_after_failure: bool,
}

struct RootLane {
    claimed: bool,
    pending_sweep: Option<PendingSweep>,
    pending_mutations: VecDeque<MutationWaiter>,
}

impl RootLane {
    const fn new() -> Self {
        Self {
            claimed: false,
            pending_sweep: None,
            pending_mutations: VecDeque::new(),
        }
    }
}

struct SweepCompletion {
    root: SweepRoot,
    scope: SweepScope,
    provenance: SweepProvenance,
    waiters: Vec<SweepWaiter>,
    result: Result<SweepEffect, String>,
}

#[derive(Debug, Clone, Copy)]
struct RootAdmission {
    workspace_root_pk: i64,
    first_sweep_succeeded: bool,
}

struct SweepShared {
    roots_by_root_pk: BTreeMap<i64, RootAdmission>,
    priority: BTreeSet<i64>,
    in_flight: usize,
    revision: watch::Sender<u64>,
}

impl SweepShared {
    fn changed(&self) {
        let next = self.revision.borrow().saturating_add(1);
        self.revision.send_replace(next);
    }
}

#[derive(Clone)]
pub struct SweepHandle {
    commands: crossbeam_channel::Sender<SweepCommand>,
    releases: crossbeam_channel::Sender<SweepRoot>,
    admissions: crossbeam_channel::Receiver<()>,
    admission_returns: crossbeam_channel::Sender<()>,
    shared: Arc<Mutex<SweepShared>>,
}

pub struct RootMutationPermit {
    root: Option<SweepRoot>,
    releases: crossbeam_channel::Sender<SweepRoot>,
    _admission: SweepAdmission,
}

impl Drop for RootMutationPermit {
    fn drop(&mut self) {
        if let Some(root) = self.root.take() {
            let _ = self.releases.try_send(root);
        }
    }
}

impl SweepHandle {
    fn admit(&self) -> Result<SweepAdmission, AssetProcessorError> {
        match self.admissions.try_recv() {
            Ok(()) => Ok(SweepAdmission {
                returns: self.admission_returns.clone(),
            }),
            Err(crossbeam_channel::TryRecvError::Empty) => {
                Err(AssetProcessorError::SweepOwnerOverloaded {
                    capacity: MAX_ADMITTED_SWEEP_COMMANDS,
                })
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                Err(AssetProcessorError::SweepOwnerClosed)
            }
        }
    }

    fn try_send(&self, command: SweepCommand) -> Result<(), AssetProcessorError> {
        match self.commands.try_send(command) {
            Ok(()) => Ok(()),
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                Err(AssetProcessorError::SweepOwnerOverloaded {
                    capacity: MAX_ADMITTED_SWEEP_COMMANDS,
                })
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                Err(AssetProcessorError::SweepOwnerClosed)
            }
        }
    }

    pub(crate) fn schedule(&self, request: SweepRequest) -> Result<(), AssetProcessorError> {
        self.try_send(SweepCommand::Schedule(request))
    }

    pub(crate) async fn run(
        &self,
        request: SweepRequest,
    ) -> Result<SweepEffect, AssetProcessorError> {
        let (send, receive) = oneshot::channel();
        self.try_send(SweepCommand::Run(
            request,
            SweepWaiter {
                reply: send,
                _admission: self.admit()?,
            },
        ))?;
        receive
            .await
            .map_err(|_| AssetProcessorError::SweepOwnerClosed)?
    }

    pub(crate) async fn acquire_mutation(
        &self,
        root: SweepRoot,
    ) -> Result<RootMutationPermit, AssetProcessorError> {
        let (send, receive) = oneshot::channel();
        self.try_send(SweepCommand::AcquireMutation(
            root,
            MutationWaiter {
                reply: send,
                admission: self.admit()?,
            },
        ))?;
        let admission = receive
            .await
            .map_err(|_| AssetProcessorError::SweepOwnerClosed)??;
        Ok(RootMutationPermit {
            root: Some(root),
            releases: self.releases.clone(),
            _admission: admission,
        })
    }

    pub(crate) fn catalog_changed(&self) -> Result<(), AssetProcessorError> {
        self.commands
            .send(SweepCommand::CatalogChanged)
            .map_err(|_| AssetProcessorError::SweepOwnerClosed)
    }

    pub(crate) fn root_is_admitted(&self, root_pk: i64) -> bool {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .roots_by_root_pk
            .get(&root_pk)
            .is_none_or(|root| root.first_sweep_succeeded)
    }

    pub(crate) fn in_flight(&self) -> usize {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .in_flight
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .revision
            .subscribe()
    }

    pub(crate) fn take_priority(&self) -> BTreeSet<i64> {
        std::mem::take(
            &mut self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .priority,
        )
    }
}

pub struct SweepOwner {
    handle: SweepHandle,
    thread: Option<thread::JoinHandle<()>>,
}

impl SweepOwner {
    pub(crate) fn start(
        database: AssetDb,
        writer: AssetDbWriter,
        roots: Vec<RegisteredSourceRoot>,
        catalog: Arc<Mutex<Option<AssetBuilderCatalogResult>>>,
    ) -> std::io::Result<Self> {
        let (commands, command_events) = crossbeam_channel::bounded(SWEEP_COMMAND_CAPACITY);
        let release_capacity = roots.len().max(1);
        let (releases, release_events) = crossbeam_channel::bounded(release_capacity);
        let (admission_returns, admissions) =
            crossbeam_channel::bounded(MAX_ADMITTED_SWEEP_COMMANDS);
        for _ in 0..MAX_ADMITTED_SWEEP_COMMANDS {
            admission_returns
                .send(())
                .expect("new sweep admission channel remains connected");
        }
        let (completed, completion_events) = crossbeam_channel::unbounded();
        let (revision, _) = watch::channel(0);
        let shared = Arc::new(Mutex::new(SweepShared {
            roots_by_root_pk: roots
                .iter()
                .map(|root| {
                    (
                        root.root_pk,
                        RootAdmission {
                            workspace_root_pk: root.workspace_root_pk,
                            first_sweep_succeeded: false,
                        },
                    )
                })
                .collect(),
            priority: BTreeSet::new(),
            in_flight: 0,
            revision,
        }));
        let handle = SweepHandle {
            commands,
            releases,
            admissions,
            admission_returns,
            shared: Arc::clone(&shared),
        };
        let thread = thread::Builder::new()
            .name("asset-sweep-owner".to_owned())
            .spawn(move || {
                run_owner(
                    database,
                    writer,
                    roots,
                    catalog,
                    shared,
                    command_events,
                    release_events,
                    completed,
                    completion_events,
                );
            })?;
        Ok(Self {
            handle,
            thread: Some(thread),
        })
    }

    pub(crate) fn handle(&self) -> SweepHandle {
        self.handle.clone()
    }

    pub(crate) fn stop(mut self) -> thread::Result<()> {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        if self
            .handle
            .commands
            .send(SweepCommand::Shutdown(send))
            .is_ok()
        {
            let _ = receive.recv();
        }
        self.thread.take().map_or(Ok(()), thread::JoinHandle::join)
    }
}

impl Drop for SweepOwner {
    fn drop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        if self
            .handle
            .commands
            .send(SweepCommand::Shutdown(send))
            .is_ok()
        {
            let _ = receive.recv();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// Thread entry point: every argument is moved into the spawned sweep-owner
// thread and must be owned, so it cannot take these by reference.
#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::too_many_arguments)]
fn run_owner(
    mut database: AssetDb,
    writer: AssetDbWriter,
    roots: Vec<RegisteredSourceRoot>,
    catalog: Arc<Mutex<Option<AssetBuilderCatalogResult>>>,
    shared: Arc<Mutex<SweepShared>>,
    commands: crossbeam_channel::Receiver<SweepCommand>,
    releases: crossbeam_channel::Receiver<SweepRoot>,
    completed: crossbeam_channel::Sender<SweepCompletion>,
    completion_events: crossbeam_channel::Receiver<SweepCompletion>,
) {
    let roots = roots
        .into_iter()
        .map(|root| (SweepRoot::registered(&root), root))
        .collect::<BTreeMap<_, _>>();
    let mut lanes = roots
        .keys()
        .map(|root| (*root, RootLane::new()))
        .collect::<BTreeMap<_, _>>();
    let mut workers = BTreeMap::<SweepRoot, thread::JoinHandle<()>>::new();
    let mut shutdown = None;

    loop {
        if shutdown.is_some() && lanes.values().all(|lane| !lane.claimed) {
            break;
        }
        crossbeam_channel::select! {
            recv(commands) -> command => match command {
                Ok(SweepCommand::Schedule(request)) if shutdown.is_none() => {
                    if let Some(root) = queue_sweep(&mut lanes, request, std::iter::empty()) {
                        schedule_root(
                            root, &mut database, &writer, &roots, &catalog, &shared,
                            &mut lanes, &mut workers, &completed,
                        );
                    }
                }
                Ok(SweepCommand::Run(request, waiter)) if shutdown.is_none() => {
                    if let Some(root) = queue_sweep(&mut lanes, request, std::iter::once(waiter)) {
                        schedule_root(
                            root, &mut database, &writer, &roots, &catalog, &shared,
                            &mut lanes, &mut workers, &completed,
                        );
                    }
                }
                Ok(SweepCommand::AcquireMutation(root, waiter)) if shutdown.is_none() => {
                    if let Some(lane) = lanes.get_mut(&root) {
                        lane.pending_mutations.push_back(waiter);
                        schedule_root(
                            root, &mut database, &writer, &roots, &catalog, &shared,
                            &mut lanes, &mut workers, &completed,
                        );
                    } else {
                        let _ = waiter.reply.send(Err(AssetProcessorError::UnknownSweepRoot {
                            workspace_root_pk: root.0,
                        }));
                    }
                }
                Ok(SweepCommand::CatalogChanged) if shutdown.is_none() => {
                    let roots_to_schedule = lanes.keys().copied().collect::<Vec<_>>();
                    for root in roots_to_schedule {
                        if let Some(pending) = &mut lanes.get_mut(&root).unwrap().pending_sweep {
                            pending.parked_after_failure = false;
                        }
                        schedule_root(
                            root, &mut database, &writer, &roots, &catalog, &shared,
                            &mut lanes, &mut workers, &completed,
                        );
                    }
                }
                Ok(SweepCommand::Shutdown(done)) => {
                    shutdown = Some(done);
                    reject_pending(&mut lanes);
                }
                Ok(command) => reject_closed_command(command),
                Err(_) => {
                    let (done, _) = std::sync::mpsc::sync_channel(1);
                    shutdown = Some(done);
                    reject_pending(&mut lanes);
                }
            },
            recv(releases) -> root => if let Ok(root) = root {
                if let Some(lane) = lanes.get_mut(&root) {
                    lane.claimed = false;
                }
                schedule_root(
                    root, &mut database, &writer, &roots, &catalog, &shared,
                    &mut lanes, &mut workers, &completed,
                );
            },
            recv(completion_events) -> completion => if let Ok(completion) = completion {
                let root = completion.root;
                if let Some(worker) = workers.remove(&root) {
                    let _ = worker.join();
                }
                finish_sweep(completion, &shared, &mut lanes);
                schedule_root(
                    root, &mut database, &writer, &roots, &catalog, &shared,
                    &mut lanes, &mut workers, &completed,
                );
            },
        }
    }
    for worker in workers.into_values() {
        let _ = worker.join();
    }
    if let Some(done) = shutdown {
        let _ = done.send(());
    }
}

/// Answers one command that arrived after the owner accepted shutdown.
///
/// The owner keeps draining its command channel so a caller waiting on a reply
/// learns the owner closed instead of seeing its oneshot dropped. Commands with
/// no waiter — including the `Shutdown` the owner already consumed — need no
/// answer.
fn reject_closed_command(command: SweepCommand) {
    match command {
        SweepCommand::Run(_, waiter) => {
            let _ = waiter
                .reply
                .send(Err(AssetProcessorError::SweepOwnerClosed));
        }
        SweepCommand::AcquireMutation(_, waiter) => {
            let _ = waiter
                .reply
                .send(Err(AssetProcessorError::SweepOwnerClosed));
        }
        SweepCommand::Schedule(_) | SweepCommand::CatalogChanged | SweepCommand::Shutdown(_) => {}
    }
}

fn queue_sweep(
    lanes: &mut BTreeMap<SweepRoot, RootLane>,
    request: SweepRequest,
    waiters: impl IntoIterator<Item = SweepWaiter>,
) -> Option<SweepRoot> {
    let root = request.root;
    let waiters = waiters.into_iter();
    let Some(lane) = lanes.get_mut(&root) else {
        for waiter in waiters {
            let _ = waiter
                .reply
                .send(Err(AssetProcessorError::UnknownSweepRoot {
                    workspace_root_pk: root.0,
                }));
        }
        return None;
    };
    match &mut lane.pending_sweep {
        Some(pending) => {
            pending.scope.merge(request.scope);
            pending.provenance.merge(request.provenance);
            pending.parked_after_failure = false;
            pending.waiters.extend(waiters);
        }
        None => {
            lane.pending_sweep = Some(PendingSweep {
                scope: request.scope,
                provenance: request.provenance,
                waiters: waiters.collect(),
                parked_after_failure: false,
            });
        }
    }
    Some(root)
}

fn reject_pending(lanes: &mut BTreeMap<SweepRoot, RootLane>) {
    for lane in lanes.values_mut() {
        if let Some(pending) = lane.pending_sweep.take() {
            for waiter in pending.waiters {
                let _ = waiter
                    .reply
                    .send(Err(AssetProcessorError::SweepOwnerClosed));
            }
        }
        for waiter in lane.pending_mutations.drain(..) {
            let _ = waiter
                .reply
                .send(Err(AssetProcessorError::SweepOwnerClosed));
        }
    }
}

/// Parks a sweep that could not be started, answering its waiters with why.
///
/// The scope stays on the lane so a later catalog change or coordination wake
/// picks it up again; `parked_after_failure` is what stops it from retrying on
/// its own and spinning against a durable failure.
fn park_failed_sweep(lane: &mut RootLane, root: SweepRoot, pending: PendingSweep, reason: &str) {
    lane.claimed = false;
    for waiter in pending.waiters {
        let _ = waiter.reply.send(Err(AssetProcessorError::SweepFailed {
            workspace_root_pk: root.0,
            reason: reason.to_owned(),
        }));
    }
    lane.pending_sweep = Some(PendingSweep {
        scope: pending.scope,
        provenance: pending.provenance,
        waiters: Vec::new(),
        parked_after_failure: true,
    });
}

#[allow(clippy::too_many_arguments)]
fn schedule_root(
    root: SweepRoot,
    database: &mut AssetDb,
    writer: &AssetDbWriter,
    roots: &BTreeMap<SweepRoot, RegisteredSourceRoot>,
    catalog: &Arc<Mutex<Option<AssetBuilderCatalogResult>>>,
    shared: &Arc<Mutex<SweepShared>>,
    lanes: &mut BTreeMap<SweepRoot, RootLane>,
    workers: &mut BTreeMap<SweepRoot, thread::JoinHandle<()>>,
    completed: &crossbeam_channel::Sender<SweepCompletion>,
) {
    let Some(lane) = lanes.get_mut(&root) else {
        return;
    };
    if lane.claimed {
        return;
    }
    if let Some(waiter) = lane.pending_mutations.pop_front() {
        lane.claimed = true;
        if waiter.reply.send(Ok(waiter.admission)).is_err() {
            lane.claimed = false;
            schedule_root(
                root, database, writer, roots, catalog, shared, lanes, workers, completed,
            );
        }
        return;
    }
    let Some(pending) = lane.pending_sweep.as_ref() else {
        return;
    };
    if pending.parked_after_failure {
        return;
    }
    let Some(catalog) = catalog
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
    else {
        return;
    };
    let pending = lane.pending_sweep.take().expect("pending sweep exists");
    lane.claimed = true;
    let policy = roots[&root].clone();
    let entries = match database.ordered_entries(policy.workspace_root_pk) {
        Ok(entries) => entries,
        Err(source) => {
            park_failed_sweep(lane, root, pending, &source.to_string());
            return;
        }
    };
    {
        let mut state = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.in_flight += 1;
        state.changed();
    }
    let classifiers = crate::source_asset_classifiers_from_catalog(&catalog);
    let writer = writer.clone();
    let completed = completed.clone();
    let scope = pending.scope;
    let provenance = pending.provenance;
    let thread_scope = scope.clone();
    let thread_provenance = provenance.clone();
    let waiters = pending.waiters;
    info!(
        workspace_root_pk = root.0,
        scope = scope.kind(),
        path_count = scope.path_count(),
        provenance = provenance.kind(),
        waiter_count = waiters.len(),
        "asset source sweep started"
    );
    let handle = thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| {
            crate::current_unix_ms_i64()
                .map_err(|error| error.to_string())
                .and_then(|now| {
                    execute_sweep(
                        &writer,
                        &policy,
                        entries,
                        &classifiers,
                        &thread_scope,
                        &thread_provenance,
                        now,
                    )
                    .map_err(|error| error.to_string())
                })
        }))
        .unwrap_or_else(|_| Err("sweep worker panicked".to_owned()));
        let _ = completed.send(SweepCompletion {
            root,
            scope: thread_scope,
            provenance: thread_provenance,
            waiters,
            result,
        });
    });
    workers.insert(root, handle);
}

fn finish_sweep(
    completion: SweepCompletion,
    shared: &Arc<Mutex<SweepShared>>,
    lanes: &mut BTreeMap<SweepRoot, RootLane>,
) {
    let lane = lanes
        .get_mut(&completion.root)
        .expect("completion root remains registered");
    lane.claimed = false;
    let mut state = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.in_flight = state.in_flight.saturating_sub(1);
    let scope = completion.scope.kind();
    let path_count = completion.scope.path_count();
    let provenance = completion.provenance.kind();
    let waiter_count = completion.waiters.len();
    match completion.result {
        Ok(effect) => {
            if let Some(root) = state
                .roots_by_root_pk
                .values_mut()
                .find(|root| root.workspace_root_pk == completion.root.0)
            {
                root.first_sweep_succeeded = true;
            }
            if completion.scope.is_paths() {
                state.priority.extend(effect.changed_assets.iter().copied());
            }
            for waiter in completion.waiters {
                let _ = waiter.reply.send(Ok(effect.clone()));
            }
            info!(
                workspace_root_pk = completion.root.0,
                scope,
                path_count,
                provenance,
                waiter_count,
                wrote = effect.wrote,
                "asset source sweep completed"
            );
        }
        Err(reason) => {
            for waiter in completion.waiters {
                let _ = waiter.reply.send(Err(AssetProcessorError::SweepFailed {
                    workspace_root_pk: completion.root.0,
                    reason: reason.clone(),
                }));
            }
            match &mut lane.pending_sweep {
                Some(pending) => {
                    pending.scope.merge(completion.scope);
                    pending.provenance.merge(completion.provenance);
                    pending.parked_after_failure = true;
                }
                None => {
                    lane.pending_sweep = Some(PendingSweep {
                        scope: completion.scope,
                        provenance: completion.provenance,
                        waiters: Vec::new(),
                        parked_after_failure: true,
                    });
                }
            }
            error!(
                workspace_root_pk = completion.root.0,
                scope,
                path_count,
                provenance,
                waiter_count,
                %reason,
                "asset source sweep failed; scope remains parked"
            );
        }
    }
    state.changed();
}

pub fn execute_sweep(
    writer: &AssetDbWriter,
    source_root: &RegisteredSourceRoot,
    entries: Vec<SelectEntries>,
    classifiers: &SourceAssetClassifiers,
    scope: &SweepScope,
    provenance: &SweepProvenance,
    now_unix_ms: i64,
) -> Result<SweepEffect, AssetProcessorError> {
    if classifiers.file_sources.is_empty() && classifiers.project_documents.is_empty() {
        return Err(AssetProcessorError::BuilderCatalogUnavailable);
    }
    let cancellation = az_work::CancellationToken::new();
    let mut current = entries
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let SweepScan {
        candidates,
        mut removal_paths,
    } = collect_sweep_candidates(source_root, classifiers, scope, &current, now_unix_ms)?;

    let mut records = Vec::new();
    let mut observed = 0;
    for candidate in candidates.into_values() {
        removal_paths.remove(&candidate.source_path);
        let existing_facts = current
            .remove(&candidate.source_path)
            .map(ExistingSourceFacts::from);
        if crate::source_candidate_facts_match(existing_facts.as_ref(), &candidate) {
            observed += 1;
            continue;
        }
        if let Some(record) = crate::source_root_scan_candidate_to_record(
            source_root,
            classifiers,
            candidate,
            now_unix_ms,
            &cancellation,
        )? {
            records.push(SweepRecord {
                source: SweepEntry {
                    path: record.source_path.clone(),
                    guid: record.asset_guid,
                    schema: Some(record.schema_type),
                    digest: record.content_hash,
                    diff: match existing_facts {
                        None => DbDiff::Added,
                        Some(ref entry) if entry.digest == record.content_hash => DbDiff::Clean,
                        Some(_) => DbDiff::Modified,
                    },
                    diagnostics: record.diagnostics_count,
                    updated: record.changed_unix_ms,
                    src_bytes: record.observation.source_file_byte_length,
                    src_mtime: record.observation.source_file_modified_unix_ns,
                    meta_bytes: record.observation.source_meta_byte_length,
                    meta_mtime: record.observation.source_meta_modified_unix_ns,
                    observed: record.observation.last_observed_unix_ms,
                    session: provenance.changed_by_session().map(str::to_owned),
                },
                planner: SweepPlannerJob {
                    key: ASSET_PLANNER_JOB_KEY.to_owned(),
                    platform: DEFAULT_PLATFORM_ID.as_str().to_owned(),
                },
            });
        }
    }
    let removals = removal_paths
        .into_iter()
        .filter(|path| current.contains_key(path))
        .map(|path| SweepRemoval {
            path,
            observed: now_unix_ms,
        })
        .collect::<Vec<_>>();
    if records.is_empty() && removals.is_empty() {
        return Ok(SweepEffect {
            summary: RegisteredSourceAssetsReconcileSummary {
                observed,
                ..RegisteredSourceAssetsReconcileSummary::default()
            },
            changed_assets: Vec::new(),
            wrote: false,
        });
    }
    let result = futures::executor::block_on(writer.apply_sweep_delta(ApplySweepDelta {
        workspace_pk: source_root.workspace_pk,
        workspace_root_pk: source_root.workspace_root_pk,
        records,
        removals,
    }))?;
    Ok(SweepEffect {
        summary: RegisteredSourceAssetsReconcileSummary {
            recorded: usize::try_from(result.inserted + result.updated).unwrap_or(usize::MAX),
            observed,
            deleted: usize::try_from(result.removed).unwrap_or(usize::MAX),
            planned_jobs: usize::try_from(result.planned).unwrap_or(usize::MAX),
            ..RegisteredSourceAssetsReconcileSummary::default()
        },
        changed_assets: result.changed_assets,
        wrote: true,
    })
}

/// What one sweep observed on disk before any of it is compared against the
/// already-recorded entries.
struct SweepScan {
    /// Every source the scope actually observed, keyed by canonical path.
    candidates: BTreeMap<String, SourceRootScanCandidate>,
    /// Recorded paths the scope covered. A path that an observation claims back
    /// is removed from this set; whatever remains was not found on disk.
    removal_paths: BTreeSet<String>,
}

/// Walks exactly as much of `source_root` as `scope` covers.
///
/// A root scope walks the whole root and puts every recorded path up for
/// removal. A path scope reads only the named paths — expanding a path that
/// turns out to be a directory into its subtree — so an unrelated part of the
/// root is never observed and never eligible for removal.
fn collect_sweep_candidates(
    source_root: &RegisteredSourceRoot,
    classifiers: &SourceAssetClassifiers,
    scope: &SweepScope,
    current: &BTreeMap<String, SelectEntries>,
    now_unix_ms: i64,
) -> Result<SweepScan, AssetProcessorError> {
    let mut candidates = BTreeMap::<String, SourceRootScanCandidate>::new();
    let mut removal_paths = BTreeSet::new();
    match scope {
        SweepScope::Root => {
            if let Some(inputs) = crate::SourceRootScanInputs::open(
                source_root.clone(),
                PathBuf::from(&source_root.path),
                classifiers.clone(),
                now_unix_ms,
            )? {
                for candidate in inputs {
                    let candidate = candidate?;
                    candidates.insert(candidate.source_path.clone(), candidate);
                }
            }
            removal_paths.extend(current.keys().cloned());
        }
        SweepScope::Paths(paths) => {
            for (source_path, native_relative) in paths {
                let root_path = PathBuf::from(&source_root.path);
                let physical =
                    safe_join(&root_path, native_relative.as_str()).map_err(|error| {
                        AssetProcessorError::InvalidNativeSourcePath {
                            path: root_path.join(native_relative.as_str()),
                            reason: error.to_string(),
                        }
                    })?;
                if fs::metadata(&physical).is_ok_and(|metadata| metadata.is_dir()) {
                    let prefix = format!("{}/", source_path.as_str().trim_end_matches('/'));
                    if let Some(inputs) = crate::SourceRootScanInputs::open_at(
                        source_root.clone(),
                        PathBuf::from(&source_root.path),
                        physical,
                        classifiers.clone(),
                        now_unix_ms,
                    )? {
                        for candidate in inputs {
                            let candidate = candidate?;
                            if candidate.source_path.starts_with(&prefix) {
                                candidates.insert(candidate.source_path.clone(), candidate);
                            }
                        }
                    }
                    removal_paths.extend(
                        current
                            .keys()
                            .filter(|path| path.starts_with(&prefix))
                            .cloned(),
                    );
                    continue;
                }
                removal_paths.insert(source_path.as_str().to_owned());
                if let Some(candidate) = candidate_for_path(
                    source_root,
                    classifiers,
                    source_path,
                    native_relative.as_str(),
                    now_unix_ms,
                )? {
                    candidates.insert(candidate.source_path.clone(), candidate);
                }
            }
        }
    }
    Ok(SweepScan {
        candidates,
        removal_paths,
    })
}

/// Observe one exact source path without walking its containing root.
fn candidate_for_path(
    source_root: &RegisteredSourceRoot,
    classifiers: &SourceAssetClassifiers,
    source_path: &SourcePath,
    native_relative: &str,
    observed_unix_ms: i64,
) -> Result<Option<SourceRootScanCandidate>, AssetProcessorError> {
    if source_root
        .exclusions
        .as_set()
        .contains(source_path.as_str())
        || source_path.ends_with(crate::SOURCE_META_SIDECAR_SUFFIX)
    {
        return Ok(None);
    }
    let source_root_path = PathBuf::from(&source_root.path);
    let entry_path = safe_join(&source_root_path, native_relative).map_err(|error| {
        AssetProcessorError::InvalidNativeSourcePath {
            path: source_root_path.join(native_relative),
            reason: error.to_string(),
        }
    })?;
    let metadata = match fs::metadata(&entry_path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(AssetProcessorError::SourceRootReconcileEntry {
                path: entry_path,
                source,
            });
        }
    };
    if !metadata.is_file() || crate::is_asset_root_scaffold_marker(&entry_path) {
        return Ok(None);
    }
    let source_path = source_path.as_str().to_owned();
    let file_source_schema =
        crate::classify_file_source_asset(source_root, &source_path, classifiers);
    let has_project_document_candidates =
        crate::project_document_source_path_has_candidates(source_root, &source_path, classifiers);
    if file_source_schema.is_none() && !has_project_document_candidates {
        return Ok(None);
    }
    let observation = crate::source_asset_observation(&entry_path, &metadata, observed_unix_ms)?;
    Ok(Some(SourceRootScanCandidate {
        entry_path,
        source_path,
        file_source_schema,
        has_project_document_candidates,
        observation,
    }))
}

#[cfg(test)]
mod tests {
    use az_asset_builder::AssetBuilderPattern;
    use az_assetdb::{Exclusions, RegisterWorkspace, RegisterWorkspaceRoot, WorkspaceKey};

    use super::*;
    use crate::{FileSourceClassifier, SourceRootRole};
    use futures::FutureExt;

    fn registered_root() -> (
        tempfile::TempDir,
        AssetDb,
        AssetDbWriter,
        RegisteredSourceRoot,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("assets");
        fs::create_dir_all(&source_path).unwrap();
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let workspace = writer
            .register_workspace(RegisterWorkspace {
                key: WorkspaceKey {
                    project: "sweep-tests".to_owned(),
                    root: temp.path().to_string_lossy().into_owned(),
                    branch: "main".to_owned(),
                },
                now: 1,
            })
            .wait_blocking()
            .unwrap();
        let (root, policy) = writer
            .register_workspace_root(RegisterWorkspaceRoot {
                workspace_pk: workspace.workspace_id,
                key: "project:sweep-tests:assets".to_owned(),
                owner: "sweep-tests".to_owned(),
                path: source_path.to_string_lossy().into_owned(),
                exclusions: Exclusions::default(),
            })
            .wait_blocking()
            .unwrap();
        let registered = RegisteredSourceRoot {
            workspace_pk: workspace.workspace_id,
            workspace_root_pk: policy.workspace_root_id,
            root_pk: root.root_id,
            id: root.key.clone(),
            owner: policy.owner.clone(),
            path: policy.path.clone(),
            display_name: "Sweep test assets".to_owned(),
            portable_key: root.key,
            mount: "@assets@".to_owned(),
            recursive: true,
            watch: true,
            writable: true,
            exclusions: policy.exclusions,
            output_prefix: String::new(),
            role: SourceRootRole::ProjectAssets,
        };
        (temp, db, writer, registered)
    }

    fn classifiers() -> SourceAssetClassifiers {
        SourceAssetClassifiers {
            project_documents: Vec::new(),
            file_sources: vec![FileSourceClassifier {
                source_schema_type: "az.test.SweepSource".to_owned(),
                source_root: crate::PROJECT_SOURCE_ROOT.to_owned(),
                default_path_prefix: String::new(),
                source_patterns: vec![AssetBuilderPattern::wildcard("*.ron")],
                extensions: vec!["ron".to_owned()],
            }],
            builder_claims: Vec::new(),
        }
    }

    #[test]
    fn scope_paths_validate_and_union_while_root_absorbs() {
        let mut scope =
            SweepScope::observed_path(SourcePath::new("one.ron"), "One.ron".to_owned()).unwrap();
        scope.merge(
            SweepScope::observed_path(SourcePath::new("two.ron"), "two.ron".to_owned()).unwrap(),
        );
        let SweepScope::Paths(paths) = &scope else {
            panic!("paths remain scoped")
        };
        assert_eq!(paths.len(), 2);
        assert!(
            SweepScope::observed_path(SourcePath::new("escape.ron"), "../escape.ron".to_owned(),)
                .is_err()
        );
        assert!(
            SweepScope::observed_path(SourcePath::new("one.ron"), "other.ron".to_owned()).is_err()
        );
        scope.merge(SweepScope::Root);
        assert_eq!(scope, SweepScope::Root);
    }

    #[test]
    fn path_scope_escalates_to_root_at_the_coalescing_bound() {
        let mut scope = SweepScope::path(SourcePath::new("0.ron"));
        for index in 1..=MAX_SCOPED_SWEEP_PATHS {
            scope.merge(SweepScope::path(SourcePath::new(format!("{index}.ron"))));
        }
        assert_eq!(scope, SweepScope::Root);
    }

    #[test]
    fn path_directory_expands_subtree_and_quiet_repeat_writes_nothing() {
        let (_temp, db, writer, root) = registered_root();
        let nested = PathBuf::from(&root.path).join("folder").join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("source.ron"), b"(value: 1)").unwrap();
        let sidecar = serde_json::to_vec(&crate::SourceAssetMeta::preserving_source_guid(
            uuid::Uuid::nil(),
        ))
        .unwrap();
        fs::write(
            nested.join(format!("source.ron{}", crate::SOURCE_META_SIDECAR_SUFFIX)),
            &sidecar,
        )
        .unwrap();

        let effect = execute_sweep(
            &writer,
            &root,
            db.ordered_entries(root.workspace_root_pk).unwrap(),
            &classifiers(),
            &SweepScope::path(SourcePath::new("folder")),
            &SweepProvenance::Watcher,
            10,
        )
        .unwrap();
        assert!(effect.wrote);
        let (_, entry) = db
            .source_asset(root.workspace_pk, root.root_pk, "folder/nested/source.ron")
            .unwrap()
            .unwrap();
        assert_eq!(entry.meta_bytes, i64::try_from(sidecar.len()).unwrap());

        let revision = db.subscribe_asset_processing_status().revision();
        let quiet = execute_sweep(
            &writer,
            &root,
            db.ordered_entries(root.workspace_root_pk).unwrap(),
            &classifiers(),
            &SweepScope::Root,
            &SweepProvenance::Watcher,
            11,
        )
        .unwrap();
        assert!(!quiet.wrote);
        assert_eq!(db.subscribe_asset_processing_status().revision(), revision);
    }

    fn shared_for(roots: &[(i64, i64)]) -> Arc<Mutex<SweepShared>> {
        let (revision, _) = watch::channel(0);
        Arc::new(Mutex::new(SweepShared {
            roots_by_root_pk: roots
                .iter()
                .map(|(root_pk, workspace_root_pk)| {
                    (
                        *root_pk,
                        RootAdmission {
                            workspace_root_pk: *workspace_root_pk,
                            first_sweep_succeeded: false,
                        },
                    )
                })
                .collect(),
            priority: BTreeSet::new(),
            in_flight: roots.len(),
            revision,
        }))
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn unrelated_root_completion_does_not_release_or_close_its_peer() {
        let shared = shared_for(&[(11, 101), (22, 202)]);
        let mut lanes = BTreeMap::from([
            (SweepRoot(101), RootLane::new()),
            (SweepRoot(202), RootLane::new()),
        ]);
        lanes.get_mut(&SweepRoot(101)).unwrap().claimed = true;
        lanes.get_mut(&SweepRoot(202)).unwrap().claimed = true;
        finish_sweep(
            SweepCompletion {
                root: SweepRoot(101),
                scope: SweepScope::Root,
                provenance: SweepProvenance::Startup,
                waiters: Vec::new(),
                result: Ok(SweepEffect::default()),
            },
            &shared,
            &mut lanes,
        );
        assert!(!lanes[&SweepRoot(101)].claimed);
        assert!(lanes[&SweepRoot(202)].claimed);
        let state = shared.lock().unwrap();
        assert!(state.roots_by_root_pk[&11].first_sweep_succeeded);
        assert!(!state.roots_by_root_pk[&22].first_sweep_succeeded);
        assert_eq!(state.in_flight, 1);
    }

    #[test]
    fn failed_scope_stays_parked_and_path_priority_is_scope_owned() {
        let shared = shared_for(&[(11, 101)]);
        let mut lanes = BTreeMap::from([(SweepRoot(101), RootLane::new())]);
        lanes.get_mut(&SweepRoot(101)).unwrap().claimed = true;
        finish_sweep(
            SweepCompletion {
                root: SweepRoot(101),
                scope: SweepScope::path(SourcePath::new("failed.ron")),
                provenance: SweepProvenance::Watcher,
                waiters: Vec::new(),
                result: Err("walk failed".to_owned()),
            },
            &shared,
            &mut lanes,
        );
        assert!(
            lanes[&SweepRoot(101)]
                .pending_sweep
                .as_ref()
                .unwrap()
                .parked_after_failure
        );

        lanes.get_mut(&SweepRoot(101)).unwrap().pending_sweep = None;
        lanes.get_mut(&SweepRoot(101)).unwrap().claimed = true;
        shared.lock().unwrap().in_flight = 1;
        finish_sweep(
            SweepCompletion {
                root: SweepRoot(101),
                scope: SweepScope::path(SourcePath::new("changed.ron")),
                provenance: SweepProvenance::Watcher,
                waiters: Vec::new(),
                result: Ok(SweepEffect {
                    changed_assets: vec![7],
                    ..SweepEffect::default()
                }),
            },
            &shared,
            &mut lanes,
        );
        assert_eq!(shared.lock().unwrap().priority, BTreeSet::from([7]));

        lanes.get_mut(&SweepRoot(101)).unwrap().claimed = true;
        shared.lock().unwrap().in_flight = 1;
        finish_sweep(
            SweepCompletion {
                root: SweepRoot(101),
                scope: SweepScope::Root,
                provenance: SweepProvenance::Watcher,
                waiters: Vec::new(),
                result: Ok(SweepEffect {
                    changed_assets: vec![9],
                    ..SweepEffect::default()
                }),
            },
            &shared,
            &mut lanes,
        );
        assert_eq!(shared.lock().unwrap().priority, BTreeSet::from([7]));
    }

    #[test]
    fn event_arriving_during_failed_sweep_is_merged_and_parked() {
        let shared = shared_for(&[(11, 101)]);
        let mut lanes = BTreeMap::from([(SweepRoot(101), RootLane::new())]);
        lanes.get_mut(&SweepRoot(101)).unwrap().claimed = true;
        lanes.get_mut(&SweepRoot(101)).unwrap().pending_sweep = Some(PendingSweep {
            scope: SweepScope::path(SourcePath::new("arrived.ron")),
            provenance: SweepProvenance::Watcher,
            waiters: Vec::new(),
            parked_after_failure: false,
        });

        finish_sweep(
            SweepCompletion {
                root: SweepRoot(101),
                scope: SweepScope::path(SourcePath::new("failed.ron")),
                provenance: SweepProvenance::Explicit {
                    session: "explicit-session".to_owned(),
                },
                waiters: Vec::new(),
                result: Err("walk failed".to_owned()),
            },
            &shared,
            &mut lanes,
        );

        let pending = lanes[&SweepRoot(101)].pending_sweep.as_ref().unwrap();
        assert!(pending.parked_after_failure);
        assert_eq!(
            pending.provenance,
            SweepProvenance::Explicit {
                session: "explicit-session".to_owned()
            }
        );
        let SweepScope::Paths(paths) = &pending.scope else {
            panic!("two failed path scopes remain path-scoped")
        };
        assert_eq!(
            paths.keys().map(SourcePath::as_str).collect::<Vec<_>>(),
            vec!["arrived.ron", "failed.ron"]
        );
    }

    #[test]
    fn mutation_claims_are_per_root_and_owner_shutdown_drains_them() {
        let (temp, db, writer, first) = registered_root();
        let second_path = temp.path().join("second-assets");
        fs::create_dir_all(&second_path).unwrap();
        let (second_root, second_policy) = writer
            .register_workspace_root(RegisterWorkspaceRoot {
                workspace_pk: first.workspace_pk,
                key: "project:sweep-tests:second".to_owned(),
                owner: "sweep-tests".to_owned(),
                path: second_path.to_string_lossy().into_owned(),
                exclusions: Exclusions::default(),
            })
            .wait_blocking()
            .unwrap();
        let second = RegisteredSourceRoot {
            workspace_pk: first.workspace_pk,
            workspace_root_pk: second_policy.workspace_root_id,
            root_pk: second_root.root_id,
            id: second_root.key.clone(),
            owner: second_policy.owner,
            path: second_policy.path,
            display_name: "Second assets".to_owned(),
            portable_key: second_root.key,
            mount: "@assets@".to_owned(),
            recursive: true,
            watch: true,
            writable: true,
            exclusions: second_policy.exclusions,
            output_prefix: String::new(),
            role: SourceRootRole::ProjectAssets,
        };
        let owner = SweepOwner::start(
            db.new_runtime_handle().unwrap(),
            writer,
            vec![first.clone(), second.clone()],
            Arc::new(Mutex::new(None)),
        )
        .unwrap();
        let handle = owner.handle();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (first_permit, second_permit) = runtime.block_on(async {
            let first_permit = handle
                .acquire_mutation(SweepRoot::registered(&first))
                .await
                .unwrap();
            let second_permit = handle
                .acquire_mutation(SweepRoot::registered(&second))
                .await
                .unwrap();
            let queued = handle.acquire_mutation(SweepRoot::registered(&first));
            futures::pin_mut!(queued);
            assert!(queued.as_mut().now_or_never().is_none());
            drop(first_permit);
            let queued_permit = queued.await.unwrap();
            (queued_permit, second_permit)
        });

        let (stopped, stopped_events) = std::sync::mpsc::sync_channel(1);
        let stop_thread = thread::spawn(move || {
            owner.stop().unwrap();
            let _ = stopped.send(());
        });
        assert!(stopped_events.try_recv().is_err());
        drop(first_permit);
        drop(second_permit);
        stopped_events.recv().unwrap();
        stop_thread.join().unwrap();
    }

    #[test]
    fn catalog_wake_reports_a_closed_owner() {
        let (_temp, db, writer, root) = registered_root();
        let owner = SweepOwner::start(
            db.new_runtime_handle().unwrap(),
            writer,
            vec![root],
            Arc::new(Mutex::new(None)),
        )
        .unwrap();
        let handle = owner.handle();

        owner.stop().unwrap();

        assert!(matches!(
            handle.catalog_changed(),
            Err(AssetProcessorError::SweepOwnerClosed)
        ));
    }

    #[test]
    fn fire_and_forget_sweeps_do_not_hold_terminal_admissions() {
        let (_temp, db, writer, root) = registered_root();
        let owner = SweepOwner::start(
            db.new_runtime_handle().unwrap(),
            writer,
            vec![root.clone()],
            Arc::new(Mutex::new(None)),
        )
        .unwrap();
        let handle = owner.handle();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let permit = runtime
            .block_on(handle.acquire_mutation(SweepRoot::registered(&root)))
            .unwrap();
        let admissions_before = handle.admissions.len();

        handle
            .schedule(SweepRequest {
                root: SweepRoot::registered(&root),
                scope: SweepScope::Root,
                provenance: SweepProvenance::Watcher,
            })
            .unwrap();
        while !handle.commands.is_empty() {
            thread::yield_now();
        }

        assert_eq!(handle.admissions.len(), admissions_before);
        drop(permit);
        owner.stop().unwrap();
    }

    #[test]
    fn awaited_commands_remain_bounded_after_ingress_is_drained() {
        let (_temp, db, writer, root) = registered_root();
        let owner = SweepOwner::start(
            db.new_runtime_handle().unwrap(),
            writer,
            vec![root.clone()],
            Arc::new(Mutex::new(None)),
        )
        .unwrap();
        let handle = owner.handle();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let permit = runtime
            .block_on(handle.acquire_mutation(SweepRoot::registered(&root)))
            .unwrap();

        let mut pending = Vec::new();
        for _ in 1..MAX_ADMITTED_SWEEP_COMMANDS {
            while handle.commands.is_full() {
                thread::yield_now();
            }
            let mut request = Box::pin(handle.run(SweepRequest {
                root: SweepRoot::registered(&root),
                scope: SweepScope::Root,
                provenance: SweepProvenance::Watcher,
            }));
            assert!(request.as_mut().now_or_never().is_none());
            pending.push(request);
        }
        let mut overloaded = Box::pin(handle.run(SweepRequest {
            root: SweepRoot::registered(&root),
            scope: SweepScope::Root,
            provenance: SweepProvenance::Watcher,
        }));
        assert!(matches!(
            overloaded.as_mut().now_or_never(),
            Some(Err(AssetProcessorError::SweepOwnerOverloaded {
                capacity: MAX_ADMITTED_SWEEP_COMMANDS
            }))
        ));

        drop(pending);
        drop(permit);
        owner.stop().unwrap();
    }
}
