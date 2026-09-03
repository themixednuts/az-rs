//! Cry/Lumberyard character source transforms.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceFormat,
    SourceSchemaRegistration, normalize_source_path, source_schema_type,
};
use ron::ser::PrettyConfig;
use serde::Serialize;

mod character_definition;
mod character_parameters;

pub(crate) use az_xml::{xml_cdata_content, xml_general_reference_content, xml_text_content};

pub use az_animation::character::definition::{
    AttachmentBinding, AttachmentFlags, AttachmentMaterials, AttachmentTransform, BoneAttachment,
    CharacterAttachment, CharacterAttachmentKind, CharacterDefinition, CharacterDefinitionSource,
    CharacterMirroring, ClothAttachment, ClothCollisionAttachment, ClothJointPhysics,
    FaceAttachment, JointPhysics, MirroringAxis, PendulumRowAttachment, ProxyAttachment,
    RelativeAttachmentTransform, RopeJointPhysics, RowConstraint, RowSimulation, SkinAttachment,
    SocketConstraint, SocketSimulation,
};
pub use character_definition::{
    CharacterDefinitionParseError, CharacterDefinitionSourceExt,
    is_character_definition_source_path,
};
pub use character_parameters::{
    CharacterAimIkSource, CharacterAnimationDirectiveSource,
    CharacterAnimationDrivenIkTargetListSource, CharacterAnimationDrivenIkTargetSource,
    CharacterAnimationFilterFolderSource, CharacterAnimationSetFilterSource,
    CharacterAnimationWildcardSource, CharacterBoundingBoxExtensionSource,
    CharacterBoundingBoxIncludesSource, CharacterDirectionalBlendSource, CharacterFeetLockIkSource,
    CharacterIkDefinitionSource, CharacterIkPositionSource, CharacterIkRotationSource,
    CharacterImpactJointSource, CharacterJointLodSource, CharacterLimbIkEntrySource,
    CharacterLimbIkSolverSource, CharacterLimbIkSource, CharacterLookIkSource,
    CharacterParametersDbaSource, CharacterParametersIncludeSource,
    CharacterParametersLegacyNodeSource, CharacterParametersParseError, CharacterParametersSource,
    CharacterRecoilIkSource, CharacterVector3Source, is_character_parameters_source_path,
};

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.CharacterParametersSource",
    ext = "chrparams.ron"
)]
pub struct CharacterParametersSourceFormat;

pub mod source_schemas {
    use super::{CharacterParametersSourceFormat, source_schema_type};
    use az_asset_builder::SourceSchemaType;

    pub const CHARACTER_DEFINITION: SourceSchemaType = source_schema_type::<
        az_animation::character::definition::CharacterDefinitionSourceFormat,
    >();
    pub const CHARACTER_PARAMETERS: SourceSchemaType =
        source_schema_type::<CharacterParametersSourceFormat>();
}

/// The source schemas this crate owns, for a host contribution to register.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 1] {
    [
        SourceSchemaRegistration::for_source::<CharacterParametersSourceFormat>()
            .with_category("Cry/Lumberyard Compatibility")
            .with_import_file("characters", &["chrparams.ron"]),
    ]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharacterSourceKind {
    Definition,
    Parameters,
}

impl CharacterSourceKind {
    fn from_path(source_path: &str) -> Option<Self> {
        let normalized = normalize_source_path(source_path);
        if is_character_definition_source_path(&normalized) {
            Some(Self::Definition)
        } else if is_character_parameters_source_path(&normalized) {
            Some(Self::Parameters)
        } else {
            None
        }
    }

    const fn source_schema(self) -> az_asset_builder::SourceSchemaType {
        match self {
            Self::Definition => source_schemas::CHARACTER_DEFINITION,
            Self::Parameters => source_schemas::CHARACTER_PARAMETERS,
        }
    }

    const fn source_suffix(self) -> &'static str {
        match self {
            Self::Definition => "character.ron",
            Self::Parameters => "chrparams.ron",
        }
    }

    fn source_path(self, source_path: &str) -> String {
        source_path_with_suffix(source_path, self.source_suffix())
    }

    fn to_ron_bytes(
        self,
        source_path: &str,
        bytes: &[u8],
    ) -> Result<Vec<u8>, CharacterSourceTransformError> {
        match self {
            Self::Definition => {
                Ok(CharacterDefinitionSource::from_legacy(source_path, bytes)?.to_ron_bytes()?)
            }
            Self::Parameters => {
                Ok(CharacterParametersSource::from_legacy(source_path, bytes)?.to_ron_bytes()?)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CharacterSourceTransform;

impl LegacySourceTransform for CharacterSourceTransform {
    type Error = CharacterSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let kind = CharacterSourceKind::from_path(&input.source_path).ok_or_else(|| {
            CharacterSourceTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            }
        })?;

        Ok(LegacySourceOutput::authoring_source(
            kind.source_path(&input.source_path),
            kind.source_schema(),
            kind.to_ron_bytes(&input.source_path, input.bytes)?,
        ))
    }
}

pub type CharacterDefinitionSourceTransform = CharacterSourceTransform;
pub type CharacterParametersSourceTransform = CharacterSourceTransform;

#[must_use]
pub fn is_legacy_character_source(source_path: &str) -> bool {
    CharacterSourceKind::from_path(source_path).is_some()
}

#[must_use]
pub fn is_legacy_character_definition_source(source_path: &str) -> bool {
    matches!(
        CharacterSourceKind::from_path(source_path),
        Some(CharacterSourceKind::Definition)
    )
}

#[must_use]
pub fn is_legacy_character_parameters_source(source_path: &str) -> bool {
    matches!(
        CharacterSourceKind::from_path(source_path),
        Some(CharacterSourceKind::Parameters)
    )
}

#[must_use]
pub fn character_source_path(source_path: &str) -> Option<String> {
    CharacterSourceKind::from_path(source_path).map(|kind| kind.source_path(source_path))
}

#[must_use]
pub fn character_definition_source_path(source_path: &str) -> Option<String> {
    matches!(
        CharacterSourceKind::from_path(source_path),
        Some(CharacterSourceKind::Definition)
    )
    .then(|| source_path_with_suffix(source_path, "character.ron"))
}

#[must_use]
pub fn character_parameters_source_path(source_path: &str) -> Option<String> {
    matches!(
        CharacterSourceKind::from_path(source_path),
        Some(CharacterSourceKind::Parameters)
    )
    .then(|| source_path_with_suffix(source_path, "chrparams.ron"))
}

fn source_path_with_suffix(source_path: &str, suffix: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized
        .strip_suffix(".cdf")
        .or_else(|| normalized.strip_suffix(".chrparams"))
        .unwrap_or(&normalized);
    format!("{stem}.{suffix}")
}

fn to_ron_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, ron::Error> {
    let ron = ron::ser::to_string_pretty(value, PrettyConfig::default())?;
    Ok(format!("{ron}\n").into_bytes())
}

#[derive(Debug, thiserror::Error)]
pub enum CharacterSourceTransformError {
    #[error("unsupported character XML path {path}")]
    UnsupportedPath { path: String },
    #[error("parse character definition XML source: {0}")]
    CharacterDefinition(#[from] CharacterDefinitionParseError),
    #[error("parse character parameters XML source: {0}")]
    CharacterParameters(#[from] CharacterParametersParseError),
    #[error("serialize character XML source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[cfg(test)]
mod tests;
