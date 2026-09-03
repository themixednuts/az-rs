use bevy::prelude::*;

use super::super::error::WwiseSoundBankParseError;
use super::super::ids::WwiseObjectId;

/// Wwise HIRC object type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect)]
#[repr(transparent)]
pub struct WwiseHierarchyObjectKind(pub u8);

impl WwiseHierarchyObjectKind {
    pub const STATE: Self = Self(1);
    pub const SOUND: Self = Self(2);
    pub const ACTION: Self = Self(3);
    pub const EVENT: Self = Self(4);
    pub const RANDOM_SEQUENCE_CONTAINER: Self = Self(5);
    pub const SWITCH_CONTAINER: Self = Self(6);
    pub const ACTOR_MIXER: Self = Self(7);
    pub const AUDIO_BUS: Self = Self(8);
    pub const BLEND_CONTAINER: Self = Self(9);
    pub const MUSIC_SEGMENT: Self = Self(10);
    pub const MUSIC_TRACK: Self = Self(11);
    pub const MUSIC_SWITCH_CONTAINER: Self = Self(12);
    pub const MUSIC_PLAYLIST_CONTAINER: Self = Self(13);
    pub const ATTENUATION: Self = Self(14);
    pub const DIALOGUE_EVENT: Self = Self(15);
    pub const MOTION_BUS: Self = Self(16);
    pub const MOTION_FX: Self = Self(17);
    pub const EFFECT: Self = Self(18);
    pub const AUXILIARY_BUS: Self = Self(19);
    pub const LFO_MODULATOR: Self = Self(20);
    pub const ENVELOPE_MODULATOR: Self = Self(21);
    pub const AUDIO_DEVICE: Self = Self(22);

    #[must_use]
    pub const fn new(kind: u8) -> Self {
        Self(kind)
    }

    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self.0 {
            1 => "State",
            2 => "Sound",
            3 => "Action",
            4 => "Event",
            5 => "RandomSequenceContainer",
            6 => "SwitchContainer",
            7 => "ActorMixer",
            8 => "AudioBus",
            9 => "BlendContainer",
            10 => "MusicSegment",
            11 => "MusicTrack",
            12 => "MusicSwitchContainer",
            13 => "MusicPlaylistContainer",
            14 => "Attenuation",
            15 => "DialogueEvent",
            16 => "MotionBus",
            17 => "MotionFx",
            18 => "Effect",
            19 => "AuxiliaryBus",
            20 => "LfoModulator",
            21 => "EnvelopeModulator",
            22 => "AudioDevice",
            _ => "Unknown",
        }
    }
}

/// Wwise hierarchy object header from a `HIRC` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub struct WwiseHierarchyObject {
    pub kind: WwiseHierarchyObjectKind,
    pub object_id: WwiseObjectId,
    /// Absolute offset of the object data, including the 4-byte object id.
    pub data_offset: u32,
    /// Object data size, including the 4-byte object id.
    pub data_size: u32,
    /// Parsed action count for Event objects.
    pub event_action_count: Option<u32>,
}

impl WwiseHierarchyObject {
    const OBJECT_ID_SIZE: u32 = 4;

    #[must_use]
    pub const fn body_offset(self) -> u32 {
        self.data_offset.saturating_add(Self::OBJECT_ID_SIZE)
    }

    #[must_use]
    pub const fn body_size(self) -> u32 {
        self.data_size.saturating_sub(Self::OBJECT_ID_SIZE)
    }

    /// Borrow this object's payload out of the whole bank byte buffer.
    ///
    /// # Errors
    ///
    /// Returns [`WwiseSoundBankParseError::HircObjectDataOutOfBounds`] if
    /// `body_offset() + body_size()` overflows or runs past the end of
    /// `bank_bytes`.
    pub fn body(self, bank_bytes: &[u8]) -> Result<&[u8], WwiseSoundBankParseError> {
        let start = self.body_offset() as usize;
        let size = self.body_size() as usize;
        let end =
            start
                .checked_add(size)
                .ok_or(WwiseSoundBankParseError::HircObjectDataOutOfBounds {
                    object_id: self.object_id,
                })?;
        if end > bank_bytes.len() {
            return Err(WwiseSoundBankParseError::HircObjectDataOutOfBounds {
                object_id: self.object_id,
            });
        }

        Ok(&bank_bytes[start..end])
    }

