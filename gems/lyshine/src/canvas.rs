//! `LyShine` canvas asset data and Bevy loading.
//!
//! O3DE reference: `Gems/LyShine/Code/Source/UiCanvasFileObject.cpp`.

use std::io::{Read, Write};

use az_framework::ScriptComponent;
use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::color::LinearRgba;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Current UI canvas asset schema version.
///
/// v5 removes project-defined canvas extensions from the engine product. This
/// changes the canvas header, so older products must be rebuilt.
pub const UI_CANVAS_ASSET_VERSION: u32 = 5;

/// Oldest UI canvas asset schema version this loader can still read.
pub const UI_CANVAS_ASSET_MIN_VERSION: u32 = 5;

/// UI canvas asset binary marker.
pub const UI_CANVAS_ASSET_MAGIC: &[u8; 8] = b"AZUICAN\0";

/// File extensions claimed by the `LyShine` canvas asset loader.
///
/// The product preserves Lumberyard's `.uicanvas` or `.dynamicuicanvas`
/// extension. Both extensions contain the same Azoth canvas product format.
pub const UI_CANVAS_ASSET_EXTENSIONS: &[&str] = &["uicanvas", "dynamicuicanvas"];

/// Script payload stored on UI entities.
pub type UiScript = ScriptComponent;

/// Native `LyShine` canvas asset.
#[derive(Asset, TypePath, Debug, Clone, Default, PartialEq)]
pub struct UiCanvasAsset {
    pub version: u32,
    pub canvas: UiCanvas,
    pub entities: Vec<UiEntity>,
}

impl UiCanvasAsset {
    #[must_use]
    pub const fn new(canvas: UiCanvas, entities: Vec<UiEntity>) -> Self {
        Self {
            version: UI_CANVAS_ASSET_VERSION,
            canvas,
            entities,
        }
    }

    #[must_use]
    pub fn is_engine_asset_path(path: &str) -> bool {
        UI_CANVAS_ASSET_EXTENSIONS
            .iter()
            .any(|extension| path.ends_with(extension))
    }
}

/// Stable entity identifier inside one UI canvas.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Reflect,
    Serialize,
    Deserialize,
)]
#[reflect(Serialize, Deserialize)]
pub struct UiEntityId(u64);

impl UiEntityId {
    #[inline]
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

impl From<u64> for UiEntityId {
    #[inline]
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

impl From<UiEntityId> for u64 {
    #[inline]
    fn from(value: UiEntityId) -> Self {
        value.0
    }
}

/// Canvas-level settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiCanvas {
    pub unique_id: u64,
    pub root_entity: UiEntityId,
    pub first_hover_entity: UiEntityId,
    pub tooltip_display_entity: UiEntityId,
    pub last_element_id: u32,
    pub size: Vec2,
    pub draw_order: i32,
    pub flags: UiCanvasFlags,
    pub render_target_name: Option<String>,
    pub texture_atlases: Vec<String>,
}

/// Boolean canvas settings.
// One field per Lumberyard `UiCanvasComponent` flag, in the order
// `canvas_binary` reads and writes them; a bitflags type would change
// both the on-disk shape and the serde product.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct UiCanvasFlags {
    pub snap_enabled: bool,
    pub pixel_aligned: bool,
    pub render_to_texture: bool,
    pub transform_update_optimize_enabled: bool,
    pub optimize_for_frequent_updates: bool,
    pub position_input_supported: bool,
    pub navigation_supported: bool,
    pub always_allows_hover: bool,
    pub ignore_scroll_hover: bool,
    pub enter_handling_disabled: bool,
    pub guides_locked: bool,
}

