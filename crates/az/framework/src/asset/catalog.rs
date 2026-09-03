//! Asset catalog Bevy resource — the runtime entry point that mirrors
//! Lumberyard's `AzFramework::AssetCatalog`.
//!
//! Azoth native packages ship `assetcatalog.bin` (`AZCATAL`) beside the
//! payload backend. The runtime reads only this native catalog — legacy
//! Lumberyard `RASC`/`RAOC` catalogs are extraction-time inputs owned by
//! offline conversion tools and are never read by the engine at runtime.
//!
//! [`AssetCatalog`] exposes the Lumberyard `AssetCatalogRequestBus` surface
//! — `get_by_path`, `get_by_id`, `disk_path` — as plain methods. If a path or
//! id doesn't resolve, the caller gets `None` and decides what to do; there is
//! no on-disk walking, extension guessing, or manufactured-prefix fallback.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
use az_asset::AssetCatalog as AssetCatalogFile;
use az_asset::{
    ASSET_CATALOG_FILE_NAME, AssetCatalogEntry as AssetCatalogFileEntry,
    AssetCatalogError as AssetCatalogFileError, AssetId, AssetRef, PreparedAssetCatalog,
    normalize_source_path, read_prepared_asset_catalog,
};
use az_core::{AssetData, AssetType};
use bevy::asset::{Asset, AssetServer, Handle, LoadContext};
use bevy::ecs::resource::Resource;
use sha2::{Digest, Sha256};

/// Bevy [`Resource`] wrapping the parsed native catalog plus the package root
/// it was opened from.
#[derive(Resource, Clone)]
pub struct AssetCatalog {
    asset_root: PathBuf,
    inner: Arc<NativeCatalogState>,
}

