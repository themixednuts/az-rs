use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub use az_gem_contract::{CapSet, GemTargetRole, HostCapability};

use crate::manifest::{
    GemContribution, GemHome, GemManifest, ProjectManifestError, ResolvedProjectGem,
    validate_portable_package_id,
};

/// One capability a gem publishes, declared in both forms (ADR 0034).
///
/// `cargo_features` is the **source-build** form: when the consuming project
/// compiles the gem, a selected capability lowers to those Cargo features so
/// per-capability dead-code elimination survives. `activation` is the
/// **runtime-activation** form: when the gem arrives prebuilt, the project's
/// selection cannot change what was compiled, so it becomes a flag on the
/// generated typed activation struct instead. A gem declares both; the
/// resolver decides which one applies from the gem's provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GemCapability {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default)]
    pub default: bool,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contributions: Vec<String>,

    /// Source-build form: Cargo features this capability enables on the
    /// contributions it names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cargo_features: Vec<String>,

    /// Runtime-activation form: the flag name this capability occupies on the
    /// generated activation struct. Absent means the capability id is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation: Option<String>,
}

impl GemCapability {
    /// The runtime-activation flag this capability sets, defaulting to the
    /// capability id when the manifest names no flag of its own.
    #[must_use]
    pub fn activation_flag<'a>(&'a self, capability_id: &'a str) -> &'a str {
        self.activation.as_deref().unwrap_or(capability_id)
    }

    /// Check this declaration for the shapes the resolver cannot recover from.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectManifestError::InvalidGemCapability`] if `label` or
    /// `description` is present but blank, if a contribution id, cargo feature,
    /// or activation flag is not a portable package id, if a contribution or
    /// cargo feature is declared twice, or if `cargo_features` / `activation` is
    /// declared without naming at least one contribution to carry it.
    pub fn validate(&self, gem_id: &str, capability: &str) -> Result<(), ProjectManifestError> {
        if self
            .label
            .as_deref()
            .is_some_and(|label| label.trim().is_empty())
        {
            return Err(invalid_capability(
                gem_id,
                capability,
                "label cannot be empty when present",
            ));
        }
        if self
            .description
            .as_deref()
            .is_some_and(|description| description.trim().is_empty())
        {
            return Err(invalid_capability(
                gem_id,
                capability,
                "description cannot be empty when present",
            ));
        }
        for contribution in &self.contributions {
            validate_portable_package_id("gem contribution", contribution).map_err(|error| {
                invalid_capability(
                    gem_id,
                    capability,
                    format!("invalid contribution id: {error}"),
                )
            })?;
        }
        reject_duplicates(
            gem_id,
            capability,
            "contribution",
            self.contributions.iter().map(String::as_str),
        )?;
        for feature in &self.cargo_features {
            validate_portable_package_id("cargo feature", feature).map_err(|error| {
                invalid_capability(
                    gem_id,
                    capability,
                    format!("invalid cargo feature: {error}"),
                )
            })?;
        }
        reject_duplicates(
            gem_id,
            capability,
            "cargo feature",
            self.cargo_features.iter().map(String::as_str),
        )?;
        if !self.cargo_features.is_empty() && self.contributions.is_empty() {
            return Err(invalid_capability(
                gem_id,
                capability,
                "cargo feature lowering requires at least one named contribution",
            ));
        }
        if let Some(activation) = &self.activation {
            validate_portable_package_id("capability activation flag", activation).map_err(
                |error| {
                    invalid_capability(
                        gem_id,
                        capability,
                        format!("invalid activation flag: {error}"),
                    )
                },
            )?;
            if self.contributions.is_empty() {
                return Err(invalid_capability(
                    gem_id,
                    capability,
                    "runtime activation requires at least one named contribution",
                ));
            }
        }
        Ok(())
    }
}

/// Which of a capability's two declared forms a gem's provenance selects
/// (ADR 0034).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GemCapabilityMode {
    /// The consuming project compiles the gem, so capabilities lower to
    /// Cargo features.
    SourceBuild,
    /// The gem arrives as a prebuilt artifact compiled with its own declared
    /// set, so capabilities become runtime activation flags.
    RuntimeActivation,
}

