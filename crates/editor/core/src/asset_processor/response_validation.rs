use super::{
    ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME, CatalogProductEntry,
    CatalogProductsResult, Component, DEFAULT_ASSET_PRODUCT_PLATFORM, EditorError, EditorResult,
    HashSet, InspectJobSelector, JobActivity, JobAttemptRecord, JobDependencyRecord,
    JobDependencyTarget, JobInspection, JobOwner, JobProductRecord, JobRecord, Path, PathBuf,
    ServiceDescriptor, ServiceId, ServiceRole, SystemTime, UNIX_EPOCH, Uuid, WorkspaceEntry,
    WorkspaceEntryPageResult, WorkspaceRoot, WorkspaceSnapshot,
    validate_descriptor_capability_templates,
};

pub fn asset_browser_session_mismatch(session: &str, reason: &str) -> EditorError {
    EditorError::ServiceDiscovery(format!(
        "session-supervisor status for session `{session}` does not match attached asset browser session: {reason}"
    ))
}

pub fn asset_processor_service_id() -> ServiceId {
    ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME)
}

pub fn service_label(id: &ServiceId) -> String {
    format!("{}/{}", id.namespace, id.name)
}

pub fn ensure_workspace_entry_page_result_matches_request(
    result: &WorkspaceEntryPageResult,
    after_entry_id: Option<i64>,
    page_size: u32,
) -> EditorResult<()> {
    const OPERATION: &str = "workspaceEntryPage";

    if result.entries.len() > page_size as usize {
        return Err(asset_processor_authority_mismatch(
            OPERATION,
            format!(
                "asset-processor returned {} entries for requested page size {page_size}",
                result.entries.len()
            ),
        ));
    }

    let mut seen_entry_ids = HashSet::new();
    let mut previous_entry_id = after_entry_id.unwrap_or(0);
    for entry in &result.entries {
        ensure_workspace_entry_identity(entry, OPERATION)?;
        if entry.entry_id <= previous_entry_id {
            return Err(asset_processor_authority_mismatch(
                OPERATION,
                format!(
                    "asset-processor returned entry {} after cursor {previous_entry_id}",
                    entry.entry_id
                ),
            ));
        }
        if !seen_entry_ids.insert(entry.entry_id) {
            return Err(asset_processor_authority_mismatch(
                OPERATION,
                format!(
                    "asset-processor returned duplicate entry {}",
                    entry.entry_id
                ),
            ));
        }
        previous_entry_id = entry.entry_id;
    }

    if let Some(next_after_entry_id) = result.next_after_entry_id {
        let Some(last) = result.entries.last() else {
            return Err(asset_processor_authority_mismatch(
                OPERATION,
                "asset-processor returned a next-page cursor for an empty page".to_string(),
            ));
        };
        if next_after_entry_id != last.entry_id {
            return Err(asset_processor_authority_mismatch(
                OPERATION,
                format!(
                    "asset-processor next-page cursor {next_after_entry_id} did not match last entry {}",
                    last.entry_id
                ),
            ));
        }
    }

    Ok(())
}

