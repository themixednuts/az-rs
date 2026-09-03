//! `ObjectStream` serialization and transcoding.
//!
//! The canonical binary writer emits Lumberyard's `ObjectStream` shape.
//! XML output is written directly so very deep reflected assets do
//! not need a recursive XML mirror tree.

use std::io::{self, Cursor, Read, Write};

use crate::binary::{
    BinaryElement, ensure_reader_exhausted, read_element_header, read_stream_header,
};
use crate::codec::ObjectStreamValueCodec;
use crate::context::ObjectStreamReadContext;
use crate::deserialize;
use crate::lookup::LumberyardHashes;
use crate::{
    Element, ObjectStream, ObjectStreamEncoding, ObjectStreamError, PayloadEncoding,
    ST_BINARYFLAG_ELEMENT_HEADER, ST_BINARYFLAG_EXTRA_SIZE_FIELD, ST_BINARYFLAG_HAS_NAME,
    ST_BINARYFLAG_HAS_VALUE, ST_BINARYFLAG_HAS_VERSION, StreamTag,
};
use quick_xml::events::{BytesEnd, BytesStart, Event};
use serde_json::Value;

const XML_OBJECT_STREAM: &str = "ObjectStream";
const XML_CLASS: &str = "Class";

/// Encode `stream` in the requested `ObjectStream` encoding.
///
/// # Errors
///
/// Returns [`ObjectStreamError::UnsupportedVersion`] for a stream version
/// outside 0..=3, [`ObjectStreamError::MissingV2Specialization`] or
/// [`ObjectStreamError::UnexpectedSpecialization`] when an element's
/// specialization UUID does not match the stream version, and
/// [`ObjectStreamError::MissingSerializer`] when `encoding` differs from the
/// stream's source encoding and some element carries a non-text payload — that
/// re-encode needs [`to_encoding_bytes_with_context`]. The chosen writer then
/// contributes [`ObjectStreamError::FieldCrcMismatch`],
/// [`ObjectStreamError::ElementVersionOverflow`],
/// [`ObjectStreamError::PayloadTooLarge`] and
/// [`ObjectStreamError::Io`].
pub fn to_encoding_bytes(
    stream: &ObjectStream,
    encoding: ObjectStreamEncoding,
) -> Result<Vec<u8>, ObjectStreamError> {
    let mut bytes = Vec::new();
    write_as_impl(stream, encoding, None, &mut bytes)?;
    Ok(bytes)
}

/// Write `stream` in the requested `ObjectStream` encoding.
///
/// # Errors
///
/// Returns any error [`to_encoding_bytes`] returns, or
/// [`ObjectStreamError::Io`] if `writer` rejects the encoded bytes.
pub fn write_as<W: Write>(
    stream: &ObjectStream,
    encoding: ObjectStreamEncoding,
    writer: &mut W,
) -> Result<(), ObjectStreamError> {
    let bytes = to_encoding_bytes(stream, encoding)?;
    writer.write_all(&bytes)?;
    Ok(())
}

/// Encode `stream` in the requested encoding using reflected `ClassData`.
///
/// # Errors
///
/// Returns everything [`to_encoding_bytes`] returns except the payload-free
/// [`ObjectStreamError::MissingSerializer`] restriction, plus
/// [`ObjectStreamError::IncompleteReadContext`] if `context` still has
/// unregistered serializers,
/// [`ObjectStreamError::UnresolvedElementType`] for an element the context
/// cannot resolve, [`ObjectStreamError::UnexpectedReachableChild`] and
/// [`ObjectStreamError::InvalidContainerCardinality`] from graph validation,
/// and [`ObjectStreamError::ValueConversion`] when a captured serializer
/// rejects a payload.
pub fn to_encoding_bytes_with_context(
    stream: &ObjectStream,
    encoding: ObjectStreamEncoding,
    context: &ObjectStreamReadContext,
) -> Result<Vec<u8>, ObjectStreamError> {
    let mut bytes = Vec::new();
    write_as_impl(stream, encoding, Some(context), &mut bytes)?;
    Ok(bytes)
}

/// Write `stream` in the requested encoding using reflected `ClassData`.
///
/// # Errors
///
/// Returns any error [`to_encoding_bytes_with_context`] returns, or
/// [`ObjectStreamError::Io`] if `writer` rejects the encoded bytes.
pub fn write_as_with_context<W: Write>(
    stream: &ObjectStream,
    encoding: ObjectStreamEncoding,
    context: &ObjectStreamReadContext,
    writer: &mut W,
) -> Result<(), ObjectStreamError> {
    let bytes = to_encoding_bytes_with_context(stream, encoding, context)?;
    writer.write_all(&bytes)?;
    Ok(())
}

fn write_as_impl<W: Write>(
    stream: &ObjectStream,
    encoding: ObjectStreamEncoding,
    context: Option<&ObjectStreamReadContext>,
    writer: &mut W,
) -> Result<(), ObjectStreamError> {
    crate::validate_stream_version(stream.version)?;
    for element in stream.iter_recursive() {
        crate::validate_element_specialization(stream.version, element.id, element.specialization)?;
    }
    let source_encoding = stream.encoding()?;
    if context.is_none()
        && encoding != source_encoding
        && let Some(element) = stream.iter_recursive().find(|element| {
            element.data.is_some() && element.payload_encoding != PayloadEncoding::Text
        })
    {
        return Err(ObjectStreamError::MissingSerializer {
            type_id: element.id,
        });
    }
    if let Some(context) = context {
        context.validate_complete()?;
        validate_context_graph(stream, context)?;
    }
    match encoding {
        ObjectStreamEncoding::Binary => write_binary(stream, context, writer),
        ObjectStreamEncoding::Xml => write_xml(stream, context, writer),
        ObjectStreamEncoding::Json => write_json(stream, context, writer),
    }
}

/// Convert `ObjectStream` bytes into another `ObjectStream` encoding.
///
/// Binary inputs are streamed into the requested output encoding so
/// deep reflected assets do not grow the call stack or require a
/// second mirror tree. Binary payloads require
/// [`transcode_bytes_with_context`] for `ClassData` serializer semantics;
/// the raw API can copy binary or transcode payload-free structure only.
/// Raw XML and JSON string payloads can transcode losslessly between the two
/// text encodings without a reflection context.
///
/// # Errors
///
/// Returns [`ObjectStreamError::Io`] with [`std::io::ErrorKind::UnexpectedEof`]
/// for empty `bytes` and [`ObjectStreamError::InvalidStreamTag`] when the first
/// byte is not a known stream tag. A binary input is re-parsed, so every
/// reader error reaches the caller; a binary input carrying non-text payloads
/// that must change encoding reports
/// [`ObjectStreamError::MissingSerializer`] and needs
/// [`transcode_bytes_with_context`]. XML and JSON inputs additionally
/// propagate [`ObjectStreamError::Xml`], [`ObjectStreamError::Json`] and
/// [`ObjectStreamError::Utf8`].
pub fn transcode_bytes(
    bytes: &[u8],
    encoding: ObjectStreamEncoding,
    hashes: Option<&LumberyardHashes>,
) -> Result<Vec<u8>, ObjectStreamError> {
    let mut output = Vec::new();
    transcode_to_writer_impl(bytes, encoding, hashes, &mut output)?;
    Ok(output)
}

/// Convert `ObjectStream` bytes into another encoding using reflected
/// `ClassData`.
///
/// # Errors
///
/// Returns any error [`deserialize::from_bytes_with_context`] returns while
/// reading `bytes`, then any error [`to_encoding_bytes_with_context`] returns
/// while writing the decoded stream back out.
pub fn transcode_bytes_with_context(
    bytes: &[u8],
    encoding: ObjectStreamEncoding,
    context: &ObjectStreamReadContext,
) -> Result<Vec<u8>, ObjectStreamError> {
    let stream = deserialize::from_bytes_with_context(bytes, context)?;
    to_encoding_bytes_with_context(&stream, encoding, context)
}