#[derive(Debug)]
struct NativeCatalogState {
    catalog: PreparedAssetCatalog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetCatalogFormat {
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetCatalogSource {
    Native,
}

impl AssetCatalogSource {
    /// Open the catalog for this source under `asset_root`.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogLoadError::AssetCatalogIo`] if `assetcatalog.bin`
    /// cannot be opened under `asset_root`, and
    /// [`CatalogLoadError::AssetCatalog`] if the file is present but fails to
    /// parse as an `AZCATAL` catalog.
    pub fn open(self, asset_root: impl Into<PathBuf>) -> Result<AssetCatalog, CatalogLoadError> {
        match self {
            Self::Native => AssetCatalog::open_native(asset_root),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AssetCatalogEntry<'a> {
    Native(&'a AssetCatalogFileEntry),
}

impl AssetCatalogEntry<'_> {
    #[must_use]
    pub const fn asset_id(&self) -> AssetId {
        match self {
            Self::Native(entry) => entry.asset_id,
        }
    }

    #[must_use]
    pub fn asset_type(&self) -> AssetType {
        match self {
            Self::Native(entry) => AssetType::from(entry.asset_type),
        }
    }

    #[must_use]
    pub fn relative_path(&self) -> &Path {
        match self {
            Self::Native(entry) => Path::new(entry.path.as_str()),
        }
    }

    /// Canonical authoring address recorded for this product.
    ///
    /// Runtime identity remains [`Self::asset_id`]; this path is metadata for
    /// resolving relationships to sibling products in the source namespace.
    #[must_use]
    pub fn source_path(&self) -> &Path {
        match self {
            Self::Native(entry) => Path::new(entry.source_path.as_str()),
        }
    }

    /// Additional canonical source addresses that identify this product.
    ///
    /// Aliases are catalog-owned metadata. Callers may inspect them after an
    /// [`AssetId`] lookup, but must not use them as a replacement identity.
    #[must_use]
    pub fn catalog_aliases(&self) -> impl ExactSizeIterator<Item = &Path> {
        match self {
            Self::Native(entry) => entry
                .catalog_aliases
                .iter()
                .map(|alias| Path::new(alias.as_str())),
        }
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        match self {
            Self::Native(entry) => entry.byte_len,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogLoadError {
    #[error("asset catalog I/O error at {path}: {source}")]
    AssetCatalogIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("asset catalog parse error: {0}")]
    AssetCatalog(#[from] AssetCatalogFileError),
}

/// Failure to resolve a typed, canonical asset reference through a catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AssetRefLoadError {
    #[error("asset reference {asset_id} is not present in the asset catalog")]
    Missing { asset_id: AssetId },
    #[error(
        "catalog asset {asset_id} has type {actual}, but the typed reference expects {expected}"
    )]
    TypeMismatch {
        asset_id: AssetId,
        expected: AssetType,
        actual: AssetType,
    },
}

impl AssetCatalog {
    /// Open native Azoth package output at
    /// `<asset_root>/assetcatalog.bin`.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogLoadError::AssetCatalogIo`] if
    /// `<asset_root>/assetcatalog.bin` cannot be opened, and
    /// [`CatalogLoadError::AssetCatalog`] if
    /// [`read_prepared_asset_catalog`] rejects the file contents.
    pub fn open_native(asset_root: impl Into<PathBuf>) -> Result<Self, CatalogLoadError> {
        let asset_root = asset_root.into();
        let catalog_path = asset_root.join(ASSET_CATALOG_FILE_NAME);
        let file =
            File::open(&catalog_path).map_err(|source| CatalogLoadError::AssetCatalogIo {
                path: catalog_path.clone(),
                source,
            })?;
        let catalog = read_prepared_asset_catalog(BufReader::new(file))?;
        let state = NativeCatalogState::new(catalog);
        Ok(Self {
            asset_root,
            inner: Arc::new(state),
        })
    }

    /// Asset root the catalogs were opened from. Disk paths returned by
    /// [`Self::disk_path`] are anchored at this root.
    #[inline]
    #[must_use]
    pub fn asset_root(&self) -> &Path {
        &self.asset_root
    }

    /// Look up an entry by relative path. Mirrors
    /// `AssetCatalog::GetAssetIdByPath` followed by `GetAssetInfoById` through
    /// the native catalog's hashed path-to-id index.
    ///
    /// `content_path` is folded to canonical asset-path form inside the
    /// lookup, so callers pass whatever spelling they hold and never
    /// pre-normalize. Returns `None` if the path isn't in the catalog.
    /// **No fallback** to disk-walking and no `scripts/`-prefix guessing:
    /// case and separator insensitivity is the whole of the leniency here.
    #[must_use]
    pub fn get_by_path(&self, content_path: &str) -> Option<AssetCatalogEntry<'_>> {
        self.inner
            .get_by_path(content_path)
            .map(AssetCatalogEntry::Native)
    }

    /// Look up an entry by [`AssetId`]. Mirrors
    /// `AssetCatalog::GetAssetInfoById` through the native catalog index.
    #[inline]
    #[must_use]
    pub fn get_by_id(&self, asset_id: AssetId) -> Option<AssetCatalogEntry<'_>> {
        self.inner
            .get_by_id(asset_id)
            .map(AssetCatalogEntry::Native)
    }

    /// Resolve and load a typed reference at runtime through Bevy's
    /// [`AssetServer`].
    ///
    /// The reference's canonical [`AssetId`] selects the catalog entry. Its
    /// optional path hint is never read; the product path passed to Bevy comes
    /// exclusively from that catalog entry.
    ///
    /// # Errors
    ///
    /// Returns [`AssetRefLoadError::Missing`] if the reference's [`AssetId`] is
    /// not in the catalog, and [`AssetRefLoadError::TypeMismatch`] if the
    /// catalog entry's asset type differs from `T`'s.
    pub fn load_asset_ref<T>(
        &self,
        reference: &AssetRef<T>,
        asset_server: &AssetServer,
    ) -> Result<Handle<T>, AssetRefLoadError>
    where
        T: Asset + AssetData,
    {
        let entry = self.resolve_asset_ref_entry(reference)?;
        Ok(asset_server.load(entry.relative_path().to_path_buf()))
    }

    /// Resolve and load any Bevy asset by its canonical catalog identity.
    ///
    /// This is the runtime route for Bevy-native asset types whose Rust type
    /// cannot implement [`AssetData`], such as `Gltf`. The caller supplies the
    /// engine [`AssetType`] expected at the boundary; the catalog remains the
    /// sole authority for the product path passed to Bevy.
    ///
    /// # Errors
    ///
    /// Returns [`AssetRefLoadError::Missing`] if `asset_id` is not in the
    /// catalog, and [`AssetRefLoadError::TypeMismatch`] if the catalog entry's
    /// asset type differs from `expected`.
    pub fn load_asset_id<T>(
        &self,
        asset_id: AssetId,
        expected: AssetType,
        asset_server: &AssetServer,
    ) -> Result<Handle<T>, AssetRefLoadError>
    where
        T: Asset,
    {
        let entry = self.resolve_asset_id(asset_id, expected)?;
        Ok(asset_server.load(entry.relative_path().to_path_buf()))
    }

    /// Resolve and load a typed reference while building another Bevy asset.
    ///
    /// [`LoadContext::load`] records the catalog-selected product as a load
    /// dependency. As with [`Self::load_asset_ref`], only the canonical
    /// [`AssetId`] is used to select the catalog entry; the path hint is
    /// diagnostic-only.
    ///
    /// # Errors
    ///
    /// Returns [`AssetRefLoadError::Missing`] if the reference's [`AssetId`] is
    /// not in the catalog, and [`AssetRefLoadError::TypeMismatch`] if the
    /// catalog entry's asset type differs from `T`'s.
    pub fn load_asset_ref_in_context<T>(
        &self,
        reference: &AssetRef<T>,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Handle<T>, AssetRefLoadError>
    where
        T: Asset + AssetData,
    {
        let entry = self.resolve_asset_ref_entry(reference)?;
        Ok(load_context.load(entry.relative_path().to_path_buf()))
    }

    /// Resolve and load any Bevy asset while building another asset.
    ///
    /// # Errors
    ///
    /// Returns [`AssetRefLoadError::Missing`] if `asset_id` is not in the
    /// catalog, and [`AssetRefLoadError::TypeMismatch`] if the catalog entry's
    /// asset type differs from `expected`.
    pub fn load_asset_id_in_context<T>(
        &self,
        asset_id: AssetId,
        expected: AssetType,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Handle<T>, AssetRefLoadError>
    where
        T: Asset,
    {
        let entry = self.resolve_asset_id(asset_id, expected)?;
        Ok(load_context.load(entry.relative_path().to_path_buf()))
    }

    /// Resolve an asset identity and validate its product type.
    ///
    /// # Errors
    ///
    /// Returns [`AssetRefLoadError::Missing`] if `asset_id` has no catalog
    /// entry, and [`AssetRefLoadError::TypeMismatch`] if the entry's
    /// [`AssetType`] is not `expected`.
    pub fn resolve_asset_id(
        &self,
        asset_id: AssetId,
        expected: AssetType,
    ) -> Result<AssetCatalogEntry<'_>, AssetRefLoadError> {
        let entry = self
            .get_by_id(asset_id)
            .ok_or(AssetRefLoadError::Missing { asset_id })?;
        let actual = entry.asset_type();
        if actual != expected {
            return Err(AssetRefLoadError::TypeMismatch {
                asset_id,
                expected,
                actual,
            });
        }
        Ok(entry)
    }

    fn resolve_asset_ref_entry<T: AssetData>(
        &self,
        reference: &AssetRef<T>,
    ) -> Result<AssetCatalogEntry<'_>, AssetRefLoadError> {
        self.resolve_asset_id(reference.id(), reference.asset_type())
    }

    /// Borrow every catalog entry with the requested asset type.
    ///
    /// The returned order is the catalog's stable entry order. Callers that
    /// need a path-defined order should sort by [`AssetCatalogEntry::relative_path`].
    #[must_use]
    pub fn entries_by_asset_type(&self, asset_type: AssetType) -> Vec<AssetCatalogEntry<'_>> {
        self.inner
            .catalog
            .entries()
            .iter()
            .filter(|entry| AssetType::from(entry.asset_type) == asset_type)
            .map(AssetCatalogEntry::Native)
            .collect()
    }

    /// Borrow every catalog entry whose content path lies under `prefix`.
    ///
    /// `prefix` is folded to canonical asset-path form and matched on whole
    /// path segments, so `world/regions` selects `world/regions/r_+00_+00.bin`
    /// but never `world/regions_backup/r_+00_+00.bin`. This is the catalog's
    /// answer to "what content ships under this subtree", so callers that need
    /// to enumerate a directory-shaped group of products never walk the disk.
    ///
    /// The returned order is the catalog's stable entry order. Callers that
    /// need a path-defined order should sort by
    /// [`AssetCatalogEntry::relative_path`].
    #[must_use]
    pub fn entries_with_path_prefix(&self, prefix: &str) -> Vec<AssetCatalogEntry<'_>> {
        let prefix = normalize_source_path(prefix);
        self.inner
            .catalog
            .entries()
            .iter()
            .filter(|entry| path_starts_with_segments(entry.path.as_str(), &prefix))
            .map(AssetCatalogEntry::Native)
            .collect()
    }

