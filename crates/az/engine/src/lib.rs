//! `AZoth` Engine - Game engine built on Bevy ECS
//!
//! This crate provides core engine functionality inspired by CryEngine/Lumberyard:
//! - **Core** - Geometry, colors, string interning, and math utilities
//! - **Entity** - Entity components and spawn helpers
//! - **Asset** - Bevy asset type re-exports only
//!
//! Platform, networking, backend-service, and game-specific features live in
//! project/gem plugin crates. The base engine crate stays lean so the editor
//! and project workflow can choose those plugins explicitly.
//!
//! # Quick Start
//!
//! Add the engine plugin to your Bevy app:
//!
//! ```rust,no_run
//! use bevy::prelude::*;
//! use az_engine::prelude::*;
//!
//! fn main() {
//!     App::new()
//!         .add_plugins((DefaultPlugins, EnginePlugin))
//!         .run();
//! }
//! ```
//!
//! Spawn entities with names:
//!
//! ```rust,no_run
//! use bevy::prelude::*;
//! use az_engine::prelude::*;
//!
//! fn spawn_entities(mut commands: Commands) {
//!     // Spawn a named entity (uses Bevy's Name component)
//!     commands.spawn((
//!         Name::new("MyEntity"),
//!         EntityFlags::new(),
//!     ));
//!
//!     // Spawn a temporary entity (not saved to disk)
//!     commands.spawn((
//!         Name::new("TempHelper"),
//!         EntityFlags::temporary(),
//!     ));
//! }
//! ```
//!
//! # Philosophy
//!
//! This engine leverages Bevy's excellent ECS architecture rather than
//! trying to replicate CryEngine/Lumberyard's OOP patterns. We use:
//! - Bevy Plugins instead of custom module systems
//! - Bevy Messages instead of custom event buses
//! - Bevy Resources instead of singleton registries
//! - Bevy Components instead of heavyweight interfaces
//!
//! # Error Handling
//!
//! Most engine operations return `EngineResult<T>` for consistent error handling:
//!
//! ```rust
//! use az_engine::prelude::EngineResult;
//!
//! fn safe_operation() -> EngineResult<()> {
//!     // Operations that might fail
//!     Ok(())
//! }
//! ```

pub mod asset;
pub mod core;
pub mod entity;

// Prelude with commonly used types
pub mod prelude {
    // Asset types
    pub use crate::asset::prelude::*;

    // Core types
    pub use crate::core::prelude::*;

    // Entity types
    pub use crate::entity::prelude::*;

    // Re-export Bevy's message system
    pub use bevy::prelude::{Message, MessageReader, MessageWriter, Messages};

    // Plugins
    pub use crate::EnginePlugin;
    pub use crate::entity::EntityPlugin;
}

/// Engine version
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ENGINE_NAME: &str = "AZoth Engine";

// PRE-RELEASE VERSION LOCK. The engine version is pinned to "0.1.0" until the
// first public release. Bumping it pre-release churns every project manifest,
// lock, and test fixture (`engine_version`/`EngineVersionMismatch`) for zero
// benefit — nothing consumes a higher version yet. This guard fails compilation
// if `az-engine`'s package version drifts. When we actually cut a release, raise
// BOTH the package version and this expected string together, deliberately.
const _ENGINE_VERSION_LOCK: () = {
    const fn str_eq(a: &str, b: &str) -> bool {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        let mut i = 0;
        while i < a.len() {
            if a[i] != b[i] {
                return false;
            }
            i += 1;
        }
        true
    }
    assert!(
        str_eq(ENGINE_VERSION, "0.1.0"),
        "Engine version is LOCKED to 0.1.0 until first release — do not bump it. \
         Update this lock in crates/az/engine/src/lib.rs only as part of a real release."
    );
};

/// Main engine plugin that composes base engine systems.
pub struct EnginePlugin;

impl bevy::app::Plugin for EnginePlugin {
    fn build(&self, app: &mut bevy::app::App) {
        app.add_plugins((entity::EntityPlugin,));
    }
}

