//! Native Azoth package index format.
//!
//! `azpack` packages are a small typed index plus content-addressed chunk
//! files. The index is the runtime-facing lookup table; chunk bytes remain
//! independent files so package updates can ship only changed chunks.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use az_filesystem::safe_join;
use thiserror::Error;
use uuid::Uuid;

use crate::AssetId;
use crate::catalog::AssetTreePath;
use crate::package::{PACKAGE_CONTENT_HASH_BYTES, format_package_content_hash_hex};

pub const AZPACK_INDEX_VERSION: u32 = 1;
pub const AZPACK_INDEX_MAGIC: &[u8; 8] = b"AZPKIDX\0";
pub const AZPACK_INDEX_FILE_NAME: &str = "package.azpack.index";
pub const AZPACK_DEFAULT_CHUNK_SIZE: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzPackIndex {
    pub version: u32,
    pub profile: AzPackIndexProfile,
    pub chunk_size: u32,
    pub entries: Vec<AzPackIndexEntry>,
}

impl AzPackIndex {
    /// # Errors
    ///
    /// Returns [`AzPackIndexError`] when the assembled index fails
    /// [`AzPackIndex::validate`].
    pub fn new(
        profile: AzPackIndexProfile,
        chunk_size: u32,
        entries: Vec<AzPackIndexEntry>,
    ) -> Result<Self, AzPackIndexError> {
        let index = Self {
            version: AZPACK_INDEX_VERSION,
            profile,
            chunk_size,
            entries,
        };
        index.validate()?;
        Ok(index)
    }

    /// # Errors
    ///
    /// Returns [`AzPackIndexError`] when the version, chunk size, profile
    /// text, or any entry/chunk fails the azpack index invariants.
    pub fn validate(&self) -> Result<(), AzPackIndexError> {
        if self.version != AZPACK_INDEX_VERSION {
            return Err(AzPackIndexError::UnsupportedVersion {
                version: self.version,
                expected: AZPACK_INDEX_VERSION,
            });
        }
        if self.chunk_size == 0 {
            return Err(AzPackIndexError::InvalidChunkSize {
                chunk_size: self.chunk_size,
            });
        }
        validate_text("profile.name", &self.profile.name, "<profile>")?;
        validate_text(
            "profile.asset_platform",
            &self.profile.asset_platform,
            "<profile>",
        )?;
        validate_text(
            "profile.compression",
            &self.profile.compression,
            "<profile>",
        )?;

        let mut product_paths = HashSet::new();
        let mut asset_ids = HashSet::new();
        for entry in &self.entries {
            validate_asset_tree_path("product_path", &entry.product_path)?;
            validate_asset_tree_path("source_path", &entry.source_path)?;
            validate_text("job_key", &entry.job_key, entry.product_path.as_str())?;
            if entry.source_asset_guid.is_nil() {
                return Err(AzPackIndexError::NilSourceAssetGuid {
                    product_path: entry.product_path.to_string(),
                });
            }
            if entry.asset_type.is_nil() {
                return Err(AzPackIndexError::NilAssetType {
                    product_path: entry.product_path.to_string(),
                });
            }
            if !product_paths.insert(entry.product_path.clone()) {
                return Err(AzPackIndexError::DuplicateProductPath {
                    product_path: entry.product_path.to_string(),
                });
            }
            let asset_id = entry.asset_id();
            if !asset_ids.insert(asset_id) {
                return Err(AzPackIndexError::DuplicateAssetId {
                    asset_guid: asset_id.guid,
                    sub_id: entry.sub_id,
                });
            }
            validate_entry_chunks(entry)?;
        }
        Ok(())
    }