/// Write `ObjectStream` bytes in another `ObjectStream` encoding.
///
/// # Errors
///
/// Returns any error [`transcode_bytes`] returns, or
/// [`ObjectStreamError::Io`] if `writer` rejects the transcoded bytes.
pub fn transcode_to_writer<W: Write>(
    bytes: &[u8],
    encoding: ObjectStreamEncoding,
    hashes: Option<&LumberyardHashes>,
    writer: &mut W,
) -> Result<(), ObjectStreamError> {
    let mut output = Vec::new();
    transcode_to_writer_impl(bytes, encoding, hashes, &mut output)?;
    writer.write_all(&output)?;
    Ok(())
}

fn transcode_to_writer_impl<W: Write>(
    bytes: &[u8],
    encoding: ObjectStreamEncoding,
    hashes: Option<&LumberyardHashes>,
    writer: &mut W,
) -> Result<(), ObjectStreamError> {
    let Some((&tag, _)) = bytes.split_first() else {
        return Err(ObjectStreamError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "empty ObjectStream payload",
        )));
    };

    match (StreamTag::from_byte(tag), encoding) {
        (Some(StreamTag::BINARY), ObjectStreamEncoding::Binary) => {
            validate_binary(&mut Cursor::new(bytes), hashes)?;
            writer.write_all(bytes)?;
            Ok(())
        }
        (Some(StreamTag::BINARY), ObjectStreamEncoding::Xml) => {
            write_binary_xml(&mut Cursor::new(bytes), hashes, writer)
        }
        (Some(StreamTag::BINARY), ObjectStreamEncoding::Json) => {
            write_binary_json(&mut Cursor::new(bytes), hashes, writer)
        }
        (Some(StreamTag::XML | StreamTag::JSON), encoding) => {
            let stream = deserialize::from_bytes(bytes, hashes)?;
            write_as(&stream, encoding, writer)?;
            Ok(())
        }
        _ => Err(ObjectStreamError::InvalidStreamTag(tag)),
    }
}

/// Write the binary `ObjectStream` form to `writer`.
///
/// # Errors
///
/// Returns any error [`to_bytes`] returns, or [`ObjectStreamError::Io`] if
/// `writer` rejects the encoded bytes.
pub fn write_to<W: Write>(stream: &ObjectStream, writer: &mut W) -> Result<(), ObjectStreamError> {
    let bytes = to_bytes(stream)?;
    writer.write_all(&bytes)?;
    Ok(())
}

fn write_binary<W: Write>(
    stream: &ObjectStream,
    context: Option<&ObjectStreamReadContext>,
    writer: &mut W,
) -> Result<(), ObjectStreamError> {
    crate::validate_stream_version(stream.version)?;
    writer.write_all(&[StreamTag::BINARY.0])?;
    writer.write_all(&stream.version.to_be_bytes())?;
    let mut stack = Vec::new();
    for element in stream.elements.iter().rev() {
        stack.push(BinaryWriteFrame::Element(element));
    }

    while let Some(frame) = stack.pop() {
        match frame {
            BinaryWriteFrame::Element(element) => {
                write_element_header_and_data(element, stream.version, context, writer)?;
                stack.push(BinaryWriteFrame::EndOfList);
                for child in element.elements.iter().rev() {
                    stack.push(BinaryWriteFrame::Element(child));
                }
            }
            BinaryWriteFrame::EndOfList => writer.write_all(&[0])?,
        }
    }
    writer.write_all(&[0])?;
    Ok(())
}

/// Convenience: encode an `ObjectStream` into binary `ObjectStream` bytes.
///
/// # Errors
///
/// Returns [`ObjectStreamError::UnsupportedVersion`] for a stream version
/// outside 0..=3, [`ObjectStreamError::FieldCrcMismatch`] when an element
/// stores a `name_crc` that disagrees with its field name,
/// [`ObjectStreamError::ElementVersionOverflow`] for an element version above
/// 255, [`ObjectStreamError::MissingV2Specialization`] and
/// [`ObjectStreamError::UnexpectedSpecialization`] for specialization UUIDs
/// that do not match the stream version,
/// [`ObjectStreamError::PayloadTooLarge`] for a payload wider than `u32`, and
/// [`ObjectStreamError::Io`] from the in-memory writer.
#[inline]
pub fn to_bytes(stream: &ObjectStream) -> Result<Vec<u8>, ObjectStreamError> {
    let mut bytes = Vec::new();
    write_binary(stream, None, &mut bytes)?;
    Ok(bytes)
}

#[derive(Debug, Clone, Copy)]
enum BinaryWriteFrame<'a> {
    Element(&'a Element),
    EndOfList,
}

fn write_element_header_and_data<W: Write>(
    element: &Element,
    stream_version: u32,
    context: Option<&ObjectStreamReadContext>,
    writer: &mut W,
) -> Result<(), ObjectStreamError> {
    let data = binary_payload(element, context)?;
    let name_crc = canonical_name_crc(element)?;
    let (flags, extra_width) = canonical_binary_flags(element, name_crc, data.as_deref())?;
    writer.write_all(&[flags])?;
    if let Some(crc) = name_crc {
        writer.write_all(&crc.to_be_bytes())?;
    }
    if let Some(version) = element.version {
        let version = u8::try_from(version)
            .map_err(|_| ObjectStreamError::ElementVersionOverflow { version })?;
        writer.write_all(&[version])?;
    }
    writer.write_all(&element.id.as_u128().to_be_bytes())?;

    match (stream_version, element.specialization) {
        (2, Some(specialized)) => writer.write_all(specialized.as_bytes())?,
        (2, None) => {
            return Err(ObjectStreamError::MissingV2Specialization {
                type_id: element.id,
            });
        }
        (_, Some(_)) => {
            return Err(ObjectStreamError::UnexpectedSpecialization {
                stream_version,
                type_id: element.id,
            });
        }
        (_, None) => {}
    }
    if let (Some(width), Some(data)) = (extra_width, data.as_deref()) {
        match width {
            1 => writer.write_all(&[u8::try_from(data.len()).expect("width selected")])?,
            2 => writer.write_all(
                &u16::try_from(data.len())
                    .expect("width selected")
                    .to_be_bytes(),
            )?,
            4 => writer.write_all(
                &u32::try_from(data.len())
                    .expect("width selected")
                    .to_be_bytes(),
            )?,
            _ => unreachable!("canonical width is 1, 2, or 4"),
        }
    }

    if let Some(data) = &data {
        writer.write_all(data)?;
    }
    Ok(())
}

fn canonical_name_crc(element: &Element) -> Result<Option<u32>, ObjectStreamError> {
    let computed = element
        .field
        .as_ref()
        .map(|field| crate::field_name_crc(field));
    if let (Some(stored), Some(computed)) = (element.name_crc, computed)
        && stored != computed
    {
        return Err(ObjectStreamError::FieldCrcMismatch {
            field: element
                .field
                .as_ref()
                .expect("computed from field")
                .to_string(),
            stored,
            computed,
        });
    }
    Ok(element.name_crc.or(computed))
}

fn canonical_binary_flags(
    element: &Element,
    name_crc: Option<u32>,
    data: Option<&[u8]>,
) -> Result<(u8, Option<u8>), ObjectStreamError> {
    let mut flags = ST_BINARYFLAG_ELEMENT_HEADER;
    if name_crc.is_some() {
        flags |= ST_BINARYFLAG_HAS_NAME;
    }
    if element.version.is_some() {
        flags |= ST_BINARYFLAG_HAS_VERSION;
    }
    let Some(data) = data else {
        return Ok((flags, None));
    };
    flags |= ST_BINARYFLAG_HAS_VALUE;
    if data.len() <= 7 {
        flags |= u8::try_from(data.len()).expect("inline size is at most seven");
        return Ok((flags, None));
    }
    let width = match data.len() {
        size if u8::try_from(size).is_ok() => 1,
        size if u16::try_from(size).is_ok() => 2,
        size if u32::try_from(size).is_ok() => 4,
        size => {
            return Err(ObjectStreamError::PayloadTooLarge {
                type_id: element.id,
                size,
            });
        }
    };
    flags |= ST_BINARYFLAG_EXTRA_SIZE_FIELD | width;
    Ok((flags, Some(width)))
}

