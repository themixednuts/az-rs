//! Derive the resolved third-party surface a crate compiles against.
//!
//! A source-tree hash sees only the code in the repository. It cannot see that
//! `gltf` moved from 1.4.1 to 1.5.0, or that `ron` picked up a different
//! serializer. Both change the bytes a builder writes. Cargo exposes
//! nothing about resolved dependencies to a build script (there is no `DEP_*`
//! entry for a dependency without a `links` key, and no version of any kind),
//! so the resolution has to be read back out of `Cargo.lock`.
//!
//! # What enters the closure
//!
//! Starting from the crate's own lock entry, every transitively reachable
//! package contributes its name, version, source, and checksum, except
//! workspace members, where traversal stops. A member's code is covered by its
//! own source fingerprint where it publishes one, and by the composition a
//! build rule performs over the crates that frame its bytes; walking through
//! it here would claim a coverage this crate does not own, and would drag in
//! that member's development dependencies, which Cargo merges into a member's
//! lock entry.
//!
//! Membership is answered from the workspace manifest, not from the absence of
//! a `source` key. Both a workspace crate and a vendored `[patch]` redirect
//! appear in the lock as a bare path package, so testing for a missing
//! `source` would silently drop vendored third-party code. In this workspace
//! that is `gpu-allocator` under four of the hashed crates and the whole
//! `bevy_mod_scripting` stack under one of them. Anything not identified as a
//! member is treated as third-party and walked, so an unrecognised path
//! package over-invalidates rather than disappearing.
//!
//! # Development and inactive dependencies are cut at the root
//!
//! Cargo merges a member's dev-dependencies into its lock entry but resolves
//! none for registry packages, so the root entry is the only place they can
//! enter, and it is the only place they are subtracted. Optional
//! dependencies that this build did not activate are subtracted there too:
//! they are not compiled, so they cannot reach the output bytes, and leaving
//! them in would make a builder's fingerprint a fingerprint of an engine it
//! is not linking.
//!
//! # Precision limits, all in the over-invalidating direction
//!
//! Optional dependencies are pruned only at the root, because `Cargo.lock`
//! records no feature resolution deeper than that. A bare dependency
//! reference that somehow matches several versions contributes all of them. An
//! optional dependency whose activation cannot be decided is kept. None of
//! these can hide a dependency that did change.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::Path,
};

use toml::Value;

const DEPENDENCY_SECTIONS: [&str; 2] = ["dependencies", "build-dependencies"];
const DEVELOPMENT_SECTION: &str = "dev-dependencies";

