//! Semantics for the Bevy-native [`PrefabDocument`](crate::PrefabDocument).

mod composition;
mod dependency;
mod validation;

use std::collections::BTreeMap;

use bevy_reflect::TypeRegistry;
use thiserror::Error;

use crate::{
    EntityAlias, InstanceAlias, PrefabAssetPath, PrefabDocument, ReflectedPath,
    document::OverrideAction,
};

pub use composition::{ResolvedOverride, ResolvedOverrideTarget};

/// Source lookup supplied by project-host or the Prefab builder.
pub trait TypedPrefabSourceResolver {
    fn resolve(&self, source: &PrefabAssetPath) -> Option<&PrefabDocument>;
}

impl TypedPrefabSourceResolver for BTreeMap<PrefabAssetPath, PrefabDocument> {
    fn resolve(&self, source: &PrefabAssetPath) -> Option<&PrefabDocument> {
        self.get(source)
    }
}

/// Complete typed authoring semantics. Runtime consumes only the cooked result.
#[derive(Debug, Clone, Copy, Default)]
pub struct TypedPrefabSemantics;

impl TypedPrefabSemantics {
    /// Validates one document in isolation, without resolving its nested
    /// Prefab sources.
    ///
    /// # Errors
    ///
    /// Returns [`TypedPrefabSemanticsError::AliasCollision`],
    /// [`TypedPrefabSemanticsError::DanglingParent`],
    /// [`TypedPrefabSemanticsError::DanglingInstanceParent`],
    /// [`TypedPrefabSemanticsError::HierarchyCycle`],
    /// [`TypedPrefabSemanticsError::ComponentTypeMismatch`],
    /// [`TypedPrefabSemanticsError::MissingCanonicalTypeVersion`],
    /// [`TypedPrefabSemanticsError::UnusedCanonicalTypeVersion`], or
    /// [`TypedPrefabSemanticsError::Registry`] for a locally inconsistent
    /// document.
    pub fn validate_local(
        document: &PrefabDocument,
        registry: &TypeRegistry,
    ) -> Result<(), TypedPrefabSemanticsError> {
        validation::validate_local(document, registry)
    }

    /// Validates the document, every nested source it reaches, and the
    /// override precedence that composing them produces.
    ///
    /// # Errors
    ///
    /// Forwards every error of [`Self::validate_local`] for the document and
    /// each nested source, of [`Self::dependency_order`], and of
    /// [`Self::resolve_overrides`].
    pub fn validate(
        source: &PrefabAssetPath,
        document: &PrefabDocument,
        resolver: &impl TypedPrefabSourceResolver,
        registry: &TypeRegistry,
    ) -> Result<(), TypedPrefabSemanticsError> {
        validation::validate_local(document, registry)?;
        let dependencies = dependency::dependency_order(source, document, resolver)?;
        for dependency in dependencies {
            let nested = resolver.resolve(&dependency).ok_or_else(|| {
                TypedPrefabSemanticsError::MissingDependency(dependency.to_string())
            })?;
            validation::validate_local(nested, registry)?;
        }
        composition::resolve_override_precedence(source, document, resolver, registry)?;
        Ok(())
    }

    /// Returns every nested source this document depends on, deepest first and
    /// excluding `source` itself.
    ///
    /// # Errors
    ///
    /// Returns [`TypedPrefabSemanticsError::MissingDependency`] when `resolver`
    /// cannot supply a nested source, and
    /// [`TypedPrefabSemanticsError::DependencyCycle`] when the sources form a
    /// cycle.
    pub fn dependency_order(
        source: &PrefabAssetPath,
        document: &PrefabDocument,
        resolver: &impl TypedPrefabSourceResolver,
    ) -> Result<Vec<PrefabAssetPath>, TypedPrefabSemanticsError> {
        dependency::dependency_order(source, document, resolver)
    }