impl GemCapabilityMode {
    /// Engine- and project-homed gems are built from source in the project's
    /// workspace. A registry-homed gem is delivered as an artifact, so its
    /// features were fixed when it was published.
    #[must_use]
    pub const fn for_gem_home(home: GemHome) -> Self {
        match home {
            GemHome::Engine | GemHome::Project => Self::SourceBuild,
            GemHome::Registry => Self::RuntimeActivation,
        }
    }
}

impl std::fmt::Display for GemCapabilityMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::SourceBuild => "source-build",
            Self::RuntimeActivation => "runtime-activation",
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GemCapabilityResolver<'a> {
    gems: &'a [ResolvedProjectGem],
}

impl<'a> GemCapabilityResolver<'a> {
    #[must_use]
    pub const fn new(gems: &'a [ResolvedProjectGem]) -> Self {
        Self { gems }
    }

    /// Lower every gem's selected capabilities into the form its provenance
    /// calls for, once, for all consumers to share.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectManifestError::UnknownGemCapability`] if any gem selects
    /// a capability id its own manifest does not declare.
    pub fn resolve(self) -> Result<ResolvedGemCapabilityPlan<'a>, ProjectManifestError> {
        self.gems
            .iter()
            .map(resolve_gem_capabilities)
            .collect::<Result<Vec<_>, _>>()
            .map(|gems| ResolvedGemCapabilityPlan { gems })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGemCapabilityPlan<'a> {
    pub gems: Vec<ResolvedGemCapabilitySet<'a>>,
}

impl<'a> ResolvedGemCapabilityPlan<'a> {
    #[must_use]
    pub fn for_gem(&self, gem_id: &str) -> Option<&ResolvedGemCapabilitySet<'a>> {
        self.gems
            .iter()
            .find(|selection| selection.gem.manifest.gem.id == gem_id)
    }
}

/// One gem's selected capabilities, already lowered into the form its
/// provenance calls for.
///
/// The lowering happens once, in [`GemCapabilityResolver::resolve`]: every
/// consumer — generated Cargo bridges, the lock, project-service
/// diagnostics — reads the same computed answer rather than re-deriving it
/// from the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGemCapabilitySet<'a> {
    pub gem: &'a ResolvedProjectGem,
    pub capabilities: Vec<SelectedGemCapability<'a>>,
    mode: GemCapabilityMode,
    cargo_features: BTreeMap<&'a str, Vec<&'a str>>,
    activation_flags: Vec<&'a str>,
}

