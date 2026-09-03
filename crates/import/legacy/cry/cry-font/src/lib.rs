//! `CryFont` face parsing.
//!
//! Follows Lumberyard's `dev/Code/CryEngine/CryFont/FontRenderer.cpp`.

pub mod builder;
pub mod source_transform;

pub use source_transform::{FontFaceSourceTransform, FontFaceSourceTransformError};

use std::{
    fmt, io,
    path::{Path, PathBuf},
    str,
};

use thiserror::Error;

pub const SFNT_OFFSET_TABLE_SIZE: usize = 12;
pub const SFNT_TABLE_RECORD_SIZE: usize = 16;
pub const TTC_HEADER_SIZE: usize = 12;
pub const TTC_OFFSET_SIZE: usize = 4;

pub const TAG_TRUE_TYPE: OpenTypeTag = OpenTypeTag::new(*b"\0\x01\0\0");
pub const TAG_OPEN_TYPE_CFF: OpenTypeTag = OpenTypeTag::new(*b"OTTO");
pub const TAG_APPLE_TRUE_TYPE: OpenTypeTag = OpenTypeTag::new(*b"true");
pub const TAG_APPLE_TYPE1: OpenTypeTag = OpenTypeTag::new(*b"typ1");
pub const TAG_TRUE_TYPE_COLLECTION: OpenTypeTag = OpenTypeTag::new(*b"ttcf");

pub const FONT_FACE_EXTENSIONS: [&str; 2] = ["ttf", "otf"];

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FontFaceInspectionError {
    #[error("read {path:?}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse font face {path:?}: {source}")]
    Parse {
        path: PathBuf,
        source: OpenTypeParseError,
    },
}

