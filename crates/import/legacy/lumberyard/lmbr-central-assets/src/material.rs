//! Cry `.mtl` material parser used by the offline import pipeline.

use std::collections::{BTreeMap, BTreeSet};
use std::str;

use anyhow::{Context, Result, anyhow, bail};
use az_asset::{EngineTextureFormat, normalize_source_path, source_extension_key};
use az_filesystem::texture_source_asset_path;
use az_gem_lmbr_central::{
    MaterialAsset, MaterialDefinition, MaterialOverrideAsset, MaterialOverrideParam,
    MaterialOverrideParamBlock, MaterialOverrideSubTarget, MaterialOverrideSwitch,
    MaterialOverrideTarget, MaterialOverrideValueKind, MaterialPublicParam, MaterialTextureFilter,
    MaterialTextureMap, MaterialTextureReference, MaterialTextureType, material_color_from_native,
};
use bevy::color::{LinearRgba, Srgba};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

const MATERIAL_TAG: &[u8] = b"Material";
const TEXTURE_TAG: &[u8] = b"Texture";
const TEX_MOD_TAG: &[u8] = b"TexMod";
const PUBLIC_PARAMS_TAG: &[u8] = b"PublicParams";
const MATERIAL_PARAMS_OVERRIDE_TAG: &[u8] = b"MaterialParamsOverride";
const SUB_MATERIAL_TAG: &[u8] = b"SubMaterial";
const SHADER_GENERATION_PARAMS_TAG: &[u8] = b"ShaderGenerationParams";
const TEXTURE_MAPS_TAG: &[u8] = b"TextureMaps";
const SHADER_PARAMS_TAG: &[u8] = b"ShaderParams";
const PARAM_TAG: &[u8] = b"param";

pub type MaterialTransformError = anyhow::Error;

/// Parses a legacy Cry `.mtl` payload.
///
/// # Errors
///
/// Returns an error when the payload is not UTF-8, plus any error
/// [`parse_material_asset_str`] returns.
pub fn parse_material_asset(
    source_path: &str,
    bytes: &[u8],
    _: EngineTextureFormat,
) -> Result<MaterialAsset> {
    let xml = str::from_utf8(bytes).context("material XML is not UTF-8")?;
    parse_material_asset_str(source_path, xml)
}

