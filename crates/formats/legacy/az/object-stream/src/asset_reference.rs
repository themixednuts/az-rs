use std::fmt::Write as _;
use std::num::TryFromIntError;
use std::str::Utf8Error;

use thiserror::Error;
use uuid::{Uuid, uuid};

use az_asset::{AssetId, UntypedAssetRef};
use az_core::AssetType;

use crate::Element;
use crate::types;
use crate::value::{self, ElementValue, ObjectStreamValueError};

pub const SIMPLE_ASSET_REFERENCE_BASE: &str = "SimpleAssetReferenceBase";
pub const ASSET_PATH_FIELD: &str = "AssetPath";

pub const SIMPLE_ASSET_REFERENCE_TYPE_ID: Uuid = types::ASSET;
pub const SIMPLE_TEXTURE_ASSET_REFERENCE_TYPE_ID: Uuid =
    uuid!("68e92460-5c0c-4031-9620-6f1a08763243");
pub const SIMPLE_ASSET_REFERENCE_BASE_TYPE_ID: Uuid = uuid!("e16ca6c5-5c78-4ad9-8e9b-f8c1fb4d1db8");

pub const BASE_CLASS_FIELD_CRC: u32 = 3_566_360_373;
pub const ASSET_PATH_FIELD_CRC: u32 = 741_691_769;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetValue<'a> {
    guid: Uuid,
    sub_id: u32,
    asset_type: Uuid,
    hint: &'a str,
    load_behavior: Option<u8>,
}

impl<'a> AssetValue<'a> {
    #[inline]
    #[must_use]
    pub const fn new(guid: Uuid, sub_id: u32, asset_type: Uuid, hint: &'a str) -> Self {
        Self {
            guid,
            sub_id,
            asset_type,
            hint,
            load_behavior: None,
        }
    }

    #[must_use]
    pub const fn with_load_behavior(mut self, load_behavior: u8) -> Self {
        self.load_behavior = Some(load_behavior);
        self
    }

    #[inline]
    #[must_use]
    pub const fn guid(self) -> Uuid {
        self.guid
    }

    #[inline]
    #[must_use]
    pub const fn sub_id(self) -> u32 {
        self.sub_id
    }

    #[inline]
    #[must_use]
    pub const fn asset_type(self) -> Uuid {
        self.asset_type
    }

    #[inline]
    #[must_use]
    pub const fn hint(self) -> &'a str {
        self.hint
    }

    #[inline]
    #[must_use]
    pub const fn load_behavior(self) -> Option<u8> {
        self.load_behavior
    }

    #[inline]
    #[must_use]
    pub fn into_untyped_asset_ref(self) -> UntypedAssetRef {
        let hint = (!self.hint.trim().is_empty()).then(|| self.hint.to_string());
        UntypedAssetRef::new(
            AssetId::new(self.guid, self.sub_id),
            AssetType::new(self.asset_type),
            hint,
        )
    }
}