/// A single sfnt face or a TrueType collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFaceData<'a> {
    Single(OpenTypeFontFace<'a>),
    Collection(TrueTypeCollection<'a>),
}

impl<'a> FontFaceData<'a> {
    /// Parse a font face payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the sfnt header, table directory, collection
    /// directory, or table ranges are invalid.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, OpenTypeParseError> {
        let tag = OpenTypeTag(read_array(bytes, 0)?);
        if tag == TAG_TRUE_TYPE_COLLECTION {
            TrueTypeCollection::parse(bytes).map(Self::Collection)
        } else {
            OpenTypeFontFace::parse(bytes).map(Self::Single)
        }
    }

    #[inline]
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        match self {
            Self::Single(face) => face.bytes(),
            Self::Collection(collection) => collection.bytes(),
        }
    }

    #[inline]
    #[must_use]
    pub const fn face_count(self) -> u32 {
        match self {
            Self::Single(_) => 1,
            Self::Collection(collection) => collection.len(),
        }
    }

    #[must_use]
    pub fn table_count(self) -> usize {
        match self {
            Self::Single(face) => face.len() as usize,
            Self::Collection(collection) => collection
                .faces()
                .map(|face| face.len() as usize)
                .sum::<usize>(),
        }
    }

    #[inline]
    #[must_use]
    pub fn summary(self) -> FontFaceSummary {
        FontFaceSummary::from_data(self)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FontFaceSummary {
    pub collections: usize,
    pub faces: usize,
    pub tables: usize,
    pub true_type_faces: usize,
    pub open_type_cff_faces: usize,
    pub apple_true_type_faces: usize,
    pub apple_type1_faces: usize,
}

impl FontFaceSummary {
    #[must_use]
    pub fn from_data(data: FontFaceData<'_>) -> Self {
        match data {
            FontFaceData::Single(face) => {
                let mut summary = Self::default();
                summary.add_face(face);
                summary
            }
            FontFaceData::Collection(collection) => {
                let mut summary = Self {
                    collections: 1,
                    ..Self::default()
                };
                for face in collection.faces() {
                    summary.add_face(face);
                }
                summary
            }
        }
    }

    pub fn add_face(&mut self, face: OpenTypeFontFace<'_>) {
        self.faces += 1;
        self.tables += usize::from(face.len());
        match face.scaler() {
            OpenTypeScalerKind::TrueType => self.true_type_faces += 1,
            OpenTypeScalerKind::OpenTypeCff => self.open_type_cff_faces += 1,
            OpenTypeScalerKind::AppleTrueType => self.apple_true_type_faces += 1,
            OpenTypeScalerKind::AppleType1 => self.apple_type1_faces += 1,
        }
    }

    #[must_use]
    pub const fn is_collection(self) -> bool {
        self.collections != 0
    }

    #[must_use]
    pub fn label(self) -> String {
        if self.is_collection() {
            format!(
                "TrueType collection, {} faces, {} tables",
                self.faces, self.tables
            )
        } else {
            let kind = if self.true_type_faces == 1 {
                OpenTypeScalerKind::TrueType.as_str()
            } else if self.open_type_cff_faces == 1 {
                OpenTypeScalerKind::OpenTypeCff.as_str()
            } else if self.apple_true_type_faces == 1 {
                OpenTypeScalerKind::AppleTrueType.as_str()
            } else {
                OpenTypeScalerKind::AppleType1.as_str()
            };
            format!("{kind} face, {} tables", self.tables)
        }
    }
}

impl fmt::Display for FontFaceSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// Summarises a font payload: face count, table count and scaler flavour.
///
/// # Errors
///
/// Returns any error [`FontFaceData::parse`] returns —
/// [`OpenTypeParseError::TooShort`] when the buffer ends inside the sfnt
/// header or table directory, [`OpenTypeParseError::UnsupportedScaler`] for a
/// scaler tag that is neither TrueType nor `OTTO`,
/// [`OpenTypeParseError::EmptyTableDirectory`] or
/// [`OpenTypeParseError::EmptyCollection`] for a face or collection with no
/// entries, [`OpenTypeParseError::ExpectedCollection`] and
/// [`OpenTypeParseError::UnsupportedCollectionVersion`] for a malformed `ttcf`
/// header, [`OpenTypeParseError::TableOutOfBounds`] when a directory entry
/// points past the end of the payload, and
/// [`OpenTypeParseError::OffsetOverflow`] when a count or offset does not fit
/// in `usize`.
pub fn summarize_font_face(bytes: &[u8]) -> Result<FontFaceSummary, OpenTypeParseError> {
    FontFaceData::parse(bytes).map(FontFaceData::summary)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FontFaceTotals {
    pub files: usize,
    pub single_faces: usize,
    pub collections: usize,
    pub faces: usize,
    pub tables: usize,
    pub true_type_faces: usize,
    pub open_type_cff_faces: usize,
    pub apple_true_type_faces: usize,
    pub apple_type1_faces: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceFileSummary {
    pub source: String,
    pub summary: FontFaceSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FontFaceInspection {
    pub rows: Vec<FontFaceFileSummary>,
    pub totals: FontFaceTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct FontFaceInspectionReport<'a> {
    inspection: &'a FontFaceInspection,
    limit: usize,
}

impl FontFaceTotals {
    pub const fn add_summary(&mut self, summary: FontFaceSummary) {
        self.files += 1;
        self.collections += summary.collections;
        if !summary.is_collection() {
            self.single_faces += 1;
        }
        self.faces += summary.faces;
        self.tables += summary.tables;
        self.true_type_faces += summary.true_type_faces;
        self.open_type_cff_faces += summary.open_type_cff_faces;
        self.apple_true_type_faces += summary.apple_true_type_faces;
        self.apple_type1_faces += summary.apple_type1_faces;
    }
}

impl FontFaceInspection {
    pub fn add_file_summary(&mut self, row: FontFaceFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> FontFaceInspectionReport<'_> {
        FontFaceInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for FontFaceTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  single faces: {}", self.single_faces)?;
        writeln!(f, "  collections: {}", self.collections)?;
        writeln!(f, "  faces: {}", self.faces)?;
        writeln!(f, "  tables: {}", self.tables)?;
        writeln!(f, "  TrueType faces: {}", self.true_type_faces)?;
        writeln!(f, "  OpenType CFF faces: {}", self.open_type_cff_faces)?;
        writeln!(f, "  Apple TrueType faces: {}", self.apple_true_type_faces)?;
        writeln!(f, "  Apple Type 1 faces: {}", self.apple_type1_faces)
    }
}

impl fmt::Display for FontFaceInspectionReport<'_> {
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

/// Summarises one font payload, labelling the row with `path`.
///
/// `path` is only the display label; it is not read from disk.
///
/// # Errors
///
/// Returns any error [`summarize_font_face`] returns — the
/// [`OpenTypeParseError`] variants for a truncated payload, an unsupported
/// scaler or collection version, an empty table directory or collection, or a
/// table range that falls outside the payload.
pub fn inspect_font_face_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<FontFaceFileSummary, OpenTypeParseError> {
    Ok(FontFaceFileSummary {
        source: path.as_ref().display().to_string(),
        summary: summarize_font_face(bytes)?,
    })
}

/// Reads a font file from disk and summarises it.
///
/// # Errors
///
/// Returns [`FontFaceInspectionError::Read`] if `path` cannot be read (missing
/// file, permissions), or [`FontFaceInspectionError::Parse`] wrapping the
/// [`OpenTypeParseError`] from a malformed font. Both variants carry the
/// offending path.
pub fn inspect_font_face_path(
    path: impl AsRef<Path>,
) -> Result<FontFaceFileSummary, FontFaceInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| FontFaceInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_font_face_file(path, &bytes).map_err(|source| FontFaceInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Reads and summarises every font file in `paths`, accumulating totals.
///
/// Stops at the first failing path; earlier rows are discarded with it.
///
/// # Errors
///
/// Returns any error [`inspect_font_face_path`] returns for the first path
/// that fails — [`FontFaceInspectionError::Read`] for an unreadable file, or
/// [`FontFaceInspectionError::Parse`] for a malformed font.
pub fn inspect_font_face_files<I, P>(
    paths: I,
) -> Result<FontFaceInspection, FontFaceInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = FontFaceInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_font_face_path(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub fn is_font_face_extension(extension: &str) -> bool {
    FONT_FACE_EXTENSIONS
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
}

#[must_use]
pub fn is_font_face_name(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_font_face_extension)
}

#[must_use]
pub fn is_font_face_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_font_face_extension)
}

/// A borrowed OpenType sfnt face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenTypeFontFace<'a> {
    bytes: &'a [u8],
    offset: usize,
    header: OpenTypeFaceHeader,
}

impl<'a> OpenTypeFontFace<'a> {
    /// Parse a single sfnt face.
    ///
    /// # Errors
    ///
    /// Returns an error when the scaler type, table directory, or table ranges
    /// are invalid.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, OpenTypeParseError> {
        Self::parse_at(bytes, 0)
    }

    fn parse_at(bytes: &'a [u8], offset: usize) -> Result<Self, OpenTypeParseError> {
        let header = OpenTypeFaceHeader::parse_at(bytes, offset)?;
        if header.num_tables == 0 {
            return Err(OpenTypeParseError::EmptyTableDirectory);
        }
        let records_start = checked_add(offset, SFNT_OFFSET_TABLE_SIZE)?;
        let records_end = checked_section_end(
            records_start,
            u32::from(header.num_tables),
            SFNT_TABLE_RECORD_SIZE,
        )?;
        ensure_len(bytes, records_end)?;

        let face = Self {
            bytes,
            offset,
            header,
        };
        for record in face.tables() {
            let record = record?;
            ensure_table_range(bytes, record.tag, record.offset, record.length)?;
        }
        Ok(face)
    }

    #[inline]
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub const fn offset(self) -> usize {
        self.offset
    }

    #[inline]
    #[must_use]
    pub const fn header(self) -> OpenTypeFaceHeader {
        self.header
    }

    #[inline]
    #[must_use]
    pub const fn scaler(self) -> OpenTypeScalerKind {
        self.header.scaler
    }

    #[inline]
    #[must_use]
    pub const fn len(self) -> u16 {
        self.header.num_tables
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.header.num_tables == 0
    }

    #[inline]
    #[must_use]
    pub const fn tables(self) -> OpenTypeTableRecords<'a> {
        OpenTypeTableRecords {
            bytes: self.bytes,
            position: self.offset + SFNT_OFFSET_TABLE_SIZE,
            remaining: self.header.num_tables,
        }
    }

    #[must_use]
    pub fn table(self, tag: OpenTypeTag) -> Option<OpenTypeTableRecord<'a>> {
        self.tables()
            .filter_map(Result::ok)
            .find(|record| record.tag == tag)
    }
}

