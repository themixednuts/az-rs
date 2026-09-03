use super::{
    ASSET_PROCESSOR_SERVICE_NAME, AssetBrowserEntryData, AssetBrowserEntryStatus,
    AssetBrowserJobData, AssetBrowserJobStatus, AssetBuilderCatalogResult, AssetBuilderData,
    AssetBuilderDescriptor, AssetBuilderPatternData, AssetBuilderPatternDescriptor,
    AssetBuilderPatternKind, AssetBuilderPatternKindData, AssetProcessorEvent,
    AssetProcessorEventKind, AssetSourceDependentJobData, AssetSourceDependentSourceData,
    AssetSourceFileTemplateData, AssetSourceFileWorkflowData, AssetSourceSchemaAuthoringData,
    AssetSourceSchemaData, AttemptStatus, CatalogProductData, CatalogProductEntry,
    CatalogProductsResult, EditorAssetBrowserStatus, EditorAssetBuilderCatalog,
    EditorAssetProcessorActivity, EditorAssetSourceDependentsPreview, EditorCatalogProductsStatus,
    EditorJobInspection, HashSet, JobActivity, JobDependencyData, JobDependencyKind,
    JobDependencyRecord, JobDependencyTarget, JobInspection, JobOwner, JobProductData,
    JobProductRecord, JobStatus, LogLevel, ServiceHealth, ServiceHealthState,
    ServiceHealthStateData, SourceDependentJob, SourceDependentSource, SourceDependentsResult,
    SourceSchemaAuthoring, SourceSchemaDescriptor, WorkspaceEntry, WorkspaceEntryPageResult,
    WorkspaceRoot, WorkspaceRootData, WorkspaceSnapshot, publish_asset_browser_status,
    publish_console_log,
};

pub(crate) fn publish_asset_processor_event_update(
    cx: &gpui::AsyncApp,
    fence: crate::controller_set::ControllerFence,
    browser_owner_id: &str,
    event: AssetProcessorEvent,
) {
    let (log_level, log_source, log_message) = asset_processor_event_output(&event);
    let browser_owner_id = browser_owner_id.to_owned();
    let () = cx.update(move |cx| {
        if !crate::controller_set::is_current_fence(cx, fence) {
            return;
        }
        publish_console_log(cx, log_level, log_source, log_message);
        let status = match cx.try_global::<EditorAssetBrowserStatus>().cloned() {
            Some(current) => {
                apply_asset_processor_event_to_status(current, &browser_owner_id, event)
            }
            None => asset_processor_event_to_status(browser_owner_id, event),
        };
        publish_asset_browser_status(cx, status);
        cx.refresh_windows();
    });
}

pub(crate) fn asset_processor_event_output(
    event: &AssetProcessorEvent,
) -> (LogLevel, String, String) {
    let entry = &event.entry;
    let latest_activity = entry.jobs.last();
    let level = latest_activity.map_or_else(
        || {
            if entry.diagnostics_count > 0 {
                LogLevel::Warn
            } else {
                LogLevel::Info
            }
        },
        |activity| {
            let (status, error_count, warning_count) = job_activity_status(activity);
            match status {
                AssetBrowserJobStatus::Failed | AssetBrowserJobStatus::Abandoned => LogLevel::Error,
                AssetBrowserJobStatus::Queued
                | AssetBrowserJobStatus::Leased
                | AssetBrowserJobStatus::Succeeded => {
                    if error_count > 0 {
                        LogLevel::Error
                    } else if warning_count > 0 || entry.diagnostics_count > 0 {
                        LogLevel::Warn
                    } else {
                        LogLevel::Info
                    }
                }
            }
        },
    );
    let message = match (event.kind, latest_activity) {
        (AssetProcessorEventKind::JobCompleted, Some(activity)) => {
            let (status, error_count, warning_count) = job_activity_status(activity);
            let attempt_label = activity.attempt.as_ref().map_or_else(
                || "no attempt".to_string(),
                |attempt| format!("attempt {}", attempt.ordinal),
            );
            format!(
                "job {}: {} [{}:{} {}, {} errors, {} warnings]",
                status.label(),
                entry.source_path,
                activity.job.key,
                activity.job.platform,
                attempt_label,
                error_count,
                warning_count
            )
        }
        _ => format!(
            "{}: {}",
            asset_processor_event_kind_label(event.kind),
            entry.source_path
        ),
    };
    (level, ASSET_PROCESSOR_SERVICE_NAME.to_owned(), message)
}

