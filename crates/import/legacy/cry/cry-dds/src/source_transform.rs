//! Legacy DDS import classification for Azoth source workflows.

use az_asset::EngineTextureFormat;
use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceSchemaType,
};
use thiserror::Error;

use crate::{DdsAsset, ParseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureSourceTransform {
    texture_format: EngineTextureFormat,
    source_schema: SourceSchemaType,
}

impl TextureSourceTransform {
    #[inline]
    #[must_use]
    pub const fn new(texture_format: EngineTextureFormat, source_schema: SourceSchemaType) -> Self {
        Self {
            texture_format,
            source_schema,
        }
    }

    #[inline]
    #[must_use]
    pub const fn texture_format(self) -> EngineTextureFormat {
        self.texture_format
    }

    #[inline]
    #[must_use]
    pub const fn source_schema(self) -> SourceSchemaType {
        self.source_schema
    }
}

#[derive(Debug, Error)]
pub enum TextureSourceTransformError {
    #[error("parse DDS source {path:?}")]
    Parse {
        path: String,
        #[source]
        source: ParseError,
    },
}

impl LegacySourceTransform for TextureSourceTransform {
    type Error = TextureSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        DdsAsset::parse(&input.source_path, input.bytes).map_err(|source| {
            TextureSourceTransformError::Parse {
                path: input.source_path.to_string(),
                source,
            }
        })?;

        Ok(match self.texture_format {
            EngineTextureFormat::Dds => LegacySourceOutput::compatibility_evidence(
                &input.source_path,
                self.source_schema,
                input.bytes.to_vec(),
            ),
            EngineTextureFormat::Ktx2 => LegacySourceOutput::Unclassified {
                reason: "KTX2 texture source emission requires grouped DDS header + split mip payload assembly and a KTX2 writer".to_string(),
            },
        })
    }
}
