use crate::error::{ScaffoldError, ScaffoldResult};
use az_project::{
    GemContribution, GemContributionKind, GemManifest, GemTargetRole, ProjectManifest,
    ProjectTopologyKind, gem_manifest_path, load_gem_manifest, load_project_manifest,
    project_lock_path, project_manifest_path, resolve_project_lock, write_gem_manifest,
    write_project_lock, write_project_manifest,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Value};
use tracing::{info, instrument};

const COMMON_ROLES: [PrimaryGemRole; 3] = [
    PrimaryGemRole::Runtime,
    PrimaryGemRole::Authoring,
    PrimaryGemRole::Builders,
];

pub(crate) const NATIVE_ASSET_TAXONOMY: &[&str] = &[
    "ai",
    "animation/actions",
    "animation/blendspaces",
    "animation/cinematics",
    "animation/controllers",
    "animation/motions",
    "audio/music",
    "audio/wwise/controls",
    "audio/wwise/events",
    "audio/wwise/tags",
    "audio/wwise/tracts",
    "audio/wwise/banks",
    "audio/wwise/media",
    "characters/animated-props",
    "characters/cloth",
    "characters/customization",
    "characters/npc",
    "characters/player",
    "cinematics",
    "config",
    "gameplay/gamedata",
    "input",
    "localization",
    "physics/shapes",
    "rendering/materials",
    "rendering/meshes",
    "rendering/textures",
    "ui/atlases",
    "ui/canvases",
    "ui/fonts",
    "ui/images",
    "ui/prefabs",
    "ui/sprites",
    "ui/video",
    "world/entities",
    "world/levels",
    "world/prefabs",
    "world/regions",
    "world/roads",
    "world/terrain",
    "world/timeofday",
    "world/vegetation",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PrimaryGemRole {
    Runtime,
    Client,
    Server,
    P2p,
    Authoring,
    Builders,
}

impl PrimaryGemRole {
    const fn directory(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Client => "client",
            Self::Server => "server",
            Self::P2p => "p2p",
            Self::Authoring => "authoring",
            Self::Builders => "builders",
        }
    }

    fn package(self, slug: &str) -> String {
        format!("{slug}-{}", self.directory())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyChange {
    pub topology: ProjectTopologyKind,
    pub added_packages: Vec<String>,
    pub inactive_packages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyPrunePlan {
    pub topology: ProjectTopologyKind,
    pub packages: Vec<String>,
}

impl TopologyPrunePlan {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }
}

pub(crate) fn primary_gem_manifest(
    id: &str,
    name: &str,
    slug: &str,
    topology: ProjectTopologyKind,
) -> GemManifest {
    let mut manifest = GemManifest::new(id, name, "0.1.0");
    manifest.contributions = roles_for_topology(topology)
        .iter()
        .copied()
        .map(|role| contribution_for_role(role, slug))
        .collect();
    manifest.contributions.push(GemContribution::assets(
        "assets",
        &manifest.paths.assets,
        [GemTargetRole::AssetProcessor, GemTargetRole::AssetWorker],
    ));
    manifest
}

#[instrument(skip_all, fields(project_root = %project_root.display(), topology = %manifest.topology.kind))]
pub(crate) fn create_primary_project_layout(
    project_root: &Path,
    manifest: &ProjectManifest,
    slug: &str,
) -> ScaffoldResult<()> {
    let primary_gem_id = manifest
        .project
        .primary_gem
        .as_deref()
        .ok_or_else(|| missing_primary_gem(project_root))?;
    let gem_root = project_root.join("gems").join(slug);
    let roles = roles_for_topology(manifest.topology.kind);

    let project_asset_root = manifest
        .effective_asset_roots()
        .into_iter()
        .min_by_key(|root| (root.effective_tier(), root.id.clone()))
        .expect("validated project manifests always resolve at least one asset root");
    let project_assets = project_root.join(&project_asset_root.path);
    create_native_asset_taxonomy(&project_assets)?;
    std::fs::create_dir_all(project_root.join(&manifest.paths.scripts))?;
    std::fs::create_dir_all(project_root.join("gems"))?;
    std::fs::create_dir_all(project_root.join("crates"))?;
    std::fs::create_dir_all(&gem_root)?;
    std::fs::write(
        project_root.join("crates/README.md"),
        crates_placeholder_readme(),
    )?;
    std::fs::write(
        project_root.join("Cargo.toml"),
        root_workspace_manifest(slug, &roles)?,
    )?;

    let gem_manifest = primary_gem_manifest(
        primary_gem_id,
        &manifest.project.name,
        slug,
        manifest.topology.kind,
    );
    let gem_assets = gem_root.join(&gem_manifest.paths.assets);
    std::fs::create_dir_all(&gem_assets)?;
    std::fs::write(gem_assets.join("README.md"), gem_assets_readme())?;
    write_gem_manifest(&gem_root, &gem_manifest)?;
    for role in roles {
        write_new_role_package(&gem_root, primary_gem_id, slug, role)?;
    }
    for (path, contents) in crate::new::repository_metadata_files(project_root) {
        std::fs::write(path, contents)?;
    }
    Ok(())
}

fn create_native_asset_taxonomy(assets_root: &Path) -> ScaffoldResult<()> {
    std::fs::create_dir_all(assets_root)?;
    std::fs::write(assets_root.join("README.md"), project_assets_readme())?;
    for relative in NATIVE_ASSET_TAXONOMY {
        std::fs::create_dir_all(assets_root.join(relative))?;
    }
    Ok(())
}

#[instrument(skip_all, fields(project_root = %project_root.as_ref().display(), topology = %topology))]
/// Move the project onto `topology`, scaffolding whatever role packages it
/// newly requires.
///
/// # Errors
///
/// Returns [`ScaffoldError::ProjectManifest`] if the project is not a
/// primary-gem project or its manifests do not validate,
/// [`ScaffoldError::Io`] if a role package cannot be written, and any error
/// [`sync_project_contract`](crate::project_contract::sync_project_contract)
/// returns while refreshing derived state.
pub fn set(
    project_root: impl AsRef<Path>,
    topology: ProjectTopologyKind,
) -> ScaffoldResult<TopologyChange> {
    let project_root = project_root.as_ref();
    let mut context = PrimaryGemContext::load(project_root)?;
    let destination_roles = roles_for_topology(topology);
    let existing_roles = contribution_roles(&context.gem_manifest, &context.slug);
    let missing_roles = destination_roles
        .iter()
        .copied()
        .filter(|role| !existing_roles.contains(role))
        .collect::<Vec<_>>();

    for role in &missing_roles {
        preflight_role_package(&context.gem_root, &context.slug, *role)?;
        preflight_contribution(&context.gem_manifest, &context.slug, *role)?;
    }

    for role in &missing_roles {
        write_new_role_package(
            &context.gem_root,
            &context.primary_gem_id,
            &context.slug,
            *role,
        )?;
        context
            .gem_manifest
            .contributions
            .push(contribution_for_role(*role, &context.slug));
    }
    add_workspace_members(project_root, &context.slug, &missing_roles)?;
    write_gem_manifest(&context.gem_root, &context.gem_manifest)?;

    context.project_manifest.topology.kind = topology;
    write_project_manifest(project_root, &context.project_manifest)?;
    refresh_derived_project_state(project_root, &context.project_manifest)?;

    let destination = destination_roles.into_iter().collect::<BTreeSet<_>>();
    let inactive_packages = contribution_roles(&context.gem_manifest, &context.slug)
        .difference(&destination)
        .map(|role| role.package(&context.slug))
        .collect();
    let added_packages = missing_roles
        .iter()
        .map(|role| role.package(&context.slug))
        .collect();
    info!(
        ?added_packages,
        ?inactive_packages,
        "updated project topology scaffold"
    );
    Ok(TopologyChange {
        topology,
        added_packages,
        inactive_packages,
    })
}

/// List the role packages the current topology no longer activates, without
/// removing anything.
///
/// # Errors
///
/// Returns [`ScaffoldError::ProjectManifest`] if the project is not a
/// primary-gem project or its manifests cannot be loaded.
pub fn plan_prune(project_root: impl AsRef<Path>) -> ScaffoldResult<TopologyPrunePlan> {
    let project_root = project_root.as_ref();
    let context = PrimaryGemContext::load(project_root)?;
    let active = roles_for_topology(context.project_manifest.topology.kind)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let packages = contribution_roles(&context.gem_manifest, &context.slug)
        .difference(&active)
        .map(|role| role.package(&context.slug))
        .collect();
    Ok(TopologyPrunePlan {
        topology: context.project_manifest.topology.kind,
        packages,
    })
}

#[instrument(skip_all, fields(project_root = %project_root.as_ref().display()))]
/// Remove the role packages the current topology no longer activates.
///
/// # Errors
///
/// Returns [`ScaffoldError::TopologyPruneConfirmationRequired`] unless
/// `confirmed` is set, [`ScaffoldError::TopologyPruneUnsafe`] if a package
/// directory holds authored sources it will not delete,
/// [`ScaffoldError::Io`] if removal fails, and
/// [`ScaffoldError::ProjectManifest`] if the manifests cannot be loaded or the
/// refreshed contract does not validate.
pub fn prune(project_root: impl AsRef<Path>, confirmed: bool) -> ScaffoldResult<TopologyPrunePlan> {
    let project_root = project_root.as_ref();
    let mut context = PrimaryGemContext::load(project_root)?;
    let active = roles_for_topology(context.project_manifest.topology.kind)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let roles = contribution_roles(&context.gem_manifest, &context.slug)
        .difference(&active)
        .copied()
        .collect::<Vec<_>>();
    let plan = TopologyPrunePlan {
        topology: context.project_manifest.topology.kind,
        packages: roles
            .iter()
            .map(|role| role.package(&context.slug))
            .collect(),
    };
    if plan.is_empty() {
        return Ok(plan);
    }
    if !confirmed {
        return Err(ScaffoldError::TopologyPruneConfirmationRequired {
            packages: plan.packages,
        });
    }

    for role in &roles {
        verify_prunable_role(project_root, &context, *role)?;
    }
    for role in &roles {
        let role_root = context.gem_root.join(role.directory());
        debug_assert!(role_root.starts_with(&context.gem_root));
        std::fs::remove_dir_all(&role_root)?;
    }
    let removed_ids = roles
        .iter()
        .map(|role| role.directory())
        .collect::<BTreeSet<_>>();
    context
        .gem_manifest
        .contributions
        .retain(|contribution| !removed_ids.contains(contribution.id.as_str()));
    remove_workspace_members(project_root, &context.slug, &roles)?;
    write_gem_manifest(&context.gem_root, &context.gem_manifest)?;
    refresh_derived_project_state(project_root, &context.project_manifest)?;
    info!(packages = ?plan.packages, "pruned inactive topology role packages");
    Ok(plan)
}

struct PrimaryGemContext {
    project_manifest: ProjectManifest,
    gem_manifest: GemManifest,
    primary_gem_id: String,
    gem_root: PathBuf,
    slug: String,
}

impl PrimaryGemContext {
    fn load(project_root: &Path) -> ScaffoldResult<Self> {
        let project_manifest = load_project_manifest(project_root)?;
        let primary_gem_id = project_manifest
            .project
            .primary_gem
            .clone()
            .ok_or_else(|| missing_primary_gem(project_root))?;
        let declaration = project_manifest
            .gems
            .iter()
            .find(|gem| gem.id == primary_gem_id && gem.enabled)
            .ok_or_else(|| ScaffoldError::ConfigParse {
                path: project_manifest_path(project_root),
                message: format!(
                    "primary project gem `{primary_gem_id}` must be an enabled [[gems]] declaration"
                ),
            })?;
        let relative_root =
            declaration
                .path
                .as_ref()
                .ok_or_else(|| ScaffoldError::ConfigParse {
                    path: project_manifest_path(project_root),
                    message: format!(
                        "primary project gem `{primary_gem_id}` must use a project-relative path"
                    ),
                })?;
        let gem_root = project_root.join(relative_root);
        let slug = gem_root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| crate::new::is_valid_project_name(name))
            .ok_or_else(|| ScaffoldError::ConfigParse {
                path: gem_root.clone(),
                message: "primary gem directory name must be a portable Cargo package slug"
                    .to_string(),
            })?
            .to_string();
        let gem_manifest = load_gem_manifest(&gem_root)?;
        if gem_manifest.gem.id != primary_gem_id {
            return Err(ScaffoldError::ConfigParse {
                path: gem_manifest_path(&gem_root),
                message: format!(
                    "primary project gem id `{primary_gem_id}` does not match gem manifest id `{}`",
                    gem_manifest.gem.id
                ),
            });
        }
        Ok(Self {
            project_manifest,
            gem_manifest,
            primary_gem_id,
            gem_root,
            slug,
        })
    }
}

fn roles_for_topology(topology: ProjectTopologyKind) -> Vec<PrimaryGemRole> {
    let mut roles = vec![PrimaryGemRole::Runtime];
    match topology {
        ProjectTopologyKind::SinglePlayer => {}
        ProjectTopologyKind::MultiplayerClientServer => {
            roles.extend([PrimaryGemRole::Client, PrimaryGemRole::Server]);
        }
        ProjectTopologyKind::MultiplayerPeerToPeer => roles.push(PrimaryGemRole::P2p),
    }
    roles.extend([PrimaryGemRole::Authoring, PrimaryGemRole::Builders]);
    roles
}

fn contribution_roles(manifest: &GemManifest, slug: &str) -> BTreeSet<PrimaryGemRole> {
    COMMON_ROLES
        .into_iter()
        .chain([
            PrimaryGemRole::Client,
            PrimaryGemRole::Server,
            PrimaryGemRole::P2p,
        ])
        .filter(|role| {
            let expected = contribution_for_role(*role, slug);
            manifest
                .contributions
                .iter()
                .any(|actual| actual == &expected)
        })
        .collect()
}

fn contribution_for_role(role: PrimaryGemRole, slug: &str) -> GemContribution {
    let package = role.package(slug);
    let roles = match role {
        PrimaryGemRole::Runtime => vec![
            GemTargetRole::Game,
            GemTargetRole::Client,
            GemTargetRole::Server,
            GemTargetRole::Unified,
            GemTargetRole::HeadlessServer,
            GemTargetRole::RuntimeHost,
        ],
        PrimaryGemRole::Client => vec![GemTargetRole::Client, GemTargetRole::Unified],
        PrimaryGemRole::Server => vec![
            GemTargetRole::Server,
            GemTargetRole::Unified,
            GemTargetRole::HeadlessServer,
        ],
        PrimaryGemRole::P2p => vec![GemTargetRole::Game, GemTargetRole::P2p],
        PrimaryGemRole::Authoring => vec![GemTargetRole::ProjectHost],
        PrimaryGemRole::Builders => {
            return GemContribution {
                id: role.directory().to_string(),
                kind: GemContributionKind::Builder,
                package: Some(package),
                root: None,
                mount: None,
                tier: None,
                products: Vec::new(),
                recursive: None,
                watch: None,
                roles: vec![GemTargetRole::AssetProcessor, GemTargetRole::AssetWorker],
                caps: Vec::new(),
            };
        }
    };
    GemContribution::code(role.directory(), package, roles)
}

fn preflight_contribution(
    manifest: &GemManifest,
    slug: &str,
    role: PrimaryGemRole,
) -> ScaffoldResult<()> {
    let Some(actual) = manifest
        .contributions
        .iter()
        .find(|contribution| contribution.id == role.directory())
    else {
        return Ok(());
    };
    let expected = contribution_for_role(role, slug);
    if actual == &expected {
        Ok(())
    } else {
        Err(ScaffoldError::ConfigParse {
            path: PathBuf::from("gem.toml"),
            message: format!(
                "primary gem contribution `{}` collides with the ADR-0025 role declaration",
                role.directory()
            ),
        })
    }
}

fn preflight_role_package(gem_root: &Path, slug: &str, role: PrimaryGemRole) -> ScaffoldResult<()> {
    let role_root = gem_root.join(role.directory());
    if !role_root.exists() {
        return Ok(());
    }
    let cargo_path = role_root.join("Cargo.toml");
    let text =
        std::fs::read_to_string(&cargo_path).map_err(|error| ScaffoldError::ConfigParse {
            path: role_root.clone(),
            message: format!(
                "destination role directory exists but is not a complete Cargo package: {error}"
            ),
        })?;
    let document = text
        .parse::<toml::Table>()
        .map_err(|error| ScaffoldError::ConfigParse {
            path: cargo_path.clone(),
            message: error.to_string(),
        })?;
    let actual = document
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str);
    let expected = role.package(slug);
    if actual == Some(expected.as_str()) && role_root.join("src/lib.rs").is_file() {
        Ok(())
    } else {
        Err(ScaffoldError::ConfigParse {
            path: role_root,
            message: format!(
                "destination role directory collides with package `{expected}`; move or resolve it before changing topology"
            ),
        })
    }
}

