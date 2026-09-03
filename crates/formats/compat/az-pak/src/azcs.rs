//! AZCS — Lumberyard's `AzCore::Compression` inner-layer wrapper.
//!
//! After a zip entry has been decompressed, the resulting bytes may
//! still be wrapped in an AZCS frame:
//!
//! ```text
//! +0x00  signature       [u8; 4] = b"AZCS"
//! +0x04  compressor_id   u32 (big-endian)  -- ZLIB | ZSTD
//! +0x08  uncompressed_sz u64 (big-endian)
//! +0x10  payload         (compressor-specific)
//! ```
//!
//! The format is defined by O3DE's
//! `Code/Framework/AzCore/AzCore/IO/Compressor.h` and
//! `Code/Framework/AzCore/AzCore/IO/CompressorZLib.h`.

use std::fmt;
use std::io::{self, Read};

use flate2::read::ZlibDecoder;
use thiserror::Error;

/// `b"AZCS"` — the inner-format signature.
pub const AZCS_SIGNATURE: &[u8; 4] = b"AZCS";

/// Header size in bytes (signature + id + `uncompressed_size`).
pub const HEADER_LEN: usize = 16;

#[derive(Debug, Error)]
pub enum AzcsError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("invalid AZCS signature {actual:02x?} (expected {AZCS_SIGNATURE:02x?})")]
    InvalidSignature { actual: [u8; 4] },

    #[error("unsupported AZCS compressor id: {0:#010x}")]
    UnsupportedCompressor(u32),

    #[error("AZCS ZSTD compressor not yet implemented")]
    Zstd,
}

/// Inner-AZCS compressor identifier.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum AzcsId {
    Zlib = 0x7388_7D3A,
    Zstd = 0x72FD_505E,
}

impl AzcsId {
    /// `const`-friendly conversion from a raw u32.
    #[inline]
    #[must_use]
    pub const fn from_u32(value: u32) -> Option<Self> {
        match value {
            0x7388_7D3A => Some(Self::Zlib),
            0x72FD_505E => Some(Self::Zstd),
            _ => None,
        }
    }

    /// The raw u32 value as it appears on disk.
    #[inline]
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }
}

impl TryFrom<u32> for AzcsId {
    type Error = AzcsError;

    #[inline]
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::from_u32(value).ok_or(AzcsError::UnsupportedCompressor(value))
    }
}

impl fmt::Display for AzcsId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Zlib => "ZLIB",
            Self::Zstd => "ZSTD",
        })
    }
}

/// Parsed AZCS header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AzcsHeader {
    pub compressor_id: AzcsId,
    pub uncompressed_size: u64,
}

impl AzcsHeader {
    /// Read + validate the AZCS header from `reader`.
    ///
    /// # Errors
    ///
    /// Returns [`AzcsError::Io`] when the 16-byte header cannot be read in
    /// full, [`AzcsError::InvalidSignature`] when the leading four bytes are
    /// not `AZCS`, and [`AzcsError::UnsupportedCompressor`] when the
    /// compressor id is neither [`AzcsId::Zlib`] nor [`AzcsId::Zstd`].
    pub fn from_reader<R: Read>(reader: &mut R) -> Result<Self, AzcsError> {
        let mut sig = [0u8; 4];
        reader.read_exact(&mut sig)?;
        if &sig != AZCS_SIGNATURE {
            return Err(AzcsError::InvalidSignature { actual: sig });
        }

        let mut buf4 = [0u8; 4];
        reader.read_exact(&mut buf4)?;
        let compressor_id = AzcsId::try_from(u32::from_be_bytes(buf4))?;

        let mut buf8 = [0u8; 8];
        reader.read_exact(&mut buf8)?;
        let uncompressed_size = u64::from_be_bytes(buf8);

        Ok(Self {
            compressor_id,
            uncompressed_size,
        })
    }

    /// Try to decode the header from a byte slice without consuming
    /// the buffer. Returns `None` if the slice is too short or the
    /// signature doesn't match.
    #[must_use]
    pub fn peek(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER_LEN || &bytes[0..4] != AZCS_SIGNATURE {
            return None;
        }
        let compressor_id = AzcsId::from_u32(u32::from_be_bytes(bytes[4..8].try_into().ok()?))?;
        let uncompressed_size = u64::from_be_bytes(bytes[8..16].try_into().ok()?);
        Some(Self {
            compressor_id,
            uncompressed_size,
        })
    }
}

/// Cheap signature-only check: does `bytes` start with `b"AZCS"`?
#[inline]
#[must_use]
pub const fn is_azcs(bytes: &[u8]) -> bool {
    bytes.len() >= AZCS_SIGNATURE.len()
        && bytes[0] == AZCS_SIGNATURE[0]
        && bytes[1] == AZCS_SIGNATURE[1]
        && bytes[2] == AZCS_SIGNATURE[2]
        && bytes[3] == AZCS_SIGNATURE[3]
}

/// Wrap `reader` in an AZCS decompressor. Reads + validates the
/// header, then returns a streaming reader over the decompressed
/// payload.
///
/// Currently only [`AzcsId::Zlib`] is implemented; ZSTD returns
/// [`AzcsError::Zstd`].
///
/// # Errors
///
/// Propagates every [`AzcsHeader::from_reader`] failure
/// ([`AzcsError::Io`], [`AzcsError::InvalidSignature`],
/// [`AzcsError::UnsupportedCompressor`]), returns [`AzcsError::Io`] when the
/// 4-byte block-size hint that follows the header is truncated, and
/// [`AzcsError::Zstd`] for ZSTD payloads, which are not implemented.
pub fn decompress<R: Read>(reader: &mut R) -> Result<impl Read + use<'_, R>, AzcsError> {
    let header = AzcsHeader::from_reader(reader)?;
    match header.compressor_id {
        AzcsId::Zlib => {
            // O3DE's CompressorZLibHeader stores a big-endian seek-point count
            // between the common AZCS header and the zlib stream. This reader
            // supports the single-stream payload and skips that count.
            let mut skip = [0u8; 4];
            reader.read_exact(&mut skip)?;
            Ok(ZlibDecoder::new(reader))
        }
        AzcsId::Zstd => Err(AzcsError::Zstd),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn azcs_id_round_trip() {
        assert_eq!(AzcsId::from_u32(0x7388_7D3A), Some(AzcsId::Zlib));
        assert_eq!(AzcsId::from_u32(0x72FD_505E), Some(AzcsId::Zstd));
        assert_eq!(AzcsId::from_u32(0xDEAD_BEEF), None);
        assert_eq!(AzcsId::Zlib.as_u32(), 0x7388_7D3A);
        assert_eq!(AzcsId::Zstd.as_u32(), 0x72FD_505E);
    }

    #[test]
    fn is_azcs_check() {
        assert!(is_azcs(b"AZCS extra"));
        assert!(!is_azcs(b"AZC"));
        assert!(!is_azcs(b"NOPE"));
        assert!(!is_azcs(b""));
    }

    #[test]
    fn peek_header() {
        let mut buf = Vec::new();
        buf.extend_from_slice(AZCS_SIGNATURE);
        buf.extend_from_slice(&AzcsId::Zlib.as_u32().to_be_bytes());
        buf.extend_from_slice(&1234u64.to_be_bytes());
        let h = AzcsHeader::peek(&buf).unwrap();
        assert_eq!(h.compressor_id, AzcsId::Zlib);
        assert_eq!(h.uncompressed_size, 1234);

        assert!(AzcsHeader::peek(b"NOPE12345678abcd").is_none());
        assert!(AzcsHeader::peek(b"AZCS").is_none()); // too short
    }
}
