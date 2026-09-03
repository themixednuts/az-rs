use super::super::types::{
    CharacterAimIkSource, CharacterAnimationDrivenIkTargetListSource,
    CharacterAnimationDrivenIkTargetSource, CharacterDirectionalBlendSource,
    CharacterFeetLockIkSource, CharacterIkDefinitionSource, CharacterIkPositionSource,
    CharacterIkRotationSource, CharacterImpactJointSource, CharacterLegacyParameterSource,
    CharacterLimbIkEntrySource, CharacterLimbIkSolverSource, CharacterLimbIkSource,
    CharacterLookIkSource, CharacterParametersLegacyNodeSource, CharacterParametersParseError,
    CharacterRecoilIkSource,
};
use super::{attr, legacy_parameters, optional_bool_attr, optional_f32_attr, optional_i32_attr};

pub(super) fn parse_ik_definition(
    node: CharacterParametersLegacyNodeSource,
) -> Result<CharacterIkDefinitionSource, CharacterParametersParseError> {
    let mut ik = CharacterIkDefinitionSource {
        legacy_parameters: node.parameters,
        ..CharacterIkDefinitionSource::default()
    };

    for child in node.children {
        match child.name.as_str() {
            "LimbIK_Definition" => ik.limb = Some(parse_limb_ik(child)?),
            "AimIK_Definition" => ik.aim = Some(parse_aim_ik(child)?),
            "LookIK_Definition" => ik.look = Some(parse_look_ik(child)?),
            "Recoil_Definition" => ik.recoil = Some(parse_recoil_ik(child)?),
            "FeetLock_Definition" => ik.feet_lock = Some(parse_feet_lock_ik(child)),
            "Animation_Driven_IK_Targets" => {
                ik.animation_driven_targets = Some(parse_animation_driven_targets(child));
            }
            _ => ik.legacy_nodes.push(child),
        }
    }

    Ok(ik)
}

fn parse_limb_ik(
    node: CharacterParametersLegacyNodeSource,
) -> Result<CharacterLimbIkSource, CharacterParametersParseError> {
    let mut source = CharacterLimbIkSource {
        entries: Vec::new(),
        legacy_parameters: node.parameters,
        legacy_nodes: Vec::new(),
    };

    for child in node.children {
        if child.name != "IK" {
            source.legacy_nodes.push(child);
            continue;
        }

        source.entries.push(CharacterLimbIkEntrySource {
            solver: parse_limb_solver(attr(&child, "Solver")),
            handle: attr(&child, "Handle").unwrap_or_default(),
            root: attr(&child, "Root").unwrap_or_default(),
            end_effector: attr(&child, "EndEffector").unwrap_or_default(),
            step_size: optional_f32_attr(&child, "fStepSize")?,
            threshold: optional_f32_attr(&child, "fThreshold")?,
            max_iteration: optional_i32_attr(&child, "nMaxInteration")?,
            legacy_parameters: legacy_parameters(
                &child,
                &[
                    "Solver",
                    "Handle",
                    "Root",
                    "EndEffector",
                    "fStepSize",
                    "fThreshold",
                    "nMaxInteration",
                ],
            ),
        });
    }

    Ok(source)
}

fn parse_aim_ik(
    node: CharacterParametersLegacyNodeSource,
) -> Result<CharacterAimIkSource, CharacterParametersParseError> {
    let parsed = parse_aim_look_common(node)?;
    Ok(CharacterAimIkSource {
        directional_blends: parsed.directional_blends,
        rotations: parsed.rotations,
        positions: parsed.positions,
        legacy_parameters: parsed.legacy_parameters,
        legacy_nodes: parsed.legacy_nodes,
    })
}

