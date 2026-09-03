//! Release package manifest format.
//!
//! This is the first immutable package-layer artifact produced from the
//! asset-processor current products. It records package policy and
//! validated product rows, but does not contain product bytes.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::catalog::{AssetCatalogPathRegistration, AssetTreePath, ProductDependency};

/// `AZPKGM` manifest schema version.
///
/// Pinned to 1 pre-release for the same reason as [`ASSET_CATALOG_VERSION`]:
/// the reader fails closed on mismatch with no migration path, so format
/// changes are answered by reprocessing rather than by incrementing.
///
/// [`ASSET_CATALOG_VERSION`]: crate::catalog::ASSET_CATALOG_VERSION
pub const PACKAGE_MANIFEST_VERSION: u32 = 1;
pub const PACKAGE_MANIFEST_MAGIC: &[u8; 8] = b"AZPKGM\0\0";
pub const PACKAGE_CONTENT_HASH_BYTES: usize = 32;
pub const PACKAGE_RELEASE_ID_BYTES: usize = 32;
pub const PACKAGE_CONTAINER_LOOSE: &str = "loose";
pub const PACKAGE_CONTAINER_AZPACK: &str = "azpack";
pub const PACKAGE_CONTAINER_PAK: &str = "pak";
pub const PACKAGE_COMPRESSION_NONE: &str = "none";
pub const PACKAGE_COMPRESSION_OODLE: &str = "oodle";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    pub version: u32,
    pub profile: PackageManifestProfile,
    pub entries: Vec<PackageManifestEntry>,
}

impl PackageManifest {
    /// # Errors
    ///
    /// Returns [`PackageManifestError`] when `profile` or `entries` fails
    /// manifest validation.
    pub fn new(
        profile: PackageManifestProfile,
        mut entries: Vec<PackageManifestEntry>,
    ) -> Result<Self, PackageManifestError> {
        validate_profile(&profile)?;
        entries.sort_by(|left, right| {
            left.product_path
                .cmp(&right.product_path)
                .then(left.path_registration.cmp(&right.path_registration))
                .then(left.source_asset_guid.cmp(&right.source_asset_guid))
                .then(left.sub_id.cmp(&right.sub_id))
                .then(left.asset_type.cmp(&right.asset_type))
        });
        validate_entries(&entries)?;
        Ok(Self {
            version: PACKAGE_MANIFEST_VERSION,
            profile,
            entries,
        })
    }

    #[inline]
    #[must_use]
    pub fn entries(&self) -> &[PackageManifestEntry] {
        &self.entries
    }

