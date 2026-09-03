//! Response validation for source-file workflows, builder catalogs, and
//! reflected schema descriptors.

use super::{
    AssetBuilderCatalogResult, AssetBuilderDescriptor, EditorResult, ForceReprocessAssetResult,
    HashSet, JobOwner, SourceDependentsResult, SourceFileCreateResult, SourceFileDeleteResult,
    SourceFileMoveResult, SourceSchemaAuthoring, SourceSchemaDescriptor, Uuid,
    asset_processor_authority_mismatch, ensure_asset_relative_path, is_64_hex_hash,
};
pub fn ensure_source_file_create_result_matches_request(
    result: &SourceFileCreateResult,
    source_path: &str,
    schema_type: &str,
) -> EditorResult<()> {
    if result.record.asset_guid == Uuid::nil() {
        return Err(asset_processor_authority_mismatch(
            "createSourceFile",
            "asset-processor returned a nil source asset guid".to_string(),
        ));
    }
    if result.record.entry.source_path != source_path {
        return Err(asset_processor_authority_mismatch(
            "createSourceFile",
            format!(
                "asset-processor returned source path `{}` for requested `{source_path}`",
                result.record.entry.source_path
            ),
        ));
    }
    if result.record.entry.schema_type.as_deref() != Some(schema_type) {
        return Err(asset_processor_authority_mismatch(
            "createSourceFile",
            format!(
                "asset-processor returned schema type `{:?}` for requested `{schema_type}`",
                result.record.entry.schema_type
            ),
        ));
    }
    if result.record.entry.entry_id <= 0
        || result.record.entry.workspace_id <= 0
        || result.record.entry.root_id <= 0
        || result.record.entry.asset_guid.is_nil()
    {
        return Err(asset_processor_authority_mismatch(
            "createSourceFile",
            "asset-processor returned a source record without valid identities".to_string(),
        ));
    }
    if !is_64_hex_hash(&result.record.entry.content_hash) {
        return Err(asset_processor_authority_mismatch(
            "createSourceFile",
            format!(
                "asset-processor returned invalid content hash `{}`",
                result.record.entry.content_hash
            ),
        ));
    }
    Ok(())
}

pub fn ensure_source_file_delete_result_matches_request(
    result: &SourceFileDeleteResult,
    source_path: &str,
) -> EditorResult<()> {
    if result.record.entry.source_path != source_path {
        return Err(asset_processor_authority_mismatch(
            "deleteSourceFile",
            format!(
                "asset-processor returned source path `{}` for requested `{source_path}`",
                result.record.entry.source_path
            ),
        ));
    }
    if result.record.entry.diff != az_proto_asset::WorkspaceEntryDiff::Deleted {
        return Err(asset_processor_authority_mismatch(
            "deleteSourceFile",
            format!(
                "asset-processor returned status {:?} for deleted source `{source_path}`",
                result.record.entry.diff
            ),
        ));
    }
    Ok(())
}

pub fn ensure_source_file_move_result_matches_request(
    result: &SourceFileMoveResult,
    from_source_path: &str,
    to_source_path: &str,
) -> EditorResult<()> {
    if result.old_source_path != from_source_path {
        return Err(asset_processor_authority_mismatch(
            "moveSourceFile",
            format!(
                "asset-processor returned old source path `{}` for requested `{from_source_path}`",
                result.old_source_path
            ),
        ));
    }
    if result.record.entry.source_path != to_source_path {
        return Err(asset_processor_authority_mismatch(
            "moveSourceFile",
            format!(
                "asset-processor returned source path `{}` for requested `{to_source_path}`",
                result.record.entry.source_path
            ),
        ));
    }
    if result.record.asset_guid == Uuid::nil() {
        return Err(asset_processor_authority_mismatch(
            "moveSourceFile",
            "asset-processor returned a nil source asset guid".to_string(),
        ));
    }
    Ok(())
}