pub fn ensure_workspace_snapshot_identity(snapshot: &WorkspaceSnapshot) -> EditorResult<()> {
    const OPERATION: &str = "workspaceSnapshot";

    if snapshot.workspace_id <= 0 {
        return Err(asset_processor_authority_mismatch(
            OPERATION,
            format!(
                "workspace {} must carry a positive id",
                snapshot.workspace_id
            ),
        ));
    }
    if snapshot.project_id.trim().is_empty()
        || snapshot.workspace_root.trim().is_empty()
        || snapshot.branch.trim().is_empty()
    {
        return Err(asset_processor_authority_mismatch(
            OPERATION,
            format!(
                "workspace {} identity fields cannot be empty",
                snapshot.workspace_id
            ),
        ));
    }
    if snapshot.created_unix_ms < 0
        || snapshot.updated_unix_ms < 0
        || snapshot.updated_unix_ms < snapshot.created_unix_ms
    {
        return Err(asset_processor_authority_mismatch(
            OPERATION,
            format!("workspace {} has invalid timestamps", snapshot.workspace_id),
        ));
    }
    if snapshot.roots.is_empty() {
        return Err(asset_processor_authority_mismatch(
            OPERATION,
            format!("workspace {} must include roots", snapshot.workspace_id),
        ));
    }

    let mut seen_workspace_root_ids = HashSet::new();
    let mut seen_root_ids = HashSet::new();
    let mut seen_portable_keys = HashSet::new();
    for root in &snapshot.roots {
        ensure_workspace_root_matches_snapshot(snapshot, root, OPERATION)?;
        if !seen_workspace_root_ids.insert(root.workspace_root_id) {
            return Err(asset_processor_authority_mismatch(
                OPERATION,
                format!(
                    "workspace {} returned duplicate workspace-root id {}",
                    snapshot.workspace_id, root.workspace_root_id
                ),
            ));
        }
        if !seen_root_ids.insert(root.root_id) {
            return Err(asset_processor_authority_mismatch(
                OPERATION,
                format!(
                    "workspace {} returned duplicate root id {}",
                    snapshot.workspace_id, root.root_id
                ),
            ));
        }
        if !seen_portable_keys.insert(root.portable_key.as_str()) {
            return Err(asset_processor_authority_mismatch(
                OPERATION,
                format!(
                    "workspace {} returned duplicate root key `{}`",
                    snapshot.workspace_id, root.portable_key
                ),
            ));
        }
    }

    Ok(())
}

pub fn ensure_workspace_root_matches_snapshot(
    snapshot: &WorkspaceSnapshot,
    root: &WorkspaceRoot,
    operation: &'static str,
) -> EditorResult<()> {
    if root.workspace_id != snapshot.workspace_id {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!(
                "root `{}` belongs to workspace {}, expected {}",
                root.portable_key, root.workspace_id, snapshot.workspace_id
            ),
        ));
    }
    if root.workspace_root_id <= 0 || root.root_id <= 0 {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!("root `{}` must carry positive ids", root.portable_key),
        ));
    }
    if root.declared_root_id.trim().is_empty()
        || root.owner_id.trim().is_empty()
        || root.source_root.trim().is_empty()
        || root.display_name.trim().is_empty()
        || root.portable_key.trim().is_empty()
        || root.mount.trim().is_empty()
    {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!(
                "root `{}` identity fields cannot be empty",
                root.portable_key
            ),
        ));
    }
    Ok(())
}

pub fn ensure_workspace_entry_identity(
    entry: &WorkspaceEntry,
    operation: &'static str,
) -> EditorResult<()> {
    if entry.entry_id <= 0 || entry.workspace_id <= 0 || entry.root_id <= 0 {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!("entry `{}` must carry positive ids", entry.source_path),
        ));
    }
    if entry.asset_guid.is_nil() {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!("entry `{}` has a nil asset guid", entry.source_path),
        ));
    }
    ensure_asset_relative_path(&entry.source_path, operation, "entry source path")?;
    if !is_64_hex_hash(&entry.content_hash) {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!(
                "entry `{}` content hash must be 64 hexadecimal characters",
                entry.source_path
            ),
        ));
    }
    if entry.diagnostics_count < 0 || entry.updated_unix_ms < 0 {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!(
                "entry `{}` has invalid counters or timestamp",
                entry.source_path
            ),
        ));
    }
    if entry
        .schema_type
        .as_deref()
        .is_some_and(|schema_type| schema_type.trim().is_empty())
    {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!("entry `{}` has an empty schema type", entry.source_path),
        ));
    }

    let mut seen_job_ids = HashSet::new();
    for activity in &entry.jobs {
        ensure_job_activity_matches_entry(activity, entry, operation)?;
        if !seen_job_ids.insert(activity.job.job_id) {
            return Err(asset_processor_authority_mismatch(
                operation,
                format!(
                    "entry {} returned duplicate job {}",
                    entry.entry_id, activity.job.job_id
                ),
            ));
        }
    }

    Ok(())
}

