//! Runtime `LyShine` canvas spawning.
//!
//! O3DE reference: `Gems/LyShine/Code/Source/UiCanvasManager.cpp`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use az_gem_texture_atlas::TextureAtlasAsset;
use bevy::asset::{AssetId, LoadState, RenderAssetUsages};
use bevy::color::LinearRgba;
use bevy::ecs::system::SystemState;
use bevy::image::{Image, TextureAtlas, TextureAtlasLayout};
use bevy::log::{debug, info, trace, warn};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bink::runtime::{BinkAudioInfo, BinkAudioPlan, BinkRuntime, BinkSoundSystemStatus, BinkVideo};

use crate::{
    LyShineCanvasLoadQueue, LyShineCanvasLoadRequest, LyShineCanvasPurpose, LyShineSpriteAsset,
    LyShineUiScriptBinding, UiButton, UiCanvasAsset, UiEntity, UiEntityId, UiImage,
    UiImageFillType, UiImageType, UiLayoutCell, UiScript, UiText, UiTransform2d,
    lyshine_script_asset_path, pixel_border_rect, sprite_sidecar_path,
};

/// Canvas assets requested by the `LyShine` front-end boot queue.
#[derive(Debug, Clone, Default, Resource)]
pub struct LyShineLoadedCanvasAssets {
    pub canvases: Vec<LyShineLoadedCanvasAsset>,
}

impl LyShineLoadedCanvasAssets {
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.canvases.is_empty()
    }
}

/// One loaded UI canvas handle.
#[derive(Debug, Clone)]
pub struct LyShineLoadedCanvasAsset {
    pub request: LyShineCanvasLoadRequest,
    pub handle: Handle<UiCanvasAsset>,
}

/// Spawn tracking for loaded `LyShine` canvases.
#[derive(Debug, Default, Resource)]
pub struct LyShineSpawnedCanvases {
    spawned: HashSet<AssetId<UiCanvasAsset>>,
    pending: HashSet<AssetId<UiCanvasAsset>>,
    failed: HashSet<AssetId<UiCanvasAsset>>,
}

/// Bevy layout handles created from loaded `LyShine` texture-atlas products.
#[derive(Debug, Default, Resource)]
pub struct LyShineTextureAtlasLayouts {
    layouts: HashMap<AssetId<TextureAtlasAsset>, Handle<TextureAtlasLayout>>,
}

/// Runtime enabled-state for loaded `LyShine` canvases.
///
/// Loading and visibility are independent. Project code supplies each initial
/// `active_on_load` value, then Lua or components may change visibility through
/// the UI buses.
#[derive(Debug, Clone, Default, Resource)]
pub struct LyShineCanvasEnabledState {
    enabled_paths: HashSet<&'static str>,
}

impl LyShineCanvasEnabledState {
    #[inline]
    #[must_use]
    pub fn is_enabled(&self, asset_path: &'static str) -> bool {
        self.enabled_paths.contains(asset_path)
    }

    pub fn set_enabled(&mut self, asset_path: &'static str, enabled: bool) {
        if enabled {
            self.enabled_paths.insert(asset_path);
        } else {
            self.enabled_paths.remove(asset_path);
        }
    }

    pub fn enable_only(&mut self, asset_path: &'static str) {
        self.enabled_paths.clear();
        self.enabled_paths.insert(asset_path);
    }
}

/// Startup Bink video requested by the frontend flow.
#[derive(Resource, Debug, Clone)]
pub struct LyShineBinkStartupVideo {
    pub asset_path: &'static str,
    pub filesystem_path: PathBuf,
    pub playback_canvas_path: &'static str,
    pub next_canvas_path: &'static str,
    pub probe_open_flags: u32,
    pub playback_open_flags: u32,
    pub audio_planner: Option<fn(&BinkAudioInfo) -> BinkAudioPlan>,
}

impl LyShineBinkStartupVideo {
    #[must_use]
    pub const fn new(
        asset_path: &'static str,
        filesystem_path: PathBuf,
        playback_canvas_path: &'static str,
        next_canvas_path: &'static str,
    ) -> Self {
        Self {
            asset_path,
            filesystem_path,
            playback_canvas_path,
            next_canvas_path,
            probe_open_flags: 0,
            playback_open_flags: 0,
            audio_planner: None,
        }
    }

    /// Configure project-owned Bink flags and track-selection policy.
    #[must_use]
    pub const fn with_audio_plan(
        mut self,
        probe_open_flags: u32,
        playback_open_flags: u32,
        planner: fn(&BinkAudioInfo) -> BinkAudioPlan,
    ) -> Self {
        self.probe_open_flags = probe_open_flags;
        self.playback_open_flags = playback_open_flags;
        self.audio_planner = Some(planner);
        self
    }
}

#[derive(Resource, Default, Debug)]
pub struct LyShineBinkStartupState {
    attempted: bool,
    completed: bool,
}

#[derive(Debug)]
pub struct LyShineBinkVideoPlayback {
    video: BinkVideo,
    frame_texture: Handle<Image>,
    overlay: Entity,
    frames_decoded: u32,
    asset_path: &'static str,
    playback_canvas_path: &'static str,
    next_canvas_path: &'static str,
}

/// GPU upload texture for frames decoded from a Bink video source.
///
/// Bevy renders video frames by sampling a texture. The `.bk2` remains the
/// source video asset; this component only marks the per-frame upload target.
#[derive(Component, Debug, Clone, Copy)]
struct LyShineBinkFrameTexture {
    _asset_path: &'static str,
    _source_width: u32,
    _source_height: u32,
    _frame_count: u32,
}

/// Root Bevy UI node for one loaded `LyShine` canvas.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct LyShineCanvasRoot {
    pub asset_path: &'static str,
    pub purpose: LyShineCanvasPurpose,
    pub active_on_load: bool,
}

/// Texture atlases loaded for one `LyShine` canvas.
#[derive(Component, Debug, Clone)]
pub struct LyShineCanvasTextureAtlases {
    pub handles: Box<[Handle<TextureAtlasAsset>]>,
}

/// Bevy UI node spawned from one `LyShine` canvas entity.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LyShineUiEntity {
    pub entity_id: UiEntityId,
}

/// Debug metadata retained beside a spawned `LyShine` entity.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct LyShineUiEntityDebugInfo {
    pub name: Option<Box<str>>,
    pub script: Option<Box<str>>,
}

/// Image node waiting for its canvas texture atlases to load.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct LyShineImageBinding {
    pub canvas_root: Entity,
    pub sprite_path: Box<str>,
}

/// Sprite-border sidecar handle for a `Sliced` image.
///
/// Bound alongside the texture so the post-load
/// [`apply_sprite_borders`] system can rewrite the `TextureSlicer`
/// with real pixel borders once both halves (the `.dds` texture
/// and the `.sprite` sidecar) finish loading. Mirrors the
/// O3DE `Sliced` codepath in
/// `Gems/LyShine/Code/Source/UiImageComponent.cpp:1309`
/// `RenderSlicedSprite`.
///
/// Non-Sliced images (Stretched / Fixed / Tiled) don't get this
/// component — they don't consume `.sprite` borders.
#[derive(Component, Debug, Clone)]
pub struct LyShineSpriteBorderBinding {
    /// Handle on the `.sprite` companion file
    /// (`<dds-stem>.sprite`). May still be loading.
    pub sprite_handle: Handle<LyShineSpriteAsset>,
    /// The `Image` handle the borders need to be sized against —
    /// pixel border = `uv_border` × `texture_size`. Without the
    /// texture loaded we don't know the dimensions yet.
    pub image_handle: Handle<Image>,
    /// Whether [`apply_sprite_borders`] has already rewritten the
    /// `TextureSlicer` with real borders. Once true, the system
    /// short-circuits on subsequent frames.
    pub applied: bool,
}

/// Native `LyShine` button/interactable action names attached to one spawned UI node.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct LyShineButtonActions {
    pub canvas_root: Entity,
    pub hover_start_action_name: Option<Box<str>>,
    pub hover_end_action_name: Option<Box<str>>,
    pub pressed_action_name: Option<Box<str>>,
    pub released_action_name: Option<Box<str>>,
    pub action_name: Option<Box<str>>,
    pub action_name_right: Option<Box<str>>,
    pub action_name_pressed_right: Option<Box<str>>,
    pub use_click_behavior: bool,
    pub click_sq_tolerance: f32,
}

impl LyShineButtonActions {
    fn new(canvas_root: Entity, button: &UiButton) -> Self {
        Self {
            canvas_root,
            hover_start_action_name: box_action(button.hover_start_action_name.as_deref()),
            hover_end_action_name: box_action(button.hover_end_action_name.as_deref()),
            pressed_action_name: box_action(button.pressed_action_name.as_deref()),
            released_action_name: box_action(button.released_action_name.as_deref()),
            action_name: box_action(button.action_name.as_deref()),
            action_name_right: box_action(button.action_name_right.as_deref()),
            action_name_pressed_right: box_action(button.action_name_pressed_right.as_deref()),
            use_click_behavior: button.use_click_behavior,
            click_sq_tolerance: button.click_sq_tolerance,
        }
    }
}

#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LyShineButtonInteractionState {
    hovered: bool,
    pressed: bool,
}

/// Native `LyShine` dispatch mode for `UiCanvasNotificationBus::OnAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyShineUiActionDispatch {
    Immediate,
    Queued,
}

impl LyShineUiActionDispatch {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::Queued => "queued",
        }
    }
}

/// Native button/interactable action phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyShineUiActionPhase {
    HoverStart,
    HoverEnd,
    Pressed,
    Released,
    Click,
}

impl LyShineUiActionPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::HoverStart => "hover_start",
            Self::HoverEnd => "hover_end",
            Self::Pressed => "pressed",
            Self::Released => "released",
            Self::Click => "click",
        }
    }
}

/// One native-style canvas action notification emitted by `LyShine` UI input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyShineUiAction {
    pub canvas_root: Entity,
    pub source_entity: Entity,
    pub source_ui_entity: UiEntityId,
    pub dispatch: LyShineUiActionDispatch,
    pub phase: LyShineUiActionPhase,
    pub action_name: Box<str>,
    pub target_scope: Box<str>,
    pub callback_name: Box<str>,
}

/// FIFO bridge for `UiCanvasNotificationBus::OnAction` notifications.
#[derive(Resource, Debug, Default)]
pub struct LyShineQueuedUiActions {
    actions: VecDeque<LyShineUiAction>,
}

impl LyShineQueuedUiActions {
    fn push(&mut self, action: LyShineUiAction) {
        self.actions.push_back(action);
    }

