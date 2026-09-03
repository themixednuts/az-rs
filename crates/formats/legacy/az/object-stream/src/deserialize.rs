//! `ObjectStream` payload deserialization.
//!
//! Raw entrypoints preserve wire identity and text without pretending a
//! `SerializeContext` was available. Context-aware entrypoints reproduce native
//! `ClassData` lookup, apply the context's explicit unregistered-class policy,
//! and invoke only serializers explicitly registered for resolved `ClassData`.

use std::io::{self, Cursor, Read};
use std::str;

use arcstr::ArcStr;
use az_core::type_info::TypeInfo;
use serde_json::Value;

use crate::binary::{
    BinaryElement, BinaryElementHeader, ensure_reader_exhausted, read_element_header,
    read_stream_header,
};
use crate::codec::{ObjectStreamValueCodec, TextEncoding};
use crate::context::{
    ChildFate, ElementFate, ObjectStreamReadContext, ObjectStreamTypeResolver, ReflectedClassKey,
    ResolveTypeInput, ResolvedType, VersionConversionState,
};
use crate::lookup::LumberyardHashes;
use crate::validate_stream_version;
use crate::{
    Element, JSONElement, JSONObjectStream, ObjectStream, ObjectStreamEncoding, ObjectStreamError,
    PayloadEncoding, StreamTag, TypeResolution, XMLElement, XMLObjectStream,
};

struct PendingElement {
    element: Element,
    class: Option<ReflectedClassKey>,
    has_converting_ancestor: bool,
    discard_subtree: bool,
}

const XML_INTRINSIC_TYPE_INFO: [TypeInfo; 13] = [
    TypeInfo::of::<bool>(),
    TypeInfo::of::<i8>(),
    TypeInfo::of::<i16>(),
    TypeInfo::of::<i32>(),
    TypeInfo::of::<i64>(),
    TypeInfo::of::<u8>(),
    TypeInfo::of::<u16>(),
    TypeInfo::of::<u32>(),
    TypeInfo::of::<u64>(),
    TypeInfo::of::<f32>(),
    TypeInfo::of::<f64>(),
    TypeInfo::of::<()>(),
    TypeInfo::of::<String>(),
];

fn xml_name_identifies_type(type_id: uuid::Uuid, name: &str) -> bool {
    XML_INTRINSIC_TYPE_INFO
        .iter()
        .any(|info| info.type_id == type_id && info.name == name)
        || name.contains("::")
        || (name.contains('<') && name.ends_with('>'))
}

fn xml_semantic_field(value: &XMLElement) -> Option<String> {
    // Native XML has used both an explicit `field` attribute and a legacy
    // `name`-as-field spelling. Modern streams also use `name` for the C++
    // type label when `field` is absent, so only retain the fallback when the
    // name does not identify the element's type.
    value.field.clone().or_else(|| {
        (!value.name.is_empty() && !xml_name_identifies_type(value.id, &value.name))
            .then(|| value.name.clone())
    })
}

/// Read a binary `ObjectStream` from `reader`, preserving wire identity.
///
/// # Errors
///
/// Returns [`ObjectStreamError::Io`] if `reader` fails or ends mid-element,
/// [`ObjectStreamError::InvalidStreamTag`] if the first byte is not the binary
/// tag, [`ObjectStreamError::UnsupportedVersion`] for a stream version above 3,
/// [`ObjectStreamError::InvalidElementFlags`] for a header whose flag byte is
/// not a legal combination, [`ObjectStreamError::UnsupportedSizeWidth`] for an
/// extra-size field that is not 1, 2 or 4 bytes wide,
/// [`ObjectStreamError::Uuid`] for a malformed type UUID, and
/// [`ObjectStreamError::TrailingDataAfterRoot`] if bytes remain after the root
/// terminator.
pub fn from_reader<R: Read>(
    reader: &mut R,
    hashes: Option<&LumberyardHashes>,
) -> Result<ObjectStream, ObjectStreamError> {
    from_reader_impl(reader, hashes, None)
}

/// Read a binary `ObjectStream` from `reader`, resolving reflected
/// `ClassData` through `context`.
///
/// # Errors
///
/// Returns [`ObjectStreamError::IncompleteReadContext`] before reading anything
/// if `context` still has captured serializers with no registered
/// implementation, then every error [`from_reader`] returns. Resolution adds
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
pub fn from_reader_with_context<R: Read>(
    reader: &mut R,
    context: &ObjectStreamReadContext,
) -> Result<ObjectStream, ObjectStreamError> {
    context.validate_complete()?;
    from_reader_impl(reader, Some(context.names()), Some(context))
}

fn from_reader_impl<R: Read>(
    reader: &mut R,
    hashes: Option<&LumberyardHashes>,
    context: Option<&ObjectStreamReadContext>,
) -> Result<ObjectStream, ObjectStreamError> {
    let version = read_stream_header(reader)?;
    let mut stream = ObjectStream::new(version);
    let mut stack: Vec<PendingElement> = Vec::new();

    loop {
        match read_element_header(reader, version, hashes)? {
            BinaryElement::Header(header) => {
                open_binary_element(&header, reader, version, hashes, context, &mut stack)?;
            }
            BinaryElement::EndOfList => {
                if close_binary_element(&mut stack, &mut stream, context, version)?
                    == BinaryListEnd::RootTerminator
                {
                    break;
                }
            }
        }
    }
    ensure_reader_exhausted(reader)?;
    Ok(stream)
}

