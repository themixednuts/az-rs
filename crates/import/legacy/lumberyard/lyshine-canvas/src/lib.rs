//! `LyShine` canvas transformer.

pub mod builder;
pub mod source_transform;

pub use source_transform::*;

use anyhow::{Context, Result, bail};
use az_asset::{EngineTextureFormat, normalize_source_path};
use az_asset_builder::{
    BuildRuleRegistration, ProductFormat, ProductFormatRegistration, SourceFormat,
    SourceSchemaRegistration,
};
use az_core::{AssetData, AssetTypeRegistration, AzRtti, AzTypeInfo};
use az_framework::SCRIPT_COMPONENT_TYPE_ID;
use az_framework_objectstream::script::read_script_component;
use az_gem_lyshine::{
    UiBlendMode, UiButton, UiCanvas, UiCanvasAsset, UiCanvasFlags, UiChildOrder, UiComponentKind,
    UiElement, UiEntity, UiEntityId, UiFader, UiImage, UiImageFillCornerOrigin,
    UiImageFillEdgeOrigin, UiImageFillType, UiImageSpriteType, UiImageType, UiLayoutAxis,
    UiLayoutCell, UiLayoutGrid, UiMask, UiRect, UiText, UiTransform2d,
};
use az_objectstream::asset_reference::{
    AssetValueError, SimpleAssetReferenceElementError, read_asset_value,
    read_simple_asset_reference_path,
};
use az_objectstream::context::{ContainerShape, ObjectStreamReadContext};
use az_objectstream::types;
use az_objectstream::value;
use az_objectstream::value::child_by_field_any;
use az_objectstream::{Element, ObjectStream};
use bevy::color::LinearRgba;
use bevy::prelude::*;
use texture_atlas::TEXTURE_ATLAS_ASSET_REFERENCE_TYPE_ID;
use uuid::{Uuid, uuid};

pub struct UiCanvasAssetData;

impl AzTypeInfo for UiCanvasAssetData {
    const NAME: &'static str = "LyShine::UiCanvasAsset";
    const TYPE_ID: Uuid = uuid!("678edd5b-8f5c-405f-e1af-25f70de01906");
}

impl AzRtti for UiCanvasAssetData {}

impl AssetData for UiCanvasAssetData {
    const STABLE_NAME: &'static str = "azoth.compat.lyshine.ui-canvas";
}

#[derive(SourceFormat)]
#[source(
    schema = "azoth.compat.lyshine.UiCanvasSource",
    ext = "uicanvas",
    ext = "dynamicuicanvas"
)]
pub struct UiCanvasSourceFormat;

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.compat.lyshine.ui-canvas",
    version = 1,
    asset = UiCanvasAssetData
)]
pub struct UiCanvasProductFormat;

pub mod ids {
    use super::{AssetData, UiCanvasAssetData};
    use az_core::AssetType;

    /// `lyshine::UiCanvasAsset` (`.uicanvas`, `.dynamicuicanvas`, az-rs minted).
    pub const UI_CANVAS: AssetType = UiCanvasAssetData::ASSET_TYPE;
}

pub mod source_schemas {
    use super::{SourceFormat, UiCanvasSourceFormat};

    pub const UI_CANVAS: az_asset_builder::SourceSchemaType =
        match <UiCanvasSourceFormat as SourceFormat>::SCHEMA {
            Some(schema) => schema,
            None => panic!("UiCanvasSourceFormat declares a schema"),
        };
}

pub mod product_formats {
    use super::{ProductFormat, UiCanvasProductFormat};

    /// `LyShine` UI canvas product bytes.
    pub const LYSHINE_UI_CANVAS: az_asset_builder::ProductFormatId =
        <UiCanvasProductFormat as ProductFormat>::ID;
}

/// The asset types this crate owns, for a host contribution to register.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 1] {
    [
        AssetTypeRegistration::for_asset::<UiCanvasAssetData>()
            .with_owner("lyshine-canvas::builder"),
    ]
}

/// The product formats this crate owns, for a host contribution to register.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 1] {
    [ProductFormatRegistration::for_format::<UiCanvasProductFormat>()]
}

/// The source schemas this crate owns, for a host contribution to register.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 1] {
    [
        SourceSchemaRegistration::for_source::<UiCanvasSourceFormat>()
            .with_category("LyShine Compatibility")
            .with_import_file("ui", &["uicanvas", "dynamicuicanvas"]),
    ]
}