#[cfg(test)]
mod architecture_tests {
    use az_architecture_guard::{
        COMPATIBILITY_FORMAT_DEPENDENCIES, DependencyBoundary, PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
        forbidden_production_dependencies, production_source_without_cfg_test_modules,
    };
    use toml::Value;

    const FORBIDDEN_PRODUCTION_DEPS: &[&str] = &[
        "az-asset",
        "az-asset-builder",
        "az-asset-processor",
        "az-assetdb",
        "az-daemon",
        "az-editor",
        "az-editor-inspector",
        "az-editor-ui",
        "az-framework",
        "az-project",
        "az-project-host",
        "az-runtime-host",
        "az-schema",
        "az-session",
        "az-sessiond",
        "aws-plugin",
        "eos-plugin",
        "gridmate",
        "http-plugin",
        "lmbr-central",
        "logging-registry",
        "lyshine",
        "sample-plugin",
        "spacetime-plugin",
        "steam-locator",
    ];
    const FORBIDDEN_PATH_SEGMENTS: &[&str] = &["formats", "gems", "projects"];

    #[test]
    fn production_engine_stays_lean_and_project_agnostic() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read az-engine Cargo.toml");
        let manifest = toml::from_str::<Value>(&manifest).expect("parse az-engine Cargo.toml");
        let workspace_manifest =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Cargo.toml"))
                .expect("read workspace Cargo.toml");
        let workspace_manifest =
            toml::from_str::<Value>(&workspace_manifest).expect("parse workspace Cargo.toml");
        let violations = forbidden_engine_dependencies(&manifest, &workspace_manifest);

