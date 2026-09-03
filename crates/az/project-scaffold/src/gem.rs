use crate::error::{ScaffoldError, ScaffoldResult};
use crate::new;
use az_project::{
    GemCapability, GemContribution, GemDependency, GemManifest, GemOrigin, GemTargetRole,
    ProjectBuildTarget, ProjectGem, ProjectServiceTarget, ResolvedGemKind, ResolvedProjectGem,
    SESSION_AUTHORITY_CAPABILITY, gem_manifest_path, gem_validation_warnings, load_gem_manifest,
    load_project_manifest, project_id_from_name, project_lock_path, project_manifest_path,
    project_target_roles, resolve_engine_gems, resolve_engine_root, resolve_project_gems,
    validate_portable_manifest_path, write_gem_manifest, write_project_manifest,
};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Value, table};
use tracing::info;

/// Capability id a provider gem declares (ADR 0026's `AuthProvider` /
/// `CredentialSource` entry-point convention). There is no pre-existing
/// central constant for this one (unlike [`SESSION_AUTHORITY_CAPABILITY`]),
/// so `azoth gem new` templates and `gems/auth-local`/`gems/auth-steam` agree
/// on the literal by convention.
const PROVIDER_CAPABILITY: &str = "provider";

/// Gem id of the `azoth.auth` contract gem (ADR 0026) that provider and
/// session-authority gems declare a version-pinned dependency on.
const AUTH_CONTRACT_GEM_ID: &str = "azoth.auth";

/// `azoth gem new --capability <id>` templates this module can scaffold.
/// `None` (no `--capability` flag) keeps the original generic gem shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewGemCapability {
    /// ADR 0026 `AuthProvider`/`CredentialSource` provider gem.
    Provider,
    /// ADR 0026 `SessionAuthority` gem.
    SessionAuthority,
}

impl NewGemCapability {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Provider => PROVIDER_CAPABILITY,
            Self::SessionAuthority => SESSION_AUTHORITY_CAPABILITY,
        }
    }
}

/// Validate `azoth gem new --capability` selections. `az-project`'s
/// [`az_project::capability`] module and `[services.auth]` validation
/// (`validate_auth_session_authority` in `target_generation.rs`) are the
/// authority on what a gem's `[capabilities.*]` table may contain at large;
/// this CLI only offers templates for the two ADR 0026 entry-point
/// capabilities, so it validates against exactly that pair rather than
/// inventing a broader list.
fn resolve_new_gem_capability(capabilities: &[String]) -> ScaffoldResult<Option<NewGemCapability>> {
    if capabilities.is_empty() {
        return Ok(None);
    }
    if capabilities.len() > 1 {
        return Err(unknown_new_gem_capability_error(capabilities));
    }
    match capabilities[0].as_str() {
        PROVIDER_CAPABILITY => Ok(Some(NewGemCapability::Provider)),
        SESSION_AUTHORITY_CAPABILITY => Ok(Some(NewGemCapability::SessionAuthority)),
        _ => Err(unknown_new_gem_capability_error(capabilities)),
    }
}

fn unknown_new_gem_capability_error(capabilities: &[String]) -> ScaffoldError {
    ScaffoldError::UnknownGemCapability {
        requested: capabilities.join(", "),
        valid: [PROVIDER_CAPABILITY, SESSION_AUTHORITY_CAPABILITY].join(", "),
    }
}

/// Scaffold a new project gem and, unless `register` is false, enable it in
/// the project manifest.
///
/// # Errors
///
/// Returns [`ScaffoldError::InvalidProjectName`] or
/// [`ScaffoldError::InvalidPackageName`] if `name` or `package` is not legal,
/// [`ScaffoldError::UnknownGemCapability`] if `capabilities` names something
/// other than the provider or session-authority capability,
/// [`ScaffoldError::Io`] if the gem tree cannot be written, and
/// [`ScaffoldError::ProjectManifest`] if the resulting manifests do not
/// validate or the refreshed project contract cannot be written.
pub fn new_gem(
    project_path: Option<PathBuf>,
    name: String,
    path: Option<PathBuf>,
    id: Option<String>,
    package: Option<String>,
    register: bool,
    capabilities: &[String],
) -> ScaffoldResult<()> {
    if !new::is_valid_project_name(&name) {
        return Err(ScaffoldError::InvalidProjectName(name));
    }

    let project_path = project_path.unwrap_or_else(|| PathBuf::from("."));
    let package_name = package.unwrap_or_else(|| name.clone());
    if !new::is_valid_project_name(&package_name) {
        return Err(ScaffoldError::InvalidPackageName(package_name));
    }
    let capability = resolve_new_gem_capability(capabilities)?;

    let gem_path = path.unwrap_or_else(|| PathBuf::from("gems").join(&name));
    let gem_path = project_relative_path(&project_path, &gem_path);
    if register {
        let manifest_path = project_relative_or_absolute(&project_path, &gem_path);
        validate_portable_manifest_path("project gem", &manifest_path)?;
    }
    if gem_path.exists() {
        return Err(ScaffoldError::ProjectAlreadyExists(gem_path));
    }

    let gem_id = id.unwrap_or_else(|| {
        let generated_id = project_id_from_name(&name);
        match capability {
            Some(_) => format!("local.{generated_id}"),
            None => generated_id,
        }
    });
    let manifest = match capability {
        None => default_gem_manifest(&gem_id, &name, &package_name),
        Some(NewGemCapability::Provider) => provider_gem_manifest(&gem_id, &name, &package_name),
        Some(NewGemCapability::SessionAuthority) => {
            session_authority_gem_manifest(&gem_id, &name, &package_name)
        }
    };

    info!(
        project_root = %project_path.display(),
        gem_root = %gem_path.display(),
        gem_id,
        package_name,
        capability = capability.map(NewGemCapability::as_str),
        "creating Azoth gem"
    );

    match capability {
        None => create_gem_layout(&gem_path, &name, &package_name, &manifest, register)?,
        Some(capability) => {
            create_capability_gem_layout(
                &gem_path,
                &package_name,
                &manifest,
                register,
                capability,
            )?;
        }
    }

    if register
        && let Err(error) = register_project_gem(&project_path, &gem_path, &gem_id, capabilities)
    {
        cleanup_generated_gem_after_registration_failure(&gem_path, &error)?;
        return Err(error);
    }

    println!("Gem '{}' created successfully.", manifest.gem.name);
    println!("Manifest: {}", gem_path.join("gem.toml").display());
    if register {
        println!(
            "Registered with project: {}",
            project_manifest_path(&project_path).display()
        );
        println!("Lock: {}", project_lock_path(&project_path).display());
    }
    Ok(())
}

/// The generic gem shape keeps runtime, authoring, asset-build, and runtime-host
/// linkage explicit even when one Cargo package implements all four surfaces.
/// Tool and named-service targets are opt-in contracts with different entry
/// points, so a normal gem must never be linked into them implicitly.
fn default_gem_manifest(gem_id: &str, name: &str, package_name: &str) -> GemManifest {
    let mut manifest = GemManifest::new(gem_id, name, "0.1.0");
    manifest.contributions.push(GemContribution::code(
        "runtime",
        package_name,
        [
            GemTargetRole::Game,
            GemTargetRole::P2p,
            GemTargetRole::Client,
            GemTargetRole::Server,
            GemTargetRole::Unified,
            GemTargetRole::HeadlessServer,
        ],
    ));
    manifest.contributions.push(GemContribution::code(
        "authoring",
        package_name,
        [GemTargetRole::ProjectHost],
    ));
    manifest.contributions.push(GemContribution {
        id: "builders".to_string(),
        kind: az_project::GemContributionKind::Builder,
        package: Some(package_name.to_string()),
        root: None,
        mount: None,
        tier: None,
        recursive: None,
        watch: None,
        roles: vec![GemTargetRole::AssetProcessor, GemTargetRole::AssetWorker],
        caps: Vec::new(),
        products: Vec::new(),
    });
    manifest.contributions.push(GemContribution::code(
        "runtime-host",
        package_name,
        [GemTargetRole::RuntimeHost],
    ));
    manifest.contributions.push(GemContribution::assets(
        "assets",
        &manifest.paths.assets,
        [GemTargetRole::AssetProcessor, GemTargetRole::AssetWorker],
    ));
    manifest
        .tools
        .build_targets
        .push(ProjectBuildTarget::package("gem", package_name));
    manifest
}

/// A version-pinned dependency on the `azoth.auth` contract gem, mirroring
/// `gems/auth-local/gem.toml` and `gems/auth-steam/gem.toml`. The dependency
/// graph resolver pulls `azoth.auth` in from the engine catalog automatically
/// (see `az_project::manifest::visit_project_gem`), so the project does not
/// need to separately enable it.
fn auth_contract_dependency() -> GemDependency {
    let mut dependency = GemDependency::new(AUTH_CONTRACT_GEM_ID);
    dependency.version = Some("^0.1.0".to_string());
    dependency
}

/// ADR 0026 provider gem: a single `named-service` code contribution gated by
/// the `provider` capability, mirroring `gems/auth-local/gem.toml` and
/// `gems/auth-steam/gem.toml` exactly (no `client` role contribution — the
/// generated `CredentialSource` skeleton ships in the same `named-service`
/// package, just as `LocalCredentialSource` does in `az-gem-auth-local`).
fn provider_gem_manifest(gem_id: &str, name: &str, package_name: &str) -> GemManifest {
    let mut manifest = GemManifest::new(gem_id, name, "0.1.0");
    manifest.contributions.push(GemContribution::code(
        PROVIDER_CAPABILITY,
        package_name,
        [GemTargetRole::NamedService],
    ));
    manifest.capabilities.insert(
        PROVIDER_CAPABILITY.to_string(),
        GemCapability {
            label: Some("Provider".to_string()),
            description: Some(
                "Auth provider scaffolded by `azoth gem new --capability provider`; see ADR 0026."
                    .to_string(),
            ),
            default: false,
            contributions: vec![PROVIDER_CAPABILITY.to_string()],
            cargo_features: Vec::new(),
            activation: None,
        },
    );
    manifest.dependencies.push(auth_contract_dependency());
    manifest
}

