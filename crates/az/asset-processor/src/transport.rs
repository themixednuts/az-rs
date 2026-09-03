use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::thread;

use az_assetdb::{AssetDb, AssetDbWriter, OpenError, RepoError, SelectWorkspaces, WorkspaceKey};
use az_filesystem::{AzothDataHome, ProjectDataPaths};
use az_project::PortableKey;
use az_proto_asset::asset_capnp;
use az_proto_core::{
    CapabilityGrantSet, CapabilityGrantSetValidationError, Endpoint, EndpointKind,
    decode_capability_grant_set,
};
use az_rpc::AzRpcTransportError;
use az_service_catalog::ASSET_PROCESSOR_CAPABILITY_REQUIREMENTS;
use futures::future::Either;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::oneshot;
use tokio::task::LocalSet;
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    AssetProcessor, AssetProcessorDatabaseHandles, AssetProcessorError, AssetProcessorRpc,
    AssetSourceServiceCoordination, RegisteredSourceRoot, RegisteredWorkspaceAssetDb,
    product_cache_transaction_root, recover_product_cache_transactions,
};

#[derive(Debug, Error)]
pub enum AssetProcessorRpcTransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Rpc(#[from] AzRpcTransportError),

    #[error("asset DB open failed: {0}")]
    Open(#[from] OpenError),

    #[error("asset DB query failed: {0}")]
    AssetDb(#[from] RepoError),

    // Boxed: `AssetProcessorError` is the widest error in this crate, and
    // inlining it here made every `Result<_, AssetProcessorRpcTransportError>`
    // oversized. The hand-written `From` below keeps `?` working, since
    // `#[from]` on a boxed field would only yield `From<Box<..>>`.
    #[error("asset job dispatcher failed during server lifecycle: {0}")]
    Dispatcher(#[source] Box<AssetProcessorError>),

    #[error("project data-home setup failed: {0}")]
    DataHome(#[from] az_filesystem::DataHomeError),

    #[error("failed to recover product cache transactions under `{root}`: {source}")]
    ProductCacheTransactionRecovery {
        root: PathBuf,
        #[source]
        source: az_filesystem::FileTransactionError,
    },

    #[error("capability grant decode error: {0}")]
    CapabilityGrantDecode(#[from] capnp::Error),

    #[error("asset-processor RPC endpoint requires at least one brokered capability grant")]
    EmptyCapabilityGrantSet,

    #[error("invalid asset-processor RPC capability grant set: {0}")]
    InvalidCapabilityGrantSet(#[from] CapabilityGrantSetValidationError),

    #[error("invalid asset-processor RPC launch context: {reason}")]
    InvalidLaunchContext { reason: String },

    #[error(
        "asset-processor DB has no workspace for the project service; startup must register DB-owned source roots before serving RPC"
    )]
    MissingWorkspace,

    #[error(
        "asset-processor DB workspace {workspace_id} has no source roots; startup must register DB-owned source roots before serving RPC"
    )]
    MissingWorkspaceSourceRoots { workspace_id: i64 },

    #[error("asset-processor DB workspace {workspace_id} is invalid: {reason}")]
    InvalidWorkspace { workspace_id: i64, reason: String },

    #[error(
        "asset-processor DB source root {workspace_source_root_id} for workspace {workspace_id} is invalid: {reason}"
    )]
    InvalidWorkspaceSourceRoot {
        workspace_id: i64,
        workspace_source_root_id: i64,
        reason: String,
    },

    #[error("endpoint kind `{0:?}` is not supported on this platform")]
    UnsupportedEndpoint(EndpointKind),

    #[error("asset-processor RPC server thread failed to start")]
    StartupChannelClosed,

    #[error("asset-processor source watcher thread failed to start: {source}")]
    StartupSourceWatcherSpawn {
        #[source]
        source: std::io::Error,
    },

    #[error("asset-processor sweep owner thread failed to start: {source}")]
    StartupSweepOwnerSpawn {
        #[source]
        source: std::io::Error,
    },
}

/// Boxes the dispatcher error on the way in, so `?` still converts an
/// [`AssetProcessorError`] into the transport error without widening it.
impl From<AssetProcessorError> for AssetProcessorRpcTransportError {
    fn from(error: AssetProcessorError) -> Self {
        Self::Dispatcher(Box::new(error))
    }
}

/// Reports every asset-processor service thread that panicked during shutdown.
#[derive(Debug, Error)]
#[error("asset-processor service threads panicked: {threads:?}")]
pub struct AssetProcessorRpcServerStopError {
    threads: Vec<&'static str>,
}

impl AssetProcessorRpcServerStopError {
    /// Returns the stable labels of all service threads that panicked.
    #[must_use]
    pub fn threads(&self) -> &[&'static str] {
        &self.threads
    }
}

pub struct AssetProcessorRpcServer {
    endpoint: Endpoint,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
    source_watcher: Option<crate::watcher::SourceWatcherHandle>,
    sweep_owner: Option<crate::sweep::SweepOwner>,
}

#[derive(Debug, Clone)]
struct AssetProcessorStartupScope {
    workspace_id: i64,
    project_data_paths: ProjectDataPaths,
    source_roots: Vec<RegisteredSourceRoot>,
}

impl AssetProcessorStartupScope {
    fn new(
        data_home: &AzothDataHome,
        project_id: &str,
        workspace_root: &Path,
        workspace_id: i64,
    ) -> Result<Self, AssetProcessorRpcTransportError> {
        let project_data_paths = data_home.project(project_id, workspace_root);
        project_data_paths.prepare()?;
        let transaction_root = product_cache_transaction_root(&project_data_paths);
        let recovered_write_count = recover_product_cache_transactions(&project_data_paths)
            .map_err(
                |source| AssetProcessorRpcTransportError::ProductCacheTransactionRecovery {
                    root: transaction_root.clone(),
                    source,
                },
            )?;
        if recovered_write_count != 0 {
            tracing::info!(
                recovered_write_count,
                root = %transaction_root.display(),
                "recovered pending product cache transaction during asset-processor startup"
            );
        }
        Ok(Self {
            workspace_id,
            project_data_paths,
            source_roots: Vec::new(),
        })
    }

