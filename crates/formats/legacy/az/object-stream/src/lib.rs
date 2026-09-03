//! AZ `ObjectStream` binary / XML / JSON codec.
//!
//! Reads and writes AZ `ObjectStream` payloads — the serialized
//! container format used by Lumberyard/O3DE-era slices, prefabs,
//! component data, etc. — through one structured [`ObjectStream`]
//! graph. Binary, XML, and JSON variants normalize to the same
//! in-memory shape.
//!
//! # Scope
//!
//! Import-time only. `ObjectStream` is the legacy AZ wire format;
//! `az-rs` serializes transformed content into engine formats
//! (postcard, RON, and project-owned binary containers). See
//! `docs/architecture/legacy-format-ownership.md`.
//!
//! # Quick start
//!
//! ```no_run
//! use az_objectstream::ObjectStream;
//!
//! let bytes = std::fs::read("some.slice")?;
//! let stream = ObjectStream::from_bytes(&bytes, None)?;
//! for element in stream.iter_recursive() {
//!     println!("{} {}", element.raw_type_id(), element.name());
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Format boundaries live in [`deserialize`] and [`serialize`].
//! Higher-level tools should select an [`ObjectStreamEncoding`] and let
//! this crate own the byte/XML/JSON details.

pub mod asset_id;
pub mod asset_reference;
pub(crate) mod binary;
pub mod capture;
pub mod codec;
pub mod component_id;
pub mod container;
pub mod context;
pub mod de;
pub mod deserialize;
pub mod lookup;
pub mod object;
pub mod overlay;
pub mod query;
pub mod serialize;
pub mod types;
pub mod value;
pub mod visit;

use std::fmt;
use std::io::{self, Cursor, Read, Write};
use std::str;

use arcstr::ArcStr;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::lookup::LumberyardHashes;

// === binary flag bits ===

/// Top-3-bit mask isolating element-header flag bits.
pub const ST_BINARYFLAG_MASK: u8 = 0xF8;
/// Bottom-3-bit mask: encoded value-byte width (1, 2, or 4).
pub const ST_BINARY_VALUE_SIZE_MASK: u8 = 0x07;
/// Bit set when the byte introduces a new element header.
pub const ST_BINARYFLAG_ELEMENT_HEADER: u8 = 1 << 3;
pub const ST_BINARYFLAG_HAS_VALUE: u8 = 1 << 4;
pub const ST_BINARYFLAG_EXTRA_SIZE_FIELD: u8 = 1 << 5;
pub const ST_BINARYFLAG_HAS_NAME: u8 = 1 << 6;
pub const ST_BINARYFLAG_HAS_VERSION: u8 = 1 << 7;
/// Sentinel byte that terminates the current element list.
pub const ST_BINARYFLAG_ELEMENT_END: u8 = 0;

/// Canonical AZ field-name CRC.
///
/// `AZ::Crc32(AZStd::string_view)` folds ASCII uppercase before applying
/// CRC-32; `ObjectStream` `ClassElement` lookup uses the same constructor on
/// both reflected and textual field names.
#[inline]
#[must_use]
pub const fn field_name_crc(name: &str) -> u32 {
    az_core::crc::crc32_lower(name.as_bytes())
}

const BINARY_STREAM_TAG: u8 = 0;
const XML_STREAM_TAG: u8 = b'<';
const JSON_STREAM_TAG: u8 = b'{';

/// Magic-byte sequences seen at the head of an *uncompressed*
/// `ObjectStream` payload (first byte = binary tag 0, next 4 bytes =
/// version big-endian). Validated against Lumberyard-derived project data.
pub const UNCOMPRESSED_SIGNATURES: [[u8; 5]; 4] = [
    [0x00, 0x00, 0x00, 0x00, 0x03],
    [0x00, 0x00, 0x00, 0x00, 0x02],
    [0x00, 0x00, 0x00, 0x00, 0x01],
    [0x00, 0x00, 0x00, 0x00, 0x00],
];

#[derive(Debug, Error)]
pub enum ObjectStreamError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("not a valid ObjectStream binary payload (tag {0:#x})")]
    InvalidStreamTag(u8),

    #[error("unsupported ObjectStream version {0}")]
    UnsupportedVersion(u32),

    #[error("invalid ObjectStream binary element flags {0:#x}")]
    InvalidElementFlags(u8),

    #[error("ObjectStream binary payload has trailing bytes after the root terminator")]
    TrailingDataAfterRoot,

    #[error("ObjectStream encoding mismatch: expected {expected}, got {actual}")]
    UnexpectedEncoding {
        expected: ObjectStreamEncoding,
        actual: ObjectStreamEncoding,
    },

    #[error("ObjectStream XML is not UTF-8: {0}")]
    Utf8(#[from] str::Utf8Error),

    #[error("ObjectStream XML parse error: {0}")]
    Xml(#[from] quick_xml::DeError),

    #[error("ObjectStream JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("unsupported data-size value-byte width: {0}")]
    UnsupportedSizeWidth(u8),

    #[error("malformed UUID at element header: {0}")]
    Uuid(#[from] uuid::Error),

    #[error("ObjectStream value conversion failed for type {type_id}")]
    ValueConversion {
        type_id: Uuid,
        #[source]
        source: codec::ValueCodecError,
    },

    #[error("ObjectStream class {type_id} is deprecated")]
    DeprecatedClass { type_id: Uuid },

    #[error(
        "ObjectStream class {type_id} element version {element_version} is newer than reflected version {current_version}"
    )]
    NewerClassVersion {
        type_id: Uuid,
        element_version: u32,
        current_version: u32,
    },

    #[error(
        "ObjectStream class {type_id} requires an unsupported version conversion from {element_version} to {current_version}"
    )]
    UnsupportedVersionConversion {
        type_id: Uuid,
        element_version: u32,
        current_version: u32,
    },

    #[error(
        "ObjectStream ClassData converter from {source_type_id} to {target_type_id} was captured but has no registered executable implementation"
    )]
    UnsupportedDataConversion {
        target_type_id: Uuid,
        source_type_id: Uuid,
    },

    #[error("ObjectStream ClassData {type_id} has a captured serializer with no executable codec")]
    UnsupportedSerializer { type_id: Uuid },

    #[error("ObjectStream element {type_id} has no resolved ClassData serializer")]
    MissingSerializer { type_id: Uuid },

    #[error("ObjectStream element {type_id} has unresolved wire identity")]
    UnresolvedElementType { type_id: Uuid },

    #[error(
        "ObjectStream generic type {type_id} has multiple distinct native multimap targets; root resolution is ambiguous"
    )]
    AmbiguousGenericType { type_id: Uuid },

    #[error("ObjectStream ClassData identity {class:?} is not defined in the read context")]
    UnresolvedClassData { class: context::ReflectedClassKey },

    #[error("ObjectStream v2 element {type_id} is missing specialization UUID")]
    MissingV2Specialization { type_id: Uuid },

    #[error("ObjectStream v{stream_version} element {type_id} has an illegal specialization UUID")]
    UnexpectedSpecialization { stream_version: u32, type_id: Uuid },

    #[error("ObjectStream element version {version} exceeds binary u8 storage")]
    ElementVersionOverflow { version: u32 },

    #[error("ObjectStream element {type_id} payload is too large: {size} bytes")]
    PayloadTooLarge { type_id: Uuid, size: usize },

    #[error(
        "ObjectStream container {type_id} has {actual} elements, violating captured {expected:?} cardinality"
    )]
    InvalidContainerCardinality {
        type_id: Uuid,
        expected: Option<context::ContainerCardinality>,
        actual: usize,
    },

    #[error("ObjectStream container {type_id} has no captured native IDataContainer family")]
    UnsupportedContainerSemantics { type_id: Uuid },

    #[error("ObjectStream container {type_id} has no captured native cardinality contract")]
    MissingContainerCardinality { type_id: Uuid },

    #[error(
        "ObjectStream field `{field}` CRC mismatch: stored {stored:#010x}, computed {computed:#010x}"
    )]
    FieldCrcMismatch {
        field: String,
        stored: u32,
        computed: u32,
    },

    #[error("ObjectStream text output cannot resolve field name for CRC {crc:#010x}")]
    MissingFieldName { crc: u32 },

    #[error("ObjectStream text output cannot resolve ClassData name for type {type_id}")]
    MissingTypeName { type_id: Uuid },

    #[error(
        "ObjectStream child {child_type_id} with field CRC {name_crc:?} is not reachable from reflected parent {parent_type_id}"
    )]
    UnexpectedReachableChild {
        parent_type_id: Uuid,
        child_type_id: Uuid,
        name_crc: Option<u32>,
    },

    #[error("ObjectStream DataOverlayInfo is malformed: {0}")]
    InvalidDataOverlay(String),

    #[error("ObjectStream DataOverlayInfo provider {provider_id:#010x} is not registered")]
    MissingDataOverlayProvider { provider_id: u32 },

    #[error("ObjectStream SerializeContext registration is incomplete: {0}")]
    IncompleteReadContext(#[from] context::SerializerRegistrationError),
}

