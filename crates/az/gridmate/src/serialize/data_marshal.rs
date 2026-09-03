// DataMarshal equivalents (fundamental types, bool)

use super::{
    buffer::{ReadBuffer, WriteBuffer},
    container_marshal::{WIRE_VEC_CAP, wire_len},
    error::MarshalerError,
    marshaler::{Codec, Marshaler},
    vlq::VlqU32Marshaler,
};
use az_core::component::{
    Component as AzComponent, ComponentId as AzComponentId, EntityId as AzEntityId,
};
use az_core::crc::Crc32 as AzCrc32;
use az_core::entity::LocalEntityRef;
use std::marker::PhantomData;

impl Marshaler for u8 {
    const MARSHAL_SIZE: usize = 1;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_u8(*self);
    }
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        rb.read_u8()
    }
}

impl Marshaler for i8 {
    const MARSHAL_SIZE: usize = 1;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_u8(self.cast_unsigned());
    }
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(rb.read_u8()?.cast_signed())
    }
}

impl Marshaler for u16 {
    const MARSHAL_SIZE: usize = 2;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_u16(*self);
    }
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        rb.read_u16()
    }
}

impl Marshaler for i16 {
    const MARSHAL_SIZE: usize = 2;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_u16(self.cast_unsigned());
    }
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(rb.read_u16()?.cast_signed())
    }
}

impl Marshaler for u32 {
    const MARSHAL_SIZE: usize = 4;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_u32(*self);
    }
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let value = rb.read_u32()?;
        Ok(value)
    }
}

impl Marshaler for i32 {
    const MARSHAL_SIZE: usize = 4;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_u32(self.cast_unsigned());
    }
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(rb.read_u32()?.cast_signed())
    }
}

impl Marshaler for f32 {
    const MARSHAL_SIZE: usize = 4;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.to_bits().marshal(wb);
    }
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self::from_bits(u32::unmarshal(rb)?))
    }
}

impl Marshaler for u64 {
    const MARSHAL_SIZE: usize = 8;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        let bytes = match wb.endian() {
            super::buffer::Endian::BigEndian => self.to_be_bytes(),
            super::buffer::Endian::LittleEndian => self.to_le_bytes(),
        };
        wb.write_bytes(&bytes);
    }
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let bytes = rb.read_bytes(8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(bytes);
        Ok(match rb.endian() {
            super::buffer::Endian::BigEndian => Self::from_be_bytes(arr),
            super::buffer::Endian::LittleEndian => Self::from_le_bytes(arr),
        })
    }
}

impl Marshaler for AzEntityId {
    const MARSHAL_SIZE: usize = <u64 as Marshaler>::MARSHAL_SIZE;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.value().marshal(wb);
    }

    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self::new(u64::unmarshal(rb)?))
    }
}

impl Marshaler for LocalEntityRef {
    const MARSHAL_SIZE: usize = <AzEntityId as Marshaler>::MARSHAL_SIZE;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.entity_id.marshal(wb);
    }

    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self::new(AzEntityId::unmarshal(rb)?))
    }
}

impl Marshaler for AzComponentId {
    const MARSHAL_SIZE: usize = <u64 as Marshaler>::MARSHAL_SIZE;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.value().marshal(wb);
    }

    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self::new(u64::unmarshal(rb)?))
    }
}

impl Marshaler for AzComponent {
    const MARSHAL_SIZE: usize = <AzComponentId as Marshaler>::MARSHAL_SIZE;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.id.marshal(wb);
    }

    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self {
            id: AzComponentId::unmarshal(rb)?,
        })
    }
}

impl Marshaler for i64 {
    const MARSHAL_SIZE: usize = 8;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.cast_unsigned().marshal(wb);
    }
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(u64::unmarshal(rb)?.cast_signed())
    }
}

impl Marshaler for f64 {
    const MARSHAL_SIZE: usize = 8;

    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.to_bits().marshal(wb);
    }
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self::from_bits(u64::unmarshal(rb)?))
    }
}

impl Marshaler for bool {
    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_raw_bit(*self);
    }
    /// Source `Marshaler<bool>` delegates to `ReadRawBit`.
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        rb.read_raw_bit()
    }
}

/// Fallible conversion contract for values encoded through a different wire type.
///
/// The conversion belongs to the semantic value type, while
/// [`ConversionMarshaler`] remains the reusable `GridMate` field codec. This
/// keeps compact enum and identifier encodings typed without accepting every
/// bit pattern of the serialized primitive.
pub trait MarshalerConversion<SerializedType>: Copy {
    /// Project this value onto the primitive that actually goes on the wire.
    fn to_serialized(self) -> SerializedType;

