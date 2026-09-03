use std::str;

use super::to_ron_bytes;

mod extract;
mod types;
mod xml_tree;

pub use types::*;

use extract::build_character_parameters_source;
use xml_tree::XmlTreeParser;

impl CharacterParametersSource {
    /// Parses a legacy Cry `.chrparams` document into the authoring source
    /// model.
    ///
    /// # Errors
    ///
    /// Returns [`CharacterParametersParseError::UnsupportedPath`] when
    /// `source_path` does not name a `.chrparams` document,
    /// [`CharacterParametersParseError::InvalidUtf8`] when `bytes` is not
    /// UTF-8, [`CharacterParametersParseError::Xml`] or
    /// [`CharacterParametersParseError::Attribute`] for malformed XML, the
    /// element-nesting variants ([`CharacterParametersParseError::MissingRoot`],
    /// [`CharacterParametersParseError::UnexpectedRoot`],
    /// [`CharacterParametersParseError::MismatchedEnd`], and siblings) for a
    /// document that is not a `<Params>` tree, and
    /// [`CharacterParametersParseError::InvalidBool`],
    /// [`CharacterParametersParseError::InvalidFloat`], or
    /// [`CharacterParametersParseError::InvalidInteger`] for an attribute that
    /// does not parse as its declared type.
    pub fn from_legacy(
        source_path: &str,
        bytes: &[u8],
    ) -> Result<Self, CharacterParametersParseError> {
        let normalized_source_path = az_asset_builder::normalize_source_path(source_path);
        if !is_character_parameters_source_path(&normalized_source_path) {
            return Err(CharacterParametersParseError::UnsupportedPath {
                path: normalized_source_path,
            });
        }

        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let xml = str::from_utf8(bytes).map_err(CharacterParametersParseError::InvalidUtf8)?;
        let root = XmlTreeParser::parse(xml)?;
        build_character_parameters_source(normalized_source_path, root)
    }

    /// Serializes the authoring source model as pretty RON.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] raised by the RON serializer when a field
    /// cannot be represented in RON.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        to_ron_bytes(self)
    }
}