/// The build rules this crate owns, for a host contribution to register.
///
/// Empty: [`builder::desc`] claims `.uicanvas` sources but its job step needs
/// captured `ObjectStream` class data, which no composed registry carries. A host
/// that registered it would claim every canvas source and fail every one, so the
/// rule stays out of the contribution until the capture is composable.
#[must_use]
pub const fn build_rules() -> [BuildRuleRegistration; 0] {
    []
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<AssetTypeRegistration>()
        .register_many(asset_types());
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

const AZ_ENTITY_TYPE_ID: Uuid = types::AZ_ENTITY;
const ENTITY_ID_TYPE_ID: Uuid = types::ENTITY_ID;
const SLICE_COMPONENT_TYPE_ID: Uuid = types::SLICE_COMPONENT;
const UI_CANVAS_FILE_OBJECT_TYPE_ID: Uuid = uuid!("1f02632f-f113-49b1-85ad-8cd0fa78b8aa");
const UI_CANVAS_COMPONENT_TYPE_ID: Uuid = uuid!("50b8cf6c-b19a-4d86-afe9-96efb820d422");
const UI_TRANSFORM_2D_COMPONENT_TYPE_ID: Uuid = uuid!("2751a5a5-3291-4a4d-9fc0-9cb0eb8d1de6");
const UI_ELEMENT_COMPONENT_TYPE_ID: Uuid = uuid!("4a97d63e-ce7a-45b6-aae4-102db4334688");
const UI_IMAGE_COMPONENT_TYPE_ID: Uuid = uuid!("bdbefd23-dbb4-4726-a32d-4feac24e51f6");
const UI_TEXT_COMPONENT_TYPE_ID: Uuid = uuid!("5b3fb2a7-5dc4-4033-a970-001cec85b6c4");
const UI_BUTTON_COMPONENT_TYPE_ID: Uuid = uuid!("7329dfe8-0f3c-4629-b395-78b2cf646b96");
const UI_INTERACTABLE_COMPONENT_TYPE_ID: Uuid = uuid!("a42eb486-1c89-434c-ad22-a3fc6ceec46f");
const UI_FADER_COMPONENT_TYPE_ID: Uuid = uuid!("cd01ff77-2249-4ed8-bffb-33a66a47e17c");
const UI_MASK_COMPONENT_TYPE_ID: Uuid = uuid!("2279aa38-271d-4d4f-a472-e42b984088ac");
const UI_LAYOUT_ROW_COMPONENT_TYPE_ID: Uuid = uuid!("7b2820c4-7fc7-4f02-b777-6727eb4bac13");
const UI_LAYOUT_COLUMN_COMPONENT_TYPE_ID: Uuid = uuid!("4bc2e786-360b-4426-8d9c-9b254c5ea21f");
const UI_LAYOUT_GRID_COMPONENT_TYPE_ID: Uuid = uuid!("adda3ae5-b9ab-44b7-a462-8b89b398a837");
const UI_LAYOUT_CELL_COMPONENT_TYPE_ID: Uuid = uuid!("a0568e58-4382-47f8-8b88-77c64b99ac80");

const FONT_ASSET_REFERENCE_TYPE_ID: Uuid = uuid!("d6342379-a5fa-4b18-b890-702c2fe99a5a");

pub type UiCanvasTransformError = anyhow::Error;

/// Transform a `LyShine` canvas `ObjectStream` payload with a product font resolver.
///
/// # Errors
///
/// Returns an [`anyhow::Error`] (aliased as [`UiCanvasTransformError`]) when
/// the payload is not a canvas this reader accepts: the `ObjectStream` itself
/// fails to parse, the root is not a `UiCanvasAssetRef`, a required element or
/// field is absent, a field carries a reflected type the reader does not map
/// (a non-numeric scalar where a number is required, a malformed asset
/// reference), or an entity/element id does not resolve. Any error the
/// `font_path` resolver returns for an embedded font reference is propagated
/// unchanged. Each error is annotated with the field or element being read.
pub fn transform_ui_canvas_asset<F>(
    bytes: &[u8],
    context: &ObjectStreamReadContext,
    _: EngineTextureFormat,
    mut font_path: F,
) -> Result<UiCanvasAsset>
where
    F: FnMut(&str) -> Result<String>,
{
    let stream = ObjectStream::from_bytes_with_context(bytes, context)
        .context("parse UI canvas ObjectStream")?;
    let mut source_entities = Vec::new();
    for root in stream.elements() {
        collect_canvas_entities(root, &mut source_entities)?;
    }

    let mut canvas = None;
    let mut entities = Vec::with_capacity(source_entities.len());
    for entity in source_entities {
        let imported = read_ui_entity(entity, &mut font_path)?;
        if let Some(value) = imported.canvas {
            canvas = Some(value);
        }
        entities.push(imported.entity);
    }

    let canvas = canvas.context("UI canvas ObjectStream is missing a UiCanvasComponent")?;
    if canvas.root_entity.is_null() {
        bail!("UI canvas ObjectStream has a null RootElement");
    }

    Ok(UiCanvasAsset::new(canvas, entities))
}

fn collect_canvas_entities<'a>(root: &'a Element, out: &mut Vec<&'a Element>) -> Result<()> {
    match value::semantic_type_id(root)? {
        AZ_ENTITY_TYPE_ID => collect_entities_from_az_entity(root, out),
        UI_CANVAS_FILE_OBJECT_TYPE_ID => {
            if let Some(canvas_entity) = value::child_by_field(root, "CanvasEntity") {
                out.push(canvas_entity);
            }
            if let Some(root_slice_entity) = value::child_by_field(root, "RootSliceEntity") {
                collect_entities_from_az_entity(root_slice_entity, out)?;
            }
            Ok(())
        }
        actual => bail!("unexpected UI canvas root type {actual}"),
    }
}

fn collect_entities_from_az_entity<'a>(
    entity: &'a Element,
    out: &mut Vec<&'a Element>,
) -> Result<()> {
    let Some(slice_entities) = slice_component_entities(entity)? else {
        out.push(entity);
        return Ok(());
    };
    for slice_entity in slice_entities {
        collect_entities_from_az_entity(slice_entity, out)?;
    }
    Ok(())
}