    /// Rebuild the value from the wire primitive, rejecting bit patterns that
    /// are not in this type's domain.
    ///
    /// # Errors
    ///
    /// Returns the error the implementation raises for an out-of-domain
    /// primitive — [`MarshalerError::InvalidDiscriminant`] for a compact enum
    /// tag outside the declared set, [`MarshalerError::InvalidRange`] for a
    /// bounded numeric identifier. Implementations whose domain is the whole
    /// primitive (`Crc32`, for instance) never fail.
    fn try_from_serialized(value: SerializedType) -> Result<Self, MarshalerError>;
}

/// Source-shaped `GridMate::ConversionMarshaler<SerializedType, OriginalType>`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConversionMarshaler<SerializedType, OriginalType>(
    PhantomData<fn() -> (SerializedType, OriginalType)>,
);

impl<SerializedType, OriginalType> Codec<OriginalType>
    for ConversionMarshaler<SerializedType, OriginalType>
where
    SerializedType: Marshaler,
    OriginalType: MarshalerConversion<SerializedType>,
{
    const MARSHAL_SIZE: usize = SerializedType::MARSHAL_SIZE;

    fn marshal(value: &OriginalType, wb: &mut WriteBuffer) {
        value.to_serialized().marshal(wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<OriginalType, MarshalerError> {
        OriginalType::try_from_serialized(SerializedType::unmarshal(rb)?)
    }
}

impl MarshalerConversion<u32> for AzCrc32 {
    #[inline]
    fn to_serialized(self) -> u32 {
        self.value()
    }

    #[inline]
    fn try_from_serialized(value: u32) -> Result<Self, MarshalerError> {
        Ok(Self::from_u32(value))
    }
}

/// Source `Marshaler<AZ::Crc32>`: a Crc32 is carried as one raw `AZ::u32`.
impl Marshaler for AzCrc32 {
    const MARSHAL_SIZE: usize = <u32 as Marshaler>::MARSHAL_SIZE;

    fn marshal(&self, wb: &mut WriteBuffer) {
        ConversionMarshaler::<u32, Self>::marshal(self, wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        ConversionMarshaler::<u32, Self>::unmarshal(rb)
    }
}

impl Marshaler for String {
    #[inline]
    fn marshal(&self, wb: &mut WriteBuffer) {
        let bytes = self.as_bytes();
        VlqU32Marshaler.marshal(wb, wire_len(bytes.len()));
        wb.write_bytes(bytes);
    }
    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let len = VlqU32Marshaler.unmarshal(rb)? as usize;
        if len > WIRE_VEC_CAP {
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: WIRE_VEC_CAP,
            });
        }
        let bytes = rb.read_bytes(len)?;
        Ok(std::str::from_utf8(bytes)?.to_string())
    }
}

/// Marker for source `IsFundamentalMarshalType`.
///
/// Rust exposes the positive case as a trait bound rather than a SFINAE false
/// branch. It covers the numeric source set: floats plus signed/unsigned
/// fixed-width integers. `bool` is intentionally excluded, matching source.
pub trait FundamentalMarshalType: Marshaler {}

impl FundamentalMarshalType for f32 {}
impl FundamentalMarshalType for f64 {}
impl FundamentalMarshalType for u8 {}
impl FundamentalMarshalType for u16 {}
impl FundamentalMarshalType for u32 {}
impl FundamentalMarshalType for u64 {}
impl FundamentalMarshalType for i8 {}
impl FundamentalMarshalType for i16 {}
impl FundamentalMarshalType for i32 {}
impl FundamentalMarshalType for i64 {}

pub struct IsFundamentalMarshalType<T>(PhantomData<fn() -> T>);

impl<T: FundamentalMarshalType> IsFundamentalMarshalType<T> {
    pub const VALUE: bool = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::buffer::{CARRIER_ENDIAN, ReadBuffer, WriteBuffer};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CompactState {
        Ready,
    }

    impl MarshalerConversion<u8> for CompactState {
        fn to_serialized(self) -> u8 {
            match self {
                Self::Ready => 1,
            }
        }

        fn try_from_serialized(value: u8) -> Result<Self, MarshalerError> {
            match value {
                1 => Ok(Self::Ready),
                value => Err(MarshalerError::InvalidDiscriminant { value }),
            }
        }
    }

