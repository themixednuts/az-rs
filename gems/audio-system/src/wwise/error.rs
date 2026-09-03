//! Wwise asset and parser errors.

use thiserror::Error;

use super::ids::{WwiseMediaId, WwiseObjectId, WwiseSectionId};
use super::media_file::WwiseMediaParseError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WwiseSoundBankParseError {
    #[error("Wwise soundbank is missing a BKHD section")]
    MissingBankHeader,
    #[error("unexpected end of Wwise soundbank while reading {context}")]
    UnexpectedEof { context: &'static str },
    #[error("Wwise section {section} extends past the end of the bank")]
    SectionOutOfBounds { section: WwiseSectionId },
    #[error("Wwise section {section} offset does not fit in u32")]
    SectionOffsetTooLarge { section: WwiseSectionId },
    #[error("BKHD section has invalid size {size}; expected at least 8 bytes")]
    InvalidBankHeaderSize { size: usize },
    #[error("DIDX section size {size} is not a multiple of 12")]
    InvalidDidxSize { size: usize },
    #[error("HIRC object {index} has invalid size {size}; expected at least 4 bytes")]
    InvalidHircObjectSize { index: u32, size: u32 },
    #[error("HIRC object {index} extends past the end of the section")]
    HircObjectOutOfBounds { index: u32 },
    #[error("HIRC object {object_id:?} data range points past the end of the bank")]
    HircObjectDataOutOfBounds { object_id: WwiseObjectId },
    #[error("HIRC packed integer overflow while reading {context}")]
    HircPackedIntegerOverflow { context: &'static str },
    #[error("HIRC event object {object_id:?} action list extends past the object payload")]
    HircEventActionListOutOfBounds { object_id: WwiseObjectId },
    #[error("DIDX media id {media_id:?} points past the DATA section")]
    InvalidMediaRange { media_id: WwiseMediaId },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WwiseTriggerBankMapParseError {
    #[error("Wwise trigger bank map size {size} is not a multiple of 16")]
    InvalidSize { size: usize },
}

/// Error for Wwise asset reads.
#[derive(Debug, Error)]
pub enum WwiseAssetLoadError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse Wwise soundbank: {0}")]
    SoundBank(#[from] WwiseSoundBankParseError),
    #[error("failed to parse Wwise media: {0}")]
    Media(#[from] WwiseMediaParseError),
}