fn write_new_role_package(
    gem_root: &Path,
    gem_id: &str,
    slug: &str,
    role: PrimaryGemRole,
) -> ScaffoldResult<()> {
    let role_root = gem_root.join(role.directory());
    if role_root.exists() {
        return Ok(());
    }
    for (relative_path, contents) in role_files(gem_id, slug, role) {
        let path = role_root.join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
    }
    Ok(())
}

fn role_files(gem_id: &str, slug: &str, role: PrimaryGemRole) -> Vec<(PathBuf, String)> {
    let package = role.package(slug);
    let mut files = vec![(
        PathBuf::from("Cargo.toml"),
        role_cargo_toml(slug, role, &package),
    )];
    match role {
        PrimaryGemRole::Runtime => {
            files.push((
                PathBuf::from("src/lib.rs"),
                runtime_lib_rs(gem_id, &package),
            ));
            files.push((
                PathBuf::from("src/graphs.rs"),
                crate::new::project_graphs_rs()
                    .replace("crate::azoth::graphs::", "crate::graphs::"),
            ));
            files.push((
                PathBuf::from("src/runtime.rs"),
                crate::new::project_runtime_rs().to_string(),
            ));
        }
        PrimaryGemRole::Authoring => {
            files.push((PathBuf::from("src/lib.rs"), authoring_lib_rs(gem_id, slug)));
        }
        PrimaryGemRole::Builders => {
            files.push((PathBuf::from("src/lib.rs"), builders_lib_rs(gem_id)));
            files.push((
                PathBuf::from("src/assets.rs"),
                crate::new::project_assets_rs().to_string(),
            ));
        }
        PrimaryGemRole::Client | PrimaryGemRole::Server | PrimaryGemRole::P2p => files.push((
            PathBuf::from("src/lib.rs"),
            network_role_lib_rs(gem_id, slug, role),
        )),
    }
    files
}

