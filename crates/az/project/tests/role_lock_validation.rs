use std::fs;
use std::path::Path;

use az_project::{
    AZOTH_ENGINE_ROOT_ENV, EngineManifest, GemTargetRole, GeneratedTargetPackage,
    GeneratedTargetsSyncReport, GeneratedTargetsSyncStatus, engine_manifest_path,
    ensure_project_engine_patch_table_to, validate_project_generated_target_workspaces,
};

#[test]
fn generated_workspace_validation_uses_project_and_engine_lock_packages() {
    let fixture = tempfile::tempdir().unwrap();
    let engine_root = fixture.path().join("engine");
    let project_root = fixture.path().join("project");
    write_engine(&engine_root);
    let project_lock = write_project(&project_root);
    ensure_project_engine_patch_table_to(&project_root, &engine_root).unwrap();

    let workspace_root = project_root.join(".azoth/targets");
    let role_root = workspace_root.join("server");
    let target_directory = project_root.join("target");
    write_role_workspace(&role_root, &target_directory, "1.2.3");
    let report = GeneratedTargetsSyncReport {
        status: GeneratedTargetsSyncStatus::Unchanged,
        target_directory,
        workspace_root: Some(workspace_root),
        old_fingerprint: None,
        fingerprint: None,
        targets: vec![GeneratedTargetPackage {
            name: "server".to_string(),
            package: "azoth-target-server".to_string(),
            roles: vec![GemTargetRole::Server],
            linked_packages: Vec::new(),
        }],
        manifests: Vec::new(),
    };

    unsafe {
        std::env::set_var(AZOTH_ENGINE_ROOT_ENV, &engine_root);
    }
    validate_project_generated_target_workspaces(&project_root, &report).unwrap();

    write_role_lock(&role_root, "1.2.4");
    let error = validate_project_generated_target_workspaces(&project_root, &report).unwrap_err();

    assert!(error.to_string().contains("engine-only-package 1.2.4"));
    assert_eq!(
        fs::read(project_root.join("Cargo.lock")).unwrap(),
        project_lock
    );
}

fn write_engine(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        engine_manifest_path(root),
        toml::to_string_pretty(&EngineManifest::new(
            "fixture-engine",
            "Fixture Engine",
            "0.1.0",
        ))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture-engine\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\nresolver = \"3\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn engine() {}\n").unwrap();
    fs::write(
        root.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"engine-only-package\"\nversion = \"1.2.3\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"engine-checksum\"\n\n[[package]]\nname = \"fixture-engine\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
}

fn write_project(root: &Path) -> Vec<u8> {
    fs::create_dir_all(root.join("crates/app/src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/app\"]\nresolver = \"3\"\n",
    )
    .unwrap();
    fs::write(
        root.join("crates/app/Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("crates/app/src/lib.rs"), "pub fn app() {}\n").unwrap();
    fs::write(
        root.join("azoth.lock"),
        "[engine]\nid = \"fixture-engine\"\nrevision = \"fixture-revision\"\n",
    )
    .unwrap();
    let lock = b"version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n".to_vec();
    fs::write(root.join("Cargo.lock"), &lock).unwrap();
    lock
}

fn write_role_workspace(root: &Path, target_directory: &Path, version: &str) {
    fs::create_dir_all(root.join(".cargo")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"azoth-target-server\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\nresolver = \"3\"\n",
    )
    .unwrap();
    // Debug-formatting the lossy string, not the `Path`, produces the same
    // escaped TOML literal without tripping `unnecessary_debug_formatting`.
    let target_directory = target_directory.to_string_lossy();
    fs::write(
        root.join(".cargo/config.toml"),
        format!("[build]\ntarget-dir = {target_directory:?}\n"),
    )
    .unwrap();
    write_role_lock(root, version);
}

fn write_role_lock(root: &Path, version: &str) {
    fs::write(
        root.join("Cargo.lock"),
        format!(
            "version = 4\n\n[[package]]\nname = \"azoth-target-server\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"engine-only-package\"\nversion = {version:?}\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"engine-checksum\"\n"
        ),
    )
    .unwrap();
}