/// One entity in a canvas.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiEntity {
    pub entity_id: UiEntityId,
    pub name: Option<String>,
    pub dependency_ready: bool,
    pub runtime_active: bool,
    pub transform: Option<UiTransform2d>,
    pub element: Option<UiElement>,
    pub image: Option<UiImage>,
    pub text: Option<UiText>,
    pub button: Option<UiButton>,
    pub fader: Option<UiFader>,
    pub mask: Option<UiMask>,
    pub layout_row: Option<UiLayoutAxis>,
    pub layout_column: Option<UiLayoutAxis>,
    pub layout_grid: Option<UiLayoutGrid>,
    pub layout_cell: Option<UiLayoutCell>,
    pub script: Option<UiScript>,
    pub components: Vec<UiComponentKind>,
}

impl UiEntity {
    #[must_use]
    pub fn new(entity_id: UiEntityId) -> Self {
        Self {
            entity_id,
            ..Default::default()
        }
    }
}

/// Component categories preserved by the native canvas asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum UiComponentKind {
    Canvas,
    Transform2d,
    Element,
    Image,
    Text,
    Button,
    Interactable,
    Fader,
    Mask,
    LayoutRow,
    LayoutColumn,
    LayoutGrid,
    LayoutCell,
    Script,
    Other,
}

/// Four-edge UI rectangle data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl UiRect {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    #[inline]
    #[must_use]
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

/// 2D transform settings for a UI element.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiTransform2d {
    pub anchors: UiRect,
    pub offsets: UiRect,
    pub pivot: Vec2,
    pub rotation: f32,
    pub scale: Vec2,
    pub scale_to_device: bool,
    pub compute_transform_when_hidden: bool,
}

impl Default for UiTransform2d {
    fn default() -> Self {
        Self {
            anchors: UiRect::ZERO,
            offsets: UiRect::ZERO,
            pivot: Vec2::ZERO,
            rotation: 0.0,
            scale: Vec2::ONE,
            scale_to_device: false,
            compute_transform_when_hidden: false,
        }
    }
}

/// Entity hierarchy and render-order settings.
// One field per Lumberyard `UiElementComponent` flag, in the order
// `canvas_binary` reads and writes them; a bitflags type would change
// both the on-disk shape and the serde product.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiElement {
    pub local_id: u32,
    pub enabled: bool,
    pub visible_in_editor: bool,
    pub selectable_in_editor: bool,
    pub selected_in_editor: bool,
    pub expanded_in_editor: bool,
    pub child_order: Vec<UiChildOrder>,
    pub children_render_sortable: bool,
    pub render_priority: i32,
    pub multithread_children: bool,
}

/// Child entity order entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiChildOrder {
    pub entity_id: UiEntityId,
    pub sort_index: u64,
}

impl UiChildOrder {
    #[inline]
    #[must_use]
    pub const fn new(entity_id: UiEntityId, sort_index: u64) -> Self {
        Self {
            entity_id,
            sort_index,
        }
    }
}

/// Image drawing settings.
// One field per Lumberyard `UiImageComponent` flag, in the order
// `canvas_binary` reads and writes them; a bitflags type would change
// both the on-disk shape and the serde product.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiImage {
    pub sprite_type: UiImageSpriteType,
    pub sprite_path: Option<String>,
    pub sprite_index: u32,
    pub render_target_name: Option<String>,
    pub render_target_srgb: bool,
    pub color: LinearRgba,
    pub alpha: f32,
    pub image_type: UiImageType,
    pub fill_center: bool,
    pub stretch_sliced: bool,
    pub blend_mode: UiBlendMode,
    pub fill_type: UiImageFillType,
    pub fill_amount: f32,
    pub fill_start_angle: f32,
    pub fill_corner_origin: UiImageFillCornerOrigin,
    pub fill_edge_origin: UiImageFillEdgeOrigin,
    pub fill_clockwise: bool,
}