const fn asset_processor_event_kind_label(kind: AssetProcessorEventKind) -> &'static str {
    match kind {
        AssetProcessorEventKind::SourceRecorded => "source recorded",
        AssetProcessorEventKind::SourceCreated => "source created",
        AssetProcessorEventKind::SourceDeleted => "source deleted",
        AssetProcessorEventKind::SourceMoved => "source moved",
        AssetProcessorEventKind::SourceReprocessed => "source reprocessed",
        AssetProcessorEventKind::SourceReconciled => "source reconciled",
        AssetProcessorEventKind::JobCompleted => "job completed",
    }
}

#[must_use]
pub fn apply_asset_processor_event_to_status(
    mut current: EditorAssetBrowserStatus,
    browser_owner_id: &str,
    event: AssetProcessorEvent,
) -> EditorAssetBrowserStatus {
    if current.session_id != browser_owner_id {
        return current;
    }

    let entry = workspace_entry_to_ui(event.entry);
    if !current.roots.is_empty()
        && !current
            .roots
            .iter()
            .any(|root| root.root_id == entry.root_id)
    {
        return current;
    }

    if let Some(index) = current
        .entries
        .iter()
        .position(|existing| existing.entry_id == entry.entry_id)
    {
        current.entries[index] = entry;
    } else {
        let index = current
            .entries
            .partition_point(|existing| existing.entry_id < entry.entry_id);
        current.entries.insert(index, entry);
    }
    current.status_error = None;
    current
}

pub(crate) fn asset_processor_event_to_status(
    browser_owner_id: impl Into<String>,
    event: AssetProcessorEvent,
) -> EditorAssetBrowserStatus {
    EditorAssetBrowserStatus::new(
        browser_owner_id,
        Vec::new(),
        vec![workspace_entry_to_ui(event.entry)],
        None,
    )
}

#[must_use]
pub fn workspace_entry_page_to_ui(
    session_id: impl Into<String>,
    roots: Vec<WorkspaceRootData>,
    result: WorkspaceEntryPageResult,
) -> EditorAssetBrowserStatus {
    EditorAssetBrowserStatus::new(
        session_id,
        roots,
        result
            .entries
            .into_iter()
            .map(workspace_entry_to_ui)
            .collect(),
        result.next_after_entry_id,
    )
}

#[must_use]
pub fn asset_builder_catalog_to_ui(
    catalog: AssetBuilderCatalogResult,
) -> EditorAssetBuilderCatalog {
    EditorAssetBuilderCatalog::new(
        catalog
            .builders
            .into_iter()
            .map(asset_builder_to_ui)
            .collect(),
        catalog
            .source_schemas
            .into_iter()
            .map(asset_source_schema_to_ui)
            .collect(),
    )
}

#[must_use]
pub fn catalog_products_to_ui(
    session_id: impl Into<String>,
    platform: impl Into<String>,
    result: CatalogProductsResult,
) -> EditorCatalogProductsStatus {
    EditorCatalogProductsStatus::new(
        session_id,
        platform,
        result
            .entries
            .into_iter()
            .map(catalog_product_to_ui)
            .collect(),
    )
}

