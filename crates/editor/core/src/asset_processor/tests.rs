// test-only module
// Canonical protocol fixtures for editor-side asset projections.
use super::*;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;

use crate::watch_handle::WatchHandle;

use az_asset_processor::test_support::{SyntheticFixture, SyntheticSourceSpec};
use az_filesystem::AzothDataHome;
use az_proto_asset::{
    AssetBuilderCatalogResult, AssetBuilderDescriptor, AssetBuilderPatternDescriptor,
    AssetBuilderPatternKind, AttemptStatus, CatalogPathRegistration, CatalogProductDependency,
    CatalogProductEntry, CatalogProductsRequest, CatalogProductsResult, InspectJobRequest,
    InspectJobResult, InspectJobSelector, JobActivity, JobAttemptRecord, JobDependencyKind,
    JobDependencyRecord, JobDependencyTarget, JobInspection, JobOwner, JobProductEdgeRecord,
    JobProductRecord, JobRecord, JobStatus, SourceAssetRecordResult, SourceDependentJob,
    SourceDependentSource, SourceDependentsResult, SourceFileCreateResult, SourceFileDeleteResult,
    SourceFileMoveResult, SourceFileTemplateDescriptor, SourceFileWorkflowDescriptor,
    SourceRelation, SourceSchemaAuthoring, SourceSchemaDescriptor, WorkspaceEntry,
    WorkspaceEntryDiff, WorkspaceEntryPageRequest, WorkspaceEntryPageResult, WorkspaceRoot,
    WorkspaceSnapshot, WorkspaceSnapshotRequest, WorkspaceSnapshotResult, asset_capnp,
};
use az_proto_core::{Endpoint, EndpointKind};
use az_service_catalog::asset_processor_service_descriptor;
use az_session::{CreateSessionRequest, SessionManager, SessionSupervisorRpc};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::oneshot;
use tokio::task::LocalSet;
use uuid::Uuid;

const SESSION_ID: &str = "018f0c5a-0000-7000-8000-000000000555";

#[derive(Clone)]
struct CanonicalAssetProcessor {
    snapshot: Option<WorkspaceSnapshot>,
    page: WorkspaceEntryPageResult,
    inspection: Option<JobInspection>,
    catalog: CatalogProductsResult,
    builders: AssetBuilderCatalogResult,
    requests: Rc<RefCell<Vec<&'static str>>>,
}