/// sfnt face header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenTypeFaceHeader {
    pub scaler: OpenTypeScalerKind,
    pub num_tables: u16,
    pub search_range: u16,
    pub entry_selector: u16,
    pub range_shift: u16,
}

impl OpenTypeFaceHeader {
    fn parse_at(bytes: &[u8], offset: usize) -> Result<Self, OpenTypeParseError> {
        let scaler_tag = OpenTypeTag(read_array(bytes, offset)?);
        let Some(scaler) = OpenTypeScalerKind::from_tag(scaler_tag) else {
            return Err(OpenTypeParseError::UnsupportedScaler { tag: scaler_tag });
        };

        Ok(Self {
            scaler,
            num_tables: read_u16_be_at(bytes, offset + 4)?,
            search_range: read_u16_be_at(bytes, offset + 6)?,
            entry_selector: read_u16_be_at(bytes, offset + 8)?,
            range_shift: read_u16_be_at(bytes, offset + 10)?,
        })
    }
}

/// sfnt scaler type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OpenTypeScalerKind {
    TrueType,
    OpenTypeCff,
    AppleTrueType,
    AppleType1,
}

impl OpenTypeScalerKind {
    #[must_use]
    pub const fn from_tag(tag: OpenTypeTag) -> Option<Self> {
        if tag.eq(TAG_TRUE_TYPE) {
            Some(Self::TrueType)
        } else if tag.eq(TAG_OPEN_TYPE_CFF) {
            Some(Self::OpenTypeCff)
        } else if tag.eq(TAG_APPLE_TRUE_TYPE) {
            Some(Self::AppleTrueType)
        } else if tag.eq(TAG_APPLE_TYPE1) {
            Some(Self::AppleType1)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn tag(self) -> OpenTypeTag {
        match self {
            Self::TrueType => TAG_TRUE_TYPE,
            Self::OpenTypeCff => TAG_OPEN_TYPE_CFF,
            Self::AppleTrueType => TAG_APPLE_TRUE_TYPE,
            Self::AppleType1 => TAG_APPLE_TYPE1,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TrueType => "TrueType",
            Self::OpenTypeCff => "OpenType CFF",
            Self::AppleTrueType => "Apple TrueType",
            Self::AppleType1 => "Apple Type 1",
        }
    }
}

/// Four-byte OpenType tag.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct OpenTypeTag([u8; 4]);

impl OpenTypeTag {
    #[must_use]
    pub const fn new(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 4] {
        self.0
    }

    const fn eq(self, other: Self) -> bool {
        let lhs = self.0;
        let rhs = other.0;
        lhs[0] == rhs[0] && lhs[1] == rhs[1] && lhs[2] == rhs[2] && lhs[3] == rhs[3]
    }
}

impl fmt::Debug for OpenTypeTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for OpenTypeTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match str::from_utf8(&self.0) {
            Ok(value) if value.chars().all(|ch| !ch.is_control()) => f.write_str(value),
            _ => write!(
                f,
                "{:02x}{:02x}{:02x}{:02x}",
                self.0[0], self.0[1], self.0[2], self.0[3]
            ),
        }
    }
}