type DependencySection<'a> = (&'static str, Vec<(String, String, &'a Value)>);

/// Root dependencies that must not seed the closure.
///
/// Two disjoint reasons, unioned because the traversal treats them the same:
/// a development-only dependency is never compiled into the library, and an
/// unactivated optional dependency is not compiled at all.
pub fn excluded_root_dependencies(
    manifest: &Value,
    activated_features: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut excluded = development_only_names(manifest);
    excluded.extend(inactive_optional_names(manifest, activated_features));
    excluded
}

/// Names declared *only* under a development dependency section.
///
/// A name that also appears under `[dependencies]` or `[build-dependencies]`
/// is not returned: dropping it would hide a real dependency, and the whole
/// point of this module is that nothing gets hidden. Renames are followed
/// through the `package` key so the returned names are lock names.
fn development_only_names(manifest: &Value) -> BTreeSet<String> {
    let mut development = BTreeSet::new();
    let mut compiled = BTreeSet::new();

    for (section, entries) in dependency_sections(manifest) {
        let sink = if section == DEVELOPMENT_SECTION {
            &mut development
        } else {
            &mut compiled
        };
        for (_, lock_name, _) in entries {
            sink.insert(lock_name);
        }
    }

    development
        .into_iter()
        .filter(|name| !compiled.contains(name))
        .collect()
}

/// Optional dependencies that no activated feature turns on.
///
/// The activated set Cargo hands the build script is already transitively
/// closed, so a single scan over the features it names is enough. No feature
/// graph has to be walked. An optional dependency is kept whenever activation
/// is uncertain.
fn inactive_optional_names(
    manifest: &Value,
    activated_features: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut activated_dependencies = BTreeSet::new();
    if let Some(features) = manifest.get("features").and_then(Value::as_table) {
        for (feature, enables) in features {
            if !activated_features.contains(feature.as_str()) {
                continue;
            }
            let entries = enables.as_array().map(Vec::as_slice).unwrap_or_default();
            for entry in entries.iter().filter_map(Value::as_str) {
                if let Some(name) = activated_dependency(entry) {
                    activated_dependencies.insert(name.to_owned());
                }
            }
        }
    }

    let mut inactive = BTreeSet::new();
    for (section, entries) in dependency_sections(manifest) {
        if section == DEVELOPMENT_SECTION {
            continue;
        }
        for (declared_name, lock_name, specification) in entries {
            if specification.get("optional").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            // An optional dependency gets an implicit feature under its
            // declared name unless some feature references it as `dep:`.
            let implicitly_on = activated_features.contains(&declared_name);
            if !implicitly_on && !activated_dependencies.contains(&declared_name) {
                inactive.insert(lock_name);
            }
        }
    }
    inactive
}

/// The dependency a feature entry switches on, if any.
///
/// `dep:name` and `name/feature` both activate an optional dependency.
/// `name?/feature` deliberately does not activate it. That is the whole point
/// of the weak form, and a bare entry names another feature, not a dependency.
fn activated_dependency(entry: &str) -> Option<&str> {
    if let Some(name) = entry.strip_prefix("dep:") {
        return Some(name);
    }
    let (name, _) = entry.split_once('/')?;
    if name.ends_with('?') {
        None
    } else {
        Some(name)
    }
}

/// Every dependency declaration in the manifest, as
/// `(section, (declared name, lock name, specification))`.
///
/// Platform-scoped tables are folded in under their own section names so a
/// `[target.'cfg(windows)'.dependencies]` entry is treated like any other.
fn dependency_sections(manifest: &Value) -> Vec<DependencySection<'_>> {
    let mut sections = sections_of(manifest);
    if let Some(targets) = manifest.get("target").and_then(Value::as_table) {
        for platform in targets.values() {
            sections.extend(sections_of(platform));
        }
    }
    sections
}

fn sections_of(parent: &Value) -> Vec<DependencySection<'_>> {
    let mut found = Vec::new();
    for section in DEPENDENCY_SECTIONS.into_iter().chain([DEVELOPMENT_SECTION]) {
        let Some(table) = parent.get(section).and_then(Value::as_table) else {
            continue;
        };
        let entries = table
            .iter()
            .map(|(key, specification)| {
                let lock_name = specification
                    .get("package")
                    .and_then(Value::as_str)
                    .unwrap_or(key.as_str())
                    .to_owned();
                (key.clone(), lock_name, specification)
            })
            .collect::<Vec<_>>();
        found.push((section, entries));
    }
    found
}

/// Package names belonging to the workspace that owns this lock file.
///
/// # Panics
///
/// Panics if a declared member cannot be read. A member that is missing means
/// the membership answer is wrong, and a wrong answer here decides whether a
/// package's dependencies are hashed at all.
pub fn workspace_member_names(workspace_root: &Path) -> BTreeSet<String> {
    let manifest_path = workspace_root.join("Cargo.toml");
    let text = fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "failed to read workspace manifest `{}`: {error}",
            manifest_path.display()
        )
    });
    let manifest: Value = toml::from_str(&text)
        .unwrap_or_else(|error| panic!("failed to parse `{}`: {error}", manifest_path.display()));

    let mut names = BTreeSet::new();
    if let Some(name) = package_name(&manifest) {
        names.insert(name);
    }

    let members = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for member in members.iter().filter_map(Value::as_str) {
        for directory in expand_member_pattern(workspace_root, member) {
            let member_manifest = directory.join("Cargo.toml");
            let text = fs::read_to_string(&member_manifest).unwrap_or_else(|error| {
                panic!(
                    "failed to read workspace member manifest `{}`: {error}",
                    member_manifest.display()
                )
            });
            let parsed: Value = toml::from_str(&text).unwrap_or_else(|error| {
                panic!("failed to parse `{}`: {error}", member_manifest.display())
            });
            let name = package_name(&parsed).unwrap_or_else(|| {
                panic!(
                    "workspace member `{}` has no `[package].name`",
                    member_manifest.display()
                )
            });
            names.insert(name);
        }
    }
    names
}

