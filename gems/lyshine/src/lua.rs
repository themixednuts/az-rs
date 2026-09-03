//! `LyShine` Lua bytecode loading.
//!
//! Source intent:
//! - Legacy `LyShine` projects can ship scripts as Lua 5.1 bytecode assets.
//! - `LyShineUI/shared.lua` (cooked from `shared.luac`) is loaded before
//!   front-end canvases and queues `globals.PostLoad` after the module is
//!   available.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use az_lua_bytecode::{
    Instruction, LuaBytecode, LuaConstant, LuaHeader, LuaProto, Opcode, has_legacy_prefix,
};
use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, LoadState};
use bevy::prelude::*;
use thiserror::Error;

use crate::{LyShineLuaLoadQueue, LyShineLuaModuleLoadRequest, LyShineUiEntity};

/// File extensions handled by [`LyShineLuaAssetLoader`].
///
/// Cooked products are decompiled Lua source (`.lua`) for the `LuaJIT` runtime.
/// Legacy `.luac` is still accepted so uncooked/analysis paths keep working.
pub const LYSHINE_LUA_ASSET_EXTENSIONS: &[&str] = &["lua", "luac"];

/// Parsed Lua script module (bytecode metadata and/or decompiled source).
#[derive(Asset, TypePath, Debug, Clone, PartialEq, Eq)]
pub struct LyShineLuaAsset {
    pub original_bytes: Box<[u8]>,
    pub lua_chunk_bytes: Box<[u8]>,
    pub header: Option<LuaHeader>,
    pub has_legacy_prefix: bool,
    pub required_modules: Box<[Box<str>]>,
    pub instruction_count: usize,
    pub constant_count: usize,
    pub nested_proto_count: usize,
    pub max_stack_size: u8,
    pub upvalue_count: u8,
    pub param_count: u8,
    /// True when `original_bytes` are Lua source rather than a binary chunk.
    pub is_source: bool,
}

/// Bevy asset loader for cooked Lua script products (`.lua` / legacy `.luac`).
#[derive(Default, TypePath)]
pub struct LyShineLuaAssetLoader;

impl AssetLoader for LyShineLuaAssetLoader {
    type Asset = LyShineLuaAsset;
    type Settings = ();
    type Error = LyShineLuaAssetError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        parse_lua_asset(bytes)
    }

    fn extensions(&self) -> &[&str] {
        LYSHINE_LUA_ASSET_EXTENSIONS
    }
}

