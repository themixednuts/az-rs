//! Native source-metadata sidecar (`azoth.source-meta/v1`).
//!
//! # The contract
//!
//! Every native source file the asset processor scans may be accompanied by a
//! sibling metadata sidecar. The sidecar carries out-of-band identity facts
//! that cannot live inside the source payload itself — most importantly the
//! *preserved* [`AssetId`] an imported asset must keep so existing references
//! resolve after an offline converter remaps its source path.
//!
//! This module is the single, authoritative definition of that contract. It is
//! the exact shape offline importers write and builders and the processor read
//! — there is no second reader.
//!
//! ## File name
//!
//! For a native source at `<dir>/<file>`, the sidecar is
//! `<dir>/<file>[SOURCE_META_SIDECAR_SUFFIX]`, i.e. the full source file name
//! (extension included) with `.azmeta.json` appended:
//!
//! ```text
//! rendering/textures/items/weapons/sword_diff.dds
//! rendering/textures/items/weapons/sword_diff.dds.azmeta.json
//! ```
//!
//! The sidecar is never itself treated as a source asset; the scan skips any
//! file whose name ends in [`SOURCE_META_SIDECAR_SUFFIX`].
//!
//! ## Shape
//!
//! Canonical UTF-8 JSON:
//!
//! ```json
//! {
//!   "spec": "azoth.source-meta/v1",
//!   "preserved_asset_id": { "guid": "aaf4c61d-dcc5-58a2-9abe-de0f3b482cd9", "sub_id": 0 }
//! }
//! ```
//!
//! - `spec` (required) — must equal [`SOURCE_META_SPEC`]. A mismatch is a hard
//!   error so a stale or foreign sidecar can never be silently ignored.
//! - `preserved_asset_id` (optional; may be `null` or omitted) — the identity
//!   the native source must keep. `guid` is a lowercase hyphenated RFC-4122
//!   string; `sub_id` is a JSON number.
//! - `preserved_source_guid` (optional; mutually exclusive with
//!   `preserved_asset_id`) — the source GUID for a source that emits several
//!   products and therefore has no single primary product identity.
//! ## Semantics
//!
//! When the scan records a source asset it resolves the source's identity GUID
//! as follows (see [`resolve_source_asset_guid`]):
//!
//! - if a sidecar is present with `preserved_asset_id`, the source identity
//!   uses `preserved_asset_id.guid` verbatim — this is how an imported legacy
//!   asset keeps its upstream catalog id even when its native source path
//!   differs from the legacy path;
//! - if a sidecar instead carries `preserved_source_guid`, that GUID is used
//!   verbatim while every product keeps the stable sub-ID authored by its
//!   builder;
//! - otherwise the identity GUID is `AZ::Uuid::CreateName` over the normalized
//!   source path (`az_asset_builder::source_guid`), matching Lumberyard's
//!   path-derived `AssetId.guid`.
//!
//! The complete `preserved_asset_id` identifies the one primary runtime product
//! that represents the imported source. The scan assigns its GUID to the source;
//! the worker carries its sub-ID into the process request, where the owning
//! builder explicitly constructs that primary product from the preserved
//! identity. Secondary products keep builder-assigned sub-IDs. A preserved GUID
//! and a path-derived GUID never collide because the former is supplied
//! out-of-band and the latter is a SHA-1 name hash of the path.

// The public contract types intentionally carry the `Source*Meta` names the
// crate root re-exports; the module-name overlap is by design.
#![allow(clippy::module_name_repetitions)]

use std::path::{Path, PathBuf};

use crate::source_guid;
use az_core::AssetId;
use az_filesystem::{SafeJoinError, safe_join};
use serde::{Deserialize, Serialize};

/// File-name suffix that identifies a source-metadata sidecar.
pub const SOURCE_META_SIDECAR_SUFFIX: &str = ".azmeta.json";

/// Spec identifier carried by every source-metadata sidecar.
pub const SOURCE_META_SPEC: &str = "azoth.source-meta/v1";

