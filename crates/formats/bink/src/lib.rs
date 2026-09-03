//! Parser for Bink video asset headers.

pub mod builder;
pub mod runtime;
pub mod source_transform;

pub use source_transform::{
    BINK_MEDIA_SOURCE_SCHEMA, BinkMediaSourceFormat, BinkMediaSourceTransform,
    BinkMediaSourceTransformError,
};

use az_asset_builder::{
    BuildRuleRegistration, ProductFormat, ProductFormatId, ProductFormatRegistration,
    product_format_id,
};
use thiserror::Error;

#[derive(ProductFormat)]
#[product_format(id = "azoth.media.bink.video", version = 1, asset = az_core::ConfigAsset)]
pub struct BinkVideoProductFormat;

pub const BINK_VIDEO_PRODUCT_FORMAT_ID: ProductFormatId =
    product_format_id::<BinkVideoProductFormat>();

/// The product formats this crate owns, for a host contribution to register.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 1] {
    [ProductFormatRegistration::for_format::<
        BinkVideoProductFormat,
    >()]
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
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

pub const HEADER_SIZE: usize = 28;
const DECLARED_SIZE_OFFSET: usize = 4;
const FILE_SIZE_BIAS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinkFile<'a> {
    bytes: &'a [u8],
    header: BinkHeader,
}

impl<'a> BinkFile<'a> {
    /// Parses a Bink container header and validates its declared size against
    /// the buffer.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::TooShort`] if `bytes` is shorter than the fixed
    /// header, [`ParseError::InvalidMagic`] if the first four bytes are not a
    /// recognised Bink signature, [`ParseError::DeclaredSizeOverflow`] if the
    /// declared size plus the header bias does not fit in `usize`,
    /// [`ParseError::DeclaredSizeTooSmall`] if the declared total is smaller
    /// than the header itself, and [`ParseError::TruncatedPayload`] if the
    /// buffer is shorter than the declared total.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        if bytes.len() < HEADER_SIZE {
            return Err(ParseError::TooShort {
                needed: HEADER_SIZE,
                actual: bytes.len(),
            });
        }

        let magic = BinkMagic::parse(&bytes[..4])?;
        let declared_size = read_u32(bytes, DECLARED_SIZE_OFFSET)?;
        let declared_total_size = usize::try_from(declared_size)
            .ok()
            .and_then(|size| size.checked_add(FILE_SIZE_BIAS))
            .ok_or(ParseError::DeclaredSizeOverflow { declared_size })?;
        if declared_total_size < HEADER_SIZE {
            return Err(ParseError::DeclaredSizeTooSmall {
                declared: declared_total_size,
                header: HEADER_SIZE,
            });
        }
        if declared_total_size > bytes.len() {
            return Err(ParseError::TruncatedPayload {
                declared: declared_total_size,
                actual: bytes.len(),
            });
        }

        let header = BinkHeader {
            magic,
            declared_size,
            frame_count: read_u32(bytes, 8)?,
            largest_frame_size: read_u32(bytes, 12)?,
            frame_count_again: read_u32(bytes, 16)?,
            width: read_u32(bytes, 20)?,
            height: read_u32(bytes, 24)?,
        };

        Ok(Self { bytes, header })
    }

    #[inline]
    #[must_use]
    pub const fn header(&self) -> BinkHeader {
        self.header
    }

    #[inline]
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub fn payload(&self) -> &'a [u8] {
        let declared_total = self.header.declared_total_size();
        &self.bytes[HEADER_SIZE..declared_total]
    }

    #[inline]
    #[must_use]
    pub fn trailing_bytes(&self) -> &'a [u8] {
        let declared_total = self.header.declared_total_size();
        &self.bytes[declared_total..]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinkHeader {
    pub magic: BinkMagic,
    pub declared_size: u32,
    pub frame_count: u32,
    pub largest_frame_size: u32,
    pub frame_count_again: u32,
    pub width: u32,
    pub height: u32,
}

