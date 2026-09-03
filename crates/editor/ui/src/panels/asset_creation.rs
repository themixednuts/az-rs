//! Shared projections and validation for creating file-backed asset sources.

use std::fmt;

use super::{
    AssetSourceFileWorkflowData, AssetSourceSchemaAuthoringData, AssetSourceSchemaData,
    EditorAssetBuilderCatalog,
};

/// UI-ready file-backed source schema that can create new source files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatableAssetSourceData {
    pub schema_type: String,
    pub label: String,
    pub category: String,
    pub owner: String,
    pub source_root: String,
    pub default_path_prefix: String,
    pub extensions: Vec<String>,
}

impl CreatableAssetSourceData {
    #[must_use]
    pub fn extension_hint(&self) -> String {
        extension_hint(&self.extensions)
    }
}

/// Neutral request shape that maps directly to `CreateAssetSourceFile`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetSourceCreateRequestData {
    pub schema_type: String,
    pub source_root: String,
    pub source_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetSourceCreateRequestError {
    EmptyName,
    InvalidTargetFolder { value: String },
    InvalidSourcePath { value: String },
}

impl fmt::Display for AssetSourceCreateRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(f, "enter an asset name"),
            Self::InvalidTargetFolder { value } => {
                write!(
                    f,
                    "target folder `{value}` must be a portable relative path"
                )
            }
            Self::InvalidSourcePath { value } => {
                write!(f, "source path `{value}` must be a portable relative path")
            }
        }
    }
}

impl std::error::Error for AssetSourceCreateRequestError {}

#[must_use]
pub fn creatable_asset_sources(
    catalog: &EditorAssetBuilderCatalog,
) -> Vec<CreatableAssetSourceData> {
    let mut sources = catalog
        .source_schemas
        .iter()
        .filter_map(creatable_asset_source_from_schema)
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.schema_type.cmp(&right.schema_type))
    });
    sources
}

#[must_use]
pub fn creatable_asset_source_from_schema(
    schema: &AssetSourceSchemaData,
) -> Option<CreatableAssetSourceData> {
    let AssetSourceSchemaAuthoringData::File { workflow } = &schema.authoring else {
        return None;
    };
    if !workflow.can_create {
        return None;
    }
    Some(CreatableAssetSourceData {
        schema_type: schema.schema_type.clone(),
        label: asset_source_schema_label(schema),
        category: non_empty_or(&schema.category, "Assets"),
        owner: schema.owner.clone(),
        source_root: workflow.source_root.clone(),
        default_path_prefix: workflow.default_path_prefix.clone(),
        extensions: workflow.extensions.clone(),
    })
}

/// Builds the typed create request for one asset source file.
///
/// # Errors
///
/// Returns any error [`build_asset_source_path`] returns:
/// [`AssetSourceCreateRequestError::EmptyName`] if `name` is blank,
/// [`AssetSourceCreateRequestError::InvalidTargetFolder`] if `target_folder`
/// is not a portable relative path, or
/// [`AssetSourceCreateRequestError::InvalidSourcePath`] if the assembled path
/// is not one.
pub fn build_asset_source_create_request(
    source: &CreatableAssetSourceData,
    target_folder: &str,
    name: &str,
) -> Result<AssetSourceCreateRequestData, AssetSourceCreateRequestError> {
    let source_path = build_asset_source_path(target_folder, name, &source.extensions)?;
    Ok(AssetSourceCreateRequestData {
        schema_type: source.schema_type.clone(),
        source_root: source.source_root.clone(),
        source_path,
    })
}

#[must_use]
pub fn default_target_folder_for_source(
    current_folder: &str,
    workflow: &AssetSourceFileWorkflowData,
) -> String {
    let folder = current_folder_label_to_path(current_folder);
    if folder.is_empty() {
        return workflow.default_path_prefix.trim_matches('/').to_string();
    }
    folder
}