    fn roundtrip<T: Marshaler + PartialEq + std::fmt::Debug>(value: &T) -> T {
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        value.marshal(&mut wb);
        let data = wb.into_vec();
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &data);
        T::unmarshal(&mut rb).expect("unmarshal should succeed")
    }

    #[test]
    fn test_u8_roundtrip() {
        assert_eq!(roundtrip(&0u8), 0u8);
        assert_eq!(roundtrip(&127u8), 127u8);
        assert_eq!(roundtrip(&255u8), 255u8);
    }

    #[test]
    fn test_u16_roundtrip() {
        assert_eq!(roundtrip(&0u16), 0u16);
        assert_eq!(roundtrip(&1234u16), 1234u16);
        assert_eq!(roundtrip(&u16::MAX), u16::MAX);
    }

    #[test]
    fn test_u32_roundtrip() {
        assert_eq!(roundtrip(&0u32), 0u32);
        assert_eq!(roundtrip(&123_456_u32), 123_456_u32);
        assert_eq!(roundtrip(&u32::MAX), u32::MAX);
    }

    #[test]
    fn test_u64_roundtrip() {
        assert_eq!(roundtrip(&0u64), 0u64);
        assert_eq!(roundtrip(&123_456_789_012_345_u64), 123_456_789_012_345_u64);
        assert_eq!(roundtrip(&u64::MAX), u64::MAX);
    }

    #[test]
    fn test_az_component_roundtrip_uses_component_id_wire_shape() {
        let component = AzComponent {
            id: AzComponentId::new(0x0123_4567_89ab_cdef),
        };

        assert_eq!(roundtrip(&component), component);

        let mut component_bytes = WriteBuffer::new(CARRIER_ENDIAN);
        component.marshal(&mut component_bytes);
        let mut id_bytes = WriteBuffer::new(CARRIER_ENDIAN);
        component.id.marshal(&mut id_bytes);
        assert_eq!(component_bytes.into_vec(), id_bytes.into_vec());
    }

    #[test]
    fn test_f32_roundtrip() {
        // Compare bit patterns: the wire form *is* `to_bits` / `from_bits`, so
        // the round-trip is exact by construction. An epsilon would weaken the
        // assertion; comparing bits also distinguishes `0.0` from `-0.0`.
        for value in [0.0f32, std::f32::consts::PI, -1.5f32] {
            assert_eq!(roundtrip(&value).to_bits(), value.to_bits());
        }
    }

    #[test]
    fn test_f64_roundtrip() {
        // Bit-pattern comparison for the same reason as `test_f32_roundtrip`.
        for value in [0.0f64, std::f64::consts::PI, -1.5f64, f64::MIN, f64::MAX] {
            assert_eq!(roundtrip(&value).to_bits(), value.to_bits());
        }
    }

    /// Wire form of `f64` is the IEEE-754 64-bit bit pattern in carrier
    /// endian — i.e. the same 8 bytes that `u64::marshal` would emit for
    /// `f64::to_bits()`. Locks that in.
    #[test]
    fn test_f64_wire_matches_u64_bits() {
        let value = 1.5f64;
        let mut wb1 = WriteBuffer::new(CARRIER_ENDIAN);
        value.marshal(&mut wb1);
        let mut wb2 = WriteBuffer::new(CARRIER_ENDIAN);
        value.to_bits().marshal(&mut wb2);
        assert_eq!(wb1.into_vec(), wb2.into_vec());
    }

    #[test]
    fn test_bool_roundtrip() {
        assert!(roundtrip(&true));
        assert!(!roundtrip(&false));
    }

    #[test]
    fn conversion_marshaler_preserves_the_wire_width_and_rejects_invalid_values() {
        type CompactStateByte = ConversionMarshaler<u8, CompactState>;

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        CompactStateByte::marshal(&CompactState::Ready, &mut wb);
        assert_eq!(wb.as_slice(), &[1]);

        let mut valid = ReadBuffer::new(CARRIER_ENDIAN, &[1]);
        assert_eq!(
            CompactStateByte::unmarshal(&mut valid).unwrap(),
            CompactState::Ready
        );

        let mut invalid = ReadBuffer::new(CARRIER_ENDIAN, &[0xff]);
        assert!(matches!(
            CompactStateByte::unmarshal(&mut invalid),
            Err(MarshalerError::InvalidDiscriminant { value: 0xff })
        ));
    }

    #[test]
    fn test_string_roundtrip() {
        assert_eq!(roundtrip(&String::new()), String::new());
        assert_eq!(roundtrip(&"hello".to_string()), "hello".to_string());
        assert_eq!(
            roundtrip(&"Hello, World! 🌍".to_string()),
            "Hello, World! 🌍".to_string()
        );

        // Test longer string
        let long_string = "a".repeat(1000);
        assert_eq!(roundtrip(&long_string), long_string);
    }

    #[test]
    fn test_string_marshal_bytes() {
        // Verify the string bytes are actually written (not just length 0)
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        "test".to_string().marshal(&mut wb);
        let data = wb.into_vec();

        assert_eq!(data.len(), 5);
        assert_eq!(data[0], 4);
        assert_eq!(&data[1..], b"test");
    }
}