    #[inline]
    #[must_use]
    pub fn entries(&self) -> &[AzPackIndexEntry] {
        &self.entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzPackIndexProfile {
    pub name: String,
    pub asset_platform: String,
    pub compression: String,
}

impl AzPackIndexProfile {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        asset_platform: impl Into<String>,
        compression: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            asset_platform: asset_platform.into(),
            compression: compression.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzPackIndexEntry {
    pub product_path: AssetTreePath,
    pub source_path: AssetTreePath,
    pub job_key: String,
    pub source_asset_guid: Uuid,
    pub asset_type: Uuid,
    pub sub_id: u32,
    pub byte_len: u64,
    pub content_hash: [u8; PACKAGE_CONTENT_HASH_BYTES],
    pub chunks: Vec<AzPackIndexChunk>,
}

impl AzPackIndexEntry {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        product_path: impl Into<AssetTreePath>,
        source_path: impl Into<AssetTreePath>,
        job_key: impl Into<String>,
        source_asset_guid: Uuid,
        asset_type: Uuid,
        sub_id: u32,
        byte_len: u64,
        content_hash: [u8; PACKAGE_CONTENT_HASH_BYTES],
        chunks: Vec<AzPackIndexChunk>,
    ) -> Self {
        Self {
            product_path: product_path.into(),
            source_path: source_path.into(),
            job_key: job_key.into(),
            source_asset_guid,
            asset_type,
            sub_id,
            byte_len,
            content_hash,
            chunks,
        }
    }

    #[inline]
    #[must_use]
    pub const fn asset_id(&self) -> AssetId {
        AssetId::new(self.source_asset_guid, self.sub_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzPackIndexChunk {
    pub raw_offset: u64,
    pub raw_len: u32,
    pub encoded_len: u32,
    pub compression: AzPackChunkCompression,
    pub raw_hash: [u8; PACKAGE_CONTENT_HASH_BYTES],
    pub encoded_hash: [u8; PACKAGE_CONTENT_HASH_BYTES],
    pub chunk_path: AssetTreePath,
}

impl AzPackIndexChunk {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        raw_offset: u64,
        raw_len: u32,
        encoded_len: u32,
        compression: AzPackChunkCompression,
        raw_hash: [u8; PACKAGE_CONTENT_HASH_BYTES],
        encoded_hash: [u8; PACKAGE_CONTENT_HASH_BYTES],
        chunk_path: impl Into<AssetTreePath>,
    ) -> Self {
        Self {
            raw_offset,
            raw_len,
            encoded_len,
            compression,
            raw_hash,
            encoded_hash,
            chunk_path: chunk_path.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzPackChunkCompression {
    Stored,
    Oodle,
}

impl AzPackChunkCompression {
    pub const STORED_CODE: u8 = 0;
    pub const OODLE_CODE: u8 = 1;

    #[inline]
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Stored => Self::STORED_CODE,
            Self::Oodle => Self::OODLE_CODE,
        }
    }

    const fn from_code(code: u8) -> Result<Self, AzPackIndexError> {
        match code {
            Self::STORED_CODE => Ok(Self::Stored),
            Self::OODLE_CODE => Ok(Self::Oodle),
            _ => Err(AzPackIndexError::UnknownChunkCompression { code }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AzPackReader {
    root: PathBuf,
    index: AzPackIndex,
    entries_by_path: HashMap<AssetTreePath, usize>,
    entries_by_asset_id: HashMap<AssetId, usize>,
}

impl AzPackReader {
    /// # Errors
    ///
    /// Returns [`AzPackReadError`] when the index file cannot be opened,
    /// decoded, or fails validation.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, AzPackReadError> {
        let root = root.as_ref().to_path_buf();
        let file = File::open(root.join(AZPACK_INDEX_FILE_NAME))?;
        let index = read_azpack_index(BufReader::new(file))?;
        Self::from_index(root, index)
    }

    /// # Errors
    ///
    /// Returns [`AzPackReadError`] when `index` fails validation.
    pub fn from_index(
        root: impl Into<PathBuf>,
        index: AzPackIndex,
    ) -> Result<Self, AzPackReadError> {
        index.validate()?;
        let mut entries_by_path = HashMap::with_capacity(index.entries.len());
        let mut entries_by_asset_id = HashMap::with_capacity(index.entries.len());
        for (position, entry) in index.entries.iter().enumerate() {
            entries_by_path.insert(entry.product_path.clone(), position);
            entries_by_asset_id.insert(entry.asset_id(), position);
        }
        Ok(Self {
            root: root.into(),
            index,
            entries_by_path,
            entries_by_asset_id,
        })
    }

    #[inline]
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[inline]
    #[must_use]
    pub const fn index(&self) -> &AzPackIndex {
        &self.index
    }

    #[must_use]
    pub fn entry_by_path(&self, product_path: impl AsRef<str>) -> Option<&AzPackIndexEntry> {
        let product_path = AssetTreePath::new(product_path.as_ref());
        self.entries_by_path
            .get(&product_path)
            .and_then(|position| self.index.entries.get(*position))
    }

    #[must_use]
    pub fn entry_by_asset_id(&self, asset_guid: Uuid, sub_id: u32) -> Option<&AzPackIndexEntry> {
        self.entries_by_asset_id
            .get(&AssetId::new(asset_guid, sub_id))
            .and_then(|position| self.index.entries.get(*position))
    }

    /// # Errors
    ///
    /// Returns [`AzPackReadError`] when `product_path` is not present in the
    /// index, or reading/validating its chunks fails.
    pub fn read_product_path(
        &self,
        product_path: impl AsRef<str>,
    ) -> Result<Vec<u8>, AzPackReadError> {
        let product_path = AssetTreePath::new(product_path.as_ref());
        let entry = self
            .entries_by_path
            .get(&product_path)
            .and_then(|position| self.index.entries.get(*position))
            .ok_or_else(|| AzPackReadError::MissingProductPath {
                product_path: product_path.to_string(),
            })?;
        self.read_product(entry)
    }

    /// # Errors
    ///
    /// Returns [`AzPackReadError`] when the asset id is not present in the
    /// index, or reading/validating its chunks fails.
    pub fn read_product_asset_id(
        &self,
        asset_guid: Uuid,
        sub_id: u32,
    ) -> Result<Vec<u8>, AzPackReadError> {
        let asset_id = AssetId::new(asset_guid, sub_id);
        let entry = self
            .entries_by_asset_id
            .get(&asset_id)
            .and_then(|position| self.index.entries.get(*position))
            .ok_or(AzPackReadError::MissingAssetId { asset_id })?;
        self.read_product(entry)
    }

    /// # Errors
    ///
    /// Returns [`AzPackReadError`] when a chunk fails to decode, or the
    /// assembled product's length or content hash does not match `entry`.
    pub fn read_product(&self, entry: &AzPackIndexEntry) -> Result<Vec<u8>, AzPackReadError> {
        let capacity =
            usize::try_from(entry.byte_len).map_err(|_| AzPackReadError::ProductTooLarge {
                product_path: entry.product_path.to_string(),
                byte_len: entry.byte_len,
            })?;
        let mut bytes = Vec::with_capacity(capacity);
        for chunk in &entry.chunks {
            bytes.extend_from_slice(&self.read_chunk(entry, chunk)?);
        }

        if bytes.len() as u64 != entry.byte_len {
            return Err(AzPackReadError::ProductLengthMismatch {
                product_path: entry.product_path.to_string(),
                expected: entry.byte_len,
                actual: bytes.len() as u64,
            });
        }

        let actual_hash = *blake3::hash(&bytes).as_bytes();
        if actual_hash != entry.content_hash {
            return Err(AzPackReadError::ProductHashMismatch {
                product_path: entry.product_path.to_string(),
                expected: format_package_content_hash_hex(&entry.content_hash),
                actual: format_package_content_hash_hex(&actual_hash),
            });
        }

        Ok(bytes)
    }

    /// # Errors
    ///
    /// Returns [`AzPackReadError`] when the chunk file is missing, fails an
    /// encoded/raw length or hash check, or fails to decode.
    pub fn read_chunk(
        &self,
        entry: &AzPackIndexEntry,
        chunk: &AzPackIndexChunk,
    ) -> Result<Vec<u8>, AzPackReadError> {
        let encoded_path = safe_join(&self.root, chunk.chunk_path.as_str()).map_err(|source| {
            AzPackReadError::InvalidChunkPath {
                root: self.root.clone(),
                chunk_path: chunk.chunk_path.to_string(),
                source,
            }
        })?;
        if !encoded_path.is_file() {
            return Err(AzPackReadError::MissingChunk {
                product_path: entry.product_path.to_string(),
                chunk_path: chunk.chunk_path.to_string(),
                path: encoded_path,
            });
        }

        let encoded = fs::read(&encoded_path)?;
        if encoded.len() as u64 != u64::from(chunk.encoded_len) {
            return Err(AzPackReadError::ChunkEncodedLengthMismatch {
                product_path: entry.product_path.to_string(),
                chunk_path: chunk.chunk_path.to_string(),
                expected: chunk.encoded_len,
                actual: encoded.len() as u64,
            });
        }
        let actual_encoded_hash = *blake3::hash(&encoded).as_bytes();
        if actual_encoded_hash != chunk.encoded_hash {
            return Err(AzPackReadError::ChunkEncodedHashMismatch {
                product_path: entry.product_path.to_string(),
                chunk_path: chunk.chunk_path.to_string(),
                expected: format_package_content_hash_hex(&chunk.encoded_hash),
                actual: format_package_content_hash_hex(&actual_encoded_hash),
            });
        }

        let decoded = match chunk.compression {
            AzPackChunkCompression::Stored => encoded,
            AzPackChunkCompression::Oodle => {
                #[cfg(not(feature = "oodle"))]
                return Err(AzPackReadError::OodleUnavailable {
                    product_path: entry.product_path.to_string(),
                    chunk_path: chunk.chunk_path.to_string(),
                });
                #[cfg(feature = "oodle")]
                {
                    let mut output = vec![0_u8; chunk.raw_len as usize];
                    let decoded_len = oodle_safe::decompress(
                        &encoded,
                        &mut output,
                        None,
                        None,
                        None,
                        Some(oodle_safe::DecodeThreadPhase::All),
                    )
                    .map_err(|_| AzPackReadError::OodleDecode {
                        product_path: entry.product_path.to_string(),
                        chunk_path: chunk.chunk_path.to_string(),
                        expected_size: chunk.raw_len,
                    })?;
                    if decoded_len != chunk.raw_len as usize {
                        return Err(AzPackReadError::ChunkRawLengthMismatch {
                            product_path: entry.product_path.to_string(),
                            chunk_path: chunk.chunk_path.to_string(),
                            expected: chunk.raw_len,
                            actual: decoded_len as u64,
                        });
                    }
                    output
                }
            }
        };

        if decoded.len() as u64 != u64::from(chunk.raw_len) {
            return Err(AzPackReadError::ChunkRawLengthMismatch {
                product_path: entry.product_path.to_string(),
                chunk_path: chunk.chunk_path.to_string(),
                expected: chunk.raw_len,
                actual: decoded.len() as u64,
            });
        }
        let actual_raw_hash = *blake3::hash(&decoded).as_bytes();
        if actual_raw_hash != chunk.raw_hash {
            return Err(AzPackReadError::ChunkRawHashMismatch {
                product_path: entry.product_path.to_string(),
                chunk_path: chunk.chunk_path.to_string(),
                expected: format_package_content_hash_hex(&chunk.raw_hash),
                actual: format_package_content_hash_hex(&actual_raw_hash),
            });
        }

        Ok(decoded)
    }
}

/// # Errors
///
/// Returns [`AzPackIndexError`] when `index` fails validation or the writer
/// returns an I/O error.
pub fn write_azpack_index(
    index: &AzPackIndex,
    mut writer: impl Write,
) -> Result<(), AzPackIndexError> {
    index.validate()?;
    writer.write_all(AZPACK_INDEX_MAGIC)?;
    write_u32(&mut writer, AZPACK_INDEX_VERSION)?;
    write_text(
        &mut writer,
        "profile.name",
        &index.profile.name,
        "<profile>",
    )?;
    write_text(
        &mut writer,
        "profile.asset_platform",
        &index.profile.asset_platform,
        "<profile>",
    )?;
    write_text(
        &mut writer,
        "profile.compression",
        &index.profile.compression,
        "<profile>",
    )?;
    write_u32(&mut writer, index.chunk_size)?;
    write_u32(
        &mut writer,
        u32::try_from(index.entries.len()).map_err(|_| AzPackIndexError::TooManyEntries {
            count: index.entries.len(),
        })?,
    )?;

    for entry in &index.entries {
        writer.write_all(entry.source_asset_guid.as_bytes())?;
        writer.write_all(entry.asset_type.as_bytes())?;
        write_u32(&mut writer, entry.sub_id)?;
        write_u64(&mut writer, entry.byte_len)?;
        writer.write_all(&entry.content_hash)?;
        write_text(
            &mut writer,
            "product_path",
            entry.product_path.as_str(),
            entry.product_path.as_str(),
        )?;
        write_text(
            &mut writer,
            "source_path",
            entry.source_path.as_str(),
            entry.product_path.as_str(),
        )?;
        write_text(
            &mut writer,
            "job_key",
            &entry.job_key,
            entry.product_path.as_str(),
        )?;
        write_u32(
            &mut writer,
            u32::try_from(entry.chunks.len()).map_err(|_| AzPackIndexError::TooManyChunks {
                product_path: entry.product_path.to_string(),
                count: entry.chunks.len(),
            })?,
        )?;
        for chunk in &entry.chunks {
            write_u64(&mut writer, chunk.raw_offset)?;
            write_u32(&mut writer, chunk.raw_len)?;
            write_u32(&mut writer, chunk.encoded_len)?;
            writer.write_all(&[chunk.compression.code()])?;
            writer.write_all(&chunk.raw_hash)?;
            writer.write_all(&chunk.encoded_hash)?;
            write_text(
                &mut writer,
                "chunk_path",
                chunk.chunk_path.as_str(),
                entry.product_path.as_str(),
            )?;
        }
    }
    Ok(())
}

/// # Errors
///
/// Returns [`AzPackIndexError`] when the stream has a bad magic, an
/// unsupported version, malformed text, or the decoded index fails
/// validation.
pub fn read_azpack_index(mut reader: impl Read) -> Result<AzPackIndex, AzPackIndexError> {
    let mut magic = [0_u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != AZPACK_INDEX_MAGIC {
        return Err(AzPackIndexError::BadMagic { found: magic });
    }

    let version = read_u32(&mut reader)?;
    if version != AZPACK_INDEX_VERSION {
        return Err(AzPackIndexError::UnsupportedVersion {
            version,
            expected: AZPACK_INDEX_VERSION,
        });
    }

    let profile = AzPackIndexProfile::new(
        read_text(&mut reader)?,
        read_text(&mut reader)?,
        read_text(&mut reader)?,
    );
    let chunk_size = read_u32(&mut reader)?;
    let entry_count = read_u32(&mut reader)?;
    let mut entries = Vec::new();

    for _ in 0..entry_count {
        let source_asset_guid = read_uuid(&mut reader)?;
        let asset_type = read_uuid(&mut reader)?;
        let sub_id = read_u32(&mut reader)?;
        let byte_len = read_u64(&mut reader)?;
        let mut content_hash = [0_u8; PACKAGE_CONTENT_HASH_BYTES];
        reader.read_exact(&mut content_hash)?;
        let product_path = read_text(&mut reader)?;
        let source_path = read_text(&mut reader)?;
        let job_key = read_text(&mut reader)?;
        let chunk_count = read_u32(&mut reader)?;
        let mut chunks = Vec::new();

        for _ in 0..chunk_count {
            let raw_offset = read_u64(&mut reader)?;
            let raw_len = read_u32(&mut reader)?;
            let encoded_len = read_u32(&mut reader)?;
            let compression = AzPackChunkCompression::from_code(read_u8(&mut reader)?)?;
            let mut raw_hash = [0_u8; PACKAGE_CONTENT_HASH_BYTES];
            reader.read_exact(&mut raw_hash)?;
            let mut encoded_hash = [0_u8; PACKAGE_CONTENT_HASH_BYTES];
            reader.read_exact(&mut encoded_hash)?;
            let chunk_path = read_text(&mut reader)?;
            chunks.push(AzPackIndexChunk::new(
                raw_offset,
                raw_len,
                encoded_len,
                compression,
                raw_hash,
                encoded_hash,
                chunk_path,
            ));
        }

        entries.push(AzPackIndexEntry::new(
            product_path,
            source_path,
            job_key,
            source_asset_guid,
            asset_type,
            sub_id,
            byte_len,
            content_hash,
            chunks,
        ));
    }

    let index = AzPackIndex {
        version,
        profile,
        chunk_size,
        entries,
    };
    index.validate()?;
    Ok(index)
}

#[derive(Debug, Error)]
pub enum AzPackIndexError {
    #[error("bad azpack index magic: {found:?}")]
    BadMagic { found: [u8; 8] },

    #[error("unsupported azpack index version {version}, expected {expected}")]
    UnsupportedVersion { version: u32, expected: u32 },

    #[error("{field} cannot be empty")]
    EmptyText {
        field: &'static str,
        product_path: String,
    },

    #[error("{field} for `{product_path}` is too long: {byte_len} bytes")]
    TextTooLong {
        field: &'static str,
        product_path: String,
        byte_len: usize,
    },

    #[error("{field} path `{path}` must be an asset-tree relative path")]
    InvalidPath { field: &'static str, path: String },

    #[error("azpack chunk path `{path}` must stay under chunks/ and end in .azchunk")]
    InvalidChunkPath { path: String },

    #[error("azpack index has too many entries: {count}")]
    TooManyEntries { count: usize },

    #[error("azpack entry `{product_path}` has too many chunks: {count}")]
    TooManyChunks { product_path: String, count: usize },

    #[error("azpack index chunk size {chunk_size} is invalid")]
    InvalidChunkSize { chunk_size: u32 },

    #[error("entry `{product_path}` has nil source asset guid")]
    NilSourceAssetGuid { product_path: String },

    #[error("entry `{product_path}` has nil asset type")]
    NilAssetType { product_path: String },

    #[error("duplicate azpack product path `{product_path}`")]
    DuplicateProductPath { product_path: String },

    #[error("duplicate azpack asset id {asset_guid}:{sub_id}")]
    DuplicateAssetId { asset_guid: Uuid, sub_id: u32 },

    #[error("azpack chunk compression code {code} is unknown")]
    UnknownChunkCompression { code: u8 },

    #[error("azpack chunk for `{product_path}` has zero length")]
    EmptyChunk { product_path: String },

    #[error(
        "azpack chunk for `{product_path}` starts at {actual_offset}, expected {expected_offset}"
    )]
    InvalidChunkOffset {
        product_path: String,
        expected_offset: u64,
        actual_offset: u64,
    },

    #[error("azpack chunks for `{product_path}` cover {actual} bytes, expected {expected}")]
    ChunkLengthMismatch {
        product_path: String,
        expected: u64,
        actual: u64,
    },

    #[error("invalid UTF-8 text: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum AzPackReadError {
    #[error(transparent)]
    Index(#[from] AzPackIndexError),

    #[error("azpack product path `{product_path}` is not present")]
    MissingProductPath { product_path: String },

    #[error("azpack asset id {asset_id} is not present")]
    MissingAssetId { asset_id: AssetId },

    #[error("azpack product `{product_path}` is too large to read: {byte_len} bytes")]
    ProductTooLarge { product_path: String, byte_len: u64 },

    #[error("azpack chunk path `{chunk_path}` is invalid under `{root}`: {source}")]
    InvalidChunkPath {
        root: PathBuf,
        chunk_path: String,
        #[source]
        source: az_filesystem::SafeJoinError,
    },

    #[error("azpack chunk `{chunk_path}` for `{product_path}` is missing at {path}")]
    MissingChunk {
        product_path: String,
        chunk_path: String,
        path: PathBuf,
    },

    #[error(
        "azpack encoded chunk `{chunk_path}` for `{product_path}` has {actual} bytes, expected {expected}"
    )]
    ChunkEncodedLengthMismatch {
        product_path: String,
        chunk_path: String,
        expected: u32,
        actual: u64,
    },

    #[error(
        "azpack encoded chunk `{chunk_path}` for `{product_path}` has hash {actual}, expected {expected}"
    )]
    ChunkEncodedHashMismatch {
        product_path: String,
        chunk_path: String,
        expected: String,
        actual: String,
    },

    #[error(
        "Oodle failed to decode azpack chunk `{chunk_path}` for `{product_path}` into {expected_size} bytes"
    )]
    OodleDecode {
        product_path: String,
        chunk_path: String,
        expected_size: u32,
    },

    #[error(
        "Oodle support is disabled while reading azpack chunk `{chunk_path}` for `{product_path}`"
    )]
    OodleUnavailable {
        product_path: String,
        chunk_path: String,
    },

    #[error(
        "azpack decoded chunk `{chunk_path}` for `{product_path}` has {actual} bytes, expected {expected}"
    )]
    ChunkRawLengthMismatch {
        product_path: String,
        chunk_path: String,
        expected: u32,
        actual: u64,
    },

    #[error(
        "azpack decoded chunk `{chunk_path}` for `{product_path}` has hash {actual}, expected {expected}"
    )]
    ChunkRawHashMismatch {
        product_path: String,
        chunk_path: String,
        expected: String,
        actual: String,
    },

    #[error("azpack product `{product_path}` has {actual} bytes, expected {expected}")]
    ProductLengthMismatch {
        product_path: String,
        expected: u64,
        actual: u64,
    },

    #[error("azpack product `{product_path}` has hash {actual}, expected {expected}")]
    ProductHashMismatch {
        product_path: String,
        expected: String,
        actual: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

fn validate_entry_chunks(entry: &AzPackIndexEntry) -> Result<(), AzPackIndexError> {
    if entry.byte_len == 0 {
        if entry.chunks.is_empty() {
            return Ok(());
        }
        return Err(AzPackIndexError::ChunkLengthMismatch {
            product_path: entry.product_path.to_string(),
            expected: 0,
            actual: entry.chunks.iter().fold(0_u64, |total, chunk| {
                total.saturating_add(u64::from(chunk.raw_len))
            }),
        });
    }

    let mut expected_offset = 0_u64;
    for chunk in &entry.chunks {
        if chunk.raw_len == 0 || chunk.encoded_len == 0 {
            return Err(AzPackIndexError::EmptyChunk {
                product_path: entry.product_path.to_string(),
            });
        }
        if chunk.raw_offset != expected_offset {
            return Err(AzPackIndexError::InvalidChunkOffset {
                product_path: entry.product_path.to_string(),
                expected_offset,
                actual_offset: chunk.raw_offset,
            });
        }
        validate_chunk_path(&chunk.chunk_path)?;
        expected_offset = expected_offset.saturating_add(u64::from(chunk.raw_len));
    }
    if expected_offset != entry.byte_len {
        return Err(AzPackIndexError::ChunkLengthMismatch {
            product_path: entry.product_path.to_string(),
            expected: entry.byte_len,
            actual: expected_offset,
        });
    }
    Ok(())
}

fn validate_chunk_path(path: &AssetTreePath) -> Result<(), AzPackIndexError> {
    validate_asset_tree_path("chunk_path", path)?;
    let text = path.as_str();
    if !text.starts_with("chunks/") || !text.ends_with(".azchunk") {
        return Err(AzPackIndexError::InvalidChunkPath {
            path: text.to_string(),
        });
    }
    Ok(())
}

fn validate_asset_tree_path(
    field: &'static str,
    path: &AssetTreePath,
) -> Result<(), AzPackIndexError> {
    let text = path.as_str();
    if text.is_empty()
        || text.starts_with('/')
        || text.contains('\\')
        || text.contains(':')
        || text
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(AzPackIndexError::InvalidPath {
            field,
            path: text.to_string(),
        });
    }
    Ok(())
}