        assert!(
            violations.is_empty(),
            "az-engine production dependencies must stay lean and project/gem agnostic; forbidden direct deps: {}",
            violations.join(", ")
        );
    }

    #[test]
    fn production_engine_rejects_forbidden_workspace_package_aliases() {
        let manifest = toml::from_str::<Value>(
            r"
            [dependencies]
            net = { workspace = true }
            ",
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<Value>(
            r#"
            [workspace.dependencies]
            net = { package = "gridmate", path = "../gridmate" }
            "#,
        )
        .unwrap();
        let violations = forbidden_engine_dependencies(&manifest, &workspace_manifest);

        assert_eq!(
            violations,
            vec!["dependencies.net(workspace package gridmate)"]
        );
    }

    #[test]
    fn production_engine_rejects_editor_legacy_and_gem_aliases() {
        let manifest = toml::from_str::<Value>(
            r#"
            [dependencies]
            editor = { package = "az-editor", path = "../az-editor" }
            chunk-format = { package = "cry-chunk-assets", path = "../formats/legacy/cry-chunk-assets" }
            proto = { package = "az-proto-project", path = "../az-proto-project" }
            terrain-gem = { package = "az-gem-legacy-terrain", path = "../gems/legacy-terrain" }
            "#,
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<Value>("[workspace.dependencies]").unwrap();
        let violations = forbidden_engine_dependencies(&manifest, &workspace_manifest);

        assert_eq!(
            violations,
            vec![
                "dependencies.chunk-format(package cry-chunk-assets)",
                "dependencies.editor(package az-editor)",
                "dependencies.proto(package az-proto-project)",
                "dependencies.terrain-gem(package az-gem-legacy-terrain)",
            ]
        );
    }

    #[test]
    fn production_engine_rejects_target_specific_and_path_only_dependencies() {
        let manifest = toml::from_str::<Value>(
            r#"
            [target.'cfg(windows)'.dependencies]
            platform-runtime = { package = "gridmate", path = "../gridmate" }
            terrain = { path = "../gems/legacy-terrain" }

            [target.'cfg(unix)'.build-dependencies]
            project-proto = { package = "az-proto-project", path = "../az-proto-project" }
            legacy-format = { path = "../formats/legacy/cry-chunk-assets" }
            compatible-pak = { path = "../formats/compat/az-pak" }
            "#,
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<Value>("[workspace.dependencies]").unwrap();
        let violations = forbidden_engine_dependencies(&manifest, &workspace_manifest);

        assert_eq!(
            violations,
            vec![
                "target.cfg(unix).build-dependencies.compatible-pak(path ../formats/compat/az-pak)",
                "target.cfg(unix).build-dependencies.legacy-format(path ../formats/legacy/cry-chunk-assets)",
                "target.cfg(unix).build-dependencies.project-proto(package az-proto-project)",
                "target.cfg(windows).dependencies.platform-runtime(package gridmate)",
                "target.cfg(windows).dependencies.terrain(path ../gems/legacy-terrain)",
            ]
        );
    }

    #[test]
    fn production_engine_does_not_expose_backend_plugin_modules() {
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
            .expect("read az-engine lib.rs");
        let production_source = production_source_without_cfg_test_modules(&source);

        for forbidden in [
            "pub mod network",
            "pub mod physics",
            "NetworkPlugin",
            "PhysicsPlugin",
        ] {
            assert!(
                !production_source.contains(forbidden),
                "az-engine base API must not expose backend plugin surface `{forbidden}`; put selected networking/physics backends in project/gem crates"
            );
        }
    }

    #[test]
    fn production_engine_does_not_define_runtime_asset_inventory_manifests() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read az-engine Cargo.toml");
        let asset_source =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/asset/mod.rs"))
                .expect("read az-engine asset module");
        let bundle_source =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/asset/bundles.rs"));

        assert!(
            !bundle_source.exists(),
            "az-engine must not keep an asset bundle manifest module; runtime inventory is package roots plus Asset Catalog"
        );

        for forbidden in [
            "pub mod bundles",
            "AssetBundleManifest",
            "AssetBundleManager",
            "AssetBundlePlugin",
            "bundle.ron",
        ] {
            assert!(
                !asset_source.contains(forbidden),
                "az-engine asset facade must not expose runtime inventory manifest API `{forbidden}`"
            );
        }

        for forbidden in ["ron =", "serde ="] {
            assert!(
                !manifest.contains(forbidden),
                "az-engine must not depend on `{forbidden}` for runtime inventory manifests"
            );
        }
    }

    #[test]
    fn engine_readme_documents_only_current_base_surface() {
        let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))
            .expect("read az-engine README.md");

        for forbidden in [
            "az_engine::module",
            "az_engine::system",
            "az_engine::ebus",
            "EngineModule",
            "EngineSystem",
            "SystemRegistry",
            "MyNetworkGem",
            "PhysicsSystem",
            "EBUS_EVENT",
        ] {
            assert!(
                !readme.contains(forbidden),
                "az-engine README must not document stale base-engine or backend API `{forbidden}`"
            );
        }
    }

    #[test]
    fn production_engine_has_no_dormant_backend_plugin_source_modules() {
        for forbidden_dir in ["network", "physics"] {
            let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
                .join(forbidden_dir);

            assert!(
                !path.exists(),
                "az-engine base source must not keep dormant backend plugin module `{forbidden_dir}`; move backend implementations into project/gem crates"
            );
        }
    }

    fn forbidden_engine_dependencies(manifest: &Value, workspace_manifest: &Value) -> Vec<String> {
        let mut exact_names = Vec::new();
        exact_names.extend_from_slice(FORBIDDEN_PRODUCTION_DEPS);
        exact_names.extend_from_slice(COMPATIBILITY_FORMAT_DEPENDENCIES);
        let mut prefixes = vec!["az-proto-"];
        prefixes.extend_from_slice(PROJECT_OR_GAME_DEPENDENCY_PREFIXES);
        forbidden_production_dependencies(
            manifest,
            workspace_manifest,
            DependencyBoundary::new(&exact_names, &prefixes, FORBIDDEN_PATH_SEGMENTS),
        )
    }
}
