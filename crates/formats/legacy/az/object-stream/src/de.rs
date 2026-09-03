//! Serde deserialization over an `ObjectStream` element tree.
//!
//! `ObjectStream` keeps AZ type UUIDs on every node. This adapter uses
//! those UUIDs to decode leaf values, while serde owns Rust struct,
//! map, sequence, and default-field behavior.

use std::slice;

use serde::de::{self, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor};

use crate::Element;
use crate::types;
use crate::value::{self, ObjectStreamValueError};

/// Deserialize `T` from a reflected `ObjectStream` element tree.
///
/// # Errors
///
/// Returns [`ObjectStreamValueError::InvalidValue`] when an element's reflected
/// shape has no serde counterpart (a container with the wrong child count, or a
/// leaf whose type UUID this adapter cannot decode),
/// [`ObjectStreamValueError::UnexpectedType`],
/// [`ObjectStreamValueError::MissingData`],
/// [`ObjectStreamValueError::InvalidLength`],
/// [`ObjectStreamValueError::Utf8`] and
/// [`ObjectStreamValueError::IntegerOutOfRange`] from the leaf readers, and
/// [`ObjectStreamValueError::Message`] for anything `T`'s own `Deserialize`
/// implementation rejects (missing fields, unknown variants, failed
/// invariants).
pub fn from_element<'de, T>(element: &'de Element) -> Result<T, ObjectStreamValueError>
where
    T: serde::Deserialize<'de>,
{
    T::deserialize(Deserializer::new(element))
}

/// Deserialize reflected data whose nested concrete classes are represented
/// by serde sum types using a synthetic `$type` UUID discriminator.
///
/// `ObjectStream` stores that discriminator in each element header instead of
/// as a child field. This mode projects the resolved header UUID into struct
/// maps while preserving the normal container semantics for actual maps.
///
/// # Errors
///
/// Returns the same errors as [`from_element`]. The projected `$type`
/// discriminator adds [`ObjectStreamValueError::Message`] from serde when the
/// resolved header UUID matches no variant of the target sum type.
pub fn from_polymorphic_element<'de, T>(element: &'de Element) -> Result<T, ObjectStreamValueError>
where
    T: serde::Deserialize<'de>,
{
    T::deserialize(Deserializer::with_type_tags(element))
}

#[derive(Debug, Clone, Copy)]
pub struct Deserializer<'de> {
    element: &'de Element,
    include_type_tags: bool,
}

impl<'de> Deserializer<'de> {
    #[inline]
    #[must_use]
    pub const fn new(element: &'de Element) -> Self {
        Self {
            element,
            include_type_tags: false,
        }
    }

    #[inline]
    #[must_use]
    pub const fn with_type_tags(element: &'de Element) -> Self {
        Self {
            element,
            include_type_tags: true,
        }
    }

    #[inline]
    const fn nested(self, element: &'de Element) -> Self {
        Self {
            element,
            include_type_tags: self.include_type_tags,
        }
    }

    fn field_name(&self) -> String {
        self.element
            .field()
            .map_or_else(|| "<object>".to_string(), ToString::to_string)
    }

    /// The canonical text form of an `AZ::Uuid` element, or `None` when this
    /// element is not one.
    ///
    /// Native reflects `AZ::Uuid` with its own serializer whose `DataToText`
    /// emits the hyphenated form, so an `AZ::Uuid` slot has a well-defined
    /// string representation distinct from `AZStd::string` storage. Every
    /// other lowering already agrees on it (`deserialize_any` below, and
    /// `SchemaValue::Uuid` in the schema path), but `Deserializer` does not
    /// override `is_human_readable`, so `uuid::Uuid` asks for `deserialize_str`
    /// rather than `deserialize_any` — leaving the two paths inconsistent and
    /// every reflected `AZ::Uuid` field undecodable. Resolve it from the
    /// element's own reflected type, not from the requested Rust type.
    fn az_uuid_text(&self) -> Result<Option<String>, ObjectStreamValueError> {
        if value::semantic_type_id(self.element)? != types::AZ_UUID {
            return Ok(None);
        }
        Ok(Some(
            value::read_uuid(self.element)?.hyphenated().to_string(),
        ))
    }

    fn reflected_enum_signed(
        self,
        target: &'static str,
    ) -> Result<Option<i64>, ObjectStreamValueError> {
        if self.element.reflected_enum_type_id().is_none() {
            return Ok(None);
        }
        match value::read_reflected_enum_discriminant(self.element)? {
            value::ReflectedEnumDiscriminant::Signed(value) => Ok(Some(value)),
            value::ReflectedEnumDiscriminant::Unsigned(value) => i64::try_from(value)
                .map(Some)
                .map_err(|_| ObjectStreamValueError::IntegerOutOfRange {
                    field: self.field_name(),
                    value,
                    target,
                }),
        }
    }