    /// Return one entry for each physical payload path.
    ///
    /// Validation guarantees that logical entries sharing a path describe the
    /// same bytes. Sorting places the registered lookup owner first, so this
    /// iterator preserves its identity in payload indexes when one exists.
    pub fn physical_payload_entries(&self) -> impl Iterator<Item = &PackageManifestEntry> {
        let mut previous_path: Option<&AssetTreePath> = None;
        self.entries.iter().filter(move |entry| {
            let unique = previous_path != Some(&entry.product_path);
            previous_path = Some(&entry.product_path);
            unique
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifestProfile {
    pub name: String,
    pub asset_platform: String,
    pub cargo_profile: String,
    pub container: String,
    pub compression: String,
    pub oodle_compressor: Option<String>,
    pub oodle_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifestEntry {
    pub product_path: AssetTreePath,
    /// Whether this logical product owns by-path catalog lookup for the shared
    /// physical payload.
    #[serde(default)]
    pub path_registration: AssetCatalogPathRegistration,
    /// Additional lookup paths for this same product.
    #[serde(default)]
    pub catalog_aliases: Vec<AssetTreePath>,
    pub asset_type: Uuid,
    pub sub_id: u32,
    pub product_format: String,
    pub product_format_version: u32,
    pub content_hash: [u8; PACKAGE_CONTENT_HASH_BYTES],
    pub byte_len: u64,
    pub source_asset_guid: Uuid,
    pub source_path: AssetTreePath,
    pub job_key: String,
    /// Runtime product dependencies for this product, in deterministic order.
    #[serde(default)]
    pub dependencies: Vec<ProductDependency>,
}

impl PackageManifestEntry {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        product_path: impl Into<AssetTreePath>,
        asset_type: Uuid,
        sub_id: u32,
        product_format: impl Into<String>,
        product_format_version: u32,
        content_hash: [u8; PACKAGE_CONTENT_HASH_BYTES],
        byte_len: u64,
        source_asset_guid: Uuid,
        source_path: impl Into<AssetTreePath>,
        job_key: impl Into<String>,
    ) -> Self {
        Self {
            product_path: product_path.into(),
            path_registration: AssetCatalogPathRegistration::Registered,
            catalog_aliases: Vec::new(),
            asset_type,
            sub_id,
            product_format: product_format.into(),
            product_format_version,
            content_hash,
            byte_len,
            source_asset_guid,
            source_path: source_path.into(),
            job_key: job_key.into(),
            dependencies: Vec::new(),
        }
    }

    /// Attach additional lookup paths for this same product.
    #[must_use]
    pub fn with_catalog_aliases(
        mut self,
        aliases: impl IntoIterator<Item = impl Into<AssetTreePath>>,
    ) -> Self {
        self.catalog_aliases = aliases.into_iter().map(Into::into).collect();
        self.catalog_aliases.sort();
        self
    }

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
    /// The list is normalized to a deterministic order so identical inputs
    /// always serialize identically.
    #[must_use]
    pub fn with_dependencies(mut self, mut dependencies: Vec<ProductDependency>) -> Self {
        dependencies.sort();
        self.dependencies = dependencies;
        self
    }
}

/// # Errors
///
/// Returns [`PackageManifestError`] when `manifest` fails validation, an
/// entry field exceeds its wire-format limits, or the writer returns an I/O
/// error.
pub fn write_package_manifest(
    manifest: &PackageManifest,
    mut writer: impl Write,
) -> Result<(), PackageManifestError> {
    validate_profile(&manifest.profile)?;
    validate_entries(&manifest.entries)?;

    writer.write_all(PACKAGE_MANIFEST_MAGIC)?;
    write_u32(&mut writer, PACKAGE_MANIFEST_VERSION)?;
    write_profile(&mut writer, &manifest.profile)?;
    write_u32(
        &mut writer,
        u32::try_from(manifest.entries.len()).map_err(|_| {
            PackageManifestError::TooManyEntries {
                count: manifest.entries.len(),
            }
        })?,
    )?;
    for entry in &manifest.entries {
        write_uuid(&mut writer, entry.source_asset_guid)?;
        write_uuid(&mut writer, entry.asset_type)?;
        write_u32(&mut writer, entry.sub_id)?;
        write_u32(&mut writer, entry.product_format_version)?;
        write_u64(&mut writer, entry.byte_len)?;
        writer.write_all(&entry.content_hash)?;
        write_text(&mut writer, &entry.product_format)?;
        write_text(&mut writer, entry.product_path.as_str())?;
        write_u32(&mut writer, entry.path_registration as u32)?;
        write_text(&mut writer, entry.source_path.as_str())?;
        write_text(&mut writer, &entry.job_key)?;
        write_u32(
            &mut writer,
            u32::try_from(entry.catalog_aliases.len()).map_err(|_| {
                PackageManifestError::TooManyEntries {
                    count: entry.catalog_aliases.len(),
                }
            })?,
        )?;
        for alias in &entry.catalog_aliases {
            write_text(&mut writer, alias.as_str())?;
        }
        write_u32(
            &mut writer,
            u32::try_from(entry.dependencies.len()).map_err(|_| {
                PackageManifestError::TooManyEntries {
                    count: entry.dependencies.len(),
                }
            })?,
        )?;
        for dependency in &entry.dependencies {
            write_uuid(&mut writer, dependency.id.guid)?;
            write_u32(&mut writer, dependency.id.sub_id)?;
            write_uuid(&mut writer, dependency.asset_type)?;
            write_optional_text(&mut writer, dependency.hint.as_deref())?;
        }
    }
    Ok(())
}

/// # Errors
///
/// Returns [`PackageManifestError`] when the stream has a bad magic, an
/// unsupported version, malformed records, or the decoded manifest fails
/// validation.
pub fn read_package_manifest(
    mut reader: impl Read,
) -> Result<PackageManifest, PackageManifestError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != PACKAGE_MANIFEST_MAGIC {
        return Err(PackageManifestError::BadMagic { found: magic });
    }

    let version = read_u32(&mut reader)?;
    if version != PACKAGE_MANIFEST_VERSION {
        return Err(PackageManifestError::UnsupportedVersion {
            version,
            expected: PACKAGE_MANIFEST_VERSION,
        });
    }

    let profile = read_profile(&mut reader)?;
    let entry_count = read_u32(&mut reader)? as usize;
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        let source_asset_guid = read_uuid(&mut reader)?;
        let asset_type = read_uuid(&mut reader)?;
        let sub_id = read_u32(&mut reader)?;
        let product_format_version = read_u32(&mut reader)?;
        let byte_len = read_u64(&mut reader)?;
        let mut content_hash = [0u8; PACKAGE_CONTENT_HASH_BYTES];
        reader.read_exact(&mut content_hash)?;
        let product_format = read_text(&mut reader)?;
        let product_path = read_text(&mut reader)?;
        let path_registration_raw = read_u32(&mut reader)?;
        let path_registration = match path_registration_raw {
            0 => AssetCatalogPathRegistration::Registered,
            1 => AssetCatalogPathRegistration::AssetIdOnly,
            value => return Err(PackageManifestError::InvalidPathRegistration { value }),
        };
        let source_path = read_text(&mut reader)?;
        let job_key = read_text(&mut reader)?;
        let alias_count = read_u32(&mut reader)? as usize;
        let mut catalog_aliases = Vec::with_capacity(alias_count);
        for _ in 0..alias_count {
            catalog_aliases.push(read_text(&mut reader)?);
        }
        let dependency_count = read_u32(&mut reader)? as usize;
        let mut dependencies = Vec::with_capacity(dependency_count);
        for _ in 0..dependency_count {
            let dependency_guid = read_uuid(&mut reader)?;
            let dependency_sub_id = read_u32(&mut reader)?;
            let dependency_asset_type = read_uuid(&mut reader)?;
            let hint = read_optional_text(&mut reader)?;
            let mut dependency = ProductDependency::new(
                crate::AssetId::new(dependency_guid, dependency_sub_id),
                dependency_asset_type,
            );
            dependency.hint = hint;
            dependencies.push(dependency);
        }
        entries.push(
            PackageManifestEntry::new(
                product_path,
                asset_type,
                sub_id,
                product_format,
                product_format_version,
                content_hash,
                byte_len,
                source_asset_guid,
                source_path,
                job_key,
            )
            .with_path_registration(path_registration)
            .with_catalog_aliases(catalog_aliases)
            .with_dependencies(dependencies),
        );
    }

    PackageManifest::new(profile, entries)
}

/// # Errors
///
/// Returns [`PackageManifestError`] when `value` is not exactly
/// [`PACKAGE_CONTENT_HASH_BYTES`] * 2 hex digits.
pub fn parse_package_content_hash_hex(
    value: &str,
) -> Result<[u8; PACKAGE_CONTENT_HASH_BYTES], PackageManifestError> {
    if value.len() != PACKAGE_CONTENT_HASH_BYTES * 2 {
        return Err(PackageManifestError::InvalidContentHash {
            value: value.to_string(),
        });
    }

    let mut bytes = [0u8; PACKAGE_CONTENT_HASH_BYTES];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_value(chunk[0]).ok_or_else(|| PackageManifestError::InvalidContentHash {
            value: value.to_string(),
        })?;
        let low = hex_value(chunk[1]).ok_or_else(|| PackageManifestError::InvalidContentHash {
            value: value.to_string(),
        })?;
        bytes[index] = (high << 4) | low;
    }
    Ok(bytes)
}

#[must_use]
pub fn format_package_content_hash_hex(hash: &[u8; PACKAGE_CONTENT_HASH_BYTES]) -> String {
    format_hex(hash)
}

/// # Errors
///
/// Returns [`PackageManifestError`] when `manifest` fails to serialize
/// (e.g. an entry field exceeds its wire-format limits).
pub fn package_manifest_release_id(
    manifest: &PackageManifest,
) -> Result<[u8; PACKAGE_RELEASE_ID_BYTES], PackageManifestError> {
    let mut bytes = Vec::new();
    write_package_manifest(manifest, &mut bytes)?;
    Ok(*blake3::hash(&bytes).as_bytes())
}

#[must_use]
pub fn format_package_release_id_hex(release_id: &[u8; PACKAGE_RELEASE_ID_BYTES]) -> String {
    format_hex(release_id)
}

#[derive(Debug, Error)]
pub enum PackageManifestError {
    #[error("bad package manifest magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported package manifest version {version}, expected {expected}")]
    UnsupportedVersion { version: u32, expected: u32 },
    #[error("package manifest has too many entries: {count}")]
    TooManyEntries { count: usize },
    #[error("{field} cannot be empty")]
    EmptyText { field: &'static str },
    #[error("{field} is too long: {byte_len} bytes")]
    TextTooLong {
        field: &'static str,
        byte_len: usize,
    },
    #[error("{field} `{path}` must be an asset-tree relative path")]
    InvalidRelativePath { field: &'static str, path: String },
    #[error("entry `{path}` has nil source asset guid")]
    NilSourceAssetGuid { path: String },
    #[error("entry `{path}` has nil asset type")]
    NilAssetType { path: String },
    #[error("entry `{path}` has invalid product format version {version}")]
    InvalidProductFormatVersion { path: String, version: u32 },
    #[error("duplicate package product path `{path}`")]
    DuplicateProductPath { path: String },
    #[error("package manifest has unknown catalog path-registration value {value}")]
    InvalidPathRegistration { value: u32 },
    #[error("asset-id-only package product `{path}` cannot declare catalog aliases")]
    AssetIdOnlyWithAliases { path: String },
    #[error(
        "logical products {first_asset_guid}:{first_sub_id} and {asset_guid}:{sub_id} share physical path `{path}` but disagree on its backing payload"
    )]
    SharedProductBackingMismatch {
        path: String,
        first_asset_guid: Uuid,
        first_sub_id: u32,
        asset_guid: Uuid,
        sub_id: u32,
    },
    #[error("duplicate package asset id {asset_guid}:{sub_id}")]
    DuplicateAssetId { asset_guid: Uuid, sub_id: u32 },
    #[error("invalid package content hash `{value}`")]
    InvalidContentHash { value: String },
    #[error(
        "package profile `{profile}` has unsupported compression `{compression}` (expected `none` or `oodle`)"
    )]
    UnsupportedCompression {
        profile: String,
        compression: String,
    },
    #[error(
        "package profile `{profile}` has unsupported container `{container}` (expected `loose`, `azpack`, or `pak`)"
    )]
    UnsupportedContainer { profile: String, container: String },
    #[error(
        "package profile `{profile}` does not support container `{container}` with compression `{compression}`"
    )]
    UnsupportedContainerCompression {
        profile: String,
        container: String,
        compression: String,
    },
    #[error("package profile `{profile}` compression `{compression}` requires Oodle {field}")]
    MissingCompressionSetting {
        profile: String,
        compression: String,
        field: &'static str,
    },
    #[error("package profile `{profile}` compression `{compression}` must not set Oodle {field}")]
    UnexpectedCompressionSetting {
        profile: String,
        compression: String,
        field: &'static str,
    },
    #[error("invalid UTF-8 text: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

