//! Native asset catalog for the engine asset tree.
//!
//! O3DE reference: `Code/Tools/AssetProcessor/native/AssetManager/AssetCatalog.cpp`.
//!
//! The native `AZCATAL` layout is intentionally streamable. Its schema version
//! is [`ASSET_CATALOG_VERSION`]; this prose deliberately does not restate the
//! number, which drifted to `v4` here while the constant said `5`:
//!
//! ```text
//! Header: magic, version, entry_count, legacy_redirect_count
//!   entries[entry_count]              length-delimited product records,
//!                                     including zero or more path aliases,
//!                                     each carrying its runtime product
//!                                     dependency list
//!   legacy_redirects[redirect_count]  fixed AssetId -> AssetId redirects
//! ```
//!
//! Writers append a fixed-width optimized index after the canonical stream in
//! the same file. Streaming readers can stop after the canonical records.
//! Runtime readers can use the tail index for path/id/legacy lookup without
//! making the streamable section non-canonical.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path};

use az_filesystem::normalize_source_path;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::AssetId;
use crate::package::{PACKAGE_CONTENT_HASH_BYTES, PackageManifest};

/// Asset catalog schema version.
///
/// Pre-release this stays at 1. Both readers reject a mismatched header
/// outright and no migration path exists, so an incompatible catalog change
/// is answered by changing the format and reprocessing, never by incrementing
/// this gate. The number starts moving at the release boundary, when a
/// catalog that shipped can no longer be reprocessed out from under a user.
pub const ASSET_CATALOG_VERSION: u32 = 1;

/// Asset catalog filename in a package output.
pub const ASSET_CATALOG_FILE_NAME: &str = "assetcatalog.bin";

/// Asset catalog binary marker.
///
/// Keep format family identity separate from the header version. The
/// following `u32` version field owns compatibility.
pub const ASSET_CATALOG_MAGIC: &[u8; 8] = b"AZCATAL\0";
pub const ASSET_CATALOG_OPTIMIZED_INDEX_MAGIC: &[u8; 8] = b"AZCATIX\0";
pub const ASSET_CATALOG_OPTIMIZED_INDEX_FOOTER_MAGIC: &[u8; 8] = b"AZCATFT\0";
pub const ASSET_CATALOG_OPTIMIZED_INDEX_VERSION: u32 = 1;

const OPTIMIZED_INDEX_HEADER_SIZE: usize = 24;
const OPTIMIZED_ID_ROW_SIZE: usize = 24;
const OPTIMIZED_PATH_ROW_SIZE: usize = 24;
const OPTIMIZED_LEGACY_ROW_SIZE: usize = 24;
const OPTIMIZED_INDEX_FOOTER_SIZE: usize = 24;

/// Relative path inside an asset tree, in canonical form.
///
/// Asset paths are **case-insensitive identities**, inherited from
/// Lumberyard's cross-platform asset semantics: `Objects/Foo.CGF` and
/// `objects\foo.cgf` name the same asset. This type is where that invariant
/// lives.
///
/// * **Canonical storage.** The wrapped string is always the folded, canonical
///   spelling produced by [`normalize_source_path`] — lowercase, `/`-separated,
///   relative, with no redundant separators. Construction is the only way in,
///   so an `AssetTreePath` cannot hold a non-canonical value.
/// * **Byte stability.** That spelling is a persistence and wire contract. It
///   is written verbatim into `assetcatalog.bin` and `azpack` index records,
///   keys asset-database rows, and is hashed into `AssetId` GUIDs and catalog
///   path hashes. Changing the canonical form invalidates every processed
///   catalog and stored id, so pre-release it changes only together with a
///   full reprocess of every affected artifact.
/// * **Comparison is byte comparison.** Because construction folds, `Eq`,
///   `Ord`, and `Hash` are plain byte operations, and every by-path lookup
///   surface folds its argument by constructing this type. Callers never need
///   to pre-normalize a path to query one.
/// * **Display casing is out of band.** Authored capitalization is *not* part
///   of the identity and is deliberately not recoverable from this type. Any
///   caller that needs to show a human-authored spelling (capture pins, editor
///   labels, diagnostics quoting source text) owns that string separately, as
///   metadata alongside the canonical path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AssetTreePath(String);

impl AssetTreePath {
    /// Create a canonical asset-tree path.
    ///
    /// Delegates to [`normalize_source_path`], the single canonicalization for
    /// the asset-path domain. Because that transform is idempotent, passing an
    /// already-canonical path is free of surprises.
    #[must_use]
    pub fn new(path: impl AsRef<str>) -> Self {
        Self(normalize_source_path(path))
    }

    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for AssetTreePath {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for AssetTreePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for AssetTreePath {
    #[inline]
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for AssetTreePath {
    #[inline]
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// Stable 128-bit path key used by the optimized in-file index.
///
/// This is an Azoth-native key, not Lumberyard's `Uuid::CreateName`: we only
/// use it to accelerate lookup inside our own `assetcatalog.bin`. Lookup still
/// verifies the canonical path to handle the theoretical hash-collision case.
///
/// Taking [`AssetTreePath`] rather than `&str` is what makes the key stable:
/// the type already guarantees the canonical spelling, so hashing never has to
/// re-normalize and cannot be handed a raw path by mistake.
fn asset_catalog_path_hash(path: &AssetTreePath) -> [u8; 16] {
    let digest = blake3::hash(path.as_str().as_bytes());
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

/// A per-product runtime dependency edge.
///
/// Lumberyard's asset registry carries a product-dependency list per product so
/// the runtime can preload/resolve referenced products by identity. The native
/// catalog carries the same information: the target product [`AssetId`], its
/// native asset type, and an optional diagnostic hint. Runtime resolution uses
/// the [`AssetId`] only; the hint is never a resolver.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProductDependency {
    /// Target product identity.
    pub id: AssetId,
    /// Target product native asset type.
    pub asset_type: Uuid,
    /// Optional diagnostic `@assets@/...` hint; never a resolver.
    pub hint: Option<String>,
}

/// Whether a product claims its physical path in catalog path lookup.
///
/// Every entry remains a real [`AssetId`] and retains its physical product
/// path. `AssetIdOnly` is used when several identities share those same bytes:
/// exactly one entry may own by-path lookup, while the other entries remain
/// directly addressable by id without creating aliases or redirecting ids.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[repr(u8)]
pub enum AssetCatalogPathRegistration {
    #[default]
    Registered = 0,
    AssetIdOnly = 1,
}

impl AssetCatalogPathRegistration {
    #[must_use]
    pub const fn is_registered(self) -> bool {
        matches!(self, Self::Registered)
    }

    const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Registered),
            1 => Some(Self::AssetIdOnly),
            _ => None,
        }
    }
}

impl ProductDependency {
    /// Create a product dependency edge to `id` of `asset_type`.
    #[inline]
    #[must_use]
    pub const fn new(id: AssetId, asset_type: Uuid) -> Self {
        Self {
            id,
            asset_type,
            hint: None,
        }
    }

    /// Attach a diagnostic `@assets@/...` hint.
    #[inline]
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// One asset catalog entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetCatalogEntry {
    pub asset_id: AssetId,
    pub asset_type: Uuid,
    pub product_format: String,
    pub product_format_version: u32,
    /// Canonical authoring address in the virtual `@assets@` namespace.
    ///
    /// This is diagnostic metadata only. Runtime lookup remains keyed by
    /// [`AssetId`].
    pub source_path: AssetTreePath,
    pub path: AssetTreePath,
    /// Controls whether `path` and `catalog_aliases` participate in by-path
    /// lookup. The physical path is retained for payload access in both modes.
    #[serde(default)]
    pub path_registration: AssetCatalogPathRegistration,
    /// Additional lookup paths that resolve to this same product entry.
    #[serde(default)]
    pub catalog_aliases: Vec<AssetTreePath>,
    pub schema_version: Option<u32>,
    pub byte_len: u64,
    pub content_hash: [u8; PACKAGE_CONTENT_HASH_BYTES],
    /// Runtime product dependencies for this product, in deterministic order.
    #[serde(default)]
    pub dependencies: Vec<ProductDependency>,
}

impl AssetCatalogEntry {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        asset_id: AssetId,
        asset_type: Uuid,
        product_format: impl Into<String>,
        product_format_version: u32,
        path: impl Into<AssetTreePath>,
        schema_version: Option<u32>,
        byte_len: u64,
        content_hash: [u8; PACKAGE_CONTENT_HASH_BYTES],
    ) -> Self {
        let path = path.into();
        Self {
            asset_id,
            asset_type,
            product_format: product_format.into(),
            product_format_version,
            source_path: path.clone(),
            path,
            path_registration: AssetCatalogPathRegistration::Registered,
            catalog_aliases: Vec::new(),
            schema_version,
            byte_len,
            content_hash,
            dependencies: Vec::new(),
        }
    }

    /// Attach the canonical virtual source address for this product.
    #[must_use]
    pub fn with_source_path(mut self, source_path: impl Into<AssetTreePath>) -> Self {
        self.source_path = source_path.into();
        self
    }

    /// Attach additional lookup paths for this same product entry.
    #[must_use]
    pub fn with_catalog_aliases(
        mut self,
        aliases: impl IntoIterator<Item = impl Into<AssetTreePath>>,
    ) -> Self {
        self.catalog_aliases = aliases.into_iter().map(Into::into).collect();
        self.catalog_aliases.sort();
        self
    }

    /// Select how this real product identity participates in path lookup.
    #[must_use]
    pub const fn with_path_registration(
        mut self,
        registration: AssetCatalogPathRegistration,
    ) -> Self {
        self.path_registration = registration;
        self
    }

    /// Attach this product's runtime dependency list.
    ///
    /// The list is normalized to a deterministic order (by target id, then
    /// asset type, then hint) so identical inputs always serialize identically.
    #[must_use]
    pub fn with_dependencies(mut self, mut dependencies: Vec<ProductDependency>) -> Self {
        dependencies.sort();
        self.dependencies = dependencies;
        self
    }
}

/// Explicit compatibility redirect from a legacy asset id to the real catalog id.
///
/// Mirrors Lumberyard's `m_legacyAssetIdToRealAssetId` behavior: runtime lookup
/// checks the real `AssetId` table first, then follows this redirect table. It
/// is not a fuzzy `guid + type` alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AssetCatalogLegacyRedirect {
    pub legacy: AssetId,
    pub real: AssetId,
}

