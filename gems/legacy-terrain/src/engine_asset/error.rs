//! Terrain engine asset errors.

use thiserror::Error;

/// Error for engine terrain-region reads and writes.
#[derive(Debug, Error)]
pub enum TerrainRegionAssetFormatError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad terrain region magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported terrain region version {0}")]
    UnsupportedVersion(u32),
    #[error("{what} count {count} exceeds u32")]
    TooManyItems { what: &'static str, count: usize },
    #[error("invalid terrain region data: {0}")]
    InvalidData(&'static str),
    #[error("invalid UTF-8 string: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

/// Error for engine terrain world manifest reads and writes.
#[derive(Debug, Error)]
pub enum TerrainWorldManifestFormatError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad terrain world manifest magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported terrain world manifest version {version}, expected {expected}")]
    UnsupportedVersion { version: u32, expected: u32 },
    #[error("{what} count {count} exceeds u32")]
    TooManyItems { what: &'static str, count: usize },
    #[error("invalid UTF-8 string: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}