fn write_profile(
    writer: &mut impl Write,
    profile: &PackageManifestProfile,
) -> Result<(), PackageManifestError> {
    write_text(writer, &profile.name)?;
    write_text(writer, &profile.asset_platform)?;
    write_text(writer, &profile.cargo_profile)?;
    write_text(writer, &profile.container)?;
    write_text(writer, &profile.compression)?;
    write_optional_text(writer, profile.oodle_compressor.as_deref())?;
    write_optional_text(writer, profile.oodle_effort.as_deref())?;
    Ok(())
}

fn read_profile(reader: &mut impl Read) -> Result<PackageManifestProfile, PackageManifestError> {
    Ok(PackageManifestProfile {
        name: read_text(reader)?,
        asset_platform: read_text(reader)?,
        cargo_profile: read_text(reader)?,
        container: read_text(reader)?,
        compression: read_text(reader)?,
        oodle_compressor: read_optional_text(reader)?,
        oodle_effort: read_optional_text(reader)?,
    })
}

fn validate_profile(profile: &PackageManifestProfile) -> Result<(), PackageManifestError> {
    validate_required_text("profile name", &profile.name)?;
    validate_required_text("profile asset platform", &profile.asset_platform)?;
    validate_required_text("profile cargo profile", &profile.cargo_profile)?;
    validate_required_text("profile container", &profile.container)?;
    validate_required_text("profile compression", &profile.compression)?;
    if let Some(value) = &profile.oodle_compressor {
        validate_required_text("profile Oodle compressor", value)?;
    }
    if let Some(value) = &profile.oodle_effort {
        validate_required_text("profile Oodle effort", value)?;
    }
    validate_profile_compression_policy(profile)?;
    validate_profile_container_policy(profile)?;
    Ok(())
}