    /// Decode this object as an Event, or `None` if it is another kind.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::body`] returns, or
    /// [`WwiseSoundBankParseError::HircPackedIntegerOverflow`] /
    /// [`WwiseSoundBankParseError::UnexpectedEof`] /
    /// [`WwiseSoundBankParseError::HircEventActionListOutOfBounds`] from
    /// [`parse_event_body`] when the action list is malformed.
    pub fn event(
        self,
        bank_bytes: &[u8],
    ) -> Result<Option<WwiseEventObject<'_>>, WwiseSoundBankParseError> {
        if self.kind != WwiseHierarchyObjectKind::EVENT {
            return Ok(None);
        }

        parse_event_body(self.object_id, self.body(bank_bytes)?).map(Some)
    }
}

/// Borrowed view of a Wwise Event HIRC object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WwiseEventObject<'a> {
    object_id: WwiseObjectId,
    action_count: u32,
    action_bytes: &'a [u8],
}

impl<'a> WwiseEventObject<'a> {
    #[must_use]
    pub const fn object_id(self) -> WwiseObjectId {
        self.object_id
    }

    #[must_use]
    pub const fn action_count(self) -> u32 {
        self.action_count
    }

    #[must_use]
    pub fn action_ids(self) -> WwiseEventActionIds<'a> {
        WwiseEventActionIds {
            chunks: self.action_bytes.chunks_exact(4),
        }
    }
}

/// Borrowed iterator over Wwise Event action object ids.
#[derive(Debug, Clone)]
pub struct WwiseEventActionIds<'a> {
    chunks: std::slice::ChunksExact<'a, u8>,
}

impl Iterator for WwiseEventActionIds<'_> {
    type Item = WwiseObjectId;

    fn next(&mut self) -> Option<Self::Item> {
        let chunk = self.chunks.next()?;
        Some(WwiseObjectId(u32::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3],
        ])))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.chunks.size_hint()
    }
}

impl ExactSizeIterator for WwiseEventActionIds<'_> {}

/// Parse the payload of a `HIRC` Event object into a borrowed view.
///
/// # Errors
///
/// Returns [`WwiseSoundBankParseError::UnexpectedEof`] or
/// [`WwiseSoundBankParseError::HircPackedIntegerOverflow`] if the leading
/// packed action count is truncated or overflows `u32`, and
/// [`WwiseSoundBankParseError::HircEventActionListOutOfBounds`] if the declared
/// action list does not fit in `body`.
pub fn parse_event_body(
    object_id: WwiseObjectId,
    body: &[u8],
) -> Result<WwiseEventObject<'_>, WwiseSoundBankParseError> {
    let (action_count, cursor) = read_packed_u32(body, "HIRC event action count")?;
    let action_bytes_len = action_count
        .checked_mul(4)
        .and_then(|len| usize::try_from(len).ok())
        .ok_or(WwiseSoundBankParseError::HircEventActionListOutOfBounds { object_id })?;
    let action_bytes_end = cursor
        .checked_add(action_bytes_len)
        .ok_or(WwiseSoundBankParseError::HircEventActionListOutOfBounds { object_id })?;
    if action_bytes_end > body.len() {
        return Err(WwiseSoundBankParseError::HircEventActionListOutOfBounds { object_id });
    }

    Ok(WwiseEventObject {
        object_id,
        action_count,
        action_bytes: &body[cursor..action_bytes_end],
    })
}

fn read_packed_u32(
    bytes: &[u8],
    context: &'static str,
) -> Result<(u32, usize), WwiseSoundBankParseError> {
    let mut cursor = 0usize;
    let mut byte = *bytes
        .get(cursor)
        .ok_or(WwiseSoundBankParseError::UnexpectedEof { context })?;
    cursor += 1;
    let mut value = u32::from(byte & 0x7f);

    while byte & 0x80 != 0 {
        byte = *bytes
            .get(cursor)
            .ok_or(WwiseSoundBankParseError::UnexpectedEof { context })?;
        cursor += 1;
        if value > u32::MAX >> 7 {
            return Err(WwiseSoundBankParseError::HircPackedIntegerOverflow { context });
        }
        value = (value << 7) | u32::from(byte & 0x7f);
    }

    Ok((value, cursor))
}
