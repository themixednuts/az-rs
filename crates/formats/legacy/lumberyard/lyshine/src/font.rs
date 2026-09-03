//! `CryFont` XML font descriptor parsing.
//!
//! Follows Lumberyard's `dev/Code/CryEngine/CryFont/FFontXML.cpp`.

use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt, io,
    num::{ParseFloatError, ParseIntError},
    path::{Path, PathBuf},
    str,
};

use bevy_color::Srgba;
use glam::{UVec2, Vec2};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use thiserror::Error;
use uuid::{Uuid, uuid};

const FONT_SHADER: &[u8] = b"fontshader";
const FONT: &[u8] = b"font";
const EFFECT: &[u8] = b"effect";
const EFFECT_FILE: &[u8] = b"effectfile";
const PASS: &[u8] = b"pass";
const COLOR: &[u8] = b"color";
const POS: &[u8] = b"pos";
const OFFSET: &[u8] = b"offset";
const BLEND: &[u8] = b"blend";
const BLENDING: &[u8] = b"blending";
const SIZE_CACHE: &[u8] = b"sizecache";
const FONT_CACHE: &[u8] = b"fontcache";

/// File extension used by `LyShine` `.font` descriptors.
pub const FONT_DESCRIPTOR_EXTENSION: &str = "font";

/// `LyShine::FontAsset`.
pub const FONT_ASSET_TYPE_ID: Uuid = uuid!("57767d37-0ebe-43be-8f60-ab36d2056ef8");

/// `AzFramework::SimpleAssetReference<LyShine::FontAsset>`.
pub const FONT_ASSET_REFERENCE_TYPE_ID: Uuid = uuid!("d6342379-a5fa-4b18-b890-702c2fe99a5a");

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FontInspectionError {
    #[error("read {path:?}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse LyShine font {path:?}: {source}")]
    Parse {
        path: PathBuf,
        source: FontParseError,
    },
}

/// Summary returned after visiting a `.font` descriptor.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FontStats {
    pub descriptors: usize,
    pub fonts: usize,
    pub effect_files: usize,
    pub effects: usize,
    pub passes: usize,
    pub colors: usize,
    pub offsets: usize,
    pub blends: usize,
    pub size_caches: usize,
    pub font_caches: usize,
}

/// Deterministic summary for one `.font` descriptor.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FontSummary {
    pub stats: FontStats,
    pub size_behavior: Option<FontSizeBehavior>,
}

impl fmt::Display for FontSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stats = self.stats;
        write!(
            f,
            "{} fonts, {} effect files, {} effects, {} passes, {} cached sizes",
            stats.fonts, stats.effect_files, stats.effects, stats.passes, stats.font_caches
        )
    }
}

