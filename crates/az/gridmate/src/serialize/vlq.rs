// GridMate VLQ marshalers (match C++ API in CompressionMarshal.h)

use derive_more::{AsRef, Deref, DerefMut, Display, From, Into};

use super::{
    buffer::{ReadBuffer, WriteBuffer},
    error::MarshalerError,
    marshaler::Marshaler,
};

/// Quantizes a u16 into 1..=3 bytes using `GridMate`'s VLQ format.
///
/// The wire shape is a `VlqU32` value bounded to `u16::MAX`; the dedicated
/// codec adds a range check on read and the type-level bound on write so call
/// sites that carry a `u16` field on the wire don't have to re-derive the
/// pattern every time.
#[derive(Debug, Clone, Copy, Default)]
pub struct VlqU16Marshaler;

impl VlqU16Marshaler {
    #[inline]
    pub fn marshal(&self, wb: &mut WriteBuffer, v: u16) {
        VlqU32Marshaler.marshal(wb, u32::from(v));
    }

    /// Read a `GridMate` VLQ value and narrow it to `u16`.
    ///
    /// # Errors
    ///
    /// Returns any error [`VlqU32Marshaler::unmarshal`] returns — a short read
    /// when the buffer ends mid-encoding. Returns
    /// [`MarshalerError::ContainerOverflow`] if the decoded value exceeds
    /// [`u16::MAX`].
    #[inline]
    pub fn unmarshal(&self, rb: &mut ReadBuffer) -> Result<u16, MarshalerError> {
        let value = VlqU32Marshaler.unmarshal(rb)?;
        u16::try_from(value).map_err(|_| MarshalerError::ContainerOverflow {
            len: value as usize,
            capacity: u16::MAX as usize,
        })
    }
}

/// Newtype wrapper around `u16` that marshals as a `u16`-bounded `GridMate` VLQ.
///
/// Use as the field type when the wire carries a logical u16 as a VLQ — a type
/// swap (`pub count: VlqU16`) is enough to get the correct wire shape and the
/// bound check.
///
/// Mirrors [`VlqU32`] / [`VlqU64`] ergonomics: `Deref<Target = u16>`,
/// `From<u16>`, `Into<u16>`, `PartialEq<u16>`, `Display`, `AsRef<u16>`.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    AsRef,
    Deref,
    DerefMut,
    Display,
    From,
    Into,
)]
pub struct VlqU16(pub u16);

impl VlqU16 {
    /// Construct from a raw `u16`. Same as `VlqU16::from(value)` but `const`.
    #[inline]
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Extract the inner `u16`. Same as `u16::from(value)` but `const`.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl PartialEq<u16> for VlqU16 {
    #[inline]
    fn eq(&self, other: &u16) -> bool {
        self.0 == *other
    }
}

impl PartialEq<VlqU16> for u16 {
    #[inline]
    fn eq(&self, other: &VlqU16) -> bool {
        *self == other.0
    }
}

impl Marshaler for VlqU16 {
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU16Marshaler.marshal(wb, self.0);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self(VlqU16Marshaler.unmarshal(rb)?))
    }
}

/// Quantizes a u32 into 1..=5 bytes using `GridMate`'s VLQ format.
#[derive(Debug, Clone, Copy, Default)]
pub struct VlqU32Marshaler;

impl VlqU32Marshaler {
    pub const MAX_ENCODING_BYTES: usize = 5;

    #[inline]
    pub fn marshal(&self, wb: &mut WriteBuffer, v: u32) {
        let mut data = [0u8; 5];
        if v < 0x80 {
            // `v < 0x80` on this branch, so the mask is an identity that also
            // makes the narrowing provably lossless.
            data[0] = (v & 0x7f) as u8;
            wb.write_bytes(&data[..1]);
        } else if v < 0x4000 {
            data[0] = 0x80 | (v & 0x3f) as u8;
            data[1] = ((v & 0x3fc0) >> 6) as u8;
            wb.write_bytes(&data[..2]);
        } else if v < 0x0020_0000 {
            data[0] = 0xc0 | (v & 0x1f) as u8;
            data[1] = ((v & 0x1fe0) >> 5) as u8;
            data[2] = ((v & 0x001f_e000) >> 13) as u8;
            wb.write_bytes(&data[..3]);
        } else if v < 0x1000_0000 {
            data[0] = 0xe0 | (v & 0x0f) as u8;
            data[1] = ((v & 0x0ff0) >> 4) as u8;
            data[2] = ((v & 0x000f_f000) >> 12) as u8;
            data[3] = ((v & 0x0ff0_0000) >> 20) as u8;
            wb.write_bytes(&data[..4]);
        } else {
            data[0] = 0xf0 | (v & 0x07) as u8;
            data[1] = ((v & 0x0000_07f8) >> 3) as u8;
            data[2] = ((v & 0x0007_f800) >> 11) as u8;
            data[3] = ((v & 0x07f8_0000) >> 19) as u8;
            data[4] = ((v & 0xf800_0000) >> 27) as u8;
            wb.write_bytes(&data[..5]);
        }
    }

