//! Bounded structural read of an authored table source's prologue.
//!
//! Dependency discovery wants one thing from a source file: which table it is
//! and which merged row schema it claims. Materializing every row to learn that
//! is wasteful on a corpus of large tables, and slicing the file at a byte
//! pattern is worse — it makes the reader depend on the serializer's exact
//! pretty-print, so a comment that happens to contain the pattern cuts the file
//! in half.
//!
//! [`header`] reads the envelope's grammar instead. It consumes trivia,
//! identifiers, and string literals up to the `rows` field and stops there, so
//! a comment is a comment however it is spelled, and the read never depends on
//! how large the table is.

use std::fmt;

/// The prologue of an authored table source: everything the envelope declares
/// about itself before its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    name: String,
    schema: String,
    key: Option<String>,
}

impl Header {
    /// The physical table name this source declares.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The merged logical row schema this source claims.
    #[inline]
    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// The row-key column override, when the source names one.
    #[inline]
    #[must_use]
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }
}

/// Why a source's prologue could not be read.
///
/// The two failing shapes are kept apart on purpose: a prologue that ends
/// mid-token is an incomplete *file* (a partial write, a truncated transfer),
/// while a prologue the grammar rejects is a wrong *file* (the wrong format, or
/// an envelope with a field the format does not have). They call for different
/// answers, so they are different variants rather than one string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderError {
    /// The input ended while the prologue still needed more of it.
    Truncated {
        /// What the grammar was waiting for when the bytes ran out.
        expected: &'static str,
    },
    /// The prologue is present but is not an authored table envelope.
    Malformed {
        /// Byte offset into the source where the grammar gave up.
        offset: usize,
        /// What the grammar could not accept there.
        reason: &'static str,
    },
    /// The prologue is well formed but does not declare a field the envelope
    /// requires. A source that puts `rows` before `name` lands here: the rows
    /// are past the read's bound, so the field is genuinely not in the header.
    Missing {
        /// The envelope field that never appeared before the rows did.
        field: &'static str,
    },
}

impl fmt::Display for HeaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { expected } => write!(
                formatter,
                "GameData table source ends before its header does: expected {expected}"
            ),
            Self::Malformed { offset, reason } => write!(
                formatter,
                "GameData table source header is malformed at byte {offset}: {reason}"
            ),
            Self::Missing { field } => write!(
                formatter,
                "GameData table source header declares no `{field}` before its rows"
            ),
        }
    }
}

impl std::error::Error for HeaderError {}

