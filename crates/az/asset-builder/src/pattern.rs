//! `AssetBuilderPattern` — file-name pattern that dispatches a
//! source file to a builder.
//!
//! Mirrors `AssetBuilderSDK::AssetBuilderPattern`
//! (`AssetBuilderSDK.h:174-200`). Two flavors:
//!
//! - **Wildcard** — glob with `*` and `?` (case-insensitive). The
//!   common case; e.g. `*.cgf`, `*.dds.split`.
//! - **Regex** — full Rust regex with `^`/`$` matching across the
//!   normalized source path. Use sparingly — wildcard is faster.
//!
//! Patterns are not paths, but matching is defined as *fold both
//! sides*: pattern and candidate are both put through
//! [`normalize_source_path`] — the one canonical asset-path form, the
//! same one we hash for `AssetId` — so a match is a byte comparison
//! and there is no separate case rule living in this module.

use regex::Regex;
use thiserror::Error;

use az_filesystem::normalize_source_path;

/// One pattern in a builder's pattern set.
#[derive(Debug, Clone)]
pub enum AssetBuilderPattern {
    /// Static glob with `*` (any chars) and `?` (one char). This variant is
    /// usable from typed descriptor constants.
    ///
    /// A `const` cannot run the canonicalizer, so the constant itself must
    /// already be written in canonical asset-path form (lowercase, forward
    /// slashes). See [`AssetBuilderPattern::static_wildcard`].
    StaticWildcard(&'static str),
    /// Glob with `*` (any chars) and `?` (one char), stored in canonical
    /// asset-path form so matching is a byte comparison.
    Wildcard(String),
    /// Compiled Rust regex matched against the normalized path.
    Regex(Regex),
}

impl PartialEq for AssetBuilderPattern {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::StaticWildcard(left), Self::StaticWildcard(right)) => left == right,
            (Self::Wildcard(left), Self::Wildcard(right)) => left == right,
            (Self::Regex(left), Self::Regex(right)) => left.as_str() == right.as_str(),
            _ => false,
        }
    }
}

impl Eq for AssetBuilderPattern {}

#[derive(Debug, Error)]
pub enum PatternError {
    #[error("invalid regex pattern: {0}")]
    InvalidRegex(#[from] regex::Error),
}

impl AssetBuilderPattern {
    /// Build a wildcard pattern from a glob (e.g. `"*.cgf"`).
    ///
    /// Matching is defined as *fold both sides*: the glob is canonicalized
    /// here with [`normalize_source_path`] and the candidate path is
    /// canonicalized with the same function in [`Self::matches`], so the two
    /// share one spelling and comparison is purely byte-for-byte. Folding at
    /// construction rather than per match keeps the hot path allocation-free
    /// on the pattern side; it is not a separate case rule.
    #[must_use]
    pub fn wildcard(glob: impl AsRef<str>) -> Self {
        Self::Wildcard(normalize_source_path(glob))
    }

    /// Build a const-friendly wildcard pattern from a static glob.
    ///
    /// Prefer [`Self::wildcard`] for runtime/dynamic pattern construction. This
    /// variant exists so `SourceFormat` descriptors can carry patterns in
    /// associated constants.
    ///
    /// A `const fn` cannot call the canonicalizer, so `glob` must already be
    /// written in canonical asset-path form — lowercase, `/`-separated, no
    /// leading `./` or `/`. The `asset-builder-derive` macro emits patterns
    /// that already satisfy this; hand-written constants must too, or they
    /// will silently never match.
    #[must_use]
    pub const fn static_wildcard(glob: &'static str) -> Self {
        Self::StaticWildcard(glob)
    }

    /// Build a regex pattern. Returns an error if the regex doesn't
    /// compile.
    /// # Errors
    ///
    /// Returns [`PatternError`] when `pattern` is not a valid regular expression.
    pub fn regex(pattern: &str) -> Result<Self, PatternError> {
        Ok(Self::Regex(Regex::new(pattern)?))
    }

    /// Test whether this pattern matches the given source path.
    ///
    /// `path` is canonicalized with [`normalize_source_path`] before matching
    /// — the same transform the pattern itself went through — so callers can
    /// pass a raw `PakEntry` name verbatim and must never pre-normalize.
    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        let normalized = normalize_source_path(path);
        match self {
            Self::StaticWildcard(glob) => wildcard_match(glob, &normalized),
            Self::Wildcard(glob) => wildcard_match(glob, &normalized),
            Self::Regex(re) => re.is_match(&normalized),
        }
    }
}