fn validate_profile_compression_policy(
    profile: &PackageManifestProfile,
) -> Result<(), PackageManifestError> {
    match profile.compression.as_str() {
        PACKAGE_COMPRESSION_NONE => {
            if profile.oodle_compressor.is_some() {
                return Err(PackageManifestError::UnexpectedCompressionSetting {
                    profile: profile.name.clone(),
                    compression: profile.compression.clone(),
                    field: "compressor",
                });
            }
            if profile.oodle_effort.is_some() {
                return Err(PackageManifestError::UnexpectedCompressionSetting {
                    profile: profile.name.clone(),
                    compression: profile.compression.clone(),
                    field: "effort",
                });
            }
            Ok(())
        }
        PACKAGE_COMPRESSION_OODLE => {
            if profile.oodle_compressor.is_none() {
                return Err(PackageManifestError::MissingCompressionSetting {
                    profile: profile.name.clone(),
                    compression: profile.compression.clone(),
                    field: "compressor",
                });
            }
            if profile.oodle_effort.is_none() {
                return Err(PackageManifestError::MissingCompressionSetting {
                    profile: profile.name.clone(),
                    compression: profile.compression.clone(),
                    field: "effort",
                });
            }
            Ok(())
        }
        _ => Err(PackageManifestError::UnsupportedCompression {
            profile: profile.name.clone(),
            compression: profile.compression.clone(),
        }),
    }
}

