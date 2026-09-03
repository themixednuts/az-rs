//! ADR 0022 typed Prefab project-host state and command policy.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fmt::Write as _,
    path::{Component, Path, PathBuf},
};

use az_core::{
    DiagnosticSeverity as CoreDiagnosticSeverity, EditorActionId, EditorActionOutcome,
    EditorPolicyTypeData, ReflectedPathSegment as CorePathSegment, ValidationTypeData,
};
use az_filesystem::AzothDataHome;
use az_gem_contract::Registry;
use az_prefab::{
    EntityAlias, InstanceAlias, OverrideOperation, PrefabAssetPath, PrefabCodec, PrefabDocument,
    PrefabEntity, PrefabInstance, PrefabType, PrefabTypeData, ReflectedPath as PrefabReflectedPath,
    SparseValue, TypedOverrideAction, TypedOverrideTarget, TypedPrefabSemantics,
};
use az_project::{SourceRevision, SourceSession};
use az_proto_project::vnext::{
    DiagnosticSeverity, PrefabComponentSnapshot, PrefabDiagnostic, PrefabEditCommand,
    PrefabEntitySnapshot, PrefabHierarchyEdge, PrefabInstanceSnapshot, PrefabOverrideOperation,
    PrefabOverrideSnapshot, PrefabRpcResult, PrefabSourceSnapshot, PrefabValueTarget,
    ReflectedPath, ReflectedPathSegment, ReflectedValueEncoding, ReflectedValueEnvelope,
    SourceSessionCommand, SourceSessionResult, SourceSessionStatus, TypeRegistrySnapshot,
    TypedActionResult,
};
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_reflect::{
    PartialReflect, ReflectMut, ReflectRef, TypeRegistration, TypeRegistry,
    enums::{DynamicEnum, DynamicVariant, VariantInfo},
    std_traits::ReflectDefault,
    structs::DynamicStruct,
    tuple::DynamicTuple,
};
use thiserror::Error;
use tracing::{info, instrument};

use crate::{
    ComposedTypeRegistry, FileSourcePersistence, NoSourceRecoveryStore, SourceDiagnostic,
    SourceDiagnosticPhase, SourceSessionHost, SourceSessionHostError, TypedSourcePolicy,
    compose_type_registry, project_type_registry,
};

enum PrefabSourceCommand {
    Edit(PrefabEditCommand),
    Action {
        target: PrefabValueTarget,
        action_id: String,
    },
}

struct PrefabSourcePolicy {
    registry: AppTypeRegistry,
    action_outcome: RefCell<Option<EditorActionOutcome>>,
}

impl PrefabSourcePolicy {
    fn new(registry: &ComposedTypeRegistry) -> Self {
        Self {
            registry: registry.app_registry.clone(),
            action_outcome: RefCell::new(None),
        }
    }

    fn take_action_outcome(&self) -> Option<EditorActionOutcome> {
        self.action_outcome.borrow_mut().take()
    }
}

impl TypedSourcePolicy for PrefabSourcePolicy {
    type Value = PrefabDocument;
    type Command = PrefabSourceCommand;

    fn decode(&self, bytes: &[u8]) -> Result<Self::Value, SourceDiagnostic> {
        let source = std::str::from_utf8(bytes).map_err(|error| {
            SourceDiagnostic::new(SourceDiagnosticPhase::Serialize, error.to_string())
        })?;
        let registry = self.registry.read();
        PrefabCodec::new(&registry)
            .and_then(|codec| codec.decode(source))
            .map_err(|error| {
                SourceDiagnostic::new(SourceDiagnosticPhase::Serialize, error.to_string())
            })
    }

    fn apply(
        &self,
        candidate: &mut Self::Value,
        command: &Self::Command,
    ) -> Result<(), SourceDiagnostic> {
        self.action_outcome.borrow_mut().take();
        let registry = self.registry.read();
        let codec = PrefabCodec::new(&registry).map_err(edit_diagnostic)?;
        match command {
            PrefabSourceCommand::Edit(command) => {
                apply_prefab_edit(candidate, command, &codec, &registry)
            }
            PrefabSourceCommand::Action { target, action_id } => {
                let outcome = invoke_action(candidate, target, action_id, &registry)?;
                drop(registry);
                self.action_outcome.replace(Some(outcome));
                Ok(())
            }
        }
    }

    fn validate_document(&self, candidate: &Self::Value) -> Vec<SourceDiagnostic> {
        let registry = self.registry.read();
        validate_components(candidate, &registry)
            .into_iter()
            .map(|diagnostic| {
                SourceDiagnostic::new(
                    SourceDiagnosticPhase::WholeDocument,
                    format!("{}: {}", diagnostic.code, diagnostic.message),
                )
                .at_path(diagnostic_path(&diagnostic))
            })
            .collect()
    }

    fn validate_prefab_semantics(&self, candidate: &Self::Value) -> Vec<SourceDiagnostic> {
        let registry = self.registry.read();
        TypedPrefabSemantics::validate_local(candidate, &registry)
            .err()
            .map(|error| {
                SourceDiagnostic::new(SourceDiagnosticPhase::PrefabSemantics, error.to_string())
            })
            .into_iter()
            .collect()
    }

    fn validate_dependencies(&self, _candidate: &Self::Value) -> Vec<SourceDiagnostic> {
        Vec::new()
    }

    fn serialize(&self, candidate: &Self::Value) -> Result<Vec<u8>, SourceDiagnostic> {
        let registry = self.registry.read();
        PrefabCodec::new(&registry)
            .and_then(|codec| codec.encode(candidate))
            .map(String::into_bytes)
            .map_err(|error| {
                SourceDiagnostic::new(SourceDiagnosticPhase::Serialize, error.to_string())
            })
    }
}

type PrefabSessions = SourceSessionHost<
    PrefabSourcePolicy,
    FileSourcePersistence,
    NoSourceRecoveryStore<PrefabDocument>,
>;

/// Stateful typed service used by the additive vNext RPC methods.
pub struct VNextProjectHost {
    source_root: Option<PathBuf>,
    registry: ComposedTypeRegistry,
    session_host: PrefabSessions,
    sessions: BTreeMap<String, SourceSession<PrefabDocument>>,
}

impl VNextProjectHost {
    pub(crate) fn compose(
        source_root: Option<&Path>,
        prefab_types: Option<&Registry<PrefabType>>,
    ) -> Result<Self, VNextHostError> {
        let registry = compose_type_registry(prefab_types)?;
        let source_root = source_root.map(Path::to_path_buf);
        let transaction_root = if let Some(root) = source_root.as_deref() {
            let manifest = az_project::load_project_manifest(root)?;
            let data_paths = AzothDataHome::resolve().project(&manifest.project.name, root);
            data_paths.prepare()?;
            data_paths
                .project_host_transactions_dir()
                .join("project-host-vnext")
        } else {
            let data_home = AzothDataHome::resolve();
            data_home.prepare()?;
            data_home.runtime_dir().join("project-host-vnext")
        };
        let session_host = SourceSessionHost::new(
            PrefabSourcePolicy::new(&registry),
            FileSourcePersistence::new(transaction_root),
            NoSourceRecoveryStore::new(),
        );
        Ok(Self {
            source_root,
            registry,
            session_host,
            sessions: BTreeMap::new(),
        })
    }