impl AssetCatalogLegacyRedirect {
    #[inline]
    #[must_use]
    pub const fn new(legacy: AssetId, real: AssetId) -> Self {
        Self { legacy, real }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssetCatalogOptimizedIndex {
    id: Vec<OptimizedIdRow>,
    path_hash: Vec<OptimizedPathRow>,
    legacy: Vec<OptimizedLegacyRow>,
}

impl AssetCatalogOptimizedIndex {
    fn entry_index_by_id(&self, id: AssetId) -> Option<usize> {
        self.id
            .binary_search_by_key(&id, |row| row.asset_id)
            .ok()
            .map(|position| self.id[position].entry_index as usize)
    }

    fn entry_index_by_path(&self, entries: &[AssetCatalogEntry], path: &str) -> Option<usize> {
        let normalized = AssetTreePath::new(path);
        let hash = asset_catalog_path_hash(&normalized);
        let mut position = self
            .path_hash
            .binary_search_by_key(&hash, |row| row.path_hash)
            .ok()?;
        while position > 0 && self.path_hash[position - 1].path_hash == hash {
            position -= 1;
        }
        for row in self.path_hash[position..]
            .iter()
            .take_while(|row| row.path_hash == hash)
        {
            let index = row.entry_index as usize;
            if entries.get(index).is_some_and(|entry| {
                entry.path == normalized || entry.catalog_aliases.binary_search(&normalized).is_ok()
            }) {
                return Some(index);
            }
        }
        None
    }

    fn legacy_target_entry_index(&self, legacy: AssetId) -> Option<usize> {
        self.legacy
            .binary_search_by_key(&legacy, |row| row.legacy)
            .ok()
            .map(|position| self.legacy[position].real_entry_index as usize)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OptimizedIdRow {
    asset_id: AssetId,
    entry_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OptimizedPathRow {
    path_hash: [u8; 16],
    entry_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OptimizedLegacyRow {
    legacy: AssetId,
    real_entry_index: u32,
}

/// Native asset catalog saved into an asset tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetCatalog {
    pub version: u32,
    pub entries: Vec<AssetCatalogEntry>,
    #[serde(default)]
    pub legacy_redirects: Vec<AssetCatalogLegacyRedirect>,
}

impl AssetCatalog {
    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] when `entries` fails catalog validation.
    pub fn new(entries: Vec<AssetCatalogEntry>) -> Result<Self, AssetCatalogError> {
        Self::with_legacy_redirects(entries, Vec::new())
    }

    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] when `entries` or `legacy_redirects`
    /// fails catalog validation.
    pub fn with_legacy_redirects(
        mut entries: Vec<AssetCatalogEntry>,
        mut legacy_redirects: Vec<AssetCatalogLegacyRedirect>,
    ) -> Result<Self, AssetCatalogError> {
        entries.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.asset_id.cmp(&right.asset_id))
                .then(left.asset_type.cmp(&right.asset_type))
        });
        legacy_redirects.sort_unstable();
        let catalog = Self {
            version: ASSET_CATALOG_VERSION,
            entries,
            legacy_redirects,
        };
        catalog.validate()?;
        Ok(catalog)
    }

    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] when the entries derived from
    /// `manifest` fail catalog validation.
    pub fn from_package_manifest(manifest: &PackageManifest) -> Result<Self, AssetCatalogError> {
        let entries = manifest
            .entries()
            .iter()
            .map(|entry| {
                let path = entry.product_path.clone();
                AssetCatalogEntry::new(
                    AssetId::new(entry.source_asset_guid, entry.sub_id),
                    entry.asset_type,
                    entry.product_format.clone(),
                    entry.product_format_version,
                    path,
                    None,
                    entry.byte_len,
                    entry.content_hash,
                )
                .with_source_path(format!("@assets@/{}", entry.source_path.as_str()))
                .with_path_registration(entry.path_registration)
                .with_catalog_aliases(entry.catalog_aliases.clone())
                .with_dependencies(entry.dependencies.clone())
            })
            .collect::<Vec<_>>();
        Self::new(entries)
    }

    #[inline]
    #[must_use]
    pub fn entries(&self) -> &[AssetCatalogEntry] {
        &self.entries
    }

    #[inline]
    #[must_use]
    pub fn legacy_redirects(&self) -> &[AssetCatalogLegacyRedirect] {
        &self.legacy_redirects
    }

    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] when the version is unsupported, or any
    /// entry or legacy redirect fails the catalog invariants.
    pub fn validate(&self) -> Result<(), AssetCatalogError> {
        if self.version != ASSET_CATALOG_VERSION {
            return Err(AssetCatalogError::UnsupportedVersion {
                version: self.version,
                expected: ASSET_CATALOG_VERSION,
            });
        }
        validate_entries(&self.entries)?;
        validate_legacy_redirects(&self.entries, &self.legacy_redirects)
    }

    /// Read the streamable canonical catalog section.
    ///
    /// This stops after the canonical records and ignores any optimized lookup
    /// tail appended to the file.
    ///
    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] when the stream is malformed or the
    /// decoded catalog fails validation.
    pub fn read(reader: impl Read) -> Result<Self, AssetCatalogError> {
        read_asset_catalog(reader)
    }

    /// Write the canonical catalog and its in-file lookup tail.
    ///
    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] when validation fails or the writer
    /// returns an I/O error.
    pub fn write(&self, writer: impl Write) -> Result<(), AssetCatalogError> {
        write_asset_catalog(self, writer)
    }

    /// Prepare this catalog for repeated path/id lookup.
    #[must_use]
    pub fn prepare(self) -> PreparedAssetCatalog {
        PreparedAssetCatalog::from_catalog(self)
    }

    /// Read the complete catalog file and prepare it for repeated lookup.
    ///
    /// If the file has an optimized lookup tail, that tail is validated and
    /// used. Tail-less files are still supported by materializing equivalent
    /// lookup maps from the canonical stream.
    ///
    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] when the stream is malformed, the
    /// decoded catalog fails validation, or an optimized tail fails to parse
    /// or validate against the canonical catalog.
    pub fn read_prepared(reader: impl Read) -> Result<PreparedAssetCatalog, AssetCatalogError> {
        read_prepared_asset_catalog(reader)
    }
}

/// Asset catalog prepared for repeated runtime lookup.
///
/// This owns the canonical [`AssetCatalog`] and an internal lookup
/// representation. Callers do not need to know whether lookup is backed by the
/// fixed in-file tail or materialized from a stream-only catalog.
#[derive(Debug, Clone)]
pub struct PreparedAssetCatalog {
    catalog: AssetCatalog,
    lookup: CatalogLookupIndex,
}

impl PreparedAssetCatalog {
    /// Prepare an already-read canonical catalog for lookup.
    #[must_use]
    pub fn from_catalog(catalog: AssetCatalog) -> Self {
        Self::from_parts(catalog, None)
    }

    fn from_parts(
        catalog: AssetCatalog,
        optimized_index: Option<AssetCatalogOptimizedIndex>,
    ) -> Self {
        let lookup = optimized_index.map_or_else(
            || CatalogLookupIndex::materialized(&catalog),
            CatalogLookupIndex::Optimized,
        );
        Self { catalog, lookup }
    }

    #[inline]
    #[must_use]
    pub const fn catalog(&self) -> &AssetCatalog {
        &self.catalog
    }

    #[inline]
    #[must_use]
    pub fn into_catalog(self) -> AssetCatalog {
        self.catalog
    }

    #[inline]
    #[must_use]
    pub fn entries(&self) -> &[AssetCatalogEntry] {
        self.catalog.entries()
    }

    #[inline]
    #[must_use]
    pub fn legacy_redirects(&self) -> &[AssetCatalogLegacyRedirect] {
        self.catalog.legacy_redirects()
    }

    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }

    /// Look up a catalog entry by content path.
    ///
    /// `path` is folded to its [`AssetTreePath`] canonical form here, so any
    /// spelling of the path resolves and callers must never pre-normalize.
    /// Lookup always validates against the canonical path string, including
    /// when an optimized hash table is available.
    #[must_use]
    pub fn entry_by_path(&self, path: &str) -> Option<&AssetCatalogEntry> {
        self.lookup
            .entry_index_by_path(self.catalog.entries(), path)
            .and_then(|position| self.catalog.entries().get(position))
    }

    /// Look up a catalog entry by real or legacy [`AssetId`].
    ///
    /// Real ids are checked first. Legacy ids only resolve through explicit
    /// catalog redirects, matching Lumberyard's catalog behavior.
    #[must_use]
    pub fn entry_by_id(&self, asset_id: AssetId) -> Option<&AssetCatalogEntry> {
        self.lookup
            .entry_index_by_id(self.catalog.entries(), asset_id)
            .and_then(|position| self.catalog.entries().get(position))
    }

    /// Look up a product [`AssetId`] by content path.
    ///
    /// Folds `path` exactly as [`Self::entry_by_path`] does.
    #[inline]
    #[must_use]
    pub fn asset_id_for_path(&self, path: &str) -> Option<AssetId> {
        self.entry_by_path(path).map(|entry| entry.asset_id)
    }
}

impl From<AssetCatalog> for PreparedAssetCatalog {
    fn from(catalog: AssetCatalog) -> Self {
        Self::from_catalog(catalog)
    }
}

#[derive(Debug, Clone)]
enum CatalogLookupIndex {
    Optimized(AssetCatalogOptimizedIndex),
    Materialized {
        by_path: FxHashMap<AssetTreePath, usize>,
        by_id: FxHashMap<AssetId, usize>,
        by_legacy: FxHashMap<AssetId, AssetId>,
    },
}

impl CatalogLookupIndex {
    fn materialized(catalog: &AssetCatalog) -> Self {
        let path_count = catalog
            .entries
            .iter()
            .filter(|entry| entry.path_registration.is_registered())
            .map(|entry| entry.catalog_aliases.len() + 1)
            .sum();
        let mut by_path = FxHashMap::with_capacity_and_hasher(path_count, FxBuildHasher);
        let mut by_id = FxHashMap::with_capacity_and_hasher(catalog.entries.len(), FxBuildHasher);
        let mut by_legacy =
            FxHashMap::with_capacity_and_hasher(catalog.legacy_redirects.len(), FxBuildHasher);

        for (position, entry) in catalog.entries.iter().enumerate() {
            if entry.path_registration.is_registered() {
                by_path.insert(entry.path.clone(), position);
                for alias in &entry.catalog_aliases {
                    by_path.insert(alias.clone(), position);
                }
            }
            by_id.insert(entry.asset_id, position);
        }
        for redirect in &catalog.legacy_redirects {
            by_legacy.insert(redirect.legacy, redirect.real);
        }

        Self::Materialized {
            by_path,
            by_id,
            by_legacy,
        }
    }

    fn entry_index_by_path(&self, entries: &[AssetCatalogEntry], path: &str) -> Option<usize> {
        match self {
            Self::Optimized(index) => index.entry_index_by_path(entries, path),
            Self::Materialized { by_path, .. } => {
                let path = AssetTreePath::new(path);
                by_path.get(&path).copied()
            }
        }
    }

    fn entry_index_by_id(&self, entries: &[AssetCatalogEntry], asset_id: AssetId) -> Option<usize> {
        match self {
            Self::Optimized(index) => index
                .entry_index_by_id(asset_id)
                .or_else(|| index.legacy_target_entry_index(asset_id)),
            Self::Materialized {
                by_id, by_legacy, ..
            } => {
                if let Some(position) = by_id.get(&asset_id).copied() {
                    return Some(position);
                }
                let real = by_legacy.get(&asset_id)?;
                by_id.get(real).copied()
            }
        }
        .filter(|position| entries.get(*position).is_some())
    }
}

/// Write an asset catalog.
///
/// The format is intentionally streamable: once the small header is read,
/// callers can consume each length-delimited entry and redirect in order.
///
/// # Errors
///
/// Returns [`AssetCatalogError`] when `catalog` fails validation, an entry
/// field exceeds its wire-format limits, or the writer returns an I/O error.
pub fn write_asset_catalog(
    catalog: &AssetCatalog,
    writer: impl Write,
) -> Result<(), AssetCatalogError> {
    catalog.validate()?;
    let mut writer = CountingWriter::new(writer);

    let entry_count =
        u32::try_from(catalog.entries.len()).map_err(|_| AssetCatalogError::TooManyEntries {
            count: catalog.entries.len(),
        })?;
    let legacy_redirect_count = u32::try_from(catalog.legacy_redirects.len()).map_err(|_| {
        AssetCatalogError::TooManyLegacyRedirects {
            count: catalog.legacy_redirects.len(),
        }
    })?;

    writer.write_all(ASSET_CATALOG_MAGIC)?;
    write_u32(&mut writer, ASSET_CATALOG_VERSION)?;
    write_u32(&mut writer, entry_count)?;
    write_u32(&mut writer, legacy_redirect_count)?;

    for entry in &catalog.entries {
        write_stream_entry(&mut writer, entry)?;
    }

    for redirect in &catalog.legacy_redirects {
        write_uuid(&mut writer, redirect.legacy.guid)?;
        write_u32(&mut writer, redirect.legacy.sub_id)?;
        write_uuid(&mut writer, redirect.real.guid)?;
        write_u32(&mut writer, redirect.real.sub_id)?;
    }

    write_optimized_index(catalog, &mut writer)?;
    Ok(())
}

/// Receipt returned after a streaming catalog encoder has finalized its tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetCatalogWriteReceipt {
    /// Number of canonical product entries emitted.
    pub entry_count: u32,
    /// Complete encoded length, including the optimized lookup tail.
    pub byte_count: u64,
}

/// Typestate for an [`AssetCatalogStreamEncoder`] that knows its entry count.
///
/// This is inferred by [`AssetCatalogStreamEncoder::new`]. Callers normally do
/// not need to name it.
#[derive(Debug, Clone, Copy)]
pub struct KnownEntryCount {
    expected: u32,
}

/// Typestate for an [`AssetCatalogStreamEncoder`] that patches its entry count
/// into a seekable header when finalized.
///
/// This is inferred by [`AssetCatalogStreamEncoder::new_unknown_count`].
/// Callers normally do not need to name it.
#[derive(Debug, Clone, Copy)]
pub struct UnknownEntryCount {
    header_entry_count_offset: u64,
}