/// Parses a legacy Cry `.mtl` document.
///
/// # Errors
///
/// Returns an error when the XML is malformed, when an attribute name or
/// value is not UTF-8, when a `Material` end tag has no matching start tag,
/// when a `TexMod` or child element appears outside its `Texture`/`Material`
/// parent, when the document declares more than one root `Material`, when it
/// declares none, or when it ends with elements still open.
pub fn parse_material_asset_str(source_path: &str, xml: &str) -> Result<MaterialAsset> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut stack = Vec::new();
    let mut root = None;
    let mut sub_materials = Vec::new();

    loop {
        match reader.read_event().context("read material XML event")? {
            Event::Start(event) if event.name().as_ref() == MATERIAL_TAG => {
                stack.push(parse_material_start(&reader, &event)?);
            }
            Event::Empty(event) if event.name().as_ref() == MATERIAL_TAG => {
                finish_material(
                    parse_material_start(&reader, &event)?,
                    &stack,
                    &mut root,
                    &mut sub_materials,
                )?;
            }
            Event::End(event) if event.name().as_ref() == MATERIAL_TAG => {
                let material = stack
                    .pop()
                    .ok_or_else(|| anyhow!("material end tag without matching start tag"))?;
                finish_material(material, &stack, &mut root, &mut sub_materials)?;
            }
            Event::Start(event) | Event::Empty(event) if event.name().as_ref() == TEXTURE_TAG => {
                let texture = parse_texture_start(&reader, &event)?;
                current_material_mut(&mut stack)?.textures.push(texture);
            }
            Event::Start(event) | Event::Empty(event) if event.name().as_ref() == TEX_MOD_TAG => {
                let modifier = parse_public_params(&reader, &event)?;
                let material = current_material_mut(&mut stack)?;
                let texture = material
                    .textures
                    .last_mut()
                    .ok_or_else(|| anyhow!("TexMod element outside a Texture element"))?;
                texture.texture_modifier.extend(modifier);
            }
            Event::Start(event) | Event::Empty(event)
                if event.name().as_ref() == PUBLIC_PARAMS_TAG =>
            {
                let params = parse_public_params(&reader, &event)?;
                current_material_mut(&mut stack)?
                    .public_params
                    .extend(params);
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if !stack.is_empty() {
        bail!("material XML ended before all Material elements closed");
    }

    Ok(MaterialAsset {
        source_path: normalize_source_path(source_path),
        root: root.ok_or_else(|| anyhow!("material XML did not contain a Material root"))?,
        sub_materials,
    })
}

/// Parses a legacy material-override XML payload.
///
/// # Errors
///
/// Returns an error when the payload is not UTF-8, plus any error
/// [`parse_material_override_asset_str`] returns.
pub fn parse_material_override_asset(
    source_path: &str,
    bytes: &[u8],
    _: EngineTextureFormat,
) -> Result<MaterialOverrideAsset> {
    let xml = str::from_utf8(bytes).context("material override XML is not UTF-8")?;
    parse_material_override_asset_str(source_path, xml)
}

/// Parses a legacy `MaterialParamsOverride` document.
///
/// # Errors
///
/// Returns an error when the XML is malformed, when the document has no
/// `MaterialParamsOverride` root or more than one, when `Material`,
/// `SubMaterial`, parameter sections, or parameter blocks nest in a way the
/// override schema does not allow, when an end tag has no matching start tag,
/// or when the document ends with elements still open.
pub fn parse_material_override_asset_str(
    source_path: &str,
    xml: &str,
) -> Result<MaterialOverrideAsset> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut state = MaterialOverrideReader::new(source_path);
    loop {
        let event = reader
            .read_event()
            .context("read material override XML event")?;
        if matches!(event, Event::Eof) {
            break;
        }
        state.visit(&reader, event)?;
    }

    state.finish()
}

/// Element-nesting state for one `MaterialParamsOverride` document.
#[derive(Default)]
struct MaterialOverrideReader {
    asset: MaterialOverrideAsset,
    saw_root: bool,
    current_material: Option<MaterialOverrideTarget>,
    current_sub_material: Option<MaterialOverrideSubTarget>,
    current_section: Option<MaterialOverrideSection>,
    current_block: Option<MaterialOverrideParamBlock>,
}

impl MaterialOverrideReader {
    fn new(source_path: &str) -> Self {
        Self {
            asset: MaterialOverrideAsset {
                source_path: normalize_source_path(source_path),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn visit(&mut self, reader: &Reader<&[u8]>, event: Event<'_>) -> Result<()> {
        // A self-closing element reaches the section reader as `Empty`, which
        // commits its parameter block immediately instead of waiting for an
        // end tag.
        let is_empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(event) | Event::Empty(event)
                if event.name().as_ref() == MATERIAL_PARAMS_OVERRIDE_TAG =>
            {
                parse_material_override_root(reader, &event, &mut self.asset, &mut self.saw_root)?;
            }
            Event::Start(event) if event.name().as_ref() == MATERIAL_TAG => {
                start_material_override_target(reader, &event, &mut self.current_material)?;
            }
            Event::Empty(event) if event.name().as_ref() == MATERIAL_TAG => {
                self.asset
                    .materials
                    .push(parse_material_override_target_start(reader, &event)?);
            }
            Event::End(event) if event.name().as_ref() == MATERIAL_TAG => {
                end_material_override_target(
                    &mut self.asset,
                    &mut self.current_material,
                    self.current_sub_material.as_ref(),
                    self.current_block.as_ref(),
                )?;
            }
            Event::Start(event) if event.name().as_ref() == SUB_MATERIAL_TAG => {
                start_material_override_subtarget(reader, &event, &mut self.current_sub_material)?;
            }
            Event::Empty(event) if event.name().as_ref() == SUB_MATERIAL_TAG => {
                push_material_override_subtarget(
                    &mut self.current_material,
                    parse_material_override_subtarget_start(reader, &event)?,
                )?;
            }
            Event::End(event) if event.name().as_ref() == SUB_MATERIAL_TAG => {
                end_material_override_subtarget(
                    &mut self.current_material,
                    &mut self.current_sub_material,
                    self.current_block.as_ref(),
                )?;
            }
            Event::Start(event) if material_override_section(event.name().as_ref()).is_some() => {
                open_material_override_section(&event, &mut self.current_section)?;
            }
            Event::Empty(event) if material_override_section(event.name().as_ref()).is_some() => {}
            Event::End(event) if material_override_section(event.name().as_ref()).is_some() => {
                close_material_override_section(
                    &mut self.current_section,
                    self.current_block.as_ref(),
                )?;
            }
            Event::Start(event) | Event::Empty(event) => {
                read_material_override_section_child(
                    reader,
                    &event,
                    is_empty,
                    self.current_section,
                    &mut self.current_sub_material,
                    &mut self.current_block,
                )?;
            }
            Event::End(event)
                if self
                    .current_block
                    .as_ref()
                    .is_some_and(|block| event.name().as_ref() == block.name.as_bytes()) =>
            {
                close_material_override_param_block(
                    &mut self.current_sub_material,
                    self.current_section,
                    &mut self.current_block,
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<MaterialOverrideAsset> {
        let has_open_element = self.current_material.is_some()
            || self.current_sub_material.is_some()
            || self.current_block.is_some();
        finish_material_override(self.asset, self.saw_root, has_open_element)
    }
}

/// Rejects a document with no `MaterialParamsOverride` root, or one that ended
/// while an element was still open.
fn finish_material_override(
    asset: MaterialOverrideAsset,
    saw_root: bool,
    has_open_element: bool,
) -> Result<MaterialOverrideAsset> {
    if !saw_root {
        bail!("material override XML did not contain a MaterialParamsOverride root");
    }
    if has_open_element {
        bail!("material override XML ended before all elements closed");
    }
    Ok(asset)
}

/// Opens a parameter section; rejects a nested one.
fn open_material_override_section(
    event: &BytesStart<'_>,
    current_section: &mut Option<MaterialOverrideSection>,
) -> Result<()> {
    if current_section.is_some() {
        bail!("nested material override parameter sections are not supported");
    }
    *current_section = material_override_section(event.name().as_ref());
    Ok(())
}

/// Closes a parameter section; rejects an end tag that arrives while a
/// parameter block is still open.
fn close_material_override_section(
    current_section: &mut Option<MaterialOverrideSection>,
    current_block: Option<&MaterialOverrideParamBlock>,
) -> Result<()> {
    if current_block.is_some() {
        bail!("parameter section ended before its parameter block closed");
    }
    *current_section = None;
    Ok(())
}

/// Collects the texture source paths a legacy `.mtl` references.
///
/// # Errors
///
/// Returns an error when the payload is not UTF-8 or the XML is malformed.
pub fn material_texture_source_paths(bytes: &[u8]) -> Result<Vec<String>> {
    let xml = str::from_utf8(bytes).context("material XML is not UTF-8")?;
    material_texture_source_paths_str(xml)
}

/// Collects the texture source paths a legacy material-override XML
/// references.
///
/// # Errors
///
/// Returns an error when the payload is not UTF-8 or the XML is malformed.
pub fn material_override_texture_source_paths(bytes: &[u8]) -> Result<Vec<String>> {
    let xml = str::from_utf8(bytes).context("material override XML is not UTF-8")?;
    material_override_texture_source_paths_str(xml)
}

fn material_texture_source_paths_str(xml: &str) -> Result<Vec<String>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut paths = BTreeSet::new();
    loop {
        match reader.read_event().context("read material XML event")? {
            Event::Start(event) | Event::Empty(event) if event.name().as_ref() == TEXTURE_TAG => {
                let attrs = attributes(&reader, &event)?;
                if let Some(path) = string_attr(&attrs, "File") {
                    paths.insert(texture_source_asset_path(&path));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(paths.into_iter().collect())
}

fn material_override_texture_source_paths_str(xml: &str) -> Result<Vec<String>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut in_texture_maps = 0usize;
    let mut paths = BTreeSet::new();
    loop {
        match reader
            .read_event()
            .context("read material override XML event")?
        {
            Event::Start(event) if event.name().as_ref() == TEXTURE_MAPS_TAG => {
                in_texture_maps += 1;
            }
            Event::End(event) if event.name().as_ref() == TEXTURE_MAPS_TAG => {
                in_texture_maps = in_texture_maps.saturating_sub(1);
            }
            Event::Start(event) | Event::Empty(event)
                if in_texture_maps > 0 && event.name().as_ref() == PARAM_TAG =>
            {
                let attrs = attributes(&reader, &event)?;
                if is_texture_string_param(&attrs)
                    && let Some(path) = string_attr(&attrs, "value")
                        .map(|value| texture_source_asset_path(&value))
                        .filter(|path| is_texture_source_path(path))
                {
                    paths.insert(path);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(paths.into_iter().collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaterialOverrideSection {
    ShaderGenerationParams,
    TextureMaps,
    ShaderParams,
}

fn material_override_section(name: &[u8]) -> Option<MaterialOverrideSection> {
    match name {
        SHADER_GENERATION_PARAMS_TAG => Some(MaterialOverrideSection::ShaderGenerationParams),
        TEXTURE_MAPS_TAG => Some(MaterialOverrideSection::TextureMaps),
        SHADER_PARAMS_TAG => Some(MaterialOverrideSection::ShaderParams),
        _ => None,
    }
}

fn parse_material_override_root(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    asset: &mut MaterialOverrideAsset,
    saw_root: &mut bool,
) -> Result<()> {
    if *saw_root {
        bail!("material override XML contained more than one root element");
    }
    let attrs = attributes(reader, event)?;
    asset.max_trigger_distance = string_attr(&attrs, "MaxTriggerDistance");
    asset.extra_attributes = extra_attrs(&attrs, &["MaxTriggerDistance"]);
    *saw_root = true;
    Ok(())
}

/// Opens a `<Material>` override target; rejects a nested one.
fn start_material_override_target(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    current_material: &mut Option<MaterialOverrideTarget>,
) -> Result<()> {
    if current_material.is_some() {
        bail!("nested Material elements are not supported in material overrides");
    }
    *current_material = Some(parse_material_override_target_start(reader, event)?);
    Ok(())
}

/// Closes a `<Material>` override target; rejects an unbalanced end tag or an
/// end tag that arrives while a child element is still open.
fn end_material_override_target(
    asset: &mut MaterialOverrideAsset,
    current_material: &mut Option<MaterialOverrideTarget>,
    current_sub_material: Option<&MaterialOverrideSubTarget>,
    current_block: Option<&MaterialOverrideParamBlock>,
) -> Result<()> {
    if current_sub_material.is_some() || current_block.is_some() {
        bail!("Material element ended before its child elements closed");
    }
    asset.materials.push(
        current_material
            .take()
            .ok_or_else(|| anyhow!("Material end tag without matching start tag"))?,
    );
    Ok(())
}

/// Opens a `<SubMaterial>` override target; rejects a nested one.
fn start_material_override_subtarget(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    current_sub_material: &mut Option<MaterialOverrideSubTarget>,
) -> Result<()> {
    if current_sub_material.is_some() {
        bail!("nested SubMaterial elements are not supported in material overrides");
    }
    *current_sub_material = Some(parse_material_override_subtarget_start(reader, event)?);
    Ok(())
}

/// Closes a `<SubMaterial>` override target; rejects an unbalanced end tag or
/// an end tag that arrives while a parameter block is still open.
fn end_material_override_subtarget(
    current_material: &mut Option<MaterialOverrideTarget>,
    current_sub_material: &mut Option<MaterialOverrideSubTarget>,
    current_block: Option<&MaterialOverrideParamBlock>,
) -> Result<()> {
    if current_block.is_some() {
        bail!("SubMaterial element ended before its parameter block closed");
    }
    let sub_material = current_sub_material
        .take()
        .ok_or_else(|| anyhow!("SubMaterial end tag without matching start tag"))?;
    push_material_override_subtarget(current_material, sub_material)
}

/// Commits the open parameter block to its section.
fn close_material_override_param_block(
    current_sub_material: &mut Option<MaterialOverrideSubTarget>,
    section: Option<MaterialOverrideSection>,
    current_block: &mut Option<MaterialOverrideParamBlock>,
) -> Result<()> {
    let block = current_block
        .take()
        .ok_or_else(|| anyhow!("parameter block was missing"))?;
    push_material_override_param_block(current_sub_material, section, block)
}

fn parse_material_override_target_start(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<MaterialOverrideTarget> {
    let attrs = attributes(reader, event)?;
    Ok(MaterialOverrideTarget {
        name: string_attr(&attrs, "name").or_else(|| string_attr(&attrs, "Name")),
        exclude: string_attr(&attrs, "exclude").or_else(|| string_attr(&attrs, "Exclude")),
        sub_materials: Vec::new(),
        extra_attributes: extra_attrs(&attrs, &["name", "Name", "exclude", "Exclude"]),
    })
}

fn parse_material_override_subtarget_start(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<MaterialOverrideSubTarget> {
    let attrs = attributes(reader, event)?;
    Ok(MaterialOverrideSubTarget {
        name: string_attr(&attrs, "name").or_else(|| string_attr(&attrs, "Name")),
        shader_generation_params: Vec::new(),
        texture_maps: Vec::new(),
        shader_params: Vec::new(),
        extra_attributes: extra_attrs(&attrs, &["name", "Name"]),
    })
}

fn read_material_override_section_child(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    is_empty: bool,
    section: Option<MaterialOverrideSection>,
    current_sub_material: &mut Option<MaterialOverrideSubTarget>,
    current_block: &mut Option<MaterialOverrideParamBlock>,
) -> Result<()> {
    let Some(section) = section else {
        return Ok(());
    };

    match section {
        MaterialOverrideSection::ShaderGenerationParams => {
            current_sub_material_mut(current_sub_material)?
                .shader_generation_params
                .push(parse_material_override_switch_start(reader, event)?);
        }
        MaterialOverrideSection::TextureMaps | MaterialOverrideSection::ShaderParams => {
            if event.name().as_ref() == PARAM_TAG {
                let resolve_texture = section == MaterialOverrideSection::TextureMaps;
                let param = parse_material_override_param_start(reader, event, resolve_texture)?;
                current_block
                    .as_mut()
                    .ok_or_else(|| anyhow!("material override param outside a parameter block"))?
                    .params
                    .push(param);
            } else {
                if current_block.is_some() {
                    bail!("nested material override parameter blocks are not supported");
                }
                let block = parse_material_override_param_block_start(reader, event)?;
                if is_empty {
                    push_material_override_param_block(current_sub_material, Some(section), block)?;
                } else {
                    *current_block = Some(block);
                }
            }
        }
    }

    Ok(())
}

fn parse_material_override_switch_start(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<MaterialOverrideSwitch> {
    let attrs = attributes(reader, event)?;
    Ok(MaterialOverrideSwitch {
        name: tag_name(event.name().as_ref())?,
        enabled: bool_attr(&attrs, "enabled")
            .or_else(|| bool_attr(&attrs, "Enabled"))
            .unwrap_or(true),
        extra_attributes: extra_attrs(&attrs, &["enabled", "Enabled"]),
    })
}

fn parse_material_override_param_block_start(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<MaterialOverrideParamBlock> {
    Ok(MaterialOverrideParamBlock {
        name: tag_name(event.name().as_ref())?,
        params: Vec::new(),
        extra_attributes: extra_attrs(&attributes(reader, event)?, &[]),
    })
}

fn parse_material_override_param_start(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    resolve_texture: bool,
) -> Result<MaterialOverrideParam> {
    let attrs = attributes(reader, event)?;
    let value_kind = string_attr(&attrs, "type")
        .map(|value| MaterialOverrideValueKind::from_native_name(&value))
        .unwrap_or_default();
    let mut value = string_attr(&attrs, "value").unwrap_or_default();
    if resolve_texture
        && value_kind == MaterialOverrideValueKind::String
        && let Some(engine_path) = texture_param_engine_asset_path(&value)
    {
        value = engine_path;
    }

    Ok(MaterialOverrideParam {
        name: string_attr(&attrs, "name").unwrap_or_default(),
        value_kind,
        value,
        extra_attributes: extra_attrs(&attrs, &["name", "type", "value"]),
    })
}

fn push_material_override_subtarget(
    current_material: &mut Option<MaterialOverrideTarget>,
    sub_material: MaterialOverrideSubTarget,
) -> Result<()> {
    current_material
        .as_mut()
        .ok_or_else(|| anyhow!("SubMaterial element outside a Material element"))?
        .sub_materials
        .push(sub_material);
    Ok(())
}

fn push_material_override_param_block(
    current_sub_material: &mut Option<MaterialOverrideSubTarget>,
    section: Option<MaterialOverrideSection>,
    block: MaterialOverrideParamBlock,
) -> Result<()> {
    let sub_material = current_sub_material_mut(current_sub_material)?;
    match section {
        Some(MaterialOverrideSection::TextureMaps) => sub_material.texture_maps.push(block),
        Some(MaterialOverrideSection::ShaderParams) => sub_material.shader_params.push(block),
        Some(MaterialOverrideSection::ShaderGenerationParams) | None => {
            bail!("parameter block outside a parameter section");
        }
    }
    Ok(())
}

fn current_sub_material_mut(
    current_sub_material: &mut Option<MaterialOverrideSubTarget>,
) -> Result<&mut MaterialOverrideSubTarget> {
    current_sub_material
        .as_mut()
        .ok_or_else(|| anyhow!("material override data outside a SubMaterial element"))
}

fn finish_material(
    material: MaterialDefinition,
    stack: &[MaterialDefinition],
    root: &mut Option<MaterialDefinition>,
    sub_materials: &mut Vec<MaterialDefinition>,
) -> Result<()> {
    if stack.is_empty() {
        if root.replace(material).is_some() {
            bail!("material XML contained more than one root Material element");
        }
    } else {
        sub_materials.push(material);
    }
    Ok(())
}

fn current_material_mut(stack: &mut [MaterialDefinition]) -> Result<&mut MaterialDefinition> {
    stack
        .last_mut()
        .ok_or_else(|| anyhow!("material child element outside a Material element"))
}

fn parse_material_start(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<MaterialDefinition> {
    let attrs = attributes(reader, event)?;
    let known = [
        "AlphaTest",
        "CloakAmount",
        "Diffuse",
        "Emissive",
        "Emittance",
        "GenMask",
        "MtlFlags",
        "Name",
        "Opacity",
        "Shader",
        "Shininess",
        "Specular",
        "StringGenMask",
        "SurfaceType",
    ];

    Ok(MaterialDefinition {
        name: string_attr(&attrs, "Name"),
        shader: string_attr(&attrs, "Shader"),
        surface_type: string_attr(&attrs, "SurfaceType"),
        diffuse: string_attr(&attrs, "Diffuse")
            .and_then(|value| material_color_from_native(&value)),
        specular: string_attr(&attrs, "Specular")
            .and_then(|value| material_color_from_native(&value)),
        emissive: string_attr(&attrs, "Emissive")
            .and_then(|value| material_color_from_native(&value)),
        emittance: string_attr(&attrs, "Emittance")
            .and_then(|value| material_emittance_from_native(&value)),
        opacity: f32_attr(&attrs, "Opacity").unwrap_or(1.0),
        shininess: f32_attr(&attrs, "Shininess").unwrap_or(0.0),
        alpha_test: f32_attr(&attrs, "AlphaTest"),
        gen_mask: string_attr(&attrs, "GenMask"),
        string_gen_mask: string_attr(&attrs, "StringGenMask"),
        material_flags: u64_attr(&attrs, "MtlFlags"),
        cloak_amount: f32_attr(&attrs, "CloakAmount"),
        textures: Vec::new(),
        public_params: Vec::new(),
        extra_attributes: extra_attrs(&attrs, &known),
    })
}

fn parse_texture_start(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<MaterialTextureReference> {
    let attrs = attributes(reader, event)?;

    Ok(MaterialTextureReference {
        map: string_attr(&attrs, "Map").map_or_else(
            || MaterialTextureMap::Unknown(String::new()),
            |value| MaterialTextureMap::from_native_name(&value),
        ),
        image_asset_path: string_attr(&attrs, "File")
            .map(|value| texture_engine_asset_path(&value)),
        asset_id: string_attr(&attrs, "AssetId"),
        filter: i32_attr(&attrs, "Filter").and_then(MaterialTextureFilter::from_native_value),
        is_tile_u: bool_attr(&attrs, "IsTileU").unwrap_or(true),
        is_tile_v: bool_attr(&attrs, "IsTileV").unwrap_or(true),
        texture_type: i32_attr(&attrs, "TexType").and_then(MaterialTextureType::from_native_value),
        texture_modifier: Vec::new(),
    })
}

fn texture_engine_asset_path(source_path: &str) -> String {
    texture_source_asset_path(source_path)
}

fn texture_param_engine_asset_path(source_path: &str) -> Option<String> {
    let source_path = texture_source_asset_path(source_path);
    is_texture_source_path(&source_path).then(|| texture_engine_asset_path(&source_path))
}

fn is_texture_string_param(attrs: &BTreeMap<String, String>) -> bool {
    string_attr(attrs, "type")
        .map(|value| MaterialOverrideValueKind::from_native_name(&value))
        .unwrap_or_default()
        == MaterialOverrideValueKind::String
}

fn is_texture_source_path(path: &str) -> bool {
    matches!(
        source_extension_key(path).as_deref(),
        Some("dds" | "tif" | "tiff" | "png" | "tga" | "jpg" | "jpeg" | "bmp" | "gif")
    )
}

fn parse_public_params(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Vec<MaterialPublicParam>> {
    Ok(attributes(reader, event)?
        .into_iter()
        .map(|(name, value)| MaterialPublicParam { name, value })
        .collect())
}

fn material_emittance_from_native(value: &str) -> Option<LinearRgba> {
    let (color, intensity) = material_emittance_parts(value)?;
    Some(LinearRgba::from(color) * intensity)
}

fn material_emittance_parts(value: &str) -> Option<(Srgba, f32)> {
    let mut values = value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::parse::<f32>);

    let red = values.next()?.ok()?.max(0.0);
    let green = values.next()?.ok()?.max(0.0);
    let blue = values.next()?.ok()?.max(0.0);
    let intensity = values.next().transpose().ok()?.unwrap_or(1.0).max(0.0);

    Some((Srgba::new(red, green, blue, 1.0), intensity))
}

fn attributes(reader: &Reader<&[u8]>, event: &BytesStart<'_>) -> Result<BTreeMap<String, String>> {
    let mut attrs = BTreeMap::new();
    for attr in event.attributes().with_checks(false) {
        let attr = attr.context("read material XML attribute")?;
        let key = str::from_utf8(attr.key.as_ref())
            .context("material XML attribute name is not UTF-8")?
            .to_string();
        let value = attr
            .decoded_and_normalized_value(quick_xml::XmlVersion::default(), reader.decoder())
            .context("decode material XML attribute value")?
            .trim()
            .to_string();
        attrs.insert(key, value);
    }
    Ok(attrs)
}

fn tag_name(name: &[u8]) -> Result<String> {
    str::from_utf8(name)
        .context("material override XML tag name is not UTF-8")
        .map(str::to_string)
}

fn string_attr(attrs: &BTreeMap<String, String>, name: &str) -> Option<String> {
    attrs
        .get(name)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn f32_attr(attrs: &BTreeMap<String, String>, name: &str) -> Option<f32> {
    string_attr(attrs, name).and_then(|value| value.parse().ok())
}

fn i32_attr(attrs: &BTreeMap<String, String>, name: &str) -> Option<i32> {
    string_attr(attrs, name).and_then(|value| value.parse().ok())
}

fn u64_attr(attrs: &BTreeMap<String, String>, name: &str) -> Option<u64> {
    string_attr(attrs, name).and_then(|value| {
        value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .map_or_else(
                || value.parse().ok(),
                |hex| u64::from_str_radix(hex, 16).ok(),
            )
    })
}

fn bool_attr(attrs: &BTreeMap<String, String>, name: &str) -> Option<bool> {
    match string_attr(attrs, name)?.as_str() {
        "0" | "false" | "False" | "FALSE" => Some(false),
        "1" | "true" | "True" | "TRUE" => Some(true),
        _ => None,
    }
}

fn extra_attrs(attrs: &BTreeMap<String, String>, known: &[&str]) -> Vec<MaterialPublicParam> {
    attrs
        .iter()
        .filter(|(name, _)| !known.contains(&name.as_str()))
        .map(|(name, value)| MaterialPublicParam {
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_gem_lmbr_central::material_color;

    #[test]
    fn parses_material_group_with_submaterial_textures() {
        let xml = r#"
<Material MtlFlags="524544" vertModifType="0">
 <SubMaterials>
  <Material Name="example_hero_chest" MtlFlags="524418" Shader="Illum" GenMask="2000C8031" StringGenMask="%NORMAL_MAP%SPECULAR_MAP%" SurfaceType="mat_default" Diffuse="0.48514995,0.48514995,0.48514995,1" Specular="1,1,1,1" Opacity="1" Shininess="255" AlphaTest="0.5" LayerAct="1">
   <Textures>
    <Texture Map="Bumpmap" File="objects/characters/example/hero/textures/hero_chest_ddna.dds"/>
    <Texture Map="Diffuse" File="objects/characters/example/hero/textures/hero_chest_diff.tif" Filter="3" IsTileU="1" IsTileV="0" TexType="1">
     <TexMod TileU="2" TileV="3"/>
    </Texture>
   </Textures>
   <PublicParams SSSIndex="0" IndirectColor="0.25,0.25,0.25,0.25"/>
  </Material>
 </SubMaterials>
</Material>
"#;

        let asset = parse_material_asset_str("Materials/Foo/Example.mtl", xml).unwrap();

        assert_eq!(asset.source_path, "materials/foo/example.mtl");
        assert_eq!(asset.root.material_flags, Some(524_544));
        assert_eq!(asset.root.extra_attributes[0].name, "vertModifType");
        assert_eq!(asset.sub_materials.len(), 1);

        let sub = &asset.sub_materials[0];
        assert_eq!(sub.name.as_deref(), Some("example_hero_chest"));
        assert_eq!(sub.shader.as_deref(), Some("Illum"));
        assert_eq!(sub.surface_type.as_deref(), Some("mat_default"));
        assert_eq!(
            sub.diffuse,
            Some(material_color(
                0.485_149_95,
                0.485_149_95,
                0.485_149_95,
                1.0
            ))
        );
        assert_eq!(sub.alpha_test, Some(0.5));
        assert_eq!(sub.material_flags, Some(524_418));
        assert_eq!(sub.extra_attributes[0].name, "LayerAct");
        assert_eq!(sub.public_params[0].name, "IndirectColor");

        let diffuse = sub
            .textures
            .iter()
            .find(|texture| texture.map == MaterialTextureMap::Diffuse)
            .unwrap();
        assert_eq!(
            diffuse.image_asset_path.as_deref(),
            Some("objects/characters/example/hero/textures/hero_chest_diff.dds")
        );
        assert_eq!(diffuse.filter, Some(MaterialTextureFilter::Trilinear));
        assert!(diffuse.is_tile_u);
        assert!(!diffuse.is_tile_v);
        assert_eq!(
            diffuse.texture_type,
            Some(MaterialTextureType::TwoDimensional)
        );
        assert_eq!(diffuse.texture_modifier[0].name, "TileU");
        assert_eq!(diffuse.texture_modifier[0].value, "2");
    }

    #[test]
    fn collects_texture_dependencies_as_dds_sources() {
        let xml = r#"
<Material>
 <Textures>
  <Texture Map="Diffuse" File="Objects/Foo/Diff.tif"/>
  <Texture Map="Bumpmap" File="Objects/Foo/Normal.dds"/>
 </Textures>
</Material>
"#;

        let paths = material_texture_source_paths_str(xml).unwrap();

        assert_eq!(
            paths,
            vec!["objects/foo/diff.dds", "objects/foo/normal.dds"]
        );
    }

    #[test]
    fn parses_emittance_color_and_intensity() {
        let xml = r#"
<Material Shader="Illum" Emittance="0.5,0.25,0.125,20"/>
"#;

        let asset = parse_material_asset_str("Materials/Foo/Glow.mtl", xml).unwrap();

        assert_eq!(
            asset.root.emittance,
            Some(LinearRgba::from(Srgba::new(0.5, 0.25, 0.125, 1.0)) * 20.0)
        );
    }

    #[test]
    fn parses_material_override_asset() {
        let xml = r#"
<MaterialParamsOverride MaxTriggerDistance="close">
 <Material name="All" exclude="materials/vfx/_test/occlusion/occlusion_xray">
  <SubMaterial name="All">
   <ShaderGenerationParams>
    <Rim_Diffuse_Lighting enabled="true"/>
   </ShaderGenerationParams>
   <TextureMaps>
    <CustomSecondaryMap>
     <param name="start" type="string" value="textures/defaults/noise_03_diff.dds"/>
    </CustomSecondaryMap>
   </TextureMaps>
   <ShaderParams>
    <Rim_Fill_Intensity>
     <param name="start" type="float" value="0.7"/>
    </Rim_Fill_Intensity>
   </ShaderParams>
  </SubMaterial>
 </Material>
</MaterialParamsOverride>
"#;

        let asset =
            parse_material_override_asset_str("Libs/MaterialOverrides/Example/Rim.xml", xml)
                .unwrap();

        assert_eq!(asset.source_path, "libs/materialoverrides/example/rim.xml");
        assert_eq!(asset.max_trigger_distance.as_deref(), Some("close"));
        let material = &asset.materials[0];
        assert_eq!(material.name.as_deref(), Some("All"));
        assert_eq!(
            material.exclude.as_deref(),
            Some("materials/vfx/_test/occlusion/occlusion_xray")
        );
        let sub_material = &material.sub_materials[0];
        assert_eq!(sub_material.name.as_deref(), Some("All"));
        assert_eq!(
            sub_material.shader_generation_params[0].name,
            "Rim_Diffuse_Lighting"
        );
        assert!(sub_material.shader_generation_params[0].enabled);
        assert_eq!(sub_material.texture_maps[0].name, "CustomSecondaryMap");
        assert_eq!(
            sub_material.texture_maps[0].params[0].value,
            "textures/defaults/noise_03_diff.dds"
        );
        assert_eq!(sub_material.shader_params[0].name, "Rim_Fill_Intensity");
        assert_eq!(
            sub_material.shader_params[0].params[0].value_kind,
            MaterialOverrideValueKind::Float
        );
    }

    #[test]
    fn collects_material_override_texture_dependencies_as_dds_sources() {
        let xml = r#"
<MaterialParamsOverride>
 <Material name="All">
  <SubMaterial name="All">
   <TextureMaps>
    <CustomSecondaryMap>
     <param name="start" type="string" value="textures/defaults/noise_03_diff.tif"/>
    </CustomSecondaryMap>
   </TextureMaps>
   <ShaderParams>
    <NotATexture>
     <param name="start" type="string" value="plain text"/>
    </NotATexture>
   </ShaderParams>
  </SubMaterial>
 </Material>
</MaterialParamsOverride>
"#;

        let paths = material_override_texture_source_paths_str(xml).unwrap();

        assert_eq!(paths, vec!["textures/defaults/noise_03_diff.dds"]);
    }
}
