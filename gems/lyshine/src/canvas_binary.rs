//! UI canvas native binary format.

use std::future::Future;
use std::io::{Cursor, Read, Write};
use std::pin::Pin;

use az_framework::{
    ScriptComponent, ScriptDynamicClassArrayValue, ScriptDynamicClassValue, ScriptDynamicField,
    ScriptDynamicValue, ScriptProperty, ScriptPropertyGroup, ScriptPropertyKey,
    ScriptPropertyValue,
};
use bevy::asset::AsyncReadExt;
use bevy::asset::io::Reader;
use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::canvas::{
    UI_CANVAS_ASSET_MAGIC, UI_CANVAS_ASSET_MIN_VERSION, UI_CANVAS_ASSET_VERSION, UiBlendMode,
    UiButton, UiCanvas, UiCanvasAsset, UiCanvasAssetFormatError, UiCanvasFlags, UiChildOrder,
    UiComponentKind, UiElement, UiEntity, UiEntityId, UiFader, UiImage, UiImageFillCornerOrigin,
    UiImageFillEdgeOrigin, UiImageFillType, UiImageSpriteType, UiImageType, UiLayoutAxis,
    UiLayoutCell, UiLayoutGrid, UiMask, UiRect, UiScript, UiText, UiTransform2d,
};

pub fn write_ui_canvas_asset(
    asset: &UiCanvasAsset,
    mut writer: impl Write,
) -> Result<(), UiCanvasAssetFormatError> {
    writer.write_all(UI_CANVAS_ASSET_MAGIC)?;
    write_u32(&mut writer, UI_CANVAS_ASSET_VERSION)?;
    write_canvas(&mut writer, &asset.canvas)?;
    write_u32(
        &mut writer,
        checked_u32(asset.entities.len(), "UI entities")?,
    )?;
    for entity in &asset.entities {
        write_entity(&mut writer, entity)?;
    }
    Ok(())
}

pub fn read_ui_canvas_asset(bytes: &[u8]) -> Result<UiCanvasAsset, UiCanvasAssetFormatError> {
    read_ui_canvas_asset_from_reader(Cursor::new(bytes))
}

pub fn read_ui_canvas_asset_from_reader(
    mut reader: impl Read,
) -> Result<UiCanvasAsset, UiCanvasAssetFormatError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != UI_CANVAS_ASSET_MAGIC {
        return Err(UiCanvasAssetFormatError::BadMagic { found: magic });
    }
    read_asset_after_magic(reader)
}

pub async fn read_ui_canvas_asset_from_bevy_reader(
    reader: &mut dyn Reader,
) -> Result<UiCanvasAsset, UiCanvasAssetFormatError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic).await?;
    if &magic != UI_CANVAS_ASSET_MAGIC {
        return Err(UiCanvasAssetFormatError::BadMagic { found: magic });
    }
    read_asset_from_bevy_reader_after_magic(reader).await
}

fn read_asset_after_magic(
    mut reader: impl Read,
) -> Result<UiCanvasAsset, UiCanvasAssetFormatError> {
    let version = read_u32(&mut reader)?;
    if !(UI_CANVAS_ASSET_MIN_VERSION..=UI_CANVAS_ASSET_VERSION).contains(&version) {
        return Err(UiCanvasAssetFormatError::UnsupportedVersion {
            version,
            expected: UI_CANVAS_ASSET_VERSION,
        });
    }

    let canvas = read_canvas(&mut reader)?;
    let entity_count = read_u32(&mut reader)? as usize;
    let mut entities = Vec::with_capacity(entity_count);
    for _ in 0..entity_count {
        entities.push(read_entity(&mut reader, version)?);
    }
    Ok(UiCanvasAsset {
        version,
        canvas,
        entities,
    })
}

async fn read_asset_from_bevy_reader_after_magic(
    reader: &mut dyn Reader,
) -> Result<UiCanvasAsset, UiCanvasAssetFormatError> {
    let version = read_async_u32(reader).await?;
    if !(UI_CANVAS_ASSET_MIN_VERSION..=UI_CANVAS_ASSET_VERSION).contains(&version) {
        return Err(UiCanvasAssetFormatError::UnsupportedVersion {
            version,
            expected: UI_CANVAS_ASSET_VERSION,
        });
    }

    let canvas = read_async_canvas(reader).await?;
    let entity_count = read_async_u32(reader).await? as usize;
    let mut entities = Vec::with_capacity(entity_count);
    for _ in 0..entity_count {
        entities.push(read_async_entity(reader, version).await?);
    }
    Ok(UiCanvasAsset {
        version,
        canvas,
        entities,
    })
}

fn write_canvas(
    writer: &mut impl Write,
    canvas: &UiCanvas,
) -> Result<(), UiCanvasAssetFormatError> {
    write_u64(writer, canvas.unique_id)?;
    write_entity_id(writer, canvas.root_entity)?;
    write_entity_id(writer, canvas.first_hover_entity)?;
    write_entity_id(writer, canvas.tooltip_display_entity)?;
    write_u32(writer, canvas.last_element_id)?;
    write_vec2(writer, canvas.size)?;
    write_i32(writer, canvas.draw_order)?;
    write_canvas_flags(writer, canvas.flags)?;
    write_option_string(writer, canvas.render_target_name.as_deref())?;
    write_strings(writer, &canvas.texture_atlases)
}

fn read_canvas(reader: &mut impl Read) -> Result<UiCanvas, UiCanvasAssetFormatError> {
    Ok(UiCanvas {
        unique_id: read_u64(reader)?,
        root_entity: read_entity_id(reader)?,
        first_hover_entity: read_entity_id(reader)?,
        tooltip_display_entity: read_entity_id(reader)?,
        last_element_id: read_u32(reader)?,
        size: read_vec2(reader)?,
        draw_order: read_i32(reader)?,
        flags: read_canvas_flags(reader)?,
        render_target_name: read_option_string(reader)?,
        texture_atlases: read_strings(reader)?,
    })
}

async fn read_async_canvas(reader: &mut dyn Reader) -> Result<UiCanvas, UiCanvasAssetFormatError> {
    Ok(UiCanvas {
        unique_id: read_async_u64(reader).await?,
        root_entity: read_async_entity_id(reader).await?,
        first_hover_entity: read_async_entity_id(reader).await?,
        tooltip_display_entity: read_async_entity_id(reader).await?,
        last_element_id: read_async_u32(reader).await?,
        size: read_async_vec2(reader).await?,
        draw_order: read_async_i32(reader).await?,
        flags: read_async_canvas_flags(reader).await?,
        render_target_name: read_async_option_string(reader).await?,
        texture_atlases: read_async_strings(reader).await?,
    })
}

fn write_entity(
    writer: &mut impl Write,
    entity: &UiEntity,
) -> Result<(), UiCanvasAssetFormatError> {
    write_entity_id(writer, entity.entity_id)?;
    write_option_string(writer, entity.name.as_deref())?;
    write_bool(writer, entity.dependency_ready)?;
    write_bool(writer, entity.runtime_active)?;
    write_option_transform(writer, entity.transform.as_ref())?;
    write_option_element(writer, entity.element.as_ref())?;
    write_option_image(writer, entity.image.as_ref())?;
    write_option_text(writer, entity.text.as_ref())?;
    write_option_button(writer, entity.button.as_ref())?;
    write_option_fader(writer, entity.fader.as_ref())?;
    write_option_mask(writer, entity.mask.as_ref())?;
    write_option_layout_axis(writer, entity.layout_row.as_ref())?;
    write_option_layout_axis(writer, entity.layout_column.as_ref())?;
    write_option_layout_grid(writer, entity.layout_grid.as_ref())?;
    write_option_layout_cell(writer, entity.layout_cell.as_ref())?;
    write_option_script(writer, entity.script.as_ref())?;
    write_component_kinds(writer, &entity.components)
}

