//! Legacy/import source transform contract.
//!
//! Build rules cook Azoth source into products. Source transforms are the
//! earlier, one-time import step: they classify legacy bytes and, where the
//! format is understood, rewrite them into native authoring source.

use az_filesystem::SourcePath;

use crate::value::SourceSchemaType;

/// Legacy bytes plus their logical asset path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySourceInput<'a> {
    pub source_path: SourcePath,
    pub bytes: &'a [u8],
}

impl<'a> LegacySourceInput<'a> {
    #[must_use]
    pub fn new(source_path: impl AsRef<str>, bytes: &'a [u8]) -> Self {
        Self {
            source_path: SourcePath::new(source_path),
            bytes,
        }
    }
}

/// File-like output emitted by a legacy source transform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySourceArtifact {
    pub path: String,
    pub schema: SourceSchemaType,
    pub bytes: Vec<u8>,
}

impl LegacySourceArtifact {
    #[must_use]
    pub fn new(path: impl AsRef<str>, schema: SourceSchemaType, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            path: SourcePath::new(path).into_string(),
            schema,
            bytes: bytes.into(),
        }
    }
}

/// Result of classifying and transforming one legacy source payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacySourceOutput {
    /// Native source that the editor/dev workflow may edit after import.
    AuthoringSource(LegacySourceArtifact),
    /// Product bytes whose true editable source is missing or external.
    ExternalProductPayload(LegacySourceArtifact),
    /// Compatibility evidence kept for provenance/catalog analysis.
    CompatibilityEvidence(LegacySourceArtifact),
    /// Build configuration generated beside an emitted authoring source.
    ///
    /// The importer authors these bytes; the legacy payload carries no
    /// counterpart for them and the shipping catalog holds no entry for their
    /// path. They are therefore never the payload's own source representation
    /// and never inherit its preserved catalog identity.
    GeneratedConfiguration(LegacySourceArtifact),
    /// Known source that the native runtime deterministically refuses to load.
    ///
    /// No authoring artifact is emitted because doing so would invent runtime
    /// behavior that the packaged source cannot produce.
    RuntimeUnavailable { reason: String },
    /// Intentionally skipped because the payload is not source.
    Excluded { reason: String },
    /// Known legacy payload whose owner has not classified it yet.
    Unclassified { reason: String },
}

impl LegacySourceOutput {
    #[must_use]
    pub fn authoring_source(
        path: impl AsRef<str>,
        schema: SourceSchemaType,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        Self::AuthoringSource(LegacySourceArtifact::new(path, schema, bytes))
    }

    #[must_use]
    pub fn external_product_payload(
        path: impl AsRef<str>,
        schema: SourceSchemaType,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        Self::ExternalProductPayload(LegacySourceArtifact::new(path, schema, bytes))
    }

    #[must_use]
    pub fn compatibility_evidence(
        path: impl AsRef<str>,
        schema: SourceSchemaType,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        Self::CompatibilityEvidence(LegacySourceArtifact::new(path, schema, bytes))
    }

    #[must_use]
    pub fn generated_configuration(
        path: impl AsRef<str>,
        schema: SourceSchemaType,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        Self::GeneratedConfiguration(LegacySourceArtifact::new(path, schema, bytes))
    }

    #[must_use]
    pub fn runtime_unavailable(reason: impl Into<String>) -> Self {
        Self::RuntimeUnavailable {
            reason: reason.into(),
        }
    }

    #[must_use]
    pub const fn artifact(&self) -> Option<&LegacySourceArtifact> {
        match self {
            Self::AuthoringSource(artifact)
            | Self::ExternalProductPayload(artifact)
            | Self::CompatibilityEvidence(artifact)
            | Self::GeneratedConfiguration(artifact) => Some(artifact),
            Self::RuntimeUnavailable { .. } | Self::Excluded { .. } | Self::Unclassified { .. } => {
                None
            }
        }
    }

    /// Whether this output stands in for the legacy payload itself and so
    /// inherits the payload's preserved catalog identity.
    ///
    /// A legacy payload owns exactly one catalog identity, so at most one
    /// output per payload may report `true`. Anything the importer generates
    /// alongside that output reports `false` and is identified by its own
    /// path instead.
    #[must_use]
    pub const fn carries_native_identity(&self) -> bool {
        match self {
            Self::AuthoringSource(_)
            | Self::ExternalProductPayload(_)
            | Self::CompatibilityEvidence(_) => true,
            Self::GeneratedConfiguration(_)
            | Self::RuntimeUnavailable { .. }
            | Self::Excluded { .. }
            | Self::Unclassified { .. } => false,
        }
    }
}

/// Implemented by the crate that owns a legacy format.
pub trait LegacySourceTransform {
    type Error;

    /// # Errors
    ///
    /// Propagates the implementing transform's error type when classification or
    /// conversion fails.
    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: SourceSchemaType = SourceSchemaType::__from_static("test.Schema");

    #[test]
    fn only_payload_representations_carry_the_native_identity() {
        assert!(
            LegacySourceOutput::authoring_source("a.ron", SCHEMA, [1]).carries_native_identity()
        );
        assert!(
            LegacySourceOutput::external_product_payload("a.bin", SCHEMA, [1])
                .carries_native_identity()
        );
        assert!(
            LegacySourceOutput::compatibility_evidence("a.xml", SCHEMA, [1])
                .carries_native_identity()
        );
        assert!(
            !LegacySourceOutput::generated_configuration("a.texture.ron", SCHEMA, [1])
                .carries_native_identity()
        );
        assert!(!LegacySourceOutput::runtime_unavailable("nope").carries_native_identity());
    }

    #[test]
    fn generated_configuration_is_still_a_file_to_publish() {
        let output = LegacySourceOutput::generated_configuration("a.texture.ron", SCHEMA, [7]);
        let artifact = output
            .artifact()
            .expect("generated configuration is file-like");
        assert_eq!(artifact.path, "a.texture.ron");
        assert_eq!(artifact.bytes, vec![7]);
    }
}
