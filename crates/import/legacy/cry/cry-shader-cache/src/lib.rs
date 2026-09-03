//! Parser for Cry/Lumberyard shader cache assets.
//!
//! Cry renderer shader caches use `.cfxb`, `.cfib`, and `.fxcb` containers.

pub mod builder;
mod lookup_data;
pub mod source_transform;

use std::str;

use thiserror::Error;

pub use lookup_data::*;
pub use source_transform::{
    ShaderCacheSourceTransform, ShaderCacheSourceTransformError, is_legacy_shader_cache_source,
};

pub const SHADER_BIN_HEADER_SIZE: usize = 28;
pub const SHADER_PARAM_BLOCK_HEADER_SIZE: usize = 32;
pub const RESOURCE_HEADER_SIZE: usize = 20;
pub const RESOURCE_ENTRY_SIZE: usize = 12;
pub const RESOURCE_REF_SIZE: usize = 8;

const SHADER_BIN_MAGIC: [u8; 4] = *b"FXB0";
const RESOURCE_MAGIC: [u8; 4] = *b"CPCK";
const MAX_SHADER_PARAM_COUNT: u32 = u16::MAX as u32;
const RESOURCE_SIZE_MASK: u32 = 0x00ff_ffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaderBin<'a> {
    bytes: &'a [u8],
    header: ShaderBinHeader,
}

impl<'a> ShaderBin<'a> {
    /// Parses an `FXB0` shader token binary (`.cfib` / `.cfxb`).
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::TooShort`] when `bytes` is shorter than the
    /// 28-byte header, [`ParseError::InvalidMagic`] when the magic is not
    /// `FXB0`, [`ParseError::UnexpectedStringTableOffset`] when the header's
    /// string-table offset disagrees with its token count,
    /// [`ParseError::InvalidOffsets`] or [`ParseError::OffsetOverflow`] when
    /// the header offsets are not ordered inside the file, and the
    /// token-table / parameter-block variants
    /// ([`ParseError::UnterminatedTokenString`], [`ParseError::InvalidUtf8`],
    /// [`ParseError::TrailingTokenBytes`], [`ParseError::ParamCountTooLarge`],
    /// [`ParseError::TruncatedParamWords`], [`ParseError::TrailingParamBytes`])
    /// when the body does not validate.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        if bytes.len() < SHADER_BIN_HEADER_SIZE {
            return Err(ParseError::TooShort {
                needed: SHADER_BIN_HEADER_SIZE,
                actual: bytes.len(),
            });
        }

        let magic = read_array::<4>(bytes, 0)?;
        if magic != SHADER_BIN_MAGIC {
            return Err(ParseError::InvalidMagic {
                expected: SHADER_BIN_MAGIC,
                actual: magic,
            });
        }

        let token_count = read_u32(bytes, 20)?;
        let header_size =
            u32::try_from(SHADER_BIN_HEADER_SIZE).map_err(|_| ParseError::OffsetOverflow)?;
        let expected_string_table_offset = token_count
            .checked_mul(4)
            .and_then(|bytes| bytes.checked_add(header_size))
            .ok_or(ParseError::OffsetOverflow)?;

        let header = ShaderBinHeader {
            crc32: read_u32(bytes, 4)?,
            version: ShaderCacheVersion {
                minor: read_u16(bytes, 8)?,
                major: read_u16(bytes, 10)?,
            },
            string_table_offset: read_u32(bytes, 12)?,
            local_params_offset: read_u32(bytes, 16)?,
            token_count,
            source_crc32: read_u32(bytes, 24)?,
        };

        if header.string_table_offset != expected_string_table_offset {
            return Err(ParseError::UnexpectedStringTableOffset {
                expected: expected_string_table_offset,
                actual: header.string_table_offset,
            });
        }

        let string_table_offset =
            usize::try_from(header.string_table_offset).map_err(|_| ParseError::OffsetOverflow)?;
        let local_params_offset =
            usize::try_from(header.local_params_offset).map_err(|_| ParseError::OffsetOverflow)?;
        if string_table_offset > local_params_offset || local_params_offset > bytes.len() {
            return Err(ParseError::InvalidOffsets {
                string_table: header.string_table_offset,
                local_params: header.local_params_offset,
                len: bytes.len(),
            });
        }

        let bin = Self { bytes, header };
        bin.validate_token_table()?;
        bin.validate_param_blocks()?;

        Ok(bin)
    }

    #[inline]
    #[must_use]
    pub const fn header(&self) -> ShaderBinHeader {
        self.header
    }

    #[inline]
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub fn token_words(&self) -> WordSlice<'a> {
        let end = self.header.string_table_offset as usize;
        WordSlice::new(&self.bytes[SHADER_BIN_HEADER_SIZE..end])
    }

    #[inline]
    #[must_use]
    pub fn token_table(&self) -> TokenStrings<'a> {
        let start = self.header.string_table_offset as usize;
        let end = self.header.local_params_offset as usize;
        TokenStrings {
            bytes: &self.bytes[start..end],
            position: 0,
        }
    }

    #[inline]
    #[must_use]
    pub fn param_blocks(&self) -> ParamBlocks<'a> {
        let start = self.header.local_params_offset as usize;
        ParamBlocks {
            bytes: &self.bytes[start..],
            position: 0,
        }
    }

    fn validate_token_table(&self) -> Result<(), ParseError> {
        let mut strings = self.token_table();
        for value in strings.by_ref() {
            value?;
        }
        if strings.position != strings.bytes.len() {
            return Err(ParseError::TrailingTokenBytes {
                trailing: strings.bytes.len() - strings.position,
            });
        }
        Ok(())
    }

    fn validate_param_blocks(&self) -> Result<(), ParseError> {
        let mut blocks = self.param_blocks();
        for block in blocks.by_ref() {
            block?;
        }
        if blocks.position != blocks.bytes.len() {
            return Err(ParseError::TrailingParamBytes {
                trailing: blocks.bytes.len() - blocks.position,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaderBinHeader {
    pub crc32: u32,
    pub version: ShaderCacheVersion,
    pub string_table_offset: u32,
    pub local_params_offset: u32,
    pub token_count: u32,
    pub source_crc32: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaderCacheVersion {
    pub major: u16,
    pub minor: u16,
}

impl ShaderCacheVersion {
    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenString<'a> {
    pub token: u32,
    pub name: &'a str,
}

/// Iterating a `TokenStrings` advances it, so it is deliberately not `Copy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenStrings<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Iterator for TokenStrings<'a> {
    type Item = Result<TokenString<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position == self.bytes.len() {
            return None;
        }