impl<'a> ResolvedGemCapabilitySet<'a> {
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.capabilities.iter().map(|selection| selection.id)
    }

    /// Which declaration form this gem's provenance selects.
    #[must_use]
    pub const fn mode(&self) -> GemCapabilityMode {
        self.mode
    }

    /// The runtime-activation flags this selection raises.
    ///
    /// Empty for a source-built gem: there the selection was already spent on
    /// Cargo features, and raising the same choice twice would let a
    /// capability be compiled out and activated at once.
    #[must_use]
    pub fn activation_flags(&self) -> &[&'a str] {
        &self.activation_flags
    }

    #[must_use]
    pub fn cargo_features_for(&self, target_role: GemTargetRole) -> Vec<&'a str> {
        self.gem
            .manifest
            .contributions
            .iter()
            .filter(|contribution| contribution.roles.contains(&target_role))
            .filter_map(|contribution| self.cargo_features.get(contribution.id.as_str()))
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Returns the Cargo features activated specifically for one contribution.
    ///
    /// A gem may contribute several Cargo packages to the same generated
    /// target. Capability features belong only to the contributions named by
    /// that capability; applying the role-wide union to every package would
    /// make unrelated role packages accept each other's feature vocabulary.
    #[must_use]
    pub fn cargo_features_for_contribution(&self, contribution_id: &str) -> Vec<&'a str> {
        self.cargo_features
            .get(contribution_id)
            .cloned()
            .unwrap_or_default()
    }

    /// The runtime-activation flags raised for one contribution.
    ///
    /// The generated glue passes exactly these to that contribution's entry
    /// item as its product-capability activation. Empty for a source-built
    /// gem, whose selection was already spent on Cargo features — the mirror
    /// of [`Self::cargo_features_for_contribution`], never both at once.
    #[must_use]
    pub fn activation_flags_for_contribution(&self, contribution_id: &str) -> Vec<&'a str> {
        if self.mode != GemCapabilityMode::RuntimeActivation {
            return Vec::new();
        }
        self.capabilities
            .iter()
            .filter(|selection| {
                selection
                    .capability
                    .contributions
                    .iter()
                    .any(|named| named == contribution_id)
            })
            .map(|selection| selection.capability.activation_flag(selection.id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[must_use]
    pub fn cargo_features_for_any(
        &self,
        target_roles: impl IntoIterator<Item = GemTargetRole>,
    ) -> Vec<&'a str> {
        target_roles
            .into_iter()
            .flat_map(|role| self.cargo_features_for(role))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[must_use]
    pub fn contributions_for(&self, target_role: GemTargetRole) -> Vec<&GemContribution> {
        active_contributions_for_manifest(
            &self.gem.manifest,
            self.capabilities.iter().map(|selection| selection.id),
            target_role,
        )
    }

    #[must_use]
    pub fn contributes_to(&self, target_role: GemTargetRole) -> bool {
        !self.contributions_for(target_role).is_empty()
    }

    #[must_use]
    pub fn contributes_to_any(
        &self,
        target_roles: impl IntoIterator<Item = GemTargetRole>,
    ) -> bool {
        target_roles
            .into_iter()
            .any(|role| self.contributes_to(role))
    }

    #[must_use]
    pub fn applies_to(&self, target_role: GemTargetRole) -> bool {
        self.contributes_to(target_role)
    }
}

/// Tests a locked/selected capability set against one target role.
///
/// This is shared by graph resolution, generated Cargo bridges, and running
/// project-service diagnostics so all three boundaries agree about which gem
/// inventories should be linked.
#[must_use]
pub fn selected_gem_contributions_apply_to(
    manifest: &GemManifest,
    selected_capabilities: &[String],
    target_role: GemTargetRole,
) -> bool {
    !active_contributions_for_manifest(
        manifest,
        selected_capabilities.iter().map(String::as_str),
        target_role,
    )
    .is_empty()
}

#[must_use]
pub fn selected_gem_contributions_for_role<'a>(
    manifest: &'a GemManifest,
    selected_capabilities: &[String],
    target_role: GemTargetRole,
) -> Vec<&'a GemContribution> {
    active_contributions_for_manifest(
        manifest,
        selected_capabilities.iter().map(String::as_str),
        target_role,
    )
}

#[must_use]
pub fn selected_gem_contributions_apply_to_any(
    manifest: &GemManifest,
    selected_capabilities: &[String],
    target_roles: impl IntoIterator<Item = GemTargetRole>,
) -> bool {
    target_roles
        .into_iter()
        .any(|role| selected_gem_contributions_apply_to(manifest, selected_capabilities, role))
}

/// Compatibility API name retained while callers migrate to contribution
/// terminology.
///
/// Unlike the old implementation, a capability-free manifest does not apply to
/// any role unless it declares an unconditional contribution.
#[must_use]
pub fn selected_gem_capabilities_apply_to(
    manifest: &GemManifest,
    selected_capabilities: &[String],
    target_role: GemTargetRole,
) -> bool {
    selected_gem_contributions_apply_to(manifest, selected_capabilities, target_role)
}

/// The role a cargo feature names, if it names one.
///
/// Roles are alias bundles on the build side, so a feature spelled like a role
/// is that role's build-time mirror. Anything else is an ordinary product
/// feature and answers no question about the host.
#[must_use]
pub fn feature_role(feature: &str) -> Option<GemTargetRole> {
    GemTargetRole::deserialize(
        serde::de::value::StrDeserializer::<serde::de::value::Error>::new(feature),
    )
    .ok()
}