fn read_entity(reader: &mut impl Read, version: u32) -> Result<UiEntity, UiCanvasAssetFormatError> {
    let mut entity = UiEntity {
        entity_id: read_entity_id(reader)?,
        name: read_option_string(reader)?,
        dependency_ready: read_bool(reader)?,
        runtime_active: read_bool(reader)?,
        transform: read_option_transform(reader)?,
        element: read_option_element(reader)?,
        image: read_option_image(reader)?,
        text: read_option_text(reader)?,
        button: read_option_button(reader)?,
        fader: read_option_fader(reader)?,
        mask: None,
        layout_row: None,
        layout_column: None,
        layout_grid: None,
        layout_cell: None,
        script: None,
        components: Vec::new(),
    };
    if version >= 2 {
        entity.mask = read_option_mask(reader)?;
        entity.layout_row = read_option_layout_axis(reader)?;
        entity.layout_column = read_option_layout_axis(reader)?;
        entity.layout_grid = read_option_layout_grid(reader)?;
        entity.layout_cell = read_option_layout_cell(reader)?;
        entity.script = read_option_script(reader)?;
    }
    entity.components = read_component_kinds(reader)?;
    Ok(entity)
}

async fn read_async_entity(
    reader: &mut dyn Reader,
    version: u32,
) -> Result<UiEntity, UiCanvasAssetFormatError> {
    let mut entity = UiEntity {
        entity_id: read_async_entity_id(reader).await?,
        name: read_async_option_string(reader).await?,
        dependency_ready: read_async_bool(reader).await?,
        runtime_active: read_async_bool(reader).await?,
        transform: read_async_option_transform(reader).await?,
        element: read_async_option_element(reader).await?,
        image: read_async_option_image(reader).await?,
        text: read_async_option_text(reader).await?,
        button: read_async_option_button(reader).await?,
        fader: read_async_option_fader(reader).await?,
        mask: None,
        layout_row: None,
        layout_column: None,
        layout_grid: None,
        layout_cell: None,
        script: None,
        components: Vec::new(),
    };
    if version >= 2 {
        entity.mask = read_async_option_mask(reader).await?;
        entity.layout_row = read_async_option_layout_axis(reader).await?;
        entity.layout_column = read_async_option_layout_axis(reader).await?;
        entity.layout_grid = read_async_option_layout_grid(reader).await?;
        entity.layout_cell = read_async_option_layout_cell(reader).await?;
        entity.script = read_async_option_script(reader).await?;
    }
    entity.components = read_async_component_kinds(reader).await?;
    Ok(entity)
}

fn write_canvas_flags(writer: &mut impl Write, flags: UiCanvasFlags) -> Result<(), std::io::Error> {
    write_bool(writer, flags.snap_enabled)?;
    write_bool(writer, flags.pixel_aligned)?;
    write_bool(writer, flags.render_to_texture)?;
    write_bool(writer, flags.transform_update_optimize_enabled)?;
    write_bool(writer, flags.optimize_for_frequent_updates)?;
    write_bool(writer, flags.position_input_supported)?;
    write_bool(writer, flags.navigation_supported)?;
    write_bool(writer, flags.always_allows_hover)?;
    write_bool(writer, flags.ignore_scroll_hover)?;
    write_bool(writer, flags.enter_handling_disabled)?;
    write_bool(writer, flags.guides_locked)
}

fn read_canvas_flags(reader: &mut impl Read) -> Result<UiCanvasFlags, std::io::Error> {
    Ok(UiCanvasFlags {
        snap_enabled: read_bool(reader)?,
        pixel_aligned: read_bool(reader)?,
        render_to_texture: read_bool(reader)?,
        transform_update_optimize_enabled: read_bool(reader)?,
        optimize_for_frequent_updates: read_bool(reader)?,
        position_input_supported: read_bool(reader)?,
        navigation_supported: read_bool(reader)?,
        always_allows_hover: read_bool(reader)?,
        ignore_scroll_hover: read_bool(reader)?,
        enter_handling_disabled: read_bool(reader)?,
        guides_locked: read_bool(reader)?,
    })
}

async fn read_async_canvas_flags(reader: &mut dyn Reader) -> Result<UiCanvasFlags, std::io::Error> {
    Ok(UiCanvasFlags {
        snap_enabled: read_async_bool(reader).await?,
        pixel_aligned: read_async_bool(reader).await?,
        render_to_texture: read_async_bool(reader).await?,
        transform_update_optimize_enabled: read_async_bool(reader).await?,
        optimize_for_frequent_updates: read_async_bool(reader).await?,
        position_input_supported: read_async_bool(reader).await?,
        navigation_supported: read_async_bool(reader).await?,
        always_allows_hover: read_async_bool(reader).await?,
        ignore_scroll_hover: read_async_bool(reader).await?,
        enter_handling_disabled: read_async_bool(reader).await?,
        guides_locked: read_async_bool(reader).await?,
    })
}

fn write_option_transform(
    writer: &mut impl Write,
    value: Option<&UiTransform2d>,
) -> Result<(), UiCanvasAssetFormatError> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_rect(writer, value.anchors)?;
        write_rect(writer, value.offsets)?;
        write_vec2(writer, value.pivot)?;
        write_f32(writer, value.rotation)?;
        write_vec2(writer, value.scale)?;
        write_bool(writer, value.scale_to_device)?;
        write_bool(writer, value.compute_transform_when_hidden)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_transform(
    reader: &mut impl Read,
) -> Result<Option<UiTransform2d>, UiCanvasAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(UiTransform2d {
        anchors: read_rect(reader)?,
        offsets: read_rect(reader)?,
        pivot: read_vec2(reader)?,
        rotation: read_f32(reader)?,
        scale: read_vec2(reader)?,
        scale_to_device: read_bool(reader)?,
        compute_transform_when_hidden: read_bool(reader)?,
    }))
}

async fn read_async_option_transform(
    reader: &mut dyn Reader,
) -> Result<Option<UiTransform2d>, UiCanvasAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(UiTransform2d {
        anchors: read_async_rect(reader).await?,
        offsets: read_async_rect(reader).await?,
        pivot: read_async_vec2(reader).await?,
        rotation: read_async_f32(reader).await?,
        scale: read_async_vec2(reader).await?,
        scale_to_device: read_async_bool(reader).await?,
        compute_transform_when_hidden: read_async_bool(reader).await?,
    }))
}

fn write_option_element(
    writer: &mut impl Write,
    value: Option<&UiElement>,
) -> Result<(), UiCanvasAssetFormatError> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_u32(writer, value.local_id)?;
        write_bool(writer, value.enabled)?;
        write_bool(writer, value.visible_in_editor)?;
        write_bool(writer, value.selectable_in_editor)?;
        write_bool(writer, value.selected_in_editor)?;
        write_bool(writer, value.expanded_in_editor)?;
        write_child_order(writer, &value.child_order)?;
        write_bool(writer, value.children_render_sortable)?;
        write_i32(writer, value.render_priority)?;
        write_bool(writer, value.multithread_children)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_element(
    reader: &mut impl Read,
) -> Result<Option<UiElement>, UiCanvasAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(UiElement {
        local_id: read_u32(reader)?,
        enabled: read_bool(reader)?,
        visible_in_editor: read_bool(reader)?,
        selectable_in_editor: read_bool(reader)?,
        selected_in_editor: read_bool(reader)?,
        expanded_in_editor: read_bool(reader)?,
        child_order: read_child_order(reader)?,
        children_render_sortable: read_bool(reader)?,
        render_priority: read_i32(reader)?,
        multithread_children: read_bool(reader)?,
    }))
}

async fn read_async_option_element(
    reader: &mut dyn Reader,
) -> Result<Option<UiElement>, UiCanvasAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(UiElement {
        local_id: read_async_u32(reader).await?,
        enabled: read_async_bool(reader).await?,
        visible_in_editor: read_async_bool(reader).await?,
        selectable_in_editor: read_async_bool(reader).await?,
        selected_in_editor: read_async_bool(reader).await?,
        expanded_in_editor: read_async_bool(reader).await?,
        child_order: read_async_child_order(reader).await?,
        children_render_sortable: read_async_bool(reader).await?,
        render_priority: read_async_i32(reader).await?,
        multithread_children: read_async_bool(reader).await?,
    }))
}