#[must_use]
pub fn current_folder_label_to_path(label: &str) -> String {
    let trimmed = label.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("assets")
        || trimmed.eq_ignore_ascii_case("all")
        || trimmed.eq_ignore_ascii_case("source roots")
    {
        return String::new();
    }

    trimmed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else if ch == '/' || ch == '-' || ch == '_' || ch.is_ascii_whitespace() {
                '/'
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Assembles the AssetDB-relative source path for a new asset source file.
///
/// # Errors
///
/// Returns [`AssetSourceCreateRequestError::EmptyName`] if `name` is empty
/// after trimming, [`AssetSourceCreateRequestError::InvalidTargetFolder`] if
/// `target_folder` is non-empty but not a portable AssetDB-relative path, or
/// [`AssetSourceCreateRequestError::InvalidSourcePath`] if the assembled path
/// (folder, name and enforced extension) is not one.
pub fn build_asset_source_path(
    target_folder: &str,
    name: &str,
    extensions: &[String],
) -> Result<String, AssetSourceCreateRequestError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AssetSourceCreateRequestError::EmptyName);
    }

    let target_folder = target_folder.trim().trim_matches('/');
    let target_folder = if target_folder.is_empty() {
        String::new()
    } else {
        validate_asset_db_relative_path(target_folder).ok_or_else(|| {
            AssetSourceCreateRequestError::InvalidTargetFolder {
                value: target_folder.to_string(),
            }
        })?
    };

    let path = if target_folder.is_empty() {
        name.to_string()
    } else {
        format!("{target_folder}/{name}")
    };
    let path = enforce_source_extension(&path, extensions);
    validate_asset_db_relative_path(&path).ok_or_else(|| {
        AssetSourceCreateRequestError::InvalidSourcePath {
            value: path.clone(),
        }
    })
}