    fn pop(&mut self) -> Option<LyShineUiAction> {
        self.actions.pop_front()
    }
}

/// Retained UI actions that were delivered to the current `LyShine` bus bridge.
#[derive(Resource, Debug, Default)]
pub struct LyShineDispatchedUiActions {
    pub actions: Vec<LyShineUiAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LyShineParentLayout {
    Row,
    Column,
    Grid,
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
pub fn load_queued_canvas_assets(
    queue: Res<LyShineCanvasLoadQueue>,
    asset_server: Option<Res<AssetServer>>,
    mut loaded: ResMut<LyShineLoadedCanvasAssets>,
    mut enabled_state: ResMut<LyShineCanvasEnabledState>,
) {
    let Some(asset_server) = asset_server else {
        return;
    };

    loaded.canvases.clear();
    loaded.canvases.reserve(queue.canvases.len());
    loaded
        .canvases
        .extend(queue.canvases.iter().cloned().map(|request| {
            if request.active_on_load {
                enabled_state.set_enabled(request.asset_path, true);
            }
            let handle = asset_server.load(request.asset_path);
            trace!(
                "Queued LyShine canvas request path={} purpose={:?} active_on_load={} handle={:?}",
                request.asset_path, request.purpose, request.active_on_load, handle
            );
            LyShineLoadedCanvasAsset { request, handle }
        }));
    info!(
        "Requested {} LyShine canvas asset(s)",
        loaded.canvases.len()
    );
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
pub fn spawn_loaded_canvas_assets(
    loaded: Option<Res<LyShineLoadedCanvasAssets>>,
    canvas_assets: Res<Assets<UiCanvasAsset>>,
    asset_server: Option<Res<AssetServer>>,
    enabled_state: Res<LyShineCanvasEnabledState>,
    mut spawned: ResMut<LyShineSpawnedCanvases>,
    mut commands: Commands,
) {
    let Some(loaded) = loaded else {
        return;
    };
    let asset_server = asset_server.as_deref();

    for loaded_canvas in &loaded.canvases {
        let asset_id = loaded_canvas.handle.id();
        if spawned.spawned.contains(&asset_id) {
            continue;
        }
        let Some(canvas_asset) = canvas_assets.get(&loaded_canvas.handle) else {
            if let Some(asset_server) = asset_server {
                let load_state = asset_server.load_state(asset_id);
                if let LoadState::Failed(error) = &load_state {
                    spawned.pending.remove(&asset_id);
                    if spawned.failed.insert(asset_id) {
                        warn!(
                            "LyShine canvas asset failed to load; skipping canvas path={} purpose={:?} active_on_load={} asset_id={:?} error={}",
                            loaded_canvas.request.asset_path,
                            loaded_canvas.request.purpose,
                            loaded_canvas.request.active_on_load,
                            asset_id,
                            error
                        );
                    }
                } else if spawned.pending.insert(asset_id) {
                    trace!(
                        "LyShine canvas asset pending path={} purpose={:?} active_on_load={} asset_id={:?} load_state={:?}",
                        loaded_canvas.request.asset_path,
                        loaded_canvas.request.purpose,
                        loaded_canvas.request.active_on_load,
                        asset_id,
                        load_state
                    );
                }
            }
            continue;
        };

        spawned.pending.remove(&asset_id);
        if !enabled_state.is_enabled(loaded_canvas.request.asset_path) {
            continue;
        }
        spawn_canvas_asset(
            &mut commands,
            asset_server,
            &enabled_state,
            &loaded_canvas.request,
            canvas_asset,
        );
        info!(
            "Spawned LyShine canvas {} ({} entity/entities, {} atlas/es)",
            loaded_canvas.request.asset_path,
            canvas_asset.entities.len(),
            canvas_asset.canvas.texture_atlases.len()
        );
        spawned.spawned.insert(asset_id);
    }
}

fn spawn_canvas_asset(
    commands: &mut Commands,
    asset_server: Option<&AssetServer>,
    enabled_state: &LyShineCanvasEnabledState,
    request: &LyShineCanvasLoadRequest,
    canvas_asset: &UiCanvasAsset,
) -> Entity {
    let canvas_enabled = enabled_state.is_enabled(request.asset_path);
    let root_ids = root_entity_ids(canvas_asset);
    debug!(
        "Mounting LyShine canvas path={} purpose={:?} request_active_on_load={} runtime_enabled={} unique_id={} size={:?} draw_order={} root_entity={} roots={:?} atlases={:?} flags={:?} render_target={:?}",
        request.asset_path,
        request.purpose,
        request.active_on_load,
        canvas_enabled,
        canvas_asset.canvas.unique_id,
        canvas_asset.canvas.size,
        canvas_asset.canvas.draw_order,
        canvas_asset.canvas.root_entity.as_u64(),
        root_ids.iter().map(|id| id.as_u64()).collect::<Vec<_>>(),
        canvas_asset.canvas.texture_atlases,
        canvas_asset.canvas.flags,
        canvas_asset.canvas.render_target_name,
    );

    let root = commands
        .spawn((
            Name::new(format!("LyShine Canvas {}", request.asset_path)),
            LyShineCanvasRoot {
                asset_path: request.asset_path,
                purpose: request.purpose,
                active_on_load: request.active_on_load,
            },
            LyShineCanvasTextureAtlases {
                handles: canvas_asset
                    .canvas
                    .texture_atlases
                    .iter()
                    .filter_map(|path| {
                        asset_server.map(|asset_server| asset_server.load(path.clone()))
                    })
                    .collect(),
            },
            canvas_root_node(),
            GlobalZIndex(canvas_asset.canvas.draw_order),
            if canvas_enabled {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
        ))
        .id();
    trace!(
        "Mounted LyShine canvas root path={} entity={:?} node={}",
        request.asset_path,
        root,
        format_node(&canvas_root_node())
    );

    let entities_by_id: HashMap<UiEntityId, &UiEntity> = canvas_asset
        .entities
        .iter()
        .map(|entity| (entity.entity_id, entity))
        .collect();
    let mut visited = HashSet::with_capacity(canvas_asset.entities.len());
    let ctx = UiTreeContext {
        asset_server,
        canvas_root: root,
        entities_by_id: &entities_by_id,
    };

    for entity_id in root_ids {
        let Some(entity) = entities_by_id.get(&entity_id).copied() else {
            continue;
        };
        spawn_ui_entity_tree(commands, &ctx, root, entity, None, &mut visited);
    }

    root
}

/// The parts of a canvas spawn that stay fixed for the whole tree walk.
struct UiTreeContext<'a, 'e> {
    asset_server: Option<&'a AssetServer>,
    canvas_root: Entity,
    entities_by_id: &'a HashMap<UiEntityId, &'e UiEntity>,
}

fn spawn_ui_entity_tree(
    commands: &mut Commands,
    ctx: &UiTreeContext<'_, '_>,
    parent: Entity,
    entity: &UiEntity,
    parent_layout: Option<LyShineParentLayout>,
    visited: &mut HashSet<UiEntityId>,
) {
    let canvas_root = ctx.canvas_root;
    if !visited.insert(entity.entity_id) {
        trace!(
            "Skipping repeated LyShine UI entity canvas_root={:?} parent={:?} entity_id={}",
            canvas_root,
            parent,
            entity.entity_id.as_u64()
        );
        return;
    }

    let spawned = spawn_ui_entity_node(
        commands,
        ctx.asset_server,
        canvas_root,
        entity,
        parent_layout,
    );
    trace!(
        "Attached LyShine UI entity canvas_root={:?} parent={:?} child={:?} entity_id={} child_count={}",
        canvas_root,
        parent,
        spawned,
        entity.entity_id.as_u64(),
        entity
            .element
            .as_ref()
            .map_or(0, |element| element.child_order.len())
    );
    commands.entity(parent).add_child(spawned);

    let child_parent_layout = parent_layout_for(entity);
    for child in ordered_children(entity, ctx.entities_by_id) {
        spawn_ui_entity_tree(commands, ctx, spawned, child, child_parent_layout, visited);
    }
}

fn spawn_ui_entity_node(
    commands: &mut Commands,
    asset_server: Option<&AssetServer>,
    canvas_root: Entity,
    entity: &UiEntity,
    parent_layout: Option<LyShineParentLayout>,
) -> Entity {
    let mut node = node_from_transform(entity.transform.as_ref());
    apply_mask_node(entity, &mut node);
    apply_layout_container_node(entity, &mut node);
    apply_layout_child_node(entity, parent_layout, &mut node);
    let visibility = entity_visibility(entity);
    let fader_alpha = entity
        .fader
        .as_ref()
        .map_or(1.0, |fader| fader.fade.clamp(0.0, 1.0));
    debug!(
        "Mounting LyShine UI entity canvas_root={:?} entity_id={} name={} dependency_ready={} runtime_active={} visibility={:?} components={:?} parent_layout={:?} transform={} computed_node={} element={} image={} text={} button={} fader={:?} mask={} layout_row={} layout_column={} layout_grid={} layout_cell={} script={}",
        canvas_root,
        entity.entity_id.as_u64(),
        entity.name.as_deref().unwrap_or("<unnamed>"),
        entity.dependency_ready,
        entity.runtime_active,
        visibility,
        entity.components,
        parent_layout,
        format_transform(entity.transform.as_ref()),
        format_node(&node),
        format_element(entity.element.as_ref()),
        format_image(entity.image.as_ref()),
        format_text(entity.text.as_ref()),
        format_button(entity.button.as_ref()),
        entity.fader,
        format_mask(entity.mask.as_ref()),
        format_layout_axis(entity.layout_row.as_ref()),
        format_layout_axis(entity.layout_column.as_ref()),
        format_layout_grid(entity.layout_grid.as_ref()),
        format_layout_cell(entity.layout_cell.as_ref()),
        format_script(entity.script.as_ref())
    );

    let spawned = commands
        .spawn((
            ui_entity_name(entity),
            LyShineUiEntity {
                entity_id: entity.entity_id,
            },
            LyShineUiEntityDebugInfo {
                name: entity
                    .name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .map(Into::into),
                script: entity
                    .script
                    .as_ref()
                    .map(|script| format_script(Some(script)).into_boxed_str()),
            },
            node,
            visibility,
        ))
        .id();

    if let Some(binding) = entity.script.as_ref().and_then(ui_script_binding) {
        commands.entity(spawned).insert(binding);
    }

    if let Some(image) = entity.image.as_ref() {
        commands
            .entity(spawned)
            .insert(image_node(image, fader_alpha));
        if let Some(sprite_path) = image.sprite_path.as_deref().filter(|path| !path.is_empty()) {
            trace!(
                "Queued LyShine image binding canvas_root={:?} entity={:?} entity_id={} sprite_path={} sprite_index={} sprite_type={:?}",
                canvas_root,
                spawned,
                entity.entity_id.as_u64(),
                sprite_path,
                image.sprite_index,
                image.sprite_type
            );
            commands.entity(spawned).insert(LyShineImageBinding {
                canvas_root,
                sprite_path: sprite_path.into(),
            });
        }
    }
    if let Some(button) = entity.button.as_ref() {
        commands.entity(spawned).insert((
            Button,
            LyShineButtonActions::new(canvas_root, button),
            LyShineButtonInteractionState::default(),
        ));
    }
    if let Some(text) = entity.text.as_ref() {
        let text_node = commands
            .spawn(text_bundle(asset_server, text, fader_alpha))
            .id();
        commands.entity(spawned).add_child(text_node);
    }

    spawned
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
pub fn bind_loaded_canvas_images(
    mut commands: Commands,
    root_atlases: Query<&LyShineCanvasTextureAtlases>,
    atlas_assets: Res<Assets<TextureAtlasAsset>>,
    asset_server: Option<Res<AssetServer>>,
    mut atlas_layouts: ResMut<Assets<TextureAtlasLayout>>,
    mut layout_cache: ResMut<LyShineTextureAtlasLayouts>,
    mut images: Query<(
        Entity,
        &LyShineUiEntity,
        &Node,
        &LyShineImageBinding,
        &mut ImageNode,
    )>,
) {
    let Some(asset_server) = asset_server else {
        return;
    };

    for (entity, ui_entity, node, binding, mut image_node) in &mut images {
        let Ok(canvas_atlases) = root_atlases.get(binding.canvas_root) else {
            continue;
        };
        let site = ImageBindSite {
            canvas_root: binding.canvas_root,
            entity,
            entity_id: ui_entity.entity_id,
            node,
            sprite_path: &binding.sprite_path,
        };
        match bindable_atlas_region(
            &binding.sprite_path,
            canvas_atlases,
            &atlas_assets,
            &mut atlas_layouts,
            &mut layout_cache,
        ) {
            AtlasBinding::Bound(bound) => {
                bind_atlas_image(&asset_server, &mut image_node, &site, bound);
            }
            AtlasBinding::Pending => {
                trace!(
                    "Waiting for LyShine atlas before binding image canvas_root={:?} sprite_path={}",
                    binding.canvas_root, binding.sprite_path
                );
            }
            AtlasBinding::NoRegion => {
                let texture = bind_direct_image(&asset_server, &mut image_node, &site);
                // If this image renders in `Sliced` mode and we
                // haven't already kicked off the 9-slice sidecar
                // load, do it now. Lumberyard's RenderSlicedSprite
                // needs the per-sprite borders from
                // `<sprite-stem>.sprite`; without them every panel
                // chrome stretches flat (since `BorderRect::ZERO`
                // collapses the 9-slice into a single centre).
                if matches!(image_node.image_mode, NodeImageMode::Sliced(_))
                    && let Some(sidecar_path) = sprite_sidecar_path(&binding.sprite_path)
                {
                    let sprite_handle =
                        asset_server.load::<LyShineSpriteAsset>(sidecar_path.clone());
                    trace!(
                        "Queued LyShine sprite border sidecar canvas_root={:?} entity={:?} sprite_path={} sidecar_path={}",
                        binding.canvas_root, entity, binding.sprite_path, sidecar_path,
                    );
                    commands.entity(entity).insert(LyShineSpriteBorderBinding {
                        sprite_handle,
                        image_handle: texture,
                        applied: false,
                    });
                }
            }
        }
    }
}

/// Rewrite the `TextureSlicer::border` on `Sliced` images once
/// both the sprite sidecar and the texture have finished loading.
///
/// Mirrors `UiImageComponent::RenderSlicedSprite`
/// (O3DE reference: `Gems/LyShine/Code/Source/UiImageComponent.cpp:1318-1336`)
/// — the four border values are derived from the sprite's UV
/// borders multiplied by the texture's pixel dimensions:
///
/// ```text
/// left   = m_left            * texture_width
/// right  = (1.0 - m_right)   * texture_width
/// top    = m_top             * texture_height
/// bottom = (1.0 - m_bottom)  * texture_height
/// ```
///
/// Runs every frame; short-circuits per binding once `applied=true`.
// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
pub fn apply_sprite_borders(
    sprite_assets: Res<Assets<LyShineSpriteAsset>>,
    image_assets: Res<Assets<Image>>,
    mut bindings: Query<(Entity, &mut LyShineSpriteBorderBinding, &mut ImageNode)>,
) {
    for (entity, mut binding, mut image_node) in &mut bindings {
        if binding.applied {
            continue;
        }
        let Some(sprite) = sprite_assets.get(&binding.sprite_handle) else {
            continue;
        };
        let Some(image) = image_assets.get(&binding.image_handle) else {
            continue;
        };

        let extent = image.texture_descriptor.size;
        let texture_size = Vec2::new(
            texture_extent_f32(extent.width),
            texture_extent_f32(extent.height),
        );
        let border = pixel_border_rect(sprite.borders, texture_size);

        // Pull the slicer config out, replace its border, put it back.
        // `NodeImageMode::Sliced` carries the full `TextureSlicer`
        // we built at spawn time (with the stretch / tile mode
        // chosen from `image.stretch_sliced`); we only rewrite the
        // `border` field so we preserve the rest.
        if let NodeImageMode::Sliced(ref mut slicer) = image_node.image_mode {
            slicer.border = border;
            debug!(
                "Applied LyShine sprite border entity={:?} texture_size={:?} uv_borders=({:.4},{:.4},{:.4},{:.4}) pixel_border=(L{:.1} R{:.1} T{:.1} B{:.1})",
                entity,
                texture_size,
                sprite.borders.left,
                sprite.borders.right,
                sprite.borders.top,
                sprite.borders.bottom,
                border.min_inset.x,
                border.max_inset.x,
                border.min_inset.y,
                border.max_inset.y,
            );
        } else {
            // Image_mode flipped to non-Sliced after binding (e.g.
            // a Lua script reassigned the sprite). Drop the
            // tracking so we don't keep re-checking.
            trace!(
                "Skipping LyShine sprite border apply (non-Sliced mode) entity={:?}",
                entity
            );
        }
        binding.applied = true;
    }
}

/// One texture dimension in pixels, as the `f32` the border math needs.
///
/// `Extent3d` counts pixels in `u32`, but the 9-slice borders are
/// fractions of the texture size and are computed in `f32`. Only
/// dimensions above 2^24 would lose an integer, and no graphics backend
/// accepts a texture that wide (`wgpu`'s 2D limit is 16384).
#[inline]
#[allow(
    clippy::cast_precision_loss,
    reason = "texture dimensions are bounded far below f32's 2^24 exact-integer range"
)]
const fn texture_extent_f32(pixels: u32) -> f32 {
    pixels as f32
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
pub fn sync_canvas_enabled_state(
    enabled_state: Res<LyShineCanvasEnabledState>,
    mut canvases: Query<(&LyShineCanvasRoot, &mut Visibility)>,
) {
    if !enabled_state.is_changed() {
        return;
    }

    for (root, mut visibility) in &mut canvases {
        let next = if enabled_state.is_enabled(root.asset_path) {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != next {
            debug!(
                "LyShine canvas enabled-state changed path={} purpose={:?} enabled={} visibility={:?}",
                root.asset_path,
                root.purpose,
                enabled_state.is_enabled(root.asset_path),
                next
            );
            *visibility = next;
        }
    }
}

pub fn dispatch_button_actions(
    mut queued_actions: ResMut<LyShineQueuedUiActions>,
    mut buttons: Query<
        (
            Entity,
            &LyShineUiEntity,
            &Interaction,
            &LyShineButtonActions,
            &mut LyShineButtonInteractionState,
        ),
        Changed<Interaction>,
    >,
) {
    for (bevy_entity, ui_entity, interaction, actions, mut state) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                if !state.hovered {
                    emit_button_action(
                        &mut queued_actions,
                        bevy_entity,
                        *ui_entity,
                        actions,
                        LyShineUiActionDispatch::Immediate,
                        LyShineUiActionPhase::HoverStart,
                        actions.hover_start_action_name.as_deref(),
                    );
                    state.hovered = true;
                }
                if !state.pressed {
                    emit_button_action(
                        &mut queued_actions,
                        bevy_entity,
                        *ui_entity,
                        actions,
                        LyShineUiActionDispatch::Queued,
                        LyShineUiActionPhase::Pressed,
                        actions.pressed_action_name.as_deref(),
                    );
                    state.pressed = true;
                }
            }
            Interaction::Hovered => {
                if !state.hovered {
                    emit_button_action(
                        &mut queued_actions,
                        bevy_entity,
                        *ui_entity,
                        actions,
                        LyShineUiActionDispatch::Immediate,
                        LyShineUiActionPhase::HoverStart,
                        actions.hover_start_action_name.as_deref(),
                    );
                    state.hovered = true;
                }
                if state.pressed {
                    emit_button_action(
                        &mut queued_actions,
                        bevy_entity,
                        *ui_entity,
                        actions,
                        LyShineUiActionDispatch::Queued,
                        LyShineUiActionPhase::Released,
                        actions.released_action_name.as_deref(),
                    );
                    emit_button_action(
                        &mut queued_actions,
                        bevy_entity,
                        *ui_entity,
                        actions,
                        LyShineUiActionDispatch::Queued,
                        LyShineUiActionPhase::Click,
                        actions.action_name.as_deref(),
                    );
                    state.pressed = false;
                }
            }
            Interaction::None => {
                if state.pressed {
                    emit_button_action(
                        &mut queued_actions,
                        bevy_entity,
                        *ui_entity,
                        actions,
                        LyShineUiActionDispatch::Queued,
                        LyShineUiActionPhase::Released,
                        actions.released_action_name.as_deref(),
                    );
                    emit_button_action(
                        &mut queued_actions,
                        bevy_entity,
                        *ui_entity,
                        actions,
                        LyShineUiActionDispatch::Queued,
                        LyShineUiActionPhase::Click,
                        actions.action_name.as_deref(),
                    );
                    state.pressed = false;
                }
                if state.hovered {
                    emit_button_action(
                        &mut queued_actions,
                        bevy_entity,
                        *ui_entity,
                        actions,
                        LyShineUiActionDispatch::Immediate,
                        LyShineUiActionPhase::HoverEnd,
                        actions.hover_end_action_name.as_deref(),
                    );
                    state.hovered = false;
                }
            }
        }
    }
}

fn emit_button_action(
    queued_actions: &mut LyShineQueuedUiActions,
    bevy_entity: Entity,
    ui_entity: LyShineUiEntity,
    actions: &LyShineButtonActions,
    dispatch: LyShineUiActionDispatch,
    phase: LyShineUiActionPhase,
    action_name: Option<&str>,
) {
    let Some(action_name) = action_name
        .map(str::trim)
        .filter(|action_name| !action_name.is_empty())
    else {
        return;
    };
    let (target_scope, callback_name) = split_action_name(action_name);
    debug!(
        "LyShine UI button action queued UiCanvasNotificationBus::OnAction canvas_root={:?} entity={:?} entity_id={} dispatch={} interaction_phase={} action_name={} target_scope={} callback_name={} use_click_behavior={} click_sq_tolerance={} right_action_name={:?} right_pressed_action_name={:?}",
        actions.canvas_root,
        bevy_entity,
        ui_entity.entity_id.as_u64(),
        dispatch.as_str(),
        phase.as_str(),
        action_name,
        target_scope,
        callback_name,
        actions.use_click_behavior,
        actions.click_sq_tolerance,
        actions.action_name_right.as_deref(),
        actions.action_name_pressed_right.as_deref(),
    );
    queued_actions.push(LyShineUiAction {
        canvas_root: actions.canvas_root,
        source_entity: bevy_entity,
        source_ui_entity: ui_entity.entity_id,
        dispatch,
        phase,
        action_name: action_name.into(),
        target_scope: target_scope.into(),
        callback_name: callback_name.into(),
    });
}

pub fn dispatch_queued_ui_actions(
    mut queued_actions: ResMut<LyShineQueuedUiActions>,
    mut dispatched_actions: ResMut<LyShineDispatchedUiActions>,
    canvas_roots: Query<&LyShineCanvasRoot>,
    entity_debug: Query<&LyShineUiEntityDebugInfo>,
) {
    while let Some(action) = queued_actions.pop() {
        let canvas_path = canvas_roots
            .get(action.canvas_root)
            .map_or("<missing canvas root>", |root| root.asset_path);
        let (source_name, source_script) = entity_debug
            .get(action.source_entity)
            .map_or((None, None), |debug| {
                (debug.name.as_deref(), debug.script.as_deref())
            });
        debug!(
            "Dispatching LyShine UiCanvasNotificationBus::OnAction canvas_root={:?} canvas_path={} source_entity={:?} source_entity_id={} source_name={} source_script={} dispatch={} interaction_phase={} action_name={} target_scope={} callback_name={}",
            action.canvas_root,
            canvas_path,
            action.source_entity,
            action.source_ui_entity.as_u64(),
            source_name.unwrap_or("<unnamed>"),
            source_script.unwrap_or("none"),
            action.dispatch.as_str(),
            action.phase.as_str(),
            action.action_name,
            action.target_scope,
            action.callback_name,
        );
        dispatched_actions.actions.push(action);
    }
}

fn split_action_name(action_name: &str) -> (&str, &str) {
    action_name
        .split_once(':')
        .unwrap_or(("entity", action_name))
}

fn ui_script_binding(script: &UiScript) -> Option<LyShineUiScriptBinding> {
    let source_script = script.script.as_deref()?.trim();
    let asset_path = lyshine_script_asset_path(source_script)?;
    Some(LyShineUiScriptBinding {
        source_script: source_script.into(),
        asset_path,
        context_id: script.context_id,
        run_on_client: script.run_on_client,
        run_on_server: script.run_on_server,
        net_sync_enabled: script.net_bindable.is_net_sync_enabled,
    })
}

fn box_action(action_name: Option<&str>) -> Option<Box<str>> {
    action_name
        .map(str::trim)
        .filter(|action_name| !action_name.is_empty())
        .map(Into::into)
}

fn format_button(button: Option<&UiButton>) -> String {
    let Some(button) = button else {
        return "none".into();
    };
    format!(
        "hover_start={:?} hover_end={:?} pressed={:?} released={:?} action={:?} action_right={:?} action_pressed_right={:?} use_click_behavior={} click_sq_tolerance={}",
        button.hover_start_action_name,
        button.hover_end_action_name,
        button.pressed_action_name,
        button.released_action_name,
        button.action_name,
        button.action_name_right,
        button.action_name_pressed_right,
        button.use_click_behavior,
        button.click_sq_tolerance,
    )
}

fn format_mask(mask: Option<&crate::UiMask>) -> String {
    let Some(mask) = mask else {
        return "none".into();
    };
    format!(
        "enable_masking={} mask_interaction={} child_mask={} rtt={} draw_behind={} draw_in_front={} alpha_test={}",
        mask.enable_masking,
        mask.mask_interaction,
        mask.child_mask_element.as_u64(),
        mask.use_render_to_texture,
        mask.draw_behind,
        mask.draw_in_front,
        mask.use_alpha_test,
    )
}

fn format_layout_axis(layout: Option<&crate::UiLayoutAxis>) -> String {
    let Some(layout) = layout else {
        return "none".into();
    };
    format!(
        "padding=({:.1},{:.1},{:.1},{:.1}) spacing={:.1} order={} h_align={} v_align={} ignore_default_cells={}",
        layout.padding.left,
        layout.padding.top,
        layout.padding.right,
        layout.padding.bottom,
        layout.spacing,
        layout.order,
        layout.child_h_alignment,
        layout.child_v_alignment,
        layout.ignore_default_layout_cells,
    )
}

fn format_layout_grid(layout: Option<&crate::UiLayoutGrid>) -> String {
    let Some(layout) = layout else {
        return "none".into();
    };
    format!(
        "padding=({:.1},{:.1},{:.1},{:.1}) spacing=({:.1},{:.1}) cell_size=({:.1},{:.1}) h_order={} v_order={} starting_with={} h_align={} v_align={}",
        layout.padding.left,
        layout.padding.top,
        layout.padding.right,
        layout.padding.bottom,
        layout.spacing.x,
        layout.spacing.y,
        layout.cell_size.x,
        layout.cell_size.y,
        layout.horizontal_order,
        layout.vertical_order,
        layout.starting_with,
        layout.child_h_alignment,
        layout.child_v_alignment,
    )
}

fn format_layout_cell(cell: Option<&UiLayoutCell>) -> String {
    let Some(cell) = cell else {
        return "none".into();
    };
    format!(
        "min=({}:{:.1},{}:{:.1}) target=({}:{:.1},{}:{:.1}) max=({}:{:.1},{}:{:.1}) extra=({}:{:.3},{}:{:.3})",
        cell.min_width_overridden,
        cell.min_width,
        cell.min_height_overridden,
        cell.min_height,
        cell.target_width_overridden,
        cell.target_width,
        cell.target_height_overridden,
        cell.target_height,
        cell.max_width_overridden,
        cell.max_width,
        cell.max_height_overridden,
        cell.max_height,
        cell.extra_width_ratio_overridden,
        cell.extra_width_ratio,
        cell.extra_height_ratio_overridden,
        cell.extra_height_ratio,
    )
}

fn format_script(script: Option<&crate::UiScript>) -> String {
    let Some(script) = script else {
        return "none".into();
    };
    format!(
        "context={} script={:?} run_server={} run_client={} net_sync={} root_group={} property_count={} group_count={}",
        script.context_id,
        script.script,
        script.run_on_server,
        script.run_on_client,
        script.net_bindable.is_net_sync_enabled,
        script.properties.name,
        script.properties.properties.len(),
        script.properties.groups.len(),
    )
}

/// The system parameters [`start_bink_startup_video`] pulls out of the
/// world by hand.
///
/// It runs as an exclusive `&mut World` system, so it cannot take these
/// as ordinary system parameters.
type BinkStartupParams<'w, 's> = (
    Commands<'w, 's>,
    Option<Res<'w, LyShineBinkStartupVideo>>,
    ResMut<'w, LyShineBinkStartupState>,
    ResMut<'w, LyShineCanvasEnabledState>,
    ResMut<'w, Assets<Image>>,
);

pub fn start_bink_startup_video(world: &mut World) {
    let mut system_state: SystemState<BinkStartupParams<'static, 'static>> =
        SystemState::new(world);

    let Ok(params) = system_state.get_mut(world) else {
        warn!(
            target: "az_gem_lyshine",
            "failed to acquire bink startup video system state"
        );
        return;
    };
    let Some(playback_to_insert) = mount_bink_startup_overlay(params) else {
        return;
    };
    let overlay = playback_to_insert.overlay;
    let playback_canvas_path = playback_to_insert.playback_canvas_path;

    trace!(
        target: "az_gem_lyshine",
        overlay = ?overlay,
        canvas_path = playback_canvas_path,
        "applying bink startup overlay commands"
    );
    system_state.apply(world);
    trace!(
        target: "az_gem_lyshine",
        overlay = ?overlay,
        canvas_path = playback_canvas_path,
        "applied bink startup overlay commands"
    );

    world.insert_non_send(playback_to_insert);
    trace!(
        target: "az_gem_lyshine",
        canvas_path = playback_canvas_path,
        "inserted non-send bink startup playback"
    );
}

/// Open the startup video and mount its full-screen overlay.
///
/// Returns `None` — always after logging why — when the startup video
/// resource is absent, playback was already attempted or completed, the
/// file is missing on disk, the native bink runtime or the video itself
/// fails to open, or the frame buffer its dimensions ask for cannot be
/// addressed on this target.
fn mount_bink_startup_overlay(
    params: BinkStartupParams<'_, '_>,
) -> Option<LyShineBinkVideoPlayback> {
    let (mut commands, startup_video, mut startup_state, mut enabled_state, mut images) = params;

    if startup_state.attempted || startup_state.completed {
        return None;
    }

    let startup_video = startup_video?;

    startup_state.attempted = true;

    if !startup_video.filesystem_path.is_file() {
        warn!(
            target: "az_gem_lyshine",
            asset_path = startup_video.asset_path,
            filesystem_path = %startup_video.filesystem_path.display(),
            "bink startup video file missing"
        );
        return None;
    }

    let runtime = match BinkRuntime::load_default() {
        Ok(runtime) => runtime,
        Err(error) => {
            warn!(
                target: "az_gem_lyshine",
                asset_path = startup_video.asset_path,
                filesystem_path = %startup_video.filesystem_path.display(),
                %error,
                "failed to load native bink runtime"
            );
            return None;
        }
    };

    let sound_status = runtime.sound_system_status();
    let opened = if let Some(planner) = startup_video.audio_planner {
        runtime.open_with_audio_plan(
            &startup_video.filesystem_path,
            startup_video.probe_open_flags,
            startup_video.playback_open_flags,
            planner,
        )
    } else {
        runtime.open(
            &startup_video.filesystem_path,
            startup_video.playback_open_flags,
        )
    };
    let video = match opened {
        Ok(mut video) => {
            if startup_video.audio_planner.is_none() {
                video.set_sound_enabled(true);
            }
            video
        }
        Err(error) => {
            warn!(
                target: "az_gem_lyshine",
                asset_path = startup_video.asset_path,
                filesystem_path = %startup_video.filesystem_path.display(),
                %error,
                "failed to open bink startup video"
            );
            return None;
        }
    };

    let frame_texture = images.add(bink_frame_image(&startup_video, &video)?);

    enabled_state.enable_only(startup_video.playback_canvas_path);

    let overlay = spawn_bink_overlay(&mut commands, &startup_video, &video, &frame_texture);
    log_bink_overlay_mounted(&startup_video, &sound_status, &video, overlay);

    Some(LyShineBinkVideoPlayback {
        video,
        frame_texture,
        overlay,
        frames_decoded: 0,
        asset_path: startup_video.asset_path,
        playback_canvas_path: startup_video.playback_canvas_path,
        next_canvas_path: startup_video.next_canvas_path,
    })
}

/// Allocate the CPU-side RGBA buffer the decoder writes each frame into.
///
/// Returns `None`, after a warning, when `width * height * 4` overflows
/// what a `Vec` can address on this target.
fn bink_frame_image(startup_video: &LyShineBinkStartupVideo, video: &BinkVideo) -> Option<Image> {
    let width = video.width();
    let height = video.height();
    let Some(data_size) = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
    else {
        warn!(
            target: "az_gem_lyshine",
            asset_path = startup_video.asset_path,
            width,
            height,
            "invalid bink startup video dimensions"
        );
        return None;
    };

    let size = Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let mut image = Image::new(
        size,
        TextureDimension::D2,
        vec![0; data_size],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    Some(image)
}

/// Mount the full-screen node the decoded frames are displayed on.
fn spawn_bink_overlay(
    commands: &mut Commands<'_, '_>,
    startup_video: &LyShineBinkStartupVideo,
    video: &BinkVideo,
    frame_texture: &Handle<Image>,
) -> Entity {
    commands
        .spawn((
            Name::new("LyShine Bink Startup Video"),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            ImageNode::new(frame_texture.clone()),
            LyShineBinkFrameTexture {
                _asset_path: startup_video.asset_path,
                _source_width: video.width(),
                _source_height: video.height(),
                _frame_count: video.frame_count(),
            },
            GlobalZIndex(i32::MAX),
            Visibility::Visible,
        ))
        .id()
}

/// Record everything the bink overlay was mounted with, in one event.
fn log_bink_overlay_mounted(
    startup_video: &LyShineBinkStartupVideo,
    sound_status: &BinkSoundSystemStatus,
    video: &BinkVideo,
    overlay: Entity,
) {
    let audio_info = video.audio_info();
    info!(
        target: "az_gem_lyshine",
        asset_path = startup_video.asset_path,
        filesystem_path = %startup_video.filesystem_path.display(),
        sound_system = ?sound_status.selected,
        xaudio2_result = sound_status.xaudio2_result,
        direct_sound_result = ?sound_status.direct_sound_result,
        bink_open_flags = format_args!("{:#x}", audio_info.open_flags),
        audio_track_count = audio_info.track_count,
        audio_track_ids = ?audio_info.track_ids,
        audio_track_info = ?audio_info.track_info,
        bink_sound_enabled = audio_info.sound_enabled,
        bink_sound_size = video.sound_size(),
        width = video.width(),
        height = video.height(),
        frame_count = video.frame_count(),
        overlay = ?overlay,
        canvas_path = startup_video.playback_canvas_path,
        x = 0.0f32,
        y = 0.0f32,
        width_percent = 100.0f32,
        height_percent = 100.0f32,
        z = i32::MAX,
        "mounted bink startup video overlay"
    );
}

pub fn advance_bink_startup_video(
    mut commands: Commands,
    mut startup_state: ResMut<LyShineBinkStartupState>,
    mut enabled_state: ResMut<LyShineCanvasEnabledState>,
    playback: Option<NonSendMut<LyShineBinkVideoPlayback>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut playback) = playback else {
        return;
    };

    trace!(
        target: "az_gem_lyshine",
        asset_path = playback.asset_path,
        filesystem_path = %playback.video.path().display(),
        frames_decoded = playback.frames_decoded,
        frame_count = playback.video.frame_count(),
        bink_sound_size = playback.video.sound_size(),
        overlay = ?playback.overlay,
        "advancing bink startup video"
    );

    if playback.frames_decoded >= playback.video.frame_count() {
        info!(
            target: "az_gem_lyshine",
            asset_path = playback.asset_path,
            filesystem_path = %playback.video.path().display(),
            frames_decoded = playback.frames_decoded,
            frame_count = playback.video.frame_count(),
            bink_sound_size = playback.video.sound_size(),
            overlay = ?playback.overlay,
            canvas_path = playback.next_canvas_path,
            "completed bink startup video"
        );
        finish_bink_startup_playback(
            &mut commands,
            &mut enabled_state,
            &mut startup_state,
            playback.overlay,
            playback.next_canvas_path,
        );
        return;
    }

    trace!(
        target: "az_gem_lyshine",
        asset_path = playback.asset_path,
        frames_decoded = playback.frames_decoded,
        "checking bink startup wait state"
    );
    if playback.video.should_wait() {
        trace!(
            target: "az_gem_lyshine",
            asset_path = playback.asset_path,
            frames_decoded = playback.frames_decoded,
            "bink startup video waiting"
        );
        return;
    }

    match decode_next_bink_frame(&mut playback, &mut images) {
        BinkFrameStep::Decoded => {}
        BinkFrameStep::Deferred => return,
        BinkFrameStep::Failed => {
            finish_bink_startup_playback(
                &mut commands,
                &mut enabled_state,
                &mut startup_state,
                playback.overlay,
                playback.next_canvas_path,
            );
            return;
        }
    }

    playback.frames_decoded += 1;

    trace!(
        target: "az_gem_lyshine",
        asset_path = playback.asset_path,
        filesystem_path = %playback.video.path().display(),
        frame = playback.frames_decoded,
        frame_count = playback.video.frame_count(),
        source_width = playback.video.width(),
        source_height = playback.video.height(),
        bink_sound_size = playback.video.sound_size(),
        overlay = ?playback.overlay,
        x = 0.0f32,
        y = 0.0f32,
        width_percent = 100.0f32,
        height_percent = 100.0f32,
        z = i32::MAX,
        "rendered bink startup video frame"
    );
}

/// What one [`decode_next_bink_frame`] call managed to do.
enum BinkFrameStep {
    /// The frame landed in the overlay texture's CPU upload buffer.
    Decoded,
    /// The texture was unavailable this tick; nothing to tear down.
    Deferred,
    /// The decoder failed; playback has to end.
    Failed,
}

/// Decode one frame into the overlay texture's CPU upload buffer.
fn decode_next_bink_frame(
    playback: &mut LyShineBinkVideoPlayback,
    images: &mut Assets<Image>,
) -> BinkFrameStep {
    trace!(
        target: "az_gem_lyshine",
        asset_path = playback.asset_path,
        frames_decoded = playback.frames_decoded,
        "accessing bink startup frame texture"
    );
    let Some(mut frame_texture) = images.get_mut(&playback.frame_texture) else {
        warn!(
            target: "az_gem_lyshine",
            asset_path = playback.asset_path,
            overlay = ?playback.overlay,
            "bink startup video frame texture disappeared"
        );
        return BinkFrameStep::Deferred;
    };

    let Some(data) = frame_texture.data.as_mut() else {
        warn!(
            target: "az_gem_lyshine",
            asset_path = playback.asset_path,
            overlay = ?playback.overlay,
            "bink startup video frame texture has no CPU upload buffer"
        );
        return BinkFrameStep::Deferred;
    };

    trace!(
        target: "az_gem_lyshine",
        asset_path = playback.asset_path,
        frames_decoded = playback.frames_decoded,
        bytes = data.len(),
        "decoding bink startup frame"
    );
    if let Err(error) = playback.video.decode_next_frame_rgba(data) {
        warn!(
            target: "az_gem_lyshine",
            asset_path = playback.asset_path,
            filesystem_path = %playback.video.path().display(),
            frames_decoded = playback.frames_decoded,
            frame_count = playback.video.frame_count(),
            %error,
            "failed to decode bink startup video frame"
        );
        return BinkFrameStep::Failed;
    }
    BinkFrameStep::Decoded
}

/// Despawn the bink overlay and hand the screen to the landing canvas.
fn finish_bink_startup_playback(
    commands: &mut Commands<'_, '_>,
    enabled_state: &mut LyShineCanvasEnabledState,
    startup_state: &mut LyShineBinkStartupState,
    overlay: Entity,
    next_canvas_path: &'static str,
) {
    commands.entity(overlay).despawn();
    commands.queue(|world: &mut World| {
        world.remove_non_send::<LyShineBinkVideoPlayback>();
    });
    enabled_state.enable_only(next_canvas_path);
    startup_state.completed = true;
}

struct BoundAtlasRegion<'a> {
    image_path: &'a str,
    layout: Handle<TextureAtlasLayout>,
    index: usize,
}

enum AtlasBinding<'a> {
    Bound(BoundAtlasRegion<'a>),
    Pending,
    NoRegion,
}

fn bindable_atlas_region<'a>(
    sprite_path: &str,
    canvas_atlases: &'a LyShineCanvasTextureAtlases,
    atlas_assets: &'a Assets<TextureAtlasAsset>,
    atlas_layouts: &mut Assets<TextureAtlasLayout>,
    layout_cache: &mut LyShineTextureAtlasLayouts,
) -> AtlasBinding<'a> {
    let mut pending = false;
    for handle in &canvas_atlases.handles {
        let Some(atlas) = atlas_assets.get(handle) else {
            pending = true;
            continue;
        };
        let Some(region) = atlas.find_region(sprite_path) else {
            continue;
        };
        let layout = layout_cache
            .layouts
            .entry(handle.id())
            .or_insert_with(|| atlas_layouts.add(atlas.layout.clone()))
            .clone();
        return AtlasBinding::Bound(BoundAtlasRegion {
            image_path: atlas.image_path(),
            layout,
            index: region.index,
        });
    }
    if pending {
        AtlasBinding::Pending
    } else {
        AtlasBinding::NoRegion
    }
}

