use crate::error::{ScaffoldError, ScaffoldResult};
use az_project::{
    LoreSourceControlConfig, ProjectGem, ProjectManifest, ProjectTopologyKind,
    project_id_from_name, project_lock_path, write_project_manifest,
};
use az_source_control::{CreateRepositoryRequest, LoreCli, SourceControlProvider, StageMode};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tracing::info;

pub(super) const PROJECT_GEMS_DIR: &str = "gems";
pub const INITIAL_PROJECT_COMMIT_MESSAGE: &str = "Initialize Azoth project";
pub const PROJECT_WORKFLOW_UPDATE_COMMIT_MESSAGE: &str = "Update Azoth project workflow";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectCreateOptions {
    pub lore_url: Option<String>,
    pub enabled_engine_gems: Vec<String>,
    pub topology: ProjectTopologyKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoreRepositorySetup {
    pub remote_url: String,
    pub description: Option<String>,
    pub use_shared_store: bool,
}

impl LoreRepositorySetup {
    #[must_use]
    pub fn new(remote_url: impl Into<String>) -> Self {
        Self {
            remote_url: remote_url.into(),
            description: None,
            use_shared_store: true,
        }
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Create a new project with default options.
///
/// # Errors
///
/// Returns any error [`execute_with_options`] returns.
pub fn execute(
    name: String,
    path: Option<PathBuf>,
    lore_url: Option<String>,
) -> ScaffoldResult<()> {
    execute_with_options(
        name,
        path,
        ProjectCreateOptions {
            lore_url,
            enabled_engine_gems: Vec::new(),
            topology: ProjectTopologyKind::default(),
        },
    )
}

/// Create a new project, choosing its topology, enabled engine gems, and
/// optional Lore remote.
///
/// # Errors
///
/// Returns [`ScaffoldError::InvalidProjectName`] if `name` is not a legal
/// project name, [`ScaffoldError::ProjectAlreadyExists`] if the destination
/// already holds a project, [`ScaffoldError::Io`] if the tree cannot be
/// written, [`ScaffoldError::ProjectManifest`] if the generated manifests do
/// not validate or the project contract cannot be synchronized, and
/// [`ScaffoldError::SourceControl`] if the requested Lore repository cannot be
/// created or checkpointed.
pub fn execute_with_options(
    name: String,
    path: Option<PathBuf>,
    options: ProjectCreateOptions,
) -> ScaffoldResult<()> {
    info!("Creating new AZoth project: {}", name);

    if !is_valid_project_name(&name) {
        return Err(ScaffoldError::InvalidProjectName(name));
    }

    let project_path = path.unwrap_or_else(|| PathBuf::from(&name));
    if project_path.exists() && !project_path_is_empty_dir(&project_path)? {
        return Err(ScaffoldError::ProjectAlreadyExists(project_path));
    }

    info!("Project path: {}", project_path.display());
    info!(lore_url = ?options.lore_url, "Lore repository setup requested");

    let manifest = scaffold_primary_project(&project_path, &name, &options)?;

    let setup = options
        .lore_url
        .map(LoreRepositorySetup::new)
        .map(|setup| setup.description(format!("Azoth project {}", manifest.project.name)));
    let source_control_state = ensure_project_lore_checkpoint(
        &project_path,
        INITIAL_PROJECT_COMMIT_MESSAGE,
        setup.as_ref(),
    )?;

    println!("Project '{}' created successfully.", manifest.project.name);
    println!("Manifest: {}", project_path.join("azoth.toml").display());
    println!("Lock: {}", project_lock_path(&project_path).display());
    println!("Next steps:");
    println!("  cd {}", project_path.display());
    print_project_workflow_next_steps(source_control_state, INITIAL_PROJECT_COMMIT_MESSAGE, None);

    Ok(())
}

/// Write the primary-gem layout and every piece of engine-owned state derived
/// from it.
///
/// The one place a project layout is produced: `azoth project new` calls it
/// with an empty directory, `azoth project init` calls it with a directory
/// that already holds authored content. Neither has a second layout to fall
/// back to.
pub(super) fn scaffold_primary_project(
    project_path: &Path,
    name: &str,
    options: &ProjectCreateOptions,
) -> ScaffoldResult<ProjectManifest> {
    let slug = project_slug(name);
    let mut manifest = topology_project_manifest(name, &slug, options.topology);
    apply_enabled_engine_gems(&mut manifest, &options.enabled_engine_gems);
    if let Some(remote_url) = &options.lore_url {
        manifest.source_control.lore = Some(LoreSourceControlConfig::new(remote_url));
    }

    create_project_layout(project_path, &manifest, &slug)?;
    write_project_manifest(project_path, &manifest)?;
    crate::project_contract::sync_project_contract(project_path)?;
    Ok(manifest)
}

fn apply_enabled_engine_gems(manifest: &mut ProjectManifest, gem_ids: &[String]) {
    manifest.gems.extend(
        gem_ids
            .iter()
            .filter_map(|id| {
                let id = id.trim();
                (!id.is_empty()).then(|| id.to_string())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|id| ProjectGem {
                id,
                enabled: true,
                capabilities: Vec::new(),
                linkage: None,
                path: None,
            }),
    );
}

fn project_path_is_empty_dir(project_path: &Path) -> ScaffoldResult<bool> {
    if !project_path.is_dir() {
        return Ok(false);
    }
    Ok(std::fs::read_dir(project_path)?.next().is_none())
}

fn topology_project_manifest(
    name: &str,
    slug: &str,
    topology: ProjectTopologyKind,
) -> ProjectManifest {
    let project_id = project_id_from_name(name);
    let primary_gem_id = format!("{project_id}.game");
    let mut manifest = ProjectManifest::new(project_id, name, env!("CARGO_PKG_VERSION"));
    manifest.project.primary_gem = Some(primary_gem_id.clone());
    manifest.topology.kind = topology;
    manifest.gems.push(ProjectGem {
        id: primary_gem_id,
        enabled: true,
        capabilities: Vec::new(),
        linkage: None,
        path: Some(PathBuf::from(PROJECT_GEMS_DIR).join(slug)),
    });
    manifest
}

fn create_project_layout(
    project_path: &Path,
    manifest: &ProjectManifest,
    slug: &str,
) -> ScaffoldResult<()> {
    crate::topology::create_primary_project_layout(project_path, manifest, slug)
}

pub(super) fn project_slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_was_separator = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if matches!(ch, '_' | '-' | ' ') && !last_was_separator && !slug.is_empty() {
            slug.push('-');
            last_was_separator = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "project".to_string()
    } else {
        slug
    }
}

pub(super) const PROJECT_ENGINE_WORKSPACE_CRATES: &[&str] = &[
    "az-animation",
    "az-gem-animation",
    "az-asset-builder",
    "az-asset-processor",
    "az-asset-worker",
    "az-core",
    "az-filesystem",
    "az-gem-auth",
    // The registration surface every scaffolded gem depends on.
    "az-gem-contract",
    "az-graph-builder",
    "az-graph-runtime",
    "az-material",
    "az-material-builder",
    "az-node-graph",
    "az-prefab",
    "az-prefab-builder",
    "az-project-host",
    "az-runtime-app",
    "az-runtime-host",
    "az-scene",
    "az-service-entrypoint",
    "az-terrain",
    "az-terrain-builder",
    "az-terrain-runtime",
    "az-texture-builder",
];

pub(super) const PROJECT_ENGINE_BUILD_WORKSPACE_CRATES: &[&str] =
    &["az-filesystem", "az-graph-builder", "az-project"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProjectSharedWorkspaceDependency {
    pub name: &'static str,
    pub requirement: &'static str,
    pub features: &'static [&'static str],
}

pub(super) const PROJECT_SHARED_WORKSPACE_DEPENDENCIES: &[ProjectSharedWorkspaceDependency] = &[
    ProjectSharedWorkspaceDependency {
        name: "clap",
        requirement: "4.6",
        features: &["derive"],
    },
    ProjectSharedWorkspaceDependency {
        name: "tracing",
        requirement: "0.1",
        features: &[],
    },
    ProjectSharedWorkspaceDependency {
        name: "uuid",
        requirement: "1.23",
        features: &["v4", "v7", "serde"],
    },
];

pub(super) const PROJECT_SHARED_WORKSPACE_ONLY_DEPENDENCIES: &[ProjectSharedWorkspaceDependency] =
    &[
        ProjectSharedWorkspaceDependency {
            name: "ron",
            requirement: "0.12",
            features: &[],
        },
        ProjectSharedWorkspaceDependency {
            name: "serde",
            requirement: "1.0",
            features: &["derive"],
        },
    ];

pub(super) const PROJECT_SHARED_BUILD_WORKSPACE_DEPENDENCIES:
    &[ProjectSharedWorkspaceDependency] = &[ProjectSharedWorkspaceDependency {
    name: "toml",
    requirement: "1",
    features: &[],
}];

pub(super) const fn workspace_dev_profile_toml() -> &'static str {
    r#"# Match the Azoth engine workspace: light opts on project crates for
# iteration, heavy opts on dependencies so large Bevy/engine graphs do not
# emit multi-GB unoptimized rlibs in ordinary `cargo build` / `azoth build`.
[profile.dev]
opt-level = 1

[profile.dev.package."*"]
opt-level = 3"#
}

pub(super) fn workspace_dependencies_toml() -> ScaffoldResult<String> {
    let mut crate_names = Vec::new();
    for crate_name in PROJECT_ENGINE_WORKSPACE_CRATES
        .iter()
        .chain(PROJECT_ENGINE_BUILD_WORKSPACE_CRATES)
        .copied()
    {
        if !crate_names.contains(&crate_name) {
            crate_names.push(crate_name);
        }
    }

    let mut entries = crate_names
        .into_iter()
        .map(|crate_name| {
            engine_workspace_crate_version(crate_name)
                .map(|version| format!("{crate_name} = \"{version}\""))
        })
        .collect::<ScaffoldResult<Vec<_>>>()?;
    entries.extend(
        PROJECT_SHARED_WORKSPACE_DEPENDENCIES
            .iter()
            .chain(PROJECT_SHARED_WORKSPACE_ONLY_DEPENDENCIES)
            .chain(PROJECT_SHARED_BUILD_WORKSPACE_DEPENDENCIES)
            .map(shared_workspace_dependency_toml),
    );
    entries.push(
        "bevy = { version = \"0.19\", default-features = false, features = [\"bevy_asset\", \"serialize\"] }"
            .to_string(),
    );
    Ok(entries.join("\n"))
}

fn shared_workspace_dependency_toml(dependency: &ProjectSharedWorkspaceDependency) -> String {
    if dependency.features.is_empty() {
        format!("{} = \"{}\"", dependency.name, dependency.requirement)
    } else {
        let features = dependency
            .features
            .iter()
            .map(|feature| format!("\"{feature}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} = {{ version = \"{}\", features = [{}] }}",
            dependency.name, dependency.requirement, features
        )
    }
}

pub(super) fn engine_workspace_crate_version(crate_name: &str) -> ScaffoldResult<String> {
    crate::azoth_workspace_crate_version(crate_name)
}

pub(super) const fn project_assets_rs() -> &'static str {
    r"//! Project-owned asset builder registrations.
//!
//! Engine-owned builders — typed Prefabs to AZSCENE products, material
//! property documents to material products, terrain and texture sources —
//! reach a worker through the engine builder floor its composition already
//! carries. This module names none of them and links none of them: it is for
//! the `BuildRule`s this project owns.
//!
//! Register project-specific `BuildRule`s here and enumerate them from the
//! gem's `#[contribution]` registrar.
"
}

pub(super) const fn project_graphs_rs() -> &'static str {
    r#"//! Project-owned visual graph types and starter nodes.
//!
//! Graph descriptors are project/gem registrations. The universal editor reads
//! them through project-host and mutates graph documents through RPC; runtime
//! execution is produced by the graph compiler as generated Rust, not by
//! interpreting editor metadata.

use az_core::{AssetData, AssetType};
use az_graph_builder::AotGraphManifestAssetData;
use az_node_graph::{
    GraphCompilerBackendDescriptor, GraphDocumentTemplate, GraphNode, GraphNodeCatalogRequirement,
    GraphNodeId, GraphSourceWorkflow, GraphTypeDescriptor, GraphTypeRegistration, NodeCapability,
    NodePortDescriptor, NodePortId, NodeRuntimeBinding, NodeSourceLink, NodeTypeDescriptor,
    NodeTypeRegistration, RuntimeGraphExecutionStrategy, RuntimeGraphProductDescriptor,
    VisualGraphDocument,
};
use uuid::uuid;

pub const PROJECT_LOGIC_GRAPH_TYPE: &str = "azoth.project.logic-graph";
pub const PROJECT_TRACE_MARKER_NODE_TYPE: &str = "azoth.project.trace-marker";
pub const PROJECT_GRAPH_AOT_ASSET_TYPE_NAME: &str = AotGraphManifestAssetData::STABLE_NAME;
pub const PROJECT_GRAPH_AOT_ASSET_TYPE: AssetType = AotGraphManifestAssetData::ASSET_TYPE;

#[derive(Debug, Default)]
pub struct ProjectGraphContext {
    pub trace_markers_executed: u64,
}

pub fn trace_marker(
    context: &mut ProjectGraphContext,
) -> Result<(), az_graph_runtime::AotGraphExecutionError> {
    context.trace_markers_executed = context.trace_markers_executed.saturating_add(1);
    Ok(())
}

fn trace_marker_node_type() -> NodeTypeDescriptor {
    let _trace_marker: fn(&mut ProjectGraphContext) -> Result<(), az_graph_runtime::AotGraphExecutionError> =
        trace_marker;

    NodeTypeDescriptor::new(PROJECT_TRACE_MARKER_NODE_TYPE, 1, "Trace Marker")
        .with_category_path(["Project".to_string(), "Flow".to_string()])
        .with_description("Project starter node that compiles to a direct Rust function call.")
        .with_port(NodePortDescriptor::execution_input(NodePortId::new(1), "in"))
        .with_port(NodePortDescriptor::execution_output(NodePortId::new(2), "then"))
        .with_capability(NodeCapability::new("azoth.node.call").with_marker("aot-rust"))
        .with_runtime_binding(NodeRuntimeBinding::rust_symbol(
            env!("CARGO_PKG_NAME"),
            "crate::azoth::graphs::trace_marker",
        ))
        .with_source_link(NodeSourceLink::rust_symbol(
            env!("CARGO_PKG_NAME"),
            module_path!(),
            "crate::azoth::graphs::trace_marker",
            file!(),
            line!(),
            column!(),
        ))
        .with_tag("project")
}

fn project_logic_graph_template() -> GraphDocumentTemplate {
    let mut document = VisualGraphDocument::new(PROJECT_LOGIC_GRAPH_TYPE);
    document.nodes.push(GraphNode::new(
        GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000101")),
        PROJECT_TRACE_MARKER_NODE_TYPE,
        1,
    ));
    GraphDocumentTemplate { document }
}

fn project_logic_graph_type() -> GraphTypeDescriptor {
    GraphTypeDescriptor::runtime_compiled(
        PROJECT_LOGIC_GRAPH_TYPE,
        1,
        "Project Logic Graph",
        GraphSourceWorkflow::file("azoth.project.logic-graph.source", "azgraph.ron")
            .with_default_path_prefix("graphs"),
        GraphCompilerBackendDescriptor::generated_rust_context_schedule(
            "azoth.project.logic-graph.compiler",
            env!("CARGO_PKG_NAME"),
            "azoth_project_generated_graphs::execute_graph",
        )
        .with_capability_marker("generated-rust")
        .with_capability_marker("zero-cost")
        .with_capability_marker("aot"),
        RuntimeGraphProductDescriptor::new(
            PROJECT_GRAPH_AOT_ASSET_TYPE_NAME,
            "azoth.graph.aot-manifest",
            RuntimeGraphExecutionStrategy::aot_compiled_rust(
                env!("CARGO_PKG_NAME"),
                "azoth_project_generated_graphs::execute_graph",
                "crate::azoth::graphs::ProjectGraphContext",
            ),
        ),
    )
    .with_description("Project-authored logic graph compiled into generated Rust runtime code.")
    .with_category_path(["Project".to_string(), "Logic".to_string()])
    .with_template(project_logic_graph_template())
    .with_node_catalog(GraphNodeCatalogRequirement::new("azoth.project.nodes"))
    .with_tag("project")
    .with_tag("zero-cost")
}

/// The node types this gem owns, for its contribution to register.
pub fn node_types() -> [NodeTypeRegistration; 1] {
    [NodeTypeRegistration::new(trace_marker_node_type())]
}

/// The graph types this gem owns, for its contribution to register.
///
/// Composed values rather than link-time submissions: a node or graph type is
/// keyed by id *and* version, so two gems claiming one key collide at compose
/// time with both named, instead of whichever the linker reached first winning.
pub fn graph_types() -> [GraphTypeRegistration; 1] {
    [GraphTypeRegistration::new(project_logic_graph_type())]
}
"#
}

pub(super) const fn project_runtime_rs() -> &'static str {
    r#"//! Project-owned runtime projections.
//!
//! This module is compiled only into project service binaries. The universal
//! editor talks to runtime-host over RPC and never loads this crate.

use az_runtime_host::{
    RuntimeProjection, RuntimeProjectionError, RuntimeProjectionLaunchContext,
    RuntimeProjectionRegistration, RuntimeProjectionUpdate, RuntimeRole,
};

struct ProjectRuntimeProjection;

impl RuntimeProjection for ProjectRuntimeProjection {
    fn launch(
        &mut self,
        context: &RuntimeProjectionLaunchContext<'_>,
    ) -> Result<RuntimeProjectionUpdate, RuntimeProjectionError> {
        let package_summary = context
            .primary_asset_package_root()
            .map(|root| {
                format!(
                    "primary package {}:{} {} at {}",
                    root.profile,
                    root.asset_platform,
                    root.container.as_str(),
                    root.mount_root
                )
            })
            .unwrap_or_else(|| "no built package roots".to_string());
        Ok(RuntimeProjectionUpdate::running().with_diagnostic(format!(
            "project runtime `{}` launched {:?} from asset view {} with {} package root(s): {}",
            context.snapshot.project_id,
            context.role,
            context.snapshot.workspace_id,
            context.asset_package_roots().len(),
            package_summary
        )))
    }
}

fn project_runtime_projection() -> Box<dyn RuntimeProjection> {
    Box::new(ProjectRuntimeProjection)
}

/// The runtime projections this gem owns, for its contribution to register.
///
/// A composed value, not a link-time submission: the runtime-host runs what its
/// composition registered, so a projection this crate declares and never lists
/// is absent rather than mysteriously present.
pub fn projections() -> [RuntimeProjectionRegistration; 1] {
    [RuntimeProjectionRegistration::new(
        "azoth.project.default-runtime-projection",
        &[
            RuntimeRole::EditorWorld,
            RuntimeRole::PlayPreview,
            RuntimeRole::Validation,
        ],
        &[],
        project_runtime_projection,
    )
    .with_priority(10)]
}
"#
}

