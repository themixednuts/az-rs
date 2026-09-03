//! Editor-owned mannequin animation preview catalog and transport controller.
//!
//! The motion catalog derives from asset-pipeline truth: the asset processor's
//! workspace asset status ([`EditorAssetBrowserStatus`]) filtered to animation
//! sources (`azoth.animation.AnimationSource`, `.anim.glb`), with absolute
//! paths resolved through the view's source roots — not a filesystem walk. A
//! catalog watch re-derives it whenever the tracked animation sources (or the
//! selected preview character) change. Blend-space RON is decoded through the
//! same engine-owned source structs used by the asset builder. Project-owned
//! Mannequin documents stay outside the engine catalog.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use az_animation::blend_space_asset::{
    BlendSpaceReference, BlendSpaceSource, CombinedBlendSpaceSource,
};
use az_core::reflect::ReflectedValueEnvelope;
use az_editor_inspector::{
    ReflectedEntityInspection, ReflectedInspectionChild, ReflectedInspectionField,
    ReflectedMapValueEntry, ReflectedScalar, ReflectedValue, ReflectedValueNode,
    decode_reflected_envelope,
};
use az_editor_ui::actions::{
    AddAnimationBlendSpaceExample, AddMannequinFragmentOption, AnimationBlendSpaceCoordinateEdit,
    EditAnimationBlendSpaceDimensionRange, EditAnimationBlendSpaceExampleCoordinate,
    EditMannequinFragmentOptionAnimation, EditMannequinFragmentOptionTags,
    RemoveAnimationBlendSpaceExample, RemoveMannequinFragmentOption, ScrubAnimationPreview,
    SelectAnimationBlendSpace, SelectAnimationCharacter, SelectAnimationMotion,
    SelectMannequinFragment, SetAnimationBlendSpaceParameter, SetAnimationBlendSpaceParameters,
    SetAnimationPreviewLoop, SetAnimationPreviewPlaying, SetMannequinTag, StopAnimationPreview,
};
use az_editor_ui::panels::{
    AssetBrowserEntryData, AssetBrowserEntryStatus, EditorAnimationEventData,
    EditorAnimationJointData, EditorAnimationMotionData, EditorAnimationPreviewCatalog,
    EditorAssetBrowserStatus, EditorBlendSpaceAssetData, EditorBlendSpaceAssetKind,
    EditorBlendSpaceCoordinateData, EditorBlendSpaceData, EditorBlendSpaceDimensionData,
    EditorBlendSpaceExampleData, EditorBlendSpacePreview, EditorBlendSpacePreviewCatalog,
    EditorBlendSpaceVirtualExampleData, EditorMannequinAnimationRefData,
    EditorMannequinAuthoringCatalog, EditorMannequinFragmentBlendData, EditorMannequinFragmentData,
    EditorMannequinFragmentDefinitionData, EditorMannequinFragmentOptionData,
    EditorMannequinFragmentOverrideData, EditorMannequinPreview,
    EditorMannequinResolvedAnimationData, EditorMannequinScopeContextData,
    EditorMannequinScopeData, EditorMannequinTagData, EditorMannequinTagGroupData,
    EditorTypeRegistry,
};
use az_proto_project::vnext::{
    PrefabEditCommand, ReflectedTypeDescriptor, ReflectedTypeKind, TypeRegistrySnapshot,
};
use gpui::App;
use serde::Deserialize;
use tracing::{info, instrument, warn};

use crate::asset_processor::{
    invalidate_animation_catalog_input, subscribe_animation_catalog_input_invalidation,
};
use crate::attach::EditorAttachSession;
use crate::authored_selection::{EditorReflectedSelectionState, apply_reflected_prefab_command};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MannequinPreviewAction {
    SelectCharacter(String),
    SelectMotion(String),
    SetPlaying(bool),
    Stop,
    SetLooping(bool),
    Scrub(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MannequinAuthoringAction {
    SelectFragment(String),
    SetTag { tag: String, enabled: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlendSpacePreviewAction {
    SelectBlendSpace(String),
    SetParameter { dimension: String, value: f32 },
    SetParameters(Vec<f32>),
}

pub fn apply_animation_preview_action(
    preview: &mut EditorMannequinPreview,
    action: MannequinPreviewAction,
) -> bool {
    let before = preview.clone();
    match action {
        MannequinPreviewAction::SelectCharacter(character_glb) => {
            preview.select_character(character_glb);
        }
        MannequinPreviewAction::SelectMotion(motion_glb) => {
            preview.select_motion(motion_glb);
        }
        MannequinPreviewAction::SetPlaying(playing) => {
            preview.set_playing(playing);
        }
        MannequinPreviewAction::Stop => {
            preview.stop();
        }
        MannequinPreviewAction::SetLooping(looping) => {
            preview.set_looping(looping);
        }
        MannequinPreviewAction::Scrub(position_millis) => {
            preview.seek_millis(position_millis);
        }
    }
    *preview != before
}

pub fn apply_mannequin_authoring_action(
    catalog: &mut EditorMannequinAuthoringCatalog,
    motion_catalog: &EditorAnimationPreviewCatalog,
    action: MannequinAuthoringAction,
) -> bool {
    let before = catalog.clone();
    match action {
        MannequinAuthoringAction::SelectFragment(fragment_key) => {
            if catalog
                .fragments
                .iter()
                .any(|fragment| fragment.key == fragment_key)
            {
                catalog.selected_fragment_key = Some(fragment_key);
            }
        }
        MannequinAuthoringAction::SetTag { tag, enabled } => {
            if catalog.has_tag(&tag) {
                set_mannequin_tag_state(catalog, &tag, enabled);
            }
        }
    }
    catalog.resolved = resolve_mannequin_preview_motion(catalog, motion_catalog);
    *catalog != before
}

pub fn apply_blend_space_preview_action(
    preview: &mut EditorBlendSpacePreview,
    project_asset_root: &Path,
    action: BlendSpacePreviewAction,
) -> bool {
    let before = preview.clone();
    preview.project_asset_root = Some(project_asset_root.to_path_buf());
    match action {
        BlendSpacePreviewAction::SelectBlendSpace(bspace_ron_path) => {
            if is_combined_blend_space_ron_path(Path::new(&bspace_ron_path)) {
                match load_combined_blend_space_source(project_asset_root, &bspace_ron_path) {
                    Ok(source) => {
                        preview.clear_selection();
                        preview.project_asset_root = Some(project_asset_root.to_path_buf());
                        preview.bspace_ron_path = Some(bspace_ron_path);
                        preview.diagnostics = combined_blend_space_preview_diagnostics(&source);
                    }
                    Err(diagnostic) => {
                        preview.clear_selection();
                        preview.project_asset_root = Some(project_asset_root.to_path_buf());
                        preview.diagnostics.push(diagnostic);
                    }
                }
            } else {
                match load_blend_space_document(project_asset_root, &bspace_ron_path) {
                    Ok((document, diagnostics)) => {
                        preview.set_document(bspace_ron_path, document, diagnostics);
                    }
                    Err(diagnostic) => {
                        preview.clear_selection();
                        preview.project_asset_root = Some(project_asset_root.to_path_buf());
                        preview.diagnostics.push(diagnostic);
                    }
                }
            }
        }
        BlendSpacePreviewAction::SetParameter { dimension, value } => {
            preview.set_param_value(&dimension, value);
        }
        BlendSpacePreviewAction::SetParameters(values) => {
            preview.set_param_values(&values);
        }
    }
    *preview != before
}

pub fn install_mannequin_animation_action_handlers(cx: &mut App) {
    install_animation_preview_action_handlers(cx);
    install_blend_space_action_handlers(cx);
    install_mannequin_fragment_action_handlers(cx);
}

/// Character and motion selection plus the preview transport.
fn install_animation_preview_action_handlers(cx: &mut App) {
    cx.on_action(|action: &SelectAnimationCharacter, cx| {
        update_mannequin_preview(
            cx,
            MannequinPreviewAction::SelectCharacter(action.character_glb.clone()),
        );
    });

    cx.on_action(|action: &SelectAnimationMotion, cx| {
        update_mannequin_preview(
            cx,
            MannequinPreviewAction::SelectMotion(action.motion_glb.clone()),
        );
    });

    cx.on_action(|action: &SetAnimationPreviewPlaying, cx| {
        update_mannequin_preview(cx, MannequinPreviewAction::SetPlaying(action.playing));
    });

    cx.on_action(|_: &StopAnimationPreview, cx| {
        update_mannequin_preview(cx, MannequinPreviewAction::Stop);
    });

    cx.on_action(|action: &SetAnimationPreviewLoop, cx| {
        update_mannequin_preview(cx, MannequinPreviewAction::SetLooping(action.looping));
    });

    cx.on_action(|action: &ScrubAnimationPreview, cx| {
        update_mannequin_preview(cx, MannequinPreviewAction::Scrub(action.position_millis));
    });
}

/// Blend-space selection, live parameter scrubbing, and the authored example
/// and dimension edits that round-trip through the reflected Prefab path.
fn install_blend_space_action_handlers(cx: &mut App) {
    cx.on_action(|action: &SelectAnimationBlendSpace, cx| {
        update_blend_space_preview(
            cx,
            BlendSpacePreviewAction::SelectBlendSpace(action.bspace_ron_path.clone()),
        );
    });

    cx.on_action(|action: &SetAnimationBlendSpaceParameter, cx| {
        update_blend_space_preview(
            cx,
            BlendSpacePreviewAction::SetParameter {
                dimension: action.dimension.clone(),
                value: action.value,
            },
        );
    });

    cx.on_action(|action: &SetAnimationBlendSpaceParameters, cx| {
        update_blend_space_preview(
            cx,
            BlendSpacePreviewAction::SetParameters(action.values.clone()),
        );
    });

    cx.on_action(|action: &EditAnimationBlendSpaceExampleCoordinate, cx| {
        dispatch_planned_reflected_edit(cx, |inspection, catalog| {
            plan_blend_space_example_coordinate_edit(
                inspection,
                catalog,
                action.example_index,
                &action.dimension,
                action.value,
            )
        });
    });

    cx.on_action(|action: &AddAnimationBlendSpaceExample, cx| {
        dispatch_planned_reflected_edit(cx, |inspection, catalog| {
            plan_blend_space_example_insert(
                inspection,
                catalog,
                &action.animation_name,
                &action.motion_path,
                &action.coordinates,
            )
        });
    });

    cx.on_action(|action: &RemoveAnimationBlendSpaceExample, cx| {
        dispatch_planned_reflected_edit(cx, |inspection, catalog| {
            plan_blend_space_example_remove(inspection, catalog, action.example_index)
        });
    });

    cx.on_action(|action: &EditAnimationBlendSpaceDimensionRange, cx| {
        dispatch_planned_reflected_edit(cx, |inspection, catalog| {
            plan_blend_space_dimension_range_edit(
                inspection,
                catalog,
                &action.dimension,
                action.min,
                action.max,
            )
        });
    });
}

/// Mannequin fragment selection, tag state, and the authored option edits that
/// round-trip through the reflected Prefab path.
fn install_mannequin_fragment_action_handlers(cx: &mut App) {
    cx.on_action(|action: &SelectMannequinFragment, cx| {
        update_mannequin_authoring(
            cx,
            MannequinAuthoringAction::SelectFragment(action.fragment_key.clone()),
        );
    });

    cx.on_action(|action: &SetMannequinTag, cx| {
        update_mannequin_authoring(
            cx,
            MannequinAuthoringAction::SetTag {
                tag: action.tag.clone(),
                enabled: action.enabled,
            },
        );
    });

    cx.on_action(|action: &EditMannequinFragmentOptionAnimation, cx| {
        dispatch_planned_reflected_edit(cx, |inspection, catalog| {
            plan_mannequin_option_animation_edit(
                inspection,
                catalog,
                &action.fragment_key,
                action.option_index,
                action.layer_index,
                action.animation_index,
                &action.animation_ref,
            )
        });
    });

    cx.on_action(|action: &EditMannequinFragmentOptionTags, cx| {
        dispatch_planned_reflected_edit(cx, |inspection, catalog| {
            plan_mannequin_option_tags_edit(
                inspection,
                catalog,
                &action.fragment_key,
                action.option_index,
                &action.tag_condition,
            )
        });
    });

    cx.on_action(|action: &AddMannequinFragmentOption, cx| {
        dispatch_planned_reflected_edit(cx, |inspection, catalog| {
            plan_mannequin_option_insert(
                inspection,
                catalog,
                &action.fragment_key,
                &action.animation_ref,
                &action.tag_condition,
            )
        });
    });

    cx.on_action(|action: &RemoveMannequinFragmentOption, cx| {
        dispatch_planned_reflected_edit(cx, |inspection, catalog| {
            plan_mannequin_option_remove(
                inspection,
                catalog,
                &action.fragment_key,
                action.option_index,
            )
        });
    });
}

/// The index an append lands at, as the `u32` reflected Prefab edit commands
/// address list elements with.
fn list_append_index(len: usize) -> Result<u32, String> {
    u32::try_from(len).map_err(|_| format!("reflected list holds more than {} elements", u32::MAX))
}

fn dispatch_planned_reflected_edit(
    cx: &mut App,
    planner: impl FnOnce(
        &ReflectedEntityInspection,
        &TypeRegistrySnapshot,
    ) -> Result<PrefabEditCommand, String>,
) {
    let outcome = {
        let Some(inspection) = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(|state| state.current())
        else {
            warn!(
                "open the source as an authored document before editing animation authoring data",
            );
            return;
        };
        let Some(type_registry) = cx.try_global::<EditorTypeRegistry>() else {
            warn!("type registry is not loaded for animation authored edits");
            return;
        };
        planner(inspection, &type_registry.snapshot)
    };

    match outcome {
        Ok(command) => apply_reflected_prefab_command(cx, command),
        Err(message) => {
            warn!(%message, "failed to plan reflected animation edit");
        }
    }
}

#[derive(Clone, Debug)]
struct ReflectedFieldRef<'a> {
    field: &'a ReflectedInspectionField,
}

#[derive(Clone, Debug)]
struct BlendSpaceDocumentRef<'a> {
    dimensions_field: ReflectedFieldRef<'a>,
    examples_field: ReflectedFieldRef<'a>,
}

fn plan_blend_space_example_coordinate_edit(
    inspection: &ReflectedEntityInspection,
    catalog: &TypeRegistrySnapshot,
    example_index: u32,
    dimension: &str,
    value: f32,
) -> Result<PrefabEditCommand, String> {
    let document = reflected_blend_space_document(inspection)?;
    let examples = reflected_list_items(&document.examples_field.field.value);
    let example = examples
        .get(example_index as usize)
        .ok_or_else(|| format!("blend-space example index {example_index} does not exist"))?;

    if let Some(coordinates_field) = reflected_node_field(example, &["coordinates"]) {
        let coordinates = reflected_list_items(&coordinates_field.value);
        if let Some(coordinate_index) = coordinate_index_for_dimension(&coordinates, dimension) {
            let value_field = reflected_node_field(coordinates[coordinate_index], &["value"])
                .ok_or_else(|| "blend-space coordinate has no value field".to_owned())?;
            return Ok(value_field.value.binding.set_value(float_envelope(
                catalog,
                &value_field.value.type_path,
                value,
            )?));
        }

        let coordinate_type = list_element_type(&coordinates_field.value.type_path)?;
        let coordinate =
            blend_space_coordinate_reflected_value(catalog, &coordinate_type, dimension, value)?;
        return Ok(coordinates_field.value.binding.list_insert(
            list_append_index(coordinates.len())?,
            encode_reflected_value(catalog, &coordinate_type, &coordinate)?,
        ));
    }

    let parameters_field = reflected_node_field(example, &["parameters"])
        .ok_or_else(|| "blend-space example has no parameters field".to_owned())?;
    let dimension_index = blend_space_dimension_index(&document, dimension)?;
    let parameter = reflected_list_items(&parameters_field.value)
        .get(dimension_index)
        .copied()
        .ok_or_else(|| format!("blend-space parameter index {dimension_index} does not exist"))?;
    Ok(parameter
        .binding
        .set_value(float_envelope(catalog, &parameter.type_path, value)?))
}

fn plan_blend_space_example_insert(
    inspection: &ReflectedEntityInspection,
    catalog: &TypeRegistrySnapshot,
    animation_name: &str,
    motion_path: &str,
    coordinates: &[AnimationBlendSpaceCoordinateEdit],
) -> Result<PrefabEditCommand, String> {
    let document = reflected_blend_space_document(inspection)?;
    let examples = reflected_list_items(&document.examples_field.field.value);
    let example_type = list_element_type(&document.examples_field.field.value.type_path)?;
    let value = if reflected_type_field(catalog, &example_type, &["coordinates"]).is_some() {
        blend_space_editor_example_reflected_value(
            catalog,
            &document,
            &example_type,
            animation_name,
            motion_path,
            coordinates,
        )?
    } else {
        blend_space_cry_example_reflected_value(
            catalog,
            &document,
            &example_type,
            animation_name,
            motion_path,
            coordinates,
        )?
    };

    Ok(document.examples_field.field.value.binding.list_insert(
        list_append_index(examples.len())?,
        encode_reflected_value(catalog, &example_type, &value)?,
    ))
}

fn plan_blend_space_example_remove(
    inspection: &ReflectedEntityInspection,
    _catalog: &TypeRegistrySnapshot,
    example_index: u32,
) -> Result<PrefabEditCommand, String> {
    let document = reflected_blend_space_document(inspection)?;
    let examples = reflected_list_items(&document.examples_field.field.value);
    if examples.get(example_index as usize).is_none() {
        return Err(format!(
            "blend-space example index {example_index} does not exist"
        ));
    }
    Ok(document
        .examples_field
        .field
        .value
        .binding
        .list_remove(example_index))
}

fn plan_blend_space_dimension_range_edit(
    inspection: &ReflectedEntityInspection,
    catalog: &TypeRegistrySnapshot,
    dimension: &str,
    min: f32,
    max: f32,
) -> Result<PrefabEditCommand, String> {
    let document = reflected_blend_space_document(inspection)?;
    let dimensions = reflected_list_items(&document.dimensions_field.field.value);
    let dimension_index = blend_space_dimension_index(&document, dimension)?;
    let dimension_value = dimensions
        .get(dimension_index)
        .ok_or_else(|| format!("blend-space dimension `{dimension}` does not exist"))?;
    let mut value = materialized_node_value(dimension_value)?;
    set_reflected_struct_field(
        &mut value,
        "min",
        ReflectedValue::Scalar(ReflectedScalar::Float(min.to_string())),
    )?;
    set_reflected_struct_field(
        &mut value,
        "max",
        ReflectedValue::Scalar(ReflectedScalar::Float(max.to_string())),
    )?;

    Ok(dimension_value.binding.set_value(encode_reflected_value(
        catalog,
        &dimension_value.type_path,
        &value,
    )?))
}

fn plan_mannequin_option_animation_edit(
    inspection: &ReflectedEntityInspection,
    _catalog: &TypeRegistrySnapshot,
    fragment_key: &str,
    option_index: u32,
    layer_index: u32,
    animation_index: u32,
    animation_ref: &str,
) -> Result<PrefabEditCommand, String> {
    let fragment = reflected_mannequin_fragment_path(inspection, fragment_key, option_index)?;
    let fragment_value = fragment.fragment_value;
    let layers_field = reflected_node_field(fragment_value, &["animation_layers"])
        .ok_or_else(|| "Mannequin fragment has no animation_layers field".to_owned())?;
    let layers = reflected_list_items(&layers_field.value);
    let layer = layers
        .get(layer_index as usize)
        .ok_or_else(|| format!("Mannequin animation layer index {layer_index} does not exist"))?;
    let animations_field = reflected_node_field(layer, &["animations"])
        .ok_or_else(|| "Mannequin layer has no animations field".to_owned())?;
    let animation = reflected_list_items(&animations_field.value)
        .get(animation_index as usize)
        .copied()
        .ok_or_else(|| format!("Mannequin animation index {animation_index} does not exist"))?;
    let name_field = reflected_node_field(animation, &["name"])
        .ok_or_else(|| "Mannequin animation has no name field".to_owned())?;

    Ok(name_field
        .value
        .binding
        .set_value(string_envelope(&name_field.value.type_path, animation_ref)?))
}

fn plan_mannequin_option_tags_edit(
    inspection: &ReflectedEntityInspection,
    catalog: &TypeRegistrySnapshot,
    fragment_key: &str,
    option_index: u32,
    tag_condition: &str,
) -> Result<PrefabEditCommand, String> {
    let fragment = reflected_mannequin_fragment_path(inspection, fragment_key, option_index)?;
    let tags_field = reflected_node_field(fragment.fragment_value, &["tags"])
        .ok_or_else(|| "Mannequin fragment has no tags field".to_owned())?;
    let value = optional_string_reflected_value(tag_condition);
    Ok(tags_field.value.binding.set_value(encode_reflected_value(
        catalog,
        &tags_field.value.type_path,
        &value,
    )?))
}

fn plan_mannequin_option_insert(
    inspection: &ReflectedEntityInspection,
    catalog: &TypeRegistrySnapshot,
    fragment_key: &str,
    animation_ref: &str,
    tag_condition: &str,
) -> Result<PrefabEditCommand, String> {
    let group = reflected_mannequin_fragment_group(inspection, fragment_key)?;
    let fragments = reflected_list_items(&group.fragments_field.value);
    let fragment_type = list_element_type(&group.fragments_field.value.type_path)?;
    let value =
        mannequin_fragment_reflected_value(catalog, &fragment_type, animation_ref, tag_condition)?;

    Ok(group.fragments_field.value.binding.list_insert(
        list_append_index(fragments.len())?,
        encode_reflected_value(catalog, &fragment_type, &value)?,
    ))
}

fn plan_mannequin_option_remove(
    inspection: &ReflectedEntityInspection,
    _catalog: &TypeRegistrySnapshot,
    fragment_key: &str,
    option_index: u32,
) -> Result<PrefabEditCommand, String> {
    let fragment = reflected_mannequin_fragment_path(inspection, fragment_key, option_index)?;
    Ok(fragment
        .fragments_field
        .value
        .binding
        .list_remove(option_index))
}

fn reflected_blend_space_document(
    inspection: &ReflectedEntityInspection,
) -> Result<BlendSpaceDocumentRef<'_>, String> {
    let root_field =
        reflected_root_field(inspection, &["blend_space", "document"]).ok_or_else(|| {
            format!(
                "active reflected source `{}` is not a blend-space source document",
                inspection.selection.source_path
            )
        })?;
    let dimensions_field = reflected_node_field(&root_field.value, &["dimensions"])
        .ok_or_else(|| "blend-space document has no dimensions field".to_owned())?;
    let examples_field = reflected_node_field(&root_field.value, &["examples"])
        .ok_or_else(|| "blend-space document has no examples field".to_owned())?;
    Ok(BlendSpaceDocumentRef {
        dimensions_field: ReflectedFieldRef {
            field: dimensions_field,
        },
        examples_field: ReflectedFieldRef {
            field: examples_field,
        },
    })
}

fn blend_space_dimension_index(
    document: &BlendSpaceDocumentRef<'_>,
    dimension: &str,
) -> Result<usize, String> {
    let dimensions = reflected_list_items(&document.dimensions_field.field.value);
    if let Ok(index) = dimension.parse::<usize>()
        && index < dimensions.len()
    {
        return Ok(index);
    }
    dimensions
        .iter()
        .enumerate()
        .find_map(|(index, value)| {
            (blend_space_dimension_name(value, index) == dimension).then_some(index)
        })
        .ok_or_else(|| format!("blend-space dimension `{dimension}` does not exist"))
}

fn blend_space_editor_example_reflected_value(
    catalog: &TypeRegistrySnapshot,
    document: &BlendSpaceDocumentRef<'_>,
    example_type: &str,
    animation_name: &str,
    motion_path: &str,
    coordinates: &[AnimationBlendSpaceCoordinateEdit],
) -> Result<ReflectedValue, String> {
    let mut value = default_reflected_value(catalog, example_type)?;
    let animation_field = reflected_type_field(catalog, example_type, &["animation"])
        .ok_or_else(|| format!("type `{example_type}` has no animation field"))?;
    let mut animation = default_reflected_value(catalog, &animation_field.type_path)?;
    if reflected_type_field(catalog, &animation_field.type_path, &["name"]).is_some() {
        set_reflected_struct_field(
            &mut animation,
            "name",
            ReflectedValue::Scalar(ReflectedScalar::String(animation_name.to_owned())),
        )?;
    }
    if reflected_type_field(catalog, &animation_field.type_path, &["motion_path"]).is_some() {
        set_reflected_struct_field(
            &mut animation,
            "motion_path",
            optional_string_reflected_value(motion_path),
        )?;
    }
    set_reflected_struct_field(&mut value, "animation", animation)?;

    let coordinates_field = reflected_type_field(catalog, example_type, &["coordinates"])
        .ok_or_else(|| format!("type `{example_type}` has no coordinates field"))?;
    let coordinate_type = list_element_type(&coordinates_field.type_path)?;
    let coordinate_values = if coordinates.is_empty() {
        blend_space_default_coordinate_edits(document)
    } else {
        coordinates.to_vec()
    };
    set_reflected_struct_field(
        &mut value,
        "coordinates",
        ReflectedValue::List(
            coordinate_values
                .iter()
                .map(|coordinate| {
                    blend_space_coordinate_reflected_value(
                        catalog,
                        &coordinate_type,
                        &coordinate.dimension,
                        coordinate.value,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
    )?;
    if reflected_type_field(catalog, example_type, &["playback_scale"]).is_some() {
        set_reflected_struct_field(
            &mut value,
            "playback_scale",
            ReflectedValue::Scalar(ReflectedScalar::Float("1.0".to_owned())),
        )?;
    }
    Ok(value)
}

fn blend_space_cry_example_reflected_value(
    catalog: &TypeRegistrySnapshot,
    document: &BlendSpaceDocumentRef<'_>,
    example_type: &str,
    animation_name: &str,
    motion_path: &str,
    coordinates: &[AnimationBlendSpaceCoordinateEdit],
) -> Result<ReflectedValue, String> {
    let mut value = default_reflected_value(catalog, example_type)?;
    set_reflected_struct_field(
        &mut value,
        "animation",
        ReflectedValue::Scalar(ReflectedScalar::String(non_empty_string_or(
            motion_path,
            animation_name,
        ))),
    )?;
    if reflected_type_field(catalog, example_type, &["playback_scale"]).is_some() {
        set_reflected_struct_field(
            &mut value,
            "playback_scale",
            ReflectedValue::Scalar(ReflectedScalar::Float("1.0".to_owned())),
        )?;
    }
    if reflected_type_field(catalog, example_type, &["parameters"]).is_some() {
        let defaults = blend_space_default_coordinate_edits(document);
        let dimension_count = defaults.len().max(4);
        let values = (0..dimension_count)
            .map(|index| {
                defaults
                    .get(index)
                    .and_then(|default| {
                        coordinates
                            .iter()
                            .find(|coordinate| coordinate.dimension == default.dimension)
                    })
                    .map_or(ReflectedValue::Optional(None), |coordinate| {
                        ReflectedValue::Optional(Some(Box::new(ReflectedValue::Scalar(
                            ReflectedScalar::Float(coordinate.value.to_string()),
                        ))))
                    })
            })
            .collect();
        set_reflected_struct_field(&mut value, "parameters", ReflectedValue::List(values))?;
    }
    Ok(value)
}

fn blend_space_default_coordinate_edits(
    document: &BlendSpaceDocumentRef<'_>,
) -> Vec<AnimationBlendSpaceCoordinateEdit> {
    let dimensions = reflected_list_items(&document.dimensions_field.field.value);
    dimensions
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let name = blend_space_dimension_name(value, index);
            let min = reflected_node_field(value, &["min"])
                .and_then(|field| reflected_float(field_value(field)))
                .unwrap_or(0.0);
            let max = reflected_node_field(value, &["max"])
                .and_then(|field| reflected_float(field_value(field)))
                .unwrap_or(1.0);
            AnimationBlendSpaceCoordinateEdit {
                dimension: name,
                value: (min + max) * 0.5,
            }
        })
        .collect()
}

fn blend_space_coordinate_reflected_value(
    catalog: &TypeRegistrySnapshot,
    coordinate_type: &str,
    dimension: &str,
    value: f32,
) -> Result<ReflectedValue, String> {
    let mut reflected = default_reflected_value(catalog, coordinate_type)?;
    set_reflected_struct_field(
        &mut reflected,
        "dimension",
        ReflectedValue::Scalar(ReflectedScalar::String(dimension.to_owned())),
    )?;
    let value_field = reflected_type_field(catalog, coordinate_type, &["value"])
        .ok_or_else(|| format!("type `{coordinate_type}` has no value field"))?;
    let value = if reflected_type(catalog, &value_field.type_path)
        .is_some_and(|descriptor| descriptor.kind == ReflectedTypeKind::Optional)
    {
        ReflectedValue::Optional(Some(Box::new(ReflectedValue::Scalar(
            ReflectedScalar::Float(value.to_string()),
        ))))
    } else {
        ReflectedValue::Scalar(ReflectedScalar::Float(value.to_string()))
    };
    set_reflected_struct_field(&mut reflected, "value", value)?;
    Ok(reflected)
}

fn coordinate_index_for_dimension(
    coordinates: &[&ReflectedValueNode],
    dimension: &str,
) -> Option<usize> {
    coordinates.iter().enumerate().find_map(|(index, value)| {
        let coordinate_dimension = reflected_node_field(value, &["dimension"])?;
        (reflected_string(field_value(coordinate_dimension)).as_deref() == Some(dimension))
            .then_some(index)
    })
}

#[derive(Clone, Debug)]
struct MannequinFragmentGroupRef<'a> {
    fragments_field: &'a ReflectedInspectionField,
}

#[derive(Clone, Debug)]
struct MannequinFragmentRef<'a> {
    fragments_field: &'a ReflectedInspectionField,
    fragment_value: &'a ReflectedValueNode,
}

fn reflected_mannequin_fragment_group<'a>(
    inspection: &'a ReflectedEntityInspection,
    fragment_key: &str,
) -> Result<MannequinFragmentGroupRef<'a>, String> {
    let database = reflected_root_field(inspection, &["database"]).ok_or_else(|| {
        format!(
            "active reflected source `{}` is not a Mannequin animation database",
            inspection.selection.source_path
        )
    })?;
    let groups_field = reflected_node_field(&database.value, &["fragment_groups"])
        .ok_or_else(|| "Mannequin database has no fragment_groups field".to_owned())?;
    let groups = reflected_list_items(&groups_field.value);
    let fragment_name = fragment_name_from_key(fragment_key);
    let group_value = groups
        .iter()
        .find(|group| {
            reflected_node_field(group, &["name"])
                .and_then(|field| reflected_string(field_value(field)))
                .as_deref()
                == Some(fragment_name.as_str())
        })
        .ok_or_else(|| format!("Mannequin fragment `{fragment_name}` does not exist"))?;
    let fragments_field = reflected_node_field(group_value, &["fragments"])
        .ok_or_else(|| "Mannequin fragment group has no fragments field".to_owned())?;
    Ok(MannequinFragmentGroupRef { fragments_field })
}