/// A target's role disagreeing with what it composes — the generation-time
/// coherence assertion ADR 0036 owes and ADR 0041 defines.
///
/// Both variants are the same incoherence seen from the two vocabularies: a
/// host capability the role does not have, reached either through a
/// contribution's declared floor or through a cargo feature that names another
/// role. Both are generation errors, never warnings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoherence {
    /// A selected contribution requires a host capability this role lacks.
    Floor {
        gem: String,
        contribution: String,
        role: GemTargetRole,
        required: CapSet,
        provided: CapSet,
    },
    /// A selected cargo feature names a role this role does not cover.
    Feature {
        package: String,
        feature: String,
        named: GemTargetRole,
        role: GemTargetRole,
        provided: CapSet,
    },
}

impl std::fmt::Display for Incoherence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Floor {
                gem,
                contribution,
                role,
                required,
                provided,
            } => write!(
                formatter,
                "gem `{gem}` contribution `{contribution}` requires {required} but role \
                 `{role}` provides {provided} (missing {})",
                required.missing_from(*provided)
            ),
            Self::Feature {
                package,
                feature,
                named,
                role,
                provided,
            } => write!(
                formatter,
                "package `{package}` selects cargo feature `{feature}`, which names role \
                 `{named}` requiring {}, but role `{role}` provides {provided} (missing {})",
                named.provided_caps(),
                named.provided_caps().missing_from(*provided)
            ),
        }
    }
}

impl std::error::Error for Incoherence {}

/// The floor half of the coherence assertion.
#[must_use]
pub fn floor_gap(
    gem_id: &str,
    contribution: &GemContribution,
    role: GemTargetRole,
    provided: CapSet,
) -> Option<Incoherence> {
    let required = contribution.floor();
    if provided.contains(required) {
        return None;
    }
    Some(Incoherence::Floor {
        gem: gem_id.to_string(),
        contribution: contribution.id.clone(),
        role,
        required,
        provided,
    })
}

/// The feature half of the coherence assertion: a role and its selected
/// feature set must name the same facts.
#[must_use]
pub fn feature_gap(
    package: &str,
    feature: &str,
    role: GemTargetRole,
    provided: CapSet,
) -> Option<Incoherence> {
    let named = feature_role(feature)?;
    if provided.contains(named.provided_caps()) {
        return None;
    }
    Some(Incoherence::Feature {
        package: package.to_string(),
        feature: feature.to_string(),
        named,
        role,
        provided,
    })
}

