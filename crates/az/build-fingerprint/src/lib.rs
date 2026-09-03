//! Derive a deterministic fingerprint of everything that decides a crate's
//! compiled behaviour.
//!
//! Asset reprocessing decides staleness from a build rule's descriptor hash.
//! That hash can only see values a rule *declares*: a version counter and an
//! analysis fingerprint. Neither can see the code that turns analyzed data
//! into product bytes, so changing a codec, a parser, or an analysis pass
//! leaves previously processed products silently in place.
//!
//! Declared identity does not survive contact with a real codebase. A counter
//! is a promise someone must remember to keep, and both this workspace's rule
//! counters and its product-format constants have already drifted from the
//! values they were meant to track. This crate removes the human from the
//! loop: the fingerprint is read off the build inputs themselves, so it cannot
//! be forgotten.
//!
//! # Coverage
//!
//! Four inputs, each covering a way the same source tree can produce different
//! bytes:
//!
//! - **Sources.** Every `.rs` file under the crate's `src/`.
//! - **Manifest.** The crate's own `Cargo.toml` carries the edition,
//!   the feature definitions, the dependency declarations, and the per-
//!   dependency feature selections.
//! - **Activated features.** The feature set Cargo resolved for *this* build,
//!   which the manifest cannot show because a dependent decides it.
//! - **Resolved dependencies.** The transitive closure of everything outside
//!   the workspace, read back out of `Cargo.lock`. A source hash cannot see a
//!   `gltf` or `ron` bump, and Cargo hands a build script nothing about its
//!   own resolution, so the lock file is the only place the answer exists.
//!   Optional dependencies this build did not activate are cut, because a
//!   builder's fingerprint should not be a fingerprint of an engine it is not
//!   linking. See [`dependencies`].
//!
//! Generated code is covered too, but only through [`generate_and_emit`]:
//! hashing `OUT_DIR` from [`emit`] would read the *previous* build's output
//! and be one build stale, which is under-invalidation wearing a disguise.
//! [`emit`] therefore refuses to run when `OUT_DIR` is not empty, so a crate
//! that starts generating code fails loudly instead of quietly losing
//! coverage.
//!
//! Still outside the fingerprint, deliberately: the compiler version, the
//! profile, `RUSTFLAGS`, and the target triple. Those describe the machine
//! that ran the build rather than the code it built, and folding them in would
//! reprocess every asset on a toolchain bump or a debug/release switch.
//! Workspace crates reached as path dependencies are also outside it. Their
//! code is covered where they publish their own fingerprint, and a build rule
//! composes the ones that frame its bytes. A vendored `[patch]` redirect is
//! not a workspace crate: it enters the closure by name and version, though
//! its checked-in sources are not hashed.
//!
//! # Cost
//!
//! Hashing whole files over-invalidates: a comment-only edit changes the
//! fingerprint and reprocesses that rule's assets. Narrower schemes were
//! rejected because they reintroduce the failure being fixed. Hashing only
//! declared type structure misses hand-rolled byte writers entirely, and
//! hashing a golden encode output misses any code path the fixture does not
//! exercise. Over-invalidation is a bounded cost; under-invalidation is a
//! silent stale product.
//!
//! # Determinism
//!
//! The same inputs hash identically on every platform. Paths enter the hash as
//! `/`-separated relative strings, entries are sorted by that string, `\r\n`
//! is normalized to `\n` so a checkout configured for CRLF agrees with one
//! configured for LF, and the dependency closure and feature list are sorted
//! before hashing.
//!
//! # Usage
//!
//! ```ignore
//! // build.rs
//! fn main() {
//!     az_build_fingerprint::emit();
//! }
//! ```
//!
//! ```ignore
//! // build.rs, for a crate that generates code
//! fn main() {
//!     az_build_fingerprint::generate_and_emit(|out_dir| write_tables(out_dir));
//! }
//! ```
//!
//! ```ignore
//! // src/lib.rs
//! pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");
//! ```

