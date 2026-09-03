//! Reflected Prefab selection and inspection for the ADR-0022 vNext contract.
//!
//! A selection is a source path plus an entity alias. Resolution happens only
//! against project-host Prefab and type-registry snapshots; each selected
//! component is projected by the UI-neutral reflected inspector model.

pub use az_editor_inspector::{
    ReflectedComponentInspection, ReflectedEntityInspection, ReflectedPrefabSelection,
};
use az_editor_inspector::{
    ReflectedEditBinding, ReflectedInspectionInput, ReflectedInspectionModel,
    ReflectedOverrideOperation, ReflectedProjectionError,
};
pub use az_editor_ui::panels::EditorReflectedSelectionState;
use az_editor_ui::panels::{ComponentCapabilityData, CreatableAuthoredSchemaData};
use az_proto_project::vnext::{
    PrefabDiagnostic, PrefabEditCommand, PrefabRpcResult, PrefabSourceSnapshot,
    ReflectedValueEnvelope, SourceSessionResult, TypeRegistrySnapshot,
};
use gpui::App;
use thiserror::Error;
use tracing::{info, instrument};

use crate::attach::EditorAttachSession;
use crate::authored_edit::{
    ReflectedHistoryDirection, ReflectedPrefabEdit, ReflectedPrefabEditSession,
};
use crate::error::{EditorError, EditorResult};

/// Failures raised while resolving neutral selection snapshots.
#[derive(Debug, Error)]
pub enum ReflectedSelectionError {
    #[error("project-host returned no Prefab snapshot for `{source_path}` during {operation}")]
    MissingSnapshot {
        source_path: String,
        operation: &'static str,
    },
    #[error("Prefab source `{source_path}` has no entity alias `{entity_alias}`")]
    MissingEntity {
        source_path: String,
        entity_alias: String,
    },
    #[error(
        "failed to project component `{component_type_path}` on `{entity_alias}` in `{source_path}`"
    )]
    ComponentProjection {
        source_path: String,
        entity_alias: String,
        component_type_path: String,
        // Boxed to keep `ReflectedSelectionError` under the `result_large_err`
        // threshold; the three paths already cost 72 bytes on their own.
        #[source]
        source: Box<ReflectedProjectionError>,
    },
}

