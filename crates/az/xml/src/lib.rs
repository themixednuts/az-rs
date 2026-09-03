//! Shared XML reader and visitor utilities.

use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt, io,
    path::{Path, PathBuf},
    str,
};

use quick_xml::{
    Reader, XmlVersion,
    errors::IllFormedError,
    escape::{EscapeError, resolve_predefined_entity},
    events::{BytesCData, BytesRef, BytesStart, BytesText, Event},
};
use smallvec::SmallVec;
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseError {
    #[error("XML asset is not UTF-8")]
    InvalidUtf8(#[from] str::Utf8Error),

    #[error("XML parser error: {0}")]
    Xml(#[from] quick_xml::Error),

    #[error("XML attribute error: {0}")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),

    #[error("unexpected closing element </{name}>")]
    UnexpectedEnd { name: String },

    #[error("mismatched closing element </{found}>; expected </{expected}>")]
    MismatchedEnd { expected: String, found: String },

    #[error("XML document ended before closing <{name}>")]
    UnclosedElement { name: String },

    #[error("XML document is empty")]
    EmptyDocument,

    #[error("XML document contains more than one root element")]
    MultipleRoots,
}

pub type XmlParseError = ParseError;

pub const XML_EXTENSION: &str = "xml";
pub const XML_EXTENSIONS: &[&str] = &[XML_EXTENSION];

/// Owned XML element tree that preserves unknown attributes, children, and
/// text for lossless legacy-format transforms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlElement {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub text: String,
    pub children: Vec<Self>,
}

impl XmlElement {
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Self> {
        self.children.iter().filter(move |child| child.name == name)
    }
}

/// Parse one XML document into an owned, loss-preserving element tree.
///
/// # Errors
///
/// Returns [`ParseError`] for malformed XML, mismatched or unclosed elements,
/// an empty document, or multiple document roots.
pub fn parse_tree(xml: &str) -> Result<XmlElement, ParseError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut stack = Vec::<XmlElement>::new();
    let mut root = None;

    loop {
        match reader.read_event()? {
            Event::Start(event) => stack.push(owned_element(&reader, &event)?),
            Event::Empty(event) => {
                let element = owned_element(&reader, &event)?;
                append_owned_element(&mut stack, &mut root, element)?;
            }
            Event::Text(event) => {
                if let Some(element) = stack.last_mut() {
                    element
                        .text
                        .push_str(&event.decode().map_err(quick_xml::Error::from)?);
                }
            }
            Event::CData(event) => {
                if let Some(element) = stack.last_mut() {
                    element
                        .text
                        .push_str(&event.decode().map_err(quick_xml::Error::from)?);
                }
            }
            Event::GeneralRef(event) => {
                if let Some(element) = stack.last_mut() {
                    element
                        .text
                        .push_str(&xml_general_reference_content(&event)?);
                }
            }
            Event::End(event) => {
                let found = reader
                    .decoder()
                    .decode(event.name().as_ref())
                    .map_err(quick_xml::Error::from)?
                    .into_owned();
                let element = stack.pop().ok_or_else(|| ParseError::UnexpectedEnd {
                    name: found.clone(),
                })?;
                if found != element.name {
                    return Err(ParseError::MismatchedEnd {
                        expected: element.name,
                        found,
                    });
                }
                append_owned_element(&mut stack, &mut root, element)?;
            }
            Event::Eof => break,
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) | Event::DocType(_) => {}
        }
    }

    if let Some(element) = stack.last() {
        return Err(ParseError::UnclosedElement {
            name: element.name.clone(),
        });
    }
    root.ok_or(ParseError::EmptyDocument)
}

fn owned_element(reader: &Reader<&[u8]>, event: &BytesStart<'_>) -> Result<XmlElement, ParseError> {
    let name = reader
        .decoder()
        .decode(event.name().as_ref())
        .map_err(quick_xml::Error::from)?
        .into_owned();
    let mut attributes = BTreeMap::new();
    for attribute in event.attributes() {
        let attribute = attribute?;
        let key = reader
            .decoder()
            .decode(attribute.key.as_ref())
            .map_err(quick_xml::Error::from)?
            .into_owned();
        let value = attribute
            .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())?
            .into_owned();
        attributes.insert(key, value);
    }
    Ok(XmlElement {
        name,
        attributes,
        text: String::new(),
        children: Vec::new(),
    })
}