pub fn ensure_job_activity_matches_entry(
    activity: &JobActivity,
    entry: &WorkspaceEntry,
    operation: &'static str,
) -> EditorResult<()> {
    ensure_job_record_identity(&activity.job, operation)?;
    if activity.job.workspace_id != entry.workspace_id
        || activity.job.source_guid != entry.asset_guid
        || activity.job.source_path != entry.source_path
    {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!(
                "job {} does not belong to entry {}",
                activity.job.job_id, entry.entry_id
            ),
        ));
    }
    if let Some(attempt) = &activity.attempt {
        ensure_job_attempt_identity(attempt, operation)?;
        if attempt.job_id != activity.job.job_id {
            return Err(asset_processor_authority_mismatch(
                operation,
                format!(
                    "attempt {} belongs to job {}, expected {}",
                    attempt.attempt_id, attempt.job_id, activity.job.job_id
                ),
            ));
        }
    }
    Ok(())
}

pub fn ensure_job_inspection_matches_selector(
    inspection: &JobInspection,
    selector: &InspectJobSelector,
    operation: &'static str,
) -> EditorResult<()> {
    ensure_job_record_identity(&inspection.job, operation)?;
    match selector {
        InspectJobSelector::Job(job_id) if inspection.job.job_id != *job_id => {
            return Err(asset_processor_authority_mismatch(
                operation,
                format!(
                    "asset-processor returned job {}, expected {job_id}",
                    inspection.job.job_id
                ),
            ));
        }
        InspectJobSelector::Attempt(attempt_id) => {
            let Some(attempt) = &inspection.attempt else {
                return Err(asset_processor_authority_mismatch(
                    operation,
                    format!("asset-processor omitted requested attempt {attempt_id}"),
                ));
            };
            if attempt.attempt_id != *attempt_id {
                return Err(asset_processor_authority_mismatch(
                    operation,
                    format!(
                        "asset-processor returned attempt {}, expected {attempt_id}",
                        attempt.attempt_id
                    ),
                ));
            }
        }
        InspectJobSelector::Job(_) => {}
    }

    if let Some(attempt) = &inspection.attempt {
        ensure_job_attempt_identity(attempt, operation)?;
        if attempt.job_id != inspection.job.job_id {
            return Err(asset_processor_authority_mismatch(
                operation,
                format!(
                    "attempt {} belongs to job {}, expected {}",
                    attempt.attempt_id, attempt.job_id, inspection.job.job_id
                ),
            ));
        }
    }

    ensure_job_products_match_inspection(&inspection.products, inspection.job.job_id, operation)?;
    ensure_job_dependencies_match_inspection(
        &inspection.dependencies,
        inspection.job.job_id,
        operation,
    )
}

pub fn ensure_job_record_identity(job: &JobRecord, operation: &'static str) -> EditorResult<()> {
    if job.job_id <= 0 || job.workspace_id <= 0 || job.attempts < 0 {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!("job {} has invalid ids or attempt count", job.job_id),
        ));
    }
    if job.source_guid.is_nil() {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!("job {} has a nil source guid", job.job_id),
        ));
    }
    ensure_asset_relative_path(&job.source_path, operation, "job source path")?;
    if job.source_root.trim().is_empty()
        || job.key.trim().is_empty()
        || job.platform.trim().is_empty()
        || job.key.trim() != job.key
        || job.platform.trim() != job.platform
    {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!(
                "job {} identity fields cannot be empty or untrimmed",
                job.job_id
            ),
        ));
    }
    if job
        .source_schema_type
        .as_deref()
        .is_some_and(|schema_type| schema_type.trim().is_empty())
    {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!("job {} has an empty source schema type", job.job_id),
        ));
    }
    if let JobOwner::Build(builder_guid) = &job.owner
        && builder_guid.is_nil()
    {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!("job {} has a nil build owner", job.job_id),
        ));
    }
    Ok(())
}