/// Native Lua bytecode asset format errors.
#[derive(Debug, Error)]
pub enum LyShineLuaAssetError {
    #[error("failed to read Lua bytecode asset: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid Lua bytecode asset: {0}")]
    Parse(#[from] az_lua_bytecode::ParseError),
}

fn parse_lua_asset(bytes: Vec<u8>) -> Result<LyShineLuaAsset, LyShineLuaAssetError> {
    if looks_like_lua_source(&bytes) {
        return Ok(parse_lua_source_asset(bytes));
    }

    let bytecode = LuaBytecode::parse(&bytes)?;
    let chunk = bytecode.parse_chunk()?;
    let main = &chunk.main;
    let lua_chunk_bytes = bytecode.chunk().to_vec().into_boxed_slice();
    let header = Some(bytecode.header());
    let has_legacy_prefix = bytecode.has_legacy_prefix();
    let required_modules = collect_required_modules(main);
    let instruction_count = count_instructions(main);
    let constant_count = count_constants(main);
    let nested_proto_count = count_nested_protos(main);
    let max_stack_size = main.max_stack_size;
    let upvalue_count = main.upvalue_count;
    let param_count = main.param_count;
    Ok(LyShineLuaAsset {
        original_bytes: bytes.into_boxed_slice(),
        lua_chunk_bytes,
        header,
        has_legacy_prefix,
        required_modules,
        instruction_count,
        constant_count,
        nested_proto_count,
        max_stack_size,
        upvalue_count,
        param_count,
        is_source: false,
    })
}

fn looks_like_lua_source(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    if bytes.starts_with(b"\x1bLua") || has_legacy_prefix(bytes) {
        return false;
    }
    // Cooked products are UTF-8 Lua source; treat non-signature payloads as source.
    std::str::from_utf8(bytes).is_ok()
}

fn parse_lua_source_asset(bytes: Vec<u8>) -> LyShineLuaAsset {
    let text = String::from_utf8_lossy(&bytes);
    let required_modules = collect_required_modules_from_source(&text);
    LyShineLuaAsset {
        original_bytes: bytes.clone().into_boxed_slice(),
        lua_chunk_bytes: bytes.into_boxed_slice(),
        header: None,
        has_legacy_prefix: false,
        required_modules,
        instruction_count: 0,
        constant_count: 0,
        nested_proto_count: 0,
        max_stack_size: 0,
        upvalue_count: 0,
        param_count: 0,
        is_source: true,
    }
}

fn collect_required_modules_from_source(source: &str) -> Box<[Box<str>]> {
    let mut modules = Vec::new();
    let mut seen = HashSet::new();
    for (idx, _) in source.match_indices("RequireScript") {
        let after = &source[idx + "RequireScript".len()..];
        let after = after.trim_start();
        let Some(rest) = after.strip_prefix('(') else {
            continue;
        };
        let rest = rest.trim_start();
        let quote = rest.as_bytes().first().copied();
        if quote != Some(b'"') && quote != Some(b'\'') {
            continue;
        }
        let quote = quote.unwrap() as char;
        let rest = &rest[1..];
        let Some(end) = rest.find(quote) else {
            continue;
        };
        let module = rest[..end].trim();
        if module.is_empty() {
            continue;
        }
        let module: Box<str> = module.into();
        if seen.insert(module.clone()) {
            modules.push(module);
        }
    }
    modules.into_boxed_slice()
}

fn count_instructions(proto: &LuaProto) -> usize {
    proto.instruction_count() + proto.protos.iter().map(count_instructions).sum::<usize>()
}

fn count_constants(proto: &LuaProto) -> usize {
    proto.constants.len() + proto.protos.iter().map(count_constants).sum::<usize>()
}

fn count_nested_protos(proto: &LuaProto) -> usize {
    proto.protos.len() + proto.protos.iter().map(count_nested_protos).sum::<usize>()
}

fn collect_required_modules(proto: &LuaProto) -> Box<[Box<str>]> {
    let mut modules = Vec::new();
    let mut seen = HashSet::new();
    collect_required_modules_from_proto(proto, &mut modules, &mut seen);
    modules.into_boxed_slice()
}

fn collect_required_modules_from_proto(
    proto: &LuaProto,
    modules: &mut Vec<Box<str>>,
    seen: &mut HashSet<Box<str>>,
) {
    for (index, instruction) in proto.code.iter().copied().enumerate() {
        if !is_require_script_global(proto, instruction) {
            continue;
        }
        let function_register = instruction.a();
        for candidate in proto.code.iter().copied().skip(index + 1).take(8) {
            if candidate.opcode() == Opcode::LoadK
                && candidate.a() == function_register.saturating_add(1)
                && let Some(module) = string_constant(proto, candidate.bx() as usize)
                && !module.trim().is_empty()
            {
                let module: Box<str> = module.trim().into();
                if seen.insert(module.clone()) {
                    modules.push(module);
                }
            }
            if candidate.opcode() == Opcode::Call && candidate.a() == function_register {
                break;
            }
        }
    }

    for child in &proto.protos {
        collect_required_modules_from_proto(child, modules, seen);
    }
}

fn is_require_script_global(proto: &LuaProto, instruction: Instruction) -> bool {
    instruction.opcode() == Opcode::GetGlobal
        && string_constant(proto, instruction.bx() as usize) == Some("RequireScript")
}

fn string_constant(proto: &LuaProto, index: usize) -> Option<&str> {
    proto.constants.get(index).and_then(LuaConstant::as_str)
}

/// One loaded `LyShine` Lua module handle.
#[derive(Debug, Clone)]
pub struct LyShineLoadedLuaModule {
    pub request: LyShineLuaModuleLoadRequest,
    pub handle: Handle<LyShineLuaAsset>,
}

/// `LyShine` Lua modules requested by native boot.
#[derive(Debug, Clone, Default, Resource)]
pub struct LyShineLoadedLuaModules {
    pub modules: Vec<LyShineLoadedLuaModule>,
}

/// Runtime metadata for one available Lua module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyShineLuaRuntimeModule {
    pub asset_path: &'static str,
    pub post_load_callback: &'static str,
    pub original_byte_len: usize,
    pub lua_chunk_byte_len: usize,
    pub has_legacy_prefix: bool,
    pub required_modules: Box<[Box<str>]>,
    pub instruction_count: usize,
    pub constant_count: usize,
    pub nested_proto_count: usize,
    pub max_stack_size: u8,
    pub upvalue_count: u8,
    pub param_count: u8,
}