        let start = self.position;
        if start + 4 > self.bytes.len() {
            self.position = self.bytes.len();
            return Some(Err(ParseError::TruncatedTokenString {
                offset: start,
                len: self.bytes.len() - start,
            }));
        }

        let token =
            u32::from_le_bytes(self.bytes[start..start + 4].try_into().expect("slice size"));
        let string_start = start + 4;
        let Some(nul) = self.bytes[string_start..]
            .iter()
            .position(|byte| *byte == 0)
        else {
            self.position = self.bytes.len();
            return Some(Err(ParseError::UnterminatedTokenString {
                offset: string_start,
            }));
        };
        let string_end = string_start + nul;
        self.position = string_end + 1;

        let name = match str::from_utf8(&self.bytes[string_start..string_end]) {
            Ok(name) => name,
            Err(source) => {
                return Some(Err(ParseError::InvalidUtf8 {
                    offset: string_start,
                    source,
                }));
            }
        };

        Some(Ok(TokenString { token, name }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamBlock<'a> {
    pub header: ParamBlockHeader,
    params: WordSlice<'a>,
    samplers: WordSlice<'a>,
    textures: WordSlice<'a>,
    functions: WordSlice<'a>,
}

impl<'a> ParamBlock<'a> {
    #[inline]
    #[must_use]
    pub const fn params(self) -> WordSlice<'a> {
        self.params
    }

    #[inline]
    #[must_use]
    pub const fn samplers(self) -> WordSlice<'a> {
        self.samplers
    }

    #[inline]
    #[must_use]
    pub const fn textures(self) -> WordSlice<'a> {
        self.textures
    }

    #[inline]
    #[must_use]
    pub const fn functions(self) -> WordSlice<'a> {
        self.functions
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamBlockHeader {
    pub mask: u64,
    pub name: u32,
    pub param_count: u32,
    pub sampler_count: u32,
    pub texture_count: u32,
    pub function_count: u32,
    pub reserved: u32,
}

