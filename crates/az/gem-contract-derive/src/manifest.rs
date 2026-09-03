//! The narrow manifest reader behind the manifest forms of `#[contribution]`.
//!
//! `az-project` owns the manifest schema and is the crate that validates it;
//! this reader is deliberately not a second copy of it. It reads five facts —
//! the declaring unit's id, and the id, roles, capability floor, and generated
//! products of one `[[contributions]]` entry — and every other key stays
//! invisible, so growing the schema cannot break expansion. Depending on
//! `az-project` here is not an option in either direction: it depends on
//! `az-gem-contract`, which depends on this crate.
//!
//! Two manifests declare contributions and the grammar is the same in both: a
//! gem's `gem.toml` and the engine's own `engine.toml`. The engine is a
//! manifested composition root rather than a gem, so its crates take identity
//! from `[engine] id` through this same reader, with the same diagnostics
//! (asset-contract ticket 014, D1/D2).
//!
//! Which entry is *this* block's is answered by the crate being compiled: the
//! package selects it, and a bare token disambiguates when one package hosts
//! several. Both are validated against the same declarations, so a token that
//! names nothing, or names another package's contribution, is refused here
//! rather than silently expanding into the wrong identity.

use std::path::{Path, PathBuf};

use proc_macro2::Span;
use syn::Ident;

/// The file a gem is declared in, spelled where a proc macro can reach it.
/// `az_project::GEM_MANIFEST_FILE` is the same name in the crate that owns the
/// schema.
const GEM: &str = "gem.toml";

/// The file the engine is declared in. `az_project::ENGINE_MANIFEST_FILE` is
/// the same name in the crate that owns the schema.
const ENGINE: &str = "engine.toml";

/// The terminal manifests, nearest first within one directory.
///
/// A gem root never also carries an `engine.toml`, so the order only decides a
/// case that does not arise; what it does guarantee is that the walk stops at
/// the *first* declaring unit above the crate, never continuing past a gem to
/// the engine that hosts it.
const FILES: [&str; 2] = [GEM, ENGINE];

/// Target roles in `GemTargetRole`'s own kebab-case serde spelling.
///
/// Correctness never rests on this list: every role it accepts is emitted as a
/// `GemTargetRole` variant, so a name this table admits and the enum does not
/// still fails to compile. The table exists to say *which* names are legal at
/// the manifest, before rustc reduces the question to one missing variant.
const ROLES: [&str; 12] = [
    "game",
    "p2p",
    "client",
    "server",
    "unified",
    "headless-server",
    "project-host",
    "asset-processor",
    "asset-worker",
    "runtime-host",
    "tool",
    "named-service",
];

/// Host capabilities, spelled as the contract's type tokens — the one spelling
/// `gem.toml`, `caps = [...]`, and a generated load manifest share.
const CAPS: [&str; 4] = ["HostsWorld", "HasClient", "HasAuthority", "OwnsFixedClock"];

/// Generated products, in `GemProduct`'s own kebab-case serde spelling.
///
/// Sealed like the two sets above, and for the same reason: each name is one
/// emitter writing one projection into `OUT_DIR`, and the expansion knows the
/// module each writes. A crate declares what its build script compiled in, and
/// the expansion — not the author — writes the call that registers it.
pub const PRODUCTS: [&str; 1] = ["graphs"];

/// A stanza with no `id`, kept in the reading rather than dropped: a
/// contribution that forgot its id still has to be named in a diagnostic.
const NO_ID: &str = "<no id>";

/// One manifest-resolved contribution: identity plus the three sealed sets.
pub struct Entry {
    /// The declaring unit's id — `[gem] id`, or `[engine] id` for the engine.
    pub gem: String,
    pub id: String,
    pub roles: Vec<Ident>,
    pub caps: Vec<Ident>,
    /// Declared generated products, in the sealed set's own spelling: the
    /// values are borrowed from [`PRODUCTS`], so a resolved entry cannot carry
    /// a name the expansion has no projection for.
    pub products: Vec<&'static str>,
    /// The convention entry item's name, folded from `id`.
    pub call: Ident,
    /// Absolute path of the manifest, forward-slashed for `include_bytes!`.
    pub path: String,
}

