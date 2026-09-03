//! Shared `AzSectionFile` container used by compiled table formats.

use bytes::Buf;
#[cfg(any(feature = "authoring", test))]
use bytes::BufMut;

use crate::GameDataError;
use crate::format::{GAMEDATA_TABLE_MAGIC, GAMEDATA_TABLE_VERSION};

pub const SECTION_FILE_PREFIX_LEN: usize = 20;
pub const SECTION_DESCRIPTOR_LEN: usize = 24;
pub const SECTION_TRAILER_LEN: usize = 8;

#[cfg(any(feature = "authoring", test))]
pub const FLAG_LITTLE_ENDIAN: u32 = 1 << 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionDescriptor {
    pub id: u32,
    pub offset: u64,
    pub length: u64,
    pub schema_hash: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSectionFile {
    pub version: u32,
    pub flags: u32,
    pub sections: Vec<SectionDescriptor>,
}

impl ParsedSectionFile {
    pub fn parse(bytes: &[u8]) -> Result<Self, GameDataError> {
        if bytes.len() < SECTION_FILE_PREFIX_LEN + SECTION_TRAILER_LEN {
            return Err(GameDataError::Decode(format!(
                "section file too short: {} bytes",
                bytes.len()
            )));
        }
        if bytes[..8] != GAMEDATA_TABLE_MAGIC {
            return Err(GameDataError::Decode(
                "section file missing GameData table magic".into(),
            ));
        }

        let mut data = &bytes[8..];
        let version = read_u32(&mut data, "version")?;
        if version != GAMEDATA_TABLE_VERSION {
            return Err(GameDataError::Decode(format!(
                "unsupported GameData table version {version} (expected {GAMEDATA_TABLE_VERSION})"
            )));
        }
        let flags = read_u32(&mut data, "flags")?;
        let section_count = read_u32(&mut data, "section_count")? as usize;

        let index_len = SECTION_FILE_PREFIX_LEN
            .checked_add(
                section_count
                    .checked_mul(SECTION_DESCRIPTOR_LEN)
                    .ok_or_else(|| {
                        GameDataError::Decode("section_count overflows section index".into())
                    })?,
            )
            .and_then(|len| len.checked_add(SECTION_TRAILER_LEN))
            .ok_or_else(|| GameDataError::Decode("section index length overflow".into()))?;
        if bytes.len() < index_len {
            return Err(GameDataError::Decode(format!(
                "section index truncated: need {index_len} bytes, have {}",
                bytes.len()
            )));
        }

        let mut sections = Vec::with_capacity(section_count);
        for index in 0..section_count {
            sections.push(SectionDescriptor {
                id: read_u32(&mut data, &format!("section[{index}].id"))?,
                offset: read_u64(&mut data, &format!("section[{index}].offset"))?,
                length: read_u64(&mut data, &format!("section[{index}].length"))?,
                schema_hash: read_u32(&mut data, &format!("section[{index}].schema_hash"))?,
            });
        }

        Ok(Self {
            version,
            flags,
            sections,
        })
    }

    pub fn section_payload<'a>(
        &'a self,
        bytes: &'a [u8],
        section_id: u32,
    ) -> Result<&'a [u8], GameDataError> {
        let descriptor = self
            .sections
            .iter()
            .find(|entry| entry.id == section_id)
            .ok_or_else(|| GameDataError::Decode(format!("missing section id {section_id}")))?;
        let start = usize::try_from(descriptor.offset).map_err(|_| {
            GameDataError::Decode(format!(
                "section {section_id} offset {} exceeds address space",
                descriptor.offset
            ))
        })?;
        let end = start
            .checked_add(usize::try_from(descriptor.length).map_err(|_| {
                GameDataError::Decode(format!(
                    "section {section_id} length {} exceeds address space",
                    descriptor.length
                ))
            })?)
            .ok_or_else(|| {
                GameDataError::Decode(format!("section {section_id} length overflow"))
            })?;
        bytes.get(start..end).ok_or_else(|| {
            GameDataError::Decode(format!(
                "section {section_id} payload out of bounds: {start}..{end} in {} byte file",
                bytes.len()
            ))
        })
    }

    #[must_use]
    pub fn section_offset(&self, section_id: u32) -> Option<u64> {
        self.sections
            .iter()
            .find(|entry| entry.id == section_id)
            .map(|entry| entry.offset)
    }
}

#[cfg(any(feature = "authoring", test))]
pub fn build_section_file(
    flags: u32,
    sections: &[(u32, Vec<u8>, u32)],
) -> Result<Vec<u8>, GameDataError> {
    let index_len =
        SECTION_FILE_PREFIX_LEN + sections.len() * SECTION_DESCRIPTOR_LEN + SECTION_TRAILER_LEN;
    let payload_len: usize = sections.iter().map(|(_, payload, _)| payload.len()).sum();
    let mut bytes = Vec::with_capacity(index_len + payload_len);
    bytes.put_slice(&GAMEDATA_TABLE_MAGIC);
    bytes.put_u32_le(GAMEDATA_TABLE_VERSION);
    bytes.put_u32_le(flags);
    bytes.put_u32_le(
        u32::try_from(sections.len())
            .map_err(|_| GameDataError::Decode("section_count exceeds u32".into()))?,
    );

    let mut payload_offset = index_len as u64;
    for (section_id, payload, schema_hash) in sections {
        bytes.put_u32_le(*section_id);
        bytes.put_u64_le(payload_offset);
        bytes.put_u64_le(u64::try_from(payload.len()).map_err(|_| {
            GameDataError::Decode(format!("section {section_id} payload length exceeds u64"))
        })?);
        bytes.put_u32_le(*schema_hash);
        payload_offset = payload_offset
            .checked_add(payload.len() as u64)
            .ok_or_else(|| {
                GameDataError::Decode(format!("section {section_id} payload offset overflow"))
            })?;
    }
    bytes.put_u64_le(0);

    for (_, payload, _) in sections {
        bytes.put_slice(payload);
    }
    Ok(bytes)
}

fn read_u32(data: &mut &[u8], field: &str) -> Result<u32, GameDataError> {
    if data.remaining() < 4 {
        return Err(truncated(field, data.remaining()));
    }
    Ok(data.get_u32_le())
}

fn read_u64(data: &mut &[u8], field: &str) -> Result<u64, GameDataError> {
    if data.remaining() < 8 {
        return Err(truncated(field, data.remaining()));
    }
    Ok(data.get_u64_le())
}

fn truncated(field: &str, remaining: usize) -> GameDataError {
    GameDataError::Decode(format!(
        "section file truncated while reading {field} ({remaining} byte(s) left)"
    ))
}
