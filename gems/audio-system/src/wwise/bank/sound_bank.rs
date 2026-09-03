use bevy::prelude::*;

use super::super::error::WwiseSoundBankParseError;
use super::super::ids::WwiseSectionId;
use super::super::parse::{
    parse_bank_header, parse_hierarchy, parse_media_index, read_section_id_at, read_u32_at,
};
use super::{WwiseBankHeader, WwiseBankSection, WwiseHierarchyObject, WwiseMediaEntry};

/// Metadata parsed from a Wwise `.bnk` soundbank.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
pub struct WwiseSoundBank {
    pub header: Option<WwiseBankHeader>,
    pub sections: Vec<WwiseBankSection>,
    pub media: Vec<WwiseMediaEntry>,
    pub hierarchy: Vec<WwiseHierarchyObject>,
}

impl WwiseSoundBank {
    /// Parse the section table and metadata of a Wwise `.bnk` soundbank.
    ///
    /// # Errors
    ///
    /// Returns [`WwiseSoundBankParseError::UnexpectedEof`] if a section header
    /// is truncated, [`WwiseSoundBankParseError::SectionOutOfBounds`] or
    /// [`WwiseSoundBankParseError::SectionOffsetTooLarge`] if a section's
    /// declared payload does not lie inside `bytes` or its offset does not fit
    /// in `u32`, [`WwiseSoundBankParseError::MissingBankHeader`] if no `BKHD`
    /// section was seen, [`WwiseSoundBankParseError::InvalidMediaRange`] if a
    /// `DIDX` entry points outside the `DATA` section, and any error the
    /// `BKHD`, `DIDX`, and `HIRC` section parsers return.
    pub fn parse(bytes: &[u8]) -> Result<Self, WwiseSoundBankParseError> {
        let mut bank = Self::default();
        let mut cursor = 0usize;

        while cursor < bytes.len() {
            let id = read_section_id_at(bytes, cursor, "section id")?;
            let size = read_u32_at(bytes, cursor + 4, "section size")?;
            let payload_offset = cursor
                .checked_add(8)
                .ok_or(WwiseSoundBankParseError::SectionOutOfBounds { section: id })?;
            let payload_size = usize::try_from(size)
                .map_err(|_| WwiseSoundBankParseError::SectionOutOfBounds { section: id })?;
            let payload_end = payload_offset
                .checked_add(payload_size)
                .ok_or(WwiseSoundBankParseError::SectionOutOfBounds { section: id })?;
            if payload_end > bytes.len() {
                return Err(WwiseSoundBankParseError::SectionOutOfBounds { section: id });
            }

            let section = WwiseBankSection {
                id,
                offset: u32::try_from(payload_offset)
                    .map_err(|_| WwiseSoundBankParseError::SectionOffsetTooLarge { section: id })?,
                size,
            };
            let payload = &bytes[payload_offset..payload_end];

            if id == WwiseSectionId::BKHD {
                bank.header = Some(parse_bank_header(payload)?);
            } else if id == WwiseSectionId::DIDX {
                bank.media = parse_media_index(payload)?;
            } else if id == WwiseSectionId::HIRC {
                bank.hierarchy = parse_hierarchy(payload, section.offset)?;
            }

            bank.sections.push(section);
            cursor = payload_end;
        }

        if bank.header.is_none() {
            return Err(WwiseSoundBankParseError::MissingBankHeader);
        }

        if let Some(data_section) = bank.section(WwiseSectionId::DATA) {
            for entry in &bank.media {
                match entry.end_offset() {
                    Some(end) if end <= data_section.size => {}
                    _ => {
                        return Err(WwiseSoundBankParseError::InvalidMediaRange {
                            media_id: entry.id,
                        });
                    }
                }
            }
        }

        Ok(bank)
    }

    #[must_use]
    pub fn section(&self, id: WwiseSectionId) -> Option<&WwiseBankSection> {
        self.sections.iter().find(|section| section.id == id)
    }

    #[must_use]
    pub fn has_section(&self, id: WwiseSectionId) -> bool {
        self.section(id).is_some()
    }
}
