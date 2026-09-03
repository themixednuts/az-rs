//! Job request/response shapes — the wire format between the asset
//! processor and a builder.
//!
//! Mirrors:
//! - `AssetBuilderSDK::CreateJobsRequest` / `CreateJobsResponse`
//!   (`AssetBuilderSDK.h:510-600`).
//! - `AssetBuilderSDK::ProcessJobRequest` / `ProcessJobResponse`
//!   (`AssetBuilderSDK.h:695-780`).
//! - `AssetBuilderSDK::JobDescriptor`, `JobProduct`
//!   (`AssetBuilderSDK.h:430-660`).
//!
//! Lumberyard sends these over IPC to external builder processes.
//! Our builders are in-process, so we pass them as plain structs.

use std::fmt;
use std::marker::PhantomData;
use std::path::Path;

use az_core::{AssetData, AssetType};
use az_filesystem::SourcePath;
use az_gem_contract::{Registries, Registry, RegistryEntry};
use az_work::CancellationToken;
use uuid::Uuid;

use crate::AssetId;
use crate::builder::BuilderId;
use crate::dependency::{JobDependency, ProductDependency, SourceFileDependency};
use crate::format::ProductFormat;
use crate::value::{AssetBuilderValueError, JobKey, PlatformId, ProductFormatId, ProductPath};

/// The composed host state a build rule reads while planning and processing
/// jobs.
///
/// A rule callback is a plain `fn` pointer, so it captures nothing; everything
/// it needs about the rest of the composition arrives through the request it is
/// handed. The host borrows its own [`Registries`] — there is no second copy
/// and no process-global fallback, so a rule that reads nothing composed sees
/// an empty registry because the host composed nothing, never because the
/// plumbing dropped it.
#[derive(Clone, Copy)]
pub struct JobContext<'a> {
    registries: &'a Registries,
}

impl<'a> JobContext<'a> {
    #[must_use]
    pub const fn new(registries: &'a Registries) -> Self {
        Self { registries }
    }

    #[must_use]
    pub const fn registries(&self) -> &'a Registries {
        self.registries
    }

    /// The composed entries of one registry family, or `None` when this host
    /// composed no contribution that registers into it.
    #[must_use]
    pub fn registry<T: RegistryEntry>(&self) -> Option<&'a Registry<T>> {
        self.registries.get::<T>()
    }
}

impl fmt::Debug for JobContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobContext").finish_non_exhaustive()
    }
}

/// Build platform identifier (e.g. `"pc"`, `"server"`).
///
/// Mirrors `AssetBuilderSDK::PlatformInfo::m_identifier`. Stored as
/// `&'static str` — the active platform set is fixed per build.
pub type Platform = &'static str;

/// Default platform when nothing else is specified.
pub const DEFAULT_PLATFORM: Platform = "pc";

/// Default platform when nothing else is specified.
pub const DEFAULT_PLATFORM_ID: PlatformId = PlatformId::new_static("pc");

// -- CreateJobs ------------------------------------------------------

/// Mirrors `CreateJobsRequest` (`AssetBuilderSDK.h:510`).
#[derive(Debug, Clone)]
pub struct CreateJobsRequest<'a> {
    /// Builder's own id — passed for builders that share dispatch
    /// code across patterns.
    pub builder_id: BuilderId,
    /// Normalized virtual source path relative to the source root.
    pub source_path: SourcePath,
    /// Source root the path is relative to (e.g. the pak's mounted
    /// asset dir).
    pub source_root: &'a Path,
    /// Source file UUID — `AssetId.guid` for the source. Mirrors
    /// `m_sourceFileUuid`.
    pub source_uuid: Uuid,
    /// Optional Azoth authored/source schema type recorded by the asset DB
    /// for this source, for example `azoth.project.Prefab`.
    pub source_schema_type: Option<&'a str>,
    /// Saved source bytes used for build planning. For DB-authored sources this
    /// is the saved checkpoint, not unsaved editor draft state.
    pub source_bytes: &'a [u8],
    /// Active platforms for this build run.
    pub platforms: &'a [PlatformId],
    /// The host's composition. Rules whose planning depends on what else
    /// composed — reflected Prefab types, compilable graph types — read it
    /// here.
    pub context: &'a JobContext<'a>,
}

