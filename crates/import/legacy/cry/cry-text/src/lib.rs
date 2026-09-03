//! Cry/Lumberyard text-backed asset parsers.

pub mod source_transform;

use az_asset_builder::{SourceFormat, SourceSchemaRegistration, source_schema_type};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt, io,
    num::ParseIntError,
    path::{Path, PathBuf},
    str,
};

use encoding_rs::WINDOWS_1252;
use thiserror::Error;

pub use source_transform::{
    GpuDeviceSourceEntry, GpuDeviceTableSource, LevelReferenceListSource,
    LevelReferenceListSourceEntry, LevelReferenceListSourceKind, TextSourceTransform,
    TextSourceTransformError, gpu_device_table_source_path, is_legacy_text_source,
    level_reference_list_source_path, text_source_path,
};

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.GpuDeviceTableSource",
    ext = "gpudevices.ron"
)]
pub struct GpuDeviceTableSourceFormat;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.LevelReferenceListSource",
    ext = "levellist.ron"
)]
pub struct LevelReferenceListSourceFormat;

pub mod source_schemas {
    use super::{GpuDeviceTableSourceFormat, LevelReferenceListSourceFormat, source_schema_type};
    use az_asset_builder::SourceSchemaType;

    pub const GPU_DEVICE_TABLE: SourceSchemaType =
        source_schema_type::<GpuDeviceTableSourceFormat>();
    pub const LEVEL_REFERENCE_LIST: SourceSchemaType =
        source_schema_type::<LevelReferenceListSourceFormat>();
}