/// ADR 0026 session-authority gem: a single `named-service` code contribution
/// gated by the `session-authority` capability, mirroring the provider gems'
/// shape but keyed on [`SESSION_AUTHORITY_CAPABILITY`].
fn session_authority_gem_manifest(gem_id: &str, name: &str, package_name: &str) -> GemManifest {
    let mut manifest = GemManifest::new(gem_id, name, "0.1.0");
    manifest.contributions.push(GemContribution::code(
        SESSION_AUTHORITY_CAPABILITY,
        package_name,
        [GemTargetRole::NamedService],
    ));
    manifest.capabilities.insert(
        SESSION_AUTHORITY_CAPABILITY.to_string(),
        GemCapability {
            label: Some("Session Authority".to_string()),
            description: Some(
                "Session-authority gem scaffolded by `azoth gem new --capability \
                 session-authority`; see ADR 0026."
                    .to_string(),
            ),
            default: false,
            contributions: vec![SESSION_AUTHORITY_CAPABILITY.to_string()],
            cargo_features: Vec::new(),
            activation: None,
        },
    );
    manifest.dependencies.push(auth_contract_dependency());
    manifest
}

/// Enable an already-authored gem directory in the project manifest.
///
/// # Errors
///
/// Returns [`ScaffoldError::ProjectManifest`] if `path` is not portable
/// relative to the project or the manifests do not validate,
/// [`ScaffoldError::ConfigParse`] if the gem manifest is unreadable or its id
/// does not match an explicitly requested `id`, and [`ScaffoldError::Io`] if
/// the project manifest cannot be rewritten.
pub fn register_existing_gem(
    project_path: Option<PathBuf>,
    path: impl AsRef<Path>,
    id: Option<String>,
    package: Option<String>,
) -> ScaffoldResult<()> {
    let project_path = project_path.unwrap_or_else(|| PathBuf::from("."));
    let gem_path = project_relative_path(&project_path, path.as_ref());
    let relative_gem_path = project_relative_or_absolute(&project_path, &gem_path);
    validate_portable_manifest_path("project gem", &relative_gem_path)?;

    let manifest = load_gem_manifest(&gem_path)?;
    let gem_id = manifest.gem.id.clone();
    if let Some(expected) = id
        && expected != gem_id
    {
        return Err(ScaffoldError::ConfigParse {
            path: gem_manifest_path(&gem_path),
            message: format!(
                "registered gem id `{expected}` does not match gem.toml id `{gem_id}`"
            ),
        });
    }

    let cargo_path = gem_path.join("Cargo.toml");
    let package_name = read_cargo_package_name(&cargo_path)?;
    if let Some(expected) = package
        && expected != package_name
    {
        return Err(ScaffoldError::ConfigParse {
            path: cargo_path,
            message: format!(
                "registered gem package `{expected}` does not match Cargo.toml package `{package_name}`"
            ),
        });
    }

    info!(
        project_root = %project_path.display(),
        gem_root = %gem_path.display(),
        gem_id,
        package_name,
        "registering existing Azoth gem"
    );

    register_project_gem(&project_path, &gem_path, &gem_id, &[])?;

    println!("Gem '{}' registered successfully.", manifest.gem.name);
    println!("Manifest: {}", gem_manifest_path(&gem_path).display());
    println!(
        "Registered with project: {}",
        project_manifest_path(&project_path).display()
    );
    println!("Lock: {}", project_lock_path(&project_path).display());
    Ok(())
}

/// Enable one engine-supplied gem, with the given selected capabilities.
///
/// # Errors
///
/// Returns [`ScaffoldError::ProjectManifest`] if the engine root cannot be
/// resolved or the resulting manifests do not validate,
/// [`ScaffoldError::ConfigParse`] if `id` names no engine gem,
/// [`ScaffoldError::UnknownGemCapability`] if `capabilities` names one the gem
/// does not declare, and [`ScaffoldError::Io`] if the project manifest cannot
/// be rewritten.
pub fn enable_engine_gem(
    project_path: Option<PathBuf>,
    id: String,
    capabilities: Vec<String>,
) -> ScaffoldResult<()> {
    let project_path = project_path.unwrap_or_else(|| PathBuf::from("."));
    let engine_root = resolve_engine_root()?;
    let resolved = resolve_engine_gems(&engine_root)?
        .into_iter()
        .find(|gem| gem.manifest.gem.id == id)
        .ok_or_else(|| ScaffoldError::ConfigParse {
            path: engine_root.join("engine.toml"),
            message: format!("engine gem `{id}` is not registered by engine.toml"),
        })?;

    let package_name = read_cargo_package_name(&resolved.root.join("Cargo.toml"))?;
    info!(
        project_root = %project_path.display(),
        engine_root = %engine_root.display(),
        gem_root = %resolved.root.display(),
        gem_id = %id,
        package_name,
        "enabling Azoth engine gem"
    );

    let mut manifest = load_project_manifest(&project_path)?;
    if let Some(existing) = manifest.gems.iter_mut().find(|gem| gem.id == id) {
        if existing.path.is_some() {
            return Err(ScaffoldError::ConfigParse {
                path: project_manifest_path(&project_path),
                message: format!(
                    "gem `{id}` is already registered as a path-backed project gem; remove its path before enabling the engine gem"
                ),
            });
        }
        existing.enabled = true;
        if !capabilities.is_empty() {
            existing.capabilities = capabilities;
        }
    } else {
        manifest.gems.push(ProjectGem {
            id,
            enabled: true,
            capabilities,
            linkage: None,
            path: None,
        });
    }
    resolve_project_gems(&project_path, &manifest)?;
    write_project_manifest(&project_path, &manifest)?;
    sync_enabled_gem_dependencies(Some(project_path.clone()))?;

    println!(
        "Engine gem '{}' enabled successfully.",
        resolved.manifest.gem.name
    );
    println!(
        "Registered with project: {}",
        project_manifest_path(&project_path).display()
    );
    println!("Lock: {}", project_lock_path(&project_path).display());
    Ok(())
}

/// Print the project's gems — enabled only, or every resolvable gem when
/// `all` is set.
///
/// # Errors
///
/// Returns [`ScaffoldError::ProjectManifest`] if the project or engine
/// manifests cannot be loaded or validated.
pub fn list(project_path: Option<PathBuf>, all: bool) -> ScaffoldResult<()> {
    let project_path = project_path.unwrap_or_else(|| PathBuf::from("."));
    let report = gem_report(&project_path, all)?;

    println!("Project {} gems:", report.project_id);
    if report.gems.is_empty() {
        println!("  none");
        return Ok(());
    }

    for gem in &report.gems {
        println!("  {}", gem.id);
        println!("    enabled: {}", gem.enabled);
        if let Some(path) = &gem.declared_path {
            println!("    path: {}", path.display());
        }
        match &gem.resolved {
            Some(resolved) => {
                println!("    name: {}", resolved.name);
                println!("    version: {}", resolved.version);
                println!("    root: {}", resolved.root.display());
                println!("    manifest: {}", resolved.manifest_path.display());
                println!(
                    "    provenance: {} catalog {} ({})",
                    resolved.home,
                    resolved.catalog_id,
                    resolved.catalog_path.display()
                );
                print_origin(resolved.origin.as_ref());
                print_target_summary("contributions", &resolved.contributions);
                print_target_summary("build_targets", &resolved.build_targets);
                print_target_summary("service_targets", &resolved.service_targets);
            }
            None => {
                println!("    status: disabled");
            }
        }
    }

    Ok(())
}