fn package_name(manifest: &Value) -> Option<String> {
    manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Resolve a `members` entry, expanding `*` one directory component at a time.
fn expand_member_pattern(root: &Path, pattern: &str) -> Vec<std::path::PathBuf> {
    let mut candidates = vec![root.to_path_buf()];
    for component in pattern.split('/').filter(|component| !component.is_empty()) {
        let mut next = Vec::new();
        for candidate in candidates {
            if component == "*" {
                let entries = fs::read_dir(&candidate).unwrap_or_else(|error| {
                    panic!(
                        "failed to expand workspace member pattern `{pattern}` at `{}`: {error}",
                        candidate.display()
                    )
                });
                next.extend(
                    entries
                        .map(|entry| {
                            entry.unwrap_or_else(|error| {
                                panic!(
                                    "failed to inspect workspace member pattern `{pattern}` at `{}`: {error}",
                                    candidate.display()
                                )
                            })
                        })
                        .map(|entry| entry.path())
                        .filter(|path| path.is_dir()),
                );
            } else {
                next.push(candidate.join(component));
            }
        }
        candidates = next;
    }
    candidates
}

/// Every package outside the workspace that the named crate compiles against,
/// sorted and rendered as `name version source checksum`.
///
/// Version alone would miss a git dependency re-pointed at a new revision
/// under an unchanged version, and source alone would miss a registry bump, so
/// all four fields are rendered. A vendored path package has no source or
/// checksum and contributes its name and version.
///
/// # Panics
///
/// Panics if the crate has no entry in the lock file it was resolved by. A
/// build script that cannot find its own resolution must not guess: emitting a
/// fingerprint that omits the dependency surface would leave exactly the
/// silent staleness this module exists to close.
pub fn resolved_closure(
    lock: &Value,
    root_name: &str,
    root_version: &str,
    excluded_roots: &BTreeSet<String>,
    workspace_members: &BTreeSet<String>,
) -> Vec<String> {
    let packages = lock
        .get("package")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let mut by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, package) in packages.iter().enumerate() {
        if let Some(name) = package.get("name").and_then(Value::as_str) {
            by_name.entry(name).or_default().push(index);
        }
    }

    let mut roots = resolve_root(packages, &by_name, root_name, root_version);
    let root = match roots.len() {
        1 => roots.pop().expect("one root package was resolved"),
        count => panic!(
            "`{root_name} {root_version}` resolved to {count} lock packages; \
             a dependency fingerprint cannot choose its root unambiguously"
        ),
    };

    let mut queue: VecDeque<usize> = VecDeque::new();
    for reference in dependency_references(&packages[root]) {
        let reference = Reference::parse(reference);
        if excluded_roots.contains(reference.name) {
            continue;
        }
        queue.extend(resolve(packages, &by_name, &reference));
    }

    let mut visited: BTreeSet<usize> = BTreeSet::new();
    let mut entries: BTreeSet<String> = BTreeSet::new();
    while let Some(index) = queue.pop_front() {
        if !visited.insert(index) {
            continue;
        }
        let package = &packages[index];
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("Cargo.lock package at index {index} has no name"));
        if workspace_members.contains(name) {
            continue;
        }

        let version = package
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("Cargo.lock package `{name}` has no version"));
        entries.insert(format!(
            "{name} {version} {} {}",
            package
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            package
                .get("checksum")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ));

        for reference in dependency_references(package) {
            let reference = Reference::parse(reference);
            queue.extend(resolve(packages, &by_name, &reference));
        }
    }

    entries.into_iter().collect()
}

fn dependency_references(package: &Value) -> impl Iterator<Item = &str> {
    let name = package
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("<unnamed>");
    let dependencies = package.get("dependencies").map_or(&[][..], |value| {
        value.as_array().unwrap_or_else(|| {
            panic!("Cargo.lock package `{name}` has a non-array dependencies field")
        })
    });
    dependencies.iter().map(move |reference| {
        reference.as_str().unwrap_or_else(|| {
            panic!("Cargo.lock package `{name}` has a non-string dependency reference")
        })
    })
}

