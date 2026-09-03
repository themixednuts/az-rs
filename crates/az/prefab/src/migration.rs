//! Canonical Prefab source-tag resolution and monotonic sparse migrations.

use std::{any::TypeId, collections::BTreeMap};

use bevy_reflect::{TypeRegistration, TypeRegistry};
use thiserror::Error;

use crate::{
    document::{PrefabDocumentError, SparseValue},
    type_data::{ErasedPrefabValue, PrefabMigrationStep, PrefabTypeData},
};

const RESERVED_TAG_PREFIXES: &[&str] = &["__", "core.", "azoth.internal."];

#[derive(Debug, Clone, Copy)]
struct TagResolution {
    type_id: TypeId,
    alias_version: Option<u32>,
}

/// Validated view of Prefab-specific registrations in one Bevy registry.
pub struct PrefabRegistry<'a> {
    registry: &'a TypeRegistry,
    tags: BTreeMap<String, TagResolution>,
    type_paths: BTreeMap<String, TypeId>,
    source_types: BTreeMap<TypeId, TypeId>,
}

/// One resolved canonical Prefab registration.
#[derive(Clone, Copy)]
pub struct ResolvedPrefabType<'a> {
    /// Runtime component registration selected by the Prefab tag or type path.
    pub registration: &'a TypeRegistration,
    /// Reflected authoring registration decoded, migrated, and overridden before construction.
    ///
    /// This is the runtime registration itself for direct Prefab components and
    /// the explicitly declared template registration for template-backed ones.
    pub source_registration: &'a TypeRegistration,
    pub prefab: &'a PrefabTypeData,
    pub alias_version: Option<u32>,
}

impl<'a> PrefabRegistry<'a> {
    /// Builds the tag index and rejects malformed source identity or migration
    /// graphs once, before any document walk.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabMigrationError::InvalidTag`],
    /// [`PrefabMigrationError::InvalidAliasVersion`],
    /// [`PrefabMigrationError::CyclicMigration`],
    /// [`PrefabMigrationError::GappedMigration`],
    /// [`PrefabMigrationError::AmbiguousMigration`],
    /// [`PrefabMigrationError::MigrationVersionOutOfRange`], or
    /// [`PrefabMigrationError::MissingMigrationStep`] for a malformed
    /// registration, and [`PrefabMigrationError::TagCollision`] when two
    /// registrations claim the same canonical tag or alias.
    pub fn try_new(registry: &'a TypeRegistry) -> Result<Self, PrefabMigrationError> {
        let mut tags = BTreeMap::new();
        let mut type_paths = BTreeMap::new();
        let mut source_types = BTreeMap::new();

        for registration in registry.iter() {
            type_paths.insert(
                registration.type_info().type_path().to_owned(),
                registration.type_id(),
            );
            let Some(prefab) = registration.data::<PrefabTypeData>() else {
                continue;
            };
            validate_registration(registry, registration, prefab)?;
            let source_type_info = match prefab.construction {
                crate::PrefabConstruction::ReflectDefaultOrFromWorld => registration.type_info(),
                crate::PrefabConstruction::Template { template_type_info } => template_type_info(),
            };
            if let Some(existing) =
                source_types.insert(source_type_info.type_id(), registration.type_id())
                && existing != registration.type_id()
            {
                let existing = registry
                    .get(existing)
                    .map(TypeRegistration::type_info)
                    .map_or(
                        "an unregistered Prefab type",
                        bevy_reflect::TypeInfo::type_path,
                    );
                return Err(PrefabMigrationError::SourceTypeCollision {
                    source_type_path: source_type_info.type_path().to_owned(),
                    first: existing.to_owned(),
                    second: registration.type_info().type_path().to_owned(),
                });
            }
            insert_tag(
                &mut tags,
                prefab.tag,
                registration.type_id(),
                None,
                registration.type_info().type_path(),
            )?;
            for alias in prefab.aliases {
                insert_tag(
                    &mut tags,
                    alias.tag,
                    registration.type_id(),
                    Some(alias.source_version),
                    registration.type_info().type_path(),
                )?;
            }
        }

        Ok(Self {
            registry,
            tags,
            type_paths,
            source_types,
        })
    }

