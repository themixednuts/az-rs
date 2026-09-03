//! Bridge `LyShine` UI script bindings to BMS [`ScriptComponent`].
//!
//! `az-gem-lyshine` mounts a [`LyShineUiScriptBinding`] onto every UI entity
//! that has a `Script` component in the source canvas. The binding records
//! which cooked script product to run (`.lua`, decompiled from legacy
//! `.luac`) and the per-context flags (`run_on_client`, `run_on_server`,
//! `net_sync_enabled`). For a Bevy/BMS-driven runtime we only care about the
//! client side; server contexts run network handlers while client contexts run
//! UI behavior.
//!
//! ## Activation contract
//!
//! Lumberyard's `AzFramework::ScriptComponent` does more than load and
//! execute the script module: it expects the script to *return* a module
//! table, then assigns `module.entityId = <owning entity id>` and invokes
//! `module:OnActivate()` and `OnDeactivate` on teardown. Compatible UI modules
//! rely on `self.entityId` being set before activation.
//!
//! BMS doesn't model "module table + activation" — it just runs script
//! chunks. We synthesise the activation glue per-binding by emitting a
//! tiny Lua source script that:
//!
//! 1. `require()`s the cooked `.lua` module (our require hook resolves the
//!    path, loads the source product, caches the returned table in
//!    `_LOADED`).
//! 2. Sets `module.entityId = <UiEntityId.as_u64()>`.
//! 3. Calls `module:OnActivate()` if the function exists.
//!
//! This synthetic source becomes a `ScriptAsset` added directly to
//! `Assets<ScriptAsset>`, then attached as the `ScriptComponent` for
//! BMS to execute. The cooked script itself is loaded lazily on first
//! `require()` rather than eagerly through the `AssetServer`.

use crate::canvas::UiEntityId;
use crate::{LyShineUiEntity, LyShineUiScriptBinding};
use bevy::prelude::*;
use bevy_mod_scripting::asset::{Language, ScriptAsset};
use bevy_mod_scripting::core::script::ScriptComponent;

/// System that attaches a BMS [`ScriptComponent`] to every entity carrying a
/// fresh client-side [`LyShineUiScriptBinding`]. Runs in `Update` so it picks
/// up canvases as their entities are spawned.
fn attach_bms_scripts(
    mut commands: Commands,
    mut script_assets: ResMut<Assets<ScriptAsset>>,
    bindings: Query<(Entity, &LyShineUiEntity, &LyShineUiScriptBinding), Without<ScriptComponent>>,
) {
    for (entity, ui, binding) in &bindings {
        if !binding.run_on_client {
            // Server-only scripts: no client-side execution.
            continue;
        }

        let module_name = asset_path_to_module_name(&binding.asset_path);
        let bootstrap = build_activation_script(&module_name, ui.entity_id, &binding.asset_path);

        let asset = ScriptAsset {
            content: bootstrap.into_bytes().into_boxed_slice(),
            language: Language::Lua,
        };
        let handle = script_assets.add(asset);
        commands
            .entity(entity)
            .insert(ScriptComponent(vec![handle]));
        debug!(
            target: "az_gem_lyshine::script::bms_bridge",
            entity = ?entity,
            ui_id = ui.entity_id.as_u64(),
            asset_path = %binding.asset_path,
            module = %module_name,
            source_script = %binding.source_script,
            context_id = binding.context_id,
            "attached BMS ScriptComponent (bootstrap activation)"
        );
    }
}

/// Convert a `LyShine` asset path (for example, `"ui/scripts/bootstrap.lua"`)
/// into a require-style module name (`"ui.scripts.bootstrap"`).
///
/// Inverse of [`crate::lua::lyshine_required_module_asset_path`] —
/// any path produced by that helper round-trips through this fn
/// back to a module name that, when passed to our require hook,
/// yields the original asset path. That round-trip is what makes
/// Lua's `_LOADED` cache work correctly for cross-canvas sharing.
fn asset_path_to_module_name(asset_path: &str) -> String {
    let trimmed = asset_path
        .strip_suffix(".lua")
        .or_else(|| asset_path.strip_suffix(".luac"))
        .unwrap_or(asset_path);
    trimmed.replace('/', ".").to_ascii_lowercase()
}

/// Generate the per-binding bootstrap Lua source.
///
/// The script is intentionally defensive: if the loaded module isn't a
/// table (for example, a script that does not follow the activation contract),
/// or if it lacks `OnActivate`, we log and skip rather than raise — the
/// engine's `Lua` BMS plugin would otherwise abort the whole canvas.
fn build_activation_script(module: &str, ui_id: UiEntityId, asset_path: &str) -> String {
    let id = ui_id.as_u64();
    format!(
        r#"-- bms_bridge bootstrap: {asset_path}
-- LyShine UI script activation glue: load module, set entityId, invoke
-- OnActivate. See `gems/lyshine/src/script/bms_bridge.rs`.
local _ok, _m = pcall(require, "{module}")
if not _ok then
    if Debug and Debug.Log then
        Debug.Log("bms_bridge[{asset_path}]: require failed: " .. tostring(_m))
    end
    return
end
if type(_m) ~= "table" then
    if Debug and Debug.Log then
        Debug.Log("bms_bridge[{asset_path}]: module is not a table; activation skipped")
    end
    return
end
_m.entityId = {id}
if type(_m.OnActivate) == "function" then
    local _activate_ok, _activate_err = pcall(_m.OnActivate, _m)
    if not _activate_ok and Debug and Debug.Log then
        Debug.Log("bms_bridge[{asset_path}]: OnActivate raised: " .. tostring(_activate_err))
    end
end
"#
    )
}

/// Plugin wrapping [`attach_bms_scripts`].
pub struct LyShineBmsBridgePlugin;

impl Plugin for LyShineBmsBridgePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, attach_bms_scripts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_strips_extension_and_normalises() {
        // Round-trips with `lyshine_required_module_asset_path`:
        // `ui.scripts.bootstrap` → `ui/scripts/bootstrap.lua` → back.
        assert_eq!(
            asset_path_to_module_name("ui/scripts/bootstrap.lua"),
            "ui.scripts.bootstrap",
        );
        // Legacy `.luac` product paths still strip cleanly.
        assert_eq!(
            asset_path_to_module_name("ui/scripts/bootstrap.luac"),
            "ui.scripts.bootstrap",
        );
        // Mixed case / nested path normalises to lowercase.
        assert_eq!(
            asset_path_to_module_name("LyShineUI/Main Menu/landing.lua"),
            "lyshineui.main menu.landing",
        );
        // Extensionless paths pass through unchanged (apart from / -> .).
        assert_eq!(asset_path_to_module_name("foo/bar"), "foo.bar");
    }

    #[test]
    fn bootstrap_contains_module_and_id() {
        let src = build_activation_script(
            "ui.scripts.bootstrap",
            UiEntityId::new(0xdead_beef),
            "ui/scripts/bootstrap.lua",
        );
        assert!(
            src.contains(r#"pcall(require, "ui.scripts.bootstrap")"#),
            "expected pcall(require, ...) wrapper; got:\n{src}",
        );
        assert!(
            src.contains("_m.entityId = 3735928559"),
            "expected entityId = decimal UiEntityId; got:\n{src}",
        );
        assert!(
            src.contains("pcall(_m.OnActivate, _m)"),
            "expected pcall(_m.OnActivate, _m); got:\n{src}",
        );
    }
}