    /// Decode a 1..=5-byte `GridMate` VLQ into a `u32`.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if the buffer ends before
    /// the continuation bits in the lead byte have been satisfied. The
    /// encoding is total over `u32`, so no other failure is reachable.
    #[inline]
    pub fn unmarshal(&self, rb: &mut ReadBuffer) -> Result<u32, MarshalerError> {
        let first = rb.read_u8()?;
        if first < 0x80 {
            Ok(u32::from(first))
        } else if first < 0xc0 {
            let b1 = rb.read_u8()?;
            let v = (u32::from(first & !0xc0)) | ((u32::from(b1)) << 6);
            Ok(v)
        } else if first < 0xe0 {
            let b1 = rb.read_u8()?;
            let b2 = rb.read_u8()?;
            let v = (u32::from(first & !0xe0)) | ((u32::from(b1)) << 5) | ((u32::from(b2)) << 13);
            Ok(v)
        } else if first < 0xf0 {
            let b1 = rb.read_u8()?;
            let b2 = rb.read_u8()?;
            let b3 = rb.read_u8()?;
            let v = (u32::from(first & !0xf0))
                | ((u32::from(b1)) << 4)
                | ((u32::from(b2)) << 12)
                | ((u32::from(b3)) << 20);
            Ok(v)
        } else {
            let b1 = rb.read_u8()?;
            let b2 = rb.read_u8()?;
            let b3 = rb.read_u8()?;
            let b4 = rb.read_u8()?;
            let v = (u32::from(first & !0xf8))
                | ((u32::from(b1)) << 3)
                | ((u32::from(b2)) << 11)
                | ((u32::from(b3)) << 19)
                | ((u32::from(b4)) << 27);
            Ok(v)
        }
    }
}

/// `VlqU32Marshaler` doubles as a [`Codec<u32>`] policy for fields that hold a
/// raw `u32` but encode it as a VLQ.
///
/// [`Codec<u32>`]: super::marshaler::Codec
impl super::marshaler::Codec<u32> for VlqU32Marshaler {
    fn marshal(value: &u32, wb: &mut WriteBuffer) {
        Self.marshal(wb, *value);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<u32, MarshalerError> {
        Self.unmarshal(rb)
    }
}

/// Newtype wrapper around `u32` that marshals as a 1..=5-byte `GridMate` VLQ.
///
/// This is the same encoding `Vec<T>::Marshaler` uses for its length prefix.
/// Use it *as the field type* — e.g. `pub seq: VlqU32` — whenever the wire
/// format is a standalone VLQ rather than the default 4-byte big-endian
/// `Marshaler<u32>` write.
///
/// `Deref`/`DerefMut` to `u32` plus a full set of native trait impls
/// (`From<u32>`, `From<VlqU32> for u32`, `PartialEq<u32>`, `Display`,
/// `AsRef<u32>`, `From<&VlqU32> for VlqU32`) let the wrapper drop into
/// existing formatting and conversion sites without ceremony — and
/// `*v` (deref) yields a `u32` for arithmetic. A field carrying a logical
/// `u32` only needs the type swap to gain the right wire shape.
///
/// ```
/// use gridmate::serialize::VlqU32;
/// let v: VlqU32 = 42_u32.into();
/// assert_eq!(*v, 42);                 // Deref to u32 for arithmetic
/// assert_eq!(*v + 1, 43);
/// assert_eq!(v, 42_u32);              // PartialEq<u32>
/// assert_eq!(format!("{v}"), "42");   // Display passthrough
/// let raw: u32 = v.into();            // From<VlqU32> for u32
/// assert_eq!(raw, 42);
/// ```
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    AsRef,
    Deref,
    DerefMut,
    Display,
    From,
    Into,
)]
pub struct VlqU32(pub u32);

