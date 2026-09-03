//! Composition, validation, and unrouted ADR 0022 registry projection.

use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
};

use az_core::{
    ApplicabilityTypeData, EditorFieldAttributes, EditorPolicyTypeData, EditorTypeAttributes,
    ValidationTypeData, register_editor_builtins,
};
use az_gem_contract::Registry;
use az_prefab::{PrefabCodec, PrefabConstruction, PrefabType, PrefabTypeData, SparseValue};
use az_proto_project::vnext::{
    ApplicabilityDescriptor, EditorAttributes, FieldConstraints, NumericRange,
    ReflectedFieldDescriptor, ReflectedTypeDescriptor, ReflectedTypeKind, ReflectedValueEncoding,
    ReflectedValueEnvelope, ReflectedVariantDescriptor, TypeRegistrySnapshot,
};
use bevy_ecs::{
    reflect::{AppTypeRegistry, ReflectComponent, ReflectFromWorld},
    world::World,
};
use bevy_reflect::{
    NamedField, TypeInfo, TypeRegistration, TypeRegistry, UnnamedField, enums::VariantInfo,
    std_traits::ReflectDefault,
};
use thiserror::Error;
use tracing::{info, instrument};

const RESERVED_TAG_PREFIXES: &[&str] = &["__", "core.", "azoth.internal."];

/// A composed Bevy registry plus its validated source-tag resolution map.
#[derive(Clone)]
pub struct ComposedTypeRegistry {
    pub app_registry: AppTypeRegistry,
    /// Canonical tags and aliases both resolve to one canonical Rust `TypePath`.
    pub tag_to_type_path: BTreeMap<String, String>,
}

/// Process roles which must observe the same serialized prefab registrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryConsumer {
    Client,
    Server,
    Builder,
    ProjectHost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefabManifestEntry {
    pub type_path: String,
    pub tag: String,
    pub source_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryManifest {
    pub prefabs: Vec<PrefabManifestEntry>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RegistryValidationError {
    #[error("Prefab type `{type_path}` has invalid canonical tag `{tag}`")]
    InvalidCanonicalTag { type_path: String, tag: String },
    #[error("Prefab type `{type_path}` has invalid alias tag `{tag}`")]
    InvalidAliasTag { type_path: String, tag: String },
    #[error("canonical Prefab tag `{tag}` is registered by both `{first}` and `{second}`")]
    DuplicateCanonicalTag {
        tag: String,
        first: String,
        second: String,
    },
    #[error("Prefab alias `{tag}` for `{type_path}` collides with `{existing_type_path}`")]
    AliasCollision {
        tag: String,
        type_path: String,
        existing_type_path: String,
    },
    #[error("Prefab type `{type_path}` alias `{tag}` has invalid source version {version}")]
    InvalidAliasVersion {
        type_path: String,
        tag: String,
        version: u32,
    },
    #[error("Prefab type `{type_path}` is missing ReflectComponent")]
    MissingReflectComponent { type_path: String },
    #[error("Prefab type `{type_path}` is missing required TypeData `{type_data}`")]
    MissingRequiredTypeData {
        type_path: String,
        type_data: &'static str,
    },
    #[error(
        "Prefab type `{type_path}` declares unregistered construction template `{template_type_path}`"
    )]
    UnregisteredTemplateType {
        type_path: String,
        template_type_path: String,
    },
    #[error("Prefab type `{type_path}` has ambiguous migrations from version {from_version}")]
    AmbiguousMigrationEdge {
        type_path: String,
        from_version: u32,
    },
    #[error(
        "Prefab type `{type_path}` has a cyclic/non-monotonic migration {from_version}->{to_version}"
    )]
    CyclicMigration {
        type_path: String,
        from_version: u32,
        to_version: u32,
    },
    #[error("Prefab type `{type_path}` has a gapped migration {from_version}->{to_version}")]
    GappedMigration {
        type_path: String,
        from_version: u32,
        to_version: u32,
    },
    #[error(
        "Prefab type `{type_path}` migration chain from {start_version} ends at {last_version}, not current version {current_version}"
    )]
    MigrationCurrentVersionMismatch {
        type_path: String,
        start_version: u32,
        last_version: u32,
        current_version: u32,
    },
    #[error("array capacity {capacity} for `{type_path}` exceeds the vNext u32 contract")]
    ArrayCapacityOverflow { type_path: String, capacity: usize },
    #[error("failed to project reflected default for `{type_path}`: {message}")]
    ReflectedDefaultProjection { type_path: String, message: String },
}

/// Composes the engine's reflected types with one host's composed Prefab types
/// into the single Bevy registry authority, then validates every Prefab
/// extension at startup.
///
/// Engine types are applied directly — they compile into this process and are
/// not contributions. `prefab_types` is the host's composed
/// `Registry<PrefabType>`; `None` is the uncomposed host, which reflects the
/// engine types alone until the prebuilt host composes gem types (Push 2).
///
/// # Errors
///
/// Returns any [`RegistryValidationError`] [`validate_type_registry`] raises
/// over the composed registry — a duplicate or reserved Prefab tag, a malformed
/// alias, or a migration chain that does not reach the current version.
#[instrument(skip_all, fields(prefab_types = prefab_types.map_or(0, Registry::len)))]
pub fn compose_type_registry(
    prefab_types: Option<&Registry<PrefabType>>,
) -> Result<ComposedTypeRegistry, RegistryValidationError> {
    let app_registry = AppTypeRegistry::default();
    {
        let mut registry = app_registry.write();
        register_editor_builtins(&mut registry);
        az_transform::register_transform_prefab_types(&mut registry);
        az_render::register_render_prefab_types(&mut registry);
        az_framework::register_framework_prefab_types(&mut registry);
        if let Some(prefab_types) = prefab_types {
            for prefab_type in prefab_types.entries() {
                prefab_type.apply(&mut registry);
            }
        }
    }
    let tag_to_type_path = {
        let registry = app_registry.read();
        validate_type_registry(&registry)?
    };
    info!(
        reflected_type_count = app_registry.read().iter().count(),
        prefab_tag_count = tag_to_type_path.len(),
        "composed and validated Bevy type registry"
    );
    Ok(ComposedTypeRegistry {
        app_registry,
        tag_to_type_path,
    })
}

