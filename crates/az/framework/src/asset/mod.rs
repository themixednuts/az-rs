//! Asset catalog Bevy integration.
//!
//! Lumberyard reference: `Code/Framework/AzFramework/AzFramework/Asset/`
//! (especially `AssetCatalog.cpp` and `AssetCatalogBus.h`). The native asset
//! catalog/package readers live in `crates/az/asset`. This module exposes the
//! native catalog as the runtime [`AssetCatalog`] Bevy `Resource` and provides
//! a [`Plugin`] that opens the native `assetcatalog.bin` at app boot and plumbs
//! it through Bevy's [`AssetReader`](bevy::asset::io::AssetReader) chain via
//! [`CatalogAssetReader`]. Legacy `RASC`/`RAOC` catalogs are extraction-time
//! inputs owned by offline conversion tools; the runtime never reads them.

pub mod catalog;
pub mod reader;

pub use catalog::{
    AssetCatalog, AssetCatalogEntry, AssetCatalogFormat, AssetCatalogSource, AssetRefLoadError,
    CatalogLoadError,
};
pub use reader::{CatalogAssetPayloadBackend, CatalogAssetReader, CatalogAssetReaderOpenError};

use std::path::PathBuf;

use az_asset::PackagePayloadKind;
use bevy::asset::AssetApp;
use bevy::asset::io::{AssetSourceBuilder, AssetSourceId};
use bevy::prelude::*;

/// Bevy plugin making the native Azoth catalog the only asset path authority.
///
/// Opens the native catalog at `asset_root`, exposes it as the
/// [`AssetCatalog`] resource, and installs
/// [`CatalogAssetReader`] as the default [`AssetSource`] so every
/// `asset_server.load::<T>(path)` call resolves through the catalog
/// before touching disk.
///
/// Catalog open errors are **fatal** — the runtime mirrors
/// Lumberyard's behaviour, where an `AssetCatalog`-less process
/// cannot resolve a single asset and is unsalvageable.
///
/// # Ordering
///
/// This plugin **must be added before `DefaultPlugins`** (or before
/// whichever plugin builds `AssetPlugin`). Bevy locks the
/// [`AssetSource`] table in once `AssetPlugin::build` runs; adding
/// `AssetCatalogPlugin` afterwards is a no-op for the reader path
/// (the resource still lands, but the default source stays
/// filesystem-only).
///
/// ```ignore
/// app.add_plugins(AssetCatalogPlugin::new(asset_root))
///    .add_plugins(DefaultPlugins);
/// ```
#[derive(Debug)]
pub struct AssetCatalogPlugin {
    pub asset_root: PathBuf,
    pub catalog_source: AssetCatalogSource,
    pub payload_backend: CatalogAssetPayloadBackend,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssetCatalogPluginConfigError {
    #[error("unsupported native package container `{container}`")]
    UnsupportedNativePackageContainer { container: String },
}

impl AssetCatalogPlugin {
    #[must_use]
    pub fn new(asset_root: impl Into<PathBuf>) -> Self {
        Self {
            asset_root: asset_root.into(),
            catalog_source: AssetCatalogSource::Native,
            payload_backend: CatalogAssetPayloadBackend::Auto,
        }
    }

    #[must_use]
    pub fn native_package(
        mount_root: impl Into<PathBuf>,
        container: PackagePayloadKind,
        payload_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            asset_root: mount_root.into(),
            catalog_source: AssetCatalogSource::Native,
            payload_backend: CatalogAssetPayloadBackend::from_package_payload_kind(
                container,
                payload_path,
            ),
        }
    }

    /// Build a native-package plugin from a service-reported container name.
    ///
    /// # Errors
    ///
    /// Returns [`AssetCatalogPluginConfigError::UnsupportedNativePackageContainer`]
    /// if `container` is not a name [`PackagePayloadKind::from_container_name`]
    /// recognises.
    pub fn try_native_package(
        mount_root: impl Into<PathBuf>,
        container: &str,
        payload_path: impl Into<PathBuf>,
    ) -> Result<Self, AssetCatalogPluginConfigError> {
        let container = PackagePayloadKind::from_container_name(container).ok_or_else(|| {
            AssetCatalogPluginConfigError::UnsupportedNativePackageContainer {
                container: container.to_string(),
            }
        })?;
        Ok(Self::native_package(mount_root, container, payload_path))
    }

    #[must_use]
    pub const fn with_catalog_source(mut self, catalog_source: AssetCatalogSource) -> Self {
        self.catalog_source = catalog_source;
        self
    }

    #[must_use]
    pub fn with_payload_backend(mut self, payload_backend: CatalogAssetPayloadBackend) -> Self {
        self.payload_backend = payload_backend;
        self
    }
}

impl Plugin for AssetCatalogPlugin {
    fn build(&self, app: &mut App) {
        let catalog = match self.catalog_source.open(self.asset_root.clone()) {
            Ok(catalog) => catalog,
            Err(err) => panic!(
                "AssetCatalogPlugin: failed to load catalog at {}: {err}",
                self.asset_root.display(),
            ),
        };

        tracing::info!(
            target: "az_framework::asset",
            asset_root = %self.asset_root.display(),
            source = ?self.catalog_source,
            format = ?catalog.format(),
            entries = catalog.len(),
            "AssetCatalog loaded",
        );

        // Replace the default `AssetSource` reader with one that
        // routes every load through the catalog. The closure is
        // invoked by Bevy once per source instance (one for the
        // unprocessed source, one for the processed source if asset
        // processing is enabled) — we clone the catalog into each
        // because `AssetCatalog` is cheap to clone (inner `Arc`s).
        let catalog_for_reader = catalog.clone();
        let payload_backend = self.payload_backend.clone();
        app.register_asset_source(
            AssetSourceId::Default,
            AssetSourceBuilder::new(move || {
                let payload_backend = payload_backend.clone();
                Box::new(
                    CatalogAssetReader::try_new_with_backend(
                        catalog_for_reader.clone(),
                        payload_backend,
                    )
                    .unwrap_or_else(|err| {
                        panic!("AssetCatalogPlugin: failed to open payload backend: {err}")
                    }),
                )
            }),
        );

        // Expose the catalog as a `Resource` for code paths that
        // need to ask the catalog directly (the Lua `require()`
        // hook, asset-id-driven loads, prefetch heuristics, …).
        app.insert_resource(catalog);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_package_plugin_uses_explicit_payload_backend() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("package");
        let payload = package.join("game.pak");
        let plugin =
            AssetCatalogPlugin::native_package(&package, PackagePayloadKind::Pak, &payload);

        assert_eq!(plugin.asset_root, package);
        assert_eq!(plugin.catalog_source, AssetCatalogSource::Native);
        assert_eq!(
            plugin.payload_backend,
            CatalogAssetPayloadBackend::Pak {
                payload_path: payload
            }
        );
    }

    #[test]
    fn native_package_plugin_accepts_service_reported_container_names() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("package");
        let payload = package.join("game.pak");
        let plugin = AssetCatalogPlugin::try_native_package(&package, "pak", &payload).unwrap();

        assert_eq!(plugin.asset_root, package);
        assert_eq!(plugin.catalog_source, AssetCatalogSource::Native);
        assert_eq!(
            plugin.payload_backend,
            CatalogAssetPayloadBackend::Pak {
                payload_path: payload
            }
        );

        let error =
            AssetCatalogPlugin::try_native_package(&package, "bundle", package.join("game.bundle"))
                .unwrap_err();
        assert!(matches!(
            error,
            AssetCatalogPluginConfigError::UnsupportedNativePackageContainer { .. }
        ));
    }
}