fn validate_text(
    field: &'static str,
    text: &str,
    product_path: &str,
) -> Result<(), AzPackIndexError> {
    if text.is_empty() {
        return Err(AzPackIndexError::EmptyText {
            field,
            product_path: product_path.to_string(),
        });
    }
    u32::try_from(text.len()).map_err(|_| AzPackIndexError::TextTooLong {
        field,
        product_path: product_path.to_string(),
        byte_len: text.len(),
    })?;
    Ok(())
}

fn write_text(
    writer: &mut impl Write,
    field: &'static str,
    value: &str,
    product_path: &str,
) -> Result<(), AzPackIndexError> {
    validate_text(field, value, product_path)?;
    write_u32(
        writer,
        u32::try_from(value.len()).map_err(|_| AzPackIndexError::TextTooLong {
            field,
            product_path: product_path.to_string(),
            byte_len: value.len(),
        })?,
    )?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

fn read_text(reader: &mut impl Read) -> Result<String, AzPackIndexError> {
    let len = read_u32(reader)? as usize;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| AzPackIndexError::TextTooLong {
            field: "<encoded>",
            product_path: "<encoded>".to_string(),
            byte_len: len,
        })?;
    bytes.resize(len, 0);
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes)?)
}

fn read_uuid(reader: &mut impl Read) -> io::Result<Uuid> {
    let mut bytes = [0_u8; 16];
    reader.read_exact(&mut bytes)?;
    Ok(Uuid::from_bytes(bytes))
}

