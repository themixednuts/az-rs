//! Cry/Lumberyard XML-backed source transforms.

use az_asset_builder::{SourceFormat, SourceSchemaRegistration, source_schema_type};
use std::{
    collections::BTreeMap,
    fmt, io,
    path::{Path, PathBuf},
};

pub mod source_transform;
pub mod surface_types;

pub(crate) use az_xml::{xml_cdata_content, xml_general_reference_content, xml_text_content};

pub use source_transform::{
    ColorRgbSource, ColorRgbaSource, LevelInfoSource, LevelInfoSourceTransform, LevelMissionSource,
    LevelTerrainInfoSource, MaterialEffectAudioSource, MaterialEffectAudioSwitchSource,
    MaterialEffectDecalSource, MaterialEffectFilterSource, MaterialEffectForceFeedbackSource,
    MaterialEffectParticleDirectionSource, MaterialEffectParticleNameSource,
    MaterialEffectParticleSource, MaterialEffectRandomSource, MaterialEffectReferenceSource,
    MaterialEffectResourceSource, MaterialEffectSource, MaterialEffectsInteractionAxisEntrySource,
    MaterialEffectsInteractionCellSource, MaterialEffectsInteractionIndexSource,
    MaterialEffectsInteractionRowKindSource, MaterialEffectsInteractionRowSource,
    MaterialEffectsLibrarySource, MaterialEffectsSource, MaterialEffectsSourceTransform,
    MaterialEffectsSpreadsheetCellMetadataSource, MaterialOverrideAttributeSource,
    MaterialOverrideMaterialSource, MaterialOverrideMaxTriggerDistanceSource,
    MaterialOverrideNodeSource, MaterialOverrideParamSource, MaterialOverrideSource,
    MaterialOverrideSourceTransform, MaterialOverrideSubMaterialSource, ParticleAttributeSource,
    ParticleEffectSource, ParticleExtraNodeSource, ParticleLibraryFolderSource,
    ParticleLibrarySettingsSource, ParticleLibrarySource, ParticleLibrarySourceTransform,
    ParticleLodLevelSource, ParticleLodParticleSource, ParticleLodsSource, ParticleParamBagSource,
    PostEffectBlendCurve, PostEffectBlendSource, PostEffectColorParamValueSource,
    PostEffectEffectSource, PostEffectFloatParamValueSource, PostEffectGroupSource,
    PostEffectGroupSourceTransform, PostEffectKeySource, PostEffectParamSource,
    PostEffectParamValueSource, PostEffectStringParamValueSource,
    PostEffectTextureParamValueSource, PostEffectUnknownBlendCurveSource,
    PostEffectVec4ParamValueSource, SplineKeyFlagsSource, SplineTangentSource,
    SplineTangentUnknownSource, TimeOfDayColorValueSource, TimeOfDayFloatValueSource,
    TimeOfDayProfileSource, TimeOfDayProfileSourceTransform, TimeOfDaySplineKeySource,
    TimeOfDaySplineSource, TimeOfDayValueSource, TimeOfDayVariableSource, Vec4Source,
    XmlSourceTransform, XmlSourceTransformError, is_legacy_xml_source, level_info_source_path,
    material_effects_source_path, material_override_source_path, particle_library_source_path,
    post_effect_group_source_path, time_of_day_source_path, xml_source_path,
};
pub use surface_types::{
    SurfaceTypeSourceEntry, SurfaceTypesParseError, SurfaceTypesSource,
    SurfaceTypesSourceTransform, SurfaceTypesSourceTransformError, is_legacy_surface_types_source,
    surface_types_source_path,
};

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.MaterialEffectsSource",
    ext = "materialeffects.ron"
)]
pub struct MaterialEffectsSourceFormat;

