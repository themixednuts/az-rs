//! Build rule registration for the offline asset pipeline.
//!
//! This intentionally keeps the useful shape of Lumberyard's
//! `AssetBuilderSDK::AssetBuilderDesc` (`AssetBuilderSDK.h:229-300`)
//! while naming the Azoth abstraction by what it does: a rule claims a
//! source shape and converts matching sources into one or more build
//! products.
//!
//! Lumberyard implements builders as gem components that connect to
//! `AssetBuilderBus` and call `RegisterBuilderInformation` with their
//! builder descriptors. We model the same registration as a plain
//! struct; a registry (`crate::registry::BuildRuleRegistry`) walks
//! every registered rule and dispatches.

use std::borrow::Cow;

use az_core::uuid::AzUuidExt;
use uuid::Uuid;

use crate::job::{CreateJobsFn, ProcessJobFn};
use crate::pattern::AssetBuilderPattern;
use crate::value::{ProductFormatId, SourceSchemaType};

/// Stable identifier for a builder.
///
/// Mirrors `AssetBuilderDesc::m_busId` — a UUID minted once when the
/// rule is authored and never changed thereafter. Changing it creates a new
/// builder identity and forces affected job attempts to be rebuilt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct BuilderId(pub Uuid);

impl BuilderId {
    #[must_use]
    pub const fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Mint a stable [`BuilderId`] from a builder's name.
    ///
    /// Hashes `name` with `AZ::Uuid::CreateName` (the same primitive
    /// Lumberyard uses to derive a builder's bus id from its string name), so a
    /// given name always yields the same id across builds and machines.
    #[must_use]
    pub fn from_name(name: &[u8]) -> Self {
        Self(Uuid::create_name(name))
    }
}

impl std::fmt::Display for BuilderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{{}}}", self.0.as_hyphenated())
    }
}

impl From<Uuid> for BuilderId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

/// Declares how a build rule identifies the primary source it owns.
#[derive(Debug, Clone)]
pub enum SourceMatcher {
    /// Path patterns alone identify the source. This is the right shape
    /// for imported/raw file formats such as `.dds`, `.cgf`, `.xml`, etc.
    PathOnly(&'static [AssetBuilderPattern]),
    /// Path patterns identify the container syntax and authored schema
    /// types identify the document semantics inside it. This is the right
    /// shape for shared authoring formats such as `.ron` where multiple
    /// project/editor document types can coexist.
    ///
    /// The schema list is a [`Cow`] because a rule whose claim is decided by
    /// its host's composition — the graph compiler claims exactly the graph
    /// types the host composed a compiler backend for — owns the list it
    /// resolved, while an ordinary typed rule borrows its descriptor's static
    /// one.
    Schemas {
        patterns: &'static [AssetBuilderPattern],
        schemas: Cow<'static, [SourceSchemaType]>,
    },
}

impl SourceMatcher {
    #[must_use]
    pub fn matches(&self, source_path: &str, source_schema_type: Option<&str>) -> bool {
        self.matches_path(source_path) && self.matches_source_schema(source_schema_type)
    }

    #[must_use]
    pub fn matches_path(&self, source_path: &str) -> bool {
        self.patterns().iter().any(|p| p.matches(source_path))
    }

    #[must_use]
    pub fn matches_source_schema(&self, source_schema_type: Option<&str>) -> bool {
        match self {
            Self::PathOnly(_) => true,
            Self::Schemas { schemas, .. } => source_schema_type.is_some_and(|schema_type| {
                schemas
                    .iter()
                    .any(|candidate| candidate.as_str() == schema_type)
            }),
        }
    }

    #[must_use]
    pub const fn patterns(&self) -> &'static [AssetBuilderPattern] {
        match self {
            Self::PathOnly(patterns) | Self::Schemas { patterns, .. } => patterns,
        }
    }

    #[must_use]
    pub fn schema_types(&self) -> &[SourceSchemaType] {
        match self {
            Self::PathOnly(_) => &[],
            Self::Schemas { schemas, .. } => schemas,
        }
    }
}

/// Declares one source dependency family that can affect job creation.
///
/// Most rules start with no static dependencies and return precise dynamic
/// dependencies from `create_jobs`. This exists for source families whose
/// dependency shape is known before parsing a specific source.
#[derive(Debug, Clone)]
pub struct SourceDependencyRule {
    pub kind: SourceDependencyKind,
    pub matcher: SourceMatcher,
}