/// Lua callback that is ready to be delivered to the native script bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyShineLuaCallback {
    pub module_asset_path: &'static str,
    pub callback_name: &'static str,
}

/// Native `AzFramework::ScriptComponent` binding mounted on a `LyShine` entity.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct LyShineUiScriptBinding {
    pub source_script: Box<str>,
    pub asset_path: Box<str>,
    pub context_id: u32,
    pub run_on_client: bool,
    pub run_on_server: bool,
    pub net_sync_enabled: bool,
}

/// Asset handle for a mounted `LyShine` entity script.
#[derive(Component, Debug, Clone)]
pub struct LyShineUiScriptHandle {
    pub handle: Handle<LyShineLuaAsset>,
}

/// Runtime metadata for one mounted entity script instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyShineLuaScriptInstance {
    pub bevy_entity: Entity,
    pub ui_entity_id: u64,
    pub source_script: Box<str>,
    pub asset_path: Box<str>,
    pub context_id: u32,
    pub run_on_client: bool,
    pub run_on_server: bool,
    pub net_sync_enabled: bool,
    pub required_modules: Box<[Box<str>]>,
    pub instruction_count: usize,
    pub constant_count: usize,
    pub nested_proto_count: usize,
}

/// Retained `LyShine` Lua runtime state.
#[derive(Debug, Default, Resource)]
pub struct LyShineLuaRuntime {
    pub modules: HashMap<&'static str, LyShineLuaRuntimeModule>,
    pub post_load_callbacks: VecDeque<LyShineLuaCallback>,
    pub script_instances: HashMap<Entity, LyShineLuaScriptInstance>,
    pub dependency_handles: HashMap<Box<str>, Handle<LyShineLuaAsset>>,
    pub loaded_dependencies: HashSet<Box<str>>,
    pending: HashSet<AssetId<LyShineLuaAsset>>,
    failed: HashSet<AssetId<LyShineLuaAsset>>,
    pending_scripts: HashSet<Entity>,
    failed_scripts: HashSet<Entity>,
    pending_dependencies: HashSet<AssetId<LyShineLuaAsset>>,
    failed_dependencies: HashSet<AssetId<LyShineLuaAsset>>,
}

/// Convert an authored `LyShine` script reference to a product asset path.
///
/// The input is the source path stored on a canvas `ScriptComponent`. The
/// asset builder decompiles `.luac` inputs to `.lua` products, so path
/// resolution rewrites the extension after normalising slashes and case.
#[must_use]
pub fn lyshine_script_asset_path(source_script: &str) -> Option<Box<str>> {
    let normalized = source_script.trim().replace('\\', "/").to_lowercase();
    if normalized.is_empty() {
        return None;
    }
    let with_ext = if has_lua_extension(&normalized) {
        normalized
    } else if let Some(stem) = normalized.strip_suffix(".luac") {
        format!("{stem}.lua")
    } else {
        format!("{normalized}.lua")
    };
    Some(with_ext.into_boxed_str())
}

/// Convert a native `RequireScript("Foo.Bar.Baz")` module name to
/// the product asset path the require hook should load.
///
/// This follows Lumberyard's `DefaultRequireHook` path shaping
/// (`o3de/Code/Framework/AzCore/AzCore/Script/ScriptSystemComponent.cpp:467`),
/// rewritten for Azoth's decompiled-source product path:
///
/// 1. Replace every `.` with `/`.
/// 2. Append `.lua` if not already present (`.luac` is rewritten to `.lua`).
/// 3. Look up the asset by that path (case-insensitive on Windows).
///
/// Example: `RequireScript("LyShineUI.Menu")` becomes `lyshineui/menu.lua`.
#[must_use]
pub fn lyshine_required_module_asset_path(module_name: &str) -> Option<Box<str>> {
    let module = module_name.trim();
    if module.is_empty() {
        return None;
    }
    let mut path = module.replace('.', "/").to_lowercase();
    if let Some(stem) = path.strip_suffix(".luac") {
        path = format!("{stem}.lua");
    } else if !has_lua_extension(&path) {
        path.push_str(".lua");
    }
    Some(path.into_boxed_str())
}