    fn with_source_roots(mut self, source_roots: Vec<RegisteredSourceRoot>) -> Self {
        self.source_roots = source_roots;
        self
    }
}

impl AssetProcessorRpcServer {
    #[must_use]
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Stops and joins every thread owned by the RPC server.
    ///
    /// All thread panics are aggregated into one terminal error.
    ///
    /// # Errors
    ///
    /// Returns [`AssetProcessorRpcServerStopError`] naming every service thread that
    /// panicked. All thread panics are aggregated into that one terminal error.
    pub fn stop(mut self) -> Result<(), AssetProcessorRpcServerStopError> {
        self.shutdown();
        let mut panicked = Vec::new();
        if self
            .source_watcher
            .take()
            .is_some_and(|source_watcher| source_watcher.stop().is_err())
        {
            panicked.push("source watcher");
        }
        if self
            .thread
            .take()
            .is_some_and(|thread| thread.join().is_err())
        {
            panicked.push("RPC server");
        }
        if self
            .sweep_owner
            .take()
            .is_some_and(|owner| owner.stop().is_err())
        {
            panicked.push("sweep owner");
        }
        if panicked.is_empty() {
            Ok(())
        } else {
            Err(AssetProcessorRpcServerStopError { threads: panicked })
        }
    }

    fn shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

impl Drop for AssetProcessorRpcServer {
    fn drop(&mut self) {
        // Signal AND join: the listener thread must not outlive this struct.
        // `stop()` already does both explicitly; mirroring the thread teardown
        // here means a server
        // dropped without an explicit `stop()` call still tears its RPC
        // thread down deterministically instead of leaking it past the
        // struct's lifetime.
        self.shutdown();
        if let Some(source_watcher) = self.source_watcher.take() {
            let _ = source_watcher.stop();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some(owner) = self.sweep_owner.take() {
            let _ = owner.stop();
        }
    }
}

/// Start the production RPC service from the registered, still-open database
/// created during source-root registration.
///
/// Consuming the registered database preserves its Turso owner across all
/// startup phases instead of reconstructing the database for each phase.
///
/// # Errors
///
/// Returns [`AssetProcessorRpcTransportError::CapabilityGrantDecode`] if the grants file cannot be read or
/// decoded, plus the same startup failures as
/// [`start_asset_processor_rpc_server_with_capability_grants`].
pub fn start_asset_processor_rpc_server_with_registered_workspace_db(
    registered_db: RegisteredWorkspaceAssetDb,
    endpoint: Endpoint,
    run: Uuid,
    capability_grants_file: impl AsRef<Path>,
) -> Result<AssetProcessorRpcServer, AssetProcessorRpcTransportError> {
    let data_home = AzothDataHome::resolve();
    let (db_path, database, asset_db_writer, source_roots, registration) =
        registered_db.into_parts();
    let bytes = std::fs::read(capability_grants_file.as_ref())?;
    let capability_grants = decode_capability_grant_set(&bytes)?;
    validate_asset_processor_rpc_capability_grants(&capability_grants)?;
    let workspace_id = validate_asset_processor_db_registered_for_workspace(
        &database,
        &registration.project_id,
        &registration.workspace_root.to_string_lossy(),
        &registration.branch,
        &source_roots,
    )?;
    if workspace_id != registration.workspace_id {
        return Err(AssetProcessorRpcTransportError::InvalidWorkspace {
            workspace_id,
            reason: format!(
                "startup registration selected workspace {}, but the database resolved {workspace_id}",
                registration.workspace_id
            ),
        });
    }
    start_asset_processor_rpc_server_with_database(
        database,
        asset_db_writer,
        db_path,
        endpoint,
        run,
        capability_grants,
        Some(
            AssetProcessorStartupScope::new(
                &data_home,
                &registration.project_id,
                &registration.workspace_root,
                workspace_id,
            )?
            .with_source_roots(source_roots.clone()),
        ),
        Some(source_roots),
        true,
    )
}

/// # Errors
///
/// Returns [`AssetProcessorRpcTransportError::EmptyCapabilityGrantSet`] if no brokered grant was
/// supplied, [`AssetProcessorRpcTransportError::InvalidCapabilityGrantSet`] if the grant set fails
/// validation, [`AssetProcessorRpcTransportError::Open`] or [`AssetProcessorRpcTransportError::AssetDb`] if the `AssetDB` cannot be
/// opened or queried, [`AssetProcessorRpcTransportError::MissingWorkspace`],
/// [`AssetProcessorRpcTransportError::MissingWorkspaceSourceRoots`], [`AssetProcessorRpcTransportError::InvalidWorkspace`] or
/// [`AssetProcessorRpcTransportError::InvalidWorkspaceSourceRoot`] if startup has not registered usable
/// DB-owned source roots, [`AssetProcessorRpcTransportError::UnsupportedEndpoint`] if the endpoint kind
/// is not supported on this platform, [`AssetProcessorRpcTransportError::ProductCacheTransactionRecovery`]
/// if a pending product-cache transaction cannot be recovered, and the
/// `Startup*` variants if a service thread fails to start.
#[cfg(any(test, feature = "test-support"))]
pub fn start_asset_processor_rpc_server_with_capability_grants(
    db_path: impl AsRef<Path>,
    endpoint: Endpoint,
    capability_grants: CapabilityGrantSet,
) -> Result<AssetProcessorRpcServer, AssetProcessorRpcTransportError> {
    if capability_grants.is_empty() {
        return Err(AssetProcessorRpcTransportError::EmptyCapabilityGrantSet);
    }
    let db_path = db_path.as_ref().to_path_buf();
    let database = AssetDb::open(&db_path)?;
    let asset_db_writer = database.writer()?;
    let startup_scope = test_startup_scope(&database, &db_path)?;
    start_asset_processor_rpc_server_with_database(
        database,
        asset_db_writer,
        db_path,
        endpoint,
        Uuid::now_v7(),
        capability_grants,
        Some(startup_scope),
        None,
        false,
    )
}

#[cfg(any(test, feature = "test-support"))]
fn test_startup_scope(
    database: &AssetDb,
    db_path: &Path,
) -> Result<AssetProcessorStartupScope, AssetProcessorRpcTransportError> {
    let mut views = database.workspaces_for_test()?;
    if views.len() != 1 {
        return Err(AssetProcessorRpcTransportError::MissingWorkspace);
    }
    let view = views.pop().expect("length checked");
    let workspace_root = PathBuf::from(&view.root);
    let data_home = AzothDataHome::new(
        db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("azoth-test-home"),
    );
    AssetProcessorStartupScope::new(
        &data_home,
        &view.project,
        &workspace_root,
        view.workspace_id,
    )
}

/// # Errors
///
/// Returns the same failures as
/// [`start_asset_processor_rpc_server_with_capability_grants`]; the extra builder
/// registry does not add a failure mode.
#[cfg(any(test, feature = "test-support"))]
pub fn start_asset_processor_rpc_server_with_builder_registry_and_capability_grants(
    db_path: impl AsRef<Path>,
    endpoint: Endpoint,
    capability_grants: CapabilityGrantSet,
    builders: az_asset_builder::BuildRuleRegistry,
) -> Result<AssetProcessorRpcServer, AssetProcessorRpcTransportError> {
    if capability_grants.is_empty() {
        return Err(AssetProcessorRpcTransportError::EmptyCapabilityGrantSet);
    }

    match endpoint.kind {
        EndpointKind::Tcp => start_tcp_server_with_builder_registry(
            db_path.as_ref().to_path_buf(),
            endpoint,
            Uuid::now_v7(),
            capability_grants,
            builders,
        ),
        EndpointKind::WindowsNamedPipe
        | EndpointKind::UnixDomainSocket
        | EndpointKind::InProcess => Err(AssetProcessorRpcTransportError::UnsupportedEndpoint(
            endpoint.kind,
        )),
    }
}

/// The service one asset-processor RPC listener will serve, independent of the
/// transport that carries it.
///
/// TCP, named-pipe and Unix-socket startup each hand this identical set down
/// through a listener to [`asset_processor_rpc`], which is where it is finally
/// consumed. Spelled out, that is the same positional list repeated in six
/// signatures across two platform `cfg` pairs, several of whose entries share a
/// type -- a listener that transposed two of them would still compile. It
/// travels as one value so a transport can forward the service without being
/// able to reshape it.
struct AssetProcessorServiceSetup {
    database: AssetDb,
    asset_db_writer: AssetDbWriter,
    db_path: PathBuf,
    service_run: Uuid,
    capability_grants: CapabilityGrantSet,
    source_coordination: AssetSourceServiceCoordination,
    startup_scope: Option<AssetProcessorStartupScope>,
}

// This is the assembly point: it is the one function that still receives the
// startup facts loose, decides the source coordination from
// `spawn_source_watcher`, and builds the `AssetProcessorServiceSetup` every
// listener below it takes as a single value. Grouping its own parameters would
// only re-declare that same list one level up, and `endpoint`, `source_roots`
// and `spawn_source_watcher` are not part of the service it assembles.
#[expect(
    clippy::too_many_arguments,
    reason = "assembles AssetProcessorServiceSetup from loose startup facts; grouping its inputs would restate that struct"
)]
#[expect(
    clippy::significant_drop_tightening,
    reason = "`setup` is moved into the per-endpoint server constructor in three of the four match arms, so the suggested `drop(setup)` after the match is a use-after-move"
)]
fn start_asset_processor_rpc_server_with_database(
    database: AssetDb,
    asset_db_writer: AssetDbWriter,
    db_path: PathBuf,
    endpoint: Endpoint,
    service_run: Uuid,
    capability_grants: CapabilityGrantSet,
    startup_scope: Option<AssetProcessorStartupScope>,
    source_roots: Option<Vec<RegisteredSourceRoot>>,
    spawn_source_watcher: bool,
) -> Result<AssetProcessorRpcServer, AssetProcessorRpcTransportError> {
    if capability_grants.is_empty() {
        return Err(AssetProcessorRpcTransportError::EmptyCapabilityGrantSet);
    }

    let source_coordination = if spawn_source_watcher {
        AssetSourceServiceCoordination::production()
    } else {
        AssetSourceServiceCoordination::default()
    };
    let mut sweep_owner = {
        let sweep_database = spawn_source_watcher
            .then(|| database.new_runtime_handle())
            .transpose()?;
        match (sweep_database, source_roots.clone()) {
            (Some(database), Some(roots)) => Some(
                crate::sweep::SweepOwner::start(
                    database,
                    asset_db_writer.clone(),
                    roots,
                    source_coordination.builder_catalog_store(),
                )
                .map_err(|source| {
                    AssetProcessorRpcTransportError::StartupSweepOwnerSpawn { source }
                })?,
            ),
            _ => None,
        }
    };
    if let Some(owner) = sweep_owner.as_ref() {
        source_coordination.install_sweep(owner.handle());
    }
    let setup = AssetProcessorServiceSetup {
        database,
        asset_db_writer,
        db_path,
        service_run,
        capability_grants,
        source_coordination: source_coordination.clone(),
        startup_scope: startup_scope.clone(),
    };
    let mut server = match endpoint.kind {
        EndpointKind::Tcp => start_tcp_server(setup, endpoint),
        EndpointKind::WindowsNamedPipe => start_named_pipe_server(setup, &endpoint),
        EndpointKind::UnixDomainSocket => start_unix_socket_server(setup, &endpoint),
        EndpointKind::InProcess => Err(AssetProcessorRpcTransportError::UnsupportedEndpoint(
            EndpointKind::InProcess,
        )),
    }?;

    if let Some(scope) = startup_scope {
        let source_roots = source_roots.ok_or(
            AssetProcessorRpcTransportError::MissingWorkspaceSourceRoots {
                workspace_id: scope.workspace_id,
            },
        )?;
        let watcher = crate::watcher::spawn_source_watcher(
            scope.workspace_id,
            source_roots,
            source_coordination.sweep_handle().ok_or(
                AssetProcessorRpcTransportError::MissingWorkspaceSourceRoots {
                    workspace_id: scope.workspace_id,
                },
            )?,
        )
        .map_err(|source| AssetProcessorRpcTransportError::StartupSourceWatcherSpawn { source })?;
        server.source_watcher = Some(watcher);
    }
    server.sweep_owner = sweep_owner.take();

    Ok(server)
}