fn write_option_image(
    writer: &mut impl Write,
    value: Option<&UiImage>,
) -> Result<(), UiCanvasAssetFormatError> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_u8(writer, value.sprite_type.as_u8())?;
        write_option_string(writer, value.sprite_path.as_deref())?;
        write_u32(writer, value.sprite_index)?;
        write_option_string(writer, value.render_target_name.as_deref())?;
        write_bool(writer, value.render_target_srgb)?;
        write_color(writer, value.color)?;
        write_f32(writer, value.alpha)?;
        write_u8(writer, value.image_type.as_u8())?;
        write_bool(writer, value.fill_center)?;
        write_bool(writer, value.stretch_sliced)?;
        write_i32(writer, value.blend_mode.as_i32())?;
        write_u8(writer, value.fill_type.as_u8())?;
        write_f32(writer, value.fill_amount)?;
        write_f32(writer, value.fill_start_angle)?;
        write_u8(writer, value.fill_corner_origin.as_u8())?;
        write_u8(writer, value.fill_edge_origin.as_u8())?;
        write_bool(writer, value.fill_clockwise)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_image(reader: &mut impl Read) -> Result<Option<UiImage>, UiCanvasAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(UiImage {
        sprite_type: read_sprite_type(reader)?,
        sprite_path: read_option_string(reader)?,
        sprite_index: read_u32(reader)?,
        render_target_name: read_option_string(reader)?,
        render_target_srgb: read_bool(reader)?,
        color: read_color(reader)?,
        alpha: read_f32(reader)?,
        image_type: read_image_type(reader)?,
        fill_center: read_bool(reader)?,
        stretch_sliced: read_bool(reader)?,
        blend_mode: read_blend_mode(reader)?,
        fill_type: read_fill_type(reader)?,
        fill_amount: read_f32(reader)?,
        fill_start_angle: read_f32(reader)?,
        fill_corner_origin: read_fill_corner_origin(reader)?,
        fill_edge_origin: read_fill_edge_origin(reader)?,
        fill_clockwise: read_bool(reader)?,
    }))
}

async fn read_async_option_image(
    reader: &mut dyn Reader,
) -> Result<Option<UiImage>, UiCanvasAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(UiImage {
        sprite_type: read_async_sprite_type(reader).await?,
        sprite_path: read_async_option_string(reader).await?,
        sprite_index: read_async_u32(reader).await?,
        render_target_name: read_async_option_string(reader).await?,
        render_target_srgb: read_async_bool(reader).await?,
        color: read_async_color(reader).await?,
        alpha: read_async_f32(reader).await?,
        image_type: read_async_image_type(reader).await?,
        fill_center: read_async_bool(reader).await?,
        stretch_sliced: read_async_bool(reader).await?,
        blend_mode: read_async_blend_mode(reader).await?,
        fill_type: read_async_fill_type(reader).await?,
        fill_amount: read_async_f32(reader).await?,
        fill_start_angle: read_async_f32(reader).await?,
        fill_corner_origin: read_async_fill_corner_origin(reader).await?,
        fill_edge_origin: read_async_fill_edge_origin(reader).await?,
        fill_clockwise: read_async_bool(reader).await?,
    }))
}

fn read_sprite_type(reader: &mut impl Read) -> Result<UiImageSpriteType, UiCanvasAssetFormatError> {
    parse_sprite_type(read_u8(reader)?)
}

async fn read_async_sprite_type(
    reader: &mut dyn Reader,
) -> Result<UiImageSpriteType, UiCanvasAssetFormatError> {
    parse_sprite_type(read_async_u8(reader).await?)
}

fn parse_sprite_type(value: u8) -> Result<UiImageSpriteType, UiCanvasAssetFormatError> {
    UiImageSpriteType::from_u8(value).ok_or_else(|| UiCanvasAssetFormatError::InvalidEnum {
        field: "SpriteType",
        value: value.into(),
    })
}

fn read_image_type(reader: &mut impl Read) -> Result<UiImageType, UiCanvasAssetFormatError> {
    parse_image_type(read_u8(reader)?)
}

async fn read_async_image_type(
    reader: &mut dyn Reader,
) -> Result<UiImageType, UiCanvasAssetFormatError> {
    parse_image_type(read_async_u8(reader).await?)
}

fn parse_image_type(value: u8) -> Result<UiImageType, UiCanvasAssetFormatError> {
    UiImageType::from_u8(value).ok_or_else(|| UiCanvasAssetFormatError::InvalidEnum {
        field: "ImageType",
        value: value.into(),
    })
}

fn read_blend_mode(reader: &mut impl Read) -> Result<UiBlendMode, UiCanvasAssetFormatError> {
    parse_blend_mode(read_i32(reader)?)
}

async fn read_async_blend_mode(
    reader: &mut dyn Reader,
) -> Result<UiBlendMode, UiCanvasAssetFormatError> {
    parse_blend_mode(read_async_i32(reader).await?)
}

fn parse_blend_mode(value: i32) -> Result<UiBlendMode, UiCanvasAssetFormatError> {
    UiBlendMode::from_i32(value).ok_or(UiCanvasAssetFormatError::InvalidEnum {
        field: "BlendMode",
        value,
    })
}

fn read_fill_type(reader: &mut impl Read) -> Result<UiImageFillType, UiCanvasAssetFormatError> {
    parse_fill_type(read_u8(reader)?)
}

async fn read_async_fill_type(
    reader: &mut dyn Reader,
) -> Result<UiImageFillType, UiCanvasAssetFormatError> {
    parse_fill_type(read_async_u8(reader).await?)
}

fn parse_fill_type(value: u8) -> Result<UiImageFillType, UiCanvasAssetFormatError> {
    UiImageFillType::from_u8(value).ok_or_else(|| UiCanvasAssetFormatError::InvalidEnum {
        field: "FillType",
        value: value.into(),
    })
}

fn read_fill_corner_origin(
    reader: &mut impl Read,
) -> Result<UiImageFillCornerOrigin, UiCanvasAssetFormatError> {
    parse_fill_corner_origin(read_u8(reader)?)
}

async fn read_async_fill_corner_origin(
    reader: &mut dyn Reader,
) -> Result<UiImageFillCornerOrigin, UiCanvasAssetFormatError> {
    parse_fill_corner_origin(read_async_u8(reader).await?)
}

fn parse_fill_corner_origin(
    value: u8,
) -> Result<UiImageFillCornerOrigin, UiCanvasAssetFormatError> {
    UiImageFillCornerOrigin::from_u8(value).ok_or_else(|| UiCanvasAssetFormatError::InvalidEnum {
        field: "FillCornerOrigin",
        value: value.into(),
    })
}

fn read_fill_edge_origin(
    reader: &mut impl Read,
) -> Result<UiImageFillEdgeOrigin, UiCanvasAssetFormatError> {
    parse_fill_edge_origin(read_u8(reader)?)
}

async fn read_async_fill_edge_origin(
    reader: &mut dyn Reader,
) -> Result<UiImageFillEdgeOrigin, UiCanvasAssetFormatError> {
    parse_fill_edge_origin(read_async_u8(reader).await?)
}

fn parse_fill_edge_origin(value: u8) -> Result<UiImageFillEdgeOrigin, UiCanvasAssetFormatError> {
    UiImageFillEdgeOrigin::from_u8(value).ok_or_else(|| UiCanvasAssetFormatError::InvalidEnum {
        field: "FillEdgeOrigin",
        value: value.into(),
    })
}

fn write_option_text(
    writer: &mut impl Write,
    value: Option<&UiText>,
) -> Result<(), UiCanvasAssetFormatError> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_string(writer, &value.text)?;
        write_bool(writer, value.markup_enabled)?;
        write_bool(writer, value.images_enabled)?;
        write_bool(writer, value.update_on_input_change)?;
        write_color(writer, value.color)?;
        write_f32(writer, value.alpha)?;
        write_option_string(writer, value.font_path.as_deref())?;
        write_u32(writer, value.font_effect_index)?;
        write_f32(writer, value.font_size)?;
        write_f32(writer, value.character_spacing)?;
        write_f32(writer, value.line_spacing)?;
        write_i32(writer, value.horizontal_alignment)?;
        write_i32(writer, value.vertical_alignment)?;
        write_i32(writer, value.wrap_text_setting)?;
        write_i32(writer, value.overflow_mode)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_text(reader: &mut impl Read) -> Result<Option<UiText>, UiCanvasAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(UiText {
        text: read_string(reader)?,
        markup_enabled: read_bool(reader)?,
        images_enabled: read_bool(reader)?,
        update_on_input_change: read_bool(reader)?,
        color: read_color(reader)?,
        alpha: read_f32(reader)?,
        font_path: read_option_string(reader)?,
        font_effect_index: read_u32(reader)?,
        font_size: read_f32(reader)?,
        character_spacing: read_f32(reader)?,
        line_spacing: read_f32(reader)?,
        horizontal_alignment: read_i32(reader)?,
        vertical_alignment: read_i32(reader)?,
        wrap_text_setting: read_i32(reader)?,
        overflow_mode: read_i32(reader)?,
    }))
}