    /// The composed reflected registry this host serves.
    ///
    /// Only the test-support surface reads this back out, so it is gated the
    /// same way [`crate::ProjectHostRpc::registry`] is.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) const fn registry(&self) -> &ComposedTypeRegistry {
        &self.registry
    }

    pub(crate) fn registry_snapshot(&self) -> Result<TypeRegistrySnapshot, VNextHostError> {
        Ok(project_type_registry(&self.registry.app_registry.read())?)
    }

    /// Deterministic 32-byte identity of the composed reflected type catalog.
    ///
    /// Runtime launch snapshots carry this so the runtime host can confirm it
    /// was cooked against the same schema catalog the editor authored against.
    /// It folds the sorted set of reflected type paths, so it changes only when
    /// the catalog composition changes.
    pub(crate) fn schema_catalog_hash(&self) -> Result<Vec<u8>, VNextHostError> {
        Ok(self.registry_snapshot()?.schema_catalog_hash)
    }

    pub(crate) fn prefab_snapshot(&self, source_path: &str) -> PrefabRpcResult {
        match self.snapshot(source_path) {
            Ok(snapshot) => PrefabRpcResult {
                snapshot: Some(snapshot),
                diagnostics: Vec::new(),
            },
            Err(error) => PrefabRpcResult {
                snapshot: None,
                diagnostics: vec![host_diagnostic(&error)],
            },
        }
    }

    #[instrument(skip_all, fields(source_path, expected_revision))]
    pub(crate) fn apply_edit(
        &mut self,
        source_path: &str,
        expected_revision: u64,
        command: PrefabEditCommand,
    ) -> PrefabRpcResult {
        let result = self.commit(
            source_path,
            expected_revision,
            &PrefabSourceCommand::Edit(command),
        );
        match result.and_then(|()| self.snapshot(source_path)) {
            Ok(snapshot) => PrefabRpcResult {
                snapshot: Some(snapshot),
                diagnostics: Vec::new(),
            },
            Err(error) => PrefabRpcResult {
                snapshot: self.snapshot(source_path).ok(),
                diagnostics: diagnostics_from_host_error(error),
            },
        }
    }

    #[instrument(skip_all, fields(source_path, expected_revision, action_id))]
    pub(crate) fn invoke_action(
        &mut self,
        source_path: &str,
        expected_revision: u64,
        target: PrefabValueTarget,
        action_id: String,
    ) -> TypedActionResult {
        let component_type_path = target.path.component_type_path.clone();
        let result = self.commit(
            source_path,
            expected_revision,
            &PrefabSourceCommand::Action { target, action_id },
        );
        match result {
            Ok(()) => {
                let outcome = self
                    .session_host
                    .policy()
                    .take_action_outcome()
                    .unwrap_or_default();
                TypedActionResult {
                    snapshot: self.snapshot(source_path).ok(),
                    changed_paths: outcome
                        .changed_paths
                        .into_iter()
                        .map(|path| ReflectedPath {
                            component_type_path: component_type_path.clone(),
                            segments: path.0.into_iter().map(project_core_segment).collect(),
                        })
                        .collect(),
                    diagnostics: outcome
                        .diagnostics
                        .into_iter()
                        .map(|diagnostic| PrefabDiagnostic {
                            severity: project_severity(diagnostic.severity),
                            code: diagnostic.code,
                            message: diagnostic.message,
                            target: None,
                        })
                        .collect(),
                }
            }
            Err(error) => TypedActionResult {
                snapshot: self.snapshot(source_path).ok(),
                changed_paths: Vec::new(),
                diagnostics: diagnostics_from_host_error(error),
            },
        }
    }

    pub(crate) fn diagnostics(&self, source_path: &str) -> Vec<PrefabDiagnostic> {
        let Some(session) = self.sessions.get(source_path) else {
            return vec![host_diagnostic(&VNextHostError::SessionNotOpen(
                source_path.to_owned(),
            ))];
        };
        validate_components(session.value(), &self.registry.app_registry.read())
    }

    #[instrument(skip_all, fields(source_path, ?command, expected_revision))]
    pub(crate) fn lifecycle(
        &mut self,
        source_path: &str,
        command: SourceSessionCommand,
        expected_revision: u64,
    ) -> SourceSessionResult {
        let result = self.lifecycle_inner(source_path, command, expected_revision);
        match result {
            Ok(()) => self.session_result(source_path, Vec::new()),
            Err(error) => self.session_result(source_path, diagnostics_from_host_error(error)),
        }
    }

    fn lifecycle_inner(
        &mut self,
        source_path: &str,
        command: SourceSessionCommand,
        expected_revision: u64,
    ) -> Result<(), VNextHostError> {
        match command {
            SourceSessionCommand::Open => {
                if self.sessions.contains_key(source_path) {
                    return Ok(());
                }
                let path = self.resolve_source_path(source_path)?;
                let session = self.session_host.open(path)?;
                self.sessions.insert(source_path.to_owned(), session);
                info!(source_path, "opened vNext typed Prefab source session");
            }
            SourceSessionCommand::Save => {
                self.require_revision(source_path, expected_revision)?;
                // Typed commits persist atomically; Save is an explicit barrier/status check.
            }
            SourceSessionCommand::SaveRecovery => {
                self.require_revision(source_path, expected_revision)?;
                let (host, sessions) = (&mut self.session_host, &self.sessions);
                let session = sessions
                    .get(source_path)
                    .ok_or_else(|| VNextHostError::SessionNotOpen(source_path.to_owned()))?;
                if let Some(diagnostic) = host.save_recovery(session) {
                    return Err(VNextHostError::Diagnostic(diagnostic.message));
                }
            }
            SourceSessionCommand::Undo => {
                let (host, sessions) = (&mut self.session_host, &mut self.sessions);
                let session = sessions
                    .get_mut(source_path)
                    .ok_or_else(|| VNextHostError::SessionNotOpen(source_path.to_owned()))?;
                host.undo(session, SourceRevision::new(expected_revision))?;
            }
            SourceSessionCommand::Redo => {
                let (host, sessions) = (&mut self.session_host, &mut self.sessions);
                let session = sessions
                    .get_mut(source_path)
                    .ok_or_else(|| VNextHostError::SessionNotOpen(source_path.to_owned()))?;
                host.redo(session, SourceRevision::new(expected_revision))?;
            }
            SourceSessionCommand::Close => {
                self.require_revision(source_path, expected_revision)?;
                let (host, sessions) = (&mut self.session_host, &self.sessions);
                let session = sessions
                    .get(source_path)
                    .ok_or_else(|| VNextHostError::SessionNotOpen(source_path.to_owned()))?;
                if let Some(diagnostic) = host.close(session) {
                    return Err(VNextHostError::Diagnostic(diagnostic.message));
                }
                self.sessions.remove(source_path);
                info!(source_path, "closed vNext typed Prefab source session");
            }
            SourceSessionCommand::Status => {}
        }
        Ok(())
    }

    fn commit(
        &mut self,
        source_path: &str,
        expected_revision: u64,
        command: &PrefabSourceCommand,
    ) -> Result<(), VNextHostError> {
        let (host, sessions) = (&mut self.session_host, &mut self.sessions);
        let session = sessions
            .get_mut(source_path)
            .ok_or_else(|| VNextHostError::SessionNotOpen(source_path.to_owned()))?;
        host.commit(session, SourceRevision::new(expected_revision), command)?;
        Ok(())
    }

    fn snapshot(&self, source_path: &str) -> Result<PrefabSourceSnapshot, VNextHostError> {
        let session = self.session(source_path)?;
        project_prefab_snapshot(session.value(), session.revision().value(), &self.registry)
    }

    fn session(&self, source_path: &str) -> Result<&SourceSession<PrefabDocument>, VNextHostError> {
        self.sessions
            .get(source_path)
            .ok_or_else(|| VNextHostError::SessionNotOpen(source_path.to_owned()))
    }

    fn require_revision(
        &self,
        source_path: &str,
        expected_revision: u64,
    ) -> Result<(), VNextHostError> {
        let actual = self.session(source_path)?.revision().value();
        if actual == expected_revision {
            Ok(())
        } else {
            Err(VNextHostError::RevisionConflict {
                expected: expected_revision,
                actual,
            })
        }
    }

    fn resolve_source_path(&self, source_path: &str) -> Result<PathBuf, VNextHostError> {
        let root = self
            .source_root
            .as_ref()
            .ok_or(VNextHostError::MissingSourceRoot)?;
        let relative = Path::new(source_path);
        if source_path.trim() != source_path
            || source_path.is_empty()
            || relative.is_absolute()
            || source_path.contains('\\')
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::Prefix(_)
                        | Component::RootDir
                        | Component::ParentDir
                        | Component::CurDir
                )
            })
        {
            return Err(VNextHostError::InvalidSourcePath(source_path.to_owned()));
        }
        Ok(root.join(relative))
    }

    fn session_result(
        &self,
        source_path: &str,
        diagnostics: Vec<PrefabDiagnostic>,
    ) -> SourceSessionResult {
        let session = self.sessions.get(source_path);
        SourceSessionResult {
            status: session.map_or(
                SourceSessionStatus {
                    open: false,
                    revision: 0,
                    dirty: false,
                    undo_depth: 0,
                    redo_depth: 0,
                },
                |session| SourceSessionStatus {
                    open: true,
                    revision: session.revision().value(),
                    dirty: session.is_dirty(),
                    undo_depth: u32::try_from(session.undo_len()).unwrap_or(u32::MAX),
                    redo_depth: u32::try_from(session.redo_len()).unwrap_or(u32::MAX),
                },
            ),
            snapshot: session.and_then(|_| self.snapshot(source_path).ok()),
            diagnostics,
        }
    }
}

