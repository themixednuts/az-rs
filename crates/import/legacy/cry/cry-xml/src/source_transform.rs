//! Source transforms for typed Cry/Lumberyard XML-backed assets.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, normalize_source_path,
};
use ron::ser::PrettyConfig;
use serde::Serialize;

use crate::{XmlAssetKind, source_schemas};

mod level_info;
mod material_effects;
mod material_override;
mod particle_library;
mod post_effect;
mod time_of_day;

pub use level_info::{
    LevelInfoParseError, LevelInfoSource, LevelMissionSource, LevelTerrainInfoSource,
};
pub use material_effects::{
    MaterialEffectAudioSource, MaterialEffectAudioSwitchSource, MaterialEffectDecalSource,
    MaterialEffectFilterSource, MaterialEffectForceFeedbackSource,
    MaterialEffectParticleDirectionSource, MaterialEffectParticleNameSource,
    MaterialEffectParticleSource, MaterialEffectRandomSource, MaterialEffectReferenceSource,
    MaterialEffectResourceSource, MaterialEffectSource, MaterialEffectsInteractionAxisEntrySource,
    MaterialEffectsInteractionCellSource, MaterialEffectsInteractionIndexSource,
    MaterialEffectsInteractionRowKindSource, MaterialEffectsInteractionRowSource,
    MaterialEffectsLibrarySource, MaterialEffectsParseError, MaterialEffectsSource,
    MaterialEffectsSpreadsheetCellMetadataSource,
};
pub use material_override::{
    MaterialOverrideAttributeSource, MaterialOverrideMaterialSource,
    MaterialOverrideMaxTriggerDistanceSource, MaterialOverrideNodeSource,
    MaterialOverrideParamSource, MaterialOverrideParseError, MaterialOverrideSource,
    MaterialOverrideSubMaterialSource,
};
pub use particle_library::{
    ParticleAttributeSource, ParticleEffectSource, ParticleExtraNodeSource,
    ParticleLibraryFolderSource, ParticleLibraryParseError, ParticleLibrarySettingsSource,
    ParticleLibrarySource, ParticleLodLevelSource, ParticleLodParticleSource, ParticleLodsSource,
    ParticleParamBagSource,
};
pub use post_effect::{
    ColorRgbaSource, PostEffectBlendCurve, PostEffectBlendSource, PostEffectColorParamValueSource,
    PostEffectEffectSource, PostEffectFloatParamValueSource, PostEffectGroupParseError,
    PostEffectGroupSource, PostEffectKeySource, PostEffectParamSource, PostEffectParamValueSource,
    PostEffectStringParamValueSource, PostEffectTextureParamValueSource,
    PostEffectUnknownBlendCurveSource, PostEffectVec4ParamValueSource, Vec4Source,
};
pub use time_of_day::{
    ColorRgbSource, SplineKeyFlagsSource, SplineTangentSource, SplineTangentUnknownSource,
    TimeOfDayColorValueSource, TimeOfDayFloatValueSource, TimeOfDayParseError,
    TimeOfDayProfileSource, TimeOfDaySplineKeySource, TimeOfDaySplineSource, TimeOfDayValueSource,
    TimeOfDayVariableSource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XmlSourceKind {
    LevelInfo,
    MaterialEffects,
    MaterialOverride,
    ParticleLibrary,
    TimeOfDay,
    PostEffectGroup,
}

impl XmlSourceKind {
    fn from_path(source_path: &str) -> Option<Self> {
        match XmlAssetKind::from_path(source_path) {
            XmlAssetKind::LevelInfo => Some(Self::LevelInfo),
            XmlAssetKind::MaterialEffects => Some(Self::MaterialEffects),
            XmlAssetKind::MaterialOverride => Some(Self::MaterialOverride),
            XmlAssetKind::ParticleLibrary => Some(Self::ParticleLibrary),
            XmlAssetKind::TimeOfDay => Some(Self::TimeOfDay),
            XmlAssetKind::PostEffectGroup => Some(Self::PostEffectGroup),
            _ => None,
        }
    }

    const fn source_schema(self) -> az_asset_builder::SourceSchemaType {
        match self {
            Self::LevelInfo => source_schemas::LEVEL_INFO,
            Self::MaterialEffects => source_schemas::MATERIAL_EFFECTS,
            Self::MaterialOverride => source_schemas::MATERIAL_OVERRIDE,
            Self::ParticleLibrary => source_schemas::PARTICLE_LIBRARY,
            Self::TimeOfDay => source_schemas::TIME_OF_DAY,
            Self::PostEffectGroup => source_schemas::POST_EFFECT_GROUP,
        }
    }

    const fn source_suffix(self) -> &'static str {
        match self {
            Self::LevelInfo => "levelinfo.ron",
            Self::MaterialEffects => "materialeffects.ron",
            Self::MaterialOverride => "materialoverride.ron",
            Self::ParticleLibrary => "particle.ron",
            Self::TimeOfDay => "timeofday.ron",
            Self::PostEffectGroup => "posteffect.ron",
        }
    }

    fn source_path(self, source_path: &str) -> String {
        match self {
            Self::LevelInfo => {
                source_path_with_root(source_path, "levels/", "levels/", self.source_suffix())
            }
            Self::MaterialEffects => source_path_with_root(
                source_path,
                "libs/materialeffects/",
                "materials/effects/",
                self.source_suffix(),
            ),
            Self::MaterialOverride => source_path_with_root(
                source_path,
                "libs/materialoverrides/",
                "materials/effects/overrides/",
                self.source_suffix(),
            ),
            Self::ParticleLibrary => source_path_with_root(
                source_path,
                "libs/particles/",
                "particles/",
                self.source_suffix(),
            ),
            Self::TimeOfDay => source_path_with_root(
                source_path,
                "libs/timeofday/",
                "timeofday/",
                self.source_suffix(),
            ),
            Self::PostEffectGroup => source_path_with_root(
                source_path,
                "libs/posteffectgroups/",
                "posteffects/",
                self.source_suffix(),
            ),
        }
    }

    fn to_ron_bytes(
        self,
        source_path: &str,
        bytes: &[u8],
    ) -> Result<Vec<u8>, XmlSourceTransformError> {
        match self {
            Self::LevelInfo => {
                Ok(LevelInfoSource::from_legacy(source_path, bytes)?.to_ron_bytes()?)
            }
            Self::MaterialEffects => {
                Ok(MaterialEffectsSource::from_legacy(source_path, bytes)?.to_ron_bytes()?)
            }
            Self::MaterialOverride => {
                Ok(MaterialOverrideSource::from_legacy(source_path, bytes)?.to_ron_bytes()?)
            }
            Self::ParticleLibrary => {
                Ok(ParticleLibrarySource::from_legacy(source_path, bytes)?.to_ron_bytes()?)
            }
            Self::TimeOfDay => {
                Ok(TimeOfDayProfileSource::from_legacy(source_path, bytes)?.to_ron_bytes()?)
            }
            Self::PostEffectGroup => {
                Ok(PostEffectGroupSource::from_legacy(source_path, bytes)?.to_ron_bytes()?)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct XmlSourceTransform;

impl LegacySourceTransform for XmlSourceTransform {
    type Error = XmlSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let kind = XmlSourceKind::from_path(&input.source_path).ok_or_else(|| {
            XmlSourceTransformError::UnsupportedPath {
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

pub type PostEffectGroupSourceTransform = XmlSourceTransform;
pub type LevelInfoSourceTransform = XmlSourceTransform;
pub type TimeOfDayProfileSourceTransform = XmlSourceTransform;
pub type MaterialEffectsSourceTransform = XmlSourceTransform;
pub type MaterialOverrideSourceTransform = XmlSourceTransform;
pub type ParticleLibrarySourceTransform = XmlSourceTransform;

#[must_use]
pub fn is_legacy_xml_source(source_path: &str) -> bool {
    XmlSourceKind::from_path(source_path).is_some()
}

#[must_use]
pub fn xml_source_path(source_path: &str) -> Option<String> {
    XmlSourceKind::from_path(source_path).map(|kind| kind.source_path(source_path))
}

#[must_use]
pub fn level_info_source_path(source_path: &str) -> Option<String> {
    matches!(
        XmlSourceKind::from_path(source_path),
        Some(XmlSourceKind::LevelInfo)
    )
    .then(|| XmlSourceKind::LevelInfo.source_path(source_path))
}

#[must_use]
pub fn post_effect_group_source_path(source_path: &str) -> Option<String> {
    matches!(
        XmlSourceKind::from_path(source_path),
        Some(XmlSourceKind::PostEffectGroup)
    )
    .then(|| XmlSourceKind::PostEffectGroup.source_path(source_path))
}

#[must_use]
pub fn material_effects_source_path(source_path: &str) -> Option<String> {
    matches!(
        XmlSourceKind::from_path(source_path),
        Some(XmlSourceKind::MaterialEffects)
    )
    .then(|| XmlSourceKind::MaterialEffects.source_path(source_path))
}

#[must_use]
pub fn material_override_source_path(source_path: &str) -> Option<String> {
    matches!(
        XmlSourceKind::from_path(source_path),
        Some(XmlSourceKind::MaterialOverride)
    )
    .then(|| XmlSourceKind::MaterialOverride.source_path(source_path))
}

#[must_use]
pub fn particle_library_source_path(source_path: &str) -> Option<String> {
    matches!(
        XmlSourceKind::from_path(source_path),
        Some(XmlSourceKind::ParticleLibrary)
    )
    .then(|| XmlSourceKind::ParticleLibrary.source_path(source_path))
}

#[must_use]
pub fn time_of_day_source_path(source_path: &str) -> Option<String> {
    matches!(
        XmlSourceKind::from_path(source_path),
        Some(XmlSourceKind::TimeOfDay)
    )
    .then(|| XmlSourceKind::TimeOfDay.source_path(source_path))
}

fn source_path_with_root(
    source_path: &str,
    legacy_root: &str,
    target_root: &str,
    suffix: &str,
) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized.strip_suffix(".xml").unwrap_or(&normalized);
    let rest = stem.strip_prefix(legacy_root).unwrap_or(stem);
    format!("{target_root}{rest}.{suffix}")
}

fn to_ron_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, ron::Error> {
    let ron = ron::ser::to_string_pretty(value, PrettyConfig::default())?;
    Ok(format!("{ron}\n").into_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct XmlAttribute {
    name: String,
    value: String,
}

#[derive(Debug, thiserror::Error)]
pub enum XmlSourceTransformError {
    #[error("unsupported typed XML path {path}")]
    UnsupportedPath { path: String },
    #[error("parse typed XML source: {0}")]
    Parse(#[from] PostEffectGroupParseError),
    #[error("parse level-info XML source: {0}")]
    LevelInfo(#[from] LevelInfoParseError),
    #[error("parse material-effects XML source: {0}")]
    MaterialEffects(#[from] MaterialEffectsParseError),
    #[error("parse material-override XML source: {0}")]
    MaterialOverride(#[from] MaterialOverrideParseError),
    #[error("parse particle-library XML source: {0}")]
    ParticleLibrary(#[from] ParticleLibraryParseError),
    #[error("parse time-of-day XML source: {0}")]
    TimeOfDay(#[from] TimeOfDayParseError),
    #[error("serialize typed XML source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[cfg(test)]
mod tests;