/// Push one binary element header (and its payload) onto the pending stack.
fn open_binary_element<'a, R: Read>(
    header: &BinaryElementHeader<'a>,
    reader: &mut R,
    version: u32,
    hashes: Option<&'a LumberyardHashes>,
    context: Option<&ObjectStreamReadContext>,
    stack: &mut Vec<PendingElement>,
) -> Result<(), ObjectStreamError> {
    let discarded_by_ancestor = stack.last().is_some_and(|parent| parent.discard_subtree);
    let resolved = if discarded_by_ancestor {
        None
    } else {
        context.map(|context| {
            context.resolve_type(ResolveTypeInput {
                stream_version: version,
                raw_type_id: header.id,
                specialization_type_id: header.specialization,
                element_version: header.version.unwrap_or(0),
                name_crc: header.name_crc,
                parent: stack.last().and_then(|parent| parent.class),
            })
        })
    };
    let discarded_by_unregistered_class =
        if let (Some(context), Some(resolved)) = (context, resolved) {
            context.resolved_element_fate(resolved, header.id)? == ElementFate::Skip
        } else {
            false
        };
    let version_state = match (
        context,
        (!discarded_by_unregistered_class)
            .then_some(resolved)
            .flatten()
            .and_then(|resolved| resolved.class),
    ) {
        (Some(context), Some(class)) => {
            context.validate_class_version(class, header.version.unwrap_or(0))?
        }
        _ => VersionConversionState::default(),
    };
    let has_converting_ancestor = stack.last().is_some_and(|ancestor| {
        ancestor.has_converting_ancestor || ancestor.element.version_state.requires_dom_converter()
    });
    let discard_subtree = discarded_by_ancestor
        || discarded_by_unregistered_class
        || (version_state.discards_subtree() && !has_converting_ancestor);
    if !discard_subtree
        && let (Some(context), Some(class)) =
            (context, resolved.and_then(|resolved| resolved.class))
    {
        context.validate_payload_support(class, header.data_size.is_some())?;
    }
    let resolution = resolution_state(
        context,
        resolved,
        stack.last().and_then(|parent| parent.class),
        header.name_crc,
    );
    let semantic_name = resolved
        .and_then(|resolved| resolved.type_id)
        .and_then(|type_id| hashes.and_then(|hashes| hashes.type_name(&type_id)));
    let mut element = Element {
        flags: header.flags,
        name_crc: header.name_crc,
        version: header.version,
        id: header.id,
        specialization: header.specialization,
        resolution,
        version_state,
        name: semantic_name.or(header.name).cloned().unwrap_or_default(),
        field: header.field.cloned(),
        data_size: header.data_size,
        payload_encoding: PayloadEncoding::BinaryBigEndian,
        ..Default::default()
    };
    if let Some(data_size) = element.data_size {
        let mut data = vec![0; data_size];
        reader.read_exact(&mut data)?;
        element.data = Some(data);
    }
    stack.push(PendingElement {
        element,
        class: resolved.and_then(|resolved| resolved.class),
        has_converting_ancestor,
        discard_subtree,
    });
    Ok(())
}

/// Which end-of-list marker [`close_binary_element`] just consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinaryListEnd {
    /// A child list closed; the element was attached, skipped, or discarded.
    Element,
    /// The stack was already empty, so this was the root terminator.
    RootTerminator,
}

/// Close the innermost pending element and attach it to its parent.
fn close_binary_element(
    stack: &mut Vec<PendingElement>,
    stream: &mut ObjectStream,
    context: Option<&ObjectStreamReadContext>,
    version: u32,
) -> Result<BinaryListEnd, ObjectStreamError> {
    let Some(mut pending) = stack.pop() else {
        return Ok(BinaryListEnd::RootTerminator);
    };
    if pending.discard_subtree {
        if !stack.last().is_some_and(|parent| parent.discard_subtree) {
            // A deprecated ClassData with no converter discards its complete
            // subtree.
            stream.skipped_children += 1;
        }
        return Ok(BinaryListEnd::Element);
    }
    if let (Some(context), Some(class)) = (context, pending.class) {
        let parent = stack.last().and_then(|parent| parent.class);
        let defer_parent_validation = pending.has_converting_ancestor;
        pending.class = Some(context.convert_element(
            class,
            pending.element.version_state,
            &mut pending.element,
            version,
            parent,
        )?);
        if !defer_parent_validation {
            context.validate_container_cardinality(
                pending.class.expect("converted ClassData remains resolved"),
                pending.element.elements.len(),
            )?;
        }
        if pending.element.resolved_type_id() == Some(&crate::types::DATA_OVERLAY_INFO) {
            pending.element =
                context.materialize_data_overlay(&pending.element, version, parent)?;
            pending.class = pending.element.resolved_class();
        } else if let Some(parent) = parent
            && !defer_parent_validation
        {
            match context.finalize_reachable_child(parent, &mut pending.element, version)? {
                ChildFate::Retain => {
                    pending.class = pending.element.resolved_class();
                }
                ChildFate::Skip => {
                    // Native default-filter load discards the subtree
                    // and continues; drop it without pushing.
                    stream.skipped_children += 1;
                    return Ok(BinaryListEnd::Element);
                }
            }
        }
    }
    if let Some(parent) = stack.last_mut() {
        parent.element.elements.push(pending.element);
    } else {
        stream.elements.push(pending.element);
    }
    Ok(BinaryListEnd::Element)
}