#[must_use]
pub fn source_dependents_to_ui(
    session_id: impl Into<String>,
    source_root: impl Into<String>,
    result: SourceDependentsResult,
) -> EditorAssetSourceDependentsPreview {
    EditorAssetSourceDependentsPreview {
        session_id: session_id.into(),
        source_root: source_root.into(),
        source_path: result.source_path,
        source_dependents: result
            .source_dependents
            .into_iter()
            .map(source_dependent_source_to_ui)
            .collect(),
        job_dependents: result
            .job_dependents
            .into_iter()
            .map(source_dependent_job_to_ui)
            .collect(),
        loading: false,
        error: None,
    }
}

#[must_use]
pub fn append_asset_browser_status(
    mut current: EditorAssetBrowserStatus,
    next: EditorAssetBrowserStatus,
) -> EditorAssetBrowserStatus {
    if current.session_id != next.session_id {
        return next;
    }
    if !next.roots.is_empty() {
        current.roots = next.roots;
    }

    let mut seen = HashSet::with_capacity(current.entries.len() + next.entries.len());
    current.entries.retain(|entry| seen.insert(entry.entry_id));
    current.entries.extend(
        next.entries
            .into_iter()
            .filter(|entry| seen.insert(entry.entry_id)),
    );
    current.next_after_entry_id = next.next_after_entry_id;
    current.status_error = next.status_error;
    current
}

pub(crate) fn workspace_roots_to_ui(snapshot: &WorkspaceSnapshot) -> Vec<WorkspaceRootData> {
    snapshot.roots.iter().map(workspace_root_to_ui).collect()
}

pub(crate) fn workspace_root_to_ui(root: &WorkspaceRoot) -> WorkspaceRootData {
    WorkspaceRootData {
        workspace_root_id: root.workspace_root_id,
        root_id: root.root_id,
        declared_root_id: root.declared_root_id.clone(),
        owner_id: root.owner_id.clone(),
        source_root: root.source_root.clone(),
        display_name: root.display_name.clone(),
        portable_key: root.portable_key.clone(),
        output_prefix: root.output_prefix.clone(),
    }
}

pub(crate) fn asset_builder_to_ui(builder: AssetBuilderDescriptor) -> AssetBuilderData {
    AssetBuilderData {
        name: builder.name,
        builder_guid: builder.builder_guid.to_string(),
        version: builder.version,
        source_schema_types: builder.source_schema_types,
        patterns: builder
            .patterns
            .into_iter()
            .map(asset_builder_pattern_to_ui)
            .collect(),
    }
}

pub(crate) fn asset_builder_pattern_to_ui(
    pattern: AssetBuilderPatternDescriptor,
) -> AssetBuilderPatternData {
    AssetBuilderPatternData {
        kind: asset_builder_pattern_kind_to_ui(pattern.kind),
        pattern: pattern.pattern,
    }
}

const fn asset_builder_pattern_kind_to_ui(
    kind: AssetBuilderPatternKind,
) -> AssetBuilderPatternKindData {
    match kind {
        AssetBuilderPatternKind::Wildcard => AssetBuilderPatternKindData::Wildcard,
        AssetBuilderPatternKind::Regex => AssetBuilderPatternKindData::Regex,
    }
}

pub(crate) fn asset_source_schema_to_ui(
    source_schema: SourceSchemaDescriptor,
) -> AssetSourceSchemaData {
    AssetSourceSchemaData {
        schema_type: source_schema.schema_type,
        owner: source_schema.owner,
        label: source_schema.label,
        category: source_schema.category,
        authoring: asset_source_schema_authoring_to_ui(source_schema.authoring),
        file_templates: source_schema
            .file_templates
            .into_iter()
            .map(|template| AssetSourceFileTemplateData {
                owner: template.owner,
                source_path: template.source_path,
                label: template.label,
                description: template.description,
            })
            .collect(),
    }
}