fn reflected_mannequin_fragment_path<'a>(
    inspection: &'a ReflectedEntityInspection,
    fragment_key: &str,
    option_index: u32,
) -> Result<MannequinFragmentRef<'a>, String> {
    let group = reflected_mannequin_fragment_group(inspection, fragment_key)?;
    let fragments = reflected_list_items(&group.fragments_field.value);
    let fragment_value = fragments
        .get(option_index as usize)
        .copied()
        .ok_or_else(|| format!("Mannequin fragment option index {option_index} does not exist"))?;
    Ok(MannequinFragmentRef {
        fragments_field: group.fragments_field,
        fragment_value,
    })
}

fn mannequin_fragment_reflected_value(
    catalog: &TypeRegistrySnapshot,
    fragment_type: &str,
    animation_ref: &str,
    tag_condition: &str,
) -> Result<ReflectedValue, String> {
    let mut fragment = default_reflected_value(catalog, fragment_type)?;
    if reflected_type_field(catalog, fragment_type, &["tags"]).is_some() {
        set_reflected_struct_field(
            &mut fragment,
            "tags",
            optional_string_reflected_value(tag_condition),
        )?;
    }
    let layers_field = reflected_type_field(catalog, fragment_type, &["animation_layers"])
        .ok_or_else(|| format!("type `{fragment_type}` has no animation_layers field"))?;
    let layer_type = list_element_type(&layers_field.type_path)?;
    let mut layer = default_reflected_value(catalog, &layer_type)?;
    let animations_field = reflected_type_field(catalog, &layer_type, &["animations"])
        .ok_or_else(|| format!("type `{layer_type}` has no animations field"))?;
    let animation_type = list_element_type(&animations_field.type_path)?;
    let mut animation = default_reflected_value(catalog, &animation_type)?;
    set_reflected_struct_field(
        &mut animation,
        "name",
        ReflectedValue::Scalar(ReflectedScalar::String(animation_ref.to_owned())),
    )?;
    set_reflected_struct_field(
        &mut layer,
        "animations",
        ReflectedValue::List(vec![animation]),
    )?;
    set_reflected_struct_field(
        &mut fragment,
        "animation_layers",
        ReflectedValue::List(vec![layer]),
    )?;
    Ok(fragment)
}

fn fragment_name_from_key(fragment_key: &str) -> String {
    fragment_key
        .rsplit_once('#')
        .map_or(fragment_key, |(_, fragment)| fragment)
        .to_owned()
}

fn reflected_type_field<'a>(
    catalog: &'a TypeRegistrySnapshot,
    type_path: &str,
    names: &[&str],
) -> Option<&'a az_proto_project::vnext::ReflectedFieldDescriptor> {
    reflected_type(catalog, type_path)?
        .fields
        .iter()
        .find(|field| names.iter().any(|name| field.name == *name))
}

fn reflected_root_field<'a>(
    inspection: &'a ReflectedEntityInspection,
    names: &[&str],
) -> Option<&'a ReflectedInspectionField> {
    inspection.components.iter().find_map(|component| {
        component
            .model
            .fields
            .iter()
            .find(|field| names.iter().any(|name| field.name == *name))
    })
}

fn reflected_node_field<'a>(
    value: &'a ReflectedValueNode,
    names: &[&str],
) -> Option<&'a ReflectedInspectionField> {
    value.children.iter().find_map(|child| match child {
        ReflectedInspectionChild::Field(field) if names.iter().any(|name| field.name == *name) => {
            Some(field.as_ref())
        }
        _ => None,
    })
}

fn reflected_list_items(value: &ReflectedValueNode) -> Vec<&ReflectedValueNode> {
    value
        .children
        .iter()
        .filter_map(|child| match child {
            ReflectedInspectionChild::ListItem(item) => Some(item.value.as_ref()),
            _ => None,
        })
        .collect()
}

const fn field_value(field: &ReflectedInspectionField) -> Option<&ReflectedValue> {
    field.value.current.effective.as_ref()
}

fn reflected_string(value: Option<&ReflectedValue>) -> Option<String> {
    match value? {
        ReflectedValue::Scalar(ReflectedScalar::String(value)) => Some(value.clone()),
        ReflectedValue::Optional(Some(value)) => reflected_string(Some(value)),
        ReflectedValue::OpaqueRon(value) => Some(value.trim_matches('"').to_owned()),
        _ => None,
    }
}

fn reflected_float(value: Option<&ReflectedValue>) -> Option<f32> {
    match value? {
        ReflectedValue::Scalar(
            ReflectedScalar::Float(value)
            | ReflectedScalar::Signed(value)
            | ReflectedScalar::Unsigned(value),
        ) => value.parse().ok(),
        ReflectedValue::Optional(Some(value)) => reflected_float(Some(value)),
        _ => None,
    }
}

fn reflected_u32(value: Option<&ReflectedValue>) -> Option<u32> {
    match value? {
        ReflectedValue::Scalar(
            ReflectedScalar::Unsigned(value) | ReflectedScalar::Signed(value),
        ) => value.parse().ok(),
        ReflectedValue::Optional(Some(value)) => reflected_u32(Some(value)),
        _ => None,
    }
}

fn reflected_bool(value: Option<&ReflectedValue>) -> Option<bool> {
    match value? {
        ReflectedValue::Scalar(ReflectedScalar::Bool(value)) => Some(*value),
        ReflectedValue::Optional(Some(value)) => reflected_bool(Some(value)),
        _ => None,
    }
}

fn reflected_variant_name(value: Option<&ReflectedValue>) -> Option<String> {
    match value? {
        ReflectedValue::Enum { variant, .. } => Some(variant.clone()),
        ReflectedValue::Scalar(ReflectedScalar::String(value)) => Some(value.clone()),
        ReflectedValue::Optional(Some(value)) => reflected_variant_name(Some(value)),
        _ => None,
    }
}

fn reflected_type<'a>(
    catalog: &'a TypeRegistrySnapshot,
    type_path: &str,
) -> Option<&'a ReflectedTypeDescriptor> {
    catalog.types.iter().find(|descriptor| {
        descriptor.type_path == type_path
            || descriptor.short_path == type_path
            || descriptor.type_path.rsplit("::").next() == type_path.rsplit("::").next()
    })
}

fn list_element_type(type_path: &str) -> Result<String, String> {
    generic_arguments(type_path)
        .into_iter()
        .next()
        .ok_or_else(|| format!("reflected list type `{type_path}` has no element argument"))
}

