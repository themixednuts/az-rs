//! Neutral wire carriers shared by structured-authoring service protocols.

// Machine-generated Cap'n Proto output: written into OUT_DIR by build.rs and
// regenerated on every build, so it is not in git and has no per-site fix. The
// macro completes upstream's own `#![allow(clippy::all)]` for the pedantic and
// nursery groups this workspace denies.
az_proto_core::generated_schema!(pub mod authoring_capnp, "azoth/authoring_capnp.rs");

use std::collections::BTreeSet;

pub use az_core::{ReflectedValueEncoding, ReflectedValueEnvelope};
use capnp::Error;

pub use authoring_capnp as azoth_authoring_capnp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileEditObject {
    pub object_id: String,
    pub schema: String,
    pub value: ReflectedValueEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileEditDocument {
    pub root_object_id: Option<String>,
    pub root_schema: String,
    pub value: ReflectedValueEnvelope,
    pub objects: Vec<SourceFileEditObject>,
    pub codec_state: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFileEditOperation {
    AppendDefault,
    DuplicateObject { object_id: String },
    RemoveObject { object_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFileCodecOperation {
    Load,
    Edit(SourceFileEditOperation),
    RestoreDocument(SourceFileEditDocument),
}

fn invalid_value(kind: &str, detail: impl Into<String>) -> Error {
    Error::failed(format!("invalid {kind}: {}", detail.into()))
}

fn require_non_empty(kind: &str, field: &str, value: &str) -> Result<(), Error> {
    if value.is_empty() {
        return Err(invalid_value(kind, format!("{field} must not be empty")));
    }
    if value.trim() != value {
        return Err(invalid_value(
            kind,
            format!("{field} must not have leading or trailing whitespace"),
        ));
    }
    Ok(())
}

fn write_optional_text(
    value: Option<&str>,
    mut builder: az_proto_core::core_capnp::optional_text::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value),
        None => builder.set_none(()),
    }
}

fn read_optional_text(
    reader: az_proto_core::core_capnp::optional_text::Reader<'_>,
) -> Result<Option<String>, Error> {
    match reader.which()? {
        az_proto_core::core_capnp::optional_text::Which::None(()) => Ok(None),
        az_proto_core::core_capnp::optional_text::Which::Value(value) => {
            Ok(Some(value?.to_string()?))
        }
    }
}

/// Narrows a `usize` list length or element index to the `u32` Cap'n Proto
/// uses for both.
///
/// The casts this replaces were lossy on 64-bit targets: an oversized
/// collection would have silently written a wrapped length and produced a
/// message that decodes to the wrong number of elements. Failing the write is
/// the honest outcome, and every caller is already in a `Result`.
///
/// # Errors
///
/// Returns an error if `value` does not fit in a `u32`.
fn capnp_list_index(value: usize) -> Result<u32, Error> {
    u32::try_from(value)
        .map_err(|_| Error::failed("Cap'n Proto list length exceeds u32 range".to_string()))
}

/// # Errors
///
/// Returns an error if the envelope's type path is empty or has leading or
/// trailing whitespace.
pub fn validate_reflected_value_envelope(value: &ReflectedValueEnvelope) -> Result<(), Error> {
    require_non_empty("reflected value envelope", "type path", &value.type_path)
}

/// # Errors
///
/// Returns an error if the envelope fails [`validate_reflected_value_envelope`],
/// or if the message runs out of space while writing the envelope.
pub fn write_reflected_value_envelope(
    value: &ReflectedValueEnvelope,
    mut builder: authoring_capnp::reflected_value_envelope::Builder<'_>,
) -> Result<(), Error> {
    validate_reflected_value_envelope(value)?;
    builder.set_type_path(&value.type_path);
    builder.set_encoding(match value.encoding {
        ReflectedValueEncoding::BevyRemoteJson => {
            authoring_capnp::ReflectedValueEncoding::BevyRemoteJson
        }
        ReflectedValueEncoding::TypedRon => authoring_capnp::ReflectedValueEncoding::TypedRon,
        ReflectedValueEncoding::CapnpData => authoring_capnp::ReflectedValueEncoding::CapnpData,
    });
    builder.set_payload(&value.payload);
    Ok(())
}

/// # Errors
///
/// Returns an error if the encoding, type path or payload is absent from the
/// message or is not valid UTF-8, or if the decoded envelope fails
/// [`validate_reflected_value_envelope`].
pub fn read_reflected_value_envelope(
    reader: authoring_capnp::reflected_value_envelope::Reader<'_>,
) -> Result<ReflectedValueEnvelope, Error> {
    let encoding = match reader.get_encoding()? {
        authoring_capnp::ReflectedValueEncoding::BevyRemoteJson => {
            ReflectedValueEncoding::BevyRemoteJson
        }
        authoring_capnp::ReflectedValueEncoding::TypedRon => ReflectedValueEncoding::TypedRon,
        authoring_capnp::ReflectedValueEncoding::CapnpData => ReflectedValueEncoding::CapnpData,
    };
    let value = ReflectedValueEnvelope {
        type_path: reader.get_type_path()?.to_string()?,
        encoding,
        payload: reader.get_payload()?.to_vec(),
    };
    validate_reflected_value_envelope(&value)?;
    Ok(value)
}

/// # Errors
///
/// Returns an error if the object's schema is empty or whitespace-padded, if
/// its value fails [`validate_reflected_value_envelope`], or if the schema does
/// not match the reflected value's type path.
pub fn validate_source_file_edit_object(object: &SourceFileEditObject) -> Result<(), Error> {
    require_non_empty("source file edit object", "schema", &object.schema)?;
    validate_reflected_value_envelope(&object.value)?;
    if object.schema != object.value.type_path {
        return Err(invalid_value(
            "source file edit object",
            "schema must match the reflected value type path",
        ));
    }
    Ok(())
}

/// # Errors
///
/// Returns an error if the root schema is empty or whitespace-padded, if the
/// root value fails [`validate_reflected_value_envelope`], if the root schema
/// does not match the root value's type path, if any object fails
/// [`validate_source_file_edit_object`], if a multi-object document has an
/// object with an empty id, if two objects share an id, or if `root_object_id`
/// names an object the document does not carry.
pub fn validate_source_file_edit_document(document: &SourceFileEditDocument) -> Result<(), Error> {
    require_non_empty(
        "source file edit document",
        "root schema",
        &document.root_schema,
    )?;
    validate_reflected_value_envelope(&document.value)?;
    if document.root_schema != document.value.type_path {
        return Err(invalid_value(
            "source file edit document",
            "root schema must match the root reflected value type path",
        ));
    }

    let mut object_ids = BTreeSet::new();
    for object in &document.objects {
        validate_source_file_edit_object(object)?;
        if document.objects.len() > 1 && object.object_id.is_empty() {
            return Err(invalid_value(
                "source file edit document",
                "multi-object documents require stable non-empty object ids",
            ));
        }
        if !object_ids.insert(object.object_id.as_str()) {
            return Err(invalid_value(
                "source file edit document",
                format!("duplicate object id `{}`", object.object_id),
            ));
        }
    }

    if let Some(root_object_id) = document.root_object_id.as_deref()
        && !root_object_id.is_empty()
        && !object_ids.contains(root_object_id)
    {
        return Err(invalid_value(
            "source file edit document",
            format!("root object `{root_object_id}` is not present in the document"),
        ));
    }
    Ok(())
}

/// # Errors
///
/// Returns an error if a `DuplicateObject` or `RemoveObject` operation carries
/// an object id that is empty or whitespace-padded.
pub fn validate_source_file_edit_operation(
    operation: &SourceFileEditOperation,
) -> Result<(), Error> {
    match operation {
        SourceFileEditOperation::AppendDefault => Ok(()),
        SourceFileEditOperation::DuplicateObject { object_id }
        | SourceFileEditOperation::RemoveObject { object_id } => {
            require_non_empty("source file edit operation", "object id", object_id)
        }
    }
}

impl SourceFileEditObject {
    /// # Errors
    ///
    /// Returns an error if the object fails
    /// [`validate_source_file_edit_object`], or if the message runs out of
    /// space while writing the object.
    pub fn to_capnp(
        &self,
        mut builder: authoring_capnp::source_file_edit_object::Builder<'_>,
    ) -> Result<(), Error> {
        validate_source_file_edit_object(self)?;
        builder.set_object_id(&self.object_id);
        builder.set_schema(&self.schema);
        write_reflected_value_envelope(&self.value, builder.reborrow().init_value())
    }

    /// # Errors
    ///
    /// Returns an error if a field of the object is absent from the message or
    /// is not valid UTF-8, or if the decoded object fails
    /// [`validate_source_file_edit_object`].
    pub fn from_capnp(
        reader: authoring_capnp::source_file_edit_object::Reader<'_>,
    ) -> Result<Self, Error> {
        let object = Self {
            object_id: reader.get_object_id()?.to_string()?,
            schema: reader.get_schema()?.to_string()?,
            value: read_reflected_value_envelope(reader.get_value()?)?,
        };
        validate_source_file_edit_object(&object)?;
        Ok(object)
    }
}

impl SourceFileEditDocument {
    /// # Errors
    ///
    /// Returns an error if the document fails
    /// [`validate_source_file_edit_document`], if the object list is longer
    /// than a Cap'n Proto list can address, or if the message runs out of space
    /// while writing the document.
    pub fn to_capnp(
        &self,
        mut builder: authoring_capnp::source_file_edit_document::Builder<'_>,
    ) -> Result<(), Error> {
        validate_source_file_edit_document(self)?;
        write_optional_text(
            self.root_object_id.as_deref(),
            builder.reborrow().init_root_object_id(),
        );
        builder.set_root_schema(&self.root_schema);
        write_reflected_value_envelope(&self.value, builder.reborrow().init_value())?;
        let mut objects =
            builder
                .reborrow()
                .init_objects(self.objects.len().try_into().map_err(|_| {
                    invalid_value("source file edit document", "object count exceeds u32")
                })?);
        for (index, object) in self.objects.iter().enumerate() {
            object.to_capnp(objects.reborrow().get(capnp_list_index(index)?))?;
        }
        builder.set_codec_state(&self.codec_state);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if a field of the document, or of any object within it,
    /// is absent from the message or is not valid UTF-8, or if the decoded
    /// document fails [`validate_source_file_edit_document`].
    pub fn from_capnp(
        reader: authoring_capnp::source_file_edit_document::Reader<'_>,
    ) -> Result<Self, Error> {
        let document = Self {
            root_object_id: read_optional_text(reader.get_root_object_id()?)?,
            root_schema: reader.get_root_schema()?.to_string()?,
            value: read_reflected_value_envelope(reader.get_value()?)?,
            objects: reader
                .get_objects()?
                .iter()
                .map(SourceFileEditObject::from_capnp)
                .collect::<Result<Vec<_>, _>>()?,
            codec_state: reader.get_codec_state()?.to_vec(),
        };
        validate_source_file_edit_document(&document)?;
        Ok(document)
    }
}

impl SourceFileEditOperation {
    /// # Errors
    ///
    /// Returns an error if the operation fails
    /// [`validate_source_file_edit_operation`], or if the message runs out of
    /// space while writing the operation.
    pub fn to_capnp(
        &self,
        mut builder: authoring_capnp::source_file_edit_operation::Builder<'_>,
    ) -> Result<(), Error> {
        validate_source_file_edit_operation(self)?;
        match self {
            Self::AppendDefault => builder.set_append_default(()),
            Self::DuplicateObject { object_id } => builder.set_duplicate_object(object_id),
            Self::RemoveObject { object_id } => builder.set_remove_object(object_id),
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the operation's union tag is unrecognized, if its
    /// object id is absent from the message or is not valid UTF-8, or if the
    /// decoded operation fails [`validate_source_file_edit_operation`].
    pub fn from_capnp(
        reader: authoring_capnp::source_file_edit_operation::Reader<'_>,
    ) -> Result<Self, Error> {
        let operation = match reader.which()? {
            authoring_capnp::source_file_edit_operation::AppendDefault(()) => Self::AppendDefault,
            authoring_capnp::source_file_edit_operation::DuplicateObject(object_id) => {
                Self::DuplicateObject {
                    object_id: object_id?.to_string()?,
                }
            }
            authoring_capnp::source_file_edit_operation::RemoveObject(object_id) => {
                Self::RemoveObject {
                    object_id: object_id?.to_string()?,
                }
            }
        };
        validate_source_file_edit_operation(&operation)?;
        Ok(operation)
    }
}

impl SourceFileCodecOperation {
    /// # Errors
    ///
    /// Returns any error [`SourceFileEditOperation::to_capnp`] or
    /// [`SourceFileEditDocument::to_capnp`] returns for the wrapped payload.
    /// The `Load` variant carries no payload and cannot fail.
    pub fn to_capnp(
        &self,
        mut builder: authoring_capnp::source_file_codec_operation::Builder<'_>,
    ) -> Result<(), Error> {
        match self {
            Self::Load => {
                builder.set_load(());
                Ok(())
            }
            Self::Edit(operation) => operation.to_capnp(builder.reborrow().init_edit()),
            Self::RestoreDocument(document) => {
                document.to_capnp(builder.reborrow().init_restore_document())
            }
        }
    }

    /// # Errors
    ///
    /// Returns an error if the operation's union tag is unrecognized or its
    /// payload is absent from the message, plus any error
    /// [`SourceFileEditOperation::from_capnp`] or
    /// [`SourceFileEditDocument::from_capnp`] returns for that payload.
    pub fn from_capnp(
        reader: authoring_capnp::source_file_codec_operation::Reader<'_>,
    ) -> Result<Self, Error> {
        match reader.which()? {
            authoring_capnp::source_file_codec_operation::Load(()) => Ok(Self::Load),
            authoring_capnp::source_file_codec_operation::Edit(operation) => {
                Ok(Self::Edit(SourceFileEditOperation::from_capnp(operation?)?))
            }
            authoring_capnp::source_file_codec_operation::RestoreDocument(document) => Ok(
                Self::RestoreDocument(SourceFileEditDocument::from_capnp(document?)?),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use capnp::message;

    use super::*;

    fn document() -> SourceFileEditDocument {
        SourceFileEditDocument {
            root_object_id: Some("row:one".to_string()),
            root_schema: "example.Row".to_string(),
            value: ReflectedValueEnvelope::typed_ron("example.Row", "(id: \"one\")"),
            objects: vec![SourceFileEditObject {
                object_id: "row:one".to_string(),
                schema: "example.Row".to_string(),
                value: ReflectedValueEnvelope::typed_ron("example.Row", "(id: \"one\")"),
            }],
            codec_state: b"stable-row-layout".to_vec(),
        }
    }

    #[test]
    fn source_file_edit_document_round_trips_codec_state() {
        let expected = document();
        let mut message = message::Builder::new_default();
        expected
            .to_capnp(
                message.init_root::<authoring_capnp::source_file_edit_document::Builder<'_>>(),
            )
            .unwrap();
        let reader = message
            .get_root_as_reader::<authoring_capnp::source_file_edit_document::Reader<'_>>()
            .unwrap();

        assert_eq!(
            SourceFileEditDocument::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn source_file_edit_document_rejects_object_schema_mismatch() {
        let mut invalid = document();
        invalid.objects[0].schema = "example.OtherRow".to_string();
        let mut message = message::Builder::new_default();

        assert!(
            invalid
                .to_capnp(
                    message.init_root::<authoring_capnp::source_file_edit_document::Builder<'_>>()
                )
                .is_err()
        );
    }
}