impl VlqU32 {
    /// Construct from a raw `u32`. Same as `VlqU32::from(value)` but `const`.
    #[inline]
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Extract the inner `u32`. Same as `u32::from(value)` but `const`.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl PartialEq<u32> for VlqU32 {
    #[inline]
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}

impl PartialEq<VlqU32> for u32 {
    #[inline]
    fn eq(&self, other: &VlqU32) -> bool {
        *self == other.0
    }
}

impl Marshaler for VlqU32 {
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU32Marshaler.marshal(wb, self.0);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self(VlqU32Marshaler.unmarshal(rb)?))
    }
}

/// Quantizes a u64 into 1,2,3,4,5,6,7,8, or 9 bytes using `GridMate`'s VLQ format.
#[derive(Debug, Clone, Copy, Default)]
pub struct VlqU64Marshaler;

impl VlqU64Marshaler {
    /// Source `VlqU64Marshaler::MaxEncodingBytes`.
    pub const MAX_ENCODING_BYTES: usize = 9;

    #[inline]
    const fn byte_after_bits(v: u64, bits: u32) -> u8 {
        ((v >> bits) & 0xff) as u8
    }

    #[inline]
    pub fn marshal(&self, wb: &mut WriteBuffer, v: u64) {
        let mut data = [0u8; 9];
        if v < 0x80 {
            // `v < 0x80` on this branch, so the mask is an identity that also
            // makes the narrowing provably lossless.
            data[0] = (v & 0x7f) as u8;
            wb.write_bytes(&data[..1]);
        } else if v < 0x4000 {
            data[0] = 0x80 | (v & 0x3f) as u8;
            data[1] = Self::byte_after_bits(v, 6);
            wb.write_bytes(&data[..2]);
        } else if v < 0x0020_0000 {
            data[0] = 0xc0 | (v & 0x1f) as u8;
            data[1] = Self::byte_after_bits(v, 5);
            data[2] = Self::byte_after_bits(v, 13);
            wb.write_bytes(&data[..3]);
        } else if v < 0x1000_0000 {
            data[0] = 0xe0 | (v & 0x0f) as u8;
            data[1] = Self::byte_after_bits(v, 4);
            data[2] = Self::byte_after_bits(v, 12);
            data[3] = Self::byte_after_bits(v, 20);
            wb.write_bytes(&data[..4]);
        } else if v < 0x0000_0000_0800_0000 {
            data[0] = 0xf0 | (v & 0x07) as u8;
            data[1] = Self::byte_after_bits(v, 3);
            data[2] = Self::byte_after_bits(v, 11);
            data[3] = Self::byte_after_bits(v, 19);
            data[4] = Self::byte_after_bits(v, 27);
            wb.write_bytes(&data[..5]);
        } else if v < 0x0000_0400_0000_0000 {
            data[0] = 0xF8 | (v & 0x03) as u8;
            data[1] = Self::byte_after_bits(v, 2);
            data[2] = Self::byte_after_bits(v, 10);
            data[3] = Self::byte_after_bits(v, 18);
            data[4] = Self::byte_after_bits(v, 26);
            data[5] = Self::byte_after_bits(v, 34);
            wb.write_bytes(&data[..6]);
        } else if v < 0x0002_0000_0000_0000 {
            data[0] = 0xFC | (v & 0x01) as u8;
            data[1] = Self::byte_after_bits(v, 1);
            data[2] = Self::byte_after_bits(v, 9);
            data[3] = Self::byte_after_bits(v, 17);
            data[4] = Self::byte_after_bits(v, 25);
            data[5] = Self::byte_after_bits(v, 33);
            data[6] = Self::byte_after_bits(v, 41);
            wb.write_bytes(&data[..7]);
        } else if v < 0x0100_0000_0000_0000 {
            data[0] = 0xFE;
            data[1] = Self::byte_after_bits(v, 0);
            data[2] = Self::byte_after_bits(v, 8);
            data[3] = Self::byte_after_bits(v, 16);
            data[4] = Self::byte_after_bits(v, 24);
            data[5] = Self::byte_after_bits(v, 32);
            data[6] = Self::byte_after_bits(v, 40);
            data[7] = Self::byte_after_bits(v, 48);
            wb.write_bytes(&data[..8]);
        } else {
            data[0] = 0xFF;
            data[1] = Self::byte_after_bits(v, 0);
            data[2] = Self::byte_after_bits(v, 8);
            data[3] = Self::byte_after_bits(v, 16);
            data[4] = Self::byte_after_bits(v, 24);
            data[5] = Self::byte_after_bits(v, 32);
            data[6] = Self::byte_after_bits(v, 40);
            data[7] = Self::byte_after_bits(v, 48);
            data[8] = Self::byte_after_bits(v, 56);
            wb.write_bytes(&data[..9]);
        }
    }

