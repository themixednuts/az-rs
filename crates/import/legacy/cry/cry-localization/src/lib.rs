//! Cry/Lumberyard localization XML parser.

pub mod builder;
pub mod source_transform;

pub use source_transform::{
    LocalizationSource, LocalizationSourceAttribute, LocalizationSourceEntry,
    LocalizationSourceTransform, LocalizationSourceTransformError, localization_source_path,
};

use az_asset_builder::{
    BuildRuleRegistration, ProductFormat, ProductFormatRegistration, SourceFormat,
    SourceSchemaRegistration, product_format_id, source_schema_type,
};
use az_core::{AssetData, AssetTypeRegistration, AzRtti, AzTypeInfo};
use az_filesystem::normalize_source_path;
use std::{
    borrow::Cow,
    fmt, io,
    path::{Path, PathBuf},
    str,
};

use quick_xml::{
    Reader,
    escape::{EscapeError, resolve_predefined_entity},
    events::{BytesRef, BytesStart, Event},
};
use smallvec::SmallVec;
use thiserror::Error;
use uuid::{Uuid, uuid};

pub type LocalizationAttributes<'a> = SmallVec<[LocalizationAttribute<'a>; 8]>;
pub const LOCALIZATION_SOURCE_EXTENSION: &str = "loc.xml";
pub const LOCALIZATION_LEGACY_EXTENSION: &str = "loc";

pub struct LocalizationAssetData;

impl AzTypeInfo for LocalizationAssetData {
    const NAME: &'static str = "Cry::LocalizationAsset";
    const TYPE_ID: Uuid = uuid!("def554c2-f6cd-47a1-5921-9c6e67577b0d");
}

impl AzRtti for LocalizationAssetData {}

impl AssetData for LocalizationAssetData {
    const STABLE_NAME: &'static str = "azoth.compat.cry.localization";
}

#[derive(SourceFormat)]
#[source(schema = "azoth.compat.cry.LocalizationSource", ext = "loc.ron")]
pub struct LocalizationSourceFormat;

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.compat.cry.localization",
    version = 1,
    asset = LocalizationAssetData
)]
pub struct LocalizationProductFormat;

pub mod ids {
    use super::{AssetData, LocalizationAssetData};
    use az_core::AssetType;

    /// Cry/Lumberyard localization table source/product.
    pub const LOCALIZATION: AssetType = LocalizationAssetData::ASSET_TYPE;
}

pub mod source_schemas {
    use super::{LocalizationSourceFormat, source_schema_type};
    use az_asset_builder::SourceSchemaType;

    pub const LOCALIZATION: SourceSchemaType = source_schema_type::<LocalizationSourceFormat>();
}

pub mod product_formats {
    use super::{LocalizationProductFormat, product_format_id};
    use az_asset_builder::ProductFormatId;

    pub const CRY_LOCALIZATION: ProductFormatId = product_format_id::<LocalizationProductFormat>();
}

/// The asset types this crate owns, for a host contribution to register.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 1] {
    [AssetTypeRegistration::for_asset::<LocalizationAssetData>()
        .with_owner("cry-localization::builder")]
}

/// The product formats this crate owns, for a host contribution to register.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 1] {
    [ProductFormatRegistration::for_format::<
        LocalizationProductFormat,
    >()]
}

/// The source schemas this crate owns, for a host contribution to register.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 1] {
    [
        SourceSchemaRegistration::for_source::<LocalizationSourceFormat>()
            .with_category("Cry/Lumberyard Compatibility")
            .with_import_file("localization", &["loc.ron"]),
    ]
}

