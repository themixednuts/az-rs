use super::super::bank::{
    WwiseBankHeader, WwiseHierarchyObject, WwiseHierarchyObjectKind, WwiseMediaEntry,
    parse_event_body,
};
use super::super::error::WwiseSoundBankParseError;
use super::super::ids::{WwiseBankId, WwiseMediaId, WwiseObjectId, WwiseSectionId};
use super::reader::{read_optional_u32_at, read_u32_at};

/// Parse a `BKHD` section payload.
///
/// # Errors
///
/// Returns [`WwiseSoundBankParseError::InvalidBankHeaderSize`] if `payload` is
/// shorter than the 8-byte version + bank id prefix, or
/// [`WwiseSoundBankParseError::UnexpectedEof`] if a declared field runs past
/// the payload.
pub fn parse_bank_header(payload: &[u8]) -> Result<WwiseBankHeader, WwiseSoundBankParseError> {
    if payload.len() < 8 {
        return Err(WwiseSoundBankParseError::InvalidBankHeaderSize {
            size: payload.len(),
        });
    }

    Ok(WwiseBankHeader {
        version: read_u32_at(payload, 0, "BKHD version")?,
        bank_id: WwiseBankId(read_u32_at(payload, 4, "BKHD bank id")?),
        language_id: read_optional_u32_at(payload, 8, "BKHD language id")?,
        feedback_in_bank: read_optional_u32_at(payload, 12, "BKHD feedback flag")?,
    })
}

/// Parse a `DIDX` media-index section payload.
///
/// # Errors
///
/// Returns [`WwiseSoundBankParseError::InvalidDidxSize`] if `payload` is not a
/// whole number of 12-byte records.
pub fn parse_media_index(payload: &[u8]) -> Result<Vec<WwiseMediaEntry>, WwiseSoundBankParseError> {
    if !payload.len().is_multiple_of(12) {
        return Err(WwiseSoundBankParseError::InvalidDidxSize {
            size: payload.len(),
        });
    }

    let mut entries = Vec::with_capacity(payload.len() / 12);
    for chunk in payload.chunks_exact(12) {
        entries.push(WwiseMediaEntry {
            id: WwiseMediaId(read_u32_at(chunk, 0, "DIDX media id")?),
            offset: read_u32_at(chunk, 4, "DIDX media offset")?,
            size: read_u32_at(chunk, 8, "DIDX media size")?,
        });
    }
    Ok(entries)
}

/// Parse a `HIRC` section payload into hierarchy object headers.
///
/// # Errors
///
/// Returns [`WwiseSoundBankParseError::UnexpectedEof`] if the section is
/// truncated mid-record, [`WwiseSoundBankParseError::InvalidHircObjectSize`] or
/// [`WwiseSoundBankParseError::HircObjectOutOfBounds`] if an object's declared
/// size is under 4 bytes or runs past the payload,
/// [`WwiseSoundBankParseError::SectionOffsetTooLarge`] if an absolute object
/// offset does not fit in `u32`, and any error [`parse_event_body`] returns for
/// an Event object.
pub fn parse_hierarchy(
    payload: &[u8],
    payload_offset: u32,
) -> Result<Vec<WwiseHierarchyObject>, WwiseSoundBankParseError> {
    let object_count = read_u32_at(payload, 0, "HIRC object count")?;
    let mut cursor = 4usize;
    let mut objects = Vec::with_capacity(object_count as usize);
    let payload_offset = usize::try_from(payload_offset).map_err(|_| {
        WwiseSoundBankParseError::SectionOffsetTooLarge {
            section: WwiseSectionId::HIRC,
        }
    })?;

    for index in 0..object_count {
        let object_type = *payload
            .get(cursor)
            .ok_or(WwiseSoundBankParseError::UnexpectedEof {
                context: "HIRC object type",
            })?;
        cursor += 1;

        let data_size = read_u32_at(payload, cursor, "HIRC object size")?;
        cursor += 4;

        if data_size < 4 {
            return Err(WwiseSoundBankParseError::InvalidHircObjectSize {
                index,
                size: data_size,
            });
        }

        let data_size_usize = usize::try_from(data_size)
            .map_err(|_| WwiseSoundBankParseError::HircObjectOutOfBounds { index })?;
        let object_end = cursor
            .checked_add(data_size_usize)
            .ok_or(WwiseSoundBankParseError::HircObjectOutOfBounds { index })?;
        if object_end > payload.len() {
            return Err(WwiseSoundBankParseError::HircObjectOutOfBounds { index });
        }
        let data_offset_usize = payload_offset.checked_add(cursor).ok_or(
            WwiseSoundBankParseError::SectionOffsetTooLarge {
                section: WwiseSectionId::HIRC,
            },
        )?;
        let data_offset = u32::try_from(data_offset_usize).map_err(|_| {
            WwiseSoundBankParseError::SectionOffsetTooLarge {
                section: WwiseSectionId::HIRC,
            }
        })?;
        let kind = WwiseHierarchyObjectKind::new(object_type);
        let object_id = WwiseObjectId(read_u32_at(payload, cursor, "HIRC object id")?);
        let body = &payload[cursor + 4..object_end];
        let event_action_count = if kind == WwiseHierarchyObjectKind::EVENT {
            Some(parse_event_body(object_id, body)?.action_count())
        } else {
            None
        };

        objects.push(WwiseHierarchyObject {
            kind,
            object_id,
            data_offset,
            data_size,
            event_action_count,
        });
        cursor = object_end;
    }

    Ok(objects)
}