impl Default for UiImage {
    fn default() -> Self {
        Self {
            sprite_type: UiImageSpriteType::default(),
            sprite_path: None,
            sprite_index: 0,
            render_target_name: None,
            render_target_srgb: false,
            color: LinearRgba::WHITE,
            alpha: 1.0,
            image_type: UiImageType::default(),
            fill_center: true,
            stretch_sliced: false,
            blend_mode: UiBlendMode::default(),
            fill_type: UiImageFillType::default(),
            fill_amount: 1.0,
            fill_start_angle: 0.0,
            fill_corner_origin: UiImageFillCornerOrigin::default(),
            fill_edge_origin: UiImageFillEdgeOrigin::default(),
            fill_clockwise: true,
        }
    }
}

/// Source kind for image pixels.
#[repr(u8)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum UiImageSpriteType {
    #[default]
    SpriteAsset = 0,
    RenderTarget = 1,
}

impl UiImageSpriteType {
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[inline]
    #[must_use]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::SpriteAsset),
            1 => Some(Self::RenderTarget),
            _ => None,
        }
    }
}

/// Texture mapping mode for a UI image.
#[repr(u8)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum UiImageType {
    #[default]
    Stretched = 0,
    Sliced = 1,
    Fixed = 2,
    Tiled = 3,
    StretchedToFit = 4,
    StretchedToFill = 5,
}

impl UiImageType {
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[inline]
    #[must_use]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Stretched),
            1 => Some(Self::Sliced),
            2 => Some(Self::Fixed),
            3 => Some(Self::Tiled),
            4 => Some(Self::StretchedToFit),
            5 => Some(Self::StretchedToFill),
            _ => None,
        }
    }
}

/// Blend operation used by `LyShine` image drawing.
#[repr(i32)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum UiBlendMode {
    #[default]
    Normal = 0,
    Add = 1,
    Screen = 2,
    Darken = 3,
    Lighten = 4,
}

impl UiBlendMode {
    #[inline]
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    #[inline]
    #[must_use]
    pub const fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Normal),
            1 => Some(Self::Add),
            2 => Some(Self::Screen),
            3 => Some(Self::Darken),
            4 => Some(Self::Lighten),
            _ => None,
        }
    }
}

/// Partial-fill mode for a UI image.
#[repr(u8)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum UiImageFillType {
    #[default]
    None = 0,
    Linear = 1,
    Radial = 2,
    RadialCorner = 3,
    RadialEdge = 4,
}

impl UiImageFillType {
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[inline]
    #[must_use]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Linear),
            2 => Some(Self::Radial),
            3 => Some(Self::RadialCorner),
            4 => Some(Self::RadialEdge),
            _ => None,
        }
    }
}

/// Corner origin for radial-corner image fills.
#[repr(u8)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum UiImageFillCornerOrigin {
    #[default]
    TopLeft = 0,
    TopRight = 1,
    BottomRight = 2,
    BottomLeft = 3,
}

impl UiImageFillCornerOrigin {
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[inline]
    #[must_use]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::TopLeft),
            1 => Some(Self::TopRight),
            2 => Some(Self::BottomRight),
            3 => Some(Self::BottomLeft),
            _ => None,
        }
    }
}

/// Edge origin for linear and radial-edge image fills.
#[repr(u8)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum UiImageFillEdgeOrigin {
    #[default]
    Left = 0,
    Top = 1,
    Right = 2,
    Bottom = 3,
}

impl UiImageFillEdgeOrigin {
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[inline]
    #[must_use]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Left),
            1 => Some(Self::Top),
            2 => Some(Self::Right),
            3 => Some(Self::Bottom),
            _ => None,
        }
    }
}

/// Text drawing settings.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiText {
    pub text: String,
    pub markup_enabled: bool,
    pub images_enabled: bool,
    pub update_on_input_change: bool,
    pub color: LinearRgba,
    pub alpha: f32,
    pub font_path: Option<String>,
    pub font_effect_index: u32,
    pub font_size: f32,
    pub character_spacing: f32,
    pub line_spacing: f32,
    pub horizontal_alignment: i32,
    pub vertical_alignment: i32,
    pub wrap_text_setting: i32,
    pub overflow_mode: i32,
}