impl<'a> CreateJobsRequest<'a> {
    #[expect(
        clippy::too_many_arguments,
        reason = "every argument is one public field of the struct being built, so the only coherent params struct is `CreateJobsRequest` itself"
    )]
    pub fn new(
        builder_id: BuilderId,
        source_path: impl AsRef<str>,
        source_root: &'a Path,
        source_uuid: Uuid,
        source_schema_type: Option<&'a str>,
        source_bytes: &'a [u8],
        platforms: &'a [PlatformId],
        context: &'a JobContext<'a>,
    ) -> Self {
        Self {
            builder_id,
            source_path: SourcePath::new(source_path),
            source_root,
            source_uuid,
            source_schema_type,
            source_bytes,
            platforms,
            context,
        }
    }
}

/// Mirrors `CreateJobsResponse` (`AssetBuilderSDK.h:603`).
#[derive(Debug, Default, Clone)]
pub struct CreateJobsResponse {
    /// One descriptor per (platform, jobKey) the builder wants run.
    pub jobs: Vec<JobDescriptor>,
    /// Other source files this source depends on. Touching one of
    /// them re-fingerprints this source.
    pub source_dependencies: Vec<SourceFileDependency>,
    /// Was the request handled? `Failed` aborts the source.
    pub result: CreateJobsResult,
}

/// Mirrors `CreateJobsResultCode` (`AssetBuilderSDK.h:608`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CreateJobsResult {
    #[default]
    Success,
    /// Builder doesn't recognize this source despite the pattern
    /// claiming it. Treated as "skip silently".
    Skipped,
    /// Hard failure — log + propagate.
    Failed,
}

/// Mirrors `JobDescriptor` (`AssetBuilderSDK.h:430`).
#[derive(Debug, Clone)]
pub struct JobDescriptor {
    /// Job key — discriminates multiple jobs on the same source. Common case:
    /// `"default"`.
    pub job_key: JobKey,
    /// Target platform.
    pub platform: PlatformId,
    /// Other jobs that must run before this one (potentially on
    /// different sources).
    pub job_dependencies: Vec<JobDependency>,
    /// Critical jobs block startup until they finish (mirrors
    /// `m_critical`).
    pub critical: bool,
}

impl JobDescriptor {
    /// Convenience constructor for the common-case "one default job
    /// per active platform, no dependencies" pattern.
    #[must_use]
    pub fn default_for_platform(platform: PlatformId) -> Self {
        Self {
            job_key: JobKey::default_key(),
            platform,
            job_dependencies: Vec::new(),
            critical: false,
        }
    }
}

pub type CreateJobsFn = fn(&CreateJobsRequest<'_>) -> CreateJobsResponse;

// -- ProcessJob ------------------------------------------------------

/// Mirrors `ProcessJobRequest` (`AssetBuilderSDK.h:695`).
pub struct ProcessJobRequest<'a> {
    pub builder_id: BuilderId,
    /// Normalized virtual source path relative to the source root.
    pub source_path: SourcePath,
    pub source_root: &'a Path,
    pub source_uuid: Uuid,
    /// Complete source identity preserved by import metadata for a one-to-one
    /// port, when present.
    ///
    /// The GUID is identical to `source_uuid`; the sub-ID is carried as well so
    /// the builder can assign the original identity to its explicitly selected
    /// primary product. Secondary products retain their builder-owned sub-IDs.
    pub preserved_source_asset_id: Option<AssetId>,
    /// Optional Azoth authored/source schema type recorded by the asset DB
    /// for this source. Product types stay on [`BuildProduct::asset_type`].
    pub source_schema_type: Option<&'a str>,
    /// In-memory bytes of the source. Builders that need to write
    /// derived products consume these.
    pub source_bytes: &'a [u8],
    pub platform: &'a str,
    pub job_key: &'a str,
    /// Dev product cache root for the target platform (e.g.
    /// `<project>/Cache/pc/`).
    pub cache_root: &'a Path,
    /// Cooperative cancellation token for the current worker run.
    pub cancellation: &'a CancellationToken,
    /// The host's composition. Builders read it directly — no type-erasure,
    /// no cast, no downcast.
    pub context: &'a JobContext<'a>,
}