fn generic_arguments(type_path: &str) -> Vec<String> {
    let Some((_, arguments)) = type_path.split_once('<') else {
        return Vec::new();
    };
    let arguments = arguments.strip_suffix('>').unwrap_or(arguments);
    let mut depth = 0_u32;
    let mut start = 0;
    let mut parsed = Vec::new();
    for (index, character) in arguments.char_indices() {
        match character {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parsed.push(arguments[start..index].trim().to_owned());
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = arguments[start..].trim();
    if !tail.is_empty() {
        parsed.push(tail.to_owned());
    }
    parsed
}

fn materialized_node_value(node: &ReflectedValueNode) -> Result<ReflectedValue, String> {
    match node.kind {
        ReflectedTypeKind::Struct => Ok(ReflectedValue::Struct(
            node.children
                .iter()
                .filter_map(|child| match child {
                    ReflectedInspectionChild::Field(field) => Some(
                        materialized_node_value(&field.value)
                            .map(|value| (field.name.clone(), value)),
                    ),
                    _ => None,
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        ReflectedTypeKind::List | ReflectedTypeKind::Array { .. } => Ok(ReflectedValue::List(
            reflected_list_items(node)
                .into_iter()
                .map(materialized_node_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        _ => node
            .current
            .effective
            .clone()
            .ok_or_else(|| format!("reflected value `{}` is not projected", node.type_path)),
    }
}

fn set_reflected_struct_field(
    value: &mut ReflectedValue,
    field_name: &str,
    replacement: ReflectedValue,
) -> Result<(), String> {
    let ReflectedValue::Struct(fields) = value else {
        return Err("reflected value is not a struct".to_owned());
    };
    if let Some((_, value)) = fields.iter_mut().find(|(name, _)| name == field_name) {
        *value = replacement;
    } else {
        fields.push((field_name.to_owned(), replacement));
    }
    Ok(())
}

fn default_reflected_value(
    catalog: &TypeRegistrySnapshot,
    type_path: &str,
) -> Result<ReflectedValue, String> {
    let descriptor = reflected_type(catalog, type_path)
        .ok_or_else(|| format!("reflected type `{type_path}` is not loaded"))?;
    if let Some(default) = &descriptor.reflected_default {
        return decode_reflected_envelope(catalog, default).map_err(|error| error.to_string());
    }
    match descriptor.kind {
        ReflectedTypeKind::Struct => descriptor
            .fields
            .iter()
            .map(|field| {
                default_reflected_value(catalog, &field.type_path)
                    .map(|value| (field.name.clone(), value))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(ReflectedValue::Struct),
        ReflectedTypeKind::Tuple | ReflectedTypeKind::TupleStruct => descriptor
            .fields
            .iter()
            .map(|field| default_reflected_value(catalog, &field.type_path))
            .collect::<Result<Vec<_>, _>>()
            .map(ReflectedValue::Tuple),
        ReflectedTypeKind::List | ReflectedTypeKind::Array { .. } => {
            Ok(ReflectedValue::List(Vec::new()))
        }
        ReflectedTypeKind::Map => Ok(ReflectedValue::Map(Vec::new())),
        ReflectedTypeKind::Enum => {
            let variant = descriptor
                .variants
                .first()
                .ok_or_else(|| format!("enum `{type_path}` has no variants"))?;
            let fields = variant
                .fields
                .iter()
                .map(|field| {
                    default_reflected_value(catalog, &field.type_path)
                        .map(|value| (field.name.clone(), value))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ReflectedValue::Enum {
                variant: variant.name.clone(),
                fields,
            })
        }
        ReflectedTypeKind::Optional => Ok(ReflectedValue::Optional(None)),
        ReflectedTypeKind::Bool => Ok(ReflectedValue::Scalar(ReflectedScalar::Bool(false))),
        ReflectedTypeKind::SignedInteger { .. } => Ok(ReflectedValue::Scalar(
            ReflectedScalar::Signed("0".to_owned()),
        )),
        ReflectedTypeKind::UnsignedInteger { .. } => Ok(ReflectedValue::Scalar(
            ReflectedScalar::Unsigned("0".to_owned()),
        )),
        ReflectedTypeKind::Float { .. } => Ok(ReflectedValue::Scalar(ReflectedScalar::Float(
            "0.0".to_owned(),
        ))),
        ReflectedTypeKind::String => Ok(ReflectedValue::Scalar(ReflectedScalar::String(
            String::new(),
        ))),
        ReflectedTypeKind::Set => Ok(ReflectedValue::OpaqueRon("[]".to_owned())),
        ReflectedTypeKind::Opaque => Ok(ReflectedValue::OpaqueRon("()".to_owned())),
    }
}

fn optional_string_reflected_value(value: &str) -> ReflectedValue {
    if value.trim().is_empty() {
        ReflectedValue::Optional(None)
    } else {
        ReflectedValue::Optional(Some(Box::new(ReflectedValue::Scalar(
            ReflectedScalar::String(value.to_owned()),
        ))))
    }
}

/// The authored dimension name, its parameter variant name, or the list index
/// rendered as a string when the node carries neither.
fn blend_space_dimension_name(value: &ReflectedValueNode, fallback_index: usize) -> String {
    if let Some(field) = reflected_node_field(value, &["name"])
        && let Some(name) = reflected_string(field_value(field))
    {
        return name;
    }
    if let Some(field) = reflected_node_field(value, &["parameter"])
        && let Some(name) = reflected_variant_name(field_value(field))
    {
        return name;
    }
    fallback_index.to_string()
}

fn float_envelope(
    catalog: &TypeRegistrySnapshot,
    type_path: &str,
    value: f32,
) -> Result<ReflectedValueEnvelope, String> {
    let value = ReflectedValue::Scalar(ReflectedScalar::Float(value.to_string()));
    let value = if reflected_type(catalog, type_path)
        .is_some_and(|descriptor| descriptor.kind == ReflectedTypeKind::Optional)
    {
        ReflectedValue::Optional(Some(Box::new(value)))
    } else {
        value
    };
    encode_reflected_value(catalog, type_path, &value)
}

fn string_envelope(type_path: &str, value: &str) -> Result<ReflectedValueEnvelope, String> {
    Ok(ReflectedValueEnvelope::typed_ron(
        type_path,
        ron::ser::to_string(value).map_err(|error| error.to_string())?,
    ))
}

fn encode_reflected_value(
    catalog: &TypeRegistrySnapshot,
    type_path: &str,
    value: &ReflectedValue,
) -> Result<ReflectedValueEnvelope, String> {
    Ok(ReflectedValueEnvelope::typed_ron(
        type_path,
        reflected_value_ron(catalog, type_path, value)?,
    ))
}

/// RON for one reflected map value, keyed and valued by the map's own generic
/// arguments.
fn reflected_map_ron(
    catalog: &TypeRegistrySnapshot,
    type_path: &str,
    entries: &[ReflectedMapValueEntry],
) -> Result<String, String> {
    let arguments = generic_arguments(type_path);
    let (key_type, value_type) = arguments
        .first()
        .zip(arguments.get(1))
        .ok_or_else(|| format!("map `{type_path}` has no key/value arguments"))?;
    entries
        .iter()
        .map(|entry| {
            Ok(format!(
                "{}:{}",
                reflected_value_ron(catalog, key_type, &entry.key)?,
                reflected_value_ron(catalog, value_type, &entry.value)?
            ))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(|entries| format!("{{{}}}", entries.join(",")))
}

/// RON for one reflected enum value.
///
/// Only a variant that declares no field at all is spelled bare. A variant that
/// declares fields keeps its body even when the sparse value retains none of
/// them — the producer rejects the bare name for a struct-shaped variant, whose
/// empty form is `Named()`.
fn reflected_enum_ron(
    catalog: &TypeRegistrySnapshot,
    type_path: &str,
    descriptor: &ReflectedTypeDescriptor,
    variant: &str,
    fields: &[(String, ReflectedValue)],
) -> Result<String, String> {
    let variant_descriptor = descriptor
        .variants
        .iter()
        .find(|candidate| candidate.name == *variant)
        .ok_or_else(|| format!("enum `{type_path}` has no variant `{variant}`"))?;
    if fields.is_empty() && variant_descriptor.fields.is_empty() {
        return Ok(variant.to_owned());
    }
    // A sparse value retains any subset of the declared fields, so each
    // retained value is matched to its declaration by the name it carries, the
    // same way the struct arm matches its fields.
    let mut encoded = Vec::new();
    for field in &variant_descriptor.fields {
        let Some((_, value)) = fields.iter().find(|(name, _)| name == &field.name) else {
            continue;
        };
        encoded.push((
            field.name.as_str(),
            reflected_value_ron(catalog, &field.type_path, value)?,
        ));
    }
    let tuple_shaped = variant_descriptor
        .fields
        .iter()
        .all(|field| field.name.parse::<usize>().is_ok());
    let body = if tuple_shaped {
        encoded
            .into_iter()
            .map(|(_, value)| value)
            .collect::<Vec<_>>()
    } else {
        encoded
            .into_iter()
            .map(|(name, value)| format!("{name}:{value}"))
            .collect::<Vec<_>>()
    };
    Ok(format!("{variant}({})", body.join(",")))
}

fn reflected_value_ron(
    catalog: &TypeRegistrySnapshot,
    type_path: &str,
    value: &ReflectedValue,
) -> Result<String, String> {
    let descriptor = reflected_type(catalog, type_path)
        .ok_or_else(|| format!("reflected type `{type_path}` is not loaded"))?;
    match (&descriptor.kind, value) {
        (ReflectedTypeKind::Struct, ReflectedValue::Struct(values)) => {
            let mut fields = Vec::new();
            for field in &descriptor.fields {
                if let Some((_, value)) = values.iter().find(|(name, _)| name == &field.name) {
                    fields.push(format!(
                        "{}:{}",
                        field.name,
                        reflected_value_ron(catalog, &field.type_path, value)?
                    ));
                }
            }
            Ok(format!("({})", fields.join(",")))
        }
        (
            ReflectedTypeKind::Tuple | ReflectedTypeKind::TupleStruct,
            ReflectedValue::Tuple(values),
        ) => descriptor
            .fields
            .iter()
            .zip(values)
            .map(|(field, value)| reflected_value_ron(catalog, &field.type_path, value))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| format!("({})", values.join(","))),
        (
            ReflectedTypeKind::List | ReflectedTypeKind::Array { .. },
            ReflectedValue::List(values),
        ) => {
            let item_type = list_element_type(type_path)?;
            values
                .iter()
                .map(|value| reflected_value_ron(catalog, &item_type, value))
                .collect::<Result<Vec<_>, _>>()
                .map(|values| format!("[{}]", values.join(",")))
        }
        (ReflectedTypeKind::Map, ReflectedValue::Map(entries)) => {
            reflected_map_ron(catalog, type_path, entries)
        }
        (ReflectedTypeKind::Enum, ReflectedValue::Enum { variant, fields }) => {
            reflected_enum_ron(catalog, type_path, descriptor, variant, fields)
        }
        (ReflectedTypeKind::Optional, ReflectedValue::Optional(None)) => Ok("None".to_owned()),
        (ReflectedTypeKind::Optional, ReflectedValue::Optional(Some(value))) => {
            let value_type = list_element_type(type_path)?;
            Ok(format!(
                "Some({})",
                reflected_value_ron(catalog, &value_type, value)?
            ))
        }
        (ReflectedTypeKind::Bool, ReflectedValue::Scalar(ReflectedScalar::Bool(value))) => {
            Ok(value.to_string())
        }
        // Integers and opaque RON are already spelled the way RON wants them,
        // so both pass their text through unchanged.
        (
            ReflectedTypeKind::SignedInteger { .. },
            ReflectedValue::Scalar(ReflectedScalar::Signed(value)),
        )
        | (
            ReflectedTypeKind::UnsignedInteger { .. },
            ReflectedValue::Scalar(ReflectedScalar::Unsigned(value)),
        )
        | (_, ReflectedValue::OpaqueRon(value)) => Ok(value.clone()),
        (
            ReflectedTypeKind::Float { .. },
            ReflectedValue::Scalar(ReflectedScalar::Float(value)),
        ) => Ok(if value.contains(['.', 'e', 'E']) {
            value.clone()
        } else {
            format!("{value}.0")
        }),
        (ReflectedTypeKind::String, ReflectedValue::Scalar(ReflectedScalar::String(value))) => {
            ron::ser::to_string(value).map_err(|error| error.to_string())
        }
        (_, ReflectedValue::Encoded(envelope)) if envelope.type_path == type_path => {
            String::from_utf8(envelope.payload.clone()).map_err(|error| error.to_string())
        }
        (_, ReflectedValue::Unit) => Ok("()".to_owned()),
        _ => Err(format!(
            "reflected value does not match type `{type_path}` ({:?})",
            descriptor.kind
        )),
    }
}

pub fn sync_animation_preview_from_reflected_inspection(
    cx: &mut App,
    inspection: &ReflectedEntityInspection,
) {
    match reflected_blend_space_preview_data(inspection) {
        Ok(Some(document)) => {
            sync_blend_space_preview_document(cx, inspection, document);
            return;
        }
        Ok(None) => {}
        Err(message) => {
            warn!(%message, "failed to project reflected blend-space preview");
        }
    }

    match reflected_mannequin_preview_data(inspection, cx) {
        Ok(Some((source_path, fragments, fragment_blends))) => {
            sync_mannequin_authoring_document(cx, &source_path, fragments, fragment_blends);
        }
        Ok(None) => {}
        Err(message) => {
            warn!(%message, "failed to project reflected Mannequin preview");
        }
    }
}

fn sync_blend_space_preview_document(
    cx: &mut App,
    inspection: &ReflectedEntityInspection,
    document: EditorBlendSpaceData,
) {
    let path = non_empty_string_or(&document.source_path, &inspection.selection.source_path);
    let diagnostics = Vec::new();
    let dimension_count = document.dimensions.len();
    let example_count = document.examples.len();
    let has_vgrid = !document.virtual_examples.is_empty();
    {
        let catalog = cx.default_global::<EditorBlendSpacePreviewCatalog>();
        upsert_blend_space_catalog_entry(
            catalog,
            EditorBlendSpaceAssetData {
                asset_path: path.clone(),
                source_path: document.source_path.clone(),
                name: blend_space_display_name(&path),
                asset_kind: EditorBlendSpaceAssetKind::BlendSpace,
                dimension_count,
                example_count,
                has_vgrid,
                member_paths: Vec::new(),
            },
        );
    }
    let changed = {
        let preview = cx.default_global::<EditorBlendSpacePreview>();
        let before = preview.clone();
        preview.set_document(path.clone(), document, diagnostics);
        *preview != before
    };
    if changed {
        info!(
            blend_space = %path,
            dimension_count,
            example_count,
            "synced authored blend-space document into live preview"
        );
        cx.refresh_windows();
    }
}

fn upsert_blend_space_catalog_entry(
    catalog: &mut EditorBlendSpacePreviewCatalog,
    entry: EditorBlendSpaceAssetData,
) {
    if let Some(existing) = catalog
        .blend_spaces
        .iter_mut()
        .find(|candidate| candidate.asset_path == entry.asset_path)
    {
        *existing = entry;
    } else {
        catalog.blend_spaces.push(entry);
        catalog
            .blend_spaces
            .sort_by(|left, right| left.asset_path.cmp(&right.asset_path));
    }
}

fn sync_mannequin_authoring_document(
    cx: &mut App,
    source_path: &str,
    fragments: Vec<EditorMannequinFragmentData>,
    fragment_blends: Vec<EditorMannequinFragmentBlendData>,
) {
    let motion_catalog = cx
        .try_global::<EditorAnimationPreviewCatalog>()
        .cloned()
        .unwrap_or_else(EditorAnimationPreviewCatalog::empty);
    let (changed, resolved) = {
        let catalog = cx.default_global::<EditorMannequinAuthoringCatalog>();
        let before = catalog.clone();
        catalog
            .fragments
            .retain(|fragment| fragment.source_path != source_path);
        catalog.fragments.extend(fragments);
        catalog
            .fragments
            .sort_by(|left, right| left.key.cmp(&right.key));
        catalog
            .fragment_blends
            .retain(|blend| blend.source_path != source_path);
        catalog.fragment_blends.extend(fragment_blends);
        catalog
            .fragment_blends
            .sort_by(|left, right| left.key.cmp(&right.key));
        let selected_exists = catalog
            .selected_fragment_key
            .as_ref()
            .is_some_and(|selected| {
                catalog
                    .fragments
                    .iter()
                    .any(|fragment| fragment.key == *selected)
            });
        if !selected_exists {
            catalog.selected_fragment_key = catalog
                .fragments
                .iter()
                .find(|fragment| fragment.source_path == source_path)
                .or_else(|| catalog.fragments.first())
                .map(|fragment| fragment.key.clone());
        }
        catalog.resolved = resolve_mannequin_preview_motion(catalog, &motion_catalog);
        (*catalog != before, catalog.resolved.clone())
    };
    let preview_changed = {
        let preview = cx.default_global::<EditorMannequinPreview>();
        apply_resolved_mannequin_preview(preview, resolved.as_ref())
    };
    if changed || preview_changed {
        info!(
            source_path = %source_path,
            resolved_motion = ?resolved.as_ref().and_then(|resolved| resolved.motion_glb.as_deref()),
            "synced authored Mannequin document into live preview"
        );
        cx.refresh_windows();
    }
}

fn reflected_blend_space_preview_data(
    inspection: &ReflectedEntityInspection,
) -> Result<Option<EditorBlendSpaceData>, String> {
    let Some(document_field) = reflected_root_field(inspection, &["blend_space", "document"])
    else {
        return Ok(None);
    };
    let source_path = reflected_root_field(inspection, &["source_path"])
        .and_then(|field| reflected_string(field_value(field)))
        .unwrap_or_else(|| inspection.selection.source_path.clone());
    let dimensions_field = reflected_node_field(&document_field.value, &["dimensions"])
        .ok_or_else(|| "blend-space document has no dimensions field".to_owned())?;
    let dimensions = reflected_blend_space_dimensions(dimensions_field);
    let examples_field = reflected_node_field(&document_field.value, &["examples"])
        .ok_or_else(|| "blend-space document has no examples field".to_owned())?;
    let examples = reflected_blend_space_examples(examples_field, &dimensions)?;
    let virtual_examples = reflected_node_field(&document_field.value, &["virtual_examples"])
        .map(reflected_blend_space_virtual_examples)
        .transpose()?
        .unwrap_or_default();

    Ok(Some(EditorBlendSpaceData {
        source_path: normalize_asset_path(&source_path),
        dimensions,
        examples,
        virtual_examples,
    }))
}

fn reflected_blend_space_dimensions(
    dimensions_field: &ReflectedInspectionField,
) -> Vec<EditorBlendSpaceDimensionData> {
    reflected_list_items(&dimensions_field.value)
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let name = blend_space_dimension_name(value, index);
            let parameter_id = reflected_node_field(value, &["parameter_id"])
                .and_then(|field| reflected_u32(field_value(field)));
            let min = reflected_node_field(value, &["min"])
                .and_then(|field| reflected_float(field_value(field)))
                .unwrap_or(0.0);
            let max = reflected_node_field(value, &["max"])
                .and_then(|field| reflected_float(field_value(field)))
                .unwrap_or(1.0);
            let cells = reflected_node_field(value, &["cells"])
                .and_then(|field| reflected_u32(field_value(field)))
                .unwrap_or(2)
                .max(1) as usize;
            let locked = reflected_node_field(value, &["locked"])
                .and_then(|field| reflected_bool(field_value(field)))
                .unwrap_or(false);
            EditorBlendSpaceDimensionData {
                name,
                parameter_id,
                min,
                max,
                cells,
                locked,
            }
        })
        .collect()
}

fn reflected_blend_space_examples(
    examples_field: &ReflectedInspectionField,
    dimensions: &[EditorBlendSpaceDimensionData],
) -> Result<Vec<EditorBlendSpaceExampleData>, String> {
    reflected_list_items(&examples_field.value)
        .iter()
        .map(|value| reflected_blend_space_example(value, dimensions))
        .collect()
}

fn reflected_blend_space_example(
    value: &ReflectedValueNode,
    dimensions: &[EditorBlendSpaceDimensionData],
) -> Result<EditorBlendSpaceExampleData, String> {
    let playback_scale = reflected_node_field(value, &["playback_scale"])
        .and_then(|field| reflected_float(field_value(field)));
    if let Some(animation_field) = reflected_node_field(value, &["animation"]) {
        if let Some(animation_name) = reflected_string(field_value(animation_field)) {
            let motion_path = normalize_animation_ref(&animation_name)
                .unwrap_or_else(|| normalize_asset_reference_path(&animation_name));
            let coordinates = reflected_blend_space_parameter_coordinates(value, dimensions)?;
            return Ok(EditorBlendSpaceExampleData {
                animation_name: motion_display_name(Path::new(&animation_name)),
                motion_path,
                coordinates,
                playback_scale,
            });
        }
        let animation_name = reflected_node_field(&animation_field.value, &["name"])
            .and_then(|field| reflected_string(field_value(field)))
            .unwrap_or_default();
        let motion_path = reflected_node_field(&animation_field.value, &["motion_path"])
            .and_then(|field| reflected_string(field_value(field)))
            .unwrap_or_else(|| normalize_asset_reference_path(&animation_name));
        let coordinates = reflected_blend_space_named_coordinates(value)?;
        return Ok(EditorBlendSpaceExampleData {
            animation_name: non_empty_string_or(&animation_name, &motion_path),
            motion_path: normalize_asset_reference_path(&motion_path),
            coordinates,
            playback_scale,
        });
    }

    Err("blend-space example has no animation field".to_owned())
}

fn reflected_blend_space_named_coordinates(
    value: &ReflectedValueNode,
) -> Result<Vec<EditorBlendSpaceCoordinateData>, String> {
    let coordinates_field = reflected_node_field(value, &["coordinates"])
        .ok_or_else(|| "blend-space example has no coordinates field".to_owned())?;
    Ok(reflected_list_items(&coordinates_field.value)
        .iter()
        .filter_map(|coordinate| {
            let dimension = reflected_node_field(coordinate, &["dimension"])
                .and_then(|field| reflected_string(field_value(field)))?;
            let value = reflected_node_field(coordinate, &["value"])
                .and_then(|field| reflected_float(field_value(field)))?;
            Some(EditorBlendSpaceCoordinateData { dimension, value })
        })
        .collect())
}

fn reflected_blend_space_parameter_coordinates(
    value: &ReflectedValueNode,
    dimensions: &[EditorBlendSpaceDimensionData],
) -> Result<Vec<EditorBlendSpaceCoordinateData>, String> {
    let parameters_field = reflected_node_field(value, &["parameters"])
        .ok_or_else(|| "blend-space example has no parameters field".to_owned())?;
    Ok(reflected_list_items(&parameters_field.value)
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            let dimension = dimensions.get(index)?;
            let value = reflected_float(value.current.effective.as_ref())?;
            Some(EditorBlendSpaceCoordinateData {
                dimension: dimension.name.clone(),
                value,
            })
        })
        .collect())
}

fn reflected_blend_space_virtual_examples(
    field: &ReflectedInspectionField,
) -> Result<Vec<EditorBlendSpaceVirtualExampleData>, String> {
    reflected_list_items(&field.value)
        .iter()
        .map(|value| {
            let indices_field = reflected_node_field(value, &["indices"])
                .ok_or_else(|| "blend-space virtual example has no indices field".to_owned())?;
            let weights_field = reflected_node_field(value, &["weights"])
                .ok_or_else(|| "blend-space virtual example has no weights field".to_owned())?;
            let indices = reflected_list_items(&indices_field.value);
            let weights = reflected_list_items(&weights_field.value);
            let mut out_indices = Vec::new();
            let mut out_weights = Vec::new();
            for (index, weight) in indices.iter().zip(weights) {
                let Some(weight) = reflected_float(weight.current.effective.as_ref())
                    .filter(|weight| *weight > 0.0)
                else {
                    continue;
                };
                let Some(index) = reflected_u32(index.current.effective.as_ref()) else {
                    continue;
                };
                out_indices.push(index as usize);
                out_weights.push(weight);
            }
            Ok(EditorBlendSpaceVirtualExampleData {
                indices: out_indices,
                weights: out_weights,
            })
        })
        .collect::<Result<Vec<_>, String>>()
}

/// What one inspected mannequin reflects into: the character glTF source path
/// plus the fragment rows and their blend rows. `None` when the inspected
/// entity carries no `database` field and so is not a mannequin at all.
type ReflectedMannequinPreviewData = Option<(
    String,
    Vec<EditorMannequinFragmentData>,
    Vec<EditorMannequinFragmentBlendData>,
)>;

fn reflected_mannequin_preview_data(
    inspection: &ReflectedEntityInspection,
    cx: &App,
) -> Result<ReflectedMannequinPreviewData, String> {
    let Some(database_field) = reflected_root_field(inspection, &["database"]) else {
        return Ok(None);
    };
    let source_path = reflected_root_field(inspection, &["source_path"])
        .and_then(|field| reflected_string(field_value(field)))
        .unwrap_or_else(|| inspection.selection.source_path.clone());
    let motion_catalog = cx
        .try_global::<EditorAnimationPreviewCatalog>()
        .cloned()
        .unwrap_or_else(EditorAnimationPreviewCatalog::empty);
    let known_motion_paths = motion_catalog
        .motions
        .iter()
        .map(|motion| motion.asset_path.as_str())
        .collect::<BTreeSet<_>>();
    let definition_by_name = cx
        .try_global::<EditorMannequinAuthoringCatalog>()
        .map(|catalog| {
            catalog
                .fragment_definitions
                .iter()
                .map(|definition| (definition.name.as_str(), definition))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let fragments = reflected_mannequin_fragments(
        &database_field.value,
        &source_path,
        &known_motion_paths,
        &definition_by_name,
    )?;
    let blends = reflected_mannequin_fragment_blends(&database_field.value, &source_path)?;
    Ok(Some((
        normalize_asset_path(&source_path),
        fragments,
        blends,
    )))
}

fn reflected_mannequin_fragments(
    database: &ReflectedValueNode,
    source_path: &str,
    known_motion_paths: &BTreeSet<&str>,
    definition_by_name: &BTreeMap<&str, &EditorMannequinFragmentDefinitionData>,
) -> Result<Vec<EditorMannequinFragmentData>, String> {
    let groups_field = reflected_node_field(database, &["fragment_groups"])
        .ok_or_else(|| "Mannequin database has no fragment_groups field".to_owned())?;
    reflected_list_items(&groups_field.value)
        .iter()
        .map(|group| {
            let name = reflected_node_field(group, &["name"])
                .and_then(|field| reflected_string(field_value(field)))
                .unwrap_or_default();
            let key = mannequin_fragment_key(source_path, &name);
            let fragments_field = reflected_node_field(group, &["fragments"])
                .ok_or_else(|| "Mannequin fragment group has no fragments field".to_owned())?;
            let options = reflected_list_items(&fragments_field.value)
                .iter()
                .enumerate()
                .map(|(index, fragment)| {
                    reflected_mannequin_fragment_option(&key, index, fragment, known_motion_paths)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let definition = definition_by_name.get(name.as_str()).copied();
            Ok(EditorMannequinFragmentData {
                key,
                name,
                source_path: source_path.to_owned(),
                option_count: options.len(),
                options,
                scopes: definition
                    .map(|definition| definition.scopes.clone())
                    .unwrap_or_default(),
                flags: definition.and_then(|definition| definition.flags.clone()),
            })
        })
        .collect()
}

fn reflected_mannequin_fragment_option(
    fragment_key: &str,
    index: usize,
    fragment: &ReflectedValueNode,
    known_motion_paths: &BTreeSet<&str>,
) -> Result<EditorMannequinFragmentOptionData, String> {
    let condition = reflected_node_field(fragment, &["tags"])
        .and_then(|field| reflected_string(field_value(field)));
    let fragment_tags = reflected_node_field(fragment, &["fragment_tags"])
        .and_then(|field| reflected_string(field_value(field)));
    let animation_refs = reflected_mannequin_animation_refs(fragment, known_motion_paths)?;
    let condition = parse_tag_tokens(condition.as_deref());
    let fragment_tags = parse_tag_tokens(fragment_tags.as_deref()).required;
    Ok(EditorMannequinFragmentOptionData {
        key: format!("{fragment_key}:{index}"),
        index,
        required_tags: condition.required,
        excluded_tags: condition.excluded,
        fragment_tags,
        animation_refs,
    })
}

fn reflected_mannequin_animation_refs(
    fragment: &ReflectedValueNode,
    known_motion_paths: &BTreeSet<&str>,
) -> Result<Vec<EditorMannequinAnimationRefData>, String> {
    let layers_field = reflected_node_field(fragment, &["animation_layers"])
        .ok_or_else(|| "Mannequin fragment has no animation_layers field".to_owned())?;
    let layers = reflected_list_items(&layers_field.value);
    let mut refs = Vec::new();
    for (layer_index, layer) in layers.iter().enumerate() {
        let animations_field = reflected_node_field(layer, &["animations"])
            .ok_or_else(|| "Mannequin layer has no animations field".to_owned())?;
        let animations = reflected_list_items(&animations_field.value);
        for (animation_index, animation) in animations.iter().enumerate() {
            let name = reflected_node_field(animation, &["name"])
                .and_then(|field| reflected_string(field_value(field)))
                .unwrap_or_default();
            let motion_glb = normalize_animation_ref(&name);
            let unresolved = motion_glb
                .as_deref()
                .is_none_or(|motion_glb| !known_motion_paths.contains(motion_glb));
            refs.push(EditorMannequinAnimationRefData {
                original: name,
                motion_glb,
                unresolved,
                layer_index,
                animation_index,
            });
        }
    }
    Ok(refs)
}

fn reflected_mannequin_fragment_blends(
    database: &ReflectedValueNode,
    source_path: &str,
) -> Result<Vec<EditorMannequinFragmentBlendData>, String> {
    let Some(blends_field) = reflected_node_field(database, &["fragment_blends"]) else {
        return Ok(Vec::new());
    };
    reflected_list_items(&blends_field.value)
        .iter()
        .enumerate()
        .map(|(index, blend)| {
            let from = reflected_node_field(blend, &["from"])
                .and_then(|field| reflected_string(field_value(field)));
            let to = reflected_node_field(blend, &["to"])
                .and_then(|field| reflected_string(field_value(field)));
            let variants = reflected_node_field(blend, &["variants"])
                .map(|field| reflected_list_items(&field.value).len())
                .unwrap_or_default();
            Ok(EditorMannequinFragmentBlendData {
                key: format!("{source_path}#blend:{index}"),
                source_path: source_path.to_owned(),
                from,
                to,
                variant_count: variants,
                fragment_count: variants,
            })
        })
        .collect::<Result<Vec<_>, String>>()
}

fn update_mannequin_preview(cx: &mut App, action: MannequinPreviewAction) -> bool {
    let character_changed = matches!(action, MannequinPreviewAction::SelectCharacter(_));
    let preview = cx.default_global::<EditorMannequinPreview>();
    let changed = apply_animation_preview_action(preview, action);
    if changed {
        info!(
            character = ?preview.character_glb,
            motion = ?preview.motion_glb,
            playing = preview.playing,
            looping = preview.looping,
            position_millis = preview.position_millis,
            "updated mannequin animation preview"
        );
        if character_changed {
            invalidate_animation_catalog_input(cx);
        }
        cx.refresh_windows();
    }
    changed
}

fn update_blend_space_preview(cx: &mut App, action: BlendSpacePreviewAction) -> bool {
    let asset_root = cx
        .try_global::<EditorBlendSpacePreview>()
        .and_then(|preview| preview.project_asset_root.clone())
        .or_else(|| {
            cx.try_global::<EditorBlendSpacePreviewCatalog>()
                .and_then(|catalog| catalog.project_asset_root.clone())
        });
    let Some(asset_root) = asset_root else {
        return false;
    };

    let preview = cx.default_global::<EditorBlendSpacePreview>();
    let changed = apply_blend_space_preview_action(preview, &asset_root, action);
    if changed {
        info!(
            blend_space = ?preview.bspace_ron_path,
            params = ?preview.param_values,
            weight_count = preview.weights.len(),
            "updated mannequin blend-space preview"
        );
        cx.refresh_windows();
    }
    changed
}

fn update_mannequin_authoring(cx: &mut App, action: MannequinAuthoringAction) -> bool {
    let motion_catalog = cx
        .try_global::<EditorAnimationPreviewCatalog>()
        .cloned()
        .unwrap_or_else(EditorAnimationPreviewCatalog::empty);
    let (changed, resolved) = {
        let catalog = cx.default_global::<EditorMannequinAuthoringCatalog>();
        let changed = apply_mannequin_authoring_action(catalog, &motion_catalog, action);
        (changed, catalog.resolved.clone())
    };

    if changed {
        let preview_changed = {
            let preview = cx.default_global::<EditorMannequinPreview>();
            apply_resolved_mannequin_preview(preview, resolved.as_ref())
        };
        let selected_fragment = cx
            .try_global::<EditorMannequinAuthoringCatalog>()
            .and_then(|catalog| catalog.selected_fragment_key.clone());
        let enabled_tags = cx
            .try_global::<EditorMannequinAuthoringCatalog>()
            .map(|catalog| catalog.enabled_tags.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        info!(
            ?selected_fragment,
            ?enabled_tags,
            resolved_motion = ?resolved.as_ref().and_then(|resolved| resolved.motion_glb.as_deref()),
            unresolved = resolved.as_ref().is_some_and(|resolved| resolved.unresolved),
            preview_changed,
            "updated mannequin authoring state"
        );
        cx.refresh_windows();
    }
    changed
}

/// Retains the cancellation sender for the attached mannequin catalog work.
/// Dropping or replacing this slot stops the detached invalidation task.
pub struct EditorMannequinAnimationController {
    _close: tokio::sync::watch::Sender<()>,
}

// The `ControllerInstaller` fn-pointer table in `controller_set` fixes this
// signature; every slot installer takes the session by value.
#[allow(clippy::needless_pass_by_value)]
#[instrument(skip(cx, session))]
pub fn install_mannequin_animation_slot(
    cx: &mut App,
    session: EditorAttachSession,
    fence: crate::controller_set::ControllerFence,
) {
    let asset_root = session.project_root.join("assets");

    cx.set_global(EditorAnimationPreviewCatalog::empty());
    cx.set_global(EditorMannequinAuthoringCatalog::empty());
    cx.set_global(EditorBlendSpacePreviewCatalog::empty());
    cx.set_global(EditorBlendSpacePreview::with_project_asset_root(
        asset_root.clone(),
    ));

    let (close, mut close_rx) = tokio::sync::watch::channel(());
    if !crate::controller_set::complete_mannequin_animation(
        cx,
        fence,
        EditorMannequinAnimationController { _close: close },
    ) {
        return;
    }
    let mut status_changes = subscribe_animation_catalog_input_invalidation(cx);

    // The initial attach plus subsequent status/preview publications are the
    // causal inputs. `watch` coalesces changes while parsing happens off-main
    // thread, so the catalog never needs a signature scan or frame cadence.
    cx.spawn(async move |cx| {
        let mut rebuild = true;
        info!(
            asset_root = %asset_root.display(),
            "installed mannequin animation catalog invalidation subscription"
        );
        loop {
            if !rebuild {
                tokio::select! {
                    changed = status_changes.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    _ = close_rx.changed() => break,
                }
            }
            rebuild = false;
            let step = cx.update(move |cx| {
                if !crate::controller_set::is_current_fence(cx, fence) {
                    return None;
                }
                let status = cx.try_global::<EditorAssetBrowserStatus>()?;
                let character_glb = cx
                    .try_global::<EditorMannequinPreview>()
                    .and_then(|preview| preview.character_glb.clone());
                Some((Box::new(status.clone()), character_glb))
            });

            let Some((status, character_glb)) = step else {
                continue;
            };

            // Heavy glTF/RON parsing runs off the main thread.
            let build_asset_root = asset_root.clone();
            let (catalog, authoring_catalog, blend_space_catalog) = cx
                .background_executor()
                .spawn(async move {
                    let catalog = build_animation_preview_catalog_from_status(
                        &status,
                        &build_asset_root,
                        character_glb.as_deref(),
                    );
                    let authoring_catalog =
                        build_mannequin_authoring_catalog(&build_asset_root, &catalog);
                    let blend_space_catalog = build_blend_space_preview_catalog(&build_asset_root);
                    (catalog, authoring_catalog, blend_space_catalog)
                })
                .await;

            let motion_count = catalog.motions.len();
            let joint_count = catalog.skeleton_joints.len();
            let diagnostic_count = catalog.diagnostics.len();
            let fragment_count = authoring_catalog.fragments.len();
            let blend_space_count = blend_space_catalog.blend_spaces.len();
            let cancelled = close_rx.has_changed().unwrap_or(true);
            let published = cx.update(move |cx| {
                if cancelled || !crate::controller_set::is_current_fence(cx, fence) {
                    return false;
                }
                publish_animation_preview_catalog(cx, catalog);
                publish_mannequin_authoring_catalog(cx, authoring_catalog);
                publish_blend_space_preview_catalog(cx, blend_space_catalog);
                true
            });
            if !published {
                break;
            }
            info!(
                motion_count,
                joint_count,
                fragment_count,
                blend_space_count,
                diagnostic_count,
                "published mannequin animation preview catalog from asset-pipeline workspace status"
            );
        }
    })
    .detach();
}

fn publish_animation_preview_catalog(cx: &mut App, catalog: EditorAnimationPreviewCatalog) {
    {
        let preview = cx.default_global::<EditorMannequinPreview>();
        reconcile_mannequin_preview_with_catalog(preview, &catalog);
    }
    cx.set_global(catalog);
    cx.refresh_windows();
}

fn publish_mannequin_authoring_catalog(cx: &mut App, mut catalog: EditorMannequinAuthoringCatalog) {
    let motion_catalog = cx
        .try_global::<EditorAnimationPreviewCatalog>()
        .cloned()
        .unwrap_or_else(EditorAnimationPreviewCatalog::empty);
    {
        let preview = cx.default_global::<EditorMannequinPreview>();
        reconcile_mannequin_authoring_catalog(&mut catalog, &motion_catalog, preview);
    }
    cx.set_global(catalog);
    cx.refresh_windows();
}

fn publish_blend_space_preview_catalog(cx: &mut App, catalog: EditorBlendSpacePreviewCatalog) {
    let selected_path = {
        let preview = cx.default_global::<EditorBlendSpacePreview>();
        reconcile_blend_space_preview_with_catalog(preview, &catalog);
        preview.bspace_ron_path.clone()
    };
    cx.set_global(catalog);
    if let Some(selected_path) = selected_path {
        let asset_root = cx
            .try_global::<EditorBlendSpacePreviewCatalog>()
            .and_then(|catalog| catalog.project_asset_root.clone());
        if let Some(asset_root) = asset_root {
            let preview = cx.default_global::<EditorBlendSpacePreview>();
            if preview.document.is_none() {
                let _ = apply_blend_space_preview_action(
                    preview,
                    &asset_root,
                    BlendSpacePreviewAction::SelectBlendSpace(selected_path),
                );
            }
        }
    }
    cx.refresh_windows();
}

pub fn reconcile_mannequin_preview_with_catalog(
    preview: &mut EditorMannequinPreview,
    catalog: &EditorAnimationPreviewCatalog,
) {
    if let Some(asset_root) = catalog.project_asset_root.clone() {
        preview.project_asset_root = Some(asset_root);
    }

    let selected_motion_exists = preview.motion_glb.as_ref().is_some_and(|motion_glb| {
        catalog
            .motions
            .iter()
            .any(|motion| motion.asset_path == *motion_glb)
    });

    if selected_motion_exists {
        return;
    }

    if let Some(first_motion) = catalog.motions.first() {
        preview.motion_glb = Some(first_motion.asset_path.clone());
        preview.playing = true;
    } else {
        preview.motion_glb = None;
        preview.playing = false;
    }
    preview.position_millis = 0;
}

pub fn reconcile_blend_space_preview_with_catalog(
    preview: &mut EditorBlendSpacePreview,
    catalog: &EditorBlendSpacePreviewCatalog,
) {
    if let Some(asset_root) = catalog.project_asset_root.clone() {
        preview.project_asset_root = Some(asset_root);
    }

    let selected_exists = preview.bspace_ron_path.as_ref().is_some_and(|selected| {
        catalog
            .blend_spaces
            .iter()
            .any(|blend_space| blend_space.asset_path == *selected)
    });
    if selected_exists {
        return;
    }

    if let Some(first) = catalog.blend_spaces.first() {
        preview.bspace_ron_path = Some(first.asset_path.clone());
        preview.document = None;
        preview.param_values.clear();
        preview.weights.clear();
        preview.diagnostics.clear();
    } else {
        preview.clear_selection();
        preview
            .project_asset_root
            .clone_from(&catalog.project_asset_root);
    }
}

pub fn reconcile_mannequin_authoring_catalog(
    catalog: &mut EditorMannequinAuthoringCatalog,
    motion_catalog: &EditorAnimationPreviewCatalog,
    preview: &mut EditorMannequinPreview,
) {
    let known_tags = catalog
        .tags
        .iter()
        .map(|tag| tag.name.as_str())
        .collect::<BTreeSet<_>>();
    catalog.enabled_tags = catalog
        .enabled_tags
        .iter()
        .filter(|tag| known_tags.contains(tag.as_str()))
        .cloned()
        .collect();

    let selected_exists = catalog
        .selected_fragment_key
        .as_ref()
        .is_some_and(|selected| {
            catalog
                .fragments
                .iter()
                .any(|fragment| fragment.key == *selected)
        });
    if !selected_exists {
        catalog.selected_fragment_key = catalog
            .fragments
            .first()
            .map(|fragment| fragment.key.clone());
    }

    catalog.resolved = resolve_mannequin_preview_motion(catalog, motion_catalog);
    if catalog.selected_fragment_key.is_some() {
        apply_resolved_mannequin_preview(preview, catalog.resolved.as_ref());
    }
}

pub fn resolve_mannequin_preview_motion(
    catalog: &EditorMannequinAuthoringCatalog,
    motion_catalog: &EditorAnimationPreviewCatalog,
) -> Option<EditorMannequinResolvedAnimationData> {
    let fragment = catalog.selected_fragment()?;
    let Some(option) = best_matching_fragment_option(fragment, &catalog.enabled_tags) else {
        return Some(EditorMannequinResolvedAnimationData {
            fragment_key: fragment.key.clone(),
            option_key: String::new(),
            animation_ref: None,
            motion_glb: None,
            unresolved: true,
            reason: Some("no fragment option matches the active Mannequin tag state".to_owned()),
            required_tags: Vec::new(),
            excluded_tags: Vec::new(),
        });
    };

    let Some(animation_ref) = option.animation_refs.first() else {
        return Some(EditorMannequinResolvedAnimationData {
            fragment_key: fragment.key.clone(),
            option_key: option.key.clone(),
            animation_ref: None,
            motion_glb: None,
            unresolved: true,
            reason: Some("matching fragment option has no animation references".to_owned()),
            required_tags: option.required_tags.clone(),
            excluded_tags: option.excluded_tags.clone(),
        });
    };

    let motion_glb = animation_ref.motion_glb.as_ref().and_then(|motion_glb| {
        motion_catalog
            .motions
            .iter()
            .any(|motion| motion.asset_path == *motion_glb)
            .then(|| motion_glb.clone())
    });
    let unresolved = animation_ref.unresolved || motion_glb.is_none();
    let reason = if unresolved {
        Some(if animation_ref.motion_glb.is_some() {
            "animation reference is not present in the extracted .anim.glb catalog".to_owned()
        } else {
            "animation reference is not a resolved .anim.glb asset path".to_owned()
        })
    } else {
        None
    };

    Some(EditorMannequinResolvedAnimationData {
        fragment_key: fragment.key.clone(),
        option_key: option.key.clone(),
        animation_ref: Some(animation_ref.original.clone()),
        motion_glb,
        unresolved,
        reason,
        required_tags: option.required_tags.clone(),
        excluded_tags: option.excluded_tags.clone(),
    })
}

fn best_matching_fragment_option<'a>(
    fragment: &'a EditorMannequinFragmentData,
    enabled_tags: &BTreeSet<String>,
) -> Option<&'a EditorMannequinFragmentOptionData> {
    fragment
        .options
        .iter()
        .filter(|option| mannequin_option_matches_tags(option, enabled_tags))
        .max_by(|left, right| {
            let left_score = left.required_tags.len() + left.excluded_tags.len();
            let right_score = right.required_tags.len() + right.excluded_tags.len();
            left_score
                .cmp(&right_score)
                .then_with(|| right.index.cmp(&left.index))
        })
}

fn mannequin_option_matches_tags(
    option: &EditorMannequinFragmentOptionData,
    enabled_tags: &BTreeSet<String>,
) -> bool {
    option
        .required_tags
        .iter()
        .all(|tag| enabled_tags.contains(tag))
        && option
            .excluded_tags
            .iter()
            .all(|tag| !enabled_tags.contains(tag))
}

fn set_mannequin_tag_state(
    catalog: &mut EditorMannequinAuthoringCatalog,
    tag: &str,
    enabled: bool,
) {
    if enabled {
        if let Some(group) = catalog.tag_group_for_tag(tag).map(str::to_owned) {
            let group_tags = catalog
                .tag_groups
                .iter()
                .find(|tag_group| tag_group.name == group)
                .map(|tag_group| tag_group.tags.clone())
                .unwrap_or_default();
            for grouped_tag in group_tags {
                catalog.enabled_tags.remove(&grouped_tag);
            }
        }
        catalog.enabled_tags.insert(tag.to_owned());
    } else {
        catalog.enabled_tags.remove(tag);
    }
}

pub fn apply_resolved_mannequin_preview(
    preview: &mut EditorMannequinPreview,
    resolved: Option<&EditorMannequinResolvedAnimationData>,
) -> bool {
    let before = preview.clone();
    if let Some(motion_glb) = resolved
        .filter(|resolved| !resolved.unresolved)
        .and_then(|resolved| resolved.motion_glb.as_deref())
    {
        preview.select_motion(motion_glb.to_owned());
    } else if resolved.is_some() {
        preview.motion_glb = None;
        preview.playing = false;
        preview.position_millis = 0;
    }
    *preview != before
}

/// Source-schema type the asset pipeline assigns to `.anim.glb` motion
/// sources (`azoth.animation.AnimationSource`, registered by `az-animation` —
/// see `crates/az/animation/src/lib.rs`). Kept as a literal here so the editor
/// does not link the builder crate for one identifier.
pub const ANIMATION_SOURCE_SCHEMA_TYPE: &str = "azoth.animation.AnimationSource";

/// One animation motion source resolved from the attached workspace snapshot
/// and entry page: the loader path (project-asset-root relative, what the
/// in-process bevy preview consumes), the absolute source file (via the
/// workspace roots, for glTF metadata parsing), and the pipeline status label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimationMotionSource {
    pub asset_path: String,
    pub absolute_path: PathBuf,
    pub pipeline_status: String,
}

fn is_animation_source_entry(entry: &AssetBrowserEntryData) -> bool {
    entry.schema_type.as_deref() == Some(ANIMATION_SOURCE_SCHEMA_TYPE)
}

/// Derive the animation motion sources from the asset-processor workspace view:
/// entries with the animation source schema, resolved to absolute files via
/// the view's source roots, excluding deleted entries. Motions outside the
/// project asset root are skipped with a diagnostic — the in-process preview
/// loader resolves paths against that root only. Pure.
#[must_use]
pub fn animation_motion_sources_from_asset_entries(
    status: &EditorAssetBrowserStatus,
    project_asset_root: &Path,
    diagnostics: &mut Vec<String>,
) -> Vec<AnimationMotionSource> {
    let root_by_scan_folder: BTreeMap<i64, &str> = status
        .roots
        .iter()
        .map(|root| (root.root_id, root.source_root.as_str()))
        .collect();

    let mut sources = Vec::new();
    for entry in &status.entries {
        if !is_animation_source_entry(entry) {
            continue;
        }
        if entry.status == AssetBrowserEntryStatus::Deleted {
            continue;
        }
        let Some(source_root) = root_by_scan_folder.get(&entry.root_id) else {
            diagnostics.push(format!(
                "animation source {} references unknown scan folder {}",
                entry.source_path, entry.root_id
            ));
            continue;
        };
        let absolute_path = Path::new(source_root).join(Path::new(&entry.source_path));
        let Ok(relative) = absolute_path.strip_prefix(project_asset_root) else {
            diagnostics.push(format!(
                "animation source {} lives outside the project asset root ({}); the \
                 in-process preview loader cannot resolve it",
                entry.source_path, source_root
            ));
            continue;
        };
        let mut pipeline_status = entry.status.label().to_owned();
        if let Some(job) = &entry.latest_job {
            pipeline_status.push_str(" · job ");
            pipeline_status.push_str(job.status.label());
        }
        sources.push(AnimationMotionSource {
            asset_path: relative.to_string_lossy().replace('\\', "/"),
            absolute_path,
            pipeline_status,
        });
    }
    sources.sort_by(|left, right| left.asset_path.cmp(&right.asset_path));
    sources
}

/// Build the animation preview catalog from asset-pipeline truth: the motion
/// list comes from the asset processor's workspace asset entries (animation
/// source schema), not a filesystem walk, so it reflects exactly what the AP
/// tracks — with per-source job status attached. glTF metadata (duration,
/// channels, events) is read from each source file resolved through the
/// view's source roots. The preview character is not an AP-tracked source (no
/// mesh builder is registered), so its skeleton still reads directly from the
/// selected character glTF under the asset root.
pub fn build_animation_preview_catalog_from_status(
    status: &EditorAssetBrowserStatus,
    project_asset_root: impl AsRef<Path>,
    character_glb: Option<&str>,
) -> EditorAnimationPreviewCatalog {
    let project_asset_root = project_asset_root.as_ref().to_path_buf();
    let mut diagnostics = Vec::new();
    if let Some(status_error) = &status.status_error {
        diagnostics.push(format!("asset processor status error: {status_error}"));
    }

    let sources =
        animation_motion_sources_from_asset_entries(status, &project_asset_root, &mut diagnostics);
    let motions = sources
        .iter()
        .map(|source| {
            let mut motion =
                read_motion_data(&project_asset_root, &source.absolute_path, &mut diagnostics);
            motion.pipeline_status = Some(source.pipeline_status.clone());
            motion
        })
        .collect::<Vec<_>>();

    let skeleton_joints = character_glb
        .and_then(|character_glb| {
            let character_path = project_asset_root.join(Path::new(character_glb));
            if character_path.exists() {
                Some(read_skeleton_joints(&character_path, &mut diagnostics))
            } else {
                diagnostics.push(format!(
                    "mannequin character glTF not found: {}",
                    normalize_relative_path(&project_asset_root, &character_path)
                ));
                None
            }
        })
        .unwrap_or_default();

    EditorAnimationPreviewCatalog::new(
        Some(project_asset_root),
        motions,
        skeleton_joints,
        diagnostics,
    )
}

/// The parsed Mannequin authoring RON under a project's mannequin root,
/// grouped by the schema each file carries.
#[derive(Default)]
struct MannequinAuthoringSources {
    animation_databases: Vec<(String, RonMannequinAnimationDatabaseSource)>,
    tags: Vec<RonMannequinTagDefinitionSource>,
    controllers: Vec<RonMannequinControllerDefinitionSource>,
}

/// Read and parse every Mannequin authoring RON under `mannequin_root`,
/// recording per-file read and parse failures as catalog diagnostics.
fn parse_mannequin_authoring_sources(
    project_asset_root: &Path,
    mannequin_root: &Path,
    diagnostics: &mut Vec<String>,
) -> MannequinAuthoringSources {
    let mut ron_paths = Vec::new();
    collect_mannequin_ron_files(mannequin_root, &mut ron_paths, diagnostics);
    ron_paths.sort_by(|left, right| {
        normalize_relative_path(project_asset_root, left)
            .cmp(&normalize_relative_path(project_asset_root, right))
    });

    let mut sources = MannequinAuthoringSources::default();
    for path in ron_paths {
        let rel = normalize_relative_path(project_asset_root, &path);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                diagnostics.push(format!(
                    "failed to read Mannequin authoring RON {rel}: {error}"
                ));
                continue;
            }
        };
        let lower = rel.to_ascii_lowercase();
        if lower.ends_with(".adb.ron") {
            match ron::de::from_bytes::<RonMannequinAnimationDatabaseSource>(&bytes) {
                Ok(source) => sources.animation_databases.push((rel, source)),
                Err(error) => {
                    diagnostics.push(format!("failed to parse Mannequin ADB RON {rel}: {error}"));
                }
            }
        } else if lower.ends_with(".mannequin.tags.ron")
            || lower.ends_with(".mannequin.actions.ron")
        {
            match ron::de::from_bytes::<RonMannequinTagDefinitionSource>(&bytes) {
                Ok(source) => sources.tags.push(source),
                Err(error) => {
                    diagnostics.push(format!("failed to parse Mannequin tag RON {rel}: {error}"));
                }
            }
        } else if lower.ends_with(".mannequin.controller.ron") {
            match ron::de::from_bytes::<RonMannequinControllerDefinitionSource>(&bytes) {
                Ok(source) => sources.controllers.push(source),
                Err(error) => diagnostics.push(format!(
                    "failed to parse Mannequin controller RON {rel}: {error}"
                )),
            }
        }
    }
    sources
}

/// The scope contexts and scopes the controller definitions declare, flattened
/// across every controller file.
fn mannequin_scopes_from_controllers(
    controllers: &[RonMannequinControllerDefinitionSource],
) -> (
    Vec<EditorMannequinScopeContextData>,
    Vec<EditorMannequinScopeData>,
) {
    let scope_contexts = controllers
        .iter()
        .flat_map(|source| {
            source
                .scope_contexts
                .iter()
                .map(|scope_context| EditorMannequinScopeContextData {
                    name: scope_context.name.clone(),
                })
        })
        .collect();
    let scopes = controllers
        .iter()
        .flat_map(|source| {
            source.scopes.iter().map(|scope| EditorMannequinScopeData {
                name: scope.name.clone(),
                layer: scope.layer,
                num_layers: scope.num_layers,
                context: scope.context.clone(),
                tags: parse_tag_tokens(scope.tags.as_deref()).required,
            })
        })
        .collect();
    (scope_contexts, scopes)
}

/// Order and de-duplicate every authoring list so the panels render stably no
/// matter what order the source files were discovered in.
fn sort_mannequin_authoring_catalog(catalog: &mut EditorMannequinAuthoringCatalog) {
    catalog.fragments.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source_path.cmp(&right.source_path))
            .then_with(|| left.key.cmp(&right.key))
    });
    catalog.tags.sort_by(|left, right| {
        left.group
            .cmp(&right.group)
            .then_with(|| left.name.cmp(&right.name))
    });
    catalog
        .tags
        .dedup_by(|left, right| left.name == right.name && left.group == right.group);
    catalog
        .tag_groups
        .sort_by(|left, right| left.name.cmp(&right.name));
    catalog
        .tag_groups
        .dedup_by(|left, right| left.name == right.name);
    catalog.scopes.sort_by(|left, right| {
        left.context
            .cmp(&right.context)
            .then_with(|| left.layer.cmp(&right.layer))
            .then_with(|| left.name.cmp(&right.name))
    });
    catalog
        .scope_contexts
        .sort_by(|left, right| left.name.cmp(&right.name));
    catalog
        .scope_contexts
        .dedup_by(|left, right| left.name == right.name);
}

pub fn build_mannequin_authoring_catalog(
    project_asset_root: impl AsRef<Path>,
    motion_catalog: &EditorAnimationPreviewCatalog,
) -> EditorMannequinAuthoringCatalog {
    let project_asset_root = project_asset_root.as_ref().to_path_buf();
    let mut catalog = EditorMannequinAuthoringCatalog::new(Some(project_asset_root.clone()));
    let mannequin_root = project_asset_root.join("animations").join("mannequin");
    if !mannequin_root.exists() {
        return catalog;
    }

    let sources = parse_mannequin_authoring_sources(
        &project_asset_root,
        &mannequin_root,
        &mut catalog.diagnostics,
    );
    catalog.tags = mannequin_tags_from_sources(&sources.tags);
    catalog.tag_groups = mannequin_tag_groups_from_sources(&sources.tags);
    catalog.fragment_definitions =
        mannequin_fragment_definitions_from_controllers(&sources.controllers);
    let (scope_contexts, scopes) = mannequin_scopes_from_controllers(&sources.controllers);
    catalog.scope_contexts = scope_contexts;
    catalog.scopes = scopes;

    let known_motion_paths = motion_catalog
        .motions
        .iter()
        .map(|motion| motion.asset_path.as_str())
        .collect::<BTreeSet<_>>();
    let fragment_definition_by_name = catalog
        .fragment_definitions
        .iter()
        .map(|definition| (definition.name.as_str(), definition))
        .collect::<BTreeMap<_, _>>();
    for (source_path, source) in &sources.animation_databases {
        catalog.fragments.extend(mannequin_fragments_from_database(
            source_path,
            source,
            &known_motion_paths,
            &fragment_definition_by_name,
        ));
        catalog
            .fragment_blends
            .extend(mannequin_fragment_blends_from_database(source_path, source));
    }

    sort_mannequin_authoring_catalog(&mut catalog);
    catalog
}

pub fn build_blend_space_preview_catalog(
    project_asset_root: impl AsRef<Path>,
) -> EditorBlendSpacePreviewCatalog {
    let project_asset_root = project_asset_root.as_ref().to_path_buf();
    let mut diagnostics = Vec::new();
    let mut paths = Vec::new();
    let animation_root = project_asset_root.join("animations");
    if animation_root.exists() {
        collect_blend_space_ron_files(&animation_root, &mut paths, &mut diagnostics);
    }
    paths.sort_by(|left, right| {
        normalize_relative_path(&project_asset_root, left)
            .cmp(&normalize_relative_path(&project_asset_root, right))
    });

    let blend_spaces = paths
        .into_iter()
        .filter_map(|path| {
            let asset_path = normalize_relative_path(&project_asset_root, &path);
            if is_combined_blend_space_ron_path(&path) {
                combined_blend_space_asset_data(&project_asset_root, &asset_path, &mut diagnostics)
            } else {
                blend_space_asset_data(&project_asset_root, &asset_path, &mut diagnostics)
            }
        })
        .collect();

    EditorBlendSpacePreviewCatalog::new(Some(project_asset_root), blend_spaces, diagnostics)
}

pub fn load_blend_space_document(
    project_asset_root: &Path,
    bspace_ron_path: &str,
) -> Result<(EditorBlendSpaceData, Vec<String>), String> {
    let path = project_asset_root.join(Path::new(bspace_ron_path));
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "failed to read blend-space RON {}: {error}",
            normalize_relative_path(project_asset_root, &path)
        )
    })?;
    let source = BlendSpaceSource::from_ron_bytes(&bytes).map_err(|error| {
        format!(
            "failed to parse blend-space RON {}: {error}",
            normalize_relative_path(project_asset_root, &path)
        )
    })?;

    let mut diagnostics = Vec::new();
    let document = blend_space_document_from_ron(project_asset_root, &source, &mut diagnostics);
    Ok((document, diagnostics))
}

fn blend_space_asset_data(
    project_asset_root: &Path,
    asset_path: &str,
    diagnostics: &mut Vec<String>,
) -> Option<EditorBlendSpaceAssetData> {
    match load_blend_space_document(project_asset_root, asset_path) {
        Ok((document, mut document_diagnostics)) => {
            diagnostics.append(&mut document_diagnostics);
            Some(EditorBlendSpaceAssetData {
                source_path: document.source_path.clone(),
                name: blend_space_display_name(asset_path),
                asset_kind: EditorBlendSpaceAssetKind::BlendSpace,
                dimension_count: document.dimensions.len(),
                example_count: document.examples.len(),
                has_vgrid: !document.virtual_examples.is_empty(),
                member_paths: Vec::new(),
                asset_path: asset_path.to_owned(),
            })
        }
        Err(error) => {
            diagnostics.push(error);
            None
        }
    }
}

fn combined_blend_space_asset_data(
    project_asset_root: &Path,
    asset_path: &str,
    diagnostics: &mut Vec<String>,
) -> Option<EditorBlendSpaceAssetData> {
    match load_combined_blend_space_source(project_asset_root, asset_path) {
        Ok(source) => Some(EditorBlendSpaceAssetData {
            source_path: non_empty_string_or(&source.source_path, asset_path),
            name: blend_space_display_name(asset_path),
            asset_kind: EditorBlendSpaceAssetKind::CombinedBlendSpace,
            dimension_count: source.combined_blend_space.dimensions.len(),
            example_count: 0,
            has_vgrid: false,
            member_paths: source
                .combined_blend_space
                .blend_spaces
                .iter()
                .filter_map(combined_blend_space_member_path)
                .collect(),
            asset_path: asset_path.to_owned(),
        }),
        Err(error) => {
            diagnostics.push(error);
            None
        }
    }
}

fn load_combined_blend_space_source(
    project_asset_root: &Path,
    comb_ron_path: &str,
) -> Result<CombinedBlendSpaceSource, String> {
    let path = project_asset_root.join(Path::new(comb_ron_path));
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "failed to read combined blend-space RON {}: {error}",
            normalize_relative_path(project_asset_root, &path)
        )
    })?;
    CombinedBlendSpaceSource::from_ron_bytes(&bytes).map_err(|error| {
        format!(
            "failed to parse combined blend-space RON {}: {error}",
            normalize_relative_path(project_asset_root, &path)
        )
    })
}