// The codec borrows the read guard for its whole lifetime (`PrefabCodec<'a>`
// holds `&'a TypeRegistry`), so clippy's "drop the guard here" rewrite does not
// compile.
#[allow(clippy::significant_drop_tightening)]
fn project_prefab_snapshot(
    document: &PrefabDocument,
    revision: u64,
    registry: &ComposedTypeRegistry,
) -> Result<PrefabSourceSnapshot, VNextHostError> {
    let registry_guard = registry.app_registry.read();
    let codec = PrefabCodec::new(&registry_guard)?;
    let type_versions = project_type_versions(document, registry)?;
    let entities = project_entities(document, &codec)?;
    let instances = project_instances(document, &codec)?;

    Ok(PrefabSourceSnapshot {
        document_version: document.version,
        type_versions,
        entities: entities.entities,
        hierarchy: entities.hierarchy,
        components: entities.components,
        instances: instances.instances,
        overrides: instances.overrides,
        revision,
    })
}

struct ProjectedEntities {
    entities: Vec<PrefabEntitySnapshot>,
    hierarchy: Vec<PrefabHierarchyEdge>,
    components: Vec<PrefabComponentSnapshot>,
}

struct ProjectedInstances {
    instances: Vec<PrefabInstanceSnapshot>,
    overrides: Vec<PrefabOverrideSnapshot>,
}

fn project_type_versions(
    document: &PrefabDocument,
    registry: &ComposedTypeRegistry,
) -> Result<BTreeMap<String, u32>, VNextHostError> {
    document
        .type_versions
        .iter()
        .map(|(tag, version)| {
            Ok((
                registry
                    .tag_to_type_path
                    .get(tag)
                    .cloned()
                    .ok_or_else(|| VNextHostError::UnknownComponent(tag.clone()))?,
                *version,
            ))
        })
        .collect()
}

fn project_entities(
    document: &PrefabDocument,
    codec: &PrefabCodec<'_>,
) -> Result<ProjectedEntities, VNextHostError> {
    let mut entities = Vec::with_capacity(document.entities.len());
    let mut hierarchy = Vec::with_capacity(document.entities.len());
    let mut components = Vec::new();
    for (alias, entity) in &document.entities {
        entities.push(PrefabEntitySnapshot {
            alias: alias.as_str().to_owned(),
        });
        hierarchy.push(PrefabHierarchyEdge {
            child_alias: alias.as_str().to_owned(),
            parent_alias: entity
                .parent
                .as_ref()
                .map(|parent| parent.as_str().to_owned()),
        });
        for value in entity.components.values() {
            components.push(PrefabComponentSnapshot {
                entity_alias: alias.as_str().to_owned(),
                type_path: value.type_path().to_owned(),
                sparse_value: ReflectedValueEnvelope {
                    type_path: value.type_path().to_owned(),
                    encoding: ReflectedValueEncoding::TypedRon,
                    payload: codec.encode_sparse_value(value)?,
                },
            });
        }
    }
    Ok(ProjectedEntities {
        entities,
        hierarchy,
        components,
    })
}

fn project_instances(
    document: &PrefabDocument,
    codec: &PrefabCodec<'_>,
) -> Result<ProjectedInstances, VNextHostError> {
    let mut instances = Vec::with_capacity(document.instances.len());
    let mut overrides = Vec::new();
    for (alias, instance) in &document.instances {
        instances.push(PrefabInstanceSnapshot {
            alias: alias.as_str().to_owned(),
            source_asset: instance.source.as_str().to_owned(),
            parent_entity_alias: instance
                .parent
                .as_ref()
                .map(|parent| parent.as_str().to_owned()),
        });
        for operation in &instance.overrides {
            let mut instance_alias_chain = vec![alias.as_str().to_owned()];
            instance_alias_chain.extend(
                operation
                    .target
                    .instance_chain
                    .iter()
                    .map(|alias| alias.as_str().to_owned()),
            );
            let target = PrefabValueTarget {
                instance_alias_chain,
                entity_alias: operation.target.entity.as_str().to_owned(),
                path: ReflectedPath {
                    component_type_path: operation.target.component.clone(),
                    segments: operation
                        .target
                        .path
                        .segments()
                        .iter()
                        .cloned()
                        .map(ReflectedPathSegment::Field)
                        .collect(),
                },
            };
            overrides.push(PrefabOverrideSnapshot {
                operation: project_override_operation(&operation.action, target, codec)?,
            });
        }
    }
    Ok(ProjectedInstances {
        instances,
        overrides,
    })
}

