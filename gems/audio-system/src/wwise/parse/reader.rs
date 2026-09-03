use super::super::error::WwiseSoundBankParseError;
use super::super::ids::WwiseSectionId;

pub fn read_section_id_at(
    bytes: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<WwiseSectionId, WwiseSoundBankParseError> {
    let tag = read_4_at(bytes, offset, context)?;
    Ok(WwiseSectionId::from_tag(tag))
}

pub fn read_optional_u32_at(
    bytes: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<Option<u32>, WwiseSoundBankParseError> {
    if offset >= bytes.len() {
        return Ok(None);
    }
    Ok(Some(read_u32_at(bytes, offset, context)?))
}

pub fn read_u32_at(
    bytes: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<u32, WwiseSoundBankParseError> {
    Ok(u32::from_le_bytes(read_4_at(bytes, offset, context)?))
}

fn read_4_at(
    bytes: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<[u8; 4], WwiseSoundBankParseError> {
    let end = offset
        .checked_add(4)
        .ok_or(WwiseSoundBankParseError::UnexpectedEof { context })?;
    if end > bytes.len() {
        return Err(WwiseSoundBankParseError::UnexpectedEof { context });
    }
    Ok([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