pub(crate) fn asset_source_schema_authoring_to_ui(
    authoring: SourceSchemaAuthoring,
) -> AssetSourceSchemaAuthoringData {
    match authoring {
        SourceSchemaAuthoring::File { workflow } => AssetSourceSchemaAuthoringData::File {
            workflow: AssetSourceFileWorkflowData {
                source_root: workflow.source_root,
                default_path_prefix: workflow.default_path_prefix,
                extensions: workflow.extensions,
                can_create: workflow.can_create,
                can_edit: workflow.can_edit,
            },
        },
        SourceSchemaAuthoring::ProjectDocument { schema_type } => {
            AssetSourceSchemaAuthoringData::ProjectDocument { schema_type }
        }
    }
}

pub(crate) fn catalog_product_to_ui(product: CatalogProductEntry) -> CatalogProductData {
    CatalogProductData {
        job_id: product.job_id,
        product_id: product.product_id,
        asset_guid: product.asset_guid.to_string(),
        source_path: product.source_path,
        builder_guid: product.builder_guid.to_string(),
        job_key: product.job_key,
        platform: product.platform,
        product_path: product.product_path,
        asset_type: product.asset_type.to_string(),
        sub_id: product.sub_id,
        product_format: product.product_format,
        product_format_version: product.product_format_version,
        content_hash: product.content_hash,
        byte_length: product.byte_length,
    }
}

pub(crate) fn source_dependent_source_to_ui(
    dependent: SourceDependentSource,
) -> AssetSourceDependentSourceData {
    AssetSourceDependentSourceData {
        source_path: dependent.source_path,
        builder_guid: dependent.builder_guid.to_string(),
        relation: source_relation_label(dependent.relation).to_string(),
    }
}

pub(crate) fn source_dependent_job_to_ui(
    dependent: SourceDependentJob,
) -> AssetSourceDependentJobData {
    AssetSourceDependentJobData {
        job_edge_id: dependent.job_edge_id,
        job_id: dependent.job_id,
        latest_attempt_id: dependent.latest_attempt_id,
        source_path: dependent.source_path,
        owner: match dependent.owner {
            JobOwner::Plan => "plan".to_string(),
            JobOwner::Build(builder_guid) => format!("build:{builder_guid}"),
        },
        job_key: dependent.job_key,
        platform: dependent.platform,
        dependency_job_key: dependent.dependency_job_key,
        dependency_platform: dependent.dependency_platform,
        dependency_kind: dependent.kind as i64,
        product_paths: dependent.product_paths,
    }
}

pub(crate) fn workspace_entry_to_ui(entry: WorkspaceEntry) -> AssetBrowserEntryData {
    AssetBrowserEntryData {
        entry_id: entry.entry_id,
        workspace_id: entry.workspace_id,
        asset_guid: entry.asset_guid.to_string(),
        root_id: entry.root_id,
        source_path: entry.source_path,
        schema_type: entry.schema_type,
        content_hash: entry.content_hash,
        status: workspace_entry_diff_to_ui(entry.diff),
        diagnostics_count: entry.diagnostics_count,
        latest_job: entry.jobs.into_iter().last().map(job_activity_to_ui),
    }
}

pub(crate) fn job_activity_to_ui(activity: JobActivity) -> AssetBrowserJobData {
    let (status, error_count, warning_count) = job_activity_status(&activity);
    AssetBrowserJobData {
        job_id: activity.job.job_id,
        attempt_id: activity.attempt.as_ref().map(|attempt| attempt.attempt_id),
        job_key: activity.job.key,
        platform: activity.job.platform,
        ordinal: activity.attempt.as_ref().map(|attempt| attempt.ordinal),
        status,
        error_count,
        warning_count,
    }
}