#[derive(Debug, Error)]
pub enum AssetValueError {
    #[error("ObjectStream type resolution failed")]
    Value(#[from] ObjectStreamValueError),

    #[error("expected AZ::Data::Asset, got {actual}")]
    UnexpectedType { actual: Uuid },

    #[error("AZ::Data::Asset has no value bytes")]
    MissingData,

    #[error("AZ::Data::Asset has {actual} bytes, expected at least {expected}")]
    TooShort { expected: usize, actual: usize },

    #[error("AZ::Data::Asset hint length {declared} exceeds {actual} available bytes")]
    InvalidHintLength { declared: u64, actual: usize },

    #[error("AZ::Data::Asset sub id overflows u32")]
    SubIdOverflow(#[from] TryFromIntError),

    #[error("AZ::Data::Asset text does not match AssetSerializer syntax")]
    InvalidText,

    #[error("AZ::Data::Asset serializer version {0} is unsupported")]
    UnsupportedVersion(u32),

    #[error("AZ::Data::Asset native-endian payload has no serializer endian metadata")]
    MissingNativeEndianMetadata,

    #[error("AZ::Data::Asset hint is not valid UTF-8")]
    Utf8(#[from] Utf8Error),

    #[error("AZ::Data::Asset does not match a known value layout in {0} bytes")]
    UnsupportedLayout(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimpleAssetReferenceValue<'a> {
    guid: Uuid,
    asset_type: Uuid,
    path: &'a str,
}

impl<'a> SimpleAssetReferenceValue<'a> {
    #[inline]
    #[must_use]
    pub const fn new(guid: Uuid, asset_type: Uuid, path: &'a str) -> Self {
        Self {
            guid,
            asset_type,
            path,
        }
    }

    #[inline]
    #[must_use]
    pub const fn guid(self) -> Uuid {
        self.guid
    }

    #[inline]
    #[must_use]
    pub const fn asset_type(self) -> Uuid {
        self.asset_type
    }

    #[inline]
    #[must_use]
    pub const fn path(self) -> &'a str {
        self.path
    }
}

#[derive(Debug, Error)]
pub enum SimpleAssetReferenceValueError {
    #[error("ObjectStream type resolution failed")]
    Value(#[from] ObjectStreamValueError),

    #[error("expected SimpleAssetReference, got {actual}")]
    UnexpectedType { actual: Uuid },

    #[error("SimpleAssetReference has no value bytes")]
    MissingData,

    #[error("SimpleAssetReference has {actual} bytes, expected at least {expected}")]
    TooShort { expected: usize, actual: usize },

    #[error("SimpleAssetReference reserved AssetId bytes are not zero")]
    NonZeroAssetIdExtension,

    #[error("SimpleAssetReference path length {declared} does not match {actual} bytes")]
    InvalidPathLength { declared: usize, actual: usize },

    #[error("SimpleAssetReference path length overflows usize")]
    PathLengthOverflow(#[from] TryFromIntError),

    #[error("SimpleAssetReference path is not valid UTF-8")]
    Utf8(#[from] Utf8Error),
}

#[derive(Debug, Error)]
pub enum SimpleAssetReferenceElementError {
    #[error("expected SimpleAssetReference type {expected}, got {actual}")]
    UnexpectedType { expected: Uuid, actual: Uuid },

    #[error("SimpleAssetReference is missing BaseClass1")]
    MissingBase,

    #[error("SimpleAssetReference base has unexpected type {actual}")]
    UnexpectedBaseType { actual: Uuid },

    #[error("SimpleAssetReference has unexpected field with type {type_id}")]
    UnexpectedField { type_id: Uuid },

    #[error("SimpleAssetReference base is missing AssetPath")]
    MissingAssetPath,

    #[error("SimpleAssetReference value error")]
    Value(#[from] ObjectStreamValueError),
}

#[derive(Debug, Error)]
pub enum AssetHintError {
    #[error("asset serializer value error")]
    Asset(#[from] AssetValueError),
    #[error("expected AZ::Data::Asset, got {0}")]
    UnexpectedType(Uuid),
    #[error("asset reference has no value bytes")]
    MissingData,
    #[error("asset reference has {actual} bytes, expected at least {expected}")]
    TooShort { expected: usize, actual: usize },
    #[error("asset reference has no valid hint layout in {0} bytes")]
    InvalidHintLayout(usize),
    #[error("asset hint is not valid UTF-8")]
    Utf8(#[from] Utf8Error),
}

#[derive(Debug, Error)]
pub enum TypedAssetOrSimpleReferenceError {
    #[error("typed asset hint error")]
    Hint(#[from] AssetHintError),

    #[error("simple asset reference path error")]
    Path(#[from] ObjectStreamValueError),
}

#[derive(Debug, Error)]
pub enum AssetPathOrStringError {
    #[error("typed asset hint error")]
    Hint(#[from] AssetHintError),

    #[error("asset path value error")]
    Value(#[from] ObjectStreamValueError),
}

/// Decode a reflected `AZ::Data::Asset<T>` value payload.
///
/// # Errors
///
/// Returns [`AssetValueError::Value`] if the element's semantic type cannot be
/// resolved or if its payload is text (which carries no byte order to decode),
/// [`AssetValueError::UnexpectedType`] if the proven serializer is not the AZ
/// `AssetSerializer`, [`AssetValueError::MissingData`] if the element has no
/// value bytes, [`AssetValueError::MissingNativeEndianMetadata`] for a
/// native-endian payload with no captured serializer descriptor to name its
/// byte order, and, from the layout decode,
/// [`AssetValueError::TooShort`], [`AssetValueError::InvalidHintLength`],
/// [`AssetValueError::SubIdOverflow`], [`AssetValueError::Utf8`],
/// [`AssetValueError::UnsupportedVersion`] and
/// [`AssetValueError::UnsupportedLayout`].
pub fn read_asset_value<E>(element: &E) -> Result<AssetValue<'_>, AssetValueError>
where
    E: ElementValue + ?Sized,
{
    let actual = value::semantic_type_id(element)?;
    let serializer = element.builtin_serializer();
    let serializer_kind = serializer.map(|descriptor| descriptor.kind).or_else(|| {
        // Context-free streaming retains the canonical wire UUID but has no
        // ClassData descriptor. Stable built-ins are still unambiguous in the
        // binary big-endian wire format.
        crate::codec::builtin_serializer_kind(actual)
    });
    if serializer_kind != Some(crate::codec::BuiltinSerializerKind::Asset) {
        return Err(AssetValueError::UnexpectedType { actual });
    }
    let data = element.data().ok_or(AssetValueError::MissingData)?;
    match element.payload_encoding() {
        crate::PayloadEncoding::BinaryBigEndian => {
            AssetValueLayout::load(data, element.element_version())
        }
        crate::PayloadEncoding::BinaryNativeEndian => {
            let serializer = serializer.ok_or(AssetValueError::MissingNativeEndianMetadata)?;
            AssetValueLayout::load_with_endian(
                data,
                element.element_version(),
                serializer.native_endian,
            )
        }
        crate::PayloadEncoding::Text => Err(ObjectStreamValueError::UnexpectedPayloadEncoding {
            field: value::field_name(element),
            actual: crate::PayloadEncoding::Text,
        }
        .into()),
    }
}

/// Decode a `SimpleAssetReference` value payload.
///
/// # Errors
///
/// Returns [`SimpleAssetReferenceValueError::Value`] if the semantic type cannot
/// be resolved or the payload is not canonical big-endian,
/// [`SimpleAssetReferenceValueError::UnexpectedType`] if the element is not
/// [`SIMPLE_ASSET_REFERENCE_TYPE_ID`],
/// [`SimpleAssetReferenceValueError::MissingData`] if it has no value bytes, and
/// from the fixed layout [`SimpleAssetReferenceValueError::TooShort`],
/// [`SimpleAssetReferenceValueError::NonZeroAssetIdExtension`],
/// [`SimpleAssetReferenceValueError::InvalidPathLength`],
/// [`SimpleAssetReferenceValueError::PathLengthOverflow`] and
/// [`SimpleAssetReferenceValueError::Utf8`].
pub fn read_simple_asset_reference_value<E>(
    element: &E,
) -> Result<SimpleAssetReferenceValue<'_>, SimpleAssetReferenceValueError>
where
    E: ElementValue + ?Sized,
{
    let actual = value::semantic_type_id(element)?;
    if actual != SIMPLE_ASSET_REFERENCE_TYPE_ID {
        return Err(SimpleAssetReferenceValueError::UnexpectedType { actual });
    }
    if element.payload_encoding() != crate::PayloadEncoding::BinaryBigEndian {
        return Err(ObjectStreamValueError::UnexpectedPayloadEncoding {
            field: value::field_name(element),
            actual: element.payload_encoding(),
        }
        .into());
    }
    let data = element
        .data()
        .ok_or(SimpleAssetReferenceValueError::MissingData)?;
    SimpleAssetReferenceLayout::read(data)
}

/// Read the `AssetPath` of a `SimpleAssetReference` wrapper of a known type.
///
/// # Errors
///
/// Returns [`SimpleAssetReferenceElementError::UnexpectedType`] if the element
/// is not `expected_type_id`,
/// [`SimpleAssetReferenceElementError::MissingBase`] if it has no
/// `SimpleAssetReferenceBase` child,
/// [`SimpleAssetReferenceElementError::UnexpectedField`] if it has more than
/// that one child,
/// [`SimpleAssetReferenceElementError::UnexpectedBaseType`] if the base child is
/// not [`SIMPLE_ASSET_REFERENCE_BASE_TYPE_ID`], and
/// [`SimpleAssetReferenceElementError::Value`] wrapping the type-resolution or
/// string-decode failure — including a missing or duplicated
/// [`ASSET_PATH_FIELD`] under the base.
pub fn read_simple_asset_reference_path(
    element: &Element,
    expected_type_id: Uuid,
) -> Result<&str, SimpleAssetReferenceElementError> {
    let actual =
        value::semantic_type_id(element).map_err(SimpleAssetReferenceElementError::Value)?;
    if actual != expected_type_id {
        return Err(SimpleAssetReferenceElementError::UnexpectedType {
            expected: expected_type_id,
            actual,
        });
    }

    // Lumberyard serializes the base as a single BaseClass1 child. Prefer a
    // context-proven base-class edge when present; otherwise accept the raw
    // field-name layout used by XML/binary dumps without ClassData.
    let mut bases = element
        .children()
        .iter()
        .filter(|child| is_simple_asset_base_child(child));
    let Some(base) = bases.next() else {
        return Err(SimpleAssetReferenceElementError::MissingBase);
    };
    if bases.next().is_some() || element.children().len() != 1 {
        return Err(SimpleAssetReferenceElementError::UnexpectedField {
            type_id: *base.raw_type_id(),
        });
    }
    let base_type =
        value::semantic_type_id(base).map_err(SimpleAssetReferenceElementError::Value)?;
    if base_type != SIMPLE_ASSET_REFERENCE_BASE_TYPE_ID {
        return Err(SimpleAssetReferenceElementError::UnexpectedBaseType { actual: base_type });
    }
    read_exact_asset_path(base).map_err(SimpleAssetReferenceElementError::Value)
}

/// Read a reflected simple asset reference path without requiring a
/// specific wrapper type.
///
/// Some legacy `ObjectStreams` store `AssetPath` directly under the
/// reference object, while others place it one level down under a
/// `BaseClass1` / `SimpleAssetReferenceBase` child. Walk either layout so
/// callers stay free of the field-name detail.
///
/// # Errors
///
/// An element with no [`ASSET_PATH_FIELD`] anywhere in the two accepted layouts
/// is `Ok(None)`, not an error. When the field is present, returns any error
/// [`value::read_string`] returns for it —
/// [`ObjectStreamValueError::UnexpectedType`] for a non-string element,
/// [`ObjectStreamValueError::MissingData`] for one with no payload, and
/// [`ObjectStreamValueError::Utf8`] for bytes that are not valid UTF-8.
pub fn read_simple_asset_reference_path_any(
    element: &Element,
) -> Result<Option<&str>, ObjectStreamValueError> {
    let Some(field) = value::child_by_field(element, ASSET_PATH_FIELD).or_else(|| {
        element
            .children()
            .iter()
            .find_map(|child| value::child_by_field(child, ASSET_PATH_FIELD))
    }) else {
        return Ok(None);
    };

    let path = value::read_string(field)?.trim();
    Ok((!path.is_empty()).then_some(path))
}

/// Owning form of [`read_simple_asset_reference_path_any`].
///
/// # Errors
///
/// Returns any error [`read_simple_asset_reference_path_any`] returns.
pub fn read_simple_asset_reference_path_any_owned(
    element: &Element,
) -> Result<Option<String>, ObjectStreamValueError> {
    read_simple_asset_reference_path_any(element).map(|path| path.map(str::to_string))
}

/// Read an asset path serialized either as an `AZStd::string` payload
/// or as a reflected `SimpleAssetReference` object.
///
/// # Errors
///
/// For an `AZ::Data::Asset` element, returns
/// [`AssetPathOrStringError::Hint`] wrapping any error
/// [`optional_asset_hint_from_data`] returns. For a reflected string, and for
/// the `SimpleAssetReference` fallback, returns
/// [`AssetPathOrStringError::Value`] wrapping the string-decode failure —
/// [`ObjectStreamValueError::MissingData`],
/// [`ObjectStreamValueError::Utf8`] or
/// [`ObjectStreamValueError::UnexpectedType`].
pub fn read_asset_path_or_string(
    element: &Element,
) -> Result<Option<&str>, AssetPathOrStringError> {
    if is_asset_wire_type(element) {
        return optional_asset_hint_from_data(element).map_err(Into::into);
    }
    let is_string = element
        .builtin_serializer()
        .is_some_and(|serializer| serializer.kind == crate::codec::BuiltinSerializerKind::String)
        || (element.resolved_type_id().is_none()
            && matches!(
                *element.raw_type_id(),
                types::AZSTD_STRING | types::AZSTD_BASIC_STRING | types::AZSTD_STRING_LEGACY_XML
            ));
    if is_string {
        value::read_trimmed_string(element).map_err(Into::into)
    } else {
        read_simple_asset_reference_path_any(element).map_err(Into::into)
    }
}

/// Owning form of [`read_asset_path_or_string`].
///
/// # Errors
///
/// Returns any error [`read_asset_path_or_string`] returns.
pub fn read_asset_path_or_string_owned(
    element: &Element,
) -> Result<Option<String>, AssetPathOrStringError> {
    read_asset_path_or_string(element).map(|path| path.map(str::to_string))
}

/// Read the hint string out of a typed `AZ::Data::Asset` element.
///
/// # Errors
///
/// Returns [`AssetHintError::UnexpectedType`] if the element is not an
/// `AZ::Data::Asset` by proven serializer or wire UUID, plus any error
/// [`asset_hint_from_data`] returns.
pub fn asset_hint(element: &Element) -> Result<Option<&str>, AssetHintError> {
    if !is_asset_wire_type(element) {
        return Err(AssetHintError::UnexpectedType(*element.raw_type_id()));
    }

    asset_hint_from_data(element)
}

/// Owning form of [`asset_hint`].
///
/// # Errors
///
/// Returns any error [`asset_hint`] returns.
pub fn asset_hint_owned(element: &Element) -> Result<Option<String>, AssetHintError> {
    asset_hint(element).map(|path| path.map(str::to_string))
}

/// Read the hint string out of an element's asset value bytes, requiring a
/// payload.
///
/// # Errors
///
/// Returns [`AssetHintError::MissingData`] if the element carries no value
/// bytes, [`AssetHintError::TooShort`] if a sub-minimal payload is also not
/// readable as text, [`AssetHintError::Utf8`] if the hint bytes are not valid
/// UTF-8, [`AssetHintError::Asset`] if the versioned `AssetSerializer` fallback
/// rejects the payload, and [`AssetHintError::InvalidHintLayout`] if no
/// candidate layout matched and the element carries no version to fall back
/// on.
pub fn asset_hint_from_data(element: &Element) -> Result<Option<&str>, AssetHintError> {
    let data = element.data().ok_or(AssetHintError::MissingData)?;
    asset_hint_from_bytes(data, element.version())
}

/// Owning form of [`asset_hint_from_data`].
///
/// # Errors
///
/// Returns any error [`asset_hint_from_data`] returns.
pub fn asset_hint_from_data_owned(element: &Element) -> Result<Option<String>, AssetHintError> {
    asset_hint_from_data(element).map(|path| path.map(str::to_string))
}

/// Read an asset hint from raw asset value bytes when those bytes are
/// present.
///
/// Missing value bytes are treated as `Ok(None)`, matching reflected
/// typed asset references where an empty asset field is encoded as an
/// element without a payload. Malformed present payloads still fail.
///
/// # Errors
///
/// Missing value bytes are `Ok(None)`, not
/// [`AssetHintError::MissingData`]. A payload that is present returns the same
/// errors as [`asset_hint_from_data`]: [`AssetHintError::TooShort`],
/// [`AssetHintError::Utf8`], [`AssetHintError::Asset`] and
/// [`AssetHintError::InvalidHintLayout`].
pub fn optional_asset_hint_from_data(element: &Element) -> Result<Option<&str>, AssetHintError> {
    let Some(data) = element.data() else {
        return Ok(None);
    };
    asset_hint_from_bytes(data, element.version())
}

/// Owning form of [`optional_asset_hint_from_data`].
///
/// # Errors
///
/// Returns any error [`optional_asset_hint_from_data`] returns.
pub fn optional_asset_hint_from_data_owned(
    element: &Element,
) -> Result<Option<String>, AssetHintError> {
    optional_asset_hint_from_data(element).map(|path| path.map(str::to_string))
}

/// Read a typed asset hint when a value payload is present, otherwise
/// read a reflected simple asset reference path.
///
/// This matches legacy `AZ::Data::Asset<T>` fields that may be emitted
/// either as raw asset value bytes or as a reflected `SimpleAssetReference`
/// shape.
///
/// # Errors
///
/// When the element carries value bytes, returns
/// [`TypedAssetOrSimpleReferenceError::Hint`] wrapping any error
/// [`asset_hint_from_data`] returns. Otherwise returns
/// [`TypedAssetOrSimpleReferenceError::Path`] wrapping any error
/// [`read_simple_asset_reference_path_any`] returns; an element with neither a
/// payload nor an [`ASSET_PATH_FIELD`] is `Ok(None)`.
pub fn typed_asset_hint_or_simple_path(
    element: &Element,
) -> Result<Option<&str>, TypedAssetOrSimpleReferenceError> {
    if element.data().is_some() {
        asset_hint_from_data(element).map_err(TypedAssetOrSimpleReferenceError::Hint)
    } else {
        read_simple_asset_reference_path_any(element)
            .map_err(TypedAssetOrSimpleReferenceError::Path)
    }
}

fn read_exact_asset_path(base: &Element) -> Result<&str, ObjectStreamValueError> {
    let actual = value::semantic_type_id(base)?;
    if actual != SIMPLE_ASSET_REFERENCE_BASE_TYPE_ID {
        return Err(ObjectStreamValueError::UnexpectedType {
            field: value::field_name(base),
            expected: SIMPLE_ASSET_REFERENCE_BASE,
            actual,
        });
    }
    let path = value::child_by_field(base, ASSET_PATH_FIELD).ok_or_else(|| {
        ObjectStreamValueError::MissingField {
            field: ASSET_PATH_FIELD.to_owned(),
        }
    })?;
    if base.children().len() != 1 {
        return Err(ObjectStreamValueError::UnknownField {
            field: base
                .children()
                .iter()
                .find(|child| !std::ptr::eq(*child, path))
                .map_or_else(|| ASSET_PATH_FIELD.to_owned(), value::field_name),
        });
    }
    value::read_string(path)
}

/// Whether a child is the `SimpleAssetReferenceBase` slot.
///
/// Proven `ClassData` base-class edges always count. Raw dumps without
/// `ClassData` still mark the slot with the Lumberyard `BaseClass1` field
/// name (or its CRC).
fn is_simple_asset_base_child(child: &Element) -> bool {
    let type_matches =
        value::semantic_type_id(child).ok() == Some(SIMPLE_ASSET_REFERENCE_BASE_TYPE_ID);
    if !type_matches {
        return false;
    }
    child.is_base_class_edge()
        || child
            .field()
            .is_some_and(|field| field.as_str() == "BaseClass1")
        || child.name_crc() == Some(BASE_CLASS_FIELD_CRC)
}

/// Owning form of [`typed_asset_hint_or_simple_path`].
///
/// # Errors
///
/// Returns any error [`typed_asset_hint_or_simple_path`] returns.
pub fn typed_asset_hint_or_simple_path_owned(
    element: &Element,
) -> Result<Option<String>, TypedAssetOrSimpleReferenceError> {
    typed_asset_hint_or_simple_path(element).map(|path| path.map(str::to_string))
}

fn is_asset_wire_type(element: &Element) -> bool {
    element
        .builtin_serializer()
        .is_some_and(|serializer| serializer.kind == crate::codec::BuiltinSerializerKind::Asset)
        || element
            .resolved_type_id()
            .copied()
            // Unresolved elements keep the wire UUID as their typed identity.
            .unwrap_or_else(|| *element.raw_type_id())
            == types::ASSET
}

/// Decode an asset hint from value bytes.
///
/// Prefer multi-layout scanning used by historical binary dumps and synthetic
/// fixtures (HEAD Lumberyard `ObjectStream`). When an explicit element version is
/// present and the multi-layout scan fails, fall back to the versioned
/// `AssetSerializer` Load path for production payloads.
fn asset_hint_from_bytes(
    data: &[u8],
    element_version: Option<u32>,
) -> Result<Option<&str>, AssetHintError> {
    if data.len() < AssetHintLayout::MINIMUM_LEN {
        let hint = asset_hint_from_text(data)?.ok_or(AssetHintError::TooShort {
            expected: AssetHintLayout::MINIMUM_LEN,
            actual: data.len(),
        })?;
        return Ok(Some(hint));
    }

    for layout in AssetHintLayout::CANDIDATES {
        if let Some(hint) = layout.read_hint(data)? {
            let hint = hint.trim();
            return Ok((!hint.is_empty()).then_some(hint));
        }
    }

    if let Some(hint) = asset_hint_from_text(data)? {
        return Ok(Some(hint));
    }

    if let Some(version) = element_version {
        let value = AssetValueLayout::load(data, version)?;
        let hint = value.hint().trim();
        return Ok((!hint.is_empty()).then_some(hint));
    }

    Err(AssetHintError::InvalidHintLayout(data.len()))
}

fn asset_hint_from_text(data: &[u8]) -> Result<Option<&str>, Utf8Error> {
    let value = std::str::from_utf8(data)?.trim();
    let Some(after_prefix) = value
        .find("hint={")
        .map(|start| &value[start + "hint={".len()..])
    else {
        return Ok(None);
    };
    let hint = after_prefix
        .split_once('}')
        .map_or(after_prefix, |(hint, _)| hint)
        .trim();
    Ok((!hint.is_empty()).then_some(hint))
}

#[derive(Debug, Clone, Copy)]
struct AssetHintLayout {
    hint_size_offset: usize,
}

impl AssetHintLayout {
    /// Historical / fixture layouts store the hint length at different offsets
    /// depending on `AssetId` padding width. HEAD scans 40, 36, then 48.
    const MINIMUM_LEN: usize = 44;
    const CANDIDATES: &'static [Self] = &[
        Self {
            hint_size_offset: 40,
        },
        Self {
            hint_size_offset: 36,
        },
        Self {
            hint_size_offset: 48,
        },
    ];

    fn read_hint(self, data: &[u8]) -> Result<Option<&str>, AssetHintError> {
        let Some(size_bytes) = data.get(self.hint_size_offset..self.hint_size_offset + 8) else {
            return Ok(None);
        };
        let size_bytes: [u8; 8] = size_bytes.try_into().expect("slice width is eight");
        let hint_start = self.hint_size_offset + 8;
        let Some(available) = data.len().checked_sub(hint_start) else {
            return Ok(None);
        };

        // HEAD exact-fit: declared length must consume the rest of the payload
        // (no trailing loadBehavior byte). Be and le length encodings accepted.
        for declared in [
            u64::from_be_bytes(size_bytes),
            u64::from_le_bytes(size_bytes),
        ] {
            // A length that does not fit `usize` cannot equal `available`.
            if usize::try_from(declared).is_ok_and(|declared| declared == available) {
                return Ok(Some(std::str::from_utf8(&data[hint_start..])?));
            }
        }

        Ok(None)
    }
}

#[derive(Debug, Clone, Copy)]
struct SimpleAssetReferenceLayout;

impl SimpleAssetReferenceLayout {
    const ASSET_ID_EXTENSION: std::ops::Range<usize> = 16..32;
    const ASSET_TYPE: std::ops::Range<usize> = 32..48;
    const PATH_LEN: std::ops::Range<usize> = 48..56;
    const PATH_START: usize = 56;

    fn read(data: &[u8]) -> Result<SimpleAssetReferenceValue<'_>, SimpleAssetReferenceValueError> {
        if data.len() < Self::PATH_START {
            return Err(SimpleAssetReferenceValueError::TooShort {
                expected: Self::PATH_START,
                actual: data.len(),
            });
        }
        if data[Self::ASSET_ID_EXTENSION].iter().any(|byte| *byte != 0) {
            return Err(SimpleAssetReferenceValueError::NonZeroAssetIdExtension);
        }

        let guid = Uuid::from_bytes(data[0..16].try_into().expect("guid slice is sixteen bytes"));
        let asset_type = Uuid::from_bytes(
            data[Self::ASSET_TYPE]
                .try_into()
                .expect("asset type slice is sixteen bytes"),
        );
        let path_len: usize = u64::from_be_bytes(
            data[Self::PATH_LEN]
                .try_into()
                .expect("path length slice is eight bytes"),
        )
        .try_into()?;
        let path_end = Self::PATH_START + path_len;
        if path_end != data.len() {
            return Err(SimpleAssetReferenceValueError::InvalidPathLength {
                declared: path_len,
                actual: data.len().saturating_sub(Self::PATH_START),
            });
        }
        let path = std::str::from_utf8(&data[Self::PATH_START..path_end])?;
        Ok(SimpleAssetReferenceValue::new(guid, asset_type, path))
    }
}

/// Logical value used by the native `AssetSerializer` operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssetValueLayout {
    guid: Uuid,
    sub_id: u32,
    asset_type: Uuid,
    hint: String,
    load_behavior: u8,
}

impl AssetValueLayout {
    const TYPE_OFFSET: usize = 32;
    const TYPE_END: usize = 48;
    const HINT_LEN_OFFSET: usize = 48;
    const HINT_OFFSET: usize = 56;
    const DEFAULT_LOAD_BEHAVIOR: u8 = 1; // AssetLoadBehavior::PreLoad

    /// Native Load operation: consume fields according to the serialized
    /// element version. Later fields are deliberately ignored, matching
    /// `AssetSerializer::Load` when `TextToData` saved a newer current layout.
    pub(crate) fn load(
        data: &[u8],
        element_version: u32,
    ) -> Result<AssetValue<'_>, AssetValueError> {
        Self::load_with_endian(data, element_version, crate::codec::NativeEndian::Big)
    }

    pub(crate) fn load_with_endian(
        data: &[u8],
        element_version: u32,
        endian: crate::codec::NativeEndian,
    ) -> Result<AssetValue<'_>, AssetValueError> {
        let logical = Self::load_owned(data, element_version, endian)?;
        let hint_start = if element_version == 0 {
            data.len()
        } else {
            Self::HINT_OFFSET
        };
        let hint_end = hint_start + logical.hint.len();
        let hint = if element_version == 0 {
            ""
        } else {
            std::str::from_utf8(&data[hint_start..hint_end])?
        };
        let value = AssetValue::new(logical.guid, logical.sub_id, logical.asset_type, hint);
        Ok(if element_version > 1 {
            value.with_load_behavior(logical.load_behavior)
        } else {
            value
        })
    }

    pub(crate) fn from_text(text: &str, text_version: u32) -> Result<Self, AssetValueError> {
        let (id, after_id) = text
            .strip_prefix("id=")
            .and_then(|text| text.split_once(",type="))
            .ok_or(AssetValueError::InvalidText)?;
        let (guid, sub_id) = id.rsplit_once(':').ok_or(AssetValueError::InvalidText)?;
        let (type_text, remainder) = after_id
            .split_once('}')
            .ok_or(AssetValueError::InvalidText)?;
        let asset_type = parse_asset_uuid(&format!("{type_text}}}"))?;
        let mut hint = String::new();
        let mut load_behavior = Self::DEFAULT_LOAD_BEHAVIOR;
        if text_version > 0 {
            let hint_start = remainder
                .strip_prefix(",hint={")
                .ok_or(AssetValueError::InvalidText)?;
            let (parsed_hint, after_hint) = hint_start
                .split_once('}')
                .ok_or(AssetValueError::InvalidText)?;
            parsed_hint.clone_into(&mut hint);
            if text_version > 1 {
                let behavior = after_hint
                    .strip_prefix(",loadBehavior=")
                    .ok_or(AssetValueError::InvalidText)?;
                load_behavior =
                    u8::from_str_radix(behavior, 16).map_err(|_| AssetValueError::InvalidText)?;
            }
        }
        Ok(Self {
            guid: parse_asset_uuid(guid)?,
            sub_id: u32::from_str_radix(sub_id, 16).map_err(|_| AssetValueError::InvalidText)?,
            asset_type,
            hint,
            load_behavior,
        })
    }

    /// Native Save operation for the registered serializer generation.
    pub(crate) fn to_big_endian_bytes(
        &self,
        serializer_version: u32,
    ) -> Result<Vec<u8>, AssetValueError> {
        if serializer_version > 2 {
            return Err(AssetValueError::UnsupportedVersion(serializer_version));
        }
        let mut bytes = Vec::with_capacity(
            Self::TYPE_END
                + usize::from(serializer_version > 0) * (8 + self.hint.len())
                + usize::from(serializer_version > 1),
        );
        bytes.extend_from_slice(self.guid.as_bytes());
        bytes.extend_from_slice(&self.sub_id.to_be_bytes());
        bytes.extend_from_slice(&[0; 12]);
        bytes.extend_from_slice(self.asset_type.as_bytes());
        if serializer_version > 0 {
            bytes.extend_from_slice(&(self.hint.len() as u64).to_be_bytes());
            bytes.extend_from_slice(self.hint.as_bytes());
        }
        if serializer_version > 1 {
            bytes.push(self.load_behavior);
        }
        Ok(bytes)
    }

    /// Native serializer transcode: Load using the serialized element version,
    /// then emit text using the currently registered serializer generation.
    pub(crate) fn historical_data_to_current_text(
        data: &[u8],
        element_version: u32,
        payload_endian: crate::codec::NativeEndian,
        current_version: u32,
    ) -> Result<String, AssetValueError> {
        Self::load_owned(data, element_version, payload_endian)?.to_text(current_version)
    }

    fn load_owned(
        data: &[u8],
        element_version: u32,
        endian: crate::codec::NativeEndian,
    ) -> Result<Self, AssetValueError> {
        if element_version > 2 {
            return Err(AssetValueError::UnsupportedVersion(element_version));
        }
        let minimum = Self::TYPE_END
            + usize::from(element_version > 0) * 8
            + usize::from(element_version > 1);
        if data.len() < minimum {
            return Err(AssetValueError::TooShort {
                expected: minimum,
                actual: data.len(),
            });
        }
        let guid = Uuid::from_bytes(data[..16].try_into().expect("guid width is sixteen"));
        let sub_id_bytes = data[16..20].try_into().expect("sub id width is four");
        let sub_id = match endian {
            crate::codec::NativeEndian::Little => u32::from_le_bytes(sub_id_bytes),
            crate::codec::NativeEndian::Big => u32::from_be_bytes(sub_id_bytes),
        };
        let asset_type = Uuid::from_bytes(
            data[Self::TYPE_OFFSET..Self::TYPE_END]
                .try_into()
                .expect("asset type width is sixteen"),
        );
        let (hint, load_behavior) = if element_version == 0 {
            (String::new(), Self::DEFAULT_LOAD_BEHAVIOR)
        } else {
            let hint_len_bytes = data[Self::HINT_LEN_OFFSET..Self::HINT_OFFSET]
                .try_into()
                .expect("hint length width is eight");
            let hint_len = match endian {
                crate::codec::NativeEndian::Little => u64::from_le_bytes(hint_len_bytes),
                crate::codec::NativeEndian::Big => u64::from_be_bytes(hint_len_bytes),
            };
            let trailing = usize::from(element_version > 1);
            let available = data.len() - Self::HINT_OFFSET - trailing;
            let available_u64 = u64::try_from(available).unwrap_or(u64::MAX);
            if hint_len > available_u64 {
                return Err(AssetValueError::InvalidHintLength {
                    declared: hint_len,
                    actual: available,
                });
            }
            let hint_len =
                usize::try_from(hint_len).map_err(|_| AssetValueError::InvalidHintLength {
                    declared: hint_len,
                    actual: available,
                })?;
            let hint_end = Self::HINT_OFFSET + hint_len;
            let hint = std::str::from_utf8(&data[Self::HINT_OFFSET..hint_end])?.to_owned();
            let behavior = if element_version > 1 {
                data[hint_end]
            } else {
                Self::DEFAULT_LOAD_BEHAVIOR
            };
            (hint, behavior)
        };
        Ok(Self {
            guid,
            sub_id,
            asset_type,
            hint,
            load_behavior,
        })
    }

    fn to_text(&self, text_version: u32) -> Result<String, AssetValueError> {
        if text_version > 2 {
            return Err(AssetValueError::UnsupportedVersion(text_version));
        }
        let mut text = format!(
            "id={}:{:X},type={}",
            self.guid.as_braced().to_string().to_uppercase(),
            self.sub_id,
            self.asset_type.as_braced().to_string().to_uppercase()
        );
        if text_version > 0 {
            write!(text, ",hint={{{}}}", self.hint).expect("writing to a String cannot fail");
        }
        if text_version > 1 {
            write!(text, ",loadBehavior={}", self.load_behavior)
                .expect("writing to a String cannot fail");
        }
        Ok(text)
    }
}

fn parse_asset_uuid(text: &str) -> Result<Uuid, AssetValueError> {
    Uuid::parse_str(text.trim().trim_start_matches('{').trim_end_matches('}'))
        .map_err(|_| AssetValueError::InvalidText)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Element;

    #[test]
    fn reads_padded_asset_value() {
        let guid = uuid!("7a1472d1-df54-5362-bc71-9974d5f25572");
        let asset_type = uuid!("78802abf-9595-463a-8d2b-d022f906f9b1");
        let mut element = typed_asset(asset_bytes_with_padding(
            guid, 2, [0xa5; 12], asset_type, "bobber",
        ));
        element.version = Some(1);

        let asset = read_asset_value(&element).unwrap();

        assert_eq!(asset.guid(), guid);
        assert_eq!(asset.sub_id(), 2);
        assert_eq!(asset.asset_type(), asset_type);
        assert_eq!(asset.hint(), "bobber");
    }

    #[test]
    fn reads_context_free_raw_asset_value_from_stable_wire_uuid() {
        let guid = uuid!("7a1472d1-df54-5362-bc71-9974d5f25572");
        let asset_type = uuid!("78802abf-9595-463a-8d2b-d022f906f9b1");
        let mut element = Element::new(types::ASSET).with_data(asset_bytes(
            guid,
            2,
            asset_type,
            "objects/fishing/bobber.cgf",
        ));
        element.version = Some(1);

        let asset = read_asset_value(&element).unwrap();

        assert_eq!(asset.guid(), guid);
        assert_eq!(asset.sub_id(), 2);
        assert_eq!(asset.asset_type(), asset_type);
        assert_eq!(asset.hint(), "objects/fishing/bobber.cgf");
    }

    #[test]
    fn treats_asset_id_tail_bytes_as_opaque_native_padding() {
        let guid = uuid!("699fa9e5-4f8a-5b01-87b2-d5f718c927b8");
        let padding_evidence = uuid!("d087f9c9-0000-0000-0000-000000000001");
        let asset_type = uuid!("c2869e3b-dda0-4e01-8fe3-6770d788866b");
        let element = typed_asset(asset_bytes_with_uuid_shaped_id_tail(
            guid,
            padding_evidence,
            asset_type,
            "slices/dungeon/firstlight/ancientgrate_circular__28236438930.cgf",
        ));

        let asset = read_asset_value(&element).unwrap();

        assert_eq!(asset.guid(), guid);
        assert_eq!(asset.sub_id(), 0xd087_f9c9);
        assert_eq!(asset.asset_type(), asset_type);
        assert_eq!(
            asset.hint(),
            "slices/dungeon/firstlight/ancientgrate_circular__28236438930.cgf"
        );
    }

    #[test]
    fn rejects_non_native_u32_reserved_layout() {
        let guid = uuid!("7a1472d1-df54-5362-bc71-9974d5f25572");
        let asset_type = uuid!("78802abf-9595-463a-8d2b-d022f906f9b1");
        let element = typed_asset(u32_reserved_asset_bytes(
            guid,
            2,
            asset_type,
            "slices/spawner.slice",
        ));

        assert!(read_asset_value(&element).is_err());
    }

    #[test]
    fn reads_simple_asset_reference_value() {
        let guid = uuid!("7a1472d1-df54-5362-bc71-9974d5f25572");
        let asset_type = uuid!("78802abf-9595-463a-8d2b-d022f906f9b1");
        let element = Element::new(SIMPLE_ASSET_REFERENCE_TYPE_ID)
            .with_test_class()
            .with_data(simple_asset_reference_bytes(
                guid,
                asset_type,
                "objects/cannon.cgf",
            ));

        let asset = read_simple_asset_reference_value(&element).unwrap();

        assert_eq!(asset.guid(), guid);
        assert_eq!(asset.asset_type(), asset_type);
        assert_eq!(asset.path(), "objects/cannon.cgf");
    }

    #[test]
    fn reads_nested_simple_asset_reference_path() {
        let element = simple_reference(SIMPLE_TEXTURE_ASSET_REFERENCE_TYPE_ID, "textures/icon.dds");

        let path =
            read_simple_asset_reference_path(&element, SIMPLE_TEXTURE_ASSET_REFERENCE_TYPE_ID)
                .unwrap();

        assert_eq!(path, "textures/icon.dds");
    }

    #[test]
    fn reads_relaxed_simple_asset_reference_path_direct_field() {
        let element = simple_base(" objects/cannon.cgf ");

        let path = read_simple_asset_reference_path_any(&element).unwrap();

        assert_eq!(path, Some("objects/cannon.cgf"));
    }

    #[test]
    fn reads_relaxed_simple_asset_reference_path_nested_field() {
        let element = simple_reference(SIMPLE_TEXTURE_ASSET_REFERENCE_TYPE_ID, "textures/icon.dds");

        let path = read_simple_asset_reference_path_any_owned(&element).unwrap();

        assert_eq!(path, Some("textures/icon.dds".to_string()));
    }

    #[test]
    fn skips_blank_relaxed_simple_asset_reference_path() {
        let element = simple_base("  ");

        let path = read_simple_asset_reference_path_any(&element).unwrap();

        assert_eq!(path, None);
    }

    #[test]
    fn reads_asset_path_or_string_from_string_payload() {
        let element = string_element(" textures/cubemap.dds ");

        let path = read_asset_path_or_string(&element).unwrap();

        assert_eq!(path, Some("textures/cubemap.dds"));
    }

    #[test]
    fn reads_asset_path_or_string_from_simple_reference() {
        let element = simple_reference(
            SIMPLE_TEXTURE_ASSET_REFERENCE_TYPE_ID,
            "textures/cubemap.dds",
        );

        let path = read_asset_path_or_string_owned(&element).unwrap();

        assert_eq!(path, Some("textures/cubemap.dds".to_string()));
    }

    #[test]
    fn reads_asset_path_or_string_from_typed_asset_payload() {
        let element = typed_asset(asset_bytes(
            Uuid::nil(),
            0,
            Uuid::nil(),
            "slices/characters/sandworm.dynamicslice",
        ));

        let path = read_asset_path_or_string_owned(&element).unwrap();

        assert_eq!(
            path.as_deref(),
            Some("slices/characters/sandworm.dynamicslice")
        );
    }

    #[test]
    fn skips_blank_asset_path_string() {
        let element = string_element("   ");

        let path = read_asset_path_or_string(&element).unwrap();

        assert_eq!(path, None);
    }

    #[test]
    fn reads_asset_hint_from_text_payload() {
        let element = Element::new(types::ASSET).with_data(
            b"id={1E9A1948-F2A6-5500-B918-964558497331}:0,type={F46985B5-F7FF-4FCB-8E8C-DC240D701841},hint={materials/foo.mtl}",
        );

        assert_eq!(asset_hint(&element).unwrap(), Some("materials/foo.mtl"));
    }

    #[test]
    fn reads_asset_hint_from_short_text_hint_only_payload() {
        let element = Element::new(types::ASSET).with_data("hint={Characters/Paperdoll.asset}");

        assert_eq!(
            optional_asset_hint_from_data(&element).unwrap(),
            Some("Characters/Paperdoll.asset")
        );
    }

    #[test]
    fn reads_asset_hint_from_big_endian_layout() {
        let hint = "slices/foo.slice";
        let mut data = vec![0; 48];
        data[40..48].copy_from_slice(&(hint.len() as u64).to_be_bytes());
        data.extend_from_slice(hint.as_bytes());
        let element = Element::new(types::ASSET).with_data(data);

        assert_eq!(asset_hint(&element).unwrap(), Some(hint));
    }

    #[test]
    fn reads_asset_hint_from_compact_four_byte_pad_layout() {
        let hint = "Timelines/IceCastle/Gate.timeline";
        let mut data = Vec::new();
        data.extend(Uuid::from_u128(0x11111111_2222_3333_4444_555555555555).as_bytes());
        data.extend(0x20000_u32.to_be_bytes());
        data.extend([0; 4]);
        data.extend(Uuid::from_u128(0x22222222_3333_4444_5555_666666666666).as_bytes());
        data.extend((hint.len() as u64).to_be_bytes());
        data.extend(hint.as_bytes());
        let element = Element::new(types::ASSET).with_data(data);

        assert_eq!(optional_asset_hint_from_data(&element).unwrap(), Some(hint));
    }

    #[test]
    fn rejects_asset_hint_length_larger_than_available_payload() {
        let mut data = asset_bytes(Uuid::nil(), 0, Uuid::nil(), "");
        data[AssetValueLayout::HINT_LEN_OFFSET..AssetValueLayout::HINT_OFFSET]
            .copy_from_slice(&u64::MAX.to_be_bytes());
        let element = typed_asset(data);

        assert!(matches!(
            read_asset_value(&element),
            Err(AssetValueError::InvalidHintLength {
                declared: u64::MAX,
                actual: 0,
            })
        ));
    }

    #[test]
    fn reads_owned_asset_hint_from_data_without_type_check() {
        let element = Element::new(uuid!("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")).with_data(
            b"id={1E9A1948-F2A6-5500-B918-964558497331}:0,type={F46985B5-F7FF-4FCB-8E8C-DC240D701841},hint={scripts/foo.lua}",
        );

        let hint = asset_hint_from_data_owned(&element).unwrap();

        assert_eq!(hint, Some("scripts/foo.lua".to_string()));
    }

    #[test]
    fn reads_optional_asset_hint_from_data_without_type_check() {
        let missing = Element::new(uuid!("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"));
        assert_eq!(optional_asset_hint_from_data_owned(&missing).unwrap(), None);

        let present = Element::new(uuid!("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")).with_data(
            b"id={1E9A1948-F2A6-5500-B918-964558497331}:0,type={F46985B5-F7FF-4FCB-8E8C-DC240D701841},hint={scripts/foo.lua}",
        );
        assert_eq!(
            optional_asset_hint_from_data(&present).unwrap(),
            Some("scripts/foo.lua")
        );

        let malformed = Element::new(uuid!("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")).with_data([1]);
        assert!(matches!(
            optional_asset_hint_from_data(&malformed).unwrap_err(),
            AssetHintError::TooShort {
                expected: AssetHintLayout::MINIMUM_LEN,
                actual: 1
            }
        ));
    }

    #[test]
    fn reads_typed_asset_hint_or_simple_path() {
        let typed = Element::new(uuid!("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")).with_data(
            b"id={1E9A1948-F2A6-5500-B918-964558497331}:0,type={F46985B5-F7FF-4FCB-8E8C-DC240D701841},hint={scripts/foo.lua}",
        );
        assert_eq!(
            typed_asset_hint_or_simple_path(&typed).unwrap(),
            Some("scripts/foo.lua")
        );

        let simple = simple_reference(SIMPLE_TEXTURE_ASSET_REFERENCE_TYPE_ID, "textures/icon.dds");
        assert_eq!(
            typed_asset_hint_or_simple_path_owned(&simple).unwrap(),
            Some("textures/icon.dds".to_string())
        );

        let blank = simple_base("  ");
        assert_eq!(typed_asset_hint_or_simple_path(&blank).unwrap(), None);

        let malformed = typed_asset_with_id(uuid!("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"), [1]);
        assert!(matches!(
            typed_asset_hint_or_simple_path(&malformed).unwrap_err(),
            TypedAssetOrSimpleReferenceError::Hint(AssetHintError::TooShort {
                expected: AssetHintLayout::MINIMUM_LEN,
                actual: 1
            })
        ));
    }

    #[test]
    fn reads_owned_asset_hint_from_typed_asset() {
        let hint = "materials/foo.mtl";
        let element = typed_asset(asset_bytes(Uuid::nil(), 0, Uuid::nil(), hint));

        let hint = asset_hint_owned(&element).unwrap();

        assert_eq!(hint, Some("materials/foo.mtl".to_string()));
    }

    fn asset_bytes(guid: Uuid, sub_id: u32, asset_type: Uuid, hint: &str) -> Vec<u8> {
        asset_bytes_with_padding(guid, sub_id, [0; 12], asset_type, hint)
    }

    fn asset_bytes_with_padding(
        guid: Uuid,
        sub_id: u32,
        padding: [u8; 12],
        asset_type: Uuid,
        hint: &str,
    ) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(56 + hint.len());
        bytes.extend_from_slice(guid.as_bytes());
        bytes.extend_from_slice(&sub_id.to_be_bytes());
        bytes.extend_from_slice(&padding);
        bytes.extend_from_slice(asset_type.as_bytes());
        bytes.extend_from_slice(&(hint.len() as u64).to_be_bytes());
        bytes.extend_from_slice(hint.as_bytes());
        bytes
    }

    fn typed_asset(data: impl Into<Vec<u8>>) -> Element {
        typed_asset_with_id(types::ASSET, data)
    }

    fn typed_asset_with_id(id: Uuid, data: impl Into<Vec<u8>>) -> Element {
        let mut element = Element::new(id)
            .with_builtin_serializer(crate::codec::BuiltinSerializerDescriptor::new(
                crate::codec::BuiltinSerializerKind::Asset,
                1,
            ))
            .with_data(data);
        element.version = Some(1);
        element
    }

    fn string_element(value: &str) -> Element {
        Element::new(types::AZSTD_STRING)
            .with_builtin_serializer(crate::codec::BuiltinSerializerDescriptor::new(
                crate::codec::BuiltinSerializerKind::String,
                0,
            ))
            .with_data(value.as_bytes())
    }

    fn simple_base(path: &str) -> Element {
        Element::new(SIMPLE_ASSET_REFERENCE_BASE_TYPE_ID)
            .with_test_class()
            .with_children([string_element(path).with_field(ASSET_PATH_FIELD)])
    }

    fn simple_reference(type_id: Uuid, path: &str) -> Element {
        Element::new(type_id)
            .with_test_class()
            .with_children([simple_base(path)
                .with_field("BaseClass1")
                .with_test_base_class_edge()])
    }

    fn asset_bytes_with_uuid_shaped_id_tail(
        guid: Uuid,
        sub_id: Uuid,
        asset_type: Uuid,
        hint: &str,
    ) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(56 + hint.len());
        bytes.extend_from_slice(guid.as_bytes());
        bytes.extend_from_slice(sub_id.as_bytes());
        bytes.extend_from_slice(asset_type.as_bytes());
        bytes.extend_from_slice(&(hint.len() as u64).to_be_bytes());
        bytes.extend_from_slice(hint.as_bytes());
        bytes
    }

    fn u32_reserved_asset_bytes(guid: Uuid, sub_id: u32, asset_type: Uuid, hint: &str) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(48 + hint.len());
        bytes.extend_from_slice(guid.as_bytes());
        bytes.extend_from_slice(&sub_id.to_be_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(asset_type.as_bytes());
        bytes.extend_from_slice(&(hint.len() as u64).to_be_bytes());
        bytes.extend_from_slice(hint.as_bytes());
        bytes
    }

    fn simple_asset_reference_bytes(guid: Uuid, asset_type: Uuid, path: &str) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(56 + path.len());
        bytes.extend_from_slice(guid.as_bytes());
        bytes.extend_from_slice(&[0; 16]);
        bytes.extend_from_slice(asset_type.as_bytes());
        bytes.extend_from_slice(&(path.len() as u64).to_be_bytes());
        bytes.extend_from_slice(path.as_bytes());
        bytes
    }
}
