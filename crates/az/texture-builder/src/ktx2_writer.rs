use thiserror::Error;

const KTX2_ID: &[u8; 12] = b"\xABKTX 20\xBB\r\n\x1A\n";
const HEADER_LEN: u32 = 80;
const LEVEL_INDEX_LEN: u32 = 24;
const LEVEL_COUNT: u32 = 1;
const TYPE_SIZE_BLOCK_COMPRESSED: u32 = 1;
const SUPERCOMPRESSION_NONE: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ktx2TextureShape {
    pub pixel_height: u32,
    pub pixel_depth: u32,
    pub layer_count: u32,
    pub face_count: u32,
}

#[derive(Debug, Error)]
pub enum Ktx2WriteError {
    #[error("image dimensions must be non-zero")]
    EmptyImage,
    #[error("image data is empty")]
    EmptyLevel,
    #[error("invalid KTX2 texture shape: {reason}")]
    InvalidShape { reason: &'static str },
    #[error("KTX2 DFD lookup: {0}")]
    Dfd(#[from] vk2dfd::Error),
    #[error("KTX2 section is too large")]
    SizeOverflow,
}

pub fn write_single_mip_with_shape(
    vk_format: u32,
    width: u32,
    shape: Ktx2TextureShape,
    level_data: &[u8],
) -> Result<Vec<u8>, Ktx2WriteError> {
    if width == 0 {
        return Err(Ktx2WriteError::EmptyImage);
    }
    if !matches!(shape.face_count, 1 | 6) {
        return Err(Ktx2WriteError::InvalidShape {
            reason: "face count must be 1 or 6",
        });
    }
    if shape.pixel_height == 0 && (shape.pixel_depth != 0 || shape.face_count != 1) {
        return Err(Ktx2WriteError::InvalidShape {
            reason: "1D textures cannot have depth or cubemap faces",
        });
    }
    if shape.pixel_depth != 0 && (shape.layer_count != 0 || shape.face_count != 1) {
        return Err(Ktx2WriteError::InvalidShape {
            reason: "3D textures cannot be arrays or cubemaps",
        });
    }
    if level_data.is_empty() {
        return Err(Ktx2WriteError::EmptyLevel);
    }

    let dfd_words = vk2dfd::vk2dfd(vk_format)?;
    let dfd_byte_length = u32::try_from(
        dfd_words
            .len()
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or(Ktx2WriteError::SizeOverflow)?,
    )
    .map_err(|_| Ktx2WriteError::SizeOverflow)?;
    let dfd_byte_offset = HEADER_LEN + LEVEL_INDEX_LEN * LEVEL_COUNT;
    let level_byte_offset = align4(dfd_byte_offset + dfd_byte_length);
    let level_byte_length =
        u64::try_from(level_data.len()).map_err(|_| Ktx2WriteError::SizeOverflow)?;

    let mut bytes = Vec::with_capacity(
        usize::try_from(level_byte_offset)
            .map_err(|_| Ktx2WriteError::SizeOverflow)?
            .checked_add(level_data.len())
            .ok_or(Ktx2WriteError::SizeOverflow)?,
    );
    bytes.extend_from_slice(KTX2_ID);
    push_u32(&mut bytes, vk_format);
    push_u32(&mut bytes, TYPE_SIZE_BLOCK_COMPRESSED);
    push_u32(&mut bytes, width);
    push_u32(&mut bytes, shape.pixel_height);
    push_u32(&mut bytes, shape.pixel_depth);
    push_u32(&mut bytes, shape.layer_count);
    push_u32(&mut bytes, shape.face_count);
    push_u32(&mut bytes, LEVEL_COUNT);
    push_u32(&mut bytes, SUPERCOMPRESSION_NONE);
    push_u32(&mut bytes, dfd_byte_offset);
    push_u32(&mut bytes, dfd_byte_length);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u64(&mut bytes, 0);
    push_u64(&mut bytes, 0);
    debug_assert_eq!(bytes.len(), HEADER_LEN as usize);

    push_u64(&mut bytes, u64::from(level_byte_offset));
    push_u64(&mut bytes, level_byte_length);
    push_u64(&mut bytes, level_byte_length);
    debug_assert_eq!(bytes.len(), (HEADER_LEN + LEVEL_INDEX_LEN) as usize);

    for word in dfd_words {
        push_u32(&mut bytes, *word);
    }
    while bytes.len() < level_byte_offset as usize {
        bytes.push(0);
    }
    bytes.extend_from_slice(level_data);

    Ok(bytes)
}

const fn align4(value: u32) -> u32 {
    (value + 3) & !3
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