/// Aggregate summary across many `.font` descriptors.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FontTotals {
    pub files: usize,
    pub fonts: usize,
    pub effect_files: usize,
    pub effects: usize,
    pub passes: usize,
    pub colors: usize,
    pub offsets: usize,
    pub blends: usize,
    pub size_caches: usize,
    pub font_caches: usize,
    pub by_size_behavior: BTreeMap<FontSizeBehavior, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFileSummary {
    pub source: String,
    pub summary: FontSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FontInspection {
    pub rows: Vec<FontFileSummary>,
    pub totals: FontTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct FontInspectionReport<'a> {
    inspection: &'a FontInspection,
    limit: usize,
}

impl FontTotals {
    pub fn add_summary(&mut self, summary: FontSummary) {
        self.add(summary.stats, summary.size_behavior);
    }

    pub fn add(&mut self, stats: FontStats, size_behavior: Option<FontSizeBehavior>) {
        self.files += 1;
        self.fonts += stats.fonts;
        self.effect_files += stats.effect_files;
        self.effects += stats.effects;
        self.passes += stats.passes;
        self.colors += stats.colors;
        self.offsets += stats.offsets;
        self.blends += stats.blends;
        self.size_caches += stats.size_caches;
        self.font_caches += stats.font_caches;
        if let Some(size_behavior) = size_behavior {
            *self.by_size_behavior.entry(size_behavior).or_default() += 1;
        }
    }
}

impl FontInspection {
    pub fn add_file_summary(&mut self, row: FontFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> FontInspectionReport<'_> {
        FontInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for FontTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  fonts: {}", self.fonts)?;
        writeln!(f, "  effect files: {}", self.effect_files)?;
        writeln!(f, "  effects: {}", self.effects)?;
        writeln!(f, "  passes: {}", self.passes)?;
        writeln!(f, "  colors: {}", self.colors)?;
        writeln!(f, "  offsets: {}", self.offsets)?;
        writeln!(f, "  blends: {}", self.blends)?;
        writeln!(f, "  size caches: {}", self.size_caches)?;
        writeln!(f, "  font caches: {}", self.font_caches)?;
        for (behavior, count) in &self.by_size_behavior {
            writeln!(f, "  {behavior:?}: {count}")?;
        }
        Ok(())
    }
}

impl fmt::Display for FontInspectionReport<'_> {
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

/// Borrowed item produced by the streaming font visitor.
#[derive(Debug, Clone, PartialEq)]
pub enum FontItem<'a> {
    Descriptor,
    Font(FontFace<'a>),
    EffectFile(Cow<'a, str>),
    Effect(FontEffectRef<'a>),
    Pass,
    Color(Srgba),
    Offset(Vec2),
    Blend(FontBlend),
    SizeCache,
    FontCache(u32),
}

/// Parse a `.font` descriptor and return the summary used by inspection tools.
///
/// # Errors
///
/// Returns any error [`visit_font_shader`] returns.
pub fn summarize_font_shader(bytes: &[u8]) -> Result<FontSummary, FontParseError> {
    let mut size_behavior = None;
    let stats = visit_font_shader(bytes, |item| {
        if let FontItem::Font(font) = item {
            size_behavior = font.size_behavior;
        }
        Ok(())
    })?;
    Ok(FontSummary {
        stats,
        size_behavior,
    })
}

/// Inspect one `.font` descriptor with its display source.
///
/// # Errors
///
/// Returns any error [`summarize_font_shader`] returns.
pub fn inspect_font_shader_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<FontFileSummary, FontParseError> {
    Ok(FontFileSummary {
        source: path.as_ref().display().to_string(),
        summary: summarize_font_shader(bytes)?,
    })
}

/// Read and summarize one `.font` descriptor from disk.
///
/// # Errors
///
/// Returns [`FontInspectionError::Read`] when `path` cannot be read, and
/// [`FontInspectionError::Parse`] when its contents are not a valid
/// `fontshader` descriptor.
pub fn inspect_font_shader_path(
    path: impl AsRef<Path>,
) -> Result<FontFileSummary, FontInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| FontInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_font_shader_file(path, &bytes).map_err(|source| FontInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Read and aggregate every `.font` descriptor in `paths`.
///
/// # Errors
///
/// Returns the first error [`inspect_font_shader_path`] returns; the walk
/// stops at that path and the partial inspection is discarded.
pub fn inspect_font_shader_files<I, P>(paths: I) -> Result<FontInspection, FontInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = FontInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_font_shader_path(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub const fn is_font_descriptor_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(FONT_DESCRIPTOR_EXTENSION)
}

#[must_use]
pub fn is_font_descriptor_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| is_font_descriptor_extension(extension))
}

#[must_use]
pub fn is_font_descriptor_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_font_descriptor_extension)
}

/// Parsed `.font` descriptor.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FontShader<'a> {
    pub fonts: Vec<FontFace<'a>>,
    pub effect_files: Vec<Cow<'a, str>>,
    pub effects: Vec<FontEffect<'a>>,
    pub font_caches: Vec<u32>,
}