pub fn ensure_job_attempt_identity(
    attempt: &JobAttemptRecord,
    operation: &'static str,
) -> EditorResult<()> {
    if attempt.attempt_id <= 0 || attempt.job_id <= 0 || attempt.ordinal <= 0 {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!("attempt {} has invalid ids or ordinal", attempt.attempt_id),
        ));
    }
    if attempt.error_count < 0 || attempt.warning_count < 0 {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!(
                "attempt {} has negative diagnostic counters",
                attempt.attempt_id
            ),
        ));
    }
    if attempt
        .finished_unix_ms
        .is_some_and(|finished_unix_ms| finished_unix_ms < 0)
    {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!(
                "attempt {} has a negative completion timestamp",
                attempt.attempt_id
            ),
        ));
    }
    for (label, value) in [
        ("owner", attempt.owner.as_deref()),
        ("staging", attempt.staging.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(asset_processor_authority_mismatch(
                operation,
                format!("attempt {} has an empty {label}", attempt.attempt_id),
            ));
        }
    }
    Ok(())
}

pub fn ensure_job_products_match_inspection(
    products: &[JobProductRecord],
    expected_job_id: i64,
    operation: &'static str,
) -> EditorResult<()> {
    let mut seen_product_ids = HashSet::new();
    for product in products {
        ensure_job_product_matches_inspection(product, expected_job_id, operation)?;
        if !seen_product_ids.insert(product.product_id) {
            return Err(asset_processor_authority_mismatch(
                operation,
                format!(
                    "asset-processor returned duplicate product {}",
                    product.product_id
                ),
            ));
        }
    }
    Ok(())
}

pub fn ensure_job_product_matches_inspection(
    product: &JobProductRecord,
    expected_job_id: i64,
    operation: &'static str,
) -> EditorResult<()> {
    if product.product_id <= 0 || product.job_id != expected_job_id {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!(
                "product {} belongs to job {}, expected {expected_job_id}",
                product.product_id, product.job_id
            ),
        ));
    }
    ensure_asset_relative_path(&product.path, operation, "job product path")?;
    if product.asset_type.is_nil()
        || product.sub_id < 0
        || product.product_format.trim().is_empty()
        || !is_64_hex_hash(&product.content_hash)
        || product.byte_length < 0
    {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!(
                "product {} has invalid identity or content",
                product.product_id
            ),
        ));
    }

    let mut seen_edge_ids = HashSet::new();
    let mut seen_targets = HashSet::new();
    for edge in &product.edges {
        if edge.product_edge_id <= 0
            || edge.product_id != product.product_id
            || edge.asset_guid.is_nil()
            || edge.sub_id < 0
            || !seen_edge_ids.insert(edge.product_edge_id)
            || !seen_targets.insert((edge.asset_guid, edge.sub_id))
        {
            return Err(asset_processor_authority_mismatch(
                operation,
                format!(
                    "product {} has an invalid or duplicate edge",
                    product.product_id
                ),
            ));
        }
    }
    Ok(())
}

pub fn ensure_job_dependencies_match_inspection(
    dependencies: &[JobDependencyRecord],
    expected_job_id: i64,
    operation: &'static str,
) -> EditorResult<()> {
    let mut seen_edge_ids = HashSet::new();
    for dependency in dependencies {
        if dependency.job_edge_id <= 0
            || dependency.job_id != expected_job_id
            || dependency.key.trim().is_empty()
            || dependency.platform.trim().is_empty()
            || dependency.key.trim() != dependency.key
            || dependency.platform.trim() != dependency.platform
            || !seen_edge_ids.insert(dependency.job_edge_id)
        {
            return Err(asset_processor_authority_mismatch(
                operation,
                format!(
                    "job dependency {} is invalid or does not belong to job {expected_job_id}",
                    dependency.job_edge_id
                ),
            ));
        }
        match &dependency.target {
            JobDependencyTarget::Guid(asset_guid) if asset_guid.is_nil() => {
                return Err(asset_processor_authority_mismatch(
                    operation,
                    format!(
                        "job dependency {} has a nil target guid",
                        dependency.job_edge_id
                    ),
                ));
            }
            JobDependencyTarget::Path(path) => {
                ensure_asset_relative_path(path, operation, "job dependency path")?;
            }
            JobDependencyTarget::Guid(_) => {}
        }
    }
    Ok(())
}

