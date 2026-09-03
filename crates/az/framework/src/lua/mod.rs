//! Lua bridge: BMS plugin configuration, custom `require`, AZ globals,
//! and the EBus-to-Lua bus factory.
//!
//! Lumberyard reference: `AzCore/Script/ScriptContext.cpp` (especially
//! `Bind*` methods at line 4902 / 5096) + `AzCore/Script/lua/`. The
//! `ScriptSystemComponent::DefaultRequireHook` lives at the O3DE reference
//! `Code/Framework/AzCore/AzCore/Script/ScriptSystemComponent.cpp:404-491`.
//!
//! ## Module layout
//!
//! - [`bus`] — `install_bus(lua, name)` Lua-side `EBus` factory wired to
//!   [`crate::ebus::EBusRegistry`].
//! - [`globals`] — `Debug.Log`, `Vector2/3`, `Color`, `Entity`,
//!   `WallClockTimePoint` — common constructors and namespaces exposed
//!   through Lumberyard's `BehaviorContext::Class<T>("Name")`.
//! - [`require`] — custom `require()` resolver that mirrors
//!   `DefaultRequireHook`, looking up every content path in the
//!   [`crate::asset::AssetCatalog`] resource. No on-disk fallback —
//!   missing-from-catalog is a `require()` failure, exactly like
//!   Lumberyard.
//!
//! ## Boot order
//!
//! [`init_az_lua_context`] is registered as a BMS context initializer
//! (via [`AzLuaScriptingPlugin`]) and runs once per Lua context create or
//! reload. It installs:
//!
//! 1. `globals::register` — constructors + `Debug` first because
//!    `Scripts/_Common/Common.lua` references `Debug.Log` early.
//! 2. `dynamic_bus::install` and lazy `*Bus` auto-registration — concrete
//!    gem/project bus handlers register separately at Bevy `Startup`.
//! 3. `require::install` — custom resolver. Last so it can rely on
//!    everything else being present.

pub mod activation;
pub mod bus;
pub mod dynamic_bus;
pub mod globals;
pub mod luac_loader;
pub mod require;
mod script_component;
pub mod types;

pub use activation::{LuaActivationScript, LuaScriptModule};
pub use luac_loader::{LuacAssetLoader, LuacAssetLoaderPlugin, LuacLoadError};

use bevy::prelude::*;
use bevy_mod_scripting::bindings::InteropError;
use bevy_mod_scripting::lua::{LuaScriptingPlugin, mlua};
use bevy_mod_scripting::script::ScriptAttachment;

/// BMS context initializer that wires AZ/Lumberyard Lua globals into a freshly-loaded
/// Lua context. Runs once per context creation and once per reload.
///
/// # Errors
///
/// Returns [`InteropError::external`] wrapping the `mlua::Error` raised by any
/// of the installation steps: [`globals::register`], [`types::register`],
/// [`dynamic_bus::install`], the `_G` bus auto-registration metatable, or
/// [`require::install`].
pub fn init_az_lua_context(
    _attachment: &ScriptAttachment,
    context: &mut bevy_mod_scripting::lua::LuaContext,
) -> Result<(), InteropError> {
    let lua: &mlua::Lua = context;

    globals::register(lua).map_err(InteropError::external)?;
    types::register(lua).map_err(InteropError::external)?;
    dynamic_bus::install(lua).map_err(InteropError::external)?;
    install_bus_auto_register(lua).map_err(InteropError::external)?;
    require::install(lua).map_err(InteropError::external)?;

    Ok(())
}

/// Install a `_G` metatable that lazily creates a bus shape for any
/// unknown global whose name matches the AZ `EBus` naming convention.
///
/// Project and gem scripts can reference buses before their native
/// handlers have registered. Auto-registration gives those scripts the
/// standard Lua `EBus` table shape; actual dispatch still depends on
/// handlers registered in [`crate::ebus::EBusRegistry`].
///
/// Lua doesn't put a metatable on `_G` by default; setting one
/// intercepts ONLY missing-key lookups, so existing globals (Debug,
/// Vector2, registered buses, etc.) keep working unmodified. The
/// metatable's `__index` checks the requested key against a bus naming
/// pattern; if it matches, [`bus::install_bus`] creates the shape (which
/// also stamps it into `_G`, so subsequent accesses bypass this hook).
fn install_bus_auto_register(lua: &mlua::Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    let mt = lua.create_table()?;
    mt.set(
        "__index",
        lua.create_function(
            |lua, (_t, key): (mlua::Table, mlua::Value)| -> mlua::Result<mlua::Value> {
                let name = match key {
                    mlua::Value::String(s) => s.to_str()?.to_string(),
                    _ => return Ok(mlua::Value::Nil),
                };
                // Heuristic: anything ending in `Bus` is an EBus by AZ
                // convention (`UiCanvasBus`, `ConfigSystemEventBus`,
                // `LyShineScriptBindRequestBus`, etc.). `EventBus`,
                // `RequestBus`, `NotificationBus` all match the suffix
                // check. Tighter patterns can land later if false-positives
                // bite (no known case yet).
                if !name.ends_with("Bus") {
                    return Ok(mlua::Value::Nil);
                }
                tracing::debug!(
                    target: "az_framework::lua::bus_autoregister",
                    bus = %name,
                    "auto-registering AZ EBus on first access",
                );
                let bus = bus::install_bus(lua, &name)?;
                Ok(mlua::Value::Table(bus))
            },
        )?,
    )?;
    globals.set_metatable(Some(mt));
    Ok(())
}

/// Bevy plugin: installs the [`crate::ebus::EBusRegistry`] resource and
/// swaps BMS's default Lua plugin for one configured with AZ's
/// shared-context policy + the [`init_az_lua_context`] initializer.
///
/// Use via the BMS `plugin_group!`'s `.set(...)`:
///
/// ```ignore
/// app.add_plugins(BMSPlugin.build().set(AzLuaScriptingPlugin::lua_plugin()));
/// app.add_plugins(AzLuaScriptingPlugin::default());
/// ```
#[derive(Default)]
pub struct AzLuaScriptingPlugin;

impl Plugin for AzLuaScriptingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::ebus::EBusRegistry>();
        // Register the Lua script asset loader so BMS picks up engine-shipped
        // Lua source products (and legacy `.luac` analysis paths) through the
        // standard `AssetServer::load::<ScriptAsset>` path.
        app.add_plugins(LuacAssetLoaderPlugin);
        app.add_systems(Update, script_component::attach_script_components);
    }
}

impl AzLuaScriptingPlugin {
    /// Build the [`LuaScriptingPlugin`] that should be set into BMS's
    /// plugin group. Returned plugin uses `ContextPolicy::shared()` (so
    /// canvas scripts share `g_cachedScripts` etc.) and our context
    /// initializer.
    pub fn lua_plugin() -> LuaScriptingPlugin {
        use bevy_mod_scripting::core::ConfigureScriptPlugin;
        use bevy_mod_scripting::core::script::ContextPolicy;

        LuaScriptingPlugin::default()
            .set_context_policy(ContextPolicy::shared())
            .add_context_initializer(init_az_lua_context)
    }
}
