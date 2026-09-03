//! `GridMate` marshalers for canonical `AzCore` asset identity types.

use az_core::asset::AssetId;

use super::{Marshaler, MarshalerError, ReadBuffer, WriteBuffer};

impl Marshaler for AssetId {
    const MARSHAL_SIZE: usize = 20;

    fn marshal(&self, wb: &mut WriteBuffer) {
        self.guid.marshal(wb);
        self.sub_id.marshal(wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self::new(
            rb.field("guid", uuid::Uuid::unmarshal)?,
            rb.field("sub_id", u32::unmarshal)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::serialize::CARRIER_ENDIAN;

    #[test]
    fn asset_id_is_uuid_then_carrier_endian_sub_id() {
        let asset = AssetId::new(
            Uuid::from_bytes([
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
                0x0f, 0x10,
            ]),
            0x1122_3344,
        );
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);

        asset.marshal(&mut wb);

        assert_eq!(
            wb.as_slice(),
            &[
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
                0x0f, 0x10, 0x11, 0x22, 0x33, 0x44,
            ]
        );

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, wb.as_slice());
        assert_eq!(AssetId::unmarshal(&mut rb).unwrap(), asset);
        assert_eq!(rb.left(), 0);
    }
}
