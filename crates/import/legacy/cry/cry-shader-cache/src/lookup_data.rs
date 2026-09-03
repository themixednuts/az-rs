//! Shader resource lookup data.
//!
//! Follows Lumberyard's `dev/Code/CryEngine/RenderDll/Common/ResFileLookupDataMan.cpp`.

use std::str;

use crate::{ParseError, ResourceVersion};

pub const LOOKUP_DATA_MAGIC: &[u8; 4] = b"CPCK";
pub const LOOKUP_DATA_HEADER_SIZE: usize = 28;
pub const LOOKUP_DATA_CACHE_VERSION_SIZE: usize = 16;
pub const LOOKUP_DATA_ENTRY_SIZE: usize = 24;
pub const LOOKUP_DATA_CFX_ENTRY_SIZE: usize = 8;

/// `lookupdata.bin` loaded by `CResFileLookupDataMan`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaderLookupData<'a> {
    bytes: &'a [u8],
    header: ShaderLookupHeader<'a>,
}

impl<'a> ShaderLookupData<'a> {
    /// Parse a shader lookup data payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the header, cache version, counts, or trailing
    /// bytes are invalid.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        if bytes.len() < LOOKUP_DATA_HEADER_SIZE {
            return Err(ParseError::TooShort {
                needed: LOOKUP_DATA_HEADER_SIZE,
                actual: bytes.len(),
            });
        }
        let magic = read_array::<4>(bytes, 0)?;
        if magic != *LOOKUP_DATA_MAGIC {
            return Err(ParseError::InvalidMagic {
                expected: *LOOKUP_DATA_MAGIC,
                actual: magic,
            });
        }
        let version_raw = read_i32_at(bytes, 4)?;
        let resource_version = ResourceVersion::from_native_value(version_raw).ok_or(
            ParseError::InvalidResourceVersion {
                version: version_raw,
            },
        )?;
        let cache_version_bytes =
            bytes
                .get(8..8 + LOOKUP_DATA_CACHE_VERSION_SIZE)
                .ok_or(ParseError::TooShort {
                    needed: 8 + LOOKUP_DATA_CACHE_VERSION_SIZE,
                    actual: bytes.len(),
                })?;
        let cache_version = cache_version(cache_version_bytes)?;
        let lookup_count = read_u32_at(bytes, 24)?;
        let entries_start = LOOKUP_DATA_HEADER_SIZE;
        let cfx_count_offset =
            checked_section_end(entries_start, lookup_count, LOOKUP_DATA_ENTRY_SIZE)?;
        if cfx_count_offset + 4 > bytes.len() {
            return Err(ParseError::TooShort {
                needed: cfx_count_offset + 4,
                actual: bytes.len(),
            });
        }
        let cfx_lookup_count = read_u32_at(bytes, cfx_count_offset)?;
        let cfx_entries_start = cfx_count_offset + 4;
        let expected_len = checked_section_end(
            cfx_entries_start,
            cfx_lookup_count,
            LOOKUP_DATA_CFX_ENTRY_SIZE,
        )?;
        if expected_len > bytes.len() {
            return Err(ParseError::TooShort {
                needed: expected_len,
                actual: bytes.len(),
            });
        }
        if expected_len < bytes.len() {
            return Err(ParseError::TrailingLookupData {
                trailing: bytes.len() - expected_len,
            });
        }

        Ok(Self {
            bytes,
            header: ShaderLookupHeader {
                resource_version,
                cache_version,
                lookup_count,
                cfx_lookup_count,
            },
        })
    }

    #[inline]
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub const fn header(self) -> ShaderLookupHeader<'a> {
        self.header
    }

    #[inline]
    #[must_use]
    pub const fn entries(self) -> ShaderLookupEntries<'a> {
        ShaderLookupEntries {
            bytes: self.bytes,
            position: LOOKUP_DATA_HEADER_SIZE,
            remaining: self.header.lookup_count,
        }
    }

    #[inline]
    #[must_use]
    pub const fn cfx_entries(self) -> ShaderCfxLookupEntries<'a> {
        let start = LOOKUP_DATA_HEADER_SIZE
            + self.header.lookup_count as usize * LOOKUP_DATA_ENTRY_SIZE
            + 4;
        ShaderCfxLookupEntries {
            bytes: self.bytes,
            position: start,
            remaining: self.header.cfx_lookup_count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaderLookupHeader<'a> {
    pub resource_version: ResourceVersion,
    pub cache_version: &'a str,
    pub lookup_count: u32,
    pub cfx_lookup_count: u32,
}

/// `SResFileLookupDataDisk`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaderLookupEntry {
    pub name_crc: u32,
    pub unique_file_count: i32,
    pub referenced_file_count: i32,
    pub directory_offset: u32,
    pub data_crc: u32,
    pub cache_major_version: u16,
    pub cache_minor_version: u16,
}

