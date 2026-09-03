use bevy::math::{Affine3A, Mat3A, Vec3A, Vec4, bounding::Aabb3d};

use crate::ParseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endian {
    Little,
    Big,
}

impl Endian {
    #[inline]
    #[must_use]
    pub const fn from_big_endian_flag(big_endian: bool) -> Self {
        if big_endian { Self::Big } else { Self::Little }
    }

    /// Read a 16-bit signed integer at `offset`.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::UnexpectedEof`] when fewer than 2 bytes remain at
    /// `offset`.
    #[inline]
    pub fn read_i16(self, bytes: &[u8], offset: usize) -> Result<i16, ParseError> {
        let window = bytes
            .get(offset..offset + 2)
            .ok_or_else(|| ParseError::UnexpectedEof {
                offset,
                needed: 2,
                actual: bytes.len().saturating_sub(offset),
            })?;
        Ok(match self {
            Self::Little => i16::from_le_bytes([window[0], window[1]]),
            Self::Big => i16::from_be_bytes([window[0], window[1]]),
        })
    }

    /// Read a 16-bit unsigned integer at `offset`.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::UnexpectedEof`] when fewer than 2 bytes remain at
    /// `offset`.
    #[inline]
    pub fn read_u16(self, bytes: &[u8], offset: usize) -> Result<u16, ParseError> {
        let window = bytes
            .get(offset..offset + 2)
            .ok_or_else(|| ParseError::UnexpectedEof {
                offset,
                needed: 2,
                actual: bytes.len().saturating_sub(offset),
            })?;
        Ok(match self {
            Self::Little => u16::from_le_bytes([window[0], window[1]]),
            Self::Big => u16::from_be_bytes([window[0], window[1]]),
        })
    }

    /// Read a 32-bit signed integer at `offset`.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::UnexpectedEof`] when fewer than 4 bytes remain at
    /// `offset`.
    #[inline]
    pub fn read_i32(self, bytes: &[u8], offset: usize) -> Result<i32, ParseError> {
        let window = bytes
            .get(offset..offset + 4)
            .ok_or_else(|| ParseError::UnexpectedEof {
                offset,
                needed: 4,
                actual: bytes.len().saturating_sub(offset),
            })?;
        Ok(match self {
            Self::Little => i32::from_le_bytes([window[0], window[1], window[2], window[3]]),
            Self::Big => i32::from_be_bytes([window[0], window[1], window[2], window[3]]),
        })
    }

    /// Read a 32-bit unsigned integer at `offset`.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::UnexpectedEof`] when fewer than 4 bytes remain at
    /// `offset`.
    #[inline]
    pub fn read_u32(self, bytes: &[u8], offset: usize) -> Result<u32, ParseError> {
        let window = bytes
            .get(offset..offset + 4)
            .ok_or_else(|| ParseError::UnexpectedEof {
                offset,
                needed: 4,
                actual: bytes.len().saturating_sub(offset),
            })?;
        Ok(match self {
            Self::Little => u32::from_le_bytes([window[0], window[1], window[2], window[3]]),
            Self::Big => u32::from_be_bytes([window[0], window[1], window[2], window[3]]),
        })
    }

    /// Read a 64-bit unsigned integer at `offset`.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::UnexpectedEof`] when fewer than 8 bytes remain at
    /// `offset`.
    #[inline]
    pub fn read_u64(self, bytes: &[u8], offset: usize) -> Result<u64, ParseError> {
        let window = bytes
            .get(offset..offset + 8)
            .ok_or_else(|| ParseError::UnexpectedEof {
                offset,
                needed: 8,
                actual: bytes.len().saturating_sub(offset),
            })?;
        Ok(match self {
            Self::Little => u64::from_le_bytes([
                window[0], window[1], window[2], window[3], window[4], window[5], window[6],
                window[7],
            ]),
            Self::Big => u64::from_be_bytes([
                window[0], window[1], window[2], window[3], window[4], window[5], window[6],
                window[7],
            ]),
        })
    }

    /// Read an IEEE-754 single-precision float at `offset`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::read_u32`] returns.
    #[inline]
    pub fn read_f32(self, bytes: &[u8], offset: usize) -> Result<f32, ParseError> {
        Ok(f32::from_bits(self.read_u32(bytes, offset)?))
    }

