//! Generated id-constant crates for the units that mint ids.
//!
//! A gem's crate is derived from its `gem.toml` and committed beside it at
//! `gems/<name>/ids`; the engine's is derived from `engine.toml` and committed
//! at `crates/engine/ids`. One emitter writes both, so the engine's output is
//! the gem's shape with the noun, the manifest, the command, and the const the
//! unit's own id binds to swapped, and nothing else (asset-contract ticket
//! 014, D3). A second emitter would be a second thing to keep in step.
//!
//! Generated `Cargo.toml` files always depend on `az-gem-contract` via
//! `{ workspace = true }`. Engine and project workspaces both declare that
//! workspace dependency (path member vs version + patch table); the emitted
//! manifest does not need a dependency-style enum.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::atomic_write::atomic_write;
use crate::manifest::{EngineManifest, GemManifest, ProjectManifestError};

const CARGO_TOML_NAME: &str = "Cargo.toml";
const LIB_RS_RELATIVE: &str = "src/lib.rs";
const FINGERPRINT_PREFIX: &str = "//! fingerprint: ";

/// Where the engine's own ids crate is committed, relative to the engine root.
///
/// Beside the engine's contribution bundles rather than under `gems/`: the
/// engine is not one of its own gems, and `crates/engine/` is where the
/// bundles that consume these consts live.
pub const ENGINE_IDS_ROOT: &str = "crates/engine/ids";

/// The engine's generated ids package. Fixed rather than derived from the
/// engine id: there is exactly one engine, and `azoth-ids` would read as a
/// gem's crate.
const ENGINE_IDS_PACKAGE: &str = "az-engine-ids";

/// How a generated ids-crate file compares to what is already on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdsFileStatus {
    /// Desired contents were written.
    Written,
    /// On-disk contents already matched.
    Unchanged,
}

/// Result of generating one unit's ids crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdsCrateOutcome {
    /// Cargo package name: `<gem-id-with-dots-as-dashes>-ids` for a gem,
    /// `az-engine-ids` for the engine.
    pub package_name: String,
    /// Status of `ids/Cargo.toml`.
    pub cargo_toml: IdsFileStatus,
    /// Status of `ids/src/lib.rs`.
    pub lib_rs: IdsFileStatus,
}

impl IdsCrateOutcome {
    /// True when both generated files already matched disk.
    #[must_use]
    pub const fn is_unchanged(&self) -> bool {
        matches!(self.cargo_toml, IdsFileStatus::Unchanged)
            && matches!(self.lib_rs, IdsFileStatus::Unchanged)
    }
}

/// Failures while mapping gem ids or writing the generated crate.
#[derive(Debug, Error)]
pub enum IdsGenerationError {
    #[error("gem `{gem_id}` {namespace} ids {ids} map to the same const ident `{ident}`")]
    CollidingIdents {
        gem_id: String,
        namespace: &'static str,
        ids: String,
        ident: String,
    },

