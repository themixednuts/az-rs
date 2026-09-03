//! Custom `require()` resolver mirroring O3DE's
//! `ScriptSystemComponent::DefaultRequireHook`
//! (O3DE reference: `Code/Framework/AzCore/AzCore/Script/ScriptSystemComponent.cpp:404-491`).
//!
//! AZ/Lumberyard Lua modules use dotted paths (`"LyShineUI._Common.Common"`)
//! where stock Lua `require` uses filesystem paths. The Lumberyard hook
//! lowercases the module name, replaces `.` with `/`, and appends a script
//! extension. Azoth cooks decompiled Lua source products (`.lua`) so the
//! `LuaJIT` runtime can load them; this resolver therefore looks up `.lua`
//! (not raw `.luac` bytecode).
//!
//! We do the same catalog-first lookup Lumberyard does. Every lookup goes
//! through the [`AssetCatalog`] resource (parsed RASC + RAOC, unified).
//! There is no filesystem fallback — a content path the catalog doesn't
//! know is a `require()` failure, same as Lumberyard.
//!
//! Cooked products are UTF-8 Lua source. Legacy `.luac` bytecode is only
//! accepted by analysis loaders; the runtime product path is `.lua`.
//!
//! The result cache lives in Lua's `_LOADED` registry table — the same
//! key stock Lua 5.1 uses, so subsequent `require("X")` calls
//! short-circuit through the registry table for free.

use std::path::PathBuf;

use bevy_mod_scripting::bindings::ThreadWorldContainer;
use bevy_mod_scripting::lua::mlua::{self, Lua, Result, Table, Value};

use crate::asset::AssetCatalog;

/// Replace the global `require` with the AZ asset-catalog resolver, pulling the asset
/// root + catalogs from the Bevy world via [`ThreadWorldContainer`].
/// Idempotent.
///
/// # Errors
///
/// Returns the [`mlua::Error`] raised while creating the resolver closure or
/// while assigning it to the `require` global.
pub fn install(lua: &Lua) -> Result<()> {
    let require_fn =
        lua.create_function(|lua, module: String| az_require_via_world(lua, &module))?;
    lua.globals().set("require", require_fn)?;
    Ok(())
}

/// Test-only variant: install with an explicit resolver (no Bevy world).
/// Useful for smoke-testing the Lua side without spinning up Bevy + BMS.
#[doc(hidden)]
pub fn install_with_resolver<F>(lua: &Lua, resolver: F) -> Result<()>
where
    F: Fn(&str) -> Option<PathBuf> + Send + Sync + 'static,
{
    let resolver = std::sync::Arc::new(resolver);
    let require_fn = lua.create_function(move |lua, module: String| -> Result<Value> {
        az_require(lua, &module, |path| (*resolver)(path))
    })?;
    lua.globals().set("require", require_fn)?;
    Ok(())
}

fn az_require_via_world(lua: &Lua, module: &str) -> Result<Value> {
    // Snapshot the resolver out of the world once per call, then drop
    // the borrow before executing the chunk so the script body can
    // re-enter `require()` without lock conflicts.
    let world = ThreadWorldContainer
        .try_get_context()
        .map_err(mlua::Error::external)?
        .world;

    // The AssetCatalog is the single source of truth — mirrors
    // Lumberyard's `ScriptSystemComponent::DefaultRequireHook`, which
    // routes every lookup through `AssetCatalogRequestBus::
    // GetAssetIdByPath` and never inspects the filesystem directly.
    az_require(lua, module, |asset_path| {
        world
            .with_resource(|catalog: &AssetCatalog| catalog.disk_path(asset_path))
            .ok()
            .flatten()
    })
}