    /// Resolve an [`AssetId`] to its on-disk absolute path under the
    /// asset root.
    ///
    /// Returns `None` only when the id isn't in the catalog. The caller
    /// gets one deterministic answer; whether the file is actually
    /// present on disk is the caller's concern (typically Bevy's
    /// `AssetReader` will surface a `NotFound` if extraction is
    /// incomplete).
    #[must_use]
    pub fn disk_path_by_id(&self, asset_id: AssetId) -> Option<PathBuf> {
        self.get_by_id(asset_id)
            .map(|entry| self.asset_root.join(entry.relative_path()))
    }

    /// Resolve a content path to its on-disk absolute path. Combines
    /// [`Self::get_by_path`] (catalog lookup) with
    /// `<asset_root>/<relative_path>`. **No fallback** locations are
    /// tried — if the catalog doesn't know the path, this returns
    /// `None`.
    #[must_use]
    pub fn disk_path(&self, content_path: &str) -> Option<PathBuf> {
        self.get_by_path(content_path)
            .map(|entry| self.asset_root.join(entry.relative_path()))
    }

    /// Return the [`AssetId`] for a content path. Mirrors
    /// `AssetCatalog::GetAssetIdByPath` directly.
    #[inline]
    #[must_use]
    pub fn id_for_path(&self, content_path: &str) -> Option<AssetId> {
        self.get_by_path(content_path).map(|entry| entry.asset_id())
    }