/// The UI node an image binding lands on, carried together so the debug
/// log can name the canvas, entity, layout and sprite it rewrote.
struct ImageBindSite<'a> {
    canvas_root: Entity,
    entity: Entity,
    entity_id: UiEntityId,
    node: &'a Node,
    sprite_path: &'a str,
}

fn bind_atlas_image(
    asset_server: &AssetServer,
    image_node: &mut ImageNode,
    site: &ImageBindSite<'_>,
    bound: BoundAtlasRegion<'_>,
) {
    let texture = asset_server.load(bound.image_path.to_string());
    let atlas = TextureAtlas {
        layout: bound.layout,
        index: bound.index,
    };
    if image_node.image.id() == texture.id()
        && image_node.texture_atlas.as_ref().is_some_and(|current| {
            current.layout.id() == atlas.layout.id() && current.index == atlas.index
        })
    {
        return;
    }
    debug!(
        "Bound LyShine atlas image canvas_root={:?} entity={:?} entity_id={} node={} sprite_path={} atlas_image={} atlas_index={}",
        site.canvas_root,
        site.entity,
        site.entity_id.as_u64(),
        format_node(site.node),
        site.sprite_path,
        bound.image_path,
        bound.index
    );
    image_node.image = texture;
    image_node.texture_atlas = Some(atlas);
}

