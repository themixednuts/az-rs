use crate::error::{CliError, CliResult};
use az_project::ids_generation::{
    check_engine_ids_crate, check_gem_ids_crate, generate_engine_ids_crate, generate_gem_ids_crate,
};
use az_project::{
    GemManifest, ResolvedGemKind, load_engine_manifest, load_project_manifest, resolve_engine_gems,
    resolve_engine_root, resolve_project_gems,
};
use std::path::PathBuf;

pub fn new_gem(
    project_path: Option<PathBuf>,
    name: String,
    path: Option<PathBuf>,
    id: Option<String>,
    package: Option<String>,
    register: bool,
    capabilities: &[String],
) -> CliResult<()> {
    az_project_scaffold::gem::new_gem(
        project_path,
        name,
        path,
        id,
        package,
        register,
        capabilities,
    )?;
    Ok(())
}

pub fn register(
    project_path: Option<PathBuf>,
    path: PathBuf,
    id: Option<String>,
    package: Option<String>,
) -> CliResult<()> {
    az_project_scaffold::gem::register_existing_gem(project_path, path, id, package)?;
    Ok(())
}

pub fn enable_engine(
    project_path: Option<PathBuf>,
    id: String,
    capabilities: Vec<String>,
) -> CliResult<()> {
    az_project_scaffold::gem::enable_engine_gem(project_path, id, capabilities)?;
    Ok(())
}

pub fn list(project_path: Option<PathBuf>, all: bool) -> CliResult<()> {
    az_project_scaffold::gem::list(project_path, all)?;
    Ok(())
}

pub fn validate(project_path: Option<PathBuf>) -> CliResult<()> {
    az_project_scaffold::gem::validate(project_path)?;
    Ok(())
}

/// Generate or verify committed ids crates from each manifest that mints ids.
///
/// The engine mode covers the engine itself as well as its catalog gems: the
/// engine is a manifested composition root with the same stanza grammar, so it
/// has contribution ids to mint and one crate to mint them into
/// (asset-contract ticket 014, D3).
pub fn generate_ids(project_path: Option<PathBuf>, engine: bool, check: bool) -> CliResult<()> {
    let mut stale = Vec::new();
    let gems = if engine {
        let engine_root = resolve_engine_root()?;
        let manifest = load_engine_manifest(&engine_root)?;
        let id = manifest.engine.id.clone();
        if check {
            if check_engine_ids_crate(&manifest, &engine_root)? {
                println!("{id}: unchanged");
            } else {
                println!("{id}: stale");
                stale.push(id);
            }
        } else if generate_engine_ids_crate(&manifest, &engine_root)?.is_unchanged() {
            println!("{id}: unchanged");
        } else {
            println!("{id}: generated");
        }
        engine_ids_targets(&engine_root)?
    } else {
        project_ids_targets(project_path)?
    };

    for (id, root, manifest) in gems {
        if check {
            if check_gem_ids_crate(&manifest, &root)? {
                println!("{id}: unchanged");
            } else {
                println!("{id}: stale");
                stale.push(id);
            }
        } else {
            let outcome = generate_gem_ids_crate(&manifest, &root)?;
            if outcome.is_unchanged() {
                println!("{id}: unchanged");
            } else {
                println!("{id}: generated");
            }
        }
    }

    if stale.is_empty() {
        Ok(())
    } else {
        Err(CliError::StaleGemIds {
            crates: stale.join(", "),
        })
    }
}

fn engine_ids_targets(engine_root: &PathBuf) -> CliResult<Vec<(String, PathBuf, GemManifest)>> {
    let mut gems = resolve_engine_gems(engine_root)?
        .into_iter()
        .map(|gem| (gem.manifest.gem.id.clone(), gem.root, gem.manifest))
        .collect::<Vec<_>>();
    gems.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(gems)
}

fn project_ids_targets(
    project_path: Option<PathBuf>,
) -> CliResult<Vec<(String, PathBuf, GemManifest)>> {
    let project_path = project_path.unwrap_or_else(|| PathBuf::from("."));
    let manifest = load_project_manifest(&project_path)?;
    let mut gems = resolve_project_gems(&project_path, &manifest)?
        .into_iter()
        .filter(|gem| gem.kind == ResolvedGemKind::Project)
        .map(|gem| (gem.manifest.gem.id.clone(), gem.root, gem.manifest))
        .collect::<Vec<_>>();
    gems.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(gems)
}
