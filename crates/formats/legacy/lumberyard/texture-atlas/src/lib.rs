//! Lumberyard `TextureAtlas` index parsing.
//!
//! O3DE reference: `Gems/TextureAtlas/Code/Source/TextureAtlasImpl.cpp`.

use std::fmt;
use std::io;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use std::str;

use bevy_math::{URect, UVec2};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use thiserror::Error;
use uuid::{Uuid, uuid};

const OBJECT_STREAM: &[u8] = b"ObjectStream";
const CLASS: &[u8] = b"Class";
const TEXTURE_ATLAS_IMPL: &[u8] = b"TextureAtlasImpl";
const UNORDERED_MAP: &[u8] = b"AZStd::unordered_map";
const PAIR: &[u8] = b"AZStd::pair";
const STRING: &[u8] = b"AZStd::string";
const ATLAS_COORDINATES: &[u8] = b"AtlasCoordinates";
const INT: &[u8] = b"int";

pub const TEXTURE_ATLAS_INDEX_EXTENSION: &str = "texatlasidx";

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TextureAtlasInspectionError {
    #[error("read {path:?}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse TextureAtlas index {path:?}: {source}")]
    Parse {
        path: PathBuf,
        source: TextureAtlasParseError,
    },
}

/// `TextureAtlasNamespace::TextureAtlasImpl` `ObjectStream` UUID.
pub const TEXTURE_ATLAS_IMPL_TYPE_ID: Uuid = uuid!("2CA51C61-1B5F-4480-A257-F28D8944AA35");

/// `TextureAtlasNamespace::AtlasCoordinates` `ObjectStream` UUID.
pub const ATLAS_COORDINATES_TYPE_ID: Uuid = uuid!("FC5D6A60-1056-4F6C-96F7-6A47912F8A35");

/// `TextureAtlasNamespace::TextureAtlasAsset` simple asset UUID.
pub const TEXTURE_ATLAS_ASSET_TYPE_ID: Uuid = uuid!("BFC6C91F-66CE-4D78-B68A-7F697C9EA2E8");

/// `AzFramework::SimpleAssetReference<TextureAtlasAsset>` UUID.
pub const TEXTURE_ATLAS_ASSET_REFERENCE_TYPE_ID: Uuid =
    uuid!("6F612FE6-A054-4E49-830C-0288F3C79A52");

/// Supported Lumberyard `TextureAtlas` `ObjectStream` version.
pub const TEXTURE_ATLAS_OBJECT_STREAM_VERSION: u32 = 3;

/// Latest reflected `TextureAtlasImpl` version supported by the loader.
pub const TEXTURE_ATLAS_IMPL_VERSION: u32 = 2;

/// Summary returned after visiting one `.texatlasidx` asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureAtlasStats {
    pub object_stream_version: u32,
    pub texture_atlas_version: u32,
    pub size: UVec2,
    pub regions: usize,
}

impl fmt::Display for TextureAtlasStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}x{}, {} regions, ObjectStream v{}, TextureAtlasImpl v{}",
            self.size.x,
            self.size.y,
            self.regions,
            self.object_stream_version,
            self.texture_atlas_version
        )
    }
}