fn validate_profile_container_policy(
    profile: &PackageManifestProfile,
) -> Result<(), PackageManifestError> {
    match profile.container.as_str() {
        PACKAGE_CONTAINER_LOOSE => {
            if profile.compression == PACKAGE_COMPRESSION_OODLE {
                return Err(PackageManifestError::UnsupportedContainerCompression {
                    profile: profile.name.clone(),
                    container: profile.container.clone(),
                    compression: profile.compression.clone(),
                });
            }
            Ok(())
        }
        PACKAGE_CONTAINER_AZPACK | PACKAGE_CONTAINER_PAK => Ok(()),
        _ => Err(PackageManifestError::UnsupportedContainer {
            profile: profile.name.clone(),
            container: profile.container.clone(),
        }),
    }
}

fn validate_entries(entries: &[PackageManifestEntry]) -> Result<(), PackageManifestError> {
    let mut catalog_paths = BTreeSet::new();
    let mut physical_paths =
        std::collections::BTreeMap::<&AssetTreePath, &PackageManifestEntry>::new();
    let mut asset_ids = BTreeSet::new();

    for entry in entries {
        validate_asset_tree_path("product path", entry.product_path.as_str())?;
        for alias in &entry.catalog_aliases {
            validate_asset_tree_path("catalog alias", alias.as_str())?;
        }
        validate_asset_tree_path("source path", entry.source_path.as_str())?;
        validate_required_text("product format", &entry.product_format)?;
        validate_required_text("job key", &entry.job_key)?;

        if entry.source_asset_guid == Uuid::nil() {
            return Err(PackageManifestError::NilSourceAssetGuid {
                path: entry.product_path.to_string(),
            });
        }
        if entry.asset_type == Uuid::nil() {
            return Err(PackageManifestError::NilAssetType {
                path: entry.product_path.to_string(),
            });
        }
        if entry.product_format_version == 0 {
            return Err(PackageManifestError::InvalidProductFormatVersion {
                path: entry.product_path.to_string(),
                version: entry.product_format_version,
            });
        }

        if !entry.path_registration.is_registered() && !entry.catalog_aliases.is_empty() {
            return Err(PackageManifestError::AssetIdOnlyWithAliases {
                path: entry.product_path.to_string(),
            });
        }
        if entry.path_registration.is_registered() {
            if !catalog_paths.insert(entry.product_path.as_str().to_string()) {
                return Err(PackageManifestError::DuplicateProductPath {
                    path: entry.product_path.to_string(),
                });
            }
            for alias in &entry.catalog_aliases {
                if !catalog_paths.insert(alias.as_str().to_string()) {
                    return Err(PackageManifestError::DuplicateProductPath {
                        path: alias.to_string(),
                    });
                }
            }
        }
        if let Some(first) = physical_paths.get(&entry.product_path) {
            if !same_package_payload(first, entry) {
                return Err(PackageManifestError::SharedProductBackingMismatch {
                    path: entry.product_path.to_string(),
                    first_asset_guid: first.source_asset_guid,
                    first_sub_id: first.sub_id,
                    asset_guid: entry.source_asset_guid,
                    sub_id: entry.sub_id,
                });
            }
        } else {
            physical_paths.insert(&entry.product_path, entry);
        }
        if !asset_ids.insert((entry.source_asset_guid, entry.sub_id)) {
            return Err(PackageManifestError::DuplicateAssetId {
                asset_guid: entry.source_asset_guid,
                sub_id: entry.sub_id,
            });
        }
    }

    Ok(())
}