/// First byte of an `ObjectStream` payload — flags the binary / XML /
/// JSON variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StreamTag(pub u8);

impl StreamTag {
    pub const BINARY: Self = Self(BINARY_STREAM_TAG);
    pub const XML: Self = Self(XML_STREAM_TAG);
    pub const JSON: Self = Self(JSON_STREAM_TAG);

    /// Detect the stream variant from the first byte. Returns
    /// `None` if the byte doesn't match any known tag.
    #[inline]
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            BINARY_STREAM_TAG => Some(Self::BINARY),
            XML_STREAM_TAG => Some(Self::XML),
            JSON_STREAM_TAG => Some(Self::JSON),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn is_binary(self) -> bool {
        self.0 == BINARY_STREAM_TAG
    }

    #[inline]
    #[must_use]
    pub const fn is_xml(self) -> bool {
        self.0 == XML_STREAM_TAG
    }

    #[inline]
    #[must_use]
    pub const fn is_json(self) -> bool {
        self.0 == JSON_STREAM_TAG
    }
}

pub(crate) const fn validate_stream_version(version: u32) -> Result<u32, ObjectStreamError> {
    // SerializeContext::ObjectStream::Start accepts every historical version
    // through the current v3 format. Version 0 follows the same element-header
    // path as v1 (no specialization UUID).
    if version <= 3 {
        Ok(version)
    } else {
        Err(ObjectStreamError::UnsupportedVersion(version))
    }
}

pub(crate) const fn validate_element_specialization(
    stream_version: u32,
    type_id: Uuid,
    specialization: Option<Uuid>,
) -> Result<(), ObjectStreamError> {
    match (stream_version, specialization) {
        (0 | 1 | 3, Some(_)) => Err(ObjectStreamError::UnexpectedSpecialization {
            stream_version,
            type_id,
        }),
        _ => Ok(()),
    }
}

impl Default for StreamTag {
    fn default() -> Self {
        Self::BINARY
    }
}

impl fmt::Display for StreamTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match *self {
            Self::BINARY => "BINARY",
            Self::XML => "XML",
            Self::JSON => "JSON",
            _ => "UNKNOWN",
        })
    }
}

impl PartialEq<u8> for StreamTag {
    #[inline]
    fn eq(&self, other: &u8) -> bool {
        self.0 == *other
    }
}

/// Supported `ObjectStream` encodings.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ObjectStreamEncoding {
    /// Lumberyard binary `ObjectStream`.
    #[default]
    Binary,
    /// XML `ObjectStream`.
    Xml,
    /// JSON `ObjectStream`.
    Json,
}

/// Representation retained for an element payload.
///
/// Mirrors Lumberyard/O3DE `DataElement::DataType`. Binary `ObjectStream` values
/// are big-endian, serializer `TextToData` output can be native-endian, and
/// unknown XML/JSON values remain text until a registered codec converts them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PayloadEncoding {
    #[default]
    BinaryBigEndian,
    BinaryNativeEndian,
    Text,
}

/// Public diagnostic state for an element's wire identity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeResolutionState {
    #[default]
    Raw,
    Unresolved,
    Resolved,
}

/// Context-derived semantic proof. This is deliberately crate-private: typed
/// readers must not accept caller-manufactured ClassData/serializer/container
/// claims.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TypeResolution {
    /// Parsed for inspection without a reflection context.
    #[default]
    Raw,
    /// A context was supplied but native `ClassData` lookup failed.
    Unresolved,
    /// Native `ClassData` lookup succeeded and selected this semantic identity.
    Resolved {
        type_id: Uuid,
        class: context::ReflectedClassKey,
        edge: Option<context::ReflectedEdge>,
        enum_type_id: Option<Uuid>,
        builtin_serializer: Option<codec::BuiltinSerializerDescriptor>,
        is_container: bool,
        container_shape: Option<context::ContainerShape>,
    },
}

impl ObjectStreamEncoding {
    #[inline]
    #[must_use]
    pub const fn from_tag(tag: StreamTag) -> Option<Self> {
        match tag {
            StreamTag::BINARY => Some(Self::Binary),
            StreamTag::XML => Some(Self::Xml),
            StreamTag::JSON => Some(Self::Json),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn from_tag_byte(byte: u8) -> Option<Self> {
        match byte {
            BINARY_STREAM_TAG => Some(Self::Binary),
            XML_STREAM_TAG => Some(Self::Xml),
            JSON_STREAM_TAG => Some(Self::Json),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn stream_tag(self) -> StreamTag {
        match self {
            Self::Binary => StreamTag::BINARY,
            Self::Xml => StreamTag::XML,
            Self::Json => StreamTag::JSON,
        }
    }

    #[inline]
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Binary => "",
            Self::Xml => "xml",
            Self::Json => "json",
        }
    }
}

impl fmt::Display for ObjectStreamEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Binary => "binary",
            Self::Xml => "xml",
            Self::Json => "json",
        })
    }
}

/// In-memory `ObjectStream` graph.
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ObjectStream {
    pub tag: StreamTag,
    pub version: u32,
    pub elements: Vec<Element>,
    /// Number of subtrees discarded during a context-aware read. Native
    /// loading discards an unregistered class under
    /// [`UnregisteredClassPolicy::NativeSkip`](crate::context::UnregisteredClassPolicy),
    /// an unbound child under
    /// [`UnmatchedChildPolicy::NativeSkip`](crate::context::UnmatchedChildPolicy),
    /// or a deprecated `ClassData` that has no converter. This is a diagnostic
    /// side count, not part of the serialized wire form.
    #[serde(skip)]
    pub skipped_children: usize,
}

impl ObjectStream {
    /// Construct an empty binary stream at `version`.
    #[inline]
    #[must_use]
    pub const fn new(version: u32) -> Self {
        Self {
            tag: StreamTag::BINARY,
            version,
            elements: Vec::new(),
            skipped_children: 0,
        }
    }

    #[inline]
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    #[inline]
    #[must_use]
    pub const fn tag(&self) -> StreamTag {
        self.tag
    }

    /// Encoding detected when this `ObjectStream` was read.
    ///
    /// The decoded graph is independent of this value; write methods
    /// choose their output encoding explicitly.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamError::InvalidStreamTag`] if this stream's `tag` byte
    /// is not one of the binary, XML or JSON tags — only reachable on a value
    /// assembled by hand rather than read from a payload.
    #[inline]
    pub fn encoding(&self) -> Result<ObjectStreamEncoding, ObjectStreamError> {
        ObjectStreamEncoding::from_tag(self.tag)
            .ok_or(ObjectStreamError::InvalidStreamTag(self.tag.0))
    }

