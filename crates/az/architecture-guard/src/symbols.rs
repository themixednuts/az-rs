//! Formatting-insensitive symbol checks over Rust source text.
//!
//! Architecture guards used to pin exact source snippets with
//! `source.contains("self.foo(event, window, cx);")`. Those break on any
//! reformat or line-wrap while simultaneously matching prose inside comments
//! and string literals. [`symbol_skeleton`] replaces that pattern: it strips
//! comments and literal contents, splits the remaining code into word and
//! punctuation units separated by single spaces, and guards match patterns
//! built by running the same normalizer over a natural snippet. The check
//! then asserts on *symbols* — identifier and punctuation sequences — which
//! survive reformatting but still fire on renames and removals.

/// Normalize Rust source (or any fragment, including a single function body
/// sliced out of a file) into a single-line code skeleton.
///
/// - Line comments, nested block comments, and the contents of string, raw
///   string, byte-string, char, and byte-char literals are removed; each
///   literal becomes one `_` unit so surrounding tokens cannot merge across
///   it.
/// - Identifier runs (`[A-Za-z0-9_]+`) stay whole, so `status_bar` never
///   matches a `status` pattern.
/// - Every other character becomes its own unit and all units are joined
///   with exactly one space, so wrapped and unwrapped copies of the same
///   code normalize identically.
///
/// Accepted limitations: the scanner is not a full Rust lexer. Generic const
/// expressions inside array-length brackets, labeled raw strings deeper than
/// the common `r#` form, and macro-invented syntax normalize imperfectly. No
/// boundary pattern this crate matches depends on those corners; extend the
/// scanner rather than reverting a guard to raw `contains` if one ever does.
#[must_use]
pub fn symbol_skeleton(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut unit = String::new();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];

        if ch.is_whitespace() {
            flush_unit(&mut unit, &mut out);
            index += 1;
            continue;
        }

        if let Some(next) = skip_comment(&chars, index) {
            flush_unit(&mut unit, &mut out);
            index = next;
            continue;
        }

        // Strings and raw strings, including b"" / br#"..."# prefixes.
        if ch == '"' {
            flush_unit(&mut unit, &mut out);
            index = skip_quoted_literal(&chars, index + 1, '"');
            push_placeholder(&mut unit, &mut out);
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == '_' {
            index = lex_word_or_prefixed_literal(&chars, index, &mut unit, &mut out);
            continue;
        }

        // Char literals vs lifetimes.
        if ch == '\'' {
            flush_unit(&mut unit, &mut out);
            index = lex_char_or_lifetime(&chars, index, &mut unit, &mut out);
            continue;
        }

        // Numbers (including underscores and suffixes like 1_000u32, 0xFF).
        if ch.is_ascii_digit() {
            while index < chars.len() && is_ident_char(chars[index]) {
                unit.push(chars[index]);
                index += 1;
            }
            flush_unit(&mut unit, &mut out);
            continue;
        }

        // Any other punctuation: one unit per character.
        unit.push(ch);
        flush_unit(&mut unit, &mut out);
        index += 1;
    }

    flush_unit(&mut unit, &mut out);
    out
}

/// If a comment starts at `index`, returns the index just past it.
///
/// Handles both `//` line comments and `/* */` block comments, which nest.
fn skip_comment(chars: &[char], index: usize) -> Option<usize> {
    if chars[index] != '/' {
        return None;
    }
    match chars.get(index + 1) {
        Some(&'/') => {
            let mut cursor = index + 2;
            while cursor < chars.len() && chars[cursor] != '\n' {
                cursor += 1;
            }
            Some(cursor)
        }
        Some(&'*') => {
            let mut depth = 1;
            let mut cursor = index + 2;
            while cursor < chars.len() && depth > 0 {
                if chars[cursor] == '/' && chars.get(cursor + 1) == Some(&'*') {
                    depth += 1;
                    cursor += 2;
                } else if chars[cursor] == '*' && chars.get(cursor + 1) == Some(&'/') {
                    depth -= 1;
                    cursor += 2;
                } else {
                    cursor += 1;
                }
            }
            Some(cursor)
        }
        _ => None,
    }
}