impl asset_capnp::asset_processor::Server for CanonicalAssetProcessor {
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn workspace_snapshot(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::WorkspaceSnapshotParams,
        mut results: asset_capnp::asset_processor::WorkspaceSnapshotResults,
    ) -> Result<(), capnp::Error> {
        self.requests.borrow_mut().push("workspaceSnapshot");
        let _request = WorkspaceSnapshotRequest::from_capnp(params.get()?.get_request()?)?;
        WorkspaceSnapshotResult {
            snapshot: self.snapshot.clone(),
        }
        .to_capnp(results.get().init_result())?;
        Ok(())
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn workspace_entry_page(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::WorkspaceEntryPageParams,
        mut results: asset_capnp::asset_processor::WorkspaceEntryPageResults,
    ) -> Result<(), capnp::Error> {
        self.requests.borrow_mut().push("workspaceEntryPage");
        let request = WorkspaceEntryPageRequest::from_capnp(params.get()?.get_request()?)?;
        let mut page = self.page.clone();
        if let Some(after_entry_id) = request.after_entry_id {
            page.entries.retain(|entry| entry.entry_id > after_entry_id);
            if page.entries.is_empty() {
                page.next_after_entry_id = None;
            }
        }
        page.to_capnp(results.get().init_result())?;
        Ok(())
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn inspect_job(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::InspectJobParams,
        mut results: asset_capnp::asset_processor::InspectJobResults,
    ) -> Result<(), capnp::Error> {
        self.requests.borrow_mut().push("inspectJob");
        let _request = InspectJobRequest::from_capnp(params.get()?.get_request()?)?;
        InspectJobResult {
            inspection: self.inspection.clone(),
        }
        .to_capnp(results.get().init_result())?;
        Ok(())
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn catalog_products(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::CatalogProductsParams,
        mut results: asset_capnp::asset_processor::CatalogProductsResults,
    ) -> Result<(), capnp::Error> {
        self.requests.borrow_mut().push("catalogProducts");
        let _request = CatalogProductsRequest::from_capnp(params.get()?.get_request()?)?;
        self.catalog.clone().to_capnp(results.get().init_result())?;
        Ok(())
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn builder_catalog(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::BuilderCatalogParams,
        mut results: asset_capnp::asset_processor::BuilderCatalogResults,
    ) -> Result<(), capnp::Error> {
        self.requests.borrow_mut().push("builderCatalog");
        let _request = AssetBuilderCatalogRequest::from_capnp(params.get()?.get_request()?)?;
        self.builders
            .clone()
            .to_capnp(results.get().init_result())?;
        Ok(())
    }
}

struct EndpointAssetProcessor {
    snapshot: WorkspaceSnapshot,
    builders: AssetBuilderCatalogResult,
}

impl asset_capnp::asset_processor::Server for EndpointAssetProcessor {
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn workspace_snapshot(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::WorkspaceSnapshotParams,
        mut results: asset_capnp::asset_processor::WorkspaceSnapshotResults,
    ) -> Result<(), capnp::Error> {
        let _request = WorkspaceSnapshotRequest::from_capnp(params.get()?.get_request()?)?;
        WorkspaceSnapshotResult {
            snapshot: Some(self.snapshot.clone()),
        }
        .to_capnp(results.get().init_result())?;
        Ok(())
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn builder_catalog(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::BuilderCatalogParams,
        mut results: asset_capnp::asset_processor::BuilderCatalogResults,
    ) -> Result<(), capnp::Error> {
        let _request = AssetBuilderCatalogRequest::from_capnp(params.get()?.get_request()?)?;
        self.builders
            .clone()
            .to_capnp(results.get().init_result())?;
        Ok(())
    }
}

struct EndpointAssetProcessorServer {
    endpoint: Endpoint,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl EndpointAssetProcessorServer {
    fn start(processor: EndpointAssetProcessor) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = Endpoint::new(
            EndpointKind::Tcp,
            listener.local_addr().unwrap().to_string(),
        );
        let (shutdown, mut shutdown_rx) = oneshot::channel();
        let thread = thread::spawn(move || {
            let runtime = Builder::new_current_thread().enable_io().build().unwrap();
            let local = LocalSet::new();
            runtime.block_on(local.run_until(async move {
                let listener = TcpListener::from_std(listener).unwrap();
                let client: asset_capnp::asset_processor::Client = capnp_rpc::new_client(processor);
                let mut connections = Vec::new();
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let (stream, _) = accepted.unwrap();
                            connections.push(az_rpc::spawn_twoparty_server(
                                stream,
                                client.client.clone(),
                            ));
                        }
                        _ = &mut shutdown_rx => break,
                    }
                }
                for connection in connections {
                    connection.abort();
                }
            }));
        });
        Self {
            endpoint,
            shutdown: Some(shutdown),
            thread: Some(thread),
        }
    }

    fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn editor_read_grant() -> Capability {
    Capability::new(
        ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
        ServiceRole::Editor,
    )
    .with_audience(ASSET_PROCESSOR_AUDIENCE)
    .with_permissions([ASSET_READ_PERMISSION])
    .with_token_hash([0xea, 0xad])
}

fn canonical_client(processor: CanonicalAssetProcessor) -> AssetProcessorClient {
    let client: asset_capnp::asset_processor::Client = capnp_rpc::new_client(processor);
    AssetProcessorClient::new(client)
        .with_session_scope(Uuid::parse_str(SESSION_ID).unwrap())
        .with_capability_templates(vec![editor_read_grant()])
}

fn hash(character: char) -> String {
    character.to_string().repeat(64)
}

fn asset_guid() -> Uuid {
    Uuid::from_bytes([0x11; 16])
}

fn builder_guid() -> Uuid {
    Uuid::from_bytes([0x22; 16])
}

fn asset_type() -> Uuid {
    Uuid::from_bytes([0x33; 16])
}

fn valid_workspace_root() -> WorkspaceRoot {
    WorkspaceRoot {
        workspace_root_id: 8,
        workspace_id: 7,
        root_id: 9,
        declared_root_id: "project.assets".to_string(),
        owner_id: "local.editor".to_string(),
        source_root: "/work/editor/assets".to_string(),
        display_name: "Project Assets".to_string(),
        portable_key: "project:local.editor:assets".to_string(),
        mount: "@assets@".to_string(),
        recursive: true,
        watch: true,
        writable: true,
        output_prefix: String::new(),
        is_root: true,
    }
}

fn valid_workspace_snapshot() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        workspace_id: 7,
        project_id: "local.editor".to_string(),
        workspace_root: "/work/editor".to_string(),
        branch: "az/editor".to_string(),
        created_unix_ms: 10,
        updated_unix_ms: 20,
        roots: vec![valid_workspace_root()],
    }
}

fn valid_job() -> JobRecord {
    JobRecord {
        job_id: 20,
        workspace_id: 7,
        source_guid: asset_guid(),
        source_path: "textures/editor.png".to_string(),
        source_root: "/work/editor/assets".to_string(),
        source_schema_type: Some("texture".to_string()),
        owner: JobOwner::Build(builder_guid()),
        key: "compile-texture".to_string(),
        platform: "pc".to_string(),
        status: JobStatus::Succeeded,
        ready: true,
        attempts: 1,
    }
}

fn valid_job_attempt() -> JobAttemptRecord {
    JobAttemptRecord {
        attempt_id: 12,
        job_id: 20,
        ordinal: 1,
        status: AttemptStatus::Succeeded,
        owner: Some("worker-a".to_string()),
        staging: Some("staging/texture".to_string()),
        finished_unix_ms: Some(30),
        error_count: 0,
        warning_count: 1,
    }
}

fn valid_job_activity() -> JobActivity {
    JobActivity {
        job: valid_job(),
        attempt: Some(valid_job_attempt()),
    }
}

fn valid_workspace_entry(entry_id: i64) -> WorkspaceEntry {
    WorkspaceEntry {
        entry_id,
        workspace_id: 7,
        asset_guid: asset_guid(),
        root_id: 9,
        source_path: "textures/editor.png".to_string(),
        schema_type: Some("texture".to_string()),
        content_hash: hash('a'),
        diff: WorkspaceEntryDiff::Modified,
        diagnostics_count: 0,
        updated_unix_ms: 30,
        jobs: vec![valid_job_activity()],
    }
}

fn valid_job_product() -> JobProductRecord {
    JobProductRecord {
        product_id: 30,
        job_id: 20,
        path: "cache/textures/editor.dds".to_string(),
        asset_type: asset_type(),
        sub_id: 9,
        product_format: "az.test.raw".to_string(),
        product_format_version: 1,
        catalog_path_registration: CatalogPathRegistration::Registered,
        content_hash: hash('b'),
        byte_length: 8_192,
        aliases: vec!["textures/editor.dds".to_string()],
        edges: vec![JobProductEdgeRecord {
            product_edge_id: 31,
            product_id: 30,
            asset_guid: Uuid::from_bytes([0x44; 16]),
            sub_id: 3,
            flags: 1,
        }],
    }
}

fn valid_job_dependency() -> JobDependencyRecord {
    JobDependencyRecord {
        job_edge_id: 40,
        job_id: 20,
        target: JobDependencyTarget::Path("materials/base.ron".to_string()),
        key: "compile-material".to_string(),
        platform: "pc".to_string(),
        kind: JobDependencyKind::Fingerprint,
    }
}

fn valid_job_inspection() -> JobInspection {
    JobInspection {
        job: valid_job(),
        attempt: Some(valid_job_attempt()),
        products: vec![valid_job_product()],
        dependencies: vec![valid_job_dependency()],
    }
}

fn valid_catalog_product() -> CatalogProductEntry {
    CatalogProductEntry {
        job_id: 20,
        product_id: 30,
        asset_guid: asset_guid(),
        source_path: "textures/editor.png".to_string(),
        builder_guid: builder_guid(),
        job_key: "compile-texture".to_string(),
        platform: "pc".to_string(),
        product_path: "cache/textures/editor.dds".to_string(),
        asset_type: asset_type(),
        sub_id: 9,
        product_format: "az.test.raw".to_string(),
        product_format_version: 1,
        content_hash: hash('c'),
        catalog_aliases: vec!["textures/editor.dds".to_string()],
        catalog_path_registration: CatalogPathRegistration::Registered,
        byte_length: 8_192,
        dependencies: vec![CatalogProductDependency {
            asset_guid: Uuid::from_bytes([0x44; 16]),
            sub_id: 3,
            asset_type: Some(asset_type()),
            hint: Some("@assets@/materials/base.material".to_string()),
        }],
    }
}

fn valid_catalog_products() -> CatalogProductsResult {
    CatalogProductsResult {
        entries: vec![valid_catalog_product()],
    }
}

fn valid_source_schema() -> SourceSchemaDescriptor {
    SourceSchemaDescriptor {
        schema_type: "texture".to_string(),
        owner: "local.editor".to_string(),
        label: "Texture".to_string(),
        category: "Rendering".to_string(),
        authoring: SourceSchemaAuthoring::File {
            workflow: SourceFileWorkflowDescriptor {
                source_root: "project:local.editor:assets".to_string(),
                default_path_prefix: "textures".to_string(),
                extensions: vec!["png".to_string()],
                can_create: true,
                can_edit: true,
            },
        },
        file_templates: vec![SourceFileTemplateDescriptor {
            owner: "local.editor".to_string(),
            source_path: "templates/default.png".to_string(),
            label: "Default texture".to_string(),
            description: "A blank texture source".to_string(),
        }],
    }
}

fn valid_builder_catalog() -> AssetBuilderCatalogResult {
    AssetBuilderCatalogResult {
        builders: vec![AssetBuilderDescriptor {
            name: "Texture Builder".to_string(),
            builder_guid: builder_guid(),
            version: 1,
            analysis_fingerprint: hash('d'),
            patterns: vec![AssetBuilderPatternDescriptor {
                kind: AssetBuilderPatternKind::Wildcard,
                pattern: "*.png".to_string(),
            }],
            source_schema_types: vec!["texture".to_string()],
        }],
        source_schemas: vec![valid_source_schema()],
        product_formats: Vec::new(),
    }
}

fn canonical_processor() -> CanonicalAssetProcessor {
    CanonicalAssetProcessor {
        snapshot: Some(valid_workspace_snapshot()),
        page: WorkspaceEntryPageResult {
            entries: vec![valid_workspace_entry(1)],
            next_after_entry_id: None,
        },
        inspection: Some(valid_job_inspection()),
        catalog: valid_catalog_products(),
        builders: valid_builder_catalog(),
        requests: Rc::new(RefCell::new(Vec::new())),
    }
}

fn canonical_processor_with_request_log(
    requests: Rc<RefCell<Vec<&'static str>>>,
) -> CanonicalAssetProcessor {
    let mut processor = canonical_processor();
    processor.requests = requests;
    processor
}

fn endpoint_processor() -> EndpointAssetProcessor {
    EndpointAssetProcessor {
        snapshot: valid_workspace_snapshot(),
        builders: valid_builder_catalog(),
    }
}

fn write_session_project(root: &std::path::Path) {
    let manifest = az_project::ProjectManifest::new("local.editor", "Editor", "0.1.0");
    az_project::write_project_manifest(root, &manifest).unwrap();
    az_project::refresh_project_lock(root).unwrap();
}

fn canonical_browser_controller(
    processor: CanonicalAssetProcessor,
) -> EditorAssetBrowserController {
    EditorAssetBrowserController::new(canonical_client(processor), SESSION_ID)
        .with_attached_workspace_identity("local.editor", "/work/editor", "az/editor", 7)
}

fn valid_source_record(
    source_path: &str,
    schema_type: &str,
    diff: WorkspaceEntryDiff,
) -> SourceAssetRecordResult {
    let mut entry = valid_workspace_entry(50);
    entry.source_path = source_path.to_string();
    entry.schema_type = Some(schema_type.to_string());
    entry.diff = diff;
    SourceAssetRecordResult {
        asset_guid: entry.asset_guid,
        entry,
    }
}

#[test]
fn canonical_rpc_fixture_projects_snapshot_page_job_and_catalog() {
    let controller = canonical_browser_controller(canonical_processor());

    let page = futures::executor::block_on(controller.load_first_page()).unwrap();
    assert_eq!(page.roots.len(), 1);
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].entry_id, 1);
    assert_eq!(page.entries[0].status, AssetBrowserEntryStatus::Modified);
    assert_eq!(
        page.entries[0].latest_job.as_ref().unwrap().attempt_id,
        Some(12)
    );

    let inspection =
        futures::executor::block_on(controller.inspect_job(InspectJobSelector::Attempt(12)))
            .unwrap();
    assert_eq!(inspection.job_id, 20);
    assert_eq!(inspection.attempt_id, Some(12));
    assert_eq!(inspection.products[0].product_id, 30);
    assert_eq!(inspection.dependencies[0].job_edge_id, 40);

    let catalog = futures::executor::block_on(controller.load_catalog_products("pc")).unwrap();
    assert_eq!(catalog.platform, "pc");
    assert_eq!(catalog.entries[0].product_id, 30);

    let builders = futures::executor::block_on(controller.load_builder_catalog()).unwrap();
    assert_eq!(builders.builders[0].name, "Texture Builder");
    assert_eq!(builders.source_schemas[0].schema_type, "texture");
}

