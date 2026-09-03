//! Legacy font face import classification.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceSchemaType,
    normalize_source_path,
};

use crate::{FontFaceData, OpenTypeParseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontFaceSourceTransform {
    source_schema: SourceSchemaType,
}

impl FontFaceSourceTransform {
    #[inline]
    #[must_use]
    pub const fn new(source_schema: SourceSchemaType) -> Self {
        Self { source_schema }
    }

    #[inline]
    #[must_use]
    pub const fn source_schema(self) -> SourceSchemaType {
        self.source_schema
    }
}

impl LegacySourceTransform for FontFaceSourceTransform {
    type Error = FontFaceSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        FontFaceData::parse(input.bytes).map_err(|source| FontFaceSourceTransformError::Parse {
            path: input.source_path.to_string(),
            source,
        })?;
        Ok(LegacySourceOutput::external_product_payload(
            font_face_source_path(&input.source_path),
            self.source_schema,
            input.bytes.to_vec(),
        ))
    }
}

#[must_use]
pub fn font_face_source_path(source_path: &str) -> String {
    /// Legacy font roots and where each moves to, longest prefix first.
    const REMAPS: [(&str, &str); 3] = [
        ("lyshineui/fonts/", "ui/fonts/"),
        ("fonts/", "ui/fonts/"),
        ("lyshineui/", "ui/lyshineui/"),
    ];

    let normalized = normalize_source_path(source_path);
    for (prefix, replacement) in REMAPS {
        if let Some(rest) = normalized.strip_prefix(prefix) {
            return format!("{replacement}{rest}");
        }
    }
    normalized
}

#[derive(Debug, thiserror::Error)]
pub enum FontFaceSourceTransformError {
    #[error("parse font face {path:?}")]
    Parse {
        path: String,
        #[source]
        source: OpenTypeParseError,
    },
}