/// Iterating a `ParamBlocks` advances it, so it is deliberately not `Copy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamBlocks<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Iterator for ParamBlocks<'a> {
    type Item = Result<ParamBlock<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position == self.bytes.len() {
            return None;
        }

        let start = self.position;
        let header_end = start + SHADER_PARAM_BLOCK_HEADER_SIZE;
        if header_end > self.bytes.len() {
            self.position = self.bytes.len();
            return Some(Err(ParseError::TruncatedParamBlockHeader {
                offset: start,
                len: self.bytes.len() - start,
            }));
        }

        let header = ParamBlockHeader {
            mask: u64::from_le_bytes(self.bytes[start..start + 8].try_into().expect("slice size")),
            name: u32::from_le_bytes(
                self.bytes[start + 8..start + 12]
                    .try_into()
                    .expect("slice size"),
            ),
            param_count: u32::from_le_bytes(
                self.bytes[start + 12..start + 16]
                    .try_into()
                    .expect("slice size"),
            ),
            sampler_count: u32::from_le_bytes(
                self.bytes[start + 16..start + 20]
                    .try_into()
                    .expect("slice size"),
            ),
            texture_count: u32::from_le_bytes(
                self.bytes[start + 20..start + 24]
                    .try_into()
                    .expect("slice size"),
            ),
            function_count: u32::from_le_bytes(
                self.bytes[start + 24..start + 28]
                    .try_into()
                    .expect("slice size"),
            ),
            reserved: u32::from_le_bytes(
                self.bytes[start + 28..start + 32]
                    .try_into()
                    .expect("slice size"),
            ),
        };

        if let Err(err) = validate_param_count(header.param_count)
            .and_then(|()| validate_param_count(header.sampler_count))
            .and_then(|()| validate_param_count(header.texture_count))
            .and_then(|()| validate_param_count(header.function_count))
        {
            self.position = self.bytes.len();
            return Some(Err(err));
        }

        let mut position = header_end;
        let params = match read_word_slice(self.bytes, &mut position, header.param_count) {
            Ok(values) => values,
            Err(err) => {
                self.position = self.bytes.len();
                return Some(Err(err));
            }
        };
        let samplers = match read_word_slice(self.bytes, &mut position, header.sampler_count) {
            Ok(values) => values,
            Err(err) => {
                self.position = self.bytes.len();
                return Some(Err(err));
            }
        };
        let textures = match read_word_slice(self.bytes, &mut position, header.texture_count) {
            Ok(values) => values,
            Err(err) => {
                self.position = self.bytes.len();
                return Some(Err(err));
            }
        };
        let functions = match read_word_slice(self.bytes, &mut position, header.function_count) {
            Ok(values) => values,
            Err(err) => {
                self.position = self.bytes.len();
                return Some(Err(err));
            }
        };

        self.position = position;
        Some(Ok(ParamBlock {
            header,
            params,
            samplers,
            textures,
            functions,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceFile<'a> {
    bytes: &'a [u8],
    header: ResourceHeader,
}

impl<'a> ResourceFile<'a> {
    /// Parses a `CPCK` Cry resource cache (`.fxcb`).
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::TooShort`] when `bytes` is shorter than the
    /// 20-byte header, [`ParseError::InvalidMagic`] when the magic is not
    /// `CPCK`, [`ParseError::InvalidResourceVersion`] for a version outside
    /// [`ResourceVersion`], [`ParseError::InvalidResourceFileCount`] for a
    /// non-positive file count, [`ParseError::OffsetOverflow`] or
    /// [`ParseError::TruncatedResourceDirectory`] when the directory does not
    /// fit inside the file, and [`ParseError::InvalidEntryOffset`] or
    /// [`ParseError::TruncatedResourceEntry`] when an entry's payload does.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        if bytes.len() < RESOURCE_HEADER_SIZE {
            return Err(ParseError::TooShort {
                needed: RESOURCE_HEADER_SIZE,
                actual: bytes.len(),
            });
        }

        let magic = read_array::<4>(bytes, 0)?;
        if magic != RESOURCE_MAGIC {
            return Err(ParseError::InvalidMagic {
                expected: RESOURCE_MAGIC,
                actual: magic,
            });
        }

        let version = ResourceVersion::parse(read_i32(bytes, 4)?)?;
        let file_count = read_i32(bytes, 8)?;
        if file_count <= 0 {
            return Err(ParseError::InvalidResourceFileCount { file_count });
        }

        let header = ResourceHeader {
            version,
            file_count: u32::try_from(file_count)
                .map_err(|_| ParseError::InvalidResourceFileCount { file_count })?,
            directory_offset: read_u32(bytes, 12)?,
            ref_count: read_u32(bytes, 16)?,
        };

        let directory_offset =
            usize::try_from(header.directory_offset).map_err(|_| ParseError::OffsetOverflow)?;
        let directory_size = resource_directory_size(header.file_count, header.ref_count)?;
        let directory_end = directory_offset
            .checked_add(directory_size)
            .ok_or(ParseError::OffsetOverflow)?;
        if directory_end > bytes.len() {
            return Err(ParseError::TruncatedResourceDirectory {
                directory_offset: header.directory_offset,
                needed: directory_end,
                actual: bytes.len(),
            });
        }

        let file = Self { bytes, header };
        for entry in file.entries() {
            let entry = entry?;
            entry.payload()?;
        }
        for reference in file.references() {
            reference?;
        }

        Ok(file)
    }

    #[inline]
    #[must_use]
    pub const fn header(&self) -> ResourceHeader {
        self.header
    }

    #[inline]
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub const fn entries(&self) -> ResourceEntries<'a> {
        ResourceEntries {
            bytes: self.bytes,
            position: self.header.directory_offset as usize,
            remaining: self.header.file_count,
        }
    }

    #[inline]
    #[must_use]
    pub const fn references(&self) -> ResourceRefs<'a> {
        let start = self.header.directory_offset as usize
            + self.header.file_count as usize * RESOURCE_ENTRY_SIZE;
        ResourceRefs {
            bytes: self.bytes,
            position: start,
            remaining: self.header.ref_count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceHeader {
    pub version: ResourceVersion,
    pub file_count: u32,
    pub directory_offset: u32,
    pub ref_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceVersion {
    Lzss,
    Lzma,
    Debug,
}

impl ResourceVersion {
    pub const LZSS_VALUE: i32 = 10;
    pub const LZMA_VALUE: i32 = 11;
    pub const DEBUG_VALUE: i32 = 12;

    #[must_use]
    pub const fn from_native_value(value: i32) -> Option<Self> {
        match value {
            Self::LZSS_VALUE => Some(Self::Lzss),
            Self::LZMA_VALUE => Some(Self::Lzma),
            Self::DEBUG_VALUE => Some(Self::Debug),
            _ => None,
        }
    }

    fn parse(value: i32) -> Result<Self, ParseError> {
        Self::from_native_value(value).ok_or(ParseError::InvalidResourceVersion { version: value })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceEntry<'a> {
    bytes: &'a [u8],
    pub name_crc: u32,
    pub size: u32,
    pub flags: ResourceFlags,
    pub offset: i32,
}

impl<'a> ResourceEntry<'a> {
    /// Borrows this entry's payload bytes out of the owning resource file.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::InvalidEntryOffset`] for a negative entry offset,
    /// [`ParseError::OffsetOverflow`] when `offset + size` overflows `usize`,
    /// and [`ParseError::TruncatedResourceEntry`] when the payload runs past
    /// the end of the file.
    #[inline]
    pub fn payload(self) -> Result<&'a [u8], ParseError> {
        let offset = usize::try_from(self.offset).map_err(|_| ParseError::InvalidEntryOffset {
            name_crc: self.name_crc,
            offset: self.offset,
        })?;
        let size = usize::try_from(self.size).map_err(|_| ParseError::OffsetOverflow)?;
        let end = offset.checked_add(size).ok_or(ParseError::OffsetOverflow)?;
        if end > self.bytes.len() {
            return Err(ParseError::TruncatedResourceEntry {
                name_crc: self.name_crc,
                needed: end,
                actual: self.bytes.len(),
            });
        }
        Ok(&self.bytes[offset..end])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceFlags(u8);

impl ResourceFlags {
    pub const NOT_SAVED: Self = Self(0x01);
    pub const COMPRESS: Self = Self(0x04);
    pub const TEMP_DATA: Self = Self(0x08);
    pub const TOKENS: Self = Self(0x20);
    pub const COMPRESSED: Self = Self(0x80);

    #[inline]
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    #[inline]
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Iterating a `ResourceEntries` advances it, so it is deliberately not `Copy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceEntries<'a> {
    bytes: &'a [u8],
    position: usize,
    remaining: u32,
}

impl<'a> Iterator for ResourceEntries<'a> {
    type Item = Result<ResourceEntry<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;

        let start = self.position;
        let end = start + RESOURCE_ENTRY_SIZE;
        if end > self.bytes.len() {
            self.position = self.bytes.len();
            return Some(Err(ParseError::TruncatedResourceDirectory {
                // The offset is only reported in the error message, and a
                // directory past `u32::MAX` is already a corrupt file, so
                // saturate instead of adding a second failure mode.
                directory_offset: u32::try_from(start).unwrap_or(u32::MAX),
                needed: end,
                actual: self.bytes.len(),
            }));
        }

        self.position = end;
        let size_and_flags = u32::from_le_bytes(
            self.bytes[start + 4..start + 8]
                .try_into()
                .expect("slice size"),
        );

        Some(Ok(ResourceEntry {
            bytes: self.bytes,
            name_crc: u32::from_le_bytes(
                self.bytes[start..start + 4].try_into().expect("slice size"),
            ),
            size: size_and_flags & RESOURCE_SIZE_MASK,
            flags: ResourceFlags::from_bits((size_and_flags >> 24) as u8),
            offset: i32::from_le_bytes(self.bytes[start + 8..end].try_into().expect("slice size")),
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceRef {
    pub name_crc: u32,
    pub entry_index: u32,
}

/// Iterating a `ResourceRefs` advances it, so it is deliberately not `Copy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRefs<'a> {
    bytes: &'a [u8],
    position: usize,
    remaining: u32,
}

impl Iterator for ResourceRefs<'_> {
    type Item = Result<ResourceRef, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;

        let start = self.position;
        let end = start + RESOURCE_REF_SIZE;
        if end > self.bytes.len() {
            self.position = self.bytes.len();
            return Some(Err(ParseError::TruncatedResourceDirectory {
                // The offset is only reported in the error message, and a
                // directory past `u32::MAX` is already a corrupt file, so
                // saturate instead of adding a second failure mode.
                directory_offset: u32::try_from(start).unwrap_or(u32::MAX),
                needed: end,
                actual: self.bytes.len(),
            }));
        }

        self.position = end;
        Some(Ok(ResourceRef {
            name_crc: u32::from_le_bytes(
                self.bytes[start..start + 4].try_into().expect("slice size"),
            ),
            entry_index: u32::from_le_bytes(
                self.bytes[start + 4..end].try_into().expect("slice size"),
            ),
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordSlice<'a> {
    bytes: &'a [u8],
}

impl<'a> WordSlice<'a> {
    #[inline]
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    #[inline]
    #[must_use]
    pub const fn len(self) -> usize {
        self.bytes.len() / 4
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    #[inline]
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub fn get(self, index: usize) -> Option<u32> {
        let start = index.checked_mul(4)?;
        let end = start.checked_add(4)?;
        let bytes: [u8; 4] = self.bytes.get(start..end)?.try_into().ok()?;
        Some(u32::from_le_bytes(bytes))
    }

    #[inline]
    #[must_use]
    pub const fn iter(self) -> Words<'a> {
        Words {
            bytes: self.bytes,
            position: 0,
        }
    }
}

/// Iterating a `Words` advances it, so it is deliberately not `Copy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Words<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Iterator for Words<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        let end = self.position.checked_add(4)?;
        let bytes = self.bytes.get(self.position..end)?;
        self.position = end;
        Some(u32::from_le_bytes(bytes.try_into().expect("slice size")))
    }
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("shader cache file is too short: need at least {needed} bytes, got {actual}")]
    TooShort { needed: usize, actual: usize },

    #[error("invalid magic {actual:?}, expected {expected:?}")]
    InvalidMagic { expected: [u8; 4], actual: [u8; 4] },

    #[error("shader cache offset overflows usize")]
    OffsetOverflow,

    #[error("unexpected shader string table offset: expected {expected}, got {actual}")]
    UnexpectedStringTableOffset { expected: u32, actual: u32 },

    #[error(
        "invalid shader offsets: string table {string_table}, local params {local_params}, len {len}"
    )]
    InvalidOffsets {
        string_table: u32,
        local_params: u32,
        len: usize,
    },

    #[error("truncated shader token string at offset {offset}: {len} bytes remain")]
    TruncatedTokenString { offset: usize, len: usize },

    #[error("unterminated shader token string at offset {offset}")]
    UnterminatedTokenString { offset: usize },

    #[error("invalid UTF-8 shader token string at offset {offset}: {source}")]
    InvalidUtf8 {
        offset: usize,
        source: str::Utf8Error,
    },

    #[error("trailing bytes after shader token table: {trailing}")]
    TrailingTokenBytes { trailing: usize },

    #[error("truncated shader parameter block header at offset {offset}: {len} bytes remain")]
    TruncatedParamBlockHeader { offset: usize, len: usize },

    #[error("shader parameter count is too large: {count}")]
    ParamCountTooLarge { count: u32 },

    #[error("truncated shader parameter words: need {needed} bytes, got {actual}")]
    TruncatedParamWords { needed: usize, actual: usize },

    #[error("trailing bytes after shader parameter blocks: {trailing}")]
    TrailingParamBytes { trailing: usize },

    #[error("invalid Cry resource version: {version}")]
    InvalidResourceVersion { version: i32 },

    #[error("invalid Cry resource file count: {file_count}")]
    InvalidResourceFileCount { file_count: i32 },

    #[error(
        "truncated Cry resource directory at {directory_offset}: need {needed} bytes, got {actual}"
    )]
    TruncatedResourceDirectory {
        directory_offset: u32,
        needed: usize,
        actual: usize,
    },

    #[error("invalid Cry resource entry offset for {name_crc:#010x}: {offset}")]
    InvalidEntryOffset { name_crc: u32, offset: i32 },

    #[error("truncated Cry resource entry {name_crc:#010x}: need {needed} bytes, got {actual}")]
    TruncatedResourceEntry {
        name_crc: u32,
        needed: usize,
        actual: usize,
    },

    #[error("invalid shader lookup cache version {version:?}")]
    InvalidLookupCacheVersion { version: [u8; 16] },

    #[error("shader lookup data count overflows usize")]
    LookupDataCountOverflow,

    #[error("trailing bytes after shader lookup data: {trailing}")]
    TrailingLookupData { trailing: usize },
}