/// One sfnt table record with borrowed table bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenTypeTableRecord<'a> {
    pub tag: OpenTypeTag,
    pub checksum: u32,
    pub offset: u32,
    pub length: u32,
    data: &'a [u8],
}

impl<'a> OpenTypeTableRecord<'a> {
    #[inline]
    #[must_use]
    pub const fn data(self) -> &'a [u8] {
        self.data
    }
}

/// Borrowed iterator over sfnt table records.
// Not `Copy`: a copyable iterator silently restarts when passed by value.
#[derive(Debug, Clone)]
pub struct OpenTypeTableRecords<'a> {
    bytes: &'a [u8],
    position: usize,
    remaining: u16,
}

impl<'a> Iterator for OpenTypeTableRecords<'a> {
    type Item = Result<OpenTypeTableRecord<'a>, OpenTypeParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let start = self.position;
        self.position += SFNT_TABLE_RECORD_SIZE;
        self.remaining -= 1;

        Some(parse_table_record(self.bytes, start))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.remaining as usize;
        (len, Some(len))
    }
}

impl ExactSizeIterator for OpenTypeTableRecords<'_> {}

fn parse_table_record(
    bytes: &[u8],
    offset: usize,
) -> Result<OpenTypeTableRecord<'_>, OpenTypeParseError> {
    let tag = OpenTypeTag(read_array(bytes, offset)?);
    let checksum = read_u32_be_at(bytes, offset + 4)?;
    let table_offset = read_u32_be_at(bytes, offset + 8)?;
    let length = read_u32_be_at(bytes, offset + 12)?;
    let data = table_data(bytes, table_offset, length, tag)?;
    Ok(OpenTypeTableRecord {
        tag,
        checksum,
        offset: table_offset,
        length,
        data,
    })
}