/// Repository metadata Azoth owns in every project root.
pub(super) fn repository_metadata_files(project_path: &Path) -> Vec<(PathBuf, &'static str)> {
    vec![(project_path.join(".loreignore"), loreignore())]
}

const fn loreignore() -> &'static str {
    r"/target/
/.azoth/
/.cargo/config.toml
/Cache/
/AssetProcessorTemp/
*.log
"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectWorkflowSourceControlState {
    pub has_lore_repository: bool,
    pub has_committed_revision: bool,
    pub has_local_changes: bool,
}

/// Report whether the project is a Lore repository, has a committed revision,
/// and has uncommitted changes.
///
/// # Errors
///
/// Returns [`ScaffoldError::SourceControl`] if the tree is a Lore repository
/// but its status cannot be read. A tree with no repository is reported as
/// `Ok` with every flag false.
pub fn project_workflow_source_control_state(
    project_path: &Path,
) -> ScaffoldResult<ProjectWorkflowSourceControlState> {
    if !has_lore_repository(project_path) {
        return Ok(ProjectWorkflowSourceControlState {
            has_lore_repository: false,
            has_committed_revision: false,
            has_local_changes: false,
        });
    }

    let status = LoreCli.status(project_path, true)?;
    Ok(ProjectWorkflowSourceControlState {
        has_lore_repository: true,
        has_committed_revision: status.revision_id.is_some(),
        has_local_changes: !status.clean(),
    })
}