    #[inline]
    #[must_use]
    pub const fn format(&self) -> AssetCatalogFormat {
        AssetCatalogFormat::Native
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.catalog.len()
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Deterministic fingerprint of the loaded catalog entries.
    ///
    /// This fingerprints the catalog already opened by the runtime. It does
    /// not read the catalog file again or inspect asset bytes.
    #[must_use]
    pub fn catalog_fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"azoth-native-catalog-v1");
        for entry in self.inner.catalog.entries() {
            hasher.update(entry.asset_id.guid.as_bytes());
            hasher.update(entry.asset_id.sub_id.to_le_bytes());
            hasher.update(entry.asset_type.as_bytes());
            hash_len_prefixed(&mut hasher, entry.product_format.as_bytes());
            hasher.update(entry.product_format_version.to_le_bytes());
            hash_len_prefixed(&mut hasher, entry.path.as_str().as_bytes());
            hasher.update(entry.byte_len.to_le_bytes());
            hasher.update(entry.content_hash);
        }
        let digest = hasher.finalize();
        let mut fingerprint = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(&mut fingerprint, "{byte:02x}");
        }
        fingerprint
    }

    /// Deterministic fingerprint of entries belonging to one runtime asset type.
    ///
    /// This is stable regardless of which products have been loaded and is
    /// suitable for subsystem release identities.
    #[must_use]
    pub fn asset_type_fingerprint(&self, asset_type: AssetType) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"azoth-asset-type-catalog-v1");
        hasher.update(asset_type.as_uuid().as_bytes());
        for entry in self
            .inner
            .catalog
            .entries()
            .iter()
            .filter(|entry| AssetType::from(entry.asset_type) == asset_type)
        {
            hasher.update(entry.asset_id.guid.as_bytes());
            hasher.update(entry.asset_id.sub_id.to_le_bytes());
            hash_len_prefixed(&mut hasher, entry.product_format.as_bytes());
            hasher.update(entry.product_format_version.to_le_bytes());
            hash_len_prefixed(&mut hasher, entry.path.as_str().as_bytes());
            hasher.update(entry.byte_len.to_le_bytes());
            hasher.update(entry.content_hash);
        }
        let digest = hasher.finalize();
        let mut fingerprint = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(&mut fingerprint, "{byte:02x}");
        }
        fingerprint
    }

    /// Deterministic fingerprint of an explicit catalog path set.
    ///
    /// Input order and duplicates do not affect the result. Missing paths are
    /// included as missing markers so independently packaged targets still
    /// derive a stable, comparable subsystem identity.
    ///
    /// Paths are canonicalized with the one asset-path rule before they are
    /// sorted, deduplicated, and hashed, so the digest is keyed by asset
    /// identity — the same key [`Self::get_by_path`] resolves — and not by how
    /// the caller happened to spell it.
    #[must_use]
    pub fn selected_paths_fingerprint(&self, paths: &[&str]) -> String {
        let mut paths = paths
            .iter()
            .copied()
            .map(normalize_source_path)
            .collect::<Vec<_>>();
        paths.sort_unstable();
        paths.dedup();

        let mut hasher = Sha256::new();
        hasher.update(b"azoth-selected-catalog-paths-v1");
        for path in paths {
            hash_len_prefixed(&mut hasher, path.as_bytes());
            let Some(entry) = self.inner.get_by_path(&path) else {
                hasher.update([0]);
                continue;
            };
            hasher.update([1]);
            hasher.update(entry.asset_id.guid.as_bytes());
            hasher.update(entry.asset_id.sub_id.to_le_bytes());
            hasher.update(entry.asset_type.as_bytes());
            hash_len_prefixed(&mut hasher, entry.product_format.as_bytes());
            hasher.update(entry.product_format_version.to_le_bytes());
            hash_len_prefixed(&mut hasher, entry.path.as_str().as_bytes());
            hasher.update(entry.byte_len.to_le_bytes());
            hasher.update(entry.content_hash);
        }
        let digest = hasher.finalize();
        let mut fingerprint = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(&mut fingerprint, "{byte:02x}");
        }
        fingerprint
    }
}