/// Read an `ObjectStream` from `bytes`, dispatching on the leading stream tag.
///
/// # Errors
///
/// Returns [`ObjectStreamError::Io`] with
/// [`std::io::ErrorKind::UnexpectedEof`] for empty `bytes` and
/// [`ObjectStreamError::InvalidStreamTag`] when the first byte is not a known
/// tag. A binary payload returns everything [`from_reader`] returns. A text
/// payload additionally returns [`ObjectStreamError::Utf8`] for non-UTF-8
/// bytes, [`ObjectStreamError::Xml`] or [`ObjectStreamError::Json`] for a
/// malformed document, [`ObjectStreamError::UnsupportedVersion`] for a version
/// above 3, [`ObjectStreamError::MissingV2Specialization`] or
/// [`ObjectStreamError::UnexpectedSpecialization`] for a specialization slot
/// that disagrees with the stream version, and
/// [`ObjectStreamError::ValueConversion`] when a leaf's text payload cannot be
/// decoded by the builtin codec its wire UUID implies.
pub fn from_bytes(
    bytes: &[u8],
    hashes: Option<&LumberyardHashes>,
) -> Result<ObjectStream, ObjectStreamError> {
    from_bytes_impl(bytes, hashes, None)
}

/// Read an `ObjectStream` from `bytes`, resolving reflected `ClassData`
/// through `context`.
///
/// # Errors
///
/// Returns [`ObjectStreamError::IncompleteReadContext`] before reading anything
/// if `context` still has captured serializers with no registered
/// implementation, then every error [`from_bytes`] returns plus the resolution
/// failures listed on [`from_reader_with_context`].
pub fn from_bytes_with_context(
    bytes: &[u8],
    context: &ObjectStreamReadContext,
) -> Result<ObjectStream, ObjectStreamError> {
    context.validate_complete()?;
    from_bytes_impl(bytes, Some(context.names()), Some(context))
}

