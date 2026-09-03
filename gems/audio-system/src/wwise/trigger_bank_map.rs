//! Wwise ATL trigger-bank map metadata.

use crate::{AudioControlId, WwiseBankId, WwiseTriggerBankMapParseError};

pub const WWISE_TRIGGER_BANK_MAP_FILE: &str = "libs/gameaudio/wwise/triggerbankmapatlbin.bin";
pub const WWISE_TRIGGER_BANK_MAP_RECORD_SIZE: usize = 16;

/// `triggerbankmapatlbin.bin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WwiseTriggerBankMap<'a> {
    bytes: &'a [u8],
}

impl<'a> WwiseTriggerBankMap<'a> {
    /// Parse a Wwise ATL trigger-bank map.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload is not a sequence of 16-byte records.
    pub const fn parse(bytes: &'a [u8]) -> Result<Self, WwiseTriggerBankMapParseError> {
        if !bytes
            .len()
            .is_multiple_of(WWISE_TRIGGER_BANK_MAP_RECORD_SIZE)
        {
            return Err(WwiseTriggerBankMapParseError::InvalidSize { size: bytes.len() });
        }
        Ok(Self { bytes })
    }

    #[inline]
    #[must_use]
    pub const fn len(self) -> usize {
        self.bytes.len() / WWISE_TRIGGER_BANK_MAP_RECORD_SIZE
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    #[inline]
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    #[must_use]
    pub const fn entries(self) -> WwiseTriggerBankMapEntries<'a> {
        WwiseTriggerBankMapEntries {
            bytes: self.bytes,
            position: 0,
        }
    }
}

impl<'a> IntoIterator for WwiseTriggerBankMap<'a> {
    type Item = WwiseTriggerBankMapEntry;
    type IntoIter = WwiseTriggerBankMapEntries<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries()
    }
}

/// One ATL control-to-bank map record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WwiseTriggerBankMapEntry {
    pub bank_id: WwiseBankId,
    pub control_ids: [AudioControlId; 3],
}

/// Borrowed iterator over Wwise trigger-bank map entries.
///
/// Deliberately not `Copy`: a copied iterator silently restarts from its own
/// cursor, which reads as a bug at every call site.
#[derive(Debug, Clone)]
pub struct WwiseTriggerBankMapEntries<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Iterator for WwiseTriggerBankMapEntries<'_> {
    type Item = WwiseTriggerBankMapEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let end = self
            .position
            .checked_add(WWISE_TRIGGER_BANK_MAP_RECORD_SIZE)?;
        let bytes = self.bytes.get(self.position..end)?;
        self.position = end;
        Some(WwiseTriggerBankMapEntry {
            bank_id: WwiseBankId(read_u32(bytes, 0)),
            control_ids: [
                AudioControlId(u64::from(read_u32(bytes, 4))),
                AudioControlId(u64::from(read_u32(bytes, 8))),
                AudioControlId(u64::from(read_u32(bytes, 12))),
            ],
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = (self.bytes.len() - self.position) / WWISE_TRIGGER_BANK_MAP_RECORD_SIZE;
        (len, Some(len))
    }
}

impl ExactSizeIterator for WwiseTriggerBankMapEntries<'_> {}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("slice size"))
}