    /// Top-level elements (non-recursive).
    #[inline]
    #[must_use]
    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    /// Iterate the top-level element list.
    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, Element> {
        self.elements.iter()
    }

    /// Iterate every element in the tree (depth-first pre-order).
    #[inline]
    #[must_use]
    pub fn iter_recursive(&self) -> RecursiveElements<'_> {
        RecursiveElements::new(&self.elements)
    }

    /// Depth-first search for the first element matching `query`.
    pub fn find<F>(&self, query: F) -> Option<&Element>
    where
        F: Fn(&Element) -> bool,
    {
        for element in &self.elements {
            if let Some(result) = element.find(&query) {
                return Some(result);
            }
        }
        None
    }

    /// Write the binary form back to a writer (round-trips
    /// [`Self::from_reader`]).
    ///
    /// # Errors
    ///
    /// Returns any error [`serialize::to_bytes`] returns —
    /// [`ObjectStreamError::UnsupportedVersion`],
    /// [`ObjectStreamError::FieldCrcMismatch`],
    /// [`ObjectStreamError::ElementVersionOverflow`],
    /// [`ObjectStreamError::MissingV2Specialization`],
    /// [`ObjectStreamError::UnexpectedSpecialization`] and
    /// [`ObjectStreamError::PayloadTooLarge`] — plus [`ObjectStreamError::Io`] if
    /// `writer` rejects the bytes.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<(), ObjectStreamError> {
        serialize::write_to(self, writer)
    }

    /// Parse binary, XML, or JSON `ObjectStream` bytes for raw inspection.
    /// Typed decoding requires [`Self::from_bytes_with_context`].
    ///
    /// # Errors
    ///
    /// Returns any error [`deserialize::from_bytes`] returns:
    /// [`ObjectStreamError::Io`] for an empty or truncated payload,
    /// [`ObjectStreamError::InvalidStreamTag`] for an unknown leading byte,
    /// [`ObjectStreamError::UnsupportedVersion`],
    /// [`ObjectStreamError::InvalidElementFlags`],
    /// [`ObjectStreamError::UnsupportedSizeWidth`],
    /// [`ObjectStreamError::Uuid`] and
    /// [`ObjectStreamError::TrailingDataAfterRoot`] from the binary reader, and
    /// [`ObjectStreamError::Utf8`], [`ObjectStreamError::Xml`],
    /// [`ObjectStreamError::Json`], [`ObjectStreamError::MissingV2Specialization`],
    /// [`ObjectStreamError::UnexpectedSpecialization`] and
    /// [`ObjectStreamError::ValueConversion`] from the text readers.
    #[inline]
    pub fn from_bytes(
        bytes: &[u8],
        hashes: Option<&LumberyardHashes>,
    ) -> Result<Self, ObjectStreamError> {
        deserialize::from_bytes(bytes, hashes)
    }

    /// Parse using parent-aware reflected type metadata.
    ///
    /// This is the strict semantic entrypoint: every reached element must
    /// resolve to captured `ClassData`. Use [`Self::from_bytes`] only for raw
    /// carrier diagnostics that deliberately have no typed identity.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamError::IncompleteReadContext`] if `context` still has
    /// captured serializers with no registered implementation, every error
    /// [`Self::from_bytes`] returns, and the reflection failures resolution adds:
    /// [`ObjectStreamError::UnresolvedElementType`],
    /// [`ObjectStreamError::UnresolvedClassData`],
    /// [`ObjectStreamError::AmbiguousGenericType`],
    /// [`ObjectStreamError::DeprecatedClass`],
    /// [`ObjectStreamError::NewerClassVersion`],
    /// [`ObjectStreamError::UnsupportedVersionConversion`],
    /// [`ObjectStreamError::UnsupportedDataConversion`],
    /// [`ObjectStreamError::UnsupportedSerializer`],
    /// [`ObjectStreamError::MissingSerializer`],
    /// [`ObjectStreamError::InvalidContainerCardinality`],
    /// [`ObjectStreamError::MissingContainerCardinality`],
    /// [`ObjectStreamError::UnsupportedContainerSemantics`],
    /// [`ObjectStreamError::UnexpectedReachableChild`],
    /// [`ObjectStreamError::InvalidDataOverlay`] and
    /// [`ObjectStreamError::MissingDataOverlayProvider`].
    #[inline]
    pub fn from_bytes_with_context(
        bytes: &[u8],
        context: &context::ObjectStreamReadContext,
    ) -> Result<Self, ObjectStreamError> {
        deserialize::from_bytes_with_context(bytes, context)
    }

    /// Read a binary `ObjectStream` payload for raw inspection.
    /// Typed decoding requires [`Self::from_reader_with_context`].
    ///
    /// # Errors
    ///
    /// Returns the binary-reader errors listed on [`Self::from_bytes`], with
    /// [`ObjectStreamError::Io`] additionally covering any failure of `reader`
    /// itself. Text payloads are not accepted here — a non-binary tag is
    /// [`ObjectStreamError::InvalidStreamTag`].
    #[inline]
    pub fn from_reader<R: Read>(
        reader: &mut R,
        hashes: Option<&LumberyardHashes>,
    ) -> Result<Self, ObjectStreamError> {
        deserialize::from_reader(reader, hashes)
    }

    /// Read binary `ObjectStream` data using parent-aware reflected metadata.
    /// Every reached element must resolve to captured `ClassData`.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamError::IncompleteReadContext`] for an incomplete
    /// `context`, the binary-reader errors listed on [`Self::from_reader`], and the
    /// reflection failures listed on [`Self::from_bytes_with_context`].
    #[inline]
    pub fn from_reader_with_context<R: Read>(
        reader: &mut R,
        context: &context::ObjectStreamReadContext,
    ) -> Result<Self, ObjectStreamError> {
        deserialize::from_reader_with_context(reader, context)
    }

    /// Parse `ObjectStream` bytes that must use `encoding`.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamError::UnexpectedEncoding`] when the payload's own
    /// leading tag does not name `encoding`, plus every error [`Self::from_bytes`]
    /// returns.
    #[inline]
    pub fn from_encoding_bytes(
        bytes: &[u8],
        encoding: ObjectStreamEncoding,
        hashes: Option<&LumberyardHashes>,
    ) -> Result<Self, ObjectStreamError> {
        deserialize::from_encoding_bytes(bytes, encoding, hashes)
    }

    /// Parse `ObjectStream` bytes that must use `encoding`, with strict
    /// parent-aware `ClassData` and serializer resolution.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamError::UnexpectedEncoding`] when the payload is not in
    /// `encoding`, plus every error [`Self::from_bytes_with_context`] returns.
    #[inline]
    pub fn from_encoding_bytes_with_context(
        bytes: &[u8],
        encoding: ObjectStreamEncoding,
        context: &context::ObjectStreamReadContext,
    ) -> Result<Self, ObjectStreamError> {
        deserialize::from_encoding_bytes_with_context(bytes, encoding, context)
    }

    /// Encode this `ObjectStream` in binary form.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamError::UnsupportedVersion`] for a stream version above
    /// 3, [`ObjectStreamError::FieldCrcMismatch`] when an element's stored
    /// `name_crc` disagrees with its field name,
    /// [`ObjectStreamError::ElementVersionOverflow`] for an element version above
    /// 255, [`ObjectStreamError::MissingV2Specialization`] and
    /// [`ObjectStreamError::UnexpectedSpecialization`] for specialization UUIDs
    /// that do not match the stream version, and
    /// [`ObjectStreamError::PayloadTooLarge`] for a payload wider than `u32`.
    #[inline]
    pub fn to_bytes(&self) -> Result<Vec<u8>, ObjectStreamError> {
        serialize::to_bytes(self)
    }

    /// Encode this `ObjectStream` in the requested encoding.
    ///
    /// # Errors
    ///
    /// Returns everything [`Self::to_bytes`] returns, plus
    /// [`ObjectStreamError::MissingSerializer`] when `encoding` differs from the
    /// encoding this stream was read in and some element carries a non-text
    /// payload — that re-encode needs
    /// [`Self::to_encoding_bytes_with_context`].
    #[inline]
    pub fn to_encoding_bytes(
        &self,
        encoding: ObjectStreamEncoding,
    ) -> Result<Vec<u8>, ObjectStreamError> {
        serialize::to_encoding_bytes(self, encoding)
    }

    /// Convert `ObjectStream` bytes into another `ObjectStream` encoding.
    ///
    /// Binary inputs use the crate's streaming reader/writer path so
    /// deep reflected assets can be dumped for research without
    /// growing the process stack.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamError::Io`] for an empty payload,
    /// [`ObjectStreamError::InvalidStreamTag`] for an unknown leading byte, every
    /// error the reader for that encoding raises (see [`Self::from_bytes`]), and
    /// [`ObjectStreamError::MissingSerializer`] when a binary input carrying
    /// non-text payloads must change encoding — that case needs
    /// [`Self::transcode_bytes_with_context`].
    #[inline]
    pub fn transcode_bytes(
        bytes: &[u8],
        encoding: ObjectStreamEncoding,
        hashes: Option<&LumberyardHashes>,
    ) -> Result<Vec<u8>, ObjectStreamError> {
        serialize::transcode_bytes(bytes, encoding, hashes)
    }

    /// Write `ObjectStream` bytes in another `ObjectStream` encoding.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::transcode_bytes`] returns, or
    /// [`ObjectStreamError::Io`] if `writer` rejects the transcoded bytes.
    #[inline]
    pub fn transcode_to_writer<W: Write>(
        bytes: &[u8],
        encoding: ObjectStreamEncoding,
        hashes: Option<&LumberyardHashes>,
        writer: &mut W,
    ) -> Result<(), ObjectStreamError> {
        serialize::transcode_to_writer(bytes, encoding, hashes, writer)
    }

    /// Write this `ObjectStream` in the requested encoding.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::to_encoding_bytes`] returns, or
    /// [`ObjectStreamError::Io`] if `writer` rejects the encoded bytes.
    #[inline]
    pub fn write_as<W: Write>(
        &self,
        encoding: ObjectStreamEncoding,
        writer: &mut W,
    ) -> Result<(), ObjectStreamError> {
        serialize::write_as(self, encoding, writer)
    }

    /// Write with reflected `ClassData` serializer semantics.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::to_encoding_bytes_with_context`] returns, or
    /// [`ObjectStreamError::Io`] if `writer` rejects the encoded bytes.
    pub fn write_as_with_context<W: Write>(
        &self,
        encoding: ObjectStreamEncoding,
        context: &context::ObjectStreamReadContext,
        writer: &mut W,
    ) -> Result<(), ObjectStreamError> {
        serialize::write_as_with_context(self, encoding, context, writer)
    }

    /// Encode this `ObjectStream` in `encoding` using reflected `ClassData`.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamError::IncompleteReadContext`] for an incomplete
    /// `context`, everything [`Self::to_bytes`] returns, and the graph-validation
    /// and serializer failures reflection adds:
    /// [`ObjectStreamError::UnresolvedElementType`],
    /// [`ObjectStreamError::UnexpectedReachableChild`],
    /// [`ObjectStreamError::InvalidContainerCardinality`] and
    /// [`ObjectStreamError::ValueConversion`].
    pub fn to_encoding_bytes_with_context(
        &self,
        encoding: ObjectStreamEncoding,
        context: &context::ObjectStreamReadContext,
    ) -> Result<Vec<u8>, ObjectStreamError> {
        serialize::to_encoding_bytes_with_context(self, encoding, context)
    }

    /// Convert `ObjectStream` bytes into `encoding` using reflected `ClassData`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::from_bytes_with_context`] returns while reading
    /// `bytes`, then any error [`Self::to_encoding_bytes_with_context`] returns
    /// while writing the decoded stream back out.
    pub fn transcode_bytes_with_context(
        bytes: &[u8],
        encoding: ObjectStreamEncoding,
        context: &context::ObjectStreamReadContext,
    ) -> Result<Vec<u8>, ObjectStreamError> {
        serialize::transcode_bytes_with_context(bytes, encoding, context)
    }
}