    fn reflected_enum_unsigned(
        self,
        target: &'static str,
    ) -> Result<Option<u64>, ObjectStreamValueError> {
        if self.element.reflected_enum_type_id().is_none() {
            return Ok(None);
        }
        match value::read_reflected_enum_discriminant(self.element)? {
            value::ReflectedEnumDiscriminant::Unsigned(value) => Ok(Some(value)),
            value::ReflectedEnumDiscriminant::Signed(value) => u64::try_from(value)
                .map(Some)
                .map_err(|_| ObjectStreamValueError::IntegerOutOfRange {
                    field: self.field_name(),
                    value: value.cast_unsigned(),
                    target,
                }),
        }
    }
}

impl<'de> de::Deserializer<'de> for Deserializer<'de> {
    type Error = ObjectStreamValueError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        use crate::context::ContainerShape;

        match self.element.container_shape() {
            Some(
                ContainerShape::Sequence
                | ContainerShape::Set
                | ContainerShape::Pair
                | ContainerShape::Tuple,
            ) => {
                return visitor.visit_seq(ChildrenSeqAccess {
                    iter: self.element.children().iter(),
                    include_type_tags: self.include_type_tags,
                });
            }
            Some(ContainerShape::Map) => {
                return visitor.visit_map(PairMapAccess {
                    iter: self.element.children().iter(),
                    value: None,
                    include_type_tags: self.include_type_tags,
                });
            }
            Some(ContainerShape::SmartPointer | ContainerShape::Optional) => {
                return match self.element.children() {
                    [] => visitor.visit_none(),
                    [value] => visitor.visit_some(self.nested(value)),
                    _ => Err(ObjectStreamValueError::InvalidValue {
                        field: self.field_name(),
                        expected: "captured optional/smart-pointer container with zero or one child",
                    }),
                };
            }
            Some(ContainerShape::Variant | ContainerShape::Wrapper) => {
                let [value] = self.element.children() else {
                    return Err(ObjectStreamValueError::InvalidValue {
                        field: self.field_name(),
                        expected: "captured variant/wrapper container with exactly one child",
                    });
                };
                return visitor.visit_newtype_struct(self.nested(value));
            }
            None => {}
        }

        match value::semantic_type_id(self.element)? {
            types::BOOL => self.deserialize_bool(visitor),
            types::CHAR | types::SIGNED_CHAR | types::AZ_S8 => self.deserialize_i8(visitor),
            types::SHORT => self.deserialize_i16(visitor),
            types::INT | types::LONG => self.deserialize_i32(visitor),
            types::AZ_S64 => self.deserialize_i64(visitor),
            types::UNSIGNED_CHAR => self.deserialize_u8(visitor),
            types::UNSIGNED_SHORT => self.deserialize_u16(visitor),
            types::UNSIGNED_INT | types::UNSIGNED_LONG => self.deserialize_u32(visitor),
            types::AZ_U64 => self.deserialize_u64(visitor),
            types::FLOAT | types::VECTOR_FLOAT => self.deserialize_f32(visitor),
            types::DOUBLE => self.deserialize_f64(visitor),
            types::AZSTD_STRING | types::AZSTD_BASIC_STRING | types::AZSTD_STRING_LEGACY_XML => {
                self.deserialize_str(visitor)
            }
            types::AZ_UUID => visitor.visit_string(
                self.az_uuid_text()?
                    .expect("semantic_type_id already proved this element is AZ::Uuid"),
            ),
            types::BYTE_STREAM => self.deserialize_bytes(visitor),
            types::COLOR | types::VECTOR2 | types::VECTOR3 | types::VECTOR4 | types::QUATERNION => {
                self.deserialize_seq(visitor)
            }
            _ if self.element.data().is_none() && self.element.builtin_serializer().is_none() => {
                visitor.visit_map(FieldMapAccess {
                    iter: self.element.children().iter(),
                    value: None,
                    type_tag: self
                        .include_type_tags
                        .then(|| value::semantic_type_id(self.element))
                        .transpose()?,
                    include_type_tags: self.include_type_tags,
                })
            }
            _ => Err(ObjectStreamValueError::InvalidValue {
                field: self.field_name(),
                expected: "serde-supported AZ ObjectStream value",
            }),
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_bool(value::read_bool(self.element)?)
    }

    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(value) = self.reflected_enum_signed("i8")? {
            return visitor.visit_i8(i8::try_from(value).map_err(|_| {
                ObjectStreamValueError::IntegerOutOfRange {
                    field: self.field_name(),
                    value: value.cast_unsigned(),
                    target: "i8",
                }
            })?);
        }
        visitor.visit_i8(value::read_i8(self.element)?)
    }

    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(value) = self.reflected_enum_signed("i16")? {
            return visitor.visit_i16(i16::try_from(value).map_err(|_| {
                ObjectStreamValueError::IntegerOutOfRange {
                    field: self.field_name(),
                    value: value.cast_unsigned(),
                    target: "i16",
                }
            })?);
        }
        visitor.visit_i16(value::read_i16(self.element)?)
    }

    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(value) = self.reflected_enum_signed("i32")? {
            return visitor.visit_i32(i32::try_from(value).map_err(|_| {
                ObjectStreamValueError::IntegerOutOfRange {
                    field: self.field_name(),
                    value: value.cast_unsigned(),
                    target: "i32",
                }
            })?);
        }
        visitor.visit_i32(value::read_i32_scalar(self.element)?)
    }

    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(value) = self.reflected_enum_signed("i64")? {
            return visitor.visit_i64(value);
        }
        let value = match value::semantic_type_id(self.element)? {
            types::LONG => i64::from(value::read_long(self.element)?),
            _ => value::read_i64(self.element)?,
        };
        visitor.visit_i64(value)
    }

    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(value) = self.reflected_enum_unsigned("u8")? {
            return visitor.visit_u8(u8::try_from(value).map_err(|_| {
                ObjectStreamValueError::IntegerOutOfRange {
                    field: self.field_name(),
                    value,
                    target: "u8",
                }
            })?);
        }
        visitor.visit_u8(value::read_u8_scalar(self.element)?)
    }

    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(value) = self.reflected_enum_unsigned("u16")? {
            return visitor.visit_u16(u16::try_from(value).map_err(|_| {
                ObjectStreamValueError::IntegerOutOfRange {
                    field: self.field_name(),
                    value,
                    target: "u16",
                }
            })?);
        }
        visitor.visit_u16(value::read_u16_scalar(self.element)?)
    }

    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(value) = self.reflected_enum_unsigned("u32")? {
            return visitor.visit_u32(u32::try_from(value).map_err(|_| {
                ObjectStreamValueError::IntegerOutOfRange {
                    field: self.field_name(),
                    value,
                    target: "u32",
                }
            })?);
        }
        let value = match value::semantic_type_id(self.element)? {
            types::CRC32 => value::read_crc32(self.element)?,
            types::UNSIGNED_LONG => value::read_unsigned_long(self.element)?,
            _ => value::read_u32_scalar(self.element)?,
        };
        visitor.visit_u32(value)
    }

    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(value) = self.reflected_enum_unsigned("u64")? {
            return visitor.visit_u64(value);
        }
        let value = match value::semantic_type_id(self.element)? {
            types::UNSIGNED_LONG => u64::from(value::read_unsigned_long(self.element)?),
            _ => value::read_u64_scalar(self.element)?,
        };
        visitor.visit_u64(value)
    }

    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_f32(value::read_f32_value(self.element)?)
    }

    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_f64(value::read_f64_scalar(self.element)?)
    }

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let value = value::read_string(self.element)?;
        let mut chars = value.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) => visitor.visit_char(ch),
            _ => Err(ObjectStreamValueError::InvalidValue {
                field: self.field_name(),
                expected: "single character string",
            }),
        }
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(text) = self.az_uuid_text()? {
            return visitor.visit_string(text);
        }
        visitor.visit_borrowed_str(value::read_string(self.element)?)
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(text) = self.az_uuid_text()? {
            return visitor.visit_string(text);
        }
        visitor.visit_string(value::read_string(self.element)?.to_owned())
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_bytes(value::read_byte_stream(self.element)?)
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_byte_buf(value::read_byte_stream(self.element)?.to_vec())
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if matches!(
            self.element.container_shape(),
            Some(
                crate::context::ContainerShape::SmartPointer
                    | crate::context::ContainerShape::Optional
            )
        ) {
            return match self.element.children() {
                [] => visitor.visit_none(),
                [value] => visitor.visit_some(self.nested(value)),
                _ => Err(ObjectStreamValueError::InvalidValue {
                    field: self.field_name(),
                    expected: "captured optional/smart-pointer container with zero or one child",
                }),
            };
        }
        visitor.visit_some(self)
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if matches!(
            self.element.container_shape(),
            Some(
                crate::context::ContainerShape::SmartPointer
                    | crate::context::ContainerShape::Wrapper
            )
        ) {
            let [value] = self.element.children() else {
                return Err(ObjectStreamValueError::InvalidValue {
                    field: self.field_name(),
                    expected: "captured smart-pointer/wrapper container with exactly one child",
                });
            };
            return visitor.visit_newtype_struct(self.nested(value));
        }
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(values) = scalar_f32_sequence(self.element)? {
            return visitor.visit_seq(F32SeqAccess::new(values));
        }

        value::require_container_shape(
            self.element,
            crate::context::ContainerShape::Sequence,
            "captured sequence IDataContainer",
        )?;

        visitor.visit_seq(ChildrenSeqAccess {
            iter: self.element.children().iter(),
            include_type_tags: self.include_type_tags,
        })
    }

    fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(values) = scalar_f32_sequence(self.element)? {
            return visitor.visit_seq(F32SeqAccess::new(values));
        }
        let valid_shape = match self.element.container_shape() {
            Some(crate::context::ContainerShape::Tuple) => true,
            Some(crate::context::ContainerShape::Pair) => len == 2,
            _ => false,
        };
        if !valid_shape || self.element.children().len() != len {
            return Err(ObjectStreamValueError::InvalidValue {
                field: self.field_name(),
                expected: "captured tuple/pair IDataContainer with the requested fixed arity",
            });
        }
        visitor.visit_seq(ChildrenSeqAccess {
            iter: self.element.children().iter(),
            include_type_tags: self.include_type_tags,
        })
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_tuple(len, visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self.include_type_tags
            && self.element.container_shape() != Some(crate::context::ContainerShape::Map)
        {
            return visitor.visit_map(FieldMapAccess {
                iter: self.element.children().iter(),
                value: None,
                type_tag: Some(value::semantic_type_id(self.element)?),
                include_type_tags: true,
            });
        }
        value::require_container_shape(
            self.element,
            crate::context::ContainerShape::Map,
            "captured map IDataContainer",
        )?;
        visitor.visit_map(PairMapAccess {
            iter: self.element.children().iter(),
            value: None,
            include_type_tags: self.include_type_tags,
        })
    }

    fn deserialize_struct<V>(
        self,
        name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if let Some(values) = scalar_struct_fields(name, self.element)? {
            return visitor.visit_map(values);
        }

        visitor.visit_map(FieldMapAccess {
            iter: self.element.children().iter(),
            value: None,
            type_tag: self
                .include_type_tags
                .then(|| value::semantic_type_id(self.element))
                .transpose()?,
            include_type_tags: self.include_type_tags,
        })
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(ObjectStreamValueError::InvalidValue {
            field: self.field_name(),
            expected: "generated enum #[serde(try_from = \"<captured underlying integer>\", into = \"<captured underlying integer>\")]; direct Serde enum ordinals are not native discriminants",
        })
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    serde::forward_to_deserialize_any! {
        i128 u128
    }
}

