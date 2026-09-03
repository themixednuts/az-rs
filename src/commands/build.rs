use crate::error::{CliError, CliResult, CommandFailedDetails};
use az_asset::{
    AssetCatalog, AssetCatalogPathRegistration, AssetId, PackageManifest, PackageManifestEntry,
    PackageManifestProfile, PackagePayloadWriteRequest, ProductDependency,
    format_package_release_id_hex, package_manifest_release_id, parse_package_content_hash_hex,
    write_asset_catalog, write_package_manifest,
};
use az_filesystem::AzothDataHome;
use az_proto_asset::{CatalogPathRegistration, CatalogProductEntry};
use az_proto_core::{Endpoint, EndpointKind};
use az_proto_daemon::{
    DAEMON_PROJECTS_PERMISSION, PlanProjectBuildRequest, ProjectBuildCommand,
    ProjectBuildPackageProfile, ProjectBuildPlan, ProjectRecord, RegisterProjectRootRequest,
};
use clap::ValueEnum;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use tracing::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum BuildAssetOutput {
    /// Process runtime assets into the Azoth developer product cache only.
    Cache,
    /// Process runtime assets and assemble the selected package profile.
    #[default]
    Package,
}

/// The flags of a single `azoth build` invocation: what to build (profile, package
/// selectors, cargo target), which project directory and session supply its assets, and how
/// to reach azd for build planning. Clap parses them as one unit and the build consumes them
/// as one unit, so they travel together instead of as eight positional arguments.
#[derive(Debug)]
pub struct BuildOptions {
    pub profile: String,
    pub asset_output: BuildAssetOutput,
    pub package_selectors: Vec<String>,
    pub session: Option<String>,
    pub target: Option<String>,
    pub path: Option<PathBuf>,
    pub daemon_endpoint_kind: Option<EndpointKind>,
    pub daemon_endpoint: Option<String>,
}

pub fn execute(options: BuildOptions) -> CliResult<()> {
    let project_path = options.path.unwrap_or_else(|| PathBuf::from("."));
    az_project::preflight_project_build_selectors(&project_path, &options.package_selectors)?;
    crate::commands::host_tools::ensure_project_host_tools()?;
    az_project_scaffold::project_contract::sync_project_contract(&project_path)?;
    crate::commands::daemon::start(
        std::slice::from_ref(&project_path),
        None,
        crate::commands::daemon::DEFAULT_DAEMON_START_TIMEOUT_MS,
        options.daemon_endpoint_kind,
        options.daemon_endpoint.as_deref(),
    )?;
    let daemon_endpoint = crate::commands::daemon::optional_project_daemon_endpoint_with_source(
        options.daemon_endpoint_kind,
        options.daemon_endpoint.as_deref(),
        &project_path,
    )?;

    info!("Building project in: {}", project_path.display());
    info!("Profile: {}", options.profile);
    if let Some(ref t) = options.target {
        info!("Target: {}", t);
    }

    let staging_profile = options.profile.clone();
    let staging_selectors = options.package_selectors.clone();
    let staging_target = options.target.clone();
    let (project_id, plan) = plan_build_commands(
        &project_path,
        options.profile,
        options.package_selectors,
        options.target,
        daemon_endpoint.as_ref(),
    )?;

    for command in &plan.commands {
        run_build_command(command)?;
    }

    // Stage authored runtime sidecars after cargo succeeds. The CLI executes the
    // daemon-planned commands itself, so staging lives here (not in the plan RPC)
    // and covers `azoth build` without a protocol version bump.
    let staging_reports = az_project::stage_selected_authored_runtime_files_for_target(
        &project_path,
        &staging_selectors,
        &staging_profile,
        staging_target.as_deref(),
    )?;
    for report in &staging_reports {
        for entry in &report.entries {
            match entry.action {
                az_project::RuntimeFileStagingAction::Staged => {
                    println!(
                        "Staged runtime file for '{}': {} -> {}",
                        report.target_name,
                        entry.relative_source,
                        entry.destination.display()
                    );
                    info!(
                        target = %report.target_name,
                        source = %entry.source.display(),
                        destination = %entry.destination.display(),
                        "staged build target runtime file"
                    );
                }
                az_project::RuntimeFileStagingAction::AlreadyFresh => {
                    info!(
                        target = %report.target_name,
                        destination = %entry.destination.display(),
                        "build target runtime file already fresh"
                    );
                }
            }
        }
    }

    if let Some(package_profile) = &plan.package_profile {
        let resolved_daemon = daemon_endpoint
            .as_ref()
            .ok_or(CliError::MissingDaemonEndpoint {
                operation: "project asset processing",
            })?;
        let (asset_session, catalog) = crate::commands::session::process_project_assets(
            &project_path,
            options.session.as_deref(),
            &package_profile.asset_platform,
            resolved_daemon,
            true,
        )?;
        println!(
            "Runtime-ready asset cache: {} entries in {} (session '{}')",
            catalog.entry_count, catalog.catalog_path, asset_session
        );
        if matches!(options.asset_output, BuildAssetOutput::Package) {
            write_package_build_input(
                &project_id,
                &project_path,
                package_profile,
                Some(&asset_session),
                daemon_endpoint.as_ref(),
            )?;
        }
    }

    println!("Build completed successfully for project '{project_id}'.");

    Ok(())
}