#[derive(SourceFormat)]
#[source(schema = "azoth.compat.cry.LevelInfoSource", ext = "levelinfo.ron")]
pub struct LevelInfoSourceFormat;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.MaterialOverrideSource",
    ext = "materialoverride.ron"
)]
pub struct MaterialOverrideSourceFormat;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.ParticleLibrarySource",
    ext = "particle.ron"
)]
pub struct ParticleLibrarySourceFormat;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.PostEffectGroupSource",
    ext = "posteffect.ron"
)]
pub struct PostEffectGroupSourceFormat;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.SurfaceTypesSource",
    ext = "surfacetypes.ron"
)]
pub struct SurfaceTypesSourceFormat;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.TimeOfDayProfileSource",
    ext = "timeofday.ron"
)]
pub struct TimeOfDayProfileSourceFormat;

pub mod source_schemas {
    use super::{
        LevelInfoSourceFormat, MaterialEffectsSourceFormat, MaterialOverrideSourceFormat,
        ParticleLibrarySourceFormat, PostEffectGroupSourceFormat, SurfaceTypesSourceFormat,
        TimeOfDayProfileSourceFormat, source_schema_type,
    };
    use az_asset_builder::SourceSchemaType;

    pub const MATERIAL_EFFECTS: SourceSchemaType =
        source_schema_type::<MaterialEffectsSourceFormat>();
    pub const LEVEL_INFO: SourceSchemaType = source_schema_type::<LevelInfoSourceFormat>();
    pub const MATERIAL_OVERRIDE: SourceSchemaType =
        source_schema_type::<MaterialOverrideSourceFormat>();
    pub const PARTICLE_LIBRARY: SourceSchemaType =
        source_schema_type::<ParticleLibrarySourceFormat>();
    pub const POST_EFFECT_GROUP: SourceSchemaType =
        source_schema_type::<PostEffectGroupSourceFormat>();
    pub const SURFACE_TYPES: SourceSchemaType = source_schema_type::<SurfaceTypesSourceFormat>();
    pub const TIME_OF_DAY: SourceSchemaType = source_schema_type::<TimeOfDayProfileSourceFormat>();
}

/// The source schemas this crate owns, for a host contribution to register.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 7] {
    [
        SourceSchemaRegistration::for_source::<LevelInfoSourceFormat>()
            .with_category("Cry/Lumberyard Compatibility")
            .with_editable_file("levels", &["levelinfo.ron"]),
        SourceSchemaRegistration::for_source::<MaterialEffectsSourceFormat>()
            .with_category("Cry/Lumberyard Compatibility")
            .with_editable_file("libs/materialeffects", &["materialeffects.ron"]),
        SourceSchemaRegistration::for_source::<MaterialOverrideSourceFormat>()
            .with_category("Cry/Lumberyard Compatibility")
            .with_editable_file("libs/materialoverrides", &["materialoverride.ron"]),
        SourceSchemaRegistration::for_source::<ParticleLibrarySourceFormat>()
            .with_category("Cry/Lumberyard Compatibility")
            .with_editable_file("libs/particles", &["particle.ron"]),
        SourceSchemaRegistration::for_source::<PostEffectGroupSourceFormat>()
            .with_category("Cry/Lumberyard Compatibility")
            .with_editable_file("libs/posteffectgroups", &["posteffect.ron"]),
        SourceSchemaRegistration::for_source::<SurfaceTypesSourceFormat>()
            .with_category("Cry/Lumberyard Compatibility")
            .with_editable_file("libs/materialeffects", &["surfacetypes.ron"]),
        SourceSchemaRegistration::for_source::<TimeOfDayProfileSourceFormat>()
            .with_category("Cry/Lumberyard Compatibility")
            .with_editable_file("libs/timeofday", &["timeofday.ron"]),
    ]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
}

