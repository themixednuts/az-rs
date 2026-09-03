//! Strong protocol-time values owned by Amazon Pervasives.

use crate::protocol_time::define_protocol_time;

define_protocol_time!(
    /// `Amazon::Pervasives::Duration`, carried as an unsigned nanosecond count.
    Duration
);

define_protocol_time!(
    /// `Amazon::Pervasives::Timestamp`, carried as an unsigned nanosecond count.
    Timestamp
);

#[cfg(test)]
mod tests {
    use crate::serialize::Marshaler;
    use crate::serialize::buffer::{CARRIER_ENDIAN, ReadBuffer, WriteBuffer};

    use super::*;

    #[test]
    fn duration_preserves_the_fixed_u64_wire_shape() {
        let value = Duration::from_nanoseconds(0x0102_0304_0506_0708);
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        value.marshal(&mut wb);

        let bytes = wb.into_vec();
        assert_eq!(bytes.len(), 8);
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let decoded = Duration::unmarshal(&mut rb).expect("decode Pervasives duration");
        assert_eq!(decoded, value);
        assert_eq!(rb.left(), 0);
    }
}
