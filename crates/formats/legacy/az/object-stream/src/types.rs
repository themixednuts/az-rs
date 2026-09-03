//! AZ type-UUID constants used by `ObjectStream` decoding.
//!
//! These UUIDs identify common primitive, container, and math value types.
//! The full reflected class registry comes from `SerializeContext` data; this
//! module only owns the fixed UUIDs needed by generic `ObjectStream` readers.

use az_core::component::ComponentId;
use az_core::type_info::AzTypeInfo;
use az_core::uuid::type_ids;
use serde_json::{Value, json};
use thiserror::Error;
use uuid::{Uuid, uuid};

pub use az_core::uuid::type_ids::{
    AABB, AZ_UUID, AZSTD_ARRAY, AZSTD_BASIC_STRING, AZSTD_BASIC_STRING_VIEW, AZSTD_BITSET,
    AZSTD_CHAR_TRAITS, AZSTD_EQUAL_TO, AZSTD_FIXED_FORWARD_LIST, AZSTD_FIXED_LIST,
    AZSTD_FIXED_VECTOR, AZSTD_FORWARD_LIST, AZSTD_FUNCTION, AZSTD_GREATER, AZSTD_GREATER_EQUAL,
    AZSTD_HASH, AZSTD_INTRUSIVE_PTR, AZSTD_LESS, AZSTD_LESS_EQUAL, AZSTD_LIST, AZSTD_MAP,
    AZSTD_MONOSTATE, AZSTD_OPTIONAL, AZSTD_PAIR, AZSTD_SET, AZSTD_SHARED_PTR, AZSTD_STRING,
    AZSTD_STRING_LEGACY_XML, AZSTD_UNORDERED_MAP, AZSTD_UNORDERED_MULTIMAP,
    AZSTD_UNORDERED_MULTISET, AZSTD_UNORDERED_SET, AZSTD_VECTOR, AZSTD_VECTOR_LEGACY_XML, BOOL,
    CHAR, COLOR, COLORB, COLORF, CRC32, DOUBLE, ENTITY_ID, FLOAT, INT, LONG, MATRIX3X3, MATRIX4X4,
    OBB, PLANE, PLATFORM_ID, QUATERNION, SHORT, SIGNED_CHAR, TRANSFORM, VARIANT, VECTOR_FLOAT,
    VECTOR2, VECTOR3, VECTOR4, VOID,
};

pub const AZ_S8: Uuid = type_ids::S8;
pub const AZ_S64: Uuid = type_ids::S64;
pub const UNSIGNED_CHAR: Uuid = type_ids::U8;
pub const UNSIGNED_SHORT: Uuid = type_ids::U16;
pub const UNSIGNED_INT: Uuid = type_ids::U32;
pub const UNSIGNED_LONG: Uuid = type_ids::ULONG;
pub const AZ_U64: Uuid = type_ids::U64;
/// Folded `AZStd::vector<AZ::ComponentId>` used for component-id lists.
pub const COMPONENT_ID_VECTOR: Uuid = <Vec<ComponentId> as AzTypeInfo>::TYPE_ID;

/// Reflected `AZ::Entity` root object used by slice/UI `ObjectStreams`.
pub const AZ_ENTITY: Uuid = type_ids::AZ_ENTITY;

/// Reflected `SliceComponent` wrapper for embedded slice entities.
pub const SLICE_COMPONENT: Uuid = uuid!("AFD304E4-1773-47C8-855A-8B622398934F");

pub const ASSET: Uuid = type_ids::AZ_DATA_ASSET_REFLECTION;
pub const ASSET_ID: Uuid = type_ids::AZ_DATA_ASSET_ID;
pub const BYTE_STREAM: Uuid = type_ids::BYTE_STREAM;
pub const DYNAMIC_SERIALIZABLE_FIELD: Uuid = uuid!("D761E0C2-A098-497C-B8EB-EA62F5ED896B");

/// `AZStd::any`.
///
/// Retail registers it with an `IDataContainer` whose `EnumElements`
/// fabricates a single dynamic `m_data` `ClassElement` rather than
/// enumerating a collection, so `ObjectStream` serializes an `any` as the same
/// `{TypeId, m_data}` shape as [`DYNAMIC_SERIALIZABLE_FIELD`]. Lowering seeds
/// that shape and does not treat this type as an opaque collection container.
pub const AZSTD_ANY: Uuid = uuid!("03924488-C7F4-4D6D-948B-ABC2D1AE2FD3");
pub const DATA_OVERLAY_INFO: Uuid = uuid!("42DD05B6-19A8-4DEF-8718-8BF5CAC8828E");

/// Failure modes of [`uuid_data_to_serialize`].
///
/// Unknown, composite, and serializer-owned types fail explicitly; that
/// display helper never guesses a payload layout from raw bytes.
#[derive(Debug, Error)]
pub enum UuidDataError {
    #[error("type {type_id} has invalid payload length {actual}")]
    InvalidLength { type_id: Uuid, actual: usize },
    #[error("type {type_id} payload is not UTF-8")]
    Utf8 {
        type_id: Uuid,
        #[source]
        source: std::str::Utf8Error,
    },
    #[error("type {0} requires a resolved ClassData serializer context")]
    SerializerContextRequired(Uuid),
    #[error("type {0} has no registered legacy display codec")]
    UnsupportedType(Uuid),
}

