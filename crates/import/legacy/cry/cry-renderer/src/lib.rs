//! Cry renderer asset parsers.
//!
//! Follows Lumberyard's `dev/Code/CryEngine/RenderDll`.

pub mod sky_light;
pub mod source_transform;

use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use az_asset_builder::normalize_source_path;
use thiserror::Error;

pub use source_transform::{
    DxorbisSourceTransform, DxorbisSourceTransformError, SkyLightLutSourceTransform,
    SkyLightLutSourceTransformError, is_legacy_dxorbis_source, is_legacy_sky_light_lut_source,
};

pub const DXORBIS_PATH: &str = "engineassets/dxorbis/";
pub const DXORBIS_SHADER_RECORD_HEADER_SIZE: usize = 8;
pub const DXORBIS_SHADER_PAYLOAD_TAG_OFFSET: usize = 36;
pub const DXORBIS_SHADER_PAYLOAD_TAG: &[u8; 4] = b"Shdr";
pub const HARDWARE_CURSOR_IMAGE_FILE_NAME: &str = "hw_cursor_image.bin";
pub const HARDWARE_CURSOR_WIDTH: u32 = 64;
pub const HARDWARE_CURSOR_HEIGHT: u32 = 64;
pub const HARDWARE_CURSOR_BYTES_PER_PIXEL: usize = 4;
pub const HARDWARE_CURSOR_IMAGE_SIZE: usize = HARDWARE_CURSOR_WIDTH as usize
    * HARDWARE_CURSOR_HEIGHT as usize
    * HARDWARE_CURSOR_BYTES_PER_PIXEL;
pub const PSSL_EXTENSION: &str = "pssl";

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PsslSourceInspectionError {
    #[error("read {path:?}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse PSSL source {path:?}: {source}")]
    Parse { path: PathBuf, source: ParseError },
}

/// Path-selected `engineassets/dxorbis` asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxorbisAsset<'a> {
    ShaderBinary(DxorbisShaderBinary<'a>),
    ShaderSource(DxorbisShaderSource<'a>),
    HardwareCursorImage(HardwareCursorImage<'a>),
}

impl<'a> DxorbisAsset<'a> {
    /// Parse a `dxorbis` payload using the asset path to select its family.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::UnknownDxorbisPath`] if `path` is outside
    /// `engineassets/dxorbis` or names neither the hardware cursor image nor a
    /// `.bin`/`.pssl` file. Otherwise the selected parser's error is
    /// propagated: [`ParseError::InvalidHardwareCursorSize`] for a cursor image
    /// that is not exactly the expected byte count, and
    /// [`ParseError::UnexpectedEof`], [`ParseError::InvalidMagic`],
    /// [`ParseError::UnsupportedVersion`],
    /// [`ParseError::UnsupportedDxorbisShaderStage`],
    /// [`ParseError::InvalidDxorbisShaderPayload`] or
    /// [`ParseError::TrailingDxorbisShaderBytes`] for a malformed shader.
    pub fn parse_path(path: impl AsRef<Path>, bytes: &'a [u8]) -> Result<Self, ParseError> {
        let path = normalize_source_path(path.as_ref().to_string_lossy());
        if !path.contains(DXORBIS_PATH) {
            return Err(ParseError::UnknownDxorbisPath);
        }

        let extension = Path::new(&path).extension();
        if path.ends_with(HARDWARE_CURSOR_IMAGE_FILE_NAME) {
            HardwareCursorImage::parse(bytes).map(Self::HardwareCursorImage)
        } else if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("bin")) {
            DxorbisShaderBinary::parse(bytes).map(Self::ShaderBinary)
        } else if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("pssl")) {
            DxorbisShaderSource::parse(bytes).map(Self::ShaderSource)
        } else {
            Err(ParseError::UnknownDxorbisPath)
        }
    }
}

/// Compiled `dxorbis` shader bytecode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DxorbisShaderBinary<'a> {
    bytes: &'a [u8],
    record_count: usize,
}

impl<'a> DxorbisShaderBinary<'a> {
    /// Parse a compiled `dxorbis` shader bytecode file.
    ///
    /// # Errors
    ///
    /// Returns an error when a record header, payload size, stage, or payload
    /// tag is invalid.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let mut records = DxorbisShaderRecords { bytes, position: 0 };
        let mut record_count = 0;
        for record in records.by_ref() {
            record?;
            record_count += 1;
        }
        if records.position != bytes.len() {
            return Err(ParseError::TrailingDxorbisShaderBytes {
                trailing: bytes.len() - records.position,
            });
        }
        Ok(Self {
            bytes,
            record_count,
        })
    }

    #[inline]
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub const fn len(self) -> usize {
        self.record_count
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.record_count == 0
    }

    #[inline]
    #[must_use]
    pub const fn records(self) -> DxorbisShaderRecords<'a> {
        DxorbisShaderRecords {
            bytes: self.bytes,
            position: 0,
        }
    }
}

