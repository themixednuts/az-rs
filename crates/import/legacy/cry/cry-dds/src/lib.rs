//! Parser for Cry/Lumberyard DDS texture assets.
//!
//! Cry texture sources may use the DDS header extension for base texture
//! headers and raw split-mip payload files for external mips.

pub mod builder;
pub mod source_transform;

pub use source_transform::{TextureSourceTransform, TextureSourceTransformError};

use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use thiserror::Error;

pub const DDS_HEADER_WITH_MAGIC_SIZE: usize = 128;
pub const DDS_HEADER_SIZE_VALUE: u32 = 124;
pub const DDS_PIXEL_FORMAT_SIZE: u32 = 32;
pub const DX10_HEADER_SIZE: usize = 20;

pub const FOURCC_DDS: [u8; 4] = *b"DDS ";
pub const FOURCC_DX10: [u8; 4] = *b"DX10";
pub const FOURCC_FYRC: [u8; 4] = *b"FYRC";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsAsset<'a> {
    kind: DdsAssetKind<'a>,
}

impl<'a> DdsAsset<'a> {
    /// Parses a DDS asset, using `path` to tell a split mip payload from a
    /// full file.
    ///
    /// A `.dds.N` mip payload is stored as raw bytes and is never rejected;
    /// only header parts and unsplit files go through [`DdsFile::parse`].
    ///
    /// # Errors
    ///
    /// Returns any error [`DdsFile::parse`] returns —
    /// [`ParseError::TooShort`] when the buffer is shorter than the magic plus
    /// header, [`ParseError::InvalidMagic`] when the first four bytes are not
    /// `DDS `, [`ParseError::InvalidHeaderSize`] or
    /// [`ParseError::InvalidPixelFormatSize`] when a declared struct size does
    /// not match the format, and [`ParseError::OffsetOverflow`] when a
    /// computed field offset does not fit in `usize`.
    pub fn parse(path: &str, bytes: &'a [u8]) -> Result<Self, ParseError> {
        match DdsSplitPart::from_path(path) {
            Some(DdsSplitPart::Header | DdsSplitPart::AlphaHeader) => {
                DdsFile::parse(bytes).map(|file| Self {
                    kind: DdsAssetKind::File(file),
                })
            }
            Some(part @ DdsSplitPart::Mip { .. }) => Ok(Self {
                kind: DdsAssetKind::SplitPayload(DdsSplitPayload { part, bytes }),
            }),
            None => DdsFile::parse(bytes).map(|file| Self {
                kind: DdsAssetKind::File(file),
            }),
        }
    }