/// The build rules this crate owns, for a host contribution to register.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 1] {
    [BuildRuleRegistration::new(
        builder::NAME,
        builder::ID,
        builder::desc,
    )]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<AssetTypeRegistration>()
        .register_many(asset_types());
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseError {
    #[error("localization XML is not UTF-8")]
    InvalidUtf8(#[from] str::Utf8Error),

    #[error("XML parser error: {0}")]
    Xml(#[from] quick_xml::Error),

    #[error("XML attribute error: {0}")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),

    #[error("expected <resources> root element")]
    MissingResourcesRoot,

    #[error("unexpected root element <{name}>")]
    UnexpectedRoot { name: String },

    #[error("unexpected <{name}> inside <resources>")]
    UnexpectedElement { name: String },

    #[error("<string> entry is missing required key attribute")]
    MissingKey,

    #[error("nested <string> entries are not valid")]
    NestedString,

    #[error("non-whitespace text outside <string> entry")]
    TextOutsideString,
}

pub type LocalizationParseError = ParseError;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LocalizationInspectionError {
    #[error("read {path:?}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse localization XML {path:?}: {source}")]
    Parse { path: PathBuf, source: ParseError },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizationDocument<'a> {
    entries: Vec<LocalizationEntry<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalizationEntry<'a> {
    String(LocalizationString<'a>),
    Nil(LocalizationNil<'a>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizationString<'a> {
    key: Cow<'a, str>,
    value: Cow<'a, str>,
    attributes: LocalizationAttributes<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizationNil<'a> {
    value: Cow<'a, str>,
    attributes: LocalizationAttributes<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizationAttribute<'a> {
    name: Cow<'a, str>,
    value: Cow<'a, str>,
}

pub trait LocalizationVisitor<'a> {
    fn entry(&mut self, entry: LocalizationEntry<'a>);
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LocalizationSummary {
    pub entries: usize,
    pub nil_entries: usize,
    pub attributes: usize,
    pub empty_values: usize,
    pub text_bytes: usize,
}

impl<'a> LocalizationVisitor<'a> for LocalizationSummary {
    fn entry(&mut self, entry: LocalizationEntry<'a>) {
        self.entries += 1;
        if entry.is_nil() {
            self.nil_entries += 1;
        }
        self.attributes += entry.attributes().len();
        self.text_bytes += entry.value().len();
        if entry.value().is_empty() {
            self.empty_values += 1;
        }
    }
}

impl fmt::Display for LocalizationSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} strings, {} metadata attributes",
            self.entries, self.attributes
        )
    }
}

/// Counts the entries and metadata attributes in a localization XML document.
///
/// # Errors
///
/// Returns any error [`visit_bytes`] returns — [`ParseError::InvalidUtf8`] for
/// non-UTF-8 bytes, [`ParseError::Xml`] or [`ParseError::Attribute`] for a
/// malformed document, and the structural variants
/// [`ParseError::UnexpectedRoot`], [`ParseError::UnexpectedElement`],
/// [`ParseError::NestedString`], [`ParseError::TextOutsideString`] and
/// [`ParseError::MissingKey`].
pub fn summarize_localization_bytes(bytes: &[u8]) -> Result<LocalizationSummary, ParseError> {
    let mut summary = LocalizationSummary::default();
    visit_bytes(bytes, &mut summary)?;
    Ok(summary)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LocalizationTotals {
    pub files: usize,
    pub entries: usize,
    pub nil_entries: usize,
    pub attributes: usize,
    pub empty_values: usize,
    pub text_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizationFileSummary {
    pub source: String,
    pub summary: LocalizationSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LocalizationInspection {
    pub rows: Vec<LocalizationFileSummary>,
    pub totals: LocalizationTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct LocalizationInspectionReport<'a> {
    inspection: &'a LocalizationInspection,
    limit: usize,
}

impl LocalizationTotals {
    pub const fn add_summary(&mut self, summary: LocalizationSummary) {
        self.files += 1;
        self.entries += summary.entries;
        self.nil_entries += summary.nil_entries;
        self.attributes += summary.attributes;
        self.empty_values += summary.empty_values;
        self.text_bytes += summary.text_bytes;
    }
}

impl LocalizationInspection {
    pub fn add_file_summary(&mut self, row: LocalizationFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> LocalizationInspectionReport<'_> {
        LocalizationInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for LocalizationTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  strings: {}", self.entries)?;
        writeln!(f, "  nil strings: {}", self.nil_entries)?;
        writeln!(f, "  metadata attributes: {}", self.attributes)?;
        writeln!(f, "  empty strings: {}", self.empty_values)?;
        writeln!(f, "  text bytes: {}", self.text_bytes)
    }
}

impl fmt::Display for LocalizationInspectionReport<'_> {
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

/// Summarises one localization document's bytes, labelling the row with
/// `path`.
///
/// `path` is only the display label; it is not read from disk.
///
/// # Errors
///
/// Returns any error [`summarize_localization_bytes`] returns — the
/// [`ParseError`] variants for non-UTF-8 bytes, malformed XML, or a document
/// that violates the `<resources>`/`<string>` structure.
pub fn inspect_localization_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<LocalizationFileSummary, ParseError> {
    Ok(LocalizationFileSummary {
        source: path.as_ref().display().to_string(),
        summary: summarize_localization_bytes(bytes)?,
    })
}

/// Reads a localization document from disk and summarises it.
///
/// # Errors
///
/// Returns [`LocalizationInspectionError::Read`] if `path` cannot be read
/// (missing file, permissions), or [`LocalizationInspectionError::Parse`]
/// wrapping the [`ParseError`] from a malformed document. Both variants carry
/// the offending path.
pub fn inspect_localization_path(
    path: impl AsRef<Path>,
) -> Result<LocalizationFileSummary, LocalizationInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| LocalizationInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_localization_file(path, &bytes).map_err(|source| LocalizationInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Reads and summarises every localization document in `paths`, accumulating
/// totals.
///
/// Stops at the first failing path; earlier rows are discarded with it.
///
/// # Errors
///
/// Returns any error [`inspect_localization_path`] returns for the first path
/// that fails — [`LocalizationInspectionError::Read`] for an unreadable file,
/// or [`LocalizationInspectionError::Parse`] for a malformed document.
pub fn inspect_localization_files<I, P>(
    paths: I,
) -> Result<LocalizationInspection, LocalizationInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = LocalizationInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_localization_path(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub fn is_localization_source_name(name: &str) -> bool {
    let normalized = normalize_source_path(name);
    let path = Path::new(&normalized);
    let Some(extension) = path.extension() else {
        return false;
    };
    if extension.eq_ignore_ascii_case("loc") {
        return true;
    }
    // The other spelling is `.loc.xml`, so check the stem's own extension.
    extension.eq_ignore_ascii_case("xml")
        && path
            .file_stem()
            .and_then(|stem| Path::new(stem).extension())
            .is_some_and(|stem_extension| stem_extension.eq_ignore_ascii_case("loc"))
}

#[must_use]
pub fn is_localization_source_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    is_localization_source_name(path.as_ref())
}

impl<'a> LocalizationDocument<'a> {
    #[must_use]
    #[inline]
    pub fn entries(&self) -> &[LocalizationEntry<'a>] {
        &self.entries
    }

    /// Borrowing iterator over the parsed entries, in document order.
    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, LocalizationEntry<'a>> {
        self.entries.iter()
    }

    #[must_use]
    #[inline]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Parses a localization XML document from raw bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::InvalidUtf8`] if `bytes` is not valid UTF-8, plus
    /// any error [`Self::parse_str`] returns.
    pub fn parse_bytes(bytes: &'a [u8]) -> Result<Self, ParseError> {
        Self::parse_str(str::from_utf8(bytes)?)
    }

    /// Parses a localization XML document from an already-decoded string.
    ///
    /// # Errors
    ///
    /// Returns any error [`visit_str`] returns — [`ParseError::Xml`] or
    /// [`ParseError::Attribute`] for a malformed document,
    /// [`ParseError::UnexpectedRoot`] when the root is not `<resources>`,
    /// [`ParseError::UnexpectedElement`] for a foreign element,
    /// [`ParseError::NestedString`] for a nested `<string>`,
    /// [`ParseError::TextOutsideString`] for stray text, and
    /// [`ParseError::MissingKey`] for a `<string>` with no key attribute.
    pub fn parse_str(xml: &'a str) -> Result<Self, ParseError> {
        let mut collector = EntryCollector::default();
        visit_str(xml, &mut collector)?;
        Ok(Self {
            entries: collector.entries,
        })
    }
}

impl<'a> IntoIterator for LocalizationDocument<'a> {
    type IntoIter = std::vec::IntoIter<LocalizationEntry<'a>>;
    type Item = LocalizationEntry<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'doc, 'a> IntoIterator for &'doc LocalizationDocument<'a> {
    type IntoIter = std::slice::Iter<'doc, LocalizationEntry<'a>>;
    type Item = &'doc LocalizationEntry<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl<'a> LocalizationEntry<'a> {
    #[must_use]
    #[inline]
    pub const fn string(
        key: Cow<'a, str>,
        value: Cow<'a, str>,
        attributes: LocalizationAttributes<'a>,
    ) -> Self {
        Self::String(LocalizationString::new(key, value, attributes))
    }

    #[must_use]
    #[inline]
    pub const fn nil(value: Cow<'a, str>, attributes: LocalizationAttributes<'a>) -> Self {
        Self::Nil(LocalizationNil::new(value, attributes))
    }

    #[must_use]
    #[inline]
    pub const fn as_string(&self) -> Option<&LocalizationString<'a>> {
        match self {
            Self::String(entry) => Some(entry),
            Self::Nil(_) => None,
        }
    }

    #[must_use]
    #[inline]
    pub const fn as_nil(&self) -> Option<&LocalizationNil<'a>> {
        match self {
            Self::String(_) => None,
            Self::Nil(entry) => Some(entry),
        }
    }

    #[must_use]
    #[inline]
    pub const fn is_nil(&self) -> bool {
        matches!(self, Self::Nil(_))
    }

    #[must_use]
    #[inline]
    pub fn key(&self) -> Option<&str> {
        self.as_string().map(LocalizationString::key)
    }

    #[must_use]
    #[inline]
    pub fn value(&self) -> &str {
        match self {
            Self::String(entry) => entry.value(),
            Self::Nil(entry) => entry.value(),
        }
    }

    #[must_use]
    #[inline]
    pub fn attributes(&self) -> &[LocalizationAttribute<'a>] {
        match self {
            Self::String(entry) => entry.attributes(),
            Self::Nil(entry) => entry.attributes(),
        }
    }

    #[must_use]
    #[inline]
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes()
            .iter()
            .find(|attribute| attribute.name == name)
            .map(LocalizationAttribute::value)
    }
}

impl<'a> LocalizationString<'a> {
    #[must_use]
    #[inline]
    pub const fn new(
        key: Cow<'a, str>,
        value: Cow<'a, str>,
        attributes: LocalizationAttributes<'a>,
    ) -> Self {
        Self {
            key,
            value,
            attributes,
        }
    }

    #[must_use]
    #[inline]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    #[inline]
    pub const fn key_cow(&self) -> &Cow<'a, str> {
        &self.key
    }

    #[must_use]
    #[inline]
    pub fn value(&self) -> &str {
        &self.value
    }

    #[must_use]
    #[inline]
    pub const fn value_cow(&self) -> &Cow<'a, str> {
        &self.value
    }

    #[must_use]
    #[inline]
    pub fn attributes(&self) -> &[LocalizationAttribute<'a>] {
        &self.attributes
    }

    #[must_use]
    #[inline]
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(LocalizationAttribute::value)
    }
}

