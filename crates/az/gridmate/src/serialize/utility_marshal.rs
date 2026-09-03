// Utility marshalers: UUID as 16 bytes; Duration as u32 milliseconds

use super::{
    buffer::{ReadBuffer, WriteBuffer},
    error::MarshalerError,
    marshaler::Marshaler,
};
use uuid::Uuid;

impl Marshaler for az_core::Uid {
    const MARSHAL_SIZE: usize = Self::BYTE_LEN;

    fn marshal(&self, wb: &mut WriteBuffer) {
        self.as_uuid().marshal(wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self::new(Uuid::unmarshal(rb)?))
    }
}

// UUID marshaled as 16 raw bytes
impl Marshaler for Uuid {
    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_bytes(self.as_bytes());
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let bytes = rb.read_bytes(16)?;
        Ok(Self::from_slice(bytes)?)
    }
}

/// Value marshaled in host byte order.
///
/// Native-endian fields are rare protocol outliers. Most scalar fields use
/// carrier byte order through their normal [`Marshaler`] implementations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, derive_more::From)]
pub struct NativeEndian<T>(pub T);

impl Marshaler for NativeEndian<u64> {
    const MARSHAL_SIZE: usize = 8;

    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_bytes(&self.0.to_ne_bytes());
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let bytes = rb.read_bytes(8)?;
        let mut raw = [0u8; 8];
        raw.copy_from_slice(bytes);
        Ok(Self(u64::from_ne_bytes(raw)))
    }
}

/// Base-state revision emitted as eight native-endian bytes.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, derive_more::From, derive_more::Into,
)]
pub struct RawU64Revision(pub u64);

impl Marshaler for RawU64Revision {
    const MARSHAL_SIZE: usize = 8;

    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_bytes(&self.0.to_ne_bytes());
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        NativeEndian::<u64>::unmarshal(rb).map(|value| Self(value.0))
    }
}

/// 16-bit half-precision float wire encoding for an `f32` payload.
///
/// **C++ analog:** `GridMate::HalfMarshaler` — emits the IEEE 754 binary16
/// bit pattern as a raw `u16` for fields where a full 32-bit IEEE 754 single
/// is unnecessary.
///
/// Use at field sites via `#[marshal(as = "HalfF32")]`; the field stays
/// typed as `f32` and round-trips through `From<f32>` / `From<HalfF32>`.
/// Wire shape is 2 raw bytes (carrier-endian, like every other integer in
/// this crate).
#[derive(Debug, Clone, Copy, Default, PartialEq, derive_more::From, derive_more::Into)]
pub struct HalfF32(pub f32);

impl Marshaler for HalfF32 {
    const MARSHAL_SIZE: usize = 2;

    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_u16(half::f16::from_f32(self.0).to_bits());
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let bits = rb.read_u16()?;
        Ok(Self(half::f16::from_bits(bits).to_f32()))
    }
}

// std::time::Duration as u32 milliseconds
impl Marshaler for std::time::Duration {
    const MARSHAL_SIZE: usize = <u32 as Marshaler>::MARSHAL_SIZE;

    fn marshal(&self, wb: &mut WriteBuffer) {
        // Saturates at `u32::MAX` ms (~49.7 days) exactly as the previous
        // `min(u32::MAX) as u32` did, without an unchecked narrowing.
        let ms = u32::try_from(self.as_millis()).unwrap_or(u32::MAX);
        ms.marshal(wb);
    }
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let ms = u32::unmarshal(rb)?;
        Ok(Self::from_millis(u64::from(ms)))
    }
}

/// Boxed marshaler — wire shape is identical to the inner `T`.
///
/// Lets recursive types compose with the existing derive without a hand-written
/// implementation. `T` is sized because [`Marshaler::unmarshal`] returns it by
/// value.
impl<T: Marshaler> Marshaler for Box<T> {
    const MARSHAL_SIZE: usize = T::MARSHAL_SIZE;

    fn marshal(&self, wb: &mut WriteBuffer) {
        (**self).marshal(wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self::new(T::unmarshal(rb)?))
    }
}

/// Lumberyard `Marshaler<AZStd::bitset<N>>` — raw machine-words on the wire.
///
/// Emits `value.num_words()` words via `wb.Write(data[i])`. On x64 the default
/// word size is `AZ::u64`, so the wire shape is `ceil(N / 64) * 8`
/// carrier-endian bytes.
///
/// Const-generic over the **word count** (`WORDS`), not the bit count,
/// because stable Rust can't yet compute `WORDS = (BITS + 63) / 64` at
/// trait-impl scope (requires `feature(generic_const_exprs)`). Pick
/// `WORDS = ceil(BITS / 64)` at the call site — e.g. `BitSet<1>` for a
/// `bitset<64>`, `BitSet<2>` for `bitset<128>`, etc.
///
/// Use this type for a literal C++ `AZStd::bitset<N>` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitSet<const WORDS: usize>(pub [u64; WORDS]);