fn from_bytes_impl(
    bytes: &[u8],
    hashes: Option<&LumberyardHashes>,
    context: Option<&ObjectStreamReadContext>,
) -> Result<ObjectStream, ObjectStreamError> {
    let Some((&tag, _)) = bytes.split_first() else {
        return Err(ObjectStreamError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "empty ObjectStream payload",
        )));
    };

    match StreamTag::from_byte(tag) {
        Some(StreamTag::BINARY) => {
            let mut cursor = Cursor::new(bytes);
            from_reader_impl(&mut cursor, hashes, context)
        }
        Some(StreamTag::XML) => {
            let xml: XMLObjectStream = quick_xml::de::from_str(str::from_utf8(bytes)?)?;
            let version = validate_stream_version(xml.version)?;
            let mut skipped_children = 0usize;
            let elements = xml
                .elements
                .into_iter()
                .map(|element| {
                    xml_element(
                        element,
                        version,
                        hashes,
                        context,
                        None,
                        false,
                        &mut skipped_children,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect();
            Ok(ObjectStream {
                tag: StreamTag::XML,
                version,
                elements,
                skipped_children,
            })
        }
        Some(StreamTag::JSON) => {
            let json: JSONObjectStream = serde_json::from_slice(bytes)?;
            let version = validate_stream_version(json.version)?;
            let mut skipped_children = 0usize;
            let elements = json
                .elements
                .into_iter()
                .map(|element| {
                    json_element(
                        element,
                        version,
                        hashes,
                        context,
                        None,
                        false,
                        &mut skipped_children,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect();
            Ok(ObjectStream {
                tag: StreamTag::JSON,
                version,
                elements,
                skipped_children,
            })
        }
        _ => Err(ObjectStreamError::InvalidStreamTag(tag)),
    }
}

fn xml_element(
    value: XMLElement,
    stream_version: u32,
    hashes: Option<&LumberyardHashes>,
    context: Option<&ObjectStreamReadContext>,
    parent: Option<ReflectedClassKey>,
    defer_parent_validation: bool,
    skipped: &mut usize,
) -> Result<Option<Element>, ObjectStreamError> {
    crate::validate_element_specialization(stream_version, value.id, value.specialization)?;
    let semantic_field = xml_semantic_field(&value);
    let name_crc = semantic_field.as_deref().map(crate::field_name_crc);
    // v2 streams carry an explicit specialization slot; a missing one is the
    // nil UUID rather than "no specialization".
    let specialization =
        if stream_version == 2 && value.id != crate::types::ASSET && value.specialization.is_none()
        {
            Some(uuid::Uuid::nil())
        } else {
            value.specialization
        };
    let resolved = context.map(|context| {
        resolve_text_type(
            context,
            stream_version,
            value.id,
            specialization,
            value.version,
            name_crc,
            parent,
        )
    });
    if let (Some(context), Some(resolved)) = (context, resolved)
        && context.resolved_element_fate(resolved, value.id)? == ElementFate::Skip
    {
        *skipped += 1;
        return Ok(None);
    }
    let version_state = match (context, resolved.and_then(|resolved| resolved.class)) {
        (Some(context), Some(class)) => {
            context.validate_class_version(class, value.version.unwrap_or(0))?
        }
        _ => VersionConversionState::default(),
    };
    if version_state.discards_subtree() && !defer_parent_validation {
        *skipped += 1;
        return Ok(None);
    }
    if let (Some(context), Some(class)) = (context, resolved.and_then(|resolved| resolved.class)) {
        context.validate_payload_support(class, value.value.is_some())?;
    }
    let (data, payload_encoding) = decode_text_value(
        value.value.as_ref(),
        value.id,
        value.version.unwrap_or(0),
        TextEncoding::Xml,
        context,
        resolved,
    )?;
    let mut elements = Vec::new();
    for child in value.elements {
        if let Some(child) = xml_element(
            child,
            stream_version,
            hashes,
            context,
            resolved.and_then(|resolved| resolved.class),
            defer_parent_validation || version_state.requires_dom_converter(),
            skipped,
        )? {
            elements.push(child);
        }
    }
    let semantic_name = resolved
        .and_then(|resolved| resolved.type_id)
        .and_then(|type_id| hashes.and_then(|hashes| hashes.type_name(&type_id)));

    let element = Element {
        id: value.id,
        specialization: value.specialization,
        resolution: resolution_state(context, resolved, parent, name_crc),
        version_state,
        name: semantic_name
            .cloned()
            .unwrap_or_else(|| ArcStr::from(value.name)),
        field: semantic_field.map(ArcStr::from),
        name_crc,
        version: value.version,
        data_size: data.as_ref().map(Vec::len),
        data,
        payload_encoding,
        elements,
        ..Default::default()
    };
    finalize_text_element(
        element,
        stream_version,
        context,
        resolved,
        parent,
        defer_parent_validation,
        skipped,
    )
}

fn json_element(
    value: JSONElement,
    stream_version: u32,
    hashes: Option<&LumberyardHashes>,
    context: Option<&ObjectStreamReadContext>,
    parent: Option<ReflectedClassKey>,
    defer_parent_validation: bool,
    skipped: &mut usize,
) -> Result<Option<Element>, ObjectStreamError> {
    crate::validate_element_specialization(stream_version, value.id, value.specialization)?;
    let name_crc = value.field.as_deref().map(crate::field_name_crc);
    let resolved = context.map(|context| {
        resolve_text_type(
            context,
            stream_version,
            value.id,
            value.specialization,
            value.version,
            name_crc,
            parent,
        )
    });
    if let (Some(context), Some(resolved)) = (context, resolved)
        && context.resolved_element_fate(resolved, value.id)? == ElementFate::Skip
    {
        *skipped += 1;
        return Ok(None);
    }
    let version_state = match (context, resolved.and_then(|resolved| resolved.class)) {
        (Some(context), Some(class)) => {
            context.validate_class_version(class, value.version.unwrap_or(0))?
        }
        _ => VersionConversionState::default(),
    };
    if version_state.discards_subtree() && !defer_parent_validation {
        *skipped += 1;
        return Ok(None);
    }
    if let (Some(context), Some(class)) = (context, resolved.and_then(|resolved| resolved.class)) {
        context.validate_payload_support(class, value.value.is_some())?;
    }
    let (data, payload_encoding) = decode_text_value(
        value.value.as_ref(),
        value.id,
        value.version.unwrap_or(0),
        TextEncoding::Json,
        context,
        resolved,
    )?;
    let mut elements = Vec::new();
    for child in value.elements.unwrap_or_default() {
        if let Some(child) = json_element(
            child,
            stream_version,
            hashes,
            context,
            resolved.and_then(|resolved| resolved.class),
            defer_parent_validation || version_state.requires_dom_converter(),
            skipped,
        )? {
            elements.push(child);
        }
    }
    let semantic_name = resolved
        .and_then(|resolved| resolved.type_id)
        .and_then(|type_id| hashes.and_then(|hashes| hashes.type_name(&type_id)));

    let element = Element {
        id: value.id,
        specialization: value.specialization,
        resolution: resolution_state(context, resolved, parent, name_crc),
        version_state,
        name: semantic_name
            .cloned()
            .unwrap_or_else(|| ArcStr::from(value.name)),
        field: value.field.map(ArcStr::from),
        name_crc,
        version: value.version,
        data_size: data.as_ref().map(Vec::len),
        data,
        payload_encoding,
        elements,
        ..Default::default()
    };
    finalize_text_element(
        element,
        stream_version,
        context,
        resolved,
        parent,
        defer_parent_validation,
        skipped,
    )
}

/// Apply reflected `ClassData` conversion and parent validation to a freshly
/// built text element.
///
/// Shared by the XML and JSON readers. `Ok(None)` means the native
/// default-filter load discarded the subtree, which the caller counts in
/// `skipped`.
fn finalize_text_element(
    mut element: Element,
    stream_version: u32,
    context: Option<&ObjectStreamReadContext>,
    resolved: Option<ResolvedType>,
    parent: Option<ReflectedClassKey>,
    defer_parent_validation: bool,
    skipped: &mut usize,
) -> Result<Option<Element>, ObjectStreamError> {
    let (Some(context), Some(class)) = (context, resolved.and_then(|resolved| resolved.class))
    else {
        return Ok(Some(element));
    };
    let version_state = element.version_state;
    context.convert_element(class, version_state, &mut element, stream_version, parent)?;
    if !defer_parent_validation {
        context.validate_container_cardinality(
            element
                .resolved_class()
                .expect("converted ClassData remains resolved"),
            element.elements.len(),
        )?;
    }
    if element.resolved_type_id() == Some(&crate::types::DATA_OVERLAY_INFO) {
        element = context.materialize_data_overlay(&element, stream_version, parent)?;
    } else if let Some(parent) = parent
        && !defer_parent_validation
    {
        match context.finalize_reachable_child(parent, &mut element, stream_version)? {
            ChildFate::Retain => {}
            ChildFate::Skip => {
                // Native default-filter load discards the subtree and continues.
                *skipped += 1;
                return Ok(None);
            }
        }
    }
    Ok(Some(element))
}

fn resolve_text_type(
    context: &ObjectStreamReadContext,
    stream_version: u32,
    raw_type_id: uuid::Uuid,
    specialization_type_id: Option<uuid::Uuid>,
    element_version: Option<u32>,
    name_crc: Option<u32>,
    parent: Option<ReflectedClassKey>,
) -> ResolvedType {
    context.resolve_type(ResolveTypeInput {
        stream_version,
        raw_type_id,
        specialization_type_id,
        element_version: element_version.unwrap_or(0),
        name_crc,
        parent,
    })
}

fn resolution_state(
    context: Option<&ObjectStreamReadContext>,
    resolved: Option<ResolvedType>,
    parent: Option<ReflectedClassKey>,
    name_crc: Option<u32>,
) -> TypeResolution {
    match (context, resolved) {
        (None, _) => TypeResolution::Raw,
        (
            Some(context),
            Some(ResolvedType {
                type_id: Some(type_id),
                class: Some(class),
                builtin_serializer,
                is_container,
                container_shape,
                ambiguous_generic: false,
            }),
        ) => TypeResolution::Resolved {
            type_id,
            class,
            edge: context.resolved_edge(parent, name_crc),
            enum_type_id: context.resolved_enum_type(parent, name_crc, type_id),
            builtin_serializer,
            is_container,
            container_shape,
        },
        (Some(_), _) => TypeResolution::Unresolved,
    }
}

fn decode_text_value(
    value: Option<&Value>,
    raw_type_id: uuid::Uuid,
    element_version: u32,
    encoding: TextEncoding,
    context: Option<&ObjectStreamReadContext>,
    resolved: Option<ResolvedType>,
) -> Result<(Option<Vec<u8>>, PayloadEncoding), ObjectStreamError> {
    let Some(value) = value else {
        return Ok((None, PayloadEncoding::Text));
    };
    let text = value
        .as_str()
        .ok_or(crate::codec::ValueCodecError::NonStringJsonValue)
        .map_err(|source| ObjectStreamError::ValueConversion {
            type_id: raw_type_id,
            source,
        })?;

    // Prefer ClassData-registered codecs when the element is resolved.
    if let Some((context, resolved_type, class)) =
        context.zip(resolved).and_then(|(context, resolved)| {
            resolved
                .type_id
                .zip(resolved.class)
                .map(|pair| (context, pair.0, pair.1))
        })
        && let Some(codec) = context.codec(class)
    {
        let payload = codec
            .text_to_data(resolved_type, text, element_version, encoding)
            .map_err(|source| ObjectStreamError::ValueConversion {
                type_id: resolved_type,
                source,
            })?;
        return Ok((Some(payload.bytes), payload.encoding));
    }

    // Raw dumps / fixtures without ClassData still convert known AZ builtins by
    // wire UUID so typed readers receive binary big-endian payloads (HEAD
    // Lumberyard text ObjectStream behavior).
    if let Some(kind) = crate::codec::builtin_serializer_kind(raw_type_id) {
        let codec = crate::codec::BuiltinValueCodec::new(kind, element_version);
        let payload = codec
            .text_to_data(raw_type_id, text, element_version, encoding)
            .map_err(|source| ObjectStreamError::ValueConversion {
                type_id: raw_type_id,
                source,
            })?;
        // `text_to_data` emits the codec's native-endian payload. A raw element
        // keeps no serializer descriptor, so nothing downstream can recover
        // which platform produced those bytes; store the canonical big-endian
        // wire form instead.
        let bytes = codec
            .to_big_endian(
                raw_type_id,
                &payload.bytes,
                payload.encoding,
                element_version,
            )
            .map_err(|source| ObjectStreamError::ValueConversion {
                type_id: raw_type_id,
                source,
            })?;
        return Ok((Some(bytes), PayloadEncoding::BinaryBigEndian));
    }

    Ok((Some(text.as_bytes().to_vec()), PayloadEncoding::Text))
}

/// Read an `ObjectStream` from `bytes`, requiring it to be in `encoding`.
///
/// # Errors
///
/// Returns [`ObjectStreamError::Io`] with
/// [`std::io::ErrorKind::UnexpectedEof`] for empty `bytes`,
/// [`ObjectStreamError::InvalidStreamTag`] when the first byte is not a known
/// tag, and [`ObjectStreamError::UnexpectedEncoding`] when the payload's own
/// encoding is not `encoding`. Otherwise returns whatever [`from_bytes`]
/// returns.
pub fn from_encoding_bytes(
    bytes: &[u8],
    encoding: ObjectStreamEncoding,
    hashes: Option<&LumberyardHashes>,
) -> Result<ObjectStream, ObjectStreamError> {
    validate_encoding(bytes, encoding)?;
    from_bytes(bytes, hashes)
}

/// Read an `ObjectStream` from `bytes` in `encoding`, resolving reflected
/// `ClassData` through `context`.
///
/// # Errors
///
/// Returns [`ObjectStreamError::UnexpectedEncoding`] when the payload is not in
/// `encoding`, plus every error [`from_bytes_with_context`] returns.
pub fn from_encoding_bytes_with_context(
    bytes: &[u8],
    encoding: ObjectStreamEncoding,
    context: &ObjectStreamReadContext,
) -> Result<ObjectStream, ObjectStreamError> {
    validate_encoding(bytes, encoding)?;
    from_bytes_with_context(bytes, context)
}

fn validate_encoding(
    bytes: &[u8],
    encoding: ObjectStreamEncoding,
) -> Result<(), ObjectStreamError> {
    let Some((&tag, _)) = bytes.split_first() else {
        return Err(ObjectStreamError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "empty ObjectStream payload",
        )));
    };
    let Some(actual) = ObjectStreamEncoding::from_tag_byte(tag) else {
        return Err(ObjectStreamError::InvalidStreamTag(tag));
    };
    if actual != encoding {
        return Err(ObjectStreamError::UnexpectedEncoding {
            expected: encoding,
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::context::{
        ObjectStreamVersionConverter, ReflectedClass, ReflectedField, UnregisteredClassPolicy,
    };
    use crate::types;

    fn int_context() -> ObjectStreamReadContext {
        let mut context = ObjectStreamReadContext::default();
        let class = context
            .insert_class(types::INT, ReflectedClass::new(types::INT))
            .unwrap();
        context.insert_builtin_codec(class, types::INT, 0).unwrap();
        context
    }

    #[derive(Debug)]
    struct RenameOld;

    impl ObjectStreamVersionConverter for RenameOld {
        fn convert(
            &self,
            element: &mut Element,
            _from: u32,
            _to: u32,
        ) -> Result<(), ObjectStreamError> {
            element.field = Some(ArcStr::from("converted"));
            Ok(())
        }
    }

    #[derive(Debug)]
    struct RenameLegacyChild;

    impl ObjectStreamVersionConverter for RenameLegacyChild {
        fn convert(
            &self,
            element: &mut Element,
            _from: u32,
            _to: u32,
        ) -> Result<(), ObjectStreamError> {
            let child = element.elements.first_mut().ok_or(
                ObjectStreamError::UnexpectedReachableChild {
                    parent_type_id: element.id,
                    child_type_id: element.id,
                    name_crc: None,
                },
            )?;
            child.field = Some(ArcStr::from("CurrentChild"));
            child.name_crc = Some(crate::field_name_crc("CurrentChild"));
            Ok(())
        }
    }

    #[test]
    fn dom_runs_registered_version_converter() {
        let id = uuid::Uuid::from_u128(77);
        let json = format!(
            r#"{{"name":"ObjectStream","version":3,"Objects":[{{"typeId":"{{{id}}}","typeName":"Old","version":0,"Objects":[]}}]}}"#
        );
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
            .insert_version_converter(class, Arc::new(RenameOld))
            .unwrap();

        let stream = ObjectStream::from_bytes_with_context(json.as_bytes(), &context).unwrap();
        assert_eq!(stream.elements()[0].version(), Some(1));
        assert_eq!(
            stream.elements()[0].field().map(arcstr::ArcStr::as_str),
            Some("converted")
        );
        assert_eq!(
            stream.elements()[0].version_state,
            crate::context::VersionConversionState::Current
        );
    }

    #[test]
    fn binary_discards_deprecated_subtree_without_resolving_its_descendants() {
        let parent_id = uuid::Uuid::from_u128(80);
        let deprecated_id = uuid::Uuid::from_u128(81);
        let unknown_descendant_id = uuid::Uuid::from_u128(82);
        let field = "EditorMetadata";

        let mut parent = ReflectedClass::new(parent_id);
        parent.insert_field(
            crate::field_name_crc(field),
            crate::context::ReflectedField::new(deprecated_id),
        );
        let mut context = ObjectStreamReadContext::default();
        context.insert_class(parent_id, parent).unwrap();
        context
            .insert_class(
                deprecated_id,
                ReflectedClass::new(deprecated_id).deprecated(),
            )
            .unwrap();

        let raw = ObjectStream {
            version: 3,
            elements: vec![
                Element::new(parent_id).with_children([Element::new(deprecated_id)
                    .with_field(field)
                    .with_children([Element::new(unknown_descendant_id)])]),
            ],
            ..ObjectStream::default()
        }
        .to_bytes()
        .unwrap();

        let stream = ObjectStream::from_bytes_with_context(&raw, &context).unwrap();
        assert_eq!(stream.elements().len(), 1);
        assert!(stream.elements()[0].children().is_empty());
        assert_eq!(stream.skipped_children, 1);
    }

    fn unregistered_class_fixture() -> (uuid::Uuid, uuid::Uuid, uuid::Uuid, ObjectStreamReadContext)
    {
        let parent_id = uuid::Uuid::from_u128(0x901);
        let unknown_id = uuid::Uuid::from_u128(0x902);
        let unknown_descendant_id = uuid::Uuid::from_u128(0x903);
        let mut parent = ReflectedClass::new(parent_id);
        parent.insert_field(
            crate::field_name_crc("known"),
            ReflectedField::new(types::INT),
        );
        let mut context = ObjectStreamReadContext::default()
            .with_unregistered_class_policy(UnregisteredClassPolicy::NativeSkip);
        context.insert_class(parent_id, parent).unwrap();
        let int = context
            .insert_class(types::INT, ReflectedClass::new(types::INT))
            .unwrap();
        context.insert_builtin_codec(int, types::INT, 0).unwrap();
        (parent_id, unknown_id, unknown_descendant_id, context)
    }

    #[test]
    fn native_skip_discards_complete_unregistered_subtrees_in_every_encoding() {
        let (parent_id, unknown_id, unknown_descendant_id, context) = unregistered_class_fixture();
        let binary = ObjectStream {
            version: 3,
            elements: vec![
                Element::new(parent_id).with_children([
                    Element::new(unknown_id)
                        .with_field("unknown")
                        .with_children([Element::new(unknown_descendant_id)]),
                    Element::new(types::INT).with_field("known"),
                ]),
            ],
            ..ObjectStream::default()
        }
        .to_bytes()
        .unwrap();
        let xml = format!(
            r#"<ObjectStream version="3"><Class name="Parent" type="{{{parent_id}}}"><Class name="Unknown" field="unknown" type="{{{unknown_id}}}"><Class name="UnknownDescendant" type="{{{unknown_descendant_id}}}"/></Class><Class name="int" field="known" value="7" type="{{{int_id}}}"/></Class></ObjectStream>"#,
            int_id = types::INT,
        );
        let json = format!(
            r#"{{"name":"ObjectStream","version":3,"Objects":[{{"typeId":"{{{parent_id}}}","typeName":"Parent","Objects":[{{"field":"unknown","typeId":"{{{unknown_id}}}","typeName":"Unknown","Objects":[{{"typeId":"{{{unknown_descendant_id}}}","typeName":"UnknownDescendant","Objects":[]}}]}},{{"field":"known","typeId":"{{{int_id}}}","typeName":"int","value":"7","Objects":[]}}]}}]}}"#,
            int_id = types::INT,
        );

        for bytes in [binary, xml.into_bytes(), json.into_bytes()] {
            let stream = ObjectStream::from_bytes_with_context(&bytes, &context).unwrap();
            assert_eq!(stream.elements().len(), 1);
            assert_eq!(stream.elements()[0].children().len(), 1);
            assert_eq!(
                stream.elements()[0].children()[0].resolved_type_id(),
                Some(&types::INT),
            );
            assert_eq!(stream.skipped_children, 1);
        }
    }

    #[test]
    fn strict_context_still_rejects_an_unregistered_class() {
        let unknown_id = uuid::Uuid::from_u128(0x904);
        let json = format!(
            r#"{{"name":"ObjectStream","version":3,"Objects":[{{"typeId":"{{{unknown_id}}}","typeName":"Unknown","Objects":[]}}]}}"#
        );

        assert!(matches!(
            ObjectStream::from_bytes_with_context(
                json.as_bytes(),
                &ObjectStreamReadContext::default()
            ),
            Err(ObjectStreamError::UnresolvedElementType { type_id }) if type_id == unknown_id
        ));
    }

    #[test]
    fn native_skip_applies_to_unregistered_root_elements() {
        let (parent_id, unknown_id, _, context) = unregistered_class_fixture();
        let json = format!(
            r#"{{"name":"ObjectStream","version":3,"Objects":[{{"typeId":"{{{unknown_id}}}","typeName":"Unknown","Objects":[]}},{{"typeId":"{{{parent_id}}}","typeName":"Parent","Objects":[]}}]}}"#
        );

        let stream = ObjectStream::from_bytes_with_context(json.as_bytes(), &context).unwrap();
        assert_eq!(stream.elements().len(), 1);
        assert_eq!(stream.elements()[0].resolved_type_id(), Some(&parent_id));
        assert_eq!(stream.skipped_children, 1);
    }

    #[test]
    fn parent_converter_sees_complete_legacy_child_before_reachability_validation() {
        let parent_id = uuid::Uuid::from_u128(78);
        let child_id = uuid::Uuid::from_u128(79);
        let json = format!(
            r#"{{"name":"ObjectStream","version":3,"Objects":[{{"typeId":"{{{parent_id}}}","typeName":"Parent","version":0,"Objects":[{{"field":"LegacyChild","typeId":"{{{child_id}}}","typeName":"Child","Objects":[]}}]}}]}}"#
        );
        let mut parent = ReflectedClass::new(parent_id)
            .with_version(1)
            .with_captured_version_converter();
        parent.insert_field(
            crate::field_name_crc("CurrentChild"),
            crate::context::ReflectedField::new(child_id),
        );
        let mut context = ObjectStreamReadContext::default();
        let parent = context.insert_class(parent_id, parent).unwrap();
        context
            .insert_class(child_id, ReflectedClass::new(child_id))
            .unwrap();
        context
            .insert_version_converter(parent, Arc::new(RenameLegacyChild))
            .unwrap();

        let stream = ObjectStream::from_bytes_with_context(json.as_bytes(), &context).unwrap();

        let child = &stream.elements()[0].children()[0];
        assert_eq!(child.field().map(AsRef::as_ref), Some("CurrentChild"));
        assert_eq!(
            child.name_crc(),
            Some(crate::field_name_crc("currentchild"))
        );
    }

    #[test]
    fn no_context_decodes_builtin_payload_without_assigning_semantic_identity() {
        let xml = format!(
            "<ObjectStream version=\"3\"><Class name=\"Count\" value=\"42\" type=\"{{{}}}\"/></ObjectStream>",
            types::INT.as_hyphenated().to_string().to_uppercase()
        );
        let stream = from_bytes(xml.as_bytes(), None).unwrap();
        let element = &stream.elements[0];
        assert_eq!(element.type_resolution(), crate::TypeResolutionState::Raw);
        // A raw element keeps no serializer descriptor, so its payload is
        // stored in the canonical big-endian wire form rather than the
        // decoding codec's native layout.
        assert_eq!(element.payload_encoding(), PayloadEncoding::BinaryBigEndian);
        assert_eq!(crate::value::read_i32(element).unwrap(), 42);
    }

    #[test]
    fn context_aware_load_rejects_unresolved_v2_before_typed_decode() {
        let raw = uuid::Uuid::from_u128(1);
        let specialization = uuid::Uuid::from_u128(2);
        let xml = format!(
            "<ObjectStream version=\"2\"><Class name=\"Count\" value=\"42\" type=\"{{{raw}}}\" specializationTypeId=\"{{{specialization}}}\"/></ObjectStream>"
        );
        assert_eq!(
            from_bytes_with_context(xml.as_bytes(), &ObjectStreamReadContext::default())
                .unwrap_err()
                .to_string(),
            ObjectStreamError::UnresolvedElementType { type_id: raw }.to_string()
        );
    }

    #[test]
    fn v2_missing_specialization_matches_native_xml_and_json_asymmetry() {
        let raw = uuid::Uuid::from_u128(0x77);
        let mut context = ObjectStreamReadContext::default();
        context.insert_class(raw, ReflectedClass::new(raw)).unwrap();
        let xml = format!(
            "<ObjectStream version=\"2\"><Class name=\"Value\" type=\"{{{raw}}}\"/></ObjectStream>"
        );
        let json = format!(
            "{{\"name\":\"ObjectStream\",\"version\":2,\"Objects\":[{{\"typeId\":\"{{{raw}}}\"}}]}}"
        );

        assert!(matches!(
            from_bytes_with_context(xml.as_bytes(), &context),
            Err(ObjectStreamError::UnresolvedElementType { type_id }) if type_id == raw
        ));
        let json = from_bytes_with_context(json.as_bytes(), &context).unwrap();
        assert_eq!(
            json.elements()[0].resolved_type_id(),
            Some(&raw),
            "native JSON keeps the raw id when specializationTypeId is absent"
        );
    }

    #[test]
    fn lumberyard_v2_asset_missing_specialization_uses_raw_asset_class() {
        let mut context = ObjectStreamReadContext::default()
            .with_dialect(crate::context::ObjectStreamDialect::Lumberyard);
        context
            .insert_class(types::ASSET, ReflectedClass::new(types::ASSET))
            .unwrap();
        let xml = format!(
            "<ObjectStream version=\"2\"><Class name=\"Asset\" type=\"{{{}}}\"/></ObjectStream>",
            types::ASSET
        );
        let stream = from_bytes_with_context(xml.as_bytes(), &context).unwrap();
        assert_eq!(stream.elements()[0].resolved_type_id(), Some(&types::ASSET));
    }

    #[test]
    fn json_value_must_be_a_string() {
        let json = format!(
            "{{\"name\":\"ObjectStream\",\"version\":3,\"Objects\":[{{\"typeId\":\"{{{}}}\",\"value\":42}}]}}",
            types::INT.as_hyphenated().to_string().to_uppercase()
        );
        assert!(matches!(
            from_bytes_with_context(json.as_bytes(), &int_context()),
            Err(ObjectStreamError::ValueConversion {
                source: crate::codec::ValueCodecError::NonStringJsonValue,
                ..
            })
        ));
    }

    #[test]
    fn xml_field_overrides_legacy_name_for_crc() {
        let xml = format!(
            "<ObjectStream version=\"3\"><Class name=\"Legacy\" field=\"Current\" type=\"{{{0}}}\"/><Class name=\"LegacyOnly\" type=\"{{{0}}}\"/></ObjectStream>",
            types::INT.as_hyphenated().to_string().to_uppercase(),
        );
        let stream = from_bytes(xml.as_bytes(), None).unwrap();
        let element = &stream.elements[0];
        assert_eq!(element.field().map(arcstr::ArcStr::as_str), Some("Current"));
        assert_eq!(element.name_crc(), Some(crate::field_name_crc("Current")));
        let legacy = &stream.elements[1];
        assert_eq!(
            legacy.field().map(arcstr::ArcStr::as_str),
            Some("LegacyOnly")
        );
        assert_eq!(legacy.name_crc(), Some(crate::field_name_crc("LegacyOnly")));
    }

    #[test]
    fn xml_type_labels_without_fields_remain_anonymous() {
        let xml = format!(
            "<ObjectStream version=\"3\"><Class name=\"bool\" type=\"{{{0}}}\"/><Class name=\"AZStd::vector&lt;AZStd::string&gt;\" type=\"{{{1}}}\"/></ObjectStream>",
            types::BOOL.as_hyphenated().to_string().to_uppercase(),
            <Vec<String> as az_core::type_info::AzTypeInfo>::TYPE_ID
                .as_hyphenated()
                .to_string()
                .to_uppercase(),
        );
        let stream = from_bytes(xml.as_bytes(), None).unwrap();

        for element in &stream.elements {
            assert_eq!(element.field(), None);
            assert_eq!(element.name_crc(), None);
        }
    }
}
