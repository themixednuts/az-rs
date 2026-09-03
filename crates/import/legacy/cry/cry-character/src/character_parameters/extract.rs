use super::types::{
    CharacterAnimationDirectiveSource, CharacterAnimationFilterFolderSource,
    CharacterAnimationSetFilterSource, CharacterAnimationWildcardSource,
    CharacterBoundingBoxExtensionSource, CharacterBoundingBoxIncludesSource,
    CharacterIkDefinitionSource, CharacterJointLodSource, CharacterLegacyParameterSource,
    CharacterParametersDbaSource, CharacterParametersIncludeSource,
    CharacterParametersLegacyNodeSource, CharacterParametersParseError, CharacterParametersSource,
    CharacterVector3Source,
};
mod ik;

use ik::parse_ik_definition;

pub(super) fn build_character_parameters_source(
    source_path: String,
    root: CharacterParametersLegacyNodeSource,
) -> Result<CharacterParametersSource, CharacterParametersParseError> {
    CharacterParametersBuilder::new(source_path).build(root)
}

struct CharacterParametersBuilder {
    source_path: String,
}

impl CharacterParametersBuilder {
    const fn new(source_path: String) -> Self {
        Self { source_path }
    }

    fn build(
        self,
        root: CharacterParametersLegacyNodeSource,
    ) -> Result<CharacterParametersSource, CharacterParametersParseError> {
        if root.name != "Params" {
            return Err(CharacterParametersParseError::UnexpectedRoot { element: root.name });
        }

        let mut source = CharacterParametersSource {
            skeleton_path: skeleton_path_for_chrparams(&self.source_path),
            source_path: self.source_path,
            root_parameters: root.parameters,
            includes: Vec::new(),
            animation_set_filter: CharacterAnimationSetFilterSource::default(),
            animation_event_database: None,
            face_lib_file: None,
            dba_path: None,
            individual_dbas: Vec::new(),
            bounding_box_includes: None,
            bounding_box_extension: None,
            joint_lods: Vec::new(),
            ik_definition: CharacterIkDefinitionSource::default(),
            legacy_animation_entries: Vec::new(),
            legacy_lod_nodes: Vec::new(),
            legacy_nodes: Vec::new(),
            legacy_text: root.text,
            comments: root.comments,
        };

        for child in root.children {
            match child.name.as_str() {
                "AnimationList" => parse_animation_list(&mut source, child)?,
                "BBoxIncludeList" => {
                    source.bounding_box_includes = Some(parse_bbox_includes(child));
                }
                "BBoxExtensionList" => {
                    source.bounding_box_extension = Some(parse_bbox_extension(child)?);
                }
                "Lod" => parse_lod(&mut source, child)?,
                "IK_Definition" => {
                    source.ik_definition = parse_ik_definition(child)?;
                }
                _ => source.legacy_nodes.push(child),
            }
        }

        Ok(source)
    }
}

fn parse_animation_list(
    source: &mut CharacterParametersSource,
    node: CharacterParametersLegacyNodeSource,
) -> Result<(), CharacterParametersParseError> {
    for child in node.children {
        if child.name != "Animation" {
            source
                .legacy_animation_entries
                .push(CharacterAnimationDirectiveSource {
                    name: child.name.clone(),
                    path: None,
                    flags: None,
                    legacy_parameters: child.parameters,
                });
            continue;
        }

        let name = attr(&child, "name").unwrap_or_default();
        let path = attr(&child, "path");
        let flags = attr(&child, "flags");
        let legacy_parameters = legacy_parameters(&child, &["name", "path", "flags"]);

        match name.as_str() {
            "$Include" => {
                source.includes.push(CharacterParametersIncludeSource {
                    filename: path.unwrap_or_default(),
                    legacy_parameters,
                });
            }
            "$AnimEventDatabase" => {
                source.animation_event_database = path;
            }
            "$TracksDatabase" => {
                if let Some(path) = path {
                    if let Some(wildcard) = path.find('*') {
                        source.dba_path = Some(path[..wildcard].to_string());
                    } else {
                        source.individual_dbas.push(CharacterParametersDbaSource {
                            filename: path,
                            persistent: flags
                                .as_deref()
                                .is_some_and(|value| value.contains("persistent")),
                            flags,
                            legacy_parameters,
                        });
                    }
                }
            }
            "$FaceLib" => {
                source.face_lib_file = path;
            }
            "#filepath" => {
                source
                    .animation_set_filter
                    .folders
                    .push(CharacterAnimationFilterFolderSource {
                        path: path.unwrap_or_default(),
                        parse_subfolders: None,
                        wildcards: Vec::new(),
                        legacy_parameters,
                    });
            }
            "#ParseSubFolders" => {
                let parse_subfolders =
                    path.as_deref()
                        .map(parse_bool)
                        .transpose()
                        .map_err(|value| CharacterParametersParseError::InvalidBool {
                            attribute: "path",
                            value,
                        })?;
                ensure_animation_folder(&mut source.animation_set_filter).parse_subfolders =
                    parse_subfolders;
            }
            _ if name.starts_with('$') => {
                source
                    .legacy_animation_entries
                    .push(CharacterAnimationDirectiveSource {
                        name,
                        path,
                        flags,
                        legacy_parameters,
                    });
            }
            _ => {
                ensure_animation_folder(&mut source.animation_set_filter)
                    .wildcards
                    .push(CharacterAnimationWildcardSource {
                        rename_mask: name,
                        file_wildcard: path.unwrap_or_default(),
                        legacy_parameters,
                    });
            }
        }
    }
    Ok(())
}