fn project_override_operation(
    action: &TypedOverrideAction,
    target: PrefabValueTarget,
    codec: &PrefabCodec<'_>,
) -> Result<PrefabOverrideOperation, VNextHostError> {
    Ok(match action {
        TypedOverrideAction::Set(value) => PrefabOverrideOperation::Set {
            target,
            value: reflected_envelope(value, codec)?,
        },
        TypedOverrideAction::Clear => PrefabOverrideOperation::Clear { target },
        TypedOverrideAction::Insert { index, value } => PrefabOverrideOperation::Insert {
            target,
            index: override_index(*index)?,
            value: reflected_envelope(value, codec)?,
        },
        TypedOverrideAction::Remove { index } => PrefabOverrideOperation::Remove {
            target,
            index: override_index(*index)?,
        },
        TypedOverrideAction::Move { from, to } => PrefabOverrideOperation::Move {
            target,
            from: override_index(*from)?,
            to: override_index(*to)?,
        },
    })
}

fn reflected_envelope(
    value: &SparseValue,
    codec: &PrefabCodec<'_>,
) -> Result<ReflectedValueEnvelope, VNextHostError> {
    Ok(ReflectedValueEnvelope {
        type_path: value.type_path().to_owned(),
        encoding: ReflectedValueEncoding::TypedRon,
        payload: codec.encode_sparse_value(value)?,
    })
}

fn override_index(index: usize) -> Result<u32, VNextHostError> {
    u32::try_from(index).map_err(|_| {
        VNextHostError::Diagnostic(format!(
            "override index {index} exceeds the vNext u32 contract"
        ))
    })
}

fn apply_prefab_edit(
    document: &mut PrefabDocument,
    command: &PrefabEditCommand,
    codec: &PrefabCodec<'_>,
    registry: &TypeRegistry,
) -> Result<(), SourceDiagnostic> {
    match command {
        PrefabEditCommand::SetValue { target, value } => {
            set_component_value(document, target, value, codec)
        }
        PrefabEditCommand::ListInsert {
            target,
            index,
            value,
        } => list_insert(document, target, *index, value, codec),
        PrefabEditCommand::ListRemove { target, index } => list_remove(document, target, *index),
        PrefabEditCommand::ListMove { target, from, to } => list_move(document, target, *from, *to),
        PrefabEditCommand::MapInsert { target, key, value } => {
            map_insert(document, target, key, value, codec)
        }
        PrefabEditCommand::MapRemove { target, key } => map_remove(document, target, key, codec),
        PrefabEditCommand::SetVariant {
            target,
            variant_name,
            value,
        } => set_variant(
            document,
            target,
            variant_name,
            value.as_ref(),
            codec,
            registry,
        ),
        PrefabEditCommand::AddComponent {
            entity_alias,
            component_type_path,
            initial_value,
        } => add_component(
            document,
            entity_alias,
            component_type_path,
            initial_value.as_ref(),
            codec,
            registry,
        ),
        PrefabEditCommand::RemoveComponent {
            entity_alias,
            component_type_path,
        } => remove_component(document, entity_alias, component_type_path),
        PrefabEditCommand::AddEntity {
            alias,
            parent_alias,
        } => add_entity(document, alias, parent_alias.as_ref()),
        PrefabEditCommand::RemoveEntity { alias } => remove_entity(document, alias),
        PrefabEditCommand::ReparentEntity {
            alias,
            parent_alias,
        } => reparent_entity(document, alias, parent_alias.as_ref()),
        PrefabEditCommand::AddInstance {
            alias,
            source_asset,
            parent_entity_alias,
        } => add_instance(document, alias, source_asset, parent_entity_alias.as_ref()),
        PrefabEditCommand::RemoveInstance { alias } => remove_instance(document, alias),
        PrefabEditCommand::ReparentInstance {
            alias,
            parent_entity_alias,
        } => reparent_instance(document, alias, parent_entity_alias.as_ref()),
        PrefabEditCommand::SetOverride { target, value } => {
            set_override(document, target, value, codec)
        }
        PrefabEditCommand::ClearOverride { target } => clear_override(document, target),
        PrefabEditCommand::InsertOverride {
            target,
            index,
            value,
        } => insert_override(document, target, *index, value, codec),
        PrefabEditCommand::RemoveOverrideItem { target, index } => {
            remove_override_item(document, target, *index)
        }
        PrefabEditCommand::MoveOverride { target, from, to } => {
            move_override(document, target, *from, *to)
        }
        PrefabEditCommand::RemoveOverride { target } => remove_override(document, target),
    }
}

fn set_component_value(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    value: &ReflectedValueEnvelope,
    codec: &PrefabCodec<'_>,
) -> Result<(), SourceDiagnostic> {
    ensure_direct_target(target)?;
    let decoded = decode_envelope(codec, value)?;
    let (_, component) = component_mut(document, target)?;
    if target.path.segments.is_empty() {
        *component = decoded;
    } else {
        set_value_at_path(component.value_mut(), &target.path.segments, &decoded)?;
    }
    Ok(())
}

fn list_insert(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    index: u32,
    value: &ReflectedValueEnvelope,
    codec: &PrefabCodec<'_>,
) -> Result<(), SourceDiagnostic> {
    ensure_direct_target(target)?;
    let decoded = decode_envelope(codec, value)?;
    let (_, component) = component_mut(document, target)?;
    let target_value = value_at_path_mut(component.value_mut(), &target.path.segments)?;
    let ReflectMut::List(list) = target_value.reflect_mut() else {
        return Err(command_error("list insert target is not a list"));
    };
    let index = index as usize;
    if index > list.len() {
        return Err(command_error(format!(
            "list insert index {index} exceeds length {}",
            list.len()
        )));
    }
    list.insert(index, clone_partial(decoded.value()));
    Ok(())
}

fn list_remove(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    index: u32,
) -> Result<(), SourceDiagnostic> {
    ensure_direct_target(target)?;
    let (_, component) = component_mut(document, target)?;
    let target_value = value_at_path_mut(component.value_mut(), &target.path.segments)?;
    let ReflectMut::List(list) = target_value.reflect_mut() else {
        return Err(command_error("list remove target is not a list"));
    };
    let index = index as usize;
    if index >= list.len() {
        return Err(command_error(format!(
            "list remove index {index} exceeds length {}",
            list.len()
        )));
    }
    list.remove(index);
    Ok(())
}

fn list_move(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    from: u32,
    to: u32,
) -> Result<(), SourceDiagnostic> {
    ensure_direct_target(target)?;
    let (_, component) = component_mut(document, target)?;
    let target_value = value_at_path_mut(component.value_mut(), &target.path.segments)?;
    let ReflectMut::List(list) = target_value.reflect_mut() else {
        return Err(command_error("list move target is not a list"));
    };
    let (from, to) = (from as usize, to as usize);
    if from >= list.len() || to >= list.len() {
        return Err(command_error(format!(
            "list move {from}->{to} exceeds length {}",
            list.len()
        )));
    }
    let value = list.remove(from);
    list.insert(to, value);
    Ok(())
}

fn map_insert(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    key: &ReflectedValueEnvelope,
    value: &ReflectedValueEnvelope,
    codec: &PrefabCodec<'_>,
) -> Result<(), SourceDiagnostic> {
    ensure_direct_target(target)?;
    let key = decode_envelope(codec, key)?;
    let value = decode_envelope(codec, value)?;
    let (_, component) = component_mut(document, target)?;
    let target_value = value_at_path_mut(component.value_mut(), &target.path.segments)?;
    let ReflectMut::Map(map) = target_value.reflect_mut() else {
        return Err(command_error("map insert target is not a map"));
    };
    map.insert_boxed(clone_partial(key.value()), clone_partial(value.value()));
    Ok(())
}