    /// Read three consecutive floats at `offset` as a [`Vec3A`].
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::read_f32`] returns for any of the three
    /// components.
    #[inline]
    pub fn read_vec3a(self, bytes: &[u8], offset: usize) -> Result<Vec3A, ParseError> {
        Ok(Vec3A::new(
            self.read_f32(bytes, offset)?,
            self.read_f32(bytes, offset + 4)?,
            self.read_f32(bytes, offset + 8)?,
        ))
    }

    /// Read four consecutive floats at `offset` as a [`Vec4`].
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::read_f32`] returns for any of the four
    /// components.
    #[inline]
    pub fn read_vec4(self, bytes: &[u8], offset: usize) -> Result<Vec4, ParseError> {
        Ok(Vec4::new(
            self.read_f32(bytes, offset)?,
            self.read_f32(bytes, offset + 4)?,
            self.read_f32(bytes, offset + 8)?,
            self.read_f32(bytes, offset + 12)?,
        ))
    }

    /// Read a min/max float pair at `offset` as an [`Aabb3d`].
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::read_vec3a`] returns for either bound.
    #[inline]
    pub fn read_aabb3d(self, bytes: &[u8], offset: usize) -> Result<Aabb3d, ParseError> {
        Ok(Aabb3d::from_min_max(
            self.read_vec3a(bytes, offset)?,
            self.read_vec3a(bytes, offset + 12)?,
        ))
    }

    /// Read a row-major 3x4 matrix at `offset` as an [`Affine3A`].
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::read_vec4`] returns for any of the three rows.
    #[inline]
    pub fn read_matrix34(self, bytes: &[u8], offset: usize) -> Result<Affine3A, ParseError> {
        let row_0 = self.read_vec4(bytes, offset)?;
        let row_1 = self.read_vec4(bytes, offset + 16)?;
        let row_2 = self.read_vec4(bytes, offset + 32)?;
        Ok(Affine3A::from_cols(
            Vec3A::new(row_0.x, row_1.x, row_2.x),
            Vec3A::new(row_0.y, row_1.y, row_2.y),
            Vec3A::new(row_0.z, row_1.z, row_2.z),
            Vec3A::new(row_0.w, row_1.w, row_2.w),
        ))
    }

    /// Read a row-major 3x3 matrix at `offset` as a [`Mat3A`].
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::read_vec3a`] returns for any of the three rows.
    #[inline]
    pub fn read_matrix33(self, bytes: &[u8], offset: usize) -> Result<Mat3A, ParseError> {
        let row_0 = self.read_vec3a(bytes, offset)?;
        let row_1 = self.read_vec3a(bytes, offset + 12)?;
        let row_2 = self.read_vec3a(bytes, offset + 24)?;
        Ok(Mat3A::from_cols(
            Vec3A::new(row_0.x, row_1.x, row_2.x),
            Vec3A::new(row_0.y, row_1.y, row_2.y),
            Vec3A::new(row_0.z, row_1.z, row_2.z),
        ))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    #[inline]
    #[must_use]
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    #[inline]
    #[must_use]
    pub(crate) const fn position(self) -> usize {
        self.position
    }

    #[inline]
    #[must_use]
    pub(crate) const fn remaining(self) -> usize {
        self.bytes.len() - self.position
    }

    #[inline]
    pub(crate) fn read_u8(&mut self) -> Result<u8, ParseError> {
        let byte = *self
            .bytes
            .get(self.position)
            .ok_or(ParseError::UnexpectedEof {
                offset: self.position,
                needed: 1,
                actual: 0,
            })?;
        self.position += 1;
        Ok(byte)
    }

    #[inline]
    pub(crate) fn read_i32(&mut self, endian: Endian) -> Result<i32, ParseError> {
        let value = endian.read_i32(self.bytes, self.position)?;
        self.position += 4;
        Ok(value)
    }

    #[inline]
    pub(crate) fn read_u32(&mut self, endian: Endian) -> Result<u32, ParseError> {
        let value = endian.read_u32(self.bytes, self.position)?;
        self.position += 4;
        Ok(value)
    }

    #[inline]
    pub(crate) fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], ParseError> {
        let start = self.position;
        let end = start.checked_add(len).ok_or(ParseError::IntegerOverflow)?;
        let bytes = self
            .bytes
            .get(start..end)
            .ok_or_else(|| ParseError::UnexpectedEof {
                offset: start,
                needed: len,
                actual: self.bytes.len().saturating_sub(start),
            })?;
        self.position = end;
        Ok(bytes)
    }

    #[inline]
    pub(crate) fn skip(&mut self, len: usize) -> Result<(), ParseError> {
        self.read_bytes(len).map(|_| ())
    }

    #[inline]
    pub(crate) fn align_remaining_to_4(&mut self) -> Result<usize, ParseError> {
        let padding = self.remaining() & 3;
        self.skip(padding)?;
        Ok(padding)
    }
}
