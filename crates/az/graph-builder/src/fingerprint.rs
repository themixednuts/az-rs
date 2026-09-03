//! Derived identity for the `az.graph.compiler` build rule.
//!
//! This rule is the hardest case in the pipeline, because two independent
//! things decide what bytes it produces and neither is visible to the other.
//!
//! **Its own code.** Four encoders live inline in this crate, and the magic
//! numbers and format versions they write come from `az-graph-runtime`. No
//! runtime value reflects any of it: reordering fields in `encode_packed_ir`
//! changes every output byte and changes no descriptor. Both crates therefore
//! contribute a build-time source fingerprint.
//!
//! **Its composed registries.** Uniquely among the build rules, this rule's
//! *shape* is computed rather than declared — the schema list it matches on
//! comes from the composed graph types, so contributing a new graph type
//! changes which sources the rule claims. Node types, compiler backends, asset
//! types, and product formats are likewise contributed by crates this one does
//! not depend on and whose sources it cannot hash. A source fingerprint can
//! never see a node type contributed by a downstream gem.
//!
//! Neither half subsumes the other, so the fingerprint carries both.
//!
//! Graph and node catalogs are hashed through their own `content_hash`, which
//! is a blake3 over the serialized catalog. That is deliberate: `Serialize` is
//! derived, so a field added to a descriptor enters the fingerprint with no
//! one remembering to list it here. The catalogs are built with the same
//! coordinates the compiler itself passes, so the fingerprint describes the
//! catalog the compiler actually compiles against.

use az_asset_builder::{JobContext, composed_product_formats, fingerprint::AnalysisFingerprint};
use az_core::composed_asset_types;
use az_gem_contract::Registries;
use az_node_graph::{GraphTypeCatalog, NodeTypeCatalog};

use crate::registered_graph_compiler_backends;

const DOMAIN: &str = "az.graph-compiler/v1:";

/// Catalog coordinates matching every other `compose` call in this crate.
/// `generated_unix_ms` must stay 0: it is part of the serialized catalog, so a
/// real timestamp would make the fingerprint differ on every process start and
/// reprocess every graph asset each run.
const CATALOG_VERSION: u32 = 1;
const CATALOG_GENERATED_UNIX_MS: u64 = 0;

/// The rule's own view of the catalogs it compiles against, as its host
/// composed them.
///
/// This is not cached per process: two hosts in one process compose different
/// graph and node types, and the whole point of the value is to differ when
/// they do.
pub fn analysis(context: JobContext<'_>) -> String {
    compute(context.registries())
}

fn compute(registries: &Registries) -> String {
    AnalysisFingerprint::new(DOMAIN)
        .field("graph-builder-sources", crate::SOURCE_FINGERPRINT)
        .field(
            "graph-runtime-sources",
            az_graph_runtime::SOURCE_FINGERPRINT,
        )
        .field("graph-types", &graph_type_catalog_hash(registries))
        .field("node-types", &node_type_catalog_hash(registries))
        .sorted_list("compiler-backends", backend_entries())
        .sorted_list("asset-types", asset_type_entries(registries))
        .sorted_list("product-formats", product_format_entries(registries))
        .finish()
}

/// Render a catalog digest, folding failures into the value instead of
/// panicking.
///
/// An invalid catalog already fails every compile that touches it, so the
/// fingerprint's only duty here is to stay deterministic and to keep differing
/// from the healthy value.
fn graph_type_catalog_hash(registries: &Registries) -> String {
    match GraphTypeCatalog::compose(CATALOG_VERSION, CATALOG_GENERATED_UNIX_MS, registries) {
        Ok(catalog) => match catalog.content_hash() {
            Ok(hash) => hex(hash),
            Err(error) => format!("hash-error:{error}"),
        },
        Err(error) => format!("catalog-error:{error}"),
    }
}

fn node_type_catalog_hash(registries: &Registries) -> String {
    match NodeTypeCatalog::compose(CATALOG_VERSION, CATALOG_GENERATED_UNIX_MS, registries) {
        Ok(catalog) => match catalog.content_hash() {
            Ok(hash) => hex(hash),
            Err(error) => format!("hash-error:{error}"),
        },
        Err(error) => format!("catalog-error:{error}"),
    }
}

