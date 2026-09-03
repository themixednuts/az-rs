//! Shared build-script plumbing for the Azoth Cap'n Proto mirror crates.
//!
//! Every `az-proto-*` crate compiles one schema file that imports the schemas
//! of the sibling crates it layers on. Historically each `build.rs` copy-pasted
//! ~60 lines of `capnp` executable discovery plus a hand-maintained table of
//! sibling schema-ID literals for `crate_provides`. Both drift independently.
//!
//! This crate is the single source of truth: [`ProtoCrate`] carries each
//! schema's directory, filename, generated Rust module ident, and 64-bit
//! Cap'n Proto file ID; [`compile`] wires the `capnpc` command from that data.
//! ADR 0031 Correction 4(f).

use std::env;
use std::path::PathBuf;

/// One Azoth protocol schema crate.
///
/// The 64-bit ids are the `@0x…` file ids declared at the top of each
/// `schema/azoth/<name>.capnp`; `capnpc`'s `crate_provides` needs them to route
/// imported types to the sibling crate's generated module instead of inlining a
/// duplicate. Keep these in lockstep with the schema headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtoCrate {
    Core,
    Authoring,
    Daemon,
    Asset,
    Project,
    Runtime,
    Session,
    Observability,
}

impl ProtoCrate {
    /// Directory name under `crates/az/proto/` (also the schema stem).
    #[must_use]
    pub const fn dir_name(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Authoring => "authoring",
            Self::Daemon => "daemon",
            Self::Asset => "asset",
            Self::Project => "project",
            Self::Runtime => "runtime",
            Self::Session => "session",
            Self::Observability => "observability",
        }
    }

    /// Generated-crate ident passed to `capnpc`'s `crate_provides`.
    #[must_use]
    pub const fn crate_ident(self) -> &'static str {
        match self {
            Self::Core => "az_proto_core",
            Self::Authoring => "az_proto_authoring",
            Self::Daemon => "az_proto_daemon",
            Self::Asset => "az_proto_asset",
            Self::Project => "az_proto_project",
            Self::Runtime => "az_proto_runtime",
            Self::Session => "az_proto_session",
            Self::Observability => "az_proto_observability",
        }
    }

    /// The `@0x…` Cap'n Proto file id declared in the schema header.
    #[must_use]
    pub const fn schema_id(self) -> u64 {
        match self {
            Self::Core => 0xb9d8_d393_4f22_5ef0,
            Self::Authoring => 0xf0fe_538a_5c80_6112,
            Self::Daemon => 0xa871_ca06_2d49_b47f,
            Self::Asset => 0xdb6f_9c0d_89a3_fef1,
            Self::Project => 0xe203_af83_e430_44c1,
            Self::Runtime => 0xd8ea_3a6a_f1d5_294e,
            Self::Session => 0xcee0_76d5_c796_2bc2,
            Self::Observability => 0xf35c_9e7a_1a4c_2b61,
        }
    }

    /// Schema file path relative to this crate's `schema/` src-prefix root,
    /// e.g. `azoth/core.capnp`.
    fn schema_rel(self) -> String {
        format!("azoth/{}.capnp", self.dir_name())
    }
}

/// Compile `own`'s schema, importing the schemas of each crate in `deps`.
///
/// Emits `rerun-if-changed` for the own schema and every dependency schema,
/// wires `import_path`/`crate_provides` for the siblings, and locates the
/// `capnp` executable. Call from a mirror crate's `build.rs` main:
///
/// ```ignore
/// az_proto_build::compile(
///     az_proto_build::ProtoCrate::Daemon,
///     &[ProtoCrate::Core, ProtoCrate::Session, ProtoCrate::Project,
///       ProtoCrate::Asset, ProtoCrate::Runtime],
/// );
/// ```
///
/// # Panics
/// Panics if `capnpc` code generation fails (mirrors the previous per-crate
/// `expect`).
pub fn compile(own: ProtoCrate, deps: &[ProtoCrate]) {
    println!("cargo:rerun-if-changed=schema/{}", own.schema_rel());
    for dep in deps {
        println!(
            "cargo:rerun-if-changed=../{}/schema/{}",
            dep.dir_name(),
            dep.schema_rel()
        );
    }
    println!("cargo:rerun-if-env-changed=CAPNP");

    let mut command = capnpc::CompilerCommand::new();
    command.src_prefix("schema").import_path("schema");

    for dep in deps {
        command.import_path(format!("../{}/schema", dep.dir_name()));
        command.crate_provides(dep.crate_ident(), [dep.schema_id()]);
    }

    command.file(format!("schema/{}", own.schema_rel()));

    if let Some(capnp) = capnp_executable() {
        if let Some(schema_root) = standard_schema_import_path(&capnp) {
            command.import_path(schema_root);
        }
        command.capnp_executable(capnp);
    }

    command.run().unwrap_or_else(|error| {
        panic!(
            "generate {} Cap'n Proto bindings: {error}",
            own.crate_ident()
        )
    });
}

/// Find the import root containing Cap'n Proto's standard `capnp/*.capnp`
/// schemas beside the selected compiler installation.
fn standard_schema_import_path(capnp: &std::path::Path) -> Option<PathBuf> {
    for ancestor in capnp.ancestors().take(5) {
        for candidate in [
            ancestor.to_path_buf(),
            ancestor.join("include"),
            ancestor.join("src"),
            ancestor.join("share"),
        ] {
            if candidate.join("capnp").join("stream.capnp").is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(ancestor) {
            for entry in entries.flatten().filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("capnproto-c++-")
            }) {
                let candidate = entry.path().join("src");
                if candidate.join("capnp").join("stream.capnp").is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Locate the `capnp` schema compiler: `CAPNP` env override, then `PATH`, then
/// (on Windows) the winget install tree.
fn capnp_executable() -> Option<PathBuf> {
    if let Some(path) = env::var_os("CAPNP") {
        return Some(PathBuf::from(path));
    }

    if let Some(path) = find_on_path("capnp.exe").or_else(|| find_on_path("capnp")) {
        return Some(path);
    }

    #[cfg(windows)]
    {
        let local_app_data = env::var_os("LOCALAPPDATA")?;
        let packages = PathBuf::from(local_app_data)
            .join("Microsoft")
            .join("WinGet")
            .join("Packages");
        let entries = std::fs::read_dir(packages).ok()?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if !name.starts_with("capnproto.capnproto_") {
                continue;
            }
            if let Some(path) = find_under(&entry.path(), "capnp.exe") {
                return Some(path);
            }
        }
    }

    None
}

fn find_on_path(binary: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())
}

#[cfg(windows)]
fn find_under(root: &std::path::Path, file_name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(file_name)
            {
                return Some(path);
            }
        }
    }
    None
}