async fn read_async_option_text(
    reader: &mut dyn Reader,
) -> Result<Option<UiText>, UiCanvasAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(UiText {
        text: read_async_string(reader).await?,
        markup_enabled: read_async_bool(reader).await?,
        images_enabled: read_async_bool(reader).await?,
        update_on_input_change: read_async_bool(reader).await?,
        color: read_async_color(reader).await?,
        alpha: read_async_f32(reader).await?,
        font_path: read_async_option_string(reader).await?,
        font_effect_index: read_async_u32(reader).await?,
        font_size: read_async_f32(reader).await?,
        character_spacing: read_async_f32(reader).await?,
        line_spacing: read_async_f32(reader).await?,
        horizontal_alignment: read_async_i32(reader).await?,
        vertical_alignment: read_async_i32(reader).await?,
        wrap_text_setting: read_async_i32(reader).await?,
        overflow_mode: read_async_i32(reader).await?,
    }))
}

fn write_option_button(
    writer: &mut impl Write,
    value: Option<&UiButton>,
) -> Result<(), UiCanvasAssetFormatError> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_option_string(writer, value.hover_start_action_name.as_deref())?;
        write_option_string(writer, value.hover_end_action_name.as_deref())?;
        write_option_string(writer, value.pressed_action_name.as_deref())?;
        write_option_string(writer, value.released_action_name.as_deref())?;
        write_option_string(writer, value.action_name.as_deref())?;
        write_option_string(writer, value.action_name_right.as_deref())?;
        write_option_string(writer, value.action_name_pressed_right.as_deref())?;
        write_bool(writer, value.use_click_behavior)?;
        write_f32(writer, value.click_sq_tolerance)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_button(
    reader: &mut impl Read,
) -> Result<Option<UiButton>, UiCanvasAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(UiButton {
        hover_start_action_name: read_option_string(reader)?,
        hover_end_action_name: read_option_string(reader)?,
        pressed_action_name: read_option_string(reader)?,
        released_action_name: read_option_string(reader)?,
        action_name: read_option_string(reader)?,
        action_name_right: read_option_string(reader)?,
        action_name_pressed_right: read_option_string(reader)?,
        use_click_behavior: read_bool(reader)?,
        click_sq_tolerance: read_f32(reader)?,
    }))
}

async fn read_async_option_button(
    reader: &mut dyn Reader,
) -> Result<Option<UiButton>, UiCanvasAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(UiButton {
        hover_start_action_name: read_async_option_string(reader).await?,
        hover_end_action_name: read_async_option_string(reader).await?,
        pressed_action_name: read_async_option_string(reader).await?,
        released_action_name: read_async_option_string(reader).await?,
        action_name: read_async_option_string(reader).await?,
        action_name_right: read_async_option_string(reader).await?,
        action_name_pressed_right: read_async_option_string(reader).await?,
        use_click_behavior: read_async_bool(reader).await?,
        click_sq_tolerance: read_async_f32(reader).await?,
    }))
}

fn write_option_fader(
    writer: &mut impl Write,
    value: Option<&UiFader>,
) -> Result<(), std::io::Error> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_f32(writer, value.fade)?;
        write_bool(writer, value.use_render_to_texture)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_fader(reader: &mut impl Read) -> Result<Option<UiFader>, std::io::Error> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(UiFader {
        fade: read_f32(reader)?,
        use_render_to_texture: read_bool(reader)?,
    }))
}

async fn read_async_option_fader(
    reader: &mut dyn Reader,
) -> Result<Option<UiFader>, std::io::Error> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(UiFader {
        fade: read_async_f32(reader).await?,
        use_render_to_texture: read_async_bool(reader).await?,
    }))
}

fn write_option_mask(
    writer: &mut impl Write,
    value: Option<&UiMask>,
) -> Result<(), UiCanvasAssetFormatError> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_bool(writer, value.enable_masking)?;
        write_bool(writer, value.mask_interaction)?;
        write_entity_id(writer, value.child_mask_element)?;
        write_bool(writer, value.use_render_to_texture)?;
        write_bool(writer, value.draw_behind)?;
        write_bool(writer, value.draw_in_front)?;
        write_bool(writer, value.use_alpha_test)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_mask(reader: &mut impl Read) -> Result<Option<UiMask>, UiCanvasAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(UiMask {
        enable_masking: read_bool(reader)?,
        mask_interaction: read_bool(reader)?,
        child_mask_element: read_entity_id(reader)?,
        use_render_to_texture: read_bool(reader)?,
        draw_behind: read_bool(reader)?,
        draw_in_front: read_bool(reader)?,
        use_alpha_test: read_bool(reader)?,
    }))
}

async fn read_async_option_mask(
    reader: &mut dyn Reader,
) -> Result<Option<UiMask>, UiCanvasAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(UiMask {
        enable_masking: read_async_bool(reader).await?,
        mask_interaction: read_async_bool(reader).await?,
        child_mask_element: read_async_entity_id(reader).await?,
        use_render_to_texture: read_async_bool(reader).await?,
        draw_behind: read_async_bool(reader).await?,
        draw_in_front: read_async_bool(reader).await?,
        use_alpha_test: read_async_bool(reader).await?,
    }))
}

fn write_option_layout_axis(
    writer: &mut impl Write,
    value: Option<&UiLayoutAxis>,
) -> Result<(), UiCanvasAssetFormatError> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_rect(writer, value.padding)?;
        write_f32(writer, value.spacing)?;
        write_i32(writer, value.order)?;
        write_i32(writer, value.child_h_alignment)?;
        write_i32(writer, value.child_v_alignment)?;
        write_bool(writer, value.ignore_default_layout_cells)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_layout_axis(
    reader: &mut impl Read,
) -> Result<Option<UiLayoutAxis>, UiCanvasAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(UiLayoutAxis {
        padding: read_rect(reader)?,
        spacing: read_f32(reader)?,
        order: read_i32(reader)?,
        child_h_alignment: read_i32(reader)?,
        child_v_alignment: read_i32(reader)?,
        ignore_default_layout_cells: read_bool(reader)?,
    }))
}

async fn read_async_option_layout_axis(
    reader: &mut dyn Reader,
) -> Result<Option<UiLayoutAxis>, UiCanvasAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(UiLayoutAxis {
        padding: read_async_rect(reader).await?,
        spacing: read_async_f32(reader).await?,
        order: read_async_i32(reader).await?,
        child_h_alignment: read_async_i32(reader).await?,
        child_v_alignment: read_async_i32(reader).await?,
        ignore_default_layout_cells: read_async_bool(reader).await?,
    }))
}

fn write_option_layout_grid(
    writer: &mut impl Write,
    value: Option<&UiLayoutGrid>,
) -> Result<(), UiCanvasAssetFormatError> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_rect(writer, value.padding)?;
        write_vec2(writer, value.spacing)?;
        write_vec2(writer, value.cell_size)?;
        write_i32(writer, value.horizontal_order)?;
        write_i32(writer, value.vertical_order)?;
        write_i32(writer, value.starting_with)?;
        write_i32(writer, value.child_h_alignment)?;
        write_i32(writer, value.child_v_alignment)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_layout_grid(
    reader: &mut impl Read,
) -> Result<Option<UiLayoutGrid>, UiCanvasAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(UiLayoutGrid {
        padding: read_rect(reader)?,
        spacing: read_vec2(reader)?,
        cell_size: read_vec2(reader)?,
        horizontal_order: read_i32(reader)?,
        vertical_order: read_i32(reader)?,
        starting_with: read_i32(reader)?,
        child_h_alignment: read_i32(reader)?,
        child_v_alignment: read_i32(reader)?,
    }))
}

