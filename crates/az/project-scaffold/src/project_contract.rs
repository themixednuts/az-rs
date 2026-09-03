//! Azoth-owned project workspace and resolution contract.

use std::path::{Path, PathBuf};

use az_filesystem::AzothDataHome;
use az_project::{
    GeneratedTargetsSyncReport, GeneratedTargetsSyncStatus, ProjectEnginePatchSyncReport,
    ProjectManifest, ProjectTopologyKind, ensure_project_engine_patch_table,
    ensure_project_generated_targets, load_project_manifest, resolve_project_lock,
    sync_project_engine_patch_table, write_project_lock,
};
use tracing::{info, instrument};

use crate::{ScaffoldResult, init};

const RETIRED_GENERATED_WORKSPACE_MEMBERS: &[&str] = &[".azoth/targets/*"];

/// Result of synchronizing all engine-owned project-local Cargo state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectContractSyncReport {
    pub cargo_manifest_path: PathBuf,
    pub project_lock_path: PathBuf,
    pub generated_graphs_root: PathBuf,
    pub engine_patch: ProjectEnginePatchSyncReport,
    pub generated_targets: GeneratedTargetsSyncReport,
}

/// Idempotently repairs the authored workspace policy and all derived locks.
///
/// Project source manifests remain portable: engine paths live only in the
/// ignored, generated `.cargo/config.toml`. Cargo uses its normal platform
/// linker because that generated config owns no linker or rustflags override.
#[instrument(skip_all, fields(project_root = %project_root.as_ref().display()))]
/// # Errors
///
/// Returns any error [`sync_project_contract_force_engine`] returns; this
/// variant only skips recomputing the engine projection when it is already
/// current.
pub fn sync_project_contract(
    project_root: impl AsRef<Path>,
) -> ScaffoldResult<ProjectContractSyncReport> {
    sync_project_contract_with_engine_mode(project_root.as_ref(), false)
}

/// Synchronizes the project contract and unconditionally recomputes the
/// selected engine projection and workspace lock.
///
/// # Errors
///
/// Returns [`ScaffoldError::ProjectManifest`] if the project or gem manifests
/// cannot be loaded or validated, if the engine root cannot be resolved, or if
/// the generated target workspace, engine patch table, or project lock cannot
/// be written; and [`ScaffoldError::Io`] if the workspace files themselves
/// cannot be read or written.
#[instrument(skip_all, fields(project_root = %project_root.as_ref().display()))]
pub fn sync_project_contract_force_engine(
    project_root: impl AsRef<Path>,
) -> ScaffoldResult<ProjectContractSyncReport> {
    sync_project_contract_with_engine_mode(project_root.as_ref(), true)
}

fn sync_project_contract_with_engine_mode(
    project_root: &Path,
    force_engine: bool,
) -> ScaffoldResult<ProjectContractSyncReport> {
    let manifest = load_project_manifest(project_root)?;
    let generated_graphs_root =
        ensure_generated_data_contract(&AzothDataHome::resolve(), project_root, &manifest)?;
    let default_members = native_default_members(project_root, &manifest);
    // Existing projects own their dependency catalog; the contract only
    // repairs engine-owned workspace policy. Generated targets are independent
    // workspaces and therefore never enter this authored member set.
    init::ensure_workspace_contract(
        project_root,
        &default_members,
        &default_members,
        RETIRED_GENERATED_WORKSPACE_MEMBERS,
    )?;

    let lock = resolve_project_lock(project_root, &manifest)?;
    write_project_lock(project_root, &lock)?;

    // Legacy projects report LegacyLayout. Primary-gem projects regenerate or
    // verify their ignored role-filtered target packages and commit marker.
    // Target generation owns the authored-only bootstrap used when an old
    // generated manifest cannot resolve against the newly selected engine.
    let generated_targets = ensure_project_generated_targets(project_root)?;
    // Target generation changes the Cargo workspace graph after its bootstrap
    // patch projection has already been resolved. Recompute the full engine
    // projection and lock whenever targets changed; a freshness-only patch
    // check cannot prove that the new generated packages are locked.
    let engine_patch =
        if engine_projection_requires_full_sync(force_engine, generated_targets.status) {
            sync_project_engine_patch_table(project_root)?
        } else {
            ensure_project_engine_patch_table(project_root)?
        };

    let report = ProjectContractSyncReport {
        cargo_manifest_path: project_root.join("Cargo.toml"),
        project_lock_path: az_project::project_lock_path(project_root),
        generated_graphs_root,
        engine_patch,
        generated_targets,
    };
    info!(
        patch_status = ?report.engine_patch.status,
        target_status = ?report.generated_targets.status,
        "synchronized Azoth project contract"
    );
    Ok(report)
}

const fn engine_projection_requires_full_sync(
    force_engine: bool,
    status: GeneratedTargetsSyncStatus,
) -> bool {
    force_engine
        || matches!(
            status,
            GeneratedTargetsSyncStatus::LegacyLayout | GeneratedTargetsSyncStatus::Regenerated
        )
}