fn slice_component_entities(entity: &Element) -> Result<Option<&[Element]>> {
    let Some(components) = value::child_by_field(entity, "Components") else {
        return Ok(None);
    };
    require_sequence(components, "AZStd::vector<AZ::Component*>")?;
    let Some(slice) = find_child_by_semantic_type(components, SLICE_COMPONENT_TYPE_ID)? else {
        return Ok(None);
    };
    let entities =
        value::child_by_field(slice, "Entities").context("SliceComponent is missing Entities")?;
    require_sequence(entities, "AZStd::vector<AZ::Entity>")?;
    Ok(Some(entities.children()))
}

struct ImportedUiEntity {
    entity: UiEntity,
    canvas: Option<UiCanvas>,
}

fn read_ui_entity(
    element: &Element,
    font_path: &mut impl FnMut(&str) -> Result<String>,
) -> Result<ImportedUiEntity> {
    let element_type = value::semantic_type_id(element)?;
    if element_type != AZ_ENTITY_TYPE_ID {
        bail!("expected AZ::Entity, got {element_type}");
    }

    let entity_id = read_entity_id_field_any(element, &["id", "ID"])?;
    let mut entity = UiEntity::new(entity_id);
    entity.name = read_string_field(element, "Name")?.map(str::to_owned);
    entity.dependency_ready = read_bool_field(element, "IsDependencyReady")?.unwrap_or(false);
    entity.runtime_active = read_bool_field(element, "IsRuntimeActive")?.unwrap_or(false);

    let mut canvas = None;
    if let Some(components) = value::child_by_field(element, "Components") {
        require_sequence(components, "AZStd::vector<AZ::Component*>")?;
        for component in components.children() {
            let component_type = value::semantic_type_id(component)?;
            let kind = component_kind(component_type);
            if !entity.components.contains(&kind) {
                entity.components.push(kind);
            }

            match component_type {
                UI_CANVAS_COMPONENT_TYPE_ID => {
                    canvas = Some(read_canvas_component(component)?);
                }
                UI_TRANSFORM_2D_COMPONENT_TYPE_ID => {
                    entity.transform = Some(read_transform_component(component)?);
                }
                UI_ELEMENT_COMPONENT_TYPE_ID => {
                    entity.element = Some(read_element_component(component)?);
                }
                UI_IMAGE_COMPONENT_TYPE_ID => {
                    entity.image = Some(read_image_component(component)?);
                }
                UI_TEXT_COMPONENT_TYPE_ID => {
                    entity.text = Some(read_text_component(component, font_path)?);
                }
                UI_BUTTON_COMPONENT_TYPE_ID => {
                    entity.button = Some(read_button_component(component)?);
                    if !entity.components.contains(&UiComponentKind::Interactable) {
                        entity.components.push(UiComponentKind::Interactable);
                    }
                }
                UI_FADER_COMPONENT_TYPE_ID => {
                    entity.fader = Some(read_fader_component(component)?);
                }
                UI_MASK_COMPONENT_TYPE_ID => {
                    entity.mask = Some(read_mask_component(component)?);
                }
                UI_LAYOUT_ROW_COMPONENT_TYPE_ID => {
                    entity.layout_row = Some(read_layout_axis_component(component)?);
                }
                UI_LAYOUT_COLUMN_COMPONENT_TYPE_ID => {
                    entity.layout_column = Some(read_layout_axis_component(component)?);
                }
                UI_LAYOUT_GRID_COMPONENT_TYPE_ID => {
                    entity.layout_grid = Some(read_layout_grid_component(component)?);
                }
                UI_LAYOUT_CELL_COMPONENT_TYPE_ID => {
                    entity.layout_cell = Some(read_layout_cell_component(component)?);
                }
                SCRIPT_COMPONENT_TYPE_ID => {
                    entity.script = Some(read_script_component(component)?);
                }
                _ => {}
            }
        }
    }

    Ok(ImportedUiEntity { entity, canvas })
}

fn read_canvas_component(element: &Element) -> Result<UiCanvas> {
    let mut canvas = UiCanvas {
        unique_id: read_u64_field(element, "UniqueId")?.unwrap_or(0),
        root_entity: read_entity_id_field(element, "RootElement")?,
        first_hover_entity: read_entity_id_field(element, "FirstHoverElement")
            .unwrap_or(UiEntityId::new(0)),
        tooltip_display_entity: read_entity_id_field(element, "TooltipDisplayElement")
            .unwrap_or(UiEntityId::new(0)),
        last_element_id: read_u32_field(element, "LastElement")?.unwrap_or(0),
        size: read_vec2_field(element, "CanvasSize")?.unwrap_or(Vec2::ZERO),
        draw_order: read_i32_field(element, "DrawOrder")?.unwrap_or(0),
        render_target_name: read_non_empty_string_field(element, "RenderTargetName")?,
        ..Default::default()
    };
    canvas.flags = UiCanvasFlags {
        snap_enabled: read_bool_field(element, "IsSnapEnabled")?.unwrap_or(false),
        pixel_aligned: read_bool_field(element, "IsPixelAligned")?.unwrap_or(false),
        render_to_texture: read_bool_field(element, "RenderToTexture")?.unwrap_or(false),
        transform_update_optimize_enabled: read_bool_field(
            element,
            "EnableTransformUpdateOptimize",
        )?
        .unwrap_or(false),
        optimize_for_frequent_updates: read_bool_field(element, "OptimizeForFrequentUpdates")?
            .unwrap_or(false),
        position_input_supported: read_bool_field(element, "IsPosInputSupported")?.unwrap_or(false),
        navigation_supported: read_bool_field(element, "IsNavigationSupported")?.unwrap_or(false),
        always_allows_hover: read_bool_field(element, "IsAlwaysAllowingHover")?.unwrap_or(false),
        ignore_scroll_hover: read_bool_field(element, "IgnoreScrollHover")?.unwrap_or(false),
        enter_handling_disabled: read_bool_field(element, "DisableEnterHandling")?.unwrap_or(false),
        guides_locked: read_bool_field(element, "GuidesLocked")?.unwrap_or(false),
    };
    canvas.texture_atlases = read_texture_atlases(element, "TextureAtlases")?.unwrap_or_default();
    Ok(canvas)
}