fn active_contributions_for_manifest<'manifest, 'selection>(
    manifest: &'manifest GemManifest,
    selected_capabilities: impl IntoIterator<Item = &'selection str>,
    target_role: GemTargetRole,
) -> Vec<&'manifest GemContribution> {
    let mut selected = selected_capabilities
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    selected.extend(
        manifest
            .capabilities
            .iter()
            .filter(|(_, capability)| capability.default)
            .map(|(id, _)| id.clone()),
    );
    manifest
        .contributions
        .iter()
        .filter(|contribution| contribution.roles.contains(&target_role))
        .filter(|contribution| {
            let mut activating_capabilities = manifest
                .capabilities
                .iter()
                .filter(|(_, capability)| capability.contributions.contains(&contribution.id));
            let Some((first_id, _)) = activating_capabilities.next() else {
                return true;
            };
            selected.contains(first_id.as_str())
                || activating_capabilities.any(|(id, _)| selected.contains(id.as_str()))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedGemCapability<'a> {
    pub id: &'a str,
    pub capability: &'a GemCapability,
}

/// Resolve one gem's capabilities for their errors alone, discarding the plan.
///
/// # Errors
///
/// Returns [`ProjectManifestError::UnknownGemCapability`] if `gem` selects a
/// capability id its manifest does not declare.
pub fn validate_resolved_gem_capabilities(
    gem: &ResolvedProjectGem,
) -> Result<(), ProjectManifestError> {
    resolve_gem_capabilities(gem).map(|_| ())
}

#[must_use]
pub fn selected_capability_ids_for_gem(gem: &ResolvedProjectGem) -> Vec<String> {
    selected_capability_id_set(gem)
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn resolve_gem_capabilities(
    gem: &ResolvedProjectGem,
) -> Result<ResolvedGemCapabilitySet<'_>, ProjectManifestError> {
    let capabilities = selected_capability_id_set(gem)
        .into_iter()
        .map(|id| {
            let capability = gem.manifest.capabilities.get(id).ok_or_else(|| {
                ProjectManifestError::UnknownGemCapability {
                    gem_id: gem.manifest.gem.id.clone(),
                    capability: id.to_string(),
                }
            })?;
            Ok(SelectedGemCapability { id, capability })
        })
        .collect::<Result<Vec<_>, ProjectManifestError>>()?;

    let mode = GemCapabilityMode::for_gem_home(gem.provenance.home);
    let mut cargo_features: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut activation_flags = BTreeSet::new();
    for selection in &capabilities {
        match mode {
            GemCapabilityMode::SourceBuild => {
                for contribution in &selection.capability.contributions {
                    cargo_features
                        .entry(contribution.as_str())
                        .or_default()
                        .extend(
                            selection
                                .capability
                                .cargo_features
                                .iter()
                                .map(String::as_str),
                        );
                }
            }
            GemCapabilityMode::RuntimeActivation => {
                activation_flags.insert(selection.capability.activation_flag(selection.id));
            }
        }
    }

    Ok(ResolvedGemCapabilitySet {
        gem,
        capabilities,
        mode,
        cargo_features: cargo_features
            .into_iter()
            .map(|(contribution, features)| (contribution, features.into_iter().collect()))
            .collect(),
        activation_flags: activation_flags.into_iter().collect(),
    })
}

fn selected_capability_id_set(gem: &ResolvedProjectGem) -> BTreeSet<&str> {
    default_capability_ids(gem)
        .chain(gem.declaration.capabilities.iter().map(String::as_str))
        .collect()
}

fn default_capability_ids(gem: &ResolvedProjectGem) -> impl Iterator<Item = &str> {
    gem.manifest
        .capabilities
        .iter()
        .filter(|(_, capability)| capability.default)
        .map(|(id, _)| id.as_str())
}

fn reject_duplicates<T>(
    gem_id: &str,
    capability: &str,
    item_name: &str,
    values: impl IntoIterator<Item = T>,
) -> Result<(), ProjectManifestError>
where
    T: Ord,
{
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(invalid_capability(
                gem_id,
                capability,
                format!("{item_name} is declared more than once"),
            ));
        }
    }
    Ok(())
}

