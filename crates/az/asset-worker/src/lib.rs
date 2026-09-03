//! Project-owned asset worker runtime.
//!
//! This crate contains the worker-side builder execution loop and its RPC
//! client. It deliberately has no asset database dependency: the worker leases
//! opaque job payloads from an asset processor and returns staged products.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use std::fmt;

use az_asset::{AssetCatalogPathRegistration, AssetId};
use az_asset_builder::JobContext;
use az_asset_builder::{
    AssetBuilderPattern, BuildRule, BuildRuleRegistry, BuilderId, CreateJobsRequest,
    CreateJobsResult, DEFAULT_PLATFORM_ID, ProcessJobRequest, ProcessJobResult, ProductDependency,
    ProductFormatId, SourceFileDependency, SourceFileEditCodecContext,
    SourceFileEditDocument as BuilderSourceFileEditDocument,
    SourceFileEditLoadRequest as BuilderSourceFileEditLoadRequest,
    SourceFileEditObject as BuilderSourceFileEditObject,
    SourceFileEditOperation as BuilderSourceFileEditOperation,
    SourceFileEditOperationRequest as BuilderSourceFileEditOperationRequest,
    SourceFileEditSaveRequest as BuilderSourceFileEditSaveRequest, SourceFileTemplateRegistration,
    SourcePath, SourceSchemaAuthoring as BuilderSourceSchemaAuthoring, SourceSchemaRegistration,
    SourceSchemaType, composed_product_format, composed_product_formats,
    composed_source_file_edit_codecs, composed_source_file_templates, composed_source_schemas,
};
use az_gem_contract::{
    Attributed, ComposeError, Composer, CompositionShutdownGate, GemTargetRole, ProcessComposition,
    ProcessCompositionCleanupError, Registries, RegistryLease, RegistryLeaseError,
};
use az_graph_builder::{
    RegisteredGraphSourceAuthoring, RegisteredGraphSourceSchema, graph_source_schemas,
};
use az_proto_asset::{
    ASSET_JOB_PLAN_ASSET_TYPE, ASSET_JOB_PLAN_PRODUCT_FORMAT, ASSET_JOB_PLAN_PRODUCT_PATH,
    ASSET_JOBS_PERMISSION, ASSET_PROCESSOR_AUDIENCE, ASSET_WORKER_SERVICE_NAME,
    ASSET_WORKER_SERVICE_NAMESPACE, AssetBuilderCatalogResult, AssetBuilderDescriptor,
    AssetBuilderPatternDescriptor, AssetBuilderPatternKind, AttemptStatus, CatalogPathRegistration,
    CompleteAssetJobAttemptRequest, JobOwner, LeaseAssetJobRequest, LeasedAssetJob,
    ProductFormatDescriptor, ProductManifest, ProductManifestProduct,
    ProductManifestProductDependency, PublishBuilderCatalogRequest, PublishBuilderCatalogResult,
    RenewAssetJobLeaseRequest, SourceFileCodecOperation, SourceFileCodecReplacement,
    SourceFileCodecRequest, SourceFileCodecResult, SourceFileEditDocument, SourceFileEditObject,
    SourceFileEditOperation, SourceFileTemplateDescriptor, SourceFileWorkflowDescriptor,
    SourceSchemaAuthoring, SourceSchemaDescriptor, asset_capnp, encode_product_manifest,
};
use az_proto_core::{
    Capability, Endpoint, ProtocolVersion, ServiceRole, SideChannelCapabilityError,
    SideChannelHandle, StagingFileSideChannelError, decode_capability_grant_set,
    read_verified_staging_file, validate_side_channel_capability_matches,
    validated_staging_file_path, write_named_staging_file_atomic,
};
use az_work::CancellationToken;
use capnp::Error;
use thiserror::Error;
use tracing::{debug, instrument, warn};

mod transport;
pub use transport::{AssetWorkerRpcTransportError, connect_asset_processor_rpc_client};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct WorkerCreateJobsPlan {
    builders: Vec<WorkerCreateJobsBuilderPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct WorkerCreateJobsBuilderPlan {
    builder_guid: uuid::Uuid,
    jobs: Vec<WorkerCreateJobsJobPlan>,
    source_dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct WorkerCreateJobsJobPlan {
    job_key: String,
    platform: String,
}
#[derive(Debug, Clone)]
pub struct AssetWorkerRunOnce {
    pub capability: Capability,
    pub lease_owner: String,
    pub lease_duration: Duration,
    pub staging_root: PathBuf,
    pub cache_root: PathBuf,
    pub cancellation: CancellationToken,
}

#[cfg(test)]
fn default_asset_worker_max_inflight() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

#[derive(Clone)]
struct AssetWorkerLoopConfig {
    pub asset_processor_endpoint: Endpoint,
    pub service_name: String,
    pub capability: Capability,
    pub lease_owner: String,
    pub lease_duration: Duration,
    /// Maximum concurrent leased job attempts processed inside this worker process.
    pub max_inflight: usize,
    pub staging_root: PathBuf,
    pub cache_root: PathBuf,
    pub cancellation: CancellationToken,
    registries: WorkerRegistries,
}

/// An owned read lease over the immutable registry set used by worker tasks.
///
/// Production uses [`WorkerRegistries::Composed`], which is kept private so a
/// caller cannot detach the registry from the worker process that owns its
/// contribution lifecycle. The static variant preserves the existing narrow
/// standalone test surface.
#[derive(Clone)]
enum WorkerRegistries {
    #[cfg(any(test, feature = "test-support"))]
    Static(&'static Registries),
    Composed(RegistryLease),
}

impl WorkerRegistries {
    fn as_ref(&self) -> &Registries {
        match self {
            #[cfg(any(test, feature = "test-support"))]
            Self::Static(registries) => registries,
            Self::Composed(registries) => registries.as_ref(),
        }
    }
}

impl fmt::Debug for AssetWorkerLoopConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetWorkerLoopConfig")
            .field("service_name", &self.service_name)
            .field("lease_owner", &self.lease_owner)
            .field("max_inflight", &self.max_inflight)
            .field("staging_root", &self.staging_root)
            .field("cache_root", &self.cache_root)
            .finish_non_exhaustive()
    }
}

impl AssetWorkerLoopConfig {
    #[cfg(test)]
    #[must_use]
    fn new(
        asset_processor_endpoint: Endpoint,
        service_name: impl Into<String>,
        capability: Capability,
        staging_root: impl Into<PathBuf>,
        cache_root: impl Into<PathBuf>,
        registries: &'static Registries,
    ) -> Self {
        let service_name = service_name.into();
        Self {
            asset_processor_endpoint,
            service_name: service_name.clone(),
            capability,
            lease_owner: format!("{service_name}:{}", std::process::id()),
            lease_duration: Duration::from_secs(30),
            max_inflight: default_asset_worker_max_inflight(),
            staging_root: staging_root.into(),
            cache_root: cache_root.into(),
            cancellation: CancellationToken::new(),
            registries: WorkerRegistries::Static(registries),
        }
    }

    fn from_composed(
        asset_processor_endpoint: Endpoint,
        service_name: String,
        capability: Capability,
        staging_root: PathBuf,
        cache_root: PathBuf,
        max_inflight: usize,
        registries: RegistryLease,
    ) -> Self {
        Self {
            asset_processor_endpoint,
            lease_owner: format!("{service_name}:{}", std::process::id()),
            service_name,
            capability,
            lease_duration: Duration::from_secs(30),
            max_inflight: max_inflight.max(1),
            staging_root,
            cache_root,
            cancellation: CancellationToken::new(),
            registries: WorkerRegistries::Composed(registries),
        }
    }
}

/// Immutable process inputs for one composed asset-worker service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetWorkerServiceStartup {
    pub asset_processor_endpoint: Endpoint,
    pub service_name: String,
    pub capability_grants_file: PathBuf,
    pub staging_root: PathBuf,
    pub cache_root: PathBuf,
    pub max_inflight: usize,
}

struct AssetWorkerComposition {
    // Field order is the teardown order. The worker's final registry lease
    // must leave before the composition tries contribution cleanup.
    registries: RegistryLease,
    process: ProcessComposition,
}

impl AssetWorkerComposition {
    fn from_composer(composer: Composer) -> Result<Self, AssetWorkerError> {
        let actual = composer.host().role();
        if actual != GemTargetRole::AssetWorker {
            return Err(AssetWorkerError::WrongCompositionRole { actual });
        }
        let process = ProcessComposition::new(composer)?;
        if !process.is_ready() {
            return Err(AssetWorkerError::CompositionNotReady);
        }
        let registries = process
            .registry_lease()
            .map_err(AssetWorkerError::RegistryLease)?;
        Ok(Self {
            registries,
            process,
        })
    }
}

/// Owns one finalized worker composition and every task that reads it.
///
/// Construction publishes the worker's catalog before the service becomes
/// ready. [`Self::run`] joins the worker loop before releasing the composition,
/// so contribution finish/cleanup cannot race a builder or source-codec task.
pub struct AssetWorkerService {
    prepared: PreparedAssetWorkerLoop,
    composition: AssetWorkerComposition,
}

/// Entry-point-owned shutdown capability for one worker service. It first
/// closes composition lease issuance, then cancels the loop; `run` joins the
/// loop and performs finish/cleanup afterward.
#[derive(Clone)]
pub struct AssetWorkerShutdown {
    composition: CompositionShutdownGate,
    cancellation: CancellationToken,
}