/// Resolves one entity selection and projects each of its components.
///
/// # Errors
///
/// Returns [`ReflectedSelectionError::MissingEntity`] if `snapshot` holds no
/// entity under the selection's alias, or
/// [`ReflectedSelectionError::ComponentProjection`] if one of that entity's
/// components fails reflected projection — its type is absent from `registry`,
/// or its typed-RON envelope does not match that type's reflected structure.
pub fn project_reflected_selection(
    selection: ReflectedPrefabSelection,
    registry: &TypeRegistrySnapshot,
    snapshot: &PrefabSourceSnapshot,
    diagnostics: Vec<PrefabDiagnostic>,
) -> Result<ReflectedEntityInspection, ReflectedSelectionError> {
    if !snapshot
        .entities
        .iter()
        .any(|entity| entity.alias == selection.entity_alias)
    {
        return Err(ReflectedSelectionError::MissingEntity {
            source_path: selection.source_path,
            entity_alias: selection.entity_alias,
        });
    }

    let components = snapshot
        .components
        .iter()
        .filter(|component| component.entity_alias == selection.entity_alias)
        .map(|component| {
            ReflectedInspectionModel::project(
                ReflectedInspectionInput::new(registry, component).with_diagnostics(&diagnostics),
            )
            .map(|model| ReflectedComponentInspection {
                component: component.clone(),
                model,
            })
            .map_err(|source| ReflectedSelectionError::ComponentProjection {
                source_path: selection.source_path.clone(),
                entity_alias: selection.entity_alias.clone(),
                component_type_path: component.type_path.clone(),
                source: Box::new(source),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let overrides = snapshot
        .overrides
        .iter()
        .filter(|snapshot| snapshot.operation.target().entity_alias == selection.entity_alias)
        .map(ReflectedOverrideOperation::project)
        .collect();

    Ok(ReflectedEntityInspection {
        selection,
        registry_schema_catalog_hash: registry.schema_catalog_hash.clone(),
        document_version: snapshot.document_version,
        type_versions: snapshot.type_versions.clone(),
        revision: snapshot.revision,
        components,
        overrides,
        diagnostics,
    })
}

/// Projects runtime-exported reflected component registrations for Add Component UI.
#[must_use]
pub fn addable_reflected_component_data(
    registry: &TypeRegistrySnapshot,
) -> Vec<CreatableAuthoredSchemaData> {
    let mut components = registry
        .types
        .iter()
        .filter(|descriptor| {
            descriptor
                .type_data_flags
                .iter()
                .any(|flag| flag == "ReflectComponent")
                && descriptor
                    .type_data_flags
                    .iter()
                    .any(|flag| flag == "Prefab")
                && descriptor.applicability.default_available
        })
        .map(|descriptor| CreatableAuthoredSchemaData {
            schema_type: descriptor.type_path.clone(),
            label: descriptor
                .editor_attributes
                .label
                .clone()
                .unwrap_or_else(|| descriptor.short_path.clone()),
            category: descriptor.editor_attributes.category.clone(),
            icon: descriptor.editor_attributes.icon.clone(),
            component_capabilities: Some(ComponentCapabilityData {
                provides: descriptor.applicability.provides.clone(),
                requires: descriptor.applicability.requires.clone(),
                incompatible: descriptor.applicability.incompatible.clone(),
            }),
        })
        .collect::<Vec<_>>();
    components.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.schema_type.cmp(&right.schema_type))
    });
    components
}

/// Running editor controller for vNext Prefab selection and inspection.
#[derive(Clone)]
pub struct EditorReflectedSelectionController {
    edit_session: ReflectedPrefabEditSession,
}

impl EditorReflectedSelectionController {
    #[cfg(test)]
    #[must_use]
    pub const fn new(edit_session: ReflectedPrefabEditSession) -> Self {
        Self { edit_session }
    }

    /// Opens the reconnecting vNext edit session this controller drives.
    ///
    /// # Errors
    ///
    /// Returns any error [`ReflectedPrefabEditSession::connect_attached`]
    /// returns. The reconnecting production bridge only records `session`'s
    /// descriptors, so it cannot fail here; the in-process test bridge dials
    /// project-host eagerly and surfaces that connection's failures.
    #[instrument(
        skip(session),
        fields(session = %session.session_slug, session_id = %session.session_id)
    )]
    pub async fn connect_attached(session: &EditorAttachSession) -> EditorResult<Self> {
        Ok(Self {
            edit_session: ReflectedPrefabEditSession::connect_attached(session).await?,
        })
    }

    #[must_use]
    pub const fn edit_session(&self) -> &ReflectedPrefabEditSession {
        &self.edit_session
    }

    /// Loads and projects one entity directly from vNext source and registry snapshots.
    ///
    /// # Errors
    ///
    /// Returns any error [`ReflectedPrefabEditSession::source_snapshot`],
    /// [`ReflectedPrefabEditSession::type_registry_snapshot`], or
    /// [`ReflectedPrefabEditSession::diagnostics`] returns, or
    /// [`EditorError::ReflectedSelection`] if project-host answered with no
    /// Prefab snapshot, if that snapshot holds no entity under the selected
    /// alias, or if one of the entity's components fails reflected projection.
    #[instrument(
        skip(self),
        fields(source_path = %selection.source_path, entity_alias = %selection.entity_alias)
    )]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn inspect(
        &self,
        selection: ReflectedPrefabSelection,
    ) -> EditorResult<ReflectedEntityInspection> {
        let result = self
            .edit_session
            .source_snapshot(&selection.source_path)
            .await?;
        self.project_rpc_result(selection, result, "selection")
            .await
    }

    /// Applies a vNext command and reprojects the selected entity from the
    /// authoritative result snapshot.
    ///
    /// # Errors
    ///
    /// Returns any error [`ReflectedPrefabEditSession::apply`],
    /// [`ReflectedPrefabEditSession::type_registry_snapshot`], or
    /// [`ReflectedPrefabEditSession::diagnostics`] returns, or
    /// [`EditorError::ReflectedSelection`] if the edit result carries no Prefab
    /// snapshot, if that snapshot holds no entity under the selected alias, or
    /// if one of the entity's components fails reflected projection.
    #[instrument(
        skip(self, current, command),
        fields(
            source_path = %current.selection.source_path,
            entity_alias = %current.selection.entity_alias,
            expected_revision = current.revision
        )
    )]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn apply_command(
        &self,
        current: &ReflectedEntityInspection,
        command: PrefabEditCommand,
    ) -> EditorResult<ReflectedEntityInspection> {
        let edit = ReflectedPrefabEdit::new(
            current.selection.source_path.clone(),
            current.revision,
            command,
        );
        let result = self.edit_session.apply(&edit).await?;
        self.project_rpc_result(current.selection.clone(), result, "edit")
            .await
    }

    /// Applies a value from a binding emitted by the current reflected model.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::InvalidArgument`] if `binding` targets a nested
    /// instance, another entity alias, or a component `current` does not hold,
    /// and otherwise any error [`Self::apply_command`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn set_value(
        &self,
        current: &ReflectedEntityInspection,
        binding: &ReflectedEditBinding,
        value: ReflectedValueEnvelope,
    ) -> EditorResult<ReflectedEntityInspection> {
        ensure_binding_belongs_to_inspection(current, binding)?;
        self.apply_command(current, binding.set_value(value)).await
    }

    /// Invokes a reflected action and refreshes the current selection.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::InvalidArgument`] if `binding` targets a nested
    /// instance, another entity alias, or a component `current` does not hold;
    /// any error [`ReflectedPrefabEditSession::invoke_action`] returns; and,
    /// when the action answers with no snapshot, any error [`Self::inspect`]
    /// returns for the re-read. An action project-host does not recognize comes
    /// back as a diagnostic on the refreshed inspection, not as an error.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn invoke_action(
        &self,
        current: &ReflectedEntityInspection,
        binding: &ReflectedEditBinding,
        action_id: &str,
    ) -> EditorResult<ReflectedEntityInspection> {
        ensure_binding_belongs_to_inspection(current, binding)?;
        let result = self
            .edit_session
            .invoke_action(
                &current.selection.source_path,
                current.revision,
                binding,
                action_id,
            )
            .await?;
        match result.snapshot {
            Some(snapshot) => {
                self.project_snapshot(current.selection.clone(), snapshot, result.diagnostics)
                    .await
            }
            None => self.inspect(current.selection.clone()).await,
        }
    }

    /// Traverses project-host-owned history and reprojects the current entity.
    ///
    /// # Errors
    ///
    /// Returns any error [`ReflectedPrefabEditSession::history`] returns and,
    /// when the lifecycle result carries no snapshot, any error
    /// [`ReflectedPrefabEditSession::source_snapshot`] returns for the re-read.
    /// Returns [`EditorError::ReflectedSelection`] if neither call yields a
    /// Prefab snapshot, if the resulting snapshot holds no entity under the
    /// selected alias, or if one of the entity's components fails reflected
    /// projection.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn history(
        &self,
        current: &ReflectedEntityInspection,
        direction: ReflectedHistoryDirection,
    ) -> EditorResult<ReflectedEntityInspection> {
        let result = self
            .edit_session
            .history(&current.selection.source_path, current.revision, direction)
            .await?;
        self.project_lifecycle_result(current.selection.clone(), result, "history")
            .await
    }

    /// Steps the source session back one revision and reprojects the entity.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::history`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn undo(
        &self,
        current: &ReflectedEntityInspection,
    ) -> EditorResult<ReflectedEntityInspection> {
        self.history(current, ReflectedHistoryDirection::Undo).await
    }

    /// Steps the source session forward one revision and reprojects the entity.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::history`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn redo(
        &self,
        current: &ReflectedEntityInspection,
    ) -> EditorResult<ReflectedEntityInspection> {
        self.history(current, ReflectedHistoryDirection::Redo).await
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn project_rpc_result(
        &self,
        selection: ReflectedPrefabSelection,
        result: PrefabRpcResult,
        operation: &'static str,
    ) -> EditorResult<ReflectedEntityInspection> {
        let snapshot = result
            .snapshot
            .ok_or_else(|| ReflectedSelectionError::MissingSnapshot {
                source_path: selection.source_path.clone(),
                operation,
            })?;
        self.project_snapshot(selection, snapshot, result.diagnostics)
            .await
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn project_lifecycle_result(
        &self,
        selection: ReflectedPrefabSelection,
        result: SourceSessionResult,
        operation: &'static str,
    ) -> EditorResult<ReflectedEntityInspection> {
        if let Some(snapshot) = result.snapshot {
            self.project_snapshot(selection, snapshot, result.diagnostics)
                .await
        } else {
            let source = self
                .edit_session
                .source_snapshot(&selection.source_path)
                .await?;
            self.project_rpc_result(selection, source, operation).await
        }
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn project_snapshot(
        &self,
        selection: ReflectedPrefabSelection,
        snapshot: PrefabSourceSnapshot,
        mut diagnostics: Vec<PrefabDiagnostic>,
    ) -> EditorResult<ReflectedEntityInspection> {
        let registry = self.edit_session.type_registry_snapshot().await?;
        let current_diagnostics = self
            .edit_session
            .diagnostics(&selection.source_path)
            .await?;
        merge_diagnostics(&mut diagnostics, current_diagnostics);
        let inspection = project_reflected_selection(selection, &registry, &snapshot, diagnostics)?;
        info!(
            source_path = %inspection.selection.source_path,
            entity_alias = %inspection.selection.entity_alias,
            revision = inspection.revision,
            registry_schema_catalog_hash = ?inspection.registry_schema_catalog_hash,
            component_count = inspection.components.len(),
            diagnostic_count = inspection.diagnostics.len(),
            "projected reflected Prefab selection"
        );
        Ok(inspection)
    }
}

/// Installs reflected inspector command, history, action, and refresh routes.
pub fn install_reflected_selection_action_handlers(cx: &mut App) {
    cx.on_action(
        |action: &az_editor_ui::actions::ApplyReflectedPrefabEdit, cx| {
            apply_reflected_prefab_command(cx, action.command.clone());
        },
    );
    cx.on_action(
        |action: &az_editor_ui::actions::InvokeReflectedInspectorAction, cx| {
            invoke_reflected_inspector_action(cx, action.binding.clone(), action.action_id.clone());
        },
    );
    cx.on_action(|_: &az_editor_ui::actions::UndoReflectedPrefabEdit, cx| {
        traverse_reflected_history(cx, ReflectedHistoryDirection::Undo);
    });
    cx.on_action(|_: &az_editor_ui::actions::RedoReflectedPrefabEdit, cx| {
        traverse_reflected_history(cx, ReflectedHistoryDirection::Redo);
    });
    cx.on_action(|_: &az_editor_ui::actions::Undo, cx| {
        if !crate::game_data_catalog::try_undo_active_game_data(cx) {
            traverse_reflected_history(cx, ReflectedHistoryDirection::Undo);
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::Redo, cx| {
        if !crate::game_data_catalog::try_redo_active_game_data(cx) {
            traverse_reflected_history(cx, ReflectedHistoryDirection::Redo);
        }
    });
    cx.on_action(
        |_: &az_editor_ui::actions::RefreshReflectedInspection, cx| {
            refresh_reflected_inspection(cx);
        },
    );
}

pub(crate) fn apply_reflected_prefab_command(cx: &mut App, command: PrefabEditCommand) {
    let (controller, fence, current) = match reflected_controller_and_selection(cx) {
        Ok(controller) => controller,
        Err(error) => {
            tracing::error!(%error, "cannot apply reflected Prefab edit");
            return;
        }
    };
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "reflected-inspector-edit",
        move || async move { controller.apply_command(&current, command).await },
        move |cx, result| publish_reflected_inspector_result(cx, fence, result),
    );
}

fn invoke_reflected_inspector_action(
    cx: &mut App,
    binding: ReflectedEditBinding,
    action_id: String,
) {
    let (controller, fence, current) = match reflected_controller_and_selection(cx) {
        Ok(controller) => controller,
        Err(error) => {
            tracing::error!(%error, "cannot invoke reflected inspector action");
            return;
        }
    };
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "reflected-inspector-action",
        move || async move {
            controller
                .invoke_action(&current, &binding, &action_id)
                .await
        },
        move |cx, result| publish_reflected_inspector_result(cx, fence, result),
    );
}

fn traverse_reflected_history(cx: &mut App, direction: ReflectedHistoryDirection) {
    let (controller, fence, current) = match reflected_controller_and_selection(cx) {
        Ok(controller) => controller,
        Err(error) => {
            tracing::error!(%error, "cannot traverse reflected Prefab history");
            return;
        }
    };
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "reflected-inspector-history",
        move || async move { controller.history(&current, direction).await },
        move |cx, result| publish_reflected_inspector_result(cx, fence, result),
    );
}

fn refresh_reflected_inspection(cx: &mut App) {
    let (controller, fence, current) = match reflected_controller_and_selection(cx) {
        Ok(controller) => controller,
        Err(error) => {
            tracing::error!(%error, "cannot refresh reflected inspection");
            return;
        }
    };
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "reflected-inspector-refresh",
        move || async move { controller.inspect(current.selection).await },
        move |cx, result| publish_reflected_inspector_result(cx, fence, result),
    );
}