    /// Decode a 1..=9-byte `GridMate` VLQ into a `u64`.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if the buffer ends before
    /// the continuation bits in the lead byte have been satisfied. The
    /// encoding is total over `u64`, so no other failure is reachable.
    #[inline]
    pub fn unmarshal(&self, rb: &mut ReadBuffer) -> Result<u64, MarshalerError> {
        let first = rb.read_u8()?;
        if first < 0x80 {
            Ok(u64::from(first))
        } else if first < 0xc0 {
            let b1 = rb.read_u8()?;
            Ok((u64::from(first & !0xc0)) | ((u64::from(b1)) << 6))
        } else if first < 0xe0 {
            let b1 = rb.read_u8()?;
            let b2 = rb.read_u8()?;
            Ok((u64::from(first & !0xe0)) | ((u64::from(b1)) << 5) | ((u64::from(b2)) << 13))
        } else if first < 0xf0 {
            let b1 = rb.read_u8()?;
            let b2 = rb.read_u8()?;
            let b3 = rb.read_u8()?;
            Ok((u64::from(first & !0xf0))
                | ((u64::from(b1)) << 4)
                | ((u64::from(b2)) << 12)
                | ((u64::from(b3)) << 20))
        } else if first < 0xF8 {
            let b1 = rb.read_u8()?;
            let b2 = rb.read_u8()?;
            let b3 = rb.read_u8()?;
            let b4 = rb.read_u8()?;
            Ok((u64::from(first & !0xf8))
                | ((u64::from(b1)) << 3)
                | ((u64::from(b2)) << 11)
                | ((u64::from(b3)) << 19)
                | ((u64::from(b4)) << 27))
        } else if first < 0xFC {
            let b1 = rb.read_u8()?;
            let b2 = rb.read_u8()?;
            let b3 = rb.read_u8()?;
            let b4 = rb.read_u8()?;
            let b5 = rb.read_u8()?;
            Ok((u64::from(first & !0xFC))
                | ((u64::from(b1)) << 2)
                | ((u64::from(b2)) << 10)
                | ((u64::from(b3)) << 18)
                | ((u64::from(b4)) << 26)
                | ((u64::from(b5)) << 34))
        } else if first < 0xFE {
            let b1 = rb.read_u8()?;
            let b2 = rb.read_u8()?;
            let b3 = rb.read_u8()?;
            let b4 = rb.read_u8()?;
            let b5 = rb.read_u8()?;
            let b6 = rb.read_u8()?;
            Ok((u64::from(first & !0xFE))
                | ((u64::from(b1)) << 1)
                | ((u64::from(b2)) << 9)
                | ((u64::from(b3)) << 17)
                | ((u64::from(b4)) << 25)
                | ((u64::from(b5)) << 33)
                | ((u64::from(b6)) << 41))
        } else if first < 0xFF {
            let b1 = rb.read_u8()?;
            let b2 = rb.read_u8()?;
            let b3 = rb.read_u8()?;
            let b4 = rb.read_u8()?;
            let b5 = rb.read_u8()?;
            let b6 = rb.read_u8()?;
            let b7 = rb.read_u8()?;
            Ok(u64::from(b1)
                | ((u64::from(b2)) << 8)
                | ((u64::from(b3)) << 16)
                | ((u64::from(b4)) << 24)
                | ((u64::from(b5)) << 32)
                | ((u64::from(b6)) << 40)
                | ((u64::from(b7)) << 48))
        } else {
            let b1 = rb.read_u8()?;
            let b2 = rb.read_u8()?;
            let b3 = rb.read_u8()?;
            let b4 = rb.read_u8()?;
            let b5 = rb.read_u8()?;
            let b6 = rb.read_u8()?;
            let b7 = rb.read_u8()?;
            let b8 = rb.read_u8()?;
            Ok(u64::from(b1)
                | ((u64::from(b2)) << 8)
                | ((u64::from(b3)) << 16)
                | ((u64::from(b4)) << 24)
                | ((u64::from(b5)) << 32)
                | ((u64::from(b6)) << 40)
                | ((u64::from(b7)) << 48)
                | ((u64::from(b8)) << 56))
        }
    }
}