struct FieldMapAccess<'de> {
    iter: slice::Iter<'de, Element>,
    value: Option<FieldMapValue<'de>>,
    type_tag: Option<uuid::Uuid>,
    include_type_tags: bool,
}

enum FieldMapValue<'de> {
    TypeTag(uuid::Uuid),
    Element(&'de Element),
}

impl<'de> MapAccess<'de> for FieldMapAccess<'de> {
    type Error = ObjectStreamValueError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        if let Some(type_tag) = self.type_tag.take() {
            self.value = Some(FieldMapValue::TypeTag(type_tag));
            return seed.deserialize("$type".into_deserializer()).map(Some);
        }
        let Some(element) = self.iter.next() else {
            return Ok(None);
        };
        let Some(field) = element.field() else {
            return Err(ObjectStreamValueError::InvalidValue {
                field: "<object>".to_string(),
                expected: "named ObjectStream child field",
            });
        };

        self.value = Some(FieldMapValue::Element(element));
        seed.deserialize(field.as_str().into_deserializer())
            .map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let value = self.value.take().ok_or_else(|| {
            ObjectStreamValueError::Message("serde requested a value before a key".to_string())
        })?;
        match value {
            FieldMapValue::TypeTag(type_id) => {
                seed.deserialize(type_id.to_string().into_deserializer())
            }
            FieldMapValue::Element(element) => seed.deserialize(Deserializer {
                element,
                include_type_tags: self.include_type_tags,
            }),
        }
    }
}