    #[must_use]
    pub const fn type_registry(&self) -> &'a TypeRegistry {
        self.registry
    }

    /// Resolves a canonical tag or alias to its registration and Prefab data.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabMigrationError::UnknownTag`] when `tag` is not indexed,
    /// [`PrefabMigrationError::UnregisteredTypeId`] when the indexed type id is
    /// no longer in the registry, and
    /// [`PrefabMigrationError::MissingPrefabTypeData`] when that registration
    /// has lost its [`PrefabTypeData`].
    pub fn resolve_tag(&self, tag: &str) -> Result<ResolvedPrefabType<'a>, PrefabMigrationError> {
        let resolution = self
            .tags
            .get(tag)
            .ok_or_else(|| PrefabMigrationError::UnknownTag(tag.to_owned()))?;
        let registration = self.registry.get(resolution.type_id).ok_or_else(|| {
            PrefabMigrationError::UnregisteredTypeId {
                tag: tag.to_owned(),
            }
        })?;
        let prefab = registration.data::<PrefabTypeData>().ok_or_else(|| {
            PrefabMigrationError::MissingPrefabTypeData {
                type_path: registration.type_info().type_path().to_owned(),
            }
        })?;
        self.resolve_registration(registration, prefab, resolution.alias_version)
    }

    /// Resolves a reflected `TypePath` to its Bevy registration.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabMigrationError::UnregisteredTypePath`] when `type_path`
    /// is absent from the index or its type id is no longer registered.
    pub fn resolve_type_path(
        &self,
        type_path: &str,
    ) -> Result<&'a TypeRegistration, PrefabMigrationError> {
        let type_id = self
            .type_paths
            .get(type_path)
            .ok_or_else(|| PrefabMigrationError::UnregisteredTypePath(type_path.to_owned()))?;
        self.registry
            .get(*type_id)
            .ok_or_else(|| PrefabMigrationError::UnregisteredTypePath(type_path.to_owned()))
    }

    /// Resolves a runtime component `TypePath` to its Prefab registration and
    /// its direct or template-backed authoring registration.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabMigrationError::UnregisteredTypePath`] when the runtime
    /// type path is absent, [`PrefabMigrationError::MissingPrefabTypeData`] when
    /// it is not a Prefab component, or
    /// [`PrefabMigrationError::UnregisteredTemplateType`] when a declared
    /// construction template is not registered.
    pub fn resolve_prefab_type_path(
        &self,
        type_path: &str,
    ) -> Result<ResolvedPrefabType<'a>, PrefabMigrationError> {
        let registration = self.resolve_type_path(type_path)?;
        let prefab = registration.data::<PrefabTypeData>().ok_or_else(|| {
            PrefabMigrationError::MissingPrefabTypeData {
                type_path: type_path.to_owned(),
            }
        })?;
        self.resolve_registration(registration, prefab, None)
    }

    /// Resolves an extracted direct component or authoring template `TypeId`
    /// to the one canonical runtime Prefab registration that owns it.
    ///
    /// This is the inverse of template-backed construction and lets offline
    /// importers retain the authoring value while still emitting the runtime
    /// component's canonical tag and source version.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabMigrationError::UnregisteredSourceType`] when no Prefab
    /// component declares the reflected source type.
    pub fn resolve_source_type_id(
        &self,
        source_type_id: TypeId,
    ) -> Result<ResolvedPrefabType<'a>, PrefabMigrationError> {
        let output_type_id = self
            .source_types
            .get(&source_type_id)
            .ok_or(PrefabMigrationError::UnregisteredSourceType { source_type_id })?;
        let registration = self.registry.get(*output_type_id).ok_or_else(|| {
            PrefabMigrationError::UnregisteredTypeId {
                tag: "source-type lookup".to_owned(),
            }
        })?;
        let prefab = registration.data::<PrefabTypeData>().ok_or_else(|| {
            PrefabMigrationError::MissingPrefabTypeData {
                type_path: registration.type_info().type_path().to_owned(),
            }
        })?;
        self.resolve_registration(registration, prefab, None)
    }

    fn resolve_registration(
        &self,
        registration: &'a TypeRegistration,
        prefab: &'a PrefabTypeData,
        alias_version: Option<u32>,
    ) -> Result<ResolvedPrefabType<'a>, PrefabMigrationError> {
        let source_type_info = match prefab.construction {
            crate::PrefabConstruction::ReflectDefaultOrFromWorld => registration.type_info(),
            crate::PrefabConstruction::Template { template_type_info } => template_type_info(),
        };
        let source_registration =
            self.registry
                .get(source_type_info.type_id())
                .ok_or_else(|| PrefabMigrationError::UnregisteredTemplateType {
                    type_path: registration.type_info().type_path().to_owned(),
                    template_type_path: source_type_info.type_path().to_owned(),
                })?;
        Ok(ResolvedPrefabType {
            registration,
            source_registration,
            prefab,
            alias_version,
        })
    }

    /// Resolves an old tag/version once, runs its complete chain, and returns
    /// only the canonical current identity and sparse value.
    ///
    /// # Errors
    ///
    /// Forwards every error of [`Self::resolve_tag`], then returns
    /// [`PrefabMigrationError::AliasVersionMismatch`] when an alias tag is
    /// paired with the wrong version,
    /// [`PrefabMigrationError::UnsupportedSourceVersion`] for a future or
    /// unversioned document, [`PrefabMigrationError::ValueTypeMismatch`] when
    /// `value` does not represent the resolved type before or after the chain,
    /// [`PrefabMigrationError::MissingMigrationStep`] when the chain has no
    /// edge out of the current version, and
    /// [`PrefabMigrationError::MigrationFailed`] wrapping any error a
    /// migration step itself returns. [`PrefabMigrationError::Document`] wraps
    /// the final `SparseValue::for_type` re-check of the migrated value.
    pub fn migrate(
        &self,
        encoded_tag: &str,
        encoded_version: u32,
        value: SparseValue,
    ) -> Result<(String, u32, SparseValue), PrefabMigrationError> {
        let resolved = self.resolve_tag(encoded_tag)?;
        if let Some(alias_version) = resolved.alias_version
            && alias_version != encoded_version
        {
            return Err(PrefabMigrationError::AliasVersionMismatch {
                tag: encoded_tag.to_owned(),
                expected: alias_version,
                actual: encoded_version,
            });
        }
        if encoded_version > resolved.prefab.source_version
            || (encoded_version == 0
                && resolved.prefab.source_version != 0
                && resolved.alias_version.is_none())
        {
            return Err(PrefabMigrationError::UnsupportedSourceVersion {
                tag: encoded_tag.to_owned(),
                version: encoded_version,
                current: resolved.prefab.source_version,
            });
        }
        if value.type_info().type_id() != resolved.source_registration.type_id() {
            return Err(PrefabMigrationError::ValueTypeMismatch {
                tag: encoded_tag.to_owned(),
                expected: resolved
                    .source_registration
                    .type_info()
                    .type_path()
                    .to_owned(),
                actual: value.type_path().to_owned(),
            });
        }

        let mut cursor = encoded_version;
        let mut value = ErasedPrefabValue {
            type_info: value.type_info(),
            value: value.into_value(),
        };
        while cursor < resolved.prefab.source_version {
            let step = migration_from(resolved.prefab.migrations, cursor).ok_or_else(|| {
                PrefabMigrationError::MissingMigrationStep {
                    type_path: resolved.registration.type_info().type_path().to_owned(),
                    from_version: cursor,
                    current_version: resolved.prefab.source_version,
                }
            })?;
            value =
                (step.migrate)(value).map_err(|error| PrefabMigrationError::MigrationFailed {
                    type_path: resolved.registration.type_info().type_path().to_owned(),
                    from_version: step.from_version,
                    to_version: step.to_version,
                    message: error.to_string(),
                })?;
            cursor = step.to_version;
        }

        if value.type_info.type_id() != resolved.source_registration.type_id() {
            return Err(PrefabMigrationError::ValueTypeMismatch {
                tag: encoded_tag.to_owned(),
                expected: resolved
                    .source_registration
                    .type_info()
                    .type_path()
                    .to_owned(),
                actual: value.type_info.type_path().to_owned(),
            });
        }

        let value = SparseValue::for_type(value.value, resolved.source_registration.type_info())?;
        Ok((
            resolved.prefab.tag.to_owned(),
            resolved.prefab.source_version,
            value,
        ))
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PrefabMigrationError {
    #[error("unknown Prefab source tag `{0}`")]
    UnknownTag(String),
    #[error("Prefab tag `{tag}` resolves to an unregistered TypeId")]
    UnregisteredTypeId { tag: String },
    #[error("unregistered reflected TypePath `{0}`")]
    UnregisteredTypePath(String),
    #[error("type `{type_path}` has no PrefabTypeData")]
    MissingPrefabTypeData { type_path: String },
    #[error(
        "Prefab type `{type_path}` declares unregistered construction template `{template_type_path}`"
    )]
    UnregisteredTemplateType {
        type_path: String,
        template_type_path: String,
    },
    #[error(
        "Prefab authoring type `{source_type_path}` is claimed by both `{first}` and `{second}`"
    )]
    SourceTypeCollision {
        source_type_path: String,
        first: String,
        second: String,
    },
    #[error("reflected source TypeId {source_type_id:?} is not owned by any Prefab component")]
    UnregisteredSourceType { source_type_id: TypeId },
    #[error("invalid Prefab {kind} tag `{tag}` on `{type_path}`")]
    InvalidTag {
        kind: &'static str,
        tag: String,
        type_path: String,
    },
    #[error("Prefab tag or alias `{tag}` is owned by both `{first}` and `{second}`")]
    TagCollision {
        tag: String,
        first: String,
        second: String,
    },
    #[error("Prefab alias `{tag}` on `{type_path}` has invalid source version {version}")]
    InvalidAliasVersion {
        type_path: String,
        tag: String,
        version: u32,
    },
    #[error(
        "Prefab migration on `{type_path}` is cyclic/non-monotonic: {from_version}->{to_version}"
    )]
    CyclicMigration {
        type_path: String,
        from_version: u32,
        to_version: u32,
    },
    #[error("Prefab migration on `{type_path}` has a gap: {from_version}->{to_version}")]
    GappedMigration {
        type_path: String,
        from_version: u32,
        to_version: u32,
    },
    #[error("Prefab migration on `{type_path}` has multiple edges from {from_version}")]
    AmbiguousMigration {
        type_path: String,
        from_version: u32,
    },
    #[error(
        "Prefab migration on `{type_path}` uses version {version} outside 0..={current_version}"
    )]
    MigrationVersionOutOfRange {
        type_path: String,
        version: u32,
        current_version: u32,
    },
    #[error(
        "Prefab migration chain on `{type_path}` from {from_version} does not reach current version {current_version}"
    )]
    MissingMigrationStep {
        type_path: String,
        from_version: u32,
        current_version: u32,
    },
    #[error("Prefab alias `{tag}` requires version {expected}, but the document declares {actual}")]
    AliasVersionMismatch {
        tag: String,
        expected: u32,
        actual: u32,
    },
    #[error("Prefab tag `{tag}` declares unsupported version {version}; current is {current}")]
    UnsupportedSourceVersion {
        tag: String,
        version: u32,
        current: u32,
    },
    #[error("Prefab value for `{tag}` represents `{actual}`, expected `{expected}`")]
    ValueTypeMismatch {
        tag: String,
        expected: String,
        actual: String,
    },
    #[error("Prefab migration `{type_path}` {from_version}->{to_version} failed: {message}")]
    MigrationFailed {
        type_path: String,
        from_version: u32,
        to_version: u32,
        message: String,
    },
    #[error(transparent)]
    Document(#[from] PrefabDocumentError),
}

