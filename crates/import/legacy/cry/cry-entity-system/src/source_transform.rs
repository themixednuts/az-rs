//! Legacy `CryEntitySystem` `.ent` source import transform.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceFormat, SourceSchemaType,
    normalize_source_path, source_schema_type,
};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

use crate::{EntityClassDescriptor, EntityClassParseError, parse_entity_class_descriptor};

pub const ENTITY_CLASS_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<EntityClassSourceFormat>();

#[derive(SourceFormat)]
#[source(schema = "azoth.compat.cry.EntityClassSource", ext = "ent.ron")]
pub struct EntityClassSourceFormat;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityClassSource {
    pub source_path: String,
    pub name: String,
    pub script: Option<String>,
    pub flags: EntityClassFlagSource,
}

impl EntityClassSource {
    /// Builds the authoring source from a legacy `.ent` descriptor.
    ///
    /// # Errors
    ///
    /// Returns any error [`parse_entity_class_descriptor`] returns —
    /// [`EntityClassParseError::MissingRoot`] when there is no `Entity`
    /// element, [`EntityClassParseError::Utf8`] for non-UTF-8 bytes,
    /// [`EntityClassParseError::Xml`] or [`EntityClassParseError::Attribute`]
    /// for a malformed document,
    /// [`EntityClassParseError::UnexpectedElement`] for a foreign element,
    /// [`EntityClassParseError::MissingAttribute`] or
    /// [`EntityClassParseError::EmptyName`] for a nameless class, and
    /// [`EntityClassParseError::InvalidBoolean`] for an unparseable flag.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, EntityClassParseError> {
        let descriptor = parse_entity_class_descriptor(bytes)?;
        Ok(Self::from_descriptor(source_path, &descriptor))
    }

    #[must_use]
    pub fn from_descriptor(source_path: &str, descriptor: &EntityClassDescriptor<'_>) -> Self {
        Self {
            source_path: normalize_source_path(source_path),
            name: descriptor.name.to_string(),
            script: descriptor.script.as_deref().map(str::to_owned),
            flags: EntityClassFlagSource {
                invisible: descriptor.is_invisible(),
                bbox_selection: descriptor.has_bbox_selection(),
            },
        }
    }

    /// Serialises this source to pretty-printed RON bytes with a trailing
    /// newline.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] the serializer reports. The fields here are
    /// plain strings and bools that RON always accepts, so this is reachable
    /// only through a serializer-internal failure.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }

    /// Parses an authoring source back from RON bytes.
    ///
    /// # Errors
    ///
    /// Returns a [`ron::error::SpannedError`] locating the first syntax error,
    /// an unknown or missing field, or a value whose type does not match the
    /// field it is assigned to.
    pub fn from_ron_bytes(bytes: &[u8]) -> Result<Self, ron::error::SpannedError> {
        ron::de::from_bytes(bytes)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityClassFlagSource {
    pub invisible: bool,
    pub bbox_selection: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EntityClassSourceTransform;

impl LegacySourceTransform for EntityClassSourceTransform {
    type Error = EntityClassSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        if !is_legacy_entity_class_source(&input.source_path) {
            return Err(EntityClassSourceTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            });
        }

        let source = EntityClassSource::from_legacy(&input.source_path, input.bytes)?;
        Ok(LegacySourceOutput::authoring_source(
            entity_class_source_path(&input.source_path),
            ENTITY_CLASS_SOURCE_SCHEMA,
            source.to_ron_bytes()?,
        ))
    }
}

#[must_use]
pub fn is_legacy_entity_class_source(source_path: &str) -> bool {
    std::path::Path::new(&normalize_source_path(source_path))
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ent"))
}

#[must_use]
pub fn entity_class_source_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized.strip_suffix(".ent").unwrap_or(&normalized);
    format!("{stem}.ent.ron")
}

#[derive(Debug, thiserror::Error)]
pub enum EntityClassSourceTransformError {
    #[error("unsupported entity class source path {path}")]
    UnsupportedPath { path: String },
    #[error("parse entity class source: {0}")]
    Parse(#[from] EntityClassParseError),
    #[error("serialize entity class source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{LegacySourceInput, LegacySourceOutput, LegacySourceTransform};

    use super::*;

    #[test]
    fn entity_class_source_path_replaces_legacy_extension() {
        assert_eq!(
            entity_class_source_path("Entities/AI/NavigationSeedPoint.ent"),
            "entities/ai/navigationseedpoint.ent.ron"
        );
    }

    #[test]
    fn classifies_only_legacy_entity_class_sources() {
        assert!(is_legacy_entity_class_source(
            "Entities/AI/NavigationSeedPoint.ent"
        ));
        assert!(!is_legacy_entity_class_source(
            "entities/ai/navigationseedpoint.ent.ron"
        ));
        assert!(!is_legacy_entity_class_source(
            "entities/ai/navigationseedpoint.xml"
        ));
    }

    #[test]
    fn transform_emits_ron_authoring_source() {
        let output = EntityClassSourceTransform
            .transform(LegacySourceInput::new("Entities/AI/NavigationSeedPoint.ent", br#"<Entity Name="NavigationSeedPoint" Script="Scripts/Entities/AI/NavigationSeedPoint.lua" Invisible="1" BBoxSelection="true"/>"#))
            .unwrap();

        let LegacySourceOutput::AuthoringSource(artifact) = output else {
            panic!("entity class should become authoring source");
        };
        assert_eq!(artifact.path, "entities/ai/navigationseedpoint.ent.ron");
        assert_eq!(artifact.schema, ENTITY_CLASS_SOURCE_SCHEMA);

        let source = EntityClassSource::from_ron_bytes(&artifact.bytes).unwrap();
        assert_eq!(source.source_path, "entities/ai/navigationseedpoint.ent");
        assert_eq!(source.name, "NavigationSeedPoint");
        assert_eq!(
            source.script.as_deref(),
            Some("Scripts/Entities/AI/NavigationSeedPoint.lua")
        );
        assert!(source.flags.invisible);
        assert!(source.flags.bbox_selection);
    }
}
