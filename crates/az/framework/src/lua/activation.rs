//! Lua module activation for `AzFramework::ScriptComponent`.

use az_core::EntityId;

/// Lua module referenced by an `AzFramework::ScriptComponent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaScriptModule {
    asset_path: String,
    module: String,
}

impl LuaScriptModule {
    /// Build a Lua module reference from an asset path such as
    /// `scripts/foo/bar.lua` (cooked product) or legacy `*.luac`.
    #[must_use]
    pub fn from_asset_path(asset_path: impl AsRef<str>) -> Option<Self> {
        let asset_path = asset_path.as_ref().trim().replace('\\', "/");
        if asset_path.is_empty() {
            return None;
        }

        let module = asset_path
            .strip_suffix(".lua")
            .or_else(|| asset_path.strip_suffix(".luac"))
            .unwrap_or(asset_path.as_str())
            .replace('/', ".")
            .to_ascii_lowercase();
        Some(Self { asset_path, module })
    }

    /// Asset path stored by the source component.
    #[must_use]
    pub fn asset_path(&self) -> &str {
        &self.asset_path
    }

    /// Module name passed to the AZ `require` hook.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.module
    }

    /// Create the script that loads this module and calls `OnActivate`.
    #[must_use]
    pub fn activate(&self, entity_id: EntityId) -> LuaActivationScript {
        LuaActivationScript::new(self, entity_id)
    }
}

/// Synthetic Lua source used to activate one component script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaActivationScript {
    source: String,
}

impl LuaActivationScript {
    #[must_use]
    fn new(module: &LuaScriptModule, entity_id: EntityId) -> Self {
        let module_name = lua_string(module.name());
        let asset_path = lua_string(module.asset_path());
        let entity_id = entity_id.value();
        let source = format!(
            r#"local _asset = {asset_path}
local _ok, _m = pcall(require, {module_name})
if not _ok then
    if Debug and Debug.Log then
        Debug.Log("ScriptComponent[" .. _asset .. "]: require failed: " .. tostring(_m))
    end
    return
end
if type(_m) ~= "table" then
    if Debug and Debug.Log then
        Debug.Log("ScriptComponent[" .. _asset .. "]: module is not a table")
    end
    return
end
_m.entityId = EntityId({entity_id})
if type(_m.OnActivate) == "function" then
    local _activate_ok, _activate_err = pcall(_m.OnActivate, _m)
    if not _activate_ok and Debug and Debug.Log then
        Debug.Log("ScriptComponent[" .. _asset .. "]: OnActivate raised: " .. tostring(_activate_err))
    end
end
"#
        );
        Self { source }
    }

    /// Lua source code.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Consume this script into bytes suitable for a BMS `ScriptAsset`.
    #[must_use]
    pub fn into_bytes(self) -> Box<[u8]> {
        self.source.into_bytes().into_boxed_slice()
    }
}

fn lua_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_path_maps_to_module_name() {
        let module =
            LuaScriptModule::from_asset_path("LyShineUI/Main Menu/landing.lua").expect("module");

        assert_eq!(module.name(), "lyshineui.main menu.landing");
        assert_eq!(module.asset_path(), "LyShineUI/Main Menu/landing.lua");

        // Legacy bytecode product paths still strip cleanly.
        let legacy =
            LuaScriptModule::from_asset_path("LyShineUI/Main Menu/landing.luac").expect("module");
        assert_eq!(legacy.name(), "lyshineui.main menu.landing");
    }

    #[test]
    fn activation_script_sets_entity_id_and_calls_on_activate() {
        let module = LuaScriptModule::from_asset_path("ui/scripts/bootstrap.lua").expect("module");
        let script = module.activate(EntityId::new(0xdead_beef));

        assert!(
            script
                .source()
                .contains(r#"pcall(require, "ui.scripts.bootstrap")"#)
        );
        assert!(
            script
                .source()
                .contains("_m.entityId = EntityId(3735928559)")
        );
        assert!(script.source().contains("pcall(_m.OnActivate, _m)"));
    }

    #[test]
    fn activation_script_quotes_lua_strings() {
        let module = LuaScriptModule::from_asset_path("scripts/a\"b.lua").expect("module");
        let script = module.activate(EntityId::new(1));

        assert!(script.source().contains(r#""scripts.a\"b""#));
        assert!(script.source().contains(r#""scripts/a\"b.lua""#));
    }
}