fn append_owned_element(
    stack: &mut [XmlElement],
    root: &mut Option<XmlElement>,
    element: XmlElement,
) -> Result<(), ParseError> {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(element);
        return Ok(());
    }
    if root.replace(element).is_some() {
        return Err(ParseError::MultipleRoots);
    }
    Ok(())
}

/// Decodes the text content of a text event, resolving character and predefined
/// entity references.
///
/// # Errors
///
/// Returns [`quick_xml::Error::Escape`] if the text contains a malformed escape
/// or an entity reference the XML predefined set does not define.
pub fn xml_text_content<'a>(event: &BytesText<'a>) -> Result<Cow<'a, str>, quick_xml::Error> {
    event
        .xml_content(XmlVersion::default())
        .map_err(quick_xml::Error::from)
}

/// Decodes the raw content of a `CDATA` section.
///
/// # Errors
///
/// Returns [`quick_xml::Error::Escape`] if the section body cannot be decoded as
/// XML content.
pub fn xml_cdata_content<'a>(event: &BytesCData<'a>) -> Result<Cow<'a, str>, quick_xml::Error> {
    event
        .xml_content(XmlVersion::default())
        .map_err(quick_xml::Error::from)
}

/// Resolves a general entity reference to its replacement text.
///
/// Numeric character references (`&#38;`) are converted directly; named
/// references are looked up in the XML predefined entity set.
///
/// # Errors
///
/// Returns [`quick_xml::Error::Escape`] if the reference is a malformed numeric
/// character reference, if the reference name is not valid UTF-8 under the
/// reader's encoding, or if the name is not one of the five XML predefined
/// entities ([`EscapeError::UnrecognizedEntity`]).
pub fn xml_general_reference_content(
    event: &BytesRef<'_>,
) -> Result<Cow<'static, str>, quick_xml::Error> {
    if let Some(ch) = event.resolve_char_ref()? {
        return Ok(Cow::Owned(ch.to_string()));
    }

    let reference = event.decode().map_err(quick_xml::Error::from)?;
    let Some(value) = resolve_predefined_entity(&reference) else {
        return Err(quick_xml::Error::from(EscapeError::UnrecognizedEntity(
            0..event.len(),
            reference.into_owned(),
        )));
    };
    Ok(Cow::Borrowed(value))
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum XmlInspectionError {
    #[error("failed to read {path:?}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to parse XML asset {path:?}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
}

/// Parsed XML document statistics.
///
/// The parser validates the complete XML stream and records structure without
/// materializing a DOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XmlDocument {
    stats: XmlStats,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct XmlStats {
    pub elements: usize,
    pub attributes: usize,
    pub text_nodes: usize,
    pub cdata_nodes: usize,
    pub comments: usize,
    pub processing_instructions: usize,
    pub declarations: usize,
    pub doctypes: usize,
    pub max_depth: usize,
    pub recovered_unmatched_ends: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XmlElementRef<'a> {
    name: &'a [u8],
    attributes: usize,
    depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XmlAttributeRef<'a> {
    name: &'a [u8],
    value: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XmlTextRef<'a> {
    raw: &'a [u8],
}