/// One compiled `dxorbis` shader record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DxorbisShaderRecord<'a> {
    pub stage: DxorbisShaderStage,
    payload: &'a [u8],
}

impl<'a> DxorbisShaderRecord<'a> {
    #[inline]
    #[must_use]
    pub const fn payload(self) -> &'a [u8] {
        self.payload
    }
}

/// Shader stage stored in a `dxorbis` shader record header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum DxorbisShaderStage {
    Vertex = 0,
    Hull = 1,
    Domain = 2,
    Geometry = 3,
    Pixel = 4,
    Compute = 5,
}

impl DxorbisShaderStage {
    #[must_use]
    pub const fn from_native_value(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Vertex),
            1 => Some(Self::Hull),
            2 => Some(Self::Domain),
            3 => Some(Self::Geometry),
            4 => Some(Self::Pixel),
            5 => Some(Self::Compute),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> u32 {
        self as u32
    }
}

/// Borrowed iterator over compiled `dxorbis` shader records.
// Not `Copy`: a copyable iterator silently restarts when passed by value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DxorbisShaderRecords<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Iterator for DxorbisShaderRecords<'a> {
    type Item = Result<DxorbisShaderRecord<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position == self.bytes.len() {
            return None;
        }

        let start = self.position;
        let header_end = match start.checked_add(DXORBIS_SHADER_RECORD_HEADER_SIZE) {
            Some(end) if end <= self.bytes.len() => end,
            _ => {
                self.position = self.bytes.len();
                return Some(Err(ParseError::UnexpectedEof {
                    offset: start,
                    needed: DXORBIS_SHADER_RECORD_HEADER_SIZE,
                    actual: self.bytes.len().saturating_sub(start),
                }));
            }
        };

        let stage_value = read_u32_at(self.bytes, start).expect("record header checked");
        let Some(stage) = DxorbisShaderStage::from_native_value(stage_value) else {
            self.position = self.bytes.len();
            return Some(Err(ParseError::UnsupportedDxorbisShaderStage {
                stage: stage_value,
            }));
        };
        let payload_size =
            read_u32_at(self.bytes, start + 4).expect("record header checked") as usize;
        let payload_end = match header_end.checked_add(payload_size) {
            Some(end) if end <= self.bytes.len() => end,
            _ => {
                self.position = self.bytes.len();
                return Some(Err(ParseError::UnexpectedEof {
                    offset: header_end,
                    needed: payload_size,
                    actual: self.bytes.len().saturating_sub(header_end),
                }));
            }
        };
        if payload_size < DXORBIS_SHADER_PAYLOAD_TAG_OFFSET + DXORBIS_SHADER_PAYLOAD_TAG.len() {
            self.position = self.bytes.len();
            return Some(Err(ParseError::InvalidDxorbisShaderPayload {
                offset: header_end,
            }));
        }
        let tag_start = header_end + DXORBIS_SHADER_PAYLOAD_TAG_OFFSET;
        if self.bytes[tag_start..tag_start + DXORBIS_SHADER_PAYLOAD_TAG.len()]
            != DXORBIS_SHADER_PAYLOAD_TAG[..]
        {
            self.position = self.bytes.len();
            return Some(Err(ParseError::InvalidDxorbisShaderPayload {
                offset: header_end,
            }));
        }

        self.position = payload_end;
        Some(Ok(DxorbisShaderRecord {
            stage,
            payload: &self.bytes[header_end..payload_end],
        }))
    }
}

/// `dxorbis` PSSL shader source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DxorbisShaderSource<'a> {
    source: &'a str,
}