fn same_package_payload(left: &PackageManifestEntry, right: &PackageManifestEntry) -> bool {
    left.asset_type == right.asset_type
        && left.product_format == right.product_format
        && left.product_format_version == right.product_format_version
        && left.content_hash == right.content_hash
        && left.byte_len == right.byte_len
        && left.dependencies == right.dependencies
}

fn validate_required_text(field: &'static str, value: &str) -> Result<(), PackageManifestError> {
    if value.trim().is_empty() {
        Err(PackageManifestError::EmptyText { field })
    } else {
        Ok(())
    }
}

fn validate_asset_tree_path(field: &'static str, value: &str) -> Result<(), PackageManifestError> {
    if value.trim().is_empty() {
        return Err(PackageManifestError::EmptyText { field });
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
                return Err(PackageManifestError::InvalidRelativePath {
                    field,
                    path: value.to_string(),
                });
            }
        }
    }

    if has_normal_component {
        Ok(())
    } else {
        Err(PackageManifestError::InvalidRelativePath {
            field,
            path: value.to_string(),
        })
    }
}

fn write_text(writer: &mut impl Write, value: &str) -> Result<(), PackageManifestError> {
    let bytes = value.as_bytes();
    write_u32(
        writer,
        u32::try_from(bytes.len()).map_err(|_| PackageManifestError::TextTooLong {
            field: "text",
            byte_len: bytes.len(),
        })?,
    )?;
    writer.write_all(bytes)?;
    Ok(())
}

fn read_text(reader: &mut impl Read) -> Result<String, PackageManifestError> {
    let byte_len = read_u32(reader)? as usize;
    let mut bytes = vec![0u8; byte_len];
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes)?)
}

fn write_optional_text(
    writer: &mut impl Write,
    value: Option<&str>,
) -> Result<(), PackageManifestError> {
    if let Some(value) = value {
        writer.write_all(&[1])?;
        write_text(writer, value)
    } else {
        writer.write_all(&[0])?;
        Ok(())
    }
}

fn read_optional_text(reader: &mut impl Read) -> Result<Option<String>, PackageManifestError> {
    let mut tag = [0u8; 1];
    reader.read_exact(&mut tag)?;
    match tag[0] {
        0 => Ok(None),
        _ => Ok(Some(read_text(reader)?)),
    }
}

fn write_uuid(writer: &mut impl Write, value: Uuid) -> std::io::Result<()> {
    writer.write_all(value.as_bytes())
}

fn read_uuid(reader: &mut impl Read) -> Result<Uuid, PackageManifestError> {
    let mut bytes = [0u8; 16];
    reader.read_exact(&mut bytes)?;
    Ok(Uuid::from_bytes(bytes))
}