    #[inline]
    #[must_use]
    pub const fn kind(&self) -> DdsAssetKind<'a> {
        self.kind
    }

    #[inline]
    #[must_use]
    pub const fn summary(&self) -> DdsAssetSummary {
        DdsAssetSummary::from_asset(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdsAssetKind<'a> {
    File(DdsFile<'a>),
    SplitPayload(DdsSplitPayload<'a>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdsAssetSummary {
    File(DdsFileSummary),
    SplitPayload(DdsSplitPayloadSummary),
}

impl DdsAssetSummary {
    #[inline]
    #[must_use]
    pub const fn from_asset(asset: &DdsAsset<'_>) -> Self {
        match asset.kind() {
            DdsAssetKind::File(file) => Self::File(DdsFileSummary::from_file(file)),
            DdsAssetKind::SplitPayload(payload) => {
                Self::SplitPayload(DdsSplitPayloadSummary::from_payload(payload))
            }
        }
    }
}

impl fmt::Display for DdsAssetSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(summary) => write!(f, "{summary}"),
            Self::SplitPayload(summary) => write!(f, "{summary}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsAssetInspectionReport<'a> {
    pub path: &'a str,
    pub summary: DdsAssetSummary,
}

impl fmt::Display for DdsAssetInspectionReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.path)?;
        write!(f, "{}", self.summary)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsAssetPathInspectionReport {
    pub path: String,
    pub summary: DdsAssetSummary,
}

impl fmt::Display for DdsAssetPathInspectionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.path)?;
        write!(f, "{}", self.summary)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsFileSummary {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mip_map_count: u32,
    pub persistent_mips: u8,
    pub pixel_four_cc: [u8; 4],
    pub cry_marker: [u8; 4],
    pub cry_flags: CryTextureFlags,
    pub split: bool,
    pub attached_alpha: bool,
    pub dx10_header: Option<Dx10Header>,
    pub payload_bytes: usize,
}

impl DdsFileSummary {
    #[inline]
    #[must_use]
    pub const fn from_file(file: DdsFile<'_>) -> Self {
        let header = file.header();
        Self {
            width: header.width,
            height: header.height,
            depth: header.depth,
            mip_map_count: header.mip_map_count,
            persistent_mips: header.persistent_mips,
            pixel_four_cc: header.pixel_format.four_cc,
            cry_marker: header.cry_marker,
            cry_flags: header.cry_flags,
            split: header.cry_flags.contains(CryTextureFlags::SPLIT),
            attached_alpha: header.cry_flags.contains(CryTextureFlags::ATTACHED_ALPHA),
            dx10_header: file.dx10_header(),
            payload_bytes: file.payload().len(),
        }
    }
}

impl fmt::Display for DdsFileSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  kind:             DDS header")?;
        writeln!(f, "  dimensions:       {}x{}", self.width, self.height)?;
        writeln!(f, "  depth:            {}", self.depth)?;
        writeln!(f, "  mip maps:         {}", self.mip_map_count)?;
        writeln!(f, "  persistent mips:  {}", self.persistent_mips)?;
        writeln!(f, "  pixel fourcc:     {}", fourcc_text(self.pixel_four_cc))?;
        writeln!(f, "  cry marker:       {}", fourcc_text(self.cry_marker))?;
        writeln!(f, "  cry flags:        {:#010x}", self.cry_flags.bits())?;
        writeln!(f, "  split:            {}", self.split)?;
        writeln!(f, "  attached alpha:   {}", self.attached_alpha)?;
        if let Some(dx10) = self.dx10_header {
            writeln!(f, "  dxgi format:      {}", dx10.dxgi_format)?;
            writeln!(f, "  resource dim:     {}", dx10.resource_dimension)?;
            writeln!(f, "  array size:       {}", dx10.array_size)?;
        }
        write!(f, "  payload bytes:    {}", self.payload_bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsSplitPayloadSummary {
    pub part: DdsSplitPart,
    pub bytes: usize,
}

impl DdsSplitPayloadSummary {
    #[inline]
    #[must_use]
    pub const fn from_payload(payload: DdsSplitPayload<'_>) -> Self {
        Self {
            part: payload.part,
            bytes: payload.bytes.len(),
        }
    }
}

impl fmt::Display for DdsSplitPayloadSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  kind:  {}", self.part)?;
        if let Some(index) = self.part.mip_index() {
            writeln!(f, "  mip:   {index}")?;
            writeln!(f, "  alpha: {}", self.part.is_alpha())?;
        }
        write!(f, "  bytes: {}", self.bytes)
    }
}

/// Summarises one DDS asset's dimensions, format and payload size.
///
/// # Errors
///
/// Returns any error [`DdsAsset::parse`] returns — [`ParseError::TooShort`],
/// [`ParseError::InvalidMagic`], [`ParseError::InvalidHeaderSize`],
/// [`ParseError::InvalidPixelFormatSize`] or [`ParseError::OffsetOverflow`]
/// for a malformed header. Split mip payloads never fail.
pub fn summarize_dds_asset(path: &str, bytes: &[u8]) -> Result<DdsAssetSummary, ParseError> {
    DdsAsset::parse(path, bytes).map(|asset| asset.summary())
}

/// Summarises one DDS asset and pairs the summary with its path for display.
///
/// # Errors
///
/// Returns any error [`DdsAsset::parse`] returns — [`ParseError::TooShort`],
/// [`ParseError::InvalidMagic`], [`ParseError::InvalidHeaderSize`],
/// [`ParseError::InvalidPixelFormatSize`] or [`ParseError::OffsetOverflow`]
/// for a malformed header.
pub fn inspect_dds_asset<'a>(
    path: &'a str,
    bytes: &[u8],
) -> Result<DdsAssetInspectionReport<'a>, ParseError> {
    DdsAsset::parse(path, bytes).map(|asset| DdsAssetInspectionReport {
        path,
        summary: asset.summary(),
    })
}