fn combined_blend_space_member_path(reference: &BlendSpaceReference) -> Option<String> {
    reference
        .authoring_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(normalize_asset_reference_path)
        .or_else(|| {
            normalize_asset_reference_path(&reference.path)
                .strip_suffix(".bspace")
                .map(|stem| format!("{stem}.bspace.ron"))
        })
}

fn combined_blend_space_preview_diagnostics(source: &CombinedBlendSpaceSource) -> Vec<String> {
    let member_count = source.combined_blend_space.blend_spaces.len();
    vec![format!(
        "combined blend-space source selects among {member_count} member blend spaces; activate a member .bspace node for concrete viewport blending"
    )]
}

fn collect_blend_space_ron_files(
    directory: &Path,
    ron_paths: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<String>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        diagnostics.push(format!(
            "failed to read blend-space authoring directory: {}",
            directory.display()
        ));
        return;
    };

    for entry in entries {
        let Ok(entry) = entry else {
            diagnostics.push(format!(
                "failed to read blend-space authoring directory entry under {}",
                directory.display()
            ));
            continue;
        };
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            diagnostics.push(format!(
                "failed to read blend-space authoring file type: {}",
                path.display()
            ));
            continue;
        };
        if file_type.is_dir() {
            collect_blend_space_ron_files(&path, ron_paths, diagnostics);
        } else if file_type.is_file() && is_blend_space_authoring_ron_path(&path) {
            ron_paths.push(path);
        }
    }
}