impl Default for UiText {
    fn default() -> Self {
        Self {
            text: String::new(),
            markup_enabled: false,
            images_enabled: false,
            update_on_input_change: false,
            color: LinearRgba::WHITE,
            alpha: 1.0,
            font_path: None,
            font_effect_index: 0,
            font_size: 24.0,
            character_spacing: 0.0,
            line_spacing: 1.0,
            horizontal_alignment: 0,
            vertical_alignment: 0,
            wrap_text_setting: 0,
            overflow_mode: 0,
        }
    }
}

/// Button action names and click behavior.
#[derive(Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiButton {
    pub hover_start_action_name: Option<String>,
    pub hover_end_action_name: Option<String>,
    pub pressed_action_name: Option<String>,
    pub released_action_name: Option<String>,
    pub action_name: Option<String>,
    pub action_name_right: Option<String>,
    pub action_name_pressed_right: Option<String>,
    pub use_click_behavior: bool,
    pub click_sq_tolerance: f32,
}

/// Mask/clipping settings for a UI element.
// One field per Lumberyard `UiMaskComponent` flag, in the order
// `canvas_binary` reads and writes them; a bitflags type would change
// both the on-disk shape and the serde product.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiMask {
    pub enable_masking: bool,
    pub mask_interaction: bool,
    pub child_mask_element: UiEntityId,
    pub use_render_to_texture: bool,
    pub draw_behind: bool,
    pub draw_in_front: bool,
    pub use_alpha_test: bool,
}

impl Default for UiMask {
    fn default() -> Self {
        Self {
            enable_masking: true,
            mask_interaction: true,
            child_mask_element: UiEntityId::new(0),
            use_render_to_texture: false,
            draw_behind: false,
            draw_in_front: false,
            use_alpha_test: false,
        }
    }
}

/// Row/column layout settings shared by `LyShine` axis layout components.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiLayoutAxis {
    pub padding: UiRect,
    pub spacing: f32,
    pub order: i32,
    pub child_h_alignment: i32,
    pub child_v_alignment: i32,
    pub ignore_default_layout_cells: bool,
}

impl Default for UiLayoutAxis {
    fn default() -> Self {
        Self {
            padding: UiRect::ZERO,
            spacing: 0.0,
            order: 0,
            child_h_alignment: 0,
            child_v_alignment: 0,
            ignore_default_layout_cells: false,
        }
    }
}

/// Grid layout settings.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiLayoutGrid {
    pub padding: UiRect,
    pub spacing: Vec2,
    pub cell_size: Vec2,
    pub horizontal_order: i32,
    pub vertical_order: i32,
    pub starting_with: i32,
    pub child_h_alignment: i32,
    pub child_v_alignment: i32,
}

impl Default for UiLayoutGrid {
    fn default() -> Self {
        Self {
            padding: UiRect::ZERO,
            spacing: Vec2::ZERO,
            cell_size: Vec2::ZERO,
            horizontal_order: 0,
            vertical_order: 0,
            starting_with: 0,
            child_h_alignment: 0,
            child_v_alignment: 0,
        }
    }
}

/// Per-child layout sizing overrides.
// Six `*_overridden` flags, each paired with the value it gates, in the
// order `canvas_binary` reads and writes them; folding them into a
// bitflags type would split those pairs and change the on-disk shape.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiLayoutCell {
    pub min_width_overridden: bool,
    pub min_width: f32,
    pub min_height_overridden: bool,
    pub min_height: f32,
    pub target_width_overridden: bool,
    pub target_width: f32,
    pub target_height_overridden: bool,
    pub target_height: f32,
    pub max_width_overridden: bool,
    pub max_width: f32,
    pub max_height_overridden: bool,
    pub max_height: f32,
    pub extra_width_ratio_overridden: bool,
    pub extra_width_ratio: f32,
    pub extra_height_ratio_overridden: bool,
    pub extra_height_ratio: f32,
}