pub trait XmlVisitor {
    fn start_element(&mut self, _element: XmlElementRef<'_>) {}

    fn empty_element(&mut self, _element: XmlElementRef<'_>) {}

    fn end_element(&mut self, _name: &[u8]) {}

    fn attribute(&mut self, _attribute: XmlAttributeRef<'_>) {}

    fn text(&mut self, _text: XmlTextRef<'_>) {}

    fn general_reference(&mut self, _reference: XmlTextRef<'_>) {}

    fn cdata(&mut self, _text: XmlTextRef<'_>) {}

    fn comment(&mut self, _text: XmlTextRef<'_>) {}

    fn processing_instruction(&mut self, _text: XmlTextRef<'_>) {}

    fn declaration(&mut self, _text: XmlTextRef<'_>) {}

    fn doctype(&mut self, _text: XmlTextRef<'_>) {}

    fn unmatched_end(&mut self, _name: &[u8]) {}

    fn stats(&mut self, _stats: XmlStats) {}
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct XmlSummary {
    pub stats: XmlStats,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct XmlTotals {
    pub files: usize,
    pub stats: XmlStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlFileSummary {
    pub source: String,
    pub summary: XmlSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct XmlInspection {
    pub rows: Vec<XmlFileSummary>,
    pub totals: XmlTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct XmlInspectionReport<'a> {
    inspection: &'a XmlInspection,
    limit: usize,
}

impl XmlVisitor for XmlSummary {
    fn stats(&mut self, stats: XmlStats) {
        self.stats = stats;
    }
}

impl fmt::Display for XmlSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} elements, depth {}",
            self.stats.elements, self.stats.max_depth
        )
    }
}

impl XmlStats {
    pub fn add_assign(&mut self, other: Self) {
        self.elements += other.elements;
        self.attributes += other.attributes;
        self.text_nodes += other.text_nodes;
        self.cdata_nodes += other.cdata_nodes;
        self.comments += other.comments;
        self.processing_instructions += other.processing_instructions;
        self.declarations += other.declarations;
        self.doctypes += other.doctypes;
        self.max_depth = self.max_depth.max(other.max_depth);
        self.recovered_unmatched_ends += other.recovered_unmatched_ends;
    }
}

impl XmlTotals {
    pub fn add_summary(&mut self, summary: XmlSummary) {
        self.files += 1;
        self.stats.add_assign(summary.stats);
    }
}

impl XmlInspection {
    pub fn add_file_summary(&mut self, row: XmlFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> XmlInspectionReport<'_> {
        XmlInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for XmlTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  elements: {}", self.stats.elements)?;
        writeln!(f, "  attributes: {}", self.stats.attributes)?;
        writeln!(f, "  text nodes: {}", self.stats.text_nodes)?;
        writeln!(f, "  cdata nodes: {}", self.stats.cdata_nodes)?;
        writeln!(f, "  comments: {}", self.stats.comments)?;
        writeln!(
            f,
            "  processing instructions: {}",
            self.stats.processing_instructions
        )?;
        writeln!(f, "  declarations: {}", self.stats.declarations)?;
        writeln!(f, "  doctypes: {}", self.stats.doctypes)?;
        writeln!(f, "  max depth: {}", self.stats.max_depth)?;
        writeln!(
            f,
            "  recovered unmatched end tags: {}",
            self.stats.recovered_unmatched_ends
        )?;
        Ok(())
    }
}

impl fmt::Display for XmlInspectionReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.limit > 0 {
            for row in self.inspection.rows.iter().take(self.limit) {
                writeln!(f, "{}: {}", row.source, row.summary)?;
            }

            if self.inspection.rows.len() > self.limit {
                writeln!(
                    f,
                    "... {} more files",
                    self.inspection.rows.len() - self.limit
                )?;
            }
        }

        write!(f, "{}", self.inspection.totals)
    }
}

impl XmlDocument {
    /// Parses `bytes` as an XML document. `_path` is accepted for call-site
    /// symmetry with the other `*_path` helpers and is not read.
    ///
    /// # Errors
    ///
    /// Returns any [`ParseError`] [`Self::parse_bytes`] returns.
    pub fn parse_path(_path: impl AsRef<Path>, bytes: &[u8]) -> Result<Self, ParseError> {
        Self::parse_bytes(bytes)
    }

    /// Parses `bytes` and records document statistics without building a DOM.
    ///
    /// # Errors
    ///
    /// Returns any [`ParseError`] [`visit_bytes`] returns: [`ParseError::InvalidUtf8`]
    /// for non-UTF-8 input, or the structural and parser errors raised while
    /// walking the document.
    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, ParseError> {
        let mut collector = StatsCollector::default();
        visit_bytes(bytes, &mut collector)?;
        Ok(Self {
            stats: collector.stats,
        })
    }

    #[must_use]
    #[inline]
    pub const fn stats(&self) -> XmlStats {
        self.stats
    }
}

impl<'a> XmlElementRef<'a> {
    #[must_use]
    #[inline]
    pub const fn name(&self) -> &'a [u8] {
        self.name
    }

    #[must_use]
    #[inline]
    pub const fn attributes(&self) -> usize {
        self.attributes
    }

    #[must_use]
    #[inline]
    pub const fn depth(&self) -> usize {
        self.depth
    }
}

impl<'a> XmlAttributeRef<'a> {
    #[must_use]
    #[inline]
    pub const fn name(&self) -> &'a [u8] {
        self.name
    }

    #[must_use]
    #[inline]
    pub const fn value(&self) -> &'a [u8] {
        self.value
    }
}

impl<'a> XmlTextRef<'a> {
    #[must_use]
    #[inline]
    pub const fn raw(&self) -> &'a [u8] {
        self.raw
    }
}

/// Walks `bytes` with `visitor`. `_path` is accepted for call-site symmetry and
/// is not read.
///
/// # Errors
///
/// Returns any [`ParseError`] [`visit_bytes`] returns.
pub fn visit_path(
    _path: impl AsRef<Path>,
    bytes: &[u8],
    visitor: &mut impl XmlVisitor,
) -> Result<(), ParseError> {
    visit_bytes(bytes, visitor)
}

/// Walks `bytes` and collects an [`XmlSummary`] of element and attribute names.
///
/// # Errors
///
/// Returns any [`ParseError`] [`visit_bytes`] returns.
pub fn summarize_xml_path(_path: impl AsRef<Path>, bytes: &[u8]) -> Result<XmlSummary, ParseError> {
    let mut summary = XmlSummary::default();
    visit_bytes(bytes, &mut summary)?;
    Ok(summary)
}

/// Summarizes `bytes` and labels the result with `path` for reporting.
///
/// # Errors
///
/// Returns any [`ParseError`] [`summarize_xml_path`] returns.
pub fn inspect_xml_path(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<XmlFileSummary, ParseError> {
    let path = path.as_ref();
    Ok(XmlFileSummary {
        source: path.display().to_string(),
        summary: summarize_xml_path(path, bytes)?,
    })
}

/// Reads the file at `path` from disk and summarizes it.
///
/// # Errors
///
/// Returns [`XmlInspectionError::Read`] if the file cannot be read (missing,
/// unreadable, or a directory), or [`XmlInspectionError::Parse`] if its contents
/// are not well-formed XML.
pub fn inspect_xml_file(path: impl AsRef<Path>) -> Result<XmlFileSummary, XmlInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| XmlInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_xml_path(path, &bytes).map_err(|source| XmlInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Summarizes every path in `paths`, accumulating the per-file rows and totals.
///
/// # Errors
///
/// Stops at the first failing path and returns whatever [`inspect_xml_file`]
/// returned for it: [`XmlInspectionError::Read`] or [`XmlInspectionError::Parse`].
pub fn inspect_xml_files<I, P>(paths: I) -> Result<XmlInspection, XmlInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = XmlInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_xml_file(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub fn is_xml_extension(extension: &str) -> bool {
    XML_EXTENSIONS
        .iter()
        .any(|expected| extension.eq_ignore_ascii_case(expected))
}

#[must_use]
pub fn is_xml_name(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_xml_extension)
}

#[must_use]
pub fn is_xml_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_xml_extension)
}

#[must_use]
pub fn is_xml_backed_extension(extension: &str) -> bool {
    is_xml_extension(extension)
}

#[must_use]
pub fn is_xml_backed_name(path: &str) -> bool {
    is_xml_name(path)
}

#[must_use]
pub fn is_xml_backed_path(path: &Path) -> bool {
    is_xml_path(path)
}

/// Strips a leading UTF-8 BOM and walks the document with `visitor`.
///
/// # Errors
///
/// Returns [`ParseError::InvalidUtf8`] if `bytes` is not valid UTF-8 after the
/// BOM, plus any [`ParseError`] [`visit_str`] returns.
pub fn visit_bytes(bytes: &[u8], visitor: &mut impl XmlVisitor) -> Result<(), ParseError> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    visit_str(str::from_utf8(bytes)?, visitor)
}

/// Walks `xml` end to end, driving `visitor` and accumulating [`XmlStats`].
///
/// # Errors
///
/// Returns [`ParseError::Xml`] for a parser-level failure that is not a
/// recoverable unmatched end tag; [`ParseError::Attribute`] for a malformed
/// attribute; [`ParseError::MismatchedEnd`] when a closing tag names an element
/// that is open but not innermost; and [`ParseError::UnclosedElement`] if the
/// document ends with elements still open. Text and entity decoding failures
/// surface as [`ParseError::Xml`].
pub fn visit_str(xml: &str, visitor: &mut impl XmlVisitor) -> Result<(), ParseError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;
    let mut stats = XmlStats::default();
    let mut depth = 0usize;
    let mut stack: Vec<ElementName> = Vec::new();

    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(quick_xml::Error::IllFormed(IllFormedError::UnmatchedEndTag(name))) => {
                close_element(
                    &reader,
                    name.as_bytes(),
                    &mut stack,
                    &mut depth,
                    &mut stats,
                    visitor,
                )?;
                continue;
            }
            Err(err) => return Err(ParseError::Xml(err)),
        };

        if matches!(
            visit_event(&reader, event, &mut stack, &mut depth, &mut stats, visitor)?,
            Flow::Break
        ) {
            break;
        }
    }