/// A borrowed TrueType collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrueTypeCollection<'a> {
    bytes: &'a [u8],
    header: TrueTypeCollectionHeader,
}

impl<'a> TrueTypeCollection<'a> {
    /// Parse a TrueType collection.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection header, face offsets, or member
    /// faces are invalid.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, OpenTypeParseError> {
        let header = TrueTypeCollectionHeader::parse(bytes)?;
        if header.face_count == 0 {
            return Err(OpenTypeParseError::EmptyCollection);
        }
        let offsets_end = checked_section_end(TTC_HEADER_SIZE, header.face_count, TTC_OFFSET_SIZE)?;
        ensure_len(bytes, offsets_end)?;

        let collection = Self { bytes, header };
        for offset in collection.face_offsets() {
            OpenTypeFontFace::parse_at(bytes, offset?)?;
        }
        Ok(collection)
    }

    #[inline]
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub const fn header(self) -> TrueTypeCollectionHeader {
        self.header
    }

    #[inline]
    #[must_use]
    pub const fn len(self) -> u32 {
        self.header.face_count
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.header.face_count == 0
    }

    #[inline]
    #[must_use]
    pub const fn face_offsets(self) -> TrueTypeCollectionOffsets<'a> {
        TrueTypeCollectionOffsets {
            bytes: self.bytes,
            position: TTC_HEADER_SIZE,
            remaining: self.header.face_count,
        }
    }

    #[inline]
    #[must_use]
    pub const fn faces(self) -> TrueTypeCollectionFaces<'a> {
        TrueTypeCollectionFaces {
            bytes: self.bytes,
            offsets: self.face_offsets(),
        }
    }
}

/// TrueType collection header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrueTypeCollectionHeader {
    pub version: TrueTypeCollectionVersion,
    pub face_count: u32,
}

impl TrueTypeCollectionHeader {
    fn parse(bytes: &[u8]) -> Result<Self, OpenTypeParseError> {
        let tag = OpenTypeTag(read_array(bytes, 0)?);
        if tag != TAG_TRUE_TYPE_COLLECTION {
            return Err(OpenTypeParseError::ExpectedCollection { tag });
        }
        let version_raw = read_u32_be_at(bytes, 4)?;
        let Some(version) = TrueTypeCollectionVersion::from_native_value(version_raw) else {
            return Err(OpenTypeParseError::UnsupportedCollectionVersion {
                version: version_raw,
            });
        };
        Ok(Self {
            version,
            face_count: read_u32_be_at(bytes, 8)?,
        })
    }
}

