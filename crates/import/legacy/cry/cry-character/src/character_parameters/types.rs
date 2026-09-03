use std::str;

use quick_xml::events::attributes::AttrError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterLegacyParameterSource {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterParametersSource {
    pub source_path: String,
    pub skeleton_path: String,
    pub root_parameters: Vec<CharacterLegacyParameterSource>,
    pub includes: Vec<CharacterParametersIncludeSource>,
    pub animation_set_filter: CharacterAnimationSetFilterSource,
    pub animation_event_database: Option<String>,
    pub face_lib_file: Option<String>,
    pub dba_path: Option<String>,
    pub individual_dbas: Vec<CharacterParametersDbaSource>,
    pub bounding_box_includes: Option<CharacterBoundingBoxIncludesSource>,
    pub bounding_box_extension: Option<CharacterBoundingBoxExtensionSource>,
    pub joint_lods: Vec<CharacterJointLodSource>,
    pub ik_definition: CharacterIkDefinitionSource,
    pub legacy_animation_entries: Vec<CharacterAnimationDirectiveSource>,
    pub legacy_lod_nodes: Vec<CharacterParametersLegacyNodeSource>,
    pub legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
    pub legacy_text: Vec<String>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterParametersIncludeSource {
    pub filename: String,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterParametersDbaSource {
    pub filename: String,
    pub persistent: bool,
    pub flags: Option<String>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterAnimationSetFilterSource {
    pub folders: Vec<CharacterAnimationFilterFolderSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterAnimationFilterFolderSource {
    pub path: String,
    pub parse_subfolders: Option<bool>,
    pub wildcards: Vec<CharacterAnimationWildcardSource>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterAnimationWildcardSource {
    pub rename_mask: String,
    pub file_wildcard: String,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterAnimationDirectiveSource {
    pub name: String,
    pub path: Option<String>,
    pub flags: Option<String>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterBoundingBoxIncludesSource {
    pub joints: Vec<String>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
    pub legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterBoundingBoxExtensionSource {
    pub negative: CharacterVector3Source,
    pub positive: CharacterVector3Source,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
    pub axis_parameters: Vec<CharacterLegacyParameterSource>,
    pub legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterVector3Source {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub z: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterJointLodSource {
    pub level: u8,
    pub joints: Vec<String>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
    pub legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterIkDefinitionSource {
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
    pub limb: Option<CharacterLimbIkSource>,
    pub aim: Option<CharacterAimIkSource>,
    pub look: Option<CharacterLookIkSource>,
    pub recoil: Option<CharacterRecoilIkSource>,
    pub feet_lock: Option<CharacterFeetLockIkSource>,
    pub animation_driven_targets: Option<CharacterAnimationDrivenIkTargetListSource>,
    pub legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterLimbIkSource {
    pub entries: Vec<CharacterLimbIkEntrySource>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
    pub legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterLimbIkEntrySource {
    pub solver: CharacterLimbIkSolverSource,
    pub handle: String,
    pub root: String,
    pub end_effector: String,
    pub step_size: Option<f32>,
    pub threshold: Option<f32>,
    pub max_iteration: Option<i32>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterLimbIkSolverSource {
    TwoBone,
    ThreeBone,
    Ccdx,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterAimIkSource {
    pub directional_blends: Vec<CharacterDirectionalBlendSource>,
    pub rotations: Vec<CharacterIkRotationSource>,
    pub positions: Vec<CharacterIkPositionSource>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
    pub legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterLookIkSource {
    pub directional_blends: Vec<CharacterDirectionalBlendSource>,
    pub rotations: Vec<CharacterIkRotationSource>,
    pub positions: Vec<CharacterIkPositionSource>,
    pub left_eye_attachment: Option<String>,
    pub right_eye_attachment: Option<String>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
    pub legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterDirectionalBlendSource {
    pub anim_token: String,
    pub parameter_joint: String,
    pub start_joint: String,
    pub reference_joint: String,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterIkRotationSource {
    pub joint: String,
    pub additive: bool,
    pub primary: bool,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterIkPositionSource {
    pub joint: String,
    pub additive: bool,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterRecoilIkSource {
    pub left_handle: Option<String>,
    pub right_handle: Option<String>,
    pub left_weapon_joint: Option<String>,
    pub right_weapon_joint: Option<String>,
    pub impact_joints: Vec<CharacterImpactJointSource>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
    pub legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterImpactJointSource {
    pub joint: String,
    pub arm: Option<f32>,
    pub delay: Option<f32>,
    pub weight: Option<f32>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterFeetLockIkSource {
    pub left_handle: Option<String>,
    pub right_handle: Option<String>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
    pub legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterAnimationDrivenIkTargetListSource {
    pub targets: Vec<CharacterAnimationDrivenIkTargetSource>,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
    pub legacy_nodes: Vec<CharacterParametersLegacyNodeSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterAnimationDrivenIkTargetSource {
    pub handle: String,
    pub target: String,
    pub weight: String,
    pub legacy_parameters: Vec<CharacterLegacyParameterSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterParametersLegacyNodeSource {
    pub name: String,
    pub parameters: Vec<CharacterLegacyParameterSource>,
    pub text: Vec<String>,
    pub comments: Vec<String>,
    pub children: Vec<Self>,
}

#[must_use]
pub fn is_character_parameters_source_path(normalized_source_path: &str) -> bool {
    normalized_source_path.ends_with(".chrparams")
}

#[derive(Debug, thiserror::Error)]
pub enum CharacterParametersParseError {
    #[error("unsupported character parameters XML path {path}")]
    UnsupportedPath { path: String },
    #[error("character parameters XML is not UTF-8")]
    InvalidUtf8(str::Utf8Error),
    #[error("XML parser error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("XML attribute error: {0}")]
    Attribute(#[from] AttrError),
    #[error("missing <Params> root element")]
    MissingRoot,
    #[error("expected <Params> root element, found <{element}>")]
    UnexpectedRoot { element: String },
    #[error("unexpected closing </{element}> in character parameters")]
    UnexpectedEnd { element: String },
    #[error("mismatched closing </{found}>; expected </{expected}>")]
    MismatchedEnd { expected: String, found: String },
    #[error("XML document ended before closing <{element}>")]
    UnclosedElement { element: String },
    #[error("element <{element}> appears after closing </Params>")]
    ElementAfterRoot { element: String },
    #[error("unexpected text outside character parameters root: {text:?}")]
    TextOutsideRoot { text: String },
    #[error("invalid boolean {value:?} in attribute {attribute}")]
    InvalidBool {
        attribute: &'static str,
        value: String,
    },
    #[error("invalid float {value:?} in attribute {attribute}")]
    InvalidFloat {
        attribute: &'static str,
        value: String,
    },
    #[error("invalid integer {value:?} in attribute {attribute}")]
    InvalidInteger {
        attribute: &'static str,
        value: String,
    },
}