impl<'a> ProcessJobRequest<'a> {
    #[expect(
        clippy::too_many_arguments,
        reason = "every argument is one public field of the struct being built, so the only coherent params struct is `ProcessJobRequest` itself"
    )]
    pub fn new(
        builder_id: BuilderId,
        source_path: impl AsRef<str>,
        source_root: &'a Path,
        source_uuid: Uuid,
        preserved_source_asset_id: Option<AssetId>,
        source_schema_type: Option<&'a str>,
        source_bytes: &'a [u8],
        platform: &'a str,
        job_key: &'a str,
        cache_root: &'a Path,
        cancellation: &'a CancellationToken,
        context: &'a JobContext<'a>,
    ) -> Self {
        Self {
            builder_id,
            source_path: SourcePath::new(source_path),
            source_root,
            source_uuid,
            preserved_source_asset_id,
            source_schema_type,
            source_bytes,
            platform,
            job_key,
            cache_root,
            cancellation,
            context,
        }
    }
}

/// Mirrors `ProcessJobResponse` (`AssetBuilderSDK.h:746`).
#[derive(Debug, Default, Clone)]
pub struct ProcessJobResponse {
    pub products: Vec<BuildProduct>,
    /// Mirrors `m_outputProducts` source dependencies (per-product
    /// asset references).
    pub product_dependencies: Vec<(usize, ProductDependency)>,
    pub result: ProcessJobResult,
}

/// Mirrors `ProcessJobResultCode` (`AssetBuilderSDK.h:680`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ProcessJobResult {
    #[default]
    Success,
    /// Source was unrecognized once read; AP marks the source as
    /// processed without products.
    Skipped,
    /// Hard failure — log + propagate.
    Failed,
    /// Cancelled (e.g. user shut AP down mid-job).
    Cancelled,
}

/// Product emitted by one successful build job.
///
/// It mirrors Lumberyard's `JobProduct` identity fields while making the
/// Azoth byte-format contract explicit. `asset_type` says what runtime/editor
/// asset this is; `format` and `format_version` say how `bytes` are encoded.
#[derive(Debug, Clone)]
pub struct BuildProduct {
    /// Path of the product file relative to the cache root. Builder
    /// chooses this — see Lumberyard's freedom note in
    /// `AssetBuilderSDK.h:633` ("Builders have freedom to output to
    /// any path relative to cache root").
    pub product_path: ProductPath,
    /// Additional catalog lookup paths for this product.
    ///
    /// Aliases never create another product or another staged file.
    pub catalog_aliases: Vec<ProductPath>,
    /// Whether this product claims its path and aliases for runtime by-path
    /// lookup. `AssetIdOnly` products remain real catalog entries and may share
    /// the same physical path with one registered identity.
    pub catalog_path_registration: az_asset::AssetCatalogPathRegistration,
    /// Type of asset being produced. Mirrors
    /// `m_productAssetType`.
    pub asset_type: AssetType,
    /// Registered product byte-format id.
    pub format: ProductFormatId,
    /// Version of the registered product byte format.
    pub format_version: u32,
    /// Mirrors `m_productSubID`. Builder must ensure unique within a
    /// (source, builder) pair (build-stable, location-stable,
    /// platform-stable, mutually-exclusive — `AssetBuilderSDK.h:118`).
    pub sub_id: u32,
    /// Bytes the builder produced. AP writes these to
    /// `<cache_root>/<product_path>` and records the row.
    pub bytes: Vec<u8>,
}

