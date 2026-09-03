//! Serde-like `ObjectStream` object traversal.
//!
//! The `value` module owns leaf payload conversion. This module owns
//! object-shaped field traversal so callers describe field intent instead of
//! manually driving `FieldCursor`.

use uuid::Uuid;

use crate::Element;
use crate::value::{DecodeAzValue, FieldCursor, ObjectStreamValueError};

pub trait Deserialize<'a>: Sized {
    /// Read `Self` out of the object's remaining fields.
    ///
    /// # Errors
    ///
    /// Implementation-defined, but every implementation in this crate reports
    /// [`ObjectStreamValueError::MissingField`] for a required field that is not
    /// present and propagates the leaf-decode error of each field it does read.
    /// Implementations must not consume fields they do not understand — [`ObjectFields::finish`]
    /// turns leftovers into [`ObjectStreamValueError::UnknownField`].
    fn deserialize(fields: &mut ObjectFields<'a>) -> Result<Self, ObjectStreamValueError>;
}

pub trait Serialize {
    fn serialize(&self) -> Element;
}

pub fn serialize<T>(value: &T) -> Element
where
    T: Serialize + ?Sized,
{
    value.serialize()
}

#[derive(Debug, Clone)]
pub struct ObjectFields<'a> {
    fields: FieldCursor<'a>,
    path: String,
}

impl<'a> ObjectFields<'a> {
    #[inline]
    #[must_use]
    pub fn from_element(element: &'a Element) -> Self {
        Self::from_element_at(
            element,
            element.field().map_or("<object>", |field| field.as_str()),
        )
    }

    #[inline]
    #[must_use]
    pub fn from_element_at(element: &'a Element, path: impl Into<String>) -> Self {
        Self {
            fields: FieldCursor::from_element(element),
            path: path.into(),
        }
    }

    /// Read a required field and decode it as `T`.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamValueError::MissingField`] with the dotted field path
    /// if no remaining child carries `field`, otherwise any error `T`'s
    /// [`DecodeAzValue`] implementation returns for that child.
    pub fn required<T>(&mut self, field: &str) -> Result<T, ObjectStreamValueError>
    where
        T: DecodeAzValue<'a>,
    {
        let element = self.required_element(field)?;
        T::decode_az_value(element)
    }

    /// Read a required field and decode it with `read`.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamValueError::MissingField`] with the dotted field path
    /// if no remaining child carries `field`, otherwise whatever `read` returns.
    pub fn required_with<T>(
        &mut self,
        field: &str,
        read: impl FnOnce(&'a Element) -> Result<T, ObjectStreamValueError>,
    ) -> Result<T, ObjectStreamValueError> {
        read(self.required_element(field)?)
    }

    /// Read an optional field, decoding it as `T` when present.
    ///
    /// # Errors
    ///
    /// An absent field is `Ok(None)`, not an error. When the field is present,
    /// returns any error `T`'s [`DecodeAzValue`] implementation returns.
    pub fn optional<T>(&mut self, field: &str) -> Result<Option<T>, ObjectStreamValueError>
    where
        T: DecodeAzValue<'a>,
    {
        self.fields.find(field).map(T::decode_az_value).transpose()
    }

    /// Read an optional field, falling back to `T::default()` when absent.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::optional`] returns; the missing-field case is
    /// resolved to the default rather than reported.
    pub fn defaulted<T>(&mut self, field: &str) -> Result<T, ObjectStreamValueError>
    where
        T: DecodeAzValue<'a> + Default,
    {
        self.optional(field).map(Option::unwrap_or_default)
    }

    /// Advance the cursor to the child named `field` and borrow it undecoded.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamValueError::MissingField`] carrying the dotted field
    /// path if no remaining child matches `field`.
    pub fn required_element(&mut self, field: &str) -> Result<&'a Element, ObjectStreamValueError> {
        self.fields
            .find(field)
            .ok_or_else(|| ObjectStreamValueError::MissingField {
                field: self.field_path(field),
            })
    }