fn is_blend_space_authoring_ron_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|name| name.ends_with(".bspace.ron") || name.ends_with(".comb.ron"))
}

fn is_combined_blend_space_ron_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".comb.ron"))
}

fn collect_mannequin_ron_files(
    directory: &Path,
    ron_paths: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<String>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        diagnostics.push(format!(
            "failed to read Mannequin authoring directory: {}",
            directory.display()
        ));
        return;
    };

    for entry in entries {
        let Ok(entry) = entry else {
            diagnostics.push(format!(
                "failed to read Mannequin authoring directory entry under {}",
                directory.display()
            ));
            continue;
        };
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            diagnostics.push(format!(
                "failed to read Mannequin authoring file type: {}",
                path.display()
            ));
            continue;
        };
        if file_type.is_dir() {
            collect_mannequin_ron_files(&path, ron_paths, diagnostics);
        } else if file_type.is_file() && is_mannequin_authoring_ron_path(&path) {
            ron_paths.push(path);
        }
    }
}

fn is_mannequin_authoring_ron_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|name| {
            name.ends_with(".adb.ron")
                || name.ends_with(".mannequin.tags.ron")
                || name.ends_with(".mannequin.actions.ron")
                || name.ends_with(".mannequin.controller.ron")
        })
}

fn mannequin_tags_from_sources(
    sources: &[RonMannequinTagDefinitionSource],
) -> Vec<EditorMannequinTagData> {
    sources
        .iter()
        .flat_map(|source| source.entries.iter())
        .flat_map(|entry| match entry {
            RonMannequinTagDefinitionEntry::Tag(tag) => {
                vec![mannequin_tag_data(tag, None)]
            }
            RonMannequinTagDefinitionEntry::Group(group) => group
                .tags
                .iter()
                .map(|tag| mannequin_tag_data(tag, Some(group.name.clone())))
                .collect::<Vec<_>>(),
        })
        .collect()
}

fn mannequin_tag_data(tag: &RonMannequinTagEntry, group: Option<String>) -> EditorMannequinTagData {
    EditorMannequinTagData {
        name: tag.name.clone(),
        group,
        priority: tag.priority,
        sub_tag_definition: tag.sub_tag_definition.clone(),
    }
}

fn mannequin_tag_groups_from_sources(
    sources: &[RonMannequinTagDefinitionSource],
) -> Vec<EditorMannequinTagGroupData> {
    sources
        .iter()
        .flat_map(|source| source.entries.iter())
        .filter_map(|entry| match entry {
            RonMannequinTagDefinitionEntry::Group(group) => Some(EditorMannequinTagGroupData {
                name: group.name.clone(),
                tags: group.tags.iter().map(|tag| tag.name.clone()).collect(),
            }),
            RonMannequinTagDefinitionEntry::Tag(_) => None,
        })
        .collect()
}

fn mannequin_fragment_definitions_from_controllers(
    sources: &[RonMannequinControllerDefinitionSource],
) -> Vec<EditorMannequinFragmentDefinitionData> {
    sources
        .iter()
        .flat_map(|source| source.fragment_definitions.iter())
        .map(|definition| EditorMannequinFragmentDefinitionData {
            name: definition.name.clone(),
            scopes: parse_tag_tokens(Some(&definition.scopes)).required,
            flags: definition.flags.clone(),
            overrides: definition
                .overrides
                .iter()
                .map(|override_data| EditorMannequinFragmentOverrideData {
                    tags: parse_tag_tokens(Some(&override_data.tags)).required,
                    scopes: parse_tag_tokens(Some(&override_data.scopes)).required,
                })
                .collect(),
        })
        .collect()
}

fn mannequin_fragments_from_database(
    source_path: &str,
    source: &RonMannequinAnimationDatabaseSource,
    known_motion_paths: &BTreeSet<&str>,
    fragment_definition_by_name: &BTreeMap<&str, &EditorMannequinFragmentDefinitionData>,
) -> Vec<EditorMannequinFragmentData> {
    source
        .database
        .fragment_groups
        .iter()
        .map(|group| {
            let key = mannequin_fragment_key(source_path, &group.name);
            let definition = fragment_definition_by_name
                .get(group.name.as_str())
                .copied();
            let options = group
                .fragments
                .iter()
                .enumerate()
                .map(|(index, fragment)| {
                    mannequin_fragment_option_data(&key, index, fragment, known_motion_paths)
                })
                .collect::<Vec<_>>();
            EditorMannequinFragmentData {
                key,
                name: group.name.clone(),
                source_path: source_path.to_owned(),
                option_count: options.len(),
                options,
                scopes: definition
                    .map(|definition| definition.scopes.clone())
                    .unwrap_or_default(),
                flags: definition.and_then(|definition| definition.flags.clone()),
            }
        })
        .collect()
}

fn mannequin_fragment_option_data(
    fragment_key: &str,
    index: usize,
    fragment: &RonMannequinFragment,
    known_motion_paths: &BTreeSet<&str>,
) -> EditorMannequinFragmentOptionData {
    let condition = parse_tag_tokens(fragment.tags.as_deref());
    let fragment_tags = parse_tag_tokens(fragment.fragment_tags.as_deref()).required;
    let animation_refs = fragment
        .animation_layers
        .iter()
        .enumerate()
        .flat_map(|(layer_index, layer)| {
            layer
                .animations
                .iter()
                .enumerate()
                .map(move |(animation_index, animation)| (layer_index, animation_index, animation))
        })
        .map(|(layer_index, animation_index, animation)| {
            let motion_glb = normalize_animation_ref(&animation.name);
            let unresolved = motion_glb
                .as_deref()
                .is_none_or(|motion_glb| !known_motion_paths.contains(motion_glb));
            EditorMannequinAnimationRefData {
                original: animation.name.clone(),
                motion_glb,
                unresolved,
                layer_index,
                animation_index,
            }
        })
        .collect();

    EditorMannequinFragmentOptionData {
        key: format!("{fragment_key}:{index}"),
        index,
        required_tags: condition.required,
        excluded_tags: condition.excluded,
        fragment_tags,
        animation_refs,
    }
}

fn mannequin_fragment_blends_from_database(
    source_path: &str,
    source: &RonMannequinAnimationDatabaseSource,
) -> Vec<EditorMannequinFragmentBlendData> {
    source
        .database
        .fragment_blends
        .iter()
        .enumerate()
        .map(|(index, blend)| EditorMannequinFragmentBlendData {
            key: format!("{source_path}#blend:{index}"),
            source_path: source_path.to_owned(),
            from: blend.from.clone(),
            to: blend.to.clone(),
            variant_count: blend.variants.len(),
            fragment_count: blend
                .variants
                .iter()
                .map(|variant| variant.fragments.len())
                .sum(),
        })
        .collect()
}

fn mannequin_fragment_key(source_path: &str, fragment_name: &str) -> String {
    format!("{source_path}#{fragment_name}")
}

fn normalize_animation_ref(value: &str) -> Option<String> {
    let normalized = normalize_asset_reference_path(value);
    if normalized.is_empty() {
        return None;
    }
    normalized
        .to_ascii_lowercase()
        .ends_with(".anim.glb")
        .then(|| normalized.clone())
}

fn normalize_asset_reference_path(value: &str) -> String {
    let normalized = value.trim().replace('\\', "/");
    let normalized = normalized.trim_start_matches('/');
    normalized
        .strip_prefix("assets/")
        .or_else(|| normalized.strip_prefix("Assets/"))
        .unwrap_or(normalized)
        .to_owned()
}

#[derive(Debug, Default)]
struct ParsedTagCondition {
    required: Vec<String>,
    excluded: Vec<String>,
}

fn parse_tag_tokens(value: Option<&str>) -> ParsedTagCondition {
    let mut condition = ParsedTagCondition::default();
    let Some(value) = value else {
        return condition;
    };

    for raw_token in
        value.split(|ch: char| ch == '+' || ch == ',' || ch == '|' || ch.is_whitespace())
    {
        let token = raw_token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(excluded) = token
            .strip_prefix('!')
            .or_else(|| token.strip_prefix('-'))
            .filter(|excluded| !excluded.trim().is_empty())
        {
            condition.excluded.push(excluded.trim().to_owned());
        } else {
            condition.required.push(token.to_owned());
        }
    }
    condition.required.sort();
    condition.required.dedup();
    condition.excluded.sort();
    condition.excluded.dedup();
    condition
}