/// The source schemas this crate owns, for a host contribution to register.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 2] {
    [
        SourceSchemaRegistration::for_source::<GpuDeviceTableSourceFormat>()
            .with_category("Cry/Lumberyard Compatibility")
            .with_import_file("config/gpu", &["gpudevices.ron"]),
        SourceSchemaRegistration::for_source::<LevelReferenceListSourceFormat>()
            .with_category("Cry/Lumberyard Compatibility")
            .with_import_file("levels", &["levellist.ron"]),
    ]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseError {
    #[error("line {line} is missing {field}")]
    MissingField { line: usize, field: &'static str },

    #[error("line {line} has invalid hex integer {value:?}")]
    InvalidHex {
        line: usize,
        value: String,
        source: ParseIntError,
    },
}

pub type TextParseError = ParseError;

pub const TEXT_EXTENSION: &str = "txt";

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TextInspectionError {
    #[error("failed to read {path:?}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to parse text asset {path:?}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextAssetKind {
    ResourceList,
    LayerResourceList,
    GpuDeviceTable,
    PlainText,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextAsset {
    ResourceList(ResourceList),
    LayerResourceList(LayerResourceList),
    GpuDeviceTable(GpuDeviceTable),
    PlainText(PlainText),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceList {
    entries: Vec<ResourceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceEntry {
    path: Box<str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceEntryRef<'a> {
    path: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerResourceList {
    entries: Vec<LayerResourceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerResourceEntry {
    layer: Box<str>,
    path: Box<str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerResourceEntryRef<'a> {
    layer: &'a str,
    path: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuDeviceTable {
    entries: Vec<GpuDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuDevice {
    vendor_id: u32,
    device_id: u32,
    bucket: i32,
    comment: Box<str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuDeviceRef<'a> {
    vendor_id: u32,
    device_id: u32,
    bucket: i32,
    comment: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlainText {
    lines: Vec<Box<str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlainLineRef<'a> {
    text: &'a str,
}

pub trait TextVisitor {
    fn kind(&mut self, _kind: TextAssetKind) {}

    fn resource_entry(&mut self, _entry: ResourceEntryRef<'_>) {}

    fn layer_resource_entry(&mut self, _entry: LayerResourceEntryRef<'_>) {}

    fn gpu_device(&mut self, _entry: GpuDeviceRef<'_>) {}

    fn plain_line(&mut self, _line: PlainLineRef<'_>) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSummary {
    pub kind: TextAssetKind,
    pub resource_entries: usize,
    pub layer_resource_entries: usize,
    pub gpu_devices: usize,
    pub plain_lines: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TextTotals {
    pub files: usize,
    pub resource_entries: usize,
    pub layer_resource_entries: usize,
    pub gpu_devices: usize,
    pub plain_lines: usize,
    pub kinds: BTreeMap<TextAssetKind, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFileSummary {
    pub source: String,
    pub summary: TextSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TextInspection {
    pub rows: Vec<TextFileSummary>,
    pub totals: TextTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct TextInspectionReport<'a> {
    inspection: &'a TextInspection,
    limit: usize,
}

impl Default for TextSummary {
    fn default() -> Self {
        Self {
            kind: TextAssetKind::PlainText,
            resource_entries: 0,
            layer_resource_entries: 0,
            gpu_devices: 0,
            plain_lines: 0,
        }
    }
}

impl TextSummary {
    #[must_use]
    pub fn label(self) -> String {
        match self.kind {
            TextAssetKind::ResourceList => format!("{} entries", self.resource_entries),
            TextAssetKind::LayerResourceList => {
                format!("{} entries", self.layer_resource_entries)
            }
            TextAssetKind::GpuDeviceTable => format!("{} devices", self.gpu_devices),
            TextAssetKind::PlainText => format!("{} lines", self.plain_lines),
        }
    }
}

impl fmt::Display for TextSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.kind.as_str(), self.label())
    }
}

impl TextVisitor for TextSummary {
    fn kind(&mut self, kind: TextAssetKind) {
        self.kind = kind;
    }

    fn resource_entry(&mut self, _entry: ResourceEntryRef<'_>) {
        self.resource_entries += 1;
    }

    fn layer_resource_entry(&mut self, _entry: LayerResourceEntryRef<'_>) {
        self.layer_resource_entries += 1;
    }

    fn gpu_device(&mut self, _entry: GpuDeviceRef<'_>) {
        self.gpu_devices += 1;
    }

    fn plain_line(&mut self, _line: PlainLineRef<'_>) {
        self.plain_lines += 1;
    }
}

impl TextTotals {
    pub fn add_summary(&mut self, summary: TextSummary) {
        self.files += 1;
        self.resource_entries += summary.resource_entries;
        self.layer_resource_entries += summary.layer_resource_entries;
        self.gpu_devices += summary.gpu_devices;
        self.plain_lines += summary.plain_lines;
        *self.kinds.entry(summary.kind).or_default() += 1;
    }
}

impl TextInspection {
    pub fn add_file_summary(&mut self, row: TextFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> TextInspectionReport<'_> {
        TextInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for TextTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  resource entries: {}", self.resource_entries)?;
        writeln!(
            f,
            "  layer resource entries: {}",
            self.layer_resource_entries
        )?;
        writeln!(f, "  gpu device rows: {}", self.gpu_devices)?;
        writeln!(f, "  plain text lines: {}", self.plain_lines)?;
        for (kind, files) in &self.kinds {
            writeln!(f, "  {}: {}", kind.as_str(), files)?;
        }
        Ok(())
    }
}

impl fmt::Display for TextInspectionReport<'_> {
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

impl TextAssetKind {
    #[must_use]
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResourceList => "resource list",
            Self::LayerResourceList => "layer resource list",
            Self::GpuDeviceTable => "GPU device table",
            Self::PlainText => "plain text",
        }
    }

    #[must_use]
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        let normalized = normalize_path(path.as_ref());
        let name = normalized.rsplit('/').next().unwrap_or(normalized.as_str());

        match name {
            "perlayerresourcelist.txt" => Self::LayerResourceList,
            "resourcelist.txt"
            | "auto_resourcelist.txt"
            | "brushlist.txt"
            | "shaderslist.txt"
            | "full_lod_asset_list.txt"
            | "tags.txt" => Self::ResourceList,
            "amd.txt" | "intel.txt" | "nvidia.txt" if normalized.contains("config/gpu/") => {
                Self::GpuDeviceTable
            }
            _ => Self::PlainText,
        }
    }
}

impl TextAsset {
    /// Classifies `path` and parses `bytes` as that kind of text asset.
    ///
    /// # Errors
    ///
    /// Depends on the kind [`TextAssetKind::from_path`] picks. A layer
    /// resource list yields [`ParseError::MissingField`] for a data line with
    /// no `;` separator. A GPU device table yields [`ParseError::MissingField`]
    /// for a row short of its `vendor_id`, `device_id` or `bucket` column, or
    /// [`ParseError::InvalidHex`] when one of those columns does not parse.
    /// Resource lists and plain text never fail; unknown bytes are decoded as
    /// Windows-1252 rather than rejected.
    pub fn parse_path(path: impl AsRef<Path>, bytes: &[u8]) -> Result<Self, ParseError> {
        let kind = TextAssetKind::from_path(path);
        let text = decode_text(bytes);
        match kind {
            TextAssetKind::ResourceList => ResourceList::parse_str(&text).map(Self::ResourceList),
            TextAssetKind::LayerResourceList => {
                LayerResourceList::parse_str(&text).map(Self::LayerResourceList)
            }
            TextAssetKind::GpuDeviceTable => {
                GpuDeviceTable::parse_str(&text).map(Self::GpuDeviceTable)
            }
            TextAssetKind::PlainText => Ok(Self::PlainText(PlainText::parse_str(&text))),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> TextAssetKind {
        match self {
            Self::ResourceList(_) => TextAssetKind::ResourceList,
            Self::LayerResourceList(_) => TextAssetKind::LayerResourceList,
            Self::GpuDeviceTable(_) => TextAssetKind::GpuDeviceTable,
            Self::PlainText(_) => TextAssetKind::PlainText,
        }
    }
}

/// Classifies `path`, then streams `bytes` to `visitor` without building an
/// owned [`TextAsset`].
///
/// # Errors
///
/// Returns the same errors as [`TextAsset::parse_path`] for the kind
/// [`TextAssetKind::from_path`] picks: [`ParseError::MissingField`] for a layer
/// resource line with no `;`, or for a GPU device row short of its
/// `vendor_id`, `device_id` or `bucket` column; and
/// [`ParseError::InvalidHex`] when one of those columns does not parse.
/// Resource lists and plain text never fail.
pub fn visit_path(
    path: impl AsRef<Path>,
    bytes: &[u8],
    visitor: &mut impl TextVisitor,
) -> Result<(), ParseError> {
    let kind = TextAssetKind::from_path(path);
    visitor.kind(kind);
    let text = decode_text(bytes);
    match kind {
        TextAssetKind::ResourceList => {
            visit_resource_list(&text, visitor);
            Ok(())
        }
        TextAssetKind::LayerResourceList => visit_layer_resource_list(&text, visitor),
        TextAssetKind::GpuDeviceTable => visit_gpu_device_table(&text, visitor),
        TextAssetKind::PlainText => {
            visit_plain_text(&text, visitor);
            Ok(())
        }
    }
}

/// Counts the entries in one text asset's bytes.
///
/// # Errors
///
/// Returns any error [`visit_path`] returns — [`ParseError::MissingField`] or
/// [`ParseError::InvalidHex`] on a malformed layer-resource or GPU-device row.
pub fn summarize_text_path(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<TextSummary, ParseError> {
    let mut summary = TextSummary::default();
    visit_path(path, bytes, &mut summary)?;
    Ok(summary)
}

/// Summarises one text asset's bytes, labelling the row with `path`.
///
/// `path` selects the asset kind and becomes the display label; it is not read
/// from disk.
///
/// # Errors
///
/// Returns any error [`summarize_text_path`] returns —
/// [`ParseError::MissingField`] or [`ParseError::InvalidHex`] on a malformed
/// layer-resource or GPU-device row.
pub fn inspect_text_path(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<TextFileSummary, ParseError> {
    let path = path.as_ref();
    Ok(TextFileSummary {
        source: path.display().to_string(),
        summary: summarize_text_path(path, bytes)?,
    })
}

/// Reads a text asset from disk and summarises its entries.
///
/// # Errors
///
/// Returns [`TextInspectionError::Read`] if `path` cannot be read (missing
/// file, permissions), or [`TextInspectionError::Parse`] wrapping the
/// [`ParseError`] from a malformed layer-resource or GPU-device row. Both
/// variants carry the offending path.
pub fn inspect_text_file(path: impl AsRef<Path>) -> Result<TextFileSummary, TextInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| TextInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_text_path(path, &bytes).map_err(|source| TextInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Reads and summarises every text asset in `paths`, accumulating totals.
///
/// Stops at the first failing path; earlier rows are discarded with it.
///
/// # Errors
///
/// Returns any error [`inspect_text_file`] returns for the first path that
/// fails — [`TextInspectionError::Read`] for an unreadable file, or
/// [`TextInspectionError::Parse`] for a malformed row.
pub fn inspect_text_files<I, P>(paths: I) -> Result<TextInspection, TextInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = TextInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_text_file(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub const fn is_text_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(TEXT_EXTENSION)
}

#[must_use]
pub fn is_text_name(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_text_extension)
}

#[must_use]
pub fn is_text_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_text_extension)
}

impl ResourceList {
    /// Parses a resource list: one asset path per non-empty line.
    ///
    /// # Errors
    ///
    /// This parser accepts every input. It returns `Ok` after collecting
    /// non-empty lines and skipping blank lines and a leading byte-order mark.
    pub fn parse_str(input: &str) -> Result<Self, ParseError> {
        let mut entries = Vec::new();
        visit_resource_list(input, &mut ResourceCollector(&mut entries));
        Ok(Self { entries })
    }

    #[must_use]
    pub fn entries(&self) -> &[ResourceEntry] {
        &self.entries
    }
}

impl ResourceEntry {
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl<'a> ResourceEntryRef<'a> {
    #[must_use]
    pub const fn path(&self) -> &'a str {
        self.path
    }
}

impl LayerResourceList {
    /// Parses a layer resource list: `layer;path` per non-empty line.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::MissingField`] with `field: "path"` for the first
    /// non-empty line that has no `;` separator; `line` is the 1-based line
    /// number.
    pub fn parse_str(input: &str) -> Result<Self, ParseError> {
        let mut entries = Vec::new();
        visit_layer_resource_list(input, &mut LayerResourceCollector(&mut entries))?;
        Ok(Self { entries })
    }

    #[must_use]
    pub fn entries(&self) -> &[LayerResourceEntry] {
        &self.entries
    }
}

impl LayerResourceEntry {
    #[must_use]
    pub fn layer(&self) -> &str {
        &self.layer
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl<'a> LayerResourceEntryRef<'a> {
    #[must_use]
    pub const fn layer(&self) -> &'a str {
        self.layer
    }

    #[must_use]
    pub const fn path(&self) -> &'a str {
        self.path
    }
}

impl GpuDeviceTable {
    /// Parses a GPU device table: `vendor_id,device_id,bucket // comment`.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::MissingField`] naming `vendor_id`, `device_id` or
    /// `bucket` for the first data row short of that column, or
    /// [`ParseError::InvalidHex`] when `vendor_id`/`device_id` is not
    /// hexadecimal or `bucket` is not a decimal `i32`. Both carry the 1-based
    /// line number. Blank lines and comment-only lines are skipped.
    pub fn parse_str(input: &str) -> Result<Self, ParseError> {
        let mut entries = Vec::new();
        visit_gpu_device_table(input, &mut GpuCollector(&mut entries))?;
        Ok(Self { entries })
    }

    #[must_use]
    pub fn entries(&self) -> &[GpuDevice] {
        &self.entries
    }
}

impl GpuDevice {
    #[must_use]
    pub const fn vendor_id(&self) -> u32 {
        self.vendor_id
    }

    #[must_use]
    pub const fn device_id(&self) -> u32 {
        self.device_id
    }

    #[must_use]
    pub const fn bucket(&self) -> i32 {
        self.bucket
    }

    #[must_use]
    pub fn comment(&self) -> &str {
        &self.comment
    }
}

impl<'a> GpuDeviceRef<'a> {
    #[must_use]
    pub const fn vendor_id(&self) -> u32 {
        self.vendor_id
    }

    #[must_use]
    pub const fn device_id(&self) -> u32 {
        self.device_id
    }

    #[must_use]
    pub const fn bucket(&self) -> i32 {
        self.bucket
    }

    #[must_use]
    pub const fn comment(&self) -> &'a str {
        self.comment
    }
}

impl PlainText {
    #[must_use]
    pub fn parse_str(input: &str) -> Self {
        Self {
            lines: input.lines().map(|line| line.trim_end().into()).collect(),
        }
    }

    #[must_use]
    pub fn lines(&self) -> &[Box<str>] {
        &self.lines
    }
}

impl<'a> PlainLineRef<'a> {
    #[must_use]
    pub const fn text(&self) -> &'a str {
        self.text
    }
}

fn visit_resource_list(input: &str, visitor: &mut impl TextVisitor) {
    for line in logical_lines(input) {
        visitor.resource_entry(ResourceEntryRef { path: line });
    }
}

fn visit_layer_resource_list(
    input: &str,
    visitor: &mut impl TextVisitor,
) -> Result<(), ParseError> {
    for (index, line) in input.lines().enumerate() {
        let line_number = index + 1;
        let line = trim_data_line(line);
        if line.is_empty() {
            continue;
        }
        let (layer, path) = line.split_once(';').ok_or(ParseError::MissingField {
            line: line_number,
            field: "path",
        })?;
        visitor.layer_resource_entry(LayerResourceEntryRef {
            layer: layer.trim(),
            path: path.trim(),
        });
    }
    Ok(())
}

fn visit_gpu_device_table(input: &str, visitor: &mut impl TextVisitor) -> Result<(), ParseError> {
    for (index, line) in input.lines().enumerate() {
        let line_number = index + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let (data, comment) = line
            .split_once("//")
            .map_or((line, ""), |(data, comment)| (data.trim(), comment.trim()));
        if data.is_empty() {
            continue;
        }

        let mut fields = data.split(',').map(str::trim);
        let vendor_id = parse_hex_field(
            line_number,
            fields.next().ok_or(ParseError::MissingField {
                line: line_number,
                field: "vendor_id",
            })?,
        )?;
        let device_id = parse_hex_field(
            line_number,
            fields.next().ok_or(ParseError::MissingField {
                line: line_number,
                field: "device_id",
            })?,
        )?;
        let bucket = fields
            .next()
            .ok_or(ParseError::MissingField {
                line: line_number,
                field: "bucket",
            })?
            .parse()
            .map_err(|source| ParseError::InvalidHex {
                line: line_number,
                value: data.to_string(),
                source,
            })?;

        visitor.gpu_device(GpuDeviceRef {
            vendor_id,
            device_id,
            bucket,
            comment,
        });
    }
    Ok(())
}

fn visit_plain_text(input: &str, visitor: &mut impl TextVisitor) {
    for line in input.lines() {
        visitor.plain_line(PlainLineRef {
            text: line.trim_end(),
        });
    }
}

fn logical_lines(input: &str) -> impl Iterator<Item = &str> {
    input
        .lines()
        .map(trim_data_line)
        .filter(|line| !line.is_empty())
}

fn trim_data_line(line: &str) -> &str {
    line.trim_start_matches('\u{feff}').trim()
}

fn parse_hex_field(line: usize, value: &str) -> Result<u32, ParseError> {
    let value = value.trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(value, 16).map_err(|source| ParseError::InvalidHex {
        line,
        value: value.to_string(),
        source,
    })
}

fn decode_text(bytes: &[u8]) -> Cow<'_, str> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    str::from_utf8(bytes).map_or_else(|_| WINDOWS_1252.decode(bytes).0, Cow::Borrowed)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

struct ResourceCollector<'a>(&'a mut Vec<ResourceEntry>);

impl TextVisitor for ResourceCollector<'_> {
    fn resource_entry(&mut self, entry: ResourceEntryRef<'_>) {
        self.0.push(ResourceEntry {
            path: entry.path.into(),
        });
    }
}

struct LayerResourceCollector<'a>(&'a mut Vec<LayerResourceEntry>);

impl TextVisitor for LayerResourceCollector<'_> {
    fn layer_resource_entry(&mut self, entry: LayerResourceEntryRef<'_>) {
        self.0.push(LayerResourceEntry {
            layer: entry.layer.into(),
            path: entry.path.into(),
        });
    }
}

struct GpuCollector<'a>(&'a mut Vec<GpuDevice>);

impl TextVisitor for GpuCollector<'_> {
    fn gpu_device(&mut self, entry: GpuDeviceRef<'_>) {
        self.0.push(GpuDevice {
            vendor_id: entry.vendor_id,
            device_id: entry.device_id,
            bucket: entry.bucket,
            comment: entry.comment.into(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_text_assets() {
        assert_eq!(
            TextAssetKind::from_path("levels/foo/resourcelist.txt"),
            TextAssetKind::ResourceList
        );
        assert_eq!(
            TextAssetKind::from_path("levels/foo/perlayerresourcelist.txt"),
            TextAssetKind::LayerResourceList
        );
        assert_eq!(
            TextAssetKind::from_path("config/gpu/amd.txt"),
            TextAssetKind::GpuDeviceTable
        );
    }

    #[test]
    fn parses_resource_lists() {
        let list = ResourceList::parse_str("\u{feff}@assets@/foo.xml\n\n@assets@/bar.dds\n")
            .expect("parse");

        assert_eq!(list.entries()[0].path(), "@assets@/foo.xml");
        assert_eq!(list.entries()[1].path(), "@assets@/bar.dds");
    }

    #[test]
    fn parses_layer_resource_lists() {
        let list = LayerResourceList::parse_str("Main;textures/test/foo.dds\n").expect("parse");

        assert_eq!(list.entries()[0].layer(), "Main");
        assert_eq!(list.entries()[0].path(), "textures/test/foo.dds");
    }

    #[test]
    fn parses_gpu_device_tables() {
        let table = GpuDeviceTable::parse_str("0x1002, 0x6759, 1 // Radeon HD 6500 Series\n")
            .expect("parse");

        assert_eq!(table.entries()[0].vendor_id(), 0x1002);
        assert_eq!(table.entries()[0].device_id(), 0x6759);
        assert_eq!(table.entries()[0].bucket(), 1);
        assert_eq!(table.entries()[0].comment(), "Radeon HD 6500 Series");
    }

    #[test]
    fn summarizes_text_assets_and_paths() {
        let path = "levels/foo/resourcelist.txt";
        let bytes = b"@assets@/foo.xml\n\n@assets@/bar.dds\n";
        let summary = summarize_text_path(path, bytes).expect("summarize text");
        let mut totals = TextTotals::default();
        totals.add_summary(summary);

        assert_eq!(summary.kind, TextAssetKind::ResourceList);
        assert_eq!(summary.resource_entries, 2);
        assert_eq!(summary.label(), "2 entries");
        assert_eq!(summary.to_string(), "resource list (2 entries)");
        assert_eq!(totals.files, 1);
        assert_eq!(totals.resource_entries, 2);
        assert_eq!(totals.kinds.get(&TextAssetKind::ResourceList), Some(&1));
        assert_eq!(
            totals.to_string(),
            "  files: 1\n  resource entries: 2\n  layer resource entries: 0\n  gpu device rows: 0\n  plain text lines: 0\n  resource list: 1\n"
        );

        let mut inspection = TextInspection::default();
        inspection.add_file_summary(inspect_text_path(path, bytes).expect("inspect text"));
        assert_eq!(
            inspection.report(20).to_string(),
            "levels/foo/resourcelist.txt: resource list (2 entries)\n  files: 1\n  resource entries: 2\n  layer resource entries: 0\n  gpu device rows: 0\n  plain text lines: 0\n  resource list: 1\n"
        );

        assert!(is_text_name("foo.TXT"));
        assert!(is_text_path(Path::new("foo.txt")));
        assert!(!is_text_name("foo.csv"));
    }

    #[test]
    fn inspect_text_files_aggregates_file_results() {
        let path = std::env::temp_dir().join(format!(
            "cry-text-plain-{}-{}.txt",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, b"alpha\nbeta\n").expect("write fixture");

        let inspection = inspect_text_files([&path]).expect("inspect fixture");

        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.plain_lines, 2);
        assert_eq!(
            inspection.totals.kinds.get(&TextAssetKind::PlainText),
            Some(&1)
        );
        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.rows[0].summary.kind, TextAssetKind::PlainText);

        std::fs::remove_file(path).expect("remove fixture");
    }
}