/// Validate every enabled gem against the project manifest and print a
/// summary.
///
/// # Errors
///
/// Returns [`ScaffoldError::ProjectManifest`] if a gem manifest is missing,
/// unreadable, or fails capability, dependency, or contribution validation.
pub fn validate(project_path: Option<PathBuf>) -> ScaffoldResult<()> {
    let project_path = project_path.unwrap_or_else(|| PathBuf::from("."));
    let report = gem_report(&project_path, false)?;

    println!(
        "Gem validation succeeded for project {} ({} enabled).",
        report.project_id,
        report.gems.len()
    );
    for gem in &report.gems {
        if let Some(resolved) = &gem.resolved {
            println!(
                "  {} -> {} ({})",
                gem.id,
                resolved.root.display(),
                resolved.version
            );
        }
    }
    for warning in &report.warnings {
        println!("  warning: {warning}");
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GemReport {
    project_id: String,
    gems: Vec<GemReportEntry>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GemReportEntry {
    id: String,
    enabled: bool,
    declared_path: Option<PathBuf>,
    resolved: Option<ResolvedGemReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedGemReport {
    name: String,
    version: String,
    root: PathBuf,
    manifest_path: PathBuf,
    home: az_project::GemHome,
    catalog_id: String,
    catalog_path: PathBuf,
    origin: Option<GemOrigin>,
    contributions: Vec<String>,
    build_targets: Vec<String>,
    service_targets: Vec<String>,
}

fn gem_report(project_path: &Path, include_disabled: bool) -> ScaffoldResult<GemReport> {
    let manifest = load_project_manifest(project_path)?;
    let resolved = resolve_project_gems(project_path, &manifest)?;
    let warnings = gem_validation_warnings(&resolved, project_target_roles(&manifest))?
        .into_iter()
        .map(|warning| warning.to_string())
        .collect();
    let resolved_by_id = resolved
        .iter()
        .map(|gem| (gem.manifest.gem.id.clone(), gem))
        .collect::<BTreeMap<_, _>>();

    let gems = manifest
        .gems
        .iter()
        .filter(|gem| include_disabled || gem.enabled)
        .map(|gem| {
            let resolved = resolved_by_id
                .get(&gem.id)
                .map(|resolved| resolved_gem_report(resolved));
            GemReportEntry {
                id: gem.id.clone(),
                enabled: gem.enabled,
                declared_path: gem.path.clone(),
                resolved,
            }
        })
        .collect();

    Ok(GemReport {
        project_id: manifest.project.id,
        gems,
        warnings,
    })
}

fn resolved_gem_report(gem: &ResolvedProjectGem) -> ResolvedGemReport {
    ResolvedGemReport {
        name: gem.manifest.gem.name.clone(),
        version: gem.manifest.gem.version.clone(),
        root: gem.root.clone(),
        manifest_path: gem_manifest_path(&gem.root),
        home: gem.provenance.home,
        catalog_id: gem.provenance.catalog_id.clone(),
        catalog_path: gem.provenance.catalog_path.clone(),
        origin: gem.manifest.gem.origin.clone(),
        contributions: gem
            .manifest
            .contributions
            .iter()
            .map(|contribution| {
                format!(
                    "{}:{}:{}",
                    contribution.id,
                    contribution.kind,
                    contribution
                        .roles
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("|")
                )
            })
            .collect(),
        build_targets: gem
            .manifest
            .tools
            .build_targets
            .iter()
            .map(build_target_label)
            .collect(),
        service_targets: gem
            .manifest
            .tools
            .service_targets
            .iter()
            .map(service_target_label)
            .collect(),
    }
}

fn print_origin(origin: Option<&GemOrigin>) {
    let Some(origin) = origin else {
        println!("    origin: unspecified");
        return;
    };
    println!(
        "    origin: publisher {}, repository {}, license {}, upstream {}",
        origin.publisher.as_deref().unwrap_or("unspecified"),
        origin.repository.as_deref().unwrap_or("unspecified"),
        origin.license.as_deref().unwrap_or("unspecified"),
        origin.upstream.as_deref().unwrap_or("unspecified")
    );
}

fn build_target_label(target: &ProjectBuildTarget) -> String {
    target.package.as_ref().map_or_else(
        || target.name.clone(),
        |package| format!("{}:{}", target.name, package),
    )
}

fn service_target_label(target: &ProjectServiceTarget) -> String {
    format!(
        "{}:{}:{}",
        target.name,
        service_role_label(target),
        target.bin
    )
}

const fn service_role_label(target: &ProjectServiceTarget) -> &'static str {
    match target.role {
        az_project::ProjectServiceRole::ProjectHost => "project-host",
        az_project::ProjectServiceRole::AssetProcessor => "asset-processor",
        az_project::ProjectServiceRole::AssetWorker => "asset-worker",
        az_project::ProjectServiceRole::RuntimeHost => "runtime-host",
    }
}

fn print_target_summary(label: &str, targets: &[String]) {
    if targets.is_empty() {
        println!("    {label}: none");
    } else {
        println!("    {label}: {}", targets.join(", "));
    }
}

fn create_gem_layout(
    gem_path: &Path,
    name: &str,
    package_name: &str,
    manifest: &GemManifest,
    use_workspace_dependencies: bool,
) -> ScaffoldResult<()> {
    std::fs::create_dir_all(gem_path.join(&manifest.paths.assets))?;
    std::fs::create_dir_all(gem_path.join(&manifest.paths.scripts))?;
    std::fs::create_dir_all(gem_path.join("src"))?;

    std::fs::write(
        gem_path.join("Cargo.toml"),
        gem_cargo_toml(package_name, use_workspace_dependencies)?,
    )?;
    std::fs::write(
        gem_path.join("src/lib.rs"),
        gem_lib_rs(name, package_name, manifest),
    )?;
    write_gem_manifest(gem_path, manifest)?;
    Ok(())
}

/// Layout for an ADR 0026 capability gem (`--capability provider` /
/// `--capability session-authority`): just `Cargo.toml`, `gem.toml`, and
/// `src/lib.rs`, mirroring `gems/auth-local` and `gems/auth-steam` exactly —
/// no `azoth/` submodule, asset builder, or prefab scaffolding, since the
/// entry-point convention requires the `auth_provider`/`session_authority`
/// function at the crate root.
fn create_capability_gem_layout(
    gem_path: &Path,
    package_name: &str,
    manifest: &GemManifest,
    use_workspace_dependencies: bool,
    capability: NewGemCapability,
) -> ScaffoldResult<()> {
    std::fs::create_dir_all(gem_path.join("src"))?;

    std::fs::write(
        gem_path.join("Cargo.toml"),
        capability_gem_cargo_toml(package_name, use_workspace_dependencies, capability)?,
    )?;
    let lib_rs = match capability {
        NewGemCapability::Provider => provider_gem_lib_rs(&manifest.gem.id, &manifest.gem.name),
        NewGemCapability::SessionAuthority => session_authority_gem_lib_rs(&manifest.gem.id),
    };
    std::fs::write(gem_path.join("src/lib.rs"), lib_rs)?;
    write_gem_manifest(gem_path, manifest)?;
    Ok(())
}

fn register_project_gem(
    project_path: &Path,
    gem_path: &Path,
    gem_id: &str,
    capabilities: &[String],
) -> ScaffoldResult<()> {
    let snapshot = ProjectRegistrationSnapshot::capture(project_path)?;
    if let Err(error) = register_project_gem_inner(project_path, gem_path, gem_id, capabilities) {
        if let Err(rollback_error) = snapshot.restore() {
            return Err(ScaffoldError::ConfigParse {
                path: project_path.to_path_buf(),
                message: format!(
                    "gem registration failed with `{error}`, and project file rollback failed: {rollback_error}"
                ),
            });
        }
        return Err(error);
    }
    Ok(())
}

/// Register a gem with the project, recording `capabilities` as the
/// project's selection for it — the same shape `enable_engine_gem` uses for
/// engine gems: an existing entry's capabilities are only overwritten when
/// the caller passes a non-empty list, and a new entry is created with them.
fn register_project_gem_inner(
    project_path: &Path,
    gem_path: &Path,
    gem_id: &str,
    capabilities: &[String],
) -> ScaffoldResult<()> {
    let relative_gem_path = project_relative_or_absolute(project_path, gem_path);
    validate_portable_manifest_path("project gem", &relative_gem_path)?;
    let mut manifest = load_project_manifest(project_path)?;
    if let Some(existing) = manifest.gems.iter_mut().find(|gem| gem.id == gem_id) {
        if existing.path.as_ref() != Some(&relative_gem_path) {
            return Err(ScaffoldError::ConfigParse {
                path: project_manifest_path(project_path),
                message: format!(
                    "gem `{gem_id}` is already registered at `{}`",
                    existing
                        .path
                        .as_ref()
                        .map_or_else(|| "<none>".to_string(), |path| path.display().to_string())
                ),
            });
        }
        existing.enabled = true;
        if !capabilities.is_empty() {
            existing.capabilities = capabilities.to_vec();
        }
    } else {
        manifest.gems.push(ProjectGem {
            id: gem_id.to_string(),
            enabled: true,
            capabilities: capabilities.to_vec(),
            linkage: None,
            path: Some(relative_gem_path),
        });
    }

    write_project_manifest(project_path, &manifest)?;
    sync_enabled_gem_dependencies(Some(project_path.to_path_buf()))?;
    Ok(())
}

#[derive(Debug)]
struct ProjectRegistrationSnapshot {
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl ProjectRegistrationSnapshot {
    fn capture(project_path: &Path) -> ScaffoldResult<Self> {
        let files = vec![
            (project_path.join("Cargo.toml"), true),
            (project_manifest_path(project_path), true),
            (project_lock_path(project_path), true),
        ];
        let files = files
            .into_iter()
            .map(|(path, required)| {
                let contents = read_snapshot_file(&path, required)?;
                Ok((path, contents))
            })
            .collect::<ScaffoldResult<Vec<_>>>()?;

        Ok(Self { files })
    }

    fn restore(&self) -> std::io::Result<()> {
        for (path, contents) in &self.files {
            match contents {
                Some(contents) => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(path, contents)?;
                }
                None => match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                },
            }
        }
        Ok(())
    }
}

fn read_snapshot_file(path: &Path, required: bool) -> ScaffoldResult<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn cleanup_generated_gem_after_registration_failure(
    gem_path: &Path,
    registration_error: &ScaffoldError,
) -> ScaffoldResult<()> {
    match std::fs::remove_dir_all(gem_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ScaffoldError::ConfigParse {
            path: gem_path.to_path_buf(),
            message: format!(
                "failed to remove generated gem after registration error `{registration_error}`: {error}"
            ),
        }),
    }
}

/// Repair the project's Cargo view of its enabled gems.
///
/// A primary-gem project owns exactly one Cargo build universe: its authored
/// root workspace plus the generated role-filtered target packages. Enabled
/// project gems become workspace members; everything downstream of that —
/// engine patch projection, generated targets, locks — is the project
/// contract's job, so this delegates rather than editing manifests itself.
/// # Errors
///
/// Returns [`ScaffoldError::LegacyProjectLayout`] if the project declares no
/// `[project].primary_gem`, and any error
/// [`sync_project_contract`](crate::project_contract::sync_project_contract)
/// returns while refreshing the workspace, engine projection, and locks.
pub fn sync_enabled_gem_dependencies(project_path: Option<PathBuf>) -> ScaffoldResult<()> {
    let project_path = project_path.unwrap_or_else(|| PathBuf::from("."));
    let manifest = load_project_manifest(&project_path)?;
    if manifest.project.primary_gem.is_none() {
        return Err(ScaffoldError::LegacyProjectLayout {
            path: project_manifest_path(&project_path),
            reason: "`azoth.toml` declares no `[project].primary_gem`".to_string(),
        });
    }
    let resolved = resolve_project_gems(&project_path, &manifest)?;
    for gem in resolved
        .iter()
        .filter(|gem| gem.kind == ResolvedGemKind::Project && gem.root.join("Cargo.toml").is_file())
    {
        ensure_project_workspace_member(&project_path, &gem.lock_root)?;
    }
    az_project::hydrate_project_local_state(&project_path)?;
    crate::project_contract::sync_project_contract(&project_path)?;
    Ok(())
}

fn ensure_project_workspace_member(
    project_path: &Path,
    relative_gem_path: &Path,
) -> ScaffoldResult<()> {
    let cargo_path = project_path.join("Cargo.toml");
    let text = std::fs::read_to_string(&cargo_path)?;
    let mut document =
        text.parse::<DocumentMut>()
            .map_err(|source| ScaffoldError::ConfigParse {
                path: cargo_path.clone(),
                message: source.to_string(),
            })?;

    if !document.as_table().contains_key("workspace") {
        document.as_table_mut().insert("workspace", table());
    }
    let workspace = document
        .as_table_mut()
        .get_mut("workspace")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| ScaffoldError::ConfigParse {
            path: cargo_path.clone(),
            message: "`workspace` must be a TOML table".to_string(),
        })?;

    let members_item = workspace
        .entry("members")
        .or_insert_with(|| Item::Value(Value::Array(Array::new())));
    let Some(members) = members_item.as_array_mut() else {
        return Err(ScaffoldError::ConfigParse {
            path: cargo_path,
            message: "`workspace.members` must be a TOML array".to_string(),
        });
    };

    let member = path_to_toml(relative_gem_path);
    if !members
        .iter()
        .any(|existing| existing.as_str() == Some(member.as_str()))
    {
        members.push(member);
    }

    let updated = document.to_string();
    if updated != text {
        std::fs::write(cargo_path, updated)?;
    }
    Ok(())
}