fn validate_asset_processor_rpc_capability_grants(
    capability_grants: &CapabilityGrantSet,
) -> Result<(), AssetProcessorRpcTransportError> {
    capability_grants
        .validate_exact_brokered_for_project(ASSET_PROCESSOR_CAPABILITY_REQUIREMENTS)?;
    Ok(())
}

fn validate_asset_processor_db_registered_for_workspace(
    db: &AssetDb,
    project_id: &str,
    workspace_root: &str,
    branch: &str,
    source_roots: &[RegisteredSourceRoot],
) -> Result<i64, AssetProcessorRpcTransportError> {
    validate_asset_processor_launch_context(project_id, workspace_root, branch)?;
    let key = WorkspaceKey {
        project: project_id.to_owned(),
        root: workspace_root.to_owned(),
        branch: branch.to_owned(),
    };
    let view = db
        .workspace(&key)?
        .ok_or_else(|| AssetProcessorRpcTransportError::MissingWorkspace)?;
    validate_registered_workspace(&view, project_id, workspace_root, branch)?;
    if source_roots.is_empty() {
        return Err(
            AssetProcessorRpcTransportError::MissingWorkspaceSourceRoots {
                workspace_id: view.workspace_id,
            },
        );
    }
    validate_registered_workspace_source_roots(&view, source_roots)?;

    Ok(view.workspace_id)
}