#[test]
fn asset_browser_rejects_snapshot_that_does_not_match_attached_identity() {
    let mut processor = canonical_processor();
    processor.snapshot.as_mut().unwrap().branch = "az/other".to_string();

    let error =
        futures::executor::block_on(canonical_browser_controller(processor).load_first_page())
            .unwrap_err();

    assert!(error.to_string().contains("does not match attached branch"));
}

#[test]
fn workspace_snapshot_validation_requires_unique_attached_roots() {
    let mut snapshot = valid_workspace_snapshot();
    snapshot.roots.push(valid_workspace_root());
    assert!(ensure_workspace_snapshot_identity(&snapshot).is_err());

    let mut missing_identity = valid_workspace_snapshot();
    missing_identity.roots[0].declared_root_id.clear();
    assert!(ensure_workspace_snapshot_identity(&missing_identity).is_err());
}

#[test]
fn workspace_entry_page_validation_fences_cursor_and_job_ownership() {
    let mut invalid_cursor = WorkspaceEntryPageResult {
        entries: vec![valid_workspace_entry(2)],
        next_after_entry_id: Some(1),
    };
    assert!(ensure_workspace_entry_page_result_matches_request(&invalid_cursor, None, 32).is_err());

    invalid_cursor.next_after_entry_id = Some(2);
    invalid_cursor.entries[0].jobs[0].job.workspace_id = 8;
    assert!(ensure_workspace_entry_page_result_matches_request(&invalid_cursor, None, 32).is_err());
}

