use az_secret_value::Secret;
use gridmate::serialize::{CARRIER_ENDIAN, Marshaler, ReadBuffer, WriteBuffer};

#[test]
fn secret_wrapper_preserves_inner_wire_shape() {
    assert_eq!(
        <Secret<u32> as Marshaler>::MARSHAL_SIZE,
        <u32 as Marshaler>::MARSHAL_SIZE
    );

    let mut inner_buffer = WriteBuffer::new(CARRIER_ENDIAN);
    0x1234_5678_u32.marshal(&mut inner_buffer);
    let mut secret_buffer = WriteBuffer::new(CARRIER_ENDIAN);
    Secret::new(0x1234_5678_u32).marshal(&mut secret_buffer);

    assert_eq!(secret_buffer.as_slice(), inner_buffer.as_slice());

    let mut read = ReadBuffer::new(CARRIER_ENDIAN, secret_buffer.as_slice());
    let decoded = Secret::<u32>::unmarshal(&mut read).expect("decode transparent secret");
    assert_eq!(*decoded.expose(), 0x1234_5678);
}
