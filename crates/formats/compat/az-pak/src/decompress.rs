//! Per-zip-entry decompression.
//!
//! Handles stored, deflated, and optional Oodle entries, then peels an AZCS
//! inner wrapper when present.

use std::fmt;
use std::io::{self, Cursor, Read};

use flate2::Decompress;
use thiserror::Error;
use zip::CompressionMethod;
use zip::read::ZipFile;

use crate::azcs::{self, AzcsError};

/// Compression method used for a pak entry.
///
/// Wraps [`zip::CompressionMethod`] with support for the non-standard method
/// `15` Oodle extension.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Compression {
    /// `Stored` — bytes are not compressed.
    Stored,
    /// `Deflated` — RFC 1951 DEFLATE, optionally wrapped in a zlib
    /// header (`0x78 0xda` sniff fall-through).
    Deflated,
    /// Optional Oodle extension (compression method `15`).
    Oodle,
    /// Any other method we don't currently handle. Surfaces as
    /// [`DecompressError::UnsupportedMethod`] when reading.
    Other(u16),
}

impl Compression {
    /// Classify a [`zip::CompressionMethod`] without reading bytes.
    #[must_use]
    pub const fn from_zip_method(method: CompressionMethod) -> Self {
        #[allow(deprecated)]
        match method {
            CompressionMethod::Stored => Self::Stored,
            CompressionMethod::Deflated => Self::Deflated,
            CompressionMethod::Unsupported(15) => Self::Oodle,
            CompressionMethod::Unsupported(other) => Self::Other(other),
            // Other variants are unsupported and retain no specialized
            // decoding path.
            _ => Self::Other(u16::MAX),
        }
    }
}

impl fmt::Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stored => f.write_str("stored"),
            Self::Deflated => f.write_str("deflated"),
            Self::Oodle => f.write_str("oodle"),
            Self::Other(method) => write!(f, "other({method})"),
        }
    }
}

#[derive(Debug, Error)]
pub enum DecompressError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("zip compression {0:?} is not supported by az-pak")]
    UnsupportedMethod(Compression),

    #[error("oodle decompression failed (entry size {expected_size} bytes)")]
    Oodle { expected_size: usize },

    #[error("entry size {size} does not fit in this target's address space")]
    EntryTooLarge { size: u64 },

    #[error("azcs inner-decompression failed: {0}")]
    Azcs(#[from] AzcsError),
}

/// Decompress one zip entry into a freshly allocated `Vec<u8>`.
///
/// Convenience wrapper around [`decompress_zip_entry_into`] that
/// allocates a buffer sized to the entry's `uncompressed_size`. Use
/// the `_into` variant when you can reuse a buffer across many
/// reads.
///
/// # Errors
///
/// Propagates [`decompress_zip_entry_into`]: [`DecompressError::Io`] on a
/// short or failed read of the entry payload,
/// [`DecompressError::UnsupportedMethod`] for compression methods other than
/// `Stored`/`Deflated`/Oodle (and for Oodle when the `oodle` feature is
/// disabled), [`DecompressError::Oodle`] when the Oodle decoder rejects the
/// payload, and [`DecompressError::Azcs`] when the inner AZCS wrapper cannot
/// be peeled.
#[inline]
pub fn decompress_zip_entry<R: Read + ?Sized>(
    entry: &mut ZipFile<'_, R>,
) -> Result<Vec<u8>, DecompressError> {
    // Capacity is only a hint: if the recorded size does not fit in `usize`
    // (32-bit targets), start unsized and let the buffer grow instead of
    // silently truncating the reservation.
    let mut out = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
    decompress_zip_entry_into(entry, &mut out)?;
    Ok(out)
}

/// Decompress one zip entry into a caller-supplied buffer.
///
/// Clears `out` first, then fills it. Handles `Stored`, `Deflated`
/// (with optional `0x78 0xda` zlib header), and the optional
/// `Unsupported(15)` Oodle extension. Peels the AZCS inner wrapper if
/// present.
///
/// Returns the final decompressed length (i.e. `out.len()`).
///
/// # Errors
///
/// Returns [`DecompressError::Io`] when the entry payload cannot be read or
/// a decoder stream fails, [`DecompressError::UnsupportedMethod`] for
/// compression methods other than `Stored`/`Deflated`/Oodle (and for Oodle
/// when the `oodle` feature is disabled), [`DecompressError::Oodle`] when the
/// Oodle decoder rejects the payload, and [`DecompressError::Azcs`] when the
/// decompressed bytes carry an AZCS wrapper that cannot be peeled.
pub fn decompress_zip_entry_into<R: Read + ?Sized>(
    entry: &mut ZipFile<'_, R>,
    out: &mut Vec<u8>,
) -> Result<usize, DecompressError> {
    out.clear();
    // Reservation is only a hint: if the recorded size does not fit in
    // `usize` (32-bit targets), reserve nothing rather than silently
    // truncating the request.
    out.reserve(usize::try_from(entry.size()).unwrap_or(0));

    if entry.size() == 0 {
        return Ok(0);
    }

    let compression = Compression::from_zip_method(entry.compression());
    match compression {
        Compression::Stored => {
            io::copy(entry, out)?;
        }
        Compression::Deflated => {
            // Sniff for a zlib header (`0x78 0xda`); fall back to raw DEFLATE.
            let mut sig = [0u8; 2];
            entry.read_exact(&mut sig)?;
            if sig == [0x78, 0xda] {
                let mut zlib = flate2::read::ZlibDecoder::new_with_decompress(
                    Cursor::new(sig).chain(entry),
                    Decompress::new(true),
                );
                io::copy(&mut zlib, out)?;
            } else {
                let mut deflate = flate2::read::DeflateDecoder::new(Cursor::new(sig).chain(entry));
                io::copy(&mut deflate, out)?;
            }
        }
        #[cfg(feature = "oodle")]
        Compression::Oodle => {
            // Unlike the reservations above, this one is load-bearing: it
            // pre-sizes the buffer Oodle writes into, so a truncated value
            // would silently produce a short result rather than a slow one.
            let expected_size = usize::try_from(entry.size())
                .map_err(|_| DecompressError::EntryTooLarge { size: entry.size() })?;
            let mut compressed =
                Vec::with_capacity(usize::try_from(entry.compressed_size()).unwrap_or(0));
            io::copy(entry, &mut compressed)?;
            out.resize(expected_size, 0);
            oodle_safe::decompress(
                &compressed,
                out.as_mut_slice(),
                None,
                None,
                None,
                Some(oodle_safe::DecodeThreadPhase::All),
            )
            .map_err(|_| DecompressError::Oodle { expected_size })?;
        }
        #[cfg(not(feature = "oodle"))]
        Compression::Oodle => {
            return Err(DecompressError::UnsupportedMethod(Compression::Oodle));
        }
        other @ Compression::Other(_) => {
            return Err(DecompressError::UnsupportedMethod(other));
        }
    }

    peel_azcs(out)?;
    Ok(out.len())
}