/// TrueType collection version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum TrueTypeCollectionVersion {
    V1 = 0x0001_0000,
    V2 = 0x0002_0000,
}

impl TrueTypeCollectionVersion {
    #[must_use]
    pub const fn from_native_value(value: u32) -> Option<Self> {
        match value {
            0x0001_0000 => Some(Self::V1),
            0x0002_0000 => Some(Self::V2),
            _ => None,
        }
    }
}

/// Borrowed iterator over collection face offsets.
// Not `Copy`: a copyable iterator silently restarts when passed by value.
#[derive(Debug, Clone)]
pub struct TrueTypeCollectionOffsets<'a> {
    bytes: &'a [u8],
    position: usize,
    remaining: u32,
}

impl Iterator for TrueTypeCollectionOffsets<'_> {
    type Item = Result<usize, OpenTypeParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let offset = read_u32_be_at(self.bytes, self.position).and_then(|offset| {
            usize::try_from(offset).map_err(|_| OpenTypeParseError::OffsetOverflow)
        });
        self.position += TTC_OFFSET_SIZE;
        self.remaining -= 1;
        Some(offset)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = usize::try_from(self.remaining).unwrap_or(usize::MAX);
        (len, Some(len))
    }
}

/// Borrowed iterator over collection faces.
// Not `Copy`: a copyable iterator silently restarts when passed by value.
#[derive(Debug, Clone)]
pub struct TrueTypeCollectionFaces<'a> {
    bytes: &'a [u8],
    offsets: TrueTypeCollectionOffsets<'a>,
}

impl<'a> Iterator for TrueTypeCollectionFaces<'a> {
    type Item = OpenTypeFontFace<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let offset = self
            .offsets
            .next()?
            .expect("validated TrueType collection face offset");
        Some(
            OpenTypeFontFace::parse_at(self.bytes, offset)
                .expect("validated TrueType collection face"),
        )
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.offsets.size_hint()
    }
}

impl ExactSizeIterator for TrueTypeCollectionFaces<'_> {}

/// Error returned while parsing an OpenType face.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OpenTypeParseError {
    #[error("font payload is too short: need {needed} bytes, got {actual}")]
    TooShort { needed: usize, actual: usize },

    #[error("unsupported sfnt scaler tag {tag}")]
    UnsupportedScaler { tag: OpenTypeTag },

    #[error("sfnt table directory is empty")]
    EmptyTableDirectory,

    #[error("TrueType collection is empty")]
    EmptyCollection,

    #[error("expected TrueType collection tag, got {tag}")]
    ExpectedCollection { tag: OpenTypeTag },

    #[error("unsupported TrueType collection version 0x{version:08x}")]
    UnsupportedCollectionVersion { version: u32 },

    #[error("OpenType count or offset overflows usize")]
    OffsetOverflow,

    #[error("table {tag} range is out of bounds: need {needed} bytes, got {actual}")]
    TableOutOfBounds {
        tag: OpenTypeTag,
        needed: usize,
        actual: usize,
    },
}

fn table_data(
    bytes: &[u8],
    offset: u32,
    length: u32,
    tag: OpenTypeTag,
) -> Result<&[u8], OpenTypeParseError> {
    let start = usize::try_from(offset).map_err(|_| OpenTypeParseError::OffsetOverflow)?;
    let length = usize::try_from(length).map_err(|_| OpenTypeParseError::OffsetOverflow)?;
    let end = checked_add(start, length)?;
    bytes
        .get(start..end)
        .ok_or(OpenTypeParseError::TableOutOfBounds {
            tag,
            needed: end,
            actual: bytes.len(),
        })
}