#[derive(Debug, Error)]
#[error("asset-worker shutdown request failed: {0}")]
pub struct AssetWorkerShutdownError(#[from] ProcessCompositionCleanupError);

impl AssetWorkerShutdown {
    /// # Errors
    ///
    /// Returns [`AssetWorkerShutdownError`] if the process composition refuses
    /// to begin shutdown. The cancellation token is still tripped either way.
    pub fn request(&self) -> Result<(), AssetWorkerShutdownError> {
        let result = self.composition.begin_shutdown().map_err(Into::into);
        self.cancellation.cancel();
        result
    }
}

impl AssetWorkerService {
    /// Finalize the generated `AssetWorker` composition, validate the brokered
    /// worker grant, and publish the composed catalog to the Asset Processor.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal before catalog publication when the composition
    /// belongs to another role, is not ready, or its grants/roots are invalid.
    /// A transport or catalog-publication failure leaves no runnable service.
    pub fn start(
        startup: AssetWorkerServiceStartup,
        composer: Composer,
    ) -> Result<Self, AssetWorkerError> {
        let composition = AssetWorkerComposition::from_composer(composer)?;
        let capability = load_asset_worker_jobs_capability_grant(&startup.capability_grants_file)?;
        let config = AssetWorkerLoopConfig::from_composed(
            startup.asset_processor_endpoint,
            startup.service_name,
            capability,
            startup.staging_root,
            startup.cache_root,
            startup.max_inflight,
            composition.registries.clone(),
        );
        let prepared = prepare_asset_worker_loop(config)?;
        Ok(Self {
            prepared,
            composition,
        })
    }

    /// Run until the worker cancellation boundary closes.
    ///
    /// The loop owns and joins every slot before the composition is dropped.
    ///
    /// # Errors
    ///
    /// Returns [`AssetWorkerError::Cleanup`] if the composition refuses to
    /// begin shutdown or to clean up, and otherwise whatever error the run loop
    /// itself produced.
    pub fn run(self) -> Result<(), AssetWorkerError> {
        let Self {
            prepared,
            composition,
        } = self;
        let result = prepared.run();
        let AssetWorkerComposition {
            registries,
            mut process,
        } = composition;
        drop(registries);
        process
            .begin_shutdown()
            .map_err(AssetWorkerError::Cleanup)?;
        process.cleanup().map_err(AssetWorkerError::Cleanup)?;
        result
    }

    /// Begin quiescence before the entrypoint cancels and joins worker slots.
    #[must_use]
    pub fn shutdown_handle(&self) -> AssetWorkerShutdown {
        AssetWorkerShutdown {
            composition: self.composition.process.shutdown_gate(),
            cancellation: self.prepared.config.cancellation.clone(),
        }
    }
}

struct PreparedAssetWorkerLoop {
    config: AssetWorkerLoopConfig,
    runtime: tokio::runtime::Runtime,
    local: tokio::task::LocalSet,
    client: az_rpc::ScopedTwopartyClient<asset_capnp::asset_processor::Client>,
    capability: Capability,
}

impl PreparedAssetWorkerLoop {
    fn run(self) -> Result<(), AssetWorkerError> {
        let max_inflight = self.config.max_inflight.max(1);
        let result = {
            let local = &self.local;
            let runtime = &self.runtime;
            local.block_on(runtime, self.run_inflight_pool(max_inflight))
        };
        let client = self.client;
        let disconnect = self.local.block_on(&self.runtime, client.disconnect());
        result.and_then(|()| disconnect.map_err(AssetWorkerError::Rpc))
    }

    // The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
    // neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
    // replaced, so it is single-threaded by construction.
    #[allow(clippy::future_not_send)]
    async fn run_inflight_pool(&self, max_inflight: usize) -> Result<(), AssetWorkerError> {
        // Every slot is a long-lived rolling consumer. A slot immediately leases
        // its next attempt after finishing the previous one, so an unrelated
        // slow builder cannot hold idle capacity behind a fixed-batch barrier.
        //
        // The slots are joined futures so service shutdown observes all worker
        // work before it releases the finalized registry lease. Blocking build
        // work receives its own owned read lease to keep the heartbeat alive.
        let slots = (0..max_inflight).map(|slot| self.run_slot(slot, max_inflight));
        futures::future::try_join_all(slots).await?;

        if self.config.cancellation.is_cancelled() {
            Ok(())
        } else {
            Err(AssetWorkerError::Runtime(std::io::Error::other(
                "asset worker pool exited while cancellation was not requested",
            )))
        }
    }

    // The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
    // neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
    // replaced, so it is single-threaded by construction.
    #[allow(clippy::future_not_send)]
    async fn run_slot(&self, slot: usize, max_inflight: usize) -> Result<(), AssetWorkerError> {
        let lease_owner = if max_inflight == 1 {
            self.config.lease_owner.clone()
        } else {
            format!("{}:slot{slot}", self.config.lease_owner)
        };
        let cancellation = &self.config.cancellation;
        let builders =
            BuildRuleRegistry::compose(&JobContext::new(self.config.registries.as_ref()));

        loop {
            if cancellation.is_cancelled() {
                return Ok(());
            }

            let outcome = run_asset_worker_once_with_registries(
                self.client.client(),
                &builders,
                self.config.registries.clone(),
                AssetWorkerRunOnce {
                    capability: self.capability.clone(),
                    lease_owner: lease_owner.clone(),
                    lease_duration: self.config.lease_duration,
                    staging_root: self.config.staging_root.clone(),
                    cache_root: self.config.cache_root.clone(),
                    cancellation: cancellation.clone(),
                },
            )
            .await?;

            match outcome {
                AssetWorkerRunOutcome::Cancelled { .. } => return Ok(()),
                AssetWorkerRunOutcome::Completed { .. } | AssetWorkerRunOutcome::Failed { .. } => {}
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetWorkerRunOutcome {
    Completed {
        asset_job_attempt_id: i64,
        product_count: usize,
    },
    Failed {
        asset_job_attempt_id: i64,
    },
    Cancelled {
        asset_job_attempt_id: Option<i64>,
    },
}

enum PreparedAssetWorkerJob {
    Succeeded {
        /// Boxed: inline, this handle made `Succeeded` dwarf the two unit
        /// variants beside it.
        manifest_handle: Box<SideChannelHandle>,
        product_count: usize,
    },
    Failed,
    Cancelled,
}

struct AttemptStagingCleanup {
    root: PathBuf,
}

impl AttemptStagingCleanup {
    const fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Drop for AttemptStagingCleanup {
    fn drop(&mut self) {
        if let Err(source) = remove_ephemeral_staging_tree(&self.root) {
            warn!(
                path = %self.root.display(),
                %source,
                "asset worker failed to remove terminal attempt staging"
            );
        }
    }
}

#[derive(Debug, Error)]
pub enum AssetWorkerError {
    #[error("asset worker received a composition for `{actual}`")]
    WrongCompositionRole { actual: GemTargetRole },

    #[error(transparent)]
    Composition(#[from] ComposeError),
    #[error(transparent)]
    RegistryLease(#[from] RegistryLeaseError),
    #[error(transparent)]
    Cleanup(#[from] ProcessCompositionCleanupError),

    #[error("asset worker composition is not ready for service publication")]
    CompositionNotReady,

    #[error("asset worker RPC failed: {0}")]
    Rpc(#[from] Error),

    #[error("asset worker transport failed: {0}")]
    Transport(#[from] AssetWorkerRpcTransportError),

    #[error("asset worker runtime failed to start: {0}")]
    Runtime(#[source] std::io::Error),

    #[error("asset worker blocking job task failed for attempt {asset_job_attempt_id}: {source}")]
    BlockingJobJoin {
        asset_job_attempt_id: i64,
        #[source]
        source: tokio::task::JoinError,
    },

    #[error("asset worker clock failed: {0}")]
    Clock(#[from] std::time::SystemTimeError),

    #[error("asset worker capability grant file `{path}` read failed: {source}")]
    CapabilityGrantRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("asset worker capability grant decode failed: {0}")]
    CapabilityGrantDecode(#[source] Error),

    #[error("asset worker grant file has no project-scoped asset.jobs capability")]
    MissingJobsCapabilityGrant,

    #[error("asset worker jobs capability is invalid: {reason}")]
    InvalidJobsCapability { reason: String },

    #[error("asset worker {flag} `{path}` must be an absolute normalized path")]
    InvalidWorkerRoot { flag: &'static str, path: String },

    #[error("asset worker failed to prepare staging root `{path}`: {source}")]
    PrepareStagingRoot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("leased attempt {asset_job_attempt_id} did not return a staging root")]
    MissingAttemptStagingRoot { asset_job_attempt_id: i64 },

    #[error(
        "leased attempt {asset_job_attempt_id} returned staging root `{actual}` instead of requested worker-owned root `{expected}`"
    )]
    UnexpectedAttemptStagingRoot {
        asset_job_attempt_id: i64,
        expected: PathBuf,
        actual: PathBuf,
    },

    #[error("leased attempt {asset_job_attempt_id} references unknown builder {builder_guid}")]
    UnknownBuilder {
        asset_job_attempt_id: i64,
        builder_guid: uuid::Uuid,
    },

    #[error("worker builder catalog construction failed: {0}")]
    BuilderCatalog(String),

    #[error("worker builder catalog construction failed: {reason}")]
    BuilderCatalogInvalid { reason: String },

    #[error("asset processor rejected worker builder catalog publication")]
    BuilderCatalogPublishRejected,

    #[error("create_jobs planner job failed for attempt {asset_job_attempt_id}: {reason}")]
    PlannerCreateJobsFailed {
        asset_job_attempt_id: i64,
        reason: String,
    },

    #[error("source path `{source_path}` for attempt {asset_job_attempt_id} is not safe relative")]
    UnsafeSourcePath {
        asset_job_attempt_id: i64,
        source_path: String,
    },

    #[error("leased attempt {asset_job_attempt_id} did not return a DB-authored source payload")]
    MissingSourcePayload { asset_job_attempt_id: i64 },

    #[error(
        "product path `{product_path}` for attempt {asset_job_attempt_id} is not safe relative"
    )]
    UnsafeProductPath {
        asset_job_attempt_id: i64,
        product_path: String,
    },

    #[error(
        "shared product path `{product_path}` for attempt {asset_job_attempt_id} is invalid: {reason}"
    )]
    InvalidSharedProductPath {
        asset_job_attempt_id: i64,
        product_path: String,
        reason: String,
    },

    #[error("invalid product dependency for attempt {asset_job_attempt_id}: {reason}")]
    InvalidProductDependency {
        asset_job_attempt_id: i64,
        reason: String,
    },

    #[error(
        "builder `{builder}` emitted undeclared product format `{product_format}` for product `{product_path}` in attempt {asset_job_attempt_id}"
    )]
    UndeclaredProductFormat {
        asset_job_attempt_id: i64,
        builder: &'static str,
        product_path: String,
        product_format: String,
    },

    #[error(
        "product `{product_path}` for attempt {asset_job_attempt_id} uses unregistered product format `{product_format}`"
    )]
    UnregisteredProductFormat {
        asset_job_attempt_id: i64,
        product_path: String,
        product_format: String,
    },

    #[error(
        "product `{product_path}` for attempt {asset_job_attempt_id} uses unsupported version {version} for product format `{product_format}` (registered current version {current_version})"
    )]
    UnsupportedProductFormatVersion {
        asset_job_attempt_id: i64,
        product_path: String,
        product_format: String,
        version: u32,
        current_version: u32,
    },

    #[error("failed to write product `{path}` for attempt {asset_job_attempt_id}: {source}")]
    WriteProduct {
        asset_job_attempt_id: i64,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "failed to create product parent `{path}` for attempt {asset_job_attempt_id}: {source}"
    )]
    CreateProductParent {
        asset_job_attempt_id: i64,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "failed to write product manifest `{path}` for attempt {asset_job_attempt_id}: {source}"
    )]
    WriteManifest {
        asset_job_attempt_id: i64,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("asset processor rejected completion for attempt {asset_job_attempt_id}")]
    CompletionRejected { asset_job_attempt_id: i64 },

    #[error("asset processor rejected lease renewal for attempt {asset_job_attempt_id}")]
    LeaseRenewalRejected { asset_job_attempt_id: i64 },

    #[error(
        "source payload side-channel capability failed for attempt {asset_job_attempt_id}: {source}"
    )]
    SourcePayloadCapability {
        asset_job_attempt_id: i64,
        #[source]
        source: SideChannelCapabilityError,
    },

    #[error(
        "failed to read source payload side channel for attempt {asset_job_attempt_id}: {source}"
    )]
    ReadSourcePayload {
        asset_job_attempt_id: i64,
        #[source]
        source: StagingFileSideChannelError,
    },
}

async fn run_asset_worker_blocking_task<T>(
    asset_job_attempt_id: i64,
    task: impl FnOnce() -> Result<T, AssetWorkerError> + Send + 'static,
) -> Result<T, AssetWorkerError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|source| AssetWorkerError::BlockingJobJoin {
            asset_job_attempt_id,
            source,
        })?
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
async fn run_asset_worker_blocking_task_with_lease<T>(
    client: &asset_capnp::asset_processor::Client,
    request: &AssetWorkerRunOnce,
    asset_job_attempt_id: i64,
    grant_key: uuid::Uuid,
    task: impl FnOnce() -> Result<T, AssetWorkerError> + Send + 'static,
) -> Result<T, AssetWorkerError>
where
    T: Send + 'static,
{
    let heartbeat_period = (request.lease_duration / 3).max(Duration::from_millis(1));
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + heartbeat_period,
        heartbeat_period,
    );
    let mut blocking = Box::pin(run_asset_worker_blocking_task(asset_job_attempt_id, task));

    loop {
        tokio::select! {
            biased;
            result = &mut blocking => return result,
            _ = heartbeat.tick() => {
                let renewed = async {
                    let mut renew_request = client.renew_lease_request();
                    (RenewAssetJobLeaseRequest {
                        capability: request.capability.clone(),
                        asset_job_attempt_id,
                        lease_owner: request.lease_owner.clone(),
                        grant_key,
                    })
                    .to_capnp(renew_request.get().init_request())?;
                    let renew_response = renew_request.send().promise.await?;
                    Ok::<_, AssetWorkerError>(renew_response.get()?.get_renewed())
                }
                .await;
                if !matches!(renewed, Ok(true)) {
                    // A spawn_blocking task cannot be stopped once running. Wait for it
                    // before the caller drops its staging cleanup guard, then discard
                    // the result because this worker no longer owns the attempt.
                    let _ = blocking.await;
                    return match renewed {
                        Ok(false) => Err(AssetWorkerError::LeaseRenewalRejected {
                            asset_job_attempt_id,
                        }),
                        Err(error) => Err(error),
                        Ok(true) => unreachable!("successful renewal handled above"),
                    };
                }
            }
        }
    }
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub async fn run_asset_worker_blocking_task_with_lease_for_test<T>(
    client: &asset_capnp::asset_processor::Client,
    request: &AssetWorkerRunOnce,
    asset_job_attempt_id: i64,
    grant_key: uuid::Uuid,
    task: impl FnOnce() -> Result<T, AssetWorkerError> + Send + 'static,
) -> Result<T, AssetWorkerError>
where
    T: Send + 'static,
{
    run_asset_worker_blocking_task_with_lease(
        client,
        request,
        asset_job_attempt_id,
        grant_key,
        task,
    )
    .await
}

fn asset_worker_runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

#[instrument(skip(config), fields(service_name = %config.service_name))]
fn prepare_asset_worker_loop(
    config: AssetWorkerLoopConfig,
) -> Result<PreparedAssetWorkerLoop, AssetWorkerError> {
    validate_asset_worker_jobs_capability(&config.capability)?;
    validate_asset_worker_root("--staging-root", &config.staging_root)?;
    validate_asset_worker_root("--cache-root", &config.cache_root)?;
    prepare_asset_worker_staging_root(&config.staging_root)?;
    let runtime = asset_worker_runtime().map_err(AssetWorkerError::Runtime)?;
    let local = tokio::task::LocalSet::new();
    let client = local.block_on(
        &runtime,
        connect_asset_processor_rpc_client(&config.asset_processor_endpoint),
    )?;
    let registries = config.registries.clone();
    let builders = BuildRuleRegistry::compose(&JobContext::new(registries.as_ref()));
    let capability = config.capability.clone();
    let source_file_codec: asset_capnp::source_file_codec::Client =
        capnp_rpc::new_client(SourceFileCodecServer {
            registries: registries.clone(),
        });
    // Publish the project-linked builder/schema catalog to the engine host so
    // the host can answer builderCatalog without linking project gems.
    local.block_on(
        &runtime,
        publish_worker_builder_catalog(
            client.client(),
            &capability,
            &builders,
            registries.as_ref(),
            source_file_codec,
        ),
    )?;
    Ok(PreparedAssetWorkerLoop {
        config,
        runtime,
        local,
        client,
        capability,
    })
}

struct SourceFileCodecServer {
    registries: WorkerRegistries,
}

impl asset_capnp::source_file_codec::Server for SourceFileCodecServer {
    // The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
    // neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
    // replaced, so it is single-threaded by construction.
    #[allow(clippy::future_not_send)]
    async fn execute(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::source_file_codec::ExecuteParams,
        mut results: asset_capnp::source_file_codec::ExecuteResults,
    ) -> Result<(), Error> {
        let request = SourceFileCodecRequest::from_capnp(params.get()?.get_request()?)?;
        let registries = self.registries.clone();
        let result = tokio::task::spawn_blocking(move || {
            execute_source_file_codec_request(registries.as_ref(), &request)
        })
        .await
        .map_err(|error| Error::failed(format!("source-file codec task failed: {error}")))??;
        result.to_capnp(results.get().init_result())?;
        Ok(())
    }
}