    pub fn optional_element(&mut self, field: &str) -> Option<&'a Element> {
        self.fields.find(field)
    }

    /// Read a required field as a nested object and require it fully consumed.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamValueError::MissingField`] if `field` is absent, any
    /// error `T::deserialize` returns, and
    /// [`ObjectStreamValueError::UnknownField`] if `T::deserialize` leaves any of
    /// the nested object's children unread.
    pub fn object<T>(&mut self, field: &str) -> Result<T, ObjectStreamValueError>
    where
        T: Deserialize<'a>,
    {
        let element = self.required_element(field)?;
        let mut fields = ObjectFields::from_element_at(element, self.field_path(field));
        let value = T::deserialize(&mut fields)?;
        fields.finish()?;
        Ok(value)
    }

    /// Read a required field as a nested object decoded by `read`.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamValueError::MissingField`] if `field` is absent,
    /// otherwise whatever `read` returns. Unlike [`Self::object`] this does not
    /// require `read` to consume every child.
    pub fn object_with<T>(
        &mut self,
        field: &str,
        read: impl FnOnce(&'a Element) -> Result<T, ObjectStreamValueError>,
    ) -> Result<T, ObjectStreamValueError> {
        read(self.required_element(field)?)
    }

    /// Read a required sequence container and decode each item with `read`.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamValueError::MissingField`] if `field` is absent,
    /// [`ObjectStreamValueError::UnexpectedContainerShape`] if the container's
    /// reflected family is not a sequence,
    /// [`ObjectStreamValueError::UnexpectedType`] for any child whose semantic type
    /// is not `item_type`, and whatever `read` returns for a child it rejects.
    pub fn list_with<T>(
        &mut self,
        field: &str,
        item_type: Uuid,
        mut read: impl FnMut(&'a Element) -> Result<T, ObjectStreamValueError>,
    ) -> Result<Vec<T>, ObjectStreamValueError> {
        let container = self.required_element(field)?;
        crate::value::require_container_shape(
            container,
            crate::context::ContainerShape::Sequence,
            "captured sequence IDataContainer",
        )?;
        container
            .children()
            .iter()
            .map(|child| {
                ensure_type(child, item_type, "requested list item type")?;
                read(child)
            })
            .collect()
    }

    /// Read a required map container and decode each key/value pair.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamValueError::MissingField`] if `field` is absent,
    /// [`ObjectStreamValueError::UnexpectedContainerShape`] if the container's
    /// reflected family is not a map or if an entry is not a reflected
    /// `AZStd::pair`, [`ObjectStreamValueError::UnexpectedType`] if a pair does not
    /// hold exactly a key and a value child, and whatever `read_key` or
    /// `read_value` returns.
    pub fn map_with<K, V>(
        &mut self,
        field: &str,
        mut read_key: impl FnMut(&'a Element) -> Result<K, ObjectStreamValueError>,
        mut read_value: impl FnMut(&'a Element) -> Result<V, ObjectStreamValueError>,
    ) -> Result<Vec<(K, V)>, ObjectStreamValueError> {
        let container = self.required_element(field)?;
        crate::value::require_container_shape(
            container,
            crate::context::ContainerShape::Map,
            "captured map IDataContainer",
        )?;
        container
            .children()
            .iter()
            .map(|pair| {
                let (key, value) = map_pair(pair)?;
                Ok((read_key(key)?, read_value(value)?))
            })
            .collect()
    }

    /// Assert that every child of the object was consumed.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamValueError::UnknownField`] naming the first child left
    /// on the cursor, using the dotted field path (or `<unnamed>` for a child with
    /// no field name).
    pub fn finish(self) -> Result<(), ObjectStreamValueError> {
        if let Some(element) = self.fields.remaining().first() {
            return Err(ObjectStreamValueError::UnknownField {
                field: self.field_path(element.field().map_or("<unnamed>", |field| field.as_str())),
            });
        }
        Ok(())
    }

    fn field_path(&self, field: &str) -> String {
        if self.path == "<object>" {
            field.to_string()
        } else {
            format!("{}.{}", self.path, field)
        }
    }
}

/// Decode `element` as `T`, requiring every child to be consumed.
///
/// # Errors
///
/// Returns any error `T::deserialize` returns, plus
/// [`ObjectStreamValueError::UnknownField`] if the implementation leaves a
/// child unread.
pub fn deserialize<'a, T>(element: &'a Element) -> Result<T, ObjectStreamValueError>
where
    T: Deserialize<'a>,
{
    let mut fields = ObjectFields::from_element(element);
    let value = T::deserialize(&mut fields)?;
    fields.finish()?;
    Ok(value)
}

/// Split a reflected `AZStd::pair` element into its key and value children.
///
/// # Errors
///
/// Returns [`ObjectStreamValueError::UnexpectedContainerShape`] if `pair` is
/// not a reflected pair container, or
/// [`ObjectStreamValueError::UnexpectedType`] if it does not hold exactly two
/// children.
pub fn map_pair(pair: &Element) -> Result<(&Element, &Element), ObjectStreamValueError> {
    crate::value::require_container_shape(
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
            actual: crate::value::semantic_type_id(pair)?,
        });
    };
    Ok((key, value))
}