/// Locate, parse, and select this block's contribution.
///
/// `root` is the crate's `CARGO_MANIFEST_DIR`; `package` its `CARGO_PKG_NAME`;
/// `token` the bare ident the attribute was given, when it was given one.
pub fn read(package: &str, root: &Path, token: Option<&str>) -> Result<Entry, String> {
    let path = locate(package, root)?;
    let shown = relative(&path, root);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("`{shown}` could not be read: {error}"))?;
    let manifest = text
        .parse::<toml::Table>()
        .map_err(|error| format!("`{shown}` is not valid TOML: {error}"))?;
    let mut entry = select(&manifest, package, &shown, token)?;
    entry.path = path.to_string_lossy().replace('\\', "/");
    Ok(entry)
}

/// The nearest enclosing manifested unit: the first `gem.toml` or
/// `engine.toml` at or above the crate.
///
/// Nearest wins by construction — a contribution crate sits inside exactly one
/// declaring unit, and continuing past it would bind a crate to a gem it does
/// not live in, or hand a gem's crate to the engine that hosts the gem.
fn locate(package: &str, root: &Path) -> Result<PathBuf, String> {
    root.ancestors()
        .flat_map(|directory| FILES.map(|file| directory.join(file)))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            format!(
                "`#[contribution]` takes its identity from the manifest that declares it, and \
                 package `{package}` has none in reach: no `{GEM}` or `{ENGINE}` in this crate's \
                 own directory, nor in any ancestor of it up to the filesystem root. A \
                 contribution crate lives inside its gem, at or below the `{GEM}` that declares \
                 it; an engine crate lives below the engine's `{ENGINE}`."
            )
        })
}

/// The manifest's path as an author would type it from this crate.
///
/// Relative, because the crate the diagnostic points at is the frame the reader
/// already has, and an absolute path would move with the checkout.
fn relative(path: &Path, root: &Path) -> String {
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(GEM);
    let Some(directory) = path.parent() else {
        return file.to_string();
    };
    let steps = root
        .ancestors()
        .take_while(|ancestor| *ancestor != directory)
        .count();
    let mut shown = "../".repeat(steps);
    shown.push_str(file);
    shown
}

/// One `[[contributions]]` stanza, read only as far as selection needs it.
struct Declared<'a> {
    id: &'a str,
    package: Option<&'a str>,
    stanza: &'a toml::Table,
}