fn validate_registration(
    registry: &TypeRegistry,
    registration: &TypeRegistration,
    prefab: &PrefabTypeData,
) -> Result<(), PrefabMigrationError> {
    let type_path = registration.type_info().type_path();
    if let crate::PrefabConstruction::Template { template_type_info } = prefab.construction {
        let template_type_info = template_type_info();
        if registry.get(template_type_info.type_id()).is_none() {
            return Err(PrefabMigrationError::UnregisteredTemplateType {
                type_path: type_path.to_owned(),
                template_type_path: template_type_info.type_path().to_owned(),
            });
        }
    }
    validate_tag(prefab.tag, "canonical", type_path)?;
    for alias in prefab.aliases {
        validate_tag(alias.tag, "alias", type_path)?;
        if alias.source_version > prefab.source_version {
            return Err(PrefabMigrationError::InvalidAliasVersion {
                type_path: type_path.to_owned(),
                tag: alias.tag.to_owned(),
                version: alias.source_version,
            });
        }
    }

    let mut edges = BTreeMap::new();
    for step in prefab.migrations {
        if step.to_version > prefab.source_version {
            return Err(PrefabMigrationError::MigrationVersionOutOfRange {
                type_path: type_path.to_owned(),
                version: step.to_version,
                current_version: prefab.source_version,
            });
        }
        if step.to_version <= step.from_version {
            return Err(PrefabMigrationError::CyclicMigration {
                type_path: type_path.to_owned(),
                from_version: step.from_version,
                to_version: step.to_version,
            });
        }
        if step.to_version != step.from_version + 1 {
            return Err(PrefabMigrationError::GappedMigration {
                type_path: type_path.to_owned(),
                from_version: step.from_version,
                to_version: step.to_version,
            });
        }
        if edges.insert(step.from_version, step.to_version).is_some() {
            return Err(PrefabMigrationError::AmbiguousMigration {
                type_path: type_path.to_owned(),
                from_version: step.from_version,
            });
        }
    }

    for start in std::iter::once(1)
        .chain(prefab.aliases.iter().map(|alias| alias.source_version))
        .chain(edges.keys().copied())
        .filter(|version| *version < prefab.source_version)
    {
        let mut cursor = start;
        while cursor < prefab.source_version {
            cursor =
                *edges
                    .get(&cursor)
                    .ok_or_else(|| PrefabMigrationError::MissingMigrationStep {
                        type_path: type_path.to_owned(),
                        from_version: cursor,
                        current_version: prefab.source_version,
                    })?;
        }
    }
    Ok(())
}