#[must_use]
pub fn job_inspection_to_ui(inspection: JobInspection) -> EditorJobInspection {
    let job_id = inspection.job.job_id;
    let attempt_id = inspection
        .attempt
        .as_ref()
        .map(|attempt| attempt.attempt_id);
    let ordinal = inspection.attempt.as_ref().map(|attempt| attempt.ordinal);
    let status = inspection.attempt.as_ref().map_or_else(
        || job_status_to_ui(inspection.job.status),
        |attempt| attempt_status_to_ui(attempt.status),
    );
    EditorJobInspection::new(
        job_id,
        attempt_id,
        inspection.job.source_path,
        inspection.job.key,
        inspection.job.platform,
        ordinal,
        status,
        inspection
            .dependencies
            .into_iter()
            .map(job_dependency_to_ui)
            .collect(),
        inspection
            .products
            .into_iter()
            .map(job_product_to_ui)
            .collect(),
    )
}

pub(crate) fn job_product_to_ui(product: JobProductRecord) -> JobProductData {
    JobProductData {
        product_id: product.product_id,
        product_path: product.path,
        asset_type: product.asset_type.to_string(),
        sub_id: product.sub_id,
        product_format: product.product_format,
        product_format_version: product.product_format_version,
        content_hash: product.content_hash,
        byte_length: product.byte_length,
    }
}

pub(crate) fn job_dependency_to_ui(dependency: JobDependencyRecord) -> JobDependencyData {
    JobDependencyData {
        job_edge_id: dependency.job_edge_id,
        target: match dependency.target {
            JobDependencyTarget::Guid(asset_guid) => format!("uuid:{asset_guid}"),
            JobDependencyTarget::Path(path) => path,
        },
        job_key: dependency.key,
        platform: dependency.platform,
        dependency_kind: job_dependency_kind_label(dependency.kind).to_string(),
    }
}

#[must_use]
pub fn job_inspection_console_lines(inspection: &EditorJobInspection) -> Vec<(LogLevel, String)> {
    let mut lines =
        Vec::with_capacity(3 + inspection.dependencies.len() + inspection.products.len());
    match (
        inspection.job_key.as_deref(),
        inspection.platform.as_deref(),
        inspection.ordinal,
        inspection.status,
    ) {
        (Some(job_key), Some(platform), Some(ordinal), Some(status)) => lines.push((
            LogLevel::Info,
            format!(
                "Job {}: {}:{} #{} {}",
                inspection.job_id,
                job_key,
                platform,
                ordinal,
                status.label()
            ),
        )),
        _ => lines.push((LogLevel::Info, format!("Job {}", inspection.job_id))),
    }

    if let Some(error) = &inspection.status_error {
        lines.push((LogLevel::Error, error.clone()));
        return lines;
    }
    if inspection.dependencies.is_empty() {
        lines.push((LogLevel::Info, "No dependencies recorded".to_string()));
    } else {
        lines.extend(inspection.dependencies.iter().map(|dependency| {
            (
                LogLevel::Info,
                format!(
                    "dependency: {} {}:{} {}",
                    dependency.target,
                    dependency.job_key,
                    dependency.platform,
                    dependency.dependency_kind
                ),
            )
        }));
    }
    if inspection.products.is_empty() {
        lines.push((LogLevel::Info, "No products recorded".to_string()));
    } else {
        lines.extend(inspection.products.iter().map(|product| {
            (
                LogLevel::Info,
                format!(
                    "product: {} sub {} {} bytes {}",
                    product.product_path, product.sub_id, product.byte_length, product.asset_type
                ),
            )
        }));
    }
    lines
}

fn job_activity_status(activity: &JobActivity) -> (AssetBrowserJobStatus, i64, i64) {
    activity.attempt.as_ref().map_or_else(
        || (job_status_to_ui(activity.job.status), 0, 0),
        |attempt| {
            (
                attempt_status_to_ui(attempt.status),
                attempt.error_count,
                attempt.warning_count,
            )
        },
    )
}