fn role_cargo_toml(slug: &str, role: PrimaryGemRole, package: &str) -> String {
    let dependencies = match role {
        PrimaryGemRole::Runtime => r"az-core = { workspace = true }
az-gem-contract = { workspace = true }
az-graph-builder = { workspace = true }
az-graph-runtime = { workspace = true }
az-node-graph = { workspace = true }
az-runtime-app = { workspace = true }
az-runtime-host = { workspace = true }
uuid = { workspace = true }
"
        .to_string(),
        PrimaryGemRole::Authoring => r"az-core = { workspace = true }
az-gem-contract = { workspace = true }
az-prefab = { workspace = true }
bevy = { workspace = true }
serde = { workspace = true }
"
        .to_string(),
        // The four engine builder crates this role used to name were
        // force-link imports, not dependencies: engine builders reach a worker
        // through the composed `az-engine-builders` floor. What a project
        // builders role needs is the `BuildRule` surface itself.
        PrimaryGemRole::Builders => format!(
            r#"az-asset-builder = {{ workspace = true }}
az-gem-contract = {{ workspace = true }}
{slug}-authoring = {{ path = "../authoring" }}
"#
        ),
        PrimaryGemRole::Client | PrimaryGemRole::P2p => format!(
            r#"az-gem-contract = {{ workspace = true }}
az-runtime-app = {{ workspace = true, features = ["windowed"] }}
{slug}-runtime = {{ path = "../runtime" }}
"#
        ),
        PrimaryGemRole::Server => format!(
            r#"az-gem-contract = {{ workspace = true }}
az-runtime-app = {{ workspace = true }}
{slug}-runtime = {{ path = "../runtime" }}
"#
        ),
    };
    format!(
        r#"[package]
name = "{package}"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"

[dependencies]
{dependencies}"#
    )
}