/// Parsed `azoth.source-meta/v1` sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAssetMeta {
    /// Spec identifier, always [`SOURCE_META_SPEC`].
    pub spec: String,
    /// The identity the native source must keep, when it was assigned one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_asset_id: Option<AssetId>,
    /// Source identity for a multi-product source with no primary product.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_source_guid: Option<uuid::Uuid>,
}

impl SourceAssetMeta {
    /// A sidecar that only preserves an identity.
    #[must_use]
    pub fn preserving(preserved_asset_id: AssetId) -> Self {
        Self {
            spec: SOURCE_META_SPEC.to_string(),
            preserved_asset_id: Some(preserved_asset_id),
            preserved_source_guid: None,
        }
    }

    /// A sidecar that preserves one multi-product source GUID.
    #[must_use]
    pub fn preserving_source_guid(preserved_source_guid: uuid::Uuid) -> Self {
        Self {
            spec: SOURCE_META_SPEC.to_string(),
            preserved_asset_id: None,
            preserved_source_guid: Some(preserved_source_guid),
        }
    }

    /// A sidecar for a source that has no catalog identity to preserve.
    #[must_use]
    pub fn uncataloged() -> Self {
        Self {
            spec: SOURCE_META_SPEC.to_string(),
            preserved_asset_id: None,
            preserved_source_guid: None,
        }
    }

    /// The source GUID declared by either valid identity shape.
    #[must_use]
    pub fn preserved_guid(&self) -> Option<uuid::Uuid> {
        self.preserved_asset_id
            .map(|asset_id| asset_id.guid)
            .or(self.preserved_source_guid)
    }
}

/// Serialize compact canonical JSON followed by one newline.
///
/// The stable field order comes from [`SourceAssetMeta`]. Optional identity
/// fields are omitted when absent so offline importers produce the same bytes.
///
/// # Errors
///
/// Returns a serialization error if the metadata cannot be encoded.
pub fn serialize_source_asset_meta(meta: &SourceAssetMeta) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec(meta)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Sidecar read/parse failure.
#[derive(Debug, thiserror::Error)]
pub enum SourceMetaError {
    /// The referenced source path was not safely relative to its source root.
    #[error("source path `{path}` is not safely relative to its source root: {source}")]
    UnsafeSourcePath {
        path: String,
        #[source]
        source: SafeJoinError,
    },
    /// The sidecar file could not be read.
    #[error("{0}")]
    Read(std::io::Error),
    /// The sidecar was not valid `azoth.source-meta/v1`.
    #[error("{0}")]
    Parse(String),
}

/// The sidecar path for a native source file.
#[must_use]
pub fn source_meta_sidecar_path(source_file: &Path) -> PathBuf {
    let mut sidecar = source_file.as_os_str().to_os_string();
    sidecar.push(SOURCE_META_SIDECAR_SUFFIX);
    PathBuf::from(sidecar)
}

/// Read and validate the metadata sidecar for a native source file.
///
/// Returns `Ok(None)` when no sidecar exists.
///
/// # Errors
///
/// Returns [`SourceMetaError`] when the sidecar exists but cannot be read or is
/// not valid `azoth.source-meta/v1` (I/O failure, malformed JSON, or a `spec`
/// mismatch).
pub fn read_source_asset_meta(
    source_file: &Path,
) -> Result<Option<SourceAssetMeta>, SourceMetaError> {
    let sidecar = source_meta_sidecar_path(source_file);
    let bytes = match std::fs::read(&sidecar) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(SourceMetaError::Read(error)),
    };
    let meta: SourceAssetMeta = serde_json::from_slice(&bytes)
        .map_err(|error| SourceMetaError::Parse(error.to_string()))?;
    if meta.spec != SOURCE_META_SPEC {
        return Err(SourceMetaError::Parse(format!(
            "unexpected spec `{}`, expected `{SOURCE_META_SPEC}`",
            meta.spec
        )));
    }
    if meta.preserved_asset_id.is_some() && meta.preserved_source_guid.is_some() {
        return Err(SourceMetaError::Parse(
            "preserved_asset_id and preserved_source_guid are mutually exclusive".to_owned(),
        ));
    }
    Ok(Some(meta))
}