/// Pure resolver — given a module name and a `resolver` that maps the
/// dot-to-slash + `.lua` path to a physical disk path, find the
/// script source, execute it, cache by `_LOADED`, return the result.
fn az_require<R>(lua: &Lua, module: &str, mut resolver: R) -> Result<Value>
where
    R: FnMut(&str) -> Option<PathBuf>,
{
    let asset_path = module.to_lowercase().replace('.', "/") + ".lua";

    let loaded = loaded_table(lua)?;
    if let Ok(cached) = loaded.get::<Value>(module.to_string())
        && !matches!(cached, Value::Nil)
    {
        return Ok(cached);
    }

    let full_path = resolver(&asset_path).ok_or_else(|| {
        mlua::Error::external(format!(
            "require({module:?}): no AssetCatalog entry for {asset_path:?}"
        ))
    })?;
    let bytes = std::fs::read(&full_path).map_err(|e| {
        mlua::Error::external(format!(
            "require({module:?}): failed to read {}: {e}",
            full_path.display()
        ))
    })?;

    tracing::debug!(
        target: "az_framework::lua::require",
        module = %module,
        path = %full_path.display(),
        bytes = bytes.len(),
        "begin loading AZ Lua module",
    );

    // Cache an empty proxy table BEFORE running the chunk. Legacy Lua
    // modules sometimes circular-require each other; pre-caching gives
    // them a non-nil table to grab references off, even before the
    // module fully initialises. The chunk's actual return value
    // overwrites the cache entry below.
    //
    // NOTE: this is the same trick stock Lua's package loader uses when
    // detecting cycles — it pre-stamps `_LOADED[name]` with `true` to
    // break the cycle. We use a proxy table instead so children that
    // index into us get something usable.
    let placeholder: Value = Value::Table(empty_proxy(lua)?);
    loaded.set(module.to_string(), placeholder.clone())?;

    // Execute the chunk. Authored modules usually end with `return <value>`, so
    // `eval()` returns whatever the script exports. Modules with no
    // return value yield `nil` — promote to `true` so the cached entry is
    // non-nil and short-circuits next time, matching stock Lua semantics.
    //
    // **Tolerant mode**: if the chunk raises, log the failure with the
    // module path and keep the placeholder in the cache so dependent
    // modules see SOMETHING usable (an empty table) rather than
    // propagating the error up the require chain. This trades fidelity
    // of dependent modules for boot-completeness, which is what we need
    // to surface the runtime EBus surface during project Lua bring-up.
    let chunk = lua.load(&bytes).set_name(asset_path.clone());
    match chunk.eval::<Value>() {
        Ok(value) => {
            let cached = if matches!(value, Value::Nil) {
                placeholder
            } else {
                value
            };
            loaded.set(module.to_string(), cached.clone())?;
            tracing::debug!(
                target: "az_framework::lua::require",
                module = %module,
                path = %full_path.display(),
                "finished loading AZ Lua module",
            );
            Ok(cached)
        }
        Err(e) => {
            tracing::warn!(
                target: "az_framework::lua::require",
                module = %module,
                path = %full_path.display(),
                error = %e,
                "AZ Lua module load raised; caching empty proxy and continuing",
            );
            // Placeholder stays in `_LOADED`; return it to the caller
            // so the require chain proceeds.
            Ok(placeholder)
        }
    }
}

/// Build a proxy table whose every access returns a callable proxy
/// (never nil). Used as the placeholder cached at module-load entry to
/// break circular requires AND as the fallback for modules that fail to
/// load — both cases need `proxy:Method()` and `proxy.field:Method()`
/// patterns to evaluate to no-ops without panicking.
///
/// Same semantics as `globals::no_op_proxy` but kept local to keep
/// `require.rs` self-contained.
fn empty_proxy(lua: &Lua) -> Result<Table> {
    let proxy = lua.create_table()?;
    proxy.set_metatable(Some(empty_proxy_metatable(lua)?));
    Ok(proxy)
}

fn empty_proxy_metatable(lua: &Lua) -> Result<Table> {
    let mt = lua.create_table()?;
    mt.set(
        "__index",
        lua.create_function(|lua, (_t, _key): (Table, Value)| -> Result<Table> {
            empty_proxy(lua)
        })?,
    )?;
    mt.set(
        "__call",
        lua.create_function(|lua, _: mlua::Variadic<Value>| -> Result<Table> { empty_proxy(lua) })?,
    )?;
    Ok(mt)
}

/// Get (or lazily create) the `_LOADED` registry table.
///
/// Stock Lua 5.1 populates this on `luaopen_package`; mlua's default Lua
/// state has it too, but if a future BMS configuration disables the
/// package library we still want `require` to function. The fallback
/// path mirrors Lumberyard (`ScriptSystemComponent.cpp:507`).
fn loaded_table(lua: &Lua) -> Result<Table> {
    if let Ok(Value::Table(t)) = lua.named_registry_value::<Value>("_LOADED") {
        return Ok(t);
    }
    let t = lua.create_table()?;
    lua.set_named_registry_value("_LOADED", t.clone())?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `require()` semantics: missing modules return a clear error, not a
    /// panic.
    #[test]
    fn missing_module_reports_clear_error() {
        let lua = Lua::new();
        install_with_resolver(&lua, |_: &str| None).expect("install");

        let err = lua
            .load(r#"return require("Some.Missing.Module")"#)
            .eval::<Value>()
            .expect_err("require should fail");

        let msg = err.to_string();
        assert!(
            msg.contains("Some.Missing.Module"),
            "error should mention the requested module: {msg}",
        );
        assert!(
            msg.contains("some/missing/module.lua"),
            "error should mention the converted path: {msg}",
        );
    }
}