impl BuildProduct {
    /// # Errors
    ///
    /// Returns [`AssetBuilderValueError`] when `product_path` is empty or not
    /// asset-relative.
    pub fn from_dynamic_parts(
        product_path: impl AsRef<str>,
        asset_type: AssetType,
        format: ProductFormatId,
        format_version: u32,
        sub_id: u32,
        bytes: Vec<u8>,
    ) -> Result<Self, AssetBuilderValueError> {
        Ok(Self {
            product_path: ProductPath::new(product_path)?,
            catalog_aliases: Vec::new(),
            catalog_path_registration: az_asset::AssetCatalogPathRegistration::Registered,
            asset_type,
            format,
            format_version,
            sub_id,
            bytes,
        })
    }

    /// Construct the primary product of a one-to-one source conversion when
    /// its product format is selected dynamically.
    ///
    /// Imported sources with preserved identity metadata keep that identity's
    /// sub-ID; native sources use `default_sub_id`. Builders must use this only
    /// for the single product that represents the source itself.
    ///
    /// # Errors
    ///
    /// Returns [`AssetBuilderValueError`] when `product_path` is empty or not
    /// asset-relative.
    pub fn from_primary_source_dynamic_parts(
        request: &ProcessJobRequest<'_>,
        product_path: impl AsRef<str>,
        asset_type: AssetType,
        format: ProductFormatId,
        format_version: u32,
        default_sub_id: u32,
        bytes: Vec<u8>,
    ) -> Result<Self, AssetBuilderValueError> {
        Self::from_dynamic_parts(
            product_path,
            asset_type,
            format,
            format_version,
            default_sub_id,
            bytes,
        )
        .map(|product| product.with_primary_source_identity(request))
    }

    /// # Errors
    ///
    /// Returns [`AssetBuilderValueError`] when `product_path` is empty or not
    /// asset-relative.
    pub fn for_format<P: ProductFormat>(
        product_path: impl AsRef<str>,
        sub_id: u32,
        bytes: Vec<u8>,
    ) -> Result<Self, AssetBuilderValueError> {
        TypedBuildProduct::<P>::new(product_path, sub_id, bytes).map(TypedBuildProduct::erase)
    }

    /// Construct the primary product of a one-to-one source conversion.
    ///
    /// Imported sources with preserved identity metadata keep that identity's
    /// sub-ID; native sources use `default_sub_id`. Builders must use this only
    /// for the single product that represents the source itself. Derived or
    /// secondary products use [`Self::for_format`] with their own stable IDs.
    ///
    /// # Errors
    ///
    /// Returns [`AssetBuilderValueError`] when `product_path` is empty or not
    /// asset-relative.
    pub fn for_primary_source_format<P: ProductFormat>(
        request: &ProcessJobRequest<'_>,
        product_path: impl AsRef<str>,
        default_sub_id: u32,
        bytes: Vec<u8>,
    ) -> Result<Self, AssetBuilderValueError> {
        Self::from_primary_source_dynamic_parts(
            request,
            product_path,
            <P::Asset as AssetData>::ASSET_TYPE,
            P::ID,
            P::VERSION,
            default_sub_id,
            bytes,
        )
    }

    /// Apply the source identity to the one product that represents it.
    ///
    /// Imported sources with preserved identity metadata keep that identity's
    /// sub-ID. Native sources retain the product's builder-owned sub-ID. All
    /// other product metadata is preserved.
    #[must_use]
    pub const fn with_primary_source_identity(mut self, request: &ProcessJobRequest<'_>) -> Self {
        if let Some(asset_id) = request.preserved_source_asset_id {
            self.sub_id = asset_id.sub_id;
        }
        self
    }

    /// Attach additional catalog lookup paths to this one product.
    #[must_use]
    pub fn with_catalog_aliases(mut self, aliases: impl IntoIterator<Item = ProductPath>) -> Self {
        self.catalog_aliases.extend(aliases);
        self
    }

    /// Select whether this product claims its physical path for by-path
    /// catalog lookup.
    #[must_use]
    pub const fn with_catalog_path_registration(
        mut self,
        registration: az_asset::AssetCatalogPathRegistration,
    ) -> Self {
        self.catalog_path_registration = registration;
        self
    }