/// `SCFXLookupData`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaderCfxLookupEntry {
    pub name_crc: u32,
    pub data_crc: u32,
}

/// Iterating a `ShaderLookupEntries` advances it, so it is deliberately not `Copy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderLookupEntries<'a> {
    bytes: &'a [u8],
    position: usize,
    remaining: u32,
}

impl Iterator for ShaderLookupEntries<'_> {
    type Item = ShaderLookupEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let start = self.position;
        self.position += LOOKUP_DATA_ENTRY_SIZE;
        self.remaining -= 1;
        Some(ShaderLookupEntry {
            name_crc: read_u32_at(self.bytes, start).expect("validated lookup entry"),
            unique_file_count: read_i32_at(self.bytes, start + 4).expect("validated lookup entry"),
            referenced_file_count: read_i32_at(self.bytes, start + 8)
                .expect("validated lookup entry"),
            directory_offset: read_u32_at(self.bytes, start + 12).expect("validated lookup entry"),
            data_crc: read_u32_at(self.bytes, start + 16).expect("validated lookup entry"),
            cache_major_version: read_u16_at(self.bytes, start + 20)
                .expect("validated lookup entry"),
            cache_minor_version: read_u16_at(self.bytes, start + 22)
                .expect("validated lookup entry"),
        })
    }
}

/// Iterating a `ShaderCfxLookupEntries` advances it, so it is deliberately not `Copy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderCfxLookupEntries<'a> {
    bytes: &'a [u8],
    position: usize,
    remaining: u32,
}

impl Iterator for ShaderCfxLookupEntries<'_> {
    type Item = ShaderCfxLookupEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let start = self.position;
        self.position += LOOKUP_DATA_CFX_ENTRY_SIZE;
        self.remaining -= 1;
        Some(ShaderCfxLookupEntry {
            name_crc: read_u32_at(self.bytes, start).expect("validated CFX lookup entry"),
            data_crc: read_u32_at(self.bytes, start + 4).expect("validated CFX lookup entry"),
        })
    }
}

fn cache_version(bytes: &[u8]) -> Result<&str, ParseError> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let value = str::from_utf8(&bytes[..end])
        .map_err(|source| ParseError::InvalidUtf8 { offset: 8, source })?;
    if !value.starts_with("Ver: ") {
        let version = bytes.try_into().expect("cache version size");
        return Err(ParseError::InvalidLookupCacheVersion { version });
    }
    Ok(value)
}

fn checked_section_end(start: usize, count: u32, stride: usize) -> Result<usize, ParseError> {
    let count = usize::try_from(count).map_err(|_| ParseError::LookupDataCountOverflow)?;
    count
        .checked_mul(stride)
        .and_then(|size| start.checked_add(size))
        .ok_or(ParseError::LookupDataCountOverflow)
}

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], ParseError> {
    bytes
        .get(offset..offset + N)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(ParseError::TooShort {
            needed: offset + N,
            actual: bytes.len(),
        })
}

fn read_u16_at(bytes: &[u8], offset: usize) -> Result<u16, ParseError> {
    Ok(u16::from_le_bytes(read_array(bytes, offset)?))
}

fn read_u32_at(bytes: &[u8], offset: usize) -> Result<u32, ParseError> {
    Ok(u32::from_le_bytes(read_array(bytes, offset)?))
}

fn read_i32_at(bytes: &[u8], offset: usize) -> Result<i32, ParseError> {
    Ok(i32::from_le_bytes(read_array(bytes, offset)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shader_lookup_data() {
        let mut bytes = Vec::new();
        bytes.extend(*LOOKUP_DATA_MAGIC);
        bytes.extend(ResourceVersion::LZSS_VALUE.to_le_bytes());
        bytes.extend(cache_version_bytes("Ver: 11.0"));
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(0x1122_3344u32.to_le_bytes());
        bytes.extend(2i32.to_le_bytes());
        bytes.extend(3i32.to_le_bytes());
        bytes.extend(4u32.to_le_bytes());
        bytes.extend(5u32.to_le_bytes());
        bytes.extend(11u16.to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(0xaabb_ccddu32.to_le_bytes());
        bytes.extend(0x5566_7788u32.to_le_bytes());

        let lookup = ShaderLookupData::parse(&bytes).unwrap();
        let entries = lookup.entries().collect::<Vec<_>>();
        let cfx_entries = lookup.cfx_entries().collect::<Vec<_>>();

        assert_eq!(lookup.header().cache_version, "Ver: 11.0");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].unique_file_count, 2);
        assert_eq!(cfx_entries.len(), 1);
        assert_eq!(cfx_entries[0].data_crc, 0x5566_7788);
    }

    fn cache_version_bytes(value: &str) -> [u8; LOOKUP_DATA_CACHE_VERSION_SIZE] {
        let mut bytes = [0; LOOKUP_DATA_CACHE_VERSION_SIZE];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        bytes
    }
}