async fn read_async_option_layout_grid(
    reader: &mut dyn Reader,
) -> Result<Option<UiLayoutGrid>, UiCanvasAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(UiLayoutGrid {
        padding: read_async_rect(reader).await?,
        spacing: read_async_vec2(reader).await?,
        cell_size: read_async_vec2(reader).await?,
        horizontal_order: read_async_i32(reader).await?,
        vertical_order: read_async_i32(reader).await?,
        starting_with: read_async_i32(reader).await?,
        child_h_alignment: read_async_i32(reader).await?,
        child_v_alignment: read_async_i32(reader).await?,
    }))
}

fn write_option_layout_cell(
    writer: &mut impl Write,
    value: Option<&UiLayoutCell>,
) -> Result<(), UiCanvasAssetFormatError> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_bool(writer, value.min_width_overridden)?;
        write_f32(writer, value.min_width)?;
        write_bool(writer, value.min_height_overridden)?;
        write_f32(writer, value.min_height)?;
        write_bool(writer, value.target_width_overridden)?;
        write_f32(writer, value.target_width)?;
        write_bool(writer, value.target_height_overridden)?;
        write_f32(writer, value.target_height)?;
        write_bool(writer, value.max_width_overridden)?;
        write_f32(writer, value.max_width)?;
        write_bool(writer, value.max_height_overridden)?;
        write_f32(writer, value.max_height)?;
        write_bool(writer, value.extra_width_ratio_overridden)?;
        write_f32(writer, value.extra_width_ratio)?;
        write_bool(writer, value.extra_height_ratio_overridden)?;
        write_f32(writer, value.extra_height_ratio)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_layout_cell(
    reader: &mut impl Read,
) -> Result<Option<UiLayoutCell>, UiCanvasAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(UiLayoutCell {
        min_width_overridden: read_bool(reader)?,
        min_width: read_f32(reader)?,
        min_height_overridden: read_bool(reader)?,
        min_height: read_f32(reader)?,
        target_width_overridden: read_bool(reader)?,
        target_width: read_f32(reader)?,
        target_height_overridden: read_bool(reader)?,
        target_height: read_f32(reader)?,
        max_width_overridden: read_bool(reader)?,
        max_width: read_f32(reader)?,
        max_height_overridden: read_bool(reader)?,
        max_height: read_f32(reader)?,
        extra_width_ratio_overridden: read_bool(reader)?,
        extra_width_ratio: read_f32(reader)?,
        extra_height_ratio_overridden: read_bool(reader)?,
        extra_height_ratio: read_f32(reader)?,
    }))
}

async fn read_async_option_layout_cell(
    reader: &mut dyn Reader,
) -> Result<Option<UiLayoutCell>, UiCanvasAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(UiLayoutCell {
        min_width_overridden: read_async_bool(reader).await?,
        min_width: read_async_f32(reader).await?,
        min_height_overridden: read_async_bool(reader).await?,
        min_height: read_async_f32(reader).await?,
        target_width_overridden: read_async_bool(reader).await?,
        target_width: read_async_f32(reader).await?,
        target_height_overridden: read_async_bool(reader).await?,
        target_height: read_async_f32(reader).await?,
        max_width_overridden: read_async_bool(reader).await?,
        max_width: read_async_f32(reader).await?,
        max_height_overridden: read_async_bool(reader).await?,
        max_height: read_async_f32(reader).await?,
        extra_width_ratio_overridden: read_async_bool(reader).await?,
        extra_width_ratio: read_async_f32(reader).await?,
        extra_height_ratio_overridden: read_async_bool(reader).await?,
        extra_height_ratio: read_async_f32(reader).await?,
    }))
}

fn write_option_script(
    writer: &mut impl Write,
    value: Option<&UiScript>,
) -> Result<(), UiCanvasAssetFormatError> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_u32(writer, value.context_id)?;
        write_script_property_group(writer, &value.properties)?;
        write_option_string(writer, value.script.as_deref())?;
        write_bool(writer, value.run_on_server)?;
        write_bool(writer, value.run_on_client)?;
        write_bool(writer, value.net_bindable.is_net_sync_enabled)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_script(
    reader: &mut impl Read,
) -> Result<Option<UiScript>, UiCanvasAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(ScriptComponent {
        az_component: az_core::component::Component::default(),
        context_id: read_u32(reader)?,
        properties: read_script_property_group(reader)?,
        name: String::new(),
        id: None,
        script: read_option_string(reader)?,
        run_on_server: read_bool(reader)?,
        run_on_client: read_bool(reader)?,
        net_bindable: az_framework::NetBindable {
            is_net_sync_enabled: read_bool(reader)?,
        },
    }))
}

async fn read_async_option_script(
    reader: &mut dyn Reader,
) -> Result<Option<UiScript>, UiCanvasAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(ScriptComponent {
        az_component: az_core::component::Component::default(),
        context_id: read_async_u32(reader).await?,
        properties: read_async_script_property_group(reader).await?,
        name: String::new(),
        id: None,
        script: read_async_option_string(reader).await?,
        run_on_server: read_async_bool(reader).await?,
        run_on_client: read_async_bool(reader).await?,
        net_bindable: az_framework::NetBindable {
            is_net_sync_enabled: read_async_bool(reader).await?,
        },
    }))
}

fn write_script_property_group(
    writer: &mut impl Write,
    value: &ScriptPropertyGroup,
) -> Result<(), UiCanvasAssetFormatError> {
    write_string(writer, &value.name)?;
    write_option_string(writer, value.id.as_deref())?;
    write_u32(
        writer,
        checked_u32(value.properties.len(), "script properties")?,
    )?;
    for property in &value.properties {
        write_script_property(writer, property)?;
    }
    write_u32(
        writer,
        checked_u32(value.groups.len(), "script property groups")?,
    )?;
    for group in &value.groups {
        write_script_property_group(writer, group)?;
    }
    Ok(())
}

fn read_script_property_group(
    reader: &mut impl Read,
) -> Result<ScriptPropertyGroup, UiCanvasAssetFormatError> {
    let name = read_string(reader)?;
    let id = read_option_string(reader)?;
    let property_count = read_u32(reader)? as usize;
    let mut properties = Vec::with_capacity(property_count);
    for _ in 0..property_count {
        properties.push(read_script_property(reader)?);
    }
    let group_count = read_u32(reader)? as usize;
    let mut groups = Vec::with_capacity(group_count);
    for _ in 0..group_count {
        groups.push(read_script_property_group(reader)?);
    }
    Ok(ScriptPropertyGroup {
        name,
        id,
        properties,
        groups,
    })
}

async fn read_async_script_property_group(
    reader: &mut dyn Reader,
) -> Result<ScriptPropertyGroup, UiCanvasAssetFormatError> {
    let name = read_async_string(reader).await?;
    let id = read_async_option_string(reader).await?;
    let property_count = read_async_u32(reader).await? as usize;
    let mut properties = Vec::with_capacity(property_count);
    for _ in 0..property_count {
        properties.push(read_async_script_property(reader).await?);
    }
    let group_count = read_async_u32(reader).await? as usize;
    let mut groups = Vec::with_capacity(group_count);
    for _ in 0..group_count {
        groups.push(Box::pin(read_async_script_property_group(reader)).await?);
    }
    Ok(ScriptPropertyGroup {
        name,
        id,
        properties,
        groups,
    })
}

fn write_script_property(
    writer: &mut impl Write,
    value: &ScriptProperty,
) -> Result<(), UiCanvasAssetFormatError> {
    write_u64(writer, value.key.id)?;
    write_string(writer, &value.key.name)?;
    write_script_property_value(writer, &value.value)
}

fn read_script_property(
    reader: &mut impl Read,
) -> Result<ScriptProperty, UiCanvasAssetFormatError> {
    Ok(ScriptProperty {
        key: ScriptPropertyKey {
            id: read_u64(reader)?,
            name: read_string(reader)?,
        },
        value: read_script_property_value(reader)?,
    })
}