#[test]
fn job_inspection_validation_requires_requested_selector_and_coherent_edges() {
    let mut inspection = valid_job_inspection();
    inspection.attempt.as_mut().unwrap().attempt_id = 13;
    assert!(
        ensure_job_inspection_matches_selector(
            &inspection,
            &InspectJobSelector::Attempt(12),
            "inspectJob",
        )
        .is_err()
    );

    let mut invalid_owner = valid_job_inspection();
    invalid_owner.job.owner = JobOwner::Build(Uuid::nil());
    assert!(
        ensure_job_inspection_matches_selector(
            &invalid_owner,
            &InspectJobSelector::Job(20),
            "inspectJob",
        )
        .is_err()
    );

    let mut invalid_edge = valid_job_inspection();
    invalid_edge.products[0].job_id = 21;
    assert!(
        ensure_job_inspection_matches_selector(
            &invalid_edge,
            &InspectJobSelector::Job(20),
            "inspectJob",
        )
        .is_err()
    );
}

#[test]
fn catalog_products_validation_requires_platform_and_deterministic_identity() {
    let mut wrong_platform = valid_catalog_products();
    wrong_platform.entries[0].platform = "android".to_string();
    assert!(ensure_catalog_products_result_matches_request(&wrong_platform, "pc").is_err());

    let mut duplicate = valid_catalog_products();
    duplicate.entries.push(duplicate.entries[0].clone());
    assert!(ensure_catalog_products_result_matches_request(&duplicate, "pc").is_err());
}

#[test]
fn job_inspection_projection_and_console_keep_job_and_attempt_identity() {
    let inspection = job_inspection_to_ui(valid_job_inspection());
    assert_eq!(inspection.job_id, 20);
    assert_eq!(inspection.attempt_id, Some(12));
    assert_eq!(inspection.ordinal, Some(1));
    assert_eq!(
        inspection.products[0].product_path,
        "cache/textures/editor.dds"
    );
    assert_eq!(inspection.dependencies[0].dependency_kind, "fingerprint");

    let lines = job_inspection_console_lines(&inspection);
    assert!(lines.iter().any(|(_, line)| line.contains("Job 20")));
    assert!(
        lines
            .iter()
            .any(|(_, line)| line.contains("cache/textures/editor.dds"))
    );
}

#[test]
fn workspace_page_and_event_projections_keep_conflicts_and_replace_entries() {
    let mut conflicted = valid_workspace_entry(2);
    conflicted.diff = WorkspaceEntryDiff::Conflicted;
    let status = workspace_entry_page_to_ui(
        SESSION_ID,
        vec![workspace_root_to_ui(&valid_workspace_root())],
        WorkspaceEntryPageResult {
            entries: vec![conflicted.clone()],
            next_after_entry_id: Some(2),
        },
    );
    assert_eq!(
        status.entries[0].status,
        AssetBrowserEntryStatus::Conflicted
    );

    let mut updated = conflicted;
    updated.diff = WorkspaceEntryDiff::Clean;
    let updated_status = apply_asset_processor_event_to_status(
        status,
        SESSION_ID,
        AssetProcessorEvent {
            seq: 4,
            kind: AssetProcessorEventKind::JobCompleted,
            event_unix_ms: 40,
            entry: updated,
        },
    );
    assert_eq!(updated_status.entries.len(), 1);
    assert_eq!(
        updated_status.entries[0].status,
        AssetBrowserEntryStatus::Clean
    );
}