    #[error("gem `{gem_id}` {namespace} id `{id}` maps to invalid const ident `{ident}`")]
    InvalidIdent {
        gem_id: String,
        namespace: &'static str,
        id: String,
        ident: String,
    },

    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Writes `<gem_root>/ids/Cargo.toml` and `<gem_root>/ids/src/lib.rs` when they
/// differ from the desired contents.
///
/// # Errors
///
/// Returns [`IdsGenerationError`] when two ids collapse to one const ident, a
/// mapped ident is not a valid Rust constant name, or a generated file cannot
/// be read or written.
pub fn generate_gem_ids_crate(
    manifest: &GemManifest,
    gem_root: &Path,
) -> Result<IdsCrateOutcome, IdsGenerationError> {
    write_ids_crate(
        &Unit::gem(manifest),
        &IdsCratePaths::from_gem_root(gem_root),
    )
}

/// Writes the engine's own ids crate beneath `engine_root`.
///
/// The engine mints ids the same way a gem does and through the same emitter;
/// what it does not have is a catalog that could disable it, so the crate is a
/// fixed member of the engine workspace rather than one the lock closure
/// selects (asset-contract ticket 014, D3/D6).
///
/// # Errors
///
/// Returns [`IdsGenerationError`] when two ids collapse to one const ident, a
/// mapped ident is not a valid Rust constant name, or a generated file cannot
/// be read or written.
pub fn generate_engine_ids_crate(
    manifest: &EngineManifest,
    engine_root: &Path,
) -> Result<IdsCrateOutcome, IdsGenerationError> {
    write_ids_crate(
        &Unit::engine(manifest),
        &IdsCratePaths::from_engine_root(engine_root),
    )
}

/// Returns whether the on-disk engine ids crate matches the desired contents.
///
/// # Errors
///
/// Returns [`IdsGenerationError`] for ident mapping failures or I/O errors
/// other than a missing file.
pub fn check_engine_ids_crate(
    manifest: &EngineManifest,
    engine_root: &Path,
) -> Result<bool, IdsGenerationError> {
    matches_disk(
        &Unit::engine(manifest),
        &IdsCratePaths::from_engine_root(engine_root),
    )
}

fn write_ids_crate(
    unit: &Unit<'_>,
    paths: &IdsCratePaths,
) -> Result<IdsCrateOutcome, IdsGenerationError> {
    let rendered = render_ids_crate(unit)?;
    Ok(IdsCrateOutcome {
        package_name: rendered.package_name,
        cargo_toml: write_if_changed(&paths.cargo_toml, rendered.cargo_toml.as_bytes())?,
        lib_rs: write_if_changed(&paths.lib_rs, rendered.lib_rs.as_bytes())?,
    })
}

fn matches_disk(unit: &Unit<'_>, paths: &IdsCratePaths) -> Result<bool, IdsGenerationError> {
    let rendered = render_ids_crate(unit)?;
    Ok(
        file_matches(&paths.cargo_toml, rendered.cargo_toml.as_bytes())?
            && file_matches(&paths.lib_rs, rendered.lib_rs.as_bytes())?,
    )
}

/// Returns whether the on-disk ids crate matches the desired generated contents.
///
/// Missing files count as stale (`false`).
///
/// # Errors
///
/// Returns [`IdsGenerationError`] for ident mapping failures or I/O errors
/// other than a missing file.
pub fn check_gem_ids_crate(
    manifest: &GemManifest,
    gem_root: &Path,
) -> Result<bool, IdsGenerationError> {
    matches_disk(
        &Unit::gem(manifest),
        &IdsCratePaths::from_gem_root(gem_root),
    )
}

struct IdsCratePaths {
    cargo_toml: PathBuf,
    lib_rs: PathBuf,
}

impl IdsCratePaths {
    fn from_gem_root(gem_root: &Path) -> Self {
        Self::at(&gem_root.join("ids"))
    }

    fn from_engine_root(engine_root: &Path) -> Self {
        Self::at(&engine_root.join(ENGINE_IDS_ROOT))
    }

    fn at(ids_root: &Path) -> Self {
        Self {
            cargo_toml: ids_root.join(CARGO_TOML_NAME),
            lib_rs: ids_root.join(LIB_RS_RELATIVE),
        }
    }
}

/// One manifested unit that mints ids, as the emitter needs it.
///
/// A gem and the engine differ in four facts and nothing else: the noun the
/// generated prose uses, the manifest that prose names, the command that
/// writes the crate, and the const the unit's own id is bound to. Everything
/// past this struct is one code path, which is what makes "byte-identical in
/// shape" a property rather than a promise.
struct Unit<'a> {
    id: &'a str,
    version: &'a str,
    package: String,
    /// The const the unit's own id is bound to.
    ident: &'static str,
    /// How the unit reads in generated prose.
    noun: &'static str,
    /// The manifest the ids come from.
    manifest: &'static str,
    /// The manifest as the generated `Cargo.toml` header points at it.
    from: &'static str,
    /// The command that writes this crate.
    command: &'static str,
    contributions: Vec<&'a str>,
    capabilities: Vec<&'a str>,
}

