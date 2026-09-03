//! ADR 0022 deletion-target inventory and no-growth guard.
//!
//! Phase 0 freezes the legacy dependency surface by file. Later phases may
//! reduce a file's count or remove it; a new matching file or a higher count is
//! a violation until the checked inventory is deliberately regenerated.

use std::{collections::BTreeMap, fs, io, path::Path};

use serde::{Deserialize, Serialize};

pub const ADR_0022_DELETION_INVENTORY_FORMAT_VERSION: u32 = 1;
pub const ADR_0022_DELETION_INVENTORY_FILE: &str =
    "crates/az/architecture-guard/deletion-target-inventory.ron";

/// Literal members of the Phase 0 whole-tree search.
///
/// Matching is case-sensitive and counts matching lines, mirroring
/// `rg --count` with the alternation recorded in the migration plan.
pub const ADR_0022_DELETION_TARGETS: &[&str] = &[
    "az_schema",
    "az-schema",
    "AuthoredDocument",
    "AuthoredObject",
    "AuthoredValue",
    "EditValue",
    "FieldId",
    "VariantId",
    "Spawnable",
    "AZSPWN",
    "AZSCNM",
    ".spawnable",
    "az-prefab-runtime",
    "ComponentDescriptor",
    "ComponentDescriptorCatalog",
    "PortableDefault",
    "EntityLowerer",
    "SpawnableValue",
    "SceneManifest",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionTargetInventory {
    pub format_version: u32,
    pub search_semantics: String,
    pub excluded_roots: Vec<String>,
    pub entries: Vec<DeletionTargetInventoryEntry>,
}

impl DeletionTargetInventory {
    #[must_use]
    pub fn new(mut entries: Vec<DeletionTargetInventoryEntry>) -> Self {
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Self {
            format_version: ADR_0022_DELETION_INVENTORY_FORMAT_VERSION,
            search_semantics:
                "case-sensitive literal alternation; count each matching text line once".to_string(),
            excluded_roots: vec![".git".to_string(), "docs".to_string(), "target".to_string()],
            entries,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionTargetInventoryEntry {
    pub path: String,
    pub matching_lines: u32,
    pub owner: String,
    pub removal_phase: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeletionTargetInventoryViolation {
    NewFile {
        path: String,
        matching_lines: u32,
    },
    IncreasedCount {
        path: String,
        allowed_matching_lines: u32,
        actual_matching_lines: u32,
    },
    InvalidAnnotation {
        path: String,
        field: &'static str,
    },
}

/// Scans repository text files for the ADR 0022 deletion-target literals.
///
/// Documentation and build/VCS output are outside the inventory, matching the
/// migration plan. The allowlist is excluded to avoid a self-referential
/// count. Non-UTF-8 files are product/binary inputs and are skipped.
///
/// # Errors
///
/// Returns a filesystem error when a directory or text file cannot be read.
pub fn scan_adr_0022_deletion_targets(
    workspace_root: impl AsRef<Path>,
) -> io::Result<Vec<DeletionTargetInventoryEntry>> {
    let workspace_root = workspace_root.as_ref();
    let mut entries = Vec::new();
    scan_directory(workspace_root, workspace_root, &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn scan_directory(
    directory: &Path,
    workspace_root: &Path,
    entries: &mut Vec<DeletionTargetInventoryEntry>,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let relative = normalized_relative_path(workspace_root, &path);
        if path.is_dir() {
            if excluded_directory(&relative) {
                continue;
            }
            scan_directory(&path, workspace_root, entries)?;
            continue;
        }
        if relative == ADR_0022_DELETION_INVENTORY_FILE || excluded_file(&relative) {
            continue;
        }

        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => continue,
            Err(error) => return Err(error),
        };
        let matching_lines = matching_line_count(&source);
        if matching_lines == 0 {
            continue;
        }
        entries.push(DeletionTargetInventoryEntry {
            owner: owning_package(workspace_root, &path),
            removal_phase: removal_phase_for(&relative),
            path: relative,
            matching_lines,
        });
    }
    Ok(())
}

fn excluded_directory(relative: &str) -> bool {
    let first = relative.split('/').next().unwrap_or_default();
    matches!(
        first,
        ".git" | ".idea" | ".vscode" | "docs" | "node_modules" | "target"
    )
}

/// Binary and image products the deletion guard never reads as source.
///
/// The comparison is case-insensitive because Windows checkouts routinely carry
/// `.PNG` / `.DLL` spellings that mean the same thing.
fn excluded_file(relative: &str) -> bool {
    const EXCLUDED_EXTENSIONS: [&str; 9] = [
        "pdb", "dll", "exe", "lib", "obj", "png", "jpg", "ico", "woff2",
    ];
    std::path::Path::new(relative)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| {
            EXCLUDED_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

fn matching_line_count(source: &str) -> u32 {
    source
        .lines()
        .filter(|line| {
            ADR_0022_DELETION_TARGETS
                .iter()
                .any(|target| line.contains(target))
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

fn normalized_relative_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn owning_package(workspace_root: &Path, path: &Path) -> String {
    let mut directory = path.parent();
    while let Some(candidate) = directory {
        let manifest = candidate.join("Cargo.toml");
        if manifest.is_file()
            && let Ok(text) = fs::read_to_string(&manifest)
            && let Ok(value) = toml::from_str::<toml::Value>(&text)
            && let Some(name) = value
                .get("package")
                .and_then(|package| package.get("name"))
                .and_then(toml::Value::as_str)
        {
            return name.to_string();
        }
        if candidate == workspace_root {
            break;
        }
        directory = candidate.parent();
    }
    "azoth-workspace".to_string()
}

fn removal_phase_for(path: &str) -> String {
    if path.starts_with("vendor/") || path.starts_with("third_party/") {
        return "not applicable: unrelated external symbol; classify again at phase 7".to_string();
    }
    if path.starts_with("crates/az/prefab-runtime/") {
        return "phase 6: runtime product cutover".to_string();
    }
    if path.starts_with("crates/az/prefab-builder/") {
        return "phase 5 reroute; phase 6 old product removal".to_string();
    }
    if path.starts_with("crates/editor/") {
        return "phase 4 reroute; phase 6 old adapter removal".to_string();
    }
    if path.starts_with("crates/az/project-host/") || path.starts_with("crates/az/proto/") {
        return "phase 4 vNext implementation; phase 6-7 old contract removal".to_string();
    }
    if path.starts_with("crates/az/project/") || path.starts_with("crates/az/prefab/") {
        return "phase 2 replacement; phase 7 old model removal".to_string();
    }
    if path.starts_with("crates/az/asset-processor/") {
        return "phase 5 product reroute; phase 6 old route removal".to_string();
    }
    if path.starts_with("crates/az/schema/")
        || path.starts_with("crates/az/schema-derive/")
        || path == "Cargo.toml"
        || path == "Cargo.lock"
    {
        return "phase 7: delete old authority and dependencies".to_string();
    }
    if path.starts_with("crates/az/transform/")
        || path.starts_with("crates/az/render/")
        || path.starts_with("crates/az/material/")
        || path.starts_with("crates/az/terrain/")
        || path.starts_with("crates/az/node-graph/")
        || path.starts_with("crates/az/graph-builder/")
        || path.starts_with("gems/gamedata")
    {
        return "phase 3: register real types and remove per-type duplication".to_string();
    }
    "phase 3 reroute; phase 7 final old-symbol removal or classification".to_string()
}

/// Compares a current scan with the Phase 0 baseline.
///
/// Reductions and missing files are allowed. Only accretion is rejected.
#[must_use]
pub fn adr_0022_deletion_inventory_violations(
    allowed: &DeletionTargetInventory,
    actual: &[DeletionTargetInventoryEntry],
) -> Vec<DeletionTargetInventoryViolation> {
    let allowed_by_path = allowed
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut violations = Vec::new();

    for entry in &allowed.entries {
        if entry.owner.trim().is_empty() {
            violations.push(DeletionTargetInventoryViolation::InvalidAnnotation {
                path: entry.path.clone(),
                field: "owner",
            });
        }
        if entry.removal_phase.trim().is_empty() {
            violations.push(DeletionTargetInventoryViolation::InvalidAnnotation {
                path: entry.path.clone(),
                field: "removal_phase",
            });
        }
    }
    for entry in actual {
        match allowed_by_path.get(entry.path.as_str()) {
            None => violations.push(DeletionTargetInventoryViolation::NewFile {
                path: entry.path.clone(),
                matching_lines: entry.matching_lines,
            }),
            Some(allowed_entry) if entry.matching_lines > allowed_entry.matching_lines => {
                violations.push(DeletionTargetInventoryViolation::IncreasedCount {
                    path: entry.path.clone(),
                    allowed_matching_lines: allowed_entry.matching_lines,
                    actual_matching_lines: entry.matching_lines,
                });
            }
            Some(_) => {}
        }
    }
    violations
}

/// # Errors
///
/// [`ron::error::SpannedError`] when `text` is not a well-formed
/// `DeletionTargetInventory` document.
pub fn decode_deletion_target_inventory(
    text: &str,
) -> Result<DeletionTargetInventory, ron::error::SpannedError> {
    ron::from_str(text)
}

/// # Errors
///
/// [`ron::Error`] when the inventory cannot be serialised.
pub fn encode_deletion_target_inventory(
    inventory: &DeletionTargetInventory,
) -> Result<String, ron::Error> {
    let mut text = ron::ser::to_string_pretty(inventory, ron::ser::PrettyConfig::default())?;
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("architecture guard lives under crates/az")
            .to_path_buf()
    }

    #[test]
    fn adr_0022_deletion_target_inventory_does_not_grow() {
        let root = workspace_root();
        let allowlist = fs::read_to_string(root.join(ADR_0022_DELETION_INVENTORY_FILE))
            .expect("read checked ADR 0022 inventory");
        let allowlist = decode_deletion_target_inventory(&allowlist)
            .expect("decode checked ADR 0022 inventory");
        let actual = scan_adr_0022_deletion_targets(&root).expect("scan ADR 0022 deletion targets");
        let violations = adr_0022_deletion_inventory_violations(&allowlist, &actual);

        assert!(
            violations.is_empty(),
            "ADR 0022 deletion-target dependencies may shrink but may not grow.\n{violations:#?}"
        );
    }

    #[test]
    fn new_matching_file_is_a_violation() {
        let allowlist = DeletionTargetInventory::new(Vec::new());
        let actual = vec![DeletionTargetInventoryEntry {
            path: "crates/az/example/src/new_dependency.rs".to_string(),
            matching_lines: 1,
            owner: "az-example".to_string(),
            removal_phase: "phase 7".to_string(),
        }];

        assert_eq!(
            adr_0022_deletion_inventory_violations(&allowlist, &actual),
            vec![DeletionTargetInventoryViolation::NewFile {
                path: "crates/az/example/src/new_dependency.rs".to_string(),
                matching_lines: 1,
            }]
        );
    }

    #[test]
    fn reductions_are_allowed() {
        let allowlist = DeletionTargetInventory::new(vec![DeletionTargetInventoryEntry {
            path: "crates/az/example/src/lib.rs".to_string(),
            matching_lines: 4,
            owner: "az-example".to_string(),
            removal_phase: "phase 7".to_string(),
        }]);
        let actual = vec![DeletionTargetInventoryEntry {
            matching_lines: 2,
            ..allowlist.entries[0].clone()
        }];

        assert!(adr_0022_deletion_inventory_violations(&allowlist, &actual).is_empty());
    }
}