/// Reads one authored table source's prologue without materializing its rows.
///
/// The read is bounded by the `rows` field: bytes after it are never examined,
/// which is the point — dependency discovery pays for the header, not for the
/// table. The bound is also the limit of what this can report. Damage beyond
/// the prologue (a file cut off inside its rows) leaves the header intact and
/// is returned as [`Ok`]; only compiling the source finds it.
///
/// There is no fallback to a full parse. A call site that wants "header, else
/// full parse" writes both calls, so the cost and the failure are visible where
/// they are chosen.
///
/// # Errors
///
/// Returns [`HeaderError::Truncated`] when the bytes run out inside the
/// prologue, [`HeaderError::Malformed`] when what is there is not an authored
/// table envelope, and [`HeaderError::Missing`] when the prologue is well
/// formed but `rows` arrives before `name`, `schema`, or `key`.
pub fn header(source: &[u8]) -> Result<Header, HeaderError> {
    let mut prologue = Prologue::new(source);
    prologue.envelope_open()?;

    let mut name = None;
    let mut schema = None;
    let mut key = None;
    let mut key_seen = false;
    let mut rows_seen = false;

    loop {
        prologue.trivia()?;
        match prologue.peek() {
            None => return Err(HeaderError::Truncated { expected: "`)`" }),
            Some(b')') => break,
            Some(_) => {}
        }

        let field = prologue.identifier("expected an envelope field name")?;
        prologue.trivia()?;
        prologue.expect(b':', "`:` after an envelope field name")?;
        if field == "rows" {
            rows_seen = true;
            break;
        }
        prologue.trivia()?;

        match field {
            "name" => set_once(&mut name, "name", prologue.string()?, prologue.at)?,
            "schema" => set_once(&mut schema, "schema", prologue.string()?, prologue.at)?,
            "key" => {
                if key_seen {
                    return Err(duplicate_field("key", prologue.at));
                }
                key = prologue.option_string()?;
                key_seen = true;
            }
            _ => {
                return Err(HeaderError::Malformed {
                    offset: prologue.at,
                    reason: "the envelope has no such field",
                });
            }
        }

        prologue.trivia()?;
        match prologue.peek() {
            Some(b',') => prologue.at += 1,
            Some(b')') => break,
            Some(_) => {
                return Err(HeaderError::Malformed {
                    offset: prologue.at,
                    reason: "expected `,` or `)` after an envelope field",
                });
            }
            None => {
                return Err(HeaderError::Truncated {
                    expected: "`,` or `)`",
                });
            }
        }
    }

    if !rows_seen {
        return Err(HeaderError::Missing { field: "rows" });
    }
    let name = name.ok_or(HeaderError::Missing { field: "name" })?;
    let schema = schema.ok_or(HeaderError::Missing { field: "schema" })?;
    if name.trim().is_empty() {
        return Err(HeaderError::Malformed {
            offset: 0,
            reason: "the envelope's `name` is empty",
        });
    }
    if schema.trim().is_empty() {
        return Err(HeaderError::Malformed {
            offset: 0,
            reason: "the envelope's `schema` is empty",
        });
    }
    if key.as_deref().is_some_and(|key| key.trim().is_empty()) {
        return Err(HeaderError::Malformed {
            offset: 0,
            reason: "the envelope's `key` is empty",
        });
    }

    Ok(Header { name, schema, key })
}

fn set_once<T>(
    slot: &mut Option<T>,
    field: &'static str,
    value: T,
    offset: usize,
) -> Result<(), HeaderError> {
    if slot.is_some() {
        return Err(duplicate_field(field, offset));
    }
    *slot = Some(value);
    Ok(())
}

fn duplicate_field(field: &'static str, offset: usize) -> HeaderError {
    HeaderError::Malformed {
        offset,
        reason: match field {
            "name" => "duplicate `name` field",
            "schema" => "duplicate `schema` field",
            "key" => "duplicate `key` field",
            _ => "duplicate envelope field",
        },
    }
}