impl<'a> LocalizationNil<'a> {
    #[must_use]
    #[inline]
    pub const fn new(value: Cow<'a, str>, attributes: LocalizationAttributes<'a>) -> Self {
        Self { value, attributes }
    }

    #[must_use]
    #[inline]
    pub fn value(&self) -> &str {
        &self.value
    }

    #[must_use]
    #[inline]
    pub const fn value_cow(&self) -> &Cow<'a, str> {
        &self.value
    }

    #[must_use]
    #[inline]
    pub fn attributes(&self) -> &[LocalizationAttribute<'a>] {
        &self.attributes
    }
}

impl<'a> LocalizationAttribute<'a> {
    #[must_use]
    #[inline]
    pub const fn new(name: Cow<'a, str>, value: Cow<'a, str>) -> Self {
        Self { name, value }
    }

    #[must_use]
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    #[inline]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Streams a localization XML document from raw bytes to `visitor`.
///
/// # Errors
///
/// Returns [`ParseError::InvalidUtf8`] if `bytes` is not valid UTF-8, plus any
/// error [`visit_str`] returns.
pub fn visit_bytes<'a>(
    bytes: &'a [u8],
    visitor: &mut impl LocalizationVisitor<'a>,
) -> Result<(), ParseError> {
    visit_str(str::from_utf8(bytes)?, visitor)
}

