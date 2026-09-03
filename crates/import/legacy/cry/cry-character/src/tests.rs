use az_asset_builder::{LegacySourceInput, LegacySourceTransform};

use super::*;
use crate::source_schemas;

#[test]
fn transforms_character_definition_to_authoring_source() {
    let output = CharacterSourceTransform
        .transform(LegacySourceInput::new("Objects/Characters/Hero/hero.cdf", br#"<CharacterDefinition>
 <Model File="Objects/Characters/Hero/hero.chr" Material="Objects/Characters/Hero/hero" Physics="Objects/Characters/Hero/hero.phys" Rig="Objects/Characters/Hero/hero.rig"/>
 <AttachmentList>
  <Attachment Type="CA_SKIN" AName="coat" Binding="Objects/Characters/Hero/coat.skin" Material="Objects/Characters/Hero/coat" Flags="8"/>
 </AttachmentList>
</CharacterDefinition>"#))
        .unwrap();

    let artifact = output.artifact().expect("authoring artifact");
    assert_eq!(artifact.path, "objects/characters/hero/hero.character.ron");
    assert_eq!(artifact.schema, source_schemas::CHARACTER_DEFINITION);

    let source: CharacterDefinitionSource = ron::de::from_bytes(&artifact.bytes).unwrap();
    assert_eq!(
        source.model.as_str(),
        "objects/characters/hero/hero.skeleton.glb"
    );
    assert_eq!(
        source.material.as_ref().map(az_core::AssetPathBuf::as_str),
        Some("objects/characters/hero/hero.material.ron")
    );
    assert_eq!(source.attachments.len(), 1);
    let attachment = &source.attachments[0];
    assert!(matches!(attachment.kind, CharacterAttachmentKind::Skin(_)));
    assert_eq!(attachment.name.as_deref(), Some("coat"));
    assert!(matches!(
        attachment.binding.as_ref(),
        Some(AttachmentBinding::SkinnedMesh(path))
            if path.as_str() == "objects/characters/hero/coat.skinnedmesh.glb"
    ));
    assert_eq!(
        attachment
            .materials
            .shared
            .as_ref()
            .map(az_core::AssetPathBuf::as_str),
        Some("objects/characters/hero/coat.material.ron")
    );
    assert_eq!(attachment.flags, AttachmentFlags::SOFTWARE_SKINNING);
}

#[test]
fn character_definition_paths_only_claim_cdf() {
    assert_eq!(
        character_definition_source_path("objects/characters/hero/hero.cdf").as_deref(),
        Some("objects/characters/hero/hero.character.ron")
    );
    assert_eq!(character_definition_source_path("hero.chrparams"), None);
    assert_eq!(character_definition_source_path("hero.xml"), None);
}

#[test]
fn native_optional_attachment_transforms_ignore_malformed_legacy_hints() {
    let source = CharacterDefinitionSource::from_legacy(
        "objects/characters/weapons/blunderbuss.cdf",
        br#"<CharacterDefinition>
 <Model File="objects/characters/base.chr"/>
 <AttachmentList>
  <Attachment Type="CA_BONE" AName="weapon"
   Rotation="not,a,quaternion"
   Position="not,a,vector"
   RelRotation="-8.5965385e-10,0.054916643,0.088664211"
   RelPosition="also,not,a,vector"/>
 </AttachmentList>
</CharacterDefinition>"#,
    )
    .unwrap();

    let attachment = &source.attachments[0];
    assert_eq!(attachment.absolute, AttachmentTransform::default());
    assert_eq!(attachment.relative, RelativeAttachmentTransform::default());
}

#[test]
fn native_optional_attachment_quaternion_uses_wxyz_text_order() {
    let source = CharacterDefinitionSource::from_legacy(
        "objects/characters/hero.cdf",
        br#"<CharacterDefinition>
 <Model File="objects/characters/base.chr"/>
 <AttachmentList>
  <Attachment Type="CA_BONE" AName="weapon"
   RelRotation="1,2,3,4"/>
 </AttachmentList>
</CharacterDefinition>"#,
    )
    .unwrap();

    assert_eq!(
        source.attachments[0].relative.rotation,
        Some(bevy_math::Quat::from_xyzw(2.0, 3.0, 4.0, 1.0))
    );
}

#[test]
fn non_native_compatibility_attributes_remain_strict() {
    let error = CharacterDefinitionSource::from_legacy(
        "objects/characters/hero.cdf",
        br#"<CharacterDefinition>
 <Model File="objects/characters/base.chr"/>
 <AttachmentList>
  <Attachment Type="CA_PROX" AName="proxy" ProxyParams="1,2,3"/>
 </AttachmentList>
</CharacterDefinition>"#,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CharacterDefinitionParseError::InvalidAttribute {
            ref attribute,
            ref reason,
            ..
        } if attribute == "ProxyParams" && reason == "expected 4 comma-separated numbers"
    ));
}

