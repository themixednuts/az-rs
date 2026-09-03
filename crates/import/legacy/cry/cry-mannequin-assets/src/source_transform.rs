//! Asset-builder adapters for legacy `CryAction` Mannequin source transforms.

use az_animation::blend_space_asset::{BlendSpaceSourceFormat, CombinedBlendSpaceSourceFormat};
use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceFormat, SourceSchemaType,
    source_schema_type,
};
use cry_mannequin::{
    BlendSpaceSourceInput, BlendSpaceSourceTransform as DecoderBlendSpaceSourceTransform,
    BlendSpaceSourceTransformError, MannequinSourceInput,
    MannequinSourceTransform as DecoderMannequinSourceTransform, MannequinSourceTransformError,
};

pub const BLEND_SPACE_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<BlendSpaceSourceFormat>();
pub const COMBINED_BLEND_SPACE_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<CombinedBlendSpaceSourceFormat>();
pub const MANNEQUIN_ACTIONS_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<MannequinActionsSourceFormat>();
pub const MANNEQUIN_ANIMATION_DATABASE_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<MannequinAnimationDatabaseSourceFormat>();
pub const MANNEQUIN_TAGS_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<MannequinTagsSourceFormat>();
pub const MANNEQUIN_CONTROLLER_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<MannequinControllerDefinitionSourceFormat>();

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.MannequinActionsSource",
    ext = "mannequin.actions.ron"
)]
pub struct MannequinActionsSourceFormat;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.MannequinAnimationDatabaseSource",
    ext = "adb.ron"
)]
pub struct MannequinAnimationDatabaseSourceFormat;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.MannequinTagsSource",
    ext = "mannequin.tags.ron"
)]
pub struct MannequinTagsSourceFormat;

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.cry.MannequinControllerDefinitionSource",
    ext = "mannequin.controller.ron"
)]
pub struct MannequinControllerDefinitionSourceFormat;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MannequinSourceTransform;

impl LegacySourceTransform for MannequinSourceTransform {
    type Error = MannequinSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let artifact = DecoderMannequinSourceTransform.transform(MannequinSourceInput {
            source_path: input.source_path.clone(),
            bytes: input.bytes,
        })?;

        Ok(LegacySourceOutput::authoring_source(
            artifact.path,
            SourceSchemaType::from_dynamic_parts(artifact.schema),
            artifact.bytes,
        ))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlendSpaceSourceTransform;

impl LegacySourceTransform for BlendSpaceSourceTransform {
    type Error = BlendSpaceSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let artifact = DecoderBlendSpaceSourceTransform.transform(BlendSpaceSourceInput {
            source_path: input.source_path.clone(),
            bytes: input.bytes,
        })?;

        let schema = if artifact.path.ends_with(".comb.ron") {
            COMBINED_BLEND_SPACE_SOURCE_SCHEMA
        } else {
            BLEND_SPACE_SOURCE_SCHEMA
        };
        Ok(LegacySourceOutput::authoring_source(
            artifact.path,
            schema,
            artifact.bytes,
        ))
    }
}

pub use cry_mannequin::{
    blend_space_source_path, is_legacy_blend_space_source, is_legacy_mannequin_source,
    mannequin_source_path,
};

#[cfg(test)]
mod tests {
    use az_asset_builder::{LegacySourceInput, LegacySourceOutput, LegacySourceTransform};

    use super::*;

    #[test]
    fn blend_space_schema_constants_are_engine_owned() {
        assert_eq!(
            BLEND_SPACE_SOURCE_SCHEMA.as_str(),
            az_animation::blend_space_asset::BLEND_SPACE_SOURCE_SCHEMA_NAME
        );
        assert_eq!(
            COMBINED_BLEND_SPACE_SOURCE_SCHEMA.as_str(),
            az_animation::blend_space_asset::COMBINED_BLEND_SPACE_SOURCE_SCHEMA_NAME
        );
    }

    #[test]
    fn mannequin_schema_constants_match_canonical_decoder_schema_names() {
        assert_eq!(
            MANNEQUIN_ACTIONS_SOURCE_SCHEMA.as_str(),
            cry_mannequin::MANNEQUIN_ACTIONS_SOURCE_SCHEMA
        );
        assert_eq!(
            MANNEQUIN_ANIMATION_DATABASE_SOURCE_SCHEMA.as_str(),
            cry_mannequin::MANNEQUIN_ANIMATION_DATABASE_SOURCE_SCHEMA
        );
        assert_eq!(
            MANNEQUIN_TAGS_SOURCE_SCHEMA.as_str(),
            cry_mannequin::MANNEQUIN_TAGS_SOURCE_SCHEMA
        );
        assert_eq!(
            MANNEQUIN_CONTROLLER_SOURCE_SCHEMA.as_str(),
            cry_mannequin::MANNEQUIN_CONTROLLER_SOURCE_SCHEMA
        );
    }

    #[test]
    fn adapts_mannequin_transform_to_legacy_source_output() {
        let output = MannequinSourceTransform
            .transform(LegacySourceInput::new(
                "Animations/Mannequin/ADB/Player/combat.adb",
                br#"<AnimDB FragDef="actions.xml" TagDef="tags.xml"><FragmentList/></AnimDB>"#,
            ))
            .unwrap();

        let LegacySourceOutput::AuthoringSource(artifact) = output else {
            panic!("expected authoring source");
        };
        assert_eq!(
            artifact.path,
            "animations/mannequin/adb/player/combat.adb.ron"
        );
        assert_eq!(artifact.schema, MANNEQUIN_ANIMATION_DATABASE_SOURCE_SCHEMA);
        let source =
            cry_mannequin::MannequinAnimationDatabaseSource::from_ron_bytes(&artifact.bytes)
                .unwrap();
        assert_eq!(
            source.source_path,
            "animations/mannequin/adb/player/combat.adb"
        );
    }

    #[test]
    fn adapts_blend_space_transform_to_legacy_source_output() {
        let output = BlendSpaceSourceTransform
            .transform(LegacySourceInput::new(
                "Animations/Mannequin/ADB/Player/locomotion.bspace",
                br#"<ParaGroup>
 <Dimensions>
  <Param Name="MoveSpeed" Min="0" Max="10" Cells="5"/>
 </Dimensions>
 <ExampleList>
  <Example AName="walk" SetPara0="1"/>
 </ExampleList>
</ParaGroup>"#,
            ))
            .unwrap();

        let LegacySourceOutput::AuthoringSource(artifact) = output else {
            panic!("expected authoring source");
        };
        assert_eq!(
            artifact.path,
            "animations/mannequin/adb/player/locomotion.bspace.ron"
        );
        assert_eq!(artifact.schema, BLEND_SPACE_SOURCE_SCHEMA);
        let source =
            az_animation::blend_space_asset::BlendSpaceSource::from_ron_bytes(&artifact.bytes)
                .unwrap();
        assert_eq!(
            source.source_path,
            "animations/mannequin/adb/player/locomotion.bspace"
        );
    }
}