/// Newtype wrapper around `u64` that marshals as a 1..=9-byte `GridMate` VLQ.
///
/// Use as the field type when the wire format is a standalone VLQ-u64 (e.g.
/// revision tags consumed mid-struct, sequence stamps) rather than the default
/// 8-byte big-endian `Marshaler<u64>` write.
///
/// # Composing with `Option<T>`
///
/// `Option<VlqU64>` expresses the concrete wire shape `[u8 has][opt VLQ u64]`
/// when a native helper emits a presence-flagged sequence or revision tag.
/// `Option<T>: Marshaler` writes the strict-bool `0`/`1` prefix and rejects
/// `> 1` on read; `VlqU64` supplies the VLQ-u64 payload, so no dedicated helper
/// type is needed:
///
/// ```ignore
/// pub revision: Option<VlqU64>,   // wire: [u8 has][opt VLQ u64]
/// ```
///
/// # Ergonomics
///
/// Mirrors [`VlqU32`]'s ergonomics: `Deref<Target = u64>`, `From<u64>`,
/// `From<VlqU64> for u64`, `PartialEq<u64>`, `Display`, and `AsRef<u64>` so
/// the wrapper drops into existing arithmetic and formatting sites without
/// ceremony. This is only a scalar encoding building block; higher-level
/// framing lives in the message or container type that owns it.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    AsRef,
    Deref,
    DerefMut,
    Display,
    From,
    Into,
)]
pub struct VlqU64(pub u64);

impl VlqU64 {
    /// Construct from a raw `u64`. Same as `VlqU64::from(value)` but `const`.
    #[inline]
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Extract the inner `u64`. Same as `u64::from(value)` but `const`.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl PartialEq<u64> for VlqU64 {
    #[inline]
    fn eq(&self, other: &u64) -> bool {
        self.0 == *other
    }
}

impl PartialEq<VlqU64> for u64 {
    #[inline]
    fn eq(&self, other: &VlqU64) -> bool {
        *self == other.0
    }
}

impl Marshaler for VlqU64 {
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU64Marshaler.marshal(wb, self.0);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self(VlqU64Marshaler.unmarshal(rb)?))
    }
}

#[cfg(test)]
mod wrapper_tests {
    use super::*;
    use crate::serialize::buffer::CARRIER_ENDIAN;

