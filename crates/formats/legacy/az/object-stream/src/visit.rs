//! Streaming visitor API for `ObjectStream` payloads.
//!
//! [`ObjectStream::from_reader`](crate::ObjectStream::from_reader) materializes the entire
//! element tree into RAM. For large slices/prefabs that's wasteful
//! when the caller only wants to extract a few fields. This module
//! provides a SAX-style visitor: each element header is reported
//! as the parser walks it, with no allocation beyond the per-call
//! data scratch buffer.
//!
//! For the common "find one / find all" case, prefer the higher-
//! level [`crate::query`] helpers — they wrap this module and avoid
//! the boilerplate of a custom visitor struct.
//!
//! ## Typed root schemas
//!
//! When an asset is just “one reflected root object + a fixed set of CRC fields”, prefer
//! [`StreamField`], [`StreamingPartial`], and [`StreamObjectVisitor`] over another bespoke
//! [`ElementVisitor`]. [`present`] and [`once`] cover the two recurring bits of glue:
//! required [`Option`] fields and duplicate-checked assignment into a slot.
//!
//! # Example
//!
//! ```no_run
//! use az_objectstream::ObjectStreamError;
//! use az_objectstream::visit::{ElementHeader, ElementVisitor, VisitFlow, parse_streaming_bytes};
//! use uuid::Uuid;
//!
//! struct CountByType {
//!     counts: std::collections::HashMap<Uuid, usize>,
//! }
//!
//! impl ElementVisitor for CountByType {
//!     type Error = ObjectStreamError;
//!     fn open_element(&mut self, header: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error> {
//!         *self.counts.entry(header.id).or_default() += 1;
//!         Ok(VisitFlow::Continue)
//!     }
//! }
//!
//! let bytes = std::fs::read("some.slice").unwrap();
//! let mut counter = CountByType { counts: Default::default() };
//! let _version = parse_streaming_bytes(&bytes, None, &mut counter).unwrap();
//! ```

use std::io::{self, Cursor, Read};

use arcstr::ArcStr;
use thiserror::Error;
use uuid::Uuid;

use crate::binary::{
    BinaryElement, ensure_reader_exhausted, read_element_header, read_stream_header,
};
use crate::context::{ObjectStreamReadContext, ReflectedClassKey, VersionConversionState};
use crate::lookup::LumberyardHashes;
use crate::value::{DecodeAzValue, ObjectStreamValueError};
use crate::{ObjectStreamError, PayloadEncoding, TypeResolution, TypeResolutionState};

/// Header information for one element.
///
/// The `data` slice is borrowed from a per-call scratch buffer and is
/// invalidated after `open_element` returns; if you need to keep it, copy it.
/// The [`ArcStr`] handles for `name` / `field` are owned by the
/// [`LumberyardHashes`] table and can be cheaply cloned to outlive
/// this header.
#[derive(Debug)]
pub struct ElementHeader<'a> {
    pub flags: u8,
    pub name_crc: Option<u32>,
    pub version: Option<u32>,
    /// Unmodified UUID from the element header, matching `Element::id`.
    pub id: Uuid,
    pub specialization: Option<Uuid>,
    pub(crate) resolution: TypeResolution,
    pub version_state: crate::context::VersionConversionState,
    /// Resolved type name from the [`LumberyardHashes`] dump (if
    /// supplied to the parser); `None` if unknown.
    pub name: Option<&'a ArcStr>,
    /// Resolved field name from the [`LumberyardHashes`] dump.
    pub field: Option<&'a ArcStr>,
    /// Inline payload bytes. `Some(&[])` is an explicitly present empty value;
    /// `None` means the value flag was absent.
    pub data: Option<&'a [u8]>,
    pub payload_encoding: PayloadEncoding,
}

impl<'a> ElementHeader<'a> {
    #[must_use]
    pub const fn raw_type_id(&self) -> &Uuid {
        &self.id
    }

    #[must_use]
    pub const fn resolved_type_id(&self) -> Option<&Uuid> {
        match &self.resolution {
            TypeResolution::Resolved { type_id, .. } => Some(type_id),
            TypeResolution::Raw | TypeResolution::Unresolved => None,
        }
    }