fn map_remove(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    key: &ReflectedValueEnvelope,
    codec: &PrefabCodec<'_>,
) -> Result<(), SourceDiagnostic> {
    ensure_direct_target(target)?;
    let key = decode_envelope(codec, key)?;
    let (_, component) = component_mut(document, target)?;
    let target_value = value_at_path_mut(component.value_mut(), &target.path.segments)?;
    let ReflectMut::Map(map) = target_value.reflect_mut() else {
        return Err(command_error("map remove target is not a map"));
    };
    if map.remove(key.value()).is_none() {
        return Err(command_error("map key is not present"));
    }
    Ok(())
}

fn set_variant(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    variant_name: &str,
    value: Option<&ReflectedValueEnvelope>,
    codec: &PrefabCodec<'_>,
    registry: &TypeRegistry,
) -> Result<(), SourceDiagnostic> {
    ensure_direct_target(target)?;
    let (_, component) = component_mut(document, target)?;
    let target_value = value_at_path_mut(component.value_mut(), &target.path.segments)?;
    if let Some(value) = value {
        let decoded = decode_envelope(codec, value)?;
        let ReflectRef::Enum(decoded_enum) = decoded.value().reflect_ref() else {
            return Err(command_error("variant value does not encode an enum"));
        };
        if decoded_enum.variant_name() != variant_name {
            return Err(command_error(format!(
                "variant payload is `{}`, expected `{variant_name}`",
                decoded_enum.variant_name()
            )));
        }
        target_value
            .try_apply(decoded.value())
            .map_err(|error| command_error(error.to_string()))?;
    } else {
        let value = empty_variant(registry, target_value, variant_name)?;
        target_value
            .try_apply(&value)
            .map_err(|error| command_error(error.to_string()))?;
    }
    Ok(())
}

fn add_component(
    document: &mut PrefabDocument,
    entity_alias: &str,
    component_type_path: &str,
    initial_value: Option<&ReflectedValueEnvelope>,
    codec: &PrefabCodec<'_>,
    registry: &TypeRegistry,
) -> Result<(), SourceDiagnostic> {
    let alias = EntityAlias::new(entity_alias.to_owned()).map_err(edit_diagnostic)?;
    let entity = document
        .entities
        .get_mut(&alias)
        .ok_or_else(|| command_error(format!("entity `{entity_alias}` does not exist")))?;
    let registration = registry
        .get_with_type_path(component_type_path)
        .ok_or_else(|| command_error(format!("unknown type `{component_type_path}`")))?;
    let prefab = registration.data::<PrefabTypeData>().ok_or_else(|| {
        command_error(format!(
            "type `{component_type_path}` is not Prefab-capable"
        ))
    })?;
    if entity.components.contains_key(prefab.tag) {
        return Err(command_error(format!(
            "entity `{entity_alias}` already has `{component_type_path}`"
        )));
    }
    let value = initial_value.map_or_else(
        || default_sparse(registration),
        |value| decode_envelope(codec, value),
    )?;
    if value.type_path() != registration.type_info().type_path() {
        return Err(command_error(format!(
            "component value represents `{}`, expected `{component_type_path}`",
            value.type_path()
        )));
    }
    entity.components.insert(prefab.tag.to_owned(), value);
    document
        .type_versions
        .insert(prefab.tag.to_owned(), prefab.source_version);
    Ok(())
}

/// # Panics
///
/// Panics if the entity the component lookup just resolved is gone; the same
/// `&mut` borrow spans both steps, so nothing can remove it in between.
fn remove_component(
    document: &mut PrefabDocument,
    entity_alias: &str,
    component_type_path: &str,
) -> Result<(), SourceDiagnostic> {
    let target = PrefabValueTarget {
        instance_alias_chain: Vec::new(),
        entity_alias: entity_alias.to_owned(),
        path: ReflectedPath {
            component_type_path: component_type_path.to_owned(),
            segments: Vec::new(),
        },
    };
    let (tag, _) = component_mut(document, &target)?;
    document
        .entities
        .get_mut(&EntityAlias::new(entity_alias.to_owned()).map_err(edit_diagnostic)?)
        .expect("component lookup verified entity")
        .components
        .remove(&tag);
    if !document
        .entities
        .values()
        .any(|entity| entity.components.contains_key(&tag))
    {
        document.type_versions.remove(&tag);
    }
    Ok(())
}

fn add_entity(
    document: &mut PrefabDocument,
    alias: &str,
    parent_alias: Option<&String>,
) -> Result<(), SourceDiagnostic> {
    let alias = EntityAlias::new(alias.to_owned()).map_err(edit_diagnostic)?;
    let parent = parent_alias
        .cloned()
        .map(EntityAlias::new)
        .transpose()
        .map_err(edit_diagnostic)?;
    if document
        .entities
        .insert(
            alias,
            PrefabEntity {
                entity_id: None,
                parent,
                components: BTreeMap::new(),
            },
        )
        .is_some()
    {
        return Err(command_error("entity alias already exists"));
    }
    Ok(())
}

fn remove_entity(document: &mut PrefabDocument, alias: &str) -> Result<(), SourceDiagnostic> {
    let alias = EntityAlias::new(alias.to_owned()).map_err(edit_diagnostic)?;
    if document.entities.remove(&alias).is_none() {
        return Err(command_error("entity does not exist"));
    }
    Ok(())
}

fn reparent_entity(
    document: &mut PrefabDocument,
    alias: &str,
    parent_alias: Option<&String>,
) -> Result<(), SourceDiagnostic> {
    let alias = EntityAlias::new(alias.to_owned()).map_err(edit_diagnostic)?;
    let parent = parent_alias
        .cloned()
        .map(EntityAlias::new)
        .transpose()
        .map_err(edit_diagnostic)?;
    document
        .entities
        .get_mut(&alias)
        .ok_or_else(|| command_error("entity does not exist"))?
        .parent = parent;
    Ok(())
}

fn add_instance(
    document: &mut PrefabDocument,
    alias: &str,
    source_asset: &str,
    parent_entity_alias: Option<&String>,
) -> Result<(), SourceDiagnostic> {
    let alias = InstanceAlias::new(alias.to_owned()).map_err(edit_diagnostic)?;
    let instance = PrefabInstance {
        source: PrefabAssetPath::new(source_asset.to_owned()).map_err(edit_diagnostic)?,
        parent: parent_entity_alias
            .cloned()
            .map(EntityAlias::new)
            .transpose()
            .map_err(edit_diagnostic)?,
        overrides: Vec::new(),
    };
    if document.instances.insert(alias, instance).is_some() {
        return Err(command_error("instance alias already exists"));
    }
    Ok(())
}

fn remove_instance(document: &mut PrefabDocument, alias: &str) -> Result<(), SourceDiagnostic> {
    let alias = InstanceAlias::new(alias.to_owned()).map_err(edit_diagnostic)?;
    if document.instances.remove(&alias).is_none() {
        return Err(command_error("instance does not exist"));
    }
    Ok(())
}