/// Cargo manifest for a scaffolded gem: the contract, and nothing else.
///
/// A gem's registration surface is `az-gem-contract` — the attribute, the
/// context, and the registrar. What it registers *into* comes from whichever
/// crate owns that registry entry type, and a gem that has not registered
/// anything yet depends on none of them, so the scaffold does not guess.
fn gem_cargo_toml(package_name: &str, use_workspace_dependencies: bool) -> ScaffoldResult<String> {
    let contract = if use_workspace_dependencies {
        "az-gem-contract = { workspace = true }\n".to_string()
    } else {
        let version = crate::azoth_workspace_crate_version("az-gem-contract")?;
        format!("az-gem-contract = \"{version}\"\n")
    };

    Ok(format!(
        r#"[package]
name = "{package_name}"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"

[dependencies]
{contract}"#
    ))
}

/// `src/lib.rs` for a scaffolded gem: one marked block per contribution the
/// manifest declares for this package.
///
/// Identity, target roles, and the host-capability floor are read from
/// `gem.toml` by the attribute, so nothing below restates them and nothing
/// names the entry item the attribute generates. Where a package implements
/// several contributions — which the default gem does — each block names which
/// stanza it is with a bare token.
fn gem_lib_rs(name: &str, package_name: &str, manifest: &GemManifest) -> String {
    let mut source = format!(
        r"//! Azoth gem crate for {name}.
//!
//! Each block below is one contribution `gem.toml` declares for this package.
//! `#[contribution]` reads that stanza — the gem id, the contribution id, the
//! target roles it composes into, and the host capabilities it requires — and
//! generates the entry item a project's generated target calls, so identity is
//! spelled once, in the manifest.
//!
//! `register` is the gem's own code: reach a typed registry with
//! `ctx.registrar::<T>()`, ask what the project selected with
//! `ctx.activation()`, and take behavior above the declared floor with
//! `ctx.when::<C>(…)`. Rename `_ctx` when a body starts using it.

use az_gem_contract::prelude::*;
"
    );

    for contribution in &manifest.contributions {
        if contribution.package.as_deref() != Some(package_name) {
            continue;
        }
        let roles = contribution
            .roles
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let type_name = pascal_case(&contribution.id);
        let token = contribution.id.replace(['-', '.'], "_");
        // Writing into the buffer avoids the intermediate `String` a
        // `push_str(&format!(..))` would allocate; `fmt::Write` on a `String`
        // is infallible, so the result is discarded.
        let _ = write!(
            source,
            r"
/// Contribution `{id}`, composed into {roles}.
struct {type_name};

#[contribution({token})]
impl Contribution for {type_name} {{
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {{}}
}}
",
            id = contribution.id,
        );
    }

    source
}

/// Cargo manifest for an ADR 0026 capability gem.
///
/// Unlike the generic [`gem_cargo_toml`] it depends directly on `az-gem-auth`
/// (the contract gem's Rust crate, not just its `gem.toml` dependency
/// declaration). It takes no registration dependency beyond the contract: a
/// capability gem registers the way every other gem does, through a marked
/// `#[contribution]` block, so ADR 0026's service conventions and the
/// registration contract are two separate things again — which is what they
/// always were.
fn capability_gem_cargo_toml(
    package_name: &str,
    use_workspace_dependencies: bool,
    capability: NewGemCapability,
) -> ScaffoldResult<String> {
    let dependencies = if use_workspace_dependencies {
        match capability {
            NewGemCapability::Provider => r"az-gem-auth = { workspace = true }
az-gem-contract = { workspace = true }
"
            .to_string(),
            // `SessionAuthority`/`SessionAuthorityContext`/`session_authority`
            // live behind `az-gem-auth`'s `host` feature (see
            // `gems/auth/Cargo.toml`).
            NewGemCapability::SessionAuthority => {
                r#"az-gem-auth = { workspace = true, features = ["host"] }
az-gem-contract = { workspace = true }
"#
                .to_string()
            }
        }
    } else {
        let gem_auth_version = crate::azoth_workspace_crate_version("az-gem-auth")?;
        let contract_version = crate::azoth_workspace_crate_version("az-gem-contract")?;
        match capability {
            NewGemCapability::Provider => format!(
                r#"az-gem-auth = "{gem_auth_version}"
az-gem-contract = "{contract_version}"
"#
            ),
            NewGemCapability::SessionAuthority => format!(
                r#"az-gem-auth = {{ version = "{gem_auth_version}", features = ["host"] }}
az-gem-contract = "{contract_version}"
"#
            ),
        }
    };

    Ok(format!(
        r#"[package]
name = "{package_name}"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"

[dependencies]
{dependencies}"#
    ))
}

/// `PascalCase` a gem name for a generated Rust type name (`weather-gem` ->
/// `WeatherGem`). Non-alphanumeric separators (`-`, `_`) start a new word;
/// `is_valid_project_name` already restricts `name` to a conservative
/// character set.
pub(super) fn pascal_case(name: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if capitalize_next {
                result.extend(ch.to_uppercase());
            } else {
                result.push(ch);
            }
            capitalize_next = false;
        } else {
            capitalize_next = true;
        }
    }
    result
}

/// `src/lib.rs` skeleton for an ADR 0026 provider gem (`--capability
/// provider`). Mirrors `gems/auth-local/src/lib.rs` and
/// `gems/auth-steam/src/lib.rs`: a host-side `AuthProvider`, a client-side
/// `CredentialSource`, and the `auth_provider` entry point at the crate root.
/// `verify_or_exchange`/`acquire` fail closed with
/// [`AuthError::HostConfiguration`] (mirroring `azoth.auth.steam`'s
/// fail-closed shape) until replaced with a real verification backend.
fn provider_gem_lib_rs(gem_id: &str, name: &str) -> String {
    let struct_name = pascal_case(name);
    format!(
        r#"//! `{gem_id}` — an `azoth.auth` provider gem scaffolded by `azoth gem new
//! --capability provider`. See Linear ADR 0026, "Session-authority
//! capability and the entry-point convention", for the full contract.
//!
//! Select it in a project with:
//!
//! ```toml
//! [services.auth]
//! provider = "{gem_id}"
//! ```
//!
//! This is a scaffold: [`{struct_name}AuthProvider::verify_or_exchange`] and
//! [`{struct_name}CredentialSource::acquire`] fail closed with
//! [`AuthError::HostConfiguration`] until replaced with a real verification
//! backend.

use az_gem_auth::credential::{{Audience, CredentialEnvelope, CredentialLease, CredentialScheme}};
use az_gem_auth::identity::ProviderId;
use az_gem_auth::provider::{{AuthProvider, CredentialSource, ProviderCapabilities}};
use az_gem_auth::verification::{{AuthError, VerifiedExternalIdentity}};
use az_gem_contract::prelude::*;

/// The stable provider id. Matches `gem.id` in `gem.toml`.
pub const PROVIDER_ID: &str = "{gem_id}";

/// The host-side provider. Replace `verify_or_exchange` with a real
/// verification backend.
#[derive(Debug, Default)]
pub struct {struct_name}AuthProvider;

impl {struct_name}AuthProvider {{
    #[must_use]
    pub fn new() -> Self {{
        Self
    }}

    fn provider() -> ProviderId {{
        ProviderId::new(PROVIDER_ID)
    }}
}}

impl AuthProvider for {struct_name}AuthProvider {{
    fn capabilities(&self) -> ProviderCapabilities {{
        // TODO: declare the credential scheme(s) this provider accepts.
        ProviderCapabilities::new(Self::provider(), Vec::new())
    }}

    fn verify_or_exchange(
        &self,
        _credential: &CredentialEnvelope,
    ) -> Result<VerifiedExternalIdentity, AuthError> {{
        Err(AuthError::HostConfiguration(
            "{gem_id}::{struct_name}AuthProvider::verify_or_exchange is not implemented yet; \
             replace this scaffold with a real verification backend (ADR 0026)"
                .to_owned(),
        ))
    }}
}}

/// The client-side credential source. Replace `acquire` with real credential
/// acquisition.
#[derive(Debug, Default, Clone)]
pub struct {struct_name}CredentialSource;

impl CredentialSource for {struct_name}CredentialSource {{
    fn acquire(
        &self,
        _scheme: &CredentialScheme,
        _audience: &Audience,
    ) -> Result<(CredentialEnvelope, CredentialLease), AuthError> {{
        Err(AuthError::HostConfiguration(
            "{gem_id}::{struct_name}CredentialSource::acquire is not implemented yet; replace \
             this scaffold with real credential acquisition (ADR 0026)"
                .to_owned(),
        ))
    }}
}}

/// The ADR 0026 provider entry-point convention. The generated auth host
/// calls this when a project selects `{gem_id}` as `[services.auth].provider`.
///
/// # Errors
/// Returns [`AuthError::HostConfiguration`] if this provider cannot be built
/// from `context` alone. This scaffold always succeeds; replace the body once
/// construction needs real configuration (an SDK handle, a live verifier,
/// ...).
pub fn auth_provider(
    _context: &az_gem_auth::AuthProviderContext,
) -> Result<Box<dyn AuthProvider>, AuthError> {{
    Ok(Box::new({struct_name}AuthProvider::new()))
}}

/// The `provider` contribution `gem.toml` declares for this package.
///
/// ADR 0026's entry-point convention above and the registration contract here
/// are two different things: the auth host calls [`auth_provider`] because the
/// project selected this gem as its provider, while this block is how the gem
/// gets composed at all. `#[contribution]` reads the stanza — id, roles,
/// capability floor — so nothing below restates it and no string is spelled
/// twice. `register` is empty because a provider gem contributes behaviour
/// through the entry point rather than entries to a registry; reach a typed
/// registry with `ctx.registrar::<T>()` when that changes.
struct Provider;

#[contribution]
impl Contribution for Provider {{
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {{}}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn auth_provider_entry_point_builds_a_provider() {{
        let context =
            az_gem_auth::AuthProviderContext::new(Audience::new("game"), ProviderId::new(PROVIDER_ID));
        let provider = auth_provider(&context).unwrap();
        assert_eq!(provider.capabilities().provider.as_str(), PROVIDER_ID);
    }}

    #[test]
    fn verify_or_exchange_fails_closed_until_implemented() {{
        let credential = CredentialEnvelope::new(
            CredentialScheme::new("todo"),
            Audience::new("game"),
            b"todo".to_vec(),
        )
        .unwrap();
        assert!(matches!(
            {struct_name}AuthProvider::new().verify_or_exchange(&credential),
            Err(AuthError::HostConfiguration(_))
        ));
    }}
}}
"#
    )
}