/// Font face configuration from a `<font>` element.
#[derive(Debug, Clone, PartialEq)]
pub struct FontFace<'a> {
    pub path: Cow<'a, str>,
    pub texture_size: Option<UVec2>,
    pub slot_size: Option<UVec2>,
    pub size_ratio: Option<f32>,
    pub font_size: Option<u32>,
    pub no_rescale: Option<f32>,
    pub size_behavior: Option<FontSizeBehavior>,
    pub hint_style: Option<FontHintStyle>,
    pub hint_behavior: Option<FontHintBehavior>,
    pub smoothing: FontSmoothing,
}

impl FontFace<'_> {
    #[must_use]
    pub fn into_owned(self) -> FontFace<'static> {
        FontFace {
            path: Cow::Owned(self.path.into_owned()),
            texture_size: self.texture_size,
            slot_size: self.slot_size,
            size_ratio: self.size_ratio,
            font_size: self.font_size,
            no_rescale: self.no_rescale,
            size_behavior: self.size_behavior,
            hint_style: self.hint_style,
            hint_behavior: self.hint_behavior,
            smoothing: self.smoothing,
        }
    }
}

/// Font rasterization behavior selected by `sizebehavior`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontSizeBehavior {
    Rerender,
    SizeCache,
}

impl FontSizeBehavior {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rerender => "rerender",
            Self::SizeCache => "sizecache",
        }
    }
}

/// `FreeType` hint style selected by `hintstyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontHintStyle {
    Normal,
    Light,
}

impl FontHintStyle {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Light => "light",
        }
    }
}

/// `FreeType` hint behavior selected by `hintbehavior`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontHintBehavior {
    Default,
    AutoHint,
    NoHinting,
}

impl FontHintBehavior {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AutoHint => "autohint",
            Self::NoHinting => "nohinting",
        }
    }
}

/// Font smoothing settings from `smooth` and `smooth_amount`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FontSmoothing {
    pub method: Option<FontSmoothMethod>,
    pub amount: Option<i32>,
}

/// Font smoothing method selected by `smooth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontSmoothMethod {
    None,
    Blur,
    Supersample,
}

impl FontSmoothMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Blur => "blur",
            Self::Supersample => "supersample",
        }
    }
}

/// Borrowed effect declaration from an `<effect>` element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontEffectRef<'a> {
    pub name: Option<Cow<'a, str>>,
}

/// Parsed font effect with rendering passes.
#[derive(Debug, Clone, PartialEq)]
pub struct FontEffect<'a> {
    pub name: Option<Cow<'a, str>>,
    pub passes: Vec<FontPass>,
}

/// Parsed rendering pass.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FontPass {
    pub color: Option<Srgba>,
    pub offset: Option<Vec2>,
    pub blend: Option<FontBlend>,
}

/// Blend settings from `<blend>` or `<blending>`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FontBlend {
    pub source: Option<FontBlendMode>,
    pub destination: Option<FontBlendMode>,
    pub kind: Option<FontBlendKind>,
}

/// Blend mode names accepted by the `CryFont` loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontBlendMode {
    Zero,
    One,
    SourceAlpha,
    InverseSourceAlpha,
    DestinationAlpha,
    InverseDestinationAlpha,
    DestinationColor,
    SourceColor,
    InverseDestinationColor,
    InverseSourceColor,
}

impl FontBlendMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::One => "one",
            Self::SourceAlpha => "srcalpha",
            Self::InverseSourceAlpha => "invsrcalpha",
            Self::DestinationAlpha => "dstalpha",
            Self::InverseDestinationAlpha => "invdstalpha",
            Self::DestinationColor => "dstcolor",
            Self::SourceColor => "srccolor",
            Self::InverseDestinationColor => "invdstcolor",
            Self::InverseSourceColor => "invsrccolor",
        }
    }
}

/// Named blend presets accepted by the `CryFont` loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontBlendKind {
    Modulate,
    Additive,
}

impl FontBlendKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Modulate => "modulate",
            Self::Additive => "additive",
        }
    }
}

