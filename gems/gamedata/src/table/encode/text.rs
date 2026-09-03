use bytes::Buf;
#[cfg(any(feature = "authoring", test))]
use bytes::BufMut;

use crate::GameDataError;
use crate::format::TableSectionId;
use crate::table::body::{ListRef, TextRef};
use crate::table::encode::pool::STRING_POOL_HEADER_LEN;
use crate::table::encode::scalar::{read_u32, truncated};
use crate::table::section::ParsedSectionFile;

pub(super) fn read_text_ref(
    bytes: &[u8],
    data: &mut &[u8],
    field: &str,
) -> Result<TextRef, GameDataError> {
    let len = read_u32(data, &format!("{field}.len"))? as usize;
    if data.remaining() < len {
        return Err(truncated(field, data.remaining()));
    }
    let offset = u32::try_from(data.as_ptr() as usize - bytes.as_ptr() as usize)
        .map_err(|_| GameDataError::Decode(format!("{field} offset exceeds u32")))?;
    let slice = &data[..len];
    std::str::from_utf8(slice)
        .map_err(|err| GameDataError::Decode(format!("{field} is not valid UTF-8: {err}")))?;
    data.advance(len);
    Ok(TextRef {
        offset,
        len: u32::try_from(len)
            .map_err(|_| GameDataError::Decode(format!("{field} length exceeds u32")))?,
    })
}

pub(super) fn read_list_ref(
    bytes: &[u8],
    data: &mut &[u8],
    string_pool_base: Option<u32>,
    field: &str,
) -> Result<ListRef, GameDataError> {
    let len = read_u32(data, &format!("{field}.len"))? as usize;
    if data.remaining() < len {
        return Err(truncated(field, data.remaining()));
    }
    let offset = u32::try_from(data.as_ptr() as usize - bytes.as_ptr() as usize)
        .map_err(|_| GameDataError::Decode(format!("{field} offset exceeds u32")))?;
    data.advance(len);
    Ok(ListRef {
        offset,
        len: u32::try_from(len)
            .map_err(|_| GameDataError::Decode(format!("{field} length exceeds u32")))?,
        string_pool_base,
    })
}

#[cfg(any(feature = "authoring", test))]
pub(super) fn write_string_ref(bytes: &mut Vec<u8>, pool_offset: u32, len: u32) {
    bytes.put_u32_le(pool_offset);
    bytes.put_u32_le(len);
}

pub(super) fn string_pool_base(
    section_file: &ParsedSectionFile,
    bytes: &[u8],
) -> Result<Option<u32>, GameDataError> {
    let Some(section_offset) = section_file.section_offset(TableSectionId::StringPool as u32)
    else {
        return Ok(None);
    };
    let payload = section_file.section_payload(bytes, TableSectionId::StringPool as u32)?;
    if payload.len() < STRING_POOL_HEADER_LEN as usize {
        return Err(GameDataError::Decode(
            "STRING_POOL section shorter than header".into(),
        ));
    }
    let blob_len = u32::from_le_bytes(payload[..4].try_into().expect("header length"));
    if payload.len() != STRING_POOL_HEADER_LEN as usize + blob_len as usize {
        return Err(GameDataError::Decode(format!(
            "STRING_POOL section length {} does not match blob length {blob_len}",
            payload.len()
        )));
    }
    let base = section_offset
        .checked_add(u64::from(STRING_POOL_HEADER_LEN))
        .ok_or_else(|| GameDataError::Decode("STRING_POOL base offset overflow".into()))?;
    Ok(Some(u32::try_from(base).map_err(|_| {
        GameDataError::Decode("STRING_POOL base offset exceeds u32".into())
    })?))
}

pub(super) fn read_pooled_string_ref(
    bytes: &[u8],
    data: &mut &[u8],
    pool_base: u32,
    field: &str,
) -> Result<TextRef, GameDataError> {
    let pool_offset = read_u32(data, &format!("{field}.pool_offset"))?;
    let len = read_u32(data, &format!("{field}.len"))? as usize;
    let offset = pool_base
        .checked_add(pool_offset)
        .ok_or_else(|| GameDataError::Decode(format!("{field} absolute offset overflow")))?;
    let end = offset as usize + len;
    let slice = bytes.get(offset as usize..end).ok_or_else(|| {
        GameDataError::Decode(format!(
            "{field} span {offset}..{end} out of bounds in {} byte table asset",
            bytes.len()
        ))
    })?;
    std::str::from_utf8(slice)
        .map_err(|err| GameDataError::Decode(format!("{field} is not valid UTF-8: {err}")))?;
    Ok(TextRef {
        offset,
        len: u32::try_from(len)
            .map_err(|_| GameDataError::Decode(format!("{field} length exceeds u32")))?,
    })
}