    /// Walks the instance tree and returns the winning override for every
    /// fully-qualified target.
    ///
    /// # Errors
    ///
    /// Returns [`TypedPrefabSemanticsError::MissingDependency`] and
    /// [`TypedPrefabSemanticsError::DependencyCycle`] while walking nested
    /// sources, [`TypedPrefabSemanticsError::SameLayerConflict`] when one layer
    /// authors the same target twice, and — while validating each override —
    /// [`TypedPrefabSemanticsError::DanglingOverrideInstance`],
    /// [`TypedPrefabSemanticsError::DanglingOverrideEntity`],
    /// [`TypedPrefabSemanticsError::DanglingOverrideComponent`],
    /// [`TypedPrefabSemanticsError::Registry`],
    /// [`TypedPrefabSemanticsError::InvalidOverridePath`],
    /// [`TypedPrefabSemanticsError::OverrideCollectionRequired`], or
    /// [`TypedPrefabSemanticsError::OverrideValueTypeMismatch`].
    pub fn resolve_overrides(
        source: &PrefabAssetPath,
        document: &PrefabDocument,
        resolver: &impl TypedPrefabSourceResolver,
        registry: &TypeRegistry,
    ) -> Result<Vec<ResolvedOverride>, TypedPrefabSemanticsError> {
        composition::resolve_override_precedence(source, document, resolver, registry)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TypedPrefabSemanticsError {
    #[error("entity alias `{0}` collides with an instance alias in the same Prefab")]
    AliasCollision(String),
    #[error("entity `{entity}` has dangling parent `{parent}`")]
    DanglingParent { entity: String, parent: String },
    #[error("instance `{instance}` has dangling parent entity `{parent}`")]
    DanglingInstanceParent { instance: String, parent: String },
    #[error("entity hierarchy cycle: {aliases:?}")]
    HierarchyCycle { aliases: Vec<String> },
    #[error(
        "component tag `{tag}` on entity `{entity}` resolves to `{registered}`, but the value represents `{actual}`"
    )]
    ComponentTypeMismatch {
        entity: String,
        tag: String,
        registered: String,
        actual: String,
    },
    #[error(
        "component tag `{tag}` on entity `{entity}` has no matching canonical type_versions entry"
    )]
    MissingCanonicalTypeVersion { entity: String, tag: String },
    #[error("type_versions contains unused canonical tag `{0}`")]
    UnusedCanonicalTypeVersion(String),
    #[error("invalid Prefab registry: {0}")]
    Registry(String),
    #[error("nested Prefab source `{0}` is missing")]
    MissingDependency(String),
    #[error("nested Prefab dependency cycle: {sources:?}")]
    DependencyCycle { sources: Vec<String> },
    #[error("override has dangling instance alias `{instance}` below `{asset_source}`")]
    DanglingOverrideInstance {
        asset_source: String,
        instance: String,
    },
    #[error("override targets missing entity `{entity}` in `{asset_source}`")]
    DanglingOverrideEntity {
        asset_source: String,
        entity: String,
    },
    #[error("override targets missing component `{component}` on `{entity}` in `{asset_source}`")]
    DanglingOverrideComponent {
        asset_source: String,
        entity: String,
        component: String,
    },
    #[error("override path `{path}` is invalid for `{component}`: {reason}")]
    InvalidOverridePath {
        component: String,
        path: String,
        reason: String,
    },
    #[error("override action {action} requires collection target `{path}`")]
    OverrideCollectionRequired { action: &'static str, path: String },
    #[error("override value at `{path}` represents `{actual}`, expected `{expected}`")]
    OverrideValueTypeMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("same-layer override conflict at `{target}`")]
    SameLayerConflict { target: String },
}

pub const fn action_name(action: &OverrideAction) -> &'static str {
    match action {
        OverrideAction::Set(_) => "Set",
        OverrideAction::Clear => "Clear",
        OverrideAction::Insert { .. } => "Insert",
        OverrideAction::Remove { .. } => "Remove",
        OverrideAction::Move { .. } => "Move",
    }
}

pub fn target_label(
    chain: &[InstanceAlias],
    entity: &EntityAlias,
    component: &str,
    path: &ReflectedPath,
) -> String {
    let mut label = chain
        .iter()
        .map(InstanceAlias::as_str)
        .collect::<Vec<_>>()
        .join("/");
    if !label.is_empty() {
        label.push('/');
    }
    label.push_str(entity.as_str());
    label.push(':');
    label.push_str(component);
    label.push_str(&path.to_string());
    label
}