fn ensure_generated_data_contract(
    data_home: &AzothDataHome,
    project_root: &Path,
    manifest: &ProjectManifest,
) -> std::io::Result<PathBuf> {
    let graphs_root = data_home
        .project(&manifest.project.id, project_root)
        .graphs_dir();
    std::fs::create_dir_all(&graphs_root)?;
    Ok(graphs_root)
}

fn native_default_members(project_root: &Path, manifest: &ProjectManifest) -> Vec<String> {
    let candidate = manifest
        .project
        .primary_gem
        .as_deref()
        .and_then(|primary_id| {
            manifest
                .gems
                .iter()
                .find(|gem| gem.enabled && gem.id == primary_id)
        })
        .and_then(|gem| gem.path.as_ref())
        .map(|root| {
            root.join(match manifest.topology.kind {
                ProjectTopologyKind::SinglePlayer => "runtime",
                ProjectTopologyKind::MultiplayerClientServer => "server",
                ProjectTopologyKind::MultiplayerPeerToPeer => "p2p",
            })
        });

    let Some(relative) =
        candidate.filter(|relative| project_root.join(relative).join("Cargo.toml").is_file())
    else {
        return Vec::new();
    };

    vec![portable_path(&relative)]
}

fn portable_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_project::{ProjectGem, project_id_from_name};

    #[test]
    fn project_contract_materializes_the_graph_watch_root_before_cargo() {
        let temp = tempfile::tempdir().unwrap();
        let data_home = AzothDataHome::new(temp.path().join("azoth-home"));
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let manifest = ProjectManifest::new(
            project_id_from_name("sample"),
            "sample",
            env!("CARGO_PKG_VERSION"),
        );

        let graphs_root =
            ensure_generated_data_contract(&data_home, &project_root, &manifest).unwrap();

        assert_eq!(
            graphs_root,
            data_home
                .project(&manifest.project.id, &project_root)
                .graphs_dir()
        );
        assert!(graphs_root.is_dir());
        assert_eq!(
            ensure_generated_data_contract(&data_home, &project_root, &manifest).unwrap(),
            graphs_root
        );
    }

    #[test]
    fn changed_generated_targets_force_full_engine_and_lock_sync() {
        assert!(engine_projection_requires_full_sync(
            false,
            GeneratedTargetsSyncStatus::LegacyLayout
        ));
        assert!(engine_projection_requires_full_sync(
            false,
            GeneratedTargetsSyncStatus::Regenerated
        ));
        assert!(!engine_projection_requires_full_sync(
            false,
            GeneratedTargetsSyncStatus::Unchanged
        ));
        assert!(engine_projection_requires_full_sync(
            true,
            GeneratedTargetsSyncStatus::Unchanged
        ));
    }

    #[test]
    fn native_defaults_follow_typed_topology_without_project_package_names() {
        let temp = tempfile::tempdir().unwrap();
        for role in ["runtime", "server", "p2p"] {
            std::fs::create_dir_all(temp.path().join("gems/sample").join(role)).unwrap();
            std::fs::write(
                temp.path()
                    .join("gems/sample")
                    .join(role)
                    .join("Cargo.toml"),
                "[package]\nname = \"sample\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();
        }
        let mut manifest = ProjectManifest::new(
            project_id_from_name("sample"),
            "sample",
            env!("CARGO_PKG_VERSION"),
        );
        manifest.project.primary_gem = Some("sample.game".to_string());
        manifest.gems.push(ProjectGem {
            id: "sample.game".to_string(),
            enabled: true,
            capabilities: Vec::new(),
            linkage: None,
            path: Some(PathBuf::from("gems/sample")),
        });

        for (topology, expected) in [
            (ProjectTopologyKind::SinglePlayer, "gems/sample/runtime"),
            (
                ProjectTopologyKind::MultiplayerClientServer,
                "gems/sample/server",
            ),
            (
                ProjectTopologyKind::MultiplayerPeerToPeer,
                "gems/sample/p2p",
            ),
        ] {
            manifest.topology.kind = topology;
            assert_eq!(native_default_members(temp.path(), &manifest), [expected]);
        }
    }

    #[test]
    fn missing_conventional_role_does_not_invent_a_workspace_member() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new(
            project_id_from_name("custom"),
            "custom",
            env!("CARGO_PKG_VERSION"),
        );
        manifest.project.primary_gem = Some("custom.game".to_string());
        manifest.gems.push(ProjectGem {
            id: "custom.game".to_string(),
            enabled: true,
            capabilities: Vec::new(),
            linkage: None,
            path: Some(PathBuf::from("some/custom-layout")),
        });

        assert!(native_default_members(temp.path(), &manifest).is_empty());
    }

    #[test]
    fn generated_targets_never_enter_the_authored_workspace() {
        let defaults = vec!["gems/sample/server".to_string()];

        assert_eq!(defaults, ["gems/sample/server"]);
    }
}