/// Incremental native catalog encoder.
///
/// [`Self::new`] writes a catalog with a known entry count to any [`Write`]
/// sink. [`Self::new_unknown_count`] writes a placeholder count to a
/// [`Write`] + [`Seek`] sink, streams entries once, then patches the count at
/// [`Self::finish`]. Both modes retain only validation state and compact
/// lookup-tail rows, never the full entry collection or complete byte stream.
/// Use an atomic temporary-file writer when an error must not leave a partial
/// destination visible.
pub struct AssetCatalogStreamEncoder<W, EntryCount = KnownEntryCount> {
    writer: CountingWriter<W>,
    entry_count: EntryCount,
    written_entry_count: u32,
    redirects: Vec<AssetCatalogLegacyRedirect>,
    index: AssetCatalogOptimizedIndex,
    catalog_paths: BTreeSet<AssetTreePath>,
    asset_ids: BTreeSet<AssetId>,
    previous: Option<StreamEntryState>,
}

impl<W: Write> AssetCatalogStreamEncoder<W, KnownEntryCount> {
    /// Begin an incremental catalog with a known entry count and redirects.
    ///
    /// Redirect order is preserved exactly; callers that require canonical
    /// redirect order should provide the sorted order from
    /// [`AssetCatalog::legacy_redirects`].
    ///
    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] when either count cannot fit the wire
    /// format or the writer cannot accept the catalog header.
    pub fn new(
        writer: W,
        entry_count: usize,
        redirects: Vec<AssetCatalogLegacyRedirect>,
    ) -> Result<Self, AssetCatalogError> {
        let expected_entry_count = u32::try_from(entry_count)
            .map_err(|_| AssetCatalogError::TooManyEntries { count: entry_count })?;
        let redirect_count = u32::try_from(redirects.len()).map_err(|_| {
            AssetCatalogError::TooManyLegacyRedirects {
                count: redirects.len(),
            }
        })?;
        let mut writer = CountingWriter::new(writer);
        writer.write_all(ASSET_CATALOG_MAGIC)?;
        write_u32(&mut writer, ASSET_CATALOG_VERSION)?;
        write_u32(&mut writer, expected_entry_count)?;
        write_u32(&mut writer, redirect_count)?;

        Ok(Self {
            writer,
            entry_count: KnownEntryCount {
                expected: expected_entry_count,
            },
            written_entry_count: 0,
            redirects,
            index: AssetCatalogOptimizedIndex {
                id: Vec::with_capacity(entry_count),
                path_hash: Vec::new(),
                legacy: Vec::new(),
            },
            catalog_paths: BTreeSet::new(),
            asset_ids: BTreeSet::new(),
            previous: None,
        })
    }

    /// Append one already ordered canonical entry.
    ///
    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] if the entry breaks catalog invariants,
    /// is out of order, exceeds the declared count, or cannot be written.
    pub fn push(&mut self, entry: &AssetCatalogEntry) -> Result<(), AssetCatalogError> {
        if self.written_entry_count == self.entry_count.expected {
            return Err(AssetCatalogError::StreamEntryCountExceeded {
                expected: self.entry_count.expected,
            });
        }
        self.push_entry(entry)
    }

    /// Finalize redirects and the optimized lookup tail, returning the sink.
    ///
    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] if the declared entry count was not met,
    /// redirects are invalid, or the tail cannot be written.
    pub fn finish(mut self) -> Result<(W, AssetCatalogWriteReceipt), AssetCatalogError> {
        if self.written_entry_count != self.entry_count.expected {
            return Err(AssetCatalogError::StreamEntryCountMismatch {
                expected: self.entry_count.expected,
                actual: self.written_entry_count,
            });
        }
        let receipt = self.finish_tail()?;
        Ok((self.writer.into_inner(), receipt))
    }
}

impl<W: Write + Seek> AssetCatalogStreamEncoder<W, UnknownEntryCount> {
    /// Begin an incremental catalog without knowing its entry count.
    ///
    /// The encoder writes a zero placeholder in the header, streams each
    /// entry once, then patches the exact count during [`Self::finish`]. The
    /// returned sink is restored to its final write position after that patch.
    /// Redirect order is preserved exactly; callers that require canonical
    /// redirect order should provide the sorted order from
    /// [`AssetCatalog::legacy_redirects`].
    ///
    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] when the redirect count cannot fit the
    /// wire format, or the seekable writer cannot accept the header.
    pub fn new_unknown_count(
        writer: W,
        redirects: Vec<AssetCatalogLegacyRedirect>,
    ) -> Result<Self, AssetCatalogError> {
        let redirect_count = u32::try_from(redirects.len()).map_err(|_| {
            AssetCatalogError::TooManyLegacyRedirects {
                count: redirects.len(),
            }
        })?;
        let mut writer = CountingWriter::new(writer);
        writer.write_all(ASSET_CATALOG_MAGIC)?;
        write_u32(&mut writer, ASSET_CATALOG_VERSION)?;
        let header_entry_count_offset = writer.inner.stream_position()?;
        write_u32(&mut writer, 0)?;
        write_u32(&mut writer, redirect_count)?;

        Ok(Self {
            writer,
            entry_count: UnknownEntryCount {
                header_entry_count_offset,
            },
            written_entry_count: 0,
            redirects,
            index: AssetCatalogOptimizedIndex {
                id: Vec::new(),
                path_hash: Vec::new(),
                legacy: Vec::new(),
            },
            catalog_paths: BTreeSet::new(),
            asset_ids: BTreeSet::new(),
            previous: None,
        })
    }

    /// Append one already ordered canonical entry.
    ///
    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] if the entry breaks catalog invariants,
    /// is out of order, exceeds the wire-format count, or cannot be written.
    pub fn push(&mut self, entry: &AssetCatalogEntry) -> Result<(), AssetCatalogError> {
        self.push_entry(entry)
    }

    /// Finalize redirects and the optimized lookup tail, patch the header
    /// count, and return the sink at its final write position.
    ///
    /// # Errors
    ///
    /// Returns [`AssetCatalogError`] if entries or redirects break catalog
    /// invariants, the tail cannot be written, or the sink cannot seek to
    /// patch and restore the header position.
    pub fn finish(mut self) -> Result<(W, AssetCatalogWriteReceipt), AssetCatalogError> {
        let receipt = self.finish_tail()?;
        self.patch_entry_count()?;
        Ok((self.writer.into_inner(), receipt))
    }
}

impl<W: Write, EntryCount> AssetCatalogStreamEncoder<W, EntryCount> {
    fn push_entry(&mut self, entry: &AssetCatalogEntry) -> Result<(), AssetCatalogError> {
        if self.written_entry_count == u32::MAX {
            return Err(AssetCatalogError::TooManyEntries {
                count: usize::try_from(u64::from(u32::MAX) + 1).unwrap_or(usize::MAX),
            });
        }
        validate_stream_entry(
            entry,
            &self.catalog_paths,
            &self.asset_ids,
            self.previous.as_ref(),
        )?;

        let entry_index = self.written_entry_count;
        write_stream_entry(&mut self.writer, entry)?;
        self.asset_ids.insert(entry.asset_id);
        if entry.path_registration.is_registered() {
            self.catalog_paths.insert(entry.path.clone());
            self.catalog_paths
                .extend(entry.catalog_aliases.iter().cloned());
        }
        self.index.id.push(OptimizedIdRow {
            asset_id: entry.asset_id,
            entry_index,
        });
        if entry.path_registration.is_registered() {
            self.index.path_hash.push(OptimizedPathRow {
                path_hash: asset_catalog_path_hash(&entry.path),
                entry_index,
            });
            self.index
                .path_hash
                .extend(entry.catalog_aliases.iter().map(|alias| OptimizedPathRow {
                    path_hash: asset_catalog_path_hash(alias),
                    entry_index,
                }));
        }
        self.previous = Some(StreamEntryState::from(entry));
        self.written_entry_count += 1;
        Ok(())
    }

    fn finish_tail(&mut self) -> Result<AssetCatalogWriteReceipt, AssetCatalogError> {
        validate_legacy_redirects_for_asset_ids(&self.asset_ids, &self.redirects)?;
        for redirect in &self.redirects {
            write_uuid(&mut self.writer, redirect.legacy.guid)?;
            write_u32(&mut self.writer, redirect.legacy.sub_id)?;
            write_uuid(&mut self.writer, redirect.real.guid)?;
            write_u32(&mut self.writer, redirect.real.sub_id)?;
        }
        self.index.id.sort_by_key(|row| row.asset_id);
        self.index.legacy = self
            .redirects
            .iter()
            .map(|redirect| {
                let real_entry_index = self
                    .index
                    .id
                    .binary_search_by_key(&redirect.real, |row| row.asset_id)
                    .map_err(|_| AssetCatalogError::MissingLegacyRedirectTarget {
                        real: redirect.real,
                    })?;
                Ok(OptimizedLegacyRow {
                    legacy: redirect.legacy,
                    real_entry_index: self.index.id[real_entry_index].entry_index,
                })
            })
            .collect::<Result<_, AssetCatalogError>>()?;
        self.index
            .path_hash
            .sort_by_key(|row| (row.path_hash, row.entry_index));
        self.index.legacy.sort_by_key(|row| row.legacy);
        write_optimized_index_rows(&self.index, &mut self.writer)?;
        let receipt = AssetCatalogWriteReceipt {
            entry_count: self.written_entry_count,
            byte_count: self.writer.bytes_written(),
        };
        Ok(receipt)
    }
}

impl<W: Write + Seek> AssetCatalogStreamEncoder<W, UnknownEntryCount> {
    fn patch_entry_count(&mut self) -> Result<(), AssetCatalogError> {
        let end_position = self.writer.inner.stream_position()?;
        self.writer
            .inner
            .seek(SeekFrom::Start(self.entry_count.header_entry_count_offset))?;
        let patch = write_u32(&mut self.writer.inner, self.written_entry_count);
        let restore = self.writer.inner.seek(SeekFrom::Start(end_position));
        patch?;
        restore?;
        Ok(())
    }
}

#[derive(Clone)]
struct StreamEntryState {
    path: AssetTreePath,
    asset_id: AssetId,
    asset_type: Uuid,
    product_format: String,
    product_format_version: u32,
    schema_version: Option<u32>,
    byte_len: u64,
    content_hash: [u8; PACKAGE_CONTENT_HASH_BYTES],
    dependencies: Vec<ProductDependency>,
}

impl From<&AssetCatalogEntry> for StreamEntryState {
    fn from(entry: &AssetCatalogEntry) -> Self {
        Self {
            path: entry.path.clone(),
            asset_id: entry.asset_id,
            asset_type: entry.asset_type,
            product_format: entry.product_format.clone(),
            product_format_version: entry.product_format_version,
            schema_version: entry.schema_version,
            byte_len: entry.byte_len,
            content_hash: entry.content_hash,
            dependencies: entry.dependencies.clone(),
        }
    }
}

/// Read an asset catalog.
///
/// # Errors
///
/// Returns [`AssetCatalogError`] when the stream has a bad magic, an
/// unsupported version, malformed records, or the decoded catalog fails
/// validation.
pub fn read_asset_catalog(mut reader: impl Read) -> Result<AssetCatalog, AssetCatalogError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != ASSET_CATALOG_MAGIC {
        return Err(AssetCatalogError::BadMagic { found: magic });
    }

    let version = read_u32(&mut reader)?;
    if version != ASSET_CATALOG_VERSION {
        return Err(AssetCatalogError::UnsupportedVersion {
            version,
            expected: ASSET_CATALOG_VERSION,
        });
    }

    let entry_count = read_u32(&mut reader)? as usize;
    let legacy_redirect_count = read_u32(&mut reader)? as usize;
    let mut entries = Vec::with_capacity(entry_count);
    for index in 0..entry_count {
        entries.push(read_stream_entry(&mut reader, index)?);
    }

    let mut legacy_redirects = Vec::with_capacity(legacy_redirect_count);
    for _ in 0..legacy_redirect_count {
        let legacy_guid = read_uuid(&mut reader)?;
        let legacy_sub_id = read_u32(&mut reader)?;
        let real_guid = read_uuid(&mut reader)?;
        let real_sub_id = read_u32(&mut reader)?;
        legacy_redirects.push(AssetCatalogLegacyRedirect::new(
            AssetId::new(legacy_guid, legacy_sub_id),
            AssetId::new(real_guid, real_sub_id),
        ));
    }

    let catalog = AssetCatalog {
        version,
        entries,
        legacy_redirects,
    };
    catalog.validate()?;
    Ok(catalog)
}