mod dependencies;

use std::{
    collections::BTreeSet,
    env, fs, io,
    path::{Path, PathBuf},
};

use toml::Value;

/// Environment variable the build script exposes to the crate being built.
pub const SOURCE_FINGERPRINT_ENV: &str = "AZ_SOURCE_FINGERPRINT";

/// Domain marker distinguishing these fingerprints from the other blake3
/// digests this workspace stores.
///
/// The marker names the digest family, a crate's own build inputs, not the
/// precise list of inputs. Widening that list moves every value it produces
/// anyway, so the list needs no version of its own.
pub const SOURCE_FINGERPRINT_DOMAIN: &str = "azoth.source-tree/v1:";

/// Hash the calling crate's build inputs and expose the result as
/// [`SOURCE_FINGERPRINT_ENV`], registering every input for rebuild tracking.
///
/// # Panics
///
/// Panics if Cargo's build-script environment is missing, if an input cannot
/// be read, if no `Cargo.lock` can be found above the crate, or if `OUT_DIR`
/// already holds generated files. Use [`generate_and_emit`] for that. A build
/// script that cannot see all of its inputs must fail loudly: emitting a
/// fingerprint that does not describe the build would reintroduce exactly the
/// silent staleness this crate exists to prevent.
pub fn emit() {
    let out_dir = out_dir();
    let generated = collect_files(&out_dir, &out_dir, &|_| true).unwrap_or_else(|error| {
        panic!(
            "failed to inspect `{}` for generated code: {error}",
            out_dir.display()
        )
    });
    assert!(
        generated.is_empty(),
        "`{}` already holds generated files, so `emit` would hash the previous build's output \
         and report a fingerprint one build stale. Call `generate_and_emit` instead, which \
         generates first and hashes after.",
        out_dir.display()
    );
    emit_with_generated(&generated);
}

/// Run a code generator into `OUT_DIR`, then hash the generated files along
/// with the crate's other build inputs.
///
/// Generation happens inside this call so that it cannot be ordered after the
/// hash by accident. Generated files are not registered for rebuild tracking:
/// the build script rewrites them on every run, so watching them would rebuild
/// forever. They are reached through the inputs that produced them.
///
/// # Panics
///
/// Panics under the same conditions as [`emit`], minus the empty-`OUT_DIR`
/// requirement.
pub fn generate_and_emit(generate: impl FnOnce(&Path)) {
    let out_dir = out_dir();
    generate(&out_dir);
    let generated = collect_files(&out_dir, &out_dir, &|_| true).unwrap_or_else(|error| {
        panic!(
            "failed to walk `{}` for generated code: {error}",
            out_dir.display()
        )
    });
    emit_with_generated(&generated);
}

fn emit_with_generated(generated: &[(String, PathBuf)]) {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR for build scripts"),
    );
    let source_root = manifest_dir.join("src");
    let manifest_path = manifest_dir.join("Cargo.toml");

    let sources = collect_files(&source_root, &source_root, &|path| {
        path.extension().is_some_and(|extension| extension == "rs")
    })
    .unwrap_or_else(|error| {
        panic!(
            "failed to walk `{}` for a source fingerprint: {error}",
            source_root.display()
        )
    });

    let manifest_text = read_normalized(&manifest_path);
    let manifest = parse_toml(&manifest_path, &manifest_text);

    let lock_path = locate_lock_file(&manifest_dir);
    let workspace_root = lock_path
        .parent()
        .expect("a located lock file has a parent directory")
        .to_path_buf();
    let lock_text = read_normalized(&lock_path);
    let lock = parse_toml(&lock_path, &lock_text);

    let features = activated_features();
    let closure = dependencies::resolved_closure(
        &lock,
        &package_environment("CARGO_PKG_NAME"),
        &package_environment("CARGO_PKG_VERSION"),
        &dependencies::excluded_root_dependencies(&manifest, &features),
        &dependencies::workspace_member_names(&workspace_root),
    );

    let mut hasher = blake3::Hasher::new();
    hash_text(&mut hasher, SOURCE_FINGERPRINT_DOMAIN);
    hash_files(&mut hasher, "sources", &sources);
    hash_section(&mut hasher, "manifest", 1);
    hash_bytes(&mut hasher, &manifest_text);
    let features = features.into_iter().collect::<Vec<_>>();
    hash_list(&mut hasher, "features", &features);
    hash_list(&mut hasher, "dependencies", &closure);
    hash_files(&mut hasher, "generated", generated);

    for (_, path) in &sources {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    // Watch the tree itself so added and removed files re-run the script, the
    // manifest and the lock file because an explicit watch list turns off
    // Cargo's default of watching the whole package, and the script itself so
    // a change to the hashing rule takes effect.
    println!("cargo:rerun-if-changed={}", source_root.display());
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    println!("cargo:rerun-if-changed={}", lock_path.display());
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("Cargo.toml").display()
    );
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rustc-env={SOURCE_FINGERPRINT_ENV}={SOURCE_FINGERPRINT_DOMAIN}{}",
        hasher.finalize().to_hex()
    );
}