/// Streams a localization XML document to `visitor`.
///
/// # Errors
///
/// Returns [`ParseError::Xml`] for a malformed document the underlying reader
/// rejects, [`ParseError::Attribute`] for an unparseable attribute, and
/// [`ParseError::MissingKey`] for a `<string>` with no key attribute.
/// Structural violations are reported as [`ParseError::UnexpectedRoot`] when
/// the first element is not `<resources>`, [`ParseError::UnexpectedElement`]
/// for a second `<resources>` or any element other than `<string>` inside it,
/// [`ParseError::NestedString`] for a `<string>` opened inside another, and
/// [`ParseError::TextOutsideString`] for non-whitespace text or CDATA outside
/// any entry.
pub fn visit_str<'a>(
    xml: &'a str,
    visitor: &mut impl LocalizationVisitor<'a>,
) -> Result<(), ParseError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut saw_root = false;
    let mut in_resources = false;
    let mut current: Option<PartialEntry<'a>> = None;

    loop {
        match reader.read_event()? {
            Event::Start(event) if event.name().as_ref() == b"resources" => {
                if saw_root {
                    return Err(ParseError::UnexpectedElement {
                        name: element_name(&reader, event.name().as_ref()),
                    });
                }
                saw_root = true;
                in_resources = true;
            }
            Event::Empty(event) if event.name().as_ref() == b"resources" => {
                if saw_root {
                    return Err(ParseError::UnexpectedElement {
                        name: element_name(&reader, event.name().as_ref()),
                    });
                }
                saw_root = true;
            }
            Event::Start(event) if !saw_root => {
                return Err(ParseError::UnexpectedRoot {
                    name: element_name(&reader, event.name().as_ref()),
                });
            }
            Event::Empty(event) if !saw_root => {
                return Err(ParseError::UnexpectedRoot {
                    name: element_name(&reader, event.name().as_ref()),
                });
            }
            Event::Start(event) if event.name().as_ref() == b"string" => {
                if !in_resources {
                    return Err(ParseError::UnexpectedElement {
                        name: element_name(&reader, event.name().as_ref()),
                    });
                }
                if current.is_some() {
                    return Err(ParseError::NestedString);
                }
                current = Some(PartialEntry::from_start(&reader, &event)?);
            }
            Event::Empty(event) if event.name().as_ref() == b"string" => {
                if !in_resources {
                    return Err(ParseError::UnexpectedElement {
                        name: element_name(&reader, event.name().as_ref()),
                    });
                }
                if current.is_some() {
                    return Err(ParseError::NestedString);
                }
                visitor.entry(PartialEntry::from_start(&reader, &event)?.finish());
            }
            Event::Start(event) | Event::Empty(event) => {
                return Err(ParseError::UnexpectedElement {
                    name: element_name(&reader, event.name().as_ref()),
                });
            }
            Event::Text(event) => {
                let value = event
                    .xml_content(quick_xml::XmlVersion::default())
                    .map_err(quick_xml::Error::from)?;
                push_character_data(current.as_mut(), value)?;
            }
            Event::CData(event) => {
                let value = event
                    .xml_content(quick_xml::XmlVersion::default())
                    .map_err(quick_xml::Error::from)?;
                push_character_data(current.as_mut(), value)?;
            }
            Event::GeneralRef(event) => {
                push_character_data(current.as_mut(), general_reference_value(&event)?)?;
            }
            Event::End(event) if event.name().as_ref() == b"string" => {
                let Some(entry) = current.take() else {
                    return Err(ParseError::UnexpectedElement {
                        name: element_name(&reader, event.name().as_ref()),
                    });
                };
                visitor.entry(entry.finish());
            }
            Event::End(event) if event.name().as_ref() == b"resources" => {
                in_resources = false;
            }
            Event::End(event) => {
                return Err(ParseError::UnexpectedElement {
                    name: element_name(&reader, event.name().as_ref()),
                });
            }
            Event::Eof => break,
            Event::Decl(_) | Event::PI(_) | Event::DocType(_) | Event::Comment(_) => {}
        }
    }

    if !saw_root {
        return Err(ParseError::MissingResourcesRoot);
    }

    Ok(())
}