/// Counts the regions and records the versions in a `.texatlasidx` index.
///
/// # Errors
///
/// Returns any error [`visit_texture_atlas_index`] returns — the
/// [`TextureAtlasParseError`] variants for non-UTF-8 bytes, malformed XML, an
/// unsupported `ObjectStream` or `TextureAtlasImpl` version, an element or
/// attribute the layout does not allow, or a missing, non-numeric, negative or
/// overflowing region coordinate.
pub fn summarize_texture_atlas_index(
    bytes: &[u8],
) -> Result<TextureAtlasStats, TextureAtlasParseError> {
    visit_texture_atlas_index(bytes, |_| {})
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TextureAtlasTotals {
    pub files: usize,
    pub regions: usize,
    pub pixels: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureAtlasFileSummary {
    pub source: String,
    pub stats: TextureAtlasStats,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TextureAtlasInspection {
    pub rows: Vec<TextureAtlasFileSummary>,
    pub totals: TextureAtlasTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct TextureAtlasInspectionReport<'a> {
    inspection: &'a TextureAtlasInspection,
    limit: usize,
}

impl TextureAtlasTotals {
    pub fn add_stats(&mut self, stats: TextureAtlasStats) {
        self.files += 1;
        self.regions += stats.regions;
        self.pixels += u64::from(stats.size.x) * u64::from(stats.size.y);
    }
}

impl TextureAtlasInspection {
    pub fn add_file_summary(&mut self, row: TextureAtlasFileSummary) {
        self.totals.add_stats(row.stats);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> TextureAtlasInspectionReport<'_> {
        TextureAtlasInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for TextureAtlasTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  regions: {}", self.regions)?;
        write!(f, "  pixels: {}", self.pixels)
    }
}

impl fmt::Display for TextureAtlasInspectionReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.limit > 0 {
            for row in self.inspection.rows.iter().take(self.limit) {
                writeln!(f, "{}: {}", row.source, row.stats)?;
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

/// Summarises one atlas index's bytes, labelling the row with `path`.
///
/// `path` is only the display label; it is not read from disk.
///
/// # Errors
///
/// Returns any error [`summarize_texture_atlas_index`] returns — the
/// [`TextureAtlasParseError`] variants for non-UTF-8 bytes, malformed XML, an
/// unsupported version, a disallowed element or attribute, or a bad region
/// coordinate.
pub fn inspect_texture_atlas_index_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<TextureAtlasFileSummary, TextureAtlasParseError> {
    Ok(TextureAtlasFileSummary {
        source: path.as_ref().display().to_string(),
        stats: summarize_texture_atlas_index(bytes)?,
    })
}

/// Reads an atlas index from disk and summarises it.
///
/// # Errors
///
/// Returns [`TextureAtlasInspectionError::Read`] if `path` cannot be read
/// (missing file, permissions), or [`TextureAtlasInspectionError::Parse`]
/// wrapping the [`TextureAtlasParseError`] from a malformed index. Both
/// variants carry the offending path.
pub fn inspect_texture_atlas_index_path(
    path: impl AsRef<Path>,
) -> Result<TextureAtlasFileSummary, TextureAtlasInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| TextureAtlasInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_texture_atlas_index_file(path, &bytes).map_err(|source| {
        TextureAtlasInspectionError::Parse {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// Reads and summarises every atlas index in `paths`, accumulating totals.
///
/// Stops at the first failing path; earlier rows are discarded with it.
///
/// # Errors
///
/// Returns any error [`inspect_texture_atlas_index_path`] returns for the
/// first path that fails — [`TextureAtlasInspectionError::Read`] for an
/// unreadable file, or [`TextureAtlasInspectionError::Parse`] for a malformed
/// index.
pub fn inspect_texture_atlas_index_files<I, P>(
    paths: I,
) -> Result<TextureAtlasInspection, TextureAtlasInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = TextureAtlasInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_texture_atlas_index_path(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub const fn is_texture_atlas_index_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(TEXTURE_ATLAS_INDEX_EXTENSION)
}

#[must_use]
pub fn is_texture_atlas_index_name(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_texture_atlas_index_extension)
}

#[must_use]
pub fn is_texture_atlas_index_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_texture_atlas_index_extension)
}

/// Borrowed view of one atlas region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureAtlasRegionRef<'a> {
    pub name: &'a str,
    pub rect: URect,
}

/// Parse a `.texatlasidx` `ObjectStream` with a streaming visitor.
///
/// # Errors
///
/// Returns [`TextureAtlasParseError::Utf8`] if `bytes` is not valid UTF-8, and
/// [`TextureAtlasParseError::Xml`] or [`TextureAtlasParseError::Attribute`]
/// for a document the reader rejects.
///
/// Document shape is checked as it streams:
/// [`TextureAtlasParseError::MissingRoot`] for no `ObjectStream` element,
/// [`TextureAtlasParseError::MissingTextureAtlas`] for no `TextureAtlasImpl`
/// class, [`TextureAtlasParseError::UnsupportedObjectStreamVersion`] or
/// [`TextureAtlasParseError::UnsupportedTextureAtlasVersion`] for a version
/// this reader does not accept, and
/// [`TextureAtlasParseError::UnexpectedElement`],
/// [`TextureAtlasParseError::UnexpectedAttribute`] or
/// [`TextureAtlasParseError::UnexpectedTypeId`] when an element, attribute
/// value or reflected type id is not the one the layout requires.
///
/// Field-level failures are
/// [`TextureAtlasParseError::MissingAttribute`],
/// [`TextureAtlasParseError::InvalidInteger`],
/// [`TextureAtlasParseError::InvalidUuid`],
/// [`TextureAtlasParseError::MissingAtlasWidth`],
/// [`TextureAtlasParseError::MissingAtlasHeight`],
/// [`TextureAtlasParseError::MissingRegionName`],
/// [`TextureAtlasParseError::MissingCoordinates`] and
/// [`TextureAtlasParseError::MissingPairValue`]. Region rectangles add
/// [`TextureAtlasParseError::CoordinateOutsidePair`] for a coordinate outside
/// an `AZStd::pair`, [`TextureAtlasParseError::MissingCoordinate`],
/// [`TextureAtlasParseError::NegativeCoordinate`] for a negative value, and
/// [`TextureAtlasParseError::CoordinateOverflow`] when base plus size exceeds
/// `u32`.
pub fn visit_texture_atlas_index<'a, F>(
    bytes: &'a [u8],
    mut visitor: F,
) -> Result<TextureAtlasStats, TextureAtlasParseError>
where
    F: for<'region> FnMut(TextureAtlasRegionRef<'region>),
{
    let xml = str::from_utf8(bytes)?;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut state = TextureAtlasState::default();

    loop {
        match reader.read_event()? {
            Event::Start(event) => {
                let element = state.visit_start(&reader, &event)?;
                state.stack.push(element);
            }
            Event::Empty(event) => {
                let element = state.visit_start(&reader, &event)?;
                state.visit_end(element, &mut visitor)?;
            }
            Event::End(_) => {
                let element = state.stack.pop().unwrap_or(ElementKind::Other);
                state.visit_end(element, &mut visitor)?;
            }
            Event::Eof => break,
            Event::Decl(_)
            | Event::PI(_)
            | Event::DocType(_)
            | Event::Text(_)
            | Event::CData(_)
            | Event::Comment(_)
            | Event::GeneralRef(_) => {}
        }
    }

    state.finish()
}

#[derive(Debug, Default)]
struct TextureAtlasState {
    stack: Vec<ElementKind>,
    object_stream_version: Option<u32>,
    texture_atlas_version: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    regions: usize,
    pair: Option<PairState>,
}

impl TextureAtlasState {
    /// Classify an opening element against the current stack.
    ///
    /// Regions are only emitted on the closing event, so this half of the walk
    /// never calls the visitor.
    fn visit_start(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<ElementKind, TextureAtlasParseError> {
        let parent = self.stack.last().copied();
        match parent {
            None => self.visit_object_stream(reader, event),
            Some(ElementKind::ObjectStream) => self.visit_object_stream_child(reader, event),
            Some(ElementKind::TextureAtlas) => self.visit_texture_atlas_child(reader, event),
            Some(ElementKind::CoordinatePairs) => self.visit_coordinate_pairs_child(reader, event),
            Some(ElementKind::Pair) => self.visit_pair_child(reader, event),
            Some(ElementKind::Coordinates) => self.visit_coordinates_child(reader, event),
            Some(ElementKind::Other) => Ok(ElementKind::Other),
        }
    }

    fn visit_end<F>(
        &mut self,
        element: ElementKind,
        visitor: &mut F,
    ) -> Result<(), TextureAtlasParseError>
    where
        F: for<'region> FnMut(TextureAtlasRegionRef<'region>),
    {
        match element {
            ElementKind::Coordinates => self.emit_pair(visitor),
            ElementKind::Pair => {
                let Some(pair) = self.pair.take() else {
                    return Ok(());
                };
                if pair.emitted {
                    Ok(())
                } else {
                    Err(TextureAtlasParseError::MissingPairValue)
                }
            }
            ElementKind::ObjectStream
            | ElementKind::TextureAtlas
            | ElementKind::CoordinatePairs
            | ElementKind::Other => Ok(()),
        }
    }

    fn visit_object_stream(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<ElementKind, TextureAtlasParseError> {
        ensure_element(b"<root>", OBJECT_STREAM, event.local_name().as_ref())?;
        let version = required_u32_attr(reader, event, b"version", "ObjectStream::version")?;
        if version != TEXTURE_ATLAS_OBJECT_STREAM_VERSION {
            return Err(TextureAtlasParseError::UnsupportedObjectStreamVersion {
                found: version,
                expected: TEXTURE_ATLAS_OBJECT_STREAM_VERSION,
            });
        }
        self.object_stream_version = Some(version);
        Ok(ElementKind::ObjectStream)
    }

    fn visit_object_stream_child(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<ElementKind, TextureAtlasParseError> {
        ensure_element(OBJECT_STREAM, CLASS, event.local_name().as_ref())?;
        ensure_attr_bytes(reader, event, b"name", TEXTURE_ATLAS_IMPL)?;
        ensure_attr_uuid(reader, event, TEXTURE_ATLAS_IMPL_TYPE_ID)?;

        let version = required_u32_attr(reader, event, b"version", "TextureAtlasImpl::version")?;
        if !(1..=TEXTURE_ATLAS_IMPL_VERSION).contains(&version) {
            return Err(TextureAtlasParseError::UnsupportedTextureAtlasVersion {
                found: version,
                expected: TEXTURE_ATLAS_IMPL_VERSION,
            });
        }

        self.texture_atlas_version = Some(version);
        Ok(ElementKind::TextureAtlas)
    }

    fn visit_texture_atlas_child(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<ElementKind, TextureAtlasParseError> {
        ensure_element(TEXTURE_ATLAS_IMPL, CLASS, event.local_name().as_ref())?;
        let name = attr_value(reader, event, &[b"name"])?;
        let field = attr_value(reader, event, &[b"field"])?;

        match (name.as_deref(), field.as_deref()) {
            (Some(name), Some("Coordinate Pairs")) if name.as_bytes() == UNORDERED_MAP => {
                Ok(ElementKind::CoordinatePairs)
            }
            (Some(name), Some("Width")) if name.as_bytes() == INT => {
                self.width = Some(required_u32_attr(reader, event, b"value", "Width")?);
                Ok(ElementKind::Other)
            }
            (Some(name), Some("Height")) if name.as_bytes() == INT => {
                self.height = Some(required_u32_attr(reader, event, b"value", "Height")?);
                Ok(ElementKind::Other)
            }
            _ => Ok(ElementKind::Other),
        }
    }

    fn visit_coordinate_pairs_child(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<ElementKind, TextureAtlasParseError> {
        ensure_element(UNORDERED_MAP, CLASS, event.local_name().as_ref())?;
        ensure_attr_bytes(reader, event, b"name", PAIR)?;
        ensure_attr_bytes(reader, event, b"field", b"element")?;
        self.pair = Some(PairState::default());
        Ok(ElementKind::Pair)
    }

    fn visit_pair_child(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<ElementKind, TextureAtlasParseError> {
        ensure_element(PAIR, CLASS, event.local_name().as_ref())?;
        let name = attr_value(reader, event, &[b"name"])?;
        let field = attr_value(reader, event, &[b"field"])?;

        match (name.as_deref(), field.as_deref()) {
            (Some(name), Some("value1")) if name.as_bytes() == STRING => {
                let value = required_attr_value(reader, event, b"value", "value1")?;
                self.current_pair()?.name = Some(value.into_owned());
                Ok(ElementKind::Other)
            }
            (Some(name), Some("value2")) if name.as_bytes() == ATLAS_COORDINATES => {
                ensure_attr_uuid(reader, event, ATLAS_COORDINATES_TYPE_ID)?;
                self.current_pair()?.coordinates = Some(AtlasCoordinatesBuilder::default());
                Ok(ElementKind::Coordinates)
            }
            _ => Ok(ElementKind::Other),
        }
    }

    fn visit_coordinates_child(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<ElementKind, TextureAtlasParseError> {
        ensure_element(ATLAS_COORDINATES, CLASS, event.local_name().as_ref())?;
        ensure_attr_bytes(reader, event, b"name", INT)?;
        let field = required_attr_value(reader, event, b"field", "AtlasCoordinates field")?;
        let value = required_i32_attr(reader, event, b"value", "AtlasCoordinates value")?;
        let value =
            u32::try_from(value).map_err(|_| TextureAtlasParseError::NegativeCoordinate {
                field: field.to_string(),
                value,
            })?;

        let coordinates = self
            .current_pair()?
            .coordinates
            .as_mut()
            .ok_or(TextureAtlasParseError::MissingCoordinates)?;
        match field.as_ref() {
            "Left" => coordinates.left = Some(value),
            "Top" => coordinates.top = Some(value),
            "Width" => coordinates.width = Some(value),
            "Height" => coordinates.height = Some(value),
            _ => {}
        }
        Ok(ElementKind::Other)
    }

    fn emit_pair<F>(&mut self, visitor: &mut F) -> Result<(), TextureAtlasParseError>
    where
        F: for<'region> FnMut(TextureAtlasRegionRef<'region>),
    {
        let pair = self.current_pair()?;
        let name = pair
            .name
            .take()
            .ok_or(TextureAtlasParseError::MissingRegionName)?;
        let coordinates = pair
            .coordinates
            .take()
            .ok_or(TextureAtlasParseError::MissingCoordinates)?;
        let rect = coordinates.rect()?;

        pair.emitted = true;
        self.regions += 1;
        visitor(TextureAtlasRegionRef { name: &name, rect });
        Ok(())
    }

    fn current_pair(&mut self) -> Result<&mut PairState, TextureAtlasParseError> {
        self.pair
            .as_mut()
            .ok_or(TextureAtlasParseError::CoordinateOutsidePair)
    }

    fn finish(self) -> Result<TextureAtlasStats, TextureAtlasParseError> {
        let object_stream_version = self
            .object_stream_version
            .ok_or(TextureAtlasParseError::MissingRoot)?;
        let texture_atlas_version = self
            .texture_atlas_version
            .ok_or(TextureAtlasParseError::MissingTextureAtlas)?;
        let width = self
            .width
            .ok_or(TextureAtlasParseError::MissingAtlasWidth)?;
        let height = self
            .height
            .ok_or(TextureAtlasParseError::MissingAtlasHeight)?;

        Ok(TextureAtlasStats {
            object_stream_version,
            texture_atlas_version,
            size: UVec2::new(width, height),
            regions: self.regions,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum ElementKind {
    ObjectStream,
    TextureAtlas,
    CoordinatePairs,
    Pair,
    Coordinates,
    Other,
}

#[derive(Debug, Default)]
struct PairState {
    name: Option<String>,
    coordinates: Option<AtlasCoordinatesBuilder>,
    emitted: bool,
}

#[derive(Debug, Default)]
struct AtlasCoordinatesBuilder {
    left: Option<u32>,
    top: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
}

impl AtlasCoordinatesBuilder {
    fn rect(self) -> Result<URect, TextureAtlasParseError> {
        let left = self
            .left
            .ok_or(TextureAtlasParseError::MissingCoordinate { field: "Left" })?;
        let top = self
            .top
            .ok_or(TextureAtlasParseError::MissingCoordinate { field: "Top" })?;
        let width = self
            .width
            .ok_or(TextureAtlasParseError::MissingCoordinate { field: "Width" })?;
        let height = self
            .height
            .ok_or(TextureAtlasParseError::MissingCoordinate { field: "Height" })?;
        let right = left
            .checked_add(width)
            .ok_or(TextureAtlasParseError::CoordinateOverflow {
                field: "Width",
                base: left,
                size: width,
            })?;
        let bottom = top
            .checked_add(height)
            .ok_or(TextureAtlasParseError::CoordinateOverflow {
                field: "Height",
                base: top,
                size: height,
            })?;
        Ok(URect {
            min: UVec2::new(left, top),
            max: UVec2::new(right, bottom),
        })
    }
}

fn ensure_element(
    parent: &[u8],
    expected: &[u8],
    actual: &[u8],
) -> Result<(), TextureAtlasParseError> {
    if actual == expected {
        Ok(())
    } else {
        Err(TextureAtlasParseError::UnexpectedElement {
            parent: String::from_utf8_lossy(parent).into_owned(),
            child: String::from_utf8_lossy(actual).into_owned(),
        })
    }
}

fn ensure_attr_bytes(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    expected: &[u8],
) -> Result<(), TextureAtlasParseError> {
    let value = required_attr_value(reader, event, key, attr_label(key))?;
    if value.as_bytes() == expected {
        Ok(())
    } else {
        Err(TextureAtlasParseError::UnexpectedAttribute {
            name: String::from_utf8_lossy(key).into_owned(),
            expected: String::from_utf8_lossy(expected).into_owned(),
            found: value.into_owned(),
        })
    }
}

fn attr_label(key: &[u8]) -> &'static str {
    match key {
        b"name" => "name",
        b"field" => "field",
        b"type" => "type",
        b"value" => "value",
        b"version" => "version",
        _ => "attribute",
    }
}

fn ensure_attr_uuid(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    expected: Uuid,
) -> Result<(), TextureAtlasParseError> {
    let found = required_uuid_attr(reader, event, b"type", "type")?;
    if found == expected {
        Ok(())
    } else {
        Err(TextureAtlasParseError::UnexpectedTypeId { expected, found })
    }
}

fn attr_value<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
    keys: &[&[u8]],
) -> Result<Option<std::borrow::Cow<'a, str>>, TextureAtlasParseError> {
    for attribute in event.attributes() {
        let attribute = attribute?;
        if keys
            .iter()
            .any(|key| attribute.key.as_ref().eq_ignore_ascii_case(key))
        {
            return Ok(Some(attribute.decoded_and_normalized_value(
                quick_xml::XmlVersion::default(),
                reader.decoder(),
            )?));
        }
    }
    Ok(None)
}

fn required_attr_value<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
    key: &[u8],
    name: &'static str,
) -> Result<std::borrow::Cow<'a, str>, TextureAtlasParseError> {
    attr_value(reader, event, &[key])?.ok_or(TextureAtlasParseError::MissingAttribute { name })
}

fn required_u32_attr(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<u32, TextureAtlasParseError> {
    let value = required_attr_value(reader, event, key, name)?;
    parse_u32(name, value.as_ref())
}

fn required_i32_attr(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<i32, TextureAtlasParseError> {
    let value = required_attr_value(reader, event, key, name)?;
    parse_i32(name, value.as_ref())
}

fn required_uuid_attr(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<Uuid, TextureAtlasParseError> {
    let value = required_attr_value(reader, event, key, name)?;
    let trimmed = value.trim().trim_start_matches('{').trim_end_matches('}');
    Uuid::parse_str(trimmed).map_err(|source| TextureAtlasParseError::InvalidUuid {
        name,
        value: value.into_owned(),
        source,
    })
}

fn parse_u32(name: &'static str, value: &str) -> Result<u32, TextureAtlasParseError> {
    value
        .trim()
        .parse()
        .map_err(|source| TextureAtlasParseError::InvalidInteger {
            name,
            value: value.to_string(),
            source,
        })
}

fn parse_i32(name: &'static str, value: &str) -> Result<i32, TextureAtlasParseError> {
    value
        .trim()
        .parse()
        .map_err(|source| TextureAtlasParseError::InvalidInteger {
            name,
            value: value.to_string(),
            source,
        })
}

/// `TextureAtlas` parse errors.
#[derive(Debug, Error)]
pub enum TextureAtlasParseError {
    #[error("expected ObjectStream root")]
    MissingRoot,
    #[error("missing TextureAtlasImpl class")]
    MissingTextureAtlas,
    #[error("missing atlas Width field")]
    MissingAtlasWidth,
    #[error("missing atlas Height field")]
    MissingAtlasHeight,
    #[error("unsupported ObjectStream version {found}, expected {expected}")]
    UnsupportedObjectStreamVersion { found: u32, expected: u32 },
    #[error("unsupported TextureAtlasImpl version {found}, expected <= {expected}")]
    UnsupportedTextureAtlasVersion { found: u32, expected: u32 },
    #[error("unexpected element `{child}` under `{parent}`")]
    UnexpectedElement { parent: String, child: String },
    #[error("unexpected `{name}` attribute `{found}`, expected `{expected}`")]
    UnexpectedAttribute {
        name: String,
        expected: String,
        found: String,
    },
    #[error("unexpected type id `{found}`, expected `{expected}`")]
    UnexpectedTypeId { expected: Uuid, found: Uuid },
    #[error("missing `{name}` attribute")]
    MissingAttribute { name: &'static str },
    #[error("invalid integer `{value}` in `{name}`")]
    InvalidInteger {
        name: &'static str,
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error("invalid UUID `{value}` in `{name}`")]
    InvalidUuid {
        name: &'static str,
        value: String,
        #[source]
        source: uuid::Error,
    },
    #[error("coordinate element appeared outside an AZStd::pair")]
    CoordinateOutsidePair,
    #[error("atlas coordinate `{field}` is negative: {value}")]
    NegativeCoordinate { field: String, value: i32 },
    #[error("atlas coordinate `{field}` is missing")]
    MissingCoordinate { field: &'static str },
    #[error("atlas coordinate `{field}` overflowed: {base} + {size}")]
    CoordinateOverflow {
        field: &'static str,
        base: u32,
        size: u32,
    },
    #[error("missing atlas region name")]
    MissingRegionName,
    #[error("missing AtlasCoordinates value")]
    MissingCoordinates,
    #[error("missing value1/value2 pair data")]
    MissingPairValue,
    #[error("xml parse error")]
    Xml(#[from] quick_xml::Error),
    #[error("xml attribute error")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    #[error("asset is not utf-8")]
    Utf8(#[from] str::Utf8Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visits_texture_atlas_regions() {
        let xml = br#"<ObjectStream version="3">
            <Class name="TextureAtlasImpl" version="1" type="{2CA51C61-1B5F-4480-A257-F28D8944AA35}">
                <Class name="AZStd::unordered_map" field="Coordinate Pairs" type="{23D80BE7-76FE-5C35-B4EA-A4492B2B058C}">
                    <Class name="AZStd::pair" field="element" type="{657F8527-0117-54A0-91E7-6E67A7464BC5}">
                        <Class name="AZStd::string" field="value1" value="WhiteTexture" type="{03AAAB3F-5C47-5A66-9EBC-D5FA4DB353C9}"/>
                        <Class name="AtlasCoordinates" field="value2" version="1" type="{FC5D6A60-1056-4F6C-96F7-6A47912F8A35}">
                            <Class name="int" field="Left" value="17" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
                            <Class name="int" field="Top" value="0" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
                            <Class name="int" field="Width" value="4" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
                            <Class name="int" field="Height" value="4" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
                        </Class>
                    </Class>
                </Class>
                <Class name="int" field="Width" value="24" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
                <Class name="int" field="Height" value="12" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/>
            </Class>
        </ObjectStream>"#;

        let mut regions = Vec::new();
        let stats = visit_texture_atlas_index(xml, |region| {
            regions.push((region.name.to_string(), region.rect));
        })
        .unwrap();

        assert_eq!(
            stats,
            TextureAtlasStats {
                object_stream_version: 3,
                texture_atlas_version: 1,
                size: UVec2::new(24, 12),
                regions: 1,
            }
        );
        assert_eq!(
            stats.to_string(),
            "24x12, 1 regions, ObjectStream v3, TextureAtlasImpl v1"
        );
        assert_eq!(regions[0].0, "WhiteTexture");
        assert_eq!(
            regions[0].1,
            URect {
                min: UVec2::new(17, 0),
                max: UVec2::new(21, 4),
            }
        );

        let mut totals = TextureAtlasTotals::default();
        totals.add_stats(stats);
        assert_eq!(
            totals,
            TextureAtlasTotals {
                files: 1,
                regions: 1,
                pixels: 288,
            }
        );
        assert_eq!(
            totals.to_string(),
            "  files: 1\n  regions: 1\n  pixels: 288"
        );

        let row = inspect_texture_atlas_index_file("ui/hud.texatlasidx", xml).unwrap();
        let mut inspection = TextureAtlasInspection::default();
        inspection.add_file_summary(row);
        assert_eq!(
            inspection.report(20).to_string(),
            "ui/hud.texatlasidx: 24x12, 1 regions, ObjectStream v3, TextureAtlasImpl v1\n  files: 1\n  regions: 1\n  pixels: 288"
        );

        let path = std::env::temp_dir().join(format!(
            "az-rs-texture-atlas-{}-hud.texatlasidx",
            std::process::id()
        ));
        std::fs::write(&path, xml).expect("write texture atlas");
        let inspection =
            inspect_texture_atlas_index_files([&path]).expect("inspect texture atlases");
        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.regions, 1);
        assert_eq!(inspection.totals.pixels, 288);
        std::fs::remove_file(path).expect("remove texture atlas");

        assert!(is_texture_atlas_index_name("ui/foo.texatlasidx"));
        assert!(is_texture_atlas_index_name("ui/foo.TEXATLASIDX"));
        assert!(!is_texture_atlas_index_name("ui/foo.xml"));
    }
}
