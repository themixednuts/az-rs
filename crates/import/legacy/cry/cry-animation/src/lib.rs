//! Parsers for `CryAnimation` metadata assets.

use std::borrow::Cow;

use quick_xml::{
    XmlVersion,
    escape::{EscapeError, resolve_predefined_entity},
    events::{BytesRef, BytesText},
};

pub mod events;
pub mod source_transform;

pub use events::*;
pub use source_transform::*;

pub(crate) fn xml_text_content<'a>(
    event: &BytesText<'a>,
) -> Result<Cow<'a, str>, quick_xml::Error> {
    event
        .xml_content(XmlVersion::default())
        .map_err(quick_xml::Error::from)
}

pub(crate) fn xml_general_reference_content(
    event: &BytesRef<'_>,
) -> Result<Cow<'static, str>, quick_xml::Error> {
    if let Some(ch) = event.resolve_char_ref()? {
        return Ok(Cow::Owned(ch.to_string()));
    }

    let reference = event.decode().map_err(quick_xml::Error::from)?;
    let Some(value) = resolve_predefined_entity(&reference) else {
        return Err(quick_xml::Error::from(EscapeError::UnrecognizedEntity(
            0..event.len(),
            reference.into_owned(),
        )));
    };
    Ok(Cow::Borrowed(value))
}