fn read_transform_component(element: &Element) -> Result<UiTransform2d> {
    Ok(UiTransform2d {
        anchors: read_rect_field(element, "Anchors")?.unwrap_or_default(),
        offsets: read_rect_field(element, "Offsets")?.unwrap_or_default(),
        pivot: read_vec2_field(element, "Pivot")?.unwrap_or(Vec2::ZERO),
        rotation: read_f32_field(element, "Rotation")?.unwrap_or(0.0),
        scale: read_vec2_field(element, "Scale")?.unwrap_or(Vec2::ONE),
        scale_to_device: read_bool_field(element, "ScaleToDevice")?.unwrap_or(false),
        compute_transform_when_hidden: read_bool_field(element, "ComputeTransformWhenHidden")?
            .unwrap_or(false),
    })
}

fn read_element_component(element: &Element) -> Result<UiElement> {
    Ok(UiElement {
        local_id: read_u32_field_any(element, &["id", "ID"])?.unwrap_or(0),
        enabled: read_bool_field(element, "IsEnabled")?.unwrap_or(true),
        visible_in_editor: read_bool_field(element, "IsVisibleInEditor")?.unwrap_or(true),
        selectable_in_editor: read_bool_field(element, "IsSelectableInEditor")?.unwrap_or(true),
        selected_in_editor: read_bool_field(element, "IsSelectedInEditor")?.unwrap_or(false),
        expanded_in_editor: read_bool_field(element, "IsExpandedInEditor")?.unwrap_or(false),
        child_order: read_child_order_field(element, "ChildEntityIdOrder")?.unwrap_or_default(),
        children_render_sortable: read_bool_field(element, "IsChildrenRenderSortable")?
            .unwrap_or(false),
        render_priority: read_i32_field(element, "RenderPriority")?.unwrap_or(0),
        multithread_children: read_bool_field(element, "MultithreadChildren")?.unwrap_or(false),
    })
}

fn read_image_component(element: &Element) -> Result<UiImage> {
    Ok(UiImage {
        sprite_type: read_ui_image_sprite_type_field(element, "SpriteType")?,
        sprite_path: read_sprite_texture_field(element, "SpriteTexture")?,
        sprite_index: read_u32_field(element, "Index")?.unwrap_or(0),
        render_target_name: read_non_empty_string_field(element, "RenderTargetName")?,
        render_target_srgb: read_bool_field(element, "IsRenderTargetSRGB")?.unwrap_or(false),
        color: read_color_field(element, "Color")?.unwrap_or(LinearRgba::WHITE),
        alpha: read_f32_field(element, "Alpha")?.unwrap_or(1.0),
        image_type: read_ui_image_type_field(element, "ImageType")?,
        fill_center: read_bool_field(element, "FillCenter")?.unwrap_or(true),
        stretch_sliced: read_bool_field(element, "StretchSliced")?.unwrap_or(false),
        blend_mode: read_ui_blend_mode_field(element, "BlendMode")?,
        fill_type: read_ui_image_fill_type_field(element, "FillType")?,
        fill_amount: read_f32_field(element, "FillAmount")?.unwrap_or(1.0),
        fill_start_angle: read_f32_field(element, "FillStartAngle")?.unwrap_or(0.0),
        fill_corner_origin: read_ui_image_fill_corner_origin_field(element, "FillCornerOrigin")?,
        fill_edge_origin: read_ui_image_fill_edge_origin_field(element, "FillEdgeOrigin")?,
        fill_clockwise: read_bool_field(element, "FillClockwise")?.unwrap_or(true),
    })
}

fn read_ui_image_sprite_type_field(
    element: &Element,
    field: &'static str,
) -> Result<UiImageSpriteType> {
    let value = read_char_field(element, field)?.unwrap_or(UiImageSpriteType::SpriteAsset.as_u8());
    UiImageSpriteType::from_u8(value)
        .with_context(|| format!("invalid UI image {field} value {value}"))
}

fn read_ui_image_type_field(element: &Element, field: &'static str) -> Result<UiImageType> {
    let value = read_char_field(element, field)?.unwrap_or(UiImageType::Stretched.as_u8());
    UiImageType::from_u8(value).with_context(|| format!("invalid UI image {field} value {value}"))
}