    /// The full `AssetId` this product will register under.
    #[must_use]
    pub const fn asset_id(&self, source_uuid: Uuid) -> AssetId {
        AssetId::new(source_uuid, self.sub_id)
    }
}

/// Typed build product whose identity is stamped from a `ProductFormat`
/// descriptor.
#[derive(Debug, Clone)]
pub struct TypedBuildProduct<P: ProductFormat> {
    product_path: ProductPath,
    sub_id: u32,
    bytes: Vec<u8>,
    _marker: PhantomData<fn() -> P>,
}

impl<P: ProductFormat> TypedBuildProduct<P> {
    /// # Errors
    ///
    /// Returns [`AssetBuilderValueError`] when `product_path` is empty or not
    /// asset-relative.
    pub fn new(
        product_path: impl AsRef<str>,
        sub_id: u32,
        bytes: Vec<u8>,
    ) -> Result<Self, AssetBuilderValueError> {
        Ok(Self {
            product_path: ProductPath::new(product_path)?,
            sub_id,
            bytes,
            _marker: PhantomData,
        })
    }

    #[must_use]
    pub fn from_trusted_path(product_path: impl AsRef<str>, sub_id: u32, bytes: Vec<u8>) -> Self {
        Self {
            product_path: ProductPath::from_trusted(product_path),
            sub_id,
            bytes,
            _marker: PhantomData,
        }
    }

    #[must_use]
    pub fn erase(self) -> BuildProduct {
        BuildProduct {
            product_path: self.product_path,
            catalog_aliases: Vec::new(),
            catalog_path_registration: az_asset::AssetCatalogPathRegistration::Registered,
            asset_type: <P::Asset as AssetData>::ASSET_TYPE,
            format: P::ID,
            format_version: P::VERSION,
            sub_id: self.sub_id,
            bytes: self.bytes,
        }
    }
}

pub type ProcessJobFn = fn(&ProcessJobRequest<'_>) -> ProcessJobResponse;

#[cfg(test)]
mod tests {
    use az_asset::AssetCatalogPathRegistration;
    use uuid::uuid;

    use super::*;

    #[test]
    fn primary_source_identity_preserves_product_metadata() {
        let preserved = AssetId::new(uuid!("c2d770c8-c04f-45f6-bc16-8df3d850057e"), 17);
        let cancellation = CancellationToken::new();
        let registries = Registries::new();
        let context = JobContext::new(&registries);
        let request = ProcessJobRequest::new(
            BuilderId::new(uuid!("e8e2496e-7a5f-428a-b0ad-b721a31c1c5a")),
            "source/example.test.ron",
            Path::new("assets"),
            preserved.guid,
            Some(preserved),
            None,
            b"source",
            "pc",
            "test",
            Path::new("cache"),
            &cancellation,
            &context,
        );
        let alias = ProductPath::new("aliases/example.test.bin").unwrap();
        let asset_type = AssetType::new(uuid!("08e414bd-bb4f-4f9e-ac4e-5876f56d5454"));
        let format = ProductFormatId::from_dynamic_parts("az.test.PrimaryIdentity");
        let product = BuildProduct::from_dynamic_parts(
            "products/example.test.bin",
            asset_type,
            format,
            3,
            5,
            vec![1, 2, 3],
        )
        .unwrap()
        .with_catalog_aliases([alias.clone()])
        .with_catalog_path_registration(AssetCatalogPathRegistration::AssetIdOnly)
        .with_primary_source_identity(&request);

        assert_eq!(product.asset_id(request.source_uuid), preserved);
        assert_eq!(product.product_path.as_str(), "products/example.test.bin");
        assert_eq!(product.catalog_aliases, vec![alias]);
        assert_eq!(
            product.catalog_path_registration,
            AssetCatalogPathRegistration::AssetIdOnly
        );
        assert_eq!(product.asset_type, asset_type);
        assert_eq!(product.format, format);
        assert_eq!(product.format_version, 3);
        assert_eq!(product.bytes, vec![1, 2, 3]);
    }
}
