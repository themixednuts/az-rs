//! `BuildRuleRegistry` — the central source → build-rule dispatcher.
//!
//! Lumberyard discovers builders via `AssetBuilderBus` connections; a host
//! composes its contributions instead, then resolves every contributed
//! [`BuildRuleRegistration`] into the rule it dispatches.
//!
//! Insertion order matters — matching builders are returned in
//! registration order so job planning remains deterministic.

use crate::builder::BuildRule;
use crate::builder::BuilderId;
use crate::job::JobContext;
use crate::registration::BuildRuleRegistration;

/// Ordered set of composed build rules.
pub struct BuildRuleRegistry {
    rules: Vec<BuildRule>,
}

impl BuildRuleRegistry {
    #[must_use]
    pub const fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Resolve every build rule the host composed, in dispatch order.
    ///
    /// Ordering is priority descending, then rule name, then builder id — the
    /// same order the link-time registry produced, minus the link order that
    /// used to decide the remaining ties invisibly.
    #[must_use]
    pub fn compose(context: &JobContext<'_>) -> Self {
        let mut registrations = context
            .registry::<BuildRuleRegistration>()
            .map(|registry| registry.entries().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        registrations.sort_by(|left, right| {
            right
                .priority()
                .cmp(&left.priority())
                .then_with(|| left.name().cmp(right.name()))
                .then_with(|| left.id().cmp(&right.id()))
        });

        Self {
            rules: registrations
                .iter()
                .map(|registration| registration.rule(context))
                .collect(),
        }
    }

    /// Register a build rule. Returns `&mut self` for chaining.
    pub fn register(&mut self, rule: BuildRule) -> &mut Self {
        self.rules.push(rule);
        self
    }

    /// First rule whose primary-source matcher claims this path. `None`
    /// when no rule matches. Prefer [`Self::matching_source`] for
    /// production scheduling so schema-specific builders sharing one
    /// extension can coexist.
    #[must_use]
    pub fn match_path(&self, source_path: &str) -> Option<&BuildRule> {
        self.rules.iter().find(|rule| rule.matches(source_path))
    }

    /// Rules whose path patterns and optional source-schema filters
    /// claim this source. Returned in registration order.
    pub fn matching_source<'a>(
        &'a self,
        source_path: &'a str,
        source_schema_type: Option<&'a str>,
    ) -> impl Iterator<Item = &'a BuildRule> + 'a {
        self.rules
            .iter()
            .filter(move |rule| rule.matches_source(source_path, source_schema_type))
    }