fn reparent_instance(
    document: &mut PrefabDocument,
    alias: &str,
    parent_entity_alias: Option<&String>,
) -> Result<(), SourceDiagnostic> {
    let alias = InstanceAlias::new(alias.to_owned()).map_err(edit_diagnostic)?;
    let parent = parent_entity_alias
        .cloned()
        .map(EntityAlias::new)
        .transpose()
        .map_err(edit_diagnostic)?;
    document
        .instances
        .get_mut(&alias)
        .ok_or_else(|| command_error("instance does not exist"))?
        .parent = parent;
    Ok(())
}

fn set_override(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    value: &ReflectedValueEnvelope,
    codec: &PrefabCodec<'_>,
) -> Result<(), SourceDiagnostic> {
    let (instance_alias, override_target) = decode_override_target(target)?;
    let value = decode_envelope(codec, value)?;
    set_override_action(
        document,
        &instance_alias,
        override_target,
        TypedOverrideAction::Set(value),
    )
}

fn clear_override(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
) -> Result<(), SourceDiagnostic> {
    let (instance_alias, override_target) = decode_override_target(target)?;
    set_override_action(
        document,
        &instance_alias,
        override_target,
        TypedOverrideAction::Clear,
    )
}

fn insert_override(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    index: u32,
    value: &ReflectedValueEnvelope,
    codec: &PrefabCodec<'_>,
) -> Result<(), SourceDiagnostic> {
    let (instance_alias, override_target) = decode_override_target(target)?;
    let value = decode_envelope(codec, value)?;
    set_override_action(
        document,
        &instance_alias,
        override_target,
        TypedOverrideAction::Insert {
            index: index as usize,
            value,
        },
    )
}

fn remove_override_item(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    index: u32,
) -> Result<(), SourceDiagnostic> {
    let (instance_alias, override_target) = decode_override_target(target)?;
    set_override_action(
        document,
        &instance_alias,
        override_target,
        TypedOverrideAction::Remove {
            index: index as usize,
        },
    )
}

fn move_override(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    from: u32,
    to: u32,
) -> Result<(), SourceDiagnostic> {
    let (instance_alias, override_target) = decode_override_target(target)?;
    set_override_action(
        document,
        &instance_alias,
        override_target,
        TypedOverrideAction::Move {
            from: from as usize,
            to: to as usize,
        },
    )
}

fn remove_override(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
) -> Result<(), SourceDiagnostic> {
    let (instance_alias, override_target) = decode_override_target(target)?;
    let instance = document
        .instances
        .get_mut(&instance_alias)
        .ok_or_else(|| command_error("override instance does not exist"))?;
    let before = instance.overrides.len();
    instance
        .overrides
        .retain(|operation| operation.target != override_target);
    if instance.overrides.len() == before {
        return Err(command_error("override does not exist"));
    }
    Ok(())
}

fn set_override_action(
    document: &mut PrefabDocument,
    instance_alias: &InstanceAlias,
    target: TypedOverrideTarget,
    action: TypedOverrideAction,
) -> Result<(), SourceDiagnostic> {
    let instance = document
        .instances
        .get_mut(instance_alias)
        .ok_or_else(|| command_error("override instance does not exist"))?;
    if let Some(operation) = instance
        .overrides
        .iter_mut()
        .find(|operation| operation.target == target)
    {
        operation.action = action;
    } else {
        instance
            .overrides
            .push(OverrideOperation { target, action });
    }
    Ok(())
}

fn invoke_action(
    document: &mut PrefabDocument,
    target: &PrefabValueTarget,
    action_id: &str,
    registry: &TypeRegistry,
) -> Result<EditorActionOutcome, SourceDiagnostic> {
    ensure_direct_target(target)?;
    if !target.path.segments.is_empty() {
        return Err(command_error(
            "typed actions currently target a component root",
        ));
    }
    let (_, component) = component_mut(document, target)?;
    let registration = registry
        .get_with_type_path(component.type_path())
        .ok_or_else(|| command_error("component type is not registered"))?;
    let policy = registration
        .data::<EditorPolicyTypeData>()
        .and_then(|policy| policy.invoke_action)
        .ok_or_else(|| command_error("component type has no typed action callback"))?;
    let mut full = materialize_component(registration, component)?;
    let outcome = policy(
        full.as_partial_reflect_mut(),
        &EditorActionId(action_id.to_owned()),
    )
    .map_err(edit_diagnostic)?;
    *component = SparseValue::try_new(full.into_partial_reflect()).map_err(edit_diagnostic)?;
    Ok(outcome)
}

fn validate_components(
    document: &PrefabDocument,
    registry: &TypeRegistry,
) -> Vec<PrefabDiagnostic> {
    let mut diagnostics = Vec::new();
    for (entity_alias, entity) in &document.entities {
        for component in entity.components.values() {
            let Some(registration) = registry.get_with_type_path(component.type_path()) else {
                diagnostics.push(PrefabDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    code: "prefab.component.unregistered".to_owned(),
                    message: format!(
                        "component type `{}` is not registered",
                        component.type_path()
                    ),
                    target: Some(component_target(
                        entity_alias,
                        component.type_path(),
                        Vec::new(),
                    )),
                });
                continue;
            };
            let Some(validation) = registration.data::<ValidationTypeData>() else {
                continue;
            };
            let full = match materialize_component(registration, component) {
                Ok(full) => full,
                Err(error) => {
                    diagnostics.push(PrefabDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        code: "prefab.component.materialize".to_owned(),
                        message: error.message,
                        target: Some(component_target(
                            entity_alias,
                            component.type_path(),
                            Vec::new(),
                        )),
                    });
                    continue;
                }
            };
            match (validation.validate)(full.as_partial_reflect()) {
                Ok(component_diagnostics) => {
                    diagnostics.extend(component_diagnostics.into_iter().map(|diagnostic| {
                        PrefabDiagnostic {
                            severity: project_severity(diagnostic.severity),
                            code: diagnostic.code,
                            message: diagnostic.message,
                            target: Some(component_target(
                                entity_alias,
                                component.type_path(),
                                diagnostic
                                    .path
                                    .0
                                    .into_iter()
                                    .map(project_core_segment)
                                    .collect(),
                            )),
                        }
                    }));
                }
                Err(error) => diagnostics.push(PrefabDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    code: "prefab.validation.callback".to_owned(),
                    message: error.to_string(),
                    target: Some(component_target(
                        entity_alias,
                        component.type_path(),
                        Vec::new(),
                    )),
                }),
            }
        }
    }
    diagnostics
}

fn materialize_component(
    registration: &TypeRegistration,
    sparse: &SparseValue,
) -> Result<Box<dyn bevy_reflect::Reflect>, SourceDiagnostic> {
    let mut full = registration
        .data::<ReflectDefault>()
        .ok_or_else(|| {
            command_error(format!(
                "type `{}` cannot be materialized without ReflectDefault",
                registration.type_info().type_path()
            ))
        })?
        .default();
    full.as_partial_reflect_mut()
        .try_apply(sparse.value())
        .map_err(|error| command_error(error.to_string()))?;
    Ok(full)
}

fn default_sparse(registration: &TypeRegistration) -> Result<SparseValue, SourceDiagnostic> {
    let value = registration
        .data::<ReflectDefault>()
        .ok_or_else(|| command_error("component has no reflected default"))?
        .default();
    SparseValue::try_new(value.into_partial_reflect()).map_err(edit_diagnostic)
}