fn parse_look_ik(
    node: CharacterParametersLegacyNodeSource,
) -> Result<CharacterLookIkSource, CharacterParametersParseError> {
    let parsed = parse_aim_look_common(node)?;
    Ok(CharacterLookIkSource {
        directional_blends: parsed.directional_blends,
        rotations: parsed.rotations,
        positions: parsed.positions,
        left_eye_attachment: parsed.left_eye_attachment,
        right_eye_attachment: parsed.right_eye_attachment,
        legacy_parameters: parsed.legacy_parameters,
        legacy_nodes: parsed.legacy_nodes,
    })
}

struct AimLookCommon {
    directional_blends: Vec<CharacterDirectionalBlendSource>,
    rotations: Vec<CharacterIkRotationSource>,
    positions: Vec<CharacterIkPositionSource>,
    left_eye_attachment: Option<String>,
    right_eye_attachment: Option<String>,
    legacy_parameters: Vec<CharacterLegacyParameterSource>,
    legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
}

fn parse_aim_look_common(
    node: CharacterParametersLegacyNodeSource,
) -> Result<AimLookCommon, CharacterParametersParseError> {
    let mut parsed = AimLookCommon {
        directional_blends: Vec::new(),
        rotations: Vec::new(),
        positions: Vec::new(),
        left_eye_attachment: None,
        right_eye_attachment: None,
        legacy_parameters: node.parameters,
        legacy_nodes: Vec::new(),
    };

    for child in node.children {
        match child.name.as_str() {
            "DirectionalBlends" => {
                parsed
                    .directional_blends
                    .extend(parse_directional_blends(child));
            }
            "RotationList" => parsed.rotations.extend(parse_rotations(child)?),
            "PositionList" => parsed.positions.extend(parse_positions(child)?),
            "LEyeAttachment" => parsed.left_eye_attachment = attr(&child, "Name"),
            "REyeAttachment" => parsed.right_eye_attachment = attr(&child, "Name"),
            _ => parsed.legacy_nodes.push(child),
        }
    }

    Ok(parsed)
}

fn parse_directional_blends(
    node: CharacterParametersLegacyNodeSource,
) -> Vec<CharacterDirectionalBlendSource> {
    let mut blends = Vec::new();
    for child in node.children {
        if child.name == "Joint" {
            blends.push(CharacterDirectionalBlendSource {
                anim_token: attr(&child, "AnimToken").unwrap_or_default(),
                parameter_joint: attr(&child, "ParameterJoint").unwrap_or_default(),
                start_joint: attr(&child, "StartJoint").unwrap_or_default(),
                reference_joint: attr(&child, "ReferenceJoint").unwrap_or_default(),
                legacy_parameters: legacy_parameters(
                    &child,
                    &[
                        "AnimToken",
                        "ParameterJoint",
                        "StartJoint",
                        "ReferenceJoint",
                    ],
                ),
            });
        }
    }
    blends
}

fn parse_rotations(
    node: CharacterParametersLegacyNodeSource,
) -> Result<Vec<CharacterIkRotationSource>, CharacterParametersParseError> {
    let mut rotations = Vec::new();
    for child in node.children {
        if child.name == "Rotation" {
            rotations.push(CharacterIkRotationSource {
                joint: attr(&child, "JointName").unwrap_or_default(),
                additive: optional_bool_attr(&child, "Additive")?.unwrap_or(false),
                primary: optional_bool_attr(&child, "Primary")?.unwrap_or(false),
                legacy_parameters: legacy_parameters(&child, &["JointName", "Additive", "Primary"]),
            });
        }
    }
    Ok(rotations)
}

fn parse_positions(
    node: CharacterParametersLegacyNodeSource,
) -> Result<Vec<CharacterIkPositionSource>, CharacterParametersParseError> {
    let mut positions = Vec::new();
    for child in node.children {
        if child.name == "Position" {
            positions.push(CharacterIkPositionSource {
                joint: attr(&child, "JointName").unwrap_or_default(),
                additive: optional_bool_attr(&child, "Additive")?.unwrap_or(false),
                legacy_parameters: legacy_parameters(&child, &["JointName", "Additive"]),
            });
        }
    }
    Ok(positions)
}