pub fn ensure_catalog_products_result_matches_request(
    result: &CatalogProductsResult,
    expected_platform: &str,
) -> EditorResult<()> {
    const OPERATION: &str = "catalogProducts";
    let mut seen_product_ids = HashSet::new();
    let mut previous_sort_key: Option<(String, Uuid, i64, i64)> = None;
    for product in &result.entries {
        ensure_catalog_product_matches_request(product, expected_platform)?;
        if !seen_product_ids.insert(product.product_id) {
            return Err(asset_processor_authority_mismatch(
                OPERATION,
                format!(
                    "asset-processor returned duplicate catalog product {}",
                    product.product_id
                ),
            ));
        }
        let sort_key = (
            product.product_path.clone(),
            product.asset_guid,
            product.sub_id,
            product.product_id,
        );
        if previous_sort_key
            .as_ref()
            .is_some_and(|previous| previous > &sort_key)
        {
            return Err(asset_processor_authority_mismatch(
                OPERATION,
                format!(
                    "asset-processor returned catalog product `{}` out of order",
                    product.product_path
                ),
            ));
        }
        previous_sort_key = Some(sort_key);
    }
    Ok(())
}

pub fn ensure_catalog_product_matches_request(
    product: &CatalogProductEntry,
    expected_platform: &str,
) -> EditorResult<()> {
    const OPERATION: &str = "catalogProducts";
    if product.platform != expected_platform {
        return Err(asset_processor_authority_mismatch(
            OPERATION,
            format!(
                "asset-processor returned product {} for platform `{}`, expected `{expected_platform}`",
                product.product_id, product.platform
            ),
        ));
    }
    if product.job_id <= 0
        || product.product_id <= 0
        || product.sub_id < 0
        || product.byte_length < 0
    {
        return Err(asset_processor_authority_mismatch(
            OPERATION,
            format!(
                "catalog product {} has invalid ids or byte length",
                product.product_id
            ),
        ));
    }
    if product.asset_guid.is_nil() || product.builder_guid.is_nil() || product.asset_type.is_nil() {
        return Err(asset_processor_authority_mismatch(
            OPERATION,
            format!("catalog product {} contains a nil UUID", product.product_id),
        ));
    }
    ensure_asset_relative_path(
        &product.source_path,
        OPERATION,
        "catalog product source path",
    )?;
    ensure_asset_relative_path(&product.product_path, OPERATION, "catalog product path")?;
    if product.job_key.trim().is_empty()
        || product.platform.trim().is_empty()
        || product.product_format.trim().is_empty()
        || product.job_key.trim() != product.job_key
        || product.platform.trim() != product.platform
        || !is_64_hex_hash(&product.content_hash)
    {
        return Err(asset_processor_authority_mismatch(
            OPERATION,
            format!(
                "catalog product {} has invalid identity or content",
                product.product_id
            ),
        ));
    }

    let mut aliases = HashSet::new();
    for alias in &product.catalog_aliases {
        if alias.trim().is_empty() || !aliases.insert(alias.as_str()) {
            return Err(asset_processor_authority_mismatch(
                OPERATION,
                format!(
                    "catalog product {} has an empty or duplicate alias",
                    product.product_id
                ),
            ));
        }
    }
    let mut dependencies = HashSet::new();
    for dependency in &product.dependencies {
        if dependency.asset_guid.is_nil()
            || dependency.sub_id < 0
            || !dependencies.insert((dependency.asset_guid, dependency.sub_id))
            || dependency
                .asset_type
                .is_some_and(|asset_type| asset_type.is_nil())
            || dependency
                .hint
                .as_deref()
                .is_some_and(|hint| hint.trim().is_empty())
        {
            return Err(asset_processor_authority_mismatch(
                OPERATION,
                format!(
                    "catalog product {} has an invalid dependency",
                    product.product_id
                ),
            ));
        }
    }
    Ok(())
}

pub fn is_64_hex_hash(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

pub fn ensure_asset_relative_path(
    value: &str,
    operation: &'static str,
    label: &'static str,
) -> EditorResult<()> {
    let path = Path::new(value);
    let mut has_normal_component = false;
    for component in path.components() {
        match component {
            Component::Normal(segment) if !segment.to_string_lossy().trim().is_empty() => {
                has_normal_component = true;
            }
            Component::Normal(_)
            | Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(asset_processor_authority_mismatch(
                    operation,
                    format!("{label} `{value}` must be an asset-relative path"),
                ));
            }
        }
    }
    if !has_normal_component {
        return Err(asset_processor_authority_mismatch(
            operation,
            format!("{label} `{value}` must include at least one path component"),
        ));
    }
    Ok(())
}