impl<'a> IntoIterator for &'a ObjectStream {
    type Item = &'a Element;
    type IntoIter = std::slice::Iter<'a, Element>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "ObjectStream")]
pub struct XMLObjectStream {
    #[serde(rename = "@version")]
    pub version: u32,
    #[serde(default, rename = "Class")]
    pub elements: Vec<XMLElement>,
}

impl XMLObjectStream {
    /// Serialize this XML `ObjectStream` document into `buf`.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error::other`] wrapping the `quick_xml` serializer failure if
    /// the document cannot be rendered, or the `io::Error` `buf` reports while the
    /// rendered text is copied into it.
    pub fn write_to(&mut self, buf: &mut impl Write) -> io::Result<u64> {
        let string = quick_xml::se::to_string(self).map_err(io::Error::other)?;
        io::copy(&mut Cursor::new(string), buf)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JSONObjectStream {
    pub name: String,
    pub version: u32,
    #[serde(rename = "Objects")]
    pub elements: Vec<JSONElement>,
}

impl JSONObjectStream {
    /// Serialize this JSON `ObjectStream` document into `buf`.
    ///
    /// # Errors
    ///
    /// Returns the `serde_json` error for any failure of `buf`, which is the only
    /// way this can fail — the document itself always serializes.
    pub fn write_to(&self, buf: &mut impl Write) -> serde_json::Result<()> {
        serde_json::to_writer_pretty(buf, self)
    }
}

/// One node in an `ObjectStream` tree.
///
/// `name` and `field` are [`ArcStr`]s — cloning them is an atomic
/// refcount bump, and the bytes are shared across every `Element`
/// that resolved to the same lookup table entry.
#[derive(PartialEq, Default, Debug, Serialize, Deserialize)]
pub struct Element {
    pub flags: u8,
    pub name_crc: Option<u32>,
    pub version: Option<u32>,
    #[serde(with = "uuid::serde::compact")]
    pub id: Uuid,
    pub specialization: Option<Uuid>,
    #[serde(skip)]
    resolution: TypeResolution,
    pub version_state: context::VersionConversionState,
    #[serde(skip)]
    pub name: ArcStr,
    pub data_size: Option<usize>,
    pub data: Option<Vec<u8>>,
    pub payload_encoding: PayloadEncoding,
    pub elements: Vec<Self>,
    pub field: Option<ArcStr>,
}

impl Element {
    #[inline]
    #[must_use]
    pub fn new(id: Uuid) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }

    /// Set the exact serialized header flags for a raw wire fixture.
    ///
    /// This does not manufacture semantic `ClassData` resolution. It is intended
    /// for source-format tests and tools that need to construct canonical
    /// `ObjectStream` bytes.
    #[inline]
    #[must_use]
    pub const fn with_wire_flags(mut self, flags: u8) -> Self {
        self.flags = flags;
        self
    }

    /// Set the serialized field-name CRC for a raw wire fixture.
    #[inline]
    #[must_use]
    pub const fn with_name_crc(mut self, name_crc: u32) -> Self {
        self.name_crc = Some(name_crc);
        self
    }

    /// Set the serialized element version for a raw wire fixture.
    #[inline]
    #[must_use]
    pub const fn with_version(mut self, version: u32) -> Self {
        self.version = Some(version);
        self
    }

    /// Set the serialized specialization UUID for a raw wire fixture.
    #[inline]
    #[must_use]
    pub const fn with_specialization(mut self, specialization: Uuid) -> Self {
        self.specialization = Some(specialization);
        self
    }

    /// Set the payload size encoded by an extra-size header field.
    #[inline]
    #[must_use]
    pub const fn with_declared_data_size(mut self, data_size: usize) -> Self {
        self.data_size = Some(data_size);
        self
    }

    /// Set the retained representation of the element payload.
    #[inline]
    #[must_use]
    pub const fn with_payload_encoding(mut self, encoding: PayloadEncoding) -> Self {
        self.payload_encoding = encoding;
        self
    }

    #[inline]
    #[must_use]
    pub fn with_field(mut self, field: impl Into<ArcStr>) -> Self {
        self.field = Some(field.into());
        self
    }

    /// Set the element display/type name (XML `name=` / Class node name).
    ///
    /// Distinct from [`Self::with_field`]: field is the reflected slot name
    /// (CRC-bearing), while name is the type/display label used by dumps and
    /// hash-less fixtures.
    #[inline]
    #[must_use]
    pub fn with_name(mut self, name: impl Into<ArcStr>) -> Self {
        self.name = name.into();
        self
    }

    #[inline]
    #[must_use]
    pub fn with_data(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.data = Some(data.into());
        self
    }

    #[inline]
    #[must_use]
    pub fn with_children(mut self, children: impl Into<Vec<Self>>) -> Self {
        self.elements = children.into();
        self
    }

    /// Unmodified UUID from the serialized element header.
    ///
    /// Prefer [`Self::raw_type_id`] for diagnostics and
    /// [`Self::resolved_type_id`] for all typed dispatch. This compatibility
    /// spelling carries no reflection proof.
    #[deprecated(
        since = "0.1.0",
        note = "id() is raw wire identity; use raw_type_id() for diagnostics or resolved_type_id() for typed dispatch"
    )]
    #[inline]
    #[must_use]
    pub const fn id(&self) -> &Uuid {
        self.raw_type_id()
    }