fn ensure_table_range(
    bytes: &[u8],
    tag: OpenTypeTag,
    offset: u32,
    length: u32,
) -> Result<(), OpenTypeParseError> {
    let _ = table_data(bytes, offset, length, tag)?;
    Ok(())
}

const fn ensure_len(bytes: &[u8], needed: usize) -> Result<(), OpenTypeParseError> {
    if bytes.len() < needed {
        Err(OpenTypeParseError::TooShort {
            needed,
            actual: bytes.len(),
        })
    } else {
        Ok(())
    }
}

fn checked_add(start: usize, size: usize) -> Result<usize, OpenTypeParseError> {
    start
        .checked_add(size)
        .ok_or(OpenTypeParseError::OffsetOverflow)
}

fn checked_section_end(
    start: usize,
    count: u32,
    stride: usize,
) -> Result<usize, OpenTypeParseError> {
    let count = usize::try_from(count).map_err(|_| OpenTypeParseError::OffsetOverflow)?;
    count
        .checked_mul(stride)
        .and_then(|size| start.checked_add(size))
        .ok_or(OpenTypeParseError::OffsetOverflow)
}

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], OpenTypeParseError> {
    let end = offset
        .checked_add(N)
        .ok_or(OpenTypeParseError::OffsetOverflow)?;
    bytes
        .get(offset..end)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(OpenTypeParseError::TooShort {
            needed: end,
            actual: bytes.len(),
        })
}

fn read_u16_be_at(bytes: &[u8], offset: usize) -> Result<u16, OpenTypeParseError> {
    Ok(u16::from_be_bytes(read_array(bytes, offset)?))
}