fn parse_recoil_ik(
    node: CharacterParametersLegacyNodeSource,
) -> Result<CharacterRecoilIkSource, CharacterParametersParseError> {
    let mut source = CharacterRecoilIkSource {
        left_handle: None,
        right_handle: None,
        left_weapon_joint: None,
        right_weapon_joint: None,
        impact_joints: Vec::new(),
        legacy_parameters: node.parameters,
        legacy_nodes: Vec::new(),
    };

    for child in node.children {
        match child.name.as_str() {
            "LIKHandle" => source.left_handle = attr(&child, "Handle"),
            "RIKHandle" => source.right_handle = attr(&child, "Handle"),
            "LWeaponJoint" => source.left_weapon_joint = attr(&child, "JointName"),
            "RWeaponJoint" => source.right_weapon_joint = attr(&child, "JointName"),
            "ImpactJoints" => {
                source.impact_joints.extend(parse_impact_joints(child)?);
            }
            _ => source.legacy_nodes.push(child),
        }
    }

    Ok(source)
}

fn parse_impact_joints(
    node: CharacterParametersLegacyNodeSource,
) -> Result<Vec<CharacterImpactJointSource>, CharacterParametersParseError> {
    let mut impact_joints = Vec::new();
    for child in node.children {
        if child.name == "ImpactJoint" {
            impact_joints.push(CharacterImpactJointSource {
                joint: attr(&child, "JointName").unwrap_or_default(),
                arm: optional_f32_attr(&child, "Arm")?,
                delay: optional_f32_attr(&child, "Delay")?,
                weight: optional_f32_attr(&child, "Weight")?,
                legacy_parameters: legacy_parameters(
                    &child,
                    &["JointName", "Arm", "Delay", "Weight"],
                ),
            });
        }
    }
    Ok(impact_joints)
}

fn parse_feet_lock_ik(node: CharacterParametersLegacyNodeSource) -> CharacterFeetLockIkSource {
    let mut source = CharacterFeetLockIkSource {
        left_handle: None,
        right_handle: None,
        legacy_parameters: node.parameters,
        legacy_nodes: Vec::new(),
    };

    for child in node.children {
        match child.name.as_str() {
            "LIKHandle" => source.left_handle = attr(&child, "Handle"),
            "RIKHandle" => source.right_handle = attr(&child, "Handle"),
            _ => source.legacy_nodes.push(child),
        }
    }

    source
}

fn parse_animation_driven_targets(
    node: CharacterParametersLegacyNodeSource,
) -> CharacterAnimationDrivenIkTargetListSource {
    let mut source = CharacterAnimationDrivenIkTargetListSource {
        targets: Vec::new(),
        legacy_parameters: node.parameters,
        legacy_nodes: Vec::new(),
    };

    for child in node.children {
        if child.name == "ADIKTarget" {
            source.targets.push(CharacterAnimationDrivenIkTargetSource {
                handle: attr(&child, "Handle").unwrap_or_default(),
                target: attr(&child, "Target").unwrap_or_default(),
                weight: attr(&child, "Weight").unwrap_or_default(),
                legacy_parameters: legacy_parameters(&child, &["Handle", "Target", "Weight"]),
            });
        } else {
            source.legacy_nodes.push(child);
        }
    }

    source
}

fn parse_limb_solver(value: Option<String>) -> CharacterLimbIkSolverSource {
    // A missing solver attribute is recorded the same way an unrecognized one
    // is: as an empty `Unknown`.
    let value = value.unwrap_or_default();
    match value.as_str() {
        "2BIK" => CharacterLimbIkSolverSource::TwoBone,
        "3BIK" => CharacterLimbIkSolverSource::ThreeBone,
        "CCDX" => CharacterLimbIkSolverSource::Ccdx,
        _ => CharacterLimbIkSolverSource::Unknown(value),
    }
}