fn bind_direct_image(
    asset_server: &AssetServer,
    image_node: &mut ImageNode,
    site: &ImageBindSite<'_>,
) -> Handle<Image> {
    let texture = asset_server.load(site.sprite_path.to_string());
    if image_node.image.id() == texture.id() && image_node.texture_atlas.is_none() {
        return texture;
    }
    debug!(
        "Bound LyShine direct image canvas_root={:?} entity={:?} entity_id={} node={} image_path={}",
        site.canvas_root,
        site.entity,
        site.entity_id.as_u64(),
        format_node(site.node),
        site.sprite_path
    );
    image_node.image = texture.clone();
    image_node.texture_atlas = None;
    texture
}

fn root_entity_ids(canvas_asset: &UiCanvasAsset) -> Vec<UiEntityId> {
    if !canvas_asset.canvas.root_entity.is_null() {
        return vec![canvas_asset.canvas.root_entity];
    }

    let mut roots = Vec::new();
    let mut child_ids = HashSet::new();
    for entity in &canvas_asset.entities {
        if let Some(element) = entity.element.as_ref() {
            child_ids.extend(element.child_order.iter().map(|child| child.entity_id));
        }
    }

    for entity in &canvas_asset.entities {
        if !child_ids.contains(&entity.entity_id) && !roots.contains(&entity.entity_id) {
            roots.push(entity.entity_id);
        }
    }
    if roots.is_empty() {
        roots.extend(canvas_asset.entities.iter().map(|entity| entity.entity_id));
    }
    roots
}