    /// `VlqU16` round-trips identically to `VlqU16Marshaler` itself, and
    /// rejects values that don't fit in u16 on read.
    #[test]
    fn vlq_u16_wrapper_round_trip_matches_codec() {
        for &v in &[0u16, 0x7f, 0x80, 0x3fff, 0x4000, u16::MAX] {
            let mut wb_codec = WriteBuffer::new(CARRIER_ENDIAN);
            VlqU16Marshaler.marshal(&mut wb_codec, v);

            let mut wb_wrapper = WriteBuffer::new(CARRIER_ENDIAN);
            VlqU16(v).marshal(&mut wb_wrapper);

            let codec_bytes = wb_codec.into_vec();
            let wrapper_bytes = wb_wrapper.into_vec();
            assert_eq!(codec_bytes, wrapper_bytes, "byte mismatch for {v}");

            let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &wrapper_bytes);
            let decoded = VlqU16::unmarshal(&mut rb).unwrap();
            assert_eq!(decoded.get(), v);
            assert_eq!(rb.left(), 0);
        }
    }

    #[test]
    fn vlq_u16_unmarshal_rejects_overflow() {
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        VlqU32Marshaler.marshal(&mut wb, u32::from(u16::MAX) + 1);
        let bytes = wb.into_vec();
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        match VlqU16Marshaler.unmarshal(&mut rb) {
            Err(MarshalerError::ContainerOverflow { .. }) => {}
            other => panic!("expected ContainerOverflow, got {other:?}"),
        }
    }

    /// `VlqU32` round-trips identically to `VlqU32Marshaler` itself. The
    /// wrapper is supposed to be a pure ergonomic shell over the existing
    /// codec, so the byte streams must match exactly.
    #[test]
    fn vlq_u32_wrapper_round_trip_matches_codec() {
        for &v in &[
            0u32,
            0x7f,
            0x80,
            0x3fff,
            0x4000,
            0x001f_ffff,
            0x0020_0000,
            u32::MAX,
        ] {
            let mut wb_codec = WriteBuffer::new(CARRIER_ENDIAN);
            VlqU32Marshaler.marshal(&mut wb_codec, v);

            let mut wb_wrapper = WriteBuffer::new(CARRIER_ENDIAN);
            VlqU32(v).marshal(&mut wb_wrapper);

            let codec_bytes = wb_codec.into_vec();
            let wrapper_bytes = wb_wrapper.into_vec();
            assert_eq!(
                codec_bytes, wrapper_bytes,
                "wrapper must emit the same bytes as the codec for {v}"
            );

            let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &wrapper_bytes);
            let decoded = VlqU32::unmarshal(&mut rb).unwrap();
            assert_eq!(decoded.get(), v);
            assert_eq!(
                rb.left(),
                0,
                "wrapper unmarshal must consume exactly the encoded bytes"
            );
        }
    }

    /// `VlqU64` round-trips identically to `VlqU64Marshaler` itself.
    #[test]
    fn vlq_u64_wrapper_round_trip_matches_codec() {
        for &v in &[
            0u64,
            0x7f,
            0x80,
            0x3fff,
            0x4000,
            0x001f_ffff,
            0x1000_0000,
            0x0800_0000,
            0x0400_0000_0000,
            0x0002_0000_0000_0000,
            0x0100_0000_0000_0000,
            u64::MAX,
        ] {
            let mut wb_codec = WriteBuffer::new(CARRIER_ENDIAN);
            VlqU64Marshaler.marshal(&mut wb_codec, v);

            let mut wb_wrapper = WriteBuffer::new(CARRIER_ENDIAN);
            VlqU64(v).marshal(&mut wb_wrapper);

            let codec_bytes = wb_codec.into_vec();
            let wrapper_bytes = wb_wrapper.into_vec();
            assert_eq!(codec_bytes, wrapper_bytes, "byte mismatch for {v}");

            let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &wrapper_bytes);
            let decoded = VlqU64::unmarshal(&mut rb).unwrap();
            assert_eq!(decoded.get(), v);
            assert_eq!(rb.left(), 0);
        }
    }

    /// Native trait coverage: `From`/`Into`, `Deref`, `PartialEq` against the
    /// inner integer, and `Display`. These are what makes the wrapper usable
    /// as a drop-in field type.
    #[test]
    fn vlq_u32_native_traits() {
        const C: VlqU32 = VlqU32::new(7);

        let v: VlqU32 = 42_u32.into();
        // Deref into u32 enables arithmetic and indexing.
        assert_eq!(*v, 42);
        // PartialEq against u32 (both directions).
        assert_eq!(v, 42_u32);
        assert_eq!(42_u32, v);
        // Into<u32> via the symmetric From impl.
        let raw: u32 = v.into();
        assert_eq!(raw, 42);
        // Display passes through to the underlying integer formatter.
        assert_eq!(format!("{v}"), "42");
        // Const constructor.
        assert_eq!(C.get(), 7);
    }

    #[test]
    fn vlq_u64_native_traits() {
        const C: VlqU64 = VlqU64::new(7);

        let v: VlqU64 = 0xDEAD_BEEF_u64.into();
        assert_eq!(*v, 0xDEAD_BEEF);
        assert_eq!(v, 0xDEAD_BEEF_u64);
        assert_eq!(0xDEAD_BEEF_u64, v);
        let raw: u64 = v.into();
        assert_eq!(raw, 0xDEAD_BEEF);
        assert_eq!(format!("{v}"), "3735928559");
        assert_eq!(C.get(), 7);
    }
}