fn write_u32(writer: &mut impl Write, value: u32) -> std::io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u64(writer: &mut impl Write, value: u64) -> std::io::Result<()> {
    writer.write_all(&value.to_le_bytes())
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

const fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn format_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_profile() -> PackageManifestProfile {
        PackageManifestProfile {
            name: "pc-release".to_string(),
            asset_platform: "pc".to_string(),
            cargo_profile: "release".to_string(),
            container: "pak".to_string(),
            compression: "oodle".to_string(),
            oodle_compressor: Some("kraken".to_string()),
            oodle_effort: Some("normal".to_string()),
        }
    }

    fn valid_entry(product_path: &str) -> PackageManifestEntry {
        PackageManifestEntry::new(
            product_path,
            Uuid::from_bytes([2; 16]),
            7,
            "az.test.raw",
            1,
            parse_package_content_hash_hex(&"ab".repeat(32)).unwrap(),
            42,
            Uuid::from_bytes([1; 16]),
            "prefabs/source.prefab.ron",
            "BuildPrefab",
        )
    }

    #[test]
    fn package_manifest_round_trips_binary_format() {
        let manifest =
            PackageManifest::new(valid_profile(), vec![valid_entry("products/a.azbin")]).unwrap();
        let mut bytes = Vec::new();
        write_package_manifest(&manifest, &mut bytes).unwrap();

        assert_eq!(&bytes[..8], PACKAGE_MANIFEST_MAGIC);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            PACKAGE_MANIFEST_VERSION
        );
        let decoded = read_package_manifest(bytes.as_slice()).unwrap();
        assert_eq!(decoded, manifest);
    }

    #[test]
    fn package_manifest_preserves_shared_logical_ids_with_one_payload() {
        let mut id_only = valid_entry("products/shared.azbin");
        id_only.sub_id = 3;
        id_only.path_registration = AssetCatalogPathRegistration::AssetIdOnly;
        let mut registered = valid_entry("products/shared.azbin");
        registered.sub_id = 7;
        let manifest = PackageManifest::new(valid_profile(), vec![id_only, registered]).unwrap();

        assert_eq!(manifest.entries().len(), 2);
        assert_eq!(manifest.physical_payload_entries().count(), 1);
        assert_eq!(
            manifest
                .physical_payload_entries()
                .next()
                .unwrap()
                .path_registration,
            AssetCatalogPathRegistration::Registered
        );

        let mut bytes = Vec::new();
        write_package_manifest(&manifest, &mut bytes).unwrap();
        assert_eq!(read_package_manifest(bytes.as_slice()).unwrap(), manifest);
    }

    #[test]
    fn package_manifest_round_trips_product_dependencies() {
        let entry = valid_entry("products/a.azbin").with_dependencies(vec![
            ProductDependency::new(
                crate::AssetId::new(Uuid::from_bytes([9; 16]), 2),
                Uuid::from_bytes([8; 16]),
            )
            .with_hint("@assets@/products/b.azbin"),
            ProductDependency::new(
                crate::AssetId::new(Uuid::from_bytes([4; 16]), 0),
                Uuid::from_bytes([5; 16]),
            ),
        ]);
        let manifest = PackageManifest::new(valid_profile(), vec![entry]).unwrap();

        let mut bytes = Vec::new();
        write_package_manifest(&manifest, &mut bytes).unwrap();
        let decoded = read_package_manifest(bytes.as_slice()).unwrap();

        assert_eq!(decoded, manifest);
        assert_eq!(decoded.entries[0].dependencies.len(), 2);
        // Deterministic order: sorted by AssetId.
        assert_eq!(
            decoded.entries[0].dependencies[0].id.guid,
            Uuid::from_bytes([4; 16])
        );
        assert_eq!(
            decoded.entries[0].dependencies[1].hint.as_deref(),
            Some("@assets@/products/b.azbin")
        );
    }

    #[test]
    fn package_manifest_rejects_version_in_magic() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"AZPKGM01");
        bytes.extend_from_slice(&PACKAGE_MANIFEST_VERSION.to_le_bytes());

        let error = read_package_manifest(bytes.as_slice()).unwrap_err();
        assert!(matches!(error, PackageManifestError::BadMagic { .. }));
    }

    #[test]
    fn package_manifest_validates_safe_unique_entries() {
        let duplicate_path = PackageManifest::new(
            valid_profile(),
            vec![
                valid_entry("products/a.azbin"),
                valid_entry("products/a.azbin"),
            ],
        )
        .unwrap_err();
        assert!(matches!(
            duplicate_path,
            PackageManifestError::DuplicateProductPath { .. }
        ));

        let mut bad_path = valid_entry("products/a.azbin");
        bad_path.source_path = AssetTreePath::new("../source.prefab.ron");
        let error = PackageManifest::new(valid_profile(), vec![bad_path]).unwrap_err();
        assert!(matches!(
            error,
            PackageManifestError::InvalidRelativePath { .. }
        ));

        let mut missing_format = valid_entry("products/b.azbin");
        missing_format.product_format.clear();
        let error = PackageManifest::new(valid_profile(), vec![missing_format]).unwrap_err();
        assert!(matches!(
            error,
            PackageManifestError::EmptyText {
                field: "product format"
            }
        ));

        let mut bad_format_version = valid_entry("products/c.azbin");
        bad_format_version.product_format_version = 0;
        let error = PackageManifest::new(valid_profile(), vec![bad_format_version]).unwrap_err();
        assert!(matches!(
            error,
            PackageManifestError::InvalidProductFormatVersion { .. }
        ));
    }

    #[test]
    fn package_manifest_validates_compression_policy_at_manifest_boundary() {
        let mut none_with_oodle = valid_profile();
        none_with_oodle.compression = "none".to_string();
        let error = PackageManifest::new(none_with_oodle, vec![valid_entry("products/a.azbin")])
            .unwrap_err();
        assert!(matches!(
            error,
            PackageManifestError::UnexpectedCompressionSetting {
                field: "compressor",
                ..
            }
        ));

        let mut oodle_missing_effort = valid_profile();
        oodle_missing_effort.oodle_effort = None;
        let error =
            PackageManifest::new(oodle_missing_effort, vec![valid_entry("products/a.azbin")])
                .unwrap_err();
        assert!(matches!(
            error,
            PackageManifestError::MissingCompressionSetting {
                field: "effort",
                ..
            }
        ));

        let mut unsupported = valid_profile();
        unsupported.compression = "brotli".to_string();
        let error =
            PackageManifest::new(unsupported, vec![valid_entry("products/a.azbin")]).unwrap_err();
        assert!(matches!(
            error,
            PackageManifestError::UnsupportedCompression { .. }
        ));
    }

    #[test]
    fn package_manifest_validates_container_policy_at_manifest_boundary() {
        let mut unsupported_container = valid_profile();
        unsupported_container.container = "bundle".to_string();
        let error =
            PackageManifest::new(unsupported_container, vec![valid_entry("products/a.azbin")])
                .unwrap_err();
        assert!(matches!(
            error,
            PackageManifestError::UnsupportedContainer { .. }
        ));

        let mut loose_oodle = valid_profile();
        loose_oodle.container = "loose".to_string();
        let error =
            PackageManifest::new(loose_oodle, vec![valid_entry("products/a.azbin")]).unwrap_err();
        assert!(matches!(
            error,
            PackageManifestError::UnsupportedContainerCompression { .. }
        ));
    }

    #[test]
    fn package_content_hash_hex_parses_and_formats() {
        let text = "0a".repeat(32);
        let hash = parse_package_content_hash_hex(&text).unwrap();
        assert_eq!(format_package_content_hash_hex(&hash), text);
        assert!(matches!(
            parse_package_content_hash_hex("not-a-hash"),
            Err(PackageManifestError::InvalidContentHash { .. })
        ));
    }

    #[test]
    fn package_release_id_fingerprints_manifest_bytes() {
        let manifest =
            PackageManifest::new(valid_profile(), vec![valid_entry("products/a.azbin")]).unwrap();

        let release_id = package_manifest_release_id(&manifest).unwrap();
        let mut bytes = Vec::new();
        write_package_manifest(&manifest, &mut bytes).unwrap();

        assert_eq!(release_id, *blake3::hash(&bytes).as_bytes());
        assert_eq!(
            format_package_release_id_hex(&release_id).len(),
            PACKAGE_RELEASE_ID_BYTES * 2
        );

        let changed =
            PackageManifest::new(valid_profile(), vec![valid_entry("products/b.azbin")]).unwrap();
        assert_ne!(release_id, package_manifest_release_id(&changed).unwrap());
    }
}