#[test]
fn transforms_character_parameters_to_authoring_source() {
    let output = CharacterSourceTransform
        .transform(LegacySourceInput::new("Objects/Characters/Hero/hero.chrparams", br##"<Params author="CharacterTool">
 <AnimationList>
  <Animation name="$Include" path="Objects/Characters/Shared/shared.chrparams"/>
  <Animation name="$AnimEventDatabase" path="Animations/Events/hero.animevents"/>
  <Animation name="$TracksDatabase" path="Animations/Hero/*.dba"/>
  <Animation name="$TracksDatabase" path="Animations/Hero/idle.dba" flags="persistent"/>
  <Animation name="$FaceLib" path="Objects/Characters/Hero/hero.fxl"/>
  <Animation name="#filepath" path="Animations/Hero"/>
  <Animation name="idle_*" path="idle/*.caf"/>
 </AnimationList>
 <BBoxIncludeList><Joint name="Bip01 Pelvis"/></BBoxIncludeList>
 <BBoxExtensionList><Axis negX="-1" negY="-2" negZ="-3" posX="1" posY="2" posZ="3"/></BBoxExtensionList>
 <Lod>
  <JointList level="1"><Joint name="Bip01 Pelvis"/><Joint name="Head"/></JointList>
 </Lod>
 <IK_Definition>
  <LimbIK_Definition>
   <IK Solver="2BIK" Handle="LeftArm" Root="Bip01 L UpperArm" EndEffector="Bip01 L Hand"/>
  </LimbIK_Definition>
  <LookIK_Definition>
   <DirectionalBlends><Joint AnimToken="look" ParameterJoint="Head" StartJoint="Neck" ReferenceJoint="Bip01 Pelvis"/></DirectionalBlends>
   <RotationList><Rotation JointName="Head" Additive="0" Primary="1"/></RotationList>
   <PositionList><Position JointName="Head" Additive="1"/></PositionList>
   <LEyeAttachment Name="eye_l"/>
   <REyeAttachment Name="eye_r"/>
  </LookIK_Definition>
 </IK_Definition>
 <UnknownFeature enabled="1"><Child value="kept"/></UnknownFeature>
</Params>"##))
        .unwrap();

    let artifact = output.artifact().expect("authoring artifact");
    assert_eq!(artifact.path, "objects/characters/hero/hero.chrparams.ron");
    assert_eq!(artifact.schema, source_schemas::CHARACTER_PARAMETERS);

    let source: CharacterParametersSource = ron::de::from_bytes(&artifact.bytes).unwrap();
    assert_eq!(source.source_path, "objects/characters/hero/hero.chrparams");
    assert_eq!(source.skeleton_path, "objects/characters/hero/hero.chr");
    assert_eq!(source.root_parameters[0].name, "author");
    assert_eq!(
        source.includes[0].filename,
        "Objects/Characters/Shared/shared.chrparams"
    );
    assert_eq!(
        source.animation_event_database.as_deref(),
        Some("Animations/Events/hero.animevents")
    );
    assert_eq!(source.dba_path.as_deref(), Some("Animations/Hero/"));
    assert_eq!(
        source.individual_dbas[0].filename,
        "Animations/Hero/idle.dba"
    );
    assert!(source.individual_dbas[0].persistent);
    assert_eq!(
        source.face_lib_file.as_deref(),
        Some("Objects/Characters/Hero/hero.fxl")
    );
    assert_eq!(
        source.animation_set_filter.folders[0].path,
        "Animations/Hero"
    );
    assert_eq!(
        source.animation_set_filter.folders[0].wildcards[0].file_wildcard,
        "idle/*.caf"
    );
    assert_eq!(
        source.bounding_box_includes.as_ref().unwrap().joints,
        ["Bip01 Pelvis"]
    );
    assert_eq!(
        source.bounding_box_extension.as_ref().unwrap().negative.x,
        Some(-1.0)
    );
    assert_eq!(source.joint_lods[0].level, 1);
    assert_eq!(source.joint_lods[0].joints, ["Bip01 Pelvis", "Head"]);

    let limb = source.ik_definition.limb.as_ref().expect("limb ik");
    assert_eq!(limb.entries[0].handle, "LeftArm");
    assert_eq!(limb.entries[0].solver, CharacterLimbIkSolverSource::TwoBone);

    let look = source.ik_definition.look.as_ref().expect("look ik");
    assert_eq!(look.directional_blends[0].anim_token, "look");
    assert_eq!(look.rotations[0].joint, "Head");
    assert!(look.rotations[0].primary);
    assert_eq!(look.positions[0].joint, "Head");
    assert!(look.positions[0].additive);
    assert_eq!(look.left_eye_attachment.as_deref(), Some("eye_l"));
    assert_eq!(look.right_eye_attachment.as_deref(), Some("eye_r"));

    assert_eq!(source.legacy_nodes[0].name, "UnknownFeature");
    assert_eq!(source.legacy_nodes[0].children[0].name, "Child");
}

#[test]
fn character_parameters_paths_only_claim_chrparams() {
    assert_eq!(
        character_parameters_source_path("objects/characters/hero/hero.chrparams").as_deref(),
        Some("objects/characters/hero/hero.chrparams.ron")
    );
    assert_eq!(character_parameters_source_path("hero.cdf"), None);
    assert_eq!(character_parameters_source_path("hero.xml"), None);
}