fn write_package_build_input(
    project_id: &str,
    project_path: &Path,
    profile: &ProjectBuildPackageProfile,
    session: Option<&str>,
    daemon_endpoint: Option<&crate::commands::daemon::OptionalDaemonEndpoint>,
) -> CliResult<()> {
    for line in package_profile_summary_lines(profile) {
        println!("{line}");
    }

    if let Some(session) = session {
        let entries = crate::commands::session::catalog_products_for_session(
            project_path,
            session,
            &profile.asset_platform,
            daemon_endpoint,
        )?;
        print_package_product_summary(session, &profile.asset_platform, &entries);
        let manifest = package_manifest_from_products(profile, &entries)?;
        let release_id = format_package_release_id_hex(&package_manifest_release_id(&manifest)?);
        let manifest_path = package_manifest_output_path(project_path, &profile.name, session);
        write_package_manifest_file(&manifest_path, &manifest)?;
        println!("  package_release_id: {release_id}");
        println!("  package_manifest: {}", manifest_path.display());
        if let Some(output) = write_package_payload(project_id, project_path, session, &manifest)? {
            println!("  package_payload: {}", output.payload_path.display());
            println!("  asset_catalog: {}", output.asset_catalog_path.display());
        }
    } else {
        println!(
            "  products: asset-processor catalogProducts --platform {}",
            profile.asset_platform
        );
        println!("  catalog_session: not queried; pass --session <name> to validate inputs");
    }

    Ok(())
}

fn package_manifest_from_products(
    profile: &ProjectBuildPackageProfile,
    entries: &[CatalogProductEntry],
) -> CliResult<PackageManifest> {
    let profile = PackageManifestProfile {
        name: profile.name.clone(),
        asset_platform: profile.asset_platform.clone(),
        cargo_profile: profile.cargo_profile.clone(),
        container: profile.container.clone(),
        compression: profile.compression.clone(),
        oodle_compressor: profile.oodle_compressor.clone(),
        oodle_effort: profile.oodle_effort.clone(),
    };

    let entries = entries
        .iter()
        .map(package_manifest_entry_from_product)
        .collect::<CliResult<Vec<_>>>()?;
    Ok(PackageManifest::new(profile, entries)?)
}