pub const XML_EXTENSION: &str = "xml";
pub const CDF_EXTENSION: &str = "cdf";
pub const CHR_PARAMS_EXTENSION: &str = "chrparams";
pub const XML_BACKED_EXTENSIONS: &[&str] = &[XML_EXTENSION, CDF_EXTENSION, CHR_PARAMS_EXTENSION];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum XmlAssetKind {
    LevelData,
    LevelInfo,
    LevelParticles,
    LevelDataAction,
    LensFlareList,
    MovieData,
    Mission,
    MaterialEffects,
    MaterialOverride,
    ParticleLibrary,
    MergedMeshSurfaceTypes,
    MannequinControllerDefinitions,
    MannequinActions,
    MannequinTags,
    CharacterDefinition,
    CharacterParameters,
    TimeOfDay,
    PostEffectGroup,
    LocalizationConfig,
    Serialize,
    DynamicTextureSourceLayer,
    PlainXml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XmlSummary {
    pub kind: XmlAssetKind,
    pub stats: az_xml::XmlStats,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct XmlTotals {
    pub files: usize,
    pub stats: az_xml::XmlStats,
    pub kinds: BTreeMap<XmlAssetKind, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlFileSummary {
    pub source: String,
    pub summary: XmlSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct XmlInspection {
    pub rows: Vec<XmlFileSummary>,
    pub totals: XmlTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct XmlInspectionReport<'a> {
    inspection: &'a XmlInspection,
    limit: usize,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum XmlInspectionError {
    #[error("failed to read {path:?}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to parse XML asset {path:?}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: az_xml::ParseError,
    },
}

impl Default for XmlSummary {
    fn default() -> Self {
        Self {
            kind: XmlAssetKind::PlainXml,
            stats: az_xml::XmlStats::default(),
        }
    }
}

impl fmt::Display for XmlSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({} elements, depth {})",
            self.kind.as_str(),
            self.stats.elements,
            self.stats.max_depth
        )
    }
}

impl XmlTotals {
    pub fn add_summary(&mut self, summary: XmlSummary) {
        self.files += 1;
        self.stats.add_assign(summary.stats);
        *self.kinds.entry(summary.kind).or_default() += 1;
    }
}

impl XmlInspection {
    pub fn add_file_summary(&mut self, row: XmlFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> XmlInspectionReport<'_> {
        XmlInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for XmlTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  elements: {}", self.stats.elements)?;
        writeln!(f, "  attributes: {}", self.stats.attributes)?;
        writeln!(f, "  text nodes: {}", self.stats.text_nodes)?;
        writeln!(f, "  cdata nodes: {}", self.stats.cdata_nodes)?;
        writeln!(f, "  comments: {}", self.stats.comments)?;
        writeln!(
            f,
            "  processing instructions: {}",
            self.stats.processing_instructions
        )?;
        writeln!(f, "  declarations: {}", self.stats.declarations)?;
        writeln!(f, "  doctypes: {}", self.stats.doctypes)?;
        writeln!(f, "  max depth: {}", self.stats.max_depth)?;
        writeln!(
            f,
            "  recovered unmatched end tags: {}",
            self.stats.recovered_unmatched_ends
        )?;
        for (kind, files) in &self.kinds {
            writeln!(f, "  {}: {}", kind.as_str(), files)?;
        }
        Ok(())
    }
}

impl fmt::Display for XmlInspectionReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.limit > 0 {
            for row in self.inspection.rows.iter().take(self.limit) {
                writeln!(f, "{}: {}", row.source, row.summary)?;
            }

            if self.inspection.rows.len() > self.limit {
                writeln!(
                    f,
                    "... {} more files",
                    self.inspection.rows.len() - self.limit
                )?;
            }
        }

        write!(f, "{}", self.inspection.totals)
    }
}

