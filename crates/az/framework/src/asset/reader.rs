//! Bevy [`AssetReader`] that routes every load through [`AssetCatalog`].
//!
//! Mirrors Lumberyard's `AssetManager → AssetCatalog::GetAssetInfoById
//! → physical-file-open` chain: the catalog is the **only** path
//! authority. Loads for paths the shipped catalog doesn't know about
//! resolve to [`AssetReaderError::NotFound`]; there is no on-disk
//! walking, no extension guessing, no manufactured-prefix fallback.
//!
//! The actual payload IO is delegated to the selected package backend:
//! Bevy's stock [`FileAssetReader`] for loose products, `AzPackReader`
//! for native chunked packages, or `PakMmapReader` for explicit
//! Lumberyard/O3DE-compatible pak outputs. Only path resolution moves
//! into the catalog.
//!
//! Registered as the default [`AssetSource`] by
//! [`super::AssetCatalogPlugin::build`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use az_asset::{AZPACK_INDEX_FILE_NAME, AzPackReadError, AzPackReader, PackagePayloadKind};
use az_pak::{PakError, PakMmapReader};
use bevy::asset::io::file::FileAssetReader;
use bevy::asset::io::{AssetReader, AssetReaderError, PathStream, Reader, VecReader};

use super::catalog::{AssetCatalog, AssetCatalogEntry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogAssetPayloadBackend {
    Auto,
    Loose,
    AzPack,
    Pak { payload_path: PathBuf },
}

impl CatalogAssetPayloadBackend {
    #[must_use]
    pub fn pak(payload_path: impl Into<PathBuf>) -> Self {
        Self::Pak {
            payload_path: payload_path.into(),
        }
    }

    #[must_use]
    pub fn from_package_payload_kind(
        kind: PackagePayloadKind,
        payload_path: impl Into<PathBuf>,
    ) -> Self {
        match kind {
            PackagePayloadKind::Loose => Self::Loose,
            PackagePayloadKind::AzPack => Self::AzPack,
            PackagePayloadKind::Pak => Self::pak(payload_path),
        }
    }

    #[must_use]
    pub fn from_container_and_payload(
        container: &str,
        payload_path: impl Into<PathBuf>,
    ) -> Option<Self> {
        PackagePayloadKind::from_container_name(container)
            .map(|kind| Self::from_package_payload_kind(kind, payload_path))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogAssetReaderOpenError {
    #[error("failed to open azpack payload reader: {0}")]
    AzPack(#[from] AzPackReadError),
    #[error("failed to open pak payload reader at {path}: {source}")]
    Pak {
        path: PathBuf,
        #[source]
        source: PakError,
    },
    #[error("multiple pak payloads found under {root}; pass an explicit pak payload path")]
    AmbiguousPakPayload {
        root: PathBuf,
        first: PathBuf,
        second: PathBuf,
    },
}

/// A Bevy [`AssetReader`] backed by an [`AssetCatalog`].
///
/// Cheap to construct — `AssetCatalog` itself is `Clone`, and the
/// payload backend is either a fresh Bevy `FileAssetReader` for loose
/// roots or a shared native `AzPackReader` for chunked package roots.
pub struct CatalogAssetReader {
    catalog: AssetCatalog,
    backend: CatalogAssetReaderBackend,
}

enum CatalogAssetReaderBackend {
    Loose { file_reader: FileAssetReader },
    AzPack { reader: Arc<AzPackReader> },
    Pak { reader: Arc<PakMmapReader> },
}

impl CatalogAssetReader {
    /// Build a reader for the given catalog. The underlying
    /// [`FileAssetReader`] is rooted at [`AssetCatalog::asset_root`]
    /// so the catalog's `relative_path` resolves directly.
    ///
    /// # Panics
    ///
    /// Panics if backend auto-detection fails, i.e. whenever
    /// [`Self::try_new`] would return a [`CatalogAssetReaderOpenError`].
    #[must_use]
    pub fn new(catalog: AssetCatalog) -> Self {
        Self::try_new(catalog).unwrap_or_else(|err| panic!("failed to open catalog reader: {err}"))
    }

    /// Build a reader for the given catalog, auto-detecting the payload backend.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogAssetReaderOpenError::AzPack`] if an `azpack` index is
    /// present but cannot be opened, [`CatalogAssetReaderOpenError::Pak`] if the
    /// asset root cannot be scanned or the discovered pak payload cannot be
    /// opened, and [`CatalogAssetReaderOpenError::AmbiguousPakPayload`] if the
    /// asset root holds more than one `.pak` payload.
    pub fn try_new(catalog: AssetCatalog) -> Result<Self, CatalogAssetReaderOpenError> {
        Self::try_new_with_backend(catalog, CatalogAssetPayloadBackend::Auto)
    }

    /// Build a reader for the given catalog using an explicit payload backend.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogAssetReaderOpenError::AzPack`] if the `azpack` payload
    /// reader cannot open the asset root, [`CatalogAssetReaderOpenError::Pak`]
    /// if the asset root cannot be scanned or the pak payload cannot be opened,
    /// and [`CatalogAssetReaderOpenError::AmbiguousPakPayload`] if
    /// [`CatalogAssetPayloadBackend::Auto`] discovers more than one `.pak`
    /// payload under the asset root.
    pub fn try_new_with_backend(
        catalog: AssetCatalog,
        backend: CatalogAssetPayloadBackend,
    ) -> Result<Self, CatalogAssetReaderOpenError> {
        let asset_root = catalog.asset_root().to_path_buf();
        let backend = match backend {
            CatalogAssetPayloadBackend::Auto => {
                if asset_root.join(AZPACK_INDEX_FILE_NAME).exists() {
                    CatalogAssetReaderBackend::AzPack {
                        reader: Arc::new(AzPackReader::open(&asset_root)?),
                    }
                } else if let Some(payload_path) = discover_single_pak_payload(&asset_root)? {
                    CatalogAssetReaderBackend::Pak {
                        reader: Arc::new(open_pak_payload_reader(payload_path)?),
                    }
                } else {
                    CatalogAssetReaderBackend::Loose {
                        file_reader: FileAssetReader::new(asset_root),
                    }
                }
            }
            CatalogAssetPayloadBackend::Loose => CatalogAssetReaderBackend::Loose {
                file_reader: FileAssetReader::new(asset_root),
            },
            CatalogAssetPayloadBackend::AzPack => CatalogAssetReaderBackend::AzPack {
                reader: Arc::new(AzPackReader::open(&asset_root)?),
            },
            CatalogAssetPayloadBackend::Pak { payload_path } => CatalogAssetReaderBackend::Pak {
                reader: Arc::new(open_pak_payload_reader(payload_path)?),
            },
        };
        Ok(Self { catalog, backend })
    }

    /// Build a reader that always reads loose products from the asset root.
    ///
    /// # Panics
    ///
    /// Panics if [`Self::try_new_with_backend`] fails for
    /// [`CatalogAssetPayloadBackend::Loose`]. That branch only calls
    /// [`FileAssetReader::new`], which is infallible, so the panic is
    /// unreachable.
    #[must_use]
    pub fn new_loose(catalog: AssetCatalog) -> Self {
        Self::try_new_with_backend(catalog, CatalogAssetPayloadBackend::Loose)
            .expect("loose catalog reader construction cannot fail")
    }

    /// Build a reader that reads products from the native `azpack` payload at
    /// the catalog's asset root.
    ///
    /// # Errors
    ///
    /// Returns the [`AzPackReadError`] raised by `AzPackReader::open` when the
    /// asset root is not a complete `azpack` package.
    pub fn new_azpack(catalog: AssetCatalog) -> Result<Self, AzPackReadError> {
        Self::try_new_with_backend(catalog, CatalogAssetPayloadBackend::AzPack).map_err(|error| {
            match error {
                CatalogAssetReaderOpenError::AzPack(error) => error,
                CatalogAssetReaderOpenError::Pak { .. } => {
                    unreachable!("azpack reader construction cannot open pak payloads")
                }
                CatalogAssetReaderOpenError::AmbiguousPakPayload { .. } => {
                    unreachable!("azpack reader construction cannot discover pak payloads")
                }
            }
        })
    }

    /// Build a reader that reads products from an explicit pak payload.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogAssetReaderOpenError::Pak`] if `payload_path` cannot be
    /// opened by [`PakMmapReader::open`].
    pub fn new_pak(
        catalog: AssetCatalog,
        payload_path: impl Into<PathBuf>,
    ) -> Result<Self, CatalogAssetReaderOpenError> {
        Self::try_new_with_backend(catalog, CatalogAssetPayloadBackend::pak(payload_path))
    }
}

impl AssetReader for CatalogAssetReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        // The catalog stores normalised forward-slash UTF-8; reject
        // anything else outright instead of guessing.
        let path_str = path
            .to_str()
            .ok_or_else(|| AssetReaderError::NotFound(path.to_path_buf()))?;

        // Catalog lookup is the only path authority. A missing entry
        // is `NotFound`, exactly like Lumberyard's
        // `AssetCatalog::GetAssetIdByPath` returning a null id.
        let entry = self.catalog.get_by_path(path_str).ok_or_else(|| {
            tracing::trace!(
                target: "az_framework::asset::reader",
                requested = %path.display(),
                "no AssetCatalog entry for path",
            );
            AssetReaderError::NotFound(path.to_path_buf())
        })?;

        // The catalog's `relative_path` is what the underlying
        // `FileAssetReader` needs (relative to `asset_root`).
        // For the identity-on-pak-source layout the extractor
        // produces, this is byte-identical to the requested path,
        // but we honour the catalog's answer unconditionally so any
        // future alias rules ride along for free.
        let product_path = entry_relative_path(entry);
        match &self.backend {
            CatalogAssetReaderBackend::Loose { file_reader } => {
                let reader = file_reader.read(product_path).await?;
                Ok(Box::new(reader) as Box<dyn Reader + 'a>)
            }
            CatalogAssetReaderBackend::AzPack { reader } => {
                let product_path = product_path_to_string(path, product_path)?;
                let requested_path = path.to_path_buf();
                let bytes = reader.read_product_path(product_path).map_err(|error| {
                    azpack_read_error_to_asset_reader_error(error, requested_path)
                })?;
                Ok(Box::new(VecReader::new(bytes)) as Box<dyn Reader + 'a>)
            }
            CatalogAssetReaderBackend::Pak { reader } => {
                let product_path = product_path_to_string(path, product_path)?;
                let requested_path = path.to_path_buf();
                let bytes = reader
                    .read(&product_path)
                    .map_err(|error| pak_read_error_to_asset_reader_error(error, requested_path))?;
                Ok(Box::new(VecReader::new(bytes)) as Box<dyn Reader + 'a>)
            }
        }
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        // Compatibility asset roots may omit Bevy `.meta` sidecars. Delegate to
        // `FileAssetReader` which returns `NotFound` when no `.meta`
        // is present on disk; Bevy treats that as "no overrides,
        // use loader defaults" — the right answer for every shipped
        // asset.
        match &self.backend {
            CatalogAssetReaderBackend::Loose { file_reader } => {
                Ok(Box::new(file_reader.read_meta(path).await?) as Box<dyn Reader + 'a>)
            }
            // Neither package backend stores `.meta` sidecars, so both report
            // the same `NotFound` Bevy reads as "use loader defaults".
            CatalogAssetReaderBackend::AzPack { .. } | CatalogAssetReaderBackend::Pak { .. } => {
                Err(AssetReaderError::NotFound(meta_path_for_not_found(path)))
            }
        }
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        // The catalog is a flat path table; directory enumeration
        // would mean rebuilding a prefix tree on every call. The
        // compatibility runtimes load by path or id instead of enumerating
        // directories, so refuse the operation
        // rather than fake an answer.
        Err(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        // Same rationale as `read_directory`: the catalog is a flat
        // path → id table with no concept of directories. Reporting
        // every path as a non-directory matches Bevy's default
        // behaviour for asset sources that don't model directories.
        Ok(false)
    }
}

fn entry_relative_path(entry: AssetCatalogEntry<'_>) -> &Path {
    match entry {
        AssetCatalogEntry::Native(entry) => Path::new(entry.path.as_str()),
    }
}

fn product_path_to_string(
    requested_path: &Path,
    product_path: &Path,
) -> Result<String, AssetReaderError> {
    product_path
        .to_str()
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| AssetReaderError::NotFound(requested_path.to_path_buf()))
}

fn azpack_read_error_to_asset_reader_error(
    error: AzPackReadError,
    requested_path: PathBuf,
) -> AssetReaderError {
    match error {
        AzPackReadError::MissingProductPath { .. } | AzPackReadError::MissingAssetId { .. } => {
            AssetReaderError::NotFound(requested_path)
        }
        other => AssetReaderError::Io(Arc::new(std::io::Error::other(other))),
    }
}

fn pak_read_error_to_asset_reader_error(
    error: PakError,
    requested_path: PathBuf,
) -> AssetReaderError {
    match error {
        PakError::NotFound(_) => AssetReaderError::NotFound(requested_path),
        other => AssetReaderError::Io(Arc::new(std::io::Error::other(other))),
    }
}

fn discover_single_pak_payload(
    asset_root: &Path,
) -> Result<Option<PathBuf>, CatalogAssetReaderOpenError> {
    let mut pak_payload = None;
    let read_dir = match std::fs::read_dir(asset_root) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CatalogAssetReaderOpenError::Pak {
                path: asset_root.to_path_buf(),
                source: PakError::Io(error),
            });
        }
    };

    for entry in read_dir {
        let entry = entry.map_err(|source| CatalogAssetReaderOpenError::Pak {
            path: asset_root.to_path_buf(),
            source: PakError::Io(source),
        })?;
        let path = entry.path();
        if !path.is_file() || !path_has_extension(&path, "pak") {
            continue;
        }
        if let Some(first) = pak_payload.replace(path.clone()) {
            return Err(CatalogAssetReaderOpenError::AmbiguousPakPayload {
                root: asset_root.to_path_buf(),
                first,
                second: path,
            });
        }
    }
    Ok(pak_payload)
}

fn open_pak_payload_reader(
    payload_path: PathBuf,
) -> Result<PakMmapReader, CatalogAssetReaderOpenError> {
    PakMmapReader::open(&payload_path).map_err(|source| CatalogAssetReaderOpenError::Pak {
        path: payload_path,
        source,
    })
}

fn path_has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

fn meta_path_for_not_found(path: &Path) -> PathBuf {
    let mut path = path.as_os_str().to_os_string();
    path.push(".meta");
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use az_asset::{
        AssetCatalog as AssetCatalogFile, PACKAGE_CONTENT_HASH_BYTES, PackageManifest,
        PackageManifestEntry, PackageManifestProfile, PackagePayloadWriteRequest,
        write_asset_catalog, write_package_payload,
    };
    use az_filesystem::default_platform_product_cache_dir;
    use bevy::asset::io::{AssetReader as _, Reader as _};
    use bevy::tasks::block_on;

    use super::*;

    #[test]
    fn payload_backend_uses_package_payload_kind_without_path_probing() {
        assert_eq!(
            CatalogAssetPayloadBackend::from_package_payload_kind(
                az_asset::PackagePayloadKind::Loose,
                "ignored.pak",
            ),
            CatalogAssetPayloadBackend::Loose
        );
        assert_eq!(
            CatalogAssetPayloadBackend::from_package_payload_kind(
                az_asset::PackagePayloadKind::AzPack,
                "ignored.pak",
            ),
            CatalogAssetPayloadBackend::AzPack
        );
        assert_eq!(
            CatalogAssetPayloadBackend::from_package_payload_kind(
                az_asset::PackagePayloadKind::Pak,
                std::env::temp_dir().join("packages/game.pak"),
            ),
            CatalogAssetPayloadBackend::Pak {
                payload_path: std::env::temp_dir().join("packages/game.pak")
            }
        );
    }

    #[test]
    fn reads_native_azpack_product_through_catalog_path() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let bytes = b"compiled prefab bytes for runtime reader";
        let package_root = write_azpack_fixture(temp.path(), product_path, bytes);

        let catalog = AssetCatalog::open_native(&package_root).unwrap();
        let reader = CatalogAssetReader::new(catalog);

        let mut product_reader = block_on(reader.read(Path::new(product_path))).unwrap();
        let mut actual = Vec::new();
        block_on(product_reader.read_to_end(&mut actual)).unwrap();

        assert_eq!(actual, bytes);
    }

    #[test]
    fn azpack_reader_rejects_paths_missing_from_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let package_root = write_azpack_fixture(
            temp.path(),
            "products/prefab.azbin",
            b"compiled prefab bytes for runtime reader",
        );

        let catalog = AssetCatalog::open_native(&package_root).unwrap();
        let reader = CatalogAssetReader::new(catalog);
        let Err(error) = block_on(reader.read(Path::new("products/missing.azbin"))) else {
            panic!("missing catalog path should not return a reader");
        };

        assert!(
            matches!(error, AssetReaderError::NotFound(path) if path == *"products/missing.azbin")
        );
    }

    #[test]
    fn explicit_azpack_backend_reports_incomplete_package_root() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let manifest = PackageManifest::new(
            PackageManifestProfile {
                name: "pc-release".to_string(),
                asset_platform: "pc".to_string(),
                cargo_profile: "release".to_string(),
                container: "azpack".to_string(),
                compression: "none".to_string(),
                oodle_compressor: None,
                oodle_effort: None,
            },
            vec![PackageManifestEntry::new(
                product_path,
                uuid::Uuid::from_bytes([2; 16]),
                7,
                "az.test.raw",
                1,
                content_hash(b"compiled prefab bytes"),
                b"compiled prefab bytes".len() as u64,
                uuid::Uuid::from_bytes([1; 16]),
                "prefabs/source.prefab.ron",
                "BuildPrefab",
            )],
        )
        .unwrap();
        let catalog = AssetCatalogFile::from_package_manifest(&manifest).unwrap();
        let mut catalog_file =
            fs::File::create(temp.path().join(az_asset::ASSET_CATALOG_FILE_NAME)).unwrap();
        write_asset_catalog(&catalog, &mut catalog_file).unwrap();

        let catalog = AssetCatalog::open_native(temp.path()).unwrap();
        let Err(error) =
            CatalogAssetReader::try_new_with_backend(catalog, CatalogAssetPayloadBackend::AzPack)
        else {
            panic!("incomplete azpack root should not open");
        };

        assert!(matches!(error, CatalogAssetReaderOpenError::AzPack(_)));
    }

    #[test]
    fn reads_compatible_pak_product_through_catalog_path() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let bytes = b"compiled prefab bytes from pak payload";
        let package_root = write_pak_fixture(temp.path(), product_path, bytes);

        let catalog = AssetCatalog::open_native(&package_root).unwrap();
        let reader = CatalogAssetReader::new(catalog);

        let mut product_reader = block_on(reader.read(Path::new(product_path))).unwrap();
        let mut actual = Vec::new();
        block_on(product_reader.read_to_end(&mut actual)).unwrap();

        assert_eq!(actual, bytes);
    }

    #[test]
    fn explicit_pak_backend_reports_missing_payload() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        write_product_catalog_fixture(temp.path(), product_path, b"compiled prefab bytes");

        let catalog = AssetCatalog::open_native(temp.path()).unwrap();
        let missing_pak = temp.path().join("missing.pak");
        let Err(error) = CatalogAssetReader::try_new_with_backend(
            catalog,
            CatalogAssetPayloadBackend::pak(&missing_pak),
        ) else {
            panic!("missing pak payload should not open");
        };

        assert!(
            matches!(error, CatalogAssetReaderOpenError::Pak { path, .. } if path == missing_pak)
        );
    }

    fn write_azpack_fixture(root: &Path, product_path: &str, bytes: &[u8]) -> PathBuf {
        let cache_root = default_platform_product_cache_dir(root);
        let cache_product = cache_root.join(product_path);
        fs::create_dir_all(cache_product.parent().unwrap()).unwrap();
        fs::write(&cache_product, bytes).unwrap();

        let manifest = PackageManifest::new(
            PackageManifestProfile {
                name: "pc-release".to_string(),
                asset_platform: "pc".to_string(),
                cargo_profile: "release".to_string(),
                container: "azpack".to_string(),
                compression: "none".to_string(),
                oodle_compressor: None,
                oodle_effort: None,
            },
            vec![PackageManifestEntry::new(
                product_path,
                uuid::Uuid::from_bytes([2; 16]),
                7,
                "az.test.raw",
                1,
                content_hash(bytes),
                bytes.len() as u64,
                uuid::Uuid::from_bytes([1; 16]),
                "prefabs/source.prefab.ron",
                "BuildPrefab",
            )],
        )
        .unwrap();

        let receipt = write_package_payload(PackagePayloadWriteRequest::new(
            &manifest,
            &cache_root,
            &root.join("package-output"),
        ))
        .unwrap();
        let catalog = AssetCatalogFile::from_package_manifest(&manifest).unwrap();
        let mut catalog_file =
            fs::File::create(receipt.path.join(az_asset::ASSET_CATALOG_FILE_NAME)).unwrap();
        write_asset_catalog(&catalog, &mut catalog_file).unwrap();
        receipt.path
    }

    fn write_pak_fixture(root: &Path, product_path: &str, bytes: &[u8]) -> PathBuf {
        let cache_root = default_platform_product_cache_dir(root);
        let cache_product = cache_root.join(product_path);
        fs::create_dir_all(cache_product.parent().unwrap()).unwrap();
        fs::write(&cache_product, bytes).unwrap();

        let manifest = PackageManifest::new(
            PackageManifestProfile {
                name: "pc-release".to_string(),
                asset_platform: "pc".to_string(),
                cargo_profile: "release".to_string(),
                container: "pak".to_string(),
                compression: "none".to_string(),
                oodle_compressor: None,
                oodle_effort: None,
            },
            vec![PackageManifestEntry::new(
                product_path,
                uuid::Uuid::from_bytes([2; 16]),
                7,
                "az.test.raw",
                1,
                content_hash(bytes),
                bytes.len() as u64,
                uuid::Uuid::from_bytes([1; 16]),
                "prefabs/source.prefab.ron",
                "BuildPrefab",
            )],
        )
        .unwrap();

        let package_root = root.join("package-output");
        write_package_payload(PackagePayloadWriteRequest::new(
            &manifest,
            &cache_root,
            &package_root,
        ))
        .unwrap();
        write_product_catalog_for_manifest(&package_root, &manifest);
        package_root
    }

    fn write_product_catalog_fixture(root: &Path, product_path: &str, bytes: &[u8]) {
        let manifest = PackageManifest::new(
            PackageManifestProfile {
                name: "pc-release".to_string(),
                asset_platform: "pc".to_string(),
                cargo_profile: "release".to_string(),
                container: "pak".to_string(),
                compression: "none".to_string(),
                oodle_compressor: None,
                oodle_effort: None,
            },
            vec![PackageManifestEntry::new(
                product_path,
                uuid::Uuid::from_bytes([2; 16]),
                7,
                "az.test.raw",
                1,
                content_hash(bytes),
                bytes.len() as u64,
                uuid::Uuid::from_bytes([1; 16]),
                "prefabs/source.prefab.ron",
                "BuildPrefab",
            )],
        )
        .unwrap();
        write_product_catalog_for_manifest(root, &manifest);
    }

    fn write_product_catalog_for_manifest(root: &Path, manifest: &PackageManifest) {
        let catalog = AssetCatalogFile::from_package_manifest(manifest).unwrap();
        let mut catalog_file =
            fs::File::create(root.join(az_asset::ASSET_CATALOG_FILE_NAME)).unwrap();
        write_asset_catalog(&catalog, &mut catalog_file).unwrap();
    }

    fn content_hash(bytes: &[u8]) -> [u8; PACKAGE_CONTENT_HASH_BYTES] {
        *blake3::hash(bytes).as_bytes()
    }
}