fn read_motion_data(
    project_asset_root: &Path,
    path: &Path,
    diagnostics: &mut Vec<String>,
) -> EditorAnimationMotionData {
    let asset_path = normalize_relative_path(project_asset_root, path);
    let mut name = motion_display_name(path);
    let set_path = path
        .parent()
        .map(|parent| normalize_relative_path(project_asset_root, parent))
        .unwrap_or_default();
    let mut duration_millis: Option<u32> = None;
    let mut channel_count = 0usize;
    let mut joint_targets = BTreeSet::new();

    match read_gltf_without_validation(path) {
        Ok(gltf) => {
            for animation in gltf.document.animations() {
                if name == motion_display_name(path)
                    && let Some(animation_name) = animation.name()
                    && !animation_name.trim().is_empty()
                {
                    animation_name.clone_into(&mut name);
                }
                for channel in animation.channels() {
                    channel_count += 1;
                    let target = channel.target();
                    let node = target.node();
                    let joint_name = node
                        .name()
                        .filter(|name| !name.trim().is_empty())
                        .map_or_else(|| format!("node_{}", node.index()), str::to_owned);
                    joint_targets.insert(joint_name);

                    if let Some(max_seconds) = animation_input_max_seconds(&channel) {
                        let millis = seconds_to_millis(max_seconds);
                        duration_millis =
                            Some(duration_millis.map_or(millis, |current| current.max(millis)));
                    }
                }
            }
        }
        Err(error) => {
            diagnostics.push(format!("failed to parse motion glTF {asset_path}: {error}"));
        }
    }

    let events = read_motion_events(project_asset_root, path, diagnostics);

    EditorAnimationMotionData {
        asset_path,
        name,
        set_path,
        duration_millis,
        channel_count,
        joint_targets: joint_targets.into_iter().collect(),
        events,
        // The caller attaches the pipeline entry/job status from the asset
        // processor's workspace view.
        pipeline_status: None,
    }
}

fn read_skeleton_joints(
    character_path: &Path,
    diagnostics: &mut Vec<String>,
) -> Vec<EditorAnimationJointData> {
    let gltf = match read_gltf_without_validation(character_path) {
        Ok(gltf) => gltf,
        Err(error) => {
            diagnostics.push(format!(
                "failed to parse mannequin character glTF {}: {error}",
                character_path.display()
            ));
            return Vec::new();
        }
    };

    let joint_indices = gltf
        .document
        .skins()
        .flat_map(|skin| skin.joints().map(|joint| joint.index()))
        .collect::<BTreeSet<_>>();
    if joint_indices.is_empty() {
        return Vec::new();
    }

    let mut parent_by_index = BTreeMap::new();
    let mut depth_by_index = BTreeMap::new();
    for scene in gltf.document.scenes() {
        for node in scene.nodes() {
            collect_node_hierarchy(&node, None, 0, &mut parent_by_index, &mut depth_by_index);
        }
    }

    let mut rows = Vec::new();
    let mut visited = BTreeSet::new();
    for scene in gltf.document.scenes() {
        for node in scene.nodes() {
            push_joint_rows_in_hierarchy(
                &node,
                &joint_indices,
                &parent_by_index,
                &depth_by_index,
                &mut visited,
                &mut rows,
            );
        }
    }

    for node in gltf.document.nodes() {
        if joint_indices.contains(&node.index()) && !visited.contains(&node.index()) {
            rows.push(EditorAnimationJointData {
                name: node
                    .name()
                    .filter(|name| !name.trim().is_empty())
                    .map_or_else(|| format!("joint_{}", node.index()), str::to_owned),
                depth: depth_by_index
                    .get(&node.index())
                    .copied()
                    .unwrap_or_default(),
                index: joint_index_u32(node.index()),
                parent: parent_by_index
                    .get(&node.index())
                    .copied()
                    .flatten()
                    .map(joint_index_u32),
            });
        }
    }

    rows
}

/// A glTF node index as the `u32` the joint rows carry. glTF stores indices as
/// 32-bit values, so the saturating fallback is unreachable and only keeps the
/// narrowing checked.
fn joint_index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

fn collect_node_hierarchy(
    node: &gltf::Node<'_>,
    parent: Option<usize>,
    depth: u32,
    parent_by_index: &mut BTreeMap<usize, Option<usize>>,
    depth_by_index: &mut BTreeMap<usize, u32>,
) {
    parent_by_index.entry(node.index()).or_insert(parent);
    depth_by_index.entry(node.index()).or_insert(depth);
    for child in node.children() {
        collect_node_hierarchy(
            &child,
            Some(node.index()),
            depth + 1,
            parent_by_index,
            depth_by_index,
        );
    }
}

fn push_joint_rows_in_hierarchy(
    node: &gltf::Node<'_>,
    joint_indices: &BTreeSet<usize>,
    parent_by_index: &BTreeMap<usize, Option<usize>>,
    depth_by_index: &BTreeMap<usize, u32>,
    visited: &mut BTreeSet<usize>,
    rows: &mut Vec<EditorAnimationJointData>,
) {
    if joint_indices.contains(&node.index()) && visited.insert(node.index()) {
        rows.push(EditorAnimationJointData {
            name: node
                .name()
                .filter(|name| !name.trim().is_empty())
                .map_or_else(|| format!("joint_{}", node.index()), str::to_owned),
            depth: depth_by_index
                .get(&node.index())
                .copied()
                .unwrap_or_default(),
            index: joint_index_u32(node.index()),
            parent: parent_by_index
                .get(&node.index())
                .copied()
                .flatten()
                .map(joint_index_u32),
        });
    }

    for child in node.children() {
        push_joint_rows_in_hierarchy(
            &child,
            joint_indices,
            parent_by_index,
            depth_by_index,
            visited,
            rows,
        );
    }
}

fn read_motion_events(
    project_asset_root: &Path,
    motion_path: &Path,
    diagnostics: &mut Vec<String>,
) -> Vec<EditorAnimationEventData> {
    for candidate in motion_event_candidates(motion_path) {
        if !candidate.exists() {
            continue;
        }

        let rel = normalize_relative_path(project_asset_root, &candidate);
        let bytes = match fs::read(&candidate) {
            Ok(bytes) => bytes,
            Err(error) => {
                diagnostics.push(format!("failed to read animation events {rel}: {error}"));
                return Vec::new();
            }
        };

        return match ron::de::from_bytes::<RonAnimationEventListSource>(&bytes) {
            Ok(source) => source
                .animations
                .iter()
                .flat_map(|animation| {
                    animation
                        .events
                        .iter()
                        .map(|event| EditorAnimationEventData {
                            animation: animation.animation.clone(),
                            name: event.name.clone(),
                            time_millis: seconds_to_millis(f64::from(event.time)),
                            end_time_millis: seconds_to_millis(f64::from(event.end_time)),
                            parameter: event.parameter.clone(),
                        })
                })
                .collect(),
            Err(error) => {
                diagnostics.push(format!("failed to parse animation events {rel}: {error}"));
                Vec::new()
            }
        };
    }

    Vec::new()
}

fn motion_event_candidates(motion_path: &Path) -> Vec<PathBuf> {
    let parent = motion_path.parent().unwrap_or_else(|| Path::new(""));
    let file_name = motion_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let motion_stem = file_name
        .strip_suffix(".anim.glb")
        .or_else(|| file_name.strip_suffix(".ANIM.GLB"))
        .unwrap_or_else(|| {
            motion_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(file_name)
        });

    vec![
        parent.join(format!("{motion_stem}.animevents.ron")),
        parent.join(format!("{motion_stem}.anim.animevents.ron")),
    ]
}