/// Ensure the project root is a Lore repository with an initial checkpoint.
///
/// Today creation only runs when the caller supplies [`LoreRepositorySetup`]
/// (optional `lore_url` on CLI/editor create). Without that URL we leave the
/// tree unlinked and push a "Create Lore repository" next-step instead.
///
/// TODO(lore-auto-create): design auto-creation that covers editor Project
/// Manager create, CLI `azoth new`/`init`, and open-existing without a repo —
/// default local `loreserver` URL, shared store, first commit, and stable
/// `.lore/instance` so machine keys never need `unlinked-*` for real projects.
/// # Errors
///
/// Returns [`ScaffoldError::SourceControl`] if `setup` is supplied but the
/// repository cannot be created, or if staging, committing, or reading status
/// on an existing repository fails. A tree with no repository and no `setup`
/// is reported as `Ok` with every flag false.
pub fn ensure_project_lore_checkpoint(
    project_path: &Path,
    message: &str,
    setup: Option<&LoreRepositorySetup>,
) -> ScaffoldResult<ProjectWorkflowSourceControlState> {
    if !has_lore_repository(project_path) {
        if let Some(setup) = setup {
            LoreCli.create_repository(&CreateRepositoryRequest {
                remote_url: setup.remote_url.clone(),
                path: project_path.to_path_buf(),
                description: setup.description.clone(),
                use_shared_store: setup.use_shared_store,
            })?;
        } else {
            return project_workflow_source_control_state(project_path);
        }
    }

    let state = project_workflow_source_control_state(project_path)?;
    if !state.has_committed_revision || state.has_local_changes {
        LoreCli.stage(project_path, &[], StageMode::Scan)?;
        LoreCli.commit(project_path, message)?;
    }

    project_workflow_source_control_state(project_path)
}