fn insert_tag(
    tags: &mut BTreeMap<String, TagResolution>,
    tag: &str,
    type_id: TypeId,
    alias_version: Option<u32>,
    type_path: &str,
) -> Result<(), PrefabMigrationError> {
    if let Some(existing) = tags.insert(
        tag.to_owned(),
        TagResolution {
            type_id,
            alias_version,
        },
    ) {
        let first = if existing.type_id.eq(&type_id) {
            type_path
        } else {
            "another registered type"
        };
        return Err(PrefabMigrationError::TagCollision {
            tag: tag.to_owned(),
            first: first.to_owned(),
            second: type_path.to_owned(),
        });
    }
    Ok(())
}

fn validate_tag(
    tag: &str,
    kind: &'static str,
    type_path: &str,
) -> Result<(), PrefabMigrationError> {
    if tag.trim().is_empty()
        || tag != tag.trim()
        || RESERVED_TAG_PREFIXES
            .iter()
            .any(|prefix| tag.starts_with(prefix))
    {
        return Err(PrefabMigrationError::InvalidTag {
            kind,
            tag: tag.to_owned(),
            type_path: type_path.to_owned(),
        });
    }
    Ok(())
}

fn migration_from(
    migrations: &[PrefabMigrationStep],
    from_version: u32,
) -> Option<&PrefabMigrationStep> {
    migrations
        .iter()
        .find(|migration| migration.from_version == from_version)
}
