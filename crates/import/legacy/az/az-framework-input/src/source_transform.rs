//! Source transform for `AzFramework` input binding assets.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceFormat, SourceSchemaType,
    normalize_source_path, source_schema_type,
};
use az_objectstream::context::ObjectStreamReadContext;
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

use crate::{
    AnalogInputEventGenerator, InputBindingsParseError, InputEventBindingsAsset,
    InputEventGenerator,
};

pub const INPUT_BINDINGS_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<InputBindingsSourceFormat>();

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.azframework.InputBindingsSource",
    ext = "inputbindings.ron"
)]
pub struct InputBindingsSourceFormat;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputBindingsSource {
    pub source_path: String,
    pub groups: Vec<InputBindingGroupSource>,
}

impl InputBindingsSource {
    /// Builds the authoring source from a legacy `.inputbindings`
    /// `ObjectStream`.
    ///
    /// # Errors
    ///
    /// Returns any error [`InputEventBindingsAsset::parse`] returns —
    /// [`InputBindingsParseError::ObjectStream`] or
    /// [`InputBindingsParseError::Value`] for an undecodable stream,
    /// [`InputBindingsParseError::UnsupportedObjectStreamVersion`],
    /// [`InputBindingsParseError::MissingRoot`] or
    /// [`InputBindingsParseError::MultipleRoots`] when the document does not
    /// hold exactly one bindings root, and
    /// [`InputBindingsParseError::UnexpectedType`],
    /// [`InputBindingsParseError::MissingField`] or
    /// [`InputBindingsParseError::UnexpectedField`] for a reflected class that
    /// does not match the expected layout.
    pub fn from_legacy(
        source_path: &str,
        bytes: &[u8],
        context: &ObjectStreamReadContext,
    ) -> Result<Self, InputBindingsParseError> {
        let asset = InputEventBindingsAsset::parse(bytes, context)?;
        Ok(Self::from_asset(source_path, &asset))
    }

    #[must_use]
    pub fn from_asset(source_path: &str, asset: &InputEventBindingsAsset) -> Self {
        Self {
            source_path: normalize_source_path(source_path),
            groups: asset
                .bindings()
                .event_groups()
                .iter()
                .map(|group| InputBindingGroupSource {
                    event_name: group.event_name.to_string(),
                    generators: group
                        .event_generators
                        .iter()
                        .map(InputBindingGeneratorSource::from_legacy)
                        .collect(),
                    exclude_from_release: group.exclude_from_release,
                })
                .collect(),
        }
    }

    /// Serialises this source to pretty-printed RON bytes with a trailing
    /// newline.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] the serializer reports. The fields here are
    /// strings, bools, floats and enums that RON always accepts, so this is
    /// reachable only through a serializer-internal failure.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputBindingGroupSource {
    pub event_name: String,
    pub generators: Vec<InputBindingGeneratorSource>,
    pub exclude_from_release: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputBindingGeneratorSource {
    Analog {
        device_type: String,
        input_name: String,
        value_multiplier: f32,
        send_continuous_updates: bool,
        dead_zone: f32,
    },
}

impl InputBindingGeneratorSource {
    fn from_legacy(generator: &InputEventGenerator) -> Self {
        match generator {
            InputEventGenerator::Analog(analog) => Self::from_legacy_analog(analog),
        }
    }

    fn from_legacy_analog(generator: &AnalogInputEventGenerator) -> Self {
        Self::Analog {
            device_type: generator.action.input_device_type.to_string(),
            input_name: generator.action.input_name.to_string(),
            value_multiplier: generator.action.event_value_multiplier,
            send_continuous_updates: generator.send_continuous_updates,
            dead_zone: generator.dead_zone,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InputBindingsSourceTransform<'a> {
    context: &'a ObjectStreamReadContext,
}

impl<'a> InputBindingsSourceTransform<'a> {
    #[must_use]
    pub const fn new(context: &'a ObjectStreamReadContext) -> Self {
        Self { context }
    }
}

impl LegacySourceTransform for InputBindingsSourceTransform<'_> {
    type Error = InputBindingsSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        if !is_legacy_input_bindings_source(&input.source_path) {
            return Err(InputBindingsSourceTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            });
        }

        let source =
            InputBindingsSource::from_legacy(&input.source_path, input.bytes, self.context)?;
        Ok(LegacySourceOutput::authoring_source(
            input_bindings_source_path(&input.source_path),
            INPUT_BINDINGS_SOURCE_SCHEMA,
            source.to_ron_bytes()?,
        ))
    }
}

#[must_use]
pub fn is_legacy_input_bindings_source(source_path: &str) -> bool {
    normalize_source_path(source_path).ends_with(".inputbindings")
}

#[must_use]
pub fn input_bindings_source_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized
        .strip_suffix(".inputbindings")
        .unwrap_or(&normalized);
    let rest = stem
        .strip_prefix("libs/config/")
        .or_else(|| stem.strip_prefix("config/"))
        .unwrap_or(stem);
    format!("input/{rest}.inputbindings.ron")
}

#[derive(Debug, thiserror::Error)]
pub enum InputBindingsSourceTransformError {
    #[error("unsupported input bindings path {path}")]
    UnsupportedPath { path: String },
    #[error("parse input bindings source: {0}")]
    Parse(#[from] InputBindingsParseError),
    #[error("serialize input bindings source RON: {0}")]
    Serialize(#[from] ron::Error),
}