#[cfg_attr(feature = "test-support", allow(dead_code))]
fn execute_source_file_codec_request(
    registries: &Registries,
    request: &SourceFileCodecRequest,
) -> Result<SourceFileCodecResult, Error> {
    validate_source_file_codec_output_destination(request)?;
    let source = read_verified_staging_file(&request.authoritative_source)
        .map_err(|error| Error::failed(format!("read authoritative source: {error}")))?;
    let codec = composed_source_file_edit_codec_for_wire(registries, &request.source.schema_type)?;
    let context = SourceFileEditCodecContext::new(registries);
    let schema_type = codec.schema_type();
    let loaded = codec
        .load(
            context,
            &BuilderSourceFileEditLoadRequest {
                schema_type,
                source_path: &request.source.source_path,
                bytes: &source.bytes,
            },
        )
        .map_err(source_file_codec_error)?;

    match &request.operation {
        SourceFileCodecOperation::Load => Ok(SourceFileCodecResult {
            document: builder_source_file_edit_document_to_proto(loaded),
            replacement: SourceFileCodecReplacement::Unchanged,
        }),
        SourceFileCodecOperation::Edit(operation) => {
            let operation = source_file_edit_operation_to_builder(operation);
            let edited = codec
                .edit(
                    context,
                    &BuilderSourceFileEditOperationRequest::new(
                        schema_type,
                        &request.source.source_path,
                        &loaded,
                        &operation,
                    ),
                )
                .map_err(source_file_codec_error)?;
            save_and_reload_source_file_edit_document(codec, context, request, schema_type, &edited)
        }
        SourceFileCodecOperation::RestoreDocument(document) => {
            let document = proto_source_file_edit_document_to_builder(document);
            save_and_reload_source_file_edit_document(
                codec,
                context,
                request,
                schema_type,
                &document,
            )
        }
    }
}

/// Executes the worker's source-file codec boundary for an in-process
/// integration test. Production callers use the Cap'n Proto server above.
///
/// # Errors
///
/// Returns any error the codec boundary returns: an output destination whose
/// platform or root does not match this worker, an unregistered or
/// non-file-backed schema, or a codec that rejects the request payload.
#[cfg(feature = "test-support")]
pub fn execute_source_file_codec_request_for_test(
    registries: &Registries,
    request: &SourceFileCodecRequest,
) -> Result<SourceFileCodecResult, Error> {
    execute_source_file_codec_request(registries, request)
}

fn validate_source_file_codec_output_destination(
    request: &SourceFileCodecRequest,
) -> Result<(), Error> {
    let destination = &request.output_destination;
    if destination.platform != std::env::consts::OS {
        return Err(Error::failed(format!(
            "source-file codec output platform `{}` does not match worker platform `{}`",
            destination.platform,
            std::env::consts::OS
        )));
    }
    let authoritative_capability = request
        .authoritative_source
        .capability
        .as_ref()
        .ok_or_else(|| Error::failed("authoritative source has no brokered capability".into()))?;
    if authoritative_capability != &destination.capability {
        return Err(Error::failed(
            "source-file codec output destination capability does not match worker capability"
                .into(),
        ));
    }
    let authoritative_path = validated_staging_file_path(&request.authoritative_source)
        .map_err(|error| Error::failed(format!("validate authoritative source: {error}")))?;
    let output_path = PathBuf::from(&destination.locator);
    if output_path == authoritative_path {
        return Err(Error::failed(
            "source-file codec output destination aliases authoritative source".into(),
        ));
    }
    Ok(())
}

fn composed_source_file_edit_codec_for_wire(
    registries: &Registries,
    schema_type: &str,
) -> Result<az_asset_builder::SourceFileEditCodecRegistration, Error> {
    composed_source_file_edit_codecs(registries)
        .into_iter()
        .find(|attributed| attributed.entry.schema_type().as_str() == schema_type)
        .map(|attributed| attributed.entry)
        .ok_or_else(|| {
            Error::failed(format!(
                "source schema `{schema_type}` has no composed source-file edit codec"
            ))
        })
}

fn source_file_edit_operation_to_builder(
    operation: &SourceFileEditOperation,
) -> BuilderSourceFileEditOperation {
    match operation {
        SourceFileEditOperation::AppendDefault => BuilderSourceFileEditOperation::AppendDefault,
        SourceFileEditOperation::DuplicateObject { object_id } => {
            BuilderSourceFileEditOperation::Duplicate {
                object_id: object_id.clone(),
            }
        }
        SourceFileEditOperation::RemoveObject { object_id } => {
            BuilderSourceFileEditOperation::Remove {
                object_id: object_id.clone(),
            }
        }
    }
}

fn builder_source_file_edit_document_to_proto(
    document: BuilderSourceFileEditDocument,
) -> SourceFileEditDocument {
    SourceFileEditDocument {
        root_object_id: document.root_object_id,
        root_schema: document.root_schema,
        value: document.value,
        objects: document
            .objects
            .into_iter()
            .map(|object| SourceFileEditObject {
                object_id: object.object_id,
                schema: object.schema,
                value: object.value,
            })
            .collect(),
        codec_state: document.codec_state,
    }
}

fn proto_source_file_edit_document_to_builder(
    document: &SourceFileEditDocument,
) -> BuilderSourceFileEditDocument {
    BuilderSourceFileEditDocument {
        root_object_id: document.root_object_id.clone(),
        root_schema: document.root_schema.clone(),
        value: document.value.clone(),
        objects: document
            .objects
            .iter()
            .map(|object| BuilderSourceFileEditObject {
                object_id: object.object_id.clone(),
                schema: object.schema.clone(),
                value: object.value.clone(),
            })
            .collect(),
        codec_state: document.codec_state.clone(),
    }
}

fn save_and_reload_source_file_edit_document(
    codec: az_asset_builder::SourceFileEditCodecRegistration,
    context: SourceFileEditCodecContext<'_>,
    request: &SourceFileCodecRequest,
    schema_type: SourceSchemaType,
    document: &BuilderSourceFileEditDocument,
) -> Result<SourceFileCodecResult, Error> {
    let saved = codec
        .save(
            context,
            &BuilderSourceFileEditSaveRequest::new(
                schema_type,
                &request.source.source_path,
                &document.root_schema,
                &document.value,
                &document.objects,
                &document.codec_state,
            ),
        )
        .map_err(source_file_codec_error)?;
    let canonical = codec
        .load(
            context,
            &BuilderSourceFileEditLoadRequest {
                schema_type,
                source_path: &request.source.source_path,
                bytes: &saved,
            },
        )
        .map_err(source_file_codec_error)?;
    let path = PathBuf::from(&request.output_destination.locator);
    let written = write_named_staging_file_atomic(&path, &saved)
        .map_err(|error| Error::failed(format!("stage saved source: {error}")))?;
    let saved_source = SideChannelHandle::staging_file(
        written.path.to_string_lossy(),
        written.byte_length,
        written.content_hash,
        std::env::consts::OS,
    )
    .with_capability(request.output_destination.capability.clone());
    Ok(SourceFileCodecResult {
        document: builder_source_file_edit_document_to_proto(canonical),
        replacement: SourceFileCodecReplacement::SavedSource(Box::new(saved_source)),
    })
}

// Passed directly to `Result::map_err`, which hands the error over by value, so this
// cannot take it by reference without wrapping every call site in a closure.
#[allow(clippy::needless_pass_by_value)]
fn source_file_codec_error(error: az_asset_builder::SourceFileEditError) -> Error {
    Error::failed(error.to_string())
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
async fn publish_worker_builder_catalog(
    client: &asset_capnp::asset_processor::Client,
    capability: &Capability,
    builders: &BuildRuleRegistry,
    registries: &Registries,
    source_file_codec: asset_capnp::source_file_codec::Client,
) -> Result<(), AssetWorkerError> {
    let catalog = worker_builder_catalog(builders, registries)?;
    let mut request = client.publish_builder_catalog_request();
    (PublishBuilderCatalogRequest {
        capability: capability.clone(),
        protocol: ProtocolVersion::CURRENT,
        catalog,
    })
    .to_capnp(request.get().init_request())
    .map_err(AssetWorkerError::Rpc)?;
    request.get().set_codec(source_file_codec);
    let response = request
        .send()
        .promise
        .await
        .map_err(AssetWorkerError::Rpc)?;
    let result = PublishBuilderCatalogResult::from_capnp(response.get()?.get_result()?);
    if !result.accepted {
        return Err(AssetWorkerError::BuilderCatalogPublishRejected);
    }
    Ok(())
}

fn worker_builder_catalog(
    builders: &BuildRuleRegistry,
    registries: &Registries,
) -> Result<AssetBuilderCatalogResult, AssetWorkerError> {
    let builders = builders
        .iter()
        .map(asset_builder_to_proto)
        .collect::<Vec<_>>();
    let source_schema_registrations = composed_source_schemas(registries);
    let source_file_template_registrations = composed_source_file_templates(registries);
    ensure_source_file_template_registrations_match_schemas(
        &source_schema_registrations,
        &source_file_template_registrations,
    )
    .map_err(|error| AssetWorkerError::BuilderCatalog(error.to_string()))?;
    let mut source_schemas = source_schema_registrations
        .into_iter()
        .map(|attributed| source_schema_to_proto(&attributed, &source_file_template_registrations))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AssetWorkerError::BuilderCatalog(error.to_string()))?;
    source_schemas.extend(
        graph_source_schemas_to_proto(registries)
            .map_err(|error| AssetWorkerError::BuilderCatalog(error.to_string()))?,
    );
    source_schemas.sort_by(|left, right| {
        left.schema_type
            .cmp(&right.schema_type)
            .then_with(|| left.owner.cmp(&right.owner))
    });
    ensure_unique_source_schema_descriptors(&source_schemas)
        .map_err(|error| AssetWorkerError::BuilderCatalog(error.to_string()))?;
    ensure_builder_catalog_source_schema_coverage(&builders, &source_schemas)
        .map_err(|error| AssetWorkerError::BuilderCatalog(error.to_string()))?;
    Ok(AssetBuilderCatalogResult {
        builders,
        source_schemas,
        product_formats: composed_product_formats_to_proto(registries),
    })
}

fn validate_asset_worker_root(flag: &'static str, path: &Path) -> Result<(), AssetWorkerError> {
    if is_normal_absolute_path(path) {
        Ok(())
    } else {
        Err(AssetWorkerError::InvalidWorkerRoot {
            flag,
            path: path.display().to_string(),
        })
    }
}

fn validate_asset_worker_jobs_capability(capability: &Capability) -> Result<(), AssetWorkerError> {
    if capability.role != ServiceRole::Worker {
        return Err(invalid_worker_jobs_capability(format!(
            "expected role Worker but got {:?}",
            capability.role
        )));
    }
    if capability.service.namespace != ASSET_WORKER_SERVICE_NAMESPACE
        || capability.service.name != ASSET_WORKER_SERVICE_NAME
    {
        return Err(invalid_worker_jobs_capability(format!(
            "expected service `{ASSET_WORKER_SERVICE_NAMESPACE}/{ASSET_WORKER_SERVICE_NAME}` but got `{}/{}`",
            capability.service.namespace, capability.service.name
        )));
    }
    if let Some(session) = capability.session {
        return Err(invalid_worker_jobs_capability(format!(
            "expected project scope but got session `{session}`"
        )));
    }
    if capability.audience != ASSET_PROCESSOR_AUDIENCE {
        return Err(invalid_worker_jobs_capability(format!(
            "expected audience `{ASSET_PROCESSOR_AUDIENCE}` but got `{}`",
            capability.audience
        )));
    }
    if !capability.has_permissions(&[ASSET_JOBS_PERMISSION]) {
        return Err(invalid_worker_jobs_capability(format!(
            "missing required permission `{ASSET_JOBS_PERMISSION}`"
        )));
    }
    if capability.token_hash.is_empty() {
        return Err(invalid_worker_jobs_capability(
            "missing brokered token hash".to_string(),
        ));
    }
    if let Err(error) = capability.validate_lifetime() {
        return Err(invalid_worker_jobs_capability(error.to_string()));
    }
    Ok(())
}

const fn invalid_worker_jobs_capability(reason: String) -> AssetWorkerError {
    AssetWorkerError::InvalidJobsCapability { reason }
}

fn matches_worker_jobs_capability_identity(capability: &Capability) -> bool {
    capability.service.namespace == ASSET_WORKER_SERVICE_NAMESPACE
        && capability.service.name == ASSET_WORKER_SERVICE_NAME
        && capability.role == ServiceRole::Worker
        && capability.session.is_none()
        && capability.audience == ASSET_PROCESSOR_AUDIENCE
        && capability.has_permissions(&[ASSET_JOBS_PERMISSION])
        && !capability.token_hash.is_empty()
}

/// # Errors
///
/// Returns [`AssetWorkerError::CapabilityGrantRead`] if the grants file cannot
/// be read, [`AssetWorkerError::CapabilityGrantDecode`] if its bytes are not a
/// decodable grant set, and a missing-grant refusal if the set carries no
/// capability with the asset-jobs permission and a non-empty token hash.
pub fn load_asset_worker_jobs_capability_grant(
    capability_grants_file: impl AsRef<Path>,
) -> Result<Capability, AssetWorkerError> {
    let path = capability_grants_file.as_ref();
    let bytes = std::fs::read(path).map_err(|source| AssetWorkerError::CapabilityGrantRead {
        path: path.to_path_buf(),
        source,
    })?;
    let grant_set =
        decode_capability_grant_set(&bytes).map_err(AssetWorkerError::CapabilityGrantDecode)?;
    let capability = grant_set
        .grants()
        .iter()
        .find(|capability| matches_worker_jobs_capability_identity(capability));

    if let Some(capability) = capability {
        validate_asset_worker_jobs_capability(capability)?;
        Ok(capability.clone())
    } else {
        Err(AssetWorkerError::MissingJobsCapabilityGrant)
    }
}

fn worker_now_unix_ms() -> Result<i64, std::time::SystemTimeError> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    Ok(i64::try_from(millis).unwrap_or(i64::MAX))
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
/// In-process worker execution retained exclusively for engine-local tests.
///
/// Production generated workers enter through [`AssetWorkerService`], which
/// owns the finalized registry lease rather than accepting a static registry.
#[cfg(any(test, feature = "test-support"))]
///
/// # Errors
///
/// Returns any error [`run_asset_worker_once_with_registries`] returns for this
/// request.
#[instrument(level = "trace", skip(client, builders, registries, request), fields(lease_owner = %request.lease_owner))]
pub async fn run_asset_worker_once(
    client: &asset_capnp::asset_processor::Client,
    builders: &BuildRuleRegistry,
    registries: &'static Registries,
    request: AssetWorkerRunOnce,
) -> Result<AssetWorkerRunOutcome, AssetWorkerError> {
    run_asset_worker_once_with_registries(
        client,
        builders,
        WorkerRegistries::Static(registries),
        request,
    )
    .await
}