struct PairMapAccess<'de> {
    iter: slice::Iter<'de, Element>,
    value: Option<&'de Element>,
    include_type_tags: bool,
}

impl<'de> MapAccess<'de> for PairMapAccess<'de> {
    type Error = ObjectStreamValueError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        let Some(pair) = self.iter.next() else {
            return Ok(None);
        };
        value::require_container_shape(
            pair,
            crate::context::ContainerShape::Pair,
            "captured AZStd::pair IDataContainer",
        )?;
        let [key, value] = pair.children() else {
            return Err(ObjectStreamValueError::UnexpectedType {
                field: pair
                    .field()
                    .map_or_else(|| "<map pair>".to_string(), ToString::to_string),
                expected: "AZStd::pair with key and value children",
                actual: value::semantic_type_id(pair)?,
            });
        };

        self.value = Some(value);
        seed.deserialize(Deserializer {
            element: key,
            include_type_tags: self.include_type_tags,
        })
        .map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let element = self.value.take().ok_or_else(|| {
            ObjectStreamValueError::Message("serde requested a value before a key".to_string())
        })?;
        seed.deserialize(Deserializer {
            element,
            include_type_tags: self.include_type_tags,
        })
    }
}

struct ChildrenSeqAccess<'de> {
    iter: slice::Iter<'de, Element>,
    include_type_tags: bool,
}

impl<'de> SeqAccess<'de> for ChildrenSeqAccess<'de> {
    type Error = ObjectStreamValueError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        self.iter
            .next()
            .map(|element| {
                seed.deserialize(Deserializer {
                    element,
                    include_type_tags: self.include_type_tags,
                })
            })
            .transpose()
    }
}

struct F32SeqAccess {
    values: Vec<f32>,
    index: usize,
}

impl F32SeqAccess {
    fn new(values: impl IntoIterator<Item = f32>) -> Self {
        Self {
            values: values.into_iter().collect(),
            index: 0,
        }
    }
}

