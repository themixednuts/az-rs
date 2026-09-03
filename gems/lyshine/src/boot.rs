//! `LyShine` asset-loading requests and Bevy plugin wiring.
//!
//! O3DE reference: `Gems/LyShine/Code/Source/LyShineSystemComponent.cpp`.

use bevy::prelude::*;

use az_gem_texture_atlas::TextureAtlasAssetPlugin;

use crate::{
    LyShineDispatchedUiActions, LyShineFontDescriptorLoader, LyShineLoadedCanvasAssets,
    LyShineLoadedLuaModules, LyShineLuaAsset, LyShineLuaAssetLoader, LyShineLuaRuntime,
    LyShineQueuedUiActions, LyShineSpawnedCanvases, LyShineSpriteAsset, LyShineSpriteAssetLoader,
    LyShineTextureAtlasLayouts, UiCanvasAsset, UiCanvasAssetLoader, advance_bink_startup_video,
    bind_loaded_canvas_images, dispatch_button_actions, dispatch_queued_ui_actions,
    load_queued_canvas_assets, load_queued_lua_modules, load_ui_script_bindings,
    register_loaded_lua_dependencies, register_loaded_lua_modules, register_loaded_ui_scripts,
    spawn_loaded_canvas_assets, start_bink_startup_video,
};

/// Project-defined role for a loaded UI canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LyShineCanvasPurpose {
    Global,
    Shared,
    Frontend,
    Overlay,
}

/// One resolved Lua module load request.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct LyShineLuaModuleLoadRequest {
    pub asset_path: &'static str,
    pub post_load_callback: &'static str,
}

/// One resolved canvas load request.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct LyShineCanvasLoadRequest {
    pub asset_path: &'static str,
    pub purpose: LyShineCanvasPurpose,
    pub active_on_load: bool,
}

/// Project-supplied canvas requests in load order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub struct LyShineCanvasLoadQueue {
    pub canvases: Vec<LyShineCanvasLoadRequest>,
}

/// Resolved Lua modules in the order needed by `LyShine` startup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub struct LyShineLuaLoadQueue {
    pub modules: Vec<LyShineLuaModuleLoadRequest>,
}

/// Register `LyShine` UI resources, loaders, and runtime systems.
#[derive(Debug, Default)]
pub struct LyShinePlugin;

impl Plugin for LyShinePlugin {
    fn build(&self, app: &mut App) {
        logging_registry::register!();

        app.add_plugins(TextureAtlasAssetPlugin);

        // `init_asset` is **NOT** idempotent — it ends in
        // `self.insert_resource(Assets::<A>::default())` which
        // clobbers any existing `Assets<A>` storage. Calling it
        // unconditionally here would replace the populated
        // `Assets<Image>` that `DefaultPlugins`' `ImagePlugin`
        // already set up, orphaning every handle allocated before
        // this plugin runs. Only init if missing, so we self-host
        // under `MinimalPlugins + AssetPlugin` (unit tests) without
        // breaking the full app's `ImagePlugin` registration.
        if !app.world().contains_resource::<Assets<Image>>() {
            app.init_asset::<Image>();
        }
        if !app
            .world()
            .contains_resource::<Assets<TextureAtlasLayout>>()
        {
            app.init_asset::<TextureAtlasLayout>();
        }

        app.init_asset::<UiCanvasAsset>()
            .init_asset_loader::<UiCanvasAssetLoader>()
            .init_asset::<LyShineLuaAsset>()
            .init_asset_loader::<LyShineLuaAssetLoader>()
            // Lumberyard `.font` files are XML descriptors for sibling font
            // faces. This loader resolves the descriptor before Bevy reads
            // the binary face.
            .init_asset_loader::<LyShineFontDescriptorLoader>()
            // Lumberyard `.sprite` sidecars carry nine-slice borders used by
            // `NodeImageMode::Sliced`.
            .init_asset::<LyShineSpriteAsset>()
            .init_asset_loader::<LyShineSpriteAssetLoader>()
            .init_resource::<LyShineCanvasLoadQueue>()
            .init_resource::<LyShineLuaLoadQueue>()
            .init_resource::<LyShineLoadedCanvasAssets>()
            .init_resource::<LyShineLoadedLuaModules>()
            .init_resource::<LyShineLuaRuntime>()
            .init_resource::<LyShineSpawnedCanvases>()
            .init_resource::<crate::LyShineCanvasEnabledState>()
            .init_resource::<crate::LyShineBinkStartupState>()
            .init_resource::<LyShineQueuedUiActions>()
            .init_resource::<LyShineDispatchedUiActions>()
            .init_resource::<LyShineTextureAtlasLayouts>()
            .add_message::<LyShineCanvasLoadRequest>()
            .add_message::<LyShineLuaModuleLoadRequest>()
            .add_systems(
                Startup,
                (load_queued_lua_modules, load_queued_canvas_assets).chain(),
            )
            .add_systems(
                Update,
                (
                    start_bink_startup_video,
                    advance_bink_startup_video,
                    register_loaded_lua_modules,
                    register_loaded_lua_dependencies,
                    spawn_loaded_canvas_assets,
                    crate::sync_canvas_enabled_state,
                    bind_loaded_canvas_images,
                    crate::apply_sprite_borders,
                    load_ui_script_bindings,
                    register_loaded_ui_scripts,
                    dispatch_button_actions,
                    dispatch_queued_ui_actions,
                )
                    .chain(),
            );
    }
}