/// Reads and verifies the attempt's DB-authored source payload, failing the attempt
/// on the processor before returning any error.
// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
async fn read_attempt_source_bytes(
    client: &asset_capnp::asset_processor::Client,
    request: &AssetWorkerRunOnce,
    attempt: &az_proto_asset::LeasedAssetJob,
    grant_key: uuid::Uuid,
) -> Result<Vec<u8>, AssetWorkerError> {
    if let Some(handle) = &attempt.source_payload {
        if let Err(source) = validate_side_channel_capability_matches(
            handle,
            &request.capability,
            "asset job source payload",
        ) {
            complete_failed(client, request, attempt.attempt_id, grant_key).await?;
            return Err(AssetWorkerError::SourcePayloadCapability {
                asset_job_attempt_id: attempt.attempt_id,
                source,
            });
        }
        match read_verified_staging_file(handle) {
            Ok(source) => Ok(source.bytes),
            Err(source) => {
                complete_failed(client, request, attempt.attempt_id, grant_key).await?;
                Err(AssetWorkerError::ReadSourcePayload {
                    asset_job_attempt_id: attempt.attempt_id,
                    source,
                })
            }
        }
    } else {
        complete_failed(client, request, attempt.attempt_id, grant_key).await?;
        Err(AssetWorkerError::MissingSourcePayload {
            asset_job_attempt_id: attempt.attempt_id,
        })
    }
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
/// Reports a prepared attempt back to the processor and maps it to the run outcome.
async fn report_prepared_outcome(
    client: &asset_capnp::asset_processor::Client,
    request: &AssetWorkerRunOnce,
    asset_job_attempt_id: i64,
    grant_key: uuid::Uuid,
    prepared: PreparedAssetWorkerJob,
) -> Result<AssetWorkerRunOutcome, AssetWorkerError> {
    match prepared {
        PreparedAssetWorkerJob::Succeeded {
            manifest_handle,
            product_count,
        } => {
            complete_succeeded(
                client,
                request,
                asset_job_attempt_id,
                grant_key,
                *manifest_handle,
            )
            .await?;
            Ok(AssetWorkerRunOutcome::Completed {
                asset_job_attempt_id,
                product_count,
            })
        }
        PreparedAssetWorkerJob::Failed => {
            complete_failed(client, request, asset_job_attempt_id, grant_key).await?;
            Ok(AssetWorkerRunOutcome::Failed {
                asset_job_attempt_id,
            })
        }
        PreparedAssetWorkerJob::Cancelled => {
            complete_cancelled(client, request, asset_job_attempt_id, grant_key).await
        }
    }
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
/// Leases one attempt from the processor and checks that the granted staging root
/// is the one this worker asked for. `None` means the run was cancelled first.
async fn lease_one_attempt(
    client: &asset_capnp::asset_processor::Client,
    request: &AssetWorkerRunOnce,
) -> Result<Option<(az_proto_asset::LeasedAssetJob, uuid::Uuid, PathBuf)>, AssetWorkerError> {
    let lease_staging_root = lease_staging_root(&request.staging_root);
    let mut lease_request = client.lease_request();
    (LeaseAssetJobRequest {
        capability: request.capability.clone(),
        lease_owner: request.lease_owner.clone(),
        lease_duration_ms: u64::try_from(request.lease_duration.as_millis()).unwrap_or(u64::MAX),
        staging_root: Some(lease_staging_root.to_string_lossy().into_owned()),
    })
    .to_capnp(lease_request.get().init_request())?;
    let mut lease = Box::pin(lease_request.send().promise);
    let lease_response = tokio::select! {
        response = &mut lease => response?,
        () = request.cancellation.cancelled() => {
            return Ok(None);
        }
    };
    let lease_result =
        az_proto_asset::LeaseAssetJobResult::from_capnp(lease_response.get()?.get_result()?)?;
    let attempt = lease_result.leased;
    let grant_key = lease_result.grant_key;
    let attempt_staging_root = attempt_staging_root(&attempt)?;
    if attempt_staging_root != lease_staging_root {
        return Err(AssetWorkerError::UnexpectedAttemptStagingRoot {
            asset_job_attempt_id: attempt.attempt_id,
            expected: lease_staging_root,
            actual: attempt_staging_root,
        });
    }
    Ok(Some((attempt, grant_key, attempt_staging_root)))
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
#[instrument(level = "trace", skip(client, builders, registries, request), fields(lease_owner = %request.lease_owner))]
async fn run_asset_worker_once_with_registries(
    client: &asset_capnp::asset_processor::Client,
    builders: &BuildRuleRegistry,
    registries: WorkerRegistries,
    request: AssetWorkerRunOnce,
) -> Result<AssetWorkerRunOutcome, AssetWorkerError> {
    if request.cancellation.is_cancelled() {
        return Ok(AssetWorkerRunOutcome::Cancelled {
            asset_job_attempt_id: None,
        });
    }

    let Some((attempt, grant_key, attempt_staging_root)) =
        lease_one_attempt(client, &request).await?
    else {
        return Ok(AssetWorkerRunOutcome::Cancelled {
            asset_job_attempt_id: None,
        });
    };
    let _staging_cleanup = AttemptStagingCleanup::new(attempt_staging_root);

    if request.cancellation.is_cancelled() {
        return complete_cancelled(client, &request, attempt.attempt_id, grant_key).await;
    }

    if validate_asset_relative_path(&attempt.source_path).is_none() {
        complete_failed(client, &request, attempt.attempt_id, grant_key).await?;
        return Err(AssetWorkerError::UnsafeSourcePath {
            asset_job_attempt_id: attempt.attempt_id,
            source_path: attempt.source_path,
        });
    }
    let source_bytes = read_attempt_source_bytes(client, &request, &attempt, grant_key).await?;

    let builder_guid = match attempt.owner.clone() {
        JobOwner::Plan => {
            return complete_planner_job(
                client,
                builders,
                registries.clone(),
                &request,
                attempt,
                source_bytes,
                grant_key,
            )
            .await;
        }
        JobOwner::Build(builder_guid) => builder_guid,
    };

    let Some(builder) = builders.by_id(BuilderId::new(builder_guid)) else {
        complete_failed(client, &request, attempt.attempt_id, grant_key).await?;
        return Err(AssetWorkerError::UnknownBuilder {
            asset_job_attempt_id: attempt.attempt_id,
            builder_guid,
        });
    };
    let asset_job_attempt_id = attempt.attempt_id;
    let generated_unix_ms = worker_now_unix_ms()?;
    let prepared = run_asset_worker_blocking_task_with_lease(
        client,
        &request,
        asset_job_attempt_id,
        grant_key,
        {
            let builder = builder.clone();
            let cache_root = request.cache_root.clone();
            let cancellation = request.cancellation.clone();
            let capability = request.capability.clone();
            move || {
                prepare_process_job_completion(
                    &builder,
                    &attempt,
                    &source_bytes,
                    &cache_root,
                    &cancellation,
                    &capability,
                    generated_unix_ms,
                    registries.as_ref(),
                )
            }
        },
    )
    .await;
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error @ AssetWorkerError::LeaseRenewalRejected { .. }) => return Err(error),
        Err(error) => {
            complete_failed(client, &request, asset_job_attempt_id, grant_key).await?;
            return Err(error);
        }
    };

    report_prepared_outcome(client, &request, asset_job_attempt_id, grant_key, prepared).await
}

#[expect(
    clippy::too_many_arguments,
    reason = "each argument is threaded straight into the `ProcessJobRequest` this builds, so any grouping here would just re-bag that request one call earlier"
)]
fn prepare_process_job_completion(
    builder: &BuildRule,
    attempt: &LeasedAssetJob,
    source_bytes: &[u8],
    cache_root: &Path,
    cancellation: &CancellationToken,
    capability: &Capability,
    generated_unix_ms: i64,
    registries: &Registries,
) -> Result<PreparedAssetWorkerJob, AssetWorkerError> {
    let response = (builder.process_job)(&ProcessJobRequest {
        builder_id: builder.id,
        source_path: SourcePath::new(&attempt.source_path),
        source_root: Path::new(&attempt.source_root),
        source_uuid: attempt.source_guid,
        preserved_source_asset_id: attempt
            .preserved_source_sub_id
            .map(|sub_id| AssetId::new(attempt.source_guid, sub_id)),
        source_schema_type: attempt.source_schema_type.as_deref(),
        source_bytes,
        platform: &attempt.platform,
        job_key: &attempt.job_key,
        cache_root,
        cancellation,
        context: &JobContext::new(registries),
    });
    if cancellation.is_cancelled() {
        return Ok(PreparedAssetWorkerJob::Cancelled);
    }

    match response.result {
        ProcessJobResult::Success | ProcessJobResult::Skipped => {
            if let Some(product) = response
                .products
                .iter()
                .find(|product| !builder.product_formats.accepts(product.format))
            {
                return Err(AssetWorkerError::UndeclaredProductFormat {
                    asset_job_attempt_id: attempt.attempt_id,
                    builder: builder.name,
                    product_path: product.product_path.as_str().to_owned(),
                    product_format: product.format.as_str().to_owned(),
                });
            }
            let staging_root = attempt_staging_root(attempt)?;
            let manifest = stage_products(
                attempt.attempt_id,
                generated_unix_ms,
                &staging_root,
                response.products,
                response.product_dependencies,
                registries,
            )?;
            if cancellation.is_cancelled() {
                return Ok(PreparedAssetWorkerJob::Cancelled);
            }
            let product_count = manifest.products.len();
            let manifest_handle = write_product_manifest_side_channel(
                capability,
                attempt.attempt_id,
                &staging_root,
                &manifest,
            )?;
            if cancellation.is_cancelled() {
                return Ok(PreparedAssetWorkerJob::Cancelled);
            }
            Ok(PreparedAssetWorkerJob::Succeeded {
                manifest_handle: Box::new(manifest_handle),
                product_count,
            })
        }
        ProcessJobResult::Failed => Ok(PreparedAssetWorkerJob::Failed),
        ProcessJobResult::Cancelled => Ok(PreparedAssetWorkerJob::Cancelled),
    }
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
async fn complete_planner_job(
    client: &asset_capnp::asset_processor::Client,
    builders: &BuildRuleRegistry,
    registries: WorkerRegistries,
    request: &AssetWorkerRunOnce,
    attempt: LeasedAssetJob,
    source_bytes: Vec<u8>,
    grant_key: uuid::Uuid,
) -> Result<AssetWorkerRunOutcome, AssetWorkerError> {
    let asset_job_attempt_id = attempt.attempt_id;
    let matching = builders
        .matching_source(&attempt.source_path, attempt.source_schema_type.as_deref())
        .cloned()
        .collect::<Vec<_>>();
    if matching.is_empty() {
        let path_matching = builders
            .iter()
            .filter(|builder| builder.matches(&attempt.source_path))
            .map(|builder| builder.name)
            .collect::<Vec<_>>();
        let schema_matching = builders
            .iter()
            .filter(|builder| builder.matches_source_schema(attempt.source_schema_type.as_deref()))
            .map(|builder| builder.name)
            .collect::<Vec<_>>();
        debug!(
            source_path = %attempt.source_path,
            source_schema_type = ?attempt.source_schema_type,
            ?path_matching,
            ?schema_matching,
            "asset planner found no matching builder",
        );
    }
    let capability = request.capability.clone();
    let cancellation = request.cancellation.clone();
    let generated_unix_ms = worker_now_unix_ms()?;
    let prepared = run_asset_worker_blocking_task_with_lease(
        client,
        request,
        asset_job_attempt_id,
        grant_key,
        move || {
            prepare_planner_job_completion(
                &matching,
                &attempt,
                &source_bytes,
                &capability,
                &cancellation,
                generated_unix_ms,
                registries.as_ref(),
            )
        },
    )
    .await;
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error @ AssetWorkerError::LeaseRenewalRejected { .. }) => return Err(error),
        Err(error) => {
            complete_failed(client, request, asset_job_attempt_id, grant_key).await?;
            return Err(error);
        }
    };

    match prepared {
        PreparedAssetWorkerJob::Succeeded {
            manifest_handle,
            product_count,
        } => {
            complete_succeeded(
                client,
                request,
                asset_job_attempt_id,
                grant_key,
                *manifest_handle,
            )
            .await?;
            Ok(AssetWorkerRunOutcome::Completed {
                asset_job_attempt_id,
                product_count,
            })
        }
        PreparedAssetWorkerJob::Failed => {
            complete_failed(client, request, asset_job_attempt_id, grant_key).await?;
            Ok(AssetWorkerRunOutcome::Failed {
                asset_job_attempt_id,
            })
        }
        PreparedAssetWorkerJob::Cancelled => {
            complete_cancelled(client, request, asset_job_attempt_id, grant_key).await
        }
    }
}

