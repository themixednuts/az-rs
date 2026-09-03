//! Editor-shell source navigation.
//!
//! Project-host is the authority for resolving source-link descriptors to
//! paths. This module owns the local editor URL that should be opened for a
//! resolved target, plus the editor-shell routing classification for material
//! sources (asset-browser activation → Materials mode).

use az_proto_project::{NodeSourceLinkPathKind, NodeSourceLinkTarget};

use crate::error::{EditorError, EditorResult};
use crate::settings::SourceNavigationSettings;

/// Material source kind classified from an authored source path / document id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialSourceKind {
    /// `.azmat.ron` material node-graph source (visual-graph document).
    Graph,
    /// `.azmaterial.ron` material instance source (property document).
    Material,
    /// `.azmaterialtype.ron` material type source (property document).
    MaterialType,
}

/// Classify a source path / document id as a material source.
///
/// Activating any of these in the asset browser routes the editor shell into
/// Materials mode. Extensions come from the az-material authored-schema
/// document hints.
#[must_use]
pub fn material_source_kind(path: &str) -> Option<MaterialSourceKind> {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let ends_with_extension =
        |extension: &str| normalized.ends_with(&format!(".{extension}").to_ascii_lowercase());
    if ends_with_extension(az_material::MATERIAL_GRAPH_EXTENSION) {
        Some(MaterialSourceKind::Graph)
    } else if ends_with_extension(az_material::MATERIAL_TYPE_EXTENSION) {
        Some(MaterialSourceKind::MaterialType)
    } else if ends_with_extension(az_material::MATERIAL_EXTENSION) {
        Some(MaterialSourceKind::Material)
    } else {
        None
    }
}

/// Absolute-ish join of a script asset's source root + relative source path.
///
/// Source roots resolved by the asset pipeline are already project-absolute
/// (`az_project::project_relative_path` joins them against the project
/// root), so this is a plain string join — no editor-side filesystem
/// scanning.
#[must_use]
pub fn join_script_source_path(source_root: &str, source_path: &str) -> String {
    let root = source_root.replace('\\', "/");
    let root = root.trim_end_matches('/');
    let relative = source_path.replace('\\', "/");
    let relative = relative.trim_start_matches('/');
    if root.is_empty() {
        relative.to_owned()
    } else if relative.is_empty() {
        root.to_owned()
    } else {
        format!("{root}/{relative}")
    }
}