#[test]
fn appended_workspace_pages_deduplicate_entries_and_keep_cursor() {
    let first = workspace_entry_page_to_ui(
        SESSION_ID,
        vec![workspace_root_to_ui(&valid_workspace_root())],
        WorkspaceEntryPageResult {
            entries: vec![valid_workspace_entry(1)],
            next_after_entry_id: Some(1),
        },
    );
    let next = workspace_entry_page_to_ui(
        SESSION_ID,
        Vec::new(),
        WorkspaceEntryPageResult {
            entries: vec![valid_workspace_entry(1), valid_workspace_entry(2)],
            next_after_entry_id: Some(2),
        },
    );

    let appended = append_asset_browser_status(first, next);
    assert_eq!(appended.entries.len(), 2);
    assert_eq!(appended.next_after_entry_id, Some(2));
}

#[test]
fn source_authoring_validations_bind_returned_records_to_the_request() {
    let created = SourceFileCreateResult {
        record: valid_source_record("textures/new.png", "texture", WorkspaceEntryDiff::Added),
    };
    ensure_source_file_create_result_matches_request(&created, "textures/new.png", "texture")
        .unwrap();

    let deleted = SourceFileDeleteResult {
        record: valid_source_record("textures/new.png", "texture", WorkspaceEntryDiff::Deleted),
    };
    ensure_source_file_delete_result_matches_request(&deleted, "textures/new.png").unwrap();

    let moved = SourceFileMoveResult {
        record: valid_source_record("textures/final.png", "texture", WorkspaceEntryDiff::Added),
        old_source_path: "textures/new.png".to_string(),
    };
    ensure_source_file_move_result_matches_request(
        &moved,
        "textures/new.png",
        "textures/final.png",
    )
    .unwrap();

    let reprocessed = ForceReprocessAssetResult {
        record: valid_source_record("textures/final.png", "texture", WorkspaceEntryDiff::Clean),
        enqueued_jobs: 1,
    };
    ensure_force_reprocess_asset_result_matches_request(&reprocessed, "textures/final.png")
        .unwrap();

    let mut wrong = created;
    wrong.record.entry.source_path = "textures/other.png".to_string();
    assert!(
        ensure_source_file_create_result_matches_request(&wrong, "textures/new.png", "texture",)
            .is_err()
    );
}

#[test]
fn source_dependents_validate_and_project_canonical_job_dependents() {
    let result = SourceDependentsResult {
        source_path: "textures/editor.png".to_string(),
        source_dependents: vec![SourceDependentSource {
            source_edge_id: 60,
            source_path: "materials/editor.material.ron".to_string(),
            builder_guid: builder_guid(),
            relation: SourceRelation::SourceToSource,
        }],
        job_dependents: vec![SourceDependentJob {
            job_edge_id: 61,
            job_id: 20,
            latest_attempt_id: Some(12),
            source_path: "materials/editor.material.ron".to_string(),
            owner: JobOwner::Build(builder_guid()),
            job_key: "compile-material".to_string(),
            platform: "pc".to_string(),
            dependency_job_key: "compile-texture".to_string(),
            dependency_platform: "pc".to_string(),
            kind: JobDependencyKind::Fingerprint,
            product_paths: vec!["cache/materials/editor.material".to_string()],
        }],
    };
    ensure_source_dependents_result_matches_request(&result, "textures/editor.png").unwrap();

    let preview = source_dependents_to_ui(SESSION_ID, "project:local.editor:assets", result);
    assert_eq!(preview.source_dependents[0].relation, "source-to-source");
    assert_eq!(preview.job_dependents[0].job_id, 20);
    assert_eq!(
        preview.job_dependents[0].owner,
        format!("build:{}", builder_guid())
    );

    let mut shortened = preview;
    assert_eq!(shortened.total_dependents(), 2);
    shortened.job_dependents.clear();
    assert_eq!(shortened.total_dependents(), 1);
}

#[test]
fn source_dependents_reject_invalid_build_owner_and_product_path() {
    let mut result = SourceDependentsResult {
        source_path: "textures/editor.png".to_string(),
        source_dependents: Vec::new(),
        job_dependents: vec![SourceDependentJob {
            job_edge_id: 61,
            job_id: 20,
            latest_attempt_id: None,
            source_path: "materials/editor.material.ron".to_string(),
            owner: JobOwner::Build(Uuid::nil()),
            job_key: "compile-material".to_string(),
            platform: "pc".to_string(),
            dependency_job_key: "compile-texture".to_string(),
            dependency_platform: "pc".to_string(),
            kind: JobDependencyKind::Fingerprint,
            product_paths: Vec::new(),
        }],
    };
    assert!(
        ensure_source_dependents_result_matches_request(&result, "textures/editor.png").is_err()
    );

    result.job_dependents[0].owner = JobOwner::Plan;
    result.job_dependents[0].product_paths = vec!["../escape.product".to_string()];
    assert!(
        ensure_source_dependents_result_matches_request(&result, "textures/editor.png").is_err()
    );
}