fn select(
    manifest: &toml::Table,
    package: &str,
    shown: &str,
    token: Option<&str>,
) -> Result<Entry, String> {
    // One reader, two manifests: a gem names itself under `[gem]`, the engine
    // under `[engine]`, and everything downstream of this line is identical.
    let gem = manifest
        .get("gem")
        .or_else(|| manifest.get("engine"))
        .and_then(toml::Value::as_table)
        .and_then(|table| table.get("id"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("`{shown}` declares no `[gem] id` or `[engine] id`"))?;

    let declared: Vec<Declared<'_>> = manifest
        .get("contributions")
        .and_then(toml::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(toml::Value::as_table)
        .map(|stanza| Declared {
            id: stanza
                .get("id")
                .and_then(toml::Value::as_str)
                .filter(|id| !id.is_empty())
                .unwrap_or(NO_ID),
            package: stanza.get("package").and_then(toml::Value::as_str),
            stanza,
        })
        .collect();
    let mine: Vec<&Declared<'_>> = declared
        .iter()
        .filter(|entry| entry.package == Some(package))
        .collect();

    let entry = match token {
        Some(token) => named(&declared, &mine, token, package, shown)?,
        None => match mine.as_slice() {
            [only] => *only,
            ambiguous => return Err(ambiguity(ambiguous, package, shown)),
        },
    };

    if entry.id == NO_ID {
        return Err(format!(
            "`{shown}` declares a contribution for package `{package}` with no `id`"
        ));
    }

    Ok(Entry {
        gem: gem.to_string(),
        id: entry.id.to_string(),
        roles: roles(entry.stanza, entry.id, shown)?,
        caps: caps(entry.stanza, entry.id, shown)?,
        products: products(entry.stanza, entry.id, shown)?,
        call: call(entry.id, shown)?,
        path: String::new(),
    })
}

/// The bare-token form: one ident, matched against the declared ids.
///
/// Matching folds both sides, because the token is a Rust ident and the id it
/// names may be hyphenated or dotted. The two miss paths are the two mistakes
/// worth telling apart: a token this package has no contribution for, and a
/// token that names a contribution some *other* package implements.
fn named<'a>(
    declared: &'a [Declared<'a>],
    mine: &[&'a Declared<'a>],
    token: &str,
    package: &str,
    shown: &str,
) -> Result<&'a Declared<'a>, String> {
    if let Some(entry) = mine.iter().copied().find(|entry| folds_to(entry.id, token)) {
        return Ok(entry);
    }
    if let Some(elsewhere) = declared.iter().find(|entry| folds_to(entry.id, token)) {
        let id = elsewhere.id;
        let Some(owner) = elsewhere.package else {
            return Err(format!(
                "`{shown}` declares contribution `{id}` with no `package`, so no crate implements \
                 it and no block can be it — a contribution without a package is content the gem \
                 mounts, not code a host links."
            ));
        };
        return Err(format!(
            "`{shown}` declares contribution `{id}` for package `{owner}`, and this crate is \
             package `{package}`{}. A contribution is implemented by the crate its stanza names, \
             so either this block belongs in `{owner}`, or the stanza's `package` is wrong.{}",
            if mine.is_empty() {
                ", which it declares no contribution for at all"
            } else {
                ""
            },
            if mine.is_empty() {
                format!("\n\nThe stanza that would name it:\n\n{}", stanza(package))
            } else {
                String::new()
            },
        ));
    }
    if mine.is_empty() {
        return Err(undeclared(package, shown));
    }
    let ids: Vec<&str> = mine.iter().map(|entry| entry.id).collect();
    Err(format!(
        "`{shown}` declares no contribution `{token}` for package `{package}`, which declares {}. \
         The token names one of those, with `-` and `.` written as `_`.",
        list(&ids),
    ))
}

/// Whether a manifest id and a bare token are the same name.
///
/// Folded on both sides: `-` and `.` are legal in an id and unspellable in an
/// ident, and the fold that renders the entry item's name is the same one.
fn folds_to(id: &str, token: &str) -> bool {
    fold(id) == fold(token)
}