fn binary_payload(
    element: &Element,
    context: Option<&ObjectStreamReadContext>,
) -> Result<Option<Vec<u8>>, ObjectStreamError> {
    validate_context_element(element, context)?;
    let Some(data) = element.data.as_deref() else {
        return Ok(None);
    };
    if element.payload_encoding == PayloadEncoding::BinaryBigEndian {
        return Ok(Some(data.to_vec()));
    }
    let type_id =
        element
            .resolved_type_id()
            .copied()
            .ok_or(ObjectStreamError::UnresolvedElementType {
                type_id: element.id,
            })?;
    let class = element.resolved_class();
    if let (Some(context), Some(class)) = (context, class) {
        context.validate_class_version(class, element.version.unwrap_or(0))?;
        if let Some(codec) = context.codec(class) {
            let bytes = match element.payload_encoding {
                PayloadEncoding::BinaryNativeEndian => codec
                    .to_big_endian(
                        type_id,
                        data,
                        PayloadEncoding::BinaryNativeEndian,
                        element.version.unwrap_or(0),
                    )
                    .map_err(|source| ObjectStreamError::ValueConversion { type_id, source })?,
                PayloadEncoding::Text => {
                    let text = std::str::from_utf8(data)?;
                    let payload = codec
                        .text_to_data(
                            type_id,
                            text,
                            element.version.unwrap_or(0),
                            crate::codec::TextEncoding::Xml,
                        )
                        .map_err(|source| ObjectStreamError::ValueConversion { type_id, source })?;
                    codec
                        .to_big_endian(
                            type_id,
                            &payload.bytes,
                            payload.encoding,
                            element.version.unwrap_or(0),
                        )
                        .map_err(|source| ObjectStreamError::ValueConversion { type_id, source })?
                }
                PayloadEncoding::BinaryBigEndian => unreachable!("handled above"),
            };
            return Ok(Some(bytes));
        }
    }
    let Some(serializer) = element.builtin_serializer() else {
        return Err(ObjectStreamError::MissingSerializer { type_id });
    };
    let codec = serializer.codec();
    let bytes = match element.payload_encoding {
        PayloadEncoding::BinaryNativeEndian => codec
            .to_big_endian(
                type_id,
                data,
                PayloadEncoding::BinaryNativeEndian,
                element.version.unwrap_or(0),
            )
            .map_err(|source| ObjectStreamError::ValueConversion { type_id, source })?,
        PayloadEncoding::Text => {
            let text = std::str::from_utf8(data)?;
            let payload = codec
                .text_to_data(
                    type_id,
                    text,
                    element.version.unwrap_or(0),
                    crate::codec::TextEncoding::Xml,
                )
                .map_err(|source| ObjectStreamError::ValueConversion { type_id, source })?;
            codec
                .to_big_endian(
                    type_id,
                    &payload.bytes,
                    payload.encoding,
                    element.version.unwrap_or(0),
                )
                .map_err(|source| ObjectStreamError::ValueConversion { type_id, source })?
        }
        PayloadEncoding::BinaryBigEndian => unreachable!("handled above"),
    };
    Ok(Some(bytes))
}

fn text_payload(
    element: &Element,
    context: Option<&ObjectStreamReadContext>,
    encoding: crate::codec::TextEncoding,
) -> Result<Option<String>, ObjectStreamError> {
    validate_context_element(element, context)?;
    let Some(data) = element.data.as_deref() else {
        return Ok(None);
    };
    if element.payload_encoding == PayloadEncoding::Text {
        // A ranged_int specialization stores a fixed-width unsigned integer with
        // no text serializer, so when its payload is captured Text-classified
        // (the raw integer bytes sat in an XML `value=` attribute) the generic
        // passthrough would copy binary bytes into the output verbatim. The
        // captured serializer descriptor is the proof of width and signedness
        // (the same principle `value.rs::read_unsigned_scalar_with_kinds` uses
        // for the binary-payload case), so decode it through the same
        // big-endian codec path used for binary RangedUnsigned payloads.
        // Any other Text payload is genuine serializer text and passes through
        // — including an element with no captured descriptor at all.
        if let Some(text) = ranged_int_text_payload(element, data, encoding)? {
            return Ok(Some(text));
        }
        return Ok(Some(std::str::from_utf8(data)?.to_owned()));
    }
    let type_id =
        element
            .resolved_type_id()
            .copied()
            .ok_or(ObjectStreamError::UnresolvedElementType {
                type_id: element.id,
            })?;
    if let (Some(context), Some(class)) = (context, element.resolved_class()) {
        context.validate_class_version(class, element.version.unwrap_or(0))?;
        if let Some(codec) = context.codec(class) {
            return codec
                .data_to_text(
                    type_id,
                    data,
                    element.payload_encoding,
                    element.version.unwrap_or(0),
                    encoding,
                )
                .map(Some)
                .map_err(|source| ObjectStreamError::ValueConversion { type_id, source });
        }
    }
    let Some(serializer) = element.builtin_serializer() else {
        return Err(ObjectStreamError::MissingSerializer { type_id });
    };
    serializer
        .codec()
        .data_to_text(
            type_id,
            data,
            element.payload_encoding,
            element.version.unwrap_or(0),
            encoding,
        )
        .map(Some)
        .map_err(|source| ObjectStreamError::ValueConversion { type_id, source })
}

/// Decodes a Text-classified `AZStd::ranged_int` payload to its decimal
/// integer text.
///
/// `AZStd::ranged_int<Unsigned, Min, Max>` folds Min/Max into its reflected
/// UUID, so every instantiation gets its own specialized id that no fixed
/// enumeration should be trusted to stay in sync with — the captured
/// serializer descriptor is the proof of width and signedness (the same
/// principle `value.rs::read_unsigned_scalar_with_kinds` uses for the
/// binary-payload case, and how a project schema codec resolves the
/// same family). Returns `Ok(None)` when the element's captured descriptor
/// isn't `RangedUnsigned` — either genuine non-ranged_int Text or an
/// unresolved element with no descriptor at all — so the caller falls
/// through to the generic Text passthrough. When it is, the bytes are
/// decoded through codec.rs's `RangedUnsigned` family as `BinaryBigEndian`,
/// and its length guard stops-and-reports (propagated error) on any width
/// mismatch; no clamping.
fn ranged_int_text_payload(
    element: &Element,
    data: &[u8],
    encoding: crate::codec::TextEncoding,
) -> Result<Option<String>, ObjectStreamError> {
    let Some(descriptor) = element.builtin_serializer() else {
        return Ok(None);
    };
    let crate::codec::BuiltinSerializerKind::RangedUnsigned { bytes } = descriptor.kind else {
        return Ok(None);
    };
    crate::codec::BuiltinValueCodec::new(
        crate::codec::BuiltinSerializerKind::RangedUnsigned { bytes },
        0,
    )
    .data_to_text(
        element.id,
        data,
        PayloadEncoding::BinaryBigEndian,
        0,
        encoding,
    )
    .map(Some)
    .map_err(|source| ObjectStreamError::ValueConversion {
        type_id: element.id,
        source,
    })
}