fn validate_asset_processor_launch_context(
    project_id: &str,
    workspace_root: &str,
    branch: &str,
) -> Result<(), AssetProcessorRpcTransportError> {
    if project_id.trim().is_empty() {
        return invalid_asset_processor_launch_context("project id cannot be empty");
    }
    if workspace_root.trim().is_empty() {
        return invalid_asset_processor_launch_context("workspace root cannot be empty");
    }
    if branch.trim().is_empty() {
        return invalid_asset_processor_launch_context("branch cannot be empty");
    }
    Ok(())
}

fn validate_registered_workspace(
    view: &SelectWorkspaces,
    expected_project_id: &str,
    expected_workspace_root: &str,
    expected_branch: &str,
) -> Result<(), AssetProcessorRpcTransportError> {
    if view.workspace_id <= 0 {
        return invalid_workspace(view, "workspace id must be positive");
    }
    if view.project.trim().is_empty() {
        return invalid_workspace(view, "project id cannot be empty");
    }
    if view.project != expected_project_id {
        return invalid_workspace(
            view,
            format!(
                "project id `{}` does not match launch project `{expected_project_id}`",
                view.project
            ),
        );
    }
    if view.root.trim().is_empty() {
        return invalid_workspace(view, "workspace root cannot be empty");
    }
    if view.root != expected_workspace_root {
        return invalid_workspace(
            view,
            format!(
                "workspace root `{}` does not match launch workspace `{expected_workspace_root}`",
                view.root
            ),
        );
    }
    if view.branch.trim().is_empty() {
        return invalid_workspace(view, "branch cannot be empty");
    }
    if view.branch != expected_branch {
        return invalid_workspace(
            view,
            format!(
                "branch `{}` does not match launch branch `{expected_branch}`",
                view.branch
            ),
        );
    }
    if view.created < 0 || view.updated < 0 {
        return invalid_workspace(view, "created/updated timestamps cannot be negative");
    }
    Ok(())
}