/// Static source dependency behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDependencyKind {
    /// Touching the matched source can require rebuilding this rule's jobs.
    Fingerprint,
    /// The matched source must be processed before this rule's jobs run.
    Order,
}

/// Product-format boundary enforced for one build rule.
///
/// Typed rules declare a closed format set. Registry-selected builders must opt
/// into `Dynamic` explicitly; this prevents an aggregate source declaration
/// from silently emitting an unrelated product format through the erased
/// [`ProcessJobFn`] boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductFormatPolicy {
    Declared(&'static [ProductFormatId]),
    Dynamic,
}

impl ProductFormatPolicy {
    #[must_use]
    pub fn accepts(self, format: ProductFormatId) -> bool {
        match self {
            Self::Declared(formats) => formats.contains(&format),
            Self::Dynamic => true,
        }
    }

    #[must_use]
    pub const fn declared(self) -> Option<&'static [ProductFormatId]> {
        match self {
            Self::Declared(formats) => Some(formats),
            Self::Dynamic => None,
        }
    }
}

/// One build rule, resolved against the host that composed it.
///
/// A rule's claim and its analysis fingerprint can both depend on what else
/// the host composed, so this value is produced from a
/// [`crate::BuildRuleRegistration`] once composition is complete — see
/// [`crate::BuildRuleRegistry::compose`].
pub struct BuildRule {
    /// Human-readable identifier (for example, `"sample.scene"`). Used as the default job
    /// key when no override is supplied.
    pub name: &'static str,

    /// Mirrors `m_busId`. Stable across releases.
    pub id: BuilderId,

    /// Primary source matcher this rule owns.
    pub primary_source: SourceMatcher,

    /// Static dependency families for this rule. Dynamic per-source
    /// dependencies still belong in `CreateJobsResponse`.
    pub source_dependencies: &'static [SourceDependencyRule],

    /// Mirrors `m_version`. Bump when a builder's output format
    /// changes so downstream attempts can be re-fingerprinted.
    pub version: u32,

    /// Mirrors `AssetBuilderDesc::m_analysisFingerprint`. Change this when
    /// the builder's analysis or job-creation logic changes in a way that may
    /// produce different jobs for otherwise unchanged sources. Unlike the
    /// process executable identity, this value is scoped to this builder.
    ///
    /// Owned, because a rule whose products embed contributed types must
    /// re-fingerprint when the host composes a different set of them.
    pub analysis_fingerprint: String,

    /// Product formats this rule is permitted to emit.
    pub product_formats: ProductFormatPolicy,

    /// Builder callback that produces a list of jobs from one source
    /// file (one job per platform, typically). Mirrors
    /// `AssetBuilderDesc::m_createJobFunction`.
    pub create_jobs: CreateJobsFn,

    /// Builder callback that processes one job: reads the source,
    /// writes products, returns metadata. Mirrors
    /// `AssetBuilderDesc::m_processJobFunction`.
    pub process_job: ProcessJobFn,
}

impl BuildRule {
    /// Test whether this builder claims the given source path.
    #[must_use]
    pub fn matches(&self, source_path: &str) -> bool {
        self.primary_source.matches_path(source_path)
    }

    #[must_use]
    pub fn matches_source(&self, source_path: &str, source_schema_type: Option<&str>) -> bool {
        self.primary_source.matches(source_path, source_schema_type)
    }

    #[must_use]
    pub fn matches_source_schema(&self, source_schema_type: Option<&str>) -> bool {
        self.primary_source
            .matches_source_schema(source_schema_type)
    }
}

impl Clone for BuildRule {
    fn clone(&self) -> Self {
        Self {
            name: self.name,
            id: self.id,
            primary_source: self.primary_source.clone(),
            source_dependencies: self.source_dependencies,
            version: self.version,
            analysis_fingerprint: self.analysis_fingerprint.clone(),
            product_formats: self.product_formats,
            create_jobs: self.create_jobs,
            process_job: self.process_job,
        }
    }
}

impl std::fmt::Debug for BuildRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Skip the function pointers — they print as opaque `0x...`
        // which adds noise to test output.
        f.debug_struct("BuildRule")
            .field("name", &self.name)
            .field("id", &self.id)
            .field("primary_source", &self.primary_source)
            .field("source_dependencies", &self.source_dependencies)
            .field("version", &self.version)
            .field("analysis_fingerprint", &self.analysis_fingerprint)
            .field("product_formats", &self.product_formats)
            .finish_non_exhaustive()
    }
}