/// A dependency reference as Cargo writes it in `Cargo.lock`.
///
/// Cargo writes `"name"` when a name is unambiguous, `"name version"` when
/// several versions are resolved, and `"name version (source)"` when the same
/// name and version come from more than one source. The source qualifier for a
/// git package omits its trailing commit hash, while the package's `source`
/// field includes it.
#[derive(Debug, PartialEq, Eq)]
struct Reference<'a> {
    name: &'a str,
    version: Option<&'a str>,
    source: Option<&'a str>,
}

impl<'a> Reference<'a> {
    fn parse(reference: &'a str) -> Self {
        let (base, source) = reference
            .rsplit_once(" (")
            .and_then(|(base, source)| source.strip_suffix(')').map(|source| (base, source)))
            .map_or((reference, None), |(base, source)| (base, Some(source)));
        let mut fields = base.splitn(3, ' ');
        let name = fields.next().unwrap_or_default();
        let version = fields.next();
        assert!(
            !name.is_empty() && fields.next().is_none(),
            "malformed Cargo.lock dependency reference `{reference}`"
        );
        Self {
            name,
            version,
            source,
        }
    }
}

/// Resolve a lock dependency reference, returning every matching candidate.
///
/// An unversioned reference should match exactly one package. If it ever
/// matches more, every match is walked. That may reprocess an extra rule, but
/// choosing one arbitrarily could drop the package that actually changed. An
/// unresolved reference is a lockfile inconsistency and fails closed rather
/// than silently omitting a dependency from the fingerprint.
fn resolve(
    packages: &[Value],
    by_name: &BTreeMap<&str, Vec<usize>>,
    reference: &Reference<'_>,
) -> Vec<usize> {
    let Some(candidates) = by_name.get(reference.name) else {
        panic!(
            "Cargo.lock dependency reference `{}` names no package",
            render_reference(reference)
        )
    };
    let matches = candidates
        .iter()
        .copied()
        .filter(|index| {
            let package = &packages[*index];
            let version_matches = reference.version.is_none_or(|version| {
                package.get("version").and_then(Value::as_str) == Some(version)
            });
            let source_matches = reference
                .source
                .is_none_or(|source| package_source_matches(package, source));
            version_matches && source_matches
        })
        .collect::<Vec<_>>();
    assert!(
        !matches.is_empty(),
        "Cargo.lock dependency reference `{}` has no matching package",
        render_reference(reference)
    );
    matches
}

fn resolve_root(
    packages: &[Value],
    by_name: &BTreeMap<&str, Vec<usize>>,
    name: &str,
    version: &str,
) -> Vec<usize> {
    let Some(candidates) = by_name.get(name) else {
        panic!(
            "`{name} {version}` has no entry in the lock file that resolved it; \
             a dependency fingerprint cannot be derived without one"
        )
    };
    let matches = candidates
        .iter()
        .copied()
        .filter(|index| {
            let package = &packages[*index];
            package.get("version").and_then(Value::as_str) == Some(version)
                && package.get("source").is_none()
        })
        .collect::<Vec<_>>();
    assert!(
        !matches.is_empty(),
        "`{name} {version}` has no source-less workspace entry in Cargo.lock; \
         a dependency fingerprint cannot choose its root"
    );
    matches
}

fn render_reference(reference: &Reference<'_>) -> String {
    let mut rendered = reference.name.to_owned();
    if let Some(version) = reference.version {
        rendered.push(' ');
        rendered.push_str(version);
    }
    if let Some(source) = reference.source {
        rendered.push_str(" (");
        rendered.push_str(source);
        rendered.push(')');
    }
    rendered
}