impl<'a> DxorbisShaderSource<'a> {
    /// Parse a PSSL shader source file.
    ///
    /// # Errors
    ///
    /// Returns an error when the source is not valid UTF-8.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let source = std::str::from_utf8(bytes).map_err(|source| ParseError::Utf8 { source })?;
        Ok(Self { source })
    }

    #[inline]
    #[must_use]
    pub const fn source(self) -> &'a str {
        self.source
    }

    #[inline]
    #[must_use]
    pub fn summary(self) -> PsslSourceSummary {
        PsslSourceSummary {
            lines: self.source.lines().count(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PsslSourceSummary {
    pub lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsslSourceFileSummary {
    pub source: String,
    pub summary: PsslSourceSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PsslSourceInspection {
    pub rows: Vec<PsslSourceFileSummary>,
    pub totals: PsslSourceTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct PsslSourceInspectionReport<'a> {
    inspection: &'a PsslSourceInspection,
    limit: usize,
}

impl fmt::Display for PsslSourceSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} lines", self.lines)
    }
}

/// Counts the lines in a `.pssl` shader source payload.
///
/// # Errors
///
/// Returns any error [`DxorbisShaderSource::parse`] returns — in practice
/// [`ParseError::UnexpectedEof`] when the payload ends before its declared
/// length.
pub fn summarize_pssl_source(bytes: &[u8]) -> Result<PsslSourceSummary, ParseError> {
    DxorbisShaderSource::parse(bytes).map(DxorbisShaderSource::summary)
}

/// Summarises one `.pssl` source, labelling the row with `path`.
///
/// `path` is only the display label; it is not read from disk.
///
/// # Errors
///
/// Returns any error [`summarize_pssl_source`] returns — in practice
/// [`ParseError::UnexpectedEof`] for a truncated payload.
pub fn inspect_pssl_source_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<PsslSourceFileSummary, ParseError> {
    Ok(PsslSourceFileSummary {
        source: path.as_ref().display().to_string(),
        summary: summarize_pssl_source(bytes)?,
    })
}

/// Reads a `.pssl` source from disk and summarises it.
///
/// # Errors
///
/// Returns [`PsslSourceInspectionError::Read`] if `path` cannot be read
/// (missing file, permissions), or [`PsslSourceInspectionError::Parse`]
/// wrapping the [`ParseError`] from a truncated payload. Both variants carry
/// the offending path.
pub fn inspect_pssl_source_path(
    path: impl AsRef<Path>,
) -> Result<PsslSourceFileSummary, PsslSourceInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| PsslSourceInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_pssl_source_file(path, &bytes).map_err(|source| PsslSourceInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Reads and summarises every `.pssl` source in `paths`, accumulating totals.
///
/// Stops at the first failing path; earlier rows are discarded with it.
///
/// # Errors
///
/// Returns any error [`inspect_pssl_source_path`] returns for the first path
/// that fails — [`PsslSourceInspectionError::Read`] for an unreadable file, or
/// [`PsslSourceInspectionError::Parse`] for a truncated payload.
pub fn inspect_pssl_source_files<I, P>(
    paths: I,
) -> Result<PsslSourceInspection, PsslSourceInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = PsslSourceInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_pssl_source_path(path)?);
    }
    Ok(inspection)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PsslSourceTotals {
    pub files: usize,
    pub lines: usize,
}

impl PsslSourceTotals {
    pub const fn add_summary(&mut self, summary: PsslSourceSummary) {
        self.files += 1;
        self.lines += summary.lines;
    }
}

impl PsslSourceInspection {
    pub fn add_file_summary(&mut self, row: PsslSourceFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> PsslSourceInspectionReport<'_> {
        PsslSourceInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for PsslSourceTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  lines: {}", self.lines)
    }
}

impl fmt::Display for PsslSourceInspectionReport<'_> {
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

#[must_use]
pub const fn is_pssl_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(PSSL_EXTENSION)
}

#[must_use]
pub fn is_pssl_name(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_pssl_extension)
}

#[must_use]
pub fn is_pssl_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_pssl_extension)
}

/// Raw hardware cursor image payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardwareCursorImage<'a> {
    pixels: &'a [u8],
}