fn prepare_planner_job_completion(
    matching: &[BuildRule],
    attempt: &LeasedAssetJob,
    source_bytes: &[u8],
    capability: &Capability,
    cancellation: &CancellationToken,
    generated_unix_ms: i64,
    registries: &Registries,
) -> Result<PreparedAssetWorkerJob, AssetWorkerError> {
    let platforms = [DEFAULT_PLATFORM_ID];
    let context = JobContext::new(registries);
    let source_schema_type = attempt.source_schema_type.as_deref();
    let mut plan = WorkerCreateJobsPlan {
        builders: Vec::new(),
    };
    for builder in matching {
        if cancellation.is_cancelled() {
            return Ok(PreparedAssetWorkerJob::Cancelled);
        }
        let jobs = (builder.create_jobs)(&CreateJobsRequest {
            builder_id: builder.id,
            source_path: SourcePath::new(&attempt.source_path),
            source_root: Path::new(&attempt.source_root),
            source_uuid: attempt.source_guid,
            source_schema_type,
            source_bytes,
            platforms: &platforms,
            context: &context,
        });
        match jobs.result {
            CreateJobsResult::Success => {}
            CreateJobsResult::Skipped => continue,
            CreateJobsResult::Failed => {
                return Err(AssetWorkerError::PlannerCreateJobsFailed {
                    asset_job_attempt_id: attempt.attempt_id,
                    reason: format!("builder `{}` create_jobs failed", builder.name),
                });
            }
        }
        let mut source_dependencies = Vec::new();
        for dependency in &jobs.source_dependencies {
            match dependency {
                SourceFileDependency::Path(path) => {
                    if let Some(path) = validate_asset_relative_path(path) {
                        source_dependencies.push(path);
                    }
                }
                SourceFileDependency::Uuid(uuid) => {
                    source_dependencies.push(format!("uuid:{uuid}"));
                }
            }
        }
        plan.builders.push(WorkerCreateJobsBuilderPlan {
            builder_guid: builder.id.0,
            jobs: jobs
                .jobs
                .iter()
                .map(|job| WorkerCreateJobsJobPlan {
                    job_key: job.job_key.as_str().to_string(),
                    platform: job.platform.as_str().to_string(),
                })
                .collect(),
            source_dependencies,
        });
    }

    let plan_bytes =
        serde_json::to_vec(&plan).map_err(|source| AssetWorkerError::PlannerCreateJobsFailed {
            asset_job_attempt_id: attempt.attempt_id,
            reason: format!("failed to encode create_jobs plan JSON: {source}"),
        })?;
    let plan_product = az_asset_builder::BuildProduct {
        product_path: az_asset_builder::ProductPath::from_trusted(ASSET_JOB_PLAN_PRODUCT_PATH),
        catalog_aliases: Vec::new(),
        catalog_path_registration: AssetCatalogPathRegistration::Registered,
        asset_type: az_core::AssetType::new(ASSET_JOB_PLAN_ASSET_TYPE),
        format: ProductFormatId::__from_static(ASSET_JOB_PLAN_PRODUCT_FORMAT),
        format_version: 1,
        sub_id: 0,
        bytes: plan_bytes,
    };
    let staging_root = attempt_staging_root(attempt)?;
    let manifest = stage_products(
        attempt.attempt_id,
        generated_unix_ms,
        &staging_root,
        vec![plan_product],
        Vec::new(),
        registries,
    )?;
    if cancellation.is_cancelled() {
        return Ok(PreparedAssetWorkerJob::Cancelled);
    }
    let product_count = manifest.products.len();
    let manifest_handle = write_product_manifest_side_channel(
        capability,
        attempt.attempt_id,
        &staging_root,
        &manifest,
    )?;
    if cancellation.is_cancelled() {
        return Ok(PreparedAssetWorkerJob::Cancelled);
    }
    Ok(PreparedAssetWorkerJob::Succeeded {
        manifest_handle: Box::new(manifest_handle),
        product_count,
    })
}

fn lease_staging_root(staging_base: &Path) -> PathBuf {
    staging_base.join(format!("lease-{}", uuid::Uuid::now_v7()))
}

fn remove_ephemeral_staging_tree(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(source),
    }
}

fn prepare_asset_worker_staging_root(staging_root: &Path) -> Result<(), AssetWorkerError> {
    let parent = staging_root
        .parent()
        .ok_or_else(|| AssetWorkerError::InvalidWorkerRoot {
            flag: "--staging-root",
            path: staging_root.display().to_string(),
        })?;
    fs::create_dir_all(parent).map_err(|source| AssetWorkerError::PrepareStagingRoot {
        path: parent.to_path_buf(),
        source,
    })?;
    remove_ephemeral_staging_tree(staging_root).map_err(|source| {
        AssetWorkerError::PrepareStagingRoot {
            path: staging_root.to_path_buf(),
            source,
        }
    })?;
    fs::create_dir_all(staging_root).map_err(|source| AssetWorkerError::PrepareStagingRoot {
        path: staging_root.to_path_buf(),
        source,
    })
}

fn attempt_staging_root(attempt: &LeasedAssetJob) -> Result<PathBuf, AssetWorkerError> {
    if attempt.staging_root.is_empty() {
        return Err(AssetWorkerError::MissingAttemptStagingRoot {
            asset_job_attempt_id: attempt.attempt_id,
        });
    }
    Ok(PathBuf::from(&attempt.staging_root))
}

/// How a physical product path is backed, so two products that share a path can be
/// checked for agreement on the bytes behind it.
#[derive(Debug)]
struct PhysicalProductBacking {
    asset_type: uuid::Uuid,
    product_format: String,
    product_format_version: u32,
    content_hash: [u8; 32],
    byte_length: usize,
    dependencies: Vec<ProductManifestProductDependency>,
}

/// Groups emitted product dependencies by product index, rejecting an index that
/// names no emitted product, a nil dependency guid, or a repeated dependency.
fn group_product_dependencies(
    asset_job_attempt_id: i64,
    product_count: usize,
    product_dependencies: Vec<(usize, ProductDependency)>,
) -> Result<Vec<Vec<ProductManifestProductDependency>>, AssetWorkerError> {
    let mut dependencies_by_product = (0..product_count)
        .map(|_| Vec::new())
        .collect::<Vec<Vec<ProductManifestProductDependency>>>();
    for (product_index, dependency) in product_dependencies {
        let Some(dependencies) = dependencies_by_product.get_mut(product_index) else {
            return Err(AssetWorkerError::InvalidProductDependency {
                asset_job_attempt_id,
                reason: format!(
                    "product index {product_index} is outside {product_count} emitted products"
                ),
            });
        };
        if dependency.dependency_id.guid.is_nil() {
            return Err(AssetWorkerError::InvalidProductDependency {
                asset_job_attempt_id,
                reason: format!("product index {product_index} has a nil dependency asset guid"),
            });
        }
        if dependencies.iter().any(|existing| {
            existing.asset_guid == dependency.dependency_id.guid
                && existing.sub_id == dependency.dependency_id.sub_id
        }) {
            return Err(AssetWorkerError::InvalidProductDependency {
                asset_job_attempt_id,
                reason: format!(
                    "product index {product_index} repeats dependency {}:{}",
                    dependency.dependency_id.guid, dependency.dependency_id.sub_id
                ),
            });
        }
        dependencies.push(ProductManifestProductDependency {
            asset_guid: dependency.dependency_id.guid,
            sub_id: dependency.dependency_id.sub_id,
            flags: dependency.flags.0,
        });
    }
    Ok(dependencies_by_product)
}