fn read_ui_blend_mode_field(element: &Element, field: &'static str) -> Result<UiBlendMode> {
    let value = read_i32_field(element, field)?.unwrap_or(UiBlendMode::Normal.as_i32());
    UiBlendMode::from_i32(value).with_context(|| format!("invalid UI image {field} value {value}"))
}

fn read_ui_image_fill_type_field(
    element: &Element,
    field: &'static str,
) -> Result<UiImageFillType> {
    let value = read_char_field(element, field)?.unwrap_or(UiImageFillType::None.as_u8());
    UiImageFillType::from_u8(value)
        .with_context(|| format!("invalid UI image {field} value {value}"))
}

fn read_ui_image_fill_corner_origin_field(
    element: &Element,
    field: &'static str,
) -> Result<UiImageFillCornerOrigin> {
    let value =
        read_char_field(element, field)?.unwrap_or(UiImageFillCornerOrigin::TopLeft.as_u8());
    UiImageFillCornerOrigin::from_u8(value)
        .with_context(|| format!("invalid UI image {field} value {value}"))
}

fn read_ui_image_fill_edge_origin_field(
    element: &Element,
    field: &'static str,
) -> Result<UiImageFillEdgeOrigin> {
    let value = read_char_field(element, field)?.unwrap_or(UiImageFillEdgeOrigin::Left.as_u8());
    UiImageFillEdgeOrigin::from_u8(value)
        .with_context(|| format!("invalid UI image {field} value {value}"))
}

fn read_text_component(
    element: &Element,
    font_path: &mut impl FnMut(&str) -> Result<String>,
) -> Result<UiText> {
    Ok(UiText {
        text: read_string_field(element, "Text")?.unwrap_or("").to_owned(),
        markup_enabled: read_bool_field(element, "MarkupEnabled")?.unwrap_or(false),
        images_enabled: read_bool_field(element, "ImagesEnabled")?.unwrap_or(false),
        update_on_input_change: read_bool_field(element, "UpdateOnInputChange")?.unwrap_or(false),
        color: read_color_field(element, "Color")?.unwrap_or(LinearRgba::WHITE),
        alpha: read_f32_field(element, "Alpha")?.unwrap_or(1.0),
        font_path: read_simple_ref_path_field(
            element,
            "FontFileName",
            FONT_ASSET_REFERENCE_TYPE_ID,
        )?
        .map(font_path)
        .transpose()?,
        font_effect_index: read_u32_field(element, "FontEffectIndex")?.unwrap_or(0),
        font_size: read_f32_field(element, "FontSize")?.unwrap_or(24.0),
        character_spacing: read_f32_field(element, "CharSpacing")?.unwrap_or(0.0),
        line_spacing: read_f32_field(element, "LineSpacing")?.unwrap_or(1.0),
        horizontal_alignment: read_i32_or_byte_field(element, "TextHAlignment")?.unwrap_or(0),
        vertical_alignment: read_i32_or_byte_field(element, "TextVAlignment")?.unwrap_or(0),
        wrap_text_setting: read_i32_or_byte_field(element, "WrapTextSetting")?.unwrap_or(0),
        overflow_mode: read_i32_or_byte_field(element, "OverflowMode")?.unwrap_or(0),
    })
}

fn read_button_component(element: &Element) -> Result<UiButton> {
    let base = find_child_by_semantic_type(element, UI_INTERACTABLE_COMPONENT_TYPE_ID)?;
    let action_source = base.unwrap_or(element);
    Ok(UiButton {
        hover_start_action_name: read_non_empty_string_field(
            action_source,
            "HoverStartActionName",
        )?,
        hover_end_action_name: read_non_empty_string_field(action_source, "HoverEndActionName")?,
        pressed_action_name: read_non_empty_string_field(action_source, "PressedActionName")?,
        released_action_name: read_non_empty_string_field(action_source, "ReleasedActionName")?,
        action_name: read_non_empty_string_field(action_source, "ActionName")?,
        action_name_right: read_non_empty_string_field(action_source, "ActionNameRight")?,
        action_name_pressed_right: read_non_empty_string_field(
            action_source,
            "ActionNamePressedRight",
        )?,
        use_click_behavior: read_bool_field(action_source, "UseClickBehavior")?.unwrap_or(false),
        click_sq_tolerance: read_f32_field(action_source, "ClickSqTolerance")?.unwrap_or(0.0),
    })
}

fn read_fader_component(element: &Element) -> Result<UiFader> {
    Ok(UiFader {
        fade: read_f32_field(element, "Fade")?.unwrap_or(1.0),
        use_render_to_texture: read_bool_field(element, "UseRenderToTexture")?.unwrap_or(false),
    })
}

fn read_mask_component(element: &Element) -> Result<UiMask> {
    Ok(UiMask {
        enable_masking: read_bool_field(element, "EnableMasking")?.unwrap_or(true),
        mask_interaction: read_bool_field(element, "MaskInteraction")?.unwrap_or(true),
        child_mask_element: read_optional_entity_id_field(element, "ChildMaskElement")?
            .unwrap_or(UiEntityId::new(0)),
        use_render_to_texture: read_bool_field(element, "UseRenderToTexture")?.unwrap_or(false),
        draw_behind: read_bool_field(element, "DrawBehind")?.unwrap_or(false),
        draw_in_front: read_bool_field(element, "DrawInFront")?.unwrap_or(false),
        use_alpha_test: read_bool_field(element, "UseAlphaTest")?.unwrap_or(false),
    })
}