#[test]
fn builder_catalog_validation_and_projection_keep_published_file_workflows() {
    let catalog = valid_builder_catalog();
    ensure_asset_builder_catalog_result_matches_request(&catalog).unwrap();
    let projected = asset_builder_catalog_to_ui(catalog.clone());
    assert_eq!(projected.builders[0].source_schema_types, ["texture"]);
    assert!(matches!(
        projected.source_schemas[0].authoring,
        AssetSourceSchemaAuthoringData::File { .. }
    ));

    let mut duplicate = catalog;
    duplicate
        .source_schemas
        .push(duplicate.source_schemas[0].clone());
    assert!(ensure_asset_builder_catalog_result_matches_request(&duplicate).is_err());
}

#[test]
fn builder_catalog_rejects_each_builder_schema_and_template_invariant() {
    let mut duplicate_builder = valid_builder_catalog();
    duplicate_builder
        .builders
        .push(duplicate_builder.builders[0].clone());
    assert!(ensure_asset_builder_catalog_result_matches_request(&duplicate_builder).is_err());

    let mut empty_pattern = valid_builder_catalog();
    empty_pattern.builders[0].patterns[0].pattern.clear();
    assert!(ensure_asset_builder_catalog_result_matches_request(&empty_pattern).is_err());

    let mut repeated_filter = valid_builder_catalog();
    repeated_filter.builders[0]
        .source_schema_types
        .push("texture".to_string());
    assert!(ensure_asset_builder_catalog_result_matches_request(&repeated_filter).is_err());

    let mut unpublished_filter = valid_builder_catalog();
    unpublished_filter.builders[0].source_schema_types = vec!["unpublished".to_string()];
    assert!(ensure_asset_builder_catalog_result_matches_request(&unpublished_filter).is_err());

    let mut non_creatable_template = valid_builder_catalog();
    let SourceSchemaAuthoring::File { workflow } =
        &mut non_creatable_template.source_schemas[0].authoring
    else {
        panic!("fixture must be file authored");
    };
    workflow.can_create = false;
    assert!(ensure_asset_builder_catalog_result_matches_request(&non_creatable_template).is_err());

    let mut document_template = valid_builder_catalog();
    document_template.source_schemas[0].authoring = SourceSchemaAuthoring::ProjectDocument {
        schema_type: "project.texture".to_string(),
    };
    assert!(ensure_asset_builder_catalog_result_matches_request(&document_template).is_err());
}

#[test]
fn source_creation_requires_published_workflow_and_canonical_path() {
    let catalog = asset_builder_catalog_to_ui(valid_builder_catalog());
    ensure_asset_source_file_workflow_is_published(
        &catalog,
        "project:local.editor:assets",
        "textures/new.png",
        "texture",
    )
    .unwrap();

    assert!(
        ensure_asset_source_file_workflow_is_published(
            &catalog,
            "project:other:assets",
            "textures/new.png",
            "texture",
        )
        .is_err()
    );
    assert!(
        ensure_asset_source_file_workflow_is_published(
            &catalog,
            "project:local.editor:assets",
            "../escape.png",
            "texture",
        )
        .is_err()
    );
    assert!(
        ensure_asset_source_file_workflow_is_published(
            &catalog,
            "project:local.editor:assets",
            "textures/new.ron",
            "texture",
        )
        .is_err()
    );
}

#[test]
fn asset_browser_pages_read_attached_snapshot_and_page_from_one_client() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let controller =
        canonical_browser_controller(canonical_processor_with_request_log(Rc::clone(&requests)));

    futures::executor::block_on(controller.load_first_page()).unwrap();
    futures::executor::block_on(controller.load_page(Some(1))).unwrap();

    assert_eq!(
        requests.borrow().as_slice(),
        [
            "workspaceSnapshot",
            "workspaceEntryPage",
            "workspaceSnapshot",
            "workspaceEntryPage",
        ]
    );
}

#[test]
fn asset_browser_pages_reject_a_snapshot_outside_the_attached_workspace() {
    let controller =
        EditorAssetBrowserController::new(canonical_client(canonical_processor()), SESSION_ID)
            .with_attached_workspace_identity("local.editor", "/work/editor", "az/editor", 8);

    let error = futures::executor::block_on(controller.load_page(None)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not match attached workspace")
    );
}

#[test]
fn asset_browser_rejects_the_workspace_directory_as_an_asset_root() {
    let snapshot = valid_workspace_snapshot();
    let mut root = valid_workspace_root();
    root.source_root = snapshot.workspace_root.clone();

    let error = ensure_asset_browser_root(&snapshot, &root).unwrap_err();
    assert!(error.to_string().contains("requires asset roots"));
}

#[test]
fn asset_browser_install_uses_the_event_pump_without_a_first_page_or_reconcile() {
    let source = include_str!("install.rs");
    let install = source
        .split("pub(crate) fn install_asset_browser_slot(")
        .nth(1)
        .expect("find asset browser install")
        .split("pub(crate) fn spawn_asset_processor_event_pump(")
        .next()
        .expect("find event pump after install");
    assert!(
        install.contains("spawn_asset_processor_event_pump"),
        "install must retain the canonical asset-processor event pump"
    );
    assert!(
        install.contains("awaiting asset processor event subscription"),
        "install must publish a waiting health state until the event stream answers"
    );
    assert!(
        !install.contains("load_first_page().await"),
        "install must not await the initial asset browser page"
    );
    assert!(
        !install.contains("reconcile_asset_sources"),
        "install must not force a source reconcile"
    );
}