/// A cursor over the envelope's prologue.
///
/// It works on bytes, not on a decoded string, so a source whose rows contain
/// invalid UTF-8 still yields its header, and validation costs only the string
/// literals actually read. Every structural byte of the grammar is ASCII, and a
/// UTF-8 continuation byte is never ASCII, so byte-wise scanning cannot land
/// inside a multi-byte character by accident.
struct Prologue<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Prologue<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.at + ahead).copied()
    }

    fn expect(&mut self, byte: u8, expected: &'static str) -> Result<(), HeaderError> {
        match self.peek() {
            Some(found) if found == byte => {
                self.at += 1;
                Ok(())
            }
            Some(_) => Err(HeaderError::Malformed {
                offset: self.at,
                reason: expected,
            }),
            None => Err(HeaderError::Truncated { expected }),
        }
    }

    /// Consumes whitespace and comments. RON block comments nest.
    fn trivia(&mut self) -> Result<(), HeaderError> {
        loop {
            match self.peek() {
                Some(byte) if byte.is_ascii_whitespace() => self.at += 1,
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    self.at += 2;
                    while self.peek().is_some_and(|byte| byte != b'\n') {
                        self.at += 1;
                    }
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    self.at += 2;
                    let mut depth = 1usize;
                    while depth > 0 {
                        match (self.peek(), self.peek_at(1)) {
                            (Some(b'/'), Some(b'*')) => {
                                depth += 1;
                                self.at += 2;
                            }
                            (Some(b'*'), Some(b'/')) => {
                                depth -= 1;
                                self.at += 2;
                            }
                            (Some(_), _) => self.at += 1,
                            (None, _) => {
                                return Err(HeaderError::Truncated {
                                    expected: "`*/` closing a block comment",
                                });
                            }
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    /// Consumes an optional struct name and the envelope's opening `(`.
    fn envelope_open(&mut self) -> Result<(), HeaderError> {
        self.trivia()?;
        if self.peek().is_some_and(is_identifier_start) {
            self.identifier("the envelope's struct name")?;
            self.trivia()?;
        }
        self.expect(b'(', "`(` opening the table envelope")
    }

    fn identifier(&mut self, expected: &'static str) -> Result<&'a str, HeaderError> {
        let start = self.at;
        if !self.peek().is_some_and(is_identifier_start) {
            return match self.peek() {
                Some(_) => Err(HeaderError::Malformed {
                    offset: self.at,
                    reason: expected,
                }),
                None => Err(HeaderError::Truncated { expected }),
            };
        }
        while self.peek().is_some_and(is_identifier_byte) {
            self.at += 1;
        }
        // Identifier bytes are ASCII by construction.
        Ok(std::str::from_utf8(&self.bytes[start..self.at]).unwrap_or_default())
    }

    /// Reads `Some("…")` or `None`.
    fn option_string(&mut self) -> Result<Option<String>, HeaderError> {
        let offset = self.at;
        match self.identifier("expected `Some(\"…\")` or `None`")? {
            "None" => Ok(None),
            "Some" => {
                self.trivia()?;
                self.expect(b'(', "`(` after `Some`")?;
                self.trivia()?;
                let value = self.string()?;
                self.trivia()?;
                self.expect(b')', "`)` closing `Some`")?;
                Ok(Some(value))
            }
            _ => Err(HeaderError::Malformed {
                offset,
                reason: "expected `Some(\"…\")` or `None`",
            }),
        }
    }

    fn string(&mut self) -> Result<String, HeaderError> {
        match self.peek() {
            Some(b'"') => self.quoted_string(),
            Some(b'r') => self.raw_string(),
            Some(_) => Err(HeaderError::Malformed {
                offset: self.at,
                reason: "expected a string literal",
            }),
            None => Err(HeaderError::Truncated {
                expected: "a string literal",
            }),
        }
    }

    fn quoted_string(&mut self) -> Result<String, HeaderError> {
        self.at += 1;
        let mut decoded = String::new();
        let mut literal_start = self.at;
        loop {
            match self.peek() {
                None => {
                    return Err(HeaderError::Truncated {
                        expected: "`\"` closing a string literal",
                    });
                }
                Some(b'"') => {
                    self.push_slice(&mut decoded, literal_start)?;
                    self.at += 1;
                    return Ok(decoded);
                }
                Some(b'\\') => {
                    self.push_slice(&mut decoded, literal_start)?;
                    self.at += 1;
                    decoded.push(self.escape()?);
                    literal_start = self.at;
                }
                Some(_) => self.at += 1,
            }
        }
    }

    fn escape(&mut self) -> Result<char, HeaderError> {
        let offset = self.at;
        let Some(byte) = self.peek() else {
            return Err(HeaderError::Truncated {
                expected: "an escape sequence",
            });
        };
        self.at += 1;
        match byte {
            b'"' => Ok('"'),
            b'\'' => Ok('\''),
            b'\\' => Ok('\\'),
            b'n' => Ok('\n'),
            b'r' => Ok('\r'),
            b't' => Ok('\t'),
            b'0' => Ok('\0'),
            b'x' => self.escape_hex(2, offset),
            b'u' => {
                self.expect(b'{', "`{` after `\\u`")?;
                let digits = self
                    .bytes
                    .get(self.at..)
                    .and_then(|rest| rest.iter().position(|byte| *byte == b'}'))
                    .ok_or(HeaderError::Truncated {
                        expected: "`}` closing a `\\u` escape",
                    })?;
                let value = self.escape_hex(digits, offset)?;
                self.at += 1;
                Ok(value)
            }
            _ => Err(HeaderError::Malformed {
                offset,
                reason: "unknown string escape",
            }),
        }
    }

    fn escape_hex(&mut self, digits: usize, offset: usize) -> Result<char, HeaderError> {
        let end = self.at + digits;
        let Some(slice) = self.bytes.get(self.at..end) else {
            return Err(HeaderError::Truncated {
                expected: "the digits of a hex escape",
            });
        };
        self.at = end;
        std::str::from_utf8(slice)
            .ok()
            .and_then(|text| u32::from_str_radix(text, 16).ok())
            .and_then(char::from_u32)
            .ok_or(HeaderError::Malformed {
                offset,
                reason: "a hex escape that is not a character",
            })
    }

    fn raw_string(&mut self) -> Result<String, HeaderError> {
        let offset = self.at;
        self.at += 1;
        let hashes = {
            let start = self.at;
            while self.peek() == Some(b'#') {
                self.at += 1;
            }
            self.at - start
        };
        self.expect(b'"', "`\"` opening a raw string literal")?;
        let start = self.at;
        loop {
            match self.peek() {
                None => {
                    return Err(HeaderError::Truncated {
                        expected: "the closing quote of a raw string literal",
                    });
                }
                Some(b'"') if (1..=hashes).all(|ahead| self.peek_at(ahead) == Some(b'#')) => {
                    let text = std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| {
                        HeaderError::Malformed {
                            offset,
                            reason: "a string literal that is not UTF-8",
                        }
                    })?;
                    let text = text.to_owned();
                    self.at += 1 + hashes;
                    return Ok(text);
                }
                Some(_) => self.at += 1,
            }
        }
    }

    fn push_slice(&self, decoded: &mut String, from: usize) -> Result<(), HeaderError> {
        let slice = &self.bytes[from..self.at];
        let text = std::str::from_utf8(slice).map_err(|_| HeaderError::Malformed {
            offset: from,
            reason: "a string literal that is not UTF-8",
        })?;
        decoded.push_str(text);
        Ok(())
    }
}

const fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

const fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::{Header, HeaderError, header};
    use crate::authoring::decode_table_source_ron;

    const CANONICAL: &[u8] = br#"(
    name: "MasterItemDefinitions",
    schema: "ItemData",
    key: Some("item_id"),
    rows: [
        (item_id: "Sword"),
    ],
)"#;

    /// The exact shape that broke the byte-sniffer this API replaces: the
    /// literal `\n    rows:` appears inside a comment, so slicing the file at
    /// its first occurrence cut the envelope in half and reported a parse
    /// failure from the middle of the file.
    const COMMENT_CLAIMS_ROWS: &[u8] = br#"(
    name: "MasterItemDefinitions",
    /* regenerated from the datasheet; the old layout put
    rows: first, before the key */
    schema: "ItemData",
    key: Some("item_id"),
    rows: [
        (item_id: "Sword"),
    ],
)"#;

    fn canonical() -> Header {
        header(CANONICAL).expect("the canonical envelope has a header")
    }

    #[test]
    fn a_comment_that_spells_the_rows_field_is_still_a_comment() {
        let read = header(COMMENT_CLAIMS_ROWS).expect("a comment is content, not structure");
        assert_eq!(read.name(), "MasterItemDefinitions");
        assert_eq!(read.schema(), "ItemData");
        assert_eq!(read.key(), Some("item_id"));
        assert_eq!(read, canonical());
    }

    #[test]
    fn a_block_comment_nests_and_does_not_end_the_prologue() {
        let source = br#"(
    /* outer /* inner
       rows: [] */ still the outer comment */
    name: "Tables",
    schema: "TableData",
    key: None,
    rows: [],
)"#;
        let read = header(source).expect("nested block comments are trivia");
        assert_eq!(read.name(), "Tables");
        assert_eq!(read.key(), None);
    }

    #[test]
    fn a_source_truncated_inside_its_prologue_says_so() {
        assert_eq!(
            header(&CANONICAL[..30]),
            Err(HeaderError::Truncated {
                expected: "`\"` closing a string literal"
            })
        );
        assert_eq!(
            header(
                br#"(
    name: "MasterItemDefinitions","#
            ),
            Err(HeaderError::Truncated { expected: "`)`" })
        );
    }

    #[test]
    fn a_source_that_is_not_an_envelope_is_malformed_not_truncated() {
        assert!(matches!(
            header(br#"[(item_id: "Sword")]"#),
            Err(HeaderError::Malformed { .. })
        ));
        assert!(matches!(
            header(br#"(name: "T", schema: "S", extra: 1, rows: [])"#),
            Err(HeaderError::Malformed {
                reason: "the envelope has no such field",
                ..
            })
        ));
        assert!(matches!(
            header(br#"(name: "T", schema: "", rows: [])"#),
            Err(HeaderError::Malformed { .. })
        ));
    }

    #[test]
    fn the_header_requires_rows_and_rejects_duplicate_fields_like_the_full_decoder() {
        assert_eq!(
            header(br#"(name: "T", schema: "S")"#),
            Err(HeaderError::Missing { field: "rows" })
        );
        for source in [
            &br#"(name: "T", name: "Other", schema: "S", rows: [])"#[..],
            &br#"(name: "T", schema: "S", schema: "Other", rows: [])"#[..],
            &br#"(name: "T", schema: "S", key: None, key: Some("id"), rows: [])"#[..],
        ] {
            assert!(matches!(header(source), Err(HeaderError::Malformed { .. })));
            assert!(
                decode_table_source_ron(source).is_err(),
                "the bounded header reader and the full envelope decoder must reject the same duplicate-field shape"
            );
        }
    }

    #[test]
    fn a_field_the_rows_hide_is_reported_as_missing_not_guessed_at() {
        assert_eq!(
            header(br#"(rows: [], name: "T", schema: "S")"#),
            Err(HeaderError::Missing { field: "name" })
        );
    }

    /// The bound is the contract, so it is asserted rather than left to be
    /// discovered: a file cut off inside its rows has an intact header and the
    /// read never looks that far. Compiling the source is what finds the cut.
    #[test]
    fn damage_past_the_prologue_is_outside_the_bound() {
        let cut_in_rows = br#"(
    name: "MasterItemDefinitions",
    schema: "ItemData",
    key: Some("item_id"),
    rows: [
        (item_id: "Swo"#;
        assert_eq!(
            header(cut_in_rows).expect("the header is intact"),
            canonical()
        );
    }

    #[test]
    fn string_literals_decode_the_way_the_envelope_writes_them() {
        let source = br##"(
    name: "Quote\"Inside",
    schema: r#"Raw\Schema"#,
    key: Some("tab\there"),
    rows: [],
)"##;
        let read = header(source).expect("escapes and raw strings are literals");
        assert_eq!(read.name(), "Quote\"Inside");
        assert_eq!(read.schema(), r"Raw\Schema");
        assert_eq!(read.key(), Some("tab\there"));
    }

    #[test]
    fn the_header_agrees_with_the_full_envelope_decode() {
        for source in [CANONICAL, COMMENT_CLAIMS_ROWS] {
            let read = header(source).expect("header");
            let envelope = decode_table_source_ron(source).expect("full decode");
            assert_eq!(read.name(), envelope.name());
            assert_eq!(read.schema(), envelope.schema());
            assert_eq!(read.key(), envelope.key());
        }
    }
}