    #[must_use]
    pub const fn resolved_class(&self) -> Option<ReflectedClassKey> {
        match self.resolution {
            TypeResolution::Resolved { class, .. } => Some(class),
            TypeResolution::Raw | TypeResolution::Unresolved => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn reflected_enum_type_id(&self) -> Option<&Uuid> {
        match &self.resolution {
            TypeResolution::Resolved { enum_type_id, .. } => enum_type_id.as_ref(),
            TypeResolution::Raw | TypeResolution::Unresolved => None,
        }
    }

    #[must_use]
    pub const fn builtin_serializer(&self) -> Option<crate::codec::BuiltinSerializerDescriptor> {
        match self.resolution {
            TypeResolution::Resolved {
                builtin_serializer, ..
            } => builtin_serializer,
            TypeResolution::Raw | TypeResolution::Unresolved => None,
        }
    }

    #[must_use]
    pub const fn container_shape(&self) -> Option<crate::context::ContainerShape> {
        match self.resolution {
            TypeResolution::Resolved {
                container_shape, ..
            } => container_shape,
            TypeResolution::Raw | TypeResolution::Unresolved => None,
        }
    }

    #[must_use]
    pub const fn type_resolution(&self) -> TypeResolutionState {
        match self.resolution {
            TypeResolution::Raw => TypeResolutionState::Raw,
            TypeResolution::Unresolved => TypeResolutionState::Unresolved,
            TypeResolution::Resolved { .. } => TypeResolutionState::Resolved,
        }
    }

    /// Semantic type identity for typed reads.
    ///
    /// A resolved header yields its `ClassData` type; a context-free walk has no
    /// semantic identity beyond the wire UUID and yields that instead.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamValueError::UnresolvedType`] when a read context was
    /// supplied but could not resolve this element — the only state in which the
    /// wire UUID is not an acceptable answer.
    pub fn semantic_type_id(&self) -> Result<Uuid, crate::value::ObjectStreamValueError> {
        match self.resolution {
            TypeResolution::Resolved { type_id, .. } => Ok(type_id),
            // A context-free streaming walk has no semantic identity beyond
            // the wire UUID. This is the same raw-inspection contract exposed
            // by `value::semantic_type_id` for materialized elements.
            TypeResolution::Raw => Ok(self.id),
            TypeResolution::Unresolved => {
                Err(crate::value::ObjectStreamValueError::UnresolvedType {
                    field: self
                        .field
                        .map_or_else(|| "<unnamed>".to_string(), ToString::to_string),
                    raw_id: self.id,
                    specialization: self.specialization,
                })
            }
        }
    }

    /// Decode this header's leaf payload as an AZ reflected value.
    ///
    /// # Errors
    ///
    /// Returns whatever `T`'s [`DecodeAzValue`] implementation returns for this
    /// header — typically [`ObjectStreamValueError::UnexpectedType`] when the
    /// element is not the AZ type `T` decodes,
    /// [`ObjectStreamValueError::MissingData`] when it carries no payload,
    /// [`ObjectStreamValueError::InvalidLength`] when the payload is the wrong
    /// width, and [`ObjectStreamValueError::UnresolvedType`] via
    /// [`Self::semantic_type_id`].
    pub fn decode<T>(&'a self) -> Result<T, crate::value::ObjectStreamValueError>
    where
        T: crate::value::DecodeAzValue<'a>,
    {
        T::decode_az_value(self)
    }

    /// Read this header as a typed field value, validating the reflected type first.
    ///
    /// # Errors
    ///
    /// Returns [`ObjectStreamValueError::UnexpectedType`] carrying `expected_name`
    /// when this header's semantic type is not `expected`, plus every error
    /// [`Self::semantic_type_id`] and [`Self::decode`] return.
    pub fn value_as<T>(
        &'a self,
        expected: Uuid,
        expected_name: &'static str,
    ) -> Result<T, crate::value::ObjectStreamValueError>
    where
        T: crate::value::DecodeAzValue<'a>,
    {
        let actual = self.semantic_type_id()?;
        if actual != expected {
            return Err(crate::value::ObjectStreamValueError::UnexpectedType {
                field: self
                    .field
                    .map_or_else(|| "<unnamed>".to_string(), ToString::to_string),
                expected: expected_name,
                actual,
            });
        }
        self.decode()
    }
}

/// Whether the visitor wants to descend into an element's children
/// — and whether it wants to keep walking the rest of the tree at
/// all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisitFlow {
    /// Recurse into children, then continue with siblings.
    Continue,
    /// Skip children — the parser advances past them silently —
    /// then continue with siblings.
    Skip,
    /// Stop the walk entirely. No further `open_element` /
    /// `close_element` callbacks will fire.
    Stop,
}

/// Streaming visitor invoked by [`parse_streaming`] /
/// [`parse_streaming_bytes`].
pub trait ElementVisitor {
    type Error: From<ObjectStreamError>;

    /// Called when an element header is read.
    ///
    /// # Errors
    ///
    /// Implementation-defined: any `Self::Error` the visitor wants to abort the
    /// walk with. Returning an error stops the parse immediately and propagates
    /// out of [`parse_streaming`]; to stop without failing, return
    /// [`VisitFlow::Stop`] instead.
    fn open_element(&mut self, header: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error>;

    /// Called after all of an element's children have been visited
    /// (or skipped). Default is a no-op. Not called for elements
    /// where `open_element` returned [`VisitFlow::Stop`].
    ///
    /// # Errors
    ///
    /// Implementation-defined: any `Self::Error` the visitor wants to abort the
    /// walk with. The default implementation never fails.
    fn close_element(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum StreamingObjectError {
    #[error("objectstream parse error")]
    ObjectStream(#[from] ObjectStreamError),
    #[error("objectstream value error")]
    Value(#[from] ObjectStreamValueError),
    #[error("{owner} is missing {field}")]
    MissingField {
        owner: &'static str,
        field: &'static str,
    },
    #[error("missing {owner} root")]
    MissingRoot { owner: &'static str },
    #[error("multiple {owner} roots")]
    MultipleRoots { owner: &'static str },
    #[error("{owner} root has unexpected type {actual}")]
    UnexpectedRootType { owner: &'static str, actual: Uuid },
    #[error("{owner} has unexpected field name CRC {name_crc:?} with type {actual}")]
    UnexpectedField {
        owner: &'static str,
        actual: Uuid,
        name_crc: Option<u32>,
    },
    #[error("{owner} has unexpected nested type {actual} for name CRC {name_crc:?}")]
    UnexpectedNestedType {
        owner: &'static str,
        actual: Uuid,
        name_crc: Option<u32>,
    },
    #[error("{owner} has duplicate {field}")]
    DuplicateField {
        owner: &'static str,
        field: &'static str,
    },
}

/// Accumulator filled during a streaming walk, then [`finalize`](StreamingPartial::finalize)d
/// into the public Rust type (same role as serde’s internal “partial state” before `deserialize`
/// returns).
pub trait StreamingPartial: Sized + 'static {
    type Output;

    /// # Errors
    ///
    /// Implementation-defined, but conventionally
    /// [`StreamingObjectError::MissingField`] for a field the walk never filled
    /// in — that is the check `finalize` exists to perform.
    fn finalize(self) -> Result<Self::Output, StreamingObjectError>;
}

#[derive(Clone, Copy)]
pub struct StreamField<T> {
    pub name_crc: u32,
    pub field: &'static str,
    pub read: for<'a> fn(&mut T, &'a ElementHeader<'a>) -> Result<(), StreamingObjectError>,
}

impl<T> StreamField<T> {
    #[inline]
    #[must_use]
    pub const fn new(
        name_crc: u32,
        field: &'static str,
        read: for<'a> fn(&mut T, &'a ElementHeader<'a>) -> Result<(), StreamingObjectError>,
    ) -> Self {
        Self {
            name_crc,
            field,
            read,
        }
    }
}

pub struct StreamObjectVisitor<T: 'static> {
    owner: &'static str,
    root_type: Uuid,
    fields: &'static [StreamField<T>],
    value: T,
    roots: usize,
    depth: usize,
}

impl<T: 'static> StreamObjectVisitor<T> {
    #[inline]
    #[must_use]
    pub const fn new(
        owner: &'static str,
        root_type: Uuid,
        fields: &'static [StreamField<T>],
        value: T,
    ) -> Self {
        Self {
            owner,
            root_type,
            fields,
            value,
            roots: 0,
            depth: 0,
        }
    }

    /// Take the accumulated value, requiring at least one matching root.
    ///
    /// # Errors
    ///
    /// Returns [`StreamingObjectError::MissingRoot`] if the walk never opened an
    /// element of this visitor's root type.
    pub fn into_value(self) -> Result<T, StreamingObjectError> {
        if self.roots == 0 {
            return Err(StreamingObjectError::MissingRoot { owner: self.owner });
        }
        Ok(self.value)
    }
}

pub struct StreamNestedObjectVisitor<T: 'static, C: StreamingPartial> {
    owner: &'static str,
    root_type: Uuid,
    child_owner: &'static str,
    child_field_crc: u32,
    child_field: &'static str,
    child_type: Uuid,
    child_fields: &'static [StreamField<C>],
    value: T,
    child: Option<C>,
    new_child: fn() -> C,
    finish_child: fn(&mut T, C::Output) -> Result<(), StreamingObjectError>,
    roots: usize,
    depth: usize,
}

pub struct NestedObjectSchema<T: 'static, C: StreamingPartial> {
    pub owner: &'static str,
    pub root_type: Uuid,
    pub child_owner: &'static str,
    pub child_field_crc: u32,
    pub child_field: &'static str,
    pub child_type: Uuid,
    pub child_fields: &'static [StreamField<C>],
    pub new_child: fn() -> C,
    pub finish_child: fn(&mut T, C::Output) -> Result<(), StreamingObjectError>,
}

impl<T: 'static, C: StreamingPartial> StreamNestedObjectVisitor<T, C> {
    #[inline]
    #[must_use]
    pub const fn new(schema: &NestedObjectSchema<T, C>, value: T) -> Self {
        Self {
            owner: schema.owner,
            root_type: schema.root_type,
            child_owner: schema.child_owner,
            child_field_crc: schema.child_field_crc,
            child_field: schema.child_field,
            child_type: schema.child_type,
            child_fields: schema.child_fields,
            value,
            child: None,
            new_child: schema.new_child,
            finish_child: schema.finish_child,
            roots: 0,
            depth: 0,
        }
    }

    /// Take the accumulated value, requiring at least one matching root.
    ///
    /// # Errors
    ///
    /// Returns [`StreamingObjectError::MissingRoot`] if the walk never opened an
    /// element of this visitor's root type.
    pub fn into_value(self) -> Result<T, StreamingObjectError> {
        if self.roots == 0 {
            return Err(StreamingObjectError::MissingRoot { owner: self.owner });
        }
        Ok(self.value)
    }
}

impl<T: StreamingPartial> StreamObjectVisitor<T> {
    /// Take the accumulated value and finalize it into the public type.
    ///
    /// # Errors
    ///
    /// Returns [`StreamingObjectError::MissingRoot`] via [`Self::into_value`], then
    /// any error `T::finalize` returns — conventionally
    /// [`StreamingObjectError::MissingField`] for a field the walk never filled.
    pub fn into_output(self) -> Result<T::Output, StreamingObjectError> {
        self.into_value()?.finalize()
    }
}

impl<T: StreamingPartial, C: StreamingPartial> StreamNestedObjectVisitor<T, C> {
    /// Take the accumulated value and finalize it into the public type.
    ///
    /// # Errors
    ///
    /// Returns [`StreamingObjectError::MissingRoot`] via [`Self::into_value`], then
    /// any error `T::finalize` returns — conventionally
    /// [`StreamingObjectError::MissingField`] for a field the walk never filled.
    pub fn into_output(self) -> Result<T::Output, StreamingObjectError> {
        self.into_value()?.finalize()
    }
}

impl<T: 'static> ElementVisitor for StreamObjectVisitor<T> {
    type Error = StreamingObjectError;

    fn open_element(&mut self, header: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error> {
        let actual = header.semantic_type_id()?;
        match self.depth {
            0 => {
                self.roots += 1;
                if self.roots > 1 {
                    return Err(StreamingObjectError::MultipleRoots { owner: self.owner });
                }
                if actual != self.root_type {
                    return Err(StreamingObjectError::UnexpectedRootType {
                        owner: self.owner,
                        actual,
                    });
                }
            }
            1 => {
                let Some(field_crc) = header.name_crc else {
                    return Err(StreamingObjectError::UnexpectedField {
                        owner: self.owner,
                        actual,
                        name_crc: header.name_crc,
                    });
                };
                let Some(field) = self.fields.iter().find(|field| field.name_crc == field_crc)
                else {
                    return Err(StreamingObjectError::UnexpectedField {
                        owner: self.owner,
                        actual,
                        name_crc: header.name_crc,
                    });
                };
                (field.read)(&mut self.value, header)?;
            }
            _ => {
                return Err(StreamingObjectError::UnexpectedNestedType {
                    owner: self.owner,
                    actual,
                    name_crc: header.name_crc,
                });
            }
        }

        self.depth += 1;
        Ok(VisitFlow::Continue)
    }

    fn close_element(&mut self) -> Result<(), Self::Error> {
        self.depth = self.depth.saturating_sub(1);
        Ok(())
    }
}

impl<T: 'static, C: StreamingPartial> ElementVisitor for StreamNestedObjectVisitor<T, C> {
    type Error = StreamingObjectError;

    fn open_element(&mut self, header: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error> {
        let actual = header.semantic_type_id()?;
        match self.depth {
            0 => {
                self.roots += 1;
                if self.roots > 1 {
                    return Err(StreamingObjectError::MultipleRoots { owner: self.owner });
                }
                if actual != self.root_type {
                    return Err(StreamingObjectError::UnexpectedRootType {
                        owner: self.owner,
                        actual,
                    });
                }
            }
            1 => {
                if header.name_crc != Some(self.child_field_crc) || actual != self.child_type {
                    return Err(StreamingObjectError::UnexpectedField {
                        owner: self.owner,
                        actual,
                        name_crc: header.name_crc,
                    });
                }
                if self.child.is_some() {
                    return Err(StreamingObjectError::DuplicateField {
                        owner: self.owner,
                        field: self.child_field,
                    });
                }
                self.child = Some((self.new_child)());
            }
            2 => {
                let Some(child) = self.child.as_mut() else {
                    return Err(StreamingObjectError::UnexpectedNestedType {
                        owner: self.child_owner,
                        actual,
                        name_crc: header.name_crc,
                    });
                };
                let Some(field_crc) = header.name_crc else {
                    return Err(StreamingObjectError::UnexpectedField {
                        owner: self.child_owner,
                        actual,
                        name_crc: header.name_crc,
                    });
                };
                let Some(field) = self
                    .child_fields
                    .iter()
                    .find(|field| field.name_crc == field_crc)
                else {
                    return Err(StreamingObjectError::UnexpectedField {
                        owner: self.child_owner,
                        actual,
                        name_crc: header.name_crc,
                    });
                };
                (field.read)(child, header)?;
            }
            _ => {
                return Err(StreamingObjectError::UnexpectedNestedType {
                    owner: self.child_owner,
                    actual,
                    name_crc: header.name_crc,
                });
            }
        }

        self.depth += 1;
        Ok(VisitFlow::Continue)
    }

    fn close_element(&mut self) -> Result<(), Self::Error> {
        let old_depth = self.depth;
        self.depth = self.depth.saturating_sub(1);
        if old_depth == 2 {
            let Some(child) = self.child.take() else {
                return Err(StreamingObjectError::MissingRoot {
                    owner: self.child_owner,
                });
            };
            (self.finish_child)(&mut self.value, child.finalize()?)?;
        }
        Ok(())
    }
}

/// Read a typed field value off a header, validating the reflected type first.
///
/// # Errors
///
/// Returns [`StreamingObjectError::Value`] wrapping any error
/// [`ElementHeader::value_as`] returns — a type mismatch against `expected`, an
/// unresolved element, a missing or wrong-width payload.
pub fn stream_value<'a, T>(
    header: &'a ElementHeader<'a>,
    expected: Uuid,
    expected_name: &'static str,
) -> Result<T, StreamingObjectError>
where
    T: DecodeAzValue<'a>,
{
    header.value_as(expected, expected_name).map_err(Into::into)
}

/// Assign `value` into `slot`, returning [`StreamingObjectError::DuplicateField`] if it was
/// already set.
///
/// # Errors
///
/// Returns [`StreamingObjectError::DuplicateField`] naming `owner` and `field`
/// when `slot` already held a value.
pub fn once<T>(
    slot: &mut Option<T>,
    value: T,
    owner: &'static str,
    field: &'static str,
) -> Result<(), StreamingObjectError> {
    if slot.replace(value).is_some() {
        return Err(StreamingObjectError::DuplicateField { owner, field });
    }
    Ok(())
}

/// [`Some`] value, or [`StreamingObjectError::MissingField`].
///
/// # Errors
///
/// Returns [`StreamingObjectError::MissingField`] naming `owner` and `field`
/// when `value` is [`None`].
pub fn present<T>(
    value: Option<T>,
    owner: &'static str,
    field: &'static str,
) -> Result<T, StreamingObjectError> {
    value.ok_or(StreamingObjectError::MissingField { owner, field })
}

/// Parse an `ObjectStream` in streaming mode, calling `visitor` for
/// each element. Returns the stream's `version` field on success.
///
/// # Errors
///
/// Returns `V::Error` converted from [`ObjectStreamError::Io`] if `reader`
/// fails or ends mid-element,
/// [`ObjectStreamError::InvalidStreamTag`] if the first byte is not the binary
/// tag, [`ObjectStreamError::UnsupportedVersion`] for a version above 3,
/// [`ObjectStreamError::InvalidElementFlags`] for an illegal header flag
/// combination, [`ObjectStreamError::UnsupportedSizeWidth`] for an extra-size
/// field that is not 1, 2 or 4 bytes wide, [`ObjectStreamError::Uuid`] for a
/// malformed type UUID, and [`ObjectStreamError::TrailingDataAfterRoot`] if
/// bytes remain after the root terminator. Any error the visitor's
/// `open_element` or `close_element` returns aborts the walk and propagates
/// unchanged.
pub fn parse_streaming<R: Read, V: ElementVisitor>(
    reader: &mut R,
    hashes: Option<&LumberyardHashes>,
    visitor: &mut V,
) -> Result<u32, V::Error> {
    parse_streaming_impl(reader, hashes, visitor)
}

/// Parse binary `ObjectStream` data with parent-aware reflection metadata.
///
/// # Errors
///
/// Returns [`ObjectStreamError::IncompleteReadContext`] before reading anything
/// if `context` still has captured serializers with no registered
/// implementation. This entrypoint materializes the fully converted DOM before
/// visiting it, so it also returns every reflection failure
/// [`crate::ObjectStream::from_bytes_with_context`] can raise, on top of the
/// parse errors listed on [`parse_streaming`].
pub fn parse_streaming_with_context<R: Read, V: ElementVisitor>(
    reader: &mut R,
    context: &ObjectStreamReadContext,
    visitor: &mut V,
) -> Result<u32, V::Error> {
    context
        .validate_complete()
        .map_err(ObjectStreamError::from)?;
    // Lumberyard preparses a complete DataElementNode subtree whenever a
    // version converter, data converter, or DataOverlayInfo is reached.  A
    // callback-only cursor cannot execute those semantics safely.  Preserve
    // the streaming visitor API while using the same fully converted DOM as
    // the typed reader; the raw no-context entrypoint remains allocation-light.
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(io_err)?;
    let stream =
        crate::ObjectStream::from_bytes_with_context(&bytes, context).map_err(V::Error::from)?;
    visit_resolved_dom(&stream, visitor)?;
    Ok(stream.version())
}

fn visit_resolved_dom<V: ElementVisitor>(
    stream: &crate::ObjectStream,
    visitor: &mut V,
) -> Result<(), V::Error> {
    for element in stream.elements() {
        if visit_resolved_element(element, visitor)? {
            break;
        }
    }
    Ok(())
}

fn visit_resolved_element<V: ElementVisitor>(
    element: &crate::Element,
    visitor: &mut V,
) -> Result<bool, V::Error> {
    let header = ElementHeader {
        flags: element.flags,
        name_crc: element.name_crc,
        version: element.version,
        id: *element.raw_type_id(),
        specialization: element.specialization,
        resolution: element.resolution,
        version_state: element.version_state,
        name: Some(element.name()),
        field: element.field(),
        data: element.data(),
        payload_encoding: element.payload_encoding(),
    };
    match visitor.open_element(&header)? {
        VisitFlow::Stop => Ok(true),
        VisitFlow::Skip => {
            visitor.close_element()?;
            Ok(false)
        }
        VisitFlow::Continue => {
            for child in element.children() {
                if visit_resolved_element(child, visitor)? {
                    return Ok(true);
                }
            }
            visitor.close_element()?;
            Ok(false)
        }
    }
}

fn parse_streaming_impl<R: Read, V: ElementVisitor>(
    reader: &mut R,
    hashes: Option<&LumberyardHashes>,
    visitor: &mut V,
) -> Result<u32, V::Error> {
    let version = read_stream_header(reader)?;
    let mut data_buf = Vec::new();
    let mut stack: Vec<WalkFrame> = Vec::new();
    let mut hidden_depth = 0usize;

    loop {
        match read_element_header(reader, version, hashes)? {
            BinaryElement::Header(header) => {
                data_buf.clear();
                let data_size = header.data_size.unwrap_or(0);
                data_buf.resize(data_size, 0);
                if data_size > 0 {
                    reader.read_exact(&mut data_buf).map_err(io_err)?;
                }

                let semantic_name = hashes.and_then(|hashes| hashes.type_name(&header.id));

                if hidden_depth > 0 {
                    stack.push(WalkFrame {
                        close_user_element: false,
                        hides_descendants: true,
                    });
                    hidden_depth += 1;
                    continue;
                }

                let flow = {
                    let header = ElementHeader {
                        flags: header.flags,
                        name_crc: header.name_crc,
                        version: header.version,
                        id: header.id,
                        specialization: header.specialization,
                        resolution: TypeResolution::Raw,
                        version_state: VersionConversionState::default(),
                        name: semantic_name.or(header.name),
                        field: header.field,
                        data: header.data_size.map(|_| data_buf.as_slice()),
                        payload_encoding: PayloadEncoding::BinaryBigEndian,
                    };
                    visitor.open_element(&header)?
                };

                match flow {
                    VisitFlow::Continue => stack.push(WalkFrame {
                        close_user_element: true,
                        hides_descendants: false,
                    }),
                    VisitFlow::Skip => {
                        stack.push(WalkFrame {
                            close_user_element: true,
                            hides_descendants: true,
                        });
                        hidden_depth += 1;
                    }
                    VisitFlow::Stop => return Ok(version),
                }
            }
            BinaryElement::EndOfList => {
                let Some(frame) = stack.pop() else {
                    break;
                };
                if frame.hides_descendants {
                    hidden_depth -= 1;
                }
                if frame.close_user_element {
                    visitor.close_element()?;
                }
            }
        }
    }
    ensure_reader_exhausted(reader)?;
    Ok(version)
}

/// Convenience: streaming parse from a `&[u8]`.
///
/// # Errors
///
/// Returns any error [`parse_streaming`] returns; the in-memory cursor cannot
/// itself fail.
#[inline]
pub fn parse_streaming_bytes<V: ElementVisitor>(
    bytes: &[u8],
    hashes: Option<&LumberyardHashes>,
    visitor: &mut V,
) -> Result<u32, V::Error> {
    let mut cursor = Cursor::new(bytes);
    parse_streaming(&mut cursor, hashes, visitor)
}

/// Convenience: context-aware streaming parse from a `&[u8]`.
///
/// # Errors
///
/// Returns any error [`parse_streaming_with_context`] returns; the in-memory
/// cursor cannot itself fail.
#[inline]
pub fn parse_streaming_bytes_with_context<V: ElementVisitor>(
    bytes: &[u8],
    context: &ObjectStreamReadContext,
    visitor: &mut V,
) -> Result<u32, V::Error> {
    let mut cursor = Cursor::new(bytes);
    parse_streaming_with_context(&mut cursor, context, visitor)
}

#[derive(Debug, Clone, Copy)]
struct WalkFrame {
    close_user_element: bool,
    hides_descendants: bool,
}

#[inline]
const fn io_err(e: io::Error) -> ObjectStreamError {
    ObjectStreamError::Io(e)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::context::{ObjectStreamVersionConverter, ReflectedClass};
    use crate::{Element, ObjectStream, StreamTag};

    struct StopAtSecond {
        seen: usize,
    }
    impl ElementVisitor for StopAtSecond {
        type Error = ObjectStreamError;
        fn open_element(&mut self, _h: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error> {
            self.seen += 1;
            if self.seen == 2 {
                Ok(VisitFlow::Stop)
            } else {
                Ok(VisitFlow::Continue)
            }
        }
    }

    #[derive(Default)]
    struct Payloads(Vec<Vec<u8>>);
    impl ElementVisitor for Payloads {
        type Error = ObjectStreamError;
        fn open_element(&mut self, h: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error> {
            self.0.push(h.data.unwrap_or_default().to_vec());
            Ok(VisitFlow::Continue)
        }
    }

    #[derive(Default)]
    struct Presence(Vec<Option<usize>>);
    impl ElementVisitor for Presence {
        type Error = ObjectStreamError;

        fn open_element(&mut self, header: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error> {
            self.0.push(header.data.map(<[u8]>::len));
            Ok(VisitFlow::Continue)
        }
    }

    #[derive(Default)]
    struct TypedValue(Option<u64>);
    impl ElementVisitor for TypedValue {
        type Error = ObjectStreamError;

        fn open_element(&mut self, header: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error> {
            assert_eq!(header.type_resolution(), TypeResolutionState::Raw);
            assert_eq!(header.semantic_type_id().unwrap(), crate::types::AZ_U64);
            self.0 = Some(header.value_as(crate::types::AZ_U64, "AZ::u64").unwrap());
            Ok(VisitFlow::Continue)
        }
    }

    #[derive(Default)]
    struct AssetHint(Option<String>);
    impl ElementVisitor for AssetHint {
        type Error = ObjectStreamError;

        fn open_element(&mut self, header: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error> {
            self.0 = Some(
                crate::asset_reference::read_asset_value(header)
                    .map_err(|error| {
                        ObjectStreamError::Io(io::Error::new(io::ErrorKind::InvalidData, error))
                    })?
                    .hint()
                    .to_owned(),
            );
            Ok(VisitFlow::Continue)
        }
    }

    #[derive(Default)]
    struct Identity {
        raw: Option<Uuid>,
        semantic: Option<Uuid>,
        resolution: Option<TypeResolutionState>,
    }
    impl ElementVisitor for Identity {
        type Error = ObjectStreamError;

        fn open_element(&mut self, header: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error> {
            self.raw = Some(header.id);
            self.semantic = header.resolved_type_id().copied();
            self.resolution = Some(header.type_resolution());
            Ok(VisitFlow::Continue)
        }
    }

    #[derive(Default)]
    struct CountVisitor {
        opens: usize,
        closes: usize,
    }

    impl ElementVisitor for CountVisitor {
        type Error = ObjectStreamError;
        fn open_element(&mut self, _h: &ElementHeader<'_>) -> Result<VisitFlow, Self::Error> {
            self.opens += 1;
            Ok(VisitFlow::Continue)
        }
        fn close_element(&mut self) -> Result<(), Self::Error> {
            self.closes += 1;
            Ok(())
        }
    }

    #[derive(Debug)]
    struct NoopConverter;

    impl ObjectStreamVersionConverter for NoopConverter {
        fn convert(
            &self,
            _element: &mut Element,
            _from: u32,
            _to: u32,
        ) -> Result<(), ObjectStreamError> {
            Ok(())
        }
    }

    #[test]
    fn context_streaming_executes_registered_dom_version_converter() {
        let id = Uuid::from_u128(70);
        let mut element = Element::new(id);
        element.version = Some(0);
        let bytes = ObjectStream {
            version: 3,
            elements: vec![element],
            ..ObjectStream::default()
        }
        .to_bytes()
        .unwrap();
        let mut context = ObjectStreamReadContext::default();
        let class = context
            .insert_class(
                id,
                ReflectedClass::new(id)
                    .with_version(1)
                    .with_captured_version_converter(),
            )
            .unwrap();
        context
            .insert_version_converter(class, Arc::new(NoopConverter))
            .unwrap();

        let mut visitor = CountVisitor::default();
        let version = parse_streaming_bytes_with_context(&bytes, &context, &mut visitor).unwrap();

        assert_eq!(version, 3);
        assert_eq!(visitor.opens, 1);
        assert_eq!(visitor.closes, 1);
    }

    #[test]
    fn empty_stream_visits_nothing() -> Result<(), ObjectStreamError> {
        let stream = ObjectStream {
            tag: StreamTag::BINARY,
            version: 3,
            elements: Vec::new(),
            ..ObjectStream::default()
        };
        let mut buf = Vec::new();
        stream.write_to(&mut buf)?;

        let mut visitor = CountVisitor::default();
        let v = parse_streaming_bytes(&buf, None, &mut visitor)?;
        assert_eq!(v, 3);
        assert_eq!(visitor.opens, 0);
        assert_eq!(visitor.closes, 0);
        Ok(())
    }

    #[test]
    fn stop_aborts_walk_early() -> Result<(), ObjectStreamError> {
        let stream = ObjectStream {
            tag: StreamTag::BINARY,
            version: 3,
            elements: vec![
                Element {
                    flags: crate::ST_BINARYFLAG_ELEMENT_HEADER,
                    id: Uuid::from_u128(1),
                    ..Default::default()
                },
                Element {
                    flags: crate::ST_BINARYFLAG_ELEMENT_HEADER,
                    id: Uuid::from_u128(2),
                    ..Default::default()
                },
                Element {
                    flags: crate::ST_BINARYFLAG_ELEMENT_HEADER,
                    id: Uuid::from_u128(3),
                    ..Default::default()
                },
            ],
            ..ObjectStream::default()
        };
        let mut buf = Vec::new();
        stream.write_to(&mut buf)?;

        let mut v = StopAtSecond { seen: 0 };
        parse_streaming_bytes(&buf, None, &mut v)?;
        assert_eq!(v.seen, 2, "should stop at second element");
        Ok(())
    }

    #[test]
    fn streaming_reuses_scratch_without_losing_payloads() -> Result<(), ObjectStreamError> {
        let stream = ObjectStream {
            tag: StreamTag::BINARY,
            version: 3,
            elements: vec![
                Element {
                    flags: crate::ST_BINARYFLAG_ELEMENT_HEADER | crate::ST_BINARYFLAG_HAS_VALUE | 3,
                    id: Uuid::from_u128(1),
                    data: Some(vec![1, 2, 3]),
                    ..Default::default()
                },
                Element {
                    flags: crate::ST_BINARYFLAG_ELEMENT_HEADER | crate::ST_BINARYFLAG_HAS_VALUE | 1,
                    id: Uuid::from_u128(2),
                    data: Some(vec![4]),
                    ..Default::default()
                },
            ],
            ..ObjectStream::default()
        };
        let mut buf = Vec::new();
        stream.write_to(&mut buf)?;

        let mut payloads = Payloads::default();
        parse_streaming_bytes(&buf, None, &mut payloads)?;
        assert_eq!(payloads.0, vec![vec![1, 2, 3], vec![4]]);
        Ok(())
    }

    #[test]
    fn streaming_preserves_empty_value_presence_like_dom() -> Result<(), ObjectStreamError> {
        let stream = ObjectStream {
            version: 3,
            elements: vec![
                Element::new(Uuid::from_u128(10)).with_data([]),
                Element::new(Uuid::from_u128(11)),
            ],
            ..ObjectStream::default()
        };
        let bytes = stream.to_bytes()?;
        let dom = ObjectStream::from_bytes(&bytes, None)?;

        let mut streaming = Presence::default();
        parse_streaming_bytes(&bytes, None, &mut streaming)?;
        assert_eq!(streaming.0, vec![Some(0), None]);
        assert_eq!(
            dom.elements()
                .iter()
                .map(|element| element.data().map(<[u8]>::len))
                .collect::<Vec<_>>(),
            streaming.0
        );
        Ok(())
    }

    #[test]
    fn context_free_streaming_uses_wire_uuid_as_semantic_identity() -> Result<(), ObjectStreamError>
    {
        let type_id = crate::types::AZ_U64;
        let stream = ObjectStream {
            version: 3,
            elements: vec![Element::new(type_id).with_data(42_u64.to_be_bytes())],
            ..ObjectStream::default()
        };
        let bytes = stream.to_bytes()?;

        let mut value = TypedValue::default();
        parse_streaming_bytes(&bytes, None, &mut value)?;
        assert_eq!(value.0, Some(42));
        Ok(())
    }

    #[test]
    fn dom_and_streaming_decode_v3_specialized_asset_through_classdata_serializer() {
        let specialized = Uuid::from_u128(0x12345678_9abc_def0_1234_56789abcdef0);
        let text = "id={01234567-89AB-CDEF-FEDC-BA9876543210}:1,type={00112233-4455-6677-8899-AABBCCDDEEFF},hint={levels/test}";
        let data = crate::asset_reference::AssetValueLayout::from_text(text, 1)
            .unwrap()
            .to_big_endian_bytes(1)
            .unwrap();
        let mut element = Element::new(specialized).with_data(data);
        element.version = Some(1);
        let stream = ObjectStream {
            version: 3,
            elements: vec![element],
            ..ObjectStream::default()
        };
        let bytes = stream.to_bytes().unwrap();

        let mut context = ObjectStreamReadContext::default();
        let class = context
            .insert_class(
                specialized,
                crate::context::ReflectedClass::new(crate::types::ASSET).with_version(1),
            )
            .unwrap();
        context
            .insert_builtin_codec(class, crate::types::ASSET, 1)
            .unwrap();

        let dom = ObjectStream::from_bytes_with_context(&bytes, &context).unwrap();
        assert_eq!(
            crate::asset_reference::read_asset_value(&dom.elements()[0])
                .unwrap()
                .hint(),
            "levels/test"
        );

        let mut visitor = AssetHint::default();
        parse_streaming_bytes_with_context(&bytes, &context, &mut visitor).unwrap();
        assert_eq!(visitor.0.as_deref(), Some("levels/test"));
    }

    #[test]
    fn streaming_reader_handles_deep_objectstream_without_recursion()
    -> Result<(), ObjectStreamError> {
        let bytes = deep_binary_chain(20_000);
        let mut visitor = CountVisitor::default();

        let version = parse_streaming_bytes(&bytes, None, &mut visitor)?;

        assert_eq!(version, 3);
        assert_eq!(visitor.opens, 20_000);
        assert_eq!(visitor.closes, 20_000);
        Ok(())
    }

    #[test]
    fn dom_and_streaming_expose_identical_raw_and_semantic_identity()
    -> Result<(), ObjectStreamError> {
        let raw = Uuid::from_u128(0x11);
        let specialization = Uuid::from_u128(0x22);
        let mut bytes = vec![StreamTag::BINARY.0];
        bytes.extend_from_slice(&2_u32.to_be_bytes());
        bytes.push(crate::ST_BINARYFLAG_ELEMENT_HEADER);
        bytes.extend_from_slice(raw.as_bytes());
        bytes.extend_from_slice(specialization.as_bytes());
        bytes.extend_from_slice(&[0, 0]);

        let mut context = ObjectStreamReadContext::default();
        context
            .insert_class(specialization, ReflectedClass::new(specialization))
            .unwrap();
        let dom = ObjectStream::from_bytes_with_context(&bytes, &context)?;
        let element = &dom.elements()[0];

        let mut identity = Identity::default();
        parse_streaming_bytes_with_context(&bytes, &context, &mut identity)?;
        assert_eq!(identity.raw, Some(*element.raw_type_id()));
        assert_eq!(identity.semantic, element.resolved_type_id().copied());
        assert_eq!(identity.resolution, Some(element.type_resolution()));
        assert_eq!(identity.raw, Some(raw));
        assert_eq!(identity.semantic, Some(specialization));
        Ok(())
    }

    fn deep_binary_chain(depth: usize) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(5 + depth * 17 + depth + 1);
        bytes.push(StreamTag::BINARY.0);
        bytes.extend_from_slice(&3u32.to_be_bytes());
        let id = Uuid::nil();
        for _ in 0..depth {
            bytes.push(crate::ST_BINARYFLAG_ELEMENT_HEADER);
            bytes.extend_from_slice(id.as_bytes());
        }
        bytes.extend(std::iter::repeat_n(0, depth + 1));
        bytes
    }
}
