//! Package payload writers for build products.
//!
//! The public `PackageManifest` is a build receipt/input. This module consumes
//! that manifest plus validated `Cache/<platform>` products and emits a native
//! chunked package, loose product tree, or CryPak-compatible ZIP container.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use az_filesystem::safe_join;
use rayon::prelude::*;
use thiserror::Error;

use crate::azpack::{
    AZPACK_DEFAULT_CHUNK_SIZE, AZPACK_INDEX_FILE_NAME, AzPackChunkCompression, AzPackIndex,
    AzPackIndexChunk, AzPackIndexEntry, AzPackIndexError, AzPackIndexProfile, write_azpack_index,
};
use crate::package::{
    PACKAGE_COMPRESSION_NONE, PACKAGE_COMPRESSION_OODLE, PACKAGE_CONTAINER_AZPACK,
    PACKAGE_CONTAINER_LOOSE, PACKAGE_CONTAINER_PAK, PACKAGE_CONTENT_HASH_BYTES, PackageManifest,
    PackageManifestEntry, PackageManifestProfile, format_package_content_hash_hex,
};

const ZIP_METHOD_STORED: u16 = 0;
const ZIP_METHOD_OODLE: u16 = 15;
const ZIP_LOCAL_FILE_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const ZIP_CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0201_4b50;
const ZIP64_END_OF_CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0606_4b50;
const ZIP64_END_OF_CENTRAL_DIRECTORY_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
const ZIP_END_OF_CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0605_4b50;
const ZIP64_EXTRA_FIELD_ID: u16 = 0x0001;
const ZIP_VERSION_NEEDED_STORED: u16 = 20;
const ZIP_VERSION_NEEDED_ZIP64: u16 = 45;
const ZIP_UTF8_FLAG: u16 = 1 << 11;
const DOS_DATE_1980_01_01: u16 = 33;
const DOS_TIME_MIDNIGHT: u16 = 0;
const PAK_PREPARE_BATCH_SIZE: usize = 64;
#[cfg(feature = "oodle")]
const OODLE_SEEK_CHUNK_LEN: u32 = 1 << 20;

pub trait PackageContainerMarker {
    const NAME: &'static str;
}

pub trait PackageCompressionMarker {
    const NAME: &'static str;
}

pub trait StreamablePackageContainer: PackageContainerMarker {}

pub trait ChunkAddressablePackageContainer: StreamablePackageContainer {
    const DEFAULT_CHUNK_SIZE: usize;
}

pub trait PatchFriendlyPackageContainer: PackageContainerMarker {}

pub trait ParallelPreparedPackageContainer: PackageContainerMarker {}

pub trait CompatibilityPackageContainer: PackageContainerMarker {
    const ECOSYSTEM: &'static str;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LooseContainer;

impl PackageContainerMarker for LooseContainer {
    const NAME: &'static str = PACKAGE_CONTAINER_LOOSE;
}

impl StreamablePackageContainer for LooseContainer {}
impl PatchFriendlyPackageContainer for LooseContainer {}
impl ParallelPreparedPackageContainer for LooseContainer {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AzPackContainer;

impl PackageContainerMarker for AzPackContainer {
    const NAME: &'static str = PACKAGE_CONTAINER_AZPACK;
}

impl StreamablePackageContainer for AzPackContainer {}

impl ChunkAddressablePackageContainer for AzPackContainer {
    const DEFAULT_CHUNK_SIZE: usize = AZPACK_DEFAULT_CHUNK_SIZE;
}

impl PatchFriendlyPackageContainer for AzPackContainer {}
impl ParallelPreparedPackageContainer for AzPackContainer {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PakContainer;

impl PackageContainerMarker for PakContainer {
    const NAME: &'static str = PACKAGE_CONTAINER_PAK;
}

impl ParallelPreparedPackageContainer for PakContainer {}

impl CompatibilityPackageContainer for PakContainer {
    const ECOSYSTEM: &'static str = "lumberyard/o3de/crypak";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoCompression;

impl PackageCompressionMarker for NoCompression {
    const NAME: &'static str = "none";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OodleCompression;

impl PackageCompressionMarker for OodleCompression {
    const NAME: &'static str = "oodle";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackagePayloadKind {
    Loose,
    AzPack,
    Pak,
}

impl PackagePayloadKind {
    #[must_use]
    pub const fn container_name(self) -> &'static str {
        match self {
            Self::Loose => LooseContainer::NAME,
            Self::AzPack => AzPackContainer::NAME,
            Self::Pak => PakContainer::NAME,
        }
    }