fn has_lore_repository(project_path: &Path) -> bool {
    project_path.join(".lore").is_dir()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkflowNextStep {
    pub kind: ProjectWorkflowNextStepKind,
    pub label: &'static str,
    pub command: String,
}

impl ProjectWorkflowNextStep {
    #[must_use]
    pub fn new(
        kind: ProjectWorkflowNextStepKind,
        label: &'static str,
        command: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            label,
            command: command.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectWorkflowNextStepKind {
    CreateLoreRepository,
    CommitProjectWorkflow,
    CreateMainSession,
    OpenEditorSession,
    InspectSessionServices,
    RunProject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkflowNextStepPlan {
    pub steps: Vec<ProjectWorkflowNextStep>,
}

impl ProjectWorkflowNextStepPlan {
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.steps.iter().map(|step| step.command.clone()).collect()
    }
}

pub(super) fn print_project_workflow_next_steps(
    source_control_state: ProjectWorkflowSourceControlState,
    commit_message: &str,
    path: Option<&Path>,
) {
    for line in project_workflow_next_step_plan(source_control_state, commit_message, path).lines()
    {
        println!("  {line}");
    }
}

#[must_use]
pub fn project_workflow_next_step_plan(
    source_control_state: ProjectWorkflowSourceControlState,
    commit_message: &str,
    path: Option<&Path>,
) -> ProjectWorkflowNextStepPlan {
    let mut steps = Vec::new();
    if !source_control_state.has_lore_repository {
        steps.push(ProjectWorkflowNextStep::new(
            ProjectWorkflowNextStepKind::CreateLoreRepository,
            "Create Lore repository",
            create_lore_repository_command(path),
        ));
    }
    if !source_control_state.has_committed_revision || source_control_state.has_local_changes {
        let commit_message = if source_control_state.has_committed_revision {
            PROJECT_WORKFLOW_UPDATE_COMMIT_MESSAGE
        } else {
            commit_message
        };
        steps.push(ProjectWorkflowNextStep::new(
            ProjectWorkflowNextStepKind::CommitProjectWorkflow,
            "Commit project workflow",
            format!(
                "lore --no-pager --non-interactive stage --scan . && lore --no-pager --non-interactive commit \"{}\"",
                shell_double_quoted(commit_message)
            ),
        ));
    }
    steps.push(ProjectWorkflowNextStep::new(
        ProjectWorkflowNextStepKind::CreateMainSession,
        "Create main session",
        scoped_azoth_command("session create main", path),
    ));
    steps.push(ProjectWorkflowNextStep::new(
        ProjectWorkflowNextStepKind::OpenEditorSession,
        "Open editor session",
        scoped_azoth_command("editor --session main", path),
    ));
    steps.push(ProjectWorkflowNextStep::new(
        ProjectWorkflowNextStepKind::InspectSessionServices,
        "Inspect session services",
        scoped_azoth_command("session services status main", path),
    ));
    steps.push(ProjectWorkflowNextStep::new(
        ProjectWorkflowNextStepKind::RunProject,
        "Run project",
        scoped_azoth_command("run --session main", path),
    ));
    ProjectWorkflowNextStepPlan { steps }
}

fn create_lore_repository_command(path: Option<&Path>) -> String {
    path.map_or_else(
        || {
            "lore --no-pager --non-interactive repository create --use-shared-store <lore-url>"
                .to_string()
        },
        |path| {
            format!(
                "lore --no-pager --non-interactive repository create --use-shared-store --repository {} <lore-url>",
                shell_path_arg(path)
            )
        },
    )
}

fn scoped_azoth_command(command: &str, path: Option<&Path>) -> String {
    path.map_or_else(
        || format!("azoth {command}"),
        |path| format!("azoth {command} --project {}", shell_path_arg(path)),
    )
}

fn shell_double_quoted(value: &str) -> String {
    value.replace('"', "\\\"")
}

fn shell_path_arg(path: &Path) -> String {
    let path = path.display().to_string();
    if path.chars().any(char::is_whitespace) {
        format!("\"{}\"", path.replace('"', "\\\""))
    } else {
        path
    }
}

pub(super) fn is_valid_project_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest_test_support::assert_no_forbidden_manifest_dependencies;
    use std::process::Command;

    #[test]
    fn test_valid_project_names() {
        assert!(is_valid_project_name("my_game"));
        assert!(is_valid_project_name("my-game"));
        assert!(is_valid_project_name("game123"));
        assert!(is_valid_project_name("_game"));
    }

    #[test]
    fn test_invalid_project_names() {
        assert!(!is_valid_project_name(""));
        assert!(!is_valid_project_name("123game"));
        assert!(!is_valid_project_name("my game"));
        assert!(!is_valid_project_name("my.game"));
    }

    #[test]
    fn project_create_options_add_enabled_engine_gems_to_manifest() {
        let mut manifest = ProjectManifest::new(
            project_id_from_name("sample-game"),
            "sample-game",
            env!("CARGO_PKG_VERSION"),
        );
        apply_enabled_engine_gems(
            &mut manifest,
            &[
                " azoth.gamedata ".to_string(),
                "azoth.audio-system".to_string(),
                String::new(),
                "azoth.gamedata".to_string(),
            ],
        );

        assert_eq!(
            manifest
                .gems
                .iter()
                .map(|gem| (gem.id.as_str(), gem.enabled, gem.path.as_ref()))
                .collect::<Vec<_>>(),
            vec![
                ("azoth.audio-system", true, None),
                ("azoth.gamedata", true, None),
            ]
        );
    }

    #[test]
    fn generated_workspace_dependency_catalog_is_versioned_and_portable() {
        let toml = workspace_dependencies_toml().unwrap();
        let dev_profile = workspace_dev_profile_toml();

        assert!(dev_profile.contains("[profile.dev]\nopt-level = 1"));
        assert!(dev_profile.contains("[profile.dev.package.\"*\"]\nopt-level = 3"));
        assert!(!toml.contains(".azoth/engine"));
        for crate_name in [
            "az-animation",
            "az-gem-animation",
            "az-asset-builder",
            "az-core",
            "az-asset-processor",
            "az-asset-worker",
            "az-graph-builder",
            "az-graph-runtime",
            "az-material-builder",
            "az-node-graph",
            "az-prefab",
            "az-prefab-builder",
            "az-project",
            "az-project-host",
            "az-runtime-host",
            "az-scene",
            "az-service-entrypoint",
            "az-terrain",
            "az-terrain-builder",
            "az-terrain-runtime",
            "az-texture-builder",
        ] {
            let version = crate::azoth_workspace_crate_version(crate_name).unwrap();
            assert!(
                toml.contains(&format!("{crate_name} = \"{version}\"")),
                "workspace dependency catalog is missing `{crate_name}`"
            );
        }
        assert_eq!(toml.matches("az-graph-builder =").count(), 1);
        assert!(!toml.contains("/crates/az-"));
        assert!(toml.contains("clap = { version = \"4.6\", features = [\"derive\"] }"));
        assert!(toml.contains("tracing = \"0.1\""));
        assert!(
            toml.contains("uuid = { version = \"1.23\", features = [\"v4\", \"v7\", \"serde\"] }")
        );
        assert!(toml.contains("toml = \"1\""));
        assert!(toml.contains("ron = \"0.12\""));
        assert!(toml.contains("serde = { version = \"1.0\", features = [\"derive\"] }"));
    }

    #[test]
    fn new_project_uses_compact_project_layout_not_engine_source_layout() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("layout-game");

        execute("layout-game".to_string(), Some(project_root.clone()), None).unwrap();

        for expected in [
            "azoth.toml",
            "azoth.lock",
            "Cargo.toml",
            "assets",
            "scripts",
            "crates/README.md",
            "gems",
            "gems/layout-game/runtime",
            "gems/layout-game/authoring",
            "gems/layout-game/builders",
        ] {
            assert!(
                project_root.join(expected).exists(),
                "generated project must contain `{expected}`"
            );
        }
        assert!(!project_root.join("crates/game").exists());
        assert!(
            az_project::project_local_cargo_config_path(&project_root).is_file(),
            "generated projects must hydrate local Cargo engine patches"
        );
        assert!(project_root.join("Cargo.lock").is_file());
        assert!(
            project_root
                .join(".azoth/targets/generation.json")
                .is_file()
        );
        let cargo_manifest = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap();
        assert!(cargo_manifest.contains("default-members = [\n    \"gems/layout-game/runtime\""));
        assert!(!cargo_manifest.contains(".azoth/targets/*"));
        assert!(cargo_manifest.contains("[profile.dev]\nopt-level = 1"));
        assert!(cargo_manifest.contains("[profile.dev.package.\"*\"]\nopt-level = 3"));
        let cargo_config =
            std::fs::read_to_string(az_project::project_local_cargo_config_path(&project_root))
                .unwrap();
        let cargo_config = toml::from_str::<toml::Value>(&cargo_config).unwrap();
        assert!(cargo_config.get("target").is_none());
        assert!(cargo_config.get("build").is_none());
        assert!(!project_root.join(".azoth/engine").exists());

        let authored_lock = std::fs::read(project_root.join("Cargo.lock")).unwrap();
        std::fs::remove_file(project_root.join("Cargo.lock")).unwrap();
        let drifted_manifest = std::fs::read_to_string(project_root.join("Cargo.toml"))
            .unwrap()
            .replace("opt-level = 1", "opt-level = 0")
            .replace("opt-level = 3", "opt-level = 0");
        std::fs::write(project_root.join("Cargo.toml"), drifted_manifest).unwrap();

        crate::project_contract::sync_project_contract(&project_root).unwrap();
        assert_eq!(
            std::fs::read(project_root.join("Cargo.lock")).unwrap(),
            authored_lock
        );
        let repaired_manifest = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap();
        assert!(repaired_manifest.contains("[profile.dev]\nopt-level = 1"));
        assert!(repaired_manifest.contains("[profile.dev.package.\"*\"]\nopt-level = 3"));
        let repaired_generation =
            std::fs::read(project_root.join(".azoth/targets/generation.json")).unwrap();

        crate::project_contract::sync_project_contract(&project_root).unwrap();
        assert_eq!(
            std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap(),
            repaired_manifest
        );
        assert_eq!(
            std::fs::read(project_root.join(".azoth/targets/generation.json")).unwrap(),
            repaired_generation
        );

        for forbidden in [
            "crates/az",
            "crates/editor",
            "crates/integrations",
            "formats",
            "projects",
        ] {
            assert!(
                !project_root.join(forbidden).exists(),
                "generated projects must not mirror engine source-tree region `{forbidden}`"
            );
        }
    }

    #[test]
    fn new_project_writes_resolved_project_lock() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");

        execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        let lock = az_project::load_project_lock(&project_root).unwrap();
        assert_eq!(lock.project.id, "sample_game");
        assert_eq!(lock.packages.len(), 2);
        assert_eq!(lock.packages[0].root, PathBuf::from("."));
        assert_eq!(lock.packages[1].id, "sample_game.game");
        assert_eq!(lock.packages[1].root, PathBuf::from("gems/sample-game"));
        assert_eq!(lock.source_roots.len(), 2);
        assert_eq!(
            lock.source_roots[0].portable_key.as_str(),
            "project:sample_game:assets"
        );
        assert_eq!(
            lock.source_roots[0].tier,
            az_project::AssetRootTier::Project
        );
        assert_eq!(
            lock.source_roots[1].portable_key.as_str(),
            "gem:sample_game.game:assets"
        );
        assert_eq!(
            lock.source_roots[1].root,
            PathBuf::from("gems/sample-game/assets")
        );
        assert_eq!(
            lock.source_roots[1].tier,
            az_project::AssetRootTier::ProjectGem
        );
        assert_eq!(lock.packaging.profiles.len(), 2);
        assert_eq!(lock.packaging.profiles[0].name, "pc-dev");
        assert_eq!(
            lock.packaging.profiles[0].container,
            az_project::ProjectPackageContainer::Loose
        );
        assert_eq!(
            lock.packaging.profiles[0].compression,
            az_project::ProjectPackageCompression::None
        );
        assert_eq!(lock.packaging.profiles[1].name, "pc-release");
        assert_eq!(
            lock.packaging.profiles[1].container,
            az_project::ProjectPackageContainer::AzPack
        );
        assert_eq!(
            lock.packaging.profiles[1].compression,
            az_project::ProjectPackageCompression::Oodle
        );
        assert_eq!(
            lock.packaging.profiles[1].oodle,
            Some(az_project::ProjectPackageOodle::default())
        );
        assert!(lock.tools.build_targets.is_empty());
        assert!(lock.tools.service_targets.is_empty());
        assert!(project_root.join("gems").exists());
        assert!(!project_root.join("crates/plugins").exists());
        assert!(!project_root.join("crates/types").exists());
        assert!(project_root.join("gems/sample-game/gem.toml").exists());
        assert!(
            project_root
                .join("gems/sample-game/runtime/src/graphs.rs")
                .exists()
        );
        assert!(
            project_root
                .join("gems/sample-game/authoring/src/lib.rs")
                .exists()
        );
        assert!(
            project_root
                .join("gems/sample-game/builders/src/assets.rs")
                .exists()
        );
        assert!(!project_root.join("crates/game").exists());
        assert!(
            std::fs::read_to_string(project_root.join("Cargo.toml"))
                .unwrap()
                .contains("\"gems/sample-game/runtime\"")
        );
        assert!(project_lock_path(&project_root).exists());
        assert!(
            !project_root.join(".cache").exists(),
            "project-scaffolded cache roots are session launch policy, not project manifest paths"
        );
        assert!(
            !project_root.join("Cache").exists(),
            "dev product cache is created by session launch, not project scaffolding"
        );
        let loreignore = std::fs::read_to_string(project_root.join(".loreignore")).unwrap();
        assert!(loreignore.contains("/Cache/"));
    }

    #[test]
    fn generated_project_matches_external_project_contract() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-project");

        execute(
            "sample-project".to_string(),
            Some(project_root.clone()),
            None,
        )
        .unwrap();

        for expected in [
            "azoth.toml",
            "azoth.lock",
            "Cargo.toml",
            ".loreignore",
            "crates/README.md",
            "gems/sample-project/gem.toml",
            "gems/sample-project/runtime/Cargo.toml",
            "gems/sample-project/authoring/Cargo.toml",
            "gems/sample-project/builders/Cargo.toml",
        ] {
            assert!(project_root.join(expected).exists(), "missing `{expected}`");
        }
        assert!(!project_root.join("crates/game").exists());
        assert!(
            az_project::project_local_cargo_config_path(&project_root).is_file(),
            "generated external project must hydrate local Cargo engine patches"
        );

        let root_cargo = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap();
        assert!(root_cargo.contains("\"gems/sample-project/runtime\""));
        assert!(root_cargo.contains("az-project = \""));
        assert!(root_cargo.contains("az-graph-builder = \""));
        assert!(!root_cargo.contains(".azoth/engine"));

        let project_toml = std::fs::read_to_string(project_root.join("azoth.toml")).unwrap();
        assert!(project_toml.contains("primary_gem = \"sample_project.game\""));
        assert!(project_toml.contains("[topology]"));
        assert!(project_toml.contains("kind = \"single-player\""));
        assert!(!project_toml.contains("[[tools.build_targets]]"));

        let lock = az_project::load_project_lock(&project_root).unwrap();
        assert_eq!(lock.source_roots.len(), 2);
        assert_eq!(
            lock.source_roots[1].portable_key.as_str(),
            "gem:sample_project.game:assets"
        );
        assert!(lock.tools.service_targets.is_empty());
        assert_eq!(lock.packaging.profiles.len(), 2);
    }

    #[test]
    fn new_project_can_use_existing_empty_directory() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        std::fs::create_dir_all(&project_root).unwrap();

        execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        assert!(project_root.join("azoth.toml").exists());
        assert!(
            project_root
                .join("gems/sample-game/runtime/Cargo.toml")
                .exists()
        );
        assert!(!project_root.join("crates/game").exists());
    }

    #[test]
    fn new_project_without_lore_url_reports_missing_repository_for_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");

        execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        assert_eq!(
            project_workflow_source_control_state(&project_root).unwrap(),
            ProjectWorkflowSourceControlState {
                has_lore_repository: false,
                has_committed_revision: false,
                has_local_changes: false,
            }
        );
    }

    #[test]
    #[ignore = "runs cargo check against a generated standalone project"]
    fn generated_project_template_compiles_as_standalone_package() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");

        execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        let status = Command::new("cargo")
            .current_dir(&project_root)
            .arg("check")
            .arg("-j4")
            .arg("--manifest-path")
            .arg(project_root.join("Cargo.toml"))
            .status()
            .unwrap();

        assert!(status.success());
    }

    #[test]
    fn generated_cargo_manifests_keep_editor_and_engine_crates_out() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        for relative in [
            "Cargo.toml",
            "gems/sample-game/runtime/Cargo.toml",
            "gems/sample-game/authoring/Cargo.toml",
            "gems/sample-game/builders/Cargo.toml",
        ] {
            let toml = std::fs::read_to_string(project_root.join(relative)).unwrap();
            assert_no_forbidden_manifest_dependencies(
                relative,
                &toml,
                crate::manifest_test_support::FORBIDDEN_PROJECT_MANIFEST_DEPENDENCIES,
                crate::manifest_test_support::FORBIDDEN_PROJECT_MANIFEST_DEPENDENCY_PREFIXES,
            );
        }
    }

    #[test]
    fn project_workflow_next_steps_create_session_before_editor_or_runtime_commands() {
        let lines = project_workflow_next_step_plan(
            ProjectWorkflowSourceControlState {
                has_lore_repository: true,
                has_committed_revision: true,
                has_local_changes: false,
            },
            INITIAL_PROJECT_COMMIT_MESSAGE,
            None,
        )
        .lines();

        assert_eq!(
            lines,
            vec![
                "azoth session create main".to_string(),
                "azoth editor --session main".to_string(),
                "azoth session services status main".to_string(),
                "azoth run --session main".to_string(),
            ]
        );
    }

    #[test]
    fn project_workflow_next_steps_use_update_commit_message_for_dirty_existing_project() {
        let lines = project_workflow_next_step_plan(
            ProjectWorkflowSourceControlState {
                has_lore_repository: true,
                has_committed_revision: true,
                has_local_changes: true,
            },
            INITIAL_PROJECT_COMMIT_MESSAGE,
            None,
        )
        .lines();

        assert_eq!(
            lines[0],
            "lore --no-pager --non-interactive stage --scan . && lore --no-pager --non-interactive commit \"Update Azoth project workflow\""
        );
    }

    #[test]
    fn project_workflow_next_step_plan_labels_editor_workflow_commands() {
        let plan = project_workflow_next_step_plan(
            ProjectWorkflowSourceControlState {
                has_lore_repository: false,
                has_committed_revision: false,
                has_local_changes: false,
            },
            INITIAL_PROJECT_COMMIT_MESSAGE,
            Some(Path::new("projects/sample-game")),
        );

        assert_eq!(
            plan.steps
                .iter()
                .map(|step| (step.label, step.command.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (
                    "Create Lore repository",
                    "lore --no-pager --non-interactive repository create --use-shared-store --repository projects/sample-game <lore-url>",
                ),
                (
                    "Commit project workflow",
                    "lore --no-pager --non-interactive stage --scan . && lore --no-pager --non-interactive commit \"Initialize Azoth project\"",
                ),
                (
                    "Create main session",
                    "azoth session create main --project projects/sample-game",
                ),
                (
                    "Open editor session",
                    "azoth editor --session main --project projects/sample-game",
                ),
                (
                    "Inspect session services",
                    "azoth session services status main --project projects/sample-game",
                ),
                (
                    "Run project",
                    "azoth run --session main --project projects/sample-game",
                ),
            ]
        );
    }

    #[test]
    fn project_workflow_next_steps_scope_initialized_projects_by_path() {
        let path = Path::new("projects/sample-game");
        let lines = project_workflow_next_step_plan(
            ProjectWorkflowSourceControlState {
                has_lore_repository: false,
                has_committed_revision: false,
                has_local_changes: false,
            },
            INITIAL_PROJECT_COMMIT_MESSAGE,
            Some(path),
        )
        .lines();

        assert_eq!(
            lines,
            vec![
                "lore --no-pager --non-interactive repository create --use-shared-store --repository projects/sample-game <lore-url>".to_string(),
                "lore --no-pager --non-interactive stage --scan . && lore --no-pager --non-interactive commit \"Initialize Azoth project\"".to_string(),
                "azoth session create main --project projects/sample-game".to_string(),
                "azoth editor --session main --project projects/sample-game".to_string(),
                "azoth session services status main --project projects/sample-game".to_string(),
                "azoth run --session main --project projects/sample-game".to_string(),
            ]
        );
    }

    #[test]
    fn project_workflow_next_steps_quote_paths_with_spaces() {
        let path = Path::new("projects/sample game");
        let lines = project_workflow_next_step_plan(
            ProjectWorkflowSourceControlState {
                has_lore_repository: false,
                has_committed_revision: false,
                has_local_changes: false,
            },
            INITIAL_PROJECT_COMMIT_MESSAGE,
            Some(path),
        )
        .lines();

        assert_eq!(
            lines,
            vec![
                "lore --no-pager --non-interactive repository create --use-shared-store --repository \"projects/sample game\" <lore-url>".to_string(),
                "lore --no-pager --non-interactive stage --scan . && lore --no-pager --non-interactive commit \"Initialize Azoth project\"".to_string(),
                "azoth session create main --project \"projects/sample game\"".to_string(),
                "azoth editor --session main --project \"projects/sample game\"".to_string(),
                "azoth session services status main --project \"projects/sample game\"".to_string(),
                "azoth run --session main --project \"projects/sample game\"".to_string(),
            ]
        );
    }

    #[test]
    fn generated_project_asset_module_links_no_engine_builders() {
        let assets = project_assets_rs();

        assert!(assets.contains("//! Project-owned asset builder registrations."));
        // Engine builders reach a worker through the composed engine builder
        // floor. A force-link import here would be a second, silent mechanism.
        for forced in [
            "use az_material_builder as _;",
            "use az_prefab_builder as _;",
            "use az_terrain_builder as _;",
            "use az_texture_builder as _;",
        ] {
            assert!(
                !assets.contains(forced),
                "assets module still force-links: {forced}"
            );
        }
        assert!(!assets.contains("inventory"));
        // The scaffold no longer ships passthrough builders or project-owned
        // registrations for engine-owned authored documents.
        assert!(!assets.contains("authored_ron_builder"));
        assert!(!assets.contains("process_authored_job"));
        assert!(!assets.contains("RAW_PRODUCT_FORMAT_ID"));
        assert!(!assets.contains("SourceSchemaRegistration"));
        assert!(!assets.contains("azoth.project.Scene"));
        assert!(!assets.contains(".compiled"));
    }

    /// The link-time apparatus is gone; nothing this crate emits may name it.
    /// The four service entrypoint templates that used to carry the anchor no
    /// longer exist (services are prebuilt engine binaries), so the remaining
    /// subject is the project source `new` still emits into role packages.
    #[test]
    fn emitted_project_source_names_no_link_time_apparatus() {
        for source in [
            project_assets_rs(),
            project_graphs_rs(),
            project_runtime_rs(),
        ] {
            for dead in [
                "_azoth_project_crate",
                "bootstrap_project_inventory",
                "azoth_project_inventory",
                "az_inventory_diagnostics",
                "az_gem_link",
                "inventory_units",
                "try_enabled_gems",
                "inventory::submit",
            ] {
                assert!(
                    !source.contains(dead),
                    "emitted project source still names `{dead}`"
                );
            }
        }
    }

    #[test]
    fn generated_project_graph_module_registers_project_graph_catalog() {
        let graphs = project_graphs_rs();

        assert!(graphs.contains("PROJECT_LOGIC_GRAPH_TYPE"));
        assert!(graphs.contains("azoth.project.logic-graph"));
        assert!(graphs.contains("PROJECT_TRACE_MARKER_NODE_TYPE"));
        assert!(graphs.contains("azoth.project.trace-marker"));
        assert!(graphs.contains("AotGraphManifestAssetData::STABLE_NAME"));
        assert!(graphs.contains("AotGraphManifestAssetData::ASSET_TYPE"));
        assert!(graphs.contains("azoth.graph.aot-manifest"));
        assert!(graphs.contains("NodeTypeRegistration::new(trace_marker_node_type())"));
        assert!(graphs.contains("GraphTypeRegistration::new(project_logic_graph_type())"));
        assert!(graphs.contains("GraphCompilerBackendDescriptor::generated_rust_context_schedule"));
        assert!(graphs.contains("RuntimeGraphExecutionStrategy::aot_compiled_rust"));
        assert!(graphs.contains("azoth_project_generated_graphs::execute_graph"));
        assert!(graphs.contains("crate::azoth::graphs::trace_marker"));
        assert!(graphs.contains("crate::azoth::graphs::ProjectGraphContext"));
        assert!(graphs.contains("with_capability_marker(\"zero-cost\")"));
        assert!(graphs.contains("GraphDocumentTemplate { document }"));
        assert!(!graphs.contains("EditorInterpreted"));
        // Node and graph types reach a host by composition; nothing counts
        // them to keep a linker from discarding them.
        assert!(!graphs.contains("inventory_units"));
    }

    #[test]
    fn generated_project_runtime_module_registers_project_projection() {
        let runtime = project_runtime_rs();

        assert!(runtime.contains("RuntimeProjectionRegistration::new"));
        assert!(runtime.contains("pub fn projections() -> [RuntimeProjectionRegistration; 1]"));
        assert!(!runtime.contains("inventory"));
        assert!(runtime.contains("RuntimeRole::EditorWorld"));
        assert!(runtime.contains("RuntimeRole::PlayPreview"));
        assert!(runtime.contains("RuntimeProjectionUpdate::running"));
        assert!(runtime.contains("primary_asset_package_root"));
        assert!(runtime.contains("asset_package_roots().len()"));
        assert!(runtime.contains("package root(s)"));
    }

    #[test]
    fn generated_loreignore_excludes_dev_product_cache() {
        let ignore = loreignore();

        assert!(ignore.contains("/Cache/"));
        assert!(!ignore.contains("/.cache/"));
    }
}