    #[cfg(test)]
    const fn ensure_test_resolution(&mut self) {
        if !matches!(self.resolution, TypeResolution::Resolved { .. }) {
            self.resolution = TypeResolution::Resolved {
                type_id: self.id,
                class: context::ReflectedClassKey::new(self.id),
                edge: None,
                enum_type_id: None,
                builtin_serializer: None,
                is_container: false,
                container_shape: None,
            };
        }
    }

    /// Crate-test-only reflected `ClassData` fixture proof.
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn with_test_class(mut self) -> Self {
        self.ensure_test_resolution();
        self
    }

    /// Crate-test-only reflected serializer fixture proof.
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn with_builtin_serializer(
        mut self,
        serializer: codec::BuiltinSerializerDescriptor,
    ) -> Self {
        self.ensure_test_resolution();
        if let TypeResolution::Resolved {
            builtin_serializer, ..
        } = &mut self.resolution
        {
            *builtin_serializer = Some(serializer);
        }
        self
    }

    /// Crate-test-only native container-family fixture proof.
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn with_container_shape(mut self, shape: context::ContainerShape) -> Self {
        self.ensure_test_resolution();
        if let TypeResolution::Resolved {
            is_container,
            container_shape,
            ..
        } = &mut self.resolution
        {
            *is_container = true;
            *container_shape = Some(shape);
        }
        self
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn with_test_base_class_edge(mut self) -> Self {
        self.ensure_test_resolution();
        if let TypeResolution::Resolved { edge, .. } = &mut self.resolution {
            *edge = Some(context::ReflectedEdge {
                is_base_class: true,
                is_pointer: false,
                is_dynamic_field: false,
                asset_type_id: None,
            });
        }
        self
    }

    /// Unmodified UUID from the serialized element header.
    #[inline]
    #[must_use]
    pub const fn raw_type_id(&self) -> &Uuid {
        &self.id
    }

    #[inline]
    #[must_use]
    pub const fn resolved_type_id(&self) -> Option<&Uuid> {
        match &self.resolution {
            TypeResolution::Resolved { type_id, .. } => Some(type_id),
            TypeResolution::Raw | TypeResolution::Unresolved => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn resolved_class(&self) -> Option<context::ReflectedClassKey> {
        match self.resolution {
            TypeResolution::Resolved { class, .. } => Some(class),
            TypeResolution::Raw | TypeResolution::Unresolved => None,
        }
    }

    /// Captured enum identity when this value is stored through a reflected
    /// EnumType-to-underlying mapping. The numeric serializer remains the
    /// underlying `ClassData` serializer; this UUID proves the enum association.
    #[inline]
    #[must_use]
    pub const fn reflected_enum_type_id(&self) -> Option<&Uuid> {
        match &self.resolution {
            TypeResolution::Resolved { enum_type_id, .. } => enum_type_id.as_ref(),
            TypeResolution::Raw | TypeResolution::Unresolved => None,
        }
    }

    /// Asset class carried by the reflected parent field's `Asset<T>` type.
    #[inline]
    #[must_use]
    pub const fn reflected_asset_type_id(&self) -> Option<&Uuid> {
        match &self.resolution {
            TypeResolution::Resolved {
                edge: Some(context::ReflectedEdge { asset_type_id, .. }),
                ..
            } => asset_type_id.as_ref(),
            TypeResolution::Raw
            | TypeResolution::Unresolved
            | TypeResolution::Resolved { edge: None, .. } => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn builtin_serializer(&self) -> Option<codec::BuiltinSerializerDescriptor> {
        match self.resolution {
            TypeResolution::Resolved {
                builtin_serializer, ..
            } => builtin_serializer,
            TypeResolution::Raw | TypeResolution::Unresolved => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn is_resolved_container(&self) -> bool {
        matches!(
            self.resolution,
            TypeResolution::Resolved {
                is_container: true,
                ..
            }
        )
    }

    #[inline]
    #[must_use]
    pub const fn container_shape(&self) -> Option<context::ContainerShape> {
        match self.resolution {
            TypeResolution::Resolved {
                container_shape, ..
            } => container_shape,
            TypeResolution::Raw | TypeResolution::Unresolved => None,
        }
    }

    /// Whether this element is reached through a captured base-class
    /// `ClassElement` on its parent.
    #[inline]
    #[must_use]
    pub const fn is_base_class_edge(&self) -> bool {
        matches!(
            self.resolution,
            TypeResolution::Resolved {
                edge: Some(context::ReflectedEdge {
                    is_base_class: true,
                    ..
                }),
                ..
            }
        )
    }

    /// Whether this element is reached through a captured pointer
    /// `ClassElement` on its parent.
    #[inline]
    #[must_use]
    pub const fn is_pointer_edge(&self) -> bool {
        matches!(
            self.resolution,
            TypeResolution::Resolved {
                edge: Some(context::ReflectedEdge {
                    is_pointer: true,
                    ..
                }),
                ..
            }
        )
    }

    /// Whether this element is reached through a captured dynamic-field
    /// `ClassElement` on its parent.
    #[inline]
    #[must_use]
    pub const fn is_dynamic_field_edge(&self) -> bool {
        matches!(
            self.resolution,
            TypeResolution::Resolved {
                edge: Some(context::ReflectedEdge {
                    is_dynamic_field: true,
                    ..
                }),
                ..
            }
        )
    }

    #[inline]
    #[must_use]
    pub const fn type_resolution(&self) -> TypeResolutionState {
        match self.resolution {
            TypeResolution::Raw => TypeResolutionState::Raw,
            TypeResolution::Unresolved => TypeResolutionState::Unresolved,
            TypeResolution::Resolved { .. } => TypeResolutionState::Resolved,
        }
    }

    /// Human-readable type name (only populated when a
    /// [`LumberyardHashes`] table was supplied during parse).
    /// Returns the wrapped [`ArcStr`] — `Deref`s to `&str` for
    /// most uses; clone it cheaply via [`ArcStr::clone`] when you
    /// want to keep an owned reference.
    #[inline]
    #[must_use]
    pub const fn name(&self) -> &ArcStr {
        &self.name
    }

    /// Resolved field name (from the [`LumberyardHashes`] CRC
    /// dump), if any.
    #[inline]
    #[must_use]
    pub const fn field(&self) -> Option<&ArcStr> {
        self.field.as_ref()
    }

    /// Field name CRC, if [`ST_BINARYFLAG_HAS_NAME`] was set.
    #[inline]
    #[must_use]
    pub const fn name_crc(&self) -> Option<u32> {
        self.name_crc
    }

    /// Class version, if [`ST_BINARYFLAG_HAS_VERSION`] was set.
    #[inline]
    #[must_use]
    pub const fn version(&self) -> Option<u32> {
        self.version
    }

    /// Element specialization UUID (only present when the parent
    /// stream is `version == 2`).
    #[inline]
    #[must_use]
    pub const fn specialization(&self) -> Option<&Uuid> {
        self.specialization.as_ref()
    }

    /// Raw payload bytes, if any.
    #[inline]
    #[must_use]
    pub fn data(&self) -> Option<&[u8]> {
        self.data.as_deref()
    }

    #[inline]
    #[must_use]
    pub const fn payload_encoding(&self) -> PayloadEncoding {
        self.payload_encoding
    }

    /// Decode this element's leaf payload as an AZ reflected value.
    ///
    /// # Errors
    ///
    /// Returns whatever `T`'s [`DecodeAzValue`](value::DecodeAzValue)
    /// implementation returns for this element — typically
    /// [`ObjectStreamValueError::UnexpectedType`](value::ObjectStreamValueError::UnexpectedType)
    /// when the element is not the AZ type `T` decodes,
    /// [`ObjectStreamValueError::MissingData`](value::ObjectStreamValueError::MissingData)
    /// when it carries no payload, and
    /// [`ObjectStreamValueError::InvalidLength`](value::ObjectStreamValueError::InvalidLength)
    /// when the payload is the wrong width.
    pub fn decode<'a, T>(&'a self) -> Result<T, value::ObjectStreamValueError>
    where
        T: value::DecodeAzValue<'a>,
    {
        T::decode_az_value(self)
    }

    /// Decode a named child field as an AZ reflected value.
    ///
    /// # Errors
    ///
    /// A missing field is `Ok(None)`, not an error. When the field is present,
    /// returns any error [`Self::decode`] returns for that child.
    pub fn field_value<'a, T>(
        &'a self,
        field: &str,
    ) -> Result<Option<T>, value::ObjectStreamValueError>
    where
        T: value::DecodeAzValue<'a>,
    {
        value::child_by_field(self, field)
            .map(T::decode_az_value)
            .transpose()
    }

    /// Deserialize this element as an `ObjectStream` object.
    ///
    /// # Errors
    ///
    /// Returns any error `T`'s [`object::Deserialize`] implementation returns, plus
    /// [`ObjectStreamValueError::UnknownField`](value::ObjectStreamValueError::UnknownField)
    /// if that implementation leaves one of this element's children unread.
    pub fn deserialize<'a, T>(&'a self) -> Result<T, value::ObjectStreamValueError>
    where
        T: object::Deserialize<'a>,
    {
        object::deserialize(self)
    }

    /// Child elements.
    #[inline]
    #[must_use]
    pub fn children(&self) -> &[Self] {
        &self.elements
    }

    /// Iterate this element's direct children.
    ///
    /// Same order and contents as [`Self::children`], and the inherent
    /// counterpart of `IntoIterator for &Element`.
    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, Self> {
        self.elements.iter()
    }

    /// First direct child whose reflected name CRC equals `name_crc`.
    ///
    /// Version converters run on the raw DOM before children are re-resolved, so
    /// this matches on the serialized name CRC (`AZ_CRC` of the field name), the
    /// same key native `DataElementNode::FindElement` uses.
    #[must_use]
    pub fn child_by_name_crc(&self, name_crc: u32) -> Option<&Self> {
        self.elements
            .iter()
            .find(|child| child.name_crc == Some(name_crc))
    }

    /// Mutable first direct child whose reflected name CRC equals `name_crc`.
    #[must_use]
    pub fn child_by_name_crc_mut(&mut self, name_crc: u32) -> Option<&mut Self> {
        self.elements
            .iter_mut()
            .find(|child| child.name_crc == Some(name_crc))
    }

    /// Remove and return the first direct child whose reflected name CRC equals
    /// `name_crc`, mirroring native `DataElementNode::RemoveElementByName`.
    /// Returns `None` (no-op) when absent, matching the native converters that
    /// guard removals on `FindElement`.
    pub fn remove_child_by_name_crc(&mut self, name_crc: u32) -> Option<Self> {
        let index = self
            .elements
            .iter()
            .position(|child| child.name_crc == Some(name_crc))?;
        Some(self.elements.remove(index))
    }

    /// `true` iff this element has no child elements.
    #[inline]
    #[must_use]
    pub const fn is_leaf(&self) -> bool {
        self.elements.is_empty()
    }

    /// `true` iff [`ST_BINARYFLAG_HAS_NAME`] is set in `flags`.
    #[inline]
    #[must_use]
    pub const fn has_name(&self) -> bool {
        self.flags & ST_BINARYFLAG_HAS_NAME != 0
    }

    /// `true` iff [`ST_BINARYFLAG_HAS_VALUE`] is set in `flags`.
    #[inline]
    #[must_use]
    pub const fn has_value(&self) -> bool {
        self.flags & ST_BINARYFLAG_HAS_VALUE != 0
    }

    /// `true` iff [`ST_BINARYFLAG_HAS_VERSION`] is set in `flags`.
    #[inline]
    #[must_use]
    pub const fn has_version(&self) -> bool {
        self.flags & ST_BINARYFLAG_HAS_VERSION != 0
    }

    /// `true` iff [`ST_BINARYFLAG_EXTRA_SIZE_FIELD`] is set in `flags`.
    #[inline]
    #[must_use]
    pub const fn has_extra_size_field(&self) -> bool {
        self.flags & ST_BINARYFLAG_EXTRA_SIZE_FIELD != 0
    }

    /// Encoded value-byte width (1, 2, or 4) from the bottom 3 bits
    /// of `flags`.
    #[inline]
    #[must_use]
    pub const fn value_width(&self) -> u8 {
        self.flags & ST_BINARY_VALUE_SIZE_MASK
    }

    /// Iterate all elements in this subtree (depth-first pre-order,
    /// includes `self`).
    #[inline]
    #[must_use]
    pub const fn iter_recursive(&self) -> RecursiveElement<'_> {
        RecursiveElement {
            root: Some(self),
            stack: Vec::new(),
        }
    }

    /// Depth-first search rooted at this element.
    pub fn find<F>(&self, query: &F) -> Option<&Self>
    where
        F: Fn(&Self) -> bool,
    {
        if query(self) {
            return Some(self);
        }
        for child in &self.elements {
            if let Some(result) = child.find(query) {
                return Some(result);
            }
        }
        None
    }
}

impl<'a> IntoIterator for &'a Element {
    type Item = &'a Element;
    type IntoIter = std::slice::Iter<'a, Element>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.elements.iter()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct XMLElement {
    #[serde(default, rename = "@name")]
    pub name: String,
    #[serde(rename = "@field", skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(rename = "@value", skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(rename = "@version", skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(rename = "@type", with = "uuid_braced_uppercase")]
    pub id: Uuid,
    #[serde(
        rename = "@specializationTypeId",
        with = "option_braced_uppercase",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub specialization: Option<Uuid>,
    #[serde(default, rename = "Class")]
    pub elements: Vec<Self>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct JSONElement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(rename = "typeId", with = "uuid_braced_uppercase")]
    pub id: Uuid,
    #[serde(default, rename = "typeName")]
    pub name: String,
    #[serde(
        rename = "specializationTypeId",
        with = "option_braced_uppercase",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub specialization: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(rename = "Objects", skip_serializing_if = "Option::is_none")]
    pub elements: Option<Vec<Self>>,
}

/// Serde helper for braced uppercase UUID rendering (`{ABCDEF...}`).
pub mod uuid_braced_uppercase {
    use serde::{Deserialize, Deserializer, Serializer};
    use uuid::Uuid;