/// Builds the empty value of `variant_name`'s declared shape for the enum
/// currently held at `target`.
///
/// A `SetVariant` command carrying no payload names a variant and nothing
/// else, so the variant's own declaration — not the command — decides what the
/// document receives. The host owns that decision because it is the only side
/// of the seam holding the authoritative descriptor.
///
/// Per shape:
/// - a unit variant takes the unit payload;
/// - a struct-shaped variant takes the empty named-field set, which the sparse
///   encoding spells `Named()`: a sparse value retains any subset of the
///   declared fields, the empty subset included, and each omitted field falls
///   back when the value is constructed;
/// - a tuple variant carries exactly as many fields as it declares, because
///   sparse tuple variants are validated on an exact field count. A variant
///   declaring none takes the empty tuple; the rest are authored from each
///   declared field type's registered reflected default, the only values the
///   host can supply without inventing them.
fn empty_variant(
    registry: &TypeRegistry,
    target: &dyn PartialReflect,
    variant_name: &str,
) -> Result<DynamicEnum, SourceDiagnostic> {
    let represented = target.get_represented_type_info().ok_or_else(|| {
        command_error(format!(
            "variant target carries no enum type declaring `{variant_name}`"
        ))
    })?;
    let info = represented.as_enum().map_err(|_| {
        command_error(format!(
            "variant target `{}` is not an enum",
            represented.type_path()
        ))
    })?;
    let variant = info.variant(variant_name).ok_or_else(|| {
        command_error(format!(
            "enum `{}` has no variant `{variant_name}`",
            info.type_path()
        ))
    })?;
    let shape = match variant {
        VariantInfo::Unit(_) => DynamicVariant::Unit,
        VariantInfo::Struct(_) => DynamicVariant::Struct(DynamicStruct::default()),
        VariantInfo::Tuple(variant) => {
            let mut fields = DynamicTuple::default();
            for index in 0..variant.field_len() {
                let field = variant
                    .field_at(index)
                    .expect("index came from the declared field count");
                let default = registry
                    .get(field.type_id())
                    .and_then(|registration| registration.data::<ReflectDefault>())
                    .ok_or_else(|| {
                        command_error(format!(
                            "tuple variant `{}::{variant_name}` field {index} of type `{}` has no reflected default",
                            info.type_path(),
                            field.type_path(),
                        ))
                    })?;
                fields.insert_boxed(default.default().into_partial_reflect());
            }
            DynamicVariant::Tuple(fields)
        }
    };
    let index = info
        .index_of(variant.name())
        .expect("resolved variant belongs to its own enum");
    let mut value = DynamicEnum::new_with_index(index, variant.name(), shape);
    value.set_represented_type(Some(represented));
    Ok(value)
}

fn component_mut<'a>(
    document: &'a mut PrefabDocument,
    target: &PrefabValueTarget,
) -> Result<(String, &'a mut SparseValue), SourceDiagnostic> {
    let alias = EntityAlias::new(target.entity_alias.clone()).map_err(edit_diagnostic)?;
    let entity = document
        .entities
        .get_mut(&alias)
        .ok_or_else(|| command_error(format!("entity `{alias}` does not exist")))?;
    let component_type_path = &target.path.component_type_path;
    let tag = entity
        .components
        .iter()
        .find_map(|(tag, value)| {
            (tag == component_type_path || value.type_path() == component_type_path)
                .then(|| tag.clone())
        })
        .ok_or_else(|| {
            command_error(format!(
                "entity `{alias}` has no component `{component_type_path}`"
            ))
        })?;
    let component = entity
        .components
        .get_mut(&tag)
        .expect("component tag came from the same map");
    Ok((tag, component))
}

fn set_value_at_path(
    root: &mut dyn PartialReflect,
    segments: &[ReflectedPathSegment],
    value: &SparseValue,
) -> Result<(), SourceDiagnostic> {
    let (last, parents) = segments
        .split_last()
        .ok_or_else(|| command_error("value path is empty"))?;
    let parent = value_at_path_mut(root, parents)?;
    match last {
        ReflectedPathSegment::Field(field) => match parent.reflect_mut() {
            ReflectMut::Struct(parent) => {
                if let Some(existing) = parent.field_mut(field) {
                    existing
                        .try_apply(value.value())
                        .map_err(|error| command_error(error.to_string()))?;
                } else if let Some(dynamic) = parent
                    .as_partial_reflect_mut()
                    .try_downcast_mut::<DynamicStruct>()
                {
                    dynamic.insert_boxed(field.clone(), clone_partial(value.value()));
                } else {
                    return Err(command_error(format!("field `{field}` is absent")));
                }
            }
            ReflectMut::Enum(parent) => parent
                .field_mut(field)
                .ok_or_else(|| command_error(format!("enum field `{field}` is absent")))?
                .try_apply(value.value())
                .map_err(|error| command_error(error.to_string()))?,
            _ => return Err(command_error("field parent is not a struct or enum")),
        },
        ReflectedPathSegment::TupleIndex(index) => {
            let index = *index as usize;
            let existing = match parent.reflect_mut() {
                ReflectMut::Tuple(parent) => parent.field_mut(index),
                ReflectMut::TupleStruct(parent) => parent.field_mut(index),
                ReflectMut::Enum(parent) => parent.field_at_mut(index),
                _ => None,
            }
            .ok_or_else(|| command_error(format!("tuple index {index} is absent")))?;
            existing
                .try_apply(value.value())
                .map_err(|error| command_error(error.to_string()))?;
        }
        ReflectedPathSegment::ListIndex(index) => {
            let index = *index as usize;
            let existing = match parent.reflect_mut() {
                ReflectMut::List(parent) => parent.get_mut(index),
                ReflectMut::Array(parent) => parent.get_mut(index),
                _ => None,
            }
            .ok_or_else(|| command_error(format!("list index {index} is absent")))?;
            existing
                .try_apply(value.value())
                .map_err(|error| command_error(error.to_string()))?;
        }
        ReflectedPathSegment::Variant(_) => {
            return Err(command_error("use SetVariant to change an enum variant"));
        }
    }
    Ok(())
}

fn value_at_path_mut<'a>(
    mut value: &'a mut dyn PartialReflect,
    segments: &[ReflectedPathSegment],
) -> Result<&'a mut dyn PartialReflect, SourceDiagnostic> {
    for segment in segments {
        value = match segment {
            ReflectedPathSegment::Field(field) => match value.reflect_mut() {
                ReflectMut::Struct(value) => value.field_mut(field),
                ReflectMut::Enum(value) => value.field_mut(field),
                _ => None,
            }
            .ok_or_else(|| command_error(format!("field `{field}` is absent")))?,
            ReflectedPathSegment::Variant(variant) => {
                let ReflectMut::Enum(value) = value.reflect_mut() else {
                    return Err(command_error(format!("`{variant}` target is not an enum")));
                };
                if value.variant_name() != variant {
                    return Err(command_error(format!(
                        "enum is `{}`, not `{variant}`",
                        value.variant_name()
                    )));
                }
                value.as_partial_reflect_mut()
            }
            ReflectedPathSegment::TupleIndex(index) => {
                let index = *index as usize;
                match value.reflect_mut() {
                    ReflectMut::Tuple(value) => value.field_mut(index),
                    ReflectMut::TupleStruct(value) => value.field_mut(index),
                    ReflectMut::Enum(value) => value.field_at_mut(index),
                    _ => None,
                }
                .ok_or_else(|| command_error(format!("tuple index {index} is absent")))?
            }
            ReflectedPathSegment::ListIndex(index) => {
                let index = *index as usize;
                match value.reflect_mut() {
                    ReflectMut::List(value) => value.get_mut(index),
                    ReflectMut::Array(value) => value.get_mut(index),
                    _ => None,
                }
                .ok_or_else(|| command_error(format!("list index {index} is absent")))?
            }
        };
    }
    Ok(value)
}