fn package_manifest_entry_from_product(
    entry: &CatalogProductEntry,
) -> CliResult<PackageManifestEntry> {
    let sub_id =
        u32::try_from(entry.sub_id).map_err(|_| CliError::AssetProcessorAuthorityMismatch {
            operation: "catalogProducts",
            reason: format!(
                "current product {} sub id {} cannot fit runtime asset sub id u32",
                entry.product_id, entry.sub_id
            ),
        })?;
    let byte_len = u64::try_from(entry.byte_length).map_err(|_| {
        CliError::AssetProcessorAuthorityMismatch {
            operation: "catalogProducts",
            reason: format!(
                "current product {} byte length {} cannot fit package manifest u64",
                entry.product_id, entry.byte_length
            ),
        }
    })?;
    let dependencies = entry
        .dependencies
        .iter()
        .map(package_product_dependency_from_product)
        .collect::<CliResult<Vec<_>>>()?;
    Ok(PackageManifestEntry::new(
        entry.product_path.clone(),
        entry.asset_type,
        sub_id,
        entry.product_format.clone(),
        entry.product_format_version,
        parse_package_content_hash_hex(&entry.content_hash)?,
        byte_len,
        entry.asset_guid,
        entry.source_path.clone(),
        entry.job_key.clone(),
    )
    .with_path_registration(match entry.catalog_path_registration {
        CatalogPathRegistration::Registered => AssetCatalogPathRegistration::Registered,
        CatalogPathRegistration::AssetIdOnly => AssetCatalogPathRegistration::AssetIdOnly,
    })
    .with_catalog_aliases(entry.catalog_aliases.clone())
    .with_dependencies(dependencies))
}

fn package_product_dependency_from_product(
    dependency: &az_proto_asset::CatalogProductDependency,
) -> CliResult<ProductDependency> {
    let sub_id = u32::try_from(dependency.sub_id).map_err(|_| {
        CliError::AssetProcessorAuthorityMismatch {
            operation: "catalogProducts",
            reason: format!(
                "catalog product dependency sub id {} cannot fit runtime asset sub id u32",
                dependency.sub_id
            ),
        }
    })?;
    let asset_type =
        dependency
            .asset_type
            .ok_or_else(|| CliError::AssetProcessorAuthorityMismatch {
                operation: "catalogProducts",
                reason: format!(
                    "catalog product dependency {}:{} is missing its runtime asset type",
                    dependency.asset_guid, dependency.sub_id
                ),
            })?;
    let mut edge = ProductDependency::new(AssetId::new(dependency.asset_guid, sub_id), asset_type);
    edge.hint.clone_from(&dependency.hint);
    Ok(edge)
}

fn write_package_manifest_file(path: &Path, manifest: &PackageManifest) -> CliResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path)?;
    write_package_manifest(manifest, &mut file)?;
    Ok(())
}

fn write_package_payload(
    project_id: &str,
    project_path: &Path,
    session: &str,
    manifest: &PackageManifest,
) -> CliResult<Option<PackageOutputReceipt>> {
    write_package_payload_with_data_home(
        &AzothDataHome::resolve(),
        project_id,
        project_path,
        session,
        manifest,
    )
}