impl<'de> SeqAccess<'de> for F32SeqAccess {
    type Error = ObjectStreamValueError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        let Some(value) = self.values.get(self.index).copied() else {
            return Ok(None);
        };
        self.index += 1;
        seed.deserialize(value.into_deserializer()).map(Some)
    }
}

struct NamedF32Access {
    fields: &'static [&'static str],
    values: Vec<f32>,
    index: usize,
}

impl NamedF32Access {
    fn new(fields: &'static [&'static str], values: impl IntoIterator<Item = f32>) -> Self {
        Self {
            fields,
            values: values.into_iter().collect(),
            index: 0,
        }
    }
}

impl<'de> MapAccess<'de> for NamedF32Access {
    type Error = ObjectStreamValueError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        let Some(field) = self.fields.get(self.index) else {
            return Ok(None);
        };
        seed.deserialize((*field).into_deserializer()).map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let Some(value) = self.values.get(self.index).copied() else {
            return Err(ObjectStreamValueError::Message(
                "serde requested too many scalar struct values".to_string(),
            ));
        };
        self.index += 1;
        seed.deserialize(value.into_deserializer())
    }
}

fn scalar_struct_fields(
    name: &'static str,
    element: &Element,
) -> Result<Option<NamedF32Access>, ObjectStreamValueError> {
    if matches!(name, "LinearRgba" | "Srgba") {
        return scalar_f32_sequence(element).map(|values| {
            values.map(|values| NamedF32Access::new(&["red", "green", "blue", "alpha"], values))
        });
    }

    Ok(None)
}