/// Internal harness used by all process-role manifest checks. The role does not
/// alter serialized data registrations.
///
/// # Errors
///
/// Returns any error [`compose_type_registry`] returns; building the manifest
/// itself cannot fail.
pub fn registry_manifest_for_consumer(
    _consumer: RegistryConsumer,
    prefab_types: Option<&Registry<PrefabType>>,
) -> Result<RegistryManifest, RegistryValidationError> {
    let composed = compose_type_registry(prefab_types)?;
    Ok(registry_manifest(&composed.app_registry.read()))
}

#[must_use]
pub fn registry_manifest(registry: &TypeRegistry) -> RegistryManifest {
    let mut prefabs = registry
        .iter()
        .filter_map(|registration| {
            registration
                .data::<PrefabTypeData>()
                .map(|prefab| PrefabManifestEntry {
                    type_path: registration.type_info().type_path().to_owned(),
                    tag: prefab.tag.to_owned(),
                    source_version: prefab.source_version,
                })
        })
        .collect::<Vec<_>>();
    prefabs.sort_by(|left, right| left.type_path.cmp(&right.type_path));
    RegistryManifest { prefabs }
}

/// Validates Prefab source identity and materialization data once at bootstrap.
///
/// # Errors
///
/// Returns [`RegistryValidationError::InvalidCanonicalTag`] or
/// [`RegistryValidationError::DuplicateCanonicalTag`] when the canonical tags
/// do not form a single valid owner set;
/// [`RegistryValidationError::InvalidAliasTag`],
/// [`RegistryValidationError::InvalidAliasVersion`], or
/// [`RegistryValidationError::AliasCollision`] for a malformed or colliding
/// alias; [`RegistryValidationError::MissingReflectComponent`],
/// [`RegistryValidationError::MissingRequiredTypeData`], or
/// [`RegistryValidationError::UnregisteredTemplateType`] when a Prefab
/// registration cannot be materialized; and
/// [`RegistryValidationError::AmbiguousMigrationEdge`],
/// [`RegistryValidationError::CyclicMigration`],
/// [`RegistryValidationError::GappedMigration`], or
/// [`RegistryValidationError::MigrationCurrentVersionMismatch`] when a type's
/// migration chain is not a contiguous run ending at its current source
/// version.
///
/// # Panics
///
/// Panics if a registration that was just filtered on having
/// `PrefabTypeData` no longer reports it — the registry is read under one
/// borrow, so that cannot happen.
pub fn validate_type_registry(
    registry: &TypeRegistry,
) -> Result<BTreeMap<String, String>, RegistryValidationError> {
    let prefab_registrations = registry
        .iter()
        .filter(|registration| registration.data::<PrefabTypeData>().is_some())
        .collect::<Vec<_>>();
    let canonical_owners = validate_canonical_tags(&prefab_registrations)?;
    let mut resolved = canonical_owners;

    for registration in prefab_registrations {
        let type_path = registration.type_info().type_path();
        let prefab = registration
            .data::<PrefabTypeData>()
            .expect("filtered Prefab registration");
        validate_prefab_registration(registry, registration, prefab)?;
        for alias in prefab.aliases {
            if !valid_tag(alias.tag) {
                return Err(RegistryValidationError::InvalidAliasTag {
                    type_path: type_path.to_owned(),
                    tag: alias.tag.to_owned(),
                });
            }
            if alias.source_version > prefab.source_version {
                return Err(RegistryValidationError::InvalidAliasVersion {
                    type_path: type_path.to_owned(),
                    tag: alias.tag.to_owned(),
                    version: alias.source_version,
                });
            }
            if let Some(existing) = resolved.insert(alias.tag.to_owned(), type_path.to_owned()) {
                return Err(RegistryValidationError::AliasCollision {
                    tag: alias.tag.to_owned(),
                    type_path: type_path.to_owned(),
                    existing_type_path: existing,
                });
            }
        }
        validate_migrations(type_path, prefab)?;
    }
    Ok(resolved)
}

fn validate_canonical_tags(
    registrations: &[&TypeRegistration],
) -> Result<BTreeMap<String, String>, RegistryValidationError> {
    let mut owners = BTreeMap::new();
    for registration in registrations {
        let type_path = registration.type_info().type_path();
        let prefab = registration
            .data::<PrefabTypeData>()
            .expect("filtered Prefab registration");
        if !valid_tag(prefab.tag) {
            return Err(RegistryValidationError::InvalidCanonicalTag {
                type_path: type_path.to_owned(),
                tag: prefab.tag.to_owned(),
            });
        }
        if let Some(first) = owners.insert(prefab.tag.to_owned(), type_path.to_owned()) {
            return Err(RegistryValidationError::DuplicateCanonicalTag {
                tag: prefab.tag.to_owned(),
                first,
                second: type_path.to_owned(),
            });
        }
    }
    Ok(owners)
}