#[derive(Debug)]
struct PartialEntry<'a> {
    key: Option<Cow<'a, str>>,
    attributes: LocalizationAttributes<'a>,
    value: TextAccumulator<'a>,
}

impl<'a> PartialEntry<'a> {
    fn from_start(reader: &Reader<&[u8]>, event: &BytesStart<'a>) -> Result<Self, ParseError> {
        let mut key = None;
        let mut attributes = LocalizationAttributes::new();

        for attribute in event.attributes() {
            let attribute = attribute?;
            let name = Cow::Owned(
                reader
                    .decoder()
                    .decode(attribute.key.as_ref())
                    .map_err(quick_xml::Error::from)?
                    .into_owned(),
            );
            let value = Cow::Owned(
                attribute
                    .decoded_and_normalized_value(
                        quick_xml::XmlVersion::default(),
                        reader.decoder(),
                    )?
                    .into_owned(),
            );

            if name == "key" {
                key = Some(value);
            } else {
                attributes.push(LocalizationAttribute::new(name, value));
            }
        }

        if key.is_none() && !is_nil(&attributes) {
            return Err(ParseError::MissingKey);
        }

        Ok(Self {
            key,
            attributes,
            value: TextAccumulator::default(),
        })
    }

    fn finish(self) -> LocalizationEntry<'a> {
        let value = self.value.finish();
        match self.key {
            Some(key) => LocalizationEntry::string(key, value, self.attributes),
            None => LocalizationEntry::nil(value, self.attributes),
        }
    }
}