const fn workspace_entry_diff_to_ui(
    diff: az_proto_asset::WorkspaceEntryDiff,
) -> AssetBrowserEntryStatus {
    match diff {
        az_proto_asset::WorkspaceEntryDiff::Clean => AssetBrowserEntryStatus::Clean,
        az_proto_asset::WorkspaceEntryDiff::Added => AssetBrowserEntryStatus::Added,
        az_proto_asset::WorkspaceEntryDiff::Modified => AssetBrowserEntryStatus::Modified,
        az_proto_asset::WorkspaceEntryDiff::Deleted => AssetBrowserEntryStatus::Deleted,
        az_proto_asset::WorkspaceEntryDiff::Conflicted => AssetBrowserEntryStatus::Conflicted,
    }
}

const fn job_status_to_ui(status: JobStatus) -> AssetBrowserJobStatus {
    match status {
        JobStatus::Queued => AssetBrowserJobStatus::Queued,
        JobStatus::Leased => AssetBrowserJobStatus::Leased,
        JobStatus::Succeeded => AssetBrowserJobStatus::Succeeded,
        JobStatus::Failed => AssetBrowserJobStatus::Failed,
    }
}

const fn attempt_status_to_ui(status: AttemptStatus) -> AssetBrowserJobStatus {
    match status {
        AttemptStatus::Leased => AssetBrowserJobStatus::Leased,
        AttemptStatus::Succeeded => AssetBrowserJobStatus::Succeeded,
        AttemptStatus::Failed => AssetBrowserJobStatus::Failed,
        AttemptStatus::Abandoned => AssetBrowserJobStatus::Abandoned,
    }
}

const fn job_dependency_kind_label(kind: JobDependencyKind) -> &'static str {
    match kind {
        JobDependencyKind::Order => "order",
        JobDependencyKind::Fingerprint => "fingerprint",
        JobDependencyKind::OrderOnly => "order-only",
    }
}

const fn source_relation_label(relation: az_proto_asset::SourceRelation) -> &'static str {
    match relation {
        az_proto_asset::SourceRelation::SourceToSource => "source-to-source",
        az_proto_asset::SourceRelation::JobToJob => "job-to-job",
        az_proto_asset::SourceRelation::SourceLikeMatch => "source-like-match",
    }
}

pub(crate) fn asset_processor_activity_to_ui(
    session_id: &str,
    health: ServiceHealth,
) -> EditorAssetProcessorActivity {
    let state = health_state_to_ui(health.state);
    let message = if health.message.trim().is_empty() {
        match health.state {
            ServiceHealthState::Busy => "asset processor is busy",
            ServiceHealthState::Degraded => "asset processor is degraded",
            ServiceHealthState::Failed => "asset processor failed",
            ServiceHealthState::Starting => "asset processor is starting",
            ServiceHealthState::Stopping => "asset processor is stopping",
            ServiceHealthState::Ready => "asset processor ready",
            ServiceHealthState::Unknown => "asset processor state unknown",
        }
        .to_owned()
    } else {
        health.message
    };

    let mut activity =
        EditorAssetProcessorActivity::new(session_id, state, health.active_operation, message)
            .with_transport(
                health.checked_unix_ms,
                health.uptime_ms,
                health.last_event_seq,
            );
    // The service reports readiness and degradation independently of its
    // state; keep its answer rather than the state's default.
    activity.ready = health.ready;
    activity.degraded = health.degraded;
    activity
}

const fn health_state_to_ui(state: ServiceHealthState) -> ServiceHealthStateData {
    match state {
        ServiceHealthState::Unknown => ServiceHealthStateData::Unknown,
        ServiceHealthState::Starting => ServiceHealthStateData::Starting,
        ServiceHealthState::Ready => ServiceHealthStateData::Ready,
        ServiceHealthState::Busy => ServiceHealthStateData::Busy,
        ServiceHealthState::Degraded => ServiceHealthStateData::Degraded,
        ServiceHealthState::Stopping => ServiceHealthStateData::Stopping,
        ServiceHealthState::Failed => ServiceHealthStateData::Failed,
    }
}