pub fn ensure_force_reprocess_asset_result_matches_request(
    result: &ForceReprocessAssetResult,
    source_path: &str,
) -> EditorResult<()> {
    if result.record.entry.source_path != source_path {
        return Err(asset_processor_authority_mismatch(
            "forceReprocessAsset",
            format!(
                "asset-processor returned source path `{}` for requested `{source_path}`",
                result.record.entry.source_path
            ),
        ));
    }
    if result.record.asset_guid == Uuid::nil() {
        return Err(asset_processor_authority_mismatch(
            "forceReprocessAsset",
            "asset-processor returned a nil source asset guid".to_string(),
        ));
    }
    if result.enqueued_jobs == 0 {
        return Err(asset_processor_authority_mismatch(
            "forceReprocessAsset",
            format!("asset-processor queued no jobs for `{source_path}`"),
        ));
    }
    Ok(())
}

pub fn ensure_source_dependents_result_matches_request(
    result: &SourceDependentsResult,
    source_path: &str,
) -> EditorResult<()> {
    if result.source_path != source_path {
        return Err(asset_processor_authority_mismatch(
            "sourceDependents",
            format!(
                "asset-processor returned dependents for `{}`, expected `{source_path}`",
                result.source_path
            ),
        ));
    }
    for dependent in &result.source_dependents {
        ensure_asset_relative_path(
            &dependent.source_path,
            "sourceDependents",
            "dependent source path",
        )?;
        if dependent.source_edge_id <= 0 || dependent.builder_guid == Uuid::nil() {
            return Err(asset_processor_authority_mismatch(
                "sourceDependents",
                format!(
                    "dependent source `{}` returned an invalid identity",
                    dependent.source_path
                ),
            ));
        }
    }
    for dependent in &result.job_dependents {
        ensure_asset_relative_path(
            &dependent.source_path,
            "sourceDependents",
            "dependent job source path",
        )?;
        if dependent.job_edge_id <= 0
            || dependent.job_id <= 0
            || dependent
                .latest_attempt_id
                .is_some_and(|attempt_id| attempt_id <= 0)
        {
            return Err(asset_processor_authority_mismatch(
                "sourceDependents",
                format!(
                    "dependent job `{}` returned invalid identities",
                    dependent.source_path
                ),
            ));
        }
        if dependent.job_key.trim().is_empty()
            || dependent.platform.trim().is_empty()
            || dependent.dependency_job_key.trim().is_empty()
            || dependent.dependency_platform.trim().is_empty()
        {
            return Err(asset_processor_authority_mismatch(
                "sourceDependents",
                format!(
                    "dependent job `{}` returned empty job identity fields",
                    dependent.source_path
                ),
            ));
        }
        if matches!(dependent.owner, JobOwner::Build(builder_guid) if builder_guid.is_nil()) {
            return Err(asset_processor_authority_mismatch(
                "sourceDependents",
                format!(
                    "dependent job `{}` returned a nil build owner",
                    dependent.source_path
                ),
            ));
        }
        for product_path in &dependent.product_paths {
            ensure_asset_relative_path(
                product_path,
                "sourceDependents",
                "dependent job product path",
            )?;
        }
    }
    Ok(())
}

pub fn ensure_asset_builder_catalog_result_matches_request(
    result: &AssetBuilderCatalogResult,
) -> EditorResult<()> {
    let mut seen_builder_guids = HashSet::new();
    for builder in &result.builders {
        ensure_asset_builder_matches_request(builder)?;
        if !seen_builder_guids.insert(builder.builder_guid) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "asset-processor returned duplicate builder guid {}",
                    builder.builder_guid
                ),
            ));
        }
    }
    let mut seen_source_schemas = HashSet::new();
    for source_schema in &result.source_schemas {
        ensure_asset_source_schema_matches_request(source_schema)?;
        if !seen_source_schemas.insert(source_schema.schema_type.as_str()) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "asset-processor returned duplicate source schema type {}",
                    source_schema.schema_type
                ),
            ));
        }
    }
    for builder in &result.builders {
        for source_schema_type in &builder.source_schema_types {
            if !seen_source_schemas.contains(source_schema_type.as_str()) {
                return Err(asset_processor_authority_mismatch(
                    "builderCatalog",
                    format!(
                        "asset builder `{}` references source schema type `{}` without a source schema descriptor",
                        builder.name, source_schema_type
                    ),
                ));
            }
        }
    }
    Ok(())
}