fn validate_registered_workspace_source_roots(
    view: &SelectWorkspaces,
    source_roots: &[RegisteredSourceRoot],
) -> Result<(), AssetProcessorRpcTransportError> {
    let mut seen_source_root_ids = BTreeSet::new();
    let mut seen_portable_keys = BTreeSet::new();
    let project_assets_root_key = String::from(PortableKey::project_assets(&view.project));
    let mut project_assets_root = None;
    for root in source_roots {
        if root.workspace_root_pk <= 0 {
            return invalid_workspace_source_root(
                view.workspace_id,
                root,
                "workspace source root id must be positive",
            );
        }
        if root.root_pk <= 0 {
            return invalid_workspace_source_root(
                view.workspace_id,
                root,
                "scan folder id must be positive",
            );
        }
        if root.workspace_pk != view.workspace_id {
            return invalid_workspace_source_root(
                view.workspace_id,
                root,
                format!(
                    "source root belongs to workspace {}, expected {}",
                    root.workspace_pk, view.workspace_id
                ),
            );
        }
        if root.display_name.trim().is_empty() {
            return invalid_workspace_source_root(
                view.workspace_id,
                root,
                "display name cannot be empty",
            );
        }
        if root.owner.trim().is_empty() {
            return invalid_workspace_source_root(
                view.workspace_id,
                root,
                "owner id cannot be empty",
            );
        }
        if root.path.trim().is_empty() {
            return invalid_workspace_source_root(
                view.workspace_id,
                root,
                "source root path cannot be empty",
            );
        }
        if root.portable_key.trim().is_empty() {
            return invalid_workspace_source_root(
                view.workspace_id,
                root,
                "portable key cannot be empty",
            );
        }
        if !seen_source_root_ids.insert(root.workspace_root_pk) {
            return invalid_workspace_source_root(
                view.workspace_id,
                root,
                format!("duplicate source root id {}", root.workspace_root_pk),
            );
        }
        if !seen_portable_keys.insert(root.portable_key.as_str()) {
            return invalid_workspace_source_root(
                view.workspace_id,
                root,
                format!("duplicate source root key `{}`", root.portable_key),
            );
        }
        if root.portable_key == project_assets_root_key {
            project_assets_root = Some(root);
        }
    }

    let Some(project_assets_root) = project_assets_root else {
        return invalid_workspace(
            view,
            format!("missing DB-owned project assets root `{project_assets_root_key}`"),
        );
    };
    validate_registered_project_assets_root(view, project_assets_root)?;
    Ok(())
}

fn validate_registered_project_assets_root(
    view: &SelectWorkspaces,
    root: &RegisteredSourceRoot,
) -> Result<(), AssetProcessorRpcTransportError> {
    if root.owner != view.project {
        return invalid_workspace_source_root(
            view.workspace_id,
            root,
            format!(
                "project assets root owner `{}` does not match project `{}`",
                root.owner, view.project
            ),
        );
    }
    let workspace_root = PathBuf::from(&view.root);
    let source_root = PathBuf::from(&root.path);
    if source_root == workspace_root || !source_root.starts_with(&workspace_root) {
        return invalid_workspace_source_root(
            view.workspace_id,
            root,
            format!(
                "project assets root path `{}` must be inside workspace root `{}`",
                root.path, view.root
            ),
        );
    }
    if !root.role.is_required() {
        return invalid_workspace_source_root(
            view.workspace_id,
            root,
            "project assets root must be marked as a root source",
        );
    }
    if !root.output_prefix.is_empty() {
        return invalid_workspace_source_root(
            view.workspace_id,
            root,
            "project assets root must use an empty output prefix",
        );
    }
    Ok(())
}

fn invalid_asset_processor_launch_context(
    reason: impl Into<String>,
) -> Result<(), AssetProcessorRpcTransportError> {
    Err(AssetProcessorRpcTransportError::InvalidLaunchContext {
        reason: reason.into(),
    })
}

fn invalid_workspace(
    view: &SelectWorkspaces,
    reason: impl Into<String>,
) -> Result<(), AssetProcessorRpcTransportError> {
    Err(AssetProcessorRpcTransportError::InvalidWorkspace {
        workspace_id: view.workspace_id,
        reason: reason.into(),
    })
}

fn invalid_workspace_source_root(
    workspace_id: i64,
    root: &RegisteredSourceRoot,
    reason: impl Into<String>,
) -> Result<(), AssetProcessorRpcTransportError> {
    Err(
        AssetProcessorRpcTransportError::InvalidWorkspaceSourceRoot {
            workspace_id,
            workspace_source_root_id: root.workspace_root_pk,
            reason: reason.into(),
        },
    )
}

/// # Errors
///
/// Returns [`AssetProcessorRpcTransportError::UnsupportedEndpoint`] if the endpoint kind is not supported on
/// this platform, and [`AssetProcessorRpcTransportError::Rpc`] if the connection or the Cap'n Proto
/// bootstrap handshake fails.
pub async fn connect_asset_processor_rpc_client(
    endpoint: &Endpoint,
) -> Result<asset_capnp::asset_processor::Client, AssetProcessorRpcTransportError> {
    Ok(az_rpc::connect_twoparty_bootstrap(endpoint).await?)
}

fn start_tcp_server(
    setup: AssetProcessorServiceSetup,
    mut endpoint: Endpoint,
) -> Result<AssetProcessorRpcServer, AssetProcessorRpcTransportError> {
    let listener = std::net::TcpListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    // Binding to port 0 lets the OS pick, so the endpoint the caller asked for
    // is only a request until the listener exists: carry the bound address back.
    endpoint.address = listener.local_addr()?.to_string();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (ready_tx, ready_rx) =
        std::sync::mpsc::sync_channel::<Result<(), AssetProcessorRpcTransportError>>(1);
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        let rpc = match asset_processor_rpc(setup) {
            Ok(rpc) => rpc,
            Err(error) => {
                let _ = ready_tx.send(Err(error));
                return;
            }
        };
        run_threaded_local(async move {
            let listener = match TcpListener::from_std(listener) {
                Ok(listener) => listener,
                Err(error) => {
                    let _ = ready_tx.send(Err(error.into()));
                    return Ok(());
                }
            };
            let _ = ready_tx.send(Ok(()));
            run_tcp_listener(thread_endpoint, listener, shutdown_rx, rpc).await
        });
    });
    ready_rx
        .recv()
        .map_err(|_| AssetProcessorRpcTransportError::StartupChannelClosed)??;
    Ok(AssetProcessorRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
        source_watcher: None,
        sweep_owner: None,
    })
}