/// Whole-segment prefix test over two already-canonical asset paths.
fn path_starts_with_segments(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

fn hash_len_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

impl NativeCatalogState {
    const fn new(catalog: PreparedAssetCatalog) -> Self {
        Self { catalog }
    }

    fn get_by_path(&self, content_path: &str) -> Option<&AssetCatalogFileEntry> {
        self.catalog.entry_by_path(content_path)
    }

    fn get_by_id(&self, asset_id: AssetId) -> Option<&AssetCatalogFileEntry> {
        self.catalog.entry_by_id(asset_id)
    }
}

#[cfg(test)]
mod tests {
    use az_core::{AzRtti, AzTypeInfo};
    use bevy::app::TaskPoolPlugin;
    use bevy::asset::{AssetApp, AssetPlugin};
    use bevy::prelude::App;
    use bevy::reflect::TypePath;
    use uuid::Uuid;

    use super::*;

    #[derive(Asset, TypePath)]
    struct CatalogReferencedAsset;

    impl AzTypeInfo for CatalogReferencedAsset {
        const NAME: &'static str = "Azoth::CatalogReferencedAsset";
        const TYPE_ID: Uuid = Uuid::from_u128(0x7d54_f47a_830e_4c89_a689_d94c_b3b9_675a);
    }

    impl AzRtti for CatalogReferencedAsset {}

    impl AssetData for CatalogReferencedAsset {
        const STABLE_NAME: &'static str = "az.test.catalog-referenced";
    }

    #[test]
    fn opens_native_product_catalog_by_path_and_asset_id() {
        let temp = tempfile::tempdir().unwrap();
        let asset_id = AssetId::new(uuid::Uuid::from_bytes([1; 16]), 7);
        let asset_type = uuid::Uuid::from_bytes([2; 16]);
        let catalog = AssetCatalogFile::new(vec![
            AssetCatalogFileEntry::new(
                asset_id,
                asset_type,
                "az.test.raw",
                1,
                "materials/armor/foo.mtl",
                Some(3),
                42,
                *blake3::hash(b"compiled material").as_bytes(),
            )
            .with_source_path("authoring/materials/armor/foo.ron")
            .with_catalog_aliases([
                "legacy/materials/armor/foo.mtl",
                "materials/armor/foo.material",
            ]),
        ])
        .unwrap();
        let mut file = std::fs::File::create(temp.path().join(ASSET_CATALOG_FILE_NAME)).unwrap();
        az_asset::write_asset_catalog(&catalog, &mut file).unwrap();

        let catalog = AssetCatalog::open_native(temp.path()).unwrap();

        assert_eq!(catalog.format(), AssetCatalogFormat::Native);
        assert_eq!(catalog.len(), 1);
        let by_path = catalog
            .get_by_path("materials/armor/foo.mtl")
            .expect("catalog entry by path");
        assert_eq!(by_path.asset_id(), asset_id);
        assert_eq!(by_path.asset_type(), AssetType::from(asset_type));
        assert_eq!(
            by_path.relative_path(),
            Path::new("materials/armor/foo.mtl")
        );
        assert_eq!(by_path.size_bytes(), 42);
        let by_id = catalog.get_by_id(asset_id).expect("catalog entry by id");
        assert_eq!(by_id.relative_path(), Path::new("materials/armor/foo.mtl"));
        assert_eq!(
            by_id.source_path(),
            Path::new("authoring/materials/armor/foo.ron")
        );
        assert_eq!(
            by_id.catalog_aliases().collect::<Vec<_>>(),
            [
                Path::new("legacy/materials/armor/foo.mtl"),
                Path::new("materials/armor/foo.material"),
            ]
        );
    }

    #[test]
    fn asset_ref_load_uses_catalog_id_and_ignores_path_hint() {
        let temp = tempfile::tempdir().unwrap();
        let asset_id = AssetId::new(Uuid::from_u128(0x000a_55e7), 19);
        let product_path = "products/catalog-selected.opaque";
        let catalog_file = AssetCatalogFile::new(vec![AssetCatalogFileEntry::new(
            asset_id,
            CatalogReferencedAsset::TYPE_ID,
            "az.test.raw",
            1,
            product_path,
            None,
            0,
            *blake3::hash(b"").as_bytes(),
        )])
        .unwrap();
        let mut file = std::fs::File::create(temp.path().join(ASSET_CATALOG_FILE_NAME)).unwrap();
        az_asset::write_asset_catalog(&catalog_file, &mut file).unwrap();
        let catalog = AssetCatalog::open_native(temp.path()).unwrap();

        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()));
        app.init_asset::<CatalogReferencedAsset>();
        let asset_server = app.world().resource::<AssetServer>();
        let reference = AssetRef::<CatalogReferencedAsset>::new(
            asset_id,
            Some("wrong/diagnostic-hint.with-the-wrong-suffix"),
        );

        let resolved = catalog
            .load_asset_ref(&reference, asset_server)
            .expect("catalog should resolve canonical AssetId");
        let empty_hint = AssetRef::<CatalogReferencedAsset>::new(asset_id, Some(""));
        let resolved_from_empty_hint = catalog
            .load_asset_ref(&empty_hint, asset_server)
            .expect("an empty hint must not affect canonical AssetId resolution");
        let expected: Handle<CatalogReferencedAsset> = asset_server.load(product_path);

        assert_eq!(resolved, expected);
        assert_eq!(resolved_from_empty_hint, expected);
        assert_eq!(
            resolved.path().map(bevy::asset::AssetPath::path),
            Some(Path::new(product_path))
        );
        assert_ne!(
            resolved.path().unwrap().path(),
            Path::new(reference.hint().unwrap())
        );
    }

    #[test]
    fn enumerates_native_entries_by_asset_type() {
        let temp = tempfile::tempdir().unwrap();
        let selected_type = uuid::Uuid::from_bytes([2; 16]);
        let other_type = uuid::Uuid::from_bytes([3; 16]);
        let catalog = AssetCatalogFile::new(vec![
            AssetCatalogFileEntry::new(
                AssetId::new(uuid::Uuid::from_bytes([1; 16]), 1),
                selected_type,
                "az.test.raw",
                1,
                "selected.asset",
                None,
                1,
                *blake3::hash(b"selected").as_bytes(),
            ),
            AssetCatalogFileEntry::new(
                AssetId::new(uuid::Uuid::from_bytes([4; 16]), 2),
                other_type,
                "az.test.raw",
                1,
                "other.asset",
                None,
                1,
                *blake3::hash(b"other").as_bytes(),
            ),
        ])
        .unwrap();
        let mut file = std::fs::File::create(temp.path().join(ASSET_CATALOG_FILE_NAME)).unwrap();
        az_asset::write_asset_catalog(&catalog, &mut file).unwrap();
        let catalog = AssetCatalog::open_native(temp.path()).unwrap();

        let entries = catalog.entries_by_asset_type(AssetType::from(selected_type));

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path(), Path::new("selected.asset"));
    }

    /// Subtree enumeration is how callers discover directory-shaped groups of
    /// products without walking the disk, so it must not leak a sibling whose
    /// name merely starts with the same characters.
    #[test]
    fn enumerates_native_entries_under_a_path_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let asset_type = uuid::Uuid::from_bytes([2; 16]);
        let entry = |index: u8, path: &str| {
            AssetCatalogFileEntry::new(
                AssetId::new(uuid::Uuid::from_bytes([index; 16]), u32::from(index)),
                asset_type,
                "az.test.raw",
                1,
                path,
                None,
                1,
                *blake3::hash(path.as_bytes()).as_bytes(),
            )
        };
        let catalog = AssetCatalogFile::new(vec![
            entry(1, "world/regions/r_+00_+00/region.distribution"),
            entry(2, "world/regions/r_+00_+01/region.slicedata"),
            entry(3, "world/regions_backup/r_+00_+00/region.distribution"),
            entry(4, "world/terrain/world.bin"),
        ])
        .unwrap();
        let mut file = std::fs::File::create(temp.path().join(ASSET_CATALOG_FILE_NAME)).unwrap();
        az_asset::write_asset_catalog(&catalog, &mut file).unwrap();
        let catalog = AssetCatalog::open_native(temp.path()).unwrap();

        let entries = catalog.entries_with_path_prefix(r"World\Regions");

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.relative_path().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            [
                "world/regions/r_+00_+00/region.distribution",
                "world/regions/r_+00_+01/region.slicedata",
            ]
        );
        assert_eq!(catalog.entries_with_path_prefix("").len(), 4);
    }

    #[test]
    fn selected_path_fingerprint_ignores_unrelated_assets_and_tracks_content() {
        fn open_catalog(selected: &[u8], unrelated: &[u8]) -> (tempfile::TempDir, AssetCatalog) {
            let temp = tempfile::tempdir().unwrap();
            let catalog = AssetCatalogFile::new(vec![
                AssetCatalogFileEntry::new(
                    AssetId::new(uuid::Uuid::from_bytes([1; 16]), 1),
                    uuid::Uuid::from_bytes([2; 16]),
                    "az.test.raw",
                    1,
                    "selected.asset",
                    None,
                    selected.len() as u64,
                    *blake3::hash(selected).as_bytes(),
                ),
                AssetCatalogFileEntry::new(
                    AssetId::new(uuid::Uuid::from_bytes([3; 16]), 2),
                    uuid::Uuid::from_bytes([4; 16]),
                    "az.test.raw",
                    1,
                    "unrelated.asset",
                    None,
                    unrelated.len() as u64,
                    *blake3::hash(unrelated).as_bytes(),
                ),
            ])
            .unwrap();
            let mut file =
                std::fs::File::create(temp.path().join(ASSET_CATALOG_FILE_NAME)).unwrap();
            az_asset::write_asset_catalog(&catalog, &mut file).unwrap();
            let opened = AssetCatalog::open_native(temp.path()).unwrap();
            (temp, opened)
        }

        let (_first_root, first) = open_catalog(b"selected-a", b"unrelated-a");
        let (_second_root, second) = open_catalog(b"selected-a", b"unrelated-b");
        let (_third_root, third) = open_catalog(b"selected-b", b"unrelated-a");

        let first = first.selected_paths_fingerprint(&["selected.asset"]);
        assert_eq!(
            first,
            second.selected_paths_fingerprint(&["selected.asset"])
        );
        assert_ne!(first, third.selected_paths_fingerprint(&["selected.asset"]));
    }

    #[test]
    fn native_catalog_follows_explicit_legacy_redirects() {
        let temp = tempfile::tempdir().unwrap();
        let real_id = AssetId::new(uuid::Uuid::from_bytes([1; 16]), 0x181a_6070);
        let legacy_id = AssetId::new(uuid::Uuid::from_bytes([3; 16]), 0xd087_f9c9);
        let catalog = AssetCatalogFile::with_legacy_redirects(
            vec![AssetCatalogFileEntry::new(
                real_id,
                uuid::Uuid::from_bytes([2; 16]),
                "az.test.raw",
                1,
                "slices/dungeon/firstlight/ancientgrate_circular__28236438930.cgf",
                None,
                1668,
                *blake3::hash(b"slice").as_bytes(),
            )],
            vec![az_asset::AssetCatalogLegacyRedirect::new(
                legacy_id, real_id,
            )],
        )
        .unwrap();
        let mut file = std::fs::File::create(temp.path().join(ASSET_CATALOG_FILE_NAME)).unwrap();
        az_asset::write_asset_catalog(&catalog, &mut file).unwrap();

        let catalog = AssetCatalog::open_native(temp.path()).unwrap();

        let resolved = catalog.get_by_id(legacy_id).expect("legacy redirect");
        assert_eq!(resolved.asset_id(), real_id);
        assert_eq!(
            resolved.relative_path(),
            Path::new("slices/dungeon/firstlight/ancientgrate_circular__28236438930.cgf")
        );
        assert!(
            catalog
                .get_by_id(AssetId::new(legacy_id.guid, legacy_id.sub_id + 1))
                .is_none()
        );
    }

    #[test]
    fn native_source_does_not_fallback_to_compatibility_catalogs() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("assetcatalog.catalog"), b"RASC").unwrap();

        let Err(error) = AssetCatalogSource::Native.open(temp.path()) else {
            panic!("native product source must require assetcatalog.bin");
        };

        assert!(
            matches!(error, CatalogLoadError::AssetCatalogIo { path, .. } if path.ends_with(ASSET_CATALOG_FILE_NAME))
        );
    }
}
