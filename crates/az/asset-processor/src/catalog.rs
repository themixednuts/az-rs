//! Runtime catalog projection ownership.
//!
//! `AssetDB` owns the declared Catalog view and reports committed invalidation
//! facts. This module owns every process-local consequence: Fresh/Publishing/
//! Stale state, same-key waiting, builder-claim filtering, keyset streaming,
//! and atomic filesystem publication.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

use az_asset::{
    ASSET_CATALOG_FILE_NAME, AssetCatalogEntry, AssetCatalogPathRegistration,
    AssetCatalogStreamEncoder, AssetId, ProductDependency as RuntimeCatalogDependency,
};
use az_asset_builder::AssetBuilderPattern;
use az_assetdb::{
    AssetDb, AssetDbWriter, CatalogProductEdge, CatalogTarget, PostCommitEffect,
    PostCommitEffectDrain, PostCommitEffectSubscription, SelectCatalog, SelectWorkspaces,
};
use az_filesystem::ProjectDataPaths;
use az_proto_asset::{
    AssetBuilderCatalogResult, AssetBuilderDescriptor, AssetBuilderPatternKind,
    CatalogPathRegistration, CatalogProductDependency, CatalogProductEntry,
    PublishAssetCatalogResult,
};
use tracing::{info, instrument, warn};
use uuid::Uuid;