#[cfg(any(test, feature = "test-support"))]
fn start_tcp_server_with_builder_registry(
    db_path: PathBuf,
    mut endpoint: Endpoint,
    service_run: Uuid,
    capability_grants: CapabilityGrantSet,
    builders: az_asset_builder::BuildRuleRegistry,
) -> Result<AssetProcessorRpcServer, AssetProcessorRpcTransportError> {
    let listener = std::net::TcpListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    // Binding to port 0 lets the OS pick, so the endpoint the caller asked for
    // is only a request until the listener exists: carry the bound address back.
    endpoint.address = listener.local_addr()?.to_string();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (ready_tx, ready_rx) =
        std::sync::mpsc::sync_channel::<Result<(), AssetProcessorRpcTransportError>>(1);
    let thread_endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        let rpc = match asset_processor_rpc_for_db_with_builder_registry(
            db_path,
            service_run,
            capability_grants,
            builders,
        ) {
            Ok(bootstrap) => bootstrap,
            Err(error) => {
                let _ = ready_tx.send(Err(error));
                return;
            }
        };
        run_threaded_local(async move {
            let listener = match TcpListener::from_std(listener) {
                Ok(listener) => listener,
                Err(error) => {
                    let _ = ready_tx.send(Err(error.into()));
                    return Ok(());
                }
            };
            let _ = ready_tx.send(Ok(()));
            run_tcp_listener(thread_endpoint, listener, shutdown_rx, rpc).await
        });
    });
    ready_rx
        .recv()
        .map_err(|_| AssetProcessorRpcTransportError::StartupChannelClosed)??;
    Ok(AssetProcessorRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
        source_watcher: None,
        sweep_owner: None,
    })
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
async fn run_tcp_listener(
    endpoint: Endpoint,
    listener: TcpListener,
    mut shutdown: oneshot::Receiver<()>,
    rpc: AssetProcessorRpc,
) -> Result<(), AssetProcessorRpcTransportError> {
    info!(endpoint = %endpoint.address, "asset-processor RPC listener started");

    let result = loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut shutdown).await {
            Either::Left((Ok((stream, peer)), _)) => {
                info!(endpoint = %endpoint.address, peer = %peer, "asset-processor RPC client connected");
                let bootstrap = rpc.for_connection().into_client();
                drop(az_rpc::spawn_twoparty_server(stream, bootstrap.client));
            }
            Either::Left((Err(error), _)) => break Err(error.into()),
            Either::Right((_, _)) => break Ok(()),
        }
    };
    let service_result = rpc.shutdown_owned_services().await.map_err(Into::into);
    result.and(service_result)
}

fn asset_processor_rpc(
    setup: AssetProcessorServiceSetup,
) -> Result<AssetProcessorRpc, AssetProcessorRpcTransportError> {
    let AssetProcessorServiceSetup {
        database,
        asset_db_writer,
        db_path,
        service_run,
        capability_grants,
        source_coordination,
        startup_scope,
    } = setup;
    let databases = AssetProcessorDatabaseHandles::open(database)?;
    let processor = match startup_scope {
        Some(scope) => AssetProcessor::with_database_handles_and_asset_db_writer(
            databases,
            az_asset_builder::BuildRuleRegistry::new(),
            capability_grants,
            crate::engine_host_registries(),
            Some(scope.workspace_id),
            Some(scope.project_data_paths),
            None,
            asset_db_writer,
        )
        .with_source_roots(scope.source_roots),
        None => AssetProcessor::with_database_handles_and_asset_db_writer(
            databases,
            az_asset_builder::BuildRuleRegistry::new(),
            capability_grants,
            crate::engine_host_registries(),
            None,
            None,
            None,
            asset_db_writer,
        ),
    };
    let rpc = AssetProcessorRpc::new(processor)
        .with_db_path(db_path)
        .with_service_run(service_run)
        .with_source_service_coordination(source_coordination);
    Ok(rpc)
}

#[cfg(any(test, feature = "test-support"))]
fn asset_processor_rpc_for_db_with_builder_registry(
    db_path: PathBuf,
    service_run: Uuid,
    capability_grants: CapabilityGrantSet,
    builders: az_asset_builder::BuildRuleRegistry,
) -> Result<AssetProcessorRpc, AssetProcessorRpcTransportError> {
    let database = AssetDb::open(&db_path)?;
    let view = database
        .workspaces_for_test()?
        .into_iter()
        .next()
        .ok_or(AssetProcessorRpcTransportError::MissingWorkspace)?;
    let workspace_root = PathBuf::from(&view.root);
    let data_home_root = db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("azoth-test-home");
    let project_data_paths =
        AzothDataHome::new(data_home_root).project(&view.project, &workspace_root);
    let processor = AssetProcessor::with_builder_registry_and_catalog(
        database,
        builders,
        capability_grants,
        crate::engine_host_registries(),
        Some(view.workspace_id),
        Some(project_data_paths),
        None,
    );
    Ok(AssetProcessorRpc::new(processor)
        .with_db_path(db_path)
        .with_service_run(service_run))
}

fn asset_processor_rpc_runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    Builder::new_current_thread().enable_all().build()
}

fn run_threaded_local<F>(future: F)
where
    F: std::future::Future<Output = Result<(), AssetProcessorRpcTransportError>> + 'static,
{
    let runtime = match asset_processor_rpc_runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            error!(error = %error, "asset-processor RPC runtime failed to start");
            return;
        }
    };
    let local = LocalSet::new();
    let result = runtime.block_on(local.run_until(future));
    if let Err(error) = result {
        error!(error = %error, "asset-processor RPC listener stopped with error");
    }
}