fn out_dir() -> PathBuf {
    PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR for build scripts"))
}

fn package_environment(key: &str) -> String {
    env::var(key).unwrap_or_else(|_| panic!("cargo sets {key} for build scripts"))
}

fn parse_toml(path: &Path, bytes: &[u8]) -> Value {
    let text = std::str::from_utf8(bytes).unwrap_or_else(|error| {
        panic!(
            "`{}` is not valid UTF-8 and cannot be parsed as TOML: {error}",
            path.display()
        )
    });
    toml::from_str(text)
        .unwrap_or_else(|error| panic!("failed to parse `{}`: {error}", path.display()))
}

/// The features Cargo activated for this build.
///
/// `CARGO_CFG_FEATURE` is already the transitive closure of activated
/// features, so no feature graph has to be walked here. A crate compiled with
/// different features is different code and earns a different fingerprint;
/// the same set also decides which optional dependencies are real for this
/// build.
fn activated_features() -> BTreeSet<String> {
    env::var("CARGO_CFG_FEATURE")
        .unwrap_or_default()
        .split(',')
        .filter(|feature| !feature.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Walk up from the crate directory to the `Cargo.lock` that resolved it.
///
/// # Panics
///
/// Panics when no lock file exists above the crate. The dependency surface is
/// not optional coverage, so a build with no resolution to read must stop
/// rather than emit a fingerprint that quietly omits it.
fn locate_lock_file(manifest_dir: &Path) -> PathBuf {
    for directory in manifest_dir.ancestors() {
        let candidate = directory.join("Cargo.lock");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "no `Cargo.lock` above `{}`; the resolved dependency surface cannot be derived without it",
        manifest_dir.display()
    )
}

fn collect_files(
    root: &Path,
    directory: &Path,
    accept: &dyn Fn(&Path) -> bool,
) -> io::Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    collect_into(root, directory, accept, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn collect_into(
    root: &Path,
    directory: &Path,
    accept: &dyn Fn(&Path) -> bool,
    files: &mut Vec<(String, PathBuf)>,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_into(root, &path, accept, files)?;
        } else if accept(&path) {
            files.push((relative_hash_path(root, &path), path));
        }
    }
    Ok(())
}

fn read_normalized(path: &Path) -> Vec<u8> {
    let contents = fs::read(path).unwrap_or_else(|error| {
        panic!(
            "failed to read `{}` for a fingerprint: {error}",
            path.display()
        )
    });
    normalize_newlines(contents)
}

/// Render a path as the `/`-separated string that enters the hash, so a
/// Windows tree and a Linux tree agree.
fn relative_hash_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_newlines(bytes: Vec<u8>) -> Vec<u8> {
    if !bytes.contains(&b'\r') {
        return bytes;
    }
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
            continue;
        }
        normalized.push(bytes[index]);
        index += 1;
    }
    normalized
}