fn write_package_payload_with_data_home(
    data_home: &AzothDataHome,
    project_id: &str,
    project_path: &Path,
    session: &str,
    manifest: &PackageManifest,
) -> CliResult<Option<PackageOutputReceipt>> {
    let paths = data_home.project(project_id, project_path);
    paths.prepare()?;
    let cache_root = paths.product_cache_dir(&manifest.profile.asset_platform)?;
    let output_root = az_session::package_output_dir(project_path, &manifest.profile.name, session);
    let receipt = az_asset::write_package_payload(PackagePayloadWriteRequest::new(
        manifest,
        &cache_root,
        &output_root,
    ))?;
    let asset_catalog_path = receipt.catalog_path.clone();
    write_runtime_asset_catalog(&asset_catalog_path, manifest)?;
    Ok(Some(PackageOutputReceipt {
        payload_path: receipt.payload_path,
        asset_catalog_path,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackageOutputReceipt {
    payload_path: PathBuf,
    asset_catalog_path: PathBuf,
}

fn write_runtime_asset_catalog(path: &Path, manifest: &PackageManifest) -> CliResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let catalog = AssetCatalog::from_package_manifest(manifest)?;
    let mut file = File::create(path)?;
    write_asset_catalog(&catalog, &mut file)?;
    Ok(())
}

fn package_manifest_output_path(project_path: &Path, profile_name: &str, session: &str) -> PathBuf {
    az_session::package_output_dir(project_path, profile_name, session)
        .join(az_session::PACKAGE_MANIFEST_FILE_NAME)
}

fn package_profile_summary_lines(profile: &ProjectBuildPackageProfile) -> Vec<String> {
    let mut lines = vec![
        format!("Package profile {}", profile.name),
        format!("  asset_platform: {}", profile.asset_platform),
        format!("  cargo_profile: {}", profile.cargo_profile),
        format!("  container: {}", profile.container),
        format!("  compression: {}", profile.compression),
    ];
    if let Some(compressor) = &profile.oodle_compressor {
        lines.push(format!("  oodle_compressor: {compressor}"));
    }
    if let Some(effort) = &profile.oodle_effort {
        lines.push(format!("  oodle_effort: {effort}"));
    }
    lines
}

fn print_package_product_summary(session: &str, platform: &str, entries: &[CatalogProductEntry]) {
    println!("  catalog_session: {session}");
    println!("  catalog_products[{platform}]: {}", entries.len());
    for entry in entries {
        println!(
            "    {}: {} type {} sub {} hash {} bytes {}",
            entry.product_id,
            entry.product_path,
            entry.asset_type,
            entry.sub_id,
            entry.content_hash,
            entry.byte_length
        );
    }
}

fn plan_build_commands(
    project_path: &Path,
    profile: String,
    package_selectors: Vec<String>,
    target: Option<String>,
    daemon_endpoint: Option<&crate::commands::daemon::OptionalDaemonEndpoint>,
) -> CliResult<(String, ProjectBuildPlan)> {
    let Some(resolved) = daemon_endpoint else {
        return Err(CliError::MissingDaemonEndpoint {
            operation: "project build planning",
        });
    };

    let endpoint = &resolved.endpoint;
    info!(
        endpoint = %endpoint.address,
        endpoint_kind = ?endpoint.kind,
        "planning build through azd"
    );
    println!(
        "Planning project build through azd at {} ({:?})",
        endpoint.address, endpoint.kind
    );
    std::io::stdout().flush()?;
    match plan_build_commands_through_daemon(
        project_path,
        profile,
        package_selectors,
        target,
        endpoint,
    ) {
        Ok(plan) => Ok(plan),
        Err(error)
            if resolved.source == crate::commands::daemon::DaemonEndpointSource::RuntimeRecord
                && crate::commands::daemon::is_daemon_connection_failure(&error) =>
        {
            crate::commands::daemon::handle_stale_project_runtime_record(&error, project_path)?;
            Err(CliError::MissingDaemonEndpoint {
                operation: "project build planning",
            })
        }
        Err(error) => Err(error),
    }
}

fn plan_build_commands_through_daemon(
    project_path: &Path,
    profile: String,
    package_selectors: Vec<String>,
    target: Option<String>,
    endpoint: &Endpoint,
) -> CliResult<(String, ProjectBuildPlan)> {
    let requested_root = project_path.to_path_buf();
    let project_root = project_path.to_string_lossy().into_owned();
    crate::commands::daemon::with_daemon_progress(
        endpoint,
        "project build planning",
        crate::commands::daemon::DAEMON_RPC_PROGRESS_INTERVAL,
        async move |client| {
            println!(
                "  azd: registering project root {}",
                requested_root.display()
            );
            std::io::stdout().flush()?;
            let mut register = client.register_project_root_request();
            (RegisterProjectRootRequest {
                capability: crate::commands::daemon::daemon_capability(DAEMON_PROJECTS_PERMISSION),
                root: project_root,
            })
            .to_capnp(register.get().init_request())?;
            let register_response = register.send().promise.await?;
            let project = ProjectRecord::from_capnp(register_response.get()?.get_project()?)?;
            crate::commands::daemon::ensure_daemon_project_record_matches_request(
                &project,
                None,
                Some(&requested_root),
                "registerProjectRoot",
            )?;

            println!(
                "  azd: project resolved as {}; requesting build plan",
                project.project_id
            );
            std::io::stdout().flush()?;
            let mut plan_request = client.plan_project_build_request();
            (PlanProjectBuildRequest {
                capability: crate::commands::daemon::daemon_capability(DAEMON_PROJECTS_PERMISSION),
                project_id: project.project_id.clone(),
                profile,
                target_triple: target,
                package_selectors,
            })
            .to_capnp(plan_request.get().init_request())?;
            let plan_response = plan_request.send().promise.await?;
            let plan = ProjectBuildPlan::from_capnp(plan_response.get()?.get_plan()?)?;
            crate::commands::daemon::ensure_daemon_project_build_plan_matches_request(
                &plan,
                &project.project_id,
            )?;
            println!(
                "  azd: build plan ready ({} build command(s))",
                plan.commands.len()
            );
            std::io::stdout().flush()?;
            Ok((project.project_id, plan))
        },
    )
}

pub fn run_build_command(command: &ProjectBuildCommand) -> CliResult<()> {
    info!(
        "Running build target '{}:{}' from owner root {} in {}: {} {:?}",
        command.owner_id,
        command.target_name,
        command.owner_root,
        command.cwd,
        command.program,
        command.args
    );
    let cwd = PathBuf::from(&command.cwd);
    println!(
        "Running build target '{}:{}' in {}",
        command.owner_id,
        command.target_name,
        cwd.display()
    );
    println!("  command: {} {}", command.program, command.args.join(" "));
    std::io::stdout().flush()?;
    let started = Instant::now();
    let mut process = Command::new(&command.program);
    process.args(&command.args).current_dir(&cwd);
    if let Some(target_dir) = &command.cargo_target_dir {
        process.env("CARGO_TARGET_DIR", target_dir);
    }
    let owner = az_work::OwnedSynchronousCommandTree::new()?;
    let mut child = owner.spawn(&mut process)?;
    let status = child.wait()?;
    if status.success() {
        println!(
            "Build target '{}:{}' finished in {}s",
            command.owner_id,
            command.target_name,
            started.elapsed().as_secs()
        );
        Ok(())
    } else {
        Err(CliError::CommandFailed(Box::new(CommandFailedDetails {
            program: command.program.clone(),
            args: command.args.clone(),
            cwd,
            status: status.code(),
        })))
    }
}

#[cfg(test)]
mod tests {
    use az_daemon::{AzDaemon, start_az_daemon_rpc_server_with_daemon};
    use az_project::{
        ProjectBuildTarget, ProjectManifest, refresh_project_lock, write_project_manifest,
    };

    use super::*;

    #[test]
    fn build_planning_can_route_through_azd_rpc() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.cli_build_rpc", "Build RPC", "0.1.0");
        manifest
            .tools
            .build_targets
            .push(ProjectBuildTarget::package("game", "game"));
        write_project_manifest(temp.path(), &manifest).unwrap();
        refresh_project_lock(temp.path()).unwrap();
        let daemon =
            AzDaemon::with_data_home(AzothDataHome::new(temp.path().join("azoth-home"))).unwrap();
        let server = start_az_daemon_rpc_server_with_daemon(
            daemon,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .unwrap();
        let daemon_endpoint = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint: server.endpoint().clone(),
            source: crate::commands::daemon::DaemonEndpointSource::Explicit,
        };

        let (project_id, plan) = plan_build_commands(
            temp.path(),
            "pc-release".to_string(),
            vec!["game".to_string()],
            Some("x86_64-pc-windows-msvc".to_string()),
            Some(&daemon_endpoint),
        )
        .unwrap();

        assert_eq!(project_id, "local.cli_build_rpc");
        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].target_name, "game");
        let mut expected_args = if cfg!(target_os = "windows") {
            vec!["build"]
        } else {
            vec!["xwin", "build"]
        };
        expected_args.extend([
            "-p",
            "game",
            "--release",
            "--target",
            "x86_64-pc-windows-msvc",
        ]);
        assert_eq!(plan.commands[0].args, expected_args);
        let package_profile = plan.package_profile.unwrap();
        assert_eq!(package_profile.name, "pc-release");
        assert_eq!(package_profile.asset_platform, "pc");
        assert_eq!(package_profile.cargo_profile, "release");
        assert_eq!(package_profile.container, "azpack");
        assert_eq!(package_profile.compression, "oodle");

        server.stop();
    }

    #[test]
    fn build_planning_requires_azd_endpoint() {
        let temp = tempfile::tempdir().unwrap();

        let error = plan_build_commands(temp.path(), "debug".to_string(), Vec::new(), None, None)
            .unwrap_err();

        match error {
            CliError::MissingDaemonEndpoint { operation } => {
                assert_eq!(operation, "project build planning");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn build_package_profile_summary_names_package_policy() {
        let lines = package_profile_summary_lines(&valid_package_profile());

        assert_eq!(lines[0], "Package profile pc-release");
        assert!(lines.iter().any(|line| line == "  asset_platform: pc"));
        assert!(lines.iter().any(|line| line == "  container: azpack"));
        assert!(lines.iter().any(|line| line == "  compression: oodle"));
        assert!(
            lines
                .iter()
                .any(|line| line == "  oodle_compressor: kraken")
        );
        assert!(lines.iter().any(|line| line == "  oodle_effort: normal"));
    }

    #[test]
    fn build_package_manifest_from_products_uses_resolved_policy_and_products() {
        let manifest = package_manifest_from_products(
            &valid_package_profile(),
            &[valid_catalog_product_entry()],
        )
        .unwrap();

        assert_eq!(manifest.profile.name, "pc-release");
        assert_eq!(manifest.profile.asset_platform, "pc");
        assert_eq!(manifest.profile.container, "azpack");
        assert_eq!(manifest.profile.compression, "oodle");
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(
            manifest.entries[0].product_path.as_str(),
            "materials/armor/foo.mtl"
        );
        assert_eq!(
            az_asset::format_package_content_hash_hex(&manifest.entries[0].content_hash),
            "ab".repeat(32)
        );
        assert_eq!(manifest.entries[0].sub_id, 7);
        assert_eq!(manifest.entries[0].byte_len, 128);
    }

    #[test]
    fn build_runtime_catalog_carries_product_dependencies() {
        let mut entry = valid_catalog_product_entry();
        entry.dependencies = vec![
            az_proto_asset::CatalogProductDependency {
                asset_guid: uuid::Uuid::from_bytes([9; 16]),
                sub_id: 2,
                asset_type: Some(uuid::Uuid::from_bytes([4; 16])),
                hint: Some("@assets@/textures/base.dds".to_string()),
            },
            az_proto_asset::CatalogProductDependency {
                asset_guid: uuid::Uuid::from_bytes([8; 16]),
                sub_id: 0,
                asset_type: Some(uuid::Uuid::nil()),
                hint: None,
            },
        ];

        let manifest = package_manifest_from_products(&valid_package_profile(), &[entry]).unwrap();
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(
            manifest.entries[0].dependencies.len(),
            2,
            "product dependencies flow into the package manifest entry"
        );

        // Write the runtime catalog through the real writer and read it back.
        let catalog = AssetCatalog::from_package_manifest(&manifest).unwrap();
        let mut bytes = Vec::new();
        write_asset_catalog(&catalog, &mut bytes).unwrap();
        let read_back = az_asset::read_asset_catalog(bytes.as_slice()).unwrap();

        assert_eq!(read_back.entries.len(), 1);
        let deps = &read_back.entries[0].dependencies;
        assert_eq!(deps.len(), 2);
        // The format layer sorts by (id, asset_type, hint): guid 8 sorts first.
        assert_eq!(deps[0].id.guid, uuid::Uuid::from_bytes([8; 16]));
        assert_eq!(deps[0].id.sub_id, 0);
        assert_eq!(deps[0].asset_type, uuid::Uuid::nil());
        assert_eq!(deps[0].hint, None);
        assert_eq!(deps[1].id.guid, uuid::Uuid::from_bytes([9; 16]));
        assert_eq!(deps[1].id.sub_id, 2);
        assert_eq!(deps[1].asset_type, uuid::Uuid::from_bytes([4; 16]));
        assert_eq!(deps[1].hint.as_deref(), Some("@assets@/textures/base.dds"));
    }

    #[test]
    fn build_package_manifest_rejects_dependency_without_runtime_asset_type() {
        let mut entry = valid_catalog_product_entry();
        entry.dependencies = vec![az_proto_asset::CatalogProductDependency {
            asset_guid: uuid::Uuid::from_bytes([9; 16]),
            sub_id: 2,
            asset_type: None,
            hint: Some("@assets@/textures/base.dds".to_string()),
        }];

        let error = package_manifest_from_products(&valid_package_profile(), &[entry]).unwrap_err();
        match error {
            CliError::AssetProcessorAuthorityMismatch { operation, reason } => {
                assert_eq!(operation, "catalogProducts");
                assert!(reason.contains("missing its runtime asset type"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn build_package_manifest_rejects_sub_ids_outside_runtime_asset_id_range() {
        let mut entry = valid_catalog_product_entry();
        entry.sub_id = i64::from(u32::MAX) + 1;

        let error = package_manifest_from_products(&valid_package_profile(), &[entry]).unwrap_err();
        match error {
            CliError::AssetProcessorAuthorityMismatch { operation, reason } => {
                assert_eq!(operation, "catalogProducts");
                assert!(reason.contains("u32"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn loose_package_payload_copies_validated_cache_products() {
        let temp = tempfile::tempdir().unwrap();
        let project_id = "local.package_test";
        let session = "play";
        let product_path = "materials/armor/foo.mtl";
        let bytes = b"compiled material bytes";
        let data_home = AzothDataHome::new(temp.path().join("azoth-home"));
        let cache_product = data_home
            .project(project_id, temp.path())
            .product_cache_dir("pc")
            .unwrap()
            .join(product_path);
        fs::create_dir_all(cache_product.parent().unwrap()).unwrap();
        fs::write(&cache_product, bytes).unwrap();
        let manifest = PackageManifest::new(
            valid_loose_package_profile(),
            vec![valid_package_manifest_entry(product_path, bytes)],
        )
        .unwrap();

        let output = write_package_payload_with_data_home(
            &data_home,
            project_id,
            temp.path(),
            session,
            &manifest,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            fs::read(output.payload_path.join(product_path)).unwrap(),
            bytes.to_vec()
        );
        assert!(output.asset_catalog_path.is_file());
        let catalog =
            az_asset::read_asset_catalog(fs::read(&output.asset_catalog_path).unwrap().as_slice())
                .unwrap();
        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(catalog.entries[0].path.as_str(), product_path);
        assert_eq!(
            catalog.entries[0].asset_id.guid,
            uuid::Uuid::from_bytes([1; 16])
        );
        assert_eq!(catalog.entries[0].asset_id.sub_id, 7);
        assert_eq!(
            catalog.entries[0].asset_type,
            uuid::Uuid::from_bytes([3; 16])
        );
        assert_eq!(
            catalog.entries[0].content_hash,
            *blake3::hash(bytes).as_bytes()
        );
    }

    #[test]
    fn loose_package_payload_rejects_hash_mismatches() {
        let temp = tempfile::tempdir().unwrap();
        let project_id = "local.package_test";
        let product_path = "materials/armor/foo.mtl";
        let data_home = AzothDataHome::new(temp.path().join("azoth-home"));
        let cache_product = data_home
            .project(project_id, temp.path())
            .product_cache_dir("pc")
            .unwrap()
            .join(product_path);
        fs::create_dir_all(cache_product.parent().unwrap()).unwrap();
        fs::write(&cache_product, b"different data").unwrap();
        let manifest = PackageManifest::new(
            valid_loose_package_profile(),
            vec![valid_package_manifest_entry(
                product_path,
                b"expected bytes",
            )],
        )
        .unwrap();

        let error = write_package_payload_with_data_home(
            &data_home,
            project_id,
            temp.path(),
            "play",
            &manifest,
        )
        .unwrap_err();

        match error {
            CliError::PackagePayload(payload) => match *payload {
                az_asset::PackagePayloadError::HashMismatch {
                    product_path: path, ..
                } => {
                    assert_eq!(path, product_path);
                }
                other => panic!("unexpected error: {other}"),
            },
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    #[cfg(feature = "oodle")]
    fn package_payload_writes_azpack_oodle_policy() {
        let temp = tempfile::tempdir().unwrap();
        let project_id = "local.package_test";
        let product_path = "materials/armor/foo.mtl";
        let bytes = b"compiled material bytes compiled material bytes";
        let data_home = AzothDataHome::new(temp.path().join("azoth-home"));
        let cache_product = data_home
            .project(project_id, temp.path())
            .product_cache_dir("pc")
            .unwrap()
            .join(product_path);
        fs::create_dir_all(cache_product.parent().unwrap()).unwrap();
        fs::write(&cache_product, bytes).unwrap();
        let manifest = PackageManifest::new(
            valid_package_manifest_profile(),
            vec![valid_package_manifest_entry(product_path, bytes)],
        )
        .unwrap();

        let output = write_package_payload_with_data_home(
            &data_home,
            project_id,
            temp.path(),
            "play",
            &manifest,
        )
        .unwrap()
        .unwrap();
        let index = fs::read(output.payload_path.join("package.azpack.index")).unwrap();

        assert_eq!(
            &index[..az_asset::AZPACK_INDEX_MAGIC.len()],
            az_asset::AZPACK_INDEX_MAGIC
        );
        assert!(output.payload_path.join("chunks").is_dir());
        let catalog =
            az_asset::read_asset_catalog(fs::read(&output.asset_catalog_path).unwrap().as_slice())
                .unwrap();
        assert_eq!(catalog.entries[0].path.as_str(), product_path);
    }

    fn valid_package_profile() -> ProjectBuildPackageProfile {
        ProjectBuildPackageProfile {
            name: "pc-release".to_string(),
            asset_platform: "pc".to_string(),
            cargo_profile: "release".to_string(),
            container: "azpack".to_string(),
            compression: "oodle".to_string(),
            oodle_compressor: Some("kraken".to_string()),
            oodle_effort: Some("normal".to_string()),
        }
    }

    #[cfg(feature = "oodle")]
    fn valid_package_manifest_profile() -> PackageManifestProfile {
        PackageManifestProfile {
            name: "pc-release".to_string(),
            asset_platform: "pc".to_string(),
            cargo_profile: "release".to_string(),
            container: "azpack".to_string(),
            compression: "oodle".to_string(),
            oodle_compressor: Some("kraken".to_string()),
            oodle_effort: Some("normal".to_string()),
        }
    }

    fn valid_loose_package_profile() -> PackageManifestProfile {
        PackageManifestProfile {
            name: "pc-dev".to_string(),
            asset_platform: "pc".to_string(),
            cargo_profile: "dev".to_string(),
            container: "loose".to_string(),
            compression: "none".to_string(),
            oodle_compressor: None,
            oodle_effort: None,
        }
    }

    fn valid_package_manifest_entry(product_path: &str, bytes: &[u8]) -> PackageManifestEntry {
        PackageManifestEntry::new(
            product_path,
            uuid::Uuid::from_bytes([3; 16]),
            7,
            "az.test.raw",
            1,
            *blake3::hash(bytes).as_bytes(),
            u64::try_from(bytes.len()).unwrap(),
            uuid::Uuid::from_bytes([1; 16]),
            "prefabs/source.prefab.ron",
            "BuildPrefab",
        )
    }

    fn valid_catalog_product_entry() -> CatalogProductEntry {
        CatalogProductEntry {
            job_id: 10,
            product_id: 20,
            asset_guid: uuid::Uuid::from_bytes([1; 16]),
            source_path: "prefabs/source.prefab.ron".to_string(),
            builder_guid: uuid::Uuid::from_bytes([2; 16]),
            job_key: "BuildPrefab".to_string(),
            platform: "pc".to_string(),
            product_path: "materials/armor/foo.mtl".to_string(),
            asset_type: uuid::Uuid::from_bytes([3; 16]),
            sub_id: 7,
            product_format: "az.test.raw".to_string(),
            product_format_version: 1,
            content_hash: "ab".repeat(32),
            byte_length: 128,
            dependencies: Vec::new(),
            catalog_aliases: Vec::new(),
            catalog_path_registration: CatalogPathRegistration::Registered,
        }
    }
}