#[cfg(windows)]
fn start_named_pipe_server(
    setup: AssetProcessorServiceSetup,
    endpoint: &Endpoint,
) -> Result<AssetProcessorRpcServer, AssetProcessorRpcTransportError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<(), std::io::Error>>(1);
    let thread_endpoint = endpoint.clone();
    let endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let server_result = az_rpc::create_owner_only_named_pipe(
                ServerOptions::new().first_pipe_instance(true),
                &thread_endpoint.address,
            );
            let server = match server_result {
                Ok(server) => server,
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return Ok(());
                }
            };
            let _ = ready_tx.send(Ok(()));
            run_named_pipe_listener(setup, thread_endpoint, server, shutdown_rx).await
        });
    });
    ready_rx
        .recv()
        .map_err(|_| AssetProcessorRpcTransportError::StartupChannelClosed)??;
    Ok(AssetProcessorRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
        source_watcher: None,
        sweep_owner: None,
    })
}

#[cfg(not(windows))]
fn start_named_pipe_server(
    _setup: AssetProcessorServiceSetup,
    endpoint: &Endpoint,
) -> Result<AssetProcessorRpcServer, AssetProcessorRpcTransportError> {
    Err(AssetProcessorRpcTransportError::UnsupportedEndpoint(
        endpoint.kind,
    ))
}

// The asset-processor dispatcher is single-threaded by design: this future holds
// `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
// be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
#[allow(clippy::future_not_send)]
#[cfg(windows)]
async fn run_named_pipe_listener(
    setup: AssetProcessorServiceSetup,
    endpoint: Endpoint,
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), AssetProcessorRpcTransportError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let rpc = asset_processor_rpc(setup)?;
    info!(endpoint = %endpoint.address, "asset-processor named-pipe RPC listener started");

    let result = loop {
        let connected = {
            let connect = Box::pin(server.connect());
            match futures::future::select(connect, &mut shutdown).await {
                Either::Left((Ok(()), _)) => true,
                Either::Left((Err(error), _)) => break Err(error.into()),
                Either::Right((_, _)) => break Ok(()),
            }
        };
        if connected {
            let next = match az_rpc::create_owner_only_named_pipe(
                &ServerOptions::new(),
                &endpoint.address,
            ) {
                Ok(next) => next,
                Err(error) => break Err(error.into()),
            };
            let connected = std::mem::replace(&mut server, next);
            let bootstrap = rpc.for_connection().into_client();
            drop(az_rpc::spawn_twoparty_server(connected, bootstrap.client));
        }
    };
    let service_result = rpc.shutdown_owned_services().await.map_err(Into::into);
    drop(rpc);
    result.and(service_result)
}

#[cfg(unix)]
fn start_unix_socket_server(
    setup: AssetProcessorServiceSetup,
    endpoint: &Endpoint,
) -> Result<AssetProcessorRpcServer, AssetProcessorRpcTransportError> {
    let listener = az_rpc::OwnedUnixListener::bind(&endpoint.address)?;
    listener.set_nonblocking(true)?;
    let (listener, socket_lease) = listener.into_parts();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let thread_endpoint = endpoint.clone();
    let endpoint = endpoint.clone();
    let thread = thread::spawn(move || {
        run_threaded_local(async move {
            let listener = tokio::net::UnixListener::from_std(listener)?;
            let result =
                run_unix_socket_listener(setup, thread_endpoint, listener, shutdown_rx).await;
            drop(socket_lease);
            result
        });
    });
    Ok(AssetProcessorRpcServer {
        endpoint,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
        source_watcher: None,
        sweep_owner: None,
    })
}

#[cfg(not(unix))]
fn start_unix_socket_server(
    _setup: AssetProcessorServiceSetup,
    endpoint: &Endpoint,
) -> Result<AssetProcessorRpcServer, AssetProcessorRpcTransportError> {
    Err(AssetProcessorRpcTransportError::UnsupportedEndpoint(
        endpoint.kind,
    ))
}

#[cfg(unix)]
async fn run_unix_socket_listener(
    setup: AssetProcessorServiceSetup,
    endpoint: Endpoint,
    listener: tokio::net::UnixListener,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), AssetProcessorRpcTransportError> {
    let rpc = asset_processor_rpc(setup)?;
    info!(endpoint = %endpoint.address, "asset-processor unix-socket RPC listener started");

    let result = loop {
        let accept = Box::pin(listener.accept());
        match futures::future::select(accept, &mut shutdown).await {
            Either::Left((Ok((stream, _)), _)) => {
                let bootstrap = rpc.for_connection().into_client();
                drop(az_rpc::spawn_twoparty_server(stream, bootstrap.client));
            }
            Either::Left((Err(error), _)) => break Err(error.into()),
            Either::Right((_, _)) => break Ok(()),
        }
    };
    let service_result = rpc.shutdown_owned_services().await.map_err(Into::into);
    result.and(service_result)
}

#[cfg(test)]
mod tests {
    use az_assetdb::{Exclusions, RegisterWorkspace, RegisterWorkspaceRoot};

    use crate::SourceRootRole;
    use uuid::Uuid;

    use super::*;

    const TEST_PROJECT_ID: &str = "local.asset_processor_transport";
    const TEST_WORKTREE_ROOT: &str = "/wt/editor";
    const TEST_BRANCH: &str = "az/session/editor";