impl Default for UiLayoutCell {
    fn default() -> Self {
        Self {
            min_width_overridden: false,
            min_width: 0.0,
            min_height_overridden: false,
            min_height: 0.0,
            target_width_overridden: false,
            target_width: 0.0,
            target_height_overridden: false,
            target_height: 0.0,
            max_width_overridden: false,
            max_width: 0.0,
            max_height_overridden: false,
            max_height: 0.0,
            extra_width_ratio_overridden: false,
            extra_width_ratio: 1.0,
            extra_height_ratio_overridden: false,
            extra_height_ratio: 1.0,
        }
    }
}

/// Fade settings applied to an entity.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct UiFader {
    pub fade: f32,
    pub use_render_to_texture: bool,
}

impl Default for UiFader {
    fn default() -> Self {
        Self {
            fade: 1.0,
            use_render_to_texture: false,
        }
    }
}

/// Write a native UI canvas asset.
///
/// # Errors
///
/// Returns [`UiCanvasAssetFormatError::TooManyItems`] if the asset holds
/// more entities, child-order entries, atlases or script properties than a
/// `u32` length prefix can name, or
/// [`UiCanvasAssetFormatError::Io`] if `writer` rejects a write.
pub fn write_ui_canvas_asset(
    asset: &UiCanvasAsset,
    writer: impl Write,
) -> Result<(), UiCanvasAssetFormatError> {
    super::canvas_binary::write_ui_canvas_asset(asset, writer)
}

/// Read a native UI canvas asset.
///
/// # Errors
///
/// Returns any error [`read_ui_canvas_asset_from_reader`] returns — plus
/// [`UiCanvasAssetFormatError::Io`] for an unexpected end of `bytes`,
/// which the in-memory cursor reports as [`std::io::ErrorKind::UnexpectedEof`].
pub fn read_ui_canvas_asset(bytes: &[u8]) -> Result<UiCanvasAsset, UiCanvasAssetFormatError> {
    super::canvas_binary::read_ui_canvas_asset(bytes)
}

pub fn register_ui_value_types(app: &mut App) {
    app.register_type::<UiEntityId>()
        .register_type::<UiRect>()
        .register_type::<UiTransform2d>()
        .register_type::<UiElement>()
        .register_type::<UiChildOrder>()
        .register_type::<UiImage>()
        .register_type::<UiImageSpriteType>()
        .register_type::<UiImageType>()
        .register_type::<UiBlendMode>()
        .register_type::<UiImageFillType>()
        .register_type::<UiImageFillCornerOrigin>()
        .register_type::<UiImageFillEdgeOrigin>()
        .register_type::<UiText>()
        .register_type::<UiButton>()
        .register_type::<UiMask>()
        .register_type::<UiLayoutAxis>()
        .register_type::<UiLayoutGrid>()
        .register_type::<UiLayoutCell>()
        .register_type::<UiFader>();
}

/// Read a native UI canvas asset from a stream.
///
/// # Errors
///
/// Returns [`UiCanvasAssetFormatError::BadMagic`] if the first eight bytes
/// are not [`UI_CANVAS_ASSET_MAGIC`],
/// [`UiCanvasAssetFormatError::UnsupportedVersion`] if the version prefix
/// falls outside `UI_CANVAS_ASSET_MIN_VERSION..=UI_CANVAS_ASSET_VERSION`,
/// [`UiCanvasAssetFormatError::InvalidEnum`] for a component enum
/// discriminant the format does not define,
/// [`UiCanvasAssetFormatError::InvalidData`] for a malformed script
/// property payload, [`UiCanvasAssetFormatError::Utf8`] for a string field
/// that is not UTF-8, and [`UiCanvasAssetFormatError::Io`] if `reader`
/// fails or ends early.
pub fn read_ui_canvas_asset_from_reader(
    reader: impl Read,
) -> Result<UiCanvasAsset, UiCanvasAssetFormatError> {
    super::canvas_binary::read_ui_canvas_asset_from_reader(reader)
}