fn invalid_capability(
    gem_id: &str,
    capability: &str,
    reason: impl Into<String>,
) -> ProjectManifestError {
    ProjectManifestError::InvalidGemCapability {
        gem_id: gem_id.to_string(),
        capability: capability.to_string(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::manifest::{
        GemContribution, GemHome, GemManifest, GemProvenance, ProjectGem, ResolvedGemKind,
        ResolvedProjectGem,
    };

    use super::*;

    #[test]
    fn resolves_selected_capability_features_for_target_roles() {
        let gem = resolved_gem_with_capabilities(
            &["client-runtime", "anti-cheat-client"],
            [
                ("client-runtime", capability(["client"], ["runtime"], false)),
                (
                    "anti-cheat-client",
                    capability(["client"], ["runtime", "anti-cheat-client"], false),
                ),
                ("server-runtime", capability(["server"], ["runtime"], false)),
            ],
        );

        let plan = GemCapabilityResolver::new(std::slice::from_ref(&gem))
            .resolve()
            .unwrap();
        let selection = plan.for_gem("azoth.platform").unwrap();

        assert_eq!(
            selection.ids().collect::<Vec<_>>(),
            vec!["anti-cheat-client", "client-runtime"]
        );
        assert_eq!(
            selection.cargo_features_for(GemTargetRole::Client),
            vec!["anti-cheat-client", "runtime"]
        );
        assert!(
            selection
                .cargo_features_for(GemTargetRole::ProjectHost)
                .is_empty()
        );
        assert!(selection.contributes_to(GemTargetRole::Client));
        assert!(!selection.contributes_to(GemTargetRole::ProjectHost));
    }

    #[test]
    fn includes_default_capabilities_without_project_selection() {
        let gem = resolved_gem_with_capabilities(
            &[],
            [
                (
                    "base-runtime",
                    capability(["client", "server"], ["runtime"], true),
                ),
                ("client-runtime", capability(["client"], ["client"], false)),
            ],
        );

        let plan = GemCapabilityResolver::new(std::slice::from_ref(&gem))
            .resolve()
            .unwrap();
        let selection = plan.for_gem("azoth.platform").unwrap();

        assert_eq!(selection.ids().collect::<Vec<_>>(), vec!["base-runtime"]);
        assert_eq!(
            selection.cargo_features_for(GemTargetRole::Server),
            vec!["runtime"]
        );
        assert!(selection.applies_to(GemTargetRole::Server));
    }

    #[test]
    fn cargo_features_are_scoped_to_the_contribution_that_activates_them() {
        let gem = resolved_gem_with_capabilities(
            &["client-runtime", "client-overlay"],
            [
                ("client-runtime", capability(["client"], ["runtime"], false)),
                (
                    "client-overlay",
                    capability(["overlay"], ["overlay"], false),
                ),
            ],
        );

        let plan = GemCapabilityResolver::new(std::slice::from_ref(&gem))
            .resolve()
            .unwrap();
        let selection = plan.for_gem("azoth.platform").unwrap();

        assert_eq!(
            selection.cargo_features_for(GemTargetRole::Client),
            vec!["overlay", "runtime"]
        );
        assert_eq!(
            selection.cargo_features_for_contribution("client"),
            vec!["runtime"]
        );
        assert_eq!(
            selection.cargo_features_for_contribution("overlay"),
            vec!["overlay"]
        );
    }

    #[test]
    fn contribution_free_gems_are_inert_for_every_role() {
        let gem = resolved_gem_with_capabilities(&[], []);
        let plan = GemCapabilityResolver::new(std::slice::from_ref(&gem))
            .resolve()
            .unwrap();
        let selection = plan.for_gem("azoth.platform").unwrap();

        assert!(!selection.contributes_to(GemTargetRole::ProjectHost));
        assert!(!selection.contributes_to(GemTargetRole::Client));
    }

    #[test]
    fn rejects_unknown_selected_capability() {
        let gem = resolved_gem_with_capabilities(
            &["missing"],
            [("client-runtime", capability(["client"], ["runtime"], false))],
        );

        let error = GemCapabilityResolver::new(std::slice::from_ref(&gem))
            .resolve()
            .unwrap_err();

        assert!(matches!(
            error,
            ProjectManifestError::UnknownGemCapability { gem_id, capability }
                if gem_id == "azoth.platform" && capability == "missing"
        ));
    }

    #[test]
    fn source_built_gems_resolve_capabilities_to_cargo_features_only() {
        let gem = resolved_gem_with_capabilities(
            &["client-runtime"],
            [(
                "client-runtime",
                capability_with_activation(["client"], ["runtime"], Some("client_runtime")),
            )],
        );

        let plan = GemCapabilityResolver::new(std::slice::from_ref(&gem))
            .resolve()
            .unwrap();
        let selection = plan.for_gem("azoth.platform").unwrap();

        assert_eq!(selection.mode(), GemCapabilityMode::SourceBuild);
        assert_eq!(
            selection.cargo_features_for(GemTargetRole::Client),
            vec!["runtime"]
        );
        assert!(selection.activation_flags().is_empty());
    }

    #[test]
    fn prebuilt_gems_resolve_capabilities_to_activation_flags_only() {
        let mut gem = resolved_gem_with_capabilities(
            &["client-runtime", "client-overlay"],
            [
                (
                    "client-runtime",
                    capability_with_activation(["client"], ["runtime"], Some("client_runtime")),
                ),
                (
                    "client-overlay",
                    capability_with_activation(["overlay"], ["overlay"], None),
                ),
            ],
        );
        gem.provenance.home = GemHome::Registry;

        let plan = GemCapabilityResolver::new(std::slice::from_ref(&gem))
            .resolve()
            .unwrap();
        let selection = plan.for_gem("azoth.platform").unwrap();

        assert_eq!(selection.mode(), GemCapabilityMode::RuntimeActivation);
        // An unnamed activation flag falls back to the capability id.
        assert_eq!(
            selection.activation_flags(),
            ["client-overlay", "client_runtime"]
        );
        assert!(
            selection
                .cargo_features_for(GemTargetRole::Client)
                .is_empty()
        );
        assert!(
            selection
                .cargo_features_for_contribution("client")
                .is_empty()
        );
    }

    #[test]
    fn activation_flags_must_be_portable_and_name_a_contribution() {
        let mut capability =
            capability_with_activation(["client"], ["runtime"], Some("not a flag"));
        let error = capability
            .validate("azoth.platform", "client-runtime")
            .unwrap_err();
        assert!(matches!(
            error,
            ProjectManifestError::InvalidGemCapability { ref reason, .. }
                if reason.contains("activation flag")
        ));

        capability.activation = Some("client_runtime".to_string());
        capability.contributions.clear();
        capability.cargo_features.clear();
        let error = capability
            .validate("azoth.platform", "client-runtime")
            .unwrap_err();
        assert!(matches!(
            error,
            ProjectManifestError::InvalidGemCapability { ref reason, .. }
                if reason.contains("runtime activation requires")
        ));
    }

    fn resolved_gem_with_capabilities<const N: usize, const M: usize>(
        selected: &[&str; N],
        capabilities: [(&str, GemCapability); M],
    ) -> ResolvedProjectGem {
        let mut manifest = GemManifest::new("azoth.platform", "Platform", "0.1.0");
        manifest.capabilities = capabilities
            .into_iter()
            .map(|(id, capability)| (id.to_string(), capability))
            .collect();
        let contribution_ids = manifest
            .capabilities
            .values()
            .flat_map(|capability| capability.contributions.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        manifest.contributions = contribution_ids
            .into_iter()
            .map(|id| {
                let role = match id.as_str() {
                    "client" | "overlay" => GemTargetRole::Client,
                    "server" => GemTargetRole::Server,
                    _ => unreachable!("test contribution role"),
                };
                GemContribution::code(id, "az-gem-platform", [role])
            })
            .collect();

        ResolvedProjectGem {
            declaration: ProjectGem {
                id: "azoth.platform".to_string(),
                enabled: true,
                capabilities: selected.iter().map(|value| (*value).to_string()).collect(),
                linkage: None,
                path: None,
            },
            kind: ResolvedGemKind::Engine,
            provenance: GemProvenance {
                home: GemHome::Engine,
                catalog_id: "azoth".to_string(),
                catalog_path: PathBuf::from("engine.toml"),
                manifest_path: PathBuf::from("gems/platform/gem.toml"),
                manifest_checksum: "blake3:test".to_string(),
            },
            root: PathBuf::from("gems/platform"),
            lock_root: PathBuf::from("gems/platform"),
            manifest,
        }
    }

    fn capability<const N: usize, const M: usize>(
        target_roles: [&str; N],
        cargo_features: [&str; M],
        default: bool,
    ) -> GemCapability {
        GemCapability {
            label: None,
            description: None,
            default,
            contributions: target_roles.into_iter().map(str::to_string).collect(),
            cargo_features: cargo_features.into_iter().map(str::to_string).collect(),
            activation: None,
        }
    }

    fn capability_with_activation<const N: usize, const M: usize>(
        target_roles: [&str; N],
        cargo_features: [&str; M],
        activation: Option<&str>,
    ) -> GemCapability {
        GemCapability {
            activation: activation.map(str::to_string),
            ..capability(target_roles, cargo_features, false)
        }
    }
}