impl<'a> Unit<'a> {
    fn gem(manifest: &'a GemManifest) -> Self {
        Self {
            id: &manifest.gem.id,
            version: &manifest.gem.version,
            package: format!("{}-ids", manifest.gem.id.replace('.', "-")),
            ident: "GEM",
            noun: "gem",
            manifest: crate::manifest::GEM_MANIFEST_FILE,
            from: "../gem.toml",
            command: "azoth gem ids",
            contributions: manifest
                .contributions
                .iter()
                .map(|contribution| contribution.id.as_str())
                .collect(),
            capabilities: manifest.capabilities.keys().map(String::as_str).collect(),
        }
    }

    fn engine(manifest: &'a EngineManifest) -> Self {
        Self {
            id: &manifest.engine.id,
            version: &manifest.engine.version,
            package: ENGINE_IDS_PACKAGE.to_string(),
            ident: "ENGINE",
            noun: "engine",
            manifest: crate::manifest::ENGINE_MANIFEST_FILE,
            from: "../../../engine.toml",
            command: "azoth gem ids --engine",
            contributions: manifest
                .contributions
                .iter()
                .map(|contribution| contribution.id.as_str())
                .collect(),
            // The engine floor is not selectable, so there is nothing here to
            // name: no catalog offers it and no project switches it off.
            capabilities: Vec::new(),
        }
    }
}

struct RenderedIdsCrate {
    package_name: String,
    cargo_toml: String,
    lib_rs: String,
}

fn render_ids_crate(unit: &Unit<'_>) -> Result<RenderedIdsCrate, IdsGenerationError> {
    let named = named_ids(unit)?;
    Ok(RenderedIdsCrate {
        package_name: unit.package.clone(),
        cargo_toml: render_cargo_toml(unit),
        lib_rs: render_lib_rs(unit, &named),
    })
}

fn named_ids(unit: &Unit<'_>) -> Result<NamedIds, IdsGenerationError> {
    Ok(NamedIds {
        contributions: map_namespace(unit.id, "contribution", unit.contributions.iter().copied())?,
        capabilities: map_namespace(unit.id, "capability", unit.capabilities.iter().copied())?,
    })
}

#[derive(Debug)]
struct NamedIds {
    contributions: Vec<(String, String)>,
    capabilities: Vec<(String, String)>,
}

fn map_namespace<'a>(
    gem_id: &str,
    namespace: &'static str,
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<(String, String)>, IdsGenerationError> {
    let ids = ids.into_iter().collect::<BTreeSet<_>>();
    let mut by_ident = BTreeMap::<String, Vec<String>>::new();
    for id in &ids {
        let ident = const_ident(id);
        if !is_valid_const_ident(&ident) {
            return Err(IdsGenerationError::InvalidIdent {
                gem_id: gem_id.to_string(),
                namespace,
                id: (*id).to_string(),
                ident,
            });
        }
        by_ident.entry(ident).or_default().push((*id).to_string());
    }

    if let Some((ident, colliding)) = by_ident.iter().find(|(_, names)| names.len() > 1) {
        return Err(IdsGenerationError::CollidingIdents {
            gem_id: gem_id.to_string(),
            namespace,
            ids: quoted_id_list(colliding),
            ident: ident.clone(),
        });
    }

    Ok(ids
        .into_iter()
        .map(|id| (id.to_string(), const_ident(id)))
        .collect())
}

/// The entry item a contribution crate exports (ADR 0036 static flavor).
///
/// `<contribution id with separators as underscores>_contribution`, taking
/// nothing and returning `impl Contribution`: the item is identity, and what
/// the project selected reaches the contribution as the second argument of
/// `Composer::add`. Generated target glue calls it by name, so a contribution
/// crate that never grew an entry item is `E0425` in the generated target: the
/// direct named call is the inventory, the anchor, and the link-time check at
/// once.
#[must_use]
pub fn entry_ident(contribution_id: &str) -> String {
    format!(
        "{}_contribution",
        const_ident(contribution_id).to_ascii_lowercase()
    )
}

/// Whether an ident derived from a manifest id can be spelled in Rust.
///
/// Manifest ids are laxer than Rust idents — they may start with a digit — so
/// the generator checks before emitting a call it would otherwise render
/// unparseable.
#[must_use]
pub fn is_rust_ident(ident: &str) -> bool {
    is_valid_const_ident(ident)
}

