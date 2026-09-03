//! Custom Bevy `AssetLoader` for cooked Lua script products.
//!
//! The `az.lua-script` builder decompiles legacy `.luac` bytecode into Lua
//! source so the `LuaJIT` runtime can load it. BMS's per-plugin
//! dispatch keys off the language tag, not the file extension (see
//! `bevy_mod_scripting_core::pipeline::start::filter_script_modifications`),
//! so this loader wraps product bytes in a BMS [`ScriptAsset`] tagged
//! [`Language::Lua`].
//!
//! Both `.lua` (preferred cooked product) and legacy `.luac` extensions are
//! accepted. Content is treated as loadable script payload for `mlua::Lua::load`.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy_mod_scripting::asset::{Language, ScriptAsset};
use thiserror::Error;

/// Asset loader for cooked Lua script products producing a BMS [`ScriptAsset`].
#[derive(Default, TypePath)]
pub struct LuacAssetLoader;

#[derive(Debug, Error)]
pub enum LuacLoadError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl AssetLoader for LuacAssetLoader {
    type Asset = ScriptAsset;
    type Settings = ();
    type Error = LuacLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(ScriptAsset {
            content: bytes.into_boxed_slice(),
            language: Language::Lua,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["lua", "luac"]
    }
}

/// Plugin that registers the Lua script asset loader. Add after `BMSPlugin`.
pub struct LuacAssetLoaderPlugin;

impl Plugin for LuacAssetLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset_loader(LuacAssetLoader);
    }
}
