//! Wwise encoded media container metadata.

use std::fmt;

use bevy::prelude::*;

/// Parsed Wwise `.wem` RIFF/WAVE container metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
pub struct WwiseMediaInfo {
    pub riff_size: u32,
    pub chunks: Vec<WwiseMediaChunk>,
}

impl WwiseMediaInfo {
    /// Parse the RIFF/WAVE chunk table of a Wwise `.wem` container.
    ///
    /// # Errors
    ///
    /// Returns [`WwiseMediaParseError::TooShort`] if `bytes` is under 12 bytes,
    /// [`WwiseMediaParseError::InvalidRiffMagic`] or
    /// [`WwiseMediaParseError::InvalidWaveFormat`] if the `RIFF`/`WAVE` tags do
    /// not match, [`WwiseMediaParseError::DeclaredSizeOverflow`] or
    /// [`WwiseMediaParseError::Truncated`] if the declared RIFF size does not
    /// fit the buffer, [`WwiseMediaParseError::ChunkOutOfBounds`] if a chunk
    /// header or payload runs past the declared end, and
    /// [`WwiseMediaParseError::ChunkOffsetTooLarge`] if a payload offset does
    /// not fit in `u32`.
    pub fn parse(bytes: &[u8]) -> Result<Self, WwiseMediaParseError> {
        if bytes.len() < 12 {
            return Err(WwiseMediaParseError::TooShort { size: bytes.len() });
        }
        if &bytes[0..4] != b"RIFF" {
            return Err(WwiseMediaParseError::InvalidRiffMagic {
                actual: [bytes[0], bytes[1], bytes[2], bytes[3]],
            });
        }
        if &bytes[8..12] != b"WAVE" {
            return Err(WwiseMediaParseError::InvalidWaveFormat {
                actual: [bytes[8], bytes[9], bytes[10], bytes[11]],
            });
        }

        let riff_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let declared_end = usize::try_from(riff_size)
            .ok()
            .and_then(|size| size.checked_add(8))
            .ok_or(WwiseMediaParseError::DeclaredSizeOverflow { riff_size })?;
        if declared_end > bytes.len() {
            return Err(WwiseMediaParseError::Truncated {
                declared: declared_end,
                actual: bytes.len(),
            });
        }

        let mut cursor = 12usize;
        let mut chunks = Vec::new();
        while cursor < declared_end {
            let header_end =
                cursor
                    .checked_add(8)
                    .ok_or(WwiseMediaParseError::ChunkOutOfBounds {
                        offset: cursor,
                        size: 0,
                    })?;
            if header_end > declared_end {
                return Err(WwiseMediaParseError::ChunkOutOfBounds {
                    offset: cursor,
                    size: 0,
                });
            }

            let id = WwiseMediaChunkId::from_tag([
                bytes[cursor],
                bytes[cursor + 1],
                bytes[cursor + 2],
                bytes[cursor + 3],
            ]);
            let size = u32::from_le_bytes([
                bytes[cursor + 4],
                bytes[cursor + 5],
                bytes[cursor + 6],
                bytes[cursor + 7],
            ]);
            let payload_offset = header_end;
            let payload_size =
                usize::try_from(size).map_err(|_| WwiseMediaParseError::ChunkOutOfBounds {
                    offset: cursor,
                    size,
                })?;
            let payload_end = payload_offset.checked_add(payload_size).ok_or(
                WwiseMediaParseError::ChunkOutOfBounds {
                    offset: cursor,
                    size,
                },
            )?;
            if payload_end > declared_end {
                return Err(WwiseMediaParseError::ChunkOutOfBounds {
                    offset: cursor,
                    size,
                });
            }

            chunks.push(WwiseMediaChunk {
                id,
                offset: u32::try_from(payload_offset)
                    .map_err(|_| WwiseMediaParseError::ChunkOffsetTooLarge { id })?,
                size,
            });

            let padded_end = payload_end + usize::from(size & 1 != 0);
            if padded_end > declared_end && payload_end != declared_end {
                return Err(WwiseMediaParseError::ChunkOutOfBounds {
                    offset: payload_end,
                    size,
                });
            }
            cursor = padded_end.min(declared_end);
        }

        Ok(Self { riff_size, chunks })
    }

    #[must_use]
    pub fn chunk(&self, id: WwiseMediaChunkId) -> Option<&WwiseMediaChunk> {
        self.chunks.iter().find(|chunk| chunk.id == id)
    }

    #[must_use]
    pub fn has_chunk(&self, id: WwiseMediaChunkId) -> bool {
        self.chunk(id).is_some()
    }
}

/// Four-byte RIFF chunk identifier inside Wwise `.wem` media.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect)]
#[repr(transparent)]
pub struct WwiseMediaChunkId(pub u32);

impl WwiseMediaChunkId {
    pub const FMT: Self = Self::from_tag(*b"fmt ");
    pub const DATA: Self = Self::from_tag(*b"data");
    pub const CUE: Self = Self::from_tag(*b"cue ");
    pub const LIST: Self = Self::from_tag(*b"LIST");
    pub const SMPL: Self = Self::from_tag(*b"smpl");
    pub const VORB: Self = Self::from_tag(*b"vorb");

    #[must_use]
    pub const fn from_tag(tag: [u8; 4]) -> Self {
        Self(u32::from_le_bytes(tag))
    }

    #[must_use]
    pub const fn tag(self) -> [u8; 4] {
        self.0.to_le_bytes()
    }

    #[must_use]
    pub fn tag_string(self) -> String {
        self.tag()
            .into_iter()
            .map(|byte| {
                if byte.is_ascii_graphic() || byte == b' ' {
                    char::from(byte)
                } else {
                    '.'
                }
            })
            .collect()
    }
}

impl fmt::Display for WwiseMediaChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.tag_string())
    }
}

/// Raw RIFF chunk location inside Wwise `.wem` media.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub struct WwiseMediaChunk {
    pub id: WwiseMediaChunkId,
    /// Absolute offset of the chunk payload.
    pub offset: u32,
    /// Chunk payload size in bytes.
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WwiseMediaParseError {
    #[error("Wwise media is too short: need at least 12 bytes, got {size}")]
    TooShort { size: usize },
    #[error("invalid Wwise media RIFF magic {actual:?}")]
    InvalidRiffMagic { actual: [u8; 4] },
    #[error("invalid Wwise media WAVE tag {actual:?}")]
    InvalidWaveFormat { actual: [u8; 4] },
    #[error("Wwise media declared RIFF size overflows usize: {riff_size}")]
    DeclaredSizeOverflow { riff_size: u32 },
    #[error("Wwise media is truncated: declared {declared} bytes, got {actual}")]
    Truncated { declared: usize, actual: usize },
    #[error("Wwise media chunk at {offset} with size {size} is out of bounds")]
    ChunkOutOfBounds { offset: usize, size: u32 },
    #[error("Wwise media chunk {id} offset does not fit in u32")]
    ChunkOffsetTooLarge { id: WwiseMediaChunkId },
}