/// Append character data to the `<string>` entry currently being read.
///
/// Text, CDATA and general references all reach the accumulator through here,
/// so they share one rule: outside an entry, only whitespace is tolerated.
fn push_character_data<'a>(
    current: Option<&mut PartialEntry<'a>>,
    value: Cow<'a, str>,
) -> Result<(), ParseError> {
    if let Some(entry) = current {
        entry.value.push(value);
    } else if !value.trim().is_empty() {
        return Err(ParseError::TextOutsideString);
    }
    Ok(())
}

#[derive(Debug, Default)]
enum TextAccumulator<'a> {
    #[default]
    Empty,
    One(Cow<'a, str>),
    Many(String),
}

impl<'a> TextAccumulator<'a> {
    fn push(&mut self, value: Cow<'a, str>) {
        if value.is_empty() {
            return;
        }

        match self {
            Self::Empty => *self = Self::One(value),
            Self::One(previous) => {
                let mut combined = String::with_capacity(previous.len() + value.len());
                combined.push_str(previous);
                combined.push_str(&value);
                *self = Self::Many(combined);
            }
            Self::Many(text) => text.push_str(&value),
        }
    }

    fn finish(self) -> Cow<'a, str> {
        match self {
            Self::Empty => Cow::Borrowed(""),
            Self::One(value) => value,
            Self::Many(value) => Cow::Owned(value),
        }
    }
}

#[derive(Default)]
struct EntryCollector<'a> {
    entries: Vec<LocalizationEntry<'a>>,
}

impl<'a> LocalizationVisitor<'a> for EntryCollector<'a> {
    fn entry(&mut self, entry: LocalizationEntry<'a>) {
        self.entries.push(entry);
    }
}

fn element_name(reader: &Reader<&[u8]>, raw: &[u8]) -> String {
    reader.decoder().decode(raw).map_or_else(
        |_| String::from_utf8_lossy(raw).into_owned(),
        Cow::into_owned,
    )
}