fn validate_context_element(
    element: &Element,
    context: Option<&ObjectStreamReadContext>,
) -> Result<(), ObjectStreamError> {
    let Some(context) = context else {
        return Ok(());
    };
    let type_id =
        element
            .resolved_type_id()
            .copied()
            .ok_or(ObjectStreamError::UnresolvedElementType {
                type_id: element.id,
            })?;
    if type_id == crate::types::DATA_OVERLAY_INFO {
        return Err(ObjectStreamError::InvalidDataOverlay(
            "unmaterialized DataOverlayInfo reached writer preflight; parse or resolve it with the provider-bearing context first".into(),
        ));
    }
    let class = element
        .resolved_class()
        .filter(|class| context.class(*class).is_some())
        .ok_or(ObjectStreamError::UnresolvedElementType {
            type_id: element.id,
        })?;
    match context.validate_class_version(class, element.version.unwrap_or(0))? {
        crate::context::VersionConversionState::RegisteredConverter { from, to }
        | crate::context::VersionConversionState::StrictDefaultStructural { from, to } => {
            return Err(ObjectStreamError::UnsupportedVersionConversion {
                type_id,
                element_version: from,
                current_version: to,
            });
        }
        crate::context::VersionConversionState::Current
        | crate::context::VersionConversionState::SerializerCompatibleOld { .. } => {}
        crate::context::VersionConversionState::DeprecatedDiscard => {
            return Err(ObjectStreamError::DeprecatedClass { type_id });
        }
    }
    context.validate_payload_support(class, element.data.is_some())?;
    if element.data.is_some() && context.codec(class).is_none() {
        return Err(ObjectStreamError::MissingSerializer { type_id });
    }
    Ok(())
}

pub(crate) fn validate_context_graph(
    stream: &ObjectStream,
    context: &ObjectStreamReadContext,
) -> Result<(), ObjectStreamError> {
    let mut stack = stream
        .elements
        .iter()
        .rev()
        .map(|element| (element, None))
        .collect::<Vec<_>>();
    while let Some((element, parent)) = stack.pop() {
        validate_context_element(element, Some(context))?;
        let class = element
            .resolved_class()
            .ok_or(ObjectStreamError::UnresolvedElementType {
                type_id: element.id,
            })?;
        context.validate_container_cardinality(class, element.elements.len())?;
        let resolved = crate::context::ResolvedType {
            type_id: element.resolved_type_id().copied(),
            class: element.resolved_class(),
            builtin_serializer: element.builtin_serializer(),
            is_container: element.is_resolved_container(),
            container_shape: element.container_shape(),
            ambiguous_generic: false,
        };
        if let Some(parent) = parent {
            context.validate_finalized_reachable_child(
                parent,
                element.name_crc,
                element.id,
                resolved,
            )?;
        }
        for child in element.elements.iter().rev() {
            stack.push((child, element.resolved_class()));
        }
    }
    Ok(())
}