/// Resolve the identity GUID for a scanned source asset.
///
/// Uses the sidecar's preserved GUID when present, otherwise the deterministic
/// `AZ::Uuid::CreateName` of the normalized source path.
#[must_use]
pub fn resolve_source_asset_guid(source_path: &str, meta: Option<&SourceAssetMeta>) -> uuid::Uuid {
    meta.and_then(SourceAssetMeta::preserved_guid)
        .unwrap_or_else(|| source_guid(source_path))
}

/// Resolve the product [`AssetId`] a builder should embed for a referenced source.
///
/// This is the cook-time equivalent of Lumberyard
/// `AssetCatalog::GetAssetIdByPath`: return the identity the asset processor
/// already assigned that source, not `GenerateAssetIdTEMP` (`CreateName` of the
/// hint path). The processor records that identity in the sibling azmeta
/// sidecar; when no sidecar exists it falls back to the same `CreateName` rule
/// the scan uses.
///
/// `product_sub_id` is used when the sidecar preserves only a source GUID (or
/// there is no sidecar). A `preserved_asset_id` is returned verbatim, including
/// its sub-id.
///
/// # Errors
///
/// Returns [`SourceMetaError`] when `source_path` is not safely relative to
/// `source_root`, or when a sidecar exists but cannot be read or is not valid
/// `azoth.source-meta/v1`.
pub fn resolve_referenced_product_id(
    source_root: &Path,
    source_path: &str,
    product_sub_id: u32,
) -> Result<AssetId, SourceMetaError> {
    let source_file = safe_join(source_root, source_path).map_err(|source| {
        SourceMetaError::UnsafeSourcePath {
            path: source_path.to_owned(),
            source,
        }
    })?;
    let meta = read_source_asset_meta(&source_file)?;
    if let Some(asset_id) = meta.as_ref().and_then(|meta| meta.preserved_asset_id) {
        return Ok(asset_id);
    }
    Ok(AssetId::new(
        resolve_source_asset_guid(source_path, meta.as_ref()),
        product_sub_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_path_appends_suffix_to_full_source_name() {
        assert_eq!(
            source_meta_sidecar_path(Path::new("a/b/sword.dds")),
            PathBuf::from(format!("a/b/sword.dds{SOURCE_META_SIDECAR_SUFFIX}"))
        );
    }

    #[test]
    fn preserved_id_is_used_when_present_else_create_name() {
        let path = "textures/items/weapons/sword_diff.dds";
        let preserved = AssetId::new(uuid::Uuid::from_u128(0x1234_5678_9abc_def0), 0);
        let meta = SourceAssetMeta::preserving(preserved);

        // With a preserved id, the source identity keeps it verbatim.
        assert_eq!(resolve_source_asset_guid(path, Some(&meta)), preserved.guid);

        // Without one, the identity is create_name(normalize(path)).
        assert_eq!(resolve_source_asset_guid(path, None), source_guid(path));

        // The two schemes never collide.
        assert_ne!(
            resolve_source_asset_guid(path, Some(&meta)),
            source_guid(path)
        );
    }

    #[test]
    fn referenced_product_id_uses_preserved_asset_id_else_create_name() {
        let temp = tempfile::tempdir().unwrap();
        let path = "cage/playeractions_flail.actionlist.ron";
        let source = temp.path().join(path);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"source").unwrap();

        assert_eq!(
            resolve_referenced_product_id(temp.path(), path, 0).unwrap(),
            crate::source_asset_id(path, 0)
        );

        let preserved = AssetId::new(uuid::Uuid::from_u128(0xb389_f04f_b9dc_5a1c), 17);
        std::fs::write(
            source_meta_sidecar_path(&source),
            serde_json::to_vec(&SourceAssetMeta::preserving(preserved)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            resolve_referenced_product_id(temp.path(), path, 0).unwrap(),
            preserved
        );
        assert_ne!(preserved, crate::source_asset_id(path, 0));
    }

    #[test]
    fn referenced_product_id_rejects_unsafe_source_paths() {
        let temp = tempfile::tempdir().unwrap();

        let absolute = std::env::temp_dir().join("outside/action.actionlist.ron");
        let absolute = absolute.to_string_lossy();
        for path in [
            "../outside/action.actionlist.ron",
            "/outside/action.actionlist.ron",
            absolute.as_ref(),
        ] {
            assert!(matches!(
                resolve_referenced_product_id(temp.path(), path, 0),
                Err(SourceMetaError::UnsafeSourcePath {
                    path: rejected,
                    ..
                }) if rejected == path
            ));
        }
    }

    #[test]
    fn reads_sidecar_from_disk_and_reports_absent_and_foreign() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("sword.dds");
        std::fs::write(&source, b"dds bytes").unwrap();

        // No sidecar -> None (source mints via create_name downstream).
        assert!(read_source_asset_meta(&source).unwrap().is_none());

        // A preserving sidecar written by an offline importer is read verbatim.
        let preserved = AssetId::new(uuid::Uuid::from_u128(0xabcd), 0);
        std::fs::write(
            source_meta_sidecar_path(&source),
            serde_json::to_vec(&SourceAssetMeta::preserving(preserved)).unwrap(),
        )
        .unwrap();
        let meta = read_source_asset_meta(&source).unwrap().unwrap();
        assert_eq!(meta.preserved_asset_id, Some(preserved));

        // A foreign spec is a hard error, never silently ignored.
        std::fs::write(
            source_meta_sidecar_path(&source),
            br#"{"spec":"some.other/v9"}"#,
        )
        .unwrap();
        assert!(matches!(
            read_source_asset_meta(&source),
            Err(SourceMetaError::Parse(_))
        ));
    }

    #[test]
    fn json_round_trips_and_rejects_foreign_spec() {
        let meta = SourceAssetMeta::preserving(AssetId::new(uuid::Uuid::from_u128(7), 3));
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: SourceAssetMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, meta);

        // An omitted preserved id parses to None.
        let minimal: SourceAssetMeta =
            serde_json::from_str(r#"{"spec":"azoth.source-meta/v1"}"#).unwrap();
        assert!(minimal.preserved_asset_id.is_none());
        assert!(minimal.preserved_source_guid.is_none());
    }

    #[test]
    fn serializer_writes_canonical_compact_json_with_trailing_newline() {
        let preserved = AssetId::new(
            uuid::Uuid::parse_str("11112222-3333-4444-5555-666677778888").unwrap(),
            7,
        );

        assert_eq!(
            serialize_source_asset_meta(&SourceAssetMeta::preserving(preserved)).unwrap(),
            br#"{"spec":"azoth.source-meta/v1","preserved_asset_id":{"guid":"11112222-3333-4444-5555-666677778888","sub_id":7}}
"#
        );
        assert_eq!(
            serialize_source_asset_meta(&SourceAssetMeta::uncataloged()).unwrap(),
            b"{\"spec\":\"azoth.source-meta/v1\"}\n"
        );
    }

    #[test]
    fn source_guid_identity_has_no_primary_product_sub_id() {
        let guid = uuid::Uuid::from_u128(0x1183_5fa2_8e74_5b52_9de9_b7a9_93c9_6746);
        let meta = SourceAssetMeta::preserving_source_guid(guid);

        assert_eq!(meta.preserved_guid(), Some(guid));
        assert_eq!(meta.preserved_asset_id, None);
        assert_eq!(
            resolve_source_asset_guid("prefabs/source.prefabs.ron", Some(&meta)),
            guid
        );
    }

    #[test]
    fn reader_rejects_two_source_identity_authorities() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.prefabs.ron");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(
            source_meta_sidecar_path(&source),
            br#"{"spec":"azoth.source-meta/v1","preserved_asset_id":{"guid":"11835fa2-8e74-5b52-9de9-b7a993c96746","sub_id":1},"preserved_source_guid":"11835fa2-8e74-5b52-9de9-b7a993c96746"}"#,
        )
        .unwrap();

        assert!(matches!(
            read_source_asset_meta(&source),
            Err(SourceMetaError::Parse(message)) if message.contains("mutually exclusive")
        ));
    }
}