/// Bevy asset loader for native UI canvas assets.
#[derive(Default, TypePath)]
pub struct UiCanvasAssetLoader;

impl AssetLoader for UiCanvasAssetLoader {
    type Asset = UiCanvasAsset;
    type Settings = ();
    type Error = UiCanvasAssetFormatError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        super::canvas_binary::read_ui_canvas_asset_from_bevy_reader(reader).await
    }

    fn extensions(&self) -> &[&str] {
        UI_CANVAS_ASSET_EXTENSIONS
    }
}

/// Native UI canvas asset format errors.
#[derive(Debug, Error)]
pub enum UiCanvasAssetFormatError {
    #[error("bad UI canvas asset magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported UI canvas asset version {version}, expected {expected}")]
    UnsupportedVersion { version: u32, expected: u32 },
    #[error("{what} count {count} exceeds u32")]
    TooManyItems { what: &'static str, count: usize },
    #[error("invalid UI canvas asset data: {0}")]
    InvalidData(&'static str),
    #[error("invalid UI canvas enum {field} value {value}")]
    InvalidEnum { field: &'static str, value: i32 },
    #[error("invalid UTF-8 string: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_canvas_asset_round_trips_binary_format() {
        let mut script = ScriptComponent::default();
        script
            .properties
            .properties
            .push(az_framework::ScriptProperty::new(
                az_framework::ScriptPropertyKey::from_name("Event"),
                az_framework::ScriptPropertyValue::DynamicClass(
                    az_framework::ScriptDynamicClassValue {
                        type_name: Some("EventData".to_owned()),
                        payload_type_id: Some("46f1804a-234d-4511-a5a0-70851cf1096f".to_owned()),
                        payload: az_framework::ScriptDynamicValue::Struct(vec![
                            az_framework::ScriptDynamicField::new(
                                "m_entityRef",
                                az_framework::ScriptDynamicValue::EntityRef(Some(42)),
                            ),
                            az_framework::ScriptDynamicField::new(
                                "m_applyRecursively",
                                az_framework::ScriptDynamicValue::Bool(true),
                            ),
                            az_framework::ScriptDynamicField::new(
                                "offset",
                                az_framework::ScriptDynamicValue::Vector2(Vec2::new(1.5, -2.0)),
                            ),
                        ]),
                    },
                ),
            ));
        let asset = UiCanvasAsset::new(
            UiCanvas {
                unique_id: 42,
                root_entity: UiEntityId::new(7),
                size: Vec2::new(1920.0, 1080.0),
                draw_order: 3,
                texture_atlases: vec!["ui/common.texatlasidx".to_string()],
                ..Default::default()
            },
            vec![UiEntity {
                entity_id: UiEntityId::new(7),
                name: Some("Root".to_string()),
                transform: Some(UiTransform2d {
                    offsets: UiRect::new(1.0, 2.0, 3.0, 4.0),
                    ..Default::default()
                }),
                element: Some(UiElement {
                    local_id: 2,
                    enabled: true,
                    child_order: vec![UiChildOrder::new(UiEntityId::new(8), 0)],
                    ..Default::default()
                }),
                image: Some(UiImage {
                    sprite_path: Some("lyshineui/images/common/gradientbox.sprite".to_string()),
                    color: LinearRgba::new(0.1, 0.2, 0.3, 0.4),
                    ..Default::default()
                }),
                text: Some(UiText {
                    text: "@ui_play".to_string(),
                    font_path: Some("fonts/nimbus.font".to_string()),
                    ..Default::default()
                }),
                script: Some(script),
                components: vec![UiComponentKind::Transform2d, UiComponentKind::Element],
                ..Default::default()
            }],
        );

        let mut bytes = Vec::new();
        write_ui_canvas_asset(&asset, &mut bytes).unwrap();
        let decoded = read_ui_canvas_asset(&bytes).unwrap();

        assert_eq!(decoded, asset);
    }
}