async fn read_async_script_property(
    reader: &mut dyn Reader,
) -> Result<ScriptProperty, UiCanvasAssetFormatError> {
    Ok(ScriptProperty {
        key: ScriptPropertyKey {
            id: read_async_u64(reader).await?,
            name: read_async_string(reader).await?,
        },
        value: read_async_script_property_value(reader).await?,
    })
}

fn write_script_property_value(
    writer: &mut impl Write,
    value: &ScriptPropertyValue,
) -> Result<(), UiCanvasAssetFormatError> {
    match value {
        ScriptPropertyValue::Nil => write_u8(writer, 0)?,
        ScriptPropertyValue::Boolean(value) => {
            write_u8(writer, 1)?;
            write_bool(writer, *value)?;
        }
        ScriptPropertyValue::Number(value) => {
            write_u8(writer, 2)?;
            write_f64(writer, *value)?;
        }
        ScriptPropertyValue::String(value) => {
            write_u8(writer, 3)?;
            write_string(writer, value)?;
        }
        ScriptPropertyValue::BooleanArray(values) => {
            write_u8(writer, 4)?;
            write_bools(writer, values)?;
        }
        ScriptPropertyValue::NumberArray(values) => {
            write_u8(writer, 5)?;
            write_f64s(writer, values)?;
        }
        ScriptPropertyValue::StringArray(values) => {
            write_u8(writer, 6)?;
            write_strings(writer, values)?;
        }
        ScriptPropertyValue::Asset(value) => {
            write_u8(writer, 7)?;
            write_option_string(writer, value.as_deref())?;
        }
        ScriptPropertyValue::EntityRef(value) => {
            write_u8(writer, 8)?;
            write_option_u64(writer, *value)?;
        }
        ScriptPropertyValue::DynamicClass(value) => {
            write_u8(writer, 9)?;
            write_option_string(writer, value.type_name.as_deref())?;
            write_option_string(writer, value.payload_type_id.as_deref())?;
            write_script_dynamic_value(writer, &value.payload)?;
        }
        ScriptPropertyValue::DynamicClassArray(value) => {
            write_u8(writer, 10)?;
            write_option_string(writer, value.element_type_name.as_deref())?;
            write_u32(writer, value.len)?;
        }
    }
    Ok(())
}

fn read_script_property_value(
    reader: &mut impl Read,
) -> Result<ScriptPropertyValue, UiCanvasAssetFormatError> {
    match read_u8(reader)? {
        0 => Ok(ScriptPropertyValue::Nil),
        1 => Ok(ScriptPropertyValue::Boolean(read_bool(reader)?)),
        2 => Ok(ScriptPropertyValue::Number(read_f64(reader)?)),
        3 => Ok(ScriptPropertyValue::String(read_string(reader)?)),
        4 => Ok(ScriptPropertyValue::BooleanArray(read_bools(reader)?)),
        5 => Ok(ScriptPropertyValue::NumberArray(read_f64s(reader)?)),
        6 => Ok(ScriptPropertyValue::StringArray(read_strings(reader)?)),
        7 => Ok(ScriptPropertyValue::Asset(read_option_string(reader)?)),
        8 => Ok(ScriptPropertyValue::EntityRef(read_option_u64(reader)?)),
        9 => Ok(ScriptPropertyValue::DynamicClass(ScriptDynamicClassValue {
            type_name: read_option_string(reader)?,
            payload_type_id: read_option_string(reader)?,
            payload: read_script_dynamic_value(reader)?,
        })),
        10 => Ok(ScriptPropertyValue::DynamicClassArray(
            ScriptDynamicClassArrayValue {
                element_type_name: read_option_string(reader)?,
                len: read_u32(reader)?,
            },
        )),
        value => Err(UiCanvasAssetFormatError::InvalidEnum {
            field: "ScriptPropertyValue",
            value: value.into(),
        }),
    }
}

async fn read_async_script_property_value(
    reader: &mut dyn Reader,
) -> Result<ScriptPropertyValue, UiCanvasAssetFormatError> {
    match read_async_u8(reader).await? {
        0 => Ok(ScriptPropertyValue::Nil),
        1 => Ok(ScriptPropertyValue::Boolean(read_async_bool(reader).await?)),
        2 => Ok(ScriptPropertyValue::Number(read_async_f64(reader).await?)),
        3 => Ok(ScriptPropertyValue::String(
            read_async_string(reader).await?,
        )),
        4 => Ok(ScriptPropertyValue::BooleanArray(
            read_async_bools(reader).await?,
        )),
        5 => Ok(ScriptPropertyValue::NumberArray(
            read_async_f64s(reader).await?,
        )),
        6 => Ok(ScriptPropertyValue::StringArray(
            read_async_strings(reader).await?,
        )),
        7 => Ok(ScriptPropertyValue::Asset(
            read_async_option_string(reader).await?,
        )),
        8 => Ok(ScriptPropertyValue::EntityRef(
            read_async_option_u64(reader).await?,
        )),
        9 => Ok(ScriptPropertyValue::DynamicClass(ScriptDynamicClassValue {
            type_name: read_async_option_string(reader).await?,
            payload_type_id: read_async_option_string(reader).await?,
            payload: read_async_script_dynamic_value(reader).await?,
        })),
        10 => Ok(ScriptPropertyValue::DynamicClassArray(
            ScriptDynamicClassArrayValue {
                element_type_name: read_async_option_string(reader).await?,
                len: read_async_u32(reader).await?,
            },
        )),
        value => Err(UiCanvasAssetFormatError::InvalidEnum {
            field: "ScriptPropertyValue",
            value: value.into(),
        }),
    }
}

fn write_script_dynamic_value(
    writer: &mut impl Write,
    value: &ScriptDynamicValue,
) -> Result<(), UiCanvasAssetFormatError> {
    match value {
        ScriptDynamicValue::Unit => write_u8(writer, 0)?,
        ScriptDynamicValue::Bool(value) => {
            write_u8(writer, 1)?;
            write_bool(writer, *value)?;
        }
        ScriptDynamicValue::I64(value) => {
            write_u8(writer, 2)?;
            write_i64(writer, *value)?;
        }
        ScriptDynamicValue::U64(value) => {
            write_u8(writer, 3)?;
            write_u64(writer, *value)?;
        }
        ScriptDynamicValue::Number(value) => {
            write_u8(writer, 4)?;
            write_f64(writer, *value)?;
        }
        ScriptDynamicValue::String(value) => {
            write_u8(writer, 5)?;
            write_string(writer, value)?;
        }
        ScriptDynamicValue::Uuid(value) => {
            write_u8(writer, 6)?;
            write_string(writer, value)?;
        }
        ScriptDynamicValue::EntityRef(value) => {
            write_u8(writer, 7)?;
            write_option_u64(writer, *value)?;
        }
        ScriptDynamicValue::Vector2(value) => {
            write_u8(writer, 8)?;
            write_vec2(writer, *value)?;
        }
        ScriptDynamicValue::Vector3(value) => {
            write_u8(writer, 9)?;
            write_vec3(writer, *value)?;
        }
        ScriptDynamicValue::Color(value) => {
            write_u8(writer, 10)?;
            write_color(writer, *value)?;
        }
        ScriptDynamicValue::List(values) => {
            write_u8(writer, 11)?;
            write_u32(writer, checked_u32(values.len(), "dynamic list values")?)?;
            for value in values {
                write_script_dynamic_value(writer, value)?;
            }
        }
        ScriptDynamicValue::Struct(fields) => {
            write_u8(writer, 12)?;
            write_u32(writer, checked_u32(fields.len(), "dynamic struct fields")?)?;
            for field in fields {
                write_string(writer, &field.name)?;
                write_script_dynamic_value(writer, &field.value)?;
            }
        }
    }
    Ok(())
}