    /// Render `uuid` as a braced uppercase string.
    ///
    /// # Errors
    ///
    /// Returns the serializer's own error; the rendering step cannot fail.
    pub fn serialize<S>(uuid: &Uuid, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = uuid.as_braced().to_string().to_uppercase();
        serializer.serialize_str(&s)
    }

    /// Parse a braced uppercase UUID string.
    ///
    /// # Errors
    ///
    /// Returns the deserializer's own error if the value is not a string, or
    /// [`serde::de::Error::custom`] carrying the `uuid` parse failure if the string
    /// is not a UUID.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Uuid, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Uuid::parse_str(&s).map_err(serde::de::Error::custom)
    }
}

/// Serde helper for `Option<Uuid>` in braced uppercase format.
pub mod option_braced_uppercase {
    use serde::{Deserialize, Deserializer, Serializer};
    use uuid::Uuid;

    /// Render `value` as a braced uppercase string, or as `none`.
    ///
    /// # Errors
    ///
    /// Returns the serializer's own error; the rendering step cannot fail.
    pub fn serialize<S>(value: &Option<Uuid>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(uuid) => super::uuid_braced_uppercase::serialize(uuid, serializer),
            None => serializer.serialize_none(),
        }
    }

    /// Parse an optional braced uppercase UUID string.
    ///
    /// # Errors
    ///
    /// Returns the deserializer's own error if the value is neither a string nor
    /// absent, or [`serde::de::Error::custom`] carrying the `uuid` parse failure if
    /// a present string is not a UUID.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Uuid>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt = Option::<String>::deserialize(deserializer)?;
        opt.map_or_else(
            || Ok(None),
            |s| {
                Uuid::parse_str(&s)
                    .map(Some)
                    .map_err(serde::de::Error::custom)
            },
        )
    }
}