fn fold(name: &str) -> String {
    name.chars()
        .map(|character| match character {
            '-' | '.' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

/// The package this crate is has no contribution at all: name the stanza to add.
fn undeclared(package: &str, shown: &str) -> String {
    format!(
        "`{shown}` declares no contribution for package `{package}`. Add the stanza that names \
         it:\n\n{}",
        stanza(package),
    )
}

/// The `[[contributions]]` stanza a crate needs, ready to paste.
fn stanza(package: &str) -> String {
    format!(
        "[[contributions]]\nid = \"<contribution-id>\"\nkind = \"code\"\npackage = \
         \"{package}\"\nroles = [\"server\", \"unified\"]\ncaps = [\"HostsWorld\"]"
    )
}

/// No token, and the package alone does not select one: name the stanza to add,
/// or the token that would.
fn ambiguity(matched: &[&Declared<'_>], package: &str, shown: &str) -> String {
    let [first, ..] = matched else {
        return undeclared(package, shown);
    };
    let ids: Vec<&str> = matched.iter().map(|entry| entry.id).collect();
    format!(
        "`{shown}` declares {} contributions for package `{package}` ({}), so identity does not \
         follow from the package alone. Name the one this block is: `#[contribution({})]`.",
        matched.len(),
        list(&ids),
        fold(first.id),
    )
}

fn roles(entry: &toml::Table, id: &str, shown: &str) -> Result<Vec<Ident>, String> {
    let declared = entry
        .get("roles")
        .and_then(toml::Value::as_array)
        .filter(|roles| !roles.is_empty())
        .ok_or_else(|| {
            format!(
                "`{shown}` gives contribution `{id}` no `roles`; a contribution names the target \
                 roles it composes into, and removing it is how it becomes inert"
            )
        })?;

    declared
        .iter()
        .map(|role| {
            let name = role.as_str().filter(|name| ROLES.contains(name)).ok_or_else(|| {
                format!(
                    "`{shown}` gives contribution `{id}` the target role {role}, which is not one \
                     of the sealed roles: {}",
                    list(&ROLES),
                )
            })?;
            Ok(ident(&pascal(name)))
        })
        .collect()
}

fn caps(entry: &toml::Table, id: &str, shown: &str) -> Result<Vec<Ident>, String> {
    let Some(declared) = entry.get("caps") else {
        return Ok(Vec::new());
    };
    let declared = declared.as_array().ok_or_else(|| {
        format!(
            "`{shown}` gives contribution `{id}` a `caps` that is not a list; the floor is a list \
             of the sealed four: {}",
            list(&CAPS),
        )
    })?;

    declared
        .iter()
        .map(|capability| {
            let name = capability
                .as_str()
                .filter(|name| CAPS.contains(name))
                .ok_or_else(|| {
                    format!(
                        "`{shown}` gives contribution `{id}` the host capability {capability}, \
                         which is not one of the sealed four: {}",
                        list(&CAPS),
                    )
                })?;
            Ok(ident(name))
        })
        .collect()
}

/// The generated products this contribution's crate compiles in.
///
/// The accepted name is returned as the sealed set's own `&'static str`, not
/// the manifest's copy of it, so an entry that resolves carries a product the
/// expansion is known to have a projection for.
fn products(entry: &toml::Table, id: &str, shown: &str) -> Result<Vec<&'static str>, String> {
    let Some(declared) = entry.get("products") else {
        return Ok(Vec::new());
    };
    let declared = declared.as_array().ok_or_else(|| {
        format!(
            "`{shown}` gives contribution `{id}` a `products` that is not a list; a crate declares \
             the generated products it compiles in as a list of the sealed names: {}",
            list(&PRODUCTS),
        )
    })?;

    declared
        .iter()
        .map(|product| {
            product
                .as_str()
                .and_then(|name| PRODUCTS.iter().copied().find(|sealed| *sealed == name))
                .ok_or_else(|| {
                    format!(
                        "`{shown}` gives contribution `{id}` the generated product {product}, \
                         which is not one of the sealed products: {}",
                        list(&PRODUCTS),
                    )
                })
        })
        .collect()
}

/// The entry item's name: the id with `-` and `.` folded to `_`.
///
/// The same fold `az_project::entry_ident` applies when it renders the call in
/// generated target glue, which is why no human writes either side of it.
fn call(id: &str, shown: &str) -> Result<Ident, String> {
    let folded = fold(id);
    let spellable = folded
        .chars()
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && folded
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric());
    if !spellable {
        return Err(format!(
            "`{shown}` gives a contribution the id `{id}`, which folds to `{folded}` and cannot be \
             spelled as the entry item's name; contribution ids start with a letter"
        ));
    }
    Ok(ident(&format!("{folded}_contribution")))
}

fn pascal(kebab: &str) -> String {
    kebab
        .split('-')
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_ascii_uppercase().to_string() + characters.as_str()
            })
        })
        .collect()
}

fn ident(name: &str) -> Ident {
    Ident::new(name, Span::call_site())
}