/// Lexes an identifier, or the `r` / `b` prefixed form of a string, byte, or
/// raw literal, starting at `index`.
///
/// Returns the index just past whatever it consumed, having already written
/// the resulting unit or literal placeholder.
fn lex_word_or_prefixed_literal(
    chars: &[char],
    index: usize,
    unit: &mut String,
    out: &mut String,
) -> usize {
    let start = index;
    let mut cursor = index;
    while cursor < chars.len() && is_ident_char(chars[cursor]) {
        cursor += 1;
    }
    let word: String = chars[start..cursor].iter().collect();
    // Raw/byte-string prefixes lex as bare `r`, `b`, or `br` words
    // immediately followed by a quote or hash-quote; anything else is
    // an ordinary identifier.
    match word.as_str() {
        "r" => {
            if let Some(hashes) = raw_string_hashes(chars, cursor) {
                flush_unit(unit, out);
                let next = skip_raw_string(chars, hashes.start, hashes.count);
                push_placeholder(unit, out);
                return next;
            }
        }
        "b" => {
            if chars.get(cursor) == Some(&'"') {
                flush_unit(unit, out);
                let next = skip_quoted_literal(chars, cursor + 1, '"');
                push_placeholder(unit, out);
                return next;
            }
            if chars.get(cursor) == Some(&'\'') {
                flush_unit(unit, out);
                let next = skip_quoted_literal(chars, cursor + 1, '\'');
                push_placeholder(unit, out);
                return next;
            }
            if chars.get(cursor) == Some(&'r')
                && let Some(hashes) = raw_string_hashes(chars, cursor + 1)
            {
                flush_unit(unit, out);
                let next = skip_raw_string(chars, hashes.start, hashes.count);
                push_placeholder(unit, out);
                return next;
            }
        }
        _ => {}
    }
    for &word_ch in &chars[start..cursor] {
        unit.push(word_ch);
    }
    flush_unit(unit, out);
    cursor
}

/// Lexes the char literal or lifetime whose opening `'` sits at `index`.
///
/// Returns the index just past whatever it consumed. The caller has already
/// flushed the pending unit.
fn lex_char_or_lifetime(
    chars: &[char],
    index: usize,
    unit: &mut String,
    out: &mut String,
) -> usize {
    match chars.get(index + 1) {
        Some(&'\\') => {
            // Escape form: '\n', '\'', '\\', '\u{...}'.
            let mut cursor = index + 2;
            while cursor < chars.len() && (chars[cursor] != '\'' || escape_open(chars, cursor)) {
                cursor += 1;
            }
            push_placeholder(unit, out);
            cursor + 1 // closing quote
        }
        Some(&next) if next.is_ascii_alphabetic() || next == '_' => {
            let run_start = index + 1;
            let mut run_end = run_start;
            while run_end < chars.len() && is_ident_char(chars[run_end]) {
                run_end += 1;
            }
            let is_char_literal = run_end - run_start == 1 && chars.get(run_end) == Some(&'\'');
            if is_char_literal {
                push_placeholder(unit, out);
                run_end + 1
            } else {
                // Lifetime: keep `'name` as one unit.
                for &lifetime_ch in &chars[index..run_end] {
                    unit.push(lifetime_ch);
                }
                flush_unit(unit, out);
                run_end
            }
        }
        _ => {
            // One-character literal such as '(' ' '.
            push_placeholder(unit, out);
            index + 3
        }
    }
}

/// Returns true when [`symbols_contain`] would find `snippet` in `source`.
///
/// # Panics
///
/// Panics if `snippet` contains no code tokens, since an empty needle would
/// match every source.
#[must_use]
pub fn symbols_contain(source: &str, snippet: &str) -> bool {
    let needle = symbol_skeleton(snippet);
    assert!(!needle.is_empty(), "snippet must contain code tokens");
    format!(" {} ", symbol_skeleton(source)).contains(&format!(" {needle} "))
}

/// Counts non-overlapping [`symbol_skeleton`] occurrences of `snippet` in
/// `source`.
///
/// # Panics
///
/// Panics if `snippet` contains no code tokens, since an empty needle would
/// match every source.
#[must_use]
pub fn symbols_count(source: &str, snippet: &str) -> usize {
    let needle = symbol_skeleton(snippet);
    assert!(!needle.is_empty(), "snippet must contain code tokens");
    format!(" {} ", symbol_skeleton(source))
        .matches(&format!(" {needle} "))
        .count()
}

/// Checks `source` for user-facing *contract text* — UI menu labels, keymap
/// bindings, element ids, and error prefixes that live inside string
/// literals in production code.
///
/// [`symbol_skeleton`] deliberately strips literal contents, so it cannot
/// check these. Raw substring matching (the original `.contains`) is kept
/// here verbatim on purpose: labels often sit inside longer literals such as
/// `"Open Visual Graph Workspace"`, so the fragment must match mid-literal.
/// This helper exists so a guard grep for raw `.contains(` in editor crates
/// stays clean while the remaining literal checks stay visible, named, and
/// reviewed as one exception class.
///
/// # Panics
///
/// Panics if `needle` is empty, since an empty needle would match every
/// source.
#[must_use]
pub fn source_literal_contains(source: &str, needle: &str) -> bool {
    assert!(!needle.is_empty(), "literal needle must be non-empty");
    source.contains(needle)
}

const fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn flush_unit(unit: &mut String, out: &mut String) {
    if !unit.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(unit);
        unit.clear();
    }
}

fn push_placeholder(unit: &mut String, out: &mut String) {
    unit.push('_');
    flush_unit(unit, out);
}

