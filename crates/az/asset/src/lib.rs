//! Azoth asset IDs, native asset catalogs, and package formats.
//!
//! Native Azoth runtime output is `assetcatalog.bin` plus a package payload
//! backend such as loose products or `azpack`. Legacy Lumberyard/O3DE
//! `RASC`/`RAOC` catalogs are extraction-time inputs owned by offline import
//! tools and are never read by the engine at runtime.
//!
//! # Quick start
//!
//! ```no_run
//! use az_asset::{AssetId, AssetCatalog, read_asset_catalog};
//!
//! let catalog = read_asset_catalog(std::io::empty())?;
//! for entry in catalog.entries() {
//!     let id = AssetId::new(entry.asset_id.guid, entry.asset_id.sub_id);
//!     println!("{id} -> {}", entry.path);
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Scope
//!
//! Main consumers:
//!
//! - **Build time** — `azoth build` writes package manifests,
//!   `assetcatalog.bin`, and the selected payload backend.
//! - **Runtime** — `az-framework` opens native `assetcatalog.bin` for
//!   Azoth package roots.
//!
//! Runtime loading uses Bevy's `AssetServer`; this crate does not reimplement
//! Lumberyard's `AssetManager`.

pub mod asset;
pub mod azpack;
pub mod catalog;
pub mod package;
pub mod package_payload;
pub mod reference;
pub mod texture_format;
pub use asset::{AssetRef, AssetTypeMismatch, ReflectAssetDependency};
// Path-string helpers live in `az_filesystem`; re-exported here as a
// convenience because every asset-handling caller wants them.
pub use az_core::{AssetId, AssetIdBytesError, AssetIdParseError};
pub use az_filesystem::{normalize_source_path, source_extension_key};
pub use azpack::{
    AZPACK_DEFAULT_CHUNK_SIZE, AZPACK_INDEX_FILE_NAME, AZPACK_INDEX_MAGIC, AZPACK_INDEX_VERSION,
    AzPackChunkCompression, AzPackIndex, AzPackIndexChunk, AzPackIndexEntry, AzPackIndexError,
    AzPackIndexProfile, AzPackReadError, AzPackReader, read_azpack_index, write_azpack_index,
};
pub use catalog::{
    ASSET_CATALOG_FILE_NAME, ASSET_CATALOG_MAGIC, ASSET_CATALOG_VERSION, AssetCatalog,
    AssetCatalogEntry, AssetCatalogError, AssetCatalogLegacyRedirect, AssetCatalogPathRegistration,
    AssetCatalogStreamEncoder, AssetCatalogWriteReceipt, AssetTreePath, PreparedAssetCatalog,
    ProductDependency, read_asset_catalog, read_prepared_asset_catalog, write_asset_catalog,
};
pub use package::{
    PACKAGE_COMPRESSION_NONE, PACKAGE_COMPRESSION_OODLE, PACKAGE_CONTAINER_AZPACK,
    PACKAGE_CONTAINER_LOOSE, PACKAGE_CONTAINER_PAK, PACKAGE_CONTENT_HASH_BYTES,
    PACKAGE_MANIFEST_MAGIC, PACKAGE_MANIFEST_VERSION, PACKAGE_RELEASE_ID_BYTES, PackageManifest,
    PackageManifestEntry, PackageManifestError, PackageManifestProfile,
    format_package_content_hash_hex, format_package_release_id_hex, package_manifest_release_id,
    parse_package_content_hash_hex, read_package_manifest, write_package_manifest,
};
#[cfg(feature = "oodle")]
pub use package_payload::OodlePackageCompressor;
pub use package_payload::{
    AzPackContainer, AzPackPayloadWriter, ChunkAddressablePackageContainer,
    CompatibilityPackageContainer, LooseContainer, LoosePayloadWriter, NoCompression,
    OodleCompression, PackageBackendCapabilities, PackageCompressionKind, PackageCompressionMarker,
    PackageCompressionMethod, PackageContainerMarker, PackageEntryCompressor, PackagePayloadError,
    PackagePayloadKind, PackagePayloadLayout, PackagePayloadPolicy, PackagePayloadReceipt,
    PackagePayloadWriteRequest, PackagePayloadWriter, PakContainer, PakPayloadWriter,
    ParallelPreparedPackageContainer, PatchFriendlyPackageContainer, StoredPackageCompressor,
    StreamablePackageContainer, package_payload_layout, write_package_payload,
};
pub use reference::{UntypedAssetRef, UntypedAssetRefPlugin};
pub use texture_format::{
    EngineTextureFormat, engine_texture_asset_path, same_stem_texture_asset_path,
};