use crate::{
    AssetProcessorConsequenceFault, AssetProcessorConsequenceHealth, AssetProcessorError,
    db_catalog_path_registration_to_proto, db_product_byte_length_to_proto,
    db_product_format_version_to_proto, db_product_sub_id_to_proto,
    validate_release_content_platform, validate_release_content_scope,
    worker_builder_catalog_digest,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const CATALOG_PAGE_SIZE: u32 = 512;
const CATALOG_COMMAND_CAPACITY: usize = 64;

#[derive(Clone)]
pub struct CatalogPublisher {
    commands: crossbeam_channel::Sender<CatalogCommand>,
    admission: Arc<Semaphore>,
    builder_catalog: CatalogBuilderCatalog,
    stopped: Arc<AtomicBool>,
    scope: Arc<CatalogScope>,
}

/// The catalog owner reads this process-owned snapshot only when it starts a
/// publication. RPC callers never carry a builder catalog snapshot into the
/// queue, so a queued request cannot publish against an obsolete generation.
#[derive(Clone)]
struct CatalogBuilderCatalog {
    state: Arc<Mutex<CatalogBuilderCatalogState>>,
    changed: crossbeam_channel::Sender<()>,
}

#[derive(Clone)]
struct CatalogBuilderCatalogState {
    generation: BuilderCatalogGeneration,
    catalog: Option<AssetBuilderCatalogResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BuilderCatalogGeneration(Option<az_assetdb::Digest>);

#[derive(Clone)]
struct CatalogBuilderCatalogSnapshot {
    generation: BuilderCatalogGeneration,
    catalog: Option<AssetBuilderCatalogResult>,
}

impl CatalogBuilderCatalog {
    fn new(catalog: Option<AssetBuilderCatalogResult>) -> (Self, crossbeam_channel::Receiver<()>) {
        let (changed, changed_rx) = crossbeam_channel::bounded(1);
        let generation =
            BuilderCatalogGeneration(catalog.as_ref().map(worker_builder_catalog_digest));
        (
            Self {
                state: Arc::new(Mutex::new(CatalogBuilderCatalogState {
                    generation,
                    catalog,
                })),
                changed,
            },
            changed_rx,
        )
    }

    fn replace(&self, catalog: Option<AssetBuilderCatalogResult>) {
        let generation =
            BuilderCatalogGeneration(catalog.as_ref().map(worker_builder_catalog_digest));
        let changed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let changed = state.generation != generation;
            state.generation = generation;
            state.catalog = catalog;
            changed
        };
        if changed {
            // One queued marker is enough: the mutex contains the latest
            // generation and the owner re-reads it when awakened.
            let _ = self.changed.try_send(());
        }
    }

    fn snapshot(&self) -> CatalogBuilderCatalogSnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        CatalogBuilderCatalogSnapshot {
            generation: state.generation.clone(),
            catalog: state.catalog.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CatalogScope {
    workspace_id: i64,
    project_data_paths: ProjectDataPaths,
}

impl CatalogScope {
    pub(crate) fn validated(
        workspace: &SelectWorkspaces,
        project_data_paths: ProjectDataPaths,
    ) -> Result<Self, AssetProcessorError> {
        validate_release_content_scope(&project_data_paths, workspace)?;
        Ok(Self {
            workspace_id: workspace.workspace_id,
            project_data_paths,
        })
    }

    fn catalog_path(&self, platform: &str) -> Result<PathBuf, AssetProcessorError> {
        validate_release_content_platform(self.workspace_id, platform)?;
        Ok(self
            .project_data_paths
            .product_cache_dir(platform)?
            .join(ASSET_CATALOG_FILE_NAME))
    }
}

pub struct CatalogPublisherOwner {
    publisher: CatalogPublisher,
    shutdown: crossbeam_channel::Sender<()>,
    task: Option<JoinHandle<()>>,
}

impl CatalogPublisherOwner {
    pub(crate) fn start(
        db: AssetDb,
        writer: &AssetDbWriter,
        scope: CatalogScope,
        builder_catalog: Option<AssetBuilderCatalogResult>,
        health: AssetProcessorConsequenceHealth,
    ) -> Result<Self, AssetProcessorError> {
        Self::start_with_writer(
            db,
            writer.subscribe_post_commit_effects(),
            scope,
            builder_catalog,
            health,
            Arc::new(write_catalog),
        )
    }

    fn start_with_writer(
        db: AssetDb,
        effects: PostCommitEffectSubscription,
        scope: CatalogScope,
        builder_catalog: Option<AssetBuilderCatalogResult>,
        health: AssetProcessorConsequenceHealth,
        catalog_writer: CatalogWriteFn,
    ) -> Result<Self, AssetProcessorError> {
        let (commands, command_rx) = crossbeam_channel::bounded(CATALOG_COMMAND_CAPACITY);
        let (shutdown, shutdown_rx) = crossbeam_channel::bounded(1);
        let (completed, completed_rx) = crossbeam_channel::unbounded();
        let (builder_catalog, builder_catalog_changed_rx) =
            CatalogBuilderCatalog::new(builder_catalog);
        let stopped = Arc::new(AtomicBool::new(false));
        let publisher = CatalogPublisher {
            commands,
            admission: Arc::new(Semaphore::new(CATALOG_COMMAND_CAPACITY)),
            builder_catalog: builder_catalog.clone(),
            stopped: Arc::clone(&stopped),
            scope: Arc::new(scope),
        };
        let task = std::thread::Builder::new()
            .name("asset-catalog-publisher".to_owned())
            .spawn(move || {
                run_catalog_publisher(CatalogOwnerContext {
                    db: Some(db),
                    effects,
                    catalog_writer,
                    command_rx,
                    completed,
                    completed_rx,
                    builder_catalog,
                    builder_catalog_changed_rx,
                    health,
                    shutdown_rx,
                    projections: HashMap::new(),
                    pending: VecDeque::new(),
                    active: None,
                });
            })
            .map_err(|error| AssetProcessorError::CatalogPublisherStart { error })?;
        Ok(Self {
            publisher,
            shutdown,
            task: Some(task),
        })
    }

    pub(crate) fn publisher(&self) -> CatalogPublisher {
        self.publisher.clone()
    }

    pub(crate) fn shutdown(mut self) -> Result<(), AssetProcessorError> {
        self.publisher.stopped.store(true, Ordering::Release);
        let _ = self.shutdown.try_send(());
        let Some(task) = self.task.take() else {
            return Ok(());
        };
        task.join()
            .map_err(|_| AssetProcessorError::CatalogPublisherPanicked)
    }
}

impl Drop for CatalogPublisherOwner {
    fn drop(&mut self) {
        self.publisher.stopped.store(true, Ordering::Release);
        let _ = self.shutdown.try_send(());
        if let Some(task) = self.task.take()
            && task.join().is_err()
        {
            warn!("asset catalog publisher owner panicked during drop");
        }
    }
}

impl CatalogPublisher {
    pub(crate) fn replace_builder_catalog(&self, catalog: Option<AssetBuilderCatalogResult>) {
        self.builder_catalog.replace(catalog);
    }

    pub(crate) async fn publish(
        &self,
        platform: String,
    ) -> Result<PublishAssetCatalogResult, AssetProcessorError> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(AssetProcessorError::CatalogPublisherStopped);
        }
        let admission = Arc::clone(&self.admission)
            .try_acquire_owned()
            .map_err(|_| AssetProcessorError::CatalogPublisherOverloaded)?;
        // Scope validation and path derivation happen on the RPC thread before
        // this request can enter the bounded blocking-owner queue.
        let catalog_path = self.scope.catalog_path(&platform)?;
        let key = CatalogKey {
            workspace_pk: self.scope.workspace_id,
            platform,
        };
        let (response, receiver) = tokio::sync::oneshot::channel();
        self.commands
            .try_send(CatalogCommand {
                request: CatalogRequest { key, catalog_path },
                waiter: CatalogWaiter {
                    response,
                    _admission: admission,
                },
                #[cfg(test)]
                accepted: None,
            })
            .map_err(|error| match error {
                crossbeam_channel::TrySendError::Full(_) => {
                    AssetProcessorError::CatalogPublisherOverloaded
                }
                crossbeam_channel::TrySendError::Disconnected(_) => {
                    AssetProcessorError::CatalogPublisherStopped
                }
            })?;
        receiver
            .await
            .map_err(|_| AssetProcessorError::CatalogPublisherStopped)?
            .into_result()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CatalogKey {
    workspace_pk: i64,
    platform: String,
}

#[derive(Debug)]
enum ProjectionState {
    Stale,
    Publishing {
        generation: BuilderCatalogGeneration,
        invalidated: bool,
        waiters: Vec<CatalogWaiter>,
    },
    Fresh {
        generation: BuilderCatalogGeneration,
        result: PublishAssetCatalogResult,
    },
}

impl ProjectionState {
    fn invalidate(&mut self) {
        match self {
            Self::Publishing { invalidated, .. } => *invalidated = true,
            Self::Stale | Self::Fresh { .. } => {
                *self = Self::Stale;
            }
        }
    }
}

#[derive(Debug, Clone)]
enum CatalogTerminal {
    Success(PublishAssetCatalogResult),
    Failure(Arc<str>),
    Stopped,
}

impl CatalogTerminal {
    fn into_result(self) -> Result<PublishAssetCatalogResult, AssetProcessorError> {
        match self {
            Self::Success(result) => Ok(result),
            Self::Failure(reason) => Err(AssetProcessorError::CatalogPublicationFailed { reason }),
            Self::Stopped => Err(AssetProcessorError::CatalogPublisherStopped),
        }
    }
}

struct CatalogCommand {
    request: CatalogRequest,
    waiter: CatalogWaiter,
    #[cfg(test)]
    accepted: Option<crossbeam_channel::Sender<()>>,
}

#[derive(Clone)]
struct CatalogRequest {
    key: CatalogKey,
    catalog_path: PathBuf,
}

#[derive(Debug)]
struct CatalogWaiter {
    response: tokio::sync::oneshot::Sender<CatalogTerminal>,
    // Admission follows the caller's request through pending and shared-waiter
    // states. Dropping it only after a terminal response makes the configured
    // capacity a bound on all retained request state, not just channel ingress.
    _admission: OwnedSemaphorePermit,
}

#[derive(Clone)]
struct CatalogWriteRequest {
    key: CatalogKey,
    catalog_path: PathBuf,
    published_catalog: Option<AssetBuilderCatalogResult>,
}

type CatalogWriteFn = Arc<
    dyn Fn(&AssetDb, &CatalogWriteRequest) -> Result<WrittenCatalog, AssetProcessorError>
        + Send
        + Sync,
>;

struct CatalogOwnerContext {
    db: Option<AssetDb>,
    effects: PostCommitEffectSubscription,
    catalog_writer: CatalogWriteFn,
    command_rx: crossbeam_channel::Receiver<CatalogCommand>,
    completed: crossbeam_channel::Sender<CatalogWorkerCompletion>,
    completed_rx: crossbeam_channel::Receiver<CatalogWorkerCompletion>,
    builder_catalog: CatalogBuilderCatalog,
    builder_catalog_changed_rx: crossbeam_channel::Receiver<()>,
    health: AssetProcessorConsequenceHealth,
    shutdown_rx: crossbeam_channel::Receiver<()>,
    projections: HashMap<CatalogKey, ProjectionState>,
    pending: VecDeque<CatalogCommand>,
    active: Option<ActiveCatalogWorker>,
}

struct ActiveCatalogWorker {
    key: CatalogKey,
    task: JoinHandle<()>,
    db_slot: Arc<Mutex<Option<AssetDb>>>,
}

struct CatalogWorkerCompletion {
    key: CatalogKey,
    generation: BuilderCatalogGeneration,
    db: Option<AssetDb>,
    terminal: CatalogTerminal,
}

// The three flagged temporaries (`_oper1`, `_oper3`, `_res`) are created by
// the `crossbeam_channel::select!` expansion itself, so there is no binding
// here to drop earlier; the only way to tighten them is to stop using `select!`.
#[allow(clippy::significant_drop_tightening)]
fn run_catalog_publisher(mut owner: CatalogOwnerContext) {
    let mut shutting_down = false;
    loop {
        owner.start_pending_if_idle();
        if shutting_down && owner.active.is_none() {
            owner.reject_pending();
            return;
        }
        crossbeam_channel::select! {
            recv(owner.shutdown_rx) -> _ => {
                shutting_down = true;
                owner.reject_queued();
            }
            recv(owner.completed_rx) -> completed => {
                if let Ok(completed) = completed {
                    owner.finish_worker(completed);
                }
            }
            recv(owner.builder_catalog_changed_rx) -> changed => {
                if changed.is_ok() {
                    owner.invalidate_changed_builder_catalog();
                }
            }
            recv(owner.command_rx) -> command => {
                match command {
                    Ok(command) if !shutting_down => owner.accept(command),
                    Ok(command) => { let _ = command.waiter.response.send(CatalogTerminal::Stopped); }
                    Err(_) => shutting_down = true,
                }
            }
        }
    }
}

impl CatalogOwnerContext {
    fn invalidate_changed_builder_catalog(&mut self) {
        let generation = self.builder_catalog.snapshot().generation;
        for state in self.projections.values_mut() {
            match state {
                ProjectionState::Publishing {
                    generation: active,
                    invalidated,
                    ..
                } if *active != generation => *invalidated = true,
                ProjectionState::Fresh {
                    generation: fresh, ..
                } if *fresh != generation => *state = ProjectionState::Stale,
                ProjectionState::Stale
                | ProjectionState::Publishing { .. }
                | ProjectionState::Fresh { .. } => {}
            }
        }
    }

    fn drain_effects(&mut self) {
        match self.effects.drain() {
            PostCommitEffectDrain::Gap => {
                for state in self.projections.values_mut() {
                    state.invalidate();
                }
            }
            PostCommitEffectDrain::Effects(effects) => {
                for effect in effects {
                    match effect {
                        PostCommitEffect::CatalogInvalidated {
                            workspace_pk,
                            platform,
                        } => {
                            for (key, state) in &mut self.projections {
                                if key.workspace_pk == workspace_pk
                                    && platform
                                        .as_deref()
                                        .is_none_or(|platform| platform == key.platform)
                                {
                                    state.invalidate();
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn accept(&mut self, command: CatalogCommand) {
        #[cfg(test)]
        let accepted = command.accepted.clone();
        self.drain_effects();
        self.invalidate_changed_builder_catalog();
        let current_generation = self.builder_catalog.snapshot().generation;
        let key = command.request.key.clone();
        match self
            .projections
            .remove(&key)
            .unwrap_or(ProjectionState::Stale)
        {
            ProjectionState::Fresh { generation, result } if generation == current_generation => {
                self.projections.insert(
                    key,
                    ProjectionState::Fresh {
                        generation,
                        result: result.clone(),
                    },
                );
                let mut reused = result;
                reused.reused = true;
                let _ = command
                    .waiter
                    .response
                    .send(CatalogTerminal::Success(reused));
            }
            ProjectionState::Fresh { .. } | ProjectionState::Stale => {
                self.projections.insert(key, ProjectionState::Stale);
                self.start_or_queue(command);
            }
            ProjectionState::Publishing {
                generation,
                mut invalidated,
                mut waiters,
            } => {
                if generation == current_generation {
                    waiters.push(command.waiter);
                } else {
                    // This writer owns the old generation and its existing
                    // callers until it reaches a terminal. A caller admitted
                    // after a builder replacement must not inherit that
                    // terminal, even though it has the same catalog key.
                    invalidated = true;
                    self.pending.push_back(command);
                }
                self.projections.insert(
                    key,
                    ProjectionState::Publishing {
                        generation,
                        invalidated,
                        waiters,
                    },
                );
            }
        }
        #[cfg(test)]
        if let Some(accepted) = accepted {
            let _ = accepted.send(());
        }
    }

    fn start_or_queue(&mut self, command: CatalogCommand) {
        if self.active.is_some() {
            self.pending.push_back(command);
        } else {
            self.start(command);
        }
    }

    fn start(&mut self, command: CatalogCommand) {
        let snapshot = match self.resolved_builder_catalog(&command.request.key) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                // The durable builder replacement committed before the local
                // worker catalog reached this process. Keep the bounded claim
                // until its coalesced generation update wakes this owner.
                self.pending.push_back(command);
                return;
            }
            Err(error) => {
                let reason = Arc::from(error.to_string());
                self.record_catalog_failure(&command.request.key, &reason);
                let _ = command
                    .waiter
                    .response
                    .send(CatalogTerminal::Failure(reason));
                return;
            }
        };
        let key = command.request.key.clone();
        let generation = snapshot.generation.clone();
        self.projections.insert(
            key.clone(),
            ProjectionState::Publishing {
                generation: generation.clone(),
                invalidated: false,
                waiters: vec![command.waiter],
            },
        );
        let Some(db) = self.db.take() else {
            self.finish_terminal(
                key,
                &generation,
                &CatalogTerminal::Failure(Arc::from("catalog database was unavailable")),
            );
            return;
        };
        let writer = Arc::clone(&self.catalog_writer);
        let completed = self.completed.clone();
        let request = CatalogWriteRequest {
            key: command.request.key,
            catalog_path: command.request.catalog_path,
            published_catalog: snapshot.catalog,
        };
        let worker_key = key.clone();
        let worker_generation = generation.clone();
        // The slot preserves the read handle if thread creation fails. Spawn
        // failure is an ordinary terminal publication failure, never a panic.
        let db_slot = Arc::new(Mutex::new(Some(db)));
        let worker_db_slot = Arc::clone(&db_slot);
        let task = std::thread::Builder::new()
            .name("asset-catalog-write".to_owned())
            .spawn(move || {
                write_catalog_on_worker(
                    &worker_db_slot,
                    &writer,
                    &request,
                    worker_key,
                    worker_generation,
                    &completed,
                );
            });
        match task {
            Ok(task) => {
                self.active = Some(ActiveCatalogWorker { key, task, db_slot });
            }
            Err(error) => {
                self.db = db_slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                self.finish_terminal(
                    key,
                    &generation,
                    &CatalogTerminal::Failure(Arc::from(format!(
                        "failed to spawn catalog writer: {error}"
                    ))),
                );
            }
        }
    }

    fn finish_worker(&mut self, completed: CatalogWorkerCompletion) {
        let Some(active) = self.active.take() else {
            if let Some(db) = completed.db {
                self.db = Some(db);
            }
            self.record_catalog_failure(
                &completed.key,
                "catalog worker completed without an active owner",
            );
            return;
        };
        debug_assert_eq!(active.key, completed.key);
        if active.task.join().is_err() {
            warn!(platform = %completed.key.platform, "asset catalog writer task panicked after reporting completion");
        }
        if let Some(db) = completed.db.or_else(|| {
            active
                .db_slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
        }) {
            self.db = Some(db);
        } else {
            self.record_catalog_failure(
                &completed.key,
                "catalog worker completed without returning its database",
            );
        }
        self.finish_terminal(completed.key, &completed.generation, &completed.terminal);
    }

    fn finish_terminal(
        &mut self,
        key: CatalogKey,
        generation: &BuilderCatalogGeneration,
        terminal: &CatalogTerminal,
    ) {
        self.drain_effects();
        self.invalidate_changed_builder_catalog();
        if let CatalogTerminal::Failure(reason) = terminal {
            self.record_catalog_failure(&key, reason);
        }
        let ProjectionState::Publishing {
            generation: published_generation,
            invalidated,
            waiters,
        } = self
            .projections
            .remove(&key)
            .unwrap_or(ProjectionState::Stale)
        else {
            return;
        };
        let invalidated = invalidated || published_generation != *generation;
        let next = match terminal {
            CatalogTerminal::Success(result)
                if !invalidated
                    && published_generation == self.builder_catalog.snapshot().generation =>
            {
                ProjectionState::Fresh {
                    generation: published_generation,
                    result: result.clone(),
                }
            }
            CatalogTerminal::Success(_)
            | CatalogTerminal::Failure(_)
            | CatalogTerminal::Stopped => ProjectionState::Stale,
        };
        self.projections.insert(key, next);
        for waiter in waiters {
            let _ = waiter.response.send(terminal.clone());
        }
    }

    fn start_pending_if_idle(&mut self) {
        if self.active.is_some() {
            return;
        }
        // A request whose durable generation has no matching local catalog is
        // requeued. Process each currently pending item once so it waits for a
        // builder-catalog update instead of spinning this owner thread.
        let pending = self.pending.len();
        for _ in 0..pending {
            let Some(command) = self.pending.pop_front() else {
                return;
            };
            self.accept(command);
            if self.active.is_some() {
                return;
            }
        }
    }

    fn reject_pending(&mut self) {
        for command in self.pending.drain(..) {
            let _ = command.waiter.response.send(CatalogTerminal::Stopped);
        }
    }

    fn reject_queued(&mut self) {
        while let Ok(command) = self.command_rx.try_recv() {
            let _ = command.waiter.response.send(CatalogTerminal::Stopped);
        }
        self.reject_pending();
    }

    fn resolved_builder_catalog(
        &self,
        key: &CatalogKey,
    ) -> Result<Option<CatalogBuilderCatalogSnapshot>, AssetProcessorError> {
        let Some(db) = self.db.as_ref() else {
            return Err(AssetProcessorError::CatalogPublicationFailed {
                reason: Arc::from("catalog database was unavailable"),
            });
        };
        let durable_generation = db
            .workspace_by_id(key.workspace_pk)?
            .and_then(|workspace| workspace.builders);
        let snapshot = self.builder_catalog.snapshot();
        Ok((snapshot.generation.0 == durable_generation).then_some(snapshot))
    }

    fn record_catalog_failure(&self, key: &CatalogKey, reason: &str) {
        self.health
            .record(AssetProcessorConsequenceFault::CatalogPublication {
                workspace_id: key.workspace_pk,
                platform: key.platform.clone(),
                reason: reason.to_owned(),
            });
    }
}

/// Runs one catalog publication on the writer thread and reports its terminal.
///
/// The read handle arrives through `db_slot` rather than by move so a failed
/// spawn leaves it with the owner. Losing it here is an ordinary publication
/// failure, not a panic.
fn write_catalog_on_worker(
    db_slot: &Mutex<Option<AssetDb>>,
    writer: &CatalogWriteFn,
    request: &CatalogWriteRequest,
    key: CatalogKey,
    generation: BuilderCatalogGeneration,
    completed: &crossbeam_channel::Sender<CatalogWorkerCompletion>,
) {
    let Some(db) = db_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        let _ = completed.send(CatalogWorkerCompletion {
            key,
            generation,
            db: None,
            terminal: CatalogTerminal::Failure(Arc::from(
                "catalog worker lost its database handoff",
            )),
        });
        return;
    };
    let terminal = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer(&db, request)))
        .map_or_else(
            |_| CatalogTerminal::Failure(Arc::from("asset catalog writer panicked")),
            |result| match result {
                Ok(written) => CatalogTerminal::Success(PublishAssetCatalogResult {
                    catalog_path: written.catalog_path.to_string_lossy().into_owned(),
                    entry_count: written.entry_count,
                    reused: false,
                }),
                Err(error) => CatalogTerminal::Failure(Arc::from(error.to_string())),
            },
        );
    let _ = completed.send(CatalogWorkerCompletion {
        key,
        generation,
        db: Some(db),
        terminal,
    });
}
pub fn active_catalog_row(
    row: &SelectCatalog,
    published_catalog: Option<&AssetBuilderCatalogResult>,
) -> Result<bool, AssetProcessorError> {
    Ok(active_catalog_builder(row, published_catalog)?.is_some())
}

pub fn active_catalog_target_kind(
    target: Option<&CatalogTarget>,
    published_catalog: Option<&AssetBuilderCatalogResult>,
) -> Result<Option<Uuid>, AssetProcessorError> {
    let Some(target) = target else {
        return Ok(None);
    };
    let builder_guid = target
        .builder
        .ok_or(AssetProcessorError::CatalogProductMissingBuilder {
            product_id: target.product_pk,
            job_id: target.job_pk,
        })?;
    let active = published_catalog.is_none_or(|catalog| {
        catalog
            .builders
            .iter()
            .find(|builder| builder.builder_guid == builder_guid)
            .is_some_and(|builder| {
                published_builder_claims_source(builder, &target.source, target.schema.as_deref())
            })
    });
    Ok(active.then_some(target.kind))
}

fn active_catalog_builder(
    row: &SelectCatalog,
    published_catalog: Option<&AssetBuilderCatalogResult>,
) -> Result<Option<Uuid>, AssetProcessorError> {
    let builder_guid = row
        .builder
        .ok_or(AssetProcessorError::CatalogProductMissingBuilder {
            product_id: row.product_pk,
            job_id: row.job_pk,
        })?;
    Ok(published_catalog
        .is_none_or(|catalog| {
            catalog
                .builders
                .iter()
                .find(|builder| builder.builder_guid == builder_guid)
                .is_some_and(|builder| published_builder_claims_catalog_row(builder, row))
        })
        .then_some(builder_guid))
}

fn published_builder_claims_catalog_row(
    builder: &AssetBuilderDescriptor,
    row: &SelectCatalog,
) -> bool {
    published_builder_claims_source(builder, &row.source, row.schema.as_deref())
}

fn published_builder_claims_source(
    builder: &AssetBuilderDescriptor,
    source: &str,
    schema: Option<&str>,
) -> bool {
    let schema_matches = builder.source_schema_types.is_empty()
        || schema.is_some_and(|schema| {
            builder
                .source_schema_types
                .iter()
                .any(|candidate| candidate == schema)
        });
    schema_matches
        && builder.patterns.iter().any(|pattern| {
            let matcher = match pattern.kind {
                AssetBuilderPatternKind::Wildcard => {
                    Some(AssetBuilderPattern::wildcard(&pattern.pattern))
                }
                AssetBuilderPatternKind::Regex => AssetBuilderPattern::regex(&pattern.pattern).ok(),
            };
            matcher.is_some_and(|matcher| matcher.matches(source))
        })
}

pub fn catalog_product_entries(
    db: &AssetDb,
    workspace_pk: i64,
    platform: &str,
    published_catalog: Option<&AssetBuilderCatalogResult>,
) -> Result<Vec<CatalogProductEntry>, AssetProcessorError> {
    let mut cursor = None;
    let mut rows = Vec::new();
    let mut edges = Vec::new();
    loop {
        let page = db.catalog_page(workspace_pk, platform, cursor.as_ref(), CATALOG_PAGE_SIZE)?;
        let active_rows = page
            .rows
            .into_iter()
            .map(|row| {
                let builder = active_catalog_builder(&row, published_catalog)?;
                Ok(builder.map(|builder| (row, builder)))
            })
            .collect::<Result<Vec<_>, AssetProcessorError>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let active_product_ids = active_rows
            .iter()
            .map(|(row, _)| row.product_pk)
            .collect::<HashSet<_>>();
        rows.extend(active_rows);
        edges.extend(
            page.product_edges
                .into_iter()
                .filter(|edge| active_product_ids.contains(&edge.edge.product_pk)),
        );
        let Some(next) = page.next else {
            break;
        };
        cursor = Some(next);
    }

    let mut edges_by_product = BTreeMap::<i64, Vec<CatalogProductEdge>>::new();
    for edge in edges {
        edges_by_product
            .entry(edge.edge.product_pk)
            .or_default()
            .push(edge);
    }
    rows.into_iter()
        .map(|(row, builder_guid)| {
            let dependencies = edges_by_product.remove(&row.product_pk).unwrap_or_default();
            catalog_product_entry_to_proto(row, builder_guid, dependencies, published_catalog)
        })
        .collect()
}

pub fn catalog_product_entry_to_proto(
    row: SelectCatalog,
    builder_guid: Uuid,
    dependencies: Vec<CatalogProductEdge>,
    published_catalog: Option<&AssetBuilderCatalogResult>,
) -> Result<CatalogProductEntry, AssetProcessorError> {
    let product_format_version = db_product_format_version_to_proto(&row.path, row.version)?;
    let catalog_path_registration = db_catalog_path_registration_to_proto(row.registration);
    let dependencies = dependencies
        .into_iter()
        .map(|dependency| {
            Ok(CatalogProductDependency {
                asset_guid: dependency.edge.guid,
                sub_id: dependency.edge.sub_id,
                asset_type: active_catalog_target_kind(
                    dependency.target.as_ref(),
                    published_catalog,
                )?,
                hint: None,
            })
        })
        .collect::<Result<Vec<_>, AssetProcessorError>>()?;
    Ok(CatalogProductEntry {
        job_id: row.job_pk,
        product_id: row.product_pk,
        asset_guid: row.guid,
        source_path: row.source,
        builder_guid,
        job_key: row.job_key,
        platform: row.platform,
        product_path: row.path,
        asset_type: row.kind,
        sub_id: row.sub_id,
        product_format: row.format,
        product_format_version,
        content_hash: row.digest.to_string(),
        catalog_aliases: row.aliases.into_vec(),
        catalog_path_registration,
        byte_length: row.bytes,
        dependencies,
    })
}

struct WrittenCatalog {
    catalog_path: PathBuf,
    entry_count: u32,
}

#[instrument(skip(db, request), fields(workspace_id = request.key.workspace_pk, platform = %request.key.platform))]
fn write_catalog(
    db: &AssetDb,
    request: &CatalogWriteRequest,
) -> Result<WrittenCatalog, AssetProcessorError> {
    let platform = request.key.platform.as_str();
    let published_catalog = request.published_catalog.as_ref();
    let catalog_path = request.catalog_path.clone();
    let parent =
        catalog_path
            .parent()
            .ok_or_else(|| AssetProcessorError::AssetCatalogInvalidPath {
                path: catalog_path.clone(),
            })?;
    fs::create_dir_all(parent).map_err(|source| AssetProcessorError::AssetCatalogWrite {
        path: parent.to_path_buf(),
        source,
    })?;
    let temp_path = parent.join(format!(
        ".assetcatalog-{}.tmp",
        uuid::Uuid::now_v7().as_simple()
    ));
    let write_result = (|| {
        let file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|source| AssetProcessorError::AssetCatalogWrite {
                path: temp_path.clone(),
                source,
            })?;
        let mut encoder = AssetCatalogStreamEncoder::new_unknown_count(file, Vec::new())?;
        let mut cursor = None;
        loop {
            let page = db.catalog_page(
                request.key.workspace_pk,
                platform,
                cursor.as_ref(),
                CATALOG_PAGE_SIZE,
            )?;
            let mut edges_by_product = BTreeMap::<i64, Vec<CatalogProductEdge>>::new();
            for edge in page.product_edges {
                edges_by_product
                    .entry(edge.edge.product_pk)
                    .or_default()
                    .push(edge);
            }
            for row in page.rows {
                if !active_catalog_row(&row, published_catalog)? {
                    continue;
                }
                let entry = asset_catalog_entry_from_catalog(
                    &row,
                    edges_by_product.remove(&row.product_pk).unwrap_or_default(),
                    published_catalog,
                )?;
                encoder.push(&entry)?;
            }
            let Some(next) = page.next else {
                break;
            };
            cursor = Some(next);
        }
        let (file, receipt) = encoder.finish()?;
        file.sync_all()
            .map_err(|source| AssetProcessorError::AssetCatalogWrite {
                path: temp_path.clone(),
                source,
            })?;
        Ok::<_, AssetProcessorError>(receipt)
    })();
    let receipt = match write_result {
        Ok(receipt) => receipt,
        Err(error) => {
            cleanup_catalog_temp_file(&temp_path);
            return Err(error);
        }
    };
    let entry_count = receipt.entry_count;
    let byte_length = receipt.byte_count;
    if let Err(source) = atomic_replace_catalog(&temp_path, &catalog_path) {
        cleanup_catalog_temp_file(&temp_path);
        return Err(AssetProcessorError::AssetCatalogWrite {
            path: catalog_path,
            source,
        });
    }
    info!(
        path = %catalog_path.display(),
        entry_count,
        byte_length,
        "published runtime asset catalog"
    );
    Ok(WrittenCatalog {
        catalog_path,
        entry_count,
    })
}

fn cleanup_catalog_temp_file(temp_path: &Path) {
    match fs::remove_file(temp_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => warn!(
            path = %temp_path.display(),
            %error,
            "failed to remove abandoned runtime catalog temporary file"
        ),
    }
}

fn asset_catalog_entry_from_catalog(
    row: &SelectCatalog,
    dependencies: Vec<CatalogProductEdge>,
    published_catalog: Option<&AssetBuilderCatalogResult>,
) -> Result<AssetCatalogEntry, AssetProcessorError> {
    let sub_id = db_product_sub_id_to_proto(&row.path, row.sub_id)?;
    let format_version = db_product_format_version_to_proto(&row.path, row.version)?;
    let byte_length = db_product_byte_length_to_proto(&row.path, row.bytes)?;
    let dependencies = dependencies
        .into_iter()
        .map(|dependency| {
            let sub_id = db_product_sub_id_to_proto(&row.path, dependency.edge.sub_id)?;
            let asset_type =
                active_catalog_target_kind(dependency.target.as_ref(), published_catalog)?.ok_or(
                    AssetProcessorError::CatalogDependencyMissingType {
                        product_id: row.product_pk,
                        asset_guid: dependency.edge.guid,
                        sub_id: dependency.edge.sub_id,
                    },
                )?;
            Ok(RuntimeCatalogDependency {
                id: AssetId::new(dependency.edge.guid, sub_id),
                asset_type,
                hint: None,
            })
        })
        .collect::<Result<Vec<_>, AssetProcessorError>>()?;
    let catalog_aliases = row.aliases.as_slice().to_vec();
    let path_registration =
        catalog_path_registration_to_asset(db_catalog_path_registration_to_proto(row.registration));
    Ok(AssetCatalogEntry::new(
        AssetId::new(row.guid, sub_id),
        row.kind,
        row.format.clone(),
        format_version,
        row.path.clone(),
        None,
        byte_length,
        *row.digest.as_bytes(),
    )
    .with_source_path(format!("@assets@/{}", row.source))
    .with_path_registration(path_registration)
    .with_catalog_aliases(catalog_aliases)
    .with_dependencies(dependencies))
}

const fn catalog_path_registration_to_asset(
    value: CatalogPathRegistration,
) -> AssetCatalogPathRegistration {
    match value {
        CatalogPathRegistration::Registered => AssetCatalogPathRegistration::Registered,
        CatalogPathRegistration::AssetIdOnly => AssetCatalogPathRegistration::AssetIdOnly,
    }
}

#[cfg(not(windows))]
fn atomic_replace_catalog(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn atomic_replace_catalog(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both owned buffers are NUL-terminated UTF-16 paths and remain
    // alive for the duration of the Win32 call.
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| std::io::Error::other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use az_assetdb::{
        AssetDb, AssetDbWriter, PlanDelta, PostCommitEffect, RegisterWorkspace,
        ReplaceBuilderCatalog, WorkspaceKey,
    };
    use az_filesystem::AzothDataHome;
    use az_proto_asset::{
        AssetBuilderCatalogResult, AssetBuilderDescriptor, AssetBuilderPatternDescriptor,
        AssetBuilderPatternKind,
    };
    use futures::poll;
    use uuid::Uuid;

    use super::{
        CATALOG_COMMAND_CAPACITY, CatalogCommand, CatalogKey, CatalogPublisher,
        CatalogPublisherOwner, CatalogRequest, CatalogScope, CatalogTerminal, CatalogWaiter,
        CatalogWriteFn, CatalogWriteRequest, WrittenCatalog,
    };
    use crate::{
        AssetProcessorError, worker_builder_catalog_descriptors, worker_builder_catalog_digest,
    };

    struct Harness {
        _temp: tempfile::TempDir,
        _db: AssetDb,
        writer: AssetDbWriter,
        workspace_id: i64,
        health: crate::AssetProcessorConsequenceHealth,
        owner: CatalogPublisherOwner,
        publisher: CatalogPublisher,
    }

    async fn harness(catalog_writer: CatalogWriteFn) -> Harness {
        let temp = tempfile::tempdir().unwrap();
        let workspace_root = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_root).unwrap();
        let project_data_paths = AzothDataHome::new(temp.path().join("data-home"))
            .project("catalog-owner-tests", &workspace_root);
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let workspace = writer
            .register_workspace(RegisterWorkspace {
                key: WorkspaceKey {
                    project: "catalog-owner-tests".to_owned(),
                    root: workspace_root.to_string_lossy().into_owned(),
                    branch: "test".to_owned(),
                },
                now: 1,
            })
            .await
            .unwrap();
        let scope = CatalogScope {
            workspace_id: workspace.workspace_id,
            project_data_paths,
        };
        let health = crate::AssetProcessorConsequenceHealth::default();
        let owner = CatalogPublisherOwner::start_with_writer(
            db.new_runtime_handle().unwrap(),
            writer.subscribe_post_commit_effects(),
            scope,
            None,
            health.clone(),
            catalog_writer,
        )
        .unwrap();
        let publisher = owner.publisher();
        Harness {
            _temp: temp,
            _db: db,
            writer,
            workspace_id: workspace.workspace_id,
            health,
            owner,
            publisher,
        }
    }

    fn successful_writer(calls: Arc<AtomicUsize>) -> CatalogWriteFn {
        Arc::new(move |_db, request: &CatalogWriteRequest| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(WrittenCatalog {
                catalog_path: request.catalog_path.clone(),
                entry_count: 7,
            })
        })
    }

    fn queued_catalog_request_with_acceptance(
        publisher: &CatalogPublisher,
        platform: &str,
    ) -> (
        tokio::sync::oneshot::Receiver<CatalogTerminal>,
        crossbeam_channel::Receiver<()>,
    ) {
        let admission = Arc::clone(&publisher.admission)
            .try_acquire_owned()
            .unwrap();
        let catalog_path = publisher.scope.catalog_path(platform).unwrap();
        let (response, receiver) = tokio::sync::oneshot::channel();
        let (accepted, accepted_receiver) = crossbeam_channel::bounded(1);
        publisher
            .commands
            .try_send(CatalogCommand {
                request: CatalogRequest {
                    key: CatalogKey {
                        workspace_pk: publisher.scope.workspace_id,
                        platform: platform.to_owned(),
                    },
                    catalog_path,
                },
                waiter: CatalogWaiter {
                    response,
                    _admission: admission,
                },
                accepted: Some(accepted),
            })
            .unwrap();
        (receiver, accepted_receiver)
    }

    fn builder_catalog(fingerprint: &str) -> AssetBuilderCatalogResult {
        AssetBuilderCatalogResult {
            builders: vec![AssetBuilderDescriptor {
                name: "catalog-generation-test".to_owned(),
                builder_guid: Uuid::from_u128(0x4b3b_918e_c3c8_4b0e_9f7f_8c1a_6f5c_b111),
                version: 1,
                analysis_fingerprint: fingerprint.to_owned(),
                patterns: vec![AssetBuilderPatternDescriptor {
                    kind: AssetBuilderPatternKind::Wildcard,
                    pattern: "*.ron".to_owned(),
                }],
                source_schema_types: Vec::new(),
            }],
            source_schemas: Vec::new(),
            product_formats: Vec::new(),
        }
    }

    fn gated_writer(
        calls: Arc<AtomicUsize>,
        started: crossbeam_channel::Sender<()>,
        release: crossbeam_channel::Receiver<()>,
        failure: Option<&'static str>,
    ) -> CatalogWriteFn {
        Arc::new(move |_db, request: &CatalogWriteRequest| {
            calls.fetch_add(1, Ordering::SeqCst);
            started.send(()).unwrap();
            release.recv().unwrap();
            if let Some(reason) = failure {
                return Err(AssetProcessorError::CatalogPublicationFailed {
                    reason: Arc::from(reason),
                });
            }
            Ok(WrittenCatalog {
                catalog_path: request.catalog_path.clone(),
                entry_count: 7,
            })
        })
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn same_key_waiters_share_one_successful_publication() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (started, did_start) = crossbeam_channel::bounded(1);
        let (release, may_finish) = crossbeam_channel::bounded(1);
        let writer_calls = Arc::clone(&calls);
        let catalog_writer: CatalogWriteFn = Arc::new(move |_db, request| {
            let call = writer_calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                started.send(()).unwrap();
                may_finish.recv().unwrap();
            }
            Ok(WrittenCatalog {
                catalog_path: request.catalog_path.clone(),
                entry_count: 7,
            })
        });
        let harness = harness(catalog_writer).await;
        let first = harness.publisher.publish("pc".to_owned());
        let second = harness.publisher.publish("pc".to_owned());
        let release = async move {
            did_start.recv().unwrap();
            release.send(()).unwrap();
        };

        let (first, second, ()) = tokio::join!(first, second, release);
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first.catalog_path, second.catalog_path);
        assert_eq!(first.entry_count, second.entry_count);
        harness.owner.shutdown().unwrap();
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn queued_publication_resolves_the_latest_durable_builder_generation() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed_catalogs = Arc::new(Mutex::new(Vec::new()));
        let (started, did_start) = crossbeam_channel::bounded(1);
        let (release, may_finish) = crossbeam_channel::bounded(1);
        let writer_calls = Arc::clone(&calls);
        let writer_catalogs = Arc::clone(&observed_catalogs);
        let catalog_writer: CatalogWriteFn = Arc::new(move |_db, request| {
            let call = writer_calls.fetch_add(1, Ordering::SeqCst);
            writer_catalogs
                .lock()
                .unwrap()
                .push(request.published_catalog.is_some());
            if call == 0 {
                started.send(()).unwrap();
                may_finish.recv().unwrap();
            }
            Ok(WrittenCatalog {
                catalog_path: request.catalog_path.clone(),
                entry_count: 7,
            })
        });
        let harness = harness(catalog_writer).await;
        let mut first = Box::pin(harness.publisher.publish("pc".to_owned()));
        assert!(poll!(first.as_mut()).is_pending());
        did_start.recv().unwrap();

        let catalog = builder_catalog("generation-two");
        let digest = worker_builder_catalog_digest(&catalog);
        harness
            .writer
            .replace_builder_catalog(ReplaceBuilderCatalog {
                workspace_pk: harness.workspace_id,
                expected: None,
                replacement: digest,
                builders: worker_builder_catalog_descriptors(&catalog),
                plan_delta: PlanDelta::default(),
                updated: 2,
            })
            .await
            .unwrap();
        harness.publisher.replace_builder_catalog(Some(catalog));

        let queued = harness.publisher.publish("ios".to_owned());
        let finish_first = async move { release.send(()).unwrap() };
        let (first, queued, ()) = tokio::join!(first, queued, finish_first);
        assert_eq!(first.unwrap().entry_count, 7);
        assert_eq!(queued.unwrap().entry_count, 7);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            observed_catalogs.lock().unwrap().as_slice(),
            &[false, true],
            "the queued request resolves the generation at write start, not RPC admission"
        );

        let fresh = harness.publisher.publish("ios".to_owned()).await.unwrap();
        assert!(fresh.reused, "Fresh belongs to the current generation only");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        harness.owner.shutdown().unwrap();
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn same_key_generation_change_keeps_old_waiters_separate_from_the_current_publication() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed_catalogs = Arc::new(Mutex::new(Vec::new()));
        let (started, did_start) = crossbeam_channel::bounded(1);
        let (release, may_finish) = crossbeam_channel::bounded(1);
        let writer_calls = Arc::clone(&calls);
        let writer_catalogs = Arc::clone(&observed_catalogs);
        let catalog_writer: CatalogWriteFn = Arc::new(move |_db, request| {
            let call = writer_calls.fetch_add(1, Ordering::SeqCst);
            writer_catalogs
                .lock()
                .unwrap()
                .push(request.published_catalog.is_some());
            if call == 0 {
                started.send(()).unwrap();
                may_finish.recv().unwrap();
            }
            Ok(WrittenCatalog {
                catalog_path: request.catalog_path.clone(),
                entry_count: u32::try_from(call + 1).unwrap(),
            })
        });
        let harness = harness(catalog_writer).await;
        let mut old = Box::pin(harness.publisher.publish("pc".to_owned()));
        assert!(poll!(old.as_mut()).is_pending());
        did_start.recv().unwrap();

        let catalog = builder_catalog("generation-two");
        let digest = worker_builder_catalog_digest(&catalog);
        harness
            .writer
            .replace_builder_catalog(ReplaceBuilderCatalog {
                workspace_pk: harness.workspace_id,
                expected: None,
                replacement: digest,
                builders: worker_builder_catalog_descriptors(&catalog),
                plan_delta: PlanDelta::default(),
                updated: 2,
            })
            .await
            .unwrap();
        harness.publisher.replace_builder_catalog(Some(catalog));

        let (current, accepted) = queued_catalog_request_with_acceptance(&harness.publisher, "pc");
        accepted
            .recv_timeout(Duration::from_secs(1))
            .expect("owner must accept the same-key request before the old write finishes");
        release.send(()).unwrap();

        let old = old.await.unwrap();
        let current = current.await.unwrap().into_result().unwrap();
        assert_eq!(old.entry_count, 1, "old waiters retain the old terminal");
        assert_eq!(
            current.entry_count, 2,
            "the later same-key waiter publishes the current generation"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(observed_catalogs.lock().unwrap().as_slice(), &[false, true]);

        let fresh = harness.publisher.publish("pc".to_owned()).await.unwrap();
        assert!(fresh.reused, "Fresh belongs only to the current generation");
        assert_eq!(fresh.entry_count, 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        harness.owner.shutdown().unwrap();
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn same_key_waiters_hold_admission_until_the_terminal_response() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (started, did_start) = crossbeam_channel::bounded(1);
        let (release, may_finish) = crossbeam_channel::bounded(1);
        let harness = harness(gated_writer(Arc::clone(&calls), started, may_finish, None)).await;
        let mut requests = (0..CATALOG_COMMAND_CAPACITY)
            .map(|_| Box::pin(harness.publisher.publish("pc".to_owned())))
            .collect::<Vec<_>>();
        for request in &mut requests {
            assert!(poll!(request.as_mut()).is_pending());
        }
        did_start.recv().unwrap();

        let overloaded = harness
            .publisher
            .publish("pc".to_owned())
            .await
            .unwrap_err();
        assert!(matches!(
            overloaded,
            AssetProcessorError::CatalogPublisherOverloaded
        ));

        release.send(()).unwrap();
        for request in requests {
            assert_eq!(request.await.unwrap().entry_count, 7);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        harness.owner.shutdown().unwrap();
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn same_key_waiters_share_the_same_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (started, did_start) = crossbeam_channel::bounded(1);
        let (release, may_finish) = crossbeam_channel::bounded(1);
        let harness = harness(gated_writer(
            Arc::clone(&calls),
            started,
            may_finish,
            Some("injected catalog failure"),
        ))
        .await;
        let first = harness.publisher.publish("pc".to_owned());
        let second = harness.publisher.publish("pc".to_owned());
        let release = async move {
            did_start.recv().unwrap();
            release.send(()).unwrap();
        };

        let (first, second, ()) = tokio::join!(first, second, release);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            first.unwrap_err().to_string(),
            second.unwrap_err().to_string()
        );
        harness.owner.shutdown().unwrap();
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn catalog_failure_records_health_after_the_last_waiter_drops() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (started, did_start) = crossbeam_channel::bounded(1);
        let (release, may_finish) = crossbeam_channel::bounded(1);
        let harness = harness(gated_writer(
            calls,
            started,
            may_finish,
            Some("injected catalog failure"),
        ))
        .await;
        let mut request = Box::pin(harness.publisher.publish("pc".to_owned()));
        assert!(poll!(request.as_mut()).is_pending());
        did_start.recv().unwrap();
        drop(request);

        release.send(()).unwrap();
        harness.owner.shutdown().unwrap();

        let (fault_count, fault) = harness.health.snapshot().unwrap();
        assert_eq!(fault_count, 1);
        assert!(matches!(
            fault,
            crate::AssetProcessorConsequenceFault::CatalogPublication {
                workspace_id,
                platform,
                reason,
            } if workspace_id == harness.workspace_id
                && platform == "pc"
                && reason.contains("injected catalog failure")
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn dropped_first_requester_cannot_create_a_second_writer() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (started, did_start) = crossbeam_channel::bounded(1);
        let (release, may_finish) = crossbeam_channel::bounded(1);
        let harness = harness(gated_writer(Arc::clone(&calls), started, may_finish, None)).await;
        let mut first = Box::pin(harness.publisher.publish("pc".to_owned()));
        assert!(poll!(first.as_mut()).is_pending());
        did_start.recv().unwrap();
        drop(first);

        let second = harness.publisher.publish("pc".to_owned());
        let release = async move { release.send(()).unwrap() };
        let (second, ()) = tokio::join!(second, release);
        assert_eq!(second.unwrap().entry_count, 7);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        harness.owner.shutdown().unwrap();
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn invalidation_during_write_leaves_projection_stale() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (started, did_start) = crossbeam_channel::bounded(1);
        let (release, may_finish) = crossbeam_channel::bounded(1);
        let writer_calls = Arc::clone(&calls);
        let catalog_writer: CatalogWriteFn = Arc::new(move |_db, request| {
            let call = writer_calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                started.send(()).unwrap();
                may_finish.recv().unwrap();
            }
            Ok(WrittenCatalog {
                catalog_path: request.catalog_path.clone(),
                entry_count: 7,
            })
        });
        let harness = harness(catalog_writer).await;
        let publish = harness.publisher.publish("pc".to_owned());
        let invalidate = async {
            did_start.recv().unwrap();
            harness.writer.record_post_commit_effect_for_test(
                PostCommitEffect::CatalogInvalidated {
                    workspace_pk: harness.workspace_id,
                    platform: Some("pc".to_owned()),
                },
            );
            release.send(()).unwrap();
        };
        let (published, ()) = tokio::join!(publish, invalidate);
        assert_eq!(published.unwrap().entry_count, 7);

        let republished = harness.publisher.publish("pc".to_owned()).await.unwrap();
        assert!(!republished.reused);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        harness.owner.shutdown().unwrap();
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn platform_invalidation_is_isolated_and_cursor_gap_invalidates_all() {
        let calls = Arc::new(AtomicUsize::new(0));
        let harness = harness(successful_writer(Arc::clone(&calls))).await;
        harness.publisher.publish("pc".to_owned()).await.unwrap();
        harness.publisher.publish("ios".to_owned()).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        harness
            .writer
            .record_post_commit_effect_for_test(PostCommitEffect::CatalogInvalidated {
                workspace_pk: harness.workspace_id,
                platform: Some("pc".to_owned()),
            });
        let ios = harness.publisher.publish("ios".to_owned()).await.unwrap();
        assert!(ios.reused);
        harness.publisher.publish("pc".to_owned()).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        for _ in 0..1_024 {
            harness.writer.record_post_commit_effect_for_test(
                PostCommitEffect::CatalogInvalidated {
                    workspace_pk: 999,
                    platform: None,
                },
            );
        }
        harness.publisher.publish("pc".to_owned()).await.unwrap();
        harness.publisher.publish("ios".to_owned()).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 5);
        harness.owner.shutdown().unwrap();
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn invalid_platform_is_rejected_before_writer_admission() {
        let calls = Arc::new(AtomicUsize::new(0));
        let harness = harness(successful_writer(Arc::clone(&calls))).await;
        let error = harness
            .publisher
            .publish("../pc".to_owned())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AssetProcessorError::InvalidReleaseContentPlatform { .. }
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        harness.owner.shutdown().unwrap();
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_drains_active_publication() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (started, did_start) = crossbeam_channel::bounded(1);
        let (release, may_finish) = crossbeam_channel::bounded(1);
        let harness = harness(gated_writer(Arc::clone(&calls), started, may_finish, None)).await;
        let mut published = Box::pin(harness.publisher.publish("pc".to_owned()));
        assert!(poll!(published.as_mut()).is_pending());
        did_start.recv().unwrap();
        let shutdown = tokio::task::spawn_blocking(move || harness.owner.shutdown());
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!shutdown.is_finished());
        release.send(()).unwrap();

        assert_eq!(published.await.unwrap().entry_count, 7);
        shutdown.await.unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[tokio::test(flavor = "current_thread")]
    async fn owner_drop_drains_active_publication() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (started, did_start) = crossbeam_channel::bounded(1);
        let (release, may_finish) = crossbeam_channel::bounded(1);
        let harness = harness(gated_writer(Arc::clone(&calls), started, may_finish, None)).await;
        let publisher = harness.publisher.clone();
        let mut in_flight = Box::pin(publisher.publish("pc".to_owned()));
        assert!(poll!(in_flight.as_mut()).is_pending());
        did_start.recv().unwrap();

        let owner = harness.owner;
        let dropped = tokio::task::spawn_blocking(move || drop(owner));
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(
            !dropped.is_finished(),
            "fallback Drop must wait for the active owner thread"
        );
        release.send(()).unwrap();

        assert_eq!(in_flight.await.unwrap().entry_count, 7);
        dropped.await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