#[derive(Debug, Error)]
pub enum DdsAssetInspectionError {
    #[error("read DDS asset {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse DDS asset {path:?}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
}

/// Reads a DDS asset from disk and summarises it.
///
/// # Errors
///
/// Returns [`DdsAssetInspectionError::Read`] if `path` cannot be read (missing
/// file, permissions), or [`DdsAssetInspectionError::Parse`] wrapping the
/// [`ParseError`] from a malformed header. Both variants carry the offending
/// path.
pub fn inspect_dds_asset_path(
    path: impl AsRef<Path>,
) -> Result<DdsAssetPathInspectionReport, DdsAssetInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| DdsAssetInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let display_path = path.to_string_lossy().into_owned();
    summarize_dds_asset(&display_path, &bytes)
        .map(|summary| DdsAssetPathInspectionReport {
            path: display_path,
            summary,
        })
        .map_err(|source| DdsAssetInspectionError::Parse {
            path: path.to_path_buf(),
            source,
        })
}

#[must_use]
pub fn fourcc_text(value: [u8; 4]) -> String {
    if value.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        String::from_utf8_lossy(&value).into_owned()
    } else {
        format!("{value:02x?}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsSplitPayload<'a> {
    pub part: DdsSplitPart,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DdsSplitPart {
    Header,
    /// Attached-alpha DDS header (`.dds.a`), paired with the same-stem color
    /// header during legacy import. It is not an independent authoring texture.
    AlphaHeader,
    /// Split mip payload. `alpha: true` identifies `.dds.<mip>a` payloads
    /// that belong to the attached-alpha image.
    Mip {
        index: u32,
        alpha: bool,
    },
}

impl DdsSplitPart {
    #[must_use]
    pub fn from_path(path: &str) -> Option<Self> {
        let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let (stem, suffix) = file_name.rsplit_once('.')?;
        if suffix.eq_ignore_ascii_case("dds") {
            return Some(Self::Header);
        }
        let (_, dds_ext) = stem.rsplit_once('.')?;
        if !dds_ext.eq_ignore_ascii_case("dds") {
            return None;
        }
        if suffix.eq_ignore_ascii_case("a") {
            return Some(Self::AlphaHeader);
        }

        let (digits, alpha) = suffix
            .strip_suffix('a')
            .or_else(|| suffix.strip_suffix('A'))
            .map_or((suffix, false), |digits| (digits, true));
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let index = digits.parse().ok()?;
        Some(Self::Mip { index, alpha })
    }

    #[inline]
    #[must_use]
    pub const fn is_alpha(self) -> bool {
        matches!(self, Self::AlphaHeader | Self::Mip { alpha: true, .. })
    }

    #[inline]
    #[must_use]
    pub const fn mip_index(self) -> Option<u32> {
        match self {
            Self::Mip { index, .. } => Some(index),
            Self::Header | Self::AlphaHeader => None,
        }
    }
}

impl fmt::Display for DdsSplitPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Header => "DDS header",
            Self::AlphaHeader => "DDS alpha header",
            Self::Mip { .. } => "DDS split mip",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsFile<'a> {
    bytes: &'a [u8],
    header: DdsHeader,
    dx10_header: Option<Dx10Header>,
    payload: &'a [u8],
}

impl<'a> DdsFile<'a> {
    /// Parses a complete DDS file: magic, header, optional DX10 header and
    /// payload.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::TooShort`] if `bytes` is shorter than the magic
    /// plus the fixed header (or than the DX10 header when the pixel format
    /// declares one), [`ParseError::InvalidMagic`] if the first four bytes are
    /// not `DDS `, [`ParseError::InvalidHeaderSize`] if the header's own size
    /// field is not 124, [`ParseError::InvalidPixelFormatSize`] if the
    /// pixel-format size field is not 32, and [`ParseError::OffsetOverflow`]
    /// if a computed field offset does not fit in `usize`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        if bytes.len() < DDS_HEADER_WITH_MAGIC_SIZE {
            return Err(ParseError::TooShort {
                needed: DDS_HEADER_WITH_MAGIC_SIZE,
                actual: bytes.len(),
            });
        }

        let magic = read_array::<4>(bytes, 0)?;
        if magic != FOURCC_DDS {
            return Err(ParseError::InvalidMagic {
                expected: FOURCC_DDS,
                actual: magic,
            });
        }

        let header = DdsHeader {
            size: read_u32(bytes, 4)?,
            flags: read_u32(bytes, 8)?,
            height: read_u32(bytes, 12)?,
            width: read_u32(bytes, 16)?,
            pitch_or_linear_size: read_u32(bytes, 20)?,
            depth: read_u32(bytes, 24)?,
            mip_map_count: read_u32(bytes, 28)?,
            alpha_bit_depth: read_u32(bytes, 32)?,
            cry_flags: CryTextureFlags(read_u32(bytes, 36)?),
            average_brightness: read_f32(bytes, 40)?,
            min_color: read_f32_array::<4>(bytes, 44)?,
            max_color: read_f32_array::<4>(bytes, 60)?,
            pixel_format: DdsPixelFormat {
                size: read_u32(bytes, 76)?,
                flags: read_u32(bytes, 80)?,
                four_cc: read_array::<4>(bytes, 84)?,
                rgb_bit_count: read_u32(bytes, 88)?,
                r_bit_mask: read_u32(bytes, 92)?,
                g_bit_mask: read_u32(bytes, 96)?,
                b_bit_mask: read_u32(bytes, 100)?,
                a_bit_mask: read_u32(bytes, 104)?,
            },
            caps: read_u32(bytes, 108)?,
            caps2: read_u32(bytes, 112)?,
            persistent_mips: bytes[116],
            tile_mode: bytes[117],
            reserved2: read_array::<6>(bytes, 118)?,
            cry_marker: read_array::<4>(bytes, 124)?,
        };

        if header.size != DDS_HEADER_SIZE_VALUE {
            return Err(ParseError::InvalidHeaderSize {
                expected: DDS_HEADER_SIZE_VALUE,
                actual: header.size,
            });
        }
        if header.pixel_format.size != DDS_PIXEL_FORMAT_SIZE {
            return Err(ParseError::InvalidPixelFormatSize {
                expected: DDS_PIXEL_FORMAT_SIZE,
                actual: header.pixel_format.size,
            });
        }

        let mut payload_offset = DDS_HEADER_WITH_MAGIC_SIZE;
        let dx10_header = if header.pixel_format.four_cc == FOURCC_DX10 {
            let end = payload_offset
                .checked_add(DX10_HEADER_SIZE)
                .ok_or(ParseError::OffsetOverflow)?;
            if end > bytes.len() {
                return Err(ParseError::TooShort {
                    needed: end,
                    actual: bytes.len(),
                });
            }
            let header = Dx10Header {
                dxgi_format: read_u32(bytes, payload_offset)?,
                resource_dimension: read_u32(bytes, payload_offset + 4)?,
                misc_flag: read_u32(bytes, payload_offset + 8)?,
                array_size: read_u32(bytes, payload_offset + 12)?,
                misc_flags2: read_u32(bytes, payload_offset + 16)?,
            };
            payload_offset = end;
            Some(header)
        } else {
            None
        };

        Ok(Self {
            bytes,
            header,
            dx10_header,
            payload: &bytes[payload_offset..],
        })
    }

    #[inline]
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub const fn header(&self) -> DdsHeader {
        self.header
    }

    #[inline]
    #[must_use]
    pub const fn dx10_header(&self) -> Option<Dx10Header> {
        self.dx10_header
    }

    #[inline]
    #[must_use]
    pub const fn payload(&self) -> &'a [u8] {
        self.payload
    }

    #[inline]
    #[must_use]
    pub const fn is_dx10(&self) -> bool {
        self.dx10_header.is_some()
    }

    #[inline]
    #[must_use]
    pub const fn is_cry_extended(&self) -> bool {
        let marker = self.header.cry_marker;
        marker[0] == FOURCC_FYRC[0]
            && marker[1] == FOURCC_FYRC[1]
            && marker[2] == FOURCC_FYRC[2]
            && marker[3] == FOURCC_FYRC[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsHeader {
    pub size: u32,
    pub flags: u32,
    pub height: u32,
    pub width: u32,
    pub pitch_or_linear_size: u32,
    pub depth: u32,
    pub mip_map_count: u32,
    pub alpha_bit_depth: u32,
    pub cry_flags: CryTextureFlags,
    pub average_brightness: u32,
    pub min_color: [u32; 4],
    pub max_color: [u32; 4],
    pub pixel_format: DdsPixelFormat,
    pub caps: u32,
    pub caps2: u32,
    pub persistent_mips: u8,
    pub tile_mode: u8,
    pub reserved2: [u8; 6],
    pub cry_marker: [u8; 4],
}

impl DdsHeader {
    #[inline]
    #[must_use]
    pub const fn average_brightness_f32(self) -> f32 {
        f32::from_bits(self.average_brightness)
    }

    #[inline]
    #[must_use]
    pub fn min_color_f32(self) -> [f32; 4] {
        self.min_color.map(f32::from_bits)
    }

    #[inline]
    #[must_use]
    pub fn max_color_f32(self) -> [f32; 4] {
        self.max_color.map(f32::from_bits)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsPixelFormat {
    pub size: u32,
    pub flags: u32,
    pub four_cc: [u8; 4],
    pub rgb_bit_count: u32,
    pub r_bit_mask: u32,
    pub g_bit_mask: u32,
    pub b_bit_mask: u32,
    pub a_bit_mask: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dx10Header {
    pub dxgi_format: u32,
    pub resource_dimension: u32,
    pub misc_flag: u32,
    pub array_size: u32,
    pub misc_flags2: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CryTextureFlags(pub u32);

impl CryTextureFlags {
    pub const CUBEMAP: Self = Self(0x1);
    pub const VOLUME_TEXTURE: Self = Self(0x2);
    pub const DECAL: Self = Self(0x4);
    pub const GREYSCALE: Self = Self(0x8);
    pub const SUPPRESS_ENGINE_REDUCE: Self = Self(0x10);
    pub const ATTACHED_ALPHA: Self = Self(0x400);
    pub const SRGB_READ: Self = Self(0x800);
    pub const DONT_RESIZE: Self = Self(0x8000);
    pub const RENORMALIZED_TEXTURE: Self = Self(0x10000);
    pub const TILED: Self = Self(0x80000);
    pub const SPLIT: Self = Self(0x20_0000);
    pub const COLOR_MODEL_MASK: Self = Self(0x700_0000);

    #[inline]
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("asset is too short: need at least {needed} bytes, got {actual}")]
    TooShort { needed: usize, actual: usize },

    #[error("invalid magic: expected {expected:?}, got {actual:?}")]
    InvalidMagic { expected: [u8; 4], actual: [u8; 4] },

    #[error("invalid DDS header size: expected {expected}, got {actual}")]
    InvalidHeaderSize { expected: u32, actual: u32 },

    #[error("invalid DDS pixel-format size: expected {expected}, got {actual}")]
    InvalidPixelFormatSize { expected: u32, actual: u32 },

    #[error("offset overflow")]
    OffsetOverflow,
}

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], ParseError> {
    let end = offset.checked_add(N).ok_or(ParseError::OffsetOverflow)?;
    let slice = bytes.get(offset..end).ok_or(ParseError::TooShort {
        needed: end,
        actual: bytes.len(),
    })?;
    Ok(slice.try_into().expect("slice width matches array width"))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ParseError> {
    read_array::<4>(bytes, offset).map(u32::from_le_bytes)
}

fn read_f32(bytes: &[u8], offset: usize) -> Result<u32, ParseError> {
    read_u32(bytes, offset)
}

fn read_f32_array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u32; N], ParseError> {
    let mut result = [0; N];
    for (index, slot) in result.iter_mut().enumerate() {
        *slot = read_f32(bytes, offset + index * 4)?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset::EngineTextureFormat;
    use az_asset_builder::{
        LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceFormat,
        source_schema_type,
    };

    const TEST_TEXTURE_SCHEMA: az_asset_builder::SourceSchemaType =
        source_schema_type::<TestTextureSource>();

    #[derive(SourceFormat)]
    #[source(schema = "az.test.TextureSource", ext = "dds")]
    struct TestTextureSource;

    #[test]
    fn classifies_dds_split_paths() {
        assert_eq!(
            DdsSplitPart::from_path("textures/foo.dds"),
            Some(DdsSplitPart::Header)
        );
        assert_eq!(
            DdsSplitPart::from_path("textures/foo.dds.a"),
            Some(DdsSplitPart::AlphaHeader)
        );
        assert_eq!(
            DdsSplitPart::from_path("textures/foo.dds.12"),
            Some(DdsSplitPart::Mip {
                index: 12,
                alpha: false,
            })
        );
        assert_eq!(
            DdsSplitPart::from_path("textures/foo.dds.12a"),
            Some(DdsSplitPart::Mip {
                index: 12,
                alpha: true,
            })
        );
        assert_eq!(DdsSplitPart::from_path("textures/foo.png"), None);
    }

    #[test]
    fn formats_split_part_labels() {
        assert_eq!(DdsSplitPart::Header.to_string(), "DDS header");
        assert_eq!(DdsSplitPart::AlphaHeader.to_string(), "DDS alpha header");
        let mip = DdsSplitPart::Mip {
            index: 12,
            alpha: true,
        };
        assert_eq!(mip.to_string(), "DDS split mip");
        assert_eq!(mip.mip_index(), Some(12));
        assert!(mip.is_alpha());
    }

    #[test]
    fn parses_cry_dx10_header_without_copying_payload() {
        let mut bytes = minimal_dx10_dds();
        bytes.extend_from_slice(&[1, 2, 3, 4]);

        let file = DdsFile::parse(&bytes).unwrap();

        assert_eq!(file.header().width, 2048);
        assert_eq!(file.header().height, 1024);
        assert_eq!(file.header().mip_map_count, 9);
        assert_eq!(file.header().persistent_mips, 3);
        assert!(file.is_dx10());
        assert!(file.is_cry_extended());
        assert!(
            file.header()
                .cry_flags
                .contains(CryTextureFlags::ATTACHED_ALPHA)
        );
        assert!(file.header().cry_flags.contains(CryTextureFlags::SPLIT));
        assert_eq!(file.dx10_header().unwrap().dxgi_format, 77);
        assert_eq!(file.payload(), &[1, 2, 3, 4]);
        assert_eq!(
            DdsFileSummary::from_file(file),
            DdsFileSummary {
                width: 2048,
                height: 1024,
                depth: 0,
                mip_map_count: 9,
                persistent_mips: 3,
                pixel_four_cc: FOURCC_DX10,
                cry_marker: FOURCC_FYRC,
                cry_flags: CryTextureFlags(
                    CryTextureFlags::ATTACHED_ALPHA.bits() | CryTextureFlags::SPLIT.bits()
                ),
                split: true,
                attached_alpha: true,
                dx10_header: Some(Dx10Header {
                    dxgi_format: 77,
                    resource_dimension: 3,
                    misc_flag: 0,
                    array_size: 1,
                    misc_flags2: 0,
                }),
                payload_bytes: 4,
            }
        );
        assert_eq!(
            DdsFileSummary::from_file(file).to_string(),
            "  kind:             DDS header\n  dimensions:       2048x1024\n  depth:            0\n  mip maps:         9\n  persistent mips:  3\n  pixel fourcc:     DX10\n  cry marker:       FYRC\n  cry flags:        0x00200400\n  split:            true\n  attached alpha:   true\n  dxgi format:      77\n  resource dim:     3\n  array size:       1\n  payload bytes:    4"
        );
    }

    #[test]
    fn parses_split_payload_by_path() {
        let bytes = [0xde, 0xad, 0xbe, 0xef];
        let asset = DdsAsset::parse("textures/foo.dds.1a", &bytes).unwrap();

        assert_eq!(
            asset.kind(),
            DdsAssetKind::SplitPayload(DdsSplitPayload {
                part: DdsSplitPart::Mip {
                    index: 1,
                    alpha: true,
                },
                bytes: &bytes,
            })
        );
        assert_eq!(
            asset.summary(),
            DdsAssetSummary::SplitPayload(DdsSplitPayloadSummary {
                part: DdsSplitPart::Mip {
                    index: 1,
                    alpha: true,
                },
                bytes: 4,
            })
        );
        assert_eq!(
            asset.summary().to_string(),
            "  kind:  DDS split mip\n  mip:   1\n  alpha: true\n  bytes: 4"
        );
    }

    #[test]
    fn formats_asset_inspection_report() {
        let bytes = [0xde, 0xad, 0xbe, 0xef];
        let report = inspect_dds_asset("textures/foo.dds.1a", &bytes).unwrap();

        assert_eq!(
            report.to_string(),
            "textures/foo.dds.1a\n  kind:  DDS split mip\n  mip:   1\n  alpha: true\n  bytes: 4"
        );
    }

    #[test]
    fn source_transform_keeps_dds_profile_as_compatibility_evidence() {
        let bytes = minimal_dx10_dds();

        let output = TextureSourceTransform::new(EngineTextureFormat::Dds, TEST_TEXTURE_SCHEMA)
            .transform(LegacySourceInput::new("Textures/Foo/Diff.dds", &bytes))
            .unwrap();

        assert_eq!(
            output,
            LegacySourceOutput::compatibility_evidence(
                "textures/foo/diff.dds",
                TEST_TEXTURE_SCHEMA,
                bytes
            )
        );
    }

    #[test]
    fn source_transform_refuses_fake_ktx2_without_grouped_rewrap() {
        let bytes = minimal_dx10_dds();

        let output = TextureSourceTransform::new(EngineTextureFormat::Ktx2, TEST_TEXTURE_SCHEMA)
            .transform(LegacySourceInput::new("Textures/Foo/Diff.dds", &bytes))
            .unwrap();

        let LegacySourceOutput::Unclassified { reason } = output else {
            panic!("KTX2 profile must not emit fake authoring source");
        };
        assert!(reason.contains("KTX2"));
        assert!(reason.contains("grouped DDS"));
    }

    #[test]
    fn split_mip_payloads_are_not_standalone_authoring_sources() {
        let bytes = [0xde, 0xad, 0xbe, 0xef];

        let output = TextureSourceTransform::new(EngineTextureFormat::Dds, TEST_TEXTURE_SCHEMA)
            .transform(LegacySourceInput::new("Textures/Foo/Diff.dds.1a", &bytes))
            .unwrap();

        assert_eq!(
            output,
            LegacySourceOutput::compatibility_evidence(
                "textures/foo/diff.dds.1a",
                TEST_TEXTURE_SCHEMA,
                bytes
            )
        );
    }

    fn minimal_dx10_dds() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FOURCC_DDS);
        push_u32(&mut bytes, DDS_HEADER_SIZE_VALUE);
        push_u32(&mut bytes, 0x21007);
        push_u32(&mut bytes, 1024);
        push_u32(&mut bytes, 2048);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 9);
        push_u32(&mut bytes, 0);
        push_u32(
            &mut bytes,
            CryTextureFlags::ATTACHED_ALPHA.bits() | CryTextureFlags::SPLIT.bits(),
        );
        push_u32(&mut bytes, 0);
        for _ in 0..4 {
            push_u32(&mut bytes, 0);
        }
        for _ in 0..4 {
            push_u32(&mut bytes, 1.0_f32.to_bits());
        }
        push_u32(&mut bytes, DDS_PIXEL_FORMAT_SIZE);
        push_u32(&mut bytes, 0x4);
        bytes.extend_from_slice(&FOURCC_DX10);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0x40_1008);
        push_u32(&mut bytes, 0);
        bytes.push(3);
        bytes.push(0);
        bytes.extend_from_slice(&[0; 6]);
        bytes.extend_from_slice(&FOURCC_FYRC);
        push_u32(&mut bytes, 77);
        push_u32(&mut bytes, 3);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 1);
        push_u32(&mut bytes, 0);
        assert_eq!(bytes.len(), DDS_HEADER_WITH_MAGIC_SIZE + DX10_HEADER_SIZE);
        bytes
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}