/// `src/lib.rs` skeleton for an ADR 0026 session-authority gem (`--capability
/// session-authority`). Delegates straight to the contract gem's reference
/// HS256 authority ([`az_gem_auth::session_authority`]) — the simplest honest
/// skeleton that compiles and actually works, pending a real implementation.
fn session_authority_gem_lib_rs(gem_id: &str) -> String {
    format!(
        r#"//! `{gem_id}` — an `azoth.auth` session-authority gem scaffolded by `azoth
//! gem new --capability session-authority`. See Linear ADR 0026,
//! "Session-authority
//! capability and the entry-point convention", for the full contract.
//!
//! Select it in a project with:
//!
//! ```toml
//! [services.auth]
//! session_authority = "{gem_id}"
//!
//! [[gems]]
//! id = "{gem_id}"
//! enabled = true
//! capabilities = ["session-authority"]
//! ```
//!
//! This scaffold delegates to the contract gem's reference HS256 authority.
//! Replace [`session_authority`] with a real implementation (a different
//! signing algorithm, key rotation, a persistent revocation store, ...) when
//! you have one.

use az_gem_auth::SessionAuthorityContext;
use az_gem_contract::prelude::*;

/// The ADR 0026 session-authority entry-point convention. The generated auth
/// host calls this when a project selects `{gem_id}` as
/// `[services.auth].session_authority`.
///
/// TODO: replace this delegation to `az_gem_auth::session_authority` (the
/// reference HS256 authority) with a real implementation.
#[must_use]
pub fn session_authority(
    context: &SessionAuthorityContext,
) -> Box<dyn az_gem_auth::SessionAuthority> {{
    az_gem_auth::session_authority(context)
}}

/// The `session-authority` contribution `gem.toml` declares for this package.
///
/// ADR 0026's entry-point convention above and the registration contract here
/// are two different things: the auth host calls [`session_authority`] because
/// the project selected this gem as its authority, while this block is how the
/// gem gets composed at all. `#[contribution]` reads the stanza — id, roles,
/// capability floor — so nothing below restates it and no string is spelled
/// twice.
struct SessionAuthority;

#[contribution]
impl Contribution for SessionAuthority {{
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {{}}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn session_authority_entry_point_builds_an_authority() {{
        let context = SessionAuthorityContext::new(
            b"test-signing-key-bytes".to_vec(),
            "issuer",
            "audience",
            900,
            2_592_000,
        );
        let _authority = session_authority(&context);
    }}
}}
"#
    )
}

pub(super) fn gem_authoring_rs(name: &str) -> String {
    let namespace = schema_namespace(name);
    format!(
        r#"//! Gem-authored Bevy components available to prefab sources.

use az_core::reflect::{{EditorFieldAttributes, EditorTypeAttributes, EditorWidget}};
use az_prefab::{{Prefab, ReflectPrefab}};
use bevy::{{
    ecs::{{component::Component, reflect::ReflectComponent}},
    reflect::{{Reflect, ReflectDeserialize, ReflectSerialize, std_traits::ReflectDefault}},
}};
use serde::{{Deserialize, Serialize}};

#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq, Component, Reflect, Prefab, Serialize, Deserialize)]
#[reflect(Component, Default, Prefab, Serialize, Deserialize)]
#[reflect(@EditorTypeAttributes::labeled("Settings")
    .in_group("Gem")
    .with_icon("package")
    .with_description("Editable settings contributed by this gem."))]
#[prefab(tag = "{namespace}.Settings", version = 1)]
pub struct GemSettings {{
    #[reflect(@EditorFieldAttributes::new("Enabled", EditorWidget::Checkbox))]
    pub enabled: bool,

    #[reflect(@EditorFieldAttributes::new("Label", EditorWidget::Default))]
    pub label: String,
}}

/// The reflected Prefab types this gem owns, for its contribution to register.
///
/// A composed value, not a call into a Bevy registry: the host applies what it
/// composed, so two gems claiming one type path collide at compose time with
/// both named, instead of the second quietly overwriting the first.
pub fn prefab_types() -> [az_prefab::PrefabType; 1] {{
    [az_prefab::PrefabType::of::<GemSettings>()]
}}
"#
    )
}

fn schema_namespace(name: &str) -> String {
    let mut namespace = String::from("azoth.gem.");
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            namespace.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' {
            namespace.push('_');
        }
    }
    namespace
}

fn project_relative_path(project_path: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_path.join(path)
    }
}

fn project_relative_or_absolute(project_path: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(project_path)
        .map_or_else(|_| path.to_path_buf(), Path::to_path_buf)
}