    if let Some(name) = stack.last() {
        return Err(ParseError::UnclosedElement {
            name: element_name(&reader, name.as_slice()),
        });
    }

    visitor.stats(stats);
    Ok(())
}

/// Whether [`visit_str`]'s read loop should keep pulling events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Break,
}

/// Dispatches one parsed event to `visitor`, folding it into `stats` and
/// maintaining the open-element `stack` and `depth`.
///
/// Returns [`Flow::Break`] at end of input and [`Flow::Continue`] otherwise.
fn visit_event(
    reader: &Reader<&[u8]>,
    event: Event<'_>,
    stack: &mut Vec<ElementName>,
    depth: &mut usize,
    stats: &mut XmlStats,
    visitor: &mut impl XmlVisitor,
) -> Result<Flow, ParseError> {
    match event {
        Event::Start(event) => {
            let (next_depth, attributes) = record_element(reader, &event, *depth, stats, visitor)?;
            let name = event.name();
            stack.push(ElementName::from_slice(name.as_ref()));
            visitor.start_element(XmlElementRef {
                name: name.as_ref(),
                attributes,
                depth: next_depth,
            });
            *depth = next_depth;
        }
        Event::Empty(event) => {
            let (next_depth, attributes) = record_element(reader, &event, *depth, stats, visitor)?;
            let name = event.name();
            visitor.empty_element(XmlElementRef {
                name: name.as_ref(),
                attributes,
                depth: next_depth,
            });
        }
        Event::End(event) => {
            let name = event.name();
            close_element(reader, name.as_ref(), stack, depth, stats, visitor)?;
        }
        Event::Text(event) => {
            xml_text_content(&event)?;
            stats.text_nodes += 1;
            visitor.text(XmlTextRef {
                raw: event.as_ref(),
            });
        }
        Event::GeneralRef(event) => {
            xml_general_reference_content(&event)?;
            visitor.general_reference(XmlTextRef {
                raw: event.as_ref(),
            });
        }
        Event::CData(event) => {
            stats.cdata_nodes += 1;
            visitor.cdata(XmlTextRef {
                raw: event.as_ref(),
            });
        }
        Event::Comment(event) => {
            stats.comments += 1;
            visitor.comment(XmlTextRef {
                raw: event.as_ref(),
            });
        }
        Event::PI(event) => {
            stats.processing_instructions += 1;
            visitor.processing_instruction(XmlTextRef {
                raw: event.as_ref(),
            });
        }
        Event::Decl(event) => {
            stats.declarations += 1;
            visitor.declaration(XmlTextRef {
                raw: event.as_ref(),
            });
        }
        Event::DocType(event) => {
            stats.doctypes += 1;
            visitor.doctype(XmlTextRef {
                raw: event.as_ref(),
            });
        }
        Event::Eof => return Ok(Flow::Break),
    }

    Ok(Flow::Continue)
}