/// Writes one staged product to disk under the attempt staging root, creating the
/// parent directory first.
fn write_staged_product(
    asset_job_attempt_id: i64,
    staging_root: &Path,
    product_path: &str,
    bytes: &[u8],
) -> Result<(), AssetWorkerError> {
    let staged_path = staging_root.join(product_path);
    if let Some(parent) = staged_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| {
            AssetWorkerError::CreateProductParent {
                asset_job_attempt_id,
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }
    std::fs::write(&staged_path, bytes).map_err(|source| AssetWorkerError::WriteProduct {
        asset_job_attempt_id,
        path: staged_path.clone(),
        source,
    })?;
    Ok(())
}

/// Stages one attempt's emitted products to disk and turns them into manifest
/// entries, enforcing catalog-path uniqueness and shared-path backing agreement.
#[allow(clippy::too_many_arguments)]
fn stage_emitted_products(
    asset_job_attempt_id: i64,
    staging_root: &Path,
    products: Vec<az_asset_builder::BuildProduct>,
    registries: &Registries,
    dependencies_by_product: &mut [Vec<ProductManifestProductDependency>],
    manifest_products: &mut Vec<ProductManifestProduct>,
    catalog_paths: &mut BTreeSet<String>,
    physical_products: &mut BTreeMap<String, PhysicalProductBacking>,
) -> Result<(), AssetWorkerError> {
    for (product_index, product) in products.into_iter().enumerate() {
        let product_path = product.product_path.into_string();
        let catalog_aliases = product
            .catalog_aliases
            .into_iter()
            .map(az_asset_builder::ProductPath::into_string)
            .collect::<Vec<_>>();
        let catalog_path_registration = match product.catalog_path_registration {
            AssetCatalogPathRegistration::Registered => CatalogPathRegistration::Registered,
            AssetCatalogPathRegistration::AssetIdOnly => CatalogPathRegistration::AssetIdOnly,
        };
        if catalog_path_registration == CatalogPathRegistration::AssetIdOnly
            && !catalog_aliases.is_empty()
        {
            return Err(AssetWorkerError::InvalidSharedProductPath {
                asset_job_attempt_id,
                product_path,
                reason: "asset-id-only product cannot declare catalog aliases".to_string(),
            });
        }
        if catalog_path_registration == CatalogPathRegistration::Registered
            && (!catalog_paths.insert(product_path.clone())
                || catalog_aliases
                    .iter()
                    .any(|alias| !catalog_paths.insert(alias.clone())))
        {
            return Err(AssetWorkerError::InvalidSharedProductPath {
                asset_job_attempt_id,
                product_path,
                reason: "more than one product claims the same catalog lookup path".to_string(),
            });
        }
        if validate_asset_relative_path(&product_path).is_none() {
            return Err(AssetWorkerError::UnsafeProductPath {
                asset_job_attempt_id,
                product_path,
            });
        }
        validate_build_product_format(
            asset_job_attempt_id,
            &product_path,
            product.format,
            product.format_version,
            registries,
        )?;

        let dependencies = std::mem::take(&mut dependencies_by_product[product_index]);
        let content_hash = *blake3::hash(&product.bytes).as_bytes();
        let backing = PhysicalProductBacking {
            asset_type: product.asset_type.into_inner(),
            product_format: product.format.as_str().to_string(),
            product_format_version: product.format_version,
            content_hash,
            byte_length: product.bytes.len(),
            dependencies: dependencies.clone(),
        };
        if let Some(first) = physical_products.get(&product_path) {
            if first.asset_type != backing.asset_type
                || first.product_format != backing.product_format
                || first.product_format_version != backing.product_format_version
                || first.content_hash != backing.content_hash
                || first.byte_length != backing.byte_length
                || first.dependencies != backing.dependencies
            {
                return Err(AssetWorkerError::InvalidSharedProductPath {
                    asset_job_attempt_id,
                    product_path,
                    reason: "logical products disagree on bytes, type, format, or dependencies"
                        .to_string(),
                });
            }
        } else {
            write_staged_product(
                asset_job_attempt_id,
                staging_root,
                &product_path,
                &product.bytes,
            )?;
            physical_products.insert(product_path.clone(), backing);
        }
        manifest_products.push(ProductManifestProduct {
            product_path: product_path.clone(),
            catalog_aliases,
            catalog_path_registration,
            asset_type: product.asset_type.into_inner(),
            sub_id: product.sub_id,
            product_format: product.format.as_str().to_string(),
            product_format_version: product.format_version,
            staged_path: product_path,
            content_hash: content_hash.to_vec(),
            byte_length: product.bytes.len() as u64,
            dependencies,
        });
    }
    Ok(())
}

fn stage_products(
    asset_job_attempt_id: i64,
    generated_unix_ms: i64,
    staging_root: &Path,
    products: Vec<az_asset_builder::BuildProduct>,
    product_dependencies: Vec<(usize, ProductDependency)>,
    registries: &Registries,
) -> Result<ProductManifest, AssetWorkerError> {
    let mut dependencies_by_product =
        group_product_dependencies(asset_job_attempt_id, products.len(), product_dependencies)?;

    let mut manifest_products = Vec::with_capacity(products.len());
    let mut catalog_paths = BTreeSet::new();
    let mut physical_products = BTreeMap::<String, PhysicalProductBacking>::new();
    stage_emitted_products(
        asset_job_attempt_id,
        staging_root,
        products,
        registries,
        &mut dependencies_by_product,
        &mut manifest_products,
        &mut catalog_paths,
        &mut physical_products,
    )?;

    Ok(ProductManifest::new(
        generated_unix_ms.try_into().unwrap_or_default(),
        manifest_products,
    ))
}

fn validate_build_product_format(
    asset_job_attempt_id: i64,
    product_path: &str,
    format: ProductFormatId,
    format_version: u32,
    registries: &Registries,
) -> Result<(), AssetWorkerError> {
    if format.as_str() == ASSET_JOB_PLAN_PRODUCT_FORMAT {
        if format_version != 1 {
            return Err(AssetWorkerError::UnsupportedProductFormatVersion {
                asset_job_attempt_id,
                product_path: product_path.to_string(),
                product_format: format.as_str().to_string(),
                version: format_version,
                current_version: 1,
            });
        }
        return Ok(());
    }
    let Some(attributed) = composed_product_format(registries, format) else {
        return Err(AssetWorkerError::UnregisteredProductFormat {
            asset_job_attempt_id,
            product_path: product_path.to_string(),
            product_format: format.as_str().to_string(),
        });
    };
    if format_version == 0 || format_version > attributed.entry.current_version() {
        return Err(AssetWorkerError::UnsupportedProductFormatVersion {
            asset_job_attempt_id,
            product_path: product_path.to_string(),
            product_format: format.as_str().to_string(),
            version: format_version,
            current_version: attributed.entry.current_version(),
        });
    }
    Ok(())
}

fn write_product_manifest_side_channel(
    capability: &Capability,
    asset_job_attempt_id: i64,
    staging_root: &Path,
    manifest: &ProductManifest,
) -> Result<SideChannelHandle, AssetWorkerError> {
    let bytes = encode_product_manifest(manifest)?;
    std::fs::create_dir_all(staging_root).map_err(|source| {
        AssetWorkerError::CreateProductParent {
            asset_job_attempt_id,
            path: staging_root.to_path_buf(),
            source,
        }
    })?;
    let path = staging_root.join(format!(
        "attempt-{asset_job_attempt_id}-products.capnp.packed"
    ));
    let written = write_named_staging_file_atomic(&path, &bytes).map_err(|error| {
        AssetWorkerError::WriteManifest {
            asset_job_attempt_id,
            path: error.path,
            source: error.source,
        }
    })?;
    Ok(SideChannelHandle::staging_file(
        written.path.to_string_lossy(),
        written.byte_length,
        written.content_hash,
        std::env::consts::OS,
    )
    .with_capability(capability.clone()))
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
async fn complete_succeeded(
    client: &asset_capnp::asset_processor::Client,
    request: &AssetWorkerRunOnce,
    asset_job_attempt_id: i64,
    grant_key: uuid::Uuid,
    product_manifest: SideChannelHandle,
) -> Result<(), AssetWorkerError> {
    complete(
        client,
        CompleteAssetJobAttemptRequest {
            capability: request.capability.clone(),
            asset_job_attempt_id,
            lease_owner: request.lease_owner.clone(),
            grant_key,
            status: AttemptStatus::Succeeded,
            finished_unix_ms: worker_now_unix_ms()?,
            error_count: 0,
            warning_count: 0,
            product_manifest: Some(product_manifest),
        },
    )
    .await
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
async fn complete_failed(
    client: &asset_capnp::asset_processor::Client,
    request: &AssetWorkerRunOnce,
    asset_job_attempt_id: i64,
    grant_key: uuid::Uuid,
) -> Result<(), AssetWorkerError> {
    complete_without_products(
        client,
        request,
        asset_job_attempt_id,
        grant_key,
        AttemptStatus::Failed,
        1,
    )
    .await
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
async fn complete_without_products(
    client: &asset_capnp::asset_processor::Client,
    request: &AssetWorkerRunOnce,
    asset_job_attempt_id: i64,
    grant_key: uuid::Uuid,
    status: AttemptStatus,
    error_count: i64,
) -> Result<(), AssetWorkerError> {
    complete(
        client,
        CompleteAssetJobAttemptRequest {
            capability: request.capability.clone(),
            asset_job_attempt_id,
            lease_owner: request.lease_owner.clone(),
            grant_key,
            status,
            finished_unix_ms: worker_now_unix_ms()?,
            error_count,
            warning_count: 0,
            product_manifest: None,
        },
    )
    .await
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
async fn complete_cancelled(
    client: &asset_capnp::asset_processor::Client,
    request: &AssetWorkerRunOnce,
    asset_job_attempt_id: i64,
    grant_key: uuid::Uuid,
) -> Result<AssetWorkerRunOutcome, AssetWorkerError> {
    complete_without_products(
        client,
        request,
        asset_job_attempt_id,
        grant_key,
        AttemptStatus::Abandoned,
        0,
    )
    .await?;
    Ok(AssetWorkerRunOutcome::Cancelled {
        asset_job_attempt_id: Some(asset_job_attempt_id),
    })
}

// The worker loop holds Cap'n Proto RPC state, which is `Cell`-based and therefore
// neither `Sync` nor `Send`. This future can only be `Send` if the RPC layer is
// replaced, so it is single-threaded by construction.
#[allow(clippy::future_not_send)]
async fn complete(
    client: &asset_capnp::asset_processor::Client,
    request: CompleteAssetJobAttemptRequest,
) -> Result<(), AssetWorkerError> {
    let asset_job_attempt_id = request.asset_job_attempt_id;
    let mut complete_request = client.complete_attempt_request();
    (request).to_capnp(complete_request.get().init_request())?;
    let complete_response = complete_request.send().promise.await?;
    if complete_response.get()?.get_completed() {
        Ok(())
    } else {
        Err(AssetWorkerError::CompletionRejected {
            asset_job_attempt_id,
        })
    }
}

fn asset_builder_to_proto(builder: &BuildRule) -> AssetBuilderDescriptor {
    AssetBuilderDescriptor {
        name: builder.name.to_string(),
        builder_guid: builder.id.0,
        version: builder.version,
        analysis_fingerprint: build_rule_analysis_fingerprint(builder),
        patterns: builder
            .primary_source
            .patterns()
            .iter()
            .map(asset_builder_pattern_to_proto)
            .collect(),
        source_schema_types: builder
            .primary_source
            .schema_types()
            .iter()
            .map(|schema_type| (*schema_type).to_string())
            .collect(),
    }
}

fn build_rule_analysis_fingerprint(builder: &BuildRule) -> String {
    let mut formats = builder.product_formats.declared().map(|formats| {
        formats
            .iter()
            .map(|format| format.as_str())
            .collect::<Vec<_>>()
    });
    if let Some(formats) = formats.as_mut() {
        formats.sort_unstable();
    }
    format!(
        "{}\nproduct-formats={}",
        builder.analysis_fingerprint,
        formats.map_or_else(|| "dynamic".to_owned(), |formats| formats.join(","))
    )
}

fn source_schema_to_proto(
    attributed: &Attributed<SourceSchemaRegistration>,
    source_file_templates: &[Attributed<SourceFileTemplateRegistration>],
) -> Result<SourceSchemaDescriptor, AssetWorkerError> {
    let registration = attributed.entry;
    let schema_type = registration.schema_type();
    let authoring = registration.authoring();
    let file_templates = match authoring {
        BuilderSourceSchemaAuthoring::File { workflow } if workflow.can_create() => {
            source_file_templates_to_proto(schema_type, workflow, source_file_templates)?
        }
        BuilderSourceSchemaAuthoring::File { .. }
        | BuilderSourceSchemaAuthoring::ProjectDocument { .. } => Vec::new(),
    };
    Ok(SourceSchemaDescriptor {
        schema_type: schema_type.as_str().to_string(),
        owner: attributed.instance.gem.as_str().to_string(),
        label: registration.label().to_string(),
        category: registration.category().to_string(),
        authoring: source_schema_authoring_to_proto(authoring),
        file_templates,
    })
}

fn composed_product_formats_to_proto(registries: &Registries) -> Vec<ProductFormatDescriptor> {
    composed_product_formats(registries)
        .into_iter()
        .map(|attributed| ProductFormatDescriptor {
            id: attributed.entry.id().as_str().to_string(),
            current_version: attributed.entry.current_version(),
            owner: attributed.instance.gem.as_str().to_string(),
        })
        .collect()
}

fn graph_source_schemas_to_proto(
    registries: &Registries,
) -> Result<Vec<SourceSchemaDescriptor>, AssetWorkerError> {
    let descriptors = graph_source_schemas(registries)
        .map_err(|source| AssetWorkerError::BuilderCatalogInvalid {
            reason: format!("registered graph source schemas are invalid: {source}"),
        })?
        .into_iter()
        .map(graph_source_schema_to_proto)
        .collect::<Vec<_>>();
    Ok(descriptors)
}

fn graph_source_schema_to_proto(schema: RegisteredGraphSourceSchema) -> SourceSchemaDescriptor {
    let authoring = match schema.authoring {
        RegisteredGraphSourceAuthoring::ProjectDocument { schema_type } => {
            SourceSchemaAuthoring::ProjectDocument { schema_type }
        }
        RegisteredGraphSourceAuthoring::File {
            source_root,
            default_path_prefix,
            extensions,
            can_create,
            can_edit,
        } => SourceSchemaAuthoring::File {
            workflow: SourceFileWorkflowDescriptor {
                source_root,
                default_path_prefix,
                extensions,
                can_create,
                can_edit,
            },
        },
    };
    SourceSchemaDescriptor {
        schema_type: schema.schema_type,
        owner: schema.owner,
        label: schema.label,
        category: schema.category,
        authoring,
        file_templates: Vec::new(),
    }
}

fn source_file_templates_to_proto(
    schema_type: SourceSchemaType,
    workflow: az_asset_builder::SourceFileWorkflow,
    source_file_templates: &[Attributed<SourceFileTemplateRegistration>],
) -> Result<Vec<SourceFileTemplateDescriptor>, AssetWorkerError> {
    let mut seen_paths = BTreeMap::<String, String>::new();
    let mut templates = Vec::new();

    for attributed in source_file_templates
        .iter()
        .filter(|attributed| attributed.entry.schema_type() == schema_type)
    {
        let registration = attributed.entry;
        let owner = attributed.instance.gem.as_str();
        for candidate in registration.candidates() {
            let source_path = validate_asset_relative_path(&candidate.source_path).ok_or_else(
                || AssetWorkerError::BuilderCatalogInvalid {
                    reason: format!(
                        "source schema `{}` template from `{}` has non-canonical source path `{}`",
                        schema_type, owner, candidate.source_path
                    ),
                },
            )?;
            if !source_path_matches_extensions(&source_path, workflow.extensions()) {
                return Err(AssetWorkerError::BuilderCatalogInvalid {
                    reason: format!(
                        "source schema `{}` template from `{}` uses source path `{}` outside registered extensions {:?}",
                        schema_type,
                        owner,
                        source_path,
                        workflow.extensions()
                    ),
                });
            }
            if let Some(first_owner) = seen_paths.insert(source_path.clone(), owner.to_string()) {
                return Err(AssetWorkerError::BuilderCatalogInvalid {
                    reason: format!(
                        "source schema `{schema_type}` has duplicate source file template path `{source_path}` from `{first_owner}` and `{owner}`"
                    ),
                });
            }
            if candidate.label.trim() != candidate.label
                || candidate.description.trim() != candidate.description
            {
                return Err(AssetWorkerError::BuilderCatalogInvalid {
                    reason: format!(
                        "source schema `{schema_type}` template `{source_path}` from `{owner}` has untrimmed UI text"
                    ),
                });
            }
            templates.push(SourceFileTemplateDescriptor {
                owner: owner.to_string(),
                source_path,
                label: candidate.label,
                description: candidate.description,
            });
        }
    }

    templates.sort_by(|left, right| {
        left.source_path
            .cmp(&right.source_path)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.owner.cmp(&right.owner))
    });
    Ok(templates)
}

fn source_schema_authoring_to_proto(
    authoring: BuilderSourceSchemaAuthoring,
) -> SourceSchemaAuthoring {
    match authoring {
        BuilderSourceSchemaAuthoring::ProjectDocument { schema_type } => {
            SourceSchemaAuthoring::ProjectDocument {
                schema_type: schema_type.to_string(),
            }
        }
        BuilderSourceSchemaAuthoring::File { workflow } => SourceSchemaAuthoring::File {
            workflow: SourceFileWorkflowDescriptor {
                source_root: workflow.source_root().to_string(),
                default_path_prefix: workflow.default_path_prefix().to_string(),
                extensions: workflow
                    .extensions()
                    .iter()
                    .map(|extension| (*extension).to_string())
                    .collect(),
                can_create: workflow.can_create(),
                can_edit: workflow.can_edit(),
            },
        },
    }
}

fn ensure_builder_catalog_source_schema_coverage(
    builders: &[AssetBuilderDescriptor],
    source_schemas: &[SourceSchemaDescriptor],
) -> Result<(), AssetWorkerError> {
    let source_schema_types = source_schemas
        .iter()
        .map(|source_schema| source_schema.schema_type.as_str())
        .collect::<BTreeSet<_>>();

    for builder in builders {
        for source_schema_type in &builder.source_schema_types {
            if !source_schema_types.contains(source_schema_type.as_str()) {
                return Err(AssetWorkerError::BuilderCatalogInvalid {
                    reason: format!(
                        "builder `{}` references source schema type `{}` without a source schema registration",
                        builder.name, source_schema_type
                    ),
                });
            }
        }
    }

    Ok(())
}

fn ensure_unique_source_schema_descriptors(
    source_schemas: &[SourceSchemaDescriptor],
) -> Result<(), AssetWorkerError> {
    let mut seen = BTreeMap::<&str, &str>::new();
    for source_schema in source_schemas {
        if let Some(first_owner) = seen.insert(
            source_schema.schema_type.as_str(),
            source_schema.owner.as_str(),
        ) {
            return Err(AssetWorkerError::BuilderCatalogInvalid {
                reason: format!(
                    "source schema `{}` is registered by both `{}` and `{}`",
                    source_schema.schema_type, first_owner, source_schema.owner
                ),
            });
        }
    }
    Ok(())
}

fn ensure_source_file_template_registrations_match_schemas(
    source_schemas: &[Attributed<SourceSchemaRegistration>],
    source_file_templates: &[Attributed<SourceFileTemplateRegistration>],
) -> Result<(), AssetWorkerError> {
    let source_schema_authoring = source_schemas
        .iter()
        .map(|attributed| (attributed.entry.schema_type(), attributed.entry.authoring()))
        .collect::<BTreeMap<_, _>>();

    for attributed in source_file_templates {
        let schema_type = attributed.entry.schema_type();
        let owner = attributed.instance.gem.as_str();
        match source_schema_authoring.get(&schema_type).copied() {
            Some(BuilderSourceSchemaAuthoring::File { workflow }) if workflow.can_create() => {}
            Some(BuilderSourceSchemaAuthoring::File { .. }) => {
                return Err(AssetWorkerError::BuilderCatalogInvalid {
                    reason: format!(
                        "source file template from `{owner}` targets source schema `{schema_type}` but that schema is not default-creatable"
                    ),
                });
            }
            Some(BuilderSourceSchemaAuthoring::ProjectDocument { .. }) => {
                return Err(AssetWorkerError::BuilderCatalogInvalid {
                    reason: format!(
                        "source file template from `{owner}` targets project-document source schema `{schema_type}`"
                    ),
                });
            }
            None => {
                return Err(AssetWorkerError::BuilderCatalogInvalid {
                    reason: format!(
                        "source file template from `{owner}` targets unregistered source schema `{schema_type}`"
                    ),
                });
            }
        }
    }

    Ok(())
}

fn asset_builder_pattern_to_proto(pattern: &AssetBuilderPattern) -> AssetBuilderPatternDescriptor {
    match pattern {
        AssetBuilderPattern::StaticWildcard(pattern) => AssetBuilderPatternDescriptor {
            kind: AssetBuilderPatternKind::Wildcard,
            pattern: (*pattern).to_string(),
        },
        AssetBuilderPattern::Wildcard(pattern) => AssetBuilderPatternDescriptor {
            kind: AssetBuilderPatternKind::Wildcard,
            pattern: pattern.clone(),
        },
        AssetBuilderPattern::Regex(regex) => AssetBuilderPatternDescriptor {
            kind: AssetBuilderPatternKind::Regex,
            pattern: regex.as_str().to_string(),
        },
    }
}

fn validate_asset_relative_path(path: &str) -> Option<String> {
    if path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.contains(':')
        || path.trim() != path
    {
        return None;
    }

    let mut has_component = false;
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return None;
        }
        has_component = true;
    }

    has_component.then(|| path.to_string())
}

fn source_path_matches_extensions(source_path: &str, extensions: &[impl AsRef<str>]) -> bool {
    let file_name = source_path
        .rsplit('/')
        .next()
        .unwrap_or(source_path)
        .to_ascii_lowercase();
    extensions.iter().any(|extension| {
        let extension = extension.as_ref().to_ascii_lowercase();
        extension == "*" || file_name == extension || file_name.ends_with(&format!(".{extension}"))
    })
}

fn is_normal_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, OnceLock,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use az_asset_builder::{
        AssetBuilderPattern, BuildProduct, BuildRule, BuilderId, CreateJobsRequest,
        CreateJobsResponse, DEFAULT_PLATFORM, ProcessJobRequest, ProcessJobResponse,
        ProductDependency, ProductDependencyFlags, ProductFormatId, ProductFormatPolicy,
        SourceMatcher,
    };
    use az_proto_asset::{CatalogPathRegistration, ProductManifestProductDependency};
    use az_proto_core::{
        CapabilityGrantSet, Endpoint, ServiceId, ServiceRole, encode_capability_grant_set,
    };
    use uuid::Uuid;

    use super::*;

    fn worker_capability() -> Capability {
        Capability::new(
            ServiceId::new(ASSET_WORKER_SERVICE_NAMESPACE, ASSET_WORKER_SERVICE_NAME),
            ServiceRole::Worker,
        )
        .with_audience(ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_JOBS_PERMISSION])
        .with_token_hash([0x6a, 0x6f, 0x62, 0x73])
    }

    az_gem_contract::declare_caps!(WorkerCaps:);

    const HARNESS: az_gem_contract::ContributionDescriptor =
        az_gem_contract::ContributionDescriptor {
            gem: az_gem_contract::GemId::new("azoth.asset-worker-tests"),
            contribution: az_gem_contract::ContributionId::new("harness"),
            roles: &[],
        };

    /// The engine's own bundles, role-filtered, ahead of anything else.
    ///
    /// Written out rather than shared with the asset-processor's
    /// `compose_engine`: the list is a *linkage* declaration, so a helper that
    /// named all four would put `az-engine-builders` in the link set of every
    /// host that called it, which is the duality ADR 0036 asks the bundles to
    /// preserve. The mechanism — role filter, default activation, no `enabled`
    /// switch — is shared, and it is `Composer::floor`.
    fn compose_engine(composer: &mut az_gem_contract::Composer) {
        for outcome in [
            composer.floor(az_engine_types::types_contribution()),
            composer.floor(az_engine_assets::assets_contribution()),
            composer.floor(az_engine_runtime::runtime_contribution()),
            composer.floor(az_engine_builders::builders_contribution()),
        ] {
            outcome.expect("the engine floor declares no host-capability floor");
        }
    }

    /// Stands in for the worker host's own composition: the SDK's fixture
    /// product format is what these staging tests emit.
    struct Harness;

    impl az_gem_contract::Contribution for Harness {
        type Caps = WorkerCaps;

        fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
            HARNESS
        }

        fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, WorkerCaps>) {
            az_asset_builder::test_support::register(ctx);
        }
    }

    /// A worker host composes once for the life of its process, so the tests
    /// hold one composition the same way a real host does — engine floor
    /// first, then what the host itself contributes.
    ///
    /// The floor is not decoration here. Until it was composed these tests ran
    /// against an empty classifier set and could not have told the difference
    /// between "the worker builds nothing" and "the worker was handed nothing
    /// to build with"; the engine bundles are what a real worker starts from
    /// (asset-contract ticket 014, D6/D7), and `az_engine_ids::contributions`
    /// is the one enumeration of them.
    fn test_registries() -> &'static Registries {
        static REGISTRIES: OnceLock<&'static Registries> = OnceLock::new();
        REGISTRIES.get_or_init(|| {
            let mut composer =
                az_gem_contract::Composer::new(az_gem_contract::GemTargetRole::AssetWorker);
            compose_engine(&mut composer);
            composer
                .add(Harness, az_gem_contract::ProductActivation::default())
                .expect("an empty floor composes");
            composer
                .finalize()
                .expect("the harness composition is valid");
            // The composer owns the registries and outlives the process, the
            // same shape a worker host has.
            Box::leak(Box::new(composer)).registries()
        })
    }

    #[test]
    fn service_composition_refuses_another_role_before_finalization() {
        let Err(error) =
            AssetWorkerComposition::from_composer(Composer::new(GemTargetRole::RuntimeHost))
        else {
            panic!("asset worker must reject another host role")
        };

        assert!(matches!(
            error,
            AssetWorkerError::WrongCompositionRole {
                actual: GemTargetRole::RuntimeHost
            }
        ));
    }

    #[test]
    fn service_composition_retains_the_finalized_registry_lease() {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Harness, az_gem_contract::ProductActivation::default())
            .expect("worker harness composes");
        let composition =
            AssetWorkerComposition::from_composer(composer).expect("worker composition is ready");

        assert_eq!(composition.process.role(), GemTargetRole::AssetWorker);
        assert!(std::ptr::eq(
            composition.registries.registries(),
            composition.process.registry_lease().unwrap().registries()
        ));
    }

    const TEST_CODEC_SCHEMA: SourceSchemaType =
        SourceSchemaType::__from_static("az.test.asset-worker-source");

    const TEST_CODEC: az_gem_contract::ContributionDescriptor =
        az_gem_contract::ContributionDescriptor {
            gem: az_gem_contract::GemId::new("azoth.asset-worker-tests"),
            contribution: az_gem_contract::ContributionId::new("source-codec"),
            roles: &[az_gem_contract::GemTargetRole::AssetWorker],
        };

    struct SourceCodecHarness;

    impl az_gem_contract::Contribution for SourceCodecHarness {
        type Caps = WorkerCaps;

        fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
            TEST_CODEC
        }

        fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, WorkerCaps>) {
            ctx.registrar::<az_asset_builder::SourceFileEditCodecRegistration>()
                .register(
                    az_asset_builder::SourceFileEditCodecRegistration::from_dynamic_parts(
                        TEST_CODEC_SCHEMA,
                        test_codec_load,
                        test_codec_save,
                    )
                    .with_edit(test_codec_edit),
                );
        }
    }

    fn test_codec_load(
        _context: SourceFileEditCodecContext<'_>,
        request: &BuilderSourceFileEditLoadRequest<'_>,
    ) -> Result<BuilderSourceFileEditDocument, az_asset_builder::SourceFileEditError> {
        let payload = std::str::from_utf8(request.bytes)
            .map_err(|error| az_asset_builder::SourceFileEditError::failed(error.to_string()))?;
        Ok(BuilderSourceFileEditDocument::new(
            request.schema_type.as_str(),
            az_core::ReflectedValueEnvelope::typed_ron(request.schema_type.as_str(), payload),
        ))
    }

    // Registered as a source-file codec callback, so the `Result` return is fixed by
    // that signature; dropping it would not compile.
    #[allow(clippy::unnecessary_wraps)]
    fn test_codec_save(
        _context: SourceFileEditCodecContext<'_>,
        request: &BuilderSourceFileEditSaveRequest<'_>,
    ) -> Result<Vec<u8>, az_asset_builder::SourceFileEditError> {
        Ok(request.value.payload.clone())
    }

    // Registered as a source-file codec callback, so the `Result` return is fixed by
    // that signature; dropping it would not compile.
    #[allow(clippy::unnecessary_wraps)]
    fn test_codec_edit(
        _context: SourceFileEditCodecContext<'_>,
        request: &BuilderSourceFileEditOperationRequest<'_>,
    ) -> Result<BuilderSourceFileEditDocument, az_asset_builder::SourceFileEditError> {
        let mut document = request.document.clone();
        document.value.payload.extend_from_slice(b"|edited");
        Ok(document)
    }

    /// The bridge owns codec selection and staging guarantees; its unit tests
    /// use a local codec contribution so the engine workspace remains project
    /// independent. Downstream projects prove their own composed registries.
    fn source_file_codec_test_registries() -> &'static Registries {
        static REGISTRIES: OnceLock<&'static Registries> = OnceLock::new();
        REGISTRIES.get_or_init(|| {
            let mut composer =
                az_gem_contract::Composer::new(az_gem_contract::GemTargetRole::AssetWorker);
            composer
                .add(
                    SourceCodecHarness,
                    az_gem_contract::ProductActivation::default(),
                )
                .expect("test source codec composes for the worker");
            composer
                .finalize()
                .expect("test source codec composition is valid");
            Box::leak(Box::new(composer)).registries()
        })
    }

    fn source_file_codec_request(
        root: &std::path::Path,
        name: &str,
        source_path: &str,
        schema_type: &str,
        bytes: &[u8],
        operation: SourceFileCodecOperation,
    ) -> SourceFileCodecRequest {
        let capability = worker_capability();
        let authoritative_path = root.join(format!("{name}-authoritative.ron"));
        let authoritative =
            write_named_staging_file_atomic(&authoritative_path, bytes).expect("stage source");
        SourceFileCodecRequest {
            source: az_proto_asset::WorkspaceSourceFileRef {
                source_root_key: "project:test:assets".to_owned(),
                source_path: source_path.to_owned(),
                schema_type: schema_type.to_owned(),
            },
            authoritative_source: SideChannelHandle::staging_file(
                authoritative.path.to_string_lossy(),
                authoritative.byte_length,
                authoritative.content_hash,
                std::env::consts::OS,
            )
            .with_capability(capability.clone()),
            operation,
            output_destination: az_proto_asset::SourceFileCodecOutputDestination {
                locator: root
                    .join(format!("{name}-worker-output.ron"))
                    .to_string_lossy()
                    .into_owned(),
                platform: std::env::consts::OS.to_owned(),
                capability,
            },
        }
    }

    #[test]
    fn source_file_codec_bridge_loads_and_edits_at_processor_destination() {
        let temp = tempfile::tempdir().expect("temporary source staging");
        let schema = TEST_CODEC_SCHEMA.as_str();
        let load = source_file_codec_request(
            temp.path(),
            "load",
            "tests/source.ron",
            schema,
            b"source",
            SourceFileCodecOperation::Load,
        );
        let loaded = execute_source_file_codec_request(source_file_codec_test_registries(), &load)
            .expect("worker loads source through its composed codec");
        assert!(matches!(
            loaded.replacement,
            SourceFileCodecReplacement::Unchanged
        ));
        assert_eq!(loaded.document.value.payload, b"source");

        let edit = source_file_codec_request(
            temp.path(),
            "edit",
            "tests/source.ron",
            schema,
            b"source",
            SourceFileCodecOperation::Edit(SourceFileEditOperation::AppendDefault),
        );
        let output_path = PathBuf::from(&edit.output_destination.locator);
        let edited = execute_source_file_codec_request(source_file_codec_test_registries(), &edit)
            .expect("worker edits source through its composed codec");
        assert_eq!(edited.document.value.payload, b"source|edited");
        let SourceFileCodecReplacement::SavedSource(saved) = edited.replacement else {
            panic!("edit must return the processor-reserved saved source");
        };
        assert_eq!(saved.locator, output_path.to_string_lossy());
        assert_eq!(saved.capability, Some(edit.output_destination.capability));
        assert_eq!(
            std::fs::read(output_path).expect("worker output at processor destination"),
            read_verified_staging_file(&saved)
                .expect("returned saved source verifies")
                .bytes
        );
    }

    #[test]
    fn source_file_codec_bridge_restores_structured_document() {
        let temp = tempfile::tempdir().expect("temporary source staging");
        let schema = TEST_CODEC_SCHEMA.as_str();
        let load = source_file_codec_request(
            temp.path(),
            "restore-load",
            "tests/source.ron",
            schema,
            b"canonical source",
            SourceFileCodecOperation::Load,
        );
        let loaded = execute_source_file_codec_request(source_file_codec_test_registries(), &load)
            .expect("load structured document for restore");
        let restore = source_file_codec_request(
            temp.path(),
            "restore",
            "tests/source.ron",
            schema,
            b"canonical source",
            SourceFileCodecOperation::RestoreDocument(loaded.document.clone()),
        );
        let restored =
            execute_source_file_codec_request(source_file_codec_test_registries(), &restore)
                .expect("worker restores structured document through codec");
        assert_eq!(restored.document, loaded.document);
        assert!(matches!(
            restored.replacement,
            SourceFileCodecReplacement::SavedSource(_)
        ));
        assert!(Path::new(&restore.output_destination.locator).is_file());
    }

    #[test]
    fn source_file_codec_bridge_refuses_mismatched_worker_authority() {
        let temp = tempfile::tempdir().expect("temporary source staging");
        let mut request = source_file_codec_request(
            temp.path(),
            "authority",
            "tests/source.ron",
            TEST_CODEC_SCHEMA.as_str(),
            b"source",
            SourceFileCodecOperation::Load,
        );
        request.output_destination.capability = Capability::new(
            ServiceId::new("project:test", "wrong-worker"),
            ServiceRole::Worker,
        )
        .with_audience(ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_JOBS_PERMISSION])
        .with_token_hash([0x71]);

        let error =
            execute_source_file_codec_request(source_file_codec_test_registries(), &request)
                .expect_err("worker refuses an output destination from another authority");
        assert!(
            error
                .to_string()
                .contains("destination capability does not match worker capability")
        );
        assert!(!Path::new(&request.output_destination.locator).exists());
    }

    fn worker_loop_config(staging_root: PathBuf, cache_root: PathBuf) -> AssetWorkerLoopConfig {
        AssetWorkerLoopConfig::new(
            Endpoint::in_process("asset-processor:unreachable"),
            ASSET_WORKER_SERVICE_NAME,
            worker_capability(),
            staging_root,
            cache_root,
            test_registries(),
        )
    }

    fn test_prefab_builder() -> BuildRule {
        static PATTERNS: OnceLock<Box<[AssetBuilderPattern]>> = OnceLock::new();

        fn no_jobs(_: &CreateJobsRequest<'_>) -> CreateJobsResponse {
            CreateJobsResponse::default()
        }

        fn emit_test_product(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
            ProcessJobResponse {
                products: vec![
                    BuildProduct::from_primary_source_dynamic_parts(
                        request,
                        "prefabs/test.compiled",
                        az_core::asset::ids::CONFIG,
                        ProductFormatId::from_dynamic_parts("az.test.raw"),
                        1,
                        1,
                        b"test product".to_vec(),
                    )
                    .expect("test product path is valid"),
                ],
                product_dependencies: Vec::new(),
                result: az_asset_builder::ProcessJobResult::Success,
            }
        }

        BuildRule {
            name: "az.test.prefab",
            id: BuilderId::new(Uuid::from_bytes([0xb0; 16])),
            primary_source: SourceMatcher::PathOnly(
                PATTERNS
                    .get_or_init(|| {
                        vec![AssetBuilderPattern::wildcard("*.prefab.ron")].into_boxed_slice()
                    })
                    .as_ref(),
            ),
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: no_jobs,
            process_job: emit_test_product,
        }
    }

    #[test]
    fn worker_runtime_enables_timer_driver() {
        let runtime = asset_worker_runtime().expect("worker runtime");
        runtime.block_on(async {
            tokio::time::sleep(Duration::ZERO).await;
        });
    }

    #[test]
    fn worker_default_width_matches_available_parallelism() {
        let expected = std::thread::available_parallelism().map_or(1, usize::from);
        assert_eq!(default_asset_worker_max_inflight(), expected);
    }

    #[test]
    fn worker_blocking_tasks_execute_concurrently() {
        let runtime = asset_worker_runtime().expect("worker runtime");
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        runtime.block_on(async {
            let first_active = Arc::clone(&active);
            let first_peak = Arc::clone(&peak);
            let first = run_asset_worker_blocking_task(1, move || {
                let active = first_active.fetch_add(1, Ordering::SeqCst) + 1;
                first_peak.fetch_max(active, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(50));
                first_active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            });
            let second_active = Arc::clone(&active);
            let second_peak = Arc::clone(&peak);
            let second = run_asset_worker_blocking_task(2, move || {
                let active = second_active.fetch_add(1, Ordering::SeqCst) + 1;
                second_peak.fetch_max(active, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(50));
                second_active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            });
            let (first, second) = tokio::join!(first, second);
            first.expect("first blocking task");
            second.expect("second blocking task");
        });
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn worker_loop_rejects_relative_staging_root_before_connecting() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let Err(error) = prepare_asset_worker_loop(worker_loop_config(
            PathBuf::from("relative-staging"),
            temp.path().join("cache"),
        )) else {
            panic!()
        };
        assert!(matches!(
            error,
            AssetWorkerError::InvalidWorkerRoot {
                flag: "--staging-root",
                ..
            }
        ));
    }

    #[test]
    fn worker_loop_rejects_relative_cache_root_before_connecting() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let Err(error) = prepare_asset_worker_loop(worker_loop_config(
            temp.path().join("staging"),
            PathBuf::from("relative-cache"),
        )) else {
            panic!()
        };
        assert!(matches!(
            error,
            AssetWorkerError::InvalidWorkerRoot {
                flag: "--cache-root",
                ..
            }
        ));
    }

    #[test]
    fn worker_staging_startup_resets_only_the_stable_worker_root() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let staging_parent = temp.path().join("staging");
        let current = staging_parent.join("asset-worker");
        let unrelated = staging_parent.join("runtime-host");
        for root in [&current, &unrelated] {
            std::fs::create_dir_all(root.join("lease")).expect("stale staging root");
            std::fs::write(root.join("lease/payload"), b"stale").expect("stale payload");
        }
        prepare_asset_worker_staging_root(&current).expect("prepare staging root");
        assert!(current.is_dir());
        assert_eq!(
            std::fs::read_dir(&current).expect("current root").count(),
            0
        );
        assert!(unrelated.join("lease/payload").exists());
    }

    #[test]
    fn worker_grant_loader_rejects_noncanonical_service() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let grant = Capability::new(
            ServiceId::new("project", "custom-asset-worker"),
            ServiceRole::Worker,
        )
        .with_audience(ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_JOBS_PERMISSION])
        .with_token_hash([0x77]);
        let path = temp.path().join("grant-custom-asset-worker.capnp.packed");
        let grants = CapabilityGrantSet::from_grants(vec![grant]);
        std::fs::write(
            &path,
            encode_capability_grant_set(&grants).expect("encode grants"),
        )
        .expect("write grants");
        assert!(matches!(
            load_asset_worker_jobs_capability_grant(&path),
            Err(AssetWorkerError::MissingJobsCapabilityGrant)
        ));
    }

    #[test]
    fn worker_grant_loader_rejects_expired_jobs_capability_before_ready() {
        let session = Uuid::from_bytes([0x52; 16]);
        let grant = Capability::new(
            ServiceId::new(ASSET_WORKER_SERVICE_NAMESPACE, ASSET_WORKER_SERVICE_NAME),
            ServiceRole::Worker,
        )
        .with_session(session)
        .with_audience(ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_JOBS_PERMISSION])
        .with_token_hash([0x77])
        .with_expires_unix_ms(1);
        let grants = CapabilityGrantSet::from_grants(vec![grant]);

        let error = encode_capability_grant_set(&grants)
            .expect_err("expired worker grant must fail before readiness publication");
        assert!(error.to_string().contains("expired"));
    }

    #[test]
    fn worker_loop_requires_brokered_canonical_project_jobs_capability() {
        let valid = worker_capability();
        validate_asset_worker_jobs_capability(&valid).expect("valid capability");

        let mut missing_token = valid.clone();
        missing_token.token_hash.clear();
        assert!(matches!(
            validate_asset_worker_jobs_capability(&missing_token),
            Err(AssetWorkerError::InvalidJobsCapability { .. })
        ));

        let wrong_session = valid.with_session(Uuid::from_bytes([0x44; 16]));
        assert!(matches!(
            validate_asset_worker_jobs_capability(&wrong_session),
            Err(AssetWorkerError::InvalidJobsCapability { .. })
        ));
    }

    #[test]
    fn worker_rejects_unregistered_product_format_before_manifest_publish() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let product = BuildProduct::from_dynamic_parts(
            "cache/textures/rpc.dds",
            az_core::asset::ids::CONFIG,
            ProductFormatId::from_dynamic_parts("az.test.unregistered"),
            1,
            4,
            b"rpc product bytes".to_vec(),
        )
        .expect("test product path is valid");
        let error = stage_products(
            42,
            1_000,
            temp.path(),
            vec![product],
            Vec::new(),
            test_registries(),
        )
        .expect_err("unregistered format must fail");
        assert!(matches!(
            error,
            AssetWorkerError::UnregisteredProductFormat {
                asset_job_attempt_id: 42,
                ..
            }
        ));
    }

    #[test]
    fn worker_rejects_product_format_outside_builder_declaration() {
        const EXPECTED_FORMATS: &[ProductFormatId] =
            &[ProductFormatId::__from_static("az.test.expected")];
        let temp = tempfile::tempdir().expect("temporary directory");
        let staging_root = temp.path().join("attempt-42");
        let mut builder = test_prefab_builder();
        builder.product_formats = ProductFormatPolicy::Declared(EXPECTED_FORMATS);
        let attempt = LeasedAssetJob {
            attempt_id: 42,
            workspace_id: 7,
            owner: JobOwner::Build(builder.id.0),
            source_guid: Uuid::from_bytes([0x42; 16]),
            preserved_source_sub_id: None,
            source_path: "prefabs/test.prefab.ron".to_owned(),
            source_root: temp.path().display().to_string(),
            source_schema_type: Some("az.test.Prefab".to_owned()),
            job_key: "compile-prefab".to_owned(),
            platform: DEFAULT_PLATFORM.to_owned(),
            ordinal: 1,
            staging_root: staging_root.display().to_string(),
            source_payload: None,
        };

        let Err(error) = prepare_process_job_completion(
            &builder,
            &attempt,
            b"prefab source",
            temp.path(),
            &CancellationToken::new(),
            &worker_capability(),
            1_000,
            test_registries(),
        ) else {
            panic!("undeclared product format must fail before staging")
        };

        assert!(matches!(
            error,
            AssetWorkerError::UndeclaredProductFormat {
                asset_job_attempt_id: 42,
                builder: "az.test.prefab",
                ..
            }
        ));
        assert!(!staging_root.exists());
    }

    #[test]
    fn worker_stages_alias_without_creating_second_file() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let product = BuildProduct::from_dynamic_parts(
            "prefabs/slices/player.scn.bin",
            az_core::asset::ids::CONFIG,
            ProductFormatId::from_dynamic_parts("az.test.raw"),
            1,
            1,
            b"one canonical product".to_vec(),
        )
        .expect("canonical product path is valid")
        .with_catalog_aliases(vec![
            az_asset_builder::ProductPath::new("slices/player.dynamicslice")
                .expect("catalog alias is valid"),
        ]);
        let manifest = stage_products(
            42,
            1_000,
            temp.path(),
            vec![product],
            Vec::new(),
            test_registries(),
        )
        .expect("stage product");
        assert_eq!(manifest.products.len(), 1);
        assert_eq!(
            manifest.products[0].product_path,
            "prefabs/slices/player.scn.bin"
        );
        assert_eq!(
            manifest.products[0].catalog_aliases,
            vec!["slices/player.dynamicslice"]
        );
        assert!(temp.path().join("prefabs/slices/player.scn.bin").is_file());
        assert!(!temp.path().join("slices/player.dynamicslice").exists());
    }

    #[test]
    fn worker_stages_shared_physical_product_once_for_logical_ids() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let product = |sub_id, registration| {
            BuildProduct::from_dynamic_parts(
                "slices/shared.slice.meta",
                az_core::asset::ids::CONFIG,
                ProductFormatId::from_dynamic_parts("az.test.raw"),
                1,
                sub_id,
                b"shared slice metadata".to_vec(),
            )
            .expect("shared product path is valid")
            .with_catalog_path_registration(registration)
        };
        let manifest = stage_products(
            42,
            1_000,
            temp.path(),
            vec![
                product(3, az_asset::AssetCatalogPathRegistration::AssetIdOnly),
                product(7, az_asset::AssetCatalogPathRegistration::Registered),
            ],
            Vec::new(),
            test_registries(),
        )
        .expect("stage shared product");
        assert_eq!(manifest.products.len(), 2);
        assert_eq!(
            manifest
                .products
                .iter()
                .map(|product| (product.sub_id, product.catalog_path_registration))
                .collect::<Vec<_>>(),
            [
                (3, CatalogPathRegistration::AssetIdOnly),
                (7, CatalogPathRegistration::Registered),
            ]
        );
        assert_eq!(
            std::fs::read(temp.path().join("slices/shared.slice.meta"))
                .expect("shared staged product"),
            b"shared slice metadata"
        );
    }

    #[test]
    fn worker_groups_product_dependencies_in_manifest() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let dependency_guid = Uuid::from_bytes([0xd6; 16]);
        let product = BuildProduct::from_dynamic_parts(
            "cache/textures/rpc.dds",
            az_core::asset::ids::CONFIG,
            ProductFormatId::from_dynamic_parts("az.test.raw"),
            1,
            4,
            b"rpc product bytes".to_vec(),
        )
        .expect("test product path is valid");
        let manifest = stage_products(
            42,
            1_000,
            temp.path(),
            vec![product],
            vec![(
                0,
                ProductDependency {
                    dependency_id: az_core::AssetId::new(dependency_guid, 7),
                    flags: ProductDependencyFlags::NO_LOAD,
                },
            )],
            test_registries(),
        )
        .expect("stage dependency product");
        assert_eq!(
            manifest.products[0].dependencies,
            vec![ProductManifestProductDependency {
                asset_guid: dependency_guid,
                sub_id: 7,
                flags: ProductDependencyFlags::NO_LOAD.0,
            }]
        );
        assert!(encode_product_manifest(&manifest).is_ok());
    }

    #[test]
    fn worker_product_manifest_side_channel_publish_leaves_only_final_verified_file() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let expected_capability = worker_capability();
        let manifest = ProductManifest::new(
            1_234,
            vec![ProductManifestProduct {
                product_path: "cache/textures/atomic.dds".to_string(),
                catalog_aliases: Vec::new(),
                catalog_path_registration: CatalogPathRegistration::Registered,
                asset_type: Uuid::from_bytes([0xc4; 16]),
                sub_id: 7,
                product_format: "az.test.raw".to_string(),
                product_format_version: 1,
                staged_path: "cache/textures/atomic.dds".to_string(),
                content_hash: blake3::hash(b"product bytes").as_bytes().to_vec(),
                byte_length: 13,
                dependencies: Vec::new(),
            }],
        );

        let handle =
            write_product_manifest_side_channel(&expected_capability, 42, temp.path(), &manifest)
                .expect("write product manifest side channel");

        let path = PathBuf::from(&handle.locator);
        assert!(path.starts_with(temp.path()));
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("attempt-42-products.capnp.packed")
        );
        assert_eq!(handle.capability.as_ref(), Some(&expected_capability));
        let bytes = std::fs::read(&path).expect("read product manifest side channel");
        assert_eq!(handle.byte_length, bytes.len() as u64);
        assert_eq!(
            handle.content_hash,
            blake3::hash(&bytes).as_bytes().to_vec()
        );
        assert_eq!(
            az_proto_asset::load_product_manifest_side_channel(&handle).expect("load manifest"),
            manifest
        );
        assert_eq!(
            temp.path()
                .read_dir()
                .expect("read staging root")
                .filter(|entry| {
                    entry
                        .as_ref()
                        .expect("staging entry")
                        .file_name()
                        .to_string_lossy()
                        .starts_with('.')
                })
                .count(),
            0
        );
    }
}