fn package_source_matches(package: &Value, expected: &str) -> bool {
    let Some(actual) = package.get("source").and_then(Value::as_str) else {
        return false;
    };
    actual == expected
        || actual
            .split_once('#')
            .is_some_and(|(without_revision, _)| without_revision == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Value {
        toml::from_str(text).expect("test fixture parses")
    }

    fn names(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    const WORKSPACE: &str = r#"
[[package]]
name = "root"
version = "0.1.0"
dependencies = ["codec", "sibling", "vendored", "harness", "engine"]

[[package]]
name = "codec"
version = "1.4.1"
source = "registry+https://example.invalid"
checksum = "aaaa"
dependencies = ["inner"]

[[package]]
name = "inner"
version = "0.3.0"
source = "registry+https://example.invalid"
checksum = "bbbb"

[[package]]
name = "sibling"
version = "0.1.0"
dependencies = ["unreachable"]

[[package]]
name = "unreachable"
version = "9.9.9"
source = "registry+https://example.invalid"
checksum = "cccc"

[[package]]
name = "vendored"
version = "0.7.0"
dependencies = ["behind-the-patch"]

[[package]]
name = "behind-the-patch"
version = "2.1.0"
source = "registry+https://example.invalid"
checksum = "eeee"

[[package]]
name = "harness"
version = "3.0.0"
source = "registry+https://example.invalid"
checksum = "dddd"

[[package]]
name = "engine"
version = "0.19.0"
source = "registry+https://example.invalid"
checksum = "ffff"
"#;

    fn closure(excluded: &[&str]) -> Vec<String> {
        resolved_closure(
            &parse(WORKSPACE),
            "root",
            "0.1.0",
            &names(excluded),
            &names(&["root", "sibling"]),
        )
    }

    fn starts_with(closure: &[String], prefix: &str) -> bool {
        closure.iter().any(|entry| entry.starts_with(prefix))
    }

    #[test]
    fn a_direct_third_party_dependency_enters_the_closure() {
        assert!(starts_with(&closure(&[]), "codec 1.4.1 "));
    }

    #[test]
    fn the_root_reference_selects_the_source_less_workspace_package() {
        let lock = format!(
            "{WORKSPACE}\n[[package]]\nname = \"root\"\nversion = \"0.1.0\"\n\
             source = \"registry+https://example.invalid\"\ndependencies = [\"wrong\"]\n\n\
             [[package]]\nname = \"wrong\"\nversion = \"1.0.0\"\n\
             source = \"registry+https://example.invalid\"\nchecksum = \"wrong\"\n"
        );
        let closure = resolved_closure(
            &parse(&lock),
            "root",
            "0.1.0",
            &names(&[]),
            &names(&["root"]),
        );

        assert!(starts_with(&closure, "codec 1.4.1 "));
        assert!(!starts_with(&closure, "wrong 1.0.0 "));
    }

    #[test]
    fn a_transitive_third_party_dependency_enters_the_closure() {
        // The whole reason direct-only was rejected: a codec's own parser can
        // move while the codec's version does not.
        assert!(starts_with(&closure(&[]), "inner 0.3.0 "));
    }

    #[test]
    fn traversal_stops_at_workspace_members() {
        let closure = closure(&[]);
        assert!(!starts_with(&closure, "sibling "));
        assert!(!starts_with(&closure, "unreachable "));
    }

    #[test]
    fn a_vendored_path_package_is_walked_rather_than_dropped() {
        // A `[patch]` redirect to a path looks exactly like a workspace crate
        // in the lock file. Membership decides, not the missing source key.
        let closure = closure(&[]);
        assert!(starts_with(&closure, "vendored 0.7.0 "));
        assert!(starts_with(&closure, "behind-the-patch 2.1.0 "));
    }

    #[test]
    fn an_excluded_root_dependency_is_subtracted() {
        assert!(!starts_with(&closure(&["harness"]), "harness "));
    }

    #[test]
    fn the_closure_is_sorted_and_free_of_duplicates() {
        let closure = closure(&[]);
        let mut sorted = closure.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(closure, sorted);
    }

    #[test]
    fn a_moved_version_changes_the_closure() {
        let bumped = WORKSPACE.replace("version = \"1.4.1\"", "version = \"1.5.0\"");
        assert_ne!(
            resolved_closure(
                &parse(WORKSPACE),
                "root",
                "0.1.0",
                &names(&[]),
                &names(&["sibling"])
            ),
            resolved_closure(
                &parse(&bumped),
                "root",
                "0.1.0",
                &names(&[]),
                &names(&["sibling"])
            ),
        );
    }

    #[test]
    fn a_repointed_git_revision_changes_the_closure_under_an_unchanged_version() {
        let repointed = WORKSPACE.replacen(
            "example.invalid\"\nchecksum = \"aaaa",
            "example.invalid?rev=2\"\nchecksum = \"aaaa",
            1,
        );
        assert_ne!(
            resolved_closure(
                &parse(WORKSPACE),
                "root",
                "0.1.0",
                &names(&[]),
                &names(&["sibling"])
            ),
            resolved_closure(
                &parse(&repointed),
                "root",
                "0.1.0",
                &names(&[]),
                &names(&["sibling"])
            ),
        );
    }

    #[test]
    fn a_source_qualified_reference_selects_the_matching_package() {
        let lock = r#"
[[package]]
name = "root"
version = "0.1.0"
dependencies = ["widget 1.0.0 (registry+https://example.invalid)"]

[[package]]
name = "widget"
version = "1.0.0"
source = "registry+https://example.invalid"
checksum = "registry"
dependencies = ["registry-only"]

[[package]]
name = "widget"
version = "1.0.0"
source = "git+https://example.invalid/widget?rev=git#abcdef"
dependencies = ["git-only"]

[[package]]
name = "registry-only"
version = "1.0.0"
source = "registry+https://example.invalid"
checksum = "registry-only"

[[package]]
name = "git-only"
version = "1.0.0"
source = "registry+https://example.invalid"
checksum = "git-only"
"#;
        let closure = resolved_closure(&parse(lock), "root", "0.1.0", &names(&[]), &names(&[]));

        assert!(starts_with(
            &closure,
            "widget 1.0.0 registry+https://example.invalid registry"
        ));
        assert!(starts_with(&closure, "registry-only 1.0.0 "));
        assert!(!starts_with(&closure, "git-only 1.0.0 "));
    }

    #[test]
    fn a_git_source_qualifier_matches_the_package_source_without_its_revision() {
        let lock = r#"
[[package]]
name = "root"
version = "0.1.0"
dependencies = ["widget 1.0.0 (git+https://example.invalid/widget?rev=git)"]

[[package]]
name = "widget"
version = "1.0.0"
source = "git+https://example.invalid/widget?rev=git#abcdef"
"#;
        let closure = resolved_closure(&parse(lock), "root", "0.1.0", &names(&[]), &names(&[]));

        assert_eq!(closure.len(), 1);
        assert!(starts_with(
            &closure,
            "widget 1.0.0 git+https://example.invalid/widget?rev=git"
        ));
    }

    #[test]
    fn an_unrelated_package_does_not_change_the_closure() {
        let extended = format!(
            "{WORKSPACE}\n[[package]]\nname = \"elsewhere\"\nversion = \"2.0.0\"\n\
             source = \"registry+https://example.invalid\"\nchecksum = \"0000\"\n"
        );
        assert_eq!(
            resolved_closure(
                &parse(WORKSPACE),
                "root",
                "0.1.0",
                &names(&[]),
                &names(&["sibling"])
            ),
            resolved_closure(
                &parse(&extended),
                "root",
                "0.1.0",
                &names(&[]),
                &names(&["sibling"])
            ),
        );
    }

    #[test]
    fn an_ambiguous_reference_walks_every_candidate() {
        // Over-invalidating on ambiguity is safe; guessing is not.
        let ambiguous = r#"
[[package]]
name = "root"
version = "0.1.0"
dependencies = ["twin"]

[[package]]
name = "twin"
version = "1.0.0"
source = "registry+https://example.invalid"
checksum = "aaaa"

[[package]]
name = "twin"
version = "2.0.0"
source = "registry+https://example.invalid"
checksum = "bbbb"
"#;
        let closure =
            resolved_closure(&parse(ambiguous), "root", "0.1.0", &names(&[]), &names(&[]));
        assert_eq!(closure.len(), 2);
    }

    #[test]
    fn a_cycle_terminates() {
        let cyclic = r#"
[[package]]
name = "root"
version = "0.1.0"
dependencies = ["left"]

[[package]]
name = "left"
version = "1.0.0"
source = "registry+https://example.invalid"
checksum = "aaaa"
dependencies = ["right"]

[[package]]
name = "right"
version = "1.0.0"
source = "registry+https://example.invalid"
checksum = "bbbb"
dependencies = ["left"]
"#;
        let closure = resolved_closure(&parse(cyclic), "root", "0.1.0", &names(&[]), &names(&[]));
        assert_eq!(closure.len(), 2);
    }

    const MANIFEST: &str = r#"
[features]
default = []
engine = ["dep:bevy"]
telemetry = ["tracing/log"]
weak = ["optional-extra?/std"]

[dependencies]
bevy = { version = "0.19", optional = true }
tracing = { version = "0.1", optional = true }
optional-extra = { version = "1", optional = true }
implicit = { version = "1", optional = true }
codec = "1"

[dev-dependencies]
harness = "3"
"#;

    #[test]
    fn an_unactivated_optional_dependency_is_excluded() {
        // The measured motive: leaving `bevy` in makes a mesh builder's
        // fingerprint a fingerprint of an engine it is not linking.
        let excluded = excluded_root_dependencies(&parse(MANIFEST), &names(&["default"]));
        assert!(excluded.contains("bevy"));
        assert!(excluded.contains("tracing"));
        assert!(excluded.contains("implicit"));
    }

    #[test]
    fn a_dep_reference_from_an_activated_feature_keeps_the_dependency() {
        let excluded = excluded_root_dependencies(&parse(MANIFEST), &names(&["default", "engine"]));
        assert!(!excluded.contains("bevy"));
    }

    #[test]
    fn a_slash_reference_from_an_activated_feature_keeps_the_dependency() {
        let excluded = excluded_root_dependencies(&parse(MANIFEST), &names(&["telemetry"]));
        assert!(!excluded.contains("tracing"));
    }

    #[test]
    fn a_weak_reference_does_not_keep_the_dependency() {
        // `name?/feature` is defined not to activate the dependency.
        let excluded = excluded_root_dependencies(&parse(MANIFEST), &names(&["weak"]));
        assert!(excluded.contains("optional-extra"));
    }

    #[test]
    fn an_implicit_feature_named_after_the_dependency_keeps_it() {
        let excluded = excluded_root_dependencies(&parse(MANIFEST), &names(&["implicit"]));
        assert!(!excluded.contains("implicit"));
    }

    #[test]
    fn a_required_dependency_is_never_excluded() {
        let excluded = excluded_root_dependencies(&parse(MANIFEST), &names(&[]));
        assert!(!excluded.contains("codec"));
    }

    #[test]
    fn a_development_only_dependency_is_excluded() {
        let excluded = excluded_root_dependencies(&parse(MANIFEST), &names(&["default"]));
        assert!(excluded.contains("harness"));
    }

    #[test]
    fn a_name_used_by_both_sections_is_not_treated_as_development_only() {
        let excluded = excluded_root_dependencies(
            &parse(
                "[dependencies]\nshared = \"1\"\n\n[dev-dependencies]\nshared = \"1\"\nharness = \"2\"\n",
            ),
            &names(&[]),
        );
        assert!(excluded.contains("harness"));
        assert!(!excluded.contains("shared"));
    }

    #[test]
    fn exclusions_follow_renames_to_their_lock_name() {
        let excluded = excluded_root_dependencies(
            &parse(
                "[dependencies]\nalias = { package = \"real-name\", version = \"1\", optional = true }\n",
            ),
            &names(&[]),
        );
        assert!(excluded.contains("real-name"));
        assert!(!excluded.contains("alias"));
    }

    #[test]
    fn a_build_dependency_is_never_development_only() {
        let excluded = excluded_root_dependencies(
            &parse(
                "[build-dependencies]\ngenerator = \"1\"\n\n[dev-dependencies]\ngenerator = \"1\"\n",
            ),
            &names(&[]),
        );
        assert!(excluded.is_empty());
    }

    #[test]
    fn platform_specific_sections_are_read() {
        let excluded = excluded_root_dependencies(
            &parse(
                "[target.'cfg(windows)'.dev-dependencies]\nwindows-harness = \"1\"\n\n\
                 [target.'cfg(unix)'.dependencies]\nnix = { version = \"1\", optional = true }\n",
            ),
            &names(&[]),
        );
        assert_eq!(
            excluded.iter().map(String::as_str).collect::<Vec<_>>(),
            ["nix", "windows-harness"],
        );
    }
}