/// Whether `path` already names a `.lua` product.
///
/// Both callers lowercase before asking, but the extension is compared
/// case-insensitively anyway so an authored `.LUA` reference resolves the
/// same way the `AssetServer`'s own case-insensitive lookup would.
fn has_lua_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lua"))
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
pub fn load_queued_lua_modules(
    queue: Res<LyShineLuaLoadQueue>,
    asset_server: Option<Res<AssetServer>>,
    mut loaded: ResMut<LyShineLoadedLuaModules>,
) {
    let Some(asset_server) = asset_server else {
        return;
    };

    loaded.modules.clear();
    loaded.modules.reserve(queue.modules.len());
    for request in &queue.modules {
        let handle = asset_server.load::<LyShineLuaAsset>(request.asset_path);
        debug!(
            "Queued LyShine Lua module asset_path={} post_load_callback={} asset_id={:?}",
            request.asset_path,
            request.post_load_callback,
            handle.id()
        );
        loaded.modules.push(LyShineLoadedLuaModule {
            request: request.clone(),
            handle,
        });
    }
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
pub fn register_loaded_lua_modules(
    loaded: Res<LyShineLoadedLuaModules>,
    lua_assets: Res<Assets<LyShineLuaAsset>>,
    asset_server: Option<Res<AssetServer>>,
    mut runtime: ResMut<LyShineLuaRuntime>,
) {
    for loaded_module in &loaded.modules {
        let asset_id = loaded_module.handle.id();
        let Some(lua_asset) = lua_assets.get(&loaded_module.handle) else {
            if let Some(asset_server) = asset_server.as_deref() {
                let load_state = asset_server.load_state(asset_id);
                if let LoadState::Failed(error) = &load_state {
                    runtime.pending.remove(&asset_id);
                    if runtime.failed.insert(asset_id) {
                        warn!(
                            "LyShine Lua module failed to load asset_path={} post_load_callback={} asset_id={:?} error={}",
                            loaded_module.request.asset_path,
                            loaded_module.request.post_load_callback,
                            asset_id,
                            error
                        );
                    }
                } else if runtime.pending.insert(asset_id) {
                    trace!(
                        "LyShine Lua module pending asset_path={} post_load_callback={} asset_id={:?} load_state={:?}",
                        loaded_module.request.asset_path,
                        loaded_module.request.post_load_callback,
                        asset_id,
                        load_state
                    );
                }
            }
            continue;
        };

        runtime.pending.remove(&asset_id);
        runtime.failed.remove(&asset_id);
        if runtime
            .modules
            .contains_key(loaded_module.request.asset_path)
        {
            continue;
        }

        let module = LyShineLuaRuntimeModule {
            asset_path: loaded_module.request.asset_path,
            post_load_callback: loaded_module.request.post_load_callback,
            original_byte_len: lua_asset.original_bytes.len(),
            lua_chunk_byte_len: lua_asset.lua_chunk_bytes.len(),
            has_legacy_prefix: lua_asset.has_legacy_prefix,
            required_modules: lua_asset.required_modules.clone(),
            instruction_count: lua_asset.instruction_count,
            constant_count: lua_asset.constant_count,
            nested_proto_count: lua_asset.nested_proto_count,
            max_stack_size: lua_asset.max_stack_size,
            upvalue_count: lua_asset.upvalue_count,
            param_count: lua_asset.param_count,
        };
        runtime.post_load_callbacks.push_back(LyShineLuaCallback {
            module_asset_path: module.asset_path,
            callback_name: module.post_load_callback,
        });
        info!(
            "Loaded LyShine Lua module asset_path={} post_load_callback={} original_bytes={} lua_chunk_bytes={} legacy_prefix={} required_modules={:?} instructions={} constants={} nested_protos={} max_stack={} upvalues={} params={} post_load_queued=true",
            module.asset_path,
            module.post_load_callback,
            module.original_byte_len,
            module.lua_chunk_byte_len,
            module.has_legacy_prefix,
            module.required_modules,
            module.instruction_count,
            module.constant_count,
            module.nested_proto_count,
            module.max_stack_size,
            module.upvalue_count,
            module.param_count,
        );
        if let Some(asset_server) = asset_server.as_deref() {
            queue_required_modules(
                asset_server,
                &mut runtime,
                module.asset_path,
                &module.required_modules,
            );
        }
        runtime.modules.insert(module.asset_path, module);
    }
}

pub fn load_ui_script_bindings(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    scripts: Query<
        (Entity, &LyShineUiEntity, &LyShineUiScriptBinding),
        Added<LyShineUiScriptBinding>,
    >,
) {
    let Some(asset_server) = asset_server else {
        return;
    };

    for (entity, ui_entity, binding) in &scripts {
        let handle = asset_server.load::<LyShineLuaAsset>(binding.asset_path.as_ref().to_owned());
        debug!(
            "Queued LyShine entity script bevy_entity={:?} ui_entity_id={} source_script={} asset_path={} context_id={} run_client={} run_server={} net_sync={} asset_id={:?}",
            entity,
            ui_entity.entity_id.as_u64(),
            binding.source_script,
            binding.asset_path,
            binding.context_id,
            binding.run_on_client,
            binding.run_on_server,
            binding.net_sync_enabled,
            handle.id()
        );
        commands
            .entity(entity)
            .insert(LyShineUiScriptHandle { handle });
    }
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
pub fn register_loaded_ui_scripts(
    scripts: Query<(
        Entity,
        &LyShineUiEntity,
        &LyShineUiScriptBinding,
        &LyShineUiScriptHandle,
    )>,
    lua_assets: Res<Assets<LyShineLuaAsset>>,
    asset_server: Option<Res<AssetServer>>,
    mut runtime: ResMut<LyShineLuaRuntime>,
) {
    for (entity, ui_entity, binding, script_handle) in &scripts {
        if runtime.script_instances.contains_key(&entity) {
            continue;
        }

        let Some(lua_asset) = lua_assets.get(&script_handle.handle) else {
            if let Some(asset_server) = asset_server.as_deref() {
                let load_state = asset_server.load_state(script_handle.handle.id());
                if let LoadState::Failed(error) = &load_state {
                    runtime.pending_scripts.remove(&entity);
                    if runtime.failed_scripts.insert(entity) {
                        warn!(
                            "LyShine entity script failed to load bevy_entity={:?} ui_entity_id={} source_script={} asset_path={} asset_id={:?} error={}",
                            entity,
                            ui_entity.entity_id.as_u64(),
                            binding.source_script,
                            binding.asset_path,
                            script_handle.handle.id(),
                            error
                        );
                    }
                } else if runtime.pending_scripts.insert(entity) {
                    trace!(
                        "LyShine entity script pending bevy_entity={:?} ui_entity_id={} source_script={} asset_path={} asset_id={:?} load_state={:?}",
                        entity,
                        ui_entity.entity_id.as_u64(),
                        binding.source_script,
                        binding.asset_path,
                        script_handle.handle.id(),
                        load_state
                    );
                }
            }
            continue;
        };

        runtime.pending_scripts.remove(&entity);
        runtime.failed_scripts.remove(&entity);
        let instance = LyShineLuaScriptInstance {
            bevy_entity: entity,
            ui_entity_id: ui_entity.entity_id.as_u64(),
            source_script: binding.source_script.clone(),
            asset_path: binding.asset_path.clone(),
            context_id: binding.context_id,
            run_on_client: binding.run_on_client,
            run_on_server: binding.run_on_server,
            net_sync_enabled: binding.net_sync_enabled,
            required_modules: lua_asset.required_modules.clone(),
            instruction_count: lua_asset.instruction_count,
            constant_count: lua_asset.constant_count,
            nested_proto_count: lua_asset.nested_proto_count,
        };
        info!(
            "Loaded LyShine entity script bevy_entity={:?} ui_entity_id={} source_script={} asset_path={} context_id={} run_client={} run_server={} net_sync={} required_modules={:?} instructions={} constants={} nested_protos={}",
            instance.bevy_entity,
            instance.ui_entity_id,
            instance.source_script,
            instance.asset_path,
            instance.context_id,
            instance.run_on_client,
            instance.run_on_server,
            instance.net_sync_enabled,
            instance.required_modules,
            instance.instruction_count,
            instance.constant_count,
            instance.nested_proto_count,
        );
        if let Some(asset_server) = asset_server.as_deref() {
            let owner = instance.asset_path.to_string();
            queue_required_modules(
                asset_server,
                &mut runtime,
                &owner,
                &instance.required_modules,
            );
        }
        runtime.script_instances.insert(entity, instance);
    }
}

// Bevy systems take owned `SystemParam` wrappers; `&Res<_>` does not
// implement `SystemParam`, so the by-reference form would not register.
#[allow(clippy::needless_pass_by_value)]
pub fn register_loaded_lua_dependencies(
    lua_assets: Res<Assets<LyShineLuaAsset>>,
    asset_server: Option<Res<AssetServer>>,
    mut runtime: ResMut<LyShineLuaRuntime>,
) {
    let Some(asset_server) = asset_server else {
        return;
    };

    let handles = runtime
        .dependency_handles
        .iter()
        .map(|(path, handle)| (path.clone(), handle.clone()))
        .collect::<Vec<_>>();

    for (asset_path, handle) in handles {
        if runtime.loaded_dependencies.contains(&asset_path) {
            continue;
        }

        let asset_id = handle.id();
        let Some(lua_asset) = lua_assets.get(&handle) else {
            let load_state = asset_server.load_state(asset_id);
            if matches!(load_state, LoadState::Failed(_)) {
                runtime.pending_dependencies.remove(&asset_id);
                if runtime.failed_dependencies.insert(asset_id) {
                    warn!(
                        "LyShine required Lua module failed to load asset_path={} asset_id={:?}",
                        asset_path, asset_id
                    );
                }
            } else if runtime.pending_dependencies.insert(asset_id) {
                trace!(
                    "LyShine required Lua module pending asset_path={} asset_id={:?} load_state={:?}",
                    asset_path, asset_id, load_state
                );
            }
            continue;
        };

        runtime.pending_dependencies.remove(&asset_id);
        runtime.failed_dependencies.remove(&asset_id);
        runtime.loaded_dependencies.insert(asset_path.clone());
        info!(
            "Loaded LyShine required Lua module asset_path={} original_bytes={} lua_chunk_bytes={} legacy_prefix={} required_modules={:?} instructions={} constants={} nested_protos={}",
            asset_path,
            lua_asset.original_bytes.len(),
            lua_asset.lua_chunk_bytes.len(),
            lua_asset.has_legacy_prefix,
            lua_asset.required_modules,
            lua_asset.instruction_count,
            lua_asset.constant_count,
            lua_asset.nested_proto_count,
        );
        queue_required_modules(
            &asset_server,
            &mut runtime,
            &asset_path,
            &lua_asset.required_modules,
        );
    }
}

fn queue_required_modules(
    asset_server: &AssetServer,
    runtime: &mut LyShineLuaRuntime,
    owner_asset_path: &str,
    required_modules: &[Box<str>],
) {
    for module in required_modules {
        // The canonical Lumberyard `DefaultRequireHook` algorithm
        // only fails on empty input; any real module name produces
        // an asset path that the AssetServer then attempts to load
        // (and reports through normal failed-load warnings if the
        // file doesn't exist).
        let Some(asset_path) = lyshine_required_module_asset_path(module) else {
            warn!(
                "LyShine Lua RequireScript ignored empty module name owner_asset_path={} module={:?}",
                owner_asset_path, module
            );
            continue;
        };
        if runtime.dependency_handles.contains_key(&asset_path)
            || runtime.loaded_dependencies.contains(&asset_path)
        {
            continue;
        }
        let handle = asset_server.load::<LyShineLuaAsset>(asset_path.as_ref().to_owned());
        debug!(
            "Queued LyShine required Lua module owner_asset_path={} module={} asset_path={} asset_id={:?}",
            owner_asset_path,
            module,
            asset_path,
            handle.id()
        );
        runtime.dependency_handles.insert(asset_path, handle);
    }
}