/// Stands in for the subscription thread without an IPC event stream: it parks
/// on the same [`WatchHandle`] edge the real subscription selects against and
/// reports its own exit, so retirement is observable rather than inferred.
fn probe_asset_processor_subscription(exited: mpsc::Sender<()>) -> AssetProcessorEventSubscription {
    let (shutdown, shutdown_rx) = WatchHandle::channel();
    let thread = thread::Builder::new()
        .name("asset-events-cancellation-probe".to_string())
        .spawn(move || {
            let _ = futures::executor::block_on(shutdown_rx);
            let _ = exited.send(());
        })
        .expect("spawn probe subscription thread");
    AssetProcessorEventSubscription::new(shutdown, thread)
}

#[test]
fn reinstalling_the_asset_browser_stream_cancels_the_prior_subscription_thread() {
    let mut owner = EditorAssetProcessorEventStreamOwner::default();
    let (exited, exit) = mpsc::channel();
    let installed = owner.begin_install(SESSION_ID.to_string());
    assert!(
        owner
            .retain_subscription(installed, probe_asset_processor_subscription(exited))
            .is_ok(),
        "the current stream must retain its subscription"
    );
    assert_eq!(
        exit.try_recv(),
        Err(mpsc::TryRecvError::Empty),
        "a retained subscription must stay alive while its stream is current"
    );

    let reinstalled = owner.begin_install(SESSION_ID.to_string());
    assert_ne!(reinstalled, installed);

    // Retiring the prior stream signalled and joined its thread, so the exit
    // is already recorded: no wait, bounded or otherwise, can be required.
    assert_eq!(
        exit.try_recv(),
        Ok(()),
        "reinstalling the asset browser stream must cancel the prior subscription thread"
    );
}

#[test]
fn finishing_the_asset_browser_stream_cancels_its_subscription_thread() {
    let mut owner = EditorAssetProcessorEventStreamOwner::default();
    let (exited, exit) = mpsc::channel();
    let installed = owner.begin_install(SESSION_ID.to_string());
    assert!(
        owner
            .retain_subscription(installed, probe_asset_processor_subscription(exited))
            .is_ok(),
        "the current stream must retain its subscription"
    );

    assert!(owner.finish(installed));

    assert_eq!(
        exit.try_recv(),
        Ok(()),
        "a terminal stream must cancel its subscription thread before it stops accepting events"
    );
}

#[test]
fn asset_processor_health_projection_and_event_stream_fences_preserve_incremental_refresh() {
    let mut health = ServiceHealth::ready(
        ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
        ServiceRole::AssetProcessor,
        Uuid::from_bytes([0x70; 16]),
        az_proto_core::ProtocolVersion::CURRENT,
    );
    health.checked_unix_ms = 100;
    health.uptime_ms = 200;
    health.last_event_seq = 3;
    let ready = asset_processor_activity_to_ui(SESSION_ID, health.clone());
    assert!(ready.ready);
    assert!(!ready.busy());

    health.state = ServiceHealthState::Busy;
    health.active_operation = "reconcile".to_string();
    let busy = asset_processor_activity_to_ui(SESSION_ID, health);
    assert!(busy.ready);
    assert!(busy.busy());
    assert_eq!(busy.operation, "reconcile");

    let mut streams = EditorAssetProcessorEventStreamOwner::default();
    let first = streams.begin_install(SESSION_ID.to_string());
    assert!(streams.admit_initial(first, 3));
    let cursor = streams.cursor(SESSION_ID).unwrap();
    assert!(matches!(
        streams.admit_event(first, 4),
        AssetProcessorEventAdmission::Apply {
            missed_events: false
        }
    ));
    assert_eq!(
        streams.snapshot_admission(SESSION_ID, Some(cursor)),
        AssetProcessorSnapshotAdmission::Stale
    );
    assert_eq!(
        streams.admit_event(first, 4),
        AssetProcessorEventAdmission::Ignore
    );
    assert!(matches!(
        streams.admit_event(first, 6),
        AssetProcessorEventAdmission::Apply {
            missed_events: true
        }
    ));

    let second = streams.begin_install(SESSION_ID.to_string());
    assert_eq!(
        streams.admit_event(first, 5),
        AssetProcessorEventAdmission::Superseded
    );
    assert!(streams.admit_initial(second, 5));
}

#[test]
fn asset_events_ignore_other_sessions_and_outside_attached_roots() {
    let current = EditorAssetBrowserStatus::new(
        SESSION_ID,
        vec![workspace_root_to_ui(&valid_workspace_root())],
        vec![workspace_entry_to_ui(valid_workspace_entry(1))],
        None,
    );
    let other_session = apply_asset_processor_event_to_status(
        current.clone(),
        "other-session",
        AssetProcessorEvent {
            seq: 2,
            kind: AssetProcessorEventKind::SourceRecorded,
            event_unix_ms: 31,
            entry: valid_workspace_entry(2),
        },
    );
    assert_eq!(other_session, current);

    let mut outside_root = valid_workspace_entry(3);
    outside_root.root_id = 99;
    let ignored = apply_asset_processor_event_to_status(
        current.clone(),
        SESSION_ID,
        AssetProcessorEvent {
            seq: 3,
            kind: AssetProcessorEventKind::SourceRecorded,
            event_unix_ms: 32,
            entry: outside_root,
        },
    );
    assert_eq!(ignored, current);
}

#[test]
fn asset_browser_errors_and_session_replacement_do_not_leak_prior_status() {
    let current = workspace_entry_page_to_ui(
        SESSION_ID,
        vec![workspace_root_to_ui(&valid_workspace_root())],
        WorkspaceEntryPageResult {
            entries: vec![valid_workspace_entry(1)],
            next_after_entry_id: Some(1),
        },
    )
    .with_error("stale read");
    let replacement = EditorAssetBrowserStatus::new(
        "next-session",
        Vec::new(),
        vec![workspace_entry_to_ui(valid_workspace_entry(2))],
        None,
    );

    assert_eq!(
        append_asset_browser_status(current, replacement.clone()),
        replacement
    );
}