fn runtime_lib_rs(_gem_id: &str, package: &str) -> String {
    format!(
        r#"//! Runtime contribution for the primary project gem.
//!
//! `#[contribution]` reads the `runtime` stanza in this gem's `gem.toml` — the
//! gem id, the contribution id, the target roles it composes into — and
//! generates the entry item a generated target calls. Nothing below restates
//! any of that, and nothing anchors anything: the named call in the generated
//! glue is what links this crate now.

pub mod graphs;
pub mod runtime;

use az_gem_contract::prelude::*;

struct Runtime;

#[contribution(runtime)]
impl Contribution for Runtime {{
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {{
        ctx.registrar::<az_node_graph::NodeTypeRegistration>()
            .register_many(graphs::node_types());
        ctx.registrar::<az_node_graph::GraphTypeRegistration>()
            .register_many(graphs::graph_types());
        ctx.registrar::<az_runtime_host::RuntimeProjectionRegistration>()
            .register_many(runtime::projections());

        // App configuration is only meaningful where the host owns a world, so
        // it is narrowed to rather than declared as a floor: this contribution
        // still composes into `runtime-host`, which owns none, and a declined
        // narrowing is named in the compose report instead of vanishing.
        ctx.when::<az_gem_contract::HostsWorld>(|ctx| {{
            ctx.registrar::<az_runtime_app::RuntimeAppContributionRegistration>()
                .register(az_runtime_app::RuntimeAppContributionRegistration::new(
                    PACKAGE_NAME,
                    // `RuntimeAppConfigurer`: run once the engine owns the app
                    // and the merged command line is parsed.
                    |_app, _context, _settings| Ok(()),
                ));
        }});
    }}
}}

/// Cargo package carrying the topology-independent runtime contribution.
pub const PACKAGE_NAME: &str = "{package}";
"#
    )
}

fn authoring_lib_rs(_gem_id: &str, slug: &str) -> String {
    let name = slug.replace('-', " ");
    let authored = crate::gem::gem_authoring_rs(&name);
    format!(
        r"{authored}
use az_gem_contract::prelude::*;

struct Authoring;

#[contribution(authoring)]
impl Contribution for Authoring {{
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {{
        ctx.registrar::<az_prefab::PrefabType>()
            .register_many(prefab_types());
    }}
}}
"
    )
}

fn builders_lib_rs(_gem_id: &str) -> String {
    r"//! Asset-builder contribution for the primary project gem.

pub mod assets;

use az_gem_contract::prelude::*;

struct Builders;

#[contribution(builders)]
impl Contribution for Builders {
    /// Empty until this gem declares a source schema or a build rule; reach a
    /// typed registry with `ctx.registrar::<T>()` when it does.
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}
"
    .to_string()
}

fn network_role_lib_rs(_gem_id: &str, slug: &str, role: PrimaryGemRole) -> String {
    let crate_name = format!("{}_runtime", slug.replace('-', "_"));
    let role_name = role.directory();
    let type_name = crate::gem::pascal_case(role_name);
    format!(
        r#"//! `{role_name}` contribution for the primary project gem.
//!
//! Which targets this composes into is the `{role_name}` stanza in `gem.toml`,
//! read by `#[contribution]`. There is no second role list here: the
//! composition *is* the role closure, since generation only builds a target's
//! glue from role-matching contributions.

pub use {crate_name} as runtime;

use az_gem_contract::prelude::*;

struct {type_name};

#[contribution({role_name})]
impl Contribution for {type_name} {{
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {{
        // Priority 100 so this layer configures after the base runtime
        // contribution has installed its plugins; ties fall to composition
        // order, which is the resolved lock closure.
        ctx.when::<az_gem_contract::HostsWorld>(|ctx| {{
            ctx.registrar::<az_runtime_app::RuntimeAppContributionRegistration>()
                .register(
                    az_runtime_app::RuntimeAppContributionRegistration::new(
                        "{slug}-{role_name}",
                        |_app, _context, _settings| Ok(()),
                    )
                    .with_priority(100),
                );
        }});
    }}
}}
"#
    )
}