fn write_xml<W: Write>(
    stream: &ObjectStream,
    context: Option<&ObjectStreamReadContext>,
    writer: &mut W,
) -> Result<(), ObjectStreamError> {
    let mut xml = quick_xml::Writer::new_with_indent(writer, b'\t', 2);
    write_xml_root_start(&mut xml, stream.version)?;

    let mut stack = Vec::new();
    for element in stream.elements.iter().rev() {
        stack.push(XmlDomFrame::Element(element));
    }

    while let Some(frame) = stack.pop() {
        match frame {
            XmlDomFrame::Element(element) => {
                let attrs = XmlClassAttrs::from_element(element, context)?;
                if element.elements.is_empty() {
                    write_xml_class(&mut xml, &attrs, true, true)?;
                } else {
                    write_xml_class(&mut xml, &attrs, false, false)?;
                    stack.push(XmlDomFrame::EndOfClass);
                    for child in element.elements.iter().rev() {
                        stack.push(XmlDomFrame::Element(child));
                    }
                }
            }
            XmlDomFrame::EndOfClass => write_xml_class_end(&mut xml)?,
        }
    }

    write_xml_root_end(&mut xml)?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum XmlDomFrame<'a> {
    Element(&'a Element),
    EndOfClass,
}

fn write_binary_xml<R: Read, W: Write>(
    reader: &mut R,
    hashes: Option<&LumberyardHashes>,
    writer: &mut W,
) -> Result<(), ObjectStreamError> {
    let version = read_stream_header(reader)?;
    let mut xml = quick_xml::Writer::new_with_indent(writer, b'\t', 2);
    write_xml_root_start(&mut xml, version)?;

    let mut stack = Vec::new();
    let mut data = Vec::new();

    loop {
        match read_element_header(reader, version, hashes)? {
            BinaryElement::Header(header) => {
                if let Some(parent) = stack.last_mut() {
                    write_pending_xml_class(&mut xml, parent, false)?;
                }

                data.clear();
                let data_size = header.data_size.unwrap_or(0);
                data.resize(data_size, 0);
                if data_size > 0 {
                    reader.read_exact(&mut data)?;
                }

                stack.push(PendingXmlClass::from_binary_header(&header, &data)?);
            }
            BinaryElement::EndOfList => {
                let Some(mut class) = stack.pop() else {
                    break;
                };
                write_pending_xml_class(&mut xml, &mut class, true)?;
                if class.start_written {
                    write_xml_class_end(&mut xml)?;
                }
            }
        }
    }

    ensure_reader_exhausted(reader)?;
    write_xml_root_end(&mut xml)?;
    Ok(())
}

fn write_binary_json<R: Read, W: Write>(
    reader: &mut R,
    hashes: Option<&LumberyardHashes>,
    writer: &mut W,
) -> Result<(), ObjectStreamError> {
    let version = read_stream_header(reader)?;
    writer.write_all(br"{")?;
    writer.write_all(b"\n  ")?;
    write_json_name(writer, "name")?;
    writer.write_all(b": ")?;
    write_json_serialized(writer, "ObjectStream")?;
    writer.write_all(b",\n  ")?;
    write_json_name(writer, "version")?;
    write!(writer, ": {version}")?;
    writer.write_all(b",\n  ")?;
    write_json_name(writer, "Objects")?;
    writer.write_all(b": [")?;

    let mut stack = Vec::new();
    let mut data = Vec::new();
    let mut root_children = 0usize;

    loop {
        match read_element_header(reader, version, hashes)? {
            BinaryElement::Header(header) => {
                let indent = if let Some(parent) = stack.last_mut() {
                    prepare_json_child_slot(writer, parent)?;
                    parent.indent + 2
                } else {
                    if root_children == 0 {
                        writer.write_all(b"\n")?;
                    } else {
                        writer.write_all(b",\n")?;
                    }
                    root_children += 1;
                    2
                };

                data.clear();
                let data_size = header.data_size.unwrap_or(0);
                data.resize(data_size, 0);
                if data_size > 0 {
                    reader.read_exact(&mut data)?;
                }

                stack.push(PendingJsonClass::from_binary_header(
                    &header, &data, indent,
                )?);
            }
            BinaryElement::EndOfList => {
                let Some(class) = stack.pop() else {
                    break;
                };

                if class.objects_open {
                    writer.write_all(b"\n")?;
                    write_indent(writer, class.indent + 1)?;
                    writer.write_all(b"]\n")?;
                    write_indent(writer, class.indent)?;
                    writer.write_all(b"}")?;
                } else {
                    write_json_class(writer, &class.attrs, class.indent, true)?;
                }
            }
        }
    }

    ensure_reader_exhausted(reader)?;
    if root_children > 0 {
        writer.write_all(b"\n  ]\n}")?;
    } else {
        writer.write_all(b"]\n}")?;
    }
    Ok(())
}

fn validate_binary<R: Read>(
    reader: &mut R,
    hashes: Option<&LumberyardHashes>,
) -> Result<(), ObjectStreamError> {
    let version = read_stream_header(reader)?;
    let mut open_classes = 0usize;
    let mut scratch = [0u8; 8192];

    loop {
        match read_element_header(reader, version, hashes)? {
            BinaryElement::Header(header) => {
                if let Some(data_size) = header.data_size {
                    read_exact_discard(reader, data_size, &mut scratch)?;
                }
                open_classes += 1;
            }
            BinaryElement::EndOfList => {
                let Some(next) = open_classes.checked_sub(1) else {
                    break;
                };
                open_classes = next;
            }
        }
    }
    ensure_reader_exhausted(reader)?;
    Ok(())
}

fn read_exact_discard<R: Read>(
    reader: &mut R,
    mut remaining: usize,
    scratch: &mut [u8],
) -> Result<(), ObjectStreamError> {
    while remaining > 0 {
        let chunk = remaining.min(scratch.len());
        reader.read_exact(&mut scratch[..chunk])?;
        remaining -= chunk;
    }
    Ok(())
}

#[derive(Debug)]
struct PendingXmlClass {
    attrs: XmlClassAttrs,
    start_written: bool,
}

impl PendingXmlClass {
    fn from_binary_header(
        header: &crate::binary::BinaryElementHeader<'_>,
        data: &[u8],
    ) -> Result<Self, ObjectStreamError> {
        Ok(Self {
            attrs: XmlClassAttrs::from_binary_header(header, data)?,
            start_written: false,
        })
    }
}

fn write_pending_xml_class<W: Write>(
    xml: &mut quick_xml::Writer<W>,
    class: &mut PendingXmlClass,
    leaf: bool,
) -> io::Result<()> {
    if class.start_written {
        return Ok(());
    }
    write_xml_class(xml, &class.attrs, leaf, leaf)?;
    class.start_written = !leaf;
    Ok(())
}

#[derive(Debug)]
struct XmlClassAttrs {
    name: String,
    field: Option<String>,
    field_crc: Option<u32>,
    specialization: Option<String>,
    value: Option<String>,
    empty_leaf_value: Option<String>,
    version: Option<String>,
    type_id: String,
}

#[derive(Debug)]
struct PendingJsonClass {
    attrs: JsonClassAttrs,
    indent: usize,
    objects_open: bool,
    children: usize,
}

impl PendingJsonClass {
    fn from_binary_header(
        header: &crate::binary::BinaryElementHeader<'_>,
        data: &[u8],
        indent: usize,
    ) -> Result<Self, ObjectStreamError> {
        Ok(Self {
            attrs: JsonClassAttrs::from_binary_header(header, data)?,
            indent,
            objects_open: false,
            children: 0,
        })
    }
}

fn prepare_json_child_slot<W: Write>(
    writer: &mut W,
    parent: &mut PendingJsonClass,
) -> io::Result<()> {
    if !parent.objects_open {
        write_json_class_fields(writer, &parent.attrs, parent.indent, false)?;
        writer.write_all(b",\n")?;
        write_indent(writer, parent.indent + 1)?;
        write_json_name(writer, "Objects")?;
        writer.write_all(b": [\n")?;
        parent.objects_open = true;
    } else if parent.children > 0 {
        writer.write_all(b",\n")?;
    }
    parent.children += 1;
    Ok(())
}

#[derive(Debug)]
struct JsonClassAttrs {
    field: Option<String>,
    field_crc: Option<u32>,
    type_id: String,
    name: String,
    specialization: Option<String>,
    value: Option<Value>,
    empty_leaf_value: Option<Value>,
    version: Option<u32>,
    data_present: bool,
}

impl JsonClassAttrs {
    fn from_element(
        element: &Element,
        context: Option<&ObjectStreamReadContext>,
    ) -> Result<Self, ObjectStreamError> {
        let value = text_payload(element, context, crate::codec::TextEncoding::Json)?;
        let (field, field_crc) = text_field(element, context)?;
        Ok(Self {
            field,
            field_crc,
            type_id: uuid_xml_attr(&element.id),
            name: text_type_name(element, context)?,
            specialization: element.specialization.map(|uuid| uuid_xml_attr(&uuid)),
            value: value.map(Value::String),
            empty_leaf_value: None,
            version: element.version,
            data_present: element.data.is_some(),
        })
    }

    fn from_binary_header(
        header: &crate::binary::BinaryElementHeader<'_>,
        data: &[u8],
    ) -> Result<Self, ObjectStreamError> {
        let data = header.data_size.map(|_| data);
        if data.is_some() {
            return Err(ObjectStreamError::MissingSerializer { type_id: header.id });
        }
        let (field, field_crc) = binary_text_field(header)?;
        Ok(Self {
            field,
            field_crc,
            type_id: uuid_xml_attr(&header.id),
            name: header.name.map_or_else(String::new, ToString::to_string),
            specialization: header.specialization.map(|uuid| uuid_xml_attr(&uuid)),
            value: None,
            empty_leaf_value: None,
            version: header.version,
            data_present: data.is_some(),
        })
    }
}

impl XmlClassAttrs {
    fn from_element(
        element: &Element,
        context: Option<&ObjectStreamReadContext>,
    ) -> Result<Self, ObjectStreamError> {
        let value = text_payload(element, context, crate::codec::TextEncoding::Xml)?;
        let (field, field_crc) = text_field(element, context)?;
        Ok(Self {
            name: text_type_name(element, context)?,
            field,
            field_crc,
            specialization: element.specialization.map(|uuid| uuid_xml_attr(&uuid)),
            value,
            empty_leaf_value: None,
            version: element.version.map(|version| version.to_string()),
            type_id: uuid_xml_attr(&element.id),
        })
    }

    fn from_binary_header(
        header: &crate::binary::BinaryElementHeader<'_>,
        data: &[u8],
    ) -> Result<Self, ObjectStreamError> {
        let data = header.data_size.map(|_| data);
        if data.is_some() {
            return Err(ObjectStreamError::MissingSerializer { type_id: header.id });
        }
        let (field, field_crc) = binary_text_field(header)?;
        Ok(Self {
            name: header.name.map_or_else(String::new, ToString::to_string),
            field,
            field_crc,
            specialization: header.specialization.map(|uuid| uuid_xml_attr(&uuid)),
            value: None,
            empty_leaf_value: None,
            version: header.version.map(|version| version.to_string()),
            type_id: uuid_xml_attr(&header.id),
        })
    }
}

fn uuid_xml_attr(uuid: &uuid::Uuid) -> String {
    uuid.as_braced().to_string().to_uppercase()
}

/// Resolves an element's field slot to `(field_name, unresolved_field_crc)`.
///
/// Exactly one component is `Some`: a resolved slot yields the name, an
/// unresolved-but-present `name_crc` yields the raw crc so the writer can emit
/// `field_crc="0x…"`. A field-less slot (`name_crc == None`) yields `(None, None)`
/// and stays field-less on output — that None/Some distinction is load-bearing.
fn text_field(
    element: &Element,
    context: Option<&ObjectStreamReadContext>,
) -> Result<(Option<String>, Option<u32>), ObjectStreamError> {
    if let Some(field) = &element.field {
        canonical_name_crc(element)?;
        return Ok((Some(field.to_string()), None));
    }
    let Some(crc) = element.name_crc else {
        return Ok((None, None));
    };
    Ok(context
        .and_then(|context| context.names().field_name(crc))
        .map_or((None, Some(crc)), |name| (Some(name.to_string()), None)))
}

fn binary_text_field(
    header: &crate::binary::BinaryElementHeader<'_>,
) -> Result<(Option<String>, Option<u32>), ObjectStreamError> {
    match (header.name_crc, header.field) {
        (None, _) => Ok((None, None)),
        (Some(crc), None) => Ok((None, Some(crc))),
        (Some(stored), Some(field)) => {
            let computed = crate::field_name_crc(field);
            if stored != computed {
                return Err(ObjectStreamError::FieldCrcMismatch {
                    field: field.to_string(),
                    stored,
                    computed,
                });
            }
            Ok((Some(field.to_string()), None))
        }
    }
}

fn text_type_name(
    element: &Element,
    context: Option<&ObjectStreamReadContext>,
) -> Result<String, ObjectStreamError> {
    let Some(context) = context else {
        return Ok(element.name.to_string());
    };
    let type_id =
        element
            .resolved_type_id()
            .copied()
            .ok_or(ObjectStreamError::UnresolvedElementType {
                type_id: element.id,
            })?;
    let class = element
        .resolved_class()
        .and_then(|class| context.class(class))
        .ok_or(ObjectStreamError::UnresolvedElementType {
            type_id: element.id,
        })?;
    class
        .name
        .as_ref()
        .or_else(|| context.names().type_name(&class.type_id))
        .or_else(|| context.names().type_name(&type_id))
        .map(ToString::to_string)
        .ok_or(ObjectStreamError::MissingTypeName { type_id })
}

fn write_xml_root_start<W: Write>(xml: &mut quick_xml::Writer<W>, version: u32) -> io::Result<()> {
    let mut root = BytesStart::new(XML_OBJECT_STREAM);
    let version = version.to_string();
    root.push_attribute(("version", version.as_str()));
    xml.write_event(Event::Start(root))
        .map_err(io::Error::other)
}

fn write_xml_root_end<W: Write>(xml: &mut quick_xml::Writer<W>) -> io::Result<()> {
    xml.write_event(Event::End(BytesEnd::new(XML_OBJECT_STREAM)))
        .map_err(io::Error::other)
}

fn write_xml_class<W: Write>(
    xml: &mut quick_xml::Writer<W>,
    attrs: &XmlClassAttrs,
    leaf: bool,
    empty: bool,
) -> io::Result<()> {
    let mut class = BytesStart::new(XML_CLASS);
    class.push_attribute(("name", attrs.name.as_str()));
    if let Some(field) = &attrs.field {
        class.push_attribute(("field", field.as_str()));
    }
    let field_crc_attr = attrs.field_crc.map(|crc| format!("{crc:#010x}"));
    if let Some(field_crc) = &field_crc_attr {
        class.push_attribute(("field_crc", field_crc.as_str()));
    }
    if let Some(specialization) = &attrs.specialization {
        class.push_attribute(("specializationTypeId", specialization.as_str()));
    }
    let value = attrs.value.as_ref().or(if leaf {
        attrs.empty_leaf_value.as_ref()
    } else {
        None
    });
    if let Some(value) = value {
        class.push_attribute(("value", value.as_str()));
    }
    if let Some(version) = &attrs.version {
        class.push_attribute(("version", version.as_str()));
    }
    class.push_attribute(("type", attrs.type_id.as_str()));
    let event = if empty {
        Event::Empty(class)
    } else {
        Event::Start(class)
    };
    xml.write_event(event).map_err(io::Error::other)
}

fn write_xml_class_end<W: Write>(xml: &mut quick_xml::Writer<W>) -> io::Result<()> {
    xml.write_event(Event::End(BytesEnd::new(XML_CLASS)))
        .map_err(io::Error::other)
}

fn write_json<W: Write>(
    stream: &ObjectStream,
    context: Option<&ObjectStreamReadContext>,
    writer: &mut W,
) -> Result<(), ObjectStreamError> {
    writer.write_all(br"{")?;
    writer.write_all(b"\n  ")?;
    write_json_name(writer, "name")?;
    writer.write_all(b": ")?;
    write_json_serialized(writer, "ObjectStream")?;
    writer.write_all(b",\n  ")?;
    write_json_name(writer, "version")?;
    write!(writer, ": {}", stream.version)?;
    writer.write_all(b",\n  ")?;
    write_json_name(writer, "Objects")?;
    writer.write_all(b": ")?;
    write_json_array(writer, &stream.elements, 2, context)?;
    writer.write_all(b"\n}")?;
    Ok(())
}

fn write_json_array<W: Write>(
    writer: &mut W,
    elements: &[Element],
    element_indent: usize,
    context: Option<&ObjectStreamReadContext>,
) -> Result<(), ObjectStreamError> {
    if elements.is_empty() {
        writer.write_all(b"[]")?;
        return Ok(());
    }

    writer.write_all(b"[\n")?;
    let mut stack = vec![JsonFrame::Array {
        elements,
        index: 0,
        element_indent,
    }];

    while let Some(frame) = stack.pop() {
        match frame {
            JsonFrame::Array {
                elements,
                index,
                element_indent,
            } => {
                if index >= elements.len() {
                    writer.write_all(b"\n")?;
                    write_indent(writer, element_indent - 1)?;
                    writer.write_all(b"]")?;
                    continue;
                }

                if index > 0 {
                    writer.write_all(b",\n")?;
                }

                let element = &elements[index];
                let attrs = JsonClassAttrs::from_element(element, context)?;
                write_json_class_fields(
                    writer,
                    &attrs,
                    element_indent,
                    element.elements.is_empty(),
                )?;

                stack.push(JsonFrame::Array {
                    elements,
                    index: index + 1,
                    element_indent,
                });

                if element.elements.is_empty() {
                    writer.write_all(b"\n")?;
                    write_indent(writer, element_indent)?;
                    writer.write_all(b"}")?;
                } else {
                    writer.write_all(b",\n")?;
                    write_indent(writer, element_indent + 1)?;
                    write_json_name(writer, "Objects")?;
                    writer.write_all(b": [\n")?;
                    stack.push(JsonFrame::EndElement { element_indent });
                    stack.push(JsonFrame::Array {
                        elements: &element.elements,
                        index: 0,
                        element_indent: element_indent + 2,
                    });
                }
            }
            JsonFrame::EndElement { element_indent } => {
                writer.write_all(b"\n")?;
                write_indent(writer, element_indent)?;
                writer.write_all(b"}")?;
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum JsonFrame<'a> {
    Array {
        elements: &'a [Element],
        index: usize,
        element_indent: usize,
    },
    EndElement {
        element_indent: usize,
    },
}

fn write_json_class<W: Write>(
    writer: &mut W,
    attrs: &JsonClassAttrs,
    indent: usize,
    leaf: bool,
) -> io::Result<()> {
    write_json_class_fields(writer, attrs, indent, leaf)?;
    writer.write_all(b"\n")?;
    write_indent(writer, indent)?;
    writer.write_all(b"}")
}

fn write_json_class_fields<W: Write>(
    writer: &mut W,
    attrs: &JsonClassAttrs,
    indent: usize,
    leaf: bool,
) -> io::Result<()> {
    write_indent(writer, indent)?;
    writer.write_all(b"{\n")?;

    let mut first = true;
    if let Some(field) = &attrs.field {
        write_json_member(writer, indent + 1, &mut first, "field", field.as_str())?;
    }
    if let Some(field_crc) = attrs.field_crc {
        let field_crc = format!("{field_crc:#010x}");
        write_json_member(
            writer,
            indent + 1,
            &mut first,
            "field_crc",
            field_crc.as_str(),
        )?;
    }
    write_json_member(
        writer,
        indent + 1,
        &mut first,
        "typeId",
        attrs.type_id.as_str(),
    )?;
    write_json_member(
        writer,
        indent + 1,
        &mut first,
        "typeName",
        attrs.name.as_str(),
    )?;
    if let Some(specialization) = &attrs.specialization {
        write_json_member(
            writer,
            indent + 1,
            &mut first,
            "specializationTypeId",
            specialization.as_str(),
        )?;
    }
    let value = attrs.value.as_ref().or(if leaf {
        attrs.empty_leaf_value.as_ref()
    } else {
        None
    });
    if let Some(value) = value {
        write_json_member_raw(writer, indent + 1, &mut first, "value", |writer| {
            write_json_serialized(writer, value)
        })?;
    }
    if let Some(version) = attrs.version {
        write_json_member_raw(writer, indent + 1, &mut first, "version", |writer| {
            write!(writer, "{version}")
        })?;
    }
    if leaf && !attrs.data_present {
        write_json_member_raw(writer, indent + 1, &mut first, "Objects", |writer| {
            writer.write_all(b"[]")
        })?;
    }
    Ok(())
}

fn write_json_member<W: Write, V: serde::Serialize>(
    writer: &mut W,
    indent: usize,
    first: &mut bool,
    name: &str,
    value: V,
) -> io::Result<()> {
    write_json_member_raw(writer, indent, first, name, |writer| {
        write_json_serialized(writer, &value)
    })
}

fn write_json_member_raw<W: Write>(
    writer: &mut W,
    indent: usize,
    first: &mut bool,
    name: &str,
    write_value: impl FnOnce(&mut W) -> io::Result<()>,
) -> io::Result<()> {
    if *first {
        *first = false;
    } else {
        writer.write_all(b",\n")?;
    }
    write_indent(writer, indent)?;
    write_json_name(writer, name)?;
    writer.write_all(b": ")?;
    write_value(writer)
}

fn write_json_name<W: Write>(writer: &mut W, name: &str) -> io::Result<()> {
    write_json_serialized(writer, name)
}

fn write_json_serialized<W: Write, V: serde::Serialize + ?Sized>(
    writer: &mut W,
    value: &V,
) -> io::Result<()> {
    serde_json::to_writer(writer, value).map_err(io::Error::other)
}

fn write_indent<W: Write>(writer: &mut W, indent: usize) -> io::Result<()> {
    for _ in 0..indent {
        writer.write_all(b"  ")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::ST_BINARYFLAG_ELEMENT_HEADER;

    #[test]
    fn binary_writer_emits_canonical_v1_v2_v3_headers_and_end_markers() {
        let id = Uuid::from_u128(0x11223344_5566_7788_99aa_bbccddeeff00);
        let specialization = Uuid::from_u128(0xffeeddcc_bbaa_9988_7766_554433221100);
        for version in 1..=3 {
            let mut element = Element::new(id).with_field("value").with_data([1, 2, 3]);
            element.version = Some(7);
            if version == 2 {
                element.specialization = Some(specialization);
            }
            let stream = ObjectStream {
                version,
                elements: vec![element],
                ..ObjectStream::default()
            };

            let bytes = to_bytes(&stream).unwrap();
            let version_byte = u8::try_from(version).expect("loop range is 1..=3");
            let mut expected = vec![0, 0, 0, 0, version_byte, 0xdb];
            expected.extend_from_slice(&crate::field_name_crc("value").to_be_bytes());
            expected.push(7);
            expected.extend_from_slice(id.as_bytes());
            if version == 2 {
                expected.extend_from_slice(specialization.as_bytes());
            }
            expected.extend_from_slice(&[1, 2, 3, 0, 0]);
            assert_eq!(bytes, expected, "stream version {version}");
        }
    }

    #[test]
    fn binary_writer_distinguishes_empty_from_absent_value() {
        let empty = Element::new(Uuid::from_u128(1)).with_data([]);
        let absent = Element::new(Uuid::from_u128(2));
        let stream = ObjectStream {
            version: 3,
            elements: vec![empty, absent],
            ..ObjectStream::default()
        };
        let bytes = to_bytes(&stream).unwrap();

        assert_eq!(
            bytes[5],
            ST_BINARYFLAG_ELEMENT_HEADER | ST_BINARYFLAG_HAS_VALUE
        );
        assert_eq!(bytes[23], ST_BINARYFLAG_ELEMENT_HEADER);
    }

    #[test]
    fn json_serializer_values_are_always_strings() {
        let mut context = ObjectStreamReadContext::default();
        let class = context
            .insert_class(
                crate::types::INT,
                crate::context::ReflectedClass::new(crate::types::INT).with_name("int"),
            )
            .unwrap();
        context
            .insert_builtin_codec(class, crate::types::INT, 0)
            .unwrap();
        let mut element = Element::new(crate::types::INT).with_data(42_i32.to_be_bytes());
        context.resolve_element_tree(3, &mut element).unwrap();
        let stream = ObjectStream {
            version: 3,
            elements: vec![element],
            ..ObjectStream::default()
        };

        let bytes =
            to_encoding_bytes_with_context(&stream, ObjectStreamEncoding::Json, &context).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["Objects"][0]["value"], "42");
    }

    #[test]
    fn v2_xml_and_json_emit_specialization_identity() {
        let id = Uuid::from_u128(1);
        let specialization = Uuid::from_u128(2);
        let mut element = Element::new(id).with_field("entry");
        element.specialization = Some(specialization);
        let stream = ObjectStream {
            version: 2,
            elements: vec![element],
            ..ObjectStream::default()
        };

        let xml = String::from_utf8(to_encoding_bytes(&stream, ObjectStreamEncoding::Xml).unwrap())
            .unwrap();
        assert!(xml.contains("specializationTypeId=\"{00000000-0000-0000-0000-000000000002}\""));

        let json: serde_json::Value = serde_json::from_slice(
            &to_encoding_bytes(&stream, ObjectStreamEncoding::Json).unwrap(),
        )
        .unwrap();
        assert_eq!(
            json["Objects"][0]["specializationTypeId"],
            "{00000000-0000-0000-0000-000000000002}"
        );
    }

    #[test]
    fn text_output_emits_field_crc_when_field_name_unresolved() {
        let mut element = Element::new(Uuid::from_u128(1));
        element.name_crc = Some(0x1234_5678);
        let stream = ObjectStream {
            version: 3,
            elements: vec![element],
            ..ObjectStream::default()
        };

        let bytes = to_encoding_bytes(&stream, ObjectStreamEncoding::Xml)
            .expect("unresolved field crc now emits field_crc rather than erroring");
        let xml = String::from_utf8(bytes).expect("xml output is utf-8");
        assert!(
            xml.contains(r#"field_crc="0x12345678""#),
            "expected field_crc attribute in output, got: {xml}"
        );
    }

    #[test]
    fn text_payload_decodes_ranged_int_u64_as_decimal() {
        // AZStd::ranged_int<u64> (e.g. m_objectiveInstanceId) has no text
        // serializer, so its value lands in the XML attribute as an 8-byte
        // big-endian payload that the generic Text passthrough would copy
        // verbatim. Resolution is descriptor-driven, not UUID-driven (the
        // specialized UUID here is realistic corpus data but is not what the
        // decoder keys on); the captured `RangedUnsigned { bytes: 8 }`
        // descriptor is what proves the width. 258 = big-endian
        // [0,0,0,0,0,0,1,2]; a little-endian decode would produce a wildly
        // different number, so the assertion pins byte order to the codec's
        // big-endian path.
        let ranged_u64 = Uuid::from_u128(0xCDAD_EE50_C32A_5AC5_9422_C610_83EF_25ED);
        let mut element = Element::new(ranged_u64)
            .with_field("m_objectiveInstanceId")
            .with_data(258_u64.to_be_bytes())
            .with_builtin_serializer(crate::codec::BuiltinSerializerDescriptor::new(
                crate::codec::BuiltinSerializerKind::RangedUnsigned { bytes: 8 },
                0,
            ));
        element.payload_encoding = PayloadEncoding::Text;
        let stream = ObjectStream {
            version: 3,
            elements: vec![element],
            ..ObjectStream::default()
        };

        let bytes = to_encoding_bytes(&stream, ObjectStreamEncoding::Xml).unwrap();
        let xml = String::from_utf8(bytes).unwrap();
        assert!(
            xml.contains(r#"value="258""#),
            "expected big-endian decimal decode, got: {xml}"
        );
    }

    #[test]
    fn text_payload_ranged_int_wrong_width_stops_and_reports() {
        // A captured `RangedUnsigned { bytes: 8 }` descriptor demands 8
        // payload bytes; a mismatched width must error rather than truncate,
        // escape, or guess. Descriptor-driven resolution changes where the
        // width comes from, not this stop-and-report guarantee.
        let ranged_u64 = Uuid::from_u128(0xCDAD_EE50_C32A_5AC5_9422_C610_83EF_25ED);
        let mut element = Element::new(ranged_u64)
            .with_data([0u8; 4])
            .with_builtin_serializer(crate::codec::BuiltinSerializerDescriptor::new(
                crate::codec::BuiltinSerializerKind::RangedUnsigned { bytes: 8 },
                0,
            ));
        element.payload_encoding = PayloadEncoding::Text;
        let stream = ObjectStream {
            version: 3,
            elements: vec![element],
            ..ObjectStream::default()
        };

        assert!(matches!(
            to_encoding_bytes(&stream, ObjectStreamEncoding::Xml),
            Err(ObjectStreamError::ValueConversion { .. })
        ));
    }

    #[test]
    fn text_payload_passes_through_non_ranged_text_verbatim() {
        // An element with no captured builtin-serializer descriptor at all
        // (unresolved, or a type outside the ranged_int family) keeps the
        // generic Text passthrough untouched.
        let mut element = Element::new(Uuid::from_u128(1)).with_data(b"plaintext".as_slice());
        element.payload_encoding = PayloadEncoding::Text;
        let stream = ObjectStream {
            version: 3,
            elements: vec![element],
            ..ObjectStream::default()
        };

        let bytes = to_encoding_bytes(&stream, ObjectStreamEncoding::Xml).unwrap();
        let xml = String::from_utf8(bytes).unwrap();
        assert!(
            xml.contains(r#"value="plaintext""#),
            "expected verbatim passthrough, got: {xml}"
        );
    }

    #[test]
    fn infallible_binary_api_was_removed_for_invalid_v2_state() {
        let stream = ObjectStream {
            version: 2,
            elements: vec![Element::new(Uuid::from_u128(1))],
            ..ObjectStream::default()
        };

        assert!(matches!(
            stream.to_bytes(),
            Err(ObjectStreamError::MissingV2Specialization { .. })
        ));
    }

    #[test]
    fn direct_writer_preflights_before_touching_caller_output() {
        let stream = ObjectStream {
            version: 2,
            elements: vec![Element::new(Uuid::from_u128(1))],
            ..ObjectStream::default()
        };
        let mut output = b"unchanged".to_vec();

        assert!(matches!(
            write_as(&stream, ObjectStreamEncoding::Binary, &mut output),
            Err(ObjectStreamError::MissingV2Specialization { .. })
        ));
        assert_eq!(output, b"unchanged");
    }

    #[test]
    fn streaming_transcoder_preflights_before_touching_caller_output() {
        let stream = ObjectStream {
            version: 3,
            elements: vec![Element::new(Uuid::from_u128(1)).with_data([1, 2, 3, 4])],
            ..ObjectStream::default()
        };
        let binary = to_bytes(&stream).unwrap();
        let mut output = b"unchanged".to_vec();

        assert!(matches!(
            transcode_to_writer(&binary, ObjectStreamEncoding::Xml, None, &mut output),
            Err(ObjectStreamError::MissingSerializer { .. })
        ));
        assert_eq!(output, b"unchanged");
    }

    #[test]
    fn raw_text_transcodes_losslessly_between_xml_and_json() {
        let id = Uuid::from_u128(1);
        let xml = format!(
            "<ObjectStream version=\"3\"><Class name=\"Label\" value=\"CamelCase value\" type=\"{{{id}}}\"/></ObjectStream>"
        );

        let json = transcode_bytes(xml.as_bytes(), ObjectStreamEncoding::Json, None).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["Objects"][0]["value"], "CamelCase value");

        let round_trip = transcode_bytes(&json, ObjectStreamEncoding::Xml, None).unwrap();
        let round_trip = std::str::from_utf8(&round_trip).unwrap();
        assert!(round_trip.contains("value=\"CamelCase value\""));
    }

    #[test]
    fn camel_case_field_crc_uses_canonical_ascii_lowercase() {
        let mut element = Element::new(Uuid::from_u128(1)).with_field("CamelCase");
        element.name_crc = Some(crate::field_name_crc("camelcase"));
        let stream = ObjectStream {
            version: 3,
            elements: vec![element],
            ..ObjectStream::default()
        };

        let bytes = to_bytes(&stream).unwrap();
        assert_eq!(
            &bytes[6..10],
            &crate::field_name_crc("CamelCase").to_be_bytes()
        );
    }

    #[test]
    fn invalid_public_stream_tag_cannot_fallback_to_binary() {
        let stream = ObjectStream {
            tag: StreamTag(0xff),
            version: 3,
            elements: Vec::new(),
            ..ObjectStream::default()
        };

        assert!(matches!(
            to_encoding_bytes(&stream, ObjectStreamEncoding::Binary),
            Err(ObjectStreamError::InvalidStreamTag(0xff))
        ));
    }

    #[test]
    fn context_write_validates_big_endian_and_text_payloads_before_early_return() {
        let id = Uuid::from_u128(9);
        let mut context = ObjectStreamReadContext::default();
        context
            .insert_class(id, crate::context::ReflectedClass::new(id))
            .unwrap();
        let mut binary_element = Element::new(id);
        context
            .resolve_element_tree(3, &mut binary_element)
            .unwrap();
        binary_element.data = Some(vec![1]);
        let binary = ObjectStream {
            version: 3,
            elements: vec![binary_element],
            ..ObjectStream::default()
        };
        assert!(matches!(
            write_as_with_context(
                &binary,
                ObjectStreamEncoding::Binary,
                &context,
                &mut Vec::new(),
            ),
            Err(ObjectStreamError::MissingSerializer { type_id }) if type_id == id
        ));

        let mut text_element = Element::new(id).with_data(b"opaque".as_slice());
        text_element.payload_encoding = PayloadEncoding::Text;
        text_element.resolution = crate::TypeResolution::Unresolved;
        let text = ObjectStream {
            tag: StreamTag::XML,
            version: 3,
            elements: vec![text_element],
            ..ObjectStream::default()
        };
        assert!(matches!(
            write_as_with_context(
                &text,
                ObjectStreamEncoding::Xml,
                &context,
                &mut Vec::new(),
            ),
            Err(ObjectStreamError::UnresolvedElementType { type_id }) if type_id == id
        ));
    }

    #[test]
    fn context_writer_rejects_structural_data_overlay_before_touching_output() {
        let stream = ObjectStream {
            version: 3,
            elements: vec![Element::new(crate::types::DATA_OVERLAY_INFO).with_test_class()],
            ..ObjectStream::default()
        };
        let mut output = b"unchanged".to_vec();

        assert!(matches!(
            write_as_with_context(
                &stream,
                ObjectStreamEncoding::Binary,
                &ObjectStreamReadContext::default(),
                &mut output,
            ),
            Err(ObjectStreamError::InvalidDataOverlay(_))
        ));
        assert_eq!(output, b"unchanged");
    }

    #[test]
    fn binary_to_xml_handles_deep_objectstream_without_recursion() -> Result<(), ObjectStreamError>
    {
        let bytes = deep_binary_chain(20_000);
        let xml = transcode_bytes(&bytes, ObjectStreamEncoding::Xml, None)?;
        let xml = std::str::from_utf8(&xml).expect("xml output is utf-8");

        assert!(xml.starts_with("<ObjectStream version=\"3\">"));
        assert_eq!(xml.matches("<Class ").count(), 20_000);
        assert!(xml.ends_with("</ObjectStream>"));
        Ok(())
    }

    #[test]
    fn binary_to_json_handles_deep_objectstream_without_recursion() -> Result<(), ObjectStreamError>
    {
        let bytes = deep_binary_chain(20_000);
        let json = transcode_bytes(&bytes, ObjectStreamEncoding::Json, None)?;
        let json = std::str::from_utf8(&json).expect("json output is utf-8");

        assert!(json.starts_with("{\n  \"name\": \"ObjectStream\""));
        assert_eq!(json.matches("\"typeId\"").count(), 20_000);
        assert!(json.ends_with("\n}"));
        Ok(())
    }

    #[test]
    fn binary_copy_validates_deep_objectstream_without_recursion() -> Result<(), ObjectStreamError>
    {
        let bytes = deep_binary_chain(20_000);
        let output = transcode_bytes(&bytes, ObjectStreamEncoding::Binary, None)?;

        assert_eq!(output, bytes);
        Ok(())
    }

    fn deep_binary_chain(depth: usize) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(5 + depth * 17 + depth + 1);
        bytes.push(StreamTag::BINARY.0);
        bytes.extend_from_slice(&3u32.to_be_bytes());
        let id = Uuid::nil();
        for _ in 0..depth {
            bytes.push(ST_BINARYFLAG_ELEMENT_HEADER);
            bytes.extend_from_slice(id.as_bytes());
        }
        bytes.extend(std::iter::repeat_n(0, depth + 1));
        bytes
    }
}