fn read_layout_axis_component(element: &Element) -> Result<UiLayoutAxis> {
    Ok(UiLayoutAxis {
        padding: read_rect_field(element, "Padding")?.unwrap_or_default(),
        spacing: read_f32_field(element, "Spacing")?.unwrap_or(0.0),
        order: read_i32_or_byte_field(element, "Order")?.unwrap_or(0),
        child_h_alignment: read_i32_or_byte_field(element, "ChildHAlignment")?.unwrap_or(0),
        child_v_alignment: read_i32_or_byte_field(element, "ChildVAlignment")?.unwrap_or(0),
        ignore_default_layout_cells: read_bool_field(element, "IgnoreDefaultLayoutCells")?
            .unwrap_or(false),
    })
}

fn read_layout_grid_component(element: &Element) -> Result<UiLayoutGrid> {
    Ok(UiLayoutGrid {
        padding: read_rect_field(element, "Padding")?.unwrap_or_default(),
        spacing: read_vec2_field(element, "Spacing")?.unwrap_or(Vec2::ZERO),
        cell_size: read_vec2_field(element, "CellSize")?.unwrap_or(Vec2::ZERO),
        horizontal_order: read_i32_or_byte_field(element, "HorizontalOrder")?.unwrap_or(0),
        vertical_order: read_i32_or_byte_field(element, "VerticalOrder")?.unwrap_or(0),
        starting_with: read_i32_or_byte_field(element, "StartingWith")?.unwrap_or(0),
        child_h_alignment: read_i32_or_byte_field(element, "ChildHAlignment")?.unwrap_or(0),
        child_v_alignment: read_i32_or_byte_field(element, "ChildVAlignment")?.unwrap_or(0),
    })
}

fn read_layout_cell_component(element: &Element) -> Result<UiLayoutCell> {
    Ok(UiLayoutCell {
        min_width_overridden: read_bool_field(element, "MinWidthOverridden")?.unwrap_or(false),
        min_width: read_f32_field(element, "MinWidth")?.unwrap_or(0.0),
        min_height_overridden: read_bool_field(element, "MinHeightOverridden")?.unwrap_or(false),
        min_height: read_f32_field(element, "MinHeight")?.unwrap_or(0.0),
        target_width_overridden: read_bool_field(element, "TargetWidthOverridden")?
            .unwrap_or(false),
        target_width: read_f32_field(element, "TargetWidth")?.unwrap_or(0.0),
        target_height_overridden: read_bool_field(element, "TargetHeightOverridden")?
            .unwrap_or(false),
        target_height: read_f32_field(element, "TargetHeight")?.unwrap_or(0.0),
        max_width_overridden: read_bool_field(element, "MaxWidthOverridden")?.unwrap_or(false),
        max_width: read_f32_field(element, "MaxWidth")?.unwrap_or(0.0),
        max_height_overridden: read_bool_field(element, "MaxHeightOverridden")?.unwrap_or(false),
        max_height: read_f32_field(element, "MaxHeight")?.unwrap_or(0.0),
        extra_width_ratio_overridden: read_bool_field(element, "ExtraWidthRatioOverridden")?
            .unwrap_or(false),
        extra_width_ratio: read_f32_field(element, "ExtraWidthRatio")?.unwrap_or(1.0),
        extra_height_ratio_overridden: read_bool_field(element, "ExtraHeightRatioOverridden")?
            .unwrap_or(false),
        extra_height_ratio: read_f32_field(element, "ExtraHeightRatio")?.unwrap_or(1.0),
    })
}

const fn component_kind(type_id: Uuid) -> UiComponentKind {
    match type_id {
        UI_CANVAS_COMPONENT_TYPE_ID => UiComponentKind::Canvas,
        UI_TRANSFORM_2D_COMPONENT_TYPE_ID => UiComponentKind::Transform2d,
        UI_ELEMENT_COMPONENT_TYPE_ID => UiComponentKind::Element,
        UI_IMAGE_COMPONENT_TYPE_ID => UiComponentKind::Image,
        UI_TEXT_COMPONENT_TYPE_ID => UiComponentKind::Text,
        UI_BUTTON_COMPONENT_TYPE_ID => UiComponentKind::Button,
        UI_INTERACTABLE_COMPONENT_TYPE_ID => UiComponentKind::Interactable,
        UI_FADER_COMPONENT_TYPE_ID => UiComponentKind::Fader,
        UI_MASK_COMPONENT_TYPE_ID => UiComponentKind::Mask,
        UI_LAYOUT_ROW_COMPONENT_TYPE_ID => UiComponentKind::LayoutRow,
        UI_LAYOUT_COLUMN_COMPONENT_TYPE_ID => UiComponentKind::LayoutColumn,
        UI_LAYOUT_GRID_COMPONENT_TYPE_ID => UiComponentKind::LayoutGrid,
        UI_LAYOUT_CELL_COMPONENT_TYPE_ID => UiComponentKind::LayoutCell,
        SCRIPT_COMPONENT_TYPE_ID => UiComponentKind::Script,
        _ => UiComponentKind::Other,
    }
}

fn read_entity_id_field(element: &Element, field: &str) -> Result<UiEntityId> {
    read_entity_id_field_any(element, &[field])
}