// === recursive iterators ===

/// Depth-first iterator over every element in an [`ObjectStream`]
/// (pre-order). Top-level call order matches `iter_recursive`.
pub struct RecursiveElements<'a> {
    stack: Vec<std::slice::Iter<'a, Element>>,
}

impl<'a> RecursiveElements<'a> {
    fn new(roots: &'a [Element]) -> Self {
        Self {
            stack: vec![roots.iter()],
        }
    }
}

impl<'a> Iterator for RecursiveElements<'a> {
    type Item = &'a Element;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(top) = self.stack.last_mut() {
            if let Some(element) = top.next() {
                if !element.elements.is_empty() {
                    self.stack.push(element.elements.iter());
                }
                return Some(element);
            }
            self.stack.pop();
        }
        None
    }
}

/// Single-rooted depth-first iterator (yields `self` first, then
/// descendants in pre-order).
pub struct RecursiveElement<'a> {
    root: Option<&'a Element>,
    stack: Vec<std::slice::Iter<'a, Element>>,
}

impl<'a> Iterator for RecursiveElement<'a> {
    type Item = &'a Element;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(root) = self.root.take() {
            if !root.elements.is_empty() {
                self.stack.push(root.elements.iter());
            }
            return Some(root);
        }
        while let Some(top) = self.stack.last_mut() {
            if let Some(element) = top.next() {
                if !element.elements.is_empty() {
                    self.stack.push(element.elements.iter());
                }
                return Some(element);
            }
            self.stack.pop();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_round_trip() {
        let xml = r#"<ObjectStream version="3"><Class name="int" field="test" value="2" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/><Class name="Asset" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"><Class name="int" field="element" value="100" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/></Class></ObjectStream>"#;
        let xml_object_stream: XMLObjectStream = quick_xml::de::from_str(xml).unwrap();
        assert_eq!(
            quick_xml::se::to_string(&xml_object_stream)
                .unwrap()
                .as_str(),
            xml
        );
    }

    #[test]
    fn from_bytes_accepts_xml_objectstream() -> Result<(), ObjectStreamError> {
        let xml = br#"<ObjectStream version="3"><Class name="int" field="test" value="2" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/></ObjectStream>"#;
        let stream = ObjectStream::from_bytes(xml, None)?;

        assert_eq!(stream.version(), 3);
        assert_eq!(stream.elements().len(), 1);
        assert_eq!(stream.elements()[0].name().as_str(), "int");
        assert_eq!(
            stream.elements()[0].field().map(arcstr::ArcStr::as_str),
            Some("test")
        );
        Ok(())
    }

    #[test]
    fn xml_scalar_values_decode_as_element_data() -> Result<(), Box<dyn std::error::Error>> {
        let xml = br#"<ObjectStream version="3"><Class name="int" field="test" value="2" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/></ObjectStream>"#;
        let context = builtin_context(types::INT, 0);
        let stream = ObjectStream::from_bytes_with_context(xml, &context)?;

        assert_eq!(value::read_i32(&stream.elements()[0])?, 2);
        Ok(())
    }

    #[test]
    fn xml_asset_values_decode_as_binary_asset_data() -> Result<(), Box<dyn std::error::Error>> {
        let xml = br#"<ObjectStream version="3"><Class name="Asset" field="Material" value="id={1E9A1948-F2A6-5500-B918-964558497331}:7,type={F46985B5-F7FF-4FCB-8E8C-DC240D701841},hint={materials/terrain/foo.mtl}" version="1" type="{77A19D40-8731-4D3C-9041-1B43047366A4}"/></ObjectStream>"#;
        let context = builtin_context(types::ASSET, 1);
        let stream = ObjectStream::from_bytes_with_context(xml, &context)?;
        let asset = asset_reference::read_asset_value(&stream.elements()[0])?;

        assert_eq!(
            asset.guid(),
            Uuid::parse_str("1E9A1948-F2A6-5500-B918-964558497331")?
        );
        assert_eq!(asset.sub_id(), 7);
        assert_eq!(
            asset.asset_type(),
            Uuid::parse_str("F46985B5-F7FF-4FCB-8E8C-DC240D701841")?
        );
        assert_eq!(asset.hint(), "materials/terrain/foo.mtl");
        Ok(())
    }

    #[test]
    fn reflected_asset_field_preserves_templated_asset_type() -> Result<(), ObjectStreamError> {
        let parent_type = Uuid::from_u128(0x100);
        let asset_type = Uuid::from_u128(0x200);
        let mut context = context::ObjectStreamReadContext::default();
        let asset_class = context.insert_class(
            types::ASSET,
            context::ReflectedClass::new(types::ASSET).with_version(1),
        )?;
        context.insert_builtin_codec(asset_class, types::ASSET, 1)?;

        let mut parent = context::ReflectedClass::new(parent_type);
        parent.insert_named_field(
            "Slice",
            context::ReflectedField::new(types::ASSET).with_generic(
                context::GenericClassInfo::new(asset_class, types::ASSET, Some(types::ASSET), None)
                    .with_templated_type_ids([asset_type]),
            ),
        );
        context.insert_class(parent_type, parent)?;

        let bytes = ObjectStream {
            version: 3,
            elements: vec![
                Element::new(parent_type)
                    .with_children([Element::new(types::ASSET).with_field("Slice")]),
            ],
            ..ObjectStream::default()
        }
        .to_bytes()?;
        let stream = ObjectStream::from_bytes_with_context(&bytes, &context)?;

        assert_eq!(
            stream.elements()[0].children()[0].reflected_asset_type_id(),
            Some(&asset_type)
        );
        Ok(())
    }

    #[test]
    fn xml_byte_stream_values_decode_as_binary_data() -> Result<(), Box<dyn std::error::Error>> {
        let xml = br#"<ObjectStream version="3"><Class name="ByteStream" field="Data" value="000102FF" type="{ADFD596B-7177-5519-9752-BC418FE42963}"/></ObjectStream>"#;
        let context = builtin_context(types::BYTE_STREAM, 0);
        let stream = ObjectStream::from_bytes_with_context(xml, &context)?;

        assert_eq!(
            value::read_byte_stream(&stream.elements()[0])?,
            &[0, 1, 2, 255]
        );
        Ok(())
    }

    #[test]
    fn json_round_trip() -> serde_json::Result<()> {
        let json = r#"{"name":"ObjectStream","version":3,"Objects":[]}"#;
        let json_object_stream: JSONObjectStream = serde_json::from_str(json)?;
        assert_eq!(serde_json::to_string(&json_object_stream)?, json);
        Ok(())
    }

    #[test]
    fn from_bytes_accepts_json_objectstream() -> Result<(), ObjectStreamError> {
        let json = br#"{"name":"ObjectStream","version":3,"Objects":[{"field":"test","typeId":"{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}","typeName":"int","value":"2"}]}"#;
        let stream = ObjectStream::from_bytes(json, None)?;

        assert_eq!(stream.version(), 3);
        assert_eq!(stream.elements().len(), 1);
        assert_eq!(stream.elements()[0].name().as_str(), "int");
        assert_eq!(
            stream.elements()[0].field().map(arcstr::ArcStr::as_str),
            Some("test")
        );
        Ok(())
    }

    #[test]
    fn stream_tag_classification() {
        assert!(StreamTag::BINARY.is_binary());
        assert!(StreamTag::XML.is_xml());
        assert!(StreamTag::JSON.is_json());
        assert_eq!(StreamTag::from_byte(b'<'), Some(StreamTag::XML));
        assert_eq!(StreamTag::from_byte(b'{'), Some(StreamTag::JSON));
        assert_eq!(StreamTag::from_byte(0), Some(StreamTag::BINARY));
        assert_eq!(StreamTag::from_byte(b'?'), None);
    }

    fn builtin_context(type_id: Uuid, current_version: u32) -> context::ObjectStreamReadContext {
        let mut context = context::ObjectStreamReadContext::default();
        let class = context
            .insert_class(
                type_id,
                context::ReflectedClass::new(type_id).with_version(current_version),
            )
            .unwrap();
        context
            .insert_builtin_codec(class, type_id, current_version)
            .unwrap();
        context
    }

    #[test]
    fn element_flag_predicates() {
        let e = Element {
            flags: ST_BINARYFLAG_HAS_NAME | ST_BINARYFLAG_HAS_VALUE | 0x04,
            ..Default::default()
        };
        assert!(e.has_name());
        assert!(e.has_value());
        assert!(!e.has_version());
        assert!(!e.has_extra_size_field());
        assert_eq!(e.value_width(), 4);
    }

    #[test]
    fn recursive_iter_pre_order() {
        let stream = ObjectStream {
            tag: StreamTag::BINARY,
            version: 3,
            elements: vec![
                Element {
                    name: ArcStr::from("A"),
                    elements: vec![
                        Element {
                            name: ArcStr::from("B"),
                            ..Default::default()
                        },
                        Element {
                            name: ArcStr::from("C"),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                Element {
                    name: ArcStr::from("D"),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let names: Vec<&str> = stream.iter_recursive().map(|e| e.name().as_str()).collect();
        assert_eq!(names, vec!["A", "B", "C", "D"]);
    }

    #[test]
    fn element_recursive_iter_pre_order() {
        let element = Element {
            name: ArcStr::from("A"),
            elements: vec![
                Element {
                    name: ArcStr::from("B"),
                    elements: vec![Element {
                        name: ArcStr::from("C"),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Element {
                    name: ArcStr::from("D"),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let names: Vec<&str> = element
            .iter_recursive()
            .map(|e| e.name().as_str())
            .collect();
        assert_eq!(names, vec!["A", "B", "C", "D"]);
    }

    #[test]
    fn binary_round_trip_empty() -> Result<(), ObjectStreamError> {
        let stream = ObjectStream::new(3);
        let mut buf = Vec::new();
        stream.write_to(&mut buf)?;
        let parsed = ObjectStream::from_bytes(&buf, None)?;
        assert_eq!(parsed.version, 3);
        assert!(parsed.elements.is_empty());
        Ok(())
    }

    #[test]
    fn binary_rejects_unsupported_stream_version() {
        // A .cgfheap sidecar may begin with little-endian
        // index data: 00 00 01 00 02. If treated as ObjectStream, that
        // misreads as tag 0 and version 0x00010002, then the next zero byte
        // looks like an empty element list.
        let bytes = [0x00, 0x00, 0x01, 0x00, 0x02, 0x00];
        let error = ObjectStream::from_bytes(&bytes, None).unwrap_err();

        assert!(matches!(
            error,
            ObjectStreamError::UnsupportedVersion(65538)
        ));
    }

    #[test]
    fn xml_rejects_unsupported_stream_version() {
        let bytes = br#"<ObjectStream version="74"></ObjectStream>"#;
        let error = ObjectStream::from_bytes(bytes, None).unwrap_err();

        assert!(matches!(error, ObjectStreamError::UnsupportedVersion(74)));
    }

    #[test]
    fn json_accepts_historical_stream_version_zero() {
        let bytes = br#"{"name":"ObjectStream","version":0,"Objects":[]}"#;
        let stream = ObjectStream::from_bytes(bytes, None).unwrap();

        assert_eq!(stream.version(), 0);
    }

    #[test]
    fn binary_version_zero_uses_the_pre_specialization_header() {
        let stream = ObjectStream::new(0);
        let bytes = stream.to_bytes().unwrap();
        assert_eq!(bytes, [0, 0, 0, 0, 0, 0]);
        assert_eq!(ObjectStream::from_bytes(&bytes, None).unwrap().version(), 0);
    }

    #[test]
    fn binary_rejects_trailing_bytes_after_root_terminator() {
        let bytes = [
            0x00, 0x00, 0x00, 0x00, 0x03, // binary ObjectStream version 3
            0x00, // root element-list terminator
            0xFF, // stale payload that does not belong to the stream
        ];
        let error = ObjectStream::from_bytes(&bytes, None).unwrap_err();

        assert!(matches!(error, ObjectStreamError::TrailingDataAfterRoot));
    }

    #[test]
    fn binary_rejects_element_flags_without_header_bit() {
        let bytes = [
            0x00,
            0x00,
            0x00,
            0x00,
            0x03,                    // binary ObjectStream version 3
            ST_BINARYFLAG_HAS_VALUE, // value flag without element-header bit
            0x00,
        ];
        let error = ObjectStream::from_bytes(&bytes, None).unwrap_err();

        assert!(matches!(
            error,
            ObjectStreamError::InvalidElementFlags(ST_BINARYFLAG_HAS_VALUE)
        ));
    }
}

// Lumberyard `Code/Framework/AzCore/AzCore/Serialization/SerializeContext.h`
// defines these ClassElement flags.
//
// SerializeContext flags:
//   FLG_POINTER          = (1 << 0)  // Element is stored as pointer (not a value).
//   FLG_BASE_CLASS       = (1 << 1)  // Element is a base class of the holding class.
//   FLG_NO_DEFAULT_VALUE = (1 << 2)  // Class element can't have a default value.
//   FLG_DYNAMIC_FIELD    = (1 << 3)  // Element represents a dynamic field
//                                    // (DynamicSerializableField::m_data).
//   FLG_UI_ELEMENT       = (1 << 4)  // Element represents a UI element tied to the
//                                    // ClassData of its parent.