fn read_script_dynamic_value(
    reader: &mut impl Read,
) -> Result<ScriptDynamicValue, UiCanvasAssetFormatError> {
    match read_u8(reader)? {
        0 => Ok(ScriptDynamicValue::Unit),
        1 => Ok(ScriptDynamicValue::Bool(read_bool(reader)?)),
        2 => Ok(ScriptDynamicValue::I64(read_i64(reader)?)),
        3 => Ok(ScriptDynamicValue::U64(read_u64(reader)?)),
        4 => Ok(ScriptDynamicValue::Number(read_f64(reader)?)),
        5 => Ok(ScriptDynamicValue::String(read_string(reader)?)),
        6 => Ok(ScriptDynamicValue::Uuid(read_string(reader)?)),
        7 => Ok(ScriptDynamicValue::EntityRef(read_option_u64(reader)?)),
        8 => Ok(ScriptDynamicValue::Vector2(read_vec2(reader)?)),
        9 => Ok(ScriptDynamicValue::Vector3(read_vec3(reader)?)),
        10 => Ok(ScriptDynamicValue::Color(read_color(reader)?)),
        11 => {
            let count = read_u32(reader)? as usize;
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push(read_script_dynamic_value(reader)?);
            }
            Ok(ScriptDynamicValue::List(values))
        }
        12 => {
            let count = read_u32(reader)? as usize;
            let mut fields = Vec::with_capacity(count);
            for _ in 0..count {
                fields.push(ScriptDynamicField::new(
                    read_string(reader)?,
                    read_script_dynamic_value(reader)?,
                ));
            }
            Ok(ScriptDynamicValue::Struct(fields))
        }
        value => Err(UiCanvasAssetFormatError::InvalidEnum {
            field: "ScriptDynamicValue",
            value: value.into(),
        }),
    }
}

fn read_async_script_dynamic_value<'a>(
    reader: &'a mut dyn Reader,
) -> Pin<Box<dyn Future<Output = Result<ScriptDynamicValue, UiCanvasAssetFormatError>> + Send + 'a>>
{
    Box::pin(async move {
        match read_async_u8(reader).await? {
            0 => Ok(ScriptDynamicValue::Unit),
            1 => Ok(ScriptDynamicValue::Bool(read_async_bool(reader).await?)),
            2 => Ok(ScriptDynamicValue::I64(read_async_i64(reader).await?)),
            3 => Ok(ScriptDynamicValue::U64(read_async_u64(reader).await?)),
            4 => Ok(ScriptDynamicValue::Number(read_async_f64(reader).await?)),
            5 => Ok(ScriptDynamicValue::String(read_async_string(reader).await?)),
            6 => Ok(ScriptDynamicValue::Uuid(read_async_string(reader).await?)),
            7 => Ok(ScriptDynamicValue::EntityRef(
                read_async_option_u64(reader).await?,
            )),
            8 => Ok(ScriptDynamicValue::Vector2(read_async_vec2(reader).await?)),
            9 => Ok(ScriptDynamicValue::Vector3(read_async_vec3(reader).await?)),
            10 => Ok(ScriptDynamicValue::Color(read_async_color(reader).await?)),
            11 => {
                let count = read_async_u32(reader).await? as usize;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(read_async_script_dynamic_value(reader).await?);
                }
                Ok(ScriptDynamicValue::List(values))
            }
            12 => {
                let count = read_async_u32(reader).await? as usize;
                let mut fields = Vec::with_capacity(count);
                for _ in 0..count {
                    fields.push(ScriptDynamicField::new(
                        read_async_string(reader).await?,
                        read_async_script_dynamic_value(reader).await?,
                    ));
                }
                Ok(ScriptDynamicValue::Struct(fields))
            }
            value => Err(UiCanvasAssetFormatError::InvalidEnum {
                field: "ScriptDynamicValue",
                value: value.into(),
            }),
        }
    })
}

fn write_child_order(
    writer: &mut impl Write,
    values: &[UiChildOrder],
) -> Result<(), UiCanvasAssetFormatError> {
    write_u32(writer, checked_u32(values.len(), "UI child order entries")?)?;
    for value in values {
        write_entity_id(writer, value.entity_id)?;
        write_u64(writer, value.sort_index)?;
    }
    Ok(())
}

fn read_child_order(reader: &mut impl Read) -> Result<Vec<UiChildOrder>, std::io::Error> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(UiChildOrder::new(
            read_entity_id(reader)?,
            read_u64(reader)?,
        ));
    }
    Ok(values)
}

async fn read_async_child_order(
    reader: &mut dyn Reader,
) -> Result<Vec<UiChildOrder>, std::io::Error> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(UiChildOrder::new(
            read_async_entity_id(reader).await?,
            read_async_u64(reader).await?,
        ));
    }
    Ok(values)
}

fn write_component_kinds(
    writer: &mut impl Write,
    values: &[UiComponentKind],
) -> Result<(), UiCanvasAssetFormatError> {
    write_u32(writer, checked_u32(values.len(), "UI component kinds")?)?;
    for value in values {
        write_u8(writer, component_kind_id(*value))?;
    }
    Ok(())
}

fn read_component_kinds(
    reader: &mut impl Read,
) -> Result<Vec<UiComponentKind>, UiCanvasAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(component_kind_from_id(read_u8(reader)?)?);
    }
    Ok(values)
}

async fn read_async_component_kinds(
    reader: &mut dyn Reader,
) -> Result<Vec<UiComponentKind>, UiCanvasAssetFormatError> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(component_kind_from_id(read_async_u8(reader).await?)?);
    }
    Ok(values)
}

const fn component_kind_id(kind: UiComponentKind) -> u8 {
    match kind {
        UiComponentKind::Canvas => 0,
        UiComponentKind::Transform2d => 2,
        UiComponentKind::Element => 3,
        UiComponentKind::Image => 4,
        UiComponentKind::Text => 5,
        UiComponentKind::Button => 6,
        UiComponentKind::Interactable => 7,
        UiComponentKind::Fader => 8,
        UiComponentKind::Mask => 9,
        UiComponentKind::LayoutRow => 10,
        UiComponentKind::LayoutColumn => 11,
        UiComponentKind::LayoutGrid => 12,
        UiComponentKind::LayoutCell => 13,
        UiComponentKind::Script => 14,
        UiComponentKind::Other => 255,
    }
}

const fn component_kind_from_id(id: u8) -> Result<UiComponentKind, UiCanvasAssetFormatError> {
    match id {
        0 => Ok(UiComponentKind::Canvas),
        1 | 255 => Ok(UiComponentKind::Other),
        2 => Ok(UiComponentKind::Transform2d),
        3 => Ok(UiComponentKind::Element),
        4 => Ok(UiComponentKind::Image),
        5 => Ok(UiComponentKind::Text),
        6 => Ok(UiComponentKind::Button),
        7 => Ok(UiComponentKind::Interactable),
        8 => Ok(UiComponentKind::Fader),
        9 => Ok(UiComponentKind::Mask),
        10 => Ok(UiComponentKind::LayoutRow),
        11 => Ok(UiComponentKind::LayoutColumn),
        12 => Ok(UiComponentKind::LayoutGrid),
        13 => Ok(UiComponentKind::LayoutCell),
        14 => Ok(UiComponentKind::Script),
        _ => Err(UiCanvasAssetFormatError::InvalidData("UI component kind")),
    }
}

fn write_rect(writer: &mut impl Write, value: UiRect) -> Result<(), std::io::Error> {
    write_f32(writer, value.left)?;
    write_f32(writer, value.top)?;
    write_f32(writer, value.right)?;
    write_f32(writer, value.bottom)
}

fn read_rect(reader: &mut impl Read) -> Result<UiRect, std::io::Error> {
    Ok(UiRect::new(
        read_f32(reader)?,
        read_f32(reader)?,
        read_f32(reader)?,
        read_f32(reader)?,
    ))
}

async fn read_async_rect(reader: &mut dyn Reader) -> Result<UiRect, std::io::Error> {
    Ok(UiRect::new(
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
    ))
}

fn write_vec2(writer: &mut impl Write, value: Vec2) -> Result<(), std::io::Error> {
    write_f32(writer, value.x)?;
    write_f32(writer, value.y)
}

fn read_vec2(reader: &mut impl Read) -> Result<Vec2, std::io::Error> {
    Ok(Vec2::new(read_f32(reader)?, read_f32(reader)?))
}

async fn read_async_vec2(reader: &mut dyn Reader) -> Result<Vec2, std::io::Error> {
    Ok(Vec2::new(
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
    ))
}