fn scalar_f32_sequence(element: &Element) -> Result<Option<Vec<f32>>, ObjectStreamValueError> {
    match value::semantic_type_id(element)? {
        types::COLOR | types::VECTOR4 | types::QUATERNION => {
            Ok(Some(value::read_float4(element)?.to_vec()))
        }
        types::VECTOR3 => Ok(Some(value::read_vec3(element)?.to_vec())),
        types::VECTOR2 => Ok(Some(value::read_vec2(element)?.to_vec())),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for the 80,366-asset `.timeline` failure.
    ///
    /// `TimelineLayer` registers `Id` as an `AZ::Uuid`, while `CAnimNode`,
    /// `CAnimSequence`, `CUiAnimNode`, and `CUiAnimSequence` register `ID` on
    /// the same case-folded CRC. The global CRC name table holds one spelling
    /// (`ID`), so before per-class stamping every `TimelineLayer::Id` element
    /// arrived misnamed, serde treated it as an unknown key, and
    /// `deserialize_ignored_any` discarded the payload into
    /// `#[serde(default)]` — a silent nil UUID in every shipped timeline.
    ///
    /// Once the name is correct the value actually reaches `uuid::Uuid`, which
    /// requests `deserialize_str`. This asserts both halves: the registration
    /// spelling wins the name, *and* the `AZ::Uuid` payload survives into the
    /// typed field.
    #[test]
    fn az_uuid_field_named_by_its_class_survives_into_a_typed_uuid() {
        use crate::context::{
            ObjectStreamDialect, ObjectStreamReadContext, ReflectedClass, ReflectedField,
        };
        use crate::lookup::LumberyardHashes;

        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct TimelineLayer {
            #[serde(rename = "Id", default)]
            id: uuid::Uuid,
            #[serde(rename = "Name", default)]
            name: String,
        }

        let id_crc = crate::field_name_crc("Id");
        assert_eq!(id_crc, crate::field_name_crc("ID"));

        let layer_id = uuid::Uuid::from_u128(0xA001);
        let mut layer = ReflectedClass::new(layer_id).with_name("TimelineLayer");
        layer.insert_named_field("Id", ReflectedField::new(types::AZ_UUID));
        layer.insert_named_field("Name", ReflectedField::new(types::AZSTD_STRING));

        // The shared table carries the sibling `CAnimNode::ID` spelling.
        let mut names = LumberyardHashes::new();
        names.extend_field_names(["ID"]);
        let mut context = ObjectStreamReadContext::new(names, ObjectStreamDialect::default());
        context
            .insert_class(types::AZ_UUID, ReflectedClass::new(types::AZ_UUID))
            .unwrap();
        context
            .insert_class(
                types::AZSTD_STRING,
                ReflectedClass::new(types::AZSTD_STRING),
            )
            .unwrap();
        let layer_key = context.insert_class(layer_id, layer).unwrap();

        let expected = uuid::Uuid::from_u128(0x7886_6504_EF2B_4518_A4DE_E3E3_4867_233A);
        let mut id_child = leaf("ID", types::AZ_UUID, expected.as_bytes().to_vec());
        id_child.name_crc = Some(id_crc);
        context
            .finalize_reachable_child(layer_key, &mut id_child, 3)
            .unwrap();
        assert_eq!(
            id_child.field().map(ToString::to_string).as_deref(),
            Some("Id"),
            "the owning class registration outranks the shared table"
        );

        let mut name_child = leaf("Name", types::AZSTD_STRING, b"PlayAnim".to_vec());
        name_child.name_crc = Some(crate::field_name_crc("Name"));
        context
            .finalize_reachable_child(layer_key, &mut name_child, 3)
            .unwrap();

        let element = Element::new(layer_id).with_children([id_child, name_child]);
        let decoded: TimelineLayer = from_element(&element).unwrap();
        assert_eq!(
            decoded,
            TimelineLayer {
                id: expected,
                name: "PlayAnim".to_owned(),
            }
        );
        assert_ne!(decoded.id, uuid::Uuid::nil(), "must not silently default");
    }

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Appearance {
        #[serde(rename = "isSkin", default)]
        is_skin: bool,
        #[serde(rename = "faceMarkColor", default)]
        face_mark_color: bevy_color::LinearRgba,
    }

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Dye {
        #[serde(rename = "m_rColorId", default)]
        r_color_id: u8,
    }

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Equipment {
        #[serde(rename = "appearanceId", default)]
        appearance_id: az_core::crc::Crc32,
    }

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct WrappedCount(u32);

    #[derive(Debug, PartialEq, Eq)]
    struct PolymorphicValue {
        type_id: uuid::Uuid,
        value: u32,
    }

    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct NestedBase {
        #[serde(rename = "StartTime")]
        start_time: f32,
        #[serde(rename = "Id")]
        id: uuid::Uuid,
    }

    #[derive(Debug, PartialEq)]
    struct BufferedPolymorphicValue {
        type_id: uuid::Uuid,
        base: NestedBase,
    }

    impl<'de> serde::Deserialize<'de> for BufferedPolymorphicValue {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut fields =
                <serde_json::Map<String, serde_json::Value> as serde::Deserialize>::deserialize(
                    deserializer,
                )?;
            let type_id = fields
                .remove("$type")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| serde::de::Error::missing_field("$type"))
                .and_then(|value| {
                    uuid::Uuid::parse_str(&value).map_err(serde::de::Error::custom)
                })?;
            let base = fields
                .remove("BaseClass1")
                .ok_or_else(|| serde::de::Error::missing_field("BaseClass1"))
                .and_then(|value| {
                    serde_json::from_value(value).map_err(serde::de::Error::custom)
                })?;
            Ok(Self { type_id, base })
        }
    }

    impl<'de> serde::Deserialize<'de> for PolymorphicValue {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            struct PolymorphicVisitor;

            impl<'de> serde::de::Visitor<'de> for PolymorphicVisitor {
                type Value = PolymorphicValue;

                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("ObjectStream object with a $type discriminator")
                }

                fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                where
                    A: serde::de::MapAccess<'de>,
                {
                    let mut type_id = None;
                    let mut value = None;
                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "$type" => {
                                let text = map.next_value::<String>()?;
                                type_id = Some(
                                    uuid::Uuid::parse_str(&text)
                                        .map_err(serde::de::Error::custom)?,
                                );
                            }
                            "Value" => value = Some(map.next_value()?),
                            _ => {
                                map.next_value::<serde::de::IgnoredAny>()?;
                            }
                        }
                    }
                    Ok(PolymorphicValue {
                        type_id: type_id.ok_or_else(|| serde::de::Error::missing_field("$type"))?,
                        value: value.ok_or_else(|| serde::de::Error::missing_field("Value"))?,
                    })
                }
            }

            deserializer.deserialize_map(PolymorphicVisitor)
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
    #[serde(try_from = "u8")]
    enum CapturedFaction {
        None,
        Faction1,
    }

    impl TryFrom<u8> for CapturedFaction {
        type Error = u8;

        fn try_from(value: u8) -> Result<Self, Self::Error> {
            match value {
                0 => Ok(Self::None),
                1 => Ok(Self::Faction1),
                value => Err(value),
            }
        }
    }

    #[derive(Debug, PartialEq, Eq, serde::Deserialize)]
    struct CapturedPlayer {
        #[serde(rename = "startingFaction")]
        starting_faction: CapturedFaction,
    }

    fn leaf(field: &str, type_id: uuid::Uuid, data: impl Into<Vec<u8>>) -> Element {
        let element = Element::new(type_id).with_field(field).with_data(data);
        if let Some(kind) = crate::codec::builtin_serializer_kind(type_id) {
            element.with_builtin_serializer(crate::codec::BuiltinSerializerDescriptor::new(kind, 0))
        } else {
            element
        }
    }

    fn floats(values: [f32; 4]) -> Vec<u8> {
        values.into_iter().flat_map(f32::to_be_bytes).collect()
    }

    fn captured_enum_stream(
        value: u8,
    ) -> (crate::ObjectStream, crate::context::ObjectStreamReadContext) {
        let parent_id = uuid::Uuid::from_u128(0x100);
        let enum_id = uuid::Uuid::parse_str("3983D142-5E97-42E5-AD7D-9EADC6C2C896").unwrap();
        let mut parent = crate::context::ReflectedClass::new(parent_id);
        parent.insert_field(
            crate::field_name_crc("startingFaction"),
            crate::context::ReflectedField::new(enum_id),
        );
        let mut context = crate::context::ObjectStreamReadContext::default();
        context.insert_class(parent_id, parent).unwrap();
        let underlying = context
            .insert_class(
                types::UNSIGNED_CHAR,
                crate::context::ReflectedClass::new(types::UNSIGNED_CHAR),
            )
            .unwrap();
        context
            .insert_builtin_codec(underlying, types::UNSIGNED_CHAR, 0)
            .unwrap();
        context
            .insert_enum_underlying_type(enum_id, types::UNSIGNED_CHAR)
            .unwrap();
        let json = format!(
            "{{\"name\":\"ObjectStream\",\"version\":3,\"Objects\":[{{\"typeId\":\"{{{parent_id}}}\",\"Objects\":[{{\"field\":\"startingFaction\",\"typeId\":\"{{{enum_id}}}\",\"value\":\"{value}\"}}]}}]}}"
        );
        let stream =
            crate::ObjectStream::from_bytes_with_context(json.as_bytes(), &context).unwrap();
        (stream, context)
    }

    #[test]
    fn captured_enum_underlying_drives_generated_try_from_contract() {
        let (stream, _context) = captured_enum_stream(1);
        let value: CapturedPlayer = from_element(&stream.elements()[0]).unwrap();
        assert_eq!(value.starting_faction, CapturedFaction::Faction1);

        let (stream, _context) = captured_enum_stream(7);
        assert!(from_element::<CapturedPlayer>(&stream.elements()[0]).is_err());
    }

    #[test]
    fn decodes_struct_with_source_field_renames_and_defaulted_fields() {
        let element = Element::new(uuid::Uuid::nil()).with_children([
            leaf("isSkin", types::BOOL, [1]),
            leaf("faceMarkColor", types::COLOR, floats([0.1, 0.2, 0.3, 0.4])),
        ]);

        let value: Appearance = from_element(&element).unwrap();

        assert_eq!(
            value,
            Appearance {
                is_skin: true,
                face_mark_color: bevy_color::LinearRgba::new(0.1, 0.2, 0.3, 0.4),
            }
        );
    }

    #[test]
    fn polymorphic_mode_projects_element_type_header_into_sum_type_map() {
        let concrete_type = uuid::Uuid::from_u128(0x1234);
        let element = Element::new(concrete_type)
            .with_test_class()
            .with_children([leaf("Value", types::UNSIGNED_INT, 42_u32.to_be_bytes())]);

        let value: PolymorphicValue = from_polymorphic_element(&element).unwrap();

        assert_eq!(
            value,
            PolymorphicValue {
                type_id: concrete_type,
                value: 42,
            }
        );
        assert!(from_element::<PolymorphicValue>(&element).is_err());
    }

    #[test]
    fn polymorphic_buffer_preserves_nested_reflected_class_maps() {
        let concrete_type = uuid::Uuid::from_u128(0x1234);
        let base_type = uuid::Uuid::from_u128(0x5678);
        let element = Element::new(concrete_type)
            .with_test_class()
            .with_children([Element::new(base_type)
                .with_test_class()
                .with_test_base_class_edge()
                .with_field("BaseClass1")
                .with_children([
                    leaf("StartTime", types::FLOAT, 1.25_f32.to_be_bytes()),
                    leaf(
                        "Id",
                        types::AZ_UUID,
                        uuid::Uuid::from_u128(0x9abc).as_bytes().to_vec(),
                    ),
                ])]);

        let value: BufferedPolymorphicValue = from_polymorphic_element(&element).unwrap();

        assert_eq!(
            value,
            BufferedPolymorphicValue {
                type_id: concrete_type,
                base: NestedBase {
                    start_time: 1.25,
                    id: uuid::Uuid::from_u128(0x9abc),
                },
            }
        );
    }

    #[test]
    fn deserialize_any_projects_captured_container_families_without_guessing() {
        let sequence = Element::new(uuid::Uuid::from_u128(100))
            .with_container_shape(crate::context::ContainerShape::Sequence)
            .with_children([
                leaf("element", types::UNSIGNED_INT, 3_u32.to_be_bytes()),
                leaf("element", types::UNSIGNED_INT, 5_u32.to_be_bytes()),
            ]);
        assert_eq!(
            from_element::<serde_json::Value>(&sequence).unwrap(),
            serde_json::json!([3, 5])
        );

        let pair = Element::new(uuid::Uuid::from_u128(101))
            .with_container_shape(crate::context::ContainerShape::Pair)
            .with_children([
                leaf("value1", types::AZSTD_STRING, b"key"),
                leaf("value2", types::UNSIGNED_INT, 7_u32.to_be_bytes()),
            ]);
        let map = Element::new(uuid::Uuid::from_u128(102))
            .with_container_shape(crate::context::ContainerShape::Map)
            .with_children([pair]);
        assert_eq!(
            from_element::<serde_json::Value>(&map).unwrap(),
            serde_json::json!({"key": 7})
        );

        let optional = Element::new(uuid::Uuid::from_u128(103))
            .with_container_shape(crate::context::ContainerShape::Optional)
            .with_children([leaf("element", types::UNSIGNED_INT, 11_u32.to_be_bytes())]);
        assert_eq!(
            from_element::<serde_json::Value>(&optional).unwrap(),
            serde_json::json!(11)
        );
    }

    #[test]
    fn decodes_objectstream_pair_maps() {
        let pair = Element::new(uuid::Uuid::nil())
            .with_container_shape(crate::context::ContainerShape::Pair)
            .with_children([
                leaf("value1", types::AZSTD_STRING, b"slot"),
                leaf("value2", types::AZSTD_STRING, b"item"),
            ]);
        let element = Element::new(uuid::Uuid::nil())
            .with_container_shape(crate::context::ContainerShape::Map)
            .with_children([pair]);

        let value: std::collections::HashMap<String, String> = from_element(&element).unwrap();

        assert_eq!(value.get("slot").map(String::as_str), Some("item"));
    }

    #[test]
    fn map_arity_cannot_replace_associative_and_pair_shape_proof() {
        let unproven_pair = Element::new(uuid::Uuid::from_u128(102)).with_children([
            leaf("value1", types::AZSTD_STRING, b"slot"),
            leaf("value2", types::AZSTD_STRING, b"item"),
        ]);
        let associative = Element::new(uuid::Uuid::from_u128(103))
            .with_container_shape(crate::context::ContainerShape::Map)
            .with_children([unproven_pair]);
        assert!(matches!(
            from_element::<std::collections::HashMap<String, String>>(&associative),
            Err(ObjectStreamValueError::UnexpectedContainerShape { .. })
        ));

        let pair = Element::new(uuid::Uuid::from_u128(102))
            .with_container_shape(crate::context::ContainerShape::Pair)
            .with_children([
                leaf("value1", types::AZSTD_STRING, b"slot"),
                leaf("value2", types::AZSTD_STRING, b"item"),
            ]);
        let unproven_map = Element::new(uuid::Uuid::from_u128(103)).with_children([pair]);
        assert!(matches!(
            from_element::<std::collections::HashMap<String, String>>(&unproven_map),
            Err(ObjectStreamValueError::UnexpectedContainerShape { .. })
        ));
    }

    #[test]
    fn smart_pointer_shape_drives_optional_and_newtype_unwrap() {
        let empty = Element::new(uuid::Uuid::from_u128(100))
            .with_container_shape(crate::context::ContainerShape::SmartPointer);
        let absent: Option<u32> = from_element(&empty).unwrap();
        assert_eq!(absent, None);

        let present = Element::new(uuid::Uuid::from_u128(100))
            .with_container_shape(crate::context::ContainerShape::SmartPointer)
            .with_children([leaf("element", types::UNSIGNED_INT, 7_u32.to_be_bytes())]);
        let value: Option<u32> = from_element(&present).unwrap();
        assert_eq!(value, Some(7));
        let wrapped: WrappedCount = from_element(&present).unwrap();
        assert_eq!(wrapped, WrappedCount(7));
    }

    #[test]
    fn smart_pointer_rejects_multiple_children_and_arity_cannot_fake_shape() {
        let invalid = Element::new(uuid::Uuid::from_u128(101))
            .with_container_shape(crate::context::ContainerShape::SmartPointer)
            .with_children([
                leaf("element", types::UNSIGNED_INT, 1_u32.to_be_bytes()),
                leaf("element", types::UNSIGNED_INT, 2_u32.to_be_bytes()),
            ]);
        assert!(matches!(
            from_element::<Option<u32>>(&invalid),
            Err(ObjectStreamValueError::InvalidValue { .. })
        ));

        let unproven = Element::new(uuid::Uuid::from_u128(101)).with_children([leaf(
            "element",
            types::UNSIGNED_INT,
            3_u32.to_be_bytes(),
        )]);
        assert!(from_element::<WrappedCount>(&unproven).is_err());
    }

    #[test]
    fn decodes_ranged_integer_leaf_as_scalar_value() {
        let ranged_u8 = az_core::uuid::azstd_ranged_int(az_core::uuid::type_ids::U8, 0, 255);
        let element =
            Element::new(uuid::Uuid::nil()).with_children([leaf("m_rColorId", ranged_u8, [7])
                .with_builtin_serializer(crate::codec::BuiltinSerializerDescriptor::new(
                    crate::codec::BuiltinSerializerKind::RangedUnsigned { bytes: 1 },
                    0,
                ))]);

        let value: Dye = from_element(&element).unwrap();

        assert_eq!(value.r_color_id, 7);
    }

    #[test]
    fn decodes_crc32_object_as_transparent_u32_newtype() {
        let crc = Element::new(types::CRC32)
            .with_test_class()
            .with_field("appearanceId")
            .with_children([leaf(
                "value",
                types::UNSIGNED_INT,
                0x1234_5678_u32.to_be_bytes(),
            )]);
        let element = Element::new(uuid::Uuid::nil()).with_children([crc]);

        let value: Equipment = from_element(&element).unwrap();

        assert_eq!(value.appearance_id.value(), 0x1234_5678);
    }
}