fn ordered_children<'a>(
    entity: &UiEntity,
    entities_by_id: &HashMap<UiEntityId, &'a UiEntity>,
) -> Vec<&'a UiEntity> {
    let Some(element) = entity.element.as_ref() else {
        return Vec::new();
    };

    let mut children: Vec<_> = element
        .child_order
        .iter()
        .filter_map(|child| {
            entities_by_id
                .get(&child.entity_id)
                .copied()
                .map(|entity| (child.sort_index, entity))
        })
        .collect();
    children.sort_by_key(|(sort_index, _)| *sort_index);
    children.into_iter().map(|(_, entity)| entity).collect()
}

fn canvas_root_node() -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(0.0),
        top: Val::Px(0.0),
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..Default::default()
    }
}

const fn parent_layout_for(entity: &UiEntity) -> Option<LyShineParentLayout> {
    if entity.layout_row.is_some() {
        Some(LyShineParentLayout::Row)
    } else if entity.layout_column.is_some() {
        Some(LyShineParentLayout::Column)
    } else if entity.layout_grid.is_some() {
        Some(LyShineParentLayout::Grid)
    } else {
        None
    }
}

const fn apply_mask_node(entity: &UiEntity, node: &mut Node) {
    let Some(mask) = entity.mask.as_ref() else {
        return;
    };
    if mask.enable_masking {
        node.overflow = Overflow::clip();
    }
}