/// Require `element`'s semantic type to be exactly `expected`.
///
/// # Errors
///
/// Returns [`ObjectStreamValueError::UnexpectedType`] carrying `expected_name`
/// and the observed UUID when the semantic type differs.
pub fn ensure_type(
    element: &Element,
    expected: Uuid,
    expected_name: &'static str,
) -> Result<(), ObjectStreamValueError> {
    let actual = crate::value::semantic_type_id(element)?;
    if actual == expected {
        Ok(())
    } else {
        Err(ObjectStreamValueError::UnexpectedType {
            field: element
                .field()
                .map_or_else(|| "<object>".to_string(), ToString::to_string),
            expected: expected_name,
            actual,
        })
    }
}

/// Decode a map key that is a reflected `AZStd::string`.
///
/// # Errors
///
/// Returns any error the `Box<str>` [`DecodeAzValue`] implementation returns —
/// [`ObjectStreamValueError::UnexpectedType`] for a non-string element,
/// [`ObjectStreamValueError::MissingData`] for one with no payload, and
/// [`ObjectStreamValueError::Utf8`] for bytes that are not valid UTF-8.
pub fn string_key(element: &Element) -> Result<Box<str>, ObjectStreamValueError> {
    element.decode()
}

/// Decode every child of a reflected sequence container with `read`.
///
/// # Errors
///
/// Returns [`ObjectStreamValueError::UnexpectedContainerShape`] if `element`'s
/// reflected family is not a sequence,
/// [`ObjectStreamValueError::UnexpectedType`] for any child whose semantic type
/// is not `item_type`, and whatever `read` returns for a child it rejects.
pub fn object_vector<'a, T>(
    element: &'a Element,
    item_type: Uuid,
    mut read: impl FnMut(&'a Element) -> Result<T, ObjectStreamValueError>,
) -> Result<Vec<T>, ObjectStreamValueError> {
    crate::value::require_container_shape(
        element,
        crate::context::ContainerShape::Sequence,
        "captured sequence IDataContainer",
    )?;
    element
        .children()
        .iter()
        .map(|child| {
            ensure_type(child, item_type, "requested vector item type")?;
            read(child)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types;

    #[derive(Debug, PartialEq)]
    struct Shape {
        name: Box<str>,
        enabled: bool,
        count: u32,
    }

    #[derive(Debug, PartialEq)]
    struct Container {
        shape: Shape,
    }

    impl<'a> Deserialize<'a> for Shape {
        fn deserialize(fields: &mut ObjectFields<'a>) -> Result<Self, ObjectStreamValueError> {
            Ok(Self {
                name: fields.required("Name")?,
                enabled: fields.defaulted("Enabled")?,
                count: fields.required("Count")?,
            })
        }
    }

    impl<'a> Deserialize<'a> for Container {
        fn deserialize(fields: &mut ObjectFields<'a>) -> Result<Self, ObjectStreamValueError> {
            Ok(Self {
                shape: fields.object("Shape")?,
            })
        }
    }

    fn leaf(field: &str, id: Uuid, data: impl Into<Vec<u8>>) -> Element {
        let element = Element::new(id).with_field(field).with_data(data);
        if let Some(kind) = crate::codec::builtin_serializer_kind(id) {
            element.with_builtin_serializer(crate::codec::BuiltinSerializerDescriptor::new(kind, 0))
        } else {
            element
        }
    }

    #[test]
    fn object_deserialize_reads_required_and_defaulted_fields() {
        let element = Element::new(types::AZSTD_VECTOR).with_children([
            leaf("Name", types::AZSTD_STRING, b"Crate"),
            leaf("Count", types::UNSIGNED_INT, 7_u32.to_be_bytes()),
        ]);

        let shape: Shape = deserialize(&element).unwrap();

        assert_eq!(
            shape,
            Shape {
                name: Box::<str>::from("Crate"),
                enabled: false,
                count: 7,
            }
        );
    }

    #[test]
    fn object_deserialize_rejects_unread_fields() {
        let element = Element::new(types::AZSTD_VECTOR).with_children([
            leaf("Name", types::AZSTD_STRING, b"Crate"),
            leaf("Count", types::UNSIGNED_INT, 7_u32.to_be_bytes()),
            leaf("Extra", types::BOOL, [1]),
        ]);

        let err = deserialize::<Shape>(&element).unwrap_err();

        assert!(matches!(err, ObjectStreamValueError::UnknownField { .. }));
    }

    #[test]
    fn nested_object_errors_include_field_path() {
        let element =
            Element::new(types::AZSTD_VECTOR).with_children([Element::new(types::AZSTD_VECTOR)
                .with_field("Shape")
                .with_children([
                    leaf("Name", types::AZSTD_STRING, b"Crate"),
                    leaf("Count", types::UNSIGNED_INT, 7_u32.to_be_bytes()),
                    leaf("Extra", types::BOOL, [1]),
                ])]);

        let err = deserialize::<Container>(&element).unwrap_err();

        assert!(matches!(
            err,
            ObjectStreamValueError::UnknownField { field } if field == "Shape.Extra"
        ));
    }
}