fn read_entity_id_field_any(element: &Element, fields: &[&str]) -> Result<UiEntityId> {
    let field_element = child_by_field_any(element, fields).with_context(|| {
        format!(
            "{} is missing required EntityId field {}",
            element.name(),
            fields.join("|")
        )
    })?;
    read_entity_id(field_element)
}

fn read_optional_entity_id_field(element: &Element, field: &str) -> Result<Option<UiEntityId>> {
    value::child_by_field(element, field)
        .map(read_entity_id)
        .transpose()
}

fn read_entity_id(element: &Element) -> Result<UiEntityId> {
    let actual = value::semantic_type_id(element)?;
    if actual != ENTITY_ID_TYPE_ID {
        bail!("expected EntityId, got {actual}");
    }
    let id = child_by_field_any(element, &["id", "ID"])
        .with_context(|| format!("{} EntityId is missing id|ID value field", element.name()))?;
    Ok(UiEntityId::new(value::read_u64(id)?))
}

fn read_child_order_field(element: &Element, field: &str) -> Result<Option<Vec<UiChildOrder>>> {
    let Some(vector) = value::child_by_field(element, field) else {
        return Ok(None);
    };
    require_sequence(vector, "AZStd::vector<UiChildOrder>")?;
    let mut values = Vec::with_capacity(vector.children().len());
    for child in vector.children() {
        let entity = read_entity_id_field(child, "ChildEntityId")?;
        let sort_index = read_u64_field(child, "SortIndex")?.unwrap_or(0);
        values.push(UiChildOrder::new(entity, sort_index));
    }
    Ok(Some(values))
}

fn read_texture_atlases(element: &Element, field: &str) -> Result<Option<Vec<String>>> {
    let Some(vector) = value::child_by_field(element, field) else {
        return Ok(None);
    };
    require_sequence(vector, "AZStd::vector<SimpleAssetReference<TextureAtlas>>")?;
    let mut values = Vec::with_capacity(vector.children().len());
    for child in vector.children() {
        let path = read_simple_asset_reference_path(child, TEXTURE_ATLAS_ASSET_REFERENCE_TYPE_ID)
            .map_err(|error| simple_ref_error(&error))?;
        values.push(engine_path_for_source(path));
    }
    Ok(Some(values))
}

fn read_sprite_texture_field(element: &Element, field: &str) -> Result<Option<String>> {
    let Some(field_element) = value::child_by_field(element, field) else {
        return Ok(None);
    };
    let asset = read_asset_value(field_element).map_err(|error| asset_value_error(&error))?;
    let hint = asset.hint().trim();
    Ok((!hint.is_empty()).then(|| sprite_texture_engine_path(hint)))
}

fn read_simple_ref_path_field<'a>(
    element: &'a Element,
    field: &str,
    expected_type_id: Uuid,
) -> Result<Option<&'a str>> {
    let Some(field_element) = value::child_by_field(element, field) else {
        return Ok(None);
    };
    read_simple_asset_reference_path(field_element, expected_type_id)
        .map(Some)
        .map_err(|error| simple_ref_error(&error))
}

fn read_rect_field(element: &Element, field: &str) -> Result<Option<UiRect>> {
    let Some(rect) = value::child_by_field(element, field) else {
        return Ok(None);
    };
    Ok(Some(UiRect::new(
        read_f32_field(rect, "left")?.unwrap_or(0.0),
        read_f32_field(rect, "top")?.unwrap_or(0.0),
        read_f32_field(rect, "right")?.unwrap_or(0.0),
        read_f32_field(rect, "bottom")?.unwrap_or(0.0),
    )))
}

fn read_color_field(element: &Element, field: &str) -> Result<Option<LinearRgba>> {
    let Some(field_element) = value::child_by_field(element, field) else {
        return Ok(None);
    };
    let [red, green, blue, alpha] = value::read_color(field_element)?;
    Ok(Some(LinearRgba::new(red, green, blue, alpha)))
}

fn read_vec2_field(element: &Element, field: &str) -> Result<Option<Vec2>> {
    let Some(field_element) = value::child_by_field(element, field) else {
        return Ok(None);
    };
    Ok(Some(Vec2::from_array(value::read_vec2(field_element)?)))
}

fn read_bool_field(element: &Element, field: &str) -> Result<Option<bool>> {
    value::child_by_field(element, field)
        .map(value::read_bool)
        .transpose()
        .map_err(Into::into)
}

fn read_i32_field(element: &Element, field: &str) -> Result<Option<i32>> {
    value::child_by_field(element, field)
        .map(value::read_i32)
        .transpose()
        .map_err(Into::into)
}

fn read_i32_or_byte_field(element: &Element, field: &str) -> Result<Option<i32>> {
    let Some(field_element) = value::child_by_field(element, field) else {
        return Ok(None);
    };
    match value::semantic_type_id(field_element)? {
        types::INT => value::read_i32(field_element).map(Some).map_err(Into::into),
        types::UNSIGNED_INT => {
            let value = value::read_u32(field_element)?;
            let value = i32::try_from(value)
                .with_context(|| format!("field {field} value {value} exceeds i32"))?;
            Ok(Some(value))
        }
        types::CHAR | types::SIGNED_CHAR | types::AZ_S8 | types::UNSIGNED_CHAR => {
            Ok(Some(i32::from(read_byte_value(field_element, field)?)))
        }
        actual => bail!("field {field} has type {actual}, expected int or byte"),
    }
}