/// The animation's last input keyframe time, in seconds. glTF reports it as
/// JSON, so it stays `f64` all the way to the millisecond conversion.
fn animation_input_max_seconds(channel: &gltf::animation::Channel<'_>) -> Option<f64> {
    let max = channel.sampler().input().max()?;
    match max {
        gltf::json::Value::Array(values) => values.first().and_then(gltf::json::Value::as_f64),
        gltf::json::Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

fn read_gltf_without_validation(path: &Path) -> Result<gltf::Gltf, gltf::Error> {
    let file = fs::File::open(path)?;
    gltf::Gltf::from_reader_without_validation(BufReader::new(file))
}

fn motion_display_name(path: &Path) -> String {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    file_name
        .strip_suffix(".anim.glb")
        .or_else(|| file_name.strip_suffix(".ANIM.GLB"))
        .filter(|stem| !stem.trim().is_empty())
        .unwrap_or(file_name)
        .to_owned()
}

fn normalize_relative_path(root: &Path, path: &Path) -> String {
    let path = path.strip_prefix(root).unwrap_or(path);
    path.to_string_lossy().replace('\\', "/")
}

/// Whole milliseconds for a non-negative, finite animation time.
// The clamp bounds the value into `0..=u32::MAX` before the narrowing, so
// neither the truncation nor the sign case this conversion warns about is
// reachable.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn seconds_to_millis(seconds: f64) -> u32 {
    if seconds.is_finite() && seconds > 0.0 {
        (seconds * 1000.0).clamp(0.0, f64::from(u32::MAX)).round() as u32
    } else {
        0
    }
}

fn blend_space_display_name(asset_path: &str) -> String {
    let file_name = Path::new(asset_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(asset_path);
    file_name
        .strip_suffix(".bspace.ron")
        .or_else(|| file_name.strip_suffix(".BSPACE.RON"))
        .or_else(|| file_name.strip_suffix(".comb.ron"))
        .or_else(|| file_name.strip_suffix(".COMB.RON"))
        .filter(|stem| !stem.trim().is_empty())
        .unwrap_or(file_name)
        .to_owned()
}

fn blend_space_document_from_ron(
    project_asset_root: &Path,
    source: &BlendSpaceSource,
    diagnostics: &mut Vec<String>,
) -> EditorBlendSpaceData {
    let dimensions = source
        .blend_space
        .dimensions
        .iter()
        .map(|dimension| EditorBlendSpaceDimensionData {
            name: dimension.name.clone(),
            parameter_id: dimension.parameter_id.map(u32::from),
            min: dimension.min,
            max: dimension.max,
            cells: dimension.cells.max(1) as usize,
            locked: dimension.locked,
        })
        .collect::<Vec<_>>();

    let examples = source
        .blend_space
        .examples
        .iter()
        .filter_map(|example| {
            let Some(motion_path) = example
                .animation
                .motion_path
                .as_deref()
                .filter(|path| !path.trim().is_empty())
            else {
                diagnostics.push(format!(
                    "blend-space example {} has no resolved .anim.glb motion path",
                    example.animation.name
                ));
                return None;
            };
            let motion_asset_path = normalize_asset_path(motion_path);
            let motion_full_path = project_asset_root.join(Path::new(&motion_asset_path));
            if !motion_full_path.exists() {
                diagnostics.push(format!(
                    "blend-space example motion not found: {motion_asset_path}"
                ));
            }
            let coordinates = example
                .coordinates
                .iter()
                .filter_map(|coordinate| {
                    coordinate
                        .value
                        .map(|value| EditorBlendSpaceCoordinateData {
                            dimension: coordinate.dimension.clone(),
                            value,
                        })
                })
                .collect();
            Some(EditorBlendSpaceExampleData {
                animation_name: non_empty_string_or(&example.animation.name, &motion_asset_path),
                motion_path: motion_asset_path,
                coordinates,
                playback_scale: Some(example.playback_scale),
            })
        })
        .collect();

    let virtual_examples = source
        .blend_space
        .virtual_examples
        .iter()
        .map(|virtual_example| {
            let mut indices = Vec::new();
            let mut weights = Vec::new();
            for (index, weight) in virtual_example.indices.iter().zip(&virtual_example.weights) {
                // `try_from` rejects exactly the negative sentinels this filter
                // used to test for, so the guard and the conversion are one.
                let Ok(index) = usize::try_from(*index) else {
                    continue;
                };
                if *weight > 0.0 {
                    indices.push(index);
                    weights.push(*weight);
                }
            }
            EditorBlendSpaceVirtualExampleData { indices, weights }
        })
        .collect();

    EditorBlendSpaceData {
        source_path: normalize_asset_path(&source.source_path),
        dimensions,
        examples,
        virtual_examples,
    }
}

fn normalize_asset_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches('/')
        .trim_start_matches("./")
        .to_owned()
}

fn non_empty_string_or(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonAnimationEventListSource {
    #[serde(default)]
    animations: Vec<RonAnimationEventsSource>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonAnimationEventsSource {
    #[serde(default)]
    animation: String,
    #[serde(default)]
    events: Vec<RonAnimationEventSource>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonAnimationEventSource {
    #[serde(default)]
    name: String,
    #[serde(default)]
    time: f32,
    #[serde(default)]
    end_time: f32,
    #[serde(default)]
    parameter: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinAnimationDatabaseSource {
    #[serde(default)]
    database: RonMannequinAnimationDatabase,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinAnimationDatabase {
    #[serde(default)]
    fragment_groups: Vec<RonMannequinFragmentGroup>,
    #[serde(default)]
    fragment_blends: Vec<RonMannequinFragmentBlend>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinFragmentGroup {
    #[serde(default)]
    name: String,
    #[serde(default)]
    fragments: Vec<RonMannequinFragment>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinFragment {
    #[serde(default)]
    tags: Option<String>,
    #[serde(default)]
    fragment_tags: Option<String>,
    #[serde(default)]
    animation_layers: Vec<RonMannequinAnimationLayer>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinAnimationLayer {
    #[serde(default)]
    animations: Vec<RonMannequinAnimationEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinAnimationEntry {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinFragmentBlend {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    variants: Vec<RonMannequinFragmentBlendVariant>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinFragmentBlendVariant {
    #[serde(default)]
    fragments: Vec<RonMannequinFragment>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinTagDefinitionSource {
    #[serde(default)]
    entries: Vec<RonMannequinTagDefinitionEntry>,
}

#[derive(Debug, Clone, Deserialize)]
enum RonMannequinTagDefinitionEntry {
    Tag(RonMannequinTagEntry),
    Group(RonMannequinTagGroup),
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinTagGroup {
    #[serde(default)]
    name: String,
    #[serde(default)]
    tags: Vec<RonMannequinTagEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinTagEntry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    priority: Option<i32>,
    #[serde(default)]
    sub_tag_definition: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinControllerDefinitionSource {
    #[serde(default)]
    fragment_definitions: Vec<RonMannequinFragmentDefinition>,
    #[serde(default)]
    scope_contexts: Vec<RonMannequinScopeContext>,
    #[serde(default)]
    scopes: Vec<RonMannequinScopeDefinition>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinFragmentDefinition {
    #[serde(default)]
    name: String,
    #[serde(default)]
    scopes: String,
    #[serde(default)]
    flags: Option<String>,
    #[serde(default)]
    overrides: Vec<RonMannequinFragmentOverride>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinFragmentOverride {
    #[serde(default)]
    tags: String,
    #[serde(default)]
    scopes: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinScopeContext {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RonMannequinScopeDefinition {
    #[serde(default)]
    name: String,
    #[serde(default)]
    layer: i32,
    #[serde(default)]
    num_layers: i32,
    #[serde(default)]
    context: String,
    #[serde(default)]
    tags: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_editor_inspector::ReflectedPrefabSelection;
    use az_editor_ui::panels::{AssetBrowserJobData, AssetBrowserJobStatus};
    use az_proto_project::vnext::{
        ApplicabilityDescriptor, EditorAttributes, PrefabComponentSnapshot, PrefabEntitySnapshot,
        PrefabHierarchyEdge, PrefabSourceSnapshot, ReflectedFieldDescriptor, ReflectedPathSegment,
        ReflectedTypeDescriptor,
    };
    use std::fs;

    const TEST_SCAN_FOLDER_ID: i64 = 7;

    /// An attached workspace status whose only root is the
    /// project asset root, with one animation-schema entry per motion path.
    fn asset_status_for_motions(
        asset_root: &Path,
        motion_paths: &[&str],
    ) -> EditorAssetBrowserStatus {
        let entries = motion_paths
            .iter()
            .enumerate()
            .map(|(index, source_path)| {
                let id = i64::try_from(index).expect("fixture entry index fits i64") + 1;
                animation_entry(id, source_path, AssetBrowserEntryStatus::Clean)
            })
            .collect();
        EditorAssetBrowserStatus::new(
            "test-session",
            vec![project_source_root(asset_root)],
            entries,
            None,
        )
    }

    fn project_source_root(asset_root: &Path) -> az_editor_ui::panels::WorkspaceRootData {
        az_editor_ui::panels::WorkspaceRootData {
            workspace_root_id: 1,
            root_id: TEST_SCAN_FOLDER_ID,
            declared_root_id: "project-assets".to_owned(),
            owner_id: "project".to_owned(),
            source_root: asset_root.to_string_lossy().into_owned(),
            display_name: "Project Assets".to_owned(),
            portable_key: "project:test:assets".to_owned(),
            output_prefix: String::new(),
        }
    }

    fn animation_entry(
        id: i64,
        source_path: &str,
        status: AssetBrowserEntryStatus,
    ) -> AssetBrowserEntryData {
        AssetBrowserEntryData {
            entry_id: id,
            workspace_id: 1,
            asset_guid: format!("00000000-0000-0000-0000-{id:012x}"),
            root_id: TEST_SCAN_FOLDER_ID,
            source_path: source_path.to_owned(),
            schema_type: Some(ANIMATION_SOURCE_SCHEMA_TYPE.to_owned()),
            content_hash: format!("hash-{id}"),
            status,
            diagnostics_count: 0,
            latest_job: None,
        }
    }

    #[test]
    fn animation_preview_catalog_derives_motions_from_asset_entries() {
        let temp = tempfile::tempdir().unwrap();
        let asset_root = temp.path().join("assets");
        write_fixture_gltf(
            &asset_root.join("animations/locomotion/walk.anim.glb"),
            "Walk",
            1.25,
            "hips",
        );
        write_fixture_gltf(
            &asset_root.join("animations/combat/attack.anim.glb"),
            "Attack",
            0.75,
            "hand_r",
        );
        write_character_gltf(&asset_root.join("characters/hero.glb"));
        let status = asset_status_for_motions(
            &asset_root,
            &[
                "animations/locomotion/walk.anim.glb",
                "animations/combat/attack.anim.glb",
            ],
        );

        let catalog = build_animation_preview_catalog_from_status(
            &status,
            &asset_root,
            Some("characters/hero.glb"),
        );

        assert_eq!(catalog.motions.len(), 2);
        assert_eq!(
            catalog.motions[0].asset_path,
            "animations/combat/attack.anim.glb"
        );
        assert_eq!(catalog.motions[0].name, "Attack");
        assert_eq!(catalog.motions[0].set_path, "animations/combat");
        assert_eq!(catalog.motions[0].duration_millis, Some(750));
        assert_eq!(catalog.motions[0].channel_count, 1);
        assert_eq!(catalog.motions[0].joint_targets, vec!["hand_r"]);
        assert_eq!(catalog.motions[0].pipeline_status.as_deref(), Some("clean"));
        assert_eq!(
            catalog.motions[1].asset_path,
            "animations/locomotion/walk.anim.glb"
        );
        assert_eq!(catalog.motions[1].duration_millis, Some(1250));
        assert_eq!(catalog.skeleton_joints.len(), 3);
        assert_eq!(catalog.skeleton_joints[0].name, "root");
        assert_eq!(catalog.skeleton_joints[1].name, "hips");
        assert_eq!(catalog.skeleton_joints[2].name, "hand_r");
    }

    #[test]
    fn animation_preview_catalog_reads_adjacent_animevents() {
        let temp = tempfile::tempdir().unwrap();
        let asset_root = temp.path().join("assets");
        write_fixture_gltf(
            &asset_root.join("animations/locomotion/walk.anim.glb"),
            "Walk",
            1.25,
            "hips",
        );
        fs::write(
            asset_root.join("animations/locomotion/walk.animevents.ron"),
            r#"(
    source_path: "animations/locomotion/walk.animevents",
    animations: [
        (
            animation: "Walk",
            events: [
                (
                    name: "footstep",
                    time: 0.25,
                    end_time: 0.25,
                    parameter: "left",
                ),
            ],
        ),
    ],
)"#,
        )
        .unwrap();
        let status =
            asset_status_for_motions(&asset_root, &["animations/locomotion/walk.anim.glb"]);

        let catalog = build_animation_preview_catalog_from_status(&status, &asset_root, None);

        assert_eq!(catalog.motions.len(), 1);
        assert_eq!(catalog.motions[0].events.len(), 1);
        assert_eq!(catalog.motions[0].events[0].name, "footstep");
        assert_eq!(catalog.motions[0].events[0].animation, "Walk");
        assert_eq!(catalog.motions[0].events[0].time_millis, 250);
        assert_eq!(catalog.motions[0].events[0].parameter, "left");
    }

    #[test]
    fn animation_motion_sources_filter_to_animation_schema_and_skip_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let asset_root = temp.path().join("assets");
        let mut status = asset_status_for_motions(
            &asset_root,
            &[
                "animations/locomotion/walk.anim.glb",
                "animations/locomotion/run.anim.glb",
            ],
        );
        // Deleted animation entries and non-animation schemas never surface.
        status.entries[1].status = AssetBrowserEntryStatus::Deleted;
        status.entries.push(AssetBrowserEntryData {
            schema_type: Some("azoth.texture.TextureSource".to_owned()),
            ..animation_entry(3, "textures/grass.png", AssetBrowserEntryStatus::Clean)
        });
        status.entries.push(AssetBrowserEntryData {
            schema_type: None,
            ..animation_entry(4, "notes/readme.md", AssetBrowserEntryStatus::Clean)
        });
        // Failed jobs surface in the pipeline status label.
        status.entries[0].latest_job = Some(AssetBrowserJobData {
            job_id: 10,
            attempt_id: Some(11),
            job_key: "azoth.animation".to_owned(),
            platform: "pc".to_owned(),
            ordinal: Some(1),
            status: AssetBrowserJobStatus::Failed,
            error_count: 1,
            warning_count: 0,
        });

        let mut diagnostics = Vec::new();
        let sources =
            animation_motion_sources_from_asset_entries(&status, &asset_root, &mut diagnostics);

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].asset_path, "animations/locomotion/walk.anim.glb");
        assert_eq!(
            sources[0].absolute_path,
            asset_root.join("animations/locomotion/walk.anim.glb")
        );
        assert_eq!(sources[0].pipeline_status, "clean · job failed");
    }

    #[test]
    fn animation_motion_sources_outside_the_asset_root_are_skipped_with_diagnostics() {
        let temp = tempfile::tempdir().unwrap();
        let asset_root = temp.path().join("assets");
        let gem_root = temp.path().join("gems/motion/assets");
        let mut status =
            asset_status_for_motions(&asset_root, &["animations/locomotion/walk.anim.glb"]);
        status.roots.push(az_editor_ui::panels::WorkspaceRootData {
            workspace_root_id: 2,
            root_id: TEST_SCAN_FOLDER_ID + 1,
            declared_root_id: "motion-assets".to_owned(),
            owner_id: "gem".to_owned(),
            source_root: gem_root.to_string_lossy().into_owned(),
            display_name: "Gem Assets".to_owned(),
            portable_key: "gem:motion:assets".to_owned(),
            output_prefix: "gems/motion".to_owned(),
        });
        status.entries.push(AssetBrowserEntryData {
            root_id: TEST_SCAN_FOLDER_ID + 1,
            ..animation_entry(
                9,
                "animations/gem_walk.anim.glb",
                AssetBrowserEntryStatus::Clean,
            )
        });

        let mut diagnostics = Vec::new();
        let sources =
            animation_motion_sources_from_asset_entries(&status, &asset_root, &mut diagnostics);

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].asset_path, "animations/locomotion/walk.anim.glb");
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].contains("outside the project asset root"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn selecting_animation_motion_action_updates_preview_state() {
        let mut preview = EditorMannequinPreview::empty();

        assert!(apply_animation_preview_action(
            &mut preview,
            MannequinPreviewAction::SelectMotion("animations/locomotion/walk.anim.glb".to_owned()),
        ));

        assert_eq!(
            preview.motion_glb.as_deref(),
            Some("animations/locomotion/walk.anim.glb")
        );
        assert!(preview.playing);
        assert_eq!(preview.position_millis, 0);
    }

    #[test]
    fn animation_transport_actions_update_preview_playback() {
        let mut preview = EditorMannequinPreview::default_for_project_asset_root("assets");

        assert!(apply_animation_preview_action(
            &mut preview,
            MannequinPreviewAction::SetPlaying(false),
        ));
        assert!(!preview.playing);

        assert!(apply_animation_preview_action(
            &mut preview,
            MannequinPreviewAction::SetPlaying(true),
        ));
        assert!(preview.playing);

        assert!(apply_animation_preview_action(
            &mut preview,
            MannequinPreviewAction::SetLooping(false),
        ));
        assert!(!preview.looping);

        assert!(apply_animation_preview_action(
            &mut preview,
            MannequinPreviewAction::Scrub(640),
        ));
        assert_eq!(preview.position_millis, 640);

        assert!(apply_animation_preview_action(
            &mut preview,
            MannequinPreviewAction::Stop,
        ));
        assert!(!preview.playing);
        assert_eq!(preview.position_millis, 0);
    }

    #[test]
    fn blend_space_catalog_loads_bspace_ron_examples_and_vgrid() {
        let temp = tempfile::tempdir().unwrap();
        let asset_root = temp.path().join("assets");
        write_fixture_gltf(
            &asset_root.join("animations/locomotion/idle.anim.glb"),
            "Idle",
            1.0,
            "hips",
        );
        write_fixture_gltf(
            &asset_root.join("animations/locomotion/run.anim.glb"),
            "Run",
            1.0,
            "hips",
        );
        write_blend_space_fixture(&asset_root);

        let catalog = build_blend_space_preview_catalog(&asset_root);

        assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
        assert_eq!(catalog.blend_spaces.len(), 1);
        assert_eq!(
            catalog.blend_spaces[0].asset_path,
            "animations/locomotion/blendspace/speed.bspace.ron"
        );
        assert_eq!(catalog.blend_spaces[0].dimension_count, 1);
        assert_eq!(catalog.blend_spaces[0].example_count, 2);
        assert!(catalog.blend_spaces[0].has_vgrid);
    }

    #[test]
    fn blend_space_selection_loads_ron_and_param_action_updates_weights() {
        let temp = tempfile::tempdir().unwrap();
        let asset_root = temp.path().join("assets");
        write_fixture_gltf(
            &asset_root.join("animations/locomotion/idle.anim.glb"),
            "Idle",
            1.0,
            "hips",
        );
        write_fixture_gltf(
            &asset_root.join("animations/locomotion/run.anim.glb"),
            "Run",
            1.0,
            "hips",
        );
        write_blend_space_fixture(&asset_root);

        let mut preview = EditorBlendSpacePreview::with_project_asset_root(asset_root.clone());
        assert!(apply_blend_space_preview_action(
            &mut preview,
            &asset_root,
            BlendSpacePreviewAction::SelectBlendSpace(
                "animations/locomotion/blendspace/speed.bspace.ron".to_owned(),
            ),
        ));

        assert_eq!(
            preview.bspace_ron_path.as_deref(),
            Some("animations/locomotion/blendspace/speed.bspace.ron")
        );
        assert_eq!(preview.param_values, vec![0.5]);
        assert_eq!(preview.weights.len(), 2);
        assert!((preview.weights[0].weight - 0.5).abs() < 0.0001);
        assert!((preview.weights[1].weight - 0.5).abs() < 0.0001);

        assert!(apply_blend_space_preview_action(
            &mut preview,
            &asset_root,
            BlendSpacePreviewAction::SetParameter {
                dimension: "Speed".to_owned(),
                value: 1.0,
            },
        ));

        assert_eq!(preview.param_values, vec![1.0]);
        assert!(preview.weights[0].weight < 0.0001);
        assert!((preview.weights[1].weight - 1.0).abs() < 0.0001);
    }

    #[test]
    fn mannequin_authoring_catalog_loads_decoded_adb_tag_and_controller_ron() {
        let temp = tempfile::tempdir().unwrap();
        let asset_root = temp.path().join("assets");
        write_fixture_gltf(
            &asset_root.join("animations/locomotion/idle.anim.glb"),
            "Idle",
            1.0,
            "hips",
        );
        write_fixture_gltf(
            &asset_root.join("animations/locomotion/idle_alert.anim.glb"),
            "IdleAlert",
            1.0,
            "hips",
        );
        write_mannequin_authoring_fixture(&asset_root);
        let status = asset_status_for_motions(
            &asset_root,
            &[
                "animations/locomotion/idle.anim.glb",
                "animations/locomotion/idle_alert.anim.glb",
            ],
        );
        let motion_catalog =
            build_animation_preview_catalog_from_status(&status, &asset_root, None);

        let catalog = build_mannequin_authoring_catalog(&asset_root, &motion_catalog);

        assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
        assert_eq!(catalog.fragments.len(), 1);
        assert_eq!(catalog.fragments[0].name, "Idle");
        assert_eq!(
            catalog.fragments[0].source_path,
            "animations/mannequin/hero/hero.adb.ron"
        );
        assert_eq!(catalog.fragments[0].option_count, 3);
        assert_eq!(catalog.fragments[0].scopes, vec!["FullBody"]);
        assert_eq!(catalog.fragments[0].options[1].required_tags, vec!["Alert"]);
        assert_eq!(
            catalog.fragments[0].options[1].animation_refs[0]
                .motion_glb
                .as_deref(),
            Some("animations/locomotion/idle_alert.anim.glb")
        );
        assert!(!catalog.fragments[0].options[1].animation_refs[0].unresolved);
        assert!(catalog.fragments[0].options[2].animation_refs[0].unresolved);
        assert_eq!(
            catalog
                .tags
                .iter()
                .map(|tag| (tag.name.as_str(), tag.group.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("Alert", None),
                ("Crouch", Some("Stance")),
                ("Stand", Some("Stance"))
            ]
        );
        assert_eq!(catalog.tag_groups[0].name, "Stance");
        assert_eq!(catalog.scopes[0].name, "FullBody");
        assert_eq!(catalog.scope_contexts[0].name, "Character");
        assert_eq!(catalog.fragment_blends[0].from.as_deref(), Some("Idle"));
        assert_eq!(catalog.fragment_blends[0].to.as_deref(), Some("Move"));
    }

    #[test]
    fn selecting_mannequin_fragment_and_tags_updates_preview_motion() {
        let (mut catalog, motion_catalog, mut preview) = fixture_mannequin_authoring_state();
        let fragment_key = catalog.fragments[0].key.clone();

        assert!(apply_mannequin_authoring_action(
            &mut catalog,
            &motion_catalog,
            MannequinAuthoringAction::SelectFragment(fragment_key),
        ));
        assert!(apply_resolved_mannequin_preview(
            &mut preview,
            catalog.resolved.as_ref(),
        ));
        assert_eq!(
            preview.motion_glb.as_deref(),
            Some("animations/locomotion/idle.anim.glb")
        );

        assert!(apply_mannequin_authoring_action(
            &mut catalog,
            &motion_catalog,
            MannequinAuthoringAction::SetTag {
                tag: "Alert".to_owned(),
                enabled: true,
            },
        ));
        assert!(apply_resolved_mannequin_preview(
            &mut preview,
            catalog.resolved.as_ref(),
        ));

        assert_eq!(
            preview.motion_glb.as_deref(),
            Some("animations/locomotion/idle_alert.anim.glb")
        );
        assert!(preview.playing);
    }

    #[test]
    fn mannequin_tag_toggle_changes_resolved_option() {
        let (mut catalog, motion_catalog, mut preview) = fixture_mannequin_authoring_state();
        reconcile_mannequin_authoring_catalog(&mut catalog, &motion_catalog, &mut preview);
        assert_eq!(
            catalog
                .resolved
                .as_ref()
                .and_then(|resolved| resolved.motion_glb.as_deref()),
            Some("animations/locomotion/idle.anim.glb")
        );

        assert!(apply_mannequin_authoring_action(
            &mut catalog,
            &motion_catalog,
            MannequinAuthoringAction::SetTag {
                tag: "Alert".to_owned(),
                enabled: true,
            },
        ));
        apply_resolved_mannequin_preview(&mut preview, catalog.resolved.as_ref());
        assert_eq!(
            catalog
                .resolved
                .as_ref()
                .and_then(|resolved| resolved.motion_glb.as_deref()),
            Some("animations/locomotion/idle_alert.anim.glb")
        );

        assert!(apply_mannequin_authoring_action(
            &mut catalog,
            &motion_catalog,
            MannequinAuthoringAction::SetTag {
                tag: "Alert".to_owned(),
                enabled: false,
            },
        ));
        apply_resolved_mannequin_preview(&mut preview, catalog.resolved.as_ref());

        assert_eq!(
            preview.motion_glb.as_deref(),
            Some("animations/locomotion/idle.anim.glb")
        );
    }

    #[test]
    fn unresolved_mannequin_option_clears_preview_motion() {
        let (mut catalog, motion_catalog, mut preview) = fixture_mannequin_authoring_state();
        reconcile_mannequin_authoring_catalog(&mut catalog, &motion_catalog, &mut preview);

        assert!(apply_mannequin_authoring_action(
            &mut catalog,
            &motion_catalog,
            MannequinAuthoringAction::SetTag {
                tag: "Crouch".to_owned(),
                enabled: true,
            },
        ));
        assert!(apply_resolved_mannequin_preview(
            &mut preview,
            catalog.resolved.as_ref(),
        ));

        let resolved = catalog.resolved.as_ref().unwrap();
        assert!(resolved.unresolved);
        assert_eq!(
            resolved.animation_ref.as_deref(),
            Some("unresolved_idle_crouch.caf")
        );
        assert!(preview.motion_glb.is_none());
        assert!(!preview.playing);
    }

    /// A one-dimension blend space with Idle pinned at Speed 0.0 and Run at
    /// Speed 1.0, so a coordinate edit provably moves the blended weights.
    fn speed_blend_source() -> TestBlendSource {
        TestBlendSource {
            source_path: "animations/test/speed.bspace.ron".to_owned(),
            blend_space: TestBlendDocument {
                dimensions: vec![TestBlendDimension {
                    name: "Speed".to_owned(),
                    parameter_id: Some(0),
                    min: 0.0,
                    max: 1.0,
                    cells: 2,
                    locked: false,
                }],
                examples: vec![
                    TestBlendExample {
                        animation: TestBlendAnimation {
                            name: "Idle".to_owned(),
                            motion_path: Some("animations/idle.anim.glb".to_owned()),
                        },
                        coordinates: vec![TestBlendCoordinate {
                            dimension: "Speed".to_owned(),
                            value: Some(0.0),
                        }],
                        playback_scale: 1.0,
                    },
                    TestBlendExample {
                        animation: TestBlendAnimation {
                            name: "Run".to_owned(),
                            motion_path: Some("animations/run.anim.glb".to_owned()),
                        },
                        coordinates: vec![TestBlendCoordinate {
                            dimension: "Speed".to_owned(),
                            value: Some(1.0),
                        }],
                        playback_scale: 1.0,
                    },
                ],
                virtual_examples: Vec::new(),
            },
        }
    }

    #[test]
    fn blend_space_example_coordinate_edit_command_updates_authored_preview_weights() {
        let catalog = animation_edit_test_catalog();
        let mut source = speed_blend_source();
        let inspection = reflected_test_inspection(
            TEST_BLEND_SOURCE_TYPE,
            "animations/test/speed.bspace.ron",
            &source,
            &catalog,
        );
        let command =
            plan_blend_space_example_coordinate_edit(&inspection, &catalog, 1, "Speed", 0.25)
                .expect("coordinate edit should plan reflected command");

        let PrefabEditCommand::SetValue { target, value } = command else {
            panic!("expected reflected SetValue command");
        };
        assert_eq!(target.path.component_type_path, TEST_BLEND_SOURCE_TYPE);
        assert_eq!(
            target.path.segments,
            vec![
                ReflectedPathSegment::Field("blend_space".to_owned()),
                ReflectedPathSegment::Field("examples".to_owned()),
                ReflectedPathSegment::ListIndex(1),
                ReflectedPathSegment::Field("coordinates".to_owned()),
                ReflectedPathSegment::ListIndex(0),
                ReflectedPathSegment::Field("value".to_owned()),
            ]
        );
        assert_eq!(String::from_utf8(value.payload).unwrap(), "Some(0.25)");

        let document_before = reflected_blend_space_preview_data(&inspection)
            .unwrap()
            .unwrap();
        let mut preview = EditorBlendSpacePreview::empty();
        preview.set_document(
            "animations/test/speed.bspace.ron",
            document_before,
            Vec::new(),
        );
        preview.set_param_values(&[0.5]);
        let weights_before = preview
            .weights
            .iter()
            .map(|weight| weight.weight)
            .collect::<Vec<_>>();

        source.blend_space.examples[1].coordinates[0].value = Some(0.25);
        let edited = reflected_test_inspection(
            TEST_BLEND_SOURCE_TYPE,
            "animations/test/speed.bspace.ron",
            &source,
            &catalog,
        );
        let document_after = reflected_blend_space_preview_data(&edited)
            .unwrap()
            .unwrap();
        preview.set_document(
            "animations/test/speed.bspace.ron",
            document_after,
            Vec::new(),
        );
        let weights_after = preview
            .weights
            .iter()
            .map(|weight| weight.weight)
            .collect::<Vec<_>>();

        assert_ne!(weights_before, weights_after);
        assert!(weights_after[1] > weights_before[1]);
        assert_eq!(
            preview.document.as_ref().unwrap().examples[1].coordinate_for_dimension("Speed"),
            Some(0.25)
        );
    }

    #[test]
    fn mannequin_option_animation_edit_command_resolves_new_preview_motion() {
        let catalog = animation_edit_test_catalog();
        let mut source = TestMannequinSource {
            source_path: "animations/test/hero.adb.ron".to_owned(),
            database: TestMannequinDatabase {
                fragment_groups: vec![TestMannequinFragmentGroup {
                    name: "Idle".to_owned(),
                    fragments: vec![TestMannequinFragment {
                        tags: None,
                        fragment_tags: None,
                        animation_layers: vec![TestMannequinAnimationLayer {
                            animations: vec![TestMannequinAnimationEntry {
                                name: "animations/idle.anim.glb".to_owned(),
                            }],
                        }],
                    }],
                }],
                fragment_blends: Vec::new(),
            },
        };
        let inspection = reflected_test_inspection(
            TEST_MANNEQUIN_SOURCE_TYPE,
            "animations/test/hero.adb.ron",
            &source,
            &catalog,
        );
        let command = plan_mannequin_option_animation_edit(
            &inspection,
            &catalog,
            "animations/test/hero.adb.ron#Idle",
            0,
            0,
            0,
            "animations/run.anim.glb",
        )
        .expect("Mannequin animation edit should plan reflected command");

        let PrefabEditCommand::SetValue { target, value } = command else {
            panic!("expected reflected SetValue command");
        };
        assert_eq!(target.path.component_type_path, TEST_MANNEQUIN_SOURCE_TYPE);
        assert_eq!(
            target.path.segments,
            vec![
                ReflectedPathSegment::Field("database".to_owned()),
                ReflectedPathSegment::Field("fragment_groups".to_owned()),
                ReflectedPathSegment::ListIndex(0),
                ReflectedPathSegment::Field("fragments".to_owned()),
                ReflectedPathSegment::ListIndex(0),
                ReflectedPathSegment::Field("animation_layers".to_owned()),
                ReflectedPathSegment::ListIndex(0),
                ReflectedPathSegment::Field("animations".to_owned()),
                ReflectedPathSegment::ListIndex(0),
                ReflectedPathSegment::Field("name".to_owned()),
            ]
        );
        assert_eq!(
            String::from_utf8(value.payload).unwrap(),
            "\"animations/run.anim.glb\""
        );

        let motion_catalog = test_motion_catalog();
        let mut authoring = authoring_catalog_from_test_inspection(&inspection, &motion_catalog);
        authoring.selected_fragment_key = Some("animations/test/hero.adb.ron#Idle".to_owned());
        authoring.resolved = resolve_mannequin_preview_motion(&authoring, &motion_catalog);
        let mut preview = EditorMannequinPreview::empty();
        assert!(apply_resolved_mannequin_preview(
            &mut preview,
            authoring.resolved.as_ref()
        ));
        assert_eq!(
            preview.motion_glb.as_deref(),
            Some("animations/idle.anim.glb")
        );

        source.database.fragment_groups[0].fragments[0].animation_layers[0].animations[0].name =
            "animations/run.anim.glb".to_owned();
        let edited = reflected_test_inspection(
            TEST_MANNEQUIN_SOURCE_TYPE,
            "animations/test/hero.adb.ron",
            &source,
            &catalog,
        );
        let mut edited_authoring = authoring_catalog_from_test_inspection(&edited, &motion_catalog);
        edited_authoring.selected_fragment_key =
            Some("animations/test/hero.adb.ron#Idle".to_owned());
        edited_authoring.resolved =
            resolve_mannequin_preview_motion(&edited_authoring, &motion_catalog);

        assert!(apply_resolved_mannequin_preview(
            &mut preview,
            edited_authoring.resolved.as_ref()
        ));
        assert_eq!(
            preview.motion_glb.as_deref(),
            Some("animations/run.anim.glb")
        );
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestBlendSource {
        source_path: String,
        blend_space: TestBlendDocument,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestBlendDocument {
        dimensions: Vec<TestBlendDimension>,
        examples: Vec<TestBlendExample>,
        virtual_examples: Vec<TestBlendVirtualExample>,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestBlendDimension {
        name: String,
        parameter_id: Option<u32>,
        min: f32,
        max: f32,
        cells: u32,
        locked: bool,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestBlendExample {
        animation: TestBlendAnimation,
        coordinates: Vec<TestBlendCoordinate>,
        playback_scale: f32,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestBlendAnimation {
        name: String,
        motion_path: Option<String>,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestBlendCoordinate {
        dimension: String,
        value: Option<f32>,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestBlendVirtualExample {
        indices: Vec<u32>,
        weights: Vec<f32>,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestMannequinSource {
        source_path: String,
        database: TestMannequinDatabase,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestMannequinDatabase {
        fragment_groups: Vec<TestMannequinFragmentGroup>,
        fragment_blends: Vec<TestMannequinFragmentBlend>,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestMannequinFragmentGroup {
        name: String,
        fragments: Vec<TestMannequinFragment>,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestMannequinFragment {
        tags: Option<String>,
        fragment_tags: Option<String>,
        animation_layers: Vec<TestMannequinAnimationLayer>,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestMannequinAnimationLayer {
        animations: Vec<TestMannequinAnimationEntry>,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestMannequinAnimationEntry {
        name: String,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize)]
    struct TestMannequinFragmentBlend {
        from: Option<String>,
        to: Option<String>,
        fragments: Vec<TestMannequinFragment>,
    }

    const TEST_BLEND_SOURCE_TYPE: &str = "test::BlendSource";
    const TEST_BLEND_DOCUMENT_TYPE: &str = "test::BlendDocument";
    const TEST_BLEND_DIMENSION_TYPE: &str = "test::BlendDimension";
    const TEST_BLEND_EXAMPLE_TYPE: &str = "test::BlendExample";
    const TEST_BLEND_ANIMATION_TYPE: &str = "test::BlendAnimation";
    const TEST_BLEND_COORDINATE_TYPE: &str = "test::BlendCoordinate";
    const TEST_BLEND_VIRTUAL_EXAMPLE_TYPE: &str = "test::BlendVirtualExample";
    const TEST_MANNEQUIN_SOURCE_TYPE: &str = "test::MannequinSource";
    const TEST_MANNEQUIN_DATABASE_TYPE: &str = "test::MannequinDatabase";
    const TEST_MANNEQUIN_GROUP_TYPE: &str = "test::MannequinFragmentGroup";
    const TEST_MANNEQUIN_FRAGMENT_TYPE: &str = "test::MannequinFragment";
    const TEST_MANNEQUIN_LAYER_TYPE: &str = "test::MannequinAnimationLayer";
    const TEST_MANNEQUIN_ENTRY_TYPE: &str = "test::MannequinAnimationEntry";
    const TEST_MANNEQUIN_BLEND_TYPE: &str = "test::MannequinFragmentBlend";
    const TEST_STRING_TYPE: &str = "alloc::string::String";
    const TEST_OPTION_STRING_TYPE: &str = "core::option::Option<alloc::string::String>";
    const TEST_OPTION_U32_TYPE: &str = "core::option::Option<u32>";
    const TEST_OPTION_F32_TYPE: &str = "core::option::Option<f32>";

    fn animation_edit_test_catalog() -> TypeRegistrySnapshot {
        let mut types = test_scalar_descriptors();
        types.extend(test_blend_space_descriptors());
        types.extend(test_mannequin_descriptors());
        types.extend(test_list_descriptors());
        TypeRegistrySnapshot {
            schema_catalog_hash: vec![1; 32],
            types,
        }
    }

    /// Scalar and `Option<T>` descriptors the authored fixtures reference.
    fn test_scalar_descriptors() -> Vec<ReflectedTypeDescriptor> {
        vec![
            test_reflected_type(TEST_STRING_TYPE, ReflectedTypeKind::String, vec![]),
            test_reflected_type("bool", ReflectedTypeKind::Bool, vec![]),
            test_reflected_type(
                "u32",
                ReflectedTypeKind::UnsignedInteger { bits: 32 },
                vec![],
            ),
            test_reflected_type("f32", ReflectedTypeKind::Float { bits: 32 }, vec![]),
            test_reflected_type(TEST_OPTION_STRING_TYPE, ReflectedTypeKind::Optional, vec![]),
            test_reflected_type(TEST_OPTION_U32_TYPE, ReflectedTypeKind::Optional, vec![]),
            test_reflected_type(TEST_OPTION_F32_TYPE, ReflectedTypeKind::Optional, vec![]),
        ]
    }

    /// Blend-space source, document, dimension, example, animation, coordinate,
    /// and virtual-example descriptors.
    fn test_blend_space_descriptors() -> Vec<ReflectedTypeDescriptor> {
        vec![
            test_reflected_type(
                TEST_BLEND_SOURCE_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field("source_path", TEST_STRING_TYPE),
                    test_reflected_field("blend_space", TEST_BLEND_DOCUMENT_TYPE),
                ],
            ),
            test_reflected_type(
                TEST_BLEND_DOCUMENT_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field("dimensions", &test_list_type(TEST_BLEND_DIMENSION_TYPE)),
                    test_reflected_field("examples", &test_list_type(TEST_BLEND_EXAMPLE_TYPE)),
                    test_reflected_field(
                        "virtual_examples",
                        &test_list_type(TEST_BLEND_VIRTUAL_EXAMPLE_TYPE),
                    ),
                ],
            ),
            test_reflected_type(
                TEST_BLEND_DIMENSION_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field("name", TEST_STRING_TYPE),
                    test_reflected_field("parameter_id", TEST_OPTION_U32_TYPE),
                    test_reflected_field("min", "f32"),
                    test_reflected_field("max", "f32"),
                    test_reflected_field("cells", "u32"),
                    test_reflected_field("locked", "bool"),
                ],
            ),
            test_reflected_type(
                TEST_BLEND_EXAMPLE_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field("animation", TEST_BLEND_ANIMATION_TYPE),
                    test_reflected_field(
                        "coordinates",
                        &test_list_type(TEST_BLEND_COORDINATE_TYPE),
                    ),
                    test_reflected_field("playback_scale", "f32"),
                ],
            ),
            test_reflected_type(
                TEST_BLEND_ANIMATION_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field("name", TEST_STRING_TYPE),
                    test_reflected_field("motion_path", TEST_OPTION_STRING_TYPE),
                ],
            ),
            test_reflected_type(
                TEST_BLEND_COORDINATE_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field("dimension", TEST_STRING_TYPE),
                    test_reflected_field("value", TEST_OPTION_F32_TYPE),
                ],
            ),
            test_reflected_type(
                TEST_BLEND_VIRTUAL_EXAMPLE_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field("indices", &test_list_type("u32")),
                    test_reflected_field("weights", &test_list_type("f32")),
                ],
            ),
        ]
    }

    /// Mannequin source, database, fragment group, fragment, animation layer,
    /// entry, and fragment-blend descriptors.
    fn test_mannequin_descriptors() -> Vec<ReflectedTypeDescriptor> {
        vec![
            test_reflected_type(
                TEST_MANNEQUIN_SOURCE_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field("source_path", TEST_STRING_TYPE),
                    test_reflected_field("database", TEST_MANNEQUIN_DATABASE_TYPE),
                ],
            ),
            test_reflected_type(
                TEST_MANNEQUIN_DATABASE_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field(
                        "fragment_groups",
                        &test_list_type(TEST_MANNEQUIN_GROUP_TYPE),
                    ),
                    test_reflected_field(
                        "fragment_blends",
                        &test_list_type(TEST_MANNEQUIN_BLEND_TYPE),
                    ),
                ],
            ),
            test_reflected_type(
                TEST_MANNEQUIN_GROUP_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field("name", TEST_STRING_TYPE),
                    test_reflected_field(
                        "fragments",
                        &test_list_type(TEST_MANNEQUIN_FRAGMENT_TYPE),
                    ),
                ],
            ),
            test_reflected_type(
                TEST_MANNEQUIN_FRAGMENT_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field("tags", TEST_OPTION_STRING_TYPE),
                    test_reflected_field("fragment_tags", TEST_OPTION_STRING_TYPE),
                    test_reflected_field(
                        "animation_layers",
                        &test_list_type(TEST_MANNEQUIN_LAYER_TYPE),
                    ),
                ],
            ),
            test_reflected_type(
                TEST_MANNEQUIN_LAYER_TYPE,
                ReflectedTypeKind::Struct,
                vec![test_reflected_field(
                    "animations",
                    &test_list_type(TEST_MANNEQUIN_ENTRY_TYPE),
                )],
            ),
            test_reflected_type(
                TEST_MANNEQUIN_ENTRY_TYPE,
                ReflectedTypeKind::Struct,
                vec![test_reflected_field("name", TEST_STRING_TYPE)],
            ),
            test_reflected_type(
                TEST_MANNEQUIN_BLEND_TYPE,
                ReflectedTypeKind::Struct,
                vec![
                    test_reflected_field("from", TEST_OPTION_STRING_TYPE),
                    test_reflected_field("to", TEST_OPTION_STRING_TYPE),
                    test_reflected_field(
                        "fragments",
                        &test_list_type(TEST_MANNEQUIN_FRAGMENT_TYPE),
                    ),
                ],
            ),
        ]
    }

    /// One list descriptor per element type the fixtures nest.
    fn test_list_descriptors() -> Vec<ReflectedTypeDescriptor> {
        vec![
            test_list_descriptor(TEST_BLEND_DIMENSION_TYPE),
            test_list_descriptor(TEST_BLEND_EXAMPLE_TYPE),
            test_list_descriptor(TEST_BLEND_VIRTUAL_EXAMPLE_TYPE),
            test_list_descriptor(TEST_BLEND_COORDINATE_TYPE),
            test_list_descriptor("u32"),
            test_list_descriptor("f32"),
            test_list_descriptor(TEST_MANNEQUIN_GROUP_TYPE),
            test_list_descriptor(TEST_MANNEQUIN_BLEND_TYPE),
            test_list_descriptor(TEST_MANNEQUIN_FRAGMENT_TYPE),
            test_list_descriptor(TEST_MANNEQUIN_LAYER_TYPE),
            test_list_descriptor(TEST_MANNEQUIN_ENTRY_TYPE),
        ]
    }

    fn test_reflected_type(
        type_path: &str,
        kind: ReflectedTypeKind,
        fields: Vec<ReflectedFieldDescriptor>,
    ) -> ReflectedTypeDescriptor {
        ReflectedTypeDescriptor {
            type_path: type_path.to_owned(),
            short_path: type_path
                .rsplit("::")
                .next()
                .unwrap_or(type_path)
                .to_owned(),
            kind,
            fields,
            variants: Vec::new(),
            editor_attributes: EditorAttributes::default(),
            type_data_flags: Vec::new(),
            applicability: ApplicabilityDescriptor::default(),
            reflected_default: None,
        }
    }

    fn test_reflected_field(name: &str, type_path: &str) -> ReflectedFieldDescriptor {
        ReflectedFieldDescriptor {
            name: name.to_owned(),
            type_path: type_path.to_owned(),
            editor_attributes: EditorAttributes::default(),
        }
    }

    fn test_list_type(element_type: &str) -> String {
        format!("alloc::vec::Vec<{element_type}>")
    }

    fn test_list_descriptor(element_type: &str) -> ReflectedTypeDescriptor {
        test_reflected_type(
            &test_list_type(element_type),
            ReflectedTypeKind::List,
            Vec::new(),
        )
    }

    fn reflected_test_inspection(
        type_path: &str,
        source_path: &str,
        value: &impl serde::Serialize,
        registry: &TypeRegistrySnapshot,
    ) -> ReflectedEntityInspection {
        let snapshot = PrefabSourceSnapshot {
            document_version: 1,
            type_versions: BTreeMap::new(),
            entities: vec![PrefabEntitySnapshot {
                alias: "root".to_owned(),
            }],
            hierarchy: vec![PrefabHierarchyEdge {
                child_alias: "root".to_owned(),
                parent_alias: None,
            }],
            components: vec![PrefabComponentSnapshot {
                entity_alias: "root".to_owned(),
                type_path: type_path.to_owned(),
                sparse_value: ReflectedValueEnvelope::typed_ron(
                    type_path,
                    ron::ser::to_string(value).expect("test value serializes as RON"),
                ),
            }],
            instances: Vec::new(),
            overrides: Vec::new(),
            revision: 1,
        };
        crate::authored_selection::project_reflected_selection(
            ReflectedPrefabSelection::new(source_path, "root"),
            registry,
            &snapshot,
            Vec::new(),
        )
        .expect("test Prefab projects as reflected inspection")
    }

    fn test_motion_catalog() -> EditorAnimationPreviewCatalog {
        EditorAnimationPreviewCatalog::new(
            None,
            vec![
                test_motion("animations/idle.anim.glb", "Idle"),
                test_motion("animations/run.anim.glb", "Run"),
            ],
            Vec::new(),
            Vec::new(),
        )
    }

    fn test_motion(asset_path: &str, name: &str) -> EditorAnimationMotionData {
        EditorAnimationMotionData {
            asset_path: asset_path.to_owned(),
            name: name.to_owned(),
            set_path: "animations".to_owned(),
            duration_millis: Some(1000),
            channel_count: 1,
            joint_targets: vec!["hips".to_owned()],
            events: Vec::new(),
            pipeline_status: Some("current".to_owned()),
        }
    }

    fn authoring_catalog_from_test_inspection(
        inspection: &ReflectedEntityInspection,
        motion_catalog: &EditorAnimationPreviewCatalog,
    ) -> EditorMannequinAuthoringCatalog {
        let database = reflected_root_field(inspection, &["database"]).unwrap();
        let known_motion_paths = motion_catalog
            .motions
            .iter()
            .map(|motion| motion.asset_path.as_str())
            .collect::<BTreeSet<_>>();
        let source_path = reflected_root_field(inspection, &["source_path"])
            .and_then(|field| reflected_string(field_value(field)))
            .unwrap();
        let fragments = reflected_mannequin_fragments(
            &database.value,
            &source_path,
            &known_motion_paths,
            &BTreeMap::new(),
        )
        .unwrap();
        let mut authoring = EditorMannequinAuthoringCatalog::empty();
        authoring.fragments = fragments;
        authoring
    }

    fn fixture_mannequin_authoring_state() -> (
        EditorMannequinAuthoringCatalog,
        EditorAnimationPreviewCatalog,
        EditorMannequinPreview,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let asset_root = temp.path().join("assets");
        write_fixture_gltf(
            &asset_root.join("animations/locomotion/idle.anim.glb"),
            "Idle",
            1.0,
            "hips",
        );
        write_fixture_gltf(
            &asset_root.join("animations/locomotion/idle_alert.anim.glb"),
            "IdleAlert",
            1.0,
            "hips",
        );
        write_mannequin_authoring_fixture(&asset_root);
        let status = asset_status_for_motions(
            &asset_root,
            &[
                "animations/locomotion/idle.anim.glb",
                "animations/locomotion/idle_alert.anim.glb",
            ],
        );
        let motion_catalog =
            build_animation_preview_catalog_from_status(&status, &asset_root, None);
        let catalog = build_mannequin_authoring_catalog(&asset_root, &motion_catalog);
        let preview = EditorMannequinPreview::default_for_project_asset_root(asset_root);
        (catalog, motion_catalog, preview)
    }

    fn write_mannequin_authoring_fixture(asset_root: &Path) {
        let root = asset_root.join("animations/mannequin/hero");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("hero.adb.ron"),
            r#"(
  database: (
    fragment_groups: [
      (
        name: "Idle",
        fragments: [
          (
            tags: None,
            fragment_tags: None,
            animation_layers: [
              (animations: [(name: "animations/locomotion/idle.anim.glb")]),
            ],
          ),
          (
            tags: Some("Alert"),
            fragment_tags: None,
            animation_layers: [
              (animations: [(name: "animations/locomotion/idle_alert.anim.glb")]),
            ],
          ),
          (
            tags: Some("Crouch"),
            fragment_tags: None,
            animation_layers: [
              (animations: [(name: "unresolved_idle_crouch.caf")]),
            ],
          ),
        ],
      ),
    ],
    fragment_blends: [
      (from: Some("Idle"), to: Some("Move"), variants: [(fragments: [])]),
    ],
  ),
)"#,
        )
        .unwrap();
        fs::write(
            root.join("hero.mannequin.tags.ron"),
            r#"(
  entries: [
    Tag((name: "Alert", priority: Some(10), sub_tag_definition: None)),
    Group((
      name: "Stance",
      tags: [
        (name: "Crouch", priority: None, sub_tag_definition: None),
        (name: "Stand", priority: None, sub_tag_definition: None),
      ],
    )),
  ],
)"#,
        )
        .unwrap();
        fs::write(
            root.join("hero.mannequin.controller.ron"),
            r#"(
  fragment_definitions: [
    (name: "Idle", scopes: "FullBody", flags: None, overrides: []),
  ],
  scope_contexts: [(name: "Character")],
  scopes: [
    (name: "FullBody", layer: 0, num_layers: 1, context: "Character", tags: None),
  ],
)"#,
        )
        .unwrap();
    }

    fn write_blend_space_fixture(asset_root: &Path) {
        let root = asset_root.join("animations/locomotion/blendspace");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("speed.bspace.ron"),
            r#"(
  source_path: "animations/locomotion/blendspace/speed.bspace",
  blend_space: (
    threshold: None,
    idle_to_move: false,
    dimensions: [
      (
        name: "Speed",
        parameter_id: Some(0),
        unresolved_parameter_reason: None,
        min: 0.0,
        max: 1.0,
        cells: 2,
        debug_visual_scale: 1.0,
        start_key: 0.0,
        end_key: 1.0,
        joint_name: None,
        locked: false,
      ),
    ],
    examples: [
      (
        animation: (
          name: "Idle",
          motion_path: Some("animations/locomotion/idle.anim.glb"),
          unresolved_motion_reason: None,
        ),
        coordinates: [
          (dimension: "Speed", value: Some(0.0), use_directly_for_delta_motion: false),
        ],
        playback_scale: 1.0,
      ),
      (
        animation: (
          name: "Run",
          motion_path: Some("animations/locomotion/run.anim.glb"),
          unresolved_motion_reason: None,
        ),
        coordinates: [
          (dimension: "Speed", value: Some(1.0), use_directly_for_delta_motion: false),
        ],
        playback_scale: 1.0,
      ),
    ],
    virtual_examples: [
      (
        indices: [0],
        weights: [1.0],
      ),
      (
        indices: [1],
        weights: [1.0],
      ),
    ],
  ),
)"#,
        )
        .unwrap();
    }

    fn write_fixture_gltf(path: &Path, animation_name: &str, duration: f32, target_joint: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let target_index = if target_joint == "hand_r" { 2 } else { 1 };
        fs::write(
            path,
            format!(
                r#"{{
  "asset": {{ "version": "2.0" }},
  "nodes": [
    {{ "name": "root", "children": [1] }},
    {{ "name": "hips", "children": [2] }},
    {{ "name": "hand_r" }}
  ],
  "scenes": [{{ "nodes": [0] }}],
  "scene": 0,
  "accessors": [
    {{ "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0.0], "max": [{duration}] }},
    {{ "componentType": 5126, "count": 2, "type": "VEC3" }}
  ],
  "animations": [{{
    "name": "{animation_name}",
    "samplers": [{{ "input": 0, "output": 1 }}],
    "channels": [{{ "sampler": 0, "target": {{ "node": {target_index}, "path": "translation" }} }}]
  }}]
}}"#
            ),
        )
        .unwrap();
    }

    fn write_character_gltf(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(
            path,
            r#"{
  "asset": { "version": "2.0" },
  "nodes": [
    { "name": "root", "children": [1] },
    { "name": "hips", "children": [2] },
    { "name": "hand_r" }
  ],
  "skins": [{ "joints": [0, 1, 2], "skeleton": 0 }],
  "scenes": [{ "nodes": [0] }],
  "scene": 0
}"#,
        )
        .unwrap();
    }

    #[test]
    fn dropping_the_slot_owned_controller_cancels_catalog_invalidation_work() {
        let (close, mut close_rx) = tokio::sync::watch::channel(());
        let controller = EditorMannequinAnimationController { _close: close };

        drop(controller);

        assert!(
            futures::executor::block_on(close_rx.changed()).is_err(),
            "the invalidation task must observe aggregate-slot replacement"
        );
    }

    /// Ticket 046 pinned the producer's emission table for enum variants, and
    /// this encoder sat on the wrong side of it: a struct-shaped variant
    /// retaining no field was written as the bare variant name — the one
    /// spelling the producer rejects — rather than the empty body it decodes.
    /// The declared descriptor is the authority: a variant that declares
    /// fields keeps its body even when the sparse value retains none of them,
    /// and only a variant declaring nothing is spelled bare.
    ///
    /// Both halves come from the real producer: `project_type_registry` builds
    /// the catalog this encoder reads, and `PrefabCodec` is asked to parse
    /// what it emits, so the acceptance claim is the producer's own.
    #[test]
    fn an_empty_struct_shaped_variant_encodes_the_body_the_producer_accepts() {
        use bevy::reflect::{Reflect, TypePath, TypeRegistry, std_traits::ReflectDefault};

        #[derive(Debug, Clone, Default, PartialEq, Reflect)]
        #[reflect(Default)]
        enum VariantShapeMode {
            #[default]
            Marker,
            Fieldless(),
            Pair(f32, bool),
            Named {
                alpha: f32,
                beta: bool,
            },
        }

        let mut registry = TypeRegistry::new();
        registry.register::<VariantShapeMode>();
        let catalog = az_project_host::project_type_registry(&registry)
            .expect("project the variant-shape catalog");
        let codec = az_prefab::PrefabCodec::new(&registry).expect("bind the Prefab codec");
        let type_path = VariantShapeMode::type_path();

        let encode = |value: &ReflectedValue| {
            reflected_value_ron(&catalog, type_path, value).expect("encode a variant")
        };
        let accepted = |payload: &str| {
            codec
                .decode_sparse_value(type_path, payload.as_bytes())
                .is_ok()
        };
        let variant = |name: &str, fields: Vec<(&str, ReflectedValue)>| ReflectedValue::Enum {
            variant: name.to_owned(),
            fields: fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        };
        let float = |value: &str| ReflectedValue::Scalar(ReflectedScalar::Float(value.to_owned()));
        let boolean = ReflectedValue::Scalar(ReflectedScalar::Bool(true));

        // The fix: a struct-shaped variant retaining nothing keeps its body...
        let empty_named = encode(&variant("Named", Vec::new()));
        assert_eq!(empty_named, "Named()");
        assert!(
            accepted(&empty_named),
            "the producer parses `{empty_named}`"
        );
        // ...and the spelling this encoder used to emit is the one the
        // producer refuses, so the old output was unreadable by its consumer.
        assert!(
            !accepted("Named"),
            "a struct-shaped variant still requires a body",
        );

        // Negative control: a variant declaring no field keeps the bare
        // spelling it always had.
        let marker = encode(&variant("Marker", Vec::new()));
        assert_eq!(marker, "Marker");
        assert!(accepted(&marker));

        // Negative controls: every populated shape encodes unchanged, and the
        // producer parses each one.
        let populated = encode(&variant(
            "Named",
            vec![("alpha", float("1.0")), ("beta", boolean.clone())],
        ));
        assert_eq!(populated, "Named(alpha:1.0,beta:true)");
        assert!(accepted(&populated));

        let partial = encode(&variant("Named", vec![("beta", boolean.clone())]));
        assert_eq!(partial, "Named(beta:true)");
        assert!(accepted(&partial));

        let pair = encode(&variant("Pair", vec![("0", float("1.0")), ("1", boolean)]));
        assert_eq!(pair, "Pair(1.0,true)");
        assert!(accepted(&pair));
    }
}