    /// Rule with the given stable id. Used by workers when a leased
    /// durable attempt names the exact rule that created the job.
    #[must_use]
    pub fn by_id(&self, id: BuilderId) -> Option<&BuildRule> {
        self.rules.iter().find(|rule| rule.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &BuildRule> {
        self.rules.iter()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.rules.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

impl Default for BuildRuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for BuildRuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildRuleRegistry")
            .field("len", &self.rules.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use uuid::uuid;

    use super::*;
    use crate::builder::{BuildRule, BuilderId, ProductFormatPolicy, SourceMatcher};
    use crate::job::{
        CreateJobsRequest, CreateJobsResponse, ProcessJobRequest, ProcessJobResponse,
    };
    use crate::pattern::AssetBuilderPattern;
    use crate::value::SourceSchemaType;

    const PREFAB_SCHEMA_TYPES: &[SourceSchemaType] =
        &[SourceSchemaType::__from_static("az.test.Prefab")];
    const MATERIAL_SCHEMA_TYPES: &[SourceSchemaType] =
        &[SourceSchemaType::__from_static("az.test.Material")];

    fn no_jobs(_: &CreateJobsRequest<'_>) -> CreateJobsResponse {
        CreateJobsResponse::default()
    }
    fn no_process(_: &ProcessJobRequest<'_>) -> ProcessJobResponse {
        ProcessJobResponse::default()
    }

    fn cgf_only_patterns() -> &'static [AssetBuilderPattern] {
        static PATTERNS: std::sync::OnceLock<Box<[AssetBuilderPattern]>> =
            std::sync::OnceLock::new();
        PATTERNS
            .get_or_init(|| vec![AssetBuilderPattern::wildcard("*.cgf")].into_boxed_slice())
            .as_ref()
    }

    fn mesh_patterns() -> &'static [AssetBuilderPattern] {
        static PATTERNS: std::sync::OnceLock<Box<[AssetBuilderPattern]>> =
            std::sync::OnceLock::new();
        PATTERNS
            .get_or_init(|| {
                vec![
                    AssetBuilderPattern::wildcard("*.cgf"),
                    AssetBuilderPattern::wildcard("*.cga"),
                ]
                .into_boxed_slice()
            })
            .as_ref()
    }

    fn ron_patterns() -> &'static [AssetBuilderPattern] {
        static PATTERNS: std::sync::OnceLock<Box<[AssetBuilderPattern]>> =
            std::sync::OnceLock::new();
        PATTERNS
            .get_or_init(|| vec![AssetBuilderPattern::wildcard("*.ron")].into_boxed_slice())
            .as_ref()
    }

    #[test]
    fn match_returns_first_winner() {
        // Two builders, the first claims `*.cgf` only — second claims
        // any `*.cgf` or `*.cga`. With first-wins ordering the first
        // builder must win for `.cgf`.
        let cgf_builder: BuildRule = BuildRule {
            name: "cgf-only",
            id: BuilderId::new(uuid!("00000000-0000-0000-0000-000000000001")),
            primary_source: SourceMatcher::PathOnly(cgf_only_patterns()),
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: no_jobs,
            process_job: no_process,
        };
        let mesh_builder: BuildRule = BuildRule {
            name: "mesh-any",
            id: BuilderId::new(uuid!("00000000-0000-0000-0000-000000000002")),
            primary_source: SourceMatcher::PathOnly(mesh_patterns()),
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: no_jobs,
            process_job: no_process,
        };

        let mut registry = BuildRuleRegistry::new();
        registry.register(cgf_builder).register(mesh_builder);

        assert_eq!(registry.match_path("foo.cgf").unwrap().name, "cgf-only");
        assert_eq!(registry.match_path("foo.cga").unwrap().name, "mesh-any");
        assert!(registry.match_path("foo.bar").is_none());
        assert_eq!(
            registry
                .by_id(BuilderId::new(uuid!(
                    "00000000-0000-0000-0000-000000000002"
                )))
                .unwrap()
                .name,
            "mesh-any"
        );
    }

    #[test]
    fn matching_source_filters_by_schema_type() {
        let prefab_builder: BuildRule = BuildRule {
            name: "prefab",
            id: BuilderId::new(uuid!("00000000-0000-0000-0000-000000000003")),
            primary_source: SourceMatcher::Schemas {
                patterns: ron_patterns(),
                schemas: PREFAB_SCHEMA_TYPES.into(),
            },
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: no_jobs,
            process_job: no_process,
        };
        let material_builder: BuildRule = BuildRule {
            name: "material",
            id: BuilderId::new(uuid!("00000000-0000-0000-0000-000000000004")),
            primary_source: SourceMatcher::Schemas {
                patterns: ron_patterns(),
                schemas: MATERIAL_SCHEMA_TYPES.into(),
            },
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: no_jobs,
            process_job: no_process,
        };

        let mut registry = BuildRuleRegistry::new();
        registry.register(prefab_builder).register(material_builder);

        let names = registry
            .matching_source("foo.ron", Some("az.test.Material"))
            .map(|builder| builder.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["material"]);
        assert!(
            registry
                .matching_source("foo.ron", Some("az.test.Unknown"))
                .next()
                .is_none()
        );
    }
}