impl<'a> HardwareCursorImage<'a> {
    /// Parse a hardware cursor image payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload is not the expected 64x64x4 byte size.
    pub const fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        if bytes.len() != HARDWARE_CURSOR_IMAGE_SIZE {
            return Err(ParseError::InvalidHardwareCursorSize {
                expected: HARDWARE_CURSOR_IMAGE_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self { pixels: bytes })
    }

    #[inline]
    #[must_use]
    pub const fn width(self) -> u32 {
        HARDWARE_CURSOR_WIDTH
    }

    #[inline]
    #[must_use]
    pub const fn height(self) -> u32 {
        HARDWARE_CURSOR_HEIGHT
    }

    #[inline]
    #[must_use]
    pub const fn pixels(self) -> &'a [u8] {
        self.pixels
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseError {
    #[error("unexpected end of file at {offset}: needed {needed} bytes, had {actual}")]
    UnexpectedEof {
        offset: usize,
        needed: usize,
        actual: usize,
    },
    #[error("unknown dxorbis asset path")]
    UnknownDxorbisPath,
    #[error("unsupported dxorbis shader stage {stage}")]
    UnsupportedDxorbisShaderStage { stage: u32 },
    #[error("invalid dxorbis shader payload at {offset}")]
    InvalidDxorbisShaderPayload { offset: usize },
    #[error("trailing bytes after dxorbis shader records: {trailing}")]
    TrailingDxorbisShaderBytes { trailing: usize },
    #[error("invalid hardware cursor image size: expected {expected}, got {actual}")]
    InvalidHardwareCursorSize { expected: usize, actual: usize },
    #[error("invalid magic for {asset}: expected {expected:?}, found {found:?}")]
    InvalidMagic {
        asset: &'static str,
        expected: &'static [u8],
        found: Vec<u8>,
    },
    #[error("unsupported {asset} version {found}, expected {expected}")]
    UnsupportedVersion {
        asset: &'static str,
        expected: u16,
        found: u16,
    },
    #[error("unsupported sky light LUT table set {found}, expected {expected}")]
    UnsupportedSkyLightLutTableSet { expected: u16, found: u16 },
    #[error("invalid sky light LUT size: expected {expected}, got {actual}")]
    InvalidSkyLightLutSize { expected: usize, actual: usize },
    #[error("invalid UTF-8 shader source: {source}")]
    Utf8 { source: std::str::Utf8Error },
}

fn read_u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let bytes = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes(bytes.try_into().expect("slice size")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_dxorbis_shader_records() {
        let mut bytes = Vec::new();
        push_shader_record(&mut bytes, DxorbisShaderStage::Vertex, 48);
        push_shader_record(&mut bytes, DxorbisShaderStage::Pixel, 40);

        let shader = DxorbisShaderBinary::parse(&bytes).unwrap();
        let stages = shader
            .records()
            .map(|record| record.unwrap().stage)
            .collect::<Vec<_>>();

        assert_eq!(shader.len(), 2);
        assert_eq!(
            stages,
            [DxorbisShaderStage::Vertex, DxorbisShaderStage::Pixel]
        );
    }

    #[test]
    fn parses_path_selected_cursor_image() {
        let bytes = [0; HARDWARE_CURSOR_IMAGE_SIZE];
        let asset =
            DxorbisAsset::parse_path("engineassets/dxorbis/hw_cursor_image.bin", &bytes).unwrap();

        assert!(matches!(asset, DxorbisAsset::HardwareCursorImage(_)));
    }

    #[test]
    fn summarizes_pssl_source_and_paths() {
        let summary = summarize_pssl_source(b"line one\nline two\n").unwrap();
        let mut totals = PsslSourceTotals::default();
        totals.add_summary(summary);
        totals.add_summary(PsslSourceSummary { lines: 1 });

        assert_eq!(summary, PsslSourceSummary { lines: 2 });
        assert_eq!(totals.files, 2);
        assert_eq!(totals.lines, 3);
        assert_eq!(summary.to_string(), "2 lines");
        assert_eq!(totals.to_string(), "  files: 2\n  lines: 3\n");

        let mut inspection = PsslSourceInspection::default();
        inspection.add_file_summary(
            inspect_pssl_source_file("engineassets/dxorbis/foo.pssl", b"line one\nline two\n")
                .expect("inspect pssl"),
        );
        assert_eq!(
            inspection.report(20).to_string(),
            "engineassets/dxorbis/foo.pssl: 2 lines\n  files: 1\n  lines: 2\n"
        );

        assert!(is_pssl_name("engineassets/dxorbis/foo.pssl"));
        assert!(is_pssl_name("FOO.PSSL"));
        assert!(!is_pssl_name("foo.bin"));
    }

    #[test]
    fn inspect_pssl_source_files_aggregates_file_results() {
        let path = std::env::temp_dir().join(format!(
            "az-rs-cry-renderer-{}-foo.pssl",
            std::process::id()
        ));
        std::fs::write(&path, b"line one\nline two\n").expect("write pssl");

        let inspection = inspect_pssl_source_files([&path]).expect("inspect pssl files");

        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.lines, 2);

        std::fs::remove_file(path).expect("remove pssl");
    }

    fn push_shader_record(bytes: &mut Vec<u8>, stage: DxorbisShaderStage, payload_size: usize) {
        bytes.extend(stage.native_value().to_le_bytes());
        let payload_size_le =
            u32::try_from(payload_size).expect("test fixture payload fits in u32");
        bytes.extend(payload_size_le.to_le_bytes());
        let start = bytes.len();
        bytes.resize(start + payload_size, 0);
        bytes[start + DXORBIS_SHADER_PAYLOAD_TAG_OFFSET
            ..start + DXORBIS_SHADER_PAYLOAD_TAG_OFFSET + DXORBIS_SHADER_PAYLOAD_TAG.len()]
            .copy_from_slice(DXORBIS_SHADER_PAYLOAD_TAG);
    }
}
