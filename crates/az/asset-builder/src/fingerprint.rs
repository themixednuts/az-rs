//! Compose a build rule's analysis fingerprint from derived inputs.
//!
//! A rule's descriptor hash decides whether previously processed products are
//! still current. It can only see what the rule declares, so a rule that
//! declares nothing about the code behind its bytes cannot notice when that
//! code changes. Declared identity — a hand-maintained counter, a hand-written
//! descriptor — has already drifted in this workspace; derived identity
//! cannot be forgotten.
//!
//! Two kinds of input are derivable, and most rules need both:
//!
//! - **Source-tree fingerprints**, produced by `az-build-fingerprint` for each
//!   crate whose code frames the rule's bytes. These cover encoder and
//!   analysis logic that no runtime value reflects.
//! - **Link-time registry fingerprints**, computed from the inventories a rule
//!   reads at runtime. These cover contributions from crates the rule does not
//!   depend on and whose sources it therefore cannot hash.
//!
//! Neither subsumes the other. A source hash cannot see a node type registered
//! by a downstream gem; a registry hash cannot see a reordered byte writer.
//!
//! Every list is sorted before hashing. `inventory` iteration order follows
//! link order and is not stable, so an unsorted registry hash would change
//! between builds of identical code and reprocess everything.

use std::sync::OnceLock;

/// Accumulates the derived inputs identifying the code behind a rule's
/// products, then renders them as a single domain-prefixed digest.
///
/// ```
/// use az_asset_builder::fingerprint::AnalysisFingerprint;
///
/// let fingerprint = AnalysisFingerprint::new("azoth.example/v1:")
///     .field("codec", "azoth.source-tree/v1:abc123")
///     .sorted_list("formats", ["azoth.example.b", "azoth.example.a"])
///     .finish();
/// assert!(fingerprint.starts_with("azoth.example/v1:"));
/// ```
#[must_use]
pub struct AnalysisFingerprint {
    domain: &'static str,
    hasher: blake3::Hasher,
}

impl AnalysisFingerprint {
    /// Start a fingerprint under a domain marker.
    ///
    /// The marker is both hashed and prefixed onto the rendered value, so two
    /// domains never collide and a stored fingerprint says what produced it.
    pub fn new(domain: &'static str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hash_text(&mut hasher, domain);
        Self { domain, hasher }
    }

    /// Add a labelled text input, such as another crate's source fingerprint.
    pub fn field(mut self, label: &str, value: &str) -> Self {
        hash_text(&mut self.hasher, label);
        hash_text(&mut self.hasher, value);
        self
    }

    /// Add a labelled numeric input.
    pub fn number(mut self, label: &str, value: u64) -> Self {
        hash_text(&mut self.hasher, label);
        self.hasher.update(&value.to_le_bytes());
        self
    }

    /// Add a labelled list, sorted so link order cannot change the digest.
    ///
    /// The entry count is hashed alongside the entries, so a list cannot be
    /// confused with a shorter list whose entries concatenate to the same
    /// text.
    pub fn sorted_list<I, S>(mut self, label: &str, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut entries = values
            .into_iter()
            .map(|value| value.as_ref().to_owned())
            .collect::<Vec<_>>();
        entries.sort_unstable();

        hash_text(&mut self.hasher, label);
        hash_len(&mut self.hasher, entries.len());
        for entry in &entries {
            hash_text(&mut self.hasher, entry);
        }
        self
    }

    /// Render the domain-prefixed fingerprint.
    #[must_use]
    pub fn finish(self) -> String {
        format!("{}{}", self.domain, self.hasher.finalize().to_hex())
    }
}

/// Compute a fingerprint once per process and hand out a `&'static str`.
///
/// Rules publish `analysis_fingerprint` as a `&'static str`, but a derived
/// fingerprint is only known at runtime. This is the shared plumbing so each
/// rule does not reimplement the cache.
pub fn cached(slot: &'static OnceLock<String>, compute: impl FnOnce() -> String) -> &'static str {
    slot.get_or_init(compute).as_str()
}

fn hash_text(hasher: &mut blake3::Hasher, value: &str) {
    hash_len(hasher, value.len());
    hasher.update(value.as_bytes());
}

fn hash_len(hasher: &mut blake3::Hasher, value: usize) {
    let value = u64::try_from(value).expect("fingerprint lengths fit in u64");
    hasher.update(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAIN: &str = "azoth.test-fingerprint/v1:";

    fn fingerprint() -> AnalysisFingerprint {
        AnalysisFingerprint::new(DOMAIN)
    }

    #[test]
    fn rendered_fingerprint_is_domain_prefixed() {
        assert!(fingerprint().finish().starts_with(DOMAIN));
    }

    #[test]
    fn distinct_domains_do_not_collide_on_identical_inputs() {
        let left = AnalysisFingerprint::new("azoth.left/v1:")
            .field("a", "b")
            .finish();
        let right = AnalysisFingerprint::new("azoth.right/v1:")
            .field("a", "b")
            .finish();
        assert_ne!(
            left.trim_start_matches("azoth.left/v1:"),
            right.trim_start_matches("azoth.right/v1:"),
        );
    }

    #[test]
    fn a_changed_field_changes_the_fingerprint() {
        assert_ne!(
            fingerprint().field("codec", "one").finish(),
            fingerprint().field("codec", "two").finish(),
        );
    }

    #[test]
    fn an_unrelated_field_is_not_reachable_by_relabelling() {
        // Label and value are each length-prefixed, so moving text across the
        // boundary must not produce the same digest.
        assert_ne!(
            fingerprint().field("ab", "c").finish(),
            fingerprint().field("a", "bc").finish(),
        );
    }

    #[test]
    fn sorted_list_ignores_input_order() {
        assert_eq!(
            fingerprint().sorted_list("types", ["a", "b", "c"]).finish(),
            fingerprint().sorted_list("types", ["c", "a", "b"]).finish(),
        );
    }

    #[test]
    fn sorted_list_tracks_membership() {
        assert_ne!(
            fingerprint().sorted_list("types", ["a", "b"]).finish(),
            fingerprint().sorted_list("types", ["a", "b", "c"]).finish(),
        );
    }

    #[test]
    fn sorted_list_is_distinct_from_a_concatenated_entry() {
        assert_ne!(
            fingerprint().sorted_list("types", ["a", "b"]).finish(),
            fingerprint().sorted_list("types", ["ab"]).finish(),
        );
    }

    #[test]
    fn numbers_and_text_occupy_distinct_slots() {
        assert_ne!(
            fingerprint().number("version", 1).finish(),
            fingerprint().field("version", "1").finish(),
        );
    }

    #[test]
    fn cached_computes_once() {
        static SLOT: OnceLock<String> = OnceLock::new();
        let first = cached(&SLOT, || "first".to_owned());
        let second = cached(&SLOT, || "second".to_owned());
        assert_eq!(first, "first");
        assert_eq!(second, "first");
    }
}