/// Build the "open in external editor" navigation intent for a script asset
/// source.
///
/// Mirrors [`source_navigation_file_intent`] for graph node source links, but
/// a script source is located by asset-browser source root + relative path
/// rather than a project-host-resolved node source link.
///
/// # Errors
///
/// Returns [`EditorError::SourceNavigationMissingResolvedPath`] if joining
/// `source_root` and `source_path` yields a blank path, or any error
/// [`expand_source_navigation_template`] returns for the configured file URL
/// template.
pub fn script_source_navigation_intent(
    settings: &SourceNavigationSettings,
    source_root: &str,
    source_path: &str,
) -> EditorResult<SourceNavigationIntent> {
    let path = join_script_source_path(source_root, source_path);
    if path.trim().is_empty() {
        return Err(EditorError::SourceNavigationMissingResolvedPath {
            target: source_path.to_owned(),
            path_kind: "script-source",
        });
    }
    let url = expand_source_navigation_template(
        &settings.file_url_template,
        &SourceNavigationTemplateValues {
            raw_path: &path,
            line: None,
            column: None,
        },
    )?;
    Ok(SourceNavigationIntent {
        label: format!("script {path}"),
        url,
        kind: SourceNavigationIntentKind::File,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceNavigationIntent {
    pub url: String,
    pub label: String,
    pub kind: SourceNavigationIntentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceNavigationIntentKind {
    File,
    Docs,
}

#[must_use]
pub fn default_source_navigation_settings() -> SourceNavigationSettings {
    SourceNavigationSettings::default()
}

/// Build the navigation intent for a resolved graph node source link, routing
/// docs-only targets to their docs URL and path targets to the external editor.
///
/// # Errors
///
/// Returns [`EditorError::SourceNavigationUnresolved`] if the link's path kind
/// is `Unresolved`, [`EditorError::SourceNavigationMissingDocsUrl`] if a
/// docs-only target carries no docs URL,
/// [`EditorError::SourceNavigationMissingResolvedPath`] if a path target
/// carries no resolved path, [`EditorError::SourceNavigationMissingFile`] if
/// that path does not exist, or
/// [`EditorError::InvalidSourceNavigationTemplate`] if the configured URL
/// template cannot be expanded.
pub fn source_navigation_intent(
    settings: &SourceNavigationSettings,
    target: &NodeSourceLinkTarget,
) -> EditorResult<SourceNavigationIntent> {
    match target.path_kind {
        NodeSourceLinkPathKind::DocsOnly => source_navigation_docs_intent(target),
        NodeSourceLinkPathKind::Absolute
        | NodeSourceLinkPathKind::PackageRelative
        | NodeSourceLinkPathKind::WorkspaceRelative => {
            source_navigation_file_intent(settings, target)
        }
        NodeSourceLinkPathKind::Unresolved => Err(EditorError::SourceNavigationUnresolved {
            target: source_navigation_source_label(target),
        }),
    }
}

fn source_navigation_docs_intent(
    target: &NodeSourceLinkTarget,
) -> EditorResult<SourceNavigationIntent> {
    let url = target
        .source_link
        .docs_url
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| EditorError::SourceNavigationMissingDocsUrl {
            target: source_navigation_source_label(target),
        })?;
    Ok(SourceNavigationIntent {
        label: format!("docs {url}"),
        url,
        kind: SourceNavigationIntentKind::Docs,
    })
}

fn source_navigation_file_intent(
    settings: &SourceNavigationSettings,
    target: &NodeSourceLinkTarget,
) -> EditorResult<SourceNavigationIntent> {
    let path = target
        .resolved_path
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| EditorError::SourceNavigationMissingResolvedPath {
            target: source_navigation_source_label(target),
            path_kind: source_navigation_path_kind_label(target.path_kind),
        })?;
    if !target.exists {
        return Err(EditorError::SourceNavigationMissingFile {
            target: source_navigation_source_label(target),
            path: path.clone(),
        });
    }

    let url = expand_source_navigation_template(
        &settings.file_url_template,
        &SourceNavigationTemplateValues {
            raw_path: path,
            line: target.source_link.line,
            column: target.source_link.column,
        },
    )?;
    Ok(SourceNavigationIntent {
        label: format!(
            "{} {}",
            source_navigation_path_kind_label(target.path_kind),
            source_navigation_file_location(
                path,
                target.source_link.line,
                target.source_link.column
            )
        ),
        url,
        kind: SourceNavigationIntentKind::File,
    })
}

struct SourceNavigationTemplateValues<'a> {
    raw_path: &'a str,
    line: Option<u32>,
    column: Option<u32>,
}

fn expand_source_navigation_template(
    template: &str,
    values: &SourceNavigationTemplateValues<'_>,
) -> EditorResult<String> {
    let mut expanded = String::with_capacity(template.len() + values.raw_path.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        expanded.push_str(&rest[..start]);
        let token_start = start + 1;
        let Some(end_offset) = rest[token_start..].find('}') else {
            return Err(invalid_source_navigation_template(
                template,
                "unterminated token",
            ));
        };
        let token_end = token_start + end_offset;
        let token = &rest[token_start..token_end];
        expanded.push_str(&source_navigation_template_value(template, token, values)?);
        rest = &rest[(token_end + 1)..];
    }
    if rest.contains('}') {
        return Err(invalid_source_navigation_template(
            template,
            "unmatched closing brace",
        ));
    }
    expanded.push_str(rest);
    Ok(expanded)
}

fn source_navigation_template_value(
    template: &str,
    token: &str,
    values: &SourceNavigationTemplateValues<'_>,
) -> EditorResult<String> {
    match token {
        "path" => Ok(url_escape_source_path(values.raw_path)),
        "raw_path" => Ok(values.raw_path.to_string()),
        "line" => Ok(values.line.map(|line| line.to_string()).unwrap_or_default()),
        "column" => Ok(values
            .column
            .map(|column| column.to_string())
            .unwrap_or_default()),
        "line_suffix" => Ok(values
            .line
            .map(|line| format!(":{line}"))
            .unwrap_or_default()),
        "line_column_suffix" => Ok(match (values.line, values.column) {
            (Some(line), Some(column)) => format!(":{line}:{column}"),
            (Some(line), None) => format!(":{line}"),
            _ => String::new(),
        }),
        _ => Err(invalid_source_navigation_template(
            template,
            &format!("unknown token `{token}`"),
        )),
    }
}

fn invalid_source_navigation_template(template: &str, reason: &str) -> EditorError {
    EditorError::InvalidSourceNavigationTemplate {
        template: template.to_string(),
        reason: reason.to_string(),
    }
}

const fn source_navigation_path_kind_label(kind: NodeSourceLinkPathKind) -> &'static str {
    match kind {
        NodeSourceLinkPathKind::Unresolved => "unresolved",
        NodeSourceLinkPathKind::Absolute => "absolute",
        NodeSourceLinkPathKind::PackageRelative => "package-relative",
        NodeSourceLinkPathKind::WorkspaceRelative => "workspace-relative",
        NodeSourceLinkPathKind::DocsOnly => "docs",
    }
}