/// Read a complete catalog file and prepare it for repeated lookup.
///
/// Use [`read_asset_catalog`] when the caller needs pure streaming behavior.
/// Use this for runtime/package loading when random-access lookup matters.
///
/// # Errors
///
/// Returns [`AssetCatalogError`] when the canonical stream fails to parse or
/// validate, or an optimized tail is present but fails to parse or validate
/// against the canonical catalog.
pub fn read_prepared_asset_catalog(
    mut reader: impl Read,
) -> Result<PreparedAssetCatalog, AssetCatalogError> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    let mut cursor = Cursor::new(bytes.as_slice());
    let catalog = read_asset_catalog(&mut cursor)?;
    // `cursor` reads from `bytes.as_slice()`, so its position can never
    // exceed `bytes.len()` (a `usize`); the round-trip through `u64` cannot
    // truncate.
    #[allow(clippy::cast_possible_truncation)]
    let canonical_len = cursor.position() as usize;
    let optimized_index = read_optimized_index_tail(&bytes, canonical_len, &catalog)?;
    Ok(PreparedAssetCatalog::from_parts(catalog, optimized_index))
}

/// Asset catalog errors.
#[derive(Debug, Error)]
pub enum AssetCatalogError {
    #[error("bad asset catalog magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported asset catalog version {version}, expected {expected}")]
    UnsupportedVersion { version: u32, expected: u32 },
    #[error("bad optimized catalog footer magic: {found:?}")]
    BadOptimizedFooterMagic { found: [u8; 8] },
    #[error("bad optimized catalog magic: {found:?}")]
    BadOptimizedMagic { found: [u8; 8] },
    #[error("unsupported optimized catalog version {version}, expected {expected}")]
    UnsupportedOptimizedVersion { version: u32, expected: u32 },
    #[error(
        "optimized catalog index offset {offset} length {len} is outside file length {file_len}"
    )]
    OptimizedIndexOutOfBounds {
        offset: u64,
        len: u64,
        file_len: usize,
    },
    #[error("optimized catalog index size mismatch: expected {expected} bytes, got {actual}")]
    OptimizedIndexSizeMismatch { expected: usize, actual: usize },
    #[error(
        "optimized catalog {table} row points at entry {entry_index}, but catalog has {entry_count} entries"
    )]
    OptimizedIndexEntryOutOfBounds {
        table: &'static str,
        entry_index: u32,
        entry_count: usize,
    },
    #[error("optimized catalog {table} row {entry_index} does not match the canonical stream")]
    OptimizedIndexMismatch {
        table: &'static str,
        entry_index: u32,
    },
    #[error("optimized catalog {table} table is not sorted")]
    OptimizedIndexUnsorted { table: &'static str },
    #[error("entry {index} {field} is invalid UTF-8: {source}")]
    InvalidUtf8 {
        index: usize,
        field: &'static str,
        source: std::str::Utf8Error,
    },
    #[error("product asset `{path}` has nil asset id")]
    NilAssetId { path: String },
    #[error("product asset `{path}` has nil asset type")]
    NilAssetType { path: String },
    #[error("product asset `{path}` has empty product format")]
    EmptyProductFormat { path: String },
    #[error("product asset `{path}` has invalid product format version {version}")]
    InvalidProductFormatVersion { path: String, version: u32 },
    #[error("duplicate asset catalog path `{path}`")]
    DuplicateProductPath { path: String },
    #[error("asset-id-only product `{path}` ({asset_id}) cannot declare catalog aliases")]
    AssetIdOnlyWithAliases { path: String, asset_id: AssetId },
    #[error(
        "products {first_asset_id} and {asset_id} share physical path `{path}` but disagree on its backing product contract"
    )]
    SharedProductBackingMismatch {
        path: String,
        first_asset_id: AssetId,
        asset_id: AssetId,
    },
    #[error("entry {index} has unknown catalog path-registration value {value}")]
    InvalidPathRegistration { index: usize, value: u32 },
    #[error("duplicate asset catalog asset id {asset_id}")]
    DuplicateAssetId { asset_id: AssetId },
    #[error("streamed catalog entries are not in canonical order: `{previous}` before `{current}`")]
    StreamEntriesOutOfOrder { previous: String, current: String },
    #[error("streamed catalog declared {expected} entries but received {actual}")]
    StreamEntryCountMismatch { expected: u32, actual: u32 },
    #[error("streamed catalog declared {expected} entries and cannot accept another")]
    StreamEntryCountExceeded { expected: u32 },
    #[error("legacy redirect has nil legacy asset id")]
    NilLegacyAssetId,
    #[error("legacy redirect for {legacy} has nil real asset id")]
    NilLegacyRedirectTarget { legacy: AssetId },
    #[error("legacy redirect {legacy} points to itself")]
    SelfLegacyRedirect { legacy: AssetId },
    #[error("legacy redirect duplicates catalog asset id {legacy}")]
    LegacyRedirectShadowsCatalogAssetId { legacy: AssetId },
    #[error("legacy redirect target {real} is not present in the catalog")]
    MissingLegacyRedirectTarget { real: AssetId },
    #[error("duplicate legacy redirect for {legacy}")]
    DuplicateLegacyRedirect { legacy: AssetId },
    #[error("{field} `{path}` must be an asset-tree relative path")]
    InvalidRelativePath { field: &'static str, path: String },
    #[error("asset catalog has too many entries: {count}")]
    TooManyEntries { count: usize },
    #[error("product asset `{path}` has too many product dependencies: {count}")]
    TooManyDependencies { path: String, count: usize },
    #[error("product asset `{path}` has too many catalog aliases: {count}")]
    TooManyCatalogAliases { path: String, count: usize },
    #[error("asset catalog has too many legacy redirects: {count}")]
    TooManyLegacyRedirects { count: usize },
    #[error("asset catalog path is too long: {path} ({byte_len} bytes)")]
    PathTooLong { path: String, byte_len: usize },
    #[error("{field} is too long: {byte_len} bytes")]
    TextTooLong {
        field: &'static str,
        byte_len: usize,
    },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

fn validate_entries(entries: &[AssetCatalogEntry]) -> Result<(), AssetCatalogError> {
    let mut catalog_paths = BTreeSet::new();
    let mut physical_paths = BTreeMap::<&AssetTreePath, &AssetCatalogEntry>::new();
    let mut asset_ids = BTreeSet::new();
    for entry in entries {
        validate_entry(entry)?;
        if entry.path_registration.is_registered() {
            if !catalog_paths.insert(entry.path.as_str().to_string()) {
                return Err(AssetCatalogError::DuplicateProductPath {
                    path: entry.path.to_string(),
                });
            }
            for alias in &entry.catalog_aliases {
                if !catalog_paths.insert(alias.as_str().to_string()) {
                    return Err(AssetCatalogError::DuplicateProductPath {
                        path: alias.to_string(),
                    });
                }
            }
        }
        if let Some(first) = physical_paths.get(&entry.path) {
            if !same_product_backing(first, entry) {
                return Err(AssetCatalogError::SharedProductBackingMismatch {
                    path: entry.path.to_string(),
                    first_asset_id: first.asset_id,
                    asset_id: entry.asset_id,
                });
            }
        } else {
            physical_paths.insert(&entry.path, entry);
        }
        if !asset_ids.insert(entry.asset_id) {
            return Err(AssetCatalogError::DuplicateAssetId {
                asset_id: entry.asset_id,
            });
        }
    }
    Ok(())
}

fn validate_stream_entry(
    entry: &AssetCatalogEntry,
    catalog_paths: &BTreeSet<AssetTreePath>,
    asset_ids: &BTreeSet<AssetId>,
    previous: Option<&StreamEntryState>,
) -> Result<(), AssetCatalogError> {
    validate_entry(entry)?;
    if let Some(previous) = previous {
        let previous_key = (&previous.path, previous.asset_id, previous.asset_type);
        let current_key = (&entry.path, entry.asset_id, entry.asset_type);
        if current_key < previous_key {
            return Err(AssetCatalogError::StreamEntriesOutOfOrder {
                previous: previous.path.to_string(),
                current: entry.path.to_string(),
            });
        }
        if entry.path == previous.path && !same_stream_product_backing(previous, entry) {
            return Err(AssetCatalogError::SharedProductBackingMismatch {
                path: entry.path.to_string(),
                first_asset_id: previous.asset_id,
                asset_id: entry.asset_id,
            });
        }
    }
    if asset_ids.contains(&entry.asset_id) {
        return Err(AssetCatalogError::DuplicateAssetId {
            asset_id: entry.asset_id,
        });
    }
    if entry.path_registration.is_registered() {
        if catalog_paths.contains(&entry.path) {
            return Err(AssetCatalogError::DuplicateProductPath {
                path: entry.path.to_string(),
            });
        }
        let mut entry_aliases = BTreeSet::new();
        for alias in &entry.catalog_aliases {
            if catalog_paths.contains(alias) || alias == &entry.path || !entry_aliases.insert(alias)
            {
                return Err(AssetCatalogError::DuplicateProductPath {
                    path: alias.to_string(),
                });
            }
        }
    }
    Ok(())
}

fn validate_entry(entry: &AssetCatalogEntry) -> Result<(), AssetCatalogError> {
    validate_asset_tree_path("source path", entry.source_path.as_str())?;
    validate_asset_tree_path("product path", entry.path.as_str())?;
    if entry.asset_id.guid.is_nil() {
        return Err(AssetCatalogError::NilAssetId {
            path: entry.path.to_string(),
        });
    }
    if entry.asset_type.is_nil() {
        return Err(AssetCatalogError::NilAssetType {
            path: entry.path.to_string(),
        });
    }
    if entry.product_format.trim().is_empty() {
        return Err(AssetCatalogError::EmptyProductFormat {
            path: entry.path.to_string(),
        });
    }
    if entry.product_format_version == 0 {
        return Err(AssetCatalogError::InvalidProductFormatVersion {
            path: entry.path.to_string(),
            version: entry.product_format_version,
        });
    }
    if !entry.path_registration.is_registered() && !entry.catalog_aliases.is_empty() {
        return Err(AssetCatalogError::AssetIdOnlyWithAliases {
            path: entry.path.to_string(),
            asset_id: entry.asset_id,
        });
    }
    for alias in &entry.catalog_aliases {
        validate_asset_tree_path("catalog alias", alias.as_str())?;
    }
    Ok(())
}

fn same_stream_product_backing(left: &StreamEntryState, right: &AssetCatalogEntry) -> bool {
    left.asset_type == right.asset_type
        && left.product_format == right.product_format
        && left.product_format_version == right.product_format_version
        && left.schema_version == right.schema_version
        && left.byte_len == right.byte_len
        && left.content_hash == right.content_hash
        && left.dependencies == right.dependencies
}

fn same_product_backing(left: &AssetCatalogEntry, right: &AssetCatalogEntry) -> bool {
    left.asset_type == right.asset_type
        && left.product_format == right.product_format
        && left.product_format_version == right.product_format_version
        && left.schema_version == right.schema_version
        && left.byte_len == right.byte_len
        && left.content_hash == right.content_hash
        && left.dependencies == right.dependencies
}

fn validate_legacy_redirects(
    entries: &[AssetCatalogEntry],
    redirects: &[AssetCatalogLegacyRedirect],
) -> Result<(), AssetCatalogError> {
    let asset_ids = entries
        .iter()
        .map(|entry| entry.asset_id)
        .collect::<BTreeSet<_>>();
    validate_legacy_redirects_for_asset_ids(&asset_ids, redirects)
}

fn validate_legacy_redirects_for_asset_ids(
    asset_ids: &BTreeSet<AssetId>,
    redirects: &[AssetCatalogLegacyRedirect],
) -> Result<(), AssetCatalogError> {
    let mut legacy_ids = BTreeSet::new();

    for redirect in redirects {
        if redirect.legacy.guid.is_nil() {
            return Err(AssetCatalogError::NilLegacyAssetId);
        }
        if redirect.real.guid.is_nil() {
            return Err(AssetCatalogError::NilLegacyRedirectTarget {
                legacy: redirect.legacy,
            });
        }
        if redirect.legacy == redirect.real {
            return Err(AssetCatalogError::SelfLegacyRedirect {
                legacy: redirect.legacy,
            });
        }
        if asset_ids.contains(&redirect.legacy) {
            return Err(AssetCatalogError::LegacyRedirectShadowsCatalogAssetId {
                legacy: redirect.legacy,
            });
        }
        if !asset_ids.contains(&redirect.real) {
            return Err(AssetCatalogError::MissingLegacyRedirectTarget {
                real: redirect.real,
            });
        }
        if !legacy_ids.insert(redirect.legacy) {
            return Err(AssetCatalogError::DuplicateLegacyRedirect {
                legacy: redirect.legacy,
            });
        }
    }

    Ok(())
}

fn validate_asset_tree_path(field: &'static str, value: &str) -> Result<(), AssetCatalogError> {
    if value.trim().is_empty() {
        return Err(AssetCatalogError::InvalidRelativePath {
            field,
            path: value.to_string(),
        });
    }

    let normalized = value.replace('\\', "/");
    let path = Path::new(&normalized);
    let mut has_normal_component = false;
    for component in path.components() {
        match component {
            Component::Normal(segment) if !segment.to_string_lossy().trim().is_empty() => {
                has_normal_component = true;
            }
            Component::Normal(_)
            | Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(AssetCatalogError::InvalidRelativePath {
                    field,
                    path: value.to_string(),
                });
            }
        }
    }

    if has_normal_component {
        Ok(())
    } else {
        Err(AssetCatalogError::InvalidRelativePath {
            field,
            path: value.to_string(),
        })
    }
}