fn apply_layout_container_node(entity: &UiEntity, node: &mut Node) {
    if let Some(layout) = entity.layout_row.as_ref() {
        node.display = Display::Flex;
        node.flex_direction = FlexDirection::Row;
        node.padding = ui_padding(layout.padding);
        node.column_gap = Val::Px(layout.spacing);
        node.justify_content = horizontal_justify_content(layout.child_h_alignment);
        node.align_items = vertical_align_items(layout.child_v_alignment);
    }
    if let Some(layout) = entity.layout_column.as_ref() {
        node.display = Display::Flex;
        node.flex_direction = FlexDirection::Column;
        node.padding = ui_padding(layout.padding);
        node.row_gap = Val::Px(layout.spacing);
        node.justify_content = vertical_justify_content(layout.child_v_alignment);
        node.align_items = horizontal_align_items(layout.child_h_alignment);
    }
    if let Some(layout) = entity.layout_grid.as_ref() {
        node.display = Display::Grid;
        node.padding = ui_padding(layout.padding);
        node.column_gap = Val::Px(layout.spacing.x);
        node.row_gap = Val::Px(layout.spacing.y);
        node.justify_items = horizontal_justify_items(layout.child_h_alignment);
        node.align_items = vertical_align_items(layout.child_v_alignment);
        if layout.cell_size.x > f32::EPSILON {
            node.grid_auto_columns = vec![GridTrack::px(layout.cell_size.x)];
        }
        if layout.cell_size.y > f32::EPSILON {
            node.grid_auto_rows = vec![GridTrack::px(layout.cell_size.y)];
        }
    }
}

const fn apply_layout_child_node(
    entity: &UiEntity,
    parent_layout: Option<LyShineParentLayout>,
    node: &mut Node,
) {
    let Some(parent_layout) = parent_layout else {
        return;
    };
    node.position_type = PositionType::Relative;
    node.left = Val::Auto;
    node.right = Val::Auto;
    node.top = Val::Auto;
    node.bottom = Val::Auto;
    // node_from_transform put the LyShine pixel offsets on `margin` for
    // absolute positioning. Inside a flex parent the entity is laid out by
    // the parent row/column/grid, so the anchor-derived margin doesn't apply.
    node.margin = bevy::ui::UiRect::ZERO;

    if let Some(layout_cell) = entity.layout_cell.as_ref() {
        apply_layout_cell_node(layout_cell, parent_layout, node);
    }
}

const fn apply_layout_cell_node(
    cell: &UiLayoutCell,
    parent_layout: LyShineParentLayout,
    node: &mut Node,
) {
    if cell.min_width_overridden {
        node.min_width = Val::Px(cell.min_width);
    }
    if cell.min_height_overridden {
        node.min_height = Val::Px(cell.min_height);
    }
    if cell.target_width_overridden {
        node.width = Val::Px(cell.target_width);
    }
    if cell.target_height_overridden {
        node.height = Val::Px(cell.target_height);
    }
    if cell.max_width_overridden {
        node.max_width = Val::Px(cell.max_width);
    }
    if cell.max_height_overridden {
        node.max_height = Val::Px(cell.max_height);
    }
    match parent_layout {
        LyShineParentLayout::Row => {
            if cell.extra_width_ratio_overridden {
                node.flex_grow = cell.extra_width_ratio.max(0.0);
            }
        }
        LyShineParentLayout::Column => {
            if cell.extra_height_ratio_overridden {
                node.flex_grow = cell.extra_height_ratio.max(0.0);
            }
        }
        LyShineParentLayout::Grid => {}
    }
}

const fn ui_padding(rect: crate::UiRect) -> bevy::ui::UiRect {
    bevy::ui::UiRect {
        left: Val::Px(rect.left),
        top: Val::Px(rect.top),
        right: Val::Px(rect.right),
        bottom: Val::Px(rect.bottom),
    }
}

const fn horizontal_justify_content(value: i32) -> JustifyContent {
    match value {
        1 => JustifyContent::Center,
        2 => JustifyContent::FlexEnd,
        _ => JustifyContent::FlexStart,
    }
}

const fn vertical_justify_content(value: i32) -> JustifyContent {
    match value {
        1 => JustifyContent::Center,
        2 => JustifyContent::FlexEnd,
        _ => JustifyContent::FlexStart,
    }
}

const fn horizontal_align_items(value: i32) -> AlignItems {
    match value {
        1 => AlignItems::Center,
        2 => AlignItems::FlexEnd,
        _ => AlignItems::FlexStart,
    }
}

const fn vertical_align_items(value: i32) -> AlignItems {
    match value {
        1 => AlignItems::Center,
        2 => AlignItems::FlexEnd,
        _ => AlignItems::FlexStart,
    }
}

const fn horizontal_justify_items(value: i32) -> JustifyItems {
    match value {
        1 => JustifyItems::Center,
        2 => JustifyItems::End,
        _ => JustifyItems::Start,
    }
}

fn format_node(node: &Node) -> String {
    format!(
        "display={:?} pos={:?} overflow={:?} left={:?} right={:?} width={:?} min_width={:?} max_width={:?} top={:?} bottom={:?} height={:?} min_height={:?} max_height={:?} flex_dir={:?} flex_grow={:.3} margin={:?} padding={:?} row_gap={:?} column_gap={:?}",
        node.display,
        node.position_type,
        node.overflow,
        node.left,
        node.right,
        node.width,
        node.min_width,
        node.max_width,
        node.top,
        node.bottom,
        node.height,
        node.min_height,
        node.max_height,
        node.flex_direction,
        node.flex_grow,
        node.margin,
        node.padding,
        node.row_gap,
        node.column_gap
    )
}

fn node_from_transform(transform: Option<&UiTransform2d>) -> Node {
    let Some(transform) = transform else {
        return canvas_root_node();
    };
    let horizontal = axis_layout(
        transform.anchors.left,
        transform.anchors.right,
        transform.offsets.left,
        transform.offsets.right,
    );
    let vertical = axis_layout(
        transform.anchors.top,
        transform.anchors.bottom,
        transform.offsets.top,
        transform.offsets.bottom,
    );

    Node {
        position_type: PositionType::Absolute,
        left: horizontal.start,
        right: horizontal.end,
        width: horizontal.size,
        top: vertical.start,
        bottom: vertical.end,
        height: vertical.size,
        // Pixel offsets ride on `margin` so they stack on top of percentage
        // anchors (CSS-style `left: 50%; margin-left: -W/2` centering).
        margin: bevy::ui::UiRect {
            left: Val::Px(horizontal.start_margin),
            right: Val::Px(horizontal.end_margin),
            top: Val::Px(vertical.start_margin),
            bottom: Val::Px(vertical.end_margin),
        },
        ..Default::default()
    }
}