/// Decompress raw compressed bytes (no zip framing) into a
/// caller-supplied buffer.
///
/// Same compressors as [`decompress_zip_entry_into`], but works
/// from the raw bytes you'd get by reading the zip entry's
/// payload directly. Used by [`crate::PakFile::extract_parallel`]
/// after raw-`pread`-style entry reads bypass `zip::ZipFile`.
///
/// `expected_uncompressed_size` is used to size the output buffer
/// up-front for `Oodle` (which writes into a pre-sized buffer)
/// and as a hint elsewhere.
///
/// # Errors
///
/// Returns [`DecompressError::Io`] when a decoder stream fails,
/// [`DecompressError::UnsupportedMethod`] for compression methods other than
/// `Stored`/`Deflated`/Oodle (and for Oodle when the `oodle` feature is
/// disabled), [`DecompressError::Oodle`] when the Oodle decoder rejects the
/// payload, and [`DecompressError::Azcs`] when the decompressed bytes carry
/// an AZCS wrapper that cannot be peeled.
pub fn decompress_bytes_into(
    method: Compression,
    compressed: &[u8],
    expected_uncompressed_size: usize,
    out: &mut Vec<u8>,
) -> Result<usize, DecompressError> {
    out.clear();
    out.reserve(expected_uncompressed_size);

    if compressed.is_empty() {
        return Ok(0);
    }

    match method {
        Compression::Stored => {
            out.extend_from_slice(compressed);
        }
        Compression::Deflated => {
            if compressed.len() >= 2 && compressed[0] == 0x78 && compressed[1] == 0xda {
                let mut zlib = flate2::read::ZlibDecoder::new_with_decompress(
                    Cursor::new(compressed),
                    Decompress::new(true),
                );
                io::copy(&mut zlib, out)?;
            } else {
                let mut deflate = flate2::read::DeflateDecoder::new(Cursor::new(compressed));
                io::copy(&mut deflate, out)?;
            }
        }
        #[cfg(feature = "oodle")]
        Compression::Oodle => {
            out.resize(expected_uncompressed_size, 0);
            oodle_safe::decompress(
                compressed,
                out.as_mut_slice(),
                None,
                None,
                None,
                Some(oodle_safe::DecodeThreadPhase::All),
            )
            .map_err(|_| DecompressError::Oodle {
                expected_size: expected_uncompressed_size,
            })?;
        }
        #[cfg(not(feature = "oodle"))]
        Compression::Oodle => {
            return Err(DecompressError::UnsupportedMethod(Compression::Oodle));
        }
        other @ Compression::Other(_) => {
            return Err(DecompressError::UnsupportedMethod(other));
        }
    }

    peel_azcs(out)?;
    Ok(out.len())
}

/// Internal: if `out` starts with `b"AZCS"`, peel the inner AZCS
/// wrapper. AZCS may be nested inside the zip compression layer.
fn peel_azcs(out: &mut Vec<u8>) -> Result<(), DecompressError> {
    if azcs::is_azcs(out.as_slice()) {
        let mut inner = Vec::with_capacity(out.len());
        let outer = std::mem::take(out);
        {
            let mut cursor = Cursor::new(outer.as_slice());
            let mut reader = azcs::decompress(&mut cursor)?;
            io::copy(&mut reader, &mut inner)?;
        }
        *out = inner;
    }
    Ok(())
}

#[cfg(all(test, not(feature = "oodle")))]
mod tests {
    use super::*;

    #[test]
    fn disabled_oodle_provider_is_reported_as_unsupported() {
        let mut output = Vec::new();

        let error =
            decompress_bytes_into(Compression::Oodle, b"encoded", 32, &mut output).unwrap_err();

        assert!(matches!(
            error,
            DecompressError::UnsupportedMethod(Compression::Oodle)
        ));
    }
}
