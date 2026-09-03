use crate::ParseError;

pub const STAR_MAGIC: &[u8; 4] = b"STAR";
pub const STAR_VERSION: u32 = 0x0001_0001;
pub const STAR_RECORD_SIZE: usize = 12;
pub const STAR_HEADER_SIZE: usize = 12;

/// `engineassets/sky/stars.dat`.
///
/// Follows Lumberyard's `dev/Code/CryEngine/RenderDll/Common/RendElements/CRESky.cpp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StarsDat<'a> {
    records: &'a [u8],
    count: u32,
}

impl<'a> StarsDat<'a> {
    /// Parse a star catalog payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the tag, version, count, or file length is invalid.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let magic = bytes.get(..4).ok_or(ParseError::UnexpectedEof {
            offset: 0,
            needed: 4,
            actual: bytes.len(),
        })?;
        if magic != STAR_MAGIC {
            return Err(ParseError::InvalidMagic {
                asset: "stars.dat",
                expected: STAR_MAGIC,
                found: magic.to_vec(),
            });
        }

        let version = read_u32_le(bytes, 4)?;
        if version != STAR_VERSION {
            return Err(ParseError::UnsupportedVersion {
                asset: "stars.dat",
                expected: i64::from(STAR_VERSION),
                found: i64::from(version),
            });
        }

        let count = read_u32_le(bytes, 8)?;
        let record_bytes = (count as usize)
            .checked_mul(STAR_RECORD_SIZE)
            .ok_or(ParseError::IntegerOverflow)?;
        let expected_len = STAR_HEADER_SIZE
            .checked_add(record_bytes)
            .ok_or(ParseError::IntegerOverflow)?;
        if bytes.len() != expected_len {
            return Err(ParseError::ChunkSizeMismatch {
                declared: expected_len,
                actual: bytes.len(),
            });
        }

        Ok(Self {
            records: &bytes[STAR_HEADER_SIZE..],
            count,
        })
    }

    #[inline]
    #[must_use]
    pub const fn len(self) -> u32 {
        self.count
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    #[inline]
    #[must_use]
    pub const fn records(self) -> StarRecords<'a> {
        StarRecords {
            bytes: self.records,
            index: 0,
            count: self.count,
        }
    }
}

impl<'a> IntoIterator for StarsDat<'a> {
    type Item = StarRecord;
    type IntoIter = StarRecords<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.records()
    }
}

/// One star record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StarRecord {
    pub right_ascension: f32,
    pub declination: f32,
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub magnitude: u8,
}

/// Borrowed iterator over star records.
#[derive(Debug, Clone)]
pub struct StarRecords<'a> {
    bytes: &'a [u8],
    index: u32,
    count: u32,
}

impl Iterator for StarRecords<'_> {
    type Item = StarRecord;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index == self.count {
            return None;
        }
        let offset = self.index as usize * STAR_RECORD_SIZE;
        let bytes = &self.bytes[offset..offset + STAR_RECORD_SIZE];
        self.index += 1;
        Some(StarRecord {
            right_ascension: f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            declination: f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            red: bytes[8],
            green: bytes[9],
            blue: bytes[10],
            magnitude: bytes[11],
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = (self.count - self.index) as usize;
        (len, Some(len))
    }
}

impl ExactSizeIterator for StarRecords<'_> {}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, ParseError> {
    let window = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| ParseError::UnexpectedEof {
            offset,
            needed: 4,
            actual: bytes.len().saturating_sub(offset),
        })?;
    Ok(u32::from_le_bytes([
        window[0], window[1], window[2], window[3],
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_star_file() {
        let bytes = [b'S', b'T', b'A', b'R', 1, 0, 1, 0, 0, 0, 0, 0];
        let stars = StarsDat::parse(&bytes).unwrap();

        assert_eq!(stars.len(), 0);
        assert!(stars.is_empty());
    }
}