fn format_transform(transform: Option<&UiTransform2d>) -> String {
    let Some(transform) = transform else {
        return "none".to_string();
    };
    format!(
        "anchors=({:.3},{:.3},{:.3},{:.3}) offsets=({:.3},{:.3},{:.3},{:.3}) pivot=({:.3},{:.3}) rotation={:.3} scale=({:.3},{:.3}) scale_to_device={} compute_when_hidden={}",
        transform.anchors.left,
        transform.anchors.top,
        transform.anchors.right,
        transform.anchors.bottom,
        transform.offsets.left,
        transform.offsets.top,
        transform.offsets.right,
        transform.offsets.bottom,
        transform.pivot.x,
        transform.pivot.y,
        transform.rotation,
        transform.scale.x,
        transform.scale.y,
        transform.scale_to_device,
        transform.compute_transform_when_hidden
    )
}

fn format_element(element: Option<&crate::UiElement>) -> String {
    let Some(element) = element else {
        return "none".to_string();
    };
    let child_order = element
        .child_order
        .iter()
        .map(|child| format!("{}:{}", child.entity_id.as_u64(), child.sort_index))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "local_id={} enabled={} render_priority={} sortable={} multithread_children={} child_order=[{}]",
        element.local_id,
        element.enabled,
        element.render_priority,
        element.children_render_sortable,
        element.multithread_children,
        child_order
    )
}

fn format_image(image: Option<&UiImage>) -> String {
    let Some(image) = image else {
        return "none".to_string();
    };
    format!(
        "sprite_type={:?} sprite_path={:?} sprite_index={} render_target={:?} srgb={} color=({:.3},{:.3},{:.3},{:.3}) alpha={:.3} image_type={:?} fill_center={} stretch_sliced={} blend={:?} fill_type={:?} fill_amount={:.3} fill_start_angle={:.3} fill_corner={:?} fill_edge={:?} fill_clockwise={}",
        image.sprite_type,
        image.sprite_path,
        image.sprite_index,
        image.render_target_name,
        image.render_target_srgb,
        image.color.red,
        image.color.green,
        image.color.blue,
        image.color.alpha,
        image.alpha,
        image.image_type,
        image.fill_center,
        image.stretch_sliced,
        image.blend_mode,
        image.fill_type,
        image.fill_amount,
        image.fill_start_angle,
        image.fill_corner_origin,
        image.fill_edge_origin,
        image.fill_clockwise
    )
}

fn format_text(text: Option<&UiText>) -> String {
    let Some(text) = text else {
        return "none".to_string();
    };
    format!(
        "len={} value={} markup={} images={} update_on_input={} font={:?} font_effect={} font_size={:.3} color=({:.3},{:.3},{:.3},{:.3}) alpha={:.3} char_spacing={:.3} line_spacing={:.3} h_align={} v_align={} wrap={} overflow={}",
        text.text.chars().count(),
        compact_text(&text.text),
        text.markup_enabled,
        text.images_enabled,
        text.update_on_input_change,
        text.font_path,
        text.font_effect_index,
        text.font_size,
        text.color.red,
        text.color.green,
        text.color.blue,
        text.color.alpha,
        text.alpha,
        text.character_spacing,
        text.line_spacing,
        text.horizontal_alignment,
        text.vertical_alignment,
        text.wrap_text_setting,
        text.overflow_mode
    )
}

fn compact_text(value: &str) -> String {
    const LIMIT: usize = 120;
    let mut compact = value.replace(['\r', '\n', '\t'], " ");
    if compact.chars().count() > LIMIT {
        compact = compact.chars().take(LIMIT).collect::<String>();
        compact.push_str("...");
    }
    format!("{compact:?}")
}

struct AxisLayout {
    /// Distance from this edge of parent to corresponding edge of child as a percentage.
    start: Val,
    /// Pixel offset added to `start` (uses `margin` so it stacks on top of `Percent`).
    start_margin: f32,
    /// Distance from far edge of parent to far edge of child as a percentage.
    end: Val,
    /// Pixel offset added to `end` (uses `margin` so it stacks on top of `Percent`).
    end_margin: f32,
    /// Width or height of the child.
    size: Val,
}

/// Convert a single `LyShine` axis (anchors + offsets) into Bevy `Node` insets.
///
/// `LyShine`'s anchor rect math:
///   `start_edge` = `parent_size` * `start_anchor` + `start_offset`
///   `end_edge`   = `parent_size` * `end_anchor`   + `end_offset`
///
/// Bevy `Val` can't express `Percent + Px` in a single property, so the
/// percent component lands on `left/right/top/bottom` and the pixel offset is
/// emitted as `margin` (which stacks additively for absolutely-positioned
/// nodes, mirroring the CSS centering trick `left: 50%; margin-left: -W/2`).
fn axis_layout(
    start_anchor: f32,
    end_anchor: f32,
    start_offset: f32,
    end_offset: f32,
) -> AxisLayout {
    if (end_anchor - start_anchor).abs() > f32::EPSILON {
        // Stretch mode — child spans between two different parent points,
        // shifted inward by start_offset/end_offset.
        AxisLayout {
            start: Val::Percent(start_anchor * 100.0),
            start_margin: start_offset,
            end: Val::Percent((1.0 - end_anchor) * 100.0),
            end_margin: -end_offset,
            size: Val::Auto,
        }
    } else {
        // Fixed-size mode — single anchor point with the offset extent
        // defining width/height. `end` is left Auto so flexbox sizes from
        // the explicit width/height.
        let extent = (end_offset - start_offset).abs();
        let size = if extent > f32::EPSILON {
            Val::Px(extent)
        } else {
            Val::Percent(100.0)
        };
        AxisLayout {
            start: Val::Percent(start_anchor * 100.0),
            start_margin: start_offset,
            end: Val::Auto,
            end_margin: 0.0,
            size,
        }
    }
}

fn image_node(image: &UiImage, fader_alpha: f32) -> ImageNode {
    ImageNode::solid_color(linear_color(
        image.color,
        image.alpha * fader_alpha * image_fill_alpha(image),
    ))
    .with_mode(node_image_mode(image.image_type, image.stretch_sliced))
}

fn image_fill_alpha(image: &UiImage) -> f32 {
    if image_has_visible_fill(image) {
        1.0
    } else {
        0.0
    }
}

fn image_has_visible_fill(image: &UiImage) -> bool {
    image.fill_type == UiImageFillType::None || image.fill_amount > f32::EPSILON
}

const fn node_image_mode(image_type: UiImageType, stretch_sliced: bool) -> NodeImageMode {
    match image_type {
        UiImageType::Stretched | UiImageType::StretchedToFit | UiImageType::StretchedToFill => {
            NodeImageMode::Stretch
        }
        UiImageType::Fixed => NodeImageMode::Auto,
        UiImageType::Tiled => NodeImageMode::Tiled {
            tile_x: true,
            tile_y: true,
            stretch_value: 1.0,
        },
        UiImageType::Sliced => NodeImageMode::Sliced(TextureSlicer {
            border: BorderRect::ZERO,
            center_scale_mode: sliced_scale_mode(stretch_sliced),
            sides_scale_mode: sliced_scale_mode(stretch_sliced),
            max_corner_scale: 1.0,
        }),
    }
}

const fn sliced_scale_mode(stretch_sliced: bool) -> SliceScaleMode {
    if stretch_sliced {
        SliceScaleMode::Stretch
    } else {
        SliceScaleMode::Tile { stretch_value: 1.0 }
    }
}

fn text_bundle(
    asset_server: Option<&AssetServer>,
    text: &UiText,
    fader_alpha: f32,
) -> (Name, Node, Text, TextFont, TextColor) {
    let font = text
        .font_path
        .as_ref()
        .and_then(|path| asset_server.map(|asset_server| asset_server.load(path.clone())))
        .unwrap_or_default();

    (
        Name::new("UI Text"),
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..Default::default()
        },
        Text::new(text.text.clone()),
        TextFont {
            font: FontSource::Handle(font),
            font_size: FontSize::Px(text.font_size.max(1.0)),
            ..Default::default()
        },
        TextColor(linear_color(text.color, text.alpha * fader_alpha)),
    )
}