/// Which backends are linked in. What each one *does* lives in this crate's
/// sources and is covered by the source fingerprint.
fn backend_entries() -> Vec<String> {
    registered_graph_compiler_backends()
        .into_iter()
        .map(|backend| backend.backend_id().to_owned())
        .collect()
}

fn asset_type_entries(registries: &Registries) -> Vec<String> {
    composed_asset_types(registries)
        .into_iter()
        .map(|registration| {
            format!(
                "{}={}|{}",
                registration.stable_name(),
                registration.asset_type().0,
                registration.owner(),
            )
        })
        .collect()
}

fn product_format_entries(registries: &Registries) -> Vec<String> {
    composed_product_formats(registries)
        .into_iter()
        .map(|attributed| {
            format!(
                "{}={}|{}",
                attributed.entry.id().as_str(),
                attributed.entry.current_version(),
                attributed.instance.gem.as_str(),
            )
        })
        .collect()
}

fn hex(hash: [u8; 32]) -> String {
    blake3::Hash::from(hash).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::test_composer;

    #[test]
    fn fingerprint_is_domain_prefixed_and_stable_for_one_composition() {
        let composer = test_composer();
        let context = JobContext::new(composer.registries());
        let fingerprint = analysis(context);
        assert!(fingerprint.starts_with(DOMAIN));
        assert_eq!(fingerprint, analysis(context));
    }

    #[test]
    fn the_rule_publishes_the_derived_fingerprint_instead_of_an_empty_string() {
        let composer = test_composer();
        let context = JobContext::new(composer.registries());
        let rule = crate::graph_compiler_build_rule(&context);
        assert_eq!(rule.analysis_fingerprint, analysis(context));
        assert!(!rule.analysis_fingerprint.is_empty());
    }

    #[test]
    fn source_fingerprints_are_real_and_participate() {
        // A crate that failed to run its build script would silently publish
        // an empty component and hash like every other broken crate.
        assert!(crate::SOURCE_FINGERPRINT.starts_with("azoth.source-tree/v1:"));
        assert!(az_graph_runtime::SOURCE_FINGERPRINT.starts_with("azoth.source-tree/v1:"));
        assert_ne!(
            crate::SOURCE_FINGERPRINT,
            az_graph_runtime::SOURCE_FINGERPRINT
        );
    }

    #[test]
    fn every_registry_is_actually_reachable() {
        // Guards against a registry silently emptying out and the fingerprint
        // degenerating to a constant that never changes again.
        let composer = test_composer();
        let registries = composer.registries();
        assert!(!backend_entries().is_empty());
        assert!(!asset_type_entries(registries).is_empty());
        assert!(!product_format_entries(registries).is_empty());
        assert!(!az_node_graph::node_types(registries).is_empty());
    }

    #[test]
    fn catalog_hashes_are_healthy_digests_not_folded_errors() {
        let composer = test_composer();
        let registries = composer.registries();
        for hash in [
            graph_type_catalog_hash(registries),
            node_type_catalog_hash(registries),
        ] {
            assert!(
                !hash.contains("error:"),
                "catalog hash degraded to an error marker: {hash}"
            );
            assert_eq!(hash.len(), 64, "expected a blake3 hex digest, got {hash}");
        }
    }

    #[test]
    fn composed_catalogs_move_the_fingerprint() {
        // The whole point of the catalog fields: a host that composed graph
        // and node types must not fingerprint like one that composed none.
        assert_ne!(
            compute(&Registries::new()),
            compute(test_composer().registries()),
            "composed catalogs must reach the fingerprint"
        );
    }

    #[test]
    fn each_registry_occupies_a_distinct_slot() {
        // Feeding one registry's entries under another's label must not
        // reproduce the fingerprint, or a change in one could mask a change
        // in the other.
        let composer = test_composer();
        let registries = composer.registries();
        let base = AnalysisFingerprint::new(DOMAIN)
            .sorted_list("asset-types", asset_type_entries(registries))
            .finish();
        let swapped = AnalysisFingerprint::new(DOMAIN)
            .sorted_list("product-formats", asset_type_entries(registries))
            .finish();
        assert_ne!(base, swapped);
    }
}