struct CountingWriter<W> {
    inner: W,
    bytes_written: u64,
}

impl<W> CountingWriter<W> {
    const fn new(inner: W) -> Self {
        Self {
            inner,
            bytes_written: 0,
        }
    }

    const fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.bytes_written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn write_optimized_index<W: Write>(
    catalog: &AssetCatalog,
    writer: &mut CountingWriter<W>,
) -> Result<(), AssetCatalogError> {
    let index = build_optimized_index(catalog)?;
    write_optimized_index_rows(&index, writer)
}

fn write_optimized_index_rows<W: Write>(
    index: &AssetCatalogOptimizedIndex,
    writer: &mut CountingWriter<W>,
) -> Result<(), AssetCatalogError> {
    let offset = writer.bytes_written();

    writer.write_all(ASSET_CATALOG_OPTIMIZED_INDEX_MAGIC)?;
    write_u32(writer, ASSET_CATALOG_OPTIMIZED_INDEX_VERSION)?;
    write_u32(
        writer,
        u32::try_from(index.id.len()).map_err(|_| AssetCatalogError::TooManyEntries {
            count: index.id.len(),
        })?,
    )?;
    write_u32(
        writer,
        u32::try_from(index.path_hash.len()).map_err(|_| AssetCatalogError::TooManyEntries {
            count: index.path_hash.len(),
        })?,
    )?;
    write_u32(
        writer,
        u32::try_from(index.legacy.len()).map_err(|_| {
            AssetCatalogError::TooManyLegacyRedirects {
                count: index.legacy.len(),
            }
        })?,
    )?;

    for row in &index.id {
        write_uuid(writer, row.asset_id.guid)?;
        write_u32(writer, row.asset_id.sub_id)?;
        write_u32(writer, row.entry_index)?;
    }
    for row in &index.path_hash {
        writer.write_all(&row.path_hash)?;
        write_u32(writer, row.entry_index)?;
        write_u32(writer, 0)?;
    }
    for row in &index.legacy {
        write_uuid(writer, row.legacy.guid)?;
        write_u32(writer, row.legacy.sub_id)?;
        write_u32(writer, row.real_entry_index)?;
    }

    let len = writer.bytes_written() - offset;
    writer.write_all(ASSET_CATALOG_OPTIMIZED_INDEX_FOOTER_MAGIC)?;
    write_u64(writer, offset)?;
    write_u64(writer, len)?;
    Ok(())
}

fn write_stream_entry(
    writer: &mut impl Write,
    entry: &AssetCatalogEntry,
) -> Result<(), AssetCatalogError> {
    write_uuid(writer, entry.asset_id.guid)?;
    write_u32(writer, entry.asset_id.sub_id)?;
    write_uuid(writer, entry.asset_type)?;
    write_u32(writer, entry.product_format_version)?;
    write_u32(writer, entry.schema_version.unwrap_or(u32::MAX))?;
    write_u64(writer, entry.byte_len)?;
    writer.write_all(&entry.content_hash)?;

    let product_format = entry.product_format.as_bytes();
    write_u32(
        writer,
        u32::try_from(product_format.len()).map_err(|_| AssetCatalogError::TextTooLong {
            field: "product format",
            byte_len: product_format.len(),
        })?,
    )?;
    writer.write_all(product_format)?;

    let source_path = entry.source_path.as_str().as_bytes();
    write_u32(
        writer,
        u32::try_from(source_path.len()).map_err(|_| AssetCatalogError::PathTooLong {
            path: entry.source_path.to_string(),
            byte_len: source_path.len(),
        })?,
    )?;
    writer.write_all(source_path)?;

    let path = entry.path.as_str().as_bytes();
    write_u32(
        writer,
        u32::try_from(path.len()).map_err(|_| AssetCatalogError::PathTooLong {
            path: entry.path.to_string(),
            byte_len: path.len(),
        })?,
    )?;
    writer.write_all(path)?;

    write_u32(writer, entry.path_registration as u32)?;
    write_u32(
        writer,
        u32::try_from(entry.catalog_aliases.len()).map_err(|_| {
            AssetCatalogError::TooManyCatalogAliases {
                path: entry.path.to_string(),
                count: entry.catalog_aliases.len(),
            }
        })?,
    )?;
    for alias in &entry.catalog_aliases {
        let alias_bytes = alias.as_str().as_bytes();
        write_u32(
            writer,
            u32::try_from(alias_bytes.len()).map_err(|_| AssetCatalogError::PathTooLong {
                path: alias.to_string(),
                byte_len: alias_bytes.len(),
            })?,
        )?;
        writer.write_all(alias_bytes)?;
    }

    write_u32(
        writer,
        u32::try_from(entry.dependencies.len()).map_err(|_| {
            AssetCatalogError::TooManyDependencies {
                path: entry.path.to_string(),
                count: entry.dependencies.len(),
            }
        })?,
    )?;
    for dependency in &entry.dependencies {
        write_uuid(writer, dependency.id.guid)?;
        write_u32(writer, dependency.id.sub_id)?;
        write_uuid(writer, dependency.asset_type)?;
        write_optional_str(writer, dependency.hint.as_deref())?;
    }
    Ok(())
}

fn build_optimized_index(
    catalog: &AssetCatalog,
) -> Result<AssetCatalogOptimizedIndex, AssetCatalogError> {
    let mut entry_indices = BTreeMap::new();
    let mut by_id = Vec::with_capacity(catalog.entries.len());
    let path_count = catalog
        .entries
        .iter()
        .filter(|entry| entry.path_registration.is_registered())
        .map(|entry| entry.catalog_aliases.len() + 1)
        .sum();
    let mut by_path_hash = Vec::with_capacity(path_count);

    for (index, entry) in catalog.entries.iter().enumerate() {
        let entry_index = u32::try_from(index).map_err(|_| AssetCatalogError::TooManyEntries {
            count: catalog.entries.len(),
        })?;
        entry_indices.insert(entry.asset_id, entry_index);
        by_id.push(OptimizedIdRow {
            asset_id: entry.asset_id,
            entry_index,
        });
        if entry.path_registration.is_registered() {
            by_path_hash.push(OptimizedPathRow {
                path_hash: asset_catalog_path_hash(&entry.path),
                entry_index,
            });
            by_path_hash.extend(entry.catalog_aliases.iter().map(|alias| OptimizedPathRow {
                path_hash: asset_catalog_path_hash(alias),
                entry_index,
            }));
        }
    }

    let mut by_legacy = Vec::with_capacity(catalog.legacy_redirects.len());
    for redirect in &catalog.legacy_redirects {
        let Some(&real_entry_index) = entry_indices.get(&redirect.real) else {
            return Err(AssetCatalogError::MissingLegacyRedirectTarget {
                real: redirect.real,
            });
        };
        by_legacy.push(OptimizedLegacyRow {
            legacy: redirect.legacy,
            real_entry_index,
        });
    }

    by_id.sort_by_key(|row| row.asset_id);
    by_path_hash.sort_by_key(|row| (row.path_hash, row.entry_index));
    by_legacy.sort_by_key(|row| row.legacy);

    Ok(AssetCatalogOptimizedIndex {
        id: by_id,
        path_hash: by_path_hash,
        legacy: by_legacy,
    })
}

fn read_optimized_index_tail(
    bytes: &[u8],
    canonical_len: usize,
    catalog: &AssetCatalog,
) -> Result<Option<AssetCatalogOptimizedIndex>, AssetCatalogError> {
    if bytes.len() == canonical_len {
        return Ok(None);
    }
    if bytes.len() < canonical_len + OPTIMIZED_INDEX_FOOTER_SIZE {
        return Err(AssetCatalogError::OptimizedIndexOutOfBounds {
            offset: canonical_len as u64,
            len: (bytes.len() - canonical_len) as u64,
            file_len: bytes.len(),
        });
    }

    let footer_offset = bytes.len() - OPTIMIZED_INDEX_FOOTER_SIZE;
    let footer_magic = read_array::<8>(bytes, footer_offset)?;
    if &footer_magic != ASSET_CATALOG_OPTIMIZED_INDEX_FOOTER_MAGIC {
        return Err(AssetCatalogError::BadOptimizedFooterMagic {
            found: footer_magic,
        });
    }

    let index_offset = read_u64_le(bytes, footer_offset + 8)?;
    let index_len = read_u64_le(bytes, footer_offset + 16)?;
    let index_end = index_offset.checked_add(index_len).ok_or(
        AssetCatalogError::OptimizedIndexOutOfBounds {
            offset: index_offset,
            len: index_len,
            file_len: bytes.len(),
        },
    )?;
    // `index_offset`/`index_end` are untrusted 64-bit file offsets; this
    // workspace only targets 64-bit hosts, where the `usize` round-trip is
    // exact. Even on a hypothetical 32-bit host, `index_offset` and
    // `index_end` would truncate consistently with the slice bounds used
    // below, so a bogus offset fails the bounds/slice checks rather than
    // causing unsafe out-of-bounds access.
    #[allow(clippy::cast_possible_truncation)]
    let index_end_usize = index_end as usize;
    if index_offset < canonical_len as u64
        || index_end != footer_offset as u64
        || index_end_usize > bytes.len()
    {
        return Err(AssetCatalogError::OptimizedIndexOutOfBounds {
            offset: index_offset,
            len: index_len,
            file_len: bytes.len(),
        });
    }

    // `index_offset <= index_end <= bytes.len()` is established above, so
    // both truncate identically to `index_end_usize` and stay in bounds.
    #[allow(clippy::cast_possible_truncation)]
    let index_offset_usize = index_offset as usize;
    let section = &bytes[index_offset_usize..index_end_usize];
    let index = parse_optimized_index(section, catalog)?;
    Ok(Some(index))
}

fn parse_optimized_index(
    section: &[u8],
    catalog: &AssetCatalog,
) -> Result<AssetCatalogOptimizedIndex, AssetCatalogError> {
    if section.len() < OPTIMIZED_INDEX_HEADER_SIZE {
        return Err(AssetCatalogError::OptimizedIndexSizeMismatch {
            expected: OPTIMIZED_INDEX_HEADER_SIZE,
            actual: section.len(),
        });
    }
    let magic = read_array::<8>(section, 0)?;
    if &magic != ASSET_CATALOG_OPTIMIZED_INDEX_MAGIC {
        return Err(AssetCatalogError::BadOptimizedMagic { found: magic });
    }
    let version = read_u32_le(section, 8)?;
    if version != ASSET_CATALOG_OPTIMIZED_INDEX_VERSION {
        return Err(AssetCatalogError::UnsupportedOptimizedVersion {
            version,
            expected: ASSET_CATALOG_OPTIMIZED_INDEX_VERSION,
        });
    }

    let id_count = read_u32_le(section, 12)? as usize;
    let path_count = read_u32_le(section, 16)? as usize;
    let legacy_count = read_u32_le(section, 20)? as usize;
    let id_offset = OPTIMIZED_INDEX_HEADER_SIZE;
    let path_offset = checked_index_end(id_offset, id_count, OPTIMIZED_ID_ROW_SIZE)?;
    let legacy_offset = checked_index_end(path_offset, path_count, OPTIMIZED_PATH_ROW_SIZE)?;
    let expected_len = checked_index_end(legacy_offset, legacy_count, OPTIMIZED_LEGACY_ROW_SIZE)?;
    if expected_len != section.len() {
        return Err(AssetCatalogError::OptimizedIndexSizeMismatch {
            expected: expected_len,
            actual: section.len(),
        });
    }

    let by_id = section[id_offset..path_offset]
        .par_chunks_exact(OPTIMIZED_ID_ROW_SIZE)
        .map(parse_optimized_id_row)
        .collect::<Result<Vec<_>, _>>()?;
    let by_path_hash = section[path_offset..legacy_offset]
        .par_chunks_exact(OPTIMIZED_PATH_ROW_SIZE)
        .map(parse_optimized_path_row)
        .collect::<Result<Vec<_>, _>>()?;
    let by_legacy = section[legacy_offset..expected_len]
        .par_chunks_exact(OPTIMIZED_LEGACY_ROW_SIZE)
        .map(parse_optimized_legacy_row)
        .collect::<Result<Vec<_>, _>>()?;

    let index = AssetCatalogOptimizedIndex {
        id: by_id,
        path_hash: by_path_hash,
        legacy: by_legacy,
    };
    validate_optimized_index(catalog, &index)?;
    Ok(index)
}

fn validate_optimized_index(
    catalog: &AssetCatalog,
    index: &AssetCatalogOptimizedIndex,
) -> Result<(), AssetCatalogError> {
    if index.id.len() != catalog.entries.len() {
        return Err(AssetCatalogError::OptimizedIndexSizeMismatch {
            expected: catalog.entries.len(),
            actual: index.id.len(),
        });
    }
    let path_count = catalog
        .entries
        .iter()
        .filter(|entry| entry.path_registration.is_registered())
        .map(|entry| entry.catalog_aliases.len() + 1)
        .sum();
    if index.path_hash.len() != path_count {
        return Err(AssetCatalogError::OptimizedIndexSizeMismatch {
            expected: path_count,
            actual: index.path_hash.len(),
        });
    }
    if index.legacy.len() != catalog.legacy_redirects.len() {
        return Err(AssetCatalogError::OptimizedIndexSizeMismatch {
            expected: catalog.legacy_redirects.len(),
            actual: index.legacy.len(),
        });
    }
    if !index
        .id
        .windows(2)
        .all(|pair| pair[0].asset_id <= pair[1].asset_id)
    {
        return Err(AssetCatalogError::OptimizedIndexUnsorted { table: "by_id" });
    }
    if !index.path_hash.windows(2).all(|pair| {
        (pair[0].path_hash, pair[0].entry_index) <= (pair[1].path_hash, pair[1].entry_index)
    }) {
        return Err(AssetCatalogError::OptimizedIndexUnsorted {
            table: "by_path_hash",
        });
    }
    if !index
        .legacy
        .windows(2)
        .all(|pair| pair[0].legacy <= pair[1].legacy)
    {
        return Err(AssetCatalogError::OptimizedIndexUnsorted { table: "by_legacy" });
    }

    for row in &index.id {
        let entry = catalog_entry_for_optimized_row(catalog, "by_id", row.entry_index)?;
        if entry.asset_id != row.asset_id {
            return Err(AssetCatalogError::OptimizedIndexMismatch {
                table: "by_id",
                entry_index: row.entry_index,
            });
        }
    }
    for row in &index.path_hash {
        let entry = catalog_entry_for_optimized_row(catalog, "by_path_hash", row.entry_index)?;
        if !entry.path_registration.is_registered()
            || (asset_catalog_path_hash(&entry.path) != row.path_hash
                && !entry
                    .catalog_aliases
                    .iter()
                    .any(|alias| asset_catalog_path_hash(alias) == row.path_hash))
        {
            return Err(AssetCatalogError::OptimizedIndexMismatch {
                table: "by_path_hash",
                entry_index: row.entry_index,
            });
        }
    }
    for row in &index.legacy {
        let entry = catalog_entry_for_optimized_row(catalog, "by_legacy", row.real_entry_index)?;
        let Some(redirect) = catalog
            .legacy_redirects
            .binary_search_by_key(&row.legacy, |redirect| redirect.legacy)
            .ok()
            .and_then(|position| catalog.legacy_redirects.get(position))
        else {
            return Err(AssetCatalogError::OptimizedIndexMismatch {
                table: "by_legacy",
                entry_index: row.real_entry_index,
            });
        };
        if entry.asset_id != redirect.real {
            return Err(AssetCatalogError::OptimizedIndexMismatch {
                table: "by_legacy",
                entry_index: row.real_entry_index,
            });
        }
    }

    Ok(())
}

fn catalog_entry_for_optimized_row<'a>(
    catalog: &'a AssetCatalog,
    table: &'static str,
    entry_index: u32,
) -> Result<&'a AssetCatalogEntry, AssetCatalogError> {
    catalog.entries.get(entry_index as usize).ok_or(
        AssetCatalogError::OptimizedIndexEntryOutOfBounds {
            table,
            entry_index,
            entry_count: catalog.entries.len(),
        },
    )
}