pub fn ensure_asset_builder_matches_request(builder: &AssetBuilderDescriptor) -> EditorResult<()> {
    if builder.name.trim().is_empty() {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            "asset builder name cannot be empty".to_string(),
        ));
    }

    if builder.builder_guid == Uuid::nil() {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!("asset builder `{}` guid cannot be nil", builder.name),
        ));
    }

    if builder.version == 0 {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!("asset builder `{}` version must be positive", builder.name),
        ));
    }

    if builder.patterns.is_empty() {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "asset builder `{}` must declare at least one pattern",
                builder.name
            ),
        ));
    }

    for pattern in &builder.patterns {
        if pattern.pattern.trim().is_empty() {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "asset builder `{}` contains an empty {:?} pattern",
                    builder.name, pattern.kind
                ),
            ));
        }
    }

    let mut seen_source_schema_types = HashSet::new();
    for source_schema_type in &builder.source_schema_types {
        if source_schema_type.trim().is_empty() {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "asset builder `{}` contains an empty source schema type",
                    builder.name
                ),
            ));
        }
        if !seen_source_schema_types.insert(source_schema_type.as_str()) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "asset builder `{}` contains duplicate source schema type `{}`",
                    builder.name, source_schema_type
                ),
            ));
        }
    }

    Ok(())
}

pub fn ensure_asset_source_schema_matches_request(
    source_schema: &SourceSchemaDescriptor,
) -> EditorResult<()> {
    if source_schema.schema_type.trim().is_empty() {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            "source schema type cannot be empty".to_string(),
        ));
    }

    if source_schema.owner.trim() != source_schema.owner {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "source schema `{}` owner must not have leading or trailing whitespace",
                source_schema.schema_type
            ),
        ));
    }

    if source_schema.label.trim() != source_schema.label {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "source schema `{}` label must not have leading or trailing whitespace",
                source_schema.schema_type
            ),
        ));
    }

    if source_schema.category.trim() != source_schema.category {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "source schema `{}` category must not have leading or trailing whitespace",
                source_schema.schema_type
            ),
        ));
    }

    match &source_schema.authoring {
        SourceSchemaAuthoring::File { workflow } => {
            if workflow.extensions.is_empty() {
                return Err(asset_processor_authority_mismatch(
                    "builderCatalog",
                    format!(
                        "source schema `{}` file workflow has no extensions",
                        source_schema.schema_type
                    ),
                ));
            }
        }
        SourceSchemaAuthoring::ProjectDocument { schema_type } => {
            if schema_type.trim().is_empty() {
                return Err(asset_processor_authority_mismatch(
                    "builderCatalog",
                    format!(
                        "source schema `{}` has empty project document schema type",
                        source_schema.schema_type
                    ),
                ));
            }
        }
    }

    ensure_asset_source_file_templates_match_request(source_schema)
}

pub fn ensure_asset_source_file_templates_match_request(
    source_schema: &SourceSchemaDescriptor,
) -> EditorResult<()> {
    if !source_schema.file_templates.is_empty() {
        match &source_schema.authoring {
            SourceSchemaAuthoring::File { workflow } if workflow.can_create => {}
            SourceSchemaAuthoring::File { .. } => {
                return Err(asset_processor_authority_mismatch(
                    "builderCatalog",
                    format!(
                        "source schema `{}` exposes file templates but is not default-creatable",
                        source_schema.schema_type
                    ),
                ));
            }
            SourceSchemaAuthoring::ProjectDocument { .. } => {
                return Err(asset_processor_authority_mismatch(
                    "builderCatalog",
                    format!(
                        "source schema `{}` exposes file templates but is project-document backed",
                        source_schema.schema_type
                    ),
                ));
            }
        }
    }

    let mut seen_paths = HashSet::new();
    for template in &source_schema.file_templates {
        if template.owner.trim() != template.owner
            || template.label.trim() != template.label
            || template.description.trim() != template.description
        {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "source schema `{}` file template display metadata must be trimmed",
                    source_schema.schema_type
                ),
            ));
        }
        if !asset_source_template_path_is_safe(&template.source_path) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "source schema `{}` file template has invalid source path `{}`",
                    source_schema.schema_type, template.source_path
                ),
            ));
        }
        if !seen_paths.insert(template.source_path.as_str()) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "source schema `{}` repeats file template source path `{}`",
                    source_schema.schema_type, template.source_path
                ),
            ));
        }
    }
    Ok(())
}

pub fn asset_source_template_path_is_safe(path: &str) -> bool {
    !path.trim().is_empty()
        && path.trim() == path
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}
