//! Reflected serializer boundary for `ObjectStream` leaf payloads.
//!
//! Lumberyard invokes the `IDataSerializer` stored on resolved `ClassData`.
//! Accordingly, codecs are registered against `ClassData` keys by the read
//! context; recognizing a UUID never invokes a codec by itself.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::asset_reference::AssetValueLayout;
use crate::{PayloadEncoding, types};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Xml,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeEndian {
    Little,
    Big,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeAbi {
    /// Windows ABI used by Lumberyard: C/C++ `long` is 32-bit.
    Llp64,
    /// Unix 64-bit ABI: C/C++ `long` is 64-bit.
    Lp64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuiltinSerializerKind {
    Empty,
    /// `AZStd::ranged_int<Unsigned, Min, Max>` delegates binary width and text
    /// formatting to its unsigned storage type. Native text loading uses
    /// `strtoul`/`strtoull` followed by a width cast; Min/Max participate only
    /// in the reflected type identity.
    RangedUnsigned {
        bytes: u8,
    },
    I8,
    I16,
    I32,
    Long,
    I64,
    U8,
    U16,
    U32,
    UnsignedLong,
    U64,
    F32,
    F64,
    Bool,
    Uuid,
    Asset,
    F32Sequence {
        count: usize,
    },
    String,
    ByteStream,
}

impl BuiltinSerializerKind {
    /// Build the exact ranged-unsigned serializer family for a captured
    /// primitive underlying descriptor.
    #[must_use]
    pub const fn ranged_from_underlying(underlying: Self) -> Option<Self> {
        let bytes = match underlying {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::U32 | Self::UnsignedLong => 4,
            Self::U64 => 8,
            _ => return None,
        };
        Some(Self::RangedUnsigned { bytes })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedPayload {
    pub bytes: Vec<u8>,
    pub encoding: PayloadEncoding,
}

impl EncodedPayload {
    #[must_use]
    pub const fn new(bytes: Vec<u8>, encoding: PayloadEncoding) -> Self {
        Self { bytes, encoding }
    }

    #[must_use]
    pub const fn binary_big_endian(bytes: Vec<u8>) -> Self {
        Self::new(bytes, PayloadEncoding::BinaryBigEndian)
    }
}

#[derive(Debug, Error)]
pub enum ValueCodecError {
    #[error("ObjectStream JSON `value` must be a string")]
    NonStringJsonValue,
    #[error("invalid {expected} value `{value}`")]
    InvalidText {
        expected: &'static str,
        value: String,
    },
    #[error("invalid {expected} payload length {actual}")]
    InvalidLength {
        expected: &'static str,
        actual: usize,
    },
    #[error("payload is not valid UTF-8")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("invalid hexadecimal byte stream: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("asset payload is invalid: {0}")]
    Asset(String),
    #[error("codec does not support {encoding:?} payloads")]
    UnsupportedPayloadEncoding { encoding: PayloadEncoding },
}

pub trait ObjectStreamValueCodec: fmt::Debug + Send + Sync {
    /// Decode the serializer's text form into its native payload bytes.
    ///
    /// # Errors
    ///
    /// Implementation-defined. The builtin codec reports
    /// [`ValueCodecError::InvalidText`] for a token the serializer's grammar
    /// rejects, [`ValueCodecError::InvalidLength`] when a fixed-arity value (a
    /// float sequence, a ranged-unsigned width) has the wrong element count,
    /// [`ValueCodecError::Hex`] for a malformed byte stream, and
    /// [`ValueCodecError::Asset`] for an unparsable asset reference.
    fn text_to_data(
        &self,
        type_id: Uuid,
        text: &str,
        element_version: u32,
        encoding: TextEncoding,
    ) -> Result<EncodedPayload, ValueCodecError>;

    /// `ObjectStream` XML attributes and JSON `value` members both carry the
    /// serializer's text verbatim. JSON does not expose native JSON values.
    ///
    /// # Errors
    ///
    /// Implementation-defined. The builtin codec reports
    /// [`ValueCodecError::InvalidLength`] when `data` is not the width the
    /// serializer stores, [`ValueCodecError::Utf8`] when a string payload is not
    /// valid UTF-8, [`ValueCodecError::UnsupportedPayloadEncoding`] when
    /// `payload_encoding` is [`PayloadEncoding::Text`] for a codec that needs
    /// binary input, and [`ValueCodecError::Asset`] for an asset payload that
    /// cannot be re-rendered.
    fn data_to_text(
        &self,
        type_id: Uuid,
        data: &[u8],
        payload_encoding: PayloadEncoding,
        _element_version: u32,
        encoding: TextEncoding,
    ) -> Result<String, ValueCodecError>;

    /// Normalize `data` to the canonical big-endian wire form.
    ///
    /// # Errors
    ///
    /// Implementation-defined. The builtin codec reports
    /// [`ValueCodecError::InvalidLength`] when `data` is not the serializer's
    /// storage width and [`ValueCodecError::UnsupportedPayloadEncoding`] when
    /// `payload_encoding` is [`PayloadEncoding::Text`], which carries no byte order
    /// to normalize.
    fn to_big_endian(
        &self,
        type_id: Uuid,
        data: &[u8],
        payload_encoding: PayloadEncoding,
        element_version: u32,
    ) -> Result<Vec<u8>, ValueCodecError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltinSerializerDescriptor {
    pub kind: BuiltinSerializerKind,
    pub native_abi: NativeAbi,
    pub native_endian: NativeEndian,
    pub current_version: u32,
}

impl BuiltinSerializerDescriptor {
    #[must_use]
    pub const fn new(kind: BuiltinSerializerKind, current_version: u32) -> Self {
        Self {
            kind,
            native_abi: NativeAbi::Llp64,
            native_endian: NativeEndian::Little,
            current_version,
        }
    }

    #[must_use]
    pub const fn with_platform(
        mut self,
        native_abi: NativeAbi,
        native_endian: NativeEndian,
    ) -> Self {
        self.native_abi = native_abi;
        self.native_endian = native_endian;
        self
    }

    #[must_use]
    pub const fn codec(self) -> BuiltinValueCodec {
        BuiltinValueCodec { descriptor: self }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BuiltinValueCodec {
    descriptor: BuiltinSerializerDescriptor,
}

impl BuiltinValueCodec {
    #[must_use]
    pub const fn new(kind: BuiltinSerializerKind, asset_serializer_version: u32) -> Self {
        BuiltinSerializerDescriptor::new(kind, asset_serializer_version).codec()
    }

    #[must_use]
    pub const fn with_platform(
        kind: BuiltinSerializerKind,
        asset_serializer_version: u32,
        native_abi: NativeAbi,
        native_endian: NativeEndian,
    ) -> Self {
        BuiltinSerializerDescriptor::new(kind, asset_serializer_version)
            .with_platform(native_abi, native_endian)
            .codec()
    }
}

#[must_use]
pub const fn builtin_serializer_kind(type_id: Uuid) -> Option<BuiltinSerializerKind> {
    Some(match type_id {
        types::CHAR | types::AZ_S8 | types::SIGNED_CHAR => BuiltinSerializerKind::I8,
        types::SHORT => BuiltinSerializerKind::I16,
        types::INT => BuiltinSerializerKind::I32,
        types::LONG => BuiltinSerializerKind::Long,
        types::AZ_S64 => BuiltinSerializerKind::I64,
        types::UNSIGNED_CHAR => BuiltinSerializerKind::U8,
        types::UNSIGNED_SHORT => BuiltinSerializerKind::U16,
        types::UNSIGNED_INT => BuiltinSerializerKind::U32,
        types::UNSIGNED_LONG => BuiltinSerializerKind::UnsignedLong,
        types::AZ_U64 => BuiltinSerializerKind::U64,
        types::FLOAT => BuiltinSerializerKind::F32,
        types::DOUBLE => BuiltinSerializerKind::F64,
        types::BOOL => BuiltinSerializerKind::Bool,
        types::AZ_UUID => BuiltinSerializerKind::Uuid,
        types::ASSET => BuiltinSerializerKind::Asset,
        types::VECTOR_FLOAT => BuiltinSerializerKind::F32Sequence { count: 1 },
        types::VECTOR2 => BuiltinSerializerKind::F32Sequence { count: 2 },
        types::VECTOR3 => BuiltinSerializerKind::F32Sequence { count: 3 },
        types::VECTOR4 | types::COLOR | types::QUATERNION => {
            BuiltinSerializerKind::F32Sequence { count: 4 }
        }
        types::TRANSFORM => BuiltinSerializerKind::F32Sequence { count: 12 },
        types::MATRIX3X3 => BuiltinSerializerKind::F32Sequence { count: 9 },
        types::MATRIX4X4 => BuiltinSerializerKind::F32Sequence { count: 16 },
        types::AZSTD_STRING | types::AZSTD_BASIC_STRING | types::AZSTD_STRING_LEGACY_XML => {
            BuiltinSerializerKind::String
        }
        types::BYTE_STREAM => BuiltinSerializerKind::ByteStream,
        _ => return None,
    })
}

#[must_use]
pub const fn is_builtin_type(type_id: Uuid) -> bool {
    builtin_serializer_kind(type_id).is_some()
}

impl ObjectStreamValueCodec for BuiltinValueCodec {
    fn text_to_data(
        &self,
        _type_id: Uuid,
        text: &str,
        element_version: u32,
        _encoding: TextEncoding,
    ) -> Result<EncodedPayload, ValueCodecError> {
        let bytes = match self.descriptor.kind {
            BuiltinSerializerKind::Empty => {
                if text.is_empty() {
                    Vec::new()
                } else {
                    return Err(ValueCodecError::InvalidText {
                        expected: "empty serializer text",
                        value: text.to_owned(),
                    });
                }
            }
            BuiltinSerializerKind::RangedUnsigned { bytes } => {
                encode_ranged_unsigned(text, bytes, self.descriptor.native_endian)?
            }
            BuiltinSerializerKind::I8 => parse_number::<i8>(text, "i8")?.to_be_bytes().to_vec(),
            BuiltinSerializerKind::I16 => {
                native_i16(parse_number(text, "i16")?, self.descriptor.native_endian)
            }
            BuiltinSerializerKind::I32 => {
                native_i32(parse_number(text, "i32")?, self.descriptor.native_endian)
            }
            BuiltinSerializerKind::Long if self.descriptor.native_abi == NativeAbi::Llp64 => {
                native_i32(parse_number(text, "long")?, self.descriptor.native_endian)
            }
            BuiltinSerializerKind::Long => {
                native_i64(parse_number(text, "long")?, self.descriptor.native_endian)
            }
            BuiltinSerializerKind::I64 => {
                native_i64(parse_number(text, "i64")?, self.descriptor.native_endian)
            }
            BuiltinSerializerKind::U8 => parse_number::<u8>(text, "u8")?.to_be_bytes().to_vec(),
            BuiltinSerializerKind::U16 => {
                native_u16(parse_number(text, "u16")?, self.descriptor.native_endian)
            }
            BuiltinSerializerKind::U32 => {
                native_u32(parse_number(text, "u32")?, self.descriptor.native_endian)
            }
            BuiltinSerializerKind::UnsignedLong
                if self.descriptor.native_abi == NativeAbi::Llp64 =>
            {
                native_u32(
                    parse_number(text, "unsigned long")?,
                    self.descriptor.native_endian,
                )
            }
            BuiltinSerializerKind::UnsignedLong => native_u64(
                parse_number(text, "unsigned long")?,
                self.descriptor.native_endian,
            ),
            BuiltinSerializerKind::U64 => {
                native_u64(parse_number(text, "u64")?, self.descriptor.native_endian)
            }
            BuiltinSerializerKind::F32 => {
                native_f32(parse_f32(text)?, self.descriptor.native_endian)
            }
            BuiltinSerializerKind::F64 => {
                native_f64(parse_number(text, "f64")?, self.descriptor.native_endian)
            }
            // Lumberyard's BoolSerializer uses strcmp("true", text), which
            // silently maps every malformed token to false. The importer is
            // lossless/fail-closed: accept the two canonical serializer
            // spellings and never fabricate false from corrupt text.
            BuiltinSerializerKind::Bool => vec![u8::from(parse_bool(text)?)],
            BuiltinSerializerKind::Uuid => parse_uuid(text)?.into_bytes().to_vec(),
            BuiltinSerializerKind::Asset => {
                let bytes = AssetValueLayout::from_text(text, element_version)
                    .map_err(|error| ValueCodecError::Asset(error.to_string()))?
                    .to_big_endian_bytes(self.descriptor.current_version)
                    .map_err(|error| ValueCodecError::Asset(error.to_string()))?;
                asset_big_to_native(
                    bytes,
                    self.descriptor.current_version,
                    self.descriptor.native_endian,
                )?
            }
            BuiltinSerializerKind::F32Sequence { count } => {
                encode_f32_sequence(text, count, self.descriptor.native_endian)?
            }
            BuiltinSerializerKind::String => text.as_bytes().to_vec(),
            BuiltinSerializerKind::ByteStream => hex::decode(text.trim())?,
        };
        Ok(EncodedPayload::new(
            bytes,
            PayloadEncoding::BinaryNativeEndian,
        ))
    }

    fn data_to_text(
        &self,
        type_id: Uuid,
        data: &[u8],
        payload_encoding: PayloadEncoding,
        element_version: u32,
        _encoding: TextEncoding,
    ) -> Result<String, ValueCodecError> {
        if self.descriptor.kind == BuiltinSerializerKind::Asset {
            let endian = match payload_encoding {
                PayloadEncoding::BinaryBigEndian => NativeEndian::Big,
                PayloadEncoding::BinaryNativeEndian => self.descriptor.native_endian,
                PayloadEncoding::Text => {
                    return Err(ValueCodecError::UnsupportedPayloadEncoding {
                        encoding: PayloadEncoding::Text,
                    });
                }
            };
            return AssetValueLayout::historical_data_to_current_text(
                data,
                element_version,
                endian,
                self.descriptor.current_version,
            )
            .map_err(|error| ValueCodecError::Asset(error.to_string()));
        }
        let bytes = self.to_big_endian(type_id, data, payload_encoding, element_version)?;
        match self.descriptor.kind {
            BuiltinSerializerKind::Empty => Ok(String::new()),
            BuiltinSerializerKind::RangedUnsigned { bytes: 1 } => {
                Ok(u8::from_be_bytes(fixed(&bytes, "ranged u8")?).to_string())
            }
            BuiltinSerializerKind::RangedUnsigned { bytes: 2 } => {
                Ok(u16::from_be_bytes(fixed(&bytes, "ranged u16")?).to_string())
            }
            BuiltinSerializerKind::RangedUnsigned { bytes: 4 } => {
                Ok(u32::from_be_bytes(fixed(&bytes, "ranged u32")?).to_string())
            }
            BuiltinSerializerKind::RangedUnsigned { bytes: 8 } => {
                Ok(u64::from_be_bytes(fixed(&bytes, "ranged u64")?).to_string())
            }
            BuiltinSerializerKind::RangedUnsigned { .. } => Err(ValueCodecError::InvalidLength {
                expected: "1-, 2-, 4-, or 8-byte ranged unsigned value",
                actual: bytes.len(),
            }),
            BuiltinSerializerKind::I8 => Ok(i8::from_be_bytes(fixed(&bytes, "i8")?).to_string()),
            BuiltinSerializerKind::I16 => Ok(i16::from_be_bytes(fixed(&bytes, "i16")?).to_string()),
            BuiltinSerializerKind::I32 => Ok(i32::from_be_bytes(fixed(&bytes, "i32")?).to_string()),
            BuiltinSerializerKind::Long if self.descriptor.native_abi == NativeAbi::Llp64 => {
                Ok(i32::from_be_bytes(fixed(&bytes, "long")?).to_string())
            }
            BuiltinSerializerKind::Long | BuiltinSerializerKind::I64 => {
                Ok(i64::from_be_bytes(fixed(&bytes, "i64")?).to_string())
            }
            BuiltinSerializerKind::U8 => Ok(u8::from_be_bytes(fixed(&bytes, "u8")?).to_string()),
            BuiltinSerializerKind::U16 => Ok(u16::from_be_bytes(fixed(&bytes, "u16")?).to_string()),
            BuiltinSerializerKind::U32 => Ok(u32::from_be_bytes(fixed(&bytes, "u32")?).to_string()),
            BuiltinSerializerKind::UnsignedLong
                if self.descriptor.native_abi == NativeAbi::Llp64 =>
            {
                Ok(u32::from_be_bytes(fixed(&bytes, "unsigned long")?).to_string())
            }
            BuiltinSerializerKind::UnsignedLong | BuiltinSerializerKind::U64 => {
                Ok(u64::from_be_bytes(fixed(&bytes, "u64")?).to_string())
            }
            BuiltinSerializerKind::F32 => {
                Ok(format!("{:.7}", f32::from_be_bytes(fixed(&bytes, "f32")?)))
            }
            BuiltinSerializerKind::F64 => {
                Ok(format!("{:.7}", f64::from_be_bytes(fixed(&bytes, "f64")?)))
            }
            BuiltinSerializerKind::Bool => Ok(if u8::from_be_bytes(fixed(&bytes, "bool")?) == 0 {
                "false".to_owned()
            } else {
                "true".to_owned()
            }),
            BuiltinSerializerKind::Uuid => Ok(uuid_text(Uuid::from_bytes(fixed(&bytes, "UUID")?))),
            BuiltinSerializerKind::Asset => unreachable!("handled before normalization"),
            BuiltinSerializerKind::F32Sequence { count } => f32_sequence_to_text(&bytes, count),
            BuiltinSerializerKind::String => Ok(std::str::from_utf8(&bytes)?.to_owned()),
            BuiltinSerializerKind::ByteStream => Ok(hex::encode_upper(&bytes)),
        }
    }

    fn to_big_endian(
        &self,
        _type_id: Uuid,
        data: &[u8],
        payload_encoding: PayloadEncoding,
        _element_version: u32,
    ) -> Result<Vec<u8>, ValueCodecError> {
        match payload_encoding {
            PayloadEncoding::BinaryBigEndian => Ok(data.to_vec()),
            PayloadEncoding::BinaryNativeEndian => native_to_big(
                self.descriptor.kind,
                data,
                self.descriptor.native_abi,
                self.descriptor.native_endian,
                self.descriptor.current_version,
            ),
            PayloadEncoding::Text => Err(ValueCodecError::UnsupportedPayloadEncoding {
                encoding: PayloadEncoding::Text,
            }),
        }
    }
}

fn parse_number<T>(text: &str, expected: &'static str) -> Result<T, ValueCodecError>
where
    T: std::str::FromStr,
{
    text.trim().parse().map_err(|_| invalid(expected, text))
}

/// Parse serializer text as `f64`, then narrow to the `f32` storage width.
///
/// Lumberyard's float serializers accept the wider text grammar and store the
/// narrowed value, so the parse stays at `f64`.
// `f32::try_from(f64)` does not exist, so clippy's checked-conversion advice
// cannot be applied here; the narrowing is the serializer's storage width.
#[allow(clippy::cast_possible_truncation)]
fn parse_f32(text: &str) -> Result<f32, ValueCodecError> {
    Ok(parse_number::<f64>(text, "f32")? as f32)
}

fn encode_ranged_unsigned(
    text: &str,
    bytes: u8,
    endian: NativeEndian,
) -> Result<Vec<u8>, ValueCodecError> {
    if !matches!(bytes, 1 | 2 | 4 | 8) {
        return Err(ValueCodecError::InvalidLength {
            expected: "1-, 2-, 4-, or 8-byte ranged unsigned value",
            actual: usize::from(bytes),
        });
    }

    // The LLP64 ranged_int serializer calls the CRT strtoul for
    // 8/16/32-bit storage and strtoull for 64-bit storage, then casts to T.
    // Those functions skip leading whitespace, accept a sign, stop at the
    // first non-decimal byte, return zero when no digits were consumed, and
    // saturate to the source function's maximum on overflow. Preserve those
    // semantics exactly; the reflected Min/Max values do not clamp or reject.
    let source_max = if bytes <= 4 {
        u64::from(u32::MAX)
    } else {
        u64::MAX
    };
    let mut input = text.trim_start().as_bytes();
    let negative = match input.first() {
        Some(b'+') => {
            input = &input[1..];
            false
        }
        Some(b'-') => {
            input = &input[1..];
            true
        }
        _ => false,
    };
    let mut value = 0_u64;
    let mut overflow = false;
    let mut consumed = false;
    for digit in input.iter().copied().take_while(u8::is_ascii_digit) {
        consumed = true;
        let digit = u64::from(digit - b'0');
        match value
            .checked_mul(10)
            .and_then(|value| value.checked_add(digit))
            .filter(|value| *value <= source_max)
        {
            Some(next) => value = next,
            None => overflow = true,
        }
    }
    if !consumed {
        value = 0;
    } else if overflow {
        value = source_max;
    } else if negative {
        value = value.wrapping_neg() & source_max;
    }

    // Narrowing to the storage width is the C cast the serializer performs;
    // slicing the native-order encoding reproduces it without a lossy `as`.
    let width = usize::from(bytes);
    Ok(match endian {
        NativeEndian::Little => value.to_le_bytes()[..width].to_vec(),
        NativeEndian::Big => value.to_be_bytes()[size_of::<u64>() - width..].to_vec(),
    })
}

fn parse_bool(text: &str) -> Result<bool, ValueCodecError> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ValueCodecError::InvalidText {
            expected: "bool (`true` or `false`)",
            value: text.to_owned(),
        }),
    }
}

fn parse_uuid(text: &str) -> Result<Uuid, ValueCodecError> {
    Uuid::parse_str(text.trim().trim_start_matches('{').trim_end_matches('}'))
        .map_err(|_| invalid("UUID", text))
}

fn encode_f32_sequence(
    text: &str,
    count: usize,
    endian: NativeEndian,
) -> Result<Vec<u8>, ValueCodecError> {
    let parts = text.split_whitespace().collect::<Vec<_>>();
    if parts.len() != count {
        return Err(ValueCodecError::InvalidLength {
            expected: float_sequence_name(count),
            actual: parts.len(),
        });
    }
    let mut bytes = Vec::with_capacity(count * 4);
    for part in parts {
        bytes.extend_from_slice(&native_f32(parse_f32(part)?, endian));
    }
    Ok(bytes)
}

fn f32_sequence_to_text(data: &[u8], count: usize) -> Result<String, ValueCodecError> {
    if data.len() != count * 4 {
        return Err(ValueCodecError::InvalidLength {
            expected: float_sequence_name(count),
            actual: data.len(),
        });
    }
    Ok(data
        .chunks_exact(4)
        .map(|bytes| {
            format!(
                "{:.7}",
                f32::from_be_bytes(bytes.try_into().expect("chunk width is four"))
            )
        })
        .collect::<Vec<_>>()
        .join(" "))
}

const fn float_sequence_name(count: usize) -> &'static str {
    match count {
        1 => "one f32",
        2 => "two f32 values",
        3 => "three f32 values",
        4 => "four f32 values",
        9 => "nine f32 values",
        12 => "twelve f32 values",
        16 => "sixteen f32 values",
        _ => "fixed f32 sequence",
    }
}

fn native_to_big(
    kind: BuiltinSerializerKind,
    data: &[u8],
    abi: NativeAbi,
    endian: NativeEndian,
    asset_serializer_version: u32,
) -> Result<Vec<u8>, ValueCodecError> {
    if kind == BuiltinSerializerKind::Empty {
        return if data.is_empty() {
            Ok(Vec::new())
        } else {
            Err(ValueCodecError::InvalidLength {
                expected: "empty serializer payload",
                actual: data.len(),
            })
        };
    }
    if endian == NativeEndian::Big
        || matches!(
            kind,
            BuiltinSerializerKind::I8
                | BuiltinSerializerKind::U8
                | BuiltinSerializerKind::RangedUnsigned { bytes: 1 }
                | BuiltinSerializerKind::Bool
                | BuiltinSerializerKind::Uuid
                | BuiltinSerializerKind::String
                | BuiltinSerializerKind::ByteStream
        )
    {
        return Ok(data.to_vec());
    }
    match kind {
        BuiltinSerializerKind::RangedUnsigned { bytes: 2 } => {
            swap_chunks(data, 2, "ranged unsigned 16-bit value")
        }
        BuiltinSerializerKind::RangedUnsigned { bytes: 4 } => {
            swap_chunks(data, 4, "ranged unsigned 32-bit value")
        }
        BuiltinSerializerKind::RangedUnsigned { bytes: 8 } => {
            swap_chunks(data, 8, "ranged unsigned 64-bit value")
        }
        BuiltinSerializerKind::RangedUnsigned { .. } => Err(ValueCodecError::InvalidLength {
            expected: "1-, 2-, 4-, or 8-byte ranged unsigned value",
            actual: data.len(),
        }),
        BuiltinSerializerKind::I16 | BuiltinSerializerKind::U16 => {
            swap_chunks(data, 2, "16-bit value")
        }
        BuiltinSerializerKind::I32 | BuiltinSerializerKind::U32 | BuiltinSerializerKind::F32 => {
            swap_chunks(data, 4, "32-bit value")
        }
        BuiltinSerializerKind::Long | BuiltinSerializerKind::UnsignedLong
            if abi == NativeAbi::Llp64 =>
        {
            swap_chunks(data, 4, "LLP64 long")
        }
        BuiltinSerializerKind::Long
        | BuiltinSerializerKind::UnsignedLong
        | BuiltinSerializerKind::I64
        | BuiltinSerializerKind::U64
        | BuiltinSerializerKind::F64 => swap_chunks(data, 8, "64-bit value"),
        BuiltinSerializerKind::Asset => asset_native_to_big(data, asset_serializer_version, endian),
        BuiltinSerializerKind::F32Sequence { count } => {
            if data.len() != count * 4 {
                return Err(ValueCodecError::InvalidLength {
                    expected: float_sequence_name(count),
                    actual: data.len(),
                });
            }
            swap_chunks(data, 4, float_sequence_name(count))
        }
        BuiltinSerializerKind::I8
        | BuiltinSerializerKind::U8
        | BuiltinSerializerKind::Bool
        | BuiltinSerializerKind::Uuid
        | BuiltinSerializerKind::String
        | BuiltinSerializerKind::ByteStream => Ok(data.to_vec()),
        BuiltinSerializerKind::Empty => unreachable!("handled before endian normalization"),
    }
}

fn swap_chunks(
    data: &[u8],
    width: usize,
    expected: &'static str,
) -> Result<Vec<u8>, ValueCodecError> {
    if data.is_empty() || !data.len().is_multiple_of(width) {
        return Err(ValueCodecError::InvalidLength {
            expected,
            actual: data.len(),
        });
    }
    let mut bytes = data.to_vec();
    for chunk in bytes.chunks_exact_mut(width) {
        chunk.reverse();
    }
    Ok(bytes)
}

fn asset_big_to_native(
    bytes: Vec<u8>,
    serializer_version: u32,
    endian: NativeEndian,
) -> Result<Vec<u8>, ValueCodecError> {
    if endian == NativeEndian::Big {
        return Ok(bytes);
    }
    asset_swap_integer_fields(bytes, serializer_version)
}

fn asset_native_to_big(
    data: &[u8],
    serializer_version: u32,
    endian: NativeEndian,
) -> Result<Vec<u8>, ValueCodecError> {
    asset_big_to_native(data.to_vec(), serializer_version, endian)
}

fn asset_swap_integer_fields(
    mut bytes: Vec<u8>,
    serializer_version: u32,
) -> Result<Vec<u8>, ValueCodecError> {
    let minimum =
        48 + usize::from(serializer_version > 0) * 8 + usize::from(serializer_version > 1);
    if bytes.len() < minimum {
        return Err(ValueCodecError::InvalidLength {
            expected: "Asset serializer payload",
            actual: bytes.len(),
        });
    }
    bytes[16..20].reverse();
    if serializer_version > 0 {
        bytes[48..56].reverse();
    }
    Ok(bytes)
}

fn native_i16(value: i16, endian: NativeEndian) -> Vec<u8> {
    match endian {
        NativeEndian::Little => value.to_le_bytes(),
        NativeEndian::Big => value.to_be_bytes(),
    }
    .to_vec()
}

fn native_i32(value: i32, endian: NativeEndian) -> Vec<u8> {
    match endian {
        NativeEndian::Little => value.to_le_bytes(),
        NativeEndian::Big => value.to_be_bytes(),
    }
    .to_vec()
}

fn native_i64(value: i64, endian: NativeEndian) -> Vec<u8> {
    match endian {
        NativeEndian::Little => value.to_le_bytes(),
        NativeEndian::Big => value.to_be_bytes(),
    }
    .to_vec()
}

fn native_u16(value: u16, endian: NativeEndian) -> Vec<u8> {
    match endian {
        NativeEndian::Little => value.to_le_bytes(),
        NativeEndian::Big => value.to_be_bytes(),
    }
    .to_vec()
}

fn native_u32(value: u32, endian: NativeEndian) -> Vec<u8> {
    match endian {
        NativeEndian::Little => value.to_le_bytes(),
        NativeEndian::Big => value.to_be_bytes(),
    }
    .to_vec()
}

fn native_u64(value: u64, endian: NativeEndian) -> Vec<u8> {
    match endian {
        NativeEndian::Little => value.to_le_bytes(),
        NativeEndian::Big => value.to_be_bytes(),
    }
    .to_vec()
}

fn native_f32(value: f32, endian: NativeEndian) -> Vec<u8> {
    match endian {
        NativeEndian::Little => value.to_le_bytes(),
        NativeEndian::Big => value.to_be_bytes(),
    }
    .to_vec()
}

fn native_f64(value: f64, endian: NativeEndian) -> Vec<u8> {
    match endian {
        NativeEndian::Little => value.to_le_bytes(),
        NativeEndian::Big => value.to_be_bytes(),
    }
    .to_vec()
}

fn fixed<const N: usize>(data: &[u8], expected: &'static str) -> Result<[u8; N], ValueCodecError> {
    data.try_into().map_err(|_| ValueCodecError::InvalidLength {
        expected,
        actual: data.len(),
    })
}

fn uuid_text(uuid: Uuid) -> String {
    uuid.as_braced().to_string().to_uppercase()
}

fn invalid(expected: &'static str, value: impl Into<String>) -> ValueCodecError {
    ValueCodecError::InvalidText {
        expected,
        value: value.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_conversion_preserves_native_endian_provenance() {
        let payload = BuiltinValueCodec::new(BuiltinSerializerKind::I32, 0)
            .text_to_data(types::INT, "42", 0, TextEncoding::Xml)
            .unwrap();
        assert_eq!(payload.encoding, PayloadEncoding::BinaryNativeEndian);
        assert_eq!(payload.bytes, 42_i32.to_le_bytes());
        assert_eq!(
            BuiltinValueCodec::new(BuiltinSerializerKind::I32, 0)
                .to_big_endian(types::INT, &payload.bytes, payload.encoding, 0)
                .unwrap(),
            42_i32.to_be_bytes()
        );
    }

    #[test]
    fn asset_text_to_data_uses_registered_serializer_generation() {
        let v1_text = "id={01234567-89AB-CDEF-FEDC-BA9876543210}:33774488,type={00112233-4455-6677-8899-AABBCCDDEEFF},hint={abcd}";
        let lumberyard = BuiltinValueCodec::new(BuiltinSerializerKind::Asset, 1)
            .text_to_data(types::ASSET, v1_text, 1, TextEncoding::Xml)
            .unwrap();
        assert_eq!(lumberyard.bytes.len(), 60);

        let v2_text = format!("{v1_text},loadBehavior=2");
        let o3de_codec = BuiltinValueCodec::new(BuiltinSerializerKind::Asset, 2);
        let o3de = o3de_codec
            .text_to_data(types::ASSET, &v2_text, 2, TextEncoding::Json)
            .unwrap();
        assert_eq!(o3de.bytes.len(), 61);
        assert_eq!(o3de.bytes.last(), Some(&2));
        assert_eq!(
            o3de_codec
                .data_to_text(
                    types::ASSET,
                    &o3de.bytes,
                    o3de.encoding,
                    2,
                    TextEncoding::Json,
                )
                .unwrap(),
            v2_text
        );
    }

    #[test]
    fn older_asset_text_is_saved_in_current_o3de_layout() {
        let text = "id={01234567-89AB-CDEF-FEDC-BA9876543210}:33774488,type={00112233-4455-6677-8899-AABBCCDDEEFF}";
        let payload = BuiltinValueCodec::new(BuiltinSerializerKind::Asset, 2)
            .text_to_data(types::ASSET, text, 0, TextEncoding::Xml)
            .unwrap();
        assert_eq!(payload.bytes.len(), 57);
        assert_eq!(payload.bytes.last(), Some(&1));
    }

    #[test]
    fn historical_asset_binary_loads_old_version_before_current_text_save() {
        let text = "id={01234567-89AB-CDEF-FEDC-BA9876543210}:1,type={00112233-4455-6677-8899-AABBCCDDEEFF}";
        let old = AssetValueLayout::from_text(text, 0)
            .unwrap()
            .to_big_endian_bytes(0)
            .unwrap();
        let codec = BuiltinValueCodec::new(BuiltinSerializerKind::Asset, 1);

        assert_eq!(
            codec
                .data_to_text(
                    types::ASSET,
                    &old,
                    PayloadEncoding::BinaryBigEndian,
                    0,
                    TextEncoding::Xml,
                )
                .unwrap(),
            format!("{text},hint={{}}")
        );
    }

    #[test]
    fn bool_text_to_data_rejects_native_false_fabrication_for_invalid_tokens() {
        let codec = BuiltinValueCodec::new(BuiltinSerializerKind::Bool, 0);
        for text in ["True", "1", " true", "anything"] {
            assert!(matches!(
                codec.text_to_data(types::BOOL, text, 0, TextEncoding::Xml),
                Err(ValueCodecError::InvalidText { .. })
            ));
        }
        assert_eq!(
            codec
                .text_to_data(types::BOOL, "false", 0, TextEncoding::Xml)
                .unwrap()
                .bytes,
            [0]
        );
        assert_eq!(
            codec
                .text_to_data(types::BOOL, "true", 0, TextEncoding::Xml)
                .unwrap()
                .bytes,
            [1]
        );
    }

    #[test]
    fn ranged_unsigned_uses_native_crt_parse_then_storage_width_cast() {
        let codec = BuiltinValueCodec::new(BuiltinSerializerKind::RangedUnsigned { bytes: 1 }, 0);
        for (text, expected) in [
            ("300", 44),
            ("-1", 255),
            ("garbage", 0),
            ("12trailing", 12),
            ("999999999999999999999", 255),
        ] {
            assert_eq!(
                codec
                    .text_to_data(Uuid::nil(), text, 0, TextEncoding::Xml)
                    .unwrap()
                    .bytes,
                [expected]
            );
        }
    }

    #[test]
    fn empty_serializer_supports_only_the_native_zero_byte_value() {
        let codec = BuiltinValueCodec::new(BuiltinSerializerKind::Empty, 0);
        assert_eq!(
            codec
                .text_to_data(Uuid::nil(), "", 0, TextEncoding::Xml)
                .unwrap()
                .bytes,
            Vec::<u8>::new()
        );
        assert!(matches!(
            codec.text_to_data(Uuid::nil(), "0", 0, TextEncoding::Xml),
            Err(ValueCodecError::InvalidText { .. })
        ));
        assert!(matches!(
            codec.to_big_endian(Uuid::nil(), &[0], PayloadEncoding::BinaryNativeEndian, 0),
            Err(ValueCodecError::InvalidLength { .. })
        ));
    }

    #[test]
    fn llp64_long_is_four_bytes_and_az_s64_is_eight() {
        let long = BuiltinValueCodec::new(BuiltinSerializerKind::Long, 0)
            .text_to_data(types::LONG, "7", 0, TextEncoding::Json)
            .unwrap();
        let s64 = BuiltinValueCodec::new(BuiltinSerializerKind::I64, 0)
            .text_to_data(types::AZ_S64, "7", 0, TextEncoding::Json)
            .unwrap();
        assert_eq!(long.bytes.len(), 4);
        assert_eq!(s64.bytes.len(), 8);
    }

    #[test]
    fn math_serializers_reject_wrong_component_counts() {
        let codec = BuiltinValueCodec::new(BuiltinSerializerKind::F32Sequence { count: 3 }, 0);
        assert!(matches!(
            codec.text_to_data(types::VECTOR3, "1 2", 0, TextEncoding::Xml),
            Err(ValueCodecError::InvalidLength { actual: 2, .. })
        ));
    }
}