    fn registered_workspace() -> (AssetDb, SelectWorkspaces, RegisteredSourceRoot) {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let workspace = writer
            .register_workspace(RegisterWorkspace {
                key: WorkspaceKey {
                    project: TEST_PROJECT_ID.into(),
                    root: TEST_WORKTREE_ROOT.into(),
                    branch: TEST_BRANCH.into(),
                },
                now: 1,
            })
            .wait_blocking()
            .unwrap();
        let key = String::from(PortableKey::project_assets(TEST_PROJECT_ID));
        let (root, policy) = writer
            .register_workspace_root(RegisterWorkspaceRoot {
                workspace_pk: workspace.workspace_id,
                key: key.clone(),
                owner: TEST_PROJECT_ID.into(),
                path: format!("{TEST_WORKTREE_ROOT}/assets"),
                exclusions: Exclusions::default(),
            })
            .wait_blocking()
            .unwrap();
        let registered = RegisteredSourceRoot {
            workspace_pk: workspace.workspace_id,
            workspace_root_pk: policy.workspace_root_id,
            root_pk: root.root_id,
            id: key.clone(),
            owner: TEST_PROJECT_ID.into(),
            path: policy.path,
            display_name: "Project Assets".into(),
            portable_key: key,
            mount: "@assets@".into(),
            recursive: true,
            watch: true,
            writable: true,
            exclusions: policy.exclusions,
            output_prefix: String::new(),
            role: SourceRootRole::ProjectAssets,
        };
        (db, workspace, registered)
    }

    #[test]
    fn service_catalog_asset_processor_grants_match_transport_admission() {
        let descriptor = az_service_catalog::asset_processor_service_descriptor(
            Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        let grants = CapabilityGrantSet::from_grants(
            descriptor
                .capabilities
                .into_iter()
                .filter(|capability| {
                    !az_service_catalog::is_observability_control_grant(capability)
                        && !az_service_catalog::is_service_lifecycle_grant(capability)
                })
                .collect::<Vec<_>>(),
        );

        validate_asset_processor_rpc_capability_grants(&grants).unwrap();
    }

    #[test]
    fn workspace_key_rejects_blank_branch_before_database_open() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("asset.db");
        assert!(matches!(
            validate_asset_processor_launch_context(TEST_PROJECT_ID, TEST_WORKTREE_ROOT, " "),
            Err(AssetProcessorRpcTransportError::InvalidLaunchContext { reason })
                if reason.contains("branch")
        ));
        assert!(!db_path.exists());
    }

    #[test]
    fn production_transport_rejects_malformed_workspace_source_roots_before_serving() {
        let (_, workspace, mut root) = registered_workspace();
        root.owner.clear();
        assert!(matches!(
            validate_registered_workspace_source_roots(&workspace, &[root]),
            Err(AssetProcessorRpcTransportError::InvalidWorkspaceSourceRoot {
                workspace_id,
                reason,
                ..
            }) if workspace_id == workspace.workspace_id && reason.contains("owner id")
        ));
    }

    #[test]
    fn production_transport_requires_project_assets_root_before_serving() {
        let (_, workspace, mut root) = registered_workspace();
        root.portable_key = "gem:local.asset_processor_transport:assets".into();
        assert!(matches!(
            validate_registered_workspace_source_roots(&workspace, &[root]),
            Err(AssetProcessorRpcTransportError::InvalidWorkspace {
                workspace_id,
                reason,
            }) if workspace_id == workspace.workspace_id
                && reason.contains("missing DB-owned project assets root")
        ));
    }

    #[test]
    fn production_transport_rejects_project_assets_root_shape_mismatch_before_serving() {
        let (_, workspace, mut root) = registered_workspace();
        root.path = TEST_WORKTREE_ROOT.into();
        assert!(matches!(
            validate_registered_workspace_source_roots(&workspace, &[root]),
            Err(AssetProcessorRpcTransportError::InvalidWorkspaceSourceRoot {
                workspace_id,
                reason,
                ..
            }) if workspace_id == workspace.workspace_id
                && reason.contains("must be inside workspace root")
        ));
    }

    #[test]
    fn production_transport_requires_registered_workspace_asset_source_roots_before_serving() {
        let (db, workspace, root) = registered_workspace();
        assert!(matches!(
            validate_asset_processor_db_registered_for_workspace(
                &db,
                TEST_PROJECT_ID,
                TEST_WORKTREE_ROOT,
                TEST_BRANCH,
                &[],
            ),
            Err(AssetProcessorRpcTransportError::MissingWorkspaceSourceRoots {
                workspace_id,
            }) if workspace_id == workspace.workspace_id
        ));
        assert_eq!(
            validate_asset_processor_db_registered_for_workspace(
                &db,
                TEST_PROJECT_ID,
                TEST_WORKTREE_ROOT,
                TEST_BRANCH,
                &[root],
            )
            .unwrap(),
            workspace.workspace_id
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_transport_recovers_stale_socket_and_cleans_up_on_stop() {
        use std::os::unix::net::{UnixListener, UnixStream};

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("asset-processor.sock");
        drop(UnixListener::bind(&path).unwrap());
        let (database, _, _) = registered_workspace();
        let asset_db_writer = database.writer().unwrap();
        let capability_grants =
            CapabilityGrantSet::from_grants(vec![az_proto_core::Capability::new(
                az_proto_core::ServiceId::new("azoth", "editor"),
                az_proto_core::ServiceRole::Editor,
            )]);

        let server = start_unix_socket_server(
            AssetProcessorServiceSetup {
                database,
                asset_db_writer,
                db_path: temp.path().join("assetdb.sqlite"),
                service_run: Uuid::now_v7(),
                capability_grants,
                source_coordination: AssetSourceServiceCoordination::default(),
                startup_scope: None,
            },
            &Endpoint::new(EndpointKind::UnixDomainSocket, path.to_string_lossy()),
        )
        .unwrap();

        assert!(UnixStream::connect(&path).is_ok());
        server.stop().unwrap();
        assert!(!path.exists());
    }
}
