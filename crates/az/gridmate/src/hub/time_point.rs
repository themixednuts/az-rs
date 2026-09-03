//! `ManifoldBinding` monotonic time measured from server start.

use az_derive::AzRtti;
use bevy_reflect::Reflect;

use crate::Marshaler;

/// `MB::TimePoint` (`0989A3E9-37E8-4381-8766-B208F460C1A3`).
///
/// This is a protocol and reflected-value type, distinct from wall-clock time.
/// Its native payload is one unsigned nanosecond count measured from server
/// start.
#[repr(transparent)]
#[derive(
    AzRtti,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Marshaler,
    serde::Serialize,
    serde::Deserialize,
    Reflect,
)]
#[az_rtti("0989A3E9-37E8-4381-8766-B208F460C1A3")]
pub struct TimePoint {
    #[serde(rename = "m_nanosecondsSinceServerStart", default)]
    nanoseconds_since_server_start: u64,
}

impl TimePoint {
    #[inline]
    #[must_use]
    pub const fn from_nanoseconds(nanoseconds_since_server_start: u64) -> Self {
        Self {
            nanoseconds_since_server_start,
        }
    }

    #[inline]
    #[must_use]
    pub const fn as_nanoseconds(self) -> u64 {
        self.nanoseconds_since_server_start
    }
}

impl From<u64> for TimePoint {
    fn from(value: u64) -> Self {
        Self::from_nanoseconds(value)
    }
}

impl From<TimePoint> for u64 {
    fn from(value: TimePoint) -> Self {
        value.as_nanoseconds()
    }
}

#[cfg(test)]
mod tests {
    use az_core::type_info::AzTypeInfo;
    use uuid::uuid;

    use super::*;
    use crate::serialize::Marshaler;
    use crate::serialize::buffer::{CARRIER_ENDIAN, ReadBuffer, WriteBuffer};

    #[test]
    fn identity_and_wire_shape_match_native() {
        assert_eq!(
            <TimePoint as AzTypeInfo>::TYPE_ID,
            uuid!("0989A3E9-37E8-4381-8766-B208F460C1A3")
        );

        let value = TimePoint::from_nanoseconds(0x0102_0304_0506_0708);
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        value.marshal(&mut wb);
        assert_eq!(wb.into_vec(), 0x0102_0304_0506_0708_u64.to_be_bytes());

        let bytes = 0x1112_1314_1516_1718_u64.to_be_bytes();
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let decoded = TimePoint::unmarshal(&mut rb).expect("decode TimePoint");
        assert_eq!(decoded.as_nanoseconds(), 0x1112_1314_1516_1718);
        assert_eq!(rb.left(), 0);
    }
}