fn hash_files(hasher: &mut blake3::Hasher, label: &str, files: &[(String, PathBuf)]) {
    hash_section(hasher, label, files.len());
    for (relative_path, absolute_path) in files {
        hash_text(hasher, relative_path);
        hash_bytes(hasher, &read_normalized(absolute_path));
    }
}

fn hash_list(hasher: &mut blake3::Hasher, label: &str, entries: &[String]) {
    hash_section(hasher, label, entries.len());
    for entry in entries {
        hash_text(hasher, entry);
    }
}

/// Open a labelled section, so widening one input cannot produce the digest of
/// a different arrangement of another.
fn hash_section(hasher: &mut blake3::Hasher, label: &str, count: usize) {
    hash_text(hasher, label);
    hash_len(hasher, count);
}

fn hash_bytes(hasher: &mut blake3::Hasher, value: &[u8]) {
    hash_len(hasher, value.len());
    hasher.update(value);
}

fn hash_text(hasher: &mut blake3::Hasher, value: &str) {
    hash_bytes(hasher, value.as_bytes());
}

fn hash_len(hasher: &mut blake3::Hasher, value: usize) {
    let value = u64::try_from(value).expect("fingerprint lengths fit in u64");
    hasher.update(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_and_lf_sources_hash_identically() {
        assert_eq!(
            normalize_newlines(b"fn main() {\r\n}\r\n".to_vec()),
            normalize_newlines(b"fn main() {\n}\n".to_vec()),
        );
    }

    #[test]
    fn a_lone_carriage_return_is_preserved() {
        // Only line terminators are normalized. A `\r` inside a string
        // literal is real content and must keep changing the fingerprint.
        assert_eq!(
            normalize_newlines(b"\"a\rb\"".to_vec()),
            b"\"a\rb\"".to_vec()
        );
    }

    #[test]
    fn hash_text_is_length_prefixed_so_concatenations_stay_distinct() {
        let mut joined = blake3::Hasher::new();
        hash_text(&mut joined, "ab");
        hash_text(&mut joined, "c");

        let mut split = blake3::Hasher::new();
        hash_text(&mut split, "a");
        hash_text(&mut split, "bc");

        assert_ne!(joined.finalize(), split.finalize());
    }

    #[test]
    fn relative_paths_use_forward_slashes_on_every_platform() {
        let root = Path::new("crate").join("src");
        let nested = root.join("codec").join("mod.rs");
        assert_eq!(relative_hash_path(&root, &nested), "codec/mod.rs");
    }

    #[test]
    fn an_empty_section_still_marks_its_label() {
        // Two adjacent empty sections must not collapse into one, or widening
        // the input list could reproduce an older digest.
        let mut labelled = blake3::Hasher::new();
        hash_section(&mut labelled, "features", 0);
        hash_section(&mut labelled, "dependencies", 0);

        let mut single = blake3::Hasher::new();
        hash_section(&mut single, "features", 0);

        assert_ne!(labelled.finalize(), single.finalize());
    }

    #[test]
    fn a_moved_list_entry_changes_the_digest() {
        let before_entries = vec!["gltf 1.4.1".to_owned()];
        let mut before = blake3::Hasher::new();
        hash_list(&mut before, "dependencies", &before_entries);

        let after_entries = vec!["gltf 1.5.0".to_owned()];
        let mut after = blake3::Hasher::new();
        hash_list(&mut after, "dependencies", &after_entries);

        assert_ne!(before.finalize(), after.finalize());
    }

    #[test]
    fn an_unreadable_tree_is_an_error_rather_than_an_empty_hash() {
        // Silently hashing nothing is the failure this crate exists to
        // prevent, so a tree that cannot be walked has to surface.
        let absent = Path::new("does-not-exist-anywhere");
        assert!(collect_files(absent, absent, &|_| true).is_err());
    }
}