fn root_workspace_manifest(slug: &str, roles: &[PrimaryGemRole]) -> ScaffoldResult<String> {
    let members = roles
        .iter()
        .map(|role| format!("    \"{}\",", workspace_member(slug, *role)))
        .collect::<Vec<_>>()
        .join("\n");
    let default_role = roles
        .iter()
        .copied()
        .find(|role| *role == PrimaryGemRole::Server)
        .or_else(|| {
            roles
                .iter()
                .copied()
                .find(|role| *role == PrimaryGemRole::P2p)
        })
        .unwrap_or(PrimaryGemRole::Runtime);
    let default_member = workspace_member(slug, default_role);
    Ok(format!(
        r#"[workspace]
members = [
{members}
]
default-members = [
    "{default_member}",
]
resolver = "3"

[workspace.dependencies]
{dependencies}

{dev_profile}
"#,
        dependencies = crate::new::workspace_dependencies_toml()?,
        dev_profile = crate::new::workspace_dev_profile_toml(),
    ))
}

fn workspace_member(slug: &str, role: PrimaryGemRole) -> String {
    format!("gems/{slug}/{}", role.directory())
}

fn add_workspace_members(
    project_root: &Path,
    slug: &str,
    roles: &[PrimaryGemRole],
) -> ScaffoldResult<()> {
    edit_workspace_members(project_root, |members| {
        for role in roles {
            let member = workspace_member(slug, *role);
            if !members
                .iter()
                .any(|existing| existing.as_str() == Some(member.as_str()))
            {
                members.push(member);
            }
        }
    })
}

fn remove_workspace_members(
    project_root: &Path,
    slug: &str,
    roles: &[PrimaryGemRole],
) -> ScaffoldResult<()> {
    let removed = roles
        .iter()
        .map(|role| workspace_member(slug, *role))
        .collect::<BTreeSet<_>>();
    edit_workspace_members(project_root, |members| {
        let retained = members
            .iter()
            .filter_map(|item| item.as_str())
            .filter(|member| !removed.contains(*member))
            .map(str::to_string)
            .collect::<Vec<_>>();
        members.clear();
        for member in retained {
            members.push(member);
        }
    })
}

fn edit_workspace_members(
    project_root: &Path,
    edit: impl FnOnce(&mut Array),
) -> ScaffoldResult<()> {
    let path = project_root.join("Cargo.toml");
    let text = std::fs::read_to_string(&path)?;
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|error| ScaffoldError::ConfigParse {
            path: path.clone(),
            message: error.to_string(),
        })?;
    let members = document
        .get_mut("workspace")
        .and_then(Item::as_table_mut)
        .and_then(|workspace| workspace.get_mut("members"))
        .and_then(Item::as_value_mut)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ScaffoldError::ConfigParse {
            path: path.clone(),
            message: "project Cargo.toml must declare workspace.members as an array".to_string(),
        })?;
    edit(members);
    let updated = document.to_string();
    if updated != text {
        std::fs::write(path, updated)?;
    }
    Ok(())
}

fn verify_prunable_role(
    project_root: &Path,
    context: &PrimaryGemContext,
    role: PrimaryGemRole,
) -> ScaffoldResult<()> {
    let role_root = context.gem_root.join(role.directory());
    let expected_files = role_files(&context.primary_gem_id, &context.slug, role);
    let actual_files = collect_files(&role_root)?;
    let expected_paths = expected_files
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<BTreeSet<_>>();
    if actual_files != expected_paths {
        return Err(ScaffoldError::TopologyPruneUnsafe {
            path: role_root,
            reason: "package contains added, removed, or renamed files".to_string(),
        });
    }
    for (relative_path, expected) in expected_files {
        let path = role_root.join(relative_path);
        if std::fs::read_to_string(&path)? != expected {
            return Err(ScaffoldError::TopologyPruneUnsafe {
                path,
                reason: "package contains authored modifications".to_string(),
            });
        }
    }
    let package = role.package(&context.slug);
    if let Some(path) = find_package_reference(project_root, &role_root, &package)? {
        return Err(ScaffoldError::TopologyPruneUnsafe {
            path,
            reason: format!("package `{package}` is referenced by another Cargo manifest"),
        });
    }
    Ok(())
}

fn collect_files(root: &Path) -> ScaffoldResult<BTreeSet<PathBuf>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeSet<PathBuf>) -> ScaffoldResult<()> {
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                visit(root, &path, files)?;
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .expect("visited files remain beneath the role root")
                        .to_path_buf(),
                );
            }
        }
        Ok(())
    }

    let mut files = BTreeSet::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn find_package_reference(
    project_root: &Path,
    ignored_root: &Path,
    package: &str,
) -> ScaffoldResult<Option<PathBuf>> {
    fn visit(
        project_root: &Path,
        directory: &Path,
        ignored_root: &Path,
        package: &str,
    ) -> ScaffoldResult<Option<PathBuf>> {
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path();
            if path == ignored_root
                || path.starts_with(project_root.join("target"))
                || path.starts_with(project_root.join(".azoth"))
            {
                continue;
            }
            if path.is_dir() {
                if let Some(reference) = visit(project_root, &path, ignored_root, package)? {
                    return Ok(Some(reference));
                }
            } else if path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml")
                && path != project_root.join("Cargo.toml")
                && std::fs::read_to_string(&path)?.contains(package)
            {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    visit(project_root, project_root, ignored_root, package)
}

fn refresh_derived_project_state(
    project_root: &Path,
    manifest: &ProjectManifest,
) -> ScaffoldResult<()> {
    az_project::sync_project_engine_patch_table(project_root)?;
    let lock = resolve_project_lock(project_root, manifest)?;
    write_project_lock(project_root, &lock)?;
    debug_assert!(project_lock_path(project_root).is_file());
    Ok(())
}

fn missing_primary_gem(project_root: &Path) -> ScaffoldError {
    ScaffoldError::ConfigParse {
        path: project_manifest_path(project_root),
        message: "project has no [project].primary_gem; migrate the crates/game-era project before changing topology"
            .to_string(),
    }
}

const fn crates_placeholder_readme() -> &'static str {
    "# Project libraries\n\nPlace independently testable domain libraries here. Project runtime, authoring, and builder roles belong to the primary project gem under `gems/`.\n"
}

const fn project_assets_readme() -> &'static str {
    "# Project assets\n\nThis root contributes native authoring sources to `@assets@`. Keep sources lowercase and organized by the scaffolded domain taxonomy.\n"
}