fn decode_envelope(
    codec: &PrefabCodec<'_>,
    envelope: &ReflectedValueEnvelope,
) -> Result<SparseValue, SourceDiagnostic> {
    if envelope.encoding != ReflectedValueEncoding::TypedRon {
        return Err(command_error(format!(
            "unsupported reflected value encoding {:?}",
            envelope.encoding
        )));
    }
    codec
        .decode_sparse_value(&envelope.type_path, &envelope.payload)
        .map_err(edit_diagnostic)
}

fn decode_override_target(
    target: &PrefabValueTarget,
) -> Result<(InstanceAlias, TypedOverrideTarget), SourceDiagnostic> {
    let (root, rest) = target
        .instance_alias_chain
        .split_first()
        .ok_or_else(|| command_error("override target requires an instance alias"))?;
    let root = InstanceAlias::new(root.clone()).map_err(edit_diagnostic)?;
    let chain = rest
        .iter()
        .cloned()
        .map(InstanceAlias::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(edit_diagnostic)?;
    let named_path = target
        .path
        .segments
        .iter()
        .map(|segment| match segment {
            ReflectedPathSegment::Field(field) => Ok(field.clone()),
            _ => Err(command_error(
                "nested Prefab overrides currently require named field paths",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let target = TypedOverrideTarget::new(
        chain,
        EntityAlias::new(target.entity_alias.clone()).map_err(edit_diagnostic)?,
        target.path.component_type_path.clone(),
        PrefabReflectedPath::new(named_path).map_err(edit_diagnostic)?,
    )
    .map_err(edit_diagnostic)?;
    Ok((root, target))
}

fn ensure_direct_target(target: &PrefabValueTarget) -> Result<(), SourceDiagnostic> {
    if target.instance_alias_chain.is_empty() {
        Ok(())
    } else {
        Err(command_error(
            "direct component edits cannot carry an instance alias chain",
        ))
    }
}

fn clone_partial(value: &dyn PartialReflect) -> Box<dyn PartialReflect> {
    value.reflect_clone().map_or_else(
        |_| value.to_dynamic(),
        bevy_reflect::PartialReflect::into_partial_reflect,
    )
}

fn component_target(
    entity: &EntityAlias,
    component_type_path: &str,
    segments: Vec<ReflectedPathSegment>,
) -> PrefabValueTarget {
    PrefabValueTarget {
        instance_alias_chain: Vec::new(),
        entity_alias: entity.as_str().to_owned(),
        path: ReflectedPath {
            component_type_path: component_type_path.to_owned(),
            segments,
        },
    }
}

fn project_core_segment(segment: CorePathSegment) -> ReflectedPathSegment {
    match segment {
        CorePathSegment::Field(value) => ReflectedPathSegment::Field(value),
        CorePathSegment::Variant(value) => ReflectedPathSegment::Variant(value),
        CorePathSegment::TupleIndex(value) => ReflectedPathSegment::TupleIndex(value),
        CorePathSegment::ListIndex(value) => ReflectedPathSegment::ListIndex(value),
    }
}

const fn project_severity(severity: CoreDiagnosticSeverity) -> DiagnosticSeverity {
    match severity {
        CoreDiagnosticSeverity::Info => DiagnosticSeverity::Info,
        CoreDiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
        CoreDiagnosticSeverity::Error => DiagnosticSeverity::Error,
    }
}

fn diagnostic_path(diagnostic: &PrefabDiagnostic) -> String {
    let Some(target) = &diagnostic.target else {
        return String::new();
    };
    let mut path = format!(
        "{}:{}",
        target.entity_alias, target.path.component_type_path
    );
    for segment in &target.path.segments {
        // Writing into a `String` is infallible.
        let _ = match segment {
            ReflectedPathSegment::Field(field) => write!(path, ".{field}"),
            ReflectedPathSegment::Variant(variant) => write!(path, "::{variant}"),
            ReflectedPathSegment::TupleIndex(index) => write!(path, ".{index}"),
            ReflectedPathSegment::ListIndex(index) => write!(path, "[{index}]"),
        };
    }
    path
}

fn command_error(message: impl Into<String>) -> SourceDiagnostic {
    SourceDiagnostic::new(SourceDiagnosticPhase::Edit, message)
}

fn edit_diagnostic(error: impl std::fmt::Display) -> SourceDiagnostic {
    command_error(error.to_string())
}

fn host_diagnostic(error: &VNextHostError) -> PrefabDiagnostic {
    PrefabDiagnostic {
        severity: DiagnosticSeverity::Error,
        code: error.code().to_owned(),
        message: error.to_string(),
        target: None,
    }
}

fn diagnostics_from_host_error(error: VNextHostError) -> Vec<PrefabDiagnostic> {
    match error {
        VNextHostError::Session(SourceSessionHostError::Diagnostics(diagnostics)) => diagnostics
            .into_iter()
            .map(|diagnostic| PrefabDiagnostic {
                severity: DiagnosticSeverity::Error,
                code: "prefab.source.validation".to_owned(),
                message: diagnostic.message,
                target: None,
            })
            .collect(),
        error => vec![host_diagnostic(&error)],
    }
}

#[derive(Debug, Error)]
pub enum VNextHostError {
    #[error("project-host has no project source root")]
    MissingSourceRoot,
    #[error("invalid typed source path `{0}`")]
    InvalidSourcePath(String),
    #[error("typed source session `{0}` is not open")]
    SessionNotOpen(String),
    #[error("typed source revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("unknown Prefab component `{0}`")]
    UnknownComponent(String),
    #[error(transparent)]
    Registry(#[from] crate::RegistryValidationError),
    #[error(transparent)]
    Codec(#[from] az_prefab::PrefabCodecError),
    #[error(transparent)]
    Session(#[from] SourceSessionHostError),
    #[error(transparent)]
    Project(#[from] az_project::ProjectManifestError),
    #[error(transparent)]
    DataHome(#[from] az_filesystem::DataHomeError),
    #[error("typed source operation failed: {0}")]
    Diagnostic(String),
}

impl VNextHostError {
    const fn code(&self) -> &'static str {
        match self {
            Self::MissingSourceRoot => "prefab.source.missing_root",
            Self::InvalidSourcePath(_) => "prefab.source.invalid_path",
            Self::SessionNotOpen(_) => "prefab.source.not_open",
            Self::RevisionConflict { .. } => "prefab.source.revision_conflict",
            Self::UnknownComponent(_) => "prefab.component.unknown",
            Self::Registry(_) => "prefab.registry.invalid",
            Self::Codec(_) => "prefab.codec.failed",
            Self::Session(_) => "prefab.session.failed",
            Self::Project(_) => "prefab.project.failed",
            Self::DataHome(_) => "prefab.data_home.failed",
            Self::Diagnostic(_) => "prefab.operation.failed",
        }
    }
}