/// Parse a `.font` descriptor into structured data.
///
/// # Errors
///
/// Returns any error [`visit_font_shader`] returns, plus
/// [`FontParseError::PassWithoutEffect`] for a `<pass>` outside an `<effect>`
/// and [`FontParseError::PassDataWithoutPass`] for `<color>`, `<pos>`, or
/// `<blend>` outside a `<pass>`.
pub fn parse_font_shader(bytes: &[u8]) -> Result<FontShader<'static>, FontParseError> {
    let mut builder = FontShaderBuilder::default();
    visit_font_shader(bytes, |item| builder.visit(item))?;
    Ok(builder.finish())
}

/// Parse a `.font` descriptor with a streaming visitor.
///
/// # Errors
///
/// Returns [`FontParseError::Utf8`] when `bytes` is not UTF-8,
/// [`FontParseError::Xml`] or [`FontParseError::Attribute`] for malformed XML,
/// [`FontParseError::MissingRoot`] when no `<fontshader>` element was seen,
/// [`FontParseError::UnexpectedElement`] for an element outside the `CryFont`
/// schema, [`FontParseError::MissingAttribute`] for a required `path` or
/// `size`, [`FontParseError::InvalidEnum`] for an unrecognized
/// `sizebehavior`/`hintstyle`/`hintbehavior`/`smooth`/blend name, and
/// [`FontParseError::InvalidFloat`] or [`FontParseError::InvalidInteger`] for
/// an attribute that does not parse. Any error `visitor` returns is
/// propagated unchanged.
pub fn visit_font_shader<F>(bytes: &[u8], mut visitor: F) -> Result<FontStats, FontParseError>
where
    F: FnMut(FontItem<'_>) -> Result<(), FontParseError>,
{
    let xml = str::from_utf8(bytes)?;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut state = FontState::default();

    loop {
        match reader.read_event()? {
            Event::Start(event) | Event::Empty(event) => {
                state.visit_start(&reader, &event, &mut visitor)?;
            }
            Event::Eof => break,
            Event::End(_)
            | Event::Decl(_)
            | Event::PI(_)
            | Event::DocType(_)
            | Event::Text(_)
            | Event::CData(_)
            | Event::Comment(_)
            | Event::GeneralRef(_) => {}
        }
    }

    if state.stats.descriptors == 0 {
        return Err(FontParseError::MissingRoot);
    }

    Ok(state.stats)
}

#[derive(Debug, Default)]
struct FontState {
    stats: FontStats,
}

impl FontState {
    fn visit_start<F>(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
        visitor: &mut F,
    ) -> Result<(), FontParseError>
    where
        F: FnMut(FontItem<'_>) -> Result<(), FontParseError>,
    {
        let name = event.local_name();
        match name.as_ref() {
            FONT_SHADER => {
                self.stats.descriptors += 1;
                visitor(FontItem::Descriptor)
            }
            FONT => {
                let font = parse_font(reader, event)?;
                self.stats.fonts += 1;
                visitor(FontItem::Font(font))
            }
            EFFECT_FILE => {
                let path = required_attr(reader, event, b"path", "effectfile.path")?;
                self.stats.effect_files += 1;
                visitor(FontItem::EffectFile(path))
            }
            EFFECT => {
                let effect = FontEffectRef {
                    name: attr_value(reader, event, b"name")?,
                };
                self.stats.effects += 1;
                visitor(FontItem::Effect(effect))
            }
            PASS => {
                self.stats.passes += 1;
                visitor(FontItem::Pass)
            }
            COLOR => {
                let color = parse_color(reader, event)?;
                self.stats.colors += 1;
                visitor(FontItem::Color(color))
            }
            POS | OFFSET => {
                let offset = parse_offset(reader, event)?;
                self.stats.offsets += 1;
                visitor(FontItem::Offset(offset))
            }
            BLEND | BLENDING => {
                let blend = parse_blend(reader, event)?;
                self.stats.blends += 1;
                visitor(FontItem::Blend(blend))
            }
            SIZE_CACHE => {
                self.stats.size_caches += 1;
                visitor(FontItem::SizeCache)
            }
            FONT_CACHE => {
                let size = required_attr_u32(reader, event, b"size", "fontcache.size")?;
                self.stats.font_caches += 1;
                visitor(FontItem::FontCache(size))
            }
            _ => Err(FontParseError::UnexpectedElement {
                element: String::from_utf8_lossy(name.as_ref()).into_owned(),
            }),
        }
    }
}

#[derive(Debug, Default)]
struct FontShaderBuilder {
    shader: FontShader<'static>,
}

impl FontShaderBuilder {
    fn visit(&mut self, item: FontItem<'_>) -> Result<(), FontParseError> {
        match item {
            FontItem::Descriptor | FontItem::SizeCache => {}
            FontItem::Font(font) => self.shader.fonts.push(font.into_owned()),
            FontItem::EffectFile(path) => {
                self.shader.effect_files.push(Cow::Owned(path.into_owned()));
            }
            FontItem::Effect(effect) => self.shader.effects.push(FontEffect {
                name: effect.name.map(|name| Cow::Owned(name.into_owned())),
                passes: Vec::new(),
            }),
            FontItem::Pass => {
                self.last_effect_mut()?.passes.push(FontPass::default());
            }
            FontItem::Color(color) => {
                self.last_pass_mut()?.color = Some(color);
            }
            FontItem::Offset(offset) => {
                self.last_pass_mut()?.offset = Some(offset);
            }
            FontItem::Blend(blend) => {
                self.last_pass_mut()?.blend = Some(blend);
            }
            FontItem::FontCache(size) => self.shader.font_caches.push(size),
        }
        Ok(())
    }

    fn finish(self) -> FontShader<'static> {
        self.shader
    }

    fn last_effect_mut(&mut self) -> Result<&mut FontEffect<'static>, FontParseError> {
        self.shader
            .effects
            .last_mut()
            .ok_or(FontParseError::PassWithoutEffect)
    }

    fn last_pass_mut(&mut self) -> Result<&mut FontPass, FontParseError> {
        self.last_effect_mut()?
            .passes
            .last_mut()
            .ok_or(FontParseError::PassDataWithoutPass)
    }
}