fn visit_attributes(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    visitor: &mut impl XmlVisitor,
) -> Result<usize, ParseError> {
    let mut count = 0usize;
    for attribute in event.attributes() {
        let attribute = attribute?;
        attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::default(), reader.decoder())?;
        visitor.attribute(XmlAttributeRef {
            name: attribute.key.as_ref(),
            value: attribute.value.as_ref(),
        });
        count += 1;
    }
    Ok(count)
}

/// Counts an opening or empty element's attributes and folds it into `stats`.
///
/// Returns the element's own depth (one deeper than the enclosing `depth`) and
/// its attribute count. The caller decides whether the element also pushes onto
/// the open-element stack.
fn record_element(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    depth: usize,
    stats: &mut XmlStats,
    visitor: &mut impl XmlVisitor,
) -> Result<(usize, usize), ParseError> {
    let next_depth = depth + 1;
    let attributes = visit_attributes(reader, event, visitor)?;
    stats.elements += 1;
    stats.attributes += attributes;
    stats.max_depth = stats.max_depth.max(next_depth);
    Ok((next_depth, attributes))
}

/// Closes the element named `found`.
///
/// Shared by the well-formed `Event::End` path and the recovery path for
/// `IllFormedError::UnmatchedEndTag`, which see the same three cases: the tag
/// closes the innermost open element, it closes an outer one (an error), or it
/// closes nothing at all (recorded and reported to the visitor).
fn close_element(
    reader: &Reader<&[u8]>,
    found: &[u8],
    stack: &mut Vec<ElementName>,
    depth: &mut usize,
    stats: &mut XmlStats,
    visitor: &mut impl XmlVisitor,
) -> Result<(), ParseError> {
    match stack.last() {
        Some(expected) if expected.as_slice() == found => {
            visitor.end_element(found);
            stack.pop();
            *depth = depth.saturating_sub(1);
            Ok(())
        }
        Some(expected) if stack.iter().any(|open| open.as_slice() == found) => {
            Err(ParseError::MismatchedEnd {
                expected: element_name(reader, expected.as_slice()),
                found: element_name(reader, found),
            })
        }
        Some(_) | None => {
            stats.recovered_unmatched_ends += 1;
            visitor.unmatched_end(found);
            Ok(())
        }
    }
}