/// Render an `Element`'s raw `data` into a typed JSON [`Value`]
/// based on the type UUID.
///
/// Unknown, composite, and serializer-owned types fail explicitly; this
/// display helper never guesses a payload layout from raw bytes.
///
/// # Errors
///
/// Returns [`UuidDataError::InvalidLength`] when `data` is not exactly the
/// byte width the type demands (1/2/4/8/16 for the scalar and UUID arms,
/// `4 * component count` for the math arms),
/// [`UuidDataError::Utf8`] when a string type's bytes are not valid UTF-8,
/// [`UuidDataError::SerializerContextRequired`] for [`ASSET`], whose layout is
/// owned by a reflected serializer rather than a fixed byte shape, and
/// [`UuidDataError::UnsupportedType`] for every other `id`, including
/// [`ASSET_ID`], [`ENTITY_ID`] and [`CRC32`].
///
/// # Panics
///
/// Panics if a fixed-width arm's declared length disagrees with the array it
/// decodes into — for example if the `fixed(4)` guard were paired with
/// `i64::from_be_bytes`. Both sides are literals in this function, so the
/// `expect`s are unreachable for any `data`. The `unreachable!` in the math
/// component-count table likewise fires only if that inner `match` stops
/// covering the arm pattern that selected it.
pub fn uuid_data_to_serialize(
    id: &Uuid,
    data: &[u8],
    is_json: bool,
) -> Result<Value, UuidDataError> {
    let fixed = |expected: usize| {
        (data.len() == expected)
            .then_some(data)
            .ok_or(UuidDataError::InvalidLength {
                type_id: *id,
                actual: data.len(),
            })
    };
    let res = match *id {
        CHAR | AZ_S8 | SIGNED_CHAR => {
            Value::Number(i8::from_be_bytes(fixed(1)?.try_into().expect("length checked")).into())
        }
        SHORT => {
            Value::Number(i16::from_be_bytes(fixed(2)?.try_into().expect("length checked")).into())
        }
        INT | LONG => {
            Value::Number(i32::from_be_bytes(fixed(4)?.try_into().expect("length checked")).into())
        }
        AZ_S64 => {
            Value::Number(i64::from_be_bytes(fixed(8)?.try_into().expect("length checked")).into())
        }

        UNSIGNED_CHAR => {
            Value::Number(u8::from_be_bytes(fixed(1)?.try_into().expect("length checked")).into())
        }
        UNSIGNED_SHORT => {
            Value::Number(u16::from_be_bytes(fixed(2)?.try_into().expect("length checked")).into())
        }
        UNSIGNED_INT | UNSIGNED_LONG => {
            Value::Number(u32::from_be_bytes(fixed(4)?.try_into().expect("length checked")).into())
        }
        AZ_U64 => {
            Value::Number(u64::from_be_bytes(fixed(8)?.try_into().expect("length checked")).into())
        }

        FLOAT => json!(format!(
            "{:.7}",
            f32::from_be_bytes(fixed(4)?.try_into().expect("length checked"))
        )),
        DOUBLE => json!(format!(
            "{:.7}",
            f64::from_be_bytes(fixed(8)?.try_into().expect("length checked"))
        )),

        BOOL => Value::Bool(u8::from_be_bytes(fixed(1)?.try_into().expect("length checked")) != 0),

        AZ_UUID => json!(
            Uuid::from_bytes(fixed(16)?.try_into().expect("length checked"))
                .braced()
                .encode_upper(&mut Uuid::encode_buffer())
        ),

        ASSET => return Err(UuidDataError::SerializerContextRequired(*id)),

        VECTOR_FLOAT | VECTOR2 | VECTOR3 | VECTOR4 | TRANSFORM | QUATERNION | COLOR | MATRIX3X3
        | MATRIX4X4 => {
            let count = match *id {
                VECTOR_FLOAT => 1,
                VECTOR2 => 2,
                VECTOR3 => 3,
                VECTOR4 | QUATERNION | COLOR => 4,
                TRANSFORM => 12,
                MATRIX3X3 => 9,
                MATRIX4X4 => 16,
                _ => unreachable!("match arm proves a math type"),
            };
            if data.len() != count * 4 {
                return Err(UuidDataError::InvalidLength {
                    type_id: *id,
                    actual: data.len(),
                });
            }
            let data = data.chunks_exact(4);
            let data = data.map(|b| {
                let num = f32::from_be_bytes(b.try_into().expect("chunk width is four"));
                format!("{num:.7}")
            });

            if is_json {
                Value::Array(data.map(|v| json!(v)).collect())
            } else {
                json!(data.collect::<Vec<_>>().join(" "))
            }
        }

        AZSTD_STRING | AZSTD_BASIC_STRING | AZSTD_STRING_LEGACY_XML => {
            json!(
                std::str::from_utf8(data).map_err(|source| UuidDataError::Utf8 {
                    type_id: *id,
                    source,
                })?
            )
        }
        BYTE_STREAM => json!(hex::encode_upper(data)),

        // `ASSET_ID`, `ENTITY_ID` and `CRC32` are reflected through their own
        // serializers rather than a fixed byte layout, so they land here with
        // every type id this display helper has no legacy codec for.
        _ => return Err(UuidDataError::UnsupportedType(*id)),
    };
    Ok(res)
}
