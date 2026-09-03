//! `az-asset-builder` — Azoth's offline asset build-rule SDK.
//!
//! This is the **offline asset pipeline SDK**: the contract every
//! builder implements so the asset processor can dispatch sources to
//! the right builder, run them, and persist the products.
//!
//! It keeps the useful shape of the C++ headers under
//! Lumberyard reference: `dev/Code/Tools/AssetProcessor/AssetBuilderSDK/AssetBuilderSDK/`,
//! while using Azoth's registry/catalog model for persistence. `AssetId` /
//! `AssetId` derivation remains byte-compatible so imported legacy products are
//! addressable by the same identifiers the original game's runtime catalog
//! uses.
//!
//! ## What's here
//!
//! - [`asset_id`] — build-pipeline source path to `AssetId` derivation
//!   helpers using the `AzCore` `Uuid::CreateName` hash (SHA-1 based, NOT
//!   standard `UUIDv5`).
//! - [`source_meta`] — native source sidecar identity (`azoth.source-meta/v1`),
//!   including preserved catalog ids and the `CreateName` fallback.
//! - [`pattern`] — `AssetBuilderPattern` (wildcard / regex) for
//!   pattern → builder dispatch.
//! - [`builder`] — `BuildRule`, `BuilderId`, `SourceMatcher`.
//! - [`job`] — `CreateJobs*` / `ProcessJob*` request/response shapes
//!   and `BuildProduct`.
//! - [`dependency`] — `SourceFileDependency`, `JobDependency`,
//!   `ProductDependency` + flags.
//! - [`registry`] — `BuildRuleRegistry`, the central source → rule
//!   dispatcher.
//! - [`registration`] — link-time registration used by project-owned
//!   asset-processor binaries to discover project and gem builders.
//!
//! ## What's NOT here
//!
//! - Actual builders (they live with the owning format, project, or gem
//!   crates; project-owned workers assemble their rules for dispatch).
//! - The asset database (`az-assetdb`).
//! - The asset processor orchestration.

extern crate self as az_asset_builder;

pub mod asset_id;
pub mod builder;
pub mod dependency;
pub mod fingerprint;
pub mod format;
pub mod job;
pub mod pattern;
pub mod registration;
pub mod registry;
pub mod source_meta;
pub mod source_transform;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod value;

pub use asset_id::{source_asset_id, source_guid};
pub use az_asset::AssetCatalogPathRegistration;
pub use az_asset_builder_derive::{ProductFormat, SourceFormat};
pub use az_core::{AssetId, AssetIdBytesError, AssetIdParseError};
pub use az_filesystem::{SourcePath, normalize_source_path};
pub use az_work::CancellationToken;
pub use builder::{
    BuildRule, BuilderId, ProductFormatPolicy, SourceDependencyKind, SourceDependencyRule,
    SourceMatcher,
};
pub use dependency::{
    JobDependency, JobDependencyType, ProductDependency, ProductDependencyFlags,
    SourceFileDependency,
};
pub use format::{
    BuildRuleDynamicProductBuilder, BuildRuleProductBuilder, BuildRuleProductSetBuilder,
    BuildRuleSourceBuilder, ProductFormat, ProductFormatSet, SourceFormat, product_format_id,
    source_schema_type,
};
pub use job::{
    BuildProduct, CreateJobsFn, CreateJobsRequest, CreateJobsResponse, CreateJobsResult,
    DEFAULT_PLATFORM, DEFAULT_PLATFORM_ID, JobContext, JobDescriptor, Platform, ProcessJobFn,
    ProcessJobRequest, ProcessJobResponse, ProcessJobResult, TypedBuildProduct,
};
pub use pattern::{AssetBuilderPattern, PatternError};
pub use registration::{BuildRuleFactory, BuildRuleRegistration};
pub use registry::BuildRuleRegistry;
pub use source_meta::{
    SOURCE_META_SIDECAR_SUFFIX, SOURCE_META_SPEC, SourceAssetMeta, SourceMetaError,
    read_source_asset_meta, resolve_referenced_product_id, resolve_source_asset_guid,
    serialize_source_asset_meta, source_meta_sidecar_path,
};
pub use source_transform::{
    LegacySourceArtifact, LegacySourceInput, LegacySourceOutput, LegacySourceTransform,
};
pub use value::{
    AssetBuilderValueError, JobKey, PROJECT_SOURCE_ROOT, PlatformId, ProductFormatId,
    ProductFormatRegistration, ProductPath, SourceFileEditCodecContext,
    SourceFileEditCodecRegistration, SourceFileEditDocument, SourceFileEditError,
    SourceFileEditLoadFn, SourceFileEditLoadRequest, SourceFileEditLoadResult,
    SourceFileEditObject, SourceFileEditOperation, SourceFileEditOperationFn,
    SourceFileEditOperationRequest, SourceFileEditOperationResult, SourceFileEditSaveFn,
    SourceFileEditSaveRequest, SourceFileEditSaveResult, SourceFileTemplateCandidate,
    SourceFileTemplateCandidatesFn, SourceFileTemplateError, SourceFileTemplateFn,
    SourceFileTemplateRegistration, SourceFileTemplateRequest, SourceFileTemplateResult,
    SourceFileWorkflow, SourceSchemaAuthoring, SourceSchemaRegistration, SourceSchemaType,
    composed_product_format, composed_product_format_by_id, composed_product_formats,
    composed_source_file_edit_codec, composed_source_file_edit_codecs,
    composed_source_file_template, composed_source_file_templates, composed_source_schemas,
};

/// Kept for the registries that still discover at link time — the graph
/// compiler backends in `az-graph-builder`. Every asset-builder family reaches
/// its host through a registrar instead.
#[doc(hidden)]
pub use inventory;