const fn gem_assets_readme() -> &'static str {
    "# Primary gem assets\n\nPlace reusable primary-gem authoring sources here. They share the project `@assets@` namespace, so every virtual path must remain unique.\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_filesystem::AzothDataHome;
    use az_project::{
        GemValidationWarning, GeneratedTargetsFreshness, GeneratedTargetsSyncStatus,
        ensure_project_generated_targets, gem_validation_warnings, load_project_manifest,
        project_generated_targets_status, refresh_project_lock, resolve_project_gems,
    };

    fn scaffold(topology: ProjectTopologyKind) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("topology-game");
        crate::new::execute_with_options(
            "TopologyGame".to_string(),
            Some(project_root.clone()),
            crate::new::ProjectCreateOptions {
                topology,
                ..Default::default()
            },
        )
        .unwrap();
        (temp, project_root)
    }

    #[test]
    fn primary_gem_contribution_matrix_matches_topology_roles() {
        let cases = [
            (
                ProjectTopologyKind::SinglePlayer,
                vec!["runtime", "authoring", "builders"],
            ),
            (
                ProjectTopologyKind::MultiplayerClientServer,
                vec!["runtime", "client", "server", "authoring", "builders"],
            ),
            (
                ProjectTopologyKind::MultiplayerPeerToPeer,
                vec!["runtime", "p2p", "authoring", "builders"],
            ),
        ];

        for (topology, expected) in cases {
            let manifest = primary_gem_manifest("local.test", "Test", "test", topology);
            let mut expected = expected;
            expected.push("assets");
            assert_eq!(
                manifest
                    .contributions
                    .iter()
                    .map(|contribution| contribution.id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            manifest.validate().unwrap();
        }
    }

    #[test]
    fn scaffold_matrix_generates_exact_primary_gem_role_packages() {
        let cases = [
            (
                ProjectTopologyKind::SinglePlayer,
                vec!["runtime", "authoring", "builders"],
            ),
            (
                ProjectTopologyKind::MultiplayerClientServer,
                vec!["runtime", "client", "server", "authoring", "builders"],
            ),
            (
                ProjectTopologyKind::MultiplayerPeerToPeer,
                vec!["runtime", "p2p", "authoring", "builders"],
            ),
        ];

        for (topology, expected_roles) in cases {
            let (_temp, project_root) = scaffold(topology);
            let gem_root = project_root.join("gems/topologygame");
            let actual_roles = std::fs::read_dir(&gem_root)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<BTreeSet<_>>();
            let mut expected_directories = expected_roles
                .iter()
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>();
            expected_directories.insert("assets".to_string());
            assert_eq!(actual_roles, expected_directories);
            assert!(!project_root.join("crates/game").exists());
            assert!(project_root.join("crates/README.md").is_file());

            let gem = load_gem_manifest(&gem_root).unwrap();
            let mut expected_contributions = expected_roles.clone();
            expected_contributions.push("assets");
            assert_eq!(
                gem.contributions
                    .iter()
                    .map(|contribution| contribution.id.as_str())
                    .collect::<Vec<_>>(),
                expected_contributions
            );
            assert!(
                gem.contributions
                    .iter()
                    .filter(|contribution| contribution.kind != GemContributionKind::Assets)
                    .all(|contribution| contribution.package.is_some())
            );
            let assets = gem
                .contributions
                .iter()
                .find(|contribution| contribution.kind == GemContributionKind::Assets)
                .unwrap();
            assert_eq!(assets.root.as_deref(), Some(Path::new("assets")));
            assert!(gem_root.join("assets/README.md").is_file());
        }
    }

    #[test]
    fn new_project_scaffolds_complete_native_asset_taxonomy() {
        let (_temp, project_root) = scaffold(ProjectTopologyKind::SinglePlayer);
        let assets = project_root.join("assets");

        assert!(assets.join("README.md").is_file());
        for relative in NATIVE_ASSET_TAXONOMY {
            assert!(assets.join(relative).is_dir(), "missing assets/{relative}");
        }
        for forbidden in [
            "animations",
            "coatgen",
            "engineassets",
            "libs",
            "materials",
            "meshes",
            "objects",
            "sharedassets",
            "slices",
            "sounds",
            "textures",
            "assets-native",
        ] {
            assert!(!assets.join(forbidden).exists());
        }
    }

    #[test]
    #[ignore = "acceptance test spawns nested Cargo for three generated workspaces"]
    fn generated_target_projection_matrix_is_exact_and_portable() {
        // Every ship crate links the engine floor as well, and the same three
        // bundles every time: the six world-hosting roles name `types`,
        // `assets` and `runtime`, and none of them names `builders`. Stated
        // once here rather than repeated into each row, because the matrix
        // below is about what the *project* contributes.
        const FLOOR: [&str; 3] = ["az-engine-assets", "az-engine-runtime", "az-engine-types"];

        // Ship crates only (ADR 0040): the three service hosts and the dev
        // launchers are prebuilt engine binaries that load gems instead, so
        // nothing generates a crate for them.
        let cases = [
            (
                ProjectTopologyKind::SinglePlayer,
                vec![("game", vec!["topologygame-runtime"])],
            ),
            (
                ProjectTopologyKind::MultiplayerClientServer,
                vec![
                    (
                        "client",
                        vec!["topologygame-runtime", "topologygame-client"],
                    ),
                    (
                        "server",
                        vec!["topologygame-runtime", "topologygame-server"],
                    ),
                    (
                        "unified",
                        vec![
                            "topologygame-runtime",
                            "topologygame-client",
                            "topologygame-server",
                        ],
                    ),
                    (
                        "headless-server",
                        vec!["topologygame-runtime", "topologygame-server"],
                    ),
                ],
            ),
            (
                ProjectTopologyKind::MultiplayerPeerToPeer,
                vec![("game", vec!["topologygame-runtime", "topologygame-p2p"])],
            ),
        ];

        for (topology, expected) in cases {
            let (_temp, project_root) = scaffold(topology);
            let report = ensure_project_generated_targets(&project_root).unwrap();
            assert_eq!(report.status, GeneratedTargetsSyncStatus::Regenerated);
            let workspace_root = report.workspace_root.unwrap();
            assert_eq!(workspace_root, project_root.join(".azoth/targets"));
            assert!(project_root.join("Cargo.lock").is_file());
            assert!(workspace_root.join("generation.json").is_file());
            assert_generated_manifests_are_portable(&workspace_root);

            // Composition order is the resolved lock closure, which the
            // per-target set below does not pin; the set is what this matrix
            // is about.
            let actual = report
                .targets
                .iter()
                .map(|target| {
                    (
                        target.name.as_str(),
                        target
                            .linked_packages
                            .iter()
                            .map(String::as_str)
                            .collect::<BTreeSet<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            let expected = expected
                .iter()
                .map(|(name, packages)| {
                    (
                        *name,
                        packages
                            .iter()
                            .copied()
                            .chain(FLOOR)
                            .collect::<BTreeSet<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
            let mut expected_directories = expected
                .iter()
                .map(|(name, _)| (*name).to_string())
                .collect::<BTreeSet<_>>();
            expected_directories.insert("_bootstrap".to_string());
            let actual_directories = std::fs::read_dir(&workspace_root)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().unwrap().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<BTreeSet<_>>();
            assert_eq!(actual_directories, expected_directories);

            // Seven per-role load manifests ride alongside the ship crates as
            // flat files (ADR 0040), each digested in `generation.json`.
            assert_eq!(report.manifests.len(), 7);
            for manifest in &report.manifests {
                let path = workspace_root.join(format!("{}.manifest.json", manifest.role));
                assert!(path.is_file(), "missing load manifest {}", path.display());
                assert!(!manifest.digest.is_empty());
            }
            let generation =
                std::fs::read_to_string(workspace_root.join("generation.json")).unwrap();
            assert!(generation.contains("\"schema\": 8"));
            assert!(generation.contains("\"manifests\""));
            assert!(!generation.contains("service"));
        }
    }

    #[test]
    #[ignore = "acceptance test performs repeated generated-workspace Cargo resolution"]
    fn generated_target_fingerprint_tracks_inputs_and_recovers_after_clean() {
        let (_temp, project_root) = scaffold(ProjectTopologyKind::SinglePlayer);
        let first = ensure_project_generated_targets(&project_root).unwrap();
        assert_eq!(first.status, GeneratedTargetsSyncStatus::Regenerated);
        let first_fingerprint = first.fingerprint.unwrap();
        assert_eq!(
            ensure_project_generated_targets(&project_root)
                .unwrap()
                .status,
            GeneratedTargetsSyncStatus::Unchanged
        );

        set(&project_root, ProjectTopologyKind::MultiplayerClientServer).unwrap();
        assert_eq!(
            project_generated_targets_status(&project_root)
                .unwrap()
                .freshness,
            GeneratedTargetsFreshness::Stale
        );
        let topology = ensure_project_generated_targets(&project_root).unwrap();
        assert_eq!(topology.status, GeneratedTargetsSyncStatus::Regenerated);
        assert_ne!(
            topology.fingerprint.as_deref(),
            Some(first_fingerprint.as_str())
        );
        assert!(
            topology
                .workspace_root
                .as_ref()
                .unwrap()
                .join("headless-server")
                .is_dir()
        );

        let gem_manifest = project_root.join("gems/topologygame/gem.toml");
        let mut gem = std::fs::read_to_string(&gem_manifest).unwrap();
        gem.push_str("\n# fingerprint input mutation\n");
        std::fs::write(&gem_manifest, gem).unwrap();
        refresh_project_lock(&project_root).unwrap();
        assert_eq!(
            project_generated_targets_status(&project_root)
                .unwrap()
                .freshness,
            GeneratedTargetsFreshness::Stale
        );
        assert_eq!(
            ensure_project_generated_targets(&project_root)
                .unwrap()
                .status,
            GeneratedTargetsSyncStatus::Regenerated
        );

        let generated_root = topology.workspace_root.unwrap();
        std::fs::remove_dir_all(&generated_root).unwrap();
        assert_eq!(
            project_generated_targets_status(&project_root)
                .unwrap()
                .freshness,
            GeneratedTargetsFreshness::Missing
        );
        let recovered = ensure_project_generated_targets(&project_root).unwrap();
        assert_eq!(recovered.status, GeneratedTargetsSyncStatus::Regenerated);
        assert!(
            recovered
                .workspace_root
                .unwrap()
                .join("generation.json")
                .is_file()
        );
    }

    #[test]
    #[ignore = "acceptance test performs generated-workspace Cargo resolution"]
    fn cargo_clean_preserves_azoth_data_and_generated_targets_regenerate() {
        let (temp, project_root) = scaffold(ProjectTopologyKind::SinglePlayer);
        let manifest = az_project::load_project_manifest(&project_root).unwrap();
        let paths = AzothDataHome::new(temp.path().join("machine-home"))
            .project(&manifest.project.name, &project_root);
        std::fs::create_dir_all(paths.asset_db_dir()).unwrap();
        std::fs::create_dir_all(paths.default_product_cache_dir()).unwrap();
        std::fs::write(paths.asset_db_path(), b"durable asset db").unwrap();
        std::fs::write(
            paths.default_product_cache_dir().join("product.bin"),
            b"expensive cooked product",
        )
        .unwrap();

        let generated = ensure_project_generated_targets(&project_root).unwrap();
        let target_directory = generated.target_directory;
        paths.ensure_outside_target_dir(&target_directory).unwrap();
        assert!(!paths.asset_db_path().starts_with(&target_directory));
        assert!(
            !paths
                .default_product_cache_dir()
                .starts_with(&target_directory)
        );

        std::fs::remove_dir_all(&target_directory).unwrap();

        assert_eq!(
            std::fs::read(paths.asset_db_path()).unwrap(),
            b"durable asset db"
        );
        assert_eq!(
            std::fs::read(paths.default_product_cache_dir().join("product.bin")).unwrap(),
            b"expensive cooked product"
        );
        let regenerated = ensure_project_generated_targets(&project_root).unwrap();
        assert_eq!(regenerated.status, GeneratedTargetsSyncStatus::Unchanged);
        assert!(
            regenerated
                .workspace_root
                .unwrap()
                .join("generation.json")
                .is_file()
        );
    }

    fn assert_generated_manifests_are_portable(workspace_root: &Path) {
        let project_root = workspace_root.parent().unwrap().parent().unwrap();
        let root_manifest = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap();
        let root_manifest = toml::from_str::<toml::Value>(&root_manifest).unwrap();
        let members = root_manifest
            .get("workspace")
            .and_then(|workspace| workspace.get("members"))
            .and_then(toml::Value::as_array)
            .unwrap();
        assert!(
            members
                .iter()
                .all(|member| { member.as_str() != Some(".azoth/targets/*") })
        );
        for entry in std::fs::read_dir(workspace_root).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_dir() {
                continue;
            }
            let package_manifest =
                std::fs::read_to_string(entry.path().join("Cargo.toml")).unwrap();
            let package_manifest = toml::from_str::<toml::Value>(&package_manifest).unwrap();
            assert!(package_manifest.get("workspace").is_some());
            assert!(entry.path().join("Cargo.lock").is_file());
            assert!(entry.path().join(".cargo/config.toml").is_file());
            for dependency in package_manifest
                .get("dependencies")
                .and_then(toml::Value::as_table)
                .into_iter()
                .flat_map(toml::map::Map::values)
            {
                if let Some(path) = dependency.get("path").and_then(toml::Value::as_str) {
                    assert!(
                        !Path::new(path).is_absolute(),
                        "absolute dependency path: {path}"
                    );
                }
            }
        }
    }

    #[test]
    fn graphical_primary_gems_opt_into_windowed_runtime_composition() {
        for role in [PrimaryGemRole::Client, PrimaryGemRole::P2p] {
            let manifest = role_cargo_toml("test", role, "test-role");
            assert!(
                manifest
                    .contains("az-runtime-app = { workspace = true, features = [\"windowed\"] }"),
                "{role:?} must opt into windowed runtime composition"
            );
        }

        let server = role_cargo_toml("test", PrimaryGemRole::Server, "test-server");
        assert!(
            server.contains("az-runtime-app = { workspace = true }"),
            "server must retain the headless runtime surface"
        );
        assert!(
            !server.contains("windowed"),
            "server must not activate the graphical feature"
        );
    }

    #[test]
    fn topology_workspaces_choose_a_focused_native_default() {
        for (topology, expected) in [
            (ProjectTopologyKind::SinglePlayer, "gems/test/runtime"),
            (
                ProjectTopologyKind::MultiplayerClientServer,
                "gems/test/server",
            ),
            (ProjectTopologyKind::MultiplayerPeerToPeer, "gems/test/p2p"),
        ] {
            let manifest = root_workspace_manifest("test", &roles_for_topology(topology)).unwrap();
            assert!(
                manifest.contains(&format!("default-members = [\n    \"{expected}\",\n]")),
                "{topology:?} workspace did not select {expected}:\n{manifest}"
            );
            assert!(manifest.contains("[profile.dev]\nopt-level = 1"));
            assert!(manifest.contains("[profile.dev.package.\"*\"]\nopt-level = 3"));
        }
    }

    #[test]
    fn topology_set_is_additive_and_prune_removes_only_inactive_skeletons() {
        let (_temp, project_root) = scaffold(ProjectTopologyKind::SinglePlayer);
        let runtime_lib = project_root.join("gems/topologygame/runtime/src/lib.rs");
        let mut authored_runtime = std::fs::read_to_string(&runtime_lib).unwrap();
        authored_runtime.push_str("\npub const AUTHORED_SENTINEL: bool = true;\n");
        std::fs::write(&runtime_lib, &authored_runtime).unwrap();

        let change = set(&project_root, ProjectTopologyKind::MultiplayerClientServer).unwrap();
        assert_eq!(
            change.added_packages,
            vec![
                "topologygame-client".to_string(),
                "topologygame-server".to_string()
            ]
        );
        assert_eq!(
            std::fs::read_to_string(&runtime_lib).unwrap(),
            authored_runtime
        );

        let change = set(&project_root, ProjectTopologyKind::SinglePlayer).unwrap();
        assert_eq!(
            change.inactive_packages,
            vec![
                "topologygame-client".to_string(),
                "topologygame-server".to_string()
            ]
        );
        assert!(project_root.join("gems/topologygame/client").is_dir());
        assert!(project_root.join("gems/topologygame/server").is_dir());

        let error = prune(&project_root, false).unwrap_err();
        assert!(matches!(
            error,
            ScaffoldError::TopologyPruneConfirmationRequired { .. }
        ));
        let outcome = prune(&project_root, true).unwrap();
        assert_eq!(outcome.packages, change.inactive_packages);
        assert!(!project_root.join("gems/topologygame/client").exists());
        assert!(!project_root.join("gems/topologygame/server").exists());
        assert_eq!(
            std::fs::read_to_string(runtime_lib).unwrap(),
            authored_runtime
        );
    }

    #[test]
    fn single_player_topology_warns_for_client_only_enabled_gem() {
        let (_temp, project_root) = scaffold(ProjectTopologyKind::SinglePlayer);
        let client_root = project_root.join("gems/client-only");
        let mut client = GemManifest::new("local.client-only", "Client Only", "0.1.0");
        client.contributions.push(GemContribution::code(
            "client",
            "client-only",
            [GemTargetRole::Client],
        ));
        write_gem_manifest(&client_root, &client).unwrap();

        let mut manifest = load_project_manifest(&project_root).unwrap();
        manifest.gems.push(az_project::ProjectGem {
            id: "local.client-only".to_string(),
            enabled: true,
            capabilities: Vec::new(),
            linkage: None,
            path: Some(PathBuf::from("gems/client-only")),
        });
        let resolved = resolve_project_gems(&project_root, &manifest).unwrap();
        let warnings =
            gem_validation_warnings(&resolved, az_project::project_target_roles(&manifest))
                .unwrap();

        assert!(
            warnings.contains(&GemValidationWarning::InertForPresentRoles {
                gem_id: "local.client-only".to_string(),
                present_roles: az_project::project_target_roles(&manifest)
                    .into_iter()
                    .collect(),
            })
        );
    }
}
