use az_secret_value::Secret;

use super::{Marshaler, MarshalerError, ReadBuffer, WriteBuffer};

impl<T: Marshaler> Marshaler for Secret<T> {
    const MARSHAL_SIZE: usize = T::MARSHAL_SIZE;

    fn marshal(&self, buffer: &mut WriteBuffer) {
        self.expose().marshal(buffer);
    }

    fn unmarshal(buffer: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        T::unmarshal(buffer).map(Self::new)
    }
}