struct RawStringHashes {
    /// Index of the opening quote.
    start: usize,
    count: usize,
}

/// If a raw-string quote begins at `index`, returns the quote position and
/// the number of hash marks preceding it.
///
/// Accepts both the bare quote form and the hash-delimited form.
fn raw_string_hashes(chars: &[char], index: usize) -> Option<RawStringHashes> {
    if chars.get(index) == Some(&'"') {
        return Some(RawStringHashes {
            start: index,
            count: 0,
        });
    }
    let mut count = 0;
    let mut cursor = index;
    while chars.get(cursor) == Some(&'#') {
        count += 1;
        cursor += 1;
    }
    if count > 0 && chars.get(cursor) == Some(&'"') {
        return Some(RawStringHashes {
            start: cursor,
            count,
        });
    }
    None
}

/// Skips a quoted literal starting just after the opening `quote`, honoring
/// backslash escapes; returns the index just past the closing quote.
fn skip_quoted_literal(chars: &[char], mut index: usize, quote: char) -> usize {
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 2,
            ch if ch == quote => return index + 1,
            _ => index += 1,
        }
    }
    index
}

/// Skips a raw string whose opening quote is at `start` with `count` hashes;
/// returns the index just past the closing `"###…`.
fn skip_raw_string(chars: &[char], start: usize, count: usize) -> usize {
    let mut index = start + 1;
    while index < chars.len() {
        if chars[index] == '"' {
            let mut hashes = 0;
            while hashes < count && chars.get(index + 1 + hashes) == Some(&'#') {
                hashes += 1;
            }
            if hashes == count {
                return index + 1 + count;
            }
        }
        index += 1;
    }
    index
}

/// True when the `'` at `index` terminates an escape-form char literal —
/// i.e. it does not itself belong to an escape sequence.
fn escape_open(chars: &[char], index: usize) -> bool {
    // Count consecutive backslashes before `index`; an odd run means the
    // quote is escaped.
    let mut slashes = 0;
    let mut cursor = index;
    while cursor > 0 && chars[cursor - 1] == '\\' {
        slashes += 1;
        cursor -= 1;
    }
    slashes % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_code_matches_unwrapped_snippets() {
        let source = r"
        let view = self
            .session_workspace_asset_view_with_client(client.clone())
            .await?;
        ";
        assert!(symbols_contain(
            source,
            "self.session_workspace_asset_view_with_client(client.clone())"
        ));
        assert!(symbols_contain(source, ".await?"));
    }

    #[test]
    fn comments_and_string_contents_never_match() {
        // The comment and the literal both mention `.status(`; only real code counts.
        let source = r#"
        // supervisor.status(session_slug).await? is forbidden here.
        let hint = ".status( must not appear";
        let ok = 1;
        "#;
        assert!(!symbols_contain(
            source,
            "supervisor.status(session_slug).await?"
        ));
        // Literal *contents* are stripped, so prose inside strings cannot
        // match code patterns either.
        assert!(!symbols_contain(source, "must not appear"));
        assert!(symbols_contain(source, "let hint = _ ;"));
    }

    #[test]
    fn identifiers_stay_whole() {
        let source = "let status_bar_width = compute_status_bar();";
        assert!(!symbols_contain(source, "status_bar ("));
        assert!(symbols_contain(source, "compute_status_bar ()"));
    }

    #[test]
    fn nested_block_comments_are_skipped() {
        let source = "/* outer /* inner .forbidden_call( */ still comment */ real_code();";
        assert!(!symbols_contain(source, ".forbidden_call("));
        assert!(symbols_contain(source, "real_code ()"));
    }

    #[test]
    fn raw_and_byte_strings_become_placeholders() {
        let source = r##"
        let raw = r#"regex ".status(" in raw"#;
        let bytes = br".status(";
        let plain = "x";
        let after = 1;
        "##;
        assert!(!symbols_contain(source, ".status("));
        assert!(symbols_contain(source, "let after = 1 ;"));
    }

    #[test]
    fn lifetimes_survive_but_char_literals_do_not_swallow_code() {
        let source = "fn f<'a>(x: &'a str) -> char { '\\'' }";
        assert!(symbols_contain(source, "fn f < 'a > ( x : & 'a str )"));
        assert!(symbols_contain(source, "-> char { _ }"));
    }

    #[test]
    fn counting_uses_padded_units_so_prefixes_do_not_collide() {
        let source = "a.status(); b.status(); c.status_bar();";
        assert_eq!(symbols_count(source, ".status()"), 2);
    }

    #[test]
    fn unbalanced_fragments_normalize_without_panicking() {
        // Function bodies sliced out mid-file end without their closing brace.
        let fragment = "if ready { load(x).await?";
        assert!(symbols_contain(fragment, "load ( x ) . await ?"));
    }
}