fn reflected_controller_and_selection(
    cx: &App,
) -> EditorResult<(
    EditorReflectedSelectionController,
    crate::controller_set::ControllerFence,
    ReflectedEntityInspection,
)> {
    let attached = crate::controller_set::reflected_selection_controller(cx)?;
    let current = cx
        .try_global::<EditorReflectedSelectionState>()
        .ok_or_else(|| {
            EditorError::InvalidArgument("reflected selection state is unavailable".into())
        })?
        .current()
        .ok_or_else(|| EditorError::InvalidArgument("no reflected Prefab selection".into()))?
        .clone();
    Ok((attached.controller, attached.fence, current))
}

fn publish_reflected_inspector_result(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    result: EditorResult<ReflectedEntityInspection>,
) {
    if !crate::controller_set::is_current_fence(cx, fence) {
        return;
    }
    match result {
        Ok(inspection) => {
            crate::recovery::note_source_session_dirty(
                cx,
                inspection.selection.source_path.clone(),
                inspection.revision,
            );
            publish_reflected_inspection(cx, inspection);
        }
        Err(error) => tracing::error!(%error, "reflected inspector operation failed"),
    }
}

/// Installs the reconnecting vNext controller for an attached editor session.
pub(crate) fn install_reflected_selection_slot(
    cx: &mut App,
    session: EditorAttachSession,
    fence: crate::controller_set::ControllerFence,
) {
    let session_slug = session.session_slug.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "reflected-selection-install",
        move || async move { EditorReflectedSelectionController::connect_attached(&session).await },
        move |cx, result| match result {
            Ok(controller) => {
                if !crate::controller_set::complete_reflected_selection(cx, fence, controller) {
                    return;
                }
                cx.set_global(EditorReflectedSelectionState::new());
                cx.refresh_windows();
                info!(
                    session = %session_slug,
                    "installed reflected Prefab selection controller"
                );
            }
            Err(error) => {
                crate::controller_set::fail_controller(cx, fence, error.to_string());
                tracing::error!(
                    %error,
                    session = %session_slug,
                    "failed to install reflected Prefab selection controller"
                );
            }
        },
    );
}