impl XmlAssetKind {
    #[must_use]
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LevelData => "Cry3DEngine level data",
            Self::LevelInfo => "level info",
            Self::LevelParticles => "level particles",
            Self::LevelDataAction => "level action data",
            Self::LensFlareList => "lens flare list",
            Self::MovieData => "movie data",
            Self::Mission => "mission",
            Self::MaterialEffects => "material effects",
            Self::MaterialOverride => "material override",
            Self::ParticleLibrary => "particle library",
            Self::MergedMeshSurfaceTypes => "merged mesh surface types",
            Self::MannequinControllerDefinitions => "Mannequin controller definitions",
            Self::MannequinActions => "Mannequin actions",
            Self::MannequinTags => "Mannequin tags",
            Self::CharacterDefinition => "character definition",
            Self::CharacterParameters => "character parameters",
            Self::TimeOfDay => "time of day",
            Self::PostEffectGroup => "post effect group",
            Self::LocalizationConfig => "localization config",
            Self::Serialize => "serialize metadata",
            Self::DynamicTextureSourceLayer => "dynamic texture source layer",
            Self::PlainXml => "XML document",
        }
    }

    #[must_use]
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        let normalized = normalize_path(path.as_ref());
        let name = normalized.rsplit('/').next().unwrap_or(normalized.as_str());

        match name {
            "leveldata.xml" => Self::LevelData,
            "levelinfo.xml" => Self::LevelInfo,
            "levelparticles.xml" => Self::LevelParticles,
            "leveldataaction.xml" => Self::LevelDataAction,
            "lensflarelist.xml" => Self::LensFlareList,
            "moviedata.xml" => Self::MovieData,
            "materialeffects.xml" => Self::MaterialEffects,
            "mergedmeshsurfacetypes.xml" => Self::MergedMeshSurfaceTypes,
            "tod.xml" | "timeofday.xml" => Self::TimeOfDay,
            "localization.xml" => Self::LocalizationConfig,
            "serialize.xml" => Self::Serialize,
            "dyntexsrclayeract.xml" => Self::DynamicTextureSourceLayer,
            _ if normalized.starts_with("libs/timeofday/") && has_extension(name, "xml") => {
                Self::TimeOfDay
            }
            _ if normalized.starts_with("libs/posteffectgroups/") && has_extension(name, "xml") => {
                Self::PostEffectGroup
            }
            _ if normalized.starts_with("libs/materialeffects/fxlibs/")
                && has_extension(name, "xml") =>
            {
                Self::MaterialEffects
            }
            _ if normalized.starts_with("libs/materialoverrides/")
                && has_extension(name, "xml") =>
            {
                Self::MaterialOverride
            }
            _ if normalized.starts_with("libs/particles/") && has_extension(name, "xml") => {
                Self::ParticleLibrary
            }
            _ if name.starts_with("mission_") => Self::Mission,
            _ if name.ends_with("_controllerdefs.xml") || name == "controllerdefs.xml" => {
                Self::MannequinControllerDefinitions
            }
            _ if name.ends_with("_actions.xml") || name == "actions.xml" => Self::MannequinActions,
            _ if name.ends_with("_tags.xml")
                || (name == "tags.xml" && normalized.contains("/mannequin/")) =>
            {
                Self::MannequinTags
            }
            _ if has_extension(name, "cdf") => Self::CharacterDefinition,
            _ if name.ends_with(".chrparams") => Self::CharacterParameters,
            _ => Self::PlainXml,
        }
    }
}