fn read_u8(reader: &mut impl Read) -> io::Result<u8> {
    let mut bytes = [0_u8; 1];
    reader.read_exact(&mut bytes)?;
    Ok(bytes[0])
}

fn read_u32(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn write_u32(writer: &mut impl Write, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u64(writer: &mut impl Write, value: u64) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_filesystem::platform_product_cache_dir;

    use crate::package::{
        PackageManifest, PackageManifestEntry, PackageManifestProfile,
        format_package_content_hash_hex,
    };
    use crate::package_payload::{PackagePayloadWriteRequest, write_package_payload};

    #[test]
    fn azpack_index_round_trips() {
        let index = sample_index();

        let mut bytes = Vec::new();
        write_azpack_index(&index, &mut bytes).unwrap();
        assert_eq!(&bytes[..AZPACK_INDEX_MAGIC.len()], AZPACK_INDEX_MAGIC);

        let decoded = read_azpack_index(bytes.as_slice()).unwrap();
        assert_eq!(decoded, index);
    }

    #[test]
    fn azpack_index_rejects_bad_magic() {
        let mut bytes = Vec::new();
        write_azpack_index(&sample_index(), &mut bytes).unwrap();
        bytes[0] = b'X';

        let error = read_azpack_index(bytes.as_slice()).unwrap_err();

        assert!(matches!(error, AzPackIndexError::BadMagic { .. }));
    }

    #[test]
    fn azpack_index_rejects_escaping_paths() {
        let mut index = sample_index();
        index.entries[0].chunks[0].chunk_path = AssetTreePath::new("chunks/../bad.azchunk");

        let error = write_azpack_index(&index, Vec::new()).unwrap_err();

        assert!(matches!(error, AzPackIndexError::InvalidPath { .. }));
    }

    #[test]
    fn azpack_index_rejects_chunk_gaps() {
        let mut index = sample_index();
        index.entries[0].chunks[0].raw_offset = 1;

        let error = write_azpack_index(&index, Vec::new()).unwrap_err();

        assert!(matches!(error, AzPackIndexError::InvalidChunkOffset { .. }));
    }

    #[test]
    fn azpack_reader_reads_stored_product_by_path_and_asset_id() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let bytes = b"compiled prefab bytes";
        let package_root = write_test_package(temp.path(), product_path, bytes, "none");

        let reader = AzPackReader::open(&package_root).unwrap();
        let by_path = reader.read_product_path(product_path).unwrap();
        let entry = reader.entry_by_path(product_path).unwrap();
        let by_asset_id = reader
            .read_product_asset_id(entry.source_asset_guid, entry.sub_id)
            .unwrap();

        assert_eq!(by_path, bytes);
        assert_eq!(by_asset_id, bytes);
    }

    #[cfg(feature = "oodle")]
    #[test]
    fn azpack_reader_reads_oodle_product() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let bytes = b"compiled prefab bytes compiled prefab bytes compiled prefab bytes";
        let package_root = write_test_package(temp.path(), product_path, bytes, "oodle");

        let reader = AzPackReader::open(&package_root).unwrap();

        assert_eq!(reader.read_product_path(product_path).unwrap(), bytes);
    }

    #[test]
    fn azpack_reader_rejects_corrupt_chunk_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let bytes = b"compiled prefab bytes";
        let package_root = write_test_package(temp.path(), product_path, bytes, "none");
        let reader = AzPackReader::open(&package_root).unwrap();
        let entry = reader.entry_by_path(product_path).unwrap();
        let chunk_path = package_root.join(entry.chunks[0].chunk_path.as_str());
        let mut corrupted = fs::read(&chunk_path).unwrap();
        corrupted[0] ^= 0xff;
        fs::write(&chunk_path, corrupted).unwrap();

        let error = reader.read_product_path(product_path).unwrap_err();

        assert!(matches!(
            error,
            AzPackReadError::ChunkEncodedHashMismatch { .. }
        ));
    }

    fn sample_index() -> AzPackIndex {
        let payload = b"compiled product bytes";
        let hash = *blake3::hash(payload).as_bytes();
        let encoded = b"encoded product bytes";
        let encoded_hash = *blake3::hash(encoded).as_bytes();
        AzPackIndex::new(
            AzPackIndexProfile::new("pc-release", "pc", "oodle"),
            u32::try_from(AZPACK_DEFAULT_CHUNK_SIZE).unwrap(),
            vec![AzPackIndexEntry::new(
                "products/prefab.azbin",
                "prefabs/source.prefab.ron",
                "BuildPrefab",
                Uuid::from_bytes([1; 16]),
                Uuid::from_bytes([3; 16]),
                7,
                payload.len() as u64,
                hash,
                vec![AzPackIndexChunk::new(
                    0,
                    u32::try_from(payload.len()).unwrap(),
                    u32::try_from(encoded.len()).unwrap(),
                    AzPackChunkCompression::Oodle,
                    hash,
                    encoded_hash,
                    format!(
                        "chunks/{}/{}.azchunk",
                        &format_package_content_hash_hex(&encoded_hash)[..2],
                        format_package_content_hash_hex(&encoded_hash)
                    ),
                )],
            )],
        )
        .unwrap()
    }

    fn write_test_package(
        project_root: &Path,
        product_path: &str,
        bytes: &[u8],
        compression: &str,
    ) -> PathBuf {
        write_cache_product(project_root, "pc", product_path, bytes);
        let manifest = PackageManifest::new(
            package_profile(compression),
            vec![package_entry(product_path, bytes)],
        )
        .unwrap();
        let receipt = write_package_payload(PackagePayloadWriteRequest::new(
            &manifest,
            &platform_product_cache_dir(project_root, "pc"),
            &project_root.join("target/package"),
        ))
        .unwrap();
        receipt.path
    }

    fn package_profile(compression: &str) -> PackageManifestProfile {
        PackageManifestProfile {
            name: "pc-release".to_string(),
            asset_platform: "pc".to_string(),
            cargo_profile: "release".to_string(),
            container: "azpack".to_string(),
            compression: compression.to_string(),
            oodle_compressor: (compression == "oodle").then(|| "kraken".to_string()),
            oodle_effort: (compression == "oodle").then(|| "normal".to_string()),
        }
    }

    fn package_entry(product_path: &str, bytes: &[u8]) -> PackageManifestEntry {
        PackageManifestEntry::new(
            product_path,
            Uuid::from_bytes([3; 16]),
            7,
            "az.test.raw",
            1,
            *blake3::hash(bytes).as_bytes(),
            bytes.len() as u64,
            Uuid::from_bytes([1; 16]),
            "prefabs/source.prefab.ron",
            "BuildPrefab",
        )
    }

    fn write_cache_product(project_root: &Path, platform: &str, product_path: &str, bytes: &[u8]) {
        let path = platform_product_cache_dir(project_root, platform).join(product_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}