fn general_reference_value(event: &BytesRef<'_>) -> Result<Cow<'static, str>, quick_xml::Error> {
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

fn is_nil(attributes: &[LocalizationAttribute<'_>]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.name() == "xsi:nil" && attribute.value() == "true")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registration is keyed on the builder id and ordered by the name, so
    /// a registration that disagrees with the rule it resolves would file job
    /// attempts under an identity the dispatcher never reports.
    #[test]
    fn every_registration_matches_the_rule_it_resolves() {
        let registries = az_gem_contract::Registries::new();
        let context = az_asset_builder::JobContext::new(&registries);

        for registration in build_rules() {
            let rule = registration.rule(&context);
            assert_eq!(registration.name(), rule.name);
            assert_eq!(registration.id(), rule.id);
        }
    }

    #[test]
    fn parses_empty_resources() {
        let document = LocalizationDocument::parse_str(
            r#"<resources xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"/>"#,
        )
        .expect("parse empty localization document");

        assert!(document.is_empty());
    }

    #[test]
    fn parses_string_entries_and_attributes() {
        let document = LocalizationDocument::parse_str(
            r#"<resources><string key="Quest_1" speaker="Grace" rel_version="Launch">Hello</string></resources>"#,
        )
        .expect("parse localization document");

        let entry = &document.entries()[0];
        assert_eq!(entry.key(), Some("Quest_1"));
        assert_eq!(entry.value(), "Hello");
        assert_eq!(entry.attribute("speaker"), Some("Grace"));
        assert_eq!(entry.attribute("rel_version"), Some("Launch"));
    }

    #[test]
    fn unescapes_text_and_attribute_values() {
        let document = LocalizationDocument::parse_str(
            r#"<resources><string key="Quest_&amp;_1" speaker="A&amp;B">Fish &amp; Chips</string></resources>"#,
        )
        .expect("parse escaped localization document");

        let entry = &document.entries()[0];
        assert_eq!(entry.key(), Some("Quest_&_1"));
        assert_eq!(entry.value(), "Fish & Chips");
        assert_eq!(entry.attribute("speaker"), Some("A&B"));
    }

    #[test]
    fn parses_nil_string_entries() {
        let document = LocalizationDocument::parse_str(
            r#"<resources><string rel_version="Launch" xsi:nil="true"/></resources>"#,
        )
        .expect("parse nil localization document");

        let entry = &document.entries()[0];
        assert!(entry.is_nil());
        assert_eq!(entry.key(), None);
        assert_eq!(entry.attribute("xsi:nil"), Some("true"));
        assert_eq!(entry.attribute("rel_version"), Some("Launch"));
    }

    #[test]
    fn streams_entries_without_document_collection() {
        #[derive(Default)]
        struct Counter {
            entries: usize,
            attributes: usize,
        }

        impl<'a> LocalizationVisitor<'a> for Counter {
            fn entry(&mut self, entry: LocalizationEntry<'a>) {
                self.entries += 1;
                self.attributes += entry.attributes().len();
            }
        }

        let mut counter = Counter::default();
        visit_str(
            r#"<resources><string key="a">A</string><string key="b" speaker="B">B</string></resources>"#,
            &mut counter,
        )
        .expect("visit localization document");

        assert_eq!(counter.entries, 2);
        assert_eq!(counter.attributes, 1);

        let summary = summarize_localization_bytes(
            br#"<resources><string key="a">A</string><string key="b" speaker="B">B</string></resources>"#,
        )
        .expect("summarize localization document");
        assert_eq!(summary.entries, 2);
        assert_eq!(summary.attributes, 1);

        let mut totals = LocalizationTotals::default();
        totals.add_summary(summary);
        assert_eq!(totals.files, 1);
        assert_eq!(totals.entries, 2);
        assert_eq!(summary.to_string(), "2 strings, 1 metadata attributes");
        assert_eq!(
            totals.to_string(),
            "  files: 1\n  strings: 2\n  nil strings: 0\n  metadata attributes: 1\n  empty strings: 0\n  text bytes: 2\n"
        );

        let mut inspection = LocalizationInspection::default();
        inspection.add_file_summary(
            inspect_localization_file(
                "localization/en-us.loc.xml",
                br#"<resources><string key="a">A</string><string key="b" speaker="B">B</string></resources>"#,
            )
            .expect("inspect localization"),
        );
        assert_eq!(
            inspection.report(20).to_string(),
            "localization/en-us.loc.xml: 2 strings, 1 metadata attributes\n  files: 1\n  strings: 2\n  nil strings: 0\n  metadata attributes: 1\n  empty strings: 0\n  text bytes: 2\n"
        );

        assert!(is_localization_source_name("en-us.LOC.XML"));
        assert!(is_localization_source_name("en-us.LOC"));
        assert!(!is_localization_source_name("en-us.loc.ron"));
    }

    #[test]
    fn inspect_localization_files_aggregates_file_results() {
        let path = std::env::temp_dir().join(format!(
            "az-rs-cry-localization-{}-en-us.loc.xml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            br#"<resources><string key="a">A</string><string key="b">B</string></resources>"#,
        )
        .expect("write localization");

        let inspection = inspect_localization_files([&path]).expect("inspect localization files");

        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.entries, 2);
        assert_eq!(inspection.totals.text_bytes, 2);

        std::fs::remove_file(path).expect("remove localization");
    }

    #[test]
    fn rejects_missing_key() {
        let err = LocalizationDocument::parse_str(r"<resources><string>value</string></resources>")
            .expect_err("missing key should fail");

        assert!(matches!(err, ParseError::MissingKey));
    }
}