pub fn current_unix_ms_i64() -> EditorResult<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| {
            EditorError::InvalidArgument(format!("system time before epoch: {source}"))
        })?;
    i64::try_from(duration.as_millis()).map_err(|source| {
        EditorError::InvalidArgument(format!(
            "system time does not fit in signed milliseconds: {source}"
        ))
    })
}

pub fn normalize_catalog_product_platform(platform: &str) -> String {
    let platform = platform.trim();
    if platform.is_empty() || platform.eq_ignore_ascii_case("all") {
        DEFAULT_ASSET_PRODUCT_PLATFORM.to_owned()
    } else {
        platform.to_owned()
    }
}

pub const fn asset_processor_authority_mismatch(
    operation: &'static str,
    reason: String,
) -> EditorError {
    EditorError::ServiceAuthorityMismatch {
        service: ASSET_PROCESSOR_SERVICE_NAME,
        operation,
        reason,
    }
}

pub fn ensure_asset_browser_root(
    snapshot: &WorkspaceSnapshot,
    root: &WorkspaceRoot,
) -> EditorResult<()> {
    ensure_workspace_root_matches_snapshot(snapshot, root, "workspaceSnapshot")
        .map_err(|error| EditorError::ServiceDiscovery(error.to_string()))?;
    if same_protocol_path(
        Path::new(&root.source_root),
        Path::new(&snapshot.workspace_root),
    ) {
        return Err(EditorError::ServiceDiscovery(format!(
            "workspace snapshot {} included project root `{}`; the asset browser requires asset roots",
            snapshot.workspace_id, root.portable_key
        )));
    }
    Ok(())
}

pub fn same_protocol_path(left: impl AsRef<Path>, right: impl AsRef<Path>) -> bool {
    let left = comparable_protocol_path(left.as_ref());
    let right = comparable_protocol_path(right.as_ref());
    #[cfg(windows)]
    {
        comparable_protocol_path_text(&left) == comparable_protocol_path_text(&right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

pub fn comparable_protocol_path(path: &Path) -> PathBuf {
    let normalized = if path.exists() {
        path.canonicalize().map_or_else(
            |_| cli_compatible_protocol_path(path.to_path_buf()),
            cli_compatible_protocol_path,
        )
    } else {
        cli_compatible_protocol_path(path.to_path_buf())
    };
    strip_trailing_path_separators(normalized)
}

pub fn strip_trailing_path_separators(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    let stripped = text.trim_end_matches(['/', '\\']);
    if stripped.is_empty() {
        path
    } else {
        PathBuf::from(stripped)
    }
}

#[cfg(windows)]
pub fn comparable_protocol_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', r"\")
        .to_ascii_lowercase()
}

#[cfg(windows)]
pub fn cli_compatible_protocol_path(path: PathBuf) -> PathBuf {
    let path_text = path.to_string_lossy();
    if let Some(rest) = path_text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = path_text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

#[cfg(not(windows))]
pub(crate) fn cli_compatible_protocol_path(path: PathBuf) -> PathBuf {
    path
}

pub fn validate_asset_processor_descriptor(descriptor: &ServiceDescriptor) -> EditorResult<()> {
    let expected = ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME);
    if descriptor.id != expected || descriptor.role != ServiceRole::AssetProcessor {
        return Err(EditorError::ServiceDiscovery(format!(
            "expected asset-processor descriptor `{}`/`{}` with role {:?}, got `{}`/`{}` with role {:?}",
            expected.namespace,
            expected.name,
            ServiceRole::AssetProcessor,
            descriptor.id.namespace,
            descriptor.id.name,
            descriptor.role
        )));
    }
    validate_descriptor_capability_templates(descriptor, ASSET_PROCESSOR_SERVICE_NAME)?;
    Ok(())
}

pub fn validate_asset_processor_descriptor_for_session(
    descriptor: &ServiceDescriptor,
    _session_id: Uuid,
) -> EditorResult<()> {
    validate_asset_processor_descriptor(descriptor)
}