/// Publishes one neutral reflected inspection for future vNext UI consumers.
pub fn publish_reflected_inspection(cx: &mut App, inspection: ReflectedEntityInspection) {
    crate::mannequin_animation::sync_animation_preview_from_reflected_inspection(cx, &inspection);
    cx.default_global::<EditorReflectedSelectionState>()
        .set_current(inspection);
}

/// Resolves and publishes a vNext Prefab entity selected by an editor surface.
///
/// # Errors
///
/// Returns [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the reflected selection
/// controller slot is not ready. The inspection itself runs on the RPC runtime,
/// so its failures are logged and published rather than returned here.
pub fn select_reflected_entity(
    cx: &mut App,
    source_path: impl Into<String>,
    entity_alias: impl Into<String>,
) -> EditorResult<()> {
    let attached = crate::controller_set::reflected_selection_controller(cx)?;
    let controller = attached.controller;
    let fence = attached.fence;
    let selection = ReflectedPrefabSelection::new(source_path, entity_alias);
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "reflected-surface-selection",
        move || async move { controller.inspect(selection).await },
        move |cx, result| publish_reflected_inspector_result(cx, fence, result),
    );
    Ok(())
}

fn ensure_binding_belongs_to_inspection(
    inspection: &ReflectedEntityInspection,
    binding: &ReflectedEditBinding,
) -> EditorResult<()> {
    let target = &binding.target;
    if !target.instance_alias_chain.is_empty()
        || target.entity_alias != inspection.selection.entity_alias
        || inspection
            .component(&target.path.component_type_path)
            .is_none()
    {
        return Err(EditorError::InvalidArgument(format!(
            "reflected edit target `{}`/`{}` does not belong to selected entity `{}` in `{}`",
            target.entity_alias,
            target.path.component_type_path,
            inspection.selection.entity_alias,
            inspection.selection.source_path,
        )));
    }
    Ok(())
}