fn entity_visibility(entity: &UiEntity) -> Visibility {
    let enabled = entity
        .element
        .as_ref()
        .is_none_or(|element| element.enabled);
    let fader_visible = entity
        .fader
        .as_ref()
        .is_none_or(|fader| fader.fade > f32::EPSILON);
    let image_fill_visible = entity.image.as_ref().is_none_or(image_has_visible_fill);
    if entity.dependency_ready
        && entity.runtime_active
        && enabled
        && fader_visible
        && image_fill_visible
    {
        // Inherit from the canvas root so disabling a canvas (e.g. the
        // fullscreenvideo overlay after the Bink intro completes) hides every
        // descendant instead of leaving its `FullscreenBackground` drawing
        // over the next frontend canvas.
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

fn ui_entity_name(entity: &UiEntity) -> Name {
    entity
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .map_or_else(
            || Name::new(format!("UI Entity {}", entity.entity_id.as_u64())),
            |name| Name::new(name.to_string()),
        )
}

fn linear_color(color: LinearRgba, alpha: f32) -> Color {
    Color::linear_rgba(color.red, color.green, color.blue, color.alpha * alpha)
}

#[cfg(test)]
mod tests {
    use az_gem_texture_atlas::{NameRange, TextureAtlasAsset, TextureAtlasEntry};
    use bevy::asset::{AssetApp, AssetPlugin};
    use bevy::image::Image;
    use bevy::math::{URect, UVec2};

    use super::*;
    use crate::{UiCanvas, UiChildOrder, UiElement, UiRect as LyShineRect};

    #[test]
    fn plugin_loads_project_supplied_canvas_handles() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), crate::LyShinePlugin));
        app.insert_resource(LyShineCanvasLoadQueue {
            canvases: vec![LyShineCanvasLoadRequest {
                asset_path: "test.dynamicuicanvas",
                purpose: LyShineCanvasPurpose::Frontend,
                active_on_load: true,
            }],
        });

        app.update();

        let loaded = app.world().resource::<LyShineLoadedCanvasAssets>();
        assert_eq!(loaded.canvases.len(), 1);
        assert_eq!(
            loaded.canvases[0].request.asset_path,
            "test.dynamicuicanvas"
        );
        assert!(
            app.world()
                .resource::<LyShineCanvasEnabledState>()
                .is_enabled("test.dynamicuicanvas")
        );
    }

    #[test]
    fn spawn_loaded_canvas_asset_creates_bevy_ui_nodes() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_resource::<Assets<UiCanvasAsset>>();
        app.init_asset::<TextureAtlasAsset>();
        app.init_resource::<LyShineSpawnedCanvases>();
        // The enabled set starts empty. This test enables its canvas directly.
        let mut enabled = LyShineCanvasEnabledState::default();
        enabled.set_enabled("test.dynamicuicanvas", true);
        app.insert_resource(enabled);
        app.add_systems(Update, spawn_loaded_canvas_assets);

        let handle = app
            .world_mut()
            .resource_mut::<Assets<UiCanvasAsset>>()
            .add(test_canvas_asset());
        app.insert_resource(LyShineLoadedCanvasAssets {
            canvases: vec![LyShineLoadedCanvasAsset {
                request: LyShineCanvasLoadRequest {
                    asset_path: "test.dynamicuicanvas",
                    purpose: LyShineCanvasPurpose::Frontend,
                    active_on_load: true,
                },
                handle,
            }],
        });

        app.update();

        let world = app.world_mut();
        assert_eq!(world.query::<&LyShineCanvasRoot>().iter(world).count(), 1);
        assert_eq!(world.query::<&LyShineUiEntity>().iter(world).count(), 2);
        assert_eq!(world.query::<&ImageNode>().iter(world).count(), 1);
        assert_eq!(
            world
                .query::<&LyShineCanvasTextureAtlases>()
                .single(world)
                .unwrap()
                .handles
                .len(),
            1
        );
        assert_eq!(world.query::<&Text>().iter(world).count(), 1);
    }

    #[test]
    fn bind_loaded_canvas_images_uses_texture_atlas_products() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_asset::<Image>();
        app.init_asset::<TextureAtlasAsset>();
        app.init_asset::<TextureAtlasLayout>();
        app.init_resource::<LyShineTextureAtlasLayouts>();
        app.add_systems(Update, bind_loaded_canvas_images);

        let atlas_names = "lyshineui/images/icon".to_string().into_boxed_str();
        let atlas_name_len =
            u32::try_from(atlas_names.len()).expect("fixture atlas name table fits in u32");
        let atlas = TextureAtlasAsset::new(
            "lyshineui/images/textureatlas/common.dds",
            TextureAtlasLayout {
                size: UVec2::new(64, 64),
                textures: vec![URect {
                    min: UVec2::new(4, 8),
                    max: UVec2::new(20, 24),
                }],
            },
            atlas_names,
            vec![TextureAtlasEntry::new(NameRange::new(0, atlas_name_len))].into_boxed_slice(),
        )
        .unwrap();
        let atlas_handle = app
            .world_mut()
            .resource_mut::<Assets<TextureAtlasAsset>>()
            .add(atlas);

        let root = app
            .world_mut()
            .spawn(LyShineCanvasTextureAtlases {
                handles: vec![atlas_handle].into_boxed_slice(),
            })
            .id();
        app.world_mut().spawn((
            LyShineUiEntity {
                entity_id: UiEntityId::new(1),
            },
            Node::default(),
            LyShineImageBinding {
                canvas_root: root,
                sprite_path: "lyshineui/images/icon.dds".into(),
            },
            ImageNode::solid_color(Color::WHITE),
        ));

        app.update();

        let world = app.world_mut();
        let image_node = world.query::<&ImageNode>().single(world).unwrap();
        let atlas = image_node.texture_atlas.as_ref().unwrap();
        assert_eq!(atlas.index, 0);
        assert_eq!(world.resource::<Assets<TextureAtlasLayout>>().len(), 1);
    }

    #[test]
    fn bind_loaded_canvas_images_loads_direct_texture_product_without_atlas_region() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_asset::<Image>();
        app.init_asset::<TextureAtlasAsset>();
        app.init_asset::<TextureAtlasLayout>();
        app.init_resource::<LyShineTextureAtlasLayouts>();
        app.add_systems(Update, bind_loaded_canvas_images);

        let root = app
            .world_mut()
            .spawn(LyShineCanvasTextureAtlases {
                handles: Box::default(),
            })
            .id();
        app.world_mut().spawn((
            LyShineUiEntity {
                entity_id: UiEntityId::new(2),
            },
            Node::default(),
            LyShineImageBinding {
                canvas_root: root,
                sprite_path: "lyshineui/images/icon.dds".into(),
            },
            ImageNode::solid_color(Color::WHITE),
        ));

        app.update();

        let world = app.world_mut();
        let expected = world
            .resource::<AssetServer>()
            .load::<Image>("lyshineui/images/icon.dds");
        let image_node = world.query::<&ImageNode>().single(world).unwrap();
        assert_eq!(image_node.image.id(), expected.id());
        assert!(image_node.texture_atlas.is_none());
    }

    #[test]
    fn node_from_transform_uses_stretch_anchors_with_pixel_offsets() {
        let node = node_from_transform(Some(&UiTransform2d {
            anchors: LyShineRect::new(0.0, 0.0, 1.0, 1.0),
            offsets: LyShineRect::new(12.0, 18.0, -8.0, -10.0),
            ..Default::default()
        }));

        // Anchors lay down the percentage portion of the inset; the pixel
        // offsets stack on top via margin so the totals match LyShine's
        // `parent_size * anchor + offset`.
        assert_eq!(node.left, Val::Percent(0.0));
        assert_eq!(node.right, Val::Percent(0.0));
        assert_eq!(node.width, Val::Auto);
        assert_eq!(node.top, Val::Percent(0.0));
        assert_eq!(node.bottom, Val::Percent(0.0));
        assert_eq!(node.height, Val::Auto);
        assert_eq!(node.margin.left, Val::Px(12.0));
        assert_eq!(node.margin.right, Val::Px(8.0));
        assert_eq!(node.margin.top, Val::Px(18.0));
        assert_eq!(node.margin.bottom, Val::Px(10.0));
    }

    #[test]
    fn node_from_transform_uses_fixed_offsets_without_stretch_anchors() {
        let node = node_from_transform(Some(&UiTransform2d {
            offsets: LyShineRect::new(10.0, 20.0, 210.0, 80.0),
            ..Default::default()
        }));

        assert_eq!(node.left, Val::Percent(0.0));
        assert_eq!(node.right, Val::Auto);
        assert_eq!(node.width, Val::Px(200.0));
        assert_eq!(node.top, Val::Percent(0.0));
        assert_eq!(node.bottom, Val::Auto);
        assert_eq!(node.height, Val::Px(60.0));
        assert_eq!(node.margin.left, Val::Px(10.0));
        assert_eq!(node.margin.top, Val::Px(20.0));
    }

    #[test]
    fn node_from_transform_centers_with_midpoint_anchors() {
        // Native NavMenuBg: 1920x188 pinned to top, horizontally centered on
        // parent's mid-line (anchors=(0.5, 0, 0.5, 0), offsets=(-960, 0, 960, 188)).
        let node = node_from_transform(Some(&UiTransform2d {
            anchors: LyShineRect::new(0.5, 0.0, 0.5, 0.0),
            offsets: LyShineRect::new(-960.0, 0.0, 960.0, 188.0),
            ..Default::default()
        }));

        // 50% inset + -960px margin = parent_w * 0.5 - 960 (CSS centering trick).
        assert_eq!(node.left, Val::Percent(50.0));
        assert_eq!(node.margin.left, Val::Px(-960.0));
        assert_eq!(node.width, Val::Px(1920.0));
        assert_eq!(node.top, Val::Percent(0.0));
        assert_eq!(node.margin.top, Val::Px(0.0));
        assert_eq!(node.height, Val::Px(188.0));
    }

    #[test]
    fn image_node_maps_lumberyard_image_modes_to_bevy_modes() {
        assert!(matches!(
            image_node(
                &UiImage {
                    image_type: UiImageType::Fixed,
                    ..Default::default()
                },
                1.0,
            )
            .image_mode,
            NodeImageMode::Auto
        ));
        assert!(matches!(
            image_node(
                &UiImage {
                    image_type: UiImageType::Tiled,
                    ..Default::default()
                },
                1.0,
            )
            .image_mode,
            NodeImageMode::Tiled {
                tile_x: true,
                tile_y: true,
                stretch_value: 1.0
            }
        ));
        assert!(matches!(
            image_node(
                &UiImage {
                    image_type: UiImageType::Sliced,
                    stretch_sliced: false,
                    ..Default::default()
                },
                1.0,
            )
            .image_mode,
            NodeImageMode::Sliced(TextureSlicer {
                center_scale_mode: SliceScaleMode::Tile { stretch_value: 1.0 },
                ..
            })
        ));
    }

    fn test_canvas_asset() -> UiCanvasAsset {
        UiCanvasAsset::new(
            UiCanvas {
                root_entity: UiEntityId::new(1),
                draw_order: 5,
                texture_atlases: vec!["ui/common.texatlasidx".to_string()],
                ..Default::default()
            },
            vec![
                UiEntity {
                    entity_id: UiEntityId::new(1),
                    runtime_active: true,
                    element: Some(UiElement {
                        enabled: true,
                        child_order: vec![UiChildOrder::new(UiEntityId::new(2), 0)],
                        ..Default::default()
                    }),
                    transform: Some(UiTransform2d {
                        anchors: LyShineRect::new(0.0, 0.0, 1.0, 1.0),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                UiEntity {
                    entity_id: UiEntityId::new(2),
                    name: Some("PlayButton".to_string()),
                    runtime_active: true,
                    element: Some(UiElement {
                        enabled: true,
                        ..Default::default()
                    }),
                    transform: Some(UiTransform2d {
                        offsets: LyShineRect::new(10.0, 20.0, 210.0, 80.0),
                        ..Default::default()
                    }),
                    image: Some(UiImage {
                        sprite_path: Some("lyshineui/images/icon.dds".to_string()),
                        color: LinearRgba::new(0.2, 0.4, 0.8, 1.0),
                        ..Default::default()
                    }),
                    text: Some(UiText {
                        text: "Play".to_string(),
                        font_size: 28.0,
                        ..Default::default()
                    }),
                    button: Some(UiButton::default()),
                    ..Default::default()
                },
            ],
        )
    }
}