/// Classify `path` and collect node statistics from its XML payload.
///
/// # Errors
///
/// Returns any [`az_xml::ParseError`] [`az_xml::XmlDocument::parse_bytes`]
/// reports for malformed or non-XML `bytes`.
pub fn summarize_xml_path(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<XmlSummary, az_xml::ParseError> {
    let kind = XmlAssetKind::from_path(path);
    let stats = az_xml::XmlDocument::parse_bytes(bytes)?.stats();
    Ok(XmlSummary { kind, stats })
}

/// Summarize `bytes` into a one-row inspection record naming `path`.
///
/// # Errors
///
/// Returns any error [`summarize_xml_path`] returns.
pub fn inspect_xml_path(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<XmlFileSummary, az_xml::ParseError> {
    let path = path.as_ref();
    Ok(XmlFileSummary {
        source: path.display().to_string(),
        summary: summarize_xml_path(path, bytes)?,
    })
}

/// Read `path` from disk and inspect it as an XML-backed asset.
///
/// # Errors
///
/// Returns [`XmlInspectionError::Read`] when `path` cannot be read, or
/// [`XmlInspectionError::Parse`] when its bytes are not well-formed XML.
pub fn inspect_xml_file(path: impl AsRef<Path>) -> Result<XmlFileSummary, XmlInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| XmlInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_xml_path(path, &bytes).map_err(|source| XmlInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Inspect every path in `paths`, accumulating per-kind totals.
///
/// # Errors
///
/// Returns the first error [`inspect_xml_file`] returns; remaining paths are
/// not visited.
pub fn inspect_xml_files<I, P>(paths: I) -> Result<XmlInspection, XmlInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = XmlInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_xml_file(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub fn is_xml_backed_extension(extension: &str) -> bool {
    XML_BACKED_EXTENSIONS
        .iter()
        .any(|expected| extension.eq_ignore_ascii_case(expected))
}

#[must_use]
pub fn is_xml_backed_name(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_xml_backed_extension)
}

#[must_use]
pub fn is_xml_backed_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_xml_backed_extension)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

/// Case-insensitive extension test for an already-normalized file name.
pub(crate) fn has_extension(name: &str, expected: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::{SourceSchemaType, composed_source_schemas};
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, Registries, declare_caps,
    };

    declare_caps!(XmlCaps:);

    const OWNER: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.cry-xml-tests"),
        contribution: ContributionId::new("assets"),
        roles: &[],
    };

    struct Xml;

    impl Contribution for Xml {
        type Caps = XmlCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            OWNER
        }

        fn register(&self, ctx: &mut GemContext<'_, XmlCaps>) {
            super::register(ctx);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Xml, ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    fn schema(
        registries: &Registries,
        schema_type: SourceSchemaType,
    ) -> az_gem_contract::Attributed<az_asset_builder::SourceSchemaRegistration> {
        composed_source_schemas(registries)
            .into_iter()
            .find(|attributed| attributed.entry.schema_type() == schema_type)
            .unwrap_or_else(|| panic!("{schema_type} is composed"))
    }

    #[test]
    fn surface_types_source_schema_is_attributed_to_this_crate() {
        let composer = composed();
        let attributed = schema(composer.registries(), source_schemas::SURFACE_TYPES);

        assert_eq!(attributed.instance.gem.as_str(), "azoth.cry-xml-tests");
        assert_eq!(attributed.instance.contribution.as_str(), "assets");
        let az_asset_builder::SourceSchemaAuthoring::File { workflow } =
            attributed.entry.authoring()
        else {
            panic!("surface types source schema should be file-backed");
        };
        assert_eq!(workflow.default_path_prefix(), "libs/materialeffects");
        assert_eq!(workflow.extensions(), &["surfacetypes.ron"]);
    }

    #[test]
    fn xml_source_schemas_register_editable_file_workflows() {
        let composer = composed();
        let registries = composer.registries();

        for (schema_type, prefix, extensions) in [
            (source_schemas::LEVEL_INFO, "levels", &["levelinfo.ron"][..]),
            (
                source_schemas::PARTICLE_LIBRARY,
                "libs/particles",
                &["particle.ron"][..],
            ),
            (
                source_schemas::POST_EFFECT_GROUP,
                "libs/posteffectgroups",
                &["posteffect.ron"][..],
            ),
            (
                source_schemas::TIME_OF_DAY,
                "libs/timeofday",
                &["timeofday.ron"][..],
            ),
        ] {
            let attributed = schema(registries, schema_type);
            let az_asset_builder::SourceSchemaAuthoring::File { workflow } =
                attributed.entry.authoring()
            else {
                panic!("{schema_type} should be file-backed");
            };
            assert_eq!(workflow.default_path_prefix(), prefix);
            assert_eq!(workflow.extensions(), extensions);
            assert!(
                workflow.can_edit(),
                "{schema_type} should be editor-editable"
            );
            assert!(
                !workflow.can_create(),
                "{schema_type} is imported, not templated"
            );
        }
    }

    /// A host sees exactly the schemas its own composition registered — every
    /// schema this crate declares and nothing else.
    #[test]
    fn composition_registers_every_declared_schema() {
        let composer = composed();
        let mut composed_types = composed_source_schemas(composer.registries())
            .into_iter()
            .map(|attributed| attributed.entry.schema_type().to_string())
            .collect::<Vec<_>>();
        composed_types.sort();

        let mut declared = source_schemas()
            .into_iter()
            .map(|registration| registration.schema_type().to_string())
            .collect::<Vec<_>>();
        declared.sort();

        assert_eq!(composed_types, declared);
        assert_eq!(declared.len(), 7);
    }

    #[test]
    fn every_composed_schema_is_attributed_to_its_contribution() {
        let report = composed().finalize().expect("composition is valid");
        let entries = report
            .entries
            .iter()
            .filter(|entry| entry.registry == "source-schema")
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), source_schemas().len());
        assert!(entries.iter().all(|entry| {
            entry.instance.gem.as_str() == "azoth.cry-xml-tests"
                && entry.instance.contribution.as_str() == "assets"
        }));
    }

    #[test]
    fn classifies_known_asset_paths() {
        assert_eq!(
            XmlAssetKind::from_path("levels/foo/leveldata.xml"),
            XmlAssetKind::LevelData
        );
        assert_eq!(
            XmlAssetKind::from_path("levels/foo/mission_mission0.xml"),
            XmlAssetKind::Mission
        );
        assert_eq!(
            XmlAssetKind::from_path("animations/mannequin/human/human_controllerdefs.xml"),
            XmlAssetKind::MannequinControllerDefinitions
        );
        assert_eq!(
            XmlAssetKind::from_path("animations/mannequin/human/human_actions.xml"),
            XmlAssetKind::MannequinActions
        );
        assert_eq!(
            XmlAssetKind::from_path("libs/materialeffects/materialeffects.xml"),
            XmlAssetKind::MaterialEffects
        );
        assert_eq!(
            XmlAssetKind::from_path("libs/materialoverrides/death_dissolve_1.xml"),
            XmlAssetKind::MaterialOverride
        );
        assert_eq!(
            XmlAssetKind::from_path("libs/particles/cfx_ai_recolors.xml"),
            XmlAssetKind::ParticleLibrary
        );
        assert_eq!(
            XmlAssetKind::from_path("libs/particles/shared/noise.dds"),
            XmlAssetKind::PlainXml
        );
        assert_eq!(
            XmlAssetKind::from_path("libs/posteffectgroups/default.xml"),
            XmlAssetKind::PostEffectGroup
        );
    }

    #[test]
    fn summarizes_xml_assets_and_paths() {
        let path = "levels/foo/leveldata.xml";
        let bytes = br"<Level><A/></Level>";
        let summary = summarize_xml_path(path, bytes).expect("summarize xml");
        let mut totals = XmlTotals::default();
        totals.add_summary(summary);

        assert_eq!(summary.kind, XmlAssetKind::LevelData);
        assert_eq!(summary.stats.elements, 2);
        assert_eq!(summary.stats.max_depth, 2);
        assert_eq!(
            summary.to_string(),
            "Cry3DEngine level data (2 elements, depth 2)"
        );
        assert_eq!(totals.files, 1);
        assert_eq!(totals.kinds.get(&XmlAssetKind::LevelData), Some(&1));

        let mut inspection = XmlInspection::default();
        inspection.add_file_summary(inspect_xml_path(path, bytes).expect("inspect xml"));
        assert!(
            inspection
                .report(20)
                .to_string()
                .contains("Cry3DEngine level data")
        );

        assert!(is_xml_backed_name("foo.XML"));
        assert!(is_xml_backed_path(Path::new("foo.cdf")));
        assert!(is_xml_backed_path(Path::new("foo.chrparams")));
        assert!(!is_xml_backed_name("foo.json"));
    }
}