fn parse_font<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
) -> Result<FontFace<'a>, FontParseError> {
    let path = required_attr(reader, event, b"path", "font.path")?;
    let width = attr_u32(reader, event, b"w", "font.w")?;
    let height = attr_u32(reader, event, b"h", "font.h")?;
    let width_slots = attr_u32(reader, event, b"widthslots", "font.widthslots")?;
    let height_slots = attr_u32(reader, event, b"heightslots", "font.heightslots")?;

    Ok(FontFace {
        path,
        texture_size: optional_uvec2(width, height),
        slot_size: optional_uvec2(width_slots, height_slots),
        size_ratio: attr_f32(reader, event, b"sizeratio", "font.sizeratio")?,
        font_size: attr_u32(reader, event, b"fontsize", "font.fontsize")?,
        no_rescale: attr_f32(reader, event, b"norescale", "font.norescale")?,
        size_behavior: attr_value(reader, event, b"sizebehavior")?
            .map(|value| parse_size_behavior(value.as_ref()))
            .transpose()?,
        hint_style: attr_value(reader, event, b"hintstyle")?
            .map(|value| parse_hint_style(value.as_ref()))
            .transpose()?,
        hint_behavior: attr_value(reader, event, b"hintbehavior")?
            .map(|value| parse_hint_behavior(value.as_ref()))
            .transpose()?,
        smoothing: FontSmoothing {
            method: attr_value(reader, event, b"smooth")?
                .map(|value| parse_smooth_method(value.as_ref()))
                .transpose()?,
            amount: attr_i32(reader, event, b"smooth_amount", "font.smooth_amount")?,
        },
    })
}