impl<const WORDS: usize> Default for BitSet<WORDS> {
    fn default() -> Self {
        Self([0; WORDS])
    }
}

impl<const WORDS: usize> BitSet<WORDS> {
    /// Total bit capacity (`WORDS * 64`).
    pub const BITS: usize = WORDS * 64;

    /// Read bit `index` (LSB-first within each word).
    ///
    /// Out-of-range indices return `false` — matches `AZStd::bitset`'s
    /// per-bit accessor (`operator[]` is bounds-checked in debug only;
    /// our impl defaults to "absent" for safety).
    #[must_use]
    pub const fn get(&self, index: usize) -> bool {
        let word_idx = index / 64;
        let bit_idx = index % 64;
        word_idx < WORDS && (self.0[word_idx] & (1u64 << bit_idx)) != 0
    }

    /// Set bit `index` to `value`. No-op if `index >= WORDS * 64`.
    pub const fn set(&mut self, index: usize, value: bool) {
        let word_idx = index / 64;
        let bit_idx = index % 64;
        if word_idx >= WORDS {
            return;
        }
        if value {
            self.0[word_idx] |= 1u64 << bit_idx;
        } else {
            self.0[word_idx] &= !(1u64 << bit_idx);
        }
    }

    /// Count of set bits.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.0.iter().map(|w| w.count_ones()).sum()
    }
}

impl<const WORDS: usize> Marshaler for BitSet<WORDS> {
    fn marshal(&self, wb: &mut WriteBuffer) {
        for &word in &self.0 {
            word.marshal(wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let mut words = [0u64; WORDS];
        for word in &mut words {
            *word = u64::unmarshal(rb)?;
        }
        Ok(Self(words))
    }
}

// Generic Option<T> marshaler
// Wire format: 0x00 = None, 0x01 = Some(T)
// If Some, follows with T's marshaled representation
impl<T: Marshaler> Marshaler for Option<T> {
    fn marshal(&self, wb: &mut WriteBuffer) {
        match self {
            Some(inner) => {
                1u8.marshal(wb); // 0x01 = Some
                inner.marshal(wb);
            }
            None => {
                0u8.marshal(wb); // 0x00 = None
            }
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let discriminant = u8::unmarshal(rb)?;
        match discriminant {
            0 => Ok(None),
            1 => Ok(Some(T::unmarshal(rb)?)),
            other => Err(MarshalerError::InvalidDiscriminant { value: other }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::buffer::CARRIER_ENDIAN;

    #[test]
    fn bitset_one_word_round_trip_carrier_endian() {
        let mut bs = BitSet::<1>::default();
        bs.set(0, true);
        bs.set(7, true);
        bs.set(63, true);
        assert_eq!(bs.count(), 3);

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        bs.marshal(&mut wb);
        let bytes = wb.into_vec();
        assert_eq!(bytes.len(), 8);
        // Word value: bit 0 + bit 7 + bit 63 = 0x8000_0000_0000_0081.
        // Carrier endian = big-endian, so MSB first.
        assert_eq!(bytes, [0x80, 0, 0, 0, 0, 0, 0, 0x81]);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let decoded = BitSet::<1>::unmarshal(&mut rb).unwrap();
        assert_eq!(decoded, bs);
    }

    #[test]
    fn bitset_two_words_emits_16_bytes() {
        let mut bs = BitSet::<2>::default();
        bs.set(64, true);
        bs.set(127, true);

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        bs.marshal(&mut wb);
        let bytes = wb.into_vec();
        assert_eq!(bytes.len(), 16);
        // First u64 = 0 (no bits 0..63 set), second u64 = bit 0 + bit 63
        // → 0x8000_0000_0000_0001 BE.
        assert_eq!(&bytes[0..8], &[0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&bytes[8..16], &[0x80, 0, 0, 0, 0, 0, 0, 0x01]);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let decoded = BitSet::<2>::unmarshal(&mut rb).unwrap();
        assert!(decoded.get(64));
        assert!(decoded.get(127));
        assert!(!decoded.get(0));
    }

    #[test]
    fn bitset_out_of_range_set_is_noop() {
        let mut bs = BitSet::<1>::default();
        bs.set(1000, true);
        assert_eq!(bs.count(), 0);
        assert!(!bs.get(1000));
    }
}