fn read_u32_be_at(bytes: &[u8], offset: usize) -> Result<u32, OpenTypeParseError> {
    Ok(u32::from_be_bytes(read_array(bytes, offset)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::{
        LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceFormat,
        source_schema_type,
    };

    const TEST_FONT_FACE_SCHEMA: az_asset_builder::SourceSchemaType =
        source_schema_type::<TestFontFaceSource>();

    #[derive(SourceFormat)]
    #[source(schema = "az.test.FontFaceSource", ext = "ttf")]
    struct TestFontFaceSource;

    #[test]
    fn parses_single_true_type_face() {
        let bytes = font_with_scaler(*b"\0\x01\0\0");
        let data = FontFaceData::parse(&bytes).unwrap();
        let FontFaceData::Single(face) = data else {
            panic!("expected single face");
        };
        let tables = face.tables().collect::<Result<Vec<_>, _>>().unwrap();

        assert_eq!(face.scaler(), OpenTypeScalerKind::TrueType);
        assert_eq!(face.len(), 1);
        assert_eq!(tables[0].tag, OpenTypeTag::new(*b"head"));
        assert_eq!(tables[0].data(), b"abcd");
        assert_eq!(
            data.summary(),
            FontFaceSummary {
                faces: 1,
                tables: 1,
                true_type_faces: 1,
                ..FontFaceSummary::default()
            }
        );
        assert_eq!(data.summary().label(), "TrueType face, 1 tables");
        assert_eq!(data.summary().to_string(), "TrueType face, 1 tables");
    }

    #[test]
    fn parses_open_type_cff_face() {
        let bytes = font_with_scaler(*b"OTTO");
        let FontFaceData::Single(face) = FontFaceData::parse(&bytes).unwrap() else {
            panic!("expected single face");
        };

        assert_eq!(face.scaler(), OpenTypeScalerKind::OpenTypeCff);
    }

    #[test]
    fn parses_true_type_collection() {
        let face = font_with_scaler(*b"\0\x01\0\0");
        let mut bytes = Vec::new();
        bytes.extend(*b"ttcf");
        bytes.extend(0x0001_0000u32.to_be_bytes());
        bytes.extend(1u32.to_be_bytes());
        bytes.extend(16u32.to_be_bytes());
        bytes.extend(face);

        let FontFaceData::Collection(collection) = FontFaceData::parse(&bytes).unwrap() else {
            panic!("expected collection");
        };
        let faces = collection.faces().collect::<Vec<_>>();
        let summary = FontFaceSummary::from_data(FontFaceData::Collection(collection));

        assert_eq!(collection.len(), 1);
        assert_eq!(faces[0].len(), 1);
        assert_eq!(
            summary,
            FontFaceSummary {
                collections: 1,
                faces: 1,
                tables: 1,
                true_type_faces: 1,
                ..FontFaceSummary::default()
            }
        );
        assert_eq!(summary.label(), "TrueType collection, 1 faces, 1 tables");
    }

    #[test]
    fn tracks_font_face_totals_and_extensions() {
        let mut totals = FontFaceTotals::default();
        totals.add_summary(FontFaceSummary {
            faces: 1,
            tables: 1,
            true_type_faces: 1,
            ..FontFaceSummary::default()
        });
        totals.add_summary(FontFaceSummary {
            collections: 1,
            faces: 2,
            tables: 4,
            open_type_cff_faces: 2,
            ..FontFaceSummary::default()
        });

        assert_eq!(totals.files, 2);
        assert_eq!(totals.single_faces, 1);
        assert_eq!(totals.collections, 1);
        assert_eq!(totals.faces, 3);
        assert_eq!(totals.tables, 5);
        assert_eq!(
            totals.to_string(),
            "  files: 2\n  single faces: 1\n  collections: 1\n  faces: 3\n  tables: 5\n  TrueType faces: 1\n  OpenType CFF faces: 2\n  Apple TrueType faces: 0\n  Apple Type 1 faces: 0\n"
        );

        let face = font_with_scaler(*b"\0\x01\0\0");
        let row = inspect_font_face_file("fonts/foo.ttf", &face).unwrap();
        let mut inspection = FontFaceInspection::default();
        inspection.add_file_summary(row);
        assert_eq!(
            inspection.report(20).to_string(),
            "fonts/foo.ttf: TrueType face, 1 tables\n  files: 1\n  single faces: 1\n  collections: 0\n  faces: 1\n  tables: 1\n  TrueType faces: 1\n  OpenType CFF faces: 0\n  Apple TrueType faces: 0\n  Apple Type 1 faces: 0\n"
        );
        assert!(is_font_face_name("fonts/foo.ttf"));
        assert!(is_font_face_name("fonts/foo.OTF"));
        assert!(!is_font_face_name("fonts/foo.font"));
    }

    #[test]
    fn inspect_font_face_files_aggregates_file_results() {
        let path =
            std::env::temp_dir().join(format!("az-rs-cry-font-{}-foo.ttf", std::process::id()));
        std::fs::write(&path, font_with_scaler(*b"\0\x01\0\0")).expect("write font face");

        let inspection = inspect_font_face_files([&path]).expect("inspect font face files");

        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.single_faces, 1);
        assert_eq!(inspection.totals.tables, 1);

        std::fs::remove_file(path).expect("remove font face");
    }

    #[test]
    fn source_transform_marks_font_face_as_external_product_payload() {
        let bytes = font_with_scaler(*b"\0\x01\0\0");

        let output = FontFaceSourceTransform::new(TEST_FONT_FACE_SCHEMA)
            .transform(LegacySourceInput::new("LyShineUI/Fonts/Sans.ttf", &bytes))
            .unwrap();

        assert_eq!(
            output,
            LegacySourceOutput::external_product_payload(
                "ui/fonts/sans.ttf",
                TEST_FONT_FACE_SCHEMA,
                bytes
            )
        );
    }

    fn font_with_scaler(scaler: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(scaler);
        bytes.extend(1u16.to_be_bytes());
        bytes.extend(16u16.to_be_bytes());
        bytes.extend(0u16.to_be_bytes());
        bytes.extend(0u16.to_be_bytes());
        bytes.extend(*b"head");
        bytes.extend(0u32.to_be_bytes());
        bytes.extend(28u32.to_be_bytes());
        bytes.extend(4u32.to_be_bytes());
        bytes.extend(*b"abcd");
        bytes
    }
}