fn parse_optimized_id_row(bytes: &[u8]) -> Result<OptimizedIdRow, AssetCatalogError> {
    Ok(OptimizedIdRow {
        asset_id: AssetId::new(
            Uuid::from_bytes(read_array::<16>(bytes, 0)?),
            read_u32_le(bytes, 16)?,
        ),
        entry_index: read_u32_le(bytes, 20)?,
    })
}

fn parse_optimized_path_row(bytes: &[u8]) -> Result<OptimizedPathRow, AssetCatalogError> {
    Ok(OptimizedPathRow {
        path_hash: read_array::<16>(bytes, 0)?,
        entry_index: read_u32_le(bytes, 16)?,
    })
}

fn parse_optimized_legacy_row(bytes: &[u8]) -> Result<OptimizedLegacyRow, AssetCatalogError> {
    Ok(OptimizedLegacyRow {
        legacy: AssetId::new(
            Uuid::from_bytes(read_array::<16>(bytes, 0)?),
            read_u32_le(bytes, 16)?,
        ),
        real_entry_index: read_u32_le(bytes, 20)?,
    })
}

fn checked_index_end(
    start: usize,
    count: usize,
    stride: usize,
) -> Result<usize, AssetCatalogError> {
    start
        .checked_add(count.checked_mul(stride).ok_or(
            AssetCatalogError::OptimizedIndexOutOfBounds {
                offset: start as u64,
                len: usize::MAX as u64,
                file_len: start,
            },
        )?)
        .ok_or(AssetCatalogError::OptimizedIndexOutOfBounds {
            offset: start as u64,
            len: usize::MAX as u64,
            file_len: start,
        })
}

fn read_stream_entry(
    reader: &mut impl Read,
    index: usize,
) -> Result<AssetCatalogEntry, AssetCatalogError> {
    let asset_guid = read_uuid(reader)?;
    let sub_id = read_u32(reader)?;
    let asset_type = read_uuid(reader)?;
    let product_format_version = read_u32(reader)?;
    let schema_version = match read_u32(reader)? {
        u32::MAX => None,
        version => Some(version),
    };
    let byte_len = read_u64(reader)?;
    let mut content_hash = [0u8; PACKAGE_CONTENT_HASH_BYTES];
    reader.read_exact(&mut content_hash)?;
    let product_format = read_stream_string(reader, index, "product format")?;
    let source_path = read_stream_string(reader, index, "source path")?;
    let path = read_stream_string(reader, index, "path")?;
    let path_registration_raw = read_u32(reader)?;
    let path_registration =
        AssetCatalogPathRegistration::from_u8(u8::try_from(path_registration_raw).map_err(
            |_| AssetCatalogError::InvalidPathRegistration {
                index,
                value: path_registration_raw,
            },
        )?)
        .ok_or(AssetCatalogError::InvalidPathRegistration {
            index,
            value: path_registration_raw,
        })?;

    let alias_count = read_u32(reader)? as usize;
    let mut catalog_aliases = Vec::with_capacity(alias_count);
    for _ in 0..alias_count {
        catalog_aliases.push(read_stream_string(reader, index, "catalog alias")?);
    }

    let dependency_count = read_u32(reader)? as usize;
    let mut dependencies = Vec::with_capacity(dependency_count);
    for _ in 0..dependency_count {
        let dependency_guid = read_uuid(reader)?;
        let dependency_sub_id = read_u32(reader)?;
        let dependency_asset_type = read_uuid(reader)?;
        let hint = read_stream_optional_string(reader, index, "dependency hint")?;
        let mut dependency = ProductDependency::new(
            AssetId::new(dependency_guid, dependency_sub_id),
            dependency_asset_type,
        );
        dependency.hint = hint;
        dependencies.push(dependency);
    }

    Ok(AssetCatalogEntry::new(
        AssetId::new(asset_guid, sub_id),
        asset_type,
        product_format,
        product_format_version,
        path,
        schema_version,
        byte_len,
        content_hash,
    )
    .with_source_path(source_path)
    .with_path_registration(path_registration)
    .with_catalog_aliases(catalog_aliases)
    .with_dependencies(dependencies))
}

fn read_stream_string(
    reader: &mut impl Read,
    index: usize,
    field: &'static str,
) -> Result<String, AssetCatalogError> {
    let len = read_u32(reader)? as usize;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|source| AssetCatalogError::InvalidUtf8 {
        index,
        field,
        source: source.utf8_error(),
    })
}

fn read_stream_optional_string(
    reader: &mut impl Read,
    index: usize,
    field: &'static str,
) -> Result<Option<String>, AssetCatalogError> {
    let mut tag = [0u8; 1];
    reader.read_exact(&mut tag)?;
    match tag[0] {
        0 => Ok(None),
        _ => Ok(Some(read_stream_string(reader, index, field)?)),
    }
}