fn source_navigation_source_label(target: &NodeSourceLinkTarget) -> String {
    if let Some(symbol_path) = target
        .source_link
        .symbol_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return symbol_path.to_string();
    }
    if let Some(module_path) = target
        .source_link
        .module_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return module_path.to_string();
    }
    if let Some(file) = target
        .source_link
        .file
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return file.to_string();
    }
    if let Some(docs_url) = target
        .source_link
        .docs_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return docs_url.to_string();
    }
    "source link".to_string()
}

fn source_navigation_file_location(path: &str, line: Option<u32>, column: Option<u32>) -> String {
    match (line, column) {
        (Some(line), Some(column)) => format!("{path}:{line}:{column}"),
        (Some(line), None) => format!("{path}:{line}"),
        _ => path.to_string(),
    }
}

fn url_escape_source_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut escaped = String::with_capacity(normalized.len());
    for byte in normalized.bytes() {
        if is_source_path_uri_byte(byte) {
            escaped.push(byte as char);
        } else {
            escaped.push('%');
            escaped.push(HEX[(byte >> 4) as usize] as char);
            escaped.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    escaped
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

const fn is_source_path_uri_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'.' | b'-' | b'_' | b'~')
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_node_graph::NodeSourceLink;

    fn windows_path(parts: &[&str]) -> String {
        let mut path = String::from("E:");
        for part in parts {
            path.push('\\');
            path.push_str(part);
        }
        path
    }

    #[test]
    fn classifies_material_sources_by_authored_extension() {
        assert_eq!(
            material_source_kind("materials/graphs/metal_brushed.azmat.ron"),
            Some(MaterialSourceKind::Graph)
        );
        assert_eq!(
            material_source_kind(r"materials\Metal.AZMATERIAL.RON"),
            Some(MaterialSourceKind::Material)
        );
        assert_eq!(
            material_source_kind("materials/types/pbr.azmaterialtype.ron"),
            Some(MaterialSourceKind::MaterialType)
        );
        assert_eq!(material_source_kind("gamedata/items.ron"), None);
        assert_eq!(material_source_kind("textures/metal.dds"), None);
        // A bare ".ron" that merely contains "material" is not a material doc.
        assert_eq!(material_source_kind("materials/notes.ron"), None);
    }

    #[test]
    fn joins_script_source_root_and_relative_path() {
        let source_root = windows_path(&["workspace", "scripts"]);
        let expected = source_root.replace('\\', "/");
        assert_eq!(
            join_script_source_path(&expected, "ai/open_chest.lua"),
            format!("{expected}/ai/open_chest.lua")
        );
        assert_eq!(
            join_script_source_path(&format!("{source_root}\\"), "/open_chest.lua"),
            format!("{expected}/open_chest.lua")
        );
        assert_eq!(
            join_script_source_path("", "open_chest.lua"),
            "open_chest.lua"
        );
    }

    #[test]
    fn builds_script_external_editor_intent_from_source_root_and_path() {
        let source_root = windows_path(&["workspace", "scripts"]);
        let intent = script_source_navigation_intent(
            &SourceNavigationSettings::default(),
            &source_root,
            "ai/open_chest.lua",
        )
        .expect("build script navigation intent");

        assert_eq!(intent.kind, SourceNavigationIntentKind::File);
        assert_eq!(
            intent.url,
            format!(
                "vscode://file/{}/ai/open_chest.lua",
                source_root.replace('\\', "/")
            )
        );
    }

    #[test]
    fn rejects_empty_script_source_path() {
        let error = script_source_navigation_intent(&SourceNavigationSettings::default(), "", "")
            .expect_err("empty script source path should be rejected");

        assert!(matches!(
            error,
            EditorError::SourceNavigationMissingResolvedPath { .. }
        ));
    }

    #[test]
    fn builds_default_vscode_file_deep_link() {
        let resolved_path = windows_path(&[
            "workspace",
            "sample-game",
            "crates",
            "game",
            "src",
            "systems.rs",
        ]);
        let target = NodeSourceLinkTarget {
            source_link: NodeSourceLink::rust_symbol(
                "game",
                "game::systems",
                "game::systems::spawn",
                "src/systems.rs",
                42,
                7,
            ),
            resolved_path: Some(resolved_path.clone()),
            package_root: None,
            package_id: Some("game".to_string()),
            path_kind: NodeSourceLinkPathKind::PackageRelative,
            exists: true,
        };

        let intent = source_navigation_intent(&SourceNavigationSettings::default(), &target)
            .expect("build source navigation intent");

        assert_eq!(intent.kind, SourceNavigationIntentKind::File);
        assert_eq!(
            intent.url,
            format!("vscode://file/{}:42:7", resolved_path.replace('\\', "/"))
        );
    }

    #[test]
    fn escapes_file_deep_link_path_spaces() {
        let resolved_path = windows_path(&["workspace", "sample game", "src", "main.rs"]);
        let target = NodeSourceLinkTarget {
            source_link: NodeSourceLink::rust_symbol(
                "game",
                "game",
                "game::entry",
                "src/main.rs",
                1,
                1,
            ),
            resolved_path: Some(resolved_path.clone()),
            package_root: None,
            package_id: Some("game".to_string()),
            path_kind: NodeSourceLinkPathKind::WorkspaceRelative,
            exists: true,
        };

        let intent = source_navigation_intent(&SourceNavigationSettings::default(), &target)
            .expect("build source navigation intent");

        assert_eq!(
            intent.url,
            format!(
                "vscode://file/{}:1:1",
                resolved_path.replace('\\', "/").replace(' ', "%20")
            )
        );
    }

    #[test]
    fn custom_template_can_target_cursor() {
        let resolved_path = windows_path(&["workspace", "sample-game", "src", "main.rs"]);
        let target = NodeSourceLinkTarget {
            source_link: NodeSourceLink::rust_symbol(
                "game",
                "game",
                "game::entry",
                "src/main.rs",
                3,
                5,
            ),
            resolved_path: Some(resolved_path.clone()),
            package_root: None,
            package_id: Some("game".to_string()),
            path_kind: NodeSourceLinkPathKind::WorkspaceRelative,
            exists: true,
        };
        let settings = SourceNavigationSettings {
            file_url_template: "cursor://file/{path}{line_column_suffix}".to_string(),
        };

        let intent =
            source_navigation_intent(&settings, &target).expect("build source navigation intent");

        assert_eq!(
            intent.url,
            format!("cursor://file/{}:3:5", resolved_path.replace('\\', "/"))
        );
    }

    #[test]
    fn docs_target_uses_docs_url_without_file_template() {
        let target = NodeSourceLinkTarget {
            source_link: NodeSourceLink::docs_url("https://example.test/node"),
            resolved_path: None,
            package_root: None,
            package_id: None,
            path_kind: NodeSourceLinkPathKind::DocsOnly,
            exists: true,
        };

        let intent = source_navigation_intent(&SourceNavigationSettings::default(), &target)
            .expect("build source navigation intent");

        assert_eq!(intent.kind, SourceNavigationIntentKind::Docs);
        assert_eq!(intent.url, "https://example.test/node");
    }

    #[test]
    fn rejects_missing_resolved_file() {
        let resolved_path = windows_path(&["workspace", "sample-game", "src", "main.rs"]);
        let target = NodeSourceLinkTarget {
            source_link: NodeSourceLink::rust_symbol(
                "game",
                "game",
                "game::entry",
                "src/main.rs",
                3,
                5,
            ),
            resolved_path: Some(resolved_path),
            package_root: None,
            package_id: Some("game".to_string()),
            path_kind: NodeSourceLinkPathKind::WorkspaceRelative,
            exists: false,
        };

        let error = source_navigation_intent(&SourceNavigationSettings::default(), &target)
            .expect_err("missing file should be rejected");

        assert!(matches!(
            error,
            EditorError::SourceNavigationMissingFile { .. }
        ));
    }

    #[test]
    fn rejects_unknown_template_token() {
        let resolved_path = windows_path(&["workspace", "sample-game", "src", "main.rs"]);
        let target = NodeSourceLinkTarget {
            source_link: NodeSourceLink::rust_symbol(
                "game",
                "game",
                "game::entry",
                "src/main.rs",
                1,
                1,
            ),
            resolved_path: Some(resolved_path),
            package_root: None,
            package_id: Some("game".to_string()),
            path_kind: NodeSourceLinkPathKind::WorkspaceRelative,
            exists: true,
        };
        let settings = SourceNavigationSettings {
            file_url_template: "vscode://file/{project}/{path}".to_string(),
        };

        let error = source_navigation_intent(&settings, &target)
            .expect_err("unknown token should be rejected");

        assert!(matches!(
            error,
            EditorError::InvalidSourceNavigationTemplate { .. }
        ));
    }
}
