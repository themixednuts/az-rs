use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use az_filesystem::{HostTool, HostToolBundle};

use crate::error::{CliError, CliResult, CommandFailedDetails};

const ENGINE_WORKSPACE_ROOT: &str = env!("CARGO_MANIFEST_DIR");
const PROJECT_HOST_TOOLS: [HostTool; 3] = [
    HostTool::Daemon,
    HostTool::SessionSupervisor,
    HostTool::AssetProcessor,
];
static PREPARED_SOURCE_TOOLS: OnceLock<Mutex<Vec<HostTool>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostToolBuildPlan {
    cwd: PathBuf,
    args: Vec<String>,
}

pub fn ensure_project_host_tools() -> CliResult<HostToolBundle> {
    ensure_host_tools(&PROJECT_HOST_TOOLS)
}

pub fn require_prebuilt_project_host_tools() -> CliResult<HostToolBundle> {
    require_prebuilt_host_tools(&PROJECT_HOST_TOOLS)
}

pub fn ensure_daemon() -> CliResult<PathBuf> {
    let bundle = ensure_host_tools(&[HostTool::Daemon])?;
    Ok(bundle.resolve(HostTool::Daemon)?)
}

pub fn require_prebuilt_daemon() -> CliResult<PathBuf> {
    let bundle = require_prebuilt_host_tools(&[HostTool::Daemon])?;
    Ok(bundle.resolve(HostTool::Daemon)?)
}

pub fn ensure_session_supervisor() -> CliResult<PathBuf> {
    let bundle = ensure_host_tools(&[HostTool::SessionSupervisor])?;
    Ok(bundle.resolve(HostTool::SessionSupervisor)?)
}

pub fn ensure_assetdb_maintenance() -> CliResult<PathBuf> {
    let bundle = ensure_host_tools(&[HostTool::AssetDbMaintenance])?;
    Ok(bundle.resolve(HostTool::AssetDbMaintenance)?)
}

fn ensure_host_tools(required: &[HostTool]) -> CliResult<HostToolBundle> {
    let bundle = HostToolBundle::current()?;
    let missing = bundle.missing(required);
    let prepared = PREPARED_SOURCE_TOOLS.get_or_init(|| Mutex::new(Vec::new()));
    let prepared = prepared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut build_tools = required
        .iter()
        .copied()
        .filter(|tool| !prepared.contains(tool))
        .collect::<Vec<_>>();
    drop(prepared);
    build_tools.extend(missing.iter().copied());
    build_tools.sort_unstable();
    build_tools.dedup();

    let engine_root = Path::new(ENGINE_WORKSPACE_ROOT);
    let Some(plan) = source_checkout_build_plan(engine_root, &bundle, &build_tools) else {
        if let Some(tool) = missing.first() {
            return Err(bundle.resolve(*tool).unwrap_err().into());
        }
        return Ok(bundle);
    };

    println!(
        "Preparing Azoth host tools before project work: {}",
        build_tools
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("  command: cargo {}", plan.args.join(" "));
    std::io::Write::flush(&mut std::io::stdout())?;

    let mut process = Command::new("cargo");
    process.args(&plan.args).current_dir(&plan.cwd);
    let owner = az_work::OwnedSynchronousCommandTree::new()?;
    let mut child = owner.spawn(&mut process)?;
    let status = child.wait()?;
    if !status.success() {
        return Err(CliError::CommandFailed(Box::new(CommandFailedDetails {
            program: "cargo".to_string(),
            args: plan.args,
            cwd: plan.cwd,
            status: status.code(),
        })));
    }

    for tool in required {
        bundle.resolve(*tool)?;
    }
    let mut prepared = PREPARED_SOURCE_TOOLS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for tool in build_tools {
        if !prepared.contains(&tool) {
            prepared.push(tool);
        }
    }
    drop(prepared);
    Ok(bundle)
}

fn require_prebuilt_host_tools(required: &[HostTool]) -> CliResult<HostToolBundle> {
    let bundle = HostToolBundle::current()?;
    for tool in required {
        bundle.resolve(*tool)?;
    }
    Ok(bundle)
}

fn source_checkout_build_plan(
    engine_root: &Path,
    bundle: &HostToolBundle,
    missing: &[HostTool],
) -> Option<HostToolBuildPlan> {
    if missing.is_empty() || !engine_root.join("engine.toml").is_file() {
        return None;
    }

    let engine_root = engine_root
        .canonicalize()
        .unwrap_or_else(|_| engine_root.to_path_buf());
    let bundle_directory = bundle
        .directory()
        .canonicalize()
        .unwrap_or_else(|_| bundle.directory().to_path_buf());
    let target_root = engine_root.join("target");
    let profile = bundle_directory.strip_prefix(&target_root).ok()?;
    if profile.components().count() != 1 {
        return None;
    }
    let profile = profile.file_name()?.to_str()?;
    if !matches!(profile, "debug" | "release") {
        return None;
    }

    let mut tools = missing.to_vec();
    tools.sort_unstable();
    tools.dedup();
    let mut args = vec!["build".to_string(), "--locked".to_string()];
    if profile == "release" {
        args.push("--release".to_string());
    }
    for tool in tools {
        args.extend([
            "-p".to_string(),
            tool.cargo_package().to_string(),
            "--bin".to_string(),
            tool.cargo_binary().to_string(),
        ]);
    }
    Some(HostToolBuildPlan {
        cwd: engine_root,
        args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_checkout_preflight_builds_all_missing_tools_in_one_locked_wave() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("engine.toml"), "[engine]\n").unwrap();
        std::fs::create_dir_all(temp.path().join("target/debug")).unwrap();
        let bundle =
            HostToolBundle::adjacent_to(temp.path().join("target/debug").join("azoth.exe"))
                .unwrap();

        let plan = source_checkout_build_plan(
            temp.path(),
            &bundle,
            &[
                HostTool::SessionSupervisor,
                HostTool::Daemon,
                HostTool::AssetProcessor,
            ],
        )
        .unwrap();

        assert_eq!(plan.cwd, temp.path().canonicalize().unwrap());
        assert_eq!(
            plan.args,
            [
                "build",
                "--locked",
                "-p",
                "az-daemon",
                "--bin",
                "azd",
                "-p",
                "az-sessiond",
                "--bin",
                "az-sessiond",
                "-p",
                "az-asset-processor-host",
                "--bin",
                "asset-processor",
            ]
        );
    }

    #[test]
    fn installed_bundle_does_not_plan_a_source_build() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("engine.toml"), "[engine]\n").unwrap();
        std::fs::create_dir_all(temp.path().join("install/bin")).unwrap();
        let bundle =
            HostToolBundle::adjacent_to(temp.path().join("install/bin/azoth.exe")).unwrap();

        assert_eq!(
            source_checkout_build_plan(temp.path(), &bundle, &[HostTool::Daemon]),
            None
        );
    }

    #[test]
    fn release_source_checkout_keeps_host_tools_adjacent() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("engine.toml"), "[engine]\n").unwrap();
        std::fs::create_dir_all(temp.path().join("target/release")).unwrap();
        let bundle =
            HostToolBundle::adjacent_to(temp.path().join("target/release").join("azoth.exe"))
                .unwrap();

        let plan = source_checkout_build_plan(temp.path(), &bundle, &[HostTool::Daemon]).unwrap();

        assert_eq!(plan.args[0..3], ["build", "--locked", "--release"]);
    }
}