fn write_vec3(writer: &mut impl Write, value: Vec3) -> Result<(), std::io::Error> {
    write_f32(writer, value.x)?;
    write_f32(writer, value.y)?;
    write_f32(writer, value.z)
}

fn read_vec3(reader: &mut impl Read) -> Result<Vec3, std::io::Error> {
    Ok(Vec3::new(
        read_f32(reader)?,
        read_f32(reader)?,
        read_f32(reader)?,
    ))
}

async fn read_async_vec3(reader: &mut dyn Reader) -> Result<Vec3, std::io::Error> {
    Ok(Vec3::new(
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
    ))
}

fn write_color(writer: &mut impl Write, value: LinearRgba) -> Result<(), std::io::Error> {
    write_f32(writer, value.red)?;
    write_f32(writer, value.green)?;
    write_f32(writer, value.blue)?;
    write_f32(writer, value.alpha)
}

fn read_color(reader: &mut impl Read) -> Result<LinearRgba, std::io::Error> {
    Ok(LinearRgba::new(
        read_f32(reader)?,
        read_f32(reader)?,
        read_f32(reader)?,
        read_f32(reader)?,
    ))
}

async fn read_async_color(reader: &mut dyn Reader) -> Result<LinearRgba, std::io::Error> {
    Ok(LinearRgba::new(
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
    ))
}

fn write_strings(
    writer: &mut impl Write,
    values: &[String],
) -> Result<(), UiCanvasAssetFormatError> {
    write_u32(writer, checked_u32(values.len(), "strings")?)?;
    for value in values {
        write_string(writer, value)?;
    }
    Ok(())
}

fn read_strings(reader: &mut impl Read) -> Result<Vec<String>, UiCanvasAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_string(reader)?);
    }
    Ok(values)
}

async fn read_async_strings(
    reader: &mut dyn Reader,
) -> Result<Vec<String>, UiCanvasAssetFormatError> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_async_string(reader).await?);
    }
    Ok(values)
}

fn write_bools(writer: &mut impl Write, values: &[bool]) -> Result<(), UiCanvasAssetFormatError> {
    write_u32(writer, checked_u32(values.len(), "bools")?)?;
    for value in values {
        write_bool(writer, *value)?;
    }
    Ok(())
}

fn read_bools(reader: &mut impl Read) -> Result<Vec<bool>, std::io::Error> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_bool(reader)?);
    }
    Ok(values)
}

async fn read_async_bools(reader: &mut dyn Reader) -> Result<Vec<bool>, std::io::Error> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_async_bool(reader).await?);
    }
    Ok(values)
}

fn write_f64s(writer: &mut impl Write, values: &[f64]) -> Result<(), UiCanvasAssetFormatError> {
    write_u32(writer, checked_u32(values.len(), "f64s")?)?;
    for value in values {
        write_f64(writer, *value)?;
    }
    Ok(())
}

fn read_f64s(reader: &mut impl Read) -> Result<Vec<f64>, std::io::Error> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_f64(reader)?);
    }
    Ok(values)
}

async fn read_async_f64s(reader: &mut dyn Reader) -> Result<Vec<f64>, std::io::Error> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_async_f64(reader).await?);
    }
    Ok(values)
}

fn write_option_string(
    writer: &mut impl Write,
    value: Option<&str>,
) -> Result<(), UiCanvasAssetFormatError> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_string(writer, value)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_string(reader: &mut impl Read) -> Result<Option<String>, UiCanvasAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(read_string(reader)?))
}

async fn read_async_option_string(
    reader: &mut dyn Reader,
) -> Result<Option<String>, UiCanvasAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(read_async_string(reader).await?))
}

fn write_option_u64(writer: &mut impl Write, value: Option<u64>) -> Result<(), std::io::Error> {
    if let Some(value) = value {
        write_bool(writer, true)?;
        write_u64(writer, value)?;
    } else {
        write_bool(writer, false)?;
    }
    Ok(())
}

fn read_option_u64(reader: &mut impl Read) -> Result<Option<u64>, std::io::Error> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(read_u64(reader)?))
}

async fn read_async_option_u64(reader: &mut dyn Reader) -> Result<Option<u64>, std::io::Error> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(read_async_u64(reader).await?))
}

fn write_string(writer: &mut impl Write, value: &str) -> Result<(), UiCanvasAssetFormatError> {
    write_u32(writer, checked_u32(value.len(), "string bytes")?)?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

fn read_string(reader: &mut impl Read) -> Result<String, UiCanvasAssetFormatError> {
    let len = read_u32(reader)? as usize;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes)?)
}

async fn read_async_string(reader: &mut dyn Reader) -> Result<String, UiCanvasAssetFormatError> {
    let len = read_async_u32(reader).await? as usize;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes).await?;
    Ok(String::from_utf8(bytes)?)
}

fn write_entity_id(writer: &mut impl Write, value: UiEntityId) -> Result<(), std::io::Error> {
    write_u64(writer, value.as_u64())
}

fn read_entity_id(reader: &mut impl Read) -> Result<UiEntityId, std::io::Error> {
    Ok(UiEntityId::new(read_u64(reader)?))
}

async fn read_async_entity_id(reader: &mut dyn Reader) -> Result<UiEntityId, std::io::Error> {
    Ok(UiEntityId::new(read_async_u64(reader).await?))
}

fn write_bool(writer: &mut impl Write, value: bool) -> Result<(), std::io::Error> {
    writer.write_all(&[u8::from(value)])
}

fn read_bool(reader: &mut impl Read) -> Result<bool, std::io::Error> {
    Ok(read_u8(reader)? != 0)
}

async fn read_async_bool(reader: &mut dyn Reader) -> Result<bool, std::io::Error> {
    Ok(read_async_u8(reader).await? != 0)
}

fn write_u8(writer: &mut impl Write, value: u8) -> Result<(), std::io::Error> {
    writer.write_all(&[value])
}

fn write_u32(writer: &mut impl Write, value: u32) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn write_i32(writer: &mut impl Write, value: i32) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn write_i64(writer: &mut impl Write, value: i64) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u64(writer: &mut impl Write, value: u64) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn write_f32(writer: &mut impl Write, value: f32) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn write_f64(writer: &mut impl Write, value: f64) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn read_u8(reader: &mut impl Read) -> Result<u8, std::io::Error> {
    let mut bytes = [0u8; 1];
    reader.read_exact(&mut bytes)?;
    Ok(bytes[0])
}

fn read_u32(reader: &mut impl Read) -> Result<u32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_i32(reader: &mut impl Read) -> Result<i32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(i32::from_le_bytes(bytes))
}

fn read_i64(reader: &mut impl Read) -> Result<i64, std::io::Error> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(i64::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> Result<u64, std::io::Error> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_f32(reader: &mut impl Read) -> Result<f32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}

fn read_f64(reader: &mut impl Read) -> Result<f64, std::io::Error> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(f64::from_le_bytes(bytes))
}

async fn read_async_u8(reader: &mut dyn Reader) -> Result<u8, std::io::Error> {
    let mut bytes = [0u8; 1];
    reader.read_exact(&mut bytes).await?;
    Ok(bytes[0])
}

async fn read_async_u32(reader: &mut dyn Reader) -> Result<u32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(u32::from_le_bytes(bytes))
}

async fn read_async_i32(reader: &mut dyn Reader) -> Result<i32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(i32::from_le_bytes(bytes))
}

async fn read_async_i64(reader: &mut dyn Reader) -> Result<i64, std::io::Error> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes).await?;
    Ok(i64::from_le_bytes(bytes))
}

async fn read_async_u64(reader: &mut dyn Reader) -> Result<u64, std::io::Error> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes).await?;
    Ok(u64::from_le_bytes(bytes))
}

async fn read_async_f32(reader: &mut dyn Reader) -> Result<f32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(f32::from_le_bytes(bytes))
}

async fn read_async_f64(reader: &mut dyn Reader) -> Result<f64, std::io::Error> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes).await?;
    Ok(f64::from_le_bytes(bytes))
}

fn checked_u32(count: usize, what: &'static str) -> Result<u32, UiCanvasAssetFormatError> {
    u32::try_from(count).map_err(|_| UiCanvasAssetFormatError::TooManyItems { what, count })
}
