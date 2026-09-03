//! Runtime bridge for `AzFramework::ScriptComponent`.

use az_core::EntityId;
use bevy::prelude::*;
use bevy_mod_scripting::asset::{Language, ScriptAsset};
use bevy_mod_scripting::core::script::ScriptComponent as BmsScriptComponent;

use crate::ScriptComponent as AzScriptComponent;
use crate::lua::activation::LuaScriptModule;

pub fn attach_script_components(
    mut commands: Commands,
    mut script_assets: ResMut<Assets<ScriptAsset>>,
    scripts: Query<(Entity, &AzScriptComponent, Option<&EntityId>), Without<BmsScriptComponent>>,
) {
    for (entity, component, source_entity_id) in &scripts {
        if !component.run_on_client {
            continue;
        }
        let Some(script) = component.script.as_deref() else {
            continue;
        };
        let Some(module) = LuaScriptModule::from_asset_path(script) else {
            continue;
        };

        let entity_id = source_entity_id
            .copied()
            .filter(|entity_id| entity_id.is_valid())
            .unwrap_or_else(|| EntityId::new(entity.to_bits()));
        let activation = module.activate(entity_id);
        let handle = script_assets.add(ScriptAsset {
            content: activation.into_bytes(),
            language: Language::Lua,
        });

        commands
            .entity(entity)
            .insert(BmsScriptComponent(vec![handle]));
        debug!(
            target: "az_framework::lua::script_component",
            entity = ?entity,
            entity_id = entity_id.value(),
            asset_path = %module.asset_path(),
            module = %module.name(),
            context_id = component.context_id,
            run_on_server = component.run_on_server,
            net_sync_enabled = component.net_bindable.is_net_sync_enabled,
            "attached Lua ScriptComponent"
        );
    }
}