fn ensure_animation_folder(
    filter: &mut CharacterAnimationSetFilterSource,
) -> &mut CharacterAnimationFilterFolderSource {
    if filter.folders.is_empty() {
        filter.folders.push(CharacterAnimationFilterFolderSource {
            path: String::new(),
            parse_subfolders: None,
            wildcards: Vec::new(),
            legacy_parameters: Vec::new(),
        });
    }
    filter
        .folders
        .last_mut()
        .expect("animation filter folder was just inserted")
}

fn parse_bbox_includes(
    node: CharacterParametersLegacyNodeSource,
) -> CharacterBoundingBoxIncludesSource {
    let mut source = CharacterBoundingBoxIncludesSource {
        joints: Vec::new(),
        legacy_parameters: node.parameters,
        legacy_nodes: Vec::new(),
    };

    for child in node.children {
        if child.name == "Joint" {
            if let Some(name) = attr(&child, "name") {
                source.joints.push(name);
            }
        } else {
            source.legacy_nodes.push(child);
        }
    }

    source
}

fn parse_bbox_extension(
    node: CharacterParametersLegacyNodeSource,
) -> Result<CharacterBoundingBoxExtensionSource, CharacterParametersParseError> {
    let mut extension = CharacterBoundingBoxExtensionSource {
        negative: CharacterVector3Source::default(),
        positive: CharacterVector3Source::default(),
        legacy_parameters: node.parameters,
        axis_parameters: Vec::new(),
        legacy_nodes: Vec::new(),
    };

    for child in node.children {
        if child.name == "Axis" {
            extension.negative.x = optional_f32_attr(&child, "negX")?;
            extension.negative.y = optional_f32_attr(&child, "negY")?;
            extension.negative.z = optional_f32_attr(&child, "negZ")?;
            extension.positive.x = optional_f32_attr(&child, "posX")?;
            extension.positive.y = optional_f32_attr(&child, "posY")?;
            extension.positive.z = optional_f32_attr(&child, "posZ")?;
            extension.axis_parameters =
                legacy_parameters(&child, &["negX", "negY", "negZ", "posX", "posY", "posZ"]);
        } else {
            extension.legacy_nodes.push(child);
        }
    }

    Ok(extension)
}

fn parse_lod(
    source: &mut CharacterParametersSource,
    node: CharacterParametersLegacyNodeSource,
) -> Result<(), CharacterParametersParseError> {
    for child in node.children {
        if child.name != "JointList" {
            source.legacy_lod_nodes.push(child);
            continue;
        }

        let level = attr(&child, "level")
            .map(|value| parse_u8("level", &value))
            .transpose()?
            .unwrap_or_default();
        let mut lod = CharacterJointLodSource {
            level,
            joints: Vec::new(),
            legacy_parameters: legacy_parameters(&child, &["level"]),
            legacy_nodes: Vec::new(),
        };
        for joint_node in child.children {
            if joint_node.name == "Joint" {
                if let Some(name) = attr(&joint_node, "name") {
                    lod.joints.push(name);
                }
            } else {
                lod.legacy_nodes.push(joint_node);
            }
        }
        source.joint_lods.push(lod);
    }

    Ok(())
}

fn skeleton_path_for_chrparams(source_path: &str) -> String {
    source_path
        .strip_suffix(".chrparams")
        .map_or_else(|| source_path.to_string(), |stem| format!("{stem}.chr"))
}

fn attr(node: &CharacterParametersLegacyNodeSource, name: &str) -> Option<String> {
    node.parameters
        .iter()
        .find(|parameter| parameter.name == name)
        .map(|parameter| parameter.value.clone())
}

fn legacy_parameters(
    node: &CharacterParametersLegacyNodeSource,
    known_names: &[&str],
) -> Vec<CharacterLegacyParameterSource> {
    node.parameters
        .iter()
        .filter(|parameter| !known_names.contains(&parameter.name.as_str()))
        .cloned()
        .collect()
}

fn optional_bool_attr(
    node: &CharacterParametersLegacyNodeSource,
    name: &'static str,
) -> Result<Option<bool>, CharacterParametersParseError> {
    attr(node, name)
        .map(|value| {
            parse_bool(&value).map_err(|value| CharacterParametersParseError::InvalidBool {
                attribute: name,
                value,
            })
        })
        .transpose()
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" | "" => Ok(false),
        _ => value
            .parse::<i32>()
            .map(|value| value != 0)
            .map_err(|_| value.to_string()),
    }
}

fn optional_f32_attr(
    node: &CharacterParametersLegacyNodeSource,
    name: &'static str,
) -> Result<Option<f32>, CharacterParametersParseError> {
    attr(node, name)
        .map(|value| parse_f32(name, &value))
        .transpose()
}

fn parse_f32(name: &'static str, value: &str) -> Result<f32, CharacterParametersParseError> {
    value
        .parse::<f32>()
        .map_err(|_| CharacterParametersParseError::InvalidFloat {
            attribute: name,
            value: value.to_string(),
        })
}

fn optional_i32_attr(
    node: &CharacterParametersLegacyNodeSource,
    name: &'static str,
) -> Result<Option<i32>, CharacterParametersParseError> {
    attr(node, name)
        .map(|value| parse_i32(name, &value))
        .transpose()
}

fn parse_i32(name: &'static str, value: &str) -> Result<i32, CharacterParametersParseError> {
    value
        .parse::<i32>()
        .map_err(|_| CharacterParametersParseError::InvalidInteger {
            attribute: name,
            value: value.to_string(),
        })
}

fn parse_u8(name: &'static str, value: &str) -> Result<u8, CharacterParametersParseError> {
    value
        .parse::<u8>()
        .map_err(|_| CharacterParametersParseError::InvalidInteger {
            attribute: name,
            value: value.to_string(),
        })
}
