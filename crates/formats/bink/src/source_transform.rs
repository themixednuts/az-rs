//! Legacy Bink media import classification.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceFormat, SourceSchemaType,
    source_schema_type,
};

use crate::{BinkFile, ParseError};

pub const BINK_MEDIA_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<BinkMediaSourceFormat>();

#[derive(SourceFormat)]
#[source(schema = "azoth.compat.bink.BinkMediaSource", ext = "bk2", ext = "bik")]
pub struct BinkMediaSourceFormat;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BinkMediaSourceTransform;

impl LegacySourceTransform for BinkMediaSourceTransform {
    type Error = BinkMediaSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        BinkFile::parse(input.bytes).map_err(|source| BinkMediaSourceTransformError::Parse {
            path: input.source_path.to_string(),
            source,
        })?;
        Ok(LegacySourceOutput::external_product_payload(
            &input.source_path,
            BINK_MEDIA_SOURCE_SCHEMA,
            input.bytes.to_vec(),
        ))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BinkMediaSourceTransformError {
    #[error("parse Bink media {path:?}")]
    Parse {
        path: String,
        #[source]
        source: ParseError,
    },
}