/// A sealed set or a colliding id list, quoted the way prose reads it.
pub fn list(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{ENGINE, Entry, GEM, read, relative, select};

    const MANIFEST: &str = r#"
[manifest]
kind = "gem"
schema = "azoth.gem/v3"

[gem]
id = "azoth.mount"
name = "Mount"
version = "0.1.0"

[[contributions]]
id = "runtime"
kind = "code"
package = "az-gem-mount"
roles = ["server", "unified"]
caps = ["HasAuthority", "HostsWorld"]

[[contributions]]
id = "runtime-host"
kind = "code"
package = "az-gem-mount"
roles = ["runtime-host"]

[[contributions]]
id = "prefab-types"
kind = "builder"
package = "az-gem-mount-prefab"
roles = ["asset-worker"]

[[contributions]]
id = "assets"
kind = "assets"
root = "assets"
roles = ["client"]
"#;

    /// The engine's own manifest, shaped the way `az-rs/engine.toml` is: the
    /// stanza grammar is the gem's verbatim, and the only difference is the
    /// section the id lives in.
    const ENGINE_MANIFEST: &str = r#"
[manifest]
kind = "engine"
schema = "azoth.engine/v1"

[engine]
id = "azoth"
name = "Azoth"
version = "0.1.0"

[[contributions]]
id = "builders"
kind = "builder"
package = "az-engine-builders"
roles = ["asset-processor", "asset-worker", "project-host"]

[[contributions]]
id = "graphs"
kind = "code"
package = "az-engine-graphs"
roles = ["server"]
products = ["graphs"]
"#;

    fn resolve(package: &str) -> Result<Entry, String> {
        resolve_in(MANIFEST, package, None)
    }

    fn named(package: &str, token: &str) -> Result<Entry, String> {
        resolve_in(MANIFEST, package, Some(token))
    }

    fn resolve_in(manifest: &str, package: &str, token: Option<&str>) -> Result<Entry, String> {
        let manifest = manifest.parse::<toml::Table>().expect("fixture parses");
        select(&manifest, package, GEM, token)
    }

    fn refuse(package: &str) -> String {
        resolve(package).err().expect("resolution fails")
    }

    fn refuse_named(package: &str, token: &str) -> String {
        named(package, token).err().expect("resolution fails")
    }

    fn refuse_in(manifest: &str, package: &str) -> String {
        resolve_in(manifest, package, None)
            .err()
            .expect("resolution fails")
    }

    /// Both sealed sets, spelled end to end.
    ///
    /// A name the enum grew and this crate has not fails closed — the manifest
    /// refuses it here rather than mis-spelling a variant — and a name this
    /// crate kept and the enum dropped fails at rustc, on the token the
    /// expansion emits. This pins the crossing itself: every accepted kebab
    /// name lands on the variant that spells it.
    #[test]
    fn every_sealed_role_and_capability_crosses_the_manifest() {
        let manifest = format!(
            "[gem]\nid = \"azoth.mount\"\n\n[[contributions]]\nid = \"runtime\"\npackage = \
             \"az-gem-mount\"\nroles = {:?}\ncaps = {:?}\n",
            super::ROLES,
            super::CAPS,
        )
        .parse::<toml::Table>()
        .expect("fixture parses");
        let entry = select(&manifest, "az-gem-mount", GEM, None).expect("both sealed sets resolve");

        assert_eq!(
            entry
                .roles
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            [
                "Game",
                "P2p",
                "Client",
                "Server",
                "Unified",
                "HeadlessServer",
                "ProjectHost",
                "AssetProcessor",
                "AssetWorker",
                "RuntimeHost",
                "Tool",
                "NamedService",
            ]
        );
        assert_eq!(
            entry
                .caps
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            super::CAPS
        );
    }

    /// The package selects on its own when it hosts exactly one contribution:
    /// the crate below the gem implements only `prefab-types`.
    #[test]
    fn one_contribution_for_the_package_needs_no_token() {
        let entry = resolve("az-gem-mount-prefab").expect("the package is declared");
        assert_eq!(entry.gem, "azoth.mount");
        assert_eq!(entry.id, "prefab-types");
    }

    #[test]
    fn a_token_selects_one_of_the_packages_contributions() {
        let entry = named("az-gem-mount", "runtime").expect("the package declares it");
        assert_eq!(entry.gem, "azoth.mount");
        assert_eq!(entry.id, "runtime");
        assert_eq!(
            entry
                .roles
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["Server", "Unified"]
        );
        assert_eq!(
            entry
                .caps
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["HasAuthority", "HostsWorld"]
        );
        assert_eq!(entry.call.to_string(), "runtime_contribution");
    }

    /// A hyphenated id folds on both sides of the seam: here, and in
    /// `az_project::entry_ident` where the glue's call is rendered.
    #[test]
    fn a_hyphenated_id_folds_to_the_entry_items_name() {
        let entry = resolve("az-gem-mount-prefab").expect("the package is declared");
        assert_eq!(entry.call.to_string(), "prefab_types_contribution");
        assert_eq!(entry.id, "prefab-types");
        assert_eq!(
            entry
                .roles
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["AssetWorker"]
        );
        assert!(entry.caps.is_empty(), "an omitted floor is the empty floor");
    }

    /// The token is an ident and the id it names is not, so the fold runs on
    /// both sides — the same fold that renders the entry item's name.
    #[test]
    fn a_token_folds_onto_a_hyphenated_id() {
        let entry = named("az-gem-mount", "runtime_host").expect("the package declares it");
        assert_eq!(entry.id, "runtime-host");
        assert_eq!(entry.call.to_string(), "runtime_host_contribution");
        assert_eq!(
            entry
                .roles
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["RuntimeHost"]
        );
    }

    /// An unmatched token lists what this package does declare: the fix is one
    /// of those names, and the reader should not have to open the manifest.
    #[test]
    fn a_token_nothing_declares_lists_the_packages_own_ids() {
        let message = refuse_named("az-gem-mount", "authoring");
        assert!(
            message.contains("declares no contribution `authoring` for package `az-gem-mount`"),
            "{message}"
        );
        assert!(
            message.contains("`runtime` and `runtime-host`"),
            "{message}"
        );
        assert!(message.contains("`-` and `.` written as `_`"), "{message}");
    }

    /// A token that names a real contribution of another package is the near
    /// miss worth its own message: it says which package implements it, because
    /// the block is either in the wrong crate or the stanza names the wrong one.
    #[test]
    fn a_token_owned_by_another_package_names_that_package() {
        let message = refuse_named("az-gem-mount", "prefab_types");
        assert!(
            message
                .contains("declares contribution `prefab-types` for package `az-gem-mount-prefab`"),
            "{message}"
        );
        assert!(
            message.contains("this crate is package `az-gem-mount`"),
            "{message}"
        );
    }

    /// An assets contribution names no package, so no crate implements it and
    /// no block can be it.
    #[test]
    fn a_token_naming_a_packageless_contribution_says_so() {
        let message = refuse_named("az-gem-mount", "assets");
        assert!(
            message.contains("declares contribution `assets` with no `package`"),
            "{message}"
        );
    }

    /// A token in a package the manifest never mentions carries both facts:
    /// which package owns the name, and that this one owns nothing yet.
    #[test]
    fn a_token_in_an_undeclared_package_names_the_owner_and_the_stanza() {
        let message = refuse_named("az-gem-mount-audio", "runtime");
        assert!(
            message.contains("declares contribution `runtime` for package `az-gem-mount`"),
            "{message}"
        );
        assert!(
            message.contains("`az-gem-mount-audio`, which it declares no contribution for at all"),
            "{message}"
        );
        assert!(
            message.contains("package = \"az-gem-mount-audio\""),
            "{message}"
        );
    }

    /// A token nothing in the gem declares, in a package nothing declares
    /// either: the fix is the stanza.
    #[test]
    fn an_unknown_token_in_an_undeclared_package_names_the_stanza_to_add() {
        let message = refuse_named("az-gem-mount-audio", "mixer");
        assert!(
            message.contains("declares no contribution for package `az-gem-mount-audio`"),
            "{message}"
        );
        assert!(message.contains("[[contributions]]"), "{message}");
    }

    #[test]
    fn an_undeclared_package_names_the_stanza_to_add() {
        let message = refuse("az-gem-mount-audio");
        assert!(
            message.contains("declares no contribution for package `az-gem-mount-audio`"),
            "{message}"
        );
        assert!(message.contains("[[contributions]]"), "{message}");
        assert!(
            message.contains("package = \"az-gem-mount-audio\""),
            "{message}"
        );
        assert!(message.contains(GEM), "{message}");
    }

    #[test]
    fn two_entries_for_one_package_name_both_ids() {
        let message = refuse_in(
            r#"
[gem]
id = "azoth.mount"

[[contributions]]
id = "runtime"
package = "az-gem-mount"
roles = ["server"]

[[contributions]]
id = "tooling"
package = "az-gem-mount"
roles = ["tool"]
"#,
            "az-gem-mount",
        );
        assert!(message.contains("2 contributions"), "{message}");
        assert!(message.contains("`runtime` and `tooling`"), "{message}");
        assert!(message.contains("`#[contribution(runtime)]`"), "{message}");
    }

    #[test]
    fn a_role_outside_the_sealed_set_echoes_it() {
        let message = refuse_in(
            r#"
[gem]
id = "azoth.mount"

[[contributions]]
id = "runtime"
package = "az-gem-mount"
roles = ["srever"]
"#,
            "az-gem-mount",
        );
        assert!(message.contains("the target role \"srever\""), "{message}");
        for role in super::ROLES {
            assert!(message.contains(&format!("`{role}`")), "{message}");
        }
    }

    #[test]
    fn a_capability_outside_the_sealed_four_echoes_them() {
        let message = refuse_in(
            r#"
[gem]
id = "azoth.mount"

[[contributions]]
id = "runtime"
package = "az-gem-mount"
roles = ["server"]
caps = ["IsFast"]
"#,
            "az-gem-mount",
        );
        assert!(
            message.contains("the host capability \"IsFast\""),
            "{message}"
        );
        for capability in super::CAPS {
            assert!(message.contains(&format!("`{capability}`")), "{message}");
        }
    }

    #[test]
    fn an_empty_role_list_is_refused_rather_than_composed_nowhere() {
        let message = refuse_in(
            r#"
[gem]
id = "azoth.mount"

[[contributions]]
id = "runtime"
package = "az-gem-mount"
roles = []
"#,
            "az-gem-mount",
        );
        assert!(message.contains("no `roles`"), "{message}");
    }

    #[test]
    fn an_id_that_cannot_be_spelled_names_the_fold() {
        let message = refuse_in(
            r#"
[gem]
id = "azoth.mount"

[[contributions]]
id = "3d-preview"
package = "az-gem-mount"
roles = ["tool"]
"#,
            "az-gem-mount",
        );
        assert!(message.contains("folds to `3d_preview`"), "{message}");
    }

    /// A directory directly under the filesystem root is the honest orphan:
    /// neither manifest sits at it or above it.
    ///
    /// This crate's own directory no longer serves — it is inside the engine,
    /// and since `engine.toml` is terminal the walk now finds it, which is the
    /// whole of D2.
    #[test]
    fn a_missing_manifest_names_the_search_rule_and_the_package() {
        let orphan = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .last()
            .expect("an absolute path has a root")
            .join("az-gem-contract-derive-orphan");
        let message = read("az-gem-contract-derive", &orphan, None)
            .map(|_| ())
            .expect_err("nothing declares a package under the filesystem root");
        assert!(
            message.contains("`az-gem-contract-derive` has none in reach"),
            "{message}"
        );
        assert!(message.contains("nor in any ancestor"), "{message}");
        assert!(message.contains(GEM), "{message}");
        assert!(message.contains(ENGINE), "{message}");
    }

    /// The engine is a manifested composition root, not a gem: `engine.toml`
    /// carries the same stanza grammar and `[engine] id` answers where
    /// `[gem] id` would (D1/D2).
    #[test]
    fn the_engine_manifest_answers_where_a_gem_manifest_would() {
        let entry = resolve_in(ENGINE_MANIFEST, "az-engine-builders", None)
            .expect("the engine declares the package");
        assert_eq!(entry.gem, "azoth");
        assert_eq!(entry.id, "builders");
        assert_eq!(entry.call.to_string(), "builders_contribution");
        assert_eq!(
            entry
                .roles
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["AssetProcessor", "AssetWorker", "ProjectHost"]
        );
        assert!(entry.caps.is_empty(), "an omitted floor is the empty floor");
    }

    /// Same reader, same diagnostics: an engine stanza outside the sealed role
    /// set is refused exactly as a gem's is, and the message names the engine's
    /// own manifest.
    #[test]
    fn an_engine_stanza_is_diagnosed_like_a_gem_stanza() {
        let message = resolve_in(
            "[engine]\nid = \"azoth\"\n\n[[contributions]]\nid = \"types\"\npackage = \
             \"az-engine-types\"\nroles = [\"everywhere\"]\n",
            "az-engine-types",
            None,
        )
        .err()
        .expect("resolution fails");
        assert!(
            message.contains("the target role \"everywhere\""),
            "{message}"
        );

        let undeclared = resolve_in(ENGINE_MANIFEST, "az-engine-audio", None)
            .err()
            .expect("resolution fails");
        assert!(
            undeclared.contains("declares no contribution for package `az-engine-audio`"),
            "{undeclared}"
        );
        assert!(undeclared.contains("[[contributions]]"), "{undeclared}");
    }

    /// A manifest that names neither unit says so once, for both.
    #[test]
    fn a_manifest_without_a_gem_or_engine_id_is_refused() {
        let message = refuse_in("[manifest]\nkind = \"gem\"\n", "az-gem-mount");
        assert!(message.contains("declares no `[gem] id`"), "{message}");
        assert!(message.contains("`[engine] id`"), "{message}");
    }

    /// A declared product resolves to a member of the sealed set, borrowed
    /// from it rather than copied out of the manifest — which is what lets the
    /// expansion hold one projection per name without a second table of
    /// strings to keep in step.
    #[test]
    fn a_declared_product_resolves_to_the_sealed_name() {
        let entry = resolve_in(ENGINE_MANIFEST, "az-engine-graphs", None)
            .expect("the engine declares the package");
        assert_eq!(entry.products, ["graphs"]);
        assert!(
            entry
                .products
                .iter()
                .all(|product| super::PRODUCTS.contains(product)),
            "a resolved product is one of the sealed names"
        );
    }

    #[test]
    fn an_omitted_products_list_is_the_empty_list() {
        assert!(
            resolve("az-gem-mount-prefab")
                .expect("the package is declared")
                .products
                .is_empty()
        );
    }

    #[test]
    fn a_product_outside_the_sealed_set_echoes_it() {
        let message = refuse_in(
            r#"
[gem]
id = "azoth.mount"

[[contributions]]
id = "runtime"
package = "az-gem-mount"
roles = ["server"]
products = ["shaders"]
"#,
            "az-gem-mount",
        );
        assert!(
            message.contains("the generated product \"shaders\""),
            "{message}"
        );
        for product in super::PRODUCTS {
            assert!(message.contains(&format!("`{product}`")), "{message}");
        }
    }

    #[test]
    fn a_products_key_that_is_not_a_list_is_refused() {
        let message = refuse_in(
            r#"
[gem]
id = "azoth.mount"

[[contributions]]
id = "runtime"
package = "az-gem-mount"
roles = ["server"]
products = "graphs"
"#,
            "az-gem-mount",
        );
        assert!(
            message.contains("`products` that is not a list"),
            "{message}"
        );
    }

    /// The fixture gem is reachable from a crate nested one level below it,
    /// which is the walk-up the real `gems/<gem>/<crate>` layout needs.
    #[test]
    fn the_search_walks_up_out_of_a_nested_contribution_crate() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("mount")
            .join("prefab");
        let entry = read("az-gem-mount-prefab", &root, None).expect("the fixture gem is above it");
        assert_eq!(entry.gem, "azoth.mount");
        assert_eq!(entry.call.to_string(), "prefab_types_contribution");
        assert!(
            entry.path.ends_with("tests/mount/gem.toml"),
            "{}",
            entry.path
        );
    }

    #[test]
    fn the_manifest_path_is_shown_relative_to_the_crate() {
        let root = Path::new("/gems/mount/prefab");
        assert_eq!(
            relative(Path::new("/gems/mount/prefab/gem.toml"), root),
            GEM
        );
        assert_eq!(
            relative(Path::new("/gems/mount/gem.toml"), root),
            "../gem.toml"
        );
        assert_eq!(relative(Path::new("/gem.toml"), root), "../../../gem.toml");
    }
}