#[test]
fn asset_processor_connects_over_a_real_endpoint_transport() {
    let server = EndpointAssetProcessorServer::start(endpoint_processor());
    let descriptor = asset_processor_service_descriptor(Uuid::now_v7(), server.endpoint.clone());
    let runtime = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .unwrap();
    let local = LocalSet::new();

    let snapshot = local
        .block_on(&runtime, async {
            let client = AssetProcessorClient::connect(&descriptor).await.unwrap();
            client
                .workspace_snapshot(AssetRootScope::All)
                .await
                .unwrap()
        })
        .unwrap();
    assert_eq!(snapshot.workspace_id, 7);
    server.stop();
}

#[test]
fn asset_browser_reads_supervisor_status_before_resolving_the_asset_processor() {
    let source = include_str!("browser_controller.rs");
    let helper = source
        .split("async fn asset_processor_descriptor_from_supervisor(")
        .nth(1)
        .expect("find asset processor supervisor helper")
        .split("fn ensure_status_matches_attached_identity(")
        .next()
        .expect("find status validator after supervisor helper");
    let status = helper
        .find("supervisor.status(session_slug).await?")
        .expect("read status before resolving");
    let resolve = helper
        .find(".resolve_asset_processor_descriptor(session_slug)")
        .expect("resolve current descriptor");
    assert!(status < resolve);
}

#[test]
fn asset_browser_refreshes_its_client_from_the_session_supervisor() {
    let project = tempfile::tempdir().unwrap();
    write_session_project(project.path());
    let manager = SessionManager::with_data_home(
        project.path(),
        AzothDataHome::new(project.path().join("azoth-test-home")),
    )
    .unwrap();
    let manifest = manager
        .create_session(CreateSessionRequest::new("Asset Browser Refresh"))
        .unwrap();
    let server = EndpointAssetProcessorServer::start(endpoint_processor());
    let descriptor = asset_processor_service_descriptor(Uuid::now_v7(), server.endpoint.clone());
    manager
        .attach_project_service_descriptors(&manifest.slug, &[descriptor])
        .unwrap();
    let supervisor_rpc = Rc::new(SessionSupervisorRpc::test_with_manager(manager));
    let supervisor =
        SessionSupervisorClient::new(SessionSupervisorRpc::client_from_rc(&supervisor_rpc))
            .with_session_scope(manifest.id.0);
    let controller = EditorAssetBrowserController::new(
        canonical_client(canonical_processor()),
        manifest.id.to_string(),
    )
    .with_session_supervisor(supervisor, manifest.slug, manifest.id.0);

    let runtime = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .unwrap();
    let local = LocalSet::new();
    let catalog = local.block_on(&runtime, async {
        let refreshed = controller.refresh_client_from_supervisor().await.unwrap();
        refreshed.load_builder_catalog().await.unwrap()
    });
    assert_eq!(catalog.builders[0].name, "Texture Builder");
    server.stop();
}

#[test]
fn asset_processor_client_uses_only_descriptor_capability_templates() {
    let client = canonical_client(canonical_processor());
    assert_eq!(
        client.editor_read_capability().unwrap().token_hash,
        vec![0xea, 0xad]
    );

    let client: asset_capnp::asset_processor::Client = capnp_rpc::new_client(canonical_processor());
    let missing = AssetProcessorClient::new(client).with_capability_templates(Vec::new());
    assert!(missing.editor_read_capability().is_err());
}

#[test]
fn editor_projects_an_attached_snapshot_from_the_shipping_asset_processor_fixture() {
    let workspace = tempfile::tempdir().unwrap();
    let fixture = SyntheticFixture::build(
        workspace.path(),
        "local.editor",
        [SyntheticSourceSpec::new("synthetic/fixture.ron", b"()")],
    )
    .unwrap();
    let client = AssetProcessorClient::new(fixture.client())
        .with_capability_templates(vec![fixture.read_capability()]);
    let controller = EditorAssetBrowserController::new(client, SESSION_ID)
        .with_attached_workspace_identity(
            "local.editor",
            workspace.path(),
            "synthetic",
            fixture.snapshot.workspace_id,
        );

    let status = futures::executor::block_on(controller.load_first_page()).unwrap();
    let expected_asset_guid = fixture.sources[0].guid.to_string();
    let recorded_source_asset_count = fixture.reconcile.recorded_source_asset_count;
    drop(fixture);

    assert_eq!(status.roots.len(), 1);
    assert_eq!(status.entries.len(), 1);
    assert_eq!(status.entries[0].source_path, "synthetic/fixture.ron");
    assert_eq!(status.entries[0].asset_guid, expected_asset_guid);
    assert_eq!(recorded_source_asset_count, 1);
}

#[gpui::test]
fn animation_catalog_subscription_observes_later_asset_browser_publication(
    cx: &gpui::TestAppContext,
) {
    let mut changes = cx.update(subscribe_animation_catalog_input_invalidation);
    assert_eq!(*changes.borrow_and_update(), 0);

    cx.update(|app| {
        publish_asset_browser_status(
            app,
            EditorAssetBrowserStatus::new("session-a", Vec::new(), Vec::new(), None),
        );
    });

    assert!(matches!(changes.has_changed(), Ok(true)));
    assert_eq!(*changes.borrow_and_update(), 1);
}
