//! Typo-resistant fuzzy matching for in-editor list filters.
//!
//! Backed by [`frizbee`] (the SIMD Smith–Waterman matcher at the core of the
//! `fff` file finder). This is for filtering UI lists — authored objects, gems,
//! asset entries, node palettes — by a user-typed query, NOT for filesystem
//! path search. A subsequence query like `fBr` matches `fooBar`, and small
//! typos still match, which plain `contains` can't do.

/// Whether `text` fuzzy-matches `query`.
///
/// An empty query matches everything, so
/// this is a drop-in replacement for `text.to_ascii_lowercase().contains(query)`
/// in list filters. Case-insensitive (frizbee lowercases internally).
#[must_use]
pub fn matches(query: &str, text: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    !frizbee::match_list(query, &[text], &frizbee::Config::default()).is_empty()
}

/// Whether `text` fuzzy-matches `query`, returning the match score (higher is a
/// better match) so callers can rank results.
///
/// `Some(_)` when it matches, `None`
/// otherwise. An empty query yields `Some(0)` (everything matches, no ranking).
#[must_use]
pub fn score(query: &str, text: &str) -> Option<u16> {
    let query = query.trim();
    if query.is_empty() {
        return Some(0);
    }
    frizbee::match_list(query, &[text], &frizbee::Config::default())
        .first()
        .map(|m| m.score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything() {
        assert!(matches("", "anything"));
        assert!(matches("   ", "anything"));
    }

    #[test]
    fn subsequence_matches() {
        assert!(matches("fBr", "fooBar"));
        assert!(matches("vec3", "core.vec3"));
    }

    #[test]
    fn non_match_returns_false() {
        assert!(!matches("zzzq", "fooBar"));
    }

    #[test]
    fn score_orders_better_matches_higher() {
        let exact = score("transform", "transform").unwrap();
        let loose = score("transform", "transformer_component_extra").unwrap();
        assert!(exact >= loose);
    }
}