fn parse_color(reader: &Reader<&[u8]>, event: &BytesStart<'_>) -> Result<Srgba, FontParseError> {
    Ok(Srgba::new(
        attr_f32(reader, event, b"r", "color.r")?.unwrap_or(1.0),
        attr_f32(reader, event, b"g", "color.g")?.unwrap_or(1.0),
        attr_f32(reader, event, b"b", "color.b")?.unwrap_or(1.0),
        attr_f32(reader, event, b"a", "color.a")?.unwrap_or(1.0),
    ))
}

fn parse_offset(reader: &Reader<&[u8]>, event: &BytesStart<'_>) -> Result<Vec2, FontParseError> {
    Ok(Vec2::new(
        attr_f32(reader, event, b"x", "offset.x")?.unwrap_or(0.0),
        attr_f32(reader, event, b"y", "offset.y")?.unwrap_or(0.0),
    ))
}

fn parse_blend(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<FontBlend, FontParseError> {
    Ok(FontBlend {
        source: attr_value(reader, event, b"src")?
            .map(|value| parse_blend_mode(value.as_ref()))
            .transpose()?,
        destination: attr_value(reader, event, b"dst")?
            .map(|value| parse_blend_mode(value.as_ref()))
            .transpose()?,
        kind: attr_value(reader, event, b"type")?
            .map(|value| parse_blend_kind(value.as_ref()))
            .transpose()?,
    })
}

const fn optional_uvec2(x: Option<u32>, y: Option<u32>) -> Option<UVec2> {
    match (x, y) {
        (Some(x), Some(y)) => Some(UVec2::new(x, y)),
        _ => None,
    }
}

fn parse_size_behavior(value: &str) -> Result<FontSizeBehavior, FontParseError> {
    match value {
        "rerender" => Ok(FontSizeBehavior::Rerender),
        "sizecache" => Ok(FontSizeBehavior::SizeCache),
        _ => Err(FontParseError::InvalidEnum {
            name: "font.sizebehavior",
            value: value.to_string(),
        }),
    }
}

fn parse_hint_style(value: &str) -> Result<FontHintStyle, FontParseError> {
    match value {
        "normal" => Ok(FontHintStyle::Normal),
        "light" => Ok(FontHintStyle::Light),
        _ => Err(FontParseError::InvalidEnum {
            name: "font.hintstyle",
            value: value.to_string(),
        }),
    }
}

fn parse_hint_behavior(value: &str) -> Result<FontHintBehavior, FontParseError> {
    match value {
        "default" => Ok(FontHintBehavior::Default),
        "autohint" => Ok(FontHintBehavior::AutoHint),
        "nohinting" => Ok(FontHintBehavior::NoHinting),
        _ => Err(FontParseError::InvalidEnum {
            name: "font.hintbehavior",
            value: value.to_string(),
        }),
    }
}

fn parse_smooth_method(value: &str) -> Result<FontSmoothMethod, FontParseError> {
    match value {
        "none" => Ok(FontSmoothMethod::None),
        "blur" => Ok(FontSmoothMethod::Blur),
        "supersample" => Ok(FontSmoothMethod::Supersample),
        _ => Err(FontParseError::InvalidEnum {
            name: "font.smooth",
            value: value.to_string(),
        }),
    }
}

fn parse_blend_mode(value: &str) -> Result<FontBlendMode, FontParseError> {
    match value {
        "zero" => Ok(FontBlendMode::Zero),
        "one" => Ok(FontBlendMode::One),
        "srcalpha" | "src_alpha" => Ok(FontBlendMode::SourceAlpha),
        "invsrcalpha" | "inv_src_alpha" => Ok(FontBlendMode::InverseSourceAlpha),
        "dstalpha" | "dst_alpha" => Ok(FontBlendMode::DestinationAlpha),
        "invdstalpha" | "inv_dst_alpha" => Ok(FontBlendMode::InverseDestinationAlpha),
        "dstcolor" | "dst_color" => Ok(FontBlendMode::DestinationColor),
        "srccolor" | "src_color" => Ok(FontBlendMode::SourceColor),
        "invdstcolor" | "inv_dst_color" => Ok(FontBlendMode::InverseDestinationColor),
        "invsrccolor" | "inv_src_color" => Ok(FontBlendMode::InverseSourceColor),
        _ => Err(FontParseError::InvalidEnum {
            name: "blend mode",
            value: value.to_string(),
        }),
    }
}

fn parse_blend_kind(value: &str) -> Result<FontBlendKind, FontParseError> {
    match value {
        "modulate" => Ok(FontBlendKind::Modulate),
        "additive" => Ok(FontBlendKind::Additive),
        _ => Err(FontParseError::InvalidEnum {
            name: "blend.type",
            value: value.to_string(),
        }),
    }
}

fn required_attr<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
    key: &[u8],
    name: &'static str,
) -> Result<Cow<'a, str>, FontParseError> {
    attr_value(reader, event, key)?.ok_or(FontParseError::MissingAttribute(name))
}