fn merge_diagnostics(diagnostics: &mut Vec<PrefabDiagnostic>, additional: Vec<PrefabDiagnostic>) {
    for diagnostic in additional {
        if !diagnostics.contains(&diagnostic) {
            diagnostics.push(diagnostic);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use az_proto_project::vnext::{
        ApplicabilityDescriptor, DiagnosticSeverity, EditorAttributes, FieldConstraints,
        PrefabComponentSnapshot, PrefabEntitySnapshot, PrefabOverrideOperation,
        PrefabOverrideSnapshot, PrefabValueTarget, ReflectedFieldDescriptor, ReflectedPath,
        ReflectedPathSegment, ReflectedTypeDescriptor, ReflectedTypeKind, ReflectedValueEncoding,
    };

    use super::*;

    fn attributes(label: Option<&str>) -> EditorAttributes {
        EditorAttributes {
            label: label.map(ToOwned::to_owned),
            constraints: FieldConstraints::default(),
            ..EditorAttributes::default()
        }
    }

    fn descriptor(
        type_path: &str,
        kind: ReflectedTypeKind,
        fields: Vec<ReflectedFieldDescriptor>,
    ) -> ReflectedTypeDescriptor {
        ReflectedTypeDescriptor {
            type_path: type_path.to_owned(),
            short_path: type_path
                .rsplit("::")
                .next()
                .unwrap_or(type_path)
                .to_owned(),
            kind,
            fields,
            variants: Vec::new(),
            editor_attributes: attributes(None),
            type_data_flags: Vec::new(),
            applicability: ApplicabilityDescriptor::default(),
            reflected_default: None,
        }
    }

    fn envelope(type_path: &str, value: &str) -> ReflectedValueEnvelope {
        ReflectedValueEnvelope {
            type_path: type_path.to_owned(),
            encoding: ReflectedValueEncoding::TypedRon,
            payload: value.as_bytes().to_vec(),
        }
    }

    fn registry() -> TypeRegistrySnapshot {
        TypeRegistrySnapshot {
            schema_catalog_hash: vec![17; 32],
            types: vec![
                descriptor("f32", ReflectedTypeKind::Float { bits: 32 }, Vec::new()),
                descriptor("bool", ReflectedTypeKind::Bool, Vec::new()),
                ReflectedTypeDescriptor {
                    editor_attributes: attributes(Some("Transform")),
                    type_data_flags: vec!["ReflectComponent".to_owned(), "Prefab".to_owned()],
                    applicability: ApplicabilityDescriptor {
                        default_available: true,
                        ..ApplicabilityDescriptor::default()
                    },
                    reflected_default: Some(envelope(
                        "fixture::Transform",
                        "(translation: 0.0, visible: true)",
                    )),
                    ..descriptor(
                        "fixture::Transform",
                        ReflectedTypeKind::Struct,
                        vec![
                            ReflectedFieldDescriptor {
                                name: "translation".to_owned(),
                                type_path: "f32".to_owned(),
                                editor_attributes: attributes(Some("Translation")),
                            },
                            ReflectedFieldDescriptor {
                                name: "visible".to_owned(),
                                type_path: "bool".to_owned(),
                                editor_attributes: attributes(Some("Visible")),
                            },
                        ],
                    )
                },
            ],
        }
    }

    fn snapshot() -> PrefabSourceSnapshot {
        let target = PrefabValueTarget {
            instance_alias_chain: Vec::new(),
            entity_alias: "root".to_owned(),
            path: ReflectedPath {
                component_type_path: "fixture::Transform".to_owned(),
                segments: vec![ReflectedPathSegment::Field("translation".to_owned())],
            },
        };
        PrefabSourceSnapshot {
            document_version: 3,
            type_versions: BTreeMap::from([("fixture::Transform".to_owned(), 2)]),
            entities: vec![
                PrefabEntitySnapshot {
                    alias: "root".to_owned(),
                },
                PrefabEntitySnapshot {
                    alias: "child".to_owned(),
                },
            ],
            hierarchy: Vec::new(),
            components: vec![PrefabComponentSnapshot {
                entity_alias: "root".to_owned(),
                type_path: "fixture::Transform".to_owned(),
                sparse_value: envelope("fixture::Transform", "(translation: 4.25, visible: false)"),
            }],
            instances: Vec::new(),
            overrides: vec![PrefabOverrideSnapshot {
                operation: PrefabOverrideOperation::Set {
                    target,
                    value: envelope("f32", "4.25"),
                },
            }],
            revision: 29,
        }
    }

    fn diagnostic() -> PrefabDiagnostic {
        PrefabDiagnostic {
            severity: DiagnosticSeverity::Error,
            code: "fixture.translation".to_owned(),
            message: "translation is outside the fixture range".to_owned(),
            target: Some(PrefabValueTarget {
                instance_alias_chain: Vec::new(),
                entity_alias: "root".to_owned(),
                path: ReflectedPath {
                    component_type_path: "fixture::Transform".to_owned(),
                    segments: vec![ReflectedPathSegment::Field("translation".to_owned())],
                },
            }),
        }
    }

    #[test]
    fn selection_projects_entity_components_through_reflected_model() {
        let inspection = project_reflected_selection(
            ReflectedPrefabSelection::new("levels/main.prefab.ron", "root"),
            &registry(),
            &snapshot(),
            vec![diagnostic()],
        )
        .unwrap();

        assert_eq!(inspection.registry_schema_catalog_hash, vec![17; 32]);
        assert_eq!(inspection.document_version, 3);
        assert_eq!(inspection.revision, 29);
        assert_eq!(inspection.components.len(), 1);
        assert_eq!(inspection.overrides.len(), 1);
        let model = &inspection.components[0].model;
        assert_eq!(model.type_label, "Transform");
        assert_eq!(model.fields[0].name, "translation");
        assert!(!model.fields[0].validation.is_valid());
        assert_eq!(
            model.fields[0].value.binding.target.path.segments,
            vec![ReflectedPathSegment::Field("translation".to_owned())]
        );
    }

    #[test]
    fn selection_rejects_unknown_entity_alias() {
        let error = project_reflected_selection(
            ReflectedPrefabSelection::new("levels/main.prefab.ron", "missing"),
            &registry(),
            &snapshot(),
            Vec::new(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ReflectedSelectionError::MissingEntity { .. }
        ));
    }

    #[test]
    fn selection_reports_missing_component_type_projection() {
        let error = project_reflected_selection(
            ReflectedPrefabSelection::new("levels/main.prefab.ron", "root"),
            &TypeRegistrySnapshot {
                schema_catalog_hash: vec![1; 32],
                types: Vec::new(),
            },
            &snapshot(),
            Vec::new(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ReflectedSelectionError::ComponentProjection { .. }
        ));
    }

    #[test]
    fn edit_binding_must_belong_to_selected_component() {
        let inspection = project_reflected_selection(
            ReflectedPrefabSelection::new("levels/main.prefab.ron", "root"),
            &registry(),
            &snapshot(),
            Vec::new(),
        )
        .unwrap();
        let foreign = ReflectedEditBinding::new(PrefabValueTarget {
            instance_alias_chain: Vec::new(),
            entity_alias: "child".to_owned(),
            path: ReflectedPath {
                component_type_path: "fixture::Transform".to_owned(),
                segments: Vec::new(),
            },
        });

        assert!(ensure_binding_belongs_to_inspection(&inspection, &foreign).is_err());
    }

    #[test]
    fn diagnostic_merge_is_stable_and_deduplicated() {
        let value = diagnostic();
        let mut diagnostics = vec![value.clone()];
        merge_diagnostics(&mut diagnostics, vec![value]);
        assert_eq!(diagnostics.len(), 1);
    }
}