fn write_optional_str(
    writer: &mut impl Write,
    value: Option<&str>,
) -> Result<(), AssetCatalogError> {
    match value {
        Some(value) => {
            writer.write_all(&[1])?;
            let bytes = value.as_bytes();
            write_u32(
                writer,
                u32::try_from(bytes.len()).map_err(|_| AssetCatalogError::TextTooLong {
                    field: "dependency hint",
                    byte_len: bytes.len(),
                })?,
            )?;
            writer.write_all(bytes)?;
        }
        None => {
            writer.write_all(&[0])?;
        }
    }
    Ok(())
}

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], AssetCatalogError> {
    let end = offset
        .checked_add(N)
        .ok_or(AssetCatalogError::OptimizedIndexOutOfBounds {
            offset: offset as u64,
            len: N as u64,
            file_len: bytes.len(),
        })?;
    bytes
        .get(offset..end)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(AssetCatalogError::OptimizedIndexOutOfBounds {
            offset: offset as u64,
            len: N as u64,
            file_len: bytes.len(),
        })
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, AssetCatalogError> {
    Ok(u32::from_le_bytes(read_array::<4>(bytes, offset)?))
}

fn read_u64_le(bytes: &[u8], offset: usize) -> Result<u64, AssetCatalogError> {
    Ok(u64::from_le_bytes(read_array::<8>(bytes, offset)?))
}

fn write_uuid(writer: &mut impl Write, value: Uuid) -> std::io::Result<()> {
    writer.write_all(value.as_bytes())
}

fn write_u32(writer: &mut impl Write, value: u32) -> std::io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u64(writer: &mut impl Write, value: u64) -> std::io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn read_uuid(reader: &mut impl Read) -> std::io::Result<Uuid> {
    let mut bytes = [0u8; 16];
    reader.read_exact(&mut bytes)?;
    Ok(Uuid::from_bytes(bytes))
}

fn read_u32(reader: &mut impl Read) -> std::io::Result<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> std::io::Result<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_catalog_round_trips_binary_format() {
        let first_hash = *blake3::hash(b"ui").as_bytes();
        let second_hash = *blake3::hash(b"lua").as_bytes();
        let catalog = AssetCatalog::new(vec![
            AssetCatalogEntry::new(
                AssetId::new(Uuid::from_bytes([1; 16]), 1),
                Uuid::from_bytes([2; 16]),
                "az.test.raw",
                1,
                "lyshineui/menu/main.dynamicuicanvas",
                Some(1),
                42,
                first_hash,
            ),
            AssetCatalogEntry::new(
                AssetId::new(Uuid::from_bytes([3; 16]), 2),
                Uuid::from_bytes([4; 16]),
                "az.test.raw",
                1,
                "lyshineui/shared.luac",
                None,
                7,
                second_hash,
            ),
        ])
        .unwrap();

        let mut bytes = Vec::new();
        write_asset_catalog(&catalog, &mut bytes).unwrap();
        assert_eq!(&bytes[..8], ASSET_CATALOG_MAGIC);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            ASSET_CATALOG_VERSION
        );
        let decoded = read_asset_catalog(bytes.as_slice()).unwrap();

        assert_eq!(decoded, catalog);
    }

    #[test]
    fn streaming_encoder_matches_catalog_writer_for_empty_catalog() {
        let catalog = AssetCatalog::new(Vec::new()).unwrap();
        let mut expected = Vec::new();
        write_asset_catalog(&catalog, &mut expected).unwrap();

        let encoder = AssetCatalogStreamEncoder::new(
            Vec::new(),
            catalog.entries().len(),
            catalog.legacy_redirects().to_vec(),
        )
        .unwrap();
        let (actual, receipt) = encoder.finish().unwrap();

        assert_eq!(actual, expected);
        assert_eq!(receipt.entry_count, 0);
        assert_eq!(receipt.byte_count, actual.len() as u64);
        assert_eq!(
            read_prepared_asset_catalog(actual.as_slice())
                .unwrap()
                .catalog(),
            &catalog
        );
    }

    #[test]
    fn streaming_encoder_matches_catalog_writer_with_aliases_dependencies_and_redirect() {
        let real = AssetId::new(Uuid::from_bytes([1; 16]), 7);
        let catalog = AssetCatalog::with_legacy_redirects(
            vec![
                AssetCatalogEntry::new(
                    real,
                    Uuid::from_bytes([2; 16]),
                    "az.test.raw",
                    1,
                    "products/ui/menu.azbin",
                    Some(3),
                    99,
                    *blake3::hash(b"menu").as_bytes(),
                )
                .with_catalog_aliases(vec![AssetTreePath::new("products/ui/default.azbin")])
                .with_dependencies(vec![ProductDependency::new(
                    AssetId::new(Uuid::from_bytes([3; 16]), 1),
                    Uuid::from_bytes([4; 16]),
                )]),
                AssetCatalogEntry::new(
                    AssetId::new(Uuid::from_bytes([5; 16]), 2),
                    Uuid::from_bytes([6; 16]),
                    "az.test.raw",
                    1,
                    "products/world/terrain.azbin",
                    None,
                    101,
                    *blake3::hash(b"terrain").as_bytes(),
                ),
            ],
            vec![AssetCatalogLegacyRedirect::new(
                AssetId::new(Uuid::from_bytes([8; 16]), 9),
                real,
            )],
        )
        .unwrap();
        let mut expected = Vec::new();
        write_asset_catalog(&catalog, &mut expected).unwrap();

        let mut encoder = AssetCatalogStreamEncoder::new(
            Vec::new(),
            catalog.entries().len(),
            catalog.legacy_redirects().to_vec(),
        )
        .unwrap();
        for entry in catalog.entries() {
            encoder.push(entry).unwrap();
        }
        let (actual, receipt) = encoder.finish().unwrap();

        assert_eq!(actual, expected);
        assert_eq!(
            receipt.entry_count,
            u32::try_from(catalog.entries().len()).unwrap()
        );
        assert_eq!(receipt.byte_count, actual.len() as u64);
    }

    #[test]
    fn unknown_count_streaming_encoder_patches_header_matches_known_encoding_and_restores_position()
    {
        let real = AssetId::new(Uuid::from_bytes([1; 16]), 7);
        let catalog = AssetCatalog::with_legacy_redirects(
            vec![
                AssetCatalogEntry::new(
                    real,
                    Uuid::from_bytes([2; 16]),
                    "az.test.raw",
                    1,
                    "products/ui/menu.azbin",
                    Some(3),
                    99,
                    *blake3::hash(b"menu").as_bytes(),
                )
                .with_catalog_aliases(vec![AssetTreePath::new("products/ui/default.azbin")])
                .with_dependencies(vec![ProductDependency::new(
                    AssetId::new(Uuid::from_bytes([3; 16]), 1),
                    Uuid::from_bytes([4; 16]),
                )]),
                AssetCatalogEntry::new(
                    AssetId::new(Uuid::from_bytes([5; 16]), 2),
                    Uuid::from_bytes([6; 16]),
                    "az.test.raw",
                    1,
                    "products/world/terrain.azbin",
                    None,
                    101,
                    *blake3::hash(b"terrain").as_bytes(),
                ),
            ],
            vec![AssetCatalogLegacyRedirect::new(
                AssetId::new(Uuid::from_bytes([8; 16]), 9),
                real,
            )],
        )
        .unwrap();
        let mut expected = Vec::new();
        write_asset_catalog(&catalog, &mut expected).unwrap();

        let prefix = vec![0xD0, 0x0D, 0xF0, 0x0D];
        let mut sink = Cursor::new(prefix.clone());
        sink.set_position(prefix.len() as u64);
        let mut encoder =
            AssetCatalogStreamEncoder::new_unknown_count(sink, catalog.legacy_redirects().to_vec())
                .unwrap();
        let entry_count_offset =
            prefix.len() + ASSET_CATALOG_MAGIC.len() + std::mem::size_of::<u32>();
        assert_eq!(
            u32::from_le_bytes(
                encoder.writer.inner.get_ref()[entry_count_offset..entry_count_offset + 4]
                    .try_into()
                    .unwrap()
            ),
            0
        );

        for entry in catalog.entries() {
            encoder.push(entry).unwrap();
        }
        let (sink, receipt) = encoder.finish().unwrap();
        let final_position = sink.position();
        let actual = sink.into_inner();

        assert_eq!(&actual[..prefix.len()], prefix.as_slice());
        assert_eq!(&actual[prefix.len()..], expected.as_slice());
        assert_eq!(
            u32::from_le_bytes(
                actual[entry_count_offset..entry_count_offset + 4]
                    .try_into()
                    .unwrap()
            ),
            u32::try_from(catalog.entries().len()).unwrap()
        );
        assert_eq!(
            receipt.entry_count,
            u32::try_from(catalog.entries().len()).unwrap()
        );
        assert_eq!(receipt.byte_count, expected.len() as u64);
        assert_eq!(final_position, actual.len() as u64);
        assert_eq!(
            read_prepared_asset_catalog(actual[prefix.len()..].as_ref())
                .unwrap()
                .catalog(),
            &catalog
        );
    }

    #[test]
    fn unknown_count_streaming_encoder_retains_entry_and_redirect_validation() {
        let entry = |guid_byte: u8, path: &str| {
            AssetCatalogEntry::new(
                AssetId::new(Uuid::from_bytes([guid_byte; 16]), 1),
                Uuid::from_bytes([2; 16]),
                "az.test.raw",
                1,
                path,
                None,
                1,
                *blake3::hash(path.as_bytes()).as_bytes(),
            )
        };
        let later = entry(1, "products/z.azbin");
        let earlier = entry(3, "products/a.azbin");
        let mut out_of_order =
            AssetCatalogStreamEncoder::new_unknown_count(Cursor::new(Vec::new()), Vec::new())
                .unwrap();
        out_of_order.push(&later).unwrap();
        assert!(matches!(
            out_of_order.push(&earlier),
            Err(AssetCatalogError::StreamEntriesOutOfOrder { .. })
        ));

        let mut missing_redirect_target = AssetCatalogStreamEncoder::new_unknown_count(
            Cursor::new(Vec::new()),
            vec![AssetCatalogLegacyRedirect::new(
                AssetId::new(Uuid::from_bytes([4; 16]), 1),
                AssetId::new(Uuid::from_bytes([5; 16]), 1),
            )],
        )
        .unwrap();
        missing_redirect_target
            .push(&entry(6, "products/real.azbin"))
            .unwrap();
        assert!(matches!(
            missing_redirect_target.finish(),
            Err(AssetCatalogError::MissingLegacyRedirectTarget { .. })
        ));
    }

    #[test]
    fn streaming_encoder_matches_catalog_writer_for_large_catalog() {
        let entries = (0_u32..4096)
            .map(|index| {
                AssetCatalogEntry::new(
                    AssetId::new(Uuid::from_u128(u128::from(index) + 1), index),
                    Uuid::from_bytes([9; 16]),
                    "az.test.raw",
                    1,
                    format!("products/generated/{index:05}.azbin"),
                    None,
                    u64::from(index),
                    *blake3::hash(index.to_le_bytes().as_slice()).as_bytes(),
                )
            })
            .collect();
        let catalog = AssetCatalog::new(entries).unwrap();
        let mut expected = Vec::new();
        write_asset_catalog(&catalog, &mut expected).unwrap();

        let mut encoder =
            AssetCatalogStreamEncoder::new(Vec::new(), catalog.entries().len(), Vec::new())
                .unwrap();
        for entry in catalog.entries() {
            encoder.push(entry).unwrap();
        }
        let (actual, receipt) = encoder.finish().unwrap();

        assert_eq!(actual, expected);
        assert_eq!(receipt.entry_count, 4096);
        assert_eq!(receipt.byte_count, actual.len() as u64);
    }

    #[test]
    fn catalog_alias_round_trips_and_resolves_to_the_canonical_entry() {
        let asset_id = AssetId::new(Uuid::from_bytes([1; 16]), 1);
        let catalog = AssetCatalog::new(vec![
            AssetCatalogEntry::new(
                asset_id,
                Uuid::from_bytes([2; 16]),
                "az.test.raw",
                1,
                "prefabs/slices/player.scn.bin",
                None,
                42,
                *blake3::hash(b"player").as_bytes(),
            )
            .with_catalog_aliases(["Slices/Player.dynamicslice"]),
        ])
        .unwrap();

        let mut bytes = Vec::new();
        write_asset_catalog(&catalog, &mut bytes).unwrap();
        let prepared = read_prepared_asset_catalog(bytes.as_slice()).unwrap();
        let alias = prepared
            .entry_by_path("slices/player.dynamicslice")
            .unwrap();
        assert_eq!(alias.asset_id, asset_id);
        assert_eq!(alias.path.as_str(), "prefabs/slices/player.scn.bin");
        assert_eq!(prepared.entry_by_id(asset_id), Some(alias));
        assert_eq!(prepared.entries().len(), 1);
    }

    #[test]
    fn catalog_keeps_shared_payload_identities_and_one_path_owner() {
        let guid = Uuid::from_bytes([1; 16]);
        let asset_type = Uuid::from_bytes([2; 16]);
        let bytes = b"shared slice metadata";
        let path = "slices/shared.slice.meta";
        let registered_id = AssetId::new(guid, 7);
        let id_only = AssetId::new(guid, 3);
        let entry = |asset_id| {
            AssetCatalogEntry::new(
                asset_id,
                asset_type,
                "az.test.raw",
                1,
                path,
                None,
                bytes.len() as u64,
                *blake3::hash(bytes).as_bytes(),
            )
        };
        let catalog = AssetCatalog::new(vec![
            entry(id_only).with_path_registration(AssetCatalogPathRegistration::AssetIdOnly),
            entry(registered_id),
        ])
        .unwrap();

        let mut encoded = Vec::new();
        write_asset_catalog(&catalog, &mut encoded).unwrap();
        let prepared = read_prepared_asset_catalog(encoded.as_slice()).unwrap();

        assert_eq!(prepared.entries().len(), 2);
        assert_eq!(prepared.entry_by_id(id_only).unwrap().path.as_str(), path);
        assert_eq!(
            prepared.entry_by_path(path).unwrap().asset_id,
            registered_id
        );
    }

    #[test]
    fn catalog_rejects_mismatched_shared_payload_backing() {
        let guid = Uuid::from_bytes([1; 16]);
        let path = "slices/shared.slice.meta";
        let registered = AssetCatalogEntry::new(
            AssetId::new(guid, 7),
            Uuid::from_bytes([2; 16]),
            "az.test.raw",
            1,
            path,
            None,
            4,
            *blake3::hash(b"left").as_bytes(),
        );
        let id_only = AssetCatalogEntry::new(
            AssetId::new(guid, 3),
            Uuid::from_bytes([2; 16]),
            "az.test.raw",
            1,
            path,
            None,
            5,
            *blake3::hash(b"right").as_bytes(),
        )
        .with_path_registration(AssetCatalogPathRegistration::AssetIdOnly);

        assert!(matches!(
            AssetCatalog::new(vec![registered, id_only]),
            Err(AssetCatalogError::SharedProductBackingMismatch { .. })
        ));
    }

    /// Every spelling of one asset path collapses to a single canonical
    /// string, so identity, hashing, and persisted bytes cannot diverge.
    #[test]
    fn asset_tree_paths_share_one_canonical_spelling() {
        let canonical = AssetTreePath::new("objects/foo/bar.cgf");
        assert_eq!(canonical.as_str(), "objects/foo/bar.cgf");

        for spelling in [
            r"Objects\Foo\Bar.CGF",
            "/Objects/Foo/Bar.cgf",
            "  Objects/Foo/Bar.cgf  ",
            "./objects//foo/bar.cgf",
            r"\Objects\Foo\\Bar.CGF\",
        ] {
            assert_eq!(
                AssetTreePath::new(spelling),
                canonical,
                "`{spelling}` must resolve to the canonical asset path"
            );
        }

        // Canonical form is a fixed point. That idempotence is what lets every
        // lookup surface fold an argument that is already canonical.
        assert_eq!(AssetTreePath::new(canonical.as_str()), canonical);
    }

    #[test]
    fn asset_catalog_path_lookup_is_case_and_separator_insensitive() {
        const QUERIES: [&str; 4] = [
            r"SHAREDASSETS\PHYSICS\DEFAULT.COLLISIONFILTERS",
            "/SharedAssets/Physics/default.collisionfilters",
            "  sharedassets/physics/default.collisionfilters  ",
            "./sharedassets//physics/default.collisionfilters",
        ];

        let entry = AssetCatalogEntry::new(
            AssetId::new(Uuid::from_bytes([1; 16]), 1),
            Uuid::from_bytes([2; 16]),
            "az.test.raw",
            1,
            "SharedAssets/Physics/default.collisionfilters",
            None,
            42,
            *blake3::hash(b"filters").as_bytes(),
        );
        let catalog = AssetCatalog::new(vec![entry]).unwrap();
        assert_eq!(
            catalog.entries()[0].path.as_str(),
            "sharedassets/physics/default.collisionfilters"
        );

        let prepared = catalog.clone().prepare();
        for query in QUERIES {
            assert!(
                prepared.entry_by_path(query).is_some(),
                "materialized lookup must fold `{query}`"
            );
        }

        // The optimized in-file index hashes the same canonical form, so it
        // must resolve exactly the same query spellings.
        let mut bytes = Vec::new();
        write_asset_catalog(&catalog, &mut bytes).unwrap();
        let prepared = read_prepared_asset_catalog(bytes.as_slice()).unwrap();
        for query in QUERIES {
            assert!(
                prepared.entry_by_path(query).is_some(),
                "optimized lookup must fold `{query}`"
            );
        }
    }

    #[test]
    fn asset_catalog_round_trips_product_dependencies() {
        let dep_a = ProductDependency::new(
            AssetId::new(Uuid::from_bytes([9; 16]), 3),
            Uuid::from_bytes([8; 16]),
        )
        .with_hint("@assets@/materials/shared/base.material.ron");
        let dep_b = ProductDependency::new(
            AssetId::new(Uuid::from_bytes([5; 16]), 1),
            Uuid::from_bytes([6; 16]),
        );
        let catalog = AssetCatalog::new(vec![
            AssetCatalogEntry::new(
                AssetId::new(Uuid::from_bytes([1; 16]), 1),
                Uuid::from_bytes([2; 16]),
                "az.test.raw",
                1,
                "prefabs/camp.prefab.azbin",
                Some(1),
                42,
                *blake3::hash(b"camp").as_bytes(),
            )
            // Intentionally unsorted so the round trip proves normalization.
            .with_dependencies(vec![dep_a.clone(), dep_b.clone()]),
            AssetCatalogEntry::new(
                AssetId::new(Uuid::from_bytes([3; 16]), 0),
                Uuid::from_bytes([4; 16]),
                "az.test.raw",
                1,
                "materials/shared/base.material.azbin",
                None,
                7,
                *blake3::hash(b"mat").as_bytes(),
            ),
        ])
        .unwrap();

        let find = |catalog: &AssetCatalog, path: &str| {
            catalog
                .entries()
                .iter()
                .find(|entry| entry.path.as_str() == path)
                .expect("catalog entry")
                .clone()
        };

        // Deterministic ordering: dep_b sorts before dep_a by AssetId.
        assert_eq!(
            find(&catalog, "prefabs/camp.prefab.azbin").dependencies,
            vec![dep_b.clone(), dep_a.clone()]
        );

        let mut bytes = Vec::new();
        write_asset_catalog(&catalog, &mut bytes).unwrap();

        let streamed = read_asset_catalog(bytes.as_slice()).unwrap();
        assert_eq!(streamed, catalog);
        assert_eq!(
            find(&streamed, "prefabs/camp.prefab.azbin").dependencies,
            vec![dep_b.clone(), dep_a.clone()]
        );
        assert!(
            find(&streamed, "materials/shared/base.material.azbin")
                .dependencies
                .is_empty()
        );

        // The optimized-tail reader consumes the same canonical stream, so the
        // dependency lists must survive that path too.
        let prepared = read_prepared_asset_catalog(bytes.as_slice()).unwrap();
        let entry = prepared
            .entry_by_path("prefabs/camp.prefab.azbin")
            .expect("catalog entry by path");
        assert_eq!(entry.dependencies, vec![dep_b, dep_a]);
    }

    #[test]
    fn asset_catalog_round_trips_legacy_redirects() {
        let real = AssetId::new(Uuid::from_bytes([1; 16]), 0x181a_6070);
        let legacy = AssetId::new(Uuid::from_bytes([2; 16]), 0xd087_f9c9);
        let catalog = AssetCatalog::with_legacy_redirects(
            vec![AssetCatalogEntry::new(
                real,
                Uuid::from_bytes([3; 16]),
                "az.test.raw",
                1,
                "slices/dungeon/firstlight/ancientgrate_circular__28236438930.cgf",
                None,
                1668,
                *blake3::hash(b"slice").as_bytes(),
            )],
            vec![AssetCatalogLegacyRedirect::new(legacy, real)],
        )
        .unwrap();

        let mut bytes = Vec::new();
        write_asset_catalog(&catalog, &mut bytes).unwrap();
        let decoded = read_asset_catalog(bytes.as_slice()).unwrap();

        assert_eq!(decoded.legacy_redirects(), catalog.legacy_redirects());
    }

    #[test]
    fn full_catalog_reader_uses_in_file_optimized_index() {
        let real = AssetId::new(Uuid::from_bytes([1; 16]), 0x181a_6070);
        let legacy = AssetId::new(Uuid::from_bytes([2; 16]), 0xd087_f9c9);
        let path = "slices/dungeon/firstlight/ancientgrate_circular__28236438930.cgf";
        let catalog = AssetCatalog::with_legacy_redirects(
            vec![AssetCatalogEntry::new(
                real,
                Uuid::from_bytes([3; 16]),
                "az.test.raw",
                1,
                path,
                None,
                1668,
                *blake3::hash(b"slice").as_bytes(),
            )],
            vec![AssetCatalogLegacyRedirect::new(legacy, real)],
        )
        .unwrap();

        let mut bytes = Vec::new();
        write_asset_catalog(&catalog, &mut bytes).unwrap();

        let streamed = read_asset_catalog(bytes.as_slice()).unwrap();
        assert_eq!(streamed, catalog);

        let prepared = read_prepared_asset_catalog(bytes.as_slice()).unwrap();
        assert_eq!(prepared.entries().len(), 1);
        assert_eq!(
            prepared.entry_by_id(real).map(|entry| entry.asset_id),
            Some(real)
        );
        assert_eq!(
            prepared.entry_by_path(path).map(|entry| entry.asset_id),
            Some(real)
        );
        assert_eq!(
            prepared.entry_by_id(legacy).map(|entry| entry.asset_id),
            Some(real)
        );
    }

    #[test]
    fn asset_catalog_rejects_legacy_redirect_without_real_entry() {
        let legacy = AssetId::new(Uuid::from_bytes([2; 16]), 0xd087_f9c9);
        let missing_real = AssetId::new(Uuid::from_bytes([1; 16]), 0x181a_6070);
        let error = AssetCatalog::with_legacy_redirects(
            vec![AssetCatalogEntry::new(
                AssetId::new(Uuid::from_bytes([4; 16]), 7),
                Uuid::from_bytes([3; 16]),
                "az.test.raw",
                1,
                "objects/other.cgf",
                None,
                8,
                *blake3::hash(b"other").as_bytes(),
            )],
            vec![AssetCatalogLegacyRedirect::new(legacy, missing_real)],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AssetCatalogError::MissingLegacyRedirectTarget { real } if real == missing_real
        ));
    }

    #[test]
    fn asset_catalog_builds_from_package_manifest() {
        let bytes = b"compiled material";
        let content_hash = *blake3::hash(bytes).as_bytes();
        let profile = crate::package::PackageManifestProfile {
            name: "pc-release".to_string(),
            asset_platform: "pc".to_string(),
            cargo_profile: "release".to_string(),
            container: "azpack".to_string(),
            compression: "oodle".to_string(),
            oodle_compressor: Some("kraken".to_string()),
            oodle_effort: Some("normal".to_string()),
        };
        let manifest = crate::package::PackageManifest::new(
            profile,
            vec![crate::package::PackageManifestEntry::new(
                "materials/armor/foo.mtl",
                Uuid::from_bytes([2; 16]),
                7,
                "az.test.raw",
                1,
                content_hash,
                bytes.len() as u64,
                Uuid::from_bytes([1; 16]),
                "materials/armor/foo.material.ron",
                "BuildMaterial",
            )],
        )
        .unwrap();

        let catalog = AssetCatalog::from_package_manifest(&manifest).unwrap();

        assert_eq!(catalog.entries.len(), 1);
        let entry = &catalog.entries[0];
        assert_eq!(entry.asset_id, AssetId::new(Uuid::from_bytes([1; 16]), 7));
        assert_eq!(entry.asset_type, Uuid::from_bytes([2; 16]));
        assert_eq!(entry.product_format, "az.test.raw");
        assert_eq!(entry.product_format_version, 1);
        assert_eq!(
            entry.source_path.as_str(),
            "@assets@/materials/armor/foo.material.ron"
        );
        assert_eq!(entry.path.as_str(), "materials/armor/foo.mtl");
        assert_eq!(entry.byte_len, bytes.len() as u64);
        assert_eq!(entry.content_hash, content_hash);
    }

    #[test]
    fn asset_catalog_keeps_registered_asset_type_as_authority() {
        let profile = crate::package::PackageManifestProfile {
            name: "pc-release".to_string(),
            asset_platform: "pc".to_string(),
            cargo_profile: "release".to_string(),
            container: "azpack".to_string(),
            compression: "oodle".to_string(),
            oodle_compressor: Some("kraken".to_string()),
            oodle_effort: Some("normal".to_string()),
        };
        let manifest = crate::package::PackageManifest::new(
            profile,
            vec![crate::package::PackageManifestEntry::new(
                "products/prefab.unknown-product",
                Uuid::from_bytes([2; 16]),
                7,
                "az.test.raw",
                1,
                *blake3::hash(b"compiled prefab").as_bytes(),
                15,
                Uuid::from_bytes([1; 16]),
                "prefabs/source.prefab.ron",
                "BuildPrefab",
            )],
        )
        .unwrap();

        let catalog = AssetCatalog::from_package_manifest(&manifest).unwrap();

        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(
            catalog.entries[0].path.as_str(),
            "products/prefab.unknown-product"
        );
        assert_eq!(catalog.entries[0].asset_type, Uuid::from_bytes([2; 16]));
        assert_eq!(catalog.entries[0].product_format, "az.test.raw");
        assert_eq!(catalog.entries[0].product_format_version, 1);
    }

    #[test]
    fn asset_catalog_requires_product_format_identity() {
        let hash = *blake3::hash(b"compiled prefab").as_bytes();
        let mut missing_format = AssetCatalogEntry::new(
            AssetId::new(Uuid::from_bytes([1; 16]), 7),
            Uuid::from_bytes([2; 16]),
            "",
            1,
            "products/prefab.azbin",
            None,
            15,
            hash,
        );

        let error = AssetCatalog::new(vec![missing_format.clone()]).unwrap_err();
        assert!(matches!(
            error,
            AssetCatalogError::EmptyProductFormat { .. }
        ));

        missing_format.product_format = "az.test.raw".to_string();
        missing_format.product_format_version = 0;
        let error = AssetCatalog::new(vec![missing_format]).unwrap_err();
        assert!(matches!(
            error,
            AssetCatalogError::InvalidProductFormatVersion { .. }
        ));
    }

    #[test]
    fn asset_catalog_rejects_version_in_magic() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"AZCATAL1");
        bytes.extend_from_slice(&ASSET_CATALOG_VERSION.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        let error = read_asset_catalog(bytes.as_slice()).unwrap_err();
        assert!(matches!(error, AssetCatalogError::BadMagic { .. }));
    }

    #[test]
    fn asset_catalog_rejects_unsupported_header_version() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(ASSET_CATALOG_MAGIC);
        bytes.extend_from_slice(&(ASSET_CATALOG_VERSION + 1).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        let error = read_asset_catalog(bytes.as_slice()).unwrap_err();
        assert!(matches!(
            error,
            AssetCatalogError::UnsupportedVersion { .. }
        ));
    }
}