fn attr_value<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
    key: &[u8],
) -> Result<Option<Cow<'a, str>>, FontParseError> {
    for attribute in event.attributes() {
        let attribute = attribute?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.decoded_and_normalized_value(
                quick_xml::XmlVersion::default(),
                reader.decoder(),
            )?));
        }
    }
    Ok(None)
}

fn required_attr_u32(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<u32, FontParseError> {
    attr_u32(reader, event, key, name)?.ok_or(FontParseError::MissingAttribute(name))
}

fn attr_u32(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<Option<u32>, FontParseError> {
    let Some(value) = attr_value(reader, event, key)? else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse()
        .map(Some)
        .map_err(|source| FontParseError::InvalidInteger {
            name,
            value: value.to_string(),
            source,
        })
}

fn attr_i32(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<Option<i32>, FontParseError> {
    let Some(value) = attr_value(reader, event, key)? else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse()
        .map(Some)
        .map_err(|source| FontParseError::InvalidInteger {
            name,
            value: value.to_string(),
            source,
        })
}

fn attr_f32(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<Option<f32>, FontParseError> {
    let Some(value) = attr_value(reader, event, key)? else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse()
        .map(Some)
        .map_err(|source| FontParseError::InvalidFloat {
            name,
            value: value.to_string(),
            source,
        })
}

/// Errors returned while parsing a `.font` descriptor.
#[derive(Debug, Error)]
pub enum FontParseError {
    #[error("expected fontshader root")]
    MissingRoot,
    #[error("unexpected element `{element}`")]
    UnexpectedElement { element: String },
    #[error("missing required attribute `{0}`")]
    MissingAttribute(&'static str),
    #[error("pass found before effect")]
    PassWithoutEffect,
    #[error("pass data found before pass")]
    PassDataWithoutPass,
    #[error("invalid float `{value}` in `{name}`")]
    InvalidFloat {
        name: &'static str,
        value: String,
        #[source]
        source: ParseFloatError,
    },
    #[error("invalid integer `{value}` in `{name}`")]
    InvalidInteger {
        name: &'static str,
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error("invalid value `{value}` for `{name}`")]
    InvalidEnum { name: &'static str, value: String },
    #[error("xml parse error")]
    Xml(#[from] quick_xml::Error),
    #[error("xml attribute error")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    #[error("asset is not utf-8")]
    Utf8(#[from] str::Utf8Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_size_cached_font_descriptor() {
        let xml = br#"
            <fontshader>
                <font path="IMFePIrm28P.ttf" fontsize="36" widthslots="24" heightslots="16" sizebehavior="sizecache"/>
                <effectfile path="LyShineUI/Fonts/_SharedFontEffects.xml"/>
                <sizecache>
                    <fontcache size="12"/>
                    <fontcache size="48"/>
                </sizecache>
            </fontshader>
        "#;

        let shader = parse_font_shader(xml).unwrap();

        assert_eq!(shader.fonts.len(), 1);
        assert_eq!(shader.fonts[0].path, "IMFePIrm28P.ttf");
        assert_eq!(shader.fonts[0].font_size, Some(36));
        assert_eq!(shader.fonts[0].slot_size, Some(UVec2::new(24, 16)));
        assert_eq!(
            shader.fonts[0].size_behavior,
            Some(FontSizeBehavior::SizeCache)
        );
        assert_eq!(
            shader.effect_files,
            ["LyShineUI/Fonts/_SharedFontEffects.xml"]
        );
        assert_eq!(shader.font_caches, [12, 48]);

        let summary = summarize_font_shader(xml).unwrap();
        assert_eq!(summary.stats.fonts, 1);
        assert_eq!(summary.stats.effect_files, 1);
        assert_eq!(summary.stats.font_caches, 2);
        assert_eq!(summary.size_behavior, Some(FontSizeBehavior::SizeCache));

        let mut totals = FontTotals::default();
        totals.add_summary(summary);
        assert_eq!(totals.files, 1);
        assert_eq!(totals.font_caches, 2);
        assert_eq!(
            totals.by_size_behavior.get(&FontSizeBehavior::SizeCache),
            Some(&1)
        );
        assert_eq!(
            summary.to_string(),
            "1 fonts, 1 effect files, 0 effects, 0 passes, 2 cached sizes"
        );
        assert_eq!(
            totals.to_string(),
            "  files: 1\n  fonts: 1\n  effect files: 1\n  effects: 0\n  passes: 0\n  colors: 0\n  offsets: 0\n  blends: 0\n  size caches: 1\n  font caches: 2\n  SizeCache: 1\n"
        );

        let row = inspect_font_shader_file("lyshineui/fonts/default.font", xml).unwrap();
        let mut inspection = FontInspection::default();
        inspection.add_file_summary(row);
        assert_eq!(
            inspection.report(20).to_string(),
            "lyshineui/fonts/default.font: 1 fonts, 1 effect files, 0 effects, 0 passes, 2 cached sizes\n  files: 1\n  fonts: 1\n  effect files: 1\n  effects: 0\n  passes: 0\n  colors: 0\n  offsets: 0\n  blends: 0\n  size caches: 1\n  font caches: 2\n  SizeCache: 1\n"
        );

        let path = std::env::temp_dir().join(format!(
            "az-rs-lyshine-font-{}-default.font",
            std::process::id()
        ));
        std::fs::write(&path, xml).expect("write font descriptor");
        let inspection = inspect_font_shader_files([&path]).expect("inspect font files");
        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.fonts, 1);
        assert_eq!(inspection.totals.font_caches, 2);
        std::fs::remove_file(path).expect("remove font descriptor");
    }

    #[test]
    fn parses_inline_effect_passes() {
        let xml = br#"
            <fontshader>
                <font path="VeraMono.ttf" w="512" h="256"/>
                <effect name="hud">
                    <pass>
                        <color r="0" g="0.25" b="0.5" a="1"/>
                        <pos x="1" y="2"/>
                        <blend type="additive"/>
                    </pass>
                </effect>
            </fontshader>
        "#;

        let shader = parse_font_shader(xml).unwrap();

        assert_eq!(shader.fonts[0].texture_size, Some(UVec2::new(512, 256)));
        assert_eq!(shader.effects.len(), 1);
        assert_eq!(shader.effects[0].name.as_deref(), Some("hud"));
        let pass = &shader.effects[0].passes[0];
        assert_eq!(pass.color, Some(Srgba::new(0.0, 0.25, 0.5, 1.0)));
        assert_eq!(pass.offset, Some(Vec2::new(1.0, 2.0)));
        assert_eq!(
            pass.blend,
            Some(FontBlend {
                source: None,
                destination: None,
                kind: Some(FontBlendKind::Additive),
            })
        );
    }

    #[test]
    fn recognizes_font_descriptor_paths() {
        assert!(is_font_descriptor_name("nimbus.FONT"));
        assert!(is_font_descriptor_path(Path::new(
            "lyshineui/fonts/nimbus.font"
        )));
        assert!(!is_font_descriptor_name("nimbus.otf"));
    }
}