fn const_ident(id: &str) -> String {
    id.chars()
        .map(|ch| match ch {
            '-' | '.' | '_' => '_',
            other => other,
        })
        .collect::<String>()
        .to_ascii_uppercase()
}

fn is_valid_const_ident(ident: &str) -> bool {
    let mut chars = ident.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {
            chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

fn quoted_id_list(ids: &[String]) -> String {
    debug_assert!(
        ids.len() >= 2,
        "colliding id lists always name at least two ids"
    );
    match ids {
        [left, right] => format!("`{left}` and `{right}`"),
        [rest @ .., last] => {
            let leading = rest
                .iter()
                .map(|id| format!("`{id}`"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{leading}, and `{last}`")
        }
        [] => unreachable!("colliding id lists always name at least two ids"),
    }
}

fn render_cargo_toml(unit: &Unit<'_>) -> String {
    format!(
        "\
# Generated by `{command}` from {from} — do not edit.

[package]
name = \"{package}\"
version = \"{version}\"
edition.workspace = true
description = \"Generated id constants for {noun} {id}\"

[lints]
workspace = true

[dependencies]
az-gem-contract = {{ workspace = true }}
",
        command = unit.command,
        from = unit.from,
        package = unit.package,
        version = unit.version,
        noun = unit.noun,
        id = unit.id,
    )
}

fn render_lib_rs(unit: &Unit<'_>, named: &NamedIds) -> String {
    let mut docs = String::new();
    docs.push_str("//! Generated id constants for ");
    docs.push_str(unit.noun);
    docs.push_str(" `");
    docs.push_str(unit.id);
    docs.push_str("`.\n");
    docs.push_str("//!\n");
    docs.push_str("//! Generated by `");
    docs.push_str(unit.command);
    docs.push_str("` from `");
    docs.push_str(unit.manifest);
    docs.push_str("` — do not edit by hand.\n");

    let mut body = String::new();
    body.push('\n');
    body.push_str("use az_gem_contract::GemId;\n");
    body.push('\n');
    body.push_str("/// `");
    body.push_str(unit.id);
    body.push_str("`\n");
    body.push_str("pub const ");
    body.push_str(unit.ident);
    body.push_str(": GemId = GemId::new(\"");
    body.push_str(unit.id);
    body.push_str("\");\n");

    if !named.contributions.is_empty() {
        body.push('\n');
        body.push_str("/// Contribution ids declared by `");
        body.push_str(unit.manifest);
        body.push_str("`.\n");
        body.push_str("pub mod contributions {\n");
        body.push_str("    use az_gem_contract::ContributionId;\n");
        for (id, ident) in &named.contributions {
            body.push('\n');
            body.push_str("    /// `");
            body.push_str(id);
            body.push_str("`\n");
            body.push_str("    pub const ");
            body.push_str(ident);
            body.push_str(": ContributionId = ContributionId::new(\"");
            body.push_str(id);
            body.push_str("\");\n");
        }
        body.push_str("}\n");
    }

    if !named.capabilities.is_empty() {
        body.push('\n');
        body.push_str("/// Capability ids declared by `");
        body.push_str(unit.manifest);
        body.push_str("`.\n");
        body.push_str("pub mod capabilities {\n");
        body.push_str("    use az_gem_contract::CapabilityId;\n");
        for (id, ident) in &named.capabilities {
            body.push('\n');
            body.push_str("    /// `");
            body.push_str(id);
            body.push_str("`\n");
            body.push_str("    pub const ");
            body.push_str(ident);
            body.push_str(": CapabilityId = CapabilityId::new(\"");
            body.push_str(id);
            body.push_str("\");\n");
        }
        body.push_str("}\n");
    }

    let mut hashed = String::with_capacity(docs.len() + body.len());
    hashed.push_str(&docs);
    hashed.push_str(&body);
    let fingerprint = blake3::hash(hashed.as_bytes());

    let mut output = docs;
    output.push_str(FINGERPRINT_PREFIX);
    output.push_str("blake3:");
    output.push_str(&fingerprint.to_hex());
    output.push('\n');
    output.push_str(&body);
    output
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<IdsFileStatus, IdsGenerationError> {
    if file_matches(path, contents)? {
        return Ok(IdsFileStatus::Unchanged);
    }
    atomic_write(path, contents).map_err(|error| map_write_error(path, error))?;
    Ok(IdsFileStatus::Written)
}

fn map_write_error(path: &Path, error: ProjectManifestError) -> IdsGenerationError {
    match error {
        ProjectManifestError::Write { source, .. } => IdsGenerationError::Write {
            path: path.to_path_buf(),
            source,
        },
        other => IdsGenerationError::Write {
            path: path.to_path_buf(),
            source: std::io::Error::other(other.to_string()),
        },
    }
}

fn file_matches(path: &Path, desired: &[u8]) -> Result<bool, IdsGenerationError> {
    match std::fs::read(path) {
        Ok(existing) => Ok(existing == desired),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(IdsGenerationError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::GemCapability;
    use crate::manifest::GemContribution;
    use az_gem_contract::GemTargetRole;

    #[test]
    fn const_ident_maps_kebab_and_dotted_ids() {
        assert_eq!(const_ident("prefab-types"), "PREFAB_TYPES");
        assert_eq!(const_ident("azoth.vegetation"), "AZOTH_VEGETATION");
        assert_eq!(const_ident("runtime_client"), "RUNTIME_CLIENT");
        assert_eq!(const_ident("foo-bar.baz_qux"), "FOO_BAR_BAZ_QUX");
    }

    #[test]
    fn colliding_ids_error_names_both_ids() {
        let mut manifest = GemManifest::new("local.example", "Example", "0.1.0");
        manifest.contributions = vec![
            GemContribution::code("foo-bar", "example", [GemTargetRole::Client]),
            GemContribution::code("foo.bar", "example", [GemTargetRole::Client]),
        ];

        let error = named_ids(&Unit::gem(&manifest)).unwrap_err();
        let IdsGenerationError::CollidingIdents {
            gem_id,
            namespace,
            ids,
            ident,
        } = error
        else {
            panic!("expected colliding idents, got {error:?}");
        };
        assert_eq!(gem_id, "local.example");
        assert_eq!(namespace, "contribution");
        assert_eq!(ident, "FOO_BAR");
        assert!(ids.contains("`foo-bar`"), "{ids}");
        assert!(ids.contains("`foo.bar`"), "{ids}");
    }

    #[test]
    fn digit_leading_ident_error_names_the_id() {
        let mut manifest = GemManifest::new("local.example", "Example", "0.1.0");
        manifest.contributions = vec![GemContribution::code(
            "1leading",
            "example",
            [GemTargetRole::Client],
        )];

        let error = named_ids(&Unit::gem(&manifest)).unwrap_err();
        let IdsGenerationError::InvalidIdent {
            gem_id,
            namespace,
            id,
            ident,
        } = error
        else {
            panic!("expected invalid ident, got {error:?}");
        };
        assert_eq!(gem_id, "local.example");
        assert_eq!(namespace, "contribution");
        assert_eq!(id, "1leading");
        assert_eq!(ident, "1LEADING");
    }

    #[test]
    fn full_content_snapshot_includes_contributions_and_capabilities() {
        let rendered = render_ids_crate(&Unit::gem(&snapshot_manifest())).unwrap();
        assert_eq!(
            rendered.package_name, "azoth-vegetation-ids",
            "{}",
            rendered.package_name
        );
        assert_eq!(rendered.cargo_toml, SNAPSHOT_CARGO_TOML);
        assert_eq!(
            strip_fingerprint_line(&rendered.lib_rs),
            SNAPSHOT_LIB_RS_WITHOUT_FINGERPRINT
        );
        assert!(
            rendered
                .lib_rs
                .lines()
                .any(|line| line.starts_with("//! fingerprint: blake3:")
                    && line.len() == FINGERPRINT_PREFIX.len() + "blake3:".len() + 64),
            "{}",
            rendered.lib_rs
        );
        assert_eq!(
            render_ids_crate(&Unit::gem(&snapshot_manifest()))
                .unwrap()
                .lib_rs,
            rendered.lib_rs,
            "fingerprint must be deterministic"
        );
    }

    #[test]
    fn generate_twice_reports_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let gem_root = temp.path();
        let manifest = snapshot_manifest();

        let first = generate_gem_ids_crate(&manifest, gem_root).unwrap();
        assert_eq!(first.cargo_toml, IdsFileStatus::Written);
        assert_eq!(first.lib_rs, IdsFileStatus::Written);
        assert!(!first.is_unchanged());

        let second = generate_gem_ids_crate(&manifest, gem_root).unwrap();
        assert_eq!(second.cargo_toml, IdsFileStatus::Unchanged);
        assert_eq!(second.lib_rs, IdsFileStatus::Unchanged);
        assert!(second.is_unchanged());
    }

    #[test]
    fn check_mode_detects_fresh_and_mutated_files() {
        let temp = tempfile::tempdir().unwrap();
        let gem_root = temp.path();
        let manifest = snapshot_manifest();

        assert!(!check_gem_ids_crate(&manifest, gem_root).unwrap());
        generate_gem_ids_crate(&manifest, gem_root).unwrap();
        assert!(check_gem_ids_crate(&manifest, gem_root).unwrap());

        let lib_path = gem_root.join("ids/src/lib.rs");
        let mut bytes = std::fs::read(&lib_path).unwrap();
        bytes.push(b'X');
        std::fs::write(&lib_path, bytes).unwrap();
        assert!(!check_gem_ids_crate(&manifest, gem_root).unwrap());
    }

    #[test]
    fn empty_capabilities_omits_capabilities_module() {
        let mut manifest = GemManifest::new("azoth.vegetation", "Vegetation", "0.1.0");
        manifest.contributions = vec![GemContribution::code(
            "package",
            "az-gem-vegetation",
            [GemTargetRole::Client],
        )];

        let lib_rs = render_ids_crate(&Unit::gem(&manifest)).unwrap().lib_rs;
        assert!(!lib_rs.contains("pub mod capabilities"), "{lib_rs}");
        assert!(!lib_rs.contains("CapabilityId"), "{lib_rs}");
        assert!(lib_rs.contains("pub mod contributions"), "{lib_rs}");
    }

    /// The engine's crate is the gem's crate with four words swapped.
    ///
    /// Asserted as a diff rather than as a second snapshot: a second frozen
    /// copy would drift silently the first time the gem's shape changed, and
    /// the shape agreeing is the whole of D3.
    #[test]
    fn the_engine_crate_is_the_gem_crate_with_the_nouns_swapped() {
        let engine = render_ids_crate(&Unit::engine(&engine_manifest())).unwrap();
        assert_eq!(engine.package_name, "az-engine-ids");

        // The same contributions, declared by a gem instead of the engine.
        let mut twin = GemManifest::new("azoth.vegetation", "Vegetation", "0.1.0");
        twin.contributions = engine_manifest().contributions;
        let gem = render_ids_crate(&Unit::gem(&twin)).unwrap();

        assert_eq!(
            strip_fingerprint_line(&engine.lib_rs)
                .replace("engine `azoth`", "gem `azoth.vegetation`")
                .replace("azoth gem ids --engine", "azoth gem ids")
                .replace("engine.toml", "gem.toml")
                .replace(
                    "pub const ENGINE: GemId = GemId::new(\"azoth\");",
                    "pub const GEM: GemId = GemId::new(\"azoth.vegetation\");",
                )
                .replace("/// `azoth`\n", "/// `azoth.vegetation`\n"),
            strip_fingerprint_line(&gem.lib_rs),
            "{}",
            engine.lib_rs
        );
        assert_eq!(
            engine
                .cargo_toml
                .replace("az-engine-ids", "azoth-vegetation-ids")
                .replace("azoth gem ids --engine", "azoth gem ids")
                .replace("../../../engine.toml", "../gem.toml")
                .replace("for engine azoth", "for gem azoth.vegetation"),
            gem.cargo_toml,
            "{}",
            engine.cargo_toml
        );
    }

    /// Nothing selects the engine, so there is no capability namespace to
    /// generate — and the module is absent rather than empty.
    #[test]
    fn the_engine_crate_has_no_capability_ids() {
        let lib_rs = render_ids_crate(&Unit::engine(&engine_manifest()))
            .unwrap()
            .lib_rs;
        assert!(!lib_rs.contains("pub mod capabilities"), "{lib_rs}");
        assert!(lib_rs.contains("pub const ENGINE: GemId"), "{lib_rs}");
        assert!(lib_rs.contains("pub mod contributions"), "{lib_rs}");
        assert!(lib_rs.contains("pub const PREFAB_TYPES"), "{lib_rs}");
    }

    #[test]
    fn the_engine_crate_lands_beside_the_engine_bundles() {
        let temp = tempfile::tempdir().unwrap();
        let engine_root = temp.path();
        let manifest = engine_manifest();

        assert!(!check_engine_ids_crate(&manifest, engine_root).unwrap());
        let first = generate_engine_ids_crate(&manifest, engine_root).unwrap();
        assert_eq!(first.package_name, "az-engine-ids");
        assert_eq!(first.cargo_toml, IdsFileStatus::Written);
        assert!(
            engine_root
                .join(ENGINE_IDS_ROOT)
                .join("src/lib.rs")
                .is_file()
        );
        assert!(check_engine_ids_crate(&manifest, engine_root).unwrap());
        assert!(
            generate_engine_ids_crate(&manifest, engine_root)
                .unwrap()
                .is_unchanged()
        );
    }

    fn engine_manifest() -> EngineManifest {
        let mut manifest = EngineManifest::new("azoth", "Azoth", "0.1.0");
        manifest.contributions = vec![
            GemContribution::code("package", "az-engine-assets", [GemTargetRole::Client]),
            GemContribution::code(
                "prefab-types",
                "az-engine-types",
                [GemTargetRole::AssetWorker],
            ),
        ];
        manifest
    }

    fn snapshot_manifest() -> GemManifest {
        let mut manifest = GemManifest::new("azoth.vegetation", "Vegetation", "0.1.0");
        manifest.contributions = vec![
            GemContribution::code("package", "az-gem-vegetation", [GemTargetRole::Client]),
            GemContribution::code(
                "prefab-types",
                "az-gem-vegetation",
                [GemTargetRole::AssetWorker],
            ),
        ];
        manifest.capabilities.insert(
            "runtime-client".to_string(),
            GemCapability {
                label: None,
                description: None,
                default: false,
                contributions: vec!["package".to_string()],
                cargo_features: Vec::new(),
                activation: None,
            },
        );
        manifest
    }

    fn strip_fingerprint_line(lib_rs: &str) -> String {
        lib_rs
            .lines()
            .filter(|line| !line.starts_with(FINGERPRINT_PREFIX))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    const SNAPSHOT_CARGO_TOML: &str = "\
# Generated by `azoth gem ids` from ../gem.toml — do not edit.

[package]
name = \"azoth-vegetation-ids\"
version = \"0.1.0\"
edition.workspace = true
description = \"Generated id constants for gem azoth.vegetation\"

[lints]
workspace = true

[dependencies]
az-gem-contract = { workspace = true }
";

    const SNAPSHOT_LIB_RS_WITHOUT_FINGERPRINT: &str = "\
//! Generated id constants for gem `azoth.vegetation`.
//!
//! Generated by `azoth gem ids` from `gem.toml` — do not edit by hand.

use az_gem_contract::GemId;

/// `azoth.vegetation`
pub const GEM: GemId = GemId::new(\"azoth.vegetation\");

/// Contribution ids declared by `gem.toml`.
pub mod contributions {
    use az_gem_contract::ContributionId;

    /// `package`
    pub const PACKAGE: ContributionId = ContributionId::new(\"package\");

    /// `prefab-types`
    pub const PREFAB_TYPES: ContributionId = ContributionId::new(\"prefab-types\");
}

/// Capability ids declared by `gem.toml`.
pub mod capabilities {
    use az_gem_contract::CapabilityId;

    /// `runtime-client`
    pub const RUNTIME_CLIENT: CapabilityId = CapabilityId::new(\"runtime-client\");
}
";
}