fn path_to_toml(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn read_cargo_package_name(cargo_path: &Path) -> ScaffoldResult<String> {
    let text = std::fs::read_to_string(cargo_path)?;
    let document = text
        .parse::<DocumentMut>()
        .map_err(|source| ScaffoldError::ConfigParse {
            path: cargo_path.to_path_buf(),
            message: source.to_string(),
        })?;
    document
        .get("package")
        .and_then(Item::as_table)
        .and_then(|package| package.get("name"))
        .and_then(Item::as_str)
        .map(str::to_string)
        .ok_or_else(|| ScaffoldError::MissingCargoPackageName {
            path: cargo_path.to_path_buf(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest_test_support::assert_no_forbidden_manifest_dependencies;
    use az_project::{
        GemCapability, GemContribution, GemTargetRole, load_project_lock, resolve_project_gems,
    };
    use std::process::Command;

    #[test]
    fn gem_new_creates_path_gem_and_registers_project_linkage() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        new_gem(
            Some(project_root.clone()),
            "weather-gem".to_string(),
            None,
            None,
            None,
            true,
            &[],
        )
        .unwrap();

        let gem_root = project_root.join("gems").join("weather-gem");
        assert_scaffolded_gem_shape(&gem_root);
        assert_scaffolded_gem_sources(&gem_root);
        assert_registered_gem_project_linkage(&project_root, &gem_root);
    }

    /// The manifest a freshly scaffolded gem declares: identity, build target,
    /// and the five contributions with their role sets.
    fn assert_scaffolded_gem_shape(gem_root: &Path) {
        let gem_manifest = load_gem_manifest(gem_root).unwrap();
        assert_eq!(gem_manifest.gem.id, "weather_gem");
        assert_eq!(gem_manifest.tools.build_targets[0].name, "gem");
        assert_eq!(
            gem_manifest
                .contributions
                .iter()
                .map(|contribution| contribution.id.as_str())
                .collect::<Vec<_>>(),
            ["runtime", "authoring", "builders", "runtime-host", "assets"]
        );
        assert!(gem_manifest.contributions.iter().all(|contribution| {
            !contribution.roles.contains(&GemTargetRole::Tool)
                && !contribution.roles.contains(&GemTargetRole::NamedService)
        }));
        let runtime = gem_manifest
            .contributions
            .iter()
            .find(|contribution| contribution.id == "runtime")
            .unwrap();
        assert!(runtime.roles.contains(&GemTargetRole::Client));
        assert!(runtime.roles.contains(&GemTargetRole::Server));
        assert!(!runtime.roles.contains(&GemTargetRole::ProjectHost));
        let authoring = gem_manifest
            .contributions
            .iter()
            .find(|contribution| contribution.id == "authoring")
            .unwrap();
        assert_eq!(authoring.roles, [GemTargetRole::ProjectHost]);

        // Scaffolded gems are schema v3: no legacy source-root toggle, no
        // priority number, and no tier — the gem's home supplies the tier.
        let gem_manifest_text = std::fs::read_to_string(gem_root.join("gem.toml")).unwrap();
        assert!(gem_manifest_text.contains("schema = \"azoth.gem/v3\""));
        assert!(!gem_manifest_text.contains("[source_roots]"));
        assert!(!gem_manifest_text.contains("priority"));
        assert!(!gem_manifest_text.contains("tier"));
        assert!(gem_manifest_text.contains("kind = \"assets\""));
        assert!(gem_manifest_text.contains("mount = \"@assets@\""));
    }

    /// The Cargo manifest and `src/lib.rs` the scaffold writes: the contract
    /// dependency and one marked block per contribution, and nothing from the
    /// retired link-time registration shape.
    fn assert_scaffolded_gem_sources(gem_root: &Path) {
        let gem_cargo_toml = std::fs::read_to_string(gem_root.join("Cargo.toml")).unwrap();
        // A registered gem takes the contract from the project workspace, and
        // takes nothing else: it has registered nothing yet.
        assert!(gem_cargo_toml.contains("az-gem-contract = { workspace = true }"));
        assert!(!gem_cargo_toml.contains("az-gem-link"));
        assert!(!gem_cargo_toml.contains("bevy"));
        assert!(!gem_cargo_toml.contains("/crates/az-"));

        let gem_lib = std::fs::read_to_string(gem_root.join("src/lib.rs")).unwrap();
        assert!(gem_lib.contains("use az_gem_contract::prelude::*;"));
        assert!(gem_lib.contains("#[contribution(runtime)]"));
        assert!(gem_lib.contains("#[contribution(authoring)]"));
        assert!(gem_lib.contains("#[contribution(builders)]"));
        assert!(gem_lib.contains("#[contribution(runtime_host)]"));
        assert!(gem_lib.contains("impl Contribution for Runtime {"));
        assert!(!gem_lib.contains("pub mod azoth;"));
        // The inventory shape is gone with the module it lived in.
        assert!(!gem_root.join("src/azoth").exists());
        assert!(!gem_root.join("src/gems.rs").exists());
    }

    /// Registering a gem adds a workspace member and lock entries, and nothing
    /// else: the gem reaches a host by composing, so there is no project crate
    /// to depend on it and no link bridge to write.
    fn assert_registered_gem_project_linkage(project_root: &Path, gem_root: &Path) {
        let project_manifest = load_project_manifest(project_root).unwrap();
        let resolved = resolve_project_gems(project_root, &project_manifest).unwrap();
        // The project's own primary gem plus the one just registered.
        assert_eq!(
            resolved
                .iter()
                .map(|gem| gem.manifest.gem.id.as_str())
                .collect::<Vec<_>>(),
            ["sample_game.game", "weather_gem"]
        );
        assert!(resolved.iter().any(|gem| gem.root == gem_root));
        let lock = load_project_lock(project_root).unwrap();
        assert_eq!(lock.packages.last().unwrap().id, "weather_gem");
        assert_eq!(
            lock.source_roots.last().unwrap().portable_key.as_str(),
            "gem:weather_gem:assets"
        );
        assert_eq!(
            lock.source_roots.last().unwrap().tier,
            az_project::AssetRootTier::ProjectGem
        );
        assert_eq!(
            lock.tools.build_targets.last().unwrap().owner_id,
            "weather_gem"
        );
        assert!(project_lock_path(project_root).exists());

        let root_cargo_toml = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap();
        assert!(root_cargo_toml.contains("\"gems/weather-gem\""));

        assert!(!project_root.join("crates/game").exists());
        assert!(!project_root.join("crates/asset-processor").exists());
        assert!(!project_root.join("crates/asset-worker").exists());
    }

    #[test]
    fn registered_gem_preserves_primary_gem_topology_layout() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        crate::new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();
        assert!(project_root.join("Cargo.toml").is_file());
        assert!(project_manifest_path(&project_root).is_file());
        assert!(project_lock_path(&project_root).is_file());
        let manifest_text = std::fs::read_to_string(project_manifest_path(&project_root)).unwrap();
        let manifest_value = toml::from_str::<toml::Value>(&manifest_text).unwrap();
        assert_eq!(
            manifest_value
                .get("project")
                .and_then(|project| project.get("primary_gem"))
                .and_then(toml::Value::as_str),
            Some("sample_game.game")
        );
        ProjectRegistrationSnapshot::capture(&project_root).unwrap();

        new_gem(
            Some(project_root.clone()),
            "weather-gem".to_string(),
            None,
            None,
            None,
            true,
            &[],
        )
        .unwrap();

        let manifest = load_project_manifest(&project_root).unwrap();
        assert_eq!(
            manifest.project.primary_gem.as_deref(),
            Some("sample_game.game")
        );
        assert_eq!(
            manifest
                .gems
                .iter()
                .map(|gem| gem.id.as_str())
                .collect::<Vec<_>>(),
            ["sample_game.game", "weather_gem"]
        );
        assert!(project_root.join("gems/sample-game/runtime").is_dir());
        assert!(project_root.join("gems/weather-gem").is_dir());
        assert!(!project_root.join("crates/game").exists());

        let lock = load_project_lock(&project_root).unwrap();
        assert_eq!(lock.packages.len(), 3);
        assert_eq!(lock.packages[1].id, "sample_game.game");
        assert_eq!(lock.packages[2].id, "weather_gem");
    }

    #[test]
    fn sync_records_selected_gem_capabilities_in_the_project_lock() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();
        new_gem(
            Some(project_root.clone()),
            "weather-gem".to_string(),
            None,
            None,
            None,
            true,
            &[],
        )
        .unwrap();

        let gem_root = project_root.join("gems").join("weather-gem");
        let gem_cargo_path = gem_root.join("Cargo.toml");
        let mut gem_cargo = std::fs::read_to_string(&gem_cargo_path).unwrap();
        gem_cargo.push_str(
            "\n[features]\ndefault = []\nruntime = []\nclient-runtime = []\nservice-runtime = []\n",
        );
        std::fs::write(gem_cargo_path, gem_cargo).unwrap();
        let mut gem_manifest = load_gem_manifest(&gem_root).unwrap();
        gem_manifest.contributions.clear();
        gem_manifest.contributions.push(GemContribution::code(
            "client-runtime",
            "weather-gem",
            [GemTargetRole::Client],
        ));
        gem_manifest.contributions.push(GemContribution::code(
            "service-runtime",
            "weather-gem",
            [GemTargetRole::ProjectHost],
        ));
        gem_manifest.capabilities.insert(
            "client-runtime".to_string(),
            capability_for_test(["client-runtime"], ["runtime", "client-runtime"]),
        );
        gem_manifest.capabilities.insert(
            "service-runtime".to_string(),
            capability_for_test(["service-runtime"], ["service-runtime"]),
        );
        write_gem_manifest(&gem_root, &gem_manifest).unwrap();

        let mut project_manifest = load_project_manifest(&project_root).unwrap();
        project_manifest
            .gems
            .iter_mut()
            .find(|gem| gem.id == "weather_gem")
            .expect("registered gem is in the project graph")
            .capabilities = vec!["client-runtime".to_string(), "service-runtime".to_string()];
        write_project_manifest(&project_root, &project_manifest).unwrap();

        sync_enabled_gem_dependencies(Some(project_root.clone())).unwrap();

        let root_cargo_toml = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap();
        assert!(root_cargo_toml.contains("gems/weather-gem"));
        // Capability selection is durable project state, not a Cargo feature
        // lowered into a project-owned service crate: there is no such crate.
        assert!(!project_root.join("crates/game").exists());
        let lock = load_project_lock(&project_root).unwrap();
        let weather = lock
            .packages
            .iter()
            .find(|package| package.id == "weather_gem")
            .expect("registered gem is locked");
        assert_eq!(
            weather.capabilities,
            vec!["client-runtime".to_string(), "service-runtime".to_string()]
        );
    }

    fn capability_for_test<const N: usize, const M: usize>(
        contributions: [&str; N],
        cargo_features: [&str; M],
    ) -> GemCapability {
        GemCapability {
            label: None,
            description: None,
            default: false,
            contributions: contributions.into_iter().map(str::to_string).collect(),
            cargo_features: cargo_features.into_iter().map(str::to_string).collect(),
            activation: None,
        }
    }

    #[test]
    fn registers_existing_project_gem_without_generating_source() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();
        let gem_root = project_root.join("gems").join("sample-domain");
        new_gem(
            Some(project_root.clone()),
            "sample-domain".to_string(),
            Some(PathBuf::from("gems/sample-domain")),
            Some("sample.domain".to_string()),
            Some("sample-domain".to_string()),
            false,
            &[],
        )
        .unwrap();
        let authored_source = "//! project-owned sample authoring\n";
        std::fs::write(gem_root.join("src/lib.rs"), authored_source).unwrap();

        register_existing_gem(
            Some(project_root.clone()),
            PathBuf::from("gems/sample-domain"),
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(gem_root.join("src/lib.rs")).unwrap(),
            authored_source
        );
        let project_manifest = load_project_manifest(&project_root).unwrap();
        let registered = project_manifest
            .gems
            .iter()
            .find(|gem| gem.id == "sample.domain")
            .expect("existing gem is registered in the project graph");
        assert_eq!(
            registered.path.as_deref(),
            Some(Path::new("gems/sample-domain"))
        );
        let lock = load_project_lock(&project_root).unwrap();
        assert!(
            lock.packages
                .iter()
                .any(|package| package.id == "sample.domain")
        );
        let root_cargo_toml = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap();
        assert!(root_cargo_toml.contains("\"gems/sample-domain\""));
        assert!(!project_root.join("crates/game").exists());
    }

    #[test]
    fn existing_gem_registration_rejects_id_or_package_mismatch_before_project_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();
        new_gem(
            Some(project_root.clone()),
            "sample-domain".to_string(),
            Some(PathBuf::from("gems/sample-domain")),
            Some("sample.domain".to_string()),
            Some("sample-domain".to_string()),
            false,
            &[],
        )
        .unwrap();
        let original_manifest = std::fs::read_to_string(project_manifest_path(&project_root))
            .expect("read original project manifest");
        let original_lock = std::fs::read_to_string(project_lock_path(&project_root))
            .expect("read original project lock");

        let id_error = register_existing_gem(
            Some(project_root.clone()),
            PathBuf::from("gems/sample-domain"),
            Some("wrong.id".to_string()),
            None,
        )
        .expect_err("id mismatch should fail");
        assert!(matches!(id_error, ScaffoldError::ConfigParse { .. }));
        assert_eq!(
            std::fs::read_to_string(project_manifest_path(&project_root)).unwrap(),
            original_manifest
        );
        assert_eq!(
            std::fs::read_to_string(project_lock_path(&project_root)).unwrap(),
            original_lock
        );

        let package_error = register_existing_gem(
            Some(project_root.clone()),
            PathBuf::from("gems/sample-domain"),
            None,
            Some("wrong-package".to_string()),
        )
        .expect_err("package mismatch should fail");
        assert!(matches!(package_error, ScaffoldError::ConfigParse { .. }));
        assert_eq!(
            std::fs::read_to_string(project_manifest_path(&project_root)).unwrap(),
            original_manifest
        );
        assert_eq!(
            std::fs::read_to_string(project_lock_path(&project_root)).unwrap(),
            original_lock
        );
    }

    #[test]
    #[ignore = "runs cargo check against a generated project with a registered gem"]
    fn registered_generated_gem_compiles_through_project_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        new_gem(
            Some(project_root.clone()),
            "weather-gem".to_string(),
            None,
            None,
            None,
            true,
            &[],
        )
        .unwrap();

        let status = Command::new("cargo")
            .arg("check")
            .arg("--manifest-path")
            .arg(project_root.join("Cargo.toml"))
            .status()
            .unwrap();

        assert!(status.success());
    }

    #[test]
    #[ignore = "runs cargo check against a generated project with a registered provider gem"]
    fn registered_provider_capability_gem_compiles_through_project_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        new_gem(
            Some(project_root.clone()),
            "acme-provider".to_string(),
            None,
            None,
            None,
            true,
            &["provider".to_string()],
        )
        .unwrap();

        let status = Command::new("cargo")
            .arg("check")
            .arg("--manifest-path")
            .arg(project_root.join("Cargo.toml"))
            .status()
            .unwrap();

        assert!(status.success());
    }

    #[test]
    #[ignore = "runs cargo check against a generated project with a registered session-authority gem"]
    fn registered_session_authority_capability_gem_compiles_through_project_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        new_gem(
            Some(project_root.clone()),
            "acme-authority".to_string(),
            None,
            None,
            None,
            true,
            &["session-authority".to_string()],
        )
        .unwrap();

        let status = Command::new("cargo")
            .arg("check")
            .arg("--manifest-path")
            .arg(project_root.join("Cargo.toml"))
            .status()
            .unwrap();

        assert!(status.success());
    }

    /// The scaffolded manifest depends on the contract and nothing else, and
    /// resolves against the engine workspace by version when it is not a
    /// workspace member.
    #[test]
    fn generated_gem_manifest_keeps_editor_engine_and_service_crates_out() {
        let cargo_toml = gem_cargo_toml("weather_gem", false).unwrap();

        let version = crate::azoth_workspace_crate_version("az-gem-contract").unwrap();
        assert!(cargo_toml.contains(&format!("az-gem-contract = \"{version}\"")));
        assert!(!cargo_toml.contains("az-gem-link"));
        assert!(!cargo_toml.contains(".azoth/engine"));
        assert_dependency_has_no_path(&cargo_toml, "az-gem-contract");
        assert_no_forbidden_manifest_dependencies(
            "generated gem Cargo.toml",
            &cargo_toml,
            FORBIDDEN_GENERATED_GEM_DEPENDENCIES,
            FORBIDDEN_GENERATED_GEM_DEPENDENCY_PREFIXES,
        );

        assert_eq!(
            gem_cargo_toml("weather_gem", true).unwrap(),
            r#"[package]
name = "weather_gem"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"

[dependencies]
az-gem-contract = { workspace = true }
"#
        );
    }

    /// One marked block per contribution the manifest declares for this
    /// package, each naming which stanza it is and restating none of it.
    #[test]
    fn generated_gem_source_marks_every_declared_contribution() {
        let manifest = default_gem_manifest("weather_gem", "weather-gem", "weather-gem");
        let source = gem_lib_rs("weather-gem", "weather-gem", &manifest);

        assert!(source.contains("use az_gem_contract::prelude::*;"));
        assert!(source.contains(
            r"
/// Contribution `runtime`, composed into game, p2p, client, server, unified, headless-server.
struct Runtime;

#[contribution(runtime)]
impl Contribution for Runtime {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}
"
        ));
        assert!(source.contains("#[contribution(authoring)]"));
        assert!(source.contains("#[contribution(builders)]"));
        // The hyphenated id folds to a token, and to the entry item the
        // attribute generates from the same fold.
        assert!(source.contains("#[contribution(runtime_host)]"));
        assert!(source.contains("struct RuntimeHost;"));
        // The assets contribution declares no package, so no crate implements
        // it and no block is it.
        assert!(!source.contains("#[contribution(assets)]"));
        assert_eq!(source.matches("#[contribution(").count(), 4);

        // Identity is the manifest's, and the entry-item convention is the
        // attribute's: the scaffold spells neither.
        for absent in [
            "declare_gem",
            "az_gem_link",
            "pub mod azoth",
            "inventory",
            "_contribution(",
            "ContributionDescriptor",
            "declare_caps",
            "GemId::new",
        ] {
            assert!(
                !source.contains(absent),
                "scaffolded source still has {absent}"
            );
        }
    }

    #[test]
    fn failed_gem_registration_preserves_project_manifest_and_lock() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();
        let original_manifest = std::fs::read_to_string(project_manifest_path(&project_root))
            .expect("read original project manifest");
        let original_lock = std::fs::read_to_string(project_lock_path(&project_root))
            .expect("read original project lock");
        // A root manifest Cargo cannot resolve makes the project contract fail,
        // which happens after `register_project_gem_inner` has already written
        // the project manifest -- exactly the window rollback exists for.
        let root_cargo = project_root.join("Cargo.toml");
        let original_root_cargo = std::fs::read_to_string(&root_cargo).unwrap();
        std::fs::write(
            &root_cargo,
            format!(
                "{original_root_cargo}\n[package]\nname = \"root_package\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
            ),
        )
        .unwrap();

        let error = new_gem(
            Some(project_root.clone()),
            "broken-gem".to_string(),
            None,
            None,
            None,
            true,
            &[],
        )
        .expect_err("a package root should abort registration");

        assert!(
            matches!(error, ScaffoldError::ProjectManifest(_)),
            "{error:?}"
        );
        assert!(
            !project_root.join("gems").join("broken-gem").exists(),
            "failed registered gem creation must clean the generated gem directory"
        );
        assert_eq!(
            std::fs::read_to_string(project_manifest_path(&project_root)).unwrap(),
            original_manifest
        );
        assert_eq!(
            std::fs::read_to_string(project_lock_path(&project_root)).unwrap(),
            original_lock
        );
    }

    #[test]
    fn registered_gem_new_rejects_non_portable_path_before_project_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();
        let original_cargo = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap();
        let original_manifest = std::fs::read_to_string(project_manifest_path(&project_root))
            .expect("read original project manifest");
        let original_lock = std::fs::read_to_string(project_lock_path(&project_root))
            .expect("read original project lock");

        let error = new_gem(
            Some(project_root.clone()),
            "outside-gem".to_string(),
            Some(PathBuf::from("../outside-gem")),
            None,
            None,
            true,
            &[],
        )
        .expect_err("registered gem path must stay portable");

        assert!(matches!(
            error,
            ScaffoldError::ProjectManifest(az_project::ProjectManifestError::InvalidManifestPath {
                field: "project gem",
                ..
            })
        ));
        assert!(!temp.path().join("outside-gem").exists());
        assert_eq!(
            std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap(),
            original_cargo
        );
        assert_eq!(
            std::fs::read_to_string(project_manifest_path(&project_root)).unwrap(),
            original_manifest
        );
        assert_eq!(
            std::fs::read_to_string(project_lock_path(&project_root)).unwrap(),
            original_lock
        );
    }

    #[test]
    fn gem_new_can_create_unregistered_gem() {
        let temp = tempfile::tempdir().unwrap();

        new_gem(
            Some(temp.path().to_path_buf()),
            "standalone-gem".to_string(),
            Some(PathBuf::from("standalone")),
            Some("azoth.standalone".to_string()),
            Some("standalone_gem".to_string()),
            false,
            &[],
        )
        .unwrap();

        let gem_root = temp.path().join("standalone");
        let gem_manifest = load_gem_manifest(&gem_root).unwrap();
        assert_eq!(gem_manifest.gem.id, "azoth.standalone");
        let cargo_toml = std::fs::read_to_string(gem_root.join("Cargo.toml")).unwrap();
        // Outside a project workspace the contract resolves by version.
        let version = crate::azoth_workspace_crate_version("az-gem-contract").unwrap();
        assert!(cargo_toml.contains(&format!("az-gem-contract = \"{version}\"")));
        assert!(!cargo_toml.contains(".azoth/engine"));
        assert_dependency_has_no_path(&cargo_toml, "az-gem-contract");
        assert_no_forbidden_manifest_dependencies(
            "unregistered generated gem Cargo.toml",
            &cargo_toml,
            FORBIDDEN_GENERATED_GEM_DEPENDENCIES,
            FORBIDDEN_GENERATED_GEM_DEPENDENCY_PREFIXES,
        );
        assert!(!temp.path().join("azoth.toml").exists());
    }

    fn assert_dependency_has_no_path(manifest_text: &str, dependency: &str) {
        let manifest = toml::from_str::<toml::Value>(manifest_text).unwrap();
        let dependency = &manifest["dependencies"][dependency];
        assert!(
            dependency.get("path").is_none(),
            "dependency must resolve by version or workspace inheritance: {dependency}"
        );
    }

    #[test]
    fn gem_report_resolves_enabled_gem_manifest_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();
        new_gem(
            Some(project_root.clone()),
            "weather-gem".to_string(),
            None,
            None,
            None,
            true,
            &[],
        )
        .unwrap();

        let report = gem_report(&project_root, false).unwrap();

        assert_eq!(report.project_id, "sample_game");
        assert_eq!(
            report
                .gems
                .iter()
                .map(|gem| gem.id.as_str())
                .collect::<Vec<_>>(),
            ["sample_game.game", "weather_gem"]
        );
        let gem = report
            .gems
            .iter()
            .find(|gem| gem.id == "weather_gem")
            .expect("registered gem is reported");
        assert!(gem.enabled);
        let expected_path = Path::new("gems").join("weather-gem");
        assert_eq!(gem.declared_path.as_deref(), Some(expected_path.as_path()));
        let resolved = gem.resolved.as_ref().unwrap();
        assert_eq!(resolved.name, "weather-gem");
        assert_eq!(resolved.version, "0.1.0");
        assert_eq!(resolved.root, project_root.join("gems").join("weather-gem"));
        assert_eq!(resolved.build_targets, vec!["gem:weather-gem"]);
        assert!(resolved.service_targets.is_empty());
    }

    #[test]
    fn gem_report_hides_disabled_gems_unless_requested() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();
        let mut manifest = load_project_manifest(&project_root).unwrap();
        manifest.gems.push(ProjectGem {
            id: "local.disabled".to_string(),
            enabled: false,
            capabilities: Vec::new(),
            linkage: None,
            path: None,
        });
        write_project_manifest(&project_root, &manifest).unwrap();

        let enabled_only = gem_report(&project_root, false).unwrap();
        let all = gem_report(&project_root, true).unwrap();

        assert!(
            !enabled_only
                .gems
                .iter()
                .any(|gem| gem.id == "local.disabled")
        );
        let disabled = all
            .gems
            .iter()
            .find(|gem| gem.id == "local.disabled")
            .expect("`--all` reports disabled gems");
        assert!(!disabled.enabled);
        assert!(disabled.resolved.is_none());
    }

    const FORBIDDEN_GENERATED_GEM_DEPENDENCIES: &[&str] = &[
        "az-editor",
        "az-editor-ui",
        "az-editor-inspector",
        "az-engine",
        "az-framework",
        "az-daemon",
        "az-session",
        "az-sessiond",
        "az-project-host",
        "az-asset-processor",
    ];
    const FORBIDDEN_GENERATED_GEM_DEPENDENCY_PREFIXES: &[&str] = &[];

    #[test]
    fn gem_new_provider_capability_scaffolds_the_adr_0026_entry_point() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        new_gem(
            Some(project_root.clone()),
            "acme-provider".to_string(),
            None,
            None,
            None,
            true,
            &["provider".to_string()],
        )
        .unwrap();

        let gem_root = project_root.join("gems").join("acme-provider");
        let gem_manifest = load_gem_manifest(&gem_root).unwrap();
        assert_eq!(gem_manifest.gem.id, "local.acme_provider");
        let capability = gem_manifest.capabilities.get("provider").unwrap();
        assert_eq!(capability.contributions, vec!["provider".to_string()]);
        assert!(!capability.default);
        assert_eq!(gem_manifest.contributions.len(), 1);
        assert_eq!(gem_manifest.contributions[0].id, "provider");
        assert_eq!(
            gem_manifest.contributions[0].roles,
            vec![GemTargetRole::NamedService]
        );
        assert_eq!(gem_manifest.dependencies.len(), 1);
        assert_eq!(gem_manifest.dependencies[0].id, "azoth.auth");

        let cargo_toml = std::fs::read_to_string(gem_root.join("Cargo.toml")).unwrap();
        assert!(cargo_toml.contains("az-gem-auth = { workspace = true }"));
        assert!(cargo_toml.contains("az-gem-contract = { workspace = true }"));
        assert!(!cargo_toml.contains("az-gem-link"));
        // Unlike the generic template, a capability gem is not a bevy/asset
        // pipeline crate.
        assert!(!cargo_toml.contains("az-prefab"));
        assert!(!cargo_toml.contains("bevy"));

        let lib_rs = std::fs::read_to_string(gem_root.join("src/lib.rs")).unwrap();
        assert!(lib_rs.contains(
            "pub fn auth_provider(\n    _context: &az_gem_auth::AuthProviderContext,\n) -> Result<Box<dyn AuthProvider>, AuthError> {"
        ));
        assert!(lib_rs.contains("Ok(Box::new(AcmeProviderAuthProvider::new()))"));
        assert!(lib_rs.contains("impl AuthProvider for AcmeProviderAuthProvider {"));
        assert!(lib_rs.contains("impl CredentialSource for AcmeProviderCredentialSource {"));
        assert!(lib_rs.contains("AuthError::HostConfiguration("));
        // The registration contract, not the ADR 0026 service convention:
        // a capability gem is composed through a marked block like every
        // other gem, and names no id of its own doing it.
        assert!(lib_rs.contains(
            "#[contribution]
impl Contribution for Provider {"
        ));
        assert!(!lib_rs.contains("az_gem_link"));
        assert!(lib_rs.contains("[services.auth]"));
        assert!(lib_rs.contains("provider = \"local.acme_provider\""));
        // No `azoth/` submodule scaffolding: the entry point must live at the
        // crate root.
        assert!(!gem_root.join("src/azoth").exists());

        let project_manifest = load_project_manifest(&project_root).unwrap();
        let registered = project_manifest
            .gems
            .iter()
            .find(|gem| gem.id == "local.acme_provider")
            .unwrap();
        assert_eq!(registered.capabilities, vec!["provider".to_string()]);

        // The `azoth.auth` contract gem is pulled in transitively by the
        // dependency graph resolver; the project does not need to separately
        // enable it.
        let resolved = resolve_project_gems(&project_root, &project_manifest).unwrap();
        assert!(
            resolved
                .iter()
                .any(|gem| gem.manifest.gem.id == "azoth.auth")
        );
    }

    #[test]
    fn gem_new_session_authority_capability_scaffolds_the_adr_0026_entry_point() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        new_gem(
            Some(project_root.clone()),
            "acme-authority".to_string(),
            None,
            None,
            None,
            true,
            &["session-authority".to_string()],
        )
        .unwrap();

        let gem_root = project_root.join("gems").join("acme-authority");
        let gem_manifest = load_gem_manifest(&gem_root).unwrap();
        assert_eq!(gem_manifest.gem.id, "local.acme_authority");
        let capability = gem_manifest.capabilities.get("session-authority").unwrap();
        assert_eq!(
            capability.contributions,
            vec!["session-authority".to_string()]
        );
        assert_eq!(gem_manifest.contributions.len(), 1);
        assert_eq!(gem_manifest.contributions[0].id, "session-authority");
        assert_eq!(
            gem_manifest.contributions[0].roles,
            vec![GemTargetRole::NamedService]
        );
        assert_eq!(gem_manifest.dependencies.len(), 1);
        assert_eq!(gem_manifest.dependencies[0].id, "azoth.auth");

        let cargo_toml = std::fs::read_to_string(gem_root.join("Cargo.toml")).unwrap();
        assert!(cargo_toml.contains(r#"az-gem-auth = { workspace = true, features = ["host"] }"#));
        assert!(cargo_toml.contains("az-gem-contract = { workspace = true }"));
        assert!(!cargo_toml.contains("az-gem-link"));

        let lib_rs = std::fs::read_to_string(gem_root.join("src/lib.rs")).unwrap();
        assert!(lib_rs.contains(
            "pub fn session_authority(
    context: &SessionAuthorityContext,
) -> Box<dyn az_gem_auth::SessionAuthority> {"
        ));
        assert!(lib_rs.contains("az_gem_auth::session_authority(context)"));
        assert!(lib_rs.contains(
            "#[contribution]
impl Contribution for SessionAuthority {"
        ));
        assert!(!lib_rs.contains("az_gem_link"));
        assert!(lib_rs.contains("session_authority = \"local.acme_authority\""));
        assert!(lib_rs.contains("capabilities = [\"session-authority\"]"));
        assert!(!gem_root.join("src/azoth").exists());

        let project_manifest = load_project_manifest(&project_root).unwrap();
        let registered = project_manifest
            .gems
            .iter()
            .find(|gem| gem.id == "local.acme_authority")
            .unwrap();
        assert_eq!(
            registered.capabilities,
            vec!["session-authority".to_string()]
        );
    }

    #[test]
    fn gem_new_rejects_unknown_capability() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        let error = new_gem(
            Some(project_root.clone()),
            "bad-gem".to_string(),
            None,
            None,
            None,
            true,
            &["bogus".to_string()],
        )
        .expect_err("unknown capability should be rejected");

        match error {
            ScaffoldError::UnknownGemCapability { requested, valid } => {
                assert_eq!(requested, "bogus");
                assert!(valid.contains("provider"));
                assert!(valid.contains("session-authority"));
            }
            other => panic!("expected UnknownGemCapability, got {other:?}"),
        }
        assert!(!project_root.join("gems").join("bad-gem").exists());
    }

    #[test]
    fn gem_new_rejects_more_than_one_capability() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        let error = new_gem(
            Some(project_root.clone()),
            "both-gem".to_string(),
            None,
            None,
            None,
            true,
            &["provider".to_string(), "session-authority".to_string()],
        )
        .expect_err("more than one capability should be rejected");

        assert!(matches!(error, ScaffoldError::UnknownGemCapability { .. }));
        assert!(!project_root.join("gems").join("both-gem").exists());
    }
}
