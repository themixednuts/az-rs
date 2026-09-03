//! `AzFramework` component data + Bevy / Lua / `EBus` integration.
//!
//! Mirrors Lumberyard's `Code/Framework/AzFramework/` layout in spirit:
//!
//! - [`script`] — `AzFramework::ScriptComponent` data (existing).
//! - [`asset`] — `AssetCatalog` Bevy resource backed by native
//!   `assetcatalog.bin`; legacy RASC/RAOC catalogs remain extraction-time inputs
//!   owned by offline conversion tools. Mirrors
//!   O3DE reference: `Code/Framework/AzFramework/AzFramework/Asset/AssetCatalog.cpp`.
//! - [`ebus`] — `EBusRegistry` Bevy resource + `NativeBusHandler` trait;
//!   mirrors O3DE's `Code/Framework/AzCore/AzCore/EBus/EBus.h` plus
//!   O3DE reference: `Code/Framework/AzCore/AzCore/Script/ScriptContext.cpp`
//!   for the Lua-side bus shape.
//! - [`math`] — AZ math layout bridges into Bevy runtime types; mirrors
//!   `AzCore/Math/Transform.h` storage where imported assets expose raw
//!   transform columns.
//! - [`network`] — serialized `AzFramework::NetBindable` base data used by
//!   network-aware components.
//! - [`graph`] — packed visual graph runtime assets (`AZGIR`) loaded through
//!   the asset catalog without editor graph descriptors or project-host data.
//! - [`lua`] — Lua bridge: BMS plugin builder, custom `require` hook,
//!   `Debug` namespace + constructor globals, `EBus` → Lua bus factory.
//!
//! Per-gem code (e.g. `gems/lyshine/src/script/`) hangs gem-specific
//! native bus handlers off [`ebus::EBusRegistry`].

pub mod asset;
pub mod ebus;
pub mod graph;
#[cfg(any(feature = "lua", feature = "lua54", feature = "luajit"))]
pub mod lua;
pub mod math;
pub mod network;
mod script;
pub mod simple_asset;

pub use network::*;
pub use script::*;
pub use simple_asset::*;

/// Registers the Bevy-native Prefab component types owned by `AzFramework`.
///
/// Mirrors `az_render::register_render_prefab_types`: az-framework is not a
/// gem, so this is wired manually into
/// `az_prefab_builder::azscene::engine_prefab_type_registry` alongside
/// az-transform and az-render rather than arriving as composed
/// `az_prefab::PrefabType` entries from a contribution.
pub fn register_framework_prefab_types(registry: &mut bevy::reflect::TypeRegistry) {
    registry.register::<ScriptComponent>();
}

/// The AZ types this crate registers, for a host contribution to register.
#[must_use]
pub const fn types() -> [az_core::AzTypeRegistration; 3] {
    [
        NetBindable::REGISTRATION,
        ScriptComponent::REGISTRATION,
        SimpleAssetReferenceBase::REGISTRATION,
    ]
}

/// Register this crate's AZ types into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<az_core::AzTypeRegistration>()
        .register_many(types());
}

#[cfg(test)]
mod architecture_tests {
    #[test]
    fn runtime_does_not_read_legacy_rasc_raoc_catalogs() {
        // Legacy RASC/RAOC catalogs are extraction-time inputs owned by
        // offline import tools. The runtime framework must not depend on a
        // legacy catalog parser or expose a compatibility-catalog feature.
        let manifest = include_str!("../Cargo.toml");
        assert!(
            !manifest.contains("compat-asset-catalog"),
            "runtime must not expose a legacy compatibility-catalog feature"
        );
        assert!(
            !manifest.contains("az-framework-asset-catalog"),
            "runtime must not depend on the legacy RASC/RAOC catalog parser"
        );
    }

    #[test]
    fn lua_backend_is_framework_owned() {
        let manifest = include_str!("../Cargo.toml");
        assert!(
            manifest.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with("default = [") && line.contains("\"luajit\"")
            }),
            "az-framework owns the single Lua backend choice for the engine"
        );
        assert!(
            manifest.contains("luajit = [\"bevy_mod_scripting/luajit\"]"),
            "az-framework should enable BMS Lua through a framework feature"
        );
    }

    #[test]
    fn graph_runtime_loader_does_not_depend_on_graph_builder() {
        let manifest = include_str!("../Cargo.toml");
        assert!(
            manifest.contains("az-graph-runtime = { workspace = true, features = [\"bevy\"] }"),
            "az-framework should consume the packed graph runtime format with only its Bevy loader feature"
        );
        assert!(
            !manifest.contains("az-graph-builder"),
            "az-framework runtime loading must not depend on build/editor graph compiler crates"
        );
    }
}