fn validate_prefab_registration(
    registry: &TypeRegistry,
    registration: &TypeRegistration,
    prefab: &PrefabTypeData,
) -> Result<(), RegistryValidationError> {
    let type_path = registration.type_info().type_path();
    if !registration.contains::<ReflectComponent>() {
        return Err(RegistryValidationError::MissingReflectComponent {
            type_path: type_path.to_owned(),
        });
    }
    match prefab.construction {
        PrefabConstruction::ReflectDefaultOrFromWorld => {
            if !registration.contains::<ReflectDefault>()
                && !registration.contains::<ReflectFromWorld>()
            {
                return Err(RegistryValidationError::MissingRequiredTypeData {
                    type_path: type_path.to_owned(),
                    type_data: "ReflectDefault or ReflectFromWorld",
                });
            }
        }
        PrefabConstruction::Template { template_type_info } => {
            let template_type_info = template_type_info();
            if registry.get(template_type_info.type_id()).is_none() {
                return Err(RegistryValidationError::UnregisteredTemplateType {
                    type_path: type_path.to_owned(),
                    template_type_path: template_type_info.type_path().to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn validate_migrations(
    type_path: &str,
    prefab: &PrefabTypeData,
) -> Result<(), RegistryValidationError> {
    let mut edges = BTreeMap::new();
    for migration in prefab.migrations {
        if migration.to_version <= migration.from_version {
            return Err(RegistryValidationError::CyclicMigration {
                type_path: type_path.to_owned(),
                from_version: migration.from_version,
                to_version: migration.to_version,
            });
        }
        if migration.to_version != migration.from_version + 1 {
            return Err(RegistryValidationError::GappedMigration {
                type_path: type_path.to_owned(),
                from_version: migration.from_version,
                to_version: migration.to_version,
            });
        }
        if edges
            .insert(migration.from_version, migration.to_version)
            .is_some()
        {
            return Err(RegistryValidationError::AmbiguousMigrationEdge {
                type_path: type_path.to_owned(),
                from_version: migration.from_version,
            });
        }
    }

    let mut starts = prefab
        .aliases
        .iter()
        .map(|alias| alias.source_version)
        .chain(edges.keys().copied())
        .filter(|version| *version < prefab.source_version)
        .collect::<BTreeSet<_>>();
    if let Some(first) = edges.keys().next() {
        starts.insert(*first);
    }
    for start in starts {
        let mut cursor = start;
        let mut visited = BTreeSet::new();
        while cursor < prefab.source_version {
            if !visited.insert(cursor) {
                return Err(RegistryValidationError::CyclicMigration {
                    type_path: type_path.to_owned(),
                    from_version: cursor,
                    to_version: cursor,
                });
            }
            let Some(next) = edges.get(&cursor) else {
                return Err(RegistryValidationError::MigrationCurrentVersionMismatch {
                    type_path: type_path.to_owned(),
                    start_version: start,
                    last_version: cursor,
                    current_version: prefab.source_version,
                });
            };
            cursor = *next;
        }
        if cursor != prefab.source_version {
            return Err(RegistryValidationError::MigrationCurrentVersionMismatch {
                type_path: type_path.to_owned(),
                start_version: start,
                last_version: cursor,
                current_version: prefab.source_version,
            });
        }
    }
    Ok(())
}

fn valid_tag(tag: &str) -> bool {
    !tag.trim().is_empty()
        && tag == tag.trim()
        && !RESERVED_TAG_PREFIXES
            .iter()
            .any(|prefix| tag.starts_with(prefix))
}

/// Projects Bevy structural metadata and Azoth's typed editor extensions into
/// the Phase 0 vNext DTOs. This function has no RPC routing side effect.
///
/// # Errors
///
/// Returns [`RegistryValidationError::ReflectedDefaultProjection`] when the
/// Prefab codec cannot be built over `registry` or a type's reflected default
/// cannot be projected, and [`RegistryValidationError::ArrayCapacityOverflow`]
/// when an array type's capacity does not fit the vNext `u32` contract.
pub fn project_type_registry(
    registry: &TypeRegistry,
) -> Result<TypeRegistrySnapshot, RegistryValidationError> {
    let codec = PrefabCodec::new(registry).map_err(|error| {
        RegistryValidationError::ReflectedDefaultProjection {
            type_path: "<registry>".to_owned(),
            message: error.to_string(),
        }
    })?;
    let required_capabilities = required_component_capabilities(registry);
    let mut types = registry
        .iter()
        .map(|registration| {
            project_registration(
                registration,
                &codec,
                required_capabilities
                    .get(&registration.type_info().type_id())
                    .map_or(&[][..], Vec::as_slice),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    types.sort_by(|left, right| left.type_path.cmp(&right.type_path));
    let mut hasher = blake3::Hasher::new();
    for descriptor in &types {
        hasher.update(descriptor.type_path.as_bytes());
        hasher.update(b"\n");
    }
    Ok(TypeRegistrySnapshot {
        schema_catalog_hash: hasher.finalize().as_bytes().to_vec(),
        types,
    })
}

fn project_registration(
    registration: &TypeRegistration,
    codec: &PrefabCodec<'_>,
    required_capabilities: &[String],
) -> Result<ReflectedTypeDescriptor, RegistryValidationError> {
    let type_info = registration.type_info();
    let (kind, fields, variants) = project_type_info(type_info)?;
    let editor_attributes = type_editor_attributes(type_info)
        .or_else(|| registration.data::<EditorTypeAttributes>())
        .map_or_else(EditorAttributes::default, project_type_attributes);
    let mut type_data_flags = Vec::new();
    push_flag::<PrefabTypeData>(registration, &mut type_data_flags, "Prefab");
    push_flag::<ReflectComponent>(registration, &mut type_data_flags, "ReflectComponent");
    push_flag::<ReflectDefault>(registration, &mut type_data_flags, "ReflectDefault");
    push_flag::<ReflectFromWorld>(registration, &mut type_data_flags, "ReflectFromWorld");
    push_flag::<EditorTypeAttributes>(registration, &mut type_data_flags, "EditorAttributes");
    push_flag::<ValidationTypeData>(registration, &mut type_data_flags, "Validation");
    push_flag::<EditorPolicyTypeData>(registration, &mut type_data_flags, "EditorPolicy");
    push_flag::<ApplicabilityTypeData>(registration, &mut type_data_flags, "Applicability");

    let reflected_default = project_reflected_default(registration, codec)?;
    let applicability = project_applicability(
        registration,
        required_capabilities,
        reflected_default.is_some(),
    );

    Ok(ReflectedTypeDescriptor {
        type_path: type_info.type_path().to_owned(),
        short_path: type_info.type_path_table().short_path().to_owned(),
        kind,
        fields,
        variants,
        editor_attributes,
        type_data_flags,
        applicability,
        reflected_default,
    })
}

fn project_reflected_default(
    registration: &TypeRegistration,
    codec: &PrefabCodec<'_>,
) -> Result<Option<ReflectedValueEnvelope>, RegistryValidationError> {
    let Some(default) = registration.data::<ReflectDefault>() else {
        return Ok(None);
    };
    let type_path = registration.type_info().type_path();
    let reflected = default.default();
    let primitive_payload = reflected
        .downcast_ref::<i128>()
        .map(ToString::to_string)
        .or_else(|| reflected.downcast_ref::<u128>().map(ToString::to_string));
    if let Some(payload) = primitive_payload {
        return Ok(Some(ReflectedValueEnvelope {
            type_path: type_path.to_owned(),
            encoding: ReflectedValueEncoding::TypedRon,
            payload: payload.into_bytes(),
        }));
    }
    let value = SparseValue::for_type(reflected.into_partial_reflect(), registration.type_info())
        .map_err(
        |error| RegistryValidationError::ReflectedDefaultProjection {
            type_path: type_path.to_owned(),
            message: error.to_string(),
        },
    )?;
    let payload = codec.encode_sparse_value(&value).map_err(|error| {
        RegistryValidationError::ReflectedDefaultProjection {
            type_path: type_path.to_owned(),
            message: error.to_string(),
        }
    })?;
    Ok(Some(ReflectedValueEnvelope {
        type_path: type_path.to_owned(),
        encoding: ReflectedValueEncoding::TypedRon,
        payload,
    }))
}

fn project_applicability(
    registration: &TypeRegistration,
    required_components: &[String],
    default_available: bool,
) -> ApplicabilityDescriptor {
    let mut provides = BTreeSet::new();
    let mut requires = required_components.iter().cloned().collect::<BTreeSet<_>>();
    let mut incompatible = BTreeSet::new();
    if let Some(applicability) = registration.data::<ApplicabilityTypeData>() {
        provides.extend(
            applicability
                .provides
                .iter()
                .map(|value| (*value).to_owned()),
        );
        requires.extend(
            applicability
                .requires
                .iter()
                .map(|value| (*value).to_owned()),
        );
        incompatible.extend(
            applicability
                .incompatible
                .iter()
                .map(|value| (*value).to_owned()),
        );
    }
    ApplicabilityDescriptor {
        provides: provides.into_iter().collect(),
        requires: requires.into_iter().collect(),
        incompatible: incompatible.into_iter().collect(),
        default_available,
    }
}

fn required_component_capabilities(registry: &TypeRegistry) -> BTreeMap<TypeId, Vec<String>> {
    let mut world = World::new();
    for registration in registry.iter() {
        if let Some(component) = registration.data::<ReflectComponent>() {
            component.register_component(&mut world);
        }
    }

    let components = world.components();
    registry
        .iter()
        .filter_map(|registration| {
            let type_id = registration.type_info().type_id();
            let component_id = components.get_id(type_id)?;
            let info = components.get_info(component_id)?;
            let required = info
                .required_components()
                .iter_ids()
                .flat_map(|required_id| {
                    let Some(required_info) = components.get_info(required_id) else {
                        return Vec::new();
                    };
                    let Some(required_registration) = required_info
                        .type_id()
                        .and_then(|required_type_id| registry.get(required_type_id))
                    else {
                        return vec![required_info.name().to_string()];
                    };
                    required_registration
                        .data::<ApplicabilityTypeData>()
                        .filter(|applicability| !applicability.provides.is_empty())
                        .map_or_else(
                            || vec![required_registration.type_info().type_path().to_owned()],
                            |applicability| {
                                applicability
                                    .provides
                                    .iter()
                                    .map(|value| (*value).to_owned())
                                    .collect()
                            },
                        )
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            Some((type_id, required))
        })
        .collect()
}

fn push_flag<T: bevy_reflect::TypeData>(
    registration: &TypeRegistration,
    flags: &mut Vec<String>,
    name: &str,
) {
    if registration.contains::<T>() {
        flags.push(name.to_owned());
    }
}

fn project_type_info(
    type_info: &'static TypeInfo,
) -> Result<
    (
        ReflectedTypeKind,
        Vec<ReflectedFieldDescriptor>,
        Vec<ReflectedVariantDescriptor>,
    ),
    RegistryValidationError,
> {
    match type_info {
        TypeInfo::Struct(info) => Ok((
            ReflectedTypeKind::Struct,
            info.iter().map(project_named_field).collect(),
            Vec::new(),
        )),
        TypeInfo::TupleStruct(info) => Ok((
            ReflectedTypeKind::TupleStruct,
            info.iter().map(project_unnamed_field).collect(),
            Vec::new(),
        )),
        TypeInfo::Tuple(info) => Ok((
            ReflectedTypeKind::Tuple,
            info.iter().map(project_unnamed_field).collect(),
            Vec::new(),
        )),
        TypeInfo::List(_) => Ok((ReflectedTypeKind::List, Vec::new(), Vec::new())),
        TypeInfo::Array(info) => Ok((
            ReflectedTypeKind::Array {
                capacity: u32::try_from(info.capacity()).map_err(|_| {
                    RegistryValidationError::ArrayCapacityOverflow {
                        type_path: type_info.type_path().to_owned(),
                        capacity: info.capacity(),
                    }
                })?,
            },
            Vec::new(),
            Vec::new(),
        )),
        TypeInfo::Map(_) => Ok((ReflectedTypeKind::Map, Vec::new(), Vec::new())),
        TypeInfo::Set(_) => Ok((ReflectedTypeKind::Set, Vec::new(), Vec::new())),
        TypeInfo::Enum(_info) if type_info.type_path().starts_with("core::option::Option<") => {
            Ok((ReflectedTypeKind::Optional, Vec::new(), Vec::new()))
        }
        TypeInfo::Enum(info) => Ok((
            ReflectedTypeKind::Enum,
            Vec::new(),
            info.iter().map(project_variant).collect(),
        )),
        TypeInfo::Opaque(_) => Ok((primitive_kind(type_info), Vec::new(), Vec::new())),
    }
}

fn primitive_kind(type_info: &TypeInfo) -> ReflectedTypeKind {
    let type_id = type_info.type_id();
    if type_id == TypeId::of::<bool>() {
        ReflectedTypeKind::Bool
    } else if [
        TypeId::of::<i8>(),
        TypeId::of::<i16>(),
        TypeId::of::<i32>(),
        TypeId::of::<i64>(),
        TypeId::of::<i128>(),
        TypeId::of::<isize>(),
    ]
    .contains(&type_id)
    {
        ReflectedTypeKind::SignedInteger {
            bits: integer_bits(type_id),
        }
    } else if [
        TypeId::of::<u8>(),
        TypeId::of::<u16>(),
        TypeId::of::<u32>(),
        TypeId::of::<u64>(),
        TypeId::of::<u128>(),
        TypeId::of::<usize>(),
    ]
    .contains(&type_id)
    {
        ReflectedTypeKind::UnsignedInteger {
            bits: integer_bits(type_id),
        }
    } else if type_id == TypeId::of::<f32>() {
        ReflectedTypeKind::Float { bits: 32 }
    } else if type_id == TypeId::of::<f64>() {
        ReflectedTypeKind::Float { bits: 64 }
    } else if type_id == TypeId::of::<String>() || type_id == TypeId::of::<&'static str>() {
        ReflectedTypeKind::String
    } else {
        ReflectedTypeKind::Opaque
    }
}

fn integer_bits(type_id: TypeId) -> u8 {
    if type_id == TypeId::of::<i8>() || type_id == TypeId::of::<u8>() {
        8
    } else if type_id == TypeId::of::<i16>() || type_id == TypeId::of::<u16>() {
        16
    } else if type_id == TypeId::of::<i32>() || type_id == TypeId::of::<u32>() {
        32
    } else if type_id == TypeId::of::<i64>() || type_id == TypeId::of::<u64>() {
        64
    } else if type_id == TypeId::of::<i128>() || type_id == TypeId::of::<u128>() {
        128
    } else {
        // The remaining callers are `isize`/`usize`, whose width is the target
        // pointer width: 16, 32, or 64 on every supported target.
        u8::try_from(usize::BITS).unwrap_or(u8::MAX)
    }
}

fn project_named_field(field: &NamedField) -> ReflectedFieldDescriptor {
    ReflectedFieldDescriptor {
        name: field.name().to_owned(),
        type_path: field.type_path().to_owned(),
        editor_attributes: field
            .get_attribute::<EditorFieldAttributes>()
            .map_or_else(EditorAttributes::default, project_field_attributes),
    }
}

fn project_unnamed_field(field: &UnnamedField) -> ReflectedFieldDescriptor {
    ReflectedFieldDescriptor {
        name: field.index().to_string(),
        type_path: field.type_path().to_owned(),
        editor_attributes: field
            .get_attribute::<EditorFieldAttributes>()
            .map_or_else(EditorAttributes::default, project_field_attributes),
    }
}

fn project_variant(variant: &VariantInfo) -> ReflectedVariantDescriptor {
    let fields = match variant {
        VariantInfo::Struct(info) => info.iter().map(project_named_field).collect(),
        VariantInfo::Tuple(info) => info.iter().map(project_unnamed_field).collect(),
        VariantInfo::Unit(_) => Vec::new(),
    };
    ReflectedVariantDescriptor {
        name: variant.name().to_owned(),
        fields,
        editor_attributes: variant
            .get_attribute::<EditorTypeAttributes>()
            .map_or_else(EditorAttributes::default, project_type_attributes),
    }
}

fn type_editor_attributes(type_info: &TypeInfo) -> Option<&EditorTypeAttributes> {
    match type_info {
        TypeInfo::Struct(info) => info.get_attribute(),
        TypeInfo::TupleStruct(info) => info.get_attribute(),
        TypeInfo::Tuple(_)
        | TypeInfo::List(_)
        | TypeInfo::Array(_)
        | TypeInfo::Map(_)
        | TypeInfo::Set(_)
        | TypeInfo::Opaque(_) => None,
        TypeInfo::Enum(info) => info.get_attribute(),
    }
}

fn project_type_attributes(attributes: &EditorTypeAttributes) -> EditorAttributes {
    EditorAttributes {
        label: attributes.label.clone(),
        description: attributes.description.clone(),
        category: attributes.group.clone(),
        icon: attributes.icon.clone(),
        read_only: attributes.read_only,
        hidden: attributes.hidden,
        action_ids: attributes.action_ids.clone(),
        ..EditorAttributes::default()
    }
}

fn project_field_attributes(attributes: &EditorFieldAttributes) -> EditorAttributes {
    EditorAttributes {
        label: attributes.label.clone(),
        description: attributes.description.clone(),
        category: attributes.group.clone(),
        icon: attributes.icon.clone(),
        widget: Some(attributes.widget.projection_name()),
        range: attributes.range.as_ref().map(|range| NumericRange {
            minimum: range.minimum.clone(),
            maximum: range.maximum.clone(),
            step: range.step.clone(),
            suffix: range.suffix.clone(),
        }),
        read_only: attributes.read_only,
        hidden: attributes.hidden,
        action_ids: Vec::new(),
        constraints: FieldConstraints {
            minimum_length: attributes.constraints.minimum_length,
            maximum_length: attributes.constraints.maximum_length,
            allowed_strings: attributes.constraints.allowed_strings.clone(),
            allowed_variants: attributes.constraints.allowed_variants.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use az_core::{EditorNumericRange, EditorWidget};
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, declare_caps,
    };
    use az_prefab::{
        ErasedPrefabValue, Prefab, PrefabBuildError, PrefabMigrationStep, PrefabProductPolicy,
        PrefabTagAlias, ReflectPrefab,
        type_data::{construct_reflected, insert_reflected_component},
    };
    use bevy_ecs::{component::Component, reflect::ReflectComponent};
    use bevy_reflect::{FromType, Reflect, TypeRegistry, std_traits::ReflectDefault};

    use super::*;

    declare_caps!(FixtureCaps:);

    /// Stands in for a gem's runtime contribution: authored reflected types
    /// registered as one batch through the ordinary registrar.
    struct Fixtures(Vec<PrefabType>);

    impl Contribution for Fixtures {
        type Caps = FixtureCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.project-host-tests"),
                contribution: ContributionId::new("runtime"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, FixtureCaps>) {
            ctx.registrar::<PrefabType>()
                .register_many(self.0.iter().copied());
        }
    }

    fn compose(prefab_types: Vec<PrefabType>) -> Composer {
        let mut composer = Composer::new(GemTargetRole::ProjectHost);
        crate::tests::floor(&mut composer);
        composer
            .add(Fixtures(prefab_types), ProductActivation::default())
            .expect("an empty capability floor composes");
        composer
    }

    fn prefab_types(composer: &Composer) -> Option<&Registry<PrefabType>> {
        composer.registries().get::<PrefabType>()
    }

    #[derive(Component, Reflect, Default, Prefab)]
    #[reflect(Component, Default, Prefab)]
    #[prefab(tag = "ManifestA", version = 1)]
    struct ManifestA {
        value: f32,
    }

    #[derive(Component, Reflect, Default, Prefab)]
    #[reflect(Component, Default, Prefab)]
    #[prefab(tag = "ManifestB", version = 2)]
    struct ManifestB {
        value: bool,
    }

    fn manifest_types() -> Vec<PrefabType> {
        vec![PrefabType::of::<ManifestA>(), PrefabType::of::<ManifestB>()]
    }

    #[derive(Reflect)]
    #[reflect(@EditorTypeAttributes {
        label: Some("Projection Fixture".to_owned()),
        description: Some("Every static editor attribute".to_owned()),
        group: Some("ADR 0022".to_owned()),
        icon: Some("boxes".to_owned()),
        hidden: false,
        read_only: false,
        action_ids: vec!["rebuild.preview".to_owned()],
    })]
    // One reflected `bool` field per widget/attribute combination the
    // projection golden pins: Checkbox, Toggle, hidden, and read-only. Grouping
    // them into an enum would delete exactly what this fixture measures.
    #[allow(clippy::struct_excessive_bools)]
    struct ProjectionFixture {
        #[reflect(@EditorFieldAttributes {
            label: Some("Slider".to_owned()),
            description: Some("Bounded scalar".to_owned()),
            group: Some("Numbers".to_owned()),
            icon: Some("sliders".to_owned()),
            widget: EditorWidget::Slider,
            range: Some(EditorNumericRange {
                minimum: Some("-2".to_owned()),
                maximum: Some("2".to_owned()),
                step: Some("0.25".to_owned()),
                suffix: Some("m".to_owned()),
            }),
            constraints: az_core::EditorFieldConstraints::default(),
            hidden: false,
            read_only: false,
        })]
        slider: f32,
        #[reflect(@EditorFieldAttributes::new(
            "Default",
            EditorWidget::Default,
        ))]
        default_widget: String,
        #[reflect(@EditorFieldAttributes::new("Number", EditorWidget::Number))]
        number: f32,
        #[reflect(@EditorFieldAttributes::new(
            "Checkbox",
            EditorWidget::Checkbox,
        ))]
        checkbox: bool,
        #[reflect(@EditorFieldAttributes::new("Toggle", EditorWidget::Toggle))]
        toggle: bool,
        #[reflect(@EditorFieldAttributes::new(
            "Dropdown",
            EditorWidget::Dropdown { choices: vec!["A".to_owned(), "B".to_owned()] },
        ))]
        dropdown: String,
        #[reflect(@EditorFieldAttributes::new(
            "Asset",
            EditorWidget::AssetPicker { asset_type_path: "azoth.asset.Mesh".to_owned() },
        ))]
        asset: String,
        #[reflect(@EditorFieldAttributes::new(
            "Object",
            EditorWidget::ObjectPicker { object_type_path: "azoth.prefab.Entity".to_owned() },
        ))]
        object: String,
        #[reflect(@EditorFieldAttributes::new(
            "Notes",
            EditorWidget::Multiline { rows: Some(4) },
        ))]
        notes: String,
        #[reflect(@EditorFieldAttributes::new("Tint", EditorWidget::Color))]
        color: [f32; 4],
        #[reflect(@EditorFieldAttributes::new(
            "Direction",
            EditorWidget::Vector { dimensions: 3 },
        ))]
        vector: [f32; 3],
        #[reflect(@EditorFieldAttributes {
            label: Some("Internal".to_owned()),
            hidden: true,
            ..EditorFieldAttributes::default()
        })]
        internal: bool,
        #[reflect(@EditorFieldAttributes {
            label: Some("Locked".to_owned()),
            read_only: true,
            ..EditorFieldAttributes::default()
        })]
        locked: bool,
    }

    #[derive(Reflect)]
    struct TupleFixture(f32, bool);

    #[derive(Reflect)]
    enum EnumFixture {
        #[reflect(@EditorTypeAttributes::labeled("First Choice"))]
        First,
        Second {
            value: f32,
        },
    }

    fn projection_types() -> Vec<PrefabType> {
        vec![
            PrefabType::of::<ProjectionFixture>(),
            PrefabType::of::<TupleFixture>(),
            PrefabType::of::<(f32, bool)>(),
            PrefabType::of::<Vec<f32>>(),
            PrefabType::of::<BTreeMap<String, f32>>(),
            PrefabType::of::<EnumFixture>(),
            PrefabType::of::<[f32; 3]>(),
        ]
    }

    #[test]
    fn registry_projection_golden_covers_all_type_and_editor_attribute_kinds() {
        let composer = compose(projection_types());
        let projected = compose_type_registry(prefab_types(&composer)).expect("compose registry");
        let snapshot = project_type_registry(&projected.app_registry.read()).expect("projection");
        let observed_kinds = snapshot
            .types
            .iter()
            .filter(|descriptor| {
                descriptor.type_path.contains("ProjectionFixture")
                    || descriptor.type_path.contains("TupleFixture")
                    || descriptor.type_path.contains("EnumFixture")
                    || descriptor.type_path == <(f32, bool) as bevy_reflect::TypePath>::type_path()
                    || descriptor.type_path == <Vec<f32> as bevy_reflect::TypePath>::type_path()
                    || descriptor.type_path
                        == <BTreeMap<String, f32> as bevy_reflect::TypePath>::type_path()
                    || descriptor.type_path == <[f32; 3] as bevy_reflect::TypePath>::type_path()
                    || descriptor.type_path == <bool as bevy_reflect::TypePath>::type_path()
            })
            .map(|descriptor| format!("{:?}", descriptor.kind))
            .collect::<BTreeSet<_>>();
        for required in [
            "Struct",
            "TupleStruct",
            "Tuple",
            "List",
            "Map",
            "Enum",
            "Array { capacity: 3 }",
            "Bool",
        ] {
            assert!(
                observed_kinds.contains(required),
                "missing projected kind {required}"
            );
        }

        let descriptor = snapshot
            .types
            .iter()
            .find(|descriptor| descriptor.type_path.ends_with("ProjectionFixture"))
            .expect("projection fixture descriptor");
        assert_eq!(
            projection_golden_summary(descriptor),
            include_str!("../tests/fixtures/adr0022/registry-projection.golden").trim()
        );
    }

    #[test]
    fn registry_manifests_match_for_client_server_builder_and_project_host() {
        let composer = compose(manifest_types());
        let manifests = [
            RegistryConsumer::Client,
            RegistryConsumer::Server,
            RegistryConsumer::Builder,
            RegistryConsumer::ProjectHost,
        ]
        .map(|consumer| {
            registry_manifest_for_consumer(consumer, prefab_types(&composer))
                .expect("registry manifest")
        });
        assert!(manifests.windows(2).all(|pair| pair[0] == pair[1]));
        for tag in ["ManifestA", "ManifestB"] {
            assert!(
                manifests[0].prefabs.iter().any(|entry| entry.tag == tag),
                "composed Prefab type `{tag}` must reach every consumer manifest"
            );
        }
    }

    #[test]
    fn an_uncomposed_registry_still_reflects_the_engine_types() {
        let engine = compose_type_registry(None).expect("engine-only registry");
        let composer = compose(manifest_types());
        let with_gems = compose_type_registry(prefab_types(&composer)).expect("composed registry");

        assert!(
            !registry_manifest(&engine.app_registry.read())
                .prefabs
                .is_empty()
        );
        assert!(
            !registry_manifest(&engine.app_registry.read())
                .prefabs
                .iter()
                .any(|entry| entry.tag == "ManifestA"),
            "gem types reach the registry only through composition"
        );
        assert!(
            registry_manifest(&with_gems.app_registry.read())
                .prefabs
                .iter()
                .any(|entry| entry.tag == "ManifestA")
        );
    }

    #[test]
    fn startup_validation_rejects_duplicate_canonical_tags() {
        let mut registry = TypeRegistry::default();
        register_manual_prefab::<ManifestA>(&mut registry, prefab("Same"));
        register_manual_prefab::<ManifestB>(&mut registry, prefab("Same"));
        assert!(matches!(
            validate_type_registry(&registry),
            Err(RegistryValidationError::DuplicateCanonicalTag { .. })
        ));
    }

    #[test]
    fn startup_validation_rejects_alias_collisions() {
        let mut registry = TypeRegistry::default();
        register_manual_prefab::<ManifestA>(&mut registry, prefab("Canonical"));
        let mut second = prefab("Other");
        second.aliases = &[PrefabTagAlias {
            tag: "Canonical",
            source_version: 1,
        }];
        register_manual_prefab::<ManifestB>(&mut registry, second);
        assert!(matches!(
            validate_type_registry(&registry),
            Err(RegistryValidationError::AliasCollision { .. })
        ));
    }

    #[test]
    fn startup_validation_rejects_empty_and_reserved_tags() {
        for tag in ["", "  ", "__private", "core.bool", "azoth.internal.Hidden"] {
            let mut registry = TypeRegistry::default();
            register_manual_prefab::<ManifestA>(&mut registry, prefab(tag));
            assert!(matches!(
                validate_type_registry(&registry),
                Err(RegistryValidationError::InvalidCanonicalTag { .. })
            ));
        }
    }

    #[test]
    fn startup_validation_rejects_ambiguous_migration_edges() {
        assert_migration_error(&[migration(1, 2), migration(1, 2)], 2, |error| {
            matches!(
                error,
                RegistryValidationError::AmbiguousMigrationEdge { .. }
            )
        });
    }

    #[test]
    fn startup_validation_rejects_gapped_migration_chains() {
        assert_migration_error(&[migration(1, 3)], 3, |error| {
            matches!(error, RegistryValidationError::GappedMigration { .. })
        });
    }

    #[test]
    fn startup_validation_rejects_cyclic_migration_chains() {
        assert_migration_error(&[migration(2, 1)], 2, |error| {
            matches!(error, RegistryValidationError::CyclicMigration { .. })
        });
    }

    #[test]
    fn startup_validation_rejects_migration_chain_not_ending_at_current_version() {
        assert_migration_error(&[migration(1, 2)], 3, |error| {
            matches!(
                error,
                RegistryValidationError::MigrationCurrentVersionMismatch { .. }
            )
        });
    }

    #[derive(Reflect, Default)]
    struct NotAComponent;

    #[test]
    fn startup_validation_rejects_missing_reflect_component() {
        let mut registry = TypeRegistry::default();
        registry.register::<NotAComponent>();
        let registration = registry
            .get_mut(TypeId::of::<NotAComponent>())
            .expect("registration");
        registration.insert(prefab("NotAComponent"));
        assert!(matches!(
            validate_type_registry(&registry),
            Err(RegistryValidationError::MissingReflectComponent { .. })
        ));
    }

    #[derive(Component, Reflect, Default)]
    #[reflect(Component)]
    struct MissingConstructionData;

    #[test]
    fn startup_validation_rejects_missing_default_or_construction_adapter() {
        let mut registry = TypeRegistry::default();
        registry.register::<MissingConstructionData>();
        registry
            .get_mut(TypeId::of::<MissingConstructionData>())
            .expect("registration")
            .insert(prefab("MissingConstructionData"));
        assert!(matches!(
            validate_type_registry(&registry),
            Err(RegistryValidationError::MissingRequiredTypeData { .. })
        ));
    }

    fn projection_golden_summary(descriptor: &ReflectedTypeDescriptor) -> String {
        let mut lines = vec![
            format!("kind={:?}", descriptor.kind),
            format!(
                "label={}",
                descriptor.editor_attributes.label.as_deref().unwrap_or("")
            ),
            format!(
                "description={}",
                descriptor
                    .editor_attributes
                    .description
                    .as_deref()
                    .unwrap_or("")
            ),
            format!(
                "group={}",
                descriptor
                    .editor_attributes
                    .category
                    .as_deref()
                    .unwrap_or("")
            ),
            format!(
                "icon={}",
                descriptor.editor_attributes.icon.as_deref().unwrap_or("")
            ),
            format!(
                "actions={}",
                descriptor.editor_attributes.action_ids.join("|")
            ),
        ];
        lines.extend(descriptor.fields.iter().map(|field| {
            let range = field
                .editor_attributes
                .range
                .as_ref()
                .map_or_else(String::new, |range| {
                    format!(
                        "{}..{}:{}:{}",
                        range.minimum.as_deref().unwrap_or(""),
                        range.maximum.as_deref().unwrap_or(""),
                        range.step.as_deref().unwrap_or(""),
                        range.suffix.as_deref().unwrap_or("")
                    )
                });
            format!(
                "field={}:{}:{}:{}:{}:{}:{}:{}",
                field.name,
                field.editor_attributes.label.as_deref().unwrap_or(""),
                field.editor_attributes.description.as_deref().unwrap_or(""),
                field.editor_attributes.category.as_deref().unwrap_or(""),
                field.editor_attributes.icon.as_deref().unwrap_or(""),
                field.editor_attributes.widget.as_deref().unwrap_or(""),
                range,
                if field.editor_attributes.hidden {
                    "hidden"
                } else if field.editor_attributes.read_only {
                    "read_only"
                } else {
                    "editable"
                },
            )
        }));
        lines.join("\n")
    }

    fn register_manual_prefab<T>(registry: &mut TypeRegistry, data: PrefabTypeData)
    where
        T: Reflect + bevy_reflect::GetTypeRegistration + Default + Component,
        ReflectComponent: FromType<T>,
        ReflectDefault: FromType<T>,
    {
        registry.register::<T>();
        let registration = registry.get_mut(TypeId::of::<T>()).expect("registration");
        registration.insert(data);
        if !registration.contains::<ReflectDefault>() {
            registration.insert(<ReflectDefault as FromType<T>>::from_type());
        }
        if !registration.contains::<ReflectComponent>() {
            registration.insert(<ReflectComponent as FromType<T>>::from_type());
        }
    }

    fn prefab(tag: &'static str) -> PrefabTypeData {
        PrefabTypeData {
            tag,
            source_version: 1,
            aliases: &[],
            migrations: &[],
            construction: PrefabConstruction::ReflectDefaultOrFromWorld,
            product_policy: PrefabProductPolicy::Runtime,
            construct: construct_reflected,
            insert: insert_reflected_component,
        }
    }

    fn migration(from_version: u32, to_version: u32) -> PrefabMigrationStep {
        PrefabMigrationStep {
            from_version,
            to_version,
            migrate: identity_migration,
        }
    }

    // Stored in `PrefabMigrationStep::migrate`, whose fn-pointer type fixes the
    // fallible signature; dropping the `Result` would not type-check there.
    #[allow(clippy::unnecessary_wraps)]
    fn identity_migration(value: ErasedPrefabValue) -> Result<ErasedPrefabValue, PrefabBuildError> {
        Ok(value)
    }

    fn assert_migration_error(
        migrations: &[PrefabMigrationStep],
        current_version: u32,
        predicate: impl FnOnce(RegistryValidationError) -> bool,
    ) {
        let mut registry = TypeRegistry::default();
        let mut data = prefab("MigrationFixture");
        data.source_version = current_version;
        data.migrations = Box::leak(migrations.to_vec().into_boxed_slice());
        register_manual_prefab::<ManifestA>(&mut registry, data);
        let error = validate_type_registry(&registry).expect_err("malformed migration should fail");
        assert!(predicate(error));
    }
}
