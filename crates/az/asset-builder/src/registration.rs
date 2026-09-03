//! Build-rule contribution.
//!
//! Engine, project, and gem crates contribute build rules through their
//! contribution's registrar; the host that composed them owns the resulting
//! [`crate::BuildRuleRegistry`]. Nothing is discovered at link time, so a
//! process no longer sees rules from every crate that happens to be linked
//! into it — it sees exactly what its own composition registered.

use az_gem_contract::{RegistryEntry, Unconditional};

use crate::builder::{BuildRule, BuilderId};
use crate::job::JobContext;

/// Resolves one contributed rule against the host's composition.
///
/// A rule's claim and analysis fingerprint can both depend on what else
/// composed, and neither is knowable while composition is still running, so
/// the contribution registers this and the host calls it once composition is
/// complete.
pub type BuildRuleFactory = fn(&JobContext<'_>) -> BuildRule;

/// One build rule contributed to a host.
///
/// [`BuilderId`] is the compose key because it is the identity a stored job
/// attempt names when it says which rule produced it; two rules under one id
/// would each claim the other's attempts. The name orders same-priority rules
/// and is the default job key.
///
/// Identity lives here rather than only inside the resolved [`BuildRule`]
/// because the host keys, orders and dispatches on it before any resolution
/// has happened.
#[derive(Clone, Copy)]
pub struct BuildRuleRegistration {
    name: &'static str,
    id: BuilderId,
    priority: i32,
    rule: BuildRuleFactory,
}

impl BuildRuleRegistration {
    /// Contribute a build rule at default priority.
    #[must_use]
    pub const fn new(name: &'static str, id: BuilderId, rule: BuildRuleFactory) -> Self {
        Self {
            name,
            id,
            priority: 0,
            rule,
        }
    }

    /// Override dispatch priority. Higher priority builders are registered
    /// first, which keeps diagnostics and builder-catalog output stable when
    /// multiple builders match one source.
    ///
    /// This is the one registry in the asset-builder family that orders its
    /// entries: dispatch returns the first matching rule, so the order is the
    /// answer, not a presentation detail.
    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    #[must_use]
    pub const fn id(&self) -> BuilderId {
        self.id
    }

    #[must_use]
    pub const fn priority(&self) -> i32 {
        self.priority
    }

    /// Resolve the rule against a composed host.
    #[must_use]
    pub fn rule(&self, context: &JobContext<'_>) -> BuildRule {
        (self.rule)(context)
    }
}

impl RegistryEntry for BuildRuleRegistration {
    type Key = BuilderId;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "build-rule"
    }

    fn key(&self) -> Self::Key {
        self.id
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use az_gem_contract::{
        ComposeError, Composer, Contribution, ContributionDescriptor, ContributionId, GemContext,
        GemId, GemTargetRole, ProductActivation, Registries, declare_caps,
    };
    use uuid::uuid;

    use super::*;
    use crate::builder::{BuildRule, BuilderId, ProductFormatPolicy, SourceMatcher};
    use crate::job::{
        CreateJobsRequest, CreateJobsResponse, ProcessJobRequest, ProcessJobResponse,
    };
    use crate::pattern::AssetBuilderPattern;
    use crate::registry::BuildRuleRegistry;
    use crate::value::SourceSchemaType;

    const PREFAB_SCHEMA_TYPES: &[SourceSchemaType] =
        &[SourceSchemaType::__from_static("az.test.Prefab")];

    const BROAD_RON_ID: BuilderId = BuilderId::new(uuid!("00000000-0000-0000-0000-00000000a001"));
    const PREFAB_ID: BuilderId = BuilderId::new(uuid!("00000000-0000-0000-0000-00000000a002"));

    const RULES: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.asset-builder-tests"),
        contribution: ContributionId::new("builders"),
        roles: &[],
    };
    const RIVAL: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.asset-builder-rival"),
        contribution: ContributionId::new("builders"),
        roles: &[],
    };

    fn no_jobs(_: &CreateJobsRequest<'_>) -> CreateJobsResponse {
        CreateJobsResponse::default()
    }

    fn no_process(_: &ProcessJobRequest<'_>) -> ProcessJobResponse {
        ProcessJobResponse::default()
    }

    fn broad_patterns() -> &'static [AssetBuilderPattern] {
        static PATTERNS: OnceLock<Box<[AssetBuilderPattern]>> = OnceLock::new();
        PATTERNS
            .get_or_init(|| vec![AssetBuilderPattern::wildcard("*.ron")].into_boxed_slice())
            .as_ref()
    }

    fn prefab_patterns() -> &'static [AssetBuilderPattern] {
        static PATTERNS: OnceLock<Box<[AssetBuilderPattern]>> = OnceLock::new();
        PATTERNS
            .get_or_init(|| vec![AssetBuilderPattern::wildcard("*.prefab.ron")].into_boxed_slice())
            .as_ref()
    }

    // Registered as a `BuildRuleFactory` function pointer, so the `&JobContext`
    // parameter is fixed by that type alias; taking it by value would not compile.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn broad_ron_rule(_: &JobContext<'_>) -> BuildRule {
        BuildRule {
            name: "az.test.broad-ron",
            id: BROAD_RON_ID,
            primary_source: SourceMatcher::PathOnly(broad_patterns()),
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: no_jobs,
            process_job: no_process,
        }
    }

    // Registered as a `BuildRuleFactory` function pointer, so the `&JobContext`
    // parameter is fixed by that type alias; taking it by value would not compile.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn prefab_rule(_: &JobContext<'_>) -> BuildRule {
        BuildRule {
            name: "az.test.prefab",
            id: PREFAB_ID,
            primary_source: SourceMatcher::Schemas {
                patterns: prefab_patterns(),
                schemas: PREFAB_SCHEMA_TYPES.into(),
            },
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: no_jobs,
            process_job: no_process,
        }
    }

    declare_caps!(BuilderCaps:);

    struct Rules;

    impl Contribution for Rules {
        type Caps = BuilderCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            RULES
        }

        fn register(&self, ctx: &mut GemContext<'_, BuilderCaps>) {
            ctx.registrar::<BuildRuleRegistration>().register_many([
                BuildRuleRegistration::new("az.test.broad-ron", BROAD_RON_ID, broad_ron_rule),
                BuildRuleRegistration::new("az.test.prefab", PREFAB_ID, prefab_rule)
                    .with_priority(100),
            ]);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Rules, ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    #[test]
    fn composed_rules_are_deterministic_and_priority_ordered() {
        let composer = composed();
        let registries = composer.registries();
        let registry = BuildRuleRegistry::compose(&JobContext::new(registries));
        let names = registry
            .iter()
            .map(|builder| builder.name)
            .collect::<Vec<_>>();

        assert_eq!(names, ["az.test.prefab", "az.test.broad-ron"]);
        assert_eq!(
            registry.match_path("levels/camp.prefab.ron").unwrap().name,
            "az.test.prefab"
        );
    }

    #[test]
    fn every_composed_rule_is_attributed_to_its_contribution() {
        let report = composed().finalize().expect("composition is valid");
        let rules = report
            .entries
            .iter()
            .filter(|entry| entry.registry == "build-rule")
            .collect::<Vec<_>>();

        assert_eq!(rules.len(), 2);
        assert!(rules.iter().all(|entry| {
            entry.instance.gem.as_str() == "azoth.asset-builder-tests"
                && entry.instance.contribution.as_str() == "builders"
        }));
    }

    #[test]
    fn two_contributions_claiming_one_builder_id_fail_composition() {
        struct Rival;

        impl Contribution for Rival {
            type Caps = BuilderCaps;

            fn descriptor(&self) -> ContributionDescriptor {
                RIVAL
            }

            fn register(&self, ctx: &mut GemContext<'_, BuilderCaps>) {
                // A different name under the same builder id: the id is what
                // a stored job attempt names, so this is the collision that
                // matters, not the display name.
                ctx.registrar::<BuildRuleRegistration>()
                    .register(BuildRuleRegistration::new(
                        "az.test.rival-prefab",
                        PREFAB_ID,
                        prefab_rule,
                    ));
            }
        }

        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer.add(Rules, ProductActivation::default()).unwrap();
        composer.add(Rival, ProductActivation::default()).unwrap();

        let ComposeError::Duplicate {
            registry,
            key,
            first,
            second,
        } = composer.finalize().unwrap_err()
        else {
            panic!("a repeated builder id must fail composition");
        };
        assert_eq!(registry, "build-rule");
        assert_eq!(key, PREFAB_ID.to_string());
        assert_eq!(first.gem.as_str(), "azoth.asset-builder-tests");
        assert_eq!(second.gem.as_str(), "azoth.asset-builder-rival");
    }

    #[test]
    fn a_host_that_composed_nothing_dispatches_nothing() {
        let registries = Registries::new();
        assert!(BuildRuleRegistry::compose(&JobContext::new(&registries)).is_empty());
    }
}