#[derive(Default)]
struct StatsCollector {
    stats: XmlStats,
}

impl XmlVisitor for StatsCollector {
    fn stats(&mut self, stats: XmlStats) {
        self.stats = stats;
    }
}

fn element_name(reader: &Reader<&[u8]>, raw: &[u8]) -> String {
    reader.decoder().decode(raw).map_or_else(
        |_| String::from_utf8_lossy(raw).into_owned(),
        Cow::into_owned,
    )
}

type ElementName = SmallVec<[u8; 32]>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xml_summary_without_dom() {
        let document = XmlDocument::parse_path(
            "levels/foo/leveldata.xml",
            br#"<?xml version="1.0"?><Level><SurfaceTypes><SurfaceType name="grass"/></SurfaceTypes><!--ok--></Level>"#,
        )
        .expect("parse");

        let stats = document.stats();
        assert_eq!(stats.elements, 3);
        assert_eq!(stats.attributes, 1);
        assert_eq!(stats.comments, 1);
        assert_eq!(stats.declarations, 1);
        assert_eq!(stats.max_depth, 3);
        assert_eq!(stats.recovered_unmatched_ends, 0);
    }

    #[test]
    fn owned_tree_preserves_unknown_content() {
        let root = parse_tree(
            r#"<Root custom="&amp;"><Child x="1"> value <![CDATA[raw]]></Child></Root>"#,
        )
        .expect("parse owned tree");

        assert_eq!(root.name, "Root");
        assert_eq!(root.attribute("custom"), Some("&"));
        let child = root.children_named("Child").next().expect("child");
        assert_eq!(child.attribute("x"), Some("1"));
        assert_eq!(child.text, " value raw");
    }

    #[test]
    fn owned_tree_rejects_multiple_roots() {
        let error = parse_tree("<first/><second/>").expect_err("multiple roots");
        assert!(matches!(error, ParseError::MultipleRoots));
    }

    #[test]
    fn visits_borrowed_element_and_attribute_refs() {
        #[derive(Default)]
        struct Counter {
            starts: usize,
            attrs: usize,
            text: usize,
        }

        impl XmlVisitor for Counter {
            fn start_element(&mut self, element: XmlElementRef<'_>) {
                assert!(!element.name().is_empty());
                self.starts += 1;
            }

            fn attribute(&mut self, attribute: XmlAttributeRef<'_>) {
                assert!(!attribute.name().is_empty());
                assert!(!attribute.value().is_empty());
                self.attrs += 1;
            }

            fn text(&mut self, text: XmlTextRef<'_>) {
                assert!(!text.raw().is_empty());
                self.text += 1;
            }
        }

        let mut counter = Counter::default();
        visit_str(
            r#"<root key="value"><child>text</child></root>"#,
            &mut counter,
        )
        .expect("visit");

        assert_eq!(counter.starts, 2);
        assert_eq!(counter.attrs, 1);
        assert_eq!(counter.text, 1);
    }

    #[test]
    fn rejects_malformed_xml() {
        let err = XmlDocument::parse_bytes(br"<root><child></root>").expect_err("invalid XML");

        assert!(matches!(err, ParseError::MismatchedEnd { .. }));
    }

    #[test]
    fn recovers_unmatched_legacy_end_tags() {
        let document =
            XmlDocument::parse_bytes(br"<root><child/></ghost><next/></root>").expect("parse");

        assert_eq!(document.stats().elements, 3);
        assert_eq!(document.stats().recovered_unmatched_ends, 1);
    }

    #[test]
    fn parses_bom_prefixed_xml() {
        let document =
            XmlDocument::parse_bytes(b"\xEF\xBB\xBF<root/>").expect("parse BOM prefixed XML");

        assert_eq!(document.stats().elements, 1);
    }

    #[test]
    fn summarizes_xml_assets_and_paths() {
        let path = "levels/foo/leveldata.xml";
        let bytes = br"<Level><A/></Level>";
        let summary = summarize_xml_path(path, bytes).expect("summarize xml");
        let mut totals = XmlTotals::default();
        totals.add_summary(summary);

        assert_eq!(summary.stats.elements, 2);
        assert_eq!(summary.stats.max_depth, 2);
        assert_eq!(summary.to_string(), "2 elements, depth 2");
        assert_eq!(totals.files, 1);
        assert_eq!(
            totals.to_string(),
            "  files: 1\n  elements: 2\n  attributes: 0\n  text nodes: 0\n  cdata nodes: 0\n  comments: 0\n  processing instructions: 0\n  declarations: 0\n  doctypes: 0\n  max depth: 2\n  recovered unmatched end tags: 0\n"
        );

        let mut inspection = XmlInspection::default();
        inspection.add_file_summary(inspect_xml_path(path, bytes).expect("inspect xml"));
        assert_eq!(
            inspection.report(20).to_string(),
            "levels/foo/leveldata.xml: 2 elements, depth 2\n  files: 1\n  elements: 2\n  attributes: 0\n  text nodes: 0\n  cdata nodes: 0\n  comments: 0\n  processing instructions: 0\n  declarations: 0\n  doctypes: 0\n  max depth: 2\n  recovered unmatched end tags: 0\n"
        );

        assert!(is_xml_name("foo.XML"));
        assert!(!is_xml_path(Path::new("foo.cdf")));
        assert!(!is_xml_path(Path::new("foo.chrparams")));
        assert!(!is_xml_name("foo.json"));
    }

    #[test]
    fn inspect_xml_files_aggregates_file_results() {
        let path = std::env::temp_dir().join(format!(
            "az-xml-plain-{}-{}.xml",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, br"<root><child/></root>").expect("write fixture");

        let inspection = inspect_xml_files([&path]).expect("inspect fixture");

        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.stats.elements, 2);
        assert_eq!(inspection.totals.stats.max_depth, 2);
        assert_eq!(inspection.rows.len(), 1);

        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn stats_accumulate_without_losing_max_depth() {
        let mut stats = XmlStats {
            elements: 2,
            max_depth: 4,
            ..XmlStats::default()
        };
        stats.add_assign(XmlStats {
            elements: 3,
            max_depth: 2,
            ..XmlStats::default()
        });

        assert_eq!(stats.elements, 5);
        assert_eq!(stats.max_depth, 4);
    }
}