/// Recursive wildcard matcher with `*` and `?`.
///
/// `*` matches any sequence (including empty); `?` matches exactly
/// one character. Both inputs are assumed to already be in canonical
/// asset-path form, so this compares bytes and never folds case.
fn wildcard_match(pattern: &str, candidate: &str) -> bool {
    // Iterative two-pointer with star-backtrack — O(n*m) worst case
    // but linear for typical glob/path inputs and avoids the
    // allocation overhead of a recursive impl.
    let pat = pattern.as_bytes();
    let txt = candidate.as_bytes();
    let mut p = 0usize;
    let mut t = 0usize;
    let mut star: Option<usize> = None;
    let mut star_t = 0usize;

    while t < txt.len() {
        if p < pat.len() && (pat[p] == b'?' || pat[p] == txt[t]) {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star = Some(p);
            star_t = t;
            p += 1;
        } else if let Some(sp) = star {
            p = sp + 1;
            star_t += 1;
            t = star_t;
        } else {
            return false;
        }
    }

    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matches_extension() {
        let pat = AssetBuilderPattern::wildcard("*.cgf");
        assert!(pat.matches("objects/foo/bar.cgf"));
        assert!(pat.matches("Objects\\Foo\\Bar.CGF"));
        assert!(!pat.matches("objects/foo/bar.cga"));
    }

    #[test]
    fn static_wildcard_matches_extension() {
        const PAT: AssetBuilderPattern = AssetBuilderPattern::static_wildcard("*.cgf");
        assert!(PAT.matches("objects/foo/bar.cgf"));
        assert!(PAT.matches("Objects\\Foo\\Bar.CGF"));
        assert!(!PAT.matches("objects/foo/bar.cga"));
    }

    #[test]
    fn wildcard_supports_question_mark() {
        let pat = AssetBuilderPattern::wildcard("foo?.txt");
        assert!(pat.matches("foo1.txt"));
        assert!(pat.matches("fooa.txt"));
        assert!(!pat.matches("foo.txt"));
        assert!(!pat.matches("foo12.txt"));
    }

    #[test]
    fn wildcard_handles_double_star_path_segments() {
        // Lumberyard's glob doesn't support `**`; `*` matches across
        // separators, which is what we want for things like
        // `*.entities_xml`.
        let pat = AssetBuilderPattern::wildcard("*.entities_xml");
        assert!(pat.matches("levels/frontend_settlement/mission0.entities_xml"));
    }

    #[test]
    fn wildcard_split_dds_mip_pattern() {
        // `*.dds.[0-9a-f]?` style — stick to wildcard, fall back to
        // regex if needed.
        let pat = AssetBuilderPattern::wildcard("*.dds.?");
        assert!(pat.matches("textures/foo.dds.1"));
        assert!(pat.matches("textures/foo.dds.a"));
        assert!(!pat.matches("textures/foo.dds"));
    }

    /// Pattern and candidate go through the same canonicalizer, so a glob
    /// authored with Windows separators or mixed case still matches.
    #[test]
    fn wildcard_folds_the_pattern_and_the_candidate() {
        let pat = AssetBuilderPattern::wildcard(r"Objects\*.CGF");
        assert!(pat.matches("objects/foo/bar.cgf"));
        assert!(pat.matches(r"Objects\Foo\Bar.CGF"));
        assert!(pat.matches("/objects//foo/bar.cgf"));
        assert!(!pat.matches("models/foo/bar.cgf"));

        // Canonical globs are fixed points of the same transform.
        assert_eq!(
            AssetBuilderPattern::wildcard("*.cgf"),
            AssetBuilderPattern::wildcard("*.CGF")
        );
    }

    /// `static_wildcard` cannot canonicalize in a `const`, so it relies on the
    /// constant already being canonical. Confirm the contract holds for the
    /// shape `asset-builder-derive` emits.
    #[test]
    fn static_wildcard_constants_are_already_canonical() {
        const GLOB: &str = "*.cgf";
        assert_eq!(normalize_source_path(GLOB), GLOB);
        assert!(AssetBuilderPattern::static_wildcard(GLOB).matches(r"Objects\Foo\Bar.CGF"));
    }

    #[test]
    fn regex_pattern_matches() {
        let pat = AssetBuilderPattern::regex(r"^objects/.+\.(cgf|cga)$").unwrap();
        assert!(pat.matches("objects/foo/bar.cgf"));
        assert!(pat.matches("Objects/Foo/BAR.cga"));
        assert!(!pat.matches("models/foo/bar.cgf"));
    }
}