#[must_use]
pub fn validate_asset_db_relative_path(path: &str) -> Option<String> {
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

#[must_use]
pub fn source_path_matches_extensions(source_path: &str, extensions: &[String]) -> bool {
    let file_name = source_path
        .rsplit('/')
        .next()
        .unwrap_or(source_path)
        .to_ascii_lowercase();
    extensions.iter().any(|extension| {
        let extension = normalized_extension(extension);
        extension == "*" || file_name == extension || file_name.ends_with(&format!(".{extension}"))
    })
}

#[must_use]
pub fn enforce_source_extension(source_path: &str, extensions: &[String]) -> String {
    if extensions.is_empty() || source_path_matches_extensions(source_path, extensions) {
        return source_path.to_string();
    }

    let Some(extension) = extensions
        .iter()
        .map(|extension| normalized_extension(extension))
        .find(|extension| !extension.is_empty() && extension != "*")
    else {
        return source_path.to_string();
    };
    format!("{source_path}.{extension}")
}

#[must_use]
pub fn extension_hint(extensions: &[String]) -> String {
    if extensions.is_empty() {
        return "asset".to_string();
    }
    extensions
        .iter()
        .map(|extension| {
            let extension = normalized_extension(extension);
            if extension == "*" {
                "*".to_string()
            } else {
                format!(".{extension}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn asset_source_schema_label(schema: &AssetSourceSchemaData) -> String {
    if !schema.label.trim().is_empty() {
        return schema.label.trim().to_string();
    }
    schema
        .schema_type
        .rsplit(['.', ':'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or(&schema.schema_type)
        .to_string()
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn normalized_extension(extension: &str) -> String {
    extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::{AssetBuilderData, AssetSourceFileWorkflowData};

    fn source() -> CreatableAssetSourceData {
        CreatableAssetSourceData {
            schema_type: "az.test.Prefab".to_string(),
            label: "Prefab".to_string(),
            category: "Prefabs".to_string(),
            owner: "az-test".to_string(),
            source_root: "project:source-root".to_string(),
            default_path_prefix: "prefabs".to_string(),
            extensions: vec!["prefab.ron".to_string()],
        }
    }

    #[test]
    fn builds_create_request_with_target_folder_and_enforced_extension() {
        let request =
            build_asset_source_create_request(&source(), "levels/castle", "gate").expect("request");

        assert_eq!(request.schema_type, "az.test.Prefab");
        assert_eq!(request.source_root, "project:source-root");
        assert_eq!(&request.source_path, "levels/castle/gate.prefab.ron");
    }

    #[test]
    fn preserves_existing_matching_extension_case_insensitively() {
        let request = build_asset_source_create_request(&source(), "prefabs", "gate.PREFAB.RON")
            .expect("request");

        assert_eq!(&request.source_path, "prefabs/gate.PREFAB.RON");
    }

    #[test]
    fn accepts_bare_and_prefixed_compound_names() {
        let extensions = vec!["mapsettings.json".to_string()];

        assert!(source_path_matches_extensions(
            "levels/tutorial/mapsettings.json",
            &extensions
        ));
        assert!(source_path_matches_extensions(
            "levels/tutorial/region.mapsettings.json",
            &extensions
        ));
        assert!(!source_path_matches_extensions(
            "levels/tutorial/settings.json",
            &extensions
        ));
    }

    #[test]
    fn rejects_non_portable_source_paths_before_rpc() {
        let error = build_asset_source_create_request(&source(), "prefabs", "../outside")
            .expect_err("invalid path");

        assert!(matches!(
            error,
            AssetSourceCreateRequestError::InvalidSourcePath { .. }
        ));
    }

    #[test]
    fn projects_only_file_backed_creatable_sources() {
        let catalog = EditorAssetBuilderCatalog {
            builders: vec![AssetBuilderData {
                name: "builder".to_string(),
                builder_guid: "guid".to_string(),
                version: 1,
                patterns: Vec::new(),
                source_schema_types: vec!["az.test.Prefab".to_string()],
            }],
            source_schemas: vec![
                AssetSourceSchemaData {
                    schema_type: "az.test.Prefab".to_string(),
                    owner: "az-test".to_string(),
                    label: "Prefab".to_string(),
                    category: "Prefabs".to_string(),
                    authoring: AssetSourceSchemaAuthoringData::File {
                        workflow: AssetSourceFileWorkflowData {
                            source_root: "project:source-root".to_string(),
                            default_path_prefix: "prefabs".to_string(),
                            extensions: vec!["prefab.ron".to_string()],
                            can_create: true,
                            can_edit: true,
                        },
                    },
                    file_templates: Vec::new(),
                },
                AssetSourceSchemaData {
                    schema_type: "az.test.Import".to_string(),
                    owner: "az-test".to_string(),
                    label: "Import".to_string(),
                    category: "Imports".to_string(),
                    authoring: AssetSourceSchemaAuthoringData::File {
                        workflow: AssetSourceFileWorkflowData {
                            source_root: "project:source-root".to_string(),
                            default_path_prefix: "imports".to_string(),
                            extensions: vec!["bin".to_string()],
                            can_create: false,
                            can_edit: false,
                        },
                    },
                    file_templates: Vec::new(),
                },
            ],
        };

        let sources = creatable_asset_sources(&catalog);

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].schema_type, "az.test.Prefab");
        assert_eq!(sources[0].extension_hint(), ".prefab.ron");
    }

    #[test]
    fn current_folder_label_defaults_to_portable_folder() {
        let workflow = AssetSourceFileWorkflowData {
            source_root: "project:source-root".to_string(),
            default_path_prefix: "materials".to_string(),
            extensions: vec!["material.ron".to_string()],
            can_create: true,
            can_edit: true,
        };

        assert_eq!(
            default_target_folder_for_source("Meshes", &workflow),
            "meshes"
        );
        assert_eq!(
            default_target_folder_for_source("Assets", &workflow),
            "materials"
        );
    }
}