impl BinkHeader {
    #[must_use]
    pub const fn declared_total_size(self) -> usize {
        self.declared_size as usize + FILE_SIZE_BIAS
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinkMagic {
    Bink1 { version: u8 },
    Bink2 { version: u8 },
}

impl BinkMagic {
    fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        match bytes {
            [b'B', b'I', b'K', version] => Ok(Self::Bink1 { version: *version }),
            [b'K', b'B', b'2', version] => Ok(Self::Bink2 { version: *version }),
            [a, b, c, d] => Err(ParseError::InvalidMagic {
                actual: [*a, *b, *c, *d],
            }),
            _ => Err(ParseError::TooShort {
                needed: 4,
                actual: bytes.len(),
            }),
        }
    }
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Bink file is too short: need at least {needed} bytes, got {actual}")]
    TooShort { needed: usize, actual: usize },

    #[error("invalid Bink magic {actual:?}")]
    InvalidMagic { actual: [u8; 4] },

    #[error("Bink declared size overflows usize: {declared_size}")]
    DeclaredSizeOverflow { declared_size: u32 },

    #[error("Bink declared size is smaller than the header: declared {declared}, header {header}")]
    DeclaredSizeTooSmall { declared: usize, header: usize },

    #[error("Bink payload is truncated: declared {declared} bytes, got {actual}")]
    TruncatedPayload { declared: usize, actual: usize },
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ParseError> {
    let end = offset + 4;
    if bytes.len() < end {
        return Err(ParseError::TooShort {
            needed: end,
            actual: bytes.len(),
        });
    }
    Ok(u32::from_le_bytes(
        bytes[offset..end].try_into().expect("slice size"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::{LegacySourceInput, LegacySourceOutput, LegacySourceTransform};

    #[test]
    fn parses_bink2_header() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"KB2j");
        bytes.extend_from_slice(&32u32.to_le_bytes());
        bytes.extend_from_slice(&294u32.to_le_bytes());
        bytes.extend_from_slice(&157_140u32.to_le_bytes());
        bytes.extend_from_slice(&294u32.to_le_bytes());
        bytes.extend_from_slice(&3840u32.to_le_bytes());
        bytes.extend_from_slice(&2160u32.to_le_bytes());
        bytes.resize(40, 0xaa);

        let file = BinkFile::parse(&bytes).unwrap();
        assert_eq!(file.header().magic, BinkMagic::Bink2 { version: b'j' });
        assert_eq!(file.header().declared_total_size(), 40);
        assert_eq!(file.header().frame_count, 294);
        assert_eq!(file.header().largest_frame_size, 157_140);
        assert_eq!(file.header().width, 3840);
        assert_eq!(file.header().height, 2160);
        assert_eq!(file.payload().len(), 12);
        assert!(file.trailing_bytes().is_empty());
    }

    #[test]
    fn rejects_truncated_payload() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"BIKi");
        bytes.extend_from_slice(&128u32.to_le_bytes());
        bytes.resize(HEADER_SIZE, 0);

        let err = BinkFile::parse(&bytes).unwrap_err();
        assert!(matches!(err, ParseError::TruncatedPayload { .. }));
    }

    #[test]
    fn source_transform_marks_bink_as_external_product_payload() {
        let bytes = bink2_bytes();

        let output = BinkMediaSourceTransform
            .transform(LegacySourceInput::new("LyShineUI/Videos/Intro.bk2", &bytes))
            .unwrap();

        assert_eq!(
            output,
            LegacySourceOutput::external_product_payload(
                "lyshineui/videos/intro.bk2",
                BINK_MEDIA_SOURCE_SCHEMA,
                bytes
            )
        );
    }

    fn bink2_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"KB2j");
        bytes.extend_from_slice(&32u32.to_le_bytes());
        bytes.extend_from_slice(&294u32.to_le_bytes());
        bytes.extend_from_slice(&157_140u32.to_le_bytes());
        bytes.extend_from_slice(&294u32.to_le_bytes());
        bytes.extend_from_slice(&3840u32.to_le_bytes());
        bytes.extend_from_slice(&2160u32.to_le_bytes());
        bytes.resize(40, 0xaa);
        bytes
    }
}