fn read_u32_field(element: &Element, field: &str) -> Result<Option<u32>> {
    value::child_by_field(element, field)
        .map(value::read_u32)
        .transpose()
        .map_err(Into::into)
}

fn read_u32_field_any(element: &Element, fields: &[&str]) -> Result<Option<u32>> {
    child_by_field_any(element, fields)
        .map(value::read_u32)
        .transpose()
        .map_err(Into::into)
}

fn read_u64_field(element: &Element, field: &str) -> Result<Option<u64>> {
    value::child_by_field(element, field)
        .map(value::read_u64)
        .transpose()
        .map_err(Into::into)
}

fn read_f32_field(element: &Element, field: &str) -> Result<Option<f32>> {
    value::child_by_field(element, field)
        .map(|field_element| read_f32_value(field_element, field))
        .transpose()
}

fn read_char_field(element: &Element, field: &str) -> Result<Option<u8>> {
    let Some(field_element) = value::child_by_field(element, field) else {
        return Ok(None);
    };
    Ok(Some(read_byte_value(field_element, field)?))
}

fn read_byte_value(element: &Element, field: &str) -> Result<u8> {
    match value::semantic_type_id(element)? {
        types::CHAR | types::SIGNED_CHAR | types::AZ_S8 => value::read_i8(element)
            .map(|value| u8::from_ne_bytes(value.to_ne_bytes()))
            .map_err(Into::into),
        types::UNSIGNED_CHAR => value::read_u8(element).map_err(Into::into),
        actual => bail!("field {field} has type {actual}, expected char-compatible byte"),
    }
}

/// Read a canvas numeric field, normalising whichever width it was authored at
/// to the `f32` a `LyShine` canvas stores.
///
/// Lumberyard editors have written the same field as `float`, `double`, `int`,
/// `unsigned int` and `AZ::u64` across versions, but the canvas format itself
/// only holds `f32`. Rust has no lossless conversion from any of the wider
/// types to `f32`, so the narrowing below is inherent to the format rather
/// than an oversight: integers past 2^24 shed low bits and doubles past
/// `f32::MAX` saturate to infinity — the same values the native runtime
/// loaded. Rejecting them instead would fail canvases the engine accepts.
// The four `as` casts are that documented narrowing; every alternative
// conversion is either lossless-only (and so does not compile for these types)
// or changes which canvases load.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn read_f32_value(element: &Element, field: &str) -> Result<f32> {
    match value::semantic_type_id(element)? {
        types::FLOAT => value::read_f32(element).map_err(Into::into),
        types::DOUBLE => value::read_f64(element)
            .map(|value| value as f32)
            .map_err(Into::into),
        types::INT => value::read_i32(element)
            .map(|value| value as f32)
            .map_err(Into::into),
        types::UNSIGNED_INT => value::read_u32(element)
            .map(|value| value as f32)
            .map_err(Into::into),
        types::AZ_U64 => value::read_u64(element)
            .map(|value| value as f32)
            .map_err(Into::into),
        actual => bail!("field {field} has type {actual}, expected numeric scalar"),
    }
}

fn require_sequence(element: &Element, expected: &'static str) -> Result<()> {
    value::require_container_shape(element, ContainerShape::Sequence, expected).map_err(Into::into)
}

fn find_child_by_semantic_type(element: &Element, expected: Uuid) -> Result<Option<&Element>> {
    for child in element.children() {
        if value::semantic_type_id(child)? == expected {
            return Ok(Some(child));
        }
    }
    Ok(None)
}

fn read_string_field<'a>(element: &'a Element, field: &str) -> Result<Option<&'a str>> {
    value::child_by_field(element, field)
        .map(value::read_string)
        .transpose()
        .map_err(Into::into)
}

fn read_non_empty_string_field(element: &Element, field: &str) -> Result<Option<String>> {
    Ok(read_string_field(element, field)?
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned))
}

fn engine_path_for_source(source_path: &str) -> String {
    normalize_source_path(source_path)
}

fn sprite_texture_engine_path(source_path: &str) -> String {
    // `.sprite` is the LyShine sidecar that points at a sibling
    // texture with the same stem. The canvas binary embeds the
    // texture path (`.dds`), not the sidecar — so the source-path
    // → engine-path mapping here swaps `.sprite` for `.dds` first.
    let source_path = normalize_source_path(source_path);
    let texture_source_path = source_path
        .strip_suffix(".sprite")
        .map_or_else(|| source_path.clone(), |base| format!("{base}.dds"));
    engine_path_for_source(&texture_source_path)
}

fn simple_ref_error(error: &SimpleAssetReferenceElementError) -> anyhow::Error {
    anyhow::anyhow!("{error}")
}

fn asset_value_error(error: &AssetValueError) -> anyhow::Error {
    anyhow::anyhow!("{error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_texture_path_uses_same_stem_texture_product() {
        assert_eq!(
            sprite_texture_engine_path("LyShineUI/Images/Common/GradientBox.sprite"),
            "lyshineui/images/common/gradientbox.dds"
        );
        assert_eq!(
            sprite_texture_engine_path("LyShineUI/Images/Common/Icon.dds"),
            "lyshineui/images/common/icon.dds"
        );
    }
}