    #[must_use]
    pub const fn from_container_name(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case(LooseContainer::NAME) {
            Some(Self::Loose)
        } else if value.eq_ignore_ascii_case(AzPackContainer::NAME) {
            Some(Self::AzPack)
        } else if value.eq_ignore_ascii_case(PakContainer::NAME) {
            Some(Self::Pak)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn capabilities(self) -> PackageBackendCapabilities {
        match self {
            Self::Loose => PackageBackendCapabilities {
                streamable: true,
                chunk_addressable: false,
                patch_friendly: true,
                parallel_prepared: true,
                compatibility_ecosystem: None,
            },
            Self::AzPack => PackageBackendCapabilities {
                streamable: true,
                chunk_addressable: true,
                patch_friendly: true,
                parallel_prepared: true,
                compatibility_ecosystem: None,
            },
            Self::Pak => PackageBackendCapabilities {
                streamable: false,
                chunk_addressable: false,
                patch_friendly: false,
                parallel_prepared: true,
                compatibility_ecosystem: Some(PakContainer::ECOSYSTEM),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageCompressionKind {
    None,
    Oodle,
}

impl PackageCompressionKind {
    #[must_use]
    pub const fn compression_name(self) -> &'static str {
        match self {
            Self::None => NoCompression::NAME,
            Self::Oodle => OodleCompression::NAME,
        }
    }

    #[must_use]
    pub const fn from_compression_name(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case(PACKAGE_COMPRESSION_NONE) {
            Some(Self::None)
        } else if value.eq_ignore_ascii_case(PACKAGE_COMPRESSION_OODLE) {
            Some(Self::Oodle)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackagePayloadPolicy {
    pub container: PackagePayloadKind,
    pub compression: PackageCompressionKind,
}

impl PackagePayloadPolicy {
    /// # Errors
    ///
    /// Returns [`PackagePayloadError::UnsupportedPolicy`] when `profile`
    /// names an unknown container/compression, or combines a container and
    /// compression that are not supported together.
    pub fn from_profile(profile: &PackageManifestProfile) -> Result<Self, PackagePayloadError> {
        let container =
            PackagePayloadKind::from_container_name(&profile.container).ok_or_else(|| {
                PackagePayloadError::UnsupportedPolicy {
                    profile: profile.name.clone(),
                    container: profile.container.clone(),
                    compression: profile.compression.clone(),
                }
            })?;
        let compression = PackageCompressionKind::from_compression_name(&profile.compression)
            .ok_or_else(|| PackagePayloadError::UnsupportedPolicy {
                profile: profile.name.clone(),
                container: profile.container.clone(),
                compression: profile.compression.clone(),
            })?;
        let policy = Self {
            container,
            compression,
        };
        if matches!(
            (policy.container, policy.compression),
            (PackagePayloadKind::Loose, PackageCompressionKind::Oodle)
        ) {
            return Err(PackagePayloadError::UnsupportedPolicy {
                profile: profile.name.clone(),
                container: profile.container.clone(),
                compression: profile.compression.clone(),
            });
        }
        Ok(policy)
    }

    #[must_use]
    pub const fn capabilities(self) -> PackageBackendCapabilities {
        self.container.capabilities()
    }
}

// Each field is an independently meaningful backend capability flag read by
// name at call sites (`capabilities().chunk_addressable`, etc.), not a
// same-typed parameter list a caller could transpose; a bitflags/enum
// refactor would not improve clarity here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageBackendCapabilities {
    pub streamable: bool,
    pub chunk_addressable: bool,
    pub patch_friendly: bool,
    pub parallel_prepared: bool,
    pub compatibility_ecosystem: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagePayloadLayout {
    pub kind: PackagePayloadKind,
    pub mount_root: PathBuf,
    pub payload_path: PathBuf,
    pub catalog_path: PathBuf,
}

/// # Errors
///
/// Returns [`PackagePayloadError`] when `profile` fails
/// [`PackagePayloadPolicy::from_profile`].
pub fn package_payload_layout(
    output_root: &Path,
    profile: &PackageManifestProfile,
) -> Result<PackagePayloadLayout, PackagePayloadError> {
    match PackagePayloadPolicy::from_profile(profile)?.container {
        PackagePayloadKind::Loose => {
            let root = output_root.join(LooseContainer::NAME);
            Ok(PackagePayloadLayout {
                kind: PackagePayloadKind::Loose,
                mount_root: root.clone(),
                payload_path: root.clone(),
                catalog_path: root.join(crate::ASSET_CATALOG_FILE_NAME),
            })
        }
        PackagePayloadKind::AzPack => {
            let root = output_root.join(AzPackContainer::NAME);
            Ok(PackagePayloadLayout {
                kind: PackagePayloadKind::AzPack,
                mount_root: root.clone(),
                payload_path: root.clone(),
                catalog_path: root.join(crate::ASSET_CATALOG_FILE_NAME),
            })
        }
        PackagePayloadKind::Pak => Ok(PackagePayloadLayout {
            kind: PackagePayloadKind::Pak,
            mount_root: output_root.to_path_buf(),
            payload_path: output_root.join(format!("{}.pak", safe_package_name(&profile.name))),
            catalog_path: output_root.join(crate::ASSET_CATALOG_FILE_NAME),
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagePayloadReceipt {
    pub kind: PackagePayloadKind,
    pub path: PathBuf,
    pub mount_root: PathBuf,
    pub payload_path: PathBuf,
    pub catalog_path: PathBuf,
    pub entry_count: usize,
    pub uncompressed_bytes: u64,
    pub payload_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct PackagePayloadWriteRequest<'a> {
    pub manifest: &'a PackageManifest,
    pub cache_root: &'a Path,
    pub output_root: &'a Path,
}

impl<'a> PackagePayloadWriteRequest<'a> {
    #[must_use]
    pub const fn new(
        manifest: &'a PackageManifest,
        cache_root: &'a Path,
        output_root: &'a Path,
    ) -> Self {
        Self {
            manifest,
            cache_root,
            output_root,
        }
    }
}

pub trait PackagePayloadWriter {
    type Container: PackageContainerMarker;
    type Compression: PackageCompressionMarker;

    /// # Errors
    ///
    /// Returns [`PackagePayloadError`] when the request's manifest or
    /// products are invalid, or writing the payload fails.
    fn write(
        &self,
        request: &PackagePayloadWriteRequest<'_>,
    ) -> Result<PackagePayloadReceipt, PackagePayloadError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LoosePayloadWriter;

impl PackagePayloadWriter for LoosePayloadWriter {
    type Container = LooseContainer;
    type Compression = NoCompression;

    fn write(
        &self,
        request: &PackagePayloadWriteRequest<'_>,
    ) -> Result<PackagePayloadReceipt, PackagePayloadError> {
        let layout = package_payload_layout_for_writer::<LooseContainer, NoCompression>(request)?;
        let output_root = &layout.payload_path;
        fs::create_dir_all(output_root)?;

        let entries = request
            .manifest
            .physical_payload_entries()
            .collect::<Vec<_>>();
        entries.par_iter().try_for_each(|entry| {
            copy_validated_loose_product(request.cache_root, output_root, entry)
        })?;

        Ok(PackagePayloadReceipt {
            kind: PackagePayloadKind::Loose,
            path: layout.payload_path.clone(),
            mount_root: layout.mount_root,
            payload_path: layout.payload_path,
            catalog_path: layout.catalog_path,
            entry_count: entries.len(),
            uncompressed_bytes: manifest_uncompressed_bytes(request.manifest),
            payload_bytes: manifest_uncompressed_bytes(request.manifest),
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StoredPackageCompressor;

impl PackageEntryCompressor for StoredPackageCompressor {
    type Marker = NoCompression;

    fn compress_entry(
        &self,
        _entry: &PackageManifestEntry,
        bytes: Vec<u8>,
    ) -> Result<CompressedPackageEntry, PackagePayloadError> {
        Ok(CompressedPackageEntry {
            method: PackageCompressionMethod::Stored,
            bytes,
        })
    }
}

#[derive(Debug, Clone)]
#[cfg(feature = "oodle")]
pub struct OodlePackageCompressor {
    compressor: OodleCompressorKind,
    level: OodleCompressionLevel,
    seek_chunk_len: u32,
}

#[cfg(feature = "oodle")]
impl OodlePackageCompressor {
    /// # Errors
    ///
    /// Returns [`PackagePayloadError`] when `profile` is missing its Oodle
    /// compressor/effort settings, or either setting names an unknown value.
    pub fn from_profile(profile: &PackageManifestProfile) -> Result<Self, PackagePayloadError> {
        let compressor = profile.oodle_compressor.as_deref().ok_or_else(|| {
            PackagePayloadError::MissingOodleSetting {
                profile: profile.name.clone(),
                field: "compressor",
            }
        })?;
        let effort = profile.oodle_effort.as_deref().ok_or_else(|| {
            PackagePayloadError::MissingOodleSetting {
                profile: profile.name.clone(),
                field: "effort",
            }
        })?;

        Ok(Self {
            compressor: OodleCompressorKind::parse(&profile.name, compressor)?,
            level: OodleCompressionLevel::parse(&profile.name, effort)?,
            seek_chunk_len: OODLE_SEEK_CHUNK_LEN,
        })
    }

    #[must_use]
    pub const fn with_seek_chunk_len(mut self, seek_chunk_len: u32) -> Self {
        self.seek_chunk_len = seek_chunk_len;
        self
    }
}

#[cfg(feature = "oodle")]
impl PackageEntryCompressor for OodlePackageCompressor {
    type Marker = OodleCompression;

    fn compress_entry(
        &self,
        entry: &PackageManifestEntry,
        bytes: Vec<u8>,
    ) -> Result<CompressedPackageEntry, PackagePayloadError> {
        Ok(CompressedPackageEntry {
            method: PackageCompressionMethod::Oodle,
            bytes: self.compress_bytes(entry, &bytes)?,
        })
    }
}

#[cfg(feature = "oodle")]
impl OodlePackageCompressor {
    fn compress_bytes(
        &self,
        entry: &PackageManifestEntry,
        bytes: &[u8],
    ) -> Result<Vec<u8>, PackagePayloadError> {
        if bytes.is_empty() {
            return Ok(Vec::new());
        }

        let input_len =
            isize::try_from(bytes.len()).map_err(|_| PackagePayloadError::OodleInputTooLarge {
                product_path: entry.product_path.to_string(),
                byte_len: bytes.len() as u64,
            })?;
        let output_capacity = unsafe {
            oodle_sys::OodleLZ_GetCompressedBufferSizeNeeded(self.compressor.raw(), input_len)
        };
        if output_capacity <= 0 {
            return Err(PackagePayloadError::OodleBufferSizeFailed {
                product_path: entry.product_path.to_string(),
                input_len: bytes.len() as u64,
            });
        }

        // `output_capacity > 0` is checked above, so the widening cast to
        // `usize` cannot lose the sign.
        let mut output = vec![0_u8; output_capacity.cast_unsigned()];
        let options = oodle_compress_options(self.compressor, self.level, self.seek_chunk_len)?;
        let compressed_len = unsafe {
            oodle_sys::OodleLZ_Compress(
                self.compressor.raw(),
                bytes.as_ptr().cast(),
                input_len,
                output.as_mut_ptr().cast(),
                self.level.raw(),
                std::ptr::addr_of!(options),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            )
        };
        // `oodle_safe::FAILED` is a small fixed sentinel (currently 0); the
        // widening `u32 -> isize` comparison cannot wrap on any real target.
        #[allow(clippy::cast_possible_wrap)]
        let oodle_failed = oodle_safe::FAILED as isize;
        if compressed_len <= 0 || compressed_len == oodle_failed {
            return Err(PackagePayloadError::OodleCompressFailed {
                product_path: entry.product_path.to_string(),
                code: compressed_len,
            });
        }

        // `compressed_len > 0` is checked above, so the widening cast to
        // `usize` cannot lose the sign.
        output.truncate(compressed_len.cast_unsigned());
        Ok(output)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageCompressionMethod {
    Stored,
    Oodle,
}

impl PackageCompressionMethod {
    #[inline]
    const fn zip_code(self) -> u16 {
        match self {
            Self::Stored => ZIP_METHOD_STORED,
            Self::Oodle => ZIP_METHOD_OODLE,
        }
    }

    #[inline]
    const fn azpack_compression(self) -> AzPackChunkCompression {
        match self {
            Self::Stored => AzPackChunkCompression::Stored,
            Self::Oodle => AzPackChunkCompression::Oodle,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressedPackageEntry {
    pub method: PackageCompressionMethod,
    pub bytes: Vec<u8>,
}

pub trait PackageEntryCompressor: Sync {
    type Marker: PackageCompressionMarker;

    /// # Errors
    ///
    /// Returns [`PackagePayloadError`] when `bytes` fails to compress for
    /// `entry`.
    fn compress_entry(
        &self,
        entry: &PackageManifestEntry,
        bytes: Vec<u8>,
    ) -> Result<CompressedPackageEntry, PackagePayloadError>;
}

#[derive(Debug, Clone)]
pub struct AzPackPayloadWriter<C> {
    compressor: C,
    chunk_size: usize,
}

impl<C> AzPackPayloadWriter<C> {
    #[must_use]
    pub const fn new(compressor: C) -> Self {
        Self {
            compressor,
            chunk_size: AzPackContainer::DEFAULT_CHUNK_SIZE,
        }
    }

    #[must_use]
    pub const fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size;
        self
    }
}

impl<C> PackagePayloadWriter for AzPackPayloadWriter<C>
where
    C: PackageEntryCompressor,
{
    type Container = AzPackContainer;
    type Compression = C::Marker;

    fn write(
        &self,
        request: &PackagePayloadWriteRequest<'_>,
    ) -> Result<PackagePayloadReceipt, PackagePayloadError> {
        let layout = package_payload_layout_for_writer::<AzPackContainer, C::Marker>(request)?;
        if self.chunk_size == 0 || self.chunk_size > u32::MAX as usize {
            return Err(PackagePayloadError::InvalidAzPackChunkSize {
                chunk_size: self.chunk_size,
            });
        }

        let output_root = &layout.payload_path;
        fs::create_dir_all(output_root.join("chunks"))?;

        let entries = request
            .manifest
            .physical_payload_entries()
            .collect::<Vec<_>>()
            .par_iter()
            .map(|entry| {
                prepare_azpack_entry(
                    request.cache_root,
                    output_root,
                    entry,
                    &self.compressor,
                    self.chunk_size,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let index_path = output_root.join(AZPACK_INDEX_FILE_NAME);
        write_azpack_index_file(&index_path, request.manifest, &entries, self.chunk_size)?;
        prune_unreferenced_azpack_chunks(output_root, &entries)?;

        let mut unique_chunks = HashSet::new();
        let mut chunk_payload_bytes = 0_u64;
        for entry in &entries {
            for chunk in &entry.chunks {
                if unique_chunks.insert(chunk.encoded_hash) {
                    chunk_payload_bytes =
                        chunk_payload_bytes.saturating_add(u64::from(chunk.encoded_len));
                }
            }
        }
        let index_bytes = fs::metadata(&index_path)?.len();

        Ok(PackagePayloadReceipt {
            kind: PackagePayloadKind::AzPack,
            path: layout.payload_path.clone(),
            mount_root: layout.mount_root,
            payload_path: layout.payload_path,
            catalog_path: layout.catalog_path,
            entry_count: entries.len(),
            uncompressed_bytes: manifest_uncompressed_bytes(request.manifest),
            payload_bytes: chunk_payload_bytes.saturating_add(index_bytes),
        })
    }
}

#[derive(Debug, Clone)]
pub struct PakPayloadWriter<C> {
    compressor: C,
}

impl<C> PakPayloadWriter<C> {
    #[must_use]
    pub const fn new(compressor: C) -> Self {
        Self { compressor }
    }
}

impl<C> PackagePayloadWriter for PakPayloadWriter<C>
where
    C: PackageEntryCompressor,
{
    type Container = PakContainer;
    type Compression = C::Marker;

    fn write(
        &self,
        request: &PackagePayloadWriteRequest<'_>,
    ) -> Result<PackagePayloadReceipt, PackagePayloadError> {
        let layout = package_payload_layout_for_writer::<PakContainer, C::Marker>(request)?;
        let pak_path = layout.payload_path.clone();
        if let Some(parent) = pak_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let temp_path = pak_path.with_extension("pak.tmp");
        remove_file_if_exists(&temp_path)?;

        let write_result = (|| {
            let file = File::create(&temp_path)?;
            let mut writer = CountingWriter::new(BufWriter::new(file));
            let stats = write_pak_archive(&mut writer, request, &self.compressor)?;
            writer.flush()?;
            Ok::<PakArchiveStats, PackagePayloadError>(stats)
        })();

        let stats = match write_result {
            Ok(stats) => stats,
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                return Err(error);
            }
        };

        remove_file_if_exists(&pak_path)?;
        fs::rename(&temp_path, &pak_path).map_err(|source| PackagePayloadError::Promote {
            product_path: request.manifest.profile.name.clone(),
            from: temp_path,
            to: pak_path.clone(),
            source,
        })?;

        Ok(PackagePayloadReceipt {
            kind: PackagePayloadKind::Pak,
            path: pak_path.clone(),
            mount_root: layout.mount_root,
            payload_path: pak_path,
            catalog_path: layout.catalog_path,
            entry_count: request.manifest.physical_payload_entries().count(),
            uncompressed_bytes: stats.uncompressed_bytes,
            payload_bytes: stats.payload_bytes,
        })
    }
}

/// # Errors
///
/// Returns [`PackagePayloadError`] when the manifest's profile names an
/// unsupported or unbuildable container/compression policy, or the selected
/// backend fails to write the payload.
// With `oodle` off both Oodle arms correctly reduce to the same
// `OodleUnavailable` early return, so the lint fires and the expectation is
// fulfilled. With it on the arms genuinely differ, the lint does not fire, and
// an unconditional `expect` would itself be an unfulfilled-expectation error.
#[cfg_attr(
    not(feature = "oodle"),
    expect(
        clippy::match_same_arms,
        reason = "with `oodle` off both Oodle arms reduce to the same OodleUnavailable early return"
    )
)]
pub fn write_package_payload(
    request: PackagePayloadWriteRequest<'_>,
) -> Result<PackagePayloadReceipt, PackagePayloadError> {
    let policy = PackagePayloadPolicy::from_profile(&request.manifest.profile)?;

    match (policy.container, policy.compression) {
        (PackagePayloadKind::Loose, PackageCompressionKind::None) => {
            LoosePayloadWriter.write(&request)
        }
        (PackagePayloadKind::AzPack, PackageCompressionKind::None) => {
            AzPackPayloadWriter::new(StoredPackageCompressor).write(&request)
        }
        (PackagePayloadKind::AzPack, PackageCompressionKind::Oodle) => {
            #[cfg(not(feature = "oodle"))]
            return Err(PackagePayloadError::OodleUnavailable);
            #[cfg(feature = "oodle")]
            let compressor = OodlePackageCompressor::from_profile(&request.manifest.profile)?;
            #[cfg(feature = "oodle")]
            AzPackPayloadWriter::new(compressor).write(&request)
        }
        (PackagePayloadKind::Pak, PackageCompressionKind::None) => {
            PakPayloadWriter::new(StoredPackageCompressor).write(&request)
        }
        (PackagePayloadKind::Pak, PackageCompressionKind::Oodle) => {
            #[cfg(not(feature = "oodle"))]
            return Err(PackagePayloadError::OodleUnavailable);
            #[cfg(feature = "oodle")]
            let compressor = OodlePackageCompressor::from_profile(&request.manifest.profile)?;
            #[cfg(feature = "oodle")]
            PakPayloadWriter::new(compressor).write(&request)
        }
        (PackagePayloadKind::Loose, PackageCompressionKind::Oodle) => {
            Err(PackagePayloadError::UnsupportedPolicy {
                profile: request.manifest.profile.name.clone(),
                container: request.manifest.profile.container.clone(),
                compression: request.manifest.profile.compression.clone(),
            })
        }
    }
}

fn package_payload_layout_for_writer<C, M>(
    request: &PackagePayloadWriteRequest<'_>,
) -> Result<PackagePayloadLayout, PackagePayloadError>
where
    C: PackageContainerMarker,
    M: PackageCompressionMarker,
{
    let profile = &request.manifest.profile;
    if !profile.container.eq_ignore_ascii_case(C::NAME)
        || !profile.compression.eq_ignore_ascii_case(M::NAME)
    {
        return Err(PackagePayloadError::UnsupportedPolicy {
            profile: profile.name.clone(),
            container: profile.container.clone(),
            compression: profile.compression.clone(),
        });
    }
    package_payload_layout(request.output_root, profile)
}

#[derive(Debug, Error)]
pub enum PackagePayloadError {
    #[error(
        "package payload policy `{container}`/`{compression}` is not implemented for profile `{profile}`"
    )]
    UnsupportedPolicy {
        profile: String,
        container: String,
        compression: String,
    },

    #[error("Oodle package support is disabled; enable the `oodle` feature")]
    OodleUnavailable,

    #[error("package profile `{profile}` is missing Oodle {field}")]
    MissingOodleSetting {
        profile: String,
        field: &'static str,
    },

    #[error("package profile `{profile}` has unknown Oodle compressor `{compressor}`")]
    UnknownOodleCompressor { profile: String, compressor: String },

    #[error("package profile `{profile}` has unknown Oodle effort `{effort}`")]
    UnknownOodleEffort { profile: String, effort: String },

    #[error("package payload path `{product_path}` is invalid under `{root}`: {source}")]
    InvalidPath {
        product_path: String,
        root: PathBuf,
        #[source]
        source: az_filesystem::SafeJoinError,
    },

    #[error("package payload source `{product_path}` is missing at {path}")]
    MissingPayload { product_path: String, path: PathBuf },

    #[error("failed to copy package payload `{product_path}` from {from} to {to}: {source}")]
    Copy {
        product_path: String,
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to remove package payload `{product_path}` at {path}: {source}")]
    Remove {
        product_path: String,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to promote package payload `{product_path}` from {from} to {to}: {source}")]
    Promote {
        product_path: String,
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("package payload `{product_path}` at {path} has {actual} bytes, expected {expected}")]
    ByteLengthMismatch {
        product_path: String,
        path: PathBuf,
        expected: u64,
        actual: u64,
    },

    #[error("package payload `{product_path}` at {path} has hash {actual}, expected {expected}")]
    HashMismatch {
        product_path: String,
        path: PathBuf,
        expected: String,
        actual: String,
    },

    #[error(
        "package payload `{product_path}` is too large for in-memory Oodle compression: {byte_len} bytes"
    )]
    OodleInputTooLarge { product_path: String, byte_len: u64 },

    #[error("Oodle failed to size the compressed buffer for `{product_path}` ({input_len} bytes)")]
    OodleBufferSizeFailed {
        product_path: String,
        input_len: u64,
    },

    #[error("Oodle compression failed for `{product_path}` with code {code}")]
    OodleCompressFailed { product_path: String, code: isize },

    #[error("Oodle seek chunk length {seek_chunk_len} is invalid")]
    InvalidOodleSeekChunkLen { seek_chunk_len: u32 },

    #[error("azpack chunk size {chunk_size} is invalid")]
    InvalidAzPackChunkSize { chunk_size: usize },

    #[error("azpack chunk for `{product_path}` is too large: {byte_len} bytes")]
    AzPackChunkTooLarge {
        product_path: String,
        byte_len: usize,
    },

    #[error(transparent)]
    AzPackIndex(#[from] AzPackIndexError),

    #[error("pak entry name `{product_path}` is too long: {byte_len} bytes")]
    PakEntryNameTooLong {
        product_path: String,
        byte_len: usize,
    },

    #[error("pak extra field for `{product_path}` is too long: {byte_len} bytes")]
    PakExtraFieldTooLong {
        product_path: String,
        byte_len: usize,
    },

    #[error(transparent)]
    Io(#[from] io::Error),
}

fn copy_validated_loose_product(
    cache_root: &Path,
    output_root: &Path,
    entry: &PackageManifestEntry,
) -> Result<(), PackagePayloadError> {
    let source = safe_payload_join(cache_root, entry)?;
    if !source.is_file() {
        return Err(PackagePayloadError::MissingPayload {
            product_path: entry.product_path.to_string(),
            path: source,
        });
    }

    let destination = safe_payload_join(output_root, entry)?;
    let parent = destination.parent().unwrap_or(output_root);
    fs::create_dir_all(parent)?;

    let path_hash = crc32fast::hash(entry.product_path.as_str().as_bytes());
    let temp_path = parent.join(format!(
        ".azoth-package-{path_hash:08x}-{}.tmp",
        format_package_content_hash_hex(&entry.content_hash)
    ));
    remove_file_if_exists(&temp_path)?;

    fs::copy(&source, &temp_path).map_err(|source_error| PackagePayloadError::Copy {
        product_path: entry.product_path.to_string(),
        from: source.clone(),
        to: temp_path.clone(),
        source: source_error,
    })?;

    if let Err(error) = validate_payload_file(entry, &temp_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    remove_file_if_exists_for_entry(entry, &destination)?;
    fs::rename(&temp_path, &destination).map_err(|source| PackagePayloadError::Promote {
        product_path: entry.product_path.to_string(),
        from: temp_path,
        to: destination,
        source,
    })?;

    Ok(())
}

fn prepare_azpack_entry<C>(
    cache_root: &Path,
    package_root: &Path,
    entry: &PackageManifestEntry,
    compressor: &C,
    chunk_size: usize,
) -> Result<AzPackIndexEntry, PackagePayloadError>
where
    C: PackageEntryCompressor,
{
    let path = safe_payload_join(cache_root, entry)?;
    if !path.is_file() {
        return Err(PackagePayloadError::MissingPayload {
            product_path: entry.product_path.to_string(),
            path,
        });
    }

    let metadata = fs::metadata(&path)?;
    if metadata.len() != entry.byte_len {
        return Err(PackagePayloadError::ByteLengthMismatch {
            product_path: entry.product_path.to_string(),
            path,
            expected: entry.byte_len,
            actual: metadata.len(),
        });
    }

    let mut file = File::open(&path)?;
    let mut buffer = vec![0_u8; chunk_size];
    let mut content_hasher = blake3::Hasher::new();
    let mut chunks = Vec::new();
    let mut raw_offset = 0_u64;

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }

        let raw_bytes = &buffer[..read];
        content_hasher.update(raw_bytes);
        let raw_hash = *blake3::hash(raw_bytes).as_bytes();
        let compressed = compressor.compress_entry(entry, raw_bytes.to_vec())?;
        let encoded_hash = *blake3::hash(&compressed.bytes).as_bytes();
        let encoded_len = u32::try_from(compressed.bytes.len()).map_err(|_| {
            PackagePayloadError::AzPackChunkTooLarge {
                product_path: entry.product_path.to_string(),
                byte_len: compressed.bytes.len(),
            }
        })?;
        let raw_len =
            u32::try_from(read).map_err(|_| PackagePayloadError::AzPackChunkTooLarge {
                product_path: entry.product_path.to_string(),
                byte_len: read,
            })?;

        let chunk_path = azpack_chunk_path(&encoded_hash);
        let chunk_full_path = package_root.join(&chunk_path);
        write_azpack_chunk(entry, &chunk_full_path, &compressed.bytes, &encoded_hash)?;

        chunks.push(AzPackIndexChunk::new(
            raw_offset,
            raw_len,
            encoded_len,
            compressed.method.azpack_compression(),
            raw_hash,
            encoded_hash,
            chunk_path,
        ));
        raw_offset = raw_offset.saturating_add(read as u64);
    }

    let actual_hash = *content_hasher.finalize().as_bytes();
    if actual_hash != entry.content_hash {
        return Err(PackagePayloadError::HashMismatch {
            product_path: entry.product_path.to_string(),
            path,
            expected: format_package_content_hash_hex(&entry.content_hash),
            actual: format_package_content_hash_hex(&actual_hash),
        });
    }

    Ok(AzPackIndexEntry::new(
        entry.product_path.clone(),
        entry.source_path.clone(),
        entry.job_key.clone(),
        entry.source_asset_guid,
        entry.asset_type,
        entry.sub_id,
        entry.byte_len,
        entry.content_hash,
        chunks,
    ))
}

fn write_azpack_chunk(
    entry: &PackageManifestEntry,
    path: &Path,
    bytes: &[u8],
    expected_hash: &[u8; PACKAGE_CONTENT_HASH_BYTES],
) -> Result<(), PackagePayloadError> {
    if azpack_chunk_matches(path, bytes.len() as u64, expected_hash)? {
        return Ok(());
    }

    if path.exists() {
        remove_file_if_exists(path)?;
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let path_hash = crc32fast::hash(entry.product_path.as_str().as_bytes());
    let temp_path = path.with_extension(format!(
        "azchunk.{}.{path_hash:08x}.tmp",
        std::process::id()
    ));
    remove_file_if_exists(&temp_path)?;

    let write_result = (|| -> io::Result<()> {
        let mut file = File::create(&temp_path)?;
        file.write_all(bytes)?;
        file.flush()?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(PackagePayloadError::Promote {
            product_path: entry.product_path.to_string(),
            from: temp_path,
            to: path.to_path_buf(),
            source: error,
        });
    }

    match fs::rename(&temp_path, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temp_path);
            if !azpack_chunk_matches(path, bytes.len() as u64, expected_hash)? {
                remove_file_if_exists(path)?;
                fs::write(path, bytes)?;
            }
            Ok(())
        }
        Err(source) => {
            let _ = fs::remove_file(&temp_path);
            Err(PackagePayloadError::Promote {
                product_path: entry.product_path.to_string(),
                from: temp_path,
                to: path.to_path_buf(),
                source,
            })
        }
    }
}

fn azpack_chunk_matches(
    path: &Path,
    expected_len: u64,
    expected_hash: &[u8; PACKAGE_CONTENT_HASH_BYTES],
) -> io::Result<bool> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if metadata.len() != expected_len {
        return Ok(false);
    }
    Ok(hash_file(path)? == *expected_hash)
}

fn azpack_chunk_path(encoded_hash: &[u8; PACKAGE_CONTENT_HASH_BYTES]) -> String {
    let hash = format_package_content_hash_hex(encoded_hash);
    format!("chunks/{}/{}.azchunk", &hash[..2], hash)
}

fn write_azpack_index_file(
    index_path: &Path,
    manifest: &PackageManifest,
    entries: &[AzPackIndexEntry],
    chunk_size: usize,
) -> Result<(), PackagePayloadError> {
    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp_path = index_path.with_extension("azpack.index.tmp");
    remove_file_if_exists(&temp_path)?;

    let write_result = (|| -> Result<(), PackagePayloadError> {
        let mut writer = BufWriter::new(File::create(&temp_path)?);
        let index = AzPackIndex::new(
            AzPackIndexProfile::new(
                manifest.profile.name.clone(),
                manifest.profile.asset_platform.clone(),
                manifest.profile.compression.clone(),
            ),
            u32::try_from(chunk_size)
                .map_err(|_| PackagePayloadError::InvalidAzPackChunkSize { chunk_size })?,
            entries.to_vec(),
        )?;
        write_azpack_index(&index, &mut writer)?;
        writer.flush()?;
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    remove_file_if_exists(index_path)?;
    fs::rename(&temp_path, index_path).map_err(|source| PackagePayloadError::Promote {
        product_path: manifest.profile.name.clone(),
        from: temp_path,
        to: index_path.to_path_buf(),
        source,
    })
}

fn prune_unreferenced_azpack_chunks(
    package_root: &Path,
    entries: &[AzPackIndexEntry],
) -> io::Result<()> {
    let chunks_root = package_root.join("chunks");
    if !chunks_root.is_dir() {
        return Ok(());
    }

    let referenced = entries
        .iter()
        .flat_map(|entry| entry.chunks.iter().map(|chunk| chunk.chunk_path.as_str()))
        .collect::<HashSet<_>>();
    prune_azpack_chunk_dir(&chunks_root, &chunks_root, &referenced)?;
    Ok(())
}

fn prune_azpack_chunk_dir(root: &Path, dir: &Path, referenced: &HashSet<&str>) -> io::Result<bool> {
    let mut is_empty = true;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            if prune_azpack_chunk_dir(root, &path, referenced)? {
                fs::remove_dir(&path)?;
            } else {
                is_empty = false;
            }
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let package_relative = format!("chunks/{relative}");
            if package_relative.ends_with(".azchunk")
                && !referenced.contains(package_relative.as_str())
            {
                fs::remove_file(&path)?;
            } else {
                is_empty = false;
            }
        }
    }
    Ok(is_empty)
}

fn write_pak_archive<C>(
    writer: &mut CountingWriter<BufWriter<File>>,
    request: &PackagePayloadWriteRequest<'_>,
    compressor: &C,
) -> Result<PakArchiveStats, PackagePayloadError>
where
    C: PackageEntryCompressor,
{
    let payload_entries = request
        .manifest
        .physical_payload_entries()
        .collect::<Vec<_>>();
    let mut central_entries = Vec::with_capacity(payload_entries.len());
    let mut stats = PakArchiveStats::default();

    for chunk in payload_entries.chunks(PAK_PREPARE_BATCH_SIZE) {
        let prepared = chunk
            .par_iter()
            .map(|entry| prepare_pak_entry(request.cache_root, entry, compressor))
            .collect::<Result<Vec<_>, _>>()?;

        for entry in prepared {
            write_pak_local_entry(writer, &entry, &mut central_entries)?;
            stats.uncompressed_bytes = stats
                .uncompressed_bytes
                .saturating_add(entry.uncompressed_size);
            stats.payload_bytes = stats.payload_bytes.saturating_add(entry.compressed_size);
        }
    }

    write_pak_central_directory(writer, &central_entries)?;
    Ok(stats)
}

fn prepare_pak_entry<C>(
    cache_root: &Path,
    entry: &PackageManifestEntry,
    compressor: &C,
) -> Result<PreparedPakEntry, PackagePayloadError>
where
    C: PackageEntryCompressor,
{
    let bytes = read_validated_payload(cache_root, entry)?;
    let crc32 = crc32fast::hash(&bytes);
    let uncompressed_size = bytes.len() as u64;
    let compressed = compressor.compress_entry(entry, bytes)?;
    let compressed_size = compressed.bytes.len() as u64;

    Ok(PreparedPakEntry {
        name: entry.product_path.as_str().replace('\\', "/"),
        product_path: entry.product_path.to_string(),
        method: compressed.method,
        crc32,
        uncompressed_size,
        compressed_size,
        bytes: compressed.bytes,
    })
}

fn write_pak_local_entry(
    writer: &mut CountingWriter<BufWriter<File>>,
    entry: &PreparedPakEntry,
    central_entries: &mut Vec<ZipCentralEntry>,
) -> Result<(), PackagePayloadError> {
    let local_header_offset = writer.position();
    let name = checked_entry_name(entry)?;
    let extra = zip64_local_extra(entry)?;
    let version_needed = zip_version_needed(entry.requires_zip64_sizes());

    write_u32(writer, ZIP_LOCAL_FILE_HEADER_SIGNATURE)?;
    write_u16(writer, version_needed)?;
    write_u16(writer, ZIP_UTF8_FLAG)?;
    write_u16(writer, entry.method.zip_code())?;
    write_u16(writer, DOS_TIME_MIDNIGHT)?;
    write_u16(writer, DOS_DATE_1980_01_01)?;
    write_u32(writer, entry.crc32)?;
    write_zip_size_u32(writer, entry.compressed_size)?;
    write_zip_size_u32(writer, entry.uncompressed_size)?;
    // `checked_entry_name` already rejects names longer than `u16::MAX`.
    #[allow(clippy::cast_possible_truncation)]
    write_u16(writer, name.len() as u16)?;
    write_u16(writer, checked_extra_len(&entry.product_path, &extra)?)?;
    writer.write_all(name)?;
    writer.write_all(&extra)?;
    writer.write_all(&entry.bytes)?;

    central_entries.push(ZipCentralEntry {
        name: entry.name.clone(),
        product_path: entry.product_path.clone(),
        method: entry.method,
        crc32: entry.crc32,
        compressed_size: entry.compressed_size,
        uncompressed_size: entry.uncompressed_size,
        local_header_offset,
    });

    Ok(())
}

fn write_pak_central_directory(
    writer: &mut CountingWriter<BufWriter<File>>,
    central_entries: &[ZipCentralEntry],
) -> Result<(), PackagePayloadError> {
    let central_dir_offset = writer.position();
    let mut any_zip64_entry = false;

    for entry in central_entries {
        let name = checked_central_name(entry)?;
        let extra = zip64_central_extra(entry)?;
        any_zip64_entry |= entry.requires_zip64();
        let version_needed = zip_version_needed(entry.requires_zip64());

        write_u32(writer, ZIP_CENTRAL_DIRECTORY_SIGNATURE)?;
        write_u16(writer, version_needed)?;
        write_u16(writer, version_needed)?;
        write_u16(writer, ZIP_UTF8_FLAG)?;
        write_u16(writer, entry.method.zip_code())?;
        write_u16(writer, DOS_TIME_MIDNIGHT)?;
        write_u16(writer, DOS_DATE_1980_01_01)?;
        write_u32(writer, entry.crc32)?;
        write_zip_size_u32(writer, entry.compressed_size)?;
        write_zip_size_u32(writer, entry.uncompressed_size)?;
        // `checked_central_name` already rejects names longer than `u16::MAX`.
        #[allow(clippy::cast_possible_truncation)]
        write_u16(writer, name.len() as u16)?;
        write_u16(writer, checked_extra_len(&entry.product_path, &extra)?)?;
        write_u16(writer, 0)?;
        write_u16(writer, 0)?;
        write_u16(writer, 0)?;
        write_u32(writer, 0)?;
        write_zip_offset_u32(writer, entry.local_header_offset)?;
        writer.write_all(name)?;
        writer.write_all(&extra)?;
    }

    let central_dir_size = writer.position() - central_dir_offset;
    let needs_zip64_eocd = any_zip64_entry
        || central_entries.len() > u16::MAX as usize
        || central_dir_offset > u64::from(u32::MAX)
        || central_dir_size > u64::from(u32::MAX);

    if needs_zip64_eocd {
        write_zip64_end_of_central_directory(
            writer,
            central_entries.len() as u64,
            central_dir_size,
            central_dir_offset,
        )?;
    }

    write_end_of_central_directory(
        writer,
        central_entries.len(),
        central_dir_size,
        central_dir_offset,
    )?;
    Ok(())
}

fn write_zip64_end_of_central_directory(
    writer: &mut CountingWriter<BufWriter<File>>,
    entry_count: u64,
    central_dir_size: u64,
    central_dir_offset: u64,
) -> io::Result<()> {
    let zip64_eocd_offset = writer.position();

    write_u32(writer, ZIP64_END_OF_CENTRAL_DIRECTORY_SIGNATURE)?;
    write_u64(writer, 44)?;
    write_u16(writer, ZIP_VERSION_NEEDED_ZIP64)?;
    write_u16(writer, ZIP_VERSION_NEEDED_ZIP64)?;
    write_u32(writer, 0)?;
    write_u32(writer, 0)?;
    write_u64(writer, entry_count)?;
    write_u64(writer, entry_count)?;
    write_u64(writer, central_dir_size)?;
    write_u64(writer, central_dir_offset)?;

    write_u32(writer, ZIP64_END_OF_CENTRAL_DIRECTORY_LOCATOR_SIGNATURE)?;
    write_u32(writer, 0)?;
    write_u64(writer, zip64_eocd_offset)?;
    write_u32(writer, 1)?;

    Ok(())
}

fn write_end_of_central_directory(
    writer: &mut CountingWriter<BufWriter<File>>,
    entry_count: usize,
    central_dir_size: u64,
    central_dir_offset: u64,
) -> io::Result<()> {
    write_u32(writer, ZIP_END_OF_CENTRAL_DIRECTORY_SIGNATURE)?;
    write_u16(writer, 0)?;
    write_u16(writer, 0)?;
    write_u16(writer, zip_entry_count_u16(entry_count))?;
    write_u16(writer, zip_entry_count_u16(entry_count))?;
    write_zip_size_u32(writer, central_dir_size)?;
    write_zip_offset_u32(writer, central_dir_offset)?;
    write_u16(writer, 0)?;
    Ok(())
}

fn read_validated_payload(
    cache_root: &Path,
    entry: &PackageManifestEntry,
) -> Result<Vec<u8>, PackagePayloadError> {
    let path = safe_payload_join(cache_root, entry)?;
    if !path.is_file() {
        return Err(PackagePayloadError::MissingPayload {
            product_path: entry.product_path.to_string(),
            path,
        });
    }

    let metadata = fs::metadata(&path)?;
    if metadata.len() != entry.byte_len {
        return Err(PackagePayloadError::ByteLengthMismatch {
            product_path: entry.product_path.to_string(),
            path,
            expected: entry.byte_len,
            actual: metadata.len(),
        });
    }

    let bytes = fs::read(&path)?;
    if bytes.len() as u64 != entry.byte_len {
        return Err(PackagePayloadError::ByteLengthMismatch {
            product_path: entry.product_path.to_string(),
            path,
            expected: entry.byte_len,
            actual: bytes.len() as u64,
        });
    }

    let actual_hash = *blake3::hash(&bytes).as_bytes();
    if actual_hash != entry.content_hash {
        return Err(PackagePayloadError::HashMismatch {
            product_path: entry.product_path.to_string(),
            path,
            expected: format_package_content_hash_hex(&entry.content_hash),
            actual: format_package_content_hash_hex(&actual_hash),
        });
    }

    Ok(bytes)
}

fn validate_payload_file(
    entry: &PackageManifestEntry,
    path: &Path,
) -> Result<(), PackagePayloadError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() != entry.byte_len {
        return Err(PackagePayloadError::ByteLengthMismatch {
            product_path: entry.product_path.to_string(),
            path: path.to_path_buf(),
            expected: entry.byte_len,
            actual: metadata.len(),
        });
    }

    let actual_hash = hash_file(path)?;
    if actual_hash != entry.content_hash {
        return Err(PackagePayloadError::HashMismatch {
            product_path: entry.product_path.to_string(),
            path: path.to_path_buf(),
            expected: format_package_content_hash_hex(&entry.content_hash),
            actual: format_package_content_hash_hex(&actual_hash),
        });
    }

    Ok(())
}

fn hash_file(path: &Path) -> io::Result<[u8; PACKAGE_CONTENT_HASH_BYTES]> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(*hasher.finalize().as_bytes())
}

fn safe_payload_join(
    root: &Path,
    entry: &PackageManifestEntry,
) -> Result<PathBuf, PackagePayloadError> {
    safe_join(root, entry.product_path.as_str()).map_err(|source| {
        PackagePayloadError::InvalidPath {
            product_path: entry.product_path.to_string(),
            root: root.to_path_buf(),
            source,
        }
    })
}

fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_file_if_exists_for_entry(
    entry: &PackageManifestEntry,
    path: &Path,
) -> Result<(), PackagePayloadError> {
    remove_file_if_exists(path).map_err(|source| PackagePayloadError::Remove {
        product_path: entry.product_path.to_string(),
        path: path.to_path_buf(),
        source,
    })
}

fn checked_entry_name(entry: &PreparedPakEntry) -> Result<&[u8], PackagePayloadError> {
    checked_name_bytes(&entry.product_path, &entry.name)
}

fn checked_central_name(entry: &ZipCentralEntry) -> Result<&[u8], PackagePayloadError> {
    checked_name_bytes(&entry.product_path, &entry.name)
}

fn checked_name_bytes<'a>(
    product_path: &str,
    name: &'a str,
) -> Result<&'a [u8], PackagePayloadError> {
    let bytes = name.as_bytes();
    if bytes.len() > u16::MAX as usize {
        return Err(PackagePayloadError::PakEntryNameTooLong {
            product_path: product_path.to_string(),
            byte_len: bytes.len(),
        });
    }
    Ok(bytes)
}

fn checked_extra_len(product_path: &str, extra: &[u8]) -> Result<u16, PackagePayloadError> {
    u16::try_from(extra.len()).map_err(|_| PackagePayloadError::PakExtraFieldTooLong {
        product_path: product_path.to_string(),
        byte_len: extra.len(),
    })
}

fn zip64_local_extra(entry: &PreparedPakEntry) -> Result<Vec<u8>, PackagePayloadError> {
    if !entry.requires_zip64_sizes() {
        return Ok(Vec::new());
    }

    let mut payload = Vec::with_capacity(16);
    push_u64(&mut payload, entry.uncompressed_size);
    push_u64(&mut payload, entry.compressed_size);
    zip_extra_field(&entry.product_path, ZIP64_EXTRA_FIELD_ID, &payload)
}

fn zip64_central_extra(entry: &ZipCentralEntry) -> Result<Vec<u8>, PackagePayloadError> {
    if !entry.requires_zip64() {
        return Ok(Vec::new());
    }

    let mut payload = Vec::with_capacity(24);
    if entry.requires_zip64_sizes() {
        push_u64(&mut payload, entry.uncompressed_size);
        push_u64(&mut payload, entry.compressed_size);
    }
    if entry.local_header_offset > u64::from(u32::MAX) {
        push_u64(&mut payload, entry.local_header_offset);
    }

    zip_extra_field(&entry.product_path, ZIP64_EXTRA_FIELD_ID, &payload)
}

fn zip_extra_field(
    product_path: &str,
    header_id: u16,
    payload: &[u8],
) -> Result<Vec<u8>, PackagePayloadError> {
    let payload_len =
        u16::try_from(payload.len()).map_err(|_| PackagePayloadError::PakExtraFieldTooLong {
            product_path: product_path.to_string(),
            byte_len: payload.len(),
        })?;
    let mut extra = Vec::with_capacity(4 + payload.len());
    push_u16(&mut extra, header_id);
    push_u16(&mut extra, payload_len);
    extra.extend_from_slice(payload);
    Ok(extra)
}

const fn zip_version_needed(needs_zip64: bool) -> u16 {
    if needs_zip64 {
        ZIP_VERSION_NEEDED_ZIP64
    } else {
        ZIP_VERSION_NEEDED_STORED
    }
}

fn write_zip_size_u32(writer: &mut impl Write, value: u64) -> io::Result<()> {
    write_u32(writer, u32::try_from(value).unwrap_or(u32::MAX))
}

fn write_zip_offset_u32(writer: &mut impl Write, value: u64) -> io::Result<()> {
    write_zip_size_u32(writer, value)
}

fn zip_entry_count_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(feature = "oodle")]
fn oodle_compress_options(
    compressor: OodleCompressorKind,
    level: OodleCompressionLevel,
    seek_chunk_len: u32,
) -> Result<oodle_sys::OodleLZ_CompressOptions, PackagePayloadError> {
    let seek_chunk_len_i32 = i32::try_from(seek_chunk_len)
        .map_err(|_| PackagePayloadError::InvalidOodleSeekChunkLen { seek_chunk_len })?;
    let default =
        unsafe { oodle_sys::OodleLZ_CompressOptions_GetDefault(compressor.raw(), level.raw()) };
    if default.is_null() {
        return Err(PackagePayloadError::InvalidOodleSeekChunkLen { seek_chunk_len });
    }
    let mut options = unsafe { *default };
    options.seekChunkReset = 1;
    options.seekChunkLen = seek_chunk_len_i32;
    unsafe {
        oodle_sys::OodleLZ_CompressOptions_Validate(std::ptr::addr_of_mut!(options));
    }
    Ok(options)
}

fn manifest_uncompressed_bytes(manifest: &PackageManifest) -> u64 {
    manifest
        .physical_payload_entries()
        .fold(0_u64, |total, entry| total.saturating_add(entry.byte_len))
}

fn safe_package_name(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            output.push(ch);
        } else {
            output.push('_');
        }
    }
    if output.is_empty() {
        "_".to_string()
    } else {
        output
    }
}

fn write_u16(writer: &mut impl Write, value: u16) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u32(writer: &mut impl Write, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u64(writer: &mut impl Write, value: u64) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[derive(Debug, Default)]
struct PakArchiveStats {
    uncompressed_bytes: u64,
    payload_bytes: u64,
}

#[derive(Debug)]
struct PreparedPakEntry {
    name: String,
    product_path: String,
    method: PackageCompressionMethod,
    crc32: u32,
    uncompressed_size: u64,
    compressed_size: u64,
    bytes: Vec<u8>,
}

impl PreparedPakEntry {
    fn requires_zip64_sizes(&self) -> bool {
        self.uncompressed_size > u64::from(u32::MAX) || self.compressed_size > u64::from(u32::MAX)
    }
}

#[derive(Debug)]
struct ZipCentralEntry {
    name: String,
    product_path: String,
    method: PackageCompressionMethod,
    crc32: u32,
    compressed_size: u64,
    uncompressed_size: u64,
    local_header_offset: u64,
}

impl ZipCentralEntry {
    fn requires_zip64_sizes(&self) -> bool {
        self.uncompressed_size > u64::from(u32::MAX) || self.compressed_size > u64::from(u32::MAX)
    }

    fn requires_zip64(&self) -> bool {
        self.requires_zip64_sizes() || self.local_header_offset > u64::from(u32::MAX)
    }
}

struct CountingWriter<W> {
    inner: W,
    position: u64,
}

impl<W> CountingWriter<W> {
    const fn new(inner: W) -> Self {
        Self { inner, position: 0 }
    }

    const fn position(&self) -> u64 {
        self.position
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.position = self.position.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg(feature = "oodle")]
enum OodleCompressorKind {
    Kraken,
    Mermaid,
    Selkie,
    Leviathan,
    Hydra,
}

#[cfg(feature = "oodle")]
impl OodleCompressorKind {
    fn parse(profile: &str, value: &str) -> Result<Self, PackagePayloadError> {
        match value {
            value if value.eq_ignore_ascii_case("kraken") => Ok(Self::Kraken),
            value if value.eq_ignore_ascii_case("mermaid") => Ok(Self::Mermaid),
            value if value.eq_ignore_ascii_case("selkie") => Ok(Self::Selkie),
            value if value.eq_ignore_ascii_case("leviathan") => Ok(Self::Leviathan),
            value if value.eq_ignore_ascii_case("hydra") => Ok(Self::Hydra),
            _ => Err(PackagePayloadError::UnknownOodleCompressor {
                profile: profile.to_string(),
                compressor: value.to_string(),
            }),
        }
    }

    const fn raw(self) -> oodle_sys::OodleLZ_Compressor {
        match self {
            Self::Kraken => oodle_sys::OodleLZ_Compressor_OodleLZ_Compressor_Kraken,
            Self::Mermaid => oodle_sys::OodleLZ_Compressor_OodleLZ_Compressor_Mermaid,
            Self::Selkie => oodle_sys::OodleLZ_Compressor_OodleLZ_Compressor_Selkie,
            Self::Leviathan => oodle_sys::OodleLZ_Compressor_OodleLZ_Compressor_Leviathan,
            Self::Hydra => oodle_sys::OodleLZ_Compressor_OodleLZ_Compressor_Hydra,
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg(feature = "oodle")]
enum OodleCompressionLevel {
    SuperFast,
    VeryFast,
    Fast,
    Normal,
    Optimal1,
    Optimal2,
    Optimal3,
    Optimal4,
    Optimal5,
}

#[cfg(feature = "oodle")]
impl OodleCompressionLevel {
    fn parse(profile: &str, value: &str) -> Result<Self, PackagePayloadError> {
        match value {
            value if value.eq_ignore_ascii_case("super-fast") => Ok(Self::SuperFast),
            value if value.eq_ignore_ascii_case("very-fast") => Ok(Self::VeryFast),
            value if value.eq_ignore_ascii_case("fast") => Ok(Self::Fast),
            value if value.eq_ignore_ascii_case("normal") => Ok(Self::Normal),
            value if value.eq_ignore_ascii_case("optimal1") => Ok(Self::Optimal1),
            value if value.eq_ignore_ascii_case("optimal2") => Ok(Self::Optimal2),
            value if value.eq_ignore_ascii_case("optimal3") => Ok(Self::Optimal3),
            value if value.eq_ignore_ascii_case("optimal4") => Ok(Self::Optimal4),
            value if value.eq_ignore_ascii_case("optimal5") => Ok(Self::Optimal5),
            _ => Err(PackagePayloadError::UnknownOodleEffort {
                profile: profile.to_string(),
                effort: value.to_string(),
            }),
        }
    }

    const fn raw(self) -> oodle_sys::OodleLZ_CompressionLevel {
        match self {
            Self::SuperFast => {
                oodle_sys::OodleLZ_CompressionLevel_OodleLZ_CompressionLevel_SuperFast
            }
            Self::VeryFast => oodle_sys::OodleLZ_CompressionLevel_OodleLZ_CompressionLevel_VeryFast,
            Self::Fast => oodle_sys::OodleLZ_CompressionLevel_OodleLZ_CompressionLevel_Fast,
            Self::Normal => oodle_sys::OodleLZ_CompressionLevel_OodleLZ_CompressionLevel_Normal,
            Self::Optimal1 => oodle_sys::OodleLZ_CompressionLevel_OodleLZ_CompressionLevel_Optimal1,
            Self::Optimal2 => oodle_sys::OodleLZ_CompressionLevel_OodleLZ_CompressionLevel_Optimal2,
            Self::Optimal3 => oodle_sys::OodleLZ_CompressionLevel_OodleLZ_CompressionLevel_Optimal3,
            Self::Optimal4 => oodle_sys::OodleLZ_CompressionLevel_OodleLZ_CompressionLevel_Optimal4,
            Self::Optimal5 => oodle_sys::OodleLZ_CompressionLevel_OodleLZ_CompressionLevel_Optimal5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AssetCatalogPathRegistration;
    use az_filesystem::platform_product_cache_dir;
    use uuid::Uuid;

    #[test]
    fn loose_payload_writer_copies_validated_products() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let bytes = b"compiled prefab bytes";
        write_cache_product(temp.path(), "pc", product_path, bytes);
        let manifest = PackageManifest::new(
            profile("pc-dev", "loose", "none"),
            vec![entry(product_path, bytes)],
        )
        .unwrap();
        let output_root = temp.path().join("target/package");

        let receipt = write_package_payload(PackagePayloadWriteRequest::new(
            &manifest,
            &platform_product_cache_dir(temp.path(), "pc"),
            &output_root,
        ))
        .unwrap();

        assert_eq!(receipt.kind, PackagePayloadKind::Loose);
        assert_eq!(fs::read(receipt.path.join(product_path)).unwrap(), bytes);
    }

    #[test]
    fn loose_payload_writer_writes_shared_physical_product_once() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/shared.azbin";
        let bytes = b"one physical payload, two logical ids";
        write_cache_product(temp.path(), "pc", product_path, bytes);
        let registered = entry_with_sub_id(product_path, 7, bytes);
        let id_only = entry_with_sub_id(product_path, 3, bytes)
            .with_path_registration(AssetCatalogPathRegistration::AssetIdOnly);
        let manifest = PackageManifest::new(
            profile("pc-dev", "loose", "none"),
            vec![id_only, registered],
        )
        .unwrap();
        let output_root = temp.path().join("target/package");

        let receipt = write_package_payload(PackagePayloadWriteRequest::new(
            &manifest,
            &platform_product_cache_dir(temp.path(), "pc"),
            &output_root,
        ))
        .unwrap();

        assert_eq!(receipt.entry_count, 1);
        assert_eq!(receipt.uncompressed_bytes, bytes.len() as u64);
        assert_eq!(fs::read(receipt.path.join(product_path)).unwrap(), bytes);
    }

    #[test]
    fn loose_payload_writer_rejects_hash_mismatches() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        write_cache_product(temp.path(), "pc", product_path, b"different data");
        let manifest = PackageManifest::new(
            profile("pc-dev", "loose", "none"),
            vec![entry(product_path, b"expected bytes")],
        )
        .unwrap();

        let error = write_package_payload(PackagePayloadWriteRequest::new(
            &manifest,
            &platform_product_cache_dir(temp.path(), "pc"),
            &temp.path().join("target/package"),
        ))
        .unwrap_err();

        assert!(matches!(error, PackagePayloadError::HashMismatch { .. }));
    }

    #[test]
    fn pak_payload_writer_writes_stored_zip_shape() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let bytes = b"compiled prefab bytes";
        write_cache_product(temp.path(), "pc", product_path, bytes);
        let manifest = PackageManifest::new(
            profile("pc-release", "pak", "none"),
            vec![entry(product_path, bytes)],
        )
        .unwrap();

        let receipt = write_package_payload(PackagePayloadWriteRequest::new(
            &manifest,
            &platform_product_cache_dir(temp.path(), "pc"),
            &temp.path().join("target/package"),
        ))
        .unwrap();

        assert_eq!(receipt.kind, PackagePayloadKind::Pak);
        let pak = fs::read(receipt.path).unwrap();
        let local = local_file_header(&pak);
        assert_eq!(local.method, ZIP_METHOD_STORED);
        assert_eq!(local.name, product_path);
        assert_eq!(local.payload, bytes);
    }

    #[cfg(feature = "oodle")]
    #[test]
    fn pak_payload_writer_writes_oodle_method_15_payload() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let bytes = b"compiled prefab bytes compiled prefab bytes compiled prefab bytes";
        write_cache_product(temp.path(), "pc", product_path, bytes);
        let manifest = PackageManifest::new(
            profile("pc-release", "pak", "oodle"),
            vec![entry(product_path, bytes)],
        )
        .unwrap();

        let receipt = write_package_payload(PackagePayloadWriteRequest::new(
            &manifest,
            &platform_product_cache_dir(temp.path(), "pc"),
            &temp.path().join("target/package"),
        ))
        .unwrap();

        let pak = fs::read(receipt.path).unwrap();
        let local = local_file_header(&pak);
        assert_eq!(local.method, ZIP_METHOD_OODLE);
        assert_eq!(local.name, product_path);

        let mut decompressed = vec![0_u8; bytes.len()];
        let size = oodle_safe::decompress(
            local.payload,
            &mut decompressed,
            None,
            None,
            None,
            Some(oodle_safe::DecodeThreadPhase::All),
        )
        .unwrap();
        assert_eq!(size, bytes.len());
        assert_eq!(decompressed, bytes);
    }

    #[cfg(not(feature = "oodle"))]
    #[test]
    fn oodle_profile_reports_disabled_provider() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let bytes = b"compiled prefab bytes";
        write_cache_product(temp.path(), "pc", product_path, bytes);
        let manifest = PackageManifest::new(
            profile("pc-release", "pak", "oodle"),
            vec![entry(product_path, bytes)],
        )
        .unwrap();

        let error = write_package_payload(PackagePayloadWriteRequest::new(
            &manifest,
            &platform_product_cache_dir(temp.path(), "pc"),
            &temp.path().join("target/package"),
        ))
        .unwrap_err();

        assert!(matches!(error, PackagePayloadError::OodleUnavailable));
    }

    #[test]
    fn azpack_payload_writer_writes_stored_chunks_and_index() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let bytes = b"compiled prefab bytes";
        write_cache_product(temp.path(), "pc", product_path, bytes);
        let manifest = PackageManifest::new(
            profile("pc-release", "azpack", "none"),
            vec![entry(product_path, bytes)],
        )
        .unwrap();
        let output_root = temp.path().join("target/package");
        let stale_chunk = output_root.join("azpack/chunks/ff/stale.azchunk");
        fs::create_dir_all(stale_chunk.parent().unwrap()).unwrap();
        fs::write(&stale_chunk, b"stale").unwrap();

        let receipt = write_package_payload(PackagePayloadWriteRequest::new(
            &manifest,
            &platform_product_cache_dir(temp.path(), "pc"),
            &output_root,
        ))
        .unwrap();

        assert_eq!(receipt.kind, PackagePayloadKind::AzPack);
        assert!(!stale_chunk.exists());
        let index = crate::azpack::read_azpack_index(
            fs::read(receipt.path.join("package.azpack.index"))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        assert_eq!(
            index.chunk_size,
            u32::try_from(AZPACK_DEFAULT_CHUNK_SIZE).unwrap()
        );
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].product_path.as_str(), product_path);
        assert_eq!(index.entries[0].chunks.len(), 1);
        assert_eq!(
            index.entries[0].chunks[0].compression,
            AzPackChunkCompression::Stored
        );
        assert_eq!(
            fs::read(
                receipt
                    .path
                    .join(index.entries[0].chunks[0].chunk_path.as_str())
            )
            .unwrap(),
            bytes
        );
    }

    #[test]
    fn azpack_payload_writer_reuses_identical_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let first_path = "products/prefab-a.azbin";
        let second_path = "products/prefab-b.azbin";
        let bytes = b"shared compiled product bytes";
        write_cache_product(temp.path(), "pc", first_path, bytes);
        write_cache_product(temp.path(), "pc", second_path, bytes);
        let manifest = PackageManifest::new(
            profile("pc-release", "azpack", "none"),
            vec![
                entry_with_sub_id(first_path, 7, bytes),
                entry_with_sub_id(second_path, 8, bytes),
            ],
        )
        .unwrap();

        let receipt = AzPackPayloadWriter::new(StoredPackageCompressor)
            .with_chunk_size(64)
            .write(&PackagePayloadWriteRequest::new(
                &manifest,
                &platform_product_cache_dir(temp.path(), "pc"),
                &temp.path().join("target/package"),
            ))
            .unwrap();

        let index = crate::azpack::read_azpack_index(
            fs::read(receipt.path.join("package.azpack.index"))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        assert_eq!(index.entries.len(), 2);
        assert_eq!(
            index.entries[0].chunks[0].chunk_path,
            index.entries[1].chunks[0].chunk_path
        );
        assert_eq!(count_files(&receipt.path.join("chunks")), 1);
    }

    #[cfg(feature = "oodle")]
    #[test]
    fn azpack_payload_writer_writes_oodle_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let product_path = "products/prefab.azbin";
        let bytes = b"compiled prefab bytes compiled prefab bytes compiled prefab bytes";
        write_cache_product(temp.path(), "pc", product_path, bytes);
        let manifest = PackageManifest::new(
            profile("pc-release", "azpack", "oodle"),
            vec![entry(product_path, bytes)],
        )
        .unwrap();

        let receipt = write_package_payload(PackagePayloadWriteRequest::new(
            &manifest,
            &platform_product_cache_dir(temp.path(), "pc"),
            &temp.path().join("target/package"),
        ))
        .unwrap();

        let index = crate::azpack::read_azpack_index(
            fs::read(receipt.path.join("package.azpack.index"))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let chunk = &index.entries[0].chunks[0];
        assert_eq!(chunk.compression, AzPackChunkCompression::Oodle);
        let encoded = fs::read(receipt.path.join(chunk.chunk_path.as_str())).unwrap();
        assert_eq!(encoded.len(), chunk.encoded_len as usize);

        let mut decompressed = vec![0_u8; chunk.raw_len as usize];
        let size = oodle_safe::decompress(
            &encoded,
            &mut decompressed,
            None,
            None,
            None,
            Some(oodle_safe::DecodeThreadPhase::All),
        )
        .unwrap();
        assert_eq!(size, bytes.len());
        assert_eq!(decompressed, bytes);
    }

    #[test]
    fn package_payload_rejects_unknown_policy() {
        let error = PackagePayloadPolicy::from_profile(&profile("pc-release", "bundle", "oodle"))
            .unwrap_err();

        assert!(matches!(
            error,
            PackagePayloadError::UnsupportedPolicy { .. }
        ));
    }

    #[test]
    fn package_container_markers_express_backend_capabilities() {
        fn assert_parallel<C: ParallelPreparedPackageContainer>() {}
        fn assert_patch_friendly<C: PatchFriendlyPackageContainer>() {}
        fn assert_chunk_addressable<C: ChunkAddressablePackageContainer>() {}
        fn assert_compatibility<C: CompatibilityPackageContainer>() {}

        assert_parallel::<LooseContainer>();
        assert_parallel::<AzPackContainer>();
        assert_parallel::<PakContainer>();
        assert_patch_friendly::<LooseContainer>();
        assert_patch_friendly::<AzPackContainer>();
        assert_chunk_addressable::<AzPackContainer>();
        assert_compatibility::<PakContainer>();
        assert_eq!(PakContainer::ECOSYSTEM, "lumberyard/o3de/crypak");
    }

    #[test]
    fn package_payload_policy_maps_profiles_to_typed_backends() {
        let azpack_policy =
            PackagePayloadPolicy::from_profile(&profile("pc-release", "azpack", "oodle"))
                .expect("azpack profile should map to typed policy");
        assert_eq!(azpack_policy.container, PackagePayloadKind::AzPack);
        assert_eq!(azpack_policy.compression, PackageCompressionKind::Oodle);
        assert_eq!(azpack_policy.container.container_name(), "azpack");
        assert_eq!(azpack_policy.compression.compression_name(), "oodle");
        assert_eq!(
            azpack_policy.capabilities(),
            PackageBackendCapabilities {
                streamable: true,
                chunk_addressable: true,
                patch_friendly: true,
                parallel_prepared: true,
                compatibility_ecosystem: None,
            }
        );

        let pak_policy = PackagePayloadPolicy::from_profile(&profile("pc-release", "pak", "none"))
            .expect("pak profile should map to typed policy");
        assert_eq!(
            pak_policy.capabilities().compatibility_ecosystem,
            Some(PakContainer::ECOSYSTEM)
        );
        assert!(!pak_policy.capabilities().chunk_addressable);
    }

    #[test]
    fn package_payload_policy_rejects_unimplemented_backend_pairs() {
        let error = PackagePayloadPolicy::from_profile(&profile("pc-dev", "loose", "oodle"))
            .expect_err("loose oodle is not an implemented package backend");
        assert!(matches!(
            error,
            PackagePayloadError::UnsupportedPolicy { .. }
        ));
    }

    #[test]
    fn package_payload_layout_reports_backend_paths() {
        let output_root = Path::new("target/package");

        let loose = package_payload_layout(output_root, &profile("pc-dev", "loose", "none"))
            .expect("loose layout");
        assert_eq!(loose.kind, PackagePayloadKind::Loose);
        assert_eq!(loose.mount_root, output_root.join("loose"));
        assert_eq!(loose.payload_path, output_root.join("loose"));
        assert_eq!(
            loose.catalog_path,
            output_root
                .join("loose")
                .join(crate::ASSET_CATALOG_FILE_NAME)
        );

        let pak = package_payload_layout(output_root, &profile("pc release", "pak", "none"))
            .expect("pak layout");
        assert_eq!(pak.kind, PackagePayloadKind::Pak);
        assert_eq!(pak.mount_root, output_root);
        assert_eq!(pak.payload_path, output_root.join("pc_release.pak"));
        assert_eq!(
            pak.catalog_path,
            output_root.join(crate::ASSET_CATALOG_FILE_NAME)
        );
    }

    #[test]
    fn typed_payload_writers_reject_mismatched_profiles() {
        let manifest = PackageManifest::new(
            profile("pc-release", "pak", "none"),
            vec![entry("products/prefab.azbin", b"compiled prefab bytes")],
        )
        .unwrap();

        let error = LoosePayloadWriter
            .write(&PackagePayloadWriteRequest::new(
                &manifest,
                Path::new("Cache/pc"),
                Path::new("target/package"),
            ))
            .unwrap_err();

        assert!(matches!(
            error,
            PackagePayloadError::UnsupportedPolicy { .. }
        ));
    }

    fn profile(name: &str, container: &str, compression: &str) -> PackageManifestProfile {
        PackageManifestProfile {
            name: name.to_string(),
            asset_platform: "pc".to_string(),
            cargo_profile: "release".to_string(),
            container: container.to_string(),
            compression: compression.to_string(),
            oodle_compressor: (compression == "oodle").then(|| "kraken".to_string()),
            oodle_effort: (compression == "oodle").then(|| "normal".to_string()),
        }
    }

    fn entry(product_path: &str, bytes: &[u8]) -> PackageManifestEntry {
        entry_with_sub_id(product_path, 7, bytes)
    }

    fn entry_with_sub_id(product_path: &str, sub_id: u32, bytes: &[u8]) -> PackageManifestEntry {
        PackageManifestEntry::new(
            product_path,
            Uuid::from_bytes([3; 16]),
            sub_id,
            "az.test.raw",
            1,
            *blake3::hash(bytes).as_bytes(),
            bytes.len() as u64,
            Uuid::from_bytes([1; 16]),
            "prefabs/source.prefab.ron",
            "BuildPrefab",
        )
    }

    fn write_cache_product(project_root: &Path, platform: &str, product_path: &str, bytes: &[u8]) {
        let path = platform_product_cache_dir(project_root, platform).join(product_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    struct LocalFileHeader<'a> {
        method: u16,
        name: &'a str,
        payload: &'a [u8],
    }

    fn local_file_header(bytes: &[u8]) -> LocalFileHeader<'_> {
        assert_eq!(u32_at(bytes, 0), ZIP_LOCAL_FILE_HEADER_SIGNATURE);
        let method = u16_at(bytes, 8);
        let compressed_size = u32_at(bytes, 18) as usize;
        let name_len = u16_at(bytes, 26) as usize;
        let extra_len = u16_at(bytes, 28) as usize;
        let name_start = 30;
        let name_end = name_start + name_len;
        let payload_start = name_end + extra_len;
        let payload_end = payload_start + compressed_size;
        LocalFileHeader {
            method,
            name: std::str::from_utf8(&bytes[name_start..name_end]).unwrap(),
            payload: &bytes[payload_start..payload_end],
        }
    }

    fn u16_at(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
    }

    fn u32_at(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn count_files(path: &Path) -> usize {
        let mut count = 0;
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                count += count_files(&entry.path());
            } else if metadata.is_file() {
                count += 1;
            }
        }
        count
    }
}