const fn validate_param_count(count: u32) -> Result<(), ParseError> {
    if count > MAX_SHADER_PARAM_COUNT {
        return Err(ParseError::ParamCountTooLarge { count });
    }
    Ok(())
}

fn read_word_slice<'a>(
    bytes: &'a [u8],
    position: &mut usize,
    count: u32,
) -> Result<WordSlice<'a>, ParseError> {
    let byte_count = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(4))
        .ok_or(ParseError::OffsetOverflow)?;
    let end = position
        .checked_add(byte_count)
        .ok_or(ParseError::OffsetOverflow)?;
    if end > bytes.len() {
        return Err(ParseError::TruncatedParamWords {
            needed: end,
            actual: bytes.len(),
        });
    }
    let slice = WordSlice::new(&bytes[*position..end]);
    *position = end;
    Ok(slice)
}

fn resource_directory_size(file_count: u32, ref_count: u32) -> Result<usize, ParseError> {
    let entries = usize::try_from(file_count)
        .ok()
        .and_then(|count| count.checked_mul(RESOURCE_ENTRY_SIZE))
        .ok_or(ParseError::OffsetOverflow)?;
    let refs = usize::try_from(ref_count)
        .ok()
        .and_then(|count| count.checked_mul(RESOURCE_REF_SIZE))
        .ok_or(ParseError::OffsetOverflow)?;
    entries.checked_add(refs).ok_or(ParseError::OffsetOverflow)
}

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], ParseError> {
    let end = offset + N;
    if end > bytes.len() {
        return Err(ParseError::TooShort {
            needed: end,
            actual: bytes.len(),
        });
    }
    Ok(bytes[offset..end].try_into().expect("slice size"))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ParseError> {
    Ok(u16::from_le_bytes(read_array(bytes, offset)?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ParseError> {
    Ok(u32::from_le_bytes(read_array(bytes, offset)?))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, ParseError> {
    Ok(i32::from_le_bytes(read_array(bytes, offset)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shader_bin_tokens_and_params() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FXB0");
        bytes.extend_from_slice(&0x9ab9_0fdfu32.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(&11u16.to_le_bytes());
        bytes.extend_from_slice(&36u32.to_le_bytes());
        bytes.extend_from_slice(&52u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&0xb17f_3704u32.to_le_bytes());
        bytes.extend_from_slice(&0x1234u32.to_le_bytes());
        bytes.extend_from_slice(&0x5678u32.to_le_bytes());
        bytes.extend_from_slice(&0xaabb_ccddu32.to_le_bytes());
        bytes.extend_from_slice(b"Foo\0");
        bytes.extend_from_slice(&0xeeff_0011u32.to_le_bytes());
        bytes.extend_from_slice(b"Bar\0");
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&0xed04_0cd1u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0x19fu32.to_le_bytes());
        bytes.extend_from_slice(&0xcdu32.to_le_bytes());
        bytes.extend_from_slice(&0xe2u32.to_le_bytes());

        let file = ShaderBin::parse(&bytes).unwrap();
        assert_eq!(file.header().version, ShaderCacheVersion::new(11, 8));
        assert_eq!(
            file.token_words().iter().collect::<Vec<_>>(),
            [0x1234, 0x5678]
        );
        let tokens = file.token_table().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(
            tokens,
            [
                TokenString {
                    token: 0xaabb_ccdd,
                    name: "Foo"
                },
                TokenString {
                    token: 0xeeff_0011,
                    name: "Bar"
                }
            ]
        );
        let blocks = file.param_blocks().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].header.name, 0xed04_0cd1);
        assert_eq!(blocks[0].params().iter().collect::<Vec<_>>(), [0x19f]);
        assert_eq!(
            blocks[0].functions().iter().collect::<Vec<_>>(),
            [0xcd, 0xe2]
        );
    }

    #[test]
    fn parses_include_shader_without_params() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FXB0");
        bytes.extend_from_slice(&0x9d0c_f3f5u32.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(&11u16.to_le_bytes());
        bytes.extend_from_slice(&28u32.to_le_bytes());
        bytes.extend_from_slice(&28u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0x8ee0_2d75u32.to_le_bytes());

        let file = ShaderBin::parse(&bytes).unwrap();
        assert_eq!(file.token_words().len(), 0);
        assert_eq!(file.token_table().count(), 0);
        assert_eq!(file.param_blocks().count(), 0);
    }

    #[test]
    fn parses_resource_file_directory() {
        let payload = [0xaa, 0xbb, 0xcc, 0xdd];
        let directory_offset = RESOURCE_HEADER_SIZE + payload.len();

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"CPCK");
        bytes.extend_from_slice(&ResourceVersion::LZSS_VALUE.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(directory_offset).unwrap().to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(&0x80cc_dc7eu32.to_le_bytes());
        bytes.extend_from_slice(
            &(u32::try_from(payload.len()).unwrap() | 0x2800_0000).to_le_bytes(),
        );
        bytes.extend_from_slice(&i32::try_from(RESOURCE_HEADER_SIZE).unwrap().to_le_bytes());

        let file = ResourceFile::parse(&bytes).unwrap();
        assert_eq!(file.header().version, ResourceVersion::Lzss);
        let entries = file.entries().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name_crc, 0x80cc_dc7e);
        assert_eq!(entries[0].size, 4);
        assert_eq!(entries[0].flags.bits(), 0x28);
        assert_eq!(entries[0].payload().unwrap(), payload);
    }

    #[test]
    fn rejects_bad_resource_entry_bounds() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"CPCK");
        bytes.extend_from_slice(&ResourceVersion::LZSS_VALUE.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(RESOURCE_HEADER_SIZE).unwrap().to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0x80cc_dc7eu32.to_le_bytes());
        bytes.extend_from_slice(&100u32.to_le_bytes());
        bytes.extend_from_slice(&20i32.to_le_bytes());

        let err = ResourceFile::parse(&bytes).unwrap_err();
        assert!(matches!(err, ParseError::TruncatedResourceEntry { .. }));
    }
}
